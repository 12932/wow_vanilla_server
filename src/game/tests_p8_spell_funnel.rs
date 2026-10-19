//! Round 8, against Thistle Tea's `health_funnel_test.exs` where it is
//! 1.12's: a funnel of health (aura 62, Blood Siphon's and Blood Funnel's)
//! takes from its caster and gives its target the health times its
//! multiplier (mangos-classic's `PeriodicTick`); what an amount over time
//! holds against its caster's damage done; and the coefficient of a DoT
//! with no ticks of its kind.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, pass};
use crate::game::data::GameData;
use crate::game::data::spells::{
    ATTR_EX_CHANNELED, ATTR_EX3_IGNORE_CASTER_MODIFIERS, CUSTOM_FIXED_DAMAGE, SpellDuration,
    SpellEffect, SpellTemplate,
};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::*;
use crate::game::map::spells::{mods, outcome};

/// A funnel of 100 a second for two seconds at an enemy, at a friend, at
/// oneself (twice over), and of a coefficient of a half.
const FUNNEL: u32 = 998_101;
const FRIENDLY: u32 = 998_102;
const SELF: u32 = 998_103;
const POWERED: u32 = 998_104;
/// Damage over time of 100: fixed, past its caster's modifiers, Ignite's;
/// a heal over time and a mana leech of 100.
const FIXED_DOT: u32 = 998_105;
const UNMODIFIED_DOT: u32 = 998_106;
const IGNITE: u32 = 12_654;
const HOT: u32 = 998_107;
const MANA_LEECH: u32 = 998_108;
/// Double damage done; 100 shadow damage; half again and none of a
/// funnel's multiplier (the warlocks' modifiers); a ten-second funnel
/// channelled.
const DOUBLE: u32 = 998_109;
const SHADOW_POWER: u32 = 998_110;
const HALF_AGAIN: u32 = 998_111;
const NONE_OF_IT: u32 = 998_112;
const CHANNELLED: u32 = 998_113;
const LASTS_2S: u32 = 991;
const LASTS_10S: u32 = 992;
const SHADOW: u32 = 5;

fn over_time(id: u32, kind: u32, target: u32) -> SpellTemplate {
    let mut s = bare(id, SHADOW);
    s.family_name = 5;
    s.family_flags = 1;
    s.duration_index = LASTS_2S;
    effect(
        &mut s,
        0,
        SpellEffect {
            amplitude: 1_000,
            bonus_coefficient: 0.0,
            ..aura(kind, 0, 100, target)
        },
    );
    s.effects[0].bonus_coefficient = 0.0;
    s
}

fn buff(id: u32, family: u32, e: SpellEffect) -> SpellTemplate {
    let mut s = bare(id, 0);
    s.family_name = family;
    s.duration_index = LASTS_LONG;
    effect(&mut s, 0, e);
    s
}

fn data(d: &mut GameData) {
    for (i, ms) in [(LASTS_2S, 2_000), (LASTS_10S, 10_000)] {
        d.spells.durations.insert(
            i,
            SpellDuration {
                base: ms,
                per_level: 0,
                max: ms,
            },
        );
    }
    let funnel = over_time(FUNNEL, AURA_PERIODIC_HEALTH_FUNNEL, 6);
    let friendly = over_time(FRIENDLY, AURA_PERIODIC_HEALTH_FUNNEL, 21);
    let mut own = over_time(SELF, AURA_PERIODIC_HEALTH_FUNNEL, 1);
    own.effects[0].multiple_value = 2.0;
    let mut powered = over_time(POWERED, AURA_PERIODIC_HEALTH_FUNNEL, 6);
    powered.effects[0].bonus_coefficient = 0.5;
    let mut fixed_dot = over_time(FIXED_DOT, AURA_PERIODIC_DAMAGE, 6);
    fixed_dot.custom_flags |= CUSTOM_FIXED_DAMAGE;
    let mut unmodified = over_time(UNMODIFIED_DOT, AURA_PERIODIC_DAMAGE, 6);
    unmodified.attributes[3] |= ATTR_EX3_IGNORE_CASTER_MODIFIERS;
    let ignite = over_time(IGNITE, AURA_PERIODIC_DAMAGE, 6);
    let hot = over_time(HOT, AURA_PERIODIC_HEAL, 6);
    let leech = over_time(MANA_LEECH, AURA_PERIODIC_MANA_LEECH, 6);
    let double = buff(DOUBLE, 0, aura(AURA_MOD_DAMAGE_PERCENT_DONE, 127, 100, 1));
    let power = buff(
        SHADOW_POWER,
        0,
        aura(AURA_MOD_DAMAGE_DONE, 1 << SHADOW, 100, 1),
    );
    let modifier = |id: u32, pct: i32| {
        buff(
            id,
            5,
            SpellEffect {
                item_type: 1,
                ..aura(
                    AURA_ADD_PCT_MODIFIER,
                    mods::OP_MULTIPLE_VALUE as i32,
                    pct,
                    1,
                )
            },
        )
    };
    let mut channelled = over_time(CHANNELLED, AURA_PERIODIC_HEALTH_FUNNEL, 6);
    channelled.duration_index = LASTS_10S;
    channelled.attributes[1] |= ATTR_EX_CHANNELED;
    channelled.effects[0].bonus_coefficient = -1.0;
    insert(
        d,
        [
            funnel,
            friendly,
            own,
            powered,
            fixed_dot,
            unmodified,
            ignite,
            hot,
            leech,
            double,
            power,
            modifier(HALF_AGAIN, 50),
            modifier(NONE_OF_IT, -100),
            channelled,
        ],
    );
}

/// `g` at `health` of 10,000.
fn hurt(m: &mut Map, g: Guid, health: u32) {
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_MAXHEALTH, 10_000);
        f.set_u32(UNIT_FIELD_HEALTH, health);
    }
}

/// One tick of `spell` from `a` at `t`: what `a`'s and `t`'s health
/// changed by.
fn one_tick(m: &mut Map, a: Guid, t: Guid, spell: u32) -> (i64, i64) {
    hurt(m, a, 1_000);
    if t != a {
        hurt(m, t, 1_000);
    }
    cast_fresh(m, a, spell, at(t));
    pass(m, 1_000);
    let change = |m: &Map, g: Guid| kit::health(m, g) as i64 - 1_000;
    let out = (change(m, a), change(m, t));
    remove_where(m, t, |h| h.spell == spell);
    out
}

/// A funnel's tick costs its caster its amount and gives its target that
/// times its multiplier: one where the data has none; a funnel on oneself
/// gives back to its caster; spell power adds to it by its coefficient and
/// a modifier of half again to the multiplier makes it half again; one
/// taking it all gives nothing (Thistle Tea's "self-targeted funnels
/// return the drained health through the owner", "snapshots spell power
/// and transfer modifiers independently" and "a zero transfer modifier
/// does not revert to the default ratio"; mangos-classic's
/// `HandlePeriodicHealthFunnel` and `SPELLMOD_MULTIPLE_VALUE`).
#[tokio::test]
async fn a_funnel_costs_its_caster_and_gives_its_target_by_its_multiplier() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P8HF1", "Funneller", Class::Warlock).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[FUNNEL, SELF, POWERED]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let plain = one_tick(m, a, kobold, FUNNEL);
            let own = one_tick(m, a, a, SELF);
            kit::apply(m, a, a, SHADOW_POWER);
            kit::apply(m, a, a, HALF_AGAIN);
            let powered = one_tick(m, a, kobold, POWERED);
            remove_where(m, a, |h| h.spell == HALF_AGAIN);
            kit::apply(m, a, a, NONE_OF_IT);
            let none = one_tick(m, a, kobold, FUNNEL);
            (plain, own, powered, none)
        })
        .await
        .unwrap();
    assert_eq!(out.0, (-100, 100), "plain");
    assert_eq!(out.1, (100, 100), "on oneself, twice over");
    assert_eq!(out.2, (-150, 225), "spell power and half again");
    assert_eq!(out.3, (-100, 0), "none of it");
}

/// A funnel at a friend is a helpful aura, one at an enemy a harmful one
/// (Thistle Tea's "friendly health funnels retain their positive aura
/// polarity").
#[tokio::test]
async fn a_funnel_at_a_friend_is_helpful() {
    let (friendly, hostile) = on_map(data, |m| {
        let s = |id| m.realm.data.spells.get(id).unwrap().clone();
        (positive(&s(FRIENDLY)), positive(&s(FUNNEL)))
    })
    .await;
    assert_eq!((friendly, hostile), (true, false));
}

/// Twice the damage done changes nothing of a fixed DoT's, one past its
/// caster's modifiers, Ignite's burn, a heal over time or a mana leech:
/// each holds its 100 (Thistle Tea's "outgoing damage percentages skip
/// fixed damage and healing").
#[tokio::test]
async fn damage_done_leaves_fixed_amounts_heals_and_leeches_of_mana_alone() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P8HF2", "Doubled", Class::Warlock).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            let all = [FIXED_DOT, UNMODIFIED_DOT, IGNITE, HOT, MANA_LEECH];
            kit::ready(m, a, 60, &all);
            kit::apply(m, a, a, DOUBLE);
            let kobold = kit::dummy(m, a, 60, 5.0);
            all.map(|spell| {
                cast_fresh(m, a, spell, at(kobold));
                kit::holder(m, kobold, spell).and_then(|h| h.effects.first().map(|x| x.amount))
            })
        })
        .await
        .unwrap();
    assert_eq!(out, [Some(100); 5]);
}

/// A funnel channelled for ten seconds, with no tick of damage, healing or
/// a leech to share its coefficient by, shares it by six: two of its
/// seven seconds' worth, a third a tick, not halved as a leech's (Thistle
/// Tea's "uses the vanilla fallback tick count without the leech
/// coefficient penalty"; mangos-classic's `GetSpellAuraMaxTicks`).
#[tokio::test]
async fn a_funnel_shares_its_coefficient_by_six_ticks() {
    let c = on_map(data, |m| {
        let s = m.realm.data.spells.get(CHANNELLED).unwrap().clone();
        outcome::coefficient(m, &s, 0, outcome::Kind::Dot)
    })
    .await;
    assert!((c - 1.0 / 3.0).abs() < 1e-5, "{c}");
}
