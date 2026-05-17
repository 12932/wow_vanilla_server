use ahash::AHashMap;
use std::sync::{Arc, Mutex};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, warn};
use wow_login_messages::all::{
    CMD_AUTH_LOGON_CHALLENGE_Client, CMD_AUTH_RECONNECT_CHALLENGE_Client, ProtocolVersion,
};
use wow_login_messages::errors::ExpectedOpcodeError;
use wow_login_messages::helper::{
    tokio_expect_client_message, tokio_read_initial_message, InitialMessage,
};
use wow_login_messages::ServerMessage;
use wow_srp::normalized_string::NormalizedString;
use wow_srp::server::{SrpProof, SrpServer, SrpVerifier};
use wow_srp::{PublicKey, GENERATOR, LARGE_SAFE_PRIME_LITTLE_ENDIAN};

/// Shared SRP session-key cache used by both the auth server (writes
/// successful logins, reads on reconnect challenge) and the world server
/// (reads on world handshake). Wrapped in a `Mutex` because both paths
/// can hit it concurrently.
///
/// The cache is **bounded** via TTL-based eviction. Without bounding the
/// underlying `AHashMap<String, SrpServer>` grows monotonically — every
/// unique account that has ever logged in keeps a ~200-byte `SrpServer`
/// entry forever. Eviction runs opportunistically at insert time (no
/// background task) and drops entries that haven't been read or written
/// within `IDLE_TTL`.
pub type UserCache = Arc<Mutex<UserCacheInner<SrpServer>>>;

/// Maximum idle time before an `SrpServer` cache entry is evicted. Tuned
/// so a typical "log in, play, disconnect, reconnect 10 min later" flow
/// still uses the cached entry (cheaper than a full re-handshake), while
/// stale entries from drive-by login attempts don't accumulate.
const IDLE_TTL: Duration = Duration::from_secs(30 * 60);

/// Minimum gap between consecutive prune passes — bounds the worst case
/// where a high-churn auth flood would otherwise walk the whole map on
/// every insert.
const PRUNE_INTERVAL: Duration = Duration::from_secs(60);

/// Generic so tests can drive the TTL logic with cheap value types instead
/// of standing up real SRP state per test case. Production uses
/// `UserCacheInner<SrpServer>` (see the `UserCache` type alias).
#[derive(Debug)]
pub struct UserCacheInner<V> {
    entries: AHashMap<String, (Instant, V)>,
    last_pruned: Instant,
    idle_ttl: Duration,
    prune_interval: Duration,
}

impl<V> Default for UserCacheInner<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V> UserCacheInner<V> {
    pub fn new() -> Self {
        Self::with_policy(IDLE_TTL, PRUNE_INTERVAL)
    }

    /// Used by tests to drive eviction without sleeping 30+ minutes of
    /// wall-clock time. Production callers should go through `new()`.
    pub fn with_policy(idle_ttl: Duration, prune_interval: Duration) -> Self {
        Self {
            entries: AHashMap::new(),
            last_pruned: Instant::now(),
            idle_ttl,
            prune_interval,
        }
    }

    pub fn get(&mut self, name: &str) -> Option<&V> {
        let (last_seen, v) = self.entries.get_mut(name)?;
        *last_seen = Instant::now();
        Some(v)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut V> {
        let (last_seen, v) = self.entries.get_mut(name)?;
        *last_seen = Instant::now();
        Some(v)
    }

    pub fn insert(&mut self, name: String, v: V) {
        self.maybe_prune();
        self.entries.insert(name, (Instant::now(), v));
    }

    /// Number of cached entries — for tests and observability.
    #[allow(dead_code, clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    fn maybe_prune(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_pruned) < self.prune_interval {
            return;
        }
        let before = self.entries.len();
        let cutoff = now - self.idle_ttl;
        self.entries.retain(|_, (last_seen, _)| *last_seen > cutoff);
        let evicted = before - self.entries.len();
        if evicted > 0 {
            debug!(
                "UserCache: evicted {evicted} idle entries ({} remain)",
                self.entries.len()
            );
        }
        self.last_pruned = now;
    }
}

/// World server `host:port` advertised back to the client in the realm
/// list. Resolved once at first use from `$WOW_REALM_ADDRESS`, falling
/// back to `localhost:8085` for single-machine development. Set this on
/// the deployed auth host (e.g. `WOW_REALM_ADDRESS=YOUR.SERVER.IP:8085`) so
/// remote clients are pointed at the world server they can actually
/// reach.
fn world_server_address() -> &'static str {
    static ADDR: OnceLock<String> = OnceLock::new();
    ADDR.get_or_init(|| {
        std::env::var("WOW_REALM_ADDRESS").unwrap_or_else(|_| "localhost:8085".to_string())
    })
}

/// Unwrap a `Result` or `Option`, logging the error and returning from the
/// surrounding `async fn _ -> ()` on the error/None case. Used in this file
/// to drop hostile/broken auth connections cleanly without taking the task
/// down via panic.
macro_rules! or_return {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(err) => {
                tracing::debug!("auth aborted: {err:?}");
                return;
            }
        }
    };
    ($e:expr, $msg:literal) => {
        match $e {
            Ok(v) => v,
            Err(err) => {
                tracing::debug!(concat!("auth aborted: ", $msg, ": {:?}"), err);
                return;
            }
        }
    };
    (opt: $e:expr, $msg:literal) => {
        match $e {
            Some(v) => v,
            None => {
                tracing::debug!(concat!("auth aborted: ", $msg));
                return;
            }
        }
    };
}

pub async fn auth(users: UserCache) {
    // Resolve and log the realm-list world address at startup so operators
    // can see at a glance what value clients are being told to connect to.
    tracing::info!(
        "auth: advertising world server address '{}' in realm list",
        world_server_address()
    );

    let listener = TcpListener::bind("0.0.0.0:3724")
        .await
        .expect("failed to bind auth listener on 0.0.0.0:3724");

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                warn!("auth accept failed: {e}; continuing");
                continue;
            }
        };

        tokio::spawn(handle(stream, users.clone()));
    }
}

async fn handle(mut stream: TcpStream, users: UserCache) {
    let opcode = tokio_read_initial_message(&mut stream).await;
    let opcode = match opcode {
        Ok(o) => o,
        Err(e) => {
            match e {
                ExpectedOpcodeError::Opcode(o) => warn!("invalid opcode {o}"),
                ExpectedOpcodeError::Parse(e) => warn!("parse error {:#?}", e),
                ExpectedOpcodeError::Io(i) => warn!("io error on initial read: {i}"),
            }
            return;
        }
    };

    match opcode {
        InitialMessage::Logon(l) => match l.protocol_version {
            ProtocolVersion::Two => login_version_2(stream, l, users).await,
            ProtocolVersion::Three => login_version_3(stream, l, users).await,
            ProtocolVersion::Eight => login_version_8(stream, l, users).await,
            _ => {}
        },
        InitialMessage::Reconnect(r) => match r.protocol_version {
            ProtocolVersion::Two => reconnect_version_2(stream, r, users).await,
            ProtocolVersion::Eight => reconnect_version_8(stream, r, users).await,
            _ => {}
        },
    }
}

async fn reconnect_version_8(
    mut stream: TcpStream,
    r: CMD_AUTH_RECONNECT_CHALLENGE_Client,
    users: UserCache,
) {
    use wow_login_messages::version_8::*;

    debug!("Reconnect version: {}", r.protocol_version);

    let server_reconnect_challenge_data = {
        let mut guard = or_return!(users.lock().map_err(|_| "users mutex poisoned"));
        let srp = or_return!(opt: guard.get(&r.account_name), "unknown account");
        *srp.reconnect_challenge_data()
    };

    or_return!(
        CMD_AUTH_RECONNECT_CHALLENGE_Server {
            result: CMD_AUTH_RECONNECT_CHALLENGE_Server_LoginResult::Success {
                challenge_data: server_reconnect_challenge_data,
                checksum_salt: [0; 16],
            },
        }
        .tokio_write(&mut stream)
        .await
    );

    let l = or_return!(
        tokio_expect_client_message::<CMD_AUTH_RECONNECT_PROOF_Client, _>(&mut stream).await
    );

    let success = {
        let mut guard = or_return!(users.lock().map_err(|_| "users mutex poisoned"));
        match guard.get_mut(&r.account_name) {
            None => false,
            Some(server) => server.verify_reconnection_attempt(l.proof_data, l.client_proof),
        }
    };

    if !success {
        or_return!(
            CMD_AUTH_RECONNECT_PROOF_Server {
                result: LoginResult::FailBanned,
            }
            .tokio_write(&mut stream)
            .await
        );
        return;
    }

    or_return!(
        CMD_AUTH_RECONNECT_PROOF_Server {
            result: LoginResult::Success,
        }
        .tokio_write(&mut stream)
        .await
    );

    print_version_8_realm_list(stream).await;
}

async fn reconnect_version_2(
    mut stream: TcpStream,
    r: CMD_AUTH_RECONNECT_CHALLENGE_Client,
    users: UserCache,
) {
    use wow_login_messages::version_2::*;

    debug!("Reconnect version: {}", r.protocol_version);

    let server_reconnect_challenge_data = {
        let mut guard = or_return!(users.lock().map_err(|_| "users mutex poisoned"));
        let srp = or_return!(opt: guard.get(&r.account_name), "unknown account");
        *srp.reconnect_challenge_data()
    };

    or_return!(
        CMD_AUTH_RECONNECT_CHALLENGE_Server {
            result: CMD_AUTH_RECONNECT_CHALLENGE_Server_LoginResult::Success {
                challenge_data: server_reconnect_challenge_data,
                checksum_salt: [0; 16],
            },
        }
        .tokio_write(&mut stream)
        .await
    );

    let l = or_return!(
        tokio_expect_client_message::<CMD_AUTH_RECONNECT_PROOF_Client, _>(&mut stream).await
    );

    let success = {
        let mut guard = or_return!(users.lock().map_err(|_| "users mutex poisoned"));
        match guard.get_mut(&r.account_name) {
            None => false,
            Some(server) => server.verify_reconnection_attempt(l.proof_data, l.client_proof),
        }
    };

    if !success {
        or_return!(
            CMD_AUTH_RECONNECT_PROOF_Server {
                result: LoginResult::FailBanned,
            }
            .tokio_write(&mut stream)
            .await
        );
        return;
    }

    or_return!(
        CMD_AUTH_RECONNECT_PROOF_Server {
            result: LoginResult::Success,
        }
        .tokio_write(&mut stream)
        .await
    );

    print_version_2_3_realm_list(stream).await;
}

async fn login_version_2(
    mut stream: TcpStream,
    l: CMD_AUTH_LOGON_CHALLENGE_Client,
    users: UserCache,
) {
    use wow_login_messages::version_2::*;

    debug!("Login version: {}", l.protocol_version);
    let p = get_proof(&l.account_name);

    let username = l.account_name;

    or_return!(
        CMD_AUTH_LOGON_CHALLENGE_Server {
            result: CMD_AUTH_LOGON_CHALLENGE_Server_LoginResult::Success {
                server_public_key: *p.server_public_key(),
                generator: vec![GENERATOR],
                large_safe_prime: LARGE_SAFE_PRIME_LITTLE_ENDIAN.into(),
                salt: *p.salt(),
                crc_salt: [0; 16],
            },
        }
        .tokio_write(&mut stream)
        .await
    );
    debug!("Sent Logon Challenge");

    let l = or_return!(
        tokio_expect_client_message::<CMD_AUTH_LOGON_PROOF_Client, _>(&mut stream).await
    );

    let client_public = or_return!(PublicKey::from_le_bytes(l.client_public_key));
    let (p, proof) = or_return!(p.into_server(client_public, l.client_proof));

    or_return!(
        CMD_AUTH_LOGON_PROOF_Server {
            result: CMD_AUTH_LOGON_PROOF_Server_LoginResult::Success {
                server_proof: proof,
                hardware_survey_id: 0,
            },
        }
        .tokio_write(&mut stream)
        .await
    );
    debug!("Sent Logon Proof");

    or_return!(users.lock().map_err(|_| "users mutex poisoned")).insert(username, p);

    print_version_2_3_realm_list(stream).await;
}

/// Builds an SRP proof for a username/password pair. Both fields are
/// normalized; if the username contains characters disallowed by
/// `NormalizedString` we substitute "invalid" to keep the proof flow alive
/// but make verification deterministically fail on the client side.
fn get_proof(username: &str) -> SrpProof {
    let fallback = || NormalizedString::new("invalid".to_string()).expect("'invalid' is normalizable");
    let username_norm = NormalizedString::new(username.to_string()).unwrap_or_else(|_| fallback());
    let password_norm =
        NormalizedString::new(username.to_string()).unwrap_or_else(|_| fallback());
    SrpVerifier::from_username_and_password(username_norm, password_norm).into_proof()
}

async fn login_version_3(
    mut stream: TcpStream,
    l: CMD_AUTH_LOGON_CHALLENGE_Client,
    users: UserCache,
) {
    use wow_login_messages::version_3::*;

    debug!("Login version: {}", l.protocol_version);
    let p = get_proof(&l.account_name);
    let username = l.account_name;

    or_return!(
        CMD_AUTH_LOGON_CHALLENGE_Server {
            result: CMD_AUTH_LOGON_CHALLENGE_Server_LoginResult::Success {
                server_public_key: *p.server_public_key(),
                generator: vec![GENERATOR],
                large_safe_prime: LARGE_SAFE_PRIME_LITTLE_ENDIAN.into(),
                salt: *p.salt(),
                crc_salt: [0; 16],
                security_flag: CMD_AUTH_LOGON_CHALLENGE_Server_SecurityFlag::None,
            },
        }
        .tokio_write(&mut stream)
        .await
    );
    debug!("Sent Logon Challenge");

    let l = or_return!(
        tokio_expect_client_message::<CMD_AUTH_LOGON_PROOF_Client, _>(&mut stream).await
    );

    let client_public = or_return!(PublicKey::from_le_bytes(l.client_public_key));
    let (p, proof) = or_return!(p.into_server(client_public, l.client_proof));

    or_return!(
        CMD_AUTH_LOGON_PROOF_Server {
            result: CMD_AUTH_LOGON_PROOF_Server_LoginResult::Success {
                server_proof: proof,
                hardware_survey_id: 0,
            },
        }
        .tokio_write(&mut stream)
        .await
    );
    debug!("Sent Logon Proof");

    or_return!(users.lock().map_err(|_| "users mutex poisoned"))
        .insert(username.to_string(), p);

    print_version_2_3_realm_list(stream).await;
}
async fn login_version_8(
    mut stream: TcpStream,
    l: CMD_AUTH_LOGON_CHALLENGE_Client,
    users: UserCache,
) {
    use wow_login_messages::version_8::*;

    debug!("Login version: {}", l.protocol_version);
    let p = get_proof(&l.account_name);
    let username = l.account_name;

    or_return!(
        CMD_AUTH_LOGON_CHALLENGE_Server {
            result: CMD_AUTH_LOGON_CHALLENGE_Server_LoginResult::Success {
                server_public_key: *p.server_public_key(),
                generator: vec![GENERATOR],
                large_safe_prime: LARGE_SAFE_PRIME_LITTLE_ENDIAN.into(),
                salt: *p.salt(),
                crc_salt: [0; 16],
                security_flag: CMD_AUTH_LOGON_CHALLENGE_Server_SecurityFlag::empty(),
            },
        }
        .tokio_write(&mut stream)
        .await
    );
    debug!("Sent Logon Challenge");

    let l = or_return!(
        tokio_expect_client_message::<CMD_AUTH_LOGON_PROOF_Client, _>(&mut stream).await
    );

    let client_public = or_return!(PublicKey::from_le_bytes(l.client_public_key));
    let (p, server_proof) = or_return!(p.into_server(client_public, l.client_proof));

    or_return!(
        CMD_AUTH_LOGON_PROOF_Server {
            result: CMD_AUTH_LOGON_PROOF_Server_LoginResult::Success {
                account_flag: AccountFlag::empty(),
                server_proof,
                hardware_survey_id: 0,
                unknown_flags: 0,
            },
        }
        .tokio_write(&mut stream)
        .await
    );
    debug!("Sent Logon Proof");

    or_return!(users.lock().map_err(|_| "users mutex poisoned"))
        .insert(username.to_string(), p);

    print_version_8_realm_list(stream).await;
}

async fn print_version_2_3_realm_list(mut stream: TcpStream) {
    use wow_login_messages::version_2::*;

    let addr = world_server_address().to_string();

    while (tokio_expect_client_message::<CMD_REALM_LIST_Client, _>(&mut stream).await).is_ok() {
        let msg = CMD_REALM_LIST_Server {
            realms: vec![Realm {
                realm_type: RealmType::PlayerVsEnvironment,
                flag: RealmFlag::empty(),
                name: "Location Realm".to_string(),
                address: addr.clone(),
                population: Default::default(),
                number_of_characters_on_realm: 0,
                category: Default::default(),
                realm_id: 0,
            }],
        };
        if msg.tokio_write(&mut stream).await.is_err() {
            return;
        }
        debug!("Sent Version 2/3 Realm List");
    }
}

async fn print_version_8_realm_list(mut stream: TcpStream) {
    use wow_login_messages::version_8::*;

    let addr = world_server_address().to_string();

    while (tokio_expect_client_message::<CMD_REALM_LIST_Client, _>(&mut stream).await).is_ok() {
        let mut realms = Vec::new();
        for i in 0..9 {
            realms.push(Realm {
                realm_type: RealmType::PlayerVsEnvironment,
                locked: false,
                flag: Default::default(),
                name: i.to_string(),
                address: addr.clone(),
                population: Default::default(),
                number_of_characters_on_realm: i,
                category: RealmCategory::One,
                realm_id: i,
            })
        }

        let msg = CMD_REALM_LIST_Server { realms };
        if msg.tokio_write(&mut stream).await.is_err() {
            return;
        }
        debug!("Sent Version 8 Realm List");
    }
}

#[cfg(test)]
mod user_cache_tests {
    use super::*;
    use std::thread::sleep;

    // Use String as the value type so the test doesn't need a real
    // SrpServer — UserCacheInner is generic exactly so the TTL logic can
    // be exercised with cheap stand-ins.
    fn cache(idle_ms: u64, prune_ms: u64) -> UserCacheInner<&'static str> {
        UserCacheInner::with_policy(
            Duration::from_millis(idle_ms),
            Duration::from_millis(prune_ms),
        )
    }

    #[test]
    fn insert_and_get_roundtrips() {
        let mut c = cache(60_000, 60_000);
        c.insert("alice".into(), "session-A");
        assert_eq!(c.get("alice").copied(), Some("session-A"));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn insert_replaces_existing_entry() {
        let mut c = cache(60_000, 60_000);
        c.insert("alice".into(), "old");
        c.insert("alice".into(), "new");
        assert_eq!(c.len(), 1);
        assert_eq!(c.get("alice").copied(), Some("new"));
    }

    #[test]
    fn prune_evicts_idle_entries() {
        // Short TTL so the test runs in real time. Setting prune_interval
        // to zero forces every insert to trigger a prune pass.
        let mut c = cache(50, 0);
        c.insert("alice".into(), "A");
        c.insert("bob".into(), "B");
        assert_eq!(c.len(), 2);

        // Wait past the TTL, then poke a new insert to trigger a prune.
        sleep(Duration::from_millis(80));
        c.insert("carol".into(), "C");

        // alice and bob were idle past the TTL — evicted. carol is fresh.
        assert_eq!(c.len(), 1);
        assert!(c.get("alice").is_none());
        assert!(c.get("bob").is_none());
        assert_eq!(c.get("carol").copied(), Some("C"));
    }

    #[test]
    fn get_refreshes_last_seen() {
        // get() touches the entry's last_seen, so an entry that's been
        // read recently survives a prune even if it was inserted long ago.
        let mut c = cache(50, 0);
        c.insert("alice".into(), "A");
        sleep(Duration::from_millis(30));
        // Refresh — last_seen is bumped to now.
        let _ = c.get("alice");
        sleep(Duration::from_millis(30));
        // Total elapsed since insert is ~60ms (past TTL), but only ~30ms
        // since the get() refresh — alice should survive.
        c.insert("bob".into(), "B"); // triggers prune
        assert_eq!(c.len(), 2);
        assert!(c.get("alice").is_some(), "recently-read entry must survive prune");
    }

    #[test]
    fn prune_interval_throttles_passes() {
        // With a long prune_interval, eviction only happens at the next
        // permitted prune even if TTL has elapsed. Confirms we're not
        // walking the entire map on every insert under flood conditions.
        let mut c = cache(10, 10_000); // TTL=10ms, prune-throttle=10s
        c.insert("alice".into(), "A");
        sleep(Duration::from_millis(30));
        c.insert("bob".into(), "B"); // first insert post-construction; throttle hasn't elapsed
        // alice is past TTL but prune was throttled — both still present.
        assert_eq!(c.len(), 2);
    }
}
