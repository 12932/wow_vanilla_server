//! Copies and escorts as Thistle Tea runs them: a raid reset with players
//! inside, raid groups held by their leader's saves, a group taking over
//! its leader's copy, logging back into a copy, copy limits, doors worked
//! only when their rule flips, scripted doors players may not use, initial
//! instance data, timers by key, Stoned statues, and escorts by their point
//! ids; then the same on the real world database.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::npc::Quest;
use crate::game::data::scripts::MapTemplate;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::world::ScriptPoint;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::instances;
use crate::game::map::instance_script::{self, DONE, IN_PROGRESS};
use crate::game::map::{MapHandle, MapKey};

const RAID: u32 = 409;
const DUNGEON: u32 = 36;
const LORD: u32 = 12118;
const NOT_SELECTABLE: u32 = 0x0200_0000;

fn template(entry: u32, name: &str) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 10,
        level_max: 10,
        faction: 35,
        unit_class: 1,
        health_max: 500,
        speed_walk: 1.0,
        speed_run: 1.0,
        ..Default::default()
    }
}

fn map_info(d: &mut GameData, id: u32, kind: u32, limit: u32, script: &str) {
    d.maps.insert(
        id,
        MapInfo {
            id,
            internal_name: format!("Map{id}"),
            name: format!("Map {id}"),
            instance_type: kind,
            player_limit: 0,
        },
    );
    d.scripts.map_templates.insert(
        id,
        MapTemplate {
            map: id,
            map_type: kind,
            player_limit: limit,
            script_name: script.into(),
            ..Default::default()
        },
    );
}

fn door(entry: u32) -> GameObjectTemplate {
    GameObjectTemplate {
        entry,
        kind: 0,
        display_id: 411,
        name: format!("Door {entry}"),
        size: 1.0,
        ..Default::default()
    }
}

fn object_spawn(guid: u32, entry: u32, map: u32, state: u8) -> GameObjectSpawn {
    GameObjectSpawn {
        guid,
        entry,
        map,
        x: 2.0,
        rotation: [0.0, 0.0, 0.0, 1.0],
        state,
        ..Default::default()
    }
}

/// A raid with a boss that binds it, and a dungeon that holds one.
fn copies() -> GameData {
    let mut d = GameData::empty();
    map_info(&mut d, RAID, 2, 40, "");
    map_info(&mut d, DUNGEON, 1, 1, "");
    let mut lord = template(LORD, "Lucifron");
    lord.rank = 3;
    lord.static_flags2 = instances::STATIC_FLAG2_LOCKS_RAID;
    d.creature_templates.insert(LORD, lord);
    d.creature_spawns.insert(
        RAID,
        vec![CreatureSpawn {
            guid: 56_000,
            entries: vec![LORD],
            map: RAID,
            x: 3.0,
            ..Default::default()
        }],
    );
    d
}

fn non_gm(data: GameData) -> Arc<Realm> {
    lasting(data, Duration::from_millis(200))
}

/// Players who are no GMs, and empty copies that last `unload`.
fn lasting(data: GameData, unload: Duration) -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    cfg.instance_unload = unload;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, data, None)
}

async fn player(realm: &Arc<Realm>, name: &str) -> Client {
    let mut c = Client::connect(realm, name);
    c.enter(name, Race::Human, Class::Warrior).await;
    c
}

/// Send `c` into `map`'s copy (or the open world), and wait for it to land.
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

/// `leader` and `member` in a raid group.
fn raid_group(realm: &Arc<Realm>, leader: Guid, members: &[Guid]) {
    let l = realm.registry.online(leader).unwrap();
    for m in members {
        let m = realm.registry.online(*m).unwrap();
        assert!(crate::game::social::groups::join_directly(realm, &l, &m));
    }
    crate::game::social::groups::convert_to_raid(realm, &l);
}

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

fn raid_infos(m: &ServerOpcodeMessage) -> Option<Vec<u32>> {
    match m {
        ServerOpcodeMessage::SMSG_RAID_INSTANCE_INFO(r) => {
            Some(r.raid_infos.iter().map(|i| i.instance_id).collect())
        }
        _ => None,
    }
}

async fn running(realm: &Realm, key: MapKey) -> bool {
    realm.maps.all().iter().any(|h| h.key == key)
}

/// A raid copy whose reset comes with players inside: they stay, with the
/// minute's grace and told their binds are gone, no one more goes in, and
/// it goes as soon as they have left. Saved, they heard the new save, then
/// what they are saved to.
#[tokio::test]
async fn a_raid_reset_with_players_inside_waits_for_them() {
    let realm = non_gm(copies());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    raid_group(&realm, a.guid, &[b.guid]);
    go(&realm, &mut a, RAID).await;
    go(&realm, &mut b, RAID).await;
    let copy = key_of(&realm, a.guid);
    assert_eq!(key_of(&realm, b.guid), copy);
    kill(&realm, a.guid, LORD).await;
    a.wait_for("SMSG_INSTANCE_SAVE_CREATED", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_INSTANCE_SAVE_CREATED(_)).then_some(())
    })
    .await;
    let saved = a.wait_for("SMSG_RAID_INSTANCE_INFO", raid_infos).await;
    assert_eq!(saved, vec![copy.instance], "told what she is saved to");
    // The reset day comes.
    {
        let mut inst = realm.social.instances.lock();
        inst.raids.get_mut(&copy.instance).unwrap().reset_at = 0;
    }
    instances::sweep(&realm);
    let none = a
        .wait_for("the binds gone", |m| raid_infos(m).filter(|v| v.is_empty()))
        .await;
    assert!(none.is_empty());
    assert!(running(&realm, copy).await, "not stopped under them");
    assert!(instances::expired(&realm, copy));
    let grace = b
        .wait_for("SMSG_RAID_GROUP_ONLY", |m| match m {
            ServerOpcodeMessage::SMSG_RAID_GROUP_ONLY(r) if r.homebind_timer > 0 => {
                Some(r.homebind_timer)
            }
            _ => None,
        })
        .await;
    assert_eq!(grace, 60_000, "a minute's grace");
    assert!(realm.registry.online(a.guid).is_some() && realm.registry.online(b.guid).is_some());
    // Out and back in: a new copy, not the old one.
    go(&realm, &mut a, 0).await;
    go(&realm, &mut a, RAID).await;
    assert_ne!(key_of(&realm, a.guid), copy, "the old copy takes no one");
    // Bob, the last inside, leaves: it goes at once.
    go(&realm, &mut b, 0).await;
    let mut gone = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        if !running(&realm, copy).await {
            gone = true;
            break;
        }
    }
    assert!(gone, "the old copy goes once empty");
    assert!(instances::save_of(&realm, copy.instance).is_none());
}

/// A raid group follows its leader's save: Carol, new to Alice's raid, goes
/// into the copy Alice is bound to and is bound with it, and is told; the
/// group's members belong there; under a new leader with no save they go
/// on with the group's copy.
#[tokio::test]
async fn a_raid_group_follows_its_leaders_save() {
    let realm = non_gm(copies());
    let mut a = player(&realm, "Alice").await;
    let b = player(&realm, "Bob").await;
    raid_group(&realm, a.guid, &[b.guid]);
    go(&realm, &mut a, RAID).await;
    let copy = key_of(&realm, a.guid);
    kill(&realm, a.guid, LORD).await;
    a.wait_for("SMSG_INSTANCE_SAVE_CREATED", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_INSTANCE_SAVE_CREATED(_)).then_some(())
    })
    .await;
    go(&realm, &mut a, 0).await;
    // The raid breaks up; Alice leads another, with Carol.
    let ao = realm.registry.online(a.guid).unwrap();
    crate::game::social::groups::leave(&realm, &ao);
    let mut c = player(&realm, "Carol").await;
    raid_group(&realm, a.guid, &[c.guid]);
    assert!(
        !instances::belongs(&realm, b.guid, copy),
        "Bob, not in it, no"
    );
    go(&realm, &mut c, RAID).await;
    assert_eq!(key_of(&realm, c.guid), copy, "the leader's save");
    c.wait_for("SMSG_INSTANCE_SAVE_CREATED", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_INSTANCE_SAVE_CREATED(_)).then_some(())
    })
    .await;
    let bound = c.wait_for("SMSG_RAID_INSTANCE_INFO", raid_infos).await;
    assert_eq!(bound, vec![copy.instance], "bound on the way in");
    assert!(instances::belongs(&realm, c.guid, copy));
    assert_eq!(
        realm
            .social
            .instances
            .lock()
            .raid_bind(guid::counter(c.guid), RAID),
        Some(copy.instance)
    );
}

/// A group takes over the copy its leader last went into alone, and one
/// left without an owner by a group gone; a copy saved with a group's
/// owner comes back without one, since group ids come again.
#[tokio::test]
async fn a_group_takes_its_leaders_copy_and_an_orphaned_one() {
    let mut d = copies();
    map_info(&mut d, DUNGEON, 1, 10, "");
    let realm = lasting(d, Duration::from_secs(600));
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    go(&realm, &mut a, DUNGEON).await;
    let solo = key_of(&realm, a.guid);
    go(&realm, &mut a, 0).await;
    let (ao, bo) = (
        realm.registry.online(a.guid).unwrap(),
        realm.registry.online(b.guid).unwrap(),
    );
    assert!(crate::game::social::groups::join_directly(&realm, &ao, &bo));
    go(&realm, &mut b, DUNGEON).await;
    assert_eq!(key_of(&realm, b.guid), solo, "Alice's copy is the group's");
    go(&realm, &mut b, 0).await;
    // The group goes; Alice's next group takes the copy left behind.
    crate::game::social::groups::leave(&realm, &bo);
    let mut c = player(&realm, "Carol").await;
    let co = realm.registry.online(c.guid).unwrap();
    assert!(crate::game::social::groups::join_directly(&realm, &ao, &co));
    go(&realm, &mut c, DUNGEON).await;
    assert_eq!(key_of(&realm, c.guid), solo, "the orphan, adopted");
    // A restart: the group's copy is saved, its owner is not.
    let save = instances::save_of(&realm, solo.instance).unwrap();
    assert!(matches!(save.owner, Some(instances::Owner::Group(_))));
    let record = serde_json::to_string(&save).unwrap();
    let loaded = instances::Instances::load(&[(instances::KIND_INSTANCE.to_owned(), 1, record)]);
    assert_eq!(loaded.saves[&solo.instance].owner, None);
}

/// Logged out inside a copy, a player comes back into it while it is
/// theirs; once it is gone, home.
#[tokio::test]
async fn logging_in_inside_a_copy_resumes_it_or_goes_home() {
    let mut d = copies();
    map_info(&mut d, DUNGEON, 1, 10, "");
    let mut cfg = harness::test_config();
    cfg.instance_unload = Duration::from_secs(600);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = Realm::new(cfg, store, startup, d, None);
    let mut a = player(&realm, "Alice").await;
    let me = a.guid;
    go(&realm, &mut a, DUNGEON).await;
    let copy = key_of(&realm, me);
    let relog = |realm: Arc<Realm>, a: Client| async move {
        a.disconnect();
        for _ in 0..200 {
            if realm.registry.online(me).is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        realm.store.flush().await;
        let mut a = Client::connect(&realm, "Alice");
        a.login(me).await;
        a.sees(me).await;
        a
    };
    let a = relog(realm.clone(), a).await;
    assert_eq!(key_of(&realm, me), copy, "back in the same copy");
    // The copy goes (reset while she is away): home, not a fresh copy.
    let a = {
        a.disconnect();
        for _ in 0..200 {
            if realm.registry.online(me).is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        {
            let mut inst = realm.social.instances.lock();
            inst.saves.remove(&copy.instance);
        }
        realm.store.flush().await;
        let mut a = Client::connect(&realm, "Alice");
        a.login(me).await;
        a.sees(me).await;
        a
    };
    let k = key_of(&realm, me);
    assert_eq!(k.instance, 0, "home, in the open world: {k}");
    drop(a);
}

/// A copy holds its template's player limit; 0 is no limit.
#[tokio::test]
async fn a_copy_holds_its_templates_limit() {
    let realm = non_gm(copies());
    assert_eq!(instances::limit(&realm.data, DUNGEON), 1);
    assert_eq!(instances::limit(&realm.data, RAID), 40);
    assert_eq!(instances::limit(&realm.data, 1), usize::MAX, "no template");
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let (ao, bo) = (
        realm.registry.online(a.guid).unwrap(),
        realm.registry.online(b.guid).unwrap(),
    );
    assert!(crate::game::social::groups::join_directly(&realm, &ao, &bo));
    go(&realm, &mut a, DUNGEON).await;
    let g = b.guid;
    realm
        .registry
        .online(g)
        .unwrap()
        .map()
        .call(move |m| m.teleport(g, MapKey::world(DUNGEON), Vector3d::default(), 0.0));
    let reason = b
        .wait_for("SMSG_TRANSFER_ABORTED", |m| match m {
            ServerOpcodeMessage::SMSG_TRANSFER_ABORTED(t) => Some(t.reason),
            _ => None,
        })
        .await;
    assert_eq!(reason, TransferAbortReason::IsFull);
}

/// Scholomance's doors: the room gate keeps its spawned state (open) at
/// load, Kirtonos's gate shuts and opens only as its rule flips, a gate
/// opened by hand stays open through other writes, and players can't work
/// either; the viewing room's door, which a player opens, is no scripted
/// door.
#[tokio::test]
async fn doors_turn_only_when_their_rule_flips() {
    let mut d = GameData::empty();
    map_info(&mut d, 289, 1, 5, "instance_scholomance");
    for e in [175570, 177371, 175167] {
        d.gameobject_templates.insert(e, door(e));
    }
    d.gameobject_spawns.insert(
        289,
        vec![
            object_spawn(80_000, 175570, 289, 0),
            object_spawn(80_001, 177371, 289, 0),
            object_spawn(80_002, 175167, 289, 1),
        ],
    );
    let realm = harness::realm_with(d);
    let mut a = player(&realm, "Alice").await;
    go(&realm, &mut a, 289).await;
    let map = realm.registry.online(a.guid).unwrap().map();
    let state = |map: &MapHandle, entry: u32| {
        let map = map.clone();
        async move {
            map.query(move |m| {
                m.gameobjects
                    .iter()
                    .find(|(_, o)| o.entry == entry)
                    .map(|(_, o)| {
                        (
                            o.fields.get_u32(GAMEOBJECT_STATE),
                            o.fields.get_u32(GAMEOBJECT_FLAGS) & 0x1,
                        )
                    })
            })
            .await
            .flatten()
        }
    };
    assert_eq!(state(&map, 177371).await, Some((0, 0)), "as it spawned");
    map.call(|m| instance_script::set_data(m, 7, IN_PROGRESS, 0));
    assert_eq!(
        state(&map, 175570).await,
        Some((1, 0)),
        "Kirtonos's gate shuts"
    );
    assert_eq!(
        state(&map, 177371).await,
        Some((0, 0)),
        "the room gate is left"
    );
    map.call(|m| instance_script::set_data(m, 7, DONE, 0));
    assert_eq!(state(&map, 175570).await, Some((0, 1)), "open, and in use");
    // Shut by other means, a write that flips nothing leaves it.
    map.query(|m| {
        let g = m
            .gameobjects
            .iter()
            .find(|(_, o)| o.entry == 177371)
            .map(|(_, o)| o.guid)
            .unwrap();
        crate::game::map::scripts::set_object_state(m, g, 1);
        instance_script::set_data(m, 3, DONE, 0);
    })
    .await;
    assert_eq!(state(&map, 177371).await, Some((1, 0)), "not snapped back");
    // A player can't work the scripted gate; the viewing room's door is hers.
    let me = a.guid;
    let (gate, viewing) = map
        .query(move |m| {
            let of = |e| {
                m.gameobjects
                    .iter()
                    .find(|(_, o)| o.entry == e)
                    .map(|(_, o)| o.guid)
                    .unwrap()
            };
            let (gate, viewing) = (of(175570), of(175167));
            (
                instance_script::scripted_door(m, gate),
                instance_script::scripted_door(m, viewing),
            )
        })
        .await
        .unwrap();
    assert!(gate && !viewing);
    map.query(move |m| {
        let g = m
            .gameobjects
            .iter()
            .find(|(_, o)| o.entry == 175570)
            .map(|(_, o)| o.guid)
            .unwrap();
        if let Some(p) = m.players.get_mut(&me) {
            p.movement.position = Vector3d {
                x: 2.0,
                y: 0.5,
                z: 0.0,
            };
        }
        crate::game::map::world::use_object(m, me, g);
    })
    .await;
    assert_eq!(
        state(&map, 175570).await,
        Some((0, 1)),
        "refused: still open"
    );
}

/// Dire Maul's tribute starts with its six guards alive, and counts down
/// from there; a timer scheduled again replaces the one waiting.
#[tokio::test]
async fn instance_data_starts_at_its_initial_value_and_timers_replace() {
    let mut d = GameData::empty();
    map_info(&mut d, 429, 1, 5, "instance_dire_maul");
    let realm = harness::realm_with(d);
    let mut a = player(&realm, "Alice").await;
    go(&realm, &mut a, 429).await;
    let map = realm.registry.online(a.guid).unwrap().map();
    let (start, after, timers) = map
        .query(|m| {
            let start = instance_script::get_data(m, 15);
            instance_script::set_data(m, 15, 1, 2);
            let after = instance_script::get_data(m, 15);
            instance_script::schedule(m, 77, Duration::from_secs(5));
            instance_script::schedule(m, 77, Duration::from_secs(9));
            let timers = m.instance.timers.iter().filter(|(_, k)| *k == 77).count();
            (start, after, timers)
        })
        .await
        .unwrap();
    assert_eq!((start, after), (6, 5));
    assert_eq!(timers, 1, "one chain, not two");
    assert_eq!(instance_script::initial_value(None, 15), 0);
}

/// Stoned leaves a creature unselectable while it lasts, and only then.
#[tokio::test]
async fn a_stoned_creature_cannot_be_selected() {
    let mut d = GameData::empty();
    let mut stoned = SpellTemplate {
        id: 10255,
        name: "Stoned".into(),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    stoned.effects[0] = SpellEffect {
        effect: 6,
        aura: 4,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(10255, stoned);
    d.creature_templates
        .insert(7076, template(7076, "Earthen Guardian"));
    let realm = harness::realm_with(d);
    let a = player(&realm, "Alice").await;
    let map = realm.registry.online(a.guid).unwrap().map();
    let flags = map
        .query(|m| {
            let at = m.players.values().next().unwrap().position();
            let g =
                crate::game::map::scripts::summon(m, Guid::zero(), 7076, at, 0.0, None).unwrap();
            let f = |m: &crate::game::map::Map| {
                m.creature(g)
                    .map_or(0, |c| c.fields.get_u32(UNIT_FIELD_FLAGS) & NOT_SELECTABLE)
            };
            let before = f(m);
            crate::game::map::spells::cast_triggered(m, g, 10255, Some(g));
            let on = f(m);
            crate::game::map::auras::remove_spell(m, g, 10255, None);
            (before, on, f(m))
        })
        .await
        .unwrap();
    assert_eq!(flags, (0, NOT_SELECTABLE, 0));
}

/// Volcor's escort (994): its points are 1 to 15, and it credits at point
/// 15 (its last), not the sixteenth in the list; after a fight it would
/// come back to the last point it reached; at the end it goes and comes
/// back on its spawn's own time. Brontus's (665) respawns at once.
#[tokio::test]
async fn an_escort_goes_by_its_point_ids() {
    const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
    let mut d = GameData::empty();
    for (entry, quest, points) in [(3692, 994, 1..=15), (2768, 665, 0..=20)] {
        let mut t = template(entry, "Escortee");
        t.npc_flags = 0x2;
        d.creature_templates.insert(entry, t);
        d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
            guid: entry,
            entries: vec![entry],
            map: 0,
            x: START.0 - 1.5,
            y: START.1 + if entry == 3692 { 0.0 } else { 40.0 },
            z: START.2,
            respawn_min: 600,
            respawn_max: 600,
            ..Default::default()
        });
        d.npc.quests.insert(
            quest,
            Quest {
                id: quest,
                title: format!("Quest {quest}"),
                min_level: 1,
                level: 1,
                flags: 0x4,
                ..Default::default()
            },
        );
        d.npc.creature_starts.insert(entry, vec![quest]);
        let y = START.1 + if entry == 3692 { 2.0 } else { 42.0 };
        d.world.script_waypoints.insert(
            entry,
            points
                .clone()
                .map(|i| ScriptPoint {
                    point: i,
                    x: START.0 + 2.0 * i as f32,
                    y,
                    z: START.2,
                    wait_ms: 0,
                })
                .collect(),
        );
    }
    let realm = harness::realm_with(d);
    let a = player(&realm, "Alice").await;
    let g = a.guid;
    let map = realm.registry.online(g).unwrap().map();
    let walk = |entry: u32, quest: u32| {
        let map = map.clone();
        async move {
            let escortee = guid::unit(entry, entry);
            map.query(move |m| {
                if let Some(c) = m.creature_mut(escortee) {
                    c.speeds.walk = 200.0;
                    c.speeds.run = 200.0;
                }
                if let Some(at) = m.creature(escortee).map(|c| c.position)
                    && let Some(p) = m.players.get_mut(&g)
                {
                    p.movement.position = at;
                }
                crate::game::map::quests::accept(m, g, escortee, quest);
                for e in m.escorts.iter_mut() {
                    e.skip_start();
                }
            })
            .await;
            // Along the path, the player at its side; the point of the
            // credit, and where it came home to.
            let mut credited_at = None;
            for _ in 0..400 {
                tokio::time::sleep(Duration::from_millis(20)).await;
                let (at, credited, active) = map
                    .query(move |m| {
                        let at = m.creature(escortee).map(|c| (c.position, c.home));
                        if let Some((p, _)) = at
                            && let Some(pl) = m.players.get_mut(&g)
                        {
                            pl.movement.position = p;
                        }
                        let credited = m.players[&g]
                            .journal
                            .log
                            .iter()
                            .any(|l| l.quest == quest && l.explored);
                        (at, credited, !m.escorts.is_empty())
                    })
                    .await
                    .unwrap();
                if credited && credited_at.is_none() {
                    credited_at = at;
                }
                if !active {
                    break;
                }
            }
            let life = map
                .query(move |m| {
                    let now = m.now;
                    m.creature(escortee).map(|c| match c.life {
                        crate::game::map::creature::Life::Dead { respawn_at, .. } => {
                            Some(respawn_at.saturating_duration_since(now))
                        }
                        _ => None,
                    })
                })
                .await
                .flatten()
                .flatten();
            (credited_at, life)
        }
    };
    let (credited_at, back_in) = walk(3692, 994).await;
    let (pos, home) = credited_at.expect("credited");
    let last_x = START.0 + 30.0;
    assert!(
        (pos.x - last_x).abs() < 2.5,
        "at point 15, its last: {pos:?}"
    );
    assert!(
        (home.x - pos.x).abs() < 2.5,
        "home is the point reached: {home:?}"
    );
    let back_in = back_in.expect("gone at the end, not back at once");
    assert!(
        back_in > Duration::from_secs(500),
        "on its spawn's time: {back_in:?}"
    );
    let (credited, back_in) = walk(2768, 665).await;
    assert!(credited.is_some());
    let back_in = back_in.expect("gone at the end");
    assert!(
        back_in <= Duration::from_secs(1),
        "an instant respawn: {back_in:?}"
    );
}

/// The real world, loaded once: Molten Core's rune doused by its
/// Quintessence, a scripted door refused, Scholomance's doors as their
/// rules have them, the tribute's start, Stoned statues, player limits, a
/// raid reset with a raid inside and the group's lockout, and the escort
/// paths by point id.
#[tokio::test]
#[ignore = "open: on the merged ai5 the Molten Core rune isn't doused by its Quintessence (AI5_GAPS I2)"]
async fn the_real_instances() {
    let dir = std::path::Path::new("<runs>/tt-data/db");
    if !dir.join("vmangos.sqlite").exists() {
        return;
    }
    let realm = non_gm(crate::game::data::load::load(dir));
    let data = realm.data.clone();
    // Player limits, from the templates.
    assert_eq!(instances::limit(&data, 230), 5, "Blackrock Depths");
    assert_eq!(instances::limit(&data, 33), 10, "Shadowfang Keep");
    assert_eq!(instances::limit(&data, 409), 40, "Molten Core");
    assert_eq!(instances::limit(&data, 269), usize::MAX, "Caverns of Time");
    // The escortees whose paths start at 1 act by their point ids.
    for entry in [1379, 3692, 5644, 7998, 8516, 9520, 12277] {
        let ids: Vec<u32> = data.world.script_waypoints[&entry]
            .iter()
            .map(|p| p.point)
            .collect();
        assert_eq!(ids.first(), Some(&1), "{entry}'s path starts at 1");
        for e in crate::game::map::escorts::catalog::ESCORTS
            .iter()
            .filter(|e| e.entry == entry && e.path.is_none())
        {
            if let Some(c) = e.credit_point {
                assert!(ids.contains(&c), "quest {}'s credit point {c}", e.quest);
            }
        }
    }
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    raid_group(&realm, a.guid, &[b.guid]);
    let me = a.guid;
    // Molten Core: Sulfuron dead, the Rune of Koro doused by Aqual
    // Quintessence's spell.
    go(&realm, &mut a, 409).await;
    let mc = key_of(&realm, me);
    let map = realm.registry.online(me).unwrap().map();
    let doused = map
        .query(move |m| {
            instance_script::set_data(m, 0, DONE, 0);
            let _ = crate::game::map::inventory::add_item(m, me, 17333, 1, false);
            let item = m.players[&me]
                .items
                .iter()
                .find(|i| i.entry == 17333)
                .map(|i| i.guid);
            let rune = m
                .gameobjects
                .iter()
                .find(|(_, o)| o.entry == 176951)
                .map(|(_, o)| (o.guid, o.position))
                .unwrap();
            if let Some(p) = m.players.get_mut(&me) {
                p.movement.position = Vector3d {
                    x: rune.1.x + 2.0,
                    ..rune.1
                };
            }
            crate::game::map::spells::cast(
                m,
                me,
                21358,
                crate::game::map::spells::Targets {
                    object: Some(rune.0),
                    ..Default::default()
                },
                item,
            );
            item.is_some()
        })
        .await
        .unwrap();
    assert!(doused, "the Quintessence in her bags");
    let mut out = false;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        out = map
            .query(|m| {
                instance_script::get_data(m, 16) == DONE
                    && crate::game::map::scripts::object_of_spawn(m, 43157).is_none()
            })
            .await
            .unwrap();
        if out {
            break;
        }
    }
    assert!(out, "the rune doused and its circle out");
    // A raid boss bound, then the reset with both inside.
    go(&realm, &mut b, 409).await;
    kill(&realm, me, 12118).await;
    a.wait_for("SMSG_INSTANCE_SAVE_CREATED", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_INSTANCE_SAVE_CREATED(_)).then_some(())
    })
    .await;
    // The group follows Alice's save: Carol, new, goes into it.
    let mut c = player(&realm, "Carol").await;
    raid_group(&realm, me, &[c.guid]);
    go(&realm, &mut c, 409).await;
    assert_eq!(key_of(&realm, c.guid), mc, "the group's lockout");
    c.wait_for("SMSG_INSTANCE_SAVE_CREATED", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_INSTANCE_SAVE_CREATED(_)).then_some(())
    })
    .await;
    {
        let mut inst = realm.social.instances.lock();
        inst.raids.get_mut(&mc.instance).unwrap().reset_at = 0;
    }
    instances::sweep(&realm);
    assert!(running(&realm, mc).await && instances::expired(&realm, mc));
    b.wait_for("the grace", |m| match m {
        ServerOpcodeMessage::SMSG_RAID_GROUP_ONLY(r) if r.homebind_timer > 0 => Some(()),
        _ => None,
    })
    .await;
    assert!(
        realm.registry.online(b.guid).is_some(),
        "Bob still in the world"
    );
    // Upper Blackrock Spire: Emberseer's door is the script's.
    go(&realm, &mut a, 229).await;
    let map = realm.registry.online(me).unwrap().map();
    let refused = map
        .query(move |m| {
            let (g, at) = m
                .gameobjects
                .iter()
                .find(|(_, o)| o.entry == 175244)
                .map(|(_, o)| (o.guid, o.position))
                .unwrap();
            if let Some(p) = m.players.get_mut(&me) {
                p.movement.position = at;
            }
            let before = m.gameobject(g).map(|o| o.fields.get_u32(GAMEOBJECT_STATE));
            crate::game::map::world::use_object(m, me, g);
            let after = m.gameobject(g).map(|o| o.fields.get_u32(GAMEOBJECT_STATE));
            instance_script::scripted_door(m, g) && before == after
        })
        .await
        .unwrap();
    assert!(refused, "Emberseer In is the script's to work");
    // Scholomance: the room gates open as the world has them, Kirtonos's
    // gate as his fight goes.
    go(&realm, &mut a, 289).await;
    let map = realm.registry.online(me).unwrap().map();
    let gates = map
        .query(|m| {
            let state = |m: &crate::game::map::Map, e| {
                m.gameobjects
                    .iter()
                    .find(|(_, o)| o.entry == e)
                    .map(|(_, o)| o.fields.get_u32(GAMEOBJECT_STATE))
            };
            let room = state(m, 177371);
            instance_script::set_data(m, 7, IN_PROGRESS, 0);
            let fighting = state(m, 175570);
            instance_script::set_data(m, 7, DONE, 0);
            (room, fighting, state(m, 175570))
        })
        .await
        .unwrap();
    assert_eq!(gates, (Some(0), Some(1), Some(0)));
    // Dire Maul: the tribute's six guards.
    go(&realm, &mut a, 429).await;
    let map = realm.registry.online(me).unwrap().map();
    let tribute = map.query(|m| instance_script::get_data(m, 15)).await;
    assert_eq!(tribute, Some(6));
    // Blackrock Depths: the stoned Warbringer Constructs can't be selected.
    go(&realm, &mut a, 230).await;
    let map = realm.registry.online(me).unwrap().map();
    let statue = map
        .query(|m| {
            crate::game::map::scripts::creature_of_spawn(m, 46608)
                .and_then(|g| m.creature(g))
                .map(|c| c.fields.get_u32(UNIT_FIELD_FLAGS) & NOT_SELECTABLE)
        })
        .await
        .flatten();
    assert_eq!(statue, Some(NOT_SELECTABLE));
}
