//! Razorgore the Untamed, as mangos-classic's `boss_razorgore` and
//! `instance_blackwing_lair` run him: Grethok the Controller's pull starts
//! the fight, and forty seconds on, then every fifteen, four of the lair's
//! defenders come from the hall's corners (Blackwing Legionnaires and Mages
//! to forty, Death Talon Dragonspawn to twelve). Grethok dead, the Orb of
//! Domination may be used: its user takes Razorgore over (Possess, and
//! Mind Exhaustion on the user), and Razorgore under control breaks the
//! black dragon eggs. With every egg broken the orb goes dark, the
//! defenders flee, and Razorgore, free and healed, fights the raid to the
//! death. Before that he can't be killed: brought to death, or out of his
//! fight with no one left, the encounter fails (he takes the raid with
//! him), and thirty seconds on all is as it was.

// Places as vmangos has them, to the digit.
#![allow(clippy::excessive_precision)]

use std::time::Duration;

use rand::RngExt;
use rand::seq::SliceRandom;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use super::*;
use crate::game::map::instance_adapters::{Act, operate};
use crate::game::map::instance_script::{cancel, schedule};

pub const NPC_GRETHOK: u32 = 12557;
pub const NPC_GUARDSMAN: u32 = 14456;
pub const NPC_LEGIONNAIRE: u32 = 12416;
pub const NPC_MAGE: u32 = 12420;
pub const NPC_DRAGONSPAWN: u32 = 12422;
pub const GO_ORB: u32 = 177808;
pub const GO_EGG: u32 = 177807;

/// The orb's own spell, cast on its user (its goober spell).
pub const DRAGON_ORB: u32 = 19869;
pub const POSSESS: u32 = 19832;
pub const POSSESS_VISUAL: u32 = 23014;
pub const DRAGON_ORB_AURA: u32 = 23021;
pub const MIND_EXHAUSTION: u32 = 23958;
pub const DESTROY_EGG: u32 = 19873;
pub const WARMING_FLAMES: u32 = 23040;
pub const FIREBALL: u32 = 23024;
pub const EXPLOSION: u32 = 20038;
const DOUBLE_ATTACK: u32 = 18943;

const SAY_EGGS_BROKEN: [u32; 3] = [9961, 9962, 9963];
pub const SAY_DEATH: u32 = 9591;
const EMOTE_TROOPS_FLEE: u32 = 9592;

/// At most this many of each defender at once.
pub const MAX_DRAGONSPAWN: u32 = 12;
pub const MAX_ORCS: u32 = 40;
/// The defenders come from the hall's four corners (vmangos's points:
/// mangos's world has its generators there).
const GENERATORS: [[f32; 3]; 4] = [
    [-7530.4072, -1062.4177, 407.2],
    [-7659.7128, -1043.3569, 407.2],
    [-7607.4643, -1116.4702, 407.2],
    [-7584.3247, -990.5787, 407.2],
];

// Timers.
pub const T_DEFENSE: u32 = TIMERS + 6100;
pub const T_RESET: u32 = TIMERS + 6101;
const T_PREVENTED: u32 = TIMERS + 6102;
const T_WATCH: u32 = TIMERS + 6103;
// What the copy keeps meanwhile.
pub const DRAGONSPAWN_COUNT: u32 = 6100;
pub const ORC_COUNT: u32 = 6101;
const DEFENDERS: u32 = 6100;
pub const USED_EGGS: u32 = 6100;

fn scratch(map: &Map, key: u32) -> u32 {
    map.instance.scratch.get(&key).copied().unwrap_or(0)
}

fn set_scratch(map: &mut Map, key: u32, value: u32) {
    map.instance.scratch.insert(key, value);
}

fn first_of(map: &Map, entry: u32) -> Option<Guid> {
    map.creatures
        .iter()
        .filter(|(_, c)| c.entry == entry)
        .map(|(_, c)| (c.spawn_guid.unwrap_or(u32::MAX), c.guid))
        .min()
        .map(|(_, g)| g)
}

fn razorgore(map: &Map) -> Option<Guid> {
    first_of(map, NPC_RAZORGORE)
}

fn eggs(map: &Map) -> Vec<Guid> {
    map.gameobjects
        .iter()
        .filter(|(_, o)| o.entry == GO_EGG)
        .map(|(_, o)| o.guid)
        .collect()
}

/// How many eggs the copy has, broken or not.
fn egg_spawns(map: &Map) -> usize {
    map.realm
        .data
        .gameobject_spawns
        .get(&map.key.map)
        .map_or(0, |v| v.iter().filter(|s| s.entry == GO_EGG).count())
}

/// Razorgore can't die but in his last stage (mangos's `SetDeathPrevention`).
fn death_prevention(map: &mut Map, g: Guid, on: bool) {
    if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
        s.invincible_hp = on.then_some(1);
    }
}

/// A creature's event in Razorgore's hall.
pub fn creature_event(map: &mut Map, g: Guid, entry: u32, event: Event) {
    match (entry, event) {
        (NPC_GRETHOK, Event::Aggro) => {
            set_data(map, RAZORGORE, IN_PROGRESS);
            schedule(map, T_DEFENSE, Duration::from_secs(40));
        }
        // The orb may be used.
        (NPC_GRETHOK, Event::Death) => {
            if !matches!(get_data(map, RAZORGORE), DONE | SPECIAL) {
                operate(map, GO_ORB, Act::Active);
            }
        }
        (NPC_RAZORGORE, Event::Spawned) => {
            death_prevention(map, g, get_data(map, RAZORGORE) != SPECIAL);
            if !crate::game::map::auras::auras(map, g).is_some_and(|a| a.has_spell(DOUBLE_ATTACK)) {
                crate::game::map::spells::cast_triggered(map, g, DOUBLE_ATTACK, Some(g));
            }
        }
        (NPC_RAZORGORE, Event::Aggro) => {
            crate::game::map::auras::remove_spell(map, g, POSSESS_VISUAL, None);
        }
        (NPC_RAZORGORE, Event::Death) if get_data(map, RAZORGORE) == SPECIAL => {
            set_data(map, RAZORGORE, DONE);
        }
        (NPC_LEGIONNAIRE | NPC_MAGE, Event::Death) if defender(map, g) => {
            let n = scratch(map, ORC_COUNT).saturating_sub(1);
            set_scratch(map, ORC_COUNT, n);
        }
        (NPC_DRAGONSPAWN, Event::Death) if defender(map, g) => {
            let n = scratch(map, DRAGONSPAWN_COUNT).saturating_sub(1);
            set_scratch(map, DRAGONSPAWN_COUNT, n);
        }
        _ => {}
    }
}

fn defender(map: &Map, g: Guid) -> bool {
    map.instance
        .lists
        .get(&DEFENDERS)
        .is_some_and(|l| l.contains(&g))
}

/// A blow that would have killed Razorgore while he can't die: the
/// encounter fails (mangos's `JustPreventedDeath`), once the blow is done.
pub fn death_prevented(map: &mut Map, g: Guid) {
    if map.creature(g).is_some_and(|c| c.entry == NPC_RAZORGORE)
        && get_data(map, RAZORGORE) == IN_PROGRESS
    {
        schedule(map, T_PREVENTED, Duration::ZERO);
    }
}

/// What a change of Razorgore's field brings.
pub fn changed(map: &mut Map, new: u32) {
    if new != FAILED {
        return;
    }
    cancel(map, T_DEFENSE);
    // He takes the raid with him: Fireball into the orb, and its
    // explosion through the hall.
    if let Some(r) = razorgore(map).filter(|r| map.creature(*r).is_some_and(|c| c.alive())) {
        scripts::talk(map, r, SAY_DEATH, Guid::zero(), None);
        crate::game::map::spells::cast_triggered(map, r, FIREBALL, None);
        let s = ScriptStep {
            command: 18,
            datalong: [5000, 0, 0, 0],
            ..Default::default()
        };
        scripts::execute(map, &s, r, r);
    }
    let orb = map
        .gameobjects
        .iter()
        .find(|(_, o)| o.entry == GO_ORB)
        .map(|(_, o)| o.position);
    if let Some(at) = orb {
        let doomed: Vec<Guid> = map
            .players
            .values()
            .filter(|p| p.death == crate::game::map::death::Death::Alive)
            .filter(|p| crate::game::map::combat::dist(p.position(), at) <= 100.0)
            .map(|p| p.guid)
            .collect();
        for p in doomed {
            crate::game::map::spells::cast_triggered(map, p, EXPLOSION, Some(p));
        }
    }
    operate(map, GO_ORB, Act::Inert);
    let gone = map.instance.lists.remove(&DEFENDERS).unwrap_or_default();
    for d in gone {
        if map.creature(d).is_some() {
            scripts::despawn(map, d);
        }
    }
    set_scratch(map, DRAGONSPAWN_COUNT, 0);
    set_scratch(map, ORC_COUNT, 0);
    schedule(map, T_RESET, Duration::from_secs(30));
}

/// Whether a player not a GM is alive in the copy.
fn anyone_alive(map: &Map) -> bool {
    map.players
        .values()
        .any(|p| !p.gm && p.death == crate::game::map::death::Death::Alive)
}

/// The copy's timers for Razorgore's fight; whether the key was one.
pub fn timer(map: &mut Map, key: u32) -> bool {
    match key {
        T_DEFENSE => {
            // No one left alive to fight (a wipe before Razorgore is ever
            // fought, with no evade of his to fail it): failed, as mangos's
            // leash for him has it (no living player not a GM in the map).
            if get_data(map, RAZORGORE) == IN_PROGRESS && !anyone_alive(map) {
                set_data(map, RAZORGORE, FAILED);
            } else if get_data(map, RAZORGORE) == IN_PROGRESS {
                send_defenders(map);
                schedule(map, T_DEFENSE, Duration::from_secs(15));
            }
        }
        T_PREVENTED => {
            if get_data(map, RAZORGORE) == IN_PROGRESS {
                set_data(map, RAZORGORE, FAILED);
            }
        }
        T_RESET => reset(map),
        _ => return false,
    }
    true
}

/// Thirty seconds after a failed try: Razorgore and his eggs back, and
/// Grethok and his guards to start it again (vmangos's
/// `SituationInitiale`: mangos's world brings them back by their own
/// respawn).
fn reset(map: &mut Map) {
    let now = map.now;
    let used: Vec<u32> = map.instance.seen.remove(&USED_EGGS).unwrap_or_default();
    for (at, spawn) in map.objects.respawns.iter_mut() {
        if used.contains(spawn) {
            *at = now;
        }
    }
    for entry in [NPC_RAZORGORE, NPC_GRETHOK, NPC_GUARDSMAN] {
        let all: Vec<Guid> = map
            .creatures
            .iter()
            .filter(|(_, c)| c.entry == entry && c.spawn_guid.is_some())
            .map(|(_, c)| c.guid)
            .collect();
        for g in all {
            if map.creature(g).is_some_and(|c| !c.alive()) {
                crate::game::map::ai::respawn_now(map, g);
            }
        }
    }
}

/// Four defenders, each from a corner: a dragonspawn half the time while
/// there are fewer than twelve, else a legionnaire or a mage while there
/// are fewer than forty (mangos's `m_uiDefenseTimer`).
fn send_defenders(map: &mut Map) {
    let mut rng = rand::rng();
    let mut corners = GENERATORS;
    corners.shuffle(&mut rng);
    for at in corners {
        let dragons = scratch(map, DRAGONSPAWN_COUNT);
        let orcs = scratch(map, ORC_COUNT);
        let most = if dragons < MAX_DRAGONSPAWN { 3 } else { 1 };
        let entry = match rng.random_range(0..=most) {
            0 if orcs < MAX_ORCS => NPC_LEGIONNAIRE,
            1 if orcs < MAX_ORCS => NPC_MAGE,
            2 | 3 => NPC_DRAGONSPAWN,
            _ => continue,
        };
        let at = Vector3d {
            x: at[0],
            y: at[1],
            z: at[2],
        };
        // Its corpse goes five seconds after its death.
        let Some(g) = scripts::summon(
            map,
            Guid::zero(),
            entry,
            at,
            0.0,
            Some((Duration::from_secs(5), 6)),
        ) else {
            continue;
        };
        if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
            s.summoner = None;
        }
        if entry == NPC_DRAGONSPAWN {
            set_scratch(map, DRAGONSPAWN_COUNT, dragons + 1);
        } else {
            set_scratch(map, ORC_COUNT, orcs + 1);
        }
        map.instance.lists.entry(DEFENDERS).or_default().push(g);
        // It fights everyone in the hall, Razorgore among them.
        let s = ScriptStep {
            command: 49,
            datalong: [1, 0, 0, 0],
            ..Default::default()
        };
        scripts::execute(map, &s, g, g);
    }
}

/// The orb used: its user takes Razorgore over, unless exhausted from the
/// last time.
pub fn orb_used(map: &mut Map, player: Guid) {
    if !map.players.contains_key(&player)
        || matches!(get_data(map, RAZORGORE), DONE | SPECIAL)
        || crate::game::map::auras::auras(map, player).is_some_and(|a| a.has_spell(MIND_EXHAUSTION))
    {
        return;
    }
    let Some(r) = razorgore(map).filter(|r| map.creature(*r).is_some_and(|c| c.alive())) else {
        return;
    };
    crate::game::map::spells::cast_triggered(map, player, POSSESS, Some(r));
    let held = map.creature(r).and_then(|c| c.owner) == Some(player);
    if !held {
        return;
    }
    // Possessed (mangos's `PossessRazorgore`): the user exhausted, the
    // dragon shrunk and marked.
    crate::game::map::spells::cast_triggered(map, player, MIND_EXHAUSTION, Some(player));
    crate::game::map::spells::cast_triggered(map, r, DRAGON_ORB_AURA, Some(r));
    crate::game::map::spells::cast_triggered(map, r, POSSESS_VISUAL, Some(r));
    schedule_for(map, T_WATCH, r, Duration::from_secs(1));
}

/// Watching Razorgore under control: once he's his own again, the orb's
/// marks go.
pub fn creature_timer(map: &mut Map, key: u32, g: Guid) -> bool {
    if key != T_WATCH {
        return false;
    }
    let held = map.creature(g).is_some_and(|c| c.owner.is_some());
    if held {
        schedule_for(map, T_WATCH, g, Duration::from_secs(1));
    } else {
        crate::game::map::auras::remove_spell(map, g, POSSESS_VISUAL, None);
        crate::game::map::auras::remove_spell(map, g, DRAGON_ORB_AURA, None);
    }
    true
}

/// An egg Destroy Egg broke (mangos's `DestroyEgg`): sometimes he says
/// so; it goes until the encounter is tried again; with the last, the
/// second stage.
pub fn egg_broken(map: &mut Map, caster: Guid, egg: Guid, spell: u32) -> bool {
    if spell != DESTROY_EGG {
        return false;
    }
    let Some(spawn) = map
        .gameobject(egg)
        .filter(|o| o.entry == GO_EGG)
        .map(|o| o.spawn_guid)
    else {
        return false;
    };
    let mut rng = rand::rng();
    if rng.random_range(0..=1) == 1 {
        let t = SAY_EGGS_BROKEN[rng.random_range(0..SAY_EGGS_BROKEN.len())];
        scripts::talk(map, caster, t, Guid::zero(), None);
    }
    crate::game::map::objects::despawn_then(map, egg, Some(Duration::from_secs(7 * 86_400)));
    let used = map.instance.seen.entry(USED_EGGS).or_default();
    if let Some(s) = spawn
        && !used.contains(&s)
    {
        used.push(s);
    }
    let broken = map.instance.seen.get(&USED_EGGS).map_or(0, Vec::len);
    if broken >= egg_spawns(map) && eggs(map).is_empty() {
        second_stage(map);
    }
    true
}

/// Every egg broken: the orb goes dark, Razorgore is free and healed and
/// fights to the death, and his defenders flee.
fn second_stage(map: &mut Map) {
    set_data(map, RAZORGORE, SPECIAL);
    cancel(map, T_DEFENSE);
    operate(map, GO_ORB, Act::Inert);
    if let Some(r) = razorgore(map) {
        crate::game::map::auras::remove_spell(map, r, POSSESS, None);
        death_prevention(map, r, false);
        crate::game::map::auras::remove_spell(map, r, POSSESS_VISUAL, None);
        crate::game::map::auras::remove_spell(map, r, DRAGON_ORB_AURA, None);
        crate::game::map::spells::cast_triggered(map, r, WARMING_FLAMES, Some(r));
        let s = ScriptStep {
            command: 49,
            datalong: [1, 0, 0, 0],
            ..Default::default()
        };
        scripts::execute(map, &s, r, r);
    }
    let defenders = map.instance.lists.remove(&DEFENDERS).unwrap_or_default();
    let mut told = false;
    for d in defenders {
        if !map.creature(d).is_some_and(|c| c.alive()) {
            continue;
        }
        if !told {
            scripts::talk(map, d, EMOTE_TROOPS_FLEE, Guid::zero(), None);
            told = true;
        }
        let evade = ScriptStep {
            command: 33,
            ..Default::default()
        };
        scripts::execute(map, &evade, d, d);
        let go = ScriptStep {
            command: 18,
            datalong: [10_000, 0, 0, 0],
            ..Default::default()
        };
        scripts::execute(map, &go, d, d);
    }
    set_scratch(map, DRAGONSPAWN_COUNT, 0);
    set_scratch(map, ORC_COUNT, 0);
}

/// As the copy loads: the orb is dark but once Grethok is dead and the
/// eggs still stand.
pub fn settle(map: &mut Map) {
    let grethok_dead =
        first_of(map, NPC_GRETHOK).is_none_or(|g| map.creature(g).is_some_and(|c| !c.alive()));
    if !grethok_dead || matches!(get_data(map, RAZORGORE), DONE | SPECIAL) {
        operate(map, GO_ORB, Act::Inert);
    }
}

/// Razorgore's timed casts in his last stage (mangos's `CombatAI`
/// actions), and his rest.
pub(super) fn install(b: &mut Build) {
    b.data.scripts.ai_events.remove(&NPC_RAZORGORE);
    b.timer_between(
        NPC_RAZORGORE,
        1,
        23023,
        At::Me,
        (10000, 15000),
        (15000, 25000),
    );
    b.timer_between(
        NPC_RAZORGORE,
        2,
        22425,
        At::Me,
        (15000, 20000),
        (15000, 20000),
    );
    b.timer(NPC_RAZORGORE, 3, 24375, At::Me, 30000, (30000, 30000));
    b.timer_between(
        NPC_RAZORGORE,
        4,
        19632,
        At::Victim,
        (4000, 8000),
        (4000, 8000),
    );
}
