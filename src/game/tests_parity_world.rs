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
