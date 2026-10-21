//! Round 7's fear and control, ported from Thistle Tea's
//! `test/game/core/movement/{fear,control_movement,falling}_test.exs`
//! (docs/parity/matrix.csv): a feared creature's runs and pauses, what
//! halts or takes them over, what prevents fleeing (mangos's
//! `SetFleeing`), a player's control while feared, confused and rooted,
//! and the fall a server-driven move ends.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;

use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{UNIT_FLAG_CONFUSED, UNIT_FLAG_FLEEING};
use crate::game::map::creature::Motion;
use crate::game::map::creature_combat::{self, Control};
use crate::game::map::player_motion::{self, Mode};
use crate::game::tests_parity_motion as motion;

/// tests_parity_motion's spells: fear, confusion, root and stun.
const FEAR: u32 = 92_001;
const CONFUSE: u32 = 92_002;
const STUN: u32 = 92_004;
/// Recklessness's part: what prevents fleeing.
const PREVENT: u32 = 97_730;

fn world() -> GameData {
    let mut d = motion::wolf_world();
    let base = d.spells.get(FEAR).unwrap().clone();
    d.spells.spells.insert(
        PREVENT,
        SpellTemplate {
            id: PREVENT,
            name: "Recklessness".into(),
            effects: [
                SpellEffect {
                    effect: 6,
                    aura: crate::game::map::auras::AURA_PREVENTS_FLEEING,
                    base_dice: 1,
                    die_sides: 1,
                    target_a: 1,
                    ..Default::default()
                },
                SpellEffect::default(),
                SpellEffect::default(),
            ],
            ..base
        },
    );
    d
}

fn remove(m: &mut Map, g: Guid, spell: u32) {
    crate::game::map::auras::remove_spell(m, g, spell, None);
}

/// The wolf's run under way: its spline id.
fn run_of(m: &Map, g: Guid) -> Option<u32> {
    match &m.creature(g)?.motion {
        Motion::Moving(w) => Some(w.spline_id),
        Motion::Idle { .. } => None,
    }
}

fn flags(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_FLAGS))
}

/// The wolf feared by Alice, on its first run.
fn feared(m: &mut Map, ga: Guid, g: Guid) -> u32 {
    crate::game::map::ai::attacked(m, g, ga);
    motion::apply(m, ga, g, FEAR);
    assert!(creature_combat::controlled_step(m, g));
    run_of(m, g).expect("it runs")
}

/// A fear's run ends in a pause of 0.8 to 1.5 seconds, and the next run
/// starts when it is over (Thistle Tea's "pauses after arrival and
/// requests a new run when ready"); a fear whose caster isn't there runs
/// from where the creature is as from its caster's feet, at most 25 yards
/// ("uses the current position and ignores casters in other copies",
/// "bounds the search even when the caster is missing").
#[tokio::test]
async fn a_fear_run_pauses_after_arriving_then_runs_again() {
    let (_realm, mut a, ga, map) = motion::enter(world()).await;
    let g = motion::wolf();
    a.sees(g).await;
    let out = map
        .query(move |m| {
            let first = feared(m, ga, g);
            let c = m.creature(g).unwrap();
            let Motion::Moving(w) = &c.motion else {
                unreachable!()
            };
            let (run, start) = (w.duration, m.now);
            let pause = c.engagement.next_move_at.unwrap() - start - run;
            // Arrived, it waits out the pause.
            m.now = start + run + Duration::from_millis(1);
            creature_combat::controlled_step(m, g);
            let waits = run_of(m, g).is_none();
            m.now = c_next(m, g);
            creature_combat::controlled_step(m, g);
            let next = run_of(m, g);
            (pause, waits, first, next)
        })
        .await
        .unwrap();
    let (pause, waits, first, next) = out;
    assert!(
        (Duration::from_millis(800)..Duration::from_millis(1500)).contains(&pause),
        "{pause:?}"
    );
    assert!(waits, "it pauses there");
    assert!(next.is_some_and(|n| n != first), "and runs again");
    // A frightener not on the map: a run from the creature's own place.
    let runs = map
        .query(move |m| {
            remove(m, g, FEAR);
            let nobody = crate::game::guid::unit(motion::WOLF, 999);
            let mut runs = Vec::new();
            for _ in 0..10 {
                remove(m, g, FEAR);
                let from = m.creature(g).unwrap().position;
                motion::apply(m, nobody, g, FEAR);
                creature_combat::controlled_step(m, g);
                if let Motion::Moving(w) = &m.creature(g).unwrap().motion {
                    runs.push((from, w.destination()));
                }
            }
            runs
        })
        .await
        .unwrap();
    assert!(!runs.is_empty());
    for (from, to) in runs {
        let d = crate::game::map::combat::dist(from, to);
        assert!(
            (11.1..=25.01).contains(&d),
            "from its place, as from its frightener's: {d}"
        );
    }
}

fn c_next(m: &Map, g: Guid) -> std::time::Instant {
    m.creature(g).unwrap().engagement.next_move_at.unwrap()
}

/// A run's path is cut at 30 yards (Thistle Tea's "caps the traversed
/// path when navigation returns a distant point").
#[tokio::test]
async fn a_controlled_run_is_cut_at_thirty_yards() {
    let (_realm, _a, _ga, map) = motion::enter(world()).await;
    let path = map
        .query(|m| {
            player_motion::control_path(m, motion::at(0.0, 0.0), motion::at(50.0, 50.0), 30.0)
        })
        .await
        .unwrap()
        .expect("a path");
    assert_eq!(path.len(), 2);
    let d = crate::game::map::combat::dist(path[0], path[1]);
    assert!((d - 30.0).abs() < 0.01, "{d}");
}

/// A root or stun halts a fear's run where it is, and none starts while it
/// holds; once it goes, the fear runs again (Thistle Tea's "roots and
/// stuns halt runs and fear resumes after removal"). Confusion takes the
/// run over, and its end gives it back to the fear at once ("confusion
/// takes over a fear run and removing it restores fear").
#[tokio::test]
async fn roots_stuns_and_confusion_take_a_fear_run_over_until_they_go() {
    let (_realm, mut a, ga, map) = motion::enter(world()).await;
    let g = motion::wolf();
    a.sees(g).await;
    for hold in [motion::ROOT, STUN] {
        let (halted, held, again) = map
            .query(move |m| {
                remove(m, g, FEAR);
                creature_combat::controlled_step(m, g);
                feared(m, ga, g);
                motion::apply(m, ga, g, hold);
                let halted = run_of(m, g).is_none();
                m.now += Duration::from_secs(3);
                creature_combat::controlled_step(m, g);
                let held = run_of(m, g).is_none();
                remove(m, g, hold);
                m.now += Duration::from_secs(3);
                creature_combat::controlled_step(m, g);
                (halted, held, run_of(m, g).is_some())
            })
            .await
            .unwrap();
        assert!(halted, "{hold}: halted at once");
        assert!(held, "{hold}: no run while it holds");
        assert!(again, "{hold}: the fear runs again");
    }
    let (taken, mode, back, ready) = map
        .query(move |m| {
            let first = run_of(m, g);
            motion::apply(m, ga, g, CONFUSE);
            let taken = run_of(m, g) != first;
            let mode = m.creature(g).unwrap().engagement.control;
            remove(m, g, CONFUSE);
            let c = m.creature(g).unwrap();
            let ready = c.engagement.next_move_at.is_some_and(|t| t <= m.now);
            (taken, mode, c.engagement.control, ready)
        })
        .await
        .unwrap();
    assert!(taken, "confusion stops the fear's run");
    assert!(matches!(mode, Some(Control::Confusion { .. })), "{mode:?}");
    assert!(matches!(back, Some(Control::Fear)), "{back:?}");
    assert!(ready, "the fear runs again at once");
}

/// What prevents fleeing (Recklessness) holds a fear back: one under way
/// stops, its fleeing flag goes, the fear stays on; a fear taken under it
/// leaves the cast and fight alone; when it goes, the fear takes hold: the
/// cast stops, the flag is back, and the creature runs (mangos's
/// `SetFleeing` and `HandlePreventFleeing`; Thistle Tea's "recklessness
/// suspends fear without removing it and removal resumes it", "fear
/// applied under recklessness leaves casting intact until suppression
/// ends").
#[tokio::test]
async fn what_prevents_fleeing_holds_a_fear_back_until_it_goes() {
    let (_realm, mut a, ga, map) = motion::enter(world()).await;
    let g = motion::wolf();
    a.sees(g).await;
    let out = map
        .query(move |m| {
            feared(m, ga, g);
            motion::apply(m, g, g, PREVENT);
            let c = m.creature(g).unwrap();
            let suspended = (
                run_of(m, g).is_none(),
                c.engagement.control.is_none(),
                c.auras.has(crate::game::map::auras::AURA_MOD_FEAR),
                flags(m, g) & UNIT_FLAG_FLEEING == 0,
            );
            remove(m, g, PREVENT);
            let resumed = (
                matches!(
                    m.creature(g).unwrap().engagement.control,
                    Some(Control::Fear)
                ),
                flags(m, g) & UNIT_FLAG_FLEEING != 0,
            );
            creature_combat::controlled_step(m, g);
            let runs = run_of(m, g).is_some();
            // A fear taken under it: the cast goes on, until it goes.
            remove(m, g, FEAR);
            motion::apply(m, g, g, PREVENT);
            let now = m.now;
            m.caster_mut(g).unwrap().casting = Some(crate::game::map::spells::Casting {
                spell: motion::CHARGE,
                targets: crate::game::map::spells::Targets {
                    unit: Some(ga),
                    ..Default::default()
                },
                ends: now + Duration::from_secs(3),
                item: None,
                pushbacks: 0,
                channel_until: None,
                duration: 3000,
                at: None,
            });
            motion::apply(m, ga, g, FEAR);
            let casting = m.caster(g).unwrap().casting.is_some();
            let unflagged = flags(m, g) & UNIT_FLAG_FLEEING == 0;
            remove(m, g, PREVENT);
            let stopped = m.caster(g).unwrap().casting.is_none();
            (suspended, resumed, runs, casting, unflagged, stopped)
        })
        .await
        .unwrap();
    let (suspended, resumed, runs, casting, unflagged, stopped) = out;
    assert_eq!(suspended, (true, true, true, true), "held back");
    assert_eq!(resumed, (true, true), "its end lets the fear go");
    assert!(runs, "and it runs");
    assert!(
        casting && unflagged,
        "a fear taken under it changes nothing"
    );
    assert!(stopped, "until it goes");
}

/// A fear taken stops the walk under way and keeps the creature's threat;
/// a dispelled one stops its run at once and gives the creature back to
/// itself; death ends it, the flag gone, nothing left to run (Thistle
/// Tea's "application stops old movement and preserves combat and patrol
/// state", "dispel stops a run immediately and restores normal behavior",
/// "death clears fear without restoring stale navigation"). Its target it
/// drops, as mangos's fleeing does (`AbstractRandomMovementGenerator`).
#[tokio::test]
async fn a_fear_stops_the_walk_before_it_and_its_end_stops_its_run() {
    let (_realm, mut a, ga, map) = motion::enter(world()).await;
    let g = motion::wolf();
    a.sees(g).await;
    let out = map
        .query(move |m| {
            crate::game::map::ai::attacked(m, g, ga);
            let here = m.creature(g).unwrap().position;
            let spline = m.next_spline_id;
            m.next_spline_id += 1;
            let now = m.now;
            let c = m.creature_mut(g).unwrap();
            c.walk(vec![here, motion::at(10.0, 20.0)], false, now, spline);
            motion::apply(m, ga, g, FEAR);
            let c = m.creature(g).unwrap();
            let applied = (
                run_of(m, g) != Some(spline),
                c.threat.iter().any(|(t, _)| *t == ga),
                flags(m, g) & UNIT_FLAG_FLEEING != 0,
            );
            creature_combat::controlled_step(m, g);
            remove(m, g, FEAR);
            let c = m.creature(g).unwrap();
            let dispelled = (
                run_of(m, g).is_none(),
                c.engagement.control.is_none(),
                flags(m, g) & UNIT_FLAG_FLEEING == 0,
            );
            motion::apply(m, ga, g, FEAR);
            creature_combat::controlled_step(m, g);
            crate::game::map::combat::deal_damage(m, ga, g, 1_000_000);
            let c = m.creature(g).unwrap();
            let died = (
                !c.alive(),
                run_of(m, g).is_none(),
                c.engagement.control.is_none(),
                flags(m, g) & UNIT_FLAG_FLEEING == 0,
            );
            (applied, dispelled, died)
        })
        .await
        .unwrap();
    let (applied, dispelled, died) = out;
    assert_eq!(
        applied,
        (true, true, true),
        "the walk stops, the threat stays"
    );
    assert_eq!(dispelled, (true, true, true), "dispelled: stopped at once");
    assert_eq!(died, (true, true, true, true), "dead: nothing left of it");
}

fn control(m: &ServerOpcodeMessage, g: Guid) -> Option<bool> {
    match m {
        ServerOpcodeMessage::SMSG_CLIENT_CONTROL_UPDATE(c) if c.guid == g => Some(c.allow_movement),
        _ => None,
    }
}

/// A player's control is taken by the first of fear and confusion and
/// given back by the last to go, once each; both flags while both hold;
/// confusion moves them while it lasts, the fear after (Thistle Tea's
/// "revokes control until the last overlapping effect ends"). What
/// prevents fleeing gives control back from a fear, and takes it again
/// when it goes; not from a confusion ("fleeing prevention restores
/// control unless confusion remains"). Death gives it back, the flags
/// gone ("death clears control, flags, and navigation").
#[tokio::test]
async fn a_players_control_goes_with_the_first_and_comes_back_with_the_last() {
    let (_realm, mut a, ga, map) = motion::enter(world()).await;
    let g = motion::wolf();
    a.sees(g).await;
    a.clear();
    let modes = map
        .query(move |m| {
            let mode = |m: &Map| m.players[&ga].driven.as_ref().map(|d| d.mode);
            let mut modes = Vec::new();
            motion::apply(m, g, ga, FEAR);
            modes.push((mode(m), flags(m, ga)));
            motion::apply(m, g, ga, CONFUSE);
            modes.push((mode(m), flags(m, ga)));
            remove(m, ga, CONFUSE);
            modes.push((mode(m), flags(m, ga)));
            remove(m, ga, FEAR);
            modes.push((mode(m), flags(m, ga)));
            modes
        })
        .await
        .unwrap();
    let both = UNIT_FLAG_FLEEING | UNIT_FLAG_CONFUSED;
    assert_eq!(modes[0].0, Some(Mode::Fear));
    assert!(matches!(modes[1].0, Some(Mode::Confusion { .. })));
    assert_eq!(modes[1].1 & both, both, "both flags");
    assert_eq!(modes[2].0, Some(Mode::Fear));
    assert_eq!(modes[3].0, None);
    assert_eq!(modes[3].1 & both, 0);
    let first = a.wait_for("control taken", |m| control(m, ga)).await;
    let last = a.wait_for("control back", |m| control(m, ga)).await;
    assert_eq!((first, last), (false, true), "once each");
    assert!(
        a.gets(Duration::from_millis(200), |m| control(m, ga))
            .await
            .is_none()
    );
    // What prevents fleeing.
    map.query(move |m| {
        motion::apply(m, g, ga, FEAR);
        motion::apply(m, ga, ga, PREVENT);
    })
    .await;
    assert!(!a.wait_for("taken by the fear", |m| control(m, ga)).await);
    assert!(
        a.wait_for("back for the prevention", |m| control(m, ga))
            .await
    );
    map.query(move |m| remove(m, ga, PREVENT)).await;
    assert!(
        !a.wait_for("taken again at its end", |m| control(m, ga))
            .await
    );
    let confused = map
        .query(move |m| {
            motion::apply(m, g, ga, CONFUSE);
            motion::apply(m, ga, ga, PREVENT);
            m.players[&ga].driven.as_ref().map(|d| d.mode)
        })
        .await
        .unwrap();
    assert!(matches!(confused, Some(Mode::Confusion { .. })));
    assert!(
        a.gets(Duration::from_millis(200), |m| control(m, ga))
            .await
            .is_none(),
        "confusion keeps it"
    );
    // Death gives it back.
    let dead = map
        .query(move |m| {
            crate::game::map::death::kill(m, ga);
            (m.players[&ga].driven.is_none(), flags(m, ga) & both)
        })
        .await
        .unwrap();
    assert_eq!(dead, (true, 0));
    assert!(a.wait_for("back at death", |m| control(m, ga)).await);
}

/// A feared player's run: control coming back stops it where the player
/// is, the server telling the client so, and nothing more of it runs
/// (Thistle Tea's "stops at the current spline position and discards
/// pending movement"); a root halts it without giving control back, and
/// the fear runs again once the root goes ("roots pause movement without
/// restoring client control"). The run ends a fall under way ("server-
/// driven movement clears tracked height"), and so does a charge.
#[tokio::test]
async fn a_feared_players_run_stops_where_they_are() {
    let (_realm, mut a, ga, map) = motion::enter(world()).await;
    let g = motion::wolf();
    a.sees(g).await;
    let out = map
        .query(move |m| {
            m.players.get_mut(&ga).unwrap().fall_from = Some(500.0);
            motion::apply(m, g, ga, FEAR);
            player_motion::update(m);
            let p = &m.players[&ga];
            let fall = p.fall_from;
            let running = p.driven.as_ref().is_some_and(|d| d.walk.is_some());
            let start = p.position();
            // A root: halted, still driven.
            m.now += Duration::from_millis(300);
            motion::apply(m, g, ga, motion::ROOT);
            let p = &m.players[&ga];
            let rooted = (
                p.driven.is_some(),
                p.driven.as_ref().unwrap().walk.is_none(),
            );
            let held_at = p.position();
            m.now += Duration::from_secs(3);
            player_motion::update(m);
            let still = m.players[&ga].position() == held_at;
            remove(m, ga, motion::ROOT);
            m.now += Duration::from_secs(2);
            player_motion::update(m);
            let again = m.players[&ga]
                .driven
                .as_ref()
                .is_some_and(|d| d.walk.is_some());
            // The fear's end, partway: stopped there.
            m.now += Duration::from_millis(300);
            remove(m, ga, FEAR);
            let p = &m.players[&ga];
            let stopped_at = p.position();
            let ended = p.driven.is_none();
            m.now += Duration::from_secs(5);
            player_motion::update(m);
            let stays = m.players[&ga].position() == stopped_at;
            (
                fall,
                running,
                start != held_at,
                rooted,
                still,
                again,
                ended,
                stays,
            )
        })
        .await
        .unwrap();
    let (fall, running, moved, rooted, still, again, ended, stays) = out;
    assert!(running && moved, "it runs");
    assert_eq!(fall, None, "the fall before is over");
    assert_eq!(rooted, (true, true), "a root halts it, control kept");
    assert!(still && again, "held, then running again");
    assert!(ended && stays, "the fear's end stops it there");
    // The client hears it stop (a monster move of its own, stopping).
    motion::next_move(&mut a, ga, |m| m.kind == 1).await;
    // A charge ends a fall too.
    let fall = map
        .query(move |m| {
            let p = m.players.get_mut(&ga).unwrap();
            p.fall_from = Some(500.0);
            p.movement.position = motion::at(0.0, 0.0);
            crate::game::map::charge::start(m, ga, g, true);
            m.players[&ga].fall_from
        })
        .await
        .unwrap();
    assert_eq!(fall, None, "a charge's fall is its own");
}
