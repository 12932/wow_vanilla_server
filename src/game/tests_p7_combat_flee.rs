//! Round 7's flight for help (Thistle Tea's `assistance_test.exs`): whom
//! a fleeing creature runs to and what it shows, a panic with no one to
//! run to, what bars a flight, its run and its call, and what ends it.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, UNIT_FLAG_FLEEING};
use crate::game::map::creature::Motion;
use crate::game::map::creature_combat::{self, flee_for_assistance};
use crate::game::map::spells::{Casting, Targets};
use crate::game::map::{Map, ai};
use crate::game::tests_p3_combat as p3;
use crate::game::tests_p6_wspell::put;
use crate::game::tests_p7_combat::{creature_at, off, setup_with};

/// A faction whose members answer calls, at odds with the humans.
const CALLERS: u32 = 41;
const RUNNER: u32 = 3790;

fn data() -> GameData {
    let mut d = crate::game::tests_p7_combat::data();
    let mut f = p3::faction(CALLERS, 0x2, 0x1);
    f.flags = 0x1;
    d.faction_templates.insert(CALLERS, f);
    let mut t = p3::template(RUNNER, 7);
    t.faction = CALLERS;
    // vmangos's usual reach of a call.
    t.call_for_help_range = 5.0;
    d.creature_templates.insert(RUNNER, t);
    d
}

/// A runner fighting Alice by her, and its kind 21 and 28 yards off.
fn fighting(m: &mut Map, ga: Guid, y: f32) -> (Guid, Guid, Guid) {
    put(m, ga, off(-2.0, y, 0.0));
    let runner = creature_at(m, ga, RUNNER, off(0.0, y, 0.0));
    let near = creature_at(m, ga, RUNNER, off(21.0, y, 0.0));
    let far = creature_at(m, ga, RUNNER, off(28.0, y, 0.0));
    ai::attacked(m, runner, ga);
    (runner, near, far)
}

/// A runner fighting Alice by her, alone.
fn alone(m: &mut Map, ga: Guid, y: f32) -> Guid {
    put(m, ga, off(-2.0, y, 0.0));
    let runner = creature_at(m, ga, RUNNER, off(0.0, y, 0.0));
    ai::attacked(m, runner, ga);
    runner
}

fn flee(m: &Map, g: Guid) -> Option<creature_combat::Flee> {
    m.creature(g).and_then(|c| c.engagement.flee)
}

fn fleeing_flag(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_FLEEING))
}

fn target(m: &Map, g: Guid) -> Guid {
    m.unit_fields(g).unwrap().get_guid(UNIT_FIELD_TARGET)
}

/// A spell of `id` putting `aura` on its target.
fn aura_spell(m: &mut Map, caster: Guid, on: Guid, id: u32, aura: u32) {
    let mut s = crate::game::data::spells::SpellTemplate {
        id,
        name: format!("Aura {id}"),
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0] = crate::game::data::spells::SpellEffect {
        effect: 6,
        aura,
        target_a: 6,
        ..Default::default()
    };
    let _ = auras::apply_spell(m, caster, on, &s, &[(0, 0)], auras::Context::default());
}

/// Thistle Tea's `Script.run/5`: a creature fleeing for help runs to the
/// nearest of its kind within 30 yards that answers calls, says so, and
/// shows no target while it runs, its victim kept (mangos's retreat drops
/// it too); with no one to run to, it panics for seven seconds, flagged
/// fleeing; and an aura that prevents fleeing, confusion, a feigned death,
/// a master's control or a distraction bar it, emote and all. Its cast is
/// stopped, whatever it was (mangos's `SetFleeing` and its `CastStop`).
#[tokio::test]
async fn a_flight_runs_to_the_nearest_of_its_kind_or_panics_unless_barred() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let (runner, near, _far) = fighting(m, ga, 0.0);
            // A cast movement doesn't break: stopped all the same.
            let now = m.now;
            m.caster_mut(runner).unwrap().casting = Some(Casting {
                spell: 95_330,
                targets: Targets {
                    unit: Some(ga),
                    ..Default::default()
                },
                ends: now + Duration::from_secs(5),
                item: None,
                pushbacks: 0,
                channel_until: None,
                duration: 5000,
                at: None,
            });
            flee_for_assistance(m, runner, true);
            let seeking = (
                flee(m, runner).and_then(|f| f.helper) == Some(near),
                m.creature(runner).unwrap().combat.victim == ga,
                target(m, runner).is_zero(),
                m.caster(runner).unwrap().casting.is_none(),
            );
            // No one within 30 yards: a panic of seven seconds.
            let lone = alone(m, ga, 100.0);
            let now = m.now;
            flee_for_assistance(m, lone, true);
            let panic = (
                flee(m, lone).map(|f| (f.helper, f.until))
                    == Some((None, now + Duration::from_secs(7))),
                fleeing_flag(m, lone),
            );
            // Barred.
            let mut barred = Vec::new();
            for how in 0..5 {
                let r = alone(m, ga, 200.0 + 50.0 * how as f32);
                match how {
                    0 => aura_spell(m, ga, r, 95_331, auras::AURA_PREVENTS_FLEEING),
                    1 => aura_spell(m, ga, r, 95_332, auras::AURA_MOD_CONFUSE),
                    2 => aura_spell(m, r, r, 95_333, auras::AURA_FEIGN_DEATH),
                    3 => m.creature_mut(r).unwrap().owner = Some(ga),
                    _ => {
                        let until = m.now + Duration::from_secs(5);
                        m.creature_mut(r).unwrap().engagement.distracted_until = Some(until);
                    }
                }
                flee_for_assistance(m, r, true);
                barred.push(flee(m, r).is_none());
            }
            (seeking, panic, barred)
        })
        .await
        .unwrap();
    assert_eq!(
        r.0,
        (true, true, true, true),
        "(nearest, victim kept, no target, cast stopped)"
    );
    assert_eq!(r.1, (true, true), "a panic of seven seconds, flagged");
    assert_eq!(
        r.2,
        vec![true; 5],
        "(prevented, confused, feigning, owned, distracted)"
    );
    let mut seen = 0;
    while a
        .gets(Duration::from_millis(200), |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c)
                if matches!(c.chat_type, SMSG_MESSAGECHAT_ChatType::MonsterEmote { .. })
                    && c.message.contains("run away") =>
            {
                Some(())
            }
            _ => None,
        })
        .await
        .is_some()
    {
        seen += 1;
    }
    assert_eq!(seen, 2, "the two that fled say so, the barred don't");
}

/// A tenth of a second of the creatures' time.
fn tick(m: &mut Map) {
    m.now += Duration::from_millis(100);
    ai::update_creatures(m);
    crate::game::map::combat::update_creatures(m);
}

fn walk_of(m: &Map, g: Guid) -> Option<(bool, u32, Vector3d)> {
    match &m.creature(g)?.motion {
        Motion::Moving(w) => Some((w.run, w.spline_id, *w.points.last()?)),
        Motion::Idle { .. } => None,
    }
}

/// Thistle Tea's `tick/3` run: it runs to its helper, calls it once on
/// arriving, waits a second and a half, and fights on, its target shown
/// again; and its helper comes.
#[tokio::test]
async fn it_runs_to_its_helper_calls_once_and_fights_on_a_moment_later() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let (runner, near, _) = fighting(m, ga, 0.0);
            // The helper held where it stands, called or not.
            aura_spell(m, near, near, 95_336, auras::AURA_MOD_ROOT);
            flee_for_assistance(m, runner, true);
            let mut running = false;
            let mut arrived = None;
            let mut ended = None;
            for i in 0..80u32 {
                tick(m);
                if let Some((run, _, to)) = walk_of(m, runner) {
                    running |= run && crate::game::map::combat::dist(to, off(21.0, 0.0, 0.0)) < 0.1;
                }
                let f = flee(m, runner);
                if arrived.is_none() && f.is_some_and(|f| f.called_at.is_some()) {
                    arrived = Some(i);
                }
                if arrived.is_some() && f.is_none() {
                    ended = Some(i);
                    break;
                }
            }
            (
                running,
                arrived,
                ended,
                target(m, runner) == ga,
                m.creature(runner).unwrap().combat.attacking,
                m.creature(near).unwrap().combat.victim == ga,
            )
        })
        .await
        .unwrap();
    let (running, arrived, ended, shown, fights, helped) = r;
    assert!(running, "it runs to its helper");
    let (arrived, ended) = (arrived.expect("it arrives"), ended.expect("and goes on"));
    assert!(
        (14..=16).contains(&ended.abs_diff(arrived)),
        "a second and a half: {arrived} {ended}"
    );
    assert_eq!((shown, fights, helped), (true, true, true));
}

/// Thistle Tea's `tick/3`'s ends and holds: rooted, it stands and calls
/// no one, and freed runs on to its helper; its victim changed, dead or
/// gone, the run ends; fear ends it (fear's own flag kept); an evade or
/// its death clear it; and a panic run out stops where it is, unflagged.
#[tokio::test]
async fn a_root_holds_the_run_and_a_lost_victim_fear_an_evade_death_or_time_end_it() {
    let (realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let r = map
        .query(move |m| {
            // Rooted halfway: held, no call; freed, on to its helper.
            let (runner, _, _) = fighting(m, ga, 0.0);
            flee_for_assistance(m, runner, true);
            for _ in 0..5 {
                tick(m);
            }
            aura_spell(m, ga, runner, 95_334, auras::AURA_MOD_ROOT);
            tick(m);
            tick(m);
            let held = (
                walk_of(m, runner).is_none(),
                flee(m, runner).is_some_and(|f| f.called_at.is_none()),
            );
            auras::remove_spell(m, runner, 95_334, None);
            let mut resumed = false;
            for _ in 0..15 {
                tick(m);
                resumed |= walk_of(m, runner).is_some_and(|(_, _, to)| {
                    crate::game::map::combat::dist(to, off(21.0, 0.0, 0.0)) < 0.1
                });
            }
            // Its victim changed, dead, or gone.
            let mut lost = Vec::new();
            for how in 0..3 {
                p3::level(m, gb, 60);
                let (r, _, _) = fighting(m, ga, 100.0 + 50.0 * how as f32);
                put(m, gb, off(-3.0, 100.0 + 50.0 * how as f32, 0.0));
                flee_for_assistance(m, r, true);
                match how {
                    0 => m.creature_mut(r).unwrap().combat.victim = gb,
                    1 => crate::game::map::death::kill(m, ga),
                    _ => {
                        m.creature_mut(r)
                            .unwrap()
                            .engagement
                            .flee
                            .as_mut()
                            .unwrap()
                            .from = Guid::new(0x99)
                    }
                }
                if how == 2 {
                    m.creature_mut(r).unwrap().combat.victim = Guid::new(0x99);
                }
                tick(m);
                lost.push(flee(m, r).is_none());
                if how == 1 {
                    crate::game::map::death::resurrect(m, ga, 1.0);
                }
            }
            // Feared.
            let f = alone(m, ga, 300.0);
            flee_for_assistance(m, f, true);
            aura_spell(m, ga, f, 95_335, auras::AURA_MOD_FEAR);
            tick(m);
            let feared = (flee(m, f).is_none(), fleeing_flag(m, f));
            // An evade, and death.
            let e = alone(m, ga, 350.0);
            flee_for_assistance(m, e, true);
            ai::enter_evade(m, e);
            let evaded = (flee(m, e).is_none(), target(m, e).is_zero());
            let d = alone(m, ga, 400.0);
            flee_for_assistance(m, d, true);
            tick(m);
            crate::game::map::combat::kill(m, d, d);
            let died = (flee(m, d).is_none(), walk_of(m, d).is_none());
            // A panic: seven seconds, then it stands, unflagged.
            let p = alone(m, ga, 500.0);
            flee_for_assistance(m, p, false);
            for _ in 0..69 {
                tick(m);
            }
            let before = (flee(m, p).is_some(), fleeing_flag(m, p));
            tick(m);
            tick(m);
            let after = (flee(m, p).is_none(), fleeing_flag(m, p));
            (held, resumed, lost, feared, evaded, died, before, after)
        })
        .await
        .unwrap();
    let (held, resumed, lost, feared, evaded, died, before, after) = r;
    assert_eq!(held, (true, true), "rooted: it stands, and calls no one");
    assert!(resumed, "freed, it runs on to its helper");
    assert_eq!(lost, vec![true; 3], "(changed, dead, gone)");
    assert_eq!(feared, (true, true), "fear ends it, its flag fear's");
    assert_eq!(evaded, (true, true), "an evade clears it");
    assert_eq!(died, (true, true), "and death");
    assert_eq!(before, (true, true), "a panic runs seven seconds");
    assert_eq!(after, (true, false), "then stops, unflagged");
}

/// Thistle Tea's "retimes the retreat after a running-speed change
/// without changing gait": wounded further as it runs, its run goes on at
/// its slower speed, still running, still its flight's own (it went still
/// until its old run's end before: the new walk wasn't the flight's).
#[tokio::test]
async fn a_run_for_help_slowed_by_wounds_goes_on_at_once() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let (runner, _, _) = fighting(m, ga, 0.0);
            flee_for_assistance(m, runner, true);
            tick(m);
            let before = m
                .creature(runner)
                .and_then(|c| match &c.motion {
                    Motion::Moving(w) => Some(w.duration),
                    _ => None,
                })
                .unwrap();
            // Down to a twentieth of its health: half its run speed.
            let max = m.unit_fields(runner).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
            m.unit_fields_mut(runner)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, max / 20);
            crate::game::map::creature_motion::health_changed(m, runner);
            let retimed = m.creature(runner).map(|c| {
                (
                    matches!(&c.motion, Motion::Moving(w) if w.run && w.duration > before / 2),
                    walk_of(m, runner).map(|w| w.1) == c.engagement.control_spline,
                )
            });
            tick(m);
            (retimed, walk_of(m, runner).is_some_and(|w| w.0))
        })
        .await
        .unwrap();
    assert_eq!(
        r,
        (Some((true, true)), true),
        "(retimed, running, its own), and on"
    );
}

/// Thistle Tea's "loads the Horde Laborer assistance command": his
/// `creature_ai_scripts` 1471802 is a flight for help.
#[test]
#[ignore = "extended"]
fn the_horde_laborer_flees_for_help() {
    let db = crate::game::harness::real_data("db");
    if !db.join("vmangos.sqlite").exists() {
        return;
    }
    let data = crate::game::harness::load_real(&db);
    let steps = data.scripts.ai_scripts.get(&1_471_802).expect("the script");
    assert_eq!(
        steps
            .iter()
            .map(|s| (s.command, s.datalong[0]))
            .collect::<Vec<_>>(),
        vec![(47, 1)]
    );
    assert!(crate::game::map::scripts::command_supported(47));
}
