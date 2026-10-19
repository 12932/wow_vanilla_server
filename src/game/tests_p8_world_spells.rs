//! Round 8's world tests, ported from Thistle Tea's
//! `test/game/world/spell/` (docs/parity/matrix.csv): a spell at its
//! database place.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::map::spells::{self, Targets};
use crate::game::map::{Map, objects, pets};
use crate::game::tests_p3_cast_kit::{LASTS_15S, bare, effect, insert};
use crate::game::tests_p3_combat::{BRUTE, START, data, setup_with, spawn, template};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_entity_objects::object;

const WILD: u32 = 997_800;
const GUARD: u32 = 997_801;
const OBJECT: u32 = 997_802;
const GROUND: u32 = 997_803;
const BLAST: u32 = 997_804;
const ELSEWHERE: u32 = 997_805;
const MISSING: u32 = 997_806;
const WILD_ONE: u32 = 997_810;
const GUARD_ONE: u32 = 997_811;
const STONE: u32 = 997_812;

/// The database's place, twenty yards east of where Alice stands.
const PLACE: Vector3d = Vector3d {
    x: START.0 + 20.0,
    y: START.1 + 5.0,
    z: START.2,
};
/// Where the casts are sent, five yards east of her.
const SENT: Vector3d = Vector3d {
    x: START.0 + 5.0,
    y: START.1,
    z: START.2,
};

fn database_data() -> GameData {
    let mut d = data();
    for id in [WILD_ONE, GUARD_ONE] {
        d.creature_templates.insert(id, template(id, 7));
    }
    d.gameobject_templates
        .insert(STONE, object(STONE, 5, "Stone", &[]));
    d.spells.radii.insert(8, 5.0);
    d.spells.durations.insert(
        LASTS_15S,
        crate::game::data::spells::SpellDuration {
            base: 15_000,
            per_level: 0,
            max: 15_000,
        },
    );
    let at_place = |id: u32, e: SpellEffect| {
        let mut s = bare(id, 2);
        s.mana_cost = 10;
        s.duration_index = LASTS_15S;
        effect(
            &mut s,
            0,
            SpellEffect {
                target_a: 17,
                radius_index: 8,
                ..e
            },
        );
        s
    };
    let summon = |kind: u32, entry: u32| SpellEffect {
        effect: kind,
        misc_value: entry as i32,
        ..Default::default()
    };
    insert(
        &mut d,
        [
            at_place(WILD, summon(pets::EFFECT_SUMMON_WILD, WILD_ONE)),
            at_place(GUARD, summon(pets::EFFECT_SUMMON_GUARDIAN, GUARD_ONE)),
            at_place(OBJECT, summon(objects::EFFECT_SUMMON_OBJECT_WILD, STONE)),
            at_place(ELSEWHERE, summon(objects::EFFECT_SUMMON_OBJECT_WILD, STONE)),
            at_place(MISSING, summon(objects::EFFECT_SUMMON_OBJECT_WILD, STONE)),
            at_place(
                GROUND,
                SpellEffect {
                    effect: spells::EFFECT_PERSISTENT_AREA_AURA,
                    aura: crate::game::map::auras::AURA_PERIODIC_DAMAGE,
                    base_points: 9,
                    base_dice: 1,
                    amplitude: 1000,
                    ..Default::default()
                },
            ),
            at_place(
                BLAST,
                SpellEffect {
                    effect: 2,
                    base_points: 99,
                    base_dice: 1,
                    target_b: 8,
                    ..Default::default()
                },
            ),
        ],
    );
    for id in [WILD, GUARD, OBJECT, GROUND, BLAST] {
        d.spells
            .target_positions
            .insert(id, (0, PLACE.x, PLACE.y, PLACE.z, 1.25));
    }
    d.spells
        .target_positions
        .insert(ELSEWHERE, (1, 30.0, 40.0, 50.0, 1.25));
    d
}

fn near(a: Vector3d, b: Vector3d) -> bool {
    crate::game::map::combat::dist(a, b) < 0.01
}

/// The creatures of `entry` on the map, where they are.
fn of_entry(m: &Map, entry: u32) -> Vec<(Guid, Vector3d)> {
    m.creatures
        .iter()
        .filter(|(_, c)| c.entry == entry)
        .map(|(_, c)| (c.guid, c.position))
        .collect()
}

/// Thistle Tea's `SpellLocations.resolve/4` "replaces supplied
/// coordinates with the cached same-map destination" and "leaves
/// coordinates unchanged when the row is missing or belongs to another
/// map", `resolve_requirements/4` "all summon families receive fixed
/// coordinates in the current world", "ground auras use the database
/// location" and "destination area effects use nearby units in the
/// current copy", and `resolve/2` "triggered casts resolve fixed
/// coordinates at the original caster": a spell at its database place
/// (17) whose row is on Alice's map is there, whatever place she sent:
/// its wild creature, her guardian, its stone and its ground are there,
/// its blast takes the brute there and not the one she aimed at, her mana
/// is spent and its go names the place (mangos's `SetTargetMap` for
/// `TARGET_LOCATION_DATABASE`). Triggered, it is there too. A row on
/// another map, or none, leaves the place she sent.
#[tokio::test]
async fn a_spell_at_its_database_place_is_there_on_its_map() {
    let (_realm, mut a, ga, map) = setup_with(database_data(), Class::Mage).await;
    let out = on(&map, move |m| {
        let mana = |m: &Map| m.players[&ga].fields.get_u32(UNIT_FIELD_POWER1);
        let sent = Targets {
            dest: Some(SENT),
            ..Default::default()
        };
        let before = mana(m);
        let cast = spells::cast(m, ga, WILD, sent, None);
        let spent = before - mana(m);
        let wild = of_entry(m, WILD_ONE);
        let triggered_before = wild.len();
        spells::cast_triggered(m, ga, WILD, Some(ga));
        let triggered = of_entry(m, WILD_ONE);
        spells::cast(m, ga, GUARD, sent, None);
        let guard = of_entry(m, GUARD_ONE);
        let stones = |m: &Map| -> Vec<Vector3d> {
            m.gameobjects
                .iter()
                .filter(|(_, o)| o.entry == STONE)
                .map(|(_, o)| o.position)
                .collect()
        };
        spells::cast(m, ga, OBJECT, sent, None);
        let stone = stones(m);
        spells::cast(m, ga, ELSEWHERE, sent, None);
        spells::cast(m, ga, MISSING, sent, None);
        let all_stones = stones(m);
        spells::cast(m, ga, GROUND, sent, None);
        let ground: Vec<Vector3d> = m
            .spells
            .dynamic
            .values()
            .filter(|o| o.spell == GROUND)
            .map(|o| o.position)
            .collect();
        // One brute where the blast falls, one she aims at.
        let there = spawn(m, ga, BRUTE, 20, 10.0);
        crate::game::map::scripts::place(
            m,
            there,
            Vector3d {
                x: PLACE.x + 1.0,
                ..PLACE
            },
            0.0,
        );
        let aimed = spawn(m, ga, BRUTE, 20, 8.0);
        let health = |m: &Map, g: Guid| m.creature(g).unwrap().fields.get_u32(UNIT_FIELD_HEALTH);
        spells::cast(
            m,
            ga,
            BLAST,
            Targets {
                unit: Some(aimed),
                dest: Some(SENT),
                ..Default::default()
            },
            None,
        );
        (
            (cast, spent),
            wild,
            (triggered_before, triggered),
            guard,
            stone,
            all_stones,
            ground,
            (health(m, there), health(m, aimed)),
        )
    })
    .await;
    let ((cast, spent), wild, (triggered_before, triggered), guard, stone, all, ground, hurt) = out;
    assert!(cast, "the wild summon cast");
    assert_eq!(spent, 10, "its mana spent");
    assert_eq!(wild.len(), 1);
    assert!(
        near(wild[0].1, PLACE),
        "the wild one at the place: {wild:?}"
    );
    assert_eq!(triggered.len(), triggered_before + 1);
    assert!(
        triggered.iter().all(|(_, at)| near(*at, PLACE)),
        "triggered, at the place too: {triggered:?}"
    );
    assert_eq!(guard.len(), 1);
    assert!(near(guard[0].1, PLACE), "her guardian there: {guard:?}");
    assert_eq!(stone.len(), 1);
    assert!(near(stone[0], PLACE), "its stone there: {stone:?}");
    assert_eq!(all.len(), 3);
    assert_eq!(
        all.iter().filter(|at| near(**at, SENT)).count(),
        2,
        "a row on another map, or none, leaves the place she sent: {all:?}"
    );
    assert_eq!(ground.len(), 1);
    assert!(near(ground[0], PLACE), "its ground there: {ground:?}");
    assert_eq!(
        hurt,
        (100_000 - 100, 100_000),
        "the blast takes the brute at the place, not the one aimed at"
    );
    let go = a
        .wait_for("the wild summon's go", |msg| match msg {
            ServerOpcodeMessage::SMSG_SPELL_GO(g) if g.spell == WILD => g
                .targets
                .target_flags
                .get_dest_location()
                .map(|d| d.destination),
            _ => None,
        })
        .await;
    assert!(near(go, PLACE), "its go names the place: {go:?}");
}
