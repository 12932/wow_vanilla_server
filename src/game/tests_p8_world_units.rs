//! Round 8's world tests, ported from Thistle Tea's
//! `test/game/world/presence_test.exs` (docs/parity/matrix.csv): what
//! others' spells read of a player: their creature type, by their form.

use wow_world_messages::vanilla::*;

use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::map::auras::AURA_MOD_SHAPESHIFT;
use crate::game::map::spells::{self, Targets};
use crate::game::tests_p3_cast_kit::{bare, effect, insert};
use crate::game::tests_p3_combat::{aura_of, data, setup_with};
use crate::game::tests_p3_entity::{bob, on};

/// A spell at any unit of the beasts, humanoids and critters (Polymorph's
/// 0xC1); one of the undead alone; one of the beasts alone (Hibernate's
/// kind).
const POLY: u32 = 997_830;
const UNDEAD_ONLY: u32 = 997_831;
const BEASTS_ONLY: u32 = 997_832;
const CAT_FORM: u32 = 997_833;

/// Thistle Tea's `Presence.enter/2` "player metadata admits Polymorph
/// through validation and target resolution" and `sync/2` "derives
/// creature type from the current form and restores humanoid on
/// removal": a spell of Polymorph's creature types may be cast at Bob, a
/// humanoid, and one of the undead alone may not. In cat form he is a
/// beast, so a spell of the beasts alone may be cast at him, as in his
/// bear, dire bear, travel, aquatic and Ghost Wolf forms, and not in a
/// warrior's stance, a moonkin's or Shadowform's; his form gone, he is a
/// humanoid again (mangos's `GetCreatureType` by `SpellShapeshiftForm`).
#[tokio::test]
async fn a_player_is_a_humanoid_and_a_beast_in_an_animal_form() {
    let mut d = data();
    let at_any = |id: u32, mask: u32| {
        let mut s = bare(id, 0);
        s.target_creature_type = mask;
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: 3,
                target_a: 25,
                ..Default::default()
            },
        );
        s
    };
    insert(
        &mut d,
        [
            at_any(POLY, 0xC1),
            at_any(UNDEAD_ONLY, 0x20),
            at_any(BEASTS_ONLY, 0x1),
        ],
    );
    let (realm, _a, ga, map) = setup_with(d, Class::Mage).await;
    let (_b, gb) = bob(&realm, ga).await;
    let out = on(&map, move |m| {
        let at_bob = Targets {
            unit: Some(gb),
            ..Default::default()
        };
        let cast = |m: &mut crate::game::map::Map, id: u32| {
            // Each cast afresh: no cooldown nor global one between.
            if let Some(c) = m.caster_mut(ga) {
                c.gcd_until = None;
            }
            spells::cast(m, ga, id, at_bob, None)
        };
        let humanoid = (cast(m, POLY), cast(m, UNDEAD_ONLY), cast(m, BEASTS_ONLY));
        aura_of(m, gb, CAT_FORM, AURA_MOD_SHAPESHIFT, 1, 0);
        let form = m.players[&gb].fields.get_byte(UNIT_FIELD_BYTES_1, 2);
        let cat = (cast(m, POLY), cast(m, BEASTS_ONLY));
        let mut forms = Vec::new();
        for form in [1, 3, 4, 5, 8, 14, 15, 16, 0, 2, 17, 18, 19, 28, 30, 31, 32] {
            m.players
                .get_mut(&gb)
                .unwrap()
                .fields
                .set_byte(UNIT_FIELD_BYTES_1, 2, form);
            forms.push((form, cast(m, BEASTS_ONLY)));
        }
        crate::game::map::auras::remove_spell(m, gb, CAT_FORM, None);
        let back = (
            m.players[&gb].fields.get_byte(UNIT_FIELD_BYTES_1, 2),
            cast(m, BEASTS_ONLY),
            cast(m, POLY),
        );
        (humanoid, form, cat, forms, back)
    })
    .await;
    let (humanoid, form, cat, forms, back) = out;
    assert_eq!(
        humanoid,
        (true, false, false),
        "a humanoid: Polymorph's kinds, not the undead's nor the beasts'"
    );
    assert_eq!(form, 1, "in cat form");
    assert_eq!(cat, (true, true), "a beast in it");
    let beast: Vec<u8> = forms.iter().filter(|(_, b)| *b).map(|(f, _)| *f).collect();
    assert_eq!(
        beast,
        vec![1, 3, 4, 5, 8, 14, 15, 16],
        "the animal forms and the Ghost Wolf are beasts, the rest humanoid"
    );
    assert_eq!(back, (0, false, true), "his form gone, a humanoid again");
}

const TRANSFORM: u32 = 997_834;
const GIANT: u32 = 997_835;
const GIANT_DISPLAY: u32 = 997_836;

/// Thistle Tea's `Presence.sync/2` "publishes transformed reach and
/// restores native geometry": transformed into a creature whose model is
/// 0.6 yards about and 2.5 of reach, shown at twice its size, Alice shows
/// it, twice her size, 1.2 yards about and five of reach, as Bob sees
/// her; her transform gone, she is herself again, her own size and reach
/// back (mangos's `HandleAuraTransform` and `UpdateModelData`).
#[tokio::test]
async fn a_transformed_player_has_the_reach_of_what_she_is_and_hers_after() {
    let mut d = data();
    let mut giant = crate::game::tests_p3_combat::template(GIANT, 7);
    giant.display_ids = [GIANT_DISPLAY, 0, 0, 0];
    giant.display_scales = [2.0, 0.0, 0.0, 0.0];
    d.creature_templates.insert(GIANT, giant);
    d.display_info.insert(
        GIANT_DISPLAY,
        crate::game::data::DisplayInfo {
            scale: 1.0,
            bounding_radius: 0.6,
            combat_reach: 2.5,
            ..Default::default()
        },
    );
    let mut s = bare(TRANSFORM, 0);
    effect(
        &mut s,
        0,
        crate::game::tests_p3_cast_kit::aura(
            crate::game::map::auras::AURA_TRANSFORM,
            GIANT as i32,
            0,
            1,
        ),
    );
    insert(&mut d, [s]);
    let (realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let (mut b, _gb) = bob(&realm, ga).await;
    let size = move |m: &crate::game::map::Map| {
        let f = &m.players[&ga].fields;
        (
            f.get_u32(UNIT_FIELD_DISPLAYID),
            f.get_f32(OBJECT_FIELD_SCALE_X),
            f.get_f32(UNIT_FIELD_BOUNDINGRADIUS),
            f.get_f32(UNIT_FIELD_COMBATREACH),
        )
    };
    let own = on(&map, move |m| size(m)).await;
    b.clear();
    let transformed = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, ga, TRANSFORM, Some(ga));
        size(m)
    })
    .await;
    assert_eq!(transformed, (GIANT_DISPLAY, 2.0, 1.2, 5.0), "the giant's");
    let shown = b
        .wait_for("her display, to Bob", |msg| {
            crate::game::harness::values(msg)
                .into_iter()
                .find(|(g, _)| *g == ga)
                .and_then(|(_, mask)| match mask {
                    UpdateMask::Player(p) => p.unit_displayid(),
                    _ => None,
                })
        })
        .await;
    assert_eq!(shown as u32, GIANT_DISPLAY, "Bob sees the giant");
    let back = on(&map, move |m| {
        crate::game::map::auras::remove_spell(m, ga, TRANSFORM, None);
        size(m)
    })
    .await;
    assert_eq!(back, own, "herself again: {own:?}");
    assert!((own.3 - 1.5).abs() < 0.01, "her own reach: {own:?}");
}

/// Thistle Tea's `Visibility` viewpoints "client camera selection
/// survives refresh without changing the authorized target" and "rejects
/// missing, foreign-world, and invalid camera requests": Alice looks
/// through a brute's eyes 150 yards off (Mind Vision's bind sight) and is
/// told of it there; her client's `CMSG_FAR_SIGHT` 0 brings her camera
/// home, so she forgets the brute while it walks on, her far sight still
/// named; a 1 sends it back, and she is told of the brute again. An
/// operation of 2 is dropped; with her sight released, a 1 does nothing
/// (mangos's `HandleFarSightOpcode`).
#[tokio::test]
async fn the_clients_camera_comes_home_and_goes_back_to_its_viewpoint() {
    use crate::game::map::spells::sight;
    use crate::game::tests_p3_combat::{BRUTE, spawn};
    use crate::game::tests_p3_entity::NONE_WITHIN;
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Priest).await;
    let brute = on(&map, move |m| spawn(m, ga, BRUTE, 20, 150.0)).await;
    a.clear();
    on(&map, move |m| sight::grant(m, ga, brute)).await;
    a.sees_on(&map, brute).await;
    let named = move |m: &crate::game::map::Map| m.players[&ga].fields.get_guid(PLAYER_FARSIGHT);
    a.send(CMSG_FAR_SIGHT {
        operation: FarSightOperation::Remove,
    })
    .await;
    a.forgets(brute).await;
    // It walks on twenty yards; her camera stays home.
    on(&map, move |m| {
        let at = m.position_of(brute).unwrap();
        crate::game::map::scripts::place(
            m,
            brute,
            Vector3d {
                x: at.x + 20.0,
                ..at
            },
            0.0,
        );
    })
    .await;
    for _ in 0..5 {
        crate::game::harness::an_update(&map).await;
    }
    assert!(
        !a.sees_within(brute, NONE_WITHIN).await,
        "home, she isn't told of it"
    );
    let kept = on(&map, move |m| named(m)).await;
    assert_eq!(kept, brute, "her far sight still names it");
    a.send(CMSG_FAR_SIGHT {
        operation: FarSightOperation::Add,
    })
    .await;
    a.sees_on(&map, brute).await;
    // An operation the client has none of.
    a.send_raw(0x27A, &[2]).await;
    on(&map, move |m| sight::release(m, ga, brute)).await;
    a.forgets(brute).await;
    a.send(CMSG_FAR_SIGHT {
        operation: FarSightOperation::Add,
    })
    .await;
    for _ in 0..5 {
        crate::game::harness::an_update(&map).await;
    }
    assert!(
        !a.sees_within(brute, NONE_WITHIN).await,
        "released, a 1 shows nothing"
    );
    let cleared = on(&map, move |m| named(m)).await;
    assert!(cleared.is_zero());
}
