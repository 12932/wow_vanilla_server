//! Upper Blackrock Spire's stadium: Rend Blackhand's gauntlet, as
//! mangos-classic's `instance_blackrock_spire` runs it (`at_blackrock_spire`
//! on area trigger 2026, `DoSendNextStadiumWave`, the stadium dialogue),
//! with vmangos's data: Lord Victor Nefarius and Rend stand on the balcony
//! as spawns, and their lines are broadcast texts.
//!
//! A living player (not a GM) stepping into the stadium starts it, unless
//! it is under way or won: the entry gate shuts, Nefarius speaks, and the
//! Blackhand spectators come to watch from the western balcony. Seven
//! waves of whelps, dragonspawn and handlers follow, each sixty seconds
//! after the last or as soon as the last is dead. With the seventh dead
//! Nefarius loses patience, Rend rides off to bring Gyth, and thirty
//! seconds on Gyth comes, Rend on his back; low on health he throws Rend
//! off (Gyth's own EventAI: Summon Rend Blackhand). With both dead
//! Nefarius concedes, and the gates open. One of the event's summons
//! leaving its fight fails it: Nefarius, Rend, Gyth and the spectators go,
//! and the next player through starts it again.

use std::time::Duration;

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use super::instance_adapters::{Act, TIMERS, operate};
use super::instance_script::{DONE, Event, FAILED, IN_PROGRESS, get_data, schedule, store};
use crate::game::map::Map;
use crate::game::map::scripts;

pub const AT_STADIUM: u32 = 2026;
/// The stadium's encounter field (vmangos's and mangos's `TYPE_STADIUM`).
pub const STADIUM: u32 = 3;

pub const NEFARIUS: u32 = 10162;
pub const REND: u32 = 10429;
pub const GYTH: u32 = 10339;
pub const WHELP: u32 = 10442;
pub const DRAGONSPAWN: u32 = 10447;
pub const HANDLER: u32 = 10742;
const VETERAN: u32 = 9819;
const ELITE: u32 = 10317;

pub const ENTRY_DOOR: u32 = 164726;
pub const COMBAT_DOOR: u32 = 175185;
pub const EXIT_DOOR: u32 = 175186;

// The dialogue's lines (vmangos's broadcast texts).
pub const SAY_INTRO_1: u32 = 5635;
pub const SAY_INTRO_2: u32 = 5640;
pub const SAY_LOSE: u32 = 5709;
pub const SAY_REND_ATTACK: u32 = 5722;
pub const SAY_WARCHIEF: u32 = 5720;
pub const SAY_VICTORY: u32 = 5824;
const NEFARIUS_TAUNTS: [u32; 8] = [5665, 5671, 5666, 5667, 5668, 5669, 5664, 5719];
const REND_TAUNTS: [u32; 4] = [5672, 5678, 5673, 5674];

pub const MAX_WAVES: u32 = 7;
/// The waves, as `aStadiumEventNpcs` has them.
pub const WAVES: [&[u32]; MAX_WAVES as usize] = [
    &[WHELP, WHELP, WHELP, DRAGONSPAWN],
    &[WHELP, WHELP, WHELP, DRAGONSPAWN],
    &[WHELP, WHELP, DRAGONSPAWN, HANDLER],
    &[WHELP, WHELP, DRAGONSPAWN, HANDLER],
    &[WHELP, WHELP, WHELP, DRAGONSPAWN, HANDLER],
    &[WHELP, WHELP, DRAGONSPAWN, DRAGONSPAWN, HANDLER],
    &[WHELP, WHELP, DRAGONSPAWN, DRAGONSPAWN, HANDLER],
];

/// `aStadiumLocs`: where the dragons come from, Gyth's gate, the middle of
/// the floor, and where Nefarius and Rend stand and go.
const DRAGONS_AT: [f32; 3] = [210.00, -420.30, 110.94];
const GYTH_AT: [f32; 3] = [210.14, -397.54, 111.1];
const CENTRE: [f32; 3] = [163.62, -420.33, 110.47];
const NEFARIUS_TO: [f32; 4] = [164.64, -443.30, 121.97, 1.61];
const REND_TO: [f32; 3] = [165.74, -466.46, 116.80];

const SPECTATORS: [u32; 12] = [
    VETERAN, VETERAN, VETERAN, ELITE, VETERAN, VETERAN, VETERAN, VETERAN, VETERAN, ELITE, VETERAN,
    VETERAN,
];
const SPECTATORS_FROM: [[f32; 4]; 12] = [
    [163.3209, -340.9818, 111.0216, 4.818223],
    [164.2471, -339.0313, 111.0368, 1.413717],
    [161.124, -339.5178, 111.0381, 3.001966],
    [162.5045, -337.8101, 111.0367, 4.13643],
    [160.9896, -337.7715, 111.0368, 1.117011],
    [161.8347, -335.7923, 111.0352, 2.286381],
    [113.9726, -366.0805, 116.9195, 6.252025],
    [112.7245, -368.9635, 116.9307, 4.677482],
    [110.5757, -368.2123, 116.9278, 4.310963],
    [109.3343, -366.4785, 116.9261, 2.740167],
    [110.1331, -363.9824, 116.9272, std::f32::consts::FRAC_PI_6],
    [111.9971, -363.0948, 116.929, 5.951573],
];
const SPECTATORS_TO: [[f32; 4]; 12] = [
    [160.619, -395.826, 121.9752, -1.502597],
    [162.1428, -395.1175, 121.9751, -1.67753],
    [158.6822, -395.7097, 121.9753, -1.787977],
    [164.384, -395.3787, 121.9751, -1.502597],
    [156.9669, -395.2188, 121.9752, -1.678662],
    [166.2515, -395.0366, 121.975, -1.791467],
    [143.814, -396.7092, 121.9753, -1.40136],
    [145.3893, -396.1959, 121.9752, -1.419479],
    [142.1598, -396.0284, 121.9752, -1.661444],
    [147.7274, -396.3042, 121.9753, -1.40136],
    [139.9446, -396.7277, 121.9753, -1.428414],
    [149.3754, -395.7497, 121.9753, -1.714769],
];

/// The dialogue, as `aStadiumDialogue` runs it: each step's line (and
/// who says it), and how long until the next (none: it waits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Intro1,
    Intro2,
    Spectators,
    Lose,
    RendAttack,
    Warchief,
    Victory,
    NefariusLeaves,
}

/// A step, its line (speaker, text), and the wait to the next.
type Line = (Step, Option<(u32, u32)>, Option<u64>);

const DIALOGUE: [Line; 8] = [
    (Step::Intro1, Some((NEFARIUS, SAY_INTRO_1)), Some(7000)),
    (Step::Intro2, Some((NEFARIUS, SAY_INTRO_2)), Some(5000)),
    (Step::Spectators, None, None),
    (Step::Lose, Some((NEFARIUS, SAY_LOSE)), Some(3000)),
    (Step::RendAttack, Some((REND, SAY_REND_ATTACK)), Some(2000)),
    (Step::Warchief, Some((NEFARIUS, SAY_WARCHIEF)), None),
    (Step::Victory, Some((NEFARIUS, SAY_VICTORY)), Some(5000)),
    (Step::NefariusLeaves, None, None),
];

// Timers: the dialogue's steps, and the next wave.
const T_DIALOGUE: u32 = TIMERS + 5000;
pub const T_WAVE: u32 = TIMERS + 5100;
// What the copy keeps meanwhile: the waves sent, the event's summons
// alive, the spectators.
pub const WAVES_SENT: u32 = 5000;
pub const ALIVE: u32 = 5001;
const SPECTATOR_LIST: u32 = 5000;

/// Gyth's EventAI event that throws Rend off, and the health it does so
/// under: mangos-classic's 11 percent (`boss_gyth`), where vmangos's row
/// has 20.
const GYTH_THROWS_REND: u32 = 1033902;
pub const GYTH_THROWS_AT: i32 = 11;

/// The world's data as the stadium needs it: Gyth throws Rend at
/// mangos-classic's health.
pub fn install(data: &mut crate::game::data::GameData) {
    if let Some(events) = data.scripts.ai_events.get_mut(&GYTH) {
        for e in events.iter_mut().filter(|e| e.id == GYTH_THROWS_REND) {
            e.params[0] = GYTH_THROWS_AT;
        }
    }
}

/// The doors the stadium works, which players may not.
pub const DOOR_ENTRIES: [u32; 3] = [ENTRY_DOOR, COMBAT_DOOR, EXIT_DOOR];

/// The entry gate stands open but while the event is under way; the exit
/// opens once it is won.
pub fn entry_open(d: &std::collections::BTreeMap<u32, u32>) -> bool {
    d.get(&STADIUM).copied().unwrap_or(0) != IN_PROGRESS
}

fn scratch(map: &Map, key: u32) -> u32 {
    map.instance.scratch.get(&key).copied().unwrap_or(0)
}

fn set_scratch(map: &mut Map, key: u32, value: u32) {
    map.instance.scratch.insert(key, value);
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn v3(p: [f32; 3]) -> Vector3d {
    Vector3d {
        x: p[0],
        y: p[1],
        z: p[2],
    }
}

/// The balcony's Nefarius or Rend: the spawn (the one the copy keeps, as
/// vmangos stores only the first Rend), dead or alive.
fn spawned(map: &Map, entry: u32) -> Option<Guid> {
    map.creatures
        .iter()
        .filter(|(_, c)| c.entry == entry && c.spawn_guid.is_some())
        .map(|(_, c)| (c.spawn_guid, c.guid))
        .min()
        .map(|(_, g)| g)
}

fn alive(map: &Map, g: Guid) -> bool {
    map.creature(g).is_some_and(|c| c.alive())
}

fn living_spawned(map: &Map, entry: u32) -> Option<Guid> {
    spawned(map, entry).filter(|g| alive(map, *g))
}

/// The event's own: one of its entries the copy summoned (some of the
/// same entries stand about the spire as spawns).
fn of_event(map: &Map, g: Guid) -> bool {
    map.creature(g).is_some_and(|c| {
        c.spawn_guid.is_none() && matches!(c.entry, WHELP | DRAGONSPAWN | HANDLER | GYTH | REND)
    })
}

/// Area trigger 2026: a player into the stadium.
pub fn area_trigger(map: &mut Map, player: Guid, id: u32) {
    if id != AT_STADIUM {
        return;
    }
    let Some(p) = map.players.get(&player) else {
        return;
    };
    if p.gm || p.death != crate::game::map::death::Death::Alive {
        return;
    }
    if matches!(get_data(map, STADIUM), IN_PROGRESS | DONE) {
        return;
    }
    // Nefarius and Rend back on the balcony if a failed try took them.
    for entry in [NEFARIUS, REND] {
        if let Some(g) = spawned(map, entry)
            && !alive(map, g)
        {
            crate::game::map::ai::respawn_now(map, g);
        }
    }
    // Rend can't be struck up there (mangos's `UNIT_FLAG_IMMUNE_TO_PLAYER`).
    if let Some(r) = living_spawned(map, REND) {
        flags(map, r, 0x100, 1);
    }
    set(map, IN_PROGRESS);
}

/// The stadium's field set, as `SetData(TYPE_STADIUM)` has it: the same
/// twice does nothing.
pub fn set(map: &mut Map, value: u32) {
    if get_data(map, STADIUM) == value {
        return;
    }
    store(map, STADIUM, value);
    match value {
        IN_PROGRESS => say_step(map, 0),
        DONE => {
            despawn_spectators(map);
            operate(map, EXIT_DOOR, Act::Open);
        }
        FAILED => {
            for entry in [NEFARIUS, REND] {
                if let Some(g) = living_spawned(map, entry) {
                    super::instance_script::despawn(map, g, false);
                }
            }
            for g in living(map, GYTH) {
                scripts::despawn(map, g);
            }
            despawn_spectators(map);
            super::instance_script::cancel(map, T_WAVE);
            for (i, _) in DIALOGUE.iter().enumerate() {
                super::instance_script::cancel(map, T_DIALOGUE + i as u32);
            }
            set_scratch(map, WAVES_SENT, 0);
            set_scratch(map, ALIVE, 0);
        }
        _ => {}
    }
}

fn living(map: &Map, entry: u32) -> Vec<Guid> {
    map.creatures
        .iter()
        .filter(|(_, c)| c.entry == entry && c.alive())
        .map(|(_, c)| c.guid)
        .collect()
}

fn despawn_spectators(map: &mut Map) {
    let gone = map
        .instance
        .lists
        .remove(&SPECTATOR_LIST)
        .unwrap_or_default();
    for g in gone {
        if map.creature(g).is_some() {
            scripts::despawn(map, g);
        }
    }
}

/// One step of a field flag: set (1) or cleared (2).
fn flags(map: &mut Map, g: Guid, flags: u32, mode: u32) {
    let s = crate::game::data::scripts::ScriptStep {
        command: 4,
        datalong: [crate::game::fields::UNIT_FIELD_FLAGS as u32, flags, mode, 0],
        ..Default::default()
    };
    scripts::execute(map, &s, g, g);
}

/// A dialogue step: its line said, what it does done, and the next one
/// timed.
fn say_step(map: &mut Map, i: usize) {
    let Some(&(step, line, next)) = DIALOGUE.get(i) else {
        return;
    };
    if let Some((entry, text)) = line
        && let Some(g) = living_spawned(map, entry)
    {
        scripts::talk(map, g, text, Guid::zero(), None);
    }
    did_step(map, step);
    if let Some(after) = next {
        schedule(map, T_DIALOGUE + i as u32 + 1, ms(after));
    }
}

fn did_step(map: &mut Map, step: Step) {
    match step {
        Step::Spectators => {
            schedule(map, T_WAVE, ms(1000));
            if let Some(r) = living_spawned(map, REND) {
                face(map, r, NEFARIUS_TO[3]);
            }
            let Some(n) = living_spawned(map, NEFARIUS) else {
                return;
            };
            scripts::move_with(
                map,
                n,
                v3([NEFARIUS_TO[0], NEFARIUS_TO[1], NEFARIUS_TO[2]]),
                scripts::Move {
                    face: Some(NEFARIUS_TO[3]),
                    ..Default::default()
                },
            );
            for (i, entry) in SPECTATORS.iter().enumerate() {
                let [x, y, z, o] = SPECTATORS_FROM[i];
                let Some(g) = scripts::summon(map, n, *entry, v3([x, y, z]), o, None) else {
                    continue;
                };
                if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
                    s.despawn_type = 7;
                    s.run = true;
                    s.reaction = Some(crate::game::map::assistance::Reaction::Defensive);
                }
                let [tx, ty, tz, to] = SPECTATORS_TO[i];
                home(map, g, [tx, ty, tz, to]);
                scripts::move_with(
                    map,
                    g,
                    v3([tx, ty, tz]),
                    scripts::Move {
                        run: true,
                        pathfind: true,
                        face: Some(to),
                        ..Default::default()
                    },
                );
                let l = map.instance.lists.entry(SPECTATOR_LIST).or_default();
                l.push(g);
            }
        }
        // Rend rides off for Gyth; thirty seconds on he comes.
        Step::Warchief => {
            if let Some(r) = living_spawned(map, REND) {
                if let Some(s) = crate::game::map::event_ai::scripted_mut(map, r) {
                    s.run = true;
                }
                scripts::move_with(
                    map,
                    r,
                    v3(REND_TO),
                    scripts::Move {
                        run: true,
                        pathfind: true,
                        ..Default::default()
                    },
                );
                despawn_after(map, r, 5000);
            }
            schedule(map, T_WAVE, ms(30_000));
        }
        Step::Victory => set(map, DONE),
        Step::NefariusLeaves => {
            if let Some(n) = living_spawned(map, NEFARIUS) {
                scripts::move_with(
                    map,
                    n,
                    v3(REND_TO),
                    scripts::Move {
                        pathfind: true,
                        ..Default::default()
                    },
                );
                despawn_after(map, n, 5000);
            }
        }
        _ => {}
    }
}

/// It goes after `after_ms` (`ForcedDespawn`): a spawn back on its own
/// time.
fn despawn_after(map: &mut Map, g: Guid, after_ms: u32) {
    let s = crate::game::data::scripts::ScriptStep {
        command: 18,
        datalong: [after_ms, 0, 0, 0],
        ..Default::default()
    };
    scripts::execute(map, &s, g, g);
}

fn face(map: &mut Map, g: Guid, o: f32) {
    let s = crate::game::data::scripts::ScriptStep {
        command: 35,
        datalong: [4, 0, 0, 0],
        o,
        ..Default::default()
    };
    scripts::execute(map, &s, g, g);
}

/// Its home where it is going (`SetHomePosition`), so a fight ends there.
fn home(map: &mut Map, g: Guid, at: [f32; 4]) {
    let s = crate::game::data::scripts::ScriptStep {
        command: 34,
        datalong: [0, 0, 0, 0],
        x: at[0],
        y: at[1],
        z: at[2],
        o: at[3],
        ..Default::default()
    };
    scripts::execute(map, &s, g, g);
}

/// A point at random within `radius` of `at`, no farther east than it (a
/// half circle, as the stadium is long).
fn half_circle(at: [f32; 3], radius: f32) -> Vector3d {
    let mut rng = rand::rng();
    let angle = rng.random_range(0.0..std::f32::consts::TAU);
    let r = radius * rng.random_range(0.0f32..1.0).sqrt();
    Vector3d {
        x: (at[0] + r * angle.cos()).min(at[0]),
        y: at[1] + r * angle.sin(),
        z: at[2],
    }
}

/// `DoSendNextStadiumWave`: the next wave, the dialogue once the seventh
/// is dead, or Gyth.
pub fn send_next_wave(map: &mut Map) {
    let sent = scratch(map, WAVES_SENT);
    let Some(n) = living_spawned(map, NEFARIUS) else {
        set_scratch(map, WAVES_SENT, sent + 1);
        return;
    };
    match sent {
        s if s < MAX_WAVES => {
            for entry in WAVES[s as usize] {
                let at = half_circle(DRAGONS_AT, 7.0);
                let Some(g) = scripts::summon(map, n, *entry, at, 0.0, None) else {
                    continue;
                };
                if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
                    s.despawn_type = 7;
                }
                let to = half_circle(CENTRE, 5.0);
                scripts::move_with(
                    map,
                    g,
                    to,
                    scripts::Move {
                        pathfind: true,
                        ..Default::default()
                    },
                );
                let alive = scratch(map, ALIVE);
                set_scratch(map, ALIVE, alive + 1);
            }
            use_combat_door(map);
        }
        MAX_WAVES => say_step(map, 3),
        _ => {
            scripts::move_to(
                map,
                n,
                v3([NEFARIUS_TO[0], NEFARIUS_TO[1], NEFARIUS_TO[2]]),
                false,
            );
            if let Some(g) = scripts::summon(map, n, GYTH, v3(GYTH_AT), 0.0, None) {
                if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
                    s.despawn_type = 7;
                }
                scripts::move_with(
                    map,
                    g,
                    v3(CENTRE),
                    scripts::Move {
                        pathfind: true,
                        ..Default::default()
                    },
                );
            }
            // Two to kill: Gyth, and Rend once Gyth throws him.
            set_scratch(map, ALIVE, 2);
            use_combat_door(map);
        }
    }
    let sent = sent + 1;
    set_scratch(map, WAVES_SENT, sent);
    if sent >= MAX_WAVES {
        super::instance_script::cancel(map, T_WAVE);
    } else {
        schedule(map, T_WAVE, ms(60_000));
    }
}

/// The dragons' gate, used: it opens, and shuts again on its own.
fn use_combat_door(map: &mut Map) {
    let doors: Vec<Guid> = map
        .gameobjects
        .iter()
        .filter(|(_, o)| o.entry == COMBAT_DOOR)
        .map(|(_, o)| o.guid)
        .collect();
    for d in doors {
        crate::game::map::objects::used_by_script(map, Guid::zero(), d);
    }
}

/// A creature's event in the spire.
pub fn creature_event(map: &mut Map, g: Guid, event: Event) {
    if !of_event(map, g) {
        return;
    }
    let Some(entry) = map.creature(g).map(|c| c.entry) else {
        return;
    };
    match event {
        Event::Death if matches!(entry, WHELP | DRAGONSPAWN | HANDLER) => {
            let mut rng = rand::rng();
            if rng.random_range(0..=100) < 5
                && let Some(n) = living_spawned(map, NEFARIUS)
            {
                let t = NEFARIUS_TAUNTS[rng.random_range(0..NEFARIUS_TAUNTS.len())];
                scripts::talk(map, n, t, Guid::zero(), None);
            }
            if rng.random_range(0..=100) < 5
                && let Some(r) = living_spawned(map, REND)
            {
                let t = REND_TAUNTS[rng.random_range(0..REND_TAUNTS.len())];
                scripts::talk(map, r, t, Guid::zero(), None);
            }
            let alive = scratch(map, ALIVE).saturating_sub(1);
            set_scratch(map, ALIVE, alive);
            if alive == 0 && get_data(map, STADIUM) == IN_PROGRESS {
                send_next_wave(map);
            }
        }
        Event::Death => {
            let alive = scratch(map, ALIVE).saturating_sub(1);
            set_scratch(map, ALIVE, alive);
            if alive == 0 && get_data(map, STADIUM) == IN_PROGRESS {
                say_step(map, 6);
            }
        }
        Event::Evade if get_data(map, STADIUM) == IN_PROGRESS => {
            set(map, FAILED);
            scripts::despawn(map, g);
        }
        _ => {}
    }
}

/// The stadium's timed steps; whether the key was its.
pub fn timer(map: &mut Map, key: u32) -> bool {
    if key == T_WAVE {
        if get_data(map, STADIUM) == IN_PROGRESS {
            send_next_wave(map);
        }
        return true;
    }
    if (T_DIALOGUE..T_DIALOGUE + DIALOGUE.len() as u32).contains(&key) {
        let i = (key - T_DIALOGUE) as usize;
        // Won, Nefarius still takes his leave.
        let leaving = DIALOGUE[i].0 == Step::NefariusLeaves && get_data(map, STADIUM) == DONE;
        if get_data(map, STADIUM) == IN_PROGRESS || leaving {
            say_step(map, i);
        }
        return true;
    }
    false
}
