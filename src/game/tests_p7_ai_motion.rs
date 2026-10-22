//! Round 7's creature AI tests of a fight's footwork, ported from Thistle
//! Tea's `test/game/core/ai/bt/mob_test.exs`, `aquatic_movement_test.exs`
//! and `acquisition_test.exs` (docs/parity/matrix.csv): stepping about a
//! victim, stopping at contact, a wander a fight cuts short, the habitat
//! a creature takes victims in, and the clock of a victim out of reach.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::map::combat::{self, dist};
use crate::game::map::creature::Motion;
use crate::game::map::{Map, MapHandle, ai};
use crate::game::tests_p3_combat::{BRUTE, START, data, setup_with, spawn};

/// The map's creatures take `n` turns of 50 ms.
fn turns(m: &mut Map, n: u32) {
    for _ in 0..n {
        m.now += Duration::from_millis(50);
        ai::update_creatures(m);
        combat::update_creatures(m);
    }
}

fn at(east: f32, north: f32) -> Vector3d {
    Vector3d {
        x: START.0 + east,
        y: START.1 + north,
        z: START.2,
    }
}

/// Creature `g` put at `p`, its grid cell with it.
fn put(m: &mut Map, g: Guid, p: Vector3d) {
    let c = m.creature_mut(g).unwrap();
    let old = c.cell;
    c.position = p;
    c.cell = crate::game::map::grid::cell_of(p.x, p.y);
    let new = c.cell;
    m.grid.relocate(g, old, new);
}

/// The player at `p`, standing, with health to spare.
fn stand(m: &mut Map, g: Guid, p: Vector3d) {
    let pl = m.players.get_mut(&g).unwrap();
    pl.movement.position = p;
    pl.movement.orientation = 0.0;
    pl.movement.flags = MovementInfo_MovementFlags::empty();
    pl.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
    pl.fields.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
}

/// A world of `d` with Alice in it, her first heartbeat in.
async fn world(
    d: GameData,
) -> (
    std::sync::Arc<crate::game::Realm>,
    crate::game::harness::Client,
    Guid,
    MapHandle,
) {
    let (realm, a, ga, map) = setup_with(d, Class::Warrior).await;
    let deadline = tokio::time::Instant::now() + crate::game::harness::WAIT;
    while map
        .query(move |m| dist(m.players[&ga].position(), at(0.0, 0.0)) > 0.01)
        .await
        .unwrap()
    {
        assert!(tokio::time::Instant::now() < deadline, "her heartbeat");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    (realm, a, ga, map)
}

fn moving(m: &Map, g: Guid) -> bool {
    matches!(m.creature(g).unwrap().motion, Motion::Moving(_))
}

/// Whether `t` lies within `lo..=hi` milliseconds after `now`.
fn within(t: Option<Instant>, now: Instant, lo: u64, hi: u64) -> bool {
    t.is_some_and(|t| t >= now + Duration::from_millis(lo) && t <= now + Duration::from_millis(hi))
}

/// Thistle Tea's `maybe_spread` "does nothing without a stacked
/// neighbor", "waits for the spread timer", "skips while the mob is still
/// moving", "stops nudging after the attempt cap" and "resets the spread
/// budget when the target is moving": a creature fighting in reach looks
/// every 2.5 to 3.5 seconds to step aside from one of its kind standing on
/// it, and not before; with none there it stays put, its next look set; on
/// its way it doesn't look; three steps aside and it steps no more until
/// its victim moves, which starts its count again.
#[tokio::test]
async fn a_creature_steps_aside_from_its_kind_on_its_own_clock_three_times_till_its_victim_moves() {
    let (_realm, _a, ga, map) = world(data()).await;
    let out = map
        .query(move |m| {
            stand(m, ga, at(0.0, 0.0));
            let k = spawn(m, ga, BRUTE, 20, 4.0);
            ai::attacked(m, k, ga);
            let e = |m: &Map| {
                let c = m.creature(k).unwrap();
                (c.engagement.spread_at, c.engagement.spread_tries)
            };
            // No one on it: it stays, and looks again in 2.5 to 3.5 s.
            turns(m, 1);
            let alone = (moving(m, k), e(m).1, within(e(m).0, m.now, 2_500, 3_500));
            // One of its kind on it: not before its clock says.
            let other = spawn(m, ga, BRUTE, 20, 4.0);
            put(m, other, at(4.0, 0.3));
            turns(m, 1);
            let early = (moving(m, k), e(m).1);
            // Its time: it steps aside, at a walk.
            let now = m.now;
            m.creature_mut(k).unwrap().engagement.spread_at = Some(now);
            turns(m, 1);
            let stepped = (
                matches!(&m.creature(k).unwrap().motion, Motion::Moving(w) if !w.run),
                e(m).1,
                m.creature(k).unwrap().engagement.spreading,
            );
            // On its way, it doesn't look.
            let now = m.now;
            m.creature_mut(k).unwrap().engagement.spread_at = Some(now);
            turns(m, 1);
            let on_its_way = (e(m).0 == Some(now), e(m).1);
            // Three times, and no more.
            turns(m, 60);
            put(m, k, at(4.0, 0.0));
            put(m, other, at(4.0, 0.3));
            let now = m.now;
            let c = m.creature_mut(k).unwrap();
            c.engagement.spread_tries = 3;
            c.engagement.spread_at = Some(now);
            turns(m, 1);
            let capped = (moving(m, k), e(m).1);
            // Its victim on the move: the count starts again.
            m.players.get_mut(&ga).unwrap().movement.flags =
                MovementInfo_MovementFlags::new_forward();
            let now = m.now;
            let c = m.creature_mut(k).unwrap();
            c.engagement.spread_tries = 2;
            c.engagement.spread_at = Some(now);
            turns(m, 1);
            let reset = (moving(m, k), e(m).1, within(e(m).0, m.now, 2_500, 3_500));
            (alone, early, stepped, on_its_way, capped, reset)
        })
        .await
        .unwrap();
    let (alone, early, stepped, on_its_way, capped, reset) = out;
    assert_eq!(alone, (false, 0, true), "alone, it stays; its clock set");
    assert_eq!(early, (false, 0), "not before its clock");
    assert_eq!(stepped, (true, 1, true), "a step aside, at a walk");
    assert_eq!(on_its_way, (true, 1), "on its way it doesn't look");
    assert_eq!(capped, (false, 3), "three, and no more");
    assert_eq!(reset, (false, 0, true), "its victim moving: counted afresh");
}

/// mangos-classic's chase, its run cut short once its victim has come
/// nearer it than where it is headed; Thistle Tea's `halt_at_contact`
/// "does not interrupt a spread move" and "clears the spreading flag once
/// stationary"; `interrupt_idle_movement` "halts a wander spline when
/// combat starts within melee range" and "leaves combat movement
/// untouched", and the tree "interrupts a wander spline when combat
/// begins": a chaser in reach of a victim who stands runs on to where it
/// aimed; one she comes nearer than that stops, turned to her, its
/// watchers told; a step about its victim goes on to its end, and once it
/// stands it is a step no more. A wander the fight finds in reach stops
/// at once.
#[tokio::test]
async fn a_chaser_runs_on_to_contact_a_step_about_goes_on_and_a_wander_stops_as_the_fight_begins() {
    let (realm, _a, ga, map) = world(data()).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let k = map
        .query(move |m| {
            stand(m, ga, at(0.0, 0.0));
            spawn(m, ga, BRUTE, 20, 12.0)
        })
        .await
        .unwrap();
    b.sees(k).await;
    b.clear();
    let out = map
        .query(move |m| {
            m.players.get_mut(&gb).unwrap().movement.position = at(0.0, 30.0);
            let c = m.creature_mut(k).unwrap();
            c.speeds.run = 2.0;
            ai::attacked(m, k, ga);
            turns(m, 1);
            let chasing = moving(m, k) && m.creature(k).unwrap().chase_target_at.is_some();
            // In her reach, she standing, it runs on to where it aimed.
            let her = m.position_of(ga).unwrap();
            let mut runs_on = false;
            for _ in 0..200 {
                turns(m, 1);
                let (me, them) = (m.unit_view(k).unwrap(), m.unit_view(ga).unwrap());
                if crate::game::map::combat::in_melee_range(&me, &them) {
                    runs_on = moving(m, k) && dist(me.position, her) > 3.0;
                    break;
                }
            }
            // She comes nearer it than where it is headed: it stops,
            // turned to her.
            let here = m.position_of(k).unwrap();
            stand(
                m,
                ga,
                Vector3d {
                    x: here.x - 1.0,
                    ..here
                },
            );
            turns(m, 1);
            let c = m.creature(k).unwrap();
            let to_her = {
                let p = m.position_of(ga).unwrap();
                (p.y - c.position.y).atan2(p.x - c.position.x)
            };
            let halted = (
                moving(m, k),
                (c.orientation - to_her).sin().abs() < 0.01 && (c.orientation - to_her).cos() > 0.0,
            );
            // A step about her, touching her, goes on.
            let here = m.position_of(k).unwrap();
            let now = m.now;
            let spline = m.next_spline_id;
            m.next_spline_id += 1;
            let c = m.creature_mut(k).unwrap();
            c.walk_facing(
                vec![
                    here,
                    Vector3d {
                        y: here.y + 1.0,
                        ..here
                    },
                ],
                false,
                false,
                ga,
                now,
                spline,
            );
            c.engagement.spreading = true;
            turns(m, 1);
            let step_goes_on = moving(m, k);
            turns(m, 40);
            let stood = (moving(m, k), m.creature(k).unwrap().engagement.spreading);
            (chasing, runs_on, halted, step_goes_on, stood)
        })
        .await
        .unwrap();
    let (chasing, runs_on, halted, step_goes_on, stood) = out;
    assert!(chasing, "it chases");
    assert!(runs_on, "in reach, short of her, it runs on");
    assert_eq!(
        halted,
        (false, true),
        "touching her, it stops, turned to her"
    );
    assert!(step_goes_on, "a step about her goes on");
    assert_eq!(stood, (false, false), "standing, a step no more");
    // Its watchers saw it stop and turn.
    let stop = b
        .gets(Duration::from_millis(500), |msg| match msg {
            ServerOpcodeMessage::SMSG_MONSTER_MOVE(x)
                if x.guid == k
                    && matches!(x.move_type, SMSG_MONSTER_MOVE_MonsterMoveType::Stop) =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    assert!(stop.is_some(), "a stop");
    // A wander, in reach as the fight begins: it stops.
    let wandered = map
        .query(move |m| {
            stand(m, ga, at(0.0, 0.0));
            let w = spawn(m, ga, BRUTE, 20, 4.0);
            let now = m.now;
            let spline = m.next_spline_id;
            m.next_spline_id += 1;
            let here = m.position_of(w).unwrap();
            m.creature_mut(w).unwrap().walk(
                vec![
                    here,
                    Vector3d {
                        y: here.y + 10.0,
                        ..here
                    },
                ],
                false,
                now,
                spline,
            );
            ai::attacked(m, w, ga);
            turns(m, 1);
            moving(m, w)
        })
        .await
        .unwrap();
    assert!(!wandered, "the wander stops as the fight begins");
}

const FISH: u32 = 3_760;
const WALKER: u32 = 3_761;

fn water_data() -> GameData {
    let mut d = data();
    let mut fish = crate::game::tests_p3_combat::template(FISH, 1);
    fish.inhabit_type = 2;
    fish.static_flags1 = 0x1000_0000;
    fish.detection_range = 20.0;
    d.creature_templates.insert(FISH, fish);
    let mut walker = crate::game::tests_p3_combat::template(WALKER, 1);
    walker.inhabit_type = 1;
    walker.detection_range = 20.0;
    d.creature_templates.insert(WALKER, walker);
    d
}

fn swimming(m: &mut Map, g: Guid, on: bool) {
    let p = m.players.get_mut(&g).unwrap();
    p.movement.flags = if on {
        MovementInfo_MovementFlags::empty().set_swimming(Default::default())
    } else {
        MovementInfo_MovementFlags::empty()
    };
}

/// Thistle Tea's `try_aggro` "fish aggro swimmers while ignoring equally
/// close dry targets" and `Acquisition.nearest` "requires an accessible
/// habitat": a fish takes on a player in the water and not one as near on
/// the shore; a creature of the land takes on no one swimming.
#[tokio::test]
async fn a_fish_takes_on_a_swimmer_and_a_walker_no_one_in_the_water() {
    let (realm, _a, ga, map) = world(water_data()).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let (fish, walker) = map
        .query(move |m| {
            stand(m, ga, at(0.0, 0.0));
            stand(m, gb, at(0.0, 0.0));
            (spawn(m, ga, FISH, 20, 8.0), spawn(m, ga, WALKER, 20, -8.0))
        })
        .await
        .unwrap();
    b.sees(fish).await;
    b.sees(walker).await;
    let out = map
        .query(move |m| {
            let victim = |m: &Map, g: Guid| {
                let c = m.creature(g).unwrap();
                c.combat.attacking.then_some(c.combat.victim)
            };
            let look = |m: &mut Map| {
                let now = m.now;
                for g in [fish, walker] {
                    m.creature_mut(g).unwrap().aggro_check_at = now;
                }
                turns(m, 1);
            };
            // Alice on the shore, Bob in the water, as near.
            stand(m, ga, at(0.0, 0.0));
            stand(m, gb, at(0.0, 0.0));
            swimming(m, gb, true);
            look(m);
            (victim(m, fish), victim(m, walker))
        })
        .await
        .unwrap();
    assert_eq!(out.0, Some(gb), "the fish takes the swimmer");
    assert_eq!(out.1, Some(ga), "the walker the one on the shore");
    let none = map
        .query(move |m| {
            swimming(m, ga, true);
            ai::enter_evade(m, walker);
            turns(m, 100);
            let now = m.now;
            m.creature_mut(walker).unwrap().aggro_check_at = now;
            turns(m, 1);
            m.creature(walker).unwrap().combat.attacking
        })
        .await
        .unwrap();
    assert!(!none, "the walker takes no one swimming");
}

/// Thistle Tea's chase "pursues underwater height and repaths when only
/// target depth changes": a fish chasing a swimmer paths again when she
/// goes only deeper, past its re-chase distance, and aims at her depth.
#[tokio::test]
async fn a_fish_chases_a_swimmer_down_when_only_her_depth_changes() {
    let (_realm, _a, ga, map) = world(water_data()).await;
    let out = map
        .query(move |m| {
            stand(m, ga, at(0.0, 0.0));
            swimming(m, ga, true);
            let fish = spawn(m, ga, FISH, 20, 15.0);
            m.creature_mut(fish).unwrap().speeds.run = 0.5;
            ai::attacked(m, fish, ga);
            turns(m, 1);
            let first = m.creature(fish).unwrap().chase_target_at;
            m.players.get_mut(&ga).unwrap().movement.position.z -= 5.0;
            turns(m, 1);
            let second = m.creature(fish).unwrap().chase_target_at;
            (first, second)
        })
        .await
        .unwrap();
    let (first, second) = out;
    let (first, second) = (first.unwrap(), second.unwrap());
    assert!((first.z - START.2).abs() < 0.01);
    assert!(
        (second.z - (START.2 - 5.0)).abs() < 0.01,
        "a new chase, at her depth: {second:?}"
    );
}

/// Thistle Tea's `UnreachableTarget`: "successful routing and leaving
/// combat clear a failed pursuit", "a new victim starts a new timeout and
/// control pauses do not retain stale failure", and "player companions,
/// stationary casters and exempt creatures do not evade from failed
/// paths": a way found to its victim, or the fight's end, and the clock
/// of a victim it couldn't reach is gone; so with another victim, and
/// while it is rooted. A creature someone owns, or one its template
/// exempts (`flags_extra` 0x8), doesn't go home for one it can't reach;
/// one that doesn't chase (a caster standing) starts no clock.
#[tokio::test]
async fn the_clock_of_a_victim_out_of_reach_ends_with_a_way_a_new_victim_a_root_or_the_fight() {
    let mut d = water_data();
    d.creature_templates.get_mut(&WALKER).unwrap().flags_extra = 0x8;
    let (realm, _a, ga, map) = world(d).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            stand(m, ga, at(0.0, 0.0));
            stand(m, gb, at(0.0, 20.0));
            let since = |m: &Map, g: Guid| m.creature(g).unwrap().engagement.unreachable_since;
            let fail = |m: &mut Map, g: Guid, ago: u64| {
                let t = m.now - Duration::from_secs(ago);
                m.creature_mut(g).unwrap().engagement.unreachable_since = Some(t);
            };
            // A way found: gone.
            let k = spawn(m, ga, BRUTE, 20, 15.0);
            ai::attacked(m, k, ga);
            fail(m, k, 1);
            turns(m, 1);
            let found = since(m, k);
            // A new victim: gone.
            fail(m, k, 1);
            ai::add_threat(m, k, gb, 1_000_000.0);
            ai::attacked(m, k, gb);
            turns(m, 1);
            let new_victim = (m.creature(k).unwrap().combat.victim == gb, since(m, k));
            // The fight over: gone.
            fail(m, k, 1);
            ai::enter_evade(m, k);
            let over = since(m, k);
            // Rooted: gone.
            let r = spawn(m, ga, BRUTE, 20, 15.0);
            ai::attacked(m, r, ga);
            crate::game::tests_p3_combat::aura_of(
                m,
                r,
                95_300,
                crate::game::map::auras::AURA_MOD_ROOT,
                0,
                0,
            );
            fail(m, r, 1);
            turns(m, 1);
            let rooted = since(m, r);
            // Owned, or exempt: no evade.
            let owned = spawn(m, ga, BRUTE, 20, 15.0);
            ai::attacked(m, owned, ga);
            m.creature_mut(owned).unwrap().owner = Some(gb);
            fail(m, owned, 50);
            let exempt = spawn(m, ga, WALKER, 20, 15.0);
            ai::attacked(m, exempt, ga);
            fail(m, exempt, 50);
            let plain = spawn(m, ga, BRUTE, 20, 15.0);
            ai::attacked(m, plain, ga);
            fail(m, plain, 50);
            let evades = [owned, exempt, plain]
                .map(|g| crate::game::map::creature_combat::should_evade(m, g, ga));
            // A caster that stands: no clock.
            let caster = spawn(m, ga, BRUTE, 20, 15.0);
            crate::game::map::event_ai::scripted_mut(m, caster)
                .unwrap()
                .no_combat_movement = true;
            ai::attacked(m, caster, ga);
            turns(m, 5);
            let standing = since(m, caster);
            (found, new_victim, over, rooted, evades, standing)
        })
        .await
        .unwrap();
    let (found, new_victim, over, rooted, evades, standing) = out;
    assert_eq!(found, None, "a way found");
    assert_eq!(new_victim, (true, None), "a new victim");
    assert_eq!(over, None, "the fight over");
    assert_eq!(rooted, None, "rooted");
    assert_eq!(evades, [false, false, true], "owned and exempt stay");
    assert_eq!(standing, None, "a caster standing starts none");
}

/// A creature of a faction that flees from calls for help.
const SHY: u32 = 3_762;
const SHY_FACTION: u32 = 41;

/// Thistle Tea's `Assistance.flee_from_help` "retreats from a nearby
/// enemy without joining combat", "requires wandering and rechecks roots,
/// distance and faction" and "passive creatures and critters may retreat
/// even when they cannot assist": called for help, a wanderer of a
/// faction that flees backs off to ten yards past the enemy's edge,
/// joining no fight; not one that doesn't wander, one rooted, or one ten
/// yards off already; one pacified as well.
#[tokio::test]
async fn a_shy_wanderer_called_for_help_backs_away_from_the_enemy_and_joins_no_fight() {
    use crate::game::map::creature::{MOVEMENT_IDLE, MOVEMENT_RANDOM};
    let mut d = data();
    d.faction_templates.insert(
        SHY_FACTION,
        crate::game::data::FactionTemplate {
            flags: 0x400,
            ..crate::game::tests_p3_combat::faction(SHY_FACTION, 0x2, 0x1)
        },
    );
    let mut shy = crate::game::tests_p3_combat::template(SHY, 1);
    shy.faction = SHY_FACTION;
    d.creature_templates.insert(SHY, shy);
    let (_realm, _a, ga, map) = world(d).await;
    let out = map
        .query(move |m| {
            stand(m, ga, at(0.0, 0.0));
            let caller = spawn(m, ga, BRUTE, 20, 10.0);
            ai::attacked(m, caller, ga);
            let backs = |m: &mut Map, yards: f32, how: &dyn Fn(&mut Map, Guid)| {
                let g = spawn(m, ga, SHY, 20, yards);
                m.creature_mut(g).unwrap().movement_type = MOVEMENT_RANDOM;
                how(m, g);
                crate::game::map::assistance::call_friends(m, caller, ga, 20.0);
                let c = m.creature(g).unwrap();
                let to = c
                    .scripted
                    .as_deref()
                    .and_then(|s| s.scripted_move)
                    .map(|s| s.to);
                let calm = !c.combat.attacking
                    && c.threat.is_empty()
                    && !c.fields.has_flag(
                        UNIT_FIELD_FLAGS,
                        crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                    );
                ai::die(m, g, ga);
                (to, calm)
            };
            let plain = backs(m, 3.0, &|_, _| {});
            let idle = backs(m, 3.0, &|m, g| {
                m.creature_mut(g).unwrap().movement_type = MOVEMENT_IDLE;
            });
            let rooted = backs(m, 3.0, &|m, g| {
                crate::game::tests_p3_combat::aura_of(
                    m,
                    g,
                    95_301,
                    crate::game::map::auras::AURA_MOD_ROOT,
                    0,
                    0,
                );
            });
            let far = backs(m, 11.0, &|_, _| {});
            let pacified = backs(m, 3.0, &|m, g| {
                m.creature_mut(g)
                    .unwrap()
                    .fields
                    .set_flag(UNIT_FIELD_FLAGS, 0x2_0000, true);
            });
            let edge = m
                .unit_fields(ga)
                .map_or(0.0, |f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS));
            (plain, idle, rooted, far, pacified, edge)
        })
        .await
        .unwrap();
    let (plain, idle, rooted, far, pacified, edge) = out;
    let to = plain.0.expect("it backs away");
    assert!(
        (dist(to, at(0.0, 0.0)) - (10.0 + edge)).abs() < 0.01,
        "ten yards past her edge: {to:?}"
    );
    assert!(plain.1, "joining no fight");
    assert_eq!(idle.0, None, "not one that doesn't wander");
    assert_eq!(rooted.0, None, "nor one rooted");
    assert_eq!(far.0, None, "nor one ten yards off");
    assert!(
        pacified.0.is_some() && pacified.1,
        "one pacified backs away too"
    );
}

/// Thistle Tea's distancing "finishes at the requested distance and
/// releases the spell-list deadline" and "death and combat exit clear
/// distancing": a creature backing off from its victim (movement 19)
/// stops at its distance, and its spell list tries again at once; an
/// evade, or its death, ends a retreat under way.
#[tokio::test]
async fn a_retreat_ends_at_its_distance_freeing_the_spell_list_or_with_an_evade_or_a_death() {
    use crate::game::map::spells::lists;
    let (_realm, _a, ga, map) = world(data()).await;
    let out = map
        .query(move |m| {
            stand(m, ga, at(0.0, 0.0));
            let away = crate::game::data::scripts::ScriptStep {
                x: 8.0,
                ..crate::game::tests_p3_entity::step(20, [19, 0, 0, 0])
            };
            // A caster that stands where it is.
            let retreat = |m: &mut Map| {
                let k = spawn(m, ga, BRUTE, 20, 3.0);
                crate::game::map::event_ai::scripted_mut(m, k)
                    .unwrap()
                    .no_combat_movement = true;
                ai::attacked(m, k, ga);
                crate::game::map::scripts::execute(m, &away, k, ga);
                k
            };
            // To its distance; its list's deadline gone.
            let k = retreat(m);
            let started = lists::distancing(m, k);
            let later = m.now + Duration::from_secs(10);
            m.creature_mut(k).unwrap().caster.spell_list = Some(Box::new(lists::ListState {
                next_at: Some(later),
                ..Default::default()
            }));
            turns(m, 80);
            let edge = m
                .unit_fields(ga)
                .map_or(0.0, |f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS));
            let arrived = (
                lists::distancing(m, k),
                (dist(m.position_of(k).unwrap(), at(0.0, 0.0)) - (8.0 + edge)).abs() < 0.1,
                m.creature(k)
                    .unwrap()
                    .caster
                    .spell_list
                    .as_deref()
                    .map(|l| l.next_at),
            );
            // One that chases again once there: its list freed first.
            let c = retreat(m);
            crate::game::map::event_ai::scripted_mut(m, c)
                .unwrap()
                .no_combat_movement = false;
            m.creature_mut(c).unwrap().caster.spell_list = Some(Box::new(lists::ListState {
                next_at: Some(later),
                ..Default::default()
            }));
            turns(m, 80);
            let chaser = m
                .creature(c)
                .unwrap()
                .caster
                .spell_list
                .as_deref()
                .map(|l| l.next_at);
            ai::die(m, c, ga);
            // An evade, a death: over.
            let e = retreat(m);
            ai::enter_evade(m, e);
            turns(m, 1);
            let evaded = lists::distancing(m, e);
            let d = retreat(m);
            ai::die(m, d, ga);
            turns(m, 1);
            let died = lists::distancing(m, d);
            (started, arrived, chaser, evaded, died)
        })
        .await
        .unwrap();
    let (started, arrived, chaser, evaded, died) = out;
    assert!(started, "it backs off");
    assert_eq!(arrived, (false, true, Some(None)), "there, its list free");
    assert_eq!(chaser, Some(None), "a chaser's too, before it chases");
    assert!(!evaded, "an evade ends it");
    assert!(!died, "a death ends it");
}
