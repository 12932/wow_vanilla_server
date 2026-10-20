//! Review 5's findings on the realm-wide state (`rev5-social` in
//! docs/review/review5.md): mail, the auctions' refunds and the meeting
//! stones, each shown through the protocol.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;
use crate::game::social::mail::{self, DAY, Mail};

async fn set_money(realm: &Arc<Realm>, g: Guid, money: u32) {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(move |m| {
            if let Some(p) = m.players.get_mut(&g) {
                p.set_money(money);
            }
        })
        .await
        .expect("the map answers");
}

fn letter(to: &str, subject: String, body: String) -> CMSG_SEND_MAIL {
    CMSG_SEND_MAIL {
        mailbox: Guid::zero(),
        receiver: to.into(),
        subject,
        body,
        unknown1: 0,
        unknown2: 0,
        item: Guid::zero(),
        money: Gold::new(0),
        cash_on_delivery_amount: 0,
        unknown3: 0,
        unknown4: 0,
    }
}

async fn sent(a: &mut Client) -> u32 {
    a.wait_for("SMSG_SEND_MAIL_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_SEND_MAIL_RESULT(r) => matches!(
            r.action,
            SMSG_SEND_MAIL_RESULT_MailAction::Send {
                result2: SMSG_SEND_MAIL_RESULT_MailResultTwo::Ok
            }
        )
        .then_some(r.mail_id),
        _ => None,
    })
    .await
}

/// Bob's mail list as the writer framed it: `[size BE][opcode LE][body]`,
/// checked to be framed whole, and as wow_messages reads it.
async fn mail_list(b: &mut Client) -> (Arc<[u8]>, Vec<wow_world_messages::vanilla::Mail>) {
    let before = b.captured().len();
    b.send(CMSG_GET_MAIL_LIST {
        mailbox: Guid::zero(),
    })
    .await;
    let deadline = Instant::now() + Duration::from_secs(5);
    let frame = loop {
        if let Some(f) = b.captured()[before..]
            .iter()
            .find(|f| u16::from_le_bytes([f[2], f[3]]) == 0x23B)
        {
            break f.clone();
        }
        assert!(Instant::now() < deadline, "no mail list");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    let said = usize::from(u16::from_be_bytes([frame[0], frame[1]])) + 2;
    assert_eq!(
        said,
        frame.len(),
        "the header's size is the frame's: else the client reads the rest as packets"
    );
    let mails = b
        .wait_for("SMSG_MAIL_LIST_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_MAIL_LIST_RESULT(l) => Some(l.mails.clone()),
            _ => None,
        })
        .await;
    (frame, mails)
}

/// rev5-social 1 (review 5's #3): two mails of 40,000-letter subjects made
/// Bob's mail list an 80,155-byte frame whose 16-bit size said 14,619, so
/// his client lost the stream each time he opened a mailbox. A subject is
/// kept as the client's box takes it (64 letters), a body to
/// `BODY_BYTES`, and the list stops at the client's cap, as mangos's.
#[tokio::test]
async fn a_long_subject_is_cut_and_the_mail_list_fits_the_client() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    set_money(&realm, ga, 1000).await;
    for _ in 0..2 {
        a.send(letter("Bob", "x".repeat(40_000), String::new()))
            .await;
        sent(&mut a).await;
    }
    // A body as long as one packet carries.
    a.send(letter("Bob", "Long".into(), "y".repeat(60_000)))
        .await;
    let long = sent(&mut a).await;
    b.clear();
    let (frame, mails) = mail_list(&mut b).await;
    assert_eq!(mails.len(), 3);
    assert!(frame.len() - 4 <= mail::LIST_BYTES, "{} bytes", frame.len());
    for m in mails.iter().filter(|m| m.message_id != long) {
        assert_eq!(m.subject, "x".repeat(mail::SUBJECT_LETTERS));
    }
    b.send(CMSG_ITEM_TEXT_QUERY {
        item_text_id: long,
        mail_id: long,
        unknown1: 0,
    })
    .await;
    // Read raw: wow_messages reads no text past 255 bytes (mangos-zero's
    // note on it says 8,000).
    let answer = b
        .wait_frame("SMSG_ITEM_TEXT_QUERY_RESPONSE", 0x244, |_| true)
        .await;
    assert_eq!(&answer[..4], &long.to_le_bytes());
    let text = &answer[4..answer.len() - 1];
    assert_eq!(text, "y".repeat(mail::BODY_BYTES).as_bytes());

    // A subject is cut at a letter's boundary, not a byte's.
    a.send(letter("Bob", "é".repeat(100), String::new())).await;
    let id = sent(&mut a).await;
    let kept = realm.social.mail.lock().mails[&id].subject.clone();
    assert_eq!(kept, "é".repeat(mail::SUBJECT_LETTERS));
}

/// rev5-social 1: a full mailbox of the longest subjects passes the
/// client's 32,767-byte cap (254 of 139 bytes); the list stops before it.
#[tokio::test]
async fn a_full_mailbox_lists_what_fits_the_clients_cap() {
    let realm = harness::realm();
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let now = mail::now();
    for i in 0..254 {
        mail::post(
            &realm,
            Mail {
                kind: mail::NORMAL,
                sender: 7,
                receiver: guid::counter(gb),
                subject: "z".repeat(mail::SUBJECT_LETTERS),
                deliver_at: now - i,
                expires_at: now + 30 * DAY,
                ..Default::default()
            },
        );
    }
    b.clear();
    let (frame, mails) = mail_list(&mut b).await;
    eprintln!("{} mails listed in {} bytes", mails.len(), frame.len());
    assert!(frame.len() - 4 <= mail::LIST_BYTES, "{} bytes", frame.len());
    assert!(
        (200..254).contains(&mails.len()),
        "as many as fit: {}",
        mails.len()
    );
}

/// rev5-social 4: a player queued alone at a meeting stone who logged out
/// stayed queued; as the longest waiting they led, and every sweep handed
/// the others to them, offline, and grouped none. A logout leaves the
/// queue (mangos's `LogoutPlayer`), and the sweep drops anyone offline.
#[tokio::test]
async fn one_queued_and_gone_doesnt_stall_a_meeting_stone() {
    const AREA: u32 = 1581; // the Deadmines
    let realm = harness::realm();
    let map0 = realm.maps.get(&realm, MapKey::world(0));
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Gone", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bee", Race::Human, Class::Priest).await;
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Cee", Race::Human, Class::Mage).await;
    for g in [ga, gb, gc] {
        map0.query(move |m| crate::game::map::meeting_stones::queue(m, g, AREA))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    // The social logout runs as the map lets the player go.
    let deadline = Instant::now() + Duration::from_secs(5);
    while realm.registry.online(ga).is_some() {
        assert!(Instant::now() < deadline, "Gone never went offline");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        realm.social.meeting_stones.lock().of(ga).is_none(),
        "a logout leaves the queue"
    );
    crate::game::map::meeting_stones::sweep(&realm);
    let groups = realm.social.groups.lock();
    let bee = groups.group_of(gb).expect("Bee is grouped");
    assert!(bee.members.iter().any(|m| m.guid == gc), "with Cee");
}

/// rev5-social 4: one gone without the social logout (a player lost with
/// their map) is dropped by the sweep, which groups those online.
#[tokio::test]
async fn the_sweep_drops_one_queued_and_offline() {
    const AREA: u32 = 1581;
    let realm = harness::realm();
    let map0 = realm.maps.get(&realm, MapKey::world(0));
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bee", Race::Human, Class::Priest).await;
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Cee", Race::Human, Class::Mage).await;
    // The longest waiting, and offline.
    let ghost = guid::player(99_999);
    realm
        .social
        .meeting_stones
        .lock()
        .queue
        .push(crate::game::map::meeting_stones::Queued {
            guid: ghost,
            area: AREA,
            team: crate::game::registry::Team::Alliance,
            class: 1,
            leader: false,
            since: Instant::now() - Duration::from_secs(60),
            progress_at: Instant::now() + Duration::from_secs(300),
        });
    for g in [gb, gc] {
        map0.query(move |m| crate::game::map::meeting_stones::queue(m, g, AREA))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    crate::game::map::meeting_stones::sweep(&realm);
    assert!(realm.social.meeting_stones.lock().of(ghost).is_none());
    let groups = realm.social.groups.lock();
    let bee = groups.group_of(gb).expect("Bee is grouped");
    assert!(bee.members.iter().any(|m| m.guid == gc), "with Cee");
}
