//! Phase 3's mounts, as Thistle Tea's `mount_test` has them: the mounted
//! speed and its gear's bonuses, the display, what keeps and what takes
//! the mount. On made-up spells and the fast suite.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{apply_template, has, ready};
use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_ALLOW_WHILE_MOUNTED, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::*;
use crate::game::map::derive::run_multiplier;
use crate::game::map::spells;

const MOUNT: u32 = 990_950;
const SPURS: u32 = 990_951;
const CARROT: u32 = 990_952;
const CHARM: u32 = 990_953;
const HASTE: u32 = 990_954;
const RIDING_BUFF: u32 = 990_955;

fn lasting(id: u32, e: &[SpellEffect]) -> SpellTemplate {
    let mut s = bare(id, 0);
    s.duration_index = LASTS_LONG;
    for (i, x) in e.iter().enumerate() {
        effect(&mut s, i, *x);
    }
    s
}

fn mounts(d: &mut GameData) {
    let mut ride = lasting(RIDING_BUFF, &[aura(AURA_DUMMY, 0, 0, 1)]);
    ride.attributes[0] |= ATTR_ALLOW_WHILE_MOUNTED;
    insert(
        d,
        [
            lasting(
                MOUNT,
                &[
                    aura(AURA_MOUNTED, KOBOLD as i32, 0, 1),
                    aura(AURA_MOD_INCREASE_MOUNTED_SPEED, 0, 100, 1),
                ],
            ),
            lasting(SPURS, &[aura(AURA_MOD_MOUNTED_SPEED_ALWAYS, 0, 3, 1)]),
            lasting(CARROT, &[aura(AURA_MOD_MOUNTED_SPEED_ALWAYS, 0, 2, 1)]),
            lasting(CHARM, &[aura(AURA_MOD_MOUNTED_SPEED_NOT_STACK, 0, 10, 1)]),
            lasting(HASTE, &[aura(AURA_MOD_INCREASE_SPEED, 0, 50, 1)]),
            ride,
        ],
    );
}

fn put(m: &mut Map, g: wow_world_messages::Guid, id: u32) {
    let s = m.realm.data.spells.get(id).cloned().unwrap();
    apply_template(m, g, g, &s);
}

fn speed(m: &Map, g: wow_world_messages::Guid) -> f32 {
    let mounted = has(m, g, MOUNT);
    auras(m, g).map_or(1.0, |a| run_multiplier(a, mounted)) * 7.0
}

/// The mounted speed: the mount's, times its gear's bonuses together, or
/// the strongest bonus that doesn't stack if it is better; seven yards a
/// second again once off (Thistle Tea's mounted speed, mangos's
/// `UpdateSpeed`).
#[tokio::test]
async fn a_mounts_speed_takes_its_gears_bonuses_or_the_best_that_doesnt_stack() {
    let realm = fast_realm(mounts);
    let (_c, g, map) = human(&realm, "P3MT", "Rider", Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            put(m, g, MOUNT);
            put(m, g, SPURS);
            put(m, g, CARROT);
            let geared = speed(m, g);
            put(m, g, CHARM);
            let charmed = speed(m, g);
            remove_spell(m, g, MOUNT, None);
            (geared, charmed, speed(m, g))
        })
        .await
        .unwrap();
    let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
    assert!(close(out.0, 14.0 * 1.03 * 1.02), "{}", out.0);
    assert!(close(out.1, 15.4), "the better of the two: {}", out.1);
    assert!(close(out.2, 7.0), "off the mount: {}", out.2);
}

/// The mount's display: on with the mount; kept through an unrelated aura
/// and a cast that may be made mounted; gone, and the speed with it, at
/// death (Thistle Tea's mount display).
#[tokio::test]
async fn a_mount_shows_until_death_and_an_allowed_cast_keeps_it() {
    let realm = fast_realm(mounts);
    let (_c, g, map) = human(&realm, "P3MU", "Ridertwo", Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[RIDING_BUFF]);
            let display = |m: &Map| m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_MOUNTDISPLAYID);
            put(m, g, MOUNT);
            let mounted = display(m);
            put(m, g, HASTE);
            let hasted = display(m);
            spells::cast(m, g, RIDING_BUFF, Default::default(), None);
            let cast = (has(m, g, RIDING_BUFF), display(m), has(m, g, MOUNT));
            crate::game::map::death::kill(m, g);
            (
                mounted,
                hasted,
                cast,
                display(m),
                has(m, g, MOUNT),
                speed(m, g),
            )
        })
        .await
        .unwrap();
    let (mounted, hasted, cast, dead, dead_mount, dead_speed) = out;
    assert_eq!(mounted, 903, "the kobold's display");
    assert_eq!(hasted, 903, "kept through another aura");
    assert_eq!(cast, (true, 903, true), "and a cast allowed mounted");
    assert_eq!((dead, dead_mount), (0, false), "gone at death");
    assert!(
        (dead_speed - 7.0).abs() < 1e-4,
        "and its speed: {dead_speed}"
    );
}
