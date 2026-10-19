//! Round 7's combat tests, ported from Thistle Tea's `test/game/core/combat`
//! (docs/parity/matrix.csv), with the helpers the other `tests_p7_combat_*`
//! files share: a creature's notice of those who come near it (Thistle
//! Tea's `proximity_test.exs`), judged where each stands at every look.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::tests_p3_combat::{self as p3, BRUTE, PACK, START};
use crate::game::tests_p6_wspell::put;

/// Aggressive, noticing at 10 yards (an even level's radius).
pub(crate) const WATCHER: u32 = 3750;
/// The same at 20 yards, for stealth.
pub(crate) const WATCHER_20: u32 = 3751;
/// A tameable watcher, a beast.
pub(crate) const BEAST_WATCHER: u32 = 3752;

pub(crate) fn data() -> GameData {
    let mut d = p3::data();
    for (entry, yards) in [(WATCHER, 10.0), (WATCHER_20, 20.0)] {
        let mut t = p3::template(entry, 7);
        t.detection_range = yards;
        d.creature_templates.insert(entry, t);
    }
    let mut beast = p3::template(BEAST_WATCHER, 1);
    beast.detection_range = 10.0;
    beast.type_flags = 1;
    d.creature_templates.insert(BEAST_WATCHER, beast);
    d
}

pub(crate) type Setup = (
    std::sync::Arc<crate::game::Realm>,
    crate::game::harness::Client,
    Guid,
    crate::game::map::MapHandle,
);

/// A level-60 Alice in a world of `data`.
pub(crate) async fn setup_with(data: GameData, class: Class) -> Setup {
    let (realm, a, ga, map) = p3::setup_with(data, class).await;
    map.query(move |m| p3::level(m, ga, 60)).await.unwrap();
    (realm, a, ga, map)
}

/// [`setup_with`] in a world of [`data`].
pub(crate) async fn setup(class: Class) -> Setup {
    setup_with(data(), class).await
}

/// A creature of `entry` at level 60, at `at`, its home there, and known
/// to those near.
pub(crate) fn creature_at(m: &mut Map, ga: Guid, entry: u32, at: Vector3d) -> Guid {
    let g = p3::spawn(m, ga, entry, 60, 1.0);
    put(m, g, at);
    if let Some(c) = m.creature_mut(g) {
        c.home = at;
    }
    crate::game::map::visibility::refresh_dirty(m);
    g
}

/// The start, `dx` yards east, `dy` north and `dz` up.
pub(crate) fn off(dx: f32, dy: f32, dz: f32) -> Vector3d {
    Vector3d {
        x: START.0 + dx,
        y: START.1 + dy,
        z: START.2 + dz,
    }
}

/// Half a second of the creatures' time: each looks about it once.
pub(crate) fn look(m: &mut Map) {
    m.now += Duration::from_millis(500);
    crate::game::map::ai::update_creatures(m);
}

pub(crate) fn in_combat(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT))
}

pub(crate) fn attacking(m: &Map, g: Guid, victim: Guid) -> bool {
    m.creature(g)
        .is_some_and(|c| c.combat.attacking && c.combat.victim == victim)
}

/// A creature's fight, and the player's, over at once.
pub(crate) fn part(m: &mut Map, g: Guid, player: Guid) {
    crate::game::map::ai::enter_evade(m, g);
    crate::game::map::combat::leave_combat(m, player);
}

/// Thistle Tea's `contact/4`: a walker comes into contact where its way
/// first enters the radius (not before, wherever it is headed); a way
/// that passes by, or went past already, never does; one that ended is
/// judged where it ended; and height counts toward the distance. wvs
/// judges each unit where it stands at every look (each half second),
/// rather than foretelling a path, which comes to the same.
#[tokio::test]
async fn a_walker_is_noticed_where_it_first_comes_within_reach_height_and_all() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            // Walking in from 31 yards, two at a time: noticed at 9.
            let w = creature_at(m, ga, WATCHER, off(0.0, 0.0, 0.0));
            let mut noticed_at = None;
            for step in 0..15 {
                let d = 31.0 - 2.0 * step as f32;
                put(m, ga, off(d, 0.0, 0.0));
                look(m);
                if attacking(m, w, ga) {
                    noticed_at = Some(d);
                    break;
                }
            }
            part(m, w, ga);
            // Passing by 12 yards off, and walking away from 11 yards:
            // never.
            put(m, ga, off(80.0, 12.0, 0.0));
            let by = creature_at(m, ga, WATCHER, off(100.0, 0.0, 0.0));
            let mut passed = false;
            for step in 0..21 {
                put(m, ga, off(80.0 + 2.0 * step as f32, 12.0, 0.0));
                look(m);
                passed |= in_combat(m, by) || in_combat(m, ga);
            }
            for step in 0..10 {
                put(m, ga, off(111.0 + 2.0 * step as f32, 0.0, 0.0));
                look(m);
                passed |= in_combat(m, by) || in_combat(m, ga);
            }
            // A way that ended 2 yards off: noticed there.
            put(m, ga, off(102.0, 0.0, 0.0));
            look(m);
            let ended = attacking(m, by, ga);
            part(m, by, ga);
            // 9.6 yards off on the level is within 10; three yards up as
            // well, it is 10.06 off, and out of reach.
            put(m, ga, off(209.6, 0.0, 3.0));
            let high = creature_at(m, ga, WATCHER, off(200.0, 0.0, 0.0));
            look(m);
            look(m);
            let above = attacking(m, high, ga);
            put(m, ga, off(209.6, 0.0, 0.0));
            look(m);
            (noticed_at, passed, ended, above, attacking(m, high, ga))
        })
        .await
        .unwrap();
    assert_eq!(r.0, Some(9.0), "noticed where it first comes within 10");
    assert!(!r.1, "passing by or going away, never");
    assert!(r.2, "a way that ended is judged where it ended");
    assert!(!r.3, "three yards up puts 9.6 out of 10");
    assert!(r.4, "and on the level within it");
}

/// Thistle Tea's `aggressor/1`: a creature fighting already, dead, a
/// player's pet, on its way home or passive notices no one; and a player
/// never starts a fight by coming near.
#[tokio::test]
async fn only_an_idle_living_free_aggressive_creature_notices_and_never_a_player() {
    let (realm, _a, ga, map) = setup(Class::Warrior).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Dwarf, Class::Hunter).await;
    let r = map
        .query(move |m| {
            p3::level(m, gb, 60);
            m.unit_fields_mut(gb)
                .unwrap()
                .set_u32(UNIT_FIELD_FACTIONTEMPLATE, p3::HUMANS);
            put(m, gb, off(0.0, -30.0, 0.0));
            put(m, ga, off(0.0, 0.0, 0.0));
            // One fighting another: Alice, near, is not taken on.
            let busy = creature_at(m, ga, WATCHER, off(5.0, 0.0, 0.0));
            let other = creature_at(m, ga, BRUTE, off(5.0, 2.0, 0.0));
            m.unit_fields_mut(other)
                .unwrap()
                .set_u32(UNIT_FIELD_FACTIONTEMPLATE, p3::HUMANS);
            crate::game::map::ai::attacked(m, busy, other);
            // One dead, one walking home (far), one a pet of Bob's.
            let dead = creature_at(m, ga, WATCHER, off(-5.0, 0.0, 0.0));
            crate::game::map::combat::kill(m, dead, dead);
            let home = creature_at(m, ga, WATCHER, off(0.0, 5.0, 0.0));
            m.creature_mut(home).unwrap().home = off(0.0, 80.0, 0.0);
            crate::game::map::ai::attacked(m, home, gb);
            part(m, home, gb);
            // Bob's pet, a watcher too, is of Bob's side, and a creature of
            // the pack's is by it: it stays unnoticed.
            crate::game::map::pets::give_pet(m, gb, BEAST_WATCHER, 60).expect("a pet");
            let pet = crate::game::map::pets::pet_of(m, gb).expect("Bob's pet");
            put(m, pet, off(0.0, -25.0, 0.0));
            let near_pet = creature_at(m, ga, BRUTE, off(0.0, -27.0, 0.0));
            m.unit_fields_mut(near_pet)
                .unwrap()
                .set_u32(UNIT_FIELD_FACTIONTEMPLATE, PACK);
            for _ in 0..4 {
                look(m);
                crate::game::map::pets::update(m);
            }
            let busy_took = m
                .creature(busy)
                .is_some_and(|c| c.combat.victim == ga || c.threat.iter().any(|(t, _)| *t == ga));
            let home_took = m.creature(home).is_some_and(|c| c.combat.victim == ga);
            let pet_took = m.creature(pet).is_some_and(|c| c.combat.attacking);
            // Alice, by all of them, starts nothing herself.
            let alice = in_combat(m, ga) || m.combat(ga).is_some_and(|c| c.attacking);
            (
                busy_took,
                m.creature(dead).is_some_and(|c| c.combat.attacking),
                home_took,
                pet_took,
                alice,
                attacking(m, busy, other),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        r,
        (false, false, false, false, false, true),
        "(busy, dead, home, pet, Alice) take no one; the busy one keeps its own"
    );
}

/// Thistle Tea's hidden announcer (`announcement/5`, `refresh_at/4`): one
/// stealthed is noticed only as near as the creature sees through it,
/// looked at again at each step it walks; and once it shows itself, at
/// the creature's whole reach.
#[tokio::test]
async fn a_stealthed_walker_is_noticed_only_where_it_is_seen_and_at_full_reach_once_out() {
    let (_realm, _a, ga, map) = setup(Class::Rogue).await;
    let r = map
        .query(move |m| {
            // A level-50 rogue's stealth of 250 under a level-60 eye of
            // 300, ten levels up (twice the reach): seen within 5/6 +
            // 50 x 5/6 x 2 / 5 = 17.5 yards; unstealthed, at 30 (20 and
            // the ten levels).
            p3::level(m, ga, 50);
            put(m, ga, off(29.0, 0.0, 0.0));
            let w = creature_at(m, ga, WATCHER_20, off(0.0, 0.0, 0.0));
            // Facing the rogue: from behind it would see less.
            m.creature_mut(w).unwrap().orientation = 0.0;
            p3::aura_of(
                m,
                ga,
                95_300,
                crate::game::map::auras::AURA_MOD_STEALTH,
                0,
                1,
            );
            let mut noticed_at = None;
            for step in 0..15 {
                let d = 29.0 - 1.5 * step as f32;
                put(m, ga, off(d, 0.0, 0.0));
                look(m);
                if attacking(m, w, ga) {
                    noticed_at = Some(d);
                    break;
                }
            }
            part(m, w, ga);
            // 25 yards from another, unseen; out of stealth, noticed at
            // once.
            put(m, ga, off(125.0, 0.0, 0.0));
            let other = creature_at(m, ga, WATCHER_20, off(100.0, 0.0, 0.0));
            m.creature_mut(other).unwrap().orientation = 0.0;
            look(m);
            let hidden = attacking(m, other, ga);
            crate::game::map::auras::remove_spell(m, ga, 95_300, None);
            look(m);
            (noticed_at, hidden, attacking(m, other, ga))
        })
        .await
        .unwrap();
    assert_eq!(r.0, Some(17.0), "seen within seventeen and a half yards");
    assert!(!r.1, "unseen at twenty-five");
    assert!(r.2, "and seen there once out of stealth");
}

/// Thistle Tea's "selection changes retain the live melee victim": a
/// player swinging at one creature who selects another goes on swinging at
/// the first, the second shown as her target.
#[tokio::test]
async fn selecting_another_keeps_the_swing_on_its_victim() {
    let (_realm, a, ga, map) = setup(Class::Warrior).await;
    let (brute, other) = map
        .query(move |m| {
            put(m, ga, off(0.0, 0.0, 0.0));
            (
                creature_at(m, ga, BRUTE, off(2.0, 0.0, 0.0)),
                creature_at(m, ga, BRUTE, off(0.0, 2.0, 0.0)),
            )
        })
        .await
        .unwrap();
    a.send(CMSG_ATTACKSWING { guid: brute }).await;
    a.send(CMSG_SET_SELECTION { target: other }).await;
    let mut seen = None;
    for _ in 0..200 {
        seen = map
            .query(move |m| {
                let target = m.players[&ga].fields.get_guid(UNIT_FIELD_TARGET);
                let c = m.combat(ga).unwrap();
                (target == other).then_some((c.attacking, c.victim == brute))
            })
            .await
            .unwrap();
        if seen.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(seen, Some((true, true)), "(swinging, at the first)");
}
