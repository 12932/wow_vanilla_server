//! Thistle Tea's terrain tiles: their liquids and areas, and the
//! environment's harm from them. The tests with the real tiles (and the
//! navmesh bake, for WMO liquid) pass over where those are missing.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::{AreaInfo, GameData};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::nav::Nav;
use crate::game::map::terrain::*;
use crate::game::map::{Map, MapKey, environment};

static TERRAIN: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("terrain"));
static BAKE: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("nav"));

/// Out at sea west of Stranglethorn, over three hundred yards of it.
const HIGH_SEA: (f32, f32, f32) = (-15000.0, 1024.0, -5.0);
/// Nearer the coast: ocean, not deep.
const SHALLOWS: (f32, f32, f32) = (-15000.0, 64.0, -5.0);
/// A pool of lava in Searing Gorge, surface 334.8.
const SEARING_GORGE: (f32, f32, f32) = (-6488.0, -976.0, 334.0);
/// Goldshire's road, and a lake south of it.
const GOLDSHIRE: (f32, f32, f32) = (-9460.0, 62.0, 56.0);
const ELWYNN_LAKE: (f32, f32, f32) = (-9640.0, -1040.0, 37.0);
/// Molten Core's lava, a WMO's (surface -209.9).
const MOLTEN_CORE: (f32, f32, f32) = (512.0, -820.0, -211.0);
/// Undercity's sewers' slime, a WMO's (surface -64.5).
const UNDERCITY: (f32, f32, f32) = (1450.0, 180.0, -65.5);

const ELWYNN: u32 = 12;
const GOLDSHIRE_AREA: u32 = 87;
/// Goldshire's `area_bit` in `AreaTable.dbc`.
const GOLDSHIRE_BIT: u32 = 548;

fn v((x, y, z): (f32, f32, f32)) -> Vector3d {
    Vector3d { x, y, z }
}

fn tiles() -> Option<Terrain> {
    Terrain::open(TERRAIN.as_path(), 0)
}

fn data() -> GameData {
    let mut d = GameData::empty();
    for (id, zone, explore_bit, name) in [
        (ELWYNN, 0, 126, "Elwynn Forest"),
        (GOLDSHIRE_AREA, ELWYNN, GOLDSHIRE_BIT, "Goldshire"),
        (1, 0, 119, "Dun Morogh"),
    ] {
        d.areas.insert(
            id,
            AreaInfo {
                id,
                map: 0,
                zone,
                explore_bit,
                flags: 64,
                level: 1,
                name: name.into(),
                team: 0,
            },
        );
    }
    // Naxxramas's slime: a third off every stat, and nature damage.
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura: 137,
        base_points: -91,
        base_dice: 1,
        misc_value: -1,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(
        SPELL_NAXX_SLIME,
        SpellTemplate {
            id: SPELL_NAXX_SLIME,
            name: "Slime".into(),
            school: 3,
            effects,
            equipped_item_class: -1,
            ..Default::default()
        },
    );
    d
}

/// A realm with the tiles (and the navmesh, with `nav`); `None` without
/// them.
fn realm(nav: bool) -> Option<Arc<Realm>> {
    if tiles().is_none() || (nav && !BAKE.as_path().join("Azeroth.map").exists()) {
        return None;
    }
    let mut cfg = harness::test_config();
    cfg.terrain_dir = Some(TERRAIN.clone());
    if nav {
        cfg.nav_dir = Some(BAKE.clone());
    }
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Some(Realm::new(cfg, store, startup, data(), None))
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

/// Put `g` at `at`, hardy enough to live through what is there.
fn place(m: &mut Map, g: Guid, at: (f32, f32, f32)) {
    let p = m.players.get_mut(&g).unwrap();
    p.movement.position = v(at);
    p.fields.set_u32(UNIT_FIELD_MAXHEALTH, 100_000);
    p.fields.set_u32(UNIT_FIELD_HEALTH, 100_000);
}

/// The environment's check, as the update makes it, `after` from now.
fn check(m: &mut Map, g: Guid, after: Duration) {
    m.now += after;
    if let Some(p) = m.players.get_mut(&g) {
        p.misc.env_checked_at = None;
    }
    environment::update(m);
}

#[test]
fn the_tiles_know_the_deep_sea_lava_water_and_dry_ground() {
    let Some(mut t) = tiles() else {
        return;
    };
    let sea = t.liquid(v(HIGH_SEA)).expect("the open sea");
    assert!(sea.deep(), "the open sea is the deep sea: {sea:?}");
    assert_ne!(sea.flags & LIQUID_OCEAN, 0);
    assert!(sea.surface > -1.0 && sea.floor < -300.0, "{sea:?}");
    let shallows = t.liquid(v(SHALLOWS)).expect("the coast's ocean");
    assert_eq!(shallows.flags, LIQUID_OCEAN);
    let lava = t.liquid(v(SEARING_GORGE)).expect("Searing Gorge's lava");
    assert!(lava.magma(), "{lava:?}");
    assert!(lava.touching(SEARING_GORGE.2));
    assert!(
        !lava.touching(lava.surface + 1.0),
        "a yard over it isn't touching it"
    );
    let lake = t.liquid(v(ELWYNN_LAKE)).expect("Elwynn's lake");
    assert_eq!(lake.flags, LIQUID_WATER);
    assert!(!lake.deep());
    assert_eq!(t.liquid(v(GOLDSHIRE)), None, "Goldshire's road is dry");
    assert_eq!(
        t.liquid(v((SEARING_GORGE.0, SEARING_GORGE.1, lava.floor - 3.0))),
        None,
        "under the lava's floor is out of it"
    );
    assert_eq!(
        t.area_bit(GOLDSHIRE.0, GOLDSHIRE.1),
        Some(GOLDSHIRE_BIT as u16)
    );
    assert_eq!(t.tiles_read(), 5, "only the tiles asked about are read");
}

#[test]
fn a_wmo_names_its_liquid() {
    let dir = BAKE.as_path();
    let Some(mut nav) = Nav::open(dir, "MoltenCore") else {
        return;
    };
    let (entry, surface, floor) = nav.wmo_liquid(v(MOLTEN_CORE)).expect("Molten Core's lava");
    let lava = Liquid::from_wmo(entry, surface, floor).unwrap();
    assert!(lava.magma() && lava.touching(MOLTEN_CORE.2), "{lava:?}");
    let Some(mut nav) = Nav::open(dir, "Azeroth") else {
        return;
    };
    let (entry, surface, floor) = nav.wmo_liquid(v(UNDERCITY)).expect("Undercity's slime");
    let slime = Liquid::from_wmo(entry, surface, floor).unwrap();
    assert_eq!((slime.flags, slime.entry), (LIQUID_SLIME, Some(4)));
    assert_eq!(
        slime.spell(UNDERCITY.2),
        None,
        "only Naxxramas's slime has a spell"
    );
    // Naxxramas's: in it, not over it.
    let naxx = Liquid::from_wmo(21, 10.0, 0.0).unwrap();
    assert_eq!(naxx.flags, LIQUID_SLIME);
    assert_eq!(naxx.spell(9.5), Some(SPELL_NAXX_SLIME));
    assert_eq!(naxx.spell(10.0), None);
    assert_eq!(Liquid::from_wmo(7, 10.0, 0.0), None);
}

#[tokio::test]
async fn the_liquid_is_a_wmos_first_then_the_terrains() {
    let Some(realm) = realm(true) else {
        return;
    };
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let (sewer, gorge, road) = on_map(&realm, |m| {
        (
            environment::liquid_at(m, v(UNDERCITY)),
            environment::liquid_at(m, v(SEARING_GORGE)),
            environment::liquid_at(m, v(GOLDSHIRE)),
        )
    })
    .await;
    let sewer = sewer.expect("Undercity's slime");
    assert_eq!((sewer.flags, sewer.entry), (LIQUID_SLIME, Some(4)));
    let gorge = gorge.expect("Searing Gorge's lava");
    assert!(gorge.magma() && gorge.entry.is_none(), "{gorge:?}");
    assert_eq!(road, None);
}

#[tokio::test]
async fn lava_burns_after_a_second_in_it() {
    let Some(realm) = realm(false) else {
        return;
    };
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.clear();
    let (graced, burnt, again, after) = on_map(&realm, move |m| {
        place(m, g, SEARING_GORGE);
        check(m, g, Duration::ZERO);
        let health = |m: &Map| m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH);
        let graced = health(m);
        check(m, g, Duration::from_millis(1100));
        let burnt = health(m);
        // Not again for two seconds.
        check(m, g, Duration::from_millis(500));
        let again = health(m);
        check(m, g, Duration::from_millis(1600));
        (graced, burnt, again, health(m))
    })
    .await;
    assert_eq!(graced, 100_000, "a second's grace");
    assert!((100_000 - 610..=100_000 - 605).contains(&burnt), "{burnt}");
    assert_eq!(again, burnt);
    assert!(after < burnt);
    let hurt = a
        .wait_for("SMSG_ENVIRONMENTAL_DAMAGE_LOG", |m| match m {
            ServerOpcodeMessage::SMSG_ENVIRONMENTAL_DAMAGE_LOG(d) => {
                Some((d.damage_type, d.damage))
            }
            _ => None,
        })
        .await;
    assert_eq!(hurt.0, EnvironmentalDamageType::Lava);
    assert!((605..=610).contains(&hurt.1));
    // A god doesn't burn.
    let godly = on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().misc.god = true;
        check(m, g, Duration::from_secs(3));
        check(m, g, Duration::from_secs(3));
        m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH)
    })
    .await;
    assert_eq!(godly, after);
}

#[tokio::test]
async fn the_deep_sea_tires_but_the_shallows_and_dry_ground_dont() {
    let Some(realm) = realm(false) else {
        return;
    };
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.clear();
    let started = on_map(&realm, move |m| {
        place(m, g, HIGH_SEA);
        check(m, g, Duration::ZERO);
        m.players[&g].misc.fatigue.is_some()
    })
    .await;
    assert!(started);
    a.wait_for("the fatigue timer", |m| match m {
        ServerOpcodeMessage::SMSG_START_MIRROR_TIMER(t) => {
            (t.timer == TimerType::Fatigue).then_some(())
        }
        _ => None,
    })
    .await;
    let hurt = on_map(&realm, move |m| {
        check(m, g, Duration::from_secs(61));
        m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH)
    })
    .await;
    assert!(hurt < 100_000);
    a.wait_for("exhaustion", |m| match m {
        ServerOpcodeMessage::SMSG_ENVIRONMENTAL_DAMAGE_LOG(d) => {
            (d.damage_type == EnvironmentalDamageType::Exhausted).then_some(())
        }
        _ => None,
    })
    .await;
    // In the shallows the timer fills again; on land it stops.
    let (filling, gone) = on_map(&realm, move |m| {
        place(m, g, SHALLOWS);
        check(m, g, Duration::from_secs(1));
        let filling = m.players[&g].misc.fatigue.is_some_and(|f| !f.draining);
        place(m, g, GOLDSHIRE);
        check(m, g, Duration::from_secs(1));
        let p = &m.players[&g];
        (
            filling,
            p.misc.fatigue.is_none() && p.misc.breath.is_none() && p.misc.lava.is_none(),
        )
    })
    .await;
    assert!(filling);
    assert!(gone);
}

#[tokio::test]
async fn the_terrain_gives_the_zone_and_area_without_a_navmesh() {
    let Some(realm) = realm(false) else {
        return;
    };
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let (zone, area, explored) = on_map(&realm, move |m| {
        place(m, g, GOLDSHIRE);
        crate::game::map::world::update_zone(m, g);
        let p = &m.players[&g];
        let word = p
            .fields
            .get_u32(PLAYER_EXPLORED_ZONES_1 + (GOLDSHIRE_BIT / 32) as u16);
        (p.zone, p.area, word & (1 << (GOLDSHIRE_BIT % 32)) != 0)
    })
    .await;
    assert_eq!((zone, area), (ELWYNN, GOLDSHIRE_AREA));
    assert!(explored, "Goldshire is explored");
}

#[tokio::test]
async fn naxxramas_slime_puts_its_spell_on_while_in_it() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let (on, off) = on_map(&realm, move |m| {
        environment::liquid_spell(m, g, Some(SPELL_NAXX_SLIME));
        let on = m.players[&g].auras.has_spell(SPELL_NAXX_SLIME);
        environment::liquid_spell(m, g, None);
        (on, m.players[&g].auras.has_spell(SPELL_NAXX_SLIME))
    })
    .await;
    assert!(on, "in the slime, slimed");
    assert!(!off, "out of it, not");
}

/// Where the tiles have each kind of liquid (run by hand).
#[test]
#[ignore]
fn probe_the_tiles() {
    let Some(mut t) = Terrain::open(TERRAIN.as_path(), 0) else {
        return;
    };
    let scan = |t: &mut Terrain, x0: f32, x1: f32, y0: f32, y1: f32, step: f32| {
        let mut found = std::collections::BTreeMap::<u16, Vec<(f32, f32, f32, f32)>>::new();
        let mut x = x0;
        while x < x1 {
            let mut y = y0;
            while y < y1 {
                if let Some(l) = t.liquid(v((x, y, 10_000.0))) {
                    let e = found.entry(l.flags).or_default();
                    if e.len() < 6 {
                        e.push((x, y, l.surface, l.floor));
                    }
                }
                y += step;
            }
            x += step;
        }
        found
    };
    println!(
        "Searing Gorge {:?}",
        scan(&mut t, -7300.0, -6300.0, -2000.0, -600.0, 16.0)
    );
    println!(
        "Burning Steppes {:?}",
        scan(&mut t, -8300.0, -7500.0, -3000.0, -600.0, 32.0)
    );
    println!(
        "coast {:?}",
        scan(&mut t, -15000.0, -9000.0, 0.0, 3000.0, 64.0)
    );
    println!(
        "Elwynn {:?}",
        scan(&mut t, -9800.0, -8900.0, -1200.0, 200.0, 32.0)
    );
    println!("Goldshire area bit {:?}", t.area_bit(-9460.0, 62.0));
    println!("{t:?}");

    let Some(mut nav) = Nav::open(BAKE.as_path(), "MoltenCore") else {
        return;
    };
    let mut hits = 0;
    'mc: for x in (0..120).map(|i| 400.0 + i as f32 * 8.0) {
        for y in (0..120).map(|i| -1300.0 + i as f32 * 8.0) {
            for z in (0..30).map(|i| -300.0 + i as f32 * 10.0) {
                if let Some((entry, s, f)) = nav.wmo_liquid(v((x, y, z))) {
                    println!("MC {x} {y} {z}: {entry} {s} {f}");
                    hits += 1;
                    if hits > 8 {
                        break 'mc;
                    }
                    break;
                }
            }
        }
    }
    let Some(mut nav) = Nav::open(BAKE.as_path(), "Azeroth") else {
        return;
    };
    let mut hits = 0;
    'uc: for x in (0..60).map(|i| 1450.0 + i as f32 * 5.0) {
        for y in (0..60).map(|i| 100.0 + i as f32 * 5.0) {
            for z in (0..20).map(|i| -80.0 + i as f32 * 3.0) {
                if let Some((entry, s, f)) = nav.wmo_liquid(v((x, y, z))) {
                    println!("UC {x} {y} {z}: {entry} {s} {f}");
                    hits += 1;
                    if hits > 12 {
                        break 'uc;
                    }
                    break;
                }
            }
        }
    }
}
