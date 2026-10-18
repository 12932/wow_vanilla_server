//! Phase 3's combat tests: damage shared (Thistle Tea's
//! `core/combat/damage_sharing_test.exs` and
//! `world/combat/damage_sharing_test.exs`): Blessing of Sacrifice's,
//! Soul Link's and a Voidwalker's shares, after the shields and in their
//! order, to takers alive and able, from swings, spells and ticks alike.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::auras::{self, *};
use crate::game::map::combat::{self, Outcome};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::{BRUTE, aura_of, forced_swing, setup, spawn};

const PHYSICAL: u32 = 1;
const FIRE: u32 = 4;

/// `caster`'s split aura of `kind` and `amount` on `target`, for `mask`.
fn link(m: &mut Map, caster: Guid, target: Guid, id: u32, kind: u32, amount: i32, mask: i32) {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura: kind,
        misc_value: mask,
        base_points: amount - 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    let s = SpellTemplate {
        id,
        name: format!("Share {id}"),
        equipped_item_class: -1,
        effects,
        ..Default::default()
    };
    let _ = auras::apply_spell(
        m,
        caster,
        target,
        &s,
        &[(0, amount)],
        auras::Context::default(),
    );
}

/// A unit of 1000 health, `yards` ahead of `g`.
fn unit(m: &mut Map, g: Guid, yards: f32) -> Guid {
    let c = spawn(m, g, BRUTE, 20, yards);
    let f = m.unit_fields_mut(c).unwrap();
    f.set_u32(UNIT_FIELD_MAXHEALTH, 1000);
    f.set_u32(UNIT_FIELD_HEALTH, 1000);
    c
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

async fn world() -> (Guid, MapHandle) {
    let (_realm, _a, g, map) = setup(Class::Warrior).await;
    (g, map)
}

#[tokio::test]
async fn shields_soak_first_then_the_flat_shares_then_each_share_of_what_is_left() {
    let (g, map) = world().await;
    let got = map
        .query(move |m| {
            let attacker = unit(m, g, 3.0);
            let target = unit(m, g, 4.0);
            let (two, three, four) = (unit(m, g, 5.0), unit(m, g, 6.0), unit(m, g, 7.0));
            link(m, three, target, 95_500, AURA_SPLIT_DAMAGE_PCT, 50, 127);
            aura_of(m, target, 95_501, AURA_SCHOOL_ABSORB, 127, 20);
            link(m, four, target, 95_502, AURA_SPLIT_DAMAGE_PCT, 50, 127);
            link(m, two, target, 95_503, AURA_SPLIT_DAMAGE_FLAT, 20, 127);
            let (left, absorbed) = auras::absorb_and_share(m, attacker, target, 100, PHYSICAL);
            let shield = auras::auras(m, target).is_some_and(|a| a.has(AURA_SCHOOL_ABSORB));
            (
                (left, absorbed),
                [two, three, four].map(|u| 1000 - health(m, u)),
                shield,
            )
        })
        .await
        .unwrap();
    assert_eq!(got.0, (15, 85));
    assert_eq!(got.1, [20, 30, 15]);
    assert!(!got.2, "the shield is used up");
}

#[tokio::test]
async fn nothing_is_shared_once_absorbed_nor_with_the_dead_oneself_or_another_school() {
    let (g, map) = world().await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            // All absorbed: the taker takes nothing.
            let attacker = unit(m, g, 3.0);
            let target = unit(m, g, 4.0);
            let taker = unit(m, g, 5.0);
            aura_of(m, target, 95_501, AURA_SCHOOL_ABSORB, 127, 200);
            link(m, taker, target, 95_500, AURA_SPLIT_DAMAGE_PCT, 30, 127);
            out.push(auras::absorb_and_share(m, attacker, target, 100, PHYSICAL));
            out.push((health(m, taker), 0));
            // A dead taker, one's own link, one for fire only.
            let target = unit(m, g, 4.0);
            let dead = unit(m, g, 5.0);
            let fire = unit(m, g, 6.0);
            link(m, dead, target, 95_510, AURA_SPLIT_DAMAGE_FLAT, 45, 127);
            combat::deal_damage(m, attacker, dead, 5000);
            link(
                m,
                fire,
                target,
                95_511,
                AURA_SPLIT_DAMAGE_FLAT,
                45,
                FIRE as i32,
            );
            link(m, target, target, 95_512, AURA_SPLIT_DAMAGE_FLAT, 45, 127);
            out.push((
                auras::share(m, attacker, target, 100, PHYSICAL),
                health(m, fire),
            ));
            // One's own damage isn't shared.
            let taker = unit(m, g, 5.0);
            let target = unit(m, g, 4.0);
            link(m, taker, target, 95_513, AURA_SPLIT_DAMAGE_PCT, 50, 127);
            out.push((
                auras::share(m, target, target, 100, PHYSICAL),
                health(m, taker),
            ));
            out
        })
        .await
        .unwrap();
    assert_eq!(got, [(0, 100), (1000, 0), (100, 1000), (100, 1000)]);
}

#[tokio::test]
async fn a_flat_share_takes_its_stacks_and_no_more_than_there_is_and_a_share_rounds_down() {
    let (g, map) = world().await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            let attacker = unit(m, g, 3.0);
            // Two stacks of 45: 90, then all of 20.
            let target = unit(m, g, 4.0);
            let taker = unit(m, g, 5.0);
            link(m, taker, target, 95_520, AURA_SPLIT_DAMAGE_FLAT, 45, 127);
            if let Some(a) = auras::auras_mut(m, target) {
                a.holders.iter_mut().for_each(|h| h.stacks = 2);
            }
            out.push(auras::share(m, attacker, target, 100, PHYSICAL));
            out.push(1000 - health(m, taker));
            out.push(auras::share(m, attacker, target, 20, PHYSICAL));
            // Thirty percent of 2 is none, of 4 one; two hundred percent
            // of 100 is all of it.
            let target = unit(m, g, 4.0);
            link(m, taker, target, 95_521, AURA_SPLIT_DAMAGE_PCT, 30, 127);
            out.push(auras::share(m, attacker, target, 2, PHYSICAL));
            out.push(auras::share(m, attacker, target, 4, PHYSICAL));
            let target = unit(m, g, 4.0);
            link(m, taker, target, 95_522, AURA_SPLIT_DAMAGE_PCT, 200, 127);
            out.push(auras::share(m, attacker, target, 100, PHYSICAL));
            out
        })
        .await
        .unwrap();
    assert_eq!(got, [10, 90, 0, 2, 3, 0]);
}

#[tokio::test]
async fn a_flat_share_meets_its_takers_shields_a_share_doesnt_and_nothing_comes_back() {
    let (g, map) = world().await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            let attacker = unit(m, g, 3.0);
            // A flat 45 on a taker with a shield of 20 and more damage
            // taken: 25, no more.
            let target = unit(m, g, 4.0);
            let taker = unit(m, g, 5.0);
            aura_of(m, taker, 95_530, AURA_SCHOOL_ABSORB, 127, 20);
            aura_of(m, taker, 95_531, AURA_MOD_DAMAGE_PERCENT_TAKEN, 127, 100);
            link(m, taker, target, 95_532, AURA_SPLIT_DAMAGE_FLAT, 45, 127);
            auras::share(m, attacker, target, 100, PHYSICAL);
            out.push(1000 - health(m, taker));
            // Thirty percent past a shield of 200: 30.
            let target = unit(m, g, 4.0);
            let taker = unit(m, g, 5.0);
            aura_of(m, taker, 95_530, AURA_SCHOOL_ABSORB, 127, 200);
            link(m, taker, target, 95_533, AURA_SPLIT_DAMAGE_PCT, 30, 127);
            auras::share(m, attacker, target, 100, PHYSICAL);
            out.push(1000 - health(m, taker));
            // Links both ways: the share isn't sent back.
            let target = unit(m, g, 4.0);
            let taker = unit(m, g, 5.0);
            link(m, taker, target, 95_534, AURA_SPLIT_DAMAGE_FLAT, 45, 127);
            link(m, target, taker, 95_535, AURA_SPLIT_DAMAGE_FLAT, 45, 127);
            let left = auras::share(m, attacker, target, 100, PHYSICAL);
            out.push(left);
            out.push(1000 - health(m, taker));
            out.push(1000 - health(m, target));
            // A taker immune to it loses nothing.
            let target = unit(m, g, 4.0);
            let taker = unit(m, g, 5.0);
            aura_of(m, taker, 95_536, AURA_SCHOOL_IMMUNITY, 127, 0);
            link(m, taker, target, 95_537, AURA_SPLIT_DAMAGE_FLAT, 45, 127);
            out.push(auras::share(m, attacker, target, 100, PHYSICAL));
            out.push(1000 - health(m, taker));
            out
        })
        .await
        .unwrap();
    assert_eq!(got, [25, 30, 55, 45, 0, 55, 0]);
}

#[tokio::test]
async fn a_lethal_share_kills_its_taker_and_swings_and_ticks_are_shared_too() {
    let (g, map) = world().await;
    let got = map
        .query(move |m| {
            let attacker = unit(m, g, 1.0);
            // All of 2000: the taker dies of it.
            let target = unit(m, g, 4.0);
            let taker = unit(m, g, 5.0);
            link(m, taker, target, 95_540, AURA_SPLIT_DAMAGE_PCT, 100, 127);
            auras::share(m, attacker, target, 2000, PHYSICAL);
            let dead = !m.creature(taker).unwrap().alive();
            // A player's swing at one sharing half: half to its taker.
            let target = unit(m, g, 1.0);
            let taker = unit(m, g, 5.0);
            link(m, taker, target, 95_541, AURA_SPLIT_DAMAGE_PCT, 50, 127);
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_f32(UNIT_FIELD_MINDAMAGE, 100.0);
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_f32(UNIT_FIELD_MAXDAMAGE, 100.0);
            m.unit_fields_mut(target)
                .unwrap()
                .set_u32(UNIT_FIELD_RESISTANCES, 0);
            forced_swing(m, g, target, Outcome::Hit);
            let swing = (1000 - health(m, target), 1000 - health(m, taker));
            // A tick of 100 at one sharing 30%: 30 to its taker.
            let target = unit(m, g, 4.0);
            let taker = unit(m, g, 5.0);
            link(m, taker, target, 95_542, AURA_SPLIT_DAMAGE_PCT, 30, 127);
            let mut dot =
                crate::game::tests_p3_spells_kit::made_up(95_543, 6, AURA_PERIODIC_DAMAGE, 0, 100);
            dot.effects[0].amplitude = 1000;
            let _ = auras::apply_spell(m, attacker, target, &dot, &[(0, 100)], Default::default());
            crate::game::tests_p3_spells_kit::tick_now(m, target, 95_543);
            let tick = (1000 - health(m, target), 1000 - health(m, taker));
            (dead, swing, tick)
        })
        .await
        .unwrap();
    assert!(got.0, "the taker dies");
    let (hit, shared) = got.1;
    assert!(hit > 0 && shared > 0, "{hit} {shared}");
    assert!(hit.abs_diff(shared) <= 1, "half each: {hit} {shared}");
    assert_eq!(got.2, (70, 30), "a tick is shared");
}
