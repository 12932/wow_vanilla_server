//! Phase 3's fifth round of combat tests, damage a target takes (Thistle
//! Tea's `damage_received_test.exs` and `attack_damage_taken_test.exs`): a
//! spell's or an ability's flats and percentages, in mangos's order, and
//! its crit before armor.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::combat;
use crate::game::map::spells::outcome::{self, Kind, Landing};
use crate::game::tests_p3_combat::*;

/// A damage spell of damage class `class` and school `school`, its
/// coefficient `coefficient`.
fn spell(id: u32, class: u32, school: u32, coefficient: f32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        range_index: 1,
        equipped_item_class: -1,
        dmg_class: class,
        school,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 2,
        base_points: 99,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        bonus_coefficient: coefficient,
        ..Default::default()
    };
    s
}

fn clear(m: &mut Map, g: Guid) {
    if let Some(a) = auras::auras_mut(m, g) {
        a.holders.retain(|h| !(90_870..90_890).contains(&h.spell));
    }
}

/// Thistle Tea's "weapon ticks scale both school and attack flats before
/// independent percentages", "magic reductions cap at half while attack
/// reductions may suppress the whole hit", "flat bonuses obey school masks"
/// and "zero weapon damage cannot acquire a flat damage bonus", as mangos's
/// `MeleeDamageBonusTaken` and `SpellDamageBonusTaken` have them: an
/// ability adds the flat damage taken for its school and for its kind of
/// attack, then takes every percentage; a spell its school's flat by its
/// coefficient; a flat cut takes no more than half of a spell, all of an
/// ability; a flat for another school counts for nothing; no damage gains
/// none.
#[tokio::test]
async fn an_ability_adds_every_flat_before_every_percentage_and_a_spell_loses_at_most_half() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            let taken = |m: &Map, s: &SpellTemplate, damage: f32| {
                outcome::taken_of(m, ga, brute, s, 0, Kind::Direct, 1, damage) as u32
            };
            let (fire, melee, ranged) = (
                spell(90_890, 1, 2, 1.0),
                spell(90_891, 2, 0, -1.0),
                spell(90_892, 3, 0, -1.0),
            );
            aura_of(m, brute, 90_870, AURA_MOD_DAMAGE_TAKEN, 127, 20);
            aura_of(m, brute, 90_871, AURA_MOD_MELEE_DAMAGE_TAKEN, 0, 30);
            aura_of(m, brute, 90_872, AURA_MOD_RANGED_DAMAGE_TAKEN, 0, 50);
            aura_of(m, brute, 90_873, AURA_MOD_MELEE_DAMAGE_TAKEN_PCT, 0, -50);
            aura_of(m, brute, 90_874, AURA_MOD_DAMAGE_PERCENT_TAKEN, 127, 100);
            let order = [
                taken(m, &fire, 100.0),
                taken(m, &melee, 100.0),
                taken(m, &ranged, 100.0),
                taken(m, &melee, 0.0),
            ];
            clear(m, brute);
            aura_of(m, brute, 90_870, AURA_MOD_DAMAGE_TAKEN, 127, -1000);
            let cut = [
                taken(m, &fire, 100.0),
                taken(m, &melee, 100.0),
                taken(m, &ranged, 100.0),
            ];
            clear(m, brute);
            // Forty more fire taken, half again of fire.
            aura_of(m, brute, 90_870, AURA_MOD_DAMAGE_TAKEN, 4, 40);
            aura_of(m, brute, 90_874, AURA_MOD_DAMAGE_PERCENT_TAKEN, 4, 50);
            let half = spell(90_893, 1, 2, 0.5);
            let frost = spell(90_894, 1, 4, 0.5);
            let schools = [taken(m, &half, 100.0), taken(m, &frost, 100.0)];
            (order, cut, schools)
        })
        .await
        .unwrap();
    assert_eq!(
        r.0,
        [240, 150, 340, 0],
        "(100 + 20) x 2; (100 + 20 + 30) x 2 x 0.5; (100 + 20 + 50) x 2; none"
    );
    assert_eq!(
        r.1,
        [50, 0, 0],
        "a spell loses half at most, an ability all"
    );
    assert_eq!(
        r.2,
        [180, 100],
        "(100 + 40 x 0.5) x 1.5; frost takes neither"
    );
}

/// Thistle Tea's "physical spells apply target bonuses and critical damage
/// before armor", as mangos's `CalculateSpellDamage` has it: an ability's
/// damage takes the target's modifiers, then doubles for its crit, then
/// armor takes its share of all of it.
#[tokio::test]
async fn an_abilitys_crit_comes_before_armor() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            aura_of(m, brute, 90_870, AURA_MOD_DAMAGE_TAKEN, 1, 40);
            aura_of(m, brute, 90_874, AURA_MOD_DAMAGE_PERCENT_TAKEN, 1, 50);
            m.creature_mut(brute)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_RESISTANCES, 3000);
            let s = spell(90_891, 2, 0, -1.0);
            let crit = Landing {
                crit: true,
                ..Default::default()
            };
            let d = outcome::damage(m, ga, brute, &s, 0, 100.0, &crit);
            (d.amount, combat::armor_reduction(3000, 60))
        })
        .await
        .unwrap();
    let (amount, armor) = r;
    // (100 + 40) x 1.5 = 210, doubled: 420, less armor's share.
    let expected = 420.0 - (420.0 * armor).trunc();
    assert_eq!(amount, expected as u32);
}
