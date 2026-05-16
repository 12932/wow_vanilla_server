//! World handshake + character bootstrap.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use rand::{RngExt, SeedableRng};
use rand::rngs::StdRng;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::time::timeout;
use wow_srp::normalized_string::NormalizedString;
use wow_srp::vanilla_header::{DecrypterHalf, EncrypterHalf, ProofSeed};
use wow_world_base::vanilla::RaceClass;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::ClientMessage as _;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::{
    CMSG_AUTH_SESSION, CMSG_CHAR_CREATE, CMSG_CHAR_ENUM, CMSG_PLAYER_LOGIN, Gender,
    SMSG_AUTH_RESPONSE_WorldResult, WorldResult,
};

/// Vanilla 1.12.2 client build, matches `worker::auth::CLIENT_BUILD`.
const CLIENT_BUILD: u32 = 5875;

/// Per-step timeout. World handshake should be near-instant — anything past a
/// few seconds means something is wedged and the bot should fail fast so the
/// orchestrator can mark a failure instead of leaking task time.
const STEP_TIMEOUT: Duration = Duration::from_secs(10);

pub struct WorldSession {
    pub reader: OwnedReadHalf,
    pub writer: OwnedWriteHalf,
    pub encrypter: EncrypterHalf,
    pub decrypter: DecrypterHalf,
    #[allow(dead_code)] // kept for diagnostics; the bot doesn't need it post-login
    pub character_guid: Guid,
}

pub async fn establish(
    world_addr: &str,
    username: &str,
    session_key: [u8; 40],
) -> std::io::Result<WorldSession> {
    let stream = TcpStream::connect(world_addr).await?;
    stream.set_nodelay(true)?;
    let (mut reader, mut writer) = stream.into_split();

    // Step 1: unencrypted SMSG_AUTH_CHALLENGE → grab server seed.
    let server_seed = match timeout(
        STEP_TIMEOUT,
        ServerOpcodeMessage::tokio_read_unencrypted(&mut reader),
    )
    .await
    .map_err(|_| std::io::Error::other("world: SMSG_AUTH_CHALLENGE timed out"))?
    .map_err(|e| std::io::Error::other(format!("world: read AUTH_CHALLENGE: {e:?}")))?
    {
        ServerOpcodeMessage::SMSG_AUTH_CHALLENGE(c) => c.server_seed,
        other => {
            return Err(std::io::Error::other(format!(
                "world: expected SMSG_AUTH_CHALLENGE, got {other:?}"
            )));
        }
    };

    // Step 2: compute proof + initialise header crypto for our side.
    let username_norm = NormalizedString::new(username.to_string())
        .map_err(|e| std::io::Error::other(format!("world: username normalize: {e:?}")))?;
    let client_seed = ProofSeed::new();
    let client_seed_value = client_seed.seed();
    let (client_proof, header_crypto) =
        client_seed.into_client_header_crypto(&username_norm, session_key, server_seed);
    let (encrypter, decrypter) = header_crypto.split();

    // Step 3: send CMSG_AUTH_SESSION unencrypted.
    let session = CMSG_AUTH_SESSION {
        build: CLIENT_BUILD,
        server_id: 0,
        username: username.to_string(),
        client_seed: client_seed_value,
        client_proof,
        addon_info: Vec::new(),
    };
    session
        .tokio_write_unencrypted_client(&mut writer)
        .await
        .map_err(|e| std::io::Error::other(format!("world: write AUTH_SESSION: {e}")))?;

    let mut state = HandshakeState {
        reader,
        writer,
        encrypter,
        decrypter,
    };

    // Step 4: read SMSG_AUTH_RESPONSE (encrypted from here on).
    match state.read_server_encrypted().await? {
        ServerOpcodeMessage::SMSG_AUTH_RESPONSE(c) => match c.result {
            SMSG_AUTH_RESPONSE_WorldResult::AuthOk { .. } => {}
            other => {
                return Err(std::io::Error::other(format!(
                    "world: AUTH_RESPONSE not OK: {other:?}"
                )));
            }
        },
        other => {
            return Err(std::io::Error::other(format!(
                "world: expected SMSG_AUTH_RESPONSE, got {other:?}"
            )));
        }
    }

    // Step 5: enumerate characters; create one if none exist.
    let character_guid = ensure_character(&mut state, username).await?;

    // Step 6: log into the character.
    let player_login = CMSG_PLAYER_LOGIN {
        guid: character_guid,
    };
    state
        .write_client_encrypted(&player_login)
        .await
        .map_err(|e| std::io::Error::other(format!("world: write PLAYER_LOGIN: {e}")))?;

    Ok(WorldSession {
        reader: state.reader,
        writer: state.writer,
        encrypter: state.encrypter,
        decrypter: state.decrypter,
        character_guid,
    })
}

async fn ensure_character(
    state: &mut HandshakeState,
    username: &str,
) -> std::io::Result<Guid> {
    state
        .write_client_encrypted(&CMSG_CHAR_ENUM {})
        .await
        .map_err(|e| std::io::Error::other(format!("world: write CHAR_ENUM: {e}")))?;

    match state.read_server_encrypted().await? {
        ServerOpcodeMessage::SMSG_CHAR_ENUM(enumeration) => {
            if let Some(c) = enumeration.characters.first() {
                return Ok(c.guid);
            }
        }
        other => {
            return Err(std::io::Error::other(format!(
                "world: expected SMSG_CHAR_ENUM, got {other:?}"
            )));
        }
    }

    // No characters — create one. Profile is deterministic per username so
    // a restarted bot picks up the same character it had before.
    let profile = profile_for(username);
    let create = CMSG_CHAR_CREATE {
        name: profile.name.clone(),
        race: profile.race_class.race().into(),
        class: profile.race_class.class(),
        gender: profile.gender,
        skin_color: profile.skin_color,
        face: profile.face,
        hair_style: profile.hair_style,
        hair_color: profile.hair_color,
        facial_hair: profile.facial_hair,
    };
    tracing::debug!(
        "char create: username={} name={} race_class={:?} gender={:?}",
        username,
        profile.name,
        profile.race_class,
        profile.gender,
    );
    state
        .write_client_encrypted(&create)
        .await
        .map_err(|e| std::io::Error::other(format!("world: write CHAR_CREATE: {e}")))?;

    match state.read_server_encrypted().await? {
        ServerOpcodeMessage::SMSG_CHAR_CREATE(c) => {
            if c.result != WorldResult::CharCreateSuccess {
                return Err(std::io::Error::other(format!(
                    "world: CHAR_CREATE failed: {:?}",
                    c.result
                )));
            }
        }
        other => {
            return Err(std::io::Error::other(format!(
                "world: expected SMSG_CHAR_CREATE, got {other:?}"
            )));
        }
    }

    state
        .write_client_encrypted(&CMSG_CHAR_ENUM {})
        .await
        .map_err(|e| std::io::Error::other(format!("world: write CHAR_ENUM (post-create): {e}")))?;

    match state.read_server_encrypted().await? {
        ServerOpcodeMessage::SMSG_CHAR_ENUM(enumeration) => {
            if let Some(c) = enumeration.characters.first() {
                Ok(c.guid)
            } else {
                Err(std::io::Error::other("world: char list empty after create"))
            }
        }
        other => Err(std::io::Error::other(format!(
            "world: expected SMSG_CHAR_ENUM, got {other:?}"
        ))),
    }
}

/// Deterministic-per-username character profile. The same username always
/// produces the same name, race/class, gender, and looks — so when a bot
/// reconnects after a restart it lands on the character it already created
/// (and CHAR_ENUM returns it directly, skipping CHAR_CREATE).
struct CharProfile {
    name: String,
    race_class: RaceClass,
    gender: Gender,
    skin_color: u8,
    face: u8,
    hair_style: u8,
    hair_color: u8,
    facial_hair: u8,
}

/// Every valid vanilla race/class combination. Pulled inline so we can index
/// uniformly; the enum has no `iter()` and we don't want to depend on `strum`.
const ALL_RACE_CLASSES: [RaceClass; 40] = [
    RaceClass::DwarfHunter,
    RaceClass::DwarfPaladin,
    RaceClass::DwarfPriest,
    RaceClass::DwarfRogue,
    RaceClass::DwarfWarrior,
    RaceClass::GnomeMage,
    RaceClass::GnomeRogue,
    RaceClass::GnomeWarlock,
    RaceClass::GnomeWarrior,
    RaceClass::HumanMage,
    RaceClass::HumanPaladin,
    RaceClass::HumanPriest,
    RaceClass::HumanRogue,
    RaceClass::HumanWarlock,
    RaceClass::HumanWarrior,
    RaceClass::NightElfDruid,
    RaceClass::NightElfHunter,
    RaceClass::NightElfPriest,
    RaceClass::NightElfRogue,
    RaceClass::NightElfWarrior,
    RaceClass::OrcHunter,
    RaceClass::OrcRogue,
    RaceClass::OrcShaman,
    RaceClass::OrcWarlock,
    RaceClass::OrcWarrior,
    RaceClass::TaurenDruid,
    RaceClass::TaurenHunter,
    RaceClass::TaurenShaman,
    RaceClass::TaurenWarrior,
    RaceClass::TrollHunter,
    RaceClass::TrollMage,
    RaceClass::TrollPriest,
    RaceClass::TrollRogue,
    RaceClass::TrollShaman,
    RaceClass::TrollWarrior,
    RaceClass::UndeadMage,
    RaceClass::UndeadPriest,
    RaceClass::UndeadRogue,
    RaceClass::UndeadWarlock,
    RaceClass::UndeadWarrior,
];

/// Up to 4 letters of the local hostname, lowercased. Used as the
/// human-readable prefix on every bot's character name so a fleet's bots
/// are immediately attributable to the host that spawned them.
///
/// Sourcing: `LOADTEST_NAME_PREFIX` env var overrides everything (use it
/// when you want a recognizable tag like `RAID` regardless of hostname).
/// Otherwise the `gethostname` crate calls the platform's syscall
/// (`GetComputerNameW` / `gethostname(2)`) — works on Windows, Linux, and
/// macOS without relying on `$HOSTNAME` being exported to child processes.
fn host_prefix() -> String {
    let raw = if let Ok(override_prefix) = std::env::var("LOADTEST_NAME_PREFIX") {
        override_prefix
    } else {
        gethostname::gethostname().to_string_lossy().into_owned()
    };
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .take(4)
        .collect::<String>()
        .to_lowercase();
    if cleaned.len() < 2 {
        "bot".to_string()
    } else {
        cleaned
    }
}

fn seed_for(username: &str) -> u64 {
    let mut h = DefaultHasher::new();
    username.hash(&mut h);
    h.finish()
}

fn profile_for(username: &str) -> CharProfile {
    let mut rng = StdRng::seed_from_u64(seed_for(username));

    // Name: hostname prefix (first cap, rest lower) + 5 random lowercase
    // letters. 26^5 = ~12M distinct suffixes per host prefix — collisions
    // within a fleet are vanishingly rare.
    let prefix = host_prefix();
    let mut name = String::with_capacity(12);
    let mut chars = prefix.chars();
    if let Some(c) = chars.next() {
        name.push(c.to_ascii_uppercase());
    }
    for c in chars {
        name.push(c.to_ascii_lowercase());
    }
    for _ in 0..5 {
        let n: u8 = rng.random_range(0..26);
        name.push((b'a' + n) as char);
    }
    name.truncate(12);

    let race_class = ALL_RACE_CLASSES[rng.random_range(0..ALL_RACE_CLASSES.len())];
    let gender = if rng.random_bool(0.5) {
        Gender::Male
    } else {
        Gender::Female
    };

    // Look ranges are intentionally narrow — the wider the value the more
    // likely we hit a slot the client has no asset for (which renders as a
    // default). Values 0..N here cover the canonical face/hair sets every
    // race has.
    CharProfile {
        name,
        race_class,
        gender,
        skin_color: rng.random_range(0..6),
        face: rng.random_range(0..4),
        hair_style: rng.random_range(0..8),
        hair_color: rng.random_range(0..6),
        facial_hair: rng.random_range(0..6),
    }
}

/// Helper state passed around during handshake so each step can read/write
/// encrypted frames without juggling the four halves manually.
struct HandshakeState {
    reader: OwnedReadHalf,
    writer: OwnedWriteHalf,
    encrypter: EncrypterHalf,
    decrypter: DecrypterHalf,
}

impl HandshakeState {
    async fn read_server_encrypted(&mut self) -> std::io::Result<ServerOpcodeMessage> {
        timeout(
            STEP_TIMEOUT,
            ServerOpcodeMessage::tokio_read_encrypted(&mut self.reader, &mut self.decrypter),
        )
        .await
        .map_err(|_| std::io::Error::other("world: read timed out"))?
        .map_err(|e| std::io::Error::other(format!("world: decode server message: {e:?}")))
    }

    async fn write_client_encrypted<M>(&mut self, msg: &M) -> std::io::Result<()>
    where
        M: wow_world_messages::vanilla::ClientMessage + Sync,
    {
        msg.tokio_write_encrypted_client(&mut self.writer, &mut self.encrypter)
            .await?;
        self.writer.flush().await
    }
}
