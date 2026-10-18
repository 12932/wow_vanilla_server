//! Phase 3's fifth round of combat tests, what an ability's outcome gives
//! back (Thistle Tea's `attack_feedback_test.exs`): the rage and energy an
//! avoided one refunds, the combo points a builder adds once and a finisher
//! keeps when it is avoided.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{
    ATTR_EX_DISCOUNT_POWER_ON_MISS, ATTR_EX_REQ_COMBO_POINTS, SpellEffect, SpellTemplate,
};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::spells::{self, Targets};
use crate::game::map::{Map, combat, procs};
use crate::game::tests_p3_combat::{self as base, *};

/// Rend for rage, Sinister Strike for energy, both refunding on a miss;
/// one for energy that doesn't; one for mana that says it does;
/// Eviscerate.
const RAGE_STRIKE: u32 = 90_800;
const BUILDER: u32 = 90_801;
const PLAIN: u32 = 90_802;
const MANA_STRIKE: u32 = 90_803;
const FINISHER: u32 = 90_804;

fn ability(id: u32, power_type: u32, cost: u32, refunds: bool) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Ability {id}"),
        range_index: 1,
        equipped_item_class: -1,
        dmg_class: 2,
        power_type,
        mana_cost: cost,
        ..Default::default()
    };
    if refunds {
        s.attributes[1] |= ATTR_EX_DISCOUNT_POWER_ON_MISS;
    }
    s.effects[0] = SpellEffect {
        effect: 2,
        base_points: 9,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    s
}

fn data() -> GameData {
    let mut d = base::data();
    let mut builder = ability(BUILDER, 3, 40, true);
    builder.effects[1] = SpellEffect {
        effect: spells::EFFECT_ADD_COMBO_POINTS,
        base_points: 0,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    let mut finisher = ability(FINISHER, 3, 35, false);
    finisher.attributes[1] |= ATTR_EX_REQ_COMBO_POINTS;
    finisher.effects[0].points_per_combo_point = 10.0;
    for s in [
        ability(RAGE_STRIKE, 1, 150, true),
        builder,
        ability(PLAIN, 3, 40, false),
        ability(MANA_STRIKE, 0, 150, true),
        finisher,
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn power(m: &Map, g: Guid, power: u16) -> u32 {
    m.unit_fields(g)
        .map_or(0, |f| f.get_u32(UNIT_FIELD_POWER1 + power))
}

fn set_power(m: &mut Map, g: Guid, power: u16, to: u32) {
    let f = m.unit_fields_mut(g).unwrap();
    f.set_u32(UNIT_FIELD_MAXPOWER1 + power, 1000);
    f.set_u32(UNIT_FIELD_POWER1 + power, to);
}

/// Thistle Tea's "energy builders refund miss dodge parry and immunity
/// using the paid cost", "rage refunds only dodge and parry and mana is
/// never refunded", "free casts ... cannot create power" and "abilities
/// without the refund attribute get nothing back on dodge": an ability
/// that discounts its power on a miss gives back 82% of what it was paid,
/// energy on a miss, dodge, parry or immunity and rage on a dodge or parry
/// only, nothing on a block, a resist, an evade, a reflect or a hit; mana
/// never; nothing for a cast paid nothing, nor for an ability without the
/// attribute.
#[tokio::test]
async fn an_avoided_ability_gives_back_its_rage_or_energy_as_its_outcome_allows() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Rogue).await;
    let r = map
        .query(move |m| {
            let back = |m: &mut Map, spell: u32, power_type: u16, ex: u32, paid: u32| {
                set_power(m, ga, power_type, 0);
                combat::ability_avoided(m, ga, spell, ex, paid);
                power(m, ga, power_type)
            };
            let outcomes = [
                procs::EX_MISS,
                procs::EX_DODGE,
                procs::EX_PARRY,
                procs::EX_IMMUNE,
                procs::EX_BLOCK,
                procs::EX_RESIST,
                procs::EX_EVADE,
                procs::EX_REFLECT,
                procs::EX_NORMAL_HIT,
            ];
            let energy: Vec<u32> = outcomes
                .iter()
                .map(|ex| back(m, BUILDER, 3, *ex, 40))
                .collect();
            let rage: Vec<u32> = outcomes
                .iter()
                .map(|ex| back(m, RAGE_STRIKE, 1, *ex, 150))
                .collect();
            let mana: Vec<u32> = outcomes
                .iter()
                .map(|ex| back(m, MANA_STRIKE, 0, *ex, 150))
                .collect();
            let free = back(m, BUILDER, 3, procs::EX_DODGE, 0);
            let plain = back(m, PLAIN, 3, procs::EX_DODGE, 40);
            (energy, rage, mana, free, plain)
        })
        .await
        .unwrap();
    let (energy, rage, mana, free, plain) = r;
    assert_eq!(energy, [33, 33, 33, 33, 0, 0, 0, 0, 0], "energy");
    assert_eq!(rage, [0, 123, 123, 0, 0, 0, 0, 0, 0], "rage");
    assert_eq!(mana, [0; 9], "mana");
    assert_eq!(free, 0, "a free cast");
    assert_eq!(plain, 0, "no refund attribute");
}

/// The brute, dodging all or nothing.
fn dodging(m: &mut Map, brute: Guid, all: bool) {
    m.creature_mut(brute).unwrap().derived.dodge = if all { 100.0 } else { 0.0 };
}

fn cast(m: &mut Map, g: Guid, spell: u32, at: Guid) {
    let p = m.players.get_mut(&g).unwrap();
    p.caster.casting = None;
    p.caster.gcd_until = None;
    p.caster.cooldowns.clear();
    spells::cast(
        m,
        g,
        spell,
        Targets {
            unit: Some(at),
            ..Default::default()
        },
        None,
    );
}

fn combo(m: &Map, g: Guid) -> (u8, bool) {
    let c = &m.players[&g].caster;
    (c.combo_points, !c.combo_target.is_zero())
}

/// Thistle Tea's "builder feedback does not duplicate the resolved effect
/// award", "finishers consume points on hit but retain them on avoidance"
/// and "avoided finishers retain points and spend their full cost": a
/// builder that lands adds its one point, once, and one dodged adds none
/// and gives back its energy; a finisher dodged keeps all five points and
/// what it cost; one that lands spends them.
#[tokio::test]
async fn a_builder_adds_its_point_once_and_an_avoided_finisher_keeps_them_all() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Rogue).await;
    let r = map
        .query(move |m| {
            for s in [BUILDER, FINISHER] {
                m.players.get_mut(&ga).unwrap().spells.push(s);
            }
            let brute = spawn(m, ga, BRUTE, 10, 2.0);
            dodging(m, brute, false);
            set_power(m, ga, 3, 100);
            cast(m, ga, BUILDER, brute);
            let landed = (combo(m, ga), power(m, ga, 3));
            dodging(m, brute, true);
            cast(m, ga, BUILDER, brute);
            let dodged = (combo(m, ga), power(m, ga, 3));
            let p = m.players.get_mut(&ga).unwrap();
            p.caster.combo_points = 5;
            set_power(m, ga, 3, 65);
            cast(m, ga, FINISHER, brute);
            let kept = (combo(m, ga), power(m, ga, 3));
            dodging(m, brute, false);
            set_power(m, ga, 3, 65);
            cast(m, ga, FINISHER, brute);
            let spent = combo(m, ga);
            (landed, dodged, kept, spent)
        })
        .await
        .unwrap();
    let (landed, dodged, kept, spent) = r;
    assert_eq!(landed, ((1, true), 60), "a point, once");
    assert_eq!(dodged, ((1, true), 53), "no point, 33 of 40 back");
    assert_eq!(kept, ((5, true), 30), "all five kept, the cost paid");
    assert_eq!(spent.0, 0, "spent on a hit");
}
