//! Creature combat as Thistle Tea runs it (docs/AI5_GAPS.md, "Creature
//! combat"): the chase and where it ends, `SMSG_ATTACKSTART`'s timing,
//! facing packets in a fight.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::combat::{self, dist};
use crate::game::map::creature::Motion;
use crate::game::map::{Map, MapHandle, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

const HUMANS: u32 = 1;
/// Hostile to humans.
const PACK: u32 = 14;

const KOBOLD: u32 = 4100;
/// A kobold that doesn't regenerate.
const WORN: u32 = 4101;

/// The spawns: (guid, entry, east, north).
const SPAWNS: &[(u32, u32, f32, f32)] = &[
    // Beside the start.
    (10, KOBOLD, 4.0, 0.0),
    // Off to the east, out of sight: one sitting at its fire, one worn.
    (11, KOBOLD, 30.0, 0.0),
    (12, WORN, 30.0, 6.0),
];

fn faction(id: u32, ours: u32, enemy_mask: u32) -> FactionTemplate {
    FactionTemplate {
        id,
        faction: id,
        ours,
        friend_mask: ours,
        enemy_mask,
        ..Default::default()
    }
}

fn creature(entry: u32, name: &str, faction: u32) -> CreatureTemplate {
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
        detection_range: 1.0,
        // A little slower than a running player: one who runs gets away.
        speed_run: 0.8,
        ..Default::default()
    }
}

fn at(dx: f32, dy: f32) -> Vector3d {
    Vector3d {
        x: START.0 + dx,
        y: START.1 + dy,
        z: START.2,
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.faction_templates.extend([
        (HUMANS, faction(HUMANS, 0x1, 0x2)),
        (PACK, faction(PACK, 0x2, 0x1)),
    ]);
    d.class_level_stats.insert(
        (1, 2),
        ClassLevelStats {
            health: 100,
            melee_damage: 1.0,
            ..Default::default()
        },
    );
    d.creature_templates
        .insert(KOBOLD, creature(KOBOLD, "Kobold", PACK));
    let mut worn = creature(WORN, "Worn Kobold", PACK);
    worn.static_flags1 = 0x400;
    d.creature_templates.insert(WORN, worn);
    // Sitting, weapon drawn, eating.
    d.creature_addons.insert(
        11,
        CreatureAddon {
            stand_state: 1,
            sheath_state: 1,
            emote_state: 10,
            ..Default::default()
        },
    );
    let spawns = SPAWNS
        .iter()
        .map(|(g, e, dx, dy)| {
            let p = at(*dx, *dy);
            CreatureSpawn {
                guid: *g,
                entries: vec![*e],
                map: 0,
                x: p.x,
                y: p.y,
                z: p.z,
                respawn_min: 60,
                respawn_max: 60,
                ..Default::default()
            }
        })
        .collect();
    d.creature_spawns.insert(0, spawns);
    d
}

async fn setup(class: Class) -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, class).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    a.sees(kobold(10)).await;
    // The heartbeat is in before a test moves the player itself.
    while map
        .query(move |m| dist(m.players[&ga].position(), at(0.0, 0.0)) > 0.01)
        .await
        .unwrap()
    {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    (realm, a, ga, map)
}

fn kobold(spawn: u32) -> Guid {
    guid::unit(KOBOLD, spawn)
}

/// The melee range between two units as the client measures it.
fn reach(m: &Map, a: Guid, b: Guid) -> f32 {
    combat::melee_range(&m.unit_view(a).unwrap(), &m.unit_view(b).unwrap())
}

/// The map's creatures take `steps` turns of 50 ms.
fn step(m: &mut Map, steps: u32) {
    for _ in 0..steps {
        m.now += Duration::from_millis(50);
        crate::game::map::ai::update_creatures(m);
    }
}

fn movement(x: f32, orientation: f32, forward: bool) -> MovementInfo {
    MovementInfo {
        flags: if forward {
            MovementInfo_MovementFlags::new_forward()
        } else {
            MovementInfo_MovementFlags::empty()
        },
        timestamp: 0,
        position: Vector3d {
            x,
            y: START.1,
            z: START.2,
        },
        orientation,
        fall_time: 0.0,
    }
}

/// What a client saw of a creature: its moves (where from, where to, how
/// long), its swings at the client's player, as they came.
#[derive(Debug, Clone, Copy)]
enum Seen {
    Move {
        at: Instant,
        from: Vector3d,
        to: Vector3d,
        duration: Duration,
        stop: bool,
    },
    Swing(Instant),
}

async fn watch(c: &mut Client, creature: Guid, me: Guid, within: Duration) -> Vec<Seen> {
    let deadline = Instant::now() + within;
    c.raw.clear();
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        let got = c
            .gets(Duration::from_millis(3), |m| match m {
                ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv) if mv.guid == creature => {
                    let stop = matches!(mv.move_type, SMSG_MONSTER_MOVE_MonsterMoveType::Stop);
                    Some(Seen::Move {
                        at: Instant::now(),
                        from: mv.spline_point,
                        to: mv.splines.first().copied().unwrap_or(mv.spline_point),
                        duration: Duration::from_millis(u64::from(mv.duration)),
                        stop,
                    })
                }
                ServerOpcodeMessage::SMSG_ATTACKERSTATEUPDATE(s)
                    if s.attacker == creature && s.target == me =>
                {
                    Some(Seen::Swing(Instant::now()))
                }
                _ => None,
            })
            .await;
        seen.extend(got);
        // What wow_messages can't read comes raw: a crit's swing (its hit
        // info), and a stop (1.12's ends at its type).
        for f in std::mem::take(&mut c.raw) {
            let opcode = u16::from_le_bytes([f[2], f[3]]);
            if opcode == combat::SMSG_ATTACKERSTATEUPDATE
                && packed_guids(&f[8..]) == Some((creature, me))
            {
                seen.push(Seen::Swing(Instant::now()));
            }
            if opcode == crate::game::map::ai::SMSG_MONSTER_MOVE
                && let Some((g, at)) = stop_of(&f[4..])
                && g == creature
            {
                seen.push(Seen::Move {
                    at: Instant::now(),
                    from: at,
                    to: at,
                    duration: Duration::ZERO,
                    stop: true,
                });
            }
        }
    }
    seen
}

/// A packed guid at `b[*i..]`, moving `i` past it.
fn packed_guid(b: &[u8], i: &mut usize) -> Option<Guid> {
    let mask = *b.get(*i)?;
    *i += 1;
    let mut v = 0u64;
    for bit in 0..8 {
        if mask & (1 << bit) != 0 {
            v |= u64::from(*b.get(*i)?) << (bit * 8);
            *i += 1;
        }
    }
    Some(Guid::new(v))
}

/// The two packed guids at the start of `b`.
fn packed_guids(b: &[u8]) -> Option<(Guid, Guid)> {
    let mut i = 0;
    Some((packed_guid(b, &mut i)?, packed_guid(b, &mut i)?))
}

/// A stop's monster move body: whose, and where.
fn stop_of(b: &[u8]) -> Option<(Guid, Vector3d)> {
    let mut i = 0;
    let g = packed_guid(b, &mut i)?;
    let f = |k: usize| {
        Some(f32::from_le_bytes(
            b.get(i + k..i + k + 4)?.try_into().ok()?,
        ))
    };
    let at = Vector3d {
        x: f(0)?,
        y: f(4)?,
        z: f(8)?,
    };
    (*b.get(i + 16)? == 1).then_some((g, at))
}

/// Where the client shows the creature at `t`: along its last move, as it
/// plays it from when it came.
fn shown_at(seen: &[Seen], t: Instant) -> Option<Vector3d> {
    let (at, from, to, duration) = seen.iter().rev().find_map(|s| match *s {
        Seen::Move {
            at,
            from,
            to,
            duration,
            ..
        } if at <= t => Some((at, from, to, duration)),
        _ => None,
    })?;
    let f = if duration.is_zero() {
        1.0
    } else {
        (t.duration_since(at).as_secs_f32() / duration.as_secs_f32()).min(1.0)
    };
    Some(Vector3d {
        x: from.x + (to.x - from.x) * f,
        y: from.y + (to.y - from.y) * f,
        z: from.z + (to.z - from.z) * f,
    })
}

/// The kobold's reach (the user's report): a rogue who strikes a kobold,
/// runs 25 yards and stops is chased, and the kobold's run ends where it
/// aimed, half a yard inside its reach, as the client plays it: it doesn't
/// stop on the reach's edge, where the client, showing a creature on the
/// move a little behind, would have it out of reach. Each swing after
/// comes from within reach as the client shows the kobold.
#[tokio::test]
async fn a_chased_rogue_is_struck_only_from_within_reach() {
    let (_realm, mut a, ga, map) = setup(Class::Rogue).await;
    let kobold = kobold(10);
    a.send(CMSG_ATTACKSWING { guid: kobold }).await;
    a.wait_for("the kobold's first swing", |m| match m {
        ServerOpcodeMessage::SMSG_ATTACKERSTATEUPDATE(s) if s.attacker == kobold => Some(()),
        _ => None,
    })
    .await;
    a.send(CMSG_ATTACKSTOP {}).await;
    // Run 25 yards west at 7 yards a second, and stop.
    let west = std::f32::consts::PI;
    a.send(MSG_MOVE_START_FORWARD_Client {
        info: movement(START.0, west, true),
    })
    .await;
    let mut x = START.0;
    for _ in 0..36 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        x -= 0.7;
        a.send(MSG_MOVE_HEARTBEAT_Client {
            info: movement(x, west, true),
        })
        .await;
    }
    a.clear();
    a.send(MSG_MOVE_STOP_Client {
        info: movement(x, west, false),
    })
    .await;
    let stopped = Instant::now();
    let seen = watch(&mut a, kobold, ga, Duration::from_secs(5)).await;
    let me = Vector3d {
        x,
        y: START.1,
        z: START.2,
    };
    let swings: Vec<Instant> = seen
        .iter()
        .filter_map(|s| match s {
            Seen::Swing(t) if *t > stopped + Duration::from_millis(150) => Some(*t),
            _ => None,
        })
        .collect();
    assert!(swings.len() >= 2, "it fights on: {seen:?}");
    let first = swings[0];
    let stops = seen
        .iter()
        .filter(|s| matches!(s, Seen::Move { at, stop: true, .. } if *at < first))
        .count();
    assert_eq!(
        stops, 0,
        "its run ends where it aimed, not stopped short: {seen:?}"
    );
    for t in &swings {
        // A little after, as the client shows the swing.
        let shown = shown_at(&seen, *t + Duration::from_millis(20)).expect("seen moving");
        assert!(
            dist(shown, me) <= 5.0 + 0.01,
            "a swing from {} yards: {seen:?}",
            dist(shown, me)
        );
    }
    let (resting, range) = map
        .query(move |m| {
            let c = m.creature(kobold).unwrap();
            (
                dist(c.position, m.players[&ga].position()),
                reach(m, kobold, ga),
            )
        })
        .await
        .unwrap();
    assert!(resting <= range, "it rests in reach: {resting} of {range}");
}

/// The chase's distances (Thistle Tea's `chase_target_distance` and
/// `chase_rechase_distance`): it aims half a yard inside its reach, and
/// paths again only once its victim is three quarters of the reach (less
/// its bounding radius) from where it aimed, or it has stopped.
#[tokio::test]
async fn a_chaser_aims_inside_its_reach_and_paths_again_only_when_its_victim_has_gone() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let kobold = kobold(10);
    let out = map
        .query(move |m| {
            let spline = |m: &Map| match &m.creature(kobold).unwrap().motion {
                Motion::Moving(w) => Some((w.spline_id, w.destination())),
                Motion::Idle { .. } => None,
            };
            let set = |m: &mut Map, dx: f32, dy: f32| {
                m.players.get_mut(&ga).unwrap().movement.position = at(dx, dy);
            };
            set(m, -16.0, 0.0);
            crate::game::map::ai::attacked(m, kobold, ga);
            step(m, 1);
            let range = reach(m, kobold, ga);
            let (first, goal) = spline(m).expect("it chases");
            let aimed = dist(goal, at(-16.0, 0.0));
            // Two yards: within three quarters of its reach, less the
            // victim's bounding radius.
            set(m, -16.0, 2.0);
            step(m, 1);
            let kept = spline(m).map(|s| s.0) == Some(first);
            set(m, -16.0, 4.0);
            step(m, 1);
            let again = spline(m).is_some_and(|s| s.0 != first);
            (range, aimed, kept, again)
        })
        .await
        .unwrap();
    let (range, aimed, kept, again) = out;
    assert!(
        (aimed - combat::chase_distance(range)).abs() < 0.05,
        "aims {aimed} of {range}"
    );
    assert!(kept, "two yards on, the same path");
    assert!(again, "four yards on, a new one");
}

#[test]
fn the_chase_distances_are_thistle_teas() {
    assert_eq!(combat::chase_distance(5.0), 4.5);
    assert_eq!(combat::chase_distance(0.3), 0.0);
    assert!((combat::rechase_distance(5.0, 0.388) - 3.362).abs() < 1e-4);
    assert_eq!(combat::rechase_distance(0.4, 1.0), 0.0);
}

/// What came to a client of a creature's moves and attack starts, in order.
async fn moves_and_starts(c: &mut Client, creature: Guid) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(s) = c
        .gets(Duration::from_millis(200), |m| match m {
            ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv) if mv.guid == creature => {
                let goes = mv
                    .splines
                    .first()
                    .is_some_and(|to| dist(*to, mv.spline_point) > 0.1);
                Some(match (&mv.move_type, goes) {
                    (SMSG_MONSTER_MOVE_MonsterMoveType::Stop, _) => "stop".to_owned(),
                    (SMSG_MONSTER_MOVE_MonsterMoveType::FacingTarget { target }, false) => {
                        format!("face {}", target.guid())
                    }
                    (SMSG_MONSTER_MOVE_MonsterMoveType::FacingTarget { target }, true) => {
                        format!("move facing {}", target.guid())
                    }
                    _ => "move".to_owned(),
                })
            }
            ServerOpcodeMessage::SMSG_ATTACKSTART(s) if s.attacker == creature => {
                Some(format!("start {}", s.victim.guid()))
            }
            _ => None,
        })
        .await
    {
        out.push(s);
    }
    out
}

/// `SMSG_ATTACKSTART` goes out at the creature's first swing in reach, not
/// as the fight begins, and again for a new victim once it reaches that
/// one (Thistle Tea's `attack_started`).
#[tokio::test]
async fn attack_start_comes_with_the_first_swing_in_reach_and_for_each_new_victim() {
    let (realm, mut a, ga, map) = setup(Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0 - 12.0, START.1, START.2).await;
    let kobold = kobold(10);
    a.clear();
    let out = map
        .query(move |m| {
            m.players.get_mut(&ga).unwrap().movement.position = at(-6.0, 0.0);
            m.players.get_mut(&gb).unwrap().movement.position = at(-12.0, 6.0);
            crate::game::map::ai::attacked(m, kobold, ga);
            step(m, 40);
            let reached_a =
                combat::in_melee_range(&m.unit_view(kobold).unwrap(), &m.unit_view(ga).unwrap());
            // Bob takes its attention.
            crate::game::map::ai::add_threat(m, kobold, gb, 1000.0);
            step(m, 60);
            let c = m.creature(kobold).unwrap();
            (reached_a, c.combat.victim == gb)
        })
        .await
        .unwrap();
    let (reached_a, on_bob) = out;
    assert!(reached_a && on_bob, "{out:?}");
    let seen = moves_and_starts(&mut a, kobold).await;
    let start_a = format!("start {}", ga.guid());
    let start_b = format!("start {}", gb.guid());
    let first_move = seen
        .iter()
        .position(|s| s.starts_with("move"))
        .expect("it chases");
    let at_a = seen
        .iter()
        .position(|s| *s == start_a)
        .expect("an attack start");
    let at_b = seen
        .iter()
        .position(|s| *s == start_b)
        .expect("an attack start on Bob");
    assert!(first_move < at_a, "after it closed in: {seen:?}");
    assert!(
        seen[at_a + 1..at_b].iter().any(|s| s.starts_with("move")),
        "Bob's after it went to him: {seen:?}"
    );
}

/// A creature fighting faces its victim, and tells its observers (Thistle
/// Tea's `face_target` and `face_melee_victim`): its chase faces its
/// victim, and standing, it turns to its victim as its swing comes due.
#[tokio::test]
async fn a_fighting_creature_turns_to_face_its_victim() {
    let (_realm, mut a, ga, map) = setup(Class::Warrior).await;
    let kobold = kobold(10);
    let set = |dx: f32, dy: f32| {
        move |m: &mut Map| {
            m.players.get_mut(&ga).unwrap().movement.position = at(dx, dy);
        }
    };
    map.query(set(-6.0, 0.0)).await;
    a.clear();
    let settled = map
        .query(move |m| {
            crate::game::map::ai::attacked(m, kobold, ga);
            step(m, 60);
            matches!(m.creature(kobold).unwrap().motion, Motion::Idle { .. })
        })
        .await
        .unwrap();
    assert!(settled, "it stands to fight");
    let chased = moves_and_starts(&mut a, kobold).await;
    let chase = format!("move facing {}", ga.guid());
    assert!(
        chased.contains(&chase) && chased.iter().all(|s| s != "move"),
        "its chase faces its victim: {chased:?}"
    );
    // Its victim steps round it, still in reach: as its swing comes due,
    // it turns.
    let turned = map
        .query(move |m| {
            let k = m.creature(kobold).unwrap().position;
            m.players.get_mut(&ga).unwrap().movement.position = Vector3d { y: k.y + 3.0, ..k };
            step(m, 50);
            let c = m.creature(kobold).unwrap();
            (c.orientation - std::f32::consts::FRAC_PI_2).abs() < 0.05
        })
        .await
        .unwrap();
    assert!(turned, "it faces north, at its victim");
    let seen = moves_and_starts(&mut a, kobold).await;
    assert!(
        seen.contains(&format!("face {}", ga.guid())),
        "told: {seen:?}"
    );
}

fn posture(m: &Map, g: Guid) -> (u8, u8, u32) {
    let f = m.unit_fields(g).unwrap();
    (
        f.get_byte(UNIT_FIELD_BYTES_1, 0),
        f.get_byte(UNIT_FIELD_BYTES_2, 0),
        f.get_u32(UNIT_NPC_EMOTESTATE),
    )
}

/// A creature drawn into a fight stands up and stops its idle emote, and
/// home again sits back down to it (Thistle Tea's `Emote.reset` and
/// `Posture.restore`).
#[tokio::test]
async fn a_fight_stands_a_creature_up_and_home_it_rests_again() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let sitting = kobold(11);
    let out = map
        .query(move |m| {
            let resting = posture(m, sitting);
            m.players.get_mut(&ga).unwrap().movement.position = at(24.0, 0.0);
            crate::game::map::ai::attacked(m, sitting, ga);
            let fighting = posture(m, sitting);
            step(m, 20);
            crate::game::map::ai::enter_evade(m, sitting);
            step(m, 100);
            let c = m.creature(sitting).unwrap();
            (resting, fighting, c.evading, posture(m, sitting))
        })
        .await
        .unwrap();
    let (resting, fighting, evading, home) = out;
    assert_eq!(resting, (1, 1, 10), "it sits, eating");
    assert_eq!((fighting.0, fighting.2), (0, 0), "it stands to fight");
    assert!(!evading, "home");
    assert_eq!(home, resting, "and sits again");
}

/// A creature is whole again as it leashes, not once it is home, and only
/// one that regenerates (Thistle Tea's `heal_to_full`).
#[tokio::test]
async fn a_leashing_creature_is_healed_as_it_goes_if_it_regenerates() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (fresh, worn) = (kobold(11), guid::unit(WORN, 12));
    let out = map
        .query(move |m| {
            m.players.get_mut(&ga).unwrap().movement.position = at(24.0, 3.0);
            let health = |m: &Map, g: Guid| {
                let f = m.unit_fields(g).unwrap();
                (
                    f.get_u32(UNIT_FIELD_HEALTH),
                    f.get_u32(UNIT_FIELD_MAXHEALTH),
                )
            };
            let mut out = Vec::new();
            for g in [fresh, worn] {
                crate::game::map::ai::attacked(m, g, ga);
                step(m, 20);
                let max = health(m, g).1;
                m.unit_fields_mut(g)
                    .unwrap()
                    .set_u32(UNIT_FIELD_HEALTH, max / 2);
                crate::game::map::ai::enter_evade(m, g);
                let going = (m.creature(g).unwrap().evading, health(m, g).0);
                step(m, 100);
                let home = (m.creature(g).unwrap().evading, health(m, g).0);
                out.push((max, going, home));
            }
            out
        })
        .await
        .unwrap();
    let (max, going, home) = out[0];
    assert_eq!(going, (true, max), "whole as it goes");
    assert_eq!(home, (false, max));
    let (max, going, home) = out[1];
    assert_eq!(going, (true, max / 2), "a worn one stays worn");
    assert_eq!(home, (false, max / 2), "home too");
}
