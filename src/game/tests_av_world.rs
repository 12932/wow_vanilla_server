//! Alterac Valley's world copy (map 30 outside any match, where a GM's
//! `.go` or a teleport puts a player): it holds what a match holds at its
//! start, and its leaders hold their keeps as in a match; and Drek'Thar's
//! leash, a line across the one way out of his hall.

use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p8_scripts_av::{AV, all_of, spawn_at};
use super::tests_pvp2::{creature, npc, valley};
use crate::game::harness::{self, Client};
use crate::game::map::battleground::av_scripts::{self, DREK_THAR_LEASH, Leash};
use crate::game::map::{Map, MapHandle, MapKey, ai, event_ai};

const DREK_THAR: u32 = 11946;
const DREK_HOME: (f32, f32, f32) = (-1370.9, -219.8, 98.5);
const ICEBLOOD_WARMASTER: u32 = 14773;
/// The user's point: past it, east, Drek'Thar goes home.
const LINE: (f32, f32) = (-1358.81, -254.89);

fn at(x: f32, y: f32, z: f32) -> Vector3d {
    Vector3d { x, y, z }
}

/// `c` into the valley's world copy (no match), at `to`: the map it lands
/// in.
async fn into_the_world_copy(
    realm: &std::sync::Arc<crate::game::Realm>,
    c: &mut Client,
    to: Vector3d,
) -> MapHandle {
    let g = c.guid;
    let from = realm.registry.online(g).unwrap().map();
    from.call(move |m| m.teleport(g, MapKey::world(AV), to, 0.0));
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
    let map = realm.registry.online(g).unwrap().map();
    let key = map.query(|m| m.key).await.unwrap();
    assert_eq!(key, MapKey::world(AV), "the valley's world copy");
    map
}

/// `g` held at `to`: it stays there, fighting from where it stands.
fn hold_at(m: &mut Map, g: Guid, to: Vector3d) {
    if let Some(c) = m.creature_mut(g) {
        c.position = to;
        c.motion = crate::game::map::creature::Motion::Idle { until: None };
    }
    if let Some(s) = event_ai::scripted_mut(m, g) {
        s.no_combat_movement = true;
    }
}

/// Wait until `f` holds of the map, a few seconds at most.
async fn until(map: &MapHandle, what: &str, f: impl Fn(&Map) -> bool + Send + Clone + 'static) {
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        let g = f.clone();
        if map.query(move |m| g(m)).await.unwrap_or(false) {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The line: east of the point (-y) he is drawn too far, however near
/// home; west of it he isn't, however far (the corridor's mouth 35 yd from
/// his spawn, past vmangos's 33); and the other leaders' leash is their
/// yards about home.
#[test]
fn drek_thars_leash_is_a_line_east_of_his_hall_and_the_others_are_yards() {
    let home = at(DREK_HOME.0, DREK_HOME.1, DREK_HOME.2);
    let past = |x: f32, y: f32| DREK_THAR_LEASH.past(home, at(x, y, 99.4));
    assert!(!past(home.x, home.y), "at home");
    assert!(!past(-1351.0, -243.0), "his hall's south-east corner");
    assert!(!past(-1388.0, -228.0), "its far side");
    assert!(!past(-1356.0, -252.0), "in the corridor's mouth, 35 yd out");
    assert!(
        !past(LINE.0, LINE.1 + 0.5),
        "just inside the corridor's mouth"
    );
    assert!(past(LINE.0, LINE.1 - 0.5), "just past it");
    assert!(past(-1352.0, -266.0), "down the corridor");
    assert!(past(-1342.0, -284.0), "at the gate");
    assert!(
        !past(-1342.0, -200.0),
        "west, outside his walls: not his way out"
    );
    let vanndar = Leash::Yards(35.0);
    let v = at(722.4, -11.0, 50.7);
    assert!(!vanndar.past(v, at(690.0, -11.0, 50.7)));
    assert!(vanndar.past(v, at(680.0, -11.0, 50.7)));
}

/// Drek'Thar's hall in the valley's world copy: Drek'Thar at his spawn,
/// his Iceblood Warmaster beside him, all against everyone (faction 14);
/// Alice (an Alliance warrior) by them.
async fn the_hall() -> (Client, Guid, MapHandle, Guid, Guid) {
    let mut d = valley();
    npc(&mut d, ICEBLOOD_WARMASTER, "Iceblood Warmaster", 14);
    if let Some(s) = d
        .creature_spawns
        .get_mut(&AV)
        .and_then(|s| s.iter_mut().find(|s| s.entries == [DREK_THAR]))
    {
        (s.x, s.y, s.z) = DREK_HOME;
    }
    spawn_at(
        &mut d,
        170601,
        ICEBLOOD_WARMASTER,
        (DREK_HOME.0 + 3.0, DREK_HOME.1, DREK_HOME.2),
    );
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = into_the_world_copy(
        &realm,
        &mut a,
        at(DREK_HOME.0 + 8.0, DREK_HOME.1, DREK_HOME.2),
    )
    .await;
    let (drek, warmaster) = map
        .query(|m| {
            (
                creature(m, DREK_THAR).unwrap(),
                creature(m, ICEBLOOD_WARMASTER).unwrap(),
            )
        })
        .await
        .unwrap();
    (a, alice, map, drek, warmaster)
}

/// The user's leash, on the world copy, where a match's sweep never ran:
/// Alice pulls Drek'Thar out of his hall into the corridor's mouth, 35 yd
/// from his spawn (past vmangos's 33), and he fights on; a yard past the
/// line east of it he gives up the fight and goes home. His warmaster,
/// pulled past the line (and 70 yd out), fights on: the line is his alone.
/// And fighting, he called it to Alice (his linked guards, as in a match).
#[tokio::test]
async fn drek_thar_goes_home_once_drawn_east_of_his_hall_and_his_warmaster_does_not() {
    let (_a, alice, map, drek, warmaster) = the_hall().await;
    map.query(move |m| {
        ai::attacked(m, drek, alice);
        av_scripts::sweep_soon(m);
    })
    .await;
    // His warmaster comes for Alice, called.
    until(&map, "his warmaster called to Alice", move |m| {
        m.creature(warmaster)
            .is_some_and(|c| c.combat.attacking && c.combat.victim == alice)
    })
    .await;
    // The corridor's mouth, west of the line: he fights on.
    map.query(move |m| {
        hold_at(m, drek, at(-1356.0, -252.0, 99.4));
        av_scripts::sweep_soon(m);
    })
    .await;
    harness::an_update(&map).await;
    harness::an_update(&map).await;
    let (fighting, evading) = map
        .query(move |m| {
            let c = m.creature(drek).unwrap();
            (c.combat.attacking, c.evading)
        })
        .await
        .unwrap();
    assert!(
        fighting && !evading,
        "35 yd out, west of the line, he fights on"
    );
    // His warmaster past the line, far down the corridor and out at the
    // gate: it fights on.
    map.query(move |m| {
        hold_at(m, warmaster, at(-1342.0, -290.0, 91.6));
        av_scripts::sweep_soon(m);
    })
    .await;
    harness::an_update(&map).await;
    harness::an_update(&map).await;
    // Drek'Thar a yard past the line, in the corridor's mouth: home.
    map.query(move |m| {
        hold_at(m, drek, at(LINE.0, LINE.1 - 1.0, 99.4));
        av_scripts::sweep_soon(m);
    })
    .await;
    until(&map, "Drek'Thar goes home", move |m| {
        m.creature(drek).is_some_and(|c| c.evading)
    })
    .await;
    let (warmaster_fights, warmaster_evades) = map
        .query(move |m| {
            let c = m.creature(warmaster).unwrap();
            (c.combat.attacking && c.combat.victim == alice, c.evading)
        })
        .await
        .unwrap();
    assert!(
        warmaster_fights && !warmaster_evades,
        "his warmaster, past the line, fights on"
    );
}

/// The world copy holds a match's start, not every event's spawns at once:
/// Iceblood Tower's banner as the Horde holds it, not as the Alliance
/// assaults it nor burnt; and the generals (theirs at the start too). Before,
/// all three of Iceblood's banners stood together.
#[tokio::test]
async fn the_valleys_world_copy_holds_what_a_match_holds_at_its_start() {
    let realm = harness::realm_with(valley());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = into_the_world_copy(&realm, &mut a, at(0.0, 0.0, 0.0)).await;
    let (banners, generals, matched) = map
        .query(|m| {
            let mut banners: Vec<u32> = m
                .gameobjects
                .iter()
                .filter_map(|(_, o)| o.spawn_guid)
                .filter(|s| (160011..=160013).contains(s))
                .collect();
            banners.sort_unstable();
            (
                banners,
                [DREK_THAR, 11948].map(|e| all_of(m, e).len()),
                m.battleground.is_some(),
            )
        })
        .await
        .unwrap();
    assert!(!matched, "no match on the world copy");
    assert_eq!(
        banners,
        [160011],
        "the Horde's banner, as at a match's start"
    );
    assert_eq!(generals, [1, 1]);
}

// The real valley.

/// On the real navmesh, the line is across the one way out of Drek'Thar's
/// hall: the way from his spawn to the keep's gate crosses it once, in the
/// corridor's mouth (13 yd wide there, its walls 4 to 9 yd either side of
/// the user's point); his hall's corners are reached from his spawn without
/// crossing it; and the user's point is on his hall's floor (99.4).
#[test]
#[ignore = "extended"]
fn drek_thars_line_is_across_the_one_way_out_of_his_hall_on_the_real_navmesh() {
    let dir = harness::real_data("nav");
    let mut nav = crate::game::map::nav::Nav::open(&dir, "PVPZone01")
        .unwrap_or_else(|| panic!("no Alterac Valley navmesh in {}", dir.display()));
    let home = at(DREK_HOME.0, DREK_HOME.1, DREK_HOME.2);
    let floor = nav.ground(LINE.0, LINE.1, 99.4).expect("the point's floor");
    assert!((floor - 99.37).abs() < 0.5, "its floor at {floor}");
    let past = |p: &Vector3d| DREK_THAR_LEASH.past(home, *p);
    let crossings = |path: &[Vector3d]| {
        path.windows(2)
            .filter(|w| past(&w[0]) != past(&w[1]))
            .count()
    };
    // The keep's gate, by its guards.
    let gate = at(-1342.0, -284.0, 91.6);
    let out = nav.path(home, gate).expect("a way out to the gate");
    assert_eq!(crossings(&out), 1, "the way out crosses it once: {out:?}");
    assert!(past(out.last().unwrap()));
    for corner in [
        at(-1352.0, -243.0, 99.4),
        at(-1384.0, -243.0, 99.4),
        at(-1389.0, -225.0, 99.4),
        at(-1384.0, -212.0, 99.4),
        at(-1352.0, -212.0, 99.4),
        at(-1356.0, -252.0, 99.4),
    ] {
        let to = at(
            corner.x,
            corner.y,
            nav.ground(corner.x, corner.y, 99.4).unwrap(),
        );
        let way = nav.path(home, to).expect("a way about his hall");
        assert_eq!(crossings(&way), 0, "{corner:?} is his hall's: {way:?}");
        assert!(!past(&to));
    }
    // The corridor's walls, across it along the line.
    let p = at(LINE.0, LINE.1, floor);
    let width: Vec<f32> = [1.0f32, -1.0]
        .iter()
        .map(|s| {
            let stop = nav
                .walk_hit(p, at(p.x + s * 20.0, p.y, p.z))
                .expect("a wall");
            (stop.x - p.x).abs()
        })
        .collect();
    assert!(
        width.iter().all(|w| (3.0..=10.0).contains(w))
            && (10.0..=16.0).contains(&width.iter().sum::<f32>()),
        "the corridor's walls {width:?} yd from the point"
    );
}

/// The real valley's world copy: the match's start (Drek'Thar's four
/// warmasters, of the Horde's four towers; one guard on each of the
/// gate's spots, not its four upgrades stacked), its keep hostile to an
/// Alliance player there, and Drek'Thar's leash: pulled east past the line
/// he goes home.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_valleys_world_copy_has_drek_thar_his_warmasters_and_his_line() {
    let realm = harness::realm_with(harness::load_real(&harness::real_data("db")));
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = into_the_world_copy(&realm, &mut a, at(-1340.0, -230.0, 99.4)).await;
    let warmasters: [u32; 8] = std::array::from_fn(|i| 14770 + i as u32);
    let (drek, theirs, gate, hostile) = map
        .query(move |m| {
            let drek = all_of(m, DREK_THAR);
            let theirs: Vec<u32> = warmasters
                .iter()
                .copied()
                .filter(|e| !all_of(m, *e).is_empty())
                .collect();
            let spot = at(-1344.2, -285.2, 91.1);
            let gate = m
                .creatures
                .iter()
                .filter(|(_, c)| c.alive() && crate::game::map::combat::dist(c.home, spot) < 0.5)
                .count();
            let hostile = drek
                .first()
                .is_some_and(|d| crate::game::map::combat::can_attack(m, *d, alice));
            (drek, theirs, gate, hostile)
        })
        .await
        .unwrap();
    assert_eq!(drek.len(), 1, "Drek'Thar");
    assert_eq!(
        theirs,
        [14772, 14773, 14776, 14777],
        "his warmasters of the Horde's four towers"
    );
    assert_eq!(gate, 1, "one Frostwolf Legionnaire on the gate's spot");
    assert!(hostile, "Drek'Thar is hostile to an Alliance player");
    // In one go, the map's own update not between: he fights Alice, is
    // held two yards past the line, and the leaders' sweep looks.
    let drek = drek[0];
    let (fought, went_home) = map
        .query(move |m| {
            ai::attacked(m, drek, alice);
            let fought = m.creature(drek).is_some_and(|c| c.combat.attacking);
            hold_at(m, drek, at(LINE.0 + 1.0, LINE.1 - 2.0, 99.4));
            av_scripts::sweep_soon(m);
            av_scripts::world_update(m);
            (fought, m.creature(drek).is_some_and(|c| c.evading))
        })
        .await
        .unwrap();
    assert!(fought, "Drek'Thar fights Alice");
    assert!(went_home, "past the line, Drek'Thar goes home");
}
