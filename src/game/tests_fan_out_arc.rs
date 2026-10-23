//! Creatures fighting a player fan out over an arc on the side they came
//! from (`[creatures] fan_out_arc_degrees`, `ai::arc_of`), never all round
//! them: sixteen from the left stand on the left, all on one ring at
//! their reach (`ai::ring`), two groups from either side each on their
//! own, a big victim's ring further out, and the setting sets the arc's
//! width.

use std::f32::consts::{FRAC_PI_2, PI};
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::Map;
use crate::game::map::combat::{self, dist};
use crate::game::map::creature::{Creature, Motion};
use crate::game::tests_pets::{self, WOLF};

/// One update of the creatures, on the map's clock, the player hardy.
fn turn(m: &mut Map, player: Guid) {
    m.now += Duration::from_millis(50);
    crate::game::map::ai::update_creatures(m);
    combat::update_creatures(m);
    if let Some(p) = m.players.get_mut(&player) {
        p.fields.set_u32(UNIT_FIELD_MAXHEALTH, 10_000_000);
        p.fields.set_u32(UNIT_FIELD_HEALTH, 10_000_000);
    }
}

fn wrap(a: f32) -> f32 {
    a.sin().atan2(a.cos())
}

/// The bearing of `at` from `from`.
fn bearing(from: Vector3d, at: Vector3d) -> f32 {
    (at.y - from.y).atan2(at.x - from.x)
}

fn planar(a: Vector3d, b: Vector3d) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// The widest two of `angles` are apart, in degrees.
fn span(angles: &[f32]) -> f32 {
    let mut widest = 0.0f32;
    for a in angles {
        for b in angles {
            widest = widest.max(wrap(a - b).abs().to_degrees());
        }
    }
    widest
}

/// The fan-out's width, as a reload would make it.
fn arc_degrees(m: &mut Map, degrees: f32) {
    let mut cfg = (*m.cfg).clone();
    cfg.creatures.fan_out_arc_degrees = degrees;
    m.cfg = std::sync::Arc::new(cfg);
}

/// `n` wolves `side` of the player, 10 to 16 yards off and within 15
/// degrees either way of it, each attacking them.
fn pack(m: &mut Map, player: Guid, side: f32, n: usize, counter: u32) -> Vec<Guid> {
    let data = m.realm.data.clone();
    let t = data.creature_template(WOLF).cloned().unwrap();
    let at = m.players[&player].position();
    (0..n)
        .map(|i| {
            let k = i as f32 / (n.max(2) - 1) as f32;
            let a = side + (k - 0.5) * 30f32.to_radians();
            let d = 10.0 + 6.0 * ((i * 7) % n) as f32 / n as f32;
            let pos = Vector3d {
                x: at.x + d * a.cos(),
                y: at.y + d * a.sin(),
                z: at.z,
            };
            let g = m.add_creature(Creature::new(
                crate::game::guid::unit(WOLF, counter + i as u32),
                &t,
                1,
                pos,
                a + PI,
            ));
            crate::game::map::ai::attacked(m, g, player);
            g
        })
        .collect()
}

/// Whether every one of `wolves` stands still in melee reach of `player`.
fn settled(m: &Map, wolves: &[Guid], player: Guid) -> bool {
    let them = m.unit_view(player).unwrap();
    wolves.iter().all(|g| {
        matches!(m.creature(*g).unwrap().motion, Motion::Idle { .. })
            && combat::in_melee_range(&m.unit_view(*g).unwrap(), &them)
    })
}

/// What [`fan_out`] finds: each wolf's bearing from the player, by pack;
/// how far each stands from them; how many times one moved in forty
/// updates once they stood; and a wolf's melee reach of the player.
struct Fanned {
    sides: Vec<Vec<f32>>,
    apart: Vec<f32>,
    stepped: usize,
    reach: f32,
}

/// Packs of wolves, each `(side, count)`, set at a still player with the
/// fan-out `degrees` wide, the player of bounding radius and combat reach
/// `size` if given: where they stand once all stand in reach.
async fn fan_out(degrees: f32, packs: &'static [(f32, usize)], size: Option<(f32, f32)>) -> Fanned {
    let realm = harness::realm_with(tests_pets::data());
    let mut c = Client::connect(&realm, "A");
    let player = c.enter("Fanned", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(player).expect("online").map();
    map.query(move |m| {
        arc_degrees(m, degrees);
        if let Some((radius, reach)) = size {
            let f = &mut m.players.get_mut(&player).unwrap().fields;
            f.set_f32(UNIT_FIELD_BOUNDINGRADIUS, radius);
            f.set_f32(UNIT_FIELD_COMBATREACH, reach);
        }
        let facing = m.players[&player].movement.orientation;
        let groups: Vec<Vec<Guid>> = packs
            .iter()
            .enumerate()
            .map(|(i, (side, n))| pack(m, player, facing + side, *n, 0x72_0000 + 0x100 * i as u32))
            .collect();
        let all: Vec<Guid> = groups.iter().flatten().copied().collect();
        let mut ok = false;
        for i in 0..1200 {
            turn(m, player);
            if i > 400 && settled(m, &all, player) {
                ok = true;
                break;
            }
        }
        assert!(ok, "every wolf stands in reach");
        let me = m.players[&player].position();
        let before: Vec<Vector3d> = all.iter().map(|g| m.position_of(*g).unwrap()).collect();
        let mut stepped = 0;
        for _ in 0..40 {
            turn(m, player);
            stepped += all
                .iter()
                .zip(&before)
                .filter(|(g, at)| dist(m.position_of(**g).unwrap(), **at) > 0.01)
                .count();
        }
        let sides = groups
            .iter()
            .map(|g| {
                g.iter()
                    .map(|w| wrap(bearing(me, m.position_of(*w).unwrap()) - facing))
                    .collect()
            })
            .collect();
        let apart = all
            .iter()
            .map(|g| planar(me, m.position_of(*g).unwrap()))
            .collect();
        let reach =
            combat::melee_range(&m.unit_view(all[0]).unwrap(), &m.unit_view(player).unwrap());
        Fanned {
            sides,
            apart,
            stepped,
            reach,
        }
    })
    .await
    .unwrap()
}

/// How far the furthest of `sides` is from `from`, in degrees.
fn widest_from(sides: &[f32], from: f32) -> f32 {
    sides
        .iter()
        .map(|s| wrap(s - from).abs().to_degrees())
        .fold(0.0, f32::max)
}

/// Sixteen wolves from a player's left stand on the left, within the
/// 120-degree arc about the side they came from and of where they stand,
/// none behind the player's side, every one in reach and as far from them
/// as the others; and stay there.
#[tokio::test]
async fn sixteen_from_one_side_fan_out_over_an_arc_not_all_round() {
    static LEFT: [(f32, usize); 1] = [(FRAC_PI_2, 16)];
    let Fanned {
        sides,
        apart,
        stepped,
        reach,
    } = fan_out(120.0, &LEFT, None).await;
    let sides = &sides[0];
    let from = widest_from(sides, FRAC_PI_2);
    assert!(
        from <= 61.0,
        "within 60 degrees of the side they came from: {from:.1}"
    );
    let span = span(sides);
    assert!(
        span <= 121.0,
        "all within 120 degrees of each other: {span:.1}"
    );
    assert_eq!(stepped, 0, "the spots hold while the player stands");
    let (nearest, furthest) = ring(&apart);
    assert!(
        furthest - nearest <= 1.0,
        "one ring, within half a yard of one distance: {nearest:.2} to {furthest:.2} yards"
    );
    assert!(
        (nearest - reach / 2.0).abs() <= 0.5,
        "at half their reach {reach:.2}, as they chase to: {nearest:.2}"
    );
}

/// The nearest and furthest of `apart`.
fn ring(apart: &[f32]) -> (f32, f32) {
    let nearest = apart.iter().copied().fold(f32::MAX, f32::min);
    let furthest = apart.iter().copied().fold(0.0, f32::max);
    (nearest, furthest)
}

/// Round a big player (a bounding radius of 3 and a combat reach of 6),
/// sixteen wolves from the left stand on one ring further out, at their
/// greater reach and outside the player's size, on the left still.
#[tokio::test]
async fn a_big_victims_ring_is_at_its_greater_reach() {
    static LEFT: [(f32, usize); 1] = [(FRAC_PI_2, 16)];
    let Fanned {
        sides,
        apart,
        stepped,
        reach,
    } = fan_out(120.0, &LEFT, Some((3.0, 6.0))).await;
    let (nearest, furthest) = ring(&apart);
    assert!(
        furthest - nearest <= 1.0,
        "one ring: {nearest:.2} to {furthest:.2} yards"
    );
    assert!(
        (nearest - reach / 2.0).abs() <= 0.5 && nearest > 3.0 + 0.389,
        "at half the greater reach {reach:.2}, outside the size: {nearest:.2}"
    );
    let from = widest_from(&sides[0], FRAC_PI_2);
    assert!(from <= 61.0, "within the arc: {from:.1}");
    assert_eq!(stepped, 0, "the spots hold while the player stands");
}

/// Six wolves from the left and six from the right each keep to their
/// own side.
#[tokio::test]
async fn two_packs_from_either_side_keep_each_to_their_own() {
    static BOTH: [(f32, usize); 2] = [(FRAC_PI_2, 6), (-FRAC_PI_2, 6)];
    let Fanned { sides, stepped, .. } = fan_out(120.0, &BOTH, None).await;
    let left = widest_from(&sides[0], FRAC_PI_2);
    let right = widest_from(&sides[1], -FRAC_PI_2);
    assert!(left <= 61.0, "the left pack on the left: {left:.1}");
    assert!(right <= 61.0, "the right pack on the right: {right:.1}");
    assert_eq!(stepped, 0, "the spots hold while the player stands");
}

/// The setting sets the width: sixteen from the left stand within 30
/// degrees of it at 60, and spill wider than 120 at 300.
#[tokio::test]
async fn the_arc_is_as_wide_as_the_setting() {
    static LEFT: [(f32, usize); 1] = [(FRAC_PI_2, 16)];
    let narrow = fan_out(60.0, &LEFT, None).await;
    let narrow = widest_from(&narrow.sides[0], FRAC_PI_2);
    assert!(narrow <= 31.0, "within 30 degrees at 60: {narrow:.1}");
    let wide = fan_out(300.0, &LEFT, None).await;
    let wide = widest_from(&wide.sides[0], FRAC_PI_2);
    assert!(wide > 60.0, "wider than 120 degrees at 300: {wide:.1}");
}
