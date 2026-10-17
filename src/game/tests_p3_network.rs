//! Thistle Tea's inbound and message tests (Phase 3, `game/inbound`,
//! `game/network`), through the protocol: the action bar, packets sent
//! from the character screen, channel commands, played time, attacks
//! stopped, and what the client is told.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey};

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

async fn relog(c: &mut Client) {
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    c.clear();
    let g = c.guid;
    c.login(g).await;
}

/// Wait until the map has handled everything sent before: a time query
/// goes the same way, and its answer comes after.
async fn settled(c: &mut Client) {
    c.send(CMSG_QUERY_TIME {}).await;
    c.wait_for("SMSG_QUERY_TIME_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_QUERY_TIME_RESPONSE(_)).then_some(())
    })
    .await;
}

fn button(button: u8, packed: u32) -> CMSG_SET_ACTION_BUTTON {
    CMSG_SET_ACTION_BUTTON {
        button,
        action: (packed & 0xFFFF) as u16,
        misc: ((packed >> 16) & 0xFF) as u8,
        action_type: (packed >> 24) as u8,
    }
}

/// Thistle Tea's action_buttons_test: a button is set and cleared, a spell
/// not known, a button past the last and a kind that isn't one are
/// refused, and what is kept comes back at login packed at its index.
#[tokio::test]
async fn action_buttons_are_checked_kept_and_sent_at_login() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let known = on_map(&realm, move |m| m.players[&g].spells.clone()).await;
    assert!(known.contains(&6603) && known.contains(&78));
    assert!(!known.contains(&133));
    // Attack on 3, Tough Jerky (117) on 83, a macro on 10.
    a.send(button(3, 6603)).await;
    a.send(button(83, 117 | 0x80 << 24)).await;
    a.send(button(10, 1 | 0x40 << 24)).await;
    // Heroic Strike on 4, then cleared.
    a.send(button(4, 78)).await;
    a.send(button(4, 0)).await;
    // Fireball, not known; a button past the last; a kind that isn't one;
    // an item that doesn't exist on a button already set.
    a.send(button(5, 133)).await;
    a.send(button(120, 6603)).await;
    a.send(button(6, 6603 | 0x10 << 24)).await;
    a.send(button(83, 0xFFFF | 0x80 << 24)).await;
    settled(&mut a).await;
    relog(&mut a).await;
    let bar = a
        .wait_for("SMSG_ACTION_BUTTONS", |m| match m {
            ServerOpcodeMessage::SMSG_ACTION_BUTTONS(b) => Some(b.data),
            _ => None,
        })
        .await;
    assert_eq!(bar[3], 6603, "a known spell's button");
    assert_eq!(bar[83], 117 | 0x80 << 24, "an item's, packed with its kind");
    assert_eq!(bar[10], 1 | 0x40 << 24, "a macro's");
    assert_eq!(bar[4], 0, "a button cleared");
    assert_eq!(bar[5], 0, "a spell not known is refused");
    assert_eq!(bar[6], 0, "a kind that isn't one is refused");
    let set = bar.iter().filter(|b| **b != 0).count();
    let start = on_map(&realm, move |m| m.players[&g].row.actions.len()).await;
    assert_eq!(set, start, "nothing past the last button");
}

/// Thistle Tea's CmsgSetActionbarToggles: the toggles go into
/// `PLAYER_FIELD_BYTES`' third byte, are kept with the character, and
/// clear.
#[tokio::test]
async fn the_action_bar_toggles_are_set_kept_and_cleared() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let toggles = move |m: &mut Map| m.players[&g].fields.get_byte(PLAYER_FIELD_BYTES, 2);
    a.send(CMSG_SET_ACTIONBAR_TOGGLES { action_bar: 0x0F })
        .await;
    settled(&mut a).await;
    assert_eq!(on_map(&realm, toggles).await, 0x0F);
    relog(&mut a).await;
    assert_eq!(on_map(&realm, toggles).await, 0x0F, "kept over a relog");
    a.send(CMSG_SET_ACTIONBAR_TOGGLES { action_bar: 0 }).await;
    settled(&mut a).await;
    assert_eq!(on_map(&realm, toggles).await, 0);
}

/// Create a character and list it, without logging in.
async fn create(c: &mut Client, name: &str) -> Guid {
    c.send(CMSG_CHAR_CREATE {
        name: name.to_owned(),
        race: Race::Human,
        class: Class::Warrior,
        gender: Gender::Female,
        skin_color: 1,
        face: 1,
        hair_style: 1,
        hair_color: 1,
        facial_hair: 1,
    })
    .await;
    c.wait_for("SMSG_CHAR_CREATE", |m| match m {
        ServerOpcodeMessage::SMSG_CHAR_CREATE(_) => Some(()),
        _ => None,
    })
    .await;
    c.send(CMSG_CHAR_ENUM {}).await;
    let name = name.to_owned();
    c.wait_for("SMSG_CHAR_ENUM", |m| match m {
        ServerOpcodeMessage::SMSG_CHAR_ENUM(e) => {
            e.characters.iter().find(|c| c.name == name).map(|c| c.guid)
        }
        _ => None,
    })
    .await
}

/// Thistle Tea's "ignores requests before world entry" tests: an equip, a
/// purchase, a PvP toggle, a zone, a roll and the action bar sent from the
/// character screen do nothing, and the character then logs in as it was.
#[tokio::test]
async fn world_packets_from_the_character_screen_do_nothing() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = create(&mut a, "Alice").await;
    a.clear();
    a.send(CMSG_AUTOEQUIP_ITEM_SLOT {
        guid: Guid::new(0x4000_0000_0000_0001),
        destination_slot: 0,
    })
    .await;
    a.send(CMSG_BUY_ITEM_IN_SLOT {
        vendor: Guid::new(0xF130_0000_0000_0001),
        item: 117,
        bag: g,
        bag_slot: 23,
        amount: 1,
    })
    .await;
    a.send(CMSG_TOGGLE_PVP {
        set: Some(CMSG_TOGGLE_PVP_set { enable_pvp: true }),
    })
    .await;
    a.send(CMSG_ZONEUPDATE {
        area: Area::Durotar,
    })
    .await;
    a.send(MSG_RANDOM_ROLL_Client {
        minimum: 1,
        maximum: 100,
    })
    .await;
    a.send(CMSG_SET_ACTIONBAR_TOGGLES { action_bar: 0x0F })
        .await;
    assert!(
        a.gets(Duration::from_millis(300), |m| Some(m.to_string()))
            .await
            .is_none(),
        "no answer on the character screen"
    );
    a.login(g).await;
    let (toggles, pvp, zone, gold) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (
            p.fields.get_byte(PLAYER_FIELD_BYTES, 2),
            p.fields.get_u32(PLAYER_FLAGS) & crate::game::map::player::PLAYER_FLAGS_PVP_DESIRED,
            p.zone,
            p.fields.get_u32(PLAYER_FIELD_COINAGE),
        )
    })
    .await;
    assert_eq!(toggles, 0);
    assert_eq!(pvp, 0);
    assert_ne!(zone, Area::Durotar.as_int());
    assert_eq!(gold, 0);
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// The bytes after a channel notice's kind and channel name.
fn notice<'a>(b: &'a [u8], kind: u8, channel: &str) -> Option<&'a [u8]> {
    let name = channel.as_bytes();
    (b.first() == Some(&kind)
        && b.get(1..1 + name.len()) == Some(name)
        && b.get(1 + name.len()) == Some(&0))
    .then(|| &b[2 + name.len()..])
}

/// The rest of the next `kind` notice for `channel`.
async fn notified(c: &mut Client, kind: u8, channel: &str) -> Vec<u8> {
    use crate::game::social::channels::SMSG_CHANNEL_NOTIFY;
    let f = c
        .wait_frame(&format!("notice 0x{kind:02X}"), SMSG_CHANNEL_NOTIFY, |f| {
            notice(f, kind, channel).is_some()
        })
        .await;
    notice(&f, kind, channel).unwrap().to_vec()
}

/// A realm whose players aren't GMs, to be refused what a GM may do.
fn players_realm() -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, Default::default(), None)
}

/// Thistle Tea's smsg_channel_notify_test and dispatch_test's every channel
/// command: joining tells a channel's flags and index; its owner names
/// itself, makes and unmakes a moderator, mutes, turns announcements and
/// moderation, sets a password, bans and unbans, hands the channel on; an
/// invitation reaches its player. Each notice as mangos lays it out.
#[tokio::test]
async fn every_channel_command_does_what_it_says() {
    use crate::game::social::channels::*;
    let realm = players_realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    c.enter("Carol", Race::Human, Class::Warrior).await;
    let join = |name: &str, password: &str| CMSG_JOIN_CHANNEL {
        channel_name: name.into(),
        channel_password: password.into(),
    };
    // A zone's General: its flags (general, not LFG) and index 0.
    a.send(join("General - Elwynn Forest", "")).await;
    let joined = notified(&mut a, YOU_JOINED, "General - Elwynn Forest").await;
    assert_eq!((u32_at(&joined, 0), u32_at(&joined, 4)), (0x18, 0));
    // A custom channel: custom's flag and index 0.
    a.send(join("Custom", "")).await;
    let joined = notified(&mut a, YOU_JOINED, "Custom").await;
    assert_eq!((u32_at(&joined, 0), u32_at(&joined, 4)), (0x01, 0));
    b.send(join("Custom", "")).await;
    notified(&mut b, YOU_JOINED, "Custom").await;
    notified(&mut a, JOINED, "Custom").await;
    a.clear();
    b.clear();
    let custom = |channel_name: &str| channel_name.to_owned();
    // Who owns it.
    a.send(CMSG_CHANNEL_OWNER {
        channel_name: custom("Custom"),
    })
    .await;
    assert_eq!(notified(&mut a, CHANNEL_OWNER, "Custom").await, b"Alice\0");
    // Bob made a moderator and unmade: each a mode change to all.
    let both = |p: &str| (custom("Custom"), p.to_owned());
    let (ch, p) = both("Bob");
    a.send(CMSG_CHANNEL_MODERATOR {
        channel_name: ch,
        player_name: p,
    })
    .await;
    for cl in [&mut a, &mut b] {
        let m = notified(cl, MODE_CHANGE, "Custom").await;
        assert_eq!(
            (u64_at(&m, 0), m[8], m[9]),
            (gb.guid(), 0, MEMBER_MODERATOR)
        );
    }
    let (ch, p) = both("Bob");
    a.send(CMSG_CHANNEL_UNMODERATOR {
        channel_name: ch,
        player_name: p,
    })
    .await;
    let m = notified(&mut b, MODE_CHANGE, "Custom").await;
    assert_eq!((m[8], m[9]), (MEMBER_MODERATOR, 0));
    // Muted, Bob can't speak; unmuted, he can.
    let (ch, p) = both("Bob");
    a.send(CMSG_CHANNEL_MUTE {
        channel_name: ch,
        player_name: p,
    })
    .await;
    let m = notified(&mut b, MODE_CHANGE, "Custom").await;
    assert_eq!((m[8], m[9]), (0, MEMBER_MUTED));
    let speak = || CMSG_MESSAGECHAT {
        chat_type: CMSG_MESSAGECHAT_ChatType::Channel {
            channel: "Custom".into(),
        },
        language: Language::Common,
        message: "hello".into(),
    };
    b.send(speak()).await;
    notified(&mut b, MUTED, "Custom").await;
    let (ch, p) = both("Bob");
    a.send(CMSG_CHANNEL_UNMUTE {
        channel_name: ch,
        player_name: p,
    })
    .await;
    let m = notified(&mut b, MODE_CHANGE, "Custom").await;
    assert_eq!((m[8], m[9]), (MEMBER_MUTED, 0));
    // Announcements off (a custom channel's are on), moderation on: by
    // whom.
    a.send(CMSG_CHANNEL_ANNOUNCEMENTS {
        channel_name: custom("Custom"),
    })
    .await;
    assert_eq!(
        u64_at(&notified(&mut b, ANNOUNCEMENTS_OFF, "Custom").await, 0),
        ga.guid()
    );
    a.send(CMSG_CHANNEL_MODERATE {
        channel_name: custom("Custom"),
    })
    .await;
    assert_eq!(
        u64_at(&notified(&mut b, MODERATION_ON, "Custom").await, 0),
        ga.guid()
    );
    // Moderated, Bob isn't heard.
    b.send(speak()).await;
    notified(&mut b, NOT_MODERATOR, "Custom").await;
    // A password; Bob banned (who, by whom), and kept out.
    a.send(CMSG_CHANNEL_PASSWORD {
        channel_name: custom("Custom"),
        channel_password: "secret".into(),
    })
    .await;
    assert_eq!(
        u64_at(&notified(&mut b, PASSWORD_CHANGED, "Custom").await, 0),
        ga.guid()
    );
    let (ch, p) = both("Bob");
    a.send(CMSG_CHANNEL_BAN {
        channel_name: ch,
        player_name: p,
    })
    .await;
    let banned = notified(&mut b, PLAYER_BANNED, "Custom").await;
    assert_eq!(
        (u64_at(&banned, 0), u64_at(&banned, 8)),
        (gb.guid(), ga.guid())
    );
    b.send(join("Custom", "secret")).await;
    notified(&mut b, BANNED, "Custom").await;
    let (ch, p) = both("Bob");
    a.send(CMSG_CHANNEL_UNBAN {
        channel_name: ch,
        player_name: p,
    })
    .await;
    let unbanned = notified(&mut a, PLAYER_UNBANNED, "Custom").await;
    assert_eq!(
        (u64_at(&unbanned, 0), u64_at(&unbanned, 8)),
        (gb.guid(), ga.guid())
    );
    // The wrong password, then the right one.
    b.send(join("Custom", "guess")).await;
    notified(&mut b, WRONG_PASSWORD, "Custom").await;
    b.send(join("Custom", "secret")).await;
    notified(&mut b, YOU_JOINED, "Custom").await;
    // Alice hands it to Bob: his owner's flags, and the new owner.
    a.clear();
    a.send(CMSG_CHANNEL_SET_OWNER {
        channel_name: custom("Custom"),
        new_owner: "Bob".into(),
    })
    .await;
    let m = notified(&mut a, MODE_CHANGE, "Custom").await;
    assert_eq!(
        (u64_at(&m, 0), m[9]),
        (gb.guid(), MEMBER_OWNER | MEMBER_MODERATOR)
    );
    assert_eq!(
        u64_at(&notified(&mut a, OWNER_CHANGED, "Custom").await, 0),
        gb.guid()
    );
    // Bob invites Carol: she hears from whom, he hears she was asked.
    let (ch, p) = both("Carol");
    b.send(CMSG_CHANNEL_INVITE {
        channel_name: ch,
        player_name: p,
    })
    .await;
    assert_eq!(
        u64_at(&notified(&mut c, INVITE, "Custom").await, 0),
        gb.guid()
    );
    assert_eq!(notified(&mut b, PLAYER_INVITED, "Custom").await, b"Carol\0");
}

async fn played(c: &mut Client) -> (u32, u32) {
    c.send(CMSG_PLAYED_TIME {}).await;
    c.wait_for("SMSG_PLAYED_TIME", |m| match m {
        ServerOpcodeMessage::SMSG_PLAYED_TIME(p) => {
            Some((p.total_played_time, p.level_played_time))
        }
        _ => None,
    })
    .await
}

/// Thistle Tea's character_preferences_test: `/played` tells the total and
/// the level's time, counting the time since login and not the time
/// before it.
#[tokio::test]
async fn played_time_counts_the_session_and_not_the_time_away() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    on_map(&realm, move |m| {
        let now = m.now;
        let p = m.players.get_mut(&g).unwrap();
        p.row.played_total = 7_200;
        p.row.played_level = 600;
        p.played_at = now - Duration::from_secs(30);
    })
    .await;
    let (total, level) = played(&mut a).await;
    assert!((7_230..=7_231).contains(&total), "{total}");
    assert!((630..=631).contains(&level), "{level}");
    // Two seconds away count for nothing.
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(2_100)).await;
    a.clear();
    a.login(g).await;
    let (again, _) = played(&mut a).await;
    assert!(again <= total + 1, "{total} then {again}");
}

/// Thistle Tea's cmsg_page_text_query_test and cmsg_read_item_test: a
/// letter is read; a rock, or nothing, is not found; a page query sends
/// the whole chain, a missing page says so, and a page come back to ends
/// it.
#[tokio::test]
async fn letters_are_read_whole_and_rocks_are_not() {
    use crate::game::data::{GameData, ItemTemplate};
    const LETTER: u32 = 9_000;
    const ROCK: u32 = 9_001;
    let mut d = GameData::empty();
    let item = |entry, name: &str, page_text| ItemTemplate {
        entry,
        name: name.into(),
        allowable_class: -1,
        allowable_race: -1,
        stackable: 1,
        buy_count: 1,
        page_text,
        ..Default::default()
    };
    d.items.insert(LETTER, item(LETTER, "Sealed Letter", 9_101));
    d.items.insert(ROCK, item(ROCK, "Rock", 0));
    d.misc.page_texts.insert(9_101, ("Page one.".into(), 9_102));
    d.misc.page_texts.insert(9_102, ("Page two.".into(), 0));
    d.misc.page_texts.insert(9_104, ("Loop.".into(), 9_104));
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let (letter, rock) = on_map(&realm, move |m| {
        for entry in [LETTER, ROCK] {
            crate::game::map::inventory::add_item(m, g, entry, 1, false).unwrap();
        }
        let p = &m.players[&g];
        let at = |entry| {
            p.items
                .iter()
                .find(|i| i.entry == entry)
                .map(|i| (i.guid, i.bag, i.slot))
                .unwrap()
        };
        (at(LETTER), at(ROCK))
    })
    .await;
    a.clear();
    a.send(CMSG_READ_ITEM {
        bag_index: letter.1,
        slot: letter.2,
    })
    .await;
    let read = a
        .wait_for("SMSG_READ_ITEM_OK", |m| match m {
            ServerOpcodeMessage::SMSG_READ_ITEM_OK(r) => Some(r.guid),
            _ => None,
        })
        .await;
    assert_eq!(read, letter.0);
    // The rock, and the backpack's last slot, empty: not found.
    for (bag, slot) in [(rock.1, rock.2), (255, 38)] {
        a.send(CMSG_READ_ITEM {
            bag_index: bag,
            slot,
        })
        .await;
        let f = a
            .wait_frame("SMSG_INVENTORY_CHANGE_FAILURE", 0x112, |_| true)
            .await;
        assert_eq!(f[0], InventoryResult::ItemNotFound.as_int(), "{bag}/{slot}");
    }
    assert!(
        a.gets(Duration::from_millis(100), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_READ_ITEM_FAILED(_)).then_some(())
        })
        .await
        .is_none()
    );
    let page = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::SMSG_PAGE_TEXT_QUERY_RESPONSE(r) => {
            Some((r.page_id, r.text.clone(), r.next_page_id))
        }
        _ => None,
    };
    a.send(CMSG_PAGE_TEXT_QUERY { page_id: 9_101 }).await;
    assert_eq!(
        a.wait_for("page one", page).await,
        (9_101, "Page one.".into(), 9_102)
    );
    assert_eq!(
        a.wait_for("page two", page).await,
        (9_102, "Page two.".into(), 0)
    );
    a.send(CMSG_PAGE_TEXT_QUERY { page_id: 9_103 }).await;
    assert_eq!(
        a.wait_for("a missing page", page).await,
        (9_103, "Item page missing.".into(), 0)
    );
    a.send(CMSG_PAGE_TEXT_QUERY { page_id: 9_104 }).await;
    assert_eq!(a.wait_for("the loop", page).await.0, 9_104);
    settled(&mut a).await;
    assert!(
        a.gets(Duration::from_millis(100), page).await.is_none(),
        "the loop sent once"
    );
}

fn who(zones: Vec<u32>, search_strings: Vec<String>) -> CMSG_WHO {
    CMSG_WHO {
        minimum_level: Level::new(0),
        maximum_level: Level::new(60),
        player_name: String::new(),
        guild_name: String::new(),
        race_mask: 0,
        class_mask: 0,
        zones,
        search_strings,
    }
}

/// Thistle Tea's cmsg_who_test: a `/who` with more zones (ten) or search
/// words (four) than the client can send is turned away, unanswered; one
/// at the limit is answered.
#[tokio::test]
async fn a_who_with_more_than_the_client_sends_is_turned_away() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.clear();
    // Eleven zones, five words: either would list no one, if answered.
    a.send(who(vec![9_999; 11], Vec::new())).await;
    a.send(who(Vec::new(), vec!["zzz".into(); 5])).await;
    // Four words, each Alice's: she is listed.
    a.send(who(Vec::new(), vec!["ali".into(); 4])).await;
    let reply = a
        .wait_frame("SMSG_WHO", crate::game::map::social::SMSG_WHO, |_| true)
        .await;
    assert_eq!(
        &reply[..4],
        &1u32.to_le_bytes(),
        "the first answer is the last query's"
    );
}

/// Thistle Tea's cmsg_set_active_mover_test: a mover that isn't the player
/// or a unit they control is ignored.
#[tokio::test]
async fn an_active_mover_not_controlled_is_ignored() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.send(CMSG_SET_ACTIVE_MOVER { guid: gb }).await;
    settled(&mut a).await;
    let mover = on_map(&realm, move |m| m.players[&g].owned.mover).await;
    assert_ne!(mover, Some(gb), "Bob isn't Alice's to move");
    assert_eq!(mover, None);
}

fn zlib(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(data).unwrap();
    z.finish().unwrap()
}

fn upload(kind: u32, data: &[u8]) -> Vec<u8> {
    let mut body = kind.to_le_bytes().to_vec();
    body.extend_from_slice(&(data.len() as u32).to_le_bytes());
    if !data.is_empty() {
        body.extend_from_slice(&zlib(data));
    }
    body
}

async fn logout(c: &mut Client) {
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    c.clear();
}

async fn digests(c: &mut Client) -> Vec<u8> {
    c.wait_frame("SMSG_ACCOUNT_DATA_TIMES", 0x209, |_| true)
        .await
}

/// Thistle Tea's cmsg_update_account_data_test: the caches a client uploads
/// on the character screen, after its logout, are kept: the account's for
/// the account, the character's under the character last logged in, and a
/// size of zero erases one. A zero size is an erase, data that doesn't
/// inflate is nothing, and an upload missing its checksum still inflates.
#[tokio::test]
async fn caches_uploaded_after_logout_are_kept_for_the_last_character() {
    use crate::game::map::account_data::{digest, parse_update};
    let bindings = b"bind W MOVEFORWARD\n";
    let whole = zlib(bindings);
    let mut body = upload(2, bindings);
    assert_eq!(parse_update(&body), Some((2, bindings.to_vec())));
    body.truncate(body.len() - 4);
    assert_eq!(
        parse_update(&body),
        Some((2, bindings.to_vec())),
        "no checksum: {} of {} bytes",
        body.len() - 8,
        whole.len()
    );
    assert_eq!(parse_update(&upload(4, b"")), Some((4, Vec::new())));
    let mut junk = 4u32.to_le_bytes().to_vec();
    junk.extend_from_slice(&10u32.to_le_bytes());
    junk.extend_from_slice(b"not zlib");
    assert_eq!(parse_update(&junk), None);
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    logout(&mut a).await;
    // The chat settings (Alice's own) and macros (the account's).
    a.send_raw(0x20B, &upload(7, b"chat")).await;
    a.send_raw(0x20B, &upload(4, b"MACRO 1")).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    a.login(g).await;
    let times = digests(&mut a).await;
    assert_eq!(&times[7 * 16..8 * 16], &digest(b"chat"));
    assert_eq!(&times[4 * 16..5 * 16], &digest(b"MACRO 1"));
    // Erased after another logout.
    logout(&mut a).await;
    a.send_raw(0x20B, &upload(7, b"")).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    a.login(g).await;
    let times = digests(&mut a).await;
    assert_eq!(&times[7 * 16..8 * 16], &[0; 16]);
    assert_eq!(&times[4 * 16..5 * 16], &digest(b"MACRO 1"));
}

/// Thistle Tea's cmsg_cancel_growth_aura_test: `CMSG_CANCEL_GROWTH_AURA`
/// changes nothing, a growth aura and the player's scale staying, as
/// mangos's HandleCancelGrowthAuraOpcode has it.
#[tokio::test]
async fn cancelling_growth_leaves_the_player_as_big() {
    use crate::game::data::spells::{SpellEffect, SpellTemplate};
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura: crate::game::map::auras::AURA_MOD_SCALE,
        base_points: 19,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    let grow = SpellTemplate {
        id: 30_000,
        name: "Grow".into(),
        effects,
        equipped_item_class: -1,
        ..Default::default()
    };
    let scale = on_map(&realm, move |m| {
        crate::game::map::auras::apply(m, g, g, &grow, 0, 20);
        m.players[&g].fields.get_f32(OBJECT_FIELD_SCALE_X)
    })
    .await;
    a.send(CMSG_CANCEL_GROWTH_AURA {}).await;
    settled(&mut a).await;
    let (grown, after) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (
            p.auras.has_spell(30_000),
            p.fields.get_f32(OBJECT_FIELD_SCALE_X),
        )
    })
    .await;
    assert!(grown, "the aura stays");
    assert_eq!(after, scale);
}

fn moving(flags: MovementInfo_MovementFlags, x: f32, y: f32, z: f32) -> MovementInfo {
    MovementInfo {
        flags,
        timestamp: 0,
        position: Vector3d { x, y, z },
        orientation: 0.0,
        fall_time: 0.0,
    }
}

/// Thistle Tea's msg_move_test: a heartbeat standing still keeps a sitter
/// seated and their emote going; moving stands them up, ends the emote,
/// and tells their client with `SMSG_STANDSTATE_UPDATE`.
#[tokio::test]
async fn moving_stands_a_sitter_up_and_says_so() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let at = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.set_stand_state(1);
        p.fields.set_u32(UNIT_NPC_EMOTESTATE, 10);
        p.position()
    })
    .await;
    a.clear();
    let still = moving(MovementInfo_MovementFlags::empty(), at.x, at.y, at.z);
    a.send(MSG_MOVE_HEARTBEAT_Client { info: still }).await;
    settled(&mut a).await;
    let posture = move |m: &mut Map| {
        let p = &m.players[&g];
        (p.stand_state(), p.fields.get_u32(UNIT_NPC_EMOTESTATE))
    };
    assert_eq!(on_map(&realm, posture).await, (1, 10), "still seated");
    assert!(
        a.gets(Duration::ZERO, |m| {
            matches!(m, ServerOpcodeMessage::SMSG_STANDSTATE_UPDATE(_)).then_some(())
        })
        .await
        .is_none()
    );
    let forward = moving(
        MovementInfo_MovementFlags::new_forward(),
        at.x + 0.5,
        at.y,
        at.z,
    );
    a.send(MSG_MOVE_START_FORWARD_Client { info: forward })
        .await;
    let state = a
        .wait_for("SMSG_STANDSTATE_UPDATE", |m| match m {
            ServerOpcodeMessage::SMSG_STANDSTATE_UPDATE(s) => Some(s.state),
            _ => None,
        })
        .await;
    assert_eq!(state, UnitStandState::Stand);
    assert_eq!(on_map(&realm, posture).await, (0, 0));
}

/// Thistle Tea's cmsg_move_spline_done_test and cmsg_move_time_skipped_test:
/// a spline's end the server didn't start moves no one to where it says;
/// time skipped for another's guid is no one's, and one's own reaches
/// those who see them.
#[tokio::test]
async fn a_stray_spline_end_or_anothers_time_skip_counts_for_nothing() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(ga).await;
    let at = on_map(&realm, move |m| m.players[&ga].position()).await;
    a.send(CMSG_MOVE_SPLINE_DONE {
        info: moving(MovementInfo_MovementFlags::empty(), 900.0, 0.0, 0.0),
        movement_counter: 1,
        unknown1: 0,
    })
    .await;
    settled(&mut a).await;
    let now = on_map(&realm, move |m| m.players[&ga].position()).await;
    assert_eq!((now.x, now.y, now.z), (at.x, at.y, at.z), "not moved");
    b.clear();
    a.send(CMSG_MOVE_TIME_SKIPPED { guid: gb, lag: 25 }).await;
    a.send(CMSG_MOVE_TIME_SKIPPED { guid: ga, lag: 32 }).await;
    let (who, lag) = b
        .wait_for("MSG_MOVE_TIME_SKIPPED", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_TIME_SKIPPED(t) => Some((t.player, t.time_skipped)),
            _ => None,
        })
        .await;
    assert_eq!((who, lag), (ga, 32), "only Alice's own");
}

/// Thistle Tea's cmsg_use_item_test: an item whose spell takes time to cast
/// is kept while the cast goes on, and spent when it lands.
#[tokio::test]
async fn an_items_slow_spell_spends_it_only_when_cast() {
    use crate::game::data::GameData;
    use crate::game::data::items::ItemSpell;
    use crate::game::data::spells::{SpellEffect, SpellTemplate};
    const SCROLL: u32 = 9_010;
    const SLOW: u32 = 900_201;
    let mut d = GameData::empty();
    let mut slow = SpellTemplate {
        id: SLOW,
        name: "Slow Heal".into(),
        equipped_item_class: -1,
        range_index: 1,
        casting_time_index: 99,
        ..Default::default()
    };
    slow.effects[0] = SpellEffect {
        effect: 10,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(SLOW, slow);
    d.spells.cast_times.insert(99, 1_000);
    let mut scroll = crate::game::data::ItemTemplate {
        entry: SCROLL,
        name: "Scroll".into(),
        stackable: 5,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    };
    scroll.spells[0] = ItemSpell {
        id: SLOW,
        trigger: 0,
        charges: -1,
        cooldown: -1,
        ..Default::default()
    };
    d.items.insert(SCROLL, scroll);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let (bag, slot) = on_map(&realm, move |m| {
        crate::game::map::inventory::add_item(m, g, SCROLL, 2, false).unwrap();
        let i = m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == SCROLL)
            .unwrap();
        (i.bag, i.slot)
    })
    .await;
    a.send(CMSG_USE_ITEM {
        bag_index: bag,
        bag_slot: slot,
        spell_index: 0,
        targets: SpellCastTargets {
            target_flags: SpellCastTargets_SpellCastTargetFlags::empty(),
        },
    })
    .await;
    a.wait_for("SMSG_SPELL_START", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_START(s) if s.spell == SLOW => Some(()),
        _ => None,
    })
    .await;
    let count = move |m: &mut Map| m.players[&g].count_of(SCROLL);
    assert_eq!(on_map(&realm, count).await, 2, "kept while it casts");
    a.wait_for("SMSG_SPELL_GO", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == SLOW => Some(()),
        _ => None,
    })
    .await;
    settled(&mut a).await;
    assert_eq!(on_map(&realm, count).await, 1, "spent when it lands");
}

/// Thistle Tea's cmsg_use_item_test: an item whose spell changes it into
/// another, used, becomes that other where it lay; the charge it spends
/// doesn't take the new item too.
#[tokio::test]
async fn an_item_that_changes_into_another_is_not_spent_as_well() {
    use crate::game::data::GameData;
    use crate::game::data::items::ItemSpell;
    use crate::game::data::spells::{SpellEffect, SpellTemplate};
    const SOURCE: u32 = 9_020;
    const CHANGED: u32 = 9_021;
    const CHANGE: u32 = 900_202;
    let mut d = GameData::empty();
    let mut change = SpellTemplate {
        id: CHANGE,
        name: "Transform".into(),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    change.effects[0] = SpellEffect {
        effect: 34,
        misc_value: CHANGED as i32,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(CHANGE, change);
    let item = |entry, name: &str| crate::game::data::ItemTemplate {
        entry,
        name: name.into(),
        stackable: 1,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    };
    let mut source = item(SOURCE, "Unopened Gift");
    source.spells[0] = ItemSpell {
        id: CHANGE,
        trigger: 0,
        charges: -1,
        cooldown: -1,
        ..Default::default()
    };
    d.items.insert(SOURCE, source);
    d.items.insert(CHANGED, item(CHANGED, "Opened Gift"));
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let (bag, slot) = on_map(&realm, move |m| {
        crate::game::map::inventory::add_item(m, g, SOURCE, 1, false).unwrap();
        let i = m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == SOURCE)
            .unwrap();
        (i.bag, i.slot)
    })
    .await;
    a.send(CMSG_USE_ITEM {
        bag_index: bag,
        bag_slot: slot,
        spell_index: 0,
        targets: SpellCastTargets {
            target_flags: SpellCastTargets_SpellCastTargetFlags::empty(),
        },
    })
    .await;
    a.wait_for("SMSG_SPELL_GO", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == CHANGE => Some(()),
        _ => None,
    })
    .await;
    settled(&mut a).await;
    let (source, changed) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (
            p.count_of(SOURCE),
            p.items
                .iter()
                .find(|i| i.entry == CHANGED)
                .map(|i| (i.bag, i.slot)),
        )
    })
    .await;
    assert_eq!(source, 0);
    assert_eq!(changed, Some((bag, slot)), "the new item where the old lay");
}
