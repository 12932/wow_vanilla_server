//! Round 9's world tests of who notices whom, ported from Thistle Tea's
//! `test/game/world/proximity_test.exs` and `presence_test.exs`: an
//! EventAI sight event within its radius and out of a fight, a runner
//! noticed once in a creature's reach and a still one once what hid them
//! changes, a stealthed trap hidden from its owner's opponent for a duel,
//! a running target's leeway, and the victim a pet takes up.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::MovementInfo_MovementFlags as Flags;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::scripts::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::{Creature, Motion};
use crate::game::map::duel::{self, Phase};
use crate::game::map::spells::{checks, fail, mods};
use crate::game::map::{Map, MapKey, ai, auras, combat, event_ai, grid, visibility};
use crate::game::tests_p3_cast_kit::{bare, damage, effect, fast_realm, human, insert};
use crate::game::tests_p3_combat::{self as base, PACK, START, aura_of, level, setup_with};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_spells_kit::{self as kit, at};

/// A creature with a sight event of twenty yards, and one noticing
/// within ten; both notice no one of their own accord but those.
const SEER: u32 = 997_400;
const WATCHER: u32 = 997_401;
/// The seer's sight event's script: its phase up by one.
const PHASE_UP: u32 = 997_402;
/// A stealthed trap.
const TRAP: u32 = 997_403;
/// A melee strike.
const STRIKE: u32 = 997_404;

fn data() -> GameData {
    let mut d = base::data();
    for (entry, detection) in [(SEER, 0.1), (WATCHER, 10.0)] {
        let mut t = base::template(entry, 7);
        t.faction = PACK;
        t.detection_range = detection;
        d.creature_templates.insert(entry, t);
    }
    d.scripts.ai_events.insert(
        SEER,
        vec![AiEvent {
            id: SEER,
            creature: SEER,
            kind: EVENT_OOC_LOS,
            chance: 100,
            flags: EF_REPEATABLE,
            params: [0, 20, 0, 0],
            actions: [PHASE_UP, 0, 0],
            ..Default::default()
        }],
    );
    d.scripts.ai_scripts.insert(
        PHASE_UP,
        vec![ScriptStep {
            id: PHASE_UP,
            command: 44,
            datalong: [1, 1, 0, 0],
            ..Default::default()
        }],
    );
    let mut trap = [0u32; 24];
    trap[9] = 1;
    d.gameobject_templates.insert(
        TRAP,
        crate::game::data::GameObjectTemplate {
            entry: TRAP,
            kind: 6,
            display_id: 1000,
            name: "Trap".into(),
            size: 1.0,
            data: trap,
            ..Default::default()
        },
    );
    d
}

/// `yards` east of the start.
fn east(yards: f32) -> Vector3d {
    Vector3d {
        x: START.0 + yards,
        y: START.1,
        z: START.2,
    }
}

/// A creature of `entry` at level 5, `yards` east of the start, facing
/// west, seen by those near.
fn spawn_at(m: &mut Map, entry: u32, yards: f32) -> Guid {
    let data = m.realm.data.clone();
    let t = data.creature_template(entry).cloned().unwrap();
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let c = Creature::new(
        guid::unit(entry, counter | 0x80_0000),
        &t,
        5,
        east(yards),
        std::f32::consts::PI,
    );
    let g = m.add_creature(c.with_base());
    visibility::refresh(m, g);
    g
}

/// Player `g` moved to `yards` east of the start, as walking there would.
fn put(m: &mut Map, g: Guid, yards: f32) {
    let at = east(yards);
    let p = m.players.get_mut(&g).unwrap();
    p.movement.position = at;
    p.move_check = None;
    let (anchor, old, new) = (p.vis_anchor, p.cell, grid::cell_of(at.x, at.y));
    p.cell = new;
    m.grid.relocate(g, old, new);
    visibility::moved(m, g, anchor, at);
}

fn attacking(m: &Map, c: Guid, g: Guid) -> bool {
    m.creature(c)
        .is_some_and(|c| c.combat.attacking && c.combat.victim == g)
}

/// A second of the map's clock, its creatures' turns every update.
fn a_second(m: &mut Map) {
    for _ in 0..20 {
        m.now += Duration::from_millis(50);
        ai::update_creatures(m);
    }
}

/// Thistle Tea's `hear/3` "a creature with an out-of-combat sight event
/// wakes for announcers in range": a creature with an out-of-combat sight
/// event of twenty yards fires it for Alice ten yards off, not thirty, and
/// not while it is in a fight (mangos's `EVENT_T_OOC_LOS`); as a game
/// master she goes unseen (wvs's, as mangos's `MoveInLineOfSight` skips
/// them).
#[tokio::test]
async fn a_sight_event_fires_within_its_radius_and_out_of_a_fight() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let phases = on(&map, move |m| {
        let seer = spawn_at(m, SEER, 10.0);
        event_ai::spawned(m, seer);
        let mut seen = Vec::new();
        let mut look = |m: &mut Map, gm: bool, yards: f32| {
            m.players.get_mut(&ga).unwrap().gm = gm;
            put(m, ga, 10.0 - yards);
            m.now += Duration::from_millis(1_100);
            event_ai::tick(m, seer);
            seen.push(event_ai::phase(m, seer));
        };
        look(m, true, 10.0);
        look(m, false, 30.0);
        look(m, false, 10.0);
        ai::attacked(m, seer, ga);
        look(m, false, 10.0);
        seen
    })
    .await;
    assert_eq!(phases, [0, 0, 1, 1], "a game master, thirty, ten, a fight");
}

/// Thistle Tea's `hear/3` "a walking announcer schedules one check for
/// the moment of contact": Alice running at seven yards a second toward a
/// creature noticing her within fourteen yards is noticed once within
/// them, not before, and by the next of its looks, every half second
/// (`AGGRO_SCAN_EVERY`; Thistle Tea's timer for the moment of contact is
/// its own way to the same).
#[tokio::test]
async fn a_runner_is_noticed_once_within_reach_and_not_before() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let (reach, noticed) = on(&map, move |m| {
        let watcher = spawn_at(m, WATCHER, 30.0);
        let reach = ai::attack_distance(m, watcher, ga);
        let mut x = 0.0;
        while x < 30.0 {
            x += 0.35;
            put(m, ga, x);
            m.now += Duration::from_millis(50);
            ai::update_creatures(m);
            if attacking(m, watcher, ga) {
                return (reach, Some(30.0 - x));
            }
        }
        (reach, None)
    })
    .await;
    assert_eq!(reach, 14.0);
    let d = noticed.expect("noticed");
    assert!(d <= reach && d >= reach - 3.5, "at {d} yards");
}

/// Thistle Tea's `sync/2` "announces stationary changes to every
/// detection fact" and "announces a unit when it moves two yards or
/// changes how others react to it": a creature looks again at Alice
/// standing still: stealthed beside it she goes unnoticed, and is noticed
/// once her stealth is gone; ten levels up she is out of its reach at
/// eight yards, and back at the first level, in it, noticed where she
/// stands. (Thistle Tea announces a unit every two yards it moves; wvs's
/// creature looks every half second, wherever all are.)
#[tokio::test]
async fn one_standing_still_is_noticed_once_what_hid_them_changes() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Rogue).await;
    let r = on(&map, move |m| {
        let watcher = spawn_at(m, WATCHER, 5.0);
        aura_of(m, ga, 997_410, auras::AURA_MOD_STEALTH, 0, 500);
        a_second(m);
        let stealthed = attacking(m, watcher, ga);
        auras::remove_spell(m, ga, 997_410, None);
        a_second(m);
        let seen = attacking(m, watcher, ga);
        m.remove_creature(watcher);
        combat::leave_combat(m, ga);

        level(m, ga, 10);
        let watcher = spawn_at(m, WATCHER, 8.0);
        let reach = ai::attack_distance(m, watcher, ga);
        a_second(m);
        let beyond = attacking(m, watcher, ga);
        level(m, ga, 1);
        a_second(m);
        (stealthed, seen, reach, beyond, attacking(m, watcher, ga))
    })
    .await;
    assert_eq!(r, (false, true, 5.0, false, true));
}

/// Thistle Tea's `sync/2` "a reaction change tells the traps the unit
/// owns": Alice's stealthed trap, which Bob of her group sees, is hidden
/// from him as their duel's fight begins, and shown again as it ends
/// (mangos's `GameObject::isVisibleForInState`, a player's stealthed trap
/// unseen by one in a duel with its owner; wvs works out the duelists'
/// visibility again at both).
#[tokio::test]
async fn a_duel_hides_the_owners_stealthed_trap_from_the_other_side() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0 + 3.0, START.1, START.2).await;
    let (la, lb) = (
        realm.registry.online(ga).unwrap(),
        realm.registry.online(gb).unwrap(),
    );
    assert!(crate::game::social::groups::join_directly(&realm, &la, &lb));
    let map = realm.maps.get(&realm, MapKey::world(0));
    let trap = on(&map, move |m| {
        crate::game::map::objects::summon(m, Some(ga), TRAP, east(1.0), 0.0, 0, 0, 0, None).unwrap()
    })
    .await;
    b.sees(trap).await;
    // A few updates: what the group's joining has Bob look at again is
    // looked at before the duel.
    for _ in 0..3 {
        harness::an_update(&map).await;
    }
    on(&map, move |m| {
        duel::challenge(m, ga, gb);
        duel::accept(m, gb);
        let now = m.now;
        for g in [ga, gb] {
            if let Some(d) = m.players.get_mut(&g).and_then(|p| p.duel.as_mut()) {
                d.phase = Phase::Countdown { until: now };
            }
        }
    })
    .await;
    b.forgets(trap).await;
    assert!(on(&map, move |m| duel::dueling(m, ga, gb)).await);
    on(&map, move |m| duel::cancel(m, gb)).await;
    b.sees(trap).await;
}

/// `presence_test.exs`'s "spell snapshots follow current translation
/// speed and clear it when movement stops", what that speed is for: a
/// melee strike a yard and a half past its reach lands while Alice and its
/// target both run, and not once either stops, nor while she walks
/// (mangos's `GetMinMaxRange`, `MELEE_LEEWAY` for two moving forward and
/// not walking). Slowed to half a run she has it still: mangos's rule is
/// the movement flags, where Thistle Tea's is a speed over 4.97 yards a
/// second.
#[tokio::test]
async fn a_strike_reaches_further_while_both_run_and_not_once_one_stops() {
    let realm = fast_realm(|d| {
        let mut s = bare(STRIKE, 1);
        s.dmg_class = 2;
        s.range_index = 2;
        effect(&mut s, 0, damage(10));
        insert(d, [s]);
    });
    let (_c, a, map) = human(&realm, "P9WS1", "Striker", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[STRIKE]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let (me, it) = (m.unit_view(a).unwrap(), m.unit_view(kobold).unwrap());
            let d = combat::melee_range(&me, &it) + 1.0 + 1.5;
            let (from, o) = (me.position, me.orientation);
            let ahead = |d: f32| Vector3d {
                x: from.x + d * o.cos(),
                y: from.y + d * o.sin(),
                z: from.z,
            };
            m.creature_mut(kobold).unwrap().position = ahead(d);
            let why = |m: &mut Map| {
                kit::fresh(m, a);
                let s = m.realm.data.spells.get(STRIKE).cloned().unwrap();
                let mods = mods::for_spell(m, a, &s);
                checks::check(m, a, &s, &at(kobold), None, false, &mods)
            };
            let still = why(m);
            let (now, id) = (m.now, m.next_spline_id);
            m.creature_mut(kobold).unwrap().walk_at(
                vec![ahead(d), ahead(d + 20.0)],
                true,
                false,
                None,
                now,
                id,
            );
            m.players.get_mut(&a).unwrap().movement.flags = Flags::new_forward();
            let running = why(m);
            m.players.get_mut(&a).unwrap().movement.flags = Flags::new_forward().set_walk_mode();
            let walking = why(m);
            m.players.get_mut(&a).unwrap().movement.flags = Flags::new_forward();
            m.players.get_mut(&a).unwrap().speeds.run = 3.5;
            let slowed = why(m);
            m.creature_mut(kobold).unwrap().motion = Motion::Idle { until: None };
            let stopped = why(m);
            [still, running, walking, slowed, stopped]
        })
        .await
        .unwrap();
    let far = Some(fail::OUT_OF_RANGE);
    assert_eq!(out, [far, None, far, None, far]);
}

/// `presence_test.exs`'s "publishes actual attack targets separately from
/// UI selection and releases combat references", what that is for: a
/// hunter's pet takes up the wolf she swings at, not the one she has
/// selected, though that one is nearer and fighting her (mangos's
/// `getAttackerForHelper`, her victim before her attackers; wvs's pet
/// reads her swing, never her selection).
#[tokio::test]
async fn a_pet_takes_up_what_its_master_swings_at_not_what_she_selects() {
    use crate::game::tests_pets::{WOLF, place, tamed};
    let (_realm, _h, gh, map, pet, _wolf) = tamed().await;
    let swung = place(&map, gh, WOLF, 0x70_0020, 9.0).await;
    let selected = place(&map, gh, WOLF, 0x70_0021, 3.0).await;
    let victim = map
        .query(move |m| {
            combat::start_attack(m, gh, swung);
            combat::engage(m, gh, swung);
            combat::start_attack(m, selected, gh);
            combat::engage(m, selected, gh);
            m.players
                .get_mut(&gh)
                .unwrap()
                .fields
                .set_guid(UNIT_FIELD_TARGET, selected);
            m.now += Duration::from_secs(1);
            crate::game::map::pets::update(m);
            m.combat(pet).filter(|c| c.attacking).map(|c| c.victim)
        })
        .await
        .unwrap();
    assert_eq!(victim, Some(swung));
}
