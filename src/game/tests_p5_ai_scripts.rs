//! Round 5's creature script tests, ported from Thistle Tea's
//! `test/game/core/ai/creature_script/` (docs/parity/matrix.csv): which of
//! its ports stand in for their creatures' AI, and the quest NPCs' scripts
//! walked on the real world.

use std::io::Read;

use super::tests_p3_scripts::*;
use crate::game::data::scripts::ScriptStep;

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
