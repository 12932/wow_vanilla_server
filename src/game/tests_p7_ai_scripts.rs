//! Round 7's creature AI tests of scripts, ported from Thistle Tea's
//! `test/game/core/ai/script_targets_test.exs`,
//! `script/hostile_target_test.exs`, `script_test.exs`,
//! `script/move_to_test.exs`, `creature_spell_list_test.exs` and
//! `bt/mob/spells_test.exs` (docs/parity/matrix.csv): whom a step runs
//! from and on, the hostile selectors' ranges, a move to a target, and a
//! spell list a script sets.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::scripts::*;
use crate::game::data::spells::{CreatureSpell, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::combat::dist;
use crate::game::map::{Map, ai, scripts};
use crate::game::tests_p3_combat::{BRUTE, START, data, setup_with, spawn};
use crate::game::tests_p3_entity::step;

fn at(east: f32, north: f32, up: f32) -> Vector3d {
    Vector3d {
        x: START.0 + east,
        y: START.1 + north,
        z: START.2 + up,
    }
}

/// Unit `g` put at `p`, with a reach of 1.5 and a bounding radius of 1.
fn put(m: &mut Map, g: Guid, p: Vector3d) {
    if let Some(pl) = m.players.get_mut(&g) {
        pl.movement.position = p;
    } else {
        let c = m.creature_mut(g).unwrap();
        let old = c.cell;
        c.position = p;
        c.cell = crate::game::map::grid::cell_of(p.x, p.y);
        let new = c.cell;
        m.grid.relocate(g, old, new);
    }
    let f = m.unit_fields_mut(g).unwrap();
    f.set_f32(UNIT_FIELD_COMBATREACH, 1.5);
    f.set_f32(UNIT_FIELD_BOUNDINGRADIUS, 1.0);
}

/// A bolt of 40 yards; one of 10 to 40; one of 40 that takes three
/// seconds.
const BOLT: u32 = 95_310;
const NEAR_BOLT: u32 = 95_311;
const SLOW_BOLT: u32 = 95_313;
/// A spell list of the bolt, five seconds before its first.
const LIST: u32 = 9_531;
const LIST_B: u32 = 9_532;

fn world() -> GameData {
    let mut d = data();
    // A forty-yard range (index 4; index 2 is 1.12's melee "Combat Range").
    d.spells.ranges.insert(4, (0.0, 40.0));
    d.spells.ranges.insert(3, (10.0, 40.0));
    d.spells.cast_times.insert(1, 0);
    d.spells.cast_times.insert(99, 3_000);
    for (id, range, time) in [(BOLT, 4, 1), (NEAR_BOLT, 3, 1), (SLOW_BOLT, 4, 99)] {
        let mut s = SpellTemplate {
            id,
            name: format!("Bolt {id}"),
            range_index: range,
            casting_time_index: time,
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 2,
            base_points: 0,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    for list in [LIST, LIST_B] {
        d.spells.creature_spells.insert(
            list,
            vec![CreatureSpell {
                spell: BOLT,
                probability: 100,
                delay_initial: (5_000, 5_000),
                delay_repeat: (5_000, 5_000),
                ..Default::default()
            }],
        );
    }
    d
}

async fn player(realm: &std::sync::Arc<crate::game::Realm>, account: &str) -> (Client, Guid) {
    let mut c = Client::connect(realm, account);
    let name = format!("{}{}", &account[..1], account[1..].to_lowercase());
    let g = c.enter(&name, Race::Human, Class::Warrior).await;
    (c, g)
}

fn flags(s: ScriptStep, data_flags: u32) -> ScriptStep {
    ScriptStep { data_flags, ..s }
}

/// Thistle Tea's `Script.run` "orders initial swap, selection, final
/// swap, and self targeting" (the provided target and the victim, under
/// each of the eight sets of the three flags), "an absent provided target
/// stays absent despite an active victim", "both swaps can select a buddy
/// when the initial target is absent" and "game objects can own an
/// initially swapped command": a step's initial swap hands it to its
/// target, which its selector then works from; a final swap hands it to
/// whom it selected, on the one it came from; the self flag makes the one
/// it ends with its target too. A victim is a player's too, once a swap
/// has handed the step to her. No one provided is no one; with both
/// swaps and no one there, a buddy by its spawn takes the step, on no one
/// (or itself); a selector that needs someone to start from finds none.
/// An object handed a step by the initial swap runs it.
#[tokio::test]
async fn a_steps_pair_comes_of_its_initial_swap_its_selector_its_final_swap_and_its_self_flag() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            let s = spawn(m, ga, BRUTE, 20, 5.0);
            let sv = spawn(m, ga, BRUTE, 20, 6.0);
            let tv = spawn(m, ga, BRUTE, 20, 7.0);
            m.creature_mut(s).unwrap().combat.victim = sv;
            let p = m.players.get_mut(&ga).unwrap();
            p.combat.victim = tv;
            p.combat.attacking = true;
            let t = ga;
            let cases = [
                (0, 0, (s, t)),
                (0, 1, (t, s)),
                (0, 2, (t, s)),
                (0, 3, (s, t)),
                (0, 4, (s, s)),
                (0, 5, (t, t)),
                (0, 6, (t, t)),
                (0, 7, (s, s)),
                (1, 0, (s, sv)),
                (1, 1, (t, tv)),
                (1, 2, (sv, s)),
                (1, 3, (tv, t)),
                (1, 4, (s, s)),
                (1, 5, (t, t)),
                (1, 6, (sv, sv)),
                (1, 7, (tv, tv)),
            ];
            let wrong: Vec<_> = cases
                .iter()
                .filter_map(|(target_type, f, want)| {
                    let st = ScriptStep {
                        target_type: *target_type,
                        data_flags: *f,
                        ..step(0, [0; 4])
                    };
                    let got = scripts::resolve_targets(m, &st, s, t);
                    (got != Some(*want)).then_some((*target_type, *f, got))
                })
                .collect();
            // No one provided, whatever its victim.
            let absent = scripts::resolve_targets(m, &step(0, [0; 4]), s, Guid::zero());
            // Both swaps, no one there: its buddy (spawn 77).
            m.by_spawn.insert(77, sv);
            let buddy = |target_type: u32, f: u32| ScriptStep {
                target_type,
                target_param1: 77,
                data_flags: SF_SWAP_INITIAL_TARGETS | SF_SWAP_FINAL_TARGETS | f,
                ..step(0, [0; 4])
            };
            let buddies = (
                scripts::resolve_targets(m, &buddy(11, 0), s, Guid::zero()),
                scripts::resolve_targets(m, &buddy(11, SF_TARGET_SELF), s, Guid::zero()),
                [1, 8, 25].map(|tt| scripts::resolve_targets(m, &buddy(tt, 0), s, Guid::zero())),
            );
            // An object handed its animation.
            let mut go = crate::game::map::gameobject::GameObject::from_spawn(
                &crate::game::data::GameObjectTemplate {
                    entry: 4_242,
                    name: "Thing".into(),
                    ..Default::default()
                },
                &crate::game::data::GameObjectSpawn {
                    guid: 0x80_1234,
                    entry: 4_242,
                    x: START.0,
                    y: START.1,
                    z: START.2,
                    ..Default::default()
                },
            );
            go.spawn_guid = None;
            let o = m.add_gameobject(go);
            let object = scripts::resolve_targets(
                m,
                &flags(step(89, [2, 0, 0, 0]), SF_SWAP_INITIAL_TARGETS),
                s,
                o,
            );
            (wrong, absent, buddies, object, (s, sv, o))
        })
        .await
        .unwrap();
    let (wrong, absent, buddies, object, (s, sv, o)) = out;
    assert!(wrong.is_empty(), "these resolve otherwise: {wrong:?}");
    assert_eq!(absent, Some((s, Guid::zero())), "no one provided is no one");
    assert_eq!(buddies.0, Some((sv, Guid::zero())), "its buddy, on no one");
    assert_eq!(buddies.1, Some((sv, sv)), "or on itself");
    assert_eq!(buddies.2, [None; 3], "no one to start from");
    assert_eq!(object, Some((o, s)), "the object runs it");
}

fn stand_of(m: &Map, g: Guid) -> u8 {
    m.unit_fields(g)
        .map_or(u8::MAX, |f| f.get_byte(UNIT_FIELD_BYTES_1, 0))
}

/// Thistle Tea's `Script.run` "initial swaps defer selectors and
/// conditions to the new source", "conditions use the selected target
/// even without a swap", "a missing selector skips self-targeted
/// commands", "failed target resolution or conditions cannot terminate a
/// script" and "delayed swaps retain the original pair until the delay
/// elapses": a step's condition is judged on the pair it resolves to, the
/// unit its selector chose its target (a swapped condition's source); a
/// step whose selector finds no one is skipped, its script going on; a
/// terminating step skipped so ends nothing; a delayed swap is the pair
/// the script started with, when its time comes.
#[tokio::test]
async fn a_steps_condition_is_judged_on_its_resolved_pair_and_a_skipped_step_ends_no_script() {
    const SOURCE_IS: u32 = 9_530;
    const NEVER: u32 = 9_531;
    let mut d = world();
    let c = |entry, values: [i32; 4], flags| Condition {
        entry,
        kind: 16,
        values,
        flags,
    };
    d.scripts.conditions.insert(
        SOURCE_IS,
        c(SOURCE_IS, [BRUTE as i32, 0, 0, 0], CF_SWAP_TARGETS),
    );
    d.scripts
        .conditions
        .insert(NEVER, c(NEVER, [999, 0, 0, 0], 0));
    // A terminate that can't find its unit, or fails its condition, then
    // a sit; a sit after a second, swapped.
    d.scripts.generic.insert(
        1,
        vec![
            ScriptStep {
                target_type: 11,
                target_param1: 999,
                ..step(31, [BRUTE, 30, 0, 0])
            },
            step(28, [1, 0, 0, 0]),
        ],
    );
    d.scripts.generic.insert(
        2,
        vec![
            ScriptStep {
                condition: NEVER,
                ..step(31, [BRUTE, 30, 0, 0])
            },
            step(28, [1, 0, 0, 0]),
        ],
    );
    d.scripts.generic.insert(
        3,
        vec![ScriptStep {
            delay_ms: 1_000,
            ..flags(step(28, [8, 0, 0, 0]), SF_SWAP_INITIAL_TARGETS)
        }],
    );
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let out = map
        .query(move |m| {
            let s = spawn(m, ga, BRUTE, 20, 5.0);
            let sv = spawn(m, ga, BRUTE, 20, 6.0);
            let tv = spawn(m, ga, BRUTE, 20, 7.0);
            m.creature_mut(s).unwrap().combat.victim = sv;
            let p = m.players.get_mut(&ga).unwrap();
            p.combat.victim = tv;
            p.combat.attacking = true;
            let run = |m: &mut Map, st: ScriptStep, target: Guid| {
                for g in [s, ga] {
                    if let Some(p) = m.players.get_mut(&g) {
                        p.set_stand_state(0);
                    } else {
                        m.unit_fields_mut(g)
                            .unwrap()
                            .set_byte(UNIT_FIELD_BYTES_1, 0, 0);
                    }
                }
                scripts::execute(m, &st, s, target);
                (stand_of(m, s), stand_of(m, ga))
            };
            // The victim's entry, judged on the victim: the creature sits.
            let selected = run(
                m,
                ScriptStep {
                    target_type: 1,
                    condition: SOURCE_IS,
                    ..step(28, [1, 0, 0, 0])
                },
                ga,
            );
            // Swapped to her: her victim judged, and she kneels.
            let swapped = run(
                m,
                ScriptStep {
                    target_type: 1,
                    condition: SOURCE_IS,
                    ..flags(step(28, [8, 0, 0, 0]), SF_SWAP_INITIAL_TARGETS)
                },
                ga,
            );
            let failed = run(
                m,
                ScriptStep {
                    target_type: 1,
                    condition: NEVER,
                    ..flags(step(28, [8, 0, 0, 0]), SF_SWAP_INITIAL_TARGETS)
                },
                ga,
            );
            // A unit not there, the self flag on: skipped, the script on.
            let missing = run(
                m,
                ScriptStep {
                    target_type: 11,
                    target_param1: 999,
                    ..flags(step(28, [1, 0, 0, 0]), SF_TARGET_SELF)
                },
                ga,
            );
            let skipped_ok = scripts::execute(
                m,
                &ScriptStep {
                    target_type: 11,
                    target_param1: 999,
                    ..flags(step(28, [1, 0, 0, 0]), SF_TARGET_SELF)
                },
                s,
                ga,
            );
            // The terminates skipped end nothing.
            let mut tails = Vec::new();
            for id in [1, 2] {
                m.unit_fields_mut(s)
                    .unwrap()
                    .set_byte(UNIT_FIELD_BYTES_1, 0, 0);
                scripts::start(m, ScriptTable::Generic, id, s, ga);
                tails.push(stand_of(m, s));
            }
            // A swap a second off: her, when it comes.
            m.players.get_mut(&ga).unwrap().set_stand_state(0);
            m.unit_fields_mut(s)
                .unwrap()
                .set_byte(UNIT_FIELD_BYTES_1, 0, 0);
            scripts::start(m, ScriptTable::Generic, 3, s, ga);
            let before = stand_of(m, ga);
            for r in m.scripts.runs.iter_mut() {
                r.started -= Duration::from_millis(1_000);
            }
            scripts::update(m);
            let after = (stand_of(m, ga), stand_of(m, s));
            (
                selected,
                swapped,
                failed,
                missing,
                skipped_ok,
                tails,
                (before, after),
            )
        })
        .await
        .unwrap();
    let (selected, swapped, failed, missing, skipped_ok, tails, delayed) = out;
    assert_eq!(selected, (1, 0), "judged on the victim it chose");
    assert_eq!(swapped, (0, 8), "swapped, judged on hers");
    assert_eq!(failed, (0, 0), "failed, nothing");
    assert_eq!(missing, (0, 0), "no one found: skipped");
    assert!(skipped_ok, "and its script goes on");
    assert_eq!(tails, [1, 1], "a terminate skipped ends nothing");
    assert_eq!(delayed, (0, (8, 0)), "the swap when its time comes");
}

/// Thistle Tea's hostile selectors: "filters spell range, line of sight
/// and player flags before choosing", "filters minimum spell range and
/// other copies before random selection", "selector range includes the
/// minimum and excludes the maximum", "farthest ranking uses combat reach
/// rather than center distance" and "checks horizontal surface distance
/// after choosing the farthest"; and `nearest hostile player passes over
/// dead players`: a cast's hostile target is a player (with 0x2) within
/// the spell's range, measured between the units' reaches, from its
/// least (taken) to its most (not); the farthest is the farthest by that
/// measure; a step's flat least distance is judged on the one chosen,
/// with no other tried; the nearest hostile player is a living one.
#[tokio::test]
async fn a_casts_hostile_target_is_in_its_spells_range_between_reaches_and_farthest_by_them() {
    let (realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (_b, gb) = player(&realm, "BOB").await;
    let (_c, gc) = player(&realm, "CID").await;
    let out = map
        .query(move |m| {
            let s = spawn(m, ga, BRUTE, 20, 5.0);
            let n = spawn(m, ga, BRUTE, 20, 6.0);
            put(m, s, at(0.0, 0.0, 0.0));
            let pick = |m: &mut Map,
                        target_type: u32,
                        spell: u32,
                        player_only: bool,
                        units: &[(Guid, f32, f32)]| {
                for (g, east, up) in units {
                    put(m, *g, at(*east, 0.0, *up));
                }
                m.creature_mut(s).unwrap().threat = units.iter().map(|(g, ..)| (*g, 1.0)).collect();
                let st = ScriptStep {
                    target_type,
                    target_param1: if player_only { 0x2 } else { 0 },
                    ..step(15, [spell, 0x2, 0, 0])
                };
                scripts::resolve_targets(m, &st, s, Guid::zero()).map(|(_, t)| t)
            };
            // Out of range, an NPC, and a player in range.
            let first = pick(
                m,
                7,
                BOLT,
                true,
                &[(ga, 50.0, 0.0), (n, 30.0, 0.0), (gb, 20.0, 0.0)],
            );
            // Within the least range, and outside it: at random, always
            // the one outside.
            let randoms: Vec<_> = (0..20)
                .map(|_| pick(m, 4, NEAR_BOLT, true, &[(ga, 5.0, 0.0), (gb, 15.0, 0.0)]))
                .collect();
            // The edges, 1.5 and 1.5 of reach: 13 to 43, the last not.
            let edges = [12.99, 13.0, 42.99, 43.0]
                .map(|east| pick(m, 7, NEAR_BOLT, true, &[(gb, east, 0.0)]));
            // The farthest by reach: a big one at 20 is nearer than her at
            // 18.
            let farthest = {
                put(m, gb, at(18.0, 0.0, 0.0));
                put(m, n, at(20.0, 0.0, 0.0));
                m.unit_fields_mut(n)
                    .unwrap()
                    .set_f32(UNIT_FIELD_COMBATREACH, 12.0);
                m.creature_mut(s).unwrap().threat = vec![(n, 1.0), (gb, 1.0)];
                let st = ScriptStep {
                    target_type: 7,
                    ..step(15, [BOLT, 0x2, 0, 0])
                };
                scripts::resolve_targets(m, &st, s, Guid::zero()).map(|(_, t)| t)
            };
            // The farthest, high above, judged flat from the creature's
            // edge: 7 yards or less, and no one; past it, her.
            let mut flat = Vec::new();
            for east in [6.0, 9.0, 9.01] {
                put(m, ga, at(14.0, 0.0, 0.0));
                put(m, gb, at(east, 0.0, 30.0));
                m.creature_mut(s).unwrap().threat = vec![(ga, 1.0), (gb, 1.0)];
                for g in [ga, gb] {
                    m.players.get_mut(&g).unwrap().set_stand_state(0);
                }
                let st = ScriptStep {
                    target_type: 7,
                    target_param1: 0x2,
                    ext: Some(Box::new(StepExt {
                        min_target_distance_2d: Some(7.0),
                        ..Default::default()
                    })),
                    ..step(28, [8, 1, 0, 0])
                };
                scripts::execute(m, &st, s, Guid::zero());
                flat.push((stand_of(m, ga), stand_of(m, gb)));
            }
            // The nearest hostile player: the living one.
            put(m, ga, at(3.0, 0.0, 0.0));
            put(m, gb, at(8.0, 0.0, 0.0));
            put(m, gc, at(40.0, 0.0, 0.0));
            m.players.get_mut(&ga).unwrap().death =
                crate::game::map::death::Death::Dead { since: m.now };
            let living = scripts::resolve_targets(
                m,
                &ScriptStep {
                    target_type: 26,
                    target_param1: 20,
                    ..step(0, [0; 4])
                },
                s,
                Guid::zero(),
            )
            .map(|(_, t)| t);
            m.players.get_mut(&ga).unwrap().death = crate::game::map::death::Death::Alive;
            (first, randoms, edges, farthest, flat, living)
        })
        .await
        .unwrap();
    let (first, randoms, edges, farthest, flat, living) = out;
    assert_eq!(first, Some(gb), "the player in range");
    assert!(randoms.iter().all(|r| *r == Some(gb)), "{randoms:?}");
    assert_eq!(edges, [None, Some(gb), Some(gb), None], "from 13 to 43");
    assert_eq!(farthest, Some(gb), "the farthest by reach");
    assert_eq!(flat, [(0, 0), (0, 0), (0, 8)], "past 7 yards flat");
    assert_eq!(living, Some(gb), "the living one");
}

/// Thistle Tea's `MOVE_TO` "target-relative coordinates retain authored
/// offsets and movement options" and "distance mode measures from the
/// target radius toward the source": a move to a target's offset goes
/// there, running and pathed as its options say, in the time it gives,
/// facing as it says and telling of its point on arrival, rooted or not
/// (with 0x1 of datalong4); one at a distance stops that far past the
/// target's edge on the source's side.
#[tokio::test]
async fn a_move_to_a_target_keeps_its_offset_options_and_point_or_its_distance_off_its_edge() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            let s = spawn(m, ga, BRUTE, 20, 5.0);
            let t = spawn(m, ga, BRUTE, 20, 6.0);
            put(m, s, at(0.0, 0.0, 0.0));
            put(m, t, at(10.0, 0.0, 5.0));
            crate::game::tests_p3_combat::aura_of(
                m,
                s,
                95_312,
                crate::game::map::auras::AURA_MOD_ROOT,
                0,
                0,
            );
            let offset = ScriptStep {
                x: 1.0,
                y: 2.0,
                z: 0.0,
                o: 1.5,
                dataint: [7, 0, 0, 0],
                ..step(3, [1, 2_000, 0x1 | 0x4, 0x1 | 0x2])
            };
            let ran = scripts::execute(m, &offset, s, t);
            let mv = crate::game::map::event_ai::scripted(m, s)
                .and_then(|x| x.scripted_move)
                .unwrap();
            let walk = match &m.creature(s).unwrap().motion {
                crate::game::map::creature::Motion::Moving(w) => (w.run, w.duration),
                _ => (false, Duration::ZERO),
            };
            let first = (ran, mv.to, mv.face, mv.point, walk);
            let away = ScriptStep {
                x: 2.0,
                o: -1.0,
                ..step(3, [2, 0, 0, 0x1])
            };
            scripts::execute(m, &away, s, t);
            let to = crate::game::map::event_ai::scripted(m, s)
                .and_then(|x| x.scripted_move)
                .map(|x| x.to);
            (first, to)
        })
        .await
        .unwrap();
    let ((ran, to, face, point, (run, time)), away) = out;
    assert!(ran);
    assert!(dist(to, at(11.0, 2.0, 5.0)) < 0.01, "{to:?}");
    assert_eq!((face, point), (Some(1.5), Some(7)));
    assert!(run, "running");
    assert_eq!(time, Duration::from_millis(2_000));
    let away = away.unwrap();
    assert!(
        dist(away, at(7.0, 0.0, 5.0)) < 0.01,
        "two yards past its edge: {away:?}"
    );
}

/// Thistle Tea's `try_cast` "initializes spell timers from initial delays
/// without casting" and "respects the list tick cadence";
/// `SET_SPELL_LIST` "reassigning the same list rerolls its initial
/// delays", "clearing leaves the active cast intact and prevents later
/// list casts" and "uses absolute percentage intervals and clears on an
/// uncovered roll": a list's first try rolls its spells' first delays and
/// casts nothing yet, and the next waits 1.2 seconds; the same list set
/// again starts its delays over; a list cleared leaves the cast under way
/// and casts nothing after; a script's choice of lists takes each by its
/// chance in a hundred, and none past them.
#[tokio::test]
async fn a_spell_lists_first_delays_its_cadence_and_a_scripts_choice_of_lists() {
    use crate::game::map::spells::lists;
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            let s = spawn(m, ga, BRUTE, 20, 5.0);
            ai::attacked(m, s, ga);
            lists::set_list(m, s, LIST);
            let state = |m: &Map| {
                m.creature(s)
                    .unwrap()
                    .caster
                    .spell_list
                    .as_deref()
                    .map(|l| (l.ready_at.clone(), l.next_at))
            };
            let now = m.now;
            lists::tick(m, s, ga);
            let first = state(m).unwrap();
            let first = (
                first.0 == vec![now + Duration::from_secs(5)],
                first.1 == Some(now + lists::LIST_TICK),
                m.caster(s).unwrap().casting.is_none(),
            );
            // Ready, but its list waits its cadence.
            let ready = m.now;
            if let Some(l) = m.creature_mut(s).unwrap().caster.spell_list.as_deref_mut() {
                l.ready_at = vec![ready];
            }
            lists::tick(m, s, ga);
            let waits =
                m.caster(s).unwrap().casting.is_none() && m.caster(s).unwrap().cooldowns.is_empty();
            // The same list again: its delays start over.
            scripts::execute(
                m,
                &ScriptStep {
                    dataint: [100, 0, 0, 0],
                    ..step(55, [LIST, 0, 0, 0])
                },
                s,
                ga,
            );
            m.now += lists::LIST_TICK;
            let now = m.now;
            lists::tick(m, s, ga);
            let rerolled = state(m).unwrap().0 == vec![now + Duration::from_secs(5)];
            // A cast under way, the list cleared: the cast goes on, and
            // nothing more.
            crate::game::map::spells::cast(
                m,
                s,
                SLOW_BOLT,
                crate::game::map::spells::Targets {
                    unit: Some(ga),
                    ..Default::default()
                },
                None,
            );
            let casting = m.caster(s).unwrap().casting.is_some();
            scripts::execute(
                m,
                &ScriptStep {
                    dataint: [100, 0, 0, 0],
                    ..step(55, [0, 0, 0, 0])
                },
                s,
                ga,
            );
            let cleared = (
                casting,
                lists::list_of(m, s),
                m.caster(s).unwrap().casting.is_some(),
            );
            // Thirty in a hundred each, and forty left: none.
            let mut seen = std::collections::BTreeSet::new();
            for _ in 0..300 {
                lists::set_list(m, s, LIST);
                scripts::execute(
                    m,
                    &ScriptStep {
                        dataint: [30, 30, 0, 0],
                        ..step(55, [LIST, LIST_B, 0, 0])
                    },
                    s,
                    ga,
                );
                seen.insert(lists::list_of(m, s));
            }
            (first, waits, rerolled, cleared, seen)
        })
        .await
        .unwrap();
    let (first, waits, rerolled, cleared, seen) = out;
    assert_eq!(first, (true, true, true), "first delays, no cast");
    assert!(waits, "its cadence");
    assert!(rerolled, "its delays start over");
    assert_eq!(cleared, (true, 0, true), "the cast goes on, the list gone");
    assert_eq!(
        seen.into_iter().collect::<Vec<_>>(),
        vec![0, LIST, LIST_B],
        "each by its chance, none past them"
    );
}
