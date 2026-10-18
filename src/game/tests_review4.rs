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

/// R4-9. A custom channel its last member leaves goes, its password with
/// it (mangos's `LeftChannel`, Thistle Tea's), rather than staying for
/// good, locked and ownerless: one for every name a client locks.
#[tokio::test]
async fn an_empty_locked_channel_goes() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "R4CH");
    a.enter("Locker", Race::Human, Class::Warrior).await;
    let before = realm.social.channels.lock().count();
    for i in 0..3 {
        let name = format!("Locked{i}");
        a.send(CMSG_JOIN_CHANNEL {
            channel_name: name.clone(),
            channel_password: String::new(),
        })
        .await;
        a.send(CMSG_CHANNEL_PASSWORD {
            channel_name: name.clone(),
            channel_password: "forgotten".into(),
        })
        .await;
        a.send(CMSG_LEAVE_CHANNEL { channel_name: name }).await;
    }
    handled(&mut a).await;
    let after = realm.social.channels.lock().count();
    assert_eq!(after, before, "none of them is left");
}

/// R4-10. A spawn tied to more than one event is out in any of those it is
/// tied to, and kept out by any it is kept out of, rather than tied to
/// whichever row came last: New Year's Eve's and the Lunar Festival's
/// fireworks, the Faire's building and the Faire.
#[test]
fn a_spawn_tied_to_two_events_follows_both() {
    use crate::game::events::spawned;
    let on = |ids: &[u32]| ids.iter().copied().collect::<ahash::AHashSet<u32>>();
    assert!(spawned(&[], &on(&[])), "tied to none, always out");
    assert!(spawned(&[34, 38], &on(&[34])), "in the first");
    assert!(spawned(&[34, 38], &on(&[38])), "in the second");
    assert!(!spawned(&[34, 38], &on(&[])), "in neither, not out");
    assert!(spawned(&[-23, -4], &on(&[])), "kept out of neither");
    assert!(!spawned(&[-23, -4], &on(&[23])), "kept out by the first");
    assert!(!spawned(&[-23, -4], &on(&[4])), "kept out by the second");
}

/// R4-10, in the real world: both rows of a spawn tied to two events are
/// loaded.
#[test]
#[ignore = "extended"]
fn the_real_spawns_keep_every_event_they_are_tied_to() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    let mut objects = d.event_objects.get(&363_477).cloned().unwrap_or_default();
    objects.sort_unstable();
    assert_eq!(objects, vec![34, 38], "fireworks for two new years");
    let mut faire = d.event_creatures.get(&80_360).cloned().unwrap_or_default();
    faire.sort_unstable();
    assert_eq!(faire, vec![-23, -4], "away for the Faire and its building");
}

/// R4-11. A quest taken with what it asks for already carried is told
/// complete at once (`SMSG_QUESTUPDATE_COMPLETE`, Thistle Tea's
/// `force_accept`), as one finished later is, rather than its log entry
/// shown done with no word of it.
#[tokio::test]
async fn a_quest_taken_done_is_told_complete() {
    const QUEST: u32 = 990_460;
    let realm = harness::realm_with({
        let mut d = data();
        d.creature_templates.get_mut(&VENDOR).unwrap().npc_flags |= 0x2;
        d.npc.quests.insert(
            QUEST,
            crate::game::data::npc::Quest {
                id: QUEST,
                title: "Widgets".into(),
                min_level: 1,
                level: 1,
                required_items: [(WIDGET, 1), (0, 0), (0, 0), (0, 0)],
                ..Default::default()
            },
        );
        d.npc.creature_starts.insert(VENDOR, vec![QUEST]);
        d
    });
    let mut a = Client::connect(&realm, "R4QC");
    let g = a.enter("Quester", Race::Human, Class::Warrior).await;
    a.sees(vendor_guid()).await;
    on_map(&realm, move |m| {
        crate::game::map::inventory::add_item(m, g, WIDGET, 1, false).unwrap();
    })
    .await;
    a.clear();
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: vendor_guid(),
        quest_id: QUEST,
    })
    .await;
    let told = a
        .gets(Duration::from_secs(1), |m| match m {
            ServerOpcodeMessage::SMSG_QUESTUPDATE_COMPLETE(c) => Some(c.quest_id),
            _ => None,
        })
        .await;
    let taken = on_map(&realm, move |m| m.players[&g].journal.has(QUEST)).await;
    assert!(taken, "taken");
    assert_eq!(told, Some(QUEST), "and told complete");
}

/// R4-12. A quest abandoned takes the auras its being taken gave
/// (`spell_area`'s quest_start_active), as one rewarded does, rather than
/// leaving them until the player changes area.
#[tokio::test]
async fn an_abandoned_quests_area_aura_goes() {
    use super::tests_p3_cast_kit::{LASTS_LONG, aura, bare, effect, fast_realm, insert};
    const QUEST: u32 = 990_470;
    const BLESSING: u32 = 990_471;
    let realm = fast_realm(|d| {
        let mine = data();
        d.creature_templates.extend(mine.creature_templates);
        d.creature_spawns.extend(mine.creature_spawns);
        d.creature_templates.get_mut(&VENDOR).unwrap().npc_flags |= 0x2;
        d.npc.quests.insert(
            QUEST,
            crate::game::data::npc::Quest {
                id: QUEST,
                title: "Blessed".into(),
                min_level: 1,
                level: 1,
                ..Default::default()
            },
        );
        d.npc.creature_starts.insert(VENDOR, vec![QUEST]);
        let mut s = bare(BLESSING, 0);
        s.duration_index = LASTS_LONG;
        effect(&mut s, 0, aura(29, 0, 5, 1));
        insert(d, [s]);
        d.scripts
            .spell_areas
            .push(crate::game::data::scripts::SpellArea {
                spell: BLESSING,
                quest_start: QUEST,
                quest_start_active: true,
                gender: 2,
                autocast: true,
                ..Default::default()
            });
    });
    let mut a = Client::connect(&realm, "R4SA");
    let g = a.enter("Blessed", Race::Human, Class::Warrior).await;
    a.sees(vendor_guid()).await;
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: vendor_guid(),
        quest_id: QUEST,
    })
    .await;
    handled(&mut a).await;
    let blessed = on_map(&realm, move |m| m.players[&g].auras.has_spell(BLESSING)).await;
    assert!(blessed, "blessed while it is taken");
    a.send(CMSG_QUESTLOG_REMOVE_QUEST { slot: 0 }).await;
    handled(&mut a).await;
    let (taken, still) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (p.journal.has(QUEST), p.auras.has_spell(BLESSING))
    })
    .await;
    assert!(!taken, "abandoned");
    assert!(!still, "and the blessing with it");
}

/// R4-13. A used-up spawn a script brings out for a while (command 9) comes
/// back as it was due to, once the script takes it in again, rather than
/// its return, come due while the script held it out, lost: the spawn gone
/// until the map starts again.
#[tokio::test]
async fn a_spawn_a_script_brought_out_still_comes_back() {
    use crate::game::data::scripts::{ScriptStep, ScriptTable};
    const CHEST: u32 = 990_480;
    const SPAWN: u32 = 990_481;
    const BRING: u32 = 990_482;
    let realm = harness::realm_with({
        let mut d = data();
        d.gameobject_templates.insert(
            CHEST,
            GameObjectTemplate {
                entry: CHEST,
                kind: 3,
                name: "Chest".into(),
                display_id: 10,
                size: 1.0,
                ..Default::default()
            },
        );
        d.gameobject_spawns.insert(
            0,
            vec![GameObjectSpawn {
                guid: SPAWN,
                entry: CHEST,
                map: 0,
                x: START.0 - 5.0,
                y: START.1,
                z: START.2,
                state: 1,
                respawn_min: 60,
                respawn_max: 60,
                ..Default::default()
            }],
        );
        d.scripts.generic.insert(
            BRING,
            vec![ScriptStep {
                command: 9,
                datalong: [SPAWN, 90, 0, 0],
                ..Default::default()
            }],
        );
        d
    });
    let mut a = Client::connect(&realm, "R4OB");
    let g = a.enter("Watcher", Race::Human, Class::Warrior).await;
    let back = on_map(&realm, move |m| {
        use crate::game::map::{objects, scripts};
        let out = |m: &Map| scripts::object_of_spawn(m, SPAWN);
        let chest = out(m).expect("the chest is out");
        // Used up: back in a minute.
        objects::despawn(m, chest);
        assert!(out(m).is_none());
        let t0 = m.now;
        scripts::start(m, ScriptTable::Generic, BRING, g, g);
        assert!(out(m).is_some(), "brought out");
        let mut seen = Vec::new();
        // Due back at 60 while the script holds it out until 90.
        for secs in [61, 91, 200] {
            m.now = t0 + Duration::from_secs(secs);
            scripts::update_objects(m);
            objects::update(m);
            seen.push(out(m).is_some());
        }
        seen
    })
    .await;
    assert_eq!(back, vec![true, true, true], "out, and back as it was due");
}

/// R4-14. A charter turned in must be in hand (mangos's
/// `HandleTurnInPetitionOpcode`): one sold or destroyed, its GUID sent
/// again, founds no guild.
#[tokio::test]
async fn a_charter_not_held_founds_no_guild() {
    use crate::game::social::guilds::Petition;
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "R4PT");
    let g = a.enter("Founder", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "R4PT2");
    let signer = b.enter("Signer", Race::Human, Class::Warrior).await;
    let charter = guid::item(990_490);
    realm.social.guilds.lock().petitions.insert(
        990_490,
        Petition {
            id: 990_490,
            owner: guid::counter(g),
            name: "Gone Charter".into(),
            signers: vec![guid::counter(signer)],
        },
    );
    a.send(CMSG_TURN_IN_PETITION { petition: charter }).await;
    handled(&mut a).await;
    let founded = realm.social.guilds.lock().of(g).is_some();
    assert!(!founded, "no guild without its charter");
}

/// R4-15. An auction is cancelled at an auctioneer, as every other auction
/// command is (mangos's `HandleAuctionRemoveItem`), not from anywhere.
#[tokio::test]
async fn an_auction_is_cancelled_only_at_an_auctioneer() {
    use crate::game::social::auction::Auction;
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "R4AC");
    let g = a.enter("Canceller", Race::Human, Class::Warrior).await;
    realm.social.auctions.lock().auctions.insert(
        9,
        Auction {
            id: 9,
            market: 2,
            house: 2,
            owner: guid::counter(g),
            start_bid: 1,
            expires_at: i64::MAX,
            ..Default::default()
        },
    );
    // The vendor is no auctioneer.
    a.send(CMSG_AUCTION_REMOVE_ITEM {
        auctioneer: vendor_guid(),
        auction_id: 9,
    })
    .await;
    handled(&mut a).await;
    let still = realm.social.auctions.lock().auctions.contains_key(&9);
    assert!(still, "not cancelled away from the house");
}

/// R4-16. An item bought back is held to its unique limit, as one bought
/// is (mangos's `CanStoreItem`): sell a unique ring, take another, and the
/// first can't be bought back as a second.
#[tokio::test]
async fn a_unique_item_bought_back_keeps_its_limit() {
    const RING: u32 = 990_403;
    let realm = harness::realm_with({
        let mut d = data();
        let mut ring = item(RING);
        ring.max_count = 1;
        d.items.insert(RING, ring);
        d
    });
    let mut a = Client::connect(&realm, "R4UQ");
    let g = a.enter("Ringer", Race::Human, Class::Warrior).await;
    a.sees(vendor_guid()).await;
    let first = on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(1000);
        crate::game::map::inventory::add_item(m, g, RING, 1, false).unwrap();
        let p = &m.players[&g];
        p.items.iter().find(|i| i.entry == RING).unwrap().guid
    })
    .await;
    a.send(CMSG_SELL_ITEM {
        vendor: vendor_guid(),
        item: first,
        amount: 0,
    })
    .await;
    handled(&mut a).await;
    on_map(&realm, move |m| {
        crate::game::map::inventory::add_item(m, g, RING, 1, false).expect("one again");
    })
    .await;
    a.send(CMSG_BUYBACK_ITEM {
        guid: vendor_guid(),
        slot: BuybackSlot::Slot1,
    })
    .await;
    handled(&mut a).await;
    let (rings, first_at) = on_map(&realm, move |m| {
        (m.players[&g].count_of(RING), place(m, g, first))
    })
    .await;
    assert_eq!(rings, 1, "still one");
    assert_eq!(
        first_at.map(|f| f.1),
        Some(BUYBACK_START),
        "the sold one still sold"
    );
}
