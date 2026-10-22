//! Round 7's leash (Thistle Tea's `combat_leash_test.exs` and
//! `owned_combat_leash_test.exs`), at wvs's 31-yard radius (mangos's is 30,
//! Thistle Tea's 50): where a fight is anchored, who near its start holds it,
//! the hard leash, what extends it, control, and the clocks a creature and
//! its summons share.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::creature_combat::{self, leash_extended_at, should_evade};
use crate::game::map::{Map, ai, combat};
use crate::game::tests_p3_combat::{self as p3, BRUTE, HELPER};
use crate::game::tests_p6_wspell::put;
use crate::game::tests_p7_combat::{creature_at, off, setup_with};

/// A brute with a hard leash of 30 yards.
const TETHERED: u32 = 3780;

fn data() -> GameData {
    let mut d = crate::game::tests_p7_combat::data();
    let mut t = p3::template(TETHERED, 7);
    t.leash_range = 30.0;
    d.creature_templates.insert(TETHERED, t);
    d
}

/// `g` `x` yards east of where its fight began; `victim` `vx`.
fn far(m: &mut Map, g: Guid, x: f32, victim: Guid, vx: f32) {
    let o = m.creature(g).unwrap().engagement.origin.unwrap();
    put(m, g, Vector3d { x: o.x + x, ..o });
    put(m, victim, Vector3d { x: o.x + vx, ..o });
}

/// Thistle Tea's `should_evade?/3`: a fight is anchored where it began
/// (not at the spawn), and holds more than twelve seconds after the last
/// hostile act on it, or on any creature sharing its clock; it holds
/// while the creature or its victim is within the radius of where it
/// began, and not for a victim gone from the map; and a hard leash holds
/// it to its own distance whatever the clock.
#[tokio::test]
async fn a_fight_holds_near_where_it_began_and_for_twelve_seconds_after_a_blow() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            // A patrol pulled 100 yards from its spawn: anchored there.
            let brute = creature_at(m, ga, BRUTE, off(100.0, 0.0, 0.0));
            m.creature_mut(brute).unwrap().home = off(0.0, 0.0, 0.0);
            put(m, ga, off(98.0, 0.0, 0.0));
            let start = m.now;
            ai::attacked(m, brute, ga);
            let anchored =
                m.creature(brute).unwrap().engagement.origin == Some(off(100.0, 0.0, 0.0));
            far(m, brute, 40.0, ga, 45.0);
            let at = |m: &mut Map, ms: u64| {
                m.now = start + Duration::from_millis(ms);
                should_evade(m, brute, ga)
            };
            let timing = (at(m, 12_000), at(m, 12_001));
            // A creature sharing its clock struck at 8 seconds: 20 then.
            let mate = creature_at(m, ga, BRUTE, off(110.0, 0.0, 0.0));
            m.now = start;
            ai::attacked(m, mate, ga);
            creature_combat::share_leash(m, mate, brute);
            m.now = start + Duration::from_secs(8);
            creature_combat::hostile_act(m, mate);
            let shared = (at(m, 20_000), at(m, 20_001));
            // Late: the victim within 31 of where it began holds it (at
            // 30.5 too, past mangos's 30), 32 doesn't, nor one gone from
            // the map.
            far(m, brute, 40.0, ga, 30.5);
            let victim_near = at(m, 30_000);
            far(m, brute, 40.0, ga, 32.0);
            let victim_far = at(m, 30_000);
            let gone = should_evade(m, brute, Guid::new(0x99));
            // The creature itself within 31: held.
            far(m, brute, 25.0, ga, 60.0);
            let near = at(m, 30_000);
            // A hard leash of 30, a blow just now: 29 holds, 31 doesn't.
            let tethered = creature_at(m, ga, TETHERED, off(200.0, 0.0, 0.0));
            put(m, ga, off(198.0, 0.0, 0.0));
            ai::attacked(m, tethered, ga);
            far(m, tethered, 29.0, ga, 0.0);
            let inside = should_evade(m, tethered, ga);
            far(m, tethered, 31.0, ga, 0.0);
            let outside = should_evade(m, tethered, ga);
            (
                anchored,
                timing,
                shared,
                (victim_near, victim_far, gone, near),
                (inside, outside),
            )
        })
        .await
        .unwrap();
    assert!(r.0, "anchored where it was pulled");
    assert_eq!(r.1, (false, true), "twelve seconds, not a millisecond more");
    assert_eq!(r.2, (false, true), "a mate's blow at 8 seconds: 20");
    assert_eq!(
        r.3,
        (false, true, true, false),
        "(victim at 30.5, victim at 32, victim gone, creature within 31)"
    );
    assert_eq!(r.4, (false, true), "a hard leash, whatever the clock");
}

/// Thistle Tea's `enter/3`: a hostile act in the fight extends its clock
/// without moving where it began; once over, a new fight begins where the
/// creature is then, on a clock of its own.
#[tokio::test]
async fn a_blow_in_a_fight_keeps_its_anchor_and_a_new_fight_takes_another() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = creature_at(m, ga, BRUTE, off(0.0, 0.0, 0.0));
            let start = m.now;
            ai::attacked(m, brute, ga);
            let mate = creature_at(m, ga, BRUTE, off(5.0, 0.0, 0.0));
            ai::attacked(m, mate, ga);
            creature_combat::share_leash(m, brute, mate);
            put(m, brute, off(40.0, 0.0, 0.0));
            m.now = start + Duration::from_secs(10);
            combat::engage(m, ga, brute);
            let kept = (
                m.creature(brute).unwrap().engagement.origin == Some(off(0.0, 0.0, 0.0)),
                leash_extended_at(m, brute) == Some(m.now),
                leash_extended_at(m, mate) == Some(m.now),
            );
            ai::enter_evade(m, brute);
            let over = m.creature(brute).unwrap().engagement.origin.is_none();
            m.creature_mut(brute).unwrap().evading = false;
            m.now = start + Duration::from_secs(20);
            ai::attacked(m, brute, ga);
            let anew = m.creature(brute).unwrap().engagement.origin == Some(off(40.0, 0.0, 0.0));
            // Its own clock now: a blow on it is none of the mate's.
            m.now = start + Duration::from_secs(25);
            creature_combat::hostile_act(m, brute);
            let own = leash_extended_at(m, mate) == Some(start + Duration::from_secs(10));
            (kept, over, anew, own)
        })
        .await
        .unwrap();
    assert_eq!(r, ((true, true, true), true, true, true));
}

/// Thistle Tea's `maintain/2`: rooted, stunned, confused or feared, a
/// creature's leash is extended, every three seconds and not at every
/// look; free, it runs its twelve seconds from the last.
#[tokio::test]
async fn control_extends_the_leash_every_three_seconds_and_freedom_runs_it_out() {
    use crate::game::map::auras::{UNIT_FLAG_CONFUSED, UNIT_FLAG_FLEEING};
    use crate::game::map::player::UNIT_FLAG_STUNNED;
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let mut out = Vec::new();
            for how in 0..4 {
                let brute = creature_at(m, ga, BRUTE, off(0.0, 20.0 * how as f32, 0.0));
                put(m, ga, off(1.0, 20.0 * how as f32, 0.0));
                let start = m.now;
                ai::attacked(m, brute, ga);
                far(m, brute, 60.0, ga, 60.0);
                let flag = [0, UNIT_FLAG_STUNNED, UNIT_FLAG_CONFUSED, UNIT_FLAG_FLEEING][how];
                if how == 0 {
                    p3::aura_of(
                        m,
                        brute,
                        95_320,
                        crate::game::map::auras::AURA_MOD_ROOT,
                        0,
                        0,
                    );
                } else {
                    m.unit_fields_mut(brute)
                        .unwrap()
                        .set_flag(UNIT_FIELD_FLAGS, flag, true);
                }
                let t =
                    |s: u64, ms: u64| start + Duration::from_secs(s) + Duration::from_millis(ms);
                m.now = t(20, 0);
                creature_combat::maintain_leash(m, brute);
                let extended = leash_extended_at(m, brute) == Some(t(20, 0));
                m.now = t(20, 100);
                creature_combat::maintain_leash(m, brute);
                let not_again = leash_extended_at(m, brute) == Some(t(20, 0));
                let held = should_evade(m, brute, ga);
                if how == 0 {
                    crate::game::map::auras::remove_spell(m, brute, 95_320, None);
                } else {
                    m.unit_fields_mut(brute)
                        .unwrap()
                        .set_flag(UNIT_FIELD_FLAGS, flag, false);
                }
                m.now = t(32, 0);
                let free_late = should_evade(m, brute, ga);
                m.now = t(32, 1);
                out.push((
                    extended,
                    not_again,
                    held,
                    free_late,
                    should_evade(m, brute, ga),
                ));
                m.now = start;
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(
        r,
        vec![(true, true, false, false, true); 4],
        "(root, stun, confusion, fear)"
    );
}

/// `summon` summoned by `owner`.
fn summoned_by(m: &mut Map, summon: Guid, owner: Guid) {
    crate::game::map::event_ai::scripted_mut(m, summon)
        .unwrap()
        .summoner = Some(owner);
    crate::game::map::event_ai::scripted_mut(m, owner)
        .unwrap()
        .summoned
        .push(summon);
}

/// Thistle Tea's `owned_combat_leash_test.exs`: a summon that joins its
/// summoner's fight runs on its clock, both ways, where each began kept;
/// the summoner's death leaves the summon its clock; one that fought
/// first is linked when its summoner joins, where it began kept;
/// ownership comes before a call for help; a summon of a player's, or of
/// no one, is on its own; and the summon outlives its summoner's going.
#[tokio::test]
async fn a_summon_and_its_summoner_share_a_clock_before_any_other() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let start = m.now;
            let t = |s: u64| start + Duration::from_secs(s);
            put(m, ga, off(5.0, 0.0, 0.0));
            // The owner fights at 1, its summon at 2: both on one clock.
            let owner = creature_at(m, ga, BRUTE, off(0.0, 0.0, 0.0));
            let summon = creature_at(m, ga, BRUTE, off(10.0, 0.0, 0.0));
            summoned_by(m, summon, owner);
            m.now = t(1);
            combat::engage(m, ga, owner);
            m.now = t(2);
            combat::engage(m, ga, summon);
            let joined = leash_extended_at(m, owner) == Some(t(2));
            let origins = m.creature(owner).unwrap().engagement.origin
                != m.creature(summon).unwrap().engagement.origin;
            m.now = t(9);
            creature_combat::hostile_act(m, summon);
            let both = leash_extended_at(m, owner) == Some(t(9));
            // The owner struck at 10, and dies: the summon keeps it.
            m.now = t(10);
            creature_combat::hostile_act(m, owner);
            combat::kill(m, owner, owner);
            let kept = leash_extended_at(m, summon) == Some(t(10));
            // A summon fighting before its owner is linked as it joins.
            let owner2 = creature_at(m, ga, BRUTE, off(0.0, 30.0, 0.0));
            let early = creature_at(m, ga, BRUTE, off(10.0, 30.0, 0.0));
            summoned_by(m, early, owner2);
            m.now = t(11);
            combat::engage(m, ga, early);
            let early_origin = m.creature(early).unwrap().engagement.origin;
            m.now = t(15);
            creature_combat::hostile_act(m, early);
            m.now = t(20);
            combat::engage(m, ga, owner2);
            let linked = (
                leash_extended_at(m, early) == Some(t(20)),
                m.creature(early).unwrap().engagement.origin == early_origin,
            );
            // Called by another for help, it keeps its owner's clock.
            let other = creature_at(m, ga, BRUTE, off(0.0, 60.0, 0.0));
            m.now = t(21);
            combat::engage(m, ga, other);
            m.now = t(22);
            creature_combat::hostile_act(m, owner2);
            creature_combat::share_leash(m, early, other);
            m.now = t(23);
            creature_combat::hostile_act(m, other);
            let precedence = (
                leash_extended_at(m, early) == Some(t(22)),
                leash_extended_at(m, other) == Some(t(23)),
            );
            // A player's summon, and one of no one's, are their own.
            let mut alone = Vec::new();
            for whose in [Some(ga), None] {
                let s = creature_at(m, ga, HELPER, off(20.0, 90.0, 0.0));
                m.unit_fields_mut(s)
                    .unwrap()
                    .set_u32(UNIT_FIELD_FACTIONTEMPLATE, p3::PACK);
                m.creature_mut(s).unwrap().owner = whose;
                m.now = t(24);
                combat::engage(m, ga, s);
                alone.push(leash_extended_at(m, s) == Some(t(24)));
            }
            // Its summoner gone, the summon stays, its clock its own.
            let owner3 = creature_at(m, ga, BRUTE, off(0.0, 120.0, 0.0));
            let left = creature_at(m, ga, BRUTE, off(10.0, 120.0, 0.0));
            summoned_by(m, left, owner3);
            m.now = t(25);
            combat::engage(m, ga, left);
            crate::game::map::scripts::despawn(m, owner3);
            let stays = m.creature(left).is_some_and(|c| c.alive())
                && leash_extended_at(m, left) == Some(t(25));
            (
                joined, origins, both, kept, linked, precedence, alone, stays,
            )
        })
        .await
        .unwrap();
    assert!(r.0, "the summon's blow is the owner's");
    assert!(r.1, "each where it began");
    assert!(r.2, "and on");
    assert!(r.3, "the owner's death leaves it");
    assert_eq!(
        r.4,
        (true, true),
        "linked as the owner joins, its anchor kept"
    );
    assert_eq!(r.5, (true, true), "its owner's clock before a caller's");
    assert_eq!(r.6, vec![true, true], "a player's or no one's, its own");
    assert!(r.7, "it outlives its owner's going");
}

/// The creatures' turns of 50 ms, for `ms`: what `each` says of the map
/// after each.
fn turns<T>(m: &mut Map, ms: u64, mut each: impl FnMut(&mut Map) -> T) -> Vec<T> {
    (0..ms / 50)
        .map(|_| {
            m.now += Duration::from_millis(50);
            ai::update_creatures(m);
            combat::update_creatures(m);
            each(m)
        })
        .collect()
}

/// `g` hardy, so a long fight doesn't kill it.
fn hardy(m: &mut Map, g: Guid) {
    let f = m.unit_fields_mut(g).unwrap();
    f.set_u32(UNIT_FIELD_MAXHEALTH, 10_000_000);
    f.set_u32(UNIT_FIELD_HEALTH, 10_000_000);
}

/// The user's rule: a creature that has stood still in reach of its
/// victim, attacking it, for `[creatures] leash_standing_ms` (1.25 s) has
/// that count as a hostile act, renewing its leash each 1.25 s it stands
/// and attacks (`creature_combat::stood_attacking`). Forty yards from
/// where its fight began, its victim beside it as far, nothing done to it
/// for twenty seconds: it fights on, its leash renewed every 1.25 s from
/// its first 1.25 s standing, never more than that ago.
#[tokio::test]
async fn a_creature_standing_and_swinging_beyond_its_leash_fights_on() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = creature_at(m, ga, BRUTE, off(0.0, 0.0, 0.0));
            put(m, ga, off(1.5, 0.0, 0.0));
            hardy(m, ga);
            ai::attacked(m, brute, ga);
            let start = m.now;
            put(m, brute, off(40.0, 0.0, 0.0));
            put(m, ga, off(41.5, 0.0, 0.0));
            let seen = turns(m, 20_000, |m| {
                (
                    m.now,
                    p3_attacking(m, brute, ga),
                    leash_extended_at(m, brute),
                )
            });
            let fought = seen.iter().all(|(_, on, _)| *on);
            let mut renewals: Vec<Instant> = seen.iter().filter_map(|(_, _, at)| *at).collect();
            renewals.dedup();
            let oldest = seen
                .iter()
                .map(|(now, _, at)| at.map_or(Duration::MAX, |at| now.duration_since(at)))
                .max()
                .unwrap();
            let swung = m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_HEALTH) < 10_000_000;
            (start, fought, renewals, oldest, swung)
        })
        .await
        .unwrap();
    let (start, fought, renewals, oldest, swung) = r;
    assert!(fought, "it fights on throughout");
    assert!(swung, "and swings at its victim");
    let every = Duration::from_millis(1250);
    assert_eq!(renewals[0], start, "its leash from the fight's start");
    assert!(
        renewals.len() >= 15,
        "renewed as it stands: {}",
        renewals.len()
    );
    for w in renewals[1..].windows(2) {
        assert_eq!(w[1] - w[0], every, "each 1.25 s");
    }
    assert!(oldest <= every, "never older than 1.25 s: {oldest:?}");
}

/// And one chasing its victim, beyond its leash, nothing done to it,
/// goes home once the leash's twelve seconds are up: chasing isn't
/// standing. With `leash_standing_ms` longer than the fight, one that
/// stands and swings goes home as well.
#[tokio::test]
async fn a_creature_chasing_with_no_hostile_act_goes_home_after_the_timeout() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = creature_at(m, ga, BRUTE, off(0.0, 0.0, 0.0));
            put(m, ga, off(1.5, 0.0, 0.0));
            hardy(m, ga);
            ai::attacked(m, brute, ga);
            let start = m.now;
            put(m, brute, off(40.0, 0.0, 0.0));
            put(m, ga, off(60.0, 0.0, 0.0));
            // Its victim keeps twenty yards ahead of it.
            let seen = turns(m, 14_000, |m| {
                let at = m.creature(brute).unwrap().position;
                let stood = m.creature(brute).unwrap().engagement.standing_since;
                if p3_attacking(m, brute, ga) {
                    put(
                        m,
                        ga,
                        Vector3d {
                            x: at.x + 20.0,
                            ..at
                        },
                    );
                }
                (m.now, p3_attacking(m, brute, ga), stood)
            });
            let went = seen
                .iter()
                .find(|(_, on, _)| !on)
                .map(|(now, _, _)| now.duration_since(start));
            let stood = seen.iter().any(|(_, _, s)| s.is_some());
            (went, stood)
        })
        .await
        .unwrap();
    let (went, stood) = r;
    assert!(!stood, "it never stood");
    let went = went.expect("it went home");
    assert!(
        went > Duration::from_secs(12) && went <= Duration::from_millis(12_100),
        "at the leash's twelve seconds: {went:?}"
    );
    // Standing, but the setting longer than the fight: home as well.
    let went = map
        .query(move |m| {
            let mut cfg = (*m.cfg).clone();
            cfg.creatures.leash_standing_ms = 600_000;
            m.cfg = std::sync::Arc::new(cfg);
            let brute = creature_at(m, ga, BRUTE, off(100.0, 0.0, 0.0));
            put(m, ga, off(101.5, 0.0, 0.0));
            ai::attacked(m, brute, ga);
            let start = m.now;
            put(m, brute, off(140.0, 0.0, 0.0));
            put(m, ga, off(141.5, 0.0, 0.0));
            turns(m, 14_000, |m| (m.now, p3_attacking(m, brute, ga)))
                .into_iter()
                .find(|(_, on)| !on)
                .map(|(now, _)| now.duration_since(start))
        })
        .await
        .unwrap();
    let went = went.expect("it went home");
    assert!(
        went > Duration::from_secs(12) && went <= Duration::from_millis(12_100),
        "standing, at the leash's twelve seconds: {went:?}"
    );
}

fn p3_attacking(m: &Map, g: Guid, victim: Guid) -> bool {
    crate::game::tests_p7_combat::attacking(m, g, victim)
}

/// The leash's time goes by the creature's level, in 1.12's brackets (11 s
/// to level 29, 12 to 39, 13 to 44, 14 to 49, 15 from 50), and it is looked
/// at every `[creatures] leash_check_ms` from the fight's start: a creature
/// goes home at the first look past its time, so up to a look late, never
/// early.
#[tokio::test]
async fn the_leash_runs_by_level_and_is_looked_at_on_its_tick() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let mut cfg = (*m.cfg).clone();
            cfg.creatures.leash_timeout_by_level =
                crate::config::CreaturesConfig::default().leash_timeout_by_level;
            cfg.creatures.leash_check_ms = 0;
            m.cfg = std::sync::Arc::new(cfg.clone());
            let mut brackets = Vec::new();
            for (level, secs) in [
                (10, 11),
                (29, 11),
                (30, 12),
                (39, 12),
                (40, 13),
                (44, 13),
                (45, 14),
                (49, 14),
                (50, 15),
                (60, 15),
            ] {
                let brute = creature_at(m, ga, BRUTE, off(100.0, 0.0, 0.0));
                m.unit_fields_mut(brute)
                    .unwrap()
                    .set_u32(UNIT_FIELD_LEVEL, level);
                put(m, ga, off(98.0, 0.0, 0.0));
                let start = m.now;
                ai::attacked(m, brute, ga);
                far(m, brute, 40.0, ga, 45.0);
                let at = |m: &mut Map, ms: u64| {
                    m.now = start + Duration::from_millis(ms);
                    should_evade(m, brute, ga)
                };
                let edge = (at(m, secs * 1000), at(m, secs * 1000 + 1));
                m.now = start;
                brackets.push((level, edge));
                m.remove_creature(brute);
            }
            // Looked at each second from the fight's start, a blow 0.9 s
            // in: past its 11 s at 11.9, but looked at only at 12.
            cfg.creatures.leash_check_ms = 1000;
            m.cfg = std::sync::Arc::new(cfg);
            let brute = creature_at(m, ga, BRUTE, off(100.0, 0.0, 0.0));
            m.unit_fields_mut(brute)
                .unwrap()
                .set_u32(UNIT_FIELD_LEVEL, 20);
            put(m, ga, off(98.0, 0.0, 0.0));
            let start = m.now;
            ai::attacked(m, brute, ga);
            m.now = start + Duration::from_millis(900);
            creature_combat::hostile_act(m, brute);
            far(m, brute, 40.0, ga, 45.0);
            let at = |m: &mut Map, ms: u64| {
                m.now = start + Duration::from_millis(ms);
                should_evade(m, brute, ga)
            };
            let ticked = (at(m, 11_950), at(m, 11_999), at(m, 12_000));
            (brackets, ticked)
        })
        .await
        .unwrap();
    for (level, edge) in r.0 {
        assert_eq!(
            edge,
            (false, true),
            "level {level}: its bracket's time, to the millisecond"
        );
    }
    assert_eq!(
        r.1,
        (false, false, true),
        "past its time at 11.9 s, home at the next look (12 s), never before"
    );
}

/// A hostile creature just come into the world lets a player beside it be
/// for `[creatures] spawn_grace_ms` (6 s), then attacks; one attacked
/// meanwhile fights at once.
#[tokio::test]
async fn a_creature_just_spawned_lets_those_about_it_be_for_six_seconds() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let mut cfg = (*m.cfg).clone();
            cfg.creatures.spawn_grace_ms = 6000;
            m.cfg = std::sync::Arc::new(cfg);
            put(m, ga, off(0.0, 0.0, 0.0));
            hardy(m, ga);
            let start = m.now;
            let calm = creature_at(
                m,
                ga,
                crate::game::tests_p7_combat::WATCHER,
                off(4.0, 0.0, 0.0),
            );
            // From a spawn of the world's, as a respawn is.
            m.creature_mut(calm).unwrap().spawn_guid = Some(1);
            let seen = turns(m, 8_000, |m| (m.now, p3_attacking(m, calm, ga)));
            let first = seen
                .iter()
                .find(|(_, on)| *on)
                .map(|(at, _)| at.duration_since(start));
            // Attacked inside its grace, it fights at once.
            let struck = creature_at(
                m,
                ga,
                crate::game::tests_p7_combat::WATCHER,
                off(-4.0, 0.0, 0.0),
            );
            ai::attacked(m, struck, ga);
            let fights = turns(m, 500, |m| p3_attacking(m, struck, ga))
                .into_iter()
                .any(|on| on);
            // A summon (no spawn of the world's) comes to fight at once.
            let summoned = creature_at(
                m,
                ga,
                crate::game::tests_p7_combat::WATCHER,
                off(0.0, 4.0, 0.0),
            );
            let at_once = turns(m, 1_000, |m| p3_attacking(m, summoned, ga))
                .into_iter()
                .any(|on| on);
            (first, fights, at_once)
        })
        .await
        .unwrap();
    let first = r.0.expect("it attacked once its grace was over");
    assert!(
        first >= Duration::from_secs(6) && first <= Duration::from_millis(7_500),
        "not before six seconds: {first:?}"
    );
    assert!(r.1, "attacked in its grace, it fights");
    assert!(r.2, "a summon has no grace");
}
