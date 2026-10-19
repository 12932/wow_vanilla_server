//! Phase 3's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/home_bind_test.exs`
//! (docs/parity/matrix.csv): an innkeeper's bind, where it is refused,
//! the Bind spell, and a hearthstone home across maps.

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::{CreatureTemplate, GameData, MapInfo};
use crate::game::harness::Client;
use crate::game::map::npc;
use crate::game::store::Bind;
use crate::game::tests_p3_combat::{HUMANS, START, setup_with, spawn, template};
use crate::game::tests_p3_entity::{on, spell};
use crate::game::tests_p3_player2_ammo::handled;

const INNKEEPER: u32 = 3740;
/// Bind on oneself, and a hearthstone's teleport home.
const BIND: u32 = 64_800;
const HEARTH: u32 = 64_801;

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.creature_templates.insert(
        INNKEEPER,
        CreatureTemplate {
            faction: HUMANS,
            npc_flags: npc::NPC_FLAG_INNKEEPER,
            ..template(INNKEEPER, 7)
        },
    );
    let effect = |effect: u32| crate::game::data::spells::SpellEffect {
        effect,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(BIND, spell(BIND, &[effect(11)]));
    d.spells.spells.insert(HEARTH, spell(HEARTH, &[effect(5)]));
    d
}

/// Whether `a` is told of a new home within a handling.
async fn told_home(a: &mut Client) -> Option<Vector3d> {
    handled(a).await;
    a.gets(std::time::Duration::from_millis(1), |m| match m {
        ServerOpcodeMessage::SMSG_BINDPOINTUPDATE(b) => Some(b.position),
        _ => None,
    })
    .await
}

/// Thistle Tea's "stores the player's location and terrain area and sends
/// both updates" and "replaces the old home": an innkeeper's bind keeps
/// where the player stands, in place of the home before, and tells the
/// client the new home and who bound it; and Thistle Tea's "bind returns
/// through the explicit player owner": Bind cast on oneself does the same.
#[tokio::test]
async fn an_innkeeper_and_the_bind_spell_make_where_one_stands_home() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let keeper = on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().row.state.bind = Some(Bind {
            map: 1,
            zone: 14,
            x: 1.0,
            y: 2.0,
            z: 3.0,
        });
        spawn(m, ga, INNKEEPER, 20, 3.0)
    })
    .await;
    a.sees(keeper).await;
    a.clear();
    a.send(CMSG_BINDER_ACTIVATE { guid: keeper }).await;
    let told = told_home(&mut a).await.expect("told the new home");
    assert!((told.x - START.0).abs() < 0.1, "{told:?}");
    let bound = a
        .gets(std::time::Duration::from_millis(1), |m| match m {
            ServerOpcodeMessage::SMSG_PLAYERBOUND(b) => Some(b.guid),
            _ => None,
        })
        .await;
    assert_eq!(bound, Some(keeper), "and by whom");
    let home = on(&map, move |m| m.players[&ga].row.state.bind)
        .await
        .unwrap();
    assert_eq!(home.map, 0, "the old home replaced");
    assert!((home.x - START.0).abs() < 0.1);
    // Elsewhere, Bind on oneself.
    a.heartbeat(START.0 + 20.0, START.1, START.2).await;
    handled(&mut a).await;
    a.clear();
    on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, ga, BIND, Some(ga));
    })
    .await;
    let told = told_home(&mut a).await.expect("bound by the spell");
    assert!((told.x - (START.0 + 20.0)).abs() < 0.1, "{told:?}");
}

/// Thistle Tea's "rejects dungeon and battleground maps even without an
/// instance id": an innkeeper in a dungeon's map binds no one.
#[tokio::test]
async fn an_innkeeper_in_a_dungeon_binds_no_one() {
    let mut d = world();
    d.maps.insert(
        0,
        MapInfo {
            id: 0,
            instance_type: 1,
            ..Default::default()
        },
    );
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    let keeper = on(&map, move |m| spawn(m, ga, INNKEEPER, 20, 3.0)).await;
    a.sees(keeper).await;
    // Home is where she started (mangos's `_LoadHomeBind`), and stays so.
    let start = on(&map, move |m| m.players[&ga].row.state.bind).await;
    assert!(start.is_some(), "a home from the start");
    a.clear();
    a.send(CMSG_BINDER_ACTIVATE { guid: keeper }).await;
    assert_eq!(told_home(&mut a).await, None);
    assert_eq!(
        on(&map, move |m| m.players[&ga].row.state.bind).await,
        start
    );
}

/// Thistle Tea's "home-target spells resolve to the retained home across
/// maps": a hearthstone's teleport with no place of its own takes the
/// player home, to another map.
#[tokio::test]
async fn a_hearthstone_takes_one_home_to_another_map() {
    let (realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().row.state.bind = Some(Bind {
            map: 1,
            zone: 14,
            x: 1_600.0,
            y: -4_400.0,
            z: 10.0,
        });
        crate::game::map::spells::cast_triggered(m, ga, HEARTH, Some(ga));
    })
    .await;
    let deadline = tokio::time::Instant::now() + crate::game::harness::WAIT;
    loop {
        let there = realm
            .registry
            .online(ga)
            .is_some_and(|o| o.map().key.map == 1);
        if there {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "never home");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}
