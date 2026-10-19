//! Phase 3, round 6, against Thistle Tea's `casting_test.exs`: the order
//! of a cast's packets, charged modifiers and the casts they change, a
//! channel's first tick, and the victim's own change to a spell's hit.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, casting, has, pass, power};
use crate::game::data::GameData;
use crate::game::data::spells::{
    ATTR_EX_CHANNELED, ATTR_EX2_CANT_CRIT, ATTR_EX3_ALWAYS_HIT, SpellEffect, SpellTemplate,
};
use crate::game::map::auras::{
    AURA_ADD_PCT_MODIFIER, AURA_MOD_ATTACKER_SPELL_HIT_CHANCE, AURA_PERIODIC_TRIGGER_SPELL,
};
use crate::game::map::spells::{self, mods};

/// An instant; a ten-second Pyroblast and the next one made instant; a
/// three-second channel of 45 mana (Mind Flay's kind), a 10-mana heal, and
/// the next of them free; a channel of a bolt a second.
const INSTANT_BOLT: u32 = 991_010;
const PYROBLAST: u32 = 991_011;
const PRESENCE: u32 = 991_012;
const FLAY: u32 = 991_013;
const FLASH: u32 = 991_014;
const FOCUS: u32 = 991_015;
const MISSILES: u32 = 991_016;
const MISSILE: u32 = 991_017;
const SENSES: u32 = 991_018;

fn data(d: &mut GameData) {
    let modifier = |id: u32, family: u32, mask: u64, op: usize| {
        let mut s = bare(id, 0);
        s.family_name = family;
        s.duration_index = LASTS_LONG;
        s.proc_charges = 1;
        effect(
            &mut s,
            0,
            SpellEffect {
                item_type: mask,
                ..aura(AURA_ADD_PCT_MODIFIER, op as i32, -100, 1)
            },
        );
        s
    };
    let mut instant = bare(INSTANT_BOLT, 2);
    effect(&mut instant, 0, damage(10));
    let mut pyroblast = bare(PYROBLAST, 2);
    pyroblast.family_name = 3;
    pyroblast.family_flags = 0x4000_0000;
    pyroblast.casting_time_index = CAST_10S;
    effect(&mut pyroblast, 0, damage(10));
    let mut flay = bare(FLAY, 5);
    flay.family_name = 6;
    flay.family_flags = 0x1;
    flay.mana_cost = 45;
    flay.attributes[1] |= ATTR_EX_CHANNELED;
    flay.duration_index = LASTS_5S;
    let mut flash = bare(FLASH, 1);
    flash.family_name = 6;
    flash.family_flags = 0x1;
    flash.mana_cost = 10;
    effect(
        &mut flash,
        0,
        SpellEffect {
            effect: 10,
            base_points: 9,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        },
    );
    let mut missiles = bare(MISSILES, 6);
    missiles.attributes[1] |= ATTR_EX_CHANNELED;
    missiles.duration_index = LASTS_5S;
    effect(
        &mut missiles,
        0,
        SpellEffect {
            trigger_spell: MISSILE,
            amplitude: 1_000,
            ..aura(AURA_PERIODIC_TRIGGER_SPELL, 0, 0, 1)
        },
    );
    let mut missile = bare(MISSILE, 6);
    missile.attributes[2] |= ATTR_EX2_CANT_CRIT;
    missile.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
    effect(&mut missile, 0, damage(10));
    let mut senses = bare(SENSES, 0);
    senses.duration_index = LASTS_LONG;
    effect(
        &mut senses,
        0,
        aura(AURA_MOD_ATTACKER_SPELL_HIT_CHANCE, 0x7E, -2, 1),
    );
    let spells: Vec<SpellTemplate> = vec![
        instant,
        pyroblast,
        modifier(PRESENCE, 3, 0x4000_0000, mods::OP_CASTING_TIME),
        flay,
        flash,
        modifier(FOCUS, 6, 0x1, mods::OP_COST),
        missiles,
        missile,
        senses,
    ];
    insert(d, spells);
}

/// A cast's client hears its result before its go, as mangos's
/// `Spell::cast` sends them and Thistle Tea queues them.
#[tokio::test]
async fn the_cast_result_comes_before_its_go() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P6CC", "Ordered", Class::Mage).await;
    map.query(move |m| {
        kit::ready(m, a, 60, &[INSTANT_BOLT]);
        let kobold = kit::dummy(m, a, 60, 5.0);
        cast_fresh(m, a, INSTANT_BOLT, at(kobold));
    })
    .await
    .unwrap();
    let _ = kit::saw_go(&mut c, INSTANT_BOLT).await;
    let frames = c.captured();
    let opcode = |f: &[u8]| u16::from_le_bytes([f[2], f[3]]);
    let result = frames.iter().position(|f| {
        opcode(f) == spells::SMSG_CAST_RESULT && f[4..8] == INSTANT_BOLT.to_le_bytes()
    });
    // The only go: a packed guid comes before its spell.
    let go = frames.iter().position(|f| opcode(f) == 0x132);
    assert!(result.is_some() && go.is_some(), "{result:?} {go:?}");
    assert!(result < go, "the result first");
}

/// A charged modifier is spent by the cast it changes: Presence of Mind's
/// kind makes a ten-second cast instant and goes with it; Inner Focus's
/// makes a channel free and goes as the channel starts, and a heal free
/// and goes as it is cast (Thistle Tea's `Casting.start` and `complete`
/// with charged modifiers; mangos's `SpellModifier` charges).
#[tokio::test]
async fn a_charged_modifier_goes_with_the_cast_it_changes() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6CD", "Charged", Class::Priest).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[PYROBLAST, FLAY, FLASH]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            kit::apply(m, a, a, PRESENCE);
            let before = kit::health(m, kobold);
            cast_fresh(m, a, PYROBLAST, at(kobold));
            let instant = (
                casting(m, a),
                before - kit::health(m, kobold),
                has(m, a, PRESENCE),
            );
            kit::apply(m, a, a, FOCUS);
            let mana = power(m, a, 0);
            cast_fresh(m, a, FLAY, Default::default());
            let channel = (
                m.caster(a)
                    .and_then(|c| c.casting.as_ref())
                    .is_some_and(|c| c.channel_until.is_some()),
                mana - power(m, a, 0),
                has(m, a, FOCUS),
            );
            spells::interrupt(m, a);
            kit::apply(m, a, a, FOCUS);
            let mana = power(m, a, 0);
            cast_fresh(m, a, FLASH, Default::default());
            let heal = (mana - power(m, a, 0), has(m, a, FOCUS));
            (instant, channel, heal)
        })
        .await
        .unwrap();
    let (instant, channel, heal) = out;
    assert_eq!(instant, (None, 10, false), "instant, landed, spent");
    assert_eq!(
        channel,
        (true, 0, false),
        "a free channel, spent as it starts"
    );
    assert_eq!(heal, (0, false), "a free heal, spent");
}

/// A channel's periodic trigger waits for its first period: nothing at
/// its start nor just before, its first bolt a second in (Thistle Tea's
/// "waits for the scheduled tick").
#[tokio::test]
async fn a_channels_trigger_waits_for_its_first_period() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6CE", "Patient", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[MISSILES]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let bolts = |m: &crate::game::map::Map| (1_000_000 - kit::health(m, kobold)) / 10;
            cast_fresh(m, a, MISSILES, at(kobold));
            let at_start = bolts(m);
            pass(m, 999);
            let before = bolts(m);
            pass(m, 1);
            (at_start, before, bolts(m))
        })
        .await
        .unwrap();
    assert_eq!(out, (0, 0, 1));
}

/// The victim's own change to its attackers' spells' chance to hit, of the
/// schools of its mask, comes off the spell's chance: 2% less of a shadow
/// spell, none off a physical one (Thistle Tea's school-masked attacker
/// hit chance; mangos's `SPELL_AURA_MOD_ATTACKER_SPELL_HIT_CHANCE`).
#[tokio::test]
async fn a_victims_school_hit_change_comes_off_a_spells_chance() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6CF", "Sensed", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let chance = |m: &crate::game::map::Map, school: u32| {
                let s = bare(1, school);
                spells::outcome::spell_hit_chance(m, a, kobold, &s, &mods::Mods::default())
            };
            let plain = (chance(m, 5), chance(m, 0));
            kit::apply(m, kobold, kobold, SENSES);
            (plain, (chance(m, 5), chance(m, 0)))
        })
        .await
        .unwrap();
    let ((shadow, physical), (sensed_shadow, sensed_physical)) = out;
    assert_eq!(shadow - sensed_shadow, 200, "2% off shadow");
    assert_eq!(physical, sensed_physical, "none off physical");
}
