//! Phase 3's persistent areas, Thistle Tea's `PersistentAreaTest` ported
//! as wvs has them (an area on the ground that ticks on whom it finds, not
//! an aura on each): each damage tick rolls its hit, a resisted one dealing
//! nothing and the area going on; one outside it is spared; a plain DoT
//! doesn't roll again.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::spells;
use crate::game::tests_p3_aura2::{add_spells, aura_spell, health, put};
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};
use crate::game::tests_p3_spells_kit::missed;

/// A ground area of shadow damage, 100 a second for 5 seconds, 5 yards
/// across; a plain DoT of the same; a ward against every school's spells.
const GROUND: u32 = 96_600;
const DOT: u32 = 96_601;
const WARD: u32 = 96_602;
const RADIUS: u32 = 96;
/// A ground area its caster channels, as Blizzard is.
const CHANNELLED: u32 = 96_603;

fn data() -> GameData {
    let mut d = crate::game::tests_p3_aura2::data();
    d.spells.radii.insert(RADIUS, 5.0);
    let mut ground = SpellTemplate {
        id: GROUND,
        name: "Ground".into(),
        school: 5,
        dmg_class: 1,
        equipped_item_class: -1,
        range_index: 1,
        duration_index: 5_000,
        ..Default::default()
    };
    ground.effects[0] = SpellEffect {
        effect: spells::EFFECT_PERSISTENT_AREA_AURA,
        aura: AURA_PERIODIC_DAMAGE,
        amplitude: 1000,
        base_points: 99,
        base_dice: 1,
        target_a: 18,
        radius_index: RADIUS,
        ..Default::default()
    };
    let mut channelled = ground.clone();
    channelled.id = CHANNELLED;
    channelled.attributes[1] |= crate::game::data::spells::ATTR_EX_CHANNELED;
    let mut dot = aura_spell(DOT, Some(5_000), &[(AURA_PERIODIC_DAMAGE, 0, 1000)]);
    dot.school = 5;
    dot.dmg_class = 1;
    dot.effects[0].target_a = 6;
    add_spells(
        &mut d,
        [
            ground,
            channelled,
            dot,
            aura_spell(WARD, None, &[(AURA_MOD_RESIST_CHANCE, 0x7F, 0)]),
        ],
    );
    d
}

/// Time on the map: the spells' areas and the auras.
fn pass(m: &mut Map, ms: u64) {
    m.now += Duration::from_millis(ms);
    spells::update(m);
    auras::update(m);
}

fn place(m: &mut Map, g: Guid, at: Vector3d) {
    if let Some(c) = m.creature_mut(g) {
        c.position = at;
    }
}

/// Thistle Tea's ground ticks: a tick on a unit that resists the area
/// deals nothing and is told as resisted, and the area goes on to deal
/// its next tick once the unit no longer resists; a unit gone out of it
/// isn't hit. A plain DoT on a unit that resists everything ticks all the
/// same.
#[tokio::test]
async fn each_ground_tick_rolls_its_hit_on_those_inside() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    a.clear();
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            put(m, brute, brute, WARD, &[1_000]).unwrap();
            let at = m.position_of(brute);
            spells::cast_triggered_with(
                m,
                ga,
                GROUND,
                spells::Targets {
                    dest: at,
                    ..Default::default()
                },
            );
            pass(m, 1_000);
            let resisted = health(m, brute);
            auras::remove_spell(m, brute, WARD, None);
            pass(m, 1_000);
            let hit = health(m, brute);
            let away = m
                .position_of(brute)
                .map(|p| Vector3d { x: p.x + 20.0, ..p });
            if let Some(p) = away {
                place(m, brute, p);
            }
            pass(m, 1_000);
            let outside = health(m, brute);
            // A plain DoT doesn't roll its ticks.
            let other = spawn(m, ga, BRUTE, 20, 5.0);
            put(m, other, other, WARD, &[1_000]).unwrap();
            put(m, ga, other, DOT, &[100]).unwrap();
            pass(m, 1_000);
            (resisted, hit, outside, health(m, other))
        })
        .await
        .unwrap();
    assert_eq!(got.0, 100_000, "nothing dealt");
    assert_eq!(missed(&mut a, GROUND).await, 2, "told as resisted");
    assert_eq!(got.1, 99_900, "the next tick deals its damage");
    assert_eq!(got.2, 99_900, "none out of the area");
    assert_eq!(got.3, 99_900, "the DoT's tick lands");
}

/// At `at`, `dx` yards east.
fn east(at: Vector3d, dx: f32) -> Vector3d {
    Vector3d { x: at.x + dx, ..at }
}

/// A persistent area's tick hits only those it sees from where it is, as
/// mangos's `DynamicObjectUpdater` tests sight: one inside behind a wall
/// is spared.
#[tokio::test]
async fn a_ground_tick_doesnt_reach_through_a_wall() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let open = spawn(m, ga, BRUTE, 20, 5.0);
            let walled = spawn(m, ga, BRUTE, 20, 6.0);
            let at = m.position_of(open).unwrap();
            place(m, open, east(at, 2.0));
            place(m, walled, east(at, -2.0));
            let wall = |dy: f32| Vector3d {
                x: at.x - 1.0,
                y: at.y + dy,
                z: at.z,
            };
            m.spells.walls.push((wall(-10.0), wall(10.0)));
            spells::cast_triggered_with(
                m,
                ga,
                GROUND,
                spells::Targets {
                    dest: Some(at),
                    ..Default::default()
                },
            );
            for _ in 0..5 {
                pass(m, 1_000);
            }
            (health(m, open), health(m, walled))
        })
        .await
        .unwrap();
    assert!(got.0 < 100_000, "the one in sight is hit: {}", got.0);
    assert_eq!(got.1, 100_000, "the one behind the wall isn't");
}

/// A tick that ends another area (its channeller killed) doesn't shift
/// which areas tick: one cast later, not yet due, waits for its own time.
#[tokio::test]
async fn an_area_ended_by_a_tick_doesnt_tick_another_early() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let channeller = spawn(m, ga, BRUTE, 20, 5.0);
            let later = spawn(m, ga, BRUTE, 20, 5.0);
            let at = m.position_of(channeller).unwrap();
            place(m, later, east(at, 30.0));
            crate::game::tests_p3_aura2::set(m, channeller, crate::game::fields::UNIT_FIELD_HEALTH, 1);
            let dest = |p: Vector3d| spells::Targets {
                dest: Some(p),
                ..Default::default()
            };
            spells::cast_triggered_with(m, ga, GROUND, dest(at));
            spells::cast(m, channeller, CHANNELLED, dest(east(at, -30.0)), None);
            let areas = m.spells.areas.len();
            pass(m, 500);
            spells::cast_triggered_with(m, ga, GROUND, dest(east(at, 30.0)));
            pass(m, 500);
            (
                areas,
                health(m, channeller),
                m.spells.areas.len(),
                health(m, later),
            )
        })
        .await
        .unwrap();
    assert_eq!(got.0, 2, "the ground and the channel");
    assert_eq!(got.1, 0, "the channeller killed");
    assert_eq!(got.2, 2, "its area gone, the later one there");
    assert_eq!(got.3, 100_000, "the later area not yet ticked");
}
