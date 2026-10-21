//! Round 12a: what a restart keeps, as mangos-classic keeps it. Players
//! in a dungeon or raid copy their group had are back in it at their
//! login (mangos's group binds outlast a restart); the open world's
//! respawn times are kept (`creature_respawn`, `gameobject_respawn`), so
//! a world boss or a chest that was down is still down; and an empty
//! dungeon copy lasts thirty minutes (`NORMAL_INSTANCE_RESET_TIME`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::scripts::MapTemplate;
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::instances;
use crate::game::map::{MapHandle, MapKey};
use crate::game::tests_p3_combat::START;
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_entity_objects::{object, until};

const RAID: u32 = 409;
const DUNGEON: u32 = 36;
const LORD: u32 = 12118;
const DRAGON: u32 = 6109;
const DRAGON_SPAWN: u32 = 912_001;
const BUCKET: u32 = 997_905;
const BUCKET_SPAWN: u32 = 912_002;
const LOOT: u32 = 997_910;
const COIN: u32 = 997_911;

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

fn map_info(d: &mut GameData, id: u32, kind: u32, limit: u32) {
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
            ..Default::default()
        },
    );
}

/// A raid with a boss that binds it, a dungeon, and in the open world a
/// world boss that comes back in an hour and a chest that comes back in
/// three.
fn world() -> GameData {
    let mut d = GameData::empty();
    map_info(&mut d, RAID, 2, 40);
    map_info(&mut d, DUNGEON, 1, 5);
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
    let mut dragon = template(DRAGON, "Azuregos");
    dragon.rank = 3;
    d.creature_templates.insert(DRAGON, dragon);
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: DRAGON_SPAWN,
            entries: vec![DRAGON],
            map: 0,
            x: START.0 + 5.0,
            y: START.1,
            z: START.2,
            respawn_min: 3600,
            respawn_max: 3600,
            ..Default::default()
        }],
    );
    d.gameobject_templates
        .insert(BUCKET, object(BUCKET, 3, "Bucket", &[(1, LOOT)]));
    d.gameobject_spawns
        .entry(0)
        .or_default()
        .push(GameObjectSpawn {
            guid: BUCKET_SPAWN,
            entry: BUCKET,
            map: 0,
            x: START.0 + 2.0,
            y: START.1,
            z: START.2,
            rotation: [0.0, 0.0, 0.0, 1.0],
            respawn_min: 10_800,
            state: 1,
            ..Default::default()
        });
    d.loot.gameobject.insert(
        LOOT,
        vec![LootEntry {
            item: COIN,
            chance: 100.0,
            min: 1,
            max: 1,
            ..Default::default()
        }],
    );
    d.items.insert(
        COIN,
        ItemTemplate {
            entry: COIN,
            name: "Coin".into(),
            display_id: 1000,
            stackable: 20,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
    d
}

fn store_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wvs-p12a-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("characters.sqlite")
}

/// The realm on the store at `db`, its players no GMs.
fn realm_on(db: &PathBuf) -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    cfg.instance_unload = Duration::from_secs(600);
    let (store, startup) = crate::game::store::Store::open(db).unwrap();
    Realm::new(cfg, store, startup, world(), None)
}

/// The realm stops, everyone saved, and starts again on the same store.
async fn restart(realm: Arc<Realm>, clients: Vec<Client>, db: &PathBuf) -> Arc<Realm> {
    realm.shutdown().await;
    drop(clients);
    drop(realm);
    realm_on(db)
}

async fn player(realm: &Arc<Realm>, account: &str, name: &str) -> Client {
    let mut c = Client::connect(realm, account);
    c.enter(name, Race::Human, Class::Warrior).await;
    c
}

async fn login(realm: &Arc<Realm>, account: &str, g: Guid) -> Client {
    let mut c = Client::connect(realm, account);
    c.login(g).await;
    c.sees(g).await;
    c
}

fn map_of(realm: &Realm, g: Guid) -> MapHandle {
    realm.registry.online(g).unwrap().map()
}

/// Send `c` into `map`'s copy (or the open world), and wait for it to land.
async fn go(realm: &Arc<Realm>, c: &mut Client, map: u32) {
    let g = c.guid;
    map_of(realm, g).call(move |m| m.teleport(g, MapKey::world(map), Vector3d::default(), 0.0));
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
}

/// `leader` and `members` in a group, a raid if `raid`.
fn group(realm: &Arc<Realm>, leader: Guid, members: &[Guid], raid: bool) {
    let l = realm.registry.online(leader).unwrap();
    for m in members {
        let m = realm.registry.online(*m).unwrap();
        if !crate::game::social::groups::join_directly(realm, &l, &m) {
            crate::game::social::groups::invite(realm, &l, &m.name);
            crate::game::social::groups::accept(realm, &m);
        }
    }
    if raid {
        crate::game::social::groups::convert_to_raid(realm, &l);
    }
}

/// A second and a half of the map's updates: the copies' checks of who
/// belongs (once a second) have run.
async fn a_second_and_more(map: &MapHandle) {
    let from = on(map, |m| m.stats.updates).await;
    until(map, "thirty updates", move |m| m.stats.updates >= from + 30).await;
}

/// No `SMSG_RAID_GROUP_ONLY` with a timer: no one is being sent home.
async fn no_eviction(c: &mut Client) {
    let told = c
        .gets(Duration::from_millis(100), |m| match m {
            ServerOpcodeMessage::SMSG_RAID_GROUP_ONLY(x) if x.homebind_timer != 0 => Some(()),
            _ => None,
        })
        .await;
    assert!(told.is_none(), "a homebind timer was started");
}

/// mangos's `Player::LoadFromDB` → `GetBoundInstanceSaveForSelfOrGroup`:
/// a group in a dungeon copy at a restart is back in it at its login,
/// each of them, and stays (no minute's grace); the copy is not a
/// stranger's. Before, the group's copy came back with no owner, refused
/// them (`belongs`), and they went home.
#[tokio::test]
async fn a_group_in_a_dungeon_at_a_restart_is_back_in_its_copy() {
    let db = store_file("dungeon");
    let realm = realm_on(&db);
    let mut a = player(&realm, "A", "Alice").await;
    let mut b = player(&realm, "B", "Bob").await;
    let (ga, gb) = (a.guid, b.guid);
    group(&realm, ga, &[gb], false);
    go(&realm, &mut a, DUNGEON).await;
    go(&realm, &mut b, DUNGEON).await;
    let copy = map_of(&realm, ga).key;
    assert_eq!(map_of(&realm, gb).key, copy);
    assert_ne!(copy.instance, 0);
    let realm = restart(realm, vec![a, b], &db).await;
    let mut a = login(&realm, "A", ga).await;
    assert_eq!(map_of(&realm, ga).key, copy, "Alice is back in the copy");
    let mut b = login(&realm, "B", gb).await;
    assert_eq!(map_of(&realm, gb).key, copy, "and Bob");
    a.clear();
    b.clear();
    a_second_and_more(&map_of(&realm, ga)).await;
    no_eviction(&mut a).await;
    no_eviction(&mut b).await;
    assert_eq!(map_of(&realm, ga).key, copy, "and they stay");
    // A stranger's it is not; and the group formed again takes it up.
    let c = player(&realm, "C", "Carol").await;
    assert!(!instances::belongs(&realm, c.guid, copy));
    group(&realm, ga, &[gb], false);
    let save = instances::save_of(&realm, copy.instance).unwrap();
    assert!(
        matches!(save.owner, Some(instances::Owner::Group(_))) && !save.held,
        "{save:?}"
    );
    assert!(instances::belongs(&realm, gb, copy));
}

/// The same in a raid, its boss dead and its players bound: back in the
/// copy at their login, with no raid group yet (mangos asks for one at
/// the way in, not at a login), and no one sent home.
#[tokio::test]
async fn a_raid_in_its_copy_at_a_restart_is_back_in_it() {
    let db = store_file("raid");
    let realm = realm_on(&db);
    let mut a = player(&realm, "A", "Alice").await;
    let mut b = player(&realm, "B", "Bob").await;
    let (ga, gb) = (a.guid, b.guid);
    group(&realm, ga, &[gb], true);
    go(&realm, &mut a, RAID).await;
    go(&realm, &mut b, RAID).await;
    let copy = map_of(&realm, ga).key;
    let map = map_of(&realm, ga);
    on(&map, move |m| {
        let lord = m
            .creatures
            .iter()
            .find(|(_, c)| c.entry == LORD)
            .map(|(_, c)| c.guid)
            .unwrap();
        crate::game::map::combat::deal_damage(m, ga, lord, 1_000_000);
    })
    .await;
    let realm = restart(realm, vec![a, b], &db).await;
    let mut a = login(&realm, "A", ga).await;
    let mut b = login(&realm, "B", gb).await;
    assert_eq!(map_of(&realm, ga).key, copy, "Alice is back in the raid");
    assert_eq!(map_of(&realm, gb).key, copy, "and Bob");
    a.clear();
    b.clear();
    a_second_and_more(&map_of(&realm, ga)).await;
    no_eviction(&mut a).await;
    no_eviction(&mut b).await;
    // Its boss is still dead.
    let alive = on(&map_of(&realm, ga), |m| {
        m.creatures
            .iter()
            .any(|(_, c)| c.entry == LORD && c.alive())
    })
    .await;
    assert!(!alive, "Lucifron stays dead");
}

/// mangos's `creature_respawn` and `gameobject_respawn`, for the
/// continents too: a world boss killed and a chest looted before a
/// restart are still down after it, until their times. Before, the open
/// world's were never saved (`change_save` returned for instance 0), and
/// both were back at once.
#[tokio::test]
async fn a_world_boss_and_a_chest_down_at_a_restart_stay_down() {
    let db = store_file("world");
    let realm = realm_on(&db);
    let mut a = player(&realm, "A", "Alice").await;
    let g = a.guid;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = map_of(&realm, g);
    let dragon = on(&map, |m| {
        m.creatures
            .iter()
            .find(|(_, c)| c.entry == DRAGON)
            .map(|(_, c)| c.guid)
            .unwrap()
    })
    .await;
    let bucket = crate::game::guid::game_object(BUCKET, BUCKET_SPAWN);
    a.sees(dragon).await;
    a.sees(bucket).await;
    on(&map, move |m| {
        crate::game::map::combat::deal_damage(m, g, dragon, 1_000_000);
    })
    .await;
    a.send(CMSG_GAMEOBJ_USE { guid: bucket }).await;
    a.wait_frame(
        "SMSG_LOOT_RESPONSE",
        crate::game::map::loot::SMSG_LOOT_RESPONSE,
        |_| true,
    )
    .await;
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.send(CMSG_LOOT_RELEASE { guid: bucket }).await;
    until(&map, "the bucket gone", move |m| {
        m.gameobject(bucket).is_none()
    })
    .await;
    let realm = restart(realm, vec![a], &db).await;
    let mut a = login(&realm, "A", g).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = map_of(&realm, g);
    let (alive, bucket_up, bucket_due) = on(&map, move |m| {
        (
            m.creatures
                .iter()
                .any(|(_, c)| c.entry == DRAGON && c.alive()),
            m.gameobject(bucket).is_some(),
            m.loot_state
                .respawns
                .iter()
                .find(|(_, s)| *s == BUCKET_SPAWN)
                .map(|(t, _)| (*t - m.now).as_secs()),
        )
    })
    .await;
    assert!(!alive, "Azuregos is still dead");
    assert!(!bucket_up, "the bucket is still gone");
    assert!(
        bucket_due.is_some_and(|s| (10_700..=10_800).contains(&s)),
        "back at its time: {bucket_due:?}"
    );
    let dragon_due = on(&map, move |m| {
        m.creatures.iter().find_map(|(_, c)| match c.life {
            crate::game::map::creature::Life::Dead { respawn_at, .. } if c.entry == DRAGON => {
                Some((respawn_at - m.now).as_secs())
            }
            _ => None,
        })
    })
    .await;
    assert!(
        dragon_due.is_some_and(|s| (3_500..=3_600).contains(&s)),
        "Azuregos back in his hour: {dragon_due:?}"
    );
    // Once he is back, his time is forgotten: the next start has him up.
    on(&map, |m| {
        let g = m
            .creatures
            .iter()
            .find(|(_, c)| c.entry == DRAGON)
            .map(|(_, c)| c.guid)
            .unwrap();
        crate::game::map::ai::respawn_now(m, g);
    })
    .await;
    assert!(
        realm
            .social
            .instances
            .lock()
            .world
            .creatures
            .get(&(0, DRAGON_SPAWN))
            .is_none(),
        "forgotten once back"
    );
    a.clear();
}

/// mangos's `NORMAL_INSTANCE_RESET_TIME` and `Instance.UnloadDelay`: a
/// dungeon copy left empty by default lasts thirty minutes, not five. Six
/// minutes empty it is still there to come back to; past thirty it is
/// gone. And a copy's save found after a restart is kept as long.
#[tokio::test]
async fn an_empty_dungeon_copy_lasts_thirty_minutes() {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    cfg.instance_unload =
        crate::game::RealmConfig::from_config(&Default::default()).instance_unload;
    assert_eq!(cfg.instance_unload, Duration::from_secs(1800));
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = Realm::new(cfg, store, startup, world(), None);
    let mut a = player(&realm, "A", "Alice").await;
    go(&realm, &mut a, DUNGEON).await;
    let copy = map_of(&realm, a.guid).key;
    let handle = map_of(&realm, a.guid);
    go(&realm, &mut a, 0).await;
    until(&handle, "emptied", |m| {
        m.players.is_empty() && m.empty_since.is_some()
    })
    .await;
    // Six minutes empty.
    on(&handle, |m| {
        m.empty_since = Some(m.now - Duration::from_secs(360));
    })
    .await;
    a_second_and_more(&handle).await;
    instances::sweep(&realm);
    assert!(
        instances::save_of(&realm, copy.instance).is_some(),
        "kept after six minutes"
    );
    // A save with no copy up (as after a restart), six minutes untouched:
    // kept by the sweep too.
    {
        let mut inst = realm.social.instances.lock();
        inst.saves.get_mut(&copy.instance).unwrap().touched_at -= 360;
    }
    instances::sweep(&realm);
    assert!(instances::save_of(&realm, copy.instance).is_some());
    // Thirty-one minutes empty: gone.
    on(&handle, |m| {
        m.empty_since = Some(m.now - Duration::from_secs(31 * 60));
    })
    .await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while instances::save_of(&realm, copy.instance).is_some() {
        assert!(tokio::time::Instant::now() < deadline, "gone after thirty");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    go(&realm, &mut a, DUNGEON).await;
    assert_ne!(map_of(&realm, a.guid).key, copy, "a fresh copy");
}
