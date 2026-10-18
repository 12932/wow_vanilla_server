//! Phase 3's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/{bank,npc_reach,vendor_stock,buyback}_test.exs`
//! (docs/parity/matrix.csv): who a banker or vendor serves, how far an NPC
//! reaches, and what a refused purchase or buyback leaves.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::{CreatureTemplate, GameData, VendorItem};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::creature::Life;
use crate::game::map::{Map, MapHandle, npc, vendor};
use crate::game::tests_p3_combat::{HUMANS, PACK, setup_with, spawn, template};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_player2_ammo::handled;

/// A banker and vendor of the humans', and one of their enemies'.
const CLERK: u32 = 3720;
const ENEMY_CLERK: u32 = 3721;
/// Sold three at a time, ten in stock; and a filler.
const WARES: u32 = 64_300;
const FILLER: u32 = 64_301;

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let clerk = |entry: u32, faction: u32| CreatureTemplate {
        faction,
        npc_flags: npc::NPC_FLAG_BANKER | vendor::NPC_FLAG_VENDOR,
        ..template(entry, 7)
    };
    d.creature_templates.insert(CLERK, clerk(CLERK, HUMANS));
    d.creature_templates
        .insert(ENEMY_CLERK, clerk(ENEMY_CLERK, PACK));
    for (entry, stack) in [(WARES, 20), (FILLER, 1)] {
        d.items.insert(
            entry,
            crate::game::data::ItemTemplate {
                entry,
                name: format!("Item {entry}"),
                display_id: 1000,
                stackable: stack,
                allowable_class: -1,
                allowable_race: -1,
                buy_count: 3,
                buy_price: 30,
                sell_price: 5,
                ..Default::default()
            },
        );
    }
    d.vendors.insert(
        CLERK,
        vec![VendorItem {
            slot: 0,
            item: WARES,
            max_count: 10,
            restock_secs: 600,
            ..Default::default()
        }],
    );
    d
}

/// Alice, a clerk `yards` before her, and her client seeing it.
async fn clerk(
    entry: u32,
    yards: f32,
) -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Guid,
    MapHandle,
    Guid,
) {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let c = on(&map, move |m| spawn(m, ga, entry, 20, yards)).await;
    a.sees(c).await;
    (realm, a, ga, map, c)
}

/// Whether `a` is shown the bank when it asks `banker`.
async fn shown_bank(a: &mut Client, banker: Guid) -> bool {
    a.clear();
    a.send(CMSG_BANKER_ACTIVATE { guid: banker }).await;
    handled(a).await;
    a.gets(std::time::Duration::from_millis(1), |m| match m {
        ServerOpcodeMessage::SMSG_SHOW_BANK(s) if s.guid == banker => Some(()),
        _ => None,
    })
    .await
    .is_some()
}

/// `c` at `yards` from `g` along the x axis (where `g` faces).
fn place(m: &mut Map, g: Guid, c: Guid, yards: f32) {
    let at = m.players[&g].position();
    m.creature_mut(c).unwrap().position = Vector3d {
        x: at.x + yards,
        ..at
    };
}

fn radius(m: &mut Map, g: Guid, r: f32) {
    if let Some(p) = m.players.get_mut(&g) {
        p.fields.set_f32(UNIT_FIELD_BOUNDINGRADIUS, r);
    } else if let Some(c) = m.creature_mut(g) {
        c.fields.set_f32(UNIT_FIELD_BOUNDINGRADIUS, r);
    }
}

/// Thistle Tea's "opens a nearby live friendly banker", "rejects a
/// non-mob GUID", "rejects missing, dead, and unflagged metadata",
/// "rejects hostile bankers", "rejects a banker in another world or beyond
/// five yards" and "requires a ready living character", and `NpcReach`'s
/// "reaches five yards past both bodies" and "measures from centers when
/// neither radius is known": a banker shows the bank only alive, flagged,
/// not hostile and in reach, to the living.
#[tokio::test]
async fn a_banker_serves_the_living_within_five_yards_past_both_bodies() {
    let (_realm, mut a, ga, map, c) = clerk(CLERK, 3.0).await;
    assert!(shown_bank(&mut a, c).await, "a friendly banker near");
    assert!(!shown_bank(&mut a, ga).await, "a player is no banker");
    on(&map, move |m| {
        m.creature_mut(c).unwrap().fields.set_u32(UNIT_NPC_FLAGS, 0);
    })
    .await;
    assert!(!shown_bank(&mut a, c).await, "not a banker now");
    on(&map, move |m| {
        let later = m.now + std::time::Duration::from_secs(60);
        let c = m.creature_mut(c).unwrap();
        c.fields.set_u32(UNIT_NPC_FLAGS, npc::NPC_FLAG_BANKER);
        c.life = Life::Dead {
            corpse_until: later,
            respawn_at: later,
        };
    })
    .await;
    assert!(!shown_bank(&mut a, c).await, "a dead banker");
    on(&map, move |m| m.creature_mut(c).unwrap().life = Life::Alive).await;
    // Five yards from centers, with no bodies; then past both.
    for (mine, its, yards, served) in [
        (0.0, 0.0, 4.95, true),
        (0.0, 0.0, 5.05, false),
        (0.5, 1.0, 6.45, true),
        (0.5, 1.0, 6.55, false),
    ] {
        on(&map, move |m| {
            radius(m, ga, mine);
            radius(m, c, its);
            place(m, ga, c, yards);
        })
        .await;
        assert_eq!(shown_bank(&mut a, c).await, served, "{mine} {its} {yards}");
    }
    on(&map, move |m| place(m, ga, c, 3.0)).await;
    on(&map, move |m| crate::game::map::death::kill(m, ga)).await;
    assert!(!shown_bank(&mut a, c).await, "not to the dead");
    let (_realm, mut a, _, _, enemy) = clerk(ENEMY_CLERK, 3.0).await;
    assert!(!shown_bank(&mut a, enemy).await, "not a hostile banker");
}

/// `CMSG_BUY_ITEM` of `amount` of [`WARES`] from `vendor`: the refusal.
async fn buy(a: &mut Client, vendor: Guid, amount: u8) -> Option<BuyResult> {
    a.clear();
    a.send(CMSG_BUY_ITEM {
        vendor,
        item: WARES,
        amount,
        unknown1: 0,
    })
    .await;
    handled(a).await;
    a.gets(std::time::Duration::from_millis(1), |m| match m {
        ServerOpcodeMessage::SMSG_BUY_FAILED(f) => Some(f.result),
        _ => None,
    })
    .await
}

fn left(m: &mut Map, c: Guid) -> u32 {
    let v = m.realm.data.vendor_items(CLERK)[0];
    vendor::stock_left(m, c, &v, 3)
}

/// Thistle Tea's "money and capacity failures leave stock and inventory
/// untouched" and `valid_vendor?`'s distance: short of money, the buyer is
/// told so; with no room, told by the inventory failure mangos's
/// `BuyItemFromVendor` sends; either way the stock, money and bags are
/// as they were. A vendor out of reach is too far; the dead are told
/// nothing.
#[tokio::test]
async fn a_refused_purchase_leaves_stock_money_and_bags_alone() {
    let (_realm, mut a, ga, map, c) = clerk(CLERK, 3.0).await;
    on(&map, move |m| m.players.get_mut(&ga).unwrap().set_money(10)).await;
    assert_eq!(buy(&mut a, c, 1).await, Some(BuyResult::NotEnoughtMoney));
    let (stock, money, wares) = on(&map, move |m| {
        (
            left(m, c),
            m.players[&ga].money(),
            m.players[&ga].count_of(WARES),
        )
    })
    .await;
    assert_eq!((stock, money, wares), (10, 10, 0));
    on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().set_money(1000);
        let d = m.realm.data.clone();
        let p = &m.players[&ga];
        let free = p
            .carry_slots(&d)
            .iter()
            .filter(|(b, s)| p.item_at(*b, *s).is_none())
            .count() as u32;
        crate::game::map::inventory::add_item(m, ga, FILLER, free, false).unwrap();
    })
    .await;
    a.clear();
    a.send(CMSG_BUY_ITEM {
        vendor: c,
        item: WARES,
        amount: 1,
        unknown1: 0,
    })
    .await;
    a.wait_for("the inventory failure", |m| match m {
        ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(_) => Some(()),
        _ => None,
    })
    .await;
    let (stock, money, wares) = on(&map, move |m| {
        (
            left(m, c),
            m.players[&ga].money(),
            m.players[&ga].count_of(WARES),
        )
    })
    .await;
    assert_eq!((stock, money, wares), (10, 1000, 0));
    on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.items.retain(|i| i.entry != FILLER);
        place(m, ga, c, 7.0);
    })
    .await;
    assert_eq!(buy(&mut a, c, 1).await, Some(BuyResult::DistanceTooFar));
    on(&map, move |m| {
        place(m, ga, c, 3.0);
        crate::game::map::death::kill(m, ga);
    })
    .await;
    assert_eq!(buy(&mut a, c, 1).await, None, "the dead are told nothing");
    let wares = on(&map, move |m| m.players[&ga].count_of(WARES)).await;
    assert_eq!(wares, 0);
}

/// Thistle Tea's "dispatches an identity-preserving sale and repurchase":
/// an item sold and bought back is the same item, the money back as it
/// was; a second buyback of the emptied slot can't find the item.
#[tokio::test]
async fn an_item_bought_back_is_the_one_sold_and_an_empty_slot_has_nothing() {
    let (_realm, mut a, ga, map, c) = clerk(CLERK, 3.0).await;
    let item = on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().set_money(1000);
        crate::game::map::inventory::add_item(m, ga, FILLER, 1, false).unwrap();
        let p = &m.players[&ga];
        p.items.iter().find(|i| i.entry == FILLER).unwrap().guid
    })
    .await;
    a.send(CMSG_SELL_ITEM {
        vendor: c,
        item,
        amount: 0,
    })
    .await;
    handled(&mut a).await;
    let sold = on(&map, move |m| {
        let p = &m.players[&ga];
        let i = p.item_by_guid(item).unwrap();
        (p.items[i].slot, p.money())
    })
    .await;
    assert_eq!(sold, (crate::game::map::inventory::BUYBACK_START, 1005));
    let back = CMSG_BUYBACK_ITEM {
        guid: c,
        slot: BuybackSlot::Slot1,
    };
    a.send(back).await;
    handled(&mut a).await;
    let bought = on(&map, move |m| {
        let p = &m.players[&ga];
        let i = p.item_by_guid(item).expect("the same item");
        (p.items[i].carried(), p.money(), p.count_of(FILLER))
    })
    .await;
    assert_eq!(bought, (true, 1000, 1));
    a.clear();
    a.send(back).await;
    let r = a
        .wait_for("SMSG_BUY_FAILED", |m| match m {
            ServerOpcodeMessage::SMSG_BUY_FAILED(f) => Some(f.result),
            _ => None,
        })
        .await;
    assert_eq!(r, BuyResult::CantFindItem);
}
