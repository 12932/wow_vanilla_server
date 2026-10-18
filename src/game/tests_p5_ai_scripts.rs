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
use crate::game::harness::Client;
use crate::game::map::{Map, MapHandle};

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

const RABID_THISTLE_BEAR: u32 = 2164;
const CAPTURED_BEAR: u32 = 11836;
const BEAR_CAPTURED_IN_TRAP: u32 = 9439;

/// The kills `g` has toward the quest that asks for `entry`.
fn kills_of(m: &Map, g: Guid, quest: u32) -> u32 {
    m.players[&g]
        .journal
        .log
        .iter()
        .find(|l| l.quest == quest)
        .map_or(0, |l| l.kills.iter().sum())
}

/// Another spell does nothing to a rabid thistle bear; the trap's
/// captures it for the trapper: it becomes the friendly captured bear,
/// counts for the trapper's quest, gives up its fight and follows them;
/// trapped again, it counts no more (Thistle Tea's port of vmangos's
/// `npc_rabid_thistle_bear`).
#[tokio::test]
#[ignore = "extended"]
async fn a_trapped_rabid_bear_is_captured_for_its_trapper_once() {
    let Some(realm) = realm() else {
        return;
    };
    let quest = *realm
        .data
        .npc
        .quests
        .iter()
        .find(|(_, q)| {
            q.required_kills
                .iter()
                .any(|(e, _)| *e == CAPTURED_BEAR as i32)
        })
        .expect("a quest for the captured bear")
        .0;
    let (_c, me, map) = hero(&realm, "Trapper").await;
    let at = map.query(move |m| m.players[&me].position()).await.unwrap();
    let bear = summon_near(&map, RABID_THISTLE_BEAR, off(at, 4.0, 0.0)).await;
    let (other, captured) = map
        .query(move |m| {
            crate::game::tests_p3_entity::take_quest(m, me, quest);
            crate::game::map::ai::attacked(m, bear, me);
            crate::game::map::event_ai::spell_hit(m, bear, me, 133, 4);
            let other = (m.creature(bear).unwrap().entry, kills_of(m, me, quest));
            crate::game::map::event_ai::spell_hit(m, bear, me, BEAR_CAPTURED_IN_TRAP, 1);
            let c = m.creature(bear).unwrap();
            (other, (c.entry, kills_of(m, me, quest)))
        })
        .await
        .unwrap();
    assert_eq!(other, (RABID_THISTLE_BEAR, 0), "another spell: nothing");
    assert_eq!(captured, (CAPTURED_BEAR, 1), "captured, and counted");
    assert!(
        until(&map, 3_000, move |m| at_peace(m, bear)).await,
        "its fight given up"
    );
    // It follows the trapper away.
    let away = off(at, -15.0, 0.0);
    map.query(move |m| put(m, me, away)).await;
    assert!(
        until(&map, 8_000, move |m| m
            .position_of(bear)
            .is_some_and(|p| crate::game::map::combat::dist(p, away) < 5.0))
        .await,
        "at the trapper's heels"
    );
    let again = map
        .query(move |m| {
            crate::game::map::event_ai::spell_hit(m, bear, me, BEAR_CAPTURED_IN_TRAP, 1);
            kills_of(m, me, quest)
        })
        .await
        .unwrap();
    assert_eq!(again, 1, "once");
}

const LAZY_PEON: u32 = 10556;
const PEON_SLEEP: u32 = 17743;
const AWAKEN_PEON: u32 = 19938;
const LAZY_PEONS: u32 = 5441;
const LUMBER_PILE: u32 = 175784;
const EMOTE_STATE_WORK_CHOPWOOD: u32 = 234;

/// A lazy peon in Durotar, asleep, falls asleep again whenever its sleep
/// lapses; another spell does nothing, but Awaken Peon wakes it, counting
/// once for the player's Lazy Peons however often it is hit; awake, it
/// walks to the nearest lumber pile and chops wood there, and at home it
/// sleeps again (Thistle Tea's port of vmangos's `npc_lazy_peon`).
#[tokio::test]
#[ignore = "extended"]
async fn a_lazy_peon_wakes_for_the_quest_once_chops_wood_and_sleeps_again() {
    let Some(realm) = realm() else {
        return;
    };
    let at = super::tests_p4_scripts_dungeons::spawn_of(&realm, 1, LAZY_PEON);
    let (mut c, me, _) = hero(&realm, "Waker").await;
    let map = go(&realm, &mut c, 1, at).await;
    let peon = {
        let mut found = None;
        for _ in 0..40 {
            found = map
                .query(move |m| {
                    m.creatures
                        .iter()
                        .filter(|(_, c)| c.entry == LAZY_PEON && c.alive())
                        .min_by(|(_, a), (_, b)| {
                            crate::game::map::combat::dist(a.position, at)
                                .total_cmp(&crate::game::map::combat::dist(b.position, at))
                        })
                        .map(|(_, c)| c.guid)
                })
                .await
                .flatten();
            if found.is_some() {
                break;
            }
            sleep_ms(100).await;
        }
        found.expect("a lazy peon")
    };
    assert!(
        until(&map, 5_000, move |m| has(m, peon, PEON_SLEEP)).await,
        "asleep"
    );
    map.query(move |m| crate::game::map::auras::remove_spell(m, peon, PEON_SLEEP, None))
        .await;
    assert!(
        until(&map, 5_000, move |m| has(m, peon, PEON_SLEEP)).await,
        "asleep again as its sleep lapses"
    );
    let (other, woken, again) = map
        .query(move |m| {
            crate::game::tests_p3_entity::take_quest(m, me, LAZY_PEONS);
            crate::game::map::event_ai::spell_hit(m, peon, me, 133, 4);
            let other = (kills_of(m, me, LAZY_PEONS), has(m, peon, PEON_SLEEP));
            crate::game::map::event_ai::spell_hit(m, peon, me, AWAKEN_PEON, 1);
            let woken = (
                kills_of(m, me, LAZY_PEONS),
                crate::game::map::event_ai::phase(m, peon),
            );
            crate::game::map::event_ai::spell_hit(m, peon, me, AWAKEN_PEON, 1);
            (other, woken, kills_of(m, me, LAZY_PEONS))
        })
        .await
        .unwrap();
    assert_eq!(other, (0, true), "another spell: still asleep");
    assert_eq!(woken, (1, 1), "counted, and up");
    assert_eq!(again, 1, "once a nap");
    assert!(
        until(&map, 4_000, move |m| !has(m, peon, PEON_SLEEP)).await,
        "awake"
    );
    // To the nearest pile, and to work.
    assert!(
        until(&map, 25_000, move |m| emote_state(m, peon) == EMOTE_STATE_WORK_CHOPWOOD).await,
        "chopping wood"
    );
    let (phase, by_pile) = map
        .query(move |m| {
            let p = m.position_of(peon).unwrap();
            let nearest = m
                .gameobjects
                .iter()
                .filter(|(_, o)| o.entry == LUMBER_PILE)
                .map(|(_, o)| crate::game::map::combat::dist(o.position, p))
                .fold(f32::MAX, f32::min);
            (crate::game::map::event_ai::phase(m, peon), nearest)
        })
        .await
        .unwrap();
    assert_eq!(phase, 2);
    assert!(by_pile < 6.0, "at a lumber pile: {by_pile}");
    // Home: asleep again.
    let home = map
        .query(move |m| {
            crate::game::map::event_ai::reached_home(m, peon);
            crate::game::map::event_ai::phase(m, peon)
        })
        .await
        .unwrap();
    assert_eq!(home, 0);
    assert!(
        until(&map, 5_000, move |m| has(m, peon, PEON_SLEEP)).await,
        "asleep at home"
    );
}

/// `entry` summoned by `at` and set on a dummy: the two.
async fn set_on_dummy(map: &MapHandle, entry: u32, at: Vector3d) -> (Guid, Guid) {
    map.query(move |m| {
        let g = summon(m, entry, off(at, 3.0, 0.0), 0.0);
        let d = dummy(m, off(at, 5.0, 0.0));
        tough(m, g);
        if let Some(f) = m.unit_fields_mut(g) {
            f.set_u32(UNIT_FIELD_FACTIONTEMPLATE, 14);
        }
        if let Some(f) = m.unit_fields_mut(d) {
            f.set_u32(UNIT_FIELD_MAXPOWER1, 10_000);
            f.set_u32(UNIT_FIELD_POWER1, 10_000);
        }
        crate::game::map::ai::attacked(m, g, d);
        (g, d)
    })
    .await
    .unwrap()
}

/// The engineers' combat gadgets in a fight: the battle chicken flies into
/// a fury as it starts and every 25 seconds, and squawks once, 30 to 80
/// seconds in; the arcanite dragonling buffets its victim 5 seconds in
/// and every 22.5, and breathes fire every 10 to 60; the emerald whelp
/// spits acid every two seconds; felhounds burn their victim's mana every
/// 9.8 to 15.2 (Thistle Tea's ports of vmangos's gadget AIs).
#[tokio::test]
#[ignore = "extended"]
async fn the_combat_gadgets_fight_with_their_own_spells_on_their_timers() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = hero(&realm, "Engineer").await;
    let at = map.query(move |m| m.players[&me].position()).await.unwrap();
    let (chicken, _) = set_on_dummy(&map, 8836, at).await;
    c.sees(chicken).await;
    assert!(
        until(&map, 3_000, move |m| has(m, chicken, 13168)).await,
        "a fury as it starts"
    );
    timed_cast(&mut c, &map, chicken, 883603, 13168, (25.0, 25.0), (25.0, 25.0), None).await;
    let squawk = timer(&map, chicken, 883601).await;
    assert!(between(squawk, 30.0, 80.0), "{squawk:?}");
    c.clear();
    map.query(move |m| {
        if let Some(cr) = m.caster_mut(chicken) {
            cr.gcd_until = None;
        }
    })
    .await;
    fire(&map, chicken, 883601).await;
    assert!(cast_any_of(&mut c, chicken, 23060, Duration::from_secs(3)).await.is_some());
    assert!(
        map.query(move |m| spent(m, chicken, 883601)).await.unwrap(),
        "squawks once"
    );
    clear(&map, vec![chicken]);
    for (entry, events) in [
        (12473, vec![(1247301, 9658, (5.0, 5.0), (22.5, 22.5)), (1247302, 8873, (10.0, 60.0), (10.0, 60.0))]),
        (8776, vec![(877601, 9591, (1.0, 1.0), (2.0, 2.0))]),
        (9556, vec![(955601, 15980, (1.0, 2.5), (9.8, 15.2))]),
        (10656, vec![(1065601, 15980, (1.0, 2.5), (9.8, 15.2))]),
    ] {
        let (g, d) = set_on_dummy(&map, entry, at).await;
        c.sees(g).await;
        for (id, spell, first, repeat) in events {
            timed_cast(&mut c, &map, g, id, spell, first, repeat, Some(d)).await;
        }
        clear(&map, vec![g, d]);
    }
}

/// A timed cast of a creature's kit: its event, the spell, its first and
/// repeat windows in seconds, and whether it goes at its victim (else
/// itself, or no unit).
type Timed = (u32, u32, (f32, f32), (f32, f32), bool);

/// `g`'s timed casts in its fight with dummy `d`, each as [`timed_cast`]
/// has it.
async fn kit(c: &mut Client, map: &MapHandle, g: Guid, d: Guid, casts: &[Timed]) {
    for (id, spell, first, repeat, at_victim) in casts.iter().copied() {
        timed_cast(c, map, g, id, spell, first, repeat, at_victim.then_some(d)).await;
    }
}

/// Felwood's oozes each wrap themselves in their own aura three seconds
/// into a fight and every minute; a dead one jarred with its own jar is
/// gone at once, but not with the other's (Thistle Tea's
/// `felwood_ooze`, vmangos's `npc_felwood_ooze`).
#[tokio::test]
#[ignore = "extended"]
async fn felwoods_oozes_wear_their_auras_and_go_into_their_jars() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = hero(&realm, "Jarrer").await;
    let at = map.query(move |m| m.players[&me].position()).await.unwrap();
    for (entry, id, aura, own_jar, other_jar) in [
        (7086, 708601, 13483, 15698, 15699),
        (7092, 709201, 3335, 15699, 15698),
    ] {
        let (g, d) = set_on_dummy(&map, entry, at).await;
        c.sees(g).await;
        if entry == 7086 {
            // Wither Touch, a passive, is worn rather than seen cast.
            assert!(about(timer(&map, g, id).await, 3.0), "three seconds in");
            fire(&map, g, id).await;
            assert!(map.query(move |m| has(m, g, aura)).await.unwrap(), "worn");
            assert!(about(timer(&map, g, id).await, 60.0), "a minute on");
        } else {
            kit(&mut c, &map, g, d, &[(id, aura, (3.0, 3.0), (60.0, 60.0), false)]).await;
        }
        let (other, own) = map
            .query(move |m| {
                kill(m, me, g);
                crate::game::map::event_ai::spell_hit(m, g, me, other_jar, 1);
                let other = m.creature(g).is_some();
                crate::game::map::event_ai::spell_hit(m, g, me, own_jar, 1);
                (other, m.creature(g).is_none())
            })
            .await
            .unwrap();
        assert!(other, "{entry}: the other jar");
        assert!(own, "{entry}: its own jar");
        clear(&map, vec![g, d]);
    }
}
