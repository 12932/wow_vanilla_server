//! The user's second batch of answers (2026-10-19), each as it is now: a
//! second login takes the character over (#35), fears break by chance
//! (#43), an interrupt locks out a silence-immune creature's school (#42),
//! attack-speed slows multiply (#30), cancelling Stealth takes Vanish's too
//! (#41), the quest log's slots are the group's (#47), the grey "!" shows
//! to seven levels short and is a setting (#21), locked doors and buttons
//! want their key by hand (#7); and a talent's trained ranks want the
//! talent at a trainer. Fade's threat (#46) is in `tests_p3_combat2`.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::{LASTS_15S, aura, bare, damage, effect, fast_realm, human, insert};
use super::tests_p3_spells_kit::{at, casting, dummy, ready};
use crate::game::Realm;
use crate::game::data::npc::Quest;
use crate::game::data::spells::{SkillAbility, SpellTemplate, TalentInfo};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client, WAIT};
use crate::game::map::{Map, MapHandle, auras, objects, quests, spells};
use crate::game::store::Logged;

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map")
}

/// The map `g` is on.
fn map_of(realm: &Arc<Realm>, g: Guid) -> MapHandle {
    realm.registry.online(g).expect("in the world").map()
}

/// Wait for what `c` has sent so far to have been handled: a query of the
/// map answered after it.
async fn handled(c: &mut Client) {
    c.send(CMSG_QUESTGIVER_STATUS_QUERY {
        guid: guid::unit(1, 1),
    })
    .await;
    c.wait_for("SMSG_QUESTGIVER_STATUS", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_QUESTGIVER_STATUS(_)).then_some(())
    })
    .await;
}

fn npc(entry: u32, flags: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: format!("Npc {entry}"),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        display_scales: [1.0, 0.0, 0.0, 0.0],
        level_min: 10,
        level_max: 10,
        faction: 35,
        unit_class: 1,
        health_max: 1000,
        npc_flags: flags,
        ..Default::default()
    }
}

fn beside(guid: u32, entry: u32, dx: f32) -> CreatureSpawn {
    CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 600,
        respawn_max: 600,
        ..Default::default()
    }
}

// ---- #21: the grey "!" -------------------------------------------------

/// Givers of quests 7, 8 and 29 levels above a new character's.
const SEVEN_UP: u32 = 9210;
const EIGHT_UP: u32 = 9211;
const FAR_UP: u32 = 9212;

fn grey_data() -> GameData {
    let mut d = GameData::empty();
    for (giver, min_level) in [(SEVEN_UP, 8), (EIGHT_UP, 9), (FAR_UP, 30)] {
        let id = giver + 100;
        d.npc.quests.insert(
            id,
            Quest {
                id,
                title: format!("Quest {id}"),
                min_level,
                level: i32::from(min_level),
                ..Default::default()
            },
        );
        d.npc.creature_starts.insert(giver, vec![id]);
    }
    d
}

async fn giver_status(c: &mut Client, giver: u32) -> QuestGiverStatus {
    let g = guid::unit(giver, 1);
    c.send(CMSG_QUESTGIVER_STATUS_QUERY { guid: g }).await;
    c.wait_for("SMSG_QUESTGIVER_STATUS", |m| match m {
        ServerOpcodeMessage::SMSG_QUESTGIVER_STATUS(s) if s.guid == g => Some(s.status),
        _ => None,
    })
    .await
}

/// A giver shows a level-1 character the grey "!" for a quest seven levels
/// up, and nothing for one eight up, as mangos's `Quests.HighLevelHideDiff`
/// does by default (the user's #21). Before, only four levels up showed it.
#[tokio::test]
async fn the_grey_exclamation_shows_to_seven_levels_short() {
    let realm = harness::realm_with(grey_data());
    let mut a = Client::connect(&realm, "GREY");
    a.enter("Greyone", Race::Human, Class::Warrior).await;
    assert_eq!(
        giver_status(&mut a, SEVEN_UP).await,
        QuestGiverStatus::Unavailable
    );
    assert_eq!(giver_status(&mut a, EIGHT_UP).await, QuestGiverStatus::None);
    assert_eq!(giver_status(&mut a, FAR_UP).await, QuestGiverStatus::None);
}

/// `[quests] high_level_hide_diff` is the gap, live: -1 shows the grey "!"
/// at any gap, 0 only for quests of the player's own level.
#[tokio::test]
async fn the_grey_exclamations_gap_is_a_setting() {
    let realm = harness::realm_with(grey_data());
    let mut a = Client::connect(&realm, "GREYS");
    let g = a.enter("Greytwo", Race::Human, Class::Warrior).await;
    let map = map_of(&realm, g);
    let shown = on(&map, move |m| {
        let mut shown = Vec::new();
        for gap in [-1, 0, 8] {
            let mut cfg = (*m.cfg).clone();
            cfg.quests.high_level_hide_diff = gap;
            m.cfg = Arc::new(cfg);
            shown.push(
                [SEVEN_UP, EIGHT_UP, FAR_UP]
                    .map(|giver| quests::status(m, g, guid::unit(giver, 1))),
            );
        }
        shown
    })
    .await;
    use QuestGiverStatus::{None as Nothing, Unavailable as Grey};
    assert_eq!(shown[0], [Grey, Grey, Grey], "any gap");
    assert_eq!(shown[1], [Nothing, Nothing, Nothing], "none");
    assert_eq!(shown[2], [Grey, Grey, Nothing], "eight");
}
