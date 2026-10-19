//! Round 7, against Thistle Tea's channel checks (`cast_validation_test`'s
//! `channel_in_range?`, `casting_test`'s channel grace, `range_test`, and
//! `cast_movement_test`'s turning and jumping): what ends a channel under
//! way, as mangos-classic's `Spell::update` has it.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, casting, pass};
use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_EX_CHANNELED, SpellEffect};
use crate::game::map::auras::*;
use crate::game::map::{Map, MapHandle};

/// A five-second channel of twenty yards at an enemy (Drain Life's kind),
/// and the same that a turn breaks (its channel interrupt flags' turning).
const DRAIN: u32 = 997_501;
const STARE: u32 = 997_502;
const RANGE_20: u32 = 997;

fn data(d: &mut GameData) {
    d.spells.ranges.insert(RANGE_20, (0.0, 20.0));
    let mut drain = bare(DRAIN, 5);
    drain.range_index = RANGE_20;
    drain.attributes[1] |= ATTR_EX_CHANNELED;
    drain.duration_index = LASTS_5S;
    effect(
        &mut drain,
        0,
        SpellEffect {
            amplitude: 1_000,
            ..aura(AURA_PERIODIC_DAMAGE, 0, 10, 6)
        },
    );
    let mut stare = drain.clone();
    stare.id = STARE;
    stare.channel_interrupt_flags = 0x10;
    insert(d, [drain, stare]);
}

/// A channel at an enemy goes on while it is within half its range again
/// past both their reaches (twenty yards, thirty, and the reaches' three),
/// and ends once it is beyond (mangos-classic's channel range,
/// `IsWithinCombatDistInMap(target, maxRange * 1.5)`).
#[tokio::test]
async fn a_channel_ends_once_its_target_is_half_its_range_again_away() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7CH1", "Drainer", Class::Warlock).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[DRAIN]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let reaches =
                m.unit_view(a).unwrap().combat_reach + m.unit_view(kobold).unwrap().combat_reach;
            cast_fresh(m, a, DRAIN, at(kobold));
            let me = m.players[&a].position();
            let o = m.players[&a].movement.orientation;
            let away = |m: &mut Map, d: f32| {
                m.creature_mut(kobold).unwrap().position = Vector3d {
                    x: me.x + d * o.cos(),
                    y: me.y + d * o.sin(),
                    z: me.z,
                };
                pass(m, 100);
                casting(m, a)
            };
            let within = away(m, 30.0 + reaches - 0.1);
            let beyond = away(m, 30.0 + reaches + 0.1);
            (within, beyond, kit::has(m, kobold, DRAIN))
        })
        .await
        .unwrap();
    assert_eq!(out, (Some(DRAIN), None, false));
}

async fn channelling(map: &MapHandle, a: Guid, spell: u32) {
    map.query(move |m| {
        kit::ready(m, a, 60, &[spell]);
        let kobold = kit::dummy(m, a, 60, 5.0);
        assert!(cast_fresh(m, a, spell, at(kobold)));
    })
    .await
    .unwrap();
}

/// The movement of `a` where they stand, with `flags` and `orientation`.
async fn movement(
    map: &MapHandle,
    a: Guid,
    flags: MovementInfo_MovementFlags,
    o: f32,
) -> MovementInfo {
    let at = map.query(move |m| m.players[&a].position()).await.unwrap();
    MovementInfo {
        flags,
        timestamp: 0,
        position: at,
        orientation: o,
        fall_time: 0,
    }
}

/// Until the map has taken `a`'s facing to be `o`: then what they cast.
async fn faced(map: &MapHandle, a: Guid, o: f32) -> Option<u32> {
    let deadline = tokio::time::Instant::now() + crate::game::harness::WAIT;
    loop {
        let (now, cast) = map
            .query(move |m| (m.players[&a].movement.orientation, casting(m, a)))
            .await
            .unwrap();
        if (now - o).abs() < 1e-4 {
            return cast;
        }
        assert!(tokio::time::Instant::now() < deadline, "never faced {o}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// A jump ends any channel; a turn ends one whose channel interrupt flags
/// name turning, and not another (mangos-classic's `Spell::update`:
/// `MOVEFLAG_JUMPING`, `AURA_INTERRUPT_FLAG_TURNING`; Thistle Tea's
/// "turning is channel-specific and jumping interrupts").
#[tokio::test]
async fn a_jump_ends_a_channel_and_a_turn_one_that_says_so() {
    let realm = fast_realm(data);
    let (c, a, map) = human(&realm, "P7CH2", "Fidget", Class::Warlock).await;
    // Turned on the spot, a plain channel goes on.
    channelling(&map, a, DRAIN).await;
    let info = movement(&map, a, MovementInfo_MovementFlags::empty(), 1.0).await;
    c.send(MSG_MOVE_SET_FACING_Client { info }).await;
    assert_eq!(faced(&map, a, 1.0).await, Some(DRAIN), "a turn: on");
    // A jump ends it.
    let info = movement(
        &map,
        a,
        MovementInfo_MovementFlags::empty().set_jumping(Default::default()),
        1.0,
    )
    .await;
    c.send(MSG_MOVE_JUMP_Client { info }).await;
    let deadline = tokio::time::Instant::now() + crate::game::harness::WAIT;
    while map.query(move |m| casting(m, a)).await.unwrap().is_some() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the jump never ended it"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    // A turn ends one that says so.
    let info = movement(&map, a, MovementInfo_MovementFlags::empty(), 1.0).await;
    c.send(MSG_MOVE_FALL_LAND_Client { info }).await;
    channelling(&map, a, STARE).await;
    let info = movement(&map, a, MovementInfo_MovementFlags::empty(), 2.0).await;
    c.send(MSG_MOVE_SET_FACING_Client { info }).await;
    assert_eq!(faced(&map, a, 2.0).await, None, "a turn: off");
}
