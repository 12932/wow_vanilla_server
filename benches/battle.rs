//! A faction battle in one spot, through the real sessions and map (over
//! channels, as `benches/crowd.rs` has them: no sockets, no cipher): N
//! Horde against N Alliance at level 60, flagged for PvP, on a disc in
//! Hillsbrad's fields, as peon's faction battle has them, fighting with
//! the moves, casts, auras and blows a battle makes. It measures what
//! peon's 800-a-side battle showed the server short of: the map's update,
//! step by step, and the events between updates.
//!
//! ```sh
//! WVS_DATA=data cargo bench --features world-bench --bench battle               # 200, 400 and 800 a side
//! BATTLE=800 BATTLE_SECS=20 WVS_DATA=data cargo bench --features world-bench --bench battle
//! ```
//!
//! Needs the real world (`WVS_DATA`, a `data/` with `db`): the classes'
//! spells and the gear are its. Each side is a mix of classes and parts (a
//! fifth healers), every bot made ready as peon's are (`.character level
//! 60`, `.character spells`, `.gear`, `.god on` so that none dies and the
//! fight stays as big, `.gm off`, its PvP flag on), put at its spot on a
//! disc `BATTLE_RADIUS` (40) yards across its side's half (all in sight of
//! each other), then:
//!
//! - **melee** (warriors, rogues, a paladin in three) are paired with a foe
//!   of the other side's melee, meet it halfway and circle it, strafing, a
//!   heartbeat each 500 ms, swinging (`CMSG_ATTACKSWING`) and using their
//!   strikes;
//! - **casters** stand, turn to a foe within 30 yd (`MSG_MOVE_SET_FACING`)
//!   and cast at it every 2 to 2.6 s, their class's spells by turns (bolts
//!   with a cast time, damage over time, instants);
//! - **healers** heal and shield friends within 40 yd, their heals over time
//!   and buffs among them;
//! - one caster in ten walks a few yards now and then (a start, heartbeats,
//!   a stop).
//!
//! What it reports, over `BATTLE_SECS` (20) after `BATTLE_WARM` (5) s: the
//! update's wall time (p50 / p99 / max), each step's CPU time an update
//! (mean, p99), the events between updates by kind, and what the clients
//! were sent a second, by kind.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use wow_vanilla_server::game::conn::{Connection, Frames, Inbound, Outbound};
use wow_vanilla_server::game::map::MapKey;
use wow_vanilla_server::game::map::bench_probe;
use wow_vanilla_server::game::store::Store;
use wow_vanilla_server::game::{Realm, RealmConfig, session};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::{ClientOpcodeMessage, ServerOpcodeMessage};
use wow_world_messages::vanilla::*;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Hillsbrad's fields, peon's battle site: its centre, and the way from
/// Southshore to Tarren Mill (the Horde's side).
const MAP: u32 = 0;
const CENTRE: (f32, f32, f32) = (-335.0, -552.0, 54.0);
const HORDE_WAY: (f32, f32) = (0.902_9, -0.430_0);

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Frames the clients were sent, by kind: every update packet, movement,
/// spells, blows, and the rest.
#[derive(Default)]
struct Counts {
    bytes: AtomicU64,
    frames: AtomicU64,
    updates: AtomicU64,
    update_bytes: AtomicU64,
    compressed: AtomicU64,
    moves: AtomicU64,
    spells: AtomicU64,
    blows: AtomicU64,
    cast_failed: AtomicU64,
}

static COUNTS: std::sync::LazyLock<Counts> = std::sync::LazyLock::new(Counts::default);

fn main() {
    let sides: Vec<usize> = std::env::var("BATTLE")
        .unwrap_or_else(|_| "200,400,800".into())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    {
        use tracing_subscriber::Layer;
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_filter(wow_vanilla_server::server::log_filter("error")),
            )
            .init();
    }
    let Some(data_dir) =
        std::env::var_os("WVS_DATA").map(|d| std::path::PathBuf::from(d).join("db"))
    else {
        eprintln!(
            "battle: needs WVS_DATA (a data/ with db): the classes' spells are the real world's"
        );
        std::process::exit(0);
    };
    let mut cfg = wow_vanilla_server::config::ServerConfig::default();
    cfg.spawn.enabled = true;
    cfg.spawn.map = MAP;
    cfg.spawn.x = CENTRE.0;
    cfg.spawn.y = CENTRE.1;
    cfg.spawn.z = CENTRE.2;
    cfg.world.save_interval_secs = 3600;
    cfg.world.data_dir = Some(data_dir.clone());
    // The visibility threads (`[visibility] refresh_threads`): BATTLE_THREADS,
    // 0 the machine's cores up to 8, 1 the map's thread alone.
    cfg.visibility.refresh_threads = env("BATTLE_THREADS", cfg.visibility.refresh_threads);
    // Visibility's budget an update (`[visibility] refresh_budget_ms`):
    // BATTLE_BUDGET_MS.
    cfg.visibility.refresh_budget_ms = env("BATTLE_BUDGET_MS", cfg.visibility.refresh_budget_ms);
    let cfg = wow_vanilla_server::config::install(cfg);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    for side in sides {
        let data = wow_vanilla_server::game::data::load::load(&data_dir);
        rt.block_on(battle(cfg, data, side));
    }
    std::process::exit(0);
}

fn realm(
    cfg: &'static wow_vanilla_server::config::ServerConfig,
    data: wow_vanilla_server::game::data::GameData,
) -> Arc<Realm> {
    let (store, startup) = Store::open(":memory:").expect("an in-memory store");
    let mut config = RealmConfig::from_config(cfg);
    config.everyone_gm = true;
    Realm::new(config, store, startup, data, None)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Warrior,
    Rogue,
    PaladinMelee,
    Mage,
    PriestShadow,
    Warlock,
    Elemental,
    Hunter,
    PriestHeal,
    PaladinHeal,
    ShamanHeal,
    DruidHeal,
}

/// Whom or where a spell is cast at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum At {
    Foe,
    Friend,
    Me,
    /// The ground under a foe (a Blizzard, a Rain of Fire, a Volley).
    Ground,
}

/// A spell of a rotation: its id, its target, and how long its caster is
/// busy with it (its cast or channel, at least the global cooldown).
#[derive(Clone, Copy, Debug)]
struct Cast {
    spell: u32,
    at: At,
    busy_ms: u64,
}

const fn c(spell: u32, at: At, busy_ms: u64) -> Cast {
    Cast { spell, at, busy_ms }
}

impl Role {
    fn melee(self) -> bool {
        matches!(self, Role::Warrior | Role::Rogue | Role::PaladinMelee)
    }

    fn healer(self) -> bool {
        matches!(
            self,
            Role::PriestHeal | Role::PaladinHeal | Role::ShamanHeal | Role::DruidHeal
        )
    }

    /// Its rotation, by turns (rank 1 of each: `.character spells` teaches
    /// every rank, and the cast times are the first rank's).
    fn rotation(self) -> &'static [Cast] {
        use At::*;
        match self {
            // Heroic Strike, Rend, Thunder Clap, Cleave.
            Role::Warrior => {
                const R: &[Cast] = &[
                    c(78, Foe, 1600),
                    c(772, Foe, 1600),
                    c(6343, Me, 1600),
                    c(845, Foe, 1600),
                ];
                R
            }
            // Sinister Strike.
            Role::Rogue => {
                const R: &[Cast] = &[c(1752, Foe, 1600)];
                R
            }
            // Seal of Righteousness, Judgement, Consecration.
            Role::PaladinMelee => {
                const R: &[Cast] = &[c(21084, Me, 1600), c(20271, Foe, 1600), c(26573, Me, 1600)];
                R
            }
            // Frostbolt, Blizzard, Frostbolt, Fire Blast, Arcane Explosion.
            Role::Mage => {
                const R: &[Cast] = &[
                    c(116, Foe, 1700),
                    c(10, Ground, 8200),
                    c(116, Foe, 1700),
                    c(2136, Foe, 1600),
                    c(1449, Me, 1600),
                ];
                R
            }
            // Shadow Word: Pain, Mind Blast, Smite, Psychic Scream.
            Role::PriestShadow => {
                const R: &[Cast] = &[
                    c(589, Foe, 1600),
                    c(8092, Foe, 1700),
                    c(585, Foe, 1700),
                    c(8122, Me, 1600),
                ];
                R
            }
            // Corruption, Curse of Agony, Shadow Bolt, Rain of Fire, Fear.
            Role::Warlock => {
                const R: &[Cast] = &[
                    c(172, Foe, 2100),
                    c(980, Foe, 1600),
                    c(686, Foe, 1800),
                    c(5740, Ground, 8200),
                    c(5782, Foe, 1700),
                ];
                R
            }
            // Lightning Bolt, Chain Lightning, Earth Shock.
            Role::Elemental => {
                const R: &[Cast] = &[c(403, Foe, 1700), c(421, Foe, 2600), c(8042, Foe, 1600)];
                R
            }
            // Hunter's Mark, Serpent Sting, Arcane Shot, Multi-Shot, Volley.
            Role::Hunter => {
                const R: &[Cast] = &[
                    c(1130, Foe, 1600),
                    c(1978, Foe, 1600),
                    c(3044, Foe, 1600),
                    c(2643, Foe, 1600),
                    c(1510, Ground, 6200),
                ];
                R
            }
            // Renew, Power Word: Shield, Flash Heal, Power Word: Fortitude.
            Role::PriestHeal => {
                const R: &[Cast] = &[
                    c(139, Friend, 1600),
                    c(17, Friend, 1600),
                    c(2061, Friend, 1700),
                    c(1243, Friend, 1600),
                ];
                R
            }
            // Holy Light, Flash of Light, Blessing of Might.
            Role::PaladinHeal => {
                const R: &[Cast] = &[
                    c(635, Friend, 2700),
                    c(19750, Friend, 1700),
                    c(19740, Friend, 1600),
                ];
                R
            }
            // Healing Wave, Lesser Healing Wave.
            Role::ShamanHeal => {
                const R: &[Cast] = &[c(331, Friend, 1700), c(8004, Friend, 1700)];
                R
            }
            // Rejuvenation, Regrowth, Healing Touch, Mark of the Wild.
            Role::DruidHeal => {
                const R: &[Cast] = &[
                    c(774, Friend, 1600),
                    c(8936, Friend, 2200),
                    c(5185, Friend, 1700),
                    c(1126, Friend, 1600),
                ];
                R
            }
        }
    }
}

/// The `i`th of a side's bots: its race, class and part, by turns of 20:
/// three warriors, two rogues, four mages, three priests (two healing),
/// two warlocks, three hunters (with their pets), a druid healing, and the
/// side's paladins or shamans (one healing, one not).
fn draw(horde: bool, i: usize) -> (Race, Class, Role) {
    let k = i % 20;
    if horde {
        match k {
            0..=2 => (Race::Orc, Class::Warrior, Role::Warrior),
            3 | 4 => (Race::Undead, Class::Rogue, Role::Rogue),
            5..=7 => (Race::Undead, Class::Mage, Role::Mage),
            8 | 9 => (Race::Undead, Class::Priest, Role::PriestHeal),
            10 => (Race::Undead, Class::Priest, Role::PriestShadow),
            11 | 12 => (Race::Orc, Class::Warlock, Role::Warlock),
            13 | 14 => (Race::Orc, Class::Hunter, Role::Hunter),
            15 => (Race::Tauren, Class::Druid, Role::DruidHeal),
            16 => (Race::Orc, Class::Shaman, Role::ShamanHeal),
            17 => (Race::Orc, Class::Shaman, Role::Elemental),
            18 => (Race::Troll, Class::Mage, Role::Mage),
            _ => (Race::Troll, Class::Hunter, Role::Hunter),
        }
    } else {
        match k {
            0..=2 => (Race::Human, Class::Warrior, Role::Warrior),
            3 | 4 => (Race::Human, Class::Rogue, Role::Rogue),
            5..=7 => (Race::Human, Class::Mage, Role::Mage),
            8 | 9 => (Race::Human, Class::Priest, Role::PriestHeal),
            10 => (Race::Human, Class::Priest, Role::PriestShadow),
            11 | 12 => (Race::Human, Class::Warlock, Role::Warlock),
            13 | 14 => (Race::NightElf, Class::Hunter, Role::Hunter),
            15 => (Race::NightElf, Class::Druid, Role::DruidHeal),
            16 => (Race::Human, Class::Paladin, Role::PaladinHeal),
            17 => (Race::Human, Class::Paladin, Role::PaladinMelee),
            18 => (Race::Gnome, Class::Mage, Role::Mage),
            _ => (Race::Dwarf, Class::Hunter, Role::Hunter),
        }
    }
}

/// A spot on the disc for the `i`th of `n`: a sunflower, even over it.
fn sunflower(i: usize, n: usize, radius: f32) -> (f32, f32) {
    let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
    let r = radius * ((i as f32 + 0.5) / n as f32).sqrt();
    let a = i as f32 * golden;
    (CENTRE.0 + r * a.cos(), CENTRE.1 + r * a.sin())
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn facing(from: (f32, f32), to: (f32, f32)) -> f32 {
    (to.1 - from.1)
        .atan2(to.0 - from.0)
        .rem_euclid(std::f32::consts::TAU)
}

/// A bot as the fight drives it.
struct Fighter {
    tx: mpsc::Sender<Inbound>,
    guid: Guid,
    horde: bool,
    role: Role,
    at: (f32, f32),
    /// Its spot on the disc: a caster stands there, a charge runs to it.
    home: (f32, f32),
    /// Running to its spot, after a charge's start.
    running: bool,
    /// Melee: where it meets its foe, and which side of it it circles.
    meet: (f32, f32),
    phase: f32,
    /// Its foe (melee) or its picks (casters: foes within 30 yd; healers:
    /// friends within 40), by index into the fighters.
    foe: Option<usize>,
    picks: Vec<usize>,
    turn: usize,
    next_cast: Duration,
    next_beat: Duration,
    /// A caster's walk: until when, and which way.
    walking: Option<(Duration, f32)>,
    swinging: bool,
    /// A hunter's pet, and when it is next sent at a foe.
    pet: Option<Guid>,
    next_pet: Duration,
}

fn opcode(frame: &[u8]) -> u16 {
    u16::from_le_bytes([frame[2], frame[3]])
}

const SMSG_CHAR_CREATE: u16 = 0x03A;
const SMSG_CHAR_ENUM: u16 = 0x03B;
const SMSG_MESSAGECHAT: u16 = 0x096;
const SMSG_LOGIN_VERIFY_WORLD: u16 = 0x236;
const MSG_MOVE_TELEPORT_ACK: u16 = 0x0C7;
const SMSG_UPDATE_OBJECT: u16 = 0x0A9;
const SMSG_COMPRESSED_UPDATE_OBJECT: u16 = 0x1F6;
const SMSG_CAST_RESULT: u16 = 0x130;
const SMSG_SPELL_START: u16 = 0x131;
const SMSG_SPELL_GO: u16 = 0x132;
const SMSG_ATTACKERSTATEUPDATE: u16 = 0x14A;

/// Wait for a frame of `op` that `pick` takes; others are dropped unread.
async fn wait_for<T>(
    rx: &mut Frames,
    op: u16,
    mut pick: impl FnMut(&ServerOpcodeMessage) -> Option<T>,
) -> Result<T, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let frame = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .map_err(|_| format!("no {op:#x} within 180 s"))?
            .ok_or_else(|| "the session closed".to_owned())?;
        if frame.len() < 4 || opcode(&frame) != op {
            continue;
        }
        if let Ok(m) = ServerOpcodeMessage::read_unencrypted(&frame[..])
            && let Some(r) = pick(&m)
        {
            return Ok(r);
        }
    }
}

async fn send(tx: &mpsc::Sender<Inbound>, m: impl Into<ClientOpcodeMessage>) -> Result<(), String> {
    tx.send(Inbound::Msg(m.into()))
        .await
        .map_err(|_| "the session has gone".to_owned())
}

async fn say(
    tx: &mpsc::Sender<Inbound>,
    rx: &mut Frames,
    text: &str,
    reply: &str,
) -> Result<(), String> {
    send(
        tx,
        CMSG_MESSAGECHAT {
            chat_type: CMSG_MESSAGECHAT_ChatType::Say,
            language: Language::Universal,
            message: text.to_owned(),
        },
    )
    .await?;
    wait_for(rx, SMSG_MESSAGECHAT, |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c)
            if matches!(c.chat_type, SMSG_MESSAGECHAT_ChatType::System { .. })
                && c.message.contains(reply) =>
        {
            Some(())
        }
        _ => None,
    })
    .await
    .map_err(|e| format!("`{text}`: {e}"))
}

/// The `i`th bot made, logged in, made ready and put at `at`.
async fn ready(
    realm: Arc<Realm>,
    i: usize,
    horde: bool,
    race: Race,
    class: Class,
    at: (f32, f32),
) -> Result<(mpsc::Sender<Inbound>, Frames, Guid), String> {
    let (outbound, mut rx) = Outbound::detached();
    let (tx, inbound) = mpsc::channel(256);
    tokio::spawn(session::run(
        realm.clone(),
        format!("BT{i:05}"),
        Connection { outbound, inbound },
    ));
    let letters: String = format!("{i:05}")
        .bytes()
        .map(|b| char::from(b'a' + (b - b'0')))
        .collect();
    let name = format!("B{}{letters}", if horde { "h" } else { "a" });
    send(
        &tx,
        CMSG_CHAR_CREATE {
            name,
            race,
            class,
            gender: Gender::Male,
            skin_color: 0,
            face: 0,
            hair_style: 0,
            hair_color: 0,
            facial_hair: 0,
        },
    )
    .await?;
    let made = wait_for(&mut rx, SMSG_CHAR_CREATE, |m| match m {
        ServerOpcodeMessage::SMSG_CHAR_CREATE(c) => Some(c.result),
        _ => None,
    })
    .await?;
    if made != WorldResult::CharCreateSuccess {
        return Err(format!("creation: {made:?}"));
    }
    send(&tx, CMSG_CHAR_ENUM {}).await?;
    let guid = wait_for(&mut rx, SMSG_CHAR_ENUM, |m| match m {
        ServerOpcodeMessage::SMSG_CHAR_ENUM(e) => e.characters.first().map(|c| c.guid),
        _ => None,
    })
    .await?;
    send(&tx, CMSG_PLAYER_LOGIN { guid }).await?;
    wait_for(&mut rx, SMSG_LOGIN_VERIFY_WORLD, |_| Some(())).await?;
    send(&tx, CMSG_SET_ACTIVE_MOVER { guid }).await?;
    send(&tx, CMSG_COMPLETE_CINEMATIC {}).await?;
    for (c, r) in [
        (".character level 60", "Level"),
        (".character spells", "Learned"),
        (".gear", ""),
        (".god on", "Undying on"),
        (".gm off", "GM mode off"),
    ] {
        if r.is_empty() {
            // `.gear`'s answer says what it put on, or that it found none.
            say(&tx, &mut rx, c, "").await?;
        } else {
            say(&tx, &mut rx, c, r).await?;
        }
    }
    send(
        &tx,
        CMSG_TOGGLE_PVP {
            set: Some(CMSG_TOGGLE_PVP_set { enable_pvp: true }),
        },
    )
    .await?;
    let o = if horde {
        facing((0.0, 0.0), (-HORDE_WAY.0, -HORDE_WAY.1))
    } else {
        facing((0.0, 0.0), HORDE_WAY)
    };
    send(
        &tx,
        CMSG_MESSAGECHAT {
            chat_type: CMSG_MESSAGECHAT_ChatType::Say,
            language: Language::Universal,
            message: format!(".go xyz {} {} {} {MAP} {o}", at.0, at.1, CENTRE.2),
        },
    )
    .await?;
    let counter = wait_for(&mut rx, MSG_MOVE_TELEPORT_ACK, |m| match m {
        ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) if t.guid == guid => Some(t.movement_counter),
        _ => None,
    })
    .await?;
    send(
        &tx,
        MSG_MOVE_TELEPORT_ACK_Client {
            guid,
            movement_counter: counter,
            time: Default::default(),
        },
    )
    .await?;
    Ok((tx, rx, guid))
}

/// The clients' queues, drained every 10 ms on `threads` threads of their
/// own, what they got counted.
fn drain(
    queues: Vec<Frames>,
    on: Arc<AtomicBool>,
    threads: usize,
) -> Vec<std::thread::JoinHandle<Vec<Frames>>> {
    let per = queues.len().div_ceil(threads.max(1)).max(1);
    let mut queues = queues;
    let mut handles = Vec::new();
    while !queues.is_empty() {
        let mut mine: Vec<Frames> = queues.drain(..per.min(queues.len())).collect();
        let on = on.clone();
        handles.push(std::thread::spawn(move || {
            while on.load(Ordering::Relaxed) {
                let c = &*COUNTS;
                let (mut bytes, mut frames, mut updates, mut update_bytes, mut compressed) =
                    (0, 0, 0, 0, 0);
                let (mut moves, mut spells, mut blows, mut failed) = (0, 0, 0, 0);
                for q in &mut mine {
                    while let Some(f) = q.try_recv() {
                        bytes += f.len() as u64;
                        frames += 1;
                        if f.len() < 4 {
                            continue;
                        }
                        match opcode(&f) {
                            SMSG_UPDATE_OBJECT => {
                                updates += 1;
                                update_bytes += f.len() as u64;
                            }
                            SMSG_COMPRESSED_UPDATE_OBJECT => {
                                updates += 1;
                                compressed += 1;
                                update_bytes += f.len() as u64;
                            }
                            0xB5..=0xEE => moves += 1,
                            SMSG_SPELL_START | SMSG_SPELL_GO => spells += 1,
                            SMSG_ATTACKERSTATEUPDATE => blows += 1,
                            SMSG_CAST_RESULT => failed += 1,
                            _ => {}
                        }
                    }
                }
                c.bytes.fetch_add(bytes, Ordering::Relaxed);
                c.frames.fetch_add(frames, Ordering::Relaxed);
                c.updates.fetch_add(updates, Ordering::Relaxed);
                c.update_bytes.fetch_add(update_bytes, Ordering::Relaxed);
                c.compressed.fetch_add(compressed, Ordering::Relaxed);
                c.moves.fetch_add(moves, Ordering::Relaxed);
                c.spells.fetch_add(spells, Ordering::Relaxed);
                c.blows.fetch_add(blows, Ordering::Relaxed);
                c.cast_failed.fetch_add(failed, Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(10));
            }
            mine
        }));
    }
    handles
}

/// The process's CPU seconds so far, every thread's.
fn process_cpu() -> f64 {
    // SAFETY: an all-zero rusage is valid, and getrusage writes into it.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `ru` is a valid rusage to write to.
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let secs = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    secs(ru.ru_utime) + secs(ru.ru_stime)
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

/// The map's updates and events over what the probe took.
fn report_map(key: MapKey, secs: f64) {
    let (samples, _) = bench_probe::take();
    let kinds = bench_probe::take_kinds();
    let mailbox = bench_probe::take_mailbox();
    let samples: Vec<_> = samples.into_iter().filter(|s| s.key == key).collect();
    let n = samples.len().max(1) as f64;
    let mut walls: Vec<f64> = samples.iter().map(|s| s.wall.as_secs_f64() * 1e3).collect();
    walls.sort_by(f64::total_cmp);
    let mut cpus: Vec<f64> = samples.iter().map(|s| s.cpu.as_secs_f64() * 1e3).collect();
    cpus.sort_by(f64::total_cmp);
    let update_cpu: f64 = cpus.iter().sum::<f64>() / 1e3;
    let mut steps: ahash::AHashMap<&str, Vec<f64>> = ahash::AHashMap::new();
    for s in &samples {
        for (name, d) in &s.steps {
            steps.entry(name).or_default().push(d.as_secs_f64() * 1e3);
        }
    }
    let mut steps: Vec<(&str, f64, f64)> = steps
        .into_iter()
        .map(|(name, mut v)| {
            v.sort_by(f64::total_cmp);
            (name, v.iter().sum::<f64>() / n, pct(&v, 0.99))
        })
        .collect();
    steps.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut kinds: Vec<_> = kinds.into_iter().filter(|((k, _), _)| *k == key).collect();
    kinds.sort_by_key(|a| std::cmp::Reverse(a.1.1));
    let events: u64 = kinds.iter().map(|(_, (n, _))| n).sum();
    let event_cpu: f64 = kinds.iter().map(|(_, (_, d))| d.as_secs_f64()).sum();
    let mut boxes: Vec<f64> = mailbox
        .iter()
        .filter(|(k, _)| *k == key)
        .map(|(_, n)| *n as f64)
        .collect();
    boxes.sort_by(f64::total_cmp);
    println!(
        "  updates {} ({:.1}/s): wall p50 {:.2} / p99 {:.2} / max {:.2} ms; CPU p50 {:.2} / p99 {:.2} ms",
        samples.len(),
        samples.len() as f64 / secs,
        pct(&walls, 0.5),
        pct(&walls, 0.99),
        walls.last().copied().unwrap_or(0.0),
        pct(&cpus, 0.5),
        pct(&cpus, 0.99),
    );
    println!(
        "  map thread: {:.2} s in updates, {:.2} s in {} events ({:.0}/s): busy {:.0}%; mailbox after an update p50 {:.0} / max {:.0}",
        update_cpu,
        event_cpu,
        events,
        events as f64 / secs,
        100.0 * (update_cpu + event_cpu) / secs,
        pct(&boxes, 0.5),
        boxes.last().copied().unwrap_or(0.0)
    );
    // The slowest updates, by their dearest steps.
    let slowest: usize = env("BATTLE_SLOWEST", 0);
    if slowest > 0 {
        let mut by: Vec<_> = samples.iter().collect();
        by.sort_by_key(|s| std::cmp::Reverse(s.wall));
        for s in by.iter().take(slowest) {
            let mut steps = s.steps.clone();
            steps.sort_by_key(|x| std::cmp::Reverse(x.1));
            let top: Vec<String> = steps
                .iter()
                .take(4)
                .map(|(n, d)| format!("{n} {:.1}", d.as_secs_f64() * 1e3))
                .collect();
            println!(
                "  a slow update: {:.1} ms wall, {:.1} ms CPU: {}",
                s.wall.as_secs_f64() * 1e3,
                s.cpu.as_secs_f64() * 1e3,
                top.join(", ")
            );
            // Its dearest zones under the steps (BATTLE_SLOWEST_ZONES, 0 for
            // none), each's count in brackets.
            let shown: usize = env("BATTLE_SLOWEST_ZONES", 6);
            if shown > 0 {
                let names: ahash::AHashSet<&str> = s.steps.iter().map(|(n, _)| *n).collect();
                let mut zones: Vec<_> = s
                    .zones
                    .iter()
                    .filter(|(n, _, _)| *n != "update" && !names.contains(n))
                    .collect();
                zones.sort_by_key(|z| std::cmp::Reverse(z.2));
                let top: Vec<String> = zones
                    .iter()
                    .take(shown)
                    .map(|(n, c, d)| format!("{n} {:.1} [{c}]", d.as_secs_f64() * 1e3))
                    .collect();
                println!("      zones: {}", top.join(", "));
            }
        }
    }
    println!("  steps, CPU ms an update (mean / p99):");
    for (name, mean, p99) in steps.iter().take(14) {
        println!("    {name:<20} {mean:>8.3} / {p99:>8.3}");
    }
    // Every zone on the map's thread (Tracy's, timed by the probe), inside
    // the steps and the events: with BATTLE_ZONES=n, the n dearest.
    let zones_shown: usize = env("BATTLE_ZONES", 0);
    if zones_shown > 0 {
        let mut zones = bench_probe::take_zones();
        zones.sort_by_key(|z| std::cmp::Reverse(z.2));
        println!("  zones (wall ms a second, count a second, µs each):");
        for (name, count, d) in zones.iter().take(zones_shown) {
            println!(
                "    {name:<32} {:>8.2} {:>9.0} {:>9.2}",
                d.as_secs_f64() * 1e3 / secs,
                *count as f64 / secs,
                d.as_secs_f64() * 1e6 / (*count).max(1) as f64
            );
        }
    }
    let top: Vec<String> = kinds
        .iter()
        .take(10)
        .map(|((_, k), (n, d))| {
            format!(
                "{k} {:.0}/s x {:.1} µs = {:.0} ms/s",
                *n as f64 / secs,
                d.as_secs_f64() * 1e6 / (*n).max(1) as f64,
                d.as_secs_f64() * 1e3 / secs
            )
        })
        .collect();
    println!("  dearest events (CPU):");
    for t in top {
        println!("    {t}");
    }
}

async fn battle(
    cfg: &'static wow_vanilla_server::config::ServerConfig,
    data: wow_vanilla_server::game::data::GameData,
    side: usize,
) {
    let radius: f32 = env("BATTLE_RADIUS", 40.0);
    let secs: u64 = env("BATTLE_SECS", 20);
    let warm: u64 = env("BATTLE_WARM", 5);
    let realm = realm(cfg, data);
    let key = MapKey::world(MAP);
    realm.maps.get(&realm, key).query(|_| ()).await;

    // The spots, and each side's bots drawn by turns.
    let n = side * 2;
    let mut spots: Vec<(f32, f32)> = (0..n).map(|i| sunflower(i, n, radius)).collect();
    // As many of each side: the disc's halves by the way between the towns.
    spots.sort_by(|a, b| {
        let da = (a.0 - CENTRE.0) * HORDE_WAY.0 + (a.1 - CENTRE.1) * HORDE_WAY.1;
        let db = (b.0 - CENTRE.0) * HORDE_WAY.0 + (b.1 - CENTRE.1) * HORDE_WAY.1;
        db.total_cmp(&da)
    });
    // With a charge, each side starts `BATTLE_GAP` yards back from its half,
    // out of the other's sight, and runs to its spot.
    let gap: f32 = env("BATTLE_GAP", 0.0);
    let t = Instant::now();
    let gate = Arc::new(tokio::sync::Semaphore::new(64));
    let mut joins = Vec::with_capacity(n);
    for (i, &home) in spots.iter().enumerate() {
        let horde = i < side;
        let (race, class, role) = draw(horde, if horde { i } else { i - side });
        let back = if horde { gap } else { -gap };
        let at = (home.0 + back * HORDE_WAY.0, home.1 + back * HORDE_WAY.1);
        let realm = realm.clone();
        let gate = gate.clone();
        joins.push(tokio::spawn(async move {
            let _p = gate.acquire_owned().await.ok()?;
            match ready(realm, i, horde, race, class, at).await {
                Ok((tx, rx, guid)) => Some((tx, rx, guid, horde, role, at, home)),
                Err(e) => {
                    eprintln!("  bot {i}: {e}");
                    None
                }
            }
        }));
    }
    let mut fighters = Vec::with_capacity(n);
    let mut queues = Vec::with_capacity(n);
    for j in joins {
        if let Ok(Some((tx, rx, guid, horde, role, at, home))) = j.await {
            queues.push(rx);
            fighters.push(Fighter {
                tx,
                guid,
                horde,
                role,
                at,
                home,
                running: at != home,
                meet: home,
                phase: 0.0,
                foe: None,
                picks: Vec::new(),
                turn: 0,
                next_cast: Duration::ZERO,
                next_beat: Duration::ZERO,
                walking: None,
                swinging: false,
                pet: None,
                next_pet: Duration::ZERO,
            });
        }
    }
    let setup = t.elapsed().as_secs_f64();
    let horde = fighters.iter().filter(|f| f.horde).count();
    println!(
        "battle: {horde} Horde against {} Alliance on a disc {:.0} yd across in Hillsbrad (ready in {setup:.0} s)",
        fighters.len() - horde,
        radius * 2.0
    );
    if env("BATTLE_RAIDS", 1) == 1 {
        raids(&realm, &fighters);
    }
    plan(&mut fighters);
    pets(&realm, key, &mut fighters).await;
    let on = Arc::new(AtomicBool::new(true));
    let drainers = drain(queues, on.clone(), 4);

    let stop = Arc::new(AtomicBool::new(false));
    let refused = Arc::new(AtomicU64::new(0));
    let sent = Arc::new(AtomicU64::new(0));
    // The fighters' guids and spots, for the targets.
    let guids: Arc<Vec<Guid>> = Arc::new(fighters.iter().map(|f| f.guid).collect());
    let spots: Arc<Vec<(f32, f32)>> = Arc::new(
        fighters
            .iter()
            .map(|f| if f.role.melee() { f.meet } else { f.home })
            .collect(),
    );
    let groups = 8;
    let per = fighters.len().div_ceil(groups);
    let mut tasks = Vec::new();
    let started = Instant::now();
    // With BATTLE_SIGHT_MS, the players' sight every so many ms from the
    // start: the refreshes waiting and how many objects each knows, on
    // average (cheap: no look into what they know).
    let sight_every: u64 = env("BATTLE_SIGHT_MS", 0);
    let sampler = (sight_every > 0).then(|| {
        let (realm, stop) = (realm.clone(), stop.clone());
        tokio::spawn(async move {
            let mut out = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                let t = started.elapsed().as_secs_f32();
                if let Some((waiting, known)) = realm
                    .maps
                    .get(&realm, key)
                    .query(|m| {
                        let known: usize = m.players.values().map(|p| p.known.len()).sum();
                        (m.visibility_dirty.len(), known / m.players.len().max(1))
                    })
                    .await
                {
                    out.push(format!("{t:.1}s {waiting}/{known}"));
                }
                tokio::time::sleep(Duration::from_millis(sight_every)).await;
            }
            out
        })
    });
    let mut fighters = fighters;
    let mut seed = 0u64;
    while !fighters.is_empty() {
        let group: Vec<Fighter> = fighters.drain(..per.min(fighters.len())).collect();
        seed += 1;
        let (stop, refused, sent, guids, spots) = (
            stop.clone(),
            refused.clone(),
            sent.clone(),
            guids.clone(),
            spots.clone(),
        );
        tasks.push(tokio::spawn(fight(
            group, guids, spots, started, seed, stop, refused, sent,
        )));
    }
    if gap > 0.0 {
        // The charge: from the start until every caster is at its spot and
        // a second more.
        let charge = Duration::from_secs_f32(gap / 7.0 + 1.0);
        bench_probe::enable();
        bench_probe::take();
        bench_probe::take_kinds();
        bench_probe::take_mailbox();
        bench_probe::take_zones();
        bench_probe::zones_of(Some(key));
        tokio::time::sleep(charge).await;
        bench_probe::disable();
        println!("  the charge ({:.0} s):", charge.as_secs_f32());
        report_map(key, charge.as_secs_f64());
        sight(&realm, key).await;
        println!("  the fight:");
    }
    tokio::time::sleep(Duration::from_secs(warm)).await;
    for c in [
        &COUNTS.bytes,
        &COUNTS.frames,
        &COUNTS.updates,
        &COUNTS.update_bytes,
        &COUNTS.compressed,
        &COUNTS.moves,
        &COUNTS.spells,
        &COUNTS.blows,
        &COUNTS.cast_failed,
    ] {
        c.store(0, Ordering::Relaxed);
    }
    let sent0 = sent.load(Ordering::Relaxed);
    bench_probe::enable();
    bench_probe::take();
    bench_probe::take_kinds();
    bench_probe::take_mailbox();
    bench_probe::take_zones();
    bench_probe::zones_of(Some(key));
    let t0 = Instant::now();
    let cpu0 = process_cpu();
    tokio::time::sleep(Duration::from_secs(secs)).await;
    let took = t0.elapsed().as_secs_f64();
    let cpu = process_cpu() - cpu0;
    bench_probe::disable();
    println!(
        "  the process's CPU: {:.2} cores (the server and the bots, their queues drained)",
        cpu / took
    );
    let clients = guids.len() as f64;
    let c = &*COUNTS;
    let per_client = |a: &AtomicU64| a.load(Ordering::Relaxed) as f64 / took / clients;
    println!(
        "  the bots sent {:.0} packets a second ({} not taken)",
        (sent.load(Ordering::Relaxed) - sent0) as f64 / took,
        refused.load(Ordering::Relaxed)
    );
    println!(
        "  a client got a second: {:.0} frames, {:.0} kB; update packets {:.1} ({:.0}% compressed, {:.1} kB); movement {:.0}, spell starts and goes {:.0}, blows {:.0}, cast results {:.1}",
        per_client(&c.frames),
        per_client(&c.bytes) / 1e3,
        per_client(&c.updates),
        100.0 * c.compressed.load(Ordering::Relaxed) as f64
            / c.updates.load(Ordering::Relaxed).max(1) as f64,
        per_client(&c.update_bytes) / 1e3,
        per_client(&c.moves),
        per_client(&c.spells),
        per_client(&c.blows),
        per_client(&c.cast_failed),
    );
    report_map(key, took);
    sight(&realm, key).await;
    // With BATTLE_DECIDE=n, n players' whole refreshes decided (not
    // applied) in the battle as it stands, across the visibility threads
    // and on the map's thread alone.
    let decide: usize = env("BATTLE_DECIDE", 0);
    if decide > 0 {
        let probe: Vec<Guid> = guids.iter().take(decide).copied().collect();
        let timed = realm
            .maps
            .get(&realm, key)
            .query(move |m| {
                use wow_vanilla_server::game::map::visibility::decide_whole_alone;
                let mut out = Vec::new();
                for parts in [true, false, true, false] {
                    let t = Instant::now();
                    let acts: usize = probe.iter().map(|&g| decide_whole_alone(m, g, parts)).sum();
                    out.push((parts, t.elapsed(), acts));
                }
                out
            })
            .await
            .unwrap_or_default();
        for (parts, took, acts) in timed {
            println!(
                "  {decide} whole refreshes decided {}: {:.1} µs each ({acts} decisions)",
                if parts { "in parts" } else { "alone" },
                took.as_secs_f64() * 1e6 / decide as f64
            );
        }
    }
    stop.store(true, Ordering::Relaxed);
    if let Some(s) = sampler
        && let Ok(series) = s.await
    {
        println!("  sight over time (waiting/known): {}", series.join(", "));
    }
    let mut kept = Vec::new();
    for t in tasks {
        if let Ok(g) = t.await {
            kept.push(g);
        }
    }
    on.store(false, Ordering::Relaxed);
    for d in drainers {
        let _ = d.join();
    }
    drop(kept);
    realm.shutdown_within(Duration::from_secs(20)).await;
    drop(realm);
}

/// How far the players' sight has got: how many objects each knows, on
/// average and at the least, and the refreshes waiting (a charge's sides
/// coming into each other's sight are worked out over updates, within
/// `[visibility] refresh_budget_ms`).
async fn sight(realm: &Arc<Realm>, key: MapKey) {
    let (known, least, players, waiting, of_players, units) = realm
        .maps
        .get(realm, key)
        .query(|m| {
            use wow_vanilla_server::game::guid;
            let known: usize = m.players.values().map(|p| p.known.len()).sum();
            let least = m.players.values().map(|p| p.known.len()).min().unwrap_or(0);
            let of_players: usize = m
                .players
                .values()
                .map(|p| p.known.iter().filter(|g| guid::is_player(**g)).count())
                .sum();
            let units: usize = m
                .players
                .values()
                .map(|p| p.known.iter().filter(|g| guid::is_unit(**g)).count())
                .sum();
            (known, least, m.players.len(), m.visibility_dirty.len(), of_players, units)
        })
        .await
        .unwrap_or_default();
    let n = players.max(1) as f64;
    println!(
        "  sight: each player knows {:.0} objects ({least} at the least), {:.1} players and {:.1} creatures; {waiting} refreshes waiting",
        known as f64 / n,
        of_players as f64 / n,
        units as f64 / n
    );
}

/// Each hunter's pet out (`.pet`), and its guid.
async fn pets(realm: &Arc<Realm>, key: MapKey, fighters: &mut [Fighter]) {
    let hunters: Vec<Guid> = fighters
        .iter()
        .filter(|f| f.role == Role::Hunter)
        .map(|f| f.guid)
        .collect();
    let found: Vec<(Guid, Guid)> = realm
        .maps
        .get(realm, key)
        .query(move |m| {
            use wow_vanilla_server::game::fields::UNIT_FIELD_SUMMON;
            hunters
                .iter()
                .filter_map(|&h| {
                    wow_vanilla_server::game::map::gm::command(m, h, "pet");
                    let pet = m.players.get(&h)?.fields.get_guid(UNIT_FIELD_SUMMON);
                    (!pet.is_zero()).then_some((h, pet))
                })
                .collect()
        })
        .await
        .unwrap_or_default();
    let found: ahash::AHashMap<Guid, Guid> = found.into_iter().collect();
    for f in fighters.iter_mut() {
        f.pet = found.get(&f.guid).copied();
    }
    println!("  {} hunters' pets out", found.len());
}

/// Each side in raids of 40 by its order, as peon's sides form them: the
/// first of each invites the rest, a party's four, then the raid.
fn raids(realm: &Realm, fighters: &[Fighter]) {
    use wow_vanilla_server::game::social::groups;
    for horde in [true, false] {
        let side: Vec<Guid> = fighters
            .iter()
            .filter(|f| f.horde == horde)
            .map(|f| f.guid)
            .collect();
        for raid in side.chunks(40) {
            let Some(leader) = realm.registry.online(raid[0]) else {
                continue;
            };
            for (k, &g) in raid[1..].iter().enumerate() {
                if k == 4 {
                    groups::convert_to_raid(realm, &leader);
                }
                let Some(m) = realm.registry.online(g) else {
                    continue;
                };
                groups::invite(realm, &leader, &m.name);
                groups::accept(realm, &m);
            }
        }
    }
}

/// Each melee fighter paired with the nearest unpaired foe of the other
/// side's melee, the two to meet halfway; each caster's foes within 30 yd
/// (the nearest few if none), each healer's friends within 40.
fn plan(fighters: &mut [Fighter]) {
    let n = fighters.len();
    let mut paired = vec![false; n];
    for i in 0..n {
        if !fighters[i].role.melee() || paired[i] {
            continue;
        }
        let me = fighters[i].home;
        let best = (0..n)
            .filter(|&j| {
                !paired[j]
                    && j != i
                    && fighters[j].role.melee()
                    && fighters[j].horde != fighters[i].horde
            })
            .min_by(|&a, &b| dist(me, fighters[a].at).total_cmp(&dist(me, fighters[b].at)));
        if let Some(j) = best {
            paired[i] = true;
            paired[j] = true;
            let meet = (
                (me.0 + fighters[j].home.0) / 2.0,
                (me.1 + fighters[j].home.1) / 2.0,
            );
            fighters[i].foe = Some(j);
            fighters[j].foe = Some(i);
            fighters[i].meet = meet;
            fighters[j].meet = meet;
            fighters[i].phase = 0.0;
            fighters[j].phase = std::f32::consts::PI;
        }
    }
    for i in 0..n {
        let f = &fighters[i];
        if f.role.melee() {
            continue;
        }
        let (friend, reach) = if f.role.healer() {
            (true, 40.0)
        } else {
            (false, 30.0)
        };
        let mut near: Vec<(f32, usize)> = (0..n)
            .filter(|&j| j != i && (fighters[j].horde == f.horde) == friend)
            .map(|j| {
                // A melee fighter is where it meets its foe.
                let at = if fighters[j].role.melee() {
                    fighters[j].meet
                } else {
                    fighters[j].home
                };
                (dist(f.home, at), j)
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        let within: Vec<usize> = near
            .iter()
            .filter(|(d, _)| *d <= reach)
            .map(|(_, j)| *j)
            .collect();
        let picks = if within.len() >= 3 {
            within
        } else {
            near.iter().take(8).map(|(_, j)| *j).collect()
        };
        fighters[i].picks = picks;
    }
}

fn info(at: (f32, f32), o: f32, flags: MovementInfo_MovementFlags, ms: u32) -> MovementInfo {
    MovementInfo {
        flags,
        timestamp: ms,
        position: Vector3d {
            x: at.0,
            y: at.1,
            z: CENTRE.2,
        },
        orientation: o,
        fall_time: 0,
    }
}

/// A tenth of a second's turn for a group of fighters, until `stop`.
#[allow(clippy::too_many_arguments)]
async fn fight(
    mut group: Vec<Fighter>,
    guids: Arc<Vec<Guid>>,
    spots: Arc<Vec<(f32, f32)>>,
    started: Instant,
    seed: u64,
    stop: Arc<AtomicBool>,
    refused: Arc<AtomicU64>,
    sent: Arc<AtomicU64>,
) -> Vec<Fighter> {
    let mut rng = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    // Each fighter's first action somewhere in its first 2.5 s.
    for f in &mut group {
        f.next_cast = Duration::from_millis(next() % 2500);
        f.next_beat = Duration::from_millis(next() % 500);
    }
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let try_send =
        |tx: &mpsc::Sender<Inbound>, m: ClientOpcodeMessage| match tx.try_send(Inbound::Msg(m)) {
            Ok(()) => {
                sent.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                refused.fetch_add(1, Ordering::Relaxed);
            }
        };
    while !stop.load(Ordering::Relaxed) {
        tick.tick().await;
        let now = started.elapsed();
        let ms = now.as_millis() as u32;
        for f in &mut group {
            if f.role.melee() {
                let Some(foe) = f.foe else { continue };
                if now >= f.next_beat {
                    f.next_beat = now + Duration::from_millis(500);
                    let away = dist(f.at, f.meet);
                    let m: ClientOpcodeMessage = if away > 2.0 {
                        // Running to the meeting point, 3.5 yd a beat.
                        let o = facing(f.at, f.meet);
                        let step = away.min(3.5) / away;
                        f.at = (
                            f.at.0 + (f.meet.0 - f.at.0) * step,
                            f.at.1 + (f.meet.1 - f.at.1) * step,
                        );
                        MSG_MOVE_HEARTBEAT_Client {
                            info: info(f.at, o, MovementInfo_MovementFlags::new_forward(), ms),
                        }
                        .into()
                    } else {
                        // Strafing round it, facing it, 1.5 yd out.
                        f.phase += 1.0;
                        let at = (
                            f.meet.0 + 1.5 * f.phase.cos(),
                            f.meet.1 + 1.5 * f.phase.sin(),
                        );
                        let o = facing(at, f.meet);
                        f.at = at;
                        MSG_MOVE_HEARTBEAT_Client {
                            info: info(at, o, MovementInfo_MovementFlags::new_strafe_left(), ms),
                        }
                        .into()
                    };
                    try_send(&f.tx, m);
                    if !f.swinging {
                        f.swinging = true;
                        try_send(&f.tx, CMSG_SET_SELECTION { target: guids[foe] }.into());
                        try_send(&f.tx, CMSG_ATTACKSWING { guid: guids[foe] }.into());
                    }
                }
                if now >= f.next_cast {
                    let rotation = f.role.rotation();
                    let cast = rotation[f.turn % rotation.len()];
                    f.turn += 1;
                    f.next_cast = now + Duration::from_millis(cast.busy_ms + next() % 400);
                    try_send(&f.tx, spell(cast, f.guid, guids[foe], spots[foe]));
                }
                continue;
            }
            // A charge: running to its spot, heartbeats each 500 ms, then a
            // stop.
            if f.running {
                if now >= f.next_beat {
                    f.next_beat = now + Duration::from_millis(500);
                    let away = dist(f.at, f.home);
                    let o = facing(f.at, f.home);
                    let step = away.min(3.5) / away.max(f32::EPSILON);
                    f.at = (
                        f.at.0 + (f.home.0 - f.at.0) * step,
                        f.at.1 + (f.home.1 - f.at.1) * step,
                    );
                    let m: ClientOpcodeMessage = if away <= 3.5 {
                        f.running = false;
                        f.at = f.home;
                        MSG_MOVE_STOP_Client {
                            info: info(f.at, o, MovementInfo_MovementFlags::empty(), ms),
                        }
                        .into()
                    } else {
                        MSG_MOVE_HEARTBEAT_Client {
                            info: info(f.at, o, MovementInfo_MovementFlags::new_forward(), ms),
                        }
                        .into()
                    };
                    try_send(&f.tx, m);
                }
                continue;
            }
            // A caster's walk, now and then: a start, heartbeats, a stop.
            if let Some((until, o)) = f.walking {
                if now >= f.next_beat {
                    f.next_beat = now + Duration::from_millis(500);
                    f.at = (f.at.0 + 3.5 * o.cos(), f.at.1 + 3.5 * o.sin());
                    let stopping = now >= until;
                    let flags = if stopping {
                        f.walking = None;
                        MovementInfo_MovementFlags::empty()
                    } else {
                        MovementInfo_MovementFlags::new_forward()
                    };
                    let m: ClientOpcodeMessage = if stopping {
                        MSG_MOVE_STOP_Client {
                            info: info(f.at, o, flags, ms),
                        }
                        .into()
                    } else {
                        MSG_MOVE_HEARTBEAT_Client {
                            info: info(f.at, o, flags, ms),
                        }
                        .into()
                    };
                    try_send(&f.tx, m);
                }
                continue;
            }
            if let Some(pet) = f.pet
                && now >= f.next_pet
                && !f.picks.is_empty()
            {
                f.next_pet = now + Duration::from_secs(10);
                let foe = f.picks[(next() as usize) % f.picks.len()];
                try_send(
                    &f.tx,
                    CMSG_PET_ACTION {
                        pet,
                        data: 2 | (7 << 24),
                        target: guids[foe],
                    }
                    .into(),
                );
            }
            if now < f.next_cast || f.picks.is_empty() {
                continue;
            }
            // One in ten casters walks a few yards now and then, back and
            // forth.
            if f.turn % 7 == 6 && (f.guid.guid() % 10) == 0 {
                f.turn += 1;
                let o = if f.turn % 2 == 0 {
                    0.0
                } else {
                    std::f32::consts::PI
                };
                f.walking = Some((now + Duration::from_millis(1500), o));
                f.next_beat = now + Duration::from_millis(500);
                try_send(
                    &f.tx,
                    MSG_MOVE_START_FORWARD_Client {
                        info: info(f.at, o, MovementInfo_MovementFlags::new_forward(), ms),
                    }
                    .into(),
                );
                continue;
            }
            let rotation = f.role.rotation();
            let cast = rotation[f.turn % rotation.len()];
            let pick = f.picks[(next() as usize) % f.picks.len()];
            f.turn += 1;
            f.next_cast = now + Duration::from_millis(cast.busy_ms + next() % 400);
            let o = facing(f.at, spots[pick]);
            try_send(
                &f.tx,
                MSG_MOVE_SET_FACING_Client {
                    info: info(f.at, o, MovementInfo_MovementFlags::empty(), ms),
                }
                .into(),
            );
            try_send(
                &f.tx,
                CMSG_SET_SELECTION {
                    target: guids[pick],
                }
                .into(),
            );
            try_send(&f.tx, spell(cast, f.guid, guids[pick], spots[pick]));
        }
    }
    group
}

/// `cast` at `target` (or its ground, or the caster).
fn spell(cast: Cast, me: Guid, target: Guid, spot: (f32, f32)) -> ClientOpcodeMessage {
    let flags = SpellCastTargets_SpellCastTargetFlags::empty();
    let flags = match cast.at {
        At::Foe | At::Friend => flags.set_unit(SpellCastTargets_SpellCastTargetFlags_Unit::Unit {
            unit_target: target,
        }),
        At::Me => {
            flags.set_unit(SpellCastTargets_SpellCastTargetFlags_Unit::Unit { unit_target: me })
        }
        At::Ground => flags.set_dest_location(SpellCastTargets_SpellCastTargetFlags_DestLocation {
            destination: Vector3d {
                x: spot.0,
                y: spot.1,
                z: CENTRE.2,
            },
        }),
    };
    CMSG_CAST_SPELL {
        spell: cast.spell,
        targets: SpellCastTargets {
            target_flags: flags,
        },
    }
    .into()
}
