//! The player systems beside the world's main ones, through the protocol:
//! reputation, rest, quest sharing, tickets, items, express flights,
//! talent resets, meeting stones, self-resurrection, summons, the
//! environment, exploration and character deletion.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::misc::*;
use crate::game::data::npc::*;
use crate::game::data::spells::{SpellEffect, SpellTemplate, TalentInfo};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const NONE_WITHIN: Duration = Duration::from_millis(300);

const STORMWIND: u32 = 72;
const IRONFORGE: u32 = 47;
const DEFIAS: u32 = 2000;
const VENDOR: u32 = 2001;
const TRAINER: u32 = 2002;
const FLIGHT_MASTER: u32 = 2003;
const QUEST_GIVER: u32 = 2004;
const SPIRIT_HEALER: u32 = 2005;
const JERKY: u32 = 117;
const CLAM: u32 = 9000;
const BOOK: u32 = 9001;
const PAPER: u32 = 5042;
const GIFT: u32 = 5043;
const SWORD: u32 = 9002;
const STONE: u32 = 178824;
const TALENT_SPELL: u32 = 20000;
const QUEST: u32 = 600;

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
        xp_multiplier: Some(1.0),
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

fn aura_spell(id: u32, auras: &[(u32, i32, i32)]) -> SpellTemplate {
    let mut effects = [SpellEffect::default(); 3];
    for (i, (aura, points, misc)) in auras.iter().enumerate() {
        effects[i] = SpellEffect {
            effect: 6,
            aura: *aura,
            base_points: *points - 1,
            base_dice: 1,
            misc_value: *misc,
            target_a: 1,
            ..Default::default()
        };
    }
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
    // Stormwind and Ironforge, Stormwind's gains spilling a quarter over.
    // Stormwind's peace is forced for humans; Ironforge's isn't.
    for (id, index, flags) in [(STORMWIND, 0, 0x11), (IRONFORGE, 1, 0x1)] {
        d.misc.factions.insert(
            id,
            FactionDef {
                id,
                index,
                name: format!("Faction {id}"),
                variants: [(1, 0, 0, flags), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)],
                ..Default::default()
            },
        );
        d.misc.faction_by_index.insert(index, id);
    }
    d.misc.spillovers.insert(
        STORMWIND,
        vec![Spillover {
            faction: IRONFORGE,
            rate: 0.25,
            max_rank: RANK_EXALTED,
        }],
    );
    d.misc.kill_reputation.insert(
        DEFIAS,
        vec![KillReputation {
            faction: STORMWIND,
            value: 100,
            max_rank: RANK_EXALTED,
            ..Default::default()
        }],
    );
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
    d.faction_templates.insert(
        14,
        FactionTemplate {
            id: 14,
            faction: 15,
            ours: 8,
            enemy_mask: 1,
            ..Default::default()
        },
    );
    d.creature_templates
        .insert(DEFIAS, creature(DEFIAS, "Defias Thug", 14, 0));
    d.creature_templates
        .insert(VENDOR, creature(VENDOR, "Vendor", 12, 0x4));
    let mut trainer = creature(TRAINER, "Trainer", 12, 0x10);
    trainer.trainer_class = 1;
    d.creature_templates.insert(TRAINER, trainer);
    d.creature_templates.insert(
        FLIGHT_MASTER,
        creature(FLIGHT_MASTER, "Gryphon Master", 12, 0x8),
    );
    d.creature_templates
        .insert(QUEST_GIVER, creature(QUEST_GIVER, "Quest Giver", 12, 0x2));
    d.creature_templates.insert(
        SPIRIT_HEALER,
        creature(SPIRIT_HEALER, "Spirit Healer", 35, 0x21),
    );
    d.creature_spawns.insert(
        0,
        vec![
            spawn(10, DEFIAS, 3.0),
            spawn(11, VENDOR, 2.0),
            spawn(12, TRAINER, 1.5),
            spawn(13, FLIGHT_MASTER, 4.0),
            spawn(14, QUEST_GIVER, 1.0),
            spawn(15, SPIRIT_HEALER, 2.5),
        ],
    );
    d.vendors.insert(
        VENDOR,
        vec![VendorItem {
            slot: 1,
            item: JERKY,
            ..Default::default()
        }],
    );
    let mut jerky = item(JERKY, "Tough Jerky");
    jerky.buy_price = 100;
    jerky.stackable = 20;
    d.items.insert(JERKY, jerky);
    let mut clam = item(CLAM, "Small Barnacled Clam");
    clam.flags = 0x4;
    clam.min_money_loot = 5;
    clam.max_money_loot = 5;
    d.items.insert(CLAM, clam);
    d.loot.item.insert(
        CLAM,
        vec![LootEntry {
            item: JERKY,
            chance: 100.0,
            min: 1,
            max: 1,
            ..Default::default()
        }],
    );
    let mut book = item(BOOK, "A Letter");
    book.page_text = 10;
    d.items.insert(BOOK, book);
    d.misc.page_texts.insert(10, ("Dear friend,".into(), 11));
    d.misc.page_texts.insert(11, ("Yours.".into(), 0));
    let mut paper = item(PAPER, "Gift Wrap");
    paper.flags = 0x200;
    paper.stackable = 20;
    d.items.insert(PAPER, paper);
    d.items.insert(GIFT, item(GIFT, "Wrapped Gift"));
    let mut sword = item(SWORD, "Short Sword");
    sword.class = 2;
    sword.inventory_type = 13;
    sword.delay = 2000;
    d.items.insert(SWORD, sword);
    // Three flight nodes in a row.
    for (id, dx) in [(2u32, 4.0f32), (3, 64.0), (4, 124.0)] {
        d.npc.taxi_nodes.insert(
            id,
            TaxiNode {
                id,
                map: 0,
                x: START.0 + dx,
                y: START.1,
                z: START.2,
                name: format!("Node {id}"),
                mounts: [0, 0],
            },
        );
    }
    for (from, to, path, cost) in [(2u32, 3u32, 9u32, 5u32), (3, 4, 10, 7)] {
        d.npc.taxi_paths.insert((from, to), (path, cost));
        let (a, b) = (d.npc.taxi_nodes[&from].x, d.npc.taxi_nodes[&to].x);
        d.npc.taxi_path_nodes.insert(
            path,
            vec![
                TaxiPathNode {
                    map: 0,
                    x: a,
                    y: START.1,
                    z: START.2 + 10.0,
                    ..Default::default()
                },
                TaxiPathNode {
                    map: 0,
                    x: b,
                    y: START.1,
                    z: START.2 + 10.0,
                    ..Default::default()
                },
            ],
        );
    }
    // A quest to share, with reputation and a mail.
    d.npc.quests.insert(
        QUEST,
        Quest {
            id: QUEST,
            level: 1,
            min_level: 1,
            title: "Shared Quest".into(),
            details: "Help.".into(),
            objectives: "Help.".into(),
            reputation_rewards: [(STORMWIND, 250), (0, 0), (0, 0), (0, 0), (0, 0)],
            ..Default::default()
        },
    );
    d.npc.creature_starts.insert(QUEST_GIVER, vec![QUEST]);
    d.npc.creature_ends.insert(QUEST_GIVER, vec![QUEST]);
    d.misc.quest_mail.insert(
        QUEST,
        QuestMail {
            template: 84,
            delay_secs: 0,
            money: 7,
        },
    );
    d.misc
        .mail_templates
        .insert(84, (String::new(), "Thank you, $N.".into()));
    d.misc.mail_items.insert(84, vec![(JERKY, 1)]);
    // A meeting stone.
    d.gameobject_templates.insert(
        STONE,
        GameObjectTemplate {
            entry: STONE,
            kind: 23,
            display_id: 1,
            name: "Meeting Stone".into(),
            size: 1.0,
            ..Default::default()
        },
    );
    d.gameobject_spawns.insert(
        0,
        vec![GameObjectSpawn {
            guid: 30,
            entry: STONE,
            map: 0,
            x: START.0 - 2.0,
            y: START.1,
            z: START.2,
            rotation: [0.0, 0.0, 0.0, 1.0],
            state: 1,
            ..Default::default()
        }],
    );
    d.misc.meeting_stones.insert(
        STONE,
        MeetingStone {
            min_level: 1,
            max_level: 60,
            area: 1581,
        },
    );
    // An inn's trigger where the players start.
    d.area_triggers.insert(
        100,
        AreaTrigger {
            id: 100,
            map: 0,
            x: START.0,
            y: START.1,
            z: START.2,
            radius: 8.0,
            ..Default::default()
        },
    );
    d.npc.taverns.insert(100);
    // Talents, resurrection sickness and a soulstone.
    d.spells.talents.insert(
        1,
        TalentInfo {
            id: 1,
            tab: 161,
            ranks: [TALENT_SPELL, 0, 0, 0, 0],
            ..Default::default()
        },
    );
    let mut talent = aura_spell(TALENT_SPELL, &[(29, 1, 0)]);
    talent.attributes[0] = 0x40;
    d.spells.spells.insert(TALENT_SPELL, talent);
    let mut sickness = aura_spell(15007, &[(80, -75, -1)]);
    sickness.duration_index = 0;
    d.spells.spells.insert(15007, sickness);
    let mut stone = aura_spell(20707, &[(4, 0, 0)]);
    stone.duration_index = 0;
    d.spells.spells.insert(20707, stone);
    let mut res = SpellTemplate {
        id: 3026,
        name: "Use Soulstone".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    res.effects[0] = SpellEffect {
        effect: 94,
        base_points: 19,
        base_dice: 1,
        ..Default::default()
    };
    d.spells.spells.insert(3026, res);
    d.areas.insert(
        12,
        AreaInfo {
            id: 12,
            map: 0,
            explore_bit: 33,
            level: 1,
            name: "Elwynn Forest".into(),
            ..Default::default()
        },
    );
    d.misc.exploration_xp.insert(1, 5);
    d
}

async fn setup() -> (Arc<Realm>, Client) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    (realm, a)
}

fn map(realm: &Arc<Realm>) -> crate::game::map::MapHandle {
    realm.maps.get(realm, MapKey::world(0))
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map(realm).query(f).await.expect("the map answers")
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

#[tokio::test]
async fn reputation_comes_from_kills_and_quests_and_spills_over() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    // The login's 64 slots, Stormwind's visible.
    let init = a
        .wait_frame("SMSG_INITIALIZE_FACTIONS", 0x122, |_| true)
        .await;
    assert_eq!(u32_at(&init, 0), 64);
    assert_eq!(init[4] & 0x1, 0x1, "Stormwind is visible");
    let thug = guid::unit(DEFIAS, 10);
    a.sees(thug).await;
    a.clear();
    on_map(&realm, move |m| crate::game::map::combat::kill(m, g, thug)).await;
    let change = a
        .wait_frame("SMSG_SET_FACTION_STANDING", 0x124, |_| true)
        .await;
    // Ironforge's spillover, then Stormwind's own.
    assert_eq!(u32_at(&change, 0), 2);
    let pairs: Vec<(u32, u32)> = (0..2)
        .map(|i| (u32_at(&change, 4 + i * 8), u32_at(&change, 8 + i * 8)))
        .collect();
    assert!(pairs.contains(&(0, 100)), "{pairs:?}");
    assert!(pairs.contains(&(1, 25)), "{pairs:?}");
    // A quest's reward.
    let quest = realm.data.npc.quests[&QUEST].clone();
    let (sw, ifg) = on_map(&realm, move |m| {
        crate::game::map::reputation::quest_rewarded(m, g, &quest);
        let p = &m.players[&g];
        let d = &m.realm.data;
        (
            crate::game::map::reputation::standing(p, d, STORMWIND),
            crate::game::map::reputation::standing(p, d, IRONFORGE),
        )
    })
    .await;
    assert!((349..=351).contains(&sw), "{sw}");
    assert!((86..=88).contains(&ifg), "{ifg}");
    // The client's toggles come raw: at war, inactive, watched.
    a.send_raw(0x125, &[1, 0, 0, 0, 1]).await;
    a.send_raw(0x317, &[0, 0, 0, 0, 1]).await;
    a.send_raw(0x318, &[1, 0, 0, 0]).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (war, inactive, watched) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        let flags = |f: u32| {
            p.row
                .state
                .reputation
                .iter()
                .find(|s| s.faction == f)
                .map_or(0, |s| s.flags)
        };
        (
            flags(IRONFORGE) & crate::game::map::reputation::FLAG_AT_WAR != 0,
            flags(STORMWIND) & crate::game::map::reputation::FLAG_INACTIVE != 0,
            p.fields.get_u32(PLAYER_FIELD_WATCHED_FACTION_INDEX),
        )
    })
    .await;
    assert!(war && inactive);
    assert_eq!(watched, 1);
}

#[tokio::test]
async fn an_honored_player_pays_a_tenth_less() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    let vendor = guid::unit(VENDOR, 11);
    a.sees(vendor).await;
    on_map(&realm, move |m| {
        crate::game::map::reputation::modify(m, g, STORMWIND, 9_000, false)
    })
    .await;
    a.send(CMSG_LIST_INVENTORY { guid: vendor }).await;
    let price = a
        .wait_for("SMSG_LIST_INVENTORY", |m| match m {
            ServerOpcodeMessage::SMSG_LIST_INVENTORY(l) => {
                l.items.first().map(|i| i.price.as_int())
            }
            _ => None,
        })
        .await;
    assert_eq!(price, 90);
}

#[tokio::test]
async fn an_inn_rests_and_rested_xp_doubles_a_kill() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    a.send(CMSG_AREATRIGGER { trigger_id: 100 }).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let resting = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        // A long rest, as if days had passed.
        p.row.state.rest_bonus = 40.0;
        p.fields
            .has_flag(PLAYER_FLAGS, crate::game::map::player::PLAYER_FLAGS_RESTING)
    })
    .await;
    assert!(resting);
    a.clear();
    on_map(&realm, move |m| {
        crate::game::map::stats::give_xp(m, g, 40, Some(guid::unit(DEFIAS, 10)))
    })
    .await;
    let xp = a
        .wait_for("SMSG_LOG_XPGAIN", |m| match m {
            ServerOpcodeMessage::SMSG_LOG_XPGAIN(x) => Some(x.total_exp),
            _ => None,
        })
        .await;
    assert_eq!(xp, 80);
    // Leaving the inn stops the rest.
    a.heartbeat(START.0 + 40.0, START.1, START.2).await;
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let resting = on_map(&realm, move |m| {
        m.players[&g]
            .fields
            .has_flag(PLAYER_FLAGS, crate::game::map::player::PLAYER_FLAGS_RESTING)
    })
    .await;
    assert!(!resting);
}

async fn party(realm: &Arc<Realm>) -> (Client, Client) {
    let mut a = Client::connect(realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(realm, "B");
    b.enter("Bob", Race::Human, Class::Priest).await;
    a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
    b.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    a.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) if !l.members.is_empty() => Some(()),
        _ => None,
    })
    .await;
    (a, b)
}

#[tokio::test]
async fn a_quest_is_shared_and_taken_from_a_party_member() {
    let realm = harness::realm_with(data());
    let (mut a, mut b) = party(&realm).await;
    let (ga, gb) = (a.guid, b.guid);
    let giver = guid::unit(QUEST_GIVER, 14);
    a.sees(giver).await;
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: giver,
        quest_id: QUEST,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    a.send(CMSG_PUSHQUESTTOPARTY { quest_id: QUEST }).await;
    let from = b
        .wait_for("SMSG_QUESTGIVER_QUEST_DETAILS", |m| match m {
            ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_DETAILS(d) => Some(d.guid),
            _ => None,
        })
        .await;
    assert_eq!(from, ga);
    let sharing = a
        .wait_for("MSG_QUEST_PUSH_RESULT", |m| match m {
            ServerOpcodeMessage::MSG_QUEST_PUSH_RESULT(r) => Some((r.guid, r.message)),
            _ => None,
        })
        .await;
    assert_eq!(sharing, (gb, QuestPartyMessage::SharingQuest));
    b.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: ga,
        quest_id: QUEST,
    })
    .await;
    let accepted = a
        .wait_for("MSG_QUEST_PUSH_RESULT", |m| match m {
            ServerOpcodeMessage::MSG_QUEST_PUSH_RESULT(r) => Some(r.message),
            _ => None,
        })
        .await;
    assert_eq!(accepted, QuestPartyMessage::AcceptQuest);
    let has = on_map(&realm, move |m| m.players[&gb].journal.has(QUEST)).await;
    assert!(has);
    // Taking a quest from a player who didn't share it does nothing.
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Carol", Race::Human, Class::Mage).await;
    c.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: ga,
        quest_id: QUEST,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let has = on_map(&realm, move |m| m.players[&gc].journal.has(QUEST)).await;
    assert!(!has);
}

#[tokio::test]
async fn quests_in_the_log_swap_places() {
    let mut d = data();
    let mut second = d.npc.quests[&QUEST].clone();
    second.id = 601;
    d.npc.quests.insert(601, second);
    d.npc.creature_starts.insert(QUEST_GIVER, vec![QUEST, 601]);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let giver = guid::unit(QUEST_GIVER, 14);
    a.sees(giver).await;
    for q in [QUEST, 601] {
        a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
            guid: giver,
            quest_id: q,
        })
        .await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    a.send(CMSG_QUESTLOG_SWAP_QUEST { slot1: 0, slot2: 1 })
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (first, field) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (
            p.journal.log[0].quest,
            p.fields.get_u32(PLAYER_QUEST_LOG_1_1),
        )
    })
    .await;
    assert_eq!((first, field), (601, 601));
}

#[tokio::test]
async fn a_rewarded_quest_sends_its_mail() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    let giver = guid::unit(QUEST_GIVER, 14);
    a.sees(giver).await;
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: giver,
        quest_id: QUEST,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    a.send(CMSG_QUESTGIVER_CHOOSE_REWARD {
        guid: giver,
        quest_id: QUEST,
        reward: 0,
    })
    .await;
    a.wait_for("SMSG_RECEIVED_MAIL", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_RECEIVED_MAIL(_)).then_some(())
    })
    .await;
    let mail = {
        let mails = realm.social.mail.lock();
        mails
            .mails
            .values()
            .find(|m| m.receiver == guid::counter(g))
            .map(|m| {
                (
                    m.body.clone(),
                    m.money,
                    m.item.as_ref().map(|i| i.entry),
                    m.sender,
                )
            })
    };
    assert_eq!(
        mail,
        Some(("Thank you, $N.".into(), 7, Some(JERKY), QUEST_GIVER))
    );
}

#[tokio::test]
async fn a_ticket_is_filed_answered_and_replaced() {
    let (realm, mut a) = setup().await;
    a.send(CMSG_GMTICKET_CREATE {
        category: CMSG_GMTICKET_CREATE_GmTicketType::Stuck,
        map: wow_world_messages::vanilla::Map::EasternKingdoms,
        position: Vector3d {
            x: START.0,
            y: START.1,
            z: START.2,
        },
        message: "I am stuck".into(),
        reserved_for_future_use: String::new(),
    })
    .await;
    let r = a
        .wait_for("SMSG_GMTICKET_CREATE", |m| match m {
            ServerOpcodeMessage::SMSG_GMTICKET_CREATE(c) => Some(c.response),
            _ => None,
        })
        .await;
    assert_eq!(r, GmTicketResponse::CreateSuccess);
    a.send(CMSG_GMTICKET_UPDATETEXT {
        ticket_type: GmTicketType::Stuck,
        message: "Still stuck".into(),
    })
    .await;
    let r = a
        .wait_for("SMSG_GMTICKET_UPDATETEXT", |m| match m {
            ServerOpcodeMessage::SMSG_GMTICKET_UPDATETEXT(c) => Some(c.response),
            _ => None,
        })
        .await;
    assert_eq!(r, GmTicketResponse::UpdateSuccess);
    a.send(CMSG_GMTICKET_GETTICKET {}).await;
    let text = a
        .wait_for("SMSG_GMTICKET_GETTICKET", |m| match m {
            ServerOpcodeMessage::SMSG_GMTICKET_GETTICKET(t) => match &t.status {
                SMSG_GMTICKET_GETTICKET_GmTicketStatus::HasText { text, .. } => Some(text.clone()),
                _ => None,
            },
            _ => None,
        })
        .await;
    assert_eq!(text, "Still stuck");
    // A GM lists and answers it.
    a.say(".ticket list").await;
    let listed = a.system_message("Still stuck").await;
    assert!(listed.contains("Alice"));
    let id = realm
        .social
        .tickets
        .lock()
        .tickets
        .keys()
        .copied()
        .next()
        .unwrap();
    a.say(&format!(".ticket respond {id} Try .stuck")).await;
    let answered = a
        .wait_for("the answered ticket", |m| match m {
            ServerOpcodeMessage::SMSG_GMTICKET_GETTICKET(t) => match &t.status {
                SMSG_GMTICKET_GETTICKET_GmTicketStatus::HasText { text, .. } => Some(text.clone()),
                _ => None,
            },
            _ => None,
        })
        .await;
    assert!(answered.contains("GM answer") && answered.contains("Try .stuck"));
    // Done, it can't be edited; it can be deleted.
    a.send(CMSG_GMTICKET_DELETETICKET {}).await;
    let r = a
        .wait_for("SMSG_GMTICKET_DELETETICKET", |m| match m {
            ServerOpcodeMessage::SMSG_GMTICKET_DELETETICKET(c) => Some(c.response),
            _ => None,
        })
        .await;
    assert_eq!(r, GmTicketResponse::TicketDeleted);
    assert!(realm.social.tickets.lock().tickets.is_empty());
    a.send(CMSG_GMTICKET_SYSTEMSTATUS {}).await;
    a.wait_for("SMSG_GMTICKET_SYSTEMSTATUS", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GMTICKET_SYSTEMSTATUS(_)).then_some(())
    })
    .await;
}

async fn give(realm: &Arc<Realm>, g: Guid, entry: u32, count: u32) {
    on_map(realm, move |m| {
        crate::game::map::inventory::add_item(m, g, entry, count, false).expect("room for it")
    })
    .await;
}

async fn slot_of(realm: &Arc<Realm>, g: Guid, entry: u32) -> Option<(u8, u8)> {
    on_map(realm, move |m| {
        m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == entry)
            .map(|i| (i.bag, i.slot))
    })
    .await
}

#[tokio::test]
async fn an_opened_clam_shows_its_loot_and_is_used_up() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    give(&realm, g, CLAM, 1).await;
    let (bag, slot) = slot_of(&realm, g, CLAM).await.unwrap();
    a.clear();
    a.send(CMSG_OPEN_ITEM {
        bag_index: bag,
        slot,
    })
    .await;
    // 1.12's loot window, which upstream wow_messages reads differently.
    let frame = a.wait_frame("SMSG_LOOT_RESPONSE", 0x160, |_| true).await;
    let loot = (
        Guid::new(u64::from_le_bytes(frame[0..8].try_into().unwrap())),
        frame[13],
    );
    assert_eq!(loot.1, 1);
    let jerky_before = on_map(&realm, move |m| m.players[&g].count_of(JERKY)).await;
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.send(CMSG_LOOT_MONEY {}).await;
    a.send(CMSG_LOOT_RELEASE { guid: loot.0 }).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    let (jerky, clam, money) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (p.count_of(JERKY), p.count_of(CLAM), p.money())
    })
    .await;
    assert_eq!(jerky, jerky_before + 1);
    assert_eq!(clam, 0);
    assert!(money >= 5);
}

#[tokio::test]
async fn items_are_read_wrapped_banked_and_equipped_to_a_slot() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    give(&realm, g, BOOK, 1).await;
    let (bag, slot) = slot_of(&realm, g, BOOK).await.unwrap();
    a.send(CMSG_READ_ITEM {
        bag_index: bag,
        slot,
    })
    .await;
    a.wait_for("SMSG_READ_ITEM_OK", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_READ_ITEM_OK(_)).then_some(())
    })
    .await;
    a.send(CMSG_PAGE_TEXT_QUERY { page_id: 10 }).await;
    let (text, next) = a
        .wait_for("SMSG_PAGE_TEXT_QUERY_RESPONSE", |m| match m {
            ServerOpcodeMessage::SMSG_PAGE_TEXT_QUERY_RESPONSE(r) => {
                Some((r.text.clone(), r.next_page_id))
            }
            _ => None,
        })
        .await;
    assert_eq!((text.as_str(), next), ("Dear friend,", 11));
    // Wrapped, it is a gift; opened, the book again.
    give(&realm, g, PAPER, 1).await;
    let paper = slot_of(&realm, g, PAPER).await.unwrap();
    a.send(CMSG_WRAP_ITEM {
        gift_bag_index: paper.0,
        gift_slot: paper.1,
        item_bag_index: bag,
        item_slot: slot,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (book, gift, paper_left) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (p.count_of(BOOK), p.count_of(GIFT), p.count_of(PAPER))
    })
    .await;
    assert_eq!((book, gift, paper_left), (0, 1, 0));
    a.send(CMSG_OPEN_ITEM {
        bag_index: bag,
        slot,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let book = on_map(&realm, move |m| m.players[&g].count_of(BOOK)).await;
    assert_eq!(book, 1);
    // Into the bank and back.
    a.send(CMSG_AUTOBANK_ITEM {
        bag_index: bag,
        slot_index: slot,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let banked = slot_of(&realm, g, BOOK).await.unwrap();
    assert_eq!(
        banked,
        (
            crate::game::store::BAG_PLAYER,
            crate::game::map::inventory::BANK_START
        )
    );
    a.send(CMSG_AUTOSTORE_BANK_ITEM {
        bag_index: banked.0,
        slot_index: banked.1,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let back = slot_of(&realm, g, BOOK).await.unwrap();
    assert!(
        back.1 < crate::game::map::inventory::BANK_START
            || back.0 != crate::game::store::BAG_PLAYER
    );
    // A sword into the main hand by its GUID.
    give(&realm, g, SWORD, 1).await;
    let sword = on_map(&realm, move |m| {
        m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == SWORD)
            .map(|i| i.guid)
            .unwrap()
    })
    .await;
    a.send(CMSG_AUTOEQUIP_ITEM_SLOT {
        guid: sword,
        destination_slot: 15,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        slot_of(&realm, g, SWORD).await,
        Some((crate::game::store::BAG_PLAYER, 15))
    );
}

#[tokio::test]
async fn an_express_flight_pays_for_each_hop() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    let master = guid::unit(FLIGHT_MASTER, 13);
    a.sees(master).await;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.row.state.taxi_mask = vec![(1 << 2) | (1 << 3) | (1 << 4)];
        p.set_money(100);
    })
    .await;
    a.send(CMSG_ACTIVATETAXIEXPRESS {
        guid: master,
        total_cost: 0,
        nodes: vec![2, 3, 4],
    })
    .await;
    let reply = a
        .wait_for("SMSG_ACTIVATETAXIREPLY", |m| match m {
            ServerOpcodeMessage::SMSG_ACTIVATETAXIREPLY(r) => Some(r.reply),
            _ => None,
        })
        .await;
    assert_eq!(reply, ActivateTaxiReply::Ok);
    let (money, flying) = on_map(&realm, move |m| {
        (m.players[&g].money(), m.players[&g].flight.is_some())
    })
    .await;
    assert_eq!((money, flying), (88, true));
}

#[tokio::test]
async fn talents_reset_for_a_rising_price_and_skills_unlearn() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    let trainer = guid::unit(TRAINER, 12);
    a.sees(trainer).await;
    on_map(&realm, move |m| {
        crate::game::map::spells::learn(m, g, TALENT_SPELL);
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        // A reset is for level ten and over.
        crate::game::map::stats::set_level(p, &data, 10);
        p.set_money(50_000);
    })
    .await;
    on_map(&realm, move |m| {
        crate::game::map::misc::talent_wipe_offer(m, g, trainer)
    })
    .await;
    let cost = a
        .wait_for("MSG_TALENT_WIPE_CONFIRM", |m| match m {
            ServerOpcodeMessage::MSG_TALENT_WIPE_CONFIRM(c) => Some(c.cost_in_copper),
            _ => None,
        })
        .await;
    assert_eq!(cost, 10_000);
    a.send(MSG_TALENT_WIPE_CONFIRM_Client {
        wiping_npc: trainer,
    })
    .await;
    let removed = a
        .wait_for("SMSG_REMOVED_SPELL", |m| match m {
            ServerOpcodeMessage::SMSG_REMOVED_SPELL(r) => Some(r.spell),
            _ => None,
        })
        .await;
    assert_eq!(u32::from(removed), TALENT_SPELL);
    let (money, knows, next) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (
            p.money(),
            p.spells.contains(&TALENT_SPELL),
            crate::game::map::misc::talent_wipe_cost(
                p.row.state.talent_wipes,
                p.row.state.last_talent_wipe,
                chrono::Utc::now().timestamp(),
            ),
        )
    })
    .await;
    assert_eq!((money, knows, next), (40_000, false, 50_000));
    // A skill: forgotten.
    on_map(&realm, move |m| {
        crate::game::map::skills::learn_skill(m.players.get_mut(&g).unwrap(), 182, 1)
    })
    .await;
    a.send(CMSG_UNLEARN_SKILL {
        skill: Skill::Herbalism,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let has = on_map(&realm, move |m| {
        crate::game::map::skills::value(&m.players[&g], 182)
    })
    .await;
    assert_eq!(has, None);
}

#[tokio::test]
async fn a_meeting_stone_makes_a_party_of_those_waiting() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Priest).await;
    let stone = guid::game_object(STONE, 30);
    a.sees(stone).await;
    for c in [&a, &b] {
        c.send(CMSG_MEETINGSTONE_JOIN { guid: stone }).await;
    }
    let status = a
        .wait_for("SMSG_MEETINGSTONE_SETQUEUE", |m| match m {
            ServerOpcodeMessage::SMSG_MEETINGSTONE_SETQUEUE(s) => Some(s.status),
            _ => None,
        })
        .await;
    assert_eq!(status, MeetingStoneStatus::JoinedQueue);
    tokio::time::sleep(Duration::from_millis(50)).await;
    crate::game::map::meeting_stones::sweep(&realm);
    let added = a
        .wait_for("SMSG_MEETINGSTONE_MEMBER_ADDED", |m| match m {
            ServerOpcodeMessage::SMSG_MEETINGSTONE_MEMBER_ADDED(s) => Some(s.guid),
            _ => None,
        })
        .await;
    // Whoever queued first leads; the other is added.
    assert!(added == gb || added == ga);
    let together = realm
        .social
        .groups
        .lock()
        .group_of(ga)
        .is_some_and(|gr| gr.is_member(gb));
    assert!(together);
    b.send(CMSG_MEETINGSTONE_INFO {}).await;
    b.wait_for("SMSG_MEETINGSTONE_SETQUEUE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_MEETINGSTONE_SETQUEUE(_)).then_some(())
    })
    .await;
    a.send(CMSG_MEETINGSTONE_LEAVE {}).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(realm.social.meeting_stones.lock().of(ga).is_none());
}

#[tokio::test]
async fn raid_members_swap_subgroups() {
    let realm = harness::realm_with(data());
    let (a, mut b) = party(&realm).await;
    let (ga, gb) = (a.guid, b.guid);
    a.send(CMSG_GROUP_RAID_CONVERT {}).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    // Bob to the second subgroup, then the two swap.
    a.send(CMSG_GROUP_CHANGE_SUB_GROUP {
        name: "Bob".into(),
        group_number: 1,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    b.clear();
    a.send(CMSG_GROUP_SWAP_SUB_GROUP {
        name: "Alice".into(),
        swap_with_name: "Bob".into(),
    })
    .await;
    b.wait_for("SMSG_GROUP_LIST", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_LIST(_)).then_some(())
    })
    .await;
    let subgroups = {
        let groups = realm.social.groups.lock();
        let gr = groups.group_of(ga).unwrap();
        let of = |g: Guid| gr.members.iter().find(|m| m.guid == g).unwrap().subgroup;
        (of(ga), of(gb))
    };
    assert_eq!(subgroups, (1, 0));
}

#[tokio::test]
async fn a_soulstone_lets_the_dead_rise_and_a_summons_brings_a_player() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warlock).await;
    on_map(&realm, move |m| {
        let s = m.realm.data.spells.get(20707).unwrap().clone();
        crate::game::map::auras::apply(m, gb, ga, &s, 0, 0);
        crate::game::map::combat::deal_damage(m, gb, ga, 100_000);
    })
    .await;
    let spell = on_map(&realm, move |m| {
        m.players[&ga].fields.get_u32(PLAYER_SELF_RES_SPELL)
    })
    .await;
    assert_eq!(spell, 3026);
    a.send(CMSG_SELF_RES {}).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (alive, health, max) = on_map(&realm, move |m| {
        let p = &m.players[&ga];
        (
            p.death == crate::game::map::death::Death::Alive,
            p.fields.get_u32(UNIT_FIELD_HEALTH),
            p.fields.get_u32(UNIT_FIELD_MAXHEALTH),
        )
    })
    .await;
    assert!(alive);
    assert_eq!(health, max / 5);
    // Bob summons Alice to where he stands.
    b.heartbeat(START.0 + 20.0, START.1, START.2).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    on_map(&realm, move |m| {
        crate::game::map::misc::offer_summon(m, gb, ga)
    })
    .await;
    let by = a
        .wait_for("SMSG_SUMMON_REQUEST", |m| match m {
            ServerOpcodeMessage::SMSG_SUMMON_REQUEST(s) => Some(s.summoner),
            _ => None,
        })
        .await;
    assert_eq!(by, gb);
    a.send(CMSG_SUMMON_RESPONSE { summoner: gb }).await;
    a.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
}

#[tokio::test]
async fn movement_acknowledgements_reach_those_who_watch() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(ga).await;
    let info = MovementInfo {
        flags: MovementInfo_MovementFlags::empty(),
        timestamp: 10,
        position: Vector3d {
            x: START.0,
            y: START.1,
            z: START.2,
        },
        orientation: 0.0,
        fall_time: 0.0,
    };
    // (A speed's acknowledgement isn't passed on: those watching were told
    // at the change, `tests_parity_misc`.)
    a.send(CMSG_MOVE_KNOCK_BACK_ACK {
        guid: ga,
        counter: 0,
        info: info.clone(),
    })
    .await;
    b.wait_frame("MSG_MOVE_KNOCK_BACK", 0x0F1, |_| true).await;
    a.send(CMSG_MOVE_TIME_SKIPPED { guid: ga, lag: 250 }).await;
    let skipped = b.wait_frame("MSG_MOVE_TIME_SKIPPED", 0x319, |_| true).await;
    let n = skipped.len();
    assert_eq!(u32_at(&skipped, n - 4), 250);
    // The mount's flourish and the small stubs.
    a.send(CMSG_MOUNTSPECIAL_ANIM {}).await;
    b.wait_for("SMSG_MOUNTSPECIAL_ANIM", |m| match m {
        ServerOpcodeMessage::SMSG_MOUNTSPECIAL_ANIM(s) => Some(s.guid),
        _ => None,
    })
    .await;
    a.send(MSG_LOOKING_FOR_GROUP_Client {}).await;
    a.wait_for("MSG_LOOKING_FOR_GROUP", |m| {
        matches!(m, ServerOpcodeMessage::MSG_LOOKING_FOR_GROUP(_)).then_some(())
    })
    .await;
}

#[tokio::test]
async fn a_spirit_healer_leaves_sickness_and_a_wisp_runs_faster() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut n = Client::connect(&realm, "N");
    let gn = n.enter("Nyla", Race::NightElf, Class::Warrior).await;
    let healer = guid::unit(SPIRIT_HEALER, 15);
    let die = move |g: Guid| {
        move |m: &mut Map| {
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_LEVEL, 20);
            crate::game::map::combat::deal_damage(m, g, g, 100_000);
            crate::game::map::death::release(m, g);
            m.players[&g].speeds.run
        }
    };
    let human = on_map(&realm, die(ga)).await;
    // Only a ghost sees a spirit healer.
    a.sees(healer).await;
    // The night elf is in Teldrassil.
    let elf = realm
        .registry
        .online(gn)
        .unwrap()
        .map()
        .query(die(gn))
        .await
        .unwrap();
    assert!((human - 7.0 * 1.25).abs() < 0.01, "{human}");
    assert!((elf - 7.0 * 1.5).abs() < 0.01, "{elf}");
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.movement.position = Vector3d {
            x: START.0 + 2.5,
            y: START.1,
            z: START.2,
        };
    })
    .await;
    a.send(CMSG_SPIRIT_HEALER_ACTIVATE { guid: healer }).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let sick = on_map(&realm, move |m| {
        m.players[&ga]
            .auras
            .holders
            .iter()
            .find(|h| h.spell == 15007)
            .and_then(|h| h.duration)
    })
    .await;
    assert_eq!(sick, Some(10 * 60_000));
}

#[tokio::test]
async fn under_water_the_breath_runs_out_and_drowning_hurts() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    a.clear();
    on_map(&realm, move |m| {
        let now = m.now;
        crate::game::map::environment::breathe(
            m,
            g,
            true,
            crate::game::map::environment::BREATH,
            true,
            now,
        );
    })
    .await;
    a.wait_for("SMSG_START_MIRROR_TIMER", |m| match m {
        ServerOpcodeMessage::SMSG_START_MIRROR_TIMER(t) => {
            (t.timer == TimerType::Breath).then_some(())
        }
        _ => None,
    })
    .await;
    // A minute on, the breath is gone and the water hurts. Under again at
    // once first: the map's own update, between the two, finds the player
    // dry (there is no water here) and gives the breath back.
    let (before, after) = on_map(&realm, move |m| {
        let before = m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH);
        let now = m.now;
        crate::game::map::environment::breathe(
            m,
            g,
            true,
            crate::game::map::environment::BREATH,
            true,
            now,
        );
        let later = now + Duration::from_secs(61);
        crate::game::map::environment::breathe(
            m,
            g,
            true,
            crate::game::map::environment::BREATH,
            true,
            later,
        );
        (before, m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH))
    })
    .await;
    assert!(after < before);
    a.wait_for("SMSG_ENVIRONMENTAL_DAMAGE_LOG", |m| match m {
        ServerOpcodeMessage::SMSG_ENVIRONMENTAL_DAMAGE_LOG(d) => {
            (d.damage_type == EnvironmentalDamageType::Drowning).then_some(())
        }
        _ => None,
    })
    .await;
    // Back above water, the timer fills and goes.
    on_map(&realm, move |m| {
        let later = m.now + Duration::from_secs(70);
        crate::game::map::environment::breathe(
            m,
            g,
            false,
            crate::game::map::environment::BREATH,
            false,
            later,
        );
    })
    .await;
    a.wait_for("SMSG_STOP_MIRROR_TIMER", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_STOP_MIRROR_TIMER(_)).then_some(())
    })
    .await;
}

#[tokio::test]
async fn exploring_an_area_gives_xp_once() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    a.clear();
    on_map(&realm, move |m| {
        crate::game::map::exploration::entered(m, g, 12)
    })
    .await;
    let xp = a
        .wait_for("SMSG_EXPLORATION_EXPERIENCE", |m| match m {
            ServerOpcodeMessage::SMSG_EXPLORATION_EXPERIENCE(e) => Some(e.experience),
            _ => None,
        })
        .await;
    assert_eq!(xp, 5);
    let bit = on_map(&realm, move |m| {
        m.players[&g].fields.get_u32(PLAYER_EXPLORED_ZONES_1 + 1)
    })
    .await;
    assert_eq!(bit, 0x2);
    on_map(&realm, move |m| {
        crate::game::map::exploration::entered(m, g, 12)
    })
    .await;
    let again = a
        .gets(NONE_WITHIN, |m| {
            matches!(m, ServerOpcodeMessage::SMSG_EXPLORATION_EXPERIENCE(_)).then_some(())
        })
        .await;
    assert!(again.is_none());
}

#[tokio::test]
async fn a_deleted_character_leaves_their_guild_mail_and_auctions() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    let counter = guid::counter(g);
    a.send(CMSG_GUILD_CREATE {
        guild_name: "Lonely".into(),
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(realm.social.guilds.lock().of(g).is_some());
    crate::game::social::mail::post(
        &realm,
        crate::game::social::mail::Mail {
            receiver: counter,
            subject: "hello".into(),
            expires_at: chrono::Utc::now().timestamp() + 1000,
            ..Default::default()
        },
    );
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.send(CMSG_CHAR_DELETE { guid: g }).await;
    let r = a
        .wait_for("SMSG_CHAR_DELETE", |m| match m {
            ServerOpcodeMessage::SMSG_CHAR_DELETE(d) => Some(d.result),
            _ => None,
        })
        .await;
    assert_eq!(r, WorldResult::CharDeleteSuccess);
    assert!(realm.social.guilds.lock().guilds.is_empty());
    assert!(
        !realm
            .social
            .mail
            .lock()
            .mails
            .values()
            .any(|m| m.receiver == counter)
    );
}

#[tokio::test]
async fn an_escort_asks_the_party_to_join_and_one_who_confirms_has_it() {
    let mut d = data();
    d.npc.quests.get_mut(&QUEST).unwrap().flags =
        crate::game::map::questshare::QUEST_FLAGS_PARTY_ACCEPT;
    let realm = harness::realm_with(d);
    let (mut a, mut b) = party(&realm).await;
    let (ga, gb) = (a.guid, b.guid);
    let giver = guid::unit(QUEST_GIVER, 14);
    a.sees(giver).await;
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: giver,
        quest_id: QUEST,
    })
    .await;
    let (quest, by) = b
        .wait_for("SMSG_QUEST_CONFIRM_ACCEPT", |m| match m {
            ServerOpcodeMessage::SMSG_QUEST_CONFIRM_ACCEPT(c) => Some((c.quest_id, c.guid)),
            _ => None,
        })
        .await;
    assert_eq!((quest, by), (QUEST, ga));
    b.send(CMSG_QUEST_CONFIRM_ACCEPT { quest_id: QUEST }).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(on_map(&realm, move |m| m.players[&gb].journal.has(QUEST)).await);
}

#[tokio::test]
async fn the_deep_sea_tires_a_swimmer_out() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    a.clear();
    let (before, after) = on_map(&realm, move |m| {
        let now = m.now;
        crate::game::map::environment::fatigue(m, g, true, now);
        let before = m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH);
        crate::game::map::environment::fatigue(m, g, true, now + Duration::from_secs(61));
        (before, m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH))
    })
    .await;
    assert!(after < before);
    a.wait_for("the fatigue timer", |m| match m {
        ServerOpcodeMessage::SMSG_START_MIRROR_TIMER(t) => {
            (t.timer == TimerType::Fatigue).then_some(())
        }
        _ => None,
    })
    .await;
    a.wait_for("exhaustion", |m| match m {
        ServerOpcodeMessage::SMSG_ENVIRONMENTAL_DAMAGE_LOG(d) => {
            (d.damage_type == EnvironmentalDamageType::Exhausted).then_some(())
        }
        _ => None,
    })
    .await;
}

#[tokio::test]
async fn far_sight_ends_and_growth_auras_are_cancelled() {
    let (realm, a) = setup().await;
    let g = a.guid;
    let mut grow = aura_spell(30000, &[(crate::game::map::auras::AURA_MOD_SCALE, 20, 0)]);
    grow.duration_index = 0;
    on_map(&realm, move |m| {
        crate::game::map::auras::apply(m, g, g, &grow, 0, 20);
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_guid(PLAYER_FARSIGHT, Guid::new(5));
    })
    .await;
    a.send(CMSG_CANCEL_GROWTH_AURA {}).await;
    a.send(CMSG_FAR_SIGHT {
        operation: FarSightOperation::Remove,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (grown, sight) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (p.auras.has_spell(30000), p.fields.get_guid(PLAYER_FARSIGHT))
    })
    .await;
    assert!(!grown);
    assert!(sight.is_zero());
}

#[tokio::test]
async fn targeting_a_friendly_faction_shows_it() {
    let mut d = data();
    // Ironforge hidden at first for this test's humans.
    d.misc.factions.get_mut(&IRONFORGE).unwrap().variants[0].3 = 0;
    d.faction_templates.insert(
        55,
        FactionTemplate {
            id: 55,
            faction: IRONFORGE,
            ours: 2,
            friend_mask: 2,
            ..Default::default()
        },
    );
    d.creature_templates.get_mut(&VENDOR).unwrap().faction = 55;
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let vendor = guid::unit(VENDOR, 11);
    a.sees(vendor).await;
    a.send(CMSG_SET_SELECTION { target: vendor }).await;
    let shown = a
        .wait_frame("SMSG_SET_FACTION_VISIBLE", 0x123, |_| true)
        .await;
    assert_eq!(u32_at(&shown, 0), 1);
}
