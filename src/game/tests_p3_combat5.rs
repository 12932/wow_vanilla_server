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

/// The next update looks at `g`'s fight for its end.
fn look_now(m: &mut Map, g: Guid) {
    let now = m.now;
    m.combat_mut(g).unwrap().next_check = Some(now);
}

/// Review 5's combat finding 1, of a passive creature: one no player saw
/// had no turn, and one with a turn held whoever struck it while they
/// lived, however far they went. In a fight, it has its turn though no one
/// sees it, and one gone past its threat radius, with no blow for the
/// leash's time, comes off its list.
#[tokio::test]
async fn a_passive_creature_lets_go_of_whoever_struck_it_and_went() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let dummy = spawn(m, ga, BRUTE, 20, 1.0);
            m.creature_mut(dummy)
                .unwrap()
                .scripted
                .get_or_insert_default()
                .passive = true;
            forced_swing(m, ga, dummy, Outcome::Hit);
            let struck = (
                in_combat(m, ga),
                m.creature(dummy).unwrap().threat.len(),
                m.combat(dummy).unwrap().attacking,
            );
            // Gone 200 yards, long since the blow, and seen by no one.
            m.players.get_mut(&ga).unwrap().movement.position.x += 200.0;
            let c = m.creature_mut(dummy).unwrap();
            c.observers.clear();
            c.engagement.last_hostile = None;
            ai::update_creatures(m);
            look_now(m, ga);
            combat::update_players(m);
            (
                struck,
                m.creature(dummy).unwrap().threat.len(),
                in_combat(m, ga),
            )
        })
        .await
        .unwrap();
    assert_eq!(r.0, (true, 1, false), "struck, held, not struck back");
    assert_eq!((r.1, r.2), (0, false), "let go");
}

#[tokio::test]
async fn an_attack_that_lands_nowhere_holds_no_one_and_ends_with_the_fight() {
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
            {
                let c = m.combat_mut(ga).unwrap();
                c.victim = far;
                c.attacking = true;
                c.attackers.insert(far);
            }
            // Not yet looked at: still in it.
            let now = m.now;
            m.combat_mut(ga).unwrap().next_check = Some(now + Duration::from_millis(500));
            combat::update_players(m);
            let before = in_combat(m, ga);
            // Looked at, nothing holds them: out, the attack stopped with
            // it (mangos's `HandleExitCombat`), and looked at no more.
            look_now(m, ga);
            combat::update_players(m);
            let c = m.combat(ga).unwrap();
            let after = (
                in_combat(m, ga),
                c.attacking,
                c.attackers.is_empty(),
                c.next_check,
            );
            (before, after)
        })
        .await
        .unwrap();
    assert!(r.0);
    assert_eq!(r.1, (false, false, true, None));
}

#[tokio::test]
async fn a_dead_victim_ends_the_fight_at_the_next_look_with_no_window() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            forced_swing(m, ga, brute, Outcome::Miss);
            // Begun, it is looked at the next update, then every 1.2 s.
            let first = m.combat(ga).unwrap().next_check == Some(m.now);
            combat::update_players(m);
            let held = (
                in_combat(m, ga),
                m.combat(ga).unwrap().next_check == Some(m.now + combat::COMBAT_CHECK),
            );
            combat::kill(m, ga, brute);
            let now = m.now;
            m.combat_mut(ga).unwrap().next_swing = Some(now);
            combat::try_swing(m, ga);
            // Dead, it holds no one; she is out at the next look.
            combat::update_players(m);
            let waiting = (m.combat(ga).unwrap().attacking, in_combat(m, ga));
            look_now(m, ga);
            combat::update_players(m);
            (first, held, waiting, in_combat(m, ga))
        })
        .await
        .unwrap();
    assert!(r.0, "looked at the update after it began");
    assert_eq!(r.1, (true, true));
    assert_eq!(r.2, (false, true));
    assert!(!r.3);
}

#[tokio::test]
async fn a_live_creature_with_the_player_on_its_list_holds_them_and_a_dead_or_gone_one_doesnt() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            forced_swing(m, ga, brute, Outcome::Miss);
            let c = m.creature_mut(brute).unwrap();
            c.combat.attacking = true;
            c.combat.victim = ga;
            c.threat = vec![(ga, 10.0)];
            m.combat_mut(ga).unwrap().attacking = false;
            look_now(m, ga);
            combat::update_players(m);
            let held = in_combat(m, ga);
            // Attacking her, though she is off its list: still held.
            m.creature_mut(brute).unwrap().threat.clear();
            look_now(m, ga);
            combat::update_players(m);
            let attacked = in_combat(m, ga);
            combat::kill(m, ga, brute);
            look_now(m, ga);
            combat::update_players(m);
            let killed = in_combat(m, ga);
            // Another on her list, gone from the world.
            let other = spawn(m, ga, BRUTE, 20, 1.0);
            forced_swing(m, ga, other, Outcome::Miss);
            m.remove_creature(other);
            look_now(m, ga);
            combat::update_players(m);
            (held, attacked, killed, in_combat(m, ga))
        })
        .await
        .unwrap();
    assert_eq!(r, (true, true, false, false));
}

#[tokio::test]
async fn a_creature_that_evades_lets_the_player_go_at_the_next_look() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            // Struck by it: in the fight with no timer of her own.
            forced_swing(m, brute, ga, Outcome::Miss);
            let struck = (in_combat(m, ga), m.combat(ga).unwrap().combat_until);
            look_now(m, ga);
            combat::update_players(m);
            let held = in_combat(m, ga);
            ai::enter_evade(m, brute);
            look_now(m, ga);
            combat::update_players(m);
            (struck, held, in_combat(m, ga))
        })
        .await
        .unwrap();
    assert_eq!(r.0, (true, None), "a creature's blow sets no timer");
    assert!(r.1, "held while it fights her");
    assert!(!r.2, "out once it evades");
}

#[tokio::test]
async fn a_creature_taking_the_player_on_puts_them_in_the_fight_before_any_blow() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 10.0);
            let before = in_combat(m, ga);
            // It aggroes on her (mangos's `EngageInCombatWith`).
            ai::attacked(m, brute, ga);
            (before, in_combat(m, ga))
        })
        .await
        .unwrap();
    assert_eq!(r, (false, true));
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

fn view(x: f32, y: f32, orientation: f32) -> combat::UnitView {
    combat::UnitView {
        guid: Guid::zero(),
        position: Vector3d { x, y, z: 0.0 },
        orientation,
        level: 1,
        faction: 0,
        combat_reach: 1.5,
        alive: true,
        is_player: true,
        standing: true,
        running: false,
    }
}

#[test]
fn a_swing_wants_its_victim_within_a_hundred_and_twenty_degrees_ahead() {
    let me = view(0.0, 0.0, 0.0);
    for (x, y, ahead) in [
        (3.0, 0.0, true),
        (2.0, 3.4, true),
        (2.0, 3.5, false),
        (0.0, 3.0, false),
        (-3.0, 0.0, false),
    ] {
        assert_eq!(combat::facing(&me, &view(x, y, 0.0)), ahead, "{x} {y}");
    }
    // The angle wraps.
    let turned = view(0.0, 0.0, std::f32::consts::TAU - 0.1);
    assert!(combat::facing(&turned, &view(3.0, 0.0, 0.0)));
}

#[tokio::test]
async fn facing_away_a_player_is_told_so_and_both_hands_and_the_strike_wait() {
    let (_realm, mut a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            m.players.get_mut(&ga).unwrap().spells.push(STRIKE);
            combat::queue_next_swing(m, ga, STRIKE);
            // Turned about.
            let p = m.players.get_mut(&ga).unwrap();
            p.movement.orientation += std::f32::consts::PI;
            let now = m.now;
            let before = m.unit_fields(brute).unwrap().get_u32(UNIT_FIELD_HEALTH);
            {
                let c = m.combat_mut(ga).unwrap();
                c.victim = brute;
                c.attacking = true;
                c.next_swing = Some(now);
                c.next_offhand = Some(now);
            }
            combat::try_swing(m, ga);
            let c = m.combat(ga).unwrap();
            (
                c.next_swing == Some(now),
                c.queued,
                m.unit_fields(brute).unwrap().get_u32(UNIT_FIELD_HEALTH) == before,
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (true, Some(STRIKE), true));
    a.wait_for("SMSG_ATTACKSWING_BADFACING", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_ATTACKSWING_BADFACING).then_some(())
    })
    .await;
}

#[tokio::test]
async fn an_off_hand_swing_puts_an_imminent_main_hand_a_moment_after() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            m.players
                .get_mut(&ga)
                .unwrap()
                .spells
                .push(crate::game::map::derive::SPELL_DUAL_WIELD);
            equip(m, ga, SWORD, 15);
            equip(m, ga, SWORD, 16);
            let now = m.now;
            {
                let c = m.combat_mut(ga).unwrap();
                c.victim = brute;
                c.attacking = true;
                c.next_swing = Some(now + Duration::from_millis(50));
                c.next_offhand = Some(now);
            }
            combat::FORCED_OUTCOME.with(|f| f.set(Some(Outcome::Miss)));
            combat::try_swing(m, ga);
            combat::FORCED_OUTCOME.with(|f| f.set(None));
            let c = m.combat(ga).unwrap();
            (
                c.next_swing.map(|t| t.duration_since(now).as_millis()),
                c.next_offhand.map(|t| t.duration_since(now).as_millis()),
            )
        })
        .await
        .unwrap();
    assert_eq!(r.0, Some(200));
    assert!(r.1.is_some_and(|t| t >= 1_000), "{r:?}");
}

#[tokio::test]
async fn stuns_fears_confusion_pacify_and_feigning_hold_the_swings_and_roots_and_silence_dont() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            use crate::game::map::auras::*;
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            let mut out = Vec::new();
            let kinds: [&[u32]; 9] = [
                &[AURA_MOD_STUN],
                &[AURA_MOD_CONFUSE],
                &[AURA_MOD_FEAR],
                &[AURA_MOD_PACIFY],
                &[AURA_MOD_PACIFY_SILENCE],
                &[AURA_FEIGN_DEATH],
                &[AURA_MOD_ROOT],
                &[AURA_MOD_SILENCE],
                &[AURA_MOD_FEAR, AURA_PREVENTS_FLEEING],
            ];
            for (i, kinds) in kinds.iter().enumerate() {
                for (j, kind) in kinds.iter().enumerate() {
                    aura_of(m, ga, 90_500 + 10 * i as u32 + j as u32, *kind, 0, 1);
                }
                let before = m.unit_fields(brute).unwrap().get_u32(UNIT_FIELD_HEALTH);
                forced_swing(m, ga, brute, Outcome::Hit);
                out.push(m.unit_fields(brute).unwrap().get_u32(UNIT_FIELD_HEALTH) < before);
                for j in 0..kinds.len() {
                    remove_spell(m, ga, 90_500 + 10 * i as u32 + j as u32, None);
                }
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(
        r,
        vec![false, false, false, false, false, false, true, true, true]
    );
}

#[tokio::test]
async fn no_one_swings_without_meaning_to_nor_at_or_from_the_dead() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            let health = |m: &Map| m.unit_fields(brute).unwrap().get_u32(UNIT_FIELD_HEALTH);
            // In a fight, but not attacking: no swing.
            forced_swing(m, ga, brute, Outcome::Miss);
            m.combat_mut(ga).unwrap().attacking = false;
            let now = m.now;
            m.combat_mut(ga).unwrap().next_swing = Some(now);
            let before = health(m);
            combat::FORCED_OUTCOME.with(|f| f.set(Some(Outcome::Hit)));
            combat::try_swing(m, ga);
            let idle = health(m) == before;
            // Dead, the attacker stops.
            combat::kill(m, brute, ga);
            {
                let c = m.combat_mut(ga).unwrap();
                c.attacking = true;
                c.next_swing = Some(now);
            }
            combat::try_swing(m, ga);
            let dead = (health(m) == before, m.combat(ga).unwrap().attacking);
            combat::FORCED_OUTCOME.with(|f| f.set(None));
            (idle, dead)
        })
        .await
        .unwrap();
    assert_eq!(r, (true, (true, false)));
}
