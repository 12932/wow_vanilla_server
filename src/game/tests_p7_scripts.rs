//! Round 7's creature script tests, ported from Thistle Tea's
//! `test/game/core/ai/creature_script/` (docs/parity/matrix.csv): the
//! Gizelton Caravan, driven on the real world as a player's client sees it.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use super::tests_p4_scripts_dungeons::concealed;
use super::tests_p6_scripts::*;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::creature::Motion;
use crate::game::map::{Map, MapHandle, creature_groups, creature_motion, event_ai, map_events};

/// `g` stands at `at` now, as their client would have moved them there:
/// the grid and what they see follow.
pub(super) fn stand(m: &mut Map, g: Guid, at: Vector3d) {
    let Some(p) = m.players.get_mut(&g) else {
        return;
    };
    let (old, new) = (p.cell, crate::game::map::grid::cell_of(at.x, at.y));
    let anchor = p.vis_anchor;
    p.movement.position = at;
    p.cell = new;
    m.grid.relocate(g, old, new);
    crate::game::map::visibility::moved(m, g, anchor, at);
}

/// `g`, walking its route, put half a yard short of its point `point`
/// and bound for it, its group with it and `me` beside: it arrives at
/// the next update or so. Where the point is.
pub(super) fn bring(m: &mut Map, me: Guid, g: Guid, point: u32) -> Vector3d {
    let points = creature_motion::waypoints(m, g).expect("a route");
    let i = points
        .iter()
        .position(|p| p.point == point)
        .unwrap_or_else(|| panic!("point {point}"));
    let wp = points[i];
    let at = Vector3d {
        x: wp.x,
        y: wp.y,
        z: wp.z,
    };
    let short = off(at, 0.5, 0.0);
    let mut all = creature_groups::members_of(m, g);
    if !all.contains(&g) {
        all.push(g);
    }
    for x in all {
        crate::game::map::scripts::place(m, x, short, 0.0);
    }
    let now = m.now;
    if let Some(c) = m.creature_mut(g) {
        c.waypoint = i;
        c.last_waypoint = None;
        c.motion = Motion::Idle { until: Some(now) };
        if let Some(s) = c.scripted.as_deref_mut() {
            s.hold = None;
        }
    }
    stand(m, me, off(at, 3.0, 0.0));
    at
}

/// Whether `g` has reached its point `point` (the last it came to).
pub(super) fn reached(m: &Map, g: Guid, point: u32) -> bool {
    let Some(c) = m.creature(g) else {
        return false;
    };
    let Some(points) = creature_motion::waypoints(m, g) else {
        return false;
    };
    c.last_waypoint
        .and_then(|i| points.get(i))
        .is_some_and(|p| p.point == point)
}

/// Whether `g` is held on its path by a script.
pub(super) fn held(m: &Map, g: Guid) -> bool {
    event_ai::scripted(m, g).is_some_and(|s| s.hold.is_some())
}

const CORK: u32 = 11625;
const RIGGER: u32 = 11626;
const KODO: u32 = 11564;
const BOTTOM: u32 = 5943;
const TOP: u32 = 5821;
const ESCORTEE: u32 = 495;
const QUESTGIVER: u32 = 0x2;

/// What of the caravan is here: entry, alive, spawned, where.
fn caravan_left(m: &Map) -> Vec<(u32, bool, Option<u32>, Vector3d)> {
    m.creatures
        .iter()
        .filter(|(_, c)| [CORK, RIGGER, KODO].contains(&c.entry))
        .map(|(_, c)| (c.entry, c.alive(), c.spawn_guid, c.position))
        .collect()
}

/// The caravan's members of every kind here, the dead too: those a
/// script brought (the spawns are put back, at their time, as any are).
fn of_caravan(m: &Map) -> Vec<Guid> {
    m.creatures
        .iter()
        .filter(|(_, c)| [CORK, RIGGER, KODO].contains(&c.entry) && c.spawn_guid.is_none())
        .map(|(_, c)| c.guid)
        .collect()
}

/// The caravan there was gone, and a fresh Cork at his spawn, his spawned
/// events run: he, and his npc flags as he came.
fn fresh_cork(m: &mut Map, home: Vector3d) -> (Guid, u32) {
    let mut gone = of_caravan(m);
    for e in [CORK, RIGGER, KODO] {
        gone.extend(living(m, e));
    }
    for g in gone {
        crate::game::map::scripts::despawn(m, g);
    }
    let cork = summon(m, CORK, home, 4.48);
    event_ai::update_despawns(m);
    (cork, npc_flags(m, cork))
}

/// A caravan formed up and on its way: Cork.
async fn formed(map: &MapHandle, home: Vector3d) -> Guid {
    let cork = map
        .query(move |m| {
            let (cork, _) = fresh_cork(m, home);
            hurry_runs(m, Some(cork), Duration::from_millis(2_000));
            cork
        })
        .await
        .unwrap();
    assert!(
        until(map, 3_000, move |m| living(m, KODO).len() == 2
            && living(m, RIGGER).len() == 1)
        .await,
        "the caravan formed"
    );
    cork
}

/// `c` takes `quest` from `giver`, beside them: whether its event is
/// under way then.
async fn takes(c: &mut Client, map: &MapHandle, giver: Guid, quest: u32) -> bool {
    let me = c.guid;
    map.query(move |m| {
        let at = m.position_of(giver).unwrap();
        stand(m, me, off(at, 2.0, 0.0));
    })
    .await;
    c.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: giver,
        quest_id: quest,
    })
    .await;
    until(map, 3_000, move |m| {
        m.players[&me].journal.has(quest) && map_events::active(m, quest)
    })
    .await
}

/// Two seconds after spawning, Cork calls up two kodos and Rigger, who
/// fall in behind him, clears his own flags and Rigger's offer, and sets
/// off on his path. When a kodo, Rigger or Cork himself dies, the caravan
/// disbands: both legs' events end, a leg under way failing its quest,
/// and every one of them goes (Thistle Tea's `gizelton_caravan`,
/// vmangos's `npc_cork_gizelton`).
#[tokio::test]
#[ignore = "extended"]
async fn corks_caravan_forms_up_and_disbands_at_a_death() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, _) = hero_by(&realm, "Caravaneer", CORK).await;
    let (_, home) = spawn_of(&realm, CORK);
    for dies in [KODO, RIGGER, CORK] {
        let (cork, flags, at_once) = map
            .query(move |m| {
                let (cork, flags) = fresh_cork(m, home);
                let at_once = (living(m, KODO).len(), living(m, RIGGER).len());
                (cork, flags, at_once)
            })
            .await
            .unwrap();
        assert_ne!(flags, 0, "Cork as he comes");
        assert_eq!(at_once, (0, 0), "nothing at once");
        map.query(move |m| hurry_runs(m, Some(cork), Duration::from_millis(2_000)))
            .await;
        assert!(
            until(&map, 3_000, move |m| living(m, KODO).len() == 2
                && living(m, RIGGER).len() == 1)
            .await,
            "two kodos and Rigger, two seconds on"
        );
        let (members, flags, rigger_flags, route) = map
            .query(move |m| {
                let rigger = first(m, RIGGER).unwrap();
                let mut members = creature_groups::members_of(m, cork);
                members.sort_by_key(|g| g.guid());
                let mut want = vec![cork, rigger];
                want.extend(living(m, KODO));
                want.sort_by_key(|g| g.guid());
                (
                    members == want,
                    npc_flags(m, cork),
                    npc_flags(m, rigger),
                    event_ai::scripted(m, cork).is_some_and(|s| s.route.is_some()),
                )
            })
            .await
            .unwrap();
        assert!(members, "they fall in behind Cork");
        assert_eq!(flags, 0, "Cork's flags cleared");
        assert_eq!(rigger_flags & QUESTGIVER, 0, "Rigger offers nothing yet");
        assert!(route, "Cork sets off on his path");
        // A leg under way, the first time.
        if dies == KODO {
            let rigger = map.query(|m| first(m, RIGGER)).await.flatten().unwrap();
            map.query(move |m| {
                if let Some(f) = m.unit_fields_mut(rigger) {
                    let flags = f.get_u32(UNIT_NPC_FLAGS);
                    f.set_u32(UNIT_NPC_FLAGS, flags | QUESTGIVER);
                }
            })
            .await;
            assert!(
                takes(&mut c, &map, rigger, BOTTOM).await,
                "the bottom leg taken"
            );
        }
        let victim = map.query(move |m| first(m, dies)).await.flatten().unwrap();
        map.query(move |m| kill(m, me, victim)).await;
        assert!(
            until(&map, 3_000, |m| of_caravan(m).is_empty()).await,
            "the caravan disbands as a {dies} dies: {:?}",
            map.query(|m| caravan_left(m)).await
        );
        let (bottom, top, failed_it) = map
            .query(move |m| {
                (
                    map_events::active(m, BOTTOM),
                    map_events::active(m, TOP),
                    failed(m, me, BOTTOM),
                )
            })
            .await
            .unwrap();
        assert!(!bottom && !top, "both legs' events over");
        if dies == KODO {
            assert!(failed_it, "the leg under way failed");
        }
    }
}

/// The bottom leg: at its first stop Cork holds for a quarter of an hour
/// while Rigger offers Gizelton Caravan, yelling it to the zone every
/// three minutes while he does. Taken, the leg's event starts, Rigger
/// offers it no more (nor yells), and ten seconds on the caravan moves
/// off, its escortees' faction on. At the leg's end Rigger thanks the
/// player, who is credited, the event ends well, and the caravan is at
/// peace again, running on (Thistle Tea's `gizelton_caravan` route and
/// quest start steps).
#[tokio::test]
#[ignore = "extended"]
async fn the_bottom_leg_is_offered_taken_and_finished() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, _) = hero_by(&realm, "Bodyguard", CORK).await;
    let (_, home) = spawn_of(&realm, CORK);
    let cork = formed(&map, home).await;
    let (rigger, peaceful) = map
        .query(move |m| (first(m, RIGGER).unwrap(), faction(m, cork)))
        .await
        .unwrap();
    c.clear();
    map.query(move |m| bring(m, me, cork, 14)).await;
    assert!(
        until(&map, 3_000, move |m| reached(m, cork, 14)
            && npc_flags(m, rigger) & QUESTGIVER != 0)
        .await,
        "Rigger offers the leg"
    );
    assert!(
        map.query(move |m| held(m, cork)).await.unwrap(),
        "Cork holds"
    );
    assert!(hears(&mut c, &realm, rigger, 7475).await, "Rigger yells it");
    // Three minutes on, again.
    hurry_all(&map, 180.0).await;
    assert!(
        hears(&mut c, &realm, rigger, 7475).await,
        "and again, three minutes on"
    );
    // Taken.
    assert!(
        takes(&mut c, &map, rigger, BOTTOM).await,
        "the leg's event under way"
    );
    assert_eq!(
        map.query(move |m| npc_flags(m, rigger) & QUESTGIVER).await,
        Some(0),
        "offered no more"
    );
    assert!(
        map.query(move |m| held(m, cork)).await.unwrap(),
        "not at once"
    );
    hurry_all(&map, 10.0).await;
    assert!(
        until(&map, 3_000, move |m| !held(m, cork)
            && faction(m, cork) == ESCORTEE
            && faction(m, rigger) == ESCORTEE)
        .await,
        "ten seconds on, the caravan moves off as escortees"
    );
    c.clear();
    hurry_all(&map, 180.0).await;
    assert_eq!(
        heard(&mut c, rigger, Duration::from_millis(500)).await,
        None,
        "nor yells it"
    );
    // The leg's end.
    c.clear();
    map.query(move |m| bring(m, me, cork, 42)).await;
    assert!(
        hears(&mut c, &realm, rigger, 7333).await,
        "Rigger thanks the player"
    );
    assert!(
        until(&map, 3_000, move |m| explored(m, me, BOTTOM)
            && !map_events::active(m, BOTTOM)
            && faction(m, cork) == peaceful)
        .await,
        "credited, the event over, at peace"
    );
    assert!(
        map.query(move |m| event_ai::scripted(m, cork).is_some_and(|s| s.run))
            .await
            .unwrap(),
        "running on"
    );
}

/// The top leg: at its stop Cork offers Bodyguard for Hire himself,
/// yelling it to the zone. Taken, the caravan moves off ten seconds on;
/// ambushed on the way while its event runs, it holds until the
/// ambushers are dead, Cork calling them out, and goes on then.
#[tokio::test]
#[ignore = "extended"]
async fn the_top_leg_is_corks_and_its_ambush_holds_the_caravan() {
    const AMBUSHERS: [u32; 2] = [12976, 12977];
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, _) = hero_by(&realm, "Hireling", CORK).await;
    let (_, home) = spawn_of(&realm, CORK);
    let cork = formed(&map, home).await;
    c.clear();
    map.query(move |m| bring(m, me, cork, 164)).await;
    assert!(
        until(&map, 3_000, move |m| reached(m, cork, 164)
            && npc_flags(m, cork) & QUESTGIVER != 0)
        .await,
        "Cork offers the top leg himself"
    );
    assert!(hears(&mut c, &realm, cork, 7474).await, "and yells it");
    assert!(takes(&mut c, &map, cork, TOP).await, "taken");
    hurry_all(&map, 10.0).await;
    assert!(
        until(&map, 3_000, move |m| !held(m, cork)
            && faction(m, cork) == ESCORTEE)
        .await,
        "the caravan moves off"
    );
    // The ambush.
    c.clear();
    map.query(move |m| bring(m, me, cork, 173)).await;
    assert!(
        until(&map, 3_000, move |m| reached(m, cork, 173)
            && AMBUSHERS
                .iter()
                .map(|e| living(m, *e).len())
                .sum::<usize>()
                == 4)
        .await,
        "four ambushers"
    );
    assert!(
        hears(&mut c, &realm, cork, 7310).await,
        "Cork calls them out"
    );
    sleep_ms(500).await;
    assert!(
        map.query(move |m| held(m, cork)).await.unwrap(),
        "held while they live"
    );
    map.query(move |m| {
        for e in AMBUSHERS {
            for g in living(m, e) {
                kill(m, me, g);
            }
        }
    })
    .await;
    assert!(
        until(&map, 3_000, move |m| !held(m, cork)).await,
        "and on once they are dead"
    );
}

/// The caravan's camps: at each it walks in, holds for ten minutes and
/// shows its vendor, who is hidden again as it leaves, with a word from
/// Cork, and from Rigger at the second; at its path's end the caravan
/// goes.
#[tokio::test]
#[ignore = "extended"]
async fn the_caravan_camps_with_its_vendor_and_goes_at_its_paths_end() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, _) = hero_by(&realm, "Camper", CORK).await;
    let (_, home) = spawn_of(&realm, CORK);
    let cork = formed(&map, home).await;
    let rigger = map.query(|m| first(m, RIGGER)).await.flatten().unwrap();
    for (point, vendor, speaker, text) in [(141, 12245, cork, 7505), (279, 12246, rigger, 7506)] {
        let v = map
            .query(move |m| first(m, vendor))
            .await
            .flatten()
            .expect("the vendor");
        assert!(
            map.query(move |m| concealed(m, v)).await.unwrap(),
            "{vendor} hidden"
        );
        map.query(move |m| {
            if let Some(s) = event_ai::scripted_mut(m, cork) {
                s.run = true;
            }
            bring(m, me, cork, point)
        })
        .await;
        assert!(
            until(&map, 3_000, move |m| reached(m, cork, point)
                && held(m, cork)
                && !concealed(m, v))
            .await,
            "at {point}, the caravan camps and {vendor} trades"
        );
        c.sees(v).await;
        assert!(
            !map.query(move |m| event_ai::scripted(m, cork).is_some_and(|s| s.run))
                .await
                .unwrap(),
            "walking in"
        );
        // Ten minutes, then on.
        let left = map
            .query(move |m| {
                let now = m.now;
                event_ai::scripted(m, cork)
                    .and_then(|s| s.hold.as_ref())
                    .map(|h| h.until.saturating_duration_since(now).as_secs_f32())
            })
            .await
            .flatten();
        assert!(
            left.is_some_and(|l| (590.0..=600.0).contains(&l)),
            "for ten minutes: {left:?}"
        );
        c.clear();
        map.query(move |m| {
            let now = m.now;
            if let Some(h) = event_ai::scripted_mut(m, cork).and_then(|s| s.hold.as_mut()) {
                h.until = now;
            }
        })
        .await;
        assert!(
            hears(&mut c, &realm, speaker, text).await,
            "a word as it leaves {point}"
        );
        assert!(
            until(&map, 3_000, move |m| !held(m, cork) && concealed(m, v)).await,
            "{vendor} hidden again"
        );
    }
    map.query(move |m| bring(m, me, cork, 281)).await;
    assert!(
        until(&map, 3_000, |m| of_caravan(m).is_empty()).await,
        "the caravan goes at its path's end: {:?}",
        map.query(|m| caravan_left(m)).await
    );
}
