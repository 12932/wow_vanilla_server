//! Round 8's combat tests, reactive windows and the warrior's combo point
//! (Thistle Tea's `reactive_test.exs`): a finisher spends the point at
//! once, and a player back in the world has neither windows nor points,
//! but the health bits her health gives her.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{ATTR_EX_REQ_COMBO_POINTS, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::combat::{self, AURA_STATE_DEFENSE, AURA_STATE_HEALTHLESS_20, Outcome};
use crate::game::map::spells::Targets;
use crate::game::tests_p3_combat::{self as p3, BRUTE};

/// A strike that wants the target's combo point (Overpower's kind).
const OVERPOWER: u32 = 96_900;

fn data() -> GameData {
    let mut d = p3::data();
    let mut s = SpellTemplate {
        id: OVERPOWER,
        name: "Overpower".into(),
        range_index: 1,
        equipped_item_class: -1,
        dmg_class: 2,
        ..Default::default()
    };
    s.attributes[1] |= ATTR_EX_REQ_COMBO_POINTS;
    s.effects[0] = SpellEffect {
        effect: 2,
        base_points: 4,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(OVERPOWER, s);
    d
}

/// Thistle Tea's "consume clears the combo point immediately": the point
/// a dodge gave a warrior on her target is gone as soon as the strike
/// that wants it lands, long before it would run out.
#[tokio::test]
async fn a_strike_that_wants_the_dodge_point_spends_it_at_once() {
    let (_realm, _a, ga, map) = p3::setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            p3::level(m, ga, 60);
            m.players.get_mut(&ga).unwrap().spells.push(OVERPOWER);
            let brute = p3::spawn(m, ga, BRUTE, 60, 3.0);
            combat::after_outcome(m, ga, brute, Outcome::Dodge);
            let given = m
                .caster(ga)
                .map(|c| (c.combo_points, c.combo_target == brute));
            crate::game::map::spells::cast_now(
                m,
                ga,
                OVERPOWER,
                Targets {
                    unit: Some(brute),
                    ..Default::default()
                },
            );
            let spent = m.caster(ga).map(|c| c.combo_points);
            (given, spent)
        })
        .await
        .unwrap();
    assert_eq!(r.0, Some((1, true)), "the dodge's point, on the brute");
    assert_eq!(r.1, Some(0), "spent by the strike at once");
}

/// Thistle Tea's "clears windows and combo markers while preserving
/// unrelated aura states" (its `Reactive.clear` as a player enters the
/// world): a warrior who logs out with a dodge's point on a creature and
/// her own Revenge window open, at a tenth of her health, comes back with
/// neither, but healthless.
#[tokio::test]
async fn a_player_back_in_the_world_has_no_windows_nor_points_but_her_health_bits() {
    let (_realm, mut a, ga, map) = p3::setup_with(data(), Class::Warrior).await;
    map.query(move |m| {
        p3::level(m, ga, 60);
        let brute = p3::spawn(m, ga, BRUTE, 60, 3.0);
        combat::after_outcome(m, ga, brute, Outcome::Dodge);
        combat::after_outcome(m, brute, ga, Outcome::Dodge);
        let max = m.players[&ga].fields.get_u32(UNIT_FIELD_MAXHEALTH);
        m.players
            .get_mut(&ga)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_HEALTH, max / 10);
        combat::sync_aura_state(m, ga);
        combat::leave_combat(m, ga);
    })
    .await
    .unwrap();
    let before = map
        .query(move |m| {
            let c = m.combat(ga).unwrap();
            (
                m.caster(ga).unwrap().combo_points,
                c.defense_window.is_some(),
                m.players[&ga].fields.get_u32(UNIT_FIELD_AURASTATE),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        before,
        (1, true, AURA_STATE_DEFENSE | AURA_STATE_HEALTHLESS_20),
        "a point, a window, healthless"
    );
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.login(ga).await;
    let deadline = tokio::time::Instant::now() + crate::game::harness::WAIT;
    let after = loop {
        let got = map
            .query(move |m| {
                let p = m.players.get(&ga)?;
                Some((
                    p.caster.combo_points,
                    p.caster.combo_target,
                    p.combat.defense_window.is_some(),
                    p.fields.get_u32(UNIT_FIELD_AURASTATE),
                ))
            })
            .await
            .unwrap();
        if let Some(got) = got {
            break got;
        }
        assert!(tokio::time::Instant::now() < deadline, "back in the world");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert_eq!(
        after,
        (0, Guid::zero(), false, AURA_STATE_HEALTHLESS_20),
        "(points, on whom, a window, the aura state)"
    );
}
