//! Round 8's player: the bank moves only with its banker near.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::inventory::{BANK_BAG_START, BANK_START, PACK_START};
use crate::game::map::{Map, MapHandle, MapKey};
use crate::game::store::BAG_PLAYER;

const THING: u32 = 20_000;
const STACK: u32 = 20_002;
const TRINKET: u32 = 20_001;
const PACK: (u8, u8) = (BAG_PLAYER, PACK_START);
const PACK2: (u8, u8) = (BAG_PLAYER, PACK_START + 1);
const BANK1: (u8, u8) = (BAG_PLAYER, BANK_START);
const BANK2: (u8, u8) = (BAG_PLAYER, BANK_START + 1);
const BANK3: (u8, u8) = (BAG_PLAYER, BANK_START + 2);
/// Thistle Tea's inventory result 35.
const TOO_FAR: &str = "TooFarAwayFromBank";

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
    d.items.insert(THING, item(THING));
    d.items.insert(
        STACK,
        ItemTemplate {
            stackable: 10,
            ..item(STACK)
        },
    );
    d.items.insert(
        TRINKET,
        ItemTemplate {
            class: 4,
            inventory_type: 12,
            ..item(TRINKET)
        },
    );
    d
}

async fn setup() -> (Arc<Realm>, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.clear();
        p.refresh_inventory(&d);
    })
    .await
    .unwrap();
    a.clear();
    (realm, a, g, map)
}

/// Give `count` of `entry` at `at`; its guid.
async fn give(map: &MapHandle, g: Guid, entry: u32, count: u32, at: (u8, u8)) -> Guid {
    map.query(move |m| {
        let d = m.realm.data.clone();
        let counter = m
            .realm
            .next_item
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let item = crate::game::guid::item(counter);
        let p = m.players.get_mut(&g).unwrap();
        p.items.push(crate::game::map::player::Item::new(
            &d, item, g, entry, at.0, at.1, count,
        ));
        p.refresh_inventory(&d);
        item
    })
    .await
    .unwrap()
}

/// Where `item` is, and how many.
async fn at(map: &MapHandle, g: Guid, item: Guid) -> Option<(u8, u8, u32)> {
    map.query(move |m| {
        let p = &m.players[&g];
        p.item_by_guid(item)
            .map(|i| (p.items[i].bag, p.items[i].slot, p.items[i].count()))
    })
    .await
    .unwrap()
}

/// The refusal `a` was sent for what it sent, by its name.
async fn refusal(a: &mut Client) -> Option<String> {
    a.send(CMSG_NAME_QUERY { guid: a.guid }).await;
    a.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_)).then_some(())
    })
    .await;
    let r = a
        .gets(Duration::from_millis(1), |m| match m {
            ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(f) => {
                Some(format!("{:?}", f.result))
            }
            _ => None,
        })
        .await;
    a.clear();
    r.map(|r| r.split([' ', '{']).next().unwrap().to_owned())
}

async fn swap(a: &mut Client, from: (u8, u8), to: (u8, u8)) -> Option<String> {
    a.clear();
    a.send(CMSG_SWAP_ITEM {
        destination_bag: to.0,
        destionation_slot: to.1,
        source_bag: from.0,
        source_slot: from.1,
    })
    .await;
    refusal(a).await
}

fn banker_of(m: &Map, g: Guid) -> Option<Guid> {
    m.players[&g].misc.banker
}

/// Thistle Tea's bank_test "generic inventory authorization" and
/// `authorize/1`: with no bank open, nothing goes into the bank, out of
/// it or about in it, by any packet (a swap, an auto-store from the bank
/// or into a bank bag, a split, a destroy, an equip by GUID, auto-bank and
/// auto-store-bank), each refused as too far from the bank and the items
/// where they were. Opened at a banker, the same moves go through. A
/// banker gone out of reach, or dead, shuts it: refused again, and still
/// refused once the banker is back in reach, until it is opened anew.
#[tokio::test]
async fn the_bank_moves_only_while_its_banker_serves() {
    let (_realm, mut a, g, map) = setup().await;
    let thing = give(&map, g, THING, 1, PACK).await;
    let stack = give(&map, g, STACK, 5, BANK1).await;
    let trinket = give(&map, g, TRINKET, 1, BANK2).await;
    assert_eq!(swap(&mut a, PACK, BANK3).await.as_deref(), Some(TOO_FAR));
    assert_eq!(swap(&mut a, BANK1, PACK).await.as_deref(), Some(TOO_FAR));
    a.send(CMSG_AUTOSTORE_BAG_ITEM {
        source_bag: BANK1.0,
        source_slot: BANK1.1,
        destination_bag: BAG_PLAYER,
    })
    .await;
    assert_eq!(
        refusal(&mut a).await.as_deref(),
        Some(TOO_FAR),
        "bank to bag"
    );
    a.send(CMSG_AUTOSTORE_BAG_ITEM {
        source_bag: PACK.0,
        source_slot: PACK.1,
        destination_bag: BANK_BAG_START,
    })
    .await;
    assert_eq!(
        refusal(&mut a).await.as_deref(),
        Some(TOO_FAR),
        "bag to a bank bag"
    );
    a.send(CMSG_SPLIT_ITEM {
        source_bag: BANK1.0,
        source_slot: BANK1.1,
        destination_bag: BANK3.0,
        destination_slot: BANK3.1,
        amount: 2,
    })
    .await;
    assert_eq!(refusal(&mut a).await.as_deref(), Some(TOO_FAR), "a split");
    a.send(CMSG_DESTROYITEM {
        bag: BANK1.0,
        slot: BANK1.1,
        amount: 0,
        unknown1: 0,
        unknown2: 0,
        unknown3: 0,
    })
    .await;
    assert_eq!(refusal(&mut a).await.as_deref(), Some(TOO_FAR), "a destroy");
    a.send(CMSG_AUTOEQUIP_ITEM_SLOT {
        guid: trinket,
        destination_slot: 13,
    })
    .await;
    assert_eq!(
        refusal(&mut a).await.as_deref(),
        Some(TOO_FAR),
        "equip by guid"
    );
    a.send(CMSG_AUTOBANK_ITEM {
        bag_index: PACK.0,
        slot_index: PACK.1,
    })
    .await;
    assert_eq!(refusal(&mut a).await.as_deref(), Some(TOO_FAR), "auto-bank");
    a.send(CMSG_AUTOSTORE_BANK_ITEM {
        bag_index: BANK1.0,
        slot_index: BANK1.1,
    })
    .await;
    assert_eq!(
        refusal(&mut a).await.as_deref(),
        Some(TOO_FAR),
        "auto-store-bank"
    );
    assert_eq!(at(&map, g, thing).await, Some((PACK.0, PACK.1, 1)));
    assert_eq!(at(&map, g, stack).await, Some((BANK1.0, BANK1.1, 5)));
    assert_eq!(at(&map, g, trinket).await, Some((BANK2.0, BANK2.1, 1)));

    // Open: the same go through.
    let banker = harness::open_bank(&map, g).await;
    assert_eq!(swap(&mut a, PACK, BANK3).await, None);
    assert_eq!(at(&map, g, thing).await, Some((BANK3.0, BANK3.1, 1)));
    a.send(CMSG_AUTOEQUIP_ITEM_SLOT {
        guid: trinket,
        destination_slot: 13,
    })
    .await;
    assert_eq!(refusal(&mut a).await, None);
    assert_eq!(at(&map, g, trinket).await, Some((BAG_PLAYER, 13, 1)));
    a.send(CMSG_AUTOSTORE_BANK_ITEM {
        bag_index: BANK3.0,
        slot_index: BANK3.1,
    })
    .await;
    assert_eq!(refusal(&mut a).await, None);
    assert_eq!(at(&map, g, thing).await, Some((PACK.0, PACK.1, 1)));

    // The banker walks off: shut, and shut still once it is back.
    let place = move |m: &mut Map, dx: f32| {
        let p = m.players[&g].position();
        m.creature_mut(banker).unwrap().position.x = p.x + dx;
    };
    map.query(move |m| place(m, 9.0)).await.unwrap();
    assert_eq!(swap(&mut a, PACK, BANK3).await.as_deref(), Some(TOO_FAR));
    assert_eq!(map.query(move |m| banker_of(m, g)).await.unwrap(), None);
    map.query(move |m| place(m, 1.0)).await.unwrap();
    assert_eq!(swap(&mut a, PACK, BANK3).await.as_deref(), Some(TOO_FAR));
    a.send(CMSG_BANKER_ACTIVATE { guid: banker }).await;
    assert_eq!(swap(&mut a, PACK, BANK3).await, None, "opened again");

    // Dead, it serves no one.
    map.query(move |m| {
        crate::game::map::combat::kill(m, g, banker);
    })
    .await
    .unwrap();
    assert_eq!(swap(&mut a, BANK3, PACK).await.as_deref(), Some(TOO_FAR));
    assert_eq!(at(&map, g, thing).await, Some((BANK3.0, BANK3.1, 1)));
}

/// Thistle Tea's bank_test "worldport and leave-world transitions clear
/// the capability": a bank open is shut by a worldport, and a login opens
/// none.
#[tokio::test]
async fn a_worldport_shuts_the_bank() {
    let (realm, mut a, g, map) = setup().await;
    harness::open_bank(&map, g).await;
    map.call(move |m| {
        m.teleport(
            g,
            MapKey::world(1),
            Vector3d {
                x: -618.5,
                y: -4251.7,
                z: 38.7,
            },
            0.0,
        )
    });
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(g).await;
    let kalimdor = realm.registry.online(g).unwrap().map();
    assert_eq!(kalimdor.key, MapKey::world(1));
    assert_eq!(
        kalimdor.query(move |m| banker_of(m, g)).await.unwrap(),
        None
    );
}

const PAPER: u32 = 5042;
const GIFT: u32 = 5043;

/// Thistle Tea's gifts_test "rejects bank access, remote control, death and
/// invalid paper without consuming items": an item in the bank isn't
/// wrapped with no bank open (too far from the bank), nor anything while
/// the client moves another unit; the paper and the item are kept, and
/// once both are put right it wraps.
#[tokio::test]
async fn wrapping_wants_the_bank_open_and_the_player_in_control() {
    let mut d = data();
    for (entry, stack) in [(PAPER, 10), (GIFT, 1)] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                name: format!("Item {entry}"),
                class: 15,
                flags: crate::game::map::items_misc::ITEM_FLAG_WRAPPER,
                stackable: stack,
                allowable_class: -1,
                allowable_race: -1,
                ..Default::default()
            },
        );
    }
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.clear();
        p.refresh_inventory(&d);
    })
    .await
    .unwrap();
    let paper = give(&map, g, PAPER, 3, PACK).await;
    let thing = give(&map, g, THING, 1, BANK1).await;
    a.clear();
    let wrap = |item: (u8, u8)| CMSG_WRAP_ITEM {
        gift_bag_index: PACK.0,
        gift_slot: PACK.1,
        item_bag_index: item.0,
        item_slot: item.1,
    };
    let kept = async |map: &MapHandle| {
        (
            at(map, g, paper).await.map(|x| x.2),
            map.query(move |m| m.players[&g].count_of(GIFT))
                .await
                .unwrap(),
        )
    };
    a.send(wrap(BANK1)).await;
    assert_eq!(refusal(&mut a).await.as_deref(), Some(TOO_FAR), "banked");
    assert_eq!(kept(&map).await, (Some(3), 0));
    // Out of the bank, but moving another unit.
    map.query(move |m| {
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_by_guid(thing).unwrap();
        (p.items[i].bag, p.items[i].slot) = PACK2;
        p.owned.mover = Some(crate::game::guid::unit(1, 9));
    })
    .await
    .unwrap();
    a.send(wrap(PACK2)).await;
    assert_eq!(refusal(&mut a).await.as_deref(), Some("CantDoRightNow"));
    assert_eq!(kept(&map).await, (Some(3), 0));
    map.query(move |m| m.players.get_mut(&g).unwrap().owned.mover = None)
        .await
        .unwrap();
    a.send(wrap(PACK2)).await;
    assert_eq!(refusal(&mut a).await, None);
    assert_eq!(kept(&map).await, (Some(2), 1), "wrapped");
}
