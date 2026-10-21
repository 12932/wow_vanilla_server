//! Round 11's test-only rows on the world, most on the real world
//! (Thistle Tea's `mount_test`, `player_fatigue_maps_test`,
//! `player_lava_maps_test`, `form_passives_dbc_test`,
//! `rotating_cone_dbc_test` and `conditional_triggers_dbc_test`): a mount
//! on an open deck and in Booty Bay, terrain where there is none, a ghost
//! off the navmesh sent to its zone's graveyard, Blackrock Mountain's lava,
//! a druid's hidden form passive back at login, Kazzak's whirl's arcs, and
//! Linken's Boomerang's disarm and stun.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit as kit;
use crate::game::Realm;
use crate::game::data::{AreaInfo, GameData, GameObjectTemplate, Graveyard};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::auras::*;
use crate::game::map::mounts::{self, NO_MOUNTS_ALLOWED};
use crate::game::map::{Map, MapHandle, MapKey, grid, spells};

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

fn v((x, y, z): (f32, f32, f32)) -> Vector3d {
    Vector3d { x, y, z }
}

/// Move player `g` to `at`, as the map sees it.
fn put(m: &mut Map, g: Guid, at: Vector3d) {
    let Some(p) = m.players.get_mut(&g) else {
        return;
    };
    p.movement.position = at;
    p.move_check = None;
    let (old, new) = (p.cell, grid::cell_of(at.x, at.y));
    p.cell = new;
    m.grid.relocate(g, old, new);
    m.visibility_dirty.insert(g);
}

/// A mount, the Black Qiraji Battle Tank's summons, and a ship of the
/// open-decked kind (display 3015).
const MOUNT: u32 = 997_701;
const SHIP: u32 = 64_201;

fn decks(d: &mut GameData) {
    let mut mount = bare(MOUNT, 0);
    mount.duration_index = LASTS_LONG;
    effect(&mut mount, 0, aura(AURA_MOUNTED, KOBOLD as i32, 0, 1));
    let mut tank = bare(mounts::BLACK_QIRAJI, 0);
    tank.attributes[0] |= crate::game::data::spells::ATTR_ALLOW_WHILE_MOUNTED;
    insert(d, [mount, tank]);
    d.gameobject_templates.insert(
        SHIP,
        GameObjectTemplate {
            entry: SHIP,
            kind: 15,
            display_id: 3015,
            name: "Ship".into(),
            ..Default::default()
        },
    );
}

fn aboard(at: Vector3d) -> MovementInfo_MovementFlags {
    MovementInfo_MovementFlags::empty().set_on_transport(MovementInfo_MovementFlags_OnTransport {
        transport: TransportInfo {
            guid: crate::game::guid::game_object(SHIP, 1),
            position: at,
            orientation: 0.0,
        },
    })
}

/// Thistle Tea's "transports distinguish exposed decks and the Black
/// Qiraji restriction", its decks: a mount goes on a ship's deck open to
/// the sky (`environment::on_open_deck`), not below it, and the Black
/// Qiraji Battle Tank on no deck (mangos-classic's `Spell::CheckCast`:
/// `SPELL_FAILED_NO_MOUNTS_ALLOWED` on a transport, as the tank's
/// `IsOutdoors` check is). Its Booty Bay half is on the real world, below.
#[tokio::test]
async fn a_mount_goes_on_an_open_deck_and_the_qiraji_tank_on_none() {
    let realm = fast_realm(decks);
    let (_c, a, map) = human(&realm, "P11W1", "Sailor", Class::Warrior).await;
    let out = on(&map, move |m| {
        let s = |m: &Map, id: u32| m.realm.data.spells.get(id).unwrap().clone();
        let (mount, tank) = (s(m, MOUNT), s(m, mounts::BLACK_QIRAJI));
        let at = |m: &mut Map, deck: Vector3d| {
            m.players.get_mut(&a).unwrap().movement.flags = aboard(deck);
            (
                mounts::check(m, a, &mount, false),
                mounts::check(m, a, &tank, false),
            )
        };
        let open = at(m, v((6.21, 0.12, 14.05)));
        let below = at(m, v((6.21, 0.12, 12.0)));
        m.players.get_mut(&a).unwrap().movement.flags = MovementInfo_MovementFlags::empty();
        let ashore = (
            mounts::check(m, a, &mount, false),
            mounts::check(m, a, &tank, false),
        );
        (open, below, ashore)
    })
    .await;
    let no = Some(NO_MOUNTS_ALLOWED);
    assert_eq!(out.0, (None, no), "the open deck: a mount, not the tank");
    assert_eq!(out.1, (no, no), "below deck: neither");
    assert_eq!(out.2, (None, None), "ashore: both");
}

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));
static TERRAIN: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("terrain"));
static BAKE: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("nav"));

/// A realm on `data` with the real terrain tiles, and the navmesh with
/// `nav`.
fn with_terrain(data: GameData, nav: bool) -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.terrain_dir = Some(TERRAIN.clone());
    if nav {
        cfg.nav_dir = Some(BAKE.clone());
    }
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, data, None)
}

/// Booty Bay's docks, and Stranglethorn's road north of the town.
const BOOTY_BAY: (f32, f32, f32) = (-14_297.0, 530.0, 8.8);
const STRANGLETHORN: (f32, f32, f32) = (-11_920.0, -580.0, 16.0);
const BROWN_HORSE: u32 = 458;

/// Thistle Tea's "transports distinguish exposed decks and the Black
/// Qiraji restriction", its Booty Bay half: no mount in Booty Bay (area
/// 35, mangos-classic's `Spell::CheckCast` at Spell.cpp:5897), and one on
/// Stranglethorn's road outside it, on the real terrain's areas.
#[tokio::test]
#[ignore = "extended"]
async fn no_mount_in_booty_bay_and_one_outside_it() {
    let realm = with_terrain(harness::load_real(DB.as_path()), false);
    let mut c = Client::connect(&realm, "P11W2");
    let g = c.enter("Rider", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    let out = on(&map, move |m| {
        let horse = m.realm.data.spells.get(BROWN_HORSE).cloned().unwrap();
        let at = |m: &mut Map, p: (f32, f32, f32)| {
            put(m, g, v(p));
            (
                crate::game::map::world::zone_and_area(m, v(p)),
                mounts::check(m, g, &horse, false),
            )
        };
        (at(m, BOOTY_BAY), at(m, STRANGLETHORN))
    })
    .await;
    assert_eq!(out.0.0.map(|z| z.1), Some(35), "Booty Bay: {:?}", out.0);
    assert_eq!(out.0.1, Some(NO_MOUNTS_ALLOWED), "no mount in Booty Bay");
    assert_eq!(out.1.1, None, "a mount outside it: {:?}", out.1);
}

/// Thistle Tea's "distinguishes inland water, coastal ocean, high sea and
/// unavailable terrain", its last: a map with no tiles has no terrain, and
/// a point on it no liquid, as a point on a continent off its tiles has
/// none; a map of no terrain asks no tile (`environment::liquid_at`). The
/// lake, the coast and the deep sea are
/// `tests_terrain::the_tiles_know_the_deep_sea_lava_water_and_dry_ground`.
#[tokio::test]
#[ignore = "extended"]
async fn there_is_no_liquid_where_there_is_no_terrain() {
    use crate::game::map::terrain::Terrain;
    assert!(
        Terrain::open(TERRAIN.as_path(), 9999).is_none(),
        "no map 9999"
    );
    let mut azeroth = Terrain::open(TERRAIN.as_path(), 0).expect("Azeroth's tiles");
    let far = (16_900.0, 16_900.0, 0.0);
    assert_eq!(azeroth.liquid(v(far)), None, "off its tiles");
    assert_eq!(azeroth.area_bit(far.0, far.1), None);
    // A realm without the tiles: no liquid, and no zone from them.
    let realm = harness::realm();
    let map = realm.maps.get(&realm, MapKey::world(0));
    let none = on(&map, |m| {
        crate::game::map::environment::liquid_at(m, v((-10_000.0, 2_500.0, 0.0)))
    })
    .await;
    assert_eq!(none, None);
}

/// Westfall (zone 40) and its Dagger Hills (2364, terrain bit 894); a
/// graveyard in Westfall far off and one of another zone near.
const WESTFALL: u32 = 40;
const DAGGER_HILLS: u32 = 2364;
const OFF_THE_MESH: (f32, f32, f32) = (-10_000.0, 3_500.0, 0.0);
const WESTFALL_YARD: (f32, f32, f32) = (-10_000.0, 1_900.0, 10.0);
const NEARER_YARD: (f32, f32, f32) = (-10_000.0, 3_400.0, 0.0);

fn westfall() -> GameData {
    let mut d = GameData::empty();
    for (id, zone, explore_bit, name) in [
        (WESTFALL, 0, 0, "Westfall"),
        (DAGGER_HILLS, WESTFALL, 894, "The Dagger Hills"),
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
    for (id, (x, y, z), zone) in [
        (900_001, WESTFALL_YARD, WESTFALL),
        (900_002, NEARER_YARD, 12),
    ] {
        d.graveyards.push(Graveyard {
            id,
            map: 0,
            x,
            y,
            z,
            orientation: 0.0,
            zone,
            faction: 469,
        });
    }
    d
}

/// Thistle Tea's "rescues ghosts beyond navigation coverage using terrain
/// exploration bits": a player who dies where there is no navmesh has
/// their zone from the terrain's area bit (`world::update_zone`), and
/// their ghost goes to that zone's graveyard (mangos's
/// `GetClosestGraveyard` by zone), not a nearer one of another zone.
#[tokio::test]
#[ignore = "extended"]
async fn a_ghost_off_the_navmesh_goes_to_its_terrain_zones_graveyard() {
    let realm = with_terrain(westfall(), false);
    let mut c = Client::connect(&realm, "P11W3");
    let g = c.enter("Lost", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    let zone = on(&map, move |m| {
        assert!(m.nav.is_none(), "no navmesh");
        put(m, g, v(OFF_THE_MESH));
        crate::game::map::world::update_zone(m, g);
        let p = &m.players[&g];
        let zone = (p.zone, p.area);
        crate::game::map::death::kill(m, g);
        zone
    })
    .await;
    assert_eq!(zone, (WESTFALL, DAGGER_HILLS), "from the terrain's bit");
    c.send(CMSG_REPOP_REQUEST {}).await;
    c.wait_for("the teleport to the graveyard", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: c.teleport_order(),
        time: Default::default(),
    })
    .await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        let at = on(&map, move |m| m.players[&g].position()).await;
        if (at.x - WESTFALL_YARD.0).abs() < 1.0 && (at.y - WESTFALL_YARD.1).abs() < 1.0 {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "at {at:?}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Blackrock Mountain's lava under its bridge, a WMO's (surface 168.4).
const BLACKROCK: (f32, f32, f32) = (-7_580.0, -1_140.0, 167.2);

/// Thistle Tea's "samples Blackrock Mountain lava separately from bridges
/// and buried terrain": the terrain has no liquid there, the mountain's
/// WMO has lava (entry 3, surface 168.45, floor 148.49), none on the
/// bridge over it nor in the rock under it, and the liquid there is the
/// WMO's (`environment::liquid_at`).
#[tokio::test]
#[ignore = "extended"]
async fn blackrock_mountains_lava_is_its_wmos_not_its_bridges_or_rocks() {
    use crate::game::map::nav::Nav;
    use crate::game::map::terrain::Terrain;
    let mut tiles = Terrain::open(TERRAIN.as_path(), 0).expect("Azeroth's tiles");
    assert_eq!(tiles.liquid(v(BLACKROCK)), None, "not the terrain's");
    let mut nav = Nav::open(BAKE.as_path(), "Azeroth").expect("Azeroth's bake");
    let (entry, surface, floor) = nav.wmo_liquid(v(BLACKROCK)).expect("the WMO's lava");
    assert_eq!(entry, 3);
    assert!((surface - 168.448).abs() < 0.05, "{surface}");
    assert!((floor - 148.490).abs() < 0.05, "{floor}");
    for z in [239.0, 145.0] {
        assert_eq!(
            nav.wmo_liquid(v((BLACKROCK.0, BLACKROCK.1, z))),
            None,
            "none at {z}"
        );
    }
    let realm = with_terrain(GameData::empty(), true);
    let map = realm.maps.get(&realm, MapKey::world(0));
    let lava = on(&map, |m| {
        crate::game::map::environment::liquid_at(m, v(BLACKROCK))
    })
    .await;
    let lava = lava.expect("the liquid there");
    assert!(lava.magma() && lava.touching(BLACKROCK.2), "{lava:?}");
}

const CAT_FORM: u32 = 768;
const FELINE_SWIFTNESS: u32 = 24_866;
const HIDDEN_DODGE: u32 = 24_864;

/// Thistle Tea's "rebuilding the login spellbook restores hidden passives
/// in the retained form": a druid with Feline Swiftness's second rank who
/// logs out in Cat Form logs in in it, with the rank and the hidden
/// passive the form brings (24864), as mangos's `_LoadAuras` and
/// `HandleShapeshiftBoosts` put them back.
#[tokio::test]
#[ignore = "extended"]
async fn a_druid_back_in_cat_form_has_its_hidden_passive() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (c, g, map) = kit::player(&realm, "P11W4", "Prowler", Race::NightElf, Class::Druid).await;
    let before = on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, g, 60);
        spells::learn(m, g, CAT_FORM);
        spells::learn(m, g, FELINE_SWIFTNESS);
        spells::cast_triggered(m, g, CAT_FORM, Some(g));
        [CAT_FORM, FELINE_SWIFTNESS, HIDDEN_DODGE].map(|s| kit::has(m, g, s))
    })
    .await;
    assert_eq!(before, [true; 3], "in Cat Form, with its passive");
    c.disconnect();
    harness::offline(&realm, g).await;
    let mut c = Client::connect(&realm, "P11W4");
    c.login(g).await;
    let map = realm.registry.online(g).unwrap().map();
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        let after = on(&map, move |m| {
            (
                m.players[&g].fields.get_byte(UNIT_FIELD_BYTES_1, 2),
                [CAT_FORM, FELINE_SWIFTNESS, HIDDEN_DODGE].map(|s| kit::has(m, g, s)),
            )
        })
        .await;
        if after == (1, [true; 3]) {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "back: {after:?}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

const START: (f32, f32, f32) = (-8_949.95, -132.493, 83.5312);
const KAZZAK: u32 = 12_397;
const SHADOW_BOLT_WHIRL: u32 = 24_834;
const WHIRL: [u32; 8] = [
    24_820, 24_821, 24_822, 24_823, 24_835, 24_836, 24_837, 24_838,
];

/// `yards` from the start at `turns` eighths of a turn.
fn around(turns: f32, yards: f32) -> Vector3d {
    let a = turns * std::f32::consts::FRAC_PI_4;
    v((
        START.0 + yards * a.cos(),
        START.1 + yards * a.sin(),
        START.2,
    ))
}

/// Thistle Tea's "each whirl child selects a rotated 120-degree arc with
/// world and distance filtering": each of Shadow Bolt Whirl's eight cones
/// is 120 degrees (`boss_mechanics::fixups`), turned an eighth more than
/// the last; the second hits those at its own eighth and the two beside
/// it, not one a quarter turn off, nor a dead one in its arc, nor one in
/// its arc past its radius. Another world can't be in a map actor.
#[tokio::test]
#[ignore = "extended"]
async fn kazzaks_whirl_hits_a_turned_arc_of_the_living_within_its_reach() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let mut ring = Vec::new();
    for n in 0..6 {
        let name = format!("Ring{}", (b'a' + n) as char);
        let mut c = Client::connect(&realm, &name);
        let g = c.enter(&name, Race::Human, Class::Warrior).await;
        c.send(CMSG_COMPLETE_CINEMATIC {}).await;
        ring.push((c, g));
    }
    let map = realm.registry.online(ring[0].1).unwrap().map();
    let gs: Vec<Guid> = ring.iter().map(|r| r.1).collect();
    let (kazzak, cones, reach) = on(&map, |m| {
        let cones: Vec<i32> = WHIRL
            .iter()
            .map(|s| m.realm.data.spells.cones.get(s).copied().unwrap_or(0))
            .collect();
        let s = m.realm.data.spells.get(WHIRL[2]).cloned().unwrap();
        let reach = m.realm.data.spells.radius(s.effects[0].radius_index);
        let k = crate::game::map::scripts::summon(m, Guid::zero(), KAZZAK, v(START), 0.0, None)
            .expect("Kazzak");
        if let Some(s) = crate::game::map::event_ai::scripted_mut(m, k) {
            s.summoner = None;
            s.passive = true;
            s.no_melee = true;
            s.spell_list = Some(0);
        }
        crate::game::map::scripts::stop_for(m, k);
        (k, cones, reach)
    })
    .await;
    assert_eq!(cones, vec![120; 8], "each cone 120 degrees");
    assert!(reach > 10.0, "its reach: {reach}");
    // The second cone faces two eighths round: those at one, two and three
    // eighths; one at four; one dead at two; one at two past its reach.
    let spots = [
        around(1.0, 8.0),
        around(2.0, 8.0),
        around(3.0, 8.0),
        around(4.0, 8.0),
        around(2.0, 6.0),
        around(2.0, reach + 5.0),
    ];
    let (h, x) = on(&map, move |m| {
        for (g, at) in gs.iter().zip(spots) {
            put(m, *g, at);
            let f = &mut m.players.get_mut(g).unwrap().fields;
            f.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
            f.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
        }
        crate::game::map::death::kill(m, gs[4]);
        if let Some(c) = m.creature_mut(kazzak) {
            c.orientation = 0.0;
        }
        spells::cast_triggered(m, kazzak, SHADOW_BOLT_WHIRL, Some(kazzak));
        let a = auras(m, kazzak).expect("his auras");
        let h = a
            .holders
            .iter()
            .find(|h| h.spell == SHADOW_BOLT_WHIRL)
            .cloned()
            .expect("the whirl");
        let x = h.effects.first().cloned().unwrap();
        remove_spell(m, kazzak, SHADOW_BOLT_WHIRL, None);
        (h, x)
    })
    .await;
    for (c, _) in ring.iter_mut() {
        c.clear();
    }
    let gs: Vec<Guid> = ring.iter().map(|r| r.1).collect();
    let hurt = on(&map, move |m| {
        let mut x = x;
        // The second tick: the third cone of eight, two eighths round.
        x.ticks = 1;
        crate::game::map::boss_mechanics::periodic_trigger(m, kazzak, &h, &x);
        gs.iter()
            .map(|g| 1_000_000 - kit::health(m, *g).min(1_000_000))
            .collect::<Vec<u32>>()
    })
    .await;
    for (i, what) in [(0, "one eighth"), (1, "two eighths"), (2, "three eighths")] {
        assert!(hurt[i] > 0, "{what}: hit {hurt:?}");
    }
    assert_eq!(hurt[3], 0, "a quarter turn off: {hurt:?}");
    assert_eq!(hurt[5], 0, "past its reach: {hurt:?}");
    let dead = ring[4].1;
    let told = ring[4]
        .0
        .gets(Duration::from_millis(300), |m| match m {
            ServerOpcodeMessage::SMSG_SPELLNONMELEEDAMAGELOG(d) if d.target == dead => Some(()),
            _ => None,
        })
        .await;
    assert_eq!(told, None, "not the dead");
}

const LINKENS_BOOMERANG: u32 = 15_712;
const BOOMERANG_DISARM: u32 = 15_752;
const BOOMERANG_STUN: u32 = 15_753;

/// Thistle Tea's "boomerang damage permits all four control outcomes and
/// normal recovery": Linken's Boomerang hurts, and its two chances (one in
/// 31, one in 11: `tests_items3::linkens_boomerang_rarely_stuns_and_
/// jumper_cables_mostly_fail`) bring its disarm, its stun, both or
/// neither; the stun goes after its three seconds, the disarm after its
/// ten, a disarmed warrior's blows are their fists' until then and their
/// weapon's after, and death takes both.
#[tokio::test]
#[ignore = "extended"]
async fn linkens_boomerang_disarms_and_stuns_and_both_wear_off() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (_a, a, map) = kit::player(&realm, "P11W5", "Thrower", Race::Human, Class::Warrior).await;
    let (_b, b, _) = kit::player(&realm, "P11W6", "Target", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        let data = m.realm.data.clone();
        let kind = |id: u32| {
            data.spells
                .get(id)
                .map(|s| s.effects.map(|e| e.aura))
                .unwrap_or_default()
        };
        assert!(
            kind(BOOMERANG_DISARM).contains(&AURA_MOD_DISARM),
            "15752 disarms"
        );
        assert!(kind(BOOMERANG_STUN).contains(&AURA_MOD_STUN), "15753 stuns");
        let hands = |m: &Map| {
            let f = &m.players[&b].fields;
            (
                f.get_f32(UNIT_FIELD_MAXDAMAGE),
                f.get_u32(UNIT_FIELD_BASEATTACKTIME),
            )
        };
        let state = |m: &Map| {
            (
                kit::has(m, b, BOOMERANG_DISARM),
                kit::has(m, b, BOOMERANG_STUN),
            )
        };
        let armed = hands(m);
        kit::set_health(m, b, 100_000);
        if let Some(f) = m.unit_fields_mut(b) {
            f.set_u32(UNIT_FIELD_MAXHEALTH, 100_000);
        }
        spells::cast_triggered(m, a, LINKENS_BOOMERANG, Some(b));
        let hurt = kit::health(m, b) < 100_000;
        let mut combos = Vec::new();
        for (disarm, stun) in [(false, false), (true, false), (false, true), (true, true)] {
            if disarm {
                spells::cast_triggered(m, a, BOOMERANG_DISARM, Some(b));
            }
            if stun {
                spells::cast_triggered(m, a, BOOMERANG_STUN, Some(b));
            }
            let on_it = (state(m), hands(m));
            kit::pass(m, 3_001);
            let later = state(m);
            kit::pass(m, 8_000);
            let recovered = (state(m), hands(m));
            // Again, and death takes them.
            spells::cast_triggered(m, a, BOOMERANG_DISARM, Some(b));
            spells::cast_triggered(m, a, BOOMERANG_STUN, Some(b));
            crate::game::map::death::kill(m, b);
            let dead = state(m);
            crate::game::map::death::resurrect(m, b, 1.0);
            kit::set_health(m, b, 100_000);
            combos.push(((disarm, stun), on_it, later, recovered, dead));
        }
        (armed, hurt, combos)
    })
    .await;
    let (armed, hurt, combos) = out;
    assert!(hurt, "the boomerang hurts");
    for ((disarm, stun), on_it, later, recovered, dead) in combos {
        assert_eq!(on_it.0, (disarm, stun), "{disarm} {stun}: on");
        if disarm {
            assert_ne!(on_it.1.0, armed.0, "fists' damage");
            assert_eq!(on_it.1.1, 2_000, "fists' speed");
        } else {
            assert_eq!(on_it.1, armed, "the weapon's");
        }
        assert_eq!(
            later,
            (disarm, false),
            "{disarm} {stun}: the stun gone at 3 s"
        );
        assert_eq!(recovered, ((false, false), armed), "all gone at 11 s");
        assert_eq!(dead, (false, false), "death takes both");
    }
}
