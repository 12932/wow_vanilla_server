//! Phase 5's auras on the real data, Thistle Tea's `DepletingAuraTest`
//! and its DBC test ported: Restless Strength, Brittle Armor and Mercurial
//! Shield start full, lose a stack for what lands, and go with the aura
//! that gave them.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, AURA_MOD_SKILL};
use crate::game::map::combat::Outcome;
use crate::game::map::procs::{self, *};
use crate::game::map::{Map, spells};
use crate::game::tests_p3_combat::forced_swing;

const RESTLESS_STRENGTH: u32 = 24_661;
const RESTLESS_STRENGTH_BONUS: u32 = 24_662;
const BRITTLE_ARMOR: u32 = 24_574;
const BRITTLE_ARMOR_BONUS: u32 = 24_575;
const MERCURIAL_SHIELD: u32 = 26_463;
const MERCURIAL_SHIELD_BONUS: u32 = 26_464;
const AUTO_SHOT: u32 = 75;
const ARCANE_SHOT: u32 = 3044;
const FIREBALL: u32 = 133;
const CORRUPTION: u32 = 172;

fn stacks(m: &Map, g: Guid, spell: u32) -> Option<u32> {
    holder(m, g, spell).map(|h| h.stacks)
}

/// The count the client shows for `g`'s `spell`: its slot's byte of
/// `UNIT_FIELD_AURAAPPLICATIONS`.
fn shown(m: &Map, g: Guid, spell: u32) -> Option<u32> {
    let slot = holder(m, g, spell)?.slot?;
    let word = m
        .unit_fields(g)?
        .get_u32(UNIT_FIELD_AURAAPPLICATIONS + (slot / 4) as u16);
    Some(word >> ((slot % 4) * 8) & 0xFF)
}

fn field(m: &Map, g: Guid, index: u16) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(index))
}

fn float(m: &Map, g: Guid, index: u16) -> f32 {
    m.unit_fields(g).map_or(0.0, |f| f.get_f32(index))
}

/// `who` does (or takes, `taken`) what `flags` say with `spell`, to `ex`.
fn event(actor: Guid, target: Guid, flags: (u32, u32), ex: u32, spell: Option<u32>) -> Event {
    Event {
        actor,
        target,
        actor_flags: flags.0,
        target_flags: flags.1,
        ex,
        spell,
        damage: if ex & (EX_NORMAL_HIT | EX_CRITICAL_HIT) != 0 {
            10
        } else {
            0
        },
        school: 1,
        ..Default::default()
    }
}

/// Thistle Tea's Restless Strength: twenty stacks shown as such, nothing
/// on empty hands; a stack spent for
/// each melee blow or ranged attack that lands (a crit, a glancing, a
/// crushing blow or a block too, and Auto Shot and Arcane Shot); none for
/// a miss, a dodge or parry, magic or a tick. Spent out, the bonus is gone
/// and the trinket's aura stays; on again (once it is gone), twenty.
#[tokio::test]
#[ignore = "extended"]
async fn restless_strength_starts_full_and_spends_a_stack_a_blow_that_lands() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, w, map) = player(&realm, "P5D1", "Restlessa", Race::Human, Class::Warrior).await;
    let got = map
        .query(move |m| {
            ready(m, w, 60, &[]);
            let kobold = dummy(m, w, 60, 2.0);
            let before = (
                float(m, w, UNIT_FIELD_MINDAMAGE),
                float(m, w, UNIT_FIELD_MINOFFHANDDAMAGE),
                float(m, w, UNIT_FIELD_MINRANGEDDAMAGE),
            );
            spells::cast_triggered(m, w, RESTLESS_STRENGTH, Some(w));
            let full = (
                has(m, w, RESTLESS_STRENGTH),
                stacks(m, w, RESTLESS_STRENGTH_BONUS),
                shown(m, w, RESTLESS_STRENGTH_BONUS),
            );
            let after = (
                float(m, w, UNIT_FIELD_MINDAMAGE),
                float(m, w, UNIT_FIELD_MINOFFHANDDAMAGE),
                float(m, w, UNIT_FIELD_MINRANGEDDAMAGE),
            );
            let mut spent = Vec::new();
            for outcome in [
                Outcome::Hit,
                Outcome::Crit,
                Outcome::Glancing,
                Outcome::Crushing,
                Outcome::Block,
            ] {
                let was = stacks(m, w, RESTLESS_STRENGTH_BONUS);
                forced_swing(m, w, kobold, outcome);
                spent.push((outcome, was.zip(stacks(m, w, RESTLESS_STRENGTH_BONUS))));
            }
            for (flags, spell) in [
                (PROC_DONE_RANGED, Some(AUTO_SHOT)),
                (PROC_DONE_RANGED_SPELL, Some(ARCANE_SHOT)),
            ] {
                let was = stacks(m, w, RESTLESS_STRENGTH_BONUS);
                procs::trigger(m, event(w, kobold, (flags, 0), EX_NORMAL_HIT, spell));
                spent.push((Outcome::Hit, was.zip(stacks(m, w, RESTLESS_STRENGTH_BONUS))));
            }
            let mut kept = Vec::new();
            for outcome in [Outcome::Miss, Outcome::Dodge, Outcome::Parry] {
                forced_swing(m, w, kobold, outcome);
                kept.push(stacks(m, w, RESTLESS_STRENGTH_BONUS));
            }
            for (flags, ex) in [
                (PROC_DONE_NEGATIVE_SPELL, EX_NORMAL_HIT),
                (PROC_DONE_PERIODIC, EX_NORMAL_HIT),
                (PROC_DONE_POSITIVE_SPELL, EX_NORMAL_HIT),
            ] {
                procs::trigger(m, event(w, kobold, (flags, 0), ex, Some(FIREBALL)));
                kept.push(stacks(m, w, RESTLESS_STRENGTH_BONUS));
            }
            let left = stacks(m, w, RESTLESS_STRENGTH_BONUS).unwrap_or(0);
            for _ in 0..left {
                forced_swing(m, w, kobold, Outcome::Hit);
            }
            let out = (
                has(m, w, RESTLESS_STRENGTH_BONUS),
                has(m, w, RESTLESS_STRENGTH),
                float(m, w, UNIT_FIELD_MINDAMAGE),
            );
            forced_swing(m, w, kobold, Outcome::Hit);
            // The trinket's two minutes are longer than its twenty
            // seconds: it is on again only once its aura is gone.
            auras::remove_spell(m, w, RESTLESS_STRENGTH, None);
            fresh(m, w);
            spells::cast_triggered(m, w, RESTLESS_STRENGTH, Some(w));
            let again = stacks(m, w, RESTLESS_STRENGTH_BONUS);
            (before, full, after, spent, kept, out, again)
        })
        .await
        .unwrap();
    let (before, full, after, spent, kept, out, again) = got;
    assert_eq!(full, (true, Some(20), Some(19)), "twenty, shown as such");
    // The bonus goes on each blow, not on the fields (Thistle Tea and
    // mangos put it there): the empty hands' fields stay as they were.
    assert_eq!(
        (after.1, after.2),
        (before.1, before.2),
        "not on empty hands"
    );
    let mut was = 20;
    for (outcome, s) in spent {
        assert_eq!(s, Some((was, was - 1)), "{outcome:?} spends one");
        was -= 1;
    }
    assert!(kept.iter().all(|s| *s == Some(was)), "{kept:?}");
    assert_eq!(out, (false, true, before.0), "spent out, the bonus gone");
    assert_eq!(again, Some(20));
}

/// Thistle Tea's Brittle Armor and Mercurial Shield: ten stacks, 200 armor
/// and three defense each, ten of each magic school's resistance each; a
/// blow that lands takes one off the armor, without moving its end, and a
/// harmful spell one off the shield; avoided blows, a spell resisted, a
/// tick and a blow on the shield spend none. Spent out, the bonus goes and
/// the trinket's aura stays.
#[tokio::test]
#[ignore = "extended"]
async fn brittle_armor_and_mercurial_shield_lose_a_stack_for_what_lands() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, w, map) = player(&realm, "P5D2", "Brittleb", Race::Human, Class::Warrior).await;
    let (_d, mage, _) = player(&realm, "P5D3", "Mercuryb", Race::Human, Class::Mage).await;
    let got = map
        .query(move |m| {
            ready(m, w, 60, &[]);
            let armor = field(m, w, UNIT_FIELD_RESISTANCES);
            let fire = field(m, w, UNIT_FIELD_RESISTANCES + 2);
            spells::cast_triggered(m, w, BRITTLE_ARMOR, Some(w));
            spells::cast_triggered(m, w, MERCURIAL_SHIELD, Some(w));
            let bonus = (
                field(m, w, UNIT_FIELD_RESISTANCES) - armor,
                auras::auras(m, w).map_or(0, |a| a.sum_misc(AURA_MOD_SKILL, 95)),
                (1..=6)
                    .map(|i| field(m, w, UNIT_FIELD_RESISTANCES + i))
                    .collect::<Vec<_>>(),
            );
            let full = (
                stacks(m, w, BRITTLE_ARMOR_BONUS),
                stacks(m, w, MERCURIAL_SHIELD_BONUS),
            );
            let ends = holder(m, w, BRITTLE_ARMOR_BONUS).and_then(|h| h.expires);
            let blow = |ex| {
                event(
                    mage,
                    w,
                    (PROC_DONE_MELEE, PROC_TAKEN_MELEE | PROC_TAKEN_DAMAGE),
                    ex,
                    None,
                )
            };
            let spell = |ex, taken, s| event(mage, w, (0, taken), ex, Some(s));
            let mut kept = Vec::new();
            for ex in [EX_MISS, EX_DODGE, EX_PARRY, EX_BLOCK] {
                procs::trigger(m, blow(ex));
                kept.push(stacks(m, w, BRITTLE_ARMOR_BONUS));
            }
            for e in [
                spell(EX_RESIST, PROC_TAKEN_NEGATIVE_SPELL, FIREBALL),
                spell(
                    EX_NORMAL_HIT,
                    PROC_TAKEN_PERIODIC | PROC_TAKEN_DAMAGE,
                    CORRUPTION,
                ),
            ] {
                procs::trigger(m, e);
                kept.push(stacks(m, w, MERCURIAL_SHIELD_BONUS));
            }
            m.now += Duration::from_secs(1);
            let mut armor_left = Vec::new();
            for _ in 0..9 {
                procs::trigger(m, blow(EX_NORMAL_HIT));
                armor_left.push((
                    stacks(m, w, BRITTLE_ARMOR_BONUS),
                    field(m, w, UNIT_FIELD_RESISTANCES) - armor,
                ));
            }
            let same_end = holder(m, w, BRITTLE_ARMOR_BONUS).and_then(|h| h.expires) == ends;
            let shield_after_blows = stacks(m, w, MERCURIAL_SHIELD_BONUS);
            procs::trigger(m, blow(EX_NORMAL_HIT));
            let armor_out = (
                has(m, w, BRITTLE_ARMOR_BONUS),
                has(m, w, BRITTLE_ARMOR),
                field(m, w, UNIT_FIELD_RESISTANCES) - armor,
            );
            let mut shield_left = Vec::new();
            for _ in 0..10 {
                procs::trigger(
                    m,
                    spell(
                        EX_NORMAL_HIT,
                        PROC_TAKEN_NEGATIVE_SPELL | PROC_TAKEN_DAMAGE,
                        FIREBALL,
                    ),
                );
                shield_left.push(field(m, w, UNIT_FIELD_RESISTANCES + 2) - fire);
            }
            let shield_out = (
                has(m, w, MERCURIAL_SHIELD_BONUS),
                has(m, w, MERCURIAL_SHIELD),
            );
            (
                bonus,
                full,
                kept,
                armor_left,
                same_end,
                shield_after_blows,
                armor_out,
                shield_left,
                shield_out,
            )
        })
        .await
        .unwrap();
    let (
        bonus,
        full,
        kept,
        armor_left,
        same_end,
        shield_after_blows,
        armor_out,
        shield_left,
        shield_out,
    ) = got;
    assert_eq!(
        bonus,
        (2_000, 30, vec![0, 100, 100, 100, 100, 100]),
        "holy none"
    );
    assert_eq!(full, (Some(10), Some(10)));
    assert_eq!(kept, vec![Some(10); 6], "avoided, resisted, ticked: kept");
    for (i, (s, a)) in armor_left.into_iter().enumerate() {
        assert_eq!((s, a), (Some(9 - i as u32), 200 * (9 - i as u32)));
    }
    assert!(same_end, "the end doesn't move");
    assert_eq!(shield_after_blows, Some(10), "blows don't touch the shield");
    assert_eq!(armor_out, (false, true, 0));
    assert_eq!(
        shield_left,
        (0..10).rev().map(|n| n * 10).collect::<Vec<_>>()
    );
    assert_eq!(shield_out, (false, true));
}

/// Thistle Tea's cleanup: each trinket's bonus goes when its aura is taken
/// off, runs out, or its wearer dies; a bonus cast with no trinket's aura
/// doesn't go on.
#[tokio::test]
#[ignore = "extended"]
async fn a_trinkets_bonus_goes_with_its_aura_and_never_comes_alone() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, w, map) = player(&realm, "P5D4", "Cleanc", Race::Human, Class::Warrior).await;
    let pairs = [
        (RESTLESS_STRENGTH, RESTLESS_STRENGTH_BONUS),
        (BRITTLE_ARMOR, BRITTLE_ARMOR_BONUS),
        (MERCURIAL_SHIELD, MERCURIAL_SHIELD_BONUS),
    ];
    let got = map
        .query(move |m| {
            ready(m, w, 60, &[]);
            let armor = field(m, w, UNIT_FIELD_RESISTANCES);
            let mut out = Vec::new();
            for (parent, bonus) in pairs {
                // Alone.
                spells::cast_triggered(m, w, bonus, Some(w));
                let alone = has(m, w, bonus);
                // Taken off.
                spells::cast_triggered(m, w, parent, Some(w));
                auras::remove_stack(m, w, bonus);
                auras::remove_spell(m, w, parent, None);
                let removed = has(m, w, bonus);
                // Run out.
                fresh(m, w);
                spells::cast_triggered(m, w, parent, Some(w));
                let lasts = holder(m, w, parent).and_then(|h| h.duration).unwrap_or(0);
                pass(m, u64::from(lasts));
                let expired = has(m, w, bonus) || has(m, w, parent);
                out.push((parent, alone, removed, expired));
            }
            let back = field(m, w, UNIT_FIELD_RESISTANCES) == armor;
            // Dead.
            for (parent, _) in pairs {
                fresh(m, w);
                spells::cast_triggered(m, w, parent, Some(w));
            }
            let kobold = dummy(m, w, 60, 2.0);
            crate::game::map::combat::deal_damage(m, kobold, w, 10_000_000);
            let dead = pairs.iter().any(|(p, b)| has(m, w, *p) || has(m, w, *b));
            (out, back, dead)
        })
        .await
        .unwrap();
    let (out, back, dead) = got;
    for (parent, alone, removed, expired) in out {
        assert!(!alone, "{parent}'s bonus alone");
        assert!(!removed, "{parent}'s bonus outlived it");
        assert!(!expired, "{parent} ran out with its bonus");
    }
    assert!(back, "the armor back");
    assert!(!dead, "death takes them all");
}

const HOLY_POWER: u32 = 28_789;
const TOTEMIC_POWER: u32 = 28_823;
const HOLY_LIGHT: u32 = 635;
const LESSER_HEAL: u32 = 2050;
const HOLY_SHOCK: u32 = 20_473;

/// `g`'s class, as its fields say it.
fn set_class(m: &mut Map, g: Guid, class: u8) {
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_byte(UNIT_FIELD_BYTES_0, 1, class);
    }
}

/// Thistle Tea's Holy Power and Totemic Power: the buff for the healed
/// one's class, whatever their power (a warrior's armor, a mana user's
/// mana, a fighter's attack power, a caster's spell power); none for a
/// class there isn't, nor the dead.
#[tokio::test]
#[ignore = "extended"]
async fn holy_and_totemic_power_give_the_healed_ones_classs_buff() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, healer, map) = player(&realm, "P5D5", "Holypowa", Race::Human, Class::Paladin).await;
    let (_d, other, _) = player(&realm, "P5D6", "Healeda", Race::Human, Class::Warrior).await;
    let got = map
        .query(move |m| {
            ready(m, healer, 60, &[]);
            ready(m, other, 60, &[]);
            let mut out = Vec::new();
            for (id, heal) in [(HOLY_POWER, HOLY_LIGHT), (TOTEMIC_POWER, 331)] {
                apply(m, healer, healer, id);
                let h = holder(m, healer, id).unwrap();
                for class in [1u8, 2, 3, 4, 5, 7, 8, 9, 11, 0, 6] {
                    for power in [0u8, 1, 3] {
                        set_class(m, other, class);
                        if let Some(f) = m.unit_fields_mut(other) {
                            f.set_byte(UNIT_FIELD_BYTES_0, 3, power);
                        }
                        let before: Vec<u32> = auras::auras(m, other)
                            .map(|a| a.holders.iter().map(|h| h.spell).collect())
                            .unwrap_or_default();
                        spells::class_scripts::proc(m, healer, other, &h, Some(heal), true);
                        let given: Vec<u32> = auras::auras(m, other)
                            .map(|a| {
                                a.holders
                                    .iter()
                                    .map(|h| h.spell)
                                    .filter(|s| !before.contains(s))
                                    .collect()
                            })
                            .unwrap_or_default();
                        for s in &given {
                            auras::remove_spell(m, other, *s, None);
                        }
                        out.push((id, class, given));
                    }
                }
                auras::remove_spell(m, healer, id, None);
            }
            // The dead get none.
            set_class(m, other, 1);
            apply(m, healer, healer, HOLY_POWER);
            let h = holder(m, healer, HOLY_POWER).unwrap();
            let kobold = dummy(m, healer, 60, 2.0);
            crate::game::map::combat::deal_damage(m, kobold, other, 10_000_000);
            spells::class_scripts::proc(m, healer, other, &h, Some(HOLY_LIGHT), true);
            let dead = auras::auras(m, other).is_some_and(|a| a.has_spell(28_790));
            (out, dead)
        })
        .await
        .unwrap();
    let (out, dead) = got;
    let holy = [
        28_790, 28_795, 28_791, 28_791, 28_795, 28_795, 28_793, 28_793, 28_795,
    ];
    let totemic = [
        28_827, 28_824, 28_826, 28_826, 28_824, 28_824, 28_825, 28_825, 28_824,
    ];
    for (id, class, given) in out {
        let i = [1u8, 2, 3, 4, 5, 7, 8, 9, 11]
            .iter()
            .position(|c| *c == class);
        let want: Vec<u32> = match (id, i) {
            (HOLY_POWER, Some(i)) => vec![holy[i]],
            (_, Some(i)) => vec![totemic[i]],
            _ => vec![],
        };
        assert_eq!(given, want, "{id} on class {class}");
    }
    assert!(!dead, "none for the dead");
}

/// Thistle Tea's Holy Power proc: only Holy Light and Flash of Light that
/// land on the living count (not another class's heal, Holy Shock, a
/// harmful spell, a tick, a resist or a cast's end); the aura's charges
/// and cooldown are shared, and one on oneself or an overheal counts.
#[tokio::test]
#[ignore = "extended"]
async fn holy_power_procs_on_its_heals_with_its_charges_and_cooldown() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        if let Some(s) = d.spells.spells.get_mut(&HOLY_POWER) {
            s.proc_chance = 100;
            s.proc_charges = 2;
        }
        if let Some(p) = d.spells.proc_events.get_mut(&HOLY_POWER) {
            p.cooldown = 2_000;
        }
    });
    let (_c, healer, map) = player(&realm, "P5D7", "Holypowb", Race::Human, Class::Paladin).await;
    let (_d, other, _) = player(&realm, "P5D8", "Healedb", Race::Human, Class::Mage).await;
    let got = map
        .query(move |m| {
            ready(m, healer, 60, &[]);
            ready(m, other, 60, &[]);
            set_class(m, other, 8);
            let heal = |to: Guid, flags: u32, ex: u32, spell: u32| Event {
                actor: healer,
                target: to,
                actor_flags: flags,
                target_flags: 0,
                ex,
                spell: Some(spell),
                damage: 0,
                school: 2,
                ..Default::default()
            };
            let gave = |m: &mut Map, e: Event, buff: u32| {
                procs::trigger(m, e);
                let to = e.target;
                let got = has(m, to, buff);
                auras::remove_spell(m, to, buff, None);
                got
            };
            apply(m, healer, healer, HOLY_POWER);
            let mut refused = Vec::new();
            for e in [
                heal(other, PROC_DONE_POSITIVE_SPELL, EX_NORMAL_HIT, LESSER_HEAL),
                heal(other, PROC_DONE_POSITIVE_SPELL, EX_NORMAL_HIT, HOLY_SHOCK),
                heal(other, PROC_DONE_NEGATIVE_SPELL, EX_NORMAL_HIT, HOLY_LIGHT),
                heal(other, PROC_DONE_PERIODIC, EX_PERIODIC_POSITIVE, HOLY_LIGHT),
                heal(other, PROC_DONE_POSITIVE_SPELL, EX_RESIST, HOLY_LIGHT),
                heal(other, PROC_DONE_POSITIVE_SPELL, EX_CAST_END, HOLY_LIGHT),
            ] {
                refused.push((e.spell, e.actor_flags, e.ex, gave(m, e, 28_793)));
            }
            let charges = holder(m, healer, HOLY_POWER).map(|h| h.charges);
            // An overheal (no health given) on the mage: the caster's buff.
            let first = gave(
                m,
                heal(other, PROC_DONE_POSITIVE_SPELL, EX_NORMAL_HIT, HOLY_LIGHT),
                28_793,
            );
            let spent = holder(m, healer, HOLY_POWER).map(|h| h.charges);
            m.now += Duration::from_millis(1_999);
            let cooling = gave(
                m,
                heal(healer, PROC_DONE_POSITIVE_SPELL, EX_NORMAL_HIT, HOLY_LIGHT),
                28_795,
            );
            m.now += Duration::from_millis(1);
            // On herself, a paladin: the mana user's buff, the last charge.
            let own = gave(
                m,
                heal(healer, PROC_DONE_POSITIVE_SPELL, EX_NORMAL_HIT, HOLY_LIGHT),
                28_795,
            );
            let gone = !has(m, healer, HOLY_POWER);
            (refused, charges, first, spent, cooling, own, gone)
        })
        .await
        .unwrap();
    let (refused, charges, first, spent, cooling, own, gone) = got;
    for (spell, flags, ex, gave) in refused {
        assert!(!gave, "{spell:?} {flags:#x} {ex:#x}: none");
    }
    assert_eq!(charges, Some(2), "no charge spent on those");
    assert!(first, "Holy Light, an overheal, counts");
    assert_eq!(spent, Some(1));
    assert!(!cooling, "within its cooldown");
    assert!(own && gone, "on herself, and its charges spent");
}

/// A bow of three seconds, for the rates a minute.
const SLOW_BOW: u32 = 96_900;

/// Thistle Tea's real proc chances: the ranged procs a minute (23578's
/// two, 26480's ten) are 10% and 50% by a bow of three seconds; Aspect of
/// the Hawk's own chance is none, Improved Aspect of the Hawk's ranks give
/// it 1% and 5% at once, and once; Nature's Grasp's 35% on a blow taken is
/// sure with Improved Nature's Grasp.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_proc_chances_by_the_bow_and_the_talents() {
    use crate::game::map::combat::Hand;
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        d.items.insert(
            SLOW_BOW,
            crate::game::data::items::ItemTemplate {
                entry: SLOW_BOW,
                class: 2,
                subclass: 2,
                name: "Slow Bow".into(),
                display_id: 1000,
                inventory_type: 15,
                delay: 3000,
                stackable: 1,
                ..Default::default()
            },
        );
    });
    let (_c, h, map) = player(&realm, "P5D9", "Hawka", Race::Dwarf, Class::Hunter).await;
    let (_d, dr, map1) = player(&realm, "P5D10", "Graspa", Race::NightElf, Class::Druid).await;
    let got = map
        .query(move |m| {
            ready(m, h, 60, &[]);
            crate::game::tests_p3_combat::equip(m, h, SLOW_BOW, 17);
            let data = m.realm.data.clone();
            let chance = |m: &Map, id: u32| {
                let s = data.spells.get(id).unwrap();
                crate::game::map::procs::chance(m, h, s, Hand::Ranged, true)
            };
            let rates = (chance(m, 23_578), chance(m, 26_480));
            apply(m, h, h, 13_165);
            let mut hawk = vec![chance(m, 13_165)];
            for talent in [19_552, 19_556] {
                apply(m, h, h, talent);
                fresh(m, h);
                spells::cast_triggered(m, h, 13_165, Some(h));
                hawk.push(chance(m, 13_165));
                auras::remove_spell(m, h, talent, None);
            }
            (rates, hawk)
        })
        .await
        .unwrap();
    let grasp = map1
        .query(move |m| {
            ready(m, dr, 60, &[]);
            let data = m.realm.data.clone();
            let s = data.spells.get(16_689).unwrap().clone();
            let chance = |m: &Map| crate::game::map::procs::chance(m, dr, &s, Hand::Main, false);
            let plain = chance(m);
            apply(m, dr, dr, 17_249);
            (s.proc_chance, plain, chance(m))
        })
        .await
        .unwrap();
    let (rates, hawk) = got;
    assert_eq!(rates, (10.0, 50.0));
    assert_eq!(hawk, vec![0.0, 1.0, 5.0]);
    assert_eq!(grasp, (35, 35.0, 100.0));
}

/// Thistle Tea's Separation Anxiety (23487 on Gehennas's adds, its buff
/// 23492; 21094 on Garr's, 21095): every five seconds, an add 100 yards
/// plus both bounding radii (1.5 and 2) or more from its boss, in three
/// dimensions, grows stronger; nearer it doesn't, and a buff it has runs
/// out. A dead or missing boss empowers nothing.
#[tokio::test]
#[ignore = "extended"]
async fn separation_anxiety_empowers_an_add_far_from_its_boss() {
    use crate::game::map::boss_mechanics;
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, w, map) = player(&realm, "P5D11", "Anxiousa", Race::Human, Class::Warrior).await;
    let got = map
        .query(move |m| {
            ready(m, w, 60, &[]);
            let boss = dummy(m, w, 60, 5.0);
            let add = dummy(m, w, 60, 6.0);
            for (g, r) in [(add, 1.5f32), (boss, 2.0)] {
                if let Some(f) = m.unit_fields_mut(g) {
                    f.set_f32(UNIT_FIELD_BOUNDINGRADIUS, r);
                }
            }
            let at = m.creature(boss).unwrap().position;
            // Five seconds of checks, from a fresh start: whether `buff` came.
            let check = |m: &mut Map, buff: u32| {
                auras::remove_spell(m, add, buff, None);
                m.bosses.separation.clear();
                m.bosses.next = None;
                boss_mechanics::update(m);
                m.now += Duration::from_secs(5);
                m.bosses.next = None;
                boss_mechanics::update(m);
                has(m, add, buff)
            };
            apply(m, boss, add, 23_487);
            let mut far = Vec::new();
            // Straight up a hair past it: a height of 187 holds 103.5 short in f32.
            for (dx, dz) in [
                (103.499f32, 0.0f32),
                (103.5, 0.0),
                (0.0, 103.51),
                (90.0, 40.0),
            ] {
                crate::game::map::scripts::place(
                    m,
                    add,
                    Vector3d {
                        x: at.x + dx,
                        y: at.y,
                        z: at.z + dz,
                    },
                    0.0,
                );
                far.push(check(m, 23_492));
            }
            // Garr's: near, its buff runs out unrefreshed.
            auras::remove_spell(m, add, 23_487, None);
            apply(m, boss, add, 21_094);
            crate::game::map::scripts::place(
                m,
                add,
                Vector3d {
                    x: at.x + 10.0,
                    ..at
                },
                0.0,
            );
            apply(m, add, add, 21_095);
            let lasts = holder(m, add, 21_095).and_then(|h| h.duration).unwrap_or(0);
            m.bosses.next = None;
            boss_mechanics::update(m);
            pass(m, u64::from(lasts));
            m.bosses.next = None;
            boss_mechanics::update(m);
            let near = (has(m, add, 21_095), has(m, add, 21_094));
            // A dead boss, then none.
            crate::game::map::scripts::place(
                m,
                add,
                Vector3d {
                    x: at.x + 150.0,
                    ..at
                },
                0.0,
            );
            crate::game::map::combat::deal_damage(m, w, boss, 10_000_000);
            let dead = check(m, 21_095);
            (far, near, dead)
        })
        .await
        .unwrap();
    let (far, near, dead) = got;
    assert_eq!(
        far,
        vec![false, true, true, false],
        "the strict boundary, in three dimensions"
    );
    assert_eq!(
        near,
        (false, true),
        "near, the buff runs out; the anxiety stays"
    );
    assert!(!dead, "a dead boss empowers nothing");
}
