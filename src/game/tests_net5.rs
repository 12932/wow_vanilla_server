//! Review 5's network and sessions findings (`docs/review/review5.md`,
//! `rev5-net`): a connection's life from its handshake to its close.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::duplex;
use wow_srp::normalized_string::NormalizedString;
use wow_srp::vanilla_header::{HeaderCrypto, ProofSeed};
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
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

/// Net 4's store: characters and caches kept under an account's name in
/// another case (from a login before names were put in capitals) are the
/// account's at the next start; where both cases have a cache of one
/// kind, the capitals' stays.
#[tokio::test]
async fn characters_made_under_a_lower_case_name_are_the_accounts() {
    use crate::game::store::{CharacterRow, Store};
    let path = std::env::temp_dir().join(format!("wvs-net5-case-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    {
        let (store, _) = Store::open(&path).unwrap();
        for (guid, account, name) in [(1, "CASEY", "Upper"), (2, "casey", "Lower")] {
            store
                .create(CharacterRow {
                    guid,
                    account: account.into(),
                    name: name.into(),
                    race: 1,
                    class: 1,
                    level: 1,
                    ..Default::default()
                })
                .await
                .unwrap();
        }
        store.set_account_data("CASEY", 0, 1, b"upper".to_vec());
        store.set_account_data("casey", 0, 2, b"lower".to_vec());
        store.set_account_data("casey#2", 1, 3, b"lower's".to_vec());
        store.flush().await;
    }
    let (store, startup) = Store::open(&path).unwrap();
    assert!(startup.names.iter().all(|n| n.account == "CASEY"));
    let mut names: Vec<String> = store
        .characters("CASEY")
        .await
        .into_iter()
        .map(|r| r.name)
        .collect();
    names.sort();
    assert_eq!(names, ["Lower", "Upper"]);
    assert!(store.characters("casey").await.is_empty());
    assert_eq!(
        store.account_data("CASEY").await,
        vec![(0, 1, b"upper".to_vec())]
    );
    assert!(store.account_data("casey").await.is_empty());
    assert_eq!(
        store.account_data("CASEY#2").await,
        vec![(1, 3, b"lower's".to_vec())]
    );
    drop(store);
    let _ = std::fs::remove_file(&path);
}

/// Whether `c`'s session ends within a few seconds: what it is sent then
/// goes nowhere.
async fn ends(c: &harness::Client) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while c.try_send(CMSG_CHAR_ENUM {}).await {
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    true
}

fn create(name: &str) -> CMSG_CHAR_CREATE {
    CMSG_CHAR_CREATE {
        name: name.into(),
        race: Race::Human,
        class: Class::Warrior,
        gender: Gender::Female,
        skin_color: 1,
        face: 1,
        hair_style: 1,
        hair_color: 1,
        facial_hair: 1,
    }
}

fn created(m: &ServerOpcodeMessage) -> Option<WorldResult> {
    match m {
        ServerOpcodeMessage::SMSG_CHAR_CREATE(c) => Some(c.result),
        _ => None,
    }
}

/// Net 2: an account has one session. A second, once started, is the
/// account's, and the first goes: from the character screen at once; from
/// the world with the player left where they are, for the second to take
/// over at its login, as it does.
#[tokio::test]
async fn a_second_session_of_an_account_ends_the_first() {
    let realm = harness::realm();
    let first = harness::Client::connect(&realm, "TWICE");
    let mut second = harness::Client::connect(&realm, "TWICE");
    assert!(
        ends(&first).await,
        "the first, on the character screen, goes"
    );
    let g = second.enter("Twice", Race::Human, Class::Warrior).await;
    let mut third = harness::Client::connect(&realm, "TWICE");
    assert!(ends(&second).await, "the second, in the world, goes");
    assert!(
        realm.registry.online(g).is_some(),
        "its player stays in the world"
    );
    third.login(g).await;
    let o = realm.registry.online(g).expect("online");
    assert!(
        realm
            .registry
            .session_for_account("TWICE")
            .is_some_and(|s| s.same_channel(&o.session)),
        "the third's now"
    );
}

/// A player the replaced session left in the world goes, saved, when the
/// session that replaced it ends without taking them over.
#[tokio::test]
async fn a_player_left_for_a_session_that_goes_without_them_goes_too() {
    let realm = harness::realm();
    let mut first = harness::Client::connect(&realm, "LEAVER");
    let g = first.enter("Leaver", Race::Human, Class::Warrior).await;
    let second = harness::Client::connect(&realm, "LEAVER");
    assert!(ends(&first).await);
    assert!(realm.registry.online(g).is_some(), "left in the world");
    second.disconnect();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while realm.registry.online(g).is_some() {
        assert!(tokio::time::Instant::now() < deadline, "and let go");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Net 2: a character deleted from one session as another logs it in.
/// Before, both went through (the delete told it succeeded, the login in
/// the world) and the login's first save wrote the character back, its
/// name freed meanwhile. Now the second session starts once the first has
/// gone, so one of them happens: the login, the player left in the world
/// and the delete refused; or the delete, and no login.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_character_deleted_as_it_logs_in_is_one_or_the_other() {
    let realm = harness::realm();
    for i in 0..6 {
        let name = format!("Lazarus{}", ["a", "b", "c", "d", "e", "f"][i]);
        let mut a = harness::Client::connect(&realm, "TWIN");
        a.send(create(&name)).await;
        assert_eq!(
            a.wait_for("create", created).await,
            WorldResult::CharCreateSuccess
        );
        let g = realm.registry.guid_by_name(&name).unwrap();
        a.send(CMSG_PLAYER_LOGIN { guid: g }).await;
        let mut b = harness::Client::connect(&realm, "TWIN");
        b.send(CMSG_CHAR_DELETE { guid: g }).await;
        let deleted = b
            .wait_for("delete", |m| match m {
                ServerOpcodeMessage::SMSG_CHAR_DELETE(d) => Some(d.result),
                _ => None,
            })
            .await
            == WorldResult::CharDeleteSuccess;
        let kept = realm
            .store
            .characters("TWIN")
            .await
            .iter()
            .any(|r| r.guid == crate::game::guid::counter(g));
        let online = realm.registry.online(g).is_some();
        eprintln!("round {i}: deleted {deleted}, kept {kept}, online {online}");
        assert_eq!(deleted, !kept, "a delete told is a delete done");
        assert!(!(deleted && online), "not deleted and in the world");
        assert_eq!(deleted, realm.registry.name(g).is_none());
        if online {
            // b takes them over, and leaves them, saved: still there.
            b.login(g).await;
            b.disconnect();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while realm.registry.online(g).is_some() {
                assert!(tokio::time::Instant::now() < deadline);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(
                realm
                    .store
                    .characters("TWIN")
                    .await
                    .iter()
                    .any(|r| r.guid == crate::game::guid::counter(g))
            );
        }
    }
}

/// Net 2: two creates from two sessions of an account, at once, stay
/// within its limit, where both passed the count before either was made.
#[tokio::test]
async fn two_sessions_of_an_account_create_within_its_limit() {
    let realm = harness::realm();
    let limit = realm.config().max_characters_per_account;
    let mut a = harness::Client::connect(&realm, "MANY");
    for i in 0..limit - 1 {
        let name = format!("Many{}", char::from(b'a' + i as u8));
        a.send(create(&name)).await;
        assert_eq!(
            a.wait_for("create", created).await,
            WorldResult::CharCreateSuccess
        );
    }
    a.send(create("Lasta")).await;
    let mut b = harness::Client::connect(&realm, "MANY");
    b.send(create("Lastb")).await;
    b.wait_for("create", created).await;
    let n = realm.store.characters("MANY").await.len();
    assert_eq!(n, limit, "{n} characters, the limit {limit}");
}

fn gm_realm() -> Arc<crate::game::Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = true;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    crate::game::Realm::new(
        cfg,
        store,
        startup,
        crate::game::data::GameData::empty(),
        None,
    )
}

/// Net 3: `.ban` reaches the account's session on the character screen,
/// where it kicked only a player in the world, and the session picked a
/// character and was in, banned.
#[tokio::test]
async fn a_ban_reaches_a_session_on_the_character_screen() {
    crate::accounts::install(crate::accounts::Accounts::in_memory(
        crate::accounts::Creation::Auto,
        false,
    ));
    let accounts = crate::accounts::get().unwrap();
    accounts.set_password("EXILE", "EXILE").unwrap();
    let realm = gm_realm();
    let mut warden = harness::Client::connect(&realm, "WARDEN");
    warden.enter("Warden", Race::Human, Class::Warrior).await;
    let mut exile = harness::Client::connect(&realm, "EXILE");
    exile.send(create("Exile")).await;
    assert_eq!(
        exile.wait_for("create", created).await,
        WorldResult::CharCreateSuccess
    );
    warden.say(".ban exile perm testing").await;
    let reply = warden
        .wait_for("the GM's answer", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => Some(c.message.clone()),
            _ => None,
        })
        .await;
    assert_eq!(reply, "EXILE is banned.");
    assert!(ends(&exile).await, "the session on the screen is kicked");
}

/// Net 3: a session on the character screen of an account banned since
/// its handshake (a ban that didn't kick it) can't log a character in:
/// refused, and disconnected.
#[tokio::test]
async fn a_banned_account_logs_nobody_in() {
    crate::accounts::install(crate::accounts::Accounts::in_memory(
        crate::accounts::Creation::Auto,
        false,
    ));
    let accounts = crate::accounts::get().unwrap();
    accounts.set_password("BANNED", "BANNED").unwrap();
    let realm = harness::realm();
    let mut c = harness::Client::connect(&realm, "BANNED");
    c.send(create("Banned")).await;
    assert_eq!(
        c.wait_for("create", created).await,
        WorldResult::CharCreateSuccess
    );
    let g = realm.registry.guid_by_name("Banned").unwrap();
    assert!(accounts.ban("BANNED", -1, "testing"));
    c.send(CMSG_PLAYER_LOGIN { guid: g }).await;
    assert!(ends(&c).await, "disconnected");
    assert!(realm.registry.online(g).is_none(), "nobody in the world");
    // Told so, if the refusal was taken before the close dropped it.
    let answer = c
        .gets(Duration::from_millis(100), |m| match m {
            ServerOpcodeMessage::SMSG_LOGIN_VERIFY_WORLD(_) => Some(None),
            ServerOpcodeMessage::SMSG_CHARACTER_LOGIN_FAILED(f) => Some(Some(f.result)),
            _ => None,
        })
        .await;
    assert!(
        matches!(answer, None | Some(Some(WorldResult::CharLoginFailed))),
        "{answer:?}"
    );
}
