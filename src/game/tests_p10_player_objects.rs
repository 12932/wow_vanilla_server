//! Round 10's player at objects: a spell caster for the living, a node a
//! script unlocks and locks, a coffer behind its door, a quest object's
//! credit for the group (Thistle Tea's `game_objects_test`,
//! `gathering_test`, `object_requirement_test` and `quest_objects_test`).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::npc::Quest;
use crate::game::data::*;
use crate::game::fields::GAMEOBJECT_STATE;
use crate::game::harness::{self, Client};
use crate::game::map::objects::{self, GO_CHEST, GO_GOOBER, GO_SPELLCASTER, GO_STATE_ACTIVE};
use crate::game::map::{Map, MapKey, auras, death};
use crate::game::store::Logged;
use crate::game::tests_p3_combat::{START, data};
use crate::game::tests_p3_entity::{aura_effect, on, spell};
use crate::game::tests_p3_entity_objects::object;

const PORTAL: u32 = 98_910;
const NODE: u32 = 98_911;
const DOOR: u32 = 98_912;
const COFFER: u32 = 98_913;
const RELIC: u32 = 98_914;
const DOOR_SPAWN: u32 = 9_910;
const COFFER_SPAWN: u32 = 9_911;
const LIFT: u32 = 97_990;
const SHARED: u32 = 96_910;
const OWN: u32 = 96_911;

fn world() -> GameData {
    let mut d = data();
    d.spells.spells.insert(
        LIFT,
        spell(LIFT, &[aura_effect(auras::AURA_DUMMY, 1, 0, 1)]),
    );
    for t in [
        object(PORTAL, GO_SPELLCASTER, "Portal", &[(0, LIFT), (1, 0)]),
        // A node behind a lock (Mining's), its loot table its own.
        object(NODE, GO_CHEST, "Node", &[(0, 38), (1, NODE)]),
        object(DOOR, 0, "Door", &[]),
        object(COFFER, GO_CHEST, "Coffer", &[(1, COFFER)]),
        object(RELIC, GO_GOOBER, "Relic", &[]),
    ] {
        d.gameobject_templates.insert(t.entry, t);
    }
    let spawn_at = |guid, entry, dx: f32| GameObjectSpawn {
        guid,
        entry,
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        state: 1,
        ..Default::default()
    };
    d.gameobject_spawns.entry(0).or_default().extend([
        spawn_at(DOOR_SPAWN, DOOR, 30.0),
        spawn_at(COFFER_SPAWN, COFFER, 1.0),
    ]);
    d.object_requirements.insert(COFFER_SPAWN, (1, DOOR_SPAWN));
    for (id, flags) in [
        (SHARED, crate::game::map::questshare::QUEST_FLAGS_SHARABLE),
        (OWN, 0),
    ] {
        let mut q = Quest {
            id,
            title: format!("Quest {id}"),
            min_level: 1,
            level: 1,
            flags,
            ..Default::default()
        };
        q.required_kills[0] = (-(RELIC as i32), 3);
        d.npc.quests.insert(id, q);
    }
    d
}

fn at(dx: f32, dy: f32) -> Vector3d {
    Vector3d {
        x: START.0 + dx,
        y: START.1 + dy,
        z: START.2,
    }
}

async fn alone() -> (Client, Guid, crate::game::map::MapHandle) {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    (a, g, map)
}

fn spawned(m: &Map, spawn: u32) -> Guid {
    m.gameobjects
        .iter()
        .find(|(_, o)| o.spawn_guid == Some(spawn))
        .map(|(_, o)| o.guid)
        .unwrap()
}

/// Thistle Tea's "object type #{type} requires a living nearby player":
/// a spell caster's spell is its user's to cast, so a dead one used it
/// for nothing; alive, they have it. (Its "nonhostile faction" isn't
/// mangos's, which doesn't ask a user's faction at a use.)
#[tokio::test]
async fn a_spell_caster_is_for_the_living() {
    let (a, g, map) = alone().await;
    let portal = on(&map, move |m| {
        objects::summon(m, None, PORTAL, at(1.0, 0.0), 0.0, 0, 0, 0, None).unwrap()
    })
    .await;
    let has = move |m: &Map| auras::auras(m, g).is_some_and(|x| x.has_spell(LIFT));
    on(&map, move |m| death::kill(m, g)).await;
    a.send(CMSG_GAMEOBJ_USE { guid: portal }).await;
    harness::an_update(&map).await;
    assert!(!on(&map, move |m| has(m)).await, "not by the dead");
    on(&map, move |m| {
        death::resurrect(m, g, 1.0);
    })
    .await;
    a.send(CMSG_GAMEOBJ_USE { guid: portal }).await;
    harness::an_update(&map).await;
    assert!(on(&map, move |m| has(m)).await, "by the living");
}

/// Thistle Tea's "runtime unlock and relock control key checks", as
/// mangos's `CanUseNow` reads a script's `GO_FLAG_LOCKED`: a node behind
/// a lock doesn't open by hand; a script's unlock (action 6) lets it, its
/// lock (action 7) shuts it again.
#[tokio::test]
async fn a_node_a_script_unlocks_opens_by_hand_till_it_locks_it() {
    let (a, g, map) = alone().await;
    let node = on(&map, move |m| {
        m.players.get_mut(&g).unwrap().gm = false;
        objects::summon(m, None, NODE, at(1.0, 0.0), 0.0, 0, 0, 0, None).unwrap()
    })
    .await;
    let looting = move |m: &Map| m.players[&g].looting;
    let mut seen = Vec::new();
    for action in [None, Some(6), Some(7)] {
        on(&map, move |m| {
            crate::game::map::loot::release(m, g, node);
            if let Some(action) = action {
                objects::activate(m, g, node, action);
            }
        })
        .await;
        a.send(CMSG_GAMEOBJ_USE { guid: node }).await;
        harness::an_update(&map).await;
        seen.push(on(&map, move |m| looting(m) == Some(node)).await);
    }
    assert_eq!(seen, vec![false, true, false]);
}

/// Thistle Tea's "keeps a coffer shut until its door in the same copy
/// opens" (vmangos's `gameobject_requirement`): a coffer whose door is
/// shut opens to no one; the door open, it opens.
#[tokio::test]
async fn a_coffer_waits_for_its_door() {
    let (a, g, map) = alone().await;
    let (door, coffer) = on(&map, move |m| {
        (spawned(m, DOOR_SPAWN), spawned(m, COFFER_SPAWN))
    })
    .await;
    a.send(CMSG_GAMEOBJ_USE { guid: coffer }).await;
    harness::an_update(&map).await;
    let shut = on(&map, move |m| m.players[&g].looting).await;
    assert_eq!(shut, None, "its door shut");
    on(&map, move |m| {
        m.gameobject_mut(door)
            .unwrap()
            .fields
            .set_u32(GAMEOBJECT_STATE, GO_STATE_ACTIVE);
    })
    .await;
    a.send(CMSG_GAMEOBJ_USE { guid: coffer }).await;
    harness::an_update(&map).await;
    let open = on(&map, move |m| m.players[&g].looting).await;
    assert_eq!(open, Some(coffer), "its door open");
}

/// Thistle Tea's "credits only nearby shareable quests": a quest object
/// used by one of a group counts for the others' shareable quests that
/// ask for it, theirs near enough alone; not their quests that can't be
/// shared.
#[tokio::test]
async fn a_quest_objects_use_counts_for_the_groups_shareable_quests_nearby() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let (oa, ob) = (
        realm.registry.online(ga).unwrap(),
        realm.registry.online(gb).unwrap(),
    );
    assert!(crate::game::social::groups::join_directly(&realm, &oa, &ob));
    let map = oa.map();
    let seen = on(&map, move |m| {
        for g in [ga, gb] {
            m.players.get_mut(&g).unwrap().movement.position = at(0.0, 0.0);
            for quest in [SHARED, OWN] {
                m.players.get_mut(&g).unwrap().journal.add(Logged {
                    quest,
                    ..Default::default()
                });
            }
        }
        let relic = objects::summon(m, None, RELIC, at(1.0, 0.0), 0.0, 0, 0, 0, None).unwrap();
        let count = |m: &Map, g: Guid, quest: u32| {
            let p = &m.players[&g];
            p.journal.log[p.journal.slot_of(quest).unwrap()].kills[0]
        };
        crate::game::map::quests::object_used(m, ga, relic, true);
        let near = [SHARED, OWN].map(|q| (count(m, ga, q), count(m, gb, q)));
        m.players.get_mut(&gb).unwrap().movement.position = at(400.0, 0.0);
        crate::game::map::quests::object_used(m, ga, relic, true);
        let far = [SHARED, OWN].map(|q| (count(m, ga, q), count(m, gb, q)));
        (near, far)
    })
    .await;
    assert_eq!(seen.0, [(1, 1), (1, 0)], "near: the shareable one");
    assert_eq!(seen.1, [(2, 1), (2, 0)], "far: nothing");
}
