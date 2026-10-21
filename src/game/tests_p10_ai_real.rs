//! Round 10's creature AI tests on the real world (extended), ported from
//! Thistle Tea's `test/game/core/ai/event_ai_test.exs`
//! (docs/parity/matrix.csv): a copy's script hears its creatures' spawn,
//! pull, evade and death, one with no EventAI rows too.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use super::tests_p4_scripts_dungeons::*;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::instance_script::{DONE, IN_PROGRESS, NOT_STARTED};
use crate::game::map::{Map, ai, event_ai};
use crate::game::wind_stones::{self, CRIMSON_TEMPLAR, DUKE_OF_CYNDERS, LORD_SKWOL, TEMPLARS};

/// Thistle Tea's `on_spawned/3` "enqueues exact-copy creature lifecycle
/// facts without EventAI rows": Golemagg, his events taken away, tells
/// his Molten Core copy's script of his spawn (his fight reset), his pull
/// (in progress), his evade (reset) and his death (done), each as it
/// happens.
#[tokio::test]
#[ignore = "extended"]
async fn a_copy_hears_its_creatures_spawn_pull_evade_and_death_without_eventai() {
    const GOLEMAGG: u32 = 11988;
    const FIELD: u32 = 3;
    let dir = crate::game::harness::real_data("db");
    if !dir.join("vmangos.sqlite").exists() {
        return;
    }
    // His events (Thistle Tea's port of his script) taken away.
    let mut d = crate::game::harness::load_real(&dir);
    d.scripts.ai_events.remove(&GOLEMAGG);
    let realm = crate::game::harness::realm_with(d);
    assert!(!event_ai::has_events_entry(&realm.data, GOLEMAGG));
    let (_c, me, map) = inside(&realm, "Lifecycle", 409, GOLEMAGG).await;
    let out = map
        .query(move |m| {
            let g = first(m, GOLEMAGG).expect("Golemagg");
            set(m, FIELD, IN_PROGRESS);
            ai::quiet_death(m, g);
            ai::respawn_now(m, g);
            event_ai::update_despawns(m);
            let g = first(m, GOLEMAGG).expect("Golemagg again");
            let spawned = data(m, FIELD);
            ai::attacked(m, g, me);
            let pulled = data(m, FIELD);
            ai::enter_evade(m, g);
            let evaded = data(m, FIELD);
            ai::attacked(m, g, me);
            kill(m, me, g);
            (spawned, pulled, evaded, data(m, FIELD))
        })
        .await
        .unwrap();
    assert_eq!(out, (NOT_STARTED, IN_PROGRESS, NOT_STARTED, DONE));
}

/// A lesser wind stone in Silithus.
const STONE_AT: Vector3d = Vector3d {
    x: -7_959.77,
    y: 1_824.89,
    z: 3.53474,
};

/// The living creatures of `entries` within a yard of `at`.
fn called_at(m: &Map, entries: &[u32], at: Vector3d) -> Vec<(u32, Guid)> {
    m.creatures
        .iter()
        .filter(|(_, c)| c.alive() && entries.contains(&c.entry))
        .filter(|(_, c)| crate::game::map::combat::dist(c.position, at) < 1.0)
        .map(|(_, c)| (c.entry, c.guid))
        .collect()
}

/// Thistle Tea's `activated/3` "leaves spells no script claims to the
/// object action", "a lesser wind stone calls a random templar in its
/// place and vanishes", "a crest, signet, or scepter calls the lord of its
/// element" and "the summoned lord faces its summoner, denounces them,
/// then attacks after eight seconds" (vmangos's `go_wind_stone`): Summon
/// Templar at a lesser wind stone calls one of the four templars where the
/// stone stood, the stone gone; a crest's calls the Crimson Templar, a
/// signet's at a wind stone the Duke of Cynders at the duke's place, a
/// scepter's at a greater stone Lord Skwol; no other spell, nor another
/// rank's, is the stones'. The templar, out of reach of players, turns to
/// its summoner at a second and a half, denounces them, and at eight
/// seconds can be fought and attacks them; it lasts a minute.
#[tokio::test]
#[ignore = "extended"]
async fn a_wind_stone_calls_its_lord_who_denounces_and_attacks_its_summoner() {
    let Some(realm) = realm() else {
        return;
    };
    let mut c = Client::connect(&realm, "Stones");
    let me = c.enter("Stones", Race::Human, Class::Warrior).await;
    c.send(CMSG_COMPLETE_CINEMATIC {}).await;
    let map = go(&realm, &mut c, 1, STONE_AT).await;
    let out = map
        .query(move |m| {
            let stone = m
                .gameobjects
                .iter()
                .find(|(_, o)| o.entry == wind_stones::LESSER[0])
                .map(|(_, o)| o.guid)
                .expect("the stone");
            let at = m.gameobject(stone).unwrap().position;
            let unclaimed = (
                wind_stones::activated(m, me, stone, 12_345),
                wind_stones::calls(178_145, 24_734).is_some(),
                wind_stones::calls(wind_stones::LESSER[0], 24_765).is_some(),
            );
            crate::game::map::spells::cast_triggered(m, me, 24_734, Some(me));
            let templar = called_at(m, &TEMPLARS, at);
            let gone = m.gameobject(stone).is_none();
            (unclaimed, templar, gone)
        })
        .await
        .unwrap();
    let (unclaimed, templar, gone) = out;
    assert_eq!(unclaimed, (false, false, false), "none but the stones' own");
    assert_eq!(templar.len(), 1, "one templar where the stone was");
    assert!(gone, "and the stone gone");
    let lord = templar[0].1;
    // The challenge.
    let state = move |m: &mut Map, by: f32| {
        hurry_runs(m, Some(lord), Duration::from_secs_f32(by));
        let c = m.creature(lord).unwrap();
        let facing = {
            let p = m.players[&me].position();
            let want = (p.y - c.position.y).atan2(p.x - c.position.x);
            let d = (c.orientation - want).rem_euclid(std::f32::consts::TAU);
            d.min(std::f32::consts::TAU - d) < 0.05
        };
        (
            facing,
            c.fields.get_u32(UNIT_FIELD_FLAGS) & 0x100 != 0,
            c.combat.attacking && c.combat.victim == me,
        )
    };
    let start = map.query(move |m| state(m, 0.0)).await.unwrap();
    let turned = map.query(move |m| state(m, 1.5)).await.unwrap();
    map.query(move |m| state(m, 0.1)).await;
    let said = heard(&mut c, lord, Duration::from_secs(2)).await;
    let fights = map.query(move |m| state(m, 6.4)).await.unwrap();
    assert_eq!(start, (false, true, false), "out of reach as it comes");
    assert_eq!(turned, (true, true, false), "turned to its summoner");
    assert!(said.is_some(), "its words");
    assert_eq!(fights.1..=fights.2, false..=true, "attackable, and at them");
    let lasts = map
        .query(move |m| {
            event_ai::scripted(m, lord).and_then(|s| s.despawn_after.map(|d| (d, s.despawn_type)))
        })
        .await
        .unwrap();
    assert_eq!(
        lasts,
        Some((Duration::from_secs(60), 1)),
        "a minute, timed or dead"
    );
    // The lords of a crest, a signet and a scepter.
    let lords = map
        .query(move |m| {
            let mut out = Vec::new();
            for (stone, spell, lord, place) in [
                (wind_stones::LESSER[0], 24_744, CRIMSON_TEMPLAR, None),
                (
                    wind_stones::STONES[0],
                    24_765,
                    DUKE_OF_CYNDERS,
                    Some(Vector3d {
                        x: -7_927.48,
                        y: 1_935.30,
                        z: 5.61,
                    }),
                ),
                (wind_stones::GREATER[0], 24_790, LORD_SKWOL, None),
            ] {
                let here = m.players[&me].position();
                crate::game::map::objects::summon(m, None, stone, here, 0.0, 0, 0, 0, None)
                    .expect("a stone");
                let before = called_at(m, &[lord], place.unwrap_or(here));
                crate::game::map::spells::cast_triggered(m, me, spell, Some(me));
                let new = called_at(m, &[lord], place.unwrap_or(here))
                    .into_iter()
                    .filter(|x| !before.contains(x))
                    .count();
                out.push((lord, new));
            }
            out
        })
        .await
        .unwrap();
    for (lord, n) in lords {
        assert_eq!(n, 1, "{lord} called");
    }
}
