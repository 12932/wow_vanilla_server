//! wvs's own `.tele` places in Alterac Valley's world copy (map 30): just
//! outside each general's room, inside his keep (`data::tele`).

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::{GameData, Tele};
use crate::game::harness::{self, Client};
use crate::game::map::{Map as WorldMap, MapKey, ai, assistance, combat};

const KEEP: Vector3d = Vector3d {
    x: -1361.41,
    y: -245.86,
    z: 99.36,
};
const DUN_BALDAR: Vector3d = Vector3d {
    x: 690.4,
    y: -8.0,
    z: 50.62,
};
const DREK_THAR: (u32, Vector3d) = (
    11946,
    Vector3d {
        x: -1370.26,
        y: -219.70,
        z: 98.51,
    },
);
const VANNDAR: (u32, Vector3d) = (
    11948,
    Vector3d {
        x: 722.43,
        y: -10.99,
        z: 50.70,
    },
);

fn near(a: Vector3d, b: Vector3d) -> bool {
    (a.x - b.x).abs() < 0.05 && (a.y - b.y).abs() < 0.05 && (a.z - b.z).abs() < 0.05
}

/// The way `from` faces to look at `to`.
fn facing(from: Vector3d, to: Vector3d) -> f32 {
    (to.y - from.y)
        .atan2(to.x - from.x)
        .rem_euclid(std::f32::consts::TAU)
}

/// `c`'s `.tele` to a place on another map: where it lands, and facing.
async fn tele_away(c: &mut Client, line: &str) -> (Map, Vector3d, f32) {
    c.say(line).await;
    let w = c
        .wait_for("SMSG_NEW_WORLD", |m| match m {
            ServerOpcodeMessage::SMSG_NEW_WORLD(w) => Some((w.map, w.position, w.orientation)),
            _ => None,
        })
        .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(c.guid).await;
    w
}

/// Both places face their general, and are put after the table's rows,
/// a row of the same name keeping its own.
#[test]
fn the_places_face_their_generals_and_a_row_of_their_name_wins() {
    let mut tele = Vec::new();
    crate::game::data::tele::add_extras(&mut tele);
    let find = |t: &[Tele], n: &str| t.iter().find(|t| t.name == n).cloned().unwrap();
    for (name, at, general) in [
        ("AlteracValleyFrostwolfKeep", KEEP, DREK_THAR.1),
        ("AlteracValleyDunBaldar", DUN_BALDAR, VANNDAR.1),
    ] {
        let t = find(&tele, name);
        assert_eq!(t.map, 30, "{name}");
        assert!(
            near(
                Vector3d {
                    x: t.x,
                    y: t.y,
                    z: t.z
                },
                at
            ),
            "{name}"
        );
        assert!(
            (t.orientation - facing(at, general)).abs() < 0.01,
            "{name} faces {} not {}",
            t.orientation,
            facing(at, general)
        );
    }
    let mut table = vec![Tele {
        name: "alteracvalleydunbaldar".into(),
        map: 0,
        x: 1.0,
        y: 2.0,
        z: 3.0,
        orientation: 0.0,
    }];
    crate::game::data::tele::add_extras(&mut table);
    assert_eq!(table.len(), 2, "{table:?}");
    assert_eq!(table[0].map, 0, "the table's row stays");
    assert_eq!(table[1].name, "AlteracValleyFrostwolfKeep");
}

/// `.tele AlteracValleyFrostwolfKeep` and `.tele AlteracValleyDunBaldar`
/// put a GM there, on map 30, each by its exact name over the table's
/// longer ones that start with it; and a start of both lists them.
#[tokio::test]
async fn tele_takes_a_gm_outside_each_generals_room() {
    let mut d = GameData::empty();
    let place = |name: &str, x| Tele {
        name: name.into(),
        map: 30,
        x,
        y: -400.0,
        z: 50.0,
        orientation: 0.0,
    };
    d.tele = vec![
        place("AlteracValley", 0.0),
        place("AlteracValleyFrostwolfKeepRoof", 1.0),
        place("AlteracValleyDunBaldarBridge", 2.0),
    ];
    crate::game::data::tele::add_extras(&mut d.tele);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "AVTELE");
    let g = a.enter("Telia", Race::Human, Class::Warrior).await;
    let (map, at, o) = tele_away(&mut a, ".tele AlteracValleyFrostwolfKeep").await;
    assert_eq!(map, Map::AlteracValley);
    assert!(near(at, KEEP), "{at:?}");
    assert!((o - 1.90).abs() < 0.01, "{o}");
    let there = realm.registry.online(g).unwrap().map();
    let key = there.query(|m| m.key).await.unwrap();
    assert_eq!(key, MapKey::world(30));
    // On the same map: a teleport, not a new world. Any case.
    a.say(".tele alteracvalleydunbaldar").await;
    let at = a
        .wait_for("the teleport", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => {
                Some((t.info.position, t.info.orientation))
            }
            _ => None,
        })
        .await;
    assert!(near(at.0, DUN_BALDAR), "{at:?}");
    assert!((at.1 - 6.19).abs() < 0.01, "{at:?}");
    a.say(".tele AlteracValleyDunBaldarB").await;
    let at = a
        .wait_for("the teleport", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => Some(t.info.position.x),
            _ => None,
        })
        .await;
    assert!((at - 2.0).abs() < 0.01, "the longer name by its own start");
    a.say(".tele AlteracValleyDunB").await;
    let listed = a.system_message("Places").await;
    let names: Vec<&str> = listed.trim_start_matches("Places: ").split(", ").collect();
    assert!(
        names.contains(&"AlteracValleyDunBaldar")
            && names.contains(&"AlteracValleyDunBaldarBridge"),
        "{listed}"
    );
}

/// The faults with `spot` in Alterac Valley's world copy for `g` (a
/// player standing there): a way over the navmesh from it to the general
/// at `general` that doesn't reach him, or is out of `yards`; ground not
/// under it; any creature hostile to `g`, at its spawn, that has it in its
/// aggro reach (at `g`'s level) and in sight (the server's own
/// `ai::attack_distance` and `assistance::sees`, as its aggro scan asks
/// them); and the general himself, or any of his room (within 30 yd of
/// him), that sees it at all. Also how far by path.
fn faults(
    m: &mut WorldMap,
    g: Guid,
    spot: Vector3d,
    general: (u32, Vector3d),
    yards: std::ops::RangeInclusive<f32>,
) -> (Vec<String>, f32) {
    let mut out = Vec::new();
    let nav = m.nav.as_mut().expect("the valley's navmesh");
    let ground = nav.ground(spot.x, spot.y, spot.z);
    if ground.is_none_or(|z| (z - spot.z).abs() > 0.5) {
        out.push(format!("its ground is {ground:?}"));
    }
    let way = nav.path(spot, general.1).unwrap_or_default();
    let length: f32 = way.windows(2).map(|w| combat::dist(w[0], w[1])).sum();
    if way.last().is_none_or(|l| combat::dist(*l, general.1) > 1.5) {
        out.push(format!("no way to him: {way:?}"));
    } else if !yards.contains(&length) {
        out.push(format!("{length:.1} yd from him by path"));
    }
    let watchers: Vec<(Guid, u32, Vector3d)> = m
        .creatures
        .iter()
        .filter(|(_, c)| c.alive())
        .map(|(_, c)| (c.guid, c.entry, c.home))
        .collect();
    if !watchers.iter().any(|w| w.1 == general.0) {
        out.push(format!("no {} in the world copy", general.0));
    }
    for (c, entry, home) in watchers {
        if !combat::hostile(m, c, g) || !combat::can_attack(m, c, g) {
            continue;
        }
        let d = combat::dist(home, spot);
        let reach = ai::attack_distance(m, c, g);
        let in_room = combat::dist(home, general.1) <= 30.0;
        if (d <= reach || in_room) && assistance::sees(m, home, spot) {
            out.push(format!(
                "{entry} at {home:?} sees it, {d:.1} yd off (reach {reach:.1})"
            ));
        }
    }
    (out, length)
}

/// On the real valley and its navmesh, each place is a GM's `.tele`
/// away, on the ground, a way on to the general from it, and out of the
/// aggro of everything there hostile to the side that goes for him: at a
/// raider's level (60) and a new character's (1, the widest reach).
#[tokio::test]
#[ignore = "extended"]
async fn the_real_places_reach_the_generals_unseen() {
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(harness::real_data("nav"));
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let data = harness::load_real(&harness::real_data("db"));
    let realm = crate::game::Realm::new(cfg, store, startup, data, None);
    for (name, race, line, spot, general, yards) in [
        (
            "Telia",
            Race::Human,
            ".tele AlteracValleyFrostwolfKeep",
            KEEP,
            DREK_THAR,
            30.0..=45.0,
        ),
        (
            "Grunt",
            Race::Orc,
            ".tele AlteracValleyDunBaldar",
            DUN_BALDAR,
            VANNDAR,
            30.0..=45.0,
        ),
    ] {
        let mut a = Client::connect(&realm, name);
        let g = a.enter(name, race, Class::Warrior).await;
        let (map, at, _) = tele_away(&mut a, line).await;
        assert_eq!(map, Map::AlteracValley, "{line}");
        assert!(near(at, spot), "{line}: {at:?}");
        let there = realm.registry.online(g).unwrap().map();
        let (faults, length) = there
            .query(move |m| {
                let mut all = Vec::new();
                let mut length = 0.0;
                for level in [60, 1] {
                    if let Some(p) = m.players.get_mut(&g) {
                        p.fields
                            .set_u32(crate::game::fields::UNIT_FIELD_LEVEL, level);
                    }
                    let (f, l) = faults(m, g, spot, general, yards.clone());
                    all.extend(f.into_iter().map(|f| format!("at {level}: {f}")));
                    length = l;
                }
                (all, length)
            })
            .await
            .unwrap();
        eprintln!("{line}: {length:.1} yd from the general by path");
        assert!(faults.is_empty(), "{line}: {faults:#?}");
    }
}
