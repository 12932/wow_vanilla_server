//! Round 7's charges, ported from Thistle Tea's
//! `test/game/core/movement/charge_test.exs` (docs/parity/matrix.csv): the
//! path stopping short of the target, the swings put off by the run, the
//! swing on arrival and none for a charge cut short, and which charges
//! swing at all.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_CANCELS_AUTO_ATTACK, SpellEffect};
use crate::game::map::charge;
use crate::game::map::combat::dist;
use crate::game::map::creature::{Motion, Walk};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_parity_motion as motion;

const FRIENDLY_CHARGE: u32 = 97_720;
const CALM_CHARGE: u32 = 97_721;

fn v(x: f32, y: f32) -> Vector3d {
    Vector3d { x, y, z: 0.0 }
}

/// Thistle Tea's `Charge.approach_path`: the path as far as the target's
/// reach, the leg into it cut short, the points before kept; nothing when
/// the start is within reach already.
#[test]
fn a_charges_path_stops_at_the_targets_reach_keeping_its_corners() {
    let path = charge::approach(&[v(6.0, 0.0), v(6.0, 8.0)], v(0.0, 0.0), v(6.0, 8.0), 4.5);
    assert_eq!(path.len(), 2, "{path:?}");
    assert_eq!(path[0], v(6.0, 0.0));
    assert!(dist(path[1], v(6.0, 3.5)) < 1e-4, "{path:?}");
    assert!(charge::approach(&[v(2.0, 0.0)], v(0.0, 0.0), v(2.0, 0.0), 4.5).is_empty());
}

/// The wolf 20 yards east, and two more charges: a helpful one, and one
/// that ends the fight.
fn world() -> GameData {
    let mut d = motion::wolf_world();
    d.creature_spawns.get_mut(&0).unwrap()[0] =
        motion::spawn(80, motion::WOLF, motion::at(20.0, 0.0));
    let base = d.spells.get(motion::CHARGE).unwrap().clone();
    let friendly = crate::game::data::spells::SpellTemplate {
        id: FRIENDLY_CHARGE,
        effects: [
            SpellEffect {
                effect: 96,
                target_a: 21,
                ..Default::default()
            },
            SpellEffect::default(),
            SpellEffect::default(),
        ],
        ..base.clone()
    };
    let mut calm = crate::game::data::spells::SpellTemplate {
        id: CALM_CHARGE,
        ..base
    };
    calm.attributes[0] |= ATTR_CANCELS_AUTO_ATTACK;
    d.spells.spells.insert(FRIENDLY_CHARGE, friendly);
    d.spells.spells.insert(CALM_CHARGE, calm);
    d
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

/// The map's clock `by` on, and the charges' step at that time.
fn later(m: &mut Map, by: Duration) {
    m.now += by;
    charge::update(m);
}

/// A player's charge puts both swings off by the run, from when each was
/// due if later than now: what was left of a swing's wait is kept (Thistle
/// Tea's "adds the delay to both player swings without resetting their
/// remaining cooldown"). A creature's swing isn't touched.
#[tokio::test]
async fn a_charge_puts_off_both_swings_keeping_what_was_left() {
    let (_realm, mut a, ga, map) = motion::enter(world()).await;
    let g = motion::wolf();
    a.sees(g).await;
    for left in [None, Some(-1_i64), Some(500)] {
        let (swing, offhand) = on(&map, move |m| {
            let now = m.now;
            let p = m.players.get_mut(&ga).unwrap();
            p.charge = None;
            p.movement.position = motion::at(0.0, 0.0);
            p.combat.next_swing = left.map(|ms| {
                if ms < 0 {
                    now - Duration::from_millis(1)
                } else {
                    now + Duration::from_millis(ms as u64)
                }
            });
            p.combat.next_offhand = Some(now + Duration::from_millis(900));
            charge::start(m, ga, g, true);
            let p = &m.players[&ga];
            let wait = |t: Option<std::time::Instant>| t.unwrap().duration_since(now);
            (wait(p.combat.next_swing), wait(p.combat.next_offhand))
        })
        .await;
        let delay = offhand - Duration::from_millis(900);
        assert!(delay >= Duration::from_millis(200), "{delay:?}");
        let kept = Duration::from_millis(left.filter(|ms| *ms > 0).unwrap_or(0) as u64);
        assert_eq!(swing, kept + delay, "{left:?}");
    }
    let (before, after) = on(&map, move |m| {
        let before = m.combat(g).map(|c| (c.next_swing, c.next_offhand));
        m.creature_mut(g).unwrap().position = motion::at(20.0, 20.0);
        charge::start(m, g, ga, true);
        let after = m.combat(g).map(|c| (c.next_swing, c.next_offhand));
        (before, after)
    })
    .await;
    assert_eq!(before, after, "a creature's swing is its own");
}

/// A charge that arrives swings at its target, once; one whose target died
/// on the way swings at nothing (Thistle Tea's "successful arrival requests
/// one attack on the original target lifetime"). Nor does one cut short: by
/// fear or a teleport (its `cancel`), by its charger's death, its corpse
/// lying where it fell ("death ends the charge without moving the corpse
/// on a late arrival"), or for a creature, by a newer run, which goes on
/// ("a superseding spline survives an old arrival"). A dead or rooted
/// charger, player or creature, doesn't start ("a queued charge cannot move
/// a newly dead or rooted owner").
#[tokio::test]
async fn a_charge_swings_once_on_arrival_and_not_when_cut_short() {
    let (_realm, mut a, ga, map) = motion::enter(world()).await;
    let g = motion::wolf();
    a.sees(g).await;
    // Arrives: swings, once.
    let (attacking, victim, after) = on(&map, move |m| {
        charge::start(m, ga, g, true);
        assert!(m.players[&ga].charge.is_some(), "it charges");
        later(m, Duration::from_secs(5));
        let p = &m.players[&ga];
        let (attacking, victim) = (p.combat.attacking, p.combat.victim);
        crate::game::map::combat::stop_attack(m, ga);
        later(m, Duration::from_secs(6));
        (attacking, victim, m.players[&ga].combat.attacking)
    })
    .await;
    assert!(attacking && victim == g, "it swings there");
    assert!(!after, "once");
    // Each way of cutting it short: no swing.
    type Cut = fn(&mut Map, Guid, Guid);
    let cuts: [(&str, Cut); 4] = [
        ("feared", |m, ga, _| motion::apply(m, ga, ga, 92_001)),
        ("teleported", |m, ga, _| {
            let at = m.players[&ga].position();
            crate::game::map::movement::teleport_within(m, ga, at, 0.0);
        }),
        ("dead", |m, ga, _| crate::game::map::death::kill(m, ga)),
        ("the target dead", |m, _, g| {
            crate::game::map::combat::deal_damage(m, g, g, 1_000_000);
        }),
    ];
    for (what, cut) in cuts {
        let out = on(&map, move |m| {
            crate::game::map::death::resurrect(m, ga, 1.0);
            let p = m.players.get_mut(&ga).unwrap();
            p.teleport = None;
            p.movement.position = motion::at(0.0, 0.0);
            p.charge = None;
            crate::game::map::auras::remove_spell(m, ga, 92_001, None);
            crate::game::map::combat::stop_attack(m, ga);
            if let Some(c) = m.creature_mut(g) {
                c.position = motion::at(20.0, 0.0);
            }
            if m.unit_view(g).is_some_and(|u| !u.alive) {
                return None;
            }
            charge::start(m, ga, g, true);
            later(m, Duration::from_millis(200));
            let partway = m.players[&ga].position();
            cut(m, ga, g);
            later(m, Duration::from_secs(5));
            let p = &m.players[&ga];
            Some((
                p.combat.attacking,
                p.charge.is_none(),
                dist(p.position(), partway),
            ))
        })
        .await;
        let Some((attacking, ended, moved)) = out else {
            panic!("{what}: the wolf is gone");
        };
        assert!(!attacking, "{what}: no swing");
        assert!(ended, "{what}: the charge is over");
        if what == "dead" {
            assert!(moved < 0.01, "the corpse lies where it fell: {moved}");
        }
    }
}

/// A creature's charge: cut short by a newer run, which goes on, and by
/// its death; refused to a rooted creature or a dead one.
#[tokio::test]
async fn a_creatures_charge_cut_short_swings_at_nothing() {
    let (_realm, mut a, ga, map) = motion::enter(world()).await;
    let g = motion::wolf();
    a.sees(g).await;
    let (ended, newer, attacking) = on(&map, move |m| {
        charge::start(m, g, ga, true);
        assert!(charge::charging(m, g), "it charges");
        let here = m.creature(g).unwrap().position;
        let spline = m.next_spline_id;
        m.next_spline_id += 1;
        let run = Walk::new(
            vec![here, motion::at(20.0, 40.0)],
            2.5,
            m.now,
            spline,
            false,
        );
        m.creature_mut(g).unwrap().motion = Motion::Moving(run);
        m.now += Duration::from_secs(1);
        charge::step(m, g);
        let c = m.creature(g).unwrap();
        let newer = matches!(&c.motion, Motion::Moving(w) if w.spline_id == spline);
        (
            !charge::charging(m, g),
            newer,
            m.combat(g).is_some_and(|c| c.attacking),
        )
    })
    .await;
    assert!(ended, "the charge is over");
    assert!(newer, "the newer run goes on");
    assert!(!attacking, "no swing");
    let refused = on(&map, move |m| {
        let mut refused = Vec::new();
        m.creature_mut(g).unwrap().position = motion::at(20.0, 20.0);
        m.creature_mut(g).unwrap().motion = Motion::Idle { until: None };
        motion::apply(m, ga, g, motion::ROOT);
        charge::start(m, g, ga, true);
        refused.push(("rooted", !charge::charging(m, g)));
        crate::game::map::auras::remove_spell(m, g, motion::ROOT, None);
        crate::game::map::combat::deal_damage(m, ga, g, 1_000_000);
        charge::start(m, g, ga, true);
        refused.push(("dead", !charge::charging(m, g)));
        refused
    })
    .await;
    for (what, it) in refused {
        assert!(it, "{what}: charged");
    }
}

/// Which charges swing on arriving: a harmful one, not a helpful one, nor
/// one that ends the fight (Thistle Tea's `attack_on_arrival?`: "only
/// hostile spells without the cancellation attribute start attacks").
#[tokio::test]
async fn only_a_harmful_charge_that_keeps_the_fight_swings_on_arrival() {
    let (_realm, mut a, ga, map) = motion::enter(world()).await;
    let g = motion::wolf();
    a.sees(g).await;
    for (spell, swings) in [
        (motion::CHARGE, true),
        (FRIENDLY_CHARGE, false),
        (CALM_CHARGE, false),
    ] {
        let attacking = on(&map, move |m| {
            let p = m.players.get_mut(&ga).unwrap();
            p.movement.position = motion::at(0.0, 0.0);
            p.charge = None;
            crate::game::map::combat::stop_attack(m, ga);
            crate::game::map::spells::cast_triggered(m, ga, spell, Some(g));
            assert!(m.players[&ga].charge.is_some(), "{spell}: it charges");
            later(m, Duration::from_secs(5));
            m.players[&ga].combat.attacking
        })
        .await;
        assert_eq!(attacking, swings, "{spell}");
    }
}
