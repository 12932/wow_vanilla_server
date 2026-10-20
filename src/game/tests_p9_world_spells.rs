//! Phase 9's world spells (Thistle Tea's `world/loader/*_dbc_test.exs` on
//! real spells, a map and a player): the real debuffs' worth when a bar is
//! full, procs from procs and from a periodic trigger's spell, the spells
//! that ignore their caster's modifiers, Improved Mend Pet's ranks and
//! Improved Poisons on Instant Poison.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::data::GameData;
use crate::game::data::spells::{self as sd, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::procs::{self, Event};
use crate::game::map::spells::{self, capacity, mods, outcome};
use crate::game::map::{Map, auras, enchant};

/// Thistle Tea's "real class debuffs retain their reference priority"
/// (`aura_capacity_dbc_test.exs`): each real debuff, on a unit, is worth
/// what vmangos's lists say when a full bar makes one give way (Thistle
/// Tea's `Priority.value`, wvs's `capacity::worth`): Gouge 1, Garrote and
/// Rend 2 (the rogue's and warrior's family flags), Kidney Shot 3,
/// Demoralizing Roar 4, Frostbolt 3, Polymorph 0, Corruption and Shadow
/// Word: Pain 2, Curse of the Elements 3 and Chilled 4 (the ranked lists).
#[tokio::test]
#[ignore = "extended"]
async fn the_real_debuffs_are_worth_their_reference_priority() {
    if !have_data() {
        return;
    }
    const WORTH: [(u32, u8); 11] = [
        (1776, 1),
        (703, 2),
        (772, 2),
        (408, 3),
        (99, 4),
        (116, 3),
        (118, 0),
        (172, 2),
        (589, 2),
        (1490, 3),
        (6136, 4),
    ];
    let realm = realm();
    let (_c, g, map) = player(&realm, "P9WS1", "Worthy", Race::Human, Class::Mage).await;
    let worth = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let data = m.realm.data.clone();
            WORTH
                .iter()
                .map(|(id, _)| {
                    let foe = dummy(m, g, 60, 5.0);
                    apply(m, g, foe, *id);
                    let h = holder(m, foe, *id).filter(|h| !h.positive);
                    (*id, h.map(|h| capacity::worth(&data, foe, &h)))
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    let want: Vec<(u32, Option<u8>)> = WORTH.iter().map(|(id, p)| (*id, Some(*p))).collect();
    assert_eq!(worth, want);
}

const MAGIC_ABSORPTION: u32 = 29_447;
const HELLFIRE: u32 = 1949;
const HELLFIRE_EFFECT: u32 = 5857;
const ARCANE_MISSILES: u32 = 5143;
const ARCANE_MISSILE: u32 = 7268;
/// Proc auras on harmful spells dealt, of ten charges, a charge a proc:
/// one without Magic Absorption's attributes, one with them; and one on a
/// swing that casts an Arcane Missile at what it struck.
const ORDINARY: u32 = 990_930;
const FROM_PROCS: u32 = 990_931;
const MISSILE_ON_SWING: u32 = 990_932;

fn proc_auras(d: &mut GameData) {
    let absorption = d.spells.get(MAGIC_ABSORPTION).cloned().unwrap();
    let on = |id: u32, aura: u32, flags: u32, charges: u32| {
        let mut s = made_up(id, 6, aura, 0, 0);
        s.proc_flags = flags;
        s.proc_chance = 100;
        s.proc_charges = charges;
        s
    };
    let spells = procs::PROC_DONE_NEGATIVE_SPELL | procs::PROC_DONE_AOE;
    let ordinary = on(ORDINARY, auras::AURA_DUMMY, spells, 10);
    let mut from_procs = on(FROM_PROCS, auras::AURA_DUMMY, spells, 10);
    from_procs.attributes[3] = absorption.attributes[3];
    let mut swing = on(
        MISSILE_ON_SWING,
        auras::AURA_PROC_TRIGGER_SPELL,
        procs::PROC_DONE_MELEE,
        0,
    );
    swing.effects[0].trigger_spell = ARCANE_MISSILE;
    for s in [ordinary, from_procs, swing] {
        d.spells.spells.insert(s.id, s);
    }
}

/// The charges `g`'s two spell procs have spent since `was`.
fn spent(m: &Map, g: Guid, was: (u32, u32)) -> (u32, u32) {
    let left = |id: u32| holder(m, g, id).map_or(0, |h| h.charges);
    (was.0 - left(ORDINARY), was.1 - left(FROM_PROCS))
}

/// Thistle Tea's "real channel exceptions distinguish ordinary proc auras
/// from CAN_PROC_FROM_PROCS" (`cast_proc_dbc_test.exs`): Magic Absorption
/// (29447) has `ATTR_EX3_CAN_PROC_FROM_PROCS`, and Arcane Missiles'
/// missile (7268) `ATTR_EX3_NOT_A_PROC` where Hellfire's flames (5857)
/// have none. Each spell a periodic trigger casts procs both an ordinary
/// proc aura and one with Magic Absorption's attributes: the missile as
/// Thistle Tea and mangos have it, the flames too, where they would
/// spare the ordinary one (mangos's `IsTriggeredAtSpellProcEvent` refuses
/// a triggered spell's proc to an aura without the attribute unless the
/// spell is `NOT_A_PROC`; wvs's `procs::run` refuses only a spell a proc
/// cast). A missile a proc casts procs only the one with the attribute.
#[tokio::test]
#[ignore = "extended"]
async fn a_spell_from_a_proc_procs_only_what_may_proc_from_procs() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(proc_auras);
    let d = realm.data.clone();
    let attr = |id: u32, bit: u32| d.spells.get(id).unwrap().attr(3, bit);
    assert!(attr(MAGIC_ABSORPTION, sd::ATTR_EX3_CAN_PROC_FROM_PROCS));
    assert!(attr(ARCANE_MISSILE, sd::ATTR_EX3_NOT_A_PROC));
    assert!(!attr(HELLFIRE_EFFECT, sd::ATTR_EX3_NOT_A_PROC));
    let (_c, g, map) = player(&realm, "P9WS2", "Procker", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let foe = dummy(m, g, 60, 3.0);
            for id in [ORDINARY, FROM_PROCS, MISSILE_ON_SWING] {
                apply(m, g, g, id);
            }
            // Hellfire's tick: its flames about the warlock.
            apply(m, g, g, HELLFIRE);
            tick_now(m, g, HELLFIRE);
            let flames = spent(m, g, (10, 10));
            auras::remove_spell(m, g, HELLFIRE, None);
            // Arcane Missiles' tick: a missile at the warlock's target.
            if let Some(f) = m.unit_fields_mut(g) {
                f.set_guid(UNIT_FIELD_TARGET, foe);
            }
            apply(m, g, g, ARCANE_MISSILES);
            tick_now(m, g, ARCANE_MISSILES);
            let missile = spent(m, g, (9, 9));
            auras::remove_spell(m, g, ARCANE_MISSILES, None);
            // A swing's proc: a missile from a proc.
            procs::trigger(
                m,
                Event {
                    actor: g,
                    target: foe,
                    actor_flags: procs::PROC_DONE_MELEE,
                    target_flags: procs::PROC_TAKEN_MELEE,
                    ex: procs::EX_NORMAL_HIT,
                    damage: 10,
                    school: 1,
                    ..Default::default()
                },
            );
            let from_a_proc = spent(m, g, (8, 8));
            (flames, missile, from_a_proc)
        })
        .await
        .unwrap();
    let (flames, missile, from_a_proc) = out;
    assert_eq!(missile, (1, 1), "a missile: both");
    assert_eq!(
        flames,
        (1, 1),
        "the flames: both (mangos: the flagged one only)"
    );
    assert_eq!(
        from_a_proc,
        (0, 1),
        "a proc's missile: the flagged one only"
    );
}

/// The direct spells that ignore their caster's modifiers: Volatile
/// Infection's three, Mana Detonation, Positive and Negative Charge and
/// Decimate.
const DIRECT: [u32; 7] = [3585, 3587, 18_150, 27_820, 28_062, 28_085, 28_375];
/// The periodic ones: the Gnomish Death Ray's hurt, Web Wrap and Slime.
const DEATH_RAY: u32 = 13_493;
const WEB_WRAP: u32 = 28_622;
const SLIME: u32 = 28_801;
/// Volatile Infection (3585) and Slime without the attribute.
const PLAIN_INFECTION: u32 = 990_940;
const PLAIN_SLIME: u32 = 990_941;

/// The direct spells aimed at an enemy (theirs are a party's or an
/// area's), and the two without the attribute.
fn aimed(d: &mut GameData) {
    for id in DIRECT {
        if let Some(s) = d.spells.spells.get_mut(&id) {
            s.effects[0].target_a = 6;
            s.effects[0].target_b = 0;
        }
    }
    for (from, id) in [(3585, PLAIN_INFECTION), (SLIME, PLAIN_SLIME)] {
        let mut s: SpellTemplate = d.spells.get(from).cloned().unwrap();
        s.id = id;
        s.attributes[3] &= !sd::ATTR_EX3_IGNORE_CASTER_MODIFIERS;
        d.spells.spells.insert(id, s);
    }
}

/// `spell`'s periodic damage on `g`, cast on itself, and how long it has
/// left (ms; none for Slime's, which lasts), taken off again.
fn snapshot(m: &mut Map, g: Guid, spell: u32) -> Option<(i32, Option<u64>)> {
    spells::cast_triggered(m, g, spell, Some(g));
    let now = m.now;
    let h = holder(m, g, spell)?;
    auras::remove_spell(m, g, spell, None);
    let amount = h
        .effects
        .iter()
        .find(|x| x.kind == auras::AURA_PERIODIC_DAMAGE)?
        .amount;
    let left = h.expires.map(|t| t.duration_since(now).as_millis() as u64);
    Some((amount, left))
}

/// Thistle Tea's "flagged direct spells ignore caster power, percentages,
/// and criticals" and "flagged periodic effects retain their base
/// snapshots and expiry" (`caster_modifiers_dbc_test.exs`): every one of
/// the real spells has `ATTR_EX3_IGNORE_CASTER_MODIFIERS`; under a
/// hundred spell damage, twice the damage done, every spell a crit and a
/// hundred percent more damage and duration, the direct ones deal what
/// they did without it, never a crit, and the periodic ones hold their
/// amount and last as long (the Death Ray's is its script's roll, 100 to
/// 500; Web Wrap's its dice's, 657 to 843). Volatile Infection and Slime
/// without the attribute take it all. (The direct spells are aimed at an
/// enemy for the test; made-up spells show the same in
/// `tests_p3_spells_damage::a_spell_that_ignores_its_casters_modifiers_takes_no_power_percent_or_damage_crit`.)
#[tokio::test]
#[ignore = "extended"]
async fn the_spells_that_ignore_their_caster_take_nothing_of_its_bonuses() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(aimed);
    let d = realm.data.clone();
    for id in DIRECT.into_iter().chain([DEATH_RAY, WEB_WRAP, SLIME]) {
        let s = d.spells.get(id).unwrap();
        assert!(s.attr(3, sd::ATTR_EX3_IGNORE_CASTER_MODIFIERS), "{id}");
    }
    let (_c, g, map) = player(&realm, "P9WS3", "Unswayed", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let foe = dummy(m, g, 60, 5.0);
            let strike = |m: &mut Map, id: u32| {
                let before = health(m, foe);
                spells::cast_triggered(m, g, id, Some(foe));
                before - health(m, foe)
            };
            let direct = |m: &mut Map| -> Vec<u32> {
                DIRECT
                    .into_iter()
                    .chain([PLAIN_INFECTION])
                    .map(|id| strike(m, id))
                    .collect()
            };
            let periodic = |m: &mut Map| {
                [DEATH_RAY, WEB_WRAP, SLIME, PLAIN_SLIME].map(|id| snapshot(m, g, id))
            };
            let plain = (direct(m), periodic(m));
            for (i, (kind, misc, amount)) in [
                (auras::AURA_MOD_DAMAGE_DONE, 127, 100),
                (auras::AURA_MOD_DAMAGE_PERCENT_DONE, 127, 100),
                (auras::AURA_MOD_SPELL_CRIT_CHANCE, 0, 100),
                (auras::AURA_ADD_PCT_MODIFIER, mods::OP_DAMAGE as i32, 100),
                (auras::AURA_ADD_PCT_MODIFIER, mods::OP_DURATION as i32, 100),
            ]
            .into_iter()
            .enumerate()
            {
                let mut s = made_up(990_950 + i as u32, 6, kind, misc, amount);
                s.duration_index = 21;
                apply_template(m, g, g, &s);
            }
            (plain, (direct(m), periodic(m)))
        })
        .await
        .unwrap();
    let ((direct, periodic), (boosted, boosted_periodic)) = out;
    let n = DIRECT.len();
    assert!(direct[..3].iter().all(|d| *d > 0), "{direct:?}");
    assert_eq!(boosted[..n], direct[..n], "no power, percent or crit");
    assert!(
        boosted[n] > direct[n] * 2,
        "{boosted:?}: without the attribute, all of it"
    );
    let [ray, wrap, slime, plain_slime] = periodic.map(|p| p.expect("on"));
    let [b_ray, b_wrap, b_slime, b_plain_slime] = boosted_periodic.map(|p| p.expect("on"));
    assert!((100..=500).contains(&b_ray.0), "{b_ray:?}");
    assert!((657..=843).contains(&b_wrap.0), "{b_wrap:?}");
    assert_eq!((slime.0, b_slime.0), (100, 100), "Slime's tick held");
    assert_eq!(
        [b_ray.1, b_wrap.1, b_slime.1],
        [ray.1, wrap.1, slime.1],
        "as long"
    );
    assert!(b_plain_slime.0 >= plain_slime.0 * 2, "{b_plain_slime:?}");
}

const IMPROVED_MEND_PET: [(u32, i32, i32); 2] = [(19_572, 4086, 15), (19_573, 4087, 50)];
const MEND_PET: u32 = 136;
const CLEANSE: u32 = 24_406;

/// Thistle Tea's "Mend Pet rank amounts and dispel types come from DBC"
/// (`class_script_dbc_test.exs`): Improved Mend Pet's ranks override class
/// scripts 4086 and 4087 with 15 and 50; the second rank takes the first's
/// rule (Hunter's family, Mend Pet's flag, a periodic heal's tick); a Mend
/// Pet tick then cleanses (24406, a dispel of type 7) on a roll under the
/// rank's amount and not on one at it, and Mend Pet's cast, a helpful
/// spell, never does.
#[tokio::test]
#[ignore = "extended"]
async fn improved_mend_pet_cleanses_by_its_ranks_chance_on_a_mend_pets_tick() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let cleanse = d.spells.get(CLEANSE).unwrap().effects[0];
    assert_eq!((cleanse.effect, cleanse.misc_value), (38, 7));
    let rule = |id: u32| d.spells.proc_events.get(&id).cloned();
    let first = rule(19_572).expect("a rule");
    assert_eq!(
        (first.family, first.family_mask[0], first.flags, first.ex),
        (
            9,
            0x80_0000,
            procs::PROC_DONE_PERIODIC,
            procs::EX_PERIODIC_POSITIVE
        )
    );
    assert_eq!(rule(19_573).map(|r| r.flags), Some(first.flags));
    let (mut c, hunter, map) = player(&realm, "P9WS4", "Mender", Race::Dwarf, Class::Hunter).await;
    let wolf = map
        .query(move |m| {
            ready(m, hunter, 60, &[]);
            dummy(m, hunter, 60, 3.0)
        })
        .await
        .unwrap();
    for (rank, script, chance) in IMPROVED_MEND_PET {
        let put_on = map
            .query(move |m| {
                apply(m, hunter, hunter, rank);
                apply(m, hunter, wolf, MEND_PET);
                holder(m, hunter, rank).and_then(|h| h.effects.first().map(|x| (x.misc, x.amount)))
            })
            .await
            .unwrap();
        assert_eq!(put_on, Some((script, chance)), "{rank}");
        // A tick with a roll of `roll` (hundredths of a percent): whether
        // the roll was spent.
        let tick = |roll: i32| {
            map.query(move |m| {
                m.spells.queued_rolls.lock().unwrap().push_back(roll);
                tick_now(m, wolf, MEND_PET);
                let spent = m.spells.queued_rolls.lock().unwrap().is_empty();
                m.spells.queued_rolls.lock().unwrap().clear();
                spent
            })
        };
        assert!(tick(chance * 100 - 1).await.unwrap(), "{rank}: rolled");
        assert!(saw_go(&mut c, CLEANSE).await, "{rank}: under its chance");
        assert!(tick(chance * 100).await.unwrap());
        assert!(!saw_go(&mut c, CLEANSE).await, "{rank}: at it, nothing");
        map.query(move |m| {
            m.spells.queued_rolls.lock().unwrap().push_back(0);
            let s = m.realm.data.spells.get(MEND_PET).cloned().unwrap();
            let (done, taken) = outcome::proc_flags(&s);
            procs::trigger(
                m,
                Event {
                    actor: hunter,
                    target: wolf,
                    actor_flags: done,
                    target_flags: taken,
                    ex: procs::EX_NORMAL_HIT,
                    spell: Some(MEND_PET),
                    school: s.school_mask(),
                    ..Default::default()
                },
            );
            m.spells.queued_rolls.lock().unwrap().clear();
            auras::remove_spell(m, hunter, rank, None);
            auras::remove_spell(m, wolf, MEND_PET, None);
        })
        .await
        .unwrap();
        assert!(!saw_go(&mut c, CLEANSE).await, "{rank}: the cast, nothing");
    }
}

const IMPROVED_POISONS: u32 = 14_117;
const INSTANT_POISON: u32 = 8680;
/// Instant Poison's enchantment: 8680 on 20% of blows.
const INSTANT_POISON_ENCHANT: u32 = 323;
const WORN_DAGGER: u32 = 2092;

/// Thistle Tea's "real Improved Poisons increases Instant Poison's weapon
/// proc chance" (`enchantments_dbc_test.exs`): Instant Poison's
/// enchantment goes off on a fifth of blows; Improved Poisons' first rank
/// (14117, a flat +10 to its chance of success, its mask 0x1001E000 over
/// Instant Poison's 0x2000) makes it 30%, as mangos's
/// `CastItemCombatSpell` applies the wielder's modifiers to the
/// enchantment's spell: a roll of 25 poisons with it, not without, and
/// one of 31 neither.
#[tokio::test]
#[ignore = "extended"]
async fn improved_poisons_raises_instant_poisons_chance_to_thirty() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let talent = d.spells.get(IMPROVED_POISONS).unwrap().effects[0];
    assert_eq!(
        (talent.aura, talent.misc_value, talent.item_type & 0x2000),
        (
            auras::AURA_ADD_FLAT_MODIFIER,
            mods::OP_CHANCE_OF_SUCCESS as i32,
            0x2000
        )
    );
    assert_eq!(d.spells.get(INSTANT_POISON).unwrap().family_flags, 0x2000);
    let (_c, g, map) = player(&realm, "P9WS5", "Poisoner", Race::Human, Class::Rogue).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            crate::game::tests_p3_combat::equip(m, g, WORN_DAGGER, 15);
            let p = m.players.get_mut(&g).unwrap();
            let i = p.items.iter().position(|i| i.entry == WORN_DAGGER).unwrap();
            enchant::set(
                &mut p.items[i],
                enchant::TEMP_SLOT,
                enchant::Enchant {
                    id: INSTANT_POISON_ENCHANT,
                    duration_ms: 1_800_000,
                    charges: 0,
                },
            );
            let foe = dummy(m, g, 60, 2.0);
            let poisoned = |m: &mut Map, roll: f32| {
                let before = health(m, foe);
                enchant::FORCED_PROC_ROLL.with(|f| f.set(Some(roll)));
                procs::trigger(
                    m,
                    Event {
                        actor: g,
                        target: foe,
                        actor_flags: procs::PROC_DONE_MELEE,
                        target_flags: procs::PROC_TAKEN_MELEE,
                        ex: procs::EX_NORMAL_HIT,
                        damage: 10,
                        school: 1,
                        ..Default::default()
                    },
                );
                enchant::FORCED_PROC_ROLL.with(|f| f.set(None));
                health(m, foe) < before
            };
            let untalented = (poisoned(m, 19.0), poisoned(m, 25.0));
            apply(m, g, g, IMPROVED_POISONS);
            let talented = (poisoned(m, 25.0), poisoned(m, 31.0));
            (untalented, talented)
        })
        .await
        .unwrap();
    assert_eq!(out.0, (true, false), "a fifth of blows");
    assert_eq!(out.1, (true, false), "three tenths with the talent");
}
