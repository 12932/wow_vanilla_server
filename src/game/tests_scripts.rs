//! Scripting through the protocol: EventAI, DB scripts, instance scripts,
//! saved copies and raid lockouts.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::npc::BroadcastText;
use crate::game::data::scripts::*;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;

/// Northshire, where a human starts.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

const BRUTE: u32 = 2000;
const IMP: u32 = 2001;
const BOLT: u32 = 5000;

fn template(entry: u32, name: &str) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        display_scales: [1.0, 0.0, 0.0, 0.0],
        level_min: 10,
        level_max: 10,
        faction: 14,
        unit_class: 1,
        health_max: 1000,
        ..Default::default()
    }
}

fn step(id: u32, command: u32) -> ScriptStep {
    ScriptStep {
        id,
        command,
        ..Default::default()
    }
}

/// A brute with EventAI: it yells at aggro, casts a bolt every second in
/// combat, and summons an imp at half health.
fn eventai_data() -> GameData {
    let mut d = GameData::empty();
    d.creature_templates
        .insert(BRUTE, template(BRUTE, "Scripted Brute"));
    d.creature_templates
        .insert(IMP, template(IMP, "Summoned Imp"));
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: 1,
            entries: vec![BRUTE],
            map: 0,
            x: START.0 + 3.0,
            y: START.1,
            z: START.2,
            ..Default::default()
        }],
    );
    d.npc.broadcast_texts.insert(
        100,
        BroadcastText {
            male: "You dare face me, $N?".into(),
            chat_type: 1,
            ..Default::default()
        },
    );
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 2,
        base_points: 9,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(
        BOLT,
        SpellTemplate {
            id: BOLT,
            school: 2,
            effects,
            name: "Test Bolt".into(),
            ..Default::default()
        },
    );
    let s = &mut d.scripts;
    s.ai_scripts.insert(
        1,
        vec![ScriptStep {
            dataint: [100, 0, 0, 0],
            ..step(1, 0)
        }],
    );
    s.ai_scripts.insert(
        2,
        vec![ScriptStep {
            datalong: [BOLT, 0x2, 0, 0],
            target_type: 1,
            ..step(2, 15)
        }],
    );
    s.ai_scripts.insert(
        3,
        vec![ScriptStep {
            datalong: [IMP, 60_000, 0, 0],
            dataint: [0, 0, -1, 1],
            ..step(3, 10)
        }],
    );
    let event = |id, kind, params, flags, action| AiEvent {
        id,
        creature: BRUTE,
        kind,
        chance: 100,
        flags,
        params,
        actions: [action, 0, 0],
        ..Default::default()
    };
    s.ai_events.insert(
        BRUTE,
        vec![
            event(1, EVENT_AGGRO, [0; 4], 0, 1),
            event(
                2,
                EVENT_TIMER_IN_COMBAT,
                [100, 100, 1000, 1000],
                EF_REPEATABLE,
                2,
            ),
            event(3, EVENT_HP, [50, 0, 0, 0], 0, 3),
        ],
    );
    d
}

#[tokio::test]
async fn eventai_yells_at_aggro_casts_on_a_timer_and_summons_at_half_health() {
    let realm = harness::realm_with(eventai_data());
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    let brute = guid::unit(BRUTE, 1);
    a.sees(brute).await;
    a.send(CMSG_ATTACKSWING { guid: brute }).await;
    let yell = a
        .wait_for("the aggro yell", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => match &c.chat_type {
                SMSG_MESSAGECHAT_ChatType::MonsterYell { sender1, .. } if *sender1 == brute => {
                    Some(c.message.clone())
                }
                _ => None,
            },
            _ => None,
        })
        .await;
    assert_eq!(yell, "You dare face me, Alice?");
    a.wait_for("the bolt on its timer", |m| match m {
        ServerOpcodeMessage::SMSG_SPELLNONMELEEDAMAGELOG(l) => {
            (l.attacker == brute && l.spell == BOLT && l.target == me).then_some(())
        }
        _ => None,
    })
    .await;
    // Down to 40%: the imp.
    let map = realm.registry.online(me).unwrap().map();
    map.call(move |m| {
        if let Some(c) = m.creature_mut(brute) {
            c.fields.set_u32(UNIT_FIELD_HEALTH, 400);
        }
    });
    let mut imps = 0;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        imps = map
            .query(|m| m.creatures.iter().filter(|(_, c)| c.entry == IMP).count())
            .await
            .unwrap_or(0);
        if imps > 0 {
            break;
        }
    }
    assert_eq!(imps, 1, "one imp at half health, and only one");
    tokio::time::sleep(Duration::from_millis(700)).await;
    let imps = map
        .query(|m| m.creatures.iter().filter(|(_, c)| c.entry == IMP).count())
        .await
        .unwrap();
    assert_eq!(imps, 1, "the health event doesn't repeat");
}

/// A quest whose completion has its giver say a line and give an item.
fn quest_data() -> GameData {
    let mut d = GameData::empty();
    let mut giver = template(2100, "Quest Giver");
    giver.faction = 35;
    giver.npc_flags = 0x2;
    d.creature_templates.insert(2100, giver);
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: 1,
            entries: vec![2100],
            map: 0,
            x: START.0 + 2.0,
            y: START.1,
            z: START.2,
            ..Default::default()
        }],
    );
    d.npc.quests.insert(
        300,
        crate::game::data::npc::Quest {
            id: 300,
            title: "A Scripted End".into(),
            ..Default::default()
        },
    );
    d.npc.creature_ends.insert(2100, vec![300]);
    d.npc.broadcast_texts.insert(
        200,
        BroadcastText {
            male: "Well done, $N.".into(),
            ..Default::default()
        },
    );
    d.scripts.quest_complete_script.insert(300, 500);
    d.scripts.quest_end.insert(
        500,
        vec![
            ScriptStep {
                dataint: [200, 0, 0, 0],
                ..step(500, 0)
            },
            ScriptStep {
                delay_ms: 200,
                datalong: [1, 0, 0, 0],
                ..step(500, 28)
            },
        ],
    );
    d
}

#[tokio::test]
async fn a_db_script_runs_when_a_quest_is_completed() {
    let realm = harness::realm_with(quest_data());
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    let giver = guid::unit(2100, 1);
    a.sees(giver).await;
    let map = realm.registry.online(me).unwrap().map();
    map.call(move |m| {
        if let Some(p) = m.players.get_mut(&me) {
            p.journal.log.push(crate::game::store::Logged {
                quest: 300,
                ..Default::default()
            });
        }
        crate::game::map::quests::choose_reward(m, me, giver, 300, 0);
    });
    let line = a
        .wait_for("the giver's line", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => match &c.chat_type {
                SMSG_MESSAGECHAT_ChatType::MonsterSay { sender1, .. } if *sender1 == giver => {
                    Some(c.message.clone())
                }
                _ => None,
            },
            _ => None,
        })
        .await;
    assert_eq!(line, "Well done, Alice.");
    // The second step waits its delay, then sits the giver down.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let sitting = map
        .query(move |m| {
            m.creature(giver)
                .map(|c| c.fields.get_byte(UNIT_FIELD_BYTES_1, 0))
        })
        .await
        .flatten();
    assert_eq!(sitting, Some(1));
}

/// The Deadmines with Rhahk'Zor and his door, and a raid with a boss
/// that locks it.
fn instance_data() -> GameData {
    let mut d = GameData::empty();
    for (id, name, kind) in [(36, "DeadminesInstance", 1), (409, "MoltenCore", 2)] {
        d.maps.insert(
            id,
            MapInfo {
                id,
                internal_name: name.into(),
                name: name.into(),
                instance_type: kind,
                player_limit: 0,
            },
        );
    }
    d.scripts.map_templates.insert(
        36,
        MapTemplate {
            map: 36,
            map_type: 1,
            script_name: "instance_deadmines".into(),
            ..Default::default()
        },
    );
    let mut boss = template(644, "Rhahk'Zor");
    boss.rank = 3;
    d.creature_templates.insert(644, boss);
    let mut lord = template(12118, "Lucifron");
    lord.rank = 3;
    lord.static_flags2 = crate::game::instances::STATIC_FLAG2_LOCKS_RAID;
    d.creature_templates.insert(12118, lord);
    d.creature_spawns.insert(
        36,
        vec![CreatureSpawn {
            guid: 79000,
            entries: vec![644],
            map: 36,
            x: 3.0,
            ..Default::default()
        }],
    );
    d.creature_spawns.insert(
        409,
        vec![CreatureSpawn {
            guid: 56000,
            entries: vec![12118],
            map: 409,
            x: 3.0,
            ..Default::default()
        }],
    );
    d.gameobject_templates.insert(
        13965,
        GameObjectTemplate {
            entry: 13965,
            kind: 0,
            display_id: 411,
            name: "Factory Door".into(),
            size: 1.0,
            ..Default::default()
        },
    );
    d.gameobject_spawns.insert(
        36,
        vec![GameObjectSpawn {
            guid: 16000,
            entry: 13965,
            map: 36,
            x: 10.0,
            rotation: [0.0, 0.0, 0.0, 1.0],
            state: 1,
            ..Default::default()
        }],
    );
    d
}

/// Send `c` into `map`'s copy and wait for it to land.
async fn go(realm: &Arc<Realm>, c: &mut Client, map: u32) {
    let g = c.guid;
    let from = realm.registry.online(g).unwrap().map();
    from.call(move |m| {
        m.teleport(
            g,
            MapKey::world(map),
            Vector3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
        )
    });
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
}

fn key_of(realm: &Realm, g: Guid) -> MapKey {
    realm.registry.online(g).unwrap().map().key
}

/// Kill the first creature of `entry` where `c` is, by `c`'s hand.
async fn kill(realm: &Arc<Realm>, g: Guid, entry: u32) {
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        let c = m
            .creatures
            .iter()
            .find(|(_, c)| c.entry == entry && c.alive())
            .map(|(_, c)| c.guid);
        if let Some(c) = c {
            crate::game::map::combat::deal_damage(m, g, c, 1_000_000);
        }
    })
    .await;
}

async fn alive_of(realm: &Arc<Realm>, g: Guid, entry: u32) -> Option<bool> {
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        m.creatures
            .iter()
            .find(|(_, c)| c.entry == entry)
            .map(|(_, c)| c.alive())
    })
    .await
    .flatten()
}

async fn door_state(realm: &Arc<Realm>, g: Guid) -> Option<u32> {
    let map = realm.registry.online(g).unwrap().map();
    map.query(|m| {
        m.gameobjects
            .iter()
            .find(|(_, o)| o.entry == 13965)
            .map(|(_, o)| o.fields.get_u32(GAMEOBJECT_STATE))
    })
    .await
    .flatten()
}

#[tokio::test]
async fn a_dungeon_boss_stays_dead_and_its_door_open_across_a_restart() {
    let dir = std::env::temp_dir().join(format!(
        "wvs-restart-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("characters.sqlite");
    let open = |path: &std::path::Path| {
        let (store, startup) =
            crate::game::store::Store::open(path.to_str().unwrap()).expect("a store");
        Realm::new(
            harness::test_config(),
            store,
            startup,
            instance_data(),
            None,
        )
    };
    let realm = open(&path);
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    go(&realm, &mut a, 36).await;
    let first = key_of(&realm, me);
    assert_eq!(
        door_state(&realm, me).await,
        Some(1),
        "the door starts shut"
    );
    kill(&realm, me, 644).await;
    assert_eq!(alive_of(&realm, me, 644).await, Some(false));
    assert_eq!(
        door_state(&realm, me).await,
        Some(0),
        "Rhahk'Zor's death opens it"
    );
    // Saved and written, the old server goes away with Alice in place.
    let map = realm.registry.online(me).unwrap().map();
    map.query(|m| m.save_player(m.players.keys().next().copied().unwrap()))
        .await;
    realm.store.flush().await;
    // A new server on the same file: Alice comes back to the same copy.
    let realm2 = open(&path);
    let mut a2 = Client::connect(&realm2, "A");
    a2.login(me).await;
    a2.sees(me).await;
    let again = key_of(&realm2, me);
    assert_eq!(again, first, "the same copy, after the restart");
    assert_eq!(alive_of(&realm2, me, 644).await, Some(false), "still dead");
    assert_eq!(door_state(&realm2, me).await, Some(0), "still open");
}

#[tokio::test]
async fn a_reset_brings_a_dungeon_back_as_new() {
    let mut cfg = harness::test_config();
    cfg.instance_unload = Duration::from_secs(600);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = Realm::new(cfg, store, startup, instance_data(), None);
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    go(&realm, &mut a, 36).await;
    let first = key_of(&realm, me);
    kill(&realm, me, 644).await;
    go(&realm, &mut a, 0).await;
    // Back in before a reset: the same copy, its boss dead.
    go(&realm, &mut a, 36).await;
    assert_eq!(key_of(&realm, me), first);
    assert_eq!(alive_of(&realm, me, 644).await, Some(false));
    go(&realm, &mut a, 0).await;
    a.send(CMSG_RESET_INSTANCES {}).await;
    a.wait_for("SMSG_INSTANCE_RESET", |m| match m {
        ServerOpcodeMessage::SMSG_INSTANCE_RESET(r) => (r.map.as_int() == 36).then_some(()),
        _ => None,
    })
    .await;
    go(&realm, &mut a, 36).await;
    assert_ne!(key_of(&realm, me), first, "a new copy");
    assert_eq!(alive_of(&realm, me, 644).await, Some(true), "with its boss");
    assert_eq!(door_state(&realm, me).await, Some(1));
}

#[tokio::test]
async fn a_raid_boss_kill_binds_the_raid_and_the_lockout_holds() {
    let realm = harness::realm_with(instance_data());
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    go(&realm, &mut a, 409).await;
    let first = key_of(&realm, me);
    // Not bound by going in.
    a.send(CMSG_REQUEST_RAID_INFO {}).await;
    let none = a
        .wait_for("SMSG_RAID_INSTANCE_INFO", |m| match m {
            ServerOpcodeMessage::SMSG_RAID_INSTANCE_INFO(r) => Some(r.raid_infos.len()),
            _ => None,
        })
        .await;
    assert_eq!(none, 0);
    kill(&realm, me, 12118).await;
    a.wait_for("SMSG_INSTANCE_SAVE_CREATED", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_INSTANCE_SAVE_CREATED(_)).then_some(())
    })
    .await;
    a.send(CMSG_REQUEST_RAID_INFO {}).await;
    let bound = a
        .wait_for("SMSG_RAID_INSTANCE_INFO", |m| match m {
            ServerOpcodeMessage::SMSG_RAID_INSTANCE_INFO(r) if !r.raid_infos.is_empty() => Some(
                r.raid_infos
                    .iter()
                    .map(|i| i.instance_id)
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .await;
    assert_eq!(bound, vec![first.instance]);
    // Out, the copy empties and unloads; the lockout brings her back to it,
    // the boss still dead.
    go(&realm, &mut a, 0).await;
    let mut gone = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        if !realm.maps.all().iter().any(|h| h.key == first) {
            gone = true;
            break;
        }
    }
    assert!(gone, "the empty copy unloads");
    go(&realm, &mut a, 409).await;
    assert_eq!(key_of(&realm, me), first, "the lockout holds");
    assert_eq!(alive_of(&realm, me, 12118).await, Some(false));
}

#[tokio::test]
async fn a_pool_spawns_its_limit_and_a_condition_gates_a_step() {
    let mut d = GameData::empty();
    d.creature_templates
        .insert(BRUTE, template(BRUTE, "Pooled"));
    d.creature_spawns.insert(
        0,
        (1..=3)
            .map(|g| CreatureSpawn {
                guid: g,
                entries: vec![BRUTE],
                map: 0,
                x: START.0 + g as f32,
                y: START.1,
                z: START.2,
                ..Default::default()
            })
            .collect(),
    );
    d.scripts.pools.insert(
        7,
        Pool {
            entry: 7,
            max_limit: 1,
            creatures: vec![(1, 0.0), (2, 0.0), (3, 0.0)],
            ..Default::default()
        },
    );
    for g in 1..=3 {
        d.scripts.creature_pool.insert(g, 7);
    }
    // Level at least 5, and its reverse.
    d.scripts.conditions.insert(
        1,
        Condition {
            entry: 1,
            kind: 15,
            values: [5, 1, 0, 0],
            flags: 0,
        },
    );
    d.scripts.conditions.insert(
        2,
        Condition {
            entry: 2,
            kind: 15,
            values: [5, 1, 0, 0],
            flags: CF_REVERSE_RESULT,
        },
    );
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(me).unwrap().map();
    let (pooled, low, high) = map
        .query(move |m| {
            let n = m.creatures.iter().filter(|(_, c)| c.entry == BRUTE).count();
            let low = crate::game::map::conditions::met(m, 1, None, Some(me));
            let high = crate::game::map::conditions::met(m, 2, None, Some(me));
            (n, low, high)
        })
        .await
        .unwrap();
    assert_eq!(pooled, 1, "one of the pool's three");
    assert!(!low, "a level 1 isn't level 5");
    assert!(high, "reversed");
}

/// The real data: every EventAI event type in the world database is one
/// the engine knows, and the boss scripts are in place.
#[test]
fn the_real_eventai_uses_known_event_types() {
    let dir = &crate::game::harness::real_data("db");
    if !dir.join("vmangos.sqlite").exists() {
        return;
    }
    let data = crate::game::data::load::load(dir);
    let events: Vec<&AiEvent> = data.scripts.ai_events.values().flatten().collect();
    assert!(events.len() > 6_000, "{}", events.len());
    let known = events.iter().filter(|e| event_supported(e.kind)).count();
    assert!(
        known * 1000 >= events.len() * 995,
        "{known} of {} events have a known type",
        events.len()
    );
    // Every action names a script that exists.
    let missing = events
        .iter()
        .flat_map(|e| e.actions)
        .filter(|a| *a != 0 && !data.scripts.ai_scripts.contains_key(a))
        .count();
    assert!(missing < 50, "{missing} actions name no script");
    assert!(
        data.scripts.ai_events.contains_key(&12118),
        "Lucifron's script"
    );
    assert!(
        data.scripts
            .ai_events
            .get(&9196)
            .is_some_and(|e| e.len() >= 6),
        "Highlord Omokk's timed casts, from Thistle Tea's script"
    );
    assert!(data.scripts.conditions.len() > 1_000);
    assert!(data.scripts.pools.len() > 1_000);
    assert_eq!(
        data.scripts
            .map_templates
            .get(&409)
            .map(|t| t.script_name.as_str()),
        Some("instance_molten_core")
    );
}

/// The real Deadmines: Rhahk'Zor's death opens his door, and the copy's
/// EventAI creatures fight a player without trouble.
#[tokio::test]
async fn the_real_deadmines_opens_rhahkzors_door() {
    let dir = &crate::game::harness::real_data("db");
    if !dir.join("vmangos.sqlite").exists() {
        return;
    }
    let data = crate::game::data::load::load(dir);
    let boss = data
        .creature_spawns
        .get(&36)
        .and_then(|v| v.iter().find(|s| s.entries.contains(&644)))
        .cloned()
        .expect("Rhahk'Zor's spawn");
    let realm = harness::realm_with(data);
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    let from = realm.registry.online(me).unwrap().map();
    // Northshire first: its EventAI creatures, a fight with them, and their
    // spawned events, without a panic.
    from.call(move |m| {
        let at = m.players[&me].position();
        let near: Vec<Guid> = m
            .creatures
            .iter()
            .filter(|(_, c)| {
                c.alive()
                    && crate::game::map::event_ai::has_events_entry(&m.realm.data, c.entry)
                    && crate::game::map::combat::dist(c.position, at) < 80.0
            })
            .map(|(_, c)| c.guid)
            .take(8)
            .collect();
        for c in near {
            crate::game::map::ai::attacked(m, c, me);
        }
    });
    tokio::time::sleep(Duration::from_millis(2000)).await;
    let panics = from.query(|m| m.stats.panics).await.unwrap();
    assert_eq!(panics, 0, "nothing in Northshire panicked");
    from.call(move |m| {
        m.teleport(
            me,
            MapKey::world(36),
            Vector3d {
                x: boss.x + 4.0,
                y: boss.y,
                z: boss.z,
            },
            0.0,
        )
    });
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(me).await;
    let map = realm.registry.online(me).unwrap().map();
    let (scripted, door) = map
        .query(|m| {
            let door = m
                .gameobjects
                .iter()
                .find(|(_, o)| o.entry == 13965)
                .map(|(_, o)| o.fields.get_u32(GAMEOBJECT_STATE));
            (m.instance.script, door)
        })
        .await
        .unwrap();
    assert_eq!(
        scripted,
        Some(crate::game::map::instance_script::Script::Deadmines)
    );
    assert_eq!(door, Some(1), "the factory door starts shut");
    // A fight with whatever is near, for a few updates.
    map.call(move |m| {
        let near: Vec<Guid> = m
            .creatures
            .iter()
            .filter(|(_, c)| {
                c.alive()
                    && crate::game::map::combat::dist(c.position, m.players[&me].position()) < 30.0
            })
            .map(|(_, c)| c.guid)
            .take(5)
            .collect();
        for c in near {
            crate::game::map::ai::attacked(m, c, me);
        }
    });
    tokio::time::sleep(Duration::from_millis(1500)).await;
    kill(&realm, me, 644).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let panics = map.query(|m| m.stats.panics).await.unwrap();
    assert_eq!(panics, 0, "nothing in the copy panicked");
    let door = map
        .query(|m| {
            m.gameobjects
                .iter()
                .find(|(_, o)| o.entry == 13965)
                .map(|(_, o)| o.fields.get_u32(GAMEOBJECT_STATE))
        })
        .await
        .flatten();
    assert_eq!(door, Some(0), "Rhahk'Zor's death opens it");
    let saved = crate::game::instances::save_of(&realm, key_of(&realm, me).instance).unwrap();
    assert_eq!(saved.data.get(&10), Some(&3));
    // He comes back on his spawn's timer, not before.
    let back = saved.dead.get(&boss.guid).copied().expect("saved dead");
    assert!(back > crate::game::social::mail::now() + 60, "{back}");
}

#[tokio::test]
async fn a_map_event_ends_in_success_when_its_condition_is_met() {
    let mut d = eventai_data();
    d.scripts.ai_events.clear();
    // Success when counter 0 of event 1 reaches 2.
    d.scripts.conditions.insert(
        10,
        Condition {
            entry: 10,
            kind: 35,
            values: [1, 0, 2, 1],
            flags: 0,
        },
    );
    d.npc.broadcast_texts.insert(
        300,
        BroadcastText {
            male: "The event is won.".into(),
            ..Default::default()
        },
    );
    d.scripts.generic.insert(
        600,
        vec![ScriptStep {
            dataint: [300, 0, 0, 0],
            ..step(600, 0)
        }],
    );
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    let brute = guid::unit(BRUTE, 1);
    a.sees(brute).await;
    let map = realm.registry.online(me).unwrap().map();
    map.call(move |m| {
        let start = ScriptStep {
            datalong: [1, 60, 0, 0],
            dataint: [10, 600, 0, 0],
            ..step(0, 61)
        };
        let add = ScriptStep {
            datalong: [1, 0, 1, 1],
            ..step(0, 65)
        };
        crate::game::map::scripts::execute(m, &start, brute, me);
        crate::game::map::scripts::execute(m, &add, brute, me);
        assert!(crate::game::map::map_events::active(m, 1));
        crate::game::map::scripts::execute(m, &add, brute, me);
    });
    let line = a
        .wait_for("the success script", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => Some(c.message.clone()),
            _ => None,
        })
        .await;
    assert_eq!(line, "The event is won.");
    let over = map
        .query(|m| !crate::game::map::map_events::active(m, 1))
        .await
        .unwrap();
    assert!(over);
}
