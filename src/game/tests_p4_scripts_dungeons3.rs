//! Thistle Tea's instance scripts on the real world, continued: Dire
//! Maul's generators, Immol'thar, the east wing and the Gordok tribute.

use std::time::Duration;

use wow_world_messages::Guid;

use super::tests_p3_scripts::*;
use super::tests_p4_scripts_dungeons::*;
use crate::game::map::instance_script::{self, DONE, Event, IN_PROGRESS};
use crate::game::map::{Map, scripts};

const IMMOLTHAR: u32 = 11496;
const TORTHELDRIN: u32 = 11486;
const SUMMONER: u32 = 11466;
const CHORUSH: u32 = 14324;
const FIELD: u32 = 179503;
const VORTEX: u32 = 179506;
const WALL: u32 = 177220;
const VINE: u32 = 179502;
const GENERATORS: [(u32, &[u32]); 5] = [
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

fn kill_spawn(m: &mut Map, by: Guid, spawn: u32) {
    let g = scripts::creature_of_spawn(m, spawn).unwrap_or_else(|| panic!("spawn {spawn}"));
    kill(m, by, g);
}

/// Dire Maul's west: players can't work the field, the vortex, the wall,
/// the vine or a generator; Immol'thar is held while the generators stand;
/// each goes dark with its last guard, and stays dark as it spawns again;
/// the last drops the field and the vortex, frees Immol'thar (and he spawns
/// free after) and sets the hall's summoners on him; his death opens
/// Tortheldrin to attack, as a Tortheldrin who spawns after is, though one
/// before isn't.
#[tokio::test]
#[ignore = "extended"]
async fn dire_mauls_generators_hold_immolthar() {
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = inside(&realm, "West", 429, IMMOLTHAR).await;
    map.query(move |m| {
        for e in [FIELD, VORTEX, WALL, VINE, GENERATORS[0].0] {
            let g = m
                .gameobjects
                .iter()
                .find(|(_, o)| o.entry == e)
                .map(|(_, o)| o.guid)
                .unwrap_or_else(|| panic!("object {e}"));
            assert!(instance_script::scripted_door(m, g), "{e} is the script's");
        }
        let immolthar = first(m, IMMOLTHAR).unwrap();
        instance_script::creature_event(m, immolthar, Event::Spawned);
        assert_eq!(unit_flags(m, immolthar) & 0x0200_0002, 0x0200_0002, "held");
        let tortheldrin = first(m, TORTHELDRIN).unwrap();
        let before = (unit_flags(m, tortheldrin), faction(m, tortheldrin));
        instance_script::creature_event(m, tortheldrin, Event::Spawned);
        assert_eq!(
            (unit_flags(m, tortheldrin), faction(m, tortheldrin)),
            before,
            "not open before Immol'thar falls"
        );
        // The first generator.
        let (generator, guards) = GENERATORS[0];
        let (last, others) = guards.split_last().unwrap();
        for s in others {
            kill_spawn(m, me, *s);
        }
        assert_eq!(door(m, generator), Some(1), "lit with a guard left");
        kill_spawn(m, me, *last);
        assert_eq!(door(m, generator), Some(0), "dark with its last");
        assert_eq!(respawned(m, &[generator]), vec![Some(0)]);
        assert_eq!(data(m, 1), 0);
        // The rest.
        for (_, guards) in &GENERATORS[1..] {
            for s in *guards {
                kill_spawn(m, me, *s);
            }
        }
        assert_eq!(data(m, 1), DONE);
        assert_eq!((door(m, FIELD), door(m, VORTEX)), (Some(0), Some(0)));
        assert_eq!(unit_flags(m, immolthar) & 0x0200_0202, 0, "freed");
        instance_script::creature_event(m, immolthar, Event::Spawned);
        assert_eq!(unit_flags(m, immolthar) & 0x0200_0002, 0, "spawns free");
        assert_eq!(respawned(m, &[FIELD]), vec![Some(0)]);
        let hall: Vec<Guid> = m
            .instance
            .creature_timers
            .iter()
            .map(|(_, _, g)| *g)
            .collect();
        assert!(!hall.is_empty(), "the summoners set on him");
        for s in &hall {
            assert_eq!(m.creature(*s).map(|c| c.entry), Some(SUMMONER));
            assert_eq!(faction(m, *s), 100);
        }
        let s = hall[0];
        hurry_copy(m, Duration::from_millis(1100));
        assert_eq!(victim(m, s), immolthar, "a summoner turns on Immol'thar");
        // Immol'thar's death.
        kill(m, me, immolthar);
        assert_eq!(data(m, 2), DONE);
        assert_eq!(unit_flags(m, tortheldrin) & 0x100, 0, "open to attack");
        assert_eq!(faction(m, tortheldrin), 14);
    })
    .await
    .unwrap();
    // A Tortheldrin spawning after.
    let (_c, me, map) = inside(&realm, "Late", 429, IMMOLTHAR).await;
    map.query(move |m| {
        let immolthar = first(m, IMMOLTHAR).unwrap();
        instance_script::creature_event(m, immolthar, Event::Death);
        let t = first(m, TORTHELDRIN).unwrap();
        // Back as it spawned.
        if let Some(f) = m.unit_fields_mut(t) {
            f.set_flag(crate::game::fields::UNIT_FIELD_FLAGS, 0x100, true);
            f.set_u32(crate::game::fields::UNIT_FIELD_FACTIONTEMPLATE, 35);
        }
        instance_script::creature_event(m, t, Event::Spawned);
        assert_eq!((unit_flags(m, t) & 0x100, faction(m, t)), (0, 14));
        let _ = me;
    })
    .await
    .unwrap();
}

/// Dire Maul's east and north: Zevrim's death frees Old Ironbark; Alzzin
/// pressed breaks the wall, and his death opens the vine and raises the
/// Felvine Shards, once. The tribute counts each guard once, from six, and
/// stops counting once the tribute is won, its chest raised once; the
/// king's death calls Mizzle and stands Cho'Rush down, though not a fallen
/// one.
#[tokio::test]
#[ignore = "extended"]
async fn dire_mauls_east_wing_and_the_gordok_tribute() {
    const SHARDS: [u32; 5] = [44726, 44727, 44728, 44729, 44730];
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = inside(&realm, "East", 429, 11492).await;
    map.query(move |m| {
        kill_all(m, me, 11490);
        assert_eq!(data(m, 4), DONE, "Zevrim");
        set(m, 11, 4);
        assert_eq!(door(m, WALL), Some(0), "the wall breaks");
        assert!(
            SHARDS
                .iter()
                .all(|s| scripts::object_of_spawn(m, *s).is_none())
        );
        kill_all(m, me, 11492);
        assert_eq!(door(m, VINE), Some(0), "the vine");
        assert!(
            SHARDS
                .iter()
                .all(|s| scripts::object_of_spawn(m, *s).is_some())
        );
        let a = m
            .creatures
            .iter()
            .find(|(_, c)| c.entry == 11492)
            .map(|(_, c)| c.guid);
        instance_script::creature_event(m, a.unwrap(), Event::Death);
        let n = m
            .gameobjects
            .iter()
            .filter(|(_, o)| o.spawn_guid.is_some_and(|s| SHARDS.contains(&s)))
            .count();
        assert_eq!(n, 5, "once");
        // The tribute.
        assert_eq!(data(m, 15), 6);
        kill_all(m, me, 14321);
        assert_eq!(data(m, 15), 5);
        set(m, 6, 4);
        assert_eq!(data(m, 6), IN_PROGRESS);
        kill_all(m, me, 14326);
        assert_eq!((data(m, 15), data(m, 10)), (4, DONE), "Moldar");
        assert!(scripts::object_of_spawn(m, 396409).is_none());
        set(m, 6, DONE);
        assert!(scripts::object_of_spawn(m, 396409).is_some(), "the chest");
        kill_all(m, me, CHORUSH);
        assert_eq!(data(m, 15), 4, "counted no more");
        set(m, 6, DONE);
        let chests = m
            .gameobjects
            .iter()
            .filter(|(_, o)| o.spawn_guid == Some(396409))
            .count();
        assert_eq!(chests, 1);
    })
    .await
    .unwrap();
    // The king falls, Cho'Rush standing.
    let (_c, me, map) = inside(&realm, "King", 429, 11501).await;
    map.query(move |m| {
        let chorush = first(m, CHORUSH).unwrap();
        kill_all(m, me, 11501);
        assert_eq!(living(m, 14353).len(), 1, "Mizzle");
        assert_eq!(faction(m, chorush), 35, "Cho'Rush stands down");
        assert!(
            m.instance
                .creature_timers
                .iter()
                .any(|(_, _, g)| *g == chorush),
            "and speaks"
        );
    })
    .await
    .unwrap();
    // The king falls after Cho'Rush.
    let (_c, me, map) = inside(&realm, "Fallen", 429, 11501).await;
    map.query(move |m| {
        kill_all(m, me, CHORUSH);
        kill_all(m, me, 11501);
        assert_eq!(living(m, 14353).len(), 1, "Mizzle");
        assert!(m.instance.creature_timers.is_empty(), "no one to speak");
    })
    .await
    .unwrap();
}

const BRS_ROOMS: [(u32, &[u32]); 7] = [
    (175197, &[40259, 40260]),
    (175199, &[40254, 40255, 45834]),
    (175195, &[40251]),
    (175200, &[40262, 40263]),
    (175198, &[40267, 40268, 45833]),
    (175196, &[40270, 45832]),
    (175194, &[40277, 40455]),
];

/// The first timer due under `key`: how long until it.
fn due_in(m: &Map, key: u32) -> Option<Duration> {
    m.instance
        .timers
        .iter()
        .filter(|(_, k)| *k == key)
        .map(|(t, _)| t.saturating_duration_since(m.now))
        .min()
}

/// Blackrock Spire: an alcove's rune goes dark with its guardians, and
/// stays dark as it spawns again, the others lit; a death outside the
/// alcoves changes nothing; every rune dark opens the way to the
/// Emberseer, whose defeat opens his exit for good; the Seal of Ascension
/// lights the braziers two by two, then the Dragonspine Door, which stays
/// open; Drakkisath's defeat opens the gates behind him; Father Flame
/// starts the rookery (once), its waves of hatchers and guardians, then
/// Solakar; once Drakkisath is dead it stays cold.
#[tokio::test]
#[ignore = "extended"]
async fn blackrock_spires_runes_seal_and_rookery() {
    const EMBERSEER_IN: u32 = 175244;
    const EMBERSEER_OUT: u32 = 175153;
    const DRAGONSPINE: u32 = 164725;
    const FATHER_FLAME: u32 = 175245;
    const BRAZIERS: [(u32, u32); 3] = [(175528, 175529), (175530, 175531), (175532, 175533)];
    const T_DOOR: u32 = 1500;
    const T_ROOKERY: u32 = 1510;
    let Some(realm) = realm() else {
        return;
    };
    // Each alcove's guardians: the summoners and veterans within ten yards
    // of its rune; the rookery's creatures and the seal are there.
    for (rune, guards) in BRS_ROOMS {
        let r = spawn_of(&realm, 229, rune);
        let mut near: Vec<u32> = realm.data.creature_spawns[&229]
            .iter()
            .filter(|c| c.entries.iter().any(|e| [9818, 9819].contains(e)))
            .filter(|c| (c.x - r.x).powi(2) + (c.y - r.y).powi(2) + (c.z - r.z).powi(2) < 100.0)
            .map(|c| c.guid)
            .collect();
        near.sort_unstable();
        // Some spawns by a rune roll one of two entries, a summoner or a
        // veteran among them (40252, 40253 and 40258, by the third).
        if rune == 175195 {
            assert_eq!(near, [40251, 40252, 40253, 40258]);
        } else {
            assert_eq!(near, guards, "rune {rune}");
        }
    }
    // A spawn that rolled a summoner by the rune guards it, as
    // mangos-classic sorts the alcoves; one that rolled otherwise doesn't.
    let (_c, me, map) = inside(&realm, "Rolled", 229, 175195).await;
    map.query(move |m| {
        let rolled = [40252, 40253, 40258].map(|s| scripts::creature_of_spawn(m, s).unwrap());
        for (g, entry) in rolled.iter().zip([9819, 9817, 9817]) {
            if let Some(c) = m.creature_mut(*g) {
                c.entry = entry;
            }
        }
        kill_spawn(m, me, 40251);
        assert_eq!(door(m, 175195), Some(0), "lit while the summoner stands");
        kill(m, me, rolled[0]);
        assert_eq!(door(m, 175195), Some(1), "dark");
    })
    .await
    .unwrap();
    for e in [10258, 10264, 10430, 10683] {
        assert!(realm.data.creature_template(e).is_some(), "{e}");
    }
    assert!(
        realm.data.items.contains_key(&12344),
        "the Seal of Ascension"
    );
    assert!(realm.data.npc.broadcast_texts.contains_key(&5538));
    let (_c, me, map) = inside(&realm, "Spire", 229, 175197).await;
    map.query(move |m| {
        let lit = |m: &Map, rune: u32| door(m, rune);
        let start: Vec<Option<u32>> = BRS_ROOMS.iter().map(|(r, _)| lit(m, *r)).collect();
        // A death outside the alcoves.
        let outsider = m
            .creatures
            .iter()
            .find(|(_, c)| {
                c.alive()
                    && c.spawn_guid
                        .is_some_and(|s| !BRS_ROOMS.iter().any(|(_, g)| g.contains(&s)))
                    && m.realm
                        .data
                        .creature_template(c.entry)
                        .is_some_and(|t| t.rank == 0)
            })
            .map(|(_, c)| c.guid)
            .unwrap();
        let before = m.instance.data.clone();
        kill(m, me, outsider);
        assert_eq!(m.instance.data, before, "nothing");
        // The first alcove.
        let (rune, guards) = BRS_ROOMS[0];
        kill_spawn(m, me, guards[0]);
        assert_eq!(lit(m, rune), start[0], "lit with one left");
        kill_spawn(m, me, guards[1]);
        assert_eq!(lit(m, rune), Some(1), "dark");
        instance_script::settle_spawns(m);
        assert_eq!(lit(m, rune), Some(1), "dark as it spawns again");
        assert_eq!(lit(m, BRS_ROOMS[1].0), start[1], "the next still lit");
        // The rest.
        assert_eq!(door(m, EMBERSEER_IN), Some(1));
        for (_, guards) in &BRS_ROOMS[1..] {
            for s in *guards {
                kill_spawn(m, me, *s);
            }
        }
        // And those by the third rune that rolled a guardian.
        for s in [40252, 40253, 40258] {
            kill_spawn(m, me, s);
        }
        assert_eq!(data(m, 0), DONE);
        assert_eq!(door(m, EMBERSEER_IN), Some(0), "the way to the Emberseer");
        // The Emberseer's defeat.
        set(m, 1, DONE);
        assert_eq!(respawned(m, &[EMBERSEER_OUT]), vec![Some(0)]);
        // The Seal of Ascension.
        set(m, 5, DONE);
        assert!(about(due_in(m, T_DOOR + 1), 2.0));
        set(m, 5, DONE);
        assert_eq!(
            m.instance
                .timers
                .iter()
                .filter(|(_, k)| *k == T_DOOR + 1)
                .count(),
            1
        );
        for (i, (l, r)) in BRAZIERS.iter().enumerate() {
            hurry_copy(m, Duration::from_millis(3050));
            assert_eq!((door(m, *l), door(m, *r)), (Some(0), Some(0)), "pair {i}");
            assert_eq!(door(m, DRAGONSPINE), Some(1));
        }
        hurry_copy(m, Duration::from_millis(3050));
        assert_eq!(door(m, DRAGONSPINE), Some(0), "the Dragonspine Door");
        assert_eq!(respawned(m, &[DRAGONSPINE]), vec![Some(0)]);
        // The rookery.
        instance_script::object_used(m, FATHER_FLAME);
        assert_eq!(data(m, 6), IN_PROGRESS);
        assert!(about(due_in(m, T_ROOKERY), 5.0));
        instance_script::object_used(m, FATHER_FLAME);
        assert_eq!(
            m.instance
                .timers
                .iter()
                .filter(|(_, k)| *k == T_ROOKERY)
                .count(),
            1
        );
        let (hatchers, guardians) = (living(m, 10683).len(), living(m, 10258).len());
        let mut called = 0;
        for wave in 0..5 {
            hurry_copy(m, Duration::from_secs(40));
            let now = living(m, 10683).len() + living(m, 10258).len();
            assert_eq!(now - hatchers - guardians - called, 2, "wave {wave}: two");
            called += 2;
            assert!(between(due_in(m, T_ROOKERY), 30.0, 40.0));
        }
        assert!(living(m, 10258).len() > guardians, "guardians among them");
        assert!(living(m, 10264).is_empty());
        hurry_copy(m, Duration::from_secs(40));
        assert_eq!(living(m, 10264).len(), 1, "Solakar");
        assert_eq!(data(m, 6), DONE);
        hurry_copy(m, Duration::from_secs(40));
        assert_eq!(living(m, 10264).len(), 1, "the rookery done");
        // Drakkisath's gates.
        set(m, 7, DONE);
        assert_eq!((door(m, 175946), door(m, 175947)), (Some(0), Some(0)));
    })
    .await
    .unwrap();
    // Father Flame cold once Drakkisath is dead.
    let (_c, _, map) = inside(&realm, "Cold", 229, 175197).await;
    map.query(|m| {
        set(m, 7, DONE);
        instance_script::object_used(m, FATHER_FLAME_ENTRY);
        assert_eq!(data(m, 6), 0);
    })
    .await
    .unwrap();
}

const FATHER_FLAME_ENTRY: u32 = 175245;
