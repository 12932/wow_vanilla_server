//! Thistle Tea's inbound and message tests (Phase 3, `game/inbound`,
//! `game/network`) for what NPCs do, through the protocol on a world made
//! for them: a vendor's standing and stock.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::misc::FactionDef;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const STORMWIND: u32 = 72;
const VENDOR: u32 = 2001;
const JERKY: u32 = 117;
const TABARD: u32 = 50_100;
const LIMITED: u32 = 50_101;
/// A gossiper who is a flight master and a banker too.
const GOSSIPER: u32 = 2010;
const MENU: u32 = 60;
const TALK_QUEST: u32 = 9_100;
const LEVEL_10: u32 = 70;
/// A trainer with nothing to teach a warrior, and one teaching a spell
/// that needs another and has no rank before it.
const QUIET_TRAINER: u32 = 2020;
const TRAINER: u32 = 2021;
const TAUGHT: u32 = 900_401;
const NEEDED: u32 = 900_402;

fn spawn(guid: u32, entry: u32, dx: f32) -> CreatureSpawn {
    CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 300,
        respawn_max: 300,
        ..Default::default()
    }
}

fn creature(entry: u32, name: &str, faction: u32, npc_flags: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 2,
        level_max: 2,
        faction,
        npc_flags,
        unit_class: 1,
        health_multiplier: 1.0,
        health_max: 40,
        ..Default::default()
    }
}

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

fn data() -> GameData {
    let mut d = GameData::empty();
    d.misc.factions.insert(
        STORMWIND,
        FactionDef {
            id: STORMWIND,
            index: 0,
            name: "Stormwind".into(),
            variants: [(1, 0, 0, 0x11), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)],
            ..Default::default()
        },
    );
    d.misc.faction_by_index.insert(0, STORMWIND);
    d.faction_templates.insert(
        12,
        FactionTemplate {
            id: 12,
            faction: STORMWIND,
            ours: 2,
            friend_mask: 2,
            ..Default::default()
        },
    );
    d.creature_templates
        .insert(VENDOR, creature(VENDOR, "Vendor", 12, 0x4));
    let mut gossiper = creature(GOSSIPER, "Gossiper", 12, 0x1 | 0x8 | 0x100);
    gossiper.gossip_menu_id = MENU;
    d.creature_templates.insert(GOSSIPER, gossiper);
    d.creature_spawns
        .insert(0, vec![spawn(11, VENDOR, 2.0), spawn(12, GOSSIPER, 3.0)]);
    gossip(&mut d);
    for (entry, name) in [(QUIET_TRAINER, "Quiet Trainer"), (TRAINER, "Trainer")] {
        let mut t = creature(entry, name, 12, 0x10);
        t.trainer_type = 2;
        d.creature_templates.insert(entry, t);
    }
    d.creature_spawns
        .get_mut(&0)
        .unwrap()
        .extend([spawn(13, QUIET_TRAINER, 1.0), spawn(14, TRAINER, 1.5)]);
    d.npc.trainer_greetings.insert(QUIET_TRAINER, "Hi".into());
    d.npc.trainers.insert(
        TRAINER,
        vec![crate::game::data::npc::TrainerSpell {
            spell: TAUGHT,
            cost: 10,
            required_level: 1,
            ..Default::default()
        }],
    );
    d.spells.ranks.insert(
        TAUGHT,
        crate::game::data::spells::SpellRank {
            prev: 0,
            first: TAUGHT,
            rank: 1,
            req: NEEDED,
        },
    );
    let sold = |slot, item, max_count| VendorItem {
        slot,
        item,
        max_count,
        restock_secs: 3600,
        ..Default::default()
    };
    d.vendors.insert(
        VENDOR,
        vec![sold(1, JERKY, 0), sold(2, TABARD, 0), sold(3, LIMITED, 2)],
    );
    let mut jerky = item(JERKY, "Tough Jerky");
    jerky.buy_price = 25;
    jerky.stackable = 20;
    d.items.insert(JERKY, jerky);
    // Friendly with Stormwind (rank 4) to buy.
    let mut tabard = item(TABARD, "Stormwind Tabard");
    tabard.buy_price = 1;
    tabard.required_reputation_faction = STORMWIND;
    tabard.required_reputation_rank = 4;
    d.items.insert(TABARD, tabard);
    let mut limited = item(LIMITED, "Rare Recipe");
    limited.buy_price = 1;
    d.items.insert(LIMITED, limited);
    d
}

/// The gossiper's menu: a goodbye that flies the player off by a script,
/// the flight map, the bank, a plain goodbye, and one for level 10.
fn gossip(d: &mut GameData) {
    use crate::game::data::npc::{GossipMenu, GossipOption, TaxiNode, TaxiPathNode};
    use crate::game::data::scripts::{Condition, ScriptStep};
    let option = |id, kind, npc_flag, action_menu, script, condition| GossipOption {
        id,
        text: format!("Option {id}"),
        kind,
        npc_flag,
        action_menu,
        script,
        condition,
        ..Default::default()
    };
    d.npc.gossip_menus.insert(
        MENU,
        GossipMenu {
            texts: Vec::new(),
            options: vec![
                option(0, 1, 0, -1, 900, 0),
                option(1, 4, 0x8, 0, 0, 0),
                option(2, 9, 0x100, 0, 0, 0),
                option(3, 1, 0, -1, 0, 0),
                option(4, 1, 0, -1, 900, LEVEL_10),
            ],
        },
    );
    // Script 900 sends its player along path 315.
    d.scripts.gossip.insert(
        900,
        vec![ScriptStep {
            command: 30,
            datalong: [315, 0, 0, 0],
            ..Default::default()
        }],
    );
    d.scripts.conditions.insert(
        LEVEL_10,
        Condition {
            entry: LEVEL_10,
            kind: 15,
            values: [10, 1, 0, 0],
            flags: 0,
        },
    );
    d.npc.taxi_nodes.insert(
        2,
        TaxiNode {
            id: 2,
            map: 0,
            x: START.0 + 3.0,
            y: START.1,
            z: START.2,
            name: "Northshire".into(),
            mounts: [0, 0],
        },
    );
    let node = |dx: f32| TaxiPathNode {
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2 + 20.0,
        ..Default::default()
    };
    d.npc
        .taxi_path_nodes
        .insert(315, vec![node(0.0), node(100.0), node(200.0)]);
    // Speak with the gossiper.
    d.npc.quests.insert(
        TALK_QUEST,
        crate::game::data::npc::Quest {
            id: TALK_QUEST,
            title: "Have a Word".into(),
            required_kills: [(GOSSIPER as i32, 1), (0, 0), (0, 0), (0, 0)],
            ..Default::default()
        },
    );
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

fn money(m: &Map, g: Guid) -> u32 {
    m.players[&g].fields.get_u32(PLAYER_FIELD_COINAGE)
}

async fn bought(c: &mut Client) -> Result<u32, BuyResult> {
    c.wait_for("SMSG_BUY_ITEM or SMSG_BUY_FAILED", |m| match m {
        ServerOpcodeMessage::SMSG_BUY_ITEM(b) => Some(Ok(b.amount_bought)),
        ServerOpcodeMessage::SMSG_BUY_FAILED(f) => Some(Err(f.result)),
        _ => None,
    })
    .await
}

async fn listed(c: &mut Client, vendor: Guid) -> Vec<(u32, u32)> {
    c.send(CMSG_LIST_INVENTORY { guid: vendor }).await;
    c.wait_for("SMSG_LIST_INVENTORY", |m| match m {
        ServerOpcodeMessage::SMSG_LIST_INVENTORY(l) => {
            Some(l.items.iter().map(|i| (i.item, i.max_items)).collect())
        }
        _ => None,
    })
    .await
}

/// Thistle Tea's cmsg_buy_item_reputation_test and smsg_list_inventory_test:
/// an item asking for a standing the buyer hasn't is refused with
/// `REPUTATION_REQUIRE`; an Honored buyer's discount is on the whole
/// purchase, rounded once; and limited stock sold out lists as none left,
/// not as unlimited.
#[tokio::test]
async fn a_vendor_asks_for_standing_discounts_the_whole_and_runs_out() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let vendor = guid::unit(VENDOR, 11);
    a.sees(vendor).await;
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(1_000)
    })
    .await;
    // Neutral with Stormwind: the tabard is refused, and nothing paid.
    a.send(CMSG_BUY_ITEM {
        vendor,
        item: TABARD,
        amount: 1,
        unknown1: 0,
    })
    .await;
    assert_eq!(bought(&mut a).await, Err(BuyResult::ReputationRequire));
    assert_eq!(on_map(&realm, move |m| money(m, g)).await, 1_000);
    // Honored: two jerkies at 25 each are 45, the tenth off 50 rounded
    // once.
    on_map(&realm, move |m| {
        crate::game::map::reputation::modify(m, g, STORMWIND, 9_000, false)
    })
    .await;
    a.send(CMSG_BUY_ITEM {
        vendor,
        item: JERKY,
        amount: 2,
        unknown1: 0,
    })
    .await;
    assert_eq!(bought(&mut a).await, Ok(2));
    assert_eq!(on_map(&realm, move |m| money(m, g)).await, 955);
    a.send(CMSG_BUY_ITEM {
        vendor,
        item: TABARD,
        amount: 1,
        unknown1: 0,
    })
    .await;
    assert_eq!(bought(&mut a).await, Ok(1), "friendly enough now");
    // The rare recipe's two bought: none left, not unlimited.
    assert!(listed(&mut a, vendor).await.contains(&(LIMITED, 2)));
    a.send(CMSG_BUY_ITEM {
        vendor,
        item: LIMITED,
        amount: 2,
        unknown1: 0,
    })
    .await;
    assert_eq!(bought(&mut a).await, Ok(2));
    let list = listed(&mut a, vendor).await;
    assert!(list.contains(&(LIMITED, 0)), "{list:?}");
    assert!(list.contains(&(JERKY, u32::MAX)), "unlimited: {list:?}");
}

fn select(npc: Guid, option: u32) -> CMSG_GOSSIP_SELECT_OPTION {
    CMSG_GOSSIP_SELECT_OPTION {
        guid: npc,
        gossip_list_id: option,
        unknown: None,
    }
}

/// Thistle Tea's cmsg_gossip_select_option_test: a goodbye credits a quest
/// asking to speak with the creature; an option whose condition no longer
/// holds does nothing; the bank and flight map options open them; and a
/// goodbye's script flies its player off, the client's own movement then
/// ignored (msg_move_test's taxi flight).
#[tokio::test]
async fn gossip_options_talk_open_the_bank_and_flight_map_and_run_their_scripts() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let npc = guid::unit(GOSSIPER, 12);
    a.sees(npc).await;
    on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.journal.log.push(crate::game::store::Logged {
            quest: TALK_QUEST,
            ..Default::default()
        });
        crate::game::map::quests::refresh_log(p, &d);
    })
    .await;
    a.send(CMSG_GOSSIP_HELLO { guid: npc }).await;
    let shown = a
        .wait_for("SMSG_GOSSIP_MESSAGE", |m| match m {
            ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) => {
                Some(g.gossips.iter().map(|o| o.id).collect::<Vec<_>>())
            }
            _ => None,
        })
        .await;
    assert!(!shown.contains(&4), "level 10's option hidden: {shown:?}");
    // A goodbye: the window closes, and the talk is credited.
    a.send(select(npc, 3)).await;
    a.wait_for("SMSG_GOSSIP_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
    })
    .await;
    let (quest, guid) = a
        .wait_for("SMSG_QUESTUPDATE_ADD_KILL", |m| match m {
            ServerOpcodeMessage::SMSG_QUESTUPDATE_ADD_KILL(k) => Some((k.quest_id, k.guid)),
            _ => None,
        })
        .await;
    assert_eq!((quest, guid), (TALK_QUEST, npc));
    let kills = on_map(&realm, move |m| {
        m.players[&g]
            .journal
            .log
            .iter()
            .find(|l| l.quest == TALK_QUEST)
            .map(|l| l.kills[0])
    })
    .await;
    assert_eq!(kills, Some(1));
    // The level 10 option, chosen at level 1 anyway: nothing.
    a.clear();
    a.send(select(npc, 4)).await;
    assert!(
        a.gets(std::time::Duration::from_millis(300), |m| match m {
            ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE
            | ServerOpcodeMessage::SMSG_MONSTER_MOVE(_) => Some(()),
            _ => None,
        })
        .await
        .is_none(),
        "a condition not met does nothing"
    );
    // The bank, and the flight map from the node here.
    a.send(select(npc, 2)).await;
    let bank = a
        .wait_for("SMSG_SHOW_BANK", |m| match m {
            ServerOpcodeMessage::SMSG_SHOW_BANK(b) => Some(b.guid),
            _ => None,
        })
        .await;
    assert_eq!(bank, npc);
    a.send(select(npc, 1)).await;
    let (from, nearest) = a
        .wait_for("SMSG_SHOWTAXINODES", |m| match m {
            ServerOpcodeMessage::SMSG_SHOWTAXINODES(t) => Some((t.guid, t.nearest_node)),
            _ => None,
        })
        .await;
    assert_eq!((from, nearest), (npc, 2));
    // The scripted goodbye: closed, and away on path 315.
    a.send(select(npc, 0)).await;
    a.wait_for("SMSG_GOSSIP_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
    })
    .await;
    let mut flying = false;
    for _ in 0..100 {
        flying = on_map(&realm, move |m| m.players[&g].flight.is_some()).await;
        if flying {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(flying, "the script's flight");
    // Flying, the client's own movement counts for nothing.
    let here = on_map(&realm, move |m| m.players[&g].position()).await;
    a.heartbeat(here.x + 50.0, here.y + 50.0, here.z).await;
    a.send(CMSG_QUERY_TIME {}).await;
    a.wait_for("SMSG_QUERY_TIME_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_QUERY_TIME_RESPONSE(_)).then_some(())
    })
    .await;
    let there = on_map(&realm, move |m| m.players[&g].position()).await;
    assert!(
        (there.x - (here.x + 50.0)).abs() > 1.0 || (there.y - (here.y + 50.0)).abs() > 1.0,
        "not where the client said: {there:?}"
    );
}

/// Thistle Tea's smsg_trainer_list_test: a trainer with nothing to teach
/// sends an empty list and its greeting; a spell with no rank before it
/// names the spell it needs first, the second slot left 0.
#[tokio::test]
async fn a_trainers_list_greets_and_names_what_a_spell_needs() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let quiet = guid::unit(QUIET_TRAINER, 13);
    let trainer = guid::unit(TRAINER, 14);
    a.sees(quiet).await;
    a.sees(trainer).await;
    a.send(CMSG_TRAINER_LIST { guid: quiet }).await;
    let empty = a
        .wait_frame("SMSG_TRAINER_LIST", 0x1B1, |f| {
            f[..8] == quiet.guid().to_le_bytes()
        })
        .await;
    let mut want = quiet.guid().to_le_bytes().to_vec();
    want.extend_from_slice(&2u32.to_le_bytes());
    want.extend_from_slice(&0u32.to_le_bytes());
    want.extend_from_slice(b"Hi\0");
    assert_eq!(empty, want, "an empty list, then the greeting");
    a.send(CMSG_TRAINER_LIST { guid: trainer }).await;
    let list = a
        .wait_frame("SMSG_TRAINER_LIST", 0x1B1, |f| {
            f[..8] == trainer.guid().to_le_bytes()
        })
        .await;
    assert_eq!(u32::from_le_bytes(list[12..16].try_into().unwrap()), 1);
    // The spell, its state, cost, two profession words, level, skill and
    // its value; then the three spells it needs.
    let entry = &list[16..16 + 38];
    assert_eq!(&entry[..4], &TAUGHT.to_le_bytes());
    let needs = |at: usize| u32::from_le_bytes(entry[at..at + 4].try_into().unwrap());
    assert_eq!((needs(26), needs(30), needs(34)), (NEEDED, 0, 0));
}

/// Thistle Tea's taxi_messages_test: a flight master's node status is its
/// GUID and whether the node is known; its map is a 1, the GUID, the
/// nearest node, and the eight words of the nodes known.
#[tokio::test]
async fn a_flight_masters_node_and_map_are_laid_out_as_1_12_has_them() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let npc = guid::unit(GOSSIPER, 12);
    a.sees(npc).await;
    a.send(CMSG_TAXINODE_STATUS_QUERY { guid: npc }).await;
    let status = a.wait_frame("SMSG_TAXINODE_STATUS", 0x1AB, |_| true).await;
    assert_eq!(
        status,
        [&npc.guid().to_le_bytes()[..], &[0]].concat(),
        "not known yet"
    );
    a.send(CMSG_TAXIQUERYAVAILABLENODES { guid: npc }).await;
    let map = a.wait_frame("SMSG_SHOWTAXINODES", 0x1A9, |_| true).await;
    let mut want = 1u32.to_le_bytes().to_vec();
    want.extend_from_slice(&npc.guid().to_le_bytes());
    want.extend_from_slice(&2u32.to_le_bytes());
    let mut mask = [0u32; 8];
    mask[0] = 1 << 2;
    for w in mask {
        want.extend_from_slice(&w.to_le_bytes());
    }
    assert_eq!(&map[..want.len()], &want[..]);
    assert_eq!(map.len(), want.len(), "the fixed eight words");
    a.send(CMSG_TAXINODE_STATUS_QUERY { guid: npc }).await;
    let status = a.wait_frame("SMSG_TAXINODE_STATUS", 0x1AB, |_| true).await;
    assert_eq!(status[8], 1, "known now");
}
