//! The fight's rules of 2026-10-19 (DECISIONS #20 and #33): a player is in a
//! fight against creatures while one holds them, and five seconds after a
//! blow between players; a teleport ends it, but one a spell, Stuck or a
//! chair makes short of a hundred yards doesn't (mangos-classic's
//! `CombatManager` and `TeleportTo`).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{self as sd, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::combat::{self, Outcome};
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::{Map, MapHandle, MapKey, ai, duel};
use crate::game::tests_p3_combat::*;

/// Blink's kind: twenty yards ahead.
const LEAP: u32 = 95_300;
/// A spell not for a fight (a mount's, food's).
const CALM: u32 = 95_301;

fn in_combat(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .unwrap()
        .has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT)
}

/// The fight's look comes, `by` on.
fn later(m: &mut Map, by: Duration) {
    m.now += by;
    combat::update_players(m);
}

fn world() -> GameData {
    let mut d = data();
    d.spells.radii.insert(9, 20.0);
    let mut leap = SpellTemplate {
        id: LEAP,
        name: "Leap".into(),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    leap.effects[0] = SpellEffect {
        effect: crate::game::map::spells::EFFECT_LEAP,
        target_a: 1,
        target_b: 55,
        radius_index: 9,
        ..Default::default()
    };
    let mut calm = SpellTemplate {
        id: CALM,
        name: "Calm".into(),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    calm.attributes[0] |= sd::ATTR_NOT_IN_COMBAT;
    calm.effects[0] = SpellEffect {
        effect: 3,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.extend([(LEAP, leap), (CALM, calm)]);
    d
}

/// Alice and Bob, side by side.
async fn two() -> (
    std::sync::Arc<crate::game::Realm>,
    [Client; 2],
    Guid,
    Guid,
    MapHandle,
) {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0 + 2.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    (realm, [a, b], ga, gb, map)
}

#[tokio::test]
async fn a_blow_between_players_holds_both_five_seconds_and_no_longer() {
    let (_realm, _c, ga, gb, map) = two().await;
    let r = map
        .query(move |m| {
            combat::engage(m, ga, gb);
            let timed = [ga, gb].map(|g| {
                (
                    in_combat(m, g),
                    m.combat(g).unwrap().combat_until == Some(m.now + combat::PVP_COMBAT),
                )
            });
            later(m, Duration::from_millis(4_900));
            let within = [ga, gb].map(|g| in_combat(m, g));
            // Run out by the next look, 1.2 s on.
            later(m, combat::COMBAT_CHECK);
            let past = [ga, gb].map(|g| in_combat(m, g));
            (timed, within, past)
        })
        .await
        .unwrap();
    assert_eq!(r.0, [(true, true); 2]);
    assert_eq!(r.1, [true, true]);
    assert_eq!(r.2, [false, false]);
}

#[tokio::test]
async fn a_player_hitting_a_creature_has_no_timer_and_leaves_as_it_dies() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            forced_swing(m, ga, brute, Outcome::Hit);
            let entered = (in_combat(m, ga), m.combat(ga).unwrap().combat_until);
            // Held a minute while it fights her.
            later(m, Duration::from_secs(60));
            let held = in_combat(m, ga);
            combat::kill(m, ga, brute);
            // Out at the next look, no six seconds after the kill.
            later(m, combat::COMBAT_CHECK);
            (entered, held, in_combat(m, ga))
        })
        .await
        .unwrap();
    assert_eq!(r.0, (true, None));
    assert!(r.1);
    assert!(!r.2);
}

#[tokio::test]
async fn a_duel_won_ends_both_fights_at_once_and_one_fled_leaves_the_timer() {
    let (_realm, _c, ga, gb, map) = two().await;
    let r = map
        .query(move |m| {
            let duel_on = |m: &mut Map| {
                let flag = Guid::new(0xF110_0000_0000_0001);
                for (me, them, initiator) in [(ga, gb, true), (gb, ga, false)] {
                    m.players.get_mut(&me).unwrap().duel = Some(duel::Duel {
                        opponent: them,
                        flag,
                        phase: duel::Phase::Fighting,
                        initiator,
                        out_of_bounds_since: None,
                        started: None,
                    });
                }
            };
            duel_on(m);
            // A duel's blows run the PvP timer too.
            combat::engage(m, ga, gb);
            let fighting = (
                in_combat(m, ga),
                m.combat(gb).unwrap().combat_until.is_some(),
            );
            duel::yield_to(m, gb);
            let won = [ga, gb].map(|g| (in_combat(m, g), m.combat(g).unwrap().combat_until));
            // Fled, the fight runs out on its timer.
            duel_on(m);
            combat::engage(m, ga, gb);
            duel::left(m, gb, true);
            let fled = [ga, gb].map(|g| in_combat(m, g));
            later(m, combat::PVP_COMBAT + Duration::from_millis(100));
            (fighting, won, fled, [ga, gb].map(|g| in_combat(m, g)))
        })
        .await
        .unwrap();
    assert_eq!(r.0, (true, true));
    assert_eq!(r.1, [(false, None), (false, None)]);
    assert_eq!(r.2, [true, true]);
    assert_eq!(r.3, [false, false]);
}

#[tokio::test]
async fn bloodrage_holds_a_warrior_in_the_fight_while_it_lasts() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            combat::enter(m, ga, false);
            aura_of(m, ga, combat::SPELL_BLOODRAGE_REGEN, 4, 0, 1);
            later(m, Duration::from_secs(5));
            let held = in_combat(m, ga);
            crate::game::map::auras::remove_spell(m, ga, combat::SPELL_BLOODRAGE_REGEN, None);
            later(m, combat::COMBAT_CHECK);
            (held, in_combat(m, ga))
        })
        .await
        .unwrap();
    assert_eq!(r, (true, false));
}

#[tokio::test]
async fn a_harmful_spell_between_players_and_help_for_one_in_a_fight_bring_them_in() {
    let (_realm, _c, ga, gb, map) = two().await;
    let r = map
        .query(move |m| {
            // Alice's strike lands on Bob: both in it, on the PvP timer.
            combat::struck(m, ga, gb);
            let struck = [ga, gb].map(|g| m.combat(g).unwrap().combat_until.is_some());
            combat::leave_combat(m, ga);
            combat::leave_combat(m, gb);
            // Bob heals Alice, fighting a creature: Bob is in it too, with no
            // timer, and held by the threat his heal made.
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            forced_swing(m, brute, ga, Outcome::Miss);
            combat::assisted(m, gb, ga);
            let helped = (in_combat(m, gb), m.combat(gb).unwrap().combat_until);
            m.creature_mut(brute).unwrap().threat.push((gb, 5.0));
            later(m, combat::COMBAT_CHECK);
            let held = in_combat(m, gb);
            // Help for one out of a fight brings no one in.
            combat::leave_combat(m, ga);
            ai::forget(m, gb);
            combat::leave_combat(m, gb);
            combat::assisted(m, gb, ga);
            (struck, helped, held, in_combat(m, gb))
        })
        .await
        .unwrap();
    assert_eq!(r.0, [true, true]);
    assert_eq!(r.1, (true, None));
    assert!(r.2);
    assert!(!r.3);
}

#[tokio::test]
async fn in_a_fight_a_player_cant_log_out_or_cast_what_isnt_for_one_and_can_after() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let brute = map.query(move |m| fighting(m, ga)).await.unwrap();
    let calm = move |m: &mut Map| {
        let s = m.realm.data.spells.get(CALM).unwrap().clone();
        let mods = crate::game::map::spells::mods::for_spell(m, ga, &s);
        let t = crate::game::map::spells::Targets {
            unit: Some(ga),
            ..Default::default()
        };
        crate::game::map::spells::checks::check(m, ga, &s, &t, None, false, &mods)
    };
    let refused = map.query(calm).await.unwrap();
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    let logout = a
        .wait_for("SMSG_LOGOUT_RESPONSE", |m| match m {
            ServerOpcodeMessage::SMSG_LOGOUT_RESPONSE(r) => Some(r.result),
            _ => None,
        })
        .await;
    assert_eq!(
        refused,
        Some(crate::game::map::spells::checks::fail::AFFECTING_COMBAT)
    );
    assert_eq!(logout, LogoutResult::FailureInCombat);
    // The creature dead, she is out at the next look, and free to.
    let after = map
        .query(move |m| {
            combat::kill(m, ga, brute);
            later(m, combat::COMBAT_CHECK);
            (in_combat(m, ga), calm(m))
        })
        .await
        .unwrap();
    assert_eq!(after, (false, None));
}

/// A creature holds Alice in a fight, striking for a point.
fn fighting(m: &mut Map, ga: Guid) -> Guid {
    let brute = spawn(m, ga, BRUTE, 20, 1.0);
    let f = &mut m.creature_mut(brute).unwrap().fields;
    f.set_f32(UNIT_FIELD_MINDAMAGE, 1.0);
    f.set_f32(UNIT_FIELD_MAXDAMAGE, 1.0);
    ai::attacked(m, brute, ga);
    forced_swing(m, brute, ga, Outcome::Miss);
    brute
}

fn ahead(m: &Map, g: Guid, yards: f32) -> Vector3d {
    let p = m.players[&g].position();
    Vector3d {
        x: p.x + yards,
        y: p.y,
        z: p.z,
    }
}

#[tokio::test]
async fn a_teleport_within_the_map_ends_the_players_fight_but_not_the_creatures() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = fighting(m, ga);
            // A GM's, an area trigger's, a script's: any distance.
            let to = ahead(m, ga, 10.0);
            let key = m.key;
            m.teleport(ga, key, to, 0.0);
            let left = in_combat(m, ga);
            // The creature keeps her on its list, and attacking: that
            // holds no one out of a fight, nor puts her back in it.
            let creature = (
                m.creature(brute).unwrap().combat.attacking,
                m.creature(brute)
                    .unwrap()
                    .threat
                    .iter()
                    .any(|(t, _)| *t == ga),
            );
            later(m, combat::COMBAT_CHECK);
            let after = in_combat(m, ga);
            // Its next blow does.
            combat::engage(m, brute, ga);
            (left, creature, after, in_combat(m, ga))
        })
        .await
        .unwrap();
    assert_eq!(r, (false, (true, true), false, true));
}

#[tokio::test]
async fn a_short_spell_teleport_keeps_the_fight_and_a_long_one_ends_it() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            fighting(m, ga);
            // Blink's twenty yards (mangos's `NearTeleportTo`).
            crate::game::map::spells::cast_triggered(m, ga, LEAP, None);
            let blinked = in_combat(m, ga);
            // A teleport to its place on this map, short and long.
            let near = ahead(m, ga, 99.0);
            m.near_teleport(ga, near, 0.0);
            let short = in_combat(m, ga);
            let far = ahead(m, ga, 101.0);
            m.near_teleport(ga, far, 0.0);
            let long = in_combat(m, ga);
            // One that says it keeps the fight does, any distance.
            fighting(m, ga);
            let farther = ahead(m, ga, 500.0);
            let key = m.key;
            m.teleport_with(ga, key, farther, 0.0, true);
            (blinked, short, long, in_combat(m, ga))
        })
        .await
        .unwrap();
    assert_eq!(r, (true, true, false, true));
}

#[tokio::test]
async fn a_teleport_to_another_map_ends_the_fight_though_it_says_keep_it() {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    map.call(move |m| {
        fighting(m, ga);
        m.teleport_with(ga, MapKey::world(1), Vector3d::default(), 0.0, true);
    });
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    let there = realm.maps.get(&realm, MapKey::world(1));
    let fighting = there
        .query(move |m| m.players.get(&ga).map(|_| in_combat(m, ga)))
        .await
        .unwrap();
    assert_eq!(fighting, Some(false));
}

/// A creature no one sees, and fighting no one, still holds whom its threat
/// list holds. The awake set (perf2) didn't count a threat list, so the
/// fight's look missed such a creature and let the player go; the debug
/// check caught it in the chaos test.
#[tokio::test]
async fn a_creature_unseen_and_idle_still_holds_whom_its_threat_list_holds() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let held = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            forced_swing(m, ga, brute, Outcome::Hit);
            if let Some(c) = m.creature_mut(brute) {
                c.combat.attacking = false;
                c.combat.attackers.clear();
                c.observers.clear();
            }
            // A fresh update, whose first look prunes the awake set.
            m.stats.updates += 1;
            later(m, combat::COMBAT_CHECK);
            in_combat(m, ga)
        })
        .await;
    assert_eq!(held, Some(true), "held by the brute's threat list");
}
