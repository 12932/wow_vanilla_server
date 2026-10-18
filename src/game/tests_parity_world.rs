//! Instances, quests and events as Thistle Tea has them (stage 2e and 2f):
//! raid binds for player killers only, raised chests and Razorfen Downs'
//! gong across a reload, a group adopting its leader's copies as it forms,
//! the escorts' holds and awaits, The Nightmare Manifests, and the event
//! quests walked through on the real world.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::scripts::MapTemplate;
use crate::game::data::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::instances;
use crate::game::map::MapKey;
use crate::game::map::instance_script::DONE;

const RAID: u32 = 409;
const LORD: u32 = 12118;

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
    from.call(move |m| m.teleport(g, MapKey::world(map), Vector3d::default(), 0.0));
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

/// `leader` and `members` in a raid group.
fn raid_group(realm: &Arc<Realm>, leader: Guid, members: &[Guid]) {
    let l = realm.registry.online(leader).unwrap();
    for m in members {
        let m = realm.registry.online(*m).unwrap();
        assert!(crate::game::social::groups::join_directly(realm, &l, &m));
    }
    crate::game::social::groups::convert_to_raid(realm, &l);
}

/// A raid whose two Lucifrons each bind it, a hound of no one's beside
/// them, and a dungeon.
fn raid_and_dungeon() -> GameData {
    let mut d = GameData::empty();
    map_info(&mut d, RAID, 2, 40, "");
    let mut lord = template(LORD, "Lucifron");
    lord.rank = 3;
    lord.static_flags2 = instances::STATIC_FLAG2_LOCKS_RAID;
    d.creature_templates.insert(LORD, lord);
    d.creature_templates
        .insert(11671, template(11671, "Core Hound"));
    d.creature_spawns.insert(
        RAID,
        [56_000, 56_001]
            .into_iter()
            .map(|guid| CreatureSpawn {
                guid,
                entries: vec![LORD],
                map: RAID,
                x: 3.0,
                y: (guid - 56_000) as f32 * 5.0,
                ..Default::default()
            })
            .collect(),
    );
    d
}

/// The first living creature of `entry` in `g`'s map.
async fn living(realm: &Arc<Realm>, g: Guid, entry: u32) -> Guid {
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        m.creatures
            .iter()
            .filter(|(_, c)| c.entry == entry && c.alive())
            .map(|(_, c)| c.guid)
            .min_by_key(|c| c.guid())
    })
    .await
    .flatten()
    .expect("alive")
}

/// A boss that locks its raid binds those inside only when a player (or
/// what a player owns) kills it, as Thistle Tea's `raid_killer` has it: a
/// hound's kill binds no one; a hound of Alice's binds her raid.
#[tokio::test]
async fn a_raid_is_bound_only_by_a_players_kill() {
    let realm = lasting(raid_and_dungeon(), Duration::from_millis(200));
    let mut a = player(&realm, "Alice").await;
    let b = player(&realm, "Bob").await;
    raid_group(&realm, a.guid, &[b.guid]);
    go(&realm, &mut a, RAID).await;
    let copy = key_of(&realm, a.guid);
    let me = a.guid;
    let map = realm.registry.online(me).unwrap().map();
    let first = living(&realm, me, LORD).await;
    // A hound of no one's kills the first.
    map.query(move |m| {
        let at = m.players[&me].position();
        let hound =
            crate::game::map::scripts::summon(m, Guid::zero(), 11671, at, 0.0, None).unwrap();
        crate::game::map::combat::deal_damage(m, hound, first, 1_000_000);
    })
    .await;
    let dead = map
        .query(move |m| m.creature(first).is_some_and(|c| !c.alive()))
        .await
        .unwrap();
    assert!(dead, "the first is dead");
    let saved = a
        .gets(Duration::from_millis(300), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_INSTANCE_SAVE_CREATED(_)).then_some(())
        })
        .await;
    assert!(saved.is_none(), "no save for a creature's kill");
    let bound = |realm: &Realm| {
        realm
            .social
            .instances
            .lock()
            .raid_bind(guid::counter(me), RAID)
    };
    assert_eq!(bound(&realm), None, "not bound by a creature's kill");
    // A hound of hers kills the second: she is bound.
    let second = living(&realm, me, LORD).await;
    map.query(move |m| {
        let at = m.players[&me].position();
        let hound =
            crate::game::map::scripts::summon(m, Guid::zero(), 11671, at, 0.0, None).unwrap();
        if let Some(h) = m.creature_mut(hound) {
            h.owner = Some(me);
        }
        crate::game::map::combat::deal_damage(m, hound, second, 1_000_000);
    })
    .await;
    a.wait_for("SMSG_INSTANCE_SAVE_CREATED", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_INSTANCE_SAVE_CREATED(_)).then_some(())
    })
    .await;
    assert_eq!(bound(&realm), Some(copy.instance), "bound by her hound's");
}

/// A store in a file of its own, for a restart.
fn store_file(what: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "wvs-2e-{what}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("characters.sqlite")
}

/// A server on the store at `path`, whose empty copies last.
fn server(path: &std::path::Path, data: GameData) -> Arc<Realm> {
    let (store, startup) = crate::game::store::Store::open(path.to_str().unwrap()).unwrap();
    let mut cfg = harness::test_config();
    cfg.instance_unload = Duration::from_secs(600);
    Realm::new(cfg, store, startup, data, None)
}

/// The old server saves `g` and its copies and goes; a new one on the same
/// store takes `g` back in where they were.
async fn restart(
    realm: &Arc<Realm>,
    path: &std::path::Path,
    data: GameData,
    g: Guid,
) -> (Arc<Realm>, Client) {
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| m.save_player(g)).await;
    realm.store.flush().await;
    let realm2 = server(path, data);
    let mut c = Client::connect(&realm2, "A");
    c.login(g).await;
    c.sees(g).await;
    (realm2, c)
}

const RFD: u32 = 129;
const GONG: u32 = 148917;
const TOMB_FIEND: u32 = 7349;
const TUTENKASH: u32 = 7355;
const GO_FLAG_NO_INTERACT: u32 = 0x10;

/// Razorfen Downs with its gong, and what it calls.
fn razorfen_downs() -> GameData {
    let mut d = GameData::empty();
    map_info(&mut d, RFD, 1, 5, "instance_razorfen_downs");
    d.gameobject_templates.insert(
        GONG,
        GameObjectTemplate {
            entry: GONG,
            kind: 10,
            display_id: 411,
            name: "Gong".into(),
            size: 1.0,
            ..Default::default()
        },
    );
    d.gameobject_spawns.insert(
        RFD,
        vec![GameObjectSpawn {
            guid: 32045,
            entry: GONG,
            map: RFD,
            x: 2.0,
            rotation: [0.0, 0.0, 0.0, 1.0],
            state: 1,
            ..Default::default()
        }],
    );
    for (entry, name) in [
        (TOMB_FIEND, "Tomb Fiend"),
        (7351, "Tomb Reaver"),
        (TUTENKASH, "Tuten'kash"),
    ] {
        d.creature_templates.insert(entry, template(entry, name));
    }
    d
}

/// The gong's waves called, whether it can be struck, and how many of
/// `entry` are up.
async fn gong(realm: &Arc<Realm>, g: Guid, entry: u32) -> (u32, bool, usize) {
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        let flags = m
            .gameobjects
            .iter()
            .find(|(_, o)| o.entry == GONG)
            .map_or(u32::MAX, |(_, o)| {
                o.fields.get_u32(crate::game::fields::GAMEOBJECT_FLAGS)
            });
        let up = m
            .creatures
            .iter()
            .filter(|(_, c)| c.entry == entry && c.alive())
            .count();
        (
            crate::game::map::instance_script::get_data(m, 1),
            flags & GO_FLAG_NO_INTERACT == 0,
            up,
        )
    })
    .await
    .unwrap()
}

async fn strike(realm: &Arc<Realm>, g: Guid) {
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        let gong = m
            .gameobjects
            .iter()
            .find(|(_, o)| o.entry == GONG)
            .map(|(_, o)| o.guid)
            .unwrap();
        crate::game::map::scripts::object_used(m, g, gong);
    })
    .await;
}

/// Razorfen Downs' gong across a restart: a wave under way when the copy
/// went is called again, the fiends first, then Tuten'kash, so the gong
/// is as it was before the wave was struck.
#[tokio::test]
async fn razorfen_downs_gong_is_kept_across_a_reload() {
    let path = store_file("gong");
    let realm = server(&path, razorfen_downs());
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    go(&realm, &mut a, RFD).await;
    strike(&realm, me).await;
    assert_eq!(gong(&realm, me, TOMB_FIEND).await, (1, false, 8), "struck");
    let (realm, _a) = restart(&realm, &path, razorfen_downs(), me).await;
    assert_eq!(
        gong(&realm, me, TOMB_FIEND).await,
        (0, true, 0),
        "the fiends' wave to call again"
    );
    strike(&realm, me).await;
    assert_eq!(gong(&realm, me, TOMB_FIEND).await, (1, false, 8));
    // The reavers dead, Tuten'kash called; the copy goes before he dies.
    let map = realm.registry.online(me).unwrap().map();
    map.query(|m| crate::game::map::instance_script::set_data(m, 1, 14, 0))
        .await;
    assert!(gong(&realm, me, TUTENKASH).await.1, "awake for the last");
    strike(&realm, me).await;
    assert_eq!(gong(&realm, me, TUTENKASH).await, (15, false, 1));
    let (realm, _a) = restart(&realm, &path, razorfen_downs(), me).await;
    assert_eq!(
        gong(&realm, me, TUTENKASH).await,
        (14, true, 0),
        "Tuten'kash to call again"
    );
    strike(&realm, me).await;
    assert_eq!(gong(&realm, me, TUTENKASH).await, (15, false, 1));
    // Slain, he stays slain: the gong is still for good.
    let map = realm.registry.online(me).unwrap().map();
    map.query(move |m| {
        let t = m
            .creatures
            .iter()
            .find(|(_, c)| c.entry == TUTENKASH && c.alive())
            .map(|(_, c)| c.guid)
            .unwrap();
        crate::game::map::combat::deal_damage(m, me, t, 1_000_000);
    })
    .await;
    let (realm, _a) = restart(&realm, &path, razorfen_downs(), me).await;
    assert_eq!(gong(&realm, me, TUTENKASH).await, (15, false, 0), "done");
}

const DIRE_MAUL: u32 = 429;
const TRIBUTE: u32 = 396409;
const SHARD: u32 = 44726;

/// Dire Maul with its tribute chest (three things and a little gold) and a
/// Felvine Shard (one thing), both kept back until their fights are won,
/// and taken when emptied.
fn dire_maul() -> GameData {
    let mut d = GameData::empty();
    map_info(&mut d, DIRE_MAUL, 1, 5, "instance_dire_maul");
    for (entry, name, loot, gold) in [
        (179564, "Gordok Tribute", 16577, 100),
        (179559, "Felvine Shard", 16594, 0),
    ] {
        let mut t = GameObjectTemplate {
            entry,
            kind: 3,
            display_id: 411,
            name: name.into(),
            size: 1.0,
            gold_min: gold,
            gold_max: gold,
            ..Default::default()
        };
        t.data[1] = loot;
        t.data[3] = 1;
        d.gameobject_templates.insert(entry, t);
    }
    d.gameobject_spawns.insert(
        DIRE_MAUL,
        [(TRIBUTE, 179564), (SHARD, 179559)]
            .into_iter()
            .map(|(guid, entry)| GameObjectSpawn {
                guid,
                entry,
                map: DIRE_MAUL,
                x: 2.0,
                rotation: [0.0, 0.0, 0.0, 1.0],
                respawn_min: 7200,
                respawn_max: 7200,
                state: 1,
                despawned: true,
                ..Default::default()
            })
            .collect(),
    );
    for (table, items) in [
        (16577, &[18_000u32, 18_001, 18_002][..]),
        (16594, &[18_501]),
    ] {
        d.loot.gameobject.insert(
            table,
            items
                .iter()
                .map(|item| LootEntry {
                    item: *item,
                    chance: 100.0,
                    min: 1,
                    max: 1,
                    ..Default::default()
                })
                .collect(),
        );
        for item in items {
            d.items.insert(
                *item,
                ItemTemplate {
                    entry: *item,
                    class: 15,
                    name: format!("Thing {item}"),
                    quality: 2,
                    stackable: 1,
                    ..Default::default()
                },
            );
        }
    }
    d
}

/// The raised object of `spawn`, and what is left in it: each slot's item
/// and whether it is taken, and the gold.
type Chest = Option<(Vec<(u32, bool)>, u32)>;

async fn chest(realm: &Arc<Realm>, g: Guid, spawn: u32) -> Option<Chest> {
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        crate::game::map::scripts::object_of_spawn(m, spawn).map(|o| {
            m.gameobject(o)
                .and_then(|o| o.loot.as_ref())
                .map(|l| (l.items.iter().map(|s| (s.item, s.taken)).collect(), l.gold))
        })
    })
    .await
    .flatten()
}

/// `g` opens the raised object of `spawn`, takes the first `take` of what
/// is in it and the gold, and closes it.
async fn loot_chest(realm: &Arc<Realm>, g: Guid, spawn: u32, take: u8) {
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        let o = crate::game::map::scripts::object_of_spawn(m, spawn).unwrap();
        let at = m.gameobject(o).unwrap().position;
        m.players.get_mut(&g).unwrap().movement.position = at;
        crate::game::map::loot::open_object(m, g, o);
        for slot in 0..take {
            crate::game::map::loot::take_item(m, g, slot);
        }
        crate::game::map::loot::take_money(m, g);
        crate::game::map::loot::release(m, g, o);
    })
    .await;
}

/// The chests a copy's script raises keep what is in them across a
/// reload: the tribute, opened and part taken, comes back with the rest;
/// the shard, emptied and gone, doesn't come back before its time.
#[tokio::test]
async fn raised_chests_keep_their_loot_across_a_reload() {
    let path = store_file("chests");
    let realm = server(&path, dire_maul());
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    go(&realm, &mut a, DIRE_MAUL).await;
    let map = realm.registry.online(me).unwrap().map();
    map.query(|m| {
        crate::game::map::instance_script::set_data(m, 6, DONE, 0);
        crate::game::map::instance_script::set_data(m, 11, DONE, 0);
    })
    .await;
    assert_eq!(chest(&realm, me, TRIBUTE).await, Some(None), "raised, shut");
    loot_chest(&realm, me, TRIBUTE, 1).await;
    let left = chest(&realm, me, TRIBUTE).await.flatten().expect("opened");
    assert_eq!(
        left,
        (vec![(18_000, true), (18_001, false), (18_002, false)], 0),
        "the first thing and the gold taken"
    );
    loot_chest(&realm, me, SHARD, 1).await;
    assert_eq!(chest(&realm, me, SHARD).await, None, "emptied, it goes");
    let (realm, _a) = restart(&realm, &path, dire_maul(), me).await;
    assert_eq!(
        chest(&realm, me, TRIBUTE).await,
        Some(Some(left)),
        "the tribute as it was left"
    );
    assert_eq!(
        chest(&realm, me, SHARD).await,
        None,
        "the shard not back before its time"
    );
}

const DUNGEON: u32 = 36;

fn dungeon() -> GameData {
    let mut d = GameData::empty();
    map_info(&mut d, DUNGEON, 1, 10, "");
    d
}

fn owner_of(realm: &Realm, copy: MapKey) -> Option<instances::Owner> {
    instances::save_of(realm, copy.instance).and_then(|s| s.owner)
}

fn group_id(realm: &Realm, g: Guid) -> u32 {
    realm.social.groups.lock().group_of(g).unwrap().id
}

/// A group takes over its leader's copies as it forms, as Thistle Tea's
/// `group_changed` hands them over, before anyone goes in; one disbanded
/// leaves them without an owner, for its leader's next group; and a group
/// keeps them when its leader leaves it.
#[tokio::test]
async fn a_group_adopts_its_leaders_copies_as_it_forms() {
    let realm = lasting(dungeon(), Duration::from_secs(600));
    let mut a = player(&realm, "Alice").await;
    let b = player(&realm, "Bob").await;
    let mut c = player(&realm, "Carol").await;
    let online = |g: Guid| realm.registry.online(g).unwrap();
    go(&realm, &mut a, DUNGEON).await;
    let solo = key_of(&realm, a.guid);
    go(&realm, &mut a, 0).await;
    assert_eq!(
        owner_of(&realm, solo),
        Some(instances::Owner::Player(guid::counter(a.guid)))
    );
    // Alice and Bob: the group's at once.
    assert!(crate::game::social::groups::join_directly(
        &realm,
        &online(a.guid),
        &online(b.guid)
    ));
    let first = group_id(&realm, a.guid);
    assert_eq!(
        owner_of(&realm, solo),
        Some(instances::Owner::Group(first)),
        "adopted as the group forms"
    );
    // Bob goes, and the group with him: no one's.
    crate::game::social::groups::leave(&realm, &online(b.guid));
    assert_eq!(owner_of(&realm, solo), None, "left without an owner");
    // Alice's next group takes it up.
    assert!(crate::game::social::groups::join_directly(
        &realm,
        &online(a.guid),
        &online(c.guid)
    ));
    let second = group_id(&realm, a.guid);
    assert_eq!(
        owner_of(&realm, solo),
        Some(instances::Owner::Group(second))
    );
    assert!(crate::game::social::groups::join_directly(
        &realm,
        &online(a.guid),
        &online(b.guid)
    ));
    // Alice leaves the three; Carol leads, and the copy is still theirs.
    crate::game::social::groups::leave(&realm, &online(a.guid));
    assert_eq!(
        owner_of(&realm, solo),
        Some(instances::Owner::Group(second))
    );
    go(&realm, &mut c, DUNGEON).await;
    assert_eq!(key_of(&realm, c.guid), solo, "the group's copy");
    // Alone, Alice has none of her own now.
    go(&realm, &mut a, DUNGEON).await;
    assert_ne!(key_of(&realm, a.guid), solo, "a copy of her own");
}

/// An escortee of `entry` for `quest`, its path's points `points` two
/// yards apart east from `start`, spawned a step behind the first.
fn escortee(
    d: &mut GameData,
    entry: u32,
    quest: u32,
    start: (f32, f32, f32),
    points: std::ops::RangeInclusive<u32>,
) {
    let mut t = template(entry, "Escortee");
    t.npc_flags = 0x2;
    t.health_max = 100_000;
    d.creature_templates.insert(entry, t);
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: entry,
        entries: vec![entry],
        map: 0,
        x: start.0 - 1.5,
        y: start.1,
        z: start.2,
        respawn_min: 600,
        respawn_max: 600,
        ..Default::default()
    });
    d.npc.quests.insert(
        quest,
        crate::game::data::npc::Quest {
            id: quest,
            title: format!("Quest {quest}"),
            min_level: 1,
            level: 1,
            special_flags: 2,
            ..Default::default()
        },
    );
    d.npc.creature_starts.insert(entry, vec![quest]);
    d.world.script_waypoints.insert(
        entry,
        points
            .map(|i| crate::game::data::world::ScriptPoint {
                point: i,
                x: start.0 + 2.0 * i as f32,
                y: start.1,
                z: start.2,
                wait_ms: 0,
            })
            .collect(),
    );
}

/// Accept `quest` at the escortee of `entry`, and send it on at once, fast.
async fn set_out(map: &crate::game::map::MapHandle, g: Guid, entry: u32, quest: u32) -> Guid {
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
    escortee
}

/// What an escort is doing as it goes: where its escortee is, whether it
/// is held, whether the quest is explored (or failed), and whether it is
/// over. The player keeps at the escortee's side.
#[derive(Debug, Clone, Copy)]
struct Going {
    at: Vector3d,
    held: bool,
    explored: bool,
    failed: bool,
    over: bool,
}

async fn going(map: &crate::game::map::MapHandle, g: Guid, escortee: Guid, quest: u32) -> Going {
    map.query(move |m| {
        let at = m
            .creature(escortee)
            .map_or(Vector3d::default(), |c| c.position);
        if let Some(p) = m.players.get_mut(&g) {
            p.movement.position = at;
        }
        let log = m.players[&g].journal.log.iter().find(|l| l.quest == quest);
        Going {
            at,
            held: m.escorts.iter().any(|e| e.quest == quest && e.held()),
            explored: log.is_some_and(|l| l.explored),
            failed: log.is_some_and(|l| l.failed),
            over: !m.escorts.iter().any(|e| e.quest == quest),
        }
    })
    .await
    .unwrap()
}

/// Where a new human starts: escortees stand here, in her sight.
const HUMAN_START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

const GRARK: u32 = 9520;
const AMBUSHERS: [u32; 3] = [9522, 9605, 7042];

/// Grark Lorkrub's escort (4121) holds at each ambush until the ambushers
/// are gone, however long that takes, then says his line and goes on, as
/// Thistle Tea's `{:hold, ...}` has it; at the end he is credited and dies
/// as the scene ends.
#[tokio::test]
async fn an_escort_holds_at_an_ambush_until_its_ambushers_are_gone() {
    let start = HUMAN_START;
    let mut d = GameData::empty();
    escortee(&mut d, GRARK, 4121, start, 1..=45);
    for e in AMBUSHERS.into_iter().chain([7046, 9538, 9539]) {
        d.creature_templates.insert(e, template(e, "Blackrock"));
    }
    let realm = harness::realm_with(d);
    let a = player(&realm, "Alice").await;
    let g = a.guid;
    let map = realm.registry.online(g).unwrap().map();
    let grark = set_out(&map, g, GRARK, 4121).await;
    let ambushers = move |m: &crate::game::map::Map| -> Vec<Guid> {
        m.creatures
            .iter()
            .filter(|(_, c)| AMBUSHERS.contains(&c.entry) && c.alive())
            .map(|(_, c)| c.guid)
            .collect()
    };
    // To the first ambush, where he holds.
    let mut now = going(&map, g, grark, 4121).await;
    for _ in 0..500 {
        if now.held {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        now = going(&map, g, grark, 4121).await;
    }
    assert!(now.held, "held at the ambush: {now:?}");
    let ambush_x = start.0 + 24.0;
    assert!((now.at.x - ambush_x).abs() < 2.5, "at point 12: {now:?}");
    let up = map.query(move |m| ambushers(m).len()).await.unwrap();
    assert_eq!(up, 4, "four ambushers");
    // Six minutes on, they still stand, and so does he.
    for _ in 0..6 {
        map.query(|m| {
            for e in m.escorts.iter_mut() {
                e.hurry(Duration::from_secs(60));
            }
        })
        .await;
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    let now = going(&map, g, grark, 4121).await;
    assert!(
        now.held && (now.at.x - ambush_x).abs() < 2.5,
        "still held: {now:?}"
    );
    // The ambushers dead as they come, he goes on to the end.
    let mut end = now;
    for _ in 0..1500 {
        map.query(move |m| {
            for f in ambushers(m) {
                crate::game::map::combat::deal_damage(m, g, f, 1_000_000);
            }
            for e in m.escorts.iter_mut() {
                e.hurry(Duration::from_secs(2));
            }
        })
        .await;
        end = going(&map, g, grark, 4121).await;
        if end.over {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(end.over && end.explored, "credited at the end: {end:?}");
    let dead = map
        .query(move |m| m.creature(grark).is_some_and(|c| !c.alive()))
        .await
        .unwrap();
    assert!(dead, "he dies as the scene ends");
}

const CELEBRAS: u32 = 13716;
const INCANTATION: u32 = 178965;

/// Celebras's escort (7046) waits at his altar for the tome he sets out to
/// be read, as Thistle Tea's `{:await, ...}` has it: read, he goes on and
/// the quest is done; left unread for 36 seconds, he gives up, and the
/// quest fails.
#[tokio::test]
async fn an_escort_awaits_its_release_or_gives_up() {
    let start = HUMAN_START;
    let mut d = GameData::empty();
    escortee(&mut d, CELEBRAS, 7046, start, 1..=13);
    for (entry, name) in [(INCANTATION, "Incantation of Celebras"), (178964, "Aura")] {
        d.gameobject_templates.insert(
            entry,
            GameObjectTemplate {
                entry,
                kind: 10,
                display_id: 411,
                name: name.into(),
                size: 1.0,
                ..Default::default()
            },
        );
    }
    let realm = harness::realm_with(d);
    let a = player(&realm, "Alice").await;
    let g = a.guid;
    let map = realm.registry.online(g).unwrap().map();
    let tome =
        |m: &crate::game::map::Map| m.gameobjects.iter().any(|(_, o)| o.entry == INCANTATION);
    for read in [true, false] {
        let celebras = set_out(&map, g, CELEBRAS, 7046).await;
        let mut now = going(&map, g, celebras, 7046).await;
        for _ in 0..500 {
            if now.held {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
            now = going(&map, g, celebras, 7046).await;
        }
        assert!(now.held, "waiting at the altar: {now:?}");
        assert!(
            (now.at.x - (start.0 + 8.0)).abs() < 2.5,
            "at point 4: {now:?}"
        );
        // The tome comes out; he waits on.
        map.query(|m| {
            for e in m.escorts.iter_mut() {
                e.hurry(Duration::from_secs(6));
            }
        })
        .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            map.query(move |m| tome(m)).await.unwrap(),
            "the tome set out"
        );
        assert!(going(&map, g, celebras, 7046).await.held, "still waiting");
        if read {
            // Read: its script lets him go.
            map.query(move |m| {
                let step = crate::game::data::scripts::ScriptStep {
                    command: crate::game::map::scripts::C_RELEASE_WAYPOINTS,
                    ..Default::default()
                };
                crate::game::map::scripts::execute(m, &step, celebras, g);
            })
            .await;
        } else {
            map.query(|m| {
                for e in m.escorts.iter_mut() {
                    e.hurry(Duration::from_secs(31));
                }
            })
            .await;
        }
        let mut end = going(&map, g, celebras, 7046).await;
        for _ in 0..500 {
            map.query(|m| {
                for e in m.escorts.iter_mut() {
                    e.hurry(Duration::from_secs(4));
                }
            })
            .await;
            end = going(&map, g, celebras, 7046).await;
            if end.over {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(end.over, "over: {end:?}");
        if read {
            assert!(end.explored && !end.failed, "done: {end:?}");
            // Taken again, for the other way.
            map.query(move |m| {
                let p = m.players.get_mut(&g).unwrap();
                p.journal.log.retain(|l| l.quest != 7046);
                crate::game::map::ai::respawn_now(m, celebras);
            })
            .await;
        } else {
            assert!(end.failed && !end.explored, "failed: {end:?}");
            let home = map
                .query(move |m| {
                    m.creatures
                        .iter()
                        .find(|(_, c)| c.entry == CELEBRAS && c.alive())
                        .map(|(_, c)| c.position.x)
                })
                .await
                .flatten();
            assert_eq!(home, Some(start.0 - 1.5), "back at his spawn");
        }
    }
}

// The real world: event quests walked through.

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

fn real_realm() -> Arc<Realm> {
    harness::realm_with(crate::game::data::load::load(DB.as_path()))
}

type Map = crate::game::map::Map;
type MapHandle = crate::game::map::MapHandle;

/// A player of `race` and `class` at `level`, in the world, hard to kill.
async fn hero(
    realm: &Arc<Realm>,
    account: &str,
    race: Race,
    class: Class,
    level: u8,
) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let name = format!("{}{}", &account[..1], account[1..].to_lowercase());
    let g = c.enter(&name, race, class).await;
    c.send(CMSG_COMPLETE_CINEMATIC {}).await;
    let map = realm.registry.online(g).expect("online").map();
    map.query(move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::stats::set_level(p, &data, level);
    })
    .await;
    (c, g, map)
}

/// `g` stands at `at` now, as their client would have moved them there:
/// the grid and what they see follow.
fn place(m: &mut Map, g: Guid, at: Vector3d) {
    let Some(p) = m.players.get_mut(&g) else {
        return;
    };
    let (old, new) = (p.cell, crate::game::map::grid::cell_of(at.x, at.y));
    let anchor = p.vis_anchor;
    p.movement.position = at;
    p.cell = new;
    m.grid.relocate(g, old, new);
    crate::game::map::visibility::moved(m, g, anchor, at);
}

/// Whatever comes at `g` does them no harm.
fn hardy(m: &mut Map, g: Guid) {
    if let Some(p) = m.players.get_mut(&g) {
        p.fields
            .set_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH, 1_000_000);
        p.fields
            .set_u32(crate::game::fields::UNIT_FIELD_HEALTH, 1_000_000);
    }
}

/// The first creature of `entry` alive here, and where it stands.
fn npc(m: &Map, entry: u32) -> Option<(Guid, Vector3d)> {
    m.creatures
        .iter()
        .filter(|(_, c)| c.entry == entry && c.alive())
        .map(|(_, c)| (c.guid, c.position))
        .min_by_key(|(g, _)| g.guid())
}

/// The first object of `entry` here.
fn object(m: &Map, entry: u32) -> Option<(Guid, Vector3d)> {
    m.gameobjects
        .iter()
        .filter(|(_, o)| o.entry == entry)
        .map(|(_, o)| (o.guid, o.position))
        .min_by_key(|(g, _)| g.guid())
}

/// `c` at `at`, at once.
async fn put(map: &MapHandle, c: &mut Client, at: Vector3d) {
    let g = c.guid;
    map.query(move |m| m.teleport(g, m.key, at, 0.0)).await;
    c.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
    assert!(
        until(map, move |m| m
            .players
            .get(&g)
            .is_some_and(|p| p.teleport.is_none()))
        .await,
        "teleported"
    );
}

fn beside(at: Vector3d) -> Vector3d {
    Vector3d {
        x: at.x + 2.0,
        ..at
    }
}

async fn until(map: &MapHandle, what: impl Fn(&Map) -> bool + Send + Clone + 'static) -> bool {
    for _ in 0..500 {
        let w = what.clone();
        if map.query(move |m| w(m)).await.unwrap_or(false) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// [`until`], free to change the map as it looks.
async fn until_mut(
    map: &MapHandle,
    what: impl Fn(&mut Map) -> bool + Send + Clone + 'static,
) -> bool {
    for _ in 0..500 {
        let w = what.clone();
        if map.query(move |m| w(m)).await.unwrap_or(false) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

fn logged(m: &Map, g: Guid, quest: u32) -> bool {
    m.players[&g].journal.has(quest)
}

fn done(m: &Map, g: Guid, quest: u32) -> bool {
    let p = &m.players[&g];
    let q = m.realm.data.npc.quests.get(&quest).expect("quest");
    p.journal
        .slot_of(quest)
        .is_some_and(|s| crate::game::map::quests::complete(p, q, &p.journal.log[s]))
}

fn rewarded(m: &Map, g: Guid, quest: u32) -> bool {
    m.players[&g].journal.rewarded.contains(&quest)
}

/// Where `entry` (a creature, else an object) is spawned: its map and
/// place.
fn spawn_of(realm: &Realm, entry: u32) -> Option<(u32, Vector3d)> {
    let data = &realm.data;
    let mut creatures: Vec<_> = data
        .creature_spawns
        .values()
        .flatten()
        .filter(|s| s.entries.contains(&entry))
        .map(|s| (s.guid, s.map, s.x, s.y, s.z))
        .collect();
    creatures.sort_by_key(|s| s.0);
    let mut objects: Vec<_> = data
        .gameobject_spawns
        .values()
        .flatten()
        .filter(|s| s.entry == entry)
        .map(|s| (s.guid, s.map, s.x, s.y, s.z))
        .collect();
    objects.sort_by_key(|s| s.0);
    creatures.first().or(objects.first()).map(|s| {
        (
            s.1,
            Vector3d {
                x: s.2,
                y: s.3,
                z: s.4,
            },
        )
    })
}

/// `c` at `at` in map `id`, across maps if need be: the map they are in.
async fn travel(realm: &Arc<Realm>, c: &mut Client, id: u32, at: Vector3d) -> MapHandle {
    let g = c.guid;
    let from = realm.registry.online(g).expect("online").map();
    let here = from.query(|m| m.key.map).await.unwrap();
    if here == id {
        put(&from, c, at).await;
        return from;
    }
    from.call(move |m| m.teleport(g, MapKey::world(id), at, 0.0));
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
    realm.registry.online(g).expect("online").map()
}

/// `c` beside the spawn of `entry`: the map, the creature or object, and
/// where it stands.
async fn visit(realm: &Arc<Realm>, c: &mut Client, entry: u32) -> (MapHandle, Guid, Vector3d) {
    match try_visit(realm, c, entry).await {
        Ok(v) => v,
        Err(why) => panic!("{why}"),
    }
}

/// [`visit`], or why not.
async fn try_visit(
    realm: &Arc<Realm>,
    c: &mut Client,
    entry: u32,
) -> Result<(MapHandle, Guid, Vector3d), String> {
    let (id, at) = spawn_of(realm, entry).ok_or(format!("{entry} not spawned"))?;
    let map = travel(realm, c, id, beside(at)).await;
    let (who, at) = map
        .query(move |m| npc(m, entry).or_else(|| object(m, entry)))
        .await
        .flatten()
        .ok_or(format!("{entry} not here"))?;
    put(&map, c, beside(at)).await;
    let g = c.guid;
    if guid::is_unit(who)
        && !until(&map, move |m| {
            m.creature(who).is_some_and(|c| c.observers.contains(&g))
        })
        .await
    {
        return Err(format!("{entry} not in sight"));
    }
    Ok((map, who, at))
}

/// Take `quest` from `giver`, beside it, `before` done: the map it is in.
async fn take(
    realm: &Arc<Realm>,
    c: &mut Client,
    giver: u32,
    quest: u32,
    before: &'static [u32],
) -> MapHandle {
    let g = c.guid;
    let (map, who, _) = visit(realm, c, giver).await;
    map.query(move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .journal
            .rewarded
            .extend(before.iter().copied());
        crate::game::map::quests::accept(m, g, who, quest);
        if debug() {
            let data = m.realm.data.clone();
            let p = &m.players[&g];
            let q = data.npc.quests.get(&quest).unwrap();
            eprintln!(
                "take {quest}: can_take {}, condition {}, level {}",
                crate::game::map::quests::can_take(p, q, &data),
                crate::game::map::quests::condition_ok(m, g, who, quest),
                p.level(),
            );
        }
    })
    .await;
    assert!(
        until(&map, move |m| logged(m, g, quest)).await,
        "{quest} taken"
    );
    map
}

/// Turn `quest` in at `ender`, beside it.
async fn turn_in(realm: &Arc<Realm>, c: &mut Client, ender: u32, quest: u32) {
    let g = c.guid;
    // An ender that escorted or followed takes quests again once it is
    // back as a quest giver (its escort's end brings it home).
    let mut at = None;
    for _ in 0..120 {
        let (map, who, _) = match try_visit(realm, c, ender).await {
            Ok(v) => v,
            // Gone with its escort's end: back at its spawn, as its
            // respawn brings it.
            Err(_) => {
                let (id, _) = spawn_of(realm, ender).expect("spawned");
                let map = realm.maps.get(realm, crate::game::map::MapKey::world(id));
                map.query(move |m| {
                    let dead: Vec<Guid> = m
                        .creatures
                        .iter()
                        .filter(|(_, c)| c.entry == ender && !c.alive())
                        .map(|(_, c)| c.guid)
                        .collect();
                    for d in dead {
                        crate::game::map::ai::respawn_now(m, d);
                    }
                })
                .await;
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let giving = map
            .query(move |m| {
                let giving = crate::game::map::npc::npc_entry(
                    m,
                    g,
                    who,
                    crate::game::map::npc::NPC_FLAG_QUESTGIVER,
                )
                .is_some();
                if !giving {
                    skip(m, Duration::from_secs(5));
                }
                giving
            })
            .await
            .unwrap_or(false);
        if giving {
            at = Some((map, who));
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let Some((map, who)) = at else {
        panic!("{quest}'s ender never takes quests again");
    };
    map.query(move |m| crate::game::map::quests::choose_reward(m, g, who, quest, 0))
        .await;
    assert!(
        until(&map, move |m| rewarded(m, g, quest)).await,
        "{quest} rewarded"
    );
}

/// The map's scripts, creature timers and escorts `by` on: what waits on
/// them comes sooner.
fn skip(m: &mut Map, by: Duration) {
    for r in m.scripts.runs.iter_mut() {
        if let Some(t) = r.started.checked_sub(by) {
            r.started = t;
        }
    }
    let sooner = |t: &mut std::time::Instant| {
        if let Some(s) = t.checked_sub(by) {
            *t = s;
        }
    };
    let all: Vec<Guid> = m.creatures.iter().map(|(_, c)| c.guid).collect();
    for g in all {
        if let Some(s) = crate::game::map::event_ai::scripted_mut(m, g) {
            for t in s.timers.iter_mut().flatten() {
                sooner(t);
            }
            if let Some(h) = s.hold.as_mut() {
                sooner(&mut h.until);
                sooner(&mut h.ready_at);
            }
        }
        // A wait at a waypoint.
        if let Some(c) = m.creature_mut(g)
            && let crate::game::map::creature::Motion::Idle { until: Some(t) } = &mut c.motion
        {
            sooner(t);
        }
    }
    for e in m.escorts.iter_mut() {
        e.hurry(by);
    }
}

fn health(m: &Map, g: Guid) -> (u32, u32) {
    m.unit_fields(g).map_or((0, 0), |f| {
        (
            f.get_u32(crate::game::fields::UNIT_FIELD_HEALTH),
            f.get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH),
        )
    })
}

/// Those of `entries` alive here.
fn alive_of(m: &Map, entries: &[u32]) -> Vec<Guid> {
    m.creatures
        .iter()
        .filter(|(_, c)| entries.contains(&c.entry) && c.alive())
        .map(|(_, c)| c.guid)
        .collect()
}

fn debug() -> bool {
    std::env::var("EVENTS_DEBUG").is_ok()
}

const UNIT_FLAG_NOT_ATTACKABLE: u32 = 0x2;

/// The Nightmare Manifests (8736): Keeper Remulos leads the player to his
/// shrine, pulls Eranikus from his rift and holds off the phantasms; the
/// Tyrant lands and fights; beaten to a fifth of his health, he is
/// redeemed, and Remulos credits the quest through its map event before
/// he goes.
async fn the_nightmare_manifests(realm: &Arc<Realm>) {
    const QUEST: u32 = 8736;
    const REMULOS: u32 = 11832;
    const ERANIKUS: u32 = 15491;
    const FOES: [u32; 1] = [15629];
    let (mut c, g, map) = hero(realm, "DREAMER", Race::NightElf, Class::Druid, 60).await;
    // Neutral with the Brood of Nozdormu, as the quest asks.
    map.query(move |m| crate::game::map::reputation::modify(m, g, 910, 45_000, false))
        .await;
    let map = take(realm, &mut c, REMULOS, QUEST, &[8735]).await;
    let remulos = map
        .query(|m| {
            m.escorts
                .iter()
                .find(|e| e.quest == QUEST)
                .map(|e| e.creature)
        })
        .await
        .flatten()
        .expect("the escort sets out");
    // Some fields are for the debug output alone.
    #[allow(dead_code)]
    #[derive(Debug, Clone, Default)]
    struct State {
        point: Option<u32>,
        eranikus: Option<(Guid, u32, bool, (u32, u32))>,
        at: Option<Vector3d>,
        motion: String,
        phase: Option<u32>,
        explored: bool,
        event: bool,
        escort: bool,
    }
    // One look, and the hurrying: Remulos and Eranikus fast, the player at
    // Remulos's side and unharmed, the phantasms put down as they come.
    let look = move |m: &mut Map, by: Duration, strike: bool| -> State {
        hardy(m, g);
        let at = m.creature(remulos).map(|c| c.position);
        if let Some(c) = m.creature_mut(remulos) {
            c.speeds.walk = 60.0;
            c.speeds.run = 60.0;
            let max = c.fields.get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH);
            c.fields
                .set_u32(crate::game::fields::UNIT_FIELD_HEALTH, max);
        }
        if let Some(at) = at {
            place(m, g, beside(at));
        }
        for f in alive_of(m, &FOES) {
            crate::game::map::combat::deal_damage(m, g, f, 1_000_000);
        }
        let eranikus = npc(m, ERANIKUS).map(|(e, _)| e);
        if let Some(e) = eranikus.and_then(|e| m.creature_mut(e)) {
            e.speeds.walk = 60.0;
            e.speeds.run = 60.0;
        }
        // While the escort walks, its fights are kept off it.
        let escorting = m.escorts.iter().any(|e| e.quest == QUEST);
        let fighting_eranikus = eranikus
            .and_then(|e| m.creature(e))
            .is_some_and(|c| c.combat.attacking);
        if escorting
            && !fighting_eranikus
            && m.creature(remulos).is_some_and(|c| c.combat.attacking)
        {
            crate::game::map::ai::forget(m, remulos);
            crate::game::map::combat::leave_combat(m, remulos);
        }
        if strike && let Some(e) = eranikus {
            let (hp, max) = health(m, e);
            if hp > max / 5 {
                crate::game::map::combat::deal_damage(m, g, e, (max / 8).max(1));
            }
        }
        skip(m, by);
        let log = m.players[&g].journal.log.iter().find(|l| l.quest == QUEST);
        State {
            point: m
                .escorts
                .iter()
                .find(|e| e.quest == QUEST)
                .and_then(|e| e.point()),
            eranikus: eranikus.and_then(|e| m.creature(e)).map(|c| {
                (
                    c.guid,
                    c.fields.get_u32(crate::game::fields::UNIT_FIELD_FLAGS),
                    c.combat.attacking,
                    health(m, c.guid),
                )
            }),
            at: eranikus.and_then(|e| m.creature(e)).map(|c| c.position),
            motion: eranikus
                .and_then(|e| m.creature(e))
                .map(|c| format!("{:?}", c.motion).chars().take(60).collect())
                .unwrap_or_default(),
            phase: eranikus
                .and_then(|e| crate::game::map::event_ai::scripted(m, e).map(|s| s.phase)),
            explored: log.is_some_and(|l| l.explored),
            event: crate::game::map::map_events::active(m, QUEST),
            escort: escorting,
        }
    };
    // To the shrine, through the waves, until Eranikus lands and may be
    // fought.
    let mut s = State::default();
    let mut landed = false;
    let mut hovering = false;
    for i in 0..3000 {
        s = map
            .query(move |m| look(m, Duration::from_secs(2), false))
            .await
            .unwrap();
        if debug() && i % 25 == 0 {
            eprintln!("nightmare: {s:?}");
        }
        let unreachable = s
            .eranikus
            .map(|(_, f, _, _)| f & UNIT_FLAG_NOT_ATTACKABLE != 0);
        hovering |= unreachable == Some(true);
        if hovering && unreachable == Some(false) {
            landed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(landed, "Eranikus lands to fight: {s:?}");
    assert!(s.event, "the quest's map event under way: {s:?}");
    // Beaten to a fifth, he is redeemed; Remulos credits the quest.
    for i in 0..3000 {
        // Struck down to a fifth at once, before anything else can come
        // of the fight (his leash, his evade); then the redemption hurried.
        let by = Duration::from_secs(if s.phase == Some(1) { 2 } else { 0 });
        s = map.query(move |m| look(m, by, true)).await.unwrap();
        if debug() && i % 25 == 0 {
            eprintln!("nightmare fight: {s:?}");
        }
        if s.explored {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(s.explored, "credited through the map event: {s:?}");
    assert!(
        until(&map, move |m| done(m, g, QUEST)).await,
        "{QUEST} done"
    );
    turn_in(realm, &mut c, REMULOS, QUEST).await;
}

/// The Nightmare Manifests, walked through on the real world.
#[tokio::test]
async fn the_nightmare_manifests_is_done_end_to_end() {
    if !have_data() {
        return;
    }
    let realm = real_realm();
    the_nightmare_manifests(&realm).await;
}

/// An escort runs under its quest's map event, as Thistle Tea's do: one
/// that joins the event goes with it when the escort fails (as Eranikus
/// and his helpers do), and a script that ends it well leaves nothing to
/// fail the escort after, whatever becomes of the escortee.
#[tokio::test]
async fn an_escort_runs_under_its_quests_map_event() {
    const QUEST: u32 = 4121;
    const HELPER: u32 = 9538;
    const CLEANUP: u32 = 5000;
    let mut d = GameData::empty();
    escortee(&mut d, GRARK, QUEST, HUMAN_START, 1..=45);
    d.creature_templates
        .insert(HELPER, template(HELPER, "Helper"));
    d.scripts.generic.insert(
        CLEANUP,
        vec![crate::game::data::scripts::ScriptStep {
            command: 18,
            ..Default::default()
        }],
    );
    let realm = harness::realm_with(d);
    let a = player(&realm, "Alice").await;
    let g = a.guid;
    let map = realm.registry.online(g).unwrap().map();
    let step =
        |command: u32, success: u32, failure_script: i32| crate::game::data::scripts::ScriptStep {
            command,
            datalong: [QUEST, success, 0, 0],
            dataint: [0, 0, 0, failure_script],
            ..Default::default()
        };
    // Failed: the escortee killed, its helper goes with the event.
    let grark = set_out(&map, g, GRARK, QUEST).await;
    let joined = step(63, 0, CLEANUP as i32);
    let helper = map
        .query(move |m| {
            assert!(
                crate::game::map::map_events::active(m, QUEST),
                "the quest's event under way"
            );
            assert_eq!(
                crate::game::map::map_events::target_of(m, 23, QUEST, 0),
                Some(g),
                "its target the player"
            );
            let at = m.players[&g].position();
            let h =
                crate::game::map::scripts::summon(m, Guid::zero(), HELPER, at, 0.0, None).unwrap();
            crate::game::map::scripts::execute(m, &joined, h, g);
            crate::game::map::combat::deal_damage(m, g, grark, 1_000_000);
            h
        })
        .await
        .unwrap();
    let mut now = going(&map, g, grark, QUEST).await;
    for _ in 0..200 {
        if now.over {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        now = going(&map, g, grark, QUEST).await;
    }
    assert!(now.over && now.failed, "failed: {now:?}");
    let gone = map
        .query(move |m| {
            m.creature(helper).is_none() && !crate::game::map::map_events::active(m, QUEST)
        })
        .await
        .unwrap();
    assert!(gone, "the event over, and its helper with it");
    // Ended well by a script: the escortee may go, and nothing fails.
    map.query(move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .journal
            .log
            .retain(|l| l.quest != QUEST);
    })
    .await;
    let grark = set_out(&map, g, GRARK, QUEST).await;
    let ended = step(62, 1, 0);
    map.query(move |m| {
        crate::game::map::scripts::execute(m, &ended, grark, g);
        crate::game::map::scripts::despawn(m, grark);
    })
    .await;
    let mut now = going(&map, g, grark, QUEST).await;
    for _ in 0..200 {
        if now.over {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        now = going(&map, g, grark, QUEST).await;
    }
    assert!(now.over && !now.failed, "over, not failed: {now:?}");
}

fn wanted(name: &str) -> bool {
    let yes = std::env::var("EVENTS_ONLY").map_or(true, |v| v.split(',').any(|w| w == name));
    if yes && debug() {
        eprintln!("event quests: {name}");
    }
    yes
}

/// `c` uses the item `item` from their bags on the object `object`.
async fn use_item_on_object(c: &mut Client, map: &MapHandle, item: u32, object: Guid) {
    let g = c.guid;
    let (bag, slot) = map
        .query(move |m| {
            let p = m.players.get_mut(&g).unwrap();
            p.caster.gcd_until = None;
            p.caster.cooldowns.clear();
            p.caster.category_cooldowns.clear();
            p.items
                .iter()
                .find(|i| i.entry == item)
                .map(|i| (i.bag, i.slot))
        })
        .await
        .flatten()
        .unwrap_or_else(|| panic!("item {item} in the bags"));
    c.send(CMSG_USE_ITEM {
        bag_index: bag,
        bag_slot: slot,
        spell_index: 0,
        targets: SpellCastTargets {
            target_flags: SpellCastTargets_SpellCastTargetFlags::empty().set_gameobject(
                SpellCastTargets_SpellCastTargetFlags_Gameobject::Gameobject { gameobject: object },
            ),
        },
    })
    .await;
}

/// The cast under way ends now.
fn finish_cast(m: &mut Map, g: Guid) {
    if let Some(c) = m
        .players
        .get_mut(&g)
        .and_then(|p| p.caster.casting.as_mut())
    {
        c.ends = m.now;
    }
    crate::game::map::spells::update(m);
}

/// Those who fight any of `protected` are put down, and `protected` kept
/// whole.
fn defend(m: &mut Map, protected: &[Guid]) {
    let foes: Vec<Guid> = m
        .creatures
        .iter()
        .filter(|(_, c)| c.alive() && c.combat.attacking && protected.contains(&c.combat.victim))
        .map(|(_, c)| c.guid)
        .collect();
    let by = protected
        .iter()
        .copied()
        .find(|g| guid::is_player(*g))
        .unwrap_or_default();
    for f in foes {
        crate::game::map::combat::deal_damage(m, by, f, 1_000_000);
    }
    for g in protected {
        if let Some(c) = m.creature_mut(*g) {
            let max = c.fields.get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH);
            c.fields
                .set_u32(crate::game::fields::UNIT_FIELD_HEALTH, max);
        }
    }
    if let Some(p) = protected.iter().find(|g| guid::is_player(**g)) {
        hardy(m, *p);
    }
}

/// Freedom for All Creatures (2969): Kindal Moonweaver follows the player
/// to the Grimtotem camp; the Bamboo Cage Key, used on the cage, frees the
/// sprite darters, and six of them getting away does it.
async fn freedom_for_all_creatures(realm: &Arc<Realm>) {
    const QUEST: u32 = 2969;
    const KINDAL: u32 = 7956;
    const SPRITE: u32 = 7997;
    const KEY: u32 = 12301;
    const CAGE: u32 = 143979;
    let (mut c, g, _) = hero(realm, "FREER", Race::NightElf, Class::Hunter, 47).await;
    let map = take(realm, &mut c, KINDAL, QUEST, &[]).await;
    map.query(move |m| hardy(m, g)).await;
    assert!(
        until(&map, move |m| m.players[&g]
            .items
            .iter()
            .any(|i| i.entry == KEY))
        .await,
        "the key given"
    );
    let kindal = map
        .query(|m| npc(m, KINDAL).map(|(k, _)| k))
        .await
        .flatten()
        .unwrap();
    // To the cage, Kindal following.
    let pen = Vector3d {
        x: -4533.17,
        y: 808.743,
        z: 60.29,
    };
    let cage = map
        .query(move |m| {
            m.gameobjects
                .iter()
                .filter(|(_, o)| o.entry == CAGE)
                .map(|(_, o)| (o.guid, o.position))
                .min_by(|a, b| {
                    crate::game::map::combat::dist(a.1, pen)
                        .total_cmp(&crate::game::map::combat::dist(b.1, pen))
                })
        })
        .await
        .flatten()
        .expect("the cage");
    put(&map, &mut c, beside(cage.1)).await;
    let near = until_mut(&map, move |m| {
        defend(m, &[g, kindal]);
        if let Some(k) = m.creature_mut(kindal) {
            k.speeds.walk = 20.0;
            k.speeds.run = 20.0;
        }
        skip(m, Duration::from_millis(200));
        m.creature(kindal).is_some_and(|k| {
            crate::game::map::combat::dist(k.position, m.players[&g].position()) < 6.0
        })
    })
    .await;
    assert!(near, "Kindal follows to the cage");
    // The key on the cage.
    use_item_on_object(&mut c, &map, KEY, cage.0).await;
    let opened = until_mut(&map, move |m| {
        defend(m, &[g, kindal]);
        if m.players[&g].caster.casting.is_some() {
            finish_cast(m, g);
        }
        m.gameobject(cage.0)
            .is_some_and(|o| o.fields.get_u32(crate::game::fields::GAMEOBJECT_STATE) == 0)
    })
    .await;
    assert!(opened, "the key opens the cage");
    // The sprites fly free, kept from harm.
    let mut freed = false;
    for i in 0..1500 {
        let (sprites, quest_done, state) = map
            .query(move |m| {
                let sprites = alive_of(m, &[SPRITE]);
                for s in &sprites {
                    if let Some(c) = m.creature_mut(*s) {
                        c.speeds.walk = 40.0;
                        c.speeds.run = 40.0;
                    }
                }
                let mut protected = vec![g, kindal];
                protected.extend(sprites.iter().copied());
                defend(m, &protected);
                skip(m, Duration::from_secs(1));
                let log = m.players[&g]
                    .journal
                    .log
                    .iter()
                    .find(|l| l.quest == QUEST)
                    .map(|l| (l.explored, l.failed));
                (
                    sprites.len(),
                    done(m, g, QUEST),
                    (
                        crate::game::map::map_events::active(m, QUEST),
                        crate::game::map::map_events::data(m, SPRITE, 0),
                        crate::game::map::map_events::data(m, SPRITE, 1),
                        log,
                    ),
                )
            })
            .await
            .unwrap();
        if debug() && i % 50 == 0 {
            eprintln!("freedom: {sprites} sprites, done {quest_done}, {state:?}");
        }
        if quest_done {
            freed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(freed, "{QUEST} done as the sprites get away");
    turn_in(realm, &mut c, KINDAL, QUEST).await;
}

/// The Gizelton Caravan: Cork Gizelton's caravan sets out on its loop
/// through Desolace; Rigger offers Gizelton Caravan (5943) at the first
/// stop and Cork Bodyguard for Hire (5821) at the second. Each taken, the
/// caravan moves on, holds out against three ambushes, and the quest is
/// done at the end of the leg.
async fn the_gizelton_caravan(realm: &Arc<Realm>) {
    const CORK: u32 = 11625;
    const RIGGER: u32 = 11626;
    const KODO: u32 = 11564;
    const SMEED: u32 = 11596;
    let (mut c, g, _) = hero(realm, "GUARD", Race::Orc, Class::Warrior, 40).await;
    let (map, _, _) = visit(realm, &mut c, CORK).await;
    let cork = map
        .query(|m| npc(m, CORK).map(|(k, _)| k))
        .await
        .flatten()
        .unwrap();
    // The caravan, along its way, the player beside Cork; whatever falls
    // on it put down.
    let caravan = move |m: &mut Map| -> Vec<Guid> {
        let mut v = vec![cork];
        v.extend(alive_of(m, &[RIGGER, KODO]));
        v
    };
    let ride = move |m: &mut Map, by: Duration| {
        let members = caravan(m);
        for c in &members {
            if let Some(c) = m.creature_mut(*c) {
                c.speeds.walk = 60.0;
                c.speeds.run = 60.0;
            }
        }
        if let Some(at) = m.creature(cork).map(|c| c.position) {
            place(m, g, beside(at));
        }
        let mut protected = members;
        protected.push(g);
        defend(m, &protected);
        skip(m, by);
    };
    // The caravan stops, and Rigger, or Cork, offers a quest.
    let offering = move |m: &Map, giver: u32| {
        let held = crate::game::map::event_ai::scripted(m, cork).is_some_and(|s| s.hold.is_some());
        held && npc(m, giver).is_some_and(|(r, _)| {
            m.creature(r)
                .is_some_and(|c| c.fields.get_u32(crate::game::fields::UNIT_NPC_FLAGS) & 0x2 != 0)
        })
    };
    for (quest, giver) in [(5943, RIGGER), (5821, CORK)] {
        let mut offered = false;
        for i in 0..3000 {
            let (yes, at) = map
                .query(move |m| {
                    ride(m, Duration::from_secs(4));
                    (offering(m, giver), m.creature(cork).map(|c| c.position))
                })
                .await
                .unwrap();
            if debug() && i % 100 == 0 {
                eprintln!("caravan to {quest}: Cork at {at:?}");
            }
            if yes {
                offered = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(offered, "{giver} offers {quest}");
        let who = map
            .query(move |m| npc(m, giver).map(|(r, _)| r))
            .await
            .flatten()
            .unwrap();
        map.query(move |m| {
            if let Some(at) = m.creature(who).map(|c| c.position) {
                place(m, g, beside(at));
            }
            crate::game::map::quests::accept(m, g, who, quest);
        })
        .await;
        assert!(
            until(&map, move |m| logged(m, g, quest)).await,
            "{quest} taken"
        );
        let mut credited = false;
        for i in 0..3000 {
            let (yes, at, event) = map
                .query(move |m| {
                    ride(m, Duration::from_secs(4));
                    (
                        done(m, g, quest),
                        m.creature(cork).map(|c| c.position),
                        crate::game::map::map_events::active(m, quest),
                    )
                })
                .await
                .unwrap();
            if debug() && i % 100 == 0 {
                eprintln!("caravan {quest}: Cork at {at:?}, event {event}");
            }
            if yes {
                credited = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(credited, "{quest} done at the end of the leg");
    }
    turn_in(realm, &mut c, SMEED, 5943).await;
    turn_in(realm, &mut c, SMEED, 5821).await;
}

/// The id of the option in `npc`'s menu that reads `text`.
async fn option(c: &mut Client, npc: Guid, text: &'static str) -> u32 {
    c.send(CMSG_GOSSIP_HELLO { guid: npc }).await;
    c.wait_for("the menu", |m| match m {
        ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) if g.guid == npc => g
            .gossips
            .iter()
            .find(|o| o.message.contains(text))
            .map(|o| o.id),
        _ => None,
    })
    .await
}

async fn choose(c: &Client, npc: Guid, option: u32) {
    c.send(CMSG_GOSSIP_SELECT_OPTION {
        guid: npc,
        gossip_list_id: option,
        unknown: None,
    })
    .await;
}

/// The Great Masquerade (6403): Squire Rowe signals Reginald Windsor, who
/// rides in and takes the rendezvous; Windsor talks his way over the bridge
/// and through Stormwind to the keep, waits for the player's word, and
/// unmasks Lady Prestor as Onyxia; her elite guards put down, Bolvar
/// credits the quest.
async fn the_great_masquerade(realm: &Arc<Realm>) {
    const RENDEZVOUS: u32 = 6402;
    const QUEST: u32 = 6403;
    const ROWE: u32 = 17804;
    const WINDSOR: u32 = 12580;
    const BOLVAR: u32 = 1748;
    const ELITE: u32 = 12739;
    let (mut c, g, map) = hero(realm, "UNMASKER", Race::Human, Class::Paladin, 60).await;
    map.query(move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.journal
            .rewarded
            .extend([RENDEZVOUS, 6401, 6391, 6392, 6261, 6285]);
        hardy(m, g);
    })
    .await;
    // Rowe sends the signal.
    let (map, rowe, _) = visit(realm, &mut c, ROWE).await;
    let id = option(&mut c, rowe, "Let Marshal Windsor know").await;
    choose(&c, rowe, id).await;
    // Windsor rides in to the gate and greets the player; they keep near
    // him.
    let near_windsor = move |m: &mut Map, by: Duration| -> Option<Guid> {
        hardy(m, g);
        let w = npc(m, WINDSOR).map(|(w, _)| w);
        if let Some(w) = w {
            if let Some(c) = m.creature_mut(w) {
                c.speeds.walk = 20.0;
                c.speeds.run = 20.0;
                let max = c.fields.get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH);
                c.fields
                    .set_u32(crate::game::fields::UNIT_FIELD_HEALTH, max);
            }
            if let Some(at) = m.creature(w).map(|c| c.position) {
                place(m, g, beside(at));
            }
        }
        skip(m, by);
        w
    };
    let questgiver = move |m: &Map, entry: u32| {
        npc(m, entry).is_some_and(|(w, _)| {
            m.creature(w)
                .is_some_and(|c| c.fields.get_u32(crate::game::fields::UNIT_NPC_FLAGS) & 0x2 != 0)
        })
    };
    let arrived = until_mut(&map, move |m| {
        // Rowe's run to the road, and Windsor's ride.
        if let Some((r, _)) = npc(m, ROWE)
            && let Some(c) = m.creature_mut(r)
        {
            c.speeds.walk = 20.0;
            c.speeds.run = 20.0;
        }
        near_windsor(m, Duration::from_millis(500));
        questgiver(m, WINDSOR)
    })
    .await;
    assert!(arrived, "Windsor rides in and greets");
    let windsor = map
        .query(|m| npc(m, WINDSOR).map(|(w, _)| w))
        .await
        .flatten()
        .unwrap();
    map.query(move |m| crate::game::map::quests::accept(m, g, windsor, QUEST))
        .await;
    assert!(
        until(&map, move |m| logged(m, g, QUEST)).await,
        "{QUEST} taken"
    );
    // Through the city to the keep's steps, where he waits for the word.
    let mut waiting = false;
    for i in 0..3000 {
        let (gossip, at) = map
            .query(move |m| {
                near_windsor(m, Duration::from_secs(2));
                (
                    m.creature(windsor).is_some_and(|c| {
                        c.fields.get_u32(crate::game::fields::UNIT_NPC_FLAGS) & 0x1 != 0
                    }),
                    m.creature(windsor).map(|c| c.position),
                )
            })
            .await
            .unwrap();
        if debug() && i % 100 == 0 {
            eprintln!("masquerade: Windsor at {at:?}");
        }
        if gossip {
            waiting = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(waiting, "Windsor waits for the player's word");
    let id = option(&mut c, windsor, "I am ready, as are my forces").await;
    choose(&c, windsor, id).await;
    // The throne room: Onyxia unmasked, her guards fall on Bolvar and are
    // put down.
    let mut credited = false;
    for i in 0..3000 {
        let (yes, elites, event) = map
            .query(move |m| {
                near_windsor(m, Duration::from_secs(2));
                let bolvar = npc(m, BOLVAR).map(|(b, _)| b);
                let mut protected = vec![g];
                protected.extend(bolvar);
                protected.push(windsor);
                defend(m, &protected);
                let elites = alive_of(m, &[ELITE]);
                (
                    done(m, g, QUEST),
                    elites.len(),
                    crate::game::map::map_events::active(m, QUEST),
                )
            })
            .await
            .unwrap();
        if debug() && i % 100 == 0 {
            eprintln!("masquerade throne room: {elites} elite guards, event {event}");
        }
        if yes {
            credited = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(credited, "{QUEST} credited as the elite guards fall");
    turn_in(realm, &mut c, BOLVAR, QUEST).await;
}

/// The Balance of Light and Shadow (7622): Eris Havenfire's peasants flee
/// the burning village for her light under the Scourge's arrows; the
/// priest keeps them whole and cures their plague, and fifty saved do it.
async fn the_balance_of_light_and_shadow(realm: &Arc<Realm>) {
    const QUEST: u32 = 7622;
    const ERIS: u32 = 14494;
    const PEASANTS: [u32; 2] = [14484, 14485];
    const FOES: [u32; 2] = [14486, 14489];
    const SEETHING_PLAGUE: u32 = 23072;
    let (mut c, g, _) = hero(realm, "BLESSED", Race::Human, Class::Priest, 60).await;
    let map = take(realm, &mut c, ERIS, QUEST, &[7621]).await;
    let field = map
        .query(|m| npc(m, ERIS).map(|(_, at)| at))
        .await
        .flatten()
        .unwrap();
    let mut saved = false;
    for i in 0..4000 {
        let (yes, state) = map
            .query(move |m| {
                place(m, g, field);
                hardy(m, g);
                // The Scourge put down as it comes; the peasants kept whole,
                // cured, and quick on their feet.
                for f in alive_of(m, &FOES) {
                    crate::game::map::combat::deal_damage(m, g, f, 1_000_000);
                }
                for p in alive_of(m, &PEASANTS) {
                    crate::game::map::auras::remove_spell(m, p, SEETHING_PLAGUE, None);
                    if let Some(c) = m.creature_mut(p) {
                        c.speeds.walk = 20.0;
                        c.speeds.run = 20.0;
                        let max = c.fields.get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH);
                        c.fields
                            .set_u32(crate::game::fields::UNIT_FIELD_HEALTH, max);
                    }
                }
                skip(m, Duration::from_secs(1));
                (
                    done(m, g, QUEST),
                    (
                        alive_of(m, &PEASANTS).len(),
                        crate::game::map::map_events::data(m, QUEST, 0),
                        crate::game::map::map_events::data(m, QUEST, 1),
                    ),
                )
            })
            .await
            .unwrap();
        if debug() && i % 100 == 0 {
            eprintln!("balance: peasants, saved, died {state:?}");
        }
        if yes {
            saved = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(saved, "{QUEST} done, fifty saved");
    turn_in(realm, &mut c, ERIS, QUEST).await;
}

/// The Gizelton Caravan's two quests, walked on the real world (on its
/// own, for its long road).
#[tokio::test]
async fn the_gizelton_caravan_is_done_end_to_end() {
    if !have_data() {
        return;
    }
    let realm = real_realm();
    the_gizelton_caravan(&realm).await;
}

/// The event quests Thistle Tea builds in code, walked through on the real
/// world: Freedom for All Creatures, The Great Masquerade, The Balance of
/// Light and Shadow (and the Gizelton Caravan, on its own).
#[tokio::test]
async fn the_imported_event_quests_are_done_end_to_end() {
    if !have_data() {
        return;
    }
    let realm = real_realm();
    if wanted("freedom") {
        freedom_for_all_creatures(&realm).await;
    }
    if wanted("masquerade") {
        the_great_masquerade(&realm).await;
    }
    if wanted("balance") {
        the_balance_of_light_and_shadow(&realm).await;
    }
}

/// A key used on the door it opens opens it, and is used up: the door is
/// the lock's target, not the key, which is the item used and so the
/// spell's item too (vmangos's `EffectOpenLock` takes the object first);
/// and the key used up by the cast is the lock's key all the same.
#[tokio::test]
async fn a_key_used_on_its_door_opens_it() {
    const KEY: u32 = 12301;
    const OPENING: u32 = 3366;
    const DOOR: u32 = 143979;
    let mut d = GameData::empty();
    let mut door = GameObjectTemplate {
        entry: DOOR,
        kind: 0,
        display_id: 411,
        name: "Cage Door".into(),
        size: 1.0,
        ..Default::default()
    };
    door.data[1] = 859;
    d.gameobject_templates.insert(DOOR, door);
    d.gameobject_spawns.insert(
        0,
        vec![GameObjectSpawn {
            guid: 50029,
            entry: DOOR,
            map: 0,
            x: HUMAN_START.0 + 2.0,
            y: HUMAN_START.1,
            z: HUMAN_START.2,
            rotation: [0.0, 0.0, 0.0, 1.0],
            state: 1,
            ..Default::default()
        }],
    );
    d.crafts.locks.insert(
        859,
        vec![crate::game::data::crafts::LockKey {
            kind: 1,
            index: KEY,
            skill: 0,
        }],
    );
    let mut opening = crate::game::data::spells::SpellTemplate {
        id: OPENING,
        name: "Opening".into(),
        range_index: 1,
        equipped_item_class: -1,
        ..Default::default()
    };
    opening.effects[0] = crate::game::data::spells::SpellEffect {
        effect: 59,
        target_a: 23,
        ..Default::default()
    };
    d.spells.spells.insert(OPENING, opening);
    let mut key = ItemTemplate {
        entry: KEY,
        class: 13,
        name: "Bamboo Cage Key".into(),
        quality: 1,
        stackable: 1,
        ..Default::default()
    };
    // Used up by its use, as the Bamboo Cage Key is.
    key.spells[0] = crate::game::data::items::ItemSpell {
        id: OPENING,
        charges: -1,
        ..Default::default()
    };
    d.items.insert(KEY, key);
    let realm = harness::realm_with(d);
    let mut a = player(&realm, "Alice").await;
    let g = a.guid;
    let map = realm.registry.online(g).unwrap().map();
    let door = guid::game_object(DOOR, 50029);
    a.sees(door).await;
    map.query(move |m| {
        crate::game::map::inventory::add_item(m, g, KEY, 1, false).unwrap();
    })
    .await;
    use_item_on_object(&mut a, &map, KEY, door).await;
    let r = a.wait_frame("SMSG_CAST_RESULT", 0x130, |_| true).await;
    assert_eq!(r[4], 0, "cast: {r:?}");
    let open = until_mut(&map, move |m| {
        if m.players[&g].caster.casting.is_some() {
            finish_cast(m, g);
        }
        m.gameobject(door)
            .is_some_and(|o| o.fields.get_u32(crate::game::fields::GAMEOBJECT_STATE) == 0)
    })
    .await;
    assert!(open, "the door opened");
    let kept = map
        .query(move |m| m.players[&g].count_of(KEY))
        .await
        .unwrap();
    assert_eq!(kept, 0, "the key used up");
}

/// The event quests no source raises, in Thistle Tea either (vmangos does
/// them in C++): The Glowing Shard, Jail Break!, Master Ryson's All Seeing
/// Eye, A Pawn on the Eternal Board.
const EVENTLESS: [u32; 5] = [3366, 4322, 6847, 6848, 8519];

/// What raises each event quest's event (`SpecialFlags` 0x2), as far as
/// something in the world reaches it: an area trigger, a script that
/// credits it and what starts that script, a spell that completes it and
/// what casts that spell, an escort or follower. By quest, each source
/// named.
fn event_sources(data: &GameData) -> std::collections::BTreeMap<u32, Vec<String>> {
    use crate::game::data::scripts::ScriptTable as T;
    use crate::game::map::scripts::{C_HOLD_WAYPOINTS, C_LINKED_FEIGN_DEATH};
    let s = &data.scripts;
    let tables = [
        (T::CreatureAi, &s.ai_scripts),
        (T::Generic, &s.generic),
        (T::Event, &s.event),
        (T::Spell, &s.spell),
        (T::QuestStart, &s.quest_start),
        (T::QuestEnd, &s.quest_end),
        (T::Gossip, &s.gossip),
        (T::Movement, &s.movement),
        (T::GameObject, &s.gameobject),
        (T::AreaTrigger, &s.areatrigger),
        (T::CreatureSpells, &s.creature_spells),
    ];
    let steps = || {
        tables.iter().flat_map(|(t, m)| {
            m.iter()
                .flat_map(move |(id, v)| v.iter().map(move |st| (*t, *id, st)))
        })
    };
    // Generic scripts something starts.
    let mut generic: ahash::AHashSet<u32> = s
        .areatrigger_script
        .values()
        .chain(s.object_hello.values())
        .chain(s.object_hello_entry.values())
        .copied()
        .collect();
    for (_, _, st) in steps() {
        let int = |i: usize| st.dataint[i].max(0) as u32;
        match st.command {
            39 | 90 => generic.extend(st.datalong),
            68 | 92 => {
                generic.insert(st.datalong[0]);
            }
            10 => {
                generic.insert(int(1));
            }
            61 | 63 | 69 => generic.extend([int(1), int(3)]),
            c if c == C_HOLD_WAYPOINTS || c == C_LINKED_FEIGN_DEATH => {
                generic.extend([int(0), int(1)])
            }
            _ => {}
        }
    }
    // Events something sends: a spell's, a goober's or a camera's.
    let mut events: ahash::AHashSet<u32> = data
        .spells
        .spells
        .values()
        .flat_map(|sp| sp.effects.iter())
        .filter(|e| e.effect == 61 && e.misc_value > 0)
        .map(|e| e.misc_value as u32)
        .collect();
    for t in data.gameobject_templates.values() {
        match t.kind {
            10 => {
                events.insert(t.data[2]);
            }
            13 => {
                events.insert(t.data[1]);
            }
            _ => {}
        }
    }
    // EventAI scripts some creature's events run.
    let ai: ahash::AHashSet<u32> = s
        .ai_events
        .values()
        .flatten()
        .flat_map(|e| e.actions)
        .collect();
    // Waypoint scripts some path runs, the creature scripts' routes too.
    let moves: ahash::AHashSet<u32> = data
        .waypoints
        .values()
        .chain(data.waypoints_by_entry.values())
        .chain(data.waypoints_special.values())
        .chain(s.routes.values())
        .flatten()
        .map(|w| w.script_id)
        .collect();
    let object_spawns: ahash::AHashSet<u32> = data
        .gameobject_spawns
        .values()
        .flatten()
        .map(|o| o.guid)
        .collect();
    let gossip: ahash::AHashSet<u32> = s
        .gossip_option_script
        .iter()
        .filter(|((menu, option), _)| {
            data.npc
                .gossip_menus
                .get(menu)
                .is_some_and(|m| m.options.iter().any(|o| o.id == *option))
        })
        .map(|(_, id)| *id)
        .chain(s.gossip_menu_script.values().copied())
        .chain(
            data.npc
                .gossip_menus
                .values()
                .flat_map(|m| m.options.iter().map(|o| o.script)),
        )
        .collect();
    // Spells something casts: an item, a creature's list or script, an
    // object, another spell, or this server's own code (the taming rods,
    // the battlegrounds' team credit, the heroes' tributes).
    let mut castable: ahash::AHashSet<u32> = data
        .items
        .values()
        .flat_map(|i| i.spells.iter().map(|s| s.id))
        .chain(
            data.spells
                .creature_spells
                .values()
                .flatten()
                .map(|c| c.spell),
        )
        .chain(
            data.spells
                .spells
                .values()
                .flat_map(|sp| sp.effects.iter().map(|e| e.trigger_spell)),
        )
        .collect();
    for (_, _, st) in steps() {
        if st.command == 15 {
            castable.insert(st.datalong[0]);
        }
    }
    for t in data.gameobject_templates.values() {
        match t.kind {
            22 => {
                castable.insert(t.data[0]);
            }
            6 => {
                castable.insert(t.data[3]);
            }
            _ => {}
        }
    }
    castable.extend((19597..=19597).chain(19676..=19686));
    castable.extend([23658, 24061, 24064]);
    castable.extend(24101..=24108);
    castable.remove(&0);
    let reached = |t: T, id: u32| match t {
        T::Generic => generic.contains(&id),
        T::Event => events.contains(&id),
        T::CreatureAi => ai.contains(&id),
        T::Movement => moves.contains(&id),
        T::GameObject => object_spawns.contains(&id),
        T::Spell => castable.contains(&id),
        T::QuestStart => s.quest_start_script.values().any(|v| *v == id),
        T::QuestEnd => s.quest_complete_script.values().any(|v| *v == id),
        T::Gossip => gossip.contains(&id),
        T::AreaTrigger => data.area_triggers.contains_key(&id),
        T::CreatureSpells => data
            .spells
            .creature_spells
            .values()
            .flatten()
            .any(|c| c.script == id),
    };
    let mut out: std::collections::BTreeMap<u32, Vec<String>> = data
        .npc
        .quests
        .values()
        .filter(|q| q.special_flags & 2 != 0)
        .map(|q| (q.id, Vec::new()))
        .collect();
    for (trigger, quest) in &data.npc.area_trigger_quests {
        if data.area_triggers.contains_key(trigger)
            && let Some(v) = out.get_mut(quest)
        {
            v.push(format!("trigger {trigger}"));
        }
    }
    for (t, id, st) in steps() {
        if st.command == 7
            && reached(t, id)
            && let Some(v) = out.get_mut(&st.datalong[0])
        {
            v.push(format!("{t:?} script {id}"));
        }
    }
    for sp in data.spells.spells.values() {
        for e in &sp.effects {
            if e.effect == 16
                && castable.contains(&sp.id)
                && let Some(v) = out.get_mut(&(e.misc_value.max(0) as u32))
            {
                v.push(format!("spell {}", sp.id));
            }
        }
    }
    for e in crate::game::map::escorts::catalog::ESCORTS {
        let path: Vec<u32> = match e.path {
            Some(p) => (0..p.len() as u32).collect(),
            None => data
                .world
                .script_waypoints
                .get(&e.entry)
                .map(|v| v.iter().map(|p| p.point).collect())
                .unwrap_or_default(),
        };
        let credits = match e.credit_point {
            Some(c) => path.contains(&c),
            // Credited by a script of its own (Eranikus's, Slim Jahn's),
            // counted above.
            None => false,
        };
        if credits && let Some(v) = out.get_mut(&e.quest) {
            v.push(format!("escort of {}", e.entry));
        }
    }
    for f in crate::game::map::escorts::catalog::FOLLOWERS {
        if f.credit
            && let Some(v) = out.get_mut(&f.quest)
        {
            v.push(format!("follower {}", f.entry));
        }
    }
    for v in out.values_mut() {
        v.sort();
        v.dedup();
    }
    out
}

/// Every event quest on the real world has its event's source in reach,
/// but the five no one raises: its area trigger, the script that credits
/// it and what starts that, the spell and what casts it, its escort.
#[test]
fn every_real_event_quest_has_its_source() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    let sources = event_sources(d);
    assert!(sources.len() > 200, "{} event quests", sources.len());
    let missing: Vec<u32> = sources
        .iter()
        .filter(|(q, v)| v.is_empty() && !EVENTLESS.contains(q))
        .map(|(q, _)| *q)
        .collect();
    if debug() {
        for (q, v) in &sources {
            eprintln!("event quest {q}: {v:?}");
        }
    }
    assert!(missing.is_empty(), "with no source in reach: {missing:?}");
    for q in EVENTLESS {
        assert!(
            sources.get(&q).is_some_and(|v| v.is_empty()),
            "{q} raised after all: {:?}",
            sources.get(&q)
        );
    }
}

/// The sides to try a hero of, for a quest from creature `entry`: the
/// Horde first for one of the Horde's, else the Alliance first.
fn sides_for(data: &GameData, entry: u32) -> [bool; 2] {
    let horde = data
        .creature_template(entry)
        .and_then(|t| data.faction_templates.get(&t.faction))
        .is_some_and(|f| {
            let mask = f.ours | f.friend_mask;
            mask & 0x4 != 0 && mask & 0x2 == 0
        });
    [horde, !horde]
}

/// `g` fit to take `q`: the skill and the standing it asks for.
fn fit_for(m: &mut Map, g: Guid, q: &crate::game::data::npc::Quest) {
    if q.required_skill != 0
        && let Some(p) = m.players.get_mut(&g)
    {
        crate::game::map::skills::learn_skill(p, q.required_skill, 4);
        crate::game::map::skills::set_value(p, q.required_skill, q.required_skill_value.max(1));
    }
    let (faction, _) = q.required_min_rep;
    if faction != 0 {
        crate::game::map::reputation::modify(m, g, faction, 45_000, false);
    }
}

/// How many walks go at once.
const WALKERS: usize = 4;

/// `quest` in `g`'s log, as though taken.
fn logs(m: &mut Map, g: Guid, quest: u32) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    p.journal.log.push(crate::game::store::Logged {
        quest,
        ..Default::default()
    });
    crate::game::map::quests::refresh_log(p, &data);
}

fn explored(m: &Map, g: Guid, quest: u32) -> bool {
    m.players[&g]
        .journal
        .log
        .iter()
        .any(|l| l.quest == quest && l.explored)
}

/// A race and class who may take `q`: its class, else a warrior, of its
/// race if it names one, of the Alliance (`horde` false) or the Horde if
/// it can.
fn hero_for(q: &crate::game::data::npc::Quest, horde: bool) -> (Race, Class) {
    let classes: [(u32, Class, &[u32]); 9] = [
        (1, Class::Warrior, &[1, 2, 3, 4, 5, 6, 7, 8]),
        (2, Class::Paladin, &[1, 3]),
        (3, Class::Hunter, &[2, 3, 4, 6, 8]),
        (4, Class::Rogue, &[1, 2, 3, 4, 5, 7, 8]),
        (5, Class::Priest, &[1, 3, 4, 5, 8]),
        (7, Class::Shaman, &[2, 6, 8]),
        (8, Class::Mage, &[1, 5, 7, 8]),
        (9, Class::Warlock, &[1, 2, 5, 7]),
        (11, Class::Druid, &[4, 6]),
    ];
    let (_, class, races) = classes
        .iter()
        .find(|(id, _, _)| {
            q.required_classes == 0 && *id == 1 || q.required_classes & (1 << (id - 1)) != 0
        })
        .copied()
        .unwrap_or(classes[0]);
    let allowed = |r: &u32| q.required_races == 0 || q.required_races & (1 << (r - 1)) != 0;
    let side = |r: &u32| matches!(r, 2 | 5 | 6 | 8) == horde;
    let race = races
        .iter()
        .copied()
        .find(|r| allowed(r) && side(r))
        .or_else(|| races.iter().copied().find(allowed))
        .unwrap_or(races[0]);
    let race = match race {
        1 => Race::Human,
        2 => Race::Orc,
        3 => Race::Dwarf,
        4 => Race::NightElf,
        5 => Race::Undead,
        6 => Race::Tauren,
        7 => Race::Gnome,
        _ => Race::Troll,
    };
    (race, class)
}

/// An account (and character) name of letters alone, a new one each time.
fn walker_name(kind: &str) -> String {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let letters: String = n
        .to_string()
        .chars()
        .map(|d| (b'A' + d.to_digit(10).unwrap() as u8) as char)
        .collect();
    format!("{kind}{letters}")
}

/// A hero for `quest`, at its level, with it in their log.
async fn walker(
    realm: &Arc<Realm>,
    kind: &str,
    quest: u32,
    horde: bool,
) -> (Client, Guid, MapHandle) {
    let q = realm.data.npc.quests[&quest].clone();
    let (race, class) = hero_for(&q, horde);
    let level = (q.level.max(i32::from(q.min_level))).clamp(1, 60) as u8;
    let (c, g, map) = hero(realm, &walker_name(kind), race, class, level).await;
    map.query(move |m| {
        hardy(m, g);
        logs(m, g, quest);
    })
    .await;
    (c, g, map)
}

/// Until `quest` is explored for `g`, hurrying what waits on time, in
/// whatever map they are (a trigger may take them through a portal):
/// whether it was.
async fn credited(realm: &Arc<Realm>, g: Guid, quest: u32) -> bool {
    for _ in 0..150 {
        let Some(map) = realm.registry.online(g).map(|o| o.map()) else {
            return false;
        };
        let yes = map
            .query(move |m| {
                if !m.players.contains_key(&g) {
                    return false;
                }
                hardy(m, g);
                skip(m, Duration::from_secs(1));
                explored(m, g, quest)
            })
            .await
            .unwrap_or(false);
        if yes {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// An area trigger's quest: stepped into, explored.
async fn walk_trigger(realm: &Arc<Realm>, quest: u32, trigger: u32) -> Result<(), String> {
    let (mut c, g, _) = walker(realm, "Trig", quest, false).await;
    let t = realm.data.area_triggers[&trigger].clone();
    let map = travel(
        realm,
        &mut c,
        t.map,
        Vector3d {
            x: t.x,
            y: t.y,
            z: t.z,
        },
    )
    .await;
    c.send(CMSG_AREATRIGGER {
        trigger_id: trigger,
    })
    .await;
    // A portal: through it.
    if c.gets(Duration::from_millis(500), |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await
    .is_some()
    {
        c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
        c.sees(g).await;
    }
    let _ = map;
    if credited(realm, g, quest).await {
        Ok(())
    } else {
        Err(format!("trigger {trigger} stepped into, not explored"))
    }
}

/// The menus, and the option of each, that lead from `root` to `menu`.
fn gossip_path(data: &GameData, root: u32, menu: u32) -> Option<Vec<(u32, u32)>> {
    let mut seen = ahash::AHashSet::new();
    let mut queue = std::collections::VecDeque::from([(root, Vec::new())]);
    while let Some((at, path)) = queue.pop_front() {
        if at == menu {
            return Some(path);
        }
        if !seen.insert(at) {
            continue;
        }
        for o in data
            .npc
            .gossip_menus
            .get(&at)
            .map(|m| m.options.clone())
            .unwrap_or_default()
        {
            if o.action_menu > 0 {
                let mut p = path.clone();
                p.push((at, o.id));
                queue.push_back((o.action_menu as u32, p));
            }
        }
    }
    None
}

/// A gossip option's quest: the option chosen at a creature that has its
/// menu, explored.
async fn walk_gossip(realm: &Arc<Realm>, quest: u32, script: u32) -> Result<(), String> {
    let data = realm.data.clone();
    // The option: by the menus' script table, or a code-built menu's own.
    let mut option = data
        .scripts
        .gossip_option_script
        .iter()
        .find(|(_, s)| **s == script)
        .map(|((menu, option), _)| (*menu, *option));
    if option.is_none() {
        option = data.npc.gossip_menus.iter().find_map(|(menu, m)| {
            m.options
                .iter()
                .find(|o| o.script == script)
                .map(|o| (*menu, o.id))
        });
    }
    let Some((menu, option)) = option else {
        return Err(format!("no option runs gossip script {script}"));
    };
    // A creature in the world whose menu leads to it.
    let mut found = None;
    let mut entries: Vec<&CreatureTemplate> = data
        .creature_templates
        .values()
        .filter(|t| t.gossip_menu_id != 0)
        .collect();
    entries.sort_by_key(|t| t.entry);
    for t in entries {
        if let Some(path) = gossip_path(&data, t.gossip_menu_id, menu)
            && spawn_of(realm, t.entry).is_some()
        {
            found = Some((t.entry, path));
            break;
        }
    }
    let Some((entry, path)) = found else {
        return Err(format!("no creature in the world has menu {menu}"));
    };
    if debug() {
        let steps: Vec<String> = data
            .scripts
            .gossip
            .get(&script)
            .into_iter()
            .flatten()
            .map(|s| {
                format!(
                    "{}:{:?} t{} c{}",
                    s.command, s.datalong, s.target_type, s.condition
                )
            })
            .collect();
        let opt = data
            .npc
            .gossip_menus
            .get(&menu)
            .and_then(|m| m.options.iter().find(|o| o.id == option).cloned());
        eprintln!(
            "walk gossip {quest}: script {script} steps {steps:?}, option {:?} condition {:?}",
            opt.as_ref().map(|o| o.text.clone()),
            opt.map(|o| (
                o.condition,
                data.scripts.conditions.get(&o.condition).cloned()
            ))
        );
    }
    let mut last = String::new();
    for horde in sides_for(&data, entry) {
        match gossip_once(realm, quest, entry, &path, (menu, option), horde).await {
            Ok(()) => return Ok(()),
            Err(why) => last = why,
        }
    }
    Err(last)
}

/// [`walk_gossip`] by a hero of one side.
async fn gossip_once(
    realm: &Arc<Realm>,
    quest: u32,
    entry: u32,
    path: &[(u32, u32)],
    (menu, option): (u32, u32),
    horde: bool,
) -> Result<(), String> {
    let (mut c, g, _) = walker(realm, "Goss", quest, horde).await;
    let (map, who, _) = try_visit(realm, &mut c, entry).await?;
    c.send(CMSG_GOSSIP_HELLO { guid: who }).await;
    for (_, o) in path.iter().copied().chain([(menu, option)]) {
        let shown = c
            .gets(Duration::from_secs(5), |m| match m {
                ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(gm) if gm.guid == who => Some(
                    gm.gossips
                        .iter()
                        .map(|x| (x.id, x.message.clone()))
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .await;
        if debug() {
            eprintln!("walk gossip {quest}: at {entry}, shown {shown:?}, choosing {o}");
        }
        choose(&c, who, o).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _ = map;
    if credited(realm, g, quest).await {
        Ok(())
    } else {
        Err(format!(
            "option {option} of menu {menu} at {entry} chosen, not explored"
        ))
    }
}

/// A quest whose start script credits it: taken from its giver, explored.
async fn walk_quest_start(realm: &Arc<Realm>, quest: u32) -> Result<(), String> {
    let giver = realm
        .data
        .npc
        .creature_starts
        .iter()
        .find(|(_, qs)| qs.contains(&quest))
        .map_or(0, |(e, _)| *e);
    let mut last = String::new();
    for horde in sides_for(&realm.data, giver) {
        match quest_start_once(realm, quest, horde).await {
            Ok(()) => return Ok(()),
            Err(why) => last = why,
        }
    }
    Err(last)
}

/// [`walk_quest_start`] by a hero of one side.
async fn quest_start_once(realm: &Arc<Realm>, quest: u32, horde: bool) -> Result<(), String> {
    let data = realm.data.clone();
    let giver = data
        .npc
        .creature_starts
        .iter()
        .chain(data.npc.object_starts.iter())
        .filter(|(_, qs)| qs.contains(&quest))
        .map(|(e, _)| *e)
        .find(|e| spawn_of(realm, *e).is_some());
    let Some(giver) = giver else {
        return Err("no giver in the world".into());
    };
    let q = data.npc.quests[&quest].clone();
    let (race, class) = hero_for(&q, horde);
    let level = (q.level.max(i32::from(q.min_level))).clamp(1, 60) as u8;
    let (mut c, g, _) = hero(realm, &walker_name("Strt"), race, class, level).await;
    // What it follows, done.
    let before: Vec<u32> = data
        .npc
        .quest_graph()
        .prerequisites
        .get(&quest)
        .map(|v| v.iter().map(|p| p.quest).collect())
        .unwrap_or_default();
    let (map, who, _) = try_visit(realm, &mut c, giver).await?;
    map.query(move |m| {
        hardy(m, g);
        fit_for(m, g, &q);
        m.players
            .get_mut(&g)
            .unwrap()
            .journal
            .rewarded
            .extend(before.iter().copied());
        crate::game::map::quests::accept(m, g, who, quest);
    })
    .await;
    if !until(&map, move |m| logged(m, g, quest)).await {
        return Err(format!("not taken from {giver}"));
    }
    if credited(realm, g, quest).await {
        Ok(())
    } else {
        Err(format!("taken from {giver}, not explored"))
    }
}

/// The event quests the world database raises by an area trigger, a
/// gossip option or a quest's start script, walked one by one on the real
/// world: each in a fresh hero's log, its source set off, its credit seen.
#[tokio::test]
async fn the_real_database_event_quests_are_walked() {
    if !have_data() {
        return;
    }
    let realm = real_realm();
    let data = realm.data.clone();
    let mut walks: Vec<(u32, &'static str, u32)> = Vec::new();
    let quests: ahash::AHashSet<u32> = data
        .npc
        .quests
        .values()
        .filter(|q| q.special_flags & 2 != 0)
        .map(|q| q.id)
        .collect();
    let mut triggers: Vec<(u32, u32)> = data
        .npc
        .area_trigger_quests
        .iter()
        .filter(|(t, q)| quests.contains(q) && data.area_triggers.contains_key(t))
        .map(|(t, q)| (*q, *t))
        .collect();
    triggers.sort();
    walks.extend(triggers.iter().map(|(q, t)| (*q, "trigger", *t)));
    let mut gossip: Vec<(u32, u32)> = data
        .scripts
        .gossip
        .iter()
        .flat_map(|(id, v)| v.iter().map(move |s| (s, *id)))
        .filter(|(s, _)| s.command == 7 && quests.contains(&s.datalong[0]))
        .map(|(s, id)| (s.datalong[0], id))
        .collect();
    gossip.sort();
    gossip.dedup_by_key(|(q, _)| *q);
    walks.extend(gossip.iter().map(|(q, id)| (*q, "gossip", *id)));
    let mut starts: Vec<u32> = data
        .scripts
        .quest_start_script
        .iter()
        .filter(|(q, id)| {
            quests.contains(q)
                && data
                    .scripts
                    .quest_start
                    .get(id)
                    .is_some_and(|v| v.iter().any(|s| s.command == 7 && s.datalong[0] == **q))
        })
        .map(|(q, _)| *q)
        .collect();
    starts.sort();
    walks.extend(starts.iter().map(|q| (*q, "start", 0)));
    if let Ok(only) = std::env::var("WALK_ONLY") {
        let only: Vec<u32> = only.split(',').filter_map(|q| q.parse().ok()).collect();
        walks.retain(|(q, _, _)| only.contains(q));
    }
    // A few at a time, each its own hero.
    let mut broken = Vec::new();
    let mut running = tokio::task::JoinSet::new();
    for (quest, kind, id) in walks.iter().copied() {
        if running.len() >= WALKERS
            && let Some(Ok((q, Err(why)))) = running.join_next().await
        {
            broken.push((q, why));
        }
        let realm = realm.clone();
        running.spawn(async move {
            let r = match kind {
                "trigger" => walk_trigger(&realm, quest, id).await,
                "gossip" => walk_gossip(&realm, quest, id).await,
                _ => walk_quest_start(&realm, quest).await,
            };
            if debug() {
                eprintln!("walk {quest} ({kind} {id}): {r:?}");
            }
            (quest, r)
        });
    }
    while let Some(done) = running.join_next().await {
        if let Ok((q, Err(why))) = done {
            broken.push((q, why));
        }
    }
    broken.sort();
    eprintln!(
        "database event quests walked: {} of {}; broken: {broken:?}",
        walks.len() - broken.len(),
        walks.len()
    );
    let unexpected: Vec<&(u32, String)> = broken
        .iter()
        .filter(|(q, _)| !UNWALKED.iter().any(|(k, _)| k == q))
        .collect();
    assert!(unexpected.is_empty(), "{unexpected:?}");
}

/// The database's event quests these walks can't do yet, and why: what
/// their source needs that a walk doesn't set up, or what is still to be
/// found. Each is reported, not failed.
const UNWALKED: &[(u32, &str)] = &[
    (2278, "an object's gossip (the Discs of Norgannon)"),
    (2954, "an object's gossip (the Stone Watcher of Norgannon)"),
    (3821, "a menu's own script, not an option's"),
    (4285, "an object's gossip (the Northern Crystal Pylon)"),
    (4287, "an object's gossip (the Eastern Crystal Pylon)"),
    (4288, "an object's gossip (the Western Crystal Pylon)"),
    (4506, "a menu no spawned creature has"),
    (7481, "a menu no spawned creature has (Dire Maul)"),
    (7482, "a menu no spawned creature has (Dire Maul)"),
    (
        8746,
        "Metzen the Reindeer is out only in the Feast of Winter Veil",
    ),
    (
        8762,
        "Metzen the Reindeer is out only in the Feast of Winter Veil",
    ),
    (7001, "the Stable Master's option wants a tamed ram's aura"),
    (7027, "the Stable Master's option wants a tamed wolf's aura"),
    (
        4964,
        "the warlock's orb quest wants what the walk doesn't give",
    ),
    (
        4975,
        "the warlock's orb quest wants what the walk doesn't give",
    ),
    (
        4001,
        "Kharan's menus by the player's side: still broken or unmet",
    ),
    (4866, "Ragged John's option: still broken or unmet"),
    (
        6001,
        "the Lunaclaw spirit shows no menu: still broken or unmet",
    ),
    (
        6002,
        "the Lunaclaw spirit shows no menu: still broken or unmet",
    ),
];

/// Where an escort's escortee is, for the walks: its creature and place.
fn escortee_of(m: &Map, quest: u32) -> Option<(Guid, Vector3d)> {
    m.escorts
        .iter()
        .find(|e| e.quest == quest)
        .and_then(|e| m.creature(e.creature))
        .map(|c| (c.guid, c.position))
}

/// Take `quest` from the giver nearest its escortee (or any in the world),
/// what it follows done: the map, or why not.
async fn take_escort(
    realm: &Arc<Realm>,
    quest: u32,
    escortee: u32,
    horde: bool,
) -> Result<(Client, Guid, MapHandle), String> {
    let data = realm.data.clone();
    let mut givers: Vec<u32> = data
        .npc
        .creature_starts
        .iter()
        .filter(|(_, qs)| qs.contains(&quest))
        .map(|(e, _)| *e)
        .filter(|e| spawn_of(realm, *e).is_some())
        .collect();
    givers.sort_by_key(|e| (*e != escortee, *e));
    let Some(giver) = givers.first().copied() else {
        return Err("no giver in the world".into());
    };
    let q = data.npc.quests[&quest].clone();
    let (race, class) = hero_for(&q, horde);
    let level = (q.level.max(i32::from(q.min_level))).clamp(1, 60) as u8;
    let (mut c, g, _) = hero(realm, &walker_name("Esco"), race, class, level).await;
    // The escortee and giver back if an earlier walk sent them off.
    for entry in [escortee, giver] {
        if let Some((id, _)) = spawn_of(realm, entry)
            && realm
                .data
                .maps
                .get(&id)
                .is_none_or(|m| m.instance_type == 0)
        {
            realm
                .maps
                .get(realm, MapKey::world(id))
                .query(move |m| {
                    let dead: Vec<Guid> = m
                        .creatures
                        .iter()
                        .filter(|(_, c)| c.entry == entry && !c.alive())
                        .map(|(_, c)| c.guid)
                        .collect();
                    for g in dead {
                        crate::game::map::ai::respawn_now(m, g);
                    }
                })
                .await;
        }
    }
    let before: Vec<u32> = data
        .npc
        .quest_graph()
        .prerequisites
        .get(&quest)
        .map(|v| v.iter().map(|p| p.quest).collect())
        .unwrap_or_default();
    let (map, who, _) = try_visit(realm, &mut c, giver).await?;
    map.query(move |m| {
        hardy(m, g);
        let p = m.players.get_mut(&g).unwrap();
        p.journal.rewarded.extend(before.iter().copied());
        fit_for(m, g, &q);
        crate::game::map::quests::accept(m, g, who, quest);
    })
    .await;
    if !until(&map, move |m| logged(m, g, quest)).await {
        return Err(format!("not taken from {giver}"));
    }
    Ok((c, g, map))
}

/// How fast a walk's escortee goes: an escort's at 30 yards a second; a
/// follower's (led to `goal`), which goes no further than the player's
/// side, at 200, ten yards a 50 ms update, more than the eight yards a
/// look leads it by. So a follower's walk is paced by the map's updates,
/// not by how long they take: at 30 yards a second it made 1.5 yards in a
/// 50 ms update, and Tooga's 3,300 yards outlasted the walk's 1,500 looks
/// unless the machine was busy enough to make the updates long.
fn escortee_speed(goal: Option<Vector3d>) -> f32 {
    if goal.is_some() { 200.0 } else { 30.0 }
}

/// An escort or follower quest: taken, the escortee hurried along (out of
/// its fights, which it waits out, and kept whole), the player at its
/// side, or, for a follower, the player leading it to its goal: whether
/// it was credited.
async fn walk_escort(
    realm: &Arc<Realm>,
    quest: u32,
    escortee: u32,
    goal: Option<Vector3d>,
) -> Result<(), String> {
    let mut taken = Err(String::new());
    for horde in sides_for(&realm.data, escortee) {
        taken = take_escort(realm, quest, escortee, horde).await;
        if taken.is_ok() {
            break;
        }
    }
    let (_c, g, map) = taken?;
    let mut last = String::new();
    // Counted in the map's updates, which move the escortee, not in looks,
    // which may come more than once an update.
    let (mut updates, mut seen) = (0, None);
    while updates < 1500 {
        let state = map
            .query(move |m| {
                let Some((e, at)) = escortee_of(m, quest) else {
                    return (
                        explored(m, g, quest),
                        true,
                        String::from("no escort"),
                        m.stats.updates,
                    );
                };
                // Not so fast it outruns the player between looks, or for a
                // follower, as fast as its lead (see `escortee_speed`).
                let speed = escortee_speed(goal);
                if let Some(c) = m.creature_mut(e) {
                    c.speeds.walk = speed;
                    c.speeds.run = speed;
                }
                let fighting = m
                    .creature(e)
                    .filter(|c| c.combat.attacking)
                    .map(|c| c.combat.victim);
                // One that turns on the player, or that may be fought and
                // not killed (Slim Jahn's brawl), is fought.
                let floor = crate::game::map::event_ai::scripted(m, e)
                    .is_some_and(|s| s.invincible_hp.is_some_and(|hp| hp > 0));
                let turned =
                    crate::game::map::combat::can_attack(m, g, e) && (floor || fighting == Some(g));
                if turned {
                    let (_, max) = health(m, e);
                    crate::game::map::ai::attacked(m, e, g);
                    crate::game::map::combat::deal_damage(m, g, e, (max / 10).max(1));
                } else if fighting.is_some()
                    && !m.escorts.iter().any(|x| x.quest == quest && x.held())
                {
                    crate::game::map::ai::forget(m, e);
                    crate::game::map::combat::leave_combat(m, e);
                }
                if turned {
                    hardy(m, g);
                } else {
                    defend(m, &[g, e]);
                }
                match goal {
                    // A follower: led a little nearer its goal each time.
                    Some(to) => {
                        let d = crate::game::map::combat::dist(at, to);
                        let step = if d > 8.0 { 8.0 / d } else { 1.0 };
                        let next = Vector3d {
                            x: at.x + (to.x - at.x) * step,
                            y: at.y + (to.y - at.y) * step,
                            z: at.z + (to.z - at.z) * step,
                        };
                        place(m, g, next);
                    }
                    None => place(m, g, beside(at)),
                }
                for x in m.escorts.iter_mut() {
                    x.skip_start();
                }
                skip(m, Duration::from_secs(2));
                let log = m.players[&g].journal.log.iter().find(|l| l.quest == quest);
                (
                    log.is_some_and(|l| l.explored),
                    log.is_none_or(|l| l.failed),
                    format!(
                        "point {:?} at {at:?}, health {:?}, fighting {:?}, faction {:?}, event {}",
                        m.escorts
                            .iter()
                            .find(|x| x.quest == quest)
                            .and_then(|x| x.point()),
                        health(m, e),
                        m.creature(e)
                            .map(|c| (c.combat.attacking, c.combat.victim == g)),
                        m.creature(e).map(|c| c
                            .fields
                            .get_u32(crate::game::fields::UNIT_FIELD_FACTIONTEMPLATE)),
                        crate::game::map::map_events::active(m, quest),
                    ),
                    m.stats.updates,
                )
            })
            .await
            .unwrap();
        if seen != Some(state.3) {
            seen = Some(state.3);
            updates += 1;
        }
        last = state.2;
        if std::env::var("ESCORT_TRACE").is_ok() {
            eprintln!("escort {quest}: {last}");
        }
        if state.0 {
            return Ok(());
        }
        if state.1 {
            return Err(format!("failed, or gone: {last}"));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err(format!("not credited: {last}"))
}

/// The escort and follower quests, walked one by one on the real world.
#[tokio::test]
async fn the_real_escort_quests_are_walked() {
    if !have_data() {
        return;
    }
    let realm = real_realm();
    let data = realm.data.clone();
    let mut walks: Vec<(u32, u32, Option<Vector3d>)> = Vec::new();
    let event = |q: u32| {
        data.npc
            .quests
            .get(&q)
            .is_some_and(|q| q.special_flags & 2 != 0)
    };
    for e in crate::game::map::escorts::catalog::ESCORTS {
        // The Nightmare Manifests is walked on its own.
        if e.quest != 8736 && event(e.quest) {
            walks.push((e.quest, e.entry, None));
        }
    }
    for f in crate::game::map::escorts::catalog::FOLLOWERS
        .iter()
        .filter(|f| event(f.quest))
    {
        let goal = match f.goal {
            crate::game::map::escorts::Goal::Creature(entry, _) => {
                spawn_of(&realm, entry).map(|(_, at)| at)
            }
            crate::game::map::escorts::Goal::Point(x, y, z, _) => Some(Vector3d { x, y, z }),
        };
        if let Some(goal) = goal {
            walks.push((f.quest, f.entry, Some(goal)));
        }
    }
    if let Ok(only) = std::env::var("WALK_ONLY") {
        let only: Vec<u32> = only.split(',').filter_map(|q| q.parse().ok()).collect();
        walks.retain(|(q, _, _)| only.contains(q));
    }
    let _ = &data;
    // A few at a time, each its own hero, but never two of an escortee.
    let mut broken = Vec::new();
    let mut running: tokio::task::JoinSet<(u32, u32, Result<(), String>)> =
        tokio::task::JoinSet::new();
    let mut busy: Vec<u32> = Vec::new();
    for (quest, entry, goal) in walks.iter().copied() {
        while running.len() >= WALKERS || busy.contains(&entry) {
            if let Some(Ok((q, e, r))) = running.join_next().await {
                busy.retain(|b| *b != e);
                if let Err(why) = r {
                    broken.push((q, why));
                }
            }
        }
        busy.push(entry);
        let realm = realm.clone();
        running.spawn(async move {
            let r = walk_escort(&realm, quest, entry, goal).await;
            if debug() {
                eprintln!("walk escort {quest} ({entry}): {r:?}");
            }
            (quest, entry, r)
        });
    }
    while let Some(done) = running.join_next().await {
        if let Ok((q, _, Err(why))) = done {
            broken.push((q, why));
        }
    }
    broken.sort();
    eprintln!(
        "escort quests walked: {} of {}; broken: {broken:?}",
        walks.len() - broken.len(),
        walks.len()
    );
    // Celebras the Redeemed shows once the Cursed is dead; Arei, who gives
    // Ancient Spirit, has no spawn in the world database.
    let unexpected: Vec<&(u32, String)> = broken
        .iter()
        .filter(|(q, _)| ![7046, 4261].contains(q))
        .collect();
    assert!(unexpected.is_empty(), "{unexpected:?}");
}

/// Tooga (1560), led the 3,300 yards from Gadgetzan to Torta, is
/// credited within the walk's 1,500 updates however long they take: a
/// follower keeps up with the player's lead each update.
#[tokio::test]
async fn the_real_tooga_is_led_to_torta() {
    if !have_data() {
        return;
    }
    let realm = real_realm();
    let (_, torta) = spawn_of(&realm, 6015).expect("Torta's spawn");
    walk_escort(&realm, 1560, 5955, Some(torta))
        .await
        .expect("Tooga's quest credited");
}

/// The escorts' steps the catalog's first import left out are there, as
/// Thistle Tea's catalog has them: the Defias Traitor's raiders (1651),
/// Gilthares's bandits (4904), Ringo's aura taken off (6482), Slim Jahn's
/// floor of health and the gate that fails him (1249), the
/// Threshwackonator turning on its player (2078).
#[test]
fn the_escort_catalog_has_thistle_teas_steps() {
    use crate::game::map::escorts::{Act, escort, follower};
    let at = |quest: u32, point: u32| -> &'static [Act] {
        escort(quest)
            .and_then(|e| e.points.iter().find(|(p, _)| *p == point))
            .map(|(_, a)| *a)
            .unwrap_or(&[])
    };
    let summons = |acts: &[Act], entry: u32| {
        acts.iter()
            .filter(|a| matches!(a, Act::Summon { entry: e, .. } if *e == entry))
            .count()
    };
    for (point, raiders) in [(7, 3), (8, 4), (9, 5)] {
        assert_eq!(summons(at(1651, point), 6180), raiders, "1651 at {point}");
    }
    for point in [8, 14, 21] {
        assert_eq!(summons(at(4904, point), 10758), 2, "4904 at {point}");
    }
    assert!(
        at(6482, 25)
            .iter()
            .any(|a| matches!(a, Act::RemoveAura(20514)))
    );
    let jahn = escort(1249).unwrap();
    assert!(jahn.accept.iter().any(|a| matches!(a, Act::Invincible(20))));
    assert!(at(1249, 9).iter().any(|a| matches!(a, Act::Fail)));
    assert!(
        follower(2078)
            .unwrap()
            .arrive
            .iter()
            .any(|a| matches!(a, Act::Attack))
    );
}

/// An escortee a script stops where it stands (movement idle) waits there,
/// as Thistle Tea's idle motion takes it off its path, until a script sets
/// it walking again; and a faction an escort sets makes it hostile as a
/// script's does.
#[tokio::test]
async fn a_script_halts_an_escort_and_sets_it_walking() {
    let mut d = GameData::empty();
    escortee(&mut d, GRARK, 4121, HUMAN_START, 1..=45);
    for e in AMBUSHERS.into_iter().chain([7046, 9538, 9539]) {
        d.creature_templates.insert(e, template(e, "Blackrock"));
    }
    let realm = harness::realm_with(d);
    let a = player(&realm, "Alice").await;
    let g = a.guid;
    let map = realm.registry.online(g).unwrap().map();
    let grark = set_out(&map, g, GRARK, 4121).await;
    let movement = |kind: u32| crate::game::data::scripts::ScriptStep {
        command: 20,
        datalong: [kind, 0, 0, 0],
        ..Default::default()
    };
    let mut now = going(&map, g, grark, 4121).await;
    for _ in 0..200 {
        if now.at.x > HUMAN_START.0 + 6.0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        now = going(&map, g, grark, 4121).await;
    }
    let idle = movement(0);
    map.query(move |m| {
        crate::game::map::scripts::execute(m, &idle, grark, g);
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let halted = going(&map, g, grark, 4121).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let still = going(&map, g, grark, 4121).await;
    assert!(halted.held && still.held, "halted: {still:?}");
    assert!(
        (still.at.x - halted.at.x).abs() < 0.5,
        "where it stood: {halted:?} {still:?}"
    );
    let walk = movement(2);
    map.query(move |m| {
        crate::game::map::scripts::execute(m, &walk, grark, g);
    })
    .await;
    let mut on = going(&map, g, grark, 4121).await;
    for _ in 0..200 {
        if on.at.x > still.at.x + 4.0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        on = going(&map, g, grark, 4121).await;
    }
    assert!(!on.held && on.at.x > still.at.x + 4.0, "on its way: {on:?}");
}

/// The Threshwackonator 4100 (2078), led to Gelkak, turns: the faction
/// its arrival sets makes it hostile to players, as a script's would, and
/// it goes for the player.
#[tokio::test]
async fn a_follower_turns_on_its_player_at_its_goal() {
    const THRESH: u32 = 6669;
    const GELKAK: u32 = 6667;
    let mut d = GameData::empty();
    let mut t = template(THRESH, "The Threshwackonator 4100");
    t.npc_flags = 0x2;
    d.creature_templates.insert(THRESH, t);
    d.creature_templates
        .insert(GELKAK, template(GELKAK, "Gelkak Gyromast"));
    // Monster: everyone's enemy.
    d.faction_templates.insert(
        14,
        crate::game::data::FactionTemplate {
            id: 14,
            enemy_mask: 0x7,
            ..Default::default()
        },
    );
    for (guid, entry, dx) in [(THRESH, THRESH, -1.5), (GELKAK, GELKAK, 6.0)] {
        d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
            guid,
            entries: vec![entry],
            map: 0,
            x: HUMAN_START.0 + dx,
            y: HUMAN_START.1,
            z: HUMAN_START.2,
            ..Default::default()
        });
    }
    d.npc.quests.insert(
        2078,
        crate::game::data::npc::Quest {
            id: 2078,
            title: "Gyromast's Revenge".into(),
            min_level: 1,
            level: 1,
            ..Default::default()
        },
    );
    d.npc.creature_starts.insert(THRESH, vec![2078]);
    let realm = harness::realm_with(d);
    let a = player(&realm, "Alice").await;
    let g = a.guid;
    let map = realm.registry.online(g).unwrap().map();
    let thresh = guid::unit(THRESH, THRESH);
    map.query(move |m| {
        if let Some(c) = m.creature_mut(thresh) {
            c.speeds.walk = 20.0;
            c.speeds.run = 20.0;
        }
        crate::game::map::quests::accept(m, g, thresh, 2078);
    })
    .await;
    // Led to Gelkak.
    let turned = until_mut(&map, move |m| {
        place(
            m,
            g,
            Vector3d {
                x: HUMAN_START.0 + 4.0,
                y: HUMAN_START.1,
                z: HUMAN_START.2,
            },
        );
        m.creature(thresh)
            .is_some_and(|c| c.hostile_to_players && c.combat.victim == g)
    })
    .await;
    assert!(turned, "hostile, and at the player");
}
