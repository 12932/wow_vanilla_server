//! Phase 3's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/taxi_test.exs` (docs/parity/matrix.csv):
//! a route a script or a spell sends a player along.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::npc::{TaxiNode, TaxiPathNode};
use crate::game::data::{CreatureTemplate, GameData};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::{Map, combat, npc};
use crate::game::tests_p3_combat::{BRUTE, START, setup_with, spawn, template};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_player2_ammo::handled;

/// A node where Alice stands, with the Horde's mount only, and one away.
const HERE: u32 = 40;
const THERE: u32 = 41;
const PATH: u32 = 400;
const WYVERN: u32 = 3730;
const WYVERN_LOOK: u32 = 295;

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.creature_templates.insert(
        WYVERN,
        CreatureTemplate {
            display_ids: [WYVERN_LOOK, 0, 0, 0],
            ..template(WYVERN, 1)
        },
    );
    for (id, dx) in [(HERE, 0.0), (THERE, 200.0)] {
        d.npc.taxi_nodes.insert(
            id,
            TaxiNode {
                id,
                map: 0,
                x: START.0 + dx,
                y: START.1,
                z: START.2,
                name: format!("Node {id}"),
                mounts: [0, WYVERN],
            },
        );
    }
    d.npc.taxi_paths.insert((HERE, THERE), (PATH, 50));
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
    d
}

/// What `c` is told of a flight, if anything.
async fn reply(c: &mut Client) -> Option<ActivateTaxiReply> {
    handled(c).await;
    c.gets(std::time::Duration::from_millis(1), |m| match m {
        ServerOpcodeMessage::SMSG_ACTIVATETAXIREPLY(r) => Some(r.reply),
        _ => None,
    })
    .await
}

fn flying(m: &Map, g: Guid) -> Option<u32> {
    let p = &m.players[&g];
    p.flight
        .as_ref()
        .map(|_| p.fields.get_u32(UNIT_FIELD_MOUNTDISPLAYID))
}

/// Thistle Tea's "rejects a scripted path away from its source", "scripted
/// routes use an opposite-faction mount when needed" and "rejects
/// unavailable players ... without changing their state", as mangos's
/// `ActivateTaxiPathTo` without a flight master has them: from far off its
/// first node, or in a fight, a scripted route is refused and told why;
/// from beside it the flight goes, on the other side's mount when the node
/// has none of the flier's, and free.
#[tokio::test]
async fn a_scripted_route_starts_near_its_node_out_of_a_fight_on_a_mount_it_has() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let money = on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.set_money(1000);
        let at = p.position();
        p.movement.position.x = at.x + 40.0;
        npc::fly_path(m, ga, PATH);
        flying(m, ga)
    })
    .await;
    assert_eq!(money, None, "not from forty yards off");
    assert_eq!(reply(&mut a).await, Some(ActivateTaxiReply::TooFarAway));
    let fighting = on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.movement.position.x = START.0;
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        let c = m.creature_mut(brute).unwrap();
        c.fields.set_f32(UNIT_FIELD_MINDAMAGE, 0.0);
        c.fields.set_f32(UNIT_FIELD_MAXDAMAGE, 0.0);
        combat::engage(m, ga, brute);
        npc::fly_path(m, ga, PATH);
        let flown = flying(m, ga);
        combat::leave_combat(m, ga);
        m.remove_creature(brute);
        flown
    })
    .await;
    assert_eq!(fighting, None, "not in a fight");
    assert_eq!(reply(&mut a).await, Some(ActivateTaxiReply::PlayerBusy));
    let (mount, left) = on(&map, move |m| {
        npc::fly_path(m, ga, PATH);
        (flying(m, ga), m.players[&ga].money())
    })
    .await;
    assert_eq!(
        mount,
        Some(WYVERN_LOOK),
        "the Horde's mount for want of one"
    );
    assert_eq!(left, 1000, "free");
}
