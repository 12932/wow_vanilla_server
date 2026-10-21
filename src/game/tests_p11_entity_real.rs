//! Round 11's entity tests on the real world (extended), ported from
//! Thistle Tea's `player_fatigue_maps_test.exs`, `player_lava_maps_test.exs`
//! and `effect_resolver/rotating_cone_dbc_test.exs` (docs/parity/matrix.csv):
//! the terrain's liquids at Thistle Tea's own points, a ghost's graveyard
//! beyond the navmesh, Blackrock Mountain's lava, and Lord Kazzak's whirl's
//! cone. Each passes over where its data is missing.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::nav::Nav;
use crate::game::map::spells::{self, Targets, mods::Mods, targets};
use crate::game::map::terrain::*;
use crate::game::map::{Map, MapKey, death, environment, grid, world};

static TERRAIN: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("terrain"));
static BAKE: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("nav"));
static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("db"));

/// Thistle Tea's lake, in Elwynn east of Goldshire.
const LAKE: (f32, f32, f32) = (-9500.0, -220.0, 55.47);
/// Thistle Tea's coastal ocean and high sea, off Westfall.
const COAST: (f32, f32, f32) = (-10_000.0, 2400.0, 0.0);
const HIGH_SEA: (f32, f32, f32) = (-10_000.0, 2500.0, 0.0);
/// Further out: The Great Sea (area 2364, of Westfall), where the navmesh
/// has nothing.
const GREAT_SEA: (f32, f32, f32) = (-10_000.0, 3500.0, 0.0);
const WESTFALL: u32 = 40;
const THE_GREAT_SEA: u32 = 2364;
/// The Great Sea's `area_bit` in `AreaTable.dbc`.
const GREAT_SEA_BIT: u16 = 894;
/// Westfall's one graveyard, Sentinel Hill's, for either side.
const SENTINEL_HILL: (u32, f32, f32, f32) = (4, -10_546.9, 1197.24, 31.7263);
/// Blackrock Mountain, over its lava and under its bridge.
const BLACKROCK: (f32, f32, f32) = (-7580.0, -1140.0, 167.2);
/// Lord Kazzak, his whirl's eight cones, and the one straight ahead.
const KAZZAK: u32 = 12_397;
const WHIRLS: [u32; 8] = [24820, 24821, 24822, 24823, 24835, 24836, 24837, 24838];
const WHIRL_AHEAD: u32 = 24820;

fn v((x, y, z): (f32, f32, f32)) -> Vector3d {
    Vector3d { x, y, z }
}

fn close(a: f32, b: f32, within: f32) -> bool {
    (a - b).abs() <= within
}

/// A realm whose maps have the real tiles (and navmesh, with `nav`), on
/// `data`.
fn realm(data: GameData, nav: bool) -> std::sync::Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.terrain_dir = Some(TERRAIN.clone());
    if nav {
        cfg.nav_dir = Some(BAKE.clone());
    }
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, data, None)
}

/// Thistle Tea's lake, coast and high sea are each told apart by the
/// tiles: the lake inland water (8), its surface 57.631 and floor 55.4705;
/// the coast ocean (2); the high sea ocean and deep (18). A map with no
/// tiles has no terrain, and a point of Eastern Kingdoms where it has no
/// tile has no liquid (Thistle Tea's `player_fatigue_maps_test.exs:55`,
/// "distinguishes inland water, coastal ocean, high sea and unavailable
/// terrain"; mangos's `MAP_LIQUID_TYPE_WATER`, `_OCEAN` and `_DARK_WATER`,
/// and `LIQUID_MAP_NO_WATER` without a grid map). The tests_terrain ones
/// are at points of wvs's own.
#[test]
#[ignore = "extended"]
fn thistle_teas_lake_coast_and_high_sea_and_where_there_is_no_terrain() {
    let Some(mut t) = Terrain::open(TERRAIN.as_path(), 0) else {
        return;
    };
    let lake = t.liquid(v(LAKE)).expect("the lake");
    assert_eq!(lake.flags, LIQUID_WATER, "{lake:?}");
    assert!(close(lake.surface, 57.631, 0.001), "{lake:?}");
    assert!(close(lake.floor, 55.4705, 0.001), "{lake:?}");
    let coast = t.liquid(v(COAST)).expect("the coast");
    assert_eq!(coast.flags, LIQUID_OCEAN, "{coast:?}");
    assert!(!coast.deep());
    let sea = t.liquid(v(HIGH_SEA)).expect("the high sea");
    assert_eq!(sea.flags, LIQUID_OCEAN | LIQUID_DEEP, "{sea:?}");
    assert!(sea.deep());
    assert!(
        Terrain::open(TERRAIN.as_path(), 9999).is_none(),
        "no tiles for a map that isn't"
    );
    // Tile (0, 0), which Eastern Kingdoms hasn't.
    assert_eq!(t.liquid(v((17_000.0, 17_000.0, 0.0))), None);
    assert_eq!(t.area_bit(17_000.0, 17_000.0), None);
}

/// A ghost out on The Great Sea where there is no navmesh has its zone
/// from the tiles' area bit (894: The Great Sea, of Westfall), and
/// released there goes to Westfall's graveyard. Thistle Tea's maps have
/// no navmesh at its point; wvs's bake has one wherever the tiles are,
/// and gives the same zone and area there, so the realm here has the
/// tiles alone. Thistle Tea's test gives Westfall a graveyard of its own
/// at (-10000, 1900, 10); wvs has the real data's one, Sentinel Hill's
/// (4), which the release teleports to (Thistle Tea's
/// `player_fatigue_maps_test.exs:66`, "rescues ghosts beyond navigation
/// coverage using terrain exploration bits"; mangos's
/// `GetClosestGraveYard` takes the zone from the terrain at the point).
#[tokio::test]
#[ignore = "extended"]
async fn a_ghost_beyond_the_navmesh_is_released_to_its_terrain_zones_graveyard() {
    let db = DB.as_path();
    if !db.join("vmangos.sqlite").exists() {
        return;
    }
    let (Some(mut tiles), Some(mut nav)) = (
        Terrain::open(TERRAIN.as_path(), 0),
        Nav::open(BAKE.as_path(), "Azeroth"),
    ) else {
        return;
    };
    let at = v(GREAT_SEA);
    assert_eq!(
        nav.zone_and_area(at),
        Some((WESTFALL, THE_GREAT_SEA)),
        "wvs's bake covers it"
    );
    assert_eq!(tiles.area_bit(at.x, at.y), Some(GREAT_SEA_BIT));
    drop((tiles, nav));
    let realm = realm(harness::load_real(db), false);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Drowned", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).expect("online").map();
    let (found, zone, area, graveyard) = map
        .query(move |m| {
            let found = (m.nav.is_none(), world::zone_and_area(m, at));
            m.players.get_mut(&g).unwrap().movement.position = at;
            world::update_zone(m, g);
            let (zone, area) = (m.players[&g].zone, m.players[&g].area);
            let graveyard = death::graveyard_for(&m.realm.data, 0, zone, 469, at, |_| true)
                .map(|gy| (gy.id, gy.x, gy.y, gy.z));
            death::kill(m, g);
            (found, zone, area, graveyard)
        })
        .await
        .unwrap();
    assert_eq!(found, (true, Some((WESTFALL, THE_GREAT_SEA))), "the tiles'");
    assert_eq!((zone, area), (WESTFALL, THE_GREAT_SEA));
    assert_eq!(graveyard, Some(SENTINEL_HILL));
    a.clear();
    a.send(CMSG_REPOP_REQUEST {}).await;
    let to = a
        .wait_for("the teleport to the graveyard", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => Some(t.info.position),
            _ => None,
        })
        .await;
    let (_, x, y, z) = SENTINEL_HILL;
    assert!(
        close(to.x, x, 0.01) && close(to.y, y, 0.01) && close(to.z, z, 0.01),
        "{to:?}"
    );
    let ghost = map
        .query(move |m| matches!(m.players[&g].death, death::Death::Ghost { .. }))
        .await
        .unwrap();
    assert!(ghost);
}

/// Blackrock Mountain's lava is a WMO's: magma (its entry 3), its surface
/// 168.448 and its floor 148.49, at Thistle Tea's point; the bridge over it
/// (z 239) and under its floor (z 145) are out of it, and the tiles have
/// no liquid there. The map's liquid is the WMO's, and a map neither the
/// bake nor the tiles know has none (Thistle Tea's
/// `player_lava_maps_test.exs:22`, "samples Blackrock Mountain lava
/// separately from bridges and buried terrain"; mangos's
/// `TerrainInfo::getLiquidStatus`, the vmap's liquid before the grid's).
#[tokio::test]
#[ignore = "extended"]
async fn blackrock_mountains_lava_is_the_wmos_and_not_on_its_bridge_or_under_it() {
    let (Some(mut tiles), Some(mut nav)) = (
        Terrain::open(TERRAIN.as_path(), 0),
        Nav::open(BAKE.as_path(), "Azeroth"),
    ) else {
        return;
    };
    let (x, y, _) = BLACKROCK;
    assert_eq!(tiles.liquid(v(BLACKROCK)), None, "the tiles' liquid");
    let (entry, surface, floor) = nav.wmo_liquid(v(BLACKROCK)).expect("the lava");
    let lava = Liquid::from_wmo(entry, surface, floor).expect("a liquid");
    assert_eq!(
        (lava.entry, lava.flags),
        (Some(3), LIQUID_MAGMA),
        "{lava:?}"
    );
    assert!(close(surface, 168.448, 0.01), "{lava:?}");
    assert!(close(floor, 148.490, 0.01), "{lava:?}");
    assert_eq!(nav.wmo_liquid(v((x, y, 239.0))), None, "the bridge");
    assert_eq!(nav.wmo_liquid(v((x, y, 145.0))), None, "under its floor");
    drop((tiles, nav));
    let realm = realm(GameData::empty(), true);
    let here = realm
        .maps
        .get(&realm, MapKey::world(0))
        .query(|m| environment::liquid_at(m, v(BLACKROCK)))
        .await
        .unwrap();
    assert_eq!(here, Some(lava));
    let nowhere = realm
        .maps
        .get(&realm, MapKey::world(9999))
        .query(|m| {
            (
                m.nav.is_none() && m.terrain.is_none(),
                environment::liquid_at(m, v((0.0, 0.0, 0.0))),
            )
        })
        .await
        .unwrap();
    assert_eq!(nowhere, (true, None));
}

/// Put `g` at `at`, in the grid's cell there.
fn put(m: &mut Map, g: Guid, at: Vector3d) {
    let p = m.players.get_mut(&g).unwrap();
    p.movement.position = at;
    let (from, to) = (p.cell, grid::cell_of(at.x, at.y));
    p.cell = to;
    m.grid.relocate(g, from, to);
}

/// Each of Lord Kazzak's whirl's cones is a third of a circle, a hundred
/// yards long (its radius 12). The one straight ahead (24820), cast by
/// him, takes a player 55 degrees off its axis and not one 65 degrees off
/// the other way, nor a dead one inside it, nor one a yard beyond its
/// radius: by its targets and by its cast's hits, of the players (it
/// takes the Northshire guards in it too, his enemies). Thistle Tea turns
/// each child by its own offset (an eighth of a turn a step); wvs turns
/// Kazzak himself as the tick casts it (`boss_mechanics::whirl`), which
/// tests_bosses' `kazzaks_whirl_turns` covers. Thistle Tea's other
/// world's player has no counterpart: a world is a map actor of its own
/// (Thistle Tea's `rotating_cone_dbc_test.exs:43`, "each whirl child
/// selects a rotated 120-degree arc with world and distance filtering";
/// mangos's `PUSH_IN_FRONT`, `IsInFront` within `2 * M_PI_F / 3`, of the
/// targetable for attack).
#[tokio::test]
#[ignore = "extended"]
async fn a_whirls_cone_takes_sixty_degrees_each_way_within_its_radius_and_not_the_dead() {
    let dir = DB.as_path();
    if !dir.join("vmangos.sqlite").exists() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(dir));
    let mut clients = Vec::new();
    for name in ["Inside", "Outside", "Fallen", "Distant"] {
        let mut c = Client::connect(&realm, name);
        let g = c.enter(name, Race::Human, Class::Warrior).await;
        c.send(CMSG_COMPLETE_CINEMATIC {}).await;
        clients.push((c, g));
    }
    let [inside, outside, fallen, distant] = [0, 1, 2, 3].map(|i| clients[i].1);
    let map = realm.registry.online(inside).expect("online").map();
    let kazzak = map
        .query(move |m| {
            let at = m.players[&inside].position();
            let g = crate::game::map::scripts::summon(m, Guid::zero(), KAZZAK, at, 0.0, None)
                .expect("Lord Kazzak");
            // He neither swings nor casts on his own.
            if let Some(s) = crate::game::map::event_ai::scripted_mut(m, g) {
                s.summoner = None;
                s.passive = true;
                s.no_melee = true;
                s.spell_list = Some(0);
            }
            crate::game::map::scripts::stop_for(m, g);
            g
        })
        .await
        .unwrap();
    clients[0].0.sees(kazzak).await;
    let (radius, cones, picked) = map
        .query(move |m| {
            let data = m.realm.data.clone();
            let s = data.spells.get(WHIRL_AHEAD).cloned().expect("the whirl");
            let mods = Mods::default();
            let radius = targets::radius(m, &s, 0, &mods);
            let cones = WHIRLS.map(|id| data.spells.cones.get(&id).copied());
            let k = m.creature(kazzak).expect("Lord Kazzak");
            let (from, facing) = (k.position, k.orientation);
            let at = |degrees: f32, yards: f32| {
                let a = facing + degrees.to_radians();
                Vector3d {
                    x: from.x + yards * a.cos(),
                    y: from.y + yards * a.sin(),
                    z: from.z,
                }
            };
            put(m, inside, at(55.0, 20.0));
            put(m, outside, at(-65.0, 20.0));
            put(m, fallen, at(0.0, 20.0));
            put(m, distant, at(0.0, radius + 1.0));
            death::kill(m, fallen);
            let t = Targets {
                unit: Some(kazzak),
                ..Default::default()
            };
            let picked: Vec<Guid> = targets::effect_targets(m, kazzak, &s, 0, &t, &mods)
                .into_iter()
                .filter(|g| guid::is_player(*g))
                .collect();
            (radius, cones, picked)
        })
        .await
        .unwrap();
    assert_eq!(radius, 100.0, "its radius");
    assert_eq!(cones, [Some(120); 8], "a third of a circle each");
    assert_eq!(picked, vec![inside], "55 degrees off, alone");
    clients[0].0.clear();
    map.call(move |m| spells::cast_triggered(m, kazzak, WHIRL_AHEAD, Some(kazzak)));
    let hits = clients[0]
        .0
        .wait_for("the whirl's cast", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(s)
                if s.spell == WHIRL_AHEAD && s.caster == kazzak =>
            {
                Some(
                    s.hits
                        .iter()
                        .copied()
                        .filter(|g| guid::is_player(*g))
                        .collect::<Vec<_>>(),
                )
            }
            _ => None,
        })
        .await;
    assert_eq!(hits, vec![inside], "its hits");
}
