//! Phase 3's creature AI tests, EventAI (Thistle Tea's `ai/event_ai_test.exs`,
//! `event_ai_pause_test.exs` and `ai_event_test.exs`): its timers in and out
//! of a fight, a stun's pause, the aura, mana and root events, the edge
//! events and whom they fire at, and its conditions.
//!
//! The creatures stand out of everyone's sight, where they take no turns of
//! their own, so a test drives their EventAI alone, at the times it sets; a
//! yell carries 300 yards, so the player hears what they yell, and its
//! target.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::npc::BroadcastText;
use crate::game::data::scripts::*;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::auras::{AURA_DUMMY, AURA_MOD_ROOT, AURA_MOD_STUN};
use crate::game::map::{Map, MapHandle, MapKey, ai, auras, event_ai};
use crate::game::tests_p3_combat::{START, aura_of, faction};

const HUMANS: u32 = 1;
const PACK: u32 = 14;

/// Out of everyone's sight, within a yell.
const FAR: f32 = 150.0;

const TIMERS: u32 = 4300;
const GUARDED: u32 = 4301;
const PAUSER: u32 = 4302;
const AURAS: u32 = 4303;
const ROOTS: u32 = 4304;
const EDGES: u32 = 4305;
const MOVER: u32 = 4306;
const SUMMONER: u32 = 4307;
const MINION: u32 = 4308;
const LISTENER: u32 = 4309;
const CHIEF: u32 = 4310;
const OTHER: u32 = 4311;
const SPAWNS_ITS: u32 = 4312;
const SPAWNS_OTHERS: u32 = 4313;
const SWAPPER: u32 = 4314;
const LEVELLER: u32 = 4315;
const UNKNOWN: u32 = 4316;
const EMOTER: u32 = 4317;
const WATCHER: u32 = 4318;
const RANDOM: u32 = 4319;
const PHASED: u32 = 4320;

/// The phase's step up, and one of a hundred.
const ADD1: u32 = 1;

/// Spells of the tests: an aura stacking, ones missing, a root, a stun.
const STACKS: u32 = 96_001;
const ABSENT: u32 = 96_002;
const ON_VICTIM: u32 = 96_003;
const NOT_ON_VICTIM: u32 = 96_004;
const BUFF: u32 = 96_005;
const ROOT: u32 = 96_006;
const STUN: u32 = 96_007;

/// The conditions: a spawn not its own, its own spawn, the source of the
/// entry of the one it killed, a victim of level 20, one with no row, a
/// dead target, and a mage.
const NOT_MY_SPAWN: u32 = 10;
const MY_SPAWN: u32 = 11;
const KILLED_A_CHIEF: u32 = 12;
const LEVEL_20: u32 = 13;
const NO_ROW: u32 = 14;
const DEAD_TARGET: u32 = 15;
const MAGES: u32 = 16;

fn ev(kind: u32, params: [i32; 4], flags: u32, action: u32) -> AiEvent {
    AiEvent {
        kind,
        chance: 100,
        flags,
        params,
        actions: [action, 0, 0],
        ..Default::default()
    }
}

/// Script `id` yells `line` at its target (broadcast text `id`).
fn yell(d: &mut GameData, id: u32, line: &str) {
    d.npc.broadcast_texts.insert(
        id,
        BroadcastText {
            male: line.into(),
            chat_type: 1,
            ..Default::default()
        },
    );
    d.scripts.ai_scripts.insert(
        id,
        vec![ScriptStep {
            id,
            command: 0,
            dataint: [id as i32, 0, 0, 0],
            ..Default::default()
        }],
    );
}

fn template(entry: u32, name: &str, faction: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 10,
        level_max: 10,
        faction,
        unit_class: 1,
        health_multiplier: 1.0,
        damage_multiplier: 0.01,
        base_attack_time: 2000,
        detection_range: 0.1,
        ..Default::default()
    }
}

fn at(east: f32, north: f32) -> Vector3d {
    Vector3d {
        x: START.0 + east,
        y: START.1 + north,
        z: START.2,
    }
}

/// The spawns: (guid, entry, east, north).
const SPAWNS: &[(u32, u32, f32, f32)] = &[
    (1, TIMERS, FAR, 0.0),
    (2, GUARDED, FAR, 10.0),
    (3, PAUSER, FAR, 20.0),
    (4, AURAS, FAR, 30.0),
    (5, ROOTS, FAR, 40.0),
    (6, EDGES, FAR, 50.0),
    (7, MOVER, FAR, 60.0),
    (8, SUMMONER, FAR, 70.0),
    (9, MINION, FAR, 72.0),
    (10, LISTENER, FAR, 80.0),
    (11, CHIEF, FAR, 82.0),
    (12, CHIEF, FAR, 84.0),
    (13, OTHER, FAR, 86.0),
    (99, SPAWNS_ITS, FAR, 90.0),
    (98, SPAWNS_OTHERS, FAR, 92.0),
    (14, SWAPPER, FAR, 94.0),
    (15, LEVELLER, FAR, 96.0),
    (16, UNKNOWN, FAR, 98.0),
    (17, EMOTER, 3.0, 0.0),
    (18, WATCHER, FAR, -40.0),
    (19, RANDOM, FAR, -50.0),
    (20, PHASED, FAR, -60.0),
];

fn spell(id: u32, aura: u32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        stack_amount: 5,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura,
        target_a: 1,
        ..Default::default()
    };
    s
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.faction_templates.extend([
        (HUMANS, faction(HUMANS, 0x1, 0x2)),
        (PACK, faction(PACK, 0x2, 0x1)),
    ]);
    d.class_level_stats.insert(
        (1, 10),
        ClassLevelStats {
            health: 100,
            melee_damage: 1.0,
            ..Default::default()
        },
    );
    for (id, aura) in [
        (STACKS, AURA_DUMMY),
        (ABSENT, AURA_DUMMY),
        (ON_VICTIM, AURA_DUMMY),
        (NOT_ON_VICTIM, AURA_DUMMY),
        (BUFF, AURA_DUMMY),
        (ROOT, AURA_MOD_ROOT),
        (STUN, AURA_MOD_STUN),
    ] {
        d.spells.spells.insert(id, spell(id, aura));
    }
    // Script 1 raises the phase by one.
    d.scripts.ai_scripts.insert(
        ADD1,
        vec![ScriptStep {
            id: ADD1,
            command: 44,
            datalong: [1, 1, 0, 0],
            ..Default::default()
        }],
    );
    for (id, line) in [
        (100, "evade"),
        (101, "aura"),
        (102, "missing aura"),
        (103, "target aura"),
        (104, "target missing aura"),
        (105, "rooted"),
        (106, "target mana"),
        (107, "hp"),
        (108, "friendly cc"),
        (109, "missing buff"),
        (110, "hit by spell"),
        (111, "hit target"),
        (112, "script event"),
        (113, "kill"),
        (114, "reached home"),
        (115, "spawned"),
        (116, "masked"),
        (117, "phase 0 or 2"),
        (118, "point"),
        (119, "summoned"),
        (120, "summon died"),
        (121, "summon gone"),
        (122, "chief died"),
        (123, "my spawn"),
        (124, "not my spawn"),
        (125, "killed a chief"),
        (126, "level 20"),
        (127, "unknown"),
        (128, "emote"),
        (129, "sighted"),
        (130, "random"),
        (131, "ordinary"),
    ] {
        yell(&mut d, id, line);
    }
    let s = &mut d.scripts;
    s.conditions.insert(
        NOT_MY_SPAWN,
        Condition {
            entry: NOT_MY_SPAWN,
            kind: 52,
            values: [12_345, 0, 0, 0],
            flags: 0,
        },
    );
    s.conditions.insert(
        MY_SPAWN,
        Condition {
            entry: MY_SPAWN,
            kind: 52,
            values: [99, 0, 0, 0],
            flags: 0,
        },
    );
    // The source is a chief: swapped, the one it killed.
    s.conditions.insert(
        KILLED_A_CHIEF,
        Condition {
            entry: KILLED_A_CHIEF,
            kind: 16,
            values: [CHIEF as i32, 0, 0, 0],
            flags: CF_SWAP_TARGETS,
        },
    );
    s.conditions.insert(
        LEVEL_20,
        Condition {
            entry: LEVEL_20,
            kind: 15,
            values: [20, 0, 0, 0],
            flags: 0,
        },
    );
    s.conditions.insert(
        DEAD_TARGET,
        Condition {
            entry: DEAD_TARGET,
            kind: 46,
            values: [0; 4],
            flags: CF_REVERSE_RESULT,
        },
    );
    s.conditions.insert(
        MAGES,
        Condition {
            entry: MAGES,
            kind: 14,
            values: [0, 1 << 7, 0, 0],
            flags: 0,
        },
    );
    let events: Vec<(u32, Vec<AiEvent>)> = vec![
        (
            TIMERS,
            vec![
                ev(
                    EVENT_TIMER_OOC,
                    [1000, 1000, 5000, 5000],
                    EF_REPEATABLE,
                    ADD1,
                ),
                ev(EVENT_EVADE, [0; 4], EF_REPEATABLE, 100),
            ],
        ),
        (
            GUARDED,
            vec![AiEvent {
                condition: NOT_MY_SPAWN,
                ..ev(
                    EVENT_TIMER_OOC,
                    [1000, 1000, 60_000, 60_000],
                    EF_REPEATABLE,
                    ADD1,
                )
            }],
        ),
        (
            PAUSER,
            vec![
                ev(
                    EVENT_TIMER_IN_COMBAT,
                    [2000, 2000, 5000, 5000],
                    EF_REPEATABLE | EF_PAUSE_WHILE_STUNNED,
                    ADD1,
                ),
                ev(
                    EVENT_TIMER_IN_COMBAT,
                    [2000, 2000, 5000, 5000],
                    EF_REPEATABLE,
                    131,
                ),
            ],
        ),
        (
            AURAS,
            vec![
                ev(EVENT_AURA, [STACKS as i32, 2, 0, 0], 0, 101),
                ev(EVENT_MISSING_AURA, [ABSENT as i32, 1, 0, 0], 0, 102),
                ev(EVENT_TARGET_AURA, [ON_VICTIM as i32, 5, 0, 0], 0, 103),
                ev(
                    EVENT_TARGET_MISSING_AURA,
                    [NOT_ON_VICTIM as i32, 1, 0, 0],
                    0,
                    104,
                ),
                ev(
                    EVENT_VICTIM_ROOTED,
                    [10_000, 10_000, 0, 0],
                    EF_REPEATABLE,
                    105,
                ),
                ev(EVENT_TARGET_MANA, [25, 10, 0, 0], 0, 106),
                ev(EVENT_HP, [15, 0, 0, 0], 0, 107),
                ev(EVENT_FRIENDLY_IS_CC, [0, 30, 0, 0], 0, 108),
                ev(EVENT_FRIENDLY_MISSING_BUFF, [BUFF as i32, 30, 0, 0], 0, 109),
            ],
        ),
        (
            ROOTS,
            vec![ev(
                EVENT_VICTIM_ROOTED,
                [10_000, 10_000, 0, 0],
                EF_REPEATABLE,
                105,
            )],
        ),
        (
            EDGES,
            vec![
                ev(EVENT_HIT_BY_SPELL, [116, 0, 0, 0], EF_REPEATABLE, 110),
                ev(
                    EVENT_SPELL_HIT_TARGET,
                    [14_291, 0x7F, 0, 0],
                    EF_REPEATABLE,
                    111,
                ),
                ev(EVENT_SCRIPT_EVENT, [5862, 1, 0, 0], EF_REPEATABLE, 112),
                ev(EVENT_KILL, [0, 0, 1, 0], EF_REPEATABLE, 113),
                ev(EVENT_REACHED_HOME, [0; 4], EF_REPEATABLE, 114),
                ev(EVENT_SPAWNED, [0; 4], EF_REPEATABLE, 115),
                AiEvent {
                    inverse_phase_mask: 0b1,
                    ..ev(EVENT_SPAWNED, [0; 4], EF_REPEATABLE, 116)
                },
            ],
        ),
        (
            PHASED,
            vec![AiEvent {
                inverse_phase_mask: 0b10,
                ..ev(EVENT_SPAWNED, [0; 4], EF_REPEATABLE, 117)
            }],
        ),
        (
            MOVER,
            vec![ev(
                EVENT_MOVEMENT_INFORM,
                [2, 37, 100, 100],
                EF_REPEATABLE,
                118,
            )],
        ),
        (
            SUMMONER,
            vec![
                ev(
                    EVENT_SUMMONED_UNIT,
                    [MINION as i32, 100, 100, 0],
                    EF_REPEATABLE,
                    119,
                ),
                ev(
                    EVENT_SUMMONED_JUST_DIED,
                    [MINION as i32, 100, 100, 0],
                    EF_REPEATABLE,
                    120,
                ),
                ev(
                    EVENT_SUMMONED_JUST_DESPAWN,
                    [MINION as i32, 100, 100, 0],
                    EF_REPEATABLE,
                    121,
                ),
            ],
        ),
        (
            LISTENER,
            vec![ev(
                EVENT_GROUP_MEMBER_DIED,
                [CHIEF as i32, 1, 0, 0],
                EF_REPEATABLE,
                122,
            )],
        ),
        (
            SPAWNS_ITS,
            vec![
                AiEvent {
                    condition: MY_SPAWN,
                    ..ev(EVENT_SPAWNED, [0; 4], EF_REPEATABLE, 123)
                },
                AiEvent {
                    condition: NOT_MY_SPAWN,
                    ..ev(EVENT_SPAWNED, [0; 4], EF_REPEATABLE, 124)
                },
            ],
        ),
        (
            SPAWNS_OTHERS,
            vec![AiEvent {
                condition: MY_SPAWN,
                ..ev(EVENT_SPAWNED, [0; 4], EF_REPEATABLE, 123)
            }],
        ),
        (
            SWAPPER,
            vec![AiEvent {
                condition: KILLED_A_CHIEF,
                ..ev(EVENT_KILL, [0; 4], EF_REPEATABLE, 125)
            }],
        ),
        (
            LEVELLER,
            vec![AiEvent {
                condition: LEVEL_20,
                ..ev(EVENT_TIMER_IN_COMBAT, [0, 0, 0, 0], 0, 126)
            }],
        ),
        (
            UNKNOWN,
            vec![AiEvent {
                condition: NO_ROW,
                ..ev(EVENT_SPAWNED, [0; 4], EF_REPEATABLE, 127)
            }],
        ),
        (
            EMOTER,
            vec![ev(EVENT_RECEIVE_EMOTE, [78, 0, 0, 0], EF_REPEATABLE, 128)],
        ),
        (
            WATCHER,
            vec![AiEvent {
                condition: MAGES,
                ..ev(EVENT_OOC_LOS, [0, 40, 5000, 5000], EF_REPEATABLE, 129)
            }],
        ),
        (
            RANDOM,
            // Its first action's script is missing: it is no choice.
            vec![ev(
                EVENT_SCRIPT_EVENT,
                [1, 1, 0, 0],
                EF_REPEATABLE | EF_RANDOM_ACTION,
                0,
            )],
        ),
    ];
    for (entry, mut list) in events {
        for (i, e) in list.iter_mut().enumerate() {
            e.id = entry * 100 + i as u32;
            e.creature = entry;
        }
        if entry == RANDOM {
            list[0].actions = [9_999, 130, 0];
        }
        let faction = if entry == EMOTER { HUMANS } else { PACK };
        let name = format!("Creature {entry}");
        d.creature_templates
            .insert(entry, template(entry, &name, faction));
        d.scripts.ai_events.insert(entry, list);
    }
    for entry in [MINION, CHIEF, OTHER] {
        let name = format!("Creature {entry}");
        d.creature_templates
            .insert(entry, template(entry, &name, PACK));
    }
    // The summoned's death after the condition's: the summoner's event
    // only for a dead summon.
    if let Some(e) = d.scripts.ai_events.get_mut(&SUMMONER) {
        e.push(AiEvent {
            id: SUMMONER * 100 + 9,
            creature: SUMMONER,
            condition: DEAD_TARGET,
            ..ev(EVENT_SUMMONED_JUST_DIED, [MINION as i32, 0, 0, 0], 0, 120)
        });
    }
    // The chiefs' group: 11 leads, 12 and 13 follow, the listener too;
    // its members hear of any death.
    d.scripts.groups.insert(
        11,
        [11, 12, 13, 10]
            .into_iter()
            .map(|guid| GroupMember {
                guid,
                flags: GROUP_INFORM_MEMBERS_ON_ANY_DIED,
                ..Default::default()
            })
            .collect(),
    );
    for g in [11, 12, 13, 10] {
        d.scripts.group_of.insert(g, 11);
    }
    let spawns = SPAWNS
        .iter()
        .map(|(g, e, east, north)| {
            let p = at(*east, *north);
            CreatureSpawn {
                guid: *g,
                entries: vec![*e],
                map: 0,
                x: p.x,
                y: p.y,
                z: p.z,
                respawn_min: 60,
                respawn_max: 60,
                ..Default::default()
            }
        })
        .collect();
    d.creature_spawns.insert(0, spawns);
    d
}

async fn setup(class: Class) -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    // The map's first update before she comes: its creatures' spawned
    // events (yells) go then, to no one. Were she in first (the map takes
    // its first event or its first update as either comes), they could
    // reach her after the setup's clear, among a test's own yells.
    let map = realm.maps.get(&realm, MapKey::world(0));
    harness::an_update(&map).await;
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, class).await;
    a.heartbeat(START.0, START.1, START.2).await;
    a.sees(unit(EMOTER, 17)).await;
    // The heartbeat is in before a test reads where the player is.
    while map
        .query(move |m| {
            crate::game::map::combat::dist(m.players[&me].position(), at(0.0, 0.0)) > 0.01
        })
        .await
        .unwrap()
    {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    a.clear();
    (realm, a, me, map)
}

fn unit(entry: u32, spawn: u32) -> Guid {
    guid::unit(entry, spawn)
}

/// The yells heard, each with its target, until a pause.
async fn yells(c: &mut Client) -> Vec<(String, Guid)> {
    let mut out = Vec::new();
    while let Some(y) = c
        .gets(Duration::from_millis(300), |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => match &c.chat_type {
                SMSG_MESSAGECHAT_ChatType::MonsterYell { target, .. } => {
                    Some((c.message.clone(), *target))
                }
                _ => None,
            },
            _ => None,
        })
        .await
    {
        out.push(y);
    }
    out
}

/// The lines of `heard`, sorted.
fn said(heard: &[(String, Guid)]) -> Vec<&str> {
    let mut lines: Vec<&str> = heard.iter().map(|(l, _)| l.as_str()).collect();
    lines.sort_unstable();
    lines
}

/// `g`'s EventAI looks at its timed events at `t`.
fn tick_at(m: &mut Map, g: Guid, t: Instant) {
    m.now = t;
    event_ai::tick(m, g);
}

fn phase(m: &Map, g: Guid) -> u32 {
    event_ai::phase(m, g)
}

/// `g` in a fight with `victim`, its own EventAI told of it.
fn fight(m: &mut Map, g: Guid, victim: Guid) {
    ai::attacked(m, g, victim);
}

/// `g` out of its fight where it stands, its EventAI not told.
fn calm(m: &mut Map, g: Guid) {
    if let Some(c) = m.combat_mut(g) {
        *c = Default::default();
    }
    if let Some(c) = m.creature_mut(g) {
        c.threat.clear();
        c.fields.set_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            false,
        );
    }
}

/// Out-of-combat timers fire after their initial delay and again after
/// their repeat, never in a fight; an evade fires its evade events and
/// starts them over from their initial delay; and a timer whose condition
/// fails is left as it was.
#[tokio::test]
async fn out_of_combat_timers_run_from_their_delay_repeat_and_wait_out_a_fight() {
    let (_realm, mut a, me, map) = setup(Class::Warrior).await;
    let (timers, guarded) = (unit(TIMERS, 1), unit(GUARDED, 2));
    let out = map
        .query(move |m| {
            let t0 = m.now;
            let at = |ms: u64| t0 + Duration::from_millis(ms);
            let mut seen = Vec::new();
            for ms in [0, 1100, 2200, 6200] {
                tick_at(m, timers, at(ms));
                seen.push(phase(m, timers));
            }
            // In a fight, its out-of-combat timer waits.
            fight(m, timers, me);
            tick_at(m, timers, at(20_000));
            seen.push(phase(m, timers));
            // An evade, then its timer from its initial delay.
            calm(m, timers);
            event_ai::evade(m, timers);
            let due = event_ai::scripted(m, timers).and_then(|s| s.timers[0]);
            tick_at(m, timers, at(21_100));
            seen.push(phase(m, timers));
            // A condition that fails spends nothing.
            let timer = |m: &Map| event_ai::scripted(m, guarded).and_then(|s| s.timers[0]);
            let was = timer(m);
            for ms in [1100, 2200] {
                tick_at(m, guarded, at(ms));
            }
            let kept = timer(m) == was && was.is_some_and(|w| w <= at(1100));
            (
                seen,
                due.map(|d| d.duration_since(t0)),
                kept,
                phase(m, guarded),
            )
        })
        .await
        .unwrap();
    let (seen, due, kept, guarded) = out;
    assert_eq!(
        seen,
        [0, 1, 1, 2, 2, 3],
        "after its delay, again after its repeat, never in a fight, and again after an evade"
    );
    assert_eq!(
        due,
        Some(Duration::from_millis(21_000)),
        "its initial delay from the evade"
    );
    assert!(
        kept,
        "the failed condition's timer is as it was, though due"
    );
    assert_eq!(guarded, 0, "and it never fired");
    assert_eq!(said(&yells(&mut a).await), ["evade"]);
}

/// A stun holds the timers flagged to wait (only those, and by the time
/// it lasted, from one look to the next and from the fight's start), and
/// keeps what was left of a repeat; a new pull starts the clock again, so
/// the time out of the fight isn't counted as stunned.
#[tokio::test]
async fn a_stun_pauses_only_the_timers_flagged_to_wait() {
    let (_realm, mut a, me, map) = setup(Class::Warrior).await;
    let pauser = unit(PAUSER, 3);
    let stun = move |m: &mut Map, on: bool| {
        if on {
            aura_of(m, pauser, STUN, AURA_MOD_STUN, 0, 0);
        } else {
            auras::remove_spell(m, pauser, STUN, None);
        }
    };
    let seen = map
        .query(move |m| {
            let t0 = m.now;
            let at = |ms: u64| t0 + Duration::from_millis(ms);
            // Stunned as the fight starts, until ten seconds in.
            let mut seen = Vec::new();
            fight(m, pauser, me);
            stun(m, true);
            for ms in [500, 1000, 10_000] {
                tick_at(m, pauser, at(ms));
                seen.push(phase(m, pauser));
            }
            stun(m, false);
            for ms in [11_000, 12_000] {
                tick_at(m, pauser, at(ms));
                seen.push(phase(m, pauser));
            }
            // Out of the fight a while, then a new pull, stunned a second.
            calm(m, pauser);
            event_ai::evade(m, pauser);
            event_ai::set_phase(m, pauser, 0);
            m.now = at(50_000);
            fight(m, pauser, me);
            stun(m, true);
            tick_at(m, pauser, at(51_000));
            stun(m, false);
            for ms in [52_000, 53_000] {
                tick_at(m, pauser, at(ms));
                seen.push(phase(m, pauser));
            }
            calm(m, pauser);
            seen
        })
        .await
        .unwrap();
    assert_eq!(
        seen,
        [0, 0, 0, 0, 1, 0, 1],
        "the flagged timer waits out the stun, and only the stun"
    );
    assert_eq!(
        said(&yells(&mut a).await),
        ["ordinary", "ordinary"],
        "the other timer goes on through the stun"
    );
}

/// `stacks` of `spell` (a test aura) on `g`.
fn stacked(m: &mut Map, g: Guid, spell: u32, stacks: u32) {
    aura_of(m, g, spell, AURA_DUMMY, 0, 0);
    if let Some(h) =
        auras::auras_mut(m, g).and_then(|a| a.holders.iter_mut().find(|h| h.spell == spell))
    {
        h.stacks = stacks;
    }
}

/// The aura events go by the stacks of a spell on the creature or its
/// victim; a victim held by a root (not a stun) fires its event and starts
/// its repeat; the victim's mana fires within its bounds, the creature's
/// health not outside them; and a friend held, or missing a buff, is found
/// among those about, the creature first.
#[tokio::test]
async fn aura_mana_and_root_events_read_the_creature_and_its_victim() {
    let (_realm, mut a, me, map) = setup(Class::Mage).await;
    let (mob, roots) = (unit(AURAS, 4), unit(ROOTS, 5));
    let out = map
        .query(move |m| {
            let now = m.now;
            // Its victim stunned, not rooted.
            fight(m, roots, me);
            aura_of(m, me, STUN, AURA_MOD_STUN, 0, 0);
            tick_at(m, roots, now);
            let unrooted = event_ai::scripted(m, roots).and_then(|s| s.timers[0]);
            auras::remove_spell(m, me, STUN, None);
            calm(m, roots);
            // Two stacks of one spell on it, none of another; held; at
            // nine tenths of its health.
            fight(m, mob, me);
            stacked(m, mob, STACKS, 2);
            aura_of(m, mob, STUN, AURA_MOD_STUN, 0, 0);
            if let Some(f) = m.unit_fields_mut(mob) {
                let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
                f.set_u32(UNIT_FIELD_HEALTH, max * 9 / 10);
            }
            // Five on its victim, who is rooted and has a fifth of their
            // mana.
            stacked(m, me, ON_VICTIM, 5);
            aura_of(m, me, ROOT, AURA_MOD_ROOT, 0, 0);
            if let Some(f) = m.unit_fields_mut(me) {
                let max = f.get_u32(UNIT_FIELD_MAXPOWER1);
                f.set_u32(UNIT_FIELD_POWER1, max / 5);
            }
            tick_at(m, mob, now);
            let rooted = event_ai::scripted(m, mob).and_then(|s| s.timers[4]);
            auras::remove_spell(m, me, ROOT, None);
            calm(m, mob);
            (unrooted, rooted.map(|r| r.duration_since(now)))
        })
        .await
        .unwrap();
    let (unrooted, rooted) = out;
    assert_eq!(unrooted, None, "a stunned victim is no rooted one");
    assert_eq!(rooted, Some(Duration::from_secs(10)), "its repeat started");
    let heard = yells(&mut a).await;
    assert_eq!(
        said(&heard),
        [
            "aura",
            "friendly cc",
            "missing aura",
            "missing buff",
            "rooted",
            "target aura",
            "target mana",
            "target missing aura",
        ],
        "all but the health event"
    );
    for (line, target) in &heard {
        let expected = if line.starts_with("friendly") || line.starts_with("missing buff") {
            mob
        } else {
            me
        };
        assert_eq!(*target, expected, "{line} at its unit");
    }
}

/// The edge events: a spell's hit by its id, at its caster; a spell of its
/// own hitting by id and school, at the one it hit; a script's event by
/// its id and data, at the one the script gives; a kill of a player for
/// one that wants a player; reaching home; and being spawned, unless its
/// inverse phase mask names the phase it is in.
#[tokio::test]
async fn edge_events_match_their_parameters_and_fire_at_who_caused_them() {
    let (_realm, mut a, me, map) = setup(Class::Warrior).await;
    let (mob, phased) = (unit(EDGES, 6), unit(PHASED, 20));
    map.query(move |m| {
        event_ai::spell_hit(m, mob, me, 133, 0x04);
        event_ai::spell_hit(m, mob, me, 116, 0x10);
        event_ai::spell_hit(m, me, mob, 14_291, 0x04);
        event_ai::script_event(m, mob, 5862, 2, None);
        event_ai::script_event(m, mob, 5862, 1, Some(me));
        event_ai::kill(m, mob, unit(OTHER, 13));
        event_ai::kill(m, mob, me);
        event_ai::reached_home(m, mob);
        event_ai::spawned(m, mob);
        event_ai::update_despawns(m);
        // A mask of 0b10: phases 0 and 2 run it, 1 doesn't.
        for p in [0, 1, 2] {
            event_ai::set_phase(m, phased, p);
            event_ai::spawned(m, phased);
            event_ai::update_despawns(m);
        }
    })
    .await
    .unwrap();
    let heard = yells(&mut a).await;
    assert_eq!(
        said(&heard),
        [
            "hit by spell",
            "hit target",
            "kill",
            "phase 0 or 2",
            "phase 0 or 2",
            "reached home",
            "script event",
            "spawned",
        ]
    );
    for (line, target) in &heard {
        let expected = match line.as_str() {
            "hit by spell" | "hit target" | "script event" | "kill" => me,
            _ => Guid::zero(),
        };
        assert_eq!(*target, expected, "{line} at its unit");
    }
}

/// A point reached fires again only after its repeat (the third and
/// fourth parameters), and never for the dead; a summon's events, each
/// after its own repeat (the second and third), at the summon; and an
/// event whose condition fails is kept for when it holds.
#[tokio::test]
async fn movement_and_summon_events_repeat_after_their_own_cooldowns() {
    let (_realm, mut a, me, map) = setup(Class::Warrior).await;
    let (mover, summoner, minion) = (unit(MOVER, 7), unit(SUMMONER, 8), unit(MINION, 9));
    map.query(move |m| {
        ai::die(m, mover, me);
        event_ai::movement_inform(m, mover, 2, 37);
        ai::respawn_now(m, mover);
        if let Some(s) = event_ai::scripted_mut(m, minion) {
            s.summoner = Some(summoner);
        }
        let t0 = m.now;
        for ms in [0, 99, 100] {
            m.now = t0 + Duration::from_millis(ms);
            event_ai::movement_inform(m, mover, 2, 37);
            event_ai::summoned_unit(m, summoner, minion);
            event_ai::death_heard(m, minion);
            event_ai::summoned_despawned(m, summoner, minion, MINION);
        }
        // Dead, the summon fires the event that waited for that.
        m.now = t0 + Duration::from_millis(150);
        ai::die(m, minion, me);
    })
    .await
    .unwrap();
    let heard = yells(&mut a).await;
    assert_eq!(
        said(&heard),
        [
            "point",
            "point",
            "summon died",
            "summon died",
            "summon died",
            "summon gone",
            "summon gone",
            "summoned",
            "summoned",
        ]
    );
    assert!(
        heard
            .iter()
            .filter(|(l, _)| l.starts_with("summon"))
            .all(|(_, t)| *t == minion),
        "at the summon: {heard:?}"
    );
}

/// A group's member hears of a death by the dead one's entry, and of its
/// leader's only when the event asks for the leader.
#[tokio::test]
async fn a_group_member_dying_is_heard_by_its_entry_and_leadership() {
    let (_realm, mut a, me, map) = setup(Class::Warrior).await;
    let (leader, follower, other) = (unit(CHIEF, 11), unit(CHIEF, 12), unit(OTHER, 13));
    map.query(move |m| {
        ai::die(m, other, me);
        ai::die(m, follower, me);
        ai::die(m, leader, me);
    })
    .await
    .unwrap();
    assert_eq!(yells(&mut a).await, [("chief died".to_owned(), leader)]);
}

/// An event's condition: on its spawn, met and not; on the one that caused
/// it with the sides swapped (a kill of a chief); on its victim for a timed
/// event; and one with no row is never met.
#[tokio::test]
async fn an_events_condition_judges_its_spawn_its_cause_or_its_victim() {
    let (_realm, mut a, me, map) = setup(Class::Warrior).await;
    let (its, others, swapper, leveller, unknown) = (
        unit(SPAWNS_ITS, 99),
        unit(SPAWNS_OTHERS, 98),
        unit(SWAPPER, 14),
        unit(LEVELLER, 15),
        unit(UNKNOWN, 16),
    );
    let fired_at_level = map
        .query(move |m| {
            for g in [its, others, unknown] {
                event_ai::spawned(m, g);
            }
            event_ai::update_despawns(m);
            event_ai::kill(m, swapper, unit(OTHER, 13));
            event_ai::kill(m, swapper, unit(CHIEF, 11));
            // Its victim of level 1, then of 20.
            let t0 = m.now;
            fight(m, leveller, me);
            tick_at(m, leveller, t0);
            let before = event_ai::scripted(m, leveller).is_some_and(|s| s.disabled[0]);
            if let Some(f) = m.unit_fields_mut(me) {
                f.set_u32(UNIT_FIELD_LEVEL, 20);
            }
            tick_at(m, leveller, t0 + Duration::from_millis(1100));
            calm(m, leveller);
            before
        })
        .await
        .unwrap();
    assert!(!fired_at_level, "not for a victim of level 1");
    let heard = yells(&mut a).await;
    assert_eq!(said(&heard), ["killed a chief", "level 20", "my spawn"]);
    assert!(
        heard.contains(&("killed a chief".to_owned(), unit(CHIEF, 11))),
        "{heard:?}"
    );
}

/// A player's text emote at a creature fires its emote event, by the
/// emote, at the player.
#[tokio::test]
async fn a_text_emote_at_a_creature_fires_its_emote_event() {
    let (_realm, mut a, me, _map) = setup(Class::Warrior).await;
    let emoter = unit(EMOTER, 17);
    for emote in [101, 78] {
        a.send(CMSG_TEXT_EMOTE {
            text_emote: TextEmote::try_from(emote).unwrap(),
            emote: 0,
            target: emoter,
        })
        .await;
    }
    assert_eq!(yells(&mut a).await, [("emote".to_owned(), me)]);
}

/// Move `c` within its map at once.
async fn put(map: &MapHandle, c: &mut Client, to: Vector3d) {
    let g = c.guid;
    map.query(move |m| m.teleport(g, m.key, to, 0.0))
        .await
        .unwrap();
    c.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: c.teleport_order(),
        time: Default::default(),
    })
    .await;
}

/// An out-of-combat sighting fires at the nearest unit in its radius, in
/// sight, that meets its condition, each looked at in turn: a mage
/// farther off, not the warrior beside it.
#[tokio::test]
async fn an_out_of_combat_sighting_fires_at_the_nearest_that_meets_its_condition() {
    let (realm, mut a, _me, map) = setup(Class::Warrior).await;
    let watcher = unit(WATCHER, 18);
    let spot = map
        .query(move |m| m.position_of(watcher).unwrap())
        .await
        .unwrap();
    let mut b = Client::connect(&realm, "B");
    let mage = b.enter("Bea", Race::Human, Class::Mage).await;
    // Not game masters, whom creatures don't notice.
    let me = a.guid;
    map.query(move |m| {
        for g in [me, mage] {
            if let Some(p) = m.players.get_mut(&g) {
                p.gm = false;
            }
        }
    })
    .await
    .unwrap();
    put(
        &map,
        &mut a,
        Vector3d {
            x: spot.x - 5.0,
            ..spot
        },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(yells(&mut a).await, [], "the warrior beside it isn't one");
    put(
        &map,
        &mut b,
        Vector3d {
            x: spot.x - 30.0,
            ..spot
        },
    )
    .await;
    let sighted = a
        .wait_for("the sighting", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => match &c.chat_type {
                SMSG_MESSAGECHAT_ChatType::MonsterYell { target, .. } => Some(*target),
                _ => None,
            },
            _ => None,
        })
        .await;
    assert_eq!(sighted, mage, "the mage, farther off");
}

/// A random action is one of the event's scripts that exist: one whose
/// script is missing is no choice (Thistle Tea drops it as it loads).
#[tokio::test]
async fn a_random_action_is_one_of_the_scripts_that_exist() {
    let (_realm, mut a, _me, map) = setup(Class::Warrior).await;
    let random = unit(RANDOM, 19);
    map.query(move |m| {
        let t0 = m.now;
        for i in 0..20 {
            m.now = t0 + Duration::from_millis(i);
            event_ai::script_event(m, random, 1, 1, None);
        }
    })
    .await
    .unwrap();
    assert_eq!(yells(&mut a).await.len(), 20, "every time");
}

/// Its evade events run once its fight is over, so a spell they cast
/// isn't cut short by the evade.
#[tokio::test]
async fn an_evade_events_cast_outlives_the_evade() {
    let mut d = data();
    let mut chant = spell(96_010, AURA_DUMMY);
    chant.casting_time_index = 5;
    d.spells.cast_times.insert(5, 2000);
    d.spells.spells.insert(96_010, chant);
    d.scripts.ai_scripts.insert(
        140,
        vec![ScriptStep {
            id: 140,
            command: 15,
            datalong: [96_010, 0, 0, 0],
            data_flags: SF_TARGET_SELF,
            ..Default::default()
        }],
    );
    d.scripts
        .ai_events
        .insert(TIMERS, vec![ev(EVENT_EVADE, [0; 4], EF_REPEATABLE, 140)]);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let mob = unit(TIMERS, 1);
    let out = map
        .query(move |m| {
            fight(m, mob, me);
            ai::enter_evade(m, mob);
            let c = m.creature(mob).unwrap();
            (
                c.caster.casting.as_ref().map(|c| c.spell),
                c.fields.has_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                ),
            )
        })
        .await
        .unwrap();
    assert_eq!(out, (Some(96_010), false), "out of the fight, casting");
}

/// Its victim dead, a creature goes home, turns to its spawn's facing, and
/// its reached-home events fire there: a line, and a spell on itself (the
/// Wastewanders' stealth).
#[tokio::test]
async fn home_after_its_victim_dies_a_creature_fires_reached_home() {
    const HOMER: u32 = 4321;
    let mut d = data();
    d.creature_templates
        .insert(HOMER, template(HOMER, "Homer", PACK));
    d.spells.spells.insert(96_011, spell(96_011, AURA_DUMMY));
    d.scripts.ai_scripts.insert(
        141,
        vec![
            ScriptStep {
                id: 141,
                command: 15,
                datalong: [96_011, 0, 0, 0],
                data_flags: SF_TARGET_SELF,
                ..Default::default()
            },
            ScriptStep {
                id: 141,
                command: 0,
                dataint: [114, 0, 0, 0],
                ..Default::default()
            },
        ],
    );
    d.scripts
        .ai_events
        .insert(HOMER, vec![ev(EVENT_REACHED_HOME, [0; 4], 0, 141)]);
    let p = at(4.0, 0.0);
    d.creature_spawns.get_mut(&0).unwrap().push(CreatureSpawn {
        guid: 50,
        entries: vec![HOMER],
        map: 0,
        x: p.x,
        y: p.y,
        z: p.z,
        orientation: 2.5,
        respawn_min: 60,
        respawn_max: 60,
        ..Default::default()
    });
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let homer = unit(HOMER, 50);
    a.sees(homer).await;
    map.query(move |m| {
        ai::attacked(m, homer, me);
        // Turned to its victim.
        m.creature_mut(homer).unwrap().orientation = 1.0;
        crate::game::map::death::kill(m, me);
    })
    .await
    .unwrap();
    a.wait_for("its line at home", |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c) if c.message == "reached home" => Some(()),
        _ => None,
    })
    .await;
    let (facing, stealthed, fighting) = map
        .query(move |m| {
            let c = m.creature(homer).unwrap();
            (
                c.orientation,
                c.auras.holders.iter().any(|h| h.spell == 96_011),
                c.combat.attacking || c.evading,
            )
        })
        .await
        .unwrap();
    assert_eq!(facing, 2.5, "its spawn's facing");
    assert!(stealthed, "its spell on itself");
    assert!(!fighting);
}

/// Bug hunt (creatures): a creature that evades without walking (sessile,
/// or a follower at its place) never reached home: no reached-home events,
/// no turn to its spawn's facing, no faction back. It is home at once, as
/// mangos-classic's `HomeMovementGenerator` finalizes.
#[tokio::test]
async fn a_sessile_creature_that_evades_is_home_at_once() {
    const ROOTED: u32 = 4322;
    let mut d = data();
    let mut t = template(ROOTED, "Rooted", PACK);
    // Sessile (vmangos's `CREATURE_STATIC_FLAG_SESSILE`).
    t.static_flags1 |= 0x100;
    d.creature_templates.insert(ROOTED, t);
    d.scripts.ai_scripts.insert(
        142,
        vec![ScriptStep {
            id: 142,
            command: 0,
            dataint: [114, 0, 0, 0],
            ..Default::default()
        }],
    );
    d.scripts
        .ai_events
        .insert(ROOTED, vec![ev(EVENT_REACHED_HOME, [0; 4], 0, 142)]);
    let p = at(4.0, 0.0);
    d.creature_spawns.get_mut(&0).unwrap().push(CreatureSpawn {
        guid: 51,
        entries: vec![ROOTED],
        map: 0,
        x: p.x,
        y: p.y,
        z: p.z,
        orientation: 2.5,
        respawn_min: 60,
        respawn_max: 60,
        ..Default::default()
    });
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let rooted = unit(ROOTED, 51);
    a.sees(rooted).await;
    map.query(move |m| {
        ai::attacked(m, rooted, me);
        m.creature_mut(rooted).unwrap().orientation = 1.0;
        crate::game::map::death::kill(m, me);
    })
    .await
    .unwrap();
    a.wait_for("its line at home", |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c) if c.message == "reached home" => Some(()),
        _ => None,
    })
    .await;
    let facing = map
        .query(move |m| m.creature(rooted).unwrap().orientation)
        .await
        .unwrap();
    assert_eq!(facing, 2.5, "its spawn's facing");
}
