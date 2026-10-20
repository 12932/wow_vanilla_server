//! Review 5's network and sessions findings (`docs/review/review5.md`,
//! `rev5-net`): a connection's life from its handshake to its close.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::duplex;
use wow_srp::normalized_string::NormalizedString;
use wow_srp::vanilla_header::{HeaderCrypto, ProofSeed};
use wow_world_messages::vanilla::*;

use crate::game::conn::{self, Connection, Outbound};
use crate::game::harness;
use crate::game::map::MapKey;

fn paired_crypto() -> (HeaderCrypto, HeaderCrypto) {
    let session_key = [0u8; 40];
    let username = NormalizedString::new("test".to_string()).unwrap();
    let server_seed = ProofSeed::new();
    let server_seed_value = server_seed.seed();
    let client_seed = ProofSeed::new();
    let client_seed_value = client_seed.seed();
    let (client_proof, client) =
        client_seed.into_client_header_crypto(&username, session_key, server_seed_value);
    let server = server_seed
        .into_server_header_crypto(&username, session_key, client_proof, client_seed_value)
        .unwrap();
    (server, client)
}

fn opcode(f: &[u8]) -> u16 {
    u16::from_le_bytes([f[2], f[3]])
}

async fn next_of(frames: &mut conn::Frames, op: u16) -> Arc<[u8]> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        while let Some(f) = frames.try_recv() {
            if f.len() >= 4 && opcode(&f) == op {
                return f;
            }
        }
        assert!(tokio::time::Instant::now() < deadline, "no 0x{op:X}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Net 1 (and net 5's second half): a connection whose queue is closed
/// (over budget, taken over, its map gone) ends at once, its reader, its
/// session and its player's place in the world with it, though the client
/// sends nothing more. Before, it ended only at the client's next packet
/// that wasn't a ping, or at the idle timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_connection_ends_its_reader_and_its_session() {
    let realm = harness::realm();
    let (server, client) = paired_crypto();
    let (_, decrypter) = server.split();
    let (mut enc, _) = client.split();
    let (mut a, b) = duplex(1 << 20);
    let (outbound, mut frames) = Outbound::detached();
    let (in_tx, in_rx) = tokio::sync::mpsc::channel(256);
    let reader = tokio::spawn(conn::run_reader(b, decrypter, in_tx, outbound.clone()));
    let session = tokio::spawn(crate::game::session::run(
        realm.clone(),
        "CLOSER".into(),
        Connection {
            outbound: outbound.clone(),
            inbound: in_rx,
        },
    ));
    CMSG_CHAR_CREATE {
        name: "Closer".into(),
        race: Race::Human,
        class: Class::Warrior,
        gender: Gender::Female,
        skin_color: 1,
        face: 1,
        hair_style: 1,
        hair_color: 1,
        facial_hair: 1,
    }
    .tokio_write_encrypted_client(&mut a, &mut enc)
    .await
    .unwrap();
    next_of(&mut frames, 0x3A).await; // SMSG_CHAR_CREATE
    let g = realm.registry.guid_by_name("Closer").expect("created");
    CMSG_PLAYER_LOGIN { guid: g }
        .tokio_write_encrypted_client(&mut a, &mut enc)
        .await
        .unwrap();
    next_of(&mut frames, 0x236).await; // SMSG_LOGIN_VERIFY_WORLD
    assert!(realm.registry.online(g).is_some());
    let map = realm.maps.get(&realm, MapKey::world(0));
    outbound.close();
    tokio::time::timeout(Duration::from_secs(5), reader)
        .await
        .expect("the reader ends at the close")
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), session)
        .await
        .expect("the session ends at the close")
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let in_map = map
            .query(move |m| m.players.contains_key(&g))
            .await
            .unwrap();
        if !in_map && realm.registry.online(g).is_none() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the player leaves the world"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    drop(a);
}
