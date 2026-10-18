//! Phase 3 against Thistle Tea's player world tests
//! (`test/game/world/entity/player`): an innkeeper's bind and the home it
//! keeps, and a flight master's nodes.

use std::sync::Arc;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::npc::TaxiNode;
use crate::game::data::{CreatureSpawn, CreatureTemplate, GameData};
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey};

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

const INNKEEPER: u32 = 965_001;
const INNKEEPER_A: u32 = 96_501;
const FLIGHT_MASTER: u32 = 965_002;
const FLIGHT_MASTER_A: u32 = 96_502;
const PEDDLER: u32 = 965_003;
const PEDDLER_A: u32 = 96_503;
const NODE: u32 = 2;

fn data() -> GameData {
    let mut d = GameData::empty();
    let npc = |entry, flags| CreatureTemplate {
        entry,
        name: format!("Npc {entry}"),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 30,
        level_max: 30,
        faction: 35,
        npc_flags: flags,
        unit_class: 1,
        health_multiplier: 1.0,
        health_max: 500,
        ..Default::default()
    };
    d.creature_templates.insert(INNKEEPER, npc(INNKEEPER, 0x80));
    d.creature_templates
        .insert(FLIGHT_MASTER, npc(FLIGHT_MASTER, 0x8));
    d.creature_templates.insert(PEDDLER, npc(PEDDLER, 0x4));
    let spawn = |guid, entry, dx: f32| CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 600,
        respawn_max: 600,
        ..Default::default()
    };
    d.creature_spawns.insert(
        0,
        vec![
            spawn(INNKEEPER_A, INNKEEPER, 2.0),
            spawn(FLIGHT_MASTER_A, FLIGHT_MASTER, -2.0),
            spawn(PEDDLER_A, PEDDLER, 1.0),
        ],
    );
    d.npc.taxi_nodes.insert(
        NODE,
        TaxiNode {
            id: NODE,
            map: 0,
            x: START.0,
            y: START.1,
            z: START.2,
            name: "Northshire".into(),
            mounts: [0, 0],
        },
    );
    d
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

/// Wait until the map has handled what `c` sent so far.
async fn synced(c: &mut Client) {
    c.send(CMSG_PLAYED_TIME {}).await;
    c.wait_for("SMSG_PLAYED_TIME", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_PLAYED_TIME(_)).then_some(())
    })
    .await;
}

async fn bound(c: &mut Client) -> bool {
    synced(c).await;
    c.backlog
        .iter()
        .any(|m| matches!(m, ServerOpcodeMessage::SMSG_PLAYERBOUND(_)))
}

#[tokio::test]
async fn the_innkeeper_binds_only_one_near_and_alive_and_the_home_is_told_at_every_login() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let innkeeper = guid::unit(INNKEEPER, INNKEEPER_A);
    a.sees(innkeeper).await;
    let activate = CMSG_BINDER_ACTIVATE { guid: innkeeper };
    // Too far by the time it is asked: no bind.
    a.heartbeat(START.0 + 40.0, START.1, START.2).await;
    a.clear();
    a.send(activate).await;
    assert!(!bound(&mut a).await, "out of reach");
    // Dead: none either. Back in reach first: a dead body takes no move.
    a.heartbeat(START.0, START.1, START.2).await;
    synced(&mut a).await;
    on_map(&realm, move |m| crate::game::map::death::kill(m, g)).await;
    a.clear();
    a.send(activate).await;
    assert!(!bound(&mut a).await, "dead");
    on_map(&realm, move |m| {
        crate::game::map::death::resurrect(m, g, 1.0);
        m.players.get_mut(&g).unwrap().area = 87;
    })
    .await;
    a.clear();
    a.send(activate).await;
    assert!(bound(&mut a).await, "bound");
    // Elsewhere, and in again: the home, as kept.
    a.heartbeat(START.0 + 30.0, START.1 + 30.0, START.2).await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.clear();
    a.login(g).await;
    let home = a
        .wait_for("SMSG_BINDPOINTUPDATE", |m| match m {
            ServerOpcodeMessage::SMSG_BINDPOINTUPDATE(b) => {
                Some((b.position, b.map.as_int(), b.area.as_int()))
            }
            _ => None,
        })
        .await;
    let (position, map, area) = home;
    assert_eq!((map, area), (0, 87));
    assert!(
        (position.x - START.0).abs() < 1.0 && (position.y - START.1).abs() < 1.0,
        "{position:?}"
    );
}

#[tokio::test]
async fn a_flight_master_alone_shows_its_nodes_learning_its_own_and_tells_whether_it_is_known() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let master = guid::unit(FLIGHT_MASTER, FLIGHT_MASTER_A);
    let peddler = guid::unit(PEDDLER, PEDDLER_A);
    a.sees(master).await;
    a.sees(peddler).await;
    let status = async |a: &mut Client| {
        a.send(CMSG_TAXINODE_STATUS_QUERY { guid: master }).await;
        a.wait_for("SMSG_TAXINODE_STATUS", |m| match m {
            ServerOpcodeMessage::SMSG_TAXINODE_STATUS(s) => Some(s.taxi_mask_node_known),
            _ => None,
        })
        .await
    };
    assert!(!status(&mut a).await);
    // A vendor by the node shows nothing.
    a.clear();
    a.send(CMSG_TAXIQUERYAVAILABLENODES { guid: peddler }).await;
    synced(&mut a).await;
    assert!(
        !a.backlog
            .iter()
            .any(|m| matches!(m, ServerOpcodeMessage::SMSG_SHOWTAXINODES(_)))
    );
    // The flight master teaches its node, then shows it.
    a.send(CMSG_TAXIQUERYAVAILABLENODES { guid: master }).await;
    a.wait_for("SMSG_NEW_TAXI_PATH", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_TAXI_PATH).then_some(())
    })
    .await;
    let nodes = a
        .wait_for("SMSG_SHOWTAXINODES", |m| match m {
            ServerOpcodeMessage::SMSG_SHOWTAXINODES(s) => Some((s.nearest_node, s.nodes.clone())),
            _ => None,
        })
        .await;
    // (The mask's bits by node are parity/3e's, which counts them from
    // one as the client does.)
    assert_eq!(nodes.0, NODE);
    assert_ne!(nodes.1[0], 0);
    assert!(status(&mut a).await);
}
