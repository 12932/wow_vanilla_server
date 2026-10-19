//! Phase 3, round 6, against Thistle Tea's `spell_caster_locations_test.exs`:
//! a leap's place about its caster, by its direction and radius, short of
//! what is in the way, from where the caster is as it goes off.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, cast_fresh};
use super::tests_p6_wspell::{BEAST, FRIEND, face_east, off, sides, unit_at};
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
/// Damage at an enemy's place, five yards about, thirty yards' range; one
/// cast in two seconds.
const AT_ITS_PLACE: u32 = 991_504;
const AT_ITS_PLACE_SLOW: u32 = 991_505;
/// Damage at an enemy's place and about it.
const SPLASH: u32 = 991_506;
const R5: u32 = 993;

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
    d.spells.radii.insert(R5, 5.0);
    let placed = |id: u32| {
        let mut s = bare(id, 2);
        s.range_index = super::tests_p6_wspell::RANGE_30;
        s.mana_cost = 10;
        effect(
            &mut s,
            0,
            SpellEffect {
                target_a: 53,
                radius_index: R5,
                ..damage(10)
            },
        );
        s
    };
    let mut splash = placed(SPLASH);
    splash.effects[0].target_b = 16;
    let mut placed_slow = placed(AT_ITS_PLACE_SLOW);
    placed_slow.casting_time_index = CAST_2S;
    let mut slow = leap(SLOW_BLINK, 47, R20);
    slow.casting_time_index = CAST_2S;
    let spells: Vec<SpellTemplate> = vec![
        placed(AT_ITS_PLACE),
        placed_slow,
        splash,
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

/// A spell at an enemy's place wants the enemy: there, alive, an enemy,
/// in range and selectable, as it is cast; and still there as it ends,
/// or it fails before it costs. Its splash is about where the enemy is,
/// not a place sent with it (Thistle Tea's unit locations; mangos's
/// `TARGET_LOCATION_CASTER_TARGET_POSITION` and its unit's checks).
#[tokio::test]
async fn a_spell_at_an_enemys_place_wants_the_enemy_there() {
    use crate::game::map::spells::{checks, fail, mods};
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6XU", "Placer", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[AT_ITS_PLACE, AT_ITS_PLACE_SLOW]);
            face_east(m, a);
            let why = |m: &mut Map, t: Targets| {
                kit::fresh(m, a);
                let s = m.realm.data.spells.get(AT_ITS_PLACE).cloned().unwrap();
                let mods = mods::for_spell(m, a, &s);
                checks::check(m, a, &s, &t, None, false, &mods)
            };
            let none = why(m, Targets::default());
            let enemy = unit_at(m, a, BEAST, 10.0, 0.0);
            let fine = why(m, kit::at(enemy));
            let dead = unit_at(m, a, BEAST, 10.0, 1.0);
            crate::game::map::combat::kill(m, a, dead);
            let dead = why(m, kit::at(dead));
            let friend = unit_at(m, a, FRIEND, 10.0, -1.0);
            let friend = why(m, kit::at(friend));
            let far = unit_at(m, a, BEAST, 60.0, 0.0);
            let far = why(m, kit::at(far));
            let hidden = unit_at(m, a, BEAST, 10.0, 2.0);
            m.unit_fields_mut(hidden).unwrap().set_flag(
                crate::game::fields::UNIT_FIELD_FLAGS,
                0x0200_0000,
                true,
            );
            let unselectable = why(m, kit::at(hidden));
            // A splash about the enemy, whatever place was sent.
            let beside = unit_at(m, a, BEAST, 12.0, 0.0);
            let sent = Targets {
                unit: Some(enemy),
                dest: Some(off(m, a, 0.0, 25.0)),
                ..Default::default()
            };
            let s = m.realm.data.spells.get(SPLASH).cloned().unwrap();
            let mut splashed = crate::game::map::spells::targets::effect_targets(
                m,
                a,
                &s,
                0,
                &sent,
                &mods::Mods::default(),
            );
            splashed.sort_by_key(|g| g.guid());
            let mut expected = vec![enemy, beside];
            expected.sort_by_key(|g| g.guid());
            assert_eq!(splashed, expected, "about the enemy, not the place sent");
            kit::set_power(m, a, 0, 1_000);
            let before = kit::power(m, a, 0);
            assert!(cast_fresh(m, a, AT_ITS_PLACE_SLOW, kit::at(enemy)));
            m.remove_creature(enemy);
            kit::pass(m, 2_100);
            let gone = (kit::casting(m, a), before - kit::power(m, a, 0));
            (none, fine, dead, friend, far, unselectable, gone)
        })
        .await
        .unwrap();
    assert!(out.0.is_some(), "no unit: {:?}", out.0);
    assert_eq!(out.1, None, "an enemy in range");
    assert_eq!(out.2, Some(fail::TARGETS_DEAD), "not the dead");
    assert_eq!(out.3, Some(fail::TARGET_FRIENDLY), "not a friend");
    assert_eq!(out.4, Some(fail::OUT_OF_RANGE), "not beyond range");
    assert_eq!(out.5, Some(fail::BAD_TARGETS), "not the unselectable");
    assert_eq!(out.6, (None, 0), "gone as it ends: failed, unpaid");
}
