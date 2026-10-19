//! Round 7's world tests, ported from Thistle Tea's
//! `test/game/world/spell/` (docs/parity/matrix.csv): a teleport's
//! database destination.

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::SpellEffect;
use crate::game::tests_p3_cast_kit::{bare, effect, fast_realm, human, insert};

const NEAR: u32 = 997_300;
const FAR: u32 = 997_301;

/// Thistle Tea's `SpellDatabaseLocations` "ordinary teleports retain
/// their separate cross-map destination path" and "teleports retain the
/// configured facing": a teleport whose `spell_target_position` is on her
/// map puts Alice there, facing as the row says; one whose row is on
/// Kalimdor sends her there, to its place and facing (mangos's
/// `EffectTeleportUnits`).
#[tokio::test]
async fn a_teleport_goes_to_its_rows_place_and_facing_on_its_map_or_another() {
    let realm = fast_realm(|d| {
        for id in [NEAR, FAR] {
            let mut s = bare(id, 0);
            effect(
                &mut s,
                0,
                SpellEffect {
                    effect: crate::game::map::spells::EFFECT_TELEPORT_UNITS,
                    target_a: 1,
                    ..Default::default()
                },
            );
            insert(d, [s]);
        }
        d.spells
            .target_positions
            .insert(NEAR, (0, -8_920.0, -120.0, 85.0, 1.5));
        d.spells
            .target_positions
            .insert(FAR, (1, 10.0, 20.0, 30.0, 2.5));
    });
    let (mut c, a, map) = human(&realm, "P7WT", "Porter", Class::Mage).await;
    let near = map
        .query(move |m| {
            crate::game::map::spells::cast_triggered(m, a, NEAR, Some(a));
            let p = &m.players[&a];
            (p.position(), p.movement.orientation)
        })
        .await
        .unwrap();
    assert_eq!(
        near,
        (
            Vector3d {
                x: -8_920.0,
                y: -120.0,
                z: 85.0
            },
            1.5
        )
    );
    c.clear();
    map.query(move |m| crate::game::map::spells::cast_triggered(m, a, FAR, Some(a)))
        .await
        .unwrap();
    let far = c
        .wait_for("SMSG_NEW_WORLD", |m| match m {
            ServerOpcodeMessage::SMSG_NEW_WORLD(w) => Some((w.map, w.position, w.orientation)),
            _ => None,
        })
        .await;
    assert_eq!(
        far,
        (
            Map::Kalimdor,
            Vector3d {
                x: 10.0,
                y: 20.0,
                z: 30.0
            },
            2.5
        )
    );
}
