//! Round 7's aura tests, ported from Thistle Tea's
//! `test/game/core/entity/appearance_test.exs` (docs/parity/matrix.csv):
//! a transform's size, reach and weapons, and what comes back after it.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::DisplayInfo;
use crate::game::data::spells::{CUSTOM_POSITIVE, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras;
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn, template};
use crate::game::tests_p3_entity::on;

/// Transforms into creatures 3_901 (display 100, at 0.8 of its size,
/// radius 0.5 and reach 2.0) and 3_902 (display 200), a growth by half,
/// and Ghost Wolf.
const INTO_SMALL: u32 = 97_001;
const INTO_PLAIN: u32 = 97_002;
const GROWTH: u32 = 97_010;
const GHOST_WOLF: u32 = 2645;
const SMALL: u32 = 3_901;
const PLAIN: u32 = 3_902;
/// The weapons: a sword of display 500, one of 600, and the gear the
/// small creature and the plain one hold (the plain one's empty hands).
const OWN_SWORD: u32 = 97_500;
const SMALL_SWORD: u32 = 97_600;
const SMALL_GEAR: u32 = 9_701;
const NO_GEAR: u32 = 9_702;

fn aura_spell(id: u32, aura: u32, misc: i32, points: i32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        // Helpful, as Thistle Tea's disguises: a player may cancel them.
        custom_flags: CUSTOM_POSITIVE,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura,
        misc_value: misc,
        base_points: points - 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    s
}

fn data() -> crate::game::data::GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let mut small = template(SMALL, 7);
    small.display_ids = [100, 0, 0, 0];
    small.display_scales = [0.8, 0.0, 0.0, 0.0];
    small.equipment_id = SMALL_GEAR;
    d.creature_templates.insert(SMALL, small);
    let mut plain = template(PLAIN, 7);
    plain.display_ids = [200, 0, 0, 0];
    plain.equipment_id = NO_GEAR;
    d.creature_templates.insert(PLAIN, plain);
    d.display_info.insert(
        100,
        DisplayInfo {
            scale: 1.0,
            bounding_radius: 0.5,
            combat_reach: 2.0,
            ..Default::default()
        },
    );
    d.display_info.insert(
        200,
        DisplayInfo {
            scale: 1.0,
            bounding_radius: 0.389,
            combat_reach: 1.5,
            ..Default::default()
        },
    );
    for (entry, display) in [(OWN_SWORD, 500), (SMALL_SWORD, 600)] {
        let mut sword = d.items[&crate::game::tests_p3_combat::SWORD].clone();
        sword.entry = entry;
        sword.display_id = display;
        d.items.insert(entry, sword);
    }
    d.creature_equipment.insert(SMALL_GEAR, [SMALL_SWORD, 0, 0]);
    d.creature_equipment.insert(NO_GEAR, [0, 0, 0]);
    for s in [
        aura_spell(INTO_SMALL, auras::AURA_TRANSFORM, SMALL as i32, 0),
        aura_spell(INTO_PLAIN, auras::AURA_TRANSFORM, PLAIN as i32, 0),
        aura_spell(GROWTH, auras::AURA_MOD_SCALE, 0, 50),
        aura_spell(
            GHOST_WOLF,
            auras::AURA_MOD_SHAPESHIFT,
            auras::FORM_GHOST_WOLF as i32,
            0,
        ),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn put(m: &mut Map, g: Guid, id: u32) {
    let s = m.realm.data.spells.spells[&id].clone();
    let points = crate::game::map::spells::points(&s, 0, 1);
    auras::apply_spell(m, g, g, &s, &[(0, points)], auras::Context::default()).expect("put on");
}

fn take(m: &mut Map, g: Guid, id: u32) {
    auras::remove_where(m, g, |h| h.spell == id);
}

/// Its display, size, bounding radius and combat reach.
fn looks(m: &Map, g: Guid) -> (u32, f32, f32, f32) {
    let f = m.unit_fields(g).expect("the unit");
    (
        f.get_u32(UNIT_FIELD_DISPLAYID),
        f.get_f32(OBJECT_FIELD_SCALE_X),
        f.get_f32(UNIT_FIELD_BOUNDINGRADIUS),
        f.get_f32(UNIT_FIELD_COMBATREACH),
    )
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

/// Thistle Tea's `Appearance` "scale and geometry are derived without
/// compounding and restore on removal": a player transformed into a
/// creature takes its size (0.8) and its model's radius and reach at that
/// size (0.4 and 1.6), a growth by half grows all three (1.2, 0.6, 2.4),
/// putting both on again changes nothing, and taking them off however
/// (removed, cancelled, by death) gives back the player's own.
#[tokio::test]
async fn a_transform_takes_its_creatures_size_and_reach_and_gives_them_back() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let own = looks(m, ga);
        let mut seen = Vec::new();
        for how in 0..3 {
            put(m, ga, INTO_SMALL);
            let small = looks(m, ga);
            put(m, ga, GROWTH);
            let grown = looks(m, ga);
            put(m, ga, INTO_SMALL);
            put(m, ga, GROWTH);
            let again = looks(m, ga);
            match how {
                0 => {
                    take(m, ga, GROWTH);
                    take(m, ga, INTO_SMALL);
                }
                1 => {
                    auras::cancel(m, ga, INTO_SMALL);
                    auras::cancel(m, ga, GROWTH);
                }
                _ => auras::clear_on_death(m, ga),
            }
            seen.push((small, grown, again, looks(m, ga)));
        }
        (own, seen)
    })
    .await;
    let (own, seen) = out;
    assert!(
        near(own.1, 1.0) && near(own.2, 0.389) && near(own.3, 1.5),
        "{own:?}"
    );
    for (how, (small, grown, again, back)) in seen.into_iter().enumerate() {
        assert_eq!(small.0, 100);
        assert!(
            near(small.1, 0.8) && near(small.2, 0.4) && near(small.3, 1.6),
            "{small:?}"
        );
        assert!(
            near(grown.1, 1.2) && near(grown.2, 0.6) && near(grown.3, 2.4),
            "{grown:?}"
        );
        assert_eq!(again, grown, "no compounding");
        assert_eq!(back, own, "taken off by {how}");
    }
}

/// Thistle Tea's `Appearance` "removing only the transform preserves
/// active growth": the transform gone, the growth stays on the player's
/// own size and reach (1.5 and 2.25), in its own look.
#[tokio::test]
async fn a_growth_outlasts_the_transform_it_was_put_on_with() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let (own, after, back) = on(&map, move |m| {
        let own = looks(m, ga);
        put(m, ga, INTO_SMALL);
        put(m, ga, GROWTH);
        take(m, ga, INTO_SMALL);
        let after = looks(m, ga);
        take(m, ga, GROWTH);
        (own, after, looks(m, ga))
    })
    .await;
    assert_eq!(after.0, own.0, "its own look");
    assert!(near(after.1, 1.5) && near(after.3, 2.25), "{after:?}");
    assert!(near(after.2, 0.389 * 1.5), "{after:?}");
    assert_eq!(back, own);
}

/// Thistle Tea's `Appearance` "removing a transform restores the
/// shapeshift model and scale": a Ghost Wolf transformed shows the
/// transform, and the wolf again after it, at the size it had; out of the
/// form, the player's own. (The wolf keeps the player's size, as mangos's
/// `RestoreDisplayId` keeps a form's at the unit's own, where Thistle Tea
/// shrinks it to 0.8.)
#[tokio::test]
async fn a_transform_gone_shows_the_form_again_at_its_size() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let (own, wolf, plain, again, back) = on(&map, move |m| {
        let own = looks(m, ga);
        put(m, ga, GHOST_WOLF);
        let wolf = looks(m, ga);
        put(m, ga, INTO_PLAIN);
        let plain = looks(m, ga);
        take(m, ga, INTO_PLAIN);
        let again = looks(m, ga);
        take(m, ga, GHOST_WOLF);
        (own, wolf, plain, again, looks(m, ga))
    })
    .await;
    assert_eq!(wolf.0, 4613);
    assert!(near(wolf.1, own.1), "{wolf:?}");
    assert_eq!(plain.0, 200);
    assert!(near(plain.1, 1.0), "{plain:?}");
    assert_eq!(again, wolf);
    assert_eq!(back, own);
}

/// Thistle Tea's `Appearance` "creature equipment follows the selected
/// transform and restores its template": a creature transformed takes the
/// weapons of the creature it shows (mangos's `HandleAuraTransform`
/// loading its equipment), empty hands among them, the one before's as the
/// newer goes, and its own back, every field, at the last.
#[tokio::test]
async fn a_creatures_weapons_follow_its_transform_and_come_back() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let seen = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        let data = m.realm.data.clone();
        m.creature_mut(brute)
            .unwrap()
            .set_equipment(&data, [OWN_SWORD, 0, 0]);
        let weapons = |m: &Map| -> [u32; 9] {
            let f = m.unit_fields(brute).unwrap();
            std::array::from_fn(|i| f.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + i as u16))
        };
        let own = weapons(m);
        put(m, brute, INTO_SMALL);
        let small = weapons(m);
        put(m, brute, INTO_PLAIN);
        let plain = weapons(m);
        take(m, brute, INTO_PLAIN);
        let back_to_small = weapons(m);
        take(m, brute, INTO_SMALL);
        (own, small, plain, back_to_small, weapons(m))
    })
    .await;
    let (own, small, plain, back_to_small, back) = seen;
    assert_eq!(own[0], 500);
    assert_eq!(small[0], 600);
    assert_eq!(plain[..3], [0, 0, 0]);
    assert_eq!(back_to_small, small);
    assert_eq!(back, own, "every field its own");
}
