//! Bugs found while mapping wvs's tests against Thistle Tea's, batch 3:
//! the world, visibility, spawns and the wire layouts mangos-zero gives
//! the 1.12 client.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle};

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

async fn enter(
    realm: &Arc<Realm>,
    account: &str,
    name: &str,
    race: Race,
    class: Class,
) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, race, class).await;
    let map = realm.registry.online(g).unwrap().map();
    (c, g, map)
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

const SMSG_LOG_XPGAIN: u16 = 0x1D0;

/// A kill's experience names its victim and goes on with the experience
/// before the rested bonus and the group's rate; a quest's or an
/// explorer's ends at its type (mangos-zero's `SendLogXPGain`).
#[tokio::test]
async fn experience_for_a_kill_carries_its_rested_split() {
    let realm = harness::realm();
    let (mut a, g, map) = enter(&realm, "A", "Killer", Race::Human, Class::Warrior).await;
    let victim = guid::unit(299, 7);
    on(&map, move |m| {
        crate::game::map::stats::give_xp(m, g, 40, Some(victim));
    })
    .await;
    let kill = a
        .wait_frame("the kill's experience", SMSG_LOG_XPGAIN, |_| true)
        .await;
    assert_eq!(kill.len(), 21, "guid, total, type, base and rate");
    assert_eq!(&kill[..8], &victim.guid().to_le_bytes());
    assert_eq!(u32_at(&kill, 8), 40);
    assert_eq!(kill[12], 0, "a kill");
    assert_eq!(u32_at(&kill, 13), 40, "none of it rested");
    assert_eq!(f32::from_le_bytes(kill[17..21].try_into().unwrap()), 1.0);
    on(&map, move |m| {
        crate::game::map::stats::give_xp(m, g, 25, None)
    })
    .await;
    let quest = a
        .wait_frame("the quest's experience", SMSG_LOG_XPGAIN, |_| true)
        .await;
    assert_eq!(quest.len(), 13, "guid, total and type");
    assert_eq!(u32_at(&quest, 8), 25);
    assert_eq!(quest[12], 1, "not a kill");
}

const RES: u32 = 900_300;
const REBIRTH: u32 = 900_301;
const SMSG_RESURRECT_REQUEST: u16 = 0x15B;

fn unit_spell(id: u32, effect: u32) -> spells::SpellTemplate {
    let mut s = spells::SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    s.effects[0] = spells::SpellEffect {
        effect,
        target_a: 25,
        base_points: 34,
        ..Default::default()
    };
    s
}

/// A resurrection's offer is laid out as mangos-zero's: no name for a
/// player caster, not a spirit healer, and whether the client holds the
/// acceptance back by its timer, which Rebirth's attribute skips.
#[tokio::test]
async fn a_resurrection_offer_says_whether_it_waits_on_the_timer() {
    let mut d = GameData::empty();
    d.spells.ranges.insert(1, (0.0, 30.0));
    d.spells.spells.insert(RES, unit_spell(RES, 18));
    let mut rebirth = unit_spell(REBIRTH, 18);
    rebirth.attributes[3] = spells::ATTR_EX3_NO_RES_TIMER;
    d.spells.spells.insert(REBIRTH, rebirth);
    let realm = harness::realm_with(d);
    let (_a, priest, map) = enter(&realm, "A", "Priest", Race::Human, Class::Priest).await;
    let (mut b, fallen, _) = enter(&realm, "B", "Fallen", Race::Human, Class::Warrior).await;
    on(&map, move |m| {
        crate::game::map::combat::deal_damage(m, priest, fallen, 1_000_000);
        crate::game::map::spells::cast_triggered(m, priest, RES, Some(fallen));
    })
    .await;
    let offer = b
        .wait_frame("the offer", SMSG_RESURRECT_REQUEST, |_| true)
        .await;
    let mut want = priest.guid().to_le_bytes().to_vec();
    want.extend([1, 0, 0, 0, 0]);
    want.extend([0, 1]);
    assert_eq!(offer, want, "no name, not a spirit healer, on the timer");
    on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, priest, REBIRTH, Some(fallen));
    })
    .await;
    let offer = b
        .wait_frame("Rebirth's offer", SMSG_RESURRECT_REQUEST, |_| true)
        .await;
    assert_eq!(offer[offer.len() - 2..], [0, 0], "at once");
}

const SHIELD_BASH: u32 = 900_310;
const SMSG_CAST_RESULT: u16 = 0x130;

/// A cast refused for the gear it wants names the gear: its class, its
/// subclasses and its inventory types, as mangos-zero sends them and
/// wow_messages reads them.
#[tokio::test]
async fn a_cast_refused_for_its_gear_names_the_gear() {
    let mut d = GameData::empty();
    d.spells.ranges.insert(1, (0.0, 30.0));
    let mut bash = unit_spell(SHIELD_BASH, 2);
    bash.effects[0].target_a = 1;
    bash.equipped_item_class = 4;
    bash.equipped_item_subclass_mask = 1 << 6;
    bash.equipped_item_inventory_type_mask = 1 << 14;
    d.spells.spells.insert(SHIELD_BASH, bash);
    let realm = harness::realm_with(d);
    let (mut a, g, map) = enter(&realm, "A", "Bare", Race::Human, Class::Warrior).await;
    on(&map, move |m| {
        crate::game::map::spells::learn(m, g, SHIELD_BASH)
    })
    .await;
    a.send(CMSG_CAST_SPELL {
        spell: SHIELD_BASH,
        targets: SpellCastTargets {
            target_flags: SpellCastTargets_SpellCastTargetFlags::empty(),
        },
    })
    .await;
    let refused = a
        .wait_frame("the refusal", SMSG_CAST_RESULT, |b| b.len() > 5)
        .await;
    let mut want = SHIELD_BASH.to_le_bytes().to_vec();
    want.extend([2, crate::game::map::spells::fail::EQUIPPED_ITEM_CLASS]);
    want.extend(4u32.to_le_bytes());
    want.extend((1u32 << 6).to_le_bytes());
    want.extend((1u32 << 14).to_le_bytes());
    assert_eq!(refused, want, "a shield, in the off hand");
    let parsed = a
        .wait_for("the refusal, read", |m| match m {
            ServerOpcodeMessage::SMSG_CAST_RESULT(r) => Some(r.clone()),
            _ => None,
        })
        .await;
    assert!(matches!(
        parsed.result,
        SMSG_CAST_RESULT_SimpleSpellCastResult::Failure { .. }
    ));
}

/// A spawn's looks in an event are its latest patch's, whatever order
/// the rows are stored in.
#[test]
fn an_event_creatures_looks_are_its_latest_patchs() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE game_event_creature_data (guid, patch, entry_id, display_id, \
         equipment_id, spell_start, spell_end, event);
         INSERT INTO game_event_creature_data VALUES (1, 7, 0, 200, 0, 0, 0, 5);
         INSERT INTO game_event_creature_data VALUES (1, 3, 0, 100, 0, 0, 0, 5);",
    )
    .unwrap();
    let mut d = crate::game::data::world::WorldData::default();
    crate::game::data::world::load(&db, None, &mut d);
    let looks = &d.event_creatures[&1];
    assert_eq!(looks.len(), 1);
    assert_eq!(looks[0].display, 200, "patch 7's, not patch 3's");
}

async fn group(leader: &mut Client, member: &mut Client, name: &str) {
    leader.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    member
        .wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
    member.send(CMSG_GROUP_ACCEPT {}).await;
    leader
        .wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
            _ => None,
        })
        .await;
}

async fn leads(map: &MapHandle, g: Guid) -> bool {
    on(map, move |m| {
        m.players[&g].fields.has_flag(
            PLAYER_FLAGS,
            crate::game::map::player::PLAYER_FLAGS_GROUP_LEADER,
        )
    })
    .await
}

/// The group's leader carries `PLAYER_FLAGS_GROUP_LEADER`, handed on with
/// the lead and gone with the group (mangos's `UpdateGroupLeaderFlag`).
#[tokio::test]
async fn the_group_leader_is_flagged_as_such() {
    let realm = harness::realm();
    let (mut a, ga, map) = enter(&realm, "A", "Ana", Race::Human, Class::Warrior).await;
    let (mut b, gb, _) = enter(&realm, "B", "Bea", Race::Human, Class::Warrior).await;
    assert!(!leads(&map, ga).await);
    group(&mut a, &mut b, "Bea").await;
    assert!(leads(&map, ga).await, "the one who invited leads");
    assert!(!leads(&map, gb).await);
    a.send(CMSG_GROUP_SET_LEADER { guid: gb }).await;
    a.wait_for("SMSG_GROUP_SET_LEADER", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_SET_LEADER(_)).then_some(())
    })
    .await;
    b.wait_for("the new roster", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) if l.leader == gb => Some(()),
        _ => None,
    })
    .await;
    assert!(!leads(&map, ga).await, "the lead handed on");
    assert!(leads(&map, gb).await);
    b.send(CMSG_GROUP_DISBAND {}).await;
    b.wait_for("the empty roster", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) if l.members.is_empty() => Some(()),
        _ => None,
    })
    .await;
    assert!(!leads(&map, gb).await, "no group, no lead");
}

const BOAT: u32 = 9999;

/// A boat that sails from the start on Eastern Kingdoms, crosses to
/// Kalimdor and back, every 1.6 seconds.
fn boat_data() -> GameData {
    let mut d = GameData::empty();
    let mut data = [0u32; 24];
    data[0] = 77;
    data[1] = 1000;
    data[2] = 1000;
    d.gameobject_templates.insert(
        BOAT,
        GameObjectTemplate {
            entry: BOAT,
            kind: 15,
            display_id: 3015,
            name: "The Test Boat".into(),
            size: 1.0,
            data,
            ..Default::default()
        },
    );
    d.transports.push(TransportRoute {
        entry: BOAT,
        name: "Here and There".into(),
        period: 1600,
    });
    let node = |map, x: f32, y: f32| crate::game::data::npc::TaxiPathNode {
        map,
        x,
        y,
        z: START.2,
        flags: 0,
        delay: 0,
    };
    d.npc.taxi_path_nodes.insert(
        77,
        vec![
            node(0, START.0, START.1),
            node(0, START.0 + 30.0, START.1),
            node(0, START.0 + 60.0, START.1),
            node(0, START.0 + 100.0, START.1),
            node(1, 990.0, 1000.0),
            node(1, 1000.0, 1000.0),
            node(1, 1010.0, 1000.0),
            node(1, 1020.0, 1000.0),
        ],
    );
    d
}

/// The update that makes a boat says it has a transport, as mangos's
/// `SendInitTransports` and Thistle Tea's header do; one without says not.
#[tokio::test]
async fn an_update_making_a_boat_says_it_has_a_transport() {
    let realm = harness::realm_with(boat_data());
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Sailor", Race::Human, Class::Warrior).await;
    let boat = guid::make(guid::HighGuid::MoTransport, 0, BOAT);
    let header = |m: &ServerOpcodeMessage, of: Guid| {
        let (has, objects) = match m {
            ServerOpcodeMessage::SMSG_UPDATE_OBJECT(u) => (u.has_transport, &u.objects),
            ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(u) => (u.has_transport, &u.objects),
            _ => return None,
        };
        objects
            .iter()
            .any(|o| match &o.update_type {
                Object_UpdateType::CreateObject { guid3, .. }
                | Object_UpdateType::CreateObject2 { guid3, .. } => *guid3 == of,
                _ => false,
            })
            .then_some(has)
    };
    let mine = a.wait_for("my own create", |m| header(m, me)).await;
    assert_eq!(mine, 0, "no boat in it");
    let boats = a.wait_for("the boat's create", |m| header(m, boat)).await;
    assert_eq!(boats, 1, "a transport in it");
}

const BLADE: u32 = 900_320;
const AUCTIONEER: u32 = 900_321;
/// "of the Bear", and Crusader on the blade.
const BEAR: u32 = 1201;
const CRUSADER: u32 = 1900;

/// A sword with three charges of its spell, and a goblin auctioneer by the
/// start.
fn market_data() -> GameData {
    let mut d = GameData::empty();
    let mut blade = ItemTemplate {
        entry: BLADE,
        name: "Charged Blade".into(),
        class: 2,
        display_id: 1000,
        quality: 2,
        inventory_type: 13,
        allowable_class: -1,
        allowable_race: -1,
        stackable: 1,
        ..Default::default()
    };
    blade.spells[0].id = 133;
    blade.spells[0].charges = 3;
    d.items.insert(BLADE, blade);
    d.creature_templates.insert(
        AUCTIONEER,
        CreatureTemplate {
            entry: AUCTIONEER,
            name: "Auctioneer".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 10,
            level_max: 10,
            faction: 35,
            unit_class: 1,
            health_max: 100,
            npc_flags: crate::game::map::npc::NPC_FLAG_AUCTIONEER,
            ..Default::default()
        },
    );
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: 41,
        entries: vec![AUCTIONEER],
        map: 0,
        x: START.0 + 2.0,
        y: START.1,
        z: START.2,
        ..Default::default()
    });
    d
}

/// An item's random property, permanent enchantment and charges are told
/// as it is pushed into the bags, listed at the auction house and in the
/// mailbox (mangos-zero's `SendNewItem`, `BuildAuctionInfo` and mail list),
/// so the client names it by its suffix.
#[tokio::test]
async fn an_items_suffix_enchantment_and_charges_are_shown() {
    use crate::game::social::{auction::Auction, mail::Mail};
    let realm = harness::realm_with(market_data());
    let (mut a, g, map) = enter(&realm, "A", "Trader", Race::Human, Class::Warrior).await;
    on(&map, move |m| {
        crate::game::map::inventory::add_item_with_property(m, g, BLADE, 1, true, BEAR).unwrap();
    })
    .await;
    let pushed = a
        .wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == BLADE => {
                Some(r.item_random_property_id)
            }
            _ => None,
        })
        .await;
    assert_eq!(pushed, BEAR, "pushed with its suffix");
    let row = crate::game::store::ItemRow {
        guid: 77,
        entry: BLADE,
        count: 1,
        data: format!("rp:{BEAR};e0:{CRUSADER}/0/0"),
        ..Default::default()
    };
    realm.social.auctions.lock().auctions.insert(
        5,
        Auction {
            id: 5,
            market: 7,
            house: 7,
            item: row.clone(),
            owner: 999,
            start_bid: 10,
            expires_at: chrono::Utc::now().timestamp() + 3600,
            ..Default::default()
        },
    );
    let auctioneer = on(&map, |m| {
        m.creatures
            .iter()
            .find(|(_, c)| c.entry == AUCTIONEER)
            .map(|(_, c)| c.guid)
    })
    .await
    .expect("the auctioneer");
    a.send(CMSG_AUCTION_LIST_ITEMS {
        auctioneer,
        list_start_item: 0,
        searched_name: String::new(),
        minimum_level: 0,
        maximum_level: 0,
        auction_slot_id: u32::MAX,
        auction_main_category: u32::MAX,
        auction_sub_category: u32::MAX,
        auction_quality: ItemQuality::Poor,
        usable: 0,
    })
    .await;
    let listed = a
        .wait_for("SMSG_AUCTION_LIST_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_AUCTION_LIST_RESULT(l) => l.auctions.first().map(|i| {
                (
                    i.item_enchantment,
                    i.item_random_property_id,
                    i.item_charges,
                )
            }),
            _ => None,
        })
        .await;
    assert_eq!(listed, (CRUSADER, BEAR, 3), "at the auction house");
    crate::game::social::mail::post(
        &realm,
        Mail {
            receiver: guid::counter(g),
            subject: "A blade".into(),
            item: Some(row),
            expires_at: chrono::Utc::now().timestamp() + 3600,
            ..Default::default()
        },
    );
    a.send(CMSG_GET_MAIL_LIST {
        mailbox: Guid::zero(),
    })
    .await;
    let mailed = a
        .wait_for("SMSG_MAIL_LIST_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_MAIL_LIST_RESULT(l) => l.mails.first().map(|m| {
                (
                    m.item_enchant_id,
                    m.item_random_property_id,
                    m.item_spell_charges,
                )
            }),
            _ => None,
        })
        .await;
    assert_eq!(mailed, (CRUSADER, BEAR, 3), "in the mailbox");
}
