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
