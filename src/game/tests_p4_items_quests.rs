//! Phase 4's quests, Thistle Tea's escort tests ported: each act an
//! escort's point may run (ambushes, holds, pauses, objects, casts,
//! scenes, signals, waves), Belnistrasz's idol, The Nightmare Manifests'
//! waves and an escort started by another giver. The followers, quest
//! dependencies, sharing and captured animals are in
//! `tests_p4_items_quests2`.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::npc::{BroadcastText, Quest};
use crate::game::data::scripts::{AiEvent, EF_REPEATABLE, EVENT_SCRIPT_EVENT, ScriptStep};
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::world::ScriptPoint;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::escorts::{self, Act, Aim};
use crate::game::map::{Map, MapHandle, MapKey};

/// Where a new human starts: escortees stand here, in her sight.
pub(super) const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const HUMANS: u32 = 1;
const FRIENDS: u32 = 35;
pub(super) const MONSTERS: u32 = 14;

pub(super) fn template(entry: u32, name: &str) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 10,
        level_max: 10,
        faction: FRIENDS,
        unit_class: 1,
        health_max: 500,
        speed_walk: 1.0,
        speed_run: 1.0,
        ..Default::default()
    }
}

pub(super) fn spawn(d: &mut GameData, map: u32, entry: u32, at: (f32, f32, f32)) {
    d.creature_spawns
        .entry(map)
        .or_default()
        .push(CreatureSpawn {
            guid: entry,
            entries: vec![entry],
            map,
            x: at.0,
            y: at.1,
            z: at.2,
            respawn_min: 600,
            respawn_max: 600,
            ..Default::default()
        });
}

pub(super) fn line(d: &mut GameData, id: u32) {
    d.npc.broadcast_texts.insert(
        id,
        BroadcastText {
            male: format!("Line {id}."),
            chat_type: 0,
            ..Default::default()
        },
    );
}

/// The world with the factions the tests fight with.
pub(super) fn world() -> GameData {
    let mut d = GameData::empty();
    for (id, ours, enemies) in [
        (HUMANS, 0x1, 0x2),
        (FRIENDS, 0x4, 0x0),
        (MONSTERS, 0x2, 0x1),
        (10, 0x4, 0x0),
        (79, 0x4, 0x0),
        (189, 0x4, 0x0),
    ] {
        d.faction_templates
            .insert(id, crate::game::tests_p3_combat::faction(id, ours, enemies));
    }
    d
}

/// An escortee or follower of `entry` for `quest` (an exploration that it
/// starts), standing at `at` on `map`.
pub(super) fn questgiver(d: &mut GameData, map: u32, entry: u32, quest: u32, at: (f32, f32, f32)) {
    let mut t = template(entry, "Escortee");
    t.npc_flags = 0x2;
    t.health_max = 100_000;
    d.creature_templates.insert(entry, t);
    spawn(d, map, entry, at);
    d.npc.quests.insert(
        quest,
        Quest {
            id: quest,
            title: format!("Quest {quest}"),
            min_level: 1,
            level: 1,
            special_flags: 2,
            ..Default::default()
        },
    );
    d.npc.creature_starts.insert(entry, vec![quest]);
}

/// An escortee's path: points `points`, two yards apart east from `from`.
fn path(
    d: &mut GameData,
    entry: u32,
    from: (f32, f32, f32),
    points: std::ops::RangeInclusive<u32>,
) {
    d.world.script_waypoints.insert(
        entry,
        points
            .map(|i| ScriptPoint {
                point: i,
                x: from.0 + 2.0 * i as f32,
                y: from.1,
                z: from.2,
                wait_ms: 0,
            })
            .collect(),
    );
}

/// An escortee of `entry` for `quest` a step behind `START`, its path its
/// points `points` east of it.
fn escortee(d: &mut GameData, entry: u32, quest: u32, points: std::ops::RangeInclusive<u32>) {
    questgiver(d, 0, entry, quest, (START.0 - 1.5, START.1, START.2));
    path(d, entry, START, points);
}

/// Players who are no GMs.
pub(super) fn realm_of(d: GameData) -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, d, None)
}

pub(super) async fn player(realm: &Arc<Realm>, name: &str) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, name);
    let g = c.enter(name, Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    (c, g, map)
}

/// `g` takes `quest` from `giver`, which (and its escortee) goes fast.
pub(super) async fn take(map: &MapHandle, g: Guid, giver: Guid, quest: u32) {
    map.query(move |m| {
        crate::game::map::quests::accept(m, g, giver, quest);
        let all: Vec<Guid> = m.escorts.iter().map(|e| e.creature).collect();
        for c in all.into_iter().chain([giver]) {
            if let Some(c) = m.creature_mut(c) {
                c.speeds.walk = 200.0;
                c.speeds.run = 200.0;
            }
        }
    })
    .await;
}

/// The escort of `creature` runs `acts` now.
async fn act(map: &MapHandle, creature: Guid, acts: &'static [Act]) {
    let ran = map
        .query(move |m| escorts::act(m, creature, acts))
        .await
        .unwrap();
    assert!(ran, "an escort under way");
}

/// The escorts' clocks `by` on.
pub(super) async fn hurry(map: &MapHandle, by: Duration) {
    map.query(move |m| {
        for e in m.escorts.iter_mut() {
            e.hurry(by);
        }
    })
    .await;
}

/// Wait until `f` says so, as the map has it.
pub(super) async fn until<T: Send + 'static>(
    map: &MapHandle,
    what: &str,
    f: impl Fn(&mut Map) -> Option<T> + Send + Sync + Clone + 'static,
) -> T {
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        if let Some(t) = map.query(f.clone()).await.flatten() {
            return t;
        }
        assert!(tokio::time::Instant::now() < deadline, "never {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Who says broadcast text `id`, as `c` hears it.
pub(super) async fn said(c: &mut Client, id: u32) -> Guid {
    let text = format!("Line {id}.");
    c.wait_for("the line", |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(s) if s.message == text => match &s.chat_type {
            SMSG_MESSAGECHAT_ChatType::MonsterSay { sender1, .. } => Some(*sender1),
            _ => None,
        },
        _ => None,
    })
    .await
}

fn alive_of(m: &Map, entry: u32) -> Vec<Guid> {
    m.creatures
        .iter()
        .filter(|(_, c)| c.entry == entry && c.alive())
        .map(|(_, c)| c.guid)
        .collect()
}

pub(super) fn the(m: &Map, entry: u32) -> Option<Guid> {
    alive_of(m, entry).first().copied()
}

/// `quest` in `g`'s log: explored, failed.
pub(super) fn logged(m: &Map, g: Guid, quest: u32) -> Option<(bool, bool)> {
    m.players[&g]
        .journal
        .log
        .iter()
        .find(|l| l.quest == quest)
        .map(|l| (l.explored, l.failed))
}

pub(super) fn unit_u32(m: &Map, g: Guid, field: u16) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(field))
}

pub(super) fn place(m: &mut Map, g: Guid, at: Vector3d) {
    if let Some(p) = m.players.get_mut(&g) {
        p.movement.position = at;
    }
}

pub(super) fn dist(a: Vector3d, b: Vector3d) -> f32 {
    crate::game::map::combat::dist(a, b)
}

pub(super) fn at(x: f32, y: f32, z: f32) -> Vector3d {
    Vector3d { x, y, z }
}

// Escorts' acts, each on an escort under way.

const GALEN: u32 = 3584;
const GALEN_QUEST: u32 = 945;
const RAIDER: u32 = 90_001;
const STALKER: u32 = 90_002;
const WATCHER: u32 = 90_003;
const SPEAKER: u32 = 90_004;
const HEARER: u32 = 90_005;
const BRAZIER: u32 = 190_001;
const LAMP: u32 = 190_002;
const WARD: u32 = 95_001;
const BOLT: u32 = 95_002;
pub(super) const UNIT_FLAG_IMMUNE_TO_PLAYER: u32 = 0x200;

/// Galen's escort (945), whose acts these tests try, beside creatures and
/// objects for them.
fn galen() -> GameData {
    let mut d = world();
    escortee(&mut d, GALEN, GALEN_QUEST, 1..=20);
    // He waits at his first point, so his walk ends nothing under test.
    d.world.script_waypoints.get_mut(&GALEN).unwrap()[0].wait_ms = 1_000_000_000;
    d.creature_templates.get_mut(&GALEN).unwrap().unit_flags = UNIT_FLAG_IMMUNE_TO_PLAYER;
    for (entry, name) in [
        (RAIDER, "Raider"),
        (STALKER, "Stalker"),
        (WATCHER, "Watcher"),
        (SPEAKER, "Speaker"),
        (HEARER, "Hearer"),
    ] {
        d.creature_templates.insert(entry, template(entry, name));
    }
    spawn(&mut d, 0, SPEAKER, (START.0 + 10.0, START.1 + 5.0, START.2));
    spawn(&mut d, 0, HEARER, (START.0, START.1 + 12.0, START.2));
    for id in [1188, 1189, 7001, 7002, 7003] {
        line(&mut d, id);
    }
    // The hearer says its line on the escort's word.
    d.scripts.ai_scripts.insert(
        7003,
        vec![ScriptStep {
            dataint: [7003, 0, 0, 0],
            ..Default::default()
        }],
    );
    d.scripts.ai_events.insert(
        HEARER,
        vec![AiEvent {
            id: 1,
            creature: HEARER,
            kind: EVENT_SCRIPT_EVENT,
            chance: 100,
            flags: EF_REPEATABLE,
            params: [2, 0, 0, 0],
            actions: [7003, 0, 0],
            ..Default::default()
        }],
    );
    for (entry, name) in [(BRAZIER, "Brazier"), (LAMP, "Lamp")] {
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
    d.gameobject_spawns.insert(
        0,
        vec![GameObjectSpawn {
            guid: 50,
            entry: LAMP,
            map: 0,
            x: START.0 + 5.0,
            y: START.1 - 5.0,
            z: START.2,
            rotation: [0.0, 0.0, 0.0, 1.0],
            state: 1,
            ..Default::default()
        }],
    );
    let mut ward = SpellTemplate {
        id: WARD,
        name: "Ward".into(),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    ward.effects[0] = SpellEffect {
        effect: 6,
        aura: crate::game::map::auras::AURA_DUMMY,
        target_a: 1,
        ..Default::default()
    };
    let mut bolt = ward.clone();
    bolt.id = BOLT;
    bolt.name = "Bolt".into();
    bolt.casting_time_index = 5;
    d.spells.cast_times.insert(5, 2000);
    d.spells.spells.insert(WARD, ward);
    d.spells.spells.insert(BOLT, bolt);
    d
}

fn galen_guid() -> Guid {
    guid::unit(GALEN, GALEN)
}

/// Alice beside Galen, his escort taken and still to set out.
async fn galen_set_out() -> (Arc<Realm>, Client, Guid, MapHandle) {
    let realm = realm_of(galen());
    let (mut a, g, map) = player(&realm, "Alice").await;
    a.sees(galen_guid()).await;
    a.sees(guid::unit(SPEAKER, SPEAKER)).await;
    a.sees(guid::unit(HEARER, HEARER)).await;
    take(&map, g, galen_guid(), GALEN_QUEST).await;
    (realm, a, g, map)
}

/// Thistle Tea's "lifts unit flags until the escortee respawns": Galen
/// set out is no longer immune to players, and with the escort failed he
/// is put back as he was, immune again and a quest giver.
#[tokio::test]
async fn an_escort_lifts_its_unit_flags_until_it_is_put_back() {
    let (_realm, mut a, g, map) = galen_set_out().await;
    let galen = galen_guid();
    let now = map
        .query(move |m| {
            (
                unit_u32(m, galen, UNIT_FIELD_FLAGS) & UNIT_FLAG_IMMUNE_TO_PLAYER,
                unit_u32(m, galen, UNIT_FIELD_FACTIONTEMPLATE),
                unit_u32(m, galen, UNIT_NPC_FLAGS),
            )
        })
        .await
        .unwrap();
    assert_eq!(now, (0, 79, 0), "lifted, his faction, no giver");
    assert_eq!(said(&mut a, 1189).await, galen, "his accept line");
    // Left behind: the quest fails and he is put back.
    map.query(move |m| place(m, g, at(START.0 + 500.0, START.1, START.2)))
        .await;
    a.wait_for("SMSG_QUESTUPDATE_FAILED", |m| match m {
        ServerOpcodeMessage::SMSG_QUESTUPDATE_FAILED(f) if f.quest_id == GALEN_QUEST => Some(()),
        _ => None,
    })
    .await;
    let back = until(&map, "put back", move |m| {
        let c = the(m, GALEN)?;
        m.escorts.is_empty().then_some(())?;
        Some((
            unit_u32(m, c, UNIT_FIELD_FLAGS) & UNIT_FLAG_IMMUNE_TO_PLAYER,
            unit_u32(m, c, UNIT_NPC_FLAGS),
            m.creature(c).unwrap().position.x,
        ))
    })
    .await;
    assert_eq!(
        back,
        (UNIT_FLAG_IMMUNE_TO_PLAYER, 0x2, START.0 - 1.5),
        "immune again, a giver, where he stood"
    );
}

const AMBUSH: &[Act] = &[
    // At the escortee, for it.
    Act::Summon {
        entry: RAIDER,
        at: (0.0, 0.0, 0.0, 0.0),
        aim: Aim::Escort,
        despawn_ms: 25_000,
    },
    // At the player, for them.
    Act::SummonMany {
        entry: STALKER,
        at: (0.0, 0.0, 0.0, 0.0),
        at_player: true,
        count: 1,
        scatter: 0.0,
        aim: Aim::Player,
        despawn_ms: 30_000,
    },
    // At a place, for no one, until it dies.
    Act::Summon {
        entry: WATCHER,
        at: (START.0 + 20.0, START.1 + 20.0, START.2, 0.5),
        aim: Aim::None,
        despawn_ms: u32::MAX,
    },
    // Another creature speaks.
    Act::SayBy(SPEAKER, 7001),
];

/// Thistle Tea's "summons ambushes at the escortee, the player, or
/// nobody": one at the escortee goes for it, one at the player for them,
/// one at its place for no one; each goes at its time (or, with none,
/// stays); and the nearest of another entry says a line.
#[tokio::test]
async fn an_escorts_ambushes_go_for_whom_they_are_aimed_at() {
    let (_realm, mut a, g, map) = galen_set_out().await;
    let galen = galen_guid();
    act(&map, galen, AMBUSH).await;
    let (raider, stalker, watcher) = map
        .query(move |m| {
            let one = |entry| {
                let c = m.creature(the(m, entry).unwrap()).unwrap();
                (c.position, c.combat.victim, c.combat.attacking)
            };
            (one(RAIDER), one(STALKER), one(WATCHER))
        })
        .await
        .unwrap();
    let (galen_at, alice_at) = map
        .query(move |m| {
            (
                m.creature(galen).unwrap().position,
                m.players[&g].position(),
            )
        })
        .await
        .unwrap();
    assert!(
        dist(raider.0, galen_at) < 1.0,
        "at the escortee: {raider:?}"
    );
    assert_eq!((raider.1, raider.2), (galen, true), "for the escortee");
    assert!(
        dist(stalker.0, alice_at) < 1.0,
        "at the player: {stalker:?}"
    );
    assert_eq!((stalker.1, stalker.2), (g, true), "for the player");
    assert!(
        dist(watcher.0, at(START.0 + 20.0, START.1 + 20.0, START.2)) < 0.1,
        "at its place: {watcher:?}"
    );
    assert!(!watcher.2, "for no one");
    assert_eq!(said(&mut a, 7001).await, guid::unit(SPEAKER, SPEAKER));
    // At their times: the raider at 25 s, the stalker at 30; the watcher
    // stays.
    let summons_on = |by: u64| {
        let map = map.clone();
        async move {
            map.query(move |m| {
                for e in m.escorts.iter_mut() {
                    e.hurry_summons(Duration::from_millis(by));
                }
            })
            .await;
        }
    };
    summons_on(26_000).await;
    until(&map, "the raider gone", |m| {
        (the(m, RAIDER).is_none() && the(m, STALKER).is_some()).then_some(())
    })
    .await;
    summons_on(5_000).await;
    until(&map, "the stalker gone", |m| {
        (the(m, STALKER).is_none() && the(m, WATCHER).is_some()).then_some(())
    })
    .await;
}

const HOLD: &[Act] = &[
    Act::Summon {
        entry: WATCHER,
        at: (START.0 + 20.0, START.1 + 20.0, START.2, 0.5),
        aim: Aim::None,
        despawn_ms: u32::MAX,
    },
    Act::Hold(&[Act::Say(7002)]),
];

/// Thistle Tea's "hold the path until the escortee's summons are gone,
/// then run the release": held while its summon stands, however long;
/// the summon dead, it goes on and says the release's line.
#[tokio::test]
async fn an_escort_holds_until_its_summons_are_gone_then_runs_the_release() {
    let (_realm, mut a, _g, map) = galen_set_out().await;
    let galen = galen_guid();
    act(&map, galen, HOLD).await;
    let held = |m: &mut Map| m.escorts.first().is_some_and(|e| e.held());
    assert_eq!(map.query(held).await, Some(true));
    for _ in 0..3 {
        hurry(&map, Duration::from_secs(60)).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(map.query(held).await, Some(true), "held while it stands");
    }
    map.query(move |m| {
        let w = the(m, WATCHER).unwrap();
        crate::game::map::combat::deal_damage(m, galen, w, 1_000_000);
    })
    .await;
    until(&map, "let go", move |m| (!held(m)).then_some(())).await;
    assert_eq!(said(&mut a, 7002).await, galen, "the release's line");
}

const SCENE_END: &[Act] = &[Act::EmoteBy(SPEAKER, 37), Act::Die];

/// Thistle Tea's "end the event before the escortee dies, and emote
/// through another creature": the nearest of another entry plays the
/// emote; the escortee dies with the escort ended well, so its death
/// fails nothing.
#[tokio::test]
async fn an_escort_ends_well_as_its_escortee_dies_and_another_emotes() {
    let (_realm, mut a, g, map) = galen_set_out().await;
    let galen = galen_guid();
    act(&map, galen, SCENE_END).await;
    let speaker = guid::unit(SPEAKER, SPEAKER);
    let emote = a
        .wait_for("SMSG_EMOTE", |m| match m {
            ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == speaker => Some(e.emote.as_int()),
            _ => None,
        })
        .await;
    assert_eq!(emote, 37);
    let end = until(&map, "the escort over", move |m| {
        m.escorts.is_empty().then_some(())?;
        Some((
            m.creature(galen).is_none_or(|c| !c.alive()),
            logged(m, g, GALEN_QUEST),
            crate::game::map::map_events::active(m, GALEN_QUEST),
        ))
    })
    .await;
    assert_eq!(
        end,
        (true, Some((false, false)), false),
        "dead, nothing failed"
    );
}

const PAUSE: &[Act] = &[Act::Pause(600_000), Act::NpcFlags(0x2)];

/// Thistle Tea's "pause the path and restore the escortee as a quest
/// ender": it stands for the pause, a quest giver again, and walks on once
/// the pause is up.
#[tokio::test]
async fn an_escort_pauses_as_a_quest_giver_again() {
    let (_realm, _a, _g, map) = galen_set_out().await;
    let galen = galen_guid();
    act(&map, galen, PAUSE).await;
    map.query(|m| {
        for e in m.escorts.iter_mut() {
            e.skip_start();
        }
    })
    .await;
    let here = |m: &mut Map| m.creature(galen_guid()).map(|c| c.position.x);
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(60)).await;
        let x = map.query(here).await.flatten();
        assert_eq!(x, Some(START.0 - 1.5), "standing");
    }
    assert_eq!(
        map.query(move |m| unit_u32(m, galen, UNIT_NPC_FLAGS)).await,
        Some(0x2),
        "a quest giver"
    );
    hurry(&map, Duration::from_secs(600)).await;
    until(&map, "walking on", move |m| {
        (here(m)? > START.0 + 1.0).then_some(())
    })
    .await;
}

const OBJECTS: &[Act] = &[
    Act::SummonObject(
        BRAZIER,
        (START.0 + 3.0, START.1 + 3.0, START.2, 0.5),
        40_000,
    ),
    Act::ObjectState(LAMP, 0),
];

/// Thistle Tea's "summon an object and set the state of one nearby": the
/// object at its place, and the nearest lamp set to its state.
#[tokio::test]
async fn an_escort_sets_out_an_object_and_sets_one_near() {
    let (_realm, _a, _g, map) = galen_set_out().await;
    let galen = galen_guid();
    let lamp_state = |m: &Map| {
        m.gameobjects
            .iter()
            .find(|(_, o)| o.entry == LAMP)
            .map(|(_, o)| o.fields.get_u32(GAMEOBJECT_STATE))
    };
    assert_eq!(map.query(move |m| lamp_state(m)).await.flatten(), Some(1));
    act(&map, galen, OBJECTS).await;
    let (brazier, lamp) = map
        .query(move |m| {
            let b = m
                .gameobjects
                .iter()
                .find(|(_, o)| o.entry == BRAZIER)
                .map(|(_, o)| o.position);
            (b, lamp_state(m))
        })
        .await
        .unwrap();
    let b = brazier.expect("the brazier set out");
    assert!(dist(b, at(START.0 + 3.0, START.1 + 3.0, START.2)) < 0.1);
    assert_eq!(lamp, Some(0), "the lamp's state");
}

const LATER_CAST: &[Act] = &[Act::After(11_000, &[Act::Cast(WARD)])];

/// Thistle Tea's "cast a triggered spell on the escortee": after its
/// delay, the spell is on the escortee.
#[tokio::test]
async fn an_escort_casts_a_spell_on_itself_after_its_delay() {
    let (_realm, _a, _g, map) = galen_set_out().await;
    let galen = galen_guid();
    act(&map, galen, LATER_CAST).await;
    let warded = move |m: &mut Map| {
        crate::game::map::auras::auras(m, galen).is_some_and(|a| a.has_spell(WARD))
    };
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(map.query(warded).await, Some(false), "not yet");
    hurry(&map, Duration::from_millis(11_000)).await;
    until(&map, "warded", move |m| warded(m).then_some(())).await;
}

const SCENE: &[Act] = &[
    Act::CastTimed(BOLT),
    Act::UnitFlags(0x1000),
    Act::Face(HEARER),
    Act::Signal(HEARER, 2),
];

/// Thistle Tea's "cast a spell with its cast time, flag the escortee, and
/// turn to and signal another creature": the cast under way, the flag set,
/// the escortee turned to the hearer, and the hearer's EventAI hears the
/// signal (and says its line).
#[tokio::test]
async fn an_escort_casts_flags_itself_turns_and_signals() {
    let (_realm, mut a, _g, map) = galen_set_out().await;
    let galen = galen_guid();
    act(&map, galen, SCENE).await;
    let hearer = guid::unit(HEARER, HEARER);
    let (casting, flags, facing, toward) = map
        .query(move |m| {
            let c = m.creature(galen).unwrap();
            let h = m.creature(hearer).unwrap().position;
            let toward = (h.y - c.position.y).atan2(h.x - c.position.x);
            (
                m.caster(galen)
                    .and_then(|c| c.casting.as_ref())
                    .map(|c| c.spell),
                unit_u32(m, galen, UNIT_FIELD_FLAGS) & 0x1000,
                c.orientation,
                toward.rem_euclid(std::f32::consts::TAU),
            )
        })
        .await
        .unwrap();
    assert_eq!(casting, Some(BOLT), "its cast time");
    assert_eq!(flags, 0x1000);
    assert!((facing - toward).abs() < 0.05, "{facing} toward {toward}");
    assert_eq!(said(&mut a, 7003).await, hearer, "the hearer heard");
}

const WAVES: &[Act] = &[
    Act::SummonMany {
        entry: RAIDER,
        at: (START.0 + 30.0, START.1, START.2, 0.5),
        at_player: false,
        count: 3,
        scatter: 10.0,
        aim: Aim::None,
        despawn_ms: 60_000,
    },
    Act::SummonMany {
        entry: STALKER,
        at: (0.0, 0.0, 0.0, 0.0),
        at_player: true,
        count: 4,
        scatter: 20.0,
        aim: Aim::Escort,
        despawn_ms: 60_000,
    },
];

/// Thistle Tea's "summon several at once, scattered around a point or the
/// player": three within ten yards of the place, four within twenty of
/// the player, for the escortee. (Scattered on the navmesh; with none, on
/// the place itself.)
#[tokio::test]
async fn an_escort_summons_several_scattered_around_a_place_or_the_player() {
    let (_realm, _a, g, map) = galen_set_out().await;
    let galen = galen_guid();
    act(&map, galen, WAVES).await;
    let (raiders, stalkers) = map
        .query(move |m| {
            let alice = m.players[&g].position();
            let raiders: Vec<f32> = alive_of(m, RAIDER)
                .iter()
                .map(|r| {
                    dist(
                        m.creature(*r).unwrap().position,
                        at(START.0 + 30.0, START.1, START.2),
                    )
                })
                .collect();
            let stalkers: Vec<(f32, Guid)> = alive_of(m, STALKER)
                .iter()
                .map(|s| {
                    let c = m.creature(*s).unwrap();
                    (dist(c.position, alice), c.combat.victim)
                })
                .collect();
            (raiders, stalkers)
        })
        .await
        .unwrap();
    assert_eq!(raiders.len(), 3);
    assert!(raiders.iter().all(|d| *d <= 10.0), "{raiders:?}");
    assert_eq!(stalkers.len(), 4);
    assert!(
        stalkers.iter().all(|(d, v)| *d <= 20.0 && *v == galen),
        "{stalkers:?}"
    );
}

// An escort given by another, which fails at a point.

const KAYA: u32 = 4962;
const KAYAS_GIVER: u32 = 4963;
const KAYA_QUEST: u32 = 1249;

/// Thistle Tea's "starts the escort on the escortee near another quest
/// giver" and "fails the quest at a point and credits nobody without a
/// credit point": Mikhail's quest (1249) sets Tapoke out from beside him,
/// never to fall below a fifth of her health; at point 9 the quest fails
/// while she goes on, and with no credit point no one is credited.
#[tokio::test]
async fn an_escort_given_by_another_sets_out_and_fails_at_its_point() {
    let mut d = world();
    questgiver(
        &mut d,
        0,
        KAYAS_GIVER,
        KAYA_QUEST,
        (START.0 + 2.0, START.1, START.2),
    );
    let mut t = template(KAYA, "Tapoke");
    t.health_max = 1_000;
    d.creature_templates.insert(KAYA, t);
    spawn(&mut d, 0, KAYA, (START.0 - 1.5, START.1 + 3.0, START.2));
    path(&mut d, KAYA, (START.0, START.1 + 3.0, START.2), 8..=10);
    let realm = realm_of(d);
    let (mut a, g, map) = player(&realm, "Alice").await;
    let giver = guid::unit(KAYAS_GIVER, KAYAS_GIVER);
    let kaya = guid::unit(KAYA, KAYA);
    a.sees(giver).await;
    take(&map, g, giver, KAYA_QUEST).await;
    let set_out = map
        .query(move |m| m.escorts.iter().map(|e| e.creature).collect::<Vec<_>>())
        .await
        .unwrap();
    assert_eq!(set_out, vec![kaya], "the escortee, not the giver");
    // Never below a fifth.
    let hp = map
        .query(move |m| {
            if let Some(c) = m.creature_mut(kaya) {
                c.speeds.walk = 200.0;
                c.speeds.run = 200.0;
            }
            crate::game::map::combat::deal_damage(m, g, kaya, 5_000);
            unit_u32(m, kaya, UNIT_FIELD_HEALTH)
        })
        .await
        .unwrap();
    assert_eq!(hp, 200, "a fifth left");
    map.query(move |m| {
        crate::game::map::combat::leave_combat(m, kaya);
        for e in m.escorts.iter_mut() {
            e.skip_start();
        }
    })
    .await;
    // Along, at her side: failed at point 9, the escort going on.
    let failed = until(&map, "failed at point 9", move |m| {
        let at = m.creature(kaya)?.position;
        place(m, g, at);
        let (_, failed) = logged(m, g, KAYA_QUEST)?;
        failed.then(|| (m.escorts.len(), m.escorts.first().and_then(|e| e.point())))
    })
    .await;
    assert_eq!(failed.0, 1, "going on");
    assert!(failed.1.is_some_and(|p| p >= 9), "{failed:?}");
    let end = until(&map, "the escort over", move |m| {
        if let Some(at) = m.creature(kaya).map(|c| c.position) {
            place(m, g, at);
        }
        m.escorts.is_empty().then(|| logged(m, g, KAYA_QUEST))
    })
    .await;
    assert_eq!(end, Some((false, true)), "never credited");
}

// Belnistrasz's idol (3525), in Razorfen Downs.

const RFD: u32 = 129;
const BELNISTRASZ: u32 = 8516;
const IDOL_QUEST: u32 = 3525;
const IDOL_POINT: (f32, f32, f32) = (2577.0, 947.0, 53.0);

/// Thistle Tea's catalog test for Belnistrasz: credit at point 24, 244 s
/// after it; at it, quilboar waves at 2, 41, 81, 121 and 161 s (two
/// Withered Battle Boars, 7333, in each), Plaguemaw (7356) at 221 s, his
/// chant (12774) at 1 s and its end at 244 s with the brazier (152097) and
/// the idol's fires put out (Razorfen Downs' field 2).
#[test]
fn belnistrasz_chants_through_five_waves_and_plaguemaw() {
    let e = escorts::escort(IDOL_QUEST).unwrap();
    assert_eq!(
        (e.entry, e.credit_point, e.credit_delay_ms),
        (BELNISTRASZ, Some(24), 244_000)
    );
    let ritual = e.points.iter().find(|(p, _)| *p == 24).unwrap().1;
    let mut summons = Vec::new();
    let mut casts = Vec::new();
    let mut later = Vec::new();
    for act in ritual {
        if let Act::After(ms, acts) = act {
            for a in *acts {
                match a {
                    Act::Summon { entry, .. } => summons.push((*ms, *entry, 1)),
                    Act::SummonMany { entry, count, .. } => summons.push((*ms, *entry, *count)),
                    Act::Cast(s) => casts.push((*ms, *s)),
                    other => later.push((*ms, format!("{other:?}"))),
                }
            }
        }
    }
    let mut times: Vec<u32> = summons.iter().map(|s| s.0).collect();
    times.dedup();
    assert_eq!(times, [2_000, 41_000, 81_000, 121_000, 161_000, 221_000]);
    assert_eq!(
        summons
            .iter()
            .filter(|s| s.0 == 221_000)
            .collect::<Vec<_>>(),
        [&(221_000, 7356, 1)]
    );
    assert_eq!(summons.iter().filter(|s| s.1 == 7333).count(), 5);
    assert_eq!(casts, [(1_000, 12774)]);
    let at_end: Vec<&String> = later
        .iter()
        .filter(|(ms, _)| *ms == 244_000)
        .map(|(_, a)| a)
        .collect();
    assert!(at_end.iter().any(|a| a.starts_with("Interrupt(12774)")));
    assert!(at_end.iter().any(|a| a.starts_with("SummonObject(152097")));
    assert!(at_end.iter().any(|a| a.starts_with("InstanceData(2, 1)")));
    assert!(matches!(ritual.first(), Some(Act::Say(_))));
    assert!(ritual.iter().any(|a| matches!(a, Act::React(0))));
}

/// Belnistrasz's idol walked: at point 24 he takes no heed of attackers;
/// the waves come at their times, for him; the quest is credited only at
/// 244 s, as the brazier comes out, the idol's fires go out (the copy's
/// field 2) and he heeds attackers again.
#[tokio::test]
async fn belnistrasz_holds_the_idol_and_credits_at_the_scenes_end() {
    let mut d = world();
    d.maps.insert(
        RFD,
        MapInfo {
            id: RFD,
            internal_name: "RazorfenDowns".into(),
            name: "Razorfen Downs".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    d.scripts.map_templates.insert(
        RFD,
        crate::game::data::scripts::MapTemplate {
            map: RFD,
            map_type: 1,
            player_limit: 5,
            script_name: "instance_razorfen_downs".into(),
            ..Default::default()
        },
    );
    let from = (IDOL_POINT.0 - 48.0, IDOL_POINT.1, IDOL_POINT.2);
    questgiver(
        &mut d,
        RFD,
        BELNISTRASZ,
        IDOL_QUEST,
        (from.0 - 1.5, from.1, from.2),
    );
    path(&mut d, BELNISTRASZ, from, 22..=26);
    for (entry, name) in [
        (7333, "Withered Battle Boar"),
        (7329, "Withered Quilguard"),
        (7335, "Death's Head Geomancer"),
        (7356, "Plaguemaw the Rotting"),
    ] {
        d.creature_templates.insert(entry, template(entry, name));
    }
    d.gameobject_templates.insert(
        152097,
        GameObjectTemplate {
            entry: 152097,
            kind: 10,
            display_id: 411,
            name: "Brazier".into(),
            size: 1.0,
            ..Default::default()
        },
    );
    let realm = realm_of(d);
    let (mut a, g, _) = player(&realm, "Alice").await;
    let world = realm.registry.online(g).unwrap().map();
    world.call(move |m| m.teleport(g, MapKey::world(RFD), at(from.0, from.1, from.2), 0.0));
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(g).await;
    let map = realm.registry.online(g).unwrap().map();
    let bel = until(&map, "Belnistrasz", |m| the(m, BELNISTRASZ)).await;
    take(&map, g, bel, IDOL_QUEST).await;
    map.query(|m| {
        for e in m.escorts.iter_mut() {
            e.skip_start();
        }
    })
    .await;
    // To the idol, at his side.
    let passive =
        move |m: &Map| crate::game::map::event_ai::scripted(m, bel).is_some_and(|s| s.passive);
    until(&map, "at the idol", move |m| {
        let at = m.creature(bel)?.position;
        place(m, g, at);
        passive(m).then_some(())
    })
    .await;
    let point = map
        .query(|m| m.escorts.first().and_then(|e| e.point()))
        .await
        .flatten();
    assert_eq!(point, Some(24));
    // What has come, by entry, and the state of things.
    let look = move |m: &mut Map| {
        let at = m.creature(bel).map(|c| c.position);
        if let Some(at) = at {
            place(m, g, at);
        }
        let summoned: Vec<u32> = m
            .escorts
            .first()
            .map(|e| e.summoned().iter().map(|s| guid::entry(*s)).collect())
            .unwrap_or_default();
        let count = |entry| summoned.iter().filter(|e| **e == entry).count();
        (
            [count(7333), count(7329), count(7335), count(7356)],
            logged(m, g, IDOL_QUEST).is_some_and(|(e, _)| e),
            m.gameobjects.iter().any(|(_, o)| o.entry == 152097),
            crate::game::map::instance_script::get_data(m, 2),
            passive(m),
        )
    };
    assert_eq!(
        map.query(look).await.unwrap(),
        ([0; 4], false, false, 0, true),
        "nothing yet"
    );
    // The waves, at their times; the boars kept down.
    let mut done = Duration::ZERO;
    for (t, want) in [
        (2_500, [2, 1, 1, 0]),
        (41_500, [4, 2, 2, 0]),
        (81_500, [6, 3, 3, 0]),
        (121_500, [8, 4, 4, 0]),
        (161_500, [10, 5, 5, 0]),
        (221_500, [10, 5, 5, 1]),
    ] {
        let t = Duration::from_millis(t);
        hurry(&map, t - done).await;
        done = t;
        let got = until(&map, "the wave", move |m| {
            let s = look(m);
            (s.0 == want).then_some(s)
        })
        .await;
        assert!(!got.1 && !got.2 && got.4, "not before its end: {got:?}");
        map.query(move |m| {
            for e in [7333, 7329, 7335, 7356] {
                for c in alive_of(m, e) {
                    crate::game::map::combat::deal_damage(m, g, c, 1_000_000);
                }
            }
        })
        .await;
    }
    hurry(&map, Duration::from_millis(242_500) - done).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let before = map.query(look).await.unwrap();
    assert!(!before.1, "not credited before 244 s: {before:?}");
    hurry(&map, Duration::from_millis(2_000)).await;
    let end = until(&map, "the scene's end", move |m| {
        let s = look(m);
        s.1.then_some(s)
    })
    .await;
    assert_eq!(
        (end.2, end.3, end.4),
        (true, 1, false),
        "the brazier, the fires out, heeding attackers"
    );
}

/// Thistle Tea's catalog test for The Nightmare Manifests (8736): Eranikus
/// summoned at point 16 after 10 s; told at point 24; at point 30 twelve
/// summons of phantasms, four at the player, 34 in all, none after the
/// signal that brings him down at 207 s.
#[test]
fn the_nightmare_manifests_summons_eranikus_and_ten_waves() {
    let e = escorts::escort(8736).unwrap();
    let point = |id: u32| e.points.iter().find(|(p, _)| *p == id).unwrap().1;
    let eranikus = point(16).iter().find_map(|a| match a {
        Act::After(ms, acts) => acts.iter().find_map(|a| match a {
            Act::Summon { entry, aim, .. } => Some((*ms, *entry, *aim)),
            _ => None,
        }),
        _ => None,
    });
    assert_eq!(eranikus, Some((10_000, 15491, Aim::None)));
    assert!(point(24).iter().any(|a| matches!(a, Act::Signal(15491, 1))));
    let mut waves = Vec::new();
    let mut descend = None;
    for a in point(30) {
        if let Act::After(ms, acts) = a {
            for a in *acts {
                match a {
                    Act::Summon { .. } => waves.push((*ms, false, 1)),
                    Act::SummonMany {
                        at_player, count, ..
                    } => waves.push((*ms, *at_player, *count)),
                    Act::Signal(15491, 2) => descend = Some(*ms),
                    _ => {}
                }
            }
        }
    }
    assert_eq!(waves.len(), 12);
    assert_eq!(waves.iter().filter(|w| w.1).count(), 4);
    assert_eq!(waves.iter().map(|w| w.2).sum::<u32>(), 34);
    assert_eq!(descend, Some(207_000));
    assert!(waves.iter().all(|w| w.0 <= 207_000));
}
