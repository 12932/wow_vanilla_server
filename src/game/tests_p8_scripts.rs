//! Round 8's creature scripts, outside Alterac Valley: a fight's war
//! with a creature's faction that its fight never was for the player (a
//! test hero at war with Booty Bay, round 6's Shakes O'Breen), and a
//! creature's line aimed at another creature, its name filled in
//! (Kindal's "Quickly! $n attacks!").

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::misc::FactionDef;
use crate::game::data::npc::BroadcastText;
use crate::game::data::scripts::{AiEvent, EVENT_AGGRO, ScriptStep};
use crate::game::data::*;
use crate::game::fields::UNIT_FIELD_FLAGS;
use crate::game::map::Map;
use crate::game::map::reputation::{self, FLAG_AT_WAR};
use crate::game::tests_p3_combat::{BRUTE, HELPER, PACK, data, setup_with, spawn};
use crate::game::tests_p4_world::on;

const GUILD: u32 = 529;
const CRY: u32 = 9_480_001;

/// The brutes are of the guild's faction, at peace with a human.
fn world() -> GameData {
    let mut d = data();
    d.misc.factions.insert(
        GUILD,
        FactionDef {
            id: GUILD,
            index: 13,
            name: "The Guild".into(),
            variants: [(0x1, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)],
            ..Default::default()
        },
    );
    d.misc.faction_by_index.insert(13, GUILD);
    d.faction_templates.get_mut(&PACK).unwrap().faction = GUILD;
    d
}

fn at_war(m: &Map, g: Guid) -> bool {
    reputation::at_war(&m.players[&g], &m.realm.data, GUILD)
}

/// The war a creature's fight began ends when nothing holds the player,
/// whether or not they were ever in the fight themselves: one blow that
/// put them on a creature's list and killed it leaves them at peace with
/// its faction a moment later (it left a test hero at war with Booty Bay
/// after Shakes O'Breen's death, until his next fight ended). And a
/// fight's war is the fight's: a save during one keeps none of it.
#[tokio::test]
async fn a_fights_war_ends_with_no_one_holding_the_player_and_is_never_saved() {
    let (_realm, mut a, g, map) = setup_with(world(), Class::Warrior).await;
    let (war, fighting) = on(&map, move |m| {
        let brute = spawn(m, g, BRUTE, 20, 4.0);
        crate::game::map::combat::deal_damage(m, g, brute, 1_000_000);
        (
            at_war(m, g),
            m.players[&g].fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            ),
        )
    })
    .await;
    assert_eq!((war, fighting), (true, false), "at war, never in combat");
    // Told of the peace: the guild's index, no flag.
    let peace = a
        .wait_frame("SMSG_SET_FACTION_ATWAR", 0x313, |b| {
            b.len() == 5 && b[4] == 0
        })
        .await;
    assert_eq!(peace[..4], 13u32.to_le_bytes());
    assert!(!on(&map, move |m| at_war(m, g)).await, "at peace again");
    // In a fight, saved: no war kept.
    let (war, kept) = on(&map, move |m| {
        let brute = spawn(m, g, BRUTE, 20, 4.0);
        crate::game::map::ai::attacked(m, brute, g);
        let now = m.now;
        let row = m.players.get_mut(&g).unwrap().to_row(now);
        let kept = row
            .state
            .reputation
            .iter()
            .find(|s| s.faction == GUILD)
            .map_or(0, |s| s.flags & FLAG_AT_WAR);
        (at_war(m, g), kept)
    })
    .await;
    assert!(war, "at war for the fight");
    assert_eq!(kept, 0, "not for good");
}

/// A creature's line aimed at a creature names it, as one aimed at a
/// player does them: mangos passes the target's name with its guid
/// (`MonsterChatBuilder`), and this server fills `$n` in itself.
#[tokio::test]
async fn a_line_aimed_at_a_creature_names_it() {
    let mut d = data();
    d.npc.broadcast_texts.insert(
        CRY,
        BroadcastText {
            male: "Quickly! $n attacks!".into(),
            chat_type: 0,
            ..Default::default()
        },
    );
    d.scripts.ai_scripts.insert(
        CRY,
        vec![ScriptStep {
            id: CRY,
            command: 0,
            dataint: [CRY as i32, 0, 0, 0],
            ..Default::default()
        }],
    );
    d.scripts.ai_events.insert(
        HELPER,
        vec![AiEvent {
            id: CRY,
            creature: HELPER,
            kind: EVENT_AGGRO,
            chance: 100,
            actions: [CRY, 0, 0],
            ..Default::default()
        }],
    );
    let (_realm, mut a, g, map) = setup_with(d, Class::Warrior).await;
    let (helper, name) = on(&map, move |m| {
        let helper = spawn(m, g, HELPER, 20, 4.0);
        let brute = spawn(m, g, BRUTE, 20, 6.0);
        crate::game::map::ai::attacked(m, helper, brute);
        let name = m.realm.data.creature_template(BRUTE).unwrap().name.clone();
        (helper, name)
    })
    .await;
    let heard = a
        .gets(Duration::from_secs(2), move |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => match &c.chat_type {
                SMSG_MESSAGECHAT_ChatType::MonsterSay { sender1, .. } if *sender1 == helper => {
                    Some(c.message.clone())
                }
                _ => None,
            },
            _ => None,
        })
        .await;
    assert_eq!(heard, Some(format!("Quickly! {name} attacks!")));
}
