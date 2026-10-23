//! Phase 3, round 5, against Thistle Tea's `casting_resources_test.exs`
//! and the health cost of `cast_validation_test.exs`: what a cast's power
//! is checked against, at its start and again as it goes off, by whom.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{
    self as kit, at, casting, fresh, health, pass, power, set_health, set_power,
};
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::AURA_MOD_POWER_COST_SCHOOL;
use crate::game::map::spells::{self, POWER_HEALTH, checks, fail, mods::Mods};

/// A heal of ten on oneself, two seconds to cast, of thirty of a power:
/// one spell for each power and one paid in health.
const HEAL: u32 = 990_500;
const HEALTH_PAID: u32 = 990_510;
const DEAR: u32 = 990_511;
const CHEAPER: u32 = 990_512;

fn heal(id: u32, power: u32, cost: u32) -> SpellTemplate {
    let mut s = bare(id, 2);
    s.power_type = power;
    s.mana_cost = cost;
    s.casting_time_index = CAST_2S;
    s.recovery_time = 5_000;
    s.attributes[2] |= crate::game::data::spells::ATTR_EX2_CANT_CRIT;
    effect(
        &mut s,
        0,
        SpellEffect {
            effect: 10,
            base_points: 9,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        },
    );
    s
}

fn costs(d: &mut GameData) {
    let mut spells: Vec<SpellTemplate> = (0..5).map(|p| heal(HEAL + p, p, 30)).collect();
    spells.push(heal(HEALTH_PAID, POWER_HEALTH, 30));
    spells.push(heal(DEAR, 0, 60));
    let mut cheaper = bare(CHEAPER, 0);
    cheaper.duration_index = LASTS_LONG;
    effect(
        &mut cheaper,
        0,
        aura(AURA_MOD_POWER_COST_SCHOOL, 1 << 2, -30, 1),
    );
    spells.push(cheaper);
    insert(d, spells);
}

fn self_at(g: Guid) -> spells::Targets {
    at(g)
}

/// Cast `spell` at oneself and see it through its two seconds: whether it
/// began, and whether it healed.
fn through(m: &mut Map, g: Guid, spell: u32) -> (bool, bool) {
    fresh(m, g);
    set_health(m, g, 50);
    let began = spells::cast(m, g, spell, self_at(g), None);
    pass(m, 2_100);
    (began, casting(m, g).is_none() && health(m, g) > 50)
}

/// A creature that isn't a pet casts what costs a power it has none of
/// (Thistle Tea's ordinary creatures); a player can't, refused NO_POWER
/// (its players and pets).
#[tokio::test]
async fn a_creature_casts_without_the_pool_and_a_player_is_refused() {
    let realm = fast_realm(costs);
    let (_a, a, map) = human(&realm, "P5CA", "Payer", Class::Warrior).await;
    let out = map
        .query(move |m| {
            let kobold = kit::dummy(m, a, 60, 5.0);
            if let Some(f) = m.unit_fields_mut(kobold) {
                f.set_u32(UNIT_FIELD_MAXHEALTH, 100);
                for i in 0..5 {
                    f.set_u32(UNIT_FIELD_POWER1 + i, 0);
                    f.set_u32(UNIT_FIELD_MAXPOWER1 + i, 0);
                }
            }
            let creature: Vec<_> = (0..5).map(|p| through(m, kobold, HEAL + p)).collect();
            let powers: Vec<_> = (0..5).map(|p| power(m, kobold, p)).collect();
            for p in 0..5 {
                set_power(m, a, p, 0);
            }
            let s = |m: &Map, id| m.realm.data.spells.get(id).cloned().unwrap();
            let player: Vec<_> = (0..5)
                .map(|p| {
                    let spell = s(m, HEAL + p as u32);
                    checks::check(m, a, &spell, &self_at(a), None, false, &Mods::default())
                })
                .collect();
            (creature, powers, player)
        })
        .await
        .unwrap();
    let (creature, powers, player) = out;
    assert!(creature.iter().all(|c| *c == (true, true)), "{creature:?}");
    assert_eq!(powers, [0; 5], "nothing paid");
    assert!(
        player.iter().all(|r| *r == Some(fail::NO_POWER)),
        "{player:?}"
    );
}

/// The cost is checked again as the cast goes off, and worked out again: a
/// creature drained while casting doesn't heal and keeps what it had left;
/// a cost aura arriving meanwhile makes it cheaper (Thistle Tea's mana
/// creature and launch-recalculation tests).
#[tokio::test]
async fn the_cost_is_checked_and_worked_out_again_as_the_cast_goes_off() {
    let realm = fast_realm(costs);
    let (_a, a, map) = human(&realm, "P5CB", "Drainer", Class::Mage).await;
    let out = map
        .query(move |m| {
            let kobold = kit::dummy(m, a, 60, 5.0);
            if let Some(c) = m.creature_mut(kobold) {
                c.base.max_mana = 100;
            }
            let mana = |m: &mut Map, v: u32| {
                if let Some(f) = m.unit_fields_mut(kobold) {
                    f.set_u32(UNIT_FIELD_MAXHEALTH, 100);
                    f.set_u32(UNIT_FIELD_MAXPOWER1, 100);
                    f.set_u32(UNIT_FIELD_POWER1, v);
                }
            };
            // Drained to 59 of the 60 it costs while casting.
            mana(m, 100);
            fresh(m, kobold);
            set_health(m, kobold, 50);
            let began = spells::cast(m, kobold, DEAR, self_at(kobold), None);
            mana(m, 59);
            pass(m, 2_100);
            let drained = (began, health(m, kobold), power(m, kobold, 0));
            // Not drained: it pays.
            mana(m, 100);
            let paid = (through(m, kobold, DEAR), power(m, kobold, 0));
            // A player's cost made cheaper by an aura put on while casting.
            kit::ready(m, a, 60, &[DEAR]);
            set_power(m, a, 0, 100);
            fresh(m, a);
            spells::cast(m, a, DEAR, self_at(a), None);
            kit::apply(m, a, a, CHEAPER);
            pass(m, 2_100);
            let cheaper = power(m, a, 0);
            (drained, paid, cheaper)
        })
        .await
        .unwrap();
    let (drained, paid, cheaper) = out;
    assert_eq!(drained, (true, 50, 59), "no heal, nothing taken");
    assert_eq!(paid, ((true, true), 40));
    assert_eq!(
        cheaper,
        100 - (60 - 30),
        "the cost as it goes off, thirty off"
    );
}

/// A cost in health: refused CASTER_AURASTATE when the health is no more
/// than the cost, at the start and as it goes off, and nothing taken;
/// paid from health else (Thistle Tea's health-powered validation and
/// health-cost failures).
#[tokio::test]
async fn a_cost_in_health_wants_more_health_than_it_takes() {
    let realm = fast_realm(costs);
    let (_a, a, map) = human(&realm, "P5CC", "Tapper", Class::Warlock).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[HEALTH_PAID]);
            if let Some(f) = m.unit_fields_mut(a) {
                f.set_u32(UNIT_FIELD_MAXHEALTH, 1_000);
            }
            let s = m.realm.data.spells.get(HEALTH_PAID).cloned().unwrap();
            let why = |m: &mut Map, h: u32| {
                set_health(m, a, h);
                fresh(m, a);
                checks::check(m, a, &s, &self_at(a), None, false, &Mods::default())
            };
            let checked = (why(m, 31), why(m, 30));
            // Hurt to the cost while casting: nothing taken, no heal.
            set_health(m, a, 100);
            fresh(m, a);
            spells::cast(m, a, HEALTH_PAID, self_at(a), None);
            set_health(m, a, 30);
            pass(m, 2_100);
            let hurt = (casting(m, a), health(m, a));
            // Paid: thirty taken, ten healed.
            set_health(m, a, 100);
            fresh(m, a);
            spells::cast(m, a, HEALTH_PAID, self_at(a), None);
            pass(m, 2_100);
            (checked, hurt, health(m, a), power(m, a, 0))
        })
        .await
        .unwrap();
    let (checked, hurt, paid, mana) = out;
    assert_eq!(checked, (None, Some(fail::CASTER_AURASTATE)));
    assert_eq!(hurt, (None, 30), "failed as it went off");
    assert_eq!(paid, 100 - 30 + 10, "paid in health");
    assert_eq!(mana, 100_000, "not in mana");
}

/// An item's spell costs no power: allowed with none, nothing taken, and
/// no mana spent to stop the spirit's regeneration (Thistle Tea's item
/// casts, mangos's `CheckPower` and `TakePower`).
#[tokio::test]
async fn an_items_spell_costs_no_power() {
    let realm = fast_realm(costs);
    let (_a, a, map) = human(&realm, "P5CD", "Itemer", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            set_power(m, a, 0, 0);
            let s = m.realm.data.spells.get(HEAL).cloned().unwrap();
            // One of what they carry, still carried as the cast ends.
            let item = m.players[&a]
                .items
                .iter()
                .find(|i| i.carried())
                .map(|i| i.guid)
                .expect("an item");
            let checked = (
                checks::check(m, a, &s, &self_at(a), None, false, &Mods::default()),
                checks::check(m, a, &s, &self_at(a), Some(item), false, &Mods::default()),
            );
            set_power(m, a, 0, 100);
            if let Some(p) = m.players.get_mut(&a) {
                p.mana_spent_at = None;
            }
            fresh(m, a);
            set_health(m, a, 50);
            let began = spells::cast(m, a, HEAL, self_at(a), Some(item));
            pass(m, 2_100);
            let spent = m.players.get(&a).and_then(|p| p.mana_spent_at);
            (checked, began, health(m, a), power(m, a, 0), spent)
        })
        .await
        .unwrap();
    let (checked, began, healed, mana, spent) = out;
    assert_eq!(checked, (Some(fail::NO_POWER), None));
    assert!(began);
    assert_eq!(healed, 60);
    assert_eq!(mana, 100, "nothing taken");
    assert_eq!(spent, None, "no five-second rule");
}

/// `.infinitemana` (and `.thereisnospoon`): on fills a GM's mana, a mana
/// spell is allowed with none and takes none, a rage one still costs rage;
/// off, mana is checked and taken again.
#[tokio::test]
async fn infinite_mana_takes_no_mana_until_turned_off() {
    let realm = fast_realm(costs);
    let (mut c, a, map) = human(&realm, "P5IM", "Spoonless", Class::Mage).await;
    map.query(move |m| {
        kit::ready(m, a, 60, &[]);
        set_power(m, a, 0, 0);
    })
    .await
    .unwrap();
    c.say(".thereisnospoon").await;
    c.system_message("Infinite mana on").await;
    let out = map
        .query(move |m| {
            let filled = power(m, a, 0);
            set_power(m, a, 0, 0);
            set_power(m, a, 1, 0);
            let mana = m.realm.data.spells.get(HEAL).cloned().unwrap();
            let rage = m.realm.data.spells.get(HEAL + 1).cloned().unwrap();
            let checked = (
                checks::check(m, a, &mana, &self_at(a), None, false, &Mods::default()),
                checks::check(m, a, &rage, &self_at(a), None, false, &Mods::default()),
            );
            let (began, healed) = through(m, a, HEAL);
            (filled, checked, began, healed, power(m, a, 0))
        })
        .await
        .unwrap();
    let (filled, checked, began, healed, left) = out;
    assert!(filled > 0, "on fills it");
    assert_eq!(checked, (None, Some(fail::NO_POWER)), "mana free, rage not");
    assert!(began && healed);
    assert_eq!(left, 0, "nothing taken");
    c.say(".infinitemana off").await;
    c.system_message("Infinite mana off").await;
    let checked = map
        .query(move |m| {
            set_power(m, a, 0, 0);
            // Another mana spell: the heal's own cooldown is running.
            let mana = m.realm.data.spells.get(DEAR).cloned().unwrap();
            checks::check(m, a, &mana, &self_at(a), None, false, &Mods::default())
        })
        .await
        .unwrap();
    assert_eq!(checked, Some(fail::NO_POWER));
}
