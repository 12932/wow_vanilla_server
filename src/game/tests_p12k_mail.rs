//! Round 12, stage 12k: the mail as mangos-classic's `MailHandler.cpp`
//! runs it. Its opcodes want a mailbox within reach (`CheckMailBox`); a
//! mail read keeps its time (`HandleMailMarkAsRead`; Thistle Tea's cut it
//! to three days); postage is thirty copper, item or not; money and cash
//! on delivery don't go together, nor is a conjured or timed item sent
//! (`HandleSendMail`); a mail cash on delivery isn't deleted
//! (`HandleMailDelete`); and mail delayed across a restart is announced
//! when it comes.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::harness::{self, Client};
use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit};
use crate::game::data::{GameData, ItemTemplate};
use crate::game::guid;
use crate::game::social::mail::{self, DAY};

const GOOD: u32 = 60_990;
const CONJURED: u32 = 60_991;
const TIMED: u32 = 60_992;
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

fn data(d: &mut GameData) {
    let item = |entry: u32, flags: u32, duration: u32| ItemTemplate {
        entry,
        class: 15,
        name: format!("Thing {entry}"),
        display_id: 1000,
        stackable: 1,
        flags,
        duration,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    };
    d.items.insert(GOOD, item(GOOD, 0, 0));
    d.items.insert(CONJURED, item(CONJURED, 0x2, 0));
    d.items.insert(TIMED, item(TIMED, 0, 3_600));
}

/// Alice and Bob, of two accounts, side by side; Alice with money and one
/// of each thing.
async fn two(realm: &std::sync::Arc<crate::game::Realm>) -> (Client, Client, [Guid; 3]) {
    let mut a = Client::connect(realm, "P12KMAILA");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(realm, "P12KMAILB");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    let items = world(realm)
        .query(move |m| {
            kit::ready(m, ga, 10, &[]);
            m.players.get_mut(&ga).unwrap().set_money(10_000);
            [GOOD, CONJURED, TIMED].map(|e| {
                crate::game::map::inventory::add_item(m, ga, e, 1, false).unwrap();
                m.players[&ga]
                    .items
                    .iter()
                    .find(|i| i.entry == e)
                    .map(|i| i.guid)
                    .unwrap()
            })
        })
        .await
        .unwrap();
    (a, b, items)
}

fn letter(item: Guid, money: u32, cod: u32) -> CMSG_SEND_MAIL {
    CMSG_SEND_MAIL {
        mailbox: Guid::zero(),
        receiver: "Bob".into(),
        subject: "For you".into(),
        body: "Hello.".into(),
        unknown1: 0,
        unknown2: 0,
        item,
        money: Gold::new(money),
        cash_on_delivery_amount: cod,
        unknown3: 0,
        unknown4: 0,
    }
}

async fn sent(a: &mut Client) -> SMSG_SEND_MAIL_RESULT_MailResultTwo {
    a.wait_for("SMSG_SEND_MAIL_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_SEND_MAIL_RESULT(r) => match &r.action {
            SMSG_SEND_MAIL_RESULT_MailAction::Send { result2 } => Some(result2.clone()),
            _ => None,
        },
        _ => None,
    })
    .await
}

async fn money(realm: &std::sync::Arc<crate::game::Realm>, g: Guid) -> u32 {
    world(realm)
        .query(move |m| m.players[&g].money())
        .await
        .unwrap()
}

fn bobs_mail(realm: &crate::game::Realm, b: Guid) -> Vec<mail::Mail> {
    let mails = realm.social.mail.lock();
    mails
        .ids_of(guid::counter(b))
        .iter()
        .filter_map(|id| mails.mails.get(id).cloned())
        .collect()
}

/// The mailbox list is answered at a mailbox within reach alone: from
/// across the road, or naming no mailbox but a creature, nothing.
#[tokio::test]
async fn the_mail_is_read_at_a_mailbox_alone() {
    let realm = fast_realm(data);
    let (mut a, _b, _) = two(&realm).await;
    let map = world(&realm);
    let ga = a.guid;
    let (mailbox, kobold) = map
        .query(move |m| (harness::mailbox(m, ga), kit::spawn(m, ga, KOBOLD, 10, 3.0)))
        .await
        .unwrap();
    let listed = async |a: &mut Client, mailbox: Guid| {
        a.clear();
        a.send(CMSG_GET_MAIL_LIST { mailbox }).await;
        a.gets(Duration::from_millis(400), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_MAIL_LIST_RESULT(_)).then_some(())
        })
        .await
        .is_some()
    };
    assert!(listed(&mut a, mailbox).await, "at the mailbox");
    assert!(!listed(&mut a, kobold).await, "a kobold is no mailbox");
    a.heartbeat(START.0 + 30.0, START.1, START.2).await;
    assert!(!listed(&mut a, mailbox).await, "thirty yards off");
    a.heartbeat(START.0, START.1, START.2).await;
    assert!(listed(&mut a, mailbox).await, "back at it");
}

/// Postage is thirty copper with an item too; money with cash on delivery,
/// a conjured item and a timed one are refused, and cost nothing.
#[tokio::test]
async fn postage_is_thirty_copper_and_what_mangos_refuses_isnt_sent() {
    let realm = fast_realm(data);
    let (mut a, b, [good, conjured, timed]) = two(&realm).await;
    a.clear();
    a.send(letter(good, 0, 0)).await;
    assert_eq!(sent(&mut a).await, SMSG_SEND_MAIL_RESULT_MailResultTwo::Ok);
    assert_eq!(money(&realm, a.guid).await, 10_000 - 30);
    for (item, money_sent, cod, why) in [
        (Guid::zero(), 100, 100, "money with cash on delivery"),
        (conjured, 0, 0, "a conjured item"),
        (timed, 0, 0, "a timed item"),
    ] {
        a.clear();
        a.send(letter(item, money_sent, cod)).await;
        let result = sent(&mut a).await;
        assert_ne!(result, SMSG_SEND_MAIL_RESULT_MailResultTwo::Ok, "{why}");
    }
    assert_eq!(money(&realm, a.guid).await, 10_000 - 30);
    assert_eq!(bobs_mail(&realm, b.guid).len(), 1);
}

/// Read, a mail keeps its thirty days; one cash on delivery can't be
/// deleted, and stays.
#[tokio::test]
async fn a_read_mail_keeps_its_time_and_cash_on_delivery_isnt_deleted() {
    let realm = fast_realm(data);
    let (mut a, mut b, [good, _, _]) = two(&realm).await;
    a.send(letter(Guid::zero(), 50, 0)).await;
    assert_eq!(sent(&mut a).await, SMSG_SEND_MAIL_RESULT_MailResultTwo::Ok);
    a.clear();
    a.send(letter(good, 0, 500)).await;
    assert_eq!(sent(&mut a).await, SMSG_SEND_MAIL_RESULT_MailResultTwo::Ok);
    let mails = bobs_mail(&realm, b.guid);
    let plain = mails.iter().find(|m| m.cod == 0).unwrap().clone();
    let cod = mails.iter().find(|m| m.cod > 0).unwrap().clone();
    b.send(CMSG_MAIL_MARK_AS_READ {
        mailbox: Guid::zero(),
        mail_id: plain.id,
    })
    .await;
    b.clear();
    b.send(CMSG_MAIL_DELETE {
        mailbox_id: Guid::zero(),
        mail_id: cod.id,
    })
    .await;
    let deleted = b
        .wait_for("SMSG_SEND_MAIL_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_SEND_MAIL_RESULT(r) => match &r.action {
                SMSG_SEND_MAIL_RESULT_MailAction::Deleted { result2 } => Some(result2.clone()),
                _ => None,
            },
            _ => None,
        })
        .await;
    assert_ne!(deleted, SMSG_SEND_MAIL_RESULT_MailResultTwo::Ok);
    let after = bobs_mail(&realm, b.guid);
    let read = after.iter().find(|m| m.id == plain.id).unwrap();
    assert!(read.flags & mail::READ != 0, "read");
    assert_eq!(read.expires_at, plain.expires_at, "its thirty days kept");
    assert!(read.expires_at - mail::now() > 29 * DAY);
    assert!(after.iter().any(|m| m.id == cod.id), "cash on delivery stays");
}

/// Mail on its way an hour when the realm stops is announced when it
/// comes, after the restart: the next mail's time was not rebuilt.
#[tokio::test]
async fn mail_delayed_across_a_restart_is_announced_when_it_comes() {
    let dir = std::env::temp_dir().join(format!("wvs-p12k-mail-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("characters.sqlite");
    let mut config = harness::test_config();
    config.mail_delay = Duration::from_secs(3_600);
    let (store, startup) = crate::game::store::Store::open(&db).unwrap();
    let realm = crate::game::Realm::new(
        config.clone(),
        store,
        startup,
        data_with(data),
        None,
    );
    let (mut a, b, [good, _, _]) = two(&realm).await;
    let gb = b.guid;
    a.send(letter(good, 0, 0)).await;
    assert_eq!(sent(&mut a).await, SMSG_SEND_MAIL_RESULT_MailResultTwo::Ok);
    realm.shutdown().await;
    drop((a, b));
    let (store, startup) = crate::game::store::Store::open(&db).unwrap();
    let realm = crate::game::Realm::new(config, store, startup, data_with(data), None);
    let mut b = Client::connect(&realm, "P12KMAILB");
    b.login(gb).await;
    // The hour gone by.
    for m in realm.social.mail.lock().mails.values_mut() {
        m.deliver_at = mail::now() - 1;
    }
    b.clear();
    mail::sweep(&realm);
    b.wait_for("SMSG_RECEIVED_MAIL", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_RECEIVED_MAIL(_)).then_some(())
    })
    .await;
    realm.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}
