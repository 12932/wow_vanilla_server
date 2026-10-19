//! Round 8's entity tests of mobs, ported from Thistle Tea's
//! `test/game/world/entity/mob/*_test.exs` (docs/parity/matrix.csv): the
//! Lunar Festival's firework guys.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::fireworks::{CLUSTER_CREDIT, LIFETIME, LUCKY, LUNAR_FORTUNE, ROCKET_CREDIT};
use crate::game::harness;
use crate::game::map::Map;
use crate::game::map::combat::dist;
use crate::game::tests_p3_combat::{data, setup_with, template};
use crate::game::tests_p3_entity::take_quest;
use crate::game::tests_p3_entity_objects::object;

const RED_ROCKET_GUY: u32 = 15_882;
const RED_ROCKET: u32 = 180_851;
const BLUE_BIG: u32 = 180_861;
const WHITE_BIG: u32 = 180_864;
/// Lunar Fireworks: eight rockets and two clusters.
const LUNAR_FIREWORKS: u32 = 8867;
/// A creature of no firework (Omen's minion).
const OTHER: u32 = 15_466;

fn world() -> GameData {
    let mut d = data();
    for entry in [RED_ROCKET_GUY, LUCKY, OTHER] {
        d.creature_templates.insert(entry, template(entry, 10));
    }
    for entry in [RED_ROCKET, BLUE_BIG, WHITE_BIG] {
        let t = object(entry, 10, "Firework Rocket", &[]);
        d.gameobject_templates.insert(entry, t);
    }
    d.npc.quests.insert(
        LUNAR_FIREWORKS,
        crate::game::data::npc::Quest {
            id: LUNAR_FIREWORKS,
            title: "Lunar Fireworks".into(),
            required_kills: [
                (ROCKET_CREDIT as i32, 8),
                (CLUSTER_CREDIT as i32, 2),
                (0, 0),
                (0, 0),
            ],
            ..Default::default()
        },
    );
    d
}

/// A guy of `entry` called up 5 yards before `g`, by `by` (the rocket's
/// wild summon names its caster its summoner).
fn call_up(m: &mut Map, g: Guid, entry: u32, by: Guid) -> (Guid, Vector3d) {
    let mut at = m.players[&g].position();
    at.x += 5.0;
    let guy = crate::game::map::scripts::summon(m, by, entry, at, 0.0, None).unwrap();
    (guy, at)
}

fn objects_at(m: &Map) -> Vec<(u32, Vector3d, Option<std::time::Instant>)> {
    let mut out: Vec<_> = m
        .gameobjects
        .iter()
        .map(|(_, o)| {
            let expires = m
                .objects
                .summoned
                .iter()
                .find(|s| s.guid == o.guid)
                .and_then(|s| s.expires);
            (o.entry, o.position, expires)
        })
        .collect();
    out.sort_by(|a, b| a.1.z.total_cmp(&b.1.z));
    out
}

/// A rocket's firework guy, as it appears, sends its rocket up three yards
/// above it, for five seconds, and the player who fired it has the
/// rocket's credit toward Lunar Fireworks (Thistle Tea's "sends the rocket
/// up and credits the player who fired it"; vmangos's
/// `npc_pats_firework_guy`).
#[tokio::test]
async fn a_rockets_guy_sends_it_up_and_credits_who_fired_it() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let at = map
        .query(move |m| {
            take_quest(m, ga, LUNAR_FIREWORKS);
            call_up(m, ga, RED_ROCKET_GUY, ga).1
        })
        .await
        .unwrap();
    let credit = a
        .wait_for("the rocket's credit", |m| match m {
            ServerOpcodeMessage::SMSG_QUESTUPDATE_ADD_KILL(k) => {
                Some((k.quest_id, k.creature_id, k.kill_count))
            }
            _ => None,
        })
        .await;
    assert_eq!(credit, (LUNAR_FIREWORKS, ROCKET_CREDIT, 1));
    let (sent, kills, now) = map
        .query(move |m| (objects_at(m), m.players[&ga].journal.log[0].kills, m.now))
        .await
        .unwrap();
    assert_eq!(sent.len(), 1, "one rocket");
    let (entry, burst, expires) = sent[0];
    assert_eq!(entry, RED_ROCKET);
    assert!(
        dist(
            burst,
            Vector3d {
                z: at.z + 3.0,
                ..at
            }
        ) < 0.01,
        "three yards up"
    );
    let left = expires.unwrap().saturating_duration_since(now);
    assert!(left <= LIFETIME && left > LIFETIME - Duration::from_secs(2));
    assert_eq!(kills, [1, 0, 0, 0]);
}

/// A Lucky Lunar Rocket's guy fans five large rockets around and over
/// itself, blue and white; with no player to fire it, no one has a credit;
/// and it is to cast Lunar Fortune on itself three seconds on. Another
/// creature sends nothing up (Thistle Tea's "a lucky cluster brings Lunar
/// Fortune three seconds after it bursts" and "any other creature
/// launches nothing").
#[tokio::test]
async fn a_lucky_cluster_fans_out_and_brings_lunar_fortune() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (other, spot) = map.query(move |m| call_up(m, ga, OTHER, ga)).await.unwrap();
    harness::an_update(&map).await;
    harness::an_update(&map).await;
    let none = map.query(|m| m.gameobjects.len()).await.unwrap();
    assert_eq!(none, 0, "another creature sends nothing up");
    let (guy, at) = map
        .query(move |m| {
            crate::game::map::scripts::despawn(m, other);
            let mut at = spot;
            at.y += 20.0;
            // Called up by a creature, as a script's would be.
            let c = crate::game::map::scripts::summon(m, other, LUCKY, at, 0.0, None).unwrap();
            take_quest(m, ga, LUNAR_FIREWORKS);
            (c, at)
        })
        .await
        .unwrap();
    harness::an_update(&map).await;
    harness::an_update(&map).await;
    let (sent, fortune, kills, now) = map
        .query(move |m| {
            (
                objects_at(m),
                m.objects
                    .pending_spells
                    .iter()
                    .find(|(_, _, s, by)| *s == LUNAR_FORTUNE && *by == guy)
                    .map(|(t, ..)| *t),
                m.players[&ga].journal.log[0].kills,
                m.now,
            )
        })
        .await
        .unwrap();
    let mut entries: Vec<u32> = sent.iter().map(|s| s.0).collect();
    entries.sort();
    assert_eq!(
        entries,
        [BLUE_BIG, BLUE_BIG, BLUE_BIG, WHITE_BIG, WHITE_BIG],
        "five, blue and white"
    );
    let offsets: Vec<(i32, i32, i32)> = sent
        .iter()
        .map(|s| {
            (
                ((s.1.x - at.x) * 100.0).round() as i32,
                ((s.1.y - at.y) * 100.0).round() as i32,
                ((s.1.z - at.z) * 100.0).round() as i32,
            )
        })
        .collect();
    assert_eq!(
        offsets,
        [
            (0, 0, 300),
            (0, 300, 750),
            (525, -150, 750),
            (-525, -150, 750),
            (0, 0, 1200)
        ],
        "fanned around and over it"
    );
    let fortune = fortune.expect("Lunar Fortune to come");
    let left = fortune.saturating_duration_since(now);
    assert!(left <= Duration::from_secs(3) && left > Duration::from_secs(1));
    assert_eq!(kills, [0, 0, 0, 0], "no one fired it");
}

/// On the real data: a Small Red Rocket (26286) fired calls up its guy,
/// whose rocket goes up and whose credit counts toward Lunar Fireworks
/// (8867); a Lucky Lunar Rocket (26521) sends up its five and the
/// cluster's credit.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_lunar_rockets_go_up_and_count_toward_lunar_fireworks() {
    use crate::game::tests_p3_scripts::{hero, realm};
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = hero(&realm, "Rocketeer").await;
    map.query(move |m| {
        take_quest(m, me, LUNAR_FIREWORKS);
        let mut at = m.players[&me].position();
        at.x += 3.0;
        for spell in [26_286, 26_521] {
            crate::game::map::spells::cast_triggered_with(
                m,
                me,
                spell,
                crate::game::map::spells::Targets {
                    dest: Some(at),
                    ..Default::default()
                },
            );
        }
    })
    .await;
    let mut seen = (Vec::new(), [0; 4]);
    for _ in 0..200 {
        harness::an_update(&map).await;
        seen = map
            .query(move |m| {
                let mut rockets: Vec<u32> = m
                    .gameobjects
                    .iter()
                    .map(|(_, o)| o.entry)
                    .filter(|e| (180_851..=180_865).contains(e))
                    .collect();
                rockets.sort();
                (rockets, m.players[&me].journal.log[0].kills)
            })
            .await
            .unwrap();
        if seen.1[..2] == [1, 1] {
            break;
        }
    }
    assert_eq!(seen.1[..2], [1, 1], "a rocket's credit and a cluster's");
    assert_eq!(
        seen.0,
        [
            RED_ROCKET, BLUE_BIG, BLUE_BIG, BLUE_BIG, WHITE_BIG, WHITE_BIG
        ]
    );
}
