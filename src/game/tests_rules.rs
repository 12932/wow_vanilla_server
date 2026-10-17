//! Thistle Tea's rules through the protocol: spells on the trade window's
//! seventh slot (an enchantment, a lock picked), the run back to a corpse
//! (the wait growing with deaths close together, a dungeon's corpse shown
//! at its entrance and its portal for its ghost alone), and items bound to
//! a map or zone, used on certain creatures, or timed.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::scripts::MapTemplate;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::corpses;
use crate::game::map::death::Death;
use crate::game::map::{Map, MapHandle, MapKey};
use crate::game::store::BAG_PLAYER;

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const NONE_WITHIN: Duration = Duration::from_millis(300);

const BRACER: u32 = 9200;
const DUST: u32 = 9201;
const JUNKBOX: u32 = 9202;
const BEACON: u32 = 9203;
const HOURGLASS: u32 = 9204;
const TORCH: u32 = 9205;
const ELWYNN_CHARM: u32 = 9206;
const ENCHANT_BRACER: u32 = 7418;
const HEALTH_5: u32 = 41;
const PICK_LOCK: u32 = 1804;
const LIGHT_TORCH: u32 = 9300;
const BOAR: u32 = 3100;

const DEADMINES: u32 = 36;
const STOCKADE: u32 = 34;
const DEADMINES_PORTAL: u32 = 9001;
const STOCKADE_PORTAL: u32 = 9002;
const DEADMINES_OUTSIDE: (f32, f32) = (-11207.8, 1681.15);

fn item(entry: u32, name: &str) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: name.into(),
        allowable_class: -1,
        allowable_race: -1,
        stackable: 1,
        buy_count: 1,
        ..Default::default()
    }
}

fn spell(id: u32, effect: u32, misc: i32) -> SpellTemplate {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect,
        misc_value: misc,
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
    let mut bracer = item(BRACER, "Bracers");
    bracer.class = 4;
    bracer.subclass = 1;
    bracer.inventory_type = 9;
    let mut dust = item(DUST, "Strange Dust");
    dust.stackable = 20;
    let mut junkbox = item(JUNKBOX, "Battered Junkbox");
    junkbox.lock_id = 5;
    junkbox.flags = 0x4;
    let mut beacon = item(BEACON, "Beacon");
    beacon.map_bound = 30;
    let mut hourglass = item(HOURGLASS, "Hourglass");
    hourglass.duration = 3600;
    let mut torch = item(TORCH, "Torch");
    torch.spells[0] = crate::game::data::items::ItemSpell {
        id: LIGHT_TORCH,
        trigger: 0,
        ..Default::default()
    };
    let mut charm = item(ELWYNN_CHARM, "Elwynn Charm");
    charm.area_bound = 12;
    for t in [bracer, dust, junkbox, beacon, hourglass, torch, charm] {
        d.items.insert(t.entry, t);
    }
    // Enchant Bracer - Minor Health, as vmangos has it but for a dust.
    let mut enchant = spell(ENCHANT_BRACER, 53, HEALTH_5 as i32);
    enchant.reagents[0] = (DUST, 1);
    enchant.equipped_item_class = 4;
    enchant.equipped_item_inventory_type_mask = 1 << 9;
    d.spells.spells.insert(ENCHANT_BRACER, enchant);
    d.spells.enchantments.insert(
        HEALTH_5,
        crate::game::data::spells::Enchantment {
            id: HEALTH_5,
            effects: [(4, 5, 5, 0), (0, 0, 0, 0), (0, 0, 0, 0)],
            name: "Health +5".into(),
        },
    );
    d.spells.spells.insert(PICK_LOCK, spell(PICK_LOCK, 33, 1));
    d.spells
        .spells
        .insert(LIGHT_TORCH, spell(LIGHT_TORCH, 3, 0));
    d.crafts.item_targets.insert(TORCH, vec![(BOAR, true)]);
    d.creature_templates.insert(
        BOAR,
        CreatureTemplate {
            entry: BOAR,
            name: "Boar".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 2,
            level_max: 2,
            faction: 7,
            unit_class: 1,
            health_multiplier: 1.0,
            health_max: 500,
            ..Default::default()
        },
    );
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: 30,
            entries: vec![BOAR],
            map: 0,
            x: START.0 + 4.0,
            y: START.1,
            z: START.2,
            respawn_min: 600,
            respawn_max: 600,
            ..Default::default()
        }],
    );
    // The Deadmines and the Stockade, their portals where players start.
    d.maps.insert(
        DEADMINES,
        MapInfo {
            id: DEADMINES,
            internal_name: "DeadminesInstance".into(),
            name: "Deadmines".into(),
            instance_type: 1,
            player_limit: 10,
        },
    );
    for (map, name, entrance) in [
        (
            DEADMINES,
            "Deadmines",
            Some((0, DEADMINES_OUTSIDE.0, DEADMINES_OUTSIDE.1)),
        ),
        (STOCKADE, "The Stockade", Some((0, -8762.38, 848.01))),
    ] {
        d.scripts.map_templates.insert(
            map,
            MapTemplate {
                map,
                map_type: 1,
                name: name.into(),
                ghost_entrance: entrance,
                ..Default::default()
            },
        );
    }
    for (id, to) in [(DEADMINES_PORTAL, DEADMINES), (STOCKADE_PORTAL, STOCKADE)] {
        d.area_triggers.insert(
            id,
            AreaTrigger {
                id,
                map: 0,
                x: START.0,
                y: START.1,
                z: START.2,
                radius: 10.0,
                teleport: Some(Teleport {
                    map: to,
                    x: -16.4,
                    y: -383.07,
                    z: 61.78,
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
    }
    d
}

fn map0(realm: &Arc<Realm>) -> MapHandle {
    realm.maps.get(realm, MapKey::world(0))
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map0(realm).query(f).await.expect("the map answers")
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

async fn item_field(realm: &Arc<Realm>, g: Guid, item: Guid, field: u16) -> Option<u32> {
    on_map(realm, move |m| {
        let p = &m.players[&g];
        p.item_by_guid(item)
            .map(|i| p.items[i].fields.get_u32(field))
    })
    .await
}

/// Players, not GMs (a GM spends no reagents).
fn players_realm() -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, data(), None)
}

async fn two(realm: &Arc<Realm>) -> (Client, Client) {
    let mut a = Client::connect(realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Rogue).await;
    let mut b = Client::connect(realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.sees(gb).await;
    b.sees(ga).await;
    (a, b)
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

fn at_trade_slot() -> SpellCastTargets {
    SpellCastTargets {
        target_flags: SpellCastTargets_SpellCastTargetFlags::empty().set_item(
            SpellCastTargets_SpellCastTargetFlags_Item::TradeItem {
                trade_item: Guid::new(6),
            },
        ),
    }
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

async fn completed(c: &mut Client) {
    c.wait_for("TRADE_COMPLETE", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::TradeComplete).then_some(())
        }
        _ => None,
    })
    .await;
}

#[tokio::test]
async fn an_enchanter_enchants_the_item_their_partner_shows() {
    let realm = players_realm();
    let (mut a, mut b) = two(&realm).await;
    let (ga, gb) = (a.guid, b.guid);
    on_map(&realm, move |m| {
        m.players.get_mut(&ga).unwrap().spells.push(ENCHANT_BRACER)
    })
    .await;
    give(&realm, ga, DUST, 1).await;
    let (bracer, at) = give(&realm, gb, BRACER, 1).await;
    trading(&mut a, &mut b, at).await;
    a.send(CMSG_CAST_SPELL {
        spell: ENCHANT_BRACER,
        targets: at_trade_slot(),
    })
    .await;
    // Not cast yet: it waits in Alice's offer, and Bob sees it.
    assert_eq!(cast_result(&mut a, ENCHANT_BRACER).await, Some(0x17));
    b.wait_for("Alice's spell in her offer", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS_EXTENDED(t) => {
            (t.self_player && t.spell_on_lowest_slot == ENCHANT_BRACER).then_some(())
        }
        _ => None,
    })
    .await;
    assert_eq!(
        item_field(&realm, gb, bracer, ITEM_FIELD_ENCHANTMENT).await,
        Some(0)
    );
    both_accept(&mut a, &mut b).await;
    completed(&mut b).await;
    assert_eq!(cast_result(&mut a, ENCHANT_BRACER).await, None);
    // Bob keeps his bracer, enchanted; Alice's dust is spent.
    assert_eq!(
        item_field(&realm, gb, bracer, ITEM_FIELD_ENCHANTMENT).await,
        Some(HEALTH_5)
    );
    let dust = on_map(&realm, move |m| m.players[&ga].count_of(DUST)).await;
    assert_eq!(dust, 0);
}

#[tokio::test]
async fn a_trade_spell_is_checked_again_at_the_accept_and_goes_with_its_item() {
    let realm = players_realm();
    let (mut a, mut b) = two(&realm).await;
    let (ga, gb) = (a.guid, b.guid);
    on_map(&realm, move |m| {
        m.players.get_mut(&ga).unwrap().spells.push(ENCHANT_BRACER)
    })
    .await;
    let (_, dust_at) = give(&realm, ga, DUST, 1).await;
    let (bracer, at) = give(&realm, gb, BRACER, 1).await;
    trading(&mut a, &mut b, at).await;
    let (_, other_at) = give(&realm, gb, BRACER, 1).await;
    a.send(CMSG_CAST_SPELL {
        spell: ENCHANT_BRACER,
        targets: at_trade_slot(),
    })
    .await;
    assert_eq!(cast_result(&mut a, ENCHANT_BRACER).await, Some(0x17));
    // Alice offers her only dust: at the accept it isn't hers to spend.
    a.send(CMSG_SET_TRADE_ITEM {
        trade_slot: 0,
        bag: dust_at.0,
        slot: dust_at.1,
    })
    .await;
    b.wait_for("the dust offered", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS_EXTENDED(t) => {
            (t.trade_slots[0].item == DUST).then_some(())
        }
        _ => None,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    a.clear();
    b.clear();
    both_accept(&mut a, &mut b).await;
    assert_eq!(cast_result(&mut a, ENCHANT_BRACER).await, Some(0x5c));
    b.wait_for("back to trading", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::BackToTrade).then_some(())
        }
        _ => None,
    })
    .await;
    assert!(
        b.gets(NONE_WITHIN, |m| match m {
            ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
                matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::TradeComplete).then_some(())
            }
            _ => None,
        })
        .await
        .is_none(),
        "the trade stays open"
    );
    let spell = on_map(&realm, move |m| {
        m.players[&ga].trade.as_ref().and_then(|t| t.spell)
    })
    .await;
    assert_eq!(spell, None, "the spell goes from Alice's offer");
    assert_eq!(
        item_field(&realm, gb, bracer, ITEM_FIELD_ENCHANTMENT).await,
        Some(0)
    );
    // Cast again, then Bob shows another item: the spell goes.
    a.send(CMSG_CLEAR_TRADE_ITEM { trade_slot: 0 }).await;
    give(&realm, ga, DUST, 1).await;
    a.send(CMSG_CAST_SPELL {
        spell: ENCHANT_BRACER,
        targets: at_trade_slot(),
    })
    .await;
    assert_eq!(cast_result(&mut a, ENCHANT_BRACER).await, Some(0x17));
    b.send(CMSG_SET_TRADE_ITEM {
        trade_slot: 6,
        bag: other_at.0,
        slot: other_at.1,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let spell = on_map(&realm, move |m| {
        m.players[&ga].trade.as_ref().and_then(|t| t.spell)
    })
    .await;
    assert_eq!(spell, None);
}

#[tokio::test]
async fn a_rogue_picks_the_lock_of_a_box_their_partner_keeps() {
    let realm = players_realm();
    let (mut a, mut b) = two(&realm).await;
    let (ga, gb) = (a.guid, b.guid);
    on_map(&realm, move |m| {
        m.players.get_mut(&ga).unwrap().spells.push(PICK_LOCK)
    })
    .await;
    let (junkbox, at) = give(&realm, gb, JUNKBOX, 1).await;
    trading(&mut a, &mut b, at).await;
    a.send(CMSG_CAST_SPELL {
        spell: PICK_LOCK,
        targets: at_trade_slot(),
    })
    .await;
    assert_eq!(cast_result(&mut a, PICK_LOCK).await, Some(0x17));
    both_accept(&mut a, &mut b).await;
    completed(&mut a).await;
    let flags = item_field(&realm, gb, junkbox, ITEM_FIELD_FLAGS)
        .await
        .expect("Bob keeps it");
    assert_ne!(flags & 0x4, 0, "unlocked");
    // Open now: picking it again is refused.
    let (_, at) = on_map(&realm, move |m| {
        let p = &m.players[&gb];
        let i = p.item_by_guid(junkbox).unwrap();
        (junkbox, (p.items[i].bag, p.items[i].slot))
    })
    .await;
    a.clear();
    trading(&mut a, &mut b, at).await;
    a.send(CMSG_CAST_SPELL {
        spell: PICK_LOCK,
        targets: at_trade_slot(),
    })
    .await;
    assert_eq!(cast_result(&mut a, PICK_LOCK).await, Some(0x06));
}

#[test]
fn deaths_close_together_lengthen_the_wait_and_it_fades() {
    let minute = 60_000;
    let mut until = 0;
    let mut die = |at: i64| {
        until = corpses::after_death(until, at);
        until
    };
    let u = die(0);
    assert_eq!(corpses::delay_ms(u, 1_000), 30_000);
    let u = die(minute);
    assert_eq!(corpses::delay_ms(u, minute + 1_000), 60_000);
    let u = die(2 * minute);
    assert_eq!(corpses::delay_ms(u, 2 * minute + 1_000), 120_000);
    let u = die(3 * minute);
    assert_eq!(
        corpses::delay_ms(u, 3 * minute + 1_000),
        120_000,
        "two minutes at most"
    );
    // Five minutes take a step off; long enough, all of it.
    assert_eq!(corpses::delay_ms(u, 3 * minute + 6 * minute), 60_000);
    assert_eq!(corpses::delay_ms(u, 3 * minute + 30 * minute), 30_000);
    assert_eq!(corpses::delay_ms(0, 5), 30_000);
}

/// Die, lie a moment, and release: the wait the client is told, in
/// milliseconds.
async fn released_delay(a: &mut Client) -> Duration {
    a.say(".die").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    a.send(CMSG_REPOP_REQUEST {}).await;
    let b = a
        .wait_frame(
            "SMSG_CORPSE_RECLAIM_DELAY",
            corpses::SMSG_CORPSE_RECLAIM_DELAY,
            |_| true,
        )
        .await;
    Duration::from_millis(u64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])))
}

#[tokio::test]
async fn the_wait_to_reclaim_grows_from_30_to_60_to_120_seconds() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let near = |d: Duration, s: u64| d <= Duration::from_secs(s) && d > Duration::from_secs(s - 2);
    for secs in [30, 60, 120] {
        let d = released_delay(&mut a).await;
        assert!(near(d, secs), "{d:?} for {secs}");
        if secs < 120 {
            a.say(".revive").await;
            for _ in 0..100 {
                if on_map(&realm, move |m| m.players[&ga].death == Death::Alive).await {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    }
    // The server holds to it: a minute on isn't enough, two are.
    let (early, late) = on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.death = Death::Ghost {
            since: Instant::now() - Duration::from_secs(61),
        };
        let early = corpses::ready(p, Instant::now());
        p.death = Death::Ghost {
            since: Instant::now() - Duration::from_secs(121),
        };
        (early, corpses::ready(p, Instant::now()))
    })
    .await;
    assert!(!early && late);
}

/// Alice a ghost on map 0, her corpse in the Deadmines.
async fn ghost_of_the_deadmines(realm: &Arc<Realm>, g: Guid) {
    on_map(realm, move |m| {
        crate::game::map::death::kill(m, g);
        let now = m.now;
        let p = m.players.get_mut(&g).unwrap();
        p.row.state.corpse = Some(crate::game::store::CorpseState {
            map: DEADMINES,
            x: -16.4,
            y: -383.07,
            z: 61.78,
            orientation: 0.0,
        });
        p.row.state.released_at = corpses::unix_ms();
        crate::game::map::death::become_ghost(p, now);
    })
    .await;
}

#[tokio::test]
async fn a_dungeons_corpse_is_shown_at_its_entrance_and_only_its_portal_takes_the_ghost() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    ghost_of_the_deadmines(&realm, ga).await;
    a.send(MSG_CORPSE_QUERY_Client {}).await;
    let (shown, at, corpse_map) = a
        .wait_for("the corpse", |m| match m {
            ServerOpcodeMessage::MSG_CORPSE_QUERY(q) => match &q.result {
                MSG_CORPSE_QUERY_Server_CorpseQueryResult::Found {
                    map,
                    position,
                    corpse_map,
                } => Some((*map, *position, *corpse_map)),
                _ => None,
            },
            _ => None,
        })
        .await;
    assert_eq!(shown, crate::game::map::movement::map_enum(0));
    assert_eq!((at.x, at.y), DEADMINES_OUTSIDE);
    assert_eq!(corpse_map, crate::game::map::movement::map_enum(DEADMINES));
    // The Stockade turns her away.
    a.send(CMSG_AREATRIGGER {
        trigger_id: STOCKADE_PORTAL,
    })
    .await;
    let told = a
        .wait_for("SMSG_AREA_TRIGGER_MESSAGE", |m| match m {
            ServerOpcodeMessage::SMSG_AREA_TRIGGER_MESSAGE(t) => Some(t.message.clone()),
            _ => None,
        })
        .await;
    assert_eq!(told, "You cannot enter The Stockade while in ghost form.");
    assert!(
        a.gets(NONE_WITHIN, |m| matches!(
            m,
            ServerOpcodeMessage::SMSG_NEW_WORLD(_)
        )
        .then_some(()))
            .await
            .is_none()
    );
    // The Deadmines' portal brings her back to life and in.
    a.send(CMSG_AREATRIGGER {
        trigger_id: DEADMINES_PORTAL,
    })
    .await;
    let to = a
        .wait_for("SMSG_NEW_WORLD", |m| match m {
            ServerOpcodeMessage::SMSG_NEW_WORLD(w) => Some(w.map),
            _ => None,
        })
        .await;
    assert_eq!(to, crate::game::map::movement::map_enum(DEADMINES));
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(ga).await;
    let inside = realm.registry.online(ga).unwrap().map();
    let (alive, corpse, health, max) = inside
        .query(move |m| {
            let p = &m.players[&ga];
            (
                p.death == Death::Alive,
                p.row.state.corpse.is_some(),
                p.fields.get_u32(UNIT_FIELD_HEALTH),
                p.fields.get_u32(UNIT_FIELD_MAXHEALTH),
            )
        })
        .await
        .unwrap();
    assert!(alive && !corpse);
    assert_eq!(health, max / 2);
}

#[tokio::test]
async fn an_item_bound_to_another_map_goes_but_not_from_the_dead_or_the_bank() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    // Dead, she keeps it.
    a.say(".die").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (beacon, _) = give(&realm, ga, BEACON, 1).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        item_field(&realm, ga, beacon, ITEM_FIELD_FLAGS)
            .await
            .is_some()
    );
    // Alive on map 0, it goes.
    a.clear();
    a.say(".revive").await;
    a.wait_for("the beacon destroyed", |m| match m {
        ServerOpcodeMessage::SMSG_DESTROY_OBJECT(d) => (d.guid == beacon).then_some(()),
        _ => None,
    })
    .await;
    assert!(
        item_field(&realm, ga, beacon, ITEM_FIELD_FLAGS)
            .await
            .is_none()
    );
    // In the bank, only a login's look finds it.
    let (banked, _) = give(&realm, ga, BEACON, 1).await;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        let i = p.item_by_guid(banked).unwrap();
        p.items[i].bag = BAG_PLAYER;
        p.items[i].slot = crate::game::map::inventory::BANK_START;
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        item_field(&realm, ga, banked, ITEM_FIELD_FLAGS)
            .await
            .is_some()
    );
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.misc.items_login_checked = false;
        p.misc.items_checked_at = None;
        crate::game::map::item_rules::limit_location(m, ga);
    })
    .await;
    assert!(
        item_field(&realm, ga, banked, ITEM_FIELD_FLAGS)
            .await
            .is_none()
    );
}

#[test]
fn a_zone_bound_item_is_allowed_only_in_its_zone() {
    use crate::game::map::item_rules::allowed;
    assert!(allowed(0, 0, 0, None));
    assert!(allowed(30, 0, 30, Some(2597)));
    assert!(!allowed(30, 0, 0, Some(2597)));
    assert!(allowed(0, 2597, 30, Some(2597)));
    assert!(!allowed(0, 2597, 30, Some(12)));
    assert!(
        allowed(0, 2597, 30, None),
        "a zone not known isn't held against it"
    );
}

#[tokio::test]
async fn a_zone_bound_item_goes_outside_its_zone_by_the_terrain() {
    let dir = crate::game::harness::real_data("terrain");
    if !dir.is_dir() {
        return;
    }
    let mut d = data();
    for (id, zone, bit) in [(12, 0, 126), (87, 12, 548), (1, 0, 119)] {
        d.areas.insert(
            id,
            AreaInfo {
                id,
                map: 0,
                zone,
                explore_bit: bit,
                ..Default::default()
            },
        );
    }
    let mut charm_elsewhere = item(ELWYNN_CHARM + 100, "Dun Morogh Charm");
    charm_elsewhere.area_bound = 1;
    d.items.insert(charm_elsewhere.entry, charm_elsewhere);
    let mut cfg = harness::test_config();
    cfg.terrain_dir = Some(dir);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = Realm::new(cfg, store, startup, d, None);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    on_map(&realm, move |m| {
        m.players.get_mut(&ga).unwrap().movement.position = Vector3d {
            x: -9460.0,
            y: 62.0,
            z: 56.0,
        };
    })
    .await;
    let (charm, _) = give(&realm, ga, ELWYNN_CHARM, 1).await;
    let (other, _) = give(&realm, ga, ELWYNN_CHARM + 100, 1).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        item_field(&realm, ga, charm, ITEM_FIELD_FLAGS)
            .await
            .is_some(),
        "in Elwynn"
    );
    assert!(
        item_field(&realm, ga, other, ITEM_FIELD_FLAGS)
            .await
            .is_none(),
        "not in Dun Morogh"
    );
}

#[tokio::test]
async fn an_item_for_a_creature_is_used_only_on_one_alive() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let boar = guid::unit(BOAR, 30);
    a.sees(boar).await;
    let (_, at) = give(&realm, ga, TORCH, 1).await;
    let at_unit = |unit: Guid| SpellCastTargets {
        target_flags: SpellCastTargets_SpellCastTargetFlags::empty()
            .set_unit(SpellCastTargets_SpellCastTargetFlags_Unit { unit_target: unit }),
    };
    let use_on = |targets: SpellCastTargets| CMSG_USE_ITEM {
        bag_index: at.0,
        bag_slot: at.1,
        spell_index: 0,
        targets,
    };
    a.send(use_on(at_unit(ga))).await;
    assert_eq!(
        cast_result(&mut a, LIGHT_TORCH).await,
        Some(0x0a),
        "not on herself"
    );
    a.clear();
    a.send(use_on(at_unit(boar))).await;
    let r = cast_result(&mut a, LIGHT_TORCH).await;
    assert_ne!(r, Some(0x0a), "on the boar");
    on_map(&realm, move |m| crate::game::map::combat::kill(m, ga, boar)).await;
    a.clear();
    a.send(use_on(at_unit(boar))).await;
    assert_eq!(
        cast_result(&mut a, LIGHT_TORCH).await,
        Some(0x0a),
        "not on it dead"
    );
}

#[tokio::test]
async fn a_timed_item_tells_its_time() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let (hourglass, _) = give(&realm, ga, HOURGLASS, 1).await;
    let left = a
        .wait_for("SMSG_ITEM_TIME_UPDATE", |m| match m {
            ServerOpcodeMessage::SMSG_ITEM_TIME_UPDATE(t) if t.guid == hourglass => {
                Some(t.duration)
            }
            _ => None,
        })
        .await;
    assert!((3590..=3600).contains(&left), "{left}");
    assert!(
        a.gets(Duration::from_millis(1500), |m| match m {
            ServerOpcodeMessage::SMSG_ITEM_TIME_UPDATE(t) if t.guid == hourglass => Some(()),
            _ => None,
        })
        .await
        .is_none(),
        "told once"
    );
}
