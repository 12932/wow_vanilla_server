//! Thistle Tea's Gnomeregan instance script on the real world: Emi's
//! charges and the cave-ins of Grubbis's event, and Thermaplugg's chamber,
//! faces and walking bombs.

use std::time::Duration;

use super::tests_p3_scripts::*;
use super::tests_p4_scripts_dungeons::*;
use crate::game::fields::*;
use crate::game::map::instance_script::{self, DONE, Event, FAILED, IN_PROGRESS};
use crate::game::map::{Map, scripts};

const CHARGES: [u32; 4] = [3997159, 3997160, 3997157, 3997158];
const ROCKETS: [u32; 3] = [283, 284, 285];
const SOUTH_CAVE_IN: u32 = 146086;
const NORTH_CAVE_IN: u32 = 146085;
const FINAL_CHAMBER: u32 = 142207;
const THIRD_FACE: u32 = 142209;
const FIFTH_FACE: u32 = 142213;
const FIFTH_BUTTON: u32 = 142218;
const THERMAPLUGG: u32 = 7800;
const BOMB: u32 = 7915;
const T_BOMB: u32 = 1200;

fn raised(m: &Map, spawns: &[u32]) -> Vec<bool> {
    spawns
        .iter()
        .map(|s| scripts::object_of_spawn(m, *s).is_some())
        .collect()
}

fn locked(m: &Map) -> bool {
    m.gameobjects
        .iter()
        .find(|(_, o)| o.entry == FINAL_CHAMBER)
        .is_some_and(|(_, o)| o.fields.get_u32(GAMEOBJECT_FLAGS) & 0x2 != 0)
}

/// Grubbis's event: each charge Emi plants stands (an hour), and blowing
/// them takes all four; the cave-ins open and shut with their fields;
/// Emi's death fails the event, shutting the cave-ins and taking her
/// charges; Grubbis's death ends it for good and brings back the red
/// rockets, and Emi's death after changes nothing.
#[tokio::test]
#[ignore = "extended"]
async fn gnomeregans_charges_and_cave_ins() {
    let Some(realm) = realm() else {
        return;
    };
    // The real world: the map's script, the charges held back by the cave-in
    // each is planted for, the rockets, Emi's route and the lines.
    let d = &realm.data;
    assert_eq!(
        d.scripts.map_templates[&90].script_name,
        "instance_gnomeregan"
    );
    let objects = &d.gameobject_spawns[&90];
    for (charge, cave_in) in [
        (3997157, NORTH_CAVE_IN),
        (3997158, NORTH_CAVE_IN),
        (3997159, SOUTH_CAVE_IN),
        (3997160, SOUTH_CAVE_IN),
    ] {
        let c = objects.iter().find(|o| o.guid == charge).unwrap();
        assert!(c.despawned, "{charge} held back");
        let by = objects
            .iter()
            .find(|o| o.entry == cave_in)
            .is_some_and(|o| (o.x - c.x).powi(2) + (o.y - c.y).powi(2) < 100.0);
        assert!(by, "{charge} by {cave_in}");
    }
    let mut rockets: Vec<u32> = objects
        .iter()
        .filter(|o| o.entry == 103820)
        .map(|o| o.guid)
        .collect();
    rockets.sort_unstable();
    assert_eq!(rockets, ROCKETS);
    assert!(
        d.scripts.routes.keys().any(|(e, _)| *e == 7998),
        "Emi's route"
    );
    for t in [4050, 4137, 4328, 4446, 6173, 6176] {
        assert!(d.npc.broadcast_texts.contains_key(&t), "{t}");
    }
    let (_c, me, map) = inside(&realm, "Emi", 90, 7998).await;
    map.query(move |m| {
        assert_eq!(raised(m, &CHARGES), vec![false; 4], "held back");
        for (n, s) in CHARGES.iter().enumerate() {
            set(m, 2, n as u32 + 1);
            assert!(
                scripts::object_of_spawn(m, *s).is_some(),
                "charge {}",
                n + 1
            );
        }
        set(m, 2, 5);
        assert_eq!(raised(m, &CHARGES), vec![false; 4], "blown");
        let start = (door(m, SOUTH_CAVE_IN), door(m, NORTH_CAVE_IN));
        assert_eq!(start, (Some(1), Some(1)));
        set(m, 3, 1);
        assert_eq!(door(m, SOUTH_CAVE_IN), Some(0));
        set(m, 3, 0);
        assert_eq!(door(m, SOUTH_CAVE_IN), Some(1));
        // Emi dies with the north cave-in open and her charges planted.
        set(m, 4, 1);
        set(m, 2, 1);
        kill_all(m, me, 7998);
        assert_eq!(data(m, 0), FAILED);
        assert_eq!(door(m, NORTH_CAVE_IN), Some(1), "shut");
        assert_eq!(raised(m, &CHARGES), vec![false; 4], "her charges gone");
    })
    .await
    .unwrap();
    let (_c, me, map) = inside(&realm, "Grubbis", 90, 7998).await;
    map.query(move |m| {
        // The rockets used and gone.
        for s in ROCKETS {
            let g = scripts::object_of_spawn(m, s).expect("a rocket");
            m.remove_gameobject(g);
        }
        assert_eq!(raised(m, &ROCKETS), vec![false; 3]);
        let at = m.position_of(me).unwrap();
        let g = summon(m, 7361, at, 0.0);
        instance_script::creature_event(m, g, Event::Death);
        assert_eq!(data(m, 0), DONE);
        assert_eq!(raised(m, &ROCKETS), vec![true; 3], "the red rockets");
        kill_all(m, me, 7998);
        assert_eq!(data(m, 0), DONE, "for good");
    })
    .await
    .unwrap();
}

/// Thermaplugg: his fight locks the final chamber (and it stays locked as
/// it spawns again) and opens the third face; an open face drops a walking
/// bomb every ten to twenty-five seconds, six about at most; a face's
/// button shuts it and stops its bombs; giving up unlocks the chamber,
/// shuts every face and takes his bombs.
#[tokio::test]
#[ignore = "extended"]
async fn thermaplugg_and_his_bomb_faces() {
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = inside(&realm, "Plugg", 90, THERMAPLUGG).await;
    map.query(move |m| {
        let t = first(m, THERMAPLUGG).unwrap();
        assert!(!locked(m));
        crate::game::map::ai::attacked(m, t, me);
        assert_eq!(data(m, 1), IN_PROGRESS);
        assert!(locked(m), "the chamber locked");
        assert_eq!(door(m, THIRD_FACE), Some(0), "the third face open");
        assert_eq!(data(m, 12), 1);
        instance_script::settle_spawns(m);
        assert!(
            locked(m) && door(m, THIRD_FACE) == Some(0),
            "as they spawn again"
        );
        // Bombs, six at most.
        let before = living(m, BOMB).len();
        hurry_copy(m, Duration::from_millis(3050));
        assert_eq!(living(m, BOMB).len(), before + 1, "a bomb");
        for _ in 0..7 {
            hurry_copy(m, Duration::from_secs(25));
        }
        assert_eq!(living(m, BOMB).len(), before + 6, "six about");
        let b = living(m, BOMB)[0];
        kill(m, me, b);
        hurry_copy(m, Duration::from_secs(25));
        assert_eq!(
            living(m, BOMB).len(),
            before + 6,
            "one more for the one dead"
        );
        // The fifth face, by its button.
        set(m, 14, 1);
        assert_eq!(door(m, FIFTH_FACE), Some(0));
        assert!(m.instance.timers.iter().any(|(_, k)| *k == T_BOMB + 4));
        instance_script::object_used(m, FIFTH_BUTTON);
        assert_eq!(door(m, FIFTH_FACE), Some(1), "shut by its button");
        assert!(!m.instance.timers.iter().any(|(_, k)| *k == T_BOMB + 4));
        // He gives up.
        crate::game::map::ai::enter_evade(m, t);
        assert_eq!(data(m, 1), FAILED);
        assert!(!locked(m), "unlocked");
        assert_eq!(door(m, THIRD_FACE), Some(1), "the faces shut");
        assert!(living(m, BOMB).len() <= before, "his bombs gone");
        assert!(!(0..6).any(|i| m.instance.timers.iter().any(|(_, k)| *k == T_BOMB + i)));
    })
    .await
    .unwrap();
}

const ZF_CREW: [u32; 5] = [7604, 7605, 7606, 7607, 7608];
const ZF_ALIVE: u32 = 10;
const ZF_AT_BASE: u32 = 11;
const ZF_TROLLS: [u32; 7] = [7787, 7788, 7789, 8876, 8877, 7275, 7796];

fn zf_list(m: &Map, key: u32) -> Vec<wow_world_messages::Guid> {
    m.instance.lists.get(&key).cloned().unwrap_or_default()
}

/// Summoned trolls of the pyramid's waves living here.
fn zf_summoned(m: &Map) -> Vec<wow_world_messages::Guid> {
    m.creatures
        .iter()
        .filter(|(_, c)| c.alive() && c.spawn_guid.is_none() && ZF_TROLLS.contains(&c.entry))
        .map(|(_, c)| c.guid)
        .collect()
}

fn home_of(m: &Map, entry: u32) -> Option<(f32, f32)> {
    first(m, entry)
        .and_then(|g| m.creature(g))
        .map(|c| (c.home.x, c.home.y))
}

/// Zul'Farrak's pyramid: a cage opened frees the crew to the top of the
/// stairs, once; Weegli at the stairs gathers the first wave of
/// twenty-three below (once), each troll in it once it has come; groups
/// go up the stairs every ten seconds, larger each time, none slain sent;
/// a database troll joins no wave, and a wave isn't cleared while one of
/// it lives; each cleared wave brings the next (the second ten seconds
/// on, the third with its leaders at once and the crew down the stairs
/// five after), and the last sends the crew to the floor. The end door
/// blown stays open, and Gahz'rilla rises once a copy.
#[tokio::test]
#[ignore = "extended"]
async fn zul_farraks_pyramid_waves() {
    const T_SEND: u32 = 1300;
    const T_WAVE_2: u32 = 1301;
    const T_WAVE_3: u32 = 1302;
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = inside(&realm, "Pyramid", 209, 7607).await;
    map.query(|m| {
        instance_script::object_used(m, 141072);
        assert_eq!(data(m, 1), 1);
        assert_eq!(home_of(m, 7607), Some((1883.87, 1263.49)), "Weegli up top");
        for e in ZF_CREW {
            let g = first(m, e).unwrap();
            assert_eq!(faction(m, g), 250, "{e} freed");
        }
        instance_script::object_used(m, 141070);
        assert_eq!(data(m, 1), 1, "once");
        set(m, 1, 2);
        assert_eq!(data(m, 1), 3, "the first wave");
        set(m, 1, 2);
    })
    .await
    .unwrap();
    assert!(
        until(&map, 3000, |m| zf_list(m, ZF_ALIVE).len() == 23).await,
        "twenty-three come"
    );
    map.query(move |m| {
        assert_eq!(zf_summoned(m).len(), 23, "once");
        // A database troll joins none.
        let db = m
            .creatures
            .iter()
            .find(|(_, c)| c.alive() && c.spawn_guid.is_some() && c.entry == 7789)
            .map(|(_, c)| c.guid);
        if let Some(db) = db {
            instance_script::creature_event(m, db, Event::Spawned);
            assert_eq!(zf_list(m, ZF_ALIVE).len(), 23);
            kill(m, me, db);
            assert_eq!(zf_list(m, ZF_ALIVE).len(), 23);
        }
        // The first slain is never sent.
        let base = zf_list(m, ZF_AT_BASE);
        kill(m, me, base[0]);
        hurry_copy(m, Duration::from_millis(1050));
        assert_eq!(zf_list(m, ZF_AT_BASE), base[3..], "two sent, the slain not");
        hurry_copy(m, Duration::from_millis(10_050));
        assert_eq!(zf_list(m, ZF_AT_BASE), base[6..], "then three");
        // One left: not cleared.
        let alive = zf_list(m, ZF_ALIVE);
        let (last, rest) = alive.split_last().unwrap();
        for g in rest {
            kill(m, me, *g);
        }
        assert_eq!(data(m, 1), 3, "one lives");
        kill(m, me, *last);
        assert_eq!(data(m, 1), 4, "cleared");
        assert!(!m.instance.timers.iter().any(|(_, k)| *k == T_SEND));
        assert!(m.instance.timers.iter().any(|(_, k)| *k == T_WAVE_2));
        hurry_copy(m, Duration::from_millis(10_050));
        assert_eq!(data(m, 1), 5, "the second");
    })
    .await
    .unwrap();
    assert!(until(&map, 3000, |m| zf_list(m, ZF_ALIVE).len() == 24).await);
    map.query(move |m| {
        for g in zf_list(m, ZF_ALIVE) {
            kill(m, me, g);
        }
        assert_eq!(data(m, 1), 6);
        assert_eq!(
            living(m, 7275).len() + living(m, 7796).len(),
            2,
            "its leaders"
        );
        assert!(m.instance.timers.iter().any(|(_, k)| *k == T_WAVE_3));
        hurry_copy(m, Duration::from_millis(5050));
        assert_eq!(data(m, 1), 7, "the crew comes down");
        assert_eq!(home_of(m, 7607), Some((1878.02, 1227.65)));
    })
    .await
    .unwrap();
    assert!(until(&map, 3000, |m| zf_list(m, ZF_ALIVE).len() == 7).await);
    map.query(move |m| {
        for g in zf_list(m, ZF_ALIVE) {
            kill(m, me, g);
        }
        assert_eq!(data(m, 1), 8, "all dead");
        assert_eq!(home_of(m, 7607), Some((1877.52, 1199.63)), "to the floor");
        // The end door.
        assert_eq!(door(m, 146084), Some(1));
        set(m, 3, DONE);
        assert_eq!(door(m, 146084), Some(0));
        assert_eq!(respawned(m, &[146084]), vec![Some(0)]);
        set(m, 3, 0);
        assert_eq!(data(m, 3), DONE, "for good");
        // Gahz'rilla.
        let at = m.position_of(me).unwrap();
        summon(m, 7273, at, 0.0);
    })
    .await
    .unwrap();
    assert!(
        until(&map, 3000, |m| data(m, 2) == IN_PROGRESS).await,
        "he rises"
    );
    map.query(move |m| {
        let at = m.position_of(me).unwrap();
        summon(m, 7273, at, 0.0);
    })
    .await;
    assert!(
        until(&map, 3000, |m| living(m, 7273).len() == 1).await,
        "once a copy"
    );
}
