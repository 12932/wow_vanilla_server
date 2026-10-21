//! Round 11, against Thistle Tea's `rest_test.exs` ("uses the DBC linked
//! zone for maps without navigation data"): a dungeon with neither a
//! navmesh nor terrain, and no zone of its own in `AreaTable.dbc`, is in
//! the zone `Map.dbc` links it to (Blackrock Depths, 230, in 1584 on the
//! continent); one with a zone of its own keeps it.

use std::sync::Arc;

use wow_world_messages::vanilla::Vector3d;

use super::harness;
use crate::game::Realm;
use crate::game::data::{AreaInfo, GameData, MapInfo};
use crate::game::map::MapKey;

const DEPTHS: u32 = 230;
const DEPTHS_ZONE: u32 = 1584;
/// A dungeon whose own zone (its only one) is not the one `Map.dbc` names.
const TEMPLE: u32 = 109;
const TEMPLE_OWN: u32 = 1417;
const TEMPLE_LINKED: u32 = 1477;

fn realm() -> Arc<Realm> {
    let mut d = GameData::empty();
    for (map, name) in [(DEPTHS, "BlackrockDepths"), (TEMPLE, "SunkenTemple")] {
        d.maps.insert(
            map,
            MapInfo {
                id: map,
                internal_name: name.into(),
                name: name.into(),
                instance_type: 1,
                player_limit: 5,
            },
        );
    }
    d.map_zones.insert(DEPTHS, DEPTHS_ZONE);
    d.map_zones.insert(TEMPLE, TEMPLE_LINKED);
    for (id, map) in [(DEPTHS_ZONE, 0), (TEMPLE_LINKED, 0), (TEMPLE_OWN, TEMPLE)] {
        d.areas.insert(
            id,
            AreaInfo {
                id,
                map,
                name: format!("Area {id}"),
                ..Default::default()
            },
        );
    }
    harness::realm_with(d)
}

#[tokio::test]
async fn a_dungeon_with_no_zone_of_its_own_is_in_the_one_map_dbc_links() {
    let realm = realm();
    let mut out = Vec::new();
    for map in [DEPTHS, TEMPLE] {
        let handle = realm.maps.get(&realm, MapKey { map, instance: 1 });
        out.push(
            handle
                .query(|m| {
                    let blind = m.nav.is_none() && m.terrain.is_none();
                    let at = Vector3d {
                        x: 10.0,
                        y: 10.0,
                        z: 0.0,
                    };
                    (blind, crate::game::map::world::zone_and_area(m, at))
                })
                .await
                .unwrap(),
        );
    }
    assert_eq!(
        out,
        vec![
            (true, Some((DEPTHS_ZONE, DEPTHS_ZONE))),
            (true, Some((TEMPLE_OWN, TEMPLE_OWN))),
        ]
    );
}

/// The client's `Map.dbc` links each dungeon to its zone: Zul'Gurub to
/// 1977, Blackrock Depths to 1584, Blackrock Spire to 1583, Onyxia's Lair
/// to 2159, the Stockade to 717.
#[test]
#[ignore = "extended"]
fn real_map_dbc_links_each_dungeon_to_its_zone() {
    let d = harness::load_real(&harness::real_data("db"));
    let zones: Vec<Option<u32>> = [309, 230, 229, 249, 34]
        .iter()
        .map(|m| d.map_zones.get(m).copied())
        .collect();
    assert_eq!(
        zones,
        [Some(1977), Some(1584), Some(1583), Some(2159), Some(717)]
    );
}
