//! Phase 3's conditions, Thistle Tea's `ConditionTest` ported: server
//! variables by comparison whoever is asked, a PvP rank for the unranked
//! and for creatures, a flag of the source's fields, the targets swapped
//! before a tree is judged, and a copy's script data three ways (met, unmet
//! or unknown) under not, and and or.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::scripts::{CF_REVERSE_RESULT, CF_SWAP_TARGETS, Condition};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::conditions::{Truth, check};
use crate::game::map::{Map, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
/// A server variable of this test's own (they are realm-wide statics).
const VARIABLE: u32 = 9_930_050;
const UNSET: u32 = 9_930_056;
const BOAR: u32 = 38;
const WOLF: u32 = 39;

fn data() -> GameData {
    let mut d = GameData::empty();
    let mut cond = |id: u32, kind: i32, values: [i32; 4], flags: u32| {
        d.scripts.conditions.insert(
            id,
            Condition {
                entry: id,
                kind,
                values,
                flags,
            },
        );
    };
    let v = VARIABLE as i32;
    // Server variables: equal, at least, at most; three past two; one
    // under; swapped; reversed; an unset one at zero; no such comparison.
    cond(1, 11, [v, 2, 0, 0], 0);
    cond(2, 11, [v, 2, 1, 0], 0);
    cond(3, 11, [v, 2, 2, 0], 0);
    cond(4, 11, [v, 3, 1, 0], 0);
    cond(5, 11, [v, 1, 2, 0], 0);
    cond(6, 11, [v, 2, 0, 0], CF_SWAP_TARGETS);
    cond(7, 11, [v, 2, 0, 0], CF_REVERSE_RESULT);
    cond(8, 11, [UNSET as i32, 0, 0, 0], 0);
    cond(9, 11, [v, 2, 3, 0], 0);
    // A PvP rank of none.
    cond(20, 51, [0, 0, 0, 0], 0);
    // The source's UNIT_FIELD_FLAGS has 0x2, not 0x4.
    cond(30, 31, [i32::from(UNIT_FIELD_FLAGS), 0x2, 0, 0], 0);
    cond(31, 31, [i32::from(UNIT_FIELD_FLAGS), 0x4, 0, 0], 0);
    // The source a wolf, alone and in a swapped tree.
    cond(40, 16, [WOLF as i32, 0, 0, 0], CF_SWAP_TARGETS);
    cond(41, 16, [WOLF as i32, 0, 0, 0], 0);
    cond(42, -1, [41, 0, 0, 0], CF_SWAP_TARGETS);
    // A copy's script's data, equal to nothing; a bad comparison,
    // reversed; on this map (met), with each under and and or.
    cond(50, 34, [7, 0, 0, 0], 0);
    cond(51, 34, [7, 0, 9, 0], CF_REVERSE_RESULT);
    cond(52, 33, [0, 0, 0, 0], 0);
    cond(53, -1, [52, 50, 0, 0], 0);
    cond(54, -2, [52, 50, 0, 0], 0);
    for (entry, dx) in [(BOAR, 3.0), (WOLF, 4.0)] {
        d.creature_templates.insert(
            entry,
            CreatureTemplate {
                entry,
                name: format!("Beast {entry}"),
                display_ids: [903, 0, 0, 0],
                display_probabilities: [1, 0, 0, 0],
                display_scales: [1.0, 0.0, 0.0, 0.0],
                level_min: 1,
                level_max: 1,
                faction: 7,
                unit_class: 1,
                health_max: 50,
                ..Default::default()
            },
        );
        d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
            guid: entry,
            entries: vec![entry],
            map: 0,
            x: START.0 + dx,
            y: START.1,
            z: START.2,
            ..Default::default()
        });
    }
    d
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

async fn setup() -> (Arc<Realm>, Client, Guid) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    (realm, a, g)
}

fn boar() -> Guid {
    guid::unit(BOAR, BOAR)
}

fn wolf() -> Guid {
    guid::unit(WOLF, WOLF)
}

/// Thistle Tea's server variables: compared equal, at least or at most,
/// whoever the source and target are; reversed, unmet; an unset one is
/// none; a comparison not of the three unknown.
#[tokio::test]
async fn server_variables_compare_whoever_asks() {
    let (realm, _a, g) = setup().await;
    crate::game::map::scripts::set_server_variable(VARIABLE, 2);
    let seen = on_map(&realm, move |m| {
        (1..=9)
            .map(|id| check(m, id, Some(g), None))
            .chain([check(m, 1, None, Some(boar()))])
            .collect::<Vec<_>>()
    })
    .await;
    use Truth::*;
    assert_eq!(
        seen,
        vec![Met, Met, Met, Unmet, Unmet, Met, Unmet, Met, Unknown, Met]
    );
}

/// Thistle Tea's PvP rank: an unranked player's is none; a creature has
/// none to meet it.
#[tokio::test]
async fn a_pvp_rank_is_a_players() {
    let (realm, _a, g) = setup().await;
    let seen = on_map(&realm, move |m| {
        (
            check(m, 20, None, Some(g)),
            check(m, 20, None, Some(boar())),
        )
    })
    .await;
    assert_eq!(seen, (Truth::Met, Truth::Unmet));
}

/// Thistle Tea's has-flag: a flag of a field of the source's, set or not;
/// with no source, unknown.
#[tokio::test]
async fn a_flag_of_the_sources_fields() {
    let (realm, _a, g) = setup().await;
    let seen = on_map(&realm, move |m| {
        let f = m.unit_fields_mut(g).unwrap();
        let flags = f.get_u32(UNIT_FIELD_FLAGS);
        f.set_u32(UNIT_FIELD_FLAGS, (flags & !0x7) | 0x3);
        (
            check(m, 30, Some(g), None),
            check(m, 31, Some(g), None),
            check(m, 30, None, Some(g)),
        )
    })
    .await;
    assert_eq!(seen, (Truth::Met, Truth::Unmet, Truth::Unknown));
}

/// Thistle Tea's swap: the targets swapped before a condition, or a tree,
/// is judged: a boar the source, a wolf the target, and "the source is a
/// wolf" swapped holds, as does an and of it unswapped under a swapped
/// tree.
#[tokio::test]
async fn targets_swap_before_a_tree_is_judged() {
    let (realm, _a, _g) = setup().await;
    let seen = on_map(&realm, move |m| {
        (
            check(m, 40, Some(boar()), Some(wolf())),
            check(m, 41, Some(boar()), Some(wolf())),
            check(m, 42, Some(boar()), Some(wolf())),
        )
    })
    .await;
    assert_eq!(seen, (Truth::Met, Truth::Unmet, Truth::Met));
}

/// Thistle Tea's instance data: no script, unmet; a script not ported,
/// unknown, reversed too; a bad comparison unknown; with a met condition,
/// an and unknown and an or met.
#[tokio::test]
async fn a_copys_data_is_met_unmet_or_unknown() {
    let (realm, _a, g) = setup().await;
    let seen = on_map(&realm, move |m| {
        let none = check(m, 50, None, Some(g));
        m.instance.script = Some(crate::game::map::instance_script::Script::Unported);
        (
            none,
            check(m, 50, None, Some(g)),
            check(m, 51, None, Some(g)),
            check(m, 53, None, Some(g)),
            check(m, 54, None, Some(g)),
        )
    })
    .await;
    use Truth::*;
    assert_eq!(seen, (Unmet, Unknown, Unknown, Unknown, Met));
}
