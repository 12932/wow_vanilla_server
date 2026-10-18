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
