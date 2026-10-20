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

/// On the real data: a Target Dummy (4071), a wild summon, is of its
/// summoner's side, fights nothing and stands where it is put, with its
/// passive (4044) on; its spawn effect taunts the enemy beside it onto it;
/// and fifteen seconds on it kills itself, in its fight (Thistle Tea's
/// "all target dummies apply their passive and initial area taunt";
/// mangos-classic's `npc_advanced_target_dummy`). wvs left it hostile to
/// its summoner, and for ever.
#[tokio::test]
#[ignore = "extended"]
async fn a_target_dummy_stands_taunts_and_dies_at_its_time() {
    use crate::game::fields::{UNIT_FIELD_FACTIONTEMPLATE, UNIT_FIELD_FLAGS};
    use crate::game::map::assistance::{Reaction, reaction};
    use crate::game::tests_p3_scripts::{hero, realm, summon_near};
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = hero(&realm, "Engineer").await;
    let at = map.query(move |m| m.players[&me].position()).await.unwrap();
    // A Defias Thug, an enemy beside the spot.
    let thug = summon_near(
        &map,
        38,
        Vector3d {
            x: at.x + 4.0,
            ..at
        },
    )
    .await;
    let (dummy, now) = map
        .query(move |m| {
            crate::game::map::spells::cast_triggered(m, me, 4071, Some(me));
            let dummy = m
                .creatures
                .iter()
                .find(|(_, c)| c.entry == 2673)
                .map(|(_, c)| c.guid);
            (dummy, m.now)
        })
        .await
        .unwrap();
    let dummy = dummy.expect("the dummy");
    let seen = map
        .query(move |m| {
            let c = m.creature(dummy).unwrap();
            (
                crate::game::map::auras::auras(m, dummy).is_some_and(|a| a.has_spell(4044)),
                reaction(m, dummy) == Reaction::Passive,
                c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE)
                    == m.players[&me].fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
                m.creature(thug).map(|t| t.combat.victim) == Some(dummy),
                m.objects.pending_spells.iter().any(|(t, _, s, by)| {
                    *s == 7 && *by == dummy && *t == now + Duration::from_secs(15)
                }),
            )
        })
        .await
        .unwrap();
    assert_eq!(seen, (true, true, true, true, true));
    for _ in 0..20 {
        harness::an_update(&map).await;
    }
    let (moved, fighting, still) = map
        .query(move |m| {
            let c = m.creature(dummy).unwrap();
            (
                dist(c.position, at) > 3.0,
                c.fields.get_u32(UNIT_FIELD_FLAGS) & crate::game::map::player::UNIT_FLAG_IN_COMBAT
                    != 0,
                !c.combat.attacking,
            )
        })
        .await
        .unwrap();
    assert!(!moved, "it stands where it was put");
    assert!(fighting, "in its fight");
    assert!(still, "striking no one");
    map.query(move |m| {
        let now = m.now;
        for p in m.objects.pending_spells.iter_mut() {
            if p.3 == dummy {
                p.0 = now;
            }
        }
    })
    .await;
    let mut dead = false;
    for _ in 0..40 {
        harness::an_update(&map).await;
        dead = map
            .query(move |m| m.creature(dummy).is_none_or(|c| !c.alive()))
            .await
            .unwrap();
        if dead {
            break;
        }
    }
    assert!(dead, "dead at its time, fighting");
}

/// On the real data: a Mechanical Dragonling (4073) comes with its passive
/// (23051) on, and at the full health that gives it (Thistle Tea's
/// "dragonlings start with their passive health"; mangos's
/// `HandleAuraModIncreaseHealth` raises the health with the most).
#[tokio::test]
#[ignore = "extended"]
async fn a_dragonling_comes_at_the_full_health_of_its_passive() {
    use crate::game::fields::{UNIT_FIELD_HEALTH, UNIT_FIELD_MAXHEALTH};
    use crate::game::tests_p3_scripts::{hero, realm};
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = hero(&realm, "Tinker").await;
    let seen = map
        .query(move |m| {
            crate::game::map::spells::cast_triggered(m, me, 4073, Some(me));
            let g = m.players[&me].owned.guardians.first().copied()?;
            let c = m.creature(g)?;
            let bonus = m
                .realm
                .data
                .spells
                .get(23051)
                .map_or(0, |s| s.effects[0].base_points + 1);
            Some((
                c.entry,
                crate::game::map::auras::auras(m, g).is_some_and(|a| a.has_spell(23051)),
                c.fields.get_u32(UNIT_FIELD_HEALTH),
                c.fields.get_u32(UNIT_FIELD_MAXHEALTH),
                0u32,
                bonus,
            ))
        })
        .await
        .unwrap()
        .expect("the dragonling");
    let (entry, passive, health, max, _, bonus) = seen;
    assert_eq!(entry, 2678);
    assert!(passive, "its passive on");
    assert!(bonus > 0);
    assert_eq!(health, max, "at its full health: {seen:?}");
    // And keeps it as its stats are worked out again.
    for _ in 0..60 {
        harness::an_update(&map).await;
    }
    let later = map
        .query(move |m| {
            let g = m.players[&me].owned.guardians[0];
            let c = m.creature(g).unwrap();
            (
                c.fields.get_u32(UNIT_FIELD_HEALTH),
                c.fields.get_u32(UNIT_FIELD_MAXHEALTH),
            )
        })
        .await
        .unwrap();
    assert_eq!(later, (max, max), "kept");
}

/// A script's reaction (command 59) is what the creature does: passive, it
/// looks for no fight and answers no call for help; defensive, it looks for
/// none but answers; aggressive, both (Thistle Tea's "reaction changes
/// publish acquisition and assistance eligibility"; mangos's
/// `SetReactState`, `CanAssist` refusing the passive).
#[tokio::test]
async fn a_scripts_reaction_decides_aggro_and_assistance() {
    use crate::game::map::assistance::{Reaction, available, reaction};
    use crate::game::tests_p3_combat::{BRUTE, spawn};
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let seen = map
        .query(move |m| {
            let g = spawn(m, ga, BRUTE, 20, 30.0);
            let mut seen = Vec::new();
            for d1 in [0, 1, 2] {
                let step = crate::game::data::scripts::ScriptStep {
                    command: 59,
                    datalong: [d1, 0, 0, 0],
                    ..Default::default()
                };
                crate::game::map::scripts::execute(m, &step, g, g);
                seen.push((reaction(m, g) == Reaction::Aggressive, available(m, g)));
            }
            seen
        })
        .await
        .unwrap();
    assert_eq!(seen, [(false, false), (false, true), (true, true)]);
}
