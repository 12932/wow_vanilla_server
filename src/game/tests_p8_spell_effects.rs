//! Round 8, against Thistle Tea's `spell_effect_test.exs` and
//! `casting_test.exs`: a spell's weapon effects as one blow, and a corpse
//! captured (mangos-classic's `EffectWeaponDmg`; vmangos's scripts of the
//! capture spells).

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh};
use crate::game::data::GameData;
use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{ATTR_EX2_ALLOW_DEAD_TARGET, SpellEffect};
use crate::game::fields::*;
use crate::game::map::creature::Life;
use crate::game::map::spells;

/// Half again the weapon's damage and 15 more, normalized; Capture
/// Treant's kind (a dummy at a corpse and an item for its caster), and the
/// same that may not be aimed at the dead.
const STRIKE: u32 = 998_601;
const CAPTURE: u32 = 11_885;
const CAPTURE_LIVING: u32 = 998_602;
const TREANT_SEED: u32 = 9_593;

fn data(d: &mut GameData) {
    let mut strike = bare(STRIKE, 0);
    effect(
        &mut strike,
        0,
        SpellEffect {
            effect: spells::EFFECT_WEAPON_PERCENT_DAMAGE,
            base_points: 149,
            base_dice: 1,
            target_a: 6,
            ..Default::default()
        },
    );
    effect(
        &mut strike,
        1,
        SpellEffect {
            effect: spells::EFFECT_NORMALIZED_WEAPON_DMG,
            base_points: 14,
            base_dice: 1,
            target_a: 6,
            ..Default::default()
        },
    );
    let mut capture = bare(CAPTURE, 0);
    capture.attributes[2] |= ATTR_EX2_ALLOW_DEAD_TARGET;
    effect(
        &mut capture,
        0,
        SpellEffect {
            effect: spells::EFFECT_DUMMY,
            target_a: 6,
            ..Default::default()
        },
    );
    effect(
        &mut capture,
        1,
        SpellEffect {
            effect: spells::EFFECT_CREATE_ITEM,
            base_points: 0,
            base_dice: 1,
            item_type: u64::from(TREANT_SEED),
            target_a: 1,
            ..Default::default()
        },
    );
    let mut living = capture.clone();
    living.id = CAPTURE_LIVING;
    living.attributes[2] &= !ATTR_EX2_ALLOW_DEAD_TARGET;
    insert(d, [strike, capture, living]);
    d.items.insert(
        TREANT_SEED,
        ItemTemplate {
            entry: TREANT_SEED,
            class: 12,
            name: "Treant Seed".into(),
            display_id: 1000,
            stackable: 20,
            ..Default::default()
        },
    );
}

/// A spell's weapon effects are one blow: the weapon's 100 and the flat 15
/// together, times its half again, 172 (Thistle Tea's "multiple
/// weapon-damage effects fold into a single strike").
#[tokio::test]
async fn a_spells_weapon_effects_are_one_blow() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P8EF1", "Striker", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[STRIKE]);
            let kobold = kit::dummy(m, a, 60, 3.0);
            if let Some(f) = m.unit_fields_mut(a) {
                f.set_f32(UNIT_FIELD_MINDAMAGE, 100.0);
                f.set_f32(UNIT_FIELD_MAXDAMAGE, 100.0);
            }
            cast_fresh(m, a, STRIKE, at(kobold));
            1_000_000 - kit::health(m, kobold)
        })
        .await
        .unwrap();
    assert_eq!(out, 172);
}

/// A capture spell at a corpse takes the corpse away in a second, neither
/// raised nor hurt, and gives its caster the item, who stays; one that may
/// not be aimed at the dead does nothing there (Thistle Tea's "corpse
/// capture schedules removal without reviving or damaging the corpse" and
/// "delivers corpse capture separately from its caster reward").
#[tokio::test]
async fn a_captured_corpse_goes_and_its_captor_has_the_item() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P8EF2", "Captor", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[CAPTURE, CAPTURE_LIVING]);
            let corpse = kit::dummy(m, a, 60, 3.0);
            crate::game::map::combat::deal_damage(m, a, corpse, 2_000_000);
            let went = |m: &crate::game::map::Map| match m.creature(corpse).map(|c| &c.life) {
                Some(Life::Dead { corpse_until, .. }) => {
                    corpse_until.saturating_duration_since(m.now).as_millis()
                }
                _ => u128::MAX,
            };
            let before = went(m);
            let refused = cast_fresh(m, a, CAPTURE_LIVING, at(corpse));
            let unrefused = went(m);
            let cast = cast_fresh(m, a, CAPTURE, at(corpse));
            let seeds = m.players[&a].count_of(TREANT_SEED);
            (
                before > 1_000,
                refused,
                unrefused == before,
                cast,
                went(m),
                kit::health(m, corpse),
                seeds,
                m.players.contains_key(&a),
            )
        })
        .await
        .unwrap();
    assert_eq!(out, (true, false, true, true, 1_000, 0, 1, true));
}
