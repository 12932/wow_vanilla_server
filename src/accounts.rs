//! Accounts: a name, its SRP salt and verifier, a GM level and a ban. They
//! live in the character store's file (table `accounts`), on a connection of
//! their own, so the login server and the world share them.
//!
//! Who may log in is policy: `auto` makes an account at its first login
//! with its name as its password (development and load tests); `existing`
//! admits only accounts already made.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use wow_srp::normalized_string::NormalizedString;
use wow_srp::server::SrpVerifier;

/// GM levels.
pub const PLAYER: u8 = 0;
pub const MODERATOR: u8 = 1;
pub const GAME_MASTER: u8 = 2;
pub const ADMINISTRATOR: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Creation {
    /// An unknown name gets an account, with the name as its password.
    #[default]
    Auto,
    /// Only accounts already made may log in.
    Existing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub name: String,
    pub salt: [u8; 32],
    pub verifier: [u8; 32],
    pub gm_level: u8,
    /// Unix seconds; 0 not banned, -1 for good.
    pub banned_until: i64,
    pub ban_reason: String,
}

impl Account {
    pub fn banned(&self, now: i64) -> bool {
        self.banned_until == -1 || self.banned_until > now
    }
}

/// Why a login is refused.
#[derive(Debug, Clone, PartialEq)]
pub enum Refused {
    Unknown,
    Banned,
    BadName,
}

pub struct Accounts {
    conn: Mutex<Connection>,
    creation: Creation,
    everyone_gm: bool,
    gm: ahash::AHashMap<String, u8>,
}

static ACCOUNTS: OnceLock<Accounts> = OnceLock::new();

/// The accounts, once installed at start-up.
pub fn get() -> Option<&'static Accounts> {
    ACCOUNTS.get()
}

pub fn install(a: Accounts) {
    let _ = ACCOUNTS.set(a);
}

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS accounts (
    name TEXT PRIMARY KEY COLLATE NOCASE,
    salt BLOB NOT NULL,
    verifier BLOB NOT NULL,
    gm_level INTEGER NOT NULL DEFAULT 0,
    banned_until INTEGER NOT NULL DEFAULT 0,
    ban_reason TEXT NOT NULL DEFAULT '',
    created INTEGER NOT NULL
)";

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn normalized(name: &str) -> Option<NormalizedString> {
    NormalizedString::new(name.to_string()).ok()
}

impl Accounts {
    pub fn open(
        path: &Path,
        creation: Creation,
        everyone_gm: bool,
        gm: ahash::AHashMap<String, u8>,
    ) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))?;
        info!(
            "accounts: {count} in {}, creation {creation:?}{}",
            path.display(),
            if everyone_gm { ", everyone a GM" } else { "" }
        );
        Ok(Self {
            conn: Mutex::new(conn),
            creation,
            everyone_gm,
            gm: gm.into_iter().map(|(k, v)| (k.to_uppercase(), v)).collect(),
        })
    }

    /// An in-memory set, for tests.
    pub fn in_memory(creation: Creation, everyone_gm: bool) -> Self {
        let conn = Connection::open_in_memory().expect("an in-memory database");
        conn.execute_batch(SCHEMA).expect("the accounts table");
        Self {
            conn: Mutex::new(conn),
            creation,
            everyone_gm,
            gm: ahash::AHashMap::new(),
        }
    }

    /// The connection, locked. A panic while it was held leaves it usable
    /// (SQLite rolls back what was unfinished); a poisoned lock would
    /// instead refuse every login until a restart.
    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn find(&self, name: &str) -> Option<Account> {
        let conn = self.lock();
        conn.query_row(
            "SELECT name, salt, verifier, gm_level, banned_until, ban_reason FROM accounts WHERE name = ?1",
            params![name],
            |r| {
                let salt: Vec<u8> = r.get(1)?;
                let verifier: Vec<u8> = r.get(2)?;
                Ok(Account {
                    name: r.get(0)?,
                    salt: salt.try_into().unwrap_or([0; 32]),
                    verifier: verifier.try_into().unwrap_or([0; 32]),
                    gm_level: r.get(3)?,
                    banned_until: r.get(4)?,
                    ban_reason: r.get(5)?,
                })
            },
        )
        .optional()
        .ok()
        .flatten()
    }

    /// Make an account, or set its password.
    pub fn set_password(&self, name: &str, password: &str) -> Result<(), String> {
        let (Some(n), Some(p)) = (normalized(name), normalized(password)) else {
            return Err("names and passwords are letters and digits".into());
        };
        let v = SrpVerifier::from_username_and_password(n, p);
        let conn = self.lock();
        conn.execute(
            "INSERT INTO accounts (name, salt, verifier, created) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name) DO UPDATE SET salt = ?2, verifier = ?3",
            params![
                name.to_uppercase(),
                v.salt().to_vec(),
                v.password_verifier().to_vec(),
                now()
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Whether `password` is the account's: a login's proofs worked out
    /// on both sides, the server's from its verifier.
    pub fn check_password(&self, name: &str, password: &str) -> bool {
        let (Some(n), Some(p), Some(account)) =
            (normalized(name), normalized(password), self.find(name))
        else {
            return false;
        };
        let proof =
            SrpVerifier::from_database_values(n.clone(), account.verifier, account.salt).into_proof();
        let Ok(server_key) = wow_srp::PublicKey::from_le_bytes(*proof.server_public_key()) else {
            return false;
        };
        let challenge = wow_srp::client::SrpClientUser::new(n, p).into_challenge(
            wow_srp::GENERATOR,
            wow_srp::LARGE_SAFE_PRIME_LITTLE_ENDIAN,
            server_key,
            *proof.salt(),
        );
        let Ok(client_key) = wow_srp::PublicKey::from_le_bytes(*challenge.client_public_key())
        else {
            return false;
        };
        proof
            .into_server(client_key, *challenge.client_proof())
            .is_ok()
    }

    pub fn set_gm_level(&self, name: &str, level: u8) -> bool {
        self.lock()
            .execute(
                "UPDATE accounts SET gm_level = ?2 WHERE name = ?1",
                params![name, level.min(ADMINISTRATOR)],
            )
            .is_ok_and(|n| n > 0)
    }

    /// Ban an account until `until` (Unix seconds, or -1 for good); 0 lifts it.
    pub fn ban(&self, name: &str, until: i64, reason: &str) -> bool {
        self.lock()
            .execute(
                "UPDATE accounts SET banned_until = ?2, ban_reason = ?3 WHERE name = ?1",
                params![name, until, reason],
            )
            .is_ok_and(|n| n > 0)
    }

    /// The account's SRP verifier for a login, made first if policy
    /// allows.
    pub fn login(&self, name: &str) -> Result<SrpVerifier, Refused> {
        let n = normalized(name).ok_or(Refused::BadName)?;
        let account = match self.find(name) {
            Some(a) => a,
            None if self.creation == Creation::Auto => {
                if let Err(e) = self.set_password(name, name) {
                    warn!("making account {name}: {e}");
                    return Err(Refused::BadName);
                }
                self.find(name).ok_or(Refused::Unknown)?
            }
            None => return Err(Refused::Unknown),
        };
        if account.banned(now()) {
            return Err(Refused::Banned);
        }
        Ok(SrpVerifier::from_database_values(
            n,
            account.verifier,
            account.salt,
        ))
    }

    /// The account's GM level: the store's, the config's, or every
    /// account's.
    pub fn gm_level(&self, name: &str) -> u8 {
        if self.everyone_gm {
            return ADMINISTRATOR;
        }
        let stored = self.find(name).map_or(PLAYER, |a| a.gm_level);
        stored.max(self.gm.get(&name.to_uppercase()).copied().unwrap_or(PLAYER))
    }

    pub fn banned(&self, name: &str) -> bool {
        self.find(name).is_some_and(|a| a.banned(now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_makes_an_account_and_existing_refuses_strangers() {
        let auto = Accounts::in_memory(Creation::Auto, false);
        assert!(auto.login("NEWBIE").is_ok());
        assert!(auto.find("newbie").is_some(), "names are case-blind");
        let strict = Accounts::in_memory(Creation::Existing, false);
        assert_eq!(strict.login("STRANGER").err(), Some(Refused::Unknown));
        strict.set_password("KNOWN", "SECRET").unwrap();
        assert!(strict.login("KNOWN").is_ok());
    }

    #[test]
    fn a_panic_while_the_connection_was_held_doesnt_lock_everyone_out() {
        let a = Accounts::in_memory(Creation::Auto, false);
        a.set_password("ALICE", "PW").unwrap();
        let poisoned = std::thread::scope(|s| {
            s.spawn(|| {
                let _held = a.conn.lock();
                panic!("a panic while the accounts are held");
            })
            .join()
        });
        assert!(poisoned.is_err() && a.conn.is_poisoned());
        assert!(a.find("ALICE").is_some());
        assert!(a.login("BOB").is_ok(), "a new account is still made");
    }

    #[test]
    fn a_gm_command_finds_an_account_in_any_case() {
        let a = Accounts::in_memory(Creation::Auto, false);
        a.set_password("alice", "pw").unwrap();
        assert!(a.ban("alice", -1, "testing"), "banned in lower case");
        assert_eq!(a.login("Alice").err(), Some(Refused::Banned));
        assert!(a.ban("aLiCe", 0, ""), "unbanned in mixed case");
        assert!(a.login("ALICE").is_ok());
        assert!(a.set_gm_level("alice", GAME_MASTER));
        assert_eq!(a.gm_level("Alice"), GAME_MASTER);
    }

    #[test]
    fn a_ban_holds_until_it_ends_and_gm_levels_add_up() {
        let a = Accounts::in_memory(Creation::Auto, false);
        a.set_password("ALICE", "PW").unwrap();
        assert!(a.ban("ALICE", -1, "testing"));
        assert_eq!(a.login("ALICE").err(), Some(Refused::Banned));
        assert!(a.ban("ALICE", 0, ""));
        assert!(a.login("ALICE").is_ok());
        assert_eq!(a.gm_level("ALICE"), PLAYER);
        assert!(a.set_gm_level("ALICE", GAME_MASTER));
        assert_eq!(a.gm_level("ALICE"), GAME_MASTER);
        assert_eq!(
            Accounts::in_memory(Creation::Auto, true).gm_level("ANYONE"),
            ADMINISTRATOR
        );
    }
}
