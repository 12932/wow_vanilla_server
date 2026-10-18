//! Round 5's creature script tests, ported from Thistle Tea's
//! `test/game/core/ai/creature_script/` (docs/parity/matrix.csv): which of
//! its ports stand in for their creatures' AI, and the quest NPCs' scripts
//! walked on the real world.

use std::io::Read;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use crate::game::data::scripts::ScriptStep;
use crate::game::fields::*;
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

const PLUCKY: u32 = 6626;
const TEXT_EMOTE_BECKON: u32 = 7;
const GOSSIP: u32 = 0x1;

/// Plucky Johnson, a chicken until shown otherwise: a /beckon from one not
/// on Get the Scoop leaves him one; a /chicken from anyone turns him back,
/// and he waves; and while he is himself, emotes change nothing (Thistle
/// Tea's port of vmangos's `npc_plucky_johnson`).
#[tokio::test]
#[ignore = "extended"]
async fn plucky_johnson_turns_back_for_a_chicken_but_not_a_stranger_s_beckon() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = hero(&realm, "Plucker").await;
    let at = map.query(move |m| m.players[&me].position()).await.unwrap();
    let plucky = summon_near(&map, PLUCKY, off(at, 2.0, 0.0)).await;
    c.sees(plucky).await;
    let emote = |e: u32| CMSG_TEXT_EMOTE {
        text_emote: TextEmote::try_from(e).unwrap(),
        emote: 0,
        target: plucky,
    };
    let chicken = map
        .query(move |m| (faction(m, plucky), npc_flags(m, plucky) & GOSSIP))
        .await
        .unwrap();
    c.send(emote(TEXT_EMOTE_BECKON)).await;
    sleep_ms(300).await;
    let after_beckon = map
        .query(move |m| (faction(m, plucky), npc_flags(m, plucky) & GOSSIP))
        .await
        .unwrap();
    assert_eq!(after_beckon, chicken, "a stranger's /beckon: still a chicken");
    assert_eq!(chicken.1, 0, "no talk from a chicken");
    c.clear();
    c.send(emote(TEXT_EMOTE_CHICKEN)).await;
    assert!(
        until(&map, 3_000, move |m| npc_flags(m, plucky) & GOSSIP != 0).await,
        "a /chicken turns him back"
    );
    let waved = c
        .gets(Duration::from_secs(2), |m| match m {
            ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == plucky => Some(e.emote),
            _ => None,
        })
        .await;
    assert_eq!(waved, Some(Emote::OneshotWave));
    // Himself, he answers no emote.
    let himself = map
        .query(move |m| (faction(m, plucky), npc_flags(m, plucky)))
        .await
        .unwrap();
    c.clear();
    c.send(emote(TEXT_EMOTE_BECKON)).await;
    c.send(emote(TEXT_EMOTE_CHICKEN)).await;
    sleep_ms(300).await;
    let still = map
        .query(move |m| (faction(m, plucky), npc_flags(m, plucky)))
        .await
        .unwrap();
    assert_eq!(still, himself);
    let again = c
        .gets(Duration::from_millis(300), |m| match m {
            ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == plucky => Some(()),
            _ => None,
        })
        .await;
    assert!(again.is_none(), "no second wave");
}

const YENNIKU: u32 = 2530;
const YENNIKUS_RELEASE: u32 = 3607;
const SAVING_YENNIKU: u32 = 592;

/// The emote `g` shows now: its state, else its last one-shot.
fn emote_state(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g)
        .map_or(0, |f| f.get_u32(UNIT_NPC_EMOTESTATE))
}

/// The Soul Gem does nothing to Yenniku for one off Saving Yenniku; for
/// one on it, it stuns him and turns him friendly, out of the fight, and a
/// second gem then changes nothing; when he evades he stops looking
/// stunned (Thistle Tea's port of vmangos's `npc_yenniku`).
#[tokio::test]
#[ignore = "extended"]
async fn the_soul_gem_tames_yenniku_only_for_one_on_the_quest() {
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = hero(&realm, "Gemmer").await;
    let at = map.query(move |m| m.players[&me].position()).await.unwrap();
    let yenniku = summon_near(&map, YENNIKU, off(at, 3.0, 0.0)).await;
    let read = move |m: &Map| {
        (
            crate::game::map::event_ai::phase(m, yenniku),
            faction(m, yenniku),
            fighting(m, yenniku),
        )
    };
    let (off_quest, start) = map
        .query(move |m| {
            crate::game::map::ai::attacked(m, yenniku, me);
            let start = read(m);
            crate::game::map::event_ai::spell_hit(m, yenniku, me, YENNIKUS_RELEASE, 1);
            (read(m), start)
        })
        .await
        .unwrap();
    assert_eq!(off_quest, start, "off the quest: nothing");
    assert!(start.2, "fighting");
    let (on_quest, stunned) = map
        .query(move |m| {
            crate::game::tests_p3_entity::take_quest(m, me, SAVING_YENNIKU);
            crate::game::map::event_ai::spell_hit(m, yenniku, me, YENNIKUS_RELEASE, 1);
            (read(m), emote_state(m, yenniku))
        })
        .await
        .unwrap();
    assert_eq!(on_quest, (1, 83, false), "the second phase, Horde, at peace");
    assert_eq!(stunned, 64, "he looks stunned");
    let again = map
        .query(move |m| {
            crate::game::map::event_ai::spell_hit(m, yenniku, me, YENNIKUS_RELEASE, 1);
            (read(m), emote_state(m, yenniku))
        })
        .await
        .unwrap();
    assert_eq!(again, (on_quest, 64), "a second gem: nothing new");
    let evaded = map
        .query(move |m| {
            crate::game::map::event_ai::evade(m, yenniku);
            emote_state(m, yenniku)
        })
        .await
        .unwrap();
    assert_eq!(evaded, 0, "no longer stunned-looking");
}
