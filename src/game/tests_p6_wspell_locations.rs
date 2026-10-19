//! Phase 3, round 6, against Thistle Tea's `spell_caster_locations_test.exs`:
//! a leap's place about its caster, by its direction and radius, short of
//! what is in the way, from where the caster is as it goes off.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, cast_fresh};
use super::tests_p6_wspell::{face_east, off, sides};
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::combat::dist;
use crate::game::map::spells::Targets;

/// Leaps: 10 yards back; ahead with no radius; Blink's 20 yards ahead;
/// Blink cast in two seconds.
const BACK: u32 = 991_500;
const NOWHERE: u32 = 991_501;
const BLINK: u32 = 991_502;
const SLOW_BLINK: u32 = 991_503;
const R10: u32 = 994;
const R20: u32 = 995;

fn data(d: &mut GameData) {
    sides(d);
    d.spells.radii.insert(R10, 10.0);
    d.spells.radii.insert(R20, 20.0);
    let leap = |id: u32, b: u32, radius: u32| {
        let mut s = bare(id, 0);
        s.mana_cost = 10;
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: crate::game::map::spells::EFFECT_LEAP,
                target_a: 1,
                target_b: b,
                radius_index: radius,
                ..Default::default()
            },
        );
        s
    };
    let mut slow = leap(SLOW_BLINK, 47, R20);
    slow.casting_time_index = CAST_2S;
    let spells: Vec<SpellTemplate> = vec![
        leap(BACK, 48, R10),
        leap(NOWHERE, 47, 0),
        leap(BLINK, 47, R20),
        slow,
    ];
    insert(d, spells);
}

fn at(m: &Map, g: Guid) -> (Vector3d, f32) {
    let p = &m.players[&g];
    (p.position(), p.movement.orientation)
}

fn near(a: Vector3d, b: Vector3d) -> bool {
    dist(a, b) < 0.01
}

/// A leap goes its radius in its direction off its caster's facing (back
/// for a leap back), the caster alone, facing as they were; none without a
/// radius; short of a wall on its way, half a yard before it; from where
/// the caster is as it goes off, a triggered one too (Thistle Tea's
/// `CasterLocation` and `Leap`; mangos's `EffectLeap` at its
/// caster-relative place).
#[tokio::test]
async fn a_leap_goes_its_radius_its_way_short_of_a_wall_from_where_its_caster_is() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6XM", "Leaper", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[BACK, NOWHERE, BLINK, SLOW_BLINK]);
            face_east(m, a);
            let mut out = Vec::new();
            let (start, facing) = at(m, a);
            assert!(cast_fresh(m, a, BACK, Targets::default()));
            let back = at(m, a);
            out.push(near(back.0, off_of(start, -10.0, 0.0)) && back.1 == facing);
            m.near_teleport(a, start, facing);
            assert!(cast_fresh(m, a, NOWHERE, Targets::default()));
            out.push(near(at(m, a).0, start));
            // A wall ten yards ahead.
            let (w0, w1) = (off_of(start, 10.0, -5.0), off_of(start, 10.0, 5.0));
            m.spells.walls.push((w0, w1));
            assert!(cast_fresh(m, a, BLINK, Targets::default()));
            let stopped = at(m, a).0.x - start.x;
            out.push((9.4..9.6).contains(&stopped));
            m.spells.walls.clear();
            m.near_teleport(a, start, facing);
            assert!(cast_fresh(m, a, BLINK, Targets::default()));
            out.push(near(at(m, a).0, off_of(start, 20.0, 0.0)));
            // Cast in two seconds, moved meanwhile: from the new place.
            m.near_teleport(a, start, facing);
            assert!(cast_fresh(m, a, SLOW_BLINK, Targets::default()));
            let moved = off(m, a, 0.0, 30.0);
            super::tests_p6_wspell::put(m, a, moved);
            kit::pass(m, 2_100);
            out.push(near(at(m, a).0, off_of(moved, 20.0, 0.0)));
            // Triggered, from where the caster is.
            m.near_teleport(a, start, facing);
            crate::game::map::spells::cast_triggered(m, a, BLINK, None);
            out.push(near(at(m, a).0, off_of(start, 20.0, 0.0)));
            out
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        [true, true, true, true, true, true],
        "back, facing kept; none without a radius; short of a wall; ahead; from the place it went off; triggered"
    );
}

fn off_of(p: Vector3d, dx: f32, dy: f32) -> Vector3d {
    Vector3d {
        x: p.x + dx,
        y: p.y + dy,
        z: p.z,
    }
}
