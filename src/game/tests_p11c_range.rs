//! Round 11c: a spell's range as Thistle Tea (vmangos) has it, the user's
//! call (2026-10-21), against `range_test.exs` ("distinguishes player and
//! creature admission and launch allowances") and `destination_test.exs`
//! ("checks three-dimensional distance with bounding radius and player
//! casting leeway"): past both reaches at a unit, past the caster's
//! bounding radius at a place, a player has 1.25 yards of leeway as a cast
//! begins and 6.25 as it goes off, a creature none and 2.25.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at};
use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::UNIT_FIELD_BOUNDINGRADIUS;
use crate::game::map::Map;
use crate::game::map::spells::{Targets, checks, fail, mods};

const RANGE_30: u32 = 952;
/// A bolt of 30 yards at a foe; a ground spell of 30.
const BOLT: u32 = 998_970;
const GROUND: u32 = 998_971;

fn data(d: &mut GameData) {
    d.spells.ranges.insert(RANGE_30, (0.0, 30.0));
    let mut bolt = bare(BOLT, 2);
    bolt.range_index = RANGE_30;
    effect(&mut bolt, 0, damage(10));
    let mut ground = bare(GROUND, 2);
    ground.range_index = RANGE_30;
    effect(
        &mut ground,
        0,
        SpellEffect {
            target_a: 16,
            ..damage(10)
        },
    );
    insert(d, [bolt, ground]);
}

/// Why `caster` can't begin `spell` at `t`, and why it couldn't go off
/// there, if not.
fn both(m: &mut Map, caster: Guid, spell: u32, t: &Targets) -> (Option<u8>, Option<u8>) {
    kit::fresh(m, caster);
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let mods = mods::for_spell(m, caster, &s);
    (
        checks::check(m, caster, &s, t, None, false, &mods),
        checks::at_finish(m, caster, &s, t, None, &mods),
    )
}

/// `g` put `d` yards in front of `from`.
fn place(m: &mut Map, from: Guid, g: Guid, d: f32) {
    let me = m.unit_view(from).unwrap();
    let to = Vector3d {
        x: me.position.x + d * me.orientation.cos(),
        y: me.position.y + d * me.orientation.sin(),
        z: me.position.z,
    };
    if let Some(c) = m.creature_mut(g) {
        c.position = to;
    }
}

fn ahead(m: &Map, from: Guid, d: f32) -> Targets {
    let me = m.unit_view(from).unwrap();
    Targets {
        dest: Some(Vector3d {
            x: me.position.x + d * me.orientation.cos(),
            y: me.position.y + d * me.orientation.sin(),
            z: me.position.z,
        }),
        ..Default::default()
    }
}

fn reaches(m: &Map, a: Guid, b: Guid) -> f32 {
    m.unit_view(a).unwrap().combat_reach + m.unit_view(b).unwrap().combat_reach
}

fn bound(m: &Map, g: Guid) -> f32 {
    m.unit_fields(g)
        .map_or(0.0, |f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS))
}

const OUT: Option<u8> = Some(fail::OUT_OF_RANGE);

#[tokio::test]
async fn a_player_and_a_creature_each_have_their_leeway_as_a_cast_begins_and_goes_off() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P11CRA", "Ranger", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[BOLT]);
            let k = kit::dummy(m, a, 60, 5.0);
            let r = 30.0 + reaches(m, a, k);
            let mut seen = Vec::new();
            // The player's: 1.25 yards as it begins, 6.25 as it goes off.
            for d in [r + 1.2, r + 1.3, r + 6.2, r + 6.3] {
                place(m, a, k, d);
                seen.push(both(m, a, BOLT, &at(k)));
            }
            // The creature's at the player: none, then 2.25.
            for d in [r - 0.05, r + 0.05, r + 2.2, r + 2.3] {
                place(m, a, k, d);
                seen.push(both(m, k, BOLT, &at(a)));
            }
            seen
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        vec![
            (None, None),
            (OUT, None),
            (OUT, None),
            (OUT, OUT),
            (None, None),
            (OUT, None),
            (OUT, None),
            (OUT, OUT),
        ]
    );
}

#[tokio::test]
async fn a_place_is_measured_past_the_casters_bounding_radius_with_the_same_leeways() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P11CRB", "Grounder", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[GROUND]);
            let k = kit::dummy(m, a, 60, 5.0);
            let mut seen = Vec::new();
            let r = 30.0 + bound(m, a);
            for d in [r + 1.2, r + 1.3, r + 6.2, r + 6.3] {
                let t = ahead(m, a, d);
                seen.push(both(m, a, GROUND, &t));
            }
            let r = 30.0 + bound(m, k);
            for d in [r - 0.05, r + 0.05, r + 2.2, r + 2.3] {
                let t = ahead(m, k, d);
                seen.push(both(m, k, GROUND, &t));
            }
            (seen, bound(m, a) > 0.0)
        })
        .await
        .unwrap();
    assert!(out.1, "a player has a bounding radius");
    assert_eq!(
        out.0,
        vec![
            (None, None),
            (OUT, None),
            (OUT, None),
            (OUT, OUT),
            (None, None),
            (OUT, None),
            (OUT, None),
            (OUT, OUT),
        ]
    );
}
