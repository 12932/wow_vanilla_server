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

/// A wild summon at an enemy's place (53); a teleport of its caster to
/// the place of the unit it is cast at (63), as a creature's Blink
/// (28401) has it.
const AT_ENEMY: u32 = 997_820;
const BLINK_TO: u32 = 997_821;

fn unit_place_data() -> GameData {
    let mut d = database_data();
    let mut summon = bare(AT_ENEMY, 2);
    summon.mana_cost = 10;
    summon.range_index = 1;
    effect(
        &mut summon,
        0,
        SpellEffect {
            effect: pets::EFFECT_SUMMON_WILD,
            misc_value: WILD_ONE as i32,
            target_a: 53,
            radius_index: 8,
            ..Default::default()
        },
    );
    let mut blink = bare(BLINK_TO, 3);
    effect(
        &mut blink,
        0,
        SpellEffect {
            effect: spells::EFFECT_TELEPORT_UNITS,
            target_a: 1,
            target_b: 63,
            ..Default::default()
        },
    );
    insert(&mut d, [summon, blink]);
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
    at_the_database_place().await;
}

/// Its go, naming the place, and the summons there pass the 1.12
/// client's own parsers (`tools/client_check`).
#[tokio::test]
#[ignore = "extended"]
async fn a_spell_at_its_database_place_passes_the_client() {
    use crate::game::tests_client_emu::{capture, checker, run_checker};
    let (check_py, wow_exe) = checker().expect("the client checker");
    let (_realm, mut a) = at_the_database_place().await;
    let (frames, _) = capture(vec![&mut a]);
    run_checker(&check_py, &wow_exe, &frames);
}

async fn at_the_database_place() -> (
    std::sync::Arc<crate::game::Realm>,
    crate::game::harness::Client,
) {
    let (realm, mut a, ga, map) = setup_with(database_data(), Class::Mage).await;
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
    (realm, a)
}

/// Thistle Tea's `SpellLocations.resolve_requirements/4` "summons at the
/// unit without treating its position selector as an area" and
/// "unit-location teleports move the caster rather than the selected
/// unit", and `resolve/2` "triggered summons retain the original caster
/// and selected-unit destination": Alice's wild summon at an enemy's
/// place (53) comes once where the brute stands, not where she sent it
/// nor about another brute beside it, her mana spent; triggered at the
/// brute, there too. A brute's teleport at its place's unit (63) puts the
/// brute where Alice stands, facing as it did and still fighting her
/// (mangos's `EffectTeleportUnits` to `TARGET_LOCATION_CASTER_TARGET_
/// POSITION`'s destination); her own puts her by the other brute.
#[tokio::test]
async fn a_spell_at_a_units_place_summons_there_and_its_teleport_goes_there() {
    let (_realm, _a, ga, map) = setup_with(unit_place_data(), Class::Mage).await;
    let out = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 12.0);
        let beside = spawn(m, ga, BRUTE, 20, 13.0);
        let at = m.position_of(brute).unwrap();
        let mana = |m: &Map| m.players[&ga].fields.get_u32(UNIT_FIELD_POWER1);
        let before = mana(m);
        let cast = spells::cast(
            m,
            ga,
            AT_ENEMY,
            Targets {
                unit: Some(brute),
                dest: Some(SENT),
                ..Default::default()
            },
            None,
        );
        let spent = before - mana(m);
        let summoned = of_entry(m, WILD_ONE);
        spells::cast_triggered(m, ga, AT_ENEMY, Some(brute));
        let triggered = of_entry(m, WILD_ONE);
        // The brute fights her, and blinks to her.
        crate::game::map::ai::attacked(m, brute, ga);
        let her = m.players[&ga].position();
        let facing = m.creature(brute).unwrap().orientation;
        spells::cast(
            m,
            brute,
            BLINK_TO,
            Targets {
                unit: Some(ga),
                ..Default::default()
            },
            None,
        );
        let c = m.creature(brute).unwrap();
        let blinked = (c.position, c.orientation, c.combat.victim);
        // Hers, to the brute beside.
        let there = m.position_of(beside).unwrap();
        spells::cast(
            m,
            ga,
            BLINK_TO,
            Targets {
                unit: Some(beside),
                ..Default::default()
            },
            None,
        );
        let moved = m.players[&ga].position();
        (
            at,
            (cast, spent, summoned),
            triggered,
            (her, facing, blinked),
            (there, moved),
        )
    })
    .await;
    let (at, (cast, spent, summoned), triggered, (her, facing, blinked), (there, moved)) = out;
    assert!(cast);
    assert_eq!(spent, 10, "her mana spent");
    assert_eq!(summoned.len(), 1, "one summon, no area: {summoned:?}");
    assert!(near(summoned[0].1, at), "at the brute: {summoned:?}");
    assert_eq!(triggered.len(), 2);
    assert!(
        triggered.iter().all(|(_, p)| near(*p, at)),
        "triggered at the brute, there too: {triggered:?}"
    );
    assert!(
        near(blinked.0, her),
        "the brute where she stands: {blinked:?}"
    );
    assert!(
        (blinked.1 - facing).abs() < 0.01,
        "facing as it did: {blinked:?}"
    );
    assert_eq!(blinked.2, ga, "still fighting her");
    assert!(near(moved, there), "she by the other brute: {moved:?}");
}
