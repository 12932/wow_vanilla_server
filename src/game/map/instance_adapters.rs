//! The dungeon and raid scripts past the first five, as Thistle Tea ports
//! vmangos's `instance_*` scripts: Blackfathom Deeps, Gnomeregan,
//! Razorfen Downs, Zul'Farrak, Uldaman, the Sunken Temple, Scholomance,
//! Stratholme, Blackrock Depths, Blackrock Spire, Dire Maul, the Wailing
//! Caverns and the Ruins of Ahn'Qiraj. Each keeps its encounter data in
//! the copy's saved fields, and what it only needs for the copy's life
//! (who has fallen, which wave is next) in the state's scratch, lists and
//! seen sets. Their creatures' own scripts (Thistle Tea's, see
//! [`crate::game::data::tt_scripts`]) report to them through instance
//! data, as vmangos's do.

// Places as Thistle Tea has them, to the digit.
#![allow(clippy::excessive_precision)]

use std::collections::BTreeMap;
use std::time::Duration;

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use super::instance_script::{
    DONE, Event, FAILED, IN_PROGRESS, NOT_STARTED, Open, Script, cancel, get_data, schedule, store,
};
use crate::game::data::scripts::ScriptStep;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::combat::dist;

const SPECIAL: u32 = 4;

/// The adapters' timer keys start here, clear of the first five's.
pub const TIMERS: u32 = 1000;

type Data = BTreeMap<u32, u32>;

fn v(data: &Data, field: u32) -> u32 {
    data.get(&field).copied().unwrap_or(0)
}

/// What a script does to a game object, as Thistle Tea's
/// `GameObjectActions.operate` has it: opening and destroying mark it in
/// use, the rest clear the mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Act {
    /// A closed one opens.
    Open,
    /// An open one shuts.
    Close,
    /// Broken: open for good.
    Destroy,
    /// Back to the state it spawned in.
    Reset,
    /// Not to be used.
    Inert,
    Active,
    /// Reset and locked.
    Lock,
    /// Open and unlocked.
    Unlock,
}

const GO_FLAG_IN_USE: u32 = 0x1;
const GO_FLAG_LOCKED: u32 = 0x2;
const GO_FLAG_NO_INTERACT: u32 = 0x10;

pub(super) fn operate(map: &mut Map, entry: u32, act: Act) {
    let objects: Vec<(Guid, Option<u32>)> = map
        .gameobjects
        .iter()
        .filter(|(_, o)| o.entry == entry)
        .map(|(_, o)| (o.guid, o.spawn_guid))
        .collect();
    for (g, spawn) in objects {
        let default = spawn
            .and_then(|s| {
                map.realm
                    .data
                    .gameobject_spawns
                    .get(&map.key.map)
                    .and_then(|v| v.iter().find(|x| x.guid == s))
                    .map(|x| u32::from(x.state))
            })
            .unwrap_or(1);
        let Some(o) = map.gameobject_mut(g) else {
            continue;
        };
        let f = &mut o.fields;
        let flags = f.get_u32(GAMEOBJECT_FLAGS);
        let state = f.get_u32(GAMEOBJECT_STATE);
        let in_use = |on: bool| {
            if on {
                flags | GO_FLAG_IN_USE
            } else {
                flags & !GO_FLAG_IN_USE
            }
        };
        match act {
            Act::Open => {
                if state == 1 {
                    f.set_u32(GAMEOBJECT_STATE, 0);
                }
                f.set_u32(GAMEOBJECT_FLAGS, in_use(true));
            }
            Act::Close => {
                if state == 0 {
                    f.set_u32(GAMEOBJECT_STATE, 1);
                }
                f.set_u32(GAMEOBJECT_FLAGS, in_use(false));
            }
            Act::Destroy => {
                f.set_u32(GAMEOBJECT_STATE, 2);
                f.set_u32(GAMEOBJECT_FLAGS, in_use(true));
            }
            Act::Reset => {
                f.set_u32(GAMEOBJECT_STATE, default);
                f.set_u32(GAMEOBJECT_FLAGS, in_use(false));
            }
            Act::Inert => f.set_u32(GAMEOBJECT_FLAGS, flags | GO_FLAG_NO_INTERACT),
            Act::Active => f.set_u32(GAMEOBJECT_FLAGS, flags & !GO_FLAG_NO_INTERACT),
            Act::Lock => {
                f.set_u32(GAMEOBJECT_STATE, default);
                f.set_u32(GAMEOBJECT_FLAGS, in_use(false) | GO_FLAG_LOCKED);
            }
            Act::Unlock => {
                f.set_u32(GAMEOBJECT_STATE, 0);
                f.set_u32(GAMEOBJECT_FLAGS, in_use(true) & !GO_FLAG_LOCKED);
            }
        }
    }
}

/// The living creatures of `entry` here, in spawn order.
fn creatures(map: &Map, entry: u32) -> Vec<Guid> {
    let mut out: Vec<(u32, Guid)> = map
        .creatures
        .iter()
        .filter(|(_, c)| c.entry == entry && c.alive())
        .map(|(_, c)| (c.spawn_guid.unwrap_or(u32::MAX), c.guid))
        .collect();
    out.sort();
    out.into_iter().map(|(_, g)| g).collect()
}

/// The first of `entry` here says broadcast text `text`.
fn talk(map: &mut Map, entry: u32, text: u32) {
    if let Some(g) = creatures(map, entry).into_iter().next() {
        crate::game::map::scripts::talk(map, g, text, Guid::zero(), None);
    }
}

fn talk_as(map: &mut Map, g: Guid, text: u32) {
    crate::game::map::scripts::talk(map, g, text, Guid::zero(), None);
}

/// A creature the copy summons, with no summoner of its own.
fn summon(map: &mut Map, entry: u32, at: [f32; 4], despawn: Option<(u64, i32)>) -> Option<Guid> {
    let g = crate::game::map::scripts::summon(
        map,
        Guid::zero(),
        entry,
        Vector3d {
            x: at[0],
            y: at[1],
            z: at[2],
        },
        at[3],
        despawn.map(|(ms, kind)| (Duration::from_millis(ms), kind)),
    )?;
    if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
        s.summoner = None;
        if despawn.is_none() {
            s.despawn_type = 7;
        }
    }
    Some(g)
}

fn walk_to(map: &mut Map, g: Guid, to: [f32; 3], run: bool) {
    crate::game::map::scripts::move_with(
        map,
        g,
        Vector3d {
            x: to[0],
            y: to[1],
            z: to[2],
        },
        crate::game::map::scripts::Move {
            run,
            pathfind: true,
            ..Default::default()
        },
    );
}

/// One step run at once, from `g` on itself.
fn step(map: &mut Map, g: Guid, command: u32, datalong: [u32; 4]) {
    let s = ScriptStep {
        command,
        datalong,
        ..Default::default()
    };
    crate::game::map::scripts::execute(map, &s, g, g);
}

/// Unit flags set (mode 1) or cleared (2).
fn unit_flags(map: &mut Map, g: Guid, flags: u32, mode: u32) {
    step(map, g, 4, [UNIT_FIELD_FLAGS as u32, flags, mode, 0]);
}

fn npc_flags(map: &mut Map, g: Guid, flags: u32, mode: u32) {
    step(map, g, 4, [UNIT_NPC_FLAGS as u32, flags, mode, 0]);
}

/// A faction (0 for its own).
fn faction(map: &mut Map, g: Guid, faction: u32) {
    step(map, g, 22, [faction, 1, 0, 0]);
}

fn stand(map: &mut Map, g: Guid, state: u32) {
    step(map, g, 28, [state, 0, 0, 0]);
}

fn cast_self(map: &mut Map, g: Guid, spell: u32) {
    crate::game::map::spells::cast_triggered(map, g, spell, Some(g));
}

fn remove_aura(map: &mut Map, g: Guid, spell: u32) {
    crate::game::map::auras::remove_spell(map, g, spell, None);
}

/// Everyone in the copy is its enemy.
fn zone_pulse(map: &mut Map, g: Guid) {
    step(map, g, 49, [1, 0, 0, 0]);
}

/// It attacks the nearest player within `radius`.
fn attack_nearest(map: &mut Map, g: Guid, radius: f32) {
    let Some(at) = map.position_of(g) else {
        return;
    };
    let nearest = map
        .players
        .values()
        .filter(|p| p.death == crate::game::map::death::Death::Alive)
        .map(|p| (dist(p.position(), at), p.guid))
        .filter(|(d, _)| *d <= radius)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, g)| g);
    if let Some(p) = nearest
        && crate::game::map::combat::can_attack(map, g, p)
    {
        crate::game::map::ai::attacked(map, g, p);
    }
}

/// A creature back as it spawned, even if alive; what it was waiting to do
/// is called off.
fn respawn(map: &mut Map, g: Guid) {
    super::instance_script::cancel_for(map, g);
    if map.creature(g).is_some_and(|c| c.alive()) {
        if map.creature(g).is_some_and(|c| c.spawn_guid.is_none()) {
            return;
        }
        crate::game::map::ai::quiet_death(map, g);
    }
    crate::game::map::ai::respawn_now(map, g);
}

/// A creature goes: a summoned one for good, a spawned one until its
/// spawn's own time.
fn despawn(map: &mut Map, g: Guid) {
    super::instance_script::despawn(map, g, false);
}

/// An object the world database keeps back, up for `for_ms`; one already
/// up stays as it is, as Thistle Tea's `respawn_game_object` leaves a
/// spawn it didn't start. A chest raised before the copy last went comes
/// back as it was left: with what was in it, or, emptied, not until it
/// fills again.
pub(super) fn raise_object(map: &mut Map, spawn: u32, for_ms: u64) {
    if crate::game::map::scripts::object_of_spawn(map, spawn).is_some()
        || map.loot_state.respawns.iter().any(|(_, s)| *s == spawn)
    {
        return;
    }
    map.instance.raised.insert(spawn);
    let left = super::instance_script::chest_left(map, spawn);
    if let super::instance_script::Left::Gone(after) = left {
        let at = map.now + after;
        map.loot_state.respawns.push((at, spawn));
        return;
    }
    let data = map.realm.data.clone();
    let g = data
        .gameobject_spawns
        .get(&map.key.map)
        .and_then(|v| v.iter().find(|s| s.guid == spawn))
        .and_then(|s| {
            let t = data.gameobject_templates.get(&s.entry)?;
            Some(map.add_gameobject(crate::game::map::gameobject::GameObject::from_spawn(t, s)))
        });
    if let Some(g) = g {
        crate::game::map::scripts::set_object_state(map, g, 1);
        if let super::instance_script::Left::Loot(loot) = left
            && let Some(o) = map.gameobject_mut(g)
        {
            o.loot = Some(loot);
        }
        map.visibility_dirty.insert(g);
        map.object_resets
            .push((map.now + Duration::from_millis(for_ms), g, u32::MAX));
    }
}

fn suspend_object(map: &mut Map, spawn: u32) {
    if let Some(g) = crate::game::map::scripts::object_of_spawn(map, spawn) {
        map.remove_gameobject(g);
    }
}

fn load_creatures(map: &mut Map, spawns: &[u32]) {
    for s in spawns {
        step(map, Guid::zero(), 91, [*s, 0, 0, 0]);
    }
}

fn players(map: &Map) -> Vec<Guid> {
    map.players.keys().copied().collect()
}

fn scratch(map: &Map, key: u32) -> u32 {
    map.instance.scratch.get(&key).copied().unwrap_or(0)
}

fn set_scratch(map: &mut Map, key: u32, value: u32) {
    map.instance.scratch.insert(key, value);
}

fn list(map: &Map, key: u32) -> Vec<Guid> {
    map.instance.lists.get(&key).cloned().unwrap_or_default()
}

fn push(map: &mut Map, key: u32, g: Guid) {
    let l = map.instance.lists.entry(key).or_default();
    if !l.contains(&g) {
        l.push(g);
    }
}

fn remove(map: &mut Map, key: u32, g: Guid) -> bool {
    let l = map.instance.lists.entry(key).or_default();
    let before = l.len();
    l.retain(|x| *x != g);
    l.len() != before
}

fn seen(map: &Map, key: u32) -> Vec<u32> {
    map.instance.seen.get(&key).cloned().unwrap_or_default()
}

/// Whether `id` is new to seen set `key` (and it is in it now).
fn mark(map: &mut Map, key: u32, id: u32) -> bool {
    let l = map.instance.seen.entry(key).or_default();
    if l.contains(&id) {
        false
    } else {
        l.push(id);
        true
    }
}

/// A field to `value`, unless it is done.
fn settle(map: &mut Map, field: u32, value: u32) -> u32 {
    let stored = if get_data(map, field) == DONE {
        DONE
    } else {
        value
    };
    store(map, field, stored);
    stored
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn between(lo: u64, hi: u64) -> Duration {
    ms(rand::rng().random_range(lo..=hi))
}

/// The doors the adapters' data opens.
pub fn doors(s: Script) -> &'static [(u32, Open)] {
    match s {
        Script::BlackfathomDeeps => {
            &[(21117, Open::Rule(|d| v(d, 10) == DONE && v(d, 11) == DONE))]
        }
        Script::Gnomeregan => &[
            (146086, Open::Rule(|d| v(d, 3) != 0)),
            (146085, Open::Rule(|d| v(d, 4) != 0)),
        ],
        Script::ZulFarrak => &[(146084, Open::Done(3))],
        Script::Uldaman => &[
            (124372, Open::Done(0)),
            (124367, Open::Done(1)),
            (124369, Open::Done(2)),
            (141869, Open::Rule(|d| matches!(v(d, 11), FAILED | DONE))),
        ],
        Script::SunkenTemple => &[(149431, Open::Done(5))],
        Script::Scholomance => &[
            (175570, Open::Rule(|d| v(d, 7) != IN_PROGRESS)),
            (175167, Open::Done(14)),
            (177371, Open::Rule(gandling_over)),
            (177372, Open::Rule(gandling_over)),
            (177373, Open::Rule(gandling_over)),
            (177375, Open::Rule(gandling_over)),
            (177376, Open::Rule(gandling_over)),
            (177377, Open::Rule(gandling_over)),
        ],
        Script::Stratholme => &[
            (175380, Open::Done(1)),
            (175379, Open::Done(2)),
            (175381, Open::Done(3)),
            (181071, Open::Rule(|d| matches!(v(d, 0), FAILED | DONE))),
        ],
        Script::BlackrockDepths => &[
            (170573, Open::Rule(golem_doors_open)),
            (170574, Open::Rule(golem_doors_open)),
            (170578, Open::Rule(runes_lit)),
            (170579, Open::Rule(runes_lit)),
            (170580, Open::Rule(runes_lit)),
            (170581, Open::Rule(runes_lit)),
            (170582, Open::Rule(runes_lit)),
            (170583, Open::Rule(runes_lit)),
            (170584, Open::Rule(runes_lit)),
            (BRD_BEAST_GATE, Open::Rule(|d| v(d, BRD_ARENA_STAGE) == 1)),
            (
                BRD_CHAMPION_GATE,
                Open::Rule(|d| v(d, BRD_ARENA_STAGE) == 2),
            ),
            (161524, Open::Done(BRD_RING)),
            (161523, Open::Rule(|d| v(d, BRD_RING) != IN_PROGRESS)),
            (170575, Open::Done(5)),
            (170577, Open::Done(BRD_TOMB)),
            (170576, Open::Rule(|d| v(d, BRD_TOMB) != IN_PROGRESS)),
        ],
        Script::BlackrockSpire => &[
            (175244, Open::Done(0)),
            (175153, Open::Done(1)),
            (175946, Open::Done(7)),
            (175947, Open::Done(7)),
        ],
        Script::DireMaul => &[
            (179503, Open::Done(1)),
            (179506, Open::Done(1)),
            (177220, Open::Rule(|d| matches!(v(d, 11), SPECIAL | DONE))),
            (179502, Open::Done(11)),
        ],
        _ => &[],
    }
}

fn gandling_over(d: &Data) -> bool {
    matches!(v(d, 0), FAILED | DONE)
}

fn golem_doors_open(d: &Data) -> bool {
    v(d, 4) == DONE && v(d, 5) != IN_PROGRESS
}

fn runes_lit(d: &Data) -> bool {
    v(d, 43) == IN_PROGRESS
}

/// A field set. True if the adapter took it.
pub fn set_data(map: &mut Map, s: Script, field: u32, value: u32) -> bool {
    match s {
        Script::BlackfathomDeeps => {
            settle(map, field, value);
        }
        Script::WailingCaverns => wc_set(map, field, value),
        Script::RazorfenDowns => rfd_set(map, field, value),
        Script::SunkenTemple => st_set(map, field, value),
        Script::Scholomance => scholo_set(map, field, value),
        Script::Gnomeregan => gnome_set(map, field, value),
        Script::ZulFarrak => zf_set(map, field, value),
        Script::Uldaman => uld_set(map, field, value),
        Script::BlackrockSpire => brs_set(map, field, value),
        Script::DireMaul => dm_set(map, field, value),
        Script::BlackrockDepths => brd_set(map, field, value),
        Script::Stratholme => strat_set(map, field, value),
        Script::RuinsOfAhnQiraj => {}
        _ => return false,
    }
    true
}

/// A creature's event in an adapter's copy.
pub fn creature_event(
    map: &mut Map,
    s: Script,
    g: Guid,
    entry: u32,
    spawn: Option<u32>,
    event: Event,
) {
    match s {
        Script::BlackfathomDeeps => bfd_creature(map, entry, spawn, event),
        Script::SunkenTemple => st_creature(map, g, entry, event),
        Script::Scholomance => scholo_creature(map, entry, event),
        Script::RuinsOfAhnQiraj => aq20_creature(map, g, entry, event),
        Script::Gnomeregan => gnome_creature(map, entry, event),
        Script::ZulFarrak => zf_creature(map, g, entry, spawn, event),
        Script::Uldaman => uld_creature(map, g, entry, spawn, event),
        Script::BlackrockSpire => brs_creature(map, spawn, event),
        Script::DireMaul => dm_creature(map, g, entry, spawn, event),
        Script::BlackrockDepths => brd_creature(map, g, entry, event),
        Script::Stratholme => strat_creature(map, g, entry, spawn, event),
        // Tuten'kash's death settles his encounter, as vmangos's
        // `OnCreatureDeath` has it, so a reload knows him dead.
        Script::RazorfenDowns if entry == RFD_TUTENKASH && event == Event::Death => {
            settle(map, 0, DONE);
        }
        _ => {}
    }
}

/// A game object used in an adapter's copy.
pub fn object_used(map: &mut Map, s: Script, entry: u32) {
    match s {
        Script::BlackfathomDeeps => bfd_object(map, entry),
        Script::RazorfenDowns if entry == RFD_GONG => {
            let next = get_data(map, 1) + 1;
            rfd_set(map, 1, next);
        }
        Script::Scholomance => scholo_object(map, entry),
        Script::Gnomeregan => gnome_object(map, entry),
        Script::ZulFarrak => zf_object(map, entry),
        Script::Uldaman => uld_object(map, entry),
        Script::BlackrockSpire => brs_object(map, entry),
        Script::BlackrockDepths => brd_object(map, entry),
        Script::Stratholme => strat_object(map, entry),
        _ => {}
    }
}

/// Objects as the copy's data and state leave them, once its spawns are
/// in.
pub fn settle_spawns(map: &mut Map, s: Script) {
    match s {
        Script::RazorfenDowns
            if get_data(map, 0) == DONE || get_data(map, 1) >= RFD_TUTENKASH_WAVE =>
        {
            operate(map, RFD_GONG, Act::Inert);
        }
        Script::Gnomeregan => {
            match get_data(map, GNOME_THERMAPLUGG) {
                IN_PROGRESS => operate(map, GNOME_FINAL_CHAMBER, Act::Lock),
                FAILED | DONE => operate(map, GNOME_FINAL_CHAMBER, Act::Unlock),
                _ => {}
            }
            for (i, face) in GNOME_FACES.iter().enumerate() {
                if get_data(map, GNOME_FACE_FIELDS + i as u32) != 0 {
                    operate(map, *face, Act::Open);
                }
            }
        }
        Script::Stratholme => strat_settle(map),
        Script::Scholomance => scholo_gandling(map),
        Script::BlackrockSpire => brs_settle(map),
        Script::BlackrockDepths if get_data(map, BRD_TOMB) == DONE => {
            raise_object(map, 399065, 3_600_000);
        }
        Script::DireMaul => dm_settle(map),
        _ => {}
    }
}

/// The spawns of `spawns` that lie dead as the copy loads.
fn dead_spawns(map: &Map, spawns: &[u32]) -> Vec<u32> {
    map.creatures
        .iter()
        .filter(|(_, c)| !c.alive())
        .filter_map(|(_, c)| c.spawn_guid)
        .filter(|s| spawns.contains(s))
        .collect()
}

/// A timed step of an adapter for one creature.
pub fn creature_timer(map: &mut Map, s: Script, key: u32, g: Guid) {
    match s {
        Script::Uldaman => uld_creature_timer(map, key, g),
        Script::Gnomeregan if key == T_GNOME_WALK => gnome_bomb_walks(map, g),
        Script::Stratholme if key == T_STRAT_MALOWN => strat_malown_due(map, g),
        Script::DireMaul => dm_creature_timer(map, key, g),
        Script::BlackrockDepths if key == T_BRD_PULSE => zone_pulse(map, g),
        _ => {}
    }
}

/// A timed step of an adapter.
pub fn timer(map: &mut Map, s: Script, key: u32) {
    match s {
        Script::BlackfathomDeeps => bfd_timer(map, key),
        Script::Gnomeregan => gnome_timer(map, key),
        Script::ZulFarrak => zf_timer(map, key),
        Script::Uldaman => uld_timer(map, key),
        Script::BlackrockSpire => brs_timer(map, key),
        Script::BlackrockDepths => brd_timer(map, key),
        Script::Stratholme => strat_timer(map, key),
        _ => {}
    }
}

// Blackfathom Deeps: Kelris's death opens the shrine; each Fire of
// Aku'mai lit after calls the next of four waves; with all four burning
// and their servants dead, the portal to Aku'mai opens.

const BFD_FIRES: [u32; 4] = [21118, 21119, 21120, 21121];
const BFD_SERVANTS: [u32; 4] = [4825, 4978, 4815, 4977];
const BFD_POINTS: [[f32; 4]; 6] = [
    [-768.949, -174.413, -25.87, 3.09],
    [-768.888, -164.238, -25.87, 3.09],
    [-768.951, -153.911, -25.88, 3.09],
    [-867.782, -174.352, -25.87, 6.27],
    [-867.875, -164.089, -25.87, 6.27],
    [-867.859, -153.927, -25.88, 6.27],
];
/// A wave: (entry, [(how many, at point)]) by group.
type Wave = &'static [(u32, &'static [(u32, usize)])];
const BFD_WAVES: [Wave; 4] = [
    &[(4825, &[(1, 0), (1, 1), (1, 5)]), (4825, &[(1, 4)])],
    &[(4978, &[(1, 1), (1, 4)])],
    &[(4815, &[(1, 0), (1, 2)]), (4815, &[(1, 3), (1, 4)])],
    &[
        (4977, &[(2, 0), (1, 1), (1, 2)]),
        (4977, &[(1, 3), (1, 4), (2, 5)]),
    ],
];
const BFD_LIT: u32 = 1;
const BFD_REMAINING: u32 = 2;
const BFD_WAVES_CALLED: u32 = 3;
const T_BFD_FIRE_RESET: u32 = TIMERS + 100;
const T_BFD_WAVE: u32 = TIMERS + 110;

fn bfd_object(map: &mut Map, entry: u32) {
    let Some(i) = BFD_FIRES.iter().position(|f| *f == entry) else {
        return;
    };
    let lit = seen(map, BFD_LIT);
    if lit.contains(&entry) {
        return;
    }
    if get_data(map, 10) != DONE {
        schedule(map, T_BFD_FIRE_RESET + i as u32, ms(1000));
        return;
    }
    let wave = lit.len() as u32;
    mark(map, BFD_LIT, entry);
    settle(map, 11, IN_PROGRESS);
    schedule(map, T_BFD_WAVE + wave, ms(3000));
}

fn bfd_creature(map: &mut Map, entry: u32, spawn: Option<u32>, event: Event) {
    if event != Event::Death || spawn.is_some() || !BFD_SERVANTS.contains(&entry) {
        return;
    }
    let left = scratch(map, BFD_REMAINING);
    if left == 0 {
        return;
    }
    set_scratch(map, BFD_REMAINING, left - 1);
    if left == 1 && scratch(map, BFD_WAVES_CALLED) == BFD_WAVES.len() as u32 {
        settle(map, 11, DONE);
    }
}

fn bfd_timer(map: &mut Map, key: u32) {
    if (T_BFD_FIRE_RESET..T_BFD_FIRE_RESET + 4).contains(&key) {
        let fire = BFD_FIRES[(key - T_BFD_FIRE_RESET) as usize];
        if !seen(map, BFD_LIT).contains(&fire) {
            operate(map, fire, Act::Reset);
        }
        return;
    }
    let Some(wave) = key
        .checked_sub(T_BFD_WAVE)
        .and_then(|w| BFD_WAVES.get(w as usize))
    else {
        return;
    };
    let mut called = 0;
    for (entry, groups) in wave.iter() {
        for (count, point) in groups.iter() {
            for i in 0..*count {
                let mut at = BFD_POINTS[*point];
                if *count > 1 {
                    at[1] = at[1] - 2.5 + i as f32 * 5.0 / *count as f32;
                }
                if let Some(g) = summon(map, *entry, at, None) {
                    cast_self(map, g, 7741);
                    zone_pulse(map, g);
                    called += 1;
                }
            }
        }
    }
    let left = scratch(map, BFD_REMAINING) + called;
    set_scratch(map, BFD_REMAINING, left);
    let waves = scratch(map, BFD_WAVES_CALLED) + 1;
    set_scratch(map, BFD_WAVES_CALLED, waves);
}

// Wailing Caverns: the Fanglords, and Naralex's nightmare.

const WC_WARNED: u32 = 100;
const WC_NIGHTMARE: [u32; 22] = [
    3636, 3637, 3640, 3654, 3669, 3670, 3671, 3673, 3674, 3840, 5048, 5053, 5055, 5056, 5755, 5756,
    5761, 5762, 5763, 5775, 5912, 8886,
];

fn wc_set(map: &mut Map, field: u32, value: u32) {
    let stored = settle(map, field, value);
    match (field, stored) {
        // Lady Anacondra rests: the druid at her side goes.
        (0, SPECIAL) => {
            if let Some(a) = creatures(map, 3671).into_iter().next()
                && let Some(at) = map.position_of(a)
            {
                for d in creatures(map, 3840) {
                    if map.position_of(d).is_some_and(|p| dist(p, at) <= 5.0) {
                        despawn(map, d);
                        break;
                    }
                }
            }
        }
        (0..=2, DONE) if get_data(map, WC_WARNED) == 0 => {
            store(map, WC_WARNED, 1);
            talk(map, 3673, 2102);
        }
        (5, DONE) => {
            for e in WC_NIGHTMARE {
                for c in creatures(map, e) {
                    despawn(map, c);
                }
            }
        }
        _ => {}
    }
    if (0..4).all(|f| get_data(map, f) == DONE) && get_data(map, 4) == NOT_STARTED {
        store(map, 4, SPECIAL);
        talk(map, 3678, 2101);
    }
}

// Razorfen Downs: the gong's waves, and the idol's fires.

const RFD_GONG: u32 = 148917;
const RFD_TUTENKASH_WAVE: u32 = 15;
const RFD_TUTENKASH: u32 = 7355;

fn rfd_set(map: &mut Map, field: u32, value: u32) {
    match field {
        1 => {
            store(map, 1, value);
            match value {
                9 | 14 => operate(map, RFD_GONG, Act::Active),
                1 => rfd_wave(map, 7349, 8),
                10 => rfd_wave(map, 7351, 4),
                RFD_TUTENKASH_WAVE => rfd_wave(map, RFD_TUTENKASH, 1),
                _ => {}
            }
        }
        2 => {
            store(map, 2, value);
            for f in [32027, 32029, 32030, 32031] {
                suspend_object(map, f);
            }
        }
        _ => {
            settle(map, field, value);
        }
    }
}

fn rfd_wave(map: &mut Map, entry: u32, count: usize) {
    operate(map, RFD_GONG, Act::Inert);
    const WEST: [f32; 4] = [2502.635, 844.140, 46.896, 0.633];
    const NORTH: [f32; 4] = [2546.33, 887.455, 47.69, 0.633];
    const OFFSETS: [(f32, f32); 5] = [
        (0.0, 0.0),
        (3.0, -2.0),
        (-2.0, 3.0),
        (-4.0, -3.0),
        (4.0, 3.0),
    ];
    for i in 0..count {
        let tunnel = if i % 2 == 0 { WEST } else { NORTH };
        let (dx, dy) = OFFSETS[(i / 2) % OFFSETS.len()];
        let at = [tunnel[0] + dx, tunnel[1] + dy, tunnel[2], tunnel[3]];
        if let Some(g) = summon(map, entry, at, None) {
            let gong = Vector3d {
                x: 2533.479,
                y: 870.020,
                z: 47.678,
            };
            let to = map
                .nav
                .as_mut()
                .and_then(|n| n.random_point(gong, 5.0))
                .unwrap_or(gong);
            walk_to(map, g, [to.x, to.y, to.z], true);
        }
    }
}

// The Ruins of Ahn'Qiraj: Andorov comes when Kurinnaxx falls.

const AQ20_SQUAD_LOADED: u32 = 1;

fn aq20_creature(map: &mut Map, g: Guid, entry: u32, event: Event) {
    match (entry, event) {
        (15348, Event::Death) if scratch(map, AQ20_SQUAD_LOADED) == 0 => {
            set_scratch(map, AQ20_SQUAD_LOADED, 1);
            load_creatures(map, &[301311, 301312, 301313, 301314, 301315]);
        }
        (15471, Event::Spawned) => step(map, g, 60, [0; 4]),
        _ => {}
    }
}

// The Sunken Temple: Jammal'an's barrier, and the Shade of Eranikus.

const ST_PROTECTORS: [u32; 6] = [5712, 5713, 5714, 5715, 5716, 5717];
/// The protectors slain, by bit, saved.
const ST_SLAIN: u32 = 100;
const ST_ERANIKUS: u32 = 5709;
const IMMUNE_TO_PLAYER: u32 = 0x100;

fn st_eranikus(map: &mut Map, mode: u32, stand_state: u32) {
    for g in creatures(map, ST_ERANIKUS) {
        unit_flags(map, g, IMMUNE_TO_PLAYER, mode);
        stand(map, g, stand_state);
    }
}

fn st_set(map: &mut Map, field: u32, value: u32) {
    match field {
        5 => {}
        6 => {
            let was_done = get_data(map, 6) == DONE;
            let stored = settle(map, 6, value);
            if !was_done && stored == DONE {
                st_eranikus(map, 2, 3);
            }
        }
        _ => {
            settle(map, field, value);
        }
    }
}

fn st_creature(map: &mut Map, _g: Guid, entry: u32, event: Event) {
    match event {
        Event::Death => {
            let Some(i) = ST_PROTECTORS.iter().position(|e| *e == entry) else {
                return;
            };
            let slain = get_data(map, ST_SLAIN) | (1 << i);
            store(map, ST_SLAIN, slain);
            if get_data(map, 5) != DONE && slain.count_ones() as usize == ST_PROTECTORS.len() {
                store(map, 5, DONE);
                talk(map, 5710, 4490);
            }
        }
        Event::Spawned if entry == ST_ERANIKUS && get_data(map, 6) != DONE => {
            st_eranikus(map, 1, 3);
        }
        Event::Aggro if entry == ST_ERANIKUS => st_eranikus(map, 2, 0),
        _ => {}
    }
}

// Scholomance: the room bosses bring Gandling; the Brazier of the Herald
// calls Kirtonos.

const SCHOLO_ROOM_BOSSES: [(u32, u32); 6] = [
    (11261, 1),
    (10505, 2),
    (10502, 3),
    (10504, 4),
    (10901, 5),
    (10507, 6),
];
const SCHOLO_BRAZIER: u32 = 175564;

fn scholo_set(map: &mut Map, field: u32, value: u32) {
    let stored = settle(map, field, value);
    if field == 7 && value == FAILED {
        if stored == FAILED {
            operate(map, SCHOLO_BRAZIER, Act::Reset);
        }
        return;
    }
    scholo_gandling(map);
}

/// The room bosses all dead, Gandling comes.
fn scholo_gandling(map: &mut Map) {
    if get_data(map, 0) == NOT_STARTED
        && SCHOLO_ROOM_BOSSES
            .iter()
            .all(|(_, f)| get_data(map, *f) == DONE)
    {
        settle(map, 0, IN_PROGRESS);
        summon(map, 1853, [180.771, -5.4286, 75.5702, 1.29154], None);
    }
}

fn scholo_object(map: &mut Map, entry: u32) {
    match entry {
        SCHOLO_BRAZIER if !matches!(get_data(map, 7), IN_PROGRESS | DONE) => {
            settle(map, 7, IN_PROGRESS);
            summon(map, 10506, [315.028, 70.53845, 102.1496, 0.3859715], None);
        }
        175167 => {
            settle(map, 14, DONE);
        }
        _ => {}
    }
}

fn scholo_creature(map: &mut Map, entry: u32, event: Event) {
    if event != Event::Death {
        return;
    }
    if entry == 10506 {
        settle(map, 7, DONE);
    } else if let Some((_, f)) = SCHOLO_ROOM_BOSSES.iter().find(|(e, _)| *e == entry) {
        scholo_set(map, *f, DONE);
    }
}

// Gnomeregan: Grubbis's cave-ins, and Thermaplugg's bomb faces.

const GNOME_GRUBBIS: u32 = 0;
const GNOME_THERMAPLUGG: u32 = 1;
const GNOME_CHARGE: u32 = 2;
const GNOME_FACE_FIELDS: u32 = 10;
const GNOME_FINAL_CHAMBER: u32 = 142207;
const GNOME_FACES: [u32; 6] = [142211, 142210, 142209, 142208, 142213, 142212];
const GNOME_BUTTONS: [u32; 6] = [142214, 142215, 142216, 142217, 142218, 142219];
const GNOME_CHARGES: [(u32, u32); 4] = [(1, 3997159), (2, 3997160), (3, 3997157), (4, 3997158)];
const GNOME_BOMB: u32 = 7915;
const GNOME_BOMBS: u32 = 1;
const T_GNOME_BOMB: u32 = TIMERS + 200;
const GNOME_SPAWN: [f32; 3] = [-531.324, 670.159, -325.185];
const GNOME_FACE_SPOTS: [(f32, f32); 6] = [
    (-561.236, 709.881),
    (-518.904, 718.472),
    (-485.723, 690.871),
    (-486.665, 647.652),
    (-521.022, 621.549),
    (-562.961, 632.162),
];

fn gnome_detonate(map: &mut Map) {
    for (_, spawn) in GNOME_CHARGES {
        suspend_object(map, spawn);
    }
}

fn gnome_face(map: &mut Map, index: usize, on: bool) {
    let field = GNOME_FACE_FIELDS + index as u32;
    let was = get_data(map, field) != 0;
    store(map, field, u32::from(on));
    if was == on {
        return;
    }
    if on {
        operate(map, GNOME_FACES[index], Act::Open);
        schedule(map, T_GNOME_BOMB + index as u32, ms(3000));
    } else {
        operate(map, GNOME_FACES[index], Act::Reset);
        cancel(map, T_GNOME_BOMB + index as u32);
    }
}

fn gnome_set(map: &mut Map, field: u32, value: u32) {
    match field {
        GNOME_GRUBBIS => {
            let before = get_data(map, field);
            let stored = settle(map, field, value);
            if stored == before {
                return;
            }
            match stored {
                FAILED => {
                    store(map, 3, 0);
                    store(map, 4, 0);
                    gnome_detonate(map);
                }
                DONE => {
                    for rocket in [283, 284, 285] {
                        raise_object(map, rocket, 3_600_000);
                    }
                }
                _ => {}
            }
        }
        GNOME_CHARGE => {
            store(map, field, value);
            if let Some((_, spawn)) = GNOME_CHARGES.iter().find(|(n, _)| *n == value) {
                raise_object(map, *spawn, 3_600_000);
            } else if value == 5 {
                gnome_detonate(map);
            }
        }
        3 | 4 => store(map, field, value),
        f if (GNOME_FACE_FIELDS..GNOME_FACE_FIELDS + 6).contains(&f) => {
            gnome_face(map, (f - GNOME_FACE_FIELDS) as usize, value != 0);
        }
        GNOME_THERMAPLUGG => {
            let before = get_data(map, field);
            let stored = settle(map, field, value);
            if stored == before {
                return;
            }
            match stored {
                IN_PROGRESS => {
                    operate(map, GNOME_FINAL_CHAMBER, Act::Lock);
                    gnome_face(map, 2, true);
                }
                FAILED | DONE => {
                    operate(map, GNOME_FINAL_CHAMBER, Act::Unlock);
                    for i in 0..6 {
                        gnome_face(map, i, false);
                    }
                }
                _ => {}
            }
        }
        _ => {
            settle(map, field, value);
        }
    }
}

fn gnome_object(map: &mut Map, entry: u32) {
    if let Some(i) = GNOME_BUTTONS.iter().position(|b| *b == entry) {
        gnome_face(map, i, false);
    }
}

fn gnome_creature(map: &mut Map, entry: u32, event: Event) {
    match (entry, event) {
        (7998, Event::Death) => gnome_set(map, GNOME_GRUBBIS, FAILED),
        (7361, Event::Death) => gnome_set(map, GNOME_GRUBBIS, DONE),
        (7800, Event::Aggro) => gnome_set(map, GNOME_THERMAPLUGG, IN_PROGRESS),
        (7800, Event::Death) => gnome_set(map, GNOME_THERMAPLUGG, DONE),
        (7800, Event::Evade) => {
            gnome_set(map, GNOME_THERMAPLUGG, FAILED);
            set_scratch(map, GNOME_BOMBS, 0);
            for b in creatures(map, GNOME_BOMB) {
                despawn(map, b);
            }
        }
        (GNOME_BOMB, Event::Death) => {
            let n = scratch(map, GNOME_BOMBS).saturating_sub(1);
            set_scratch(map, GNOME_BOMBS, n);
        }
        _ => {}
    }
}

/// A bomb that has landed walks to Thermaplugg.
fn gnome_bomb_walks(map: &mut Map, b: Guid) {
    let boss = creatures(map, 7800).into_iter().next();
    if let Some(at) = boss.and_then(|t| map.position_of(t)) {
        walk_to(map, b, [at.x, at.y, at.z], false);
    }
}

fn gnome_timer(map: &mut Map, key: u32) {
    let index = (key - T_GNOME_BOMB) as usize;
    if index >= 6
        || get_data(map, GNOME_FACE_FIELDS + index as u32) == 0
        || get_data(map, GNOME_THERMAPLUGG) != IN_PROGRESS
    {
        return;
    }
    schedule(map, key, between(10_000, 25_000));
    let bombs = scratch(map, GNOME_BOMBS);
    if bombs >= 6 {
        return;
    }
    set_scratch(map, GNOME_BOMBS, bombs + 1);
    let [sx, sy, sz] = GNOME_SPAWN;
    let (fx, fy) = GNOME_FACE_SPOTS[index];
    let (bx, by) = (0.35 * sx + 0.65 * fx, 0.35 * sy + 0.65 * fy);
    if let Some(b) = summon(map, GNOME_BOMB, [bx, by, -316.2625, 0.0], None) {
        if let Some(s) = crate::game::map::event_ai::scripted_mut(map, b) {
            s.despawn_type = 5;
        }
        crate::game::map::scripts::move_with(
            map,
            b,
            Vector3d {
                x: 0.2 * sx + 0.8 * bx,
                y: 0.2 * sy + 0.8 * by,
                z: sz - 2.0,
            },
            crate::game::map::scripts::Move {
                time: Some(ms(800)),
                ..Default::default()
            },
        );
        // Then it walks to Thermaplugg.
        super::instance_script::schedule_for(map, T_GNOME_WALK, b, ms(800));
    }
}

const T_GNOME_WALK: u32 = TIMERS + 210;

// Zul'Farrak: the pyramid, the end door, and Gahz'rilla.

const ZF_PYRAMID: u32 = 1;
const ZF_GAHZRILLA: u32 = 2;
const ZF_END_DOOR: u32 = 3;
const ZF_CAGES_OPEN: u32 = 1;
const ZF_ARRIVED: u32 = 2;
const ZF_WAVE_1: u32 = 3;
const ZF_PRE_WAVE_2: u32 = 4;
const ZF_WAVE_2: u32 = 5;
const ZF_PRE_WAVE_3: u32 = 6;
const ZF_WAVE_3: u32 = 7;
const ZF_ALL_DEAD: u32 = 8;
const ZF_CAGES: [u32; 5] = [141070, 141071, 141072, 141073, 141074];
const ZF_REPORTED: u32 = 1;
const ZF_GROUP_SIZE: u32 = 2;
const ZF_SENT: u32 = 3;
const ZF_ALIVE: u32 = 10;
const ZF_AT_BASE: u32 = 11;
const T_ZF_SEND: u32 = TIMERS + 300;
const T_ZF_WAVE_2: u32 = TIMERS + 301;
const T_ZF_WAVE_3: u32 = TIMERS + 302;
const ZF_STAIR_TOP: [(u32, [f32; 3]); 5] = [
    (7604, [1887.17, 1263.72, 41.484]),
    (7605, [1890.76, 1265.82, 41.43]),
    (7606, [1883.3, 1272.53, 41.87]),
    (7607, [1883.87, 1263.49, 41.55]),
    (7608, [1886.48, 1272.76, 41.76]),
];
const ZF_STAIR_FOOT: [(u32, [f32; 3]); 5] = [
    (7604, [1887.92, 1228.179, 9.98]),
    (7608, [1891.57, 1228.68, 9.69]),
    (7606, [1897.23, 1228.34, 9.43]),
    (7605, [1883.68, 1227.95, 9.543]),
    (7607, [1878.02, 1227.65, 9.485]),
];
const ZF_FLOOR: [(u32, [f32; 3]); 5] = [
    (7604, [1883.82, 1200.83, 8.87]),
    (7608, [1891.83, 1201.45, 8.87]),
    (7606, [1894.50, 1204.40, 8.87]),
    (7605, [1874.11, 1206.17, 8.87]),
    (7607, [1877.52, 1199.63, 8.87]),
];
const ZF_WAVE_ONE: &[(u32, f32, f32)] = &[
    (7789, 1894.64, 1206.29),
    (7787, 1890.08, 1218.68),
    (8876, 1883.76, 1222.3),
    (7789, 1874.18, 1221.24),
    (7787, 1892.28, 1225.49),
    (7788, 1889.94, 1212.21),
    (7787, 1879.02, 1223.06),
    (7789, 1874.45, 1204.44),
    (8876, 1898.23, 1217.97),
    (7787, 1882.07, 1225.7),
    (8877, 1896.46, 1205.62),
    (7787, 1886.97, 1225.86),
    (7787, 1894.72, 1221.91),
    (7787, 1883.5, 1218.25),
    (7787, 1886.93, 1221.4),
    (8876, 1889.82, 1222.51),
    (7788, 1893.07, 1215.26),
    (7788, 1878.57, 1214.16),
    (7788, 1883.74, 1212.35),
    (8877, 1877.0, 1207.27),
    (8877, 1873.63, 1204.65),
    (8876, 1877.4, 1216.41),
    (8877, 1899.63, 1202.52),
];
const ZF_WAVE_TWO: &[(u32, f32, f32)] = &[
    (7789, 1902.83, 1223.41),
    (8876, 1889.82, 1222.51),
    (7787, 1883.5, 1218.25),
    (7788, 1883.74, 1212.35),
    (8877, 1877.0, 1207.27),
    (7787, 1890.08, 1218.68),
    (7789, 1894.64, 1206.29),
    (8876, 1877.4, 1216.41),
    (7787, 1892.28, 1225.49),
    (7788, 1893.07, 1215.26),
    (8877, 1896.46, 1205.62),
    (7789, 1874.45, 1204.44),
    (7789, 1874.18, 1221.24),
    (7787, 1879.02, 1223.06),
    (8876, 1898.23, 1217.97),
    (7787, 1882.07, 1225.7),
    (8877, 1873.63, 1204.65),
    (7787, 1886.97, 1225.86),
    (7788, 1878.57, 1214.16),
    (7787, 1894.72, 1221.91),
    (7787, 1886.93, 1221.4),
    (8876, 1883.76, 1222.3),
    (7788, 1889.94, 1212.21),
    (8877, 1899.63, 1202.52),
];
const ZF_WAVE_THREE: &[(u32, f32, f32)] = &[
    (7788, 1878.57, 1214.16),
    (7787, 1894.72, 1221.91),
    (7787, 1886.93, 1221.4),
    (8876, 1883.76, 1222.3),
    (7788, 1889.94, 1212.21),
    (7275, 1889.23, 1207.72),
    (7796, 1879.77, 1207.96),
];

fn zf_wave(n: u32) -> &'static [(u32, f32, f32)] {
    match n {
        1 => ZF_WAVE_ONE,
        2 => ZF_WAVE_TWO,
        _ => ZF_WAVE_THREE,
    }
}

fn zf_wave_number(map: &Map) -> Option<u32> {
    match get_data(map, ZF_PYRAMID) {
        ZF_WAVE_1 => Some(1),
        ZF_WAVE_2 => Some(2),
        ZF_PRE_WAVE_3 | ZF_WAVE_3 => Some(3),
        _ => None,
    }
}

fn zf_wave_entries() -> impl Iterator<Item = u32> {
    ZF_WAVE_ONE
        .iter()
        .chain(ZF_WAVE_TWO)
        .chain(ZF_WAVE_THREE)
        .map(|(e, _, _)| *e)
}

fn zf_summon_wave(map: &mut Map, n: u32) {
    for (entry, x, y) in zf_wave(n) {
        if let Some(g) = summon(map, *entry, [*x, *y, 8.87, 0.0], None)
            && let Some(s) = crate::game::map::event_ai::scripted_mut(map, g)
        {
            s.despawn_type = 8;
        }
    }
}

fn zf_reset_waves(map: &mut Map) {
    for k in [ZF_REPORTED, ZF_SENT] {
        set_scratch(map, k, 0);
    }
    set_scratch(map, ZF_GROUP_SIZE, 2);
    map.instance.lists.remove(&ZF_ALIVE);
    map.instance.lists.remove(&ZF_AT_BASE);
}

fn zf_set(map: &mut Map, field: u32, value: u32) {
    match (field, value) {
        (ZF_PYRAMID, ZF_ARRIVED) => {
            if matches!(get_data(map, ZF_PYRAMID), NOT_STARTED | ZF_CAGES_OPEN) {
                store(map, ZF_PYRAMID, ZF_WAVE_1);
                zf_reset_waves(map);
                zf_summon_wave(map, 1);
                schedule(map, T_ZF_SEND, ms(1000));
            }
        }
        (ZF_END_DOOR, _) => {
            let was = get_data(map, ZF_END_DOOR) == DONE;
            let stored = settle(map, ZF_END_DOOR, value);
            if stored == DONE && !was {
                talk(map, 7267, 6067);
            }
        }
        _ => store(map, field, value),
    }
}

fn zf_crew_to(
    map: &mut Map,
    places: &[(u32, [f32; 3])],
    facing: f32,
    faction: Option<u32>,
    point: Option<u32>,
) {
    for (entry, at) in places {
        for g in creatures(map, *entry) {
            if let Some(c) = map.creature_mut(g) {
                c.home = Vector3d {
                    x: at[0],
                    y: at[1],
                    z: at[2],
                };
                c.home_orientation = facing;
            }
            crate::game::map::scripts::move_with(
                map,
                g,
                Vector3d {
                    x: at[0],
                    y: at[1],
                    z: at[2],
                },
                crate::game::map::scripts::Move {
                    pathfind: true,
                    face: Some(facing),
                    point,
                    ..Default::default()
                },
            );
            if let Some(f) = faction {
                self::faction(map, g, f);
            }
        }
    }
}

fn zf_object(map: &mut Map, entry: u32) {
    if ZF_CAGES.contains(&entry) && get_data(map, ZF_PYRAMID) == NOT_STARTED {
        store(map, ZF_PYRAMID, ZF_CAGES_OPEN);
        zf_crew_to(map, &ZF_STAIR_TOP, 4.7, Some(250), Some(1));
    }
}

fn zf_creature(map: &mut Map, g: Guid, entry: u32, spawn: Option<u32>, event: Event) {
    if entry == 7273 && event == Event::Spawned {
        if get_data(map, ZF_GAHZRILLA) == NOT_STARTED {
            store(map, ZF_GAHZRILLA, IN_PROGRESS);
        } else {
            despawn(map, g);
        }
        return;
    }
    if !zf_wave_entries().any(|e| e == entry) {
        return;
    }
    match event {
        Event::Spawned if spawn.is_none() => {
            let Some(n) = zf_wave_number(map) else {
                return;
            };
            if (scratch(map, ZF_REPORTED) as usize) < zf_wave(n).len() {
                set_scratch(map, ZF_REPORTED, scratch(map, ZF_REPORTED) + 1);
                push(map, ZF_ALIVE, g);
                push(map, ZF_AT_BASE, g);
            }
        }
        Event::Death => {
            if !remove(map, ZF_ALIVE, g) {
                return;
            }
            remove(map, ZF_AT_BASE, g);
            let cleared = zf_wave_number(map).is_some_and(|n| {
                scratch(map, ZF_REPORTED) as usize == zf_wave(n).len()
                    && list(map, ZF_ALIVE).is_empty()
            });
            if cleared {
                zf_clear_wave(map);
            }
        }
        _ => {}
    }
}

fn zf_clear_wave(map: &mut Map) {
    zf_reset_waves(map);
    match get_data(map, ZF_PYRAMID) {
        ZF_WAVE_1 => {
            cancel(map, T_ZF_SEND);
            store(map, ZF_PYRAMID, ZF_PRE_WAVE_2);
            schedule(map, T_ZF_WAVE_2, ms(10_000));
        }
        ZF_WAVE_2 => {
            cancel(map, T_ZF_SEND);
            store(map, ZF_PYRAMID, ZF_PRE_WAVE_3);
            zf_summon_wave(map, 3);
            schedule(map, T_ZF_WAVE_3, ms(5_000));
        }
        ZF_PRE_WAVE_3 | ZF_WAVE_3 => {
            cancel(map, T_ZF_WAVE_3);
            store(map, ZF_PYRAMID, ZF_ALL_DEAD);
            zf_crew_to(map, &ZF_FLOOR, 1.32, None, None);
        }
        _ => {}
    }
}

fn zf_timer(map: &mut Map, key: u32) {
    match key {
        T_ZF_SEND if matches!(get_data(map, ZF_PYRAMID), ZF_WAVE_1 | ZF_WAVE_2) => {
            let size = scratch(map, ZF_GROUP_SIZE).max(2) as usize;
            let mut at_base = list(map, ZF_AT_BASE);
            let group: Vec<Guid> = at_base.drain(..size.min(at_base.len())).collect();
            map.instance.lists.insert(ZF_AT_BASE, at_base.clone());
            let sent = scratch(map, ZF_SENT);
            for (i, g) in group.iter().enumerate() {
                let index = sent + i as u32;
                walk_to(
                    map,
                    *g,
                    [1880.0 + ((index * 7) % 11) as f32, 1274.0, 42.0],
                    true,
                );
            }
            set_scratch(map, ZF_GROUP_SIZE, size as u32 + 1);
            set_scratch(map, ZF_SENT, sent + group.len() as u32);
            let more = !at_base.is_empty()
                || zf_wave_number(map)
                    .is_some_and(|n| (scratch(map, ZF_REPORTED) as usize) < zf_wave(n).len());
            if more {
                schedule(map, T_ZF_SEND, ms(10_000));
            }
        }
        T_ZF_WAVE_2 if get_data(map, ZF_PYRAMID) == ZF_PRE_WAVE_2 => {
            store(map, ZF_PYRAMID, ZF_WAVE_2);
            zf_reset_waves(map);
            zf_summon_wave(map, 2);
            schedule(map, T_ZF_SEND, ms(1000));
        }
        T_ZF_WAVE_3 if get_data(map, ZF_PYRAMID) == ZF_PRE_WAVE_3 => {
            store(map, ZF_PYRAMID, ZF_WAVE_3);
            zf_crew_to(map, &ZF_STAIR_FOOT, 4.78, None, None);
        }
        _ => {}
    }
}

// Uldaman: Ironaya behind the seal, the Stone Keepers, and Archaedas.

const ULD_IRONAYA_DOOR: u32 = 0;
const ULD_KEEPERS: u32 = 1;
const ULD_ARCHAEDAS: u32 = 2;
const ULD_ANCIENT_DOOR: u32 = 11;
const ULD_GUARDIANS: u32 = 13;
const ULD_WARDERS: u32 = 14;
const ULD_ARCHAEDAS_ENTRY: u32 = 2748;
const ULD_KEEPER: u32 = 4857;
const ULD_GUARDIAN: u32 = 7076;
const ULD_IRONAYA: u32 = 7228;
const ULD_WARDER: u32 = 10120;
const ULD_TRACKED: [u32; 7] = [
    ULD_ARCHAEDAS_ENTRY,
    ULD_KEEPER,
    ULD_GUARDIAN,
    7077,
    ULD_IRONAYA,
    7309,
    ULD_WARDER,
];
const ULD_CHAMBER_WARDERS: [u32; 2] = [33544, 33549];
const ULD_STONED: u32 = 10255;
const ULD_FROZEN: u32 = 0x0200_0300;
const T_ULD_SEAL: u32 = TIMERS + 400;
const T_ULD_KEEPER: u32 = TIMERS + 401;
const T_ULD_WALL: u32 = TIMERS + 402;
const T_ULD_GUARDIANS: u32 = TIMERS + 403;
const T_ULD_WARDERS: u32 = TIMERS + 404;
const T_ULD_RESET: u32 = TIMERS + 405;
const T_ULD_CLEAR: u32 = TIMERS + 406;
const T_ULD_ATTACK: u32 = TIMERS + 407;
const T_ULD_AWAKEN_CAST: u32 = TIMERS + 408;
const T_ULD_AWAKEN_STONE: u32 = TIMERS + 409;

/// Roles: keeper, ironaya, archaedas, guardian, warder, furniture (the
/// warders outside), wall (the earthen on the walls).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Keeper,
    Ironaya,
    Archaedas,
    Guardian,
    Warder,
    Furniture,
    Wall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Frozen,
    Awake,
    Dead,
}

fn uld_role(entry: u32, spawn: Option<u32>) -> Role {
    match entry {
        ULD_KEEPER => Role::Keeper,
        ULD_IRONAYA => Role::Ironaya,
        ULD_ARCHAEDAS_ENTRY => Role::Archaedas,
        ULD_GUARDIAN => Role::Guardian,
        ULD_WARDER if spawn.is_some_and(|s| ULD_CHAMBER_WARDERS.contains(&s)) => Role::Warder,
        ULD_WARDER => Role::Furniture,
        _ => Role::Wall,
    }
}

/// A tracked creature's list key: its role and its status.
fn uld_key(role: Role, status: Status) -> u32 {
    500 + role as u32 * 10 + status as u32
}

fn uld_all(map: &Map, role: Role, status: Status) -> Vec<Guid> {
    list(map, uld_key(role, status))
}

fn uld_put(map: &mut Map, g: Guid, role: Role, status: Status) {
    for s in [Status::Frozen, Status::Awake, Status::Dead] {
        remove(map, uld_key(role, s), g);
    }
    push(map, uld_key(role, status), g);
}

/// Earlier spawns wake first.
fn spawn_order(map: &Map, g: Guid) -> u32 {
    map.creature(g)
        .and_then(|c| c.spawn_guid)
        .unwrap_or(u32::MAX)
}

fn uld_role_of(map: &Map, g: Guid) -> Option<Role> {
    [
        Role::Keeper,
        Role::Ironaya,
        Role::Archaedas,
        Role::Guardian,
        Role::Warder,
        Role::Furniture,
        Role::Wall,
    ]
    .into_iter()
    .find(|r| {
        [Status::Frozen, Status::Awake, Status::Dead]
            .iter()
            .any(|s| uld_all(map, *r, *s).contains(&g))
    })
}

fn uld_first(map: &Map, role: Role, status: Status) -> Option<Guid> {
    let mut l = uld_all(map, role, status);
    l.sort_by_key(|g| spawn_order(map, *g));
    l.into_iter().next()
}

fn uld_thaw(map: &mut Map, g: Guid) {
    unit_flags(map, g, ULD_FROZEN, 2);
    remove_aura(map, g, ULD_STONED);
}

fn uld_freeze(map: &mut Map, g: Guid) {
    unit_flags(map, g, ULD_FROZEN, 1);
    cast_self(map, g, ULD_STONED);
}

/// It wakes, and in a moment fights the nearest intruder.
fn uld_wake(map: &mut Map, g: Guid, faction_id: Option<u32>, delay: u64) {
    uld_thaw(map, g);
    if let Some(f) = faction_id {
        faction(map, g, f);
    }
    super::instance_script::schedule_for(map, T_ULD_ATTACK, g, ms(delay));
}

fn uld_set(map: &mut Map, field: u32, value: u32) {
    match field {
        ULD_KEEPERS if value == IN_PROGRESS => {
            if get_data(map, ULD_KEEPERS) != DONE {
                store(map, ULD_KEEPERS, IN_PROGRESS);
                schedule(map, T_ULD_KEEPER, ms(0));
            }
        }
        ULD_ARCHAEDAS => uld_archaedas(map, value),
        ULD_GUARDIANS | ULD_WARDERS => {
            if get_data(map, ULD_ARCHAEDAS) == IN_PROGRESS && get_data(map, field) == NOT_STARTED {
                store(map, field, value);
                schedule(
                    map,
                    if field == ULD_GUARDIANS {
                        T_ULD_GUARDIANS
                    } else {
                        T_ULD_WARDERS
                    },
                    ms(0),
                );
            }
        }
        _ => {
            settle(map, field, value);
        }
    }
}

fn uld_archaedas(map: &mut Map, value: u32) {
    let current = get_data(map, ULD_ARCHAEDAS);
    match (current, value) {
        (DONE, _) => {}
        (IN_PROGRESS, IN_PROGRESS) => schedule(map, T_ULD_WALL, ms(0)),
        (_, IN_PROGRESS) => {
            store(map, ULD_GUARDIANS, NOT_STARTED);
            store(map, ULD_WARDERS, NOT_STARTED);
            store(map, ULD_ANCIENT_DOOR, IN_PROGRESS);
            store(map, ULD_ARCHAEDAS, IN_PROGRESS);
            for g in creatures(map, ULD_ARCHAEDAS_ENTRY) {
                cast_self(map, g, 10347);
                uld_wake(map, g, None, 4000);
            }
            talk(map, ULD_ARCHAEDAS_ENTRY, 3400);
        }
        (IN_PROGRESS, FAILED) => {
            store(map, ULD_ANCIENT_DOOR, FAILED);
            store(map, ULD_ARCHAEDAS, FAILED);
        }
        (_, NOT_STARTED) => {
            store(map, ULD_GUARDIANS, NOT_STARTED);
            store(map, ULD_WARDERS, NOT_STARTED);
            store(map, ULD_ARCHAEDAS, NOT_STARTED);
            schedule(map, T_ULD_RESET, ms(0));
        }
        (_, DONE) => {
            store(map, ULD_ANCIENT_DOOR, DONE);
            store(map, ULD_ARCHAEDAS, DONE);
            schedule(map, T_ULD_CLEAR, ms(0));
        }
        _ => {}
    }
}

fn uld_object(map: &mut Map, entry: u32) {
    if entry == 124371 && get_data(map, ULD_IRONAYA_DOOR) == NOT_STARTED {
        store(map, ULD_IRONAYA_DOOR, IN_PROGRESS);
        schedule(map, T_ULD_SEAL, ms(27_000));
    }
}

fn uld_creature(map: &mut Map, g: Guid, entry: u32, spawn: Option<u32>, event: Event) {
    match event {
        Event::Spawned if ULD_TRACKED.contains(&entry) => {
            let role = uld_role(entry, spawn);
            uld_put(map, g, role, Status::Frozen);
            if role == Role::Ironaya {
                if get_data(map, ULD_IRONAYA_DOOR) == DONE {
                    uld_thaw(map, g);
                    faction(map, g, 415);
                } else {
                    uld_freeze(map, g);
                }
            }
        }
        Event::Death => {
            let Some(role) = uld_role_of(map, g) else {
                return;
            };
            uld_put(map, g, role, Status::Dead);
            match role {
                Role::Keeper if get_data(map, ULD_KEEPERS) == IN_PROGRESS => {
                    schedule(map, T_ULD_KEEPER, ms(0));
                }
                Role::Archaedas => uld_archaedas(map, DONE),
                _ => {}
            }
        }
        Event::Evade => match uld_role_of(map, g) {
            Some(Role::Keeper) if get_data(map, ULD_KEEPERS) == IN_PROGRESS => {
                let mut keepers = uld_all(map, Role::Keeper, Status::Awake);
                keepers.extend(uld_all(map, Role::Keeper, Status::Dead));
                store(map, ULD_KEEPERS, FAILED);
                for k in keepers {
                    uld_put(map, k, Role::Keeper, Status::Frozen);
                    respawn(map, k);
                }
            }
            Some(Role::Archaedas) => uld_archaedas(map, FAILED),
            _ => {}
        },
        _ => {}
    }
}

fn uld_timer(map: &mut Map, key: u32) {
    match key {
        T_ULD_SEAL => {
            settle(map, ULD_IRONAYA_DOOR, DONE);
            for g in creatures(map, ULD_IRONAYA) {
                uld_wake(map, g, Some(415), 500);
            }
        }
        T_ULD_KEEPER if get_data(map, ULD_KEEPERS) == IN_PROGRESS => {
            if uld_first(map, Role::Keeper, Status::Awake).is_some() {
                return;
            }
            if let Some(k) = uld_first(map, Role::Keeper, Status::Frozen) {
                uld_put(map, k, Role::Keeper, Status::Awake);
                uld_wake(map, k, Some(470), 500);
            } else if !uld_all(map, Role::Keeper, Status::Dead).is_empty() {
                settle(map, ULD_KEEPERS, DONE);
            } else {
                settle(map, ULD_KEEPERS, FAILED);
            }
        }
        T_ULD_WALL if get_data(map, ULD_ARCHAEDAS) == IN_PROGRESS => {
            if let Some(g) = uld_first(map, Role::Wall, Status::Frozen) {
                uld_put(map, g, Role::Wall, Status::Awake);
                if let Some(a) = creatures(map, ULD_ARCHAEDAS_ENTRY).into_iter().next() {
                    crate::game::map::spells::cast_triggered(map, a, 10259, Some(g));
                }
                uld_awaken_earthen(map, g, None);
            }
        }
        T_ULD_GUARDIANS => {
            for g in uld_all(map, Role::Guardian, Status::Frozen) {
                uld_put(map, g, Role::Guardian, Status::Awake);
                uld_awaken_earthen(map, g, None);
            }
        }
        T_ULD_WARDERS => {
            for g in uld_all(map, Role::Furniture, Status::Frozen) {
                uld_put(map, g, Role::Furniture, Status::Dead);
                despawn(map, g);
            }
            for g in uld_all(map, Role::Warder, Status::Frozen) {
                uld_put(map, g, Role::Warder, Status::Awake);
                uld_awaken_earthen(map, g, Some(415));
            }
        }
        T_ULD_RESET => {
            for role in [Role::Wall, Role::Guardian, Role::Warder, Role::Furniture] {
                let mut l = uld_all(map, role, Status::Awake);
                l.extend(uld_all(map, role, Status::Dead));
                for g in l {
                    uld_put(map, g, role, Status::Frozen);
                    respawn(map, g);
                }
            }
        }
        T_ULD_CLEAR => {
            for role in [Role::Wall, Role::Guardian, Role::Warder] {
                let mut l = uld_all(map, role, Status::Frozen);
                l.extend(uld_all(map, role, Status::Awake));
                for g in l {
                    uld_put(map, g, role, Status::Dead);
                    despawn(map, g);
                }
            }
            for g in uld_all(map, Role::Furniture, Status::Dead) {
                uld_put(map, g, Role::Furniture, Status::Frozen);
                crate::game::map::ai::respawn_now(map, g);
            }
        }
        _ => {}
    }
}

/// One creature's steps of its waking: the stone stirring, then falling
/// away and the fight.
fn uld_creature_timer(map: &mut Map, key: u32, g: Guid) {
    match key {
        T_ULD_AWAKEN_CAST => cast_self(map, g, 10254),
        T_ULD_AWAKEN_STONE => {
            remove_aura(map, g, ULD_STONED);
            attack_nearest(map, g, 80.0);
        }
        T_ULD_ATTACK => attack_nearest(map, g, 80.0),
        _ => {}
    }
}

/// An earthen wakes: free to move at once, its stone stirring after a
/// second and falling away at four, when it fights.
fn uld_awaken_earthen(map: &mut Map, g: Guid, faction_id: Option<u32>) {
    unit_flags(map, g, ULD_FROZEN, 2);
    if let Some(f) = faction_id {
        faction(map, g, f);
    }
    super::instance_script::schedule_for(map, T_ULD_AWAKEN_CAST, g, ms(1000));
    super::instance_script::schedule_for(map, T_ULD_AWAKEN_STONE, g, ms(4000));
}

// Blackrock Spire: the Hall of Blackhand's runes, the Dragonspine Door,
// Drakkisath's gates and the rookery.

const BRS_ROOMS: [(u32, &[u32]); 7] = [
    (175197, &[40259, 40260]),
    (175199, &[40254, 40255, 45834]),
    (175195, &[40251]),
    (175200, &[40262, 40263]),
    (175198, &[40267, 40268, 45833]),
    (175196, &[40270, 45832]),
    (175194, &[40277, 40455]),
];
const BRS_SLAIN: u32 = 1;
const BRS_WAVE: u32 = 1;
const BRS_BRAZIERS: [(u32, u32); 3] = [(175528, 175529), (175530, 175531), (175532, 175533)];
const T_BRS_DOOR: u32 = TIMERS + 500;
const T_BRS_ROOKERY: u32 = TIMERS + 510;
const BRS_ROOKERY: [[u32; 2]; 5] = [
    [10683, 10683],
    [10258, 10683],
    [10258, 10258],
    [10683, 10683],
    [10258, 10683],
];

fn brs_set(map: &mut Map, field: u32, value: u32) {
    let before = get_data(map, field);
    let stored = settle(map, field, value);
    if stored == before {
        return;
    }
    match (field, stored) {
        (0, DONE) => operate(map, 175244, Act::Open),
        (5, DONE) => schedule(map, T_BRS_DOOR + 1, ms(2000)),
        (7, DONE) => {
            operate(map, 175946, Act::Open);
            operate(map, 175947, Act::Open);
        }
        (6, IN_PROGRESS) => schedule(map, T_BRS_ROOKERY, ms(5000)),
        _ => {}
    }
}

/// As the copy loads: the rooms whose guards lie dead have their runes
/// out, and the Dragonspine Door and its braziers stand open once opened.
fn brs_settle(map: &mut Map) {
    let guards: Vec<u32> = BRS_ROOMS
        .iter()
        .flat_map(|(_, g)| g.iter().copied())
        .collect();
    for spawn in dead_spawns(map, &guards) {
        mark(map, BRS_SLAIN, spawn);
    }
    for (rune, _) in BRS_ROOMS {
        if brs_room_clear(map, rune) {
            operate(map, rune, Act::Close);
        }
    }
    if get_data(map, 5) == DONE {
        for (l, r) in BRS_BRAZIERS {
            operate(map, l, Act::Open);
            operate(map, r, Act::Open);
        }
        operate(map, 164725, Act::Open);
    }
}

fn brs_room_clear(map: &Map, rune: u32) -> bool {
    let slain = seen(map, BRS_SLAIN);
    BRS_ROOMS
        .iter()
        .find(|(r, _)| *r == rune)
        .is_some_and(|(_, guards)| guards.iter().all(|g| slain.contains(g)))
}

fn brs_object(map: &mut Map, entry: u32) {
    if entry == 175245 && matches!(get_data(map, 6), 0 | 2) && get_data(map, 7) != DONE {
        set_scratch(map, BRS_WAVE, 0);
        brs_set(map, 6, IN_PROGRESS);
    }
}

fn brs_creature(map: &mut Map, spawn: Option<u32>, event: Event) {
    let (Event::Death, Some(spawn)) = (event, spawn) else {
        return;
    };
    let Some((rune, _)) = BRS_ROOMS.iter().find(|(_, guards)| guards.contains(&spawn)) else {
        return;
    };
    mark(map, BRS_SLAIN, spawn);
    if brs_room_clear(map, *rune) {
        operate(map, *rune, Act::Close);
        if BRS_ROOMS.iter().all(|(r, _)| brs_room_clear(map, *r)) {
            brs_set(map, 0, DONE);
        }
    }
}

fn brs_timer(map: &mut Map, key: u32) {
    if (T_BRS_DOOR..T_BRS_ROOKERY).contains(&key) {
        let step = (key - T_BRS_DOOR) as usize;
        if let Some((l, r)) = BRS_BRAZIERS.get(step.wrapping_sub(1)).copied() {
            operate(map, l, Act::Open);
            operate(map, r, Act::Open);
            schedule(map, key + 1, ms(3000));
        } else {
            operate(map, 164725, Act::Open);
        }
        return;
    }
    if key != T_BRS_ROOKERY || get_data(map, 6) != IN_PROGRESS {
        return;
    }
    const SPOTS: [[f32; 4]; 2] = [
        [55.232342, -265.751282, 93.883, 5.0],
        [60.011333, -263.914703, 94.022, 5.0],
    ];
    let wave = scratch(map, BRS_WAVE) as usize;
    if let Some(entries) = BRS_ROOKERY.get(wave) {
        for (i, (entry, spot)) in entries.iter().zip(SPOTS).enumerate() {
            let g = summon(map, *entry, spot, Some((3_600_000, 7)));
            if wave == 0
                && i == 0
                && let Some(g) = g
            {
                talk_as(map, g, 5538);
            }
        }
        set_scratch(map, BRS_WAVE, wave as u32 + 1);
        schedule(map, T_BRS_ROOKERY, between(30_000, 40_000));
    } else {
        brs_set(map, 6, DONE);
        summon(
            map,
            10264,
            [43.7685, -259.82, 91.6483, 0.0],
            Some((3_600_000, 7)),
        );
    }
}

// Dire Maul: the crystal generators and Immol'thar, Zevrim and Alzzin, and
// King Gordok's tribute.

const DM_GENERATORS: [(u32, &[u32]); 5] = [
    (
        177259,
        &[84210, 84211, 84212, 84213, 84214, 84215, 84216, 84218],
    ),
    (
        177257,
        &[84223, 84224, 84225, 84227, 84230, 84231, 84234, 84235],
    ),
    (
        177258,
        &[84219, 84220, 84221, 84222, 84239, 84240, 84241, 84242],
    ),
    (
        179504,
        &[84245, 84246, 84247, 84257, 84258, 84259, 84260, 84261],
    ),
    (179505, &[84262, 84264, 84265, 84266, 84267]),
];
const DM_HALL_SUMMONERS: [u32; 9] = [
    56952, 300172, 300851, 300852, 300853, 300854, 300855, 300856, 300857,
];
const DM_TRIBUTE_GUARDS: [u32; 6] = [14321, 14322, 14323, 14326, 14325, 14324];
const DM_FALLEN: u32 = 1;
const DM_SUMMONERS: u32 = 1;
const DM_CHORUSH_DEAD: u32 = 1;
const DM_IMMOLTHAR: u32 = 11496;
const DM_TORTHELDRIN: u32 = 11486;
const DM_CHORUSH: u32 = 14324;
const DM_GUARDS_ALIVE: u32 = 15;
const T_DM_SUMMONER_ATTACKS: u32 = TIMERS + 800;
const T_DM_CHORUSH_TALKS: u32 = TIMERS + 801;
const DM_IMPRISONED: u32 = 0x0200_0002;
const DM_FREED: u32 = 0x0200_0202;

fn dm_set(map: &mut Map, field: u32, value: u32) {
    match (field, value) {
        (6, SPECIAL) => {
            let p = if get_data(map, 6) == DONE {
                DONE
            } else {
                IN_PROGRESS
            };
            store(map, 6, p);
        }
        (6, DONE) | (11, DONE) => {
            if get_data(map, field) != DONE {
                store(map, field, DONE);
                if field == 6 {
                    raise_object(map, 396409, 3_600_000);
                } else {
                    for s in [44726, 44727, 44728, 44729, 44730] {
                        raise_object(map, s, 3_600_000);
                    }
                }
            }
        }
        _ => {
            settle(map, field, value);
        }
    }
}

/// As the copy loads: the generators whose guards lie dead are dark, and
/// the tribute chest and the Felvine Shards stand while their fights are
/// won.
fn dm_settle(map: &mut Map) {
    let guards: Vec<u32> = DM_GENERATORS
        .iter()
        .flat_map(|(_, g)| g.iter().copied())
        .collect();
    for spawn in dead_spawns(map, &guards) {
        mark(map, DM_FALLEN, spawn);
    }
    for (generator, _) in DM_GENERATORS {
        if dm_dark(map, generator) {
            operate(map, generator, Act::Open);
        }
    }
    if get_data(map, 6) == DONE {
        raise_object(map, 396409, 3_600_000);
    }
    if get_data(map, 11) == DONE {
        for s in [44726, 44727, 44728, 44729, 44730] {
            raise_object(map, s, 3_600_000);
        }
    }
}

fn dm_dark(map: &Map, generator: u32) -> bool {
    let fallen = seen(map, DM_FALLEN);
    DM_GENERATORS
        .iter()
        .find(|(g, _)| *g == generator)
        .is_some_and(|(_, guards)| guards.iter().all(|x| fallen.contains(x)))
}

fn dm_unlock_tortheldrin(map: &mut Map) {
    for g in creatures(map, DM_TORTHELDRIN) {
        unit_flags(map, g, IMMUNE_TO_PLAYER, 2);
        faction(map, g, 14);
    }
}

fn dm_creature(map: &mut Map, g: Guid, entry: u32, spawn: Option<u32>, event: Event) {
    match event {
        Event::Spawned => match entry {
            DM_IMMOLTHAR if get_data(map, 1) != DONE => unit_flags(map, g, DM_IMPRISONED, 1),
            DM_TORTHELDRIN if get_data(map, 2) == DONE => dm_unlock_tortheldrin(map),
            DM_CHORUSH if get_data(map, 6) == DONE => {
                faction(map, g, 35);
                stand(map, g, 1);
            }
            11466 if spawn.is_some_and(|s| DM_HALL_SUMMONERS.contains(&s)) => {
                push(map, DM_SUMMONERS, g);
            }
            _ => {}
        },
        Event::Death => {
            if entry == 11466 {
                remove(map, DM_SUMMONERS, g);
            }
            if let Some(spawn) = spawn
                && let Some((generator, _)) = DM_GENERATORS
                    .iter()
                    .find(|(_, guards)| guards.contains(&spawn))
            {
                mark(map, DM_FALLEN, spawn);
                if get_data(map, 1) == DONE || !dm_dark(map, *generator) {
                    return;
                }
                operate(map, *generator, Act::Open);
                if DM_GENERATORS.iter().all(|(x, _)| dm_dark(map, *x)) {
                    settle(map, 1, DONE);
                    dm_free_immolthar(map);
                }
                return;
            }
            dm_died(map, entry);
        }
        _ => {}
    }
}

fn dm_free_immolthar(map: &mut Map) {
    for g in creatures(map, DM_IMMOLTHAR) {
        unit_flags(map, g, DM_FREED, 2);
    }
    let hall = Vector3d {
        x: -38.08,
        y: 812.44,
        z: -29.45,
    };
    for s in creatures(map, 11466) {
        if !map.position_of(s).is_some_and(|p| dist(p, hall) <= 100.0) {
            continue;
        }
        unit_flags(map, s, DM_FREED, 2);
        faction(map, s, 100);
        super::instance_script::schedule_for(map, T_DM_SUMMONER_ATTACKS, s, ms(1000));
    }
    if let Some(first) = list(map, DM_SUMMONERS).into_iter().next() {
        talk_as(map, first, 9364);
    }
}

/// A Highborne Summoner freed turns on the nearest Immol'thar; Cho'Rush,
/// standing down, speaks.
fn dm_creature_timer(map: &mut Map, key: u32, g: Guid) {
    match key {
        T_DM_SUMMONER_ATTACKS => {
            let Some(at) = map.position_of(g) else {
                return;
            };
            let nearest = creatures(map, DM_IMMOLTHAR)
                .into_iter()
                .filter_map(|i| map.position_of(i).map(|p| (dist(p, at), i)))
                .filter(|(d, _)| *d <= 100.0)
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, i)| i);
            if let Some(i) = nearest {
                crate::game::map::ai::attacked(map, g, i);
            }
        }
        T_DM_CHORUSH_TALKS if map.creature(g).is_some_and(|c| c.alive()) => talk_as(map, g, 9472),
        _ => {}
    }
}

fn dm_died(map: &mut Map, entry: u32) {
    match entry {
        DM_IMMOLTHAR => {
            settle(map, 2, DONE);
            talk(map, DM_TORTHELDRIN, 9407);
            dm_unlock_tortheldrin(map);
        }
        11490 => {
            settle(map, 4, DONE);
        }
        11492 => dm_set(map, 11, DONE),
        11501 => {
            summon(map, 14353, [693.44, 480.806, 28.175, 0.02757], None);
            if scratch(map, DM_CHORUSH_DEAD) == 0 {
                for c in creatures(map, DM_CHORUSH) {
                    faction(map, c, 35);
                    super::instance_script::schedule_for(map, T_DM_CHORUSH_TALKS, c, ms(5000));
                }
            }
        }
        e if DM_TRIBUTE_GUARDS.contains(&e) => {
            if e == DM_CHORUSH {
                set_scratch(map, DM_CHORUSH_DEAD, 1);
            }
            if e == 14326 {
                store(map, 10, DONE);
            }
            if get_data(map, 6) != DONE {
                let alive = get_data(map, DM_GUARDS_ALIVE);
                store(map, DM_GUARDS_ALIVE, alive.saturating_sub(1));
            }
        }
        _ => {}
    }
}

// Blackrock Depths: the Lyceum, Magmus, the Tomb of the Seven and the Ring
// of Law.

const BRD_RING: u32 = 0;
const BRD_TOMB: u32 = 3;
const BRD_LYCEUM: u32 = 4;
const BRD_IRON_HALL: u32 = 5;
const BRD_ARENA_STAGE: u32 = 46;
const BRD_BEAST_GATE: u32 = 161525;
const BRD_CHAMPION_GATE: u32 = 161522;
const BRD_SEVEN: [u32; 7] = [9035, 9038, 9040, 9037, 9036, 9034, 9039];
const BRD_RING_MOBS: [u32; 6] = [8925, 8926, 8927, 8928, 8933, 8932];
const BRD_CHAMPIONS: [u32; 6] = [9027, 9028, 9029, 9030, 9031, 9032];
const BRD_CROWD: [u32; 7] = [8916, 8896, 8902, 8904, 8893, 8894, 8895];
const BRD_GRIMSTONE: u32 = 10096;
const BRD_BRAZIER: u32 = 1;
const BRD_SLAIN: u32 = 2;
const BRD_MOBS: u32 = 1;
const BRD_CHAMPION: u32 = 2;
const T_BRD_TOMB: u32 = TIMERS + 600;
const T_BRD_RING_CHAMPION: u32 = TIMERS + 610;
const T_BRD_RING_FINISH: u32 = TIMERS + 611;
const T_BRD_SENTENCE: u32 = TIMERS + 612;
const T_BRD_PULSE: u32 = TIMERS + 613;

fn brd_set(map: &mut Map, field: u32, value: u32) {
    match field {
        BRD_LYCEUM => {
            let was = get_data(map, BRD_LYCEUM);
            let value = if was == DONE { DONE } else { value };
            store(map, BRD_LYCEUM, value);
            if was != DONE && value == DONE {
                talk(map, 9938, 5430);
            }
        }
        BRD_TOMB => brd_tomb(map, value),
        BRD_RING => brd_ring(map, value),
        _ => store(map, field, value),
    }
}

fn brd_tomb(map: &mut Map, value: u32) {
    let current = get_data(map, BRD_TOMB);
    match (current, value) {
        (NOT_STARTED, IN_PROGRESS) => {
            store(map, BRD_TOMB, IN_PROGRESS);
            schedule(map, T_BRD_TOMB, ms(0));
        }
        (IN_PROGRESS, FAILED) => {
            store(map, BRD_TOMB, NOT_STARTED);
            brd_cancel_calls(map);
            for e in BRD_SEVEN {
                for g in map
                    .creatures
                    .iter()
                    .filter(|(_, c)| c.entry == e)
                    .map(|(_, c)| c.guid)
                    .collect::<Vec<_>>()
                {
                    faction(map, g, 0);
                    unit_flags(map, g, IMMUNE_TO_PLAYER, 1);
                    crate::game::map::ai::respawn_now(map, g);
                }
            }
        }
        (c, DONE) if c != DONE => {
            store(map, BRD_TOMB, DONE);
            brd_cancel_calls(map);
            raise_object(map, 399065, 3_600_000);
        }
        (c, NOT_STARTED) if c != IN_PROGRESS => store(map, BRD_TOMB, NOT_STARTED),
        _ => {}
    }
}

fn brd_cancel_calls(map: &mut Map) {
    for i in 0..7 {
        cancel(map, T_BRD_TOMB + i);
    }
}

fn brd_ring(map: &mut Map, value: u32) {
    let current = get_data(map, BRD_RING);
    match (current, value) {
        (NOT_STARTED, IN_PROGRESS) => {
            store(map, BRD_RING, IN_PROGRESS);
            if let Some(g) = summon(
                map,
                BRD_GRIMSTONE,
                [625.559, -205.618, -52.735, 2.609],
                None,
            ) && let Some(s) = crate::game::map::event_ai::scripted_mut(map, g)
            {
                s.run = false;
            }
            // He passes sentence, then walks his path into the ring.
            schedule(map, T_BRD_SENTENCE, ms(1000));
        }
        (FAILED, IN_PROGRESS) => {
            store(map, BRD_RING, IN_PROGRESS);
            for e in BRD_CHAMPIONS {
                for g in creatures(map, e) {
                    zone_pulse(map, g);
                }
            }
        }
        (c, DONE) if c != DONE => {
            store(map, BRD_RING, DONE);
            for e in BRD_CROWD {
                brd_calm(map, e);
            }
        }
        _ => {}
    }
}

fn brd_grimstone_resume(map: &mut Map, point: u32) {
    for g in creatures(map, BRD_GRIMSTONE) {
        step(map, g, 60, [5, point, 0, 0]);
    }
}

fn brd_calm(map: &mut Map, entry: u32) {
    let stands = Vector3d {
        x: 595.78,
        y: -188.65,
        z: -35.5,
    };
    for g in creatures(map, entry) {
        if map.position_of(g).is_some_and(|p| dist(p, stands) <= 69.0) {
            faction(map, g, 15);
        }
    }
}

fn brd_object(map: &mut Map, entry: u32) {
    if !matches!(entry, 174744 | 174745) {
        return;
    }
    match get_data(map, BRD_LYCEUM) {
        DONE => {}
        IN_PROGRESS if scratch(map, BRD_BRAZIER) == entry => {}
        IN_PROGRESS => brd_set(map, BRD_LYCEUM, DONE),
        _ => {
            brd_set(map, BRD_LYCEUM, IN_PROGRESS);
            set_scratch(map, BRD_BRAZIER, entry);
        }
    }
}

fn brd_creature(map: &mut Map, g: Guid, entry: u32, event: Event) {
    if entry == 9938 {
        let v = match event {
            Event::Aggro => IN_PROGRESS,
            Event::Evade => FAILED,
            Event::Death => DONE,
            Event::Spawned => return,
        };
        brd_set(map, BRD_IRON_HALL, v);
        return;
    }
    if entry == 9039 && event == Event::Death {
        brd_set(map, BRD_TOMB, DONE);
        return;
    }
    if BRD_SEVEN.contains(&entry) && event == Event::Evade {
        if get_data(map, BRD_TOMB) == IN_PROGRESS {
            brd_set(map, BRD_TOMB, FAILED);
        }
        return;
    }
    let ring = get_data(map, BRD_RING);
    match event {
        Event::Spawned => {
            if BRD_RING_MOBS.contains(&entry) && ring == IN_PROGRESS {
                push(map, BRD_MOBS, g);
            } else if BRD_CHAMPIONS.contains(&entry) && ring == IN_PROGRESS {
                map.instance.lists.insert(BRD_CHAMPION, vec![g]);
            } else if BRD_CROWD.contains(&entry) && ring == DONE {
                brd_calm(map, entry);
            }
        }
        Event::Death => {
            if list(map, BRD_CHAMPION).contains(&g) {
                map.instance.lists.remove(&BRD_CHAMPION);
                schedule(map, T_BRD_RING_FINISH, ms(5000));
            } else if remove(map, BRD_MOBS, g) {
                let slain = scratch(map, BRD_SLAIN) + 1;
                set_scratch(map, BRD_SLAIN, slain);
                if slain == 8 {
                    schedule(map, T_BRD_RING_CHAMPION, ms(10_000));
                }
            }
        }
        Event::Evade => {
            let champion = list(map, BRD_CHAMPION);
            if ring == IN_PROGRESS && (champion.contains(&g) || list(map, BRD_MOBS).contains(&g)) {
                store(map, BRD_ARENA_STAGE, 0);
                if champion.is_empty() {
                    store(map, BRD_RING, NOT_STARTED);
                    for m in list(map, BRD_MOBS) {
                        despawn(map, m);
                    }
                    for gs in creatures(map, BRD_GRIMSTONE) {
                        despawn(map, gs);
                    }
                    cancel(map, T_BRD_RING_CHAMPION);
                    cancel(map, T_BRD_RING_FINISH);
                    map.instance.lists.remove(&BRD_MOBS);
                    set_scratch(map, BRD_SLAIN, 0);
                } else {
                    store(map, BRD_RING, FAILED);
                }
            }
        }
        Event::Aggro => {}
    }
}

fn brd_timer(map: &mut Map, key: u32) {
    if (T_BRD_TOMB..T_BRD_TOMB + 7).contains(&key) {
        if get_data(map, BRD_TOMB) != IN_PROGRESS {
            return;
        }
        let round = key - T_BRD_TOMB;
        let entry = BRD_SEVEN[round as usize];
        for g in creatures(map, entry) {
            unit_flags(map, g, IMMUNE_TO_PLAYER, 2);
            faction(map, g, 54);
            super::instance_script::schedule_for(map, T_BRD_PULSE, g, ms(500));
        }
        if round + 1 < 7 {
            schedule(map, key + 1, ms(30_000));
        }
        return;
    }
    if key == T_BRD_SENTENCE {
        talk(map, BRD_GRIMSTONE, 5441);
        brd_grimstone_resume(map, 0);
        return;
    }
    if matches!(key, T_BRD_RING_CHAMPION | T_BRD_RING_FINISH)
        && get_data(map, BRD_RING) == IN_PROGRESS
    {
        store(map, BRD_ARENA_STAGE, 0);
        brd_grimstone_resume(map, if key == T_BRD_RING_CHAMPION { 3 } else { 5 });
    }
}

// Stratholme: the Baron's run, the ziggurats and Ramstein, the Black
// Guard, Timmy, Willey's gate and the postboxes.

const STRAT_BARON_RUN: u32 = 0;
const STRAT_RAMSTEIN: u32 = 4;
const STRAT_BARON: u32 = 5;
const STRAT_CRYSTALS: u32 = 6;
const STRAT_RAMSTEIN_EVENT: u32 = 8;
const STRAT_BARON_ENTRY: u32 = 10440;
const STRAT_YSIDA: u32 = 16031;
const STRAT_ZIGGURAT_FOUR: u32 = 175405;
const STRAT_ZIGGURAT_FIVE: u32 = 175796;
const STRAT_PORT_GAUNTLET: u32 = 175374;
const STRAT_PORT_SLAUGHTER: u32 = 175373;
const STRAT_SLAUGHTER_GATE: u32 = 175358;
const STRAT_BARON_LOCKED: u32 = 0x0200_0002;
const STRAT_ACOLYTES: [(u32, [u32; 5]); 3] = [
    (53955, [53268, 53269, 53270, 53271, 53272]),
    (53963, [53257, 53258, 53259, 53260, 53261]),
    (53968, [53262, 53263, 53264, 53265, 53266]),
];
const STRAT_ABOMINATIONS: [u32; 13] = [
    53969, 54002, 54018, 54019, 54020, 54021, 54022, 54026, 54027, 54039, 54040, 54041, 54050,
];
const STRAT_POSTBOXES: [(u32, [f32; 4]); 6] = [
    (176346, [3690.03, -3405.43, 132.838, -1.74533]),
    (176349, [3651.28, -3165.66, 128.177, 2.02458]),
    (176350, [3659.79, -3476.73, 138.381, 1.02974]),
    (176351, [3639.65, -3641.33, 138.541, -1.23918]),
    (176352, [3564.92, -3353.24, 130.67, -0.855212]),
    (176353, [3473.59, -3294.16, 132.052, 2.19912]),
];
// Seen sets and flags.
const S_DEAD_ACOLYTES: u32 = 1;
const S_DEAD_CRYSTALS: u32 = 2;
const S_TRIGGERED_CRYSTALS: u32 = 3;
const S_DEAD_ABOMINATIONS: u32 = 4;
const S_RAMSTEIN_SUMMONED: u32 = 10;
const S_RAMSTEIN_ARRIVED: u32 = 11;
const S_RAMSTEIN_DEAD: u32 = 12;
const S_SLAUGHTER_OPEN: u32 = 13;
const S_GUARDS_ANNOUNCED: u32 = 14;
const S_TIMMY: u32 = 15;
const S_POSTBOXES: u32 = 16;
const S_BLACK_GUARDS_DEAD: u32 = 17;
const L_ABOMINATION_QUEUE: u32 = 1;
const T_STRAT_10_MIN: u32 = TIMERS + 700;
const T_STRAT_5_MIN: u32 = TIMERS + 701;
const T_STRAT_YSIDA: u32 = TIMERS + 702;
const T_STRAT_1_MIN: u32 = TIMERS + 703;
const T_STRAT_EXPIRED: u32 = TIMERS + 704;
const T_STRAT_YSIDA_REWARD: u32 = TIMERS + 705;
const T_STRAT_ABOMINATION: u32 = TIMERS + 706;
const T_STRAT_RAMSTEIN_ARRIVAL: u32 = TIMERS + 707;
const T_STRAT_BLACK_GUARD: u32 = TIMERS + 708;
const T_STRAT_GATE_RESET: u32 = TIMERS + 709;
const T_STRAT_MALOWN: u32 = TIMERS + 710;
const STRAT_MALOWN_MS: u64 = 180_000;
const S_MALOWN_FOUGHT: u32 = 18;

fn strat_players_cast(map: &mut Map, spell: u32) {
    for p in players(map) {
        crate::game::map::spells::cast_triggered(map, p, spell, Some(p));
    }
}

fn strat_set(map: &mut Map, field: u32, value: u32) {
    match field {
        STRAT_BARON_RUN => strat_baron_run(map, value),
        1..=3 => {
            store(map, field, value);
        }
        STRAT_RAMSTEIN => strat_ramstein(map, value),
        STRAT_BARON => {
            store(map, STRAT_BARON, value);
            let act = if value == IN_PROGRESS {
                Act::Close
            } else {
                Act::Open
            };
            if matches!(value, IN_PROGRESS | FAILED | DONE) {
                for gate in [
                    STRAT_ZIGGURAT_FOUR,
                    STRAT_ZIGGURAT_FIVE,
                    STRAT_PORT_GAUNTLET,
                ] {
                    operate(map, gate, act);
                }
            }
            if value == DONE && get_data(map, STRAT_BARON_RUN) == IN_PROGRESS {
                store(map, STRAT_BARON_RUN, DONE);
                strat_run_done(map);
            }
        }
        _ => store(map, field, value),
    }
}

fn strat_cancel_run(map: &mut Map) {
    for k in T_STRAT_10_MIN..=T_STRAT_EXPIRED {
        cancel(map, k);
    }
}

fn strat_baron_run(map: &mut Map, value: u32) {
    store(map, STRAT_BARON_RUN, value);
    match value {
        IN_PROGRESS => {
            summon(
                map,
                STRAT_YSIDA,
                [4044.163, -3334.2, 115.0596, 4.2],
                Some((3_600_000, 1)),
            );
            talk(map, STRAT_BARON_ENTRY, 11812);
            strat_players_cast(map, 27861);
            let min = |m: u64| Duration::from_secs(m * 60);
            schedule(map, T_STRAT_10_MIN, min(35));
            schedule(map, T_STRAT_5_MIN, min(40));
            schedule(map, T_STRAT_YSIDA, min(40) + ms(3000));
            schedule(map, T_STRAT_1_MIN, min(44));
            schedule(map, T_STRAT_EXPIRED, min(45));
        }
        DONE => strat_run_done(map),
        FAILED => strat_run_failed(map),
        _ => {}
    }
}

fn strat_run_done(map: &mut Map) {
    strat_cancel_run(map);
    for p in players(map) {
        for s in [27861, 27863, 27864, 27865] {
            remove_aura(map, p, s);
        }
        crate::game::map::quests::killed(map, p, guid::unit(STRAT_YSIDA, 0));
    }
    operate(map, 181071, Act::Open);
    for y in creatures(map, STRAT_YSIDA) {
        npc_flags(map, y, 0x3, 1);
        walk_to(map, y, [4041.2, -3339.0, 115.1], false);
    }
    schedule(map, T_STRAT_YSIDA_REWARD, ms(5000));
}

fn strat_run_failed(map: &mut Map) {
    strat_cancel_run(map);
    talk(map, STRAT_BARON_ENTRY, 11814);
    operate(map, 181071, Act::Open);
    talk(map, STRAT_YSIDA, 11817);
    for y in creatures(map, STRAT_YSIDA) {
        cast_self(map, y, 5);
    }
}

fn strat_ramstein(map: &mut Map, value: u32) {
    store(map, STRAT_RAMSTEIN, value);
    match value {
        SPECIAL => {
            operate(map, STRAT_PORT_GAUNTLET, Act::Close);
            schedule(map, T_STRAT_ABOMINATION, ms(20_000));
        }
        FAILED => {
            operate(map, STRAT_PORT_GAUNTLET, Act::Open);
            cancel(map, T_STRAT_ABOMINATION);
        }
        DONE => {
            operate(map, STRAT_SLAUGHTER_GATE, Act::Open);
            for i in 0..34u32 {
                let at = [
                    3929.6 + (i % 6) as f32 * 0.5,
                    -3384.3 + (i / 6) as f32 * 0.5,
                    120.0,
                    4.88,
                ];
                if let Some(g) = summon(map, 11030, at, Some((1_800_000, 1))) {
                    walk_to(map, g, [3941.29, -3394.84, 119.69], false);
                }
            }
            schedule(map, T_STRAT_BLACK_GUARD, ms(60_000));
            schedule(map, T_STRAT_GATE_RESET, ms(91_000));
        }
        _ => {}
    }
}

fn strat_object(map: &mut Map, entry: u32) {
    if entry == 175357 && get_data(map, STRAT_BARON_RUN) == NOT_STARTED {
        strat_baron_run(map, IN_PROGRESS);
        return;
    }
    let Some((_, spot)) = STRAT_POSTBOXES.iter().find(|(e, _)| *e == entry) else {
        return;
    };
    let uses = scratch(map, S_POSTBOXES) + 1;
    if uses > 3 {
        return;
    }
    set_scratch(map, S_POSTBOXES, uses);
    for turn in [0.0f32, 2.0, -2.0] {
        let a = spot[3] + turn;
        let at = [
            spot[0] + 1.5 * a.cos(),
            spot[1] + 1.5 * a.sin(),
            spot[2],
            a + std::f32::consts::PI,
        ];
        summon(map, 11142, at, None);
    }
    // Postmaster Malown, for three minutes: not ended mid-fight, and his
    // corpse kept, as a timed-or-dead summon.
    if uses == 3
        && let Some(g) = summon(map, 11143, *spot, None)
    {
        super::instance_script::schedule_for(map, T_STRAT_MALOWN, g, ms(STRAT_MALOWN_MS));
    }
}

/// Postmaster Malown's time is up: he goes, unless he has died or is
/// fighting; a fight over starts his time again.
fn strat_malown_due(map: &mut Map, g: Guid) {
    if !map.creature(g).is_some_and(|c| c.alive()) {
        return;
    }
    let fighting = map.creature(g).is_some_and(|c| {
        c.combat.attacking
            || c.fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            )
    });
    let fought = map
        .instance
        .seen
        .get(&S_MALOWN_FOUGHT)
        .is_some_and(|l| l.contains(&(g.guid() as u32)));
    if fighting {
        mark(map, S_MALOWN_FOUGHT, g.guid() as u32);
        super::instance_script::schedule_for(map, T_STRAT_MALOWN, g, ms(1000));
    } else if fought {
        if let Some(l) = map.instance.seen.get_mut(&S_MALOWN_FOUGHT) {
            l.retain(|x| *x != g.guid() as u32);
        }
        super::instance_script::schedule_for(map, T_STRAT_MALOWN, g, ms(STRAT_MALOWN_MS));
    } else {
        despawn(map, g);
    }
}

fn strat_creature(map: &mut Map, g: Guid, entry: u32, spawn: Option<u32>, event: Event) {
    match (entry, event) {
        (STRAT_BARON_ENTRY, Event::Spawned) if get_data(map, STRAT_RAMSTEIN) != DONE => {
            unit_flags(map, g, STRAT_BARON_LOCKED, 1);
        }
        (10436, Event::Death) => strat_door(map, 1),
        (10437, Event::Death) => strat_door(map, 2),
        (10438, Event::Death) => strat_door(map, 3),
        (10399, Event::Death) => {
            let Some(spawn) = spawn else {
                return;
            };
            if !mark(map, S_DEAD_ACOLYTES, spawn) {
                return;
            }
            let dead = seen(map, S_DEAD_ACOLYTES);
            let crystal = STRAT_ACOLYTES
                .iter()
                .find(|(_, a)| a.contains(&spawn) && a.iter().all(|x| dead.contains(x)))
                .map(|(c, _)| *c);
            if let Some(c) = crystal
                && mark(map, S_TRIGGERED_CRYSTALS, c)
                && let Some(cg) = crate::game::map::scripts::creature_of_spawn(map, c)
            {
                cast_self(map, cg, 5);
            }
        }
        (10415, Event::Death) => {
            let Some(spawn) = spawn else {
                return;
            };
            if !mark(map, S_DEAD_CRYSTALS, spawn) {
                return;
            }
            talk_as(map, g, 6527);
            let dead = seen(map, S_DEAD_CRYSTALS);
            if STRAT_ACOLYTES.iter().all(|(c, _)| dead.contains(c)) {
                store(map, STRAT_CRYSTALS, DONE);
                operate(map, STRAT_PORT_GAUNTLET, Act::Open);
                operate(map, STRAT_PORT_SLAUGHTER, Act::Open);
                talk_as(map, g, 6289);
            }
        }
        (10416 | 10417, Event::Spawned) => push(map, L_ABOMINATION_QUEUE, g),
        (10416 | 10417, Event::Death) => {
            remove(map, L_ABOMINATION_QUEUE, g);
            let id = spawn.unwrap_or(g.guid() as u32);
            if !mark(map, S_DEAD_ABOMINATIONS, id) {
                return;
            }
            let dead = seen(map, S_DEAD_ABOMINATIONS);
            if STRAT_ABOMINATIONS.iter().all(|a| dead.contains(a))
                && scratch(map, S_RAMSTEIN_SUMMONED) == 0
            {
                set_scratch(map, S_RAMSTEIN_SUMMONED, 1);
                store(map, STRAT_RAMSTEIN, IN_PROGRESS);
                store(map, STRAT_RAMSTEIN_EVENT, DONE);
                talk(map, STRAT_BARON_ENTRY, 6398);
                operate(map, STRAT_ZIGGURAT_FOUR, Act::Open);
                if let Some(r) = summon(
                    map,
                    10439,
                    [4032.35, -3380.567, 119.739571, 4.7614],
                    Some((1_800_000, 1)),
                ) {
                    walk_to(map, r, [4033.009, -3404.3293, 115.3554], false);
                }
                schedule(map, T_STRAT_RAMSTEIN_ARRIVAL, ms(5000));
            } else {
                store(map, STRAT_RAMSTEIN, IN_PROGRESS);
            }
        }
        (10439, Event::Aggro) => {
            store(map, STRAT_RAMSTEIN, IN_PROGRESS);
            operate(map, STRAT_PORT_GAUNTLET, Act::Close);
        }
        (10439, Event::Evade) => strat_ramstein(map, FAILED),
        (10439, Event::Death) if scratch(map, S_RAMSTEIN_DEAD) == 0 => {
            set_scratch(map, S_RAMSTEIN_DEAD, 1);
            set_scratch(map, S_SLAUGHTER_OPEN, 1);
            strat_ramstein(map, DONE);
        }
        (10394, Event::Spawned) if scratch(map, S_GUARDS_ANNOUNCED) == 0 => {
            set_scratch(map, S_GUARDS_ANNOUNCED, 1);
            talk_as(map, g, 6415);
        }
        (10394, Event::Death) => {
            if mark(map, S_BLACK_GUARDS_DEAD, g.guid() as u32)
                && seen(map, S_BLACK_GUARDS_DEAD).len() == 5
            {
                talk(map, STRAT_BARON_ENTRY, 6401);
            }
        }
        (10418, Event::Death) if spawn == Some(54070) && scratch(map, S_TIMMY) == 0 => {
            set_scratch(map, S_TIMMY, 1);
            if let Some(t) = summon(map, 10808, [3614.7, -3187.64, 131.406, 0.0], None) {
                talk_as(map, t, 6150);
            }
        }
        (10997, Event::Aggro) => operate(map, 175969, Act::Close),
        (10997, Event::Evade | Event::Death) => operate(map, 175969, Act::Open),
        _ => {}
    }
}

fn strat_door(map: &mut Map, field: u32) {
    strat_set(map, field, DONE);
}

fn strat_settle(map: &mut Map) {
    // Who lies dead from before: the acolytes, their crystals, the
    // abominations.
    let acolytes: Vec<u32> = STRAT_ACOLYTES.iter().flat_map(|(_, a)| *a).collect();
    for spawn in dead_spawns(map, &acolytes) {
        mark(map, S_DEAD_ACOLYTES, spawn);
    }
    let crystals: Vec<u32> = STRAT_ACOLYTES.iter().map(|(c, _)| *c).collect();
    for spawn in dead_spawns(map, &crystals) {
        mark(map, S_TRIGGERED_CRYSTALS, spawn);
        mark(map, S_DEAD_CRYSTALS, spawn);
    }
    for spawn in dead_spawns(map, &STRAT_ABOMINATIONS) {
        mark(map, S_DEAD_ABOMINATIONS, spawn);
    }
    if get_data(map, STRAT_RAMSTEIN_EVENT) == DONE {
        set_scratch(map, S_RAMSTEIN_SUMMONED, 1);
    }
    if get_data(map, STRAT_RAMSTEIN) == DONE {
        set_scratch(map, S_RAMSTEIN_DEAD, 1);
    }
    let baron = get_data(map, STRAT_BARON);
    let ramstein = get_data(map, STRAT_RAMSTEIN);
    if baron == IN_PROGRESS || matches!(ramstein, IN_PROGRESS | SPECIAL) {
        operate(map, STRAT_PORT_GAUNTLET, Act::Close);
    } else if get_data(map, STRAT_CRYSTALS) == DONE {
        operate(map, STRAT_PORT_GAUNTLET, Act::Open);
    }
    if get_data(map, STRAT_CRYSTALS) == DONE {
        operate(map, STRAT_PORT_SLAUGHTER, Act::Open);
    }
    if baron == IN_PROGRESS {
        operate(map, STRAT_ZIGGURAT_FOUR, Act::Close);
        operate(map, STRAT_ZIGGURAT_FIVE, Act::Close);
    } else if ramstein == DONE {
        operate(map, STRAT_ZIGGURAT_FOUR, Act::Open);
        operate(map, STRAT_ZIGGURAT_FIVE, Act::Open);
    } else if get_data(map, STRAT_RAMSTEIN_EVENT) == DONE {
        operate(map, STRAT_ZIGGURAT_FOUR, Act::Open);
    }
}

fn strat_timer(map: &mut Map, key: u32) {
    let running = get_data(map, STRAT_BARON_RUN) == IN_PROGRESS;
    match key {
        T_STRAT_10_MIN if running => {
            talk(map, STRAT_BARON_ENTRY, 11813);
            strat_players_cast(map, 27863);
        }
        T_STRAT_5_MIN if running => {
            talk(map, STRAT_BARON_ENTRY, 11815);
            strat_players_cast(map, 27864);
        }
        T_STRAT_YSIDA if running => talk(map, STRAT_YSIDA, 11816),
        T_STRAT_1_MIN if running => strat_players_cast(map, 27865),
        T_STRAT_EXPIRED if running => {
            store(map, STRAT_BARON_RUN, FAILED);
            strat_run_failed(map);
        }
        T_STRAT_YSIDA_REWARD if get_data(map, STRAT_BARON_RUN) == DONE => {
            talk(map, STRAT_YSIDA, 11931);
        }
        T_STRAT_ABOMINATION => {
            let mut queue = list(map, L_ABOMINATION_QUEUE);
            if queue.is_empty() {
                return;
            }
            let g = queue.remove(0);
            map.instance
                .lists
                .insert(L_ABOMINATION_QUEUE, queue.clone());
            let bile = map.creature(g).is_some_and(|c| c.entry == 10416);
            walk_to(map, g, [4037.194, -3473.741943, 121.738808], false);
            if !queue.is_empty() {
                schedule(
                    map,
                    T_STRAT_ABOMINATION,
                    ms(if bile { 45_000 } else { 37_500 }),
                );
            }
        }
        T_STRAT_RAMSTEIN_ARRIVAL => {
            set_scratch(map, S_RAMSTEIN_ARRIVED, 1);
            operate(map, STRAT_ZIGGURAT_FOUR, Act::Close);
            talk(map, 10439, 6425);
        }
        T_STRAT_BLACK_GUARD => {
            for i in 0..5u32 {
                let at = [
                    4032.84 + (i % 3) as f32,
                    -3380.567 + (i / 3) as f32,
                    119.739571,
                    4.7614,
                ];
                if let Some(g) = summon(map, 10394, at, Some((1_800_000, 1))) {
                    walk_to(map, g, [4033.34, -3419.75, 116.35], false);
                }
            }
            operate(map, STRAT_ZIGGURAT_FOUR, Act::Open);
            operate(map, STRAT_ZIGGURAT_FIVE, Act::Open);
            for b in creatures(map, STRAT_BARON_ENTRY) {
                unit_flags(map, b, STRAT_BARON_LOCKED, 2);
            }
        }
        T_STRAT_GATE_RESET => {
            set_scratch(map, S_SLAUGHTER_OPEN, 0);
            operate(map, STRAT_SLAUGHTER_GATE, Act::Close);
        }
        _ => {}
    }
}
