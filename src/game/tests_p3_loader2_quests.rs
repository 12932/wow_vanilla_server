//! Phase 3's second loader stage, the quests (Thistle Tea's
//! `world/loader/quest_vmangos_test.exs` and `quest_test.exs`): the event
//! links, the required condition trees and the quests' scripts vmangos
//! gives, over the real data when it is here, and a reward's spillover
//! mask on a world of the test's own.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::misc::{FactionDef, Spillover};
use crate::game::data::npc::Quest;
use crate::game::data::scripts::{Condition, ScriptStep};
use crate::game::harness::{self, Client};
use crate::game::map::conditions::{self, Truth};
use crate::game::map::{Map, MapHandle, MapKey};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    let db = DB.join("vmangos.sqlite");
    db.exists() && !crate::game::data::load::lfs_pointer(&db)
}

fn real() -> Option<&'static GameData> {
    crate::game::data::load::real_data()
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

/// Every condition in `id`'s tree, itself first.
fn tree(d: &GameData, id: u32) -> Vec<Condition> {
    let mut out = Vec::new();
    let mut todo = vec![id];
    while let Some(id) = todo.pop() {
        let Some(c) = d.scripts.conditions.get(&id).copied() else {
            continue;
        };
        if c.kind < 0 {
            todo.extend(c.values.iter().filter(|v| **v > 0).map(|v| *v as u32));
        }
        out.push(c);
    }
    out
}

fn start_steps(d: &GameData, quest: u32) -> &[ScriptStep] {
    let id = d.scripts.quest_start_script[&quest];
    &d.scripts.quest_start[&id]
}

fn complete_steps(d: &GameData, quest: u32) -> &[ScriptStep] {
    let id = d.scripts.quest_complete_script[&quest];
    &d.scripts.quest_end[&id]
}

fn step(steps: &[ScriptStep], command: u32) -> &ScriptStep {
    steps
        .iter()
        .find(|s| s.command == command)
        .unwrap_or_else(|| panic!("command {command}"))
}

fn object_spawn(d: &GameData, guid: u32) -> Option<&crate::game::data::GameObjectSpawn> {
    d.gameobject_spawns
        .values()
        .flatten()
        .find(|s| s.guid == guid)
}

/// Thistle Tea's world event links (`quest_vmangos_test.exs`): the
/// Midsummer, Lunar Festival and Hallow's End quests are tied to their
/// events up to patch 10, 63 in all; a quest of no event, and the war
/// effort's whose event `game_event` hasn't, are ordinary quests, offered
/// with no event on.
#[tokio::test]
#[ignore = "extended"]
async fn real_event_quests_are_tied_to_events_there_are() {
    if !have_data() {
        return;
    }
    let d = real().unwrap();
    for (quest, event) in [(172, 10), (1468, 10), (8311, 12), (8980, 8)] {
        assert_eq!(d.world.event_quests.get(&quest), Some(&event), "{quest}");
    }
    for quest in [54, 8795] {
        assert_eq!(d.world.event_quests.get(&quest), None, "{quest}");
    }
    let tied = d
        .world
        .event_quests
        .keys()
        .filter(|q| d.npc.quests.contains_key(q))
        .count();
    assert_eq!(tied, 63);
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    realm.social.events.lock().clear();
    assert!(crate::game::events::quest_offered(&realm, 8795));
    assert!(!crate::game::events::quest_offered(&realm, 172));
}

/// Thistle Tea's required condition trees (`quest_vmangos_test.exs`):
/// 76 quests have one, each of whose conditions has its row; Taste of
/// Flame's wants an item banked or carried, Zamek's Distraction a quest
/// still to take, and the Stratholme pair their copy's data. For a
/// sixtieth-level human in Elwynn the world judges 74, and leaves the two
/// in Stratholme to its copy: there, field 7 at 0 lets 5122 be taken and
/// at 2 lets 5125.
#[tokio::test]
#[ignore = "extended"]
async fn real_quest_condition_trees_resolve_and_are_judged() {
    if !have_data() {
        return;
    }
    let d = real().unwrap();
    let conditioned: Vec<(u32, u32)> = d
        .scripts
        .quest_condition
        .iter()
        .filter(|(q, c)| **c > 0 && d.npc.quests.contains_key(q))
        .map(|(q, c)| (*q, *c))
        .collect();
    assert_eq!(conditioned.len(), 76);
    for (quest, root) in &conditioned {
        let mut todo = vec![*root];
        while let Some(id) = todo.pop() {
            let c = d
                .scripts
                .conditions
                .get(&id)
                .unwrap_or_else(|| panic!("quest {quest}: condition {id}"));
            if c.kind < 0 {
                todo.extend(c.values.iter().filter(|v| **v > 0).map(|v| *v as u32));
            }
        }
    }
    // Item with bank (23), quest available (19, or not taken: 22).
    assert!(
        tree(d, d.scripts.quest_condition[&4022])
            .iter()
            .any(|c| c.kind == 23 && c.values[0] == 10_575)
    );
    assert!(
        tree(d, d.scripts.quest_condition[&1191])
            .iter()
            .any(|c| matches!(c.kind, 19 | 22) && c.values[0] == 1194)
    );
    assert!(d.scripts.quest_condition.get(&1194).is_none_or(|c| *c == 0));
    for (quest, entry) in [(5122, 3755), (5125, 3757)] {
        assert_eq!(d.scripts.quest_condition[&quest], entry);
        assert_eq!(d.scripts.conditions[&entry].kind, 34, "instance data");
    }

    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let mut c = Client::connect(&realm, "P3OQCOND");
    let g = c.enter("Condita", Race::Human, Class::Warrior).await;
    let world = realm.maps.get(&realm, MapKey::world(0));
    let data = realm.data.clone();
    let list = conditioned.clone();
    let judged = on(&world, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::stats::set_level(p, &data, 60);
        list.iter()
            .map(|(q, root)| (*q, conditions::check(m, *root, None, Some(g))))
            .collect::<Vec<_>>()
    })
    .await;
    let unknown: Vec<u32> = judged
        .iter()
        .filter(|(_, t)| *t == Truth::Unknown)
        .map(|(q, _)| *q)
        .collect();
    assert_eq!(unknown, Vec::<u32>::new(), "the world judges every one");
    // Out of their copy, the Stratholme pair are shut.
    for q in [5122, 5125] {
        assert_eq!(
            judged.iter().find(|(x, _)| *x == q).map(|(_, t)| *t),
            Some(Truth::Unmet),
            "{q}"
        );
    }
    let strat = realm.maps.get(
        &realm,
        MapKey {
            map: 329,
            instance: 7,
        },
    );
    let by_field = on(&strat, move |m| {
        [0, 2].map(|v| {
            m.instance.data.insert(7, v);
            (
                conditions::check(m, 3755, None, Some(g)),
                conditions::check(m, 3757, None, Some(g)),
            )
        })
    })
    .await;
    assert_eq!(
        by_field,
        [(Truth::Met, Truth::Unmet), (Truth::Unmet, Truth::Met)],
        "field 7 at 0, then at 2"
    );
}

/// Thistle Tea's quests' scripts (`quest_vmangos_test.exs`): A Watchful
/// Eye's start casts, emotes and talks; Rescue OOX-17/TN!'s starts a map
/// event that an escort's failure fails (64801) and its success (64802)
/// completes; Wanted: Lieutenant Fangore's... ends by opening a spawned
/// object; the Pilgrimage's respawns a spawned bench; Mor'Ladim's
/// Gnoll's... ends dressed in two items; a door opens; and a map event
/// ends a script.
#[test]
#[ignore = "extended"]
fn the_real_quest_scripts_load_with_what_they_name() {
    let Some(d) = real() else {
        return;
    };
    let commands: Vec<(u32, u32)> = start_steps(d, 54)
        .iter()
        .map(|s| (s.command, s.datalong[0]))
        .collect();
    assert_eq!(commands, vec![(15, 6245), (1, 113), (0, 0)]);
    // 648: the map event, its failure on the escort's condition.
    let event = step(start_steps(d, 648), 61);
    assert_eq!(
        (event.datalong[0], event.dataint[1..4].to_vec()),
        (648, vec![64_802, 1019, 64_801])
    );
    let failure = d.scripts.conditions[&1019];
    assert_eq!((failure.kind, failure.values[1]), (25, 80), "escort");
    let fail = &d.scripts.generic[&64_801];
    assert!(fail.iter().any(|s| s.command == 70 && s.datalong[0] == 648));
    assert!(!d.scripts.generic[&64_802].is_empty());
    // 848: an object by its spawn (target type 14).
    let activate = step(complete_steps(d, 848), 13);
    assert_eq!((activate.target_type, activate.target_param1), (14, 13_292));
    assert!(object_spawn(d, 13_292).is_some());
    // 308: a respawn of spawn 35875, a bench (270).
    let respawn = step(complete_steps(d, 308), 9);
    assert_eq!(respawn.datalong[0], 35_875);
    assert_eq!(object_spawn(d, 35_875).map(|s| s.entry), Some(270));
    // 5713: a step that ends the script while its map event is on.
    let ends = step(start_steps(d, 5713), 32);
    let c = d.scripts.conditions[&ends.datalong[0]];
    assert_eq!((ends.datalong[0], c.kind), (5713, 36), "map event active");
    // 112: the equipment, two items that are there, the third kept (-1).
    let gear = complete_steps(d, 112)
        .iter()
        .find(|s| s.command == 19 && s.delay_ms == 0)
        .unwrap();
    assert_eq!(gear.dataint[..3], [3699, 3697, -1]);
    for item in [3699, 3697] {
        assert!(d.item(item).is_some(), "{item}");
    }
    // 6482: a door by its spawn.
    let door = step(start_steps(d, 6482), 11);
    assert_eq!(door.datalong[0], 48_166);
    assert!(object_spawn(d, 48_166).is_some());
}

const STORMWIND: u32 = 72;
const IRONFORGE: u32 = 47;

/// Thistle Tea's reward reputations (`quest_test.exs`): a reward the
/// quest's `RewRepSpilloverMask` marks spills to no related faction; one
/// it doesn't, does.
#[tokio::test]
async fn a_quests_spillover_mask_keeps_a_reward_to_its_faction() {
    let mut d = GameData::empty();
    for (id, index) in [(STORMWIND, 0), (IRONFORGE, 1)] {
        d.misc.factions.insert(
            id,
            FactionDef {
                id,
                index,
                name: format!("Faction {id}"),
                variants: [(1, 0, 0, 0x1), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)],
                ..Default::default()
            },
        );
        d.misc.faction_by_index.insert(index, id);
    }
    d.misc.spillovers.insert(
        STORMWIND,
        vec![Spillover {
            faction: IRONFORGE,
            rate: 0.5,
            max_rank: crate::game::data::misc::RANK_EXALTED,
        }],
    );
    let realm = harness::realm_with(d);
    let mut c = Client::connect(&realm, "P3OSPILL");
    let g: Guid = c.enter("Spilla", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let reward = |mask| Quest {
        id: 1,
        level: 1,
        reputation_rewards: [(STORMWIND, 100), (0, 0), (0, 0), (0, 0), (0, 0)],
        reputation_no_spillover: mask,
        ..Default::default()
    };
    let (kept, spilled) = (reward(0x1), reward(0x2));
    let out = on(&map, move |m| {
        let standing = |m: &Map| {
            let p = &m.players[&g];
            let d = &m.realm.data;
            [STORMWIND, IRONFORGE].map(|f| crate::game::map::reputation::standing(p, d, f))
        };
        let before = standing(m);
        crate::game::map::reputation::quest_rewarded(m, g, &kept);
        let after_kept = standing(m);
        crate::game::map::reputation::quest_rewarded(m, g, &spilled);
        let after_spilled = standing(m);
        [0, 1].map(|i| (after_kept[i] - before[i], after_spilled[i] - after_kept[i]))
    })
    .await;
    assert_eq!(out[0], (100, 100), "Stormwind gains both");
    assert_eq!(out[1], (0, 50), "Ironforge only the unmarked");
}

/// Thistle Tea's empty texts (`quest_test.exs`): a quest's NULL text
/// columns load as empty strings (272 quests have no `EndText`).
#[test]
#[ignore = "extended"]
fn the_real_quests_null_texts_are_empty() {
    let Some(d) = real() else {
        return;
    };
    let q = &d.npc.quests[&171];
    assert!(!q.title.is_empty() && !q.details.is_empty());
    assert_eq!(q.end_text, "");
    assert_eq!(d.npc.quests[&636].offer_reward_text, "");
    // Avast Ye, Admiral!'s gates and objective: below Booty Bay's
    // friendly, to reach the Bloodsail's.
    let q = &d.npc.quests[&1036];
    assert_eq!(
        (q.required_max_rep, q.rep_objective, q.required_min_rep),
        ((21, 1), (87, 3000), (0, 0))
    );
    assert_eq!(d.npc.quests[&6].reputation_no_spillover, 1);
}
