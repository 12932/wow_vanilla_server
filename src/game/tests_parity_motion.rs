//! Creature movement as Thistle Tea has it, and its packets as the 1.12
//! client reads them (mangos-classic's `Movement/packet_builder.cpp`):
//! the spline packet's layout, wandering, charge, fear and confusion,
//! distract, formations, scripted following, pools.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::combat::dist;
use crate::game::map::creature::{Motion, Walk};
use crate::game::map::{MapHandle, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

const HUMANS: u32 = 1;
/// Hostile to humans.
const PACK: u32 = 14;
const NEUTRAL: u32 = 32;

const SMSG_MONSTER_MOVE: u16 = 0x0DD;

fn v(x: f32, y: f32, z: f32) -> Vector3d {
    Vector3d { x, y, z }
}

/// A point `dx` east and `dy` north of the start, at its height.
fn at(dx: f32, dy: f32) -> Vector3d {
    v(START.0 + dx, START.1 + dy, START.2)
}

fn faction(id: u32, ours: u32, enemy_mask: u32, flags: u32) -> FactionTemplate {
    FactionTemplate {
        id,
        faction: id,
        ours,
        friend_mask: ours,
        enemy_mask,
        flags,
        ..Default::default()
    }
}

fn template(entry: u32, name: &str, faction: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 2,
        level_max: 2,
        faction,
        unit_class: 1,
        health_multiplier: 100.0,
        damage_multiplier: 0.01,
        base_attack_time: 2000,
        detection_range: 5.0,
        ..Default::default()
    }
}

fn spawn(guid: u32, entry: u32, p: Vector3d) -> CreatureSpawn {
    CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: p.x,
        y: p.y,
        z: p.z,
        respawn_min: 60,
        respawn_max: 60,
        ..Default::default()
    }
}

/// The factions and a level's stats, with no one in the world.
fn world() -> GameData {
    let mut d = GameData::empty();
    d.faction_templates.extend([
        (HUMANS, faction(HUMANS, 0x1, 0x2, 0)),
        (PACK, faction(PACK, 0x2, 0x1, 0x01)),
        (NEUTRAL, faction(NEUTRAL, 0, 0, 0)),
    ]);
    d.class_level_stats.insert(
        (1, 2),
        ClassLevelStats {
            health: 100,
            melee_damage: 1.0,
            ..Default::default()
        },
    );
    d
}

/// A realm on `d`, and a warrior at the start.
async fn enter(d: GameData) -> (Arc<Realm>, Client, Guid, MapHandle) {
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    (realm, a, ga, map)
}

/// A packed offset as the client unpacks it: 11, 11 and 10 bits, signed,
/// in quarters of a yard.
fn unpack(p: u32) -> Vector3d {
    let field = |bits: u32, shift: u32| {
        let raw = ((p >> shift) & ((1 << bits) - 1)) as i32;
        let signed = (raw << (32 - bits)) >> (32 - bits);
        signed as f32 * 0.25
    };
    v(field(11, 0), field(11, 11), field(10, 22))
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn vec_at(b: &[u8], at: usize) -> Vector3d {
    v(f32_at(b, at), f32_at(b, at + 4), f32_at(b, at + 8))
}

/// An `SMSG_MONSTER_MOVE` as the client reads it.
#[derive(Debug, Clone)]
struct Move {
    guid: Guid,
    start: Vector3d,
    id: u32,
    /// 0 normal, 1 stop, 2 to face a spot, 3 a target, 4 an angle.
    kind: u8,
    angle: Option<f32>,
    flags: u32,
    duration: u32,
    /// The path after the start, the destination last.
    points: Vec<Vector3d>,
}

/// Read a body as the client does: a stop ends at its type; a flyer's
/// points are whole; a walk's are its destination, then offsets from it.
/// `None` if the body isn't that long exactly.
fn decode(body: &[u8]) -> Option<Move> {
    let mask = *body.first()?;
    let mut raw = [0u8; 8];
    let mut i = 1;
    for (bit, b) in raw.iter_mut().enumerate() {
        if mask & (1 << bit) != 0 {
            *b = *body.get(i)?;
            i += 1;
        }
    }
    let guid = Guid::new(u64::from_le_bytes(raw));
    let start = vec_at(body, i);
    let id = u32_at(body, i + 12);
    let kind = body[i + 16];
    let mut at = i + 17;
    let mut angle = None;
    match kind {
        1 => {
            return (body.len() == at).then_some(Move {
                guid,
                start,
                id,
                kind,
                angle,
                flags: 0,
                duration: 0,
                points: vec![],
            });
        }
        2 => at += 12,
        3 => at += 8,
        4 => {
            angle = Some(f32_at(body, at));
            at += 4;
        }
        _ => {}
    }
    let flags = u32_at(body, at);
    let duration = u32_at(body, at + 4);
    let count = u32_at(body, at + 8) as usize;
    at += 12;
    let mut points = Vec::new();
    if flags & 0x200 != 0 {
        if body.len() != at + 12 * count {
            return None;
        }
        for k in 0..count {
            points.push(vec_at(body, at + 12 * k));
        }
    } else if count > 0 {
        if body.len() != at + 12 + 4 * (count - 1) {
            return None;
        }
        let dest = vec_at(body, at);
        for k in 1..count {
            let o = unpack(u32_at(body, at + 12 + 4 * (k - 1)));
            points.push(v(dest.x - o.x, dest.y - o.y, dest.z - o.z));
        }
        points.push(dest);
    }
    Some(Move {
        guid,
        start,
        id,
        kind,
        angle,
        flags,
        duration,
        points,
    })
}

/// The next `SMSG_MONSTER_MOVE` for `g` that `pick` takes, as the client
/// reads it.
async fn next_move(a: &mut Client, g: Guid, mut pick: impl FnMut(&Move) -> bool) -> Move {
    let body = a
        .wait_frame("SMSG_MONSTER_MOVE", SMSG_MONSTER_MOVE, |b| {
            decode(b).is_some_and(|m| m.guid == g && pick(&m))
        })
        .await;
    decode(&body).expect("it reads")
}

/// SMSG_MONSTER_MOVE as mangos-classic writes it: run mode always set (the
/// client misbehaves without it), the destination whole, and the points
/// between as packed offsets from the destination, one too near it to
/// pack left out. wow_messages reads it.
#[test]
fn a_monster_moves_points_are_offsets_from_its_destination() {
    let points = vec![
        v(0.0, 0.0, 0.0),
        v(10.0, 0.0, 1.0),
        v(10.0, 10.0, 2.0),
        // Within a yard of the destination: left out.
        v(19.7, 10.2, 2.0),
        v(20.0, 10.0, 2.0),
    ];
    let w = Walk::new(points.clone(), 2.5, Instant::now(), 7, false);
    let g = guid::unit(1, 1);
    let frame = w.frame(g, points[0]).expect("encoded");
    let ServerOpcodeMessage::SMSG_MONSTER_MOVE(m) =
        ServerOpcodeMessage::read_unencrypted(&frame[..]).expect("wow_messages reads it")
    else {
        panic!("not a monster move");
    };
    assert_eq!(m.guid, g);
    assert_eq!(m.spline_id, 7);
    assert_ne!(m.spline_flags.as_int() & 0x100, 0, "run mode, walking too");
    assert_eq!(m.spline_flags.as_int() & 0x200, 0, "not flying");
    assert_eq!(m.splines.len(), 3, "the destination and two offsets");
    assert_eq!((m.splines[0].x, m.splines[0].y), (20.0, 10.0));
    // As the client reads it.
    let read = decode(&frame[4..]).expect("the client reads it");
    assert_eq!(read.points.len(), 3);
    for (got, want) in read.points.iter().zip([points[1], points[2], points[4]]) {
        assert!(
            (got.x - want.x).abs() <= 0.25
                && (got.y - want.y).abs() <= 0.25
                && (got.z - want.z).abs() <= 0.25,
            "{got:?}, not {want:?}"
        );
    }
}

/// A flyer's spline is a Catmull-Rom one: every point after the start
/// whole (mangos-classic's `WriteCatmullRomPath`), with run mode and
/// flying set.
#[test]
fn a_flyers_spline_is_written_in_whole_points() {
    let points = vec![
        v(0.0, 0.0, 50.0),
        v(10.0, 0.0, 52.0),
        v(10.0, 10.0, 54.0),
        v(0.0, 10.0, 50.0),
    ];
    let mut w = Walk::new(points.clone(), 7.0, Instant::now(), 9, false);
    w.fly = true;
    let g = guid::unit(1, 1);
    let frame = w.frame(g, points[0]).expect("encoded");
    assert_eq!(u16::from_le_bytes([frame[2], frame[3]]), SMSG_MONSTER_MOVE);
    let m = decode(&frame[4..]).expect("as long as its points");
    assert_eq!((m.guid, m.id, m.kind), (g, 9, 0));
    assert_eq!((m.start.x, m.start.y, m.start.z), (0.0, 0.0, 50.0));
    assert_eq!(
        m.flags & 0x300,
        0x300,
        "flying, in run mode: {:#x}",
        m.flags
    );
    assert_eq!(m.duration, w.duration.as_millis() as u32);
    assert_eq!(m.points.len(), 3, "the points after the start");
    for (got, want) in m.points.iter().zip(&points[1..]) {
        assert_eq!((got.x, got.y, got.z), (want.x, want.y, want.z));
    }
}

const FLYER: u32 = 5000;

/// Through the protocol: a bat circling its spawn sends its lap in whole
/// points, on the circle.
#[tokio::test]
async fn a_circling_flyers_lap_reaches_the_client_whole() {
    let mut d = world();
    let mut bat = template(FLYER, "Bat", NEUTRAL);
    bat.inhabit_type = 4;
    d.creature_templates.insert(FLYER, bat);
    let home = v(START.0 + 8.0, START.1, START.2 + 10.0);
    let mut s = spawn(60, FLYER, home);
    s.movement_type = 1;
    s.wander_distance = 5.0;
    d.creature_spawns.insert(0, vec![s]);
    let (_realm, mut a, _ga, _map) = enter(d).await;
    let bat = guid::unit(FLYER, 60);
    let lap = next_move(&mut a, bat, |m| m.kind == 0).await;
    assert_eq!(
        lap.flags & 0x300,
        0x300,
        "flying, running: {:#x}",
        lap.flags
    );
    assert!(lap.points.len() >= 8, "a lap: {lap:?}");
    for p in &lap.points {
        let r = ((p.x - home.x).powi(2) + (p.y - home.y).powi(2)).sqrt();
        assert!(
            (r - 5.0).abs() < 0.01 && p.z == home.z,
            "on the circle: {p:?}"
        );
    }
    let _ = (
        Duration::ZERO,
        dist(home, home),
        Motion::Idle { until: None },
    );
}

static BAKE: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("nav"));

/// A realm on `d` with the real navmesh, or `None` without one.
fn realm_with_nav(d: GameData) -> Option<Arc<Realm>> {
    if !BAKE.join("Azeroth.map").exists() {
        return None;
    }
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(BAKE.clone());
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Some(Realm::new(cfg, store, startup, d, None))
}

const WOLF: u32 = 5001;

/// The walks a wanderer at rest sets out on, `tries` times from its home.
async fn wanders(map: &MapHandle, g: Guid, tries: usize) -> (Vec<Vector3d>, Vec<Duration>) {
    map.query(move |m| {
        let key = m.creature_by_guid[&g];
        let (mut walks, mut waits) = (Vec::new(), Vec::new());
        for _ in 0..tries {
            let now = m.now;
            let c = m.creature_mut(g).unwrap();
            c.position = c.home;
            c.motion = Motion::Idle { until: Some(now) };
            crate::game::map::world::step_creature_key(m, key);
            match &m.creature(g).unwrap().motion {
                Motion::Moving(w) => walks.push(w.destination()),
                Motion::Idle { until: Some(t) } => waits.push(t.duration_since(now)),
                Motion::Idle { until: None } => {}
            }
        }
        (walks, waits)
    })
    .await
    .unwrap()
}

/// The real navmesh: a wanderer's points are kept within its radius of its
/// spawn, as Thistle Tea pulls in the navmesh's random point
/// (`Pathfinding.find_random_point_around_circle`).
#[tokio::test]
async fn a_wanderer_keeps_within_its_radius() {
    let mut d = world();
    d.creature_templates
        .insert(WOLF, template(WOLF, "Wolf", NEUTRAL));
    let home = at(0.0, 0.0);
    let mut s = spawn(70, WOLF, home);
    s.movement_type = 1;
    s.wander_distance = 5.0;
    d.creature_spawns.insert(0, vec![s]);
    let Some(realm) = realm_with_nav(d) else {
        return;
    };
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (walks, _) = wanders(&map, guid::unit(WOLF, 70), 300).await;
    assert!(walks.len() > 100, "it wanders: {}", walks.len());
    let far: Vec<f32> = walks
        .iter()
        .map(|p| ((p.x - home.x).powi(2) + (p.y - home.y).powi(2)).sqrt())
        .filter(|d| *d > 5.05)
        .collect();
    assert!(far.is_empty(), "beyond its radius: {far:?}");
}

/// The real navmesh: a wanderer the navmesh has no point for waits two to
/// six seconds, rather than walking to a spot in the air.
#[tokio::test]
async fn a_wanderer_with_no_point_waits() {
    let mut d = world();
    d.creature_templates
        .insert(WOLF, template(WOLF, "Wolf", NEUTRAL));
    // High over the abbey, far from any floor.
    let home = v(START.0, START.1, 600.0);
    let mut s = spawn(71, WOLF, home);
    s.movement_type = 1;
    s.wander_distance = 5.0;
    d.creature_spawns.insert(0, vec![s]);
    let Some(realm) = realm_with_nav(d) else {
        return;
    };
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (walks, waits) = wanders(&map, guid::unit(WOLF, 71), 50).await;
    assert!(walks.is_empty(), "nowhere to go: {walks:?}");
    assert_eq!(waits.len(), 50);
    assert!(
        waits
            .iter()
            .all(|w| *w >= Duration::from_secs(2) && *w <= Duration::from_secs(6)),
        "two to six seconds: {waits:?}"
    );
}
