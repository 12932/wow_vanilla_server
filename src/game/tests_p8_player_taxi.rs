//! Round 8's player: flights a spell or a script sends a player on, and
//! their end.

use wow_world_messages::Guid;
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

/// A flier's create, as another reads it: where it puts them, and its
/// spline (whether it runs and flies, its time passed, its duration and its
/// last point), if it has one.
type Spline = (bool, bool, u32, u32, Vector3d);

fn flier_in(
    m: &wow_world_messages::vanilla::opcodes::ServerOpcodeMessage,
    flier: Guid,
) -> Option<(Vector3d, Option<Spline>)> {
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
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
            } => Some((
                *living_position,
                flags.get_spline_enabled().map(|s| {
                    (
                        s.spline_flags.get_run_mode(),
                        s.spline_flags.get_flying(),
                        s.time_passed,
                        s.duration,
                        s.final_node,
                    )
                }),
            )),
            MovementBlock_UpdateFlag_Living::HasPosition { .. } => None,
        },
        _ => None,
    })
}

/// mangos's taxi flight in a create: one who comes into view of a player
/// on a flight is told of them where the flight has taken them, with its
/// path as a flying spline and its time passed, so their client flies them
/// on. The flight's `SMSG_MONSTER_MOVE` goes only to those who see it take
/// off; without this, a newcomer saw a mounted player standing still.
#[tokio::test]
async fn one_who_comes_into_view_of_a_flier_sees_them_fly_on() {
    let (realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let duration = on(&map, move |m| {
        npc::fly_path(m, ga, PATH);
        m.players[&ga]
            .flight
            .as_ref()
            .map(|f| f.walk.duration.as_millis() as u32)
    })
    .await
    .expect("flying");
    // A moment on her way, Bob comes along.
    crate::game::harness::an_update(&map).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0, START.1, START.2).await;
    let (at, spline) = b.wait_for("Alice's create", |m| flier_in(m, ga)).await;
    let (run, flying, passed, total, last) = spline.expect("her flight's spline in her create");
    assert!(run && flying, "flying, at a run");
    assert_eq!(total, duration);
    assert!(
        passed > 0 && passed < total,
        "partway: {passed} of {total} ms"
    );
    assert!(
        (last.x - (START.0 + 150.0)).abs() < 0.01,
        "to its end: {last:?}"
    );
    assert!(
        at.x >= START.0 && (at.y - START.1).abs() < 0.01,
        "on her way: {at:?}"
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
