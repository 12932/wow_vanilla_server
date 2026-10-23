//! Players' melee pets spread round their victim as creatures do round a
//! player (`ai::spread`, `ai::chase_goal`): each takes a spot of its own,
//! shared with every other fighting that victim, creatures and pets
//! alike; the spots hold while the victim stands; and an imp still casts
//! from its range.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::auras::AURA_MOD_ROOT;
use crate::game::map::combat::{self, dist};
use crate::game::map::pets::{self, testing as summon};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::aura_of;
use crate::game::tests_pets::{self, BOAR, FELHUNTER, IMP, SUMMON_IMP, WOLF, place_in, template};

const VOIDWALKER: u32 = 1860;
/// A creature of the players' side, to fight the wolf with the pets.
const FOOTMAN: u32 = 3030;
const ROOT: u32 = 9_990;

fn data() -> crate::game::data::GameData {
    let mut d = tests_pets::data();
    let mut v = template(VOIDWALKER, "Voidwalker", 14, 1);
    v.creature_type = 3;
    d.creature_templates.insert(VOIDWALKER, v);
    d.creature_templates
        .insert(FOOTMAN, template(FOOTMAN, "Footman", 1, 1));
    d
}

/// `n` players of `class` in the world: their clients, GUIDs, and map.
async fn players(
    realm: &std::sync::Arc<crate::game::Realm>,
    n: usize,
    class: Class,
) -> (Vec<Client>, Vec<Guid>, MapHandle) {
    let mut clients = Vec::new();
    let mut guids = Vec::new();
    for i in 0..n {
        let mut c = Client::connect(realm, &format!("F{i}"));
        let name = format!("Fan{}", (b'a' + i as u8) as char);
        let race = if class == Class::Warlock {
            Race::Human
        } else {
            Race::Dwarf
        };
        guids.push(c.enter(&name, race, class).await);
        clients.push(c);
    }
    let map = realm.registry.online(guids[0]).expect("online").map();
    (clients, guids, map)
}

/// The wolf they fight, `d` yards before `g`: rooted, unkillable, and of
/// bounding radius `radius` and combat reach `reach`.
fn victim(m: &mut Map, g: Guid, d: f32, radius: f32, reach: f32) -> Guid {
    let w = place_in(m, g, WOLF, 0x71_0001, d);
    aura_of(m, w, ROOT, AURA_MOD_ROOT, 0, 0);
    let f = m.unit_fields_mut(w).unwrap();
    f.set_u32(UNIT_FIELD_MAXHEALTH, 10_000_000);
    f.set_u32(UNIT_FIELD_HEALTH, 10_000_000);
    f.set_f32(UNIT_FIELD_BOUNDINGRADIUS, radius);
    f.set_f32(UNIT_FIELD_COMBATREACH, reach);
    w
}

/// One update of the creatures and pets, on the map's clock.
fn turn(m: &mut Map) {
    m.now += Duration::from_millis(50);
    crate::game::map::ai::update_creatures(m);
    combat::update_creatures(m);
    pets::update(m);
}

fn planar(a: Vector3d, b: Vector3d) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

fn radius(m: &Map, g: Guid) -> f32 {
    m.unit_fields(g)
        .map_or(0.0, |f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS))
}

/// Where each of `fighters` stands, and whether all of them stand still
/// in melee reach of `w`.
fn settled(m: &Map, fighters: &[Guid], w: Guid) -> (Vec<Vector3d>, bool) {
    let them = m.unit_view(w).unwrap();
    let at: Vec<Vector3d> = fighters.iter().map(|g| m.position_of(*g).unwrap()).collect();
    let ok = fighters.iter().all(|g| {
        let c = m.creature(*g).unwrap();
        matches!(c.motion, crate::game::map::creature::Motion::Idle { .. })
            && combat::in_melee_range(&m.unit_view(*g).unwrap(), &them)
    });
    (at, ok)
}

/// The nearest two of `fighters` (as far apart as their radii make them
/// stacked, at least), over their radii combined.
fn tightest(m: &Map, fighters: &[Guid], at: &[Vector3d]) -> f32 {
    let mut worst = f32::MAX;
    for i in 0..fighters.len() {
        for j in i + 1..fighters.len() {
            let apart = planar(at[i], at[j]);
            let room = radius(m, fighters[i]) + radius(m, fighters[j]);
            worst = worst.min(apart / room);
        }
    }
    worst
}

/// Twenty hunters' pets sent at one wolf each make for a spot of their
/// own about it: they end standing in its reach, no two nearer than
/// their radii, and stay so while it stands; and round a big wolf (a
/// boss's bounding radius and reach) outside its size.
async fn twenty_pets_on(radius_of_wolf: f32, reach_of_wolf: f32) {
    let realm = harness::realm_with(data());
    let (_clients, hunters, map) = players(&realm, 20, Class::Hunter).await;
    let out = map
        .query(move |m| {
            let pets: Vec<Guid> = hunters
                .iter()
                .map(|h| {
                    pets::give_pet(m, *h, BOAR, 10).expect("a pet");
                    pets::pet_of(m, *h).unwrap()
                })
                .collect();
            let w = victim(m, hunters[0], 12.0, radius_of_wolf, reach_of_wolf);
            for p in &pets {
                pets::ai::attack(m, *p, w);
            }
            // Twenty seconds: there, and every spread done.
            let mut ready = None;
            for i in 0..1200 {
                turn(m);
                let (_, ok) = settled(m, &pets, w);
                if ok && i > 400 {
                    ready = Some(i);
                    break;
                }
            }
            let (before, ok) = settled(m, &pets, w);
            let worst = tightest(m, &pets, &before);
            let wolf = m.position_of(w).unwrap();
            let nearest = before
                .iter()
                .map(|p| planar(*p, wolf))
                .fold(f32::MAX, f32::min);
            // Forty updates more, the wolf still: no one moves.
            let mut stepped = 0;
            for _ in 0..40 {
                turn(m);
                let (now, _) = settled(m, &pets, w);
                stepped += before
                    .iter()
                    .zip(&now)
                    .filter(|(a, b)| dist(**a, **b) > 0.01)
                    .count();
            }
            (ready, ok, worst, nearest, stepped)
        })
        .await
        .unwrap();
    let (ready, ok, worst, nearest, stepped) = out;
    assert!(ready.is_some() && ok, "all twenty stand in reach");
    assert!(
        worst >= 1.0,
        "no two nearer than their radii combined: {worst:.2} of it"
    );
    assert!(
        nearest >= radius_of_wolf,
        "outside the wolf's size {radius_of_wolf}: {nearest:.2}"
    );
    assert_eq!(stepped, 0, "the spots hold while the wolf stands");
}

#[tokio::test]
async fn twenty_hunters_pets_spread_round_one_victim_and_hold_their_spots() {
    twenty_pets_on(0.5, 1.5).await;
}

#[tokio::test]
async fn twenty_pets_spread_round_a_big_victim_outside_its_size() {
    twenty_pets_on(3.0, 6.0).await;
}

/// A Voidwalker, a Felhunter and a hunter's boar fighting a wolf with
/// three footmen share its spots: the six stand apart, each in reach.
#[tokio::test]
async fn demons_a_hunters_pet_and_creatures_spread_among_each_other() {
    let realm = harness::realm_with(data());
    let (_c, warlocks, map) = players(&realm, 2, Class::Warlock).await;
    let mut h = Client::connect(&realm, "H");
    let hunter = h.enter("Hunter", Race::Dwarf, Class::Hunter).await;
    let out = map
        .query(move |m| {
            // The warlocks by the hunter, the other side's start.
            let here = m.players[&hunter].movement;
            for g in &warlocks {
                let p = m.players.get_mut(g).unwrap();
                p.movement.position = here.position;
                p.movement.orientation = here.orientation;
            }
            summon::summon_demon(m, warlocks[0], VOIDWALKER, 0);
            summon::summon_demon(m, warlocks[1], FELHUNTER, 0);
            pets::give_pet(m, hunter, BOAR, 10).expect("a pet");
            let mut fighters: Vec<Guid> = [warlocks[0], warlocks[1], hunter]
                .iter()
                .map(|o| pets::pet_of(m, *o).unwrap())
                .collect();
            let w = victim(m, hunter, 12.0, 0.5, 1.5);
            for p in &fighters {
                pets::ai::attack(m, *p, w);
            }
            for i in 0..3 {
                let f = place_in(m, hunter, FOOTMAN, 0x71_0010 + i, 9.0);
                let fields = m.unit_fields_mut(f).unwrap();
                fields.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
                fields.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
                crate::game::map::ai::attacked(m, f, w);
                fighters.push(f);
            }
            let mut ok = false;
            for i in 0..1200 {
                turn(m);
                if i > 400 && settled(m, &fighters, w).1 {
                    ok = true;
                    break;
                }
            }
            let (at, _) = settled(m, &fighters, w);
            (ok, tightest(m, &fighters, &at))
        })
        .await
        .unwrap();
    let (ok, worst) = out;
    assert!(ok, "all six stand in reach");
    assert!(worst >= 1.0, "apart: {worst:.2} of their radii");
}

/// The imp takes no melee spot: it casts its Firebolt from its range,
/// standing off the wolf the boar spreads round.
#[tokio::test]
async fn the_imp_still_casts_from_its_range() {
    let realm = harness::realm_with(data());
    let (_c, warlocks, map) = players(&realm, 1, Class::Warlock).await;
    let out = map
        .query(move |m| {
            let g = warlocks[0];
            if let Some(p) = m.players.get_mut(&g) {
                p.spells.push(SUMMON_IMP);
            }
            summon::summon_demon(m, g, IMP, SUMMON_IMP);
            let imp = pets::pet_of(m, g).unwrap();
            assert_eq!(m.creature(imp).unwrap().entry, IMP);
            let w = victim(m, g, 20.0, 0.5, 1.5);
            pets::ai::attack(m, imp, w);
            let mut cast = false;
            let mut nearest = f32::MAX;
            for _ in 0..200 {
                turn(m);
                if let Some(f) = m.unit_fields_mut(imp) {
                    let max = f.get_u32(UNIT_FIELD_MAXPOWER1).max(1000);
                    f.set_u32(UNIT_FIELD_MAXPOWER1, max);
                    f.set_u32(UNIT_FIELD_POWER1, max);
                }
                cast |= m
                    .caster(imp)
                    .and_then(|c| c.casting.as_ref())
                    .is_some_and(|c| c.spell == tests_pets::FIREBOLT);
                nearest = nearest.min(dist(m.position_of(imp).unwrap(), m.position_of(w).unwrap()));
            }
            (cast, nearest)
        })
        .await
        .unwrap();
    let (cast, nearest) = out;
    assert!(cast, "the imp casts its Firebolt");
    assert!(nearest > 8.0, "from its range, not in melee: {nearest:.1}");
}

/// `n` hunters in Tanaris, at `to`, in a realm of the real navmesh and
/// terrain and this file's data: their clients, GUIDs, and map.
async fn hunters_in_tanaris(
    n: usize,
    to: Vector3d,
    facing: f32,
) -> (Vec<Client>, Vec<Guid>, MapHandle) {
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    let mut cfg = harness::test_config();
    cfg.terrain_dir = Some(harness::real_data("terrain"));
    cfg.nav_dir = Some(harness::real_data("nav"));
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, data(), None);
    let (mut clients, guids, home) = players(&realm, n, Class::Hunter).await;
    for (c, g) in clients.iter_mut().zip(&guids) {
        let g = *g;
        home.call(move |m| m.teleport(g, crate::game::map::MapKey::world(1), to, facing));
        c.wait_for("SMSG_NEW_WORLD", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
        })
        .await;
        c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
        c.sees(g).await;
    }
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(1));
    for _ in 0..10 {
        harness::an_update(&map).await;
    }
    (clients, guids, map)
}

/// On the real navmesh, ten pets sent at a wolf on a dune's slope in
/// Tanaris spread round it on the ground: each spot on the navmesh, within
/// a yard of the ground there, apart and in reach.
#[tokio::test]
#[ignore = "extended"]
async fn pets_spread_round_a_victim_on_a_slope_on_the_navmesh() {
    let to = Vector3d {
        x: -8012.0,
        y: -4562.0,
        z: 10.0,
    };
    let facing = 7.0f32.atan2(12.0);
    let (_c, hunters, map) = hunters_in_tanaris(10, to, facing).await;
    let out = map
        .query(move |m| {
            let pets: Vec<Guid> = hunters
                .iter()
                .map(|h| {
                    pets::give_pet(m, *h, BOAR, 10).expect("a pet");
                    pets::pet_of(m, *h).unwrap()
                })
                .collect();
            // The wolf on the dune's slope, eight yards below the hunters.
            let w = victim(m, hunters[0], 13.9, 0.5, 1.5);
            let at = m.position_of(w).unwrap();
            let z = m.nav.as_mut().unwrap().ground(at.x, at.y, at.z + 5.0).unwrap();
            let c = m.creature_mut(w).unwrap();
            c.position.z = z;
            for p in &pets {
                pets::ai::attack(m, *p, w);
            }
            let mut ok = false;
            for i in 0..1200 {
                turn(m);
                if i > 400 && settled(m, &pets, w).1 {
                    ok = true;
                    break;
                }
            }
            let (spots, _) = settled(m, &pets, w);
            let worst = tightest(m, &pets, &spots);
            let nav = m.nav.as_mut().unwrap();
            let off = spots
                .iter()
                .map(|p| {
                    nav.ground(p.x, p.y, p.z + 2.0)
                        .map_or(f32::MAX, |g| (g - p.z).abs())
                })
                .fold(0.0, f32::max);
            let slope = spots.iter().map(|p| p.z).fold(f32::MIN, f32::max)
                - spots.iter().map(|p| p.z).fold(f32::MAX, f32::min);
            (ok, worst, off, slope)
        })
        .await
        .unwrap();
    let (ok, worst, off, slope) = out;
    assert!(ok, "all ten stand in reach");
    assert!(worst >= 1.0, "apart: {worst:.2} of their radii");
    assert!(off <= 1.0, "each on the ground: {off:.2} yards off it at most");
    assert!(slope > 0.5, "on a slope: {slope:.2} yards between the spots");
}
