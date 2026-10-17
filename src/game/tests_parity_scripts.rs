//! Script commands and encounters as Thistle Tea runs them: the open
//! commands of `docs/AI5_GAPS.md`, summons' despawn types, the unkillable
//! flag, creature spell lists' last items, and an object opened by the item
//! that is its key (Molten Core's runes).

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::crafts::{LOCK_KEY_ITEM, LockKey};
use crate::game::data::items::ItemSpell;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

fn at(dx: f32, dy: f32) -> Vector3d {
    Vector3d {
        x: START.0 + dx,
        y: START.1 + dy,
        z: START.2,
    }
}

async fn player(realm: &Arc<Realm>, name: &str) -> (Client, MapHandle) {
    let mut c = Client::connect(realm, name);
    let me = c.enter(name, Race::Human, Class::Warrior).await;
    c.sees(me).await;
    let map = realm.registry.online(me).unwrap().map();
    (c, map)
}

const RUNE: u32 = 600;
const RUNE_LOCK: u32 = 900;
const QUINTESSENCE: u32 = 700;
const DOUSE: u32 = 6100;

/// A rune (a button) whose lock takes the Quintessence, and the
/// Quintessence, spent by its spell, that opens it.
fn rune_data() -> GameData {
    let mut d = GameData::empty();
    let mut rune = GameObjectTemplate {
        entry: RUNE,
        kind: 1,
        display_id: 2670,
        name: "Rune".into(),
        size: 1.0,
        ..Default::default()
    };
    rune.data[1] = RUNE_LOCK;
    d.gameobject_templates.insert(RUNE, rune);
    d.gameobject_spawns.insert(
        0,
        vec![GameObjectSpawn {
            guid: 1,
            entry: RUNE,
            map: 0,
            x: START.0 + 2.0,
            y: START.1,
            z: START.2,
            rotation: [0.0, 0.0, 0.0, 1.0],
            state: 1,
            ..Default::default()
        }],
    );
    d.crafts.locks.insert(
        RUNE_LOCK,
        vec![LockKey {
            kind: LOCK_KEY_ITEM,
            index: QUINTESSENCE,
            skill: 0,
        }],
    );
    let mut quintessence = ItemTemplate {
        entry: QUINTESSENCE,
        name: "Quintessence".into(),
        stackable: 1,
        ..Default::default()
    };
    quintessence.spells[0] = ItemSpell {
        id: DOUSE,
        charges: -1,
        cooldown: -1,
        ..Default::default()
    };
    d.items.insert(QUINTESSENCE, quintessence);
    let mut douse = SpellTemplate {
        id: DOUSE,
        name: "Douse".into(),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    douse.effects[0] = SpellEffect {
        effect: 59,
        target_a: 23,
        ..Default::default()
    };
    d.spells.spells.insert(DOUSE, douse);
    d
}

/// The item that is an object's key opens it, though its use spends it
/// before its spell lands (Aqual Quintessence on a rune of Molten Core).
#[tokio::test]
async fn an_object_opens_to_the_item_that_is_its_key() {
    let realm = harness::realm_with(rune_data());
    let (mut a, map) = player(&realm, "Alice").await;
    let me = a.guid;
    let rune = guid::game_object(RUNE, 1);
    a.sees(rune).await;
    let state = map
        .query(move |m| {
            let _ = crate::game::map::inventory::add_item(m, me, QUINTESSENCE, 1, false);
            let item = m.players[&me]
                .items
                .iter()
                .find(|i| i.entry == QUINTESSENCE)
                .map(|i| i.guid);
            crate::game::map::spells::cast(
                m,
                me,
                DOUSE,
                crate::game::map::spells::Targets {
                    object: Some(rune),
                    ..Default::default()
                },
                item,
            );
            let spent = !m.players[&me].items.iter().any(|i| i.entry == QUINTESSENCE);
            (
                spent,
                m.gameobject(rune)
                    .map(|o| o.fields.get_u32(GAMEOBJECT_STATE)),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        state,
        (true, Some(0)),
        "the Quintessence spent, the rune open"
    );
}
