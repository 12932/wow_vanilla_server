//! Phase 8's loader round, what players feel in a fight (Thistle Tea's
//! `world/loader/spell_interrupt_dbc_test.exs`, `stealth_detection_dbc_test.exs`,
//! `daze_dbc_test.exs`): the real interrupts and their lockouts, the
//! detection buffs, and Daze.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::fields::*;
use crate::game::map::auras;
use crate::game::map::spells;
use crate::game::map::spells::stealth;
use crate::game::map::{Map, MapHandle};

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

const FIREBALL: u32 = 133;
const COUNTERSPELL: u32 = 2139;
const KICK: u32 = 1766;
const PUMMEL: u32 = 6552;
const SHIELD_BASH: u32 = 72;
const EARTH_SHOCK: u32 = 8042;
const DRAIN_LIFE: u32 = 689;
/// A Kobold Vermin that can't be interrupted (mechanic 26 in its mask, as
/// Thistle Tea's test has it).
const UNINTERRUPTIBLE: u32 = 990_850;

/// The lockouts on `g`'s schools of `mask`: how long each has left, in ms.
fn locked(m: &Map, g: Guid, mask: u32) -> Vec<u128> {
    m.caster(g).map_or_else(Vec::new, |c| {
        c.school_lockout
            .iter()
            .filter(|(s, until)| s & mask != 0 && *until > m.now)
            .map(|(_, until)| until.saturating_duration_since(m.now).as_millis())
            .collect()
    })
}

/// Thistle Tea's interrupts (`spell_interrupt_dbc_test.exs`): Counterspell,
/// Kick, Pummel, Shield Bash and Earth Shock each stop a creature's
/// Fireball and lock its fire out for their own 10, 5, 4, 6 and 2
/// seconds, and no longer: once the time is up it casts fire again.
#[tokio::test]
#[ignore = "extended"]
async fn the_class_interrupts_lock_the_school_out_for_their_own_time() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P8LKICK", "Kickera", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let mut seen = Vec::new();
        for id in [COUNTERSPELL, KICK, PUMMEL, SHIELD_BASH, EARTH_SHOCK] {
            let k = dummy(m, g, 60, 3.0);
            fresh(m, g);
            let began = spells::cast(m, k, FIREBALL, at(g), None);
            spells::cast_triggered(m, g, id, Some(k));
            let stopped = casting(m, k).is_none();
            let lock = locked(m, k, 4);
            let ms = lock.first().copied().unwrap_or(0) as u64;
            m.now += Duration::from_millis(ms.saturating_sub(1));
            let still = !locked(m, k, 4).is_empty() && !spells::cast(m, k, FIREBALL, at(g), None);
            m.now += Duration::from_millis(1);
            let again = locked(m, k, 4).is_empty() && spells::cast(m, k, FIREBALL, at(g), None);
            seen.push((id, began, stopped, ms, still, again));
        }
        seen
    })
    .await;
    assert_eq!(
        out,
        vec![
            (COUNTERSPELL, true, true, 10_000, true, true),
            (KICK, true, true, 5000, true, true),
            (PUMMEL, true, true, 4000, true, true),
            (SHIELD_BASH, true, true, 6000, true, true),
            (EARTH_SHOCK, true, true, 2000, true, true),
        ]
    );
}

/// Thistle Tea's "Counterspell cancels the real Drain Life channel": a
/// warlock's channel ends, its channel field is cleared, the drain comes
/// off its target, and shadow is locked out for ten seconds.
#[tokio::test]
#[ignore = "extended"]
async fn counterspell_ends_a_drain_life_and_locks_out_shadow() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P8LDRAIN", "Drainer", Race::Human, Class::Warlock).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[DRAIN_LIFE]);
        let foe = dummy(m, g, 60, 10.0);
        let began = cast_fresh(m, g, DRAIN_LIFE, at(foe));
        let channel = |m: &Map| {
            m.unit_fields(g)
                .map_or(0, |f| f.get_u32(UNIT_CHANNEL_SPELL))
        };
        let channelling = (channel(m), has(m, foe, DRAIN_LIFE));
        spells::cast_triggered(m, foe, COUNTERSPELL, Some(g));
        (
            began,
            channelling,
            casting(m, g),
            channel(m),
            has(m, foe, DRAIN_LIFE),
            locked(m, g, 32),
        )
    })
    .await;
    assert!(out.0, "the drain begins");
    assert_eq!(out.1, (DRAIN_LIFE, true), "channelled, on its target");
    assert_eq!((out.2, out.3, out.4), (None, 0, false), "and ended");
    assert_eq!(out.5, vec![10_000], "shadow locked out ten seconds");
}

/// Thistle Tea's "interrupt-immune creatures take Kick damage without
/// losing their cast" (mangos's `IsImmuneToSpellEffect` by the effect's
/// mechanic, 26): Kick hurts it, and its Fireball goes on, its fire not
/// locked out.
#[tokio::test]
#[ignore = "extended"]
async fn a_creature_immune_to_interrupts_takes_the_kick_and_keeps_casting() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        let mut t = d.creature_template(6).cloned().unwrap();
        t.entry = UNINTERRUPTIBLE;
        t.mechanic_immune_mask = 1 << 25;
        d.creature_templates.insert(UNINTERRUPTIBLE, t);
    });
    let (_c, g, map) = player(&realm, "P8LIMMUNE", "Kickerb", Race::Human, Class::Rogue).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let k = spawn(m, g, UNINTERRUPTIBLE, 60, 3.0);
        let began = spells::cast(m, k, FIREBALL, at(g), None);
        spells::cast_triggered(m, g, KICK, Some(k));
        (
            began,
            health(m, k) < 1_000_000,
            casting(m, k),
            locked(m, k, 4),
        )
    })
    .await;
    assert_eq!(out, (true, true, Some(FIREBALL), vec![]));
}

/// `g`'s stealth detection: the sum of its detection auras for stealth
/// (misc 0; traps are 1).
fn detection(m: &Map, g: Guid) -> i32 {
    auras::auras(m, g).map_or(0, |a| a.sum_misc(auras::AURA_MOD_STEALTH_DETECT, 0))
}

const PERCEPTION: u32 = 20_600;
const STEALTH: u32 = 1784;

/// Thistle Tea's detection (`stealth_detection_dbc_test.exs`): Perception
/// gives 50 detection, Paranoia and Track Hidden 30, Detect Traps none
/// (its detection is of traps), No Stealth Detection takes 500. A level-10
/// player with Perception sees a stealthed level-10 kobold 25 yards off;
/// once Perception runs out (its 20 seconds) or is cancelled, no longer,
/// and the detection is back to none.
#[tokio::test]
#[ignore = "extended"]
async fn detection_buffs_see_further_until_they_end() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P8LSPOT", "Spotter", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        ready(m, g, 10, &[]);
        let mut bonus = Vec::new();
        for id in [PERCEPTION, 19_480, 19_885, 2836, 23_330] {
            spells::cast_triggered(m, g, id, Some(g));
            bonus.push((id, detection(m, g)));
            auras::remove_spell(m, g, id, None);
        }
        let k = dummy(m, g, 10, 25.0);
        spells::cast_triggered(m, k, STEALTH, Some(k));
        let plain = (has(m, k, STEALTH), stealth::sees(m, g, k));
        spells::cast_triggered(m, g, PERCEPTION, Some(g));
        let lasts = holder(m, g, PERCEPTION).and_then(|h| h.duration);
        let seen = stealth::sees(m, g, k);
        pass(m, 19_999);
        let late = stealth::sees(m, g, k);
        pass(m, 1);
        let expired = (
            has(m, g, PERCEPTION),
            stealth::sees(m, g, k),
            detection(m, g),
        );
        spells::cast_triggered(m, g, PERCEPTION, Some(g));
        auras::cancel(m, g, PERCEPTION);
        let cancelled = (
            has(m, g, PERCEPTION),
            stealth::sees(m, g, k),
            detection(m, g),
        );
        (bonus, plain, lasts, seen, late, expired, cancelled)
    })
    .await;
    let (bonus, plain, lasts, seen, late, expired, cancelled) = out;
    assert_eq!(
        bonus,
        vec![
            (PERCEPTION, 50),
            (19_480, 30),
            (19_885, 30),
            (2836, 0),
            (23_330, -500)
        ]
    );
    assert_eq!(plain, (true, false), "stealthed, unseen at 25 yards");
    assert_eq!(lasts, Some(20_000));
    assert!(seen && late, "seen while Perception lasts");
    assert_eq!(expired, (false, false, 0), "gone when it runs out");
    assert_eq!(cancelled, (false, false, 0), "or is cancelled");
}

/// Thistle Tea's "death removes stealth and temporary detection bonuses":
/// a rogue stealthed with Perception on, killed, has neither.
#[tokio::test]
#[ignore = "extended"]
async fn death_takes_stealth_and_detection() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P8LDIES", "Diesneak", Race::Human, Class::Rogue).await;
    let out = on(&map, move |m| {
        ready(m, g, 10, &[]);
        let foe = dummy(m, g, 10, 5.0);
        spells::cast_triggered(m, g, STEALTH, Some(g));
        spells::cast_triggered(m, g, PERCEPTION, Some(g));
        let stealthed =
            |m: &Map| auras::auras(m, g).is_some_and(|a| a.has(auras::AURA_MOD_STEALTH));
        let before = (stealthed(m), detection(m, g));
        crate::game::map::combat::deal_damage(m, foe, g, 1_000_000);
        (before, stealthed(m), detection(m, g))
    })
    .await;
    assert_eq!(out, ((true, 50), false, 0));
}

const DAZED: u32 = 1604;

/// Thistle Tea's Daze (`daze_dbc_test.exs`): 1604 halves a player's run
/// speed (7 to 3.5) for four seconds; a second Daze a second later starts
/// the four seconds again, still at half; when it runs out the speed is
/// back to 7.
#[tokio::test]
#[ignore = "extended"]
async fn daze_halves_the_run_refreshes_and_runs_out() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    assert_eq!(d.spells.get(DAZED).unwrap().name, "Dazed");
    let (_c, g, map) = player(&realm, "P8LDAZE", "Dazeda", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        ready(m, g, 30, &[]);
        let foe = dummy(m, g, 30, 3.0);
        let run = |m: &Map| m.players[&g].speeds.run;
        let left = |m: &Map| {
            holder(m, g, DAZED)
                .and_then(|h| h.expires)
                .map(|t| t.saturating_duration_since(m.now).as_millis())
        };
        let before = run(m);
        spells::cast_triggered(m, foe, DAZED, Some(g));
        let slowed = (run(m), left(m));
        pass(m, 1000);
        spells::cast_triggered(m, foe, DAZED, Some(g));
        let count =
            auras::auras(m, g).map_or(0, |a| a.holders.iter().filter(|h| h.spell == DAZED).count());
        let refreshed = (run(m), left(m), count);
        pass(m, 4000);
        (before, slowed, refreshed, has(m, g, DAZED), run(m))
    })
    .await;
    assert_eq!(out.0, 7.0);
    assert_eq!(out.1, (3.5, Some(4000)), "half, for four seconds");
    assert_eq!(out.2, (3.5, Some(4000), 1), "refreshed, one Daze");
    assert_eq!((out.3, out.4), (false, 7.0), "run out, back to 7");
}
