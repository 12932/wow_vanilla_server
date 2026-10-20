//! Round 8's player: flights a spell or a script sends a player on, and
//! their end.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::npc::{TaxiNode, TaxiPathNode};
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::{Map, npc};
use crate::game::tests_p3_combat::{START, setup_with};
use crate::game::tests_p3_entity::on;

const HERE: u32 = 50;
const THERE: u32 = 51;
const PATH: u32 = 500;
/// A spell that sends its caster along `PATH`, and gives them an aura.
const DELIVER: u32 = 94_300;
/// A cast of three seconds.
const SLOW: u32 = 94_301;
/// A node only a spell flies from.
const SPELL_ONLY: u32 = 52;

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    // Nodes without a mount for either side, the first unplaced (Thistle
    // Tea's "unpositioned source").
    for (id, x) in [(HERE, 0.0), (THERE, START.0 + 150.0)] {
        d.npc.taxi_nodes.insert(
            id,
            TaxiNode {
                id,
                map: 0,
                x,
                y: if id == HERE { 0.0 } else { START.1 },
                z: if id == HERE { 0.0 } else { START.2 },
                name: format!("Node {id}"),
                mounts: [0, 0],
            },
        );
    }
    d.npc.taxi_paths.insert((HERE, THERE), (PATH, 75));
    d.npc.taxi_path_nodes.insert(
        PATH,
        (0..4)
            .map(|i| TaxiPathNode {
                map: 0,
                x: START.0 + 50.0 * i as f32,
                y: START.1,
                z: START.2,
                ..Default::default()
            })
            .collect(),
    );
    let mut deliver = SpellTemplate {
        id: DELIVER,
        name: "Flight".into(),
        range_index: 1,
        duration_index: 21,
        equipped_item_class: -1,
        ..Default::default()
    };
    deliver.effects[0] = SpellEffect {
        effect: crate::game::map::extra_effects::EFFECT_SEND_TAXI,
        misc_value: PATH as i32,
        target_a: 1,
        ..Default::default()
    };
    deliver.effects[1] = SpellEffect {
        effect: 6,
        aura: crate::game::map::auras::AURA_DUMMY,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(DELIVER, deliver);
    d.spells.cast_times.insert(14, 3000);
    let mut slow = SpellTemplate {
        id: SLOW,
        name: "Slow".into(),
        range_index: 1,
        casting_time_index: 14,
        equipped_item_class: -1,
        ..Default::default()
    };
    slow.effects[0] = SpellEffect {
        effect: 3,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(SLOW, slow);
    d.npc.taxi_nodes.insert(
        SPELL_ONLY,
        TaxiNode {
            id: SPELL_ONLY,
            map: 0,
            name: "Spell only".into(),
            ..Default::default()
        },
    );
    d.npc.spell_only_nodes.insert(SPELL_ONLY);
    d
}

/// The look the flier is mounted on, if they are flying.
fn flying(m: &Map, g: Guid) -> Option<u32> {
    let p = &m.players[&g];
    p.flight
        .as_ref()
        .map(|_| p.fields.get_u32(UNIT_FIELD_MOUNTDISPLAYID))
}

/// Thistle Tea's taxi_test "allows a mountless spell route from an
/// unpositioned source" and "preserves the delivering spell": a spell's
/// route from a node with no mount for either side flies the player
/// unmounted, as mangos's `ActivateTaxiPathTo` allows a spell's flight a
/// model of 0 (wvs put them on a gryphon); it is free, discovers nothing,
/// and the spell's other effect lands too.
#[tokio::test]
async fn a_spells_route_with_no_mount_flies_unmounted_and_free() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (mount, money, aura, known) = on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().set_money(75);
        let before = m.players[&ga].row.state.taxi_mask.clone();
        crate::game::map::spells::cast_triggered(m, ga, DELIVER, Some(ga));
        let p = &m.players[&ga];
        (
            flying(m, ga),
            p.money(),
            p.auras.holders.iter().any(|h| h.spell == DELIVER),
            p.row.state.taxi_mask != before,
        )
    })
    .await;
    assert_eq!(mount, Some(0), "flying, on no mount");
    assert_eq!(money, 75, "free");
    assert!(aura, "the spell's other effect too");
    assert!(!known, "nothing discovered");
}

/// Thistle Tea's taxi_test "preserves the delivering spell and cancels
/// another cast" and cancel/1 "discards the route without advancing to
/// its destination": a script's flight ends a cast under way; a teleport
/// on the way ends the flight where the player is sent, off the mount and
/// free to move, never at the route's end.
#[tokio::test]
async fn a_flight_ends_a_cast_and_a_teleport_ends_the_flight() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (casting, mount) = on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().spells.push(SLOW);
        let targets = crate::game::map::spells::Targets {
            unit: Some(ga),
            ..Default::default()
        };
        crate::game::map::spells::cast(m, ga, SLOW, targets, None);
        let casting = m.players[&ga].caster.casting.is_some();
        npc::fly_path(m, ga, PATH);
        (casting, flying(m, ga))
    })
    .await;
    assert!(casting, "a cast under way");
    assert_eq!(mount, Some(0), "flying");
    let still = on(&map, move |m| m.players[&ga].caster.casting.is_some()).await;
    assert!(!still, "the cast ended");
    let away = Vector3d {
        x: START.0 - 30.0,
        y: START.1 + 10.0,
        z: START.2,
    };
    let after = on(&map, move |m| {
        m.teleport(ga, m.key, away, 0.0);
        let p = &m.players[&ga];
        (
            p.flight.is_some(),
            p.fields.get_u32(UNIT_FIELD_MOUNTDISPLAYID),
            p.fields.get_u32(UNIT_FIELD_FLAGS) & 0x0010_0000,
        )
    })
    .await;
    assert_eq!(after, (false, 0, 0), "the flight over");
    crate::game::harness::an_update(&map).await;
    let at = on(&map, move |m| m.players[&ga].position()).await;
    assert!(
        (at.x - away.x).abs() < 0.5,
        "where it was sent, not the route's end: {at:?}"
    );
}

/// Thistle Tea's taxi_test unlock_all/2 "persists every discoverable
/// node": `.debug taxi` teaches every node a flight master could, not one
/// only a spell flies from (wvs set every bit there was), and they are
/// kept over a relog.
#[tokio::test]
async fn debug_taxi_teaches_every_discoverable_node_and_keeps_them() {
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    a.say(".debug taxi").await;
    a.system_message("Every flight path known").await;
    let known = move |m: &mut Map| {
        let mask = &m.players[&ga].row.state.taxi_mask;
        [HERE, THERE, SPELL_ONLY, 53].map(|n| npc::knows_node(mask, n))
    };
    assert_eq!(on(&map, known).await, [true, true, false, false]);
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.login(ga).await;
    let map = realm.registry.online(ga).unwrap().map();
    assert_eq!(on(&map, known).await, [true, true, false, false], "kept");
}

/// A flier's create, as another reads it: where it puts them, whether it
/// has them going forward and walking (the client moves a unit on a spline
/// only going somewhere), and its spline, if it has one.
struct Seen {
    at: Vector3d,
    forward: bool,
    walking: bool,
    spline: Option<MovementBlock_MovementFlags_SplineEnabled>,
}

fn flier_in(m: &ServerOpcodeMessage, flier: Guid) -> Option<Seen> {
    let objects = match m {
        ServerOpcodeMessage::SMSG_UPDATE_OBJECT(u) => &u.objects,
        ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(u) => &u.objects,
        _ => return None,
    };
    objects.iter().find_map(|o| match &o.update_type {
        Object_UpdateType::CreateObject {
            guid3, movement2, ..
        }
        | Object_UpdateType::CreateObject2 {
            guid3, movement2, ..
        } if *guid3 == flier => match movement2.update_flag.get_living()? {
            MovementBlock_UpdateFlag_Living::Living {
                flags,
                living_position,
                ..
            } => Some(Seen {
                at: *living_position,
                forward: flags.get_forward(),
                walking: flags.get_walk_mode(),
                spline: flags.get_spline_enabled().cloned(),
            }),
            MovementBlock_UpdateFlag_Living::HasPosition { .. } => None,
        },
        _ => None,
    })
}

/// mangos's taxi flight in a create: one who comes into view of a player
/// on a flight is told of them where the flight has taken them, with its
/// path as a flying spline and its time passed, going forward along it at
/// a run, so their client flies them on. The flight's `SMSG_MONSTER_MOVE`
/// goes only to those who see it take off; without the spline, a newcomer
/// saw a mounted player standing still; without going forward, frozen in
/// the air (the 1.12 client moves a unit on a spline only if its create
/// has it going somewhere, `0x618940`).
#[tokio::test]
async fn one_who_comes_into_view_of_a_flier_sees_them_fly_on() {
    let (realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (duration, id) = on(&map, move |m| {
        npc::fly_path(m, ga, PATH);
        m.players[&ga]
            .flight
            .as_ref()
            .map(|f| (f.walk.duration.as_millis() as u32, f.walk.spline_id))
    })
    .await
    .expect("flying");
    // A moment on her way, Bob comes along.
    crate::game::harness::an_update(&map).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0, START.1, START.2).await;
    let seen = b.wait_for("Alice's create", |m| flier_in(m, ga)).await;
    let s = seen.spline.expect("her flight's spline in her create");
    assert!(
        s.spline_flags.get_run_mode() && s.spline_flags.get_flying(),
        "flying, at a run"
    );
    assert!(
        seen.forward && !seen.walking,
        "going forward along it, running, so the client moves her"
    );
    assert_eq!((s.duration, s.id), (duration, id), "the flight she took");
    assert!(
        s.time_passed > 0 && s.time_passed < s.duration,
        "partway: {} of {} ms",
        s.time_passed,
        s.duration
    );
    assert!(
        (s.final_node.x - (START.0 + 150.0)).abs() < 0.01,
        "to its end: {:?}",
        s.final_node
    );
    assert!(
        seen.at.x >= START.0 && (seen.at.y - START.1).abs() < 0.01,
        "on her way: {:?}",
        seen.at
    );
    // Her own create has none: her client flies by her flight's move.
    let own = on(&map, move |m| {
        matches!(
            m.players[&ga].create(true).placement,
            crate::game::update::Placement::Living {
                spline: Some(_),
                ..
            }
        )
    })
    .await;
    assert!(!own, "no spline in her own create");
}

/// A flight master for the routes below.
const MASTER: u32 = 3710;
/// The routes' nodes, from the start, and their paths.
const ROUTE_NODE: u32 = 60;
const ROUTE_PATH: u32 = 610;
/// Each hop's length on the ground, and the height flown at.
const HOP: f32 = 160.0;
const UP: f32 = 30.0;

/// Where a route's node `i` is: from the start, a right angle at each
/// node, east, north, east and so on.
fn node_at(i: u32) -> (f32, f32) {
    let (mut x, mut y) = (START.0, START.1);
    for k in 0..i {
        if k % 2 == 0 {
            x += HOP;
        } else {
            y += HOP;
        }
    }
    (x, y)
}

/// [`world`], with a flight master and a route of `hops` hops from the
/// start, each of `per_hop` points `UP` yards up, joined at their nodes
/// (`taxi_path_transitions`: the whole of one, the next from its second),
/// so the route passes each node and turns there.
fn route_world(hops: u32, per_hop: usize) -> GameData {
    let mut d = world();
    let mut master = crate::game::tests_p3_combat::template(MASTER, 7);
    master.npc_flags = npc::NPC_FLAG_FLIGHTMASTER;
    master.faction = crate::game::tests_p3_combat::HUMANS;
    d.creature_templates.insert(MASTER, master);
    for i in 0..=hops {
        let (x, y) = node_at(i);
        d.npc.taxi_nodes.insert(
            ROUTE_NODE + i,
            TaxiNode {
                id: ROUTE_NODE + i,
                map: 0,
                x,
                y,
                z: START.2,
                name: format!("Route {i}"),
                mounts: [0, 0],
            },
        );
    }
    for i in 0..hops {
        let path = ROUTE_PATH + i;
        d.npc
            .taxi_paths
            .insert((ROUTE_NODE + i, ROUTE_NODE + i + 1), (path, 1));
        let ((x0, y0), (x1, y1)) = (node_at(i), node_at(i + 1));
        let points = (0..per_hop)
            .map(|k| {
                let t = k as f32 / (per_hop - 1) as f32;
                TaxiPathNode {
                    map: 0,
                    x: x0 + (x1 - x0) * t,
                    y: y0 + (y1 - y0) * t,
                    z: START.2 + UP,
                    ..Default::default()
                }
            })
            .collect();
        d.npc.taxi_path_nodes.insert(path, points);
        if i > 0 {
            d.npc
                .taxi_transitions
                .insert((path - 1, path), (per_hop - 1, 1));
        }
    }
    d
}

/// Where `walk` has its flier `yards` along it.
fn along(walk: &crate::game::map::creature::Walk, yards: f32) -> Vector3d {
    walk.position_at(walk.started + Duration::from_secs_f32(yards / walk.speed))
        .0
}

/// What `frame` tells its client of `g` that could stop them on their
/// way: a create, a movement block or a forget of them, or a movement
/// packet (a relayed move, a monster move, a spline's change) naming them.
/// Their fields' changes are no such thing.
fn stopping(frame: &[u8], g: Guid) -> Vec<String> {
    if frame.len() < 4 {
        return Vec::new();
    }
    let opcode = u16::from_le_bytes([frame[2], frame[3]]);
    let objects = match ServerOpcodeMessage::read_unencrypted(frame) {
        Ok(ServerOpcodeMessage::SMSG_UPDATE_OBJECT(u)) => u.objects,
        Ok(ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(u)) => u.objects,
        _ => {
            let mut packed = Vec::new();
            crate::game::update::write_packed_guid(&mut packed, g);
            let movement = matches!(opcode, 0x0B5..=0x0EE | 0x2AE | 0x2FE..=0x310);
            return if movement && frame[4..].starts_with(&packed) {
                vec![format!("opcode {opcode:#05x}")]
            } else {
                Vec::new()
            };
        }
    };
    objects
        .iter()
        .filter_map(|o| match &o.update_type {
            Object_UpdateType::Movement { guid2, .. } if *guid2 == g => {
                Some("a movement block".to_owned())
            }
            Object_UpdateType::CreateObject { guid3, .. }
            | Object_UpdateType::CreateObject2 { guid3, .. }
                if *guid3 == g =>
            {
                Some("a create".to_owned())
            }
            Object_UpdateType::OutOfRangeObjects { guids } if guids.contains(&g) => {
                Some("a forget".to_owned())
            }
            _ => None,
        })
        .collect()
}

/// Alice flies a route of `hops` hops of `per_hop` points from the start,
/// paid at a flight master; `yards` along it, Bob, who was far off, walks
/// into view of her. Her create tells his client the whole flight she took,
/// going forward along it: its id, its duration, its path from her
/// take-off to the route's end (laid out as the client reads it), its time
/// passed where she is now; and for a second after (a heartbeat from her
/// client among it, as a client on a taxi may send), he is told nothing of
/// her that would stop her on her way. A flight is one walk however many
/// hops it has (`activate_route` stitches them), its move told to everyone
/// who sees her take off; there is no next leg for a newcomer to miss.
async fn seen_mid_flight(hops: u32, per_hop: usize, yards: f32) -> Vec<u8> {
    let (realm, a, ga, map) = setup_with(route_world(hops, per_hop), Class::Warrior).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    // Far off: he doesn't see her take off.
    b.heartbeat(START.0 - 700.0, START.1 - 700.0, START.2).await;
    let deadline = tokio::time::Instant::now() + crate::game::harness::WAIT;
    while on(&map, move |m| m.players[&gb].known.contains(&ga)).await {
        assert!(tokio::time::Instant::now() < deadline, "Bob never left");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    b.clear();
    let walk = on(&map, move |m| {
        let master = crate::game::tests_p3_combat::spawn(m, ga, MASTER, 20, 2.0);
        let p = m.players.get_mut(&ga).unwrap();
        p.row.state.taxi_mask = vec![u32::MAX; 4];
        p.set_money(10_000);
        let route: Vec<u32> = (0..=hops).map(|i| ROUTE_NODE + i).collect();
        npc::activate_route(m, ga, master, &route);
        m.players[&ga].flight.as_ref().map(|f| f.walk.clone())
    })
    .await
    .expect("flying");
    let (x, y) = node_at(hops);
    let end = walk.destination();
    assert!(
        (end.x - x).abs() < 0.01 && (end.y - y).abs() < 0.01,
        "one walk, to the route's end: {end:?}"
    );
    // Her flight `yards` on, at once: its start that much earlier.
    let by = Duration::from_secs_f32(yards / walk.speed);
    let walk = on(&map, move |m| {
        let f = m.players.get_mut(&ga).unwrap().flight.as_mut().unwrap();
        f.walk.started = f.walk.started.checked_sub(by).expect("a clock that far");
        f.walk.clone()
    })
    .await;
    crate::game::harness::an_update(&map).await;
    // Bob walks up to where she is headed, fifty yards on.
    let spot = along(&walk, yards + 50.0);
    b.heartbeat(spot.x + 10.0, spot.y - 10.0, START.2).await;
    let seen = b.wait_for("Alice's create", |m| flier_in(m, ga)).await;
    let s = seen.spline.expect("her flight in her create");
    assert!(
        s.spline_flags.get_run_mode() && s.spline_flags.get_flying(),
        "flying, at a run"
    );
    assert!(seen.forward && !seen.walking, "going forward, running");
    assert_eq!(s.id, walk.spline_id, "the move she took off with");
    assert_eq!(s.duration, walk.duration.as_millis() as u32, "its duration");
    let flown = by.as_millis() as u32;
    assert!(
        s.time_passed >= flown && s.time_passed < s.duration,
        "{} of {} ms flown, at least {flown}",
        s.time_passed,
        s.duration
    );
    assert_eq!(s.nodes.len(), walk.points.len() + 2, "the whole path");
    assert_eq!(s.nodes[1..=walk.points.len()], walk.points[..]);
    assert_eq!(s.final_node, end, "to the route's end");
    let there = walk
        .position_at(walk.started + Duration::from_millis(u64::from(s.time_passed)))
        .0;
    assert!(
        crate::game::map::combat::dist(seen.at, there) < 0.5,
        "where the spline has her: {:?}, not {there:?}",
        seen.at
    );
    // What follows, for a second: nothing that would stop her.
    let frames = b.captured();
    let mark = frames
        .iter()
        .rposition(|f| {
            ServerOpcodeMessage::read_unencrypted(&f[..]).is_ok_and(|m| flier_in(&m, ga).is_some())
        })
        .expect("the create's frame")
        + 1;
    let now = on(&map, move |m| m.players[&ga].position()).await;
    a.heartbeat(now.x, now.y, now.z).await;
    let from = on(&map, |m| m.stats.updates).await;
    while on(&map, |m| m.stats.updates).await < from + 20 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let still = on(&map, move |m| m.players[&ga].flight.is_some()).await;
    assert!(still, "on her way");
    let told: Vec<String> = b.captured()[mark..]
        .iter()
        .flat_map(|f| stopping(f, ga))
        .collect();
    assert!(told.is_empty(), "nothing to stop her: {told:?}");
    frames[mark - 1].to_vec()
}

/// [`seen_mid_flight`]: just after she takes off.
#[tokio::test]
async fn a_flier_seen_just_after_take_off_flies_on() {
    seen_mid_flight(2, 5, UP + 20.0).await;
}

/// [`seen_mid_flight`]: just into her route's second leg.
#[tokio::test]
async fn a_flier_seen_at_the_start_of_a_leg_flies_on() {
    seen_mid_flight(3, 5, UP + HOP + 10.0).await;
}

/// [`seen_mid_flight`]: halfway along a leg.
#[tokio::test]
async fn a_flier_seen_mid_leg_flies_on() {
    seen_mid_flight(3, 5, UP + 1.5 * HOP).await;
}

/// [`seen_mid_flight`]: at the node between two legs, where she turns.
#[tokio::test]
async fn a_flier_seen_at_the_turn_between_two_legs_flies_on() {
    seen_mid_flight(3, 5, UP + 2.0 * HOP).await;
}

/// [`seen_mid_flight`]: on a long route of eight legs and 314 points (1.12's
/// longest has 317), at the turn into the sixth: its create one frame,
/// compressed, well inside a frame's size, not refused for it.
#[tokio::test]
async fn a_flier_seen_on_a_long_route_flies_on() {
    let frame = seen_mid_flight(8, 40, UP + 5.0 * HOP).await;
    let opcode = u16::from_le_bytes([frame[2], frame[3]]);
    assert_eq!(
        opcode,
        crate::game::update::SMSG_COMPRESSED_UPDATE_OBJECT,
        "compressed"
    );
    assert!(frame.len() - 4 <= crate::game::update::MAX_FRAME_BODY);
}

/// Thistle Tea's dev_commands_test ".start ... rejects combat and taxi
/// flight": `.start` is refused in a fight and in flight, as mangos's
/// `HandleStartCommand` refuses them before it casts Stuck.
#[tokio::test]
async fn start_is_refused_in_a_fight_and_in_flight() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().fields.set_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            true,
        );
    })
    .await;
    a.say(".start").await;
    a.system_message("You are in combat").await;
    on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().fields.set_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            false,
        );
        npc::fly_path(m, ga, PATH);
    })
    .await;
    a.say(".start").await;
    a.system_message("You are in flight").await;
    let still = on(&map, move |m| m.players[&ga].flight.is_some()).await;
    assert!(still, "flying on");
}
