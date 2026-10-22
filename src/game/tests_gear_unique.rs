//! `.gear` and uniqueness: a unique item (1.12's `max_count`) is never
//! worn twice, by `.gear` nor any other way in; and `.gear enchant`, a GM's
//! lasting enchantment on what is worn.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::Enchantment;
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::enchant;
use crate::game::map::inventory::{self, Destination, Fail};
use crate::game::map::{Map, MapHandle, MapKey};
use crate::game::store::BAG_PLAYER;

/// Two unique rings of one name (as the battlegrounds' are, at several
/// levels), another unique one, and a plain one; a sword.
const UNIQUE: u32 = 3301;
const UNIQUE_TOO: u32 = 3302;
const OTHER_UNIQUE: u32 = 3303;
const PLAIN: u32 = 3304;
const SWORD: u32 = 3305;
const FINGERS: [u8; 2] = [10, 11];
/// An enchantment: 15 stamina.
const STAMINA: u32 = 929;

fn ring(entry: u32, name: &str, max_count: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: name.into(),
        class: 4,
        subclass: 0,
        inventory_type: 11,
        quality: 2,
        required_level: 1,
        allowable_class: -1,
        allowable_race: -1,
        stackable: 1,
        buy_count: 1,
        max_count,
        ..Default::default()
    }
}

fn data(plain: bool) -> GameData {
    let mut d = GameData::empty();
    let mut rings = vec![
        ring(UNIQUE, "Band of One", 1),
        ring(UNIQUE_TOO, "Band of One", 1),
        ring(OTHER_UNIQUE, "Seal of Another", 1),
    ];
    if plain {
        rings.push(ring(PLAIN, "Plain Band", 0));
    }
    for t in rings {
        d.items.insert(t.entry, t);
    }
    d.items.insert(
        SWORD,
        ItemTemplate {
            entry: SWORD,
            name: "Short Sword".into(),
            class: 2,
            subclass: 7,
            inventory_type: 13,
            allowable_class: -1,
            allowable_race: -1,
            stackable: 1,
            buy_count: 1,
            max_durability: 50,
            ..Default::default()
        },
    );
    let mut stamina = Enchantment {
        id: STAMINA,
        name: "Stamina +15".into(),
        ..Default::default()
    };
    stamina.effects[0] = (enchant::TYPE_STAT, 15, 15, 7);
    d.spells.enchantments.insert(STAMINA, stamina);
    d
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

async fn entered(realm: &Arc<Realm>, account: &str, name: &str) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, Race::Human, Class::Warrior).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    (c, g, map)
}

/// What `g` wears on their fingers.
async fn fingers(map: &MapHandle, g: Guid) -> Vec<u32> {
    on(map, move |m| {
        let p = &m.players[&g];
        FINGERS
            .iter()
            .filter_map(|s| p.item_at(BAG_PLAYER, *s).map(|i| p.items[i].entry))
            .collect()
    })
    .await
}

/// What `g` wears on each finger.
async fn fingers_at(map: &MapHandle, g: Guid) -> (Option<u32>, Option<u32>) {
    on(map, move |m| {
        let p = &m.players[&g];
        let at = |s| p.item_at(BAG_PLAYER, s).map(|i| p.items[i].entry);
        (at(10), at(11))
    })
    .await
}

/// `.gear` with only unique rings to give fills one finger, never both
/// with the same ring nor with two of one name; with a second unique ring
/// it fills both, each different. Again and again, as its picks are at
/// random.
#[tokio::test]
async fn gear_never_puts_on_a_unique_ring_twice() {
    let realm = harness::realm_with(data(false));
    let (mut c, g, map) = entered(&realm, "UNIQG", "Uniqgear").await;
    let name = |e: u32| realm.data.item(e).unwrap().name.clone();
    for _ in 0..12 {
        c.say(".gear").await;
        c.system_message("Equipped").await;
        let worn = fingers(&map, g).await;
        assert_eq!(worn.len(), 2, "both fingers: {worn:?}");
        assert_ne!(name(worn[0]), name(worn[1]), "two alike: {worn:?}");
        assert!(worn.contains(&OTHER_UNIQUE), "{worn:?}");
        let carried = on(&map, move |m| {
            let p = &m.players[&g];
            [UNIQUE, UNIQUE_TOO, OTHER_UNIQUE].map(|e| p.count_of(e))
        })
        .await;
        assert!(carried.iter().all(|n| *n <= 1), "{carried:?}");
    }
}

/// With a plain ring too, a plain one may be on both fingers, but a unique
/// one is on one at most.
#[tokio::test]
async fn gear_may_put_a_plain_ring_on_both_fingers_but_a_unique_one_on_one() {
    let realm = harness::realm_with(data(true));
    let (mut c, g, map) = entered(&realm, "UNIQP", "Uniqplain").await;
    for _ in 0..12 {
        c.say(".gear").await;
        c.system_message("Equipped").await;
        let worn = fingers(&map, g).await;
        assert_eq!(worn.len(), 2, "{worn:?}");
        assert!(worn[0] != worn[1] || worn[0] == PLAIN, "{worn:?}");
        let ones = worn
            .iter()
            .filter(|e| [UNIQUE, UNIQUE_TOO].contains(e))
            .count();
        assert!(ones <= 1, "{worn:?}");
    }
}

/// A unique ring worn, no second copy comes by any way in: made worn on the
/// other finger (as `.gear` and a purchase into a slot make it), into the
/// bags, nor by `.additem`; the one there is moves from finger to finger.
#[tokio::test]
async fn a_second_copy_of_a_unique_ring_worn_is_refused_every_way() {
    let realm = harness::realm_with(data(false));
    let (mut c, g, map) = entered(&realm, "UNIQN", "Uniqnorm").await;
    let refused = on(&map, move |m| {
        inventory::add_item_to(m, g, UNIQUE, 1, Destination::Worn(10)).unwrap();
        let now = m.now;
        let p = &m.players[&g];
        let t = m.realm.data.item(UNIQUE).unwrap();
        let room = inventory::room_to(p, &m.realm.data, t, 1, Destination::Worn(11), now);
        let worn = inventory::add_item_to(m, g, UNIQUE, 1, Destination::Worn(11));
        let bags = inventory::add_item(m, g, UNIQUE, 1, true);
        // Another of its name is another item: a player may carry both.
        let other = inventory::add_item(m, g, UNIQUE_TOO, 1, true);
        (room, worn, bags, other)
    })
    .await;
    assert_eq!(refused.0, Err(Fail::CantCarryMore));
    assert_eq!(refused.1, Err(Fail::CantCarryMore));
    assert_eq!(refused.2, Err(Fail::CantCarryMore));
    assert_eq!(refused.3, Ok(()));
    c.say(&format!(".additem {UNIQUE}")).await;
    c.system_message("No room for Band of One").await;
    assert_eq!(fingers(&map, g).await, [UNIQUE]);
    // The one there is, from the first finger to the second.
    c.send(CMSG_SWAP_INV_ITEM {
        source_slot: ItemSlot::try_from(10u8).unwrap(),
        destination_slot: ItemSlot::try_from(11u8).unwrap(),
    })
    .await;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while fingers_at(&map, g).await != (None, Some(UNIQUE)) {
        assert!(tokio::time::Instant::now() < deadline, "the ring moved");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let count = on(&map, move |m| m.players[&g].count_of(UNIQUE)).await;
    assert_eq!(count, 1);
}

/// `.gear enchant <slot> <id>` puts a lasting enchantment on what is worn
/// there, by number or name, its stats with it; what isn't worn, an
/// enchantment that isn't, or a bad slot, are said so.
#[tokio::test]
async fn gear_enchant_puts_a_lasting_enchantment_on_what_is_worn() {
    let realm = harness::realm_with(data(false));
    let (mut c, g, map) = entered(&realm, "UNIQE", "Uniqench").await;
    let stamina = move |m: &Map| {
        m.players[&g]
            .fields
            .get_u32(crate::game::fields::UNIT_FIELD_STAT0 + 2)
    };
    let before = on(&map, move |m| {
        inventory::add_item_to(m, g, UNIQUE, 1, Destination::Worn(10)).unwrap();
        stamina(m)
    })
    .await;
    c.say(&format!(".gear enchant finger1 {STAMINA}")).await;
    let reply = c.system_message("Enchanted").await;
    assert_eq!(reply, "Enchanted Band of One with Stamina +15.");
    let (on_it, after) = on(&map, move |m| {
        let p = &m.players[&g];
        let i = p.item_at(BAG_PLAYER, 10).unwrap();
        (enchant::get(&p.items[i], enchant::PERM_SLOT).id, stamina(m))
    })
    .await;
    assert_eq!(on_it, STAMINA);
    assert_eq!(after, before + 15, "its stamina");
    c.say(&format!(".gear enchant 11 {STAMINA}")).await;
    c.system_message("Nothing worn in slot 11").await;
    c.say(".gear enchant 10 4242").await;
    c.system_message("No enchantment 4242").await;
    c.say(".gear enchant elbow 929").await;
    c.system_message("Usage: .gear enchant").await;
    c.say(".gear enchant 10").await;
    c.system_message("Usage: .gear enchant").await;
}
