//! Round 9, the trade window's spells and the buyback: Thistle Tea's
//! `trade/opening_test.exs` and `trade/spells_test.exs` (a key and an oil
//! used at the slot not traded) and `vendor/buyback_test.exs` (time kept
//! while an item waits to be bought back), through the protocol.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::items::ItemSpell;
use crate::game::data::spells::{Enchantment, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey, enchant, skills};

const JUNKBOX: u32 = 9_791;
const STRONGBOX: u32 = 9_792;
const KEY: u32 = 9_793;
const OIL: u32 = 9_794;
const BRACER: u32 = 9_795;
const FILLER: u32 = 9_796;
const HOURGLASS: u32 = 9_797;
const VENDOR: u32 = 9_798;
const KEY_SPELL: u32 = 19_646;
const OIL_SPELL: u32 = 25_125;
const COATING: u32 = 2_623;
const LOCKPICKING: u32 = 633;
const BLACKSMITHING: u32 = 164;
const ITEM_FLAG_UNLOCKED: u32 = 0x4;
const NOT_REPORTED: u8 = 0x17;
const ITEM_GONE: u8 = 0x26;
const LOW_CASTLEVEL: u8 = 0x2c;

fn item(entry: u32, name: &str) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: name.into(),
        display_id: 1000,
        allowable_class: -1,
        allowable_race: -1,
        stackable: 1,
        buy_count: 1,
        buy_price: 100,
        sell_price: 50,
        ..Default::default()
    }
}

fn spell(id: u32, effect: u32, misc: i32, points: i32) -> SpellTemplate {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect,
        misc_value: misc,
        base_points: points - 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    SpellTemplate {
        id,
        name: format!("Spell {id}"),
        effects,
        equipped_item_class: -1,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    // A junkbox no skill opens; a strongbox of 70.
    let mut junkbox = item(JUNKBOX, "Junkbox");
    junkbox.lock_id = 5;
    junkbox.flags = 0x4;
    let mut strongbox = item(STRONGBOX, "Strongbox");
    strongbox.lock_id = 6;
    strongbox.flags = 0x4;
    for (lock, skill) in [(5, 0), (6, 70)] {
        d.crafts.locks.insert(
            lock,
            vec![crate::game::data::crafts::LockKey {
                kind: 2,
                index: 1,
                skill,
            }],
        );
    }
    // A blacksmith's key of strength 25, used up a charge at a time; an
    // oil coating a weapon half an hour.
    let mut key = item(KEY, "Skeleton Key");
    key.stackable = 20;
    key.required_skill = BLACKSMITHING;
    key.required_skill_rank = 1;
    key.spells[0] = ItemSpell {
        id: KEY_SPELL,
        charges: -1,
        ..Default::default()
    };
    let mut oil = item(OIL, "Oil");
    oil.stackable = 5;
    oil.spells[0] = ItemSpell {
        id: OIL_SPELL,
        charges: -1,
        ..Default::default()
    };
    let mut bracer = item(BRACER, "Bracers");
    bracer.class = 4;
    bracer.inventory_type = 9;
    let mut hourglass = item(HOURGLASS, "Hourglass");
    hourglass.duration = 3600;
    for t in [
        junkbox,
        strongbox,
        key,
        oil,
        bracer,
        item(FILLER, "Filler"),
        hourglass,
    ] {
        d.items.insert(t.entry, t);
    }
    d.spells
        .spells
        .insert(KEY_SPELL, spell(KEY_SPELL, 33, 1, 25));
    d.spells
        .spells
        .insert(OIL_SPELL, spell(OIL_SPELL, 54, COATING as i32, 1800));
    d.spells.enchantments.insert(
        COATING,
        Enchantment {
            id: COATING,
            effects: [(0, 0, 0, 0); 3],
            name: "Oil".into(),
        },
    );
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
            npc_flags: crate::game::map::vendor::NPC_FLAG_VENDOR,
            ..Default::default()
        },
    );
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: 2,
            entries: vec![VENDOR],
            map: 0,
            x: -8949.95 - 2.0,
            y: -132.493,
            z: 83.5312,
            ..Default::default()
        }],
    );
    d
}

/// Players, not GMs (a GM spends no reagents).
fn realm() -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, data(), None)
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

/// Alice the rogue, a blacksmith of a point, and Bob, each carrying
/// nothing and a thousand copper.
async fn two(realm: &Arc<Realm>) -> (Client, Client) {
    let mut a = Client::connect(realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Rogue).await;
    let mut b = Client::connect(realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.sees(gb).await;
    b.sees(ga).await;
    on_map(realm, move |m| {
        let d = m.realm.data.clone();
        for g in [ga, gb] {
            let p = m.players.get_mut(&g).unwrap();
            p.items.clear();
            p.set_money(1000);
            p.refresh_inventory(&d);
        }
        let p = m.players.get_mut(&ga).unwrap();
        skills::learn_skill(p, BLACKSMITHING, 1);
        skills::set_value(p, BLACKSMITHING, 1);
    })
    .await;
    (a, b)
}

/// An item of `entry` for `g`: its guid and where it is.
async fn give(realm: &Arc<Realm>, g: Guid, entry: u32, count: u32) -> (Guid, (u8, u8)) {
    on_map(realm, move |m| {
        crate::game::map::inventory::add_item(m, g, entry, count, false).expect("room");
        let p = &m.players[&g];
        let i = p.items.iter().rposition(|i| i.entry == entry).unwrap();
        (p.items[i].guid, (p.items[i].bag, p.items[i].slot))
    })
    .await
}

async fn count_of(realm: &Arc<Realm>, g: Guid, entry: u32) -> u32 {
    on_map(realm, move |m| m.players[&g].count_of(entry)).await
}

async fn unlocked(realm: &Arc<Realm>, g: Guid, item: Guid) -> bool {
    on_map(realm, move |m| {
        let p = &m.players[&g];
        let i = p.item_by_guid(item).expect("still theirs");
        p.items[i].fields.get_u32(ITEM_FIELD_FLAGS) & ITEM_FLAG_UNLOCKED != 0
    })
    .await
}

/// Alice and Bob trading, Bob's item at `at` in the slot not traded.
async fn trading(a: &mut Client, b: &mut Client, at: (u8, u8)) {
    a.send(CMSG_INITIATE_TRADE { guid: b.guid }).await;
    b.wait_for("the trade asked", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::BeginTrade { .. }).then_some(())
        }
        _ => None,
    })
    .await;
    b.send(CMSG_BEGIN_TRADE {}).await;
    b.send(CMSG_SET_TRADE_ITEM {
        trade_slot: 6,
        bag: at.0,
        slot: at.1,
    })
    .await;
    a.wait_for("Bob's item in the seventh slot", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS_EXTENDED(t) => {
            (t.trade_slots[6].item != 0).then_some(())
        }
        _ => None,
    })
    .await;
}

/// `c` uses the item at `at` on the trade's slot not traded.
async fn use_at_the_trade(c: &Client, at: (u8, u8)) {
    c.send(CMSG_USE_ITEM {
        bag_index: at.0,
        bag_slot: at.1,
        spell_index: 0,
        targets: SpellCastTargets {
            target_flags: SpellCastTargets_SpellCastTargetFlags::empty().set_item(
                SpellCastTargets_SpellCastTargetFlags_Item::TradeItem {
                    trade_item: Guid::new(6),
                },
            ),
        },
    })
    .await;
}

/// A cast result for `spell`: `None` for success, else its code.
async fn cast_result(c: &mut Client, spell: u32) -> Option<u8> {
    let b = c
        .wait_frame(
            "SMSG_CAST_RESULT",
            crate::game::map::spells::SMSG_CAST_RESULT,
            |b| b[..4] == spell.to_le_bytes(),
        )
        .await;
    (b[4] != 0).then(|| b[5])
}

async fn both_accept(a: &mut Client, b: &mut Client) {
    a.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    b.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
}

async fn status(c: &mut Client, what: &str, want: fn(&SMSG_TRADE_STATUS_TradeStatus) -> bool) {
    c.wait_for(what, |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => want(&s.status).then_some(()),
        _ => None,
    })
    .await;
}

async fn completed(c: &mut Client) {
    status(c, "TRADE_COMPLETE", |s| {
        matches!(s, SMSG_TRADE_STATUS_TradeStatus::TradeComplete)
    })
    .await;
}

async fn cancel(a: &mut Client, b: &mut Client) {
    a.send(CMSG_CANCEL_TRADE {}).await;
    status(b, "the trade cancelled", |s| {
        matches!(s, SMSG_TRADE_STATUS_TradeStatus::TradeCanceled)
    })
    .await;
    a.clear();
    b.clear();
}

/// Thistle Tea's "a queued key consumes one charge only in the successful
/// exchange": a key used at the slot not traded waits in its user's offer,
/// its charge spent only as the trade goes through, the box opened and no
/// lockpicking learned by it. A key its user may not use (a blacksmith's,
/// to one no longer a blacksmith), or one offered in the trade itself, is
/// gone.
#[tokio::test]
async fn a_key_at_the_trade_spends_its_charge_only_as_the_trade_goes_through() {
    let realm = realm();
    let (mut a, mut b) = two(&realm).await;
    let (ga, gb) = (a.guid, b.guid);
    let (_, key_at) = give(&realm, ga, KEY, 2).await;
    let (junkbox, at) = give(&realm, gb, JUNKBOX, 1).await;
    trading(&mut a, &mut b, at).await;
    use_at_the_trade(&a, key_at).await;
    assert_eq!(cast_result(&mut a, KEY_SPELL).await, Some(NOT_REPORTED));
    assert_eq!(count_of(&realm, ga, KEY).await, 2, "not spent yet");
    both_accept(&mut a, &mut b).await;
    completed(&mut b).await;
    assert_eq!(cast_result(&mut a, KEY_SPELL).await, None);
    assert_eq!(count_of(&realm, ga, KEY).await, 1, "a charge spent");
    assert!(unlocked(&realm, gb, junkbox).await, "Bob's box open");
    let lockpicking = on_map(&realm, move |m| skills::value(&m.players[&ga], LOCKPICKING)).await;
    assert_eq!(lockpicking, None, "no lockpicking from a key");
    // No blacksmith: the key isn't Alice's to use.
    a.clear();
    let (_, at) = give(&realm, gb, JUNKBOX, 1).await;
    on_map(&realm, move |m| {
        skills::forget_skill(m.players.get_mut(&ga).unwrap(), BLACKSMITHING)
    })
    .await;
    trading(&mut a, &mut b, at).await;
    use_at_the_trade(&a, key_at).await;
    assert_eq!(cast_result(&mut a, KEY_SPELL).await, Some(ITEM_GONE));
    cancel(&mut a, &mut b).await;
    // Offered in the trade: gone too.
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        skills::learn_skill(p, BLACKSMITHING, 1);
        skills::set_value(p, BLACKSMITHING, 1);
    })
    .await;
    trading(&mut a, &mut b, at).await;
    a.send(CMSG_SET_TRADE_ITEM {
        trade_slot: 0,
        bag: key_at.0,
        slot: key_at.1,
    })
    .await;
    b.wait_for("Alice's key offered", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS_EXTENDED(t) => {
            (t.self_player && t.trade_slots[0].item != 0).then_some(())
        }
        _ => None,
    })
    .await;
    use_at_the_trade(&a, key_at).await;
    assert_eq!(cast_result(&mut a, KEY_SPELL).await, Some(ITEM_GONE));
}

/// Thistle Tea's "a key uses its own opening strength instead of the
/// caster's skill": a key of 25 doesn't open a lock of 70, whatever its
/// user's lockpicking.
#[tokio::test]
async fn a_key_opens_by_its_own_strength_not_its_users_skill() {
    let realm = realm();
    let (mut a, mut b) = two(&realm).await;
    let (ga, gb) = (a.guid, b.guid);
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        skills::learn_skill(p, LOCKPICKING, 2);
        skills::set_value(p, LOCKPICKING, 100);
    })
    .await;
    let (_, key_at) = give(&realm, ga, KEY, 2).await;
    let (_, at) = give(&realm, gb, STRONGBOX, 1).await;
    trading(&mut a, &mut b, at).await;
    use_at_the_trade(&a, key_at).await;
    assert_eq!(cast_result(&mut a, KEY_SPELL).await, Some(LOW_CASTLEVEL));
    assert_eq!(count_of(&realm, ga, KEY).await, 2);
}

/// Thistle Tea's "failed recipient storage leaves the lock, key and payment
/// untouched": a trade the receiver has no room for goes back to trading,
/// the key's charge unspent, the box shut and the money where it was.
#[tokio::test]
async fn a_trade_with_no_room_spends_no_key_opens_nothing_and_pays_nothing() {
    let realm = realm();
    let (mut a, mut b) = two(&realm).await;
    let (ga, gb) = (a.guid, b.guid);
    let (_, key_at) = give(&realm, ga, KEY, 2).await;
    let (_, gift_at) = give(&realm, ga, BRACER, 1).await;
    let (junkbox, at) = give(&realm, gb, JUNKBOX, 1).await;
    // Bob's pack full.
    on_map(&realm, move |m| {
        while crate::game::map::inventory::add_item(m, gb, FILLER, 1, false).is_ok() {}
    })
    .await;
    trading(&mut a, &mut b, at).await;
    a.send(CMSG_SET_TRADE_ITEM {
        trade_slot: 0,
        bag: gift_at.0,
        slot: gift_at.1,
    })
    .await;
    a.send(CMSG_SET_TRADE_GOLD {
        gold: Gold::new(500),
    })
    .await;
    b.wait_for("Alice's offer", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS_EXTENDED(t) => {
            (t.self_player && t.trade_slots[0].item != 0 && t.money_in_trade.as_int() == 500)
                .then_some(())
        }
        _ => None,
    })
    .await;
    use_at_the_trade(&a, key_at).await;
    assert_eq!(cast_result(&mut a, KEY_SPELL).await, Some(NOT_REPORTED));
    both_accept(&mut a, &mut b).await;
    status(&mut a, "back to trading", |s| {
        matches!(s, SMSG_TRADE_STATUS_TradeStatus::BackToTrade)
    })
    .await;
    assert_eq!(
        count_of(&realm, ga, KEY).await,
        2,
        "the key's charge unspent"
    );
    assert!(!unlocked(&realm, gb, junkbox).await, "the box shut");
    let money = on_map(&realm, move |m| {
        (m.players[&ga].money(), m.players[&gb].money())
    })
    .await;
    assert_eq!(money, (1000, 1000), "no one paid");
    assert_eq!(count_of(&realm, ga, BRACER).await, 1, "the gift Alice's");
}

/// Thistle Tea's "plans temporary coatings and spends only one application
/// of a cast item": an oil used at the slot not traded coats the partner's
/// item, theirs still, for its time, and is spent once.
#[tokio::test]
async fn an_oil_at_the_trade_coats_the_partners_item_and_is_spent_once() {
    let realm = realm();
    let (mut a, mut b) = two(&realm).await;
    let (ga, gb) = (a.guid, b.guid);
    let (_, oil_at) = give(&realm, ga, OIL, 2).await;
    let (bracer, at) = give(&realm, gb, BRACER, 1).await;
    trading(&mut a, &mut b, at).await;
    use_at_the_trade(&a, oil_at).await;
    assert_eq!(cast_result(&mut a, OIL_SPELL).await, Some(NOT_REPORTED));
    both_accept(&mut a, &mut b).await;
    completed(&mut b).await;
    assert_eq!(count_of(&realm, ga, OIL).await, 1, "one application spent");
    let coated = on_map(&realm, move |m| {
        let p = &m.players[&gb];
        p.item_by_guid(bracer)
            .map(|i| enchant::get(&p.items[i], enchant::TEMP_SLOT))
    })
    .await
    .expect("Bob's still");
    assert_eq!(coated.id, COATING);
    assert!(
        (1_790_000..=1_800_000).contains(&coated.duration_ms),
        "half an hour: {}",
        coated.duration_ms
    );
}

/// Thistle Tea's "resumes both real-time item and temporary-enchantment
/// budgets after buyback": an item sold keeps what time it had, and its
/// coating's, while it waits to be bought back; bought back, they run on.
#[tokio::test]
async fn a_sold_item_keeps_its_time_till_it_is_bought_back() {
    let realm = realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let vendor = guid::unit(VENDOR, 2);
    a.sees(vendor).await;
    let (glass, _) = give(&realm, g, HOURGLASS, 1).await;
    on_map(&realm, move |m| {
        enchant::enchant_item(m, g, glass, COATING, Some((600_000, 0)));
    })
    .await;
    let times = move |m: &Map| {
        let p = &m.players[&g];
        let it = &p.items[p.item_by_guid(glass).unwrap()];
        (
            it.fields.get_u32(ITEM_FIELD_DURATION),
            enchant::get(it, enchant::TEMP_SLOT).duration_ms,
            enchant::in_buyback(it),
        )
    };
    // A minute gone by on the clocks.
    let a_minute = move |m: &mut Map| {
        let ago = m.now.checked_sub(Duration::from_secs(60)).unwrap();
        m.loot_state.items_tick_at = Some(ago);
        m.loot_state.enchant_tick_at = Some(ago);
        crate::game::map::items::update(m);
        enchant::update(m);
    };
    let before = on_map(&realm, move |m| times(m)).await;
    assert_eq!((before.0, before.1), (3600, 600_000));
    a.send(CMSG_SELL_ITEM {
        vendor,
        item: glass,
        amount: 0,
    })
    .await;
    let mut sold = false;
    for _ in 0..1000 {
        if on_map(&realm, move |m| times(m).2).await {
            sold = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(sold, "sold");
    let waiting = on_map(&realm, move |m| {
        a_minute(m);
        times(m)
    })
    .await;
    assert_eq!((waiting.0, waiting.1), (3600, 600_000), "kept while sold");
    a.send(CMSG_BUYBACK_ITEM {
        guid: vendor,
        slot: BuybackSlot::Slot1,
    })
    .await;
    let mut back = false;
    for _ in 0..1000 {
        if on_map(&realm, move |m| !times(m).2).await {
            back = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(back, "bought back");
    let after = on_map(&realm, move |m| {
        a_minute(m);
        times(m)
    })
    .await;
    assert_eq!((after.0, after.1), (3540, 540_000), "running on");
}
