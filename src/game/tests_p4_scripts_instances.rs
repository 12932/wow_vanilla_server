//! Thistle Tea's instance admission, lockouts, membership and reset
//! schedule (`test/game/core/instance/*`), through the protocol: a refusal
//! changes nothing, the hourly quota outlives resets and logouts, a raid's
//! binds follow who was inside and who leads, and a copy is a group's only
//! if the group has it.

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
use crate::game::social::groups;

const RAID: u32 = 409;
const LORD: u32 = 12118;
/// Dungeons: the Deadmines holds one, the rest hold ten.
const DEADMINES: u32 = 36;
const DUNGEONS: [u32; 6] = [33, 34, 43, 47, 48, 90];

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

/// A raid with a boss whose death binds it, and dungeons.
fn copies() -> GameData {
    let mut d = GameData::empty();
    map_info(&mut d, RAID, 2, 40);
    map_info(&mut d, DEADMINES, 1, 1);
    for id in DUNGEONS {
        map_info(&mut d, id, 1, 10);
    }
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

/// Players who are no GMs; empty copies kept ten minutes.
fn realm() -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    cfg.instance_unload = Duration::from_secs(600);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, copies(), None)
}

async fn player(realm: &Arc<Realm>, name: &str) -> Client {
    let mut c = Client::connect(realm, name);
    c.enter(name, Race::Human, Class::Warrior).await;
    c
}

fn send(realm: &Arc<Realm>, g: Guid, map: u32) {
    realm
        .registry
        .online(g)
        .unwrap()
        .map()
        .call(move |m| m.teleport(g, MapKey::world(map), Vector3d::default(), 0.0));
}

/// `c` into `map`'s copy (or the open world): where they landed.
async fn go(realm: &Arc<Realm>, c: &mut Client, map: u32) -> MapKey {
    let g = c.guid;
    send(realm, g, map);
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
    key_of(realm, g)
}

/// `c` sent to `map` and turned away: why.
async fn refused(realm: &Arc<Realm>, c: &mut Client, map: u32) -> TransferAbortReason {
    c.clear();
    send(realm, c.guid, map);
    c.wait_for("SMSG_TRANSFER_ABORTED", |m| match m {
        ServerOpcodeMessage::SMSG_TRANSFER_ABORTED(t) => Some(t.reason),
        _ => None,
    })
    .await
}

fn key_of(realm: &Realm, g: Guid) -> MapKey {
    realm.registry.online(g).unwrap().map().key
}

fn bind(realm: &Realm, g: Guid) -> Option<u32> {
    realm
        .social
        .instances
        .lock()
        .raid_bind(guid::counter(g), RAID)
}

fn group_id(realm: &Realm, g: Guid) -> Option<u32> {
    realm.social.groups.lock().group_of(g).map(|gr| gr.id)
}

/// `members` in `leader`'s raid group, invited and accepting.
fn raid_group(realm: &Arc<Realm>, leader: Guid, members: &[Guid]) {
    let l = realm.registry.online(leader).unwrap();
    for m in members {
        let m = realm.registry.online(*m).unwrap();
        if !groups::join_directly(realm, &l, &m) {
            groups::invite(realm, &l, &m.name);
            groups::accept(realm, &m);
        }
        assert_eq!(group_id(realm, m.guid), group_id(realm, leader));
    }
    groups::convert_to_raid(realm, &l);
}

fn leave(realm: &Arc<Realm>, g: Guid) {
    groups::leave(realm, &realm.registry.online(g).unwrap());
}

/// The raid's boss slain in `g`'s copy, and `g` told of a save if one was
/// made.
async fn kill_lord(realm: &Arc<Realm>, g: Guid) {
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        let c = m
            .creatures
            .iter()
            .find(|(_, c)| c.entry == LORD && c.alive())
            .map(|(_, c)| c.guid);
        if let Some(c) = c {
            crate::game::map::combat::deal_damage(m, g, c, 1_000_000);
        }
    })
    .await;
}

async fn told_saved(c: &mut Client) {
    c.wait_for("SMSG_INSTANCE_SAVE_CREATED", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_INSTANCE_SAVE_CREATED(_)).then_some(())
    })
    .await;
}

fn now() -> i64 {
    crate::game::social::mail::now()
}

/// Thistle Tea's `admit/7` denials: one turned away from a full copy stays
/// in the copy they are in, still its member, with no copy of the full map
/// made for them, no copy id spent and their account's hourly count as it
/// was; a raid alone is for a raid group, before any copy is made.
#[tokio::test]
async fn a_refusal_changes_nothing() {
    let realm = realm();
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let mine = go(&realm, &mut a, DUNGEONS[0]).await;
    let (ao, bo) = (
        realm.registry.online(a.guid).unwrap(),
        realm.registry.online(b.guid).unwrap(),
    );
    assert!(groups::join_directly(&realm, &ao, &bo));
    let party = go(&realm, &mut b, DEADMINES).await;
    let (ids, counted) = {
        let inst = realm.social.instances.lock();
        (inst.last_id(), inst.entries_within_hour("Alice", now()))
    };
    assert_eq!(counted, 1);
    assert_eq!(
        refused(&realm, &mut a, DEADMINES).await,
        TransferAbortReason::IsFull
    );
    assert_eq!(key_of(&realm, a.guid), mine, "still where she was");
    assert!(instances::belongs(&realm, a.guid, mine), "and its member");
    {
        let inst = realm.social.instances.lock();
        assert_eq!(inst.last_id(), ids, "no copy made");
        assert_eq!(inst.entries_within_hour("Alice", now()), 1, "uncounted");
        assert_eq!(
            inst.saves.values().filter(|s| s.map == DEADMINES).count(),
            1,
            "only the party's copy of the Deadmines"
        );
        assert!(
            !inst.saves[&party.instance]
                .entered
                .contains(&guid::counter(a.guid)),
            "which she isn't in"
        );
    }
    // Out of the party, alone, a raid is for a raid group: no copy for it.
    leave(&realm, a.guid);
    a.clear();
    send(&realm, a.guid, RAID);
    let error = a
        .wait_for("SMSG_RAID_GROUP_ONLY", |m| match m {
            ServerOpcodeMessage::SMSG_RAID_GROUP_ONLY(r) => Some(r.error),
            _ => None,
        })
        .await;
    assert_eq!(error, RaidGroupError::Required);
    let inst = realm.social.instances.lock();
    assert_eq!(inst.last_id(), ids, "no raid copy made");
    assert!(!inst.saves.values().any(|s| s.map == RAID));
}

/// Thistle Tea's account quota: five copies an hour, which neither
/// resetting the copies nor logging out gives back.
#[tokio::test]
async fn resets_and_logouts_keep_the_hourly_count() {
    let realm = realm();
    let mut a = player(&realm, "Alice").await;
    let me = a.guid;
    let mut first = Vec::new();
    for map in &DUNGEONS[..5] {
        first.push(go(&realm, &mut a, *map).await);
    }
    go(&realm, &mut a, 0).await;
    a.clear();
    a.send(CMSG_RESET_INSTANCES {}).await;
    for _ in 0..5 {
        a.wait_for("SMSG_INSTANCE_RESET", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_INSTANCE_RESET(_)).then_some(())
        })
        .await;
    }
    assert!(
        realm.social.instances.lock().saves.is_empty(),
        "every copy reset"
    );
    assert_eq!(
        refused(&realm, &mut a, DUNGEONS[0]).await,
        TransferAbortReason::TooManyInstances,
        "the same dungeon, a new copy: a sixth"
    );
    assert_eq!(
        refused(&realm, &mut a, DUNGEONS[5]).await,
        TransferAbortReason::TooManyInstances
    );
    // Logged out and back: still five this hour.
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
    assert_eq!(
        refused(&realm, &mut a, DUNGEONS[5]).await,
        TransferAbortReason::TooManyInstances,
        "the count kept through the logout"
    );
    assert_eq!(
        realm
            .social
            .instances
            .lock()
            .entries_within_hour("Alice", now()),
        5
    );
}

/// Thistle Tea's `bind_raid/3` with the leader outside: those inside are
/// saved, the leader isn't, so the group isn't; one of the group who goes
/// into the copy after is let in (the group's copy still) and not saved.
#[tokio::test]
async fn a_group_isnt_bound_with_its_leader_outside() {
    let realm = realm();
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let mut c = player(&realm, "Carol").await;
    raid_group(&realm, a.guid, &[b.guid, c.guid]);
    let world = go(&realm, &mut a, RAID).await;
    assert_eq!(go(&realm, &mut b, RAID).await, world);
    go(&realm, &mut a, 0).await;
    kill_lord(&realm, b.guid).await;
    told_saved(&mut b).await;
    assert_eq!(bind(&realm, b.guid), Some(world.instance), "Bob, inside");
    assert_eq!(bind(&realm, a.guid), None, "not the leader outside");
    assert_eq!(go(&realm, &mut c, RAID).await, world, "the group's copy");
    assert_eq!(bind(&realm, c.guid), None, "Carol not saved going in");
}

/// Thistle Tea's bound player in a group bound elsewhere: turned away, the
/// copy they are in theirs still, their own save as it was, the hour's
/// count untouched; the group's copy is none of theirs.
#[tokio::test]
async fn another_save_is_refused_and_changes_nothing() {
    let realm = realm();
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let mut c = player(&realm, "Carol").await;
    let d = player(&realm, "Dave").await;
    raid_group(&realm, a.guid, &[b.guid]);
    let world = go(&realm, &mut a, RAID).await;
    go(&realm, &mut b, RAID).await;
    kill_lord(&realm, a.guid).await;
    told_saved(&mut a).await;
    // Carol is saved to another copy, with Dave's raid.
    raid_group(&realm, c.guid, &[d.guid]);
    let other = go(&realm, &mut c, RAID).await;
    assert_ne!(other, world);
    kill_lord(&realm, c.guid).await;
    told_saved(&mut c).await;
    assert_eq!(bind(&realm, c.guid), Some(other.instance));
    // Out, and into Alice's raid.
    let outside = go(&realm, &mut c, 0).await;
    leave(&realm, c.guid);
    raid_group(&realm, a.guid, &[c.guid]);
    let counted = realm
        .social
        .instances
        .lock()
        .entries_within_hour("Carol", now());
    let ids = realm.social.instances.lock().last_id();
    assert_eq!(
        refused(&realm, &mut c, RAID).await,
        TransferAbortReason::NotFound,
        "her save holds her out of the group's"
    );
    assert_eq!(key_of(&realm, c.guid), outside, "where she was");
    assert_eq!(bind(&realm, c.guid), Some(other.instance), "her save kept");
    assert!(!instances::belongs(&realm, c.guid, world), "none of hers");
    let inst = realm.social.instances.lock();
    assert_eq!(inst.entries_within_hour("Carol", now()), counted);
    assert_eq!(inst.last_id(), ids, "no copy made");
}

/// Thistle Tea's `group_changed/3` promoting a leader with no save: the
/// group lets go of its raid copy, its members keep their own saves and
/// the group its dungeon copy; the new leader's way into the raid is a
/// fresh copy.
#[tokio::test]
async fn an_unsaved_leader_promoted_lets_the_raid_copy_go() {
    let realm = realm();
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let mut c = player(&realm, "Carol").await;
    raid_group(&realm, a.guid, &[b.guid, c.guid]);
    let dungeon = go(&realm, &mut a, DUNGEONS[0]).await;
    let world = go(&realm, &mut a, RAID).await;
    go(&realm, &mut b, RAID).await;
    kill_lord(&realm, a.guid).await;
    told_saved(&mut a).await;
    go(&realm, &mut a, 0).await;
    go(&realm, &mut b, 0).await;
    groups::set_leader(&realm, &realm.registry.online(a.guid).unwrap(), c.guid);
    assert_eq!(
        bind(&realm, a.guid),
        Some(world.instance),
        "Alice's save kept"
    );
    assert_eq!(bind(&realm, b.guid), Some(world.instance), "Bob's");
    let fresh = go(&realm, &mut c, RAID).await;
    assert_ne!(fresh, world, "Carol's is a new copy");
    assert_eq!(
        go(&realm, &mut c, DUNGEONS[0]).await,
        dungeon,
        "the group's dungeon copy kept"
    );
}

/// Thistle Tea's `group_changed/3` promoting a saved leader: the group
/// follows the new leader's save; one saved to the old copy is held out of
/// it, and isn't rebound; one with no save goes into the new leader's.
#[tokio::test]
async fn a_saved_leader_promoted_brings_the_groups_save() {
    let realm = realm();
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let mut c = player(&realm, "Carol").await;
    let d = player(&realm, "Dave").await;
    let mut e = player(&realm, "Eve").await;
    raid_group(&realm, a.guid, &[b.guid]);
    let world = go(&realm, &mut a, RAID).await;
    go(&realm, &mut b, RAID).await;
    kill_lord(&realm, a.guid).await;
    told_saved(&mut a).await;
    go(&realm, &mut a, 0).await;
    go(&realm, &mut b, 0).await;
    raid_group(&realm, c.guid, &[d.guid]);
    let other = go(&realm, &mut c, RAID).await;
    kill_lord(&realm, c.guid).await;
    told_saved(&mut c).await;
    go(&realm, &mut c, 0).await;
    leave(&realm, c.guid);
    raid_group(&realm, a.guid, &[c.guid, e.guid]);
    groups::set_leader(&realm, &realm.registry.online(a.guid).unwrap(), c.guid);
    assert_eq!(
        refused(&realm, &mut a, RAID).await,
        TransferAbortReason::NotFound,
        "Alice held out of Carol's"
    );
    assert_eq!(bind(&realm, a.guid), Some(world.instance), "not rebound");
    assert_eq!(bind(&realm, b.guid), Some(world.instance));
    assert_eq!(go(&realm, &mut e, RAID).await, other, "Eve into Carol's");
}

/// Thistle Tea's membership: a copy another group still has isn't taken
/// up by a new group of one who was in it; one's own copy from alone isn't
/// a member's way into their new group's.
#[tokio::test]
async fn a_copy_is_a_groups_only_if_it_has_it() {
    let realm = realm();
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let c = player(&realm, "Carol").await;
    let d = player(&realm, "Dave").await;
    let online = |g: Guid| realm.registry.online(g).unwrap();
    // Bob's party of three has a copy Alice went into.
    assert!(groups::join_directly(
        &realm,
        &online(b.guid),
        &online(a.guid)
    ));
    assert!(groups::join_directly(
        &realm,
        &online(b.guid),
        &online(c.guid)
    ));
    let theirs = go(&realm, &mut a, DUNGEONS[1]).await;
    go(&realm, &mut a, 0).await;
    let party = group_id(&realm, b.guid).unwrap();
    leave(&realm, a.guid);
    assert_eq!(group_id(&realm, b.guid), Some(party), "Bob's party lasts");
    // Alice leads a party of her own.
    assert!(groups::join_directly(
        &realm,
        &online(a.guid),
        &online(d.guid)
    ));
    assert_eq!(
        instances::save_of(&realm, theirs.instance).unwrap().owner,
        Some(instances::Owner::Group(party)),
        "still Bob's party's"
    );
    assert!(!instances::belongs(&realm, a.guid, theirs));
    let mine = go(&realm, &mut a, DUNGEONS[1]).await;
    assert_ne!(mine, theirs, "her party's is another");
    // Bob's copy alone isn't his way into Alice's party's.
    leave(&realm, b.guid);
    let solo = go(&realm, &mut b, DUNGEONS[2]).await;
    go(&realm, &mut b, 0).await;
    assert!(groups::join_directly(
        &realm,
        &online(a.guid),
        &online(b.guid)
    ));
    assert!(
        !instances::belongs(&realm, b.guid, solo),
        "his old copy isn't the party's"
    );
    assert_ne!(go(&realm, &mut b, DUNGEONS[2]).await, solo);
}

/// Thistle Tea's `ResetSchedule.advance/2`: a raid's next reset is a
/// whole number of its periods from 04:00 on a Tuesday; a moment short of
/// it, it stands, at it the next, and missed ones are skipped without the
/// hour drifting.
#[test]
fn missed_resets_are_skipped_without_drift() {
    const DAY: i64 = 86_400;
    for (map, days) in [(309, 3), (249, 5), (409, 7)] {
        let p = days * DAY;
        let d = instances::next_reset(map, 1_790_000_000);
        assert_eq!(d % DAY, 4 * 3600, "{map}: at 04:00");
        assert_eq!(instances::next_reset(map, d - 1), d);
        assert_eq!(instances::next_reset(map, d), d + p);
        assert_eq!(instances::next_reset(map, d + 2 * p + 1), d + 3 * p);
        assert_eq!(instances::next_reset(map, d + 2 * p + 1) % DAY, 4 * 3600);
    }
}

/// Thistle Tea's `materialize/2` in the open world: a spawn's creature
/// keeps its database guid as its own; in a copy it keeps it as its spawn.
#[tokio::test]
async fn the_open_world_keeps_database_guids() {
    let mut d = copies();
    d.creature_templates.insert(1000, template(1000, "Spawned"));
    for (map, guid) in [(0, 70_001), (DUNGEONS[0], 70_002)] {
        d.creature_spawns
            .entry(map)
            .or_default()
            .push(CreatureSpawn {
                guid,
                entries: vec![1000],
                map,
                x: 3.0,
                ..Default::default()
            });
    }
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = Realm::new(cfg, store, startup, d, None);
    let mut a = player(&realm, "Alice").await;
    assert_eq!(
        key_of(&realm, a.guid),
        MapKey::world(0),
        "in the open world"
    );
    let of = |realm: &Arc<Realm>, g: Guid| {
        let map = realm.registry.online(g).unwrap().map();
        async move {
            map.query(|m| {
                m.creatures
                    .iter()
                    .find(|(_, c)| c.entry == 1000)
                    .map(|(_, c)| (c.guid, c.spawn_guid))
            })
            .await
            .flatten()
            .unwrap()
        }
    };
    let (g, spawn) = of(&realm, a.guid).await;
    assert_eq!((guid::counter(g), spawn), (70_001, Some(70_001)));
    go(&realm, &mut a, DUNGEONS[0]).await;
    let (_, spawn) = of(&realm, a.guid).await;
    assert_eq!(spawn, Some(70_002), "its spawn kept in a copy");
}
