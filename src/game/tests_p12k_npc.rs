//! Round 12, stage 12k: a creature's gossip as mangos-classic's
//! `Player::PrepareGossipMenu` and `OnGossipSelect` build and answer it. A
//! guild master's "How do I form a guild?" opens the charter and "I want
//! to create a guild crest." the tabard designer; options are offered to
//! the players they are for (a trainer's to its class, the quest giver's
//! placeholder to no one); and a flight master's node is learned as its
//! menu is shown.

use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit};
use crate::game::data::GameData;
use crate::game::data::npc::{GossipMenu, GossipOption, TaxiNode, TrainerSpell};

const GUILD_MASTER: u32 = 60_960;
const TRAINER: u32 = 60_961;
const FLIGHT_MASTER: u32 = 60_962;
const GUILD_MENU: u32 = 60_963;
const FLIGHT_MENU: u32 = 60_964;
const NODE: u32 = 2;
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

fn option(id: u32, kind: u32, npc_flag: u32, text: &str) -> GossipOption {
    GossipOption {
        id,
        kind,
        npc_flag,
        text: text.into(),
        ..Default::default()
    }
}

fn data(d: &mut GameData) {
    let base = d.creature_templates[&KOBOLD].clone();
    let npc = |entry: u32, flags: u32, menu: u32| {
        let mut t = base.clone();
        t.entry = entry;
        t.npc_flags = flags;
        t.faction = 35;
        t.gossip_menu_id = menu;
        t
    };
    // mangos's menu 708: a guild master's two options.
    d.creature_templates.insert(
        GUILD_MASTER,
        npc(GUILD_MASTER, 0x1 | 0x200 | 0x400, GUILD_MENU),
    );
    d.npc.gossip_menus.insert(
        GUILD_MENU,
        GossipMenu {
            texts: vec![],
            options: vec![
                option(0, 10, 0x200, "How do I form a guild?"),
                option(1, 11, 0x400, "I want to create a guild crest."),
            ],
        },
    );
    // A mage trainer on the default menu (0): its options as vmangos has
    // them, the quest giver's placeholder among them.
    let mut trainer = npc(TRAINER, 0x1 | 0x2 | 0x10, 0);
    trainer.trainer_type = 0;
    trainer.trainer_class = 8;
    d.creature_templates.insert(TRAINER, trainer);
    d.npc.trainers.insert(
        TRAINER,
        vec![TrainerSpell {
            spell: 133,
            cost: 10,
            required_level: 1,
            ..Default::default()
        }],
    );
    d.npc.gossip_menus.insert(
        0,
        GossipMenu {
            texts: vec![],
            options: vec![
                option(0, 2, 0x2, "GOSSIP_OPTION_QUESTGIVER"),
                option(1, 3, 0x4, "I want to browse your goods."),
                option(3, 5, 0x10, "Train me."),
                option(14, 16, 0x10, "I wish to unlearn my talents."),
                option(15, 17, 0x10, "I wish to untrain my pet."),
            ],
        },
    );
    // A flight master with a menu of its own, at a node.
    d.creature_templates
        .insert(FLIGHT_MASTER, npc(FLIGHT_MASTER, 0x1 | 0x8, FLIGHT_MENU));
    d.npc.gossip_menus.insert(
        FLIGHT_MENU,
        GossipMenu {
            texts: vec![],
            options: vec![option(0, 4, 0x8, "Show me where I can fly.")],
        },
    );
    d.npc.taxi_nodes.insert(
        NODE,
        TaxiNode {
            id: NODE,
            map: 0,
            x: START.0,
            y: START.1,
            z: START.2,
            name: "Northshire".into(),
            mounts: [0, 0],
        },
    );
}

async fn menu(c: &mut crate::game::harness::Client, npc: wow_world_messages::Guid) -> Vec<String> {
    c.clear();
    c.send(CMSG_GOSSIP_HELLO { guid: npc }).await;
    c.wait_for("SMSG_GOSSIP_MESSAGE", |m| match m {
        ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) if g.guid == npc => {
            Some(g.gossips.iter().map(|o| o.message.clone()).collect())
        }
        _ => None,
    })
    .await
}

/// A guild master's "How do I form a guild?" closes the window and shows
/// the charter for sale; "I want to create a guild crest." opens the
/// tabard designer (mangos's `GOSSIP_OPTION_PETITIONER` and
/// `GOSSIP_OPTION_TABARDDESIGNER`). Both did nothing but close it.
#[tokio::test]
async fn a_guild_masters_options_open_the_charter_and_the_tabard_designer() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P12KGUILD", "Founder", Class::Warrior).await;
    let npc = map
        .query(move |m| {
            kit::ready(m, a, 10, &[]);
            kit::spawn(m, a, GUILD_MASTER, 10, 3.0)
        })
        .await
        .unwrap();
    c.sees(npc).await;
    assert_eq!(
        menu(&mut c, npc).await,
        vec!["How do I form a guild?", "I want to create a guild crest."]
    );
    c.send(CMSG_GOSSIP_SELECT_OPTION {
        guid: npc,
        gossip_list_id: 0,
        unknown: None,
    })
    .await;
    c.wait_for("SMSG_GOSSIP_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
    })
    .await;
    let charter = c
        .wait_for("SMSG_PETITION_SHOWLIST", |m| match m {
            ServerOpcodeMessage::SMSG_PETITION_SHOWLIST(l) if l.npc == npc => Some(
                l.petitions
                    .iter()
                    .map(|p| p.charter_entry)
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .await;
    assert_eq!(charter, vec![crate::game::social::guilds::CHARTER_ITEM]);
    menu(&mut c, npc).await;
    c.send(CMSG_GOSSIP_SELECT_OPTION {
        guid: npc,
        gossip_list_id: 1,
        unknown: None,
    })
    .await;
    c.wait_for("MSG_TABARDVENDOR_ACTIVATE", |m| match m {
        ServerOpcodeMessage::MSG_TABARDVENDOR_ACTIVATE(t) if t.guid == npc => Some(()),
        _ => None,
    })
    .await;
}

/// A class trainer's options are for its class: a warrior at a mage
/// trainer is offered neither "Train me." nor to unlearn talents, a mage
/// of level 10 both; no one the quest giver's placeholder, a vendor's
/// option with nothing to sell, or untraining a pet they don't have
/// (mangos's `PrepareGossipMenu`).
#[tokio::test]
async fn a_trainers_options_are_for_its_class() {
    let realm = fast_realm(data);
    let (mut w, wg, map) = human(&realm, "P12KTRAINW", "Fighter", Class::Warrior).await;
    let mut m = crate::game::harness::Client::connect(&realm, "P12KTRAINM");
    let mg = m.enter("Caster", Race::Human, Class::Mage).await;
    let npc = map
        .query(move |m| {
            kit::ready(m, wg, 10, &[]);
            kit::ready(m, mg, 10, &[]);
            kit::spawn(m, wg, TRAINER, 10, 3.0)
        })
        .await
        .unwrap();
    w.sees(npc).await;
    m.sees(npc).await;
    assert_eq!(menu(&mut w, npc).await, Vec::<String>::new());
    assert_eq!(
        menu(&mut m, npc).await,
        vec!["Train me.", "I wish to unlearn my talents."]
    );
}

/// Shown a flight master's menu, a player learns its node at once, and is
/// told (`SMSG_NEW_TAXI_PATH` and the node's status); with no quests to
/// list, that is all they are sent. The next time, the menu (mangos's
/// `SendLearnNewTaxiNode` in `PrepareGossipMenu`). The node was learned
/// only by asking for the map.
#[tokio::test]
async fn a_flight_masters_node_is_learned_as_its_menu_is_shown() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P12KTAXI", "Traveller", Class::Warrior).await;
    let npc = map
        .query(move |m| {
            kit::ready(m, a, 10, &[]);
            let p = m.players.get_mut(&a).unwrap();
            p.row.state.taxi_mask.clear();
            kit::spawn(m, a, FLIGHT_MASTER, 10, 3.0)
        })
        .await
        .unwrap();
    c.sees(npc).await;
    c.clear();
    c.send(CMSG_GOSSIP_HELLO { guid: npc }).await;
    c.wait_for("SMSG_NEW_TAXI_PATH", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_TAXI_PATH).then_some(())
    })
    .await;
    let known = c
        .wait_for("SMSG_TAXINODE_STATUS", |m| match m {
            ServerOpcodeMessage::SMSG_TAXINODE_STATUS(s) if s.guid == npc => {
                Some(s.taxi_mask_node_known)
            }
            _ => None,
        })
        .await;
    assert!(known);
    let menu_too = c
        .gets(Duration::from_millis(300), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(_)).then_some(())
        })
        .await;
    assert!(menu_too.is_none(), "the discovery alone");
    let learned = map
        .query(move |m| crate::game::map::npc::knows_node(&m.players[&a].row.state.taxi_mask, NODE))
        .await
        .unwrap();
    assert!(learned);
    assert_eq!(menu(&mut c, npc).await, vec!["Show me where I can fly."]);
}
