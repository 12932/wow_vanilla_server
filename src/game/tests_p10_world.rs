//! Round 10's world tests, of what wvs did partly or otherwise: Improved
//! Feign Death's ranks on the real data (`feign_death_test.exs`), a flyer
//! on a map with no navmesh and a walk's stop off the navmesh
//! (`pathfinding_unloaded_test.exs`, `pathfinding_test.exs`), a map
//! event's failure script (`scripted_event_test.exs`), a battleground's
//! corpse reclaimed only once the battle is on (`battleground_test.exs`)
//! and the events the world drives (`game_event_test.exs`).

use std::time::Duration;

use ahash::AHashMap;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::Condition;
use crate::game::data::{GameData, GameEvent};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::battleground::Phase;
use crate::game::map::creature_motion::route;
use crate::game::map::death::Death;
use crate::game::map::scripts::execute;
use crate::game::map::{Map, MapKey, combat, map_events};
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};
use crate::game::tests_p3_entity::{on, step};
use crate::game::tests_p3_spells_kit as kit;

// Feign Death's ranks.

const FEIGN_DEATH: u32 = 5384;
const IMPROVED_FEIGN_DEATH: [u32; 2] = [19_286, 19_287];

/// Thistle Tea's `FeignDeath.prepare/5` "uses the real Feign Death and
/// Improved Feign Death ranks": a level 50 hunter's Feign Death lands on
/// a level 50 creature 96 times in a hundred (a roll of 97 sees through
/// it), with Improved Feign Death's first rank 98 (97 doesn't, 98.5 does)
/// and its second 99, the cap (98.5 doesn't, 99.99 does): each rank takes
/// 2% off the chance it is resisted, through its spells' hit modifier
/// (mangos-classic's `HandleFeignDeath`, the attackers' miss chance of
/// the aura's spell).
#[tokio::test]
#[ignore = "extended"]
async fn improved_feign_deaths_ranks_take_two_percent_each_off_its_resistance() {
    use crate::game::map::spells::{mods, outcome};
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (_c, h, map) = kit::player(&realm, "P10WF", "Feigner", Race::Dwarf, Class::Hunter).await;
    let chances = map
        .query(move |m| {
            kit::ready(m, h, 50, &[FEIGN_DEATH]);
            let boar = kit::dummy(m, h, 50, 5.0);
            let chance = |m: &Map| {
                let s = m.realm.data.spells.get(FEIGN_DEATH).unwrap().clone();
                let mods = mods::for_spell(m, h, &s);
                outcome::spell_hit_chance(m, h, boar, &s, &mods)
            };
            let mut out = vec![chance(m)];
            for rank in IMPROVED_FEIGN_DEATH {
                crate::game::map::spells::learn(m, h, rank);
                out.push(chance(m));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(chances, vec![9600, 9800, 9900], "none, one rank, two");
}

// The navmesh.

const FLYER: u32 = 997_300;
const WALKER: u32 = 997_301;

/// Thistle Tea's `find_path/4` "flies straight too" and "walks straight
/// to the destination on a map without a navigation mesh": with no
/// navmesh a flyer's way to a point in the air is that point at once, all
/// the way and flown; a walker's is straight too, walked.
#[tokio::test]
async fn with_no_navmesh_a_flyer_flies_straight_and_a_walker_walks_so() {
    use crate::game::tests_client_emu::{START, spawn as spawned, template};
    let mut d = GameData::empty();
    for (entry, inhabit) in [(FLYER, 4), (WALKER, 1)] {
        let mut t = template(entry, "Made-up", 7);
        t.inhabit_type = inhabit;
        d.creature_templates.insert(entry, t);
    }
    d.creature_spawns.insert(
        0,
        vec![
            spawned(1, FLYER, (START.0 + 3.0, START.1, START.2)),
            spawned(2, WALKER, (START.0 - 3.0, START.1, START.2)),
        ],
    );
    let realm = harness::realm_with(d);
    let map = realm.maps.get(&realm, MapKey::world(0));
    let from = Vector3d {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    let (air, ground) = (
        Vector3d {
            x: 5.0,
            y: 6.0,
            z: 9.0,
        },
        Vector3d {
            x: 5.0,
            y: 6.0,
            z: 1.0,
        },
    );
    let ways = map
        .query(move |m| {
            let of = |entry: u32| {
                m.creatures
                    .iter()
                    .find(|(_, c)| c.entry == entry)
                    .map(|(_, c)| c.guid)
                    .unwrap()
            };
            let (flyer, walker) = (of(FLYER), of(WALKER));
            (
                m.nav.is_none(),
                route(m, flyer, from, air),
                route(m, walker, from, ground),
            )
        })
        .await
        .unwrap();
    let (no_nav, flight, walk) = ways;
    assert!(no_nav, "no navmesh");
    assert_eq!(
        flight,
        Some((vec![from, air], true, true)),
        "flown straight"
    );
    assert_eq!(walk, Some((vec![from, ground], true, false)), "walked so");
}

static BAKE: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("nav"));

/// Thistle Tea's `walk_hit_position/3` "rejects missing mesh and
/// positions far above a floor": a walk over the navmesh from fifty yards
/// over Northshire's ground finds no place to stop, where one from the
/// ground stops on it; a map with no bake has no navmesh to walk.
#[tokio::test]
#[ignore = "extended"]
async fn a_walk_from_far_over_the_floor_stops_nowhere() {
    use crate::game::map::nav::Nav;
    if !BAKE.join("Azeroth.map").exists() {
        return;
    }
    assert!(
        Nav::open(&BAKE, "NoSuchMap").is_none(),
        "no bake, no navmesh"
    );
    let mut nav = Nav::open(&BAKE, "Azeroth").expect("Azeroth");
    let v = |x: f32, y: f32, z: f32| Vector3d { x, y, z };
    let ground = nav.walk_hit(v(-8949.95, -132.49, 83.53), v(-8969.95, -132.49, 83.53));
    let high = nav.walk_hit(v(-8949.95, -132.49, 133.53), v(-8969.95, -132.49, 133.53));
    let ground = ground.expect("a walk on the ground");
    assert!((ground.x + 8969.95).abs() < 0.5, "{ground:?}");
    assert!((ground.z - 83.53).abs() < 2.0, "{ground:?}");
    assert_eq!(high, None, "nothing from fifty yards up");
}

/// Thistle Tea's `get_zone_and_area/2` "rejects unsigned unknown-area
/// sentinels on Programmer Isle": an area of 0 or the unsigned sentinel
/// names none, another is the bake's (wvs's bake labels the isle's
/// sentinel points as its own area, so the filter is shown on the values;
/// `tests_p7_world_nav::zones_and_areas_come_from_the_nearest_labelled_floor`
/// has the isle's labelled point).
#[test]
fn an_unknown_areas_sentinel_names_no_area() {
    use crate::game::map::nav::known;
    assert_eq!(known(Some((22, u32::MAX))), None, "the sentinel");
    assert_eq!(known(Some((22, 0))), None, "no area");
    assert_eq!(known(Some((22, 22))), Some((22, 22)), "the isle's");
    assert_eq!(known(None), None);
}

// A map event's failure.

const ESCORT_DEAD: u32 = 98_200;
const LOST: u32 = 98_201;
const EVENT: u32 = 648;

fn escort_world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.scripts.generic.insert(
        LOST,
        vec![step(2, [u32::from(UNIT_NPC_EMOTESTATE), 10, 0, 0])],
    );
    d.scripts.conditions.insert(
        ESCORT_DEAD,
        Condition {
            entry: ESCORT_DEAD,
            kind: 25,
            values: [1, 0, 0, 0],
            flags: 0,
        },
    );
    d
}

/// Thistle Tea's `ScriptedEvent` "escort failure conditions run the
/// configured failure script": a map event begun by an escort with its
/// failure "the escort's source dead" goes on while it lives; once it is
/// dead the event's next check fails it, running its failure script (which
/// marks its source, emote state 10), and it is over (vmangos's
/// `ScriptedEvent::UpdateEvent` and `EndEvent`).
#[tokio::test]
async fn a_map_events_escort_dead_fails_it_and_runs_its_failure_script() {
    let (_realm, _a, ga, map) = setup_with(escort_world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let escort = spawn(m, ga, BRUTE, 20, 5.0);
        let start = crate::game::data::scripts::ScriptStep {
            dataint: [0, 0, ESCORT_DEAD as i32, LOST as i32],
            ..step(61, [EVENT, 600, 0, 0])
        };
        assert!(execute(m, &start, escort, ga));
        let marked = |m: &Map| {
            m.unit_fields(escort)
                .is_some_and(|f| f.get_u32(UNIT_NPC_EMOTESTATE) == 10)
        };
        m.now += Duration::from_secs(1);
        map_events::update(m);
        let living = (map_events::active(m, EVENT), marked(m));
        combat::kill(m, ga, escort);
        m.now += Duration::from_secs(1);
        map_events::update(m);
        (living, (map_events::active(m, EVENT), marked(m)))
    })
    .await;
    assert_eq!(out.0, (true, false), "on while the escort lives");
    assert_eq!(out.1, (false, true), "failed, its script run");
}

// A battleground's corpse.

/// A packet the map answers at once, after whatever `c` sent before it.
async fn fence(c: &mut Client) {
    c.send(MSG_LOOKING_FOR_GROUP_Client {}).await;
    c.wait_for("MSG_LOOKING_FOR_GROUP", |m| {
        matches!(m, ServerOpcodeMessage::MSG_LOOKING_FOR_GROUP(_)).then_some(())
    })
    .await;
}

/// Thistle Tea's `corpse_recovery_allowed?/3` "permits only admitted
/// players after preparation ends": a ghost by their corpse in Warsong
/// Gulch can't take it up while the battle hasn't begun; once it is on,
/// they rise whole (mangos's `HandleReclaimCorpseOpcode`, a battleground's
/// whole).
#[tokio::test]
async fn a_battlegrounds_corpse_is_taken_up_only_once_the_battle_is_on() {
    let mut d = crate::game::tests_p3_combat::data();
    super::tests_pvp::battleground(&mut d, 489, 2, "WarsongGulch", [900, 901]);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = super::tests_pvp::both_into(
        &realm,
        &mut a,
        &mut b,
        wow_world_messages::vanilla::Map::WarsongGulch,
    )
    .await;
    map.query(move |m| crate::game::map::death::kill(m, ga))
        .await
        .unwrap();
    a.send(CMSG_REPOP_REQUEST {}).await;
    a.wait_for("the teleport to the graveyard", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: ga,
        movement_counter: a.teleport_order(),
        time: Default::default(),
    })
    .await;
    fence(&mut a).await;
    // By the corpse (released where they fell, with no graveyards), the
    // delay waited out, the battle not begun.
    map.query(move |m| {
        let since = m.now - Duration::from_secs(60);
        m.players.get_mut(&ga).unwrap().death = Death::Ghost { since };
        m.battleground.as_mut().unwrap().phase = Phase::Waiting;
    })
    .await
    .unwrap();
    let reclaim = CMSG_RECLAIM_CORPSE {
        guid: crate::game::guid::corpse(crate::game::guid::counter(ga)),
    };
    a.send(reclaim).await;
    fence(&mut a).await;
    let waiting = map
        .query(move |m| m.players[&ga].death == Death::Alive)
        .await
        .unwrap();
    map.query(|m| m.battleground.as_mut().unwrap().phase = Phase::Playing)
        .await
        .unwrap();
    a.send(reclaim).await;
    fence(&mut a).await;
    let (alive, health, max) = map
        .query(move |m| {
            let f = &m.players[&ga].fields;
            (
                m.players[&ga].death == Death::Alive,
                f.get_u32(UNIT_FIELD_HEALTH),
                f.get_u32(UNIT_FIELD_MAXHEALTH),
            )
        })
        .await
        .unwrap();
    assert!(!waiting, "not before the battle");
    assert!(alive, "once it is on");
    assert_eq!(health, max, "whole");
}

// The events the world drives.

const DRIVEN: u32 = 900_004;
const SCHEDULED: u32 = 900_009;

fn events_data() -> GameData {
    let mut d = GameData::empty();
    d.events.insert(
        DRIVEN,
        GameEvent {
            id: DRIVEN,
            name: "Driven".into(),
            hardcoded: true,
            ..Default::default()
        },
    );
    // A minute in each hour from the epoch's start.
    d.events.insert(
        SCHEDULED,
        GameEvent {
            id: SCHEDULED,
            name: "Scheduled".into(),
            start: 0,
            end: i64::MAX,
            occurence: 60,
            length: 1,
            hardcoded: false,
        },
    );
    d
}

/// Thistle Tea's `GameEvent.drive/2` "driven events hold their state
/// across schedule transitions": a scheduled event on in its minute is off
/// once the world drives it off, and a driven one on; both hold so into
/// the next hour's minute, where the schedule has the first on again; the
/// driven one let go is off, and an unknown event driven on is none.
#[test]
fn driven_events_hold_their_state_across_the_schedules_turns() {
    use crate::game::events::active_with;
    let d = events_data();
    let hour = 3600;
    let mut driven = AHashMap::new();
    let ids = |on: ahash::AHashSet<u32>| {
        let mut v: Vec<u32> = on.into_iter().collect();
        v.sort();
        v
    };
    assert_eq!(
        ids(active_with(&d, 10, &driven, &Default::default())),
        vec![SCHEDULED],
        "scheduled"
    );
    driven.insert(DRIVEN, true);
    driven.insert(SCHEDULED, false);
    driven.insert(99, true);
    assert_eq!(
        ids(active_with(&d, 10, &driven, &Default::default())),
        vec![DRIVEN],
        "driven"
    );
    assert_eq!(
        ids(active_with(&d, hour + 10, &driven, &Default::default())),
        vec![DRIVEN],
        "held into the schedule's next turn"
    );
    driven.insert(DRIVEN, false);
    assert_eq!(
        ids(active_with(&d, hour + 10, &driven, &Default::default())),
        Vec::<u32>::new()
    );
}
