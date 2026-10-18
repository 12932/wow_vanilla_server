//! Thistle Tea's Blackrock Depths instance script on the real world: the
//! Lyceum's braziers and Magmus, the Tomb of the Seven, and the Ring of
//! Law.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use super::tests_p3_scripts::*;
use super::tests_p4_scripts_dungeons::*;
use crate::game::map::instance_script::{self, DONE, Event, FAILED, IN_PROGRESS};
use crate::game::map::{Map, MapHandle, scripts};

const BRAZIERS: [u32; 2] = [174744, 174745];
const GOLEM_DOORS: [u32; 2] = [170573, 170574];
const THRONE_ROOM: u32 = 170575;
const TOMB_ENTER: u32 = 170576;
const TOMB_EXIT: u32 = 170577;
const MAGMUS: u32 = 9938;
const SEVEN: [u32; 7] = [9035, 9038, 9040, 9037, 9036, 9034, 9039];
const GRIMSTONE: u32 = 10096;
const BEAST_GATE: u32 = 161525;
const CHAMPION_GATE: u32 = 161522;
const EXIT_GATE: u32 = 161524;
const JAIL_GATE: u32 = 161523;
const T_TOMB: u32 = 1600;
const T_CHAMPION: u32 = 1610;
const T_FINISH: u32 = 1611;
/// The ring's middle, where its beasts and champions are summoned here.
const RING: Vector3d = Vector3d {
    x: 595.0,
    y: -188.0,
    z: -54.0,
};

fn doors_of(m: &Map, entries: &[u32]) -> Vec<Option<u32>> {
    entries.iter().map(|e| door(m, *e)).collect()
}

fn waits(m: &Map, key: u32) -> bool {
    m.instance.timers.iter().any(|(_, k)| *k == key)
}

/// `n` of `entry` summoned into the ring, and in its list once their
/// spawned events have run.
async fn into_ring(map: &MapHandle, entry: u32, n: usize, list: u32) -> Vec<Guid> {
    let gs = map
        .query(move |m| {
            (0..n)
                .map(|i| summon(m, entry, off(RING, i as f32, 0.0), 0.0))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    let want = gs.clone();
    assert!(
        until(map, 3000, move |m| m
            .instance
            .lists
            .get(&list)
            .is_some_and(|l| want.iter().all(|g| l.contains(g))))
        .await,
        "{entry} in the ring's list"
    );
    gs
}

/// The Lyceum: players can't work the depths' own doors (but the
/// braziers); one brazier twice is half the Lyceum, both open the Golem
/// Room (and they stay open as they spawn again), and a Seven dwarf walking
/// home doesn't undo it; Magmus shuts the Golem Room while he fights,
/// opens it as he gives up, and dying opens the Throne Room, which stays
/// shut till then.
#[tokio::test]
#[ignore = "extended"]
async fn the_lyceum_and_magmus() {
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = inside(&realm, "Lyceum", 230, MAGMUS).await;
    map.query(move |m| {
        for (e, scripted) in [
            (GOLEM_DOORS[0], true),
            (GOLEM_DOORS[1], true),
            (THRONE_ROOM, true),
            (TOMB_ENTER, true),
            (TOMB_EXIT, true),
            (BRAZIERS[0], false),
        ] {
            let g = m
                .gameobjects
                .iter()
                .find(|(_, o)| o.entry == e)
                .map(|(_, o)| o.guid)
                .unwrap_or_else(|| panic!("{e}"));
            assert_eq!(instance_script::scripted_door(m, g), scripted, "{e}");
        }
        assert_eq!(door(m, THRONE_ROOM), Some(1), "the Throne Room shut");
        assert_eq!(respawned(m, &[THRONE_ROOM]), vec![Some(1)]);
        let golem = doors_of(m, &GOLEM_DOORS);
        instance_script::object_used(m, BRAZIERS[0]);
        instance_script::object_used(m, BRAZIERS[0]);
        assert_eq!(data(m, 4), IN_PROGRESS);
        assert_eq!(doors_of(m, &GOLEM_DOORS), golem, "one brazier");
        instance_script::object_used(m, BRAZIERS[1]);
        assert_eq!(data(m, 4), DONE);
        assert_eq!(doors_of(m, &GOLEM_DOORS), vec![Some(0); 2], "both");
        instance_script::object_used(m, BRAZIERS[0]);
        set(m, 4, FAILED);
        assert_eq!(data(m, 4), DONE, "a dwarf walking home undoes nothing");
        assert_eq!(respawned(m, &GOLEM_DOORS), vec![Some(0); 2]);
        // Magmus.
        let magmus = first(m, MAGMUS).unwrap();
        crate::game::map::ai::attacked(m, magmus, me);
        assert_eq!(
            doors_of(m, &GOLEM_DOORS),
            vec![Some(1); 2],
            "shut in his fight"
        );
        crate::game::map::ai::enter_evade(m, magmus);
        assert_eq!(doors_of(m, &GOLEM_DOORS), vec![Some(0); 2], "his evade");
        instance_script::creature_event(m, magmus, Event::Aggro);
        assert_eq!(doors_of(m, &GOLEM_DOORS), vec![Some(1); 2]);
        instance_script::creature_event(m, magmus, Event::Death);
        assert_eq!(doors_of(m, &GOLEM_DOORS), vec![Some(0); 2]);
        assert_eq!(door(m, THRONE_ROOM), Some(0), "his death");
        assert_eq!(respawned(m, &[THRONE_ROOM]), vec![Some(0)]);
    })
    .await
    .unwrap();
}

/// The Tomb of the Seven: a challenge seals it and calls each dwarf in
/// turn (out of immunity, hostile), thirty seconds apart; Doom'rel's death
/// opens both doors, raises the Chest of the Seven and settles it. A called
/// dwarf giving up stands them all down and calls off the rest, the tomb
/// open, to be challenged again; a dwarf out of a fight with no challenge
/// changes nothing.
#[tokio::test]
#[ignore = "extended"]
async fn the_tomb_of_the_seven() {
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = inside(&realm, "Tomb", 230, 9039).await;
    map.query(move |m| {
        assert!(SEVEN.iter().all(|e| first(m, *e).is_some()), "the Seven");
        assert!(
            scripts::object_of_spawn(m, 399065).is_none(),
            "the chest held"
        );
        // Out of a challenge.
        let before = m.instance.data.clone();
        let d = first(m, SEVEN[0]).unwrap();
        instance_script::creature_event(m, d, Event::Evade);
        assert_eq!(m.instance.data, before, "nothing");
        assert_eq!(door(m, TOMB_ENTER), Some(0));
        // The challenge.
        set(m, 3, IN_PROGRESS);
        assert_eq!(door(m, TOMB_ENTER), Some(1), "sealed");
        for (round, dwarf) in SEVEN.iter().enumerate() {
            let g = first(m, *dwarf).unwrap();
            let immune = |m: &Map| unit_flags(m, g) & 0x100 != 0;
            hurry_copy(
                m,
                Duration::from_millis(if round == 0 { 50 } else { 29_000 }),
            );
            if round > 0 {
                assert!(immune(m), "{dwarf} not yet");
                hurry_copy(m, Duration::from_millis(1050));
            }
            assert!(!immune(m), "{dwarf} called");
            assert_eq!(faction(m, g), 54);
            assert_eq!(waits(m, T_TOMB + round as u32 + 1), round < 6);
        }
        kill_all(m, me, 9039);
        assert_eq!(data(m, 3), DONE);
        assert_eq!(doors_of(m, &[TOMB_EXIT, TOMB_ENTER]), vec![Some(0); 2]);
        assert!(scripts::object_of_spawn(m, 399065).is_some(), "the chest");
        assert_eq!(respawned(m, &[TOMB_EXIT]), vec![Some(0)]);
    })
    .await
    .unwrap();
    // A dwarf gives up.
    let (_c, _, map) = inside(&realm, "Giver", 230, 9039).await;
    map.query(|m| {
        set(m, 3, IN_PROGRESS);
        hurry_copy(m, Duration::from_millis(50));
        let g = first(m, SEVEN[0]).unwrap();
        instance_script::creature_event(m, g, Event::Evade);
        assert_eq!(data(m, 3), 0);
        assert_eq!(door(m, TOMB_ENTER), Some(0), "the tomb open");
        assert!(!(0..7).any(|i| waits(m, T_TOMB + i)), "the rest called off");
        for e in SEVEN {
            let d = first(m, e).unwrap();
            assert_ne!(unit_flags(m, d) & 0x100, 0, "{e} stood down");
        }
        set(m, 3, IN_PROGRESS);
        assert_eq!(data(m, 3), IN_PROGRESS, "challenged again");
    })
    .await
    .unwrap();
}

/// The Ring of Law: stepping in shuts the jail gate and brings out
/// Grimstone, who passes sentence and walks his path (once); his stages
/// open the beast gate, then the champion's; eight beasts slain send him
/// on, ten seconds after, to the champion's gate; the champion's death
/// has him finish five after; the ring won opens the exit and the jail and
/// calms the crowd, and a spectator spawning after is calm too. A wipe
/// before the champion gives the ring back as it was; one against the
/// champion leaves it waiting till they step back in.
#[tokio::test]
#[ignore = "extended"]
async fn the_ring_of_law() {
    const MOBS: u32 = 1;
    const CHAMPION: u32 = 2;
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = inside(&realm, "Ring", 230, JAIL_GATE).await;
    relocate(&map, &mut c, RING).await;
    map.query(|m| {
        assert_eq!(
            doors_of(m, &[JAIL_GATE, BEAST_GATE, CHAMPION_GATE, EXIT_GATE]),
            vec![Some(0), Some(1), Some(1), Some(1)],
            "only the jail gate open"
        );
        set(m, 0, IN_PROGRESS);
        assert_eq!(door(m, JAIL_GATE), Some(1), "the jail shut");
        assert_eq!(living(m, GRIMSTONE).len(), 1, "Grimstone");
        set(m, 0, IN_PROGRESS);
        assert_eq!(living(m, GRIMSTONE).len(), 1, "once");
        hurry_copy(m, Duration::from_millis(1050));
        let g = first(m, GRIMSTONE).unwrap();
        assert_eq!(
            m.creature(g).map(|c| c.movement_type),
            Some(crate::game::map::creature::MOVEMENT_WAYPOINT),
            "his path"
        );
        set(m, 46, 1);
        assert_eq!(
            doors_of(m, &[BEAST_GATE, CHAMPION_GATE]),
            vec![Some(0), Some(1)]
        );
    })
    .await
    .unwrap();
    let beasts = into_ring(&map, 8925, 8, MOBS).await;
    map.query(move |m| {
        for b in &beasts[..7] {
            kill(m, me, *b);
        }
        assert!(!waits(m, T_CHAMPION));
        kill(m, me, beasts[7]);
        assert!(waits(m, T_CHAMPION), "eight slain");
        hurry_copy(m, Duration::from_millis(10_050));
        assert_eq!(data(m, 46), 0);
        assert_eq!(door(m, BEAST_GATE), Some(1), "the beast gate shut");
        set(m, 46, 2);
        assert_eq!(
            doors_of(m, &[BEAST_GATE, CHAMPION_GATE]),
            vec![Some(1), Some(0)]
        );
    })
    .await
    .unwrap();
    let champion = into_ring(&map, 9031, 1, CHAMPION).await[0];
    map.query(move |m| {
        kill(m, me, champion);
        assert!(waits(m, T_FINISH));
        hurry_copy(m, Duration::from_millis(5050));
        set(m, 0, DONE);
        assert_eq!(doors_of(m, &[EXIT_GATE, JAIL_GATE]), vec![Some(0); 2]);
        let stands = Vector3d {
            x: 595.78,
            y: -188.65,
            z: -35.5,
        };
        let crowd: Vec<Guid> = [8916, 8896, 8902, 8904, 8893, 8894, 8895]
            .iter()
            .flat_map(|e| living(m, *e))
            .filter(|g| {
                m.position_of(*g)
                    .is_some_and(|p| crate::game::map::combat::dist(p, stands) <= 69.0)
            })
            .collect();
        assert!(!crowd.is_empty());
        assert!(crowd.iter().all(|g| faction(m, *g) == 15), "the crowd calm");
        let beyond: Vec<Guid> = [8916, 8896, 8902, 8904, 8893, 8894, 8895]
            .iter()
            .flat_map(|e| living(m, *e))
            .filter(|g| {
                m.position_of(*g)
                    .is_some_and(|p| crate::game::map::combat::dist(p, stands) > 75.0)
            })
            .collect();
        assert!(
            beyond.iter().all(|g| faction(m, *g) != 15),
            "only those in the stands"
        );
        let late = summon(m, 8916, stands, 0.0);
        instance_script::creature_event(m, late, Event::Spawned);
        assert_eq!(faction(m, late), 15, "one spawning after too");
    })
    .await
    .unwrap();
    // A wipe before the champion.
    let (mut c, _, map) = inside(&realm, "Wipe", 230, JAIL_GATE).await;
    relocate(&map, &mut c, RING).await;
    map.query(|m| {
        set(m, 0, IN_PROGRESS);
        set(m, 46, 1);
    })
    .await;
    let beasts = into_ring(&map, 8927, 3, MOBS).await;
    map.query(move |m| {
        instance_script::creature_event(m, beasts[1], Event::Evade);
        assert_eq!(data(m, 0), 0);
        assert_eq!(
            doors_of(m, &[BEAST_GATE, JAIL_GATE]),
            vec![Some(1), Some(0)]
        );
        assert!(living(m, 8927).is_empty() && living(m, GRIMSTONE).is_empty());
        let before = m.instance.data.clone();
        instance_script::creature_event(m, beasts[2], Event::Evade);
        assert_eq!(m.instance.data, before, "again, nothing");
    })
    .await
    .unwrap();
    // A wipe against the champion.
    let (mut c, _, map) = inside(&realm, "Champ", 230, JAIL_GATE).await;
    relocate(&map, &mut c, RING).await;
    map.query(|m| set(m, 0, IN_PROGRESS)).await;
    let champion = into_ring(&map, 9028, 1, CHAMPION).await[0];
    map.query(move |m| {
        instance_script::creature_event(m, champion, Event::Evade);
        assert_eq!(data(m, 0), FAILED, "waiting");
        assert_eq!(door(m, JAIL_GATE), Some(0));
        set(m, 0, IN_PROGRESS);
        assert_eq!(door(m, JAIL_GATE), Some(1), "back in");
        assert!(fighting(m, champion), "the champion turns on them");
    })
    .await
    .unwrap();
}

/// The real depths as their script wants them: named for it; each door,
/// brazier and rune it works spawned; the Ring's crowd (54 spectators in
/// the stands' sphere), Grimstone's path and lines; Magmus's warning; and
/// Doom'rel's challenge, through his gossip, starting the Tomb of the
/// Seven.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_depths_and_doomrels_challenge() {
    let Some(realm) = realm() else {
        return;
    };
    let d = &realm.data;
    assert_eq!(
        d.scripts.map_templates[&230].script_name,
        "instance_blackrock_depths"
    );
    let mut doors: Vec<u32> = vec![170573, 170574, 170575, 170576, 170577, 174744, 174745];
    doors.extend(170578..=170584);
    for e in &doors {
        assert!(
            d.gameobject_spawns[&230].iter().any(|s| s.entry == *e),
            "{e} spawned"
        );
    }
    let crowd = [8893, 8894, 8895, 8896, 8902, 8904, 8916];
    let in_stands = d.creature_spawns[&230]
        .iter()
        .filter(|s| s.entries.iter().any(|e| crowd.contains(e)))
        .filter(|s| {
            (s.x - 595.78).powi(2) + (s.y + 188.65).powi(2) + (s.z + 35.5).powi(2) <= 4761.0
        })
        .count();
    assert_eq!(in_stands, 54, "the crowd in its stands");
    assert!(
        d.scripts.routes.keys().any(|(e, _)| *e == GRIMSTONE),
        "Grimstone's path"
    );
    for t in (5441..=5446).chain([5430]) {
        assert!(d.npc.broadcast_texts.contains_key(&t), "text {t}");
    }
    // Doom'rel's challenge.
    let (mut c, _, map) = inside(&realm, "Challenger", 230, 9039).await;
    let doomrel = alive(&map, 9039).await[0];
    c.sees(doomrel).await;
    let mut started = false;
    for _ in 0..3 {
        c.clear();
        hello(&c, doomrel).await;
        let options = c
            .wait_for("Doom'rel's menu", |m| match m {
                wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(
                    g,
                ) if g.guid == doomrel => Some(g.gossips.iter().map(|o| o.id).collect::<Vec<_>>()),
                _ => None,
            })
            .await;
        let Some(last) = options.last() else {
            break;
        };
        choose(&c, doomrel, *last).await;
        if until(&map, 1500, |m| data(m, 3) == IN_PROGRESS).await {
            started = true;
            break;
        }
    }
    assert!(started, "the challenge starts the tomb");
    assert_eq!(map.query(|m| door(m, TOMB_ENTER)).await.flatten(), Some(1));
}
