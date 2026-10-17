//! Bugs found while mapping wvs's tests against Thistle Tea's, batch 3:
//! the world, visibility, spawns and the wire layouts mangos-zero gives
//! the 1.12 client.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::data::{scripts, spells};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::spells as spells_effect;
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

/// A group's roster ends with the byte Thistle Tea and mangos-classic send
/// past the loot rules; an empty roster ends at its leader.
#[tokio::test]
async fn a_groups_roster_ends_with_its_difficulty_byte() {
    let realm = harness::realm();
    let (mut a, _, _) = enter(&realm, "A", "Ana", Race::Human, Class::Warrior).await;
    let (mut b, gb, _) = enter(&realm, "B", "Bea", Race::Human, Class::Warrior).await;
    a.clear();
    group(&mut a, &mut b, "Bea").await;
    let roster = a
        .wait_frame("the roster", 0x07D, |b| u32_at(b, 2) == 1)
        .await;
    // Type, flags, one member ("Bea", guid, online, flags), the leader,
    // the loot rules and the last byte.
    assert_eq!(
        roster.len(),
        1 + 1 + 4 + (4 + 8 + 1 + 1) + 8 + (1 + 8 + 1) + 1
    );
    assert_eq!(roster[roster.len() - 1], 0);
    assert_eq!(&roster[6..10], b"Bea\0");
    assert_eq!(u32_at(&roster, 10), gb.guid() as u32);
}

/// What is said carries 25 yards in three dimensions, as Thistle Tea's
/// `nearby_players` and mangos's `MessageDistDeliverer` measure it: not to
/// one 30 yards overhead.
#[tokio::test]
async fn a_say_carries_in_three_dimensions() {
    let realm = harness::realm();
    let (a, ga, map) = enter(&realm, "A", "Low", Race::Human, Class::Warrior).await;
    let (mut b, gb, _) = enter(&realm, "B", "High", Race::Human, Class::Warrior).await;
    let place = move |m: &mut Map, g: Guid, dz: f32| {
        if let Some(p) = m.players.get_mut(&g) {
            p.movement.position = Vector3d {
                x: START.0 + 5.0,
                y: START.1,
                z: START.2 + dz,
            };
        }
    };
    on(&map, move |m| {
        if let Some(p) = m.players.get_mut(&ga) {
            p.movement.position = Vector3d {
                x: START.0,
                y: START.1,
                z: START.2,
            };
        }
        place(m, gb, 30.0);
    })
    .await;
    b.clear();
    a.say("up there?").await;
    let heard = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c) if c.message.contains("up there") => Some(()),
        _ => None,
    };
    assert!(
        b.gets(Duration::from_millis(400), heard).await.is_none(),
        "30 yards up is out of earshot"
    );
    on(&map, move |m| place(m, gb, 0.0)).await;
    a.say("down here").await;
    b.wait_for("the say", |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c) if c.message.contains("down here") => Some(()),
        _ => None,
    })
    .await;
}

const COMBO: u32 = 900_330;
const WOLF: u32 = 900_331;

/// Spent combo points leave `PLAYER_FIELD_COMBO_TARGET` naming the target
/// they were on, the points at 0, as mangos's `ClearComboPoints` and
/// Thistle Tea's `put_points` leave it.
#[tokio::test]
async fn spent_combo_points_keep_their_target() {
    let mut d = GameData::empty();
    d.spells.ranges.insert(1, (0.0, 30.0));
    d.spells.spells.insert(
        COMBO,
        unit_spell(COMBO, spells_effect::EFFECT_ADD_COMBO_POINTS),
    );
    d.creature_templates.insert(
        WOLF,
        CreatureTemplate {
            entry: WOLF,
            name: "Wolf".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 1,
            level_max: 1,
            faction: 7,
            unit_class: 1,
            health_max: 100,
            ..Default::default()
        },
    );
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: 42,
        entries: vec![WOLF],
        map: 0,
        x: START.0 + 3.0,
        y: START.1,
        z: START.2,
        ..Default::default()
    });
    let realm = harness::realm_with(d);
    let (_a, g, map) = enter(&realm, "A", "Rogue", Race::Human, Class::Rogue).await;
    let target = on(&map, |m| {
        m.creatures
            .iter()
            .find(|(_, c)| c.entry == WOLF)
            .map(|(_, c)| c.guid)
    })
    .await
    .expect("the wolf");
    let (field, points) = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, g, COMBO, Some(target));
        assert!(m.players[&g].fields.get_byte(PLAYER_FIELD_BYTES, 1) > 0);
        crate::game::map::spells::clear_combo_points(m, g);
        let f = &m.players[&g].fields;
        (
            f.get_guid(PLAYER_FIELD_COMBO_TARGET),
            f.get_byte(PLAYER_FIELD_BYTES, 1),
        )
    })
    .await;
    assert_eq!(points, 0, "spent");
    assert_eq!(field, target, "the target they were on");
}

const SHAMAN: u32 = 900_340;
const TOTEM: u32 = 900_341;
const TOTEM_SPELL: u32 = 900_342;

fn plain_creature(entry: u32, name: &str, faction: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 10,
        level_max: 10,
        faction,
        unit_class: 1,
        health_max: 500,
        ..Default::default()
    }
}

/// A totem is under a player's control only if its owner is: a creature's
/// has no `UNIT_FLAG_PLAYER_CONTROLLED` (mangos copies the caster's flag;
/// Thistle Tea's `attach_owner` sets it for a player's alone).
#[tokio::test]
async fn a_creatures_totem_is_not_player_controlled() {
    let mut d = GameData::empty();
    d.spells.ranges.insert(1, (0.0, 30.0));
    let mut totem_spell = unit_spell(TOTEM_SPELL, crate::game::map::pets::EFFECT_SUMMON_TOTEM);
    totem_spell.effects[0].target_a = 1;
    totem_spell.effects[0].misc_value = TOTEM as i32;
    d.spells.spells.insert(TOTEM_SPELL, totem_spell);
    d.creature_templates
        .insert(SHAMAN, plain_creature(SHAMAN, "Shaman", 35));
    d.creature_templates
        .insert(TOTEM, plain_creature(TOTEM, "Totem", 35));
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: 43,
        entries: vec![SHAMAN],
        map: 0,
        x: START.0 + 4.0,
        y: START.1,
        z: START.2,
        ..Default::default()
    });
    let realm = harness::realm_with(d);
    let (_a, g, map) = enter(&realm, "A", "Player", Race::Human, Class::Warrior).await;
    let flags = on(&map, move |m| {
        let shaman = m
            .creatures
            .iter()
            .find(|(_, c)| c.entry == SHAMAN)
            .map(|(_, c)| c.guid)
            .unwrap();
        crate::game::map::spells::cast_triggered(m, shaman, TOTEM_SPELL, None);
        crate::game::map::spells::cast_triggered(m, g, TOTEM_SPELL, None);
        let mut out: Vec<(bool, u32)> = m
            .creatures
            .iter()
            .filter(|(_, c)| c.entry == TOTEM)
            .map(|(_, c)| (c.owner == Some(g), c.fields.get_u32(UNIT_FIELD_FLAGS)))
            .collect();
        out.sort();
        out
    })
    .await;
    assert_eq!(flags.len(), 2, "both totems: {flags:?}");
    assert_eq!(
        flags[0].1 & 0x8,
        0,
        "the creature's isn't player-controlled"
    );
    assert_ne!(flags[1].1 & 0x8, 0, "the player's is");
}

const BARK: u32 = 900_350;
const BARK_EFFECT: u32 = 900_351;

/// vmangos's linked aura (192, Barkskin's second effect by
/// `spell_effect_mod`) puts its spell's auras on with it and takes them off
/// with it, as Thistle Tea's `Aura.Linked` does: Barkskin's 20 percent less
/// physical damage is "Barkskin Effect (DND)"'s.
#[tokio::test]
async fn a_linked_aura_holds_its_spell_while_it_lasts() {
    let mut d = GameData::empty();
    d.spells.ranges.insert(1, (0.0, 30.0));
    let mut bark = unit_spell(BARK, 6);
    bark.effects[0].target_a = 1;
    bark.effects[0].aura = crate::game::map::auras::AURA_RESIST_PUSHBACK;
    bark.effects[0].base_points = 99;
    bark.effects[1] = spells::SpellEffect {
        effect: 6,
        target_a: 1,
        aura: crate::game::map::auras::AURA_LINKED,
        trigger_spell: BARK_EFFECT,
        ..Default::default()
    };
    bark.duration_index = 0;
    d.spells.spells.insert(BARK, bark);
    let mut effect = unit_spell(BARK_EFFECT, 6);
    effect.effects[0].target_a = 1;
    effect.effects[0].aura = crate::game::map::auras::AURA_MOD_DAMAGE_PERCENT_TAKEN;
    effect.effects[0].base_points = -21;
    effect.effects[0].misc_value = 1;
    d.spells.spells.insert(BARK_EFFECT, effect);
    let realm = harness::realm_with(d);
    let (_a, g, map) = enter(&realm, "A", "Druid", Race::NightElf, Class::Druid).await;
    let has = move |m: &Map, spell: u32| {
        crate::game::map::auras::auras(m, g)
            .is_some_and(|a| a.holders.iter().any(|h| h.spell == spell))
    };
    let (on_it, unseen) = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, g, BARK, None);
        let unseen = crate::game::map::auras::auras(m, g).is_some_and(|a| {
            a.holders
                .iter()
                .any(|h| h.spell == BARK_EFFECT && h.slot.is_none() && h.passive)
        });
        ((has(m, BARK), has(m, BARK_EFFECT)), unseen)
    })
    .await;
    assert_eq!(on_it, (true, true), "Barkskin brings its effect");
    assert!(unseen, "held unseen, with no slot of its own");
    let off = on(&map, move |m| {
        crate::game::map::auras::remove_spell(m, g, BARK, None);
        (has(m, BARK), has(m, BARK_EFFECT))
    })
    .await;
    assert_eq!(off, (false, false), "and takes it away");
}

/// The real data, read once: Barkskin (22812) links "Barkskin Effect
/// (DND)" (22839) by `spell_effect_mod`, and the server knows the aura
/// that does it; Rajaxx's captains and Loatheb's maggots have their
/// masters by `creature_linking_template`.
#[test]
fn the_real_data_links_barkskin_and_the_masters() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    // Colonel Zerran, by Rajaxx in the Ruins (the other Rajaxx is in
    // Silithus, on another map).
    assert_eq!(d.scripts.links.get(&301_170).map(|l| l.0), Some(90_871));
    // A Diseased Maggot, Loatheb's in Naxxramas.
    assert_eq!(d.scripts.links.get(&88_233).map(|l| l.0), Some(88_226));
    let bark = d.spells.get(22_812).expect("Barkskin");
    let e = bark.effects[1];
    assert_eq!(
        (e.effect, e.aura, e.trigger_spell),
        (6, crate::game::map::auras::AURA_LINKED, 22_839)
    );
    assert!(crate::game::map::auras::supported(e.aura));
    let effect = d.spells.get(22_839).expect("Barkskin Effect");
    assert_eq!(
        effect.effects[1].aura,
        crate::game::map::auras::AURA_MOD_DAMAGE_PERCENT_TAKEN
    );
}

const SHEEP: u32 = 900_360;
const MARK: u32 = 900_361;
const BOAR: u32 = 900_362;

/// A caster's single-target auras outlive the caster's death but for
/// Hunter's Mark, as in mangos (which removes none) and Thistle Tea
/// (`SingleTargetAuras.caster_died`, only the stalked); all of them end
/// when the caster leaves the world (mangos's `RemoveNotOwnTrackedTargetAuras`,
/// Thistle Tea's `SingleTarget.detach`).
#[tokio::test]
async fn single_target_auras_outlive_their_casters_death_but_not_its_going() {
    let mut d = GameData::empty();
    d.spells.ranges.insert(1, (0.0, 30.0));
    for (id, aura) in [
        (SHEEP, crate::game::map::auras::AURA_MOD_CONFUSE),
        (MARK, crate::game::map::auras::AURA_MOD_STALKED),
    ] {
        let mut s = unit_spell(id, 6);
        s.effects[0].aura = aura;
        s.custom_flags = spells::CUSTOM_SINGLE_TARGET_AURA;
        s.duration_index = 0;
        d.spells.spells.insert(id, s);
    }
    d.creature_templates
        .insert(BOAR, plain_creature(BOAR, "Boar", 7));
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: 44,
        entries: vec![BOAR],
        map: 0,
        x: START.0 + 6.0,
        y: START.1,
        z: START.2,
        ..Default::default()
    });
    let realm = harness::realm_with(d);
    let (mut a, g, map) = enter(&realm, "A", "Mage", Race::Human, Class::Mage).await;
    let boar = on(&map, |m| {
        m.creatures
            .iter()
            .find(|(_, c)| c.entry == BOAR)
            .map(|(_, c)| c.guid)
    })
    .await
    .expect("the boar");
    let held = move |m: &Map| {
        let on_boar = |s: u32| {
            crate::game::map::auras::auras(m, boar)
                .is_some_and(|a| a.holders.iter().any(|h| h.spell == s && h.caster == g))
        };
        (on_boar(SHEEP), on_boar(MARK))
    };
    let after_death = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, g, SHEEP, Some(boar));
        crate::game::map::spells::cast_triggered(m, g, MARK, Some(boar));
        assert_eq!(held(m), (true, true), "both on");
        crate::game::map::combat::deal_damage(m, boar, g, 1_000_000);
        held(m)
    })
    .await;
    assert_eq!(
        after_death,
        (true, false),
        "only the mark ends with its caster"
    );
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    assert_eq!(
        on(&map, move |m| held(m)).await,
        (false, false),
        "its caster gone"
    );
}

const SHOVE: u32 = 900_370;
const WARD: u32 = 900_371;
const BOLT: u32 = 900_372;
const OGRE: u32 = 900_373;

/// A knockback moves no creature a server runs, as in mangos (only a
/// client-controlled unit is launched) and Thistle Tea, whose launch
/// interrupts the creature's cast and leaves it where it stands; nor does
/// it end what a teleport would.
#[tokio::test]
async fn a_knockback_leaves_a_creature_in_place_and_stops_its_cast() {
    let mut d = GameData::empty();
    d.spells.ranges.insert(1, (0.0, 30.0));
    d.spells.cast_times.insert(5, 3000);
    let mut shove = unit_spell(SHOVE, 98);
    shove.effects[0].misc_value = 150;
    d.spells.spells.insert(SHOVE, shove);
    let mut ward = unit_spell(WARD, 6);
    ward.effects[0].target_a = 1;
    ward.effects[0].aura = crate::game::map::auras::AURA_MOD_RESISTANCE;
    ward.aura_interrupt_flags = crate::game::map::auras::INTERRUPT_TELEPORTED;
    ward.duration_index = 0;
    d.spells.spells.insert(WARD, ward);
    let mut bolt = unit_spell(BOLT, 2);
    bolt.casting_time_index = 5;
    d.spells.spells.insert(BOLT, bolt);
    d.creature_templates
        .insert(OGRE, plain_creature(OGRE, "Ogre", 7));
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: 45,
        entries: vec![OGRE],
        map: 0,
        x: START.0 + 5.0,
        y: START.1,
        z: START.2,
        ..Default::default()
    });
    let realm = harness::realm_with(d);
    let (_a, g, map) = enter(&realm, "A", "Shover", Race::Human, Class::Warrior).await;
    let (before, after, warded, casting) = on(&map, move |m| {
        let ogre = m
            .creatures
            .iter()
            .find(|(_, c)| c.entry == OGRE)
            .map(|(_, c)| c.guid)
            .unwrap();
        crate::game::map::spells::cast_triggered(m, ogre, WARD, None);
        let targets = crate::game::map::spells::Targets {
            unit: Some(g),
            ..Default::default()
        };
        assert!(crate::game::map::spells::cast(m, ogre, BOLT, targets, None));
        assert!(
            m.caster(ogre).is_some_and(|c| c.casting.is_some()),
            "casting"
        );
        let before = m.creature(ogre).unwrap().position;
        crate::game::map::spells::cast_triggered(m, g, SHOVE, Some(ogre));
        let warded = crate::game::map::auras::auras(m, ogre)
            .is_some_and(|a| a.holders.iter().any(|h| h.spell == WARD));
        let casting = m.caster(ogre).is_some_and(|c| c.casting.is_some());
        (before, m.creature(ogre).unwrap().position, warded, casting)
    })
    .await;
    assert_eq!((before.x, before.y), (after.x, after.y), "in place");
    assert!(warded, "nothing a teleport ends");
    assert!(!casting, "its cast interrupted");
}

const QUARTERMASTER: u32 = 900_380;
const TABARD: u32 = 900_381;
const ARGENT_DAWN: u32 = 529;

/// An item that wants a reputation is refused to one short of it with
/// `BUY_ERR_REPUTATION_REQUIRE`, as mangos's `BuyItemFromVendor` and
/// Thistle Tea's vendor refuse it, and sold once they have it.
#[tokio::test]
async fn a_vendor_wont_sell_what_wants_more_reputation() {
    use crate::game::data::misc::FactionDef;
    let mut d = GameData::empty();
    d.misc.factions.insert(
        ARGENT_DAWN,
        FactionDef {
            id: ARGENT_DAWN,
            index: 7,
            name: "Argent Dawn".into(),
            ..Default::default()
        },
    );
    let mut qm = plain_creature(QUARTERMASTER, "Quartermaster", 35);
    qm.npc_flags = crate::game::map::npc::NPC_FLAG_VENDOR;
    d.creature_templates.insert(QUARTERMASTER, qm);
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: 46,
        entries: vec![QUARTERMASTER],
        map: 0,
        x: START.0 + 2.0,
        y: START.1,
        z: START.2,
        ..Default::default()
    });
    d.vendors.insert(
        QUARTERMASTER,
        vec![VendorItem {
            slot: 1,
            item: TABARD,
            ..Default::default()
        }],
    );
    d.items.insert(
        TABARD,
        ItemTemplate {
            entry: TABARD,
            name: "Argent Tabard".into(),
            class: 4,
            display_id: 1000,
            quality: 1,
            buy_count: 1,
            buy_price: 10,
            inventory_type: 19,
            allowable_class: -1,
            allowable_race: -1,
            stackable: 1,
            required_reputation_faction: ARGENT_DAWN,
            required_reputation_rank: crate::game::data::misc::RANK_HONORED,
            ..Default::default()
        },
    );
    let realm = harness::realm_with(d);
    let (mut a, g, map) = enter(&realm, "A", "Buyer", Race::Human, Class::Warrior).await;
    let vendor = on(&map, |m| {
        m.creatures
            .iter()
            .find(|(_, c)| c.entry == QUARTERMASTER)
            .map(|(_, c)| c.guid)
    })
    .await
    .expect("the quartermaster");
    on(&map, move |m| {
        m.players.get_mut(&g).unwrap().set_money(1000)
    })
    .await;
    let buy = CMSG_BUY_ITEM {
        vendor,
        item: TABARD,
        amount: 1,
        unknown1: 0,
    };
    a.send(buy.clone()).await;
    let refused = a
        .wait_for("SMSG_BUY_FAILED", |m| match m {
            ServerOpcodeMessage::SMSG_BUY_FAILED(f) => Some(f.result),
            _ => None,
        })
        .await;
    assert_eq!(refused, BuyResult::ReputationRequire);
    on(&map, move |m| {
        crate::game::map::reputation::modify(m, g, ARGENT_DAWN, 9000, false);
    })
    .await;
    a.send(buy).await;
    a.wait_for("SMSG_BUY_ITEM", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_BUY_ITEM(_)).then_some(())
    })
    .await;
}

const SWORD: u32 = 900_390;
const POTIONS: u32 = 900_391;

async fn trade_window(a: &mut Client, b: &mut Client, gb: Guid) {
    a.send(CMSG_INITIATE_TRADE { guid: gb }).await;
    b.wait_for("the trade asked", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::BeginTrade { .. }).then_some(())
        }
        _ => None,
    })
    .await;
    b.send(CMSG_BEGIN_TRADE {}).await;
}

async fn place(map: &MapHandle, g: Guid, entry: u32) -> (u8, u8) {
    on(map, move |m| {
        let p = &m.players[&g];
        let it = p.items.iter().find(|i| i.entry == entry).unwrap();
        (it.bag, it.slot)
    })
    .await
}

/// What the trade ends in, for `c`.
async fn trade_end(c: &mut Client) -> SMSG_TRADE_STATUS_TradeStatus {
    c.wait_for("the trade's end", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s)
            if matches!(
                s.status,
                SMSG_TRADE_STATUS_TradeStatus::TradeComplete
                    | SMSG_TRADE_STATUS_TradeStatus::TradeCanceled
            ) =>
        {
            Some(s.status.clone())
        }
        _ => None,
    })
    .await
}

/// The offer is checked again as the trade goes through, as mangos's
/// `CanBeTraded` and Thistle Tea's `validate_offers` check it: an item
/// equipped after it was offered, or a stack split smaller than the
/// partner was shown, calls the trade off.
#[tokio::test]
async fn a_trade_checks_its_offer_again_as_it_goes_through() {
    let mut d = GameData::empty();
    d.items.insert(
        SWORD,
        ItemTemplate {
            entry: SWORD,
            name: "Sword".into(),
            class: 2,
            subclass: 7,
            display_id: 1000,
            quality: 1,
            inventory_type: 13,
            allowable_class: -1,
            allowable_race: -1,
            stackable: 1,
            ..Default::default()
        },
    );
    d.items.insert(
        POTIONS,
        ItemTemplate {
            entry: POTIONS,
            name: "Potion".into(),
            class: 0,
            display_id: 1001,
            quality: 1,
            allowable_class: -1,
            allowable_race: -1,
            stackable: 20,
            ..Default::default()
        },
    );
    let realm = harness::realm_with(d);
    let (mut a, ga, map) = enter(&realm, "A", "Ana", Race::Human, Class::Warrior).await;
    let (mut b, gb, _) = enter(&realm, "B", "Bea", Race::Human, Class::Warrior).await;
    a.sees(gb).await;
    on(&map, move |m| {
        crate::game::map::inventory::add_item(m, ga, SWORD, 1, false).unwrap();
        crate::game::map::inventory::add_item(m, ga, POTIONS, 20, false).unwrap();
    })
    .await;
    // The sword offered, then worn.
    trade_window(&mut a, &mut b, gb).await;
    let (bag, slot) = place(&map, ga, SWORD).await;
    a.send(CMSG_SET_TRADE_ITEM {
        trade_slot: 0,
        bag,
        slot,
    })
    .await;
    b.wait_for("the offer", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS_EXTENDED(t) => {
            (t.trade_slots[0].item == SWORD).then_some(())
        }
        _ => None,
    })
    .await;
    a.send(CMSG_AUTOEQUIP_ITEM {
        source_bag: bag,
        source_slot: slot,
    })
    .await;
    let mut worn = place(&map, ga, SWORD).await;
    for _ in 0..100 {
        if worn != (bag, slot) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        worn = place(&map, ga, SWORD).await;
    }
    assert_eq!(
        worn,
        (crate::game::store::BAG_PLAYER, 15),
        "in the main hand"
    );
    a.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    b.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    assert_eq!(
        trade_end(&mut b).await,
        SMSG_TRADE_STATUS_TradeStatus::TradeCanceled,
        "the worn sword stays"
    );
    assert_eq!(place(&map, ga, SWORD).await, worn);
    // A stack offered whole, then split.
    trade_window(&mut a, &mut b, gb).await;
    let (bag, slot) = place(&map, ga, POTIONS).await;
    a.send(CMSG_SET_TRADE_ITEM {
        trade_slot: 0,
        bag,
        slot,
    })
    .await;
    b.wait_for("the offer", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS_EXTENDED(t) => {
            (t.trade_slots[0].stack_count == 20).then_some(())
        }
        _ => None,
    })
    .await;
    a.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    let free = on(&map, move |m| {
        let data = m.realm.data.clone();
        m.players[&ga].free_carry_slot(&data).unwrap()
    })
    .await;
    a.send(CMSG_SPLIT_ITEM {
        source_bag: bag,
        source_slot: slot,
        destination_bag: free.0,
        destination_slot: free.1,
        amount: 19,
    })
    .await;
    for _ in 0..100 {
        if on(&map, move |m| {
            m.players[&ga]
                .items
                .iter()
                .filter(|i| i.entry == POTIONS)
                .count()
        })
        .await
            == 2
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    b.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    assert_eq!(
        trade_end(&mut b).await,
        SMSG_TRADE_STATUS_TradeStatus::TradeCanceled,
        "not the one potion left of the twenty shown"
    );
    let theirs = on(&map, move |m| m.players[&gb].count_of(POTIONS)).await;
    assert_eq!(theirs, 0);
}

/// Send `c` into `map`'s copy and wait for it to land.
async fn go(realm: &Arc<Realm>, c: &mut Client, map: u32) -> MapHandle {
    let g = c.guid;
    let from = realm.registry.online(g).unwrap().map();
    from.call(move |m| {
        m.teleport(
            g,
            crate::game::map::MapKey::world(map),
            Vector3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
        )
    });
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
    realm.registry.online(g).unwrap().map()
}

/// Whether `g` comes to have `spell` (or, `on` false, to lose it) within
/// a few of world PvP's seconds.
async fn comes_to(map: &MapHandle, g: Guid, spell: u32, on_it: bool) -> bool {
    for _ in 0..40 {
        let has = on(map, move |m| {
            m.players.get(&g).is_some_and(|p| p.auras.has_spell(spell))
        })
        .await;
        if has == on_it {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// The Plaguelands' tower buff reaches Stratholme and Scholomance, and the
/// Cenarion Favor the Ruins and Temple of Ahn'Qiraj, as Thistle Tea's
/// `buff_zone?` and mangos's affected zones give them.
#[tokio::test]
async fn world_pvp_buffs_reach_their_dungeons() {
    const TWO_TOWERS: u32 = 11_414;
    const FAVOR: u32 = 30_754;
    let mut d = GameData::empty();
    for id in [11_413, TWO_TOWERS, 11_415, 1386, FAVOR] {
        let mut s = unit_spell(id, 6);
        s.effects[0].target_a = 1;
        s.effects[0].aura = crate::game::map::auras::AURA_MOD_RESISTANCE;
        s.duration_index = 0;
        d.spells.spells.insert(id, s);
    }
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Ana", Race::Human, Class::Warrior).await;
    *realm.social.outdoor.lock() = crate::game::map::outdoor_pvp::Shared {
        towers: [2, 0],
        favor: Some(0),
    };
    let strat = go(&realm, &mut a, 329).await;
    a.send(CMSG_ZONEUPDATE {
        area: Area::try_from(2017u32).unwrap(),
    })
    .await;
    assert!(
        comes_to(&strat, g, TWO_TOWERS, true).await,
        "two towers' buff in Stratholme"
    );
    let ruins = go(&realm, &mut a, 509).await;
    a.send(CMSG_ZONEUPDATE {
        area: Area::try_from(3429u32).unwrap(),
    })
    .await;
    assert!(
        comes_to(&ruins, g, FAVOR, true).await,
        "the Favor in the Ruins"
    );
    realm.social.outdoor.lock().favor = None;
    assert!(comes_to(&ruins, g, FAVOR, false).await, "gone with it");
}

const RAPTOR: u32 = 900_400;

/// The next dynamic flags `c` is told for `of`.
async fn dynamic_flags(c: &mut Client, of: Guid) -> u32 {
    c.wait_for("the creature's dynamic flags", |m| {
        harness::values(m)
            .into_iter()
            .filter_map(|(g, mask)| match mask {
                UpdateMask::Unit(u) if g == of => u.unit_dynamic_flags(),
                _ => None,
            })
            .last()
    })
    .await as u32
}

/// A tapped creature is grey to all but its tapper and the tapper's group,
/// each viewer's own (mangos-classic's per-viewer `UNIT_DYNAMIC_FLAGS`,
/// Thistle Tea's `Visibility.Tap`); one who comes later sees it grey too,
/// and it's no one's again when it evades.
#[tokio::test]
async fn a_tapped_creature_is_grey_to_all_but_its_tappers() {
    const TAPPED: u32 = 0x4;
    let mut d = GameData::empty();
    let mut raptor = plain_creature(RAPTOR, "Raptor", 7);
    raptor.speed_walk = 0.01;
    raptor.speed_run = 0.01;
    raptor.detection_range = 0.1;
    d.creature_templates.insert(RAPTOR, raptor);
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: 47,
        entries: vec![RAPTOR],
        map: 0,
        x: START.0 + 8.0,
        y: START.1,
        z: START.2,
        ..Default::default()
    });
    let realm = harness::realm_with(d);
    let (mut a, ga, map) = enter(&realm, "A", "Ana", Race::Human, Class::Warrior).await;
    let (mut b, _gb, _) = enter(&realm, "B", "Bea", Race::Human, Class::Warrior).await;
    let (mut c, _gc, _) = enter(&realm, "C", "Cid", Race::Human, Class::Warrior).await;
    group(&mut a, &mut c, "Cid").await;
    let raptor = on(&map, |m| {
        m.creatures
            .iter()
            .find(|(_, c)| c.entry == RAPTOR)
            .map(|(_, c)| c.guid)
    })
    .await
    .expect("the raptor");
    for x in [&mut a, &mut b, &mut c] {
        x.sees(raptor).await;
        x.clear();
    }
    let tapped = on(&map, move |m| {
        crate::game::map::ai::attacked(m, raptor, ga);
        m.creature(raptor).and_then(|c| c.tapped_by)
    })
    .await;
    assert_eq!(tapped, Some(ga), "tapped");
    assert_ne!(
        dynamic_flags(&mut b, raptor).await & TAPPED,
        0,
        "grey to another"
    );
    assert_eq!(
        dynamic_flags(&mut a, raptor).await & TAPPED,
        0,
        "not to its tapper"
    );
    assert_eq!(
        dynamic_flags(&mut c, raptor).await & TAPPED,
        0,
        "nor the tapper's group"
    );
    // One who comes later.
    let (mut d, _gd, _) = enter(&realm, "D", "Dax", Race::Human, Class::Warrior).await;
    d.sees(raptor).await;
    assert_ne!(
        dynamic_flags(&mut d, raptor).await & TAPPED,
        0,
        "grey to one who comes later"
    );
    on(&map, move |m| crate::game::map::ai::enter_evade(m, raptor)).await;
    assert_eq!(
        dynamic_flags(&mut b, raptor).await & TAPPED,
        0,
        "no one's when it evades"
    );
}

const HEALER: u32 = 900_410;
const NEAR_WOLF: u32 = 900_411;
const FAR_WOLF: u32 = 900_412;
const WISP: u32 = 900_413;

/// A ghost sees, of the creatures, only the living within 45 yards of its
/// corpse, those its kind are shown to (type flag 0x2), and the spirit
/// healers, which the living don't see: Thistle Tea's `Visibility.Filter`
/// and mangos-classic's `Creature::IsVisibleInGridForPlayer` and
/// `isInvisibleForAlive`.
#[tokio::test]
async fn a_ghost_sees_only_the_creatures_by_its_corpse() {
    let mut d = GameData::empty();
    let mut healer = plain_creature(HEALER, "Spirit Healer", 35);
    healer.npc_flags = crate::game::map::npc::NPC_FLAG_SPIRITHEALER;
    d.creature_templates.insert(HEALER, healer);
    for (entry, name) in [(NEAR_WOLF, "Near Wolf"), (FAR_WOLF, "Far Wolf")] {
        let mut w = plain_creature(entry, name, 35);
        w.detection_range = 0.1;
        d.creature_templates.insert(entry, w);
    }
    let mut wisp = plain_creature(WISP, "Wisp", 35);
    wisp.type_flags = 0x2;
    d.creature_templates.insert(WISP, wisp);
    let at = |guid, entry, dx: f32| CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        ..Default::default()
    };
    d.creature_spawns.insert(
        0,
        vec![
            at(48, HEALER, 5.0),
            at(49, NEAR_WOLF, 10.0),
            at(50, FAR_WOLF, 60.0),
            at(51, WISP, 70.0),
        ],
    );
    let realm = harness::realm_with(d);
    let (mut a, g, map) = enter(&realm, "A", "Ghost", Race::Human, Class::Warrior).await;
    let of = |entry: u32| {
        let map = map.clone();
        async move {
            on(&map, move |m| {
                m.creatures
                    .iter()
                    .find(|(_, c)| c.entry == entry)
                    .map(|(_, c)| c.guid)
            })
            .await
            .expect("spawned")
        }
    };
    let (healer, near, far, wisp) = (
        of(HEALER).await,
        of(NEAR_WOLF).await,
        of(FAR_WOLF).await,
        of(WISP).await,
    );
    for c in [near, far, wisp] {
        a.sees(c).await;
    }
    let short = Duration::from_millis(300);
    assert!(
        !a.sees_within(healer, short).await,
        "the living don't see a spirit healer"
    );
    on(&map, move |m| {
        if let Some(p) = m.players.get_mut(&g) {
            p.fields.set_u32(UNIT_FIELD_HEALTH, 1);
        }
        crate::game::map::combat::deal_damage(m, near, g, 1_000_000);
    })
    .await;
    a.send(CMSG_REPOP_REQUEST {}).await;
    a.wait_for("the teleport to the graveyard", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
    a.forgets(far).await;
    a.sees(healer).await;
    let known = on(&map, move |m| {
        let p = &m.players[&g];
        (
            p.known.contains(&near),
            p.known.contains(&wisp),
            p.known.contains(&far),
        )
    })
    .await;
    assert_eq!(
        known,
        (true, true, false),
        "the near wolf and the wisp, not the far wolf"
    );
    on(&map, move |m| crate::game::map::death::resurrect(m, g, 1.0)).await;
    a.sees(far).await;
    a.forgets(healer).await;
}

const RAJAXX: u32 = 900_420;
const CAPTAIN: u32 = 900_421;

/// `creature_linking_template` ties each spawn of its entry to the nearest
/// spawn of its master's within the search range, or to the map's only one
/// with no range, as vmangos and Thistle Tea's `CreatureLink` loader do: a
/// captain joins Rajaxx's fight; one out of range has no master.
#[tokio::test]
async fn a_templated_link_ties_a_captain_to_its_general() {
    let mut d = GameData::empty();
    let mut general = plain_creature(RAJAXX, "General Rajaxx", 7);
    general.detection_range = 0.1;
    d.creature_templates.insert(RAJAXX, general);
    let mut captain = plain_creature(CAPTAIN, "Captain", 7);
    captain.detection_range = 0.1;
    d.creature_templates.insert(CAPTAIN, captain);
    let at = |guid, entry, dx: f32| CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        ..Default::default()
    };
    d.creature_spawns.insert(
        0,
        vec![
            at(52, RAJAXX, 10.0),
            at(53, CAPTAIN, 20.0),
            at(54, CAPTAIN, 90.0),
        ],
    );
    d.scripts
        .link_templates
        .insert((CAPTAIN, 0), (RAJAXX, scripts::LINK_AGGRO_ON_AGGRO, 50.0));
    crate::game::data::scripts_load::link_templates(&mut d);
    assert_eq!(
        d.scripts.links.get(&53),
        Some(&(52, scripts::LINK_AGGRO_ON_AGGRO))
    );
    assert_eq!(d.scripts.links.get(&54), None, "out of the search range");
    let realm = harness::realm_with(d);
    let (_a, g, map) = enter(&realm, "A", "Ana", Race::Human, Class::Warrior).await;
    let fighting = on(&map, move |m| {
        let of = |spawn: u32| crate::game::map::scripts::creature_of_spawn(m, spawn).unwrap();
        let (rajaxx, near, far) = (of(52), of(53), of(54));
        crate::game::map::ai::attacked(m, rajaxx, g);
        let at_it = |c| m.creature(c).is_some_and(|c| c.combat.attacking);
        (at_it(rajaxx), at_it(near), at_it(far))
    })
    .await;
    assert_eq!(fighting, (true, true, false), "the near captain joins in");
}

const LOATHEB: u32 = 900_422;
const MAGGOT: u32 = 900_423;

/// A slave flagged both not to spawn while its master is dead and while
/// it is alive (Naxxramas's trash, 3072) spawns while its master lives:
/// the first flag decides, as in mangos's `CanSpawn` and Thistle Tea's
/// `spawn_allowed?`.
#[tokio::test]
async fn a_slave_barred_both_ways_spawns_while_its_master_lives() {
    let mut d = GameData::empty();
    d.creature_templates
        .insert(LOATHEB, plain_creature(LOATHEB, "Loatheb", 7));
    d.creature_templates
        .insert(MAGGOT, plain_creature(MAGGOT, "Maggot", 7));
    let at = |guid, entry, dx: f32| CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1 + 40.0,
        z: START.2,
        respawn_min: 600,
        respawn_max: 600,
        ..Default::default()
    };
    d.creature_spawns
        .insert(0, vec![at(55, LOATHEB, 0.0), at(56, MAGGOT, 5.0)]);
    d.scripts.link_templates.insert(
        (MAGGOT, 0),
        (
            LOATHEB,
            scripts::LINK_CANT_SPAWN_IF_BOSS_DEAD | scripts::LINK_CANT_SPAWN_IF_BOSS_ALIVE,
            0.0,
        ),
    );
    crate::game::data::scripts_load::link_templates(&mut d);
    assert!(d.scripts.links.contains_key(&56), "the map's only master");
    let realm = harness::realm_with(d);
    let (_a, _g, map) = enter(&realm, "A", "Ana", Race::Human, Class::Warrior).await;
    let alive = on(&map, |m| {
        let maggot = crate::game::map::scripts::creature_of_spawn(m, 56).unwrap();
        m.creature(maggot).unwrap().alive()
    })
    .await;
    assert!(alive, "up while Loatheb lives");
}
