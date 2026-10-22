//! Bugs the user found in the game, the seventh batch: Officer Jaxon and
//! Thomas Miller standing in Stormwind with nothing in their hands, and
//! corpses sliding on along the ground.

use std::collections::BTreeMap;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::{CreatureAddon, CreatureSpawn, GameData, Waypoint};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::Client;
use crate::game::map::creature::{Creature, MOVEMENT_WAYPOINT};
use crate::game::tests_bugs6::fields;
use crate::game::tests_p3_combat::{HUMANS, SHIELD, START, SWORD, setup_with, template};

/// The fields of the last create block `c` was sent for `g`.
fn created(c: &mut Client, g: Guid) -> Option<BTreeMap<u16, u32>> {
    let mut out = None;
    for frame in c.captured() {
        let Ok(m) = ServerOpcodeMessage::read_unencrypted(&frame[..]) else {
            continue;
        };
        let objects = match &m {
            ServerOpcodeMessage::SMSG_UPDATE_OBJECT(u) => &u.objects,
            ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(u) => &u.objects,
            _ => continue,
        };
        for o in objects {
            if let Object_UpdateType::CreateObject { guid3, mask2, .. }
            | Object_UpdateType::CreateObject2 { guid3, mask2, .. } = &o.update_type
                && *guid3 == g
            {
                out = Some(fields(mask2));
            }
        }
    }
    out
}

/// The item displays a create shows in a creature's hands.
fn held(f: &BTreeMap<u16, u32>) -> [u32; 3] {
    [0, 1, 2].map(|i| {
        f.get(&(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + i))
            .copied()
            .unwrap_or(0)
    })
}

/// Whether `c` hears `g` set off somewhere within `within`.
async fn sets_off(c: &mut Client, g: Guid, within: Duration) -> bool {
    c.gets(within, |m| match m {
        ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv)
            if mv.guid == g && !matches!(mv.move_type, SMSG_MONSTER_MOVE_MonsterMoveType::Stop) =>
        {
            Some(())
        }
        _ => None,
    })
    .await
    .is_some()
}

const PATROL: u32 = 3_720;
const ARMED: u32 = 3_721;
const GEAR: u32 = 80;
const OWN_GEAR: u32 = 81;

/// A spawn of `entry` `east` and five yards north of the start.
fn spawn(guid: u32, entry: u32, east: f32, movement_type: u8) -> CreatureSpawn {
    CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + east,
        y: START.1 + 5.0,
        z: START.2,
        respawn_min: 60,
        respawn_max: 60,
        movement_type,
        ..Default::default()
    }
}

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    for entry in [PATROL, ARMED] {
        let mut t = template(entry, 7);
        t.faction = HUMANS;
        d.creature_templates.insert(entry, t);
    }
    d.creature_templates.get_mut(&ARMED).unwrap().equipment_id = GEAR;
    d.creature_equipment.insert(GEAR, [SWORD, 0, 0]);
    d.creature_equipment.insert(OWN_GEAR, [0, SHIELD, 0]);
    // A path of its entry's (`creature_movement_template`), none of its
    // spawn's.
    d.waypoints_by_entry.insert(
        PATROL,
        [(1, 0.0, 5.0), (2, 12.0, 5.0), (3, 12.0, 15.0)]
            .map(|(point, east, north)| Waypoint {
                point,
                x: START.0 + east,
                y: START.1 + north,
                z: START.2,
                ..Default::default()
            })
            .to_vec(),
    );
    d.creature_spawns.insert(
        0,
        vec![
            spawn(901, PATROL, 0.0, MOVEMENT_WAYPOINT),
            spawn(902, ARMED, 2.0, 0),
            spawn(903, ARMED, 4.0, 0),
            spawn(904, ARMED, 6.0, 0),
            spawn(905, ARMED, 8.0, 0),
        ],
    );
    for (spawn, equipment_id) in [(902, -1), (903, 0), (905, OWN_GEAR as i32)] {
        d.creature_addons.insert(
            spawn,
            CreatureAddon {
                equipment_id,
                mount_display_id: -1,
                sheath_state: 1,
                ..Default::default()
            },
        );
    }
    d
}

/// Officer Jaxon and Thomas Miller, on what they come with: a spawn set to
/// walk a path with none of its own walks its entry's
/// (`creature_movement_template`, mangos's `GetDefaultPath`), not stands;
/// an addon's weapons are its template's at -1 (its column's default),
/// none at 0 and its own entry's else (vmangos's `LoadCreatureAddon`,
/// Thistle Tea's `equipment_template_id`), as one with no addon holds its
/// template's.
#[tokio::test]
async fn a_spawn_walks_its_entrys_path_and_holds_its_templates_weapons() {
    let (realm, mut a, _ga, _map) = setup_with(world(), Class::Warrior).await;
    let patrol = guid::unit(PATROL, 901);
    let armed = [902, 903, 904, 905].map(|s| guid::unit(ARMED, s));
    a.sees(patrol).await;
    for g in armed {
        a.sees(g).await;
    }
    assert!(
        sets_off(&mut a, patrol, Duration::from_secs(3)).await,
        "it walks its entry's path"
    );
    let sword = realm.data.item(SWORD).unwrap().display_id;
    let shield = realm.data.item(SHIELD).unwrap().display_id;
    let hands = armed.map(|g| held(&created(&mut a, g).expect("its create")));
    assert_eq!(
        hands,
        [[sword, 0, 0], [0, 0, 0], [sword, 0, 0], [0, shield, 0]],
        "(the template's at -1, none at 0, no addon, the addon's own)"
    );
}

/// Officer Jaxon (14423) and Thomas Miller (3518) in Stormwind, as the user
/// saw them: each walks his entry's path (112 and 46 points of
/// `creature_movement_template`, his spawn of movement type 2 and none of
/// its own), Jaxon with his two swords (2178, 5284), Miller with his bread
/// (2197); their addons' equipment is -1, their templates'.
#[tokio::test]
#[ignore = "extended"]
async fn officer_jaxon_and_thomas_miller_patrol_with_their_swords_and_bread() {
    let Some(realm) = super::tests_p3_scripts::realm() else {
        return;
    };
    for (entry, name, items) in [
        (14_423, "Jaxona", [2178, 5284, 0]),
        (3518, "Millera", [2197, 0, 0]),
    ] {
        let (mut c, _me, map, g) = super::tests_p6_scripts::hero_by(&realm, name, entry).await;
        c.sees(g).await;
        let want = items.map(|i| realm.data.item(i).map_or(0, |t| t.display_id));
        assert!(want[0] != 0, "{entry}'s items are in the data");
        let hands = held(&created(&mut c, g).expect("its create"));
        assert_eq!(hands, want, "{entry} holds his things");
        let walking = map
            .query(move |m| m.creature(g).map(|c| c.movement_type))
            .await
            .flatten();
        assert_eq!(walking, Some(MOVEMENT_WAYPOINT), "{entry} on his path");
        assert!(
            sets_off(&mut c, g, Duration::from_secs(20)).await,
            "{entry} walks"
        );
    }
}

/// The class of the bug, over every spawn of the real world: each spawn of
/// movement type 2 with a path, its own or its entry's, is on it; and each
/// holds what vmangos gives it (its addon's equipment, or at -1 or with no
/// addon its template's). Before, the 185-odd spawns with only an entry's
/// path stood, and the 2,500-odd addons at -1 held nothing.
#[tokio::test]
#[ignore = "extended"]
async fn every_real_spawn_walks_its_path_and_holds_its_weapons() {
    let Some(realm) = super::tests_p3_scripts::realm() else {
        return;
    };
    let d = &realm.data;
    let mut rng = rand::rng();
    let (mut paths, mut by_entry, mut armed, mut by_template) = (0, 0, 0, 0);
    let mut idle = Vec::new();
    let mut empty = Vec::new();
    for s in d.creature_spawns.values().flatten() {
        let Some(c) = Creature::from_spawn(d, s, &mut rng) else {
            continue;
        };
        let own = d.waypoints.get(&s.guid).is_some_and(|p| !p.is_empty());
        let entry = d
            .waypoints_by_entry
            .get(&c.entry)
            .is_some_and(|p| !p.is_empty());
        if s.movement_type == MOVEMENT_WAYPOINT && (own || entry) {
            paths += 1;
            by_entry += usize::from(!own);
            if c.movement_type != MOVEMENT_WAYPOINT {
                idle.push((s.guid, c.entry));
            }
        }
        let template = d.creature_template(c.entry).map_or(0, |t| t.equipment_id);
        let gear = match d.creature_addons.get(&s.guid) {
            Some(a) if a.equipment_id >= 0 => a.equipment_id as u32,
            _ => template,
        };
        let want = d
            .creature_equipment
            .get(&gear)
            .copied()
            .unwrap_or_default()
            .map(|i| d.item(i).map_or(0, |t| t.display_id));
        if want != [0; 3] {
            armed += 1;
            by_template += usize::from(
                d.creature_addons
                    .get(&s.guid)
                    .is_some_and(|a| a.equipment_id < 0),
            );
            let hands = [0, 1, 2].map(|i| c.fields.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + i));
            if hands != want {
                empty.push((s.guid, c.entry));
            }
        }
    }
    eprintln!(
        "{paths} spawns on paths ({by_entry} their entry's); {armed} armed ({by_template} by an addon's -1)"
    );
    assert!(by_entry > 100, "the entries' paths: {by_entry}");
    assert!(by_template > 2_000, "the addons at -1: {by_template}");
    assert!(idle.is_empty(), "standing: {idle:?}");
    assert!(
        empty.is_empty(),
        "empty-handed: {:?}",
        &empty[..empty.len().min(20)]
    );
}

/// A movement of `flags` at `at`, facing east.
fn going(flags: MovementInfo_MovementFlags, at: Vector3d) -> MovementInfo {
    MovementInfo {
        flags,
        timestamp: 0,
        position: at,
        orientation: 0.0,
        fall_time: 0,
    }
}

fn start() -> Vector3d {
    Vector3d {
        x: START.0,
        y: START.1,
        z: START.2,
    }
}

/// Where `g` is told it stopped.
fn stop_of(g: Guid) -> impl FnMut(&ServerOpcodeMessage) -> Option<Vector3d> {
    move |m| match m {
        ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv)
            if mv.guid == g && matches!(mv.move_type, SMSG_MONSTER_MOVE_MonsterMoveType::Stop) =>
        {
            Some(mv.spline_point)
        }
        _ => None,
    }
}

/// Whether the movement of `g` in a create says it runs forward.
fn created_running(m: &ServerOpcodeMessage, g: Guid) -> Option<bool> {
    let objects = match m {
        ServerOpcodeMessage::SMSG_UPDATE_OBJECT(u) => &u.objects,
        ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(u) => &u.objects,
        _ => return None,
    };
    objects.iter().find_map(|o| match &o.update_type {
        Object_UpdateType::CreateObject {
            guid3, movement2, ..
        }
        | Object_UpdateType::CreateObject2 {
            guid3, movement2, ..
        } if *guid3 == g => match movement2.update_flag.get_living()? {
            MovementBlock_UpdateFlag_Living::Living { flags, .. } => Some(flags.get_forward()),
            MovementBlock_UpdateFlag_Living::HasPosition { .. } => None,
        },
        _ => None,
    })
}

/// The city raid's sliding corpses: Bea watches a creature on its run and
/// Alice running forward, and both die. The creature is stopped where its
/// run has come to (mangos's `MoveSplineInit::Stop` computes the spline's
/// position), not where the map's last update left it; Alice is stopped
/// at all, where she fell (mangos's `SetDeathState` and `StopMoving`, a
/// stop spline to those who see her; Thistle Tea's `MovementStopped`).
/// What her client says of its run after is told on with the run's flags
/// gone, and Cid, coming after, is told of her body standing: no client
/// runs a corpse on along the ground. Bea's capture, for the client check.
async fn deaths_on_the_run() -> (Client, Client) {
    let (realm, a, ga, map) = setup_with(world(), Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bea", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0, START.1, START.2).await;
    let runner = guid::unit(ARMED, 904);
    b.sees(runner).await;
    b.sees(ga).await;
    a.send(MSG_MOVE_START_FORWARD_Client {
        info: going(MovementInfo_MovementFlags::new_forward(), start()),
    })
    .await;
    b.wait_for("Alice's run", |m| match m {
        ServerOpcodeMessage::MSG_MOVE_START_FORWARD(s) if s.guid == ga => Some(()),
        _ => None,
    })
    .await;
    b.clear();
    // The creature sets off east; a third of a second on, between two of
    // the map's updates, both die.
    let along = map
        .query(move |m| {
            let from = m.creature(runner).unwrap().position;
            let to = Vector3d {
                x: from.x + 30.0,
                ..from
            };
            crate::game::map::scripts::move_to(m, runner, to, true);
            m.now += Duration::from_millis(300);
            let along = match &m.creature(runner).unwrap().motion {
                crate::game::map::creature::Motion::Moving(w) => w.position_at(m.now).0,
                _ => panic!("the creature runs"),
            };
            crate::game::map::combat::kill(m, runner, ga);
            crate::game::map::combat::kill(m, ga, runner);
            (from, along)
        })
        .await
        .unwrap();
    let (from, along) = along;
    assert!(
        crate::game::map::combat::dist(from, along) > 1.0,
        "it ran some way: {from:?} to {along:?}"
    );
    let at = b.wait_for("the creature's stop", stop_of(runner)).await;
    assert!(
        crate::game::map::combat::dist(at, along) < 0.1,
        "stopped where it had come to, {along:?}, not {at:?}"
    );
    let at = b.wait_for("Alice's stop", stop_of(ga)).await;
    assert!(
        crate::game::map::combat::dist(at, start()) < 0.1,
        "stopped where she fell: {at:?}"
    );
    // Her client, not yet told, runs on a little.
    let on = Vector3d {
        x: START.0 + 0.3,
        ..start()
    };
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: going(MovementInfo_MovementFlags::new_forward(), on),
    })
    .await;
    let told = b
        .wait_for("her heartbeat", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(s) if s.guid == ga => Some(s.info),
            _ => None,
        })
        .await;
    assert!(
        !told.flags.get_forward(),
        "told on without the run: {told:?}"
    );
    let mut c = Client::connect(&realm, "C");
    c.enter("Cid", Race::Human, Class::Warrior).await;
    c.heartbeat(START.0, START.1, START.2).await;
    let running = c.wait_for("Alice's body", |m| created_running(m, ga)).await;
    assert!(!running, "her body is created standing");
    (a, b)
}

#[tokio::test]
async fn a_watcher_sees_a_creature_and_a_player_stop_where_they_die_on_the_run() {
    deaths_on_the_run().await;
}

/// What [`deaths_on_the_run`] sent Bea (the creature's and Alice's stops,
/// her body's values) passes the 1.12 client.
#[tokio::test]
#[ignore = "extended"]
async fn deaths_on_the_run_pass_the_client() {
    use crate::game::tests_client_emu::{capture, checker, run_checker};
    let (check_py, wow_exe) = checker().expect(
        "the client-emulation check needs tools/client_check/WoW.exe and `uv`: see docs/TESTING.md",
    );
    let (mut a, mut b) = deaths_on_the_run().await;
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        a.captured();
        b.captured();
    }
    let (capture, counts) = capture(vec![&mut a, &mut b]);
    assert!(
        counts.get(&0x0DD).copied().unwrap_or(0) >= 2,
        "the stops were captured: {counts:?}"
    );
    run_checker(&check_py, &wow_exe, &capture);
}
