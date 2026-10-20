//! Thistle Tea's auth and world handshake tests (Phase 3, `auth` and
//! `game/network`), over TCP as a client speaks them: a login refused or
//! sent in pieces, a reconnect with and without a session, and the world
//! server's proof of the session key.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use wow_srp::client::{SrpClient, SrpClientUser};
use wow_srp::normalized_string::NormalizedString;
use wow_srp::server::SrpVerifier;
use wow_srp::vanilla_header::ProofSeed;
use wow_srp::{GENERATOR, LARGE_SAFE_PRIME_LITTLE_ENDIAN, PublicKey};
use wow_world_messages::vanilla::*;

use crate::auth::{UserCache, UserCacheInner, users_lock};

/// The accounts, as `existing` admits them: only those made.
fn accounts() -> &'static crate::accounts::Accounts {
    crate::accounts::install(crate::accounts::Accounts::in_memory(
        crate::accounts::Creation::Existing,
        false,
    ));
    crate::accounts::get().expect("the accounts")
}

fn users() -> UserCache {
    Arc::new(std::sync::Mutex::new(UserCacheInner::new()))
}

/// A login connection to the auth server's handler, as a client has it.
async fn login_server(users: UserCache) -> TcpStream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (s, _) = listener.accept().await.unwrap();
        crate::auth::handle(s, users).await;
    });
    TcpStream::connect(addr).await.unwrap()
}

/// The 1.12.1 client's logon (0) or reconnect (2) challenge, as Thistle
/// Tea's test builds it.
fn challenge(opcode: u8, username: &str) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"WoW\0");
    body.extend_from_slice(&[1, 12, 1]);
    body.extend_from_slice(&5875u16.to_le_bytes());
    body.extend_from_slice(b"68x\0niW\0SUne");
    body.extend_from_slice(&[0; 8]);
    body.push(username.len() as u8);
    body.extend_from_slice(username.as_bytes());
    let mut p = vec![opcode, 3];
    p.extend_from_slice(&(body.len() as u16).to_le_bytes());
    p.extend_from_slice(&body);
    p
}

/// What the server sends until it closes.
async fn rest(s: &mut TcpStream) -> Vec<u8> {
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut out))
        .await
        .expect("the server closes")
        .unwrap();
    out
}

async fn read_n(s: &mut TcpStream, n: usize) -> Vec<u8> {
    let mut out = vec![0; n];
    tokio::time::timeout(Duration::from_secs(10), s.read_exact(&mut out))
        .await
        .expect("the server answers")
        .unwrap();
    out
}

/// A whole SRP login on `s` as `name`, its password `password`: the
/// client's session.
async fn log_in(s: &mut TcpStream, name: &str, password: &str) -> SrpClient {
    s.write_all(&challenge(0, name)).await.unwrap();
    // Opcode, protocol, result, B, g's size and g, N's size and N, salt,
    // the CRC salt and the security flag.
    let c = read_n(s, 3 + 32 + 2 + 33 + 32 + 16 + 1).await;
    assert_eq!(&c[..3], &[0, 0, 0], "the challenge");
    let b: [u8; 32] = c[3..35].try_into().unwrap();
    assert_eq!((c[35], c[36], c[37]), (1, GENERATOR, 32));
    let salt: [u8; 32] = c[70..102].try_into().unwrap();
    let user = SrpClientUser::new(
        NormalizedString::new(name).unwrap(),
        NormalizedString::new(password).unwrap(),
    );
    let ch = user.into_challenge(
        GENERATOR,
        LARGE_SAFE_PRIME_LITTLE_ENDIAN,
        PublicKey::from_le_bytes(b).unwrap(),
        salt,
    );
    let mut proof = vec![1];
    proof.extend_from_slice(ch.client_public_key());
    proof.extend_from_slice(ch.client_proof());
    proof.extend_from_slice(&[0; 20]);
    proof.extend_from_slice(&[0, 0]);
    s.write_all(&proof).await.unwrap();
    let p = read_n(s, 2 + 20 + 4).await;
    assert_eq!(&p[..2], &[1, 0], "the proof accepted");
    ch.verify_server_proof(p[2..22].try_into().unwrap())
        .expect("the server's proof")
}

/// Thistle Tea's "unknown logins receive a failure instead of crashing".
#[tokio::test]
async fn an_unknown_login_is_told_so() {
    accounts();
    let mut s = login_server(users()).await;
    s.write_all(&challenge(0, "STRANGER")).await.unwrap();
    assert_eq!(rest(&mut s).await, vec![0, 0, 4], "FailUnknownAccount");
}

/// Thistle Tea's "fragmented valid logins still receive an SRP
/// challenge": the challenge in two pieces is answered once it is whole,
/// and the login goes through.
#[tokio::test]
async fn a_login_sent_in_pieces_is_answered_once_whole() {
    accounts().set_password("PIECES", "SECRET").unwrap();
    let users = users();
    let mut s = login_server(users.clone()).await;
    let c = challenge(0, "PIECES");
    s.write_all(&c[..5]).await.unwrap();
    let mut nothing = [0u8; 1];
    assert!(
        tokio::time::timeout(Duration::from_millis(200), s.read(&mut nothing))
            .await
            .is_err(),
        "nothing until the challenge is whole"
    );
    s.write_all(&c[5..]).await.unwrap();
    let head = read_n(&mut s, 3).await;
    assert_eq!(head, vec![0, 0, 0], "the SRP challenge");
}

/// Thistle Tea's reconnect tests: an account with no session is told so
/// and cut off; one whose client logged in reconnects with a proof over
/// its session key, and a wrong proof is refused.
#[tokio::test]
async fn a_reconnect_needs_a_session_and_its_proof() {
    accounts().set_password("RETURNER", "SECRET").unwrap();
    let users = users();
    // No such account, and an account with no session.
    for name in ["NOBODY", "RETURNER"] {
        let mut s = login_server(users.clone()).await;
        s.write_all(&challenge(2, name)).await.unwrap();
        assert_eq!(rest(&mut s).await, vec![2, 4], "{name}: FailUnknownAccount");
    }
    let mut s = login_server(users.clone()).await;
    let client = log_in(&mut s, "RETURNER", "SECRET").await;
    drop(s);
    // Its session is kept for the world and for a reconnect.
    assert!(users_lock(&users).get("RETURNER").is_some());
    for good in [true, false] {
        let mut s = login_server(users.clone()).await;
        s.write_all(&challenge(2, "RETURNER")).await.unwrap();
        let c = read_n(&mut s, 2 + 16 + 16).await;
        assert_eq!(&c[..2], &[2, 0], "the reconnect challenge");
        let values = client.calculate_reconnect_values(c[2..18].try_into().unwrap());
        let mut proof = vec![3];
        proof.extend_from_slice(&values.challenge_data);
        let mut p = values.proof;
        if !good {
            p[0] ^= 1;
        }
        proof.extend_from_slice(&p);
        proof.extend_from_slice(&[0; 20]);
        proof.push(0);
        s.write_all(&proof).await.unwrap();
        let answer = read_n(&mut s, 2).await;
        if good {
            assert_eq!(answer, vec![3, 0], "the proof accepted");
        } else {
            assert_ne!(answer[1], 0, "a wrong proof refused");
            assert!(rest(&mut s).await.is_empty(), "and the connection closed");
        }
    }
}

/// A wrong reconnect proof (a stale session) isn't told the account is
/// banned: it is answered as Thistle Tea answers it, `<<3, 1>>`
/// (`FailUnknown0`), and the connection closed. Both reconnects: 1.12's
/// protocol (3, as 2) and 8.
#[tokio::test]
async fn a_wrong_reconnect_proof_is_not_told_banned() {
    accounts().set_password("STALE", "SECRET").unwrap();
    let users = users();
    let mut s = login_server(users.clone()).await;
    let client = log_in(&mut s, "STALE", "SECRET").await;
    drop(s);
    for protocol in [3u8, 8] {
        let mut s = login_server(users.clone()).await;
        let mut c = challenge(2, "STALE");
        c[1] = protocol;
        s.write_all(&c).await.unwrap();
        let c = read_n(&mut s, 2 + 16 + 16).await;
        assert_eq!(
            &c[..2],
            &[2, 0],
            "protocol {protocol}: the reconnect challenge"
        );
        let values = client.calculate_reconnect_values(c[2..18].try_into().unwrap());
        let mut proof = vec![3];
        proof.extend_from_slice(&values.challenge_data);
        let mut p = values.proof;
        p[0] ^= 1;
        proof.extend_from_slice(&p);
        proof.extend_from_slice(&[0; 20]);
        proof.push(0);
        s.write_all(&proof).await.unwrap();
        let answer = read_n(&mut s, 2).await;
        assert_eq!(
            answer,
            vec![3, 1],
            "protocol {protocol}: FailUnknown0, not FailBanned"
        );
        rest(&mut s).await;
    }
}

/// A session key the auth server holds for `name`, as a login leaves it,
/// and the client's half of it.
fn session(users: &UserCache, name: &str) -> [u8; 40] {
    let n = || NormalizedString::new(name).unwrap();
    let proof = SrpVerifier::from_username_and_password(n(), n()).into_proof();
    let ch = SrpClientUser::new(n(), n()).into_challenge(
        GENERATOR,
        LARGE_SAFE_PRIME_LITTLE_ENDIAN,
        PublicKey::from_le_bytes(*proof.server_public_key()).unwrap(),
        *proof.salt(),
    );
    let (server, server_proof) = proof
        .into_server(
            PublicKey::from_le_bytes(*ch.client_public_key()).unwrap(),
            *ch.client_proof(),
        )
        .unwrap();
    let client = ch.verify_server_proof(server_proof).unwrap();
    users_lock(users).insert(name.to_owned(), server);
    client.session_key()
}

/// A world connection to `realm`, as `serve` takes one.
async fn world_server(realm: Arc<crate::game::Realm>) -> TcpStream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (s, _) = listener.accept().await.unwrap();
        let permit = Arc::new(tokio::sync::Semaphore::new(1))
            .try_acquire_owned()
            .unwrap();
        crate::game::session::serve(s, realm, permit).await;
    });
    TcpStream::connect(addr).await.unwrap()
}

/// The world handshake's first packet: `SMSG_AUTH_CHALLENGE` and its seed.
async fn auth_challenge(s: &mut TcpStream) -> u32 {
    let c = read_n(s, 4 + 4).await;
    assert_eq!(u16::from_be_bytes([c[0], c[1]]), 6, "its size");
    assert_eq!(
        u16::from_le_bytes([c[2], c[3]]),
        0x1EC,
        "SMSG_AUTH_CHALLENGE"
    );
    u32::from_le_bytes(c[4..8].try_into().unwrap())
}

async fn auth_session(s: &mut TcpStream, name: &str, seed: u32, proof: [u8; 20]) {
    CMSG_AUTH_SESSION {
        build: 5875,
        server_id: 0,
        username: name.to_owned(),
        client_seed: seed,
        client_proof: proof,
        addon_info: vec![],
    }
    .tokio_write_unencrypted_client(&mut *s)
    .await
    .unwrap();
}

/// Thistle Tea's session_key_test and server_test's challenge, and
/// crypto_test's unencrypted refusal: the world server opens with
/// `SMSG_AUTH_CHALLENGE`, takes the client's proof over the session key
/// the auth server holds (its answer encrypted with it), and refuses a
/// wrong proof or an account it has no session for with an unencrypted
/// `AUTH_FAILED` before closing.
#[tokio::test]
async fn the_world_takes_a_proof_over_the_session_key_and_refuses_others() {
    let users = users();
    let key = session(&users, "WORLDLY");
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(
        crate::game::harness::test_config(),
        store,
        startup,
        crate::game::data::GameData::empty(),
        Some(users),
    );
    // The right proof: SMSG_AUTH_RESPONSE AUTH_OK, encrypted.
    let mut s = world_server(realm.clone()).await;
    let server_seed = auth_challenge(&mut s).await;
    let seed = ProofSeed::new();
    let client_seed = seed.seed();
    let (proof, mut crypto) = seed.into_client_header_crypto(
        &NormalizedString::new("WORLDLY").unwrap(),
        key,
        server_seed,
    );
    auth_session(&mut s, "WORLDLY", client_seed, proof).await;
    let header = read_n(&mut s, 4).await;
    let h = crypto.decrypt_server_header(header.try_into().unwrap());
    assert_eq!(h.opcode, 0x1EE, "SMSG_AUTH_RESPONSE");
    let body = read_n(&mut s, usize::from(h.size) - 2).await;
    assert_eq!(body[0], 0x0C, "AUTH_OK");
    // A proof over the wrong seed, and an account with no session.
    for (name, wrong_seed) in [("WORLDLY", true), ("UNSEEN", false)] {
        let mut s = world_server(realm.clone()).await;
        let server_seed = auth_challenge(&mut s).await;
        let seed = ProofSeed::new();
        let client_seed = seed.seed();
        let (proof, _) = seed.into_client_header_crypto(
            &NormalizedString::new(name).unwrap(),
            key,
            if wrong_seed {
                server_seed ^ 1
            } else {
                server_seed
            },
        );
        auth_session(&mut s, name, client_seed, proof).await;
        let answer = rest(&mut s).await;
        assert_eq!(
            answer,
            vec![0, 3, 0xEE, 0x01, 0x0D],
            "{name}: AUTH_FAILED, in the clear"
        );
    }
}

/// A ban outlasts the session made before it: the reconnect the 1.12
/// client tries after the ban's kick is told the account is banned, as a
/// logon is, and the world won't take the old session key (mangos's
/// HandleAuthSession answers AUTH_BANNED), where both went on as before.
#[tokio::test]
async fn a_ban_outlasts_the_session_made_before_it() {
    accounts().set_password("OUTLAW", "SECRET").unwrap();
    let users = users();
    let mut s = login_server(users.clone()).await;
    log_in(&mut s, "OUTLAW", "SECRET").await;
    drop(s);
    assert!(accounts().ban("OUTLAW", -1, "testing"));
    for protocol in [3u8, 8] {
        let mut s = login_server(users.clone()).await;
        let mut c = challenge(2, "OUTLAW");
        c[1] = protocol;
        s.write_all(&c).await.unwrap();
        assert_eq!(
            rest(&mut s).await,
            vec![2, 3],
            "protocol {protocol}: FailBanned"
        );
    }
    let key = session(&users, "OUTLAW");
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(
        crate::game::harness::test_config(),
        store,
        startup,
        crate::game::data::GameData::empty(),
        Some(users),
    );
    let mut s = world_server(realm).await;
    let server_seed = auth_challenge(&mut s).await;
    let seed = ProofSeed::new();
    let client_seed = seed.seed();
    let (proof, _) =
        seed.into_client_header_crypto(&NormalizedString::new("OUTLAW").unwrap(), key, server_seed);
    auth_session(&mut s, "OUTLAW", client_seed, proof).await;
    assert_eq!(
        rest(&mut s).await,
        vec![0, 3, 0xEE, 0x01, 28],
        "AUTH_BANNED, in the clear"
    );
}

/// Review 5's net 4: a login as `casey` is CASEY's, at the login server
/// and in the world: its session key is kept under CASEY (where it was
/// kept as sent), a reconnect in any case finds it, and the world
/// handshake as `casey` enters as CASEY, with CASEY's characters, not as
/// an account of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_login_in_lower_case_is_the_account_in_capitals() {
    accounts().set_password("CASEY", "SECRET").unwrap();
    let users = users();
    let mut s = login_server(users.clone()).await;
    let client = log_in(&mut s, "casey", "SECRET").await;
    drop(s);
    assert!(
        users_lock(&users).get("CASEY").is_some(),
        "kept in capitals"
    );
    assert!(users_lock(&users).get("casey").is_none(), "not as sent");
    // A reconnect in lower case finds it.
    let mut s = login_server(users.clone()).await;
    s.write_all(&challenge(2, "Casey")).await.unwrap();
    let c = read_n(&mut s, 2 + 16 + 16).await;
    assert_eq!(&c[..2], &[2, 0], "the reconnect challenge");
    drop(s);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(
        crate::game::harness::test_config(),
        store,
        startup,
        crate::game::data::GameData::empty(),
        Some(users),
    );
    let mut s = world_server(realm.clone()).await;
    let server_seed = auth_challenge(&mut s).await;
    let seed = ProofSeed::new();
    let client_seed = seed.seed();
    let (proof, mut crypto) = seed.into_client_header_crypto(
        &NormalizedString::new("casey").unwrap(),
        client.session_key(),
        server_seed,
    );
    auth_session(&mut s, "casey", client_seed, proof).await;
    let header = read_n(&mut s, 4).await;
    let h = crypto.decrypt_server_header(header.try_into().unwrap());
    assert_eq!(h.opcode, 0x1EE, "SMSG_AUTH_RESPONSE");
    let body = read_n(&mut s, usize::from(h.size) - 2).await;
    assert_eq!(body[0], 0x0C, "AUTH_OK");
    CMSG_CHAR_CREATE {
        name: "Lower".into(),
        race: Race::Human,
        class: Class::Warrior,
        gender: Gender::Female,
        skin_color: 1,
        face: 1,
        hair_style: 1,
        hair_color: 1,
        facial_hair: 1,
    }
    .tokio_write_encrypted_client(&mut s, crypto.encrypter())
    .await
    .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let g = loop {
        if let Some(g) = realm.registry.guid_by_name("Lower") {
            break g;
        }
        assert!(tokio::time::Instant::now() < deadline, "no character made");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert_eq!(&*realm.registry.name(g).unwrap().account, "CASEY");
    assert_eq!(realm.store.characters("CASEY").await.len(), 1);
    assert!(realm.store.characters("casey").await.is_empty());
}

/// Review 5's net 2: one login's session key opens one world session at
/// a time. A second world connection with it is the account's session,
/// and the first is closed, where any number stayed open at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_world_session_of_an_account_closes_the_first() {
    let users = users();
    let key = session(&users, "DOUBLE");
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(
        crate::game::harness::test_config(),
        store,
        startup,
        crate::game::data::GameData::empty(),
        Some(users),
    );
    let mut sessions = Vec::new();
    for _ in 0..3 {
        let mut s = world_server(realm.clone()).await;
        let server_seed = auth_challenge(&mut s).await;
        let seed = ProofSeed::new();
        let client_seed = seed.seed();
        let (proof, mut crypto) = seed.into_client_header_crypto(
            &NormalizedString::new("DOUBLE").unwrap(),
            key,
            server_seed,
        );
        auth_session(&mut s, "DOUBLE", client_seed, proof).await;
        let header = read_n(&mut s, 4).await;
        let h = crypto.decrypt_server_header(header.try_into().unwrap());
        let body = read_n(&mut s, usize::from(h.size) - 2).await;
        assert_eq!((h.opcode, body[0]), (0x1EE, 0x0C), "AUTH_OK");
        sessions.push(s);
    }
    let last = sessions.pop().unwrap();
    for mut s in sessions {
        rest(&mut s).await;
    }
    let mut nothing = [0u8; 1];
    let mut last = last;
    assert!(
        tokio::time::timeout(Duration::from_millis(300), last.read(&mut nothing))
            .await
            .is_err(),
        "the last stays open"
    );
}
