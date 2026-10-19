//! Round 7, against Thistle Tea's `spell_teleport_test.exs`: a spell's
//! teleport of its target to its caster (mangos-classic's
//! `EffectTeleUnitsFaceCaster`) and to its place in the database
//! (`EffectTeleportUnits`).

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit as kit;
use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::map::spells;
use crate::game::map::{Map, MapKey};

/// A summons of its target five yards before its caster (15734's kind);
/// the same to its caster's own spot; a teleport of its caster to its
/// place in the database.
const SUMMON: u32 = 997_201;
const TO_CASTER: u32 = 997_202;
const PORT: u32 = 997_203;
const RADIUS_5: u32 = 998;
const PORT_TO: Vector3d = Vector3d {
    x: -8_900.0,
    y: -100.0,
    z: 82.0,
};

fn data(d: &mut GameData) {
    d.spells.radii.insert(RADIUS_5, 5.0);
    let summons = |id: u32, target_b: u32| {
        let mut s = bare(id, 0);
        s.range_index = 0;
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: spells::EFFECT_TELEPORT_UNITS_FACE_CASTER,
                target_a: 6,
                target_b,
                radius_index: RADIUS_5,
                ..Default::default()
            },
        );
        s
    };
    let mut port = bare(PORT, 0);
    effect(
        &mut port,
        0,
        SpellEffect {
            effect: spells::EFFECT_TELEPORT_UNITS,
            target_a: 1,
            target_b: 17,
            ..Default::default()
        },
    );
    d.spells
        .target_positions
        .insert(PORT, (0, PORT_TO.x, PORT_TO.y, PORT_TO.z, 1.5));
    insert(d, [summons(SUMMON, 47), summons(TO_CASTER, 18), port]);
}

fn near(a: Vector3d, b: Vector3d) -> bool {
    (a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01 && (a.z - b.z).abs() < 0.01
}

/// A summons to its caster puts its target the effect's radius before the
/// caster (five yards), or on the caster's own spot for that selector; the
/// target faces the caster's facing turned about; a passenger on a flight
/// isn't taken off it (Thistle Tea's "retains forward distance", "uses the
/// caster origin for caster-destination selectors" and "rejects taxi
/// passengers"; mangos-classic's `EffectTeleUnitsFaceCaster`).
#[tokio::test]
async fn a_summons_to_its_caster_goes_before_it() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7ST1", "Summoned", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let kobold = kit::dummy(m, a, 60, 20.0);
            m.creature_mut(kobold).unwrap().orientation = 1.0;
            let from = m.position_of(kobold).unwrap();
            let at = |m: &Map| (m.players[&a].position(), m.players[&a].movement.orientation);
            spells::cast_triggered(m, kobold, SUMMON, Some(a));
            let forward = at(m);
            spells::cast_triggered(m, kobold, TO_CASTER, Some(a));
            let spot = at(m).0;
            // On a flight, it stays on it.
            let pos = m.players[&a].position();
            let far = Vector3d {
                x: pos.x + 1000.0,
                ..pos
            };
            m.players.get_mut(&a).unwrap().flight = Some(crate::game::map::npc::Flight {
                walk: crate::game::map::creature::Walk::new(vec![pos, far], 1.0, m.now, 0, true),
                destination: 0,
                mount: 0,
            });
            spells::cast_triggered(m, kobold, SUMMON, Some(a));
            let flying = (m.players[&a].flight.is_some(), at(m).0);
            (from, forward, spot, pos, flying)
        })
        .await
        .unwrap();
    let (from, (forward, facing), spot, pos, (flying, still)) = out;
    let ahead = Vector3d {
        x: from.x + 5.0 * 1.0f32.cos(),
        y: from.y + 5.0 * 1.0f32.sin(),
        z: from.z,
    };
    assert!(near(forward, ahead), "{forward:?} {ahead:?}");
    assert!(
        (facing - (std::f32::consts::TAU - 1.0)).abs() < 1e-4,
        "{facing}"
    );
    assert!(near(spot, from), "{spot:?}");
    assert!(flying && near(still, pos), "kept on the flight");
}

/// A teleport to the spell's place in the database, on the map its target
/// is in, keeps the target in its copy of that map (Thistle Tea's
/// "resolves database coordinates within the current instance"; mangos's
/// `NearTeleportTo` for a destination of the target's own map): BWL's
/// orb in a raid's copy, not the world's.
#[tokio::test]
async fn a_teleport_to_its_place_on_the_same_map_keeps_the_copy() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P7ST2", "Ported", Class::Mage).await;
    let copy = MapKey {
        map: 0,
        instance: 9,
    };
    let start = map.query(move |m| m.players[&a].position()).await.unwrap();
    map.call(move |m| m.teleport(a, copy, start, 0.0));
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(a).await;
    let there = realm.registry.online(a).unwrap().map();
    let out = there
        .query(move |m| {
            spells::cast_triggered(m, a, PORT, Some(a));
            (m.key, m.players.get(&a).map(|p| p.position()))
        })
        .await
        .unwrap();
    assert_eq!(out.0, copy);
    assert!(out.1.is_some_and(|p| near(p, PORT_TO)), "{:?}", out.1);
    assert!(
        c.gets(std::time::Duration::from_millis(300), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
        })
        .await
        .is_none(),
        "no new world: the same copy"
    );
    assert_eq!(realm.registry.online(a).unwrap().map().key, copy);
}
