//! Phase 4's gear, continued: Thistle Tea's `ProficiencyTest` ported, what
//! a player's spells and skills let them wear (the proficiency spells'
//! subclasses, Dual Wield, a fishing pole with Fishing, an item's required
//! skill, rank and spell) and parrying and blocking from what is learned.

use wow_world_messages::vanilla::*;

use super::tests_p4_items_gear::{MAIN, OFF, SHIELD, SWORD, data, item, on_map, passive, wear};
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::inventory::{self, Proficiency};
use crate::game::map::{Map, derive};

/// Proficiency spells: Staves, Wands, Cloth, Dual Wield, a second Parry
/// (as 3124 and 18848 are beside 3127), and a specialization.
const STAVES: u32 = 227;
const WANDS: u32 = 5009;
const CLOTH: u32 = 9078;
const DUAL_WIELD: u32 = 674;
const OTHER_PARRY: u32 = 3124;
const SPECIALIZATION: u32 = 9788;
const FISHING: u32 = 356;
const STAFF_BITS: u32 = 1 << 10;
const WAND_BITS: u32 = 1 << 19;
const CLOTH_BITS: u32 = 1 << 1;

fn proficiency_spell(id: u32, class: i32, mask: u32) -> SpellTemplate {
    let mut s = passive(id, &[]);
    s.effects[0] = SpellEffect {
        effect: crate::game::map::spells::EFFECT_PROFICIENCY,
        target_a: 1,
        ..Default::default()
    };
    s.equipped_item_class = class;
    s.equipped_item_subclass_mask = mask as i32;
    s
}

/// The gear's world with the proficiency spells, so a player's come from
/// what they know.
fn proficiency_data() -> GameData {
    let mut d = data();
    for s in [
        proficiency_spell(STAVES, 2, STAFF_BITS),
        proficiency_spell(WANDS, 2, WAND_BITS),
        proficiency_spell(CLOTH, 4, CLOTH_BITS),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    let mut dual = passive(DUAL_WIELD, &[]);
    dual.effects[0] = SpellEffect {
        effect: 40,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(DUAL_WIELD, dual);
    let mut parry = passive(OTHER_PARRY, &[]);
    parry.effects[0] = SpellEffect {
        effect: 22,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(OTHER_PARRY, parry);
    d.spells
        .spells
        .insert(SPECIALIZATION, passive(SPECIALIZATION, &[]));
    d
}

/// Thistle Tea's `from_spellbook/1`: each proficiency spell's subclasses
/// of its item class add up, weapons apart from armor; Dual Wield comes
/// of its effect, not without it.
#[test]
fn proficiency_spells_add_up_their_subclasses_and_dual_wield_its_effect() {
    let d = proficiency_data();
    let prof = Proficiency::from_spells(&d, &[STAVES, WANDS, CLOTH]);
    assert_eq!(prof.weapons, STAFF_BITS | WAND_BITS);
    assert_eq!(prof.armor, CLOTH_BITS);
    assert!(!prof.dual_wield);
    assert!(Proficiency::from_spells(&d, &[DUAL_WIELD]).dual_wield);
    // A staff (10) is covered, a dagger (15) not; a miscellaneous weapon
    // (14) and armor (0) want no proficiency.
    let staff = item(1, 2, 10, 17);
    let dagger = item(2, 2, 15, 13);
    assert!(prof.covers(&staff));
    assert!(!prof.covers(&dagger));
    let none = Proficiency::from_spells(&d, &[]);
    assert!(none.covers(&item(3, 2, 14, 13)));
    assert!(none.covers(&item(4, 4, 0, 11)));
    assert!(!none.covers(&staff));
}

/// Thistle Tea's skill-derived proficiency: Fishing known lets a fishing
/// pole (weapon subclass 20) be held, beside the spells' weapons; without
/// it, a pole isn't.
#[tokio::test]
async fn fishing_known_lets_a_pole_be_held_beside_the_spells_weapons() {
    let realm = harness::realm_with(proficiency_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let pole = item(5, 2, 20, 17);
        let p = m.players.get_mut(&g).unwrap();
        p.spells = vec![STAVES, CLOTH];
        let without = Proficiency::of(p, &d);
        let refused = inventory::proficient(p, &without, &pole);
        crate::game::map::skills::learn_skill(p, FISHING, 1);
        let with = Proficiency::of(p, &d);
        (
            without.weapons,
            refused,
            with.weapons,
            inventory::proficient(p, &with, &pole),
        )
    })
    .await;
    assert_eq!(got.0, STAFF_BITS);
    assert_eq!(got.1, Err(inventory::Fail::NoProficiency));
    assert_eq!(got.2, STAFF_BITS | 1 << 20);
    assert_eq!(got.3, Ok(()));
}

/// Thistle Tea's required skills and spells: a recipe asking for Tailoring
/// at rank 0 wants Tailoring still; a pole asking for Fishing at 50 wants
/// 50 (short of it, mangos's "not skilled enough" where Thistle Tea says
/// no proficiency); an item asking for a specialization wants it, armor or
/// recipe; a world without proficiency spells lets anything be worn.
#[tokio::test]
async fn an_items_required_skill_rank_and_spell_are_asked_for() {
    let realm = harness::realm_with(proficiency_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        use inventory::Fail;
        let d = m.realm.data.clone();
        let recipe = ItemTemplate {
            required_skill: 164,
            required_skill_rank: 0,
            ..item(6, 9, 0, 0)
        };
        let pole = ItemTemplate {
            required_skill: FISHING,
            required_skill_rank: 50,
            ..item(7, 2, 20, 17)
        };
        let special: Vec<ItemTemplate> = [4, 9]
            .into_iter()
            .map(|class| ItemTemplate {
                required_spell: SPECIALIZATION,
                ..item(8, class, 0, 0)
            })
            .collect();
        let p = m.players.get_mut(&g).unwrap();
        p.spells = vec![CLOTH];
        let mut out: Vec<(&str, Result<(), Fail>)> = Vec::new();
        let check = |p: &crate::game::map::player::Player, t: &ItemTemplate| {
            inventory::proficient(p, &Proficiency::of(p, &d), t)
        };
        out.push(("recipe unskilled", check(p, &recipe)));
        crate::game::map::skills::learn_skill(p, 164, 1);
        crate::game::map::skills::set_value(p, 164, 1);
        out.push(("recipe skilled", check(p, &recipe)));
        crate::game::map::skills::learn_skill(p, FISHING, 1);
        crate::game::map::skills::set_value(p, FISHING, 49);
        out.push(("pole at 49", check(p, &pole)));
        crate::game::map::skills::set_value(p, FISHING, 50);
        out.push(("pole at 50", check(p, &pole)));
        for t in &special {
            out.push(("unspecialized", check(p, t)));
        }
        p.spells.push(SPECIALIZATION);
        for t in &special {
            out.push(("specialized", check(p, t)));
        }
        p.spells.retain(|s| *s != SPECIALIZATION);
        for t in special.iter().chain([&recipe, &pole]) {
            out.push((
                "anything goes",
                inventory::proficient(p, &Proficiency::ALL, t),
            ));
        }
        out
    })
    .await;
    use inventory::Fail;
    assert_eq!(
        got,
        vec![
            ("recipe unskilled", Err(Fail::NoProficiency)),
            ("recipe skilled", Ok(())),
            ("pole at 49", Err(Fail::Skill)),
            ("pole at 50", Ok(())),
            ("unspecialized", Err(Fail::NoProficiency)),
            ("unspecialized", Err(Fail::NoProficiency)),
            ("specialized", Ok(())),
            ("specialized", Ok(())),
            ("anything goes", Ok(())),
            ("anything goes", Ok(())),
            ("anything goes", Ok(())),
            ("anything goes", Ok(())),
        ]
    );
}

/// Thistle Tea's defensive capabilities: parrying comes of any Parry
/// learned (mangos's `EffectParry` for every spell of the effect), blocking
/// of Block; neither without.
#[tokio::test]
async fn parry_and_block_come_of_what_is_learned() {
    let realm = harness::realm_with(proficiency_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().items.clear();
        wear(m, g, SWORD, MAIN);
        wear(m, g, SHIELD, OFF);
        let read = |m: &mut Map, spells: &[u32]| {
            let p = m.players.get_mut(&g).unwrap();
            p.spells = spells.to_vec();
            derive::unit(m, g);
            let f = &m.players[&g].fields;
            (
                f.get_f32(PLAYER_PARRY_PERCENTAGE) > 0.0,
                f.get_f32(PLAYER_BLOCK_PERCENTAGE) > 0.0,
            )
        };
        (
            read(m, &[CLOTH, derive::SPELL_PARRY, derive::SPELL_BLOCK]),
            read(m, &[CLOTH, OTHER_PARRY]),
            read(m, &[CLOTH]),
        )
    })
    .await;
    assert_eq!(got, ((true, true), (true, false), (false, false)));
}
