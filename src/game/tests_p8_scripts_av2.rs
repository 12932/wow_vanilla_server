//! Round 8's Alterac Valley assaults, as Thistle Tea scripts them
//! (`AlteracValleyCavalry`, `AlteracValleyGround`, `AlteracValleyAir`),
//! through the protocol: the cavalry's riders waiting on their commander,
//! or riding alone with none; both getting down to fight and up again
//! after; a fallen commander's charge to be had again once he is back;
//! the infantry of every tier waiting on their own commander; the
//! commanders' and the troops' last homes; and a fallen wing commander
//! back in her prison.

use std::time::Duration;

use wow_world_messages::vanilla::*;

use super::tests_p8_scripts_av::{all_of, pair, put, spawn_at};
use super::tests_pvp2::{arrive, choose, creature, npc, options, turn_in, valley};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness;
use crate::game::map::battleground::av_quests::{self, FleetPhase, March};
use crate::game::map::battleground::{av_routes, av_scripts};
use crate::game::map::{Map, MapHandle, ai, event_ai};

/// A route of `n` points from `from`, `step` apart along x (or, `along_y`,
/// y).
fn route(from: (f32, f32, f32), n: usize, step: f32, along_y: bool) -> Vec<(f32, f32, f32)> {
    (0..n)
        .map(|i| {
            let d = i as f32 * step;
            if along_y {
                (from.0, from.1 + d, from.2)
            } else {
                (from.0 + d, from.1, from.2)
            }
        })
        .collect()
}

/// Whether `what` comes to hold on the map, given a busy machine's while.
async fn until(map: &MapHandle, what: impl Fn(&Map) -> bool + Send + Clone + 'static) -> bool {
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        let w = what.clone();
        if map.query(move |m| w(m)).await.unwrap_or(false) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn mount_of(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g)
        .map_or(0, |f| f.get_u32(UNIT_FIELD_MOUNTDISPLAYID))
}

fn extras(m: &Map) -> &av_quests::Extras {
    &m.battleground.as_ref().unwrap().av.as_ref().unwrap().extras
}

/// Whether `g` runs, its own road under way, and goes in about ten
/// minutes.
fn alone(m: &Map, g: Guid) -> (u32, bool, bool, bool) {
    let s = event_ai::scripted(m, g);
    let goes = s.and_then(|s| s.despawn_at).is_some_and(|t| {
        let left = t.saturating_duration_since(m.now).as_secs();
        (590..=600).contains(&left)
    });
    (
        event_ai::phase(m, g),
        av_routes::walking(m, g),
        s.is_some_and(|s| s.run),
        goes,
    )
}

const RAM_COMMANDER: u32 = 13577;
const RAM_RIDER: u32 = 13576;
const WOLF_COMMANDER: u32 = 13441;
const WOLF_RIDER: u32 = 13440;
const RAMS: u32 = 2786;
const WOLVES: u32 = 1166;

/// The cavalry: the Stormpike's commander at `at`, his riders' road, and
/// the Frostwolf's commander.
fn cavalry(at: (f32, f32, f32)) -> GameData {
    let mut d = valley();
    for (entry, name) in [
        (RAM_COMMANDER, "Stormpike Ram Rider Commander"),
        (RAM_RIDER, "Stormpike Ram Rider"),
        (WOLF_COMMANDER, "Frostwolf Wolf Rider Commander"),
        (WOLF_RIDER, "Frostwolf Wolf Rider"),
    ] {
        npc(&mut d, entry, name, 35);
    }
    spawn_at(&mut d, 170901, RAM_COMMANDER, at);
    spawn_at(&mut d, 170902, WOLF_COMMANDER, (-1240.0, -620.0, 54.0));
    d.bg_routes
        .insert(RAM_COMMANDER, route(at, 93, 0.05, false));
    d.bg_routes
        .insert(RAM_RIDER, route((620.0, -50.0, 45.0), 93, 0.05, false));
    av_scripts::install(&mut d);
    d
}

/// The rams' charge ordered by Alice, honored, the stables full: the
/// commander.
async fn charge(map: &MapHandle, a: &mut harness::Client, alice: Guid) -> Guid {
    map.query(move |m| crate::game::map::reputation::modify(m, alice, 730, 9000, false))
        .await;
    let commander = map
        .query(|m| creature(m, RAM_COMMANDER))
        .await
        .flatten()
        .unwrap();
    turn_in(map, alice, 7026, 25).await;
    turn_in(map, alice, 7027, 25).await;
    let at = map
        .query(move |m| m.position_of(commander))
        .await
        .flatten()
        .unwrap();
    put(map, a, at.x + 2.0, at.y, at.z).await;
    assert_eq!(options(a, commander).await, 1, "the charge");
    choose(a, commander, 0).await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while map.query(|m| all_of(m, RAM_RIDER).len()).await != Some(8) {
        assert!(tokio::time::Instant::now() < deadline, "eight riders");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    commander
}

/// Riders waiting on their commander for his rally hear of his death: each
/// rides out alone at once, down its own road, for ten minutes (Thistle
/// Tea's riders join his group at their birth; here they wait on him, and
/// join it at the rally).
#[tokio::test]
async fn riders_waiting_for_the_rally_hear_of_their_commanders_death() {
    let realm = harness::realm_with(cavalry((612.0, -30.0, 45.0)));
    let (mut a, alice, _b, _, map) = pair(&realm).await;
    let commander = charge(&map, &mut a, alice).await;
    let waiting = map
        .query(|m| {
            all_of(m, RAM_RIDER)
                .into_iter()
                .map(|r| (event_ai::phase(m, r), av_routes::walking(m, r)))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    assert_eq!(waiting, vec![(0, false); 8], "waiting on him");
    map.query(move |m| ai::die(m, commander, alice)).await;
    let gone = map
        .query(|m| {
            all_of(m, RAM_RIDER)
                .into_iter()
                .map(|r| alone(m, r))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    assert_eq!(gone, vec![(1, true, true, true); 8]);
}

/// Riders born with no living commander within a hundred yards ride out
/// alone at once, mounted, down their own road, for ten minutes.
#[tokio::test]
async fn riders_born_without_their_commander_ride_alone() {
    let realm = harness::realm_with(cavalry((800.0, -30.0, 45.0)));
    let (mut a, alice, _b, _, map) = pair(&realm).await;
    charge(&map, &mut a, alice).await;
    let riders = map
        .query(|m| {
            all_of(m, RAM_RIDER)
                .into_iter()
                .map(|r| (alone(m, r), mount_of(m, r)))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    assert_eq!(riders, vec![((1, true, true, true), RAMS); 8]);
}

/// The cavalry's commanders and riders get down from their mounts to fight
/// and mount again as they give it up, running (the commanders mounted
/// from their spawn).
#[tokio::test]
async fn commanders_and_riders_dismount_in_combat_and_remount_on_evade() {
    let realm = harness::realm_with(cavalry((800.0, -30.0, 45.0)));
    let (mut a, alice, _b, grom, map) = pair(&realm).await;
    let (rams, wolves) = map
        .query(|m| {
            (
                creature(m, RAM_COMMANDER).unwrap(),
                creature(m, WOLF_COMMANDER).unwrap(),
            )
        })
        .await
        .unwrap();
    harness::an_update(&map).await;
    charge(&map, &mut a, alice).await;
    let rider = map.query(|m| all_of(m, RAM_RIDER)[0]).await.unwrap();
    for (g, mount, foe) in [
        (rams, RAMS, grom),
        (wolves, WOLVES, alice),
        (rider, RAMS, grom),
    ] {
        let before = map.query(move |m| mount_of(m, g)).await.unwrap();
        assert_eq!(before, mount, "{g} mounted");
        map.query(move |m| ai::attacked(m, g, foe)).await;
        let fighting = map.query(move |m| mount_of(m, g)).await.unwrap();
        assert_eq!(fighting, 0, "{g} down to fight");
        let after = map
            .query(move |m| {
                ai::enter_evade(m, g);
                (
                    mount_of(m, g),
                    event_ai::scripted(m, g).is_some_and(|s| s.run),
                )
            })
            .await
            .unwrap();
        assert_eq!(after, (mount, true), "{g} up again, running");
    }
}

/// The cavalry's commander fallen, the valley knows its charge is over;
/// back, he is mounted where he waits, and his charge is to be had again
/// once the stables are full.
#[tokio::test]
async fn a_fallen_cavalry_commander_back_can_lead_the_charge_again() {
    let realm = harness::realm_with(cavalry((612.0, -30.0, 45.0)));
    let (mut a, alice, _b, _, map) = pair(&realm).await;
    let commander = charge(&map, &mut a, alice).await;
    map.query(move |m| ai::die(m, commander, alice)).await;
    let phase = map.query(|m| extras(m).cavalry[0].phase).await.unwrap();
    assert_eq!(phase, March::Dead);
    map.query(move |m| {
        ai::respawn_now(m, commander);
        av_scripts::sweep_soon(m);
    })
    .await;
    harness::an_update(&map).await;
    harness::an_update(&map).await;
    let (back, phase) = map
        .query(|m| {
            let g = all_of(m, RAM_COMMANDER)[0];
            (
                (mount_of(m, g), event_ai::phase(m, g)),
                extras(m).cavalry[0].phase,
            )
        })
        .await
        .unwrap();
    assert_eq!(back, (RAMS, 0), "mounted, waiting");
    assert_eq!(phase, March::Ready);
    turn_in(&map, alice, 7026, 25).await;
    turn_in(&map, alice, 7027, 25).await;
    let commander = map.query(|m| all_of(m, RAM_COMMANDER)[0]).await.unwrap();
    assert_eq!(options(&mut a, commander).await, 1, "the charge again");
}

const MARSHAL: u32 = 13446;
const WARMASTER: u32 = 13449;
const QUARTERMASTER: u32 = 12096;

/// The ground assault: both commanders, every tier of their troops and
/// their roads (each troop's away north of where it assembles), and a
/// quartermaster.
fn ground() -> GameData {
    let mut d = valley();
    npc(&mut d, MARSHAL, "Field Marshal Teravaine", 35);
    npc(&mut d, WARMASTER, "Warmaster Garrick", 35);
    npc(&mut d, QUARTERMASTER, "Stormpike Quartermaster", 35);
    for entry in 13524..=13531 {
        npc(&mut d, entry, &format!("Troop {entry}"), 35);
    }
    spawn_at(&mut d, 171001, QUARTERMASTER, (-246.0, -429.0, 20.0));
    d.bg_routes
        .insert(MARSHAL, route((-243.75, -431.32, 20.0), 52, 1.0, false));
    d.bg_routes
        .insert(WARMASTER, route((-470.38, -51.84, 41.0), 57, 1.0, false));
    for entry in 13524..=13527 {
        d.bg_routes
            .insert(entry, route((-240.0, -440.0, 20.0), 53, -1.0, true));
    }
    for entry in 13528..=13531 {
        d.bg_routes
            .insert(entry, route((-472.0, -48.0, 41.0), 58, 1.0, true));
    }
    av_scripts::install(&mut d);
    d
}

/// The commander of side `s` and ten troops of `tier` assemble: the
/// commander.
fn assemble(m: &mut Map, s: usize, tier: u8) -> Guid {
    let entry = [MARSHAL, WARMASTER][s];
    let before = all_of(m, entry);
    let qm = creature(m, QUARTERMASTER).unwrap();
    av_scripts::ground_assembles(m, qm, s, tier);
    all_of(m, entry)
        .into_iter()
        .find(|g| !before.contains(g))
        .expect("the commander")
}

/// The troops of every tier, of both sides, wait on their own commander
/// until his rally: one fallen before it, each of his ten goes down its
/// own road at once, at a run, the others waiting still (Thistle Tea's
/// infantry join their commander's group at their birth, and with none
/// about take their own road; here they are summoned waiting on him, so
/// none is born without him).
#[tokio::test]
async fn every_infantry_tier_waits_on_its_own_commander_and_goes_alone_when_he_falls() {
    let realm = harness::realm_with(ground());
    let (_a, alice, _b, _, map) = pair(&realm).await;
    for s in 0..2 {
        for tier in 0..4u8 {
            let troops = [13524, 13528][s] + u32::from(tier);
            let (waiting, alone_now) = map
                .query(move |m| {
                    let commander = assemble(m, s, tier);
                    let waiting = all_of(m, troops)
                        .into_iter()
                        .filter(|t| !av_routes::walking(m, *t))
                        .count();
                    ai::die(m, commander, alice);
                    let alone_now: Vec<(u32, bool, bool)> = all_of(m, troops)
                        .into_iter()
                        .map(|t| {
                            let (phase, walking, run, _) = alone(m, t);
                            (phase, walking, run)
                        })
                        .collect();
                    (waiting, alone_now)
                })
                .await
                .unwrap();
            assert_eq!(waiting, 10, "side {s} tier {tier}: ten waiting on him");
            assert_eq!(
                alone_now,
                vec![(1, true, true); 10],
                "side {s} tier {tier}: each alone"
            );
        }
    }
}

/// The commanders stop at their last point, at the enemy's base (48 and
/// 53), their home there, their roads' ends unwalked; and every tier's
/// troops, alone, stop at theirs (49 and 54), each its own home.
#[tokio::test]
async fn commanders_stop_at_the_enemy_base_and_every_tier_has_its_own_last_home() {
    let realm = harness::realm_with(ground());
    let (_a, alice, _b, _, map) = pair(&realm).await;
    // Troops alone, of a tier each side.
    map.query(move |m| {
        for s in 0..2 {
            let commander = assemble(m, s, 2);
            ai::die(m, commander, alice);
        }
    })
    .await;
    arrive(&map).await;
    let troops = map
        .query(|m| {
            [13526, 13530].map(|entry| {
                let last = m.realm.data.bg_routes[&entry][if entry < 13528 { 49 } else { 54 }];
                all_of(m, entry).into_iter().all(|t| {
                    let c = m.creature(t).unwrap();
                    let there =
                        |v: Vector3d| (v.x - last.0).abs() < 0.01 && (v.y - last.1).abs() < 0.01;
                    there(c.position) && there(c.home) && !av_routes::walking(m, t)
                })
            })
        })
        .await
        .unwrap();
    assert_eq!(troops, [true, true], "each troop home at its last point");
    // The commanders march, alone.
    let commanders = map
        .query(|m| {
            [0, 1].map(|s| {
                let g = assemble(m, s, 0);
                for t in all_of(m, [13524, 13528][s]) {
                    crate::game::map::scripts::despawn(m, t);
                }
                av_scripts::ground_marches(m, g, s);
                g
            })
        })
        .await
        .unwrap();
    arrive(&map).await;
    let marched = map
        .query(move |m| {
            commanders.map(|g| {
                let c = m.creature(g).unwrap();
                let entry = c.entry;
                let last = m.realm.data.bg_routes[&entry][if entry == MARSHAL { 48 } else { 53 }];
                let there =
                    |v: Vector3d| (v.x - last.0).abs() < 0.01 && (v.y - last.1).abs() < 0.01;
                (there(c.position), there(c.home), av_routes::walking(m, g))
            })
        })
        .await
        .unwrap();
    assert_eq!(
        marched,
        [(true, true, false); 2],
        "at the enemy's base, home"
    );
}

const SLIDORE: u32 = 13438;
const ICHMAN: u32 = 13437;

/// A wing commander freed and home, fallen, is back in her prison: where
/// she was held, her home there, unattackable and spawned (0x101), giving
/// quests, whole, seated if she sat (Ichman, not Slidore), her phase and
/// her fleet's a prisoner's, to be freed again.
#[tokio::test]
async fn a_fallen_wing_commander_respawns_a_prisoner_where_she_was_held() {
    let mut d = valley();
    npc(&mut d, SLIDORE, "Wing Commander Slidore", 35);
    spawn_at(&mut d, 171101, SLIDORE, (4.0, 2.0, 0.0));
    d.bg_routes
        .insert(SLIDORE, route((4.0, 2.0, 0.0), 67, 0.05, false));
    for entry in [SLIDORE, ICHMAN] {
        let t = d.creature_templates.get_mut(&entry).unwrap();
        t.npc_flags = 3;
        t.unit_flags = 0x101;
    }
    if let Some(s) = d
        .creature_spawns
        .get_mut(&30)
        .and_then(|s| s.iter_mut().find(|s| s.entries == [ICHMAN]))
    {
        (s.respawn_min, s.respawn_max) = (600, 600);
    }
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (mut a, alice, _b, _, map) = pair(&realm).await;
    put(&map, &mut a, 3.0, 1.0, 0.0).await;
    harness::an_update(&map).await;
    let held = map
        .query(|m| {
            [SLIDORE, ICHMAN].map(|e| {
                let c = m.creature(creature(m, e).unwrap()).unwrap();
                c.position
            })
        })
        .await
        .unwrap();
    for entry in [SLIDORE, ICHMAN] {
        let g = map
            .query(move |m| creature(m, entry))
            .await
            .flatten()
            .unwrap();
        a.send(CMSG_GOSSIP_HELLO { guid: g }).await;
        assert!(
            until(&map, move |m| av_routes::walking(m, g)).await,
            "{entry} freed, on her way home"
        );
    }
    arrive(&map).await;
    let home = map
        .query(move |m| {
            [SLIDORE, ICHMAN].map(|e| {
                let g = creature(m, e).unwrap();
                let phase = event_ai::phase(m, g);
                ai::die(m, g, alice);
                ai::respawn_now(m, g);
                phase
            })
        })
        .await
        .unwrap();
    assert_eq!(home, [2, 2], "home, then fallen and back");
    harness::an_update(&map).await;
    harness::an_update(&map).await;
    let back = map
        .query(move |m| {
            [SLIDORE, ICHMAN].map(|e| {
                let g = all_of(m, e)[0];
                let c = m.creature(g).unwrap();
                let f = &c.fields;
                let i = [13179, 13180, 13181, 13438, 13439, 13437]
                    .iter()
                    .position(|w| *w == e)
                    .unwrap();
                (
                    (c.position, c.home),
                    f.get_u32(UNIT_FIELD_FLAGS) & 0x1101,
                    f.get_u32(UNIT_NPC_FLAGS) & 2,
                    f.get_u32(UNIT_FIELD_HEALTH) == f.get_u32(UNIT_FIELD_MAXHEALTH),
                    f.get_byte(UNIT_FIELD_BYTES_1, 0),
                    event_ai::phase(m, g),
                    extras(m).air[i].phase,
                )
            })
        })
        .await
        .unwrap();
    for (k, (e, sits)) in [(SLIDORE, 0), (ICHMAN, 1)].into_iter().enumerate() {
        let (at, flags, quests, whole, stand, phase, fleet) = back[k];
        assert_eq!(at, (held[k], held[k]), "{e}: in her prison, her home");
        assert_eq!(
            (flags, quests, whole, stand, phase, fleet),
            (0x101, 2, true, sits, 0, FleetPhase::Prisoner),
            "{e}"
        );
    }
    // To be freed again.
    let g = map.query(|m| all_of(m, ICHMAN)[0]).await.unwrap();
    a.send(CMSG_GOSSIP_HELLO { guid: g }).await;
    assert!(
        until(&map, move |m| av_routes::walking(m, g)).await,
        "freed again"
    );
}
