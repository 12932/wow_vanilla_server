//! Round 8's player: logging in again at once.

use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::harness::{self, Client};

/// A client that comes back within milliseconds of losing its connection
/// logs in to the character as their old connection's leaving saved them,
/// not as they were stored before it. The old map is held while the new
/// login starts, so that it reads the character before their save is
/// queued: before, the new login read the row then, and failed (the
/// character "already in the world") or came in with what was stored
/// before (a pet, money, the place).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_login_at_once_after_a_disconnect_waits_for_its_save() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "THRALL");
    let g = a.enter("Thrall", Race::Orc, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| m.players.get_mut(&g).unwrap().set_money(12_345))
        .await
        .unwrap();
    let (release, held) = std::sync::mpsc::channel::<()>();
    map.call(move |_| {
        tokio::task::block_in_place(|| {
            let _ = held.recv_timeout(Duration::from_secs(5));
        });
    });
    a.disconnect();
    let mut b = Client::connect(&realm, "THRALL");
    b.send(CMSG_PLAYER_LOGIN { guid: g }).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    release.send(()).unwrap();
    b.wait_for("SMSG_LOGIN_VERIFY_WORLD", |m| match m {
        ServerOpcodeMessage::SMSG_LOGIN_VERIFY_WORLD(_) => Some(()),
        ServerOpcodeMessage::SMSG_CHARACTER_LOGIN_FAILED(f) => {
            panic!("login failed: {:?}", f.result)
        }
        _ => None,
    })
    .await;
    let now = realm.registry.online(g).unwrap().map();
    let money = now.query(move |m| m.players[&g].money()).await.unwrap();
    assert_eq!(money, 12_345, "as their leaving saved them");
}
