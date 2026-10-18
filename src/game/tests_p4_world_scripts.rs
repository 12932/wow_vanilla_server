//! Phase 3's fourth round of the creature teleports, ported from Thistle
//! Tea's `test/game/creature_teleport_content_test.exs` and
//! `creature_teleport_vmangos_test.exs` (docs/parity/matrix.csv): script
//! command 6 puts a creature where it says in the map copy it is in,
//! whatever map the row names, alive or dead, and the imported rows are
//! there.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::ScriptStep;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::tests_p3_combat::{BRUTE, data, setup_with, spawn};
use crate::game::tests_p4_world::on;

/// A command 6 step to `at`, declaring `map`.
fn teleport(map: u32, at: [f32; 4]) -> ScriptStep {
    ScriptStep {
        command: 6,
        datalong: [map, 0, 0, 0],
        x: at[0],
        y: at[1],
        z: at[2],
        o: at[3],
        ..Default::default()
    }
}

/// Where `c` is, which map, and its fields that a teleport leaves be.
fn state(m: &Map, c: Guid) -> ([f32; 4], u32, (u32, u32, u32)) {
    let x = m.creature(c).unwrap();
    (
        [x.position.x, x.position.y, x.position.z, x.orientation],
        m.key.map,
        (
            x.fields.get_u32(UNIT_FIELD_HEALTH),
            x.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
            x.fields.get_u32(UNIT_FIELD_LEVEL),
        ),
    )
}

/// Thistle Tea's "all seven imported rows execute as exact same-copy
/// creature teleports", "Barthilas keeps map 329 copy 73 when the row
/// declares map zero" and "dead creatures remain eligible for the
/// server-controlled unit path": a creature's command 6 puts it at the
/// row's place and facing in its own map, the map the row declares (0 or
/// 329) taking it nowhere else; its fields and fight are its own still;
/// and a dead one is put there too.
#[tokio::test]
async fn a_creatures_teleport_puts_it_where_the_row_says_in_its_own_map() {
    let (_realm, _a, g, map) = setup_with(data(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let start = crate::game::tests_p3_combat::START;
        let mut seen = Vec::new();
        for (declared, d) in [(0, 10.0), (329, 15.0), (1, 20.0)] {
            let c = spawn(m, g, BRUTE, 20, 5.0);
            crate::game::map::ai::attacked(m, c, g);
            let before = state(m, c).2;
            let at = [start.0 + d, start.1 + 3.0, start.2, 2.47837];
            crate::game::map::scripts::execute(m, &teleport(declared, at), c, Guid::zero());
            let after = state(m, c);
            let fighting = m.creature(c).unwrap().threat.iter().any(|(t, _)| *t == g);
            seen.push((at, before, after, fighting));
        }
        let dead = spawn(m, g, BRUTE, 20, 5.0);
        crate::game::map::combat::kill(m, g, dead);
        let at = [start.0 + 4.0, start.1 + 5.0, start.2, 0.0];
        crate::game::map::scripts::execute(m, &teleport(0, at), dead, Guid::zero());
        (seen, (at, state(m, dead).0))
    })
    .await;
    let (seen, dead) = out;
    for (at, before, (now, map, fields), fighting) in seen {
        assert_eq!(now, at, "put where the row says");
        assert_eq!(map, 0, "in its own map, whatever the row declares");
        assert_eq!(fields, before, "its fields its own");
        assert!(fighting, "its fight its own");
    }
    assert_eq!(dead.1, dead.0, "a dead one too");
}

/// Thistle Tea's "pins the complete imported command-6 inventory" and
/// "pins final-target routing and creature same-world regressions": the
/// real scripts' creature teleports are loaded as vmangos has them, and
/// the final-target rows that start Aurius's and Jesse's scripts.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_creature_teleports_are_loaded() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    let rows = |table: &ahash::AHashMap<u32, Vec<ScriptStep>>| {
        let mut v: Vec<(u32, u32, [f32; 4])> = table
            .iter()
            .flat_map(|(id, steps)| {
                steps
                    .iter()
                    .filter(|s| s.command == 6)
                    .map(move |s| (*id, s.datalong[0], [s.x, s.y, s.z, s.o]))
            })
            .collect();
        v.sort_by_key(|a| a.0);
        v
    };
    let s = &d.scripts;
    assert_eq!(
        rows(&s.ai_scripts),
        [(184_202, 0, [2942.58, -1390.09, 167.421, 4.2586])]
    );
    assert_eq!(
        rows(&s.movement),
        [
            (144_507, 0, [-3660.6, -717.77, 28.156, 3.05292]),
            (144_611, 0, [-3628.43, -719.022, 10.8226, 0.0]),
            (1_043_504, 0, [4068.74, -3535.97, 122.825, 2.47837]),
        ]
    );
    assert_eq!(
        rows(&s.generic),
        [
            (4_341, 0, [-8408.25, 451.896, 123.76, 5.52986]),
            (10_917, 329, [4032.73, -3366.51, 115.063, 5.42797]),
            (144_502, 0, [-3666.06, -718.628, 9.95469, 0.0]),
        ]
    );
    let start = |table: &ahash::AHashMap<u32, Vec<ScriptStep>>, id: u32| {
        table[&id]
            .iter()
            .filter(|x| x.command == 39)
            .map(|x| (x.datalong[0], x.target_type, x.target_param1, x.data_flags))
            .collect::<Vec<_>>()
    };
    assert_eq!(start(&s.ai_scripts, 1_044_001), [(10_917, 11, 53_297, 2)]);
    assert_eq!(start(&s.movement, 148_201), [(144_502, 11, 9_566, 2)]);
    let jesse = s.generic[&4_341].iter().find(|x| x.command == 6).unwrap();
    assert_eq!(
        (jesse.target_type, jesse.target_param1, jesse.data_flags),
        (11, 45_707, 2)
    );
}
