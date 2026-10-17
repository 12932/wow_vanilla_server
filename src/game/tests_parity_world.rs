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
    let (id, at) = spawn_of(realm, entry).expect("spawned");
    let map = travel(realm, c, id, beside(at)).await;
    let (who, at) = map
        .query(move |m| npc(m, entry).or_else(|| object(m, entry)))
        .await
        .flatten()
        .unwrap_or_else(|| panic!("{entry} here"));
    put(&map, c, beside(at)).await;
    let g = c.guid;
    if guid::is_unit(who) {
        assert!(
            until(&map, move |m| m
                .creature(who)
                .is_some_and(|c| c.observers.contains(&g)))
            .await,
            "{entry} in sight"
        );
    }
    (map, who, at)
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
    let (map, who, _) = visit(realm, c, ender).await;
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
                crate::game::map::combat::deal_damage(m, g, e, (max / 20).max(1));
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
        s = map
            .query(move |m| look(m, Duration::from_secs(2), true))
            .await
            .unwrap();
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
                c.speeds.walk = 30.0;
                c.speeds.run = 30.0;
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
                    ride(m, Duration::from_secs(2));
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
                    ride(m, Duration::from_secs(2));
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

/// The event quests Thistle Tea builds in code, walked through on the real
/// world: Freedom for All Creatures, the Gizelton Caravan, The Great
/// Masquerade, The Balance of Light and Shadow.
#[tokio::test]
async fn the_imported_event_quests_are_done_end_to_end() {
    if !have_data() {
        return;
    }
    let realm = real_realm();
    if wanted("freedom") {
        freedom_for_all_creatures(&realm).await;
    }
    if wanted("caravan") {
        the_gizelton_caravan(&realm).await;
    }
    if wanted("masquerade") {
        the_great_masquerade(&realm).await;
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
