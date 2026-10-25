//! The fight record: what a map copy believes and tells its clients, a line
//! of JSON an event, for a fight to be gone over afterwards
//! (docs/TESTING.md, "The fight record"). Off unless `[record] maps` names
//! the map, or a GM says `.record on` in it.
//!
//! Every unit's state is sampled every `[record] every_ms` (players, and the
//! creatures awake: seen by a player or in a fight), a unit's line only once
//! it has changed or a second since its last, each with the height of the
//! ground under it. Beside the samples, each change as it happens: every
//! client movement packet taken, every server-driven move's spline (its
//! start, its points, its end), casts and their ends, auras, damage, healing
//! and how each landed, deaths, threat on the elites, evades, summons,
//! teleports, each creature's victim and notice, charges, and the marks a
//! run puts in (`.record mark`).
//!
//! The map's thread only pushes events (`Map::record`): their times
//! stamped, onto a list taken once an update and sent down a channel to a
//! thread of the recorder's own, which writes them out. Nothing the map does
//! waits on the file.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ahash::AHashMap;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::creature::Motion;
use crate::game::map::death::Death;

/// The format's version, in each file's first line.
pub const VERSION: u32 = 1;

/// How often a unit that hasn't changed is written again: its keyframe.
const KEYFRAME: Duration = Duration::from_secs(1);
/// How far a unit moves before the ground under it is looked up again.
const GROUND_AGAIN: f32 = 0.05;
/// How many on a threat list are written, the highest first.
const THREAT_TOP: usize = 10;
/// How often units gone from the map are looked for.
const GONE_EVERY: Duration = Duration::from_secs(1);

/// One event, as the map knows it.
#[derive(Debug, Clone)]
pub enum Ev {
    /// The file's start: its map and copy, and why it records.
    Start {
        map: u32,
        instance: u32,
        every_ms: u64,
        why: &'static str,
    },
    /// The record stopped (`.record off`, the map stopping).
    End,
    /// A unit, the first time it is written.
    Unit(Box<Intro>),
    /// A unit's sample.
    Pos(Box<Row>),
    /// A unit left the map (logged out, despawned, sent elsewhere).
    Gone { g: Guid },
    /// A client movement packet the server took, as the client said it.
    Move {
        g: Guid,
        op: u16,
        at: Vector3d,
        o: f32,
        flags: u32,
        client_ms: u32,
        fall_ms: u32,
        dz: Option<f32>,
    },
    /// A server-driven move begun: its spline, and each point's height
    /// over the ground under it.
    Spline {
        g: Guid,
        id: u32,
        points: Vec<Vector3d>,
        dz: Vec<Option<f32>>,
        speed: f32,
        ms: u32,
        run: bool,
        fly: bool,
        kind: &'static str,
    },
    /// A server-driven move over: where it ended, and whether it ended
    /// before its spline did.
    Halt {
        g: Guid,
        id: u32,
        at: Vector3d,
        early: bool,
    },
    /// A cast begun (`ms` its cast time; 0, an instant).
    Cast {
        g: Guid,
        sp: u32,
        tg: Guid,
        at: Option<Vector3d>,
        ms: u32,
    },
    /// A cast gone off (`triggered`: by another, a proc or an aura).
    Go {
        g: Guid,
        sp: u32,
        tg: Guid,
        triggered: bool,
    },
    /// A cast refused or broken off, and why (a `SpellCastResult`).
    Fail { g: Guid, sp: u32, why: u8 },
    /// A channel begun.
    Channel { g: Guid, sp: u32, ms: u32 },
    /// A cast or channel ended before its time: `how` interrupted,
    /// cancelled.
    Stop {
        g: Guid,
        sp: u32,
        how: &'static str,
    },
    /// An aura put on (`on`, `refresh` if it was there) or taken off
    /// (`expired` if it ran out).
    Aura {
        g: Guid,
        by: Guid,
        sp: u32,
        stacks: u32,
        on: bool,
        refresh: bool,
        expired: bool,
    },
    /// Damage to a unit's health: what got through, and the health left.
    Dmg {
        by: Guid,
        g: Guid,
        sp: u32,
        school: u32,
        n: u32,
        left: u32,
        periodic: bool,
    },
    /// How a blow or a spell landed (`sp` 0 a swing), as its log tells it.
    Out {
        by: Guid,
        g: Guid,
        sp: u32,
        out: &'static str,
        n: u32,
        absorbed: u32,
        resisted: u32,
        blocked: u32,
    },
    /// Healing, and what of it was over the unit's health.
    Heal {
        by: Guid,
        g: Guid,
        sp: u32,
        n: u32,
        over: u32,
        crit: bool,
        periodic: bool,
    },
    Death { g: Guid, by: Guid, sp: u32 },
    /// A creature's threat list, the highest first, when it changed.
    Threat { g: Guid, list: Vec<(Guid, f32)> },
    /// A creature's victim changed (zero: none).
    Victim { g: Guid, from: Guid, to: Guid },
    /// A creature began to go home (`on`), or got there.
    Evade { g: Guid, on: bool, at: Vector3d },
    /// A creature noticed `who`, so far off: `how` its own way (`script`)
    /// or every creature's (`aggro`).
    Notice {
        g: Guid,
        who: Guid,
        yards: f32,
        how: &'static str,
    },
    /// A charge, and its way.
    Charge {
        g: Guid,
        tg: Guid,
        points: Vec<Vector3d>,
    },
    /// A creature come into the map summoned (or a pet).
    Summon { g: Guid, by: Guid, entry: u32 },
    /// A creature come back new, under its GUID (a respawn, `.respawn`).
    Respawn { g: Guid },
    /// A teleport, within the map or out of it (`map` where to).
    Tele {
        g: Guid,
        from: Vector3d,
        to: Vector3d,
        map: u32,
    },
    /// A run's mark, said by `by` (`.record mark <text>`).
    Mark { by: Guid, text: String },
}

/// What a unit is, written once.
#[derive(Debug, Clone)]
pub struct Intro {
    pub g: Guid,
    pub kind: &'static str,
    pub entry: u32,
    pub name: String,
    pub class: u8,
    pub race: u8,
    pub level: u32,
    pub rank: u32,
    pub owner: Guid,
    pub faction: u32,
}

/// A unit's sample.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub g: Guid,
    pub at: Vector3d,
    pub o: f32,
    pub flags: u32,
    pub speed: f32,
    pub hp: u32,
    pub max_hp: u32,
    pub power: u32,
    pub max_power: u32,
    /// Its selection (a player's) or victim (a creature's).
    pub target: Guid,
    pub dead: bool,
    /// Its height over the ground under it (the navmesh's nearest surface),
    /// if the ground is known there.
    pub dz: Option<f32>,
    pub form: u8,
    pub mounted: bool,
    pub stand: u8,
    pub evading: bool,
    pub moving: u32,
}

impl Row {
    /// Whether two samples are the same as the file would have them.
    fn same(&self, o: &Row) -> bool {
        let near = |a: f32, b: f32, e: f32| (a - b).abs() < e;
        near(self.at.x, o.at.x, 0.005)
            && near(self.at.y, o.at.y, 0.005)
            && near(self.at.z, o.at.z, 0.005)
            && near(self.o, o.o, 0.005)
            && self.flags == o.flags
            && self.hp == o.hp
            && self.max_hp == o.max_hp
            && self.power == o.power
            && self.target == o.target
            && self.dead == o.dead
            && self.form == o.form
            && self.mounted == o.mounted
            && self.stand == o.stand
            && self.evading == o.evading
            && self.moving == o.moving
    }
}

/// What the recorder last wrote of a unit.
#[derive(Debug, Default)]
struct Seen {
    row: Option<Row>,
    written: Option<Instant>,
    /// Where the ground was last looked up, and its height there.
    ground: Option<(Vector3d, Option<f32>)>,
    spline: u32,
    spline_ends: Option<Instant>,
    victim: Guid,
    evading: bool,
    threat: Vec<(Guid, f32)>,
}

/// A map copy's recorder.
pub struct Recorder {
    tx: Option<mpsc::Sender<Vec<(u64, Ev)>>>,
    writer: Option<std::thread::JoinHandle<()>>,
    events: parking_lot::Mutex<Vec<(u64, Ev)>>,
    /// The instant and the Unix time in ms it was, for each event's time.
    epoch: (Instant, u64),
    every: Duration,
    next_sample: Instant,
    next_gone: Instant,
    seen: AHashMap<Guid, Seen>,
    /// The file's name, for `.record`'s answers.
    pub file: String,
    pub since: Instant,
    /// Events pushed, and lines the samples wrote.
    pub events_pushed: u64,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder").field("file", &self.file).finish()
    }
}

/// A file name a GM may give: letters, digits, `-`, `_` and `.`, not
/// starting with a dot, at most 100 long; `.jsonl` added if it has no
/// extension.
pub fn file_name(name: &str) -> Option<String> {
    let ok = !name.is_empty()
        && name.len() <= 100
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !ok {
        return None;
    }
    Some(if name.contains('.') {
        name.to_owned()
    } else {
        format!("{name}.jsonl")
    })
}

/// The default file name for a copy, from its key and the time.
fn default_name(map: u32, instance: u32) -> String {
    let now = chrono::Utc::now();
    format!(
        "map{map}-{instance}-{}.jsonl",
        now.format("%Y%m%d-%H%M%S%.3f")
    )
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl Recorder {
    /// A recorder writing to `dir/name` (made if need be): `None`, and
    /// logged, if the file can't be made.
    pub fn open(
        dir: &Path,
        name: Option<&str>,
        map: u32,
        instance: u32,
        every: Duration,
        why: &'static str,
    ) -> Result<Box<Self>, String> {
        let name = name.map_or_else(|| default_name(map, instance), str::to_owned);
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path: PathBuf = dir.join(&name);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let (tx, rx) = mpsc::channel::<Vec<(u64, Ev)>>();
        let writer = std::thread::Builder::new()
            .name(format!("record {map}/{instance}"))
            .spawn(move || write_all(file, rx))
            .map_err(|e| format!("the recorder's thread: {e}"))?;
        let now = Instant::now();
        let r = Box::new(Self {
            tx: Some(tx),
            writer: Some(writer),
            events: parking_lot::Mutex::new(Vec::with_capacity(1024)),
            epoch: (now, unix_ms()),
            every: every.max(Duration::from_millis(10)),
            next_sample: now,
            next_gone: now + GONE_EVERY,
            seen: AHashMap::new(),
            file: name,
            since: now,
            events_pushed: 0,
        });
        r.push(Ev::Start {
            map,
            instance,
            every_ms: every.as_millis() as u64,
            why,
        });
        Ok(r)
    }

    /// The Unix time in ms of `at`.
    fn ms(&self, at: Instant) -> u64 {
        self.epoch.1
            + at.saturating_duration_since(self.epoch.0).as_millis() as u64
    }

    /// An event, now.
    pub fn push(&self, ev: Ev) {
        let t = self.ms(Instant::now());
        self.events.lock().push((t, ev));
    }

    /// What has been pushed, sent on to be written.
    fn flush(&mut self) {
        let events = std::mem::take(&mut *self.events.lock());
        if events.is_empty() {
            return;
        }
        self.events_pushed += events.len() as u64;
        if let Some(tx) = &self.tx {
            // A writer gone (its file's disk full) loses the rest.
            let _ = tx.send(events);
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.push(Ev::End);
        self.flush();
        // The writer ends once it has written what was sent.
        drop(self.tx.take());
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
    }
}

/// The recorder's thread: each batch written as it comes, flushed once
/// none is waiting.
fn write_all(file: std::fs::File, rx: mpsc::Receiver<Vec<(u64, Ev)>>) {
    let mut out = std::io::BufWriter::with_capacity(256 * 1024, file);
    let mut line = String::with_capacity(512);
    let mut failed = false;
    while let Ok(batch) = rx.recv() {
        let mut batches = vec![batch];
        while let Ok(b) = rx.try_recv() {
            batches.push(b);
        }
        for (t, ev) in batches.into_iter().flatten() {
            line.clear();
            format(&mut line, t, &ev);
            line.push('\n');
            if !failed && let Err(e) = out.write_all(line.as_bytes()) {
                tracing::warn!("the fight record: {e}; nothing more is written");
                failed = true;
            }
        }
        if !failed && let Err(e) = out.flush() {
            tracing::warn!("the fight record: {e}; nothing more is written");
            failed = true;
        }
    }
    let _ = out.flush();
}

fn guid(g: Guid) -> String {
    format!("{:x}", g.guid())
}

/// A finite number to `places` places, else `null`.
fn num(s: &mut String, v: f32, places: usize) {
    if v.is_finite() {
        let _ = write!(s, "{v:.places$}");
    } else {
        s.push_str("null");
    }
}

fn point(s: &mut String, p: Vector3d) {
    s.push('[');
    num(s, p.x, 2);
    s.push(',');
    num(s, p.y, 2);
    s.push(',');
    num(s, p.z, 2);
    s.push(']');
}

fn points(s: &mut String, ps: &[Vector3d]) {
    s.push('[');
    for (i, p) in ps.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        point(s, *p);
    }
    s.push(']');
}

fn text(s: &mut String, t: &str) {
    s.push_str(&serde_json::to_string(t).unwrap_or_else(|_| "\"\"".into()));
}

/// A client's movement flags, every bit of them, as 1.12 lays them out
/// (`MovementFlags`): forward 0x1, backward 0x2, strafes 0x4 and 0x8, turns
/// 0x10 and 0x20, walking 0x100, root 0x1000, jumping 0x2000, falling far
/// 0x4000, swimming 0x200000, on a transport 0x2000000.
pub fn move_bits(f: &wow_world_messages::vanilla::MovementInfo_MovementFlags) -> u32 {
    let mut b = 0;
    for (on, bit) in [
        (f.get_forward(), 0x1),
        (f.get_backward(), 0x2),
        (f.get_strafe_left(), 0x4),
        (f.get_strafe_right(), 0x8),
        (f.get_turn_left(), 0x10),
        (f.get_turn_right(), 0x20),
        (f.get_pitch_up(), 0x40),
        (f.get_pitch_down(), 0x80),
        (f.get_walk_mode(), 0x100),
        (f.get_levitating(), 0x400),
        (f.get_fixed_z(), 0x800),
        (f.get_root(), 0x1000),
        (f.get_jumping().is_some(), 0x2000),
        (f.get_fallingfar(), 0x4000),
        (f.get_swimming().is_some(), 0x20_0000),
        (f.get_spline_enabled(), 0x40_0000),
        (f.get_can_fly(), 0x80_0000),
        (f.get_flying(), 0x100_0000),
        (f.get_on_transport().is_some(), 0x200_0000),
        (f.get_spline_elevation().is_some(), 0x400_0000),
        (f.get_waterwalking(), 0x1000_0000),
        (f.get_safe_fall(), 0x2000_0000),
        (f.get_hover(), 0x4000_0000),
    ] {
        if on {
            b |= bit;
        }
    }
    b
}

/// The name of a movement opcode the server takes from a client.
pub fn opcode_name(op: u16) -> &'static str {
    match op {
        0x0B5 => "MSG_MOVE_START_FORWARD",
        0x0B6 => "MSG_MOVE_START_BACKWARD",
        0x0B7 => "MSG_MOVE_STOP",
        0x0B8 => "MSG_MOVE_START_STRAFE_LEFT",
        0x0B9 => "MSG_MOVE_START_STRAFE_RIGHT",
        0x0BA => "MSG_MOVE_STOP_STRAFE",
        0x0BB => "MSG_MOVE_JUMP",
        0x0BC => "MSG_MOVE_START_TURN_LEFT",
        0x0BD => "MSG_MOVE_START_TURN_RIGHT",
        0x0BE => "MSG_MOVE_STOP_TURN",
        0x0BF => "MSG_MOVE_START_PITCH_UP",
        0x0C0 => "MSG_MOVE_START_PITCH_DOWN",
        0x0C1 => "MSG_MOVE_STOP_PITCH",
        0x0C2 => "MSG_MOVE_SET_RUN_MODE",
        0x0C3 => "MSG_MOVE_SET_WALK_MODE",
        0x0C9 => "MSG_MOVE_FALL_LAND",
        0x0CA => "MSG_MOVE_START_SWIM",
        0x0CB => "MSG_MOVE_STOP_SWIM",
        0x0DA => "MSG_MOVE_SET_FACING",
        0x0DB => "MSG_MOVE_SET_PITCH",
        0x0E9 => "MSG_MOVE_ROOT",
        0x0EA => "MSG_MOVE_UNROOT",
        0x0EE => "MSG_MOVE_HEARTBEAT",
        0x0F1 => "MSG_MOVE_KNOCK_BACK",
        _ => "",
    }
}

/// A `SpellCastResult`'s name, as wow_messages has it.
fn cast_result(why: u8) -> String {
    wow_world_messages::vanilla::SpellCastResult::try_from(why)
        .map_or_else(|_| format!("{why}"), |r| format!("{r:?}"))
}

/// One event's line, without its newline.
pub fn format(s: &mut String, t: u64, ev: &Ev) {
    let _ = write!(s, "{{\"t\":{t},");
    match ev {
        Ev::Start {
            map,
            instance,
            every_ms,
            why,
        } => {
            let _ = write!(
                s,
                "\"e\":\"start\",\"v\":{VERSION},\"map\":{map},\"instance\":{instance},\"every_ms\":{every_ms},\"why\":\"{why}\",\"wvs\":\"{}\"",
                env!("CARGO_PKG_VERSION")
            );
        }
        Ev::End => s.push_str("\"e\":\"end\""),
        Ev::Unit(u) => {
            let _ = write!(
                s,
                "\"e\":\"unit\",\"g\":\"{}\",\"kind\":\"{}\",\"entry\":{},\"name\":",
                guid(u.g),
                u.kind,
                u.entry
            );
            text(s, &u.name);
            let _ = write!(
                s,
                ",\"class\":{},\"race\":{},\"level\":{},\"rank\":{},\"faction\":{}",
                u.class, u.race, u.level, u.rank, u.faction
            );
            if !u.owner.is_zero() {
                let _ = write!(s, ",\"owner\":\"{}\"", guid(u.owner));
            }
        }
        Ev::Pos(r) => {
            let _ = write!(s, "\"e\":\"pos\",\"g\":\"{}\",\"p\":", guid(r.g));
            point(s, r.at);
            s.push_str(",\"o\":");
            num(s, r.o, 3);
            let _ = write!(s, ",\"fl\":{},\"sp\":", r.flags);
            num(s, r.speed, 2);
            let _ = write!(
                s,
                ",\"hp\":{},\"mhp\":{},\"pw\":{},\"mpw\":{}",
                r.hp, r.max_hp, r.power, r.max_power
            );
            if !r.target.is_zero() {
                let _ = write!(s, ",\"tg\":\"{}\"", guid(r.target));
            }
            if r.dead {
                s.push_str(",\"dead\":1");
            }
            if let Some(dz) = r.dz {
                s.push_str(",\"dz\":");
                num(s, dz, 2);
            }
            if r.form != 0 {
                let _ = write!(s, ",\"form\":{}", r.form);
            }
            if r.mounted {
                s.push_str(",\"mount\":1");
            }
            if r.stand != 0 {
                let _ = write!(s, ",\"stand\":{}", r.stand);
            }
            if r.evading {
                s.push_str(",\"evade\":1");
            }
            if r.moving != 0 {
                let _ = write!(s, ",\"spline\":{}", r.moving);
            }
        }
        Ev::Gone { g } => {
            let _ = write!(s, "\"e\":\"gone\",\"g\":\"{}\"", guid(*g));
        }
        Ev::Move {
            g,
            op,
            at,
            o,
            flags,
            client_ms,
            fall_ms,
            dz,
        } => {
            let _ = write!(
                s,
                "\"e\":\"move\",\"g\":\"{}\",\"op\":\"{}\",\"p\":",
                guid(*g),
                opcode_name(*op)
            );
            point(s, *at);
            s.push_str(",\"o\":");
            num(s, *o, 3);
            let _ = write!(s, ",\"fl\":{flags},\"ct\":{client_ms}");
            if *fall_ms != 0 {
                let _ = write!(s, ",\"fall\":{fall_ms}");
            }
            if let Some(dz) = dz {
                s.push_str(",\"dz\":");
                num(s, *dz, 2);
            }
        }
        Ev::Spline {
            g,
            id,
            points: ps,
            dz,
            speed,
            ms,
            run,
            fly,
            kind,
        } => {
            let _ = write!(
                s,
                "\"e\":\"spline\",\"g\":\"{}\",\"id\":{id},\"kind\":\"{kind}\",\"ms\":{ms},\"sp\":",
                guid(*g)
            );
            num(s, *speed, 2);
            let _ = write!(s, ",\"run\":{},\"fly\":{},\"pts\":", u8::from(*run), u8::from(*fly));
            points(s, ps);
            if dz.iter().any(Option::is_some) {
                s.push_str(",\"dz\":[");
                for (i, d) in dz.iter().enumerate() {
                    if i > 0 {
                        s.push(',');
                    }
                    match d {
                        Some(d) => num(s, *d, 2),
                        None => s.push_str("null"),
                    }
                }
                s.push(']');
            }
        }
        Ev::Halt { g, id, at, early } => {
            let _ = write!(s, "\"e\":\"halt\",\"g\":\"{}\",\"id\":{id},\"p\":", guid(*g));
            point(s, *at);
            let _ = write!(s, ",\"early\":{}", u8::from(*early));
        }
        Ev::Cast { g, sp, tg, at, ms } => {
            let _ = write!(s, "\"e\":\"cast\",\"g\":\"{}\",\"sp\":{sp},\"ms\":{ms}", guid(*g));
            if !tg.is_zero() {
                let _ = write!(s, ",\"tg\":\"{}\"", guid(*tg));
            }
            if let Some(at) = at {
                s.push_str(",\"at\":");
                point(s, *at);
            }
        }
        Ev::Go { g, sp, tg, triggered } => {
            let _ = write!(s, "\"e\":\"go\",\"g\":\"{}\",\"sp\":{sp}", guid(*g));
            if *triggered {
                s.push_str(",\"trig\":1");
            }
            if !tg.is_zero() {
                let _ = write!(s, ",\"tg\":\"{}\"", guid(*tg));
            }
        }
        Ev::Fail { g, sp, why } => {
            let _ = write!(
                s,
                "\"e\":\"fail\",\"g\":\"{}\",\"sp\":{sp},\"why\":{why},\"why_s\":\"{}\"",
                guid(*g),
                cast_result(*why)
            );
        }
        Ev::Channel { g, sp, ms } => {
            let _ = write!(s, "\"e\":\"channel\",\"g\":\"{}\",\"sp\":{sp},\"ms\":{ms}", guid(*g));
        }
        Ev::Stop { g, sp, how } => {
            let _ = write!(s, "\"e\":\"stop\",\"g\":\"{}\",\"sp\":{sp},\"how\":\"{how}\"", guid(*g));
        }
        Ev::Aura {
            g,
            by,
            sp,
            stacks,
            on,
            refresh,
            expired,
        } => {
            let _ = write!(
                s,
                "\"e\":\"aura\",\"g\":\"{}\",\"by\":\"{}\",\"sp\":{sp},\"n\":{stacks},\"on\":{}",
                guid(*g),
                guid(*by),
                u8::from(*on)
            );
            if *refresh {
                s.push_str(",\"refresh\":1");
            }
            if *expired {
                s.push_str(",\"expired\":1");
            }
        }
        Ev::Dmg {
            by,
            g,
            sp,
            school,
            n,
            left,
            periodic,
        } => {
            let _ = write!(
                s,
                "\"e\":\"dmg\",\"by\":\"{}\",\"g\":\"{}\",\"sp\":{sp},\"sc\":{school},\"n\":{n},\"left\":{left}",
                guid(*by),
                guid(*g)
            );
            if *periodic {
                s.push_str(",\"per\":1");
            }
        }
        Ev::Out {
            by,
            g,
            sp,
            out,
            n,
            absorbed,
            resisted,
            blocked,
        } => {
            let _ = write!(
                s,
                "\"e\":\"out\",\"by\":\"{}\",\"g\":\"{}\",\"sp\":{sp},\"out\":\"{out}\",\"n\":{n}",
                guid(*by),
                guid(*g)
            );
            if *absorbed > 0 {
                let _ = write!(s, ",\"abs\":{absorbed}");
            }
            if *resisted > 0 {
                let _ = write!(s, ",\"res\":{resisted}");
            }
            if *blocked > 0 {
                let _ = write!(s, ",\"blk\":{blocked}");
            }
        }
        Ev::Heal {
            by,
            g,
            sp,
            n,
            over,
            crit,
            periodic,
        } => {
            let _ = write!(
                s,
                "\"e\":\"heal\",\"by\":\"{}\",\"g\":\"{}\",\"sp\":{sp},\"n\":{n},\"over\":{over}",
                guid(*by),
                guid(*g)
            );
            if *crit {
                s.push_str(",\"crit\":1");
            }
            if *periodic {
                s.push_str(",\"per\":1");
            }
        }
        Ev::Death { g, by, sp } => {
            let _ = write!(s, "\"e\":\"death\",\"g\":\"{}\",\"by\":\"{}\"", guid(*g), guid(*by));
            if *sp != 0 {
                let _ = write!(s, ",\"sp\":{sp}");
            }
        }
        Ev::Threat { g, list } => {
            let _ = write!(s, "\"e\":\"threat\",\"g\":\"{}\",\"l\":[", guid(*g));
            for (i, (t, v)) in list.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                let _ = write!(s, "[\"{}\",", guid(*t));
                num(s, *v, 1);
                s.push(']');
            }
            s.push(']');
        }
        Ev::Victim { g, from, to } => {
            let _ = write!(
                s,
                "\"e\":\"victim\",\"g\":\"{}\",\"from\":\"{}\",\"to\":\"{}\"",
                guid(*g),
                guid(*from),
                guid(*to)
            );
        }
        Ev::Evade { g, on, at } => {
            let _ = write!(s, "\"e\":\"evade\",\"g\":\"{}\",\"on\":{},\"p\":", guid(*g), u8::from(*on));
            point(s, *at);
        }
        Ev::Notice { g, who, yards, how } => {
            let _ = write!(
                s,
                "\"e\":\"notice\",\"g\":\"{}\",\"who\":\"{}\",\"how\":\"{how}\",\"yd\":",
                guid(*g),
                guid(*who)
            );
            num(s, *yards, 2);
        }
        Ev::Charge { g, tg, points: ps } => {
            let _ = write!(
                s,
                "\"e\":\"charge\",\"g\":\"{}\",\"tg\":\"{}\",\"pts\":",
                guid(*g),
                guid(*tg)
            );
            points(s, ps);
        }
        Ev::Summon { g, by, entry } => {
            let _ = write!(
                s,
                "\"e\":\"summon\",\"g\":\"{}\",\"by\":\"{}\",\"entry\":{entry}",
                guid(*g),
                guid(*by)
            );
        }
        Ev::Respawn { g } => {
            let _ = write!(s, "\"e\":\"respawn\",\"g\":\"{}\"", guid(*g));
        }
        Ev::Tele { g, from, to, map } => {
            let _ = write!(s, "\"e\":\"tele\",\"g\":\"{}\",\"map\":{map},\"from\":", guid(*g));
            point(s, *from);
            s.push_str(",\"to\":");
            point(s, *to);
        }
        Ev::Mark { by, text: t } => {
            let _ = write!(s, "\"e\":\"mark\",\"by\":\"{}\",\"text\":", guid(*by));
            text(s, t);
        }
    }
    s.push('}');
}

impl Map {
    /// Record what `ev` makes, if this copy is recording: `ev` isn't run
    /// when it isn't.
    #[inline]
    pub fn record(&self, ev: impl FnOnce() -> Ev) {
        if let Some(r) = &self.record {
            r.push(ev());
        }
    }

    /// Whether this copy is recording.
    #[inline]
    pub fn recording(&self) -> bool {
        self.record.is_some()
    }
}

/// The recorder a new copy of `map` starts with: one if `[record] maps`
/// names it.
pub fn for_new_map(cfg: &crate::config::RecordConfig, map: u32, instance: u32) -> Option<Box<Recorder>> {
    if !cfg.maps.contains(&map) {
        return None;
    }
    match Recorder::open(
        &cfg.dir,
        None,
        map,
        instance,
        Duration::from_millis(cfg.every_ms),
        "config",
    ) {
        Ok(r) => {
            tracing::info!("map {map} #{instance}: recording to {}", cfg.dir.join(&r.file).display());
            Some(r)
        }
        Err(e) => {
            tracing::warn!("map {map} #{instance}: no fight record: {e}");
            None
        }
    }
}

/// The ground's height under `at`: the navmesh's surface nearest it.
fn ground_under(map: &mut Map, at: Vector3d) -> Option<f32> {
    map.nav.as_mut()?.ground(at.x, at.y, at.z)
}

/// The height of `at` over the ground under it, if the ground is known.
pub fn height_over_ground(map: &mut Map, at: Vector3d) -> Option<f32> {
    ground_under(map, at).map(|z| at.z - z)
}

/// The speed a player moves at, by what its flags say it does.
fn player_speed(p: &crate::game::map::player::Player) -> f32 {
    let f = &p.movement.flags;
    if !(f.get_forward() || f.get_backward() || f.get_strafe_left() || f.get_strafe_right()) {
        return 0.0;
    }
    if f.get_swimming().is_some() {
        if f.get_backward() {
            p.speeds.swim_back
        } else {
            p.speeds.swim
        }
    } else if f.get_backward() {
        p.speeds.run_back
    } else if f.get_walk_mode() {
        p.speeds.walk
    } else {
        p.speeds.run
    }
}

fn unit_row(g: Guid, f: &crate::game::fields::Fields) -> (u32, u32, u32, u32, u8, bool, u8) {
    let power_type = f.get_byte(UNIT_FIELD_BYTES_0, 3).min(4) as u16;
    (
        f.get_u32(UNIT_FIELD_HEALTH),
        f.get_u32(UNIT_FIELD_MAXHEALTH),
        f.get_u32(UNIT_FIELD_POWER1 + power_type),
        f.get_u32(UNIT_FIELD_MAXPOWER1 + power_type),
        f.get_byte(UNIT_FIELD_BYTES_1, 2),
        f.get_u32(UNIT_FIELD_MOUNTDISPLAYID) != 0 && !g.is_zero(),
        f.get_byte(UNIT_FIELD_BYTES_1, 0),
    )
}

fn player_intro(p: &crate::game::map::player::Player) -> Intro {
    Intro {
        g: p.guid,
        kind: "player",
        entry: 0,
        name: p.name.to_string(),
        class: p.row.class,
        race: p.row.race,
        level: u32::from(p.level()),
        rank: 0,
        owner: Guid::zero(),
        faction: p.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
    }
}

fn creature_intro(map: &Map, c: &crate::game::map::creature::Creature) -> Intro {
    let t = map.realm.data.creature_template(c.entry);
    let owner = c
        .owner
        .or_else(|| {
            [UNIT_FIELD_SUMMONEDBY, UNIT_FIELD_CREATEDBY, UNIT_FIELD_CHARMEDBY]
                .into_iter()
                .map(|i| c.fields.get_guid(i))
                .find(|g| !g.is_zero())
        })
        .unwrap_or_default();
    Intro {
        g: c.guid,
        kind: if c.pet.is_some() { "pet" } else { "creature" },
        entry: c.entry,
        name: t.map(|t| t.name.clone()).unwrap_or_default(),
        class: c.fields.get_byte(UNIT_FIELD_BYTES_0, 1),
        race: 0,
        level: c.fields.get_u32(UNIT_FIELD_LEVEL),
        rank: t.map_or(0, |t| t.rank),
        owner,
        faction: c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
    }
}

/// What kind of move a creature's spline is, by its state.
fn spline_kind(c: &crate::game::map::creature::Creature, w: &crate::game::map::creature::Walk) -> &'static str {
    if w.falling {
        "fall"
    } else if c.engagement.charge.is_some() {
        "charge"
    } else if c.evading {
        "home"
    } else if c.engagement.flee.is_some() {
        "flee"
    } else if c.combat.attacking {
        "chase"
    } else if c.pet.is_some() {
        "pet"
    } else if c
        .scripted
        .as_deref()
        .is_some_and(|s| s.scripted_move.is_some())
    {
        "script"
    } else if w.fly {
        "fly"
    } else {
        "walk"
    }
}

/// The update's look: each unit's spline, victim and evade as they change,
/// its sample when due, those gone; then what was pushed sent to be
/// written. Once an update, its last step.
pub fn update(map: &mut Map) {
    let Some(mut r) = map.record.take() else {
        return;
    };
    let now = map.now;
    let sample = now >= r.next_sample;
    if sample {
        r.next_sample = (r.next_sample + r.every).max(now);
    }
    observe_players(map, &mut r, now, sample);
    observe_creatures(map, &mut r, now, sample);
    if now >= r.next_gone {
        r.next_gone = now + GONE_EVERY;
        let gone: Vec<Guid> = r
            .seen
            .keys()
            .copied()
            .filter(|g| !map.players.contains_key(g) && map.creature(*g).is_none())
            .collect();
        for g in gone {
            r.seen.remove(&g);
            r.push(Ev::Gone { g });
        }
    }
    r.flush();
    map.record = Some(r);
}

/// A spline with each point's height over the ground under it.
fn grounded(map: &mut Map, ev: Ev) -> Ev {
    match ev {
        Ev::Spline {
            g,
            id,
            points,
            speed,
            ms,
            run,
            fly,
            kind,
            ..
        } => {
            let dz = points.iter().map(|p| height_over_ground(map, *p)).collect();
            Ev::Spline {
                g,
                id,
                points,
                dz,
                speed,
                ms,
                run,
                fly,
                kind,
            }
        }
        ev => ev,
    }
}

/// A row's height over the ground, looked up again only once it has moved.
fn dz_of(map: &mut Map, seen: &mut Seen, at: Vector3d) -> Option<f32> {
    if let Some((was, dz)) = seen.ground
        && (was.x - at.x).abs() < GROUND_AGAIN
        && (was.y - at.y).abs() < GROUND_AGAIN
        && (was.z - at.z).abs() < GROUND_AGAIN
    {
        return dz;
    }
    let dz = height_over_ground(map, at);
    seen.ground = Some((at, dz));
    dz
}

/// A row written if it changed, or its keyframe is due.
fn write_row(r: &mut Recorder, row: Row, now: Instant) {
    let seen = r.seen.entry(row.g).or_default();
    let changed = seen.row.as_ref().is_none_or(|o| !o.same(&row));
    let due = seen
        .written
        .is_none_or(|t| now.saturating_duration_since(t) >= KEYFRAME);
    if changed || due {
        seen.written = Some(now);
        seen.row = Some(row.clone());
        r.push(Ev::Pos(Box::new(row)));
    }
}

fn observe_players(map: &mut Map, r: &mut Recorder, now: Instant, sample: bool) {
    let guids: Vec<Guid> = map.players.keys().copied().collect();
    for g in guids {
        let Some(p) = map.players.get(&g) else {
            continue;
        };
        if !r.seen.contains_key(&g) {
            r.push(Ev::Unit(Box::new(player_intro(p))));
            r.seen.insert(g, Seen::default());
        }
        // A server-driven run: a fear's, a charge's.
        let walk = p
            .charge
            .as_ref()
            .and_then(|c| c.walk.as_ref())
            .map(|w| (w, "charge"))
            .or_else(|| p.driven.as_ref().and_then(|d| d.walk.as_ref()).map(|w| (w, "driven")))
            .filter(|(w, _)| !w.done(now));
        let spline = walk.map_or(0, |(w, _)| w.spline_id);
        let mut pending = None;
        let seen = r.seen.get(&g).map_or(0, |s| s.spline);
        if spline != seen {
            let at = p.position();
            if seen != 0 {
                let early = r
                    .seen
                    .get(&g)
                    .and_then(|s| s.spline_ends)
                    .is_some_and(|t| t > now + Duration::from_millis(60));
                r.push(Ev::Halt {
                    g,
                    id: seen,
                    at,
                    early,
                });
            }
            if let Some((w, kind)) = walk {
                pending = Some(Ev::Spline {
                    g,
                    id: w.spline_id,
                    points: w.points.clone(),
                    dz: Vec::new(),
                    speed: w.speed,
                    ms: w.duration.as_millis() as u32,
                    run: w.run,
                    fly: w.fly,
                    kind,
                });
            }
            let ends = walk.map(|(w, _)| w.started + w.duration);
            if let Some(s) = r.seen.get_mut(&g) {
                s.spline = spline;
                s.spline_ends = ends;
            }
        }
        if let Some(ev) = pending {
            r.push(grounded(map, ev));
        }
        if !sample {
            continue;
        }
        let Some(p) = map.players.get(&g) else {
            continue;
        };
        let (hp, max_hp, power, max_power, form, mounted, stand) = unit_row(g, &p.fields);
        let at = p.position();
        let mut row = Row {
            g,
            at,
            o: p.movement.orientation,
            flags: move_bits(&p.movement.flags),
            speed: player_speed(p),
            hp,
            max_hp,
            power,
            max_power,
            target: p.fields.get_guid(UNIT_FIELD_TARGET),
            dead: !matches!(p.death, Death::Alive),
            dz: None,
            form,
            mounted,
            stand,
            evading: false,
            moving: spline,
        };
        let mut seen = r.seen.remove(&g).unwrap_or_default();
        row.dz = dz_of(map, &mut seen, at);
        r.seen.insert(g, seen);
        write_row(r, row, now);
    }
}

fn observe_creatures(map: &mut Map, r: &mut Recorder, now: Instant, sample: bool) {
    let guids: Vec<Guid> = map.awake.iter().copied().collect();
    for g in guids {
        let Some(c) = map.creature(g) else {
            continue;
        };
        if !r.seen.contains_key(&g) {
            let intro = creature_intro(map, c);
            r.push(Ev::Unit(Box::new(intro)));
            r.seen.insert(g, Seen::default());
        }
        let walk = match &c.motion {
            Motion::Moving(w) if !w.done(now) => Some(w),
            _ => None,
        };
        let spline = walk.map_or(0, |w| w.spline_id);
        let (at, o) = match walk {
            Some(w) => w.position_at(now),
            None => (c.position, c.orientation),
        };
        let o = if walk.is_some_and(|w| w.falling) {
            c.orientation
        } else {
            o
        };
        let Some(seen) = r.seen.get(&g) else {
            continue;
        };
        let (was_spline, was_victim, was_evading, ends) =
            (seen.spline, seen.victim, seen.evading, seen.spline_ends);
        let mut evs = Vec::new();
        if spline != was_spline {
            if was_spline != 0 {
                evs.push(Ev::Halt {
                    g,
                    id: was_spline,
                    at,
                    early: ends.is_some_and(|t| t > now + Duration::from_millis(60)),
                });
            }
            if let Some(w) = walk {
                evs.push(Ev::Spline {
                    g,
                    id: w.spline_id,
                    points: w.points.clone(),
                    dz: Vec::new(),
                    speed: w.speed,
                    ms: w.duration.as_millis() as u32,
                    run: w.run,
                    fly: w.fly,
                    kind: spline_kind(c, w),
                });
            }
        }
        let victim = if c.combat.attacking {
            c.combat.victim
        } else {
            Guid::zero()
        };
        if victim != was_victim {
            evs.push(Ev::Victim {
                g,
                from: was_victim,
                to: victim,
            });
        }
        if c.evading != was_evading {
            evs.push(Ev::Evade {
                g,
                on: c.evading,
                at,
            });
        }
        // The elites' threat, at each sample, as it changes.
        let elite = sample
            && !c.threat.is_empty()
            && map
                .realm
                .data
                .creature_template(c.entry)
                .is_some_and(|t| t.rank >= 1);
        let threat = elite.then(|| {
            let mut l = c.threat.clone();
            l.sort_by(|a, b| b.1.total_cmp(&a.1));
            l.truncate(THREAT_TOP);
            l
        });
        let row = sample.then(|| {
            let (hp, max_hp, power, max_power, form, mounted, stand) = unit_row(g, &c.fields);
            Row {
                g,
                at,
                o,
                flags: c.move_flags,
                speed: walk.map_or(0.0, |w| w.speed),
                hp,
                max_hp,
                power,
                max_power,
                target: if victim.is_zero() {
                    c.fields.get_guid(UNIT_FIELD_TARGET)
                } else {
                    victim
                },
                dead: !c.alive(),
                dz: None,
                form,
                mounted,
                stand,
                evading: c.evading,
                moving: spline,
            }
        });
        let ends_now = walk.map(|w| w.started + w.duration);
        let evading = c.evading;
        for ev in evs {
            r.push(grounded(map, ev));
        }
        let mut seen = r.seen.remove(&g).unwrap_or_default();
        seen.spline = spline;
        seen.spline_ends = ends_now;
        seen.victim = victim;
        seen.evading = evading;
        if let Some(l) = threat {
            let changed = l.len() != seen.threat.len()
                || l.iter().zip(&seen.threat).any(|(a, b)| {
                    a.0 != b.0 || (a.1 - b.1).abs() > 0.01 * a.1.abs().max(1.0)
                });
            if changed {
                seen.threat = l.clone();
                r.push(Ev::Threat { g, list: l });
            }
        }
        if let Some(mut row) = row {
            row.dz = dz_of(map, &mut seen, row.at);
            r.seen.insert(g, seen);
            write_row(r, row, now);
        } else {
            r.seen.insert(g, seen);
        }
    }
}

/// A client's movement packet taken: its opcode read off the relay's frame
/// (its header's), and the height over the ground it says.
pub fn client_move(
    map: &mut Map,
    g: Guid,
    info: &wow_world_messages::vanilla::MovementInfo,
    frame: Option<&std::sync::Arc<[u8]>>,
) {
    if map.record.is_none() {
        return;
    }
    let op = frame
        .filter(|f| f.len() >= 4)
        .map_or(0, |f| u16::from_le_bytes([f[2], f[3]]));
    let dz = height_over_ground(map, info.position);
    map.record(|| Ev::Move {
        g,
        op,
        at: info.position,
        o: info.orientation,
        flags: move_bits(&info.flags),
        client_ms: info.timestamp,
        fall_ms: info.fall_time,
        dz,
    });
}

/// How a swing landed, by its outcome.
pub fn swing_outcome(o: crate::game::map::combat::Outcome, immune: bool) -> &'static str {
    use crate::game::map::combat::Outcome;
    if immune {
        return "immune";
    }
    match o {
        Outcome::Miss => "miss",
        Outcome::Dodge => "dodge",
        Outcome::Parry => "parry",
        Outcome::Block => "block",
        Outcome::Glancing => "glance",
        Outcome::Crushing => "crush",
        Outcome::Crit => "crit",
        Outcome::Hit => "hit",
    }
}

/// How a spell missed, by its hit's kind.
pub fn spell_outcome(h: crate::game::map::spells::outcome::Hit) -> &'static str {
    use crate::game::map::spells::outcome::Hit;
    match h {
        Hit::Hit => "hit",
        Hit::Miss => "miss",
        Hit::Resist => "resist",
        Hit::Dodge => "dodge",
        Hit::Parry => "parry",
        Hit::Block => "block",
        Hit::Blocked => "blocked",
        Hit::Evade => "evade",
        Hit::Immune => "immune",
        Hit::Deflect => "deflect",
        Hit::Absorb => "absorb",
        Hit::Reflect => "reflect",
    }
}

/// `.record`, `.record on [file]`, `.record off`, `.record mark <text>`: this
/// copy's record, started, stopped or marked.
pub fn command(map: &mut Map, g: Guid, args: &[&str]) -> String {
    match args {
        [] => match &map.record {
            Some(r) => format!(
                "Recording {} #{} to {} for {} s ({} events).",
                map.key.map,
                map.key.instance,
                r.file,
                r.since.elapsed().as_secs(),
                r.events_pushed
            ),
            None => format!("Not recording {} #{}.", map.key.map, map.key.instance),
        },
        ["on"] | ["on", _] => {
            if let Some(r) = &map.record {
                return format!("Already recording to {}.", r.file);
            }
            let name = match args.get(1) {
                Some(n) => match file_name(n) {
                    Some(n) => Some(n),
                    None => {
                        return "A record's name: letters, digits, '-', '_' and '.', not starting with '.'.".into();
                    }
                },
                None => None,
            };
            let cfg = map.cfg.record.clone();
            match Recorder::open(
                &cfg.dir,
                name.as_deref(),
                map.key.map,
                map.key.instance,
                Duration::from_millis(cfg.every_ms),
                "gm",
            ) {
                Ok(r) => {
                    let file = r.file.clone();
                    map.record = Some(r);
                    tracing::info!(
                        "{}: recording to {}, by {:x}",
                        map.key,
                        cfg.dir.join(&file).display(),
                        g.guid()
                    );
                    format!("Recording {} #{} to {file}.", map.key.map, map.key.instance)
                }
                Err(e) => {
                    tracing::warn!("{}: no fight record: {e}", map.key);
                    "The record couldn't be started; the server's log says why.".into()
                }
            }
        }
        ["off"] => match map.record.take() {
            Some(r) => {
                let file = r.file.clone();
                drop(r);
                format!("Recorded to {file}.")
            }
            None => "Not recording.".into(),
        },
        ["mark", rest @ ..] if !rest.is_empty() => {
            if map.record.is_none() {
                return "Not recording.".into();
            }
            let text: String = rest.join(" ").chars().take(200).collect();
            map.record(|| Ev::Mark {
                by: g,
                text: text.clone(),
            });
            format!("Marked: {text}")
        }
        _ => USAGE.into(),
    }
}

pub const USAGE: &str = "Use: .record, .record on [name], .record off, .record mark <text>";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_a_gm_may_give() {
        assert_eq!(file_name("av-1").as_deref(), Some("av-1.jsonl"));
        assert_eq!(file_name("av.log").as_deref(), Some("av.log"));
        assert_eq!(file_name("../x"), None);
        assert_eq!(file_name("a/b"), None);
        assert_eq!(file_name(".hidden"), None);
        assert_eq!(file_name(""), None);
    }

    #[test]
    fn lines_are_json() {
        let g = Guid::new(0xF130_0000_0100_0001);
        let evs = [
            Ev::Mark {
                by: g,
                text: "attempt \"1\" start".into(),
            },
            Ev::Pos(Box::new(Row {
                g,
                at: Vector3d {
                    x: 1.0,
                    y: 2.0,
                    z: f32::NAN,
                },
                o: 0.5,
                flags: 1,
                speed: 7.0,
                hp: 10,
                max_hp: 20,
                power: 0,
                max_power: 0,
                target: Guid::zero(),
                dead: false,
                dz: Some(0.31),
                form: 5,
                mounted: false,
                stand: 0,
                evading: false,
                moving: 3,
            })),
            Ev::Fail {
                g,
                sp: 133,
                why: 0x5F,
            },
            Ev::Threat {
                g,
                list: vec![(Guid::new(1), 10.0), (Guid::new(2), 5.5)],
            },
        ];
        for ev in &evs {
            let mut s = String::new();
            format(&mut s, 1, ev);
            let v: serde_json::Value = serde_json::from_str(&s).expect(&s);
            assert_eq!(v["t"], 1);
        }
    }
}
