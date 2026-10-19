//! Round 8, against Thistle Tea's `deep_wounds_test.exs`: what Deep
//! Wounds' bleed comes to, by which hand, and on whom (mangos-classic's
//! Deep Wounds in `HandleProcTriggerSpellAuraProc`).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{SpellDuration, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::auras::{self, *};
use crate::game::map::combat::Hand;
use crate::game::map::inventory::{SLOT_MAIN_HAND, SLOT_OFF_HAND};
use crate::game::map::proc_scripts::deep_wounds;
use crate::game::map::{Map, derive};
use crate::game::tests_p3_combat::{self as base, BRUTE, SWORD, aura_of, equip, level, setup_with};
use crate::game::tests_p3_spells_kit as kit;

const DAGGER: u32 = 63_810;
/// Deep Wounds' third rank's trigger, its bleed.
const DEEP_WOUNDS_3: u32 = 12_868;
const BLEED: u32 = 12_721;
const LASTS_12S: u32 = 993;
const DAMAGE_10: u32 = 998_201;
const DAMAGE_20_PCT: u32 = 998_202;

fn data() -> GameData {
    let mut d = base::data();
    d.items.insert(
        DAGGER,
        ItemTemplate {
            entry: DAGGER,
            class: 2,
            subclass: 15,
            name: "Dagger".into(),
            display_id: 1000,
            inventory_type: 13,
            delay: 2000,
            stackable: 1,
            max_durability: 100,
            damages: [
                (20.0, 40.0, 0),
                (0.0, 0.0, 0),
                (0.0, 0.0, 0),
                (0.0, 0.0, 0),
                (0.0, 0.0, 0),
            ],
            ..Default::default()
        },
    );
    if let Some(sword) = d.items.get_mut(&SWORD) {
        sword.damages[0] = (80.0, 120.0, 0);
    }
    d.spells.durations.insert(
        LASTS_12S,
        SpellDuration {
            base: 12_000,
            per_level: 0,
            max: 12_000,
        },
    );
    let mut bleed = SpellTemplate {
        id: BLEED,
        name: "Deep Wound".into(),
        equipped_item_class: -1,
        range_index: 1,
        duration_index: LASTS_12S,
        ..Default::default()
    };
    bleed.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_PERIODIC_DAMAGE,
        amplitude: 3_000,
        target_a: 6,
        bonus_coefficient: 0.0,
        ..Default::default()
    };
    d.spells.spells.insert(BLEED, bleed);
    d
}

/// A warrior of 60 who dual wields the sword and the dagger, and a brute.
fn armed(m: &mut Map, g: Guid) -> Guid {
    level(m, g, 60);
    m.players
        .get_mut(&g)
        .unwrap()
        .spells
        .push(derive::SPELL_DUAL_WIELD);
    equip(m, g, SWORD, SLOT_MAIN_HAND);
    equip(m, g, DAGGER, SLOT_OFF_HAND);
    derive::unit(m, g);
    base::spawn(m, g, BRUTE, 60, 3.0)
}

/// A hand's average damage, as its fields have it.
fn average(m: &Map, g: Guid, hand: Hand) -> f32 {
    let f = m.unit_fields(g).unwrap();
    let (lo, hi) = if hand == Hand::Off {
        (UNIT_FIELD_MINOFFHANDDAMAGE, UNIT_FIELD_MAXOFFHANDDAMAGE)
    } else {
        (UNIT_FIELD_MINDAMAGE, UNIT_FIELD_MAXDAMAGE)
    };
    (f.get_f32(lo) + f.get_f32(hi)) / 2.0
}

/// The third rank's tick of `avg`: three fifths over four ticks.
fn tick_of(avg: f32) -> i32 {
    (avg * 0.6 / 4.0) as i32
}

/// The bleed Deep Wounds puts on `t` with its swing timers `main` and
/// `off` milliseconds off (none, unset; negative, run out) and `hand`
/// named: its tick.
fn bleed(
    m: &mut Map,
    a: Guid,
    t: Guid,
    (main, off): (Option<i64>, Option<i64>),
    hand: Option<Hand>,
) -> Option<i32> {
    auras::remove_where(m, t, |h| h.spell == BLEED);
    let now = m.now;
    let at = |ms: Option<i64>| {
        ms.map(|ms| {
            if ms >= 0 {
                now + Duration::from_millis(ms as u64)
            } else {
                now - Duration::from_millis((-ms) as u64)
            }
        })
    };
    if let Some(c) = m.combat_mut(a) {
        c.next_swing = at(main);
        c.next_offhand = at(off);
    }
    deep_wounds(m, a, t, DEEP_WOUNDS_3, hand);
    kit::holder(m, t, BLEED).and_then(|h| h.effects.first().map(|x| x.amount))
}

/// Of an ability's crit (no hand named), the bleed is the off hand's when
/// its swing is due sooner, the main hand's when they are tied or the main
/// hand's is sooner; a timer unset or run out is due now; a broken off
/// hand counts for nothing (Thistle Tea's "uses an earlier offhand swing
/// with its normal damage penalty", "uses mainhand when timers are tied or
/// it is due first" and "compares remaining durations with unset or
/// expired timers"; mangos-classic's attack timers). A swing's crit names
/// its own hand, whatever the timers (its "honors the triggering hand after
/// its swing timer resets").
#[tokio::test]
async fn deep_wounds_bleeds_by_the_hand_that_struck_or_the_one_due_sooner() {
    let (_realm, _c, a, map) = setup_with(data(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            let t = armed(m, a);
            let main = tick_of(average(m, a, Hand::Main));
            let off = tick_of(average(m, a, Hand::Off));
            assert!(main > off && off > 0, "{main} {off}");
            let by_timers = [
                (Some(4_000), Some(3_000)),
                (Some(4_000), Some(4_000)),
                (Some(4_000), Some(5_000)),
                (Some(2_000), None),
                (None, Some(2_000)),
                (Some(-1_000), Some(-2_000)),
            ]
            .map(|timers| bleed(m, a, t, timers, None));
            let named = [
                bleed(m, a, t, (Some(2_000), Some(1_000)), Some(Hand::Main)),
                bleed(m, a, t, (Some(1_000), Some(2_000)), Some(Hand::Off)),
            ];
            // The dagger broken.
            if let Some(i) = m
                .players
                .get_mut(&a)
                .unwrap()
                .items
                .iter_mut()
                .find(|i| i.entry == DAGGER)
            {
                i.fields.set_u32(ITEM_FIELD_DURABILITY, 0);
            }
            derive::unit(m, a);
            let broken = bleed(m, a, t, (Some(4_000), Some(3_000)), None);
            (main, off, by_timers, named, broken)
        })
        .await
        .unwrap();
    let (main, off, by_timers, named, broken) = out;
    let (main, off) = (Some(main), Some(off));
    assert_eq!(
        by_timers,
        [off, main, main, off, main, main],
        "by the timers"
    );
    assert_eq!(named, [main, off], "the hand named");
    assert_eq!(broken, main, "a broken off hand");
}

/// The bleed is of the hand's damage with its flat and percentage bonuses:
/// ten more and a fifth more to physical damage raise it so (Thistle Tea's
/// "includes applicable flat and percentage weapon bonuses"); it is worked
/// out as it lands, of the damage then, and lands on none dead (its
/// "dispatches the snapshotted bleed through the source owner" and "rejects
/// missing snapshots and dead recipients").
#[tokio::test]
async fn deep_wounds_bleeds_by_the_damage_with_its_bonuses_and_not_on_the_dead() {
    let (_realm, _c, a, map) = setup_with(data(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            let t = armed(m, a);
            let before = average(m, a, Hand::Main);
            let plain = bleed(m, a, t, (None, None), Some(Hand::Main));
            aura_of(m, a, DAMAGE_10, AURA_MOD_DAMAGE_DONE, 1, 10);
            aura_of(m, a, DAMAGE_20_PCT, AURA_MOD_DAMAGE_PERCENT_DONE, 1, 20);
            derive::unit(m, a);
            let buffed = bleed(m, a, t, (None, None), Some(Hand::Main));
            let caster = kit::holder(m, t, BLEED).map(|h| h.caster);
            auras::remove_where(m, t, |h| h.spell == BLEED);
            crate::game::map::combat::deal_damage(m, a, t, 1_000_000);
            assert!(!m.unit_view(t).is_some_and(|u| u.alive), "dead");
            let dead = bleed(m, a, t, (None, None), Some(Hand::Main));
            (before, plain, buffed, caster, dead)
        })
        .await
        .unwrap();
    let (before, plain, buffed, caster, dead) = out;
    assert_eq!(plain, Some(tick_of(before)));
    assert_eq!(buffed, Some(tick_of((before + 10.0) * 1.2)), "{before}");
    assert_eq!(caster, Some(a), "the warrior's");
    assert_eq!(dead, None, "on the dead");
}
