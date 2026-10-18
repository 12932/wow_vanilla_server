//! The fourth code review's findings (the world's systems: the social
//! services, inventory, loot, quests, vendors, objects, scripts and PvP),
//! each shown on made-up data in the fast suite.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::inventory::{BUYBACK_START, PACK_START};
use crate::game::map::{Map, MapKey, vendor};
use crate::game::store::BAG_PLAYER;

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const VENDOR: u32 = 990_400;
const WIDGET: u32 = 990_401;

fn item(entry: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000,
        stackable: 1,
        allowable_class: -1,
        allowable_race: -1,
        buy_count: 1,
        buy_price: 10,
        sell_price: 5,
        ..Default::default()
    }
}

/// A vendor beside the start, and widgets in stacks of twenty.
fn data() -> GameData {
    let mut d = GameData::empty();
    d.creature_templates.insert(
        VENDOR,
        CreatureTemplate {
            entry: VENDOR,
            name: "Vendor".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            display_scales: [1.0, 0.0, 0.0, 0.0],
            level_min: 2,
            level_max: 2,
            faction: 35,
            unit_class: 1,
            health_max: 50,
            npc_flags: vendor::NPC_FLAG_VENDOR,
            ..Default::default()
        },
    );
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: 1,
            entries: vec![VENDOR],
            map: 0,
            x: START.0 + 2.0,
            y: START.1,
            z: START.2,
            ..Default::default()
        }],
    );
    let mut widget = item(WIDGET);
    widget.stackable = 20;
    d.items.insert(WIDGET, widget);
    d.vendors.insert(
        VENDOR,
        vec![VendorItem {
            slot: 0,
            item: WIDGET,
            ..Default::default()
        }],
    );
    d
}

async fn on_map<T: Send + 'static>(
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

fn vendor_guid() -> Guid {
    guid::unit(VENDOR, 1)
}

/// Wait until `a`'s earlier packets have been handled.
async fn handled(a: &mut Client) {
    a.send(CMSG_NAME_QUERY { guid: a.guid }).await;
    a.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
}

/// Where `item` is, if `g` has it.
fn place(m: &Map, g: Guid, item: Guid) -> Option<(u8, u8, u32)> {
    let p = &m.players[&g];
    p.item_by_guid(item)
        .map(|i| (p.items[i].bag, p.items[i].slot, p.items[i].count()))
}

fn carried_widgets(m: &Map, g: Guid) -> u32 {
    m.players[&g].carried_count_of(WIDGET)
}

/// R4-1. A sold item waits in a buyback slot for its price back; moving,
/// splitting, storing or mailing it from there is refused (mangos's
/// `IsValidPos` on the source), rather than taking it back for nothing
/// with the sale's money kept.
#[tokio::test]
async fn a_sold_item_leaves_its_buyback_slot_only_bought_back() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "R4BB");
    let g = a.enter("Seller", Race::Human, Class::Warrior).await;
    a.sees(vendor_guid()).await;
    let widgets = on_map(&realm, move |m| {
        crate::game::map::inventory::add_item(m, g, WIDGET, 10, false).unwrap();
        let p = &m.players[&g];
        p.items.iter().find(|i| i.entry == WIDGET).unwrap().guid
    })
    .await;
    a.send(CMSG_SELL_ITEM {
        vendor: vendor_guid(),
        item: widgets,
        amount: 0,
    })
    .await;
    handled(&mut a).await;
    let (sold_at, money) = on_map(&realm, move |m| {
        (place(m, g, widgets), m.players[&g].money())
    })
    .await;
    let slot = BUYBACK_START;
    assert_eq!(sold_at, Some((BAG_PLAYER, slot, 10)), "sold, in buyback");
    assert_eq!(money, 50, "paid for ten");
    let free = PACK_START + 5;
    a.send(CMSG_SWAP_ITEM {
        destination_bag: BAG_PLAYER,
        destionation_slot: free,
        source_bag: BAG_PLAYER,
        source_slot: slot,
    })
    .await;
    a.send(CMSG_SPLIT_ITEM {
        source_bag: BAG_PLAYER,
        source_slot: slot,
        destination_bag: BAG_PLAYER,
        destination_slot: free,
        amount: 4,
    })
    .await;
    a.send(CMSG_AUTOSTORE_BAG_ITEM {
        source_bag: BAG_PLAYER,
        source_slot: slot,
        destination_bag: BAG_PLAYER,
    })
    .await;
    a.send(CMSG_AUTOEQUIP_ITEM {
        source_bag: BAG_PLAYER,
        source_slot: slot,
    })
    .await;
    // Nor mailed to another, the sale's money kept.
    let mut b = Client::connect(&realm, "R4BB2");
    b.enter("Mailee", Race::Human, Class::Warrior).await;
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(1000)
    })
    .await;
    a.send(CMSG_SEND_MAIL {
        mailbox: Guid::zero(),
        receiver: "Mailee".into(),
        subject: "Sold".into(),
        body: String::new(),
        unknown1: 0,
        unknown2: 0,
        item: widgets,
        money: Gold::new(0),
        cash_on_delivery_amount: 0,
        unknown3: 0,
        unknown4: 0,
    })
    .await;
    handled(&mut a).await;
    let (now_at, carried) = on_map(&realm, move |m| {
        (place(m, g, widgets), carried_widgets(m, g))
    })
    .await;
    assert_eq!(now_at, Some((BAG_PLAYER, slot, 10)), "still sold");
    assert_eq!(carried, 0, "none of it back for nothing");
}

/// R4-2. Items added onto a stack the client knows show it grown: the
/// stack's new count goes out with the next update, rather than its dirty
/// bit cleared with the new items'.
#[tokio::test]
async fn a_stack_added_onto_shows_its_new_count() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "R4ST");
    let g = a.enter("Stacker", Race::Human, Class::Warrior).await;
    a.sees(vendor_guid()).await;
    let stack = on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(1000);
        crate::game::map::inventory::add_item(m, g, WIDGET, 5, false).unwrap();
        let p = &m.players[&g];
        p.items.iter().find(|i| i.entry == WIDGET).unwrap().guid
    })
    .await;
    a.sees(stack).await;
    a.clear();
    a.send(CMSG_BUY_ITEM {
        vendor: vendor_guid(),
        item: WIDGET,
        amount: 1,
        unknown1: 0,
    })
    .await;
    let count = a
        .gets(Duration::from_secs(2), |m| {
            harness::values(m)
                .into_iter()
                .find_map(|(o, mask)| match mask {
                    UpdateMask::Item(i) if o == stack => i.item_stack_count(),
                    _ => None,
                })
        })
        .await;
    let held = on_map(&realm, move |m| place(m, g, stack)).await;
    assert_eq!(held.map(|h| h.2), Some(6), "the server's stack grew");
    assert_eq!(count, Some(6), "and the client is told");
}

const SATCHEL: u32 = 990_402;

/// R4-3. A bag in the bank's bag slots with something in it isn't
/// destroyed (mangos's `IsNotEmptyBag`), as one on the bag bar isn't,
/// rather than leaving what it held in a bag that is gone.
#[tokio::test]
async fn a_full_bank_bag_is_not_destroyed() {
    use crate::game::map::inventory::BANK_BAG_START;
    let realm = harness::realm_with({
        let mut d = data();
        let mut satchel = item(SATCHEL);
        satchel.class = 1;
        satchel.inventory_type = 18;
        satchel.container_slots = 4;
        d.items.insert(SATCHEL, satchel);
        d
    });
    let mut a = Client::connect(&realm, "R4BG");
    let g = a.enter("Banker", Race::Human, Class::Warrior).await;
    let (bag, widget) = on_map(&realm, move |m| {
        crate::game::map::inventory::add_item(m, g, SATCHEL, 1, false).unwrap();
        crate::game::map::inventory::add_item(m, g, WIDGET, 1, false).unwrap();
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.fields.set_byte(crate::game::fields::PLAYER_BYTES_2, 2, 1);
        let b = p.items.iter().position(|i| i.entry == SATCHEL).unwrap();
        (p.items[b].bag, p.items[b].slot) = (BAG_PLAYER, BANK_BAG_START);
        let w = p.items.iter().position(|i| i.entry == WIDGET).unwrap();
        (p.items[w].bag, p.items[w].slot) = (BANK_BAG_START, 0);
        p.refresh_inventory(&d);
        (p.items[b].guid, p.items[w].guid)
    })
    .await;
    a.send(CMSG_DESTROYITEM {
        bag: BAG_PLAYER,
        slot: BANK_BAG_START,
        amount: 0,
        unknown1: 0,
        unknown2: 0,
        unknown3: 0,
    })
    .await;
    handled(&mut a).await;
    let (b, w) = on_map(&realm, move |m| (place(m, g, bag), place(m, g, widget))).await;
    assert_eq!(
        b.map(|b| (b.0, b.1)),
        Some((BAG_PLAYER, BANK_BAG_START)),
        "the bag stays"
    );
    assert_eq!(
        w.map(|w| (w.0, w.1)),
        Some((BANK_BAG_START, 0)),
        "with what it holds"
    );
}

/// R4-4. Only a window of one's own closes with `CMSG_LOOT_RELEASE`:
/// another player's release of a creature a rogue is picking leaves the
/// rogue's pocket loot where it is, rather than filing it as the other's.
#[tokio::test]
async fn another_players_release_leaves_a_rogues_pockets() {
    use crate::game::map::loot::{Loot, LootKind};
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "R4PP");
    let rogue = a.enter("Picker", Race::Human, Class::Rogue).await;
    let mut b = Client::connect(&realm, "R4PP2");
    let other = b.enter("Closer", Race::Human, Class::Warrior).await;
    let target = vendor_guid();
    a.sees(target).await;
    on_map(&realm, move |m| {
        let c = m.creature_mut(target).unwrap();
        c.loot = Some(Loot {
            gold: 10,
            owner: Some(rogue),
            kind: LootKind::Pickpocket,
            ..Default::default()
        });
        crate::game::map::loot::show(m, rogue, target);
    })
    .await;
    b.send(CMSG_LOOT_RELEASE { guid: target }).await;
    handled(&mut b).await;
    let (left, filed, still) = on_map(&realm, move |m| {
        (
            m.creature(target)
                .and_then(|c| c.loot.as_ref())
                .map(|l| l.gold),
            m.loot_state.pockets.contains_key(&(target, other)),
            m.players[&rogue].looting,
        )
    })
    .await;
    assert_eq!(left, Some(10), "the rogue's pockets are still open");
    assert!(!filed, "and not the closer's to find");
    assert_eq!(still, Some(target), "the rogue still looks in them");
}

/// R4-5. A player leaving the map (a disconnect, a map change) with a loot
/// window open has it closed, as a logout does: an item's loot goes with
/// it, rather than staying on the map for good.
#[tokio::test]
async fn a_loot_window_closes_with_its_looter_leaving() {
    use crate::game::map::loot::Loot;
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "R4LW");
    let g = a.enter("Leaver", Race::Human, Class::Warrior).await;
    let item = guid::item(987_654);
    on_map(&realm, move |m| {
        crate::game::map::loot::show_item_loot(
            m,
            g,
            item,
            Loot {
                gold: 5,
                owner: Some(g),
                ..Default::default()
            },
        );
        assert_eq!(m.players[&g].looting, Some(item));
        // Gone from the map, as a disconnect takes them.
        let _ = m.take_player(g);
    })
    .await;
    let left = on_map(&realm, move |m| m.loot_state.item_loot.contains_key(&item)).await;
    assert!(!left, "the item's loot went with the window");
}

/// R4-6. A script that starts itself at once (a relay to its own id with
/// no delay, a bad row) stops at a depth, rather than running until the
/// stack overflows and the server aborts.
#[tokio::test]
async fn a_script_starting_itself_at_once_stops() {
    use crate::game::data::scripts::{ScriptStep, ScriptTable};
    const LOOP: u32 = 990_450;
    let realm = harness::realm_with({
        let mut d = data();
        d.scripts.generic.insert(
            LOOP,
            vec![ScriptStep {
                command: 39,
                datalong: [LOOP, 0, 0, 0],
                ..Default::default()
            }],
        );
        d
    });
    let mut a = Client::connect(&realm, "R4SC");
    let g = a.enter("Looper", Race::Human, Class::Warrior).await;
    let left = on_map(&realm, move |m| {
        crate::game::map::scripts::start(m, ScriptTable::Generic, LOOP, g, g);
        m.scripts.runs.len()
    })
    .await;
    assert_eq!(left, 0, "it ran to its end, and was swept");
}

fn mails_to(realm: &Realm, counter: u32) -> Vec<crate::game::social::mail::Mail> {
    realm
        .social
        .mail
        .lock()
        .mails
        .values()
        .filter(|m| m.receiver == counter)
        .cloned()
        .collect()
}

/// R4-7. The house's cut of a big bid is worked out wide: 30,000 gold won
/// at the goblins' house (15%) pays the seller 25,500, rather than the cut
/// overflowing a `u32` (a panic in a debug build, a cut of almost nothing
/// in a release one).
#[test]
fn a_big_bids_cut_doesnt_overflow() {
    use crate::game::social::auction::{self, Auction};
    let realm = harness::realm_with(GameData::empty());
    realm.social.auctions.lock().auctions.insert(
        7,
        Auction {
            id: 7,
            market: 7,
            house: 7,
            item: crate::game::store::ItemRow {
                guid: 1,
                entry: WIDGET,
                count: 1,
                ..Default::default()
            },
            owner: 1,
            start_bid: 1,
            bid: 300_000_000,
            bidder: 2,
            expires_at: 0,
            ..Default::default()
        },
    );
    auction::sweep(&realm);
    let paid: Vec<u32> = mails_to(&realm, 1).iter().map(|m| m.money).collect();
    assert_eq!(paid, vec![255_000_000], "the bid less 15%");
}

/// R4-8. A seller deleted with a bid on their auction: the bid is mailed
/// back to its bidder, as a cancel mails it, rather than lost with the
/// auction.
#[test]
fn a_deleted_sellers_bidder_is_paid_back() {
    use crate::game::social::auction::{self, Auction};
    let realm = harness::realm_with(GameData::empty());
    realm.social.auctions.lock().auctions.insert(
        8,
        Auction {
            id: 8,
            market: 2,
            house: 2,
            owner: 1,
            start_bid: 1,
            bid: 500,
            bidder: 2,
            expires_at: i64::MAX,
            ..Default::default()
        },
    );
    auction::character_deleted(&realm, 1);
    let refund: Vec<u32> = mails_to(&realm, 2).iter().map(|m| m.money).collect();
    assert!(
        realm.social.auctions.lock().auctions.is_empty(),
        "the auction went"
    );
    assert_eq!(refund, vec![500], "and the bid went back");
}
