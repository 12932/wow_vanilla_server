//! Round 5's creature script tests, ported from Thistle Tea's
//! `test/game/core/ai/creature_script/` (docs/parity/matrix.csv): which of
//! its ports stand in for their creatures' AI, and the quest NPCs' scripts
//! walked on the real world.

use std::io::Read;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use crate::game::data::scripts::ScriptStep;
use crate::game::map::Map;

/// The events Thistle Tea's dump has for `entry`, by id.
fn dumped_events(entry: u32) -> Vec<u32> {
    let mut json = String::new();
    flate2::read::GzDecoder::new(&include_bytes!("data/tt_scripts.json.gz")[..])
        .read_to_string(&mut json)
        .unwrap();
    let root: serde_json::Value = serde_json::from_str(&json).unwrap();
    root["events"]
        .get(entry.to_string())
        .and_then(|l| l.as_array())
        .map(|l| l.iter().filter_map(|e| e["id"].as_u64()).map(|i| i as u32).collect())
        .unwrap_or_default()
}

/// The steps of quest `quest`'s start (or end) script.
fn quest_steps(d: &crate::game::data::GameData, quest: u32, start: bool) -> Vec<ScriptStep> {
    let s = &d.scripts;
    let (ids, table) = if start {
        (&s.quest_start_script, &s.quest_start)
    } else {
        (&s.quest_complete_script, &s.quest_end)
    };
    ids.get(&quest)
        .and_then(|id| table.get(id))
        .cloned()
        .unwrap_or_default()
}

/// Thistle Tea's ports stand over their creatures' own AI: the combat
/// gadgets, Eris Havenfire and those her trial calls, the lazy peon, the
/// rabid bear, Riggle Bassbait, the amiable demons with Precious and The
/// Cleaner, and Triage's doctors and patients. Terenthis and Tervosh keep
/// theirs, with steps added to their quests: Selarin called at either of
/// Terenthis's, the Lady's blessing at Tervosh's; and Riggle's Master
/// Angler and both Triage quests have theirs (Thistle Tea's
/// `CreatureScript.ported?` and its quest steps).
#[tokio::test]
#[ignore = "extended"]
async fn the_ported_scripts_stand_over_their_creatures_own_ai() {
    let Some(realm) = realm() else {
        return;
    };
    let d = &realm.data;
    let ported = [
        8836, 12473, 8776, 9556, 10656, // the combat gadgets
        14494, 14484, 14485, 14486, 14489, // Eris and her trial
        10556, // the lazy peon
        2164,  // the rabid bear
        15077, // Riggle
        14531, 14529, 14536, 14527, 14528, 14503, // the demons, Precious, The Cleaner
        12920, 12939, 12923, 12924, 12925, 12936, 12937, 12938, // Triage
    ];
    for entry in ported {
        let dumped = dumped_events(entry);
        let installed: Vec<u32> = d.scripts.ai_events[&entry].iter().map(|e| e.id).collect();
        assert!(!installed.is_empty(), "{entry}");
        assert!(
            installed.iter().all(|i| dumped.contains(i)),
            "{entry}: {installed:?} not Thistle Tea's {dumped:?}"
        );
    }
    let peon: Vec<u32> = d.scripts.ai_events[&10556].iter().map(|e| e.id).collect();
    assert_eq!(peon, [1_055_601, 1_055_602, 1_055_603, 1_055_604]);
    for own in [3693, 4967] {
        assert!(dumped_events(own).is_empty(), "{own} keeps its own AI");
    }
    // Tervosh's turn-in: his words and the Lady's blessing on the player.
    let tervosh = quest_steps(d, 1265, false);
    assert!(tervosh.iter().any(|s| s.command == 0 && s.dataint[0] == 1751));
    assert!(
        tervosh
            .iter()
            .any(|s| s.command == 15 && s.datalong[0] == 7120 && s.datalong[1] & 0x2 != 0)
    );
    // Either of Terenthis's escapes brings Selarin.
    for quest in [994, 995] {
        let steps = quest_steps(d, quest, false);
        assert!(
            steps.iter().any(|s| s.command == 10 && s.datalong[0] == 3694),
            "{quest}"
        );
    }
    assert!(!quest_steps(d, 8193, false).is_empty(), "Master Angler");
    for quest in [6622, 6624] {
        assert!(!quest_steps(d, quest, true).is_empty(), "Triage {quest}");
    }
    for entry in [14484, 14485, 14486, 14489, 12923, 12937, 14528, 14503, 3694] {
        assert!(d.creature_template(entry).is_some(), "{entry}");
    }
}

const CHICKEN: u32 = 620;
const CLUCK: u32 = 3861;
const SPECIAL_CHICKEN_FEED: u32 = 11109;
const TEXT_EMOTE_CHEER: u32 = 21;
const TEXT_EMOTE_CHICKEN: u32 = 22;
const QUEST_GIVER: u32 = 0x2;

/// `player`'s text emote `emote` at `creature`, `times` over: whether it
/// gave quests after.
fn emoted_at(m: &mut Map, creature: Guid, player: Guid, emote: u32, times: u32) -> bool {
    for _ in 0..times {
        crate::game::map::event_ai::receive_emote(m, creature, player, emote);
        if npc_flags(m, creature) & QUEST_GIVER != 0 {
            return true;
        }
    }
    false
}

/// A chicken offers no quest as it spawns; a /chicken now and then (three
/// in a hundred) from one who never took CLUCK! has it offer it, but never
/// for one who has; and a /cheer from one with the feed in hand has it
/// take the quest back (Thistle Tea's chicken script, vmangos's
/// `npc_chicken_cluck`).
#[tokio::test]
#[ignore = "extended"]
async fn a_chicken_offers_cluck_to_a_lucky_chicken_and_takes_it_back_for_a_cheer() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = hero(&realm, "Clucker").await;
    let at = map.query(move |m| m.players[&me].position()).await.unwrap();
    let [lucky, taken, cheered] = map
        .query(move |m| [1.0, 2.0, 3.0].map(|d| summon(m, CHICKEN, off(at, d, 0.0), 0.0)))
        .await
        .unwrap();
    assert!(
        until(&map, 3_000, move |m| [lucky, taken, cheered]
            .iter()
            .all(|g| npc_flags(m, *g) & QUEST_GIVER == 0))
        .await,
        "no quest as it spawns"
    );
    // The chicken's three in a hundred: 400 tries miss once in 200,000.
    let offered = map
        .query(move |m| emoted_at(m, lucky, me, TEXT_EMOTE_CHICKEN, 400))
        .await
        .unwrap();
    assert!(offered, "a lucky /chicken");
    let after_taking = map
        .query(move |m| {
            crate::game::map::quests::accept_asked(m, me, lucky, CLUCK);
            assert!(m.players[&me].journal.has(CLUCK), "taken");
            emoted_at(m, taken, me, TEXT_EMOTE_CHICKEN, 400)
        })
        .await
        .unwrap();
    assert!(!after_taking, "never for one who has the quest");
    // The feed in hand, and a /cheer through the client.
    map.query(move |m| {
        crate::game::map::inventory::add_item(m, me, SPECIAL_CHICKEN_FEED, 1, false).unwrap();
    })
    .await;
    c.sees(cheered).await;
    c.send(CMSG_TEXT_EMOTE {
        text_emote: TextEmote::try_from(TEXT_EMOTE_CHEER).unwrap(),
        emote: 0,
        target: cheered,
    })
    .await;
    assert!(
        until(&map, 3_000, move |m| npc_flags(m, cheered) & QUEST_GIVER != 0).await,
        "the /cheer"
    );
}
