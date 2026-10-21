//! Round 12's chat, as mangos-classic runs it: addon lines and AFK or DND
//! toggles aren't counted as a flood; a whisper to the other side is told
//! "wrong faction"; guild chat goes out in Universal; anyone speaks in
//! LocalDefense, and WorldDefense from the eleventh rank; GuildRecruitment
//! is for the unguilded; a guild invitation from one ignored isn't sent.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::harness::{self, Client};

/// A realm whose players are no GMs.
fn players_realm() -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, Default::default(), None)
}

async fn player(realm: &Arc<Realm>, name: &str, race: Race) -> Client {
    let mut c = Client::connect(realm, name);
    c.enter(name, race, Class::Warrior).await;
    c.clear();
    c
}

/// Wait until `c`'s earlier packets have been handled by its map.
async fn handled(c: &mut Client) {
    c.send(CMSG_NAME_QUERY { guid: c.guid }).await;
    c.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
}

fn line(
    chat_type: CMSG_MESSAGECHAT_ChatType,
    language: Language,
    message: &str,
) -> CMSG_MESSAGECHAT {
    CMSG_MESSAGECHAT {
        chat_type,
        language,
        message: message.into(),
    }
}

/// The text of a chat line `c` hears whose type `kind` takes, within a
/// moment, with its language.
async fn hears(
    c: &mut Client,
    kind: impl Fn(&SMSG_MESSAGECHAT_ChatType) -> bool,
) -> Option<(String, Language)> {
    handled(c).await;
    c.gets(Duration::from_millis(300), |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(l) if kind(&l.chat_type) => {
            Some((l.message.clone(), l.language))
        }
        _ => None,
    })
    .await
}

async fn muted(c: &mut Client) -> bool {
    handled(c).await;
    c.gets(Duration::from_millis(100), |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(l)
            if matches!(l.chat_type, SMSG_MESSAGECHAT_ChatType::System { .. })
                && l.message.contains("muted") =>
        {
            Some(())
        }
        _ => None,
    })
    .await
    .is_some()
}

/// mangos's `HandleMessagechatOpcode` runs the flood counter only for a
/// line that isn't an addon's, and never for `/afk` or `/dnd`: twenty of
/// either, in a second, leave a player free to speak. Twelve said lines
/// still mute.
#[tokio::test]
async fn addon_lines_and_afk_toggles_are_no_flood() {
    let realm = players_realm();
    let mut a = player(&realm, "Alice", Race::Human).await;
    for i in 0..20 {
        a.send(line(
            CMSG_MESSAGECHAT_ChatType::Party,
            Language::Addon,
            &format!("CTRA\tsync {i}"),
        ))
        .await;
        a.send(line(
            CMSG_MESSAGECHAT_ChatType::Afk,
            Language::Universal,
            "",
        ))
        .await;
    }
    a.say("still free").await;
    let said = hears(&mut a, |k| {
        matches!(k, SMSG_MESSAGECHAT_ChatType::Say { .. })
    })
    .await;
    assert_eq!(said.map(|s| s.0).as_deref(), Some("still free"));
    assert!(!muted(&mut a).await, "no mute for addon lines and toggles");
    for i in 0..12 {
        a.say(&format!("spam {i}")).await;
    }
    a.system_message("muted").await;
}

/// A whisper to one on the other side, online, is told
/// `SMSG_CHAT_WRONG_FACTION` (mangos's `SendWrongFactionNotice`); to one
/// not there, "not found".
#[tokio::test]
async fn a_whisper_across_the_sides_is_the_wrong_faction() {
    let realm = players_realm();
    let mut a = player(&realm, "Alice", Race::Human).await;
    let _o = player(&realm, "Grunt", Race::Orc).await;
    a.send(line(
        CMSG_MESSAGECHAT_ChatType::Whisper {
            target_player: "Grunt".into(),
        },
        Language::Common,
        "hello",
    ))
    .await;
    a.wait_for("SMSG_CHAT_WRONG_FACTION", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_CHAT_WRONG_FACTION).then_some(())
    })
    .await;
    a.send(line(
        CMSG_MESSAGECHAT_ChatType::Whisper {
            target_player: "Nobody".into(),
        },
        Language::Common,
        "hello",
    ))
    .await;
    a.wait_for("SMSG_CHAT_PLAYER_NOT_FOUND", |m| match m {
        ServerOpcodeMessage::SMSG_CHAT_PLAYER_NOT_FOUND(n) => (n.name == "Nobody").then_some(()),
        _ => None,
    })
    .await;
}

async fn join(c: &mut Client, channel: &str) {
    c.send(CMSG_JOIN_CHANNEL {
        channel_name: channel.into(),
        channel_password: String::new(),
    })
    .await;
    handled(c).await;
}

/// The `SMSG_CHANNEL_NOTIFY` kinds `c` was sent since it last looked.
async fn notices(c: &mut Client) -> Vec<u8> {
    handled(c).await;
    c.take_frames(crate::game::social::channels::SMSG_CHANNEL_NOTIFY)
        .iter()
        .map(|b| b[0])
        .collect()
}

/// mangos's `Channel::Say`: LocalDefense mutes no rank
/// (`SPEAK_IN_LOCALDEFENSE_RANK` 0), WorldDefense all below the eleventh
/// (`SPEAK_IN_WORLDDEFENSE_RANK`); before, both were for GMs alone.
#[tokio::test]
async fn anyone_speaks_in_local_defense_and_the_ranked_in_world_defense() {
    use crate::game::social::channels::MUTED;
    let realm = players_realm();
    let mut a = player(&realm, "Alice", Race::Human).await;
    join(&mut a, "LocalDefense - Elwynn Forest").await;
    join(&mut a, "WorldDefense").await;
    a.clear();
    let channel = |name: &'static str| move |k: &SMSG_MESSAGECHAT_ChatType| matches!(k, SMSG_MESSAGECHAT_ChatType::Channel { channel_name, .. } if channel_name == name);
    a.send(line(
        CMSG_MESSAGECHAT_ChatType::Channel {
            channel: "LocalDefense - Elwynn Forest".into(),
        },
        Language::Common,
        "orcs at the bridge",
    ))
    .await;
    let heard = hears(&mut a, channel("LocalDefense - Elwynn Forest")).await;
    assert_eq!(heard.map(|h| h.0).as_deref(), Some("orcs at the bridge"));
    a.clear();
    a.send(line(
        CMSG_MESSAGECHAT_ChatType::Channel {
            channel: "WorldDefense".into(),
        },
        Language::Common,
        "unranked",
    ))
    .await;
    assert_eq!(notices(&mut a).await, vec![MUTED], "rank 0 is muted");
    // Lieutenant Commander: rank 15, the client's 11.
    {
        let mut honor = realm.social.honor.lock();
        let me = crate::game::guid::counter(a.guid);
        honor.register(me, 0, 60);
        honor.set_rank_points(me, 45_000.0);
    }
    a.clear();
    a.send(line(
        CMSG_MESSAGECHAT_ChatType::Channel {
            channel: "WorldDefense".into(),
        },
        Language::Common,
        "ranked",
    ))
    .await;
    let heard = hears(&mut a, channel("WorldDefense")).await;
    assert_eq!(heard.map(|h| h.0).as_deref(), Some("ranked"));
}

async fn create_guild(c: &mut Client, name: &str) {
    c.send(CMSG_GUILD_CREATE {
        guild_name: name.into(),
    })
    .await;
    handled(c).await;
}

async fn guild_invite(c: &mut Client, name: &str) {
    c.send(CMSG_GUILD_INVITE {
        invited_player: name.into(),
    })
    .await;
    handled(c).await;
}

/// Guild and officer chat go out in Universal whatever the speaker chose
/// (mangos's `BroadcastToGuild(.., LANG_UNIVERSAL)`): a Dwarf's Dwarvish
/// reaches a Human guildmate readable. An addon's line keeps its marker.
#[tokio::test]
async fn guild_chat_is_universal() {
    let realm = harness::realm();
    let mut d = player(&realm, "Durin", Race::Dwarf).await;
    let mut h = player(&realm, "Hal", Race::Human).await;
    create_guild(&mut d, "Ironforge Kin").await;
    guild_invite(&mut d, "Hal").await;
    h.send(CMSG_GUILD_ACCEPT {}).await;
    handled(&mut h).await;
    h.clear();
    d.send(line(
        CMSG_MESSAGECHAT_ChatType::Guild,
        Language::Dwarvish,
        "well met",
    ))
    .await;
    let heard = hears(&mut h, |k| {
        matches!(k, SMSG_MESSAGECHAT_ChatType::Guild { .. })
    })
    .await;
    assert_eq!(
        heard,
        Some(("well met".to_owned(), Language::Universal)),
        "guild chat is in Universal"
    );
    d.send(line(
        CMSG_MESSAGECHAT_ChatType::Guild,
        Language::Addon,
        "GUILDSYNC\t1",
    ))
    .await;
    let heard = hears(&mut h, |k| {
        matches!(k, SMSG_MESSAGECHAT_ChatType::Guild { .. })
    })
    .await;
    assert_eq!(heard.map(|h| h.1), Some(Language::Addon));
}

/// mangos's `HandleGuildInviteOpcode`: one ignoring the inviter isn't
/// asked, and the inviter is told nothing.
#[tokio::test]
async fn a_guild_invite_from_one_ignored_is_not_sent() {
    let realm = harness::realm();
    let mut a = player(&realm, "Alice", Race::Human).await;
    let mut b = player(&realm, "Bob", Race::Human).await;
    create_guild(&mut a, "Pests").await;
    b.send(CMSG_ADD_IGNORE {
        name: "Alice".into(),
    })
    .await;
    handled(&mut b).await;
    b.clear();
    a.clear();
    guild_invite(&mut a, "Bob").await;
    let asked = b
        .gets(Duration::from_millis(200), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GUILD_INVITE(_)).then_some(())
        })
        .await;
    assert!(
        asked.is_none(),
        "an ignored inviter's invitation isn't sent"
    );
    let told = a
        .gets(Duration::from_millis(50), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GUILD_COMMAND_RESULT(_)).then_some(())
        })
        .await;
    assert!(told.is_none(), "and the inviter isn't told");
}

/// mangos's `Channel::Join`: a guild's member isn't let into
/// GuildRecruitment (its flags 0x38), and isn't told; one with no guild
/// is.
#[tokio::test]
async fn guild_recruitment_is_for_the_unguilded() {
    use crate::game::social::channels::YOU_JOINED;
    let realm = harness::realm();
    let mut a = player(&realm, "Alice", Race::Human).await;
    let mut b = player(&realm, "Bob", Race::Human).await;
    create_guild(&mut a, "Recruiters").await;
    a.clear();
    b.clear();
    join(&mut a, "GuildRecruitment - City").await;
    join(&mut b, "GuildRecruitment - City").await;
    assert_eq!(notices(&mut a).await, Vec::<u8>::new());
    assert_eq!(notices(&mut b).await, vec![YOU_JOINED]);
    let dir = realm.social.channels.lock();
    assert!(dir.of(a.guid).is_empty());
    assert_eq!(dir.of(b.guid), vec!["GuildRecruitment - City".to_owned()]);
}

/// mangos's `Player::ToggleAFK`: "afk player not allowed in battleground".
/// Going AFK in a match takes the player out of it (a deserter, the match
/// not over); in GM mode, they stay.
#[tokio::test]
async fn going_afk_in_a_battleground_leaves_it() {
    use crate::game::tests_p3_world_bg::{all_in, gulch, player as bg_player};
    let realm = harness::realm_with(gulch());
    let mut a = bg_player(&realm, "A", "Alice", Race::Human).await;
    let mut b = bg_player(&realm, "B", "Grom", Race::Orc).await;
    let (ga, gb) = (a.guid, b.guid);
    let map = all_in(&realm, &mut [&mut a, &mut b]).await;
    let in_match = |g| {
        realm
            .social
            .battlegrounds
            .lock()
            .matches
            .values()
            .any(|m| m.members.contains_key(&g))
    };
    // Grom in GM mode stays.
    b.say(".gm on").await;
    b.system_message("GM mode on").await;
    b.send(line(
        CMSG_MESSAGECHAT_ChatType::Afk,
        Language::Universal,
        "",
    ))
    .await;
    handled(&mut b).await;
    assert!(in_match(gb), "a GM in GM mode stays");
    a.send(line(
        CMSG_MESSAGECHAT_ChatType::Afk,
        Language::Universal,
        "brb",
    ))
    .await;
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    assert!(!in_match(ga), "Alice, away, is out of the match");
    let _ = map;
}
