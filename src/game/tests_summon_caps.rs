//! A flight master's guardians: its aggro event (vmangos's
//! `creature_ai_events`, once a fight) calls two Enraged Gryphons (or
//! Wyverns, Hippogryphs, Felbats), and no more than two are ever alive,
//! however often it is attacked (decision 101).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::scripts::{AiEvent, EVENT_AGGRO};
use crate::game::harness::Client;
use crate::game::map::{Map, MapHandle, ai, event_ai};
use crate::game::tests_p3_combat::{data, setup_with, spawn, template};
use crate::game::tests_p3_entity::{on, step};
use crate::game::tests_p3_entity_objects::until;

/// A flight master (Dungar Longdrink's script) and its gryphon; one whose
/// guardians last 30 s past their fight (Michael Garrett's), and its bat.
const MASTER: u32 = 4_352;
const GRYPHON: u32 = 4_526;
const BAT_MASTER: u32 = 4_551;
const FELBAT: u32 = 4_521;
const SCRIPT: u32 = 435_201;
const BAT_SCRIPT: u32 = 455_101;

/// vmangos's rows: two summon steps on aggro, each attacking its
/// summoner's victim (target type 1); the gryphons with no time
/// (`TEMPSUMMON_TIMED_DESPAWN_OUT_OF_COMBAT`, 0 ms), the bats 30 s
/// (`TEMPSUMMON_TIMED_OR_DEAD_DESPAWN`).
fn world() -> GameData {
    let mut d = data();
    for (entry, summon, script, delay, kind) in [
        (MASTER, GRYPHON, SCRIPT, 0, 4),
        (BAT_MASTER, FELBAT, BAT_SCRIPT, 30_000, 1),
    ] {
        d.creature_templates.insert(entry, template(entry, 7));
        d.creature_templates.insert(summon, template(summon, 1));
        d.scripts.ai_events.insert(
            entry,
            vec![AiEvent {
                id: entry * 100 + 1,
                creature: entry,
                kind: EVENT_AGGRO,
                chance: 100,
                actions: [script, 0, 0],
                ..Default::default()
            }],
        );
        let summon_step = crate::game::data::scripts::ScriptStep {
            id: script,
            dataint: [0, 0, 1, kind],
            ..step(10, [summon, delay, 0, 0])
        };
        d.scripts
            .ai_scripts
            .insert(script, vec![summon_step.clone(), summon_step]);
    }
    d
}

/// The living creatures of `entry` that `master` summoned.
fn guardians(m: &Map, master: Guid, entry: u32) -> Vec<Guid> {
    let mut out: Vec<Guid> = m
        .creatures
        .iter()
        .map(|(_, c)| c)
        .filter(|c| c.entry == entry && c.alive())
        .filter(|c| c.scripted.as_deref().and_then(|s| s.summoner) == Some(master))
        .map(|c| c.guid)
        .collect();
    out.sort();
    out
}

/// A master of `entry` in front of Alice, who takes no harm from it or
/// its guardians (`.god`), so the fights last as long as the test.
fn harmless(m: &mut Map, ga: Guid, entry: u32) -> Guid {
    m.players.get_mut(&ga).expect("Alice").misc.god = true;
    spawn(m, ga, entry, 20, 2.0)
}

/// Alice swings at `master` (again) once it is home from any evade, until
/// it is in its fight: a swing sent as her last fight ends is stopped with
/// it, as a client's is.
async fn attack(a: &mut Client, map: &MapHandle, master: Guid) {
    until(map, "home from its evade", move |m| {
        m.creature(master).is_some_and(|c| !c.evading)
    })
    .await;
    let fighting = move |m: &Map| m.creature(master).is_some_and(|c| c.combat.attacking);
    let deadline = tokio::time::Instant::now() + crate::game::harness::WAIT;
    let ga = a.guid;
    loop {
        // Her weapon's timer skipped, not waited out.
        on(map, move |m| {
            if let Some(c) = m.combat_mut(ga) {
                c.next_swing = None;
            }
        })
        .await;
        a.send(CMSG_ATTACKSWING { guid: master }).await;
        for _ in 0..15 {
            if on(map, move |m| fighting(m)).await {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(tokio::time::Instant::now() < deadline, "in its fight");
    }
}

/// The fight is over: each of `units` evades, Alice (`ga`) is out of
/// hers, and the despawns due are run.
fn evade(m: &mut Map, ga: Guid, units: &[Guid]) {
    for g in units {
        ai::enter_evade(m, *g);
    }
    crate::game::map::combat::leave_combat(m, ga);
    event_ai::update_despawns(m);
}

/// Attacked, Dungar calls two gryphons, which Alice is told of. The fight
/// over, they are gone (vmangos's out-of-combat despawn with no time), and
/// a new fight calls two more. With him gone home and pulled again while
/// his gryphons fight on, he calls none: two still. One killed goes at
/// once, and his next fight calls one to stand with the other: two again.
#[tokio::test]
async fn a_flight_masters_gryphons_are_two_however_often_it_is_attacked() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let master = on(&map, move |m| harmless(m, ga, MASTER)).await;
    a.sees(master).await;
    let count = move |m: &Map| guardians(m, master, GRYPHON);

    attack(&mut a, &map, master).await;
    let first = on(&map, move |m| count(m)).await;
    assert_eq!(first.len(), 2, "two gryphons: {first:?}");
    for g in &first {
        a.sees(*g).await;
    }

    // The fight over: they go with it.
    let (one, two) = (first[0], first[1]);
    let left = on(&map, move |m| {
        evade(m, ga, &[master, one, two]);
        count(m)
    })
    .await;
    assert!(left.is_empty(), "gone with the fight: {left:?}");
    for g in &first {
        a.forgets(*g).await;
    }

    // A new fight: two new ones.
    attack(&mut a, &map, master).await;
    let second = on(&map, move |m| count(m)).await;
    assert_eq!(second.len(), 2, "two again: {second:?}");
    assert!(second.iter().all(|g| !first.contains(g)), "new ones");

    // He alone goes home, his gryphons fighting on, and is pulled again.
    on(&map, move |m| ai::enter_evade(m, master)).await;
    attack(&mut a, &map, master).await;
    let third = on(&map, move |m| count(m)).await;
    assert_eq!(third, second, "the same two, no more");

    // One killed goes at once; the next fight calls one beside the other.
    let (dead, kept) = (second[0], second[1]);
    let gone = on(&map, move |m| {
        ai::die(m, dead, ga);
        event_ai::update_despawns(m);
        m.creature(dead).is_none()
    })
    .await;
    assert!(gone, "a dead gryphon goes at once");
    on(&map, move |m| ai::enter_evade(m, master)).await;
    attack(&mut a, &map, master).await;
    let fourth = on(&map, move |m| count(m)).await;
    assert_eq!(fourth.len(), 2, "two: {fourth:?}");
    assert!(fourth.contains(&kept), "the one left, and one new");
    assert!(!fourth.contains(&dead));
}

/// Michael Garrett's felbats last 30 s past their fight: pulled again
/// within it, he calls none, and the two he has come back to the fight;
/// 30 s out of it, they go.
#[tokio::test]
async fn felbats_waiting_out_their_time_are_called_back_not_called_again_summon() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let master = on(&map, move |m| harmless(m, ga, BAT_MASTER)).await;
    a.sees(master).await;
    let count = move |m: &Map| guardians(m, master, FELBAT);

    attack(&mut a, &map, master).await;
    let first = on(&map, move |m| count(m)).await;
    assert_eq!(first.len(), 2, "two felbats: {first:?}");

    let (one, two) = (first[0], first[1]);
    let left = on(&map, move |m| {
        evade(m, ga, &[master, one, two]);
        count(m)
    })
    .await;
    assert_eq!(left, first, "they wait out their 30 s");

    attack(&mut a, &map, master).await;
    let (again, fighting) = on(&map, move |m| {
        let fighting: Vec<bool> = [one, two]
            .iter()
            .map(|g| m.creature(*g).is_some_and(|c| c.combat.victim == ga))
            .collect();
        (count(m), fighting)
    })
    .await;
    assert_eq!(again, first, "the same two, no more");
    assert_eq!(fighting, [true, true], "back in the fight");

    let left = on(&map, move |m| {
        evade(m, ga, &[master, one, two]);
        m.now += std::time::Duration::from_secs(31);
        event_ai::update_despawns(m);
        count(m)
    })
    .await;
    assert!(left.is_empty(), "gone 30 s out of the fight: {left:?}");
}

/// vmangos's own rows (extended): Dungar Longdrink, pulled by a Horde
/// warrior, calls two Enraged Gryphons, which go with the fight, and two
/// more for the next; Vesprystus's two Enraged Hippogryphs wait out their
/// 30 s, and his next fight brings them back rather than two more.
#[tokio::test]
#[ignore = "extended"]
async fn vmangos_flight_masters_summon_two_guardians_a_fight() {
    const DUNGAR: u32 = 352;
    const ENRAGED_GRYPHON: u32 = 9526;
    const VESPRYSTUS: u32 = 3838;
    const ENRAGED_HIPPOGRYPH: u32 = 9527;
    let Some(realm) = super::tests_p3_scripts::realm() else {
        return;
    };
    let mut a = Client::connect(&realm, "Wings");
    let ga = a.enter("Wings", Race::Orc, Class::Warrior).await;
    let key = realm.registry.online(ga).expect("online").map().key;
    let map = realm.maps.get(&realm, key);
    for (entry, guardian, lasting) in [
        (DUNGAR, ENRAGED_GRYPHON, false),
        (VESPRYSTUS, ENRAGED_HIPPOGRYPH, true),
    ] {
        let master = on(&map, move |m| harmless(m, ga, entry)).await;
        a.sees(master).await;
        let count = move |m: &Map| guardians(m, master, guardian);
        attack(&mut a, &map, master).await;
        let first = on(&map, move |m| count(m)).await;
        assert_eq!(first.len(), 2, "{entry}: two guardians: {first:?}");
        let (one, two) = (first[0], first[1]);
        let left = on(&map, move |m| {
            evade(m, ga, &[master, one, two]);
            count(m)
        })
        .await;
        if lasting {
            assert_eq!(left, first, "{entry}: they wait out their time");
        } else {
            assert!(left.is_empty(), "{entry}: gone with the fight: {left:?}");
        }
        attack(&mut a, &map, master).await;
        let again = on(&map, move |m| count(m)).await;
        assert_eq!(again.len(), 2, "{entry}: two again: {again:?}");
        assert_eq!(
            again == first,
            lasting,
            "{entry}: the same ones if they wait"
        );
        on(&map, move |m| {
            for g in [master, one, two] {
                crate::game::map::scripts::despawn(m, g);
            }
            for g in count(m) {
                crate::game::map::scripts::despawn(m, g);
            }
            crate::game::map::combat::leave_combat(m, ga);
        })
        .await;
    }
}
