//! Round 7's creature tests, ported from Thistle Tea's
//! `test/game/core/creature/creature_event_test.exs` (docs/parity/matrix.csv):
//! a game event's looks on a spawn's creature, taken where it stands and
//! given back (its entry, model, size and weapons, with the spells that go
//! with them), a corpse left to its respawn, and the lowest event of those
//! on.

use std::sync::Arc;

use ahash::AHashSet;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::CreatureSpell;
use crate::game::data::world::EventCreature;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::auras::{self, AURA_MOD_HASTE, AURA_MOD_RESISTANCE, AURA_MOD_SCALE};
use crate::game::map::{Map, MapHandle, scripts};
use crate::game::tests_p3_combat::{START, aura_of, data, setup_with, template};
use crate::game::tests_p3_entity::{aura_effect, on, spell, step};

const ORIGINAL: u32 = 4_750;
const FORM: u32 = 4_751;
const FORM_LIST: u32 = 4_751;
/// Spawns: one an event makes another entry; one an event arms; one an
/// event gives a model and its spells; one two events dress.
const CHANGED: u32 = 4_750;
const ARMED: u32 = 4_751;
const MODELLED: u32 = 4_752;
const DRESSED: u32 = 4_753;
const AXE: u32 = 5_490;
const BLADE: u32 = 5_491;
const MODEL: u32 = 300;
const START_SPELL: u32 = 94_750;
const END_SPELL: u32 = 94_751;
const GROW: u32 = 94_752;
const FORM_SPELL: u32 = 94_753;

fn spawn(guid: u32, dx: f32) -> CreatureSpawn {
    CreatureSpawn {
        guid,
        entries: vec![ORIGINAL],
        map: 0,
        x: START.0 + dx,
        y: START.1 + 5.0,
        z: START.2,
        respawn_min: 60,
        respawn_max: 60,
        ..Default::default()
    }
}

fn world() -> GameData {
    let mut d = data();
    let mut original = template(ORIGINAL, 7);
    original.faction = 35;
    original.level_min = 10;
    original.level_max = 10;
    original.health_max = 100;
    original.display_scales = [1.0, 0.0, 0.0, 0.0];
    original.equipment_id = 74;
    d.creature_templates.insert(ORIGINAL, original);
    let mut form = template(FORM, 7);
    form.faction = 35;
    form.level_min = 30;
    form.level_max = 30;
    form.health_max = 400;
    form.display_ids = [904, 0, 0, 0];
    form.display_scales = [1.0, 0.0, 0.0, 0.0];
    form.spell_list_id = FORM_LIST;
    d.creature_templates.insert(FORM, form);
    d.spells.creature_spells.insert(
        FORM_LIST,
        vec![CreatureSpell {
            spell: FORM_SPELL,
            probability: 100,
            ..Default::default()
        }],
    );
    d.creature_spawns.insert(
        0,
        vec![
            spawn(CHANGED, 3.0),
            spawn(ARMED, 6.0),
            spawn(MODELLED, 9.0),
            spawn(DRESSED, 12.0),
        ],
    );
    let looks = |event: u32| EventCreature {
        event,
        ..Default::default()
    };
    d.world.event_creatures.insert(
        CHANGED,
        vec![EventCreature {
            entry: FORM,
            ..looks(49)
        }],
    );
    d.world.event_creatures.insert(
        ARMED,
        vec![EventCreature {
            equipment: 75,
            ..looks(27)
        }],
    );
    d.world.event_creatures.insert(
        MODELLED,
        vec![EventCreature {
            display: MODEL,
            spell_start: START_SPELL,
            spell_end: END_SPELL,
            ..looks(2)
        }],
    );
    d.world.event_creatures.insert(
        DRESSED,
        vec![
            EventCreature {
                display: 1_049,
                ..looks(49)
            },
            EventCreature {
                display: 1_002,
                ..looks(2)
            },
        ],
    );
    d.display_info.insert(
        MODEL,
        DisplayInfo {
            scale: 2.0,
            bounding_radius: 0.5,
            combat_reach: 2.0,
            ..Default::default()
        },
    );
    for (entry, display) in [(AXE, 5_500), (BLADE, 5_501)] {
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
    d.creature_equipment.insert(74, [AXE, 0, 0]);
    d.creature_equipment.insert(75, [BLADE, 0, 0]);
    d.spells.spells.insert(
        START_SPELL,
        spell(START_SPELL, &[aura_effect(AURA_MOD_HASTE, 50, 0, 1)]),
    );
    d.spells.spells.insert(
        END_SPELL,
        spell(END_SPELL, &[aura_effect(AURA_MOD_RESISTANCE, 5, 1, 1)]),
    );
    d.spells.spells.insert(
        FORM_SPELL,
        spell(FORM_SPELL, &[aura_effect(AURA_MOD_RESISTANCE, 5, 1, 1)]),
    );
    d
}

/// The events on `on`, as the realm works them out, brought in on `map`.
async fn events(realm: &Arc<Realm>, map: &MapHandle, on: &[u32]) {
    let on: AHashSet<u32> = on.iter().copied().collect();
    *realm.social.events.lock() = on.clone();
    on_map(map, move |m| crate::game::events::apply(m, &on)).await;
}

async fn on_map<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    on(map, f).await
}

fn of(spawn: u32) -> Guid {
    guid::unit(ORIGINAL, spawn)
}

fn creature_at(m: &Map, spawn: u32) -> Guid {
    scripts::creature_of_spawn(m, spawn).expect("the spawn's creature")
}

fn holds(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

/// Thistle Tea's "changes a living actor and restores its original
/// archetype" and "restores an empty spell list and resets changed spell
/// timers" (mangos's `UpdateEntry` from `UpdateCreatureData`): an event
/// makes a hurt creature in a fight its entry where it stands, the same
/// creature, its fight and its share of health kept, with the entry's
/// spell list, its timers anew; over, it is itself again, its share of
/// health still, and no list.
#[tokio::test]
async fn a_living_creature_takes_an_events_entry_where_it_stands_and_its_own_back() {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let g = of(CHANGED);
    a.sees(g).await;
    on_map(&map, move |m| {
        crate::game::tests_guards::hardy(m, ga);
        m.unit_fields_mut(g).unwrap().set_u32(UNIT_FIELD_HEALTH, 25);
        crate::game::map::ai::attacked(m, g, ga);
        m.creature_mut(g).unwrap().caster.spell_list = Some(Box::default());
    })
    .await;
    let read = move |m: &mut Map| {
        let c = m.creature(creature_at(m, CHANGED)).unwrap();
        (
            c.guid,
            c.entry,
            c.fields.get_u32(UNIT_FIELD_HEALTH),
            c.fields.get_u32(UNIT_FIELD_MAXHEALTH),
            c.threat.iter().any(|(t, _)| *t == ga) && c.combat.attacking,
            c.caster.spell_list.is_none(),
            crate::game::map::spells::lists::base_list(m, c.guid),
        )
    };
    events(&realm, &map, &[49]).await;
    let changed = on_map(&map, read).await;
    assert_eq!(changed, (g, FORM, 100, 400, true, true, FORM_LIST));
    events(&realm, &map, &[]).await;
    let back = on_map(&map, read).await;
    assert_eq!(back, (g, ORIGINAL, 25, 100, true, true, 0));
}

/// Thistle Tea's "does not reroll unchanged events or disturb unrelated
/// script changes": the events changing but not its own, a creature in an
/// event's looks is left as it is, what a script did to it with it.
#[tokio::test]
async fn an_event_left_on_leaves_a_creature_as_a_script_left_it() {
    let (realm, mut a, _ga, map) = setup_with(world(), Class::Warrior).await;
    a.sees(of(CHANGED)).await;
    events(&realm, &map, &[49]).await;
    let before = on_map(&map, |m| {
        let g = creature_at(m, CHANGED);
        scripts::execute(m, &step(28, [1, 0, 0, 0]), g, g);
        g
    })
    .await;
    events(&realm, &map, &[49, 7]).await;
    let after = on_map(&map, |m| {
        let g = creature_at(m, CHANGED);
        let c = m.creature(g).unwrap();
        (g, c.entry, c.fields.get_byte(UNIT_FIELD_BYTES_1, 0))
    })
    .await;
    assert_eq!(after, (before, FORM, 1), "the same, still sitting");
}

/// Thistle Tea's "changes equipment on the same entry and makes it the
/// script reset default", where mangos-classic differs for the reset: an
/// event's weapons go on the creature, its entry its own; a script's reset
/// gives it its template's (mangos's `SCRIPT_COMMAND_SET_EQUIPMENT` loads
/// the template's `EquipmentTemplateId`, not the event's); the event over,
/// its own again.
#[tokio::test]
async fn an_events_weapons_go_on_and_a_reset_is_to_the_templates() {
    let (realm, mut a, _ga, map) = setup_with(world(), Class::Warrior).await;
    a.sees(of(ARMED)).await;
    let main = |m: &mut Map| {
        let g = creature_at(m, ARMED);
        (
            m.creature(g).unwrap().entry,
            m.unit_fields(g)
                .unwrap()
                .get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY),
        )
    };
    let own = on_map(&map, main).await;
    events(&realm, &map, &[27]).await;
    let armed = on_map(&map, main).await;
    let reset = on_map(&map, move |m| {
        let g = creature_at(m, ARMED);
        scripts::execute(m, &step(19, [1, 0, 0, 0]), g, g);
        main(m)
    })
    .await;
    events(&realm, &map, &[]).await;
    let back = on_map(&map, main).await;
    assert_eq!(own, (ORIGINAL, 5_500));
    assert_eq!(armed, (ORIGINAL, 5_501), "the event's, its entry its own");
    assert_eq!(reset, (ORIGINAL, 5_500), "reset to its template's");
    assert_eq!(back, (ORIGINAL, 5_500));
}

/// Thistle Tea's "recomputes explicit event models through active scale
/// modifiers" and "removes opposite auras and emits both event spell
/// transitions" (mangos's `ApplyGameEventSpells`): an event's model goes on
/// at its own size, grown by the creature's scale aura, its end spell's
/// aura taken off and its start spell cast; the aura gone, the model's own
/// size; the event over, its own look and size, the start's aura off and
/// the end spell cast. Respawned in the event, it casts its start again.
#[tokio::test]
async fn an_events_model_goes_on_at_its_size_with_its_spells_and_comes_off() {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let g = of(MODELLED);
    a.sees(g).await;
    let read = move |m: &mut Map| {
        let f = m.unit_fields(g).unwrap();
        (
            f.get_u32(UNIT_FIELD_DISPLAYID),
            f.get_f32(OBJECT_FIELD_SCALE_X),
            holds(m, g, START_SPELL),
            holds(m, g, END_SPELL),
        )
    };
    let own = on_map(&map, move |m| {
        aura_of(m, g, GROW, AURA_MOD_SCALE, 0, 50);
        let spell = m.realm.data.spells.get(END_SPELL).cloned().unwrap();
        let _ = auras::apply_spell(m, g, g, &spell, &[(0, 5)], auras::Context::default());
        read(m)
    })
    .await;
    assert_eq!(own, (903, 1.5, false, true));
    events(&realm, &map, &[2]).await;
    let modelled = on_map(&map, read).await;
    assert_eq!(
        modelled,
        (MODEL, 3.0, true, false),
        "at twice, grown by half"
    );
    let reach = on_map(&map, move |m| {
        auras::remove_spell(m, g, GROW, Some(g));
        let f = m.unit_fields(g).unwrap();
        (
            f.get_f32(OBJECT_FIELD_SCALE_X),
            f.get_f32(UNIT_FIELD_BOUNDINGRADIUS),
            f.get_f32(UNIT_FIELD_COMBATREACH),
        )
    })
    .await;
    assert_eq!(reach, (2.0, 0.5, 2.0), "the model's own");
    events(&realm, &map, &[]).await;
    let back = on_map(&map, read).await;
    assert_eq!(back, (903, 1.0, false, true), "itself, the end's cast");
    events(&realm, &map, &[2]).await;
    let respawned = on_map(&map, move |m| {
        crate::game::map::combat::deal_damage(m, ga, g, 1_000_000);
        crate::game::map::ai::respawn_now(m, g);
        let back = creature_at(m, MODELLED);
        (back, holds(m, back, START_SPELL), holds(m, back, END_SPELL))
    })
    .await;
    assert_eq!(respawned.1, true, "its start cast as it comes back");
    assert_eq!(respawned.2, false);
}

/// Thistle Tea's "keeps corpses dead and reapplies the active event after
/// respawn": an event starting finds a corpse and leaves it one, as it
/// was; respawned, it is the event's entry, whole; the event over, its own.
#[tokio::test]
async fn a_corpse_stays_one_and_respawns_in_its_events_looks() {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let g = of(CHANGED);
    a.sees(g).await;
    on_map(&map, move |m| {
        crate::game::map::combat::deal_damage(m, ga, g, 1_000_000);
    })
    .await;
    events(&realm, &map, &[49]).await;
    let corpse = on_map(&map, move |m| {
        let c = m.creature(creature_at(m, CHANGED)).unwrap();
        (c.guid, c.entry, c.alive())
    })
    .await;
    assert_eq!(corpse, (g, ORIGINAL, false), "a corpse still");
    let respawned = on_map(&map, move |m| {
        crate::game::map::ai::respawn_now(m, g);
        let c = m.creature(creature_at(m, CHANGED)).unwrap();
        (c.entry, c.alive(), c.fields.get_u32(UNIT_FIELD_HEALTH))
    })
    .await;
    assert_eq!(respawned, (FORM, true, 400));
    events(&realm, &map, &[]).await;
    let back = on_map(&map, |m| {
        let c = m.creature(creature_at(m, CHANGED)).unwrap();
        (c.entry, c.fields.get_u32(UNIT_FIELD_HEALTH))
    })
    .await;
    assert_eq!(back, (ORIGINAL, 100));
}

/// Thistle Tea's `CreatureData.select`: of two events dressing a spawn, the
/// lowest on dresses it; it over, the other; both over, its own look.
#[tokio::test]
async fn the_lowest_event_on_dresses_a_spawn_and_the_next_when_it_ends() {
    let (realm, mut a, _ga, map) = setup_with(world(), Class::Warrior).await;
    a.sees(of(DRESSED)).await;
    let display = |m: &mut Map| {
        m.unit_fields(creature_at(m, DRESSED))
            .unwrap()
            .get_u32(UNIT_FIELD_DISPLAYID)
    };
    let mut seen = Vec::new();
    for on in [&[49, 2][..], &[49], &[]] {
        events(&realm, &map, on).await;
        seen.push(on_map(&map, display).await);
    }
    assert_eq!(seen, [1_002, 1_049, 903]);
}
