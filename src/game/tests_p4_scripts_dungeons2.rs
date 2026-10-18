//! Thistle Tea's instance scripts on the real world, continued: the
//! Sunken Temple's Shade of Eranikus, Razorfen Downs' gong and idol,
//! the Ruins of Ahn'Qiraj's Andorov, Scholomance's Gandling and Kirtonos,
//! and Blackfathom Deeps' shrine.

use std::time::Duration;

use wow_world_messages::Guid;

use super::tests_p3_scripts::*;
use super::tests_p4_scripts_dungeons::*;
use crate::game::fields::*;
use crate::game::map::instance_script::{self, DONE, Event, FAILED, IN_PROGRESS};
use crate::game::map::{Map, scripts};

const IMMUNE_TO_PLAYER: u32 = 0x100;
const GO_FLAG_NO_INTERACT: u32 = 0x10;

fn object_flags(m: &Map, entry: u32) -> u32 {
    m.gameobjects
        .iter()
        .find(|(_, o)| o.entry == entry)
        .map_or(0, |(_, o)| o.fields.get_u32(GAMEOBJECT_FLAGS))
}

/// The Sunken Temple: the Shade of Eranikus sleeps out of reach as he
/// spawns; Jammal'an's fall (and only that) makes him reachable, still
/// asleep, and a spawn after leaves him so; his fight wakes him.
#[tokio::test]
#[ignore = "extended"]
async fn the_shade_of_eranikus_sleeps_until_jammalan_falls() {
    const SHADE: u32 = 5709;
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = inside(&realm, "Shade", 109, SHADE).await;
    map.query(move |m| {
        let g = first(m, SHADE).expect("the Shade");
        let state = |m: &Map| (unit_flags(m, g) & IMMUNE_TO_PLAYER != 0, stand_state(m, g));
        instance_script::creature_event(m, g, Event::Spawned);
        assert_eq!(state(m), (true, 3), "asleep, out of reach");
        set(m, 6, IN_PROGRESS);
        assert_eq!(state(m), (true, 3));
        set(m, 6, DONE);
        assert_eq!(state(m), (false, 3), "in reach, still asleep");
        set(m, 6, IN_PROGRESS);
        assert_eq!(data(m, 6), DONE);
        instance_script::creature_event(m, g, Event::Spawned);
        assert_eq!(state(m), (false, 3), "a spawn after leaves him so");
        crate::game::map::ai::attacked(m, g, me);
        assert_eq!(state(m), (false, 0), "awake");
    })
    .await
    .unwrap();
}

/// Razorfen Downs: the gong calls eight Tomb Fiends, each at its own
/// spot, and falls silent; with the wave dead it wakes, and calls four
/// Tomb Reavers, then Tuten'kash, after which it stays silent as it spawns
/// again; the idol's fires put out take the oven, mouth and cup fires.
#[tokio::test]
#[ignore = "extended"]
async fn razorfen_downs_gong_calls_its_waves() {
    const GONG: u32 = 148917;
    let Some(realm) = realm() else {
        return;
    };
    let (_c, _me, map) = inside(&realm, "Gong", 129, GONG).await;
    map.query(|m| {
        let silent = |m: &Map| object_flags(m, GONG) & GO_FLAG_NO_INTERACT != 0;
        assert!(!silent(m));
        let gong = |m: &mut Map, entry: u32, n: usize| {
            let before = living(m, entry);
            instance_script::object_used(m, GONG);
            let new: Vec<Guid> = living(m, entry)
                .into_iter()
                .filter(|g| !before.contains(g))
                .collect();
            assert_eq!(new.len(), n, "{n} of {entry}");
            new
        };
        let fiends = gong(m, 7349, 8);
        let mut spots: Vec<(i32, i32)> = fiends
            .iter()
            .map(|g| {
                let p = m.position_of(*g).unwrap();
                ((p.x * 10.0) as i32, (p.y * 10.0) as i32)
            })
            .collect();
        spots.sort_unstable();
        spots.dedup();
        assert_eq!(spots.len(), 8, "each at its own spot");
        assert!(silent(m), "silent");
        assert_eq!(data(m, 1), 1);
        for _ in 0..7 {
            instance_script::set_data(m, 1, 1, 1);
            assert!(silent(m));
        }
        instance_script::set_data(m, 1, 1, 1);
        assert_eq!(data(m, 1), 9);
        assert!(!silent(m), "the wave dead, it wakes");
        gong(m, 7351, 4);
        for _ in 0..4 {
            instance_script::set_data(m, 1, 1, 1);
        }
        assert_eq!(data(m, 1), 14);
        assert!(!silent(m));
        gong(m, 7355, 1);
        assert!(silent(m), "Tuten'kash");
        // Spawned again, it stays silent.
        let gs: Vec<Guid> = m
            .gameobjects
            .iter()
            .filter(|(_, o)| o.entry == GONG)
            .map(|(_, o)| o.guid)
            .collect();
        for g in gs {
            if let Some(o) = m.gameobject_mut(g) {
                let f = o.fields.get_u32(GAMEOBJECT_FLAGS);
                o.fields.set_u32(GAMEOBJECT_FLAGS, f & !GO_FLAG_NO_INTERACT);
            }
        }
        instance_script::settle_spawns(m);
        assert!(silent(m), "silent as it spawns again");
        // The idol's fires.
        let fires = [32027, 32029, 32030, 32031];
        assert!(
            fires
                .iter()
                .all(|s| scripts::object_of_spawn(m, *s).is_some())
        );
        set(m, 2, 1);
        assert!(
            fires
                .iter()
                .all(|s| scripts::object_of_spawn(m, *s).is_none()),
            "the fires out"
        );
    })
    .await
    .unwrap();
}

/// The Ruins of Ahn'Qiraj: Kurinnaxx's death brings Andorov and his
/// squad, the world database's held-back spawns, once; Andorov marches as
/// he comes.
#[tokio::test]
#[ignore = "extended"]
async fn kurinnaxx_falls_and_andorov_comes() {
    const SQUAD: [u32; 5] = [301311, 301312, 301313, 301314, 301315];
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = inside(&realm, "Ruins", 509, 15348).await;
    let andorov = map
        .query(move |m| {
            assert!(
                SQUAD
                    .iter()
                    .all(|s| scripts::creature_of_spawn(m, *s).is_none())
            );
            assert!(living(m, 15471).is_empty(), "held back");
            let k = first(m, 15348).unwrap();
            kill(m, me, k);
            let squad: Vec<Option<Guid>> = SQUAD
                .iter()
                .map(|s| scripts::creature_of_spawn(m, *s))
                .collect();
            assert!(squad.iter().all(Option::is_some), "the squad: {squad:?}");
            instance_script::creature_event(m, k, Event::Death);
            assert_eq!(living(m, 15471).len(), 1, "once");
            squad[0].unwrap()
        })
        .await
        .unwrap();
    let from = map
        .query(move |m| m.position_of(andorov))
        .await
        .flatten()
        .unwrap();
    // Where a player sees him: one no player sees doesn't move.
    relocate(&map, &mut c, from).await;
    assert!(
        until(&map, 8000, move |m| {
            m.position_of(andorov)
                .is_some_and(|p| crate::game::map::combat::dist(p, from) > 2.0)
        })
        .await,
        "Andorov marches"
    );
}

/// Scholomance: Gandling's failure opens the room gates, his death keeps
/// them open, and they stand open as they spawn again; Kirtonos evading
/// opens his gate and leaves the brazier to be lit again.
#[tokio::test]
#[ignore = "extended"]
async fn scholomances_gates_follow_gandling_and_kirtonos() {
    const GATES: [u32; 6] = [177371, 177372, 177373, 177375, 177376, 177377];
    const KIRTONOS_GATE: u32 = 175570;
    const BRAZIER: u32 = 175564;
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = inside(&realm, "Gandling", 289, BRAZIER).await;
    map.query(move |m| {
        for (boss, _) in [
            (11261, 1),
            (10505, 2),
            (10502, 3),
            (10504, 4),
            (10901, 5),
            (10507, 6),
        ] {
            kill_all(m, me, boss);
        }
        assert_eq!(data(m, 0), IN_PROGRESS, "Gandling comes");
        assert_eq!(living(m, 1853).len(), 1);
        // His fight has the gates shut.
        let shut = respawned(m, &GATES);
        assert!(shut.iter().all(|s| *s == Some(1)), "{shut:?}");
        set(m, 0, FAILED);
        let open: Vec<Option<u32>> = GATES.iter().map(|e| door(m, *e)).collect();
        assert!(open.iter().all(|s| *s == Some(0)), "his failure: {open:?}");
        set(m, 0, DONE);
        assert_eq!(data(m, 0), DONE);
        let again = respawned(m, &GATES);
        assert!(again.iter().all(|s| *s == Some(0)), "{again:?}");
        // Kirtonos.
        instance_script::object_used(m, BRAZIER);
        assert_eq!(door(m, KIRTONOS_GATE), Some(1), "his gate shuts");
        let k = living(m, 10506);
        assert_eq!(k.len(), 1);
        set(m, 7, FAILED);
        assert_eq!(door(m, KIRTONOS_GATE), Some(0), "his evade opens it");
        scripts::despawn(m, k[0]);
        instance_script::object_used(m, BRAZIER);
        assert_eq!(living(m, 10506).len(), 1, "the brazier calls him again");
        assert_eq!(door(m, KIRTONOS_GATE), Some(1));
    })
    .await
    .unwrap();
}

/// Blackfathom Deeps: once Kelris is dead and the fires lit, only the
/// servants the shrine called count toward it: the dungeon's own spawns
/// of the same creatures don't.
#[tokio::test]
#[ignore = "extended"]
async fn blackfathoms_own_spawns_dont_count_toward_the_shrine() {
    const REMAINING: u32 = 2;
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = inside(&realm, "Shrine", 48, 21118).await;
    map.query(move |m| {
        set(m, 10, DONE);
        instance_script::object_used(m, 21118);
        hurry_copy(m, Duration::from_millis(3100));
        let left = m.instance.scratch.get(&REMAINING).copied();
        assert_eq!(left, Some(4), "the first wave's four");
        // One of the dungeon's own, of a servant's entry.
        let own =
            scripts::summon(m, Guid::zero(), 4825, m.position_of(me).unwrap(), 0.0, None).unwrap();
        if let Some(c) = m.creature_mut(own) {
            c.spawn_guid = Some(32_000);
        }
        kill(m, me, own);
        assert_eq!(m.instance.scratch.get(&REMAINING).copied(), Some(4));
        let called = living(m, 4825)
            .into_iter()
            .find(|g| m.creature(*g).is_some_and(|c| c.spawn_guid.is_none()))
            .unwrap();
        kill(m, me, called);
        assert_eq!(
            m.instance.scratch.get(&REMAINING).copied(),
            Some(3),
            "one it called"
        );
    })
    .await
    .unwrap();
}
