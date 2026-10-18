//! Phase 3's Stratholme, Thistle Tea's `InstanceScriptTest` ported for the
//! Baron and the ziggurats: the Baron's gates shut in his fight and open
//! after it; the timed run started once from the gauntlet's gate, its
//! milestones while it runs, won with the Baron's death or lost at its
//! end; each ziggurat opened by its boss; Timmy, and Willey's gate.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::instance_script::{
    self, DONE, Event, FAILED, IN_PROGRESS, Script, get_data, set_data,
};
use crate::game::map::{Map, MapKey};

const STRATHOLME: u32 = 329;
const AT: (f32, f32, f32) = (3_395.0, -3_380.0, 143.0);
/// The Baron's gates, the gauntlet's gate, the ziggurats' doors.
const GATES: [u32; 3] = [175_405, 175_796, 175_374];
const GAUNTLET_GATE: u32 = 175_357;
const ZIGGURATS: [(u32, u32, u32); 3] = [
    (10_436, 1, 175_380),
    (10_437, 2, 175_379),
    (10_438, 3, 175_381),
];
const BARON: u32 = 10_440;
const YSIDA: u32 = 16_031;
/// The run's first spell, and its ten-minute milestone's.
const RUN_SPELL: u32 = 27_861;
const GUARDSMAN: u32 = 10_418;
const TIMMY: u32 = 10_808;
const WILLEY: u32 = 10_997;
const WILLEYS_GATE: u32 = 175_969;
const ACOLYTE: u32 = 10_399;
const CRYSTAL: u32 = 10_415;
const TEN_MINUTES: u32 = 27_863;

fn data() -> GameData {
    let mut d = GameData::empty();
    d.maps.insert(
        STRATHOLME,
        MapInfo {
            id: STRATHOLME,
            internal_name: "Stratholme".into(),
            name: "Stratholme".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    let objects: Vec<u32> = GATES
        .iter()
        .copied()
        .chain([GAUNTLET_GATE])
        .chain(ZIGGURATS.iter().map(|(_, _, door)| *door))
        .collect();
    for (k, entry) in objects.iter().enumerate() {
        d.gameobject_templates.insert(
            *entry,
            GameObjectTemplate {
                entry: *entry,
                kind: 0,
                display_id: 1,
                name: format!("Gate {entry}"),
                size: 1.0,
                ..Default::default()
            },
        );
        d.gameobject_spawns
            .entry(STRATHOLME)
            .or_default()
            .push(GameObjectSpawn {
                guid: 100 + k as u32,
                entry: *entry,
                map: STRATHOLME,
                x: AT.0 + k as f32,
                y: AT.1,
                z: AT.2,
                rotation: [0.0, 0.0, 0.0, 1.0],
                // The ziggurats' doors shut, the rest open.
                state: u8::from(ZIGGURATS.iter().any(|(_, _, door)| door == entry)),
                ..Default::default()
            });
    }
    let creature = |entry: u32, name: &str| CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        display_scales: [1.0, 0.0, 0.0, 0.0],
        level_min: 60,
        level_max: 60,
        faction: 35,
        unit_class: 1,
        health_max: 1_000,
        ..Default::default()
    };
    d.creature_templates.insert(YSIDA, creature(YSIDA, "Ysida"));
    d.creature_templates.insert(BARON, creature(BARON, "Baron"));
    for (k, (boss, _, _)) in ZIGGURATS.iter().enumerate() {
        d.creature_templates.insert(*boss, creature(*boss, "Boss"));
        d.creature_spawns
            .entry(STRATHOLME)
            .or_default()
            .push(CreatureSpawn {
                guid: 200 + k as u32,
                entries: vec![*boss],
                map: STRATHOLME,
                x: AT.0 - 3.0 - k as f32,
                y: AT.1,
                z: AT.2,
                ..Default::default()
            });
    }
    // Timmy's guardsmen, Willey and his gate.
    for (entry, name) in [
        (GUARDSMAN, "Guardsman"),
        (TIMMY, "Timmy"),
        (WILLEY, "Willey"),
    ] {
        d.creature_templates.insert(entry, creature(entry, name));
    }
    for (spawn, entry) in [(54_069, GUARDSMAN), (54_070, GUARDSMAN), (300, WILLEY)] {
        d.creature_spawns
            .entry(STRATHOLME)
            .or_default()
            .push(CreatureSpawn {
                guid: spawn,
                entries: vec![entry],
                map: STRATHOLME,
                x: AT.0,
                y: AT.1 + 3.0,
                z: AT.2,
                ..Default::default()
            });
    }
    d.gameobject_templates.insert(
        WILLEYS_GATE,
        GameObjectTemplate {
            entry: WILLEYS_GATE,
            kind: 0,
            display_id: 1,
            name: "Willey's Gate".into(),
            size: 1.0,
            ..Default::default()
        },
    );
    d.gameobject_spawns
        .entry(STRATHOLME)
        .or_default()
        .push(GameObjectSpawn {
            guid: 150,
            entry: WILLEYS_GATE,
            map: STRATHOLME,
            x: AT.0,
            y: AT.1 - 3.0,
            z: AT.2,
            rotation: [0.0, 0.0, 0.0, 1.0],
            state: 0,
            ..Default::default()
        });
    // The first crystal and its five acolytes, and an acolyte of the
    // second's; spell 5 kills.
    d.creature_templates
        .insert(ACOLYTE, creature(ACOLYTE, "Acolyte"));
    d.creature_templates
        .insert(CRYSTAL, creature(CRYSTAL, "Crystal"));
    for (spawn, entry) in [
        (53_268, ACOLYTE),
        (53_269, ACOLYTE),
        (53_270, ACOLYTE),
        (53_271, ACOLYTE),
        (53_272, ACOLYTE),
        (53_257, ACOLYTE),
        (53_955, CRYSTAL),
    ] {
        d.creature_spawns
            .entry(STRATHOLME)
            .or_default()
            .push(CreatureSpawn {
                guid: spawn,
                entries: vec![entry],
                map: STRATHOLME,
                x: AT.0 + 3.0,
                y: AT.1 + 3.0,
                z: AT.2,
                ..Default::default()
            });
    }
    let mut kill = SpellTemplate {
        id: 5,
        name: "Death Touch".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    kill.effects[0] = SpellEffect {
        effect: 1,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(5, kill);
    for id in [RUN_SPELL, TEN_MINUTES] {
        let mut s = SpellTemplate {
            id,
            name: format!("Run {id}"),
            equipped_item_class: -1,
            duration_index: 0,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 6,
            aura: 4,
            target_a: 1,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    d
}

/// Alice in her copy of Stratholme, its script Stratholme's.
async fn inside() -> (Arc<Realm>, Client, MapKey) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let from = realm.registry.online(g).unwrap().map();
    from.call(move |m| {
        m.teleport(
            g,
            MapKey::world(STRATHOLME),
            Vector3d {
                x: AT.0,
                y: AT.1,
                z: AT.2,
            },
            0.0,
        )
    });
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(g).await;
    let key = realm.registry.online(g).unwrap().map().key;
    let (realm2, k) = (realm.clone(), key);
    realm2
        .maps
        .get(&realm2, k)
        .query(|m| m.instance.script = Some(Script::Stratholme))
        .await
        .unwrap();
    (realm, a, key)
}

async fn on<T: Send + 'static>(
    realm: &Arc<Realm>,
    key: MapKey,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, key)
        .query(f)
        .await
        .expect("the copy answers")
}

/// Whether each object of `entries` is open (state 0).
fn open(m: &Map, entries: &[u32]) -> Vec<bool> {
    entries
        .iter()
        .map(|e| {
            m.gameobjects
                .iter()
                .map(|(_, x)| x)
                .find(|o| o.entry == *e)
                .is_some_and(|o| o.fields.get_u32(GAMEOBJECT_STATE) == 0)
        })
        .collect()
}

fn count(m: &Map, entry: u32) -> usize {
    m.creatures
        .iter()
        .map(|(_, x)| x)
        .filter(|c| c.entry == entry && c.alive())
        .count()
}

/// Thistle Tea's Baron gates: shut while he is fought, open when he
/// isn't.
#[tokio::test]
async fn the_barons_gates_shut_for_his_fight() {
    let (realm, _a, key) = inside().await;
    let seen = on(&realm, key, |m| {
        set_data(m, 5, IN_PROGRESS, 0);
        let shut = open(m, &GATES);
        set_data(m, 5, FAILED, 0);
        (shut, open(m, &GATES))
    })
    .await;
    assert_eq!(seen, (vec![false; 3], vec![true; 3]));
}

/// Thistle Tea's timed run: the gauntlet's gate starts it once (Ysida
/// waiting, the run's spell on the players, its five timers); its
/// ten-minute milestone casts while it runs; the Baron dead, it is won,
/// his gates open and Ysida's reward to come; else at its end it is lost,
/// and only once.
#[tokio::test]
async fn the_barons_run_starts_once_and_ends_won_or_lost() {
    let (realm, a, key) = inside().await;
    let g = a.guid;
    let started = on(&realm, key, move |m| {
        instance_script::object_used(m, GAUNTLET_GATE);
        instance_script::object_used(m, GAUNTLET_GATE);
        (
            get_data(m, 0),
            count(m, YSIDA),
            m.instance.timers.len(),
            crate::game::map::auras::auras(m, g).is_some_and(|x| x.has_spell(RUN_SPELL)),
        )
    })
    .await;
    assert_eq!(started, (IN_PROGRESS, 1, 5, true), "started once");
    // The first milestone, brought due.
    let milestone = on(&realm, key, move |m| {
        let first = m.instance.timers.iter().map(|(t, _)| *t).min().unwrap();
        for (t, _) in m.instance.timers.iter_mut() {
            if *t == first {
                *t = m.now;
            }
        }
        instance_script::update(m);
        crate::game::map::auras::auras(m, g).is_some_and(|x| x.has_spell(TEN_MINUTES))
    })
    .await;
    assert!(milestone, "ten minutes left");
    let won = on(&realm, key, |m| {
        set_data(m, 5, DONE, 0);
        (
            get_data(m, 0),
            get_data(m, 5),
            open(m, &GATES),
            m.instance.timers.len(),
        )
    })
    .await;
    assert_eq!(
        won,
        (DONE, DONE, vec![true; 3], 1),
        "won; only the reward due"
    );

    // Another copy's run, out of time: lost, and only once.
    let (realm, _b, key) = inside().await;
    let lost = on(&realm, key, |m| {
        instance_script::object_used(m, GAUNTLET_GATE);
        let last = m.instance.timers.iter().map(|(t, _)| *t).max().unwrap();
        for (t, _) in m.instance.timers.iter_mut() {
            if *t == last {
                *t = m.now;
            }
        }
        instance_script::update(m);
        let lost = get_data(m, 0);
        instance_script::update(m);
        (lost, get_data(m, 0), m.instance.timers.len())
    })
    .await;
    assert_eq!(lost, (FAILED, FAILED, 0));
}

/// Thistle Tea's ziggurats: each boss's death marks its field done and
/// opens its door.
#[tokio::test]
async fn each_ziggurat_opens_at_its_bosss_death() {
    let (realm, _a, key) = inside().await;
    let seen = on(&realm, key, |m| {
        let mut seen = Vec::new();
        for (boss, field, door) in ZIGGURATS {
            let g: Guid = m
                .creatures
                .iter()
                .map(|(_, x)| x)
                .find(|c| c.entry == boss)
                .map(|c| c.guid)
                .unwrap();
            instance_script::creature_event(m, g, Event::Death);
            seen.push((get_data(m, field), open(m, &[door])[0]));
        }
        seen
    })
    .await;
    assert_eq!(seen, vec![(DONE, true); 3]);
}

fn of_spawn(m: &Map, spawn: u32) -> Guid {
    m.creatures
        .iter()
        .map(|(_, c)| c)
        .find(|c| c.spawn_guid == Some(spawn))
        .map(|c| c.guid)
        .expect("spawned")
}

/// Thistle Tea's live side: Timmy comes once, for the fall of his own
/// guardsman, not another's; Willey shuts his courtyard's gate when fought
/// and opens it when he goes home or dies.
#[tokio::test]
async fn timmy_comes_once_and_willey_shuts_his_gate() {
    let (realm, _a, key) = inside().await;
    let seen = on(&realm, key, |m| {
        let other = of_spawn(m, 54_069);
        let his = of_spawn(m, 54_070);
        instance_script::creature_event(m, other, Event::Death);
        let before = count(m, TIMMY);
        instance_script::creature_event(m, his, Event::Death);
        instance_script::creature_event(m, his, Event::Death);
        let timmy = (before, count(m, TIMMY));
        let willey = of_spawn(m, 300);
        let mut gate = Vec::new();
        for event in [Event::Aggro, Event::Evade, Event::Aggro, Event::Death] {
            instance_script::creature_event(m, willey, event);
            gate.push(open(m, &[WILLEYS_GATE])[0]);
        }
        (timmy, gate)
    })
    .await;
    assert_eq!(seen, ((0, 1), vec![false, true, false, true]));
}

/// Thistle Tea's crystals: a crystal topples when its own five acolytes
/// are dead, not for another crystal's acolyte, and not before the fifth.
#[tokio::test]
async fn a_crystal_topples_with_its_own_five_acolytes() {
    let (realm, _a, key) = inside().await;
    let seen = on(&realm, key, |m| {
        let crystal = of_spawn(m, 53_955);
        let alive = |m: &Map| m.creature(crystal).is_some_and(|c| c.alive());
        let mut seen = Vec::new();
        for spawn in [53_257, 53_268, 53_269, 53_270, 53_271] {
            let g = of_spawn(m, spawn);
            instance_script::creature_event(m, g, Event::Death);
        }
        seen.push(alive(m));
        let last = of_spawn(m, 53_272);
        instance_script::creature_event(m, last, Event::Death);
        seen.push(alive(m));
        seen
    })
    .await;
    assert_eq!(seen, vec![true, false]);
}
