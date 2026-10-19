//! Round 6's world tests, ported from Thistle Tea's `test/game/world/`
//! (docs/parity/matrix.csv): chat's audiences.

use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::harness::{self, Client};
use crate::game::tests_p3_combat::{START, data};
use crate::game::tests_p3_entity::NONE_WITHIN;
use crate::game::tests_p3_player2_ammo::handled;

/// `c` speaks `text` to `kind`, in Common.
async fn speak(c: &Client, kind: CMSG_MESSAGECHAT_ChatType, language: Language, text: &str) {
    c.send(CMSG_MESSAGECHAT {
        chat_type: kind,
        language,
        message: text.into(),
    })
    .await;
}

/// A chat line's kind, by its name ("Party", "RaidWarning").
fn kind(t: &SMSG_MESSAGECHAT_ChatType) -> String {
    format!("{t:?}")
        .chars()
        .take_while(|c| c.is_alphanumeric())
        .collect()
}

/// A chat line `c` hears: its kind, language and text.
type Line = (String, Language, String);

fn line(m: &ServerOpcodeMessage) -> Option<Line> {
    match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(x)
            if !matches!(x.chat_type, SMSG_MESSAGECHAT_ChatType::System { .. }) =>
        {
            Some((kind(&x.chat_type), x.language, x.message.clone()))
        }
        _ => None,
    }
}

/// The chat lines `c` hears within [`NONE_WITHIN`].
async fn lines(c: &mut Client) -> Vec<Line> {
    let mut heard = Vec::new();
    while let Some(l) = c.gets(NONE_WITHIN, line).await {
        heard.push(l);
    }
    heard
}

/// The next chat line `c` hears.
async fn next_line(c: &mut Client) -> Line {
    c.wait_for("a chat line", line).await
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

/// Thistle Tea's `Chat.handle` "restricts party chat to the subgroup and
/// authorizes raid leadership channels": in a raid, party chat is the
/// speaker's subgroup's; raid, raid-leader and raid-warning lines reach
/// all, but only from the leader or an assistant (mangos's
/// `HandleMessagechatOpcode`).
#[tokio::test]
async fn raid_party_chat_is_the_subgroups_and_raid_leading_lines_the_leaders() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    c.enter("Cid", Race::Human, Class::Warrior).await;
    group(&mut a, &mut b, "Bob").await;
    group(&mut a, &mut c, "Cid").await;
    a.send(CMSG_GROUP_RAID_CONVERT {}).await;
    a.send(CMSG_GROUP_CHANGE_SUB_GROUP {
        name: "Bob".into(),
        group_number: 1,
    })
    .await;
    handled(&mut a).await;
    for x in [&mut a, &mut b, &mut c] {
        handled(x).await;
        x.clear();
    }
    use CMSG_MESSAGECHAT_ChatType as T;
    speak(&a, T::Party, Language::Common, "subgroup").await;
    let party = ("Party".to_owned(), Language::Common, "subgroup".to_owned());
    assert_eq!(next_line(&mut a).await, party);
    assert_eq!(next_line(&mut c).await, party);
    for (kind, wire) in [
        (T::Raid, "Raid"),
        (T::RaidLeader, "RaidLeader"),
        (T::RaidWarning, "RaidWarning"),
    ] {
        speak(&a, kind, Language::Common, "raid").await;
        for x in [&mut a, &mut b, &mut c] {
            assert_eq!(
                next_line(x).await,
                (wire.to_owned(), Language::Common, "raid".to_owned())
            );
        }
    }
    for kind in [T::RaidLeader, T::RaidWarning] {
        speak(&b, kind, Language::Common, "forged").await;
    }
    handled(&mut b).await;
    assert_eq!(
        lines(&mut b).await,
        [],
        "Bob's in the second subgroup, and doesn't lead"
    );
    for x in [&mut a, &mut c] {
        assert_eq!(lines(x).await, [], "Bob doesn't lead");
    }
    a.send(CMSG_GROUP_ASSISTANT_LEADER {
        guid: gb,
        set_assistant: true,
    })
    .await;
    handled(&mut a).await;
    speak(&b, T::RaidWarning, Language::Common, "assistant").await;
    for x in [&mut a, &mut b, &mut c] {
        assert_eq!(
            next_line(x).await,
            (
                "RaidWarning".to_owned(),
                Language::Common,
                "assistant".to_owned()
            )
        );
    }
}

/// Thistle Tea's `Chat.handle` "does not turn unsupported audiences into
/// global chat": guild chat from one in no guild goes nowhere, not to
/// those near.
#[tokio::test]
async fn guild_chat_from_the_guildless_goes_nowhere() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0 + 1.0, START.1, START.2).await;
    handled(&mut b).await;
    for kind in [
        CMSG_MESSAGECHAT_ChatType::Guild,
        CMSG_MESSAGECHAT_ChatType::Officer,
        CMSG_MESSAGECHAT_ChatType::Party,
        CMSG_MESSAGECHAT_ChatType::Raid,
    ] {
        speak(&a, kind, Language::Common, "not global").await;
    }
    handled(&mut a).await;
    assert_eq!(lines(&mut b).await, []);
    assert_eq!(lines(&mut a).await, []);
}

/// Thistle Tea's `Chat.handle` "keeps addon payloads intact without
/// interpreting commands": an addon's party line reaches the party in the
/// addon language as it was sent, and a GM's starting with a dot is no
/// command.
#[tokio::test]
async fn an_addons_line_is_its_payload_and_never_a_command() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    group(&mut a, &mut b, "Bob").await;
    handled(&mut a).await;
    a.clear();
    b.clear();
    speak(
        &a,
        CMSG_MESSAGECHAT_ChatType::Party,
        Language::Addon,
        ".die",
    )
    .await;
    let line = ("Party".to_owned(), Language::Addon, ".die".to_owned());
    assert_eq!(next_line(&mut a).await, line);
    assert_eq!(next_line(&mut b).await, line);
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let alive = map
        .query(move |m| m.players[&ga].death == crate::game::map::death::Death::Alive)
        .await
        .unwrap();
    assert!(alive, "the GM's addon line didn't kill her");
    // A GM's own line is a command still.
    speak(&a, CMSG_MESSAGECHAT_ChatType::Say, Language::Common, ".die").await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while map
        .query(move |m| m.players[&ga].death == crate::game::map::death::Death::Alive)
        .await
        .unwrap()
    {
        assert!(tokio::time::Instant::now() < deadline, ".die killed no one");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
