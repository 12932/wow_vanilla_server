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

/// The option of `menu` with id `id`.
fn option(d: &GameData, menu: u32, id: u32) -> &crate::game::data::npc::GossipOption {
    d.npc.gossip_menus[&menu]
        .options
        .iter()
        .find(|o| o.id == id)
        .unwrap_or_else(|| panic!("menu {menu} option {id}"))
}

/// Thistle Tea's `gossip_vmangos_test.exs` loading on the real data: a
/// hunter trainer's class menu leading to the talent reset (16), a pet
/// trainer's untraining (17), Innkeeper Farley's home (8), Moonglade's
/// flights by side, Olivia Burnside's deposit box, a battlemaster's menu
/// with none of its own (the default's, played out in
/// `tests_p3_loader_play`), the Tamed Kodo's greeting script, and
/// Tharnariun's lost trap option with its condition and item.
#[test]
fn the_real_gossip_menus_load_as_thistle_tea_reads_them() {
    let Some(d) = real() else {
        return;
    };
    let template = |e: u32| d.creature_template(e).unwrap();
    // A hunter's class trainer, and a pet trainer that isn't one.
    let (hunter, pets) = (template(5515), template(10_090));
    assert_eq!((hunter.trainer_type, hunter.trainer_class), (0, 3));
    assert_eq!(pets.trainer_type, 3);
    assert_eq!(option(d, hunter.gossip_menu_id, 1).action_menu, 4461);
    let reset = option(d, 4461, 0);
    assert_eq!((reset.kind, reset.npc_flag), (16, 0x10));
    let untrain = d.npc.gossip_menus[&pets.gossip_menu_id]
        .options
        .iter()
        .find(|o| o.kind == 17)
        .unwrap();
    assert_eq!(untrain.npc_flag, 0x10);
    let farley = template(295);
    let home = d.npc.gossip_menus[&farley.gossip_menu_id]
        .options
        .iter()
        .find(|o| o.kind == 8)
        .unwrap();
    assert_eq!(home.npc_flag, 0x80);
    // Moonglade: each side's flight, a script's SEND_TAXI_PATH.
    for (menu, condition, path) in [(4041, 232, 315), (4042, 206, 316)] {
        let o = option(d, menu, 0);
        assert_eq!(o.condition, condition, "{menu}");
        let steps = &d.scripts.gossip[&o.script];
        assert_eq!(
            (steps[0].command, steps[0].datalong[0]),
            (30, path),
            "{menu}"
        );
    }
    let bank = option(d, 699, 0);
    assert_eq!(template(2455).gossip_menu_id, 699);
    assert_eq!(
        (bank.kind, bank.icon, bank.text.as_str()),
        (9, 6, "I would like to check my deposit box.")
    );
    let battlemaster = template(14_981);
    assert_eq!(battlemaster.gossip_menu_id, 6460);
    assert!(
        d.npc
            .gossip_menus
            .get(&6460)
            .is_none_or(|m| m.options.is_empty())
    );
    let join = option(d, 0, 10);
    assert_eq!(
        (join.kind, join.npc_flag, join.icon, join.text.as_str()),
        (12, 2048, 9, "I wish to join the battle!")
    );
    assert!(battlemaster.npc_flags & 2048 != 0);
    // The Tamed Kodo's greeting: credit, then its aura off, on the player.
    let kodo = template(11_627).gossip_menu_id;
    let steps = &d.scripts.gossip[&d.scripts.gossip_menu_script[&kodo]];
    let kinds: Vec<(u32, u32)> = steps.iter().map(|s| (s.command, s.data_flags)).collect();
    assert_eq!(kinds, vec![(83, 1), (14, 1)]);
    assert!(steps[0].condition != 0);
    assert_eq!(steps[1].datalong[0], 18_172);
    // Tharnariun: another trap, to one on the quest who has lost it.
    let trap = option(d, 269, 0);
    assert_eq!(template(3701).gossip_menu_id, 269);
    assert_eq!(
        (
            trap.kind,
            trap.action_menu,
            trap.text.as_str(),
            trap.condition
        ),
        (
            1,
            -1,
            "Tharnariun, I have lost the trap. Could you please give me another?",
            1094
        )
    );
    let give = &d.scripts.gossip[&trap.script][0];
    assert_eq!(
        (give.command, give.datalong[0], give.datalong[1]),
        (17, 7586, 1)
    );
}
