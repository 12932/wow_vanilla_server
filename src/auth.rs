use ahash::AHashMap;
use std::sync::{Arc, Mutex};
use std::sync::OnceLock;
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

pub async fn auth(users: Arc<Mutex<AHashMap<String, SrpServer>>>) {
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

async fn handle(mut stream: TcpStream, users: Arc<Mutex<AHashMap<String, SrpServer>>>) {
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
    users: Arc<Mutex<AHashMap<String, SrpServer>>>,
) {
    use wow_login_messages::version_8::*;

    debug!("Reconnect version: {}", r.protocol_version);

    let server_reconnect_challenge_data = {
        let guard = or_return!(users.lock().map_err(|_| "users mutex poisoned"));
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
    users: Arc<Mutex<AHashMap<String, SrpServer>>>,
) {
    use wow_login_messages::version_2::*;

    debug!("Reconnect version: {}", r.protocol_version);

    let server_reconnect_challenge_data = {
        let guard = or_return!(users.lock().map_err(|_| "users mutex poisoned"));
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
    users: Arc<Mutex<AHashMap<String, SrpServer>>>,
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
    users: Arc<Mutex<AHashMap<String, SrpServer>>>,
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
    users: Arc<Mutex<AHashMap<String, SrpServer>>>,
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
