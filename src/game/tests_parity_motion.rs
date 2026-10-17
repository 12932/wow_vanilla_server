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

const FEAR: u32 = 92_001;
const CONFUSE: u32 = 92_002;
const ROOT: u32 = 92_003;
const STUN: u32 = 92_004;
const DISTRACT: u32 = 92_005;
const CHARGE: u32 = 92_006;

/// The test spells: auras of fear, confusion, root and stun, ten seconds
/// each, and a rogue's distract.
fn with_spells(mut d: GameData) -> GameData {
    use crate::game::data::spells::{SpellDuration, SpellEffect, SpellTemplate};
    let s = &mut d.spells;
    s.ranges.insert(1, (0.0, 30.0));
    s.durations.insert(
        9,
        SpellDuration {
            base: 10_000,
            ..Default::default()
        },
    );
    let aura = |kind: u32| SpellEffect {
        effect: 6,
        aura: kind,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    let spell = |id: u32, name: &str, e: SpellEffect| SpellTemplate {
        id,
        name: name.into(),
        range_index: 1,
        duration_index: 9,
        equipped_item_class: -1,
        effects: [e, SpellEffect::default(), SpellEffect::default()],
        ..Default::default()
    };
    // Distract as the client's data has it (1725): ten seconds on the
    // enemies within ten yards of the point.
    s.radii.insert(13, 10.0);
    let distract = SpellTemplate {
        targets: 0x40,
        ..spell(
            DISTRACT,
            "Distract",
            SpellEffect {
                effect: 69,
                base_points: 9,
                base_dice: 1,
                die_sides: 1,
                target_a: 16,
                radius_index: 13,
                ..Default::default()
            },
        )
    };
    // A warrior's charge: the run, at an enemy.
    let charge = spell(
        CHARGE,
        "Charge",
        SpellEffect {
            effect: 96,
            target_a: 6,
            ..Default::default()
        },
    );
    for t in [
        charge,
        spell(FEAR, "Fear", aura(7)),
        spell(CONFUSE, "Confuse", aura(5)),
        spell(ROOT, "Root", aura(26)),
        spell(STUN, "Stun", aura(12)),
        distract,
    ] {
        s.spells.insert(t.id, t);
    }
    d
}

fn apply(m: &mut crate::game::map::Map, caster: Guid, target: Guid, spell: u32) {
    let data = m.realm.data.clone();
    crate::game::map::auras::apply_spell(
        m,
        caster,
        target,
        data.spells.get(spell).unwrap(),
        &[(0, 0)],
        Default::default(),
    )
    .expect("it takes");
}

/// A wolf ten yards east of the start, alone.
fn wolf_world() -> GameData {
    let mut d = with_spells(world());
    d.creature_templates
        .insert(WOLF, template(WOLF, "Wolf", PACK));
    d.creature_spawns
        .insert(0, vec![spawn(80, WOLF, at(10.0, 0.0))]);
    d
}

fn wolf() -> Guid {
    guid::unit(WOLF, 80)
}

/// Its walk under way: its spline id, if it is walking.
fn walking(m: &crate::game::map::Map, g: Guid) -> Option<u32> {
    match &m.creature(g)?.motion {
        Motion::Moving(w) => Some(w.spline_id),
        Motion::Idle { .. } => None,
    }
}

/// `g` sets off on a walk to `to`, its watchers told; the walk's id.
fn send_off(m: &mut crate::game::map::Map, g: Guid, to: Vector3d) -> u32 {
    let now = m.now;
    let id = m.next_spline_id;
    m.next_spline_id += 1;
    let c = m.creature_mut(g).unwrap();
    let from = c.position;
    let frame = c.walk(vec![from, to], true, now, id).unwrap();
    m.send_to_observers(g, &frame, false);
    id
}

/// A walk of `g`'s that is over, as though its time had passed, and its
/// pause with it.
fn finish_walk(m: &mut crate::game::map::Map, g: Guid) {
    let c = m.creature_mut(g).unwrap();
    if let Motion::Moving(w) = &mut c.motion {
        w.started -= w.duration + Duration::from_secs(1);
    }
    c.engagement.next_move_at = None;
}

/// Fear stops the walk a creature is on at once, telling its watchers,
/// rather than waiting it out (Thistle Tea's `ControlMovement.transition`).
#[tokio::test]
async fn fear_stops_the_walk_under_way_at_once() {
    let (_realm, mut a, ga, map) = enter(wolf_world()).await;
    let g = wolf();
    a.sees(g).await;
    let (old, still) = map
        .query(move |m| {
            let old = send_off(m, g, at(10.0, 30.0));
            apply(m, ga, g, FEAR);
            (old, walking(m, g) == Some(old))
        })
        .await
        .unwrap();
    assert!(!still, "the walk under way is over");
    let stop = next_move(&mut a, g, |m| m.kind == 1).await;
    assert_ne!(stop.id, old);
    // Its fear runs it off at once.
    let run = map
        .query(move |m| {
            crate::game::map::creature_combat::controlled_step(m, g);
            walking(m, g)
        })
        .await
        .unwrap();
    assert!(run.is_some_and(|r| r != old), "a fear run: {run:?}");
}

/// A fleeing or feared creature rooted or stunned stops where it is, its
/// watchers told, and runs no more until it is free.
#[tokio::test]
async fn a_rooted_runner_stops_where_it_is() {
    let (_realm, mut a, ga, map) = enter(wolf_world()).await;
    let g = wolf();
    a.sees(g).await;
    for hold in [ROOT, STUN] {
        let (ran, stopped, still) = map
            .query(move |m| {
                apply(m, ga, g, FEAR);
                crate::game::map::creature_combat::controlled_step(m, g);
                let ran = walking(m, g);
                apply(m, ga, g, hold);
                let stopped = walking(m, g).is_none();
                finish_walk(m, g);
                crate::game::map::creature_combat::controlled_step(m, g);
                let still = walking(m, g).is_none();
                crate::game::map::auras::remove_spell(m, g, hold, None);
                crate::game::map::auras::remove_spell(m, g, FEAR, None);
                crate::game::map::creature_combat::controlled_step(m, g);
                (ran, stopped, still)
            })
            .await
            .unwrap();
        let ran = ran.expect("it runs in fear");
        assert!(stopped, "{hold}: it stops at once");
        assert!(still, "{hold}: and runs no more");
        let stop = next_move(&mut a, g, |m| m.kind == 1).await;
        assert_ne!(stop.id, ran);
    }
}

/// Confusion staggers a creature about where it began, not where each
/// stagger left it.
#[tokio::test]
async fn confusion_keeps_about_where_it_began() {
    let (_realm, _a, ga, map) = enter(wolf_world()).await;
    let g = wolf();
    let (begun, ends) = map
        .query(move |m| {
            let begun = m.creature(g).unwrap().position;
            apply(m, ga, g, CONFUSE);
            let mut ends = Vec::new();
            for _ in 0..40 {
                finish_walk(m, g);
                // It is where its stagger took it.
                crate::game::map::ai::advance_motion(m, g);
                crate::game::map::creature_combat::controlled_step(m, g);
                if let Motion::Moving(w) = &m.creature(g).unwrap().motion {
                    ends.push(w.destination());
                }
            }
            (begun, ends)
        })
        .await
        .unwrap();
    assert!(ends.len() > 20, "it staggers: {}", ends.len());
    let far: Vec<f32> = ends
        .iter()
        .map(|p| dist(*p, begun))
        .filter(|d| *d > 4.01)
        .collect();
    assert!(
        far.is_empty(),
        "beyond four yards of where it began: {far:?}"
    );
}

/// The real navmesh: a feared creature runs to points the navmesh has,
/// along its paths, no more than thirty yards.
#[tokio::test]
async fn fear_runs_where_the_navmesh_goes() {
    let mut d = with_spells(world());
    d.creature_templates
        .insert(WOLF, template(WOLF, "Wolf", PACK));
    d.creature_spawns
        .insert(0, vec![spawn(80, WOLF, at(0.0, 0.0))]);
    let Some(realm) = realm_with_nav(d) else {
        return;
    };
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let g = wolf();
    let runs = map
        .query(move |m| {
            apply(m, ga, g, FEAR);
            let mut runs = Vec::new();
            for _ in 0..60 {
                finish_walk(m, g);
                crate::game::map::creature_combat::controlled_step(m, g);
                if let Motion::Moving(w) = &m.creature(g).unwrap().motion {
                    runs.push(w.points.clone());
                }
                // Back where it began, to run again.
                let c = m.creature_mut(g).unwrap();
                c.position = c.home;
            }
            let nav = m.nav.as_mut().unwrap();
            runs.into_iter()
                .map(|p| {
                    let length: f32 = p.windows(2).map(|w| dist(w[0], w[1])).sum();
                    let end = *p.last().unwrap();
                    let on_mesh = nav
                        .ground(end.x, end.y, end.z)
                        .is_some_and(|z| (z - end.z).abs() < 1.0);
                    let eye = |v: Vector3d| Vector3d { z: v.z + 1.0, ..v };
                    let clear = p
                        .windows(2)
                        .all(|w| nav.line_of_sight(eye(w[0]), eye(w[1])));
                    (length, on_mesh, clear)
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    assert!(runs.len() > 30, "it runs: {}", runs.len());
    for (length, on_mesh, clear) in runs {
        assert!(length <= 30.01, "no further than thirty yards: {length}");
        assert!(on_mesh, "to a point of the navmesh");
        assert!(clear, "along the navmesh's path");
    }
}

/// The real navmesh: a feared player is run to points the navmesh has, no
/// more than thirty yards, as a creature is.
#[tokio::test]
async fn a_feared_players_runs_keep_to_the_navmesh() {
    let mut d = with_spells(world());
    d.creature_templates
        .insert(WOLF, template(WOLF, "Wolf", PACK));
    d.creature_spawns
        .insert(0, vec![spawn(80, WOLF, at(10.0, 0.0))]);
    let Some(realm) = realm_with_nav(d) else {
        return;
    };
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let runs = map
        .query(move |m| {
            apply(m, wolf(), ga, FEAR);
            let mut runs = Vec::new();
            for _ in 0..60 {
                let now = m.now;
                let p = m.players.get_mut(&ga).unwrap();
                p.movement.position = at(0.0, 0.0);
                if let Some(d) = p.driven.as_mut() {
                    d.walk = None;
                    d.next_move_at = now;
                }
                crate::game::map::player_motion::update(m);
                if let Some(w) = m.players[&ga].driven.as_ref().and_then(|d| d.walk.as_ref()) {
                    runs.push(w.points.clone());
                }
            }
            let nav = m.nav.as_mut().unwrap();
            runs.into_iter()
                .map(|p| {
                    let length: f32 = p.windows(2).map(|w| dist(w[0], w[1])).sum();
                    let end = *p.last().unwrap();
                    let on_mesh = nav
                        .ground(end.x, end.y, end.z)
                        .is_some_and(|z| (z - end.z).abs() < 1.0);
                    (length, on_mesh)
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    assert!(runs.len() > 30, "they run: {}", runs.len());
    for (length, on_mesh) in runs {
        assert!(length <= 30.01, "no further than thirty yards: {length}");
        assert!(on_mesh, "to a point of the navmesh");
    }
}

/// Distract, as Thistle Tea's `Distraction`: a creature out of a fight
/// stops, turns to the point (its watchers told of both) and stands the
/// effect's ten seconds, not one; one in a fight pays it no heed.
#[tokio::test]
async fn distract_stops_and_turns_a_creature_for_its_seconds() {
    let mut d = wolf_world();
    d.creature_spawns
        .get_mut(&0)
        .unwrap()
        .push(spawn(81, WOLF, at(16.0, 3.0)));
    let (_realm, mut a, ga, map) = enter(d).await;
    let (g, fighter) = (wolf(), guid::unit(WOLF, 81));
    a.sees(g).await;
    let point = at(15.0, 0.0);
    let (walk, walking_after, held_for, facing, fighter_held) = map
        .query(move |m| {
            crate::game::map::ai::attacked(m, fighter, ga);
            let walk = send_off(m, g, at(10.0, 20.0));
            let now = m.now;
            crate::game::map::spells::cast_triggered_at(m, ga, DISTRACT, point);
            let held = |m: &crate::game::map::Map, x: Guid| match m.creature(x).unwrap().motion {
                Motion::Idle { until: Some(t) } => Some(t.duration_since(now)),
                _ => None,
            };
            let facing = m.creature(g).unwrap().orientation;
            (walk, walking(m, g), held(m, g), facing, held(m, fighter))
        })
        .await
        .unwrap();
    assert_eq!(walking_after, None, "it stops");
    let held = held_for.expect("it is distracted");
    assert!(
        held >= Duration::from_millis(9_900) && held <= Duration::from_secs(10),
        "for ten seconds: {held:?}"
    );
    assert!(facing.abs() < 0.01, "toward the point, east: {facing}");
    assert!(
        fighter_held.is_none_or(|t| t < Duration::from_secs(1)),
        "one fighting pays it no heed: {fighter_held:?}"
    );
    let stop = next_move(&mut a, g, |m| m.kind == 1).await;
    assert_ne!(stop.id, walk);
    let face = next_move(&mut a, g, |m| m.kind == 4).await;
    assert!(face.angle.is_some_and(|o| o.abs() < 0.01), "{face:?}");
    // It stands while it lasts, as ready to wander as it may be.
    let still = map
        .query(move |m| {
            let now = m.now;
            let c = m.creature_mut(g).unwrap();
            c.movement_type = crate::game::map::creature::MOVEMENT_RANDOM;
            c.wander_distance = 5.0;
            c.motion = Motion::Idle { until: Some(now) };
            crate::game::map::ai::update_creatures(m);
            walking(m, g).is_none()
        })
        .await
        .unwrap();
    assert!(still, "it stands");
}

const SLUGGISH: u32 = 5002;

/// A creature that leashes is whole again as it turns for home, not when
/// it gets there (Thistle Tea's `heal_to_full`); one whose template says
/// it doesn't regenerate isn't, then or at home.
#[tokio::test]
async fn a_leashing_creature_is_healed_as_it_turns_for_home() {
    use crate::game::fields::{UNIT_FIELD_HEALTH, UNIT_FIELD_MAXHEALTH};
    let mut d = wolf_world();
    let mut sluggish = template(SLUGGISH, "Sluggish Wolf", PACK);
    sluggish.static_flags1 = 0x400;
    d.creature_templates.insert(SLUGGISH, sluggish);
    d.creature_spawns
        .get_mut(&0)
        .unwrap()
        .push(spawn(82, SLUGGISH, at(12.0, 0.0)));
    let (_realm, _a, ga, map) = enter(d).await;
    let health = |m: &crate::game::map::Map, g: Guid| {
        let f = &m.creature(g).unwrap().fields;
        (
            f.get_u32(UNIT_FIELD_HEALTH),
            f.get_u32(UNIT_FIELD_MAXHEALTH),
        )
    };
    for (g, regenerates) in [(wolf(), true), (guid::unit(SLUGGISH, 82), false)] {
        let (leashing, home) = map
            .query(move |m| {
                crate::game::map::ai::attacked(m, g, ga);
                let c = m.creature_mut(g).unwrap();
                c.motion = Motion::Idle { until: None };
                c.position = at(10.0, 25.0);
                let max = c.fields.get_u32(UNIT_FIELD_MAXHEALTH);
                c.fields.set_u32(UNIT_FIELD_HEALTH, max / 2);
                crate::game::map::ai::enter_evade(m, g);
                assert!(m.creature(g).unwrap().evading, "on its way home");
                let leashing = health(m, g);
                finish_walk(m, g);
                crate::game::map::ai::update_creatures(m);
                assert!(!m.creature(g).unwrap().evading, "home");
                (leashing, health(m, g))
            })
            .await
            .unwrap();
        if regenerates {
            assert_eq!(leashing.0, leashing.1, "whole as it leashes");
        } else {
            assert_eq!(leashing.0, leashing.1 / 2, "not healed as it leashes");
            assert_eq!(home.0, home.1 / 2, "nor at home");
        }
    }
}

/// Charge runs the caster to its target at four times its run speed, at
/// most 24 yards a second, to half a yard inside its reach, and it swings
/// there, not at once (Thistle Tea's `Movement.Charge`): a player's client
/// is moved by a monster move for its own guid, its swing put off by the
/// run; a creature runs the same way.
#[tokio::test]
async fn charge_runs_to_the_target_and_swings_there() {
    let mut d = wolf_world();
    d.creature_spawns.get_mut(&0).unwrap()[0] = spawn(80, WOLF, at(20.0, 0.0));
    let (_realm, mut a, ga, map) = enter(d).await;
    let g = wolf();
    a.sees(g).await;
    let (moved_at_once, attacking_at_once, swing_in) = map
        .query(move |m| {
            let now = m.now;
            crate::game::map::spells::cast_triggered(m, ga, CHARGE, Some(g));
            let p = &m.players[&ga];
            (
                dist(p.position(), at(0.0, 0.0)),
                p.combat.attacking,
                p.combat.next_swing.map(|t| t.duration_since(now)),
            )
        })
        .await
        .unwrap();
    assert!(moved_at_once < 0.01, "not teleported: {moved_at_once}");
    assert!(!attacking_at_once, "no swing until it gets there");
    // 20 yards less the reach (about 5), less half a yard: some 15.5.
    assert!(
        swing_in.is_some_and(|t| t >= Duration::from_millis(200 + 40 * 18)),
        "its swing put off by the run: {swing_in:?}"
    );
    let run = next_move(&mut a, ga, |m| m.kind == 0).await;
    assert_ne!(run.flags & 0x100, 0, "running");
    let end = *run.points.last().unwrap();
    let gap = dist(end, at(20.0, 0.0));
    assert!(
        (4.0..5.5).contains(&gap),
        "to half a yard inside its reach: {gap}"
    );
    let length = dist(at(0.0, 0.0), end);
    let ms = (length / 24.0 * 1000.0) as u32;
    assert!(
        run.duration.abs_diff(ms) <= 2,
        "at 24 yards a second: {} {ms}",
        run.duration
    );
    tokio::time::sleep(Duration::from_millis(u64::from(run.duration) + 300)).await;
    let (there, attacking, victim) = map
        .query(move |m| {
            let p = &m.players[&ga];
            (dist(p.position(), end), p.combat.attacking, p.combat.victim)
        })
        .await
        .unwrap();
    assert!(there < 0.5, "there: {there}");
    assert!(attacking && victim == g, "and swinging at it");
    // A creature charges the same way.
    let (wolf_at_once, wolf_charging) = map
        .query(move |m| {
            let c = m.creature_mut(g).unwrap();
            c.position = at(20.0, 20.0);
            c.motion = Motion::Idle { until: None };
            crate::game::map::spells::cast_triggered(m, g, CHARGE, Some(ga));
            let c = m.creature(g).unwrap();
            (dist(c.position, at(20.0, 20.0)), walking(m, g).is_some())
        })
        .await
        .unwrap();
    assert!(wolf_at_once < 0.01 && wolf_charging, "the wolf runs");
    let wolf_run = next_move(&mut a, g, |m| m.kind == 0 && m.duration > 0).await;
    tokio::time::sleep(Duration::from_millis(u64::from(wolf_run.duration) + 300)).await;
    let (charging, fighting) = map
        .query(move |m| {
            let c = m.creature(g).unwrap();
            let near = dist(c.position, m.players[&ga].position()) < 5.0;
            (!near, c.combat.attacking && c.combat.victim == ga)
        })
        .await
        .unwrap();
    assert!(!charging && fighting, "it gets there and fights");
}

/// Scripted following, as Thistle Tea's `BT.Follow`: the spot is off where
/// the leader will be half a second on; the follower runs (a walking
/// leader too) and faces as the leader does when there; a leader out of
/// sight is waited for, not given up.
#[tokio::test]
async fn a_scripted_follower_runs_to_where_its_leader_will_be() {
    let (_realm, mut a, ga, map) = enter(wolf_world()).await;
    let g = wolf();
    a.sees(g).await;
    let leader_at = at(0.0, 0.0);
    let (dest, speed, run_speed) = map
        .query(move |m| {
            let p = m.players.get_mut(&ga).unwrap();
            p.movement.position = leader_at;
            p.movement.orientation = 0.0;
            p.movement.flags = MovementInfo_MovementFlags::new_forward().set_walk_mode();
            let c = m.creature_mut(g).unwrap();
            c.movement_type = crate::game::map::creature::MOVEMENT_IDLE;
            c.motion = Motion::Idle { until: None };
            c.scripted.get_or_insert_with(Default::default).follow = Some((ga, 2.0, 0.0));
            crate::game::map::event_ai::follow_step(m, g);
            let c = m.creature(g).unwrap();
            match &c.motion {
                Motion::Moving(w) => (Some(w.destination()), w.speed, c.speeds.run),
                Motion::Idle { .. } => (None, 0.0, c.speeds.run),
            }
        })
        .await
        .unwrap();
    let dest = dest.expect("it follows");
    // Walking east at 2.5 yards a second: 1.25 yards on, and two past that.
    let want = at(1.25 + 2.0, 0.0);
    assert!(dist(dest, want) < 0.1, "where the leader will be: {dest:?}");
    assert!(speed >= run_speed, "running: {speed} {run_speed}");
    let walk = next_move(&mut a, g, |m| m.kind != 1).await;
    assert_eq!(walk.kind, 4, "facing as its leader does: {walk:?}");
    assert!(walk.angle.is_some_and(|o| o.abs() < 0.01));
    // A leader not here is waited for.
    let ghost = guid::unit(WOLF, 999);
    let still = map
        .query(move |m| {
            let c = m.creature_mut(g).unwrap();
            c.scripted.get_or_insert_with(Default::default).follow = Some((ghost, 2.0, 0.0));
            crate::game::map::event_ai::follow_step(m, g);
            m.creature(g)
                .and_then(|c| c.scripted.as_deref())
                .and_then(|s| s.follow)
        })
        .await
        .unwrap();
    assert_eq!(still.map(|f| f.0), Some(ghost), "still following");
}

const WALKER: u32 = 5003;

/// A leader (spawn 90) and its follower (91) a pace behind, in formation
/// with `flags`; a friend of theirs (92) by the follower; the walker
/// neutral, the friend of the pack.
fn group_world(flags: u32) -> GameData {
    use crate::game::data::scripts::{GROUP_FORMATION, GroupMember};
    let mut d = wolf_world();
    let mut walker = template(WALKER, "Walker", PACK);
    walker.call_for_help_range = 5.0;
    d.creature_templates.insert(WALKER, walker);
    let spawns = d.creature_spawns.get_mut(&0).unwrap();
    spawns.push(spawn(90, WALKER, at(20.0, -40.0)));
    spawns.push(spawn(91, WALKER, at(17.0, -40.0)));
    spawns.push(spawn(92, WALKER, at(14.0, -46.0)));
    let member = |guid, distance, angle| GroupMember {
        guid,
        distance,
        angle,
        flags: GROUP_FORMATION | flags,
    };
    d.scripts.groups.insert(
        90,
        vec![member(90, 0.0, 0.0), member(91, 3.0, std::f32::consts::PI)],
    );
    d.scripts.group_of.insert(90, 90);
    d.scripts.group_of.insert(91, 90);
    d
}

fn leader() -> Guid {
    guid::unit(WALKER, 90)
}

fn follower() -> Guid {
    guid::unit(WALKER, 91)
}

/// A formation keeps its place off where its leader's walk ends, faced
/// along the walk's last leg (not the leader's old facing), timed to
/// arrive with it and facing that way when there; one a hundred yards
/// behind is brought up to its leader (Thistle Tea's `BT.Formation`).
#[tokio::test]
async fn a_formation_follows_the_last_leg_and_arrives_with_its_leader() {
    let (_realm, mut a, _ga, map) = enter(group_world(0)).await;
    let (l, f) = (leader(), follower());
    a.sees(f).await;
    let north = at(20.0, -30.0);
    let (dest, leader_ends, follower_ends) = map
        .query(move |m| {
            let now = m.now;
            m.creature_mut(l).unwrap().orientation = 0.0;
            m.creature_mut(f).unwrap().position = at(20.0, -45.0);
            let c = m.creature_mut(l).unwrap();
            let from = c.position;
            c.walk(vec![from, north], false, now, 1);
            crate::game::map::creature_groups::leader_walked(m, l);
            let ends = |m: &crate::game::map::Map, g: Guid| match &m.creature(g).unwrap().motion {
                Motion::Moving(w) => Some((w.destination(), w.started + w.duration)),
                Motion::Idle { .. } => None,
            };
            let (dest, follower_ends) = ends(m, f).expect("it follows");
            (dest, ends(m, l).unwrap().1, follower_ends)
        })
        .await
        .unwrap();
    assert!(
        dist(dest, at(20.0, -33.0)) < 0.1,
        "behind it along its way north: {dest:?}"
    );
    let apart = if leader_ends > follower_ends {
        leader_ends - follower_ends
    } else {
        follower_ends - leader_ends
    };
    assert!(
        apart < Duration::from_millis(50),
        "arriving together: {apart:?}"
    );
    let walk = next_move(&mut a, f, |m| m.kind != 1).await;
    assert_eq!(walk.kind, 4, "facing along the way: {walk:?}");
    assert!(
        walk.angle
            .is_some_and(|o| (o - std::f32::consts::FRAC_PI_2).abs() < 0.01)
    );
    // Far behind, it is brought up at once.
    let (near, moving) = map
        .query(move |m| {
            let now = m.now;
            let c = m.creature_mut(f).unwrap();
            c.motion = Motion::Idle { until: None };
            c.position = at(20.0, -160.0);
            let c = m.creature_mut(l).unwrap();
            c.motion = Motion::Idle { until: None };
            let from = c.position;
            c.walk(vec![from, at(20.0, -20.0)], false, now, 2);
            crate::game::map::creature_groups::leader_walked(m, l);
            let leader_at = m.creature(l).unwrap().position;
            (
                dist(m.creature(f).unwrap().position, leader_at),
                walking(m, f).is_some(),
            )
        })
        .await
        .unwrap();
    assert!(near < 0.1 && !moving, "brought up to its leader: {near}");
}

/// A leader that flies leads its formation as one on foot does.
#[tokio::test]
async fn a_flying_leader_leads_its_formation() {
    let mut d = group_world(0);
    let mut bat = template(FLYER, "Bat", NEUTRAL);
    bat.inhabit_type = 4;
    d.creature_templates.insert(FLYER, bat);
    let spawns = d.creature_spawns.get_mut(&0).unwrap();
    let mut s = spawn(93, FLYER, v(START.0 + 20.0, START.1 - 40.0, START.2 + 10.0));
    s.movement_type = 1;
    s.wander_distance = 5.0;
    spawns.push(s);
    d.scripts.groups.get_mut(&90).unwrap()[0].guid = 93;
    let groups = d.scripts.groups.remove(&90).unwrap();
    d.scripts.groups.insert(93, groups);
    d.scripts.group_of.insert(93, 93);
    d.scripts.group_of.insert(91, 93);
    let (_realm, _a, _ga, map) = enter(d).await;
    let bat = guid::unit(FLYER, 93);
    let follows = map
        .query(move |m| {
            let key = m.creature_by_guid[&bat];
            let now = m.now;
            m.creature_mut(bat).unwrap().motion = Motion::Idle { until: Some(now) };
            crate::game::map::creature_motion::idle_step(m, key);
            walking(m, bat).is_some() && walking(m, follower()).is_some()
        })
        .await
        .unwrap();
    assert!(follows, "its follower keeps its place");
}

/// A group's flags as Thistle Tea's `CreatureGroup` reads them: any
/// member's respawn brings back its dead (0x08), an evade does (0x20, or
/// 0x10 for the leader's), and members pulled in with the group call no
/// one themselves.
#[tokio::test]
async fn a_group_respawns_and_pulls_as_its_flags_say() {
    use crate::game::data::scripts::{GROUP_AGGRO_TOGETHER, GROUP_RESPAWN_TOGETHER};
    let (l, f) = (leader(), follower());
    let kill = |m: &mut crate::game::map::Map, g: Guid| {
        crate::game::map::ai::die(m, g, Guid::zero());
    };
    let alive = |m: &crate::game::map::Map, g: Guid| m.creature(g).is_some_and(|c| c.alive());
    // Respawning together, from the follower's respawn.
    let (_realm, _a, _ga, map) = enter(group_world(GROUP_RESPAWN_TOGETHER)).await;
    let back = map
        .query(move |m| {
            kill(m, l);
            kill(m, f);
            crate::game::map::ai::respawn_now(m, f);
            alive(m, l)
        })
        .await
        .unwrap();
    assert!(back, "the leader comes back with its follower");
    for (flag, leader_evades) in [(0x20, false), (0x10, true)] {
        let (_realm, _a, ga, map) = enter(group_world(flag)).await;
        let back = map
            .query(move |m| {
                let (evader, dead) = if leader_evades { (l, f) } else { (f, l) };
                kill(m, dead);
                crate::game::map::ai::attacked(m, evader, ga);
                crate::game::map::ai::enter_evade(m, evader);
                alive(m, dead)
            })
            .await
            .unwrap();
        assert!(back, "{flag:#x}: its dead come back with the evade");
    }
    // Pulled with the group, the follower calls no friend of its own.
    let (_realm, _a, ga, map) = enter(group_world(GROUP_AGGRO_TOGETHER)).await;
    let (joined, calling) = map
        .query(move |m| {
            crate::game::map::ai::attacked(m, l, ga);
            let c = m.creature(f).unwrap();
            (c.combat.attacking, !c.engagement.assist.is_empty())
        })
        .await
        .unwrap();
    assert!(joined, "it joins its leader's fight");
    assert!(!calling, "calling no one");
}

/// A follower goes home from a fight to its place by its leader, not to
/// its own spawn (Thistle Tea's `Formation.home_position`).
#[tokio::test]
async fn a_follower_goes_home_to_its_place_by_its_leader() {
    let (_realm, _a, ga, map) = enter(group_world(0)).await;
    let (l, f) = (leader(), follower());
    let home = map
        .query(move |m| {
            let c = m.creature_mut(l).unwrap();
            c.position = at(30.0, -40.0);
            c.orientation = std::f32::consts::FRAC_PI_2;
            crate::game::map::ai::attacked(m, f, ga);
            let c = m.creature_mut(f).unwrap();
            c.motion = Motion::Idle { until: None };
            c.position = at(17.0, -60.0);
            crate::game::map::ai::enter_evade(m, f);
            match &m.creature(f).unwrap().motion {
                Motion::Moving(w) => Some(w.destination()),
                Motion::Idle { .. } => None,
            }
        })
        .await
        .unwrap()
        .expect("on its way");
    // Three yards behind a leader facing north.
    assert!(dist(home, at(30.0, -43.0)) < 0.1, "{home:?}");
}

/// A formation's leader dies on its patrol: its first member alive leads
/// on along the patrol from the point the leader was bound for, and falls
/// back in behind it when it is back (Thistle Tea's
/// `CreatureGroup.on_death`, `on_respawn`).
#[tokio::test]
async fn a_patrol_whose_leader_dies_is_led_on_by_a_member() {
    let mut d = group_world(0);
    let point = |point, p: Vector3d| Waypoint {
        point,
        x: p.x,
        y: p.y,
        z: p.z,
        ..Default::default()
    };
    d.waypoints.insert(
        90,
        vec![
            point(1, at(20.0, -40.0)),
            point(2, at(30.0, -40.0)),
            point(3, at(30.0, -30.0)),
        ],
    );
    let (_realm, _a, _ga, map) = enter(d).await;
    let (l, f) = (leader(), follower());
    let (acting, kind, bound, walks_to) = map
        .query(move |m| {
            let c = m.creature_mut(l).unwrap();
            c.motion = Motion::Idle { until: None };
            c.waypoint = 1;
            crate::game::map::ai::die(m, l, Guid::zero());
            let acting = m.acting_leaders.get(&90).copied();
            let c = m.creature(f).unwrap();
            let (kind, bound) = (c.movement_type, c.waypoint);
            let now = m.now;
            m.creature_mut(f).unwrap().motion = Motion::Idle { until: Some(now) };
            let key = m.creature_by_guid[&f];
            crate::game::map::world::step_creature_key(m, key);
            let walks_to = match &m.creature(f).unwrap().motion {
                Motion::Moving(w) => Some(w.destination()),
                Motion::Idle { .. } => None,
            };
            (acting, kind, bound, walks_to)
        })
        .await
        .unwrap();
    assert_eq!(acting, Some(f), "its member leads on");
    assert_eq!(kind, crate::game::map::creature::MOVEMENT_WAYPOINT);
    assert_eq!(bound, 1, "from the point the leader was bound for");
    let to = walks_to.expect("along the patrol");
    assert!(dist(to, at(30.0, -40.0)) < 0.1, "{to:?}");
    let (acting, kind) = map
        .query(move |m| {
            crate::game::map::ai::respawn_now(m, l);
            (
                m.acting_leaders.get(&90).copied(),
                m.creature(f).unwrap().movement_type,
            )
        })
        .await
        .unwrap();
    assert_eq!(acting, None, "the leader back leads again");
    assert_eq!(
        kind,
        crate::game::map::creature::MOVEMENT_IDLE,
        "and its member follows"
    );
}
