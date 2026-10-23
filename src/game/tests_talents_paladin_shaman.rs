//! The paladin's and shaman's talents the Phase 1 audits found wvs left
//! out, on the real data: Divine Favor's spells and charge, Improved Seal
//! of Righteousness on the seal, Improved Lay on Hands' armor; the
//! totems' party (a raid's subgroup) and their shaman's modifiers
//! (Enhancing, Guardian and Restorative Totems, Earth's Grasp), Elemental
//! Weapons on Rockbiter and Flametongue.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::{self as kit, cast_fresh};
use crate::game::Realm;
use crate::game::map::spells::Targets;
use crate::game::map::{Map, MapHandle};
use crate::game::social::groups;

const STRENGTH_OF_EARTH_5: u32 = 25361;
const STRENGTH_OF_EARTH_AURA_5: u32 = 25362;
/// The four totems a shaman carries.
const TOTEMS: [u32; 4] = [5175, 5176, 5177, 5178];

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

/// A level-60 shaman with their totems, knowing `talents`.
fn shaman(m: &mut Map, g: Guid, talents: &[u32]) {
    kit::ready(m, g, 60, talents);
    for t in TOTEMS {
        let _ = crate::game::map::inventory::add_item(m, g, t, 1, false);
    }
}

/// Totems' turns, three of them, a second and more apart.
fn totems_think(m: &mut Map) {
    for _ in 0..3 {
        kit::pass(m, 1_100);
        crate::game::map::pets::update(m);
    }
}

fn realm() -> Option<Arc<Realm>> {
    kit::have_data().then(kit::realm)
}

/// A totem's aura reaches its shaman's subgroup in a raid, not the other
/// subgroups (mangos's `AREA_AURA_PARTY`).
#[tokio::test]
#[ignore = "extended"]
async fn a_totems_aura_reaches_its_shamans_subgroup_not_the_whole_raid() {
    let Some(realm) = realm() else { return };
    let (_a, s, map) = kit::player(&realm, "TPSTG1", "Totema", Race::Orc, Class::Shaman).await;
    let (_b, near, _) = kit::player(&realm, "TPSTG2", "Totemb", Race::Orc, Class::Warrior).await;
    let (_c, far, _) = kit::player(&realm, "TPSTG3", "Totemc", Race::Orc, Class::Warrior).await;
    let lo = realm.registry.online(s).unwrap();
    for g in [near, far] {
        let o = realm.registry.online(g).unwrap();
        assert!(groups::join_directly(&realm, &lo, &o));
    }
    groups::convert_to_raid(&realm, &lo);
    groups::change_subgroup(&realm, &lo, "Totemc", 1);
    let out = on(&map, move |m| {
        shaman(m, s, &[]);
        let at_shaman = m.players[&s].position();
        for g in [near, far] {
            kit::ready(m, g, 60, &[]);
            super::tests_p6_wspell::put(m, g, at_shaman);
        }
        assert!(cast_fresh(m, s, STRENGTH_OF_EARTH_5, Targets::default()));
        totems_think(m);
        [s, near, far].map(|g| kit::has(m, g, STRENGTH_OF_EARTH_AURA_5))
    })
    .await;
    assert_eq!(out, [true, true, false], "shaman, their subgroup, another");
}
