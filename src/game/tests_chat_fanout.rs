//! Chat fanned out to many (`channels::say`, `guilds::chat`,
//! `groups::chat`): the line goes out after the service's lock is let go,
//! with those ignoring the speaker asked once (`Lists::ignorers`). Through
//! the protocol.

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::harness::{self, Client};
use crate::game::social::channels;
use crate::game::tests_p3_entity::NONE_WITHIN;
use crate::game::tests_p3_player2_ammo::handled;

async fn speak(c: &Client, chat_type: CMSG_MESSAGECHAT_ChatType, text: &str) {
    c.send(CMSG_MESSAGECHAT {
        chat_type,
        language: Language::Common,
        message: text.into(),
    })
    .await;
}

/// The text of a player's chat line.
fn said(m: &ServerOpcodeMessage) -> Option<String> {
    match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(x)
            if !matches!(x.chat_type, SMSG_MESSAGECHAT_ChatType::System { .. }) =>
        {
            Some(x.message.clone())
        }
        _ => None,
    }
}

/// The lines `c` hears within [`NONE_WITHIN`].
async fn heard(c: &mut Client) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(l) = c.gets(NONE_WITHIN, said).await {
        out.push(l);
    }
    out
}

/// `who` ignores `name`, and is told so.
async fn ignore(who: &mut Client, name: &str) {
    who.send(CMSG_ADD_IGNORE { name: name.into() }).await;
    handled(who).await;
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

/// `leader`'s guild takes `member`, invited by `name`.
async fn guild(leader: &mut Client, member: &mut Client, name: &str) {
    leader
        .send(CMSG_GUILD_INVITE {
            invited_player: name.into(),
        })
        .await;
    member
        .wait_for("SMSG_GUILD_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GUILD_INVITE(_)).then_some(())
        })
        .await;
    member.send(CMSG_GUILD_ACCEPT {}).await;
    leader
        .wait_for("the guild's joined event", |m| match m {
            ServerOpcodeMessage::SMSG_GUILD_EVENT(e) if e.event == GuildEvent::Joined => Some(()),
            _ => None,
        })
        .await;
}

/// Party, guild and channel lines skip those ignoring their speaker, and
/// reach the rest; a channel moderator's reach even those ignoring them
/// (mangos's `Channel::Say`).
#[tokio::test]
async fn lines_to_many_skip_those_ignoring_the_speaker_and_reach_the_rest() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    c.enter("Carol", Race::Human, Class::Warrior).await;
    group(&mut a, &mut b, "Bob").await;
    group(&mut a, &mut c, "Carol").await;
    a.send(CMSG_GUILD_CREATE {
        guild_name: "Peons".into(),
    })
    .await;
    guild(&mut a, &mut b, "Bob").await;
    guild(&mut a, &mut c, "Carol").await;
    // Alice, first in, owns and moderates the channel.
    for x in [&mut a, &mut b, &mut c] {
        x.send(CMSG_JOIN_CHANNEL {
            channel_name: "Tea".into(),
            channel_password: String::new(),
        })
        .await;
        x.wait_frame("YOU_JOINED", channels::SMSG_CHANNEL_NOTIFY, |f| {
            f.first() == Some(&channels::YOU_JOINED)
        })
        .await;
    }
    ignore(&mut b, "Alice").await;
    ignore(&mut a, "Carol").await;
    for x in [&mut a, &mut b, &mut c] {
        handled(x).await;
        x.clear();
    }

    speak(&a, CMSG_MESSAGECHAT_ChatType::Party, "party").await;
    speak(&a, CMSG_MESSAGECHAT_ChatType::Guild, "guild").await;
    speak(
        &a,
        CMSG_MESSAGECHAT_ChatType::Channel {
            channel: "Tea".into(),
        },
        "moderator",
    )
    .await;
    handled(&mut a).await;
    speak(
        &c,
        CMSG_MESSAGECHAT_ChatType::Channel {
            channel: "Tea".into(),
        },
        "member",
    )
    .await;
    handled(&mut c).await;
    assert_eq!(
        heard(&mut c).await,
        ["party", "guild", "moderator", "member"]
    );
    assert_eq!(heard(&mut b).await, ["moderator", "member"]);
    assert_eq!(heard(&mut a).await, ["party", "guild", "moderator"]);
}
