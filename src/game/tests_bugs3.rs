//! Bugs found while mapping wvs's tests against Thistle Tea's, batch 3:
//! the world, visibility, spawns and the wire layouts mangos-zero gives
//! the 1.12 client.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle};

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

async fn enter(
    realm: &Arc<Realm>,
    account: &str,
    name: &str,
    race: Race,
    class: Class,
) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, race, class).await;
    let map = realm.registry.online(g).unwrap().map();
    (c, g, map)
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

const SMSG_LOG_XPGAIN: u16 = 0x1D0;

/// A kill's experience names its victim and goes on with the experience
/// before the rested bonus and the group's rate; a quest's or an
/// explorer's ends at its type (mangos-zero's `SendLogXPGain`).
#[tokio::test]
async fn experience_for_a_kill_carries_its_rested_split() {
    let realm = harness::realm();
    let (mut a, g, map) = enter(&realm, "A", "Killer", Race::Human, Class::Warrior).await;
    let victim = guid::unit(299, 7);
    on(&map, move |m| {
        crate::game::map::stats::give_xp(m, g, 40, Some(victim));
    })
    .await;
    let kill = a
        .wait_frame("the kill's experience", SMSG_LOG_XPGAIN, |_| true)
        .await;
    assert_eq!(kill.len(), 21, "guid, total, type, base and rate");
    assert_eq!(&kill[..8], &victim.guid().to_le_bytes());
    assert_eq!(u32_at(&kill, 8), 40);
    assert_eq!(kill[12], 0, "a kill");
    assert_eq!(u32_at(&kill, 13), 40, "none of it rested");
    assert_eq!(f32::from_le_bytes(kill[17..21].try_into().unwrap()), 1.0);
    on(&map, move |m| {
        crate::game::map::stats::give_xp(m, g, 25, None)
    })
    .await;
    let quest = a
        .wait_frame("the quest's experience", SMSG_LOG_XPGAIN, |_| true)
        .await;
    assert_eq!(quest.len(), 13, "guid, total and type");
    assert_eq!(u32_at(&quest, 8), 25);
    assert_eq!(quest[12], 1, "not a kill");
}
