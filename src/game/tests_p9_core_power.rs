//! Round 9, the core's stats: Thistle Tea's `target_spell_power_test.exs`,
//! `temporary_spell_power_test.exs`, `target_attack_power_test.exs`,
//! `resistance_penetration_test.exs`, `cast_speed_test.exs` and
//! `stats_test.exs`: spell power against a creature type, what a spell
//! holds from its launch, attack power against a type by the hand's own
//! speed, a crit's bonus against a type, penetration as it is now, haste
//! on a cast and a channel, and the creatures' stats.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{ATTR_EX_CHANNELED, SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, *};
use crate::game::map::combat::{self, Hand};
use crate::game::map::inventory::{SLOT_MAIN_HAND, SLOT_OFF_HAND};
use crate::game::map::spells::outcome::{self, Landing};
use crate::game::map::{Map, derive, spells};
use crate::game::tests_p3_combat::{self as base, *};

const HOLY: u32 = 1;
const FIRE: usize = 2;
/// Bit 0 of a creature type mask: beasts.
const BEASTS: i32 = 1;

const BOLT: u32 = 97_910;
const LEECH: u32 = 97_911;
const DOT: u32 = 97_912;
const LEECH_DOT: u32 = 97_913;
const HOT: u32 = 97_914;
const FIXED: u32 = 97_915;
const HEAL: u32 = 97_916;
const CAST: u32 = 97_917;
const CHANNEL: u32 = 97_918;
const DAGGER: u32 = 97_919;

fn spell(id: u32, class: u32, effect: SpellEffect) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        range_index: 1,
        equipped_item_class: -1,
        dmg_class: class,
        school: HOLY,
        duration_index: 1,
        ..Default::default()
    };
    s.effects[0] = effect;
    s
}

/// A hundred of `effect` (or aura `aura`), at a coefficient of a half.
fn hundred(effect: u32, aura: u32) -> SpellEffect {
    SpellEffect {
        effect,
        aura,
        base_points: 99,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        bonus_coefficient: 0.5,
        multiple_value: 1.0,
        amplitude: if aura != 0 { 1000 } else { 0 },
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = base::data();
    let mut fixed = spell(FIXED, 1, hundred(2, 0));
    fixed.custom_flags = crate::game::data::spells::CUSTOM_FIXED_DAMAGE;
    let mut heal = spell(HEAL, 1, hundred(10, 0));
    heal.effects[0].target_a = 1;
    // A cast of two seconds; a channel after a cast of two, six seconds
    // long, ticking every two.
    d.spells.cast_times.insert(9_791, 2_000);
    let mut cast = spell(CAST, 1, hundred(2, 0));
    cast.casting_time_index = 9_791;
    let mut channel = spell(CHANNEL, 1, hundred(6, AURA_PERIODIC_DAMAGE));
    channel.casting_time_index = 9_791;
    channel.duration_index = 2;
    channel.attributes[1] |= ATTR_EX_CHANNELED;
    channel.effects[0].amplitude = 2_000;
    for s in [
        spell(BOLT, 1, hundred(2, 0)),
        spell(LEECH, 1, hundred(9, 0)),
        spell(DOT, 1, hundred(6, AURA_PERIODIC_DAMAGE)),
        spell(LEECH_DOT, 1, hundred(6, AURA_PERIODIC_LEECH)),
        spell(HOT, 1, hundred(6, AURA_PERIODIC_HEAL)),
        fixed,
        heal,
        cast,
        channel,
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d.spells.durations.insert(
        1,
        SpellDuration {
            base: 3000,
            per_level: 0,
            max: 3000,
        },
    );
    d.spells.durations.insert(
        2,
        SpellDuration {
            base: 6000,
            per_level: 0,
            max: 6000,
        },
    );
    d.items.insert(
        DAGGER,
        crate::game::data::items::ItemTemplate {
            entry: DAGGER,
            class: 2,
            subclass: 15,
            name: "Dagger".into(),
            display_id: 1000,
            inventory_type: 13,
            delay: 1400,
            stackable: 1,
            ..Default::default()
        },
    );
    d
}

fn direct(m: &Map, g: Guid, target: Guid, spell: u32, crit: bool) -> u32 {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let l = Landing {
        crit,
        ..Default::default()
    };
    outcome::damage(m, g, target, &s, 0, 100.0, &l).amount
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn set_health(m: &mut Map, g: Guid, h: u32) {
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_HEALTH, h);
    }
}

/// The amount a periodic aura of `spell` on `target` holds.
fn holds(m: &Map, target: Guid, spell: u32) -> Option<i32> {
    auras::auras(m, target)?
        .holders
        .iter()
        .find(|h| h.spell == spell)
        .map(|h| h.effects[0].amount)
}

/// A tick of `caster`'s `spell` on `target`, now: what it took off.
fn tick(m: &mut Map, caster: Guid, target: Guid, spell: u32) -> i64 {
    let before = i64::from(health(m, target));
    let now = m.now;
    if let Some(h) =
        auras::auras_mut(m, target).and_then(|a| a.holders.iter_mut().find(|h| h.spell == spell))
    {
        h.effects[0].next_tick = Some(now);
    }
    auras::tick_due(m, target, spell, caster);
    before - i64::from(health(m, target))
}

/// Thistle Tea's "combines power before rounding and resolves each
/// recipient independently", "scales the bonus with critical hits and
/// skips fixed damage and healing" and "direct leech heals actual damage
/// after overkill": spell power and the power against the target's type
/// (aura 180) are summed, then taken by the coefficient and rounded once,
/// for each target by its own type; a crit takes the bonus too; fixed
/// damage and a heal take none; a leech heals what it took, no more.
#[tokio::test]
async fn spell_power_against_a_type_is_summed_first_and_each_target_its_own() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            aura_of(m, ga, 97_920, AURA_MOD_DAMAGE_DONE, 1 << HOLY, 1);
            aura_of(m, ga, 97_921, AURA_MOD_FLAT_SPELL_DAMAGE_VERSUS, BEASTS, 49);
            derive::unit(m, ga);
            let beast = spawn(m, ga, BEAST, 60, 3.0);
            let brute = spawn(m, ga, BRUTE, 60, 4.0);
            let summed = (
                direct(m, ga, beast, BOLT, false),
                direct(m, ga, brute, BOLT, false),
            );
            auras::remove_spell(m, ga, 97_920, None);
            auras::remove_spell(m, ga, 97_921, None);
            aura_of(
                m,
                ga,
                97_922,
                AURA_MOD_FLAT_SPELL_DAMAGE_VERSUS,
                BEASTS,
                100,
            );
            derive::unit(m, ga);
            let crit = direct(m, ga, beast, BOLT, true);
            let fixed = direct(m, ga, beast, FIXED, false);
            set_health(m, ga, 500);
            spells::cast_triggered(m, ga, HEAL, Some(ga));
            let healed = health(m, ga);
            // A leech of 150 at a beast with 120 left.
            set_health(m, beast, 120);
            set_health(m, ga, 100);
            spells::cast_triggered(m, ga, LEECH, Some(beast));
            (
                summed,
                (crit, fixed, healed),
                (health(m, beast), health(m, ga)),
            )
        })
        .await
        .unwrap();
    assert_eq!(r.0, (125, 100), "100 + 50 x 0.5; 100 + 0.5, rounded once");
    assert_eq!(r.1, (225, 100, 600), "a crit's whole; fixed; a heal");
    assert_eq!(r.2, (0, 220), "the leech heals the 120 it took");
}

/// Thistle Tea's "periodic damage snapshots each recipient and survives
/// caster removal" and "periodic leech receives the bonus but periodic
/// healing does not": a DoT holds the power against its own target's type
/// from its application, and keeps it once the power is gone; cast again,
/// it holds none; it runs its time out. A leech over time holds it and
/// heals by it; a heal over time holds none.
#[tokio::test]
async fn a_dot_holds_its_targets_type_power_from_its_launch() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            aura_of(m, ga, 97_923, AURA_MOD_FLAT_SPELL_DAMAGE_VERSUS, BEASTS, 50);
            derive::unit(m, ga);
            let beast = spawn(m, ga, BEAST, 60, 3.0);
            let brute = spawn(m, ga, BRUTE, 60, 4.0);
            for t in [beast, brute] {
                spells::cast_triggered(m, ga, DOT, Some(t));
            }
            let held = (holds(m, beast, DOT), holds(m, brute, DOT));
            spells::cast_triggered(m, ga, LEECH_DOT, Some(beast));
            spells::cast_triggered(m, ga, HOT, Some(ga));
            let others = (holds(m, beast, LEECH_DOT), holds(m, ga, HOT));
            set_health(m, ga, 300);
            let leeched = tick(m, ga, beast, LEECH_DOT);
            let healed = health(m, ga);
            auras::remove_spell(m, ga, 97_923, None);
            derive::unit(m, ga);
            let ticks = (tick(m, ga, beast, DOT), tick(m, ga, brute, DOT));
            spells::cast_triggered(m, ga, DOT, Some(beast));
            let recast = holds(m, beast, DOT);
            m.now += Duration::from_millis(3_100);
            auras::update(m);
            (
                held,
                others,
                (leeched, healed),
                ticks,
                recast,
                holds(m, beast, DOT),
            )
        })
        .await
        .unwrap();
    let (held, others, leech, ticks, recast, ended) = r;
    assert_eq!(held, (Some(125), Some(100)), "each target its own");
    assert_eq!(others, (Some(125), Some(100)), "a leech's; a heal's none");
    assert_eq!(leech, (125, 425), "the leech heals what it took");
    assert_eq!(ticks, (125, 100), "kept once the power is gone");
    assert_eq!(recast, Some(100), "cast again without it");
    assert_eq!(ended, None, "its time run out");
}

/// Thistle Tea's "removal changes new snapshots while preserving spells
/// already cast" and "periodic damage and healing retain their launch
/// bonus after the buff expires": a buff's spell and healing power gone
/// (cancelled, or run out), the fields and new spells are as before it,
/// and a DoT and a HoT cast meanwhile hold it to their end.
#[tokio::test]
async fn spells_cast_under_a_buff_keep_it_once_it_is_gone() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            // Gear's ten holy and twenty healing; a buff's sixty and 120.
            aura_of(m, ga, 97_930, AURA_MOD_DAMAGE_DONE, 1 << HOLY, 10);
            // (Healing of every school, -1, as the data's gear has it.)
            aura_of(m, ga, 97_931, AURA_MOD_HEALING_DONE, -1, 20);
            aura_of(m, ga, 97_932, AURA_MOD_DAMAGE_DONE, 1 << HOLY, 60);
            aura_of(m, ga, 97_933, AURA_MOD_HEALING_DONE, -1, 120);
            derive::unit(m, ga);
            let beast = spawn(m, ga, BEAST, 60, 3.0);
            spells::cast_triggered(m, ga, DOT, Some(beast));
            spells::cast_triggered(m, ga, HOT, Some(ga));
            let held = (holds(m, beast, DOT), holds(m, ga, HOT));
            // One cancelled, the other run out.
            auras::remove_spell(m, ga, 97_932, None);
            let now = m.now;
            if let Some(h) = auras::auras_mut(m, ga)
                .and_then(|a| a.holders.iter_mut().find(|h| h.spell == 97_933))
            {
                h.expires = Some(now);
            }
            auras::update(m);
            derive::unit(m, ga);
            let p = &m.players[&ga];
            let fields = (
                p.fields
                    .get_u32(PLAYER_FIELD_MOD_DAMAGE_DONE_POS + HOLY as u16),
                p.derived.spell_power[HOLY as usize],
                p.derived.healing_power,
            );
            let kept = (holds(m, beast, DOT), holds(m, ga, HOT));
            let dot = tick(m, ga, beast, DOT);
            set_health(m, ga, 500);
            tick(m, ga, ga, HOT);
            let hot = health(m, ga);
            let bolt = direct(m, ga, beast, BOLT, false);
            (held, fields, kept, (dot, hot), bolt)
        })
        .await
        .unwrap();
    let (held, fields, kept, ticked, bolt) = r;
    assert_eq!(
        held,
        (Some(135), Some(170)),
        "100 + 70 x 0.5; 100 + 140 x 0.5"
    );
    assert_eq!(fields, (10, 10, 20), "as before the buff");
    assert_eq!(kept, held, "held from their launch");
    assert_eq!(ticked, (135, 670));
    assert_eq!(bolt, 105, "a new spell without it");
}

/// Thistle Tea's "equipment resync preserves temporary bonuses without
/// accumulating them", "spirit-derived spell power follows stat changes
/// and restoration" and the summed half of "projects separate signed
/// penalties and multiplicative school bonuses privately": the stats
/// worked out again keep a buff's spell power once; spirit's share
/// (aura 174) follows spirit up and down, in the field the client shows
/// too; penalties take off the bonuses, and percentages multiply, by
/// their stacks.
#[tokio::test]
async fn spell_power_is_worked_out_again_whole_with_spirits_share() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            let pos = |m: &Map| {
                let p = &m.players[&ga];
                (
                    p.fields
                        .get_u32(PLAYER_FIELD_MOD_DAMAGE_DONE_POS + HOLY as u16),
                    p.derived.spell_power[HOLY as usize],
                )
            };
            aura_of(m, ga, 97_940, AURA_MOD_DAMAGE_DONE, 1 << HOLY, 10);
            aura_of(m, ga, 97_941, AURA_MOD_DAMAGE_DONE, 1 << HOLY, 50);
            for _ in 0..3 {
                derive::unit(m, ga);
                crate::game::map::inventory::sync_gear(m, ga);
            }
            let resynced = pos(m);
            auras::remove_spell(m, ga, 97_940, None);
            auras::remove_spell(m, ga, 97_941, None);
            // Half of spirit as holy power.
            let spirit = m.players[&ga].fields.get_u32(UNIT_FIELD_STAT0 + 4);
            aura_of(
                m,
                ga,
                97_942,
                AURA_MOD_SPELL_DAMAGE_OF_STAT_PERCENT,
                1 << HOLY,
                50,
            );
            derive::unit(m, ga);
            let of_spirit = pos(m);
            aura_of(m, ga, 97_943, AURA_MOD_STAT, 4, 50);
            derive::unit(m, ga);
            let more = pos(m);
            auras::remove_spell(m, ga, 97_943, None);
            derive::unit(m, ga);
            let back = pos(m);
            auras::remove_spell(m, ga, 97_942, None);
            // Fifty and a penalty of seven; twenty percent and ten less.
            aura_of(m, ga, 97_944, AURA_MOD_DAMAGE_DONE, 1 << HOLY, 50);
            aura_of(m, ga, 97_945, AURA_MOD_DAMAGE_DONE, 1 << HOLY, -7);
            aura_of(m, ga, 97_946, AURA_MOD_DAMAGE_PERCENT_DONE, 1 << HOLY, 20);
            aura_of(m, ga, 97_947, AURA_MOD_DAMAGE_PERCENT_DONE, 1 << HOLY, -10);
            derive::unit(m, ga);
            let pct = |m: &Map, school: u16| {
                m.players[&ga]
                    .fields
                    .get_f32(PLAYER_FIELD_MOD_DAMAGE_DONE_PCT + school)
            };
            let signed = (pos(m), pct(m, HOLY as u16), pct(m, FIRE as u16));
            for id in [97_946, 97_947] {
                if let Some(h) = auras::auras_mut(m, ga)
                    .and_then(|a| a.holders.iter_mut().find(|h| h.spell == id))
                {
                    h.stacks = 2;
                }
            }
            derive::unit(m, ga);
            (
                resynced,
                (spirit, of_spirit, more, back),
                signed,
                pct(m, HOLY as u16),
            )
        })
        .await
        .unwrap();
    let (resynced, (spirit, of_spirit, more, back), signed, stacked) = r;
    assert_eq!(resynced, (60, 60), "once, however often worked out");
    let share = spirit / 2;
    assert_eq!(
        of_spirit,
        (share, share as i32),
        "spirit's share, shown too"
    );
    assert_eq!(more, (share + 25, share as i32 + 25), "fifty more spirit");
    assert_eq!(back, of_spirit, "and back");
    assert_eq!((signed.0).1, 43, "fifty less seven");
    assert_eq!((signed.0).0, 50, "the bonuses alone shown as such");
    assert!((signed.1 - 1.08).abs() < 1e-4, "1.2 x 0.9: {}", signed.1);
    assert!((signed.2 - 1.0).abs() < 1e-6, "another school's none");
    assert!((stacked - 1.12).abs() < 1e-4, "1.4 x 0.8: {stacked}");
}

/// Thistle Tea's "uses actual mob types for magic damage and critical
/// multipliers": more damage against a type (aura 168) for a spell
/// against it alone, and its crit's bonus raised by Monster Slaying's
/// kind (aura 169), as a white blow's is, where its chance isn't.
#[tokio::test]
async fn a_spells_crit_against_a_type_takes_more_and_is_no_likelier() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            aura_of(m, ga, 97_950, AURA_MOD_DAMAGE_DONE_VERSUS, BEASTS, 10);
            aura_of(m, ga, 97_951, AURA_MOD_CRIT_PERCENT_VERSUS, BEASTS, 100);
            derive::unit(m, ga);
            let beast = spawn(m, ga, BEAST, 60, 3.0);
            let brute = spawn(m, ga, BRUTE, 60, 4.0);
            let damage = [
                direct(m, ga, beast, BOLT, false),
                direct(m, ga, beast, BOLT, true),
                direct(m, ga, brute, BOLT, false),
                direct(m, ga, brute, BOLT, true),
            ];
            // A roll of 99.99: no crit at a warrior's spell crit, whatever
            // the type.
            let bolt = m.realm.data.spells.get(BOLT).cloned().unwrap();
            m.spells.queued_rolls.lock().unwrap().push_back(9_999);
            let crit = outcome::roll_crit(m, ga, beast, &bolt, &Default::default());
            (damage, crit)
        })
        .await
        .unwrap();
    assert_eq!(r.0, [110, 220, 100, 150], "110 + 110 x 0.5 x 2; 100 + 50");
    assert!(!r.1, "its chance no more against the type");
}

/// Thistle Tea's "uses offhand speed, penalty, and damage modifiers
/// without haste": attack power against a type goes by the hand's weapon's
/// speed, not its hasted swing, under the hand's percentages and the off
/// hand's half and its bonus.
#[tokio::test]
async fn attack_power_against_a_type_goes_by_the_weapons_own_speed() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            m.players
                .get_mut(&ga)
                .unwrap()
                .spells
                .push(derive::SPELL_DUAL_WIELD);
            equip(m, ga, SWORD, SLOT_MAIN_HAND);
            equip(m, ga, DAGGER, SLOT_OFF_HAND);
            let beast = spawn(m, ga, BEAST, 60, 3.0);
            aura_of(
                m,
                ga,
                97_960,
                AURA_MOD_MELEE_ATTACK_POWER_VERSUS,
                BEASTS,
                140,
            );
            aura_of(m, ga, 97_961, AURA_MOD_OFFHAND_DAMAGE_PCT, 0, 50);
            aura_of(m, ga, 97_962, AURA_MOD_DAMAGE_PERCENT_DONE, 1, 100);
            // The weapons' damage a flat hundred, swung at `speed`.
            let blow = |m: &mut Map, hand: Hand, speed: u32| {
                let f = m.unit_fields_mut(ga).unwrap();
                for (lo, hi) in [
                    (UNIT_FIELD_MINDAMAGE, UNIT_FIELD_MAXDAMAGE),
                    (UNIT_FIELD_MINOFFHANDDAMAGE, UNIT_FIELD_MAXOFFHANDDAMAGE),
                ] {
                    f.set_f32(lo, 100.0);
                    f.set_f32(hi, 100.0);
                }
                f.set_u32(UNIT_FIELD_BASEATTACKTIME, speed);
                f.set_u32(UNIT_FIELD_BASEATTACKTIME + 1, speed);
                combat::weapon_damage(m, ga, beast, hand, &mut rand::rng()).0
            };
            let plain = (blow(m, Hand::Main, 2000), blow(m, Hand::Off, 1400));
            aura_of(m, ga, 97_963, AURA_MOD_HASTE, 0, 100);
            derive::unit(m, ga);
            let hasted = (blow(m, Hand::Main, 1000), blow(m, Hand::Off, 700));
            (plain, hasted)
        })
        .await
        .unwrap();
    // 140 / 14 x 2 s x 2.0; x 1.4 s x 2.0 x 0.5 x 1.5.
    assert_eq!(r.0, (140, 121));
    assert_eq!(r.1, r.0, "haste takes nothing off it");
}

/// Thistle Tea's "uses the target's current resistance at each tick" and
/// "cancellation, expiry, and death clear future attack and spell
/// snapshots": a caster's penetration is taken off the target's
/// resistance as both are when a spell or a blow lands, so a resistance
/// raised counts at once, and penetration gone (cancelled, run out, or
/// its caster dead) counts no more.
#[tokio::test]
async fn penetration_and_resistance_count_as_they_are_when_it_lands() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            let beast = spawn(m, ga, BEAST, 60, 3.0);
            let fire = UNIT_FIELD_RESISTANCES + FIRE as u16;
            m.unit_fields_mut(beast).unwrap().set_u32(fire, 600);
            m.unit_fields_mut(beast)
                .unwrap()
                .set_u32(UNIT_FIELD_RESISTANCES, 2000);
            let pen = |m: &mut Map, id: u32| {
                aura_of(m, ga, id, AURA_MOD_TARGET_RESISTANCE, 1 << FIRE, -300);
                aura_of(m, ga, id + 1, AURA_MOD_TARGET_RESISTANCE, 1, -1000);
            };
            let against = |m: &Map| {
                (
                    outcome::resistance(m, ga, beast, FIRE),
                    combat::armor_mitigate(m, ga, beast, 1000),
                )
            };
            let bare = against(m);
            pen(m, 97_970);
            let pierced = against(m);
            m.unit_fields_mut(beast).unwrap().set_u32(fire, 900);
            let raised = outcome::resistance(m, ga, beast, FIRE);
            m.unit_fields_mut(beast).unwrap().set_u32(fire, 600);
            // Cancelled.
            auras::remove_spell(m, ga, 97_970, None);
            auras::remove_spell(m, ga, 97_971, None);
            let cancelled = against(m);
            // Run out.
            pen(m, 97_972);
            let now = m.now;
            if let Some(a) = auras::auras_mut(m, ga) {
                for h in a.holders.iter_mut().filter(|h| h.spell >= 97_972) {
                    h.expires = Some(now);
                }
            }
            auras::update(m);
            let expired = against(m);
            // Dead.
            pen(m, 97_974);
            let again = against(m);
            let h = health(m, ga);
            combat::deal_damage(m, beast, ga, h);
            (bare, pierced, raised, cancelled, expired, again, against(m))
        })
        .await
        .unwrap();
    let (bare, pierced, raised, cancelled, expired, again, dead) = r;
    assert_eq!(bare.0, 600);
    assert!(pierced.1 > bare.1, "armor pierced: {pierced:?} {bare:?}");
    assert_eq!(pierced.0, 300);
    assert_eq!(raised, 600, "the target's resistance as it is now");
    assert_eq!(cancelled, bare, "cancelled");
    assert_eq!(expired, bare, "run out");
    assert_eq!(again, pierced);
    assert_eq!(dead, bare, "its caster dead");
}

/// Thistle Tea's "haste shifts the channel start without shortening its
/// ticks or duration" and "expiry restores the projection and subsequent
/// casts without changing an active cast": haste shortens a channel's cast
/// alone, not the channel or its ticks; a slow that runs out mid-cast
/// leaves the cast its length, and the next is at the speed now.
#[tokio::test]
async fn haste_shortens_a_cast_and_not_the_channel_after_it() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            let p = m.players.get_mut(&ga).unwrap();
            p.spells.extend([CAST, CHANNEL]);
            for i in 0..5 {
                p.fields.set_u32(UNIT_FIELD_MAXPOWER1 + i, 100_000);
                p.fields.set_u32(UNIT_FIELD_POWER1 + i, 100_000);
            }
            let beast = spawn(m, ga, BEAST, 60, 3.0);
            let at = spells::Targets {
                unit: Some(beast),
                ..Default::default()
            };
            let fresh = |m: &mut Map| {
                if let Some(c) = m.caster_mut(ga) {
                    c.casting = None;
                    c.gcd_until = None;
                    c.cooldowns.clear();
                }
            };
            let ends = |m: &Map| {
                m.caster(ga).and_then(|c| c.casting.as_ref()).map(|c| {
                    (
                        c.ends.saturating_duration_since(m.now),
                        c.channel_until.is_some(),
                        c.duration,
                    )
                })
            };
            // Twice as fast.
            aura_of(m, ga, 97_980, AURA_MOD_CASTING_SPEED, 0, 100);
            derive::unit(m, ga);
            assert!(spells::cast(m, ga, CHANNEL, at, None), "the channel's cast");
            let cast = ends(m);
            m.now += Duration::from_millis(1_000);
            spells::update(m);
            let channel = ends(m);
            let now = m.now;
            let ticks = auras::auras(m, beast)
                .and_then(|a| a.holders.iter().find(|h| h.spell == CHANNEL).cloned())
                .map(|h| (h.effects[0].period, h.effects[0].next_tick.map(|t| t - now)));
            auras::remove_spell(m, ga, 97_980, None);
            fresh(m);
            // Half as fast, running out a second into a cast.
            aura_of(m, ga, 97_981, AURA_MOD_CASTING_SPEED, 0, -50);
            derive::unit(m, ga);
            assert!(spells::cast(m, ga, CAST, at, None), "the slowed cast");
            let slowed = ends(m);
            m.now += Duration::from_millis(1_000);
            let now = m.now;
            if let Some(h) = auras::auras_mut(m, ga)
                .and_then(|a| a.holders.iter_mut().find(|h| h.spell == 97_981))
            {
                h.expires = Some(now);
            }
            auras::update(m);
            let speed = m.players[&ga].fields.get_f32(UNIT_MOD_CAST_SPEED);
            let left = ends(m);
            m.now += Duration::from_millis(2_000);
            spells::update(m);
            let done = ends(m);
            fresh(m);
            assert!(spells::cast(m, ga, CAST, at, None), "the next cast");
            (cast, channel, ticks, (slowed, speed, left, done), ends(m))
        })
        .await
        .unwrap();
    let (cast, channel, ticks, (slowed, speed, left, done), next) = r;
    let s = Duration::from_millis;
    assert_eq!(
        cast.map(|c| (c.0, c.1)),
        Some((s(1_000), false)),
        "a cast of one second"
    );
    assert_eq!(channel, Some((s(6_000), true, 6_000)), "a channel of six");
    assert_eq!(
        ticks,
        Some((Some(s(2_000)), Some(s(2_000)))),
        "ticks of two"
    );
    assert_eq!(slowed.map(|c| c.0), Some(s(3_000)));
    assert!((speed - 1.0).abs() < 1e-6, "the speed back: {speed}");
    assert_eq!(left.map(|c| c.0), Some(s(2_000)), "the cast its length");
    assert_eq!(done, None, "done at three seconds");
    assert_eq!(
        next.map(|c| c.0),
        Some(s(2_000)),
        "the next at the speed now"
    );
}

/// Thistle Tea's "does not invent base stats for creatures": a creature's
/// stats are its own; a percentage of every stat gives it none it hasn't,
/// nor health. (Its own strength, its base's, the percentages double
/// twice.)
#[tokio::test]
async fn a_creature_is_given_no_stats_it_has_not() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let c = spawn(m, ga, BRUTE, 20, 3.0);
            m.creature_mut(c).unwrap().base.stats[0] = 50;
            derive::unit(m, c);
            let before = m.unit_fields(c).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
            aura_of(m, c, 97_990, AURA_MOD_PERCENT_STAT, -1, 100);
            aura_of(m, c, 97_991, AURA_MOD_TOTAL_STAT_PERCENTAGE, -1, 100);
            derive::unit(m, c);
            let f = m.unit_fields(c).unwrap();
            (
                f.get_u32(UNIT_FIELD_STAT0),
                f.get_u32(UNIT_FIELD_STAT0 + 2),
                f.get_u32(UNIT_FIELD_MAXHEALTH),
                before,
            )
        })
        .await
        .unwrap();
    assert_eq!((r.0, r.1), (200, 0), "its own strength by both, no stamina");
    assert_eq!(r.2, r.3, "nor health");
}
