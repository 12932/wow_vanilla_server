//! Phase 3's combat tests, part five (Thistle Tea's
//! `player_combat_test.exs`): a player's attack started, switched and
//! stopped, and how long a fight holds them.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::combat::{self, AutoRepeat, Outcome};
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::spells;
use crate::game::map::{Map, ai};
use crate::game::tests_p3_combat::*;

fn in_combat(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .unwrap()
        .has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT)
}

/// The attack stop `attacker` sent for `enemy`.
async fn attack_stop(c: &mut crate::game::harness::Client, attacker: Guid, enemy: Guid) {
    c.wait_for("SMSG_ATTACKSTOP", |m| match m {
        ServerOpcodeMessage::SMSG_ATTACKSTOP(s) if s.player == attacker && s.enemy == enemy => {
            Some(())
        }
        _ => None,
    })
    .await;
}

#[tokio::test]
async fn a_swing_puts_a_player_in_the_fight_and_stopping_keeps_the_selection_and_other_casts() {
    let (_realm, mut a, ga, map) = setup(Class::Warrior).await;
    let (brute, entered, idle_stop, stopped) = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            m.players
                .get_mut(&ga)
                .unwrap()
                .fields
                .set_guid(UNIT_FIELD_TARGET, brute);
            // Stopping with no attack running leaves the selection.
            combat::stop_attack(m, ga);
            let idle_stop = m.unit_fields(ga).unwrap().get_guid(UNIT_FIELD_TARGET) == brute;
            forced_swing(m, ga, brute, Outcome::Miss);
            let entered = (
                in_combat(m, ga),
                m.combat(ga).unwrap().last_action == Some(m.now),
            );
            // A strike queued, a shot repeating: stopping the swings ends
            // the strike, not the shot nor the fight.
            m.players.get_mut(&ga).unwrap().spells.push(STRIKE);
            combat::queue_next_swing(m, ga, STRIKE);
            let now = m.now;
            m.combat_mut(ga).unwrap().auto_repeat = Some(AutoRepeat {
                spell: 75,
                target: brute,
                next_at: now + Duration::from_secs(4),
            });
            combat::stop_attack(m, ga);
            let c = m.combat(ga).unwrap();
            let stopped = (
                c.attacking,
                c.queued,
                c.auto_repeat.is_some(),
                in_combat(m, ga),
            );
            (brute, entered, idle_stop, stopped)
        })
        .await
        .unwrap();
    assert!(idle_stop);
    assert_eq!(entered, (true, true));
    assert_eq!(stopped, (false, None, true, true));
    attack_stop(&mut a, ga, brute).await;
    a.wait_frame("the strike interrupted", spells::SMSG_CAST_RESULT, |b| {
        b[..4] == STRIKE.to_le_bytes() && b[4] == 2 && b[5] == spells::checks::fail::INTERRUPTED
    })
    .await;
}

#[tokio::test]
async fn switching_victims_stops_the_old_attack_and_its_strike_but_keeps_the_swing_timers() {
    let (_realm, mut a, ga, map) = setup(Class::Warrior).await;
    let (one, kept, first) = map
        .query(move |m| {
            let one = spawn(m, ga, BRUTE, 20, 1.0);
            let two = spawn(m, ga, BRUTE, 20, 1.0);
            // A first attack keeps a strike already queued, and attacks
            // owed.
            m.players.get_mut(&ga).unwrap().spells.push(STRIKE);
            combat::queue_next_swing(m, ga, STRIKE);
            m.combat_mut(ga).unwrap().extra_attacks = 0;
            let now = m.now;
            {
                let c = m.combat_mut(ga).unwrap();
                c.next_swing = Some(now + Duration::from_secs(5));
                c.next_offhand = Some(now + Duration::from_secs(6));
                c.held_extra_attacks = 2;
            }
            combat::start_attack(m, ga, one);
            let first = (
                m.combat(ga).unwrap().queued,
                m.combat(ga).unwrap().held_extra_attacks,
            );
            combat::start_attack(m, ga, two);
            let c = m.combat(ga).unwrap();
            let kept = (
                c.victim == two,
                c.attacking,
                c.queued,
                c.next_swing == Some(now + Duration::from_secs(5)),
                c.next_offhand == Some(now + Duration::from_secs(6)),
            );
            (one, kept, first)
        })
        .await
        .unwrap();
    assert_eq!(first, (Some(STRIKE), 2));
    assert_eq!(kept, (true, true, None, true, true));
    attack_stop(&mut a, ga, one).await;
    a.wait_frame("the strike interrupted", spells::SMSG_CAST_RESULT, |b| {
        b[..4] == STRIKE.to_le_bytes() && b[4] == 2
    })
    .await;
}

#[tokio::test]
async fn a_fight_holds_a_player_for_its_window_and_not_for_an_attack_that_lands_nowhere() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            // Struck at, then twenty yards off, out of reach, and not
            // fighting them.
            let far = spawn(m, ga, BRUTE, 20, 1.0);
            forced_swing(m, ga, far, Outcome::Miss);
            let c = m.creature_mut(far).unwrap();
            c.position.x += 20.0;
            c.combat = Default::default();
            c.threat.clear();
            let now = m.now;
            let ago = |s: u64| now - Duration::from_millis(s);
            {
                let c = m.combat_mut(ga).unwrap();
                c.victim = far;
                c.attacking = true;
                c.last_action = Some(ago(2_000));
                c.attackers.insert(far);
            }
            combat::update_players(m);
            let within = in_combat(m, ga);
            // Past the window, swinging at it holds them no longer, though
            // the attack stays on.
            m.combat_mut(ga).unwrap().last_action = Some(ago(7_000));
            combat::update_players(m);
            let lapsed = (
                in_combat(m, ga),
                m.combat(ga).unwrap().attacking,
                m.combat(ga).unwrap().attackers.is_empty(),
            );
            (within, lapsed)
        })
        .await
        .unwrap();
    assert!(r.0);
    assert_eq!(r.1, (false, true, true));
}

#[tokio::test]
async fn a_dead_victim_stops_the_swings_and_the_fight_lingers_for_its_window() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            forced_swing(m, ga, brute, Outcome::Miss);
            combat::kill(m, ga, brute);
            let now = m.now;
            m.combat_mut(ga).unwrap().next_swing = Some(now);
            combat::try_swing(m, ga);
            combat::update_players(m);
            let lingering = (m.combat(ga).unwrap().attacking, in_combat(m, ga));
            m.combat_mut(ga).unwrap().last_action = Some(now - Duration::from_millis(7_000));
            combat::update_players(m);
            (lingering, in_combat(m, ga))
        })
        .await
        .unwrap();
    assert_eq!(r.0, (false, true));
    assert!(!r.1);
}

#[tokio::test]
async fn a_live_creature_with_the_player_on_its_list_holds_them_and_a_dead_or_gone_one_doesnt() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            forced_swing(m, ga, brute, Outcome::Miss);
            let now = m.now;
            let c = m.creature_mut(brute).unwrap();
            c.combat.attacking = true;
            c.combat.victim = ga;
            c.threat = vec![(ga, 10.0)];
            m.combat_mut(ga).unwrap().attacking = false;
            m.combat_mut(ga).unwrap().last_action = Some(now - Duration::from_secs(60));
            combat::update_players(m);
            let held = in_combat(m, ga);
            combat::kill(m, ga, brute);
            m.combat_mut(ga).unwrap().last_action = Some(now - Duration::from_secs(60));
            combat::update_players(m);
            (held, in_combat(m, ga))
        })
        .await
        .unwrap();
    assert_eq!(r, (true, false));
}

#[tokio::test]
async fn vanish_and_disengaging_clear_a_players_fight_whole() {
    let (_realm, _a, ga, map) = setup(Class::Rogue).await;
    let r = map
        .query(move |m| {
            let one = spawn(m, ga, BRUTE, 20, 1.0);
            let two = spawn(m, ga, BRUTE, 20, 1.0);
            forced_swing(m, ga, one, Outcome::Miss);
            for b in [one, two] {
                let c = m.creature_mut(b).unwrap();
                c.combat.attacking = true;
                c.combat.victim = ga;
                c.threat = vec![(ga, 10.0)];
            }
            let now = m.now;
            m.combat_mut(ga).unwrap().auto_repeat = Some(AutoRepeat {
                spell: 75,
                target: one,
                next_at: now + Duration::from_secs(4),
            });
            combat::leave_combat(m, ga);
            ai::forget(m, ga);
            let c = m.combat(ga).unwrap();
            (
                in_combat(m, ga),
                c.last_action,
                c.attacking,
                c.auto_repeat.is_none(),
                c.attackers.is_empty(),
                [one, two].map(|b| m.creature(b).unwrap().threat.is_empty()),
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (false, None, false, true, true, [true, true]));
}
