//! Gurubashi Arena's Booty Run: its schedule on the server's clock, and
//! Short John Mithril's walk down to the pit's floor with the chest, through
//! the protocol.

use std::time::Duration;

use chrono::{FixedOffset, NaiveDateTime, TimeZone};
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::config::EventsConfig;
use crate::game::data::scripts::ScriptStep;
use crate::game::data::{CreatureSpawn, CreatureTemplate, GameData, GameEvent, Waypoint};
use crate::game::events::{self, wall_clock_in};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::arena::{self, CENTRE};
use crate::game::map::creature::Motion;
use crate::game::map::{Map, MapHandle, MapKey};

fn naive(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
}

/// The Unix time of `local` in `tz`.
fn unix_at(tz: &FixedOffset, local: &str) -> i64 {
    tz.from_local_datetime(&naive(local))
        .single()
        .unwrap()
        .timestamp()
}

/// vmangos's row for the Booty Run (event 16).
fn booty_row() -> GameEvent {
    GameEvent {
        id: arena::BOOTY_RUN_EVENT,
        name: "Gurubashi Arena Booty Run".into(),
        start: naive("2020-12-31 23:00:00").and_utc().timestamp(),
        end: naive("2037-12-31 23:59:59").and_utc().timestamp(),
        occurence: 180,
        length: 120,
        hardcoded: false,
    }
}

fn row(id: u32, start: &str, occurence: i64, length: i64) -> GameEvent {
    GameEvent {
        id,
        name: format!("event {id}"),
        start: naive(start).and_utc().timestamp(),
        end: naive("2037-12-31 23:59:59").and_utc().timestamp(),
        occurence,
        length,
        hardcoded: false,
    }
}

fn with_events(rows: Vec<GameEvent>) -> GameData {
    let mut d = GameData::empty();
    for e in rows {
        d.events.insert(e.id, e);
    }
    d
}

/// Whether event `id` is on at `local` in `tz`, worked out as the sweep does.
fn on_at(d: &GameData, cfg: &EventsConfig, tz: &FixedOffset, local: &str, id: u32) -> bool {
    let wall = wall_clock_in(unix_at(tz, local), tz);
    events::active_with(d, wall, &Default::default(), cfg).contains(&id)
}

/// The Booty Run starts at 12:00 and 15:00 on the server's own clock, and
/// at no other hour between: whatever the server's zone, and whatever hour
/// vmangos's row (23:00, its UTC+1 midnight) starts it on.
#[test]
fn the_booty_run_starts_every_third_hour_from_local_midnight() {
    let d = with_events(vec![booty_row()]);
    let cfg = EventsConfig::default();
    for hours in [-7, 0, 1, 5, 9] {
        let tz = FixedOffset::east_opt(hours * 3600).unwrap();
        let on = |t: &str| on_at(&d, &cfg, &tz, t, arena::BOOTY_RUN_EVENT);
        // A start: on at the hour, and off the minute before.
        let starts = |h: u32| {
            on(&format!("2026-10-23 {h:02}:00:00"))
                && !on(&if h == 0 {
                    "2026-10-22 23:59:00".to_owned()
                } else {
                    format!("2026-10-23 {:02}:59:00", h - 1)
                })
        };
        let started: Vec<u32> = (0..24).filter(|h| starts(*h)).collect();
        assert_eq!(
            started,
            vec![0, 3, 6, 9, 12, 15, 18, 21],
            "the starts in UTC{hours:+}"
        );
        assert!(starts(12) && starts(15) && !starts(13), "UTC{hours:+}");
        // Two hours on, one off.
        assert!(on("2026-10-23 13:59:00") && !on("2026-10-23 14:00:00"));
        assert!(!on("2026-10-23 14:59:59") && on("2026-10-23 15:00:00"));
    }
    // The first hour and the interval are the config's.
    let cfg = EventsConfig {
        booty_run_first_hour: 1,
        booty_run_every_minutes: 240,
        ..Default::default()
    };
    let tz = FixedOffset::east_opt(0).unwrap();
    let on = |t: &str| on_at(&d, &cfg, &tz, t, arena::BOOTY_RUN_EVENT);
    assert!(on("2026-10-23 01:00:00") && on("2026-10-23 05:00:00"));
    assert!(!on("2026-10-23 00:30:00") && !on("2026-10-23 04:00:00"));
}

/// The wall clock is the local date and time: 12:00 in UTC+5 is 07:00 UTC.
#[test]
fn the_wall_clock_is_the_servers_local_time() {
    let tz = FixedOffset::east_opt(5 * 3600).unwrap();
    let unix = naive("2026-10-23 07:00:00").and_utc().timestamp();
    assert_eq!(
        wall_clock_in(unix, &tz),
        naive("2026-10-23 12:00:00").and_utc().timestamp()
    );
    let utc = FixedOffset::east_opt(0).unwrap();
    assert_eq!(wall_clock_in(unix, &utc), unix);
}

/// The other events keep the hours of their rows, now on the server's
/// clock as vmangos reads them: the Fishing Extravaganza's tournament on
/// Sunday at 14:00 for two hours, the night from 21:00 for twelve, the
/// Darkmoon Faire's calendar by the local date and the holidays' fireworks
/// by the local night.
#[test]
fn the_other_events_start_at_their_local_times() {
    let d = with_events(vec![
        booty_row(),
        row(15, "2017-01-08 14:00:00", 10080, 120),
        row(27, "2020-01-02 21:00:00", 1440, 720),
        row(34, "2020-12-31 06:00:00", 525600, 1445),
        GameEvent {
            id: events::FIREWORKS,
            hardcoded: true,
            ..Default::default()
        },
        GameEvent {
            id: events::DARKMOON_ELWYNN,
            hardcoded: true,
            ..Default::default()
        },
    ]);
    let cfg = EventsConfig::default();
    for hours in [-7, 0, 3, 9] {
        let tz = FixedOffset::east_opt(hours * 3600).unwrap();
        let on = |t: &str, id: u32| on_at(&d, &cfg, &tz, t, id);
        // 2026-10-25 is a Sunday.
        assert!(
            on("2026-10-25 14:00:00", 15),
            "the tournament, UTC{hours:+}"
        );
        assert!(on("2026-10-25 15:59:00", 15));
        assert!(!on("2026-10-25 13:59:00", 15) && !on("2026-10-25 16:00:00", 15));
        assert!(!on("2026-10-24 14:30:00", 15), "Saturday");
        assert!(on("2026-10-23 21:00:00", 27) && on("2026-10-24 08:59:00", 27));
        assert!(!on("2026-10-23 20:59:00", 27) && !on("2026-10-24 09:00:00", 27));
        // New Year's Eve's fireworks, the first ten minutes of a night hour.
        assert!(on("2021-12-31 23:05:00", events::FIREWORKS));
        assert!(!on("2021-12-31 12:05:00", events::FIREWORKS));
        // October 2026's Faire in Elwynn opens Monday the 5th.
        assert!(on("2026-10-05 00:30:00", events::DARKMOON_ELWYNN));
        assert!(!on("2026-10-04 23:30:00", events::DARKMOON_ELWYNN));
    }
}

const FLOOR: Vector3d = Vector3d {
    x: CENTRE.x - 1.2,
    y: CENTRE.y + 0.4,
    z: CENTRE.z,
};
/// His place in the stands, 16 yards north of the centre.
const HOME: Vector3d = Vector3d {
    x: CENTRE.x,
    y: CENTRE.y + 16.0,
    z: CENTRE.z,
};
const HOME_O: f32 = 4.41568;
const DOWN_SCRIPT: u32 = 1_450_816;
const HOME_SCRIPT: u32 = 1_450_833;
const CHEST_ENTRY: u32 = 179_697;

fn step(delay_ms: u32, command: u32, datalong: [u32; 4], dataint: [i32; 4]) -> ScriptStep {
    ScriptStep {
        delay_ms,
        command,
        datalong,
        dataint,
        ..Default::default()
    }
}

/// The pit with Short John Mithril at his place and vmangos's path and
/// scripts for him, short and quick: down two points to the chest's place,
/// back up one to his own. The Booty Run is in the data, starting in 2100,
/// so only a GM starts it.
fn pit() -> GameData {
    let mut d = GameData::empty();
    d.events.insert(
        arena::BOOTY_RUN_EVENT,
        GameEvent {
            start: naive("2100-01-01 00:00:00").and_utc().timestamp(),
            ..booty_row()
        },
    );
    d.creature_templates.insert(
        arena::ANNOUNCER_ENTRY,
        CreatureTemplate {
            entry: arena::ANNOUNCER_ENTRY,
            name: arena::ANNOUNCER_NAME.into(),
            display_ids: [14561, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            display_scales: [1.0, 0.0, 0.0, 0.0],
            level_min: 47,
            level_max: 47,
            faction: 35,
            npc_flags: 3,
            unit_class: 1,
            health_max: 2000,
            // Twenty yards a second walking, fifty-six running.
            speed_walk: 8.0,
            speed_run: 8.0,
            ..Default::default()
        },
    );
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: arena::ANNOUNCER_SPAWN,
        entries: vec![arena::ANNOUNCER_ENTRY],
        map: 0,
        x: HOME.x,
        y: HOME.y,
        z: HOME.z,
        orientation: HOME_O,
        respawn_min: 300,
        respawn_max: 300,
        ..Default::default()
    });
    let wp = |point: u32, at: Vector3d, wait_ms: u32, script_id: u32, o: Option<f32>| Waypoint {
        point,
        x: at.x,
        y: at.y,
        z: at.z,
        orientation: o,
        wait_ms,
        script_id,
    };
    let north = |y: f32| Vector3d {
        y: CENTRE.y + y,
        ..CENTRE
    };
    d.waypoints_by_entry.insert(
        arena::ANNOUNCER_ENTRY,
        vec![
            wp(1, north(12.0), 0, 0, None),
            wp(2, north(6.0), 0, 0, None),
            wp(arena::CHEST_POINT, FLOOR, 1000, DOWN_SCRIPT, None),
            wp(15, north(6.0), 0, 0, None),
            wp(31, HOME, 60_000, HOME_SCRIPT, Some(HOME_O)),
        ],
    );
    // vmangos's: run, the yell, the chest; walk, stop, gossip and quests.
    d.scripts.movement.insert(
        DOWN_SCRIPT,
        vec![
            step(0, 25, [1, 0, 0, 0], [0; 4]),
            step(1000, 0, [6, 0, 0, 0], [9731, 0, 0, 0]),
            step(2000, 9, [arena::BOOTY_CHEST_SPAWN, 10800, 0, 0], [0; 4]),
        ],
    );
    d.scripts.movement.insert(
        HOME_SCRIPT,
        vec![
            step(0, 25, [0, 0, 0, 0], [0; 4]),
            step(0, 20, [0, 0, 0, 0], [0; 4]),
            step(0, 4, [147, 1, 1, 0], [0; 4]),
            step(0, 4, [147, 2, 1, 0], [0; 4]),
        ],
    );
    for (id, text) in [
        (9730, "Arrr, Me Hearties! I be havin' some extra Treasure"),
        (9731, "Let the Bloodletting Begin!"),
    ] {
        d.npc.broadcast_texts.insert(
            id,
            crate::game::data::npc::BroadcastText {
                male: text.into(),
                chat_type: 6,
                ..Default::default()
            },
        );
    }
    d.gameobject_templates.insert(
        CHEST_ENTRY,
        crate::game::data::GameObjectTemplate {
            entry: CHEST_ENTRY,
            kind: 3,
            display_id: 259,
            size: 1.0,
            name: "Arena Treasure Chest".into(),
            ..Default::default()
        },
    );
    d.gameobject_spawns
        .entry(0)
        .or_default()
        .push(crate::game::data::GameObjectSpawn {
            guid: arena::BOOTY_CHEST_SPAWN,
            entry: CHEST_ENTRY,
            map: 0,
            x: -13202.9,
            y: 276.757,
            z: 21.857,
            state: 1,
            despawned: true,
            ..Default::default()
        });
    d
}

fn john() -> wow_world_messages::Guid {
    crate::game::guid::unit(arena::ANNOUNCER_ENTRY, arena::ANNOUNCER_SPAWN)
}

/// A GM in the pit, by its centre.
async fn gm(
    realm: &std::sync::Arc<crate::game::Realm>,
) -> (Client, wow_world_messages::Guid, MapHandle) {
    let mut c = Client::connect(realm, "Alice");
    let g = c.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    map.query(move |m| {
        let at = Vector3d {
            x: CENTRE.x + 5.0,
            ..CENTRE
        };
        m.teleport(g, m.key, at, 0.0);
    })
    .await;
    (c, g, map)
}

#[derive(Debug, Clone, Copy)]
struct Look {
    at: Vector3d,
    o: f32,
    /// Walking or running, at what speed, against his walk's.
    moving: Option<(bool, f32)>,
    walk_speed: f32,
    chest: Option<Vector3d>,
    npc_flags: u32,
    on_his_run: bool,
}

fn look(m: &Map) -> Option<Look> {
    let c = m.creature(john())?;
    let chest = crate::game::map::scripts::object_of_spawn(m, arena::BOOTY_CHEST_SPAWN)
        .and_then(|g| m.gameobject(g))
        .map(|o| o.position);
    Some(Look {
        at: c.position,
        o: c.orientation,
        moving: match &c.motion {
            Motion::Moving(w) => Some((w.run, w.speed)),
            Motion::Idle { .. } => None,
        },
        walk_speed: c.speeds.walk,
        chest,
        npc_flags: c.fields.get_u32(UNIT_NPC_FLAGS),
        on_his_run: m.arena.booty.runner.is_some(),
    })
}

fn flat(a: Vector3d, b: Vector3d) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// `.arena booty`: Short John walks (at his walking pace) down from the
/// stands toward the pit's centre, puts away his gossip, kneels at the
/// chest's place, and only then is the chest out, by him; then he yells
/// and runs back up to his place, facing as he did, his gossip back.
#[tokio::test]
async fn short_john_walks_down_kneels_leaves_the_chest_and_runs_home() {
    let realm = harness::realm_with(pit());
    let (mut a, _alice, map) = gm(&realm).await;
    a.sees(john()).await;
    a.say(".arena booty").await;
    hears(&mut a, "Arrr, Me Hearties!").await;
    let mut looks = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let l = map.query(|m| look(m)).await.flatten().expect("Short John");
        looks.push(l);
        if l.chest.is_some() && !l.on_his_run && l.moving.is_none() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "his run ends: {l:?}"
        );
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    let first_chest = looks.iter().position(|l| l.chest.is_some()).unwrap();
    let (down, back) = looks.split_at(first_chest);
    // On his way down: walking, nearer the centre all the while, his
    // gossip and quests put away.
    let walks: Vec<&Look> = down.iter().filter(|l| l.moving.is_some()).collect();
    assert!(!walks.is_empty(), "he walks down");
    for l in &walks {
        let (run, speed) = l.moving.unwrap();
        assert!(
            !run && (speed - l.walk_speed).abs() < 0.01,
            "walking: {l:?}"
        );
        assert_eq!(l.npc_flags & 3, 0, "no gossip on his way: {l:?}");
    }
    for w in down.windows(2) {
        assert!(
            flat(w[1].at, CENTRE) <= flat(w[0].at, CENTRE) + 0.5,
            "down toward the floor: {:?} then {:?}",
            w[0].at,
            w[1].at
        );
    }
    // The chest out only once he is at its place, and by him.
    let there = back[0];
    assert!(
        flat(there.at, FLOOR) < 1.0,
        "at the chest's place: {there:?}"
    );
    assert!(
        flat(there.chest.unwrap(), there.at) < 3.0,
        "the chest where he put it: {there:?}"
    );
    assert!(down.iter().all(|l| l.chest.is_none()));
    // He knelt to put it down, and yelled.
    let kneel = a
        .gets(Duration::from_secs(2), |m| match m {
            ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == john() => Some(e.emote),
            _ => None,
        })
        .await;
    assert_eq!(kneel, Some(Emote::OneshotKneel));
    hears(&mut a, "Let the Bloodletting Begin!").await;
    // Back: running, then home, as he stood.
    assert!(
        back.iter().any(|l| l.moving.is_some_and(|(run, _)| run)),
        "he runs home"
    );
    let home = *looks.last().unwrap();
    assert!(flat(home.at, HOME) < 0.5, "home: {home:?}");
    assert!((home.o - HOME_O).abs() < 0.01, "facing as he did: {home:?}");
    assert_eq!(home.npc_flags & 3, 3, "his gossip and quests back");
    assert!(home.chest.is_some(), "the chest stays out");
}

async fn hears(c: &mut Client, text: &str) -> String {
    let text = text.to_owned();
    c.wait_for("Short John", move |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(chat) if chat.message.contains(&text) => {
            Some(chat.message.clone())
        }
        _ => None,
    })
    .await
}

/// Killed on his way down, he leaves no chest; it is out anyway when he
/// would have put it down (the map's clock skipped to then), and his run
/// is over.
#[tokio::test]
async fn short_john_dead_on_his_way_still_leaves_the_chest() {
    let realm = harness::realm_with(pit());
    let (mut a, alice, map) = gm(&realm).await;
    a.sees(john()).await;
    a.say(".arena booty").await;
    hears(&mut a, "Arrr, Me Hearties!").await;
    let due = map
        .query(|m| {
            m.arena
                .booty
                .chest_due
                .map(|t| t.saturating_duration_since(m.now))
        })
        .await
        .flatten()
        .expect("a time for the chest");
    // Under a second's walk, three down at the chest, ten to spare.
    assert!(
        (Duration::from_secs(13)..=Duration::from_secs(15)).contains(&due),
        "{due:?}"
    );
    map.query(move |m| crate::game::map::combat::kill(m, alice, john()))
        .await;
    let gone = super::tests_p3_scripts::until(&map, 3000, |m| m.arena.booty.runner.is_none()).await;
    assert!(gone, "his run is over");
    let chest = |m: &mut Map| {
        crate::game::map::scripts::object_of_spawn(m, arena::BOOTY_CHEST_SPAWN).is_some()
    };
    assert!(!map.query(chest).await.unwrap(), "no chest yet");
    map.query(|m| m.arena.booty.chest_due = Some(m.now)).await;
    assert!(
        super::tests_p3_scripts::until(&map, 3000, chest).await,
        "the chest is out"
    );
}

/// A map that comes up with the Booty Run on (a restart in the middle of
/// it) has the chest out at once and Short John at his place.
#[tokio::test]
async fn a_map_up_during_the_booty_run_has_the_chest_out_and_short_john_home() {
    let realm = harness::realm_with(pit());
    assert_eq!(
        events::force(&realm, arena::BOOTY_RUN_EVENT, true),
        Some(true)
    );
    let (_a, _alice, map) = gm(&realm).await;
    let out =
        super::tests_p3_scripts::until(&map, 3000, |m| look(m).is_some_and(|l| l.chest.is_some()))
            .await;
    assert!(out, "the chest is out");
    let l = map.query(|m| look(m)).await.flatten().unwrap();
    assert!(
        flat(l.at, HOME) < 0.5 && l.moving.is_none() && !l.on_his_run,
        "{l:?}"
    );
}

/// vmangos's path for Short John on the real navmesh: from his place in the
/// stands each leg is walkable and ends where it should, his path's point
/// 14 is on the pit's floor (area 2177) by the chest's place, and its
/// script puts that chest out.
#[test]
#[ignore = "extended"]
fn short_johns_path_is_walkable_down_to_the_pit_floor() {
    let db = harness::real_data("db");
    let nav_dir = harness::real_data("nav");
    let d = harness::load_real(&db);
    let Some(mut nav) = crate::game::map::nav::Nav::open(&nav_dir, "Azeroth") else {
        panic!("no navmesh in {}", nav_dir.display());
    };
    let spawn = d.creature_spawns[&0]
        .iter()
        .find(|s| s.guid == arena::ANNOUNCER_SPAWN)
        .expect("his spawn");
    let path = &d.waypoints_by_entry[&arena::ANNOUNCER_ENTRY];
    assert_eq!(path.len(), 31);
    let mut at = Vector3d {
        x: spawn.x,
        y: spawn.y,
        z: spawn.z,
    };
    for w in path {
        let to = Vector3d {
            x: w.x,
            y: w.y,
            z: w.z,
        };
        let legs = nav
            .path(at, to)
            .unwrap_or_else(|| panic!("no path to point {}", w.point));
        let end = *legs.last().unwrap();
        assert!(
            flat(end, to) < 1.5 && (end.z - to.z).abs() < 3.0,
            "stuck short of point {}: {end:?} for {to:?}",
            w.point
        );
        at = to;
    }
    let floor = path.iter().find(|w| w.point == arena::CHEST_POINT).unwrap();
    let floor = Vector3d {
        x: floor.x,
        y: floor.y,
        z: floor.z,
    };
    assert_eq!(
        nav.zone_and_area(floor).map(|(_, a)| a),
        Some(arena::AREA_PIT),
        "point 14 on the pit's floor"
    );
    let chest = d.gameobject_spawns[&0]
        .iter()
        .find(|s| s.guid == arena::BOOTY_CHEST_SPAWN)
        .unwrap();
    assert!(
        flat(
            floor,
            Vector3d {
                x: chest.x,
                y: chest.y,
                z: chest.z
            }
        ) < 3.0
    );
    let script = &d.scripts.movement[&floor_script(path)];
    assert!(
        script
            .iter()
            .any(|s| s.command == 9 && s.datalong[0] == arena::BOOTY_CHEST_SPAWN),
        "point 14's script puts the chest out"
    );
    // He comes home to his place, facing as he stood.
    let last = path.last().unwrap();
    assert!(
        flat(
            Vector3d {
                x: last.x,
                y: last.y,
                z: last.z
            },
            Vector3d {
                x: spawn.x,
                y: spawn.y,
                z: spawn.z
            }
        ) < 0.5
    );
}

fn floor_script(path: &[Waypoint]) -> u32 {
    path.iter()
        .find(|w| w.point == arena::CHEST_POINT)
        .map_or(0, |w| w.script_id)
}

/// Short John in the real world: friendly to all (faction 35), so a player
/// free for all in the pit can't strike him, nor he them; and as the Booty
/// Run begins he sets off down his 31 points at a walk, his gossip put away.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_short_john_is_no_one_s_foe_and_sets_off_at_a_walk() {
    let Some(realm) = super::tests_p3_scripts::realm() else {
        return;
    };
    // Off as the map comes up, whatever the hour, so that it begins.
    if events::current(&realm).contains(&arena::BOOTY_RUN_EVENT) {
        events::force(&realm, arena::BOOTY_RUN_EVENT, false);
    }
    let (mut a, alice, _) =
        super::tests_p6_scripts::hero_of(&realm, "Alice", Race::Human, Class::Warrior).await;
    let by = Vector3d {
        x: CENTRE.x + 5.0,
        ..CENTRE
    };
    let map = super::tests_p6_scripts::travel(&realm, &mut a, 0, by).await;
    let seen = super::tests_p3_scripts::until(&map, 10_000, move |m| {
        m.creature(john())
            .is_some_and(|c| c.observers.contains(&alice))
    })
    .await;
    assert!(seen, "Short John in sight");
    let foes = map
        .query(move |m| {
            if let Some(p) = m.players.get_mut(&alice) {
                p.fields.set_flag(
                    PLAYER_FLAGS,
                    crate::game::map::player::PLAYER_FLAGS_FFA_PVP,
                    true,
                );
            }
            (
                crate::game::map::combat::can_attack(m, alice, john()),
                crate::game::map::combat::can_attack(m, john(), alice),
            )
        })
        .await;
    assert_eq!(foes, Some((false, false)), "no one's foe");
    a.say(".arena booty").await;
    hears(&mut a, "Arrr, Me Hearties!").await;
    let set_off = super::tests_p3_scripts::until(&map, 5000, |m| {
        look(m).is_some_and(|l| {
            l.moving
                .is_some_and(|(run, speed)| !run && (speed - l.walk_speed).abs() < 0.01)
        })
    })
    .await;
    assert!(set_off, "he walks");
    let (points, flags) = map
        .query(|m| {
            let c = m.creature(john())?;
            let points = c.scripted.as_deref()?.route.as_ref()?.points.len();
            Some((points, c.fields.get_u32(UNIT_NPC_FLAGS)))
        })
        .await
        .flatten()
        .expect("on his path");
    assert_eq!(points, 31);
    assert_eq!(flags & 3, 0, "his gossip and quests put away");
}
