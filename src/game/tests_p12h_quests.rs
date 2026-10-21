//! Round 12's stage h, quests' highest level as mangos-classic loads and
//! checks it (`MaxLevel`, `SatisfyQuestLevel`, `CanSeeStartQuest`): a
//! battleground bracket's marks are for its bracket.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;
use crate::game::tests_p3_combat::{HUMANS, setup_with, spawn, template};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_player2_ammo::handled;

const GIVER: u32 = 97_150;
/// For levels 10 to 19 only.
const BRACKET: u32 = 97_050;

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.creature_templates.insert(
        GIVER,
        crate::game::data::CreatureTemplate {
            faction: HUMANS,
            npc_flags: crate::game::map::npc::NPC_FLAG_GOSSIP
                | crate::game::map::npc::NPC_FLAG_QUESTGIVER,
            ..template(GIVER, 7)
        },
    );
    d.npc.quests.insert(
        BRACKET,
        crate::game::data::npc::Quest {
            id: BRACKET,
            title: "Fight for Warsong Gulch".into(),
            level: 15,
            min_level: 10,
            max_level: 19,
            ..Default::default()
        },
    );
    d.npc.creature_starts.insert(GIVER, vec![BRACKET]);
    d
}

async fn status(c: &mut Client, giver: Guid) -> QuestGiverStatus {
    c.send(CMSG_QUESTGIVER_STATUS_QUERY { guid: giver }).await;
    c.wait_for("the quest status", |m| match m {
        ServerOpcodeMessage::SMSG_QUESTGIVER_STATUS(s) if s.guid == giver => Some(s.status),
        _ => None,
    })
    .await
}

/// Accepting `quest`: whether it went in the log, and the refusal if any.
async fn accept(
    c: &mut Client,
    map: &MapHandle,
    giver: Guid,
    quest: u32,
) -> (bool, Option<QuestFailedReason>) {
    c.clear();
    c.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: giver,
        quest_id: quest,
    })
    .await;
    handled(c).await;
    let refused = c
        .gets(std::time::Duration::from_millis(50), |m| match m {
            ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_INVALID(r) => Some(r.msg),
            _ => None,
        })
        .await;
    let g = c.guid;
    (
        on(map, move |m| m.players[&g].journal.has(quest)).await,
        refused,
    )
}

async fn set_level(map: &MapHandle, g: Guid, level: u32) {
    on(map, move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_LEVEL, level)
    })
    .await;
}

/// mangos's `SatisfyQuestLevel` and `CanSeeStartQuest`: past a quest's
/// `MaxLevel` it isn't shown (no "!") nor taken (`DONT_HAVE_REQ`); within
/// its levels it is. wvs loaded no `MaxLevel`, and a 60 took every
/// bracket's marks.
#[tokio::test]
async fn a_quest_past_its_highest_level_isnt_offered_or_taken() {
    let (_realm, mut a, g, map) = setup_with(world(), Class::Warrior).await;
    let giver = on(&map, move |m| spawn(m, g, GIVER, 20, 2.0)).await;
    a.sees(giver).await;
    set_level(&map, g, 20).await;
    assert_eq!(status(&mut a, giver).await, QuestGiverStatus::None);
    assert_eq!(
        accept(&mut a, &map, giver, BRACKET).await,
        (false, Some(QuestFailedReason::DontHaveReq))
    );
    set_level(&map, g, 19).await;
    assert_eq!(status(&mut a, giver).await, QuestGiverStatus::Available);
    assert_eq!(accept(&mut a, &map, giver, BRACKET).await, (true, None));
}

/// The real Warsong Gulch marks' quests carry their bracket's highest
/// level (175 quests have one, 92 below 60).
#[test]
#[ignore = "extended"]
fn the_real_bracket_quests_have_their_highest_level() {
    let d = harness::load_real(&harness::real_data("db"));
    let q = |id: u32| d.npc.quests.get(&id).map(|q| (q.min_level, q.max_level));
    assert_eq!(q(8389), Some((10, 19)), "Battle of Warsong Gulch");
    assert_eq!(q(8290), Some((10, 19)), "Vanquish the Invaders");
    let below_60 = d
        .npc
        .quests
        .values()
        .filter(|q| q.max_level != 0 && q.max_level < 60)
        .count();
    assert!(below_60 >= 80, "{below_60} below 60");
}
