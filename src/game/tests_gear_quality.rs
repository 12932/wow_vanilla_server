//! `.gear green` and `.gear blue`: only the uncommon items, or only the
//! rare, where a slot has any of them; a slot with none of that quality is
//! filled from both, as `.gear` fills it.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, MapKey};
use crate::game::store::BAG_PLAYER;

/// A green ring and a blue one; a green cloak, and no blue one.
const GREEN_RING: u32 = 3401;
const BLUE_RING: u32 = 3402;
const GREEN_CLOAK: u32 = 3403;
const FINGERS: [u8; 2] = [10, 11];
const BACK: u8 = 14;

fn item(entry: u32, name: &str, inventory_type: u32, subclass: u32, quality: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: name.into(),
        class: 4,
        subclass,
        inventory_type,
        quality,
        required_level: 1,
        allowable_class: -1,
        allowable_race: -1,
        stackable: 1,
        buy_count: 1,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    for t in [
        item(GREEN_RING, "Green Band", 11, 0, 2),
        item(BLUE_RING, "Blue Band", 11, 0, 3),
        item(GREEN_CLOAK, "Green Cloak", 16, 1, 2),
    ] {
        d.items.insert(t.entry, t);
    }
    d
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

async fn entered(realm: &Arc<Realm>) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, "GEARQ");
    let g = c.enter("Gearq", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    (c, g, map)
}

/// What `g` wears on its fingers and its back.
async fn worn(map: &MapHandle, g: Guid) -> (Vec<u32>, Option<u32>) {
    on(map, move |m| {
        let p = &m.players[&g];
        let at = |s| p.item_at(BAG_PLAYER, s).map(|i| p.items[i].entry);
        (FINGERS.iter().filter_map(|s| at(*s)).collect(), at(BACK))
    })
    .await
}

/// `.gear green` puts on green rings only, `.gear blue` blue ones, and
/// each still fills the back with the green cloak, the only one there is;
/// `.gear` takes either ring. Again and again, as its picks are at random.
#[tokio::test]
async fn gear_of_a_quality_wears_only_it_where_a_slot_has_it() {
    let realm = harness::realm_with(data());
    let (mut c, g, map) = entered(&realm).await;
    let mut plain_rings = Vec::new();
    for _ in 0..6 {
        c.say(".gear green").await;
        c.system_message("Equipped").await;
        let (rings, back) = worn(&map, g).await;
        assert_eq!(rings, [GREEN_RING, GREEN_RING], "green");
        assert_eq!(back, Some(GREEN_CLOAK), "green");

        c.say(".gear blue").await;
        c.system_message("Equipped").await;
        let (rings, back) = worn(&map, g).await;
        assert_eq!(rings, [BLUE_RING, BLUE_RING], "blue");
        assert_eq!(back, Some(GREEN_CLOAK), "no blue cloak: the green one");

        c.say(".gear").await;
        c.system_message("Equipped").await;
        plain_rings.extend(worn(&map, g).await.0);
    }
    assert!(
        plain_rings.contains(&GREEN_RING) && plain_rings.contains(&BLUE_RING),
        "{plain_rings:?}"
    );
    c.say(".gear purple").await;
    c.system_message("Usage: .gear [green|blue|raid").await;
}
