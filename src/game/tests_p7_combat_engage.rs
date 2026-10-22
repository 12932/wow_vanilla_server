//! Round 7's engagement (Thistle Tea's `engagement_test.exs`): what a
//! pull sets, a pet's victim against another attacker and its answer to a
//! blow, stopping only the attack, and a summon's timer after its fight.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::{Map, MapKey, ai, combat};
use crate::game::tests_p3_combat::{self as p3, BRUTE};
use crate::game::tests_p6_wspell::put;
use crate::game::tests_p7_combat::{BEAST_WATCHER, creature_at, off, setup};

/// Thistle Tea's "changes all combat identity through one typed
/// transition" and `stop_attack/1`: a pull puts the creature in combat
/// with its puller at no threat, targeted, its fight's last hostile act
/// now; stopping only its attack ends its swing and its queued strike,
/// and keeps its list, its tap, its flag, its cast and its swing timer; a
/// blow then starts it again.
#[tokio::test]
async fn a_pull_sets_the_fight_and_stopping_the_attack_keeps_the_rest() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            put(m, ga, off(0.0, 0.0, 0.0));
            let brute = creature_at(m, ga, BRUTE, off(2.0, 0.0, 0.0));
            let now = m.now;
            ai::attacked(m, brute, ga);
            let c = m.creature(brute).unwrap();
            let pulled = (
                c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT),
                c.threat.clone() == vec![(ga, 0.0)],
                c.fields.get_guid(UNIT_FIELD_TARGET) == ga,
                c.combat.attacking && c.combat.victim == ga,
                c.engagement.last_hostile == Some(now),
                // A pull taps no one: only damage does (mangos's
                // `DealDamage`, not Thistle Tea's tap on combat entry).
                c.tapped_by.is_none(),
            );
            let realm = m.realm.clone();
            crate::game::map::tap::take(&realm, m.creature_mut(brute).unwrap(), ga);
            let swing_at = now + Duration::from_millis(1234);
            {
                let c = m.creature_mut(brute).unwrap();
                c.combat.queued = Some(95_340);
                c.combat.next_swing = Some(swing_at);
            }
            let cast = crate::game::map::spells::Casting {
                spell: 95_341,
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
            };
            m.caster_mut(brute).unwrap().casting = Some(cast);
            combat::stop_attack(m, brute);
            let c = m.creature(brute).unwrap();
            let stopped = (
                !c.combat.attacking,
                c.combat.queued.is_none(),
                c.threat.clone() == vec![(ga, 0.0)],
                c.tapped_by == Some(ga),
                c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT),
                c.caster.casting.is_some(),
                c.combat.next_swing == Some(swing_at),
            );
            combat::engage(m, ga, brute);
            let c = m.creature(brute).unwrap();
            let again = c.combat.attacking && c.fields.get_guid(UNIT_FIELD_TARGET) == ga;
            (pulled, stopped, again)
        })
        .await
        .unwrap();
    assert_eq!(
        r.0,
        (true, true, true, true, true, true),
        "(flag, list, target, swing, last act, no tap)"
    );
    assert_eq!(
        r.1,
        (true, true, true, true, true, true, true),
        "(swing, queued, list, tap, flag, cast, timer)"
    );
    assert!(r.2, "a blow starts it again");
}

/// Thistle Tea's "keeps an explicitly commanded pet victim while tracking
/// another attacker" and "pet damage requests retaliation": a pet sent at
/// one, struck by another, keeps at the one it was sent at, the other
/// among its attackers; an idle defensive pet struck turns on its
/// attacker, and a passive one doesn't.
#[tokio::test]
async fn a_sent_pet_keeps_its_victim_and_an_idle_one_answers_a_blow_by_its_stance() {
    let realm = crate::game::harness::realm_with(crate::game::tests_p7_combat::data());
    let mut h = crate::game::harness::Client::connect(&realm, "H");
    let gh = h.enter("Hana", Race::Dwarf, Class::Hunter).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let r = map
        .query(move |m| {
            p3::level(m, gh, 60);
            put(m, gh, off(0.0, 0.0, 0.0));
            crate::game::map::pets::give_pet(m, gh, BEAST_WATCHER, 60).expect("a pet");
            let pet = crate::game::map::pets::pet_of(m, gh).unwrap();
            put(m, pet, off(1.0, 0.0, 0.0));
            let sent_at = creature_at(m, gh, BRUTE, off(2.0, 0.0, 0.0));
            let other = creature_at(m, gh, BRUTE, off(1.0, 1.0, 0.0));
            crate::game::map::pets::attack_target(m, pet, sent_at);
            combat::engage(m, other, pet);
            let c = m.combat(pet).unwrap();
            let kept = (
                c.attacking && c.victim == sent_at,
                m.unit_fields(pet).unwrap().get_guid(UNIT_FIELD_TARGET) == sent_at,
                c.attackers.contains(&other),
            );
            // Called back, idle; struck, by its stance.
            crate::game::map::pets::stop_where(m, pet);
            combat::leave_combat(m, pet);
            let mut answers = Vec::new();
            for react in [
                crate::game::map::pets::REACT_DEFENSIVE,
                crate::game::map::pets::REACT_PASSIVE,
            ] {
                combat::leave_combat(m, pet);
                crate::game::map::pets::state_mut(m, pet).unwrap().react = react;
                let striker = creature_at(m, gh, BRUTE, off(1.0, -1.0, 0.0));
                combat::engage(m, striker, pet);
                answers.push(
                    m.combat(pet)
                        .is_some_and(|c| c.attacking && c.victim == striker),
                );
            }
            (kept, answers)
        })
        .await
        .unwrap();
    assert_eq!(
        r.0,
        (true, true, true),
        "(at the one it was sent at, shown, the other tracked)"
    );
    assert_eq!(
        r.1,
        vec![true, false],
        "(defensive answers, passive doesn't)"
    );
}

/// Thistle Tea's "starts a timed-or-dead summon's timer over when its
/// fight ends": a summon that waits out its fights (vmangos's
/// timed-or-dead) has its time start again when one ends, not at its
/// death; a plain timed one keeps its time.
#[tokio::test]
async fn a_summon_that_waits_out_fights_starts_its_time_again_after_one() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            put(m, ga, off(0.0, 0.0, 0.0));
            let start = m.now;
            let summon = |m: &mut Map, kind: i32, y: f32| -> Guid {
                let g = creature_at(m, ga, BRUTE, off(2.0, y, 0.0));
                let s = crate::game::map::event_ai::scripted_mut(m, g).unwrap();
                s.despawn_type = kind;
                s.despawn_after = Some(Duration::from_secs(10));
                s.despawn_at = Some(start + Duration::from_secs(10));
                g
            };
            let at = |m: &Map, g: Guid| {
                m.creature(g)
                    .and_then(|c| c.scripted.as_deref())
                    .and_then(|s| s.despawn_at)
            };
            let waits = summon(m, 1, 0.0);
            let timed = summon(m, 3, 5.0);
            let dies = summon(m, 1, 10.0);
            for g in [waits, timed, dies] {
                ai::attacked(m, g, ga);
            }
            m.now = start + Duration::from_secs(4);
            ai::enter_evade(m, waits);
            ai::enter_evade(m, timed);
            let restarted = at(m, waits) == Some(m.now + Duration::from_secs(10));
            let kept = at(m, timed) == Some(start + Duration::from_secs(10));
            combat::kill(m, dies, dies);
            let not_at_death = at(m, dies) == Some(start + Duration::from_secs(10));
            (restarted, kept, not_at_death)
        })
        .await
        .unwrap();
    assert_eq!(
        r,
        (true, true, true),
        "(restarted after a fight, timed kept, not at death)"
    );
}
