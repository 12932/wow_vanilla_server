//! Phase 3, round 5, against Thistle Tea's `warlock_spells_test.exs`, on
//! the real data: Life Tap and its talent, Soul Link's aura on the demon
//! and from it on the warlock, Health Funnel's two auras, Devour Magic with
//! nothing to devour.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::{
    self as kit, has, health, holder, pass, power, set_health, set_power,
};
use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::map::auras::{self, AURA_DUMMY};
use crate::game::map::{Map, pets, spells};

const LIFE_TAP: u32 = 1454;
const IMPROVED_LIFE_TAP: u32 = 18183;
const POWER_WORD_SHIELD: u32 = 17;
/// A made-up aura that damage breaks.
const FRAGILE: u32 = 990_900;
const SUMMON_VOIDWALKER: u32 = 697;
const SUMMON_FELHUNTER: u32 = 691;
const SOUL_LINK: u32 = 19028;
const SOUL_LINK_AURA: u32 = 25228;
/// The link Soul Link's `spell_scripts` row casts, from before its pet
/// aura (`ZZOLD` in 1.12's data).
const LEGACY_SOUL_LINK: u32 = 18814;
const HEALTH_FUNNEL: u32 = 755;
const DEVOUR_MAGIC: u32 = 19505;
const DEVOUR_MAGIC_HEAL: u32 = 19658;

fn realm() -> std::sync::Arc<crate::game::Realm> {
    kit::realm_plus(|d| {
        let mut s = super::tests_p3_cast_kit::bare(FRAGILE, 0);
        s.duration_index = super::tests_p3_cast_kit::LASTS_LONG;
        s.aura_interrupt_flags = 0x2;
        super::tests_p3_cast_kit::effect(
            &mut s,
            0,
            SpellEffect {
                ..super::tests_p3_cast_kit::aura(AURA_DUMMY, 0, 0, 1)
            },
        );
        d.spells.spells.insert(FRAGILE, s);
    })
}

fn max_health(m: &mut Map, g: Guid, v: u32) {
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_MAXHEALTH, v);
        f.set_u32(UNIT_FIELD_HEALTH, v);
    }
}

/// Life Tap turns its health into as much mana, a fifth more with Improved
/// Life Tap's second rank; the health goes past a shield and breaks nothing
/// damage would (Thistle Tea's Life Tap tests).
#[tokio::test]
#[ignore = "extended"]
async fn life_tap_pays_in_health_past_shields_and_its_talent_adds_a_fifth() {
    if !kit::have_data() {
        return;
    }
    let realm = realm();
    let (_a, w, map) = kit::player(&realm, "P5WA", "Tapper", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            kit::ready(m, w, 60, &[LIFE_TAP]);
            max_health(m, w, 5_000);
            let tap = |m: &mut Map| {
                kit::fresh(m, w);
                set_health(m, w, 1_000);
                set_power(m, w, 0, 0);
                spells::cast_triggered(m, w, LIFE_TAP, Some(w));
                (1_000 - health(m, w), power(m, w, 0))
            };
            let plain = tap(m);
            spells::learn(m, w, IMPROVED_LIFE_TAP);
            kit::apply(m, w, w, POWER_WORD_SHIELD);
            kit::apply(m, w, w, FRAGILE);
            let shield = holder(m, w, POWER_WORD_SHIELD).map(|h| h.effects[0].amount);
            let improved = tap(m);
            let kept = (
                holder(m, w, POWER_WORD_SHIELD).map(|h| h.effects[0].amount),
                has(m, w, FRAGILE),
            );
            (plain, improved, shield, kept)
        })
        .await
        .unwrap();
    let ((paid, gained), (paid2, gained2), shield, (shield_after, fragile)) = out;
    assert!(paid > 0 && gained == paid, "as much mana: {paid} {gained}");
    assert_eq!(paid2, paid, "the same health");
    assert_eq!(gained2, paid * 120 / 100, "a fifth more");
    assert!(shield.is_some());
    assert_eq!(shield_after, shield, "the shield untouched");
    assert!(fragile, "nothing broken as damage would");
}

/// Soul Link's dummy puts its aura on the demon, and the demon's area aura
/// puts it on the warlock, who then gives the demon three tenths of the
/// damage taken; its old script's link isn't cast too (Thistle Tea's Soul
/// Link pet auras and Soul Link tests).
#[tokio::test]
#[ignore = "extended"]
async fn soul_link_goes_on_the_demon_and_from_it_on_the_warlock() {
    if !kit::have_data() {
        return;
    }
    let realm = realm();
    let (_a, w, map) = kit::player(&realm, "P5WB", "Linker", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            kit::ready(m, w, 60, &[SOUL_LINK]);
            spells::cast_triggered(m, w, SUMMON_VOIDWALKER, None);
            let pet = pets::pet_of(m, w).expect("the voidwalker");
            kit::fresh(m, w);
            spells::cast_triggered(m, w, SOUL_LINK, Some(pet));
            // The demon's area aura reaches its master at its next pulse.
            for _ in 0..4 {
                pass(m, 1_000);
            }
            let on = (has(m, pet, SOUL_LINK_AURA), has(m, w, SOUL_LINK_AURA));
            let from = holder(m, w, SOUL_LINK_AURA).map(|h| h.caster);
            let kobold = kit::dummy(m, w, 60, 5.0);
            max_health(m, pet, 100_000);
            max_health(m, w, 100_000);
            let legacy = has(m, w, LEGACY_SOUL_LINK);
            let (left, _) = auras::absorb_and_share(m, kobold, w, 100, 1);
            let taken = (left, 100_000 - health(m, pet));
            (pet, on, from, legacy, taken)
        })
        .await
        .unwrap();
    let (pet, on, from, legacy, taken) = out;
    assert!(!legacy, "the pet aura's link, not the old script's");
    assert_eq!(on, (true, true), "on the demon and the warlock");
    assert_eq!(from, Some(pet), "the warlock's from the demon");
    assert_eq!(taken, (70, 30), "three tenths to the demon");
}

/// Health Funnel's heal goes on the demon and its cost aura on the
/// warlock, neither on the other; cancelled, the demon's heal goes
/// (Thistle Tea's Health Funnel tests). Devour Magic with nothing to devour
/// heals its felhunter nothing (its Devour Magic test).
#[tokio::test]
#[ignore = "extended"]
async fn health_funnel_splits_its_auras_and_devour_magic_needs_a_meal() {
    if !kit::have_data() {
        return;
    }
    let realm = realm();
    let (_a, w, map) = kit::player(&realm, "P5WC", "Funneler", Race::Human, Class::Warlock).await;
    let (_b, w2, _) = kit::player(&realm, "P5WD", "Devourer", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            kit::ready(m, w, 60, &[HEALTH_FUNNEL]);
            spells::cast_triggered(m, w, SUMMON_VOIDWALKER, None);
            let pet = pets::pet_of(m, w).expect("the voidwalker");
            max_health(m, w, 5_000);
            kit::fresh(m, w);
            let began = spells::cast(m, w, HEALTH_FUNNEL, kit::at(pet), None);
            let kinds = |m: &Map, g: Guid| {
                holder(m, g, HEALTH_FUNNEL).map_or(Vec::new(), |h| {
                    let mut k: Vec<u32> = h.effects.iter().map(|x| x.kind).collect();
                    k.sort();
                    k
                })
            };
            let on = (kinds(m, pet), kinds(m, w));
            pass(m, 1_500);
            spells::interrupt(m, w);
            let cancelled = (has(m, pet, HEALTH_FUNNEL), has(m, w, HEALTH_FUNNEL));
            // A felhunter's Devour Magic at a kobold with nothing on it.
            kit::ready(m, w2, 60, &[]);
            spells::cast_triggered(m, w2, SUMMON_FELHUNTER, None);
            let hunter = pets::pet_of(m, w2).expect("the felhunter");
            max_health(m, hunter, 10_000);
            set_health(m, hunter, 5_000);
            let kobold = kit::dummy(m, w, 60, 5.0);
            spells::cast_triggered(m, hunter, DEVOUR_MAGIC, Some(kobold));
            let devoured = (health(m, hunter), has(m, hunter, DEVOUR_MAGIC_HEAL));
            (began, on, cancelled, devoured)
        })
        .await
        .unwrap();
    let (began, (on_pet, on_warlock), cancelled, devoured) = out;
    assert!(began);
    assert_eq!(on_pet, [auras::AURA_PERIODIC_HEAL], "the heal on the demon");
    assert_eq!(
        on_warlock,
        [auras::AURA_MOD_HEALTH_REGEN_PERCENT],
        "the cost on the warlock"
    );
    assert_eq!(cancelled, (false, false), "both go with the channel");
    assert_eq!(devoured, (5_000, false), "nothing devoured, nothing healed");
}
