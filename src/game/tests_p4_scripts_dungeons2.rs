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

/// Molten Core's script: a rune can't be doused while its lieutenant
/// lives, and doused puts out its circle for good; the last rune out
/// brings Majordomo, once; Golemagg's spawn or evade resets his fight and
/// brings his ragers back as new, in his copy alone, unless he is beaten;
/// his ragers go with him and don't come back, though another boss's death
/// leaves them; Majordomo's death has Ragnaros roar seven seconds on and
/// turn on the raid three after; Ragnaros's death settles the raid; and
/// Majordomo beaten brings out the Cache of the Firelord, once.
#[tokio::test]
#[ignore = "extended"]
async fn molten_cores_script_runs_its_runes_and_bosses() {
    const LIEUTENANTS: [u32; 7] = [12098, 12056, 12264, 11988, 12057, 11982, 12259];
    const RUNES: [u32; 7] = [176951, 176952, 176953, 176954, 176955, 176956, 176957];
    const GOLEMAGG: u32 = 11988;
    const RAGER: u32 = 11672;
    const RAGERS: [u32; 2] = [56685, 56699];
    let Some(realm) = realm() else {
        return;
    };
    // The roster: Golemagg's ragers are of the raid, by him.
    let boss = spawn_of(&realm, 409, GOLEMAGG);
    for s in RAGERS {
        let r = realm.data.creature_spawns[&409]
            .iter()
            .find(|x| x.guid == s)
            .unwrap();
        assert!(r.entries.contains(&RAGER));
        assert!(((r.x - boss.x).powi(2) + (r.y - boss.y).powi(2)).sqrt() < 150.0);
    }
    let (_c, me, map) = inside(&realm, "Runes", 409, GOLEMAGG).await;
    let (_d, _, other) = inside(&realm, "Elsewhere", 409, GOLEMAGG).await;
    map.query(move |m| {
        assert_eq!(
            instance_script::guid_data(m, 12),
            first(m, GOLEMAGG),
            "Golemagg is index 12"
        );
        // Sulfuron's rune.
        assert!(scripts::object_of_spawn(m, 43157).is_some());
        instance_script::object_used(m, RUNES[0]);
        assert_eq!(data(m, 16), 0, "lit while Sulfuron lives");
        kill_all(m, me, 12098);
        instance_script::object_used(m, RUNES[0]);
        assert_eq!(data(m, 16), DONE);
        assert!(
            scripts::object_of_spawn(m, 43157).is_none(),
            "its circle out"
        );
        instance_script::settle_spawns(m);
        assert!(scripts::object_of_spawn(m, 43157).is_none(), "and stays");
        // Golemagg's fight, reset with his ragers.
        let ragers = RAGERS.map(|s| scripts::creature_of_spawn(m, s).unwrap());
        for event in [Event::Spawned, Event::Evade] {
            set(m, 3, IN_PROGRESS);
            kill(m, me, ragers[0]);
            let g = first(m, GOLEMAGG).unwrap();
            instance_script::creature_event(m, g, event);
            assert_eq!(data(m, 3), 0, "{event:?} resets it");
            assert!(
                ragers
                    .iter()
                    .all(|r| m.creature(*r).is_some_and(|c| c.alive())),
                "{event:?}: both ragers back"
            );
        }
        let g = first(m, GOLEMAGG).unwrap();
        crate::game::map::ai::attacked(m, g, me);
        assert_eq!(data(m, 3), IN_PROGRESS, "his pull");
    })
    .await
    .unwrap();
    assert_eq!(
        other.query(|m| data(m, 3)).await,
        Some(0),
        "the other copy's own"
    );
    map.query(move |m| {
        // Another boss's death leaves his ragers.
        kill_all(m, me, 12057);
        assert_eq!(living(m, RAGER).len(), 2);
        let g = first(m, GOLEMAGG).unwrap();
        crate::game::map::ai::enter_evade(m, g);
        kill(m, me, g);
        assert_eq!(data(m, 3), DONE);
        assert!(living(m, RAGER).is_empty(), "his ragers go with him");
        // Spawned again, a rager goes; nothing brings them back.
        let r = scripts::creature_of_spawn(m, RAGERS[0]).unwrap();
        crate::game::map::ai::respawn_now(m, r);
        instance_script::creature_event(m, r, Event::Spawned);
        assert!(
            living(m, RAGER).is_empty(),
            "a dead lieutenant's guard stays"
        );
        for event in [Event::Spawned, Event::Aggro, Event::Evade] {
            instance_script::creature_event(m, g, event);
            assert_eq!(data(m, 3), DONE);
            assert!(living(m, RAGER).is_empty());
        }
        // Every lieutenant dead, the runes out one by one.
        for e in LIEUTENANTS {
            kill_all(m, me, e);
        }
        for (i, rune) in RUNES.iter().enumerate() {
            assert!(living(m, 12018).is_empty(), "no Majordomo with {i} out");
            instance_script::object_used(m, *rune);
        }
        assert_eq!(living(m, 12018).len(), 1, "Majordomo, with the last");
        instance_script::object_used(m, RUNES[6]);
        assert_eq!(living(m, 12018).len(), 1, "once");
        // Majordomo beaten (he submits, rather than falls to a blow: his
        // death as the script hears of it).
        assert!(scripts::object_of_spawn(m, 362148).is_none());
        let majordomo = first(m, 12018).unwrap();
        instance_script::creature_event(m, majordomo, Event::Death);
        assert_eq!(data(m, 8), DONE);
        assert!(scripts::object_of_spawn(m, 362148).is_some(), "the Cache");
        set(m, 8, DONE);
        let caches = m
            .gameobjects
            .iter()
            .filter(|(_, o)| o.spawn_guid == Some(362148))
            .count();
        assert_eq!(caches, 1, "once");
        let roar = m
            .instance
            .timers
            .iter()
            .find(|(_, k)| *k == 3)
            .map(|(t, _)| *t);
        let left = roar.map(|t| t.saturating_duration_since(m.now));
        assert!(about(left, 7.0), "Ragnaros roars in seven: {left:?}");
        // Ragnaros, as Majordomo's script calls him up: out of reach.
        let rag = summon(
            m,
            11502,
            wow_world_messages::vanilla::Vector3d {
                x: 840.0,
                y: -812.0,
                z: -229.6,
            },
            0.0,
        );
        if let Some(f) = m.unit_fields_mut(rag) {
            f.set_flag(UNIT_FIELD_FLAGS, IMMUNE_TO_PLAYER, true);
        }
        hurry_copy(m, Duration::from_millis(7050));
        assert!(
            m.instance.timers.iter().any(|(_, k)| *k == 4),
            "then engages"
        );
        assert_ne!(unit_flags(m, rag) & IMMUNE_TO_PLAYER, 0);
        hurry_copy(m, Duration::from_millis(3050));
        assert_eq!(
            unit_flags(m, rag) & IMMUNE_TO_PLAYER,
            0,
            "three on, in reach"
        );
        kill(m, me, rag);
        assert_eq!(data(m, 9), DONE, "the raid settled");
    })
    .await
    .unwrap();
}

/// The real world's content for these scripts: Blackfathom's fires and
/// portal, one each and shut, Kelris reporting his death; the Deadmines'
/// three boss doors, its alarm's two pirates, the gunpowder and its lines;
/// Scholomance's gates spawning open, Gandling reporting his failure and
/// his death and Kirtonos his evade, the shadow portal rooms' map events;
/// Stratholme's Timmy spawner, Willey's gate open, the bosses' lines.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_dungeons_content() {
    let Some(realm) = realm() else {
        return;
    };
    let d = &realm.data;
    let states = |map: u32, entry: u32| -> Vec<u8> {
        d.gameobject_spawns[&map]
            .iter()
            .filter(|s| s.entry == entry)
            .map(|s| s.state)
            .collect()
    };
    for (map, script) in [
        (48, "instance_blackfathom_deeps"),
        (36, "instance_deadmines"),
        (289, "instance_scholomance"),
    ] {
        assert_eq!(d.scripts.map_templates[&map].script_name, script);
    }
    for e in 21117..=21121 {
        assert_eq!(states(48, e), vec![1], "{e}");
    }
    for e in [13965, 16400, 16399] {
        assert_eq!(states(36, e).len(), 1, "{e}");
    }
    for s in [79289, 79290] {
        let p = d.creature_spawns[&36].iter().find(|x| x.guid == s).unwrap();
        assert_eq!(p.entries, vec![657]);
    }
    assert!(d.items.get(&5397).is_some_and(|i| i.spells[0].id == 6250));
    for t in [1148, 1149, 6150, 6441, 6442, 6447, 6504, 6530] {
        assert!(d.npc.broadcast_texts.contains_key(&t), "{t}");
    }
    for e in [175570, 177371, 177372, 177373, 177375, 177376, 177377] {
        assert_eq!(states(289, e), vec![0], "{e} open");
    }
    for e in [175167, 175564] {
        assert_eq!(states(289, e), vec![1], "{e}");
    }
    for e in 5618..=5623 {
        assert!(
            d.scripts.event.get(&e).is_some_and(|s| s.len() >= 3),
            "map event {e}"
        );
    }
    let timmy = d.creature_spawns[&329]
        .iter()
        .find(|x| x.guid == 54070)
        .unwrap();
    assert!(timmy.entries.contains(&10418));
    assert_eq!(states(329, 175969), vec![0], "Willey's gate open");
    // Kelris.
    let (_c, me, map) = inside(&realm, "Kelris", 48, 4832).await;
    map.query(move |m| kill_all(m, me, 4832)).await;
    assert!(
        until(&map, 3000, |m| data(m, 10) == DONE).await,
        "Kelris reports his death"
    );
    // Gandling and Kirtonos.
    let (_c, me, map) = inside(&realm, "Herald", 289, 175564).await;
    let (gandling, kirtonos) = map
        .query(move |m| {
            let at = m.position_of(me).unwrap();
            let g = summon(m, 1853, at, 0.0);
            instance_script::object_used(m, 175564);
            let k = first(m, 10506).unwrap();
            crate::game::map::ai::attacked(m, g, me);
            crate::game::map::ai::attacked(m, k, me);
            crate::game::map::ai::enter_evade(m, g);
            crate::game::map::ai::enter_evade(m, k);
            (g, k)
        })
        .await
        .unwrap();
    assert!(
        until(&map, 3000, |m| data(m, 0) == FAILED && data(m, 7) == FAILED).await,
        "Gandling's failure and Kirtonos's evade reported"
    );
    map.query(move |m| {
        set(m, 0, IN_PROGRESS);
        kill(m, me, gandling);
        let _ = kirtonos;
    })
    .await;
    assert!(
        until(&map, 3000, |m| data(m, 0) == DONE).await,
        "Gandling's death reported"
    );
}
