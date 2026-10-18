//! Phase 3's periodic auras, Thistle Tea's `PeriodicModifiersTest` and
//! `PercentDamageTest` ported: activation-time modifiers on a periodic's
//! cadence, what a refresh keeps of it, and a percentage of health a tick.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{self as sd, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::spells::{self, mods};
use crate::game::tests_p3_aura2::{add_spells, aura_spell, has, health, pass, put, set};
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};

/// A mage's DoT of four seconds' amplitude for ten (family 3, flag 0x1);
/// one of another mask; one that stacks to three; an immediate pulse
/// (Earthbind's id); one whose rhythm a refresh keeps (13797's).
const DOT: u32 = 96_900;
const OTHER_MASK: u32 = 96_901;
const STACKING: u32 = 96_902;
const PULSE: u32 = 8145;
const PRESERVED: u32 = 13797;
/// Two seconds off the activation time of the mage's flag 0x1, and the
/// same with one charge.
const HASTEN: u32 = 96_903;
const HASTEN_ONCE: u32 = 96_904;
/// A tenth of the maximum health a second, a ward of physical, and an
/// immunity to its kind; and a tenth a tick with a heal of 200 beside it.
const PERCENT: u32 = 96_905;
const SHIELD: u32 = 96_906;
const IMMUNE: u32 = 96_907;
const PERCENT_HEALED: u32 = 96_908;

fn dot(id: u32, flags: u64) -> SpellTemplate {
    let mut s = aura_spell(id, Some(10_000), &[(AURA_PERIODIC_DAMAGE, 0, 4000)]);
    s.family_name = 3;
    s.family_flags = flags;
    s.effects[0].target_a = 6;
    s.effects[0].base_points = 9;
    s
}

fn data() -> GameData {
    let mut d = crate::game::tests_p3_aura2::data();
    let mut stacking = dot(STACKING, 0x1);
    stacking.stack_amount = 3;
    let mut hasten = Vec::new();
    for (id, charges) in [(HASTEN, 0), (HASTEN_ONCE, 1)] {
        let mut s = aura_spell(
            id,
            None,
            &[(AURA_ADD_FLAT_MODIFIER, mods::OP_ACTIVATION_TIME as i32, 0)],
        );
        s.family_name = 3;
        s.effects[0].item_type = 0x1;
        s.proc_charges = charges;
        if charges == 0 {
            s.attributes[0] |= sd::ATTR_PASSIVE;
        }
        hasten.push(s);
    }
    let mut percent = aura_spell(
        PERCENT,
        Some(10_000),
        &[(AURA_PERIODIC_DAMAGE_PERCENT, 0, 1000)],
    );
    percent.effects[0].target_a = 6;
    let mut healed = percent.clone();
    healed.id = PERCENT_HEALED;
    healed.effects[1] = healed.effects[0];
    healed.effects[1].aura = AURA_PERIODIC_HEAL;
    add_spells(
        &mut d,
        [
            dot(DOT, 0x1),
            dot(OTHER_MASK, 0x2),
            stacking,
            dot(PULSE, 0x1),
            dot(PRESERVED, 0x1),
            percent,
            healed,
            aura_spell(SHIELD, Some(30_000), &[(AURA_SCHOOL_ABSORB, 1, 0)]),
            aura_spell(
                IMMUNE,
                None,
                &[(AURA_STATE_IMMUNITY, AURA_PERIODIC_DAMAGE_PERCENT as i32, 0)],
            ),
        ]
        .into_iter()
        .chain(hasten),
    );
    d
}

/// `g`'s `spell`: its first effect's interval and amount, its next tick
/// and end from now, and its ticks.
fn schedule(m: &Map, g: Guid, spell: u32) -> Option<(u64, i32, u64, u64, u32, u32)> {
    let now = m.now;
    let h = auras::auras(m, g)?
        .holders
        .iter()
        .find(|h| h.spell == spell)?
        .clone();
    let x = h.effects.first()?;
    let ms = |t: Option<std::time::Instant>| {
        t.map_or(0, |t| t.saturating_duration_since(now).as_millis() as u64)
    };
    Some((
        x.period?.as_millis() as u64,
        x.amount,
        ms(x.next_tick),
        ms(h.expires),
        x.ticks,
        h.stacks,
    ))
}

fn cast(m: &mut Map, caster: Guid, spell: u32, at: Guid) {
    if let Some(c) = m.caster_mut(caster) {
        c.cooldowns.clear();
        c.gcd_until = None;
    }
    spells::cast_triggered(m, caster, spell, Some(at));
}

/// Thistle Tea's activation modifiers: two seconds off a DoT's activation
/// time halve its four-second cadence, its amount and duration as they
/// were; one of another mask keeps its four. A refresh by a caster without
/// the talent restarts at four. A modifier with a charge is spent on the
/// cast it changes, its cadence kept.
#[tokio::test]
async fn activation_modifiers_change_a_periodics_cadence() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Mage).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            put(m, ga, ga, HASTEN, &[-2000]).unwrap();
            cast(m, ga, DOT, brute);
            let hastened = schedule(m, brute, DOT);
            cast(m, ga, OTHER_MASK, brute);
            let other = schedule(m, brute, OTHER_MASK);
            auras::remove_spell(m, ga, HASTEN, None);
            m.now += Duration::from_millis(1_000);
            cast(m, ga, DOT, brute);
            let refreshed = schedule(m, brute, DOT);
            auras::remove_where(m, brute, |_| true);
            put(m, ga, ga, HASTEN_ONCE, &[-2000]).unwrap();
            // A cast of the player's own, not a triggered one, spends it.
            let begun = spells::cast(
                m,
                ga,
                DOT,
                spells::Targets {
                    unit: Some(brute),
                    ..Default::default()
                },
                None,
            );
            assert!(begun);
            (
                hastened,
                other,
                refreshed,
                schedule(m, brute, DOT),
                has(m, ga, HASTEN_ONCE),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        got.0,
        Some((2000, 10, 2000, 10_000, 0, 1)),
        "half the cadence"
    );
    assert_eq!(
        got.1,
        Some((4000, 10, 4000, 10_000, 0, 1)),
        "another mask's kept"
    );
    assert_eq!(
        got.2,
        Some((4000, 10, 4000, 10_000, 0, 1)),
        "restarted at four"
    );
    assert_eq!(got.3.map(|s| s.0), Some(2000), "the charge's cadence");
    assert!(!got.4, "its charge spent");
}

/// Thistle Tea's refreshes: a stack more keeps the pending tick and the
/// count; an immediate pulse ticks at once, then on its cadence; one
/// whose rhythm a refresh keeps keeps its pending tick.
#[tokio::test]
async fn a_refresh_keeps_the_rhythm_of_stacks_and_pulses() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Mage).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            put(m, ga, brute, STACKING, &[10]).unwrap();
            pass(m, 4_000);
            m.now += Duration::from_millis(500);
            put(m, ga, brute, STACKING, &[10]).unwrap();
            let stacked = schedule(m, brute, STACKING);
            put(m, ga, brute, PULSE, &[10]).unwrap();
            let pulse_at = schedule(m, brute, PULSE).map(|s| s.2);
            let before = health(m, brute);
            pass(m, 0);
            let pulsed = (
                before - health(m, brute),
                schedule(m, brute, PULSE).map(|s| s.2),
            );
            put(m, ga, brute, PRESERVED, &[10]).unwrap();
            pass(m, 4_000);
            m.now += Duration::from_millis(500);
            put(m, ga, brute, PRESERVED, &[10]).unwrap();
            (
                stacked,
                pulse_at,
                pulsed,
                schedule(m, brute, PRESERVED).map(|s| s.2),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        got.0,
        Some((4000, 10, 3500, 10_000, 1, 2)),
        "the tick and count kept"
    );
    assert_eq!(got.1, Some(0), "the pulse at once");
    assert_eq!(got.2, (10, Some(4000)), "then on its cadence");
    assert_eq!(got.3, Some(3500), "the pending tick kept");
}

/// Thistle Tea's `PercentDamageTest`: a tenth of the maximum health a
/// tick, through a physical ward first (100 into 150, then 50 of the
/// next, the ward gone); a percentage of nothing or less heals nothing; an
/// immunity to its kind keeps it off even cast on oneself; a killing tick
/// takes every aura and no heal in its spell undoes it.
#[tokio::test]
async fn a_percentage_of_health_a_tick_through_wards_to_death() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            put(m, brute, brute, SHIELD, &[150]).unwrap();
            put(m, ga, brute, PERCENT, &[10]).unwrap();
            set(m, brute, UNIT_FIELD_MAXHEALTH, 1_000);
            set(m, brute, UNIT_FIELD_HEALTH, 1_000);
            pass(m, 1_000);
            let first = (health(m, brute), has(m, brute, SHIELD));
            pass(m, 1_000);
            let second = (health(m, brute), has(m, brute, SHIELD));
            auras::remove_where(m, brute, |_| true);
            let mut none = Vec::new();
            for amount in [0, -10] {
                put(m, ga, brute, PERCENT, &[amount]).unwrap();
                pass(m, 1_000);
                none.push(health(m, brute));
                auras::remove_where(m, brute, |_| true);
            }
            put(m, ga, ga, IMMUNE, &[0]).unwrap();
            let immune = put(m, ga, ga, PERCENT, &[10]);
            set(m, brute, UNIT_FIELD_HEALTH, 50);
            put(m, ga, brute, PERCENT_HEALED, &[10, 200]).unwrap();
            pass(m, 1_000);
            let dead = (
                health(m, brute),
                auras::auras(m, brute).map(|a| a.holders.len()),
            );
            (first, second, none, immune, dead)
        })
        .await
        .unwrap();
    assert_eq!(got.0, (1_000, true), "the ward takes the first");
    assert_eq!(got.1, (950, false), "and half the second, and goes");
    assert_eq!(got.2, [950, 950], "nothing healed");
    assert_eq!(got.3, Err(Refused::Immune));
    assert_eq!(got.4, (0, Some(0)), "dead, its auras gone, nothing healed");
}
