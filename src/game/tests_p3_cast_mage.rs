//! Phase 3's mage and rogue spells, on the real data, as Thistle Tea's
//! class tests have them: the wards' reflection talents, Combustion's
//! charges, and the rogue's finishers, Cold Blood, Ghostly Strike and
//! Vanish.

use std::time::Duration;

use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::map::spells::{mods, outcome};
use crate::game::map::{Map, auras, procs};

const FIRE_WARD: u32 = 543;
const FROST_WARD: u32 = 6143;
const IMPROVED_FIRE_WARD: u32 = 13043;
const FROST_WARDING: u32 = 28332;
const COMBUSTION: u32 = 11129;
const COMBUSTION_BUFF: u32 = 28682;
const FIREBALL: u32 = 133;
const FROSTBOLT: u32 = 116;
/// A made-up fire bolt and frost bolt of 10, instant, for a kobold.
const FIRE_BOLT: u32 = 990_600;
const FROST_BOLT: u32 = 990_601;

fn bolts(d: &mut crate::game::data::GameData) {
    use super::tests_p3_cast_kit::{bare, damage, effect, insert};
    insert(
        d,
        [(FIRE_BOLT, 2), (FROST_BOLT, 4)].map(|(id, school)| {
            let mut s = bare(id, school);
            s.range_index = 4;
            effect(&mut s, 0, damage(10));
            s
        }),
    );
}

/// How many of `n` of the kobold's bolts of `spell` come back to it, the
/// mage's ward of `ward` up afresh for each.
fn reflected(
    m: &mut Map,
    mage: wow_world_messages::Guid,
    k: wow_world_messages::Guid,
    ward: u32,
    spell: u32,
    n: u32,
) -> u32 {
    let mut back = 0;
    for _ in 0..n {
        auras::remove_spell(m, mage, ward, None);
        apply(m, mage, mage, ward);
        set_health(m, k, 1_000_000);
        let went = cast_fresh(m, k, spell, at(mage));
        pass(m, 100);
        assert!(went, "the bolt went");
        if health(m, k) < 1_000_000 {
            back += 1;
        }
    }
    back
}

/// Improved Fire Ward's 20% to reflect fire with Fire Ward up; Frost
/// Warding's only with Frost Ward, not Fire Ward (Thistle Tea's ward
/// reflection talents).
#[tokio::test]
#[ignore = "extended"]
async fn the_ward_talents_reflect_their_own_wards_school() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(bolts);
    let (_c, mage, map) = player(&realm, "P3M1", "Wardone", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let k = dummy(m, mage, 60, 5.0);
            m.spells.rolls = Some(true);
            let none = reflected(m, mage, k, FIRE_WARD, FIRE_BOLT, 200);
            apply(m, mage, mage, IMPROVED_FIRE_WARD);
            let improved = reflected(m, mage, k, FIRE_WARD, FIRE_BOLT, 1_000);
            auras::remove_spell(m, mage, IMPROVED_FIRE_WARD, None);
            apply(m, mage, mage, FROST_WARDING);
            let warding_fire = reflected(m, mage, k, FIRE_WARD, FIRE_BOLT, 200);
            auras::remove_spell(m, mage, FIRE_WARD, None);
            let warding_frost = reflected(m, mage, k, FROST_WARD, FROST_BOLT, 1_000);
            (none, improved, warding_fire, warding_frost)
        })
        .await
        .unwrap();
    let (none, improved, warding_fire, warding_frost) = out;
    assert_eq!(none, 0, "a ward alone reflects nothing");
    assert!(
        (140..=270).contains(&improved),
        "20% with Improved Fire Ward: {improved}"
    );
    assert_eq!(warding_fire, 0, "Frost Warding does nothing for Fire Ward");
    assert!(
        (140..=270).contains(&warding_frost),
        "20% of frost with Frost Ward: {warding_frost}"
    );
}

/// A fire spell of the mage's landing on the kobold, a crit or not.
fn fire_landed(
    m: &mut Map,
    mage: wow_world_messages::Guid,
    k: wow_world_messages::Guid,
    spell: u32,
    crit: bool,
) {
    let s: SpellTemplate = m.realm.data.spells.get(spell).cloned().unwrap();
    let (done, taken) = outcome::proc_flags(&s);
    procs::trigger(
        m,
        procs::Event {
            actor: mage,
            target: k,
            actor_flags: done,
            target_flags: taken,
            ex: if crit {
                procs::EX_CRITICAL_HIT
            } else {
                procs::EX_NORMAL_HIT
            },
            spell: Some(spell),
            damage: 100,
            school: s.school_mask(),
            ..Default::default()
        },
    );
}

/// Combustion: its cast is a stack of its crit; a frost crit does
/// nothing; a fire spell that doesn't crit adds a stack; a fire crit spends a charge, and the third takes it
/// and its buff off and starts its cooldown; and cancelling the buff takes
/// Combustion with it (Thistle Tea's Combustion, mangos's proc handler).
#[tokio::test]
#[ignore = "extended"]
async fn combustion_grows_on_fire_and_goes_with_its_third_crit() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (c, mage, map) = player(&realm, "P3M2", "Combuster", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[COMBUSTION]);
            let k = dummy(m, mage, 60, 5.0);
            let charges = |m: &Map| holder(m, mage, COMBUSTION).map(|h| h.charges);
            cast_fresh(m, mage, COMBUSTION, Default::default());
            let start = charges(m);
            let stacks = |m: &Map| holder(m, mage, COMBUSTION_BUFF).map(|h| h.stacks);
            let first = stacks(m);
            fire_landed(m, mage, k, FROSTBOLT, true);
            let frost = (charges(m), stacks(m));
            fire_landed(m, mage, k, FIREBALL, false);
            let grown = (charges(m), stacks(m));
            fire_landed(m, mage, k, FIREBALL, true);
            let spent = charges(m);
            fire_landed(m, mage, k, FIREBALL, true);
            let cooling_before = cooling(m, mage);
            fire_landed(m, mage, k, FIREBALL, true);
            let gone = (has(m, mage, COMBUSTION), has(m, mage, COMBUSTION_BUFF));
            let cooling_after = cooling(m, mage);
            // Again, for the cancel.
            fresh(m, mage);
            cast_fresh(m, mage, COMBUSTION, Default::default());
            fire_landed(m, mage, k, FIREBALL, false);
            (
                start,
                first,
                frost,
                grown,
                spent,
                cooling_before,
                gone,
                cooling_after,
            )
        })
        .await
        .unwrap();
    let (start, first, frost, grown, spent, cooling_before, gone, cooling_after) = out;
    assert_eq!(start, Some(3), "three charges");
    assert_eq!(first, Some(1), "the cast's first stack of its crit");
    assert_eq!(frost, (Some(3), Some(1)), "frost does nothing");
    assert_eq!(
        grown,
        (Some(3), Some(2)),
        "a fire spell's another stack, no charge"
    );
    assert_eq!(spent, Some(2), "a fire crit's charge");
    assert!(!cooling_before, "its cooldown waits for the last charge");
    assert_eq!(gone, (false, false), "the third crit takes both");
    assert!(cooling_after, "and its cooldown starts");
    c.send(CMSG_CANCEL_AURA {
        id: COMBUSTION_BUFF,
    })
    .await;
    let mut both = (true, true);
    for _ in 0..200 {
        both = map
            .query(move |m| (has(m, mage, COMBUSTION), has(m, mage, COMBUSTION_BUFF)))
            .await
            .unwrap();
        if both == (false, false) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        both,
        (false, false),
        "cancelling the buff takes Combustion too"
    );
}

/// Whether Combustion is cooling down.
fn cooling(m: &Map, g: wow_world_messages::Guid) -> bool {
    let now = m.now;
    m.caster(g).is_some_and(|c| {
        c.cooldowns.get(&COMBUSTION).is_some_and(|t| *t > now)
            || c.category_cooldowns.values().any(|t| *t > now)
    })
}

const EXPOSE_ARMOR: u32 = 8647;
const RUPTURE: u32 = 1943;
const GHOSTLY_STRIKE: u32 = 14278;
const COLD_BLOOD: u32 = 14177;
const SINISTER_STRIKE: u32 = 1752;
const VANISH: u32 = 1856;
const HUNTERS_MARK: u32 = 1130;

/// `g`'s combo points on `target`.
fn combo(m: &mut Map, g: wow_world_messages::Guid, target: wow_world_messages::Guid, points: u8) {
    if let Some(p) = m.players.get_mut(&g) {
        p.caster.combo_points = points;
        p.caster.combo_target = target;
    }
}

/// The rogue's: Expose Armor takes its points a combo point off armor; a
/// finisher's duration goes from its base to its most by the points;
/// Ghostly Strike's dodge goes on the rogue alone; Cold Blood makes the
/// next strike a sure crit; Vanish takes Hunter's Mark off (Thistle Tea's
/// rogue tests).
#[tokio::test]
#[ignore = "extended"]
async fn the_rogues_finishers_buffs_and_vanish_do_what_they_say() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, rogue, map) = player(&realm, "P3M3", "Rogueone", Race::Human, Class::Rogue).await;
    let out = map
        .query(move |m| {
            ready(
                m,
                rogue,
                60,
                &[
                    EXPOSE_ARMOR,
                    RUPTURE,
                    GHOSTLY_STRIKE,
                    COLD_BLOOD,
                    SINISTER_STRIKE,
                    VANISH,
                ],
            );
            let k = dummy(m, rogue, 60, 2.0);
            if let Some(f) = m.unit_fields_mut(k) {
                f.set_u32(UNIT_FIELD_RESISTANCES, 500);
            }
            if let Some(c) = m.creature_mut(k) {
                c.base.resistances[0] = 500;
            }
            combo(m, rogue, k, 5);
            cast_fresh(m, rogue, EXPOSE_ARMOR, at(k));
            let armor = m.unit_fields(k).unwrap().get_u32(UNIT_FIELD_RESISTANCES);
            let exposed =
                holder(m, k, EXPOSE_ARMOR).and_then(|h| h.effects.first().map(|x| x.amount));
            // Rupture at three points.
            combo(m, rogue, k, 3);
            cast_fresh(m, rogue, RUPTURE, at(k));
            let rupture = holder(m, k, RUPTURE).and_then(|h| h.duration);
            let s = m.realm.data.spells.get(RUPTURE).cloned().unwrap();
            let base = m.realm.data.spells.duration(&s, 60).unwrap();
            let most = m
                .realm
                .data
                .spells
                .durations
                .get(&s.duration_index)
                .map(|d| d.max);
            // Ghostly Strike.
            set_power(m, rogue, 3, 100);
            cast_fresh(m, rogue, GHOSTLY_STRIKE, at(k));
            let ghostly = (has(m, rogue, GHOSTLY_STRIKE), has(m, k, GHOSTLY_STRIKE));
            // Cold Blood: Sinister Strike's crit sure.
            cast_fresh(m, rogue, COLD_BLOOD, Default::default());
            let strike = m.realm.data.spells.get(SINISTER_STRIKE).cloned().unwrap();
            let mods = mods::for_spell(m, rogue, &strike);
            let cold = mods.flat[mods::OP_CRITICAL_CHANCE];
            // Vanish takes a Hunter's Mark.
            apply(m, k, rogue, HUNTERS_MARK);
            let marked = has(m, rogue, HUNTERS_MARK);
            cast_fresh(m, rogue, VANISH, Default::default());
            let vanish = (marked, has(m, rogue, HUNTERS_MARK));
            (armor, exposed, rupture, base, most, ghostly, cold, vanish)
        })
        .await
        .unwrap();
    let (armor, exposed, rupture, base, most, ghostly, cold, vanish) = out;
    let exposed = exposed.expect("Expose Armor on the kobold");
    assert!(exposed < 0, "{exposed}");
    assert_eq!(
        armor as i32,
        (500 + exposed).max(0),
        "armor less Expose Armor's {exposed}"
    );
    // From base to most, a fifth for each point: 3 of 5.
    let most = most.unwrap() as u32;
    let expected = base + (most - base) * 3 / 5;
    assert_eq!(
        rupture,
        Some(expected),
        "Rupture's duration at three points"
    );
    assert_eq!(
        ghostly,
        (true, false),
        "Ghostly Strike's dodge on the rogue"
    );
    assert!(cold >= 100, "Cold Blood's sure crit: {cold}");
    assert_eq!(vanish, (true, false), "Vanish takes the mark off");
}
