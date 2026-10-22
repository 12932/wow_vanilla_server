//! Thistle Tea's `instance_script_test.exs` (the undead side of
//! Stratholme), `creature_teleport_content_test.exs`,
//! `creature_teleport_vmangos_test.exs` and `world_test.exs`
//! (docs/parity/matrix.csv): Stratholme's doors as the copy settles, the
//! abominations' waves and Ramstein, his aftermath and the Baron let go,
//! on made-up data; a final-target chain that moves only the creature it
//! is handed to; and, on the real world, the Baron's step handed to his
//! own copy's Aurius, who runs his delayed script there, and a database
//! guid resolved inside its own copy.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::{
    faction as faction_of, first, hurry_runs, npc_flags, realm, sleep_ms, stand_state, until,
};
use super::tests_p4_scripts_dungeons::{inside as copy_of, set as set_field};
use crate::game::Realm;
use crate::game::data::npc::BroadcastText;
use crate::game::data::scripts::{
    InlineText, SF_SWAP_FINAL_TARGETS, ScriptStep, ScriptTable, StepExt,
};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::combat::dist;
use crate::game::map::creature::Motion;
use crate::game::map::gameobject::GameObject;
use crate::game::map::instance_adapters::{Act, operate};
use crate::game::map::instance_script::{
    self, DONE, Event, IN_PROGRESS, Script, get_data, set_data,
};
use crate::game::map::{Map, MapKey, ai, event_ai, scripts};
use crate::game::tests_p3_combat::{HELPER, setup_with, spawn};

const STRATHOLME: u32 = 329;
/// By the Slaughter Square, where Ramstein and the Black Guard come: near
/// enough to hear everyone there yell.
const AT: (f32, f32, f32) = (4_030.0, -3_395.0, 118.0);
const ZIGGURAT_ONE: u32 = 175_380;
const ZIGGURAT_FOUR: u32 = 175_405;
const ZIGGURAT_FIVE: u32 = 175_796;
const PORT_GAUNTLET: u32 = 175_374;
const PORT_SLAUGHTER: u32 = 175_373;
const SLAUGHTER_GATE: u32 = 175_358;
/// Each spawns shut, as the world database has it.
const DOORS: [u32; 6] = [
    ZIGGURAT_ONE,
    ZIGGURAT_FOUR,
    ZIGGURAT_FIVE,
    PORT_GAUNTLET,
    PORT_SLAUGHTER,
    SLAUGHTER_GATE,
];
const BARONESS: u32 = 10_436;
const BARONESS_SPAWN: u32 = 200;
const BARON: u32 = 10_440;
const BARON_SPAWN: u32 = 54_241;
const RAMSTEIN: u32 = 10_439;
const MINDLESS_UNDEAD: u32 = 11_030;
const BLACK_GUARD: u32 = 10_394;
/// The abominations Ramstein waits on: (spawn, entry).
const ABOMINATIONS: [(u32, u32); 13] = [
    (53_969, 10_416),
    (54_002, 10_416),
    (54_018, 10_416),
    (54_019, 10_416),
    (54_020, 10_417),
    (54_021, 10_417),
    (54_022, 10_417),
    (54_026, 10_417),
    (54_027, 10_417),
    (54_039, 10_417),
    (54_040, 10_417),
    (54_041, 10_417),
    (54_050, 10_417),
];
/// Not attackable, not selectable: the Baron until the Black Guard comes.
const BARON_LOCKED: u32 = 0x0200_0002;
/// Ramstein's field at its abominations' march.
const SPECIAL: u32 = 4;
/// The lines yelled: Ramstein called, the Black Guard come, the Baron
/// ready.
const RAMSTEIN_CALLED: u32 = 6_398;
const GUARD_COMES: u32 = 6_415;
const BARON_READY: u32 = 6_401;

fn creature(entry: u32, name: &str) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        display_scales: [1.0, 0.0, 0.0, 0.0],
        level_min: 60,
        level_max: 60,
        faction: 35,
        unit_class: 1,
        health_max: 1_000,
        ..Default::default()
    }
}

/// Stratholme's undead side: its doors, the Baroness, the Baron, the
/// abominations, and those the script summons.
fn data() -> GameData {
    let mut d = GameData::empty();
    d.maps.insert(
        STRATHOLME,
        MapInfo {
            id: STRATHOLME,
            internal_name: "Stratholme".into(),
            name: "Stratholme".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    for (k, entry) in DOORS.iter().enumerate() {
        d.gameobject_templates.insert(
            *entry,
            GameObjectTemplate {
                entry: *entry,
                kind: 0,
                display_id: 1,
                name: format!("Door {entry}"),
                size: 1.0,
                ..Default::default()
            },
        );
        d.gameobject_spawns
            .entry(STRATHOLME)
            .or_default()
            .push(GameObjectSpawn {
                guid: 100 + k as u32,
                entry: *entry,
                map: STRATHOLME,
                x: AT.0 + k as f32,
                y: AT.1 - 5.0,
                z: AT.2,
                rotation: [0.0, 0.0, 0.0, 1.0],
                state: 1,
                ..Default::default()
            });
    }
    for (entry, name) in [
        (BARONESS, "Baroness"),
        (BARON, "Baron"),
        (10_416, "Bile Spewer"),
        (10_417, "Venom Belcher"),
        (RAMSTEIN, "Ramstein"),
        (MINDLESS_UNDEAD, "Mindless Undead"),
        (BLACK_GUARD, "Black Guard"),
    ] {
        d.creature_templates.insert(entry, creature(entry, name));
    }
    let spawns = [(BARONESS_SPAWN, BARONESS), (BARON_SPAWN, BARON)]
        .into_iter()
        .chain(ABOMINATIONS);
    for (k, (spawn, entry)) in spawns.enumerate() {
        d.creature_spawns
            .entry(STRATHOLME)
            .or_default()
            .push(CreatureSpawn {
                guid: spawn,
                entries: vec![entry],
                map: STRATHOLME,
                x: AT.0 - 10.0 + k as f32,
                y: AT.1 + 5.0,
                z: AT.2,
                ..Default::default()
            });
    }
    for id in [RAMSTEIN_CALLED, GUARD_COMES, BARON_READY] {
        d.npc.broadcast_texts.insert(
            id,
            BroadcastText {
                male: format!("line {id}"),
                chat_type: scripts::CHAT_YELL,
                ..Default::default()
            },
        );
    }
    d
}

/// Alice in her copy of Stratholme, its script Stratholme's.
async fn inside() -> (Arc<Realm>, Client, MapKey) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let from = realm.registry.online(g).unwrap().map();
    from.call(move |m| {
        m.teleport(
            g,
            MapKey::world(STRATHOLME),
            Vector3d {
                x: AT.0,
                y: AT.1,
                z: AT.2,
            },
            0.0,
        )
    });
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(g).await;
    let key = realm.registry.online(g).unwrap().map().key;
    on(&realm, key, |m| {
        m.instance.script = Some(Script::Stratholme)
    })
    .await;
    (realm, a, key)
}

async fn on<T: Send + 'static>(
    realm: &Arc<Realm>,
    key: MapKey,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, key)
        .query(f)
        .await
        .expect("the copy answers")
}

/// The state of the object of `entry` (0 open, 1 shut).
fn state(m: &Map, entry: u32) -> u32 {
    m.gameobjects
        .iter()
        .map(|(_, o)| o)
        .find(|o| o.entry == entry)
        .map_or(u32::MAX, |o| o.fields.get_u32(GAMEOBJECT_STATE))
}

/// The objects of `entries` gone and spawned again from their rows (shut,
/// as the world database has them), then the copy settled as it is once
/// its spawns are in: each one's state.
fn spawned_again(m: &mut Map, entries: &[u32]) -> Vec<u32> {
    let data = m.realm.data.clone();
    for entry in entries {
        let gone: Vec<Guid> = m
            .gameobjects
            .iter()
            .filter(|(_, o)| o.entry == *entry)
            .map(|(_, o)| o.guid)
            .collect();
        for g in gone {
            m.remove_gameobject(g);
        }
        for s in data.gameobject_spawns[&STRATHOLME]
            .iter()
            .filter(|s| s.entry == *entry)
        {
            m.add_gameobject(GameObject::from_spawn(&data.gameobject_templates[entry], s));
        }
    }
    instance_script::settle_spawns(m);
    entries.iter().map(|e| state(m, *e)).collect()
}

fn of_spawn(m: &Map, spawn: u32) -> Guid {
    scripts::creature_of_spawn(m, spawn).expect("spawned")
}

/// The living creatures of `entry`, in the order they came.
fn living(m: &Map, entry: u32) -> Vec<Guid> {
    let mut all: Vec<Guid> = m
        .creatures
        .iter()
        .map(|(_, c)| c)
        .filter(|c| c.entry == entry && c.alive())
        .map(|c| c.guid)
        .collect();
    all.sort_by_key(|g| g.guid());
    all
}

fn summon(m: &mut Map, entry: u32) -> Guid {
    let at = Vector3d {
        x: AT.0,
        y: AT.1 + 10.0,
        z: AT.2,
    };
    scripts::summon(m, Guid::zero(), entry, at, 0.0, None).expect("summoned")
}

fn unit_flags(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_FLAGS))
}

/// How long each of the copy's timers has left, shortest first.
fn waits(m: &Map) -> Vec<Duration> {
    let mut left: Vec<Duration> = m
        .instance
        .timers
        .iter()
        .map(|(t, _)| t.saturating_duration_since(m.now))
        .collect();
    left.sort();
    left
}

/// The copy's timers due in `after` brought due now, and run.
fn bring_due(m: &mut Map, after: Duration) {
    let now = m.now;
    for (t, _) in m.instance.timers.iter_mut() {
        if t.saturating_duration_since(now) == after {
            *t = now;
        }
    }
    instance_script::update(m);
}

fn secs(n: f32) -> Duration {
    Duration::from_secs_f32(n)
}

fn near(a: Vector3d, b: (f32, f32, f32)) -> bool {
    dist(
        a,
        Vector3d {
            x: b.0,
            y: b.1,
            z: b.2,
        },
    ) < 0.01
}

/// Where `g` is walking to, if it walks.
fn heading(m: &Map, g: Guid) -> Option<Vector3d> {
    match &m.creature(g)?.motion {
        Motion::Moving(w) => w.points.last().copied(),
        Motion::Idle { .. } => None,
    }
}

fn heads_for(m: &Map, g: Guid, to: (f32, f32, f32)) -> bool {
    heading(m, g).is_some_and(|at| near(at, to))
}

/// The lines creatures yell that `c` hears, by speaker, till none comes
/// for half a second.
async fn yells(c: &mut Client) -> Vec<(Guid, String)> {
    let mut heard = Vec::new();
    while let Some(line) = c
        .gets(Duration::from_millis(500), |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(s) => match &s.chat_type {
                SMSG_MESSAGECHAT_ChatType::MonsterYell { sender1, .. } => {
                    Some((*sender1, s.message.clone()))
                }
                _ => None,
            },
            _ => None,
        })
        .await
    {
        heard.push(line);
    }
    heard
}

/// Thistle Tea's "reconciles unloaded doors when their owners spawn". A
/// copy here loads every door with it and keeps it, so where Thistle Tea
/// puts a door right as it comes into the world, this server does as the
/// copy's spawns are in (`settle_spawns`): the Baroness dead, her door
/// opens, and one spawned again shut is opened from the data; the
/// crystals' field set to done moves no door of itself, but the gauntlet's
/// port (and the slaughter's) spawned again open; Ramstein dead, the
/// Slaughter Square's gate opens, and stays open as the copy settles;
/// after its reset it is shut, and one spawned again stays shut.
#[tokio::test]
async fn stratholmes_doors_come_back_as_its_data_has_them_and_the_square_gate_not_past_its_reset() {
    let (realm, _a, key) = inside().await;
    let seen = on(&realm, key, |m| {
        let baroness = of_spawn(m, BARONESS_SPAWN);
        instance_script::creature_event(m, baroness, Event::Death);
        let ziggurat = (get_data(m, 1), state(m, ZIGGURAT_ONE));
        let ziggurat_again = spawned_again(m, &[ZIGGURAT_ONE]);
        set_data(m, 6, DONE, 0);
        let crystals = (
            get_data(m, 6),
            state(m, PORT_GAUNTLET),
            state(m, PORT_SLAUGHTER),
        );
        let ports_again = spawned_again(m, &[PORT_GAUNTLET, PORT_SLAUGHTER]);
        let ramstein = summon(m, RAMSTEIN);
        instance_script::creature_event(m, ramstein, Event::Death);
        let square = state(m, SLAUGHTER_GATE);
        instance_script::settle_spawns(m);
        let square_settled = state(m, SLAUGHTER_GATE);
        bring_due(m, secs(91.0));
        let reset = state(m, SLAUGHTER_GATE);
        let reset_again = spawned_again(m, &[SLAUGHTER_GATE]);
        (
            ziggurat,
            ziggurat_again,
            crystals,
            ports_again,
            (square, square_settled),
            (reset, reset_again),
        )
    })
    .await;
    let (ziggurat, ziggurat_again, crystals, ports_again, square, reset) = seen;
    assert_eq!(ziggurat, (DONE, 0), "her death opens her door");
    assert_eq!(ziggurat_again, [0], "and it comes back open");
    assert_eq!(crystals, (DONE, 1, 1), "the field alone moves no door");
    assert_eq!(ports_again, [0, 0], "the ports come back open");
    assert_eq!(square, (0, 0), "the square's gate open, and left so");
    assert_eq!(reset, (1, vec![1]), "shut at its reset, and it stays so");
}

/// Thistle Tea's "moves abomination waves and summons Ramstein exactly
/// once": the thirteen abominations queue as they spawn; Ramstein's field
/// at 4 (special) shuts the gauntlet's port and sends the first wave in
/// twenty seconds, the first in the queue to the square's end (the next a
/// bile spewer's 45 seconds on); the thirteenth's death, and not before,
/// has Ramstein's field begun and field eight done, the Baron yell, the
/// fourth ziggurat open, and Ramstein summoned walking to the square; a
/// death heard again brings nothing.
#[tokio::test]
async fn the_abominations_march_in_waves_and_ramstein_comes_once_for_the_thirteenth() {
    let (realm, mut a, key) = inside().await;
    let seen = on(&realm, key, |m| {
        let abominations: Vec<Guid> = ABOMINATIONS.iter().map(|(s, _)| of_spawn(m, *s)).collect();
        for g in &abominations {
            instance_script::creature_event(m, *g, Event::Spawned);
        }
        // Open, as the crystals leave it.
        operate(m, PORT_GAUNTLET, Act::Open);
        set_data(m, 4, SPECIAL, 0);
        let special = (get_data(m, 4), state(m, PORT_GAUNTLET), waits(m));
        bring_due(m, secs(20.0));
        let wave = (
            heads_for(m, abominations[0], (4_037.194, -3_473.742, 121.738_81)),
            heading(m, abominations[1]).is_none(),
            waits(m),
        );
        let mut before_last = None;
        for (i, g) in abominations.iter().enumerate() {
            if i == 12 {
                before_last = Some((living(m, RAMSTEIN).len(), get_data(m, 8)));
            }
            instance_script::creature_event(m, *g, Event::Death);
        }
        let ramstein = living(m, RAMSTEIN);
        let called = (
            get_data(m, 4),
            get_data(m, 8),
            ramstein.len(),
            ramstein
                .first()
                .is_some_and(|r| heads_for(m, *r, (4_033.009, -3_404.329_3, 115.355_4))),
            state(m, ZIGGURAT_FOUR),
        );
        let timers = waits(m);
        instance_script::creature_event(m, abominations[12], Event::Death);
        let again = (living(m, RAMSTEIN).len(), waits(m) == timers);
        (
            special,
            wave,
            before_last,
            called,
            again,
            of_spawn(m, BARON_SPAWN),
        )
    })
    .await;
    let (special, wave, before_last, called, again, baron) = seen;
    assert_eq!(
        special,
        (SPECIAL, 1, vec![secs(20.0)]),
        "the port shut, the first wave in twenty seconds"
    );
    assert_eq!(
        wave,
        (true, true, vec![secs(45.0)]),
        "the first in the queue goes, the next wave a bile spewer's 45 seconds on"
    );
    assert_eq!(before_last, Some((0, 0)), "not before the thirteenth");
    assert_eq!(called, (IN_PROGRESS, DONE, 1, true, 0), "Ramstein called");
    assert_eq!(again, (1, true), "and only once");
    assert_eq!(
        yells(&mut a).await,
        [(baron, format!("line {RAMSTEIN_CALLED}"))],
        "the Baron yells once"
    );
}

/// Thistle Tea's "runs the Ramstein aftermath and unlocks Baron": the
/// Baron spawns locked (Ramstein not done); Ramstein's pull begins his
/// field and shuts the gauntlet's port; his death settles it, opens the
/// Slaughter Square's gate, brings 34 mindless undead, and the Black
/// Guard in a minute (the gate's reset at 91 seconds); the Black Guard's
/// five come, the Baron unlocked and the last ziggurats open; the first
/// of them to spawn yells, once; the fifth of them to die (a death heard
/// again not counted) has the Baron yell that he is ready.
#[tokio::test]
async fn ramsteins_death_opens_the_square_calls_the_black_guard_and_lets_the_baron_go() {
    let (realm, mut a, key) = inside().await;
    let seen = on(&realm, key, |m| {
        let baron = of_spawn(m, BARON_SPAWN);
        instance_script::creature_event(m, baron, Event::Spawned);
        let locked = unit_flags(m, baron) & BARON_LOCKED;
        operate(m, PORT_GAUNTLET, Act::Open);
        let ramstein = summon(m, RAMSTEIN);
        instance_script::creature_event(m, ramstein, Event::Aggro);
        let pulled = (get_data(m, 4), state(m, PORT_GAUNTLET));
        instance_script::creature_event(m, ramstein, Event::Death);
        let dead = (
            get_data(m, 4),
            state(m, SLAUGHTER_GATE),
            living(m, MINDLESS_UNDEAD).len(),
            waits(m),
        );
        bring_due(m, secs(60.0));
        let guards = living(m, BLACK_GUARD);
        let assault = (
            guards.len(),
            unit_flags(m, baron) & BARON_LOCKED,
            state(m, ZIGGURAT_FOUR),
            state(m, ZIGGURAT_FIVE),
        );
        instance_script::creature_event(m, guards[0], Event::Spawned);
        instance_script::creature_event(m, guards[1], Event::Spawned);
        for g in &guards[..4] {
            instance_script::creature_event(m, *g, Event::Death);
        }
        (baron, locked, pulled, dead, guards, assault)
    })
    .await;
    let (baron, locked, pulled, dead, guards, assault) = seen;
    assert_eq!(locked, BARON_LOCKED, "the Baron spawns locked");
    assert_eq!(pulled, (IN_PROGRESS, 1), "Ramstein's pull shuts the port");
    assert_eq!(
        dead,
        (DONE, 0, 34, vec![secs(60.0), secs(91.0)]),
        "his death: the square open, the undead, the guard to come"
    );
    assert_eq!(assault, (5, 0, 0, 0), "the guard come, the Baron let go");
    assert_eq!(
        yells(&mut a).await,
        [(guards[0], format!("line {GUARD_COMES}"))],
        "the first of them yells, once; four dead, the Baron not yet"
    );
    let fifth = guards[4];
    on(&realm, key, move |m| {
        instance_script::creature_event(m, fifth, Event::Death);
        instance_script::creature_event(m, fifth, Event::Death);
    })
    .await;
    assert_eq!(
        yells(&mut a).await,
        [(baron, format!("line {BARON_READY}"))],
        "the fifth: the Baron is ready, once"
    );
}

/// Ramstein called stays until he dies (mangos-classic's dead-despawn
/// summon): no time of his own runs out on him. And a copy loaded again
/// with him called but not beaten (his summon gone with the copy) has him
/// back at his post, the fourth ziggurat shut behind him, rather than the
/// Baron locked for the copy's life.
#[tokio::test]
async fn ramstein_called_but_not_beaten_is_back_when_the_copy_loads_again() {
    let (realm, _a, key) = inside().await;
    let seen = on(&realm, key, |m| {
        for (s, _) in ABOMINATIONS {
            let g = of_spawn(m, s);
            instance_script::creature_event(m, g, Event::Death);
        }
        let called = living(m, RAMSTEIN);
        let timed = called
            .iter()
            .any(|r| event_ai::scripted(m, *r).is_some_and(|s| s.despawn_after.is_some()));
        for r in &called {
            scripts::despawn(m, *r);
        }
        let saved = m.instance.data.clone();
        instance_script::load(m, &saved);
        let gone = living(m, RAMSTEIN).len();
        instance_script::settle_spawns(m);
        let back = living(m, RAMSTEIN);
        let post = back
            .first()
            .and_then(|r| m.creature(*r))
            .is_some_and(|c| near(c.position, (4_033.009, -3_404.329_3, 115.355_4)));
        (
            called.len(),
            timed,
            gone,
            back.len(),
            post,
            state(m, ZIGGURAT_FOUR),
        )
    })
    .await;
    assert_eq!(seen, (1, false, 0, 1, true, 1), "Ramstein back at his post");
}

/// `step`, its final targets swapped to the creature of spawn `spawn`.
fn to_spawn(spawn: u32, step: ScriptStep) -> ScriptStep {
    ScriptStep {
        target_type: 11,
        target_param1: spawn,
        data_flags: SF_SWAP_FINAL_TARGETS,
        ..step
    }
}

fn teleport(delay_ms: u32, at: (f32, f32, f32, f32)) -> ScriptStep {
    ScriptStep {
        delay_ms,
        command: 6,
        x: at.0,
        y: at.1,
        z: at.2,
        o: at.3,
        ..Default::default()
    }
}

fn says(delay_ms: u32, line: &str) -> ScriptStep {
    ScriptStep {
        delay_ms,
        command: 0,
        ext: Some(Box::new(StepExt {
            texts: vec![InlineText {
                text: line.into(),
                chat_type: scripts::CHAT_SAY,
                language: 0,
                emote: 0,
            }],
            ..Default::default()
        })),
        ..Default::default()
    }
}

const SPYBOT: u32 = 45_707;
const SPYBOT_TO: (f32, f32, f32, f32) = (-8_408.25, 451.896, 123.76, 5.529_86);
const JESSE: u32 = 9_566;
const JESSE_TO: (f32, f32, f32, f32) = (-3_666.06, -718.628, 9.954_69, 0.0);

/// Thistle Tea's "Spybot and Jesse final-target chains relocate only the
/// receiving owners": Spybot's generic script (4341), each step handed by
/// its final swap to the creature of spawn 45707, stops it walking and
/// puts it at the row's pose, the one running the script unmoved and left
/// with no run; the movement script (148201) whose command 39 hands
/// generic script 144502 to the creature of spawn 9566 has Jesse say his
/// two lines at three and five seconds and, at seven, put at the row's
/// pose, the one that started it unmoved.
#[tokio::test]
async fn a_final_target_chain_moves_only_the_creature_it_hands_its_steps_to() {
    let mut d = crate::game::tests_p3_combat::data();
    d.scripts.generic.insert(
        4_341,
        vec![
            to_spawn(
                SPYBOT,
                ScriptStep {
                    command: 20,
                    ..Default::default()
                },
            ),
            to_spawn(SPYBOT, teleport(0, SPYBOT_TO)),
        ],
    );
    d.scripts.movement.insert(
        148_201,
        vec![to_spawn(
            JESSE,
            ScriptStep {
                command: 39,
                datalong: [144_502, 0, 0, 0],
                dataint: [100, 0, 0, 0],
                ..Default::default()
            },
        )],
    );
    d.scripts.generic.insert(
        144_502,
        vec![
            says(3_000, "First"),
            says(5_000, "Second"),
            teleport(7_000, JESSE_TO),
        ],
    );
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    let (jesse, seen) = map
        .query(move |m| {
            let initiator = spawn(m, ga, HELPER, 20, 5.0);
            let spybot = spawn(m, ga, HELPER, 20, 6.0);
            let actor = spawn(m, ga, HELPER, 20, 7.0);
            let jesse = spawn(m, ga, HELPER, 20, 8.0);
            m.by_spawn.insert(SPYBOT, spybot);
            m.by_spawn.insert(JESSE, jesse);
            let away = m
                .position_of(spybot)
                .map(|p| Vector3d { x: p.x + 20.0, ..p });
            scripts::move_to(m, spybot, away.unwrap(), false);
            let walking = heading(m, spybot).is_some();
            let (initiator_at, actor_at) = (m.position_of(initiator), m.position_of(actor));
            scripts::start(m, ScriptTable::Generic, 4_341, initiator, Guid::zero());
            let s = m.creature(spybot).unwrap();
            let spybot_seen = (
                walking,
                heading(m, spybot).is_none(),
                near(s.position, (SPYBOT_TO.0, SPYBOT_TO.1, SPYBOT_TO.2)),
                s.orientation,
                m.position_of(initiator) == initiator_at,
                m.scripts.runs.iter().any(|r| r.source == initiator),
            );
            scripts::start(m, ScriptTable::Movement, 148_201, actor, actor);
            let mut jesse_at = Vec::new();
            for _ in 0..7 {
                hurry_runs(m, Some(jesse), secs(1.0));
                jesse_at.push(
                    m.position_of(jesse)
                        .is_some_and(|p| near(p, (JESSE_TO.0, JESSE_TO.1, JESSE_TO.2))),
                );
            }
            let jesse_seen = (jesse_at, m.position_of(actor) == actor_at);
            (jesse, (spybot_seen, jesse_seen))
        })
        .await
        .unwrap();
    let (spybot_seen, jesse_seen) = seen;
    assert_eq!(
        spybot_seen,
        (true, true, true, SPYBOT_TO.3, true, false),
        "Spybot stopped and put at the pose; the initiator unmoved, its run done"
    );
    assert_eq!(
        jesse_seen,
        (vec![false, false, false, false, false, false, true], true),
        "Jesse put at the pose at seven seconds; the actor unmoved"
    );
    let mut lines = Vec::new();
    for _ in 0..2 {
        lines.push(
            a.wait_for("Jesse's line", |m| match m {
                ServerOpcodeMessage::SMSG_MESSAGECHAT(s) => match &s.chat_type {
                    SMSG_MESSAGECHAT_ChatType::MonsterSay { sender1, .. } if *sender1 == jesse => {
                        Some(s.message.clone())
                    }
                    _ => None,
                },
                _ => None,
            })
            .await,
        );
    }
    assert_eq!(lines, ["First", "Second"], "Jesse's lines, in order");
}

const AURIUS: u32 = 10_917;
const AURIUS_SPAWN: u32 = 53_297;
const AURIUS_HOME: (f32, f32, f32) = (3_680.53, -3_643.8, 140.027);
/// The Baron's aggro script, whose command 39 hands Aurius his.
const BARON_AGGRO: u32 = 1_044_001;

/// Whether a generic script 10917 runs in `m` from `source` (any, for
/// none).
fn aurius_runs(m: &Map, source: Option<Guid>) -> bool {
    m.scripts.runs.iter().any(|r| {
        r.table == ScriptTable::Generic
            && r.id == AURIUS
            && r.next != usize::MAX
            && source.is_none_or(|s| r.source == s)
    })
}

/// Thistle Tea's "Aurius receives the Baron-forwarded delayed sequence
/// without changing copies", on the real rows: the Baron's aggro step
/// (command 39, target type 11 spawn 53297, its final targets swapped)
/// starts generic script 10917 from his copy's Aurius on him, the Baron
/// unmoved; at ten seconds Aurius takes faction 250, no NPC flags, and
/// stands, where he was; at eleven he is put at (4032.73, -3366.51,
/// 115.063) facing 5.42797, in the same copy, his home still his spawn's;
/// at twelve he walks for (4031.0, -3346.89, 115.107), his home
/// (4033.1, -3349.29, 115.107) facing 6.0; at fifteen his phase is 1.
/// Another copy's Aurius is as he was. (The step's condition, 3756, is
/// field seven at 1: set so.)
#[tokio::test]
#[ignore = "extended"]
async fn aurius_runs_the_barons_handed_script_on_its_delays_in_the_barons_copy_alone() {
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = copy_of(&realm, "Aurius", STRATHOLME, AURIUS).await;
    let (_o, _, other) = copy_of(&realm, "Otherone", STRATHOLME, AURIUS).await;
    let seen = map
        .query(move |m| {
            let baron = first(m, BARON).expect("the Baron");
            let aurius = scripts::creature_of_spawn(m, AURIUS_SPAWN).expect("Aurius");
            set_field(m, 7, 1);
            let parent = m.realm.data.scripts.ai_scripts[&BARON_AGGRO]
                .iter()
                .find(|s| s.command == 39)
                .cloned()
                .expect("the Baron's command 39");
            let pinned = (
                parent.datalong[0],
                parent.target_type,
                parent.target_param1,
                parent.data_flags,
            );
            let (baron_at, aurius_at, key) = (m.position_of(baron), m.position_of(aurius), m.key);
            scripts::execute(m, &parent, baron, me);
            let started = m.scripts.runs.iter().any(|r| {
                r.table == ScriptTable::Generic
                    && r.id == AURIUS
                    && r.source == aurius
                    && r.target == baron
            });
            hurry_runs(m, Some(aurius), secs(10.0));
            let at_ten = (
                faction_of(m, aurius),
                npc_flags(m, aurius),
                stand_state(m, aurius),
                m.position_of(aurius) == aurius_at,
            );
            hurry_runs(m, Some(aurius), secs(1.0));
            let c = m.creature(aurius).expect("still here");
            let at_eleven = (
                near(c.position, (4_032.73, -3_366.51, 115.063)),
                (c.orientation - 5.427_97).abs() < 1e-4,
                near(c.home, AURIUS_HOME),
                m.key == key && scripts::creature_of_spawn(m, AURIUS_SPAWN) == Some(aurius),
            );
            hurry_runs(m, Some(aurius), secs(1.0));
            let c = m.creature(aurius).expect("still here");
            let at_twelve = (
                heads_for(m, aurius, (4_031.0, -3_346.89, 115.107)),
                near(c.home, (4_033.1, -3_349.29, 115.107)),
                c.home_orientation,
            );
            hurry_runs(m, Some(aurius), secs(3.0));
            let at_fifteen = (event_ai::phase(m, aurius), aurius_runs(m, Some(aurius)));
            (
                pinned,
                started,
                at_ten,
                at_eleven,
                at_twelve,
                at_fifteen,
                m.position_of(baron) == baron_at,
            )
        })
        .await
        .unwrap();
    let (pinned, started, at_ten, at_eleven, at_twelve, at_fifteen, baron_unmoved) = seen;
    assert_eq!(
        pinned,
        (AURIUS, 11, AURIUS_SPAWN, SF_SWAP_FINAL_TARGETS),
        "the Baron's step"
    );
    assert!(started, "Aurius's script, from Aurius, on the Baron");
    assert_eq!(at_ten, (250, 0, 0, true), "at ten seconds");
    assert_eq!(
        at_eleven,
        (true, true, true, true),
        "at eleven, put in this copy"
    );
    assert_eq!(
        at_twelve,
        (true, true, 6.0),
        "at twelve, walking, a new home"
    );
    assert_eq!(
        at_fifteen,
        (1, false),
        "at fifteen, phase 1, the script done"
    );
    assert!(baron_unmoved, "the Baron stays");
    let theirs = other
        .query(|m| {
            let a = scripts::creature_of_spawn(m, AURIUS_SPAWN).expect("their Aurius");
            (
                faction_of(m, a),
                npc_flags(m, a),
                m.position_of(a).is_some_and(|p| near(p, AURIUS_HOME)),
                aurius_runs(m, None),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        theirs,
        (35, 3, true, false),
        "another copy's Aurius as he was"
    );
}

/// Thistle Tea's "loaded Baron aggro resolves Aurius inside the current
/// copy": the real Baron pulled (his EventAI aggro script 1044001) in a
/// copy whose field seven is 1, the condition on his command 39, starts
/// generic script 10917 from that copy's Aurius, spawn 53297, on him;
/// another copy runs none.
#[tokio::test]
#[ignore = "extended"]
async fn the_barons_pull_starts_aurius_script_from_his_own_copys_aurius() {
    let Some(realm) = realm() else {
        return;
    };
    let d = &realm.data;
    let spawn = d.creature_spawns[&STRATHOLME]
        .iter()
        .find(|s| s.guid == AURIUS_SPAWN)
        .map(|s| s.entries.clone());
    assert_eq!(spawn, Some(vec![AURIUS]), "spawn 53297 is Aurius's");
    let (_c, me, map) = copy_of(&realm, "Baron", STRATHOLME, BARON).await;
    let (_o, _, other) = copy_of(&realm, "Bystander", STRATHOLME, BARON).await;
    let (baron, aurius) = map
        .query(move |m| {
            set_field(m, 7, 1);
            let baron = first(m, BARON).expect("the Baron");
            let aurius = scripts::creature_of_spawn(m, AURIUS_SPAWN).expect("Aurius");
            ai::attacked(m, baron, me);
            (baron, aurius)
        })
        .await
        .unwrap();
    assert!(
        until(&map, 3000, move |m| m.scripts.runs.iter().any(|r| {
            r.table == ScriptTable::Generic
                && r.id == AURIUS
                && r.source == aurius
                && r.target == baron
        }))
        .await,
        "this copy's Aurius runs it, on the Baron"
    );
    sleep_ms(100).await;
    assert_eq!(
        other.query(|m| aurius_runs(m, None)).await,
        Some(false),
        "another copy's runs none"
    );
}

/// Thistle Tea's "resolves database identities inside the exact world
/// copy": in each of two copies of Stratholme, spawn 53297 is that copy's
/// own Aurius (a change to one is not the other's); the world of map 0
/// has none. (A copy's creatures take their spawn's guid, as mangos's do,
/// so the two have the same GUID, each in its own copy.)
#[tokio::test]
#[ignore = "extended"]
async fn a_spawns_creature_is_found_in_its_own_copy_alone() {
    let Some(realm) = realm() else {
        return;
    };
    let (_a, _, first_copy) = copy_of(&realm, "Firstcopy", STRATHOLME, AURIUS).await;
    let (_b, _, second_copy) = copy_of(&realm, "Secondcopy", STRATHOLME, AURIUS).await;
    let of = |m: &Map| {
        scripts::creature_of_spawn(m, AURIUS_SPAWN)
            .and_then(|g| m.creature(g))
            .map(|c| (m.key, c.entry, c.spawn_guid))
    };
    let mine = first_copy
        .query(move |m| {
            let found = of(&*m);
            if let Some(g) = scripts::creature_of_spawn(m, AURIUS_SPAWN) {
                scripts::set_faction(m, g, 250);
            }
            (found, faction_of(m, of_spawn(m, AURIUS_SPAWN)))
        })
        .await
        .unwrap();
    let theirs = second_copy
        .query(move |m| (of(&*m), faction_of(m, of_spawn(m, AURIUS_SPAWN))))
        .await
        .unwrap();
    let world = realm.maps.get(&realm, MapKey::world(0));
    let none = world
        .query(|m| scripts::creature_of_spawn(m, AURIUS_SPAWN))
        .await
        .unwrap();
    let ((mine, mine_faction), (theirs, theirs_faction)) = (mine, theirs);
    let (mine, theirs) = (mine.expect("mine"), theirs.expect("theirs"));
    assert_eq!(
        (mine.1, mine.2, theirs.1, theirs.2),
        (AURIUS, Some(AURIUS_SPAWN), AURIUS, Some(AURIUS_SPAWN)),
        "each copy's Aurius"
    );
    assert_ne!(mine.0, theirs.0, "in two copies");
    assert_eq!(
        (mine_faction, theirs_faction),
        (250, 35),
        "each its own creature"
    );
    assert_eq!(none, None, "none in the world of map 0");
}
