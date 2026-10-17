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
/// One a player, and three a player in stacks.
const RING: u32 = 9309;
const TOKEN: u32 = 9310;
/// A helm with stamina and a passive, and a shield.
const VISOR: u32 = 9311;
const SHIELD: u32 = 9312;
const GUARD: u32 = 900_603;
const WEAR: u32 = 900_600;
const MAKE: u32 = 900_602;
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
    let mut ring = item(RING);
    ring.max_count = 1;
    d.items.insert(RING, ring);
    let mut token = item(TOKEN);
    token.max_count = 3;
    token.stackable = 20;
    d.items.insert(TOKEN, token);
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
    // Repair costs: 45 a point for a sword, 49 for cloth; a
    // green's factor 1.25, a white's 1.
    let mut costs = vec![0; 29];
    costs[0] = 45;
    costs[22] = 49;
    // The same at every level, for the starting gear's sake.
    for level in 0..=70 {
        d.durability_costs.insert(level, costs.clone());
    }
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
    let mut visor = item(VISOR);
    visor.class = 4;
    visor.subclass = 1;
    visor.item_level = 10;
    visor.max_durability = 50;
    visor.inventory_type = 1;
    visor.armor = 30;
    visor.stats[0] = (7, 5);
    visor.spells[0] = crate::game::data::items::ItemSpell {
        id: GUARD,
        trigger: crate::game::map::inventory::ITEM_SPELL_ON_EQUIP,
        ..Default::default()
    };
    d.items.insert(VISOR, visor);
    let mut shield = item(SHIELD);
    shield.class = 4;
    shield.subclass = 6;
    shield.item_level = 10;
    shield.max_durability = 50;
    shield.inventory_type = 14;
    shield.armor = 50;
    shield.block = 10;
    d.items.insert(SHIELD, shield);
    let mut guard = crate::game::data::spells::SpellTemplate {
        id: GUARD,
        name: "Guard".into(),
        equipped_item_class: -1,
        attributes: [0x40, 0, 0, 0, 0],
        ..Default::default()
    };
    guard.effects[0] = crate::game::data::spells::SpellEffect {
        effect: 6,
        aura: 30,
        misc_value: 95,
        target_a: 1,
        base_points: 4,
        base_dice: 1,
        ..Default::default()
    };
    d.spells.spells.insert(GUARD, guard);
    // Five tokens made.
    let mut make = crate::game::data::spells::SpellTemplate {
        id: MAKE,
        name: "Make Tokens".into(),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    make.effects[0] = crate::game::data::spells::SpellEffect {
        effect: 24,
        target_a: 1,
        item_type: u64::from(TOKEN),
        base_points: 4,
        base_dice: 1,
        ..Default::default()
    };
    d.spells.spells.insert(MAKE, make);
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

fn stacks(m: &Map, g: Guid, entry: u32) -> Vec<u32> {
    let mut out: Vec<u32> = m.players[&g]
        .items
        .iter()
        .filter(|i| i.entry == entry)
        .map(|i| i.count())
        .collect();
    out.sort_unstable_by(|a, b| b.cmp(a));
    out
}

/// Thistle Tea's `Items.store/3` and `create/3`: one of an item a slot that
/// doesn't stack, a large grant in legal stacks, all or nothing; a unique
/// item had in the bank refused, and given once when it isn't had; a spell
/// making more than the allowance makes up to it.
#[tokio::test]
async fn a_grant_fits_whole_or_not_at_all() {
    use crate::game::map::inventory::{Fail, add_item};
    let (realm, _a, g) = setup().await;
    let first = on_map(&realm, move |m| {
        let swords = add_item(m, g, SWORD, 2, false);
        let widgets = add_item(m, g, WIDGET, 45, false);
        (swords, stacks(m, g, SWORD), widgets, stacks(m, g, WIDGET))
    })
    .await;
    assert_eq!(first, (Ok(()), vec![1, 1], Ok(()), vec![20, 20, 5]));
    // One slot left: 45 more don't go, and nothing of them does.
    let free = on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = &m.players[&g];
        p.carry_slots(&d)
            .iter()
            .filter(|(b, s)| p.item_at(*b, *s).is_none())
            .count() as u32
    })
    .await;
    let full = on_map(&realm, move |m| {
        add_item(m, g, FILLER, free - 1, false).unwrap();
        let more = add_item(m, g, WIDGET, 45, false);
        (more, stacks(m, g, WIDGET))
    })
    .await;
    assert_eq!(full, (Err(Fail::InventoryFull), vec![20, 20, 5]));
    // A ring in the bank: no second.
    let ring = on_map(&realm, move |m| {
        add_item(m, g, RING, 1, false).unwrap();
        let p = m.players.get_mut(&g).unwrap();
        let i = p.items.iter().position(|i| i.entry == RING).unwrap();
        p.items[i].slot = BANK_START;
        let banked = add_item(m, g, RING, 1, false);
        let p = &m.players[&g];
        let i = p.items.iter().position(|i| i.entry == RING).unwrap();
        crate::game::map::inventory::take_item(m, g, i, 1);
        let again = add_item(m, g, RING, 1, false);
        let twice = add_item(m, g, RING, 1, false);
        (banked, again, twice, stacks(m, g, RING))
    })
    .await;
    assert_eq!(
        ring,
        (
            Err(Fail::CantCarryMore),
            Ok(()),
            Err(Fail::CantCarryMore),
            vec![1]
        )
    );
    // A token had, of three: five made are two.
    let made = on_map(&realm, move |m| {
        let p = &m.players[&g];
        let i = p.items.iter().position(|i| i.entry == FILLER).unwrap();
        crate::game::map::inventory::take_item(m, g, i, 1);
        add_item(m, g, TOKEN, 1, false).unwrap();
        crate::game::map::spells::cast_triggered(m, g, MAKE, Some(g));
        m.players[&g].count_of(TOKEN)
    })
    .await;
    assert_eq!(made, 3);
}

/// What a player shows of their gear: stamina, armor, block, the helm's
/// passive, the helm's look.
fn showing(m: &Map, g: Guid) -> (u32, u32, f32, bool, u32) {
    let p = &m.players[&g];
    (
        p.fields.get_u32(UNIT_FIELD_STAT0 + 2),
        p.fields.get_u32(UNIT_FIELD_RESISTANCES),
        p.fields.get_f32(PLAYER_BLOCK_PERCENTAGE),
        p.auras.has_spell(GUARD),
        p.fields.get_u32(PLAYER_VISIBLE_ITEM_1_0),
    )
}

/// Thistle Tea's broken equipment: a broken helm and shield give no
/// stamina, armor, block or passive, though the helm still shows; mended,
/// they give them again.
#[tokio::test]
async fn broken_gear_gives_nothing_but_its_look() {
    let (realm, a, g) = setup().await;
    // The starting gear has no template here, nor a repair price.
    let bare = on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.retain(|i| d.item(i.entry).is_some());
        p.refresh_inventory(&d);
        showing(m, g)
    })
    .await;
    let visor = give(&realm, g, VISOR, 1).await;
    put(&realm, g, visor, (BAG_PLAYER, 0)).await;
    let shield = give(&realm, g, SHIELD, 1).await;
    put(&realm, g, shield, (BAG_PLAYER, 16)).await;
    let wanted = move |m: &mut Map| {
        let s = showing(m, g);
        s.3.then_some(s)
    };
    let worn = until(&realm, "the guard on", wanted).await;
    assert_eq!(worn.0, bare.0 + 5, "the stamina");
    assert_eq!(worn.1, bare.1 + 80, "the armor");
    assert!(worn.2 > 0.0, "a block");
    assert_eq!(worn.4, VISOR);
    on_map(&realm, move |m| vendor::wear(m, g, 1.0)).await;
    let broken = until(&realm, "the guard off", move |m| {
        let s = showing(m, g);
        (!s.3).then_some(s)
    })
    .await;
    assert_eq!(
        (broken.0, broken.1, broken.2, broken.4),
        (bare.0, bare.1, 0.0, VISOR),
        "nothing but the look"
    );
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(10_000)
    })
    .await;
    a.send(CMSG_REPAIR_ITEM {
        npc: vendor_guid(),
        item: Guid::zero(),
    })
    .await;
    let mended = until(&realm, "the guard back", wanted).await;
    assert_eq!(mended, worn);
}

/// Wait until `f` says so, as the map has it.
async fn until<T: Send + 'static>(
    realm: &Arc<Realm>,
    what: &str,
    f: impl Fn(&mut Map) -> Option<T> + Send + Sync + Clone + 'static,
) -> T {
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        if let Some(t) = on_map(realm, f.clone()).await {
            return t;
        }
        assert!(tokio::time::Instant::now() < deadline, "never {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

const AUCTIONEER: u32 = 3301;
const ROBE: u32 = 9320;
const AXE: u32 = 9321;
const RECIPE: u32 = 9322;
const HIGH: u32 = 9323;
const PLAIN: u32 = 9324;
const COOKED: u32 = 900_610;

/// An auctioneer (the goblins', for want of a faction) by the start, and
/// what is up for auction.
fn auction_data() -> GameData {
    let mut d = data();
    let mut t = d.creature_templates[&VENDOR].clone();
    t.entry = AUCTIONEER;
    t.npc_flags = crate::game::map::npc::NPC_FLAG_AUCTIONEER;
    d.creature_templates.insert(AUCTIONEER, t);
    d.creature_spawns.get_mut(&0).unwrap().push(CreatureSpawn {
        guid: 2,
        entries: vec![AUCTIONEER],
        map: 0,
        x: START.0 + 2.0,
        y: START.1 + 1.0,
        z: START.2,
        ..Default::default()
    });
    let mut robe = item(ROBE);
    (robe.name, robe.class, robe.subclass, robe.inventory_type) = ("Silk Robe".into(), 4, 1, 20);
    robe.quality = 3;
    robe.required_level = 20;
    d.items.insert(ROBE, robe);
    let mut axe = item(AXE);
    (axe.name, axe.class) = ("Axe".into(), 2);
    d.items.insert(AXE, axe);
    let mut recipe = item(RECIPE);
    (recipe.name, recipe.class) = ("Recipe: Cooked Thing".into(), 9);
    recipe.spells[0].id = 483;
    recipe.spells[1].id = COOKED;
    d.items.insert(RECIPE, recipe);
    let mut high = item(HIGH);
    high.name = "Lofty Thing".into();
    high.required_level = 30;
    d.items.insert(HIGH, high);
    let mut plain = item(PLAIN);
    plain.name = "Plain Thing".into();
    d.items.insert(PLAIN, plain);
    d.crafts
        .random_property_names
        .insert(1182, "of the Bear".into());
    d
}

fn auctioneer() -> Guid {
    guid::unit(AUCTIONEER, 2)
}

async fn search(a: &mut Client, name: &str, usable: u8) -> (Vec<u32>, u32) {
    a.clear();
    a.send(CMSG_AUCTION_LIST_ITEMS {
        auctioneer: auctioneer(),
        list_start_item: 0,
        searched_name: name.into(),
        minimum_level: 0,
        maximum_level: 0,
        auction_slot_id: u32::MAX,
        auction_main_category: u32::MAX,
        auction_sub_category: u32::MAX,
        auction_quality: ItemQuality::Poor,
        usable,
    })
    .await;
    a.wait_for("SMSG_AUCTION_LIST_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_AUCTION_LIST_RESULT(l) => {
            let mut ids: Vec<u32> = l.auctions.iter().map(|x| x.id).collect();
            ids.sort_unstable();
            Some((ids, l.total_amount_of_auctions))
        }
        _ => None,
    })
    .await
}

/// Thistle Tea's `Auction.Query` and `Eligibility`: an item found by its
/// suffix; other markets' and lapsed auctions left out; the usable filter,
/// when asked for, leaving out what the player is too low for and recipes
/// they know; the bidder's list with the auctions they ask after.
#[tokio::test]
async fn the_auction_house_finds_what_is_asked_for() {
    use crate::game::social::auction::Auction;
    let realm = harness::realm_with(auction_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(auctioneer()).await;
    let now = crate::game::social::mail::now();
    {
        let mut auctions = realm.social.auctions.lock();
        let mut up =
            |id: u32, entry: u32, data: &str, market: u32, expires_at: i64, bidder: u32| {
                auctions.auctions.insert(
                    id,
                    Auction {
                        id,
                        market,
                        house: 7,
                        item: crate::game::store::ItemRow {
                            guid: 800_000 + id,
                            entry,
                            count: 1,
                            data: data.into(),
                            ..Default::default()
                        },
                        owner: 99,
                        start_bid: 100,
                        buyout: 1000,
                        bidder,
                        bid: if bidder > 0 { 100 } else { 0 },
                        expires_at,
                        ..Default::default()
                    },
                );
            };
        up(1, ROBE, "rp:1182", 7, now + 7200, 0);
        up(2, AXE, "", 7, now + 7200, 98);
        up(3, ROBE, "", 2, now + 7200, 0);
        up(4, ROBE, "", 7, now - 1, 0);
        up(5, RECIPE, "", 7, now + 7200, 0);
        up(6, HIGH, "", 7, now + 7200, 0);
        up(7, PLAIN, "", 7, now + 7200, 0);
    }
    assert_eq!(search(&mut a, "OF THE BEAR", 0).await, (vec![1], 1));
    assert_eq!(search(&mut a, "eagle", 0).await, (vec![], 0));
    assert_eq!(
        search(&mut a, "robe", 0).await,
        (vec![1], 1),
        "not abroad nor lapsed"
    );
    assert_eq!(search(&mut a, "", 0).await.0, vec![1, 2, 5, 6, 7]);
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().spells.push(COOKED)
    })
    .await;
    assert_eq!(
        search(&mut a, "thing", 1).await,
        (vec![7], 1),
        "not too high, nor a recipe known"
    );
    assert_eq!(search(&mut a, "thing", 0).await.0, vec![5, 6, 7]);
    // Outbid on the axe, Alice asks after it.
    a.send(CMSG_AUCTION_LIST_BIDDER_ITEMS {
        auctioneer: auctioneer(),
        start_from_page: 0,
        outbid_item_ids: vec![2, 2, 99],
    })
    .await;
    let bids = a
        .wait_for("SMSG_AUCTION_BIDDER_LIST_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_AUCTION_BIDDER_LIST_RESULT(l) => {
                Some(l.auctions.iter().map(|x| x.id).collect::<Vec<_>>())
            }
            _ => None,
        })
        .await;
    assert_eq!(bids, vec![2]);
}

/// Thistle Tea's `validate_sale/5`: not a bound, conjured or timed item, a
/// banked one, nor a bag with something in it; told as an inventory error,
/// as mangos tells it.
#[tokio::test]
async fn the_auction_house_takes_only_what_may_be_sold() {
    let mut d = auction_data();
    let mut conjured = item(9325);
    conjured.flags = crate::game::map::items::TEMPLATE_FLAG_CONJURED;
    d.items.insert(9325, conjured);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(auctioneer()).await;
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(10_000)
    })
    .await;
    let sell = |item| CMSG_AUCTION_SELL_ITEM {
        auctioneer: auctioneer(),
        item,
        starting_bid: 100,
        buyout: 200,
        auction_duration_in_minutes: 120,
    };
    let bound = give(&realm, g, TRINKET, 1).await;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_by_guid(bound).unwrap();
        p.items[i].fields.set_u32(ITEM_FIELD_FLAGS, 1);
    })
    .await;
    let conjured = give(&realm, g, 9325, 1).await;
    let timed = give(&realm, g, PLAIN, 1).await;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_by_guid(timed).unwrap();
        p.items[i].fields.set_u32(ITEM_FIELD_DURATION, 600);
    })
    .await;
    let banked = give(&realm, g, AXE, 1).await;
    put(&realm, g, banked, (BAG_PLAYER, BANK_START)).await;
    let satchel = give(&realm, g, SATCHEL, 1).await;
    put(&realm, g, satchel, (BAG_PLAYER, BAG_START)).await;
    let inside = give(&realm, g, WIDGET, 1).await;
    put(&realm, g, inside, (BAG_START, 0)).await;
    for item in [bound, conjured, timed, banked, satchel] {
        a.send(sell(item)).await;
        let why = a
            .wait_for("SMSG_AUCTION_COMMAND_RESULT", |m| match m {
                ServerOpcodeMessage::SMSG_AUCTION_COMMAND_RESULT(r) => {
                    Some(format!("{:?}", r.action))
                }
                _ => None,
            })
            .await;
        assert!(why.contains("ErrInventory"), "{why}");
    }
    let kept = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (
            [bound, conjured, timed, banked, satchel, inside]
                .iter()
                .all(|i| p.item_by_guid(*i).is_some()),
            p.money(),
            m.realm.social.auctions.lock().auctions.len(),
        )
    })
    .await;
    assert_eq!(kept, (true, 10_000, 0));
}

const BANKER: u32 = 3302;

/// Thistle Tea's `Bank.buy_slot/3`: six slots for their six prices, each
/// once; short of money, or at the client's most, nothing; a vendor isn't
/// a banker. wvs tells a purchase Ok, as mangos-classic's
/// `HandleBuyBankSlotOpcode` does.
#[tokio::test]
async fn bank_slots_are_bought_for_their_prices() {
    let mut d = data();
    let mut t = d.creature_templates[&VENDOR].clone();
    t.entry = BANKER;
    t.npc_flags = crate::game::map::npc::NPC_FLAG_BANKER;
    d.creature_templates.insert(BANKER, t);
    d.creature_spawns.get_mut(&0).unwrap().push(CreatureSpawn {
        guid: 3,
        entries: vec![BANKER],
        map: 0,
        x: START.0 + 2.0,
        y: START.1 - 1.0,
        z: START.2,
        ..Default::default()
    });
    let prices = [1_000, 10_000, 100_000, 250_000, 500_000, 1_000_000];
    for (i, price) in prices.iter().enumerate() {
        d.npc.bank_slot_prices.insert(i as u32 + 1, *price);
    }
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let banker = guid::unit(BANKER, 3);
    a.sees(banker).await;
    let result = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::SMSG_BUY_BANK_SLOT_RESULT(r) => Some(r.result),
        _ => None,
    };
    let state = move |m: &mut Map| {
        let p = &m.players[&g];
        (p.fields.get_byte(PLAYER_BYTES_2, 2), p.money())
    };
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(999)
    })
    .await;
    a.send(CMSG_BUY_BANK_SLOT { guid: banker }).await;
    assert_eq!(
        a.wait_for("the result", result).await,
        BuyBankSlotResult::InsufficientFunds
    );
    assert_eq!(on_map(&realm, state).await, (0, 999));
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(2_000_000)
    })
    .await;
    let mut left = 2_000_000;
    for (n, price) in prices.iter().enumerate() {
        a.send(CMSG_BUY_BANK_SLOT { guid: banker }).await;
        assert_eq!(
            a.wait_for("the result", result).await,
            BuyBankSlotResult::Ok
        );
        left -= price;
        assert_eq!(on_map(&realm, state).await, (n as u8 + 1, left));
    }
    assert_eq!(left, 139_000);
    a.send(CMSG_BUY_BANK_SLOT { guid: banker }).await;
    assert_eq!(
        a.wait_for("the result", result).await,
        BuyBankSlotResult::FailedTooMany
    );
    a.send(CMSG_BUY_BANK_SLOT {
        guid: vendor_guid(),
    })
    .await;
    assert_eq!(
        a.wait_for("the result", result).await,
        BuyBankSlotResult::NotBanker
    );
    assert_eq!(on_map(&realm, state).await, (6, 139_000));
}

const CLAM: u32 = 9330;
const MEAT: u32 = 9331;
const LOCKBOX: u32 = 9332;
const SACK: u32 = 9333;
const FEAR: u32 = 900_620;

fn container_data() -> GameData {
    let mut d = data();
    for (entry, lock, slots) in [(CLAM, 0, 0), (LOCKBOX, 5, 0), (SACK, 0, 6)] {
        let mut t = item(entry);
        t.flags = crate::game::map::items_misc::ITEM_FLAG_LOOTABLE;
        t.lock_id = lock;
        t.container_slots = slots;
        t.min_money_loot = 25;
        t.max_money_loot = 25;
        d.items.insert(entry, t);
    }
    d.items.get_mut(&SACK).unwrap().class = 1;
    let mut meat = item(MEAT);
    meat.stackable = 20;
    d.items.insert(MEAT, meat);
    for entry in [CLAM, LOCKBOX] {
        d.loot.item.insert(
            entry,
            vec![LootEntry {
                item: MEAT,
                chance: 100.0,
                min: 2,
                max: 2,
                ..Default::default()
            }],
        );
    }
    let mut fear = crate::game::data::spells::SpellTemplate {
        id: FEAR,
        name: "Fear".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    fear.effects[0] = crate::game::data::spells::SpellEffect {
        effect: 6,
        aura: crate::game::map::auras::AURA_MOD_FEAR,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(FEAR, fear);
    d
}

/// Where an item of the player's is.
async fn place_of(realm: &Arc<Realm>, g: Guid, item: Guid) -> (u8, u8) {
    on_map(realm, move |m| at(m, g, item).map(|(b, s, _)| (b, s)))
        .await
        .unwrap()
}

/// `CMSG_OPEN_ITEM` on `item`: the window's gold and items, or the
/// inventory failure.
async fn open(a: &mut Client, realm: &Arc<Realm>, item: Guid) -> Result<(u32, Vec<u32>), u8> {
    let (bag, slot) = place_of(realm, a.guid, item).await;
    a.clear();
    a.send(CMSG_OPEN_ITEM {
        bag_index: bag,
        slot,
    })
    .await;
    let b = a
        .wait_frame_of(
            "a window or a failure",
            &[crate::game::map::loot::SMSG_LOOT_RESPONSE, 0x112],
            |_| true,
        )
        .await;
    if b.len() == 18 {
        return Err(b[0]);
    }
    let u32_at = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    let items = (0..usize::from(b[13]))
        .map(|n| u32_at(14 + n * 22 + 1))
        .collect();
    Ok((u32_at(9), items))
}

/// Thistle Tea's `ItemOpening`: not an item with nothing in it, a bag, a
/// lockbox not unlocked; not by the dead or the feared; what doesn't fit
/// stays in it, its gold too till taken, and it opens again with what was
/// left; gone, its window takes nothing.
#[tokio::test]
async fn a_container_keeps_what_is_left_in_it() {
    use InventoryResult as R;
    let realm = harness::realm_with(container_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let plain = give(&realm, g, TRINKET, 1).await;
    assert_eq!(
        open(&mut a, &realm, plain).await,
        Err(R::CantDoRightNow.as_int())
    );
    let sack = give(&realm, g, SACK, 1).await;
    assert_eq!(
        open(&mut a, &realm, sack).await,
        Err(R::CantDoRightNow.as_int()),
        "a bag"
    );
    let lockbox = give(&realm, g, LOCKBOX, 1).await;
    assert_eq!(
        open(&mut a, &realm, lockbox).await,
        Err(R::ItemLocked.as_int())
    );
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_by_guid(lockbox).unwrap();
        p.items[i].fields.set_u32(
            ITEM_FIELD_FLAGS,
            crate::game::map::items_misc::ITEM_DYNFLAG_UNLOCKED,
        );
    })
    .await;
    assert_eq!(
        open(&mut a, &realm, lockbox).await,
        Ok((25, vec![MEAT])),
        "unlocked"
    );
    let clam = give(&realm, g, CLAM, 1).await;
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().death = crate::game::map::death::Death::Dead {
            since: std::time::Instant::now(),
        };
    })
    .await;
    assert_eq!(
        open(&mut a, &realm, clam).await,
        Err(R::YouAreDead.as_int())
    );
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().death = crate::game::map::death::Death::Alive;
        let fear = m.realm.data.spells.get(FEAR).cloned().unwrap();
        crate::game::map::auras::apply_spell(m, g, g, &fear, &[(0, 0)], Default::default())
            .unwrap();
    })
    .await;
    assert_eq!(
        open(&mut a, &realm, clam).await,
        Err(R::CantDoRightNow.as_int()),
        "feared"
    );
    on_map(&realm, move |m| {
        crate::game::map::auras::remove_where(m, g, |h| h.spell == FEAR);
    })
    .await;
    // Full bags: the meat stays; the gold is taken; opened again, the meat
    // is there still, and no gold.
    fill(&realm, g).await;
    assert_eq!(open(&mut a, &realm, clam).await, Ok((25, vec![MEAT])));
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.send(CMSG_LOOT_MONEY {}).await;
    a.send(CMSG_LOOT_RELEASE { guid: clam }).await;
    handled(&mut a).await;
    assert_eq!(
        open(&mut a, &realm, clam).await,
        Ok((0, vec![MEAT])),
        "what was left"
    );
    on_map(&realm, move |m| {
        let i = m.players[&g]
            .items
            .iter()
            .position(|i| i.entry == FILLER)
            .unwrap();
        crate::game::map::inventory::take_item(m, g, i, 1);
    })
    .await;
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.send(CMSG_LOOT_RELEASE { guid: clam }).await;
    handled(&mut a).await;
    let emptied = on_map(&realm, move |m| {
        (at(m, g, clam), m.players[&g].count_of(MEAT))
    })
    .await;
    assert_eq!(emptied, (None, 2), "the meat, and the clam used up");
    // A clam gone while open gives nothing.
    let other = give(&realm, g, CLAM, 1).await;
    assert!(open(&mut a, &realm, other).await.is_ok());
    on_map(&realm, move |m| {
        let p = &m.players[&g];
        let i = p.item_by_guid(other).unwrap();
        crate::game::map::inventory::take_item(m, g, i, 1);
    })
    .await;
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    handled(&mut a).await;
    assert_eq!(
        on_map(&realm, move |m| m.players[&g].count_of(MEAT)).await,
        2
    );
}

/// Run the items' clocks on: `secs` of their time pass.
fn pass(m: &mut Map, secs: u64) {
    crate::game::map::items::update(m);
    m.now += Duration::from_secs(secs);
    crate::game::map::items::update(m);
}

fn set_duration(m: &mut Map, g: Guid, item: Guid, secs: u32) {
    let p = m.players.get_mut(&g).unwrap();
    let i = p.item_by_guid(item).unwrap();
    p.items[i].fields.set_u32(ITEM_FIELD_DURATION, secs);
}

/// Thistle Tea's `ItemLifetime` and item durations: a timed bag goes with
/// what is in it; a timed helm goes off its wearer's stamina; a timed quest
/// item gone is needed again; a timed clam's open window closes with it.
#[tokio::test]
async fn timed_items_go_with_all_they_hold() {
    let mut d = container_data();
    d.npc.quests.insert(
        70,
        crate::game::data::npc::Quest {
            id: 70,
            title: "Meat".into(),
            min_level: 1,
            level: 1,
            required_items: [(MEAT, 1), (0, 0), (0, 0), (0, 0)],
            ..Default::default()
        },
    );
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let satchel = give(&realm, g, SATCHEL, 1).await;
    put(&realm, g, satchel, (BAG_PLAYER, BAG_START)).await;
    let widget = give(&realm, g, WIDGET, 1).await;
    put(&realm, g, widget, (BAG_START, 0)).await;
    let visor = give(&realm, g, VISOR, 1).await;
    let bare = on_map(&realm, move |m| {
        m.players[&g].fields.get_u32(UNIT_FIELD_STAT0 + 2)
    })
    .await;
    put(&realm, g, visor, (BAG_PLAYER, 0)).await;
    let meat = give(&realm, g, MEAT, 1).await;
    let gone = on_map(&realm, move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .journal
            .add(crate::game::store::Logged {
                quest: 70,
                ..Default::default()
            });
        quests_check(m, g, 70);
        let done = complete_bit(m, g);
        for item in [satchel, visor, meat] {
            set_duration(m, g, item, 1);
        }
        let worn = m.players[&g].fields.get_u32(UNIT_FIELD_STAT0 + 2);
        pass(m, 2);
        let p = &m.players[&g];
        (
            done,
            worn,
            [satchel, widget, visor, meat].map(|i| p.item_by_guid(i).is_some()),
            p.fields.get_u32(UNIT_FIELD_STAT0 + 2),
            complete_bit(m, g),
        )
    })
    .await;
    assert!(gone.0, "the meat had");
    assert_eq!(gone.1, bare + 5);
    assert_eq!(
        gone.2,
        [false, false, false, false],
        "the bag with its widget"
    );
    assert_eq!(gone.3, bare, "the helm's stamina gone");
    assert!(!gone.4, "the meat needed again");
    // A clam open as its time runs out: its window closes.
    let clam = give(&realm, g, CLAM, 1).await;
    assert!(open(&mut a, &realm, clam).await.is_ok());
    on_map(&realm, move |m| {
        set_duration(m, g, clam, 1);
        pass(m, 2);
    })
    .await;
    a.wait_for("SMSG_LOOT_RELEASE_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_LOOT_RELEASE_RESPONSE(r) => (r.guid == clam).then_some(()),
        _ => None,
    })
    .await;
    let looting = on_map(&realm, move |m| m.players[&g].looting).await;
    assert_eq!(looting, None);
}

fn quests_check(m: &mut Map, g: Guid, quest: u32) {
    crate::game::map::quests::check(m, g, quest);
}

/// Whether the first quest in the log is done, by its log field.
fn complete_bit(m: &Map, g: Guid) -> bool {
    m.players[&g].fields.get_u32(PLAYER_QUEST_LOG_1_1 + 1) & (1 << 24) != 0
}

const PEDDLER: u32 = 3303;
const BUNDLE: u32 = 9340;
const MEDAL: u32 = 9341;
const SECRET: u32 = 9342;
const UNKNOWN: u32 = 9343;

/// A peddler with bundles of three that stack in fives, a rank 8 medal for
/// level 30, an item for level 10 and over, one under a condition with no
/// row; and a merchant with nothing, no menu.
fn peddler_data() -> GameData {
    let mut d = data();
    for (entry, guid_n, y) in [(PEDDLER, 4, -2.0), (PEDDLER + 1, 5, 2.0)] {
        let mut t = d.creature_templates[&VENDOR].clone();
        t.entry = entry;
        d.creature_templates.insert(entry, t);
        d.creature_spawns.get_mut(&0).unwrap().push(CreatureSpawn {
            guid: guid_n,
            entries: vec![entry],
            map: 0,
            x: START.0 + 2.0,
            y: START.1 + y,
            z: START.2,
            ..Default::default()
        });
    }
    let mut bundle = item(BUNDLE);
    (bundle.buy_price, bundle.buy_count, bundle.stackable) = (2, 3, 5);
    d.items.insert(BUNDLE, bundle);
    let mut medal = item(MEDAL);
    (
        medal.required_honor_rank,
        medal.required_level,
        medal.buy_price,
    ) = (8, 30, 1);
    d.items.insert(MEDAL, medal);
    d.items.insert(SECRET, item(SECRET));
    d.items.insert(UNKNOWN, item(UNKNOWN));
    let entry = |slot, item, condition| VendorItem {
        slot,
        item,
        condition,
        ..Default::default()
    };
    d.vendors.insert(
        PEDDLER,
        vec![
            entry(0, BUNDLE, 0),
            entry(1, MEDAL, 0),
            entry(2, SECRET, 1),
            entry(3, UNKNOWN, 999),
        ],
    );
    d.scripts.conditions.insert(
        1,
        crate::game::data::scripts::Condition {
            entry: 1,
            kind: 15,
            values: [10, 1, 0, 0],
            flags: 0,
        },
    );
    d
}

async fn listed(a: &mut Client, from: Guid) -> Vec<u32> {
    a.clear();
    a.send(CMSG_LIST_INVENTORY { guid: from }).await;
    a.wait_for("SMSG_LIST_INVENTORY", |m| match m {
        ServerOpcodeMessage::SMSG_LIST_INVENTORY(l) => {
            Some(l.items.iter().map(|i| i.item).collect())
        }
        _ => None,
    })
    .await
}

async fn buy_one(a: &mut Client, from: Guid, item: u32, amount: u8) -> Option<BuyResult> {
    a.clear();
    a.send(CMSG_BUY_ITEM {
        vendor: from,
        item,
        amount,
        unknown1: 0,
    })
    .await;
    handled(a).await;
    a.gets(Duration::from_millis(1), |m| match m {
        ServerOpcodeMessage::SMSG_BUY_FAILED(f) => Some(f.result),
        _ => None,
    })
    .await
}

/// Thistle Tea's vendor tests: a merchant with no menu opens its empty
/// list, not when dead; four bundles of three in stacks of five, paid once;
/// a ranked medal shown but refused the demoted and the young, then the
/// poor; a conditioned item hidden, shown when met, refused stale; one under
/// a condition with no row never shown.
#[tokio::test]
async fn a_vendor_sells_what_may_be_bought() {
    let realm = harness::realm_with(peddler_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let peddler = guid::unit(PEDDLER, 4);
    let empty = guid::unit(PEDDLER + 1, 5);
    a.sees(peddler).await;
    a.sees(empty).await;
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(100)
    })
    .await;
    a.clear();
    a.send(CMSG_GOSSIP_HELLO { guid: empty }).await;
    let list = a
        .wait_for("SMSG_LIST_INVENTORY", |m| match m {
            ServerOpcodeMessage::SMSG_LIST_INVENTORY(l) => Some((l.vendor, l.items.len())),
            _ => None,
        })
        .await;
    assert_eq!(list, (empty, 0), "an empty merchant");
    on_map(&realm, move |m| crate::game::map::combat::kill(m, g, empty)).await;
    a.clear();
    a.send(CMSG_GOSSIP_HELLO { guid: empty }).await;
    handled(&mut a).await;
    assert!(
        !a.backlog
            .iter()
            .any(|m| matches!(m, ServerOpcodeMessage::SMSG_LIST_INVENTORY(_))),
        "not dead"
    );
    // Four bundles: twelve, in stacks of five, for eight copper.
    assert_eq!(buy_one(&mut a, peddler, BUNDLE, 4).await, None);
    let bought = on_map(&realm, move |m| {
        let mut stacks: Vec<u32> = m.players[&g]
            .items
            .iter()
            .filter(|i| i.entry == BUNDLE)
            .map(|i| i.count())
            .collect();
        stacks.sort_unstable();
        (stacks, m.players[&g].money())
    })
    .await;
    assert_eq!(bought, (vec![2, 5, 5], 92));
    // The medal: shown, but the young and the demoted are refused.
    assert_eq!(listed(&mut a, peddler).await, vec![BUNDLE, MEDAL]);
    let rank = move |m: &mut Map, level: u8, current: u8| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::stats::set_level(p, &d, level);
        p.fields.set_byte(PLAYER_FIELD_BYTES, 3, 18);
        p.fields.set_byte(PLAYER_BYTES_3, 3, current);
    };
    on_map(&realm, move |m| rank(m, 30, 7)).await;
    assert_eq!(
        buy_one(&mut a, peddler, MEDAL, 1).await,
        Some(BuyResult::RankRequire),
        "demoted"
    );
    on_map(&realm, move |m| rank(m, 29, 8)).await;
    assert_eq!(
        buy_one(&mut a, peddler, MEDAL, 1).await,
        Some(BuyResult::RankRequire),
        "too young"
    );
    on_map(&realm, move |m| {
        rank(m, 30, 8);
        m.players.get_mut(&g).unwrap().set_money(0);
    })
    .await;
    assert_eq!(
        buy_one(&mut a, peddler, MEDAL, 1).await,
        Some(BuyResult::NotEnoughtMoney)
    );
    // Level 30 meets the secret's condition; at 9 it's hidden and refused.
    assert_eq!(listed(&mut a, peddler).await, vec![BUNDLE, MEDAL, SECRET]);
    on_map(&realm, move |m| {
        rank(m, 9, 8);
        m.players.get_mut(&g).unwrap().set_money(100);
    })
    .await;
    assert_eq!(listed(&mut a, peddler).await, vec![BUNDLE, MEDAL]);
    assert_eq!(
        buy_one(&mut a, peddler, SECRET, 1).await,
        Some(BuyResult::CantFindItem),
        "stale"
    );
}

/// Thistle Tea's `ItemProperties.select/2`: chances made to sum to one,
/// both ends of the roll; chances out of range left out; nothing of an
/// empty or worthless pool.
#[test]
fn a_random_property_is_picked_by_its_share() {
    use crate::game::map::enchant::select_property;
    let pool = [(1182, 2.0), (1183, 6.0)];
    assert_eq!(select_property(&pool, 0.0), Some(1182));
    assert_eq!(select_property(&pool, 0.25), Some(1182));
    assert_eq!(select_property(&pool, 0.25001), Some(1183));
    assert_eq!(select_property(&pool, 1.0), Some(1183));
    let odd = [(1, 0.0), (2, -1.0), (3, 0.000_001), (4, 100.1), (5, 0.5)];
    assert_eq!(select_property(&odd, 0.0), Some(5));
    assert_eq!(select_property(&odd, 1.0), Some(5));
    assert_eq!(select_property(&[], 0.5), None);
    assert_eq!(select_property(&[(1, 0.0)], 0.5), None);
}

const SUFFIXED: u32 = 9350;

/// Thistle Tea's `Items.give/3` and vendor purchases (mangos's
/// `StoreNewItem` with `GenerateItemRandomPropertyId`): each new item of a
/// template with random properties has one rolled, bought or given, and
/// the push says which.
#[tokio::test]
async fn every_new_item_has_its_random_property_rolled() {
    let mut d = data();
    let mut t = item(SUFFIXED);
    t.random_property = 77;
    d.items.insert(SUFFIXED, t);
    d.crafts.random_pools.insert(77, vec![(1182, 100.0)]);
    d.vendors.get_mut(&VENDOR).unwrap().push(VendorItem {
        slot: 2,
        item: SUFFIXED,
        ..Default::default()
    });
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(vendor_guid()).await;
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(1000)
    })
    .await;
    a.clear();
    a.send(CMSG_BUY_ITEM {
        vendor: vendor_guid(),
        item: SUFFIXED,
        amount: 1,
        unknown1: 0,
    })
    .await;
    let pushed = a
        .wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == SUFFIXED => {
                Some(r.item_random_property_id)
            }
            _ => None,
        })
        .await;
    assert_eq!(pushed, 1182, "the purchase's");
    let given = on_map(&realm, move |m| {
        crate::game::map::inventory::add_item(m, g, SUFFIXED, 2, false).unwrap();
        m.players[&g]
            .items
            .iter()
            .filter(|i| i.entry == SUFFIXED)
            .map(|i| i.fields.get_u32(ITEM_FIELD_RANDOM_PROPERTIES_ID))
            .collect::<Vec<_>>()
    })
    .await;
    assert_eq!(given, vec![1182, 1182, 1182], "each one");
}

/// Thistle Tea's `sync_visible_item` and enchantment projection (mangos's
/// `SetVisibleItemSlot`): a worn item shows others its entry, its
/// permanent and temporary enchantments and its random property; taken
/// off, nothing.
#[tokio::test]
async fn a_worn_item_shows_its_enchantments_and_suffix() {
    let mut d = data();
    let mut t = item(SUFFIXED);
    t.random_property = 77;
    t.inventory_type = 1;
    t.class = 4;
    d.items.insert(SUFFIXED, t);
    d.crafts.random_pools.insert(77, vec![(1182, 100.0)]);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let helm = give(&realm, g, SUFFIXED, 1).await;
    put(&realm, g, helm, (BAG_PLAYER, 0)).await;
    let shown = move |m: &mut Map| {
        let f = &m.players[&g].fields;
        (
            f.get_u32(PLAYER_VISIBLE_ITEM_1_0),
            f.get_u32(PLAYER_VISIBLE_ITEM_1_0 + 1),
            f.get_u32(PLAYER_VISIBLE_ITEM_1_0 + 2),
            f.get_u32(PLAYER_VISIBLE_ITEM_1_PROPERTIES),
        )
    };
    let worn = on_map(&realm, move |m| {
        let plain = shown(m);
        crate::game::map::enchant::enchant_item(m, g, helm, 107, None);
        crate::game::map::enchant::enchant_item(m, g, helm, 900, Some((60_000, 3)));
        (plain, shown(m))
    })
    .await;
    assert_eq!(worn.0, (SUFFIXED, 0, 0, 1182), "its suffix");
    assert_eq!(worn.1, (SUFFIXED, 107, 900, 1182), "and its enchantments");
    put(&realm, g, helm, (BAG_PLAYER, 23)).await;
    assert_eq!(on_map(&realm, shown).await, (0, 0, 0, 0), "taken off");
}
