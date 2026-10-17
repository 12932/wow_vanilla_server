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
