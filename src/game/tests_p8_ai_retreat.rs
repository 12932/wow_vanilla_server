//! Round 8's creature AI tests of distancing, ported from Thistle Tea's
//! `test/game/core/ai/bt/distancing_test.exs` (docs/parity/matrix.csv):
//! a script's retreat from its target (movement 19), where it goes, what
//! it needs to start, the casts it breaks and holds, and what calls it
//! off.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::scripts::ScriptStep;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::combat::{self, dist};
use crate::game::map::creature::Motion;
use crate::game::map::spells::lists;
use crate::game::map::{Map, MapHandle, ai, event_ai, scripts};
use crate::game::tests_p3_combat::{BRUTE, START, data, setup_with, spawn};

/// A slow bolt that moving breaks, and one it doesn't.
const BREAKS: u32 = 95_801;
const STAYS: u32 = 95_802;

fn world() -> GameData {
    let mut d = data();
    d.spells.ranges.insert(2, (0.0, 40.0));
    d.spells.cast_times.insert(99, 3_000);
    for (id, interrupt_flags) in [(BREAKS, 0x1), (STAYS, 0x8)] {
        let mut s = SpellTemplate {
            id,
            name: format!("Bolt {id}"),
            range_index: 2,
            casting_time_index: 99,
            interrupt_flags,
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 2,
            base_points: 0,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    d
}

/// The map's creatures take `n` turns of 50 ms.
fn turns(m: &mut Map, n: u32) {
    for _ in 0..n {
        m.now += Duration::from_millis(50);
        ai::update_creatures(m);
        combat::update_creatures(m);
    }
}

fn at(east: f32, north: f32, up: f32) -> Vector3d {
    Vector3d {
        x: START.0 + east,
        y: START.1 + north,
        z: START.2 + up,
    }
}

/// The player at `p`, facing east, with health to spare.
fn stand(m: &mut Map, g: Guid, p: Vector3d) {
    let pl = m.players.get_mut(&g).unwrap();
    pl.movement.position = p;
    pl.movement.orientation = 0.0;
    pl.movement.flags = MovementInfo_MovementFlags::empty();
    pl.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
    pl.fields.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
}

/// A world with Alice in it at `at(0, 0, 0)`, her first heartbeat in.
async fn alice() -> (
    std::sync::Arc<crate::game::Realm>,
    crate::game::harness::Client,
    Guid,
    MapHandle,
) {
    let (realm, a, ga, map) = setup_with(world(), Class::Warrior).await;
    let deadline = tokio::time::Instant::now() + crate::game::harness::WAIT;
    while map
        .query(move |m| dist(m.players[&ga].position(), at(0.0, 0.0, 0.0)) > 0.01)
        .await
        .unwrap()
    {
        assert!(tokio::time::Instant::now() < deadline, "her heartbeat");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    (realm, a, ga, map)
}

/// Movement 19: back off `x` yards past the target's edge, with at
/// least `mana` percent of its mana.
fn away(x: f32, mana: u32) -> ScriptStep {
    ScriptStep {
        x,
        ..crate::game::tests_p3_entity::step(20, [19, 0, mana, 0])
    }
}

/// A caster fighting Alice, `yards` east of her, that stands where it
/// fights.
fn caster(m: &mut Map, ga: Guid, yards: f32) -> Guid {
    let k = spawn(m, ga, BRUTE, 20, yards);
    event_ai::scripted_mut(m, k).unwrap().no_combat_movement = true;
    ai::attacked(m, k, ga);
    k
}

/// Where its retreat is taking it.
fn retreat_to(m: &Map, g: Guid) -> Option<Vector3d> {
    event_ai::scripted(m, g)
        .and_then(|s| s.scripted_move)
        .filter(|s| s.distancing.is_some())
        .map(|s| s.to)
}

fn casting(m: &Map, g: Guid) -> Option<u32> {
    m.caster(g)
        .and_then(|c| c.casting.as_ref())
        .map(|c| c.spell)
}

fn spline(m: &Map, g: Guid) -> Option<u32> {
    match &m.creature(g)?.motion {
        Motion::Moving(w) => Some(w.spline_id),
        Motion::Idle { .. } => None,
    }
}

fn edge(m: &Map, g: Guid) -> f32 {
    m.unit_fields(g)
        .map_or(0.0, |f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS))
}

/// Thistle Tea's "loads caster distancing with its mana threshold" (the
/// threshold) and "includes target size and leaves combat ownership
/// untouched": with datalong3 a creature backs off only at that share of
/// its mana or more (not at 19 of 100 for 20, at 20 and at 100); it goes
/// to x yards past its target's edge, its victim, threat and fight as they
/// were, and it doesn't flee.
#[tokio::test]
async fn a_retreat_goes_x_yards_past_its_targets_edge_at_its_share_of_mana_its_fight_as_it_was() {
    let (_realm, _a, ga, map) = alice().await;
    let out = map
        .query(move |m| {
            stand(m, ga, at(0.0, 0.0, 0.0));
            let by_mana: Vec<bool> = [19, 20, 100]
                .into_iter()
                .map(|mana| {
                    let k = caster(m, ga, 3.0);
                    let f = m.unit_fields_mut(k).unwrap();
                    f.set_u32(UNIT_FIELD_MAXPOWER1, 100);
                    f.set_u32(UNIT_FIELD_POWER1, mana);
                    scripts::execute(m, &away(12.0, 20), k, ga);
                    let started = lists::distancing(m, k);
                    ai::die(m, k, ga);
                    started
                })
                .collect();
            let k = caster(m, ga, 3.0);
            let threat = m.creature(k).unwrap().threat.clone();
            scripts::execute(m, &away(12.0, 0), k, ga);
            let c = m.creature(k).unwrap();
            let fight = (
                c.combat.victim == ga,
                c.combat.attacking,
                c.threat == threat,
                c.engagement.flee.is_none(),
            );
            let to = retreat_to(m, k).map(|to| dist(to, at(0.0, 0.0, 0.0)) - 12.0 - edge(m, ga));
            (by_mana, fight, to)
        })
        .await
        .unwrap();
    let (by_mana, fight, to) = out;
    assert_eq!(by_mana, [false, true, true], "at 20% of its mana or more");
    assert_eq!(fight, (true, true, true, true), "its fight as it was");
    assert!(
        to.is_some_and(|d| d.abs() < 0.01),
        "twelve yards past her edge: {to:?}"
    );
}

/// Thistle Tea's "checks target line of sight and ground height before
/// interrupting a cast", "failed geometry and paths preserve the current
/// cast and movement" and "preserves casts which do not interrupt on
/// movement": a retreat breaks a cast that moving breaks and leaves one
/// it doesn't; one whose point its target can't see (a wall between),
/// more than ten yards above it, or where it already stands, doesn't
/// start, the cast under way and the walk it was on left as they were.
#[tokio::test]
async fn a_retreat_needs_its_point_seen_by_its_target_and_not_far_above_else_its_cast_and_walk_go_on()
 {
    let (_realm, _a, ga, map) = alice().await;
    let out = map
        .query(move |m| {
            stand(m, ga, at(0.0, 0.0, 0.0));
            // A cast that moving breaks, one that it doesn't.
            let cast_then_back_off = |m: &mut Map, spell: u32| {
                let k = caster(m, ga, 3.0);
                scripts::cast(m, k, ga, spell, 0);
                let was = casting(m, k);
                scripts::execute(m, &away(12.0, 0), k, ga);
                let after = (was, casting(m, k), lists::distancing(m, k));
                ai::die(m, k, ga);
                after
            };
            let breaks = cast_then_back_off(m, BREAKS);
            let stays = cast_then_back_off(m, STAYS);
            // Walking north and casting, it is told to back off where it
            // can't go: its walk and cast go on.
            let refused = |m: &mut Map, lift: f32| {
                let k = caster(m, ga, 3.0);
                stand(m, ga, at(0.0, 0.0, lift));
                scripts::move_to(m, k, at(3.0, 20.0, 0.0), false);
                scripts::cast(m, k, ga, BREAKS, 0);
                let before = (spline(m, k), casting(m, k));
                scripts::execute(m, &away(12.0, 0), k, ga);
                let after = (spline(m, k), casting(m, k));
                let out = (
                    before.0.is_some() && before.1.is_some() && before == after,
                    lists::distancing(m, k),
                );
                ai::die(m, k, ga);
                stand(m, ga, at(0.0, 0.0, 0.0));
                out
            };
            // A wall across its way, between her and where it would go.
            m.spells.walls.push((at(8.0, -5.0, 0.0), at(8.0, 5.0, 0.0)));
            let walled = refused(m, 0.0);
            m.spells.walls.clear();
            // She stands on a ledge eleven yards up.
            let high = refused(m, 11.0);
            // It stands there already.
            let there = {
                let k = caster(m, ga, 12.0 + edge(m, ga));
                scripts::execute(m, &away(12.0, 0), k, ga);
                let out = (spline(m, k), lists::distancing(m, k));
                ai::die(m, k, ga);
                out
            };
            // With the way clear, or her ledge lower, it does go.
            let clear = (refused(m, 0.0).1, refused(m, 9.0).1);
            (breaks, stays, walled, high, there, clear)
        })
        .await
        .unwrap();
    let (breaks, stays, walled, high, there, clear) = out;
    assert_eq!(breaks, (Some(BREAKS), None, true), "moving breaks it");
    assert_eq!(stays, (Some(STAYS), Some(STAYS), true), "moving leaves it");
    assert_eq!(walled, (true, false), "not where she can't see");
    assert_eq!(high, (true, false), "not ten yards up");
    assert_eq!(there, (None, false), "not where it stands");
    assert_eq!(clear, (true, true), "with the way clear, it backs off");
}

/// Thistle Tea's "blocks normal casts during retreat while allowing
/// forced casts" and "continues after a speed change replaces the
/// spline": while it backs off a creature's spell list casts only a spell
/// forced (cast flag 0x04); a slow that sends its run on at its new speed
/// leaves it backing off, to the same point.
#[tokio::test]
async fn a_retreat_holds_all_but_forced_casts_and_outlasts_a_change_of_speed() {
    let (_realm, _a, ga, map) = alice().await;
    let out = map
        .query(move |m| {
            stand(m, ga, at(0.0, 0.0, 0.0));
            let k = caster(m, ga, 3.0);
            let before = (
                lists::flags_allow(m, k, 0, BREAKS, ga),
                lists::flags_allow(m, k, 0x04, BREAKS, ga),
            );
            scripts::execute(m, &away(12.0, 0), k, ga);
            let during = (
                lists::flags_allow(m, k, 0, BREAKS, ga),
                lists::flags_allow(m, k, 0x04, BREAKS, ga),
            );
            let (to, old) = (retreat_to(m, k), spline(m, k));
            turns(m, 2);
            // Wounded, it runs slower: a new spline from where it is.
            m.creature_mut(k).unwrap().wounded = 0.5;
            crate::game::map::derive::unit(m, k);
            let new = spline(m, k);
            turns(m, 2);
            let slowed = (
                new.is_some() && new != old,
                lists::distancing(m, k),
                retreat_to(m, k) == to,
            );
            turns(m, 200);
            let arrived = (
                lists::distancing(m, k),
                dist(m.position_of(k).unwrap(), to.unwrap()) < 0.1,
            );
            (before, during, slowed, arrived)
        })
        .await
        .unwrap();
    let (before, during, slowed, arrived) = out;
    assert_eq!(before, (true, true), "standing, any");
    assert_eq!(during, (false, true), "backing off, only a forced one");
    assert_eq!(slowed, (true, true, true), "slowed, it backs off still");
    assert_eq!(arrived, (false, true), "and gets there");
}

/// Thistle Tea's "root, stun, fear and victim loss cancel rather than
/// resume retreat" and "a replacement path survives cancellation of the
/// old retreat": a retreat stopped by a root, a stun or a fear isn't taken
/// up again once it ends; one whose creature turns on another victim, or
/// whose target dies, stops where it is; a walk that took its place goes
/// on.
#[tokio::test]
async fn a_retreat_held_or_its_victim_changed_stops_for_good_but_a_walk_in_its_place_goes_on() {
    use crate::game::map::auras::{AURA_MOD_FEAR, AURA_MOD_ROOT, AURA_MOD_STUN};
    let (_realm, _a, ga, map) = alice().await;
    let out = map
        .query(move |m| {
            stand(m, ga, at(0.0, 0.0, 0.0));
            let held: Vec<(bool, bool)> = [AURA_MOD_ROOT, AURA_MOD_STUN, AURA_MOD_FEAR]
                .into_iter()
                .map(|aura| {
                    let k = caster(m, ga, 3.0);
                    scripts::execute(m, &away(12.0, 0), k, ga);
                    let s = crate::game::tests_p3_spells_kit::made_up(95_810 + aura, 6, aura, 0, 0);
                    crate::game::map::auras::apply(m, ga, k, &s, 0, 0);
                    turns(m, 4);
                    crate::game::map::auras::remove_spell(m, k, s.id, None);
                    turns(m, 4);
                    let out = (lists::distancing(m, k), spline(m, k).is_some());
                    ai::die(m, k, ga);
                    out
                })
                .collect();
            // Another victim.
            let k = caster(m, ga, 3.0);
            let other = spawn(m, ga, BRUTE, 20, 6.0);
            let hers = m.players[&ga].fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE);
            m.unit_fields_mut(other)
                .unwrap()
                .set_u32(UNIT_FIELD_FACTIONTEMPLATE, hers);
            scripts::execute(m, &away(12.0, 0), k, ga);
            turns(m, 1);
            // It hates the other far more.
            m.creature_mut(k).unwrap().threat.push((other, 1e6));
            turns(m, 1);
            let turned = m.creature(k).unwrap().combat.victim == other;
            let victim = (lists::distancing(m, k), spline(m, k).is_some());
            ai::die(m, k, ga);
            ai::die(m, other, ga);
            // Another walk in its place (out of a fight, which would take
            // its movement): the retreat is forgotten.
            let k = spawn(m, ga, BRUTE, 20, 3.0);
            scripts::execute(m, &away(12.0, 0), k, ga);
            let path = vec![m.position_of(k).unwrap(), at(3.0, 20.0, 0.0)];
            let (now, id) = (m.now, m.next_spline_id);
            m.next_spline_id += 1;
            let _ = m.creature_mut(k).unwrap().walk(path, false, now, id);
            turns(m, 1);
            let replaced = (lists::distancing(m, k), spline(m, k));
            (held, turned, victim, replaced, id)
        })
        .await
        .unwrap();
    let (held, turned, victim, replaced, id) = out;
    assert_eq!(held, [(false, false); 3], "a root, stun or fear ends it");
    assert!(turned, "it turned on the other");
    assert_eq!(victim, (false, false), "another victim stops it");
    assert_eq!(replaced, (false, Some(id)), "the walk in its place goes on");
}

/// Thistle Tea's "loads caster distancing with its mana threshold" (the
/// rows): creature AI script 59901 backs off twelve yards with a fifth of
/// its mana or more; 9503 fifteen, whatever its mana.
#[test]
#[ignore = "extended"]
fn the_real_casters_retreats_carry_their_distance_and_share_of_mana() {
    let d = crate::game::data::load::real_data().expect("the real world");
    let of = |id: u32| {
        d.scripts.ai_scripts[&id]
            .iter()
            .map(|s| (s.command, s.datalong[0], s.datalong[2], s.x))
            .collect::<Vec<_>>()
    };
    assert_eq!(of(59_901), [(20, 19, 20, 12.0)]);
    assert_eq!(of(9_503), [(20, 19, 0, 15.0)]);
}
