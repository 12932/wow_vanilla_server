//! Round 5's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/{gossip,gossip_condition,home_bind,training}_test.exs`
//! (docs/parity/matrix.csv): whom an NPC's menu answers, what it offers by
//! the NPC's flags now, the menu a script gave it, and the talk credit of a
//! closing option.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::npc::{GossipMenu, GossipOption};
use crate::game::data::{CreatureTemplate, GameData};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::creature::Life;
use crate::game::map::{Map, npc};
use crate::game::tests_p3_combat::{HUMANS, setup_with, spawn, template};
use crate::game::tests_p3_entity::{on, take_quest};
use crate::game::tests_p3_player2_ammo::handled;

/// A talker of the humans' with a menu; its menu, and one a script gives it.
const TALKER: u32 = 3750;
const MENU: u32 = 4093;
const MENU_TEXT: u32 = 4995;
const SCRIPTED_MENU: u32 = 5000;
const SCRIPTED_TEXT: u32 = 5001;
/// A quest to talk to the talker.
const TALK_QUEST: u32 = 96_500;

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.creature_templates.insert(
        TALKER,
        CreatureTemplate {
            faction: HUMANS,
            gossip_menu_id: MENU,
            ..template(TALKER, 7)
        },
    );
    let option = |id: u32, npc_flag: u32, action_menu: i32, text: &str| GossipOption {
        id,
        kind: 1,
        npc_flag,
        action_menu,
        text: text.into(),
        ..Default::default()
    };
    d.npc.gossip_menus.insert(
        MENU,
        GossipMenu {
            texts: vec![(MENU_TEXT, 0)],
            options: vec![
                // Shown to a gossip NPC alone; it closes the window, which
                // is talking to it.
                option(0, npc::NPC_FLAG_GOSSIP, -1, "Tell me more."),
                // Shown to any; it closes the window too, but asks nothing.
                option(1, 0, 0, "Nothing."),
            ],
        },
    );
    d.npc.gossip_menus.insert(
        SCRIPTED_MENU,
        GossipMenu {
            texts: vec![(SCRIPTED_TEXT, 0)],
            options: Vec::new(),
        },
    );
    d.npc.quests.insert(
        TALK_QUEST,
        crate::game::data::npc::Quest {
            id: TALK_QUEST,
            title: "A Word".into(),
            required_kills: [(TALKER as i32, 1), (0, 0), (0, 0), (0, 0)],
            ..Default::default()
        },
    );
    d
}

/// The menu `c` is shown when it greets `npc`: its greeting and options.
async fn greeted(c: &mut Client, npc: Guid) -> Option<(u32, Vec<String>)> {
    c.clear();
    c.send(CMSG_GOSSIP_HELLO { guid: npc }).await;
    handled(c).await;
    c.gets(Duration::from_millis(1), |m| match m {
        ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) if g.guid == npc => Some((
            g.title_text_id,
            g.gossips.iter().map(|o| o.message.clone()).collect(),
        )),
        _ => None,
    })
    .await
}

fn place(m: &mut Map, g: Guid, c: Guid, yards: f32) {
    let at = m.players[&g].position();
    m.creature_mut(c).unwrap().position = Vector3d {
        x: at.x + yards,
        ..at
    };
}

/// Thistle Tea's `Gossip` "answers only a living creature within
/// interaction reach", "offers an option once a script grants the creature
/// the flag it needs" and "follows the menu a script gave the creature over
/// its template's": a menu is shown by a living creature in reach alone;
/// an option for gossip NPCs is offered once the creature has the flag,
/// read as it is now; and a menu a script set is the one shown.
#[tokio::test]
async fn a_menu_answers_the_near_and_living_by_its_flags_now_and_its_scripts_menu() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let c = on(&map, move |m| {
        let c = spawn(m, ga, TALKER, 20, 20.0);
        m.creature_mut(c).unwrap().fields.set_u32(UNIT_NPC_FLAGS, 0);
        c
    })
    .await;
    a.sees(c).await;
    assert_eq!(greeted(&mut a, c).await, None, "twenty yards off");
    on(&map, move |m| {
        place(m, ga, c, 2.0);
        let later = m.now + Duration::from_secs(60);
        m.creature_mut(c).unwrap().life = Life::Dead {
            corpse_until: later,
            respawn_at: later,
        };
    })
    .await;
    assert_eq!(greeted(&mut a, c).await, None, "dead");
    on(&map, move |m| m.creature_mut(c).unwrap().life = Life::Alive).await;
    assert_eq!(
        greeted(&mut a, c).await,
        Some((MENU_TEXT, vec!["Nothing.".to_owned()])),
        "no gossip flag: its option hidden"
    );
    on(&map, move |m| {
        let f = &mut m.creature_mut(c).unwrap().fields;
        f.set_u32(UNIT_NPC_FLAGS, npc::NPC_FLAG_GOSSIP);
    })
    .await;
    assert_eq!(
        greeted(&mut a, c).await,
        Some((
            MENU_TEXT,
            vec!["Tell me more.".to_owned(), "Nothing.".to_owned()]
        )),
        "granted the flag"
    );
    on(&map, move |m| {
        crate::game::map::event_ai::scripted_mut(m, c)
            .unwrap()
            .gossip_menu = Some(SCRIPTED_MENU);
    })
    .await;
    assert_eq!(
        greeted(&mut a, c).await,
        Some((SCRIPTED_TEXT, Vec::new())),
        "the script's menu"
    );
}

/// Thistle Tea's `Gossip.select` "credits talking to the speaker only when
/// its closing option asks to": of two options that close the window, the
/// one leading nowhere (mangos's menu -1, `TalkedToCreature`) counts as
/// talking to the creature for a quest asking it; the other doesn't.
#[tokio::test]
async fn a_closing_option_credits_talking_only_when_it_leads_nowhere() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let c = on(&map, move |m| {
        take_quest(m, ga, TALK_QUEST);
        let c = spawn(m, ga, TALKER, 20, 2.0);
        let f = &mut m.creature_mut(c).unwrap().fields;
        f.set_u32(UNIT_NPC_FLAGS, npc::NPC_FLAG_GOSSIP);
        c
    })
    .await;
    a.sees(c).await;
    let talked = move |m: &mut Map| m.players[&ga].journal.log[0].kills[0];
    for (option, credited) in [(1, 0), (0, 1)] {
        assert!(greeted(&mut a, c).await.is_some());
        a.send(CMSG_GOSSIP_SELECT_OPTION {
            guid: c,
            gossip_list_id: option,
            unknown: None,
        })
        .await;
        a.wait_for("SMSG_GOSSIP_COMPLETE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
        })
        .await;
        assert_eq!(on(&map, talked).await, credited, "option {option}");
    }
}

/// A talker whose menu's options and texts are gated by conditions.
const GATED: u32 = 3751;
const GATED_MENU: u32 = 4094;
/// Night elf druids, by an and of their side and their race and class.
const MOONGLADE: u32 = 700;
const ALLIANCE: u32 = 701;
const DRUIDS: u32 = 702;
/// A sword carried or banked; a condition of a kind no one judges.
const SWORD_KEPT: u32 = 703;
const UNKNOWN: u32 = 704;
const FALLBACK_TEXT: u32 = 100;
const UNKNOWN_TEXT: u32 = 200;

fn gated_world() -> GameData {
    use crate::game::data::scripts::Condition;
    let mut d = world();
    d.creature_templates.insert(
        GATED,
        CreatureTemplate {
            faction: HUMANS,
            gossip_menu_id: GATED_MENU,
            ..template(GATED, 7)
        },
    );
    for (entry, kind, values) in [
        (MOONGLADE, -1, [ALLIANCE as i32, DRUIDS as i32, 0, 0]),
        (ALLIANCE, 6, [469, 0, 0, 0]),
        (DRUIDS, 14, [0, 1024, 0, 0]),
        (
            SWORD_KEPT,
            23,
            [crate::game::tests_p3_combat::SWORD as i32, 1, 0, 0],
        ),
        (UNKNOWN, 999, [0; 4]),
    ] {
        d.scripts.conditions.insert(
            entry,
            Condition {
                entry,
                kind,
                values,
                flags: 0,
            },
        );
    }
    let option = |id: u32, condition: u32, text: &str| GossipOption {
        id,
        kind: 1,
        action_menu: -1,
        condition,
        text: text.into(),
        ..Default::default()
    };
    d.npc.gossip_menus.insert(
        GATED_MENU,
        GossipMenu {
            texts: vec![(FALLBACK_TEXT, 0), (UNKNOWN_TEXT, UNKNOWN)],
            options: vec![
                option(0, MOONGLADE, "Fly me to Moonglade."),
                option(1, SWORD_KEPT, "About that sword."),
            ],
        },
    );
    d
}

/// Thistle Tea's `GossipCondition` "gates a Moonglade taxi option by
/// faction and druid class" and `Gossip` "uses the unconditional fallback
/// when a display condition is unknown": an option under an and of the
/// Alliance and the druid class is offered to a night elf druid, not to a
/// night elf warrior nor a tauren druid; and a greeting whose condition no
/// one judges gives way to the unconditional one.
#[tokio::test]
async fn an_option_is_gated_by_side_and_class_and_an_unknown_greeting_gives_way() {
    let realm = crate::game::harness::realm_with(gated_world());
    for (n, (race, class, offered)) in [
        (Race::NightElf, Class::Druid, true),
        (Race::NightElf, Class::Warrior, false),
        (Race::Tauren, Class::Druid, false),
    ]
    .into_iter()
    .enumerate()
    {
        let mut c = Client::connect(&realm, &format!("G{n}"));
        let g = c
            .enter(
                &format!("Gated{}", "abc".as_bytes()[n] as char),
                race,
                class,
            )
            .await;
        let key = realm.registry.online(g).expect("online").map().key;
        let map = realm.maps.get(&realm, key);
        let npc = on(&map, move |m| spawn(m, g, GATED, 20, 2.0)).await;
        c.sees(npc).await;
        let (text, options) = greeted(&mut c, npc).await.expect("a menu");
        assert_eq!(text, FALLBACK_TEXT, "the unknown greeting gives way");
        assert_eq!(
            options.contains(&"Fly me to Moonglade.".to_owned()),
            offered,
            "{race:?} {class:?}"
        );
    }
}

/// Whether choosing `option` of `npc`'s menu does anything: the window
/// closes, as each of the gated menu's options closes it.
async fn chosen(c: &mut Client, npc: Guid, option: u32) -> bool {
    c.clear();
    c.send(CMSG_GOSSIP_SELECT_OPTION {
        guid: npc,
        gossip_list_id: option,
        unknown: None,
    })
    .await;
    handled(c).await;
    c.gets(Duration::from_millis(1), |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
    })
    .await
    .is_some()
}

/// Thistle Tea's "shows and selects an inclusive option while deposited
/// or withdrawn": an option asking for an item, bank included (condition
/// 23), is offered and may be chosen with the item in the bank or in the
/// bags; without it, it is neither, even chosen from a menu shown before.
#[tokio::test]
async fn an_option_asking_for_an_item_counts_the_bank_when_shown_and_chosen() {
    use crate::game::map::inventory::BANK_START;
    use crate::game::store::BAG_PLAYER;
    use crate::game::tests_p3_combat::SWORD;
    let (_realm, mut a, ga, map) = setup_with(gated_world(), Class::Warrior).await;
    let npc = on(&map, move |m| {
        crate::game::map::inventory::add_item(m, ga, SWORD, 1, false).unwrap();
        let p = m.players.get_mut(&ga).unwrap();
        let i = p.items.iter().position(|i| i.entry == SWORD).unwrap();
        p.items[i].bag = BAG_PLAYER;
        p.items[i].slot = BANK_START;
        spawn(m, ga, GATED, 20, 2.0)
    })
    .await;
    a.sees(npc).await;
    let sword = "About that sword.".to_owned();
    let (_, options) = greeted(&mut a, npc).await.expect("a menu");
    assert!(options.contains(&sword), "banked");
    assert!(chosen(&mut a, npc, 1).await, "chosen, banked");
    on(&map, move |m| {
        crate::game::map::inventory::add_item(m, ga, SWORD, 1, false).unwrap();
        let p = m.players.get_mut(&ga).unwrap();
        p.items
            .retain(|i| !(i.entry == SWORD && i.slot == BANK_START));
    })
    .await;
    let (_, options) = greeted(&mut a, npc).await.expect("a menu");
    assert!(options.contains(&sword), "in the bags");
    assert!(chosen(&mut a, npc, 1).await, "chosen, carried");
    on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.items.retain(|i| i.entry != SWORD);
    })
    .await;
    assert!(
        !chosen(&mut a, npc, 1).await,
        "gone since the menu was shown"
    );
    let (_, options) = greeted(&mut a, npc).await.expect("a menu");
    assert!(!options.contains(&sword), "not offered");
}

/// An innkeeper whose menu offers to bind, and a warriors' trainer.
const INNKEEPER: u32 = 3752;
const INN_MENU: u32 = 4095;
const TRAINER: u32 = 3753;

fn services_world() -> GameData {
    let mut d = world();
    d.creature_templates.insert(
        INNKEEPER,
        CreatureTemplate {
            faction: HUMANS,
            gossip_menu_id: INN_MENU,
            npc_flags: npc::NPC_FLAG_GOSSIP | npc::NPC_FLAG_INNKEEPER,
            ..template(INNKEEPER, 7)
        },
    );
    d.npc.gossip_menus.insert(
        INN_MENU,
        GossipMenu {
            texts: vec![(1221, 0)],
            options: vec![GossipOption {
                id: 0,
                kind: 8,
                npc_flag: npc::NPC_FLAG_INNKEEPER,
                text: "Make this inn your home.".into(),
                ..Default::default()
            }],
        },
    );
    d.creature_templates.insert(
        TRAINER,
        CreatureTemplate {
            faction: HUMANS,
            npc_flags: npc::NPC_FLAG_TRAINER,
            trainer_class: 1,
            ..template(TRAINER, 7)
        },
    );
    d
}

/// Thistle Tea's `HomeBind` "gossip requests confirmation without changing
/// the home": the innkeeper's option asks the client to confirm, and the
/// home is what it was until it does.
#[tokio::test]
async fn an_innkeepers_option_asks_to_confirm_and_leaves_the_home() {
    let (_realm, mut a, ga, map) = setup_with(services_world(), Class::Warrior).await;
    let (inn, home) = on(&map, move |m| {
        let inn = spawn(m, ga, INNKEEPER, 20, 2.0);
        let f = &mut m.creature_mut(inn).unwrap().fields;
        f.set_u32(
            UNIT_NPC_FLAGS,
            npc::NPC_FLAG_GOSSIP | npc::NPC_FLAG_INNKEEPER,
        );
        (
            inn,
            m.players[&ga]
                .row
                .state
                .bind
                .map(|b| (b.map, b.x, b.y, b.z)),
        )
    })
    .await;
    a.sees(inn).await;
    assert!(greeted(&mut a, inn).await.is_some());
    a.send(CMSG_GOSSIP_SELECT_OPTION {
        guid: inn,
        gossip_list_id: 0,
        unknown: None,
    })
    .await;
    let by = a
        .wait_for("SMSG_BINDER_CONFIRM", |m| match m {
            ServerOpcodeMessage::SMSG_BINDER_CONFIRM(b) => Some(b.guid),
            _ => None,
        })
        .await;
    assert_eq!(by, inn);
    let now = on(&map, move |m| {
        m.players[&ga]
            .row
            .state
            .bind
            .map(|b| (b.map, b.x, b.y, b.z))
    })
    .await;
    assert_eq!(now, home, "not bound yet");
}

/// Whether `c` is shown `trainer`'s list.
async fn listed(c: &mut Client, trainer: Guid) -> bool {
    c.clear();
    c.send(CMSG_TRAINER_LIST { guid: trainer }).await;
    handled(c).await;
    c.gets(Duration::from_millis(1), |m| match m {
        ServerOpcodeMessage::SMSG_TRAINER_LIST(t) if t.guid == trainer => Some(()),
        _ => None,
    })
    .await
    .is_some()
}

/// Thistle Tea's `Training` "requires a live nearby trainer in the same
/// world": a trainer lists its spells alive and within reach alone.
#[tokio::test]
async fn a_trainer_lists_only_alive_and_within_reach() {
    let (_realm, mut a, ga, map) = setup_with(services_world(), Class::Warrior).await;
    let t = on(&map, move |m| {
        let t = spawn(m, ga, TRAINER, 20, 2.0);
        let f = &mut m.creature_mut(t).unwrap().fields;
        f.set_u32(UNIT_NPC_FLAGS, npc::NPC_FLAG_TRAINER);
        t
    })
    .await;
    a.sees(t).await;
    assert!(listed(&mut a, t).await, "near and alive");
    on(&map, move |m| place(m, ga, t, 10.0)).await;
    assert!(!listed(&mut a, t).await, "ten yards off");
    on(&map, move |m| {
        place(m, ga, t, 2.0);
        let later = m.now + Duration::from_secs(60);
        m.creature_mut(t).unwrap().life = Life::Dead {
            corpse_until: later,
            respawn_at: later,
        };
    })
    .await;
    assert!(!listed(&mut a, t).await, "dead");
    // Fighting, or charmed: not now (mangos's `CanInteractNow`).
    on(&map, move |m| {
        m.creature_mut(t).unwrap().life = Life::Alive;
        let c = m.combat_mut(t).unwrap();
        c.attacking = true;
        c.victim = ga;
    })
    .await;
    assert!(!listed(&mut a, t).await, "fighting");
    on(&map, move |m| {
        m.combat_mut(t).unwrap().attacking = false;
        let f = &mut m.creature_mut(t).unwrap().fields;
        f.set_guid(UNIT_FIELD_CHARMEDBY, ga);
    })
    .await;
    assert!(!listed(&mut a, t).await, "charmed");
    on(&map, move |m| {
        let f = &mut m.creature_mut(t).unwrap().fields;
        f.set_guid(UNIT_FIELD_CHARMEDBY, Guid::zero());
    })
    .await;
    assert!(listed(&mut a, t).await, "free again");
}

/// A flight master of a faction (72) its players may be honored with, at
/// the first of three nodes in a line; each hop's fare twenty-five.
const FLIGHT_MASTER: u32 = 3754;
const FLIGHT_FACTION: u32 = 72;
const FLIGHT_TEMPLATE: u32 = 1072;

fn flights_world() -> GameData {
    use crate::game::data::npc::{TaxiNode, TaxiPathNode};
    let mut d = world();
    d.faction_templates.insert(
        FLIGHT_TEMPLATE,
        crate::game::data::FactionTemplate {
            faction: FLIGHT_FACTION,
            ..crate::game::tests_p3_combat::faction(FLIGHT_TEMPLATE, 0x1, 0x2)
        },
    );
    d.misc.factions.insert(
        FLIGHT_FACTION,
        crate::game::data::misc::FactionDef {
            id: FLIGHT_FACTION,
            index: 19,
            name: "Stormwind".into(),
            ..Default::default()
        },
    );
    d.creature_templates.insert(
        FLIGHT_MASTER,
        CreatureTemplate {
            faction: FLIGHT_TEMPLATE,
            npc_flags: npc::NPC_FLAG_FLIGHTMASTER,
            ..template(FLIGHT_MASTER, 7)
        },
    );
    let start = crate::game::tests_p3_combat::START;
    for (id, dx) in [(2, 0.0), (3, 100.0), (4, 200.0)] {
        d.npc.taxi_nodes.insert(
            id,
            TaxiNode {
                id,
                map: 0,
                x: start.0 + dx,
                y: start.1,
                z: start.2,
                name: format!("Node {id}"),
                mounts: [0, 0],
            },
        );
    }
    for (from, path) in [(2, 500), (3, 501)] {
        d.npc.taxi_paths.insert((from, from + 1), (path, 25));
        let x0 = start.0 + 100.0 * (from - 2) as f32;
        d.npc.taxi_path_nodes.insert(
            path,
            (0..3)
                .map(|i| TaxiPathNode {
                    map: 0,
                    x: x0 + 50.0 * i as f32,
                    y: start.1,
                    z: start.2,
                    ..Default::default()
                })
                .collect(),
        );
    }
    d
}

/// Thistle Tea's `Taxi.activate` "rounds the honored flightmaster discount
/// for each path": an express flight of two hops pays each hop's fare a
/// tenth off, rounded, to one honored by the flight master's faction:
/// twice 23 for twice 25, where the whole would be 45.
#[tokio::test]
async fn an_honored_flight_takes_a_tenth_off_each_hop() {
    for (honored, paid) in [(false, 50), (true, 46)] {
        let (_realm, mut a, ga, map) = setup_with(flights_world(), Class::Warrior).await;
        let master = on(&map, move |m| {
            let p = m.players.get_mut(&ga).unwrap();
            p.row.state.taxi_mask = vec![(1 << 2) | (1 << 3) | (1 << 4)];
            p.set_money(100);
            if honored {
                crate::game::map::reputation::modify(m, ga, FLIGHT_FACTION, 9000, false);
            }
            spawn(m, ga, FLIGHT_MASTER, 20, 2.0)
        })
        .await;
        a.sees(master).await;
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
        let money = on(&map, move |m| m.players[&ga].money()).await;
        assert_eq!(100 - money, paid, "honored {honored}");
    }
}
