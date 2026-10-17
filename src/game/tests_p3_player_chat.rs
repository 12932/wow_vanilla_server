//! Phase 3 against Thistle Tea's chat tests (`test/game/core/chat`): AFK
//! and DND and the party's view of them, channels' owners and members,
//! languages under a curse, and what an emote ends. Through the protocol.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::player::{PLAYER_FLAGS_AFK, PLAYER_FLAGS_DND, UNIT_FLAG_IN_COMBAT};
use crate::game::map::{Map, MapKey};
use crate::game::registry::Team;
use crate::game::social::channels::{self, SMSG_CHANNEL_NOTIFY};

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

/// A realm whose players aren't GMs, to be refused what a GM may do.
fn players_realm(data: GameData) -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, data, None)
}

/// Wait until the map has handled what `c` sent so far.
async fn synced(c: &mut Client) {
    c.send(CMSG_PLAYED_TIME {}).await;
    c.wait_for("SMSG_PLAYED_TIME", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_PLAYED_TIME(_)).then_some(())
    })
    .await;
}

fn chat(chat_type: CMSG_MESSAGECHAT_ChatType, message: &str) -> CMSG_MESSAGECHAT {
    CMSG_MESSAGECHAT {
        chat_type,
        language: Language::Common,
        message: message.into(),
    }
}

/// What `from` hears whispering `to`: the auto-reply, if `to` is away.
async fn reply(from: &mut Client, to: &str) -> Option<(bool, String)> {
    from.clear();
    from.send(chat(
        CMSG_MESSAGECHAT_ChatType::Whisper {
            target_player: to.into(),
        },
        "hello",
    ))
    .await;
    from.wait_for("the whisper's echo", |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c)
            if matches!(c.chat_type, SMSG_MESSAGECHAT_ChatType::WhisperInform { .. }) =>
        {
            Some(())
        }
        _ => None,
    })
    .await;
    from.gets(Duration::from_millis(200), |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => match c.chat_type {
            SMSG_MESSAGECHAT_ChatType::Afk { .. } => Some((true, c.message.clone())),
            SMSG_MESSAGECHAT_ChatType::Dnd { .. } => Some((false, c.message.clone())),
            _ => None,
        },
        _ => None,
    })
    .await
}

fn away_flags(m: &Map, g: Guid) -> u32 {
    m.players[&g].fields.get_u32(PLAYER_FLAGS) & (PLAYER_FLAGS_AFK | PLAYER_FLAGS_DND)
}

#[tokio::test]
async fn afk_and_dnd_change_their_reply_go_off_when_empty_exclude_each_other_and_afk_waits_out_a_fight()
 {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    let set = async |a: &mut Client, kind, text: &str| {
        a.send(chat(kind, text)).await;
        synced(a).await;
        let realm = realm.clone();
        on_map(&realm, move |m| away_flags(m, ga)).await
    };
    use CMSG_MESSAGECHAT_ChatType::{Afk, Dnd};
    assert_eq!(set(&mut a, Afk, "Tea break").await, PLAYER_FLAGS_AFK);
    assert_eq!(
        reply(&mut b, "Alice").await,
        Some((true, "Tea break".into()))
    );
    // A new message while away changes the reply, and stays away.
    assert_eq!(set(&mut a, Afk, "Still away").await, PLAYER_FLAGS_AFK);
    assert_eq!(
        reply(&mut b, "Alice").await,
        Some((true, "Still away".into()))
    );
    // None turns it off; none again, on.
    assert_eq!(set(&mut a, Afk, "").await, 0);
    assert_eq!(reply(&mut b, "Alice").await, None);
    assert_eq!(set(&mut a, Afk, "").await, PLAYER_FLAGS_AFK);
    // DND puts AFK off, and AFK DND.
    assert_eq!(set(&mut a, Dnd, "Busy").await, PLAYER_FLAGS_DND);
    assert_eq!(reply(&mut b, "Alice").await, Some((false, "Busy".into())));
    assert_eq!(set(&mut a, Afk, "Back soon").await, PLAYER_FLAGS_AFK);
    assert_eq!(set(&mut a, Afk, "").await, 0);
    // In a fight, no AFK; DND will do.
    on_map(&realm, move |m| {
        m.players.get_mut(&ga).unwrap().fields.set_flag(
            UNIT_FIELD_FLAGS,
            UNIT_FLAG_IN_COMBAT,
            true,
        );
    })
    .await;
    assert_eq!(set(&mut a, Afk, "Away").await, 0);
    assert_eq!(set(&mut a, Dnd, "Busy").await, PLAYER_FLAGS_DND);
    // Out of the fight, and a new login is neither.
    on_map(&realm, move |m| {
        m.players.get_mut(&ga).unwrap().fields.set_flag(
            UNIT_FIELD_FLAGS,
            UNIT_FLAG_IN_COMBAT,
            false,
        );
    })
    .await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.login(ga).await;
    assert_eq!(on_map(&realm, move |m| away_flags(m, ga)).await, 0);
    assert_eq!(reply(&mut b, "Alice").await, None);
}

/// The status byte of a member's full stats, as `asker` requests them.
async fn member_status(realm: &Arc<Realm>, asker: &mut Client, of: Guid) -> u8 {
    use crate::game::social::groups::SMSG_PARTY_MEMBER_STATS_FULL;
    asker.clear();
    on_map(realm, |m| {
        m.member_stats_at = m.now;
        crate::game::map::social::push_member_stats(m);
    })
    .await;
    asker
        .send(CMSG_REQUEST_PARTY_MEMBER_STATS { guid: of })
        .await;
    let full = asker
        .wait_frame("the full stats", SMSG_PARTY_MEMBER_STATS_FULL, |_| true)
        .await;
    // A packed GUID, the mask, then the status.
    let at = 1 + full[0].count_ones() as usize + 4;
    full[at]
}

#[tokio::test]
async fn the_party_sees_a_member_away_busy_and_dead() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
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
    assert_eq!(member_status(&realm, &mut b, ga).await, 0x01);
    a.send(chat(CMSG_MESSAGECHAT_ChatType::Afk, "")).await;
    synced(&mut a).await;
    assert_eq!(member_status(&realm, &mut b, ga).await, 0x41);
    a.send(chat(CMSG_MESSAGECHAT_ChatType::Dnd, "")).await;
    synced(&mut a).await;
    assert_eq!(member_status(&realm, &mut b, ga).await, 0x81);
    on_map(&realm, move |m| crate::game::map::death::kill(m, ga)).await;
    assert_eq!(member_status(&realm, &mut b, ga).await, 0x85);
}

/// The bytes after a channel notice's kind and channel name.
fn notice<'a>(b: &'a [u8], kind: u8, channel: &str) -> Option<&'a [u8]> {
    let name = channel.as_bytes();
    (b.first() == Some(&kind)
        && b.get(1..1 + name.len()) == Some(name)
        && b.get(1 + name.len()) == Some(&0))
    .then(|| &b[2 + name.len()..])
}

async fn notified(c: &mut Client, kind: u8, channel: &str) -> Vec<u8> {
    let f = c
        .wait_frame(&format!("notice 0x{kind:02X}"), SMSG_CHANNEL_NOTIFY, |f| {
            notice(f, kind, channel).is_some()
        })
        .await;
    notice(&f, kind, channel).unwrap().to_vec()
}

fn join(name: &str) -> CMSG_JOIN_CHANNEL {
    CMSG_JOIN_CHANNEL {
        channel_name: name.into(),
        channel_password: String::new(),
    }
}

fn owner_of(realm: &Realm, team: Team, name: &str) -> Option<Guid> {
    realm
        .social
        .channels
        .lock()
        .get(team, name)
        .map(|c| c.owner)
}

#[tokio::test]
async fn a_channels_owner_hands_it_on_to_the_lowest_guid_and_only_the_owner_may_give_it_or_touch_the_owner()
 {
    let realm = players_realm(GameData::empty());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Carol", Race::Human, Class::Warrior).await;
    assert!(ga.guid() < gb.guid() && gb.guid() < gc.guid());
    // Carol first, then Bob, then Alice: Carol owns it.
    for (cl, _) in [(&mut c, gc), (&mut b, gb), (&mut a, ga)] {
        cl.send(join("Tea")).await;
        notified(cl, channels::YOU_JOINED, "Tea").await;
    }
    assert_eq!(owner_of(&realm, Team::Alliance, "Tea"), Some(gc));
    // Bob, not the owner, can't hand it on, nor touch her.
    b.send(CMSG_CHANNEL_SET_OWNER {
        channel_name: "Tea".into(),
        new_owner: "Bob".into(),
    })
    .await;
    notified(&mut b, channels::NOT_OWNER, "Tea").await;
    c.send(CMSG_CHANNEL_MODERATOR {
        channel_name: "Tea".into(),
        player_name: "Bob".into(),
    })
    .await;
    notified(&mut b, channels::MODE_CHANGE, "Tea").await;
    b.send(CMSG_CHANNEL_MUTE {
        channel_name: "Tea".into(),
        player_name: "Carol".into(),
    })
    .await;
    notified(&mut b, channels::NOT_OWNER, "Tea").await;
    assert_eq!(owner_of(&realm, Team::Alliance, "Tea"), Some(gc));
    // Carol leaves: the lowest GUID, Alice, not Bob who came first.
    c.send(CMSG_LEAVE_CHANNEL {
        channel_name: "Tea".into(),
    })
    .await;
    notified(&mut a, channels::OWNER_CHANGED, "Tea").await;
    assert_eq!(owner_of(&realm, Team::Alliance, "Tea"), Some(ga));
    // Carol, out, isn't heard in it.
    c.send(chat(
        CMSG_MESSAGECHAT_ChatType::Channel {
            channel: "Tea".into(),
        },
        "hello",
    ))
    .await;
    notified(&mut c, channels::NOT_MEMBER, "Tea").await;
}

#[tokio::test]
async fn channel_names_are_one_whatever_the_case_each_side_its_own_and_going_offline_leaves_them_all()
 {
    let realm = players_realm(GameData::empty());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut h = Client::connect(&realm, "H");
    let gh = h.enter("Grom", Race::Orc, Class::Warrior).await;
    a.send(join("Tea")).await;
    notified(&mut a, channels::YOU_JOINED, "Tea").await;
    h.send(join("tea")).await;
    notified(&mut h, channels::YOU_JOINED, "tea").await;
    // One name, two sides: each its own owner.
    let ga = a.guid;
    assert_eq!(owner_of(&realm, Team::Alliance, "TEA"), Some(ga));
    assert_eq!(owner_of(&realm, Team::Horde, "Tea"), Some(gh));
    a.send(join("Two")).await;
    notified(&mut a, channels::YOU_JOINED, "Two").await;
    // Alice goes: her channels, empty, go with her.
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    for _ in 0..100 {
        if owner_of(&realm, Team::Alliance, "Tea").is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(owner_of(&realm, Team::Alliance, "Tea"), None);
    assert_eq!(owner_of(&realm, Team::Alliance, "Two"), None);
    assert_eq!(owner_of(&realm, Team::Horde, "Tea"), Some(gh));
    assert!(realm.social.channels.lock().of(ga).is_empty());
}

const CURSE: u32 = 962_001;
const DRACONIC: u32 = 962_002;

fn aura(id: u32, kind: u32, misc: i32, interrupt: u32) -> SpellTemplate {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura: kind,
        misc_value: misc,
        base_points: 0,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    SpellTemplate {
        id,
        name: format!("Aura {id}"),
        effects,
        aura_interrupt_flags: interrupt,
        equipped_item_class: -1,
        duration_index: 21,
        ..Default::default()
    }
}

fn spells() -> GameData {
    use crate::game::map::auras::AURA_MOD_LANGUAGE;
    let mut d = GameData::empty();
    d.spells
        .spells
        .insert(CURSE, aura(CURSE, AURA_MOD_LANGUAGE, 8, 0));
    d.spells
        .spells
        .insert(DRACONIC, aura(DRACONIC, AURA_MOD_LANGUAGE, 11, 0));
    d
}

fn curse(m: &mut Map, g: Guid, spell: u32) {
    let s = m.realm.data.spells.get(spell).unwrap().clone();
    crate::game::map::auras::apply_spell(m, g, g, &s, &[(0, 1)], Default::default()).unwrap();
}

/// The language Bob hears Alice say a line in.
async fn heard(a: &Client, b: &mut Client) -> Language {
    b.clear();
    a.send(chat(CMSG_MESSAGECHAT_ChatType::Say, "hello")).await;
    b.wait_for("Alice's line", |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c)
            if matches!(c.chat_type, SMSG_MESSAGECHAT_ChatType::Say { .. }) =>
        {
            Some(c.language)
        }
        _ => None,
    })
    .await
}

#[tokio::test]
async fn a_curse_of_tongues_speaks_for_the_cursed_until_it_goes_the_next_taking_over_and_none_after_death()
 {
    let realm = harness::realm_with(spells());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(ga).await;
    on_map(&realm, move |m| {
        curse(m, ga, CURSE);
        curse(m, ga, DRACONIC);
    })
    .await;
    assert_eq!(heard(&a, &mut b).await.as_int(), 8);
    // Whispers and away replies go in no language; an addon's line only
    // where addons speak (a party, not a say).
    b.clear();
    a.send(chat(
        CMSG_MESSAGECHAT_ChatType::Whisper {
            target_player: "Bob".into(),
        },
        "psst",
    ))
    .await;
    let whispered = b
        .wait_for("the whisper", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c)
                if matches!(c.chat_type, SMSG_MESSAGECHAT_ChatType::Whisper { .. }) =>
            {
                Some(c.language)
            }
            _ => None,
        })
        .await;
    assert_eq!(whispered, Language::Universal);
    a.send(chat(CMSG_MESSAGECHAT_ChatType::Afk, "Away")).await;
    let reply = reply(&mut b, "Alice").await;
    assert!(reply.is_some());
    a.send(chat(CMSG_MESSAGECHAT_ChatType::Afk, "")).await;
    b.clear();
    a.send(CMSG_MESSAGECHAT {
        chat_type: CMSG_MESSAGECHAT_ChatType::Say,
        language: Language::Addon,
        message: "ADDON\tdata".into(),
    })
    .await;
    synced(&mut a).await;
    assert!(
        b.gets(Duration::from_millis(200), |m| matches!(
            m,
            ServerOpcodeMessage::SMSG_MESSAGECHAT(_)
        )
        .then_some(()))
            .await
            .is_none(),
        "an addon's say is dropped"
    );
    on_map(&realm, move |m| {
        crate::game::map::auras::remove_spell(m, ga, CURSE, None)
    })
    .await;
    assert_eq!(heard(&a, &mut b).await.as_int(), 11);
    on_map(&realm, move |m| {
        crate::game::map::auras::remove_spell(m, ga, DRACONIC, None)
    })
    .await;
    assert_eq!(heard(&a, &mut b).await, Language::Common);
    // Death takes the curse with the rest.
    on_map(&realm, move |m| {
        curse(m, ga, CURSE);
        crate::game::map::death::kill(m, ga);
        crate::game::map::death::resurrect(m, ga, 1.0);
    })
    .await;
    assert_eq!(heard(&a, &mut b).await, Language::Common);
}

#[tokio::test]
async fn death_stands_a_dancer_up_and_a_new_login_comes_in_standing() {
    let realm = harness::realm_with(spells());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    // Dancing and kneeling, then dead: neither.
    let dead = on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.fields.set_u32(UNIT_NPC_EMOTESTATE, 10);
        p.set_stand_state(8);
        crate::game::map::death::kill(m, ga);
        let p = &m.players[&ga];
        (p.fields.get_u32(UNIT_NPC_EMOTESTATE), p.stand_state())
    })
    .await;
    assert_eq!(dead, (0, 0));
    // Back alive and dancing, out and in again: standing still.
    on_map(&realm, move |m| {
        crate::game::map::death::resurrect(m, ga, 1.0);
        let p = m.players.get_mut(&ga).unwrap();
        p.fields.set_u32(UNIT_NPC_EMOTESTATE, 10);
        p.set_stand_state(8);
    })
    .await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.login(ga).await;
    let back = on_map(&realm, move |m| {
        let p = &m.players[&ga];
        (p.fields.get_u32(UNIT_NPC_EMOTESTATE), p.stand_state())
    })
    .await;
    assert_eq!(back, (0, 0));
}
