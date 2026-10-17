//! Phase 3's vendors and wear, Thistle Tea's vendor and durability tests
//! ported: what can be sold, the buyback slots, buying back, limited stock,
//! repairs and their costs, and wear by spells and spirit healers.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::inventory::{BAG_START, BANK_START, BUYBACK_START};
use crate::game::map::{Map, MapKey, vendor};
use crate::game::store::BAG_PLAYER;

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const VENDOR: u32 = 3300;
/// A green sword and a white helm that wear; trinkets one a slot, widgets
/// in stacks; a bag; an item that sells for nothing; a limited item sold in
/// threes and an unlimited one.
const SWORD: u32 = 9300;
const HELM: u32 = 9301;
const TRINKET: u32 = 9302;
const WIDGET: u32 = 9303;
const SATCHEL: u32 = 9304;
const FREEBIE: u32 = 9305;
const LIMITED: u32 = 9306;
const PLENTY: u32 = 9307;
const FILLER: u32 = 9308;
const WEAR: u32 = 900_600;
const WEAR_PCT: u32 = 900_601;

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
            npc_flags: vendor::NPC_FLAG_VENDOR | vendor::NPC_FLAG_REPAIR,
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
    let mut sword = item(SWORD);
    sword.class = 2;
    sword.quality = 2;
    sword.item_level = 10;
    sword.max_durability = 50;
    sword.inventory_type = 13;
    sword.sell_price = 100;
    d.items.insert(SWORD, sword);
    let mut helm = item(HELM);
    helm.class = 4;
    helm.subclass = 1;
    helm.quality = 1;
    helm.item_level = 10;
    helm.max_durability = 50;
    helm.inventory_type = 1;
    d.items.insert(HELM, helm);
    for e in [TRINKET, FILLER] {
        d.items.insert(e, item(e));
    }
    let mut widget = item(WIDGET);
    widget.stackable = 20;
    d.items.insert(WIDGET, widget);
    let mut satchel = item(SATCHEL);
    satchel.class = 1;
    satchel.inventory_type = 18;
    satchel.container_slots = 4;
    d.items.insert(SATCHEL, satchel);
    let mut freebie = item(FREEBIE);
    freebie.sell_price = 0;
    d.items.insert(FREEBIE, freebie);
    let mut limited = item(LIMITED);
    limited.buy_count = 3;
    limited.stackable = 20;
    d.items.insert(LIMITED, limited);
    d.items.insert(PLENTY, item(PLENTY));
    d.vendors.insert(
        VENDOR,
        vec![
            VendorItem {
                slot: 0,
                item: LIMITED,
                max_count: 10,
                restock_secs: 10,
                ..Default::default()
            },
            VendorItem {
                slot: 1,
                item: PLENTY,
                ..Default::default()
            },
        ],
    );
    // Level 10's repair costs: 45 a point for a sword, 49 for cloth; a
    // green's factor 1.25, a white's 1.
    let mut costs = vec![0; 29];
    costs[0] = 45;
    costs[22] = 49;
    d.durability_costs.insert(10, costs);
    d.durability_quality.insert(4, 1.0);
    d.durability_quality.insert(6, 1.25);
    // Wear by points on what is carried (-2), and by a share on the main
    // hand (15).
    d.spells.ranges.insert(1, (0.0, 30.0));
    for (id, effect, misc, points) in [(WEAR, 111, -2, 4), (WEAR_PCT, 115, 15, 49)] {
        let mut s = crate::game::data::spells::SpellTemplate {
            id,
            name: format!("Wear {id}"),
            equipped_item_class: -1,
            range_index: 1,
            ..Default::default()
        };
        s.effects[0] = crate::game::data::spells::SpellEffect {
            effect,
            target_a: 25,
            misc_value: misc,
            base_points: points,
            base_dice: 1,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
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

async fn setup() -> (Arc<Realm>, Client, Guid) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(vendor_guid()).await;
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(1000)
    })
    .await;
    (realm, a, g)
}

/// Give `count` of `entry`; the guid of the last item made or grown.
async fn give(realm: &Arc<Realm>, g: Guid, entry: u32, count: u32) -> Guid {
    on_map(realm, move |m| {
        crate::game::map::inventory::add_item(m, g, entry, count, false).expect("room");
        let p = &m.players[&g];
        p.items
            .iter()
            .rev()
            .find(|i| i.entry == entry)
            .unwrap()
            .guid
    })
    .await
}

/// Put `item` at `(bag, slot)`, as if it were moved there.
async fn put(realm: &Arc<Realm>, g: Guid, item: Guid, at: (u8, u8)) {
    on_map(realm, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_by_guid(item).unwrap();
        (p.items[i].bag, p.items[i].slot) = at;
        p.refresh_inventory(&d);
    })
    .await;
}

async fn fill(realm: &Arc<Realm>, g: Guid) {
    on_map(realm, move |m| {
        let d = m.realm.data.clone();
        let p = &m.players[&g];
        let free = p
            .carry_slots(&d)
            .iter()
            .filter(|(b, s)| p.item_at(*b, *s).is_none())
            .count() as u32;
        crate::game::map::inventory::add_item(m, g, FILLER, free, false).unwrap();
    })
    .await;
}

/// `CMSG_SELL_ITEM`: the refusal, if it is refused.
async fn sell(a: &mut Client, item: Guid, amount: u8) -> Option<SellItemResult> {
    a.clear();
    a.send(CMSG_SELL_ITEM {
        vendor: vendor_guid(),
        item,
        amount,
    })
    .await;
    handled(a).await;
    a.gets(Duration::from_millis(1), |m| match m {
        ServerOpcodeMessage::SMSG_SELL_ITEM(s) if s.item == item => Some(s.result),
        _ => None,
    })
    .await
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

fn money(m: &Map, g: Guid) -> u32 {
    m.players[&g].money()
}

fn at(m: &Map, g: Guid, item: Guid) -> Option<(u8, u8, u32)> {
    let p = &m.players[&g];
    p.item_by_guid(item)
        .map(|i| (p.items[i].bag, p.items[i].slot, p.items[i].count()))
}

/// Thistle Tea's `Buyback.sell/7` refusals: an item not the player's, more
/// than the stack, a banked one, one that sells for nothing, a sale past the
/// most money, and a bag with something in it, its contents kept.
#[tokio::test]
async fn selling_refuses_what_cannot_be_sold() {
    let (realm, mut a, g) = setup().await;
    assert_eq!(
        sell(&mut a, guid::item(999_999), 0).await,
        Some(SellItemResult::CantFindItem)
    );
    let trinket = give(&realm, g, TRINKET, 1).await;
    assert_eq!(
        sell(&mut a, trinket, 2).await,
        Some(SellItemResult::CantSellItem)
    );
    put(&realm, g, trinket, (BAG_PLAYER, BANK_START)).await;
    assert_eq!(
        sell(&mut a, trinket, 0).await,
        Some(SellItemResult::CantSellItem),
        "banked"
    );
    let freebie = give(&realm, g, FREEBIE, 1).await;
    assert_eq!(
        sell(&mut a, freebie, 0).await,
        Some(SellItemResult::CantSellItem)
    );
    let widget = give(&realm, g, WIDGET, 1).await;
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(2_147_483_647)
    })
    .await;
    assert_eq!(
        sell(&mut a, widget, 0).await,
        Some(SellItemResult::CantSellItem),
        "rich"
    );
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(1000)
    })
    .await;
    let satchel = give(&realm, g, SATCHEL, 1).await;
    put(&realm, g, satchel, (BAG_PLAYER, BAG_START)).await;
    put(&realm, g, widget, (BAG_START, 0)).await;
    assert_eq!(
        sell(&mut a, satchel, 0).await,
        Some(SellItemResult::CantSellItem),
        "full"
    );
    let kept = on_map(&realm, move |m| (at(m, g, widget), money(m, g))).await;
    assert_eq!(kept, (Some((BAG_START, 0, 1)), 1000));
}

/// Thistle Tea's buyback slots: a thirteenth sale takes the oldest only; a
/// slot bought back is filled again once the next slot is taken.
#[tokio::test]
async fn the_buyback_slots_fill_in_turn_then_lose_the_oldest() {
    let (realm, mut a, g) = setup().await;
    let mut sold = Vec::new();
    for _ in 0..13 {
        let t = give(&realm, g, TRINKET, 1).await;
        assert_eq!(sell(&mut a, t, 0).await, None, "sold");
        sold.push(t);
    }
    let kept = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (
            p.item_at(BAG_PLAYER, BUYBACK_START)
                .map(|i| p.items[i].guid),
            (BUYBACK_START..BUYBACK_START + 12)
                .filter(|s| p.item_at(BAG_PLAYER, *s).is_some())
                .count(),
            money(m, g),
        )
    })
    .await;
    assert_eq!(kept.0, Some(sold[12]), "the thirteenth in the first slot");
    assert_eq!(kept.1, 12);
    assert_eq!(kept.2, 1000 + 13 * 5);
    let first = sold[0];
    let gone = on_map(&realm, move |m| at(m, g, first)).await;
    assert_eq!(gone, None, "the oldest gone");
    for s in &sold[1..12] {
        let s = *s;
        assert!(on_map(&realm, move |m| at(m, g, s)).await.is_some());
    }
}

/// Thistle Tea's buyback after eleven sales: the first slot bought back,
/// the next sale goes to the twelfth, and the one after to the first again,
/// nothing lost.
#[tokio::test]
async fn a_bought_back_slot_is_filled_again() {
    let (realm, mut a, g) = setup().await;
    let mut sold = Vec::new();
    for _ in 0..11 {
        let t = give(&realm, g, TRINKET, 1).await;
        sell(&mut a, t, 0).await;
        sold.push(t);
    }
    a.send(CMSG_BUYBACK_ITEM {
        guid: vendor_guid(),
        slot: BuybackSlot::Slot1,
    })
    .await;
    handled(&mut a).await;
    let first = sold[0];
    assert_eq!(
        on_map(&realm, move |m| at(m, g, first).map(|(b, s, _)| (b, s)))
            .await
            .map(|(b, _)| b),
        Some(BAG_PLAYER)
    );
    sell(&mut a, first, 0).await;
    let twelfth = on_map(&realm, move |m| at(m, g, first)).await;
    assert_eq!(twelfth, Some((BAG_PLAYER, BUYBACK_START + 11, 1)));
    let next = give(&realm, g, TRINKET, 1).await;
    sell(&mut a, next, 0).await;
    let placed = on_map(&realm, move |m| at(m, g, next)).await;
    assert_eq!(placed, Some((BAG_PLAYER, BUYBACK_START, 1)));
    for s in sold {
        assert!(
            on_map(&realm, move |m| at(m, g, s)).await.is_some(),
            "none lost"
        );
    }
}

/// Thistle Tea's `Buyback.restore/4`: no money or no room keeps the sale
/// as it was; a part of a stack bought back joins its stack, the sold item
/// gone.
#[tokio::test]
async fn a_buyback_wants_money_and_room_and_rejoins_its_stack() {
    let (realm, mut a, g) = setup().await;
    let helm = give(&realm, g, HELM, 1).await;
    sell(&mut a, helm, 0).await;
    on_map(&realm, move |m| m.players.get_mut(&g).unwrap().set_money(4)).await;
    a.send(CMSG_BUYBACK_ITEM {
        guid: vendor_guid(),
        slot: BuybackSlot::Slot1,
    })
    .await;
    let why = a
        .wait_for("SMSG_BUY_FAILED", |m| match m {
            ServerOpcodeMessage::SMSG_BUY_FAILED(f) => Some(f.result),
            _ => None,
        })
        .await;
    assert_eq!(why, BuyResult::NotEnoughtMoney);
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(1000)
    })
    .await;
    fill(&realm, g).await;
    a.send(CMSG_BUYBACK_ITEM {
        guid: vendor_guid(),
        slot: BuybackSlot::Slot1,
    })
    .await;
    a.wait_for("SMSG_INVENTORY_CHANGE_FAILURE", |m| match m {
        ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(_) => Some(()),
        _ => None,
    })
    .await;
    let kept = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (
            at(m, g, helm),
            p.fields.get_u32(PLAYER_FIELD_BUYBACK_PRICE_1),
            money(m, g),
        )
    })
    .await;
    assert_eq!(kept, (Some((BAG_PLAYER, BUYBACK_START, 1)), 5, 1000));
    // Two of five widgets sold and bought back, with the bags full: they
    // join the three.
    let widgets = on_map(&realm, move |m| {
        let i = m.players[&g]
            .items
            .iter()
            .position(|i| i.entry == FILLER)
            .unwrap();
        crate::game::map::inventory::take_item(m, g, i, 1);
        crate::game::map::inventory::add_item(m, g, WIDGET, 5, false).unwrap();
        m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == WIDGET)
            .unwrap()
            .guid
    })
    .await;
    sell(&mut a, widgets, 2).await;
    let split = on_map(&realm, move |m| {
        let p = &m.players[&g];
        p.item_at(BAG_PLAYER, BUYBACK_START + 1)
            .map(|i| (p.items[i].guid, p.items[i].count()))
    })
    .await
    .unwrap();
    assert_eq!(split.1, 2);
    a.send(CMSG_BUYBACK_ITEM {
        guid: vendor_guid(),
        slot: BuybackSlot::Slot2,
    })
    .await;
    handled(&mut a).await;
    let joined = on_map(&realm, move |m| {
        (
            at(m, g, widgets).map(|(_, _, n)| n),
            at(m, g, split.0),
            m.players[&g].count_of(WIDGET),
        )
    })
    .await;
    assert_eq!(joined, (Some(5), None, 5));
}

async fn bought(a: &mut Client) -> u32 {
    a.wait_for("SMSG_BUY_ITEM", |m| match m {
        ServerOpcodeMessage::SMSG_BUY_ITEM(b) => Some(b.amount_for_sale),
        _ => None,
    })
    .await
}

/// Thistle Tea's `VendorStock`: each purchase starts the restock over, a
/// buy count back an interval after the last; an unlimited item keeps no
/// stock.
#[tokio::test]
async fn limited_stock_comes_back_after_the_last_purchase() {
    let (realm, mut a, g) = setup().await;
    // On the map's clock, in one go.
    let stock = on_map(&realm, move |m| {
        let t0 = m.now;
        let v = m.realm.data.vendor_items(VENDOR)[0].clone();
        let left = |m: &mut Map| vendor::stock_left(m, vendor_guid(), &v, 3);
        vendor::buy(m, g, vendor_guid(), LIMITED, 1);
        let first = left(m);
        m.now = t0 + Duration::from_secs(9);
        let soon = left(m);
        vendor::buy(m, g, vendor_guid(), LIMITED, 1);
        let second = left(m);
        m.now = t0 + Duration::from_millis(18_999);
        let not_yet = left(m);
        m.now = t0 + Duration::from_secs(19);
        [first, soon, second, not_yet, left(m)]
    })
    .await;
    assert_eq!(
        stock,
        [7, 7, 4, 4, 7],
        "ten seconds after the last purchase"
    );
    a.clear();
    a.send(CMSG_BUY_ITEM {
        vendor: vendor_guid(),
        item: PLENTY,
        amount: 1,
        unknown1: 0,
    })
    .await;
    assert_eq!(bought(&mut a).await, u32::MAX);
    let kept = on_map(&realm, move |m| {
        m.vendor_stock.contains_key(&(vendor_guid(), PLENTY))
    })
    .await;
    assert!(!kept);
}

/// Thistle Tea's `restock_delay/3`: varied and shortened by its flags, each
/// stage truncated to a whole second.
#[test]
fn the_restock_delay_truncates_each_stage() {
    let v = |flags, restock_secs| VendorItem {
        restock_secs,
        flags,
        ..Default::default()
    };
    assert_eq!(vendor::restock_secs(&v(0, 61), 80, 5_000), 61);
    assert_eq!(vendor::restock_secs(&v(1, 61), 80, 5_000), 48);
    assert_eq!(vendor::restock_secs(&v(1, 61), 120, 5_000), 73);
    assert_eq!(vendor::restock_secs(&v(2, 61), 120, 5_000), 30);
    assert_eq!(vendor::restock_secs(&v(3, 61), 120, 5_000), 36);
    assert_eq!(vendor::restock_secs(&v(3, 61), 100, 2_500), 61);
    assert_eq!(vendor::restock_secs(&v(0, 0), 100, 0), 1);
}

/// Thistle Tea's `Durability.repair_cost/4`: the points times the item
/// level's cost and the quality's factor, rounded as the client rounds,
/// then the single-precision discount, a half copper to the even one.
#[test]
fn a_repair_costs_what_the_client_says() {
    let d = data();
    let sword = d.item(SWORD).unwrap().clone();
    let mut white = sword.clone();
    white.quality = 1;
    let cost = |t: &ItemTemplate, lost, discount| vendor::repair_cost(&d, t, lost, discount);
    assert_eq!(cost(&white, 1, 0.9), Some(40), "40.5 to the even 40");
    assert_eq!(cost(&sword, 1, 0.9), Some(50));
    assert_eq!(cost(&sword, 2, 0.9), Some(102), "112.5 up to 113 first");
    assert_eq!(cost(&sword, 3, 1.0), Some(169));
    for (lost, expected) in [(1, 40), (3, 122), (15, 608), (17, 688)] {
        assert_eq!(cost(&white, lost, 0.9), Some(expected), "{lost} points");
    }
    assert_eq!(cost(&sword, 0, 1.0), Some(0));
}

/// Thistle Tea's `Durability.lose/3`: points off, clamped, a negative
/// number mending to the most; a percentage rounded down but a point at
/// least, a negative one nothing; what doesn't wear as it was.
#[test]
fn wear_by_points_and_by_share() {
    use crate::game::map::vendor::worn;
    assert_eq!(worn(50, 47, -2, false), 49);
    assert_eq!(worn(50, 47, -50_000, false), 50);
    assert_eq!(worn(50, 47, -100, true), 47);
    assert_eq!(worn(0, 0, -50_000, false), 0);
    assert_eq!(worn(50, 47, 1000, false), 0);
    assert_eq!(worn(50, 47, 1, true), 46);
    assert_eq!(worn(50, 47, 0, true), 47);
    assert_eq!(worn(0, 0, 1, false), 0);
}

fn durability(m: &Map, g: Guid, item: Guid) -> u32 {
    let p = &m.players[&g];
    p.items[p.item_by_guid(item).unwrap()]
        .fields
        .get_u32(ITEM_FIELD_DURABILITY)
}

/// A sword worn, a helm in a bag and a sword in the bank, each worn down.
async fn worn_gear(realm: &Arc<Realm>, g: Guid) -> [Guid; 3] {
    let worn = give(realm, g, SWORD, 1).await;
    put(realm, g, worn, (BAG_PLAYER, 15)).await;
    let satchel = give(realm, g, SATCHEL, 1).await;
    put(realm, g, satchel, (BAG_PLAYER, BAG_START)).await;
    let helm = give(realm, g, HELM, 1).await;
    put(realm, g, helm, (BAG_START, 0)).await;
    let banked = give(realm, g, SWORD, 1).await;
    put(realm, g, banked, (BAG_PLAYER, BANK_START)).await;
    on_map(realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        for (item, now) in [(worn, 47), (helm, 49), (banked, 49)] {
            let i = p.item_by_guid(item).unwrap();
            p.items[i].fields.set_u32(ITEM_FIELD_DURABILITY, now);
        }
    })
    .await;
    [worn, helm, banked]
}

/// Thistle Tea's `Durability.repair/4`: one item mended alone and paid for
/// alone; one in the bank or not the player's, nothing; everything worn and
/// carried mended for one payment, the bank's not.
#[tokio::test]
async fn repairs_mend_what_is_worn_and_carried() {
    let (realm, mut a, g) = setup().await;
    let [worn, helm, banked] = worn_gear(&realm, g).await;
    let repair = |item| CMSG_REPAIR_ITEM {
        npc: vendor_guid(),
        item,
    };
    a.send(repair(helm)).await;
    handled(&mut a).await;
    let one = on_map(&realm, move |m| {
        (money(m, g), durability(m, g, helm), durability(m, g, worn))
    })
    .await;
    assert_eq!(one, (1000 - 49, 50, 47), "the helm, 49");
    for item in [banked, guid::item(999_998)] {
        a.send(repair(item)).await;
    }
    handled(&mut a).await;
    let none = on_map(&realm, move |m| (money(m, g), durability(m, g, banked))).await;
    assert_eq!(none, (1000 - 49, 49));
    a.send(repair(Guid::zero())).await;
    handled(&mut a).await;
    let all = on_map(&realm, move |m| {
        (
            money(m, g),
            durability(m, g, worn),
            durability(m, g, banked),
        )
    })
    .await;
    assert_eq!(
        all,
        (1000 - 49 - 169, 50, 49),
        "the sword, 169, the bank's not"
    );
}

/// Thistle Tea's `Durability.loss/5`: a spell's wear on what is carried
/// (-2) reaches the bags but not the bank; one on a slot (15), by a share,
/// only that slot; a spirit healer's a quarter off what is worn and
/// carried, not the bank.
#[tokio::test]
async fn wear_reaches_what_its_scope_says() {
    let (realm, _a, g) = setup().await;
    let [worn, helm, banked] = worn_gear(&realm, g).await;
    let after = on_map(&realm, move |m| {
        crate::game::map::spells::cast_triggered(m, g, WEAR, Some(g));
        let points = [worn, helm, banked].map(|i| durability(m, g, i));
        crate::game::map::spells::cast_triggered(m, g, WEAR_PCT, Some(g));
        let share = [worn, helm, banked].map(|i| durability(m, g, i));
        (points, share)
    })
    .await;
    assert_eq!(after.0, [42, 44, 49], "five off the worn and carried");
    assert_eq!(after.1, [17, 44, 49], "half the most off the main hand");
    let healer = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        for item in [worn, helm, banked] {
            let i = p.item_by_guid(item).unwrap();
            p.items[i].fields.set_u32(ITEM_FIELD_DURABILITY, 50);
        }
        vendor::wear_carried(m, g, 0.25);
        [worn, helm, banked].map(|i| durability(m, g, i))
    })
    .await;
    assert_eq!(healer, [38, 38, 50]);
}
