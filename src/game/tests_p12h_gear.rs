//! Round 12's stage h, gear and stacks as mangos-classic's `Player` has
//! them: nothing put on while dead, stunned, logging out or casting
//! (`SwapItem`, `CanEquipItem`, `CanUnequipItem`); no melee mounted
//! (`Unit::Attack`); a split onto a stack of its own (`SplitItem`).

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::inventory::PACK_START;
use crate::game::map::{Map, MapHandle};
use crate::game::store::BAG_PLAYER;
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p12h_items::{at, refusal, swap};

const AMULET: u32 = 9650;
const PEBBLE: u32 = 9651;
const NECK: (u8, u8) = (BAG_PLAYER, 1);
const PACK: (u8, u8) = (BAG_PLAYER, PACK_START);

fn data() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.items.insert(
        AMULET,
        ItemTemplate {
            class: 4,
            subclass: 0,
            inventory_type: 2,
            ..crate::game::tests_p12h_items::item(AMULET)
        },
    );
    d.items.insert(
        PEBBLE,
        ItemTemplate {
            stackable: 12,
            ..crate::game::tests_p12h_items::item(PEBBLE)
        },
    );
    d
}

async fn setup() -> (Arc<Realm>, Client, Guid, MapHandle) {
    let (realm, mut a, g, map) = setup_with(data(), Class::Warrior).await;
    on(&map, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.clear();
        p.refresh_inventory(&d);
    })
    .await;
    a.clear();
    (realm, a, g, map)
}

type Change = Box<dyn FnOnce(&mut Map) + Send>;

/// The refusal of a swap from `from` to `to` with `f` made, undone after.
async fn refused(
    map: &MapHandle,
    a: &mut Client,
    f: Change,
    undo: Change,
    from: (u8, u8),
    to: (u8, u8),
) -> Option<String> {
    on(map, f).await;
    let r = swap(a, from, to).await;
    on(map, undo).await;
    r
}

async fn give(map: &MapHandle, g: Guid, entry: u32, count: u32, at: (u8, u8)) -> Guid {
    on(map, move |m| {
        let d = m.realm.data.clone();
        let counter = m
            .realm
            .next_item
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let guid = crate::game::guid::item(counter);
        let p = m.players.get_mut(&g).unwrap();
        p.items.push(crate::game::map::player::Item::new(
            &d, guid, g, entry, at.0, at.1, count,
        ));
        p.refresh_inventory(&d);
        guid
    })
    .await
}

/// mangos's `SwapItem` (`YOU_ARE_DEAD`), `CanEquipItem` (`YOU_ARE_STUNNED`
/// stunned or logging out, `CANT_DO_RIGHT_NOW` casting or in Spirit of
/// Redemption) and `CanUnequipItem` (`YOU_ARE_STUNNED` logging out): wvs
/// let a ghost, the stunned and a caster change their gear (a +healing
/// weapon swapped in mid-cast).
#[tokio::test]
async fn gear_isnt_changed_dead_stunned_casting_or_logging_out() {
    let (realm, mut a, g, map) = setup().await;
    let amulet = give(&map, g, AMULET, 1, PACK).await;
    let stun = |on: bool| {
        Box::new(move |m: &mut Map| {
            m.players.get_mut(&g).unwrap().fields.set_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_STUNNED,
                on,
            )
        }) as Change
    };
    assert_eq!(
        refused(&map, &mut a, stun(true), stun(false), PACK, NECK)
            .await
            .as_deref(),
        Some("YouAreStunned")
    );
    let cast = |on: bool| {
        Box::new(move |m: &mut Map| {
            let now = m.now;
            m.players.get_mut(&g).unwrap().caster.casting =
                on.then(|| crate::game::map::spells::Casting {
                    spell: 1,
                    targets: Default::default(),
                    ends: now + std::time::Duration::from_secs(30),
                    item: None,
                    pushbacks: 0,
                    channel_until: None,
                    duration: 30_000,
                    at: None,
                });
        }) as Change
    };
    assert_eq!(
        refused(&map, &mut a, cast(true), cast(false), PACK, NECK)
            .await
            .as_deref(),
        Some("CantDoRightNow")
    );
    // Free to: on, and then not off while logging out.
    assert_eq!(swap(&mut a, PACK, NECK).await, None);
    assert_eq!(at(&realm, g, amulet).await, Some(NECK));
    let logout = |on: bool| {
        Box::new(move |m: &mut Map| {
            let now = m.now;
            m.players.get_mut(&g).unwrap().logout_at =
                on.then(|| now + std::time::Duration::from_secs(20));
        }) as Change
    };
    assert_eq!(
        refused(&map, &mut a, logout(true), logout(false), NECK, PACK)
            .await
            .as_deref(),
        Some("YouAreStunned")
    );
    // Nothing moves for the dead.
    on(&map, move |m| crate::game::map::death::kill(m, g)).await;
    assert_eq!(
        swap(&mut a, NECK, PACK).await.as_deref(),
        Some("YouAreDead")
    );
    assert_eq!(at(&realm, g, amulet).await, Some(NECK));
}

/// mangos's `Unit::Attack`: "player cannot attack in mount state". wvs
/// started a mounted player's swings.
#[tokio::test]
async fn a_mounted_player_doesnt_melee() {
    let (_realm, mut a, g, map) = setup().await;
    let brute = on(&map, move |m| spawn(m, g, BRUTE, 20, 2.0)).await;
    a.sees(brute).await;
    on(&map, move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_MOUNTDISPLAYID, 2402)
    })
    .await;
    a.clear();
    a.send(CMSG_ATTACKSWING { guid: brute }).await;
    a.wait_for("SMSG_ATTACKSWING_CANT_ATTACK", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_ATTACKSWING_CANT_ATTACK).then_some(())
    })
    .await;
    let attacking = on(&map, move |m| m.combat(g).is_some_and(|c| c.attacking)).await;
    assert!(!attacking);
}

/// mangos's `SplitItem`: a split onto a stack of the same item with room
/// for it joins it (`CanStoreItem` at the slot). wvs refused any occupied
/// slot (`ITEM_CANT_STACK`).
#[tokio::test]
async fn a_split_joins_a_stack_of_its_own() {
    let (realm, mut a, g, map) = setup().await;
    let from = give(&map, g, PEBBLE, 10, PACK).await;
    let onto = give(&map, g, PEBBLE, 5, (BAG_PLAYER, PACK_START + 1)).await;
    let count = |item: Guid| {
        let map = map.clone();
        async move {
            on(&map, move |m| {
                let p = &m.players[&g];
                p.items[p.item_by_guid(item).unwrap()].count()
            })
            .await
        }
    };
    a.clear();
    a.send(CMSG_SPLIT_ITEM {
        source_bag: PACK.0,
        source_slot: PACK.1,
        destination_bag: BAG_PLAYER,
        destination_slot: PACK_START + 1,
        amount: 4,
    })
    .await;
    assert_eq!(refusal(&mut a).await, None);
    assert_eq!((count(from).await, count(onto).await), (6, 9));
    // More than the stack has room for is refused.
    a.send(CMSG_SPLIT_ITEM {
        source_bag: PACK.0,
        source_slot: PACK.1,
        destination_bag: BAG_PLAYER,
        destination_slot: PACK_START + 1,
        amount: 5,
    })
    .await;
    assert_eq!(refusal(&mut a).await.as_deref(), Some("ItemCantStack"));
    assert_eq!((count(from).await, count(onto).await), (6, 9));
    let _ = realm;
}

const TUNIC: u32 = 9652;
const MAKE_TUNIC: u32 = 900_850;
const MAKE_PEBBLES: u32 = 900_851;

/// mangos's `DoCreateItem` (`HasCreator`) and `Item::SaveToDB`: a made
/// item that isn't a stack, a consumable or a quest item shows "Made by"
/// its maker, kept with it; wvs never set `ITEM_FIELD_CREATOR`.
#[tokio::test]
async fn a_made_item_names_its_maker_and_keeps_it() {
    use crate::game::data::spells::{SpellEffect, SpellTemplate};
    let mut d = data();
    d.items.insert(
        TUNIC,
        ItemTemplate {
            class: 4,
            subclass: 1,
            inventory_type: 5,
            ..crate::game::tests_p12h_items::item(TUNIC)
        },
    );
    for (id, item) in [(MAKE_TUNIC, TUNIC), (MAKE_PEBBLES, PEBBLE)] {
        let mut effects = [SpellEffect::default(); 3];
        effects[0] = SpellEffect {
            effect: 24,
            item_type: u64::from(item),
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        };
        d.spells.spells.insert(
            id,
            SpellTemplate {
                id,
                name: format!("Make {item}"),
                effects,
                equipped_item_class: -1,
                range_index: 1,
                ..Default::default()
            },
        );
    }
    let (_realm, a, g, map) = setup_with(d, Class::Warrior).await;
    for spell in [MAKE_TUNIC, MAKE_PEBBLES] {
        on(&map, move |m| crate::game::map::spells::learn(m, g, spell)).await;
        a.send(CMSG_CAST_SPELL {
            spell,
            targets: crate::game::map::spells::wire_targets(&Default::default()),
        })
        .await;
    }
    let made = move |m: &mut Map| {
        let data = m.realm.data.clone();
        let p = &m.players[&g];
        let of = |entry: u32| {
            p.items.iter().find(|i| i.entry == entry).map(|i| {
                let kept = crate::game::map::player::Item::from_row(&data, &i.to_row(), g);
                (
                    i.fields.get_guid(ITEM_FIELD_CREATOR),
                    kept.fields.get_guid(ITEM_FIELD_CREATOR),
                )
            })
        };
        (of(TUNIC), of(PEBBLE))
    };
    let mut got = (None, None);
    for _ in 0..100 {
        got = on(&map, made).await;
        if got.0.is_some() && got.1.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(got.0, Some((g, g)), "the tunic's maker, kept");
    assert_eq!(
        got.1,
        Some((Guid::zero(), Guid::zero())),
        "a stack names none"
    );
}
