//! Round 8's creature tests against Thistle Tea's
//! `test/game/core/creature/creature_event_test.exs` (docs/parity/matrix.csv):
//! a transform's looks and weapons over a game event's.

use std::sync::Arc;

use ahash::AHashSet;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::items::ItemTemplate;
use crate::game::data::world::EventCreature;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::auras::{self, AURA_TRANSFORM};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::{START, data, setup_with, template};
use crate::game::tests_p3_entity::{aura_effect, on, spell};

const ORIGINAL: u32 = 4_760;
const BEAST: u32 = 4_761;
const SPAWN: u32 = 4_760;
const EVENT: u32 = 2;
const EVENT_MODEL: u32 = 300;
const BEAST_MODEL: u32 = 905;
const AXE: u32 = 5_492;
const CLAWS: u32 = 5_493;
const TRANSFORM: u32 = 94_760;

/// A creature with an axe, a beast with claws it may be made, and an event
/// that gives the creature a model of its own.
fn world() -> GameData {
    let mut d = data();
    let mut original = template(ORIGINAL, 7);
    original.faction = 35;
    original.display_scales = [1.0, 0.0, 0.0, 0.0];
    original.equipment_id = 76;
    d.creature_templates.insert(ORIGINAL, original);
    let mut beast = template(BEAST, 1);
    beast.display_ids = [BEAST_MODEL, 0, 0, 0];
    beast.display_scales = [1.0, 0.0, 0.0, 0.0];
    beast.equipment_id = 77;
    d.creature_templates.insert(BEAST, beast);
    for (entry, display) in [(AXE, 5_510), (CLAWS, 5_511)] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                name: format!("Weapon {entry}"),
                display_id: display,
                ..Default::default()
            },
        );
    }
    d.creature_equipment.insert(76, [AXE, 0, 0]);
    d.creature_equipment.insert(77, [CLAWS, 0, 0]);
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: SPAWN,
            entries: vec![ORIGINAL],
            map: 0,
            x: START.0 + 3.0,
            y: START.1 + 5.0,
            z: START.2,
            respawn_min: 60,
            respawn_max: 60,
            ..Default::default()
        }],
    );
    d.world.event_creatures.insert(
        SPAWN,
        vec![EventCreature {
            event: EVENT,
            display: EVENT_MODEL,
            ..Default::default()
        }],
    );
    d.spells.spells.insert(
        TRANSFORM,
        spell(
            TRANSFORM,
            &[aura_effect(AURA_TRANSFORM, 0, BEAST as i32, 1)],
        ),
    );
    d
}

async fn events(realm: &Arc<Realm>, map: &MapHandle, on_now: &[u32]) {
    let set: AHashSet<u32> = on_now.iter().copied().collect();
    *realm.social.events.lock() = set.clone();
    on(map, move |m| crate::game::events::apply(m, &set)).await;
}

/// Its look and its main hand's.
fn look(m: &Map, g: Guid) -> (u32, u32) {
    let f = m.unit_fields(g).unwrap();
    (
        f.get_u32(UNIT_FIELD_DISPLAYID),
        f.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY),
    )
}

/// Thistle Tea's "preserves transform equipment across event changes"
/// (mangos's `HandleAuraTransform` loading the transform creature's
/// equipment, which an event that brings none leaves be): a creature made
/// a beast shows the beast and its claws, and an event's model coming
/// meanwhile changes neither; the transform gone, it shows the event's
/// model and its own axe; the event over, its own look.
#[tokio::test]
async fn a_transforms_look_and_weapons_stay_over_an_events_and_go_back_to_them() {
    let (realm, mut a, _ga, map) = setup_with(world(), Class::Warrior).await;
    let g = guid::unit(ORIGINAL, SPAWN);
    a.sees(g).await;
    let own = on(&map, move |m| look(m, g)).await;
    let made = on(&map, move |m| {
        let s = m.realm.data.spells.get(TRANSFORM).cloned().unwrap();
        let _ = auras::apply_spell(m, g, g, &s, &[(0, 0)], auras::Context::default());
        look(m, g)
    })
    .await;
    events(&realm, &map, &[EVENT]).await;
    let through = on(&map, move |m| look(m, g)).await;
    let undone = on(&map, move |m| {
        auras::remove_spell(m, g, TRANSFORM, None);
        look(m, g)
    })
    .await;
    events(&realm, &map, &[]).await;
    let back = on(&map, move |m| look(m, g)).await;
    assert_eq!(own, (903, 5_510), "itself, with its axe");
    assert_eq!(made, (BEAST_MODEL, 5_511), "the beast, with its claws");
    assert_eq!(through, made, "the event's model under the transform");
    assert_eq!(
        undone,
        (EVENT_MODEL, 5_510),
        "the event's model, its own axe"
    );
    assert_eq!(back, own);
}
