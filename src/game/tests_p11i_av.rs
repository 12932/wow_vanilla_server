//! Round 11's test-only rows on Alterac Valley's supplies (Thistle Tea's
//! `battleground_supplies_test.exs`): requests for a beacon or for the
//! assault orders sent together, which the valley's map handles one at a
//! time (an actor), so the first takes the stockpile and the rest find it
//! spent.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p4_items_av::{COMMANDERS, ORDERS, QUARTERMASTERS, data, menu, of, set_standing, x, xm};
use super::tests_pvp::{both_into, put};
use super::tests_pvp2::choose;
use crate::game::harness::{self, Client};
use crate::game::map::battleground::av_quests::{Fleet, FleetPhase, March};
use crate::game::map::{Map as World, MapHandle};

const MARSHAL: u32 = 13446;

async fn setup() -> (Client, Guid, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let grom = b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::AlteracValley).await;
    put(&map, &mut a, 3.0, 0.0).await;
    put(&map, &mut b, 3.0, 0.0).await;
    (a, alice, b, grom, map)
}

fn count_of(m: &World, g: Guid, entry: u32) -> u32 {
    m.players[&g].count_of(entry)
}

/// `n` choices of `option` at `npc`, sent together, and the menu's
/// closings they bring.
async fn together(c: &mut Client, npc: Guid, option: u32, n: usize) {
    c.clear();
    for _ in 0..n {
        choose(c, npc, option).await;
    }
    c.wait_for("SMSG_GOSSIP_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
    })
    .await;
    crate::game::tests_p3_player2_ammo::handled(c).await;
}

/// Thistle Tea's "parallel requests serialize against the shared
/// stockpile": eight requests for Guse's beacon sent together, his fleet
/// funded for one, give one beacon and leave the stockpile spent
/// (`av_quests::gossip_select`'s `TakeBeacon`, asked again at each).
#[tokio::test]
async fn beacon_requests_sent_together_give_one_beacon() {
    let (_a, _alice, mut b, grom, map) = setup().await;
    let (i, &(entry, _, _, goal, _, beacon, _)) = COMMANDERS
        .iter()
        .enumerate()
        .find(|(_, c)| c.0 == 13179)
        .unwrap();
    let guse = of(&map, entry).await;
    map.query(move |m| {
        xm(m).air[i] = Fleet {
            phase: FleetPhase::Ready,
            count: goal,
            launched: false,
        };
    })
    .await;
    assert_eq!(menu(&mut b, guse).await.1.len(), 2, "the beacon and the attack");
    together(&mut b, guse, 0, 8).await;
    let (count, beacons) = map
        .query(move |m| (x(m).air[i].count, count_of(m, grom, beacon)))
        .await
        .unwrap();
    assert_eq!((count, beacons), (0, 1), "one beacon, the stockpile spent");
}

/// Thistle Tea's "commits one order and receipt and deploys only once"
/// and "racing requesters cannot duplicate an infantry deployment": eight
/// requests for the assault orders sent together, the Irondeep supplies
/// funding one, give one order and assemble one field marshal; the
/// supplies spent, the ground assault assembled.
#[tokio::test]
async fn orders_requests_sent_together_give_one_order_and_one_marshal() {
    let (mut a, alice, _b, _grom, map) = setup().await;
    set_standing(&map, alice, 730, 9000).await;
    let quartermaster = of(&map, QUARTERMASTERS[0]).await;
    map.query(|m| {
        xm(m).ground[0].irondeep = 280;
    })
    .await;
    assert_eq!(menu(&mut a, quartermaster).await.1.len(), 3, "status, orders, goods");
    together(&mut a, quartermaster, 1, 8).await;
    let marshals = |m: &World| {
        m.creatures
            .iter()
            .filter(|(_, c)| c.entry == MARSHAL)
            .count()
    };
    let out = map
        .query(move |m| {
            let gr = x(m).ground[0];
            (count_of(m, alice, ORDERS), marshals(m), gr.phase, gr.irondeep)
        })
        .await
        .unwrap();
    assert_eq!(out, (1, 1, March::Assembled, 0), "one order, one marshal");
}
