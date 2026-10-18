//! Phase 3's fifth round of combat tests, the damage a blow does (Thistle
//! Tea's `target_damage_test.exs` and `weapon_bonuses_test.exs`): flat
//! bonuses against creature types, by their stacks, and a hand's
//! percentages over every flat bonus.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::combat::{self, Hand};
use crate::game::map::inventory::{SLOT_MAIN_HAND, SLOT_OFF_HAND};
use crate::game::tests_p3_combat::{self as base, *};

const ELEMENTAL: u32 = 3720;
const UNDEAD: u32 = 3721;
const DAGGER: u32 = 63_531;

fn data() -> GameData {
    let mut d = base::data();
    d.creature_templates
        .insert(ELEMENTAL, template(ELEMENTAL, 4));
    d.creature_templates.insert(UNDEAD, template(UNDEAD, 6));
    d.items.insert(
        DAGGER,
        crate::game::data::items::ItemTemplate {
            entry: DAGGER,
            class: 2,
            subclass: 15,
            name: "Dagger".into(),
            display_id: 1000,
            inventory_type: 13,
            delay: 1500,
            stackable: 1,
            ..Default::default()
        },
    );
    d
}

fn stacked(m: &mut Map, g: Guid, id: u32, kind: u32, misc: i32, amount: i32, stacks: u32) {
    aura_of(m, g, id, kind, misc, amount);
    if let Some(h) =
        auras::auras_mut(m, g).and_then(|a| a.holders.iter_mut().find(|h| h.spell == id))
    {
        h.stacks = stacks;
    }
}

/// `g`'s blow with `hand` on `victim`, its weapon's damage a flat hundred
/// and its speed `speed`.
fn blow(m: &mut Map, g: Guid, victim: Guid, hand: Hand, speed: u32) -> u32 {
    let f = m.unit_fields_mut(g).unwrap();
    for (lo, hi) in [
        (UNIT_FIELD_MINDAMAGE, UNIT_FIELD_MAXDAMAGE),
        (UNIT_FIELD_MINOFFHANDDAMAGE, UNIT_FIELD_MAXOFFHANDDAMAGE),
    ] {
        f.set_f32(lo, 100.0);
        f.set_f32(hi, 100.0);
    }
    f.set_u32(UNIT_FIELD_BASEATTACKTIME, speed);
    f.set_u32(UNIT_FIELD_BASEATTACKTIME + 1, speed);
    combat::weapon_damage(m, g, victim, hand, &mut rand::rng()).0
}

/// Thistle Tea's "combines equipment and stacked auras by live creature
/// type" and "adds flat damage before armor and crits without weapon speed
/// scaling": each aura of damage done to creature types counts its stacks
/// and the types its mask names, nothing for another type, and no more for
/// a slower weapon.
#[tokio::test]
async fn damage_done_to_creature_types_counts_its_stacks_and_its_types_alone() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            // Six for beasts and elementals, twice over; two more for
            // beasts, four for elementals.
            stacked(m, ga, 90_850, AURA_MOD_DAMAGE_DONE_CREATURE, 1 | 8, 6, 2);
            stacked(m, ga, 90_851, AURA_MOD_DAMAGE_DONE_CREATURE, 1, 2, 1);
            stacked(m, ga, 90_852, AURA_MOD_DAMAGE_DONE_CREATURE, 8, 4, 1);
            let beast = spawn(m, ga, BEAST, 60, 3.0);
            let elemental = spawn(m, ga, ELEMENTAL, 60, 3.0);
            let undead = spawn(m, ga, UNDEAD, 60, 3.0);
            let humanoid = spawn(m, ga, BRUTE, 60, 3.0);
            [
                blow(m, ga, beast, Hand::Main, 2000),
                blow(m, ga, elemental, Hand::Main, 2000),
                blow(m, ga, undead, Hand::Main, 2000),
                blow(m, ga, humanoid, Hand::Main, 2000),
                blow(m, ga, beast, Hand::Main, 1000),
                blow(m, ga, beast, Hand::Main, 4000),
            ]
        })
        .await
        .unwrap();
    assert_eq!(r, [114, 116, 100, 100, 114, 114]);
}

/// Thistle Tea's "scales the added bonus with hand and outgoing damage
/// modifiers" and "offhand penalties also scale flat damage bonuses", as
/// mangos's `MeleeDamageBonusDone` and its `TOTAL_VALUE` under the hand's
/// `TOTAL_PCT` have them: a flat bonus, to a creature type or to all
/// damage, takes the percentages the weapon's damage took (double damage
/// done, here), and in the off hand its half and the off hand's bonus too;
/// the percentage against the type goes over it all.
#[tokio::test]
async fn a_flat_bonus_takes_the_hands_percentages_and_the_off_hands_half() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            m.players
                .get_mut(&ga)
                .unwrap()
                .spells
                .push(crate::game::map::derive::SPELL_DUAL_WIELD);
            equip(m, ga, SWORD, SLOT_MAIN_HAND);
            equip(m, ga, SWORD, SLOT_OFF_HAND);
            let beast = spawn(m, ga, BEAST, 60, 3.0);
            let humanoid = spawn(m, ga, BRUTE, 60, 3.0);
            stacked(m, ga, 90_853, AURA_MOD_DAMAGE_DONE_CREATURE, 1, 20, 1);
            stacked(m, ga, 90_854, AURA_MOD_DAMAGE_PERCENT_DONE, 1, 100, 1);
            stacked(m, ga, 90_855, AURA_MOD_OFFHAND_DAMAGE_PCT, 0, 50, 1);
            stacked(m, ga, 90_856, AURA_MOD_DAMAGE_DONE_VERSUS, 1, 10, 1);
            let creature = (
                blow(m, ga, beast, Hand::Main, 2000),
                blow(m, ga, beast, Hand::Off, 2000),
            );
            auras::remove_spell(m, ga, 90_853, None);
            auras::remove_spell(m, ga, 90_856, None);
            // Ten more damage done, for all.
            stacked(m, ga, 90_857, AURA_MOD_DAMAGE_DONE, 1, 10, 1);
            let all = (
                blow(m, ga, humanoid, Hand::Main, 2000),
                blow(m, ga, humanoid, Hand::Off, 2000),
            );
            (creature, all)
        })
        .await
        .unwrap();
    assert_eq!(
        r.0,
        (154, 143),
        "(100 + 20 x 2) x 1.1; (100 + 20 x 2 x 0.75) x 1.1"
    );
    assert_eq!(r.1, (120, 115), "100 + 10 x 2; 100 + 10 x 2 x 0.75");
}

/// Thistle Tea's "weapon restrictions filter flat and percentage damage
/// for each hand" and "offhand penalties also scale flat damage bonuses":
/// four more damage done with swords counts for the sword's hand alone,
/// and four with daggers for the dagger's off hand, at its half.
#[tokio::test]
async fn a_flat_bonus_held_to_a_weapon_counts_for_its_hand_alone() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            m.players
                .get_mut(&ga)
                .unwrap()
                .spells
                .push(crate::game::map::derive::SPELL_DUAL_WIELD);
            equip(m, ga, SWORD, SLOT_MAIN_HAND);
            equip(m, ga, DAGGER, SLOT_OFF_HAND);
            let humanoid = spawn(m, ga, BRUTE, 60, 3.0);
            let held = |m: &mut Map, id: u32, subclass: u32| {
                let mut effects = [crate::game::data::spells::SpellEffect::default(); 3];
                effects[0] = crate::game::data::spells::SpellEffect {
                    effect: 6,
                    aura: AURA_MOD_DAMAGE_DONE,
                    misc_value: 1,
                    base_points: 3,
                    base_dice: 1,
                    target_a: 1,
                    ..Default::default()
                };
                let s = crate::game::data::spells::SpellTemplate {
                    id,
                    name: format!("Talent {id}"),
                    equipped_item_class: 2,
                    equipped_item_subclass_mask: 1 << subclass,
                    effects,
                    ..Default::default()
                };
                let _ = auras::apply_spell(m, ga, ga, &s, &[(0, 4)], auras::Context::default());
            };
            held(m, 90_858, 7);
            let swords = (
                blow(m, ga, humanoid, Hand::Main, 2000),
                blow(m, ga, humanoid, Hand::Off, 2000),
            );
            auras::remove_spell(m, ga, 90_858, None);
            held(m, 90_859, 15);
            let daggers = (
                blow(m, ga, humanoid, Hand::Main, 2000),
                blow(m, ga, humanoid, Hand::Off, 2000),
            );
            (swords, daggers)
        })
        .await
        .unwrap();
    assert_eq!(r.0, (104, 100), "the sword's");
    assert_eq!(r.1, (100, 102), "the dagger's, halved");
}
