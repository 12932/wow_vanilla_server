//! Phase 3, round 5, against Thistle Tea's `casting_test.exs`: a channel's
//! life, its ticks to the last at its end, what cuts it short, and what
//! its cast time and its own aura do.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, casting, health, holder, pass};
use crate::game::data::GameData;
use crate::game::data::spells::{
    ATTR_EX_CHANNELED, ATTR_EX2_CANT_CRIT, ATTR_EX3_ALWAYS_HIT, SpellEffect, SpellTemplate,
};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{AURA_DUMMY, AURA_PERIODIC_TRIGGER_SPELL};
use crate::game::map::spells;

/// Five seconds of a bolt a second at the channel's target (Arcane
/// Missiles' kind); the same after two seconds' cast; one cut short by
/// damage; a channel of a plain aura on its caster (Evocation's kind).
const MISSILES: u32 = 990_700;
const SLOW_MISSILES: u32 = 990_701;
const PUSHED_MISSILES: u32 = 990_702;
const BOLT: u32 = 990_703;
const EVOCATION: u32 = 990_704;

fn channels(d: &mut GameData) {
    let missiles = |id: u32| {
        let mut s = bare(id, 6);
        s.attributes[1] |= ATTR_EX_CHANNELED;
        s.duration_index = LASTS_5S;
        s.channel_interrupt_flags = 0x8;
        effect(
            &mut s,
            0,
            SpellEffect {
                trigger_spell: BOLT,
                amplitude: 1_000,
                ..aura(AURA_PERIODIC_TRIGGER_SPELL, 0, 0, 1)
            },
        );
        s
    };
    let mut slow = missiles(SLOW_MISSILES);
    slow.casting_time_index = CAST_2S;
    let mut pushed = missiles(PUSHED_MISSILES);
    pushed.channel_interrupt_flags |= 0x4000;
    let mut bolt = bare(BOLT, 6);
    bolt.attributes[2] |= ATTR_EX2_CANT_CRIT;
    bolt.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
    effect(&mut bolt, 0, damage(10));
    let mut evocation = bare(EVOCATION, 6);
    evocation.attributes[1] |= ATTR_EX_CHANNELED;
    evocation.duration_index = LASTS_5S;
    effect(&mut evocation, 0, aura(AURA_DUMMY, 0, 0, 1));
    let spells: Vec<SpellTemplate> = vec![missiles(MISSILES), slow, pushed, bolt, evocation];
    insert(d, spells);
}

/// The bolts the kobold has taken.
fn bolts(m: &Map, kobold: Guid) -> u32 {
    (1_000_000 - health(m, kobold)) / 10
}

fn channel_spell(m: &Map, g: Guid) -> (u32, Guid) {
    m.unit_fields(g).map_or((0, Guid::zero()), |f| {
        (
            f.get_u32(UNIT_CHANNEL_SPELL),
            f.get_guid(UNIT_FIELD_CHANNEL_OBJECT),
        )
    })
}

/// A self channel's periodic trigger goes at its target each second, the
/// last at its very end though the update comes late, and never again
/// after; then the channel is over, its fields cleared. Cancelled, it
/// delivers nothing more (Thistle Tea's `cast_tick/3`: the completion
/// tick, the late callback, the cancelled channel, ending a channel, and
/// the periodic trigger).
#[tokio::test]
async fn a_channel_ticks_to_its_end_and_a_cancelled_one_stops() {
    let realm = fast_realm(channels);
    let (_a, a, map) = human(&realm, "P5HA", "Channeler", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[MISSILES]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let began = cast_fresh(m, a, MISSILES, at(kobold));
            let fields = channel_spell(m, a);
            let mut seen = Vec::new();
            for step in [1_000, 1_000, 1_000, 1_000, 1_050] {
                pass(m, step);
                seen.push(bolts(m, kobold));
            }
            let over = (casting(m, a), channel_spell(m, a));
            pass(m, 2_000);
            let after = bolts(m, kobold);
            // Cancelled after its first tick.
            let other = kit::dummy(m, a, 60, 6.0);
            cast_fresh(m, a, MISSILES, at(other));
            pass(m, 1_000);
            pass(m, 500);
            spells::interrupt(m, a);
            let cancelled = (casting(m, a), channel_spell(m, a).0);
            for _ in 0..6 {
                pass(m, 1_000);
            }
            (
                began,
                fields,
                kobold,
                seen,
                over,
                after,
                cancelled,
                bolts(m, other),
            )
        })
        .await
        .unwrap();
    let (began, fields, kobold, seen, over, after, cancelled, other) = out;
    assert!(began);
    assert_eq!(fields, (MISSILES, kobold), "the channel and its object");
    assert_eq!(
        seen,
        [1, 2, 3, 4, 5],
        "a bolt a second, the last at its end"
    );
    assert_eq!(over, (None, (0, Guid::zero())), "over, its fields cleared");
    assert_eq!(after, 5, "none after");
    assert_eq!(cancelled, (None, 0));
    assert_eq!(other, 1, "nothing more once cancelled");
}

/// A channel with a cast time casts first and channels after (Thistle
/// Tea's "waits for a channel's cast time"); one cut short by damage loses
/// the ticks past its new end (its "shortened channel deadline").
#[tokio::test]
async fn a_channel_waits_for_its_cast_and_a_shortened_one_loses_its_last_ticks() {
    let realm = fast_realm(channels);
    let (_a, a, map) = human(&realm, "P5HB", "Slowcaster", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[SLOW_MISSILES, PUSHED_MISSILES]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            cast_fresh(m, a, SLOW_MISSILES, at(kobold));
            pass(m, 1_000);
            let casting_first = (casting(m, a), channel_spell(m, a).0, bolts(m, kobold));
            pass(m, 1_100);
            let channelling = channel_spell(m, a).0;
            for _ in 0..6 {
                pass(m, 1_000);
            }
            let slow = bolts(m, kobold);
            // Pushed back a second, once, between its first and second.
            let other = kit::dummy(m, a, 60, 6.0);
            cast_fresh(m, a, PUSHED_MISSILES, at(other));
            pass(m, 1_000);
            pass(m, 500);
            spells::pushback_by(m, a, other);
            for _ in 0..6 {
                pass(m, 1_000);
            }
            (casting_first, channelling, slow, bolts(m, other))
        })
        .await
        .unwrap();
    let (casting_first, channelling, slow, pushed) = out;
    assert_eq!(casting_first, (Some(SLOW_MISSILES), 0, 0), "cast first");
    assert_eq!(channelling, SLOW_MISSILES, "then channelled");
    assert_eq!(slow, 5);
    assert_eq!(pushed, 4, "a second shorter: its last tick lost");
}

/// A channel's own plain aura goes on once at its start, not again at
/// each tick, and goes with the channel (Thistle Tea's "does not re-apply
/// plain channel auras"; wvs takes it off at the channel's end).
#[tokio::test]
async fn a_channels_own_aura_goes_on_once() {
    let realm = fast_realm(channels);
    let (_a, a, map) = human(&realm, "P5HC", "Evoker", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[EVOCATION]);
            cast_fresh(m, a, EVOCATION, Default::default());
            let first = holder(m, a, EVOCATION).and_then(|h| h.expires);
            pass(m, 1_000);
            pass(m, 1_000);
            let later = holder(m, a, EVOCATION).and_then(|h| h.expires);
            for _ in 0..4 {
                pass(m, 1_000);
            }
            (
                first.is_some(),
                first == later,
                holder(m, a, EVOCATION).is_some(),
            )
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true, false));
}
