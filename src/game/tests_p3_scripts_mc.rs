//! Molten Core's encounters as Thistle Tea scripts them, in a copy of the
//! raid on the real world: Golemagg's leash on his ragers, Majordomo
//! Executus and his guard, and Ragnaros from his summoning to his
//! submerges. The scripts' long waits are skipped, not waited out.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use crate::game::Realm;
use crate::game::harness::Client;
use crate::game::map::combat::dist;
use crate::game::map::{Map, ai, event_ai, scripts};

const GOLEMAGG: u32 = 11988;
const CORE_RAGER: u32 = 11672;
const GOLEMAGG_SPAWN: u32 = 56684;
const RAGER_SPAWNS: [u32; 2] = [56685, 56699];
const LEASH: u32 = 1167204;

/// The flat distance between two units' edges.
fn surface_2d(m: &Map, a: Guid, b: Guid) -> f32 {
    let r = |g: Guid| {
        m.unit_fields(g).map_or(0.0, |f| {
            f.get_f32(crate::game::fields::UNIT_FIELD_BOUNDINGRADIUS)
        })
    };
    let (pa, pb) = (m.position_of(a).unwrap(), m.position_of(b).unwrap());
    ((pa.x - pb.x).hypot(pa.y - pb.y) - r(a) - r(b)).max(0.0)
}

/// Golemagg put due west of `rager`, `surface` yards edge to edge.
fn golemagg_off(m: &mut Map, golemagg: Guid, rager: Guid, surface: f32) {
    let r = |g: Guid| {
        m.unit_fields(g).map_or(0.0, |f| {
            f.get_f32(crate::game::fields::UNIT_FIELD_BOUNDINGRADIUS)
        })
    };
    let at = m.position_of(rager).unwrap();
    let d = surface + r(golemagg) + r(rager);
    scripts::place(m, golemagg, Vector3d { x: at.x - d, ..at }, 0.0);
}

/// A fighting Core Rager looks for Golemagg every three seconds: more than
/// a hundred yards off him (edge to edge, on the flat) he evades, and his
/// ragers come back whole. A rager at peace doesn't look; one with no
/// Golemagg alive, or none in its copy, finds nothing to send home.
async fn golemaggs_leash(realm: &Arc<Realm>) {
    let at = Vector3d {
        x: 790.0,
        y: -985.0,
        z: -206.8,
    };
    let (_c, me, map) = hero_in(realm, "Leash", 409, at, 70.0, &[GOLEMAGG, CORE_RAGER]).await;
    let (golemagg, rager, other) = map
        .query(|m| {
            (
                scripts::creature_of_spawn(m, GOLEMAGG_SPAWN).unwrap(),
                scripts::creature_of_spawn(m, RAGER_SPAWNS[0]).unwrap(),
                scripts::creature_of_spawn(m, RAGER_SPAWNS[1]).unwrap(),
            )
        })
        .await
        .unwrap();
    // At peace, a rager doesn't look, wherever he is.
    map.query(move |m| {
        golemagg_off(m, golemagg, rager, 150.0);
        due(m, rager, LEASH);
    })
    .await;
    sleep_ms(200).await;
    assert!(
        map.query(move |m| at_peace(m, golemagg)).await.unwrap(),
        "no leash at peace"
    );
    map.query(move |m| {
        let home = m.creature(golemagg).unwrap().home;
        scripts::place(m, golemagg, home, 0.0);
        for g in [golemagg, rager, other] {
            tough(m, g);
            if let Some(s) = event_ai::scripted_mut(m, g) {
                s.no_combat_movement = true;
            }
        }
        ai::attacked(m, rager, me);
    })
    .await;
    assert!(
        until(&map, 2000, move |m| fighting(m, golemagg)).await,
        "Golemagg fights with his rager"
    );
    assert!(
        about(timer(&map, rager, LEASH).await, 3.0),
        "every three seconds"
    );
    // Within a hundred yards, he fights on; past, he goes home.
    let near = map
        .query(move |m| {
            golemagg_off(m, golemagg, rager, 99.9);
            let d = surface_2d(m, golemagg, rager);
            due(m, rager, LEASH);
            d
        })
        .await
        .unwrap();
    sleep_ms(200).await;
    assert!(
        map.query(move |m| fighting(m, golemagg) && !m.creature(golemagg).unwrap().evading)
            .await
            .unwrap(),
        "at {near} yards he fights on"
    );
    assert!(
        about(timer(&map, rager, LEASH).await, 3.0),
        "and again in three"
    );
    map.query(move |m| {
        crate::game::map::combat::deal_damage(m, me, other, 1000);
        golemagg_off(m, golemagg, rager, 100.3);
        due(m, rager, LEASH);
    })
    .await;
    assert!(
        until(&map, 2000, move |m| m
            .creature(golemagg)
            .is_some_and(|c| c.evading || !c.combat.attacking))
        .await,
        "past a hundred yards he evades"
    );
    assert!(
        until(&map, 3000, move |m| RAGER_SPAWNS.iter().all(|s| {
            scripts::creature_of_spawn(m, *s).is_some_and(|r| {
                let (hp, max) = health(m, r);
                at_peace(m, r) && hp == max
            })
        }))
        .await,
        "his ragers back whole"
    );
    // A rager whose Golemagg is dead (with no word to the copy, which would
    // take his ragers with him) finds no one to send home; nor one with no
    // Golemagg in its world.
    map.query(move |m| ai::attacked(m, rager, me)).await;
    assert!(until(&map, 2000, move |m| fighting(m, rager)).await);
    map.query(move |m| {
        golemagg_off(m, golemagg, rager, 150.0);
        ai::quiet_death(m, golemagg);
        due(m, rager, LEASH);
    })
    .await;
    sleep_ms(300).await;
    let after = map
        .query(move |m| {
            let g = m.creature(golemagg).unwrap();
            (g.alive(), g.evading, fighting(m, rager))
        })
        .await
        .unwrap();
    assert_eq!(after, (false, false, true), "nothing sent home");
    let (_d, outside, world) = hero(realm, "Leashless").await;
    let lone = map_summon(&world, CORE_RAGER).await;
    world
        .query(move |m| {
            tough(m, lone);
            ai::attacked(m, lone, outside);
            due(m, lone, LEASH);
        })
        .await;
    sleep_ms(200).await;
    assert!(
        world.query(move |m| fighting(m, lone)).await.unwrap(),
        "no Golemagg in its world: it fights on"
    );
    clear(&world, vec![lone]);
}

/// A creature of `entry` five yards east of the start.
async fn map_summon(map: &crate::game::map::MapHandle, entry: u32) -> Guid {
    summon_near(map, entry, off(northshire(), 5.0, 0.0)).await
}

fn northshire() -> Vector3d {
    Vector3d {
        x: START.0,
        y: START.1,
        z: START.2,
    }
}

const MAJORDOMO: u32 = 12018;
const ELITE: u32 = 11664;
const HEALER: u32 = 11663;
const RAGNAROS: u32 = 11502;
const SON_OF_FLAME: u32 = 12143;

/// Majordomo's hall, where the runes bring him.
const HALL: Vector3d = Vector3d {
    x: 758.089,
    y: -1176.71,
    z: -118.64,
};
/// Where he waits in Ragnaros's lair.
const LAIR: Vector3d = Vector3d {
    x: 847.103,
    y: -816.153,
    z: -229.775,
};

fn guard(m: &Map) -> Vec<Guid> {
    let mut all = living(m, ELITE);
    all.extend(living(m, HEALER));
    all
}

/// Majordomo comes to his hall with his guard; his pull binds both kinds
/// with Separation Anxiety; a wipe sends the guard away and calls a fresh
/// one; beaten, he gives his speech and goes to wait in the lair, where his
/// gossip starts the summoning, once. The summoning calls Ragnaros up and
/// ends his immortality; Ragnaros rebukes him and burns him to ash, without
/// a gloat. What follows is [`ragnaros`]'s.
async fn majordomo(realm: &Arc<Realm>) -> (Client, Guid, crate::game::map::MapHandle, Guid) {
    let (mut c, me, map) = hero_in(realm, "Steward", 409, off(HALL, -8.0, 0.0), 110.0, &[]).await;
    map.query(move |m| {
        for g in living(m, RAGNAROS) {
            scripts::despawn(m, g);
        }
    })
    .await;
    let boss = map
        .query(|m| summon(m, MAJORDOMO, HALL, 3.12414))
        .await
        .unwrap();
    c.sees(boss).await;
    let said = heard(&mut c, boss, Duration::from_secs(2)).await;
    assert!(said.is_some(), "his arrival's words");
    assert!(
        until(&map, 2000, |m| living(m, ELITE).len() == 4
            && living(m, HEALER).len() == 4)
        .await,
        "four elites and four healers"
    );
    let first_guard = map.query(|m| guard(m)).await.unwrap();
    map.query(move |m| {
        tough(m, boss);
        ai::attacked(m, boss, me);
    })
    .await;
    assert!(
        until(&map, 2000, move |m| guard(m)
            .iter()
            .all(|g| has(m, *g, 21094)))
        .await,
        "Separation Anxiety on every one of his guard"
    );
    assert!(
        map.query(move |m| has(m, boss, 20620)).await.unwrap(),
        "the Aegis at the pull"
    );
    // A wipe: the guard goes, a fresh one comes.
    map.query(move |m| ai::enter_evade(m, boss)).await;
    assert!(
        until(&map, 3000, move |m| {
            let now = guard(m);
            now.len() == 8 && now.iter().all(|g| !first_guard.contains(g))
        })
        .await,
        "a fresh guard"
    );
    assert_eq!(
        map.query(move |m| event_ai::phase(m, boss)).await,
        Some(0),
        "counting from none again"
    );
    assert!(until(&map, 8000, move |m| at_peace(m, boss)).await);
    // The guard beaten: he yields.
    map.query(move |m| {
        ai::attacked(m, boss, me);
    })
    .await;
    sleep_ms(300).await;
    for g in map.query(|m| guard(m)).await.unwrap() {
        map.query(move |m| kill(m, me, g)).await;
        sleep_ms(30).await;
    }
    assert!(
        until(&map, 2000, move |m| event_ai::phase(m, boss) == 8
            && crate::game::map::instance_script::get_data(m, 8) == 3)
        .await,
        "beaten, his encounter done"
    );
    assert!(until(&map, 8000, move |m| at_peace(m, boss)).await);
    // His speech, then the lair.
    c.clear();
    hurry(&map, boss, 2.5).await;
    assert_eq!(
        map.query(move |m| faction(m, boss)).await,
        Some(1080),
        "he submits"
    );
    hurry(&map, boss, 35.5).await;
    let mut lines = 0;
    while heard(&mut c, boss, Duration::from_millis(300))
        .await
        .is_some()
    {
        lines += 1;
    }
    assert_eq!(lines, 3, "his three lines");
    let (at, phase, gossip) = map
        .query(move |m| {
            (
                m.position_of(boss).unwrap(),
                event_ai::phase(m, boss),
                npc_flags(m, boss) & 1,
            )
        })
        .await
        .unwrap();
    assert!(dist(at, LAIR) < 2.0, "in the lair: {at:?}");
    assert_eq!((phase, gossip), (9, 1), "waiting, to talk");
    assert_eq!(
        map.query(move |m| m.creature(boss).unwrap().home).await,
        Some(Vector3d {
            x: LAIR.x,
            y: LAIR.y,
            z: LAIR.z
        }),
        "home there"
    );
    // His gossip starts the summoning.
    relocate(&map, &mut c, off(LAIR, -3.0, 0.0)).await;
    c.sees(boss).await;
    hello(&c, boss).await;
    let id = option(&mut c, boss, "Tell me more").await;
    choose(&c, boss, id).await;
    let id = option(&mut c, boss, "What else").await;
    choose(&c, boss, id).await;
    let id = option(&mut c, boss, "Where is this master").await;
    choose(&c, boss, id).await;
    assert!(
        until(&map, 2000, move |m| event_ai::phase(m, boss) == 10).await,
        "the summoning begins"
    );
    assert_eq!(
        map.query(move |m| npc_flags(m, boss) & 1).await,
        Some(0),
        "no more to say"
    );
    // Only once: another call changes nothing.
    let runs = move |m: &Map| m.scripts.runs.iter().filter(|r| r.source == boss).count();
    let before = map.query(move |m| runs(m)).await.unwrap();
    map.query(move |m| event_ai::script_event(m, boss, 0, 0, Some(me)))
        .await;
    let after = map.query(move |m| runs(m)).await.unwrap();
    assert_eq!(before, after, "not begun again");
    // Ragnaros rises.
    hurry(&map, boss, 28.2).await;
    let rag = map.query(|m| first(m, RAGNAROS)).await.flatten();
    let rag = rag.expect("Ragnaros called up");
    assert!(
        map.query(move |m| !event_ai::combat_movement(m, rag))
            .await
            .unwrap(),
        "Ragnaros stays where he rose"
    );
    hurry(&map, boss, 47.0).await;
    assert_eq!(
        map.query(move |m| event_ai::invincible_floor(m, boss))
            .await,
        Some(0),
        "Majordomo's immortality ends"
    );
    // Ragnaros's rebuke, and the fire.
    c.clear();
    hurry(&map, rag, 48.2).await;
    assert!(
        until(&map, 3000, move |m| m
            .creature(boss)
            .is_none_or(|c| !c.alive()))
        .await,
        "burned to ash"
    );
    let mut said = Vec::new();
    while let Some(s) = heard(&mut c, rag, Duration::from_millis(300)).await {
        said.push(s);
    }
    assert_eq!(said.len(), 2, "his two lines, no gloat: {said:?}");
    (c, me, map, rag)
}

/// Ragnaros, once he engages: Melt Weapon and Elemental Fire on him; three
/// minutes in he submerges (his first call for servants) and eight Sons of
/// Flame rise; a son falling while he is up changes nothing; the last of
/// them fallen, he emerges; his next submerge calls for minions; a wipe
/// brings him up, ends the cycle, and sends the sons away. Magma Blast goes
/// only with no one in reach of his hammer.
async fn ragnaros(mut c: Client, me: Guid, map: crate::game::map::MapHandle, rag: Guid) {
    // His roar, then he comes for them.
    map.query(|m| hurry_copy(m, Duration::from_secs(8))).await;
    map.query(|m| hurry_copy(m, Duration::from_secs(4))).await;
    assert!(
        until(&map, 3000, move |m| fighting(m, rag)).await,
        "he engages"
    );
    map.query(move |m| tough(m, rag)).await;
    assert!(
        until(&map, 2000, move |m| has(m, rag, 21387)
            && has(m, rag, 20563))
        .await,
        "Melt Weapon and Elemental Fire"
    );
    // Magma Blast: not while one is in reach.
    map.query(move |m| {
        let at = m.position_of(rag).unwrap();
        put(m, me, off(at, 4.0, 0.0));
    })
    .await;
    c.clear();
    fire(&map, rag, 1150207).await;
    assert!(
        cast_of(&mut c, rag, 20565, Duration::from_millis(1600))
            .await
            .is_none(),
        "no Magma Blast with his victim in reach"
    );
    map.query(move |m| {
        let at = m.position_of(rag).unwrap();
        put(m, me, off(at, 30.0, 0.0));
    })
    .await;
    c.clear();
    fire(&map, rag, 1150207).await;
    assert_eq!(
        cast_of(&mut c, rag, 20565, Duration::from_millis(2500)).await,
        Some(me),
        "Magma Blast with none in reach"
    );
    // Three minutes in: he submerges.
    c.clear();
    hurry(&map, rag, 180.5).await;
    assert!(
        until(&map, 2000, move |m| event_ai::phase(m, rag) == 1
            && living(m, SON_OF_FLAME).len() == 8)
        .await,
        "submerged, and eight sons"
    );
    assert_eq!(map.query(move |m| stand_state(m, rag)).await, Some(9));
    let call = heard(&mut c, rag, Duration::from_secs(1)).await.unwrap();
    // The sons fall: he emerges at the last.
    let sons = map.query(|m| living(m, SON_OF_FLAME)).await.unwrap();
    for (i, s) in sons.iter().copied().enumerate() {
        map.query(move |m| kill(m, me, s)).await;
        sleep_ms(60).await;
        let phase = map.query(move |m| event_ai::phase(m, rag)).await.unwrap();
        if i < 7 {
            assert_eq!(phase, 1, "under while sons stand");
        }
    }
    assert!(
        until(&map, 2000, move |m| event_ai::phase(m, rag) == 2
            && stand_state(m, rag) == 0)
        .await,
        "emerged"
    );
    // A son falling while he is up changes nothing.
    let son = map
        .query(move |m| {
            let at = m.position_of(rag).unwrap();
            let g = scripts::summon(m, rag, SON_OF_FLAME, off(at, 5.0, 5.0), 0.0, None).unwrap();
            kill(m, me, g);
            g
        })
        .await
        .unwrap();
    sleep_ms(200).await;
    assert_eq!(
        map.query(move |m| event_ai::phase(m, rag)).await,
        Some(2),
        "{son:?}: still up"
    );
    // His next submerge calls for minions, not servants.
    c.clear();
    hurry(&map, rag, 180.5).await;
    assert!(
        until(&map, 2000, move |m| event_ai::phase(m, rag) == 1).await,
        "under again"
    );
    let again = heard(&mut c, rag, Duration::from_secs(1)).await.unwrap();
    assert_ne!(call, again, "another call");
    // A wipe brings him up and sends the sons away.
    map.query(move |m| ai::enter_evade(m, rag)).await;
    assert!(
        until(&map, 2000, move |m| event_ai::phase(m, rag) == 0
            && stand_state(m, rag) == 0
            && living(m, SON_OF_FLAME).is_empty())
        .await,
        "up, his sons gone"
    );
    let left = map
        .query(move |m| m.scripts.runs.iter().filter(|r| r.source == rag).count())
        .await
        .unwrap();
    assert_eq!(left, 0, "his cycle ends");
}

/// Molten Core's encounters, on one load of the real world.
#[tokio::test]
async fn molten_cores_encounters_run_as_thistle_tea_scripts_them() {
    let Some(realm) = realm() else {
        return;
    };
    if wanted("leash") {
        golemaggs_leash(&realm).await;
    }
    if wanted("majordomo") {
        let (c, me, map, rag) = majordomo(&realm).await;
        ragnaros(c, me, map, rag).await;
    }
}
