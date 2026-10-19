//! Round 7, against Thistle Tea's `scripts_test.exs` and
//! `semantics_test.exs`: spells vmangos names a script for, on the real
//! data.

use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, has, power};
use crate::game::map::auras::{self, AURA_MOD_STEALTH};
use crate::game::map::spells::{self, class_scripts};

const FIREBALL: u32 = 133;
const FROSTBOLT: u32 = 116;
const EXECUTE: u32 = 5308;
const WOLFSHEAD_ENERGY: u32 = 17770;
const CAT_FORM: u32 = 768;
const SHADOWMELD: u32 = 20580;
const SHADOWMELD_PASSIVE: u32 = 21009;

/// Fireball and Frostbolt leave nothing beside their own aura and cast
/// nothing as they finish; Wolfshead Helm's energy script's finish is its
/// energy (29940) (Thistle Tea's "unscripted spells trigger nothing" and
/// "Wolfshead Helm's cat form trigger grants its energy"); in cat form
/// its cast gives a druid twenty energy.
#[tokio::test]
#[ignore = "extended"]
async fn unscripted_spells_trigger_nothing_and_wolfsheads_gives_energy() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (_c, a, map) = kit::player(&realm, "P7SS1", "Wolfhead", Race::NightElf, Class::Druid).await;
    let out = map
        .query(move |m| {
            let data = m.realm.data.clone();
            let get = |id| data.spells.get(id).unwrap();
            let plain = [FIREBALL, FROSTBOLT].map(|id| {
                (
                    class_scripts::apply_trigger(get(id)),
                    class_scripts::finish_trigger(get(id)),
                )
            });
            let wolfshead = class_scripts::finish_trigger(get(WOLFSHEAD_ENERGY));
            kit::ready(m, a, 60, &[CAT_FORM]);
            spells::cast_triggered(m, a, CAT_FORM, Some(a));
            kit::set_power(m, a, 3, 0);
            spells::cast_triggered(m, a, WOLFSHEAD_ENERGY, Some(a));
            (plain, wolfshead, auras::form(m, a), power(m, a, 3))
        })
        .await
        .unwrap();
    let (plain, wolfshead, form, energy) = out;
    assert_eq!(plain, [(None, None), (None, None)]);
    assert_eq!(wolfshead, Some(29_940));
    assert_eq!(form, auras::FORM_CAT);
    assert_eq!(energy, 20, "Wolfshead Helm's energy");
}

/// Shadowmeld takes its passive companion's stealth level (five) as a
/// spare effect: cast, it stealths its night elf, five levels the more (Thistle Tea's "Shadowmeld
/// raises stealth level through its passive companion").
#[tokio::test]
#[ignore = "extended"]
async fn shadowmeld_stealths_by_its_passive_companion() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (_c, a, map) = kit::player(&realm, "P7SS2", "Meldy", Race::NightElf, Class::Warrior).await;
    let out = map
        .query(move |m| {
            let data = m.realm.data.clone();
            let passive = data
                .spells
                .get(SHADOWMELD_PASSIVE)
                .and_then(|p| p.effects.iter().find(|e| e.effect != 0).copied())
                .unwrap();
            let carried = data
                .spells
                .get(SHADOWMELD)
                .is_some_and(|s| s.effects.iter().any(|e| e.aura == passive.aura));
            kit::ready(m, a, 60, &[SHADOWMELD]);
            cast_fresh(m, a, SHADOWMELD, Default::default());
            let stealth = auras::auras(m, a).is_some_and(|x| x.has(AURA_MOD_STEALTH));
            // Its stealth level, the passive's (`SPELL_AURA_MOD_STEALTH_LEVEL`).
            let level = kit::holder(m, a, SHADOWMELD).and_then(|h| {
                h.effects
                    .iter()
                    .find(|x| x.kind == passive.aura)
                    .map(|x| x.amount)
            });
            (carried, has(m, a, SHADOWMELD), stealth, level, passive.aura)
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true, true, Some(5), 154));
}

/// Execute, by its vmangos script (`spell_warrior_execute_dummy`), is
/// refused above a fifth of its target's health and strikes below it
/// (Thistle Tea's "compiles VMangos script names into typed spell
/// rules").
#[tokio::test]
#[ignore = "extended"]
async fn execute_strikes_by_its_script_below_a_fifth() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (_c, a, map) = kit::player(&realm, "P7SS3", "Executor", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[EXECUTE]);
            let kobold = kit::dummy(m, a, 60, 2.0);
            let high = cast_fresh(m, a, EXECUTE, at(kobold));
            kit::set_health(m, kobold, 100_000);
            kit::set_power(m, a, 1, 300);
            cast_fresh(m, a, EXECUTE, at(kobold));
            (high, 100_000 - kit::health(m, kobold))
        })
        .await
        .unwrap();
    let (high, dealt) = out;
    assert!(!high, "not above a fifth");
    assert!(dealt > 0, "it struck: {dealt}");
}
