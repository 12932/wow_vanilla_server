//! Phase 3's loader tests for the world (Thistle Tea's `world/loader`
//! tests of graveyards, gossip, quests, spawns and the rest): what wvs
//! reads of the world's tables and the rules it works out of them, on the
//! real data when it is here.

use wow_world_messages::vanilla::Vector3d;

use crate::game::data::scripts::MapTemplate;
use crate::game::data::{GameData, Graveyard};
use crate::game::map::death::graveyard_for;
use crate::game::registry::Team;

fn real() -> Option<&'static GameData> {
    crate::game::data::load::real_data()
}

const ALLIANCE: u32 = 469;
const HORDE: u32 = 67;

fn at(x: f32, y: f32, z: f32) -> Vector3d {
    Vector3d { x, y, z }
}

fn graveyard(id: u32, map: u32, (x, y, z): (f32, f32, f32), zone: u32, faction: u32) -> Graveyard {
    Graveyard {
        id,
        map,
        x,
        y,
        z,
        orientation: 0.0,
        zone,
        faction,
    }
}

/// The graveyard `faction` goes to from `here` at `pos` in `zone`, all open.
fn pick(d: &GameData, here: u32, zone: u32, faction: u32, pos: Vector3d) -> Option<u32> {
    graveyard_for(d, here, zone, faction, pos, |_| true).map(|g| g.id)
}

/// Thistle Tea's `graveyard_test.exs`, as mangos's `GetClosestGraveYard`:
/// the races' sides; the zone's nearest on the map; an enemy's skipped; one
/// on another map when the zone has none on this; none at all when nothing
/// is allowed; from a dungeon, one of its own first, else the nearest by
/// horizontal distance from its entrance.
#[test]
fn a_spirit_goes_to_its_zones_nearest_graveyard_of_its_side() {
    for race in [1, 3, 4, 7] {
        assert_eq!(Team::of_race(race), Team::Alliance, "{race}");
    }
    for race in [2, 5, 6, 8] {
        assert_eq!(Team::of_race(race), Team::Horde, "{race}");
    }
    let mut d = GameData::empty();
    d.graveyards = vec![
        graveyard(1, 0, (100.0, 0.0, 0.0), 10, 0),
        graveyard(2, 0, (10.0, 0.0, 0.0), 10, 0),
        graveyard(3, 0, (50.0, 0.0, 0.0), 10, 0),
        graveyard(4, 0, (10.0, 0.0, 0.0), 11, HORDE),
        graveyard(5, 0, (100.0, 0.0, 0.0), 11, ALLIANCE),
        graveyard(6, 1, (10.0, 0.0, 0.0), 12, 0),
        graveyard(7, 2, (10.0, 0.0, 0.0), 13, HORDE),
        // The dungeon's zone: one far on the entrance map, one high above
        // the entrance there, one on another continent, an enemy's.
        graveyard(8, 0, (100.0, 0.0, 0.0), 14, 0),
        graveyard(9, 0, (11.0, 20.0, 1000.0), 14, 0),
        graveyard(10, 1, (0.0, 0.0, 0.0), 14, 0),
        graveyard(11, 0, (10.0, 20.0, 0.0), 14, HORDE),
    ];
    d.scripts.map_templates.insert(
        36,
        MapTemplate {
            map_type: 1,
            ghost_entrance: Some((0, 10.0, 20.0)),
            ..Default::default()
        },
    );
    let origin = at(0.0, 0.0, 0.0);
    assert_eq!(pick(&d, 0, 10, ALLIANCE, origin), Some(2), "the nearest");
    assert_eq!(
        pick(&d, 0, 11, ALLIANCE, origin),
        Some(5),
        "not the enemy's"
    );
    assert_eq!(pick(&d, 0, 12, ALLIANCE, origin), Some(6), "on another map");
    // Map 2's only graveyard is the Horde's: nothing for the Alliance.
    assert_eq!(pick(&d, 2, 13, ALLIANCE, origin), None, "nothing allowed");
    assert_eq!(
        pick(&d, 36, 14, ALLIANCE, origin),
        Some(9),
        "nearest the entrance"
    );
    d.graveyards
        .push(graveyard(12, 36, (900.0, 900.0, 900.0), 14, 0));
    assert_eq!(
        pick(&d, 36, 14, ALLIANCE, origin),
        Some(12),
        "the dungeon's own"
    );
}

/// Thistle Tea's `graveyard_test.exs` loading and `graveyard_maps_test.exs`
/// on the real data: a graveyard's zone links by side, the facing a spirit
/// is turned to; and from Ragefire Chasm, whose zone names no graveyard
/// inside, the Horde's own outside Orgrimmar and the Alliance the one
/// shared, measured from the entrance; nothing for a map without any.
#[test]
fn the_real_graveyards_link_their_zones_sides_and_facings() {
    let Some(d) = real() else {
        return;
    };
    let links: Vec<(u32, u32)> = d
        .graveyards
        .iter()
        .filter(|g| g.zone == 2437)
        .map(|g| (g.id, g.faction))
        .collect();
    assert!(
        links.contains(&(32, 0)) && links.contains(&(850, HORDE)),
        "{links:?}"
    );
    let facing = d
        .graveyards
        .iter()
        .find(|g| g.id == 3)
        .map(|g| g.orientation);
    assert_eq!(facing, Some(3.83972));
    let inside = at(0.797_643_1, -8.353_291, -15.566_657);
    assert_eq!(pick(d, 389, 2437, HORDE, inside), Some(850));
    assert_eq!(pick(d, 389, 2437, ALLIANCE, inside), Some(32));
    assert_eq!(pick(d, 999, 0, HORDE, at(0.0, 0.0, 0.0)), None);
}
