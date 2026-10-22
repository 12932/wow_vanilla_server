//! Nefarian's lair, as mangos-classic's `boss_victor_nefarius`,
//! `boss_nefarian` and `instance_blackwing_lair` run it.
//!
//! Lord Victor Nefarius sits at his table, speaking once Chromaggus is dead
//! (vmangos's menus; their last option starts it, by the copy's field
//! 6045). He speaks twice and rises behind his barrier to fight from where
//! he stands, the whole copy his enemy, while drakonids pour from the two
//! tunnels: the two kinds picked once for the copy (fields 11 and 12),
//! one of each every six to seven seconds, and a chromatic one from each
//! every 35 (vmangos's rates: mangos's world spawns them from spawner
//! creatures it has and vmangos's hasn't). With 42 drakonids dead,
//! Nefarius blinks away and, five seconds on, Nefarian comes: he flies in
//! (vmangos's flight, mangos's path being its world's), lands, and four
//! seconds on fights, his timed casts mangos's, a class call every 35 to
//! 40 seconds on the class of one of those he fights, and below 20 percent
//! he raises the drakonids' bones. His death wins the encounter; his
//! leaving his fight, or Nefarius's return home, fails it, and all is put
//! back.

// Places as vmangos and mangos have them, to the digit.
#![allow(clippy::excessive_precision)]

use std::time::Duration;

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use super::*;
use crate::game::fields::*;
use crate::game::map::instance_script::{cancel, cancel_for, schedule};

pub const NPC_BLUE_DRAKONID: u32 = 14261;
pub const NPC_GREEN_DRAKONID: u32 = 14262;
pub const NPC_BRONZE_DRAKONID: u32 = 14263;
pub const NPC_RED_DRAKONID: u32 = 14264;
pub const NPC_BLACK_DRAKONID: u32 = 14265;
pub const NPC_CHROMATIC_DRAKONID: u32 = 14302;
pub const NPC_BONE_CONSTRUCT: u32 = 14605;
pub const GO_DRAKONID_BONES: u32 = 179804;
/// The kinds a tunnel may give, in the order of their selection (mangos's
/// black, red, green, bronze and blue spawners).
pub const TUNNEL_KINDS: [u32; 5] = [
    NPC_BLACK_DRAKONID,
    NPC_RED_DRAKONID,
    NPC_GREEN_DRAKONID,
    NPC_BRONZE_DRAKONID,
    NPC_BLUE_DRAKONID,
];
const DRAKONIDS: [u32; 6] = [
    NPC_BLUE_DRAKONID,
    NPC_GREEN_DRAKONID,
    NPC_BRONZE_DRAKONID,
    NPC_RED_DRAKONID,
    NPC_BLACK_DRAKONID,
    NPC_CHROMATIC_DRAKONID,
];
pub const MAX_DRAKONID_SUMMONS: u32 = 42;

/// The tunnels' mouths (`aNefarianLocs`).
const TUNNELS: [[f32; 3]; 2] = [[-7599.32, -1191.72, 475.545], [-7526.27, -1135.04, 473.445]];
/// Nefarian's coming: where he appears, where he flies, where he lands
/// (vmangos's).
const NEFARIAN_FROM: [f32; 3] = [-7515.644, -1222.698, 534.7169];
const NEFARIAN_VIA: [f32; 3] = [-7449.145, -1320.647, 476.795];
const NEFARIAN_LANDS: [f32; 3] = [-7495.964, -1252.402, 476.795];
const FLIGHT_TO_VIA: u64 = 6000;
const FLIGHT_TO_LANDING: u64 = 4000;

/// The gossip script's field that starts the fight (vmangos's
/// `GOSSIP_OPTION_NEFARIUS`).
pub const HOOK_START: u32 = 6045;
/// The field a drakonid's death sets, before its own death's script
/// takes it away (and with it the copy's hearing of its death).
pub const HOOK_DRAKONID_DIED: u32 = 6402;

pub const SAY_GAMESBEGIN_1: u32 = 9907;
pub const SAY_GAMESBEGIN_2: u32 = 9845;
pub const SAY_AGGRO: u32 = 9973;
pub const SAY_SHADOW_FLAME: u32 = 9974;
pub const SAY_RAISE_SKELETONS: u32 = 9883;
const SAY_SLAY: u32 = 9972;
pub const SAY_DEATH: u32 = 9971;

pub const NEFARIUS_BARRIER: u32 = 22663;
pub const SHADOWBLINK: u32 = 22664;
const SHADOWBLINK_OUTRO: u32 = 22681;
pub const SHADOW_BOLT: u32 = 22677;
pub const FEAR: u32 = 22678;
pub const SHADOW_BOLT_VOLLEY: u32 = 22665;
pub const SILENCE: u32 = 22666;
pub const SHADOW_COMMAND: u32 = 22667;

pub const SHADOW_FLAME_PASSIVE: u32 = 22992;
pub const SHADOW_FLAME: u32 = 22539;
pub const BELLOWING_ROAR: u32 = 22686;
pub const VEIL_OF_SHADOW: u32 = 22687;
pub const CLEAVE: u32 = 19983;
pub const TAIL_LASH: u32 = 23364;
pub const RAISE_DRAKONID: u32 = 23362;
const DOUBLE_ATTACK: u32 = 19818;
pub const SPEED_BURST: u32 = 25184;

/// The class calls: (class, spell, line).
pub const CLASS_CALLS: [(u8, u32, u32); 9] = [
    (1, 23397, 9855),
    (2, 23418, 9853),
    (3, 23436, 9849),
    (4, 23414, 9856),
    (5, 23401, 9848),
    (7, 23425, 9854),
    (8, 23410, 9850),
    (9, 23427, 9852),
    (11, 23398, 9851),
];

const FACTION_BLACK_DRAGON: u32 = 103;
const FACTION_FRIENDLY: u32 = 35;
const NPC_FLAG_GOSSIP: u32 = 0x1;
const UNIT_FLAG_IMMUNE_TO_PLAYER: u32 = 0x100;
const UNIT_FLAG_NOT_SELECTABLE: u32 = 0x0200_0000;
const EMOTE_ONESHOT_LAND: u32 = 293;

// The copy's timers.
const T_INTRO: u32 = TIMERS + 6400;
const T_DRAKONIDS: u32 = TIMERS + 6402;
const T_CHROMATIC: u32 = TIMERS + 6403;
const T_NEFARIAN_COMES: u32 = TIMERS + 6404;
// Nefarian's own.
const T_VIA: u32 = TIMERS + 6410;
const T_LANDED: u32 = TIMERS + 6411;
const T_ATTACK: u32 = TIMERS + 6412;
const T_FIRST_YELL: u32 = TIMERS + 6413;
const T_CLASS_CALL: u32 = TIMERS + 6414;
// What the copy keeps meanwhile.
pub const DEAD_DRAKONIDS: u32 = 6400;
const INTRO_STEP: u32 = 6401;
const SUMMONED: u32 = 6400;

fn scratch(map: &Map, key: u32) -> u32 {
    map.instance.scratch.get(&key).copied().unwrap_or(0)
}

fn set_scratch(map: &mut Map, key: u32, value: u32) {
    map.instance.scratch.insert(key, value);
}

fn v3(p: [f32; 3]) -> Vector3d {
    Vector3d {
        x: p[0],
        y: p[1],
        z: p[2],
    }
}

fn step(map: &mut Map, g: Guid, command: u32, datalong: [u32; 4]) {
    let s = ScriptStep {
        command,
        datalong,
        ..Default::default()
    };
    scripts::execute(map, &s, g, g);
}

fn flags(map: &mut Map, g: Guid, field: u16, flags: u32, on: bool) {
    step(map, g, 4, [field as u32, flags, if on { 1 } else { 2 }, 0]);
}

/// The lair's Nefarius, the spawn at his table.
fn nefarius(map: &Map) -> Option<Guid> {
    map.creatures
        .iter()
        .filter(|(_, c)| c.entry == NPC_VICTOR_NEFARIUS && c.spawn_guid.is_some())
        .map(|(_, c)| c.guid)
        .next()
}

fn alive(map: &Map, g: Guid) -> bool {
    map.creature(g).is_some_and(|c| c.alive())
}

fn nefarian(map: &Map) -> Option<Guid> {
    map.creatures
        .iter()
        .find(|(_, c)| c.entry == NPC_NEFARIAN && c.alive())
        .map(|(_, c)| c.guid)
}

/// The two kinds of drakonid, picked the first time he is seen, kept for
/// the copy (`Nefarian Tunnel Selection`).
fn select_tunnels(map: &mut Map) {
    if get_data(map, NEFA_LTUNNEL) != 0 && get_data(map, NEFA_RTUNNEL) != 0 {
        return;
    }
    let mut rng = rand::rng();
    let right = rng.random_range(0..5usize);
    let left = (right + rng.random_range(1..5usize)) % 5;
    set_data(map, NEFA_LTUNNEL, TUNNEL_KINDS[left]);
    set_data(map, NEFA_RTUNNEL, TUNNEL_KINDS[right]);
}

/// A field one of the lair's scripts sets to tell the copy to act: the
/// last of Nefarius's menus. Whether it was one.
pub fn hook(map: &mut Map, field: u32) -> bool {
    if field == HOOK_DRAKONID_DIED {
        drakonid_died(map);
        return true;
    }
    if field != HOOK_START {
        return false;
    }
    if matches!(get_data(map, NEFARIAN), NOT_STARTED | FAILED) && nefarius(map).is_some() {
        set_scratch(map, INTRO_STEP, 0);
        intro(map);
    }
    true
}

/// `aIntroDialogue`: his two lines, and then the fight.
fn intro(map: &mut Map) {
    let Some(n) = nefarius(map).filter(|g| alive(map, *g)) else {
        return;
    };
    let i = scratch(map, INTRO_STEP);
    set_scratch(map, INTRO_STEP, i + 1);
    match i {
        0 => {
            scripts::talk(map, n, SAY_GAMESBEGIN_1, Guid::zero(), None);
            schedule(map, T_INTRO, Duration::from_secs(4));
        }
        1 => {
            scripts::talk(map, n, SAY_GAMESBEGIN_2, Guid::zero(), None);
            schedule(map, T_INTRO, Duration::from_secs(5));
        }
        _ => rises(map, n),
    }
}

/// He rises behind his barrier and fights from where he stands; the
/// drakonids begin.
fn rises(map: &mut Map, n: Guid) {
    step(map, n, 28, [0, 0, 0, 0]);
    // Back at his own home or respawn.
    step(map, n, 22, [FACTION_BLACK_DRAGON, 0x1 | 0x4, 0, 0]);
    crate::game::map::spells::cast_triggered(map, n, NEFARIUS_BARRIER, Some(n));
    scripts::cast(map, n, n, SHADOWBLINK, 0);
    flags(map, n, UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_PLAYER, false);
    flags(map, n, UNIT_FIELD_FLAGS, UNIT_FLAG_NOT_SELECTABLE, true);
    if let Some(s) = crate::game::map::event_ai::scripted_mut(map, n) {
        s.no_combat_movement = true;
    }
    step(map, n, 49, [1, 0, 0, 0]);
    set_data(map, NEFARIAN, IN_PROGRESS);
    set_scratch(map, DEAD_DRAKONIDS, 0);
    schedule(map, T_DRAKONIDS, Duration::from_secs(6));
    schedule(
        map,
        T_CHROMATIC,
        Duration::from_millis(rand::rng().random_range(7000..=9000)),
    );
}

/// A drakonid out of tunnel `i`, into the fight; gone ten seconds out of
/// it.
fn drakonid(map: &mut Map, entry: u32, i: usize) {
    if entry == 0 {
        return;
    }
    let Some(g) = scripts::summon(
        map,
        Guid::zero(),
        entry,
        v3(TUNNELS[i]),
        5.0,
        Some((Duration::from_secs(10), 4)),
    ) else {
        return;
    };
    if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
        s.summoner = None;
    }
    map.instance.lists.entry(SUMMONED).or_default().push(g);
    step(map, g, 49, [1, 0, 0, 0]);
}

/// The copy's timers for the lair's last fight; whether the key was one.
pub fn timer(map: &mut Map, key: u32) -> bool {
    let fighting = get_data(map, NEFARIAN) == IN_PROGRESS;
    match key {
        T_INTRO => intro(map),
        T_DRAKONIDS if fighting => {
            let (l, r) = (get_data(map, NEFA_LTUNNEL), get_data(map, NEFA_RTUNNEL));
            drakonid(map, l, 0);
            drakonid(map, r, 1);
            schedule(
                map,
                T_DRAKONIDS,
                Duration::from_millis(rand::rng().random_range(6000..=7000)),
            );
        }
        T_CHROMATIC if fighting => {
            drakonid(map, NPC_CHROMATIC_DRAKONID, 0);
            drakonid(map, NPC_CHROMATIC_DRAKONID, 1);
            schedule(map, T_CHROMATIC, Duration::from_secs(35));
        }
        T_NEFARIAN_COMES => nefarian_comes(map),
        T_DRAKONIDS | T_CHROMATIC => {}
        _ => return false,
    }
    true
}

/// Five seconds after the last drakonid: Nefarian, and Nefarius gone.
fn nefarian_comes(map: &mut Map) {
    if get_data(map, NEFARIAN) != SPECIAL || nefarian(map).is_some() {
        return;
    }
    let Some(n) = nefarius(map) else {
        return;
    };
    let Some(g) = scripts::summon(map, n, NPC_NEFARIAN, v3(NEFARIAN_FROM), 0.0, None) else {
        return;
    };
    if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
        s.despawn_type = 7;
        s.passive = true;
        s.no_melee = true;
        s.no_combat_movement = true;
        s.fly = true;
    }
    flags(map, g, UNIT_FIELD_FLAGS, UNIT_FLAG_NOT_SELECTABLE, true);
    for aura in [SPEED_BURST, DOUBLE_ATTACK, SHADOW_FLAME_PASSIVE] {
        crate::game::map::spells::cast_triggered(map, g, aura, Some(g));
    }
    fly(map, g, NEFARIAN_VIA, FLIGHT_TO_VIA);
    schedule_for(map, T_VIA, g, Duration::from_millis(FLIGHT_TO_VIA));
    schedule_for(map, T_FIRST_YELL, g, Duration::from_secs(10));
    if alive(map, n) {
        scripts::cast(map, n, n, SHADOWBLINK_OUTRO, 0x2);
        let s = ScriptStep {
            command: 18,
            datalong: [2500, 0, 0, 0],
            ..Default::default()
        };
        scripts::execute(map, &s, n, n);
    }
}

fn fly(map: &mut Map, g: Guid, to: [f32; 3], ms: u64) {
    scripts::move_with(
        map,
        g,
        v3(to),
        scripts::Move {
            fly: true,
            time: Some(Duration::from_millis(ms)),
            ..Default::default()
        },
    );
}

/// Nefarian's own steps, and the class calls.
pub fn creature_timer(map: &mut Map, key: u32, g: Guid) -> bool {
    match key {
        T_VIA => {
            scripts::talk(map, g, SAY_AGGRO, Guid::zero(), None);
            fly(map, g, NEFARIAN_LANDS, FLIGHT_TO_LANDING);
            schedule_for(map, T_LANDED, g, Duration::from_millis(FLIGHT_TO_LANDING));
        }
        T_LANDED => {
            step(map, g, 1, [EMOTE_ONESHOT_LAND, 0, 0, 0]);
            if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
                s.fly = false;
            }
            schedule_for(map, T_ATTACK, g, Duration::from_secs(4));
        }
        T_ATTACK => attack_start(map, g),
        T_FIRST_YELL => scripts::talk(map, g, SAY_SHADOW_FLAME, Guid::zero(), None),
        T_CLASS_CALL => class_call(map, g),
        _ => return false,
    }
    true
}

/// `HandleAttackStart`: he fights, the copy his enemy.
fn attack_start(map: &mut Map, g: Guid) {
    if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
        s.passive = false;
        s.no_melee = false;
        s.no_combat_movement = false;
    }
    crate::game::map::auras::remove_spell(map, g, SPEED_BURST, None);
    flags(map, g, UNIT_FIELD_FLAGS, UNIT_FLAG_NOT_SELECTABLE, false);
    step(map, g, 49, [1, 0, 0, 0]);
    schedule_for(map, T_CLASS_CALL, g, Duration::from_secs(35));
}

/// A class call: on the class of one of those he fights, at random; the
/// spell reaches that class alone (mangos's `CheckTargetScript`).
fn class_call(map: &mut Map, g: Guid) {
    let Some(c) = map.creature(g).filter(|c| c.alive() && c.combat.attacking) else {
        return;
    };
    let players: Vec<Guid> = c
        .threat
        .iter()
        .map(|(u, _)| *u)
        .filter(|u| {
            map.players
                .get(u)
                .is_some_and(|p| p.death == crate::game::map::death::Death::Alive)
        })
        .collect();
    let mut again = 1000;
    if !players.is_empty() {
        let t = players[rand::rng().random_range(0..players.len())];
        let class = map
            .players
            .get(&t)
            .map_or(0, |p| p.fields.get_byte(UNIT_FIELD_BYTES_0, 1));
        if let Some((_, spell, text)) = CLASS_CALLS.iter().find(|(k, _, _)| *k == class)
            && scripts::cast(map, g, g, *spell, 0)
        {
            scripts::talk(map, g, *text, Guid::zero(), None);
            again = rand::rng().random_range(35_000..=40_000);
        }
    }
    schedule_for(map, T_CLASS_CALL, g, Duration::from_millis(again));
}

/// Whether a class call's spell may land on `g`: one of its class alone.
pub fn class_call_reaches(map: &Map, spell: u32, g: Guid) -> bool {
    let Some((class, _, _)) = CLASS_CALLS.iter().find(|(_, s, _)| *s == spell) else {
        return true;
    };
    map.players
        .get(&g)
        .is_some_and(|p| p.fields.get_byte(UNIT_FIELD_BYTES_0, 1) == *class)
}

pub fn creature_event(map: &mut Map, g: Guid, entry: u32, event: Event) {
    match (entry, event) {
        (NPC_VICTOR_NEFARIUS, Event::Spawned)
            if map.creature(g).is_some_and(|c| c.spawn_guid.is_some()) =>
        {
            select_tunnels(map);
            if get_data(map, NEFARIAN) != DONE {
                at_table(map, g);
            }
        }
        (NPC_NEFARIAN, Event::Death) => set_data(map, NEFARIAN, DONE),
        (NPC_NEFARIAN, Event::Evade) if get_data(map, NEFARIAN) != DONE => {
            cancel_for(map, g);
            set_data(map, NEFARIAN, FAILED);
            scripts::despawn(map, g);
        }
        _ => {}
    }
}

/// A drakonid dead: with the 42nd, the second stage.
fn drakonid_died(map: &mut Map) {
    let dead = scratch(map, DEAD_DRAKONIDS) + 1;
    set_scratch(map, DEAD_DRAKONIDS, dead);
    if dead >= MAX_DRAKONID_SUMMONS && get_data(map, NEFARIAN) == IN_PROGRESS {
        set_data(map, NEFARIAN, SPECIAL);
    }
}

/// What a change of the fight's field brings: its second stage begun (the
/// drakonids stop, Nefarian five seconds on), or it lost (all put back).
pub fn changed(map: &mut Map, new: u32) {
    match new {
        SPECIAL => {
            cancel(map, T_DRAKONIDS);
            cancel(map, T_CHROMATIC);
            schedule(map, T_NEFARIAN_COMES, Duration::from_secs(5));
        }
        FAILED => cleanup(map),
        DONE => {
            cancel(map, T_DRAKONIDS);
            cancel(map, T_CHROMATIC);
        }
        _ => {}
    }
}

/// `CleanupNefarianStage`: the drakonids, their bones and the count gone,
/// Nefarius back at his table.
fn cleanup(map: &mut Map) {
    for k in [T_INTRO, T_DRAKONIDS, T_CHROMATIC, T_NEFARIAN_COMES] {
        cancel(map, k);
    }
    set_scratch(map, DEAD_DRAKONIDS, 0);
    let gone = map.instance.lists.remove(&SUMMONED).unwrap_or_default();
    for g in gone {
        if map.creature(g).is_some() {
            scripts::despawn(map, g);
        }
    }
    let bones: Vec<Guid> = map
        .gameobjects
        .iter()
        .filter(|(_, o)| o.entry == GO_DRAKONID_BONES)
        .map(|(_, o)| o.guid)
        .collect();
    for b in bones {
        crate::game::map::objects::despawn(map, b);
    }
    if let Some(n) = nefarius(map) {
        if alive(map, n) {
            at_table(map, n);
        } else {
            crate::game::map::ai::respawn_now(map, n);
        }
    }
}

/// Nefarius as he waits at his table, one to speak to (vmangos's `Reset`;
/// the world's faction for him is a foe's): friendly, not to be struck,
/// his menu offered, and moving in a fight again. The friendly faction is
/// set as his own, not as a script's for a while, so that the faction his
/// rising sets gives way to it at his fight's end, not to the world's.
fn at_table(map: &mut Map, n: Guid) {
    if let Some(s) = crate::game::map::event_ai::scripted_mut(map, n) {
        s.temp_faction = None;
        s.no_combat_movement = false;
    }
    scripts::set_faction(map, n, FACTION_FRIENDLY);
    flags(map, n, UNIT_NPC_FLAGS, NPC_FLAG_GOSSIP, true);
    flags(map, n, UNIT_FIELD_FLAGS, UNIT_FLAG_NOT_SELECTABLE, false);
    flags(map, n, UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_PLAYER, true);
}

fn say(text: u32) -> ScriptStep {
    ScriptStep {
        command: 0,
        dataint: [text as i32, 0, 0, 0],
        ..Default::default()
    }
}

fn cast_at(spell: u32, target_type: u32) -> ScriptStep {
    ScriptStep {
        command: 15,
        datalong: [spell, 0, 0, 0],
        data_flags: SF_ABORT_ON_FAILURE,
        target_type,
        ..Default::default()
    }
}

/// Their timed casts (mangos's `CombatAI` actions), yells and returns.
pub(super) fn install(b: &mut Build) {
    let n = NPC_VICTOR_NEFARIUS;
    b.data.scripts.ai_events.remove(&n);
    b.data.scripts.ai_events.remove(&NPC_NEFARIAN);
    // Nefarius: at random of those he fights (4), or not his first (5).
    let random = |b: &mut Build, k: u32, spell: u32, t: u32, first: u32, again: (u32, u32)| {
        b.event(
            n,
            k,
            EVENT_TIMER_IN_COMBAT,
            EF_REPEATABLE | EF_CHECK_RESULT,
            [first as i32, first as i32, again.0 as i32, again.1 as i32],
            vec![cast_at(spell, t)],
        );
    };
    random(b, 1, SHADOW_BOLT, 4, 3000, (2000, 4000));
    random(b, 2, FEAR, 5, 8000, (10_000, 20_000));
    b.timer(n, 3, SHADOW_BOLT_VOLLEY, At::Me, 13_000, (19_000, 28_000));
    random(b, 4, SILENCE, 4, 23_000, (14_000, 23_000));
    random(b, 5, SHADOW_COMMAND, 5, 30_000, (24_000, 30_000));
    b.timer(n, 6, SHADOWBLINK, At::Me, 40_000, (30_000, 40_000));
    // Home from his fight: it failed, and he sits at his table again.
    b.event(
        n,
        90,
        EVENT_REACHED_HOME,
        EF_REPEATABLE,
        [0; 4],
        vec![
            ScriptStep {
                command: 37,
                datalong: [NEFARIAN, FAILED, 0, 0],
                ..Default::default()
            },
            ScriptStep {
                command: 4,
                datalong: [UNIT_NPC_FLAGS as u32, NPC_FLAG_GOSSIP, 1, 0],
                ..Default::default()
            },
            ScriptStep {
                command: 4,
                datalong: [UNIT_FIELD_FLAGS as u32, UNIT_FLAG_NOT_SELECTABLE, 2, 0],
                ..Default::default()
            },
        ],
    );
    // Nefarian, once he fights.
    let f = NPC_NEFARIAN;
    b.timer(f, 1, SHADOW_FLAME, At::Me, 12_000, (12_000, 12_000));
    b.timer(f, 2, BELLOWING_ROAR, At::Me, 30_000, (30_000, 30_000));
    b.timer(f, 3, VEIL_OF_SHADOW, At::Victim, 15_000, (15_000, 15_000));
    b.timer(f, 4, CLEAVE, At::Victim, 7000, (7000, 7000));
    b.timer(f, 5, TAIL_LASH, At::Me, 10_000, (10_000, 10_000));
    // Below 20 percent, the bones raised, once.
    b.event(
        f,
        6,
        EVENT_HP,
        EF_CHECK_RESULT,
        [20, 0, 0, 0],
        vec![cast_step(RAISE_DRAKONID, At::Me), say(SAY_RAISE_SKELETONS)],
    );
    b.event_chance(
        f,
        7,
        EVENT_KILL,
        EF_REPEATABLE,
        20,
        [0; 4],
        vec![say(SAY_SLAY)],
    );
    b.event(f, 8, EVENT_DEATH, 0, [0; 4], vec![say(SAY_DEATH)]);
    // A drakonid's death counted first, its own death's despawn after.
    for d in DRAKONIDS {
        b.event(
            d,
            91,
            EVENT_DEATH,
            EF_REPEATABLE,
            [0; 4],
            vec![ScriptStep {
                command: 37,
                datalong: [HOOK_DRAKONID_DIED, 1, 0, 0],
                ..Default::default()
            }],
        );
        if let Some(list) = b.data.scripts.ai_events.get_mut(&d) {
            list.rotate_right(1);
        }
    }
}
