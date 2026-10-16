//! World handshake + character bootstrap.
//!
//! Every step waits for the packet it needs and skips anything else a server
//! sends meanwhile: real servers interleave packets, and wow_messages has
//! already consumed the body of anything it doesn't model or can't parse.

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
use wow_world_messages::errors::ExpectedOpcodeError;
use wow_world_messages::vanilla::{
    CMSG_AUTH_SESSION, CMSG_CHAR_CREATE, CMSG_CHAR_ENUM, CMSG_PLAYER_LOGIN,
    CMSG_SET_ACTIVE_MOVER, Gender, Map, SMSG_AUTH_RESPONSE_WorldResult, Vector3d, WorldResult,
};

/// The 1.12.1 client's build, as `worker::auth::CLIENT_BUILD`.
const CLIENT_BUILD: u32 = 5875;

/// How many times character creation retries with a longer name when the
/// server says the name is taken.
const NAME_RETRIES: u32 = 3;

/// Per-step timeout. World handshake should be near-instant — anything past a
/// few seconds means something is wedged and the bot should fail fast so the
/// orchestrator can mark a failure instead of leaking task time.
const STEP_TIMEOUT: Duration = Duration::from_secs(10);

pub struct WorldSession {
    pub reader: OwnedReadHalf,
    pub writer: OwnedWriteHalf,
    pub encrypter: EncrypterHalf,
    pub decrypter: DecrypterHalf,
    pub character_guid: Guid,
    /// Where the server put the character, from `SMSG_LOGIN_VERIFY_WORLD`.
    pub map: Map,
    pub position: Vector3d,
    pub orientation: f32,
    /// Packets the handshake skipped on its way, for the metrics.
    pub skipped: u64,
    /// The server started the intro cinematic during the handshake; the
    /// bot ends it (`CMSG_COMPLETE_CINEMATIC`) once it is in the world.
    pub cinematic: bool,
}

/// Connect to the world server and log `username` into its character,
/// creating one if the account has none. `slot` makes the character's name
/// unique. Returns once the server confirms the character is in the world
/// (`SMSG_LOGIN_VERIFY_WORLD`) and the bot has claimed it as its active mover.
pub async fn establish(
    world_addr: &str,
    username: &str,
    slot: u32,
    session_key: [u8; 40],
) -> std::io::Result<WorldSession> {
    let stream = timeout(STEP_TIMEOUT, TcpStream::connect(world_addr))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "world: connect timed out"))??;
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
        skipped: 0,
        cinematic: false,
    };

    // Step 4: SMSG_AUTH_RESPONSE (encrypted from here on). A login queue is
    // waiting, not failure; the caller's login deadline bounds it.
    let mut queued = false;
    loop {
        let step = if queued { QUEUE_TIMEOUT } else { STEP_TIMEOUT };
        let result = state
            .read_until("SMSG_AUTH_RESPONSE", step, |m| match m {
                ServerOpcodeMessage::SMSG_AUTH_RESPONSE(c) => Some(c.result),
                _ => None,
            })
            .await?;
        match result {
            SMSG_AUTH_RESPONSE_WorldResult::AuthOk { .. } => break,
            SMSG_AUTH_RESPONSE_WorldResult::AuthWaitQueue { .. } => queued = true,
            other => {
                return Err(std::io::Error::other(format!(
                    "world: AUTH_RESPONSE not OK: {other:?}"
                )));
            }
        }
    }

    // Step 5: enumerate characters; create one if none exist.
    let character_guid = ensure_character(&mut state, username, slot).await?;

    // Step 6: log into the character, and wait until the server has put it
    // in the world.
    state
        .write_client_encrypted(&CMSG_PLAYER_LOGIN {
            guid: character_guid,
        })
        .await
        .map_err(|e| std::io::Error::other(format!("world: write PLAYER_LOGIN: {e}")))?;
    let verified = state
        .read_until("SMSG_LOGIN_VERIFY_WORLD", STEP_TIMEOUT, |m| match m {
            ServerOpcodeMessage::SMSG_LOGIN_VERIFY_WORLD(v) => Some(Ok(v)),
            ServerOpcodeMessage::SMSG_CHARACTER_LOGIN_FAILED(f) => Some(Err(f.result)),
            _ => None,
        })
        .await?
        .map_err(|r| std::io::Error::other(format!("world: character login failed: {r:?}")))?;

    // Step 7: claim the character as the client's active mover, as the real
    // client does on entering the world. Some servers (Thistle Tea) ignore
    // movement until then.
    state
        .write_client_encrypted(&CMSG_SET_ACTIVE_MOVER {
            guid: character_guid,
        })
        .await
        .map_err(|e| std::io::Error::other(format!("world: write SET_ACTIVE_MOVER: {e}")))?;

    Ok(WorldSession {
        reader: state.reader,
        writer: state.writer,
        encrypter: state.encrypter,
        decrypter: state.decrypter,
        character_guid,
        map: verified.map,
        position: verified.position,
        orientation: verified.orientation,
        skipped: state.skipped,
        cinematic: state.cinematic,
    })
}

async fn enumerate(state: &mut HandshakeState) -> std::io::Result<Option<Guid>> {
    state
        .write_client_encrypted(&CMSG_CHAR_ENUM {})
        .await
        .map_err(|e| std::io::Error::other(format!("world: write CHAR_ENUM: {e}")))?;
    state
        .read_until("SMSG_CHAR_ENUM", STEP_TIMEOUT, |m| match m {
            ServerOpcodeMessage::SMSG_CHAR_ENUM(e) => Some(e.characters.first().map(|c| c.guid)),
            _ => None,
        })
        .await
}

async fn ensure_character(
    state: &mut HandshakeState,
    username: &str,
    slot: u32,
) -> std::io::Result<Guid> {
    if let Some(guid) = enumerate(state).await? {
        return Ok(guid);
    }

    // No characters: create one. The profile is deterministic per username
    // and slot, so a restarted bot gets the same character.
    let profile = profile_for(username, slot);
    let mut name = profile.name.clone();
    for attempt in 0..=NAME_RETRIES {
        let create = CMSG_CHAR_CREATE {
            name: name.clone(),
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
            "char create: username={username} name={name} race_class={:?} gender={:?}",
            profile.race_class,
            profile.gender,
        );
        state
            .write_client_encrypted(&create)
            .await
            .map_err(|e| std::io::Error::other(format!("world: write CHAR_CREATE: {e}")))?;
        let result = state
            .read_until("SMSG_CHAR_CREATE", STEP_TIMEOUT, |m| match m {
                ServerOpcodeMessage::SMSG_CHAR_CREATE(c) => Some(c.result),
                _ => None,
            })
            .await?;
        match result {
            WorldResult::CharCreateSuccess => break,
            // Another account has the name: lengthen it and try again.
            WorldResult::CharCreateNameInUse if attempt < NAME_RETRIES => {
                name.push(letter(u64::from(attempt) + u64::from(slot)));
            }
            other => {
                return Err(std::io::Error::other(format!(
                    "world: CHAR_CREATE {name} failed: {other:?}"
                )));
            }
        }
    }

    enumerate(state)
        .await?
        .ok_or_else(|| std::io::Error::other("world: char list empty after create"))
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

/// The `n % 26`th lower-case letter.
fn letter(n: u64) -> char {
    char::from(b'a' + u8::try_from(n % 26).unwrap_or(0))
}

/// A character name for `slot`: the host prefix (capitalised), then the slot
/// in base 26 as five letters. Names are letters only, as servers require,
/// and two slots never share one, so with each worker on its own slot range
/// a fleet's names don't collide.
fn character_name(prefix: &str, slot: u32) -> String {
    let mut name = String::with_capacity(12);
    let mut chars = prefix.chars();
    if let Some(c) = chars.next() {
        name.push(c.to_ascii_uppercase());
    }
    for c in chars {
        name.push(c.to_ascii_lowercase());
    }
    let mut digits = [0u64; 5];
    let mut rest = u64::from(slot);
    for d in digits.iter_mut().rev() {
        *d = rest % 26;
        rest /= 26;
    }
    name.extend(digits.iter().map(|&d| letter(d)));
    name
}

fn profile_for(username: &str, slot: u32) -> CharProfile {
    let mut rng = StdRng::seed_from_u64(seed_for(username));
    let name = character_name(&host_prefix(), slot);

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

/// How long a queued login waits between server packets.
const QUEUE_TIMEOUT: Duration = Duration::from_secs(60);

/// Helper state passed around during handshake so each step can read/write
/// encrypted frames without juggling the four halves manually.
struct HandshakeState {
    reader: OwnedReadHalf,
    writer: OwnedWriteHalf,
    encrypter: EncrypterHalf,
    decrypter: DecrypterHalf,
    skipped: u64,
    cinematic: bool,
}

impl HandshakeState {
    /// Read server packets until `pick` accepts one, skipping the rest. Each
    /// read must arrive within `step`. A packet wow_messages doesn't model or
    /// can't parse is skipped too: its body has already been read.
    async fn read_until<T>(
        &mut self,
        what: &str,
        step: Duration,
        mut pick: impl FnMut(ServerOpcodeMessage) -> Option<T>,
    ) -> std::io::Result<T> {
        loop {
            let read =
                ServerOpcodeMessage::tokio_read_encrypted(&mut self.reader, &mut self.decrypter);
            match timeout(step, read).await {
                Err(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("world: waiting for {what}: timed out"),
                    ));
                }
                Ok(Ok(msg)) => {
                    if matches!(msg, ServerOpcodeMessage::SMSG_TRIGGER_CINEMATIC(_)) {
                        self.cinematic = true;
                    }
                    match pick(msg) {
                        Some(v) => return Ok(v),
                        None => self.skipped += 1,
                    }
                }
                Ok(Err(ExpectedOpcodeError::Opcode { .. } | ExpectedOpcodeError::Parse(_))) => {
                    self.skipped += 1;
                }
                Ok(Err(ExpectedOpcodeError::Io(e))) => {
                    return Err(std::io::Error::new(
                        e.kind(),
                        format!("world: waiting for {what}: {e}"),
                    ));
                }
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_names_are_letters_and_unique_per_slot() {
        assert_eq!(character_name("bot", 0), "Botaaaaa");
        assert_eq!(character_name("bot", 1), "Botaaaab");
        assert_eq!(character_name("bot", 26), "Botaaaba");
        assert_eq!(character_name("ip", 999), "Ipaabml");
        let names: std::collections::HashSet<String> =
            (0..10_000).map(|s| character_name("bot", s)).collect();
        assert_eq!(names.len(), 10_000);
        assert!(names.iter().all(|n| n.len() <= 12 && n.chars().all(|c| c.is_ascii_alphabetic())));
    }
}
