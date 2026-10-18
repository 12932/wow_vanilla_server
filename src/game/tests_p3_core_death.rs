//! Phase 3's deaths and duels, Thistle Tea's `DeathTest` and `DuelTest`
//! ported: resurrection sickness by level, no fight carried through a
//! death, a creature's threat on the dead let go, and a duel's bounds:
//! out past 75 yards, back within 70, fled after ten seconds out or on
//! leaving.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::death::{self, resurrection_sickness_ms};
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::{Map, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const WOLF: u32 = 3180;
const DUEL: u32 = 7266;

/// Thistle Tea's `resurrection_sickness_duration_ms/1`: none below level
/// eleven, a minute a level past ten, ten minutes at most.
#[test]
fn resurrection_sickness_lasts_a_minute_a_level_past_ten() {
    assert_eq!(resurrection_sickness_ms(10), None);
    assert_eq!(resurrection_sickness_ms(11), Some(60_000));
    assert_eq!(resurrection_sickness_ms(19), Some(540_000));
    assert_eq!(resurrection_sickness_ms(20), Some(600_000));
    assert_eq!(resurrection_sickness_ms(60), Some(600_000));
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.creature_templates.insert(
        WOLF,
        CreatureTemplate {
            entry: WOLF,
            name: "Wolf".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            display_scales: [1.0, 0.0, 0.0, 0.0],
            level_min: 5,
            level_max: 5,
            faction: 7,
            unit_class: 1,
            health_max: 500,
            ..Default::default()
        },
    );
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: 1,
            entries: vec![WOLF],
            map: 0,
            x: START.0 + 3.0,
            y: START.1,
            z: START.2,
            ..Default::default()
        }],
    );
    d.spells.spells.insert(
        DUEL,
        SpellTemplate {
            id: DUEL,
            name: "Duel".into(),
            effects: [
                SpellEffect {
                    effect: 83,
                    target_a: 25,
                    ..Default::default()
                },
                SpellEffect::default(),
                SpellEffect::default(),
            ],
            ..Default::default()
        },
    );
    d
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

fn wolf() -> Guid {
    guid::unit(WOLF, 1)
}

/// Thistle Tea's `resurrect/3` and a player's death's combat cleanup: one
/// who dies fighting rises out of the fight, untargeted, off the
/// creature's threat.
#[tokio::test]
async fn a_resurrection_carries_no_fight() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(wolf()).await;
    let fighting = on_map(&realm, move |m| {
        crate::game::map::combat::deal_damage(m, wolf(), g, 1);
        crate::game::map::combat::deal_damage(m, g, wolf(), 1);
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_guid(UNIT_FIELD_TARGET, wolf());
        m.creature(wolf())
            .is_some_and(|c| c.threat.iter().any(|(t, _)| *t == g))
    })
    .await;
    assert!(fighting, "on the wolf's threat");
    on_map(&realm, move |m| {
        let health = m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH);
        crate::game::map::combat::deal_damage(m, wolf(), g, health);
    })
    .await;
    // By the wolf's next turn.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let after_death = on_map(&realm, move |m| {
        m.creature(wolf())
            .is_some_and(|c| c.threat.iter().any(|(t, _)| *t == g))
    })
    .await;
    assert!(!after_death, "the wolf's threat on her let go");
    let risen = on_map(&realm, move |m| {
        death::resurrect(m, g, 0.5);
        let p = &m.players[&g];
        (
            p.death == death::Death::Alive,
            p.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT),
            p.fields.get_guid(UNIT_FIELD_TARGET),
        )
    })
    .await;
    assert_eq!(risen, (true, false, Guid::zero()));
}

async fn duel(realm: &Arc<Realm>, a: &mut Client, b: &mut Client) -> Guid {
    let (ga, gb) = (a.guid, b.guid);
    on_map(realm, move |m| {
        m.players.get_mut(&ga).unwrap().spells.push(DUEL)
    })
    .await;
    a.send(CMSG_CAST_SPELL {
        spell: DUEL,
        targets: crate::game::map::spells::wire_targets(&crate::game::map::spells::Targets {
            unit: Some(gb),
            ..Default::default()
        }),
    })
    .await;
    let flag = b
        .wait_for("SMSG_DUEL_REQUESTED", |m| match m {
            ServerOpcodeMessage::SMSG_DUEL_REQUESTED(r) if r.target == ga => Some(r.initiator),
            _ => None,
        })
        .await;
    b.send(CMSG_DUEL_ACCEPTED { guid: flag }).await;
    a.wait_for("SMSG_DUEL_COUNTDOWN", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_DUEL_COUNTDOWN(_)).then_some(())
    })
    .await;
    on_map(realm, move |m| {
        for g in [ga, gb] {
            if let Some(d) = m.players.get_mut(&g).and_then(|p| p.duel.as_mut()) {
                d.phase = crate::game::map::duel::Phase::Countdown {
                    until: Instant::now(),
                };
            }
        }
    })
    .await;
    for _ in 0..200 {
        if on_map(realm, move |m| crate::game::map::duel::dueling(m, ga, gb)).await {
            return flag;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("the duel never began");
}

/// Put `g` `yards` east of their duel's flag.
async fn from_flag(realm: &Arc<Realm>, g: Guid, _flag: Guid, yards: f32) {
    on_map(realm, move |m| {
        let flag = m.players[&g].duel.expect("in a duel").flag;
        let at = m.gameobject(flag).expect("the flag").position;
        m.players.get_mut(&g).unwrap().movement.position = Vector3d {
            x: at.x + yards,
            ..at
        };
    })
    .await;
}

async fn out_of_bounds(realm: &Arc<Realm>, g: Guid) -> bool {
    on_map(realm, move |m| {
        m.players[&g]
            .duel
            .is_some_and(|d| d.out_of_bounds_since.is_some())
    })
    .await
}

/// Thistle Tea's `check_bounds/4`: out past 75 yards, still out at 72,
/// back in at 69; out ten seconds, fled and beaten; a duellist leaving
/// the world flees.
#[tokio::test]
async fn a_duels_bounds_are_left_at_75_yards_and_regained_at_70() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.sees(b.guid).await;
    let flag = duel(&realm, &mut a, &mut b).await;
    b.clear();
    from_flag(&realm, gb, flag, 76.0).await;
    b.wait_frame("SMSG_DUEL_OUTOFBOUNDS", 0x168, |_| true).await;
    from_flag(&realm, gb, flag, 72.0).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(out_of_bounds(&realm, gb).await, "still out at 72");
    from_flag(&realm, gb, flag, 69.0).await;
    b.wait_frame("SMSG_DUEL_INBOUNDS", 0x169, |_| true).await;
    assert!(!out_of_bounds(&realm, gb).await);

    // Out at 72 (past 70) for ten seconds: fled.
    from_flag(&realm, gb, flag, 80.0).await;
    b.wait_frame("SMSG_DUEL_OUTOFBOUNDS", 0x168, |_| true).await;
    from_flag(&realm, gb, flag, 72.0).await;
    a.clear();
    on_map(&realm, move |m| {
        if let Some(d) = m.players.get_mut(&gb).and_then(|p| p.duel.as_mut()) {
            d.out_of_bounds_since = Some(Instant::now() - Duration::from_secs(10));
        }
    })
    .await;
    let reason = a
        .wait_for("SMSG_DUEL_WINNER", |m| match m {
            ServerOpcodeMessage::SMSG_DUEL_WINNER(w) => Some(w.reason),
            _ => None,
        })
        .await;
    assert_eq!(reason, DuelWinnerReason::Fled);

    // One who leaves the world mid-duel flees it.
    let ga = a.guid;
    on_map(&realm, move |m| {
        let at = m.players[&ga].position();
        m.players.get_mut(&gb).unwrap().movement.position = at;
    })
    .await;
    duel(&realm, &mut a, &mut b).await;
    a.clear();
    b.send(CMSG_LOGOUT_REQUEST {}).await;
    let reason = a
        .wait_for("SMSG_DUEL_WINNER", |m| match m {
            ServerOpcodeMessage::SMSG_DUEL_WINNER(w) => Some(w.reason),
            _ => None,
        })
        .await;
    assert_eq!(reason, DuelWinnerReason::Fled);
}
