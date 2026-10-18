//! Phase 3's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/characters_test.exs`
//! (docs/parity/matrix.csv): what a new character starts with, and the
//! character list.

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::{GameData, ItemTemplate, StartInfo};
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;
use crate::game::store::BAG_PLAYER;
use crate::game::tests_p3_combat::START;
use crate::game::tests_p3_entity::on;

/// Rounds in twenties; a heap of them; a candle that doesn't stack; a
/// sword worn.
const ROUND: u32 = 64_600;
const HEAP: u32 = 64_601;
const CANDLE: u32 = 64_602;
const BLADE: u32 = 64_603;
const MISSING: u32 = 64_699;

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let item = |entry: u32, stackable: u32, inventory_type: u32| ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: entry - 64_000,
        stackable,
        inventory_type,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    };
    let mut blade = item(BLADE, 1, 13);
    blade.class = 2;
    blade.subclass = 7;
    blade.delay = 2000;
    for t in [
        item(ROUND, 20, 0),
        item(HEAP, 20, 0),
        item(CANDLE, 1, 0),
        blade,
    ] {
        d.items.insert(t.entry, t);
    }
    d.start.insert(
        (1, 1),
        StartInfo {
            map: 0,
            zone: 12,
            x: START.0,
            y: START.1,
            z: START.2,
            items: vec![
                (ROUND, u8::MAX, 45),
                (MISSING, u8::MAX, 1),
                (BLADE, u8::MAX, 1),
                (ROUND, u8::MAX, 10),
                (CANDLE, u8::MAX, 2),
                (HEAP, u8::MAX, 400),
            ],
            ..Default::default()
        },
    );
    d
}

/// Thistle Tea's "splits oversized starting quantities into legal item
/// instances", "rejects an oversized starting grant atomically when its
/// legal stacks cannot fit", "ignores missing starting item templates" and
/// "persists placed item state after a partial starting stack merge": a
/// new character's grants of one item are merged and dealt out in stacks
/// no bigger than it stacks, each its own item; a grant whose stacks don't
/// all fit isn't given at all, nor an item with no template; and so they
/// are after a logout.
#[tokio::test]
async fn starting_items_come_in_whole_stacks_or_not_at_all() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let held = move |m: &mut crate::game::map::Map| {
        let p = &m.players[&ga];
        let mut rounds: Vec<u32> = p
            .items
            .iter()
            .filter(|i| i.entry == ROUND)
            .map(|i| i.count())
            .collect();
        rounds.sort_unstable();
        let mut guids: Vec<_> = p.items.iter().map(|i| i.guid).collect();
        guids.sort_unstable_by_key(|g| g.guid());
        guids.dedup();
        let blade = p
            .items
            .iter()
            .find(|i| i.entry == BLADE)
            .map(|i| (i.bag, i.slot));
        (
            rounds,
            p.count_of(CANDLE),
            p.items.iter().filter(|i| i.entry == CANDLE).count(),
            p.count_of(HEAP),
            p.count_of(MISSING),
            blade,
            guids.len() == p.items.len(),
        )
    };
    let first = on(&map, held).await;
    assert_eq!(first.0, vec![15, 20, 20], "fifty-five rounds in stacks");
    assert_eq!((first.1, first.2), (2, 2), "two candles, one an item");
    assert_eq!(first.3, 0, "no heap: its twenty stacks don't fit");
    assert_eq!(first.4, 0, "nothing without a template");
    assert_eq!(first.5, Some((BAG_PLAYER, 15)), "the blade worn");
    assert!(first.6, "each its own item");
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.login(ga).await;
    let map = realm.registry.online(ga).unwrap().map();
    assert_eq!(on(&map, held).await, first, "as they were");
}

/// The names on `c`'s character list, and the look of each one's main
/// hand.
async fn list(c: &mut Client) -> Vec<(String, u32)> {
    c.send(CMSG_CHAR_ENUM {}).await;
    c.wait_for("SMSG_CHAR_ENUM", |m| match m {
        ServerOpcodeMessage::SMSG_CHAR_ENUM(e) => Some(
            e.characters
                .iter()
                .map(|c| (c.name.clone(), c.equipment[15].equipment_display_id))
                .collect(),
        ),
        _ => None,
    })
    .await
}

/// Thistle Tea's "lists the account's characters with their pose and
/// visible gear": the list shows the account's own characters only, with
/// what they wear.
#[tokio::test]
async fn the_character_list_shows_ones_own_characters_and_their_gear() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    assert_eq!(
        list(&mut a).await,
        vec![("Alice".to_string(), BLADE - 64_000)],
        "Alice, her blade showing"
    );
    let mut b = Client::connect(&realm, "B");
    assert_eq!(list(&mut b).await, vec![], "not another account's");
}
