//! Gurubashi Arena: the pit's free-for-all, the Booty Run's chest every
//! three hours, and the last man standing.
//!
//! **The Booty Run** (vmangos's event 16, every three hours from midnight
//! on the server's clock, [`crate::game::events::booty_run_on`]): when it
//! begins, Short John Mithril yells from the stands, puts away his gossip
//! and quests, and walks his entry's path (vmangos's
//! `creature_movement_template` for 14508, mangos-classic's `MoveWaypoint`
//! on `GAME_EVENT_GURUBASHI_ARENA`) down the stands' stairs to the pit's
//! floor. There (its point 14) he kneels and sets the Arena Treasure Chest
//! down, and the point's script (`creature_movement_scripts` 1450816) sets
//! him running, has him yell "Let the Bloodletting Begin!" and puts the
//! chest out; he runs back up to his place, where 1450833 sets him walking
//! and gives him back his gossip and quests. Whoever opens the chest first
//! has its loot; it goes when the event ends.
//!
//! He is kept awake on the way, seen or not. If he can't put the chest
//! down (dead, gone, with no path), it appears at its place when he would
//! have: the event's start plus his walk. A map that comes up during the
//! run (a restart) has him at home and the chest out at once.
//!
//! **Last man standing**, started by a GM (`.arena start`) or on a schedule:
//! - everyone alive on the pit's floor is a fighter; a countdown, then the
//!   fight, with no blows struck before it;
//! - death is the end: the fallen can't be brought back while it lasts, and
//!   their ghosts are sent up to the stands to watch;
//! - the announcer calls each kill to the zone with the fighters left, kill
//!   streaks, multi-kills and the final ten, three and two;
//! - after a while a ring of fire closes on the centre, and fighters
//!   outside it burn, harder at each step, so nobody can wait it out;
//! - the last one standing is crowned: the zone hears, the champion gets
//!   the Spirit of Zandalar, gold and a trinket, every fighter gets the
//!   standings, the result is kept, and the fallen rise in the stands.
//!
//! The battle checks its fighters once a second, by the grid around the
//! pit, so a hundred fighters cost no more than the pit's neighbours.

use std::time::{Duration, Instant};

use ahash::AHashMap;
use serde::{Deserialize, Serialize};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::death::Death;
use crate::game::map::{Map, MapKey};

pub const MAP: u32 = 0;
/// The pit's floor: the free-for-all. The stands are another area (1741).
pub const AREA_PIT: u32 = 2177;
pub const ZONE_STRANGLETHORN: u32 = 33;
/// The pit's centre, where the Booty Run's chest falls.
pub const CENTRE: Vector3d = Vector3d {
    x: -13202.9,
    y: 276.757,
    z: 21.857,
};
/// The floor runs flat to here; the stands rise beyond.
pub const PIT_RADIUS: f32 = 45.0;
/// The stands' first rows, where spectators watch from.
const STANDS_RADIUS: f32 = 60.0;
const STANDS_Z: f32 = 40.3;
/// Whoever is this near and within the zone hears the announcer.
const HEARING: f32 = 160.0;

pub const ANNOUNCER_ENTRY: u32 = 14508;
pub const ANNOUNCER_SPAWN: u32 = 636;
pub const ANNOUNCER_NAME: &str = "Short John Mithril";
pub const BOOTY_RUN_EVENT: u32 = 16;
pub const BOOTY_CHEST_SPAWN: u32 = 12029;
/// "Arrr, Me Hearties! I be havin' some extra Treasure..."
const BOOTY_RUN_TEXT: u32 = 9730;
/// The point of his path on the pit's floor, by the chest's place, whose
/// script sets the chest down.
pub const CHEST_POINT: u32 = 14;
/// `EMOTE_ONESHOT_KNEEL`: setting the chest down.
pub const EMOTE_KNEEL: u32 = 16;
/// How long he stays down at the chest's place: past the point's script's
/// two seconds to the chest (its own wait is one).
pub const KNEEL_HOLD: Duration = Duration::from_secs(3);
/// What he puts away for the run: gossip and quests.
const NPC_FLAGS_AWAY: u32 = 0x1 | 0x2;
/// Time given him past his walk before the chest is put out without him.
const CHEST_SLACK: Duration = Duration::from_secs(10);
/// The ring's visible burn.
pub const SPELL_RING_BURN: u32 = 20019;
const TICK: Duration = Duration::from_secs(1);
pub const KIND_ARENA: &str = "arena";
/// The most `[arena]`'s minutes (and `.arena start`'s) may be: a week.
pub const MAX_MINUTES: u64 = 7 * 24 * 60;
/// The most its seconds may be: an hour.
pub const MAX_SECS: u64 = 3600;

/// `now` and `d` later, or as late as an `Instant` goes: a time from the
/// config or a GM's command is never a panic.
fn later(now: Instant, d: Duration) -> Instant {
    now.checked_add(d)
        .or_else(|| now.checked_add(Duration::from_secs(u32::MAX.into())))
        .unwrap_or(now)
}

/// `[arena]` in the config.
pub use crate::config::ArenaConfig;

/// The pit's state on its map.
#[derive(Debug, Default)]
pub struct Arena {
    pub battle: Option<Battle>,
    tick_at: Option<Instant>,
    next_scheduled: Option<Instant>,
    pub booty: BootyRun,
    /// The fallen may be raised: the battle is ending.
    raising: bool,
}

/// The Booty Run's state on the map.
#[derive(Debug, Default)]
pub struct BootyRun {
    /// The event as last seen; `None` before the map's first look.
    on: Option<bool>,
    /// When the chest is put out if he hasn't put it down by then.
    pub chest_due: Option<Instant>,
    /// Short John on his run, kept awake.
    pub runner: Option<Guid>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Countdown { fight_at: Instant },
    Fight { since: Instant },
}

#[derive(Debug, Clone)]
pub struct Fighter {
    pub guid: Guid,
    pub name: String,
    pub race: u8,
    pub class: u8,
    pub level: u8,
    pub alive: bool,
    pub kills: u32,
    pub damage: u64,
    /// How long into the fight they fell, and how.
    pub fell: Option<(Duration, String)>,
    /// Their place, once they fall: the fighters left when they did.
    pub place: u32,
    off_floor_since: Option<Instant>,
    dead_since: Option<Instant>,
    burning: bool,
    last_kill: Option<Instant>,
    multi: u32,
}

#[derive(Debug, Clone)]
pub struct Battle {
    pub id: u64,
    pub phase: Phase,
    pub fighters: Vec<Fighter>,
    by_guid: AHashMap<Guid, usize>,
    pub alive: u32,
    /// The ring's radius, and when it next closes; none until it starts.
    pub ring: f32,
    pub(crate) ring_steps: u32,
    pub(crate) next_shrink: Option<Instant>,
    /// When the ring is at its smallest and burns its hardest.
    pub(crate) cap_at: Instant,
    announced: u64,
    roll_call_at: Instant,
    started_unix: i64,
}

impl Battle {
    pub fn fighter(&self, g: Guid) -> Option<&Fighter> {
        self.by_guid.get(&g).map(|i| &self.fighters[*i])
    }

    fn fighter_mut(&mut self, g: Guid) -> Option<&mut Fighter> {
        self.by_guid.get(&g).map(|i| &mut self.fighters[*i])
    }

    fn fighting(&self) -> bool {
        matches!(self.phase, Phase::Fight { .. })
    }

    fn elapsed(&self, now: Instant) -> Duration {
        match self.phase {
            Phase::Fight { since } => now.saturating_duration_since(since),
            Phase::Countdown { .. } => Duration::ZERO,
        }
    }
}

/// A finished battle, kept.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Outcome {
    pub id: u64,
    /// Unix seconds.
    pub ended: i64,
    pub champion: Option<String>,
    pub champion_class: u8,
    pub champion_race: u8,
    pub fighters: u32,
    pub seconds: u64,
    /// The standings: name, kills, damage, place.
    pub standings: Vec<(String, u32, u64, u32)>,
}

/// The champions, as kept.
#[derive(Debug, Default)]
pub struct History {
    pub results: Vec<Outcome>,
}

impl History {
    pub fn load(records: &[(String, u64, String)]) -> Self {
        let mut results: Vec<Outcome> = records
            .iter()
            .filter(|(k, _, _)| k == KIND_ARENA)
            .filter_map(|(_, _, d)| serde_json::from_str(d).ok())
            .collect();
        results.sort_by_key(|r| r.id);
        Self { results }
    }
}

pub fn class_name(class: u8) -> &'static str {
    match class {
        1 => "Warrior",
        2 => "Paladin",
        3 => "Hunter",
        4 => "Rogue",
        5 => "Priest",
        7 => "Shaman",
        8 => "Mage",
        9 => "Warlock",
        11 => "Druid",
        _ => "Fighter",
    }
}

pub fn race_name(race: u8) -> &'static str {
    match race {
        1 => "Human",
        2 => "Orc",
        3 => "Dwarf",
        4 => "Night Elf",
        5 => "Undead",
        6 => "Tauren",
        7 => "Gnome",
        8 => "Troll",
        _ => "",
    }
}

fn is_arena_map(map: &Map) -> bool {
    map.key == MapKey::world(MAP)
}

fn flat_dist(a: Vector3d, b: Vector3d) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// Whether a player stands on the pit's floor: the navmesh's area where it
/// knows it, else near the centre and below the stands.
pub fn on_floor(map: &Map, g: Guid) -> bool {
    let Some(p) = map.players.get(&g) else {
        return false;
    };
    let pos = p.position();
    if p.area != 0 {
        return p.area == AREA_PIT;
    }
    flat_dist(pos, CENTRE) <= PIT_RADIUS && pos.z < CENTRE.z + 8.0
}

/// A seat in the stands, spread around by `n`.
fn seat(map: &mut Map, n: usize) -> Vector3d {
    let a = n as f32 * 2.399_963;
    let (x, y) = (
        CENTRE.x + STANDS_RADIUS * a.cos(),
        CENTRE.y + STANDS_RADIUS * a.sin(),
    );
    let z = map
        .nav
        .as_mut()
        .and_then(|nav| nav.ground(x, y, STANDS_Z))
        .filter(|z| *z > CENTRE.z + 8.0)
        .unwrap_or(STANDS_Z);
    Vector3d { x, y, z }
}

fn facing_centre(at: Vector3d) -> f32 {
    (CENTRE.y - at.y).atan2(CENTRE.x - at.x)
}

/// Everyone who hears the announcer: the zone, and those near the pit.
fn hearers(map: &Map) -> Vec<Guid> {
    map.players
        .values()
        .filter(|p| p.zone == ZONE_STRANGLETHORN || flat_dist(p.position(), CENTRE) <= HEARING)
        .map(|p| p.guid)
        .collect()
}

fn announcer() -> Guid {
    guid::unit(ANNOUNCER_ENTRY, ANNOUNCER_SPAWN)
}

fn send_all(map: &Map, to: &[Guid], chat_type: SMSG_MESSAGECHAT_ChatType, text: &str) {
    let Some(frame) = crate::game::conn::encode(&SMSG_MESSAGECHAT {
        chat_type,
        language: Language::Universal,
        message: text.to_owned(),
        tag: PlayerChatTag::None,
    }) else {
        return;
    };
    for g in to {
        if let Some(p) = map.players.get(g) {
            p.outbound.send_frame(frame.clone());
        }
    }
}

/// The announcer yells to the zone.
pub fn yell(map: &Map, text: &str) {
    let to = hearers(map);
    send_all(
        map,
        &to,
        SMSG_MESSAGECHAT_ChatType::MonsterYell {
            sender1: announcer(),
            sender_name: ANNOUNCER_NAME.to_owned(),
            target: Guid::zero(),
        },
        text,
    );
}

/// A line across the middle of every hearer's screen.
pub fn shout(map: &Map, text: &str) {
    let to = hearers(map);
    send_all(
        map,
        &to,
        SMSG_MESSAGECHAT_ChatType::RaidBossEmote {
            monster: announcer(),
            monster_name: ANNOUNCER_NAME.to_owned(),
        },
        text,
    );
}

fn tell(map: &Map, g: Guid, text: &str) {
    if let Some(p) = map.players.get(&g) {
        crate::game::map::chat::send_system(&p.outbound, text);
    }
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// `.arena start [countdown] [minutes]`, `.arena stop`, `.arena status`,
/// `.arena history`.
pub fn command(map: &mut Map, g: Guid, args: &[&str]) -> String {
    let gm = map
        .players
        .get(&g)
        .is_some_and(|p| p.gm_level >= crate::accounts::GAME_MASTER);
    match args.first().copied().unwrap_or("status") {
        "start" if gm => {
            if !is_arena_map(map) {
                return "Gurubashi Arena is in Stranglethorn Vale.".into();
            }
            let config = map.cfg.arena.clone();
            let countdown = args
                .get(1)
                .and_then(|a| a.parse().ok())
                .unwrap_or(config.countdown_secs)
                .min(MAX_SECS);
            let minutes = args
                .get(2)
                .and_then(|a| a.parse().ok())
                .unwrap_or(config.max_minutes);
            match start(map, countdown, minutes) {
                Ok(n) => format!("The battle is on: {n} fighters, {countdown} s to the fight."),
                Err(e) => e,
            }
        }
        "stop" if gm => {
            if map.arena.battle.is_none() {
                return "There is no battle.".into();
            }
            yell(map, "Enough! The battle is called off. Everyone back to the stands!");
            end_battle(map, None);
            "The battle is called off.".into()
        }
        // The Booty Run now, as `.event start 16`.
        "booty" if gm => match crate::game::events::force(&map.realm, BOOTY_RUN_EVENT, true) {
            None => "There is no Booty Run in the world's data.".into(),
            Some(false) => "The Booty Run is already on.".into(),
            Some(true) => "The Booty Run begins: Short John Mithril is on his way.".into(),
        },
        "start" | "stop" | "booty" => "Only a GM may start or stop a battle or the Booty Run.".into(),
        "status" => status(map),
        "history" => history(map),
        _ => "Use: .arena start [countdown s] [max minutes], .arena stop, .arena booty, .arena status, .arena history".into(),
    }
}

fn status(map: &Map) -> String {
    let Some(b) = &map.arena.battle else {
        return "No battle is on in Gurubashi Arena.".into();
    };
    match b.phase {
        Phase::Countdown { fight_at } => format!(
            "{} fighters wait; the fight begins in {} s.",
            b.fighters.len(),
            fight_at.saturating_duration_since(map.now).as_secs()
        ),
        Phase::Fight { since } => {
            let t = map.now.saturating_duration_since(since).as_secs();
            let ring = if b.next_shrink.is_some() || b.ring < PIT_RADIUS {
                format!(", the ring at {:.0} yards", b.ring)
            } else {
                String::new()
            };
            format!(
                "{} of {} fighters stand after {}:{:02}{ring}.",
                b.alive,
                b.fighters.len(),
                t / 60,
                t % 60
            )
        }
    }
}

fn history(map: &Map) -> String {
    let h = map.realm.social.arena.lock();
    if h.results.is_empty() {
        return "Gurubashi Arena has crowned no champion yet.".into();
    }
    let mut lines = vec!["The last champions of Gurubashi Arena:".to_owned()];
    for r in h.results.iter().rev().take(5) {
        let when = chrono::DateTime::from_timestamp(r.ended, 0)
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default();
        let who = r.champion.as_deref().map_or_else(
            || "nobody survived".to_owned(),
            |c| {
                format!(
                    "{c} the {} {}",
                    race_name(r.champion_race),
                    class_name(r.champion_class)
                )
            },
        );
        lines.push(format!(
            "{when}: {who}, of {} fighters, in {}:{:02}",
            r.fighters,
            r.seconds / 60,
            r.seconds % 60
        ));
    }
    lines.join("\n")
}

/// Begin a battle with everyone alive on the floor: how many fight.
pub fn start(map: &mut Map, countdown_secs: u64, max_minutes: u64) -> Result<usize, String> {
    if map.arena.battle.is_some() {
        return Err("A battle is already on.".into());
    }
    let now = map.now;
    let min = map.cfg.arena.min_fighters.max(2) as usize;
    let mut fighters: Vec<Fighter> = map
        .grid
        .around(CENTRE.x, CENTRE.y, PIT_RADIUS + 5.0)
        .filter(|g| guid::is_player(*g))
        .filter(|g| on_floor(map, *g))
        .filter_map(|g| {
            let p = map.players.get(&g)?;
            (p.death == Death::Alive).then(|| Fighter {
                guid: g,
                name: p.name.to_string(),
                race: p.row.race,
                class: p.row.class,
                level: p.level(),
                alive: true,
                kills: 0,
                damage: 0,
                fell: None,
                place: 0,
                off_floor_since: None,
                dead_since: None,
                burning: false,
                last_kill: None,
                multi: 0,
            })
        })
        .collect();
    if fighters.len() < min {
        return Err(format!(
            "{} fighters stand in the pit; a battle needs {min}.",
            fighters.len()
        ));
    }
    fighters.sort_by(|a, b| a.name.cmp(&b.name));
    let by_guid = fighters
        .iter()
        .enumerate()
        .map(|(i, f)| (f.guid, i))
        .collect();
    let n = fighters.len();
    // A GM's numbers are held to the config's ranges.
    let countdown_secs = countdown_secs.min(MAX_SECS);
    let max_minutes = max_minutes.clamp(1, MAX_MINUTES);
    let countdown = Duration::from_secs(countdown_secs);
    let fight_at = later(now, countdown);
    map.arena.battle = Some(Battle {
        id: now_unix().max(1) as u64,
        phase: Phase::Countdown { fight_at },
        fighters,
        by_guid,
        alive: n as u32,
        ring: PIT_RADIUS,
        ring_steps: 0,
        next_shrink: None,
        cap_at: later(fight_at, Duration::from_secs(max_minutes * 60)),
        announced: countdown_secs + 1,
        roll_call_at: later(fight_at, Duration::from_secs(map.cfg.arena.roll_call_secs)),
        started_unix: now_unix(),
    });
    map.arena.raising = false;
    yell(
        map,
        &format!(
            "Hear me, Stranglethorn! {n} fighters stand in the blood of Gurubashi Arena, and only one walks out! The battle begins in {countdown_secs} seconds!"
        ),
    );
    let ids: Vec<Guid> = map
        .arena
        .battle
        .as_ref()
        .map(|b| b.fighters.iter().map(|f| f.guid).collect())
        .unwrap_or_default();
    for g in ids {
        tell(
            map,
            g,
            "You fight in Gurubashi Arena's last man standing: the last one alive wins. The fallen stay fallen until it ends, and the ring of fire closes on those who wait.",
        );
    }
    Ok(n)
}

/// What the pit does each update: the Booty Run, a scheduled battle, and
/// the battle's second.
pub fn update(map: &mut Map) {
    if !is_arena_map(map) {
        return;
    }
    let now = map.now;
    if map.arena.tick_at.is_some_and(|t| t > now) {
        // The countdown's numbers keep their own time.
        if map
            .arena
            .battle
            .as_ref()
            .is_some_and(|b| matches!(b.phase, Phase::Countdown { .. }))
        {
            countdown(map);
        }
        return;
    }
    map.arena.tick_at = Some(now + TICK);
    booty_run(map);
    schedule(map);
    if map.arena.battle.is_none() {
        return;
    }
    countdown(map);
    if !map.arena.battle.as_ref().is_some_and(Battle::fighting) {
        keep_off_the_floor(map);
        return;
    }
    keep_off_the_floor(map);
    check_fighters(map);
    ring(map);
    roll_call(map);
    if map.arena.battle.as_ref().is_some_and(|b| now >= b.cap_at) {
        // Time is up: the ring at its smallest, burning hard.
        if let Some(b) = map.arena.battle.as_mut()
            && b.ring > map.cfg.arena.ring_min_yards
        {
            b.ring = map.cfg.arena.ring_min_yards;
            b.ring_steps = b.ring_steps.max(10);
            yell(
                map,
                "Time is up! The fire takes the arena! Stand at the centre, or burn!",
            );
        }
    }
}

/// The Booty Run: Short John's walk as it begins, the chest, and the
/// chest gone as it ends.
fn booty_run(map: &mut Map) {
    let on = crate::game::events::current(&map.realm).contains(&BOOTY_RUN_EVENT);
    match (map.arena.booty.on.replace(on), on) {
        // The map came up during the run: he is at home, the chest out.
        (None, true) => put_chest(map),
        (Some(false), true) => begin_run(map),
        (Some(true), false) => end_run(map),
        _ => {}
    }
    let now = map.now;
    if let Some(due) = map.arena.booty.chest_due {
        if !on || chest(map).is_some() {
            map.arena.booty.chest_due = None;
        } else if now >= due {
            map.arena.booty.chest_due = None;
            put_chest(map);
        }
    }
    if let Some(g) = map.arena.booty.runner {
        run_step(map, g);
    }
}

fn chest(map: &Map) -> Option<Guid> {
    crate::game::map::scripts::object_of_spawn(map, BOOTY_CHEST_SPAWN)
}

/// The chest at its place on the floor, if it isn't out.
fn put_chest(map: &mut Map) {
    if chest(map).is_some() {
        return;
    }
    let data = map.realm.data.clone();
    if let Some(s) = data
        .gameobject_spawns
        .get(&MAP)
        .and_then(|v| v.iter().find(|s| s.guid == BOOTY_CHEST_SPAWN))
        && let Some(t) = data.gameobject_templates.get(&s.entry)
    {
        map.add_gameobject(crate::game::map::gameobject::GameObject::from_spawn(t, s));
    }
}

/// Short John, if he is here and alive.
pub fn short_john(map: &Map) -> Option<Guid> {
    crate::game::map::scripts::creature_of_spawn(map, ANNOUNCER_SPAWN)
        .filter(|g| map.creature(*g).is_some_and(|c| c.alive()))
}

/// The Booty Run begins: his yell, and his walk down to the floor.
fn begin_run(map: &mut Map) {
    let data = map.realm.data.clone();
    if let Some(text) = data.npc.broadcast_line(BOOTY_RUN_TEXT, false) {
        yell(map, text);
    }
    let path = data
        .waypoints_by_entry
        .get(&ANNOUNCER_ENTRY)
        .filter(|p| p.iter().any(|w| w.point == CHEST_POINT))
        .cloned();
    let (Some(g), Some(path)) = (short_john(map), path) else {
        put_chest(map);
        return;
    };
    let now = map.now;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    // His walk to the chest's place, point by point, at his walking pace.
    let mut at = c.position;
    let mut length = 0.0;
    for w in path
        .iter()
        .take_while(|w| w.point != CHEST_POINT)
        .chain(path.iter().find(|w| w.point == CHEST_POINT))
    {
        let to = Vector3d {
            x: w.x,
            y: w.y,
            z: w.z,
        };
        length += crate::game::map::combat::dist(at, to);
        at = to;
    }
    let walk =
        Duration::try_from_secs_f32(length / c.speeds.walk.max(0.1)).unwrap_or(Duration::ZERO);
    let flags = c.fields.get_u32(UNIT_NPC_FLAGS);
    c.fields.set_u32(UNIT_NPC_FLAGS, flags & !NPC_FLAGS_AWAY);
    c.movement_type = crate::game::map::creature::MOVEMENT_WAYPOINT;
    c.waypoint = 0;
    c.last_waypoint = None;
    if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
        s.route = Some(crate::game::map::event_ai::Route {
            points: std::sync::Arc::new(path),
            repeat: false,
            pathfind: true,
        });
        s.run = false;
        s.follow = None;
    }
    crate::game::map::ai::rest(map, g, Some(now));
    map.keep_awake(g, true);
    map.arena.booty.runner = Some(g);
    map.arena.booty.chest_due = Some(later(now, walk + KNEEL_HOLD + CHEST_SLACK));
}

/// He reached a point of his path (from the creatures' walk): at the
/// chest's he kneels and stays down while its script puts the chest out.
pub fn reached(map: &mut Map, g: Guid, point: u32) {
    if map.arena.booty.runner != Some(g) || point != CHEST_POINT {
        return;
    }
    crate::game::map::scripts::emote(map, g, EMOTE_KNEEL);
    let until = later(map.now, KNEEL_HOLD);
    if let Some(c) = map.creature_mut(g)
        && matches!(c.motion, crate::game::map::creature::Motion::Idle { .. })
    {
        c.motion = crate::game::map::creature::Motion::Idle { until: Some(until) };
    }
}

/// His run, checked: dead or gone, it is over; back home and off his
/// path, he is as he was.
fn run_step(map: &mut Map, g: Guid) {
    let Some(c) = map.creature(g).filter(|c| c.alive()) else {
        map.keep_awake(g, false);
        map.arena.booty.runner = None;
        return;
    };
    let walking = matches!(c.motion, crate::game::map::creature::Motion::Moving(_));
    if walking || c.movement_type == crate::game::map::creature::MOVEMENT_WAYPOINT {
        return;
    }
    let home_o = c.home_orientation;
    let npc_flags = map
        .realm
        .data
        .creature_template(ANNOUNCER_ENTRY)
        .map_or(NPC_FLAGS_AWAY, |t| t.npc_flags & NPC_FLAGS_AWAY);
    if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
        s.route = None;
        s.run = false;
    }
    if let Some(c) = map.creature_mut(g) {
        let flags = c.fields.get_u32(UNIT_NPC_FLAGS);
        c.fields.set_u32(UNIT_NPC_FLAGS, flags | npc_flags);
        c.movement_type = crate::game::map::creature::MOVEMENT_IDLE;
        c.orientation = home_o;
    }
    map.visibility_dirty.insert(g);
    map.keep_awake(g, false);
    map.arena.booty.runner = None;
}

/// The Booty Run ends: the chest goes, and he, if still on his way, runs
/// home.
fn end_run(map: &mut Map) {
    if let Some(g) = chest(map) {
        map.remove_gameobject(g);
    }
    map.arena.booty.chest_due = None;
    let Some(g) = map
        .arena
        .booty
        .runner
        .filter(|g| map.creature(*g).is_some_and(|c| c.alive()))
    else {
        return;
    };
    if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
        s.route = None;
    }
    if let Some(c) = map.creature_mut(g) {
        c.movement_type = crate::game::map::creature::MOVEMENT_IDLE;
    }
    let (home, o) = map
        .creature(g)
        .map(|c| (c.home, c.home_orientation))
        .unwrap_or((CENTRE, 0.0));
    crate::game::map::ai::rest(map, g, None);
    crate::game::map::scripts::move_with(
        map,
        g,
        home,
        crate::game::map::scripts::Move {
            run: true,
            pathfind: true,
            face: Some(o),
            ..Default::default()
        },
    );
}

/// A scheduled battle, when it's due and the pit has its fighters.
fn schedule(map: &mut Map) {
    let config = map.cfg.arena.clone();
    if config.every_minutes == 0 {
        return;
    }
    let now = map.now;
    let every = Duration::from_secs(config.every_minutes.min(MAX_MINUTES) * 60);
    let due = *map.arena.next_scheduled.get_or_insert(later(now, every));
    if now < due || map.arena.battle.is_some() {
        return;
    }
    map.arena.next_scheduled = Some(later(now, every));
    if start(map, config.countdown_secs, config.max_minutes).is_err() {
        yell(
            map,
            "The arena awaits fighters, and finds too few. Next time, cowards!",
        );
    }
}

/// The countdown's calls, and the fight's start.
fn countdown(map: &mut Map) {
    let now = map.now;
    let Some(b) = map.arena.battle.as_mut() else {
        return;
    };
    let Phase::Countdown { fight_at } = b.phase else {
        return;
    };
    let left = fight_at.saturating_duration_since(now);
    let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
    if secs > 0 {
        if secs < b.announced && matches!(secs, 1..=5 | 10 | 20 | 30 | 60) {
            b.announced = secs;
            let text = if secs <= 5 {
                format!("{secs}...")
            } else {
                format!("The battle begins in {secs} seconds!")
            };
            shout(map, &text);
        }
        return;
    }
    b.phase = Phase::Fight { since: now };
    let config = map.cfg.arena.clone();
    b.next_shrink = Some(later(now, Duration::from_secs(config.ring_after_secs)));
    b.roll_call_at = now + Duration::from_secs(map.cfg.arena.roll_call_secs);
    // Everyone starts whole.
    let ids: Vec<Guid> = b
        .fighters
        .iter()
        .filter(|f| f.alive)
        .map(|f| f.guid)
        .collect();
    for g in ids {
        crate::game::map::combat::leave_combat(map, g);
        if let Some(p) = map.players.get_mut(&g) {
            let max = p.fields.get_u32(UNIT_FIELD_MAXHEALTH);
            p.fields.set_u32(UNIT_FIELD_HEALTH, max);
            let mana = p.fields.get_u32(UNIT_FIELD_MAXPOWER1);
            p.fields.set_u32(UNIT_FIELD_POWER1, mana);
        }
        free_for_all(map, g);
    }
    shout(map, "FIGHT!");
    yell(map, "Let the blood of the Gurubashi flow! FIGHT!");
}

/// Spectators keep off the floor while a battle is on; the fallen go up to
/// the stands.
fn keep_off_the_floor(map: &mut Map) {
    let now = map.now;
    let Some(b) = map.arena.battle.as_ref() else {
        return;
    };
    let on_floor_now: Vec<Guid> = map
        .grid
        .around(CENTRE.x, CENTRE.y, PIT_RADIUS + 5.0)
        .filter(|g| guid::is_player(*g))
        .filter(|g| b.fighter(*g).is_none_or(|f| !f.alive))
        .filter(|g| {
            map.players.get(g).is_some_and(|p| {
                p.death == Death::Alive
                    && !p
                        .fields
                        .has_flag(PLAYER_FLAGS, crate::game::map::player::PLAYER_FLAGS_GM)
            })
        })
        .filter(|g| on_floor(map, *g))
        .collect();
    for (n, g) in on_floor_now.into_iter().enumerate() {
        let at = seat(map, n);
        tell(
            map,
            g,
            "A battle is on in the pit. Watch it from the stands!",
        );
        map.teleport(g, map.key, at, facing_centre(at));
    }
    // The fallen lie a moment, then their ghosts go up.
    let lying: Vec<Guid> = map
        .arena
        .battle
        .as_ref()
        .map(|b| {
            b.fighters
                .iter()
                .filter(|f| {
                    !f.alive
                        && f.dead_since.is_some_and(|t| {
                            now.duration_since(t) >= Duration::from_secs(map.cfg.arena.lie_secs)
                        })
                })
                .map(|f| f.guid)
                .collect()
        })
        .unwrap_or_default();
    for g in lying {
        if map
            .players
            .get(&g)
            .is_some_and(|p| matches!(p.death, Death::Dead { .. }))
        {
            crate::game::map::death::release(map, g);
        }
        if let Some(f) = map.arena.battle.as_mut().and_then(|b| b.fighter_mut(g)) {
            f.dead_since = None;
        }
    }
}

/// Fighters who left the floor or the world have fled.
fn check_fighters(map: &mut Map) {
    let now = map.now;
    let Some(b) = map.arena.battle.as_ref() else {
        return;
    };
    let alive: Vec<Guid> = b
        .fighters
        .iter()
        .filter(|f| f.alive)
        .map(|f| f.guid)
        .collect();
    let mut fled = Vec::new();
    let mut off: Vec<(Guid, bool)> = Vec::new();
    for g in alive {
        match map.players.get(&g) {
            None => fled.push(g),
            Some(p) if p.death != Death::Alive => {}
            Some(_) => off.push((g, !on_floor(map, g))),
        }
    }
    let off_floor = Duration::from_secs(map.cfg.arena.off_floor_secs);
    if let Some(b) = map.arena.battle.as_mut() {
        for (g, away) in off {
            if let Some(f) = b.fighter_mut(g) {
                if !away {
                    f.off_floor_since = None;
                } else if f.off_floor_since.get_or_insert(now).elapsed_since(now) >= off_floor {
                    fled.push(g);
                }
            }
        }
    }
    for g in fled {
        fall(map, g, None, Fall::Fled);
    }
}

trait SinceExt {
    fn elapsed_since(&self, now: Instant) -> Duration;
}

impl SinceExt for Instant {
    fn elapsed_since(&self, now: Instant) -> Duration {
        now.saturating_duration_since(*self)
    }
}

/// The ring closes, and burns whoever stands outside it.
fn ring(map: &mut Map) {
    let now = map.now;
    let config = map.cfg.arena.clone();
    let Some(b) = map.arena.battle.as_mut() else {
        return;
    };
    if b.next_shrink.is_some_and(|t| now >= t) && b.ring > config.ring_min_yards {
        let first = b.ring_steps == 0;
        b.ring =
            (b.ring - if first { 0.0 } else { config.ring_step_yards }).max(config.ring_min_yards);
        b.ring_steps += 1;
        b.next_shrink = Some(now + Duration::from_secs(config.ring_every_secs.max(1)));
        let r = b.ring;
        let text = if first {
            format!(
                "Enough waiting! A ring of fire closes on the arena: keep within {r:.0} yards of the centre, or burn!"
            )
        } else if r <= config.ring_min_yards {
            format!("The ring is at its smallest: {r:.0} yards! Nowhere left to hide!")
        } else {
            format!("The ring of fire closes in! {r:.0} yards!")
        };
        yell(map, &text);
    }
    let Some(b) = map.arena.battle.as_ref() else {
        return;
    };
    if b.ring_steps == 0 {
        return;
    }
    let (radius, steps) = (b.ring, b.ring_steps);
    let fighters: Vec<(Guid, bool)> = b
        .fighters
        .iter()
        .filter(|f| f.alive)
        .map(|f| (f.guid, f.burning))
        .collect();
    let data = map.realm.data.clone();
    for (g, was_burning) in fighters {
        let Some(p) = map.players.get(&g) else {
            continue;
        };
        if p.death != Death::Alive {
            continue;
        }
        let outside = flat_dist(p.position(), CENTRE) > radius;
        let max = p.fields.get_u32(UNIT_FIELD_MAXHEALTH);
        if outside != was_burning {
            if let Some(f) = map.arena.battle.as_mut().and_then(|b| b.fighter_mut(g)) {
                f.burning = outside;
            }
            match (outside, data.spells.get(SPELL_RING_BURN)) {
                (true, Some(s)) => {
                    let _ = crate::game::map::auras::apply_spell(
                        map,
                        g,
                        g,
                        s,
                        &[],
                        crate::game::map::auras::Context {
                            duration: Some(None),
                            ..Default::default()
                        },
                    );
                }
                (false, _) => crate::game::map::auras::remove_spell(map, g, SPELL_RING_BURN, None),
                _ => {}
            }
        }
        if outside {
            // A twentieth of health a second at first, more at each step.
            let pct = (3 + 2 * steps).min(25);
            let damage = (max * pct / 100).max(1);
            crate::game::map::environment::damage(map, g, EnvironmentalDamageType::Fire, damage);
        }
    }
}

/// The announcer counts the fighters now and then.
fn roll_call(map: &mut Map) {
    let now = map.now;
    let every = Duration::from_secs(map.cfg.arena.roll_call_secs);
    let Some(b) = map.arena.battle.as_mut() else {
        return;
    };
    if now < b.roll_call_at {
        return;
    }
    b.roll_call_at = now + every;
    let (alive, t) = (b.alive, b.elapsed(now).as_secs());
    if alive > 3 {
        shout(
            map,
            &format!(
                "{alive} fighters still stand after {}:{:02}!",
                t / 60,
                t % 60
            ),
        );
    }
}

/// How a fighter fell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fall {
    Slain,
    Burned,
    Fled,
    Fell,
}

/// A player died: if a fighter, they're out.
pub fn died(map: &mut Map, victim: Guid, killer: Guid) {
    let Some(b) = map.arena.battle.as_ref() else {
        return;
    };
    if !b.fighter(victim).is_some_and(|f| f.alive) {
        return;
    }
    // A pet's kill is its master's.
    let killer = map.creature(killer).and_then(|c| c.owner).unwrap_or(killer);
    let burning = b.fighter(victim).is_some_and(|f| f.burning);
    let (by, how) = if killer == victim {
        (None, if burning { Fall::Burned } else { Fall::Fell })
    } else if b.fighter(killer).is_some() {
        (Some(killer), Fall::Slain)
    } else {
        (None, Fall::Fell)
    };
    fall(map, victim, by, how);
    if let Some(f) = map
        .arena
        .battle
        .as_mut()
        .and_then(|b| b.fighter_mut(victim))
    {
        f.dead_since = Some(map.now);
    }
}

/// Damage between fighters, for the standings.
pub fn damaged(map: &mut Map, attacker: Guid, victim: Guid, damage: u32) {
    let Some(b) = map.arena.battle.as_mut() else {
        return;
    };
    if attacker == victim || !b.fighting() || b.fighter(victim).is_none() {
        return;
    }
    if let Some(f) = b.fighter_mut(attacker) {
        f.damage += u64::from(damage);
    }
}

/// A fighter is out: counted, called, and maybe the battle is over.
fn fall(map: &mut Map, g: Guid, by: Option<Guid>, how: Fall) {
    let now = map.now;
    let multi_kill = Duration::from_secs(map.cfg.arena.multi_kill_secs);
    let Some(b) = map.arena.battle.as_mut() else {
        return;
    };
    let t = b.elapsed(now);
    let left = b.alive.saturating_sub(1);
    let Some(f) = b.fighter_mut(g) else {
        return;
    };
    if !f.alive {
        return;
    }
    f.alive = false;
    f.place = left + 1;
    let victim = f.name.clone();
    let killer = by.and_then(|k| b.fighter(k).map(|f| f.name.clone()));
    let cause = match (how, &killer) {
        (Fall::Slain, Some(k)) => format!("slain by {k}"),
        (Fall::Burned, _) => "burned".to_owned(),
        (Fall::Fled, _) => "fled".to_owned(),
        _ => "fell".to_owned(),
    };
    if let Some(f) = b.fighter_mut(g) {
        f.fell = Some((t, cause));
        if f.burning {
            f.burning = false;
        }
    }
    b.alive = left;
    // The killer's streak and multi-kill.
    let mut extra = Vec::new();
    if let Some(k) = by
        && let Some(kf) = b.fighter_mut(k)
    {
        kf.kills += 1;
        kf.multi = if kf
            .last_kill
            .is_some_and(|at| now.duration_since(at) <= multi_kill)
        {
            kf.multi + 1
        } else {
            1
        };
        kf.last_kill = Some(now);
        let name = kf.name.clone();
        match kf.multi {
            2 => extra.push(format!("Double kill for {name}!")),
            3 => extra.push(format!("TRIPLE KILL! {name} is a whirlwind of death!")),
            n if n >= 4 => extra.push(format!("{name} has cut down {n} in a breath! MASSACRE!")),
            _ => {}
        }
        match kf.kills {
            3 => extra.push(format!("{name} is on a killing spree!")),
            5 => extra.push(format!("{name} is on a RAMPAGE!")),
            8 => extra.push(format!("{name} is UNSTOPPABLE!")),
            12 => extra.push(format!("{name} is GODLIKE! Will no one stop them?")),
            _ => {}
        }
    }
    let line = match how {
        Fall::Slain => format!(
            "{} has slain {victim}! {left} fighters remain.",
            killer.as_deref().unwrap_or("Someone")
        ),
        Fall::Burned => format!("{victim} was consumed by the flames! {left} fighters remain."),
        Fall::Fled => format!("{victim} has fled the arena! {left} fighters remain."),
        Fall::Fell => format!("{victim} has fallen! {left} fighters remain."),
    };
    crate::game::map::auras::remove_spell(map, g, SPELL_RING_BURN, None);
    free_for_all(map, g);
    yell(map, &line);
    for e in extra {
        shout(map, &e);
    }
    match left {
        10 => shout(map, "Only ten remain!"),
        3 => shout(map, "The final three!"),
        2 => {
            let names: Vec<String> = map
                .arena
                .battle
                .as_ref()
                .map(|b| {
                    b.fighters
                        .iter()
                        .filter(|f| f.alive)
                        .map(|f| f.name.clone())
                        .collect()
                })
                .unwrap_or_default();
            if names.len() == 2 {
                shout(
                    map,
                    &format!(
                        "It comes down to {} and {}! The final duel!",
                        names[0], names[1]
                    ),
                );
            }
        }
        _ => {}
    }
    if left <= 1 {
        let champion = map
            .arena
            .battle
            .as_ref()
            .and_then(|b| b.fighters.iter().find(|f| f.alive).map(|f| f.guid));
        end_battle(map, Some(champion));
    }
}

/// The battle ends: `Some(champion)` (or `Some(None)`, all fell) when won,
/// `None` when called off. The fallen rise in the stands.
fn end_battle(map: &mut Map, won: Option<Option<Guid>>) {
    let now = map.now;
    let Some(mut b) = map.arena.battle.take() else {
        return;
    };
    let seconds = b.elapsed(now).as_secs();
    if let Some(champion) = won {
        if let Some(g) = champion
            && let Some(f) = b.fighter_mut(g)
        {
            f.place = 1;
        }
        crown(map, &b, champion, seconds);
    }
    // The fallen rise in the stands, and the ring's burn comes off.
    map.arena.raising = true;
    let mut seat_n = 0;
    for f in &b.fighters {
        crate::game::map::auras::remove_spell(map, f.guid, SPELL_RING_BURN, None);
        free_for_all(map, f.guid);
        let Some(p) = map.players.get(&f.guid) else {
            continue;
        };
        if p.death == Death::Alive {
            continue;
        }
        // A body not yet released rises where it lies; a ghost in the
        // stands, where it watches.
        crate::game::map::death::resurrect(map, f.guid, 1.0);
        let at = seat(map, seat_n);
        seat_n += 1;
        map.teleport(f.guid, map.key, at, facing_centre(at));
    }
    map.arena.raising = false;
}

/// The champion's crowning, the standings, and the record.
fn crown(map: &mut Map, b: &Battle, champion: Option<Guid>, seconds: u64) {
    let config = map.cfg.arena.clone();
    let mut standings: Vec<&Fighter> = b.fighters.iter().collect();
    standings.sort_by(|x, y| {
        x.place
            .max(1)
            .cmp(&y.place.max(1))
            .then(y.kills.cmp(&x.kills))
            .then(y.damage.cmp(&x.damage))
    });
    let champ = champion.and_then(|g| b.fighter(g)).cloned();
    match &champ {
        Some(c) => {
            yell(
                map,
                &format!(
                    "{} the {} {} is the Champion of Gurubashi Arena! {} kills, {}:{:02} of blood and glory!",
                    c.name,
                    race_name(c.race),
                    class_name(c.class),
                    c.kills,
                    seconds / 60,
                    seconds % 60
                ),
            );
            shout(
                map,
                &format!("All hail {}, Champion of Gurubashi Arena!", c.name),
            );
            reward(map, c.guid, &config);
        }
        None => {
            yell(
                map,
                "No one survives! The arena claims them all, and the crowd roars for blood!",
            );
        }
    }
    let lines: Vec<String> = std::iter::once("The standings of Gurubashi Arena:".to_owned())
        .chain(standings.iter().take(10).map(|f| standing_line(f, seconds)))
        .collect();
    for f in &b.fighters {
        let mut mine = lines.clone();
        if !standings.iter().take(10).any(|s| s.guid == f.guid) {
            mine.push(standing_line(f, seconds));
        }
        tell(map, f.guid, &mine.join("\n"));
    }
    let result = Outcome {
        id: b.id,
        ended: now_unix(),
        champion: champ.as_ref().map(|c| c.name.clone()),
        champion_class: champ.as_ref().map_or(0, |c| c.class),
        champion_race: champ.as_ref().map_or(0, |c| c.race),
        fighters: b.fighters.len() as u32,
        seconds,
        standings: standings
            .iter()
            .map(|f| (f.name.clone(), f.kills, f.damage, f.place.max(1)))
            .collect(),
    };
    let _ = b.started_unix;
    map.realm.store.put_record(KIND_ARENA, result.id, &result);
    map.realm.social.arena.lock().results.push(result);
}

fn standing_line(f: &Fighter, seconds: u64) -> String {
    let survived = f.fell.as_ref().map_or(seconds, |(t, _)| t.as_secs());
    let how = f
        .fell
        .as_ref()
        .map_or_else(|| "champion".to_owned(), |(_, c)| c.clone());
    format!(
        "{}. {} ({} {}): {} kills, {} damage, {}:{:02}, {how}",
        f.place.max(1),
        f.name,
        race_name(f.race),
        class_name(f.class),
        f.kills,
        f.damage,
        survived / 60,
        survived % 60
    )
}

/// The champion's prize: the buff, the gold and the trinket.
fn reward(map: &mut Map, g: Guid, config: &ArenaConfig) {
    if config.champion_spell != 0 {
        crate::game::map::spells::cast_triggered(map, g, config.champion_spell, Some(g));
    }
    if let Some(p) = map.players.get_mut(&g) {
        let money = p.money().saturating_add(config.reward_copper);
        p.set_money(money);
    }
    if config.reward_item != 0
        && crate::game::map::inventory::add_item(map, g, config.reward_item, 1, true).is_err()
    {
        tell(
            map,
            g,
            "Your bags are full: the arena keeps your prize. Make room, champion.",
        );
    }
}

/// Whether `g` is a fighter still standing in a fight under way.
pub fn fighting(map: &Map, g: Guid) -> bool {
    map.arena
        .battle
        .as_ref()
        .is_some_and(|b| b.fighting() && b.fighter(g).is_some_and(|f| f.alive))
}

/// A fighter's free-for-all flag: on while they fight, else as their area
/// (and its grace) has it ([`crate::game::map::pvp::free_for_all`]).
fn free_for_all(map: &mut Map, g: Guid) {
    crate::game::map::pvp::free_for_all(map, g);
}

/// Whether `a` may strike `b` because of the battle: no blows before the
/// fight, and none between a fighter and someone outside it.
pub fn can_attack(map: &Map, a: Guid, b: Guid) -> Option<bool> {
    let battle = map.arena.battle.as_ref()?;
    // A pet, totem or guardian fights for its master, in the fight or out
    // of it: one from the stands strikes no fighter, as its master can't.
    let (ma, mb) = (
        crate::game::map::pets::master(map, a),
        crate::game::map::pets::master(map, b),
    );
    let fa = battle.fighter(ma).is_some_and(|f| f.alive);
    let fb = battle.fighter(mb).is_some_and(|f| f.alive);
    if !fa && !fb {
        return None;
    }
    if !battle.fighting() {
        return Some(false);
    }
    if fa && fb {
        return Some(true);
    }
    if guid::is_unit(ma) || guid::is_unit(mb) {
        // The pit's own creatures (none) and others keep their rules.
        return None;
    }
    Some(false)
}

/// Whether the fallen `g` must stay fallen: a fighter out of a battle
/// still on.
pub fn blocks_resurrection(map: &Map, g: Guid) -> bool {
    !map.arena.raising
        && map
            .arena
            .battle
            .as_ref()
            .is_some_and(|b| b.fighter(g).is_some_and(|f| !f.alive))
}

/// A fallen fighter's ghost goes up to the stands, not to a graveyard:
/// whether it went.
pub fn release_to_stands(map: &mut Map, g: Guid) -> bool {
    if !blocks_resurrection(map, g) {
        return false;
    }
    let n = map
        .arena
        .battle
        .as_ref()
        .map_or(0, |b| b.fighters.len() - b.alive as usize);
    let at = seat(map, n);
    tell(map, g, "You have fallen. Watch the rest from the stands.");
    map.teleport(g, map.key, at, facing_centre(at));
    true
}
