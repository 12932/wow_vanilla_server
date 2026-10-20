//! Round 9's player: the order the bank's moves are told in (Thistle Tea's
//! `bank_test` "inventory packet order").

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;
use crate::game::map::inventory::{BANK_START, PACK_START};
use crate::game::store::BAG_PLAYER;

const THING: u32 = 20_000;
const OTHER: u32 = 20_001;
const STACK: u32 = 20_002;
const PACK: (u8, u8) = (BAG_PLAYER, PACK_START);
const BANK1: (u8, u8) = (BAG_PLAYER, BANK_START);
const BANK2: (u8, u8) = (BAG_PLAYER, BANK_START + 1);

fn data() -> GameData {
    let mut d = GameData::empty();
    let item = |entry: u32, stackable: u32| ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000 + entry,
        stackable,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    };
    d.items.insert(THING, item(THING, 1));
    d.items.insert(OTHER, item(OTHER, 1));
    d.items.insert(STACK, item(STACK, 10));
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
    harness::open_bank(&map, g).await;
    (realm, a, g, map)
}

/// Give `count` of `entry` at `at`, told to no one (as an item loaded);
/// its guid.
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

/// What a client is told of objects, in order.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Told {
    Destroyed(Guid),
    Created(Guid),
    Values(Guid),
}

/// Clear what `a` was told, before an action.
async fn quiet(a: &mut Client, map: &MapHandle) {
    harness::an_update(map).await;
    harness::an_update(map).await;
    a.clear();
}

/// What `a` was told of objects since [`quiet`], once the map's updates
/// have flushed what changed.
async fn told(a: &mut Client, map: &MapHandle) -> Vec<Told> {
    harness::an_update(map).await;
    harness::an_update(map).await;
    a.gets(Duration::from_millis(20), |_| None::<()>).await;
    let mut out = Vec::new();
    for m in a.backlog.drain(..) {
        if let ServerOpcodeMessage::SMSG_DESTROY_OBJECT(d) = &m {
            out.push(Told::Destroyed(d.guid));
        }
        out.extend(harness::creates(&m).into_iter().map(Told::Created));
        out.extend(
            harness::values(&m)
                .into_iter()
                .map(|(g, _)| Told::Values(g)),
        );
    }
    out
}

fn index(told: &[Told], what: Told) -> usize {
    told.iter()
        .position(|t| *t == what)
        .unwrap_or_else(|| panic!("{what:?} in {told:?}"))
}

/// Thistle Tea's "projects a direct transfer as one player values update":
/// two items swapped between the pack and the bank are told as the
/// player's slots alone, one values block, nothing of the items.
#[tokio::test]
async fn a_swap_with_the_bank_is_one_player_values_update() {
    let (_realm, mut a, g, map) = setup().await;
    let carried = give(&map, g, THING, 1, PACK).await;
    let banked = give(&map, g, OTHER, 1, BANK1).await;
    quiet(&mut a, &map).await;
    a.send(CMSG_SWAP_ITEM {
        destination_bag: BANK1.0,
        destionation_slot: BANK1.1,
        source_bag: PACK.0,
        source_slot: PACK.1,
    })
    .await;
    let told = told(&mut a, &map).await;
    assert_eq!(told, vec![Told::Values(g)], "the player's slots alone");
    let at = map
        .query(move |m| {
            let p = &m.players[&g];
            (
                p.item_at(PACK.0, PACK.1).map(|i| p.items[i].guid),
                p.item_at(BANK1.0, BANK1.1).map(|i| p.items[i].guid),
            )
        })
        .await
        .unwrap();
    assert_eq!(at, (Some(banked), Some(carried)));
}

/// Thistle Tea's "orders merge, split, and destroy projections": a stack
/// banked onto another is destroyed before the stack it joined is told its
/// count, and that before the player's slots; a split's new stack is
/// created before the old one's count and the slots; a destroyed stack
/// goes before the slots that held it.
#[tokio::test]
async fn a_merge_split_and_destroy_are_told_in_order() {
    let (_realm, mut a, g, map) = setup().await;
    let source = give(&map, g, STACK, 3, PACK).await;
    let destination = give(&map, g, STACK, 7, BANK1).await;
    quiet(&mut a, &map).await;
    a.send(CMSG_AUTOBANK_ITEM {
        bag_index: PACK.0,
        slot_index: PACK.1,
    })
    .await;
    let merged = told(&mut a, &map).await;
    assert!(
        index(&merged, Told::Destroyed(source)) < index(&merged, Told::Values(destination)),
        "{merged:?}"
    );
    assert!(
        index(&merged, Told::Values(destination)) < index(&merged, Told::Values(g)),
        "{merged:?}"
    );
    quiet(&mut a, &map).await;
    a.send(CMSG_SPLIT_ITEM {
        source_bag: BANK1.0,
        source_slot: BANK1.1,
        destination_bag: BANK2.0,
        destination_slot: BANK2.1,
        amount: 2,
    })
    .await;
    let split = told(&mut a, &map).await;
    let new = split
        .iter()
        .find_map(|t| match t {
            Told::Created(n) => Some(*n),
            _ => None,
        })
        .expect("the new stack created");
    assert!(
        index(&split, Told::Created(new)) < index(&split, Told::Values(destination)),
        "{split:?}"
    );
    assert!(
        index(&split, Told::Values(destination)) < index(&split, Told::Values(g)),
        "{split:?}"
    );
    quiet(&mut a, &map).await;
    a.send(CMSG_DESTROYITEM {
        bag: BANK2.0,
        slot: BANK2.1,
        amount: 0,
        unknown1: 0,
        unknown2: 0,
        unknown3: 0,
    })
    .await;
    let destroyed = told(&mut a, &map).await;
    assert!(
        index(&destroyed, Told::Destroyed(new)) < index(&destroyed, Told::Values(g)),
        "{destroyed:?}"
    );
}
