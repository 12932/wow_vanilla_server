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
    // Attacked, she names her foe.
    c.clear();
    let foe = map
        .query(move |m| {
            let at = m.position_of(kindal).unwrap();
            let foe = dummy(m, off(at, 2.0, 0.0));
            crate::game::map::ai::attacked(m, kindal, foe);
            let entry = m.creature(foe).unwrap().entry;
            m.realm.data.creature_template(entry).unwrap().name.clone()
        })
        .await
        .unwrap();
    let cries: Vec<String> = (4122..=4125)
        .map(|t| line(&realm, t).replace("$n", &foe))
        .collect();
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

const ROWE: u32 = 17804;
const WINDSOR: u32 = 12580;
const RENDEZVOUS: u32 = 6402;
const MASQUERADE: u32 = 6403;
const GOSSIP: u32 = 0x1;
const QUESTGIVER: u32 = 0x2;

/// The menu `c` is shown by `npc` at its hello: its text, and its options'
/// ids and words.
pub(super) async fn menu_of(c: &mut Client, npc: Guid) -> (u32, Vec<(u32, String)>) {
    c.clear();
    hello(c, npc).await;
    c.wait_for("the menu", |m| match m {
        wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g)
            if g.guid == npc =>
        {
            Some((
                g.title_text_id,
                g.gossips
                    .iter()
                    .map(|o| (o.id, o.message.clone()))
                    .collect(),
            ))
        }
        _ => None,
    })
    .await
}

/// `me`'s quests as `log` (each complete or not) and `rewarded` have them.
fn quests_as(m: &mut Map, me: Guid, log: &[(u32, bool)], rewarded: &[u32]) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&me).unwrap();
    p.journal.log.clear();
    p.journal.rewarded.clear();
    p.journal.rewarded.extend(rewarded.iter().copied());
    for (quest, complete) in log {
        p.journal.log.push(crate::game::store::Logged {
            quest: *quest,
            // The masquerade's one objective: its event.
            explored: *complete,
            ..Default::default()
        });
    }
    crate::game::map::quests::refresh_log(p, &data);
}

/// Squire Rowe's menu: for a stranger his greeting alone; for a player
/// fresh from the rendezvous (or back on the masquerade) while Windsor is
/// away, his word for Windsor and the option to send it; for one done
/// with both, his last. Sent, the rendezvous's event starts, Rowe runs to
/// the road and kneels, lights a flare and runs back as Windsor rides in
/// from the gates, mounted and offering nothing; Rowe, at his post again,
/// gives his menu back (Windsor is up, he says so) and speaks. Windsor,
/// at the gates' end of the road, gets down, turns to the player who
/// called him, offers the masquerade and greets them; with no one taking
/// it for five minutes he leaves (Thistle Tea's `stormwind_rendezvous`).
#[tokio::test]
#[ignore = "extended"]
async fn squire_rowe_signals_windsor_who_rides_in_and_waits_for_the_masquerade() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, rowe) = hero_by(&realm, "Signaller", ROWE).await;
    // A watcher midway along the road, so that Windsor rides in sight.
    let (mut w, watcher, _) = hero(&realm, "Watcher").await;
    let midway = Vector3d {
        x: -9099.0,
        y: 407.0,
        z: 92.0,
    };
    relocate(&map, &mut w, midway).await;
    let sent = "Let Marshal Windsor know that I am ready.";
    for (log, rewarded, text, offered) in [
        (&[][..], &[][..], 9063, false),
        (&[(RENDEZVOUS, true)][..], &[][..], 9065, true),
        (&[(MASQUERADE, false)][..], &[RENDEZVOUS][..], 9065, true),
        (&[][..], &[RENDEZVOUS, MASQUERADE][..], 9066, false),
    ] {
        map.query(move |m| quests_as(m, me, log, rewarded)).await;
        let (shown, options) = menu_of(&mut c, rowe).await;
        assert_eq!(shown, text, "{log:?} {rewarded:?}");
        assert_eq!(
            options.iter().any(|(_, t)| t == sent),
            offered,
            "{log:?} {rewarded:?}: {options:?}"
        );
    }
    // Sent.
    map.query(move |m| quests_as(m, me, &[(RENDEZVOUS, true)], &[]))
        .await;
    let (_, options) = menu_of(&mut c, rowe).await;
    let id = options.iter().find(|(_, t)| t == sent).unwrap().0;
    map.query(move |m| {
        if let Some(c) = m.creature_mut(rowe) {
            c.speeds.walk = 20.0;
            c.speeds.run = 20.0;
        }
    })
    .await;
    choose(&c, rowe, id).await;
    assert!(
        until(&map, 2_000, move |m| map_events::active(m, RENDEZVOUS)
            && npc_flags(m, rowe) & GOSSIP == 0)
        .await,
        "the rendezvous under way, Rowe's menu gone"
    );
    // The signaller by Rowe's post.
    let post = Vector3d {
        x: -9042.23,
        y: 434.241,
        z: 93.2955,
    };
    map.query(move |m| {
        stand(m, watcher, midway);
        stand(m, me, off(post, -3.0, 3.0));
    })
    .await;
    let road = Vector3d {
        x: -9084.88,
        y: 419.23,
        z: 92.42,
    };
    assert!(
        until(&map, 5_000, move |m| m
            .position_of(rowe)
            .is_some_and(|at| dist(at, road) < 2.0))
        .await,
        "Rowe runs to the road"
    );
    assert!(
        until(&map, 3_000, move |m| m
            .scripts
            .runs
            .iter()
            .any(|r| r.source == rowe && r.next != usize::MAX))
        .await,
        "and kneels there"
    );
    hurry_all(&map, 5.0).await;
    assert!(
        until(&map, 2_000, |m| m
            .gameobjects
            .iter()
            .any(|(_, o)| o.entry == 181987))
        .await,
        "the flare"
    );
    hurry_all(&map, 1.5).await;
    let windsor = until_some(&map, 2_000, |m| first(m, WINDSOR))
        .await
        .expect("Windsor rides in");
    let (mount, flags) = map
        .query(move |m| {
            if let Some(c) = m.creature_mut(windsor) {
                c.speeds.walk = 40.0;
                c.speeds.run = 40.0;
            }
            (
                m.unit_fields(windsor).map_or(0, |f| {
                    f.get_u32(crate::game::fields::UNIT_FIELD_MOUNTDISPLAYID)
                }),
                npc_flags(m, windsor),
            )
        })
        .await
        .unwrap();
    assert_eq!(mount, 2410, "mounted");
    assert_eq!(flags & QUESTGIVER, 0, "offering nothing yet");
    assert!(
        until(&map, 5_000, move |m| npc_flags(m, rowe) & GOSSIP != 0).await,
        "Rowe at his post, his menu back"
    );
    assert!(hears(&mut c, &realm, rowe, 14389).await, "and speaks");
    let (shown, options) = menu_of(&mut c, rowe).await;
    assert_eq!(shown, 9064, "Windsor is up");
    assert!(options.is_empty(), "nothing to send");
    // Windsor at the road's end.
    let end = Vector3d {
        x: -9050.406,
        y: 443.9748,
        z: 93.05646,
    };
    assert!(
        until(&map, 8_000, move |m| {
            hasten(m, windsor);
            m.position_of(windsor).is_some_and(|at| dist(at, end) < 2.0)
        })
        .await,
        "Windsor rides to the road's end: {:?}",
        map.query(move |m| m.creature(windsor).map(|c| (
            c.position,
            format!("{:?}", c.motion)
                .chars()
                .take(300)
                .collect::<String>()
        )))
        .await
    );
    sleep_ms(300).await;
    c.clear();
    hurry_all(&map, 12.0).await;
    assert!(
        until(&map, 2_000, move |m| npc_flags(m, windsor) & QUESTGIVER
            != 0
            && event_ai::phase(m, windsor) == 1)
        .await,
        "he offers the masquerade"
    );
    assert!(
        hears(&mut c, &realm, windsor, 8090).await,
        "and greets the player who called him"
    );
    assert_eq!(
        map.query(move |m| m.unit_fields(windsor).map_or(1, |f| f
            .get_u32(crate::game::fields::UNIT_FIELD_MOUNTDISPLAYID)))
            .await,
        Some(0),
        "down from his horse"
    );
    // Five minutes and no one takes it.
    fire(&map, windsor, 1258002).await;
    assert!(
        until(&map, 3_000, move |m| gone(m, windsor)
            && !map_events::active(m, RENDEZVOUS))
        .await,
        "he leaves"
    );
}

/// Wait, a little at a time, up to `ms`, for `what` to find something.
pub(super) async fn until_some<T: Send + 'static>(
    map: &MapHandle,
    ms: u64,
    what: impl Fn(&mut Map) -> Option<T> + Send + Clone + 'static,
) -> Option<T> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
    loop {
        let w = what.clone();
        if let Some(v) = map.query(move |m| w(m)).await.flatten() {
            return Some(v);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        sleep_ms(25).await;
    }
}

const BOLVAR: u32 = 1748;
const PRESTOR: u32 = 1749;
const ONYXIA: u32 = 12756;
const ELITE: u32 = 12739;
const GUARD: u32 = 68;

/// The Great Masquerade: Windsor, up from the rendezvous for the player,
/// takes it; its escort's event starts, the bridge is lined with six
/// guards and Prestor's voice is heard, and he sets off down his
/// procession. At the keep's doorstep he waits for the player's word,
/// and at it walks to the throne room, where the tablets unmask Prestor
/// and her royal guards turn. Once no elite guard stands, Bolvar mourns,
/// the player is credited, both events end well, and Windsor dies
/// (Thistle Tea's `stormwind_rendezvous`, vmangos's
/// `npc_reginald_windsor`).
#[tokio::test]
#[ignore = "extended"]
async fn windsor_unmasks_prestor_at_the_players_word() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, rowe) = hero_by(&realm, "Unmasker", ROWE).await;
    let end = Vector3d {
        x: -9050.406,
        y: 443.9748,
        z: 93.05646,
    };
    // Windsor up for the player, as Rowe's signal leaves him.
    let windsor = map
        .query(move |m| {
            quests_as(m, me, &[], &[RENDEZVOUS]);
            map_events::start(m, RENDEZVOUS, rowe, me, Duration::from_secs(2400));
            stand(m, me, off(end, 2.0, 0.0));
            let w = summon(m, WINDSOR, end, 0.66);
            event_ai::set_phase(m, w, 1);
            if let Some(f) = m.unit_fields_mut(w) {
                f.set_u32(crate::game::fields::UNIT_NPC_FLAGS, QUESTGIVER);
            }
            w
        })
        .await
        .unwrap();
    c.sees(windsor).await;
    c.clear();
    assert!(
        takes(&mut c, &map, windsor, MASQUERADE).await,
        "the masquerade's escort under way"
    );
    assert!(
        hears(&mut c, &realm, windsor, 8107).await,
        "Windsor takes it"
    );
    assert_eq!(
        map.query(move |m| (event_ai::phase(m, windsor), npc_flags(m, windsor)))
            .await,
        Some((2, 0)),
        "nothing more to offer"
    );
    hurry_all(&map, 10.0).await;
    let (guards, voice, route) = map
        .query(move |m| {
            let guards = m
                .creatures
                .iter()
                .filter(|(_, c)| c.entry == GUARD && c.spawn_guid.is_none() && c.alive())
                .count();
            let voice = m
                .creatures
                .iter()
                .find(|(_, c)| c.entry == PRESTOR && c.spawn_guid.is_none())
                .map(|(_, c)| c.fields.get_u32(crate::game::fields::UNIT_FIELD_DISPLAYID));
            let route = event_ai::scripted(m, windsor)
                .and_then(|s| s.route.as_ref())
                .map(|r| r.points.len());
            (guards, voice, route)
        })
        .await
        .unwrap();
    assert_eq!(guards, 6, "the bridge lined with guards");
    assert_eq!(voice, Some(11686), "Prestor's voice");
    assert_eq!(route, Some(18), "Windsor on his procession");
    // At the bridge he talks Marcus down, for a minute and a half.
    c.clear();
    map.query(move |m| bring(m, me, windsor, 0)).await;
    assert!(
        until(&map, 3_000, move |m| reached(m, windsor, 0)).await,
        "at the bridge"
    );
    let waits = map
        .query(move |m| match m.creature(windsor).map(|c| &c.motion) {
            Some(crate::game::map::creature::Motion::Idle { until: Some(t) }) => {
                Some(t.saturating_duration_since(m.now).as_secs_f32())
            }
            _ => None,
        })
        .await
        .flatten();
    assert!(
        waits.is_some_and(|w| (86.0..=87.7).contains(&w)),
        "he stays there: {waits:?}"
    );
    let marcus = map
        .query(move |m| {
            let at = m.position_of(windsor).unwrap();
            scripts::creatures_near(m, at, 466, 150.0, true)
                .into_iter()
                .next()
        })
        .await
        .flatten()
        .expect("Marcus at the bridge");
    hurry_all(&map, 1.0).await;
    assert!(
        hears(&mut c, &realm, marcus, 8121).await,
        "Marcus stops him"
    );
    hurry_all(&map, 5.0).await;
    assert!(
        hears(&mut c, &realm, windsor, 8123).await,
        "and Windsor has words for him"
    );
    // At the keep's doorstep, he waits for the word.
    c.clear();
    map.query(move |m| bring(m, me, windsor, 16)).await;
    assert!(
        until(&map, 3_000, move |m| reached(m, windsor, 16)).await,
        "at the keep"
    );
    hurry_all(&map, 1.0).await;
    assert!(hears(&mut c, &realm, windsor, 8207).await, "he asks for it");
    assert!(
        until(&map, 2_000, move |m| npc_flags(m, windsor) & GOSSIP != 0).await,
        "his menu"
    );
    let (text, options) = menu_of(&mut c, windsor).await;
    assert_eq!(text, 5633);
    let (id, _) = options
        .iter()
        .find(|(_, t)| t.starts_with("I am ready, as are my forces."))
        .cloned()
        .expect("the word");
    c.clear();
    choose(&c, windsor, id).await;
    assert!(hears(&mut c, &realm, windsor, 8208).await, "on to the keep");
    assert!(
        until(&map, 2_000, move |m| npc_flags(m, windsor) == 0
            && event_ai::scripted(m, windsor)
                .and_then(|s| s.route.as_ref())
                .is_some_and(|r| r.points.len() == 5))
        .await,
        "his walk to the throne room"
    );
    // The throne room.
    c.clear();
    // His walk's last point: reached, it ends his route an update on.
    let throne = map.query(move |m| bring(m, me, windsor, 4)).await.unwrap();
    assert!(
        until(&map, 3_000, move |m| came_to(m, windsor, 4, throne)).await,
        "in the throne room"
    );
    let prestor = map
        .query(move |m| {
            let at = m.position_of(windsor).unwrap();
            scripts::creatures_near(m, at, PRESTOR, 60.0, true)
                .into_iter()
                .next()
        })
        .await
        .flatten()
        .expect("Prestor at her throne");
    hurry_all(&map, 55.0).await;
    assert_eq!(
        cast_any_of(&mut c, windsor, 20358, Duration::from_secs(3)).await,
        Some(prestor),
        "the tablets at Prestor"
    );
    hurry_all(&map, 10.0).await;
    assert!(
        until(&map, 2_000, move |m| m.creature(prestor).map(|c| c.entry)
            == Some(ONYXIA))
        .await,
        "Prestor unmasked"
    );
    hurry_all(&map, 11.0).await;
    assert!(
        until(&map, 2_000, |m| !living(m, ELITE).is_empty()).await,
        "her royal guards turn"
    );
    hurry_all(&map, 28.0).await;
    assert!(
        until(&map, 2_000, move |m| event_ai::phase(m, windsor) == 3).await,
        "the fight's phase"
    );
    // No elite guard standing.
    c.clear();
    map.query(move |m| {
        for g in living(m, ELITE) {
            kill(m, me, g);
        }
    })
    .await;
    let bolvar = map
        .query(|m| first(m, BOLVAR))
        .await
        .flatten()
        .expect("Bolvar");
    assert!(
        until(&map, 8_000, move |m| event_ai::phase(m, windsor) == 4).await,
        "the guards down"
    );
    hurry_all(&map, 6.0).await;
    assert!(hears(&mut c, &realm, bolvar, 8249).await, "Bolvar mourns");
    assert!(
        until(&map, 2_000, move |m| explored(m, me, MASQUERADE)
            && stand_state(m, bolvar) == 8)
        .await,
        "kneeling; the masquerade credited"
    );
    hurry_all(&map, 8.0).await;
    assert!(
        until(&map, 3_000, move |m| !map_events::active(m, MASQUERADE)
            && !map_events::active(m, RENDEZVOUS)
            && m.creature(windsor).is_some_and(|c| !c.alive()))
        .await,
        "both events over well, and Windsor dies"
    );
    assert!(
        !map.query(move |m| failed(m, me, MASQUERADE)).await.unwrap(),
        "not failed"
    );
}

const ERIS: u32 = 14494;
const INJURED: u32 = 14484;
const PLAGUED: u32 = 14485;
const FOOTSOLDIER: u32 = 14486;
const ARCHER: u32 = 14489;
const BALANCE: u32 = 7622;
const LIGHT: u32 = 179693;
const SEETHING_PLAGUE: u32 = 23072;
const ARROW: u32 = 23073;
const DEATHS_DOOR: u32 = 23127;

/// The peasants a script brought here, the fallen too.
fn peasants(m: &Map) -> Vec<Guid> {
    m.creatures
        .iter()
        .filter(|(_, c)| [INJURED, PLAGUED].contains(&c.entry) && c.spawn_guid.is_none())
        .map(|(_, c)| c.guid)
        .collect()
}

/// The Balance of Light and Shadow taken from Eris, by a priest: the
/// client, the player, the map, Eris.
async fn the_trial(realm: &std::sync::Arc<Realm>, name: &str) -> (Client, Guid, MapHandle, Guid) {
    let (mut c, me, map, eris) = hero_as_by(realm, name, Class::Priest, ERIS).await;
    map.query(move |m| quests_as(m, me, &[], &[7621])).await;
    c.clear();
    assert!(takes(&mut c, &map, eris, BALANCE).await, "the trial taken");
    (c, me, map, eris)
}

/// The trial taken, its event opens, the light is lit at the sanctuary
/// and eight archers take their posts. Its waves of peasants flee the
/// burning village, scattered about it, ten seconds on and every eighty
/// seconds after, five of them (twelve, twelve, twelve, thirteen and
/// sixteen, some plagued); at a hundred seconds the Scourge's assault
/// joins, and only from then does Eris bless the field and send
/// footsoldiers (Thistle Tea's `eris_havenfire`).
#[tokio::test]
#[ignore = "extended"]
async fn the_trial_lights_the_sanctuary_and_its_waves_flee_the_village() {
    // The navmesh, which the waves scatter over.
    let Some(realm) = nav_realm() else {
        return;
    };
    let (mut c, me, map, eris) = the_trial(&realm, "Lightbearer").await;
    let (light, archers) = map
        .query(|m| {
            (
                m.gameobjects
                    .iter()
                    .find(|(_, o)| o.entry == LIGHT)
                    .map(|(_, o)| o.position),
                living(m, ARCHER),
            )
        })
        .await
        .unwrap();
    let light = light.expect("the sanctuary lit");
    assert!(
        dist(
            light,
            Vector3d {
                x: 3327.0,
                y: -2970.0,
                z: 160.0
            }
        ) < 2.0
    );
    assert_eq!(archers.len(), 8, "eight archers at their posts");
    // Eris sends no one before the assault.
    let before = map
        .query(move |m| {
            for _ in 0..10 {
                due(m, eris, 1449401);
            }
            living(m, FOOTSOLDIER).len()
        })
        .await
        .unwrap();
    assert_eq!(before, 0, "no footsoldiers before the assault");
    // The waves, by the trial's clock.
    let waves = map.clone();
    let wave = move |by: f32| {
        let map = waves.clone();
        async move {
            map.query(move |m| {
                let had = peasants(m);
                hurry_runs(m, None, Duration::from_secs_f32(by));
                let new: Vec<(u32, Vector3d)> = peasants(m)
                    .into_iter()
                    .filter(|g| !had.contains(g))
                    .filter_map(|g| m.creature(g).map(|c| (c.entry, c.position)))
                    .collect();
                // The fallen and the saved go, so the next wave counts.
                for g in had {
                    scripts::despawn(m, g);
                }
                new
            })
            .await
            .unwrap()
        }
    };
    let village = Vector3d {
        x: 3353.8,
        y: -3042.16,
        z: 163.68,
    };
    for (n, (by, size)) in [(10.0, 12), (80.0, 12), (80.0, 12), (80.0, 13), (80.0, 16)]
        .into_iter()
        .enumerate()
    {
        if n == 2 {
            // The assault joins at a hundred seconds.
            hurry_all(&map, 10.0).await;
            assert!(
                until(&map, 2_000, move |m| event_ai::phase(m, eris) == 1).await,
                "the Scourge's assault"
            );
        }
        let came = wave(if n == 2 { 70.0 } else { by }).await;
        assert_eq!(came.len(), size, "wave {}: {came:?}", n + 1);
        let plagued = came.iter().filter(|(e, _)| *e == PLAGUED).count();
        assert!((1..=4).contains(&plagued), "some plagued: {plagued}");
        let spots: std::collections::BTreeSet<(i32, i32)> = came
            .iter()
            .map(|(_, at)| (at.x.round() as i32, at.y.round() as i32))
            .collect();
        assert!(spots.len() > 1, "scattered");
        assert!(
            came.iter().all(|(_, at)| dist(*at, village) < 60.0),
            "at the burning village"
        );
    }
    // Once it has, Eris blesses the field and sends footsoldiers.
    c.clear();
    let sent = map
        .query(move |m| {
            for _ in 0..20 {
                due(m, eris, 1449401);
                hurry_runs(m, Some(eris), Duration::from_secs(5));
                if !living(m, FOOTSOLDIER).is_empty() {
                    break;
                }
            }
            due(m, eris, 1449402);
            !living(m, FOOTSOLDIER).is_empty()
        })
        .await
        .unwrap();
    assert!(sent, "footsoldiers once the assault starts");
    assert!(
        cast_any_of(&mut c, eris, 23108, Duration::from_secs(2))
            .await
            .is_some(),
        "her blessing"
    );
    let _ = me;
}

/// A peasant of the trial stays passive and joins its event; a plagued
/// one carries the Seething Plague (an injured one doesn't). Each walks to
/// the road and on to the light, where it is saved and goes. The archers
/// loose at random peasants; an arrow hits a peasant fifty-seven harder
/// than its spell, and now and then opens Death's Door. Each fallen
/// peasant is hung on the next of the light's death posts.
#[tokio::test]
#[ignore = "extended"]
async fn peasants_flee_for_the_light_under_the_archers() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, _eris) = the_trial(&realm, "Shepherd").await;
    hurry_all(&map, 10.0).await;
    let (plagued, injured) = map
        .query(|m| {
            let all = peasants(m);
            let of = |e: u32| {
                all.iter()
                    .copied()
                    .find(|g| m.creature(*g).is_some_and(|c| c.entry == e))
                    .unwrap()
            };
            (of(PLAGUED), of(INJURED))
        })
        .await
        .unwrap();
    // Their spawned events, at the update after.
    assert!(
        until(&map, 2_000, move |m| has(m, plagued, SEETHING_PLAGUE)).await,
        "the plagued carry the plague"
    );
    let state = map
        .query(move |m| {
            let s = event_ai::scripted(m, plagued).unwrap();
            (
                s.reaction,
                map_events::targets(m, BALANCE).contains(&plagued),
                has(m, plagued, SEETHING_PLAGUE),
                has(m, injured, SEETHING_PLAGUE),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        state.0,
        Some(crate::game::map::assistance::Reaction::Passive),
        "passive"
    );
    assert!(state.1, "of the trial");
    assert!(state.2 && !state.3, "the plagued carry the plague");
    // The archers loose at the peasants.
    let archer = map.query(|m| first(m, ARCHER)).await.flatten().unwrap();
    let all = map.query(|m| peasants(m)).await.unwrap();
    c.clear();
    map.query(move |m| due(m, archer, 1448902)).await;
    let at = cast_any_of(&mut c, archer, ARROW, Duration::from_secs(3)).await;
    assert!(
        at.is_some_and(|g| all.contains(&g)),
        "an arrow at a peasant: {at:?}"
    );
    // The archers stood down, an arrow at the injured peasant.
    map.query(move |m| {
        for g in living(m, ARCHER).into_iter().filter(|g| *g != archer) {
            scripts::despawn(m, g);
        }
        tough(m, injured);
    })
    .await;
    c.clear();
    let before = map
        .query(move |m| {
            let hp = health(m, injured).0;
            crate::game::map::spells::cast_triggered(m, archer, ARROW, Some(injured));
            hp
        })
        .await
        .unwrap();
    let dealt = c
        .gets(Duration::from_secs(3), move |m| match m {
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_SPELLNONMELEEDAMAGELOG(d)
                if d.spell == ARROW && d.target == injured =>
            {
                Some(d.damage)
            }
            _ => None,
        })
        .await
        .expect("the arrow hits");
    assert!(
        until(&map, 2_000, move |m| before - health(m, injured).0
            == dealt + 57)
        .await,
        "fifty-seven more than the arrow's {dealt}: {:?}",
        map.query(move |m| before - health(m, injured).0).await
    );
    // Death's Door, now and then.
    let doors = map
        .query(move |m| {
            let mut n = 0;
            for _ in 0..60 {
                crate::game::map::auras::remove_spell(m, injured, DEATHS_DOOR, None);
                event_ai::spell_hit(m, injured, archer, ARROW, 1);
                if has(m, injured, DEATHS_DOOR) {
                    n += 1;
                }
            }
            n
        })
        .await
        .unwrap();
    assert!(
        (1..=20).contains(&doors),
        "Death's Door, now and then: {doors} of 60"
    );
    // To the road and the light.
    map.query(move |m| scripts::despawn(m, archer)).await;
    let road = Vector3d {
        x: 3353.8,
        y: -3042.16,
        z: 163.68,
    };
    let light = Vector3d {
        x: 3327.0,
        y: -2970.0,
        z: 161.0,
    };
    for (to, what) in [(road, "the road"), (light, "the light")] {
        map.query(move |m| {
            scripts::place(m, plagued, off(to, 1.5, 0.0), 0.0);
            stand(m, me, off(to, 4.0, 0.0));
            if let Some(c) = m.creature_mut(plagued) {
                c.speeds.walk = 20.0;
                c.speeds.run = 20.0;
            }
        })
        .await;
        sleep_ms(200).await;
        // A move toward it again, from beside it.
        map.query(move |m| {
            let step = crate::game::data::scripts::ScriptStep {
                command: 3,
                datalong: [3, 0, 3, 2],
                dataint: [if what == "the road" { 1 } else { 2 }, 0, 0, 0],
                x: to.x,
                y: to.y,
                z: to.z,
                ..Default::default()
            };
            scripts::execute(m, &step, plagued, plagued);
        })
        .await;
        if what == "the road" {
            assert!(
                until(&map, 3_000, move |m| event_ai::scripted(m, plagued)
                    .and_then(|s| s.scripted_move)
                    .is_some_and(|mv| mv.point == Some(2)))
                .await,
                "from the road, on to the light"
            );
        }
    }
    assert!(
        until(&map, 3_000, move |m| gone(m, plagued)
            && map_events::data(m, BALANCE, 0) == Some(1))
        .await,
        "saved at the light"
    );
    // The fallen, each on the next post.
    let (posts, hung) = map
        .query(move |m| {
            let summoned = |m: &Map| -> Vec<(Guid, Vector3d)> {
                m.gameobjects
                    .iter()
                    .filter(|(_, o)| o.spawn_guid.is_none())
                    .map(|(_, o)| (o.guid, o.position))
                    .collect()
            };
            let had = summoned(m);
            let mut out = Vec::new();
            for g in peasants(m).into_iter().take(2) {
                kill(m, me, g);
                out.push(map_events::data(m, BALANCE, 1));
            }
            let hung: Vec<Vector3d> = summoned(m)
                .into_iter()
                .filter(|o| !had.contains(o))
                .map(|(_, at)| at)
                .collect();
            (out, hung)
        })
        .await
        .unwrap();
    assert_eq!(posts, [Some(1), Some(2)], "counted fallen");
    assert_eq!(hung.len(), 2, "two posts hung");
    assert!(dist(hung[0], hung[1]) > 1.0, "each on its own post");
    assert!(
        hung.iter().any(|p| dist(
            *p,
            Vector3d {
                x: 3355.48,
                y: -3010.68,
                z: 175.212
            }
        ) < 1.0),
        "the first post first"
    );
}
