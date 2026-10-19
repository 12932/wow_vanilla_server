//! Round 8, against Thistle Tea's `binary_spell_test.exs` and
//! `area_spell_avoidance_test.exs`: a binary spell's hit against its
//! school's resistance, all or nothing; an area spell's against the
//! target's avoidance of areas (mangos-classic's `MagicSpellHitChance`,
//! vmangos's binary resistance).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, pass};
use crate::game::data::GameData;
use crate::game::data::spells::{
    ATTR_EX_CHANNELED, ATTR_EX3_ALWAYS_HIT, SpellEffect, SpellTemplate,
};
use crate::game::map::Map;
use crate::game::map::auras::*;
use crate::game::map::spells::{self, mods::Mods, outcome};

/// A frostbolt of 100 that slows; its damage alone; a channel of 100 a
/// second that slows; the frostbolt sure to hit; a fire bolt of the
/// frostbolt's kind.
const FROSTBOLT: u32 = 998_301;
const BOLT_ONLY: u32 = 998_302;
const FROST_CHANNEL: u32 = 998_303;
const SURE: u32 = 998_304;
const FIREBOLT: u32 = 998_305;
/// 300 frost resistance and 100 fire; 300 frost resistance taken off; 25
/// and 100 avoidance of areas.
const FROST_300: u32 = 998_306;
const FIRE_100: u32 = 998_307;
const PIERCE_300: u32 = 998_308;
const AVOID_25: u32 = 998_309;
const AVOID_100: u32 = 998_310;
/// Fire of 100 around the caster, at one target, and at one with a chain
/// and a radius.
const NOVA: u32 = 998_311;
const SINGLE: u32 = 998_312;
const CHAINED: u32 = 998_313;
const RADIUS_10: u32 = 995;

const FROST: u32 = 4;
const FIRE: u32 = 2;

fn slow() -> SpellEffect {
    aura(AURA_MOD_DECREASE_SPEED, 0, -50, 6)
}

fn buff(id: u32, e: SpellEffect) -> SpellTemplate {
    let mut s = bare(id, 0);
    s.duration_index = LASTS_LONG;
    effect(&mut s, 0, e);
    s
}

fn data(d: &mut GameData) {
    d.spells.radii.insert(RADIUS_10, 10.0);
    let mut frostbolt = bare(FROSTBOLT, FROST);
    frostbolt.duration_index = LASTS_5S;
    effect(&mut frostbolt, 0, damage(100));
    effect(&mut frostbolt, 1, slow());
    let mut bolt_only = frostbolt.clone();
    bolt_only.id = BOLT_ONLY;
    bolt_only.effects[1] = SpellEffect {
        bonus_coefficient: -1.0,
        ..Default::default()
    };
    let mut sure = frostbolt.clone();
    sure.id = SURE;
    sure.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
    let mut firebolt = frostbolt.clone();
    firebolt.id = FIREBOLT;
    firebolt.school = FIRE;
    let mut channel = bare(FROST_CHANNEL, FROST);
    channel.attributes[1] |= ATTR_EX_CHANNELED;
    channel.duration_index = LASTS_5S;
    effect(
        &mut channel,
        0,
        SpellEffect {
            amplitude: 1_000,
            ..aura(AURA_PERIODIC_DAMAGE, 0, 100, 6)
        },
    );
    effect(&mut channel, 1, slow());
    let mut nova = bare(NOVA, FIRE);
    effect(
        &mut nova,
        0,
        SpellEffect {
            target_a: 22,
            target_b: 15,
            radius_index: RADIUS_10,
            ..damage(100)
        },
    );
    let mut single = bare(SINGLE, FIRE);
    effect(&mut single, 0, damage(100));
    let mut chained = single.clone();
    chained.id = CHAINED;
    chained.effects[0].chain_target = 3;
    chained.effects[0].radius_index = RADIUS_10;
    insert(
        d,
        [
            frostbolt,
            bolt_only,
            sure,
            firebolt,
            channel,
            nova,
            single,
            chained,
            buff(FROST_300, aura(AURA_MOD_RESISTANCE, 1 << FROST, 300, 1)),
            buff(FIRE_100, aura(AURA_MOD_RESISTANCE, 1 << FIRE, 100, 1)),
            buff(
                PIERCE_300,
                aura(AURA_MOD_TARGET_RESISTANCE, 1 << FROST, -300, 1),
            ),
            buff(AVOID_25, aura(AURA_MOD_AOE_AVOIDANCE, 0, 25, 1)),
            buff(AVOID_100, aura(AURA_MOD_AOE_AVOIDANCE, 0, 100, 1)),
        ],
    );
}

fn template(m: &Map, id: u32) -> SpellTemplate {
    m.realm.data.spells.get(id).unwrap().clone()
}

fn chance(m: &Map, a: Guid, t: Guid, id: u32) -> i32 {
    outcome::spell_hit_chance(m, a, t, &template(m, id), &Mods::default())
}

/// A binary spell's chance to hit is cut by its school's resistance alone
/// (300 at level 60 leaves a quarter: 24%), what the caster's gear takes
/// off it counts (all 96% again), another school's doesn't (100 fire
/// leaves 72%), and a creature's level adds no resistance of its own (83%
/// three levels up); its damage alone isn't binary; one sure to hit hits
/// (Thistle Tea's "uses matching resistance and penetration without
/// adding innate creature resistance"); the resistance is read as it is
/// (its "launch reads resistance metadata and observes its removal").
#[tokio::test]
async fn a_binary_spells_hit_is_cut_by_its_schools_resistance() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P8BS1", "Binary", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            kit::apply(m, kobold, kobold, FROST_300);
            kit::apply(m, kobold, kobold, FIRE_100);
            let resisted = chance(m, a, kobold, FROSTBOLT);
            let damage_only = chance(m, a, kobold, BOLT_ONLY);
            let s = template(m, SURE);
            let sure = outcome::roll_hit(m, a, kobold, &s, &Mods::default());
            kit::apply(m, a, a, PIERCE_300);
            let pierced = chance(m, a, kobold, FROSTBOLT);
            let fire = chance(m, a, kobold, FIREBOLT);
            let higher = kit::dummy(m, a, 63, 5.0);
            kit::apply(m, higher, higher, FROST_300);
            let higher = chance(m, a, higher, FROSTBOLT);
            remove_where(m, a, |h| h.spell == PIERCE_300);
            remove_where(m, kobold, |h| h.spell == FROST_300);
            let removed = chance(m, a, kobold, FROSTBOLT);
            (resisted, damage_only, sure, pierced, fire, higher, removed)
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (2_400, 9_600, outcome::Hit::Hit, 9_600, 7_200, 8_300, 9_600)
    );
}

/// A binary spell lands whole, its damage and its slow, or not at all: a
/// resistance never takes a part of it, a tick of a binary channel
/// neither, and a channel's aura gone ticks no more (Thistle Tea's "a
/// successful binary spell lands full damage and its slow while a resist
/// lands neither" and "binary channel damage skips partial resistance and
/// cancellation stops further ticks").
#[tokio::test]
async fn a_binary_spell_lands_whole_or_not_at_all() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P8BS2", "Wholly", Class::Mage).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(true);
            kit::ready(m, a, 60, &[FROSTBOLT, FROST_CHANNEL]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            kit::apply(m, kobold, kobold, FROST_300);
            let mut outcomes = Vec::new();
            for _ in 0..60 {
                remove_where(m, kobold, |h| h.spell == FROSTBOLT);
                kit::set_health(m, kobold, 1_000_000);
                cast_fresh(m, a, FROSTBOLT, at(kobold));
                outcomes.push((
                    1_000_000 - kit::health(m, kobold),
                    kit::has(m, kobold, FROSTBOLT),
                ));
            }
            // A channel's ticks, until it is taken off.
            let mut ticks = Vec::new();
            let mut landed = false;
            for _ in 0..60 {
                cast_fresh(m, a, FROST_CHANNEL, at(kobold));
                if kit::has(m, kobold, FROST_CHANNEL) {
                    landed = true;
                    break;
                }
            }
            for _ in 0..3 {
                kit::set_health(m, kobold, 1_000_000);
                pass(m, 1_000);
                ticks.push(1_000_000 - kit::health(m, kobold));
            }
            spells::interrupt(m, a);
            remove_where(m, kobold, |h| h.spell == FROST_CHANNEL);
            kit::set_health(m, kobold, 1_000_000);
            pass(m, 1_000);
            let after = 1_000_000 - kit::health(m, kobold);
            (outcomes, landed, ticks, after)
        })
        .await
        .unwrap();
    let (outcomes, landed, ticks, after) = out;
    assert!(
        // A crit's half again is no part resisted.
        outcomes
            .iter()
            .all(|o| matches!(o, (100 | 150, true) | (0, false))),
        "{outcomes:?}"
    );
    assert!(outcomes.contains(&(100, true)) && outcomes.contains(&(0, false)));
    assert!(landed, "the channel never landed");
    assert_eq!(ticks, vec![100; 3], "whole ticks");
    assert_eq!(after, 0, "none once gone");
}

/// A spell with any effect of an area is an area spell, one at a target
/// with a chain isn't (Thistle Tea's "retains any effect's area
/// classification"); avoidance of areas cuts an area spell's chance and
/// not a single target's, as it is at the cast: an area spell avoided
/// does nothing, one that lands its whole damage (its "landed area spells
/// retain full damage and saved misses deal none" and "normal casts read
/// current avoidance and observe its removal").
#[tokio::test]
async fn avoidance_of_areas_cuts_an_area_spells_chance_as_it_is() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P8AS1", "Avoided", Class::Mage).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[NOVA]);
            let mut mixed = template(m, SINGLE);
            mixed.effects[1] = template(m, NOVA).effects[0];
            let kinds = (
                outcome::area(&mixed),
                outcome::area(&template(m, SINGLE)),
                outcome::area(&template(m, CHAINED)),
            );
            let kobold = kit::dummy(m, a, 60, 5.0);
            kit::apply(m, kobold, kobold, AVOID_25);
            let chances = (chance(m, a, kobold, NOVA), chance(m, a, kobold, SINGLE));
            remove_where(m, kobold, |h| h.spell == AVOID_25);
            kit::apply(m, kobold, kobold, AVOID_100);
            cast_fresh(m, a, NOVA, Default::default());
            let avoided = 1_000_000 - kit::health(m, kobold);
            remove_where(m, kobold, |h| h.spell == AVOID_100);
            cast_fresh(m, a, NOVA, Default::default());
            let landed = 1_000_000 - kit::health(m, kobold);
            (kinds, chances, avoided, landed)
        })
        .await
        .unwrap();
    assert_eq!(out, ((true, false, false), (7_100, 9_600), 0, 100));
}
