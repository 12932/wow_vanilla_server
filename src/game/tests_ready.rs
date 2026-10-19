//! Readying a character with GM commands, as peon's raid does each bot:
//! `.character level` and `.gear` leave it at full health and mana.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::{GameData, ItemTemplate};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::{MapHandle, MapKey};
use crate::game::tests_p3_entity::on;

/// A cloth chest of level 58: forty stamina and thirty intellect.
const ROBE: u32 = 973_001;

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.items.insert(
        ROBE,
        ItemTemplate {
            entry: ROBE,
            class: 4,
            subclass: 1,
            name: "Robe".into(),
            display_id: 1000,
            inventory_type: 5,
            quality: 3,
            required_level: 58,
            allowable_class: -1,
            allowable_race: -1,
            stats: [
                (7, 40),
                (5, 30),
                (0, 0),
                (0, 0),
                (0, 0),
                (0, 0),
                (0, 0),
                (0, 0),
                (0, 0),
                (0, 0),
            ],
            stackable: 1,
            ..Default::default()
        },
    );
    d
}

async fn player(realm: &Arc<Realm>, account: &str, class: Class) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(account, Race::Human, class).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    (c, g, map)
}

/// Health and mana, and their maximums.
async fn whole(map: &MapHandle, g: Guid) -> [u32; 4] {
    on(map, move |m| {
        let f = &m.players[&g].fields;
        [
            f.get_u32(UNIT_FIELD_HEALTH),
            f.get_u32(UNIT_FIELD_MAXHEALTH),
            f.get_u32(UNIT_FIELD_POWER1),
            f.get_u32(UNIT_FIELD_MAXPOWER1),
        ]
    })
    .await
}

/// mangos's `Player::GiveLevel`: a level set fills health and mana.
#[tokio::test]
async fn a_level_set_leaves_a_character_at_full_health_and_mana() {
    let realm = harness::realm_with(world());
    let (mut c, g, map) = player(&realm, "Ready", Class::Mage).await;
    // Hurt and drained, then levelled.
    on(&map, move |m| {
        let f = &mut m.players.get_mut(&g).unwrap().fields;
        f.set_u32(UNIT_FIELD_HEALTH, 1);
        f.set_u32(UNIT_FIELD_POWER1, 0);
    })
    .await;
    c.say(".character level 60").await;
    c.system_message("Level 60").await;
    let [health, max_health, mana, max_mana] = whole(&map, g).await;
    assert!(max_health > 100 && max_mana > 100, "a level-60 mage's");
    assert_eq!((health, mana), (max_health, max_mana));
}

/// The user's raiders came at half health: `.gear`'s stamina and intellect
/// raised their maximums and left them where they were. A GM's
/// convenience: it leaves them full, and says so.
#[tokio::test]
async fn gear_leaves_a_character_at_full_health_and_mana_and_says_so() {
    let realm = harness::realm_with(world());
    let (mut c, g, map) = player(&realm, "Geared", Class::Mage).await;
    c.say(".character level 60").await;
    c.system_message("Level 60").await;
    let before = whole(&map, g).await;
    c.say(".gear").await;
    let reply = c.system_message("Equipped").await;
    let [health, max_health, mana, max_mana] = whole(&map, g).await;
    let worn = on(&map, move |m| m.players[&g].count_of(ROBE)).await;
    assert_eq!(worn, 1, "the robe is worn");
    assert!(
        max_health >= before[1] + 400 && max_mana >= before[3] + 450,
        "the robe's stamina and intellect: {before:?} to {max_health}, {max_mana}"
    );
    assert_eq!((health, mana), (max_health, max_mana));
    assert!(reply.contains("full health and mana"), "{reply}");
}
