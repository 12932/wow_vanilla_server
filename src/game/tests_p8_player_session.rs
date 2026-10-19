//! Round 8's player: the character screen, the login and what comes
//! before the world.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::inventory::{BANK_BAG_START, BANK_START, PACK_START};
use crate::game::store::BAG_PLAYER;

const THING: u32 = 20_000;
const BANKED: u32 = 20_001;
const SATCHEL: u32 = 20_002;
const INSIDE: u32 = 20_003;
const WOLF: u32 = 299;

fn data() -> GameData {
    let mut d = GameData::empty();
    let item = |entry: u32| ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000 + entry,
        stackable: 1,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    };
    for e in [THING, BANKED, INSIDE] {
        d.items.insert(e, item(e));
    }
    d.items.insert(
        SATCHEL,
        ItemTemplate {
            class: 1,
            inventory_type: 18,
            container_slots: 6,
            ..item(SATCHEL)
        },
    );
    d.creature_templates.insert(
        WOLF,
        CreatureTemplate {
            entry: WOLF,
            name: "Wolf".into(),
            display_ids: [903, 0, 0, 0],
            family: 1,
            ..Default::default()
        },
    );
    d
}

async fn logged_out(c: &mut Client) {
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    c.clear();
}

/// The account's characters as the screen lists them.
async fn listed(c: &mut Client) -> Vec<Character> {
    c.send(CMSG_CHAR_ENUM {}).await;
    c.wait_for("SMSG_CHAR_ENUM", |m| match m {
        ServerOpcodeMessage::SMSG_CHAR_ENUM(e) => Some(e.characters.clone()),
        _ => None,
    })
    .await
}

/// Change the stored row of a character logged out.
async fn restore(
    realm: &Arc<Realm>,
    g: Guid,
    f: impl FnOnce(&mut crate::game::store::CharacterRow),
) {
    let mut row = realm.store.load(guid::counter(g)).await.unwrap();
    f(&mut row);
    realm.store.save(row);
    realm.store.flush().await;
}

/// Thistle Tea's login_test "creates carried and bank item objects exactly
/// once": at a login, an item carried, one in the bank, a bag in the
/// bank's bag slot and what is in it are each created once, and the
/// player's own fields hold the bank's and its bag slot's.
#[tokio::test]
async fn a_login_creates_each_item_carried_and_banked_once() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    logged_out(&mut a).await;
    let mut items = Vec::new();
    restore(&realm, g, |row| {
        row.items.clear();
        let next = 9_000_000;
        for (i, (entry, bag, slot)) in [
            (THING, BAG_PLAYER, PACK_START),
            (BANKED, BAG_PLAYER, BANK_START),
            (SATCHEL, BAG_PLAYER, BANK_BAG_START),
            (INSIDE, BANK_BAG_START, 0),
        ]
        .into_iter()
        .enumerate()
        {
            let guid = next + i as u32;
            row.items.push(crate::game::store::ItemRow {
                guid,
                entry,
                bag,
                slot,
                count: 1,
                ..Default::default()
            });
            items.push(guid::item(guid));
        }
        // The bank's first bag slot bought.
        row.state.bank_bag_slots = 1;
    })
    .await;
    a.login(g).await;
    a.sees(g).await;
    let map = realm.registry.online(g).unwrap().map();
    harness::an_update(&map).await;
    let mut created: ahash::AHashMap<Guid, u32> = Default::default();
    for frame in a.captured() {
        if let Ok(m) = ServerOpcodeMessage::read_unencrypted(&frame[..]) {
            for c in harness::creates(&m) {
                *created.entry(c).or_default() += 1;
            }
        }
    }
    for item in &items {
        assert_eq!(created.get(item), Some(&1), "{item} created once");
    }
    let (bank, bank_bag) = map
        .query(move |m| {
            let f = &m.players[&g].fields;
            (
                f.get_guid(PLAYER_FIELD_BANK_SLOT_1),
                f.get_guid(PLAYER_FIELD_BANKBAG_SLOT_1),
            )
        })
        .await
        .unwrap();
    assert_eq!((bank, bank_bag), (items[1], items[2]));
}

/// Thistle Tea's characters_test "projects hidden gear, ghost state, and
/// the waiting pet": the list shows the pet that comes back at login, its
/// look, level and family; a ghost's list shows none.
#[tokio::test]
async fn the_character_list_shows_the_waiting_pet_but_not_a_ghosts() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Huntress", Race::NightElf, Class::Hunter).await;
    logged_out(&mut a).await;
    restore(&realm, g, |row| {
        row.state.pets.current = Some(crate::game::store::PetRecord {
            number: 1,
            entry: WOLF,
            kind: 1,
            name: "Wolf".into(),
            level: 7,
            family: 1,
            ..Default::default()
        });
        row.state.pets.active = true;
    })
    .await;
    let c = listed(&mut a).await.remove(0);
    assert_eq!(
        (c.pet_display_id, c.pet_level.as_int(), c.pet_family),
        (903, 7, CreatureFamily::Wolf)
    );
    assert!(!c.flags.is_ghost());
    restore(&realm, g, |row| {
        row.state.dead = true;
        row.state.corpse = Some(Default::default());
    })
    .await;
    let c = listed(&mut a).await.remove(0);
    assert!(c.flags.is_ghost());
    assert_eq!((c.pet_display_id, c.pet_level.as_int()), (0, 0), "no pet");
}

/// Thistle Tea's characters_test "forgets the character, its items, and
/// its social row": deleted, the character's row goes with its items and
/// its friends, and it is off the list.
#[tokio::test]
async fn a_deleted_character_goes_with_its_items_and_friends() {
    let realm = harness::realm_with(data());
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.send(CMSG_ADD_FRIEND { name: "Bob".into() }).await;
    // SMSG_FRIEND_STATUS, laid out by hand.
    a.wait_frame("SMSG_FRIEND_STATUS", 0x068, |_| true).await;
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| crate::game::map::inventory::add_item(m, g, THING, 1, false))
        .await
        .unwrap()
        .unwrap();
    logged_out(&mut a).await;
    let row = realm.store.load(guid::counter(g)).await.unwrap();
    assert!(row.items.iter().any(|i| i.entry == THING));
    assert_eq!(row.state.friends.len(), 1);
    a.send(CMSG_CHAR_DELETE { guid: g }).await;
    let result = a
        .wait_for("SMSG_CHAR_DELETE", |m| match m {
            ServerOpcodeMessage::SMSG_CHAR_DELETE(d) => Some(d.result),
            _ => None,
        })
        .await;
    assert_eq!(result, WorldResult::CharDeleteSuccess);
    assert!(
        realm.store.load(guid::counter(g)).await.is_none(),
        "row and items gone"
    );
    assert!(listed(&mut a).await.is_empty());
}

/// Thistle Tea's logout_test "late packets after the player leaves keep
/// the character-selection connection alive" and social_test "does not
/// mutate lists before world entry": on the character screen a logout
/// request or cancel and a friend added do nothing, and the connection
/// stays; in the world, the friend list is empty.
#[tokio::test]
async fn the_character_screen_ignores_world_packets() {
    let realm = harness::realm_with(data());
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    logged_out(&mut a).await;
    a.send(CMSG_LOGOUT_CANCEL {}).await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.send(CMSG_ADD_FRIEND { name: "Bob".into() }).await;
    assert_eq!(listed(&mut a).await.len(), 1, "still answered");
    let told = a
        .gets(Duration::from_millis(50), |m| match m {
            ServerOpcodeMessage::SMSG_LOGOUT_CANCEL_ACK
            | ServerOpcodeMessage::SMSG_LOGOUT_RESPONSE(_)
            | ServerOpcodeMessage::SMSG_FRIEND_STATUS(_) => Some(m.to_string()),
            _ => None,
        })
        .await;
    assert_eq!(told, None);
    // SMSG_FRIEND_STATUS is laid out by hand: none among the frames.
    let statuses = a
        .captured()
        .iter()
        .filter(|f| u16::from_le_bytes([f[2], f[3]]) == 0x068)
        .count();
    assert_eq!(statuses, 0, "no friend status");
    a.login(g).await;
    let friends = a
        .wait_for("SMSG_FRIEND_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_FRIEND_LIST(l) => Some(l.friends.len()),
            _ => None,
        })
        .await;
    assert_eq!(friends, 0);
}

/// Thistle Tea's social_test "checks ignore state before creating a duel
/// and refreshes admission after removal": one who ignores another
/// refuses their duel, and takes it once the ignore is gone.
#[tokio::test]
async fn an_ignore_removed_lets_a_duel_be_asked_again() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let bob = b.enter("Bob", Race::Human, Class::Mage).await;
    let map = realm.registry.online(alice).unwrap().map();
    let check = move |m: &mut crate::game::map::Map| {
        let mut s = crate::game::data::spells::SpellTemplate {
            id: 7266,
            ..Default::default()
        };
        s.effects[0].effect = crate::game::map::spells::EFFECT_DUEL;
        crate::game::map::duel::check(m, alice, &s, Some(bob))
    };
    assert_eq!(map.query(check).await.unwrap(), None);
    b.send(CMSG_ADD_IGNORE {
        name: "Alice".into(),
    })
    .await;
    b.wait_for("SMSG_FRIEND_STATUS", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_FRIEND_STATUS(_)).then_some(())
    })
    .await;
    assert_eq!(map.query(check).await.unwrap(), Some(0x0a), "ignored");
    b.send(CMSG_DEL_IGNORE { guid: alice }).await;
    b.wait_for("SMSG_FRIEND_STATUS", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_FRIEND_STATUS(_)).then_some(())
    })
    .await;
    assert_eq!(map.query(check).await.unwrap(), None, "asked again");
}
