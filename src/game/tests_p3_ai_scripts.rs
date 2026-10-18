//! Phase 3's creature AI tests, scripts (Thistle Tea's `ai/script_test.exs`,
//! `script_failure_test.exs`, `script_run_test.exs` and their neighbours):
//! what a failed step ends, an EventAI event's retry when its actions fail,
//! and the script commands.
//!
//! As in `tests_p3_ai_events`, the creatures stand out of everyone's sight,
//! so a test drives them alone.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::*;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, MapKey, event_ai, scripts};
use crate::game::tests_p3_combat::{START, faction};

const HUMANS: u32 = 1;
const PACK: u32 = 14;
const FAR: f32 = 150.0;

/// A caster with mana, and others to run events and scripts.
const CASTER: u32 = 4400;
const ONE_SHOT: u32 = 4401;
const UNCHECKED: u32 = 4402;
const UNABORTED: u32 = 4403;
const GROUPS: u32 = 4404;
const RANDOM: u32 = 4405;
const NEVER: u32 = 4406;
const DIRECT: u32 = 4407;

/// An instant spell of 20 mana, on its caster.
const ZAP: u32 = 96_100;

/// The generic scripts of the tests.
const S_ABORTING: u32 = 1;
const S_GOING_ON: u32 = 2;
const S_DELAYED_FAILURE: u32 = 3;
const S_NESTED: u32 = 4;
const S_CHILD: u32 = 5;
const S_NO_VICTIM: u32 = 6;
const S_NO_VICTIM_ON: u32 = 7;
const S_CONDITION: u32 = 8;
const S_CONDITION_ON: u32 = 9;
const S_SWAP_INITIAL: u32 = 10;
const S_SWAP_INITIAL_ON: u32 = 11;
const S_SWAP_FINAL: u32 = 12;
const S_SWAP_FINAL_ON: u32 = 13;

/// The EventAI scripts: a cast that ends its script when it fails, the
/// same that doesn't, the cast then phase 1, phase 2, phase 2 five
/// seconds on.
const A_CAST: u32 = 100;
const A_CAST_ON: u32 = 101;
const A_CAST_PHASE: u32 = 102;
const A_PHASE_2: u32 = 103;
const A_LATE_PHASE: u32 = 104;

/// The source of entry 999: never met.
const NOT_999: u32 = 20;

fn phase_step(phase: u32) -> ScriptStep {
    ScriptStep {
        command: 44,
        datalong: [phase, 0, 0, 0],
        ..Default::default()
    }
}

fn cast_step(abort: bool) -> ScriptStep {
    ScriptStep {
        command: 15,
        datalong: [ZAP, 0, 0, 0],
        data_flags: SF_TARGET_SELF | if abort { SF_ABORT_ON_FAILURE } else { 0 },
        ..Default::default()
    }
}

fn delayed(step: ScriptStep, ms: u32) -> ScriptStep {
    ScriptStep {
        delay_ms: ms,
        ..step
    }
}

fn aborting(step: ScriptStep, abort: bool) -> ScriptStep {
    ScriptStep {
        data_flags: step.data_flags | if abort { SF_ABORT_ON_FAILURE } else { 0 },
        ..step
    }
}

fn template(entry: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: format!("Creature {entry}"),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 10,
        level_max: 10,
        faction: PACK,
        unit_class: 2,
        health_multiplier: 1.0,
        mana_multiplier: 1.0,
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

/// An out-of-combat timer due at once, then each minute, running `actions`.
fn timer(flags: u32, actions: [u32; 3]) -> AiEvent {
    AiEvent {
        kind: EVENT_TIMER_OOC,
        chance: 100,
        flags,
        params: [0, 0, 60_000, 60_000],
        actions,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.faction_templates.extend([
        (HUMANS, faction(HUMANS, 0x1, 0x2)),
        (PACK, faction(PACK, 0x2, 0x1)),
    ]);
    d.class_level_stats.insert(
        (2, 10),
        ClassLevelStats {
            health: 100,
            mana: 100,
            melee_damage: 1.0,
            ..Default::default()
        },
    );
    let mut zap = SpellTemplate {
        id: ZAP,
        name: "Zap".into(),
        equipped_item_class: -1,
        mana_cost: 20,
        ..Default::default()
    };
    zap.effects[0] = SpellEffect {
        effect: 6,
        aura: crate::game::map::auras::AURA_DUMMY,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(ZAP, zap);
    let s = &mut d.scripts;
    s.conditions.insert(
        NOT_999,
        Condition {
            entry: NOT_999,
            kind: 16,
            values: [999, 0, 0, 0],
            flags: 0,
        },
    );
    let generic: Vec<(u32, Vec<ScriptStep>)> = vec![
        (
            S_ABORTING,
            vec![cast_step(true), phase_step(2), delayed(phase_step(3), 1000)],
        ),
        (
            S_GOING_ON,
            vec![
                cast_step(false),
                phase_step(2),
                delayed(phase_step(3), 1000),
            ],
        ),
        (
            S_DELAYED_FAILURE,
            vec![delayed(cast_step(true), 1000), delayed(phase_step(2), 2000)],
        ),
        (
            S_NESTED,
            vec![
                ScriptStep {
                    command: 39,
                    datalong: [S_CHILD, 0, 0, 0],
                    dataint: [100, 0, 0, 0],
                    ..Default::default()
                },
                phase_step(2),
            ],
        ),
        (S_CHILD, vec![cast_step(true), phase_step(1)]),
    ];
    // A step's unit, condition or swap not found: its script ends only
    // with the abort flag.
    let no_victim = ScriptStep {
        target_type: 1,
        data_flags: SF_TARGET_SELF,
        ..phase_step(1)
    };
    let condition = ScriptStep {
        condition: NOT_999,
        ..phase_step(1)
    };
    let swap_initial = ScriptStep {
        data_flags: SF_SWAP_INITIAL_TARGETS,
        ..phase_step(1)
    };
    let swap_final = ScriptStep {
        data_flags: SF_SWAP_FINAL_TARGETS,
        ..phase_step(1)
    };
    let mut failing = Vec::new();
    for (ids, step) in [
        ((S_NO_VICTIM, S_NO_VICTIM_ON), no_victim),
        ((S_CONDITION, S_CONDITION_ON), condition),
        ((S_SWAP_INITIAL, S_SWAP_INITIAL_ON), swap_initial),
        ((S_SWAP_FINAL, S_SWAP_FINAL_ON), swap_final),
    ] {
        failing.push((ids.0, vec![aborting(step.clone(), true), phase_step(2)]));
        failing.push((ids.1, vec![aborting(step, false), phase_step(2)]));
    }
    for (id, steps) in generic.into_iter().chain(failing) {
        s.generic.insert(id, steps);
    }
    for (id, steps) in [
        (A_CAST, vec![cast_step(true)]),
        (A_CAST_ON, vec![cast_step(false)]),
        (A_CAST_PHASE, vec![cast_step(true), phase_step(1)]),
        (A_PHASE_2, vec![phase_step(2)]),
        (A_LATE_PHASE, vec![delayed(phase_step(2), 5000)]),
    ] {
        s.ai_scripts.insert(id, steps);
    }
    for (entry, event) in [
        (ONE_SHOT, timer(EF_CHECK_RESULT, [A_CAST, 0, 0])),
        (UNCHECKED, timer(0, [A_CAST, 0, 0])),
        (UNABORTED, timer(EF_CHECK_RESULT, [A_CAST_ON, 0, 0])),
        (GROUPS, timer(EF_CHECK_RESULT, [A_CAST_PHASE, A_PHASE_2, 0])),
        (
            RANDOM,
            timer(EF_CHECK_RESULT | EF_RANDOM_ACTION, [A_CAST, A_PHASE_2, 0]),
        ),
        (
            NEVER,
            AiEvent {
                chance: 0,
                ..timer(EF_CHECK_RESULT, [A_CAST, 0, 0])
            },
        ),
        (DIRECT, timer(0, [A_LATE_PHASE, 0, 0])),
    ] {
        s.ai_events.insert(
            entry,
            vec![AiEvent {
                id: entry,
                creature: entry,
                ..event
            }],
        );
    }
    let entries = [
        CASTER, ONE_SHOT, UNCHECKED, UNABORTED, GROUPS, RANDOM, NEVER, DIRECT,
    ];
    let mut spawns = Vec::new();
    for (i, entry) in entries.into_iter().enumerate() {
        d.creature_templates.insert(entry, template(entry));
        let p = at(FAR, i as f32 * 10.0);
        spawns.push(CreatureSpawn {
            guid: i as u32 + 1,
            entries: vec![entry],
            map: 0,
            x: p.x,
            y: p.y,
            z: p.z,
            respawn_min: 60,
            respawn_max: 60,
            ..Default::default()
        });
    }
    d.creature_spawns.insert(0, spawns);
    d
}

async fn setup() -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    (realm, a, me, map)
}

/// The creature of `entry` (the `n`th spawn).
fn mob(entry: u32) -> Guid {
    let n = [
        CASTER, ONE_SHOT, UNCHECKED, UNABORTED, GROUPS, RANDOM, NEVER, DIRECT,
    ]
    .iter()
    .position(|e| *e == entry)
    .unwrap() as u32;
    guid::unit(entry, n + 1)
}

fn mana(m: &mut Map, g: Guid, to: u32) {
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_POWER1, to);
    }
}

fn phase(m: &Map, g: Guid) -> u32 {
    event_ai::phase(m, g)
}

/// `g`'s scripts still waiting to run a step.
fn waiting(m: &Map, g: Guid) -> usize {
    m.scripts
        .runs
        .iter()
        .filter(|r| r.source == g && r.next != usize::MAX)
        .count()
}

/// A step that fails (a cast short of mana, a unit or condition not found,
/// a swap with no one to swap with) ends its script, its delayed steps
/// too, only with the abort flag; and a script it started that fails ends
/// alone, its parent going on.
#[tokio::test]
async fn a_failed_step_ends_its_script_only_with_the_abort_flag() {
    let (_realm, _a, _me, map) = setup().await;
    let caster = mob(CASTER);
    let out = map
        .query(move |m| {
            let mut out = Vec::new();
            let run = |m: &mut Map, script: u32| {
                event_ai::set_phase(m, caster, 0);
                scripts::start(m, ScriptTable::Generic, script, caster, Guid::zero());
                (phase(m, caster), waiting(m, caster))
            };
            mana(m, caster, 0);
            out.push(run(m, S_ABORTING));
            out.push(run(m, S_GOING_ON));
            // Its delayed step goes on, a second later.
            let t0 = m.now;
            m.now = t0 + Duration::from_millis(1000);
            scripts::update(m);
            out.push((phase(m, caster), waiting(m, caster)));
            for script in [
                S_NO_VICTIM,
                S_NO_VICTIM_ON,
                S_CONDITION,
                S_CONDITION_ON,
                S_SWAP_INITIAL,
                S_SWAP_INITIAL_ON,
                S_SWAP_FINAL,
                S_SWAP_FINAL_ON,
            ] {
                out.push(run(m, script));
            }
            // A cast a second on fails, and the step a second after it
            // never runs.
            out.push(run(m, S_DELAYED_FAILURE));
            for ms in [2000, 3000, 4000] {
                m.now = t0 + Duration::from_millis(ms);
                scripts::update(m);
            }
            out.push((phase(m, caster), waiting(m, caster)));
            out.push(run(m, S_NESTED));
            out
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        [
            (0, 0),
            (2, 1),
            (3, 0),
            (0, 0),
            (2, 0),
            (0, 0),
            (2, 0),
            (0, 0),
            (2, 0),
            (0, 0),
            (2, 0),
            (0, 1),
            (0, 0),
            (2, 0),
        ],
        "each failure ends its script only with the flag; a child's ends only the child"
    );
}

/// EventAI's result check: an event whose actions fail (a cast short of
/// mana that ends its script) is due again at once and stays enabled,
/// until it succeeds and is spent; it takes both the abort flag and the
/// check; its other actions still run and don't undo the failure; a random
/// event checks only the action it chose; a missed chance spends it; and
/// an action's steps run at once, whatever their delay.
#[tokio::test]
async fn a_checked_event_whose_actions_fail_fires_again_until_they_succeed() {
    let (_realm, _a, _me, map) = setup().await;
    let out = map
        .query(move |m| {
            let t0 = m.now;
            let at = |ms: u64| t0 + Duration::from_millis(ms);
            let state = |m: &Map, g: Guid| {
                event_ai::scripted(m, g)
                    .map(|s| {
                        (
                            s.disabled[0],
                            s.timers[0].map(|t| t.saturating_duration_since(t0).as_millis()),
                        )
                    })
                    .unwrap()
            };
            let tick = |m: &mut Map, g: Guid, ms: u64| {
                m.now = at(ms);
                event_ai::tick(m, g);
            };
            for e in [ONE_SHOT, UNCHECKED, UNABORTED, GROUPS, RANDOM, NEVER] {
                mana(m, mob(e), 0);
            }
            let one = mob(ONE_SHOT);
            tick(m, one, 0);
            let failed = state(m, one);
            mana(m, one, 100);
            tick(m, one, 1000);
            let cast = state(m, one);
            let spent = m
                .unit_fields(one)
                .map(|f| f.get_u32(UNIT_FIELD_POWER1))
                .unwrap();
            tick(m, one, 100_000);
            let again = m
                .unit_fields(one)
                .map(|f| f.get_u32(UNIT_FIELD_POWER1))
                .unwrap();
            let mut halves = Vec::new();
            for e in [UNCHECKED, UNABORTED] {
                tick(m, mob(e), 0);
                halves.push(state(m, mob(e)));
            }
            let groups = mob(GROUPS);
            tick(m, groups, 0);
            let groups = (phase(m, groups), state(m, groups));
            // A random event, many times over: spent exactly when its
            // choice (phase 2) succeeded.
            let random = mob(RANDOM);
            let mut seen = [false; 2];
            let mut agree = true;
            for i in 0..30 {
                event_ai::set_phase(m, random, 0);
                // Its events' state is there once its script has started,
                // at its first tick or the map's next update, whichever
                // came first.
                if let Some(s) = event_ai::scripted_mut(m, random) {
                    if let Some(d) = s.disabled.get_mut(0) {
                        *d = false;
                    }
                    if let Some(t) = s.timers.get_mut(0) {
                        *t = None;
                    }
                    s.next_tick = None;
                }
                tick(m, random, i * 10);
                let succeeded = phase(m, random) == 2;
                seen[usize::from(succeeded)] = true;
                agree &= state(m, random).0 == succeeded;
            }
            let never = mob(NEVER);
            tick(m, never, 0);
            let never = (
                state(m, never),
                m.unit_fields(never).map(|f| f.get_u32(UNIT_FIELD_POWER1)),
            );
            let direct = mob(DIRECT);
            tick(m, direct, 0);
            let direct = phase(m, direct);
            (
                failed, cast, spent, again, halves, groups, seen, agree, never, direct,
            )
        })
        .await
        .unwrap();
    let (failed, cast, spent, again, halves, groups, seen, agree, never, direct) = out;
    assert_eq!(
        failed,
        (false, Some(0)),
        "failed: due again at once, enabled"
    );
    assert!(cast.0, "cast: spent");
    assert_eq!(spent, 80, "its mana paid");
    assert_eq!(again, 80, "and not again");
    assert_eq!(
        halves,
        [(true, Some(60_000)), (true, Some(60_000))],
        "no retry without both the check and the abort"
    );
    assert_eq!(
        groups,
        (2, (false, Some(0))),
        "the other action ran, the failure stands"
    );
    assert!(agree && seen == [true, true], "random: {seen:?} {agree}");
    assert_eq!(
        never,
        ((true, Some(60_000)), Some(0)),
        "a missed chance spends it, casting nothing"
    );
    assert_eq!(direct, 2, "its step at once");
}
