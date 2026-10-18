//! Phase 3's combat tests, part two (Thistle Tea's `threat_test.exs`): a
//! creature's threat list, what damage and spells put on it and take off
//! it, taunts, Fade's temporary threat, and whom the creature fights.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::spells::threat_sync;
use crate::game::map::{Map, ai, combat, spells};
use crate::game::tests_p3_combat::*;

/// `g`'s threat on `creature`, if it is on the list.
fn threat(m: &Map, creature: Guid, g: Guid) -> Option<f32> {
    m.creature(creature)?
        .threat
        .iter()
        .find(|(t, _)| *t == g)
        .map(|(_, v)| *v)
}

/// A creature fighting `victim`, with `list` for its threat.
fn fighting(m: &mut Map, g: Guid, victim: Guid, list: &[(Guid, f32)]) -> Guid {
    let brute = spawn(m, g, BRUTE, 20, 1.0);
    let c = m.creature_mut(brute).unwrap();
    c.combat.victim = victim;
    c.combat.attacking = !victim.is_zero();
    c.threat = list.to_vec();
    // Fighting, it is one the update looks at (`Map::awake`).
    m.note_fighting(brute);
    brute
}

#[tokio::test]
async fn threat_adds_up_never_falls_below_none_and_a_creature_holds_none_on_itself() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (added, seeded, own, cut, missing) = map
        .query(move |m| {
            let brute = fighting(m, ga, Guid::zero(), &[]);
            ai::add_threat(m, brute, ga, 50.0);
            ai::add_threat(m, brute, ga, 25.0);
            let added = threat(m, brute, ga);
            let other = spawn(m, ga, HELPER, 20, 2.0);
            ai::add_threat(m, brute, other, 0.0);
            let seeded = threat(m, brute, other);
            ai::add_threat(m, brute, brute, 50.0);
            ai::add_threat(m, brute, Guid::zero(), 50.0);
            let own = m.creature(brute).unwrap().threat.len();
            // A Feint past all of it leaves none, not less.
            ai::add_threat(m, brute, ga, -150.0);
            let cut = threat(m, brute, ga);
            // And puts no one new on the list.
            let third = spawn(m, ga, HELPER, 20, 2.0);
            ai::add_threat(m, brute, third, -10.0);
            (added, seeded, own, cut, threat(m, brute, third))
        })
        .await
        .unwrap();
    assert_eq!(added, Some(75.0));
    assert_eq!(seeded, Some(0.0), "none is still a place on the list");
    assert_eq!(own, 2);
    assert_eq!(cut, Some(0.0));
    assert_eq!(missing, None);
}

#[tokio::test]
async fn a_threat_spell_puts_its_caster_on_the_list_and_a_negative_one_only_lowers_it() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (made, lowered, absent) = map
        .query(move |m| {
            let brute = fighting(m, ga, Guid::zero(), &[]);
            let helper = spawn(m, ga, HELPER, 20, 2.0);
            spells::cast_triggered(m, ga, THREAT_UP, Some(brute));
            let made = threat(m, brute, ga);
            ai::add_threat(m, brute, ga, 20.0);
            spells::cast_triggered(m, ga, THREAT_DOWN, Some(brute));
            let lowered = threat(m, brute, ga);
            spells::cast_triggered(m, helper, THREAT_DOWN, Some(brute));
            (made, lowered, threat(m, brute, helper))
        })
        .await
        .unwrap();
    assert_eq!(made, Some(10.0));
    assert_eq!(lowered, Some(20.0));
    assert_eq!(absent, None);
}

#[tokio::test]
async fn damage_puts_its_threat_on_the_creature_it_hits() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let dealt = map
        .query(move |m| {
            let brute = fighting(m, ga, Guid::zero(), &[]);
            combat::deal_damage(m, ga, brute, 40);
            threat(m, brute, ga)
        })
        .await
        .unwrap();
    assert_eq!(dealt, Some(40.0));
}

#[tokio::test]
async fn an_attack_me_brings_the_taunter_level_with_the_creatures_victim() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (matched, own, idle, above, seeded) = map
        .query(move |m| {
            let a = spawn(m, ga, HELPER, 20, 2.0);
            let b = spawn(m, ga, HELPER, 20, 2.0);
            let c = spawn(m, ga, HELPER, 20, 2.0);
            // Its victim's, whoever else is above.
            let brute = fighting(m, ga, a, &[(a, 200.0), (b, 50.0), (c, 250.0)]);
            threat_sync::matched(m, brute, b);
            let matched = [
                threat(m, brute, a),
                threat(m, brute, b),
                threat(m, brute, c),
            ];
            // Its own victim taunting changes nothing.
            let brute = fighting(m, ga, a, &[(a, 300.0), (b, 50.0)]);
            threat_sync::matched(m, brute, a);
            let own = [threat(m, brute, a), threat(m, brute, b)];
            // Without a victim, there's nothing to match.
            let brute = fighting(m, ga, Guid::zero(), &[(a, 100.0)]);
            threat_sync::matched(m, brute, b);
            let idle = threat(m, brute, b);
            // Above the victim, the taunter comes down to it.
            let brute = fighting(m, ga, a, &[(a, 100.0), (b, 125.0)]);
            threat_sync::matched(m, brute, b);
            let above = threat(m, brute, b);
            // The spell, from one not on the list: on it, at the victim's.
            let brute = fighting(m, ga, a, &[(a, 200.0)]);
            spells::cast_triggered(m, ga, ATTACK_ME, Some(brute));
            let seeded = (
                threat(m, brute, ga),
                m.creature(brute).unwrap().combat.victim == ga,
            );
            (matched, own, idle, above, seeded)
        })
        .await
        .unwrap();
    assert_eq!(matched, [Some(200.0), Some(200.0), Some(250.0)]);
    assert_eq!(own, [Some(300.0), Some(50.0)]);
    assert_eq!(idle, None);
    assert_eq!(above, Some(100.0));
    assert_eq!(seeded, (Some(200.0), true));
}

#[tokio::test]
async fn fades_threat_is_held_apart_and_given_back_whole() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (clamped, restored, refresh, replaced, untracked, discarded) = map
        .query(move |m| {
            let a = spawn(m, ga, HELPER, 20, 2.0);
            // 100 less 600 is none; back, it is the 600 whole.
            let brute = fighting(m, ga, a, &[(a, 100.0)]);
            threat_sync::set_temporary(m, brute, a, -600.0);
            let clamped = threat(m, brute, a);
            threat_sync::set_temporary(m, brute, a, 0.0);
            let restored = threat(m, brute, a);
            // The same again changes nothing; another undoes the first.
            let brute = fighting(m, ga, a, &[(a, 800.0)]);
            threat_sync::set_temporary(m, brute, a, -200.0);
            threat_sync::set_temporary(m, brute, a, -200.0);
            let refresh = threat(m, brute, a);
            threat_sync::set_temporary(m, brute, a, -300.0);
            let replaced = threat(m, brute, a);
            // One not on the list isn't put on it.
            let brute = fighting(m, ga, Guid::zero(), &[]);
            threat_sync::set_temporary(m, brute, a, -600.0);
            threat_sync::set_temporary(m, brute, a, 0.0);
            let untracked = (
                threat(m, brute, a),
                m.spells.temp_threat.contains_key(&(brute, a)),
            );
            // Forgotten, wiped or pruned, nothing is held to give back.
            let mut discarded = Vec::new();
            for how in 0..3 {
                let brute = fighting(m, ga, a, &[(a, 800.0)]);
                threat_sync::set_temporary(m, brute, a, -600.0);
                match how {
                    0 => {
                        threat_sync::removed(m, brute, a);
                        m.creature_mut(brute).unwrap().threat.clear();
                    }
                    1 => {
                        threat_sync::wiped(m, brute);
                        m.creature_mut(brute).unwrap().threat.clear();
                    }
                    _ => {
                        // Out of its reach (immune to creatures): pruned on its turn.
                        m.unit_fields_mut(a).unwrap().set_flag(
                            UNIT_FIELD_FLAGS,
                            combat::UNIT_FLAG_IMMUNE_TO_NPC,
                            true,
                        );
                        ai::update_creatures(m);
                        m.unit_fields_mut(a).unwrap().set_flag(
                            UNIT_FIELD_FLAGS,
                            combat::UNIT_FLAG_IMMUNE_TO_NPC,
                            false,
                        );
                    }
                }
                ai::add_threat(m, brute, a, 10.0);
                threat_sync::set_temporary(m, brute, a, 0.0);
                discarded.push(threat(m, brute, a));
            }
            (clamped, restored, refresh, replaced, untracked, discarded)
        })
        .await
        .unwrap();
    assert_eq!(clamped, Some(0.0));
    assert_eq!(restored, Some(600.0));
    assert_eq!(refresh, Some(600.0));
    assert_eq!(replaced, Some(500.0));
    assert_eq!(untracked, (None, false));
    assert_eq!(discarded, vec![Some(10.0), Some(10.0), Some(10.0)]);
}

#[tokio::test]
async fn the_gm_threat_list_runs_highest_first_and_an_evade_empties_it() {
    let (_realm, mut a, ga, map) = setup(Class::Warrior).await;
    let brute = map
        .query(move |m| {
            let wolf = spawn(m, ga, BEAST, 20, 2.0);
            let helper = spawn(m, ga, HELPER, 20, 2.0);
            let brute = fighting(m, ga, ga, &[(ga, 10.0), (wolf, 30.0), (helper, 20.0)]);
            m.players
                .get_mut(&ga)
                .unwrap()
                .fields
                .set_guid(UNIT_FIELD_TARGET, brute);
            // In the same turn of the map: its own update in between would
            // drop the wolf and helper, who aren't fighting the brute.
            crate::game::map::gm::command(m, ga, "threat");
            brute
        })
        .await
        .unwrap();
    let list = a.system_message("Threat:").await;
    assert_eq!(
        list, "Threat: creature 3701 30, creature 3702 20, Alice 10",
        "{list}"
    );
    let (tracking, after) = map
        .query(move |m| {
            let tracking = threat(m, brute, ga).is_some();
            ai::enter_evade(m, brute);
            (tracking, m.creature(brute).unwrap().threat.len())
        })
        .await
        .unwrap();
    assert!(tracking);
    assert_eq!(after, 0);
}

#[tokio::test]
async fn a_creature_keeps_its_victim_until_another_passes_it_by_a_tenth_near_or_three_tenths_far() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let picks = map
        .query(move |m| {
            let a = spawn(m, ga, HELPER, 20, 1.5);
            let b = spawn(m, ga, HELPER, 20, 1.5);
            let far = spawn(m, ga, HELPER, 20, 25.0);
            let mut picks = Vec::new();
            // With no victim yet, the top.
            let brute = fighting(m, ga, Guid::zero(), &[(a, 10.0), (b, 30.0)]);
            picks.push(ai::overtake(m, brute, Guid::zero()) == b);
            // Under a tenth more, near: the victim stays.
            let brute = fighting(m, ga, a, &[(a, 100.0), (b, 109.0)]);
            picks.push(ai::overtake(m, brute, a) == a);
            // Three tenths more, far off: it goes.
            let brute = fighting(m, ga, a, &[(a, 100.0), (far, 131.0)]);
            picks.push(ai::overtake(m, brute, a) == far);
            // The victim on top stays.
            let brute = fighting(m, ga, a, &[(a, 100.0), (b, 50.0)]);
            picks.push(ai::overtake(m, brute, a) == a);
            picks
        })
        .await
        .unwrap();
    assert_eq!(picks, vec![true; 4]);
}

#[tokio::test]
async fn a_creature_turns_from_a_victim_it_can_no_longer_fight_and_with_none_left_goes_home() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (switched, list, alone, empty) = map
        .query(move |m| {
            let a = spawn(m, ga, HELPER, 20, 1.5);
            let b = spawn(m, ga, HELPER, 20, 1.5);
            let brute = fighting(m, ga, a, &[(a, 100.0), (b, 10.0)]);
            m.unit_fields_mut(a).unwrap().set_flag(
                UNIT_FIELD_FLAGS,
                combat::UNIT_FLAG_IMMUNE_TO_NPC,
                true,
            );
            ai::update_creatures(m);
            let c = m.creature(brute).unwrap();
            let switched = c.combat.victim == b;
            let list = c.threat.clone();
            // Its last one out of its reach: home it goes, forgetting it.
            let lone = fighting(m, ga, a, &[(a, 100.0)]);
            ai::update_creatures(m);
            let c = m.creature(lone).unwrap();
            let alone = (c.evading, c.threat.len());
            // And with no one on its list at all.
            m.unit_fields_mut(a).unwrap().set_flag(
                UNIT_FIELD_FLAGS,
                combat::UNIT_FLAG_IMMUNE_TO_NPC,
                false,
            );
            let empty = fighting(m, ga, a, &[]);
            m.creature_mut(empty).unwrap().combat.victim = Guid::zero();
            ai::update_creatures(m);
            (switched, list, alone, m.creature(empty).unwrap().evading)
        })
        .await
        .unwrap();
    assert!(switched);
    assert_eq!(list.len(), 1);
    assert_eq!(alone, (true, 0));
    assert!(empty);
}

/// With its list empty, a creature that another is attacking, one that
/// hasn't harmed it yet and so isn't on the list, fights that one rather
/// than going home (mangos's `Unit::SelectHostileTarget`): Keeper
/// Remulos, his phantasm dead before Eranikus's first breath reached him.
#[tokio::test]
async fn a_creature_with_none_left_on_its_list_fights_one_attacking_it() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (evading, victim, listed) = map
        .query(move |m| {
            let a = spawn(m, ga, HELPER, 20, 1.5);
            // Too far off to have struck it yet.
            let attacker = spawn(m, ga, HELPER, 20, 25.0);
            let brute = fighting(m, ga, a, &[(a, 100.0)]);
            let c = m.creature_mut(attacker).unwrap();
            c.combat.victim = brute;
            c.combat.attacking = true;
            m.note_fighting(attacker);
            combat::deal_damage(m, ga, a, 1_000_000);
            ai::update_creatures(m);
            let c = m.creature(brute).unwrap();
            (
                c.evading,
                c.combat.victim == attacker,
                threat(m, brute, attacker),
            )
        })
        .await
        .unwrap();
    assert!(!evading, "not gone home");
    assert!(victim, "the attacker its victim");
    assert!(listed.is_some(), "and on its list");
}
