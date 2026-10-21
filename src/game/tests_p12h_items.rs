//! Round 12's stage h, the inventory as mangos-classic's `Player`: a bag
//! with things in it swapped for an empty one, its things moved into it,
//! or moved between bags' places with them (`SwapItem`).

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::inventory::{BAG_START, PACK_START};
use crate::game::map::{Map, MapKey};
use crate::game::store::BAG_PLAYER;

const SMALL: u32 = 9600;
const BIG: u32 = 9601;
const TINY: u32 = 9602;
const HERB_BAG: u32 = 9603;
const STONE: u32 = 9604;

pub(crate) fn item(entry: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000 + entry,
        stackable: 1,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    }
}

fn bag(entry: u32, subclass: u32, family: u32, slots: u32) -> ItemTemplate {
    ItemTemplate {
        class: 1,
        subclass,
        inventory_type: 18,
        container_slots: slots,
        bag_family: family,
        ..item(entry)
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    for t in [
        bag(SMALL, 0, 0, 6),
        bag(BIG, 0, 0, 10),
        bag(TINY, 0, 0, 2),
        bag(HERB_BAG, 2, 32, 12),
        item(STONE),
    ] {
        d.items.insert(t.entry, t);
    }
    d
}

pub(crate) async fn on_map<T: Send + 'static>(
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

/// A player carrying nothing.
async fn setup() -> (Arc<Realm>, Client, Guid) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.clear();
        p.refresh_inventory(&d);
    })
    .await;
    a.clear();
    (realm, a, g)
}

/// Give one of `entry` at `at`; its guid.
pub(crate) async fn give_at(realm: &Arc<Realm>, g: Guid, entry: u32, at: (u8, u8)) -> Guid {
    on_map(realm, move |m| {
        let d = m.realm.data.clone();
        let counter = m
            .realm
            .next_item
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let guid = crate::game::guid::item(counter);
        let p = m.players.get_mut(&g).unwrap();
        assert!(p.item_at(at.0, at.1).is_none(), "{at:?} is free");
        p.items.push(crate::game::map::player::Item::new(
            &d, guid, g, entry, at.0, at.1, 1,
        ));
        p.refresh_inventory(&d);
        guid
    })
    .await
}

/// Where `item` is.
pub(crate) async fn at(realm: &Arc<Realm>, g: Guid, item: Guid) -> Option<(u8, u8)> {
    on_map(realm, move |m| {
        let p = &m.players[&g];
        p.item_by_guid(item)
            .map(|i| (p.items[i].bag, p.items[i].slot))
    })
    .await
}

/// The refusal `a` was sent for its last packet, by its name.
pub(crate) async fn refusal(a: &mut Client) -> Option<String> {
    crate::game::tests_p3_player2_ammo::handled(a).await;
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

pub(crate) async fn swap(a: &mut Client, from: (u8, u8), to: (u8, u8)) -> Option<String> {
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

const BAR1: (u8, u8) = (BAG_PLAYER, BAG_START);
const BAR2: (u8, u8) = (BAG_PLAYER, BAG_START + 1);
const PACK: (u8, u8) = (BAG_PLAYER, PACK_START);

/// mangos's `SwapItem`, "bag swap with items exchange": an empty bag in
/// the pack dragged onto a full one on the bag bar takes its place with its
/// things, in order from its first slot, and the old bag goes to the pack
/// empty; refused if they are too many for it (`ITEMS_CANT_BE_SWAPPED`) or
/// don't go into it (`NONEMPTY_BAG_OVER_OTHER_BAG`). wvs refused any move
/// of a bag with things in it.
#[tokio::test]
async fn a_full_bag_is_swapped_for_an_empty_one_its_things_moved() {
    let (realm, mut a, g) = setup().await;
    let small = give_at(&realm, g, SMALL, BAR1).await;
    let stones = [
        give_at(&realm, g, STONE, (BAG_START, 1)).await,
        give_at(&realm, g, STONE, (BAG_START, 3)).await,
        give_at(&realm, g, STONE, (BAG_START, 5)).await,
    ];
    let tiny = give_at(&realm, g, TINY, PACK).await;
    assert_eq!(
        swap(&mut a, PACK, BAR1).await.as_deref(),
        Some("ItemsCantBeSwapped"),
        "three into two"
    );
    let herbs = give_at(&realm, g, HERB_BAG, (BAG_PLAYER, PACK_START + 1)).await;
    assert_eq!(
        swap(&mut a, (BAG_PLAYER, PACK_START + 1), BAR1)
            .await
            .as_deref(),
        Some("NonemptyBagOverOtherBag"),
        "stones aren't herbs"
    );
    let big = give_at(&realm, g, BIG, (BAG_PLAYER, PACK_START + 2)).await;
    assert_eq!(swap(&mut a, (BAG_PLAYER, PACK_START + 2), BAR1).await, None);
    assert_eq!(at(&realm, g, big).await, Some(BAR1));
    assert_eq!(
        at(&realm, g, small).await,
        Some((BAG_PLAYER, PACK_START + 2))
    );
    for (n, stone) in stones.iter().enumerate() {
        assert_eq!(at(&realm, g, *stone).await, Some((BAG_START, n as u8)));
    }
    let shown = on_map(&realm, move |m| {
        let p = &m.players[&g];
        let f = &p.items[p.item_by_guid(big).unwrap()].fields;
        let s = &p.items[p.item_by_guid(stones[2]).unwrap()].fields;
        (
            f.get_guid(CONTAINER_FIELD_SLOT_1 + 2 * 2),
            s.get_guid(ITEM_FIELD_CONTAINED),
        )
    })
    .await;
    assert_eq!(shown, (stones[2], big), "the client's slots");
    // The other way: the full bag dragged onto an empty one in the pack.
    assert_eq!(swap(&mut a, BAR1, (BAG_PLAYER, PACK_START + 2)).await, None);
    assert_eq!(at(&realm, g, small).await, Some(BAR1));
    assert_eq!(at(&realm, g, big).await, Some((BAG_PLAYER, PACK_START + 2)));
    assert_eq!(at(&realm, g, stones[0]).await, Some((BAG_START, 0)));
    // A full bag goes into no plain slot, nor into itself.
    assert_eq!(
        swap(&mut a, BAR1, (BAG_PLAYER, PACK_START + 5))
            .await
            .as_deref(),
        Some("CanOnlyDoWithEmptyBags")
    );
    assert_eq!(
        swap(&mut a, BAR1, (BAG_START, 4)).await.as_deref(),
        Some("NonemptyBagOverOtherBag")
    );
    let _ = (tiny, herbs);
}

/// mangos's bags carry their things: a full bag moved along the bag bar,
/// or two swapped there, keep what is in them.
#[tokio::test]
async fn full_bags_move_along_the_bar_with_their_things() {
    let (realm, mut a, g) = setup().await;
    let small = give_at(&realm, g, SMALL, BAR1).await;
    let one = give_at(&realm, g, STONE, (BAG_START, 2)).await;
    assert_eq!(swap(&mut a, BAR1, BAR2).await, None);
    assert_eq!(at(&realm, g, small).await, Some(BAR2));
    assert_eq!(at(&realm, g, one).await, Some((BAG_START + 1, 2)));
    let big = give_at(&realm, g, BIG, BAR1).await;
    let two = give_at(&realm, g, STONE, (BAG_START, 7)).await;
    assert_eq!(swap(&mut a, BAR1, BAR2).await, None);
    assert_eq!(at(&realm, g, big).await, Some(BAR2));
    assert_eq!(at(&realm, g, small).await, Some(BAR1));
    assert_eq!(at(&realm, g, one).await, Some((BAG_START, 2)));
    assert_eq!(at(&realm, g, two).await, Some((BAG_START + 1, 7)));
}
