//! Phase 3 against Thistle Tea's social and group tests
//! (`test/game/world/entity/player/social_test.exs`, `groups_test.exs`):
//! friends and ignores, their failures, removals and deleted characters,
//! the ignored told, an ignored inviter refused, and random rolls.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::harness::{self, Client};
use crate::game::social::friends::SMSG_FRIEND_STATUS;

/// A realm whose players aren't GMs.
fn players_realm() -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, GameData::empty(), None)
}

/// The next `SMSG_FRIEND_STATUS`: its result and the GUID it names, and
/// for an online friend their status, zone, level and class.
async fn friend_status(c: &mut Client) -> (u8, u64, Option<(u8, u32, u32, u32)>) {
    let b = c
        .wait_frame("SMSG_FRIEND_STATUS", SMSG_FRIEND_STATUS, |_| true)
        .await;
    let guid = u64::from_le_bytes(b[1..9].try_into().unwrap());
    let online = (b.len() > 9).then(|| {
        let u32_at = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        (b[9], u32_at(10), u32_at(14), u32_at(18))
    });
    (b[0], guid, online)
}

/// Whether no `SMSG_FRIEND_STATUS` comes in a moment, read or raw.
async fn no_friend_status(c: &mut Client) -> bool {
    let read = c
        .gets(Duration::from_millis(300), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_FRIEND_STATUS(_)).then_some(())
        })
        .await
        .is_some();
    let raw = c
        .raw
        .iter()
        .any(|f| u16::from_le_bytes([f[2], f[3]]) == SMSG_FRIEND_STATUS);
    !read && !raw
}

async fn logout(c: &mut Client) {
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
}

#[tokio::test]
async fn adding_a_friend_or_ignore_tells_why_not_and_an_online_friends_where_and_what() {
    let realm = players_realm();
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Mage).await;
    let mut o = Client::connect(&realm, "O");
    let go = o.enter("Grom", Race::Orc, Class::Warrior).await;
    logout(&mut b).await;
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let add = |name: &str| CMSG_ADD_FRIEND { name: name.into() };
    // Bob, offline, by any case; again; herself; an enemy; no one.
    a.send(add("bob")).await;
    assert_eq!(friend_status(&mut a).await, (0x07, gb.guid(), None));
    a.send(add("Bob")).await;
    assert_eq!(friend_status(&mut a).await, (0x08, gb.guid(), None));
    a.send(add("Alice")).await;
    assert_eq!(friend_status(&mut a).await, (0x09, ga.guid(), None));
    a.send(add("Grom")).await;
    assert_eq!(friend_status(&mut a).await, (0x0A, go.guid(), None));
    a.send(add("Missing")).await;
    assert_eq!(friend_status(&mut a).await, (0x04, 0, None));
    let ignore = |name: &str| CMSG_ADD_IGNORE { name: name.into() };
    a.send(ignore("Missing")).await;
    assert_eq!(friend_status(&mut a).await, (0x0D, 0, None));
    a.send(ignore("Grom")).await;
    assert_eq!(friend_status(&mut a).await, (0x0F, go.guid(), None));
    // Bob back, busy: Alice hears he is online, busy, where and what.
    b.login(gb).await;
    let (result, guid, _) = friend_status(&mut a).await;
    assert_eq!((result, guid), (0x02, gb.guid()));
    b.send(CMSG_MESSAGECHAT {
        chat_type: CMSG_MESSAGECHAT_ChatType::Dnd,
        language: Language::Common,
        message: "Busy".into(),
    })
    .await;
    b.send(CMSG_PLAYED_TIME {}).await;
    b.wait_for("SMSG_PLAYED_TIME", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_PLAYED_TIME(_)).then_some(())
    })
    .await;
    a.send(CMSG_DEL_FRIEND { guid: gb }).await;
    assert_eq!(friend_status(&mut a).await.0, 0x05);
    a.send(add("Bob")).await;
    let (result, guid, online) = friend_status(&mut a).await;
    let zone = realm.registry.online(gb).unwrap().zone();
    assert_eq!((result, guid), (0x06, gb.guid()));
    assert_eq!(online, Some((4, zone, 1, 8)));
}

#[tokio::test]
async fn a_friend_removed_is_heard_no_more_an_ignore_kept_and_the_deleted_go_from_both_lists() {
    let realm = players_realm();
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Mage).await;
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.send(CMSG_ADD_FRIEND { name: "Bob".into() }).await;
    assert_eq!(friend_status(&mut a).await.0, 0x06);
    a.send(CMSG_ADD_IGNORE { name: "Bob".into() }).await;
    assert_eq!(friend_status(&mut a).await.0, 0x0F);
    a.send(CMSG_DEL_FRIEND { guid: gb }).await;
    assert_eq!(friend_status(&mut a).await, (0x05, gb.guid(), None));
    // Bob goes and comes: Alice isn't told.
    logout(&mut b).await;
    b.login(gb).await;
    a.raw.clear();
    a.clear();
    assert!(no_friend_status(&mut a).await);
    // Still ignored: Bob is told so when Alice's client hides his line;
    // one not ignored isn't.
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Carol", Race::Human, Class::Priest).await;
    let ignored = async |b: &mut Client| {
        b.gets(Duration::from_millis(300), |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c)
                if matches!(c.chat_type, SMSG_MESSAGECHAT_ChatType::Ignored { sender2 } if sender2 == ga) =>
            {
                Some(c.message.clone())
            }
            _ => None,
        })
        .await
    };
    a.send(CMSG_CHAT_IGNORED { guid: gc }).await;
    assert_eq!(ignored(&mut c).await, None);
    a.send(CMSG_CHAT_IGNORED { guid: gb }).await;
    assert_eq!(ignored(&mut b).await.as_deref(), Some("Alice"));
    // An ignored inviter is refused; unignored, heard.
    b.send(CMSG_GROUP_INVITE {
        name: "Alice".into(),
    })
    .await;
    let refused = b
        .wait_for("SMSG_PARTY_COMMAND_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_PARTY_COMMAND_RESULT(r) => Some(r.result),
            _ => None,
        })
        .await;
    assert_eq!(refused, PartyResult::IgnoringYou);
    a.send(CMSG_DEL_IGNORE { guid: gb }).await;
    assert_eq!(friend_status(&mut a).await, (0x10, gb.guid(), None));
    b.send(CMSG_GROUP_INVITE {
        name: "alice".into(),
    })
    .await;
    a.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    // Carol a friend and ignored, then deleted: gone from both lists.
    a.send(CMSG_ADD_FRIEND {
        name: "Carol".into(),
    })
    .await;
    a.send(CMSG_ADD_IGNORE {
        name: "Carol".into(),
    })
    .await;
    logout(&mut c).await;
    c.send(CMSG_CHAR_DELETE { guid: gc }).await;
    c.wait_for("SMSG_CHAR_DELETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_CHAR_DELETE(_)).then_some(())
    })
    .await;
    a.clear();
    a.send(CMSG_FRIEND_LIST {}).await;
    let friends = a
        .wait_for("SMSG_FRIEND_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_FRIEND_LIST(l) => {
                Some(l.friends.iter().map(|f| f.guid).collect::<Vec<_>>())
            }
            _ => None,
        })
        .await;
    let ignores = a
        .wait_for("SMSG_IGNORE_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_IGNORE_LIST(l) => Some(l.ignored.clone()),
            _ => None,
        })
        .await;
    assert!(!friends.contains(&gc), "{friends:?}");
    assert!(!ignores.contains(&gc.guid()), "{ignores:?}");
}

/// The rolls `c` hears in a moment: (who, result, minimum, maximum).
async fn rolls(c: &mut Client) -> Vec<(u64, u32, u32, u32)> {
    let mut out = Vec::new();
    while let Some(r) = c
        .gets(Duration::from_millis(200), |m| match m {
            ServerOpcodeMessage::MSG_RANDOM_ROLL(r) => {
                Some((r.guid.guid(), r.actual_roll, r.minimum, r.maximum))
            }
            _ => None,
        })
        .await
    {
        out.push(r);
    }
    out
}

#[tokio::test]
async fn a_roll_is_the_groups_while_in_it_once_alone_and_refused_reversed_or_past_ten_thousand() {
    let realm = players_realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Mage).await;
    a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
    b.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    a.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
        _ => None,
    })
    .await;
    let roll = |minimum, maximum| MSG_RANDOM_ROLL_Client { minimum, maximum };
    a.send(roll(1, 100)).await;
    let (heard_a, heard_b) = (rolls(&mut a).await, rolls(&mut b).await);
    assert_eq!(heard_a.len(), 1);
    assert_eq!(heard_a, heard_b, "one roll, the same to both");
    // Bob gone, each rolls alone, and hears it once.
    b.send(CMSG_GROUP_DISBAND {}).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    a.clear();
    b.clear();
    for (c, g) in [(&mut a, ga), (&mut b, gb)] {
        c.send(roll(7, 7)).await;
        assert_eq!(rolls(c).await, vec![(g.guid(), 7, 7, 7)]);
    }
    // Reversed, or past what mangos takes: nothing.
    for (minimum, maximum) in [(100, 1), (0, 10_001), (1, u32::MAX)] {
        a.send(roll(minimum, maximum)).await;
        assert!(rolls(&mut a).await.is_empty(), "{minimum} to {maximum}");
    }
    a.send(roll(0, 10_000)).await;
    assert_eq!(rolls(&mut a).await.len(), 1);
}
