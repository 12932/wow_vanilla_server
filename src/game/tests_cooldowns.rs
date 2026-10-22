//! What a client may ask only so often (review 5's rev5-net 6): a `/who`,
//! whois or inspect once in five seconds, as mangos's opcode cooldowns,
//! and a cache upload written at most once in five seconds a kind, the
//! same again not at all.

use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::harness::{self, Client};
use crate::game::map::social::SMSG_WHO;

fn realm(cooldown: Duration) -> Arc<Realm> {
    realm_as(cooldown, false)
}

fn realm_as(cooldown: Duration, everyone_gm: bool) -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.packet_cooldown = cooldown;
    cfg.everyone_gm = everyone_gm;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, Default::default(), None)
}

fn who() -> CMSG_WHO {
    CMSG_WHO {
        minimum_level: Level::new(1),
        maximum_level: Level::new(60),
        player_name: String::new(),
        guild_name: String::new(),
        race_mask: u32::MAX,
        class_mask: u32::MAX,
        zones: Vec::new(),
        search_strings: Vec::new(),
    }
}

/// A second `/who` sooner than the cooldown is dropped, not answered; one
/// after it is.
#[tokio::test]
async fn a_who_sooner_than_its_cooldown_is_dropped() {
    let cooldown = Duration::from_millis(600);
    let realm = realm(cooldown);
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.send(who()).await;
    a.wait_frame("SMSG_WHO", SMSG_WHO, |_| true).await;
    let started = tokio::time::Instant::now();
    a.send(who()).await;
    let answered = tokio::time::timeout(
        Duration::from_millis(200),
        a.wait_frame("SMSG_WHO", SMSG_WHO, |_| true),
    )
    .await;
    assert!(answered.is_err(), "the second, at once, answered");
    tokio::time::sleep_until(started + cooldown).await;
    a.send(who()).await;
    a.wait_frame("SMSG_WHO after the cooldown", SMSG_WHO, |_| true)
        .await;
}

/// A GM's account isn't held to the cooldown: `/who` after `/who` at
/// once, every one answered.
#[tokio::test]
async fn a_gms_who_has_no_cooldown() {
    let realm = realm_as(Duration::from_secs(60), true);
    let mut a = Client::connect(&realm, "GM");
    a.enter("Gamma", Race::Human, Class::Warrior).await;
    for i in 0..5 {
        a.send(who()).await;
        tokio::time::timeout(
            Duration::from_secs(2),
            a.wait_frame("SMSG_WHO", SMSG_WHO, |_| true),
        )
        .await
        .unwrap_or_else(|_| panic!("the GM's /who {i} answered"));
    }
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(data).unwrap();
    z.finish().unwrap()
}

fn upload(kind: u32, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&kind.to_le_bytes());
    body.extend_from_slice(&(data.len() as u32).to_le_bytes());
    body.extend_from_slice(&zlib(data));
    body
}

/// What the store has of the character's cache `kind`.
async fn stored(realm: &Realm, g: wow_world_messages::Guid, kind: u8) -> Option<Vec<u8>> {
    realm.store.flush().await;
    let key = crate::game::map::account_data::key("A", crate::game::guid::counter(g), kind);
    realm
        .store
        .account_data(&key)
        .await
        .into_iter()
        .find(|(k, ..)| *k == kind)
        .map(|(.., d)| d)
}

/// A cache changed again within five seconds of its last write is held
/// (asked for, it is the latest) and written at the save, not at once; the
/// same again isn't written. Before, every upload was a store write of up
/// to 64 KB, a hundred a second.
#[tokio::test]
async fn a_cache_is_written_at_most_once_in_five_seconds() {
    let realm = realm(Duration::ZERO);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.send_raw(0x20B, &upload(5, b"first")).await;
    let until = |want: &'static [u8]| {
        let realm = realm.clone();
        async move {
            for _ in 0..200 {
                if stored(&realm, g, 5).await.as_deref() == Some(want) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("the store never had {want:?}");
        }
    };
    until(b"first").await;
    a.send_raw(0x20B, &upload(5, b"second")).await;
    a.send(CMSG_REQUEST_ACCOUNT_DATA { data_type: 5 }).await;
    let frame = a
        .wait_frame("SMSG_UPDATE_ACCOUNT_DATA", 0x20C, |_| true)
        .await;
    assert_eq!(
        crate::game::map::account_data::parse_update(&frame),
        Some((5, b"second".to_vec())),
        "held at once"
    );
    assert_eq!(
        stored(&realm, g, 5).await.as_deref(),
        Some(&b"first"[..]),
        "not written yet"
    );
    // At the logout's save.
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    until(b"second").await;
}
