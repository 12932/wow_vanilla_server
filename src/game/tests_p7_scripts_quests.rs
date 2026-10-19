//! Round 7's creature script tests, continued: the event quests Thistle
//! Tea builds in code (Freedom for All Creatures, The Balance of Light and
//! Shadow, The Great Masquerade), a step at a time on the real world.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use super::tests_p6_scripts::*;
use super::tests_p7_scripts::*;
use crate::game::Realm;
use crate::game::harness::Client;
use crate::game::map::combat::dist;
use crate::game::map::{Map, MapHandle, event_ai, map_events, scripts};

const KINDAL: u32 = 7956;
const SPRITE: u32 = 7997;
const FREEDOM: u32 = 2969;
const CAGE: u32 = 143979;
const PEN: Vector3d = Vector3d {
    x: -4533.17,
    y: 808.743,
    z: 60.29,
};

/// Freedom for All Creatures taken from Kindal: the client, the player,
/// the map, Kindal.
async fn freedom(realm: &std::sync::Arc<Realm>, name: &str) -> (Client, Guid, MapHandle, Guid) {
    let (mut c, me, map, kindal) = hero_by(realm, name, KINDAL).await;
    c.clear();
    assert!(
        takes(&mut c, &map, kindal, FREEDOM).await,
        "Freedom for All Creatures taken"
    );
    (c, me, map, kindal)
}

/// The player and Kindal at the pen, its cage opened: the sprites it
/// freed.
async fn open_the_pen(map: &MapHandle, me: Guid, kindal: Guid) -> Vec<Guid> {
    map.query(move |m| {
        let (cage, at) = m
            .gameobjects
            .iter()
            .filter(|(_, o)| o.entry == CAGE)
            .map(|(_, o)| (o.guid, o.position))
            .min_by(|a, b| dist(a.1, PEN).total_cmp(&dist(b.1, PEN)))
            .expect("the cage");
        // The Grimtotems about the camp gone, who would fall on the
        // sprites.
        clear_around(m, at, 150.0, &[SPRITE, KINDAL]);
        stand(m, me, off(at, 1.0, 0.0));
        scripts::place(m, kindal, off(at, 2.0, 0.0), 0.0);
        let sprites = scripts::creatures_near(m, at, SPRITE, 45.0, true);
        scripts::object_used(m, me, cage);
        sprites
    })
    .await
    .unwrap()
}

fn creature_motion_points(m: &Map, g: Guid) -> Vec<(f32, f32, f32)> {
    crate::game::map::creature_motion::waypoints(m, g)
        .unwrap_or_default()
        .iter()
        .map(|p| (p.x, p.y, p.z))
        .collect()
}

/// Whether `said` has `line` (its names filled in: up to its first `$`).
fn says(said: &[String], line: &str) -> bool {
    let head = line.split('$').next().unwrap_or_default();
    said.iter().any(|s| s.starts_with(head))
}

/// The escape route a sprite runs: its variant.
fn escape_route(m: &Map, g: Guid) -> Option<u32> {
    let route = event_ai::scripted(m, g)?.route.as_ref()?.points.clone();
    (1..=11).find(|v| {
        m.realm
            .data
            .scripts
            .routes
            .get(&(SPRITE, *v))
            .is_some_and(|r| r.len() == route.len() && r[0].x == route[0].x)
    })
}

/// Freedom for All Creatures taken: Kindal stands, says her piece, turns
/// escortee with no menu or offer, and three seconds on follows the
/// player; the sprites' count starts with it. Attacked, she shouts one of
/// her four battle cries (Thistle Tea's `kindal_moonweaver`).
#[tokio::test]
#[ignore = "extended"]
async fn kindal_takes_up_the_trial_follows_and_cries_out_when_attacked() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, kindal) = freedom(&realm, "Kindly").await;
    let (sprites, stood, escortee, flags, follows) = map
        .query(move |m| {
            (
                map_events::active(m, SPRITE),
                stand_state(m, kindal),
                faction(m, kindal),
                npc_flags(m, kindal),
                event_ai::scripted(m, kindal).and_then(|s| s.follow),
            )
        })
        .await
        .unwrap();
    assert!(sprites, "the sprites' count under way");
    assert_eq!(stood, 0, "she stands");
    assert_eq!(escortee, 231, "an escortee");
    assert_eq!(flags, 0, "no menu, no offer");
    assert!(follows.is_none(), "not at once");
    assert!(hears(&mut c, &realm, kindal, 4079).await, "her piece");
    hurry_all(&map, 3.0).await;
    assert!(
        until(&map, 2_000, move |m| event_ai::scripted(m, kindal)
            .and_then(|s| s.follow)
            .is_some_and(|f| f.0 == me))
        .await,
        "three seconds on, she follows"
    );
    // Attacked.
    let cries: Vec<String> = (4122..=4125).map(|t| line(&realm, t)).collect();
    c.clear();
    map.query(move |m| {
        let at = m.position_of(kindal).unwrap();
        let foe = dummy(m, off(at, 2.0, 0.0));
        crate::game::map::ai::attacked(m, kindal, foe);
    })
    .await;
    let said = heard(&mut c, kindal, Duration::from_secs(2)).await;
    assert!(
        said.as_ref().is_some_and(|s| cries.contains(s)),
        "a battle cry: {said:?}"
    );
}

/// The cage opened, the sprites by it turn friendly and run, each down
/// one of eleven escape routes out of the camp, some a moment after the
/// others; at a route's end a sprite is saved and goes. Six saved, the
/// count ends well and so does the quest's event: Kindal stops, the
/// player is credited, and she says her thanks (Thistle Tea's
/// `KindalMoonweaver.escape_steps` and routes).
#[tokio::test]
#[ignore = "extended"]
async fn six_sprites_saved_free_all_creatures() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, kindal) = freedom(&realm, "Saviour").await;
    let sprites = open_the_pen(&map, me, kindal).await;
    assert!(sprites.len() >= 6, "sprites by the cage: {}", sprites.len());
    let s2 = sprites.clone();
    let at_once = map
        .query(move |m| {
            s2.iter()
                .filter_map(|g| escape_route(m, *g))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    assert!(
        at_once.iter().all(|v| *v == 1),
        "only the first route at once: {at_once:?}"
    );
    hurry_all(&map, 3.0).await;
    let s2 = sprites.clone();
    let (routes, freed) = map
        .query(move |m| {
            (
                s2.iter().map(|g| escape_route(m, *g)).collect::<Vec<_>>(),
                s2.iter().all(|g| {
                    faction(m, *g) == 10 && event_ai::scripted(m, *g).is_some_and(|s| s.run)
                }),
            )
        })
        .await
        .unwrap();
    assert!(
        routes.iter().all(Option::is_some),
        "each down an escape route: {routes:?}"
    );
    assert!(freed, "friendly, and running");
    // Six saved, one at a time.
    for (n, s) in sprites.iter().copied().take(6).enumerate() {
        // The player (and Kindal, who keeps by them) in sight of it.
        // Quick on its wings, for where the navmesh takes it round.
        map.query(move |m| {
            let at = bring(m, me, s, 1);
            scripts::place(m, kindal, off(at, 4.0, 0.0), 0.0);
            if let Some(c) = m.creature_mut(s) {
                c.speeds.walk = 40.0;
                c.speeds.run = 40.0;
            }
        })
        .await;
        assert!(
            until(&map, 3_000, move |m| gone(m, s)).await,
            "a sprite saved goes: {:?}",
            map.query(move |m| m.creature(s).map(|c| (
                c.movement_type,
                c.waypoint,
                c.last_waypoint,
                c.position,
                format!("{:?}", c.motion)
                    .chars()
                    .take(400)
                    .collect::<String>(),
                creature_motion_points(m, s),
                escape_route(m, s),
                c.alive(),
            )))
            .await
        );
        let saved = map
            .query(move |m| map_events::data(m, SPRITE, 0))
            .await
            .flatten();
        if n < 5 {
            assert_eq!(saved, Some(n as u32 + 1), "counted");
        }
    }
    assert!(
        until(&map, 3_000, move |m| explored(m, me, FREEDOM)
            && !map_events::active(m, FREEDOM)
            && !map_events::active(m, SPRITE))
        .await,
        "six saved: done"
    );
    assert!(hears(&mut c, &realm, kindal, 4080).await, "her thanks");
    assert!(
        map.query(move |m| event_ai::scripted(m, kindal).is_none_or(|s| s.follow.is_none()))
            .await
            .unwrap(),
        "she stops"
    );
}

/// A sprite that dies is counted lost, its corpse gone ten seconds on; a
/// sixth lost fails the quest in Kindal's own words, not the timeout's,
/// and she goes thirty seconds on.
#[tokio::test]
#[ignore = "extended"]
async fn a_sixth_sprite_lost_fails_freedom_for_all_creatures() {
    let Some(realm) = realm() else {
        return;
    };
    let lost_words = line(&realm, 4081);
    let timeout_words = line(&realm, 5285);
    let (mut c, me, map, kindal) = freedom(&realm, "Loser").await;
    let sprites = open_the_pen(&map, me, kindal).await;
    hurry_all(&map, 3.0).await;
    c.clear();
    for (n, s) in sprites.iter().copied().take(6).enumerate() {
        map.query(move |m| kill(m, me, s)).await;
        if n == 0 {
            assert_eq!(
                map.query(move |m| map_events::data(m, SPRITE, 1))
                    .await
                    .flatten(),
                Some(1),
                "counted lost"
            );
            hurry_all(&map, 9.0).await;
            assert!(
                map.query(move |m| !gone(m, s)).await.unwrap(),
                "the corpse lies"
            );
            hurry_all(&map, 1.0).await;
            assert!(
                until(&map, 2_000, move |m| gone(m, s)).await,
                "and goes ten seconds on"
            );
        }
    }
    assert!(
        until(&map, 3_000, move |m| failed(m, me, FREEDOM)
            && !map_events::active(m, FREEDOM))
        .await,
        "six lost: failed"
    );
    let said: Vec<String> = lines_by(&mut c, Duration::from_millis(800))
        .await
        .into_iter()
        .filter(|(g, _)| *g == kindal)
        .map(|(_, l)| l)
        .collect();
    assert!(says(&said, &lost_words), "her own words: {said:?}");
    assert!(!says(&said, &timeout_words), "not the timeout's");
    hurry_all(&map, 30.0).await;
    assert!(
        until(&map, 2_000, move |m| gone(m, kindal)).await,
        "she goes thirty seconds on"
    );
}

/// Run out of time, Freedom for All Creatures fails in the timeout's
/// words, the sprites' count ending with nothing said for it, and Kindal
/// is put back as she was.
#[tokio::test]
#[ignore = "extended"]
async fn out_of_time_freedom_for_all_creatures_fails_in_the_timeouts_words() {
    let Some(realm) = realm() else {
        return;
    };
    let lost_words = line(&realm, 4081);
    let timeout_words = line(&realm, 5285);
    let (mut c, me, map, kindal) = freedom(&realm, "Laggard").await;
    c.clear();
    map.query(move |m| {
        let now = m.now;
        if let Some(e) = m.map_events.active.get_mut(&FREEDOM) {
            e.expires = now;
        }
        m.map_events.next_check = None;
        map_events::update(m);
    })
    .await;
    assert!(
        until(&map, 3_000, move |m| failed(m, me, FREEDOM)
            && !map_events::active(m, SPRITE))
        .await,
        "out of time: failed, the count over"
    );
    let said: Vec<String> = lines_by(&mut c, Duration::from_millis(800))
        .await
        .into_iter()
        .map(|(_, l)| l)
        .collect();
    assert!(says(&said, &timeout_words), "the timeout's words: {said:?}");
    assert!(!says(&said, &lost_words), "nothing for the count");
    assert!(
        until(&map, 3_000, move |m| m
            .creature(kindal)
            .is_none_or(|c| c.alive() && faction(m, kindal) != 231))
        .await,
        "Kindal put back as she was"
    );
}
