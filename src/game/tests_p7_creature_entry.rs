//! Round 7's creature tests, ported from Thistle Tea's
//! `test/game/core/creature` (docs/parity/matrix.csv): a creature's entry
//! changed by a script (`update_entry`), its speeds told, its auras and
//! state through the change, its weapons and spell list after it, and its
//! flight; a respawn bringing the old entry back.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::items::ItemTemplate;
use crate::game::data::scripts::ScriptStep;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::auras::{self, AURA_MOD_HASTE, AURA_MOD_RESISTANCE, AURA_MOD_SCALE};
use crate::game::map::creature_motion::FLIGHT_FLAGS;
use crate::game::map::{Map, MapHandle, scripts};
use crate::game::tests_p3_combat::{START, aura_of};
use crate::game::tests_p3_entity::{aura_effect, on, spell, step};

/// The spawned creature, and what it becomes.
const OLD: u32 = 4_710;
const NEW: u32 = 4_711;
const OLD_SPAWN: u32 = 4_710;
/// A second spawn of the old entry, with an addon of its own.
const ADDON_SPAWN: u32 = 4_711;
/// Its template's aura, the new template's, and its addon's.
const OLD_AURA: u32 = 94_710;
const NEW_AURA: u32 = 94_711;
const ADDON_AURA: u32 = 94_712;
/// An aura from someone else.
const OTHER_AURA: u32 = 94_713;
const HASTE: u32 = 94_714;
const GROW: u32 = 94_715;
const BLADE: u32 = 5_470;
const AXE: u32 = 5_471;

fn template(entry: u32, name: &str) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        display_scales: [1.0, 0.0, 0.0, 0.0],
        level_min: 10,
        level_max: 10,
        faction: 35,
        unit_class: 1,
        health_max: 500,
        speed_walk: 1.0,
        speed_run: 1.0,
        base_attack_time: 2000,
        ..Default::default()
    }
}

fn spawn(guid: u32, entry: u32, dx: f32) -> CreatureSpawn {
    CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 60,
        respawn_max: 60,
        ..Default::default()
    }
}

/// The old entry, a flyer with an aura of its own and an axe; the new, on
/// the ground, twice as big and as fast, slower to swing, with its own
/// aura and blade.
fn world() -> GameData {
    let mut d = GameData::empty();
    let mut old = template(OLD, "Old");
    old.auras = vec![OLD_AURA];
    old.inhabit_type = crate::game::map::creature_motion::INHABIT_AIR;
    old.equipment_id = 71;
    d.creature_templates.insert(OLD, old);
    let mut new = template(NEW, "New");
    new.display_scales = [2.0, 0.0, 0.0, 0.0];
    new.speed_walk = 2.0;
    new.speed_run = 2.0;
    new.base_attack_time = 3000;
    new.auras = vec![NEW_AURA];
    new.equipment_id = 72;
    d.creature_templates.insert(NEW, new);
    d.creature_spawns.insert(
        0,
        vec![spawn(OLD_SPAWN, OLD, 3.0), spawn(ADDON_SPAWN, OLD, 6.0)],
    );
    d.creature_addons.insert(
        ADDON_SPAWN,
        CreatureAddon {
            auras: vec![ADDON_AURA],
            mount_display_id: -1,
            ..Default::default()
        },
    );
    for id in [OLD_AURA, NEW_AURA, ADDON_AURA, OTHER_AURA] {
        d.spells
            .spells
            .insert(id, spell(id, &[aura_effect(AURA_MOD_RESISTANCE, 5, 1, 1)]));
    }
    for (entry, display) in [(AXE, 5_480), (BLADE, 5_481)] {
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
    d.creature_equipment.insert(71, [AXE, 0, 0]);
    d.creature_equipment.insert(72, [BLADE, 0, 0]);
    d
}

fn old() -> Guid {
    guid::unit(OLD, OLD_SPAWN)
}

fn with_addon() -> Guid {
    guid::unit(OLD, ADDON_SPAWN)
}

async fn player(realm: &Arc<Realm>) -> (Client, MapHandle) {
    let mut c = Client::connect(realm, "A");
    let me = c.enter("Alice", Race::Human, Class::Warrior).await;
    c.sees(me).await;
    let map = realm.registry.online(me).unwrap().map();
    (c, map)
}

fn become_new(m: &mut Map, g: Guid) -> bool {
    scripts::execute(m, &step(27, [NEW, 0, 0, 0]), g, g)
}

fn holds(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

/// Thistle Tea's `CreatureEntry.apply` ("replaces archetype fields" and
/// "retains and recomputes unrelated aura effects"), with mangos's
/// `InitEntry` speeds: the new entry's walk and run are its own, and those
/// who see it are told both; its auras stay and are worked through its
/// new values, its haste on its new swing and its growth on its new size,
/// and as they go they leave the new entry's own.
#[tokio::test]
async fn an_entry_change_tells_its_speeds_and_works_its_auras_through_its_new_values() {
    let realm = harness::realm_with(world());
    let (mut a, map) = player(&realm).await;
    let g = old();
    a.sees(g).await;
    a.clear();
    let changed = on(&map, move |m| {
        aura_of(m, g, HASTE, AURA_MOD_HASTE, 0, 50);
        aura_of(m, g, GROW, AURA_MOD_SCALE, 0, 50);
        assert!(become_new(m, g));
        let c = m.creature(g).unwrap();
        (
            c.entry,
            c.fields.get_f32(OBJECT_FIELD_SCALE_X),
            c.fields.get_u32(UNIT_FIELD_BASEATTACKTIME),
            c.speeds.run,
            c.speeds.walk,
            holds(m, g, HASTE) && holds(m, g, GROW),
        )
    })
    .await;
    assert_eq!(changed.0, NEW);
    assert!((changed.1 - 3.0).abs() < 0.001, "twice, grown half again");
    assert_eq!(changed.2, 2000, "3000 hastened by half");
    assert!((changed.3 - 14.0).abs() < 0.01, "{}", changed.3);
    assert!((changed.4 - 5.0).abs() < 0.01, "{}", changed.4);
    assert!(changed.5, "its auras kept");
    let run = a
        .wait_for("the new run speed", |m| match m {
            ServerOpcodeMessage::SMSG_SPLINE_SET_RUN_SPEED(s) if s.guid == g => Some(s.speed),
            _ => None,
        })
        .await;
    assert!((run - 14.0).abs() < 0.01, "{run}");
    let walk = a
        .wait_for("the new walk speed", |m| match m {
            ServerOpcodeMessage::SMSG_SPLINE_SET_WALK_SPEED(s) if s.guid == g => Some(s.speed),
            _ => None,
        })
        .await;
    assert!((walk - 5.0).abs() < 0.01, "{walk}");
    let gone = on(&map, move |m| {
        auras::remove_spell(m, g, HASTE, Some(g));
        auras::remove_spell(m, g, GROW, Some(g));
        let f = m.unit_fields(g).unwrap();
        (
            f.get_f32(OBJECT_FIELD_SCALE_X),
            f.get_u32(UNIT_FIELD_BASEATTACKTIME),
        )
    })
    .await;
    assert!((gone.0 - 2.0).abs() < 0.001, "the new size, {}", gone.0);
    assert_eq!(gone.1, 3000, "the new swing");
}

/// mangos's `UpdateEntry`, where Thistle Tea differs: the creature keeps
/// the auras it has (the template's it spawned with, an addon's, another's
/// on it), its new template's own not put on (mangos loads an addon only
/// as a creature spawns); and its react state, its AI kept (Thistle Tea
/// gives the new entry its default). A respawn brings the old entry back,
/// with its own aura and the react state a script left it.
#[tokio::test]
async fn an_entry_change_keeps_its_auras_and_react_state_and_a_respawn_its_old_self() {
    use crate::game::map::assistance::{Reaction, reaction};
    let realm = harness::realm_with(world());
    let (mut a, map) = player(&realm).await;
    let me = a.guid;
    let (g, addon) = (old(), with_addon());
    a.sees(addon).await;
    let out = on(&map, move |m| {
        let other = m.realm.data.spells.get(OTHER_AURA).cloned().unwrap();
        let _ = auras::apply_spell(m, me, g, &other, &[(0, 5)], auras::Context::default());
        scripts::execute(m, &step(59, [0, 0, 0, 0]), g, g);
        let before = (holds(m, g, OLD_AURA), holds(m, addon, ADDON_AURA));
        become_new(m, g);
        become_new(m, addon);
        let after = (
            holds(m, g, OLD_AURA),
            holds(m, g, OTHER_AURA),
            holds(m, g, NEW_AURA),
            holds(m, addon, ADDON_AURA),
            holds(m, addon, NEW_AURA),
            reaction(m, g),
        );
        crate::game::map::combat::deal_damage(m, me, g, 1_000_000);
        crate::game::map::ai::respawn_now(m, g);
        let back = scripts::creature_of_spawn(m, OLD_SPAWN).unwrap();
        let c = m.creature(back).unwrap();
        (
            before,
            after,
            (
                c.entry,
                holds(m, back, OLD_AURA),
                holds(m, back, NEW_AURA),
                reaction(m, back),
            ),
        )
    })
    .await;
    assert_eq!(
        out.0,
        (true, true),
        "spawned with its template's and its addon's"
    );
    assert_eq!(
        out.1,
        (true, true, false, true, false, Reaction::Passive),
        "(template's, another's, new template's; addon's, new template's; react state)"
    );
    assert_eq!(out.2, (OLD, true, false, Reaction::Passive), "respawned");
}

/// Thistle Tea's "equipment reset uses the current archetype" (mangos's
/// `SCRIPT_COMMAND_SET_EQUIPMENT`'s reset to the template's): changed, the
/// creature holds the new entry's weapon; emptied by a script and reset, it
/// is the new entry's again; respawned, the old entry's.
#[tokio::test]
async fn a_reset_of_weapons_is_to_the_entry_it_is_now() {
    let realm = harness::realm_with(world());
    let (mut a, map) = player(&realm).await;
    let me = a.guid;
    let g = old();
    a.sees(g).await;
    let out = on(&map, move |m| {
        let main = |m: &Map, g: Guid| {
            m.unit_fields(g)
                .map_or(0, |f| f.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY))
        };
        let spawned = main(m, g);
        become_new(m, g);
        let changed = main(m, g);
        let empty = ScriptStep {
            dataint: [0, 0, 0, 0],
            ..step(19, [0; 4])
        };
        scripts::execute(m, &empty, g, g);
        let emptied = main(m, g);
        scripts::execute(m, &step(19, [1, 0, 0, 0]), g, g);
        let reset = main(m, g);
        crate::game::map::combat::deal_damage(m, me, g, 1_000_000);
        crate::game::map::ai::respawn_now(m, g);
        let back = scripts::creature_of_spawn(m, OLD_SPAWN).unwrap();
        (spawned, changed, emptied, reset, main(m, back))
    })
    .await;
    assert_eq!(out, (5_480, 5_481, 0, 5_481, 5_480));
}

/// Thistle Tea's "changes entry before the next same-batch command and
/// resets spell timers" (mangos's `SetSpellList` at `UpdateEntry`): a new
/// entry with a spell list of its own takes it, the list a script gave the
/// old one and its timers gone; the script's next step works on the new
/// entry.
#[tokio::test]
async fn a_new_entrys_spell_list_replaces_a_scripts_and_its_timers() {
    let mut d = world();
    d.creature_templates.get_mut(&NEW).unwrap().spell_list_id = 4_711;
    let realm = harness::realm_with(d);
    let (mut a, map) = player(&realm).await;
    let g = old();
    a.sees(g).await;
    let out = on(&map, move |m| {
        if let Some(s) = crate::game::map::event_ai::scripted_mut(m, g) {
            s.spell_list = Some(9_999);
        }
        if let Some(c) = m.creature_mut(g) {
            c.caster.spell_list = Some(Box::default());
        }
        let steps = [step(27, [NEW, 0, 0, 0]), step(22, [113, 0, 0, 0])];
        let ran = steps.iter().all(|s| scripts::execute(m, s, g, g));
        let c = m.creature(g).unwrap();
        (
            ran,
            c.entry,
            c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
            c.scripted.as_deref().and_then(|s| s.spell_list),
            c.caster.spell_list.is_none(),
            crate::game::map::spells::lists::base_list(m, g),
        )
    })
    .await;
    assert_eq!(out, (true, NEW, 113, None, true, 4_711));
}

/// Thistle Tea's "entry changes reconcile flight and respawn restores the
/// original template": a flyer that becomes an entry on the ground comes
/// down out of flight; respawned, it flies again.
#[tokio::test]
async fn a_flyer_made_a_walker_lands_and_flies_again_at_its_respawn() {
    let realm = harness::realm_with(world());
    let (mut a, map) = player(&realm).await;
    let me = a.guid;
    let g = old();
    a.sees(g).await;
    let out = on(&map, move |m| {
        let flies = |m: &Map, g: Guid| m.creature(g).unwrap().move_flags & FLIGHT_FLAGS;
        let spawned = flies(m, g);
        become_new(m, g);
        let changed = flies(m, g);
        crate::game::map::combat::deal_damage(m, me, g, 1_000_000);
        crate::game::map::ai::respawn_now(m, g);
        let back = scripts::creature_of_spawn(m, OLD_SPAWN).unwrap();
        (spawned, changed, flies(m, back))
    })
    .await;
    assert_eq!(out, (FLIGHT_FLAGS, 0, FLIGHT_FLAGS));
}
