//! Round 11, against Thistle Tea's `gossip_test.exs` ("starts the shown
//! greeting's script on its speaker"): a gossip menu's script is its
//! greeting's, started only when that greeting is the one shown (mangos-
//! classic's `Player::GetGossipTextId`, each `gossip_menu` row's
//! `script_id`).

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit};
use crate::game::data::GameData;
use crate::game::data::npc::GossipMenu;
use crate::game::data::scripts::{Condition, ScriptStep};

/// A greeter whose menu has a greeting of no condition with a script, and
/// one of a condition that holds without; and a second, the other way
/// about.
const SECOND: u32 = 7;
const MENU_A: u32 = 60_910;
const MENU_B: u32 = 60_911;
const HOLDS: u32 = 60_912;
const SCRIPT_A: u32 = 60_913;
const SCRIPT_B: u32 = 60_914;

fn data(d: &mut GameData) {
    let mut a = d.creature_templates[&KOBOLD].clone();
    a.npc_flags = 1;
    a.faction = 35;
    a.gossip_menu_id = MENU_A;
    let mut b = a.clone();
    b.entry = SECOND;
    b.gossip_menu_id = MENU_B;
    d.creature_templates.insert(KOBOLD, a);
    d.creature_templates.insert(SECOND, b);
    // Any race and class.
    d.scripts.conditions.insert(
        HOLDS,
        Condition {
            entry: HOLDS,
            kind: 14,
            values: [0; 4],
            flags: 0,
        },
    );
    for (menu, plain, conditioned) in [(MENU_A, 10, 11), (MENU_B, 20, 21)] {
        d.npc.gossip_menus.insert(
            menu,
            GossipMenu {
                texts: vec![(plain, 0), (conditioned, HOLDS)],
                ..Default::default()
            },
        );
    }
    // A's script is on its plain greeting, B's on its conditioned one.
    d.scripts.gossip_menu_script.insert((MENU_A, 10), SCRIPT_A);
    d.scripts.gossip_menu_script.insert((MENU_B, 21), SCRIPT_B);
    for (script, emote) in [(SCRIPT_A, 1), (SCRIPT_B, 2)] {
        d.scripts.gossip.insert(
            script,
            vec![ScriptStep {
                id: script,
                command: 1,
                datalong: [emote, 0, 0, 0],
                ..Default::default()
            }],
        );
    }
}

/// Each greeter greets with its conditioned text, the condition holding;
/// the script of the plain greeting not shown doesn't start, and the shown
/// one's does, on its speaker: the second greeter bows.
#[tokio::test]
async fn a_menus_script_is_the_shown_greetings() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P11GGOSSIP", "Listener", Class::Warrior).await;
    let (first, second) = map
        .query(move |m| {
            kit::ready(m, a, 10, &[]);
            (
                kit::spawn(m, a, KOBOLD, 10, 3.0),
                kit::spawn(m, a, SECOND, 10, 4.0),
            )
        })
        .await
        .unwrap();
    c.sees(first).await;
    c.sees(second).await;
    let mut shown = Vec::new();
    for npc in [first, second] {
        c.clear();
        c.send(CMSG_GOSSIP_HELLO { guid: npc }).await;
        let text = c
            .wait_for("SMSG_GOSSIP_MESSAGE", |m| match m {
                ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) => Some(g.title_text_id),
                _ => None,
            })
            .await;
        let emote = c
            .gets(std::time::Duration::from_millis(400), |m| match m {
                ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == npc => Some(e.emote),
                _ => None,
            })
            .await;
        shown.push((text, emote));
    }
    assert_eq!(
        shown,
        vec![(11, None), (21, Some(Emote::OneshotBow))],
        "the shown greeting's script alone"
    );
}
