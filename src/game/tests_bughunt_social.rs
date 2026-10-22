//! The bug hunt's findings on mail, auctions and the social services
//! (`social`), each shown through the protocol.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey};
use crate::game::social::auction::Auction;
use crate::game::social::mail;
use crate::game::store::ItemRow;
use crate::game::tests_p3_items::{AXE, auction_data, auctioneer};

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

/// An auction of an axe in the goblins' market, by character 99.
fn axe(id: u32, expires_at: i64) -> Auction {
    Auction {
        id,
        market: 7,
        house: 7,
        item: ItemRow {
            guid: 800_000 + id,
            entry: AXE,
            count: 1,
            ..Default::default()
        },
        owner: 99,
        start_bid: 100,
        buyout: 1000,
        expires_at,
        ..Default::default()
    }
}

/// The bidder list looked each auction up in the client's list of ids with
/// a linear scan, under the realm's auctions lock: 16,000 ids against
/// 20,000 auctions held every auction house in the realm for seconds. The
/// ids are sorted, and searched so.
#[tokio::test]
async fn a_long_outbid_list_is_answered_at_once() {
    let realm = harness::realm_with(auction_data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(auctioneer()).await;
    let later = mail::now() + 7200;
    {
        let mut auctions = realm.social.auctions.lock();
        for id in 1..=20_000 {
            auctions.auctions.insert(id, axe(id, later));
        }
    }
    let asked: Vec<u32> = (100_000..116_000).chain([7]).collect();
    a.clear();
    let started = Instant::now();
    a.send(CMSG_AUCTION_LIST_BIDDER_ITEMS {
        auctioneer: auctioneer(),
        start_from_page: 0,
        outbid_item_ids: asked,
    })
    .await;
    let bids = a
        .wait_for("SMSG_AUCTION_BIDDER_LIST_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_AUCTION_BIDDER_LIST_RESULT(l) => {
                Some(l.auctions.iter().map(|x| x.id).collect::<Vec<_>>())
            }
            _ => None,
        })
        .await;
    let took = started.elapsed();
    assert_eq!(bids, vec![7]);
    assert!(took < Duration::from_millis(500), "{took:?}");
}

/// Packets before this are handled: the name query is answered after them.
async fn handled(a: &mut Client) {
    a.send(CMSG_NAME_QUERY { guid: a.guid }).await;
    a.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
}

/// A built-in channel's name with any zone after it ("General - 1",
/// "General - 2") made a channel that was never taken out, empty: one for
/// every packet, for good, each scanned at every zone change and logout.
/// An empty built-in channel goes as a custom one does, and the next join
/// makes it again.
#[tokio::test]
async fn an_empty_built_in_channel_goes() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    handled(&mut a).await;
    let before = realm.social.channels.lock().count();
    for i in 0..20 {
        let name = format!("General - {i}");
        a.send(CMSG_JOIN_CHANNEL {
            channel_name: name.clone(),
            channel_password: String::new(),
        })
        .await;
        a.send(CMSG_LEAVE_CHANNEL { channel_name: name }).await;
    }
    a.send(CMSG_JOIN_CHANNEL {
        channel_name: "Trade - Nowhere".into(),
        channel_password: String::new(),
    })
    .await;
    handled(&mut a).await;
    assert_eq!(realm.social.channels.lock().count(), before + 1);
    a.send(CMSG_LEAVE_CHANNEL {
        channel_name: "Trade - Nowhere".into(),
    })
    .await;
    handled(&mut a).await;
    assert_eq!(realm.social.channels.lock().count(), before);
}

async fn set_money(realm: &Arc<Realm>, g: Guid, money: u32) {
    on_map(realm, move |m| {
        if let Some(p) = m.players.get_mut(&g) {
            p.set_money(money);
        }
    })
    .await;
}

fn letter(to: &str, body: &str) -> CMSG_SEND_MAIL {
    CMSG_SEND_MAIL {
        mailbox: Guid::zero(),
        receiver: to.into(),
        subject: "A word".into(),
        body: body.into(),
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

/// The text `c` is told for `id`.
async fn text_of(c: &mut Client, id: u32) -> String {
    c.clear();
    c.send(CMSG_ITEM_TEXT_QUERY {
        item_text_id: id,
        mail_id: id,
        unknown1: 0,
    })
    .await;
    c.wait_for("SMSG_ITEM_TEXT_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_TEXT_QUERY_RESPONSE(t) if t.item_text_id == id => {
            Some(t.text.clone())
        }
        _ => None,
    })
    .await
}

/// `CMSG_ITEM_TEXT_QUERY` answered any mail's body, and any letter's text,
/// to whoever asked its id: ids are sequential, so a client walking them
/// read every letter and auction invoice in the realm. A mail's body is
/// its receiver's to read, a letter's text its holder's.
#[tokio::test]
async fn a_mail_or_letter_is_read_only_by_its_reader() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    c.enter("Carol", Race::Human, Class::Warrior).await;
    set_money(&realm, ga, 1000).await;
    a.send(letter("Bob", "meet me at the bridge")).await;
    let id = sent(&mut a).await;
    assert_eq!(text_of(&mut c, id).await, "", "not Carol's");
    assert_eq!(text_of(&mut a, id).await, "", "nor its sender's, sent");
    assert_eq!(text_of(&mut b, id).await, "meet me at the bridge");

    // Bob copies it onto a letter: its text is his, holding it.
    b.send(CMSG_MAIL_CREATE_TEXT_ITEM {
        mailbox: Guid::zero(),
        mail_id: id,
        mail_template_id: 0,
    })
    .await;
    let deadline = Instant::now() + Duration::from_secs(5);
    let text_id = loop {
        let held = on_map(&realm, move |m| {
            m.players[&gb]
                .items
                .iter()
                .find(|i| i.entry == mail::LETTER_ITEM)
                .map(|i| {
                    i.fields
                        .get_u32(crate::game::fields::ITEM_FIELD_ITEM_TEXT_ID)
                })
        })
        .await;
        if let Some(t) = held {
            break t;
        }
        assert!(Instant::now() < deadline, "no letter");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert_ne!(text_id, id);
    assert_eq!(text_of(&mut c, text_id).await, "", "not Carol's");
    assert_eq!(text_of(&mut b, text_id).await, "meet me at the bridge");}

/// A cash-on-delivery payment that expired untaken went back to the buyer,
/// who had the item too. It goes away, as mangos's
/// `ReturnOrDeleteOldMails` takes it; a plain mail with money still goes
/// back.
#[test]
fn an_expired_cod_payment_is_not_sent_back() {
    let realm = harness::realm();
    let now = mail::now();
    let (buyer, seller) = (11, 12);
    let expired = |flags: u32| mail::Mail {
        kind: mail::NORMAL,
        sender: buyer,
        receiver: seller,
        money: 500,
        deliver_at: now - 30 * mail::DAY,
        expires_at: now - 1,
        flags,
        ..Default::default()
    };
    let payment = mail::post(&realm, expired(mail::COD_PAYMENT));
    let plain = mail::post(&realm, expired(0));
    mail::sweep(&realm);
    let mails = realm.social.mail.lock();
    assert!(!mails.mails.contains_key(&payment), "the payment is gone");
    let back: Vec<u32> = mails
        .mails
        .values()
        .filter(|m| m.receiver == buyer)
        .map(|m| m.id)
        .collect();
    assert_eq!(back, vec![plain], "only the plain mail went back");
}

/// The sweep found a mail expired, let the lock go, and acted later on
/// whatever had that id: a mail its receiver had sent back meanwhile, with
/// 30 days more, was taken as one returned and expired, and deleted with
/// its item. Its expiry is read again.
#[test]
fn a_mail_sent_back_since_the_sweep_found_it_is_kept() {
    let realm = harness::realm();
    let now = mail::now();
    // As `send_back` leaves it: returned, 30 days from now.
    let id = mail::post(
        &realm,
        mail::Mail {
            kind: mail::NORMAL,
            sender: 12,
            receiver: 11,
            item: Some(ItemRow {
                guid: 900_001,
                entry: AXE,
                count: 1,
                ..Default::default()
            }),
            deliver_at: now,
            expires_at: now + 30 * mail::DAY,
            flags: mail::RETURNED,
            ..Default::default()
        },
    );
    let mut mails = realm.social.mail.lock();
    mail::expire(&realm, &mut mails, id, now);
    assert!(
        mails.mails.get(&id).is_some_and(|m| m.item.is_some()),
        "kept, item and all"
    );
}

/// `leader` invites `member` by `name`, who accepts.
async fn group(leader: &mut Client, member: &mut Client, name: &str) {
    leader.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    member
        .wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
    member.send(CMSG_GROUP_ACCEPT {}).await;
    member
        .wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) if !l.members.is_empty() => Some(()),
            _ => None,
        })
        .await;
}

/// The roster a group was sent as a member logged out was worked out with
/// them still online: it showed them connected (the raid UI's online flag
/// reads it) until the next roster change.
#[tokio::test]
async fn the_roster_at_a_logout_shows_the_member_offline() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    c.enter("Carol", Race::Human, Class::Warrior).await;
    group(&mut a, &mut b, "Bob").await;
    group(&mut a, &mut c, "Carol").await;
    a.clear();
    b.send(CMSG_LOGOUT_REQUEST {}).await;
    harness::offline(&realm, gb).await;
    let bob_online = a
        .wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => l
                .members
                .iter()
                .find(|m| m.guid == gb)
                .map(|m| m.is_online),
            _ => None,
        })
        .await;
    assert!(!bob_online, "Bob is shown offline");
}

/// Whether `c` is sent a roster naming `other` within a moment.
async fn sent_roster_with(c: &mut Client, other: Guid) -> bool {
    handled(c).await;
    c.gets(Duration::from_millis(200), |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => {
            l.members.iter().any(|m| m.guid == other).then_some(())
        }
        _ => None,
    })
    .await
    .is_some()
}

/// A player in a battleground's raid who formed or joined a group of their
/// own was sent its rosters, over the raid's on their client: only one
/// made away by `bg_joined` was kept off them. One who forms it (inviting)
/// and one who joins it (accepting) are away from it too.
#[tokio::test]
async fn a_group_made_in_a_battleground_leaves_its_raid_roster_alone() {
    use crate::game::social::groups;
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Carol", Race::Human, Class::Warrior).await;
    let mut d = Client::connect(&realm, "D");
    let gd = d.enter("Dave", Race::Human, Class::Warrior).await;
    groups::bg_joined(&realm, ga, "Alice".into(), 1, 0);
    groups::bg_joined(&realm, gd, "Dave".into(), 2, 0);
    // Alice, in a match, invites Bob.
    group(&mut a, &mut b, "Bob").await;
    assert!(!sent_roster_with(&mut a, gb).await, "Alice keeps her raid's");
    // Dave, in a match, accepts Carol's invite.
    c.send(CMSG_GROUP_INVITE {
        name: "Dave".into(),
    })
    .await;
    d.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    d.send(CMSG_GROUP_ACCEPT {}).await;
    c.wait_for("Carol's roster", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => {
            l.members.iter().any(|m| m.guid == gd).then_some(())
        }
        _ => None,
    })
    .await;
    assert!(!sent_roster_with(&mut d, gc).await, "Dave keeps his raid's");
}
