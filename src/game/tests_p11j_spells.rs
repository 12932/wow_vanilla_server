//! Round 11j's spell tests: a script's cast at an immune creature, the
//! objects a spell picks, and an object's cast at a place (Thistle Tea's
//! `spellcasting_test`, `spell_objects_test` and
//! `spell_unit_locations_test`).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::scripts::Condition;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::spells::Targets;
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};
use crate::game::tests_p3_entity::on;

const KILL: u32 = 97_400;
const LOCK_IT: u32 = 97_401;
const LOCK_AREAS: u32 = 97_402;
const CALL: u32 = 97_403;
const STONE: u32 = 97_410;
const POST: u32 = 97_411;
const READY: u32 = 97_420;

fn spell(id: u32, effects: &[SpellEffect]) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    for (i, e) in effects.iter().enumerate() {
        s.effects[i] = *e;
    }
    s
}

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.spells.radii.insert(8, 5.0);
    // Death to any unit (Thistle Tea's `any_unit`).
    d.spells.spells.insert(
        KILL,
        spell(
            KILL,
            &[SpellEffect {
                effect: 1,
                target_a: 25,
                ..Default::default()
            }],
        ),
    );
    // Lock the nearest stone a script names.
    d.spells.spells.insert(
        LOCK_IT,
        spell(
            LOCK_IT,
            &[SpellEffect {
                effect: crate::game::map::objects::EFFECT_ACTIVATE_OBJECT,
                misc_value: 7,
                target_a: 23,
                ..Default::default()
            }],
        ),
    );
    d.spells.script_targets.insert(LOCK_IT, vec![(0, STONE)]);
    d.spells.script_target_conditions.insert((LOCK_IT, 0, STONE), READY);
    d.scripts.conditions.insert(
        READY,
        Condition {
            entry: READY,
            kind: 55,
            values: [1, 0, 0, 0],
            flags: 0,
        },
    );
    // The stones about the caster, and the posts about where it is aimed.
    d.spells.spells.insert(
        LOCK_AREAS,
        spell(
            LOCK_AREAS,
            &[
                SpellEffect {
                    effect: crate::game::map::objects::EFFECT_ACTIVATE_OBJECT,
                    misc_value: 7,
                    target_a: 22,
                    target_b: 51,
                    radius_index: 8,
                    ..Default::default()
                },
                SpellEffect {
                    effect: crate::game::map::objects::EFFECT_ACTIVATE_OBJECT,
                    misc_value: 6,
                    target_a: 52,
                    radius_index: 8,
                    ..Default::default()
                },
            ],
        ),
    );
    d.spells
        .script_targets
        .insert(LOCK_AREAS, vec![(0, STONE), (0, POST)]);
    // Stones not for the second effect, posts not for the first.
    d.spells
        .script_target_masks
        .insert((LOCK_AREAS, 0, STONE), 0b10);
    d.spells.script_target_masks.insert((LOCK_AREAS, 0, POST), 0b01);
    // A mark on every unit about where the spell is aimed (mangos's
    // `TARGET_ENUM_UNITS_SCRIPT_AOE_AT_DEST_LOC`, no script units named).
    let mut call = spell(
        CALL,
        &[SpellEffect {
            effect: 6,
            aura: 4,
            target_a: 8,
            radius_index: 8,
            ..Default::default()
        }],
    );
    call.duration_index = 0;
    d.spells.spells.insert(CALL, call);
    for entry in [STONE, POST] {
        d.gameobject_templates.insert(
            entry,
            crate::game::data::GameObjectTemplate {
                entry,
                kind: 10,
                display_id: 1,
                name: format!("Object {entry}"),
                size: 1.0,
                ..Default::default()
            },
        );
    }
    d
}

fn place(m: &mut Map, entry: u32, at: Vector3d) -> Guid {
    crate::game::map::objects::summon(m, None, entry, at, 0.0, 0, 0, 0, None).unwrap()
}

fn offset(at: Vector3d, dx: f32, dz: f32) -> Vector3d {
    Vector3d {
        x: at.x + dx,
        y: at.y,
        z: at.z + dz,
    }
}

/// Thistle Tea's "any-unit casts receive template immunity from
/// metadata": a script's cast by a player of a harmful spell at any unit
/// kills a creature; at one immune to players (`UNIT_FLAG_IMMUNE_TO_PLAYER`)
/// it is refused before it costs (`checks::check`'s `can_attack`), the
/// creature alive and the client told.
#[tokio::test]
async fn a_scripts_cast_at_a_creature_immune_to_players_is_refused() {
    let (_realm, mut a, g, map) = setup_with(world(), Class::Mage).await;
    let (killed, immune) = on(&map, move |m| {
        let open = spawn(m, g, BRUTE, 10, 4.0);
        let immune = spawn(m, g, BRUTE, 10, 6.0);
        let f = &mut m.creature_mut(immune).unwrap().fields;
        let flags = f.get_u32(UNIT_FIELD_FLAGS);
        f.set_u32(UNIT_FIELD_FLAGS, flags | 0x100);
        crate::game::map::scripts::cast(m, g, open, KILL, 0);
        (!m.creature(open).is_some_and(|c| c.alive()), immune)
    })
    .await;
    assert!(killed, "any unit");
    a.clear();
    let alive = on(&map, move |m| {
        m.now = std::time::Instant::now();
        if let Some(p) = m.players.get_mut(&g) {
            p.caster.gcd_until = None;
        }
        crate::game::map::scripts::cast(m, g, immune, KILL, 0);
        m.creature(immune).is_some_and(|c| c.alive())
    })
    .await;
    assert!(alive, "immune to players");
    let refused = a
        .wait_frame("SMSG_CAST_RESULT", 0x130, |f| {
            u32::from_le_bytes(f[0..4].try_into().unwrap()) == KILL
        })
        .await;
    assert_eq!(refused[4], 2, "failed");
}

/// Thistle Tea's "selects the nearest conditioned object in this copy": a
/// spell at the nearest stone its script names, with a condition (the
/// stone's state ready, `CONDITION_GAMEOBJECT_STATE`), passes over the
/// nearest whose state isn't and takes the next; the nearest's state
/// ready, it takes that one (`objects::object_targets` with
/// `script_target`'s conditions). Another copy's stones aren't in this
/// map's grid at all.
#[tokio::test]
async fn a_spell_takes_the_nearest_object_its_condition_allows() {
    let (_realm, _a, g, map) = setup_with(world(), Class::Mage).await;
    let picked = on(&map, move |m| {
        let at = m.players[&g].position();
        place(m, STONE, offset(at, 0.0, 40.0));
        let far = place(m, STONE, offset(at, 4.0, 0.0));
        let near = place(m, STONE, offset(at, 1.0, 0.0));
        for o in [far, near] {
            m.gameobject_mut(o)
                .unwrap()
                .fields
                .set_u32(GAMEOBJECT_STATE, 1);
        }
        m.gameobject_mut(near)
            .unwrap()
            .fields
            .set_u32(GAMEOBJECT_STATE, 0);
        let s = m.realm.data.spells.get(LOCK_IT).cloned().unwrap();
        let first = crate::game::map::objects::cast_at(m, g, &s, &Targets::default());
        m.gameobject_mut(near)
            .unwrap()
            .fields
            .set_u32(GAMEOBJECT_STATE, 1);
        let second = crate::game::map::objects::cast_at(m, g, &s, &Targets::default());
        (first == vec![far], second == vec![near])
    })
    .await;
    assert_eq!(picked, (true, true), "the far one, then the near");
}

/// Thistle Tea's "retains area targets by effect and respects inverse
/// masks": a spell with an effect on the objects about its caster and one
/// on those about where it is aimed takes, for each, those its script
/// names within the radius and not kept off that effect by their row's
/// mask: the stone by the caster for the first, the post by the place for
/// the second; the stone at the place and a post six yards above it, not.
#[tokio::test]
async fn an_area_spells_objects_are_each_effects_own() {
    let (_realm, _a, g, map) = setup_with(world(), Class::Mage).await;
    let ok = on(&map, move |m| {
        let at = m.players[&g].position();
        let there = offset(at, 20.0, 0.0);
        let stone = place(m, STONE, at);
        let post = place(m, POST, there);
        place(m, STONE, there);
        place(m, POST, offset(there, 0.0, 6.0));
        let s = m.realm.data.spells.get(LOCK_AREAS).cloned().unwrap();
        let t = Targets {
            dest: Some(there),
            ..Default::default()
        };
        let mut all = crate::game::map::objects::cast_at(m, g, &s, &t);
        all.sort();
        let mut want = vec![stone, post];
        want.sort();
        all == want
    })
    .await;
    assert!(ok, "the stone by the caster and the post by the place");
}

/// Thistle Tea's "object casters resolve the selected location without a
/// unit component": a stone's cast at a creature twenty yards off, of a
/// spell on the units about where it is aimed, marks that creature and
/// not one beside the stone (`cast_triggered`'s destination, its target's
/// place, for any caster; `scripts::cast` sends an object's casts
/// through it).
#[tokio::test]
async fn an_objects_cast_at_a_creature_lands_where_it_stands() {
    let (_realm, _a, g, map) = setup_with(world(), Class::Mage).await;
    let marked = on(&map, move |m| {
        let here = m.players[&g].position();
        let stone = place(m, STONE, offset(here, 0.0, 0.0));
        let target = spawn(m, g, BRUTE, 10, 20.0);
        let beside = spawn(m, g, BRUTE, 10, 2.0);
        crate::game::map::scripts::cast(m, stone, target, CALL, 0);
        let has = |m: &Map, u: Guid| {
            crate::game::map::auras::auras(m, u).is_some_and(|a| a.has_spell(CALL))
        };
        (has(m, target), has(m, beside))
    })
    .await;
    assert_eq!(marked, (true, false), "where the creature stands");
}
