//! Round 10's player casting: what a move or a turn breaks, measured from
//! where the cast began (Thistle Tea's `cast_movement_maps_test`).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, casting};
use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_EX_CHANNELED, SpellEffect};
use crate::game::map::MapHandle;
use crate::game::map::auras::*;

/// A channel a turn breaks; a cast a move breaks, and one it doesn't.
const STARE: u32 = 997_601;
const STILL: u32 = 997_602;
const RESTLESS: u32 = 997_603;

fn data(d: &mut GameData) {
    let mut stare = bare(STARE, 5);
    stare.attributes[1] |= ATTR_EX_CHANNELED;
    stare.duration_index = LASTS_5S;
    stare.channel_interrupt_flags = 0x10;
    effect(
        &mut stare,
        0,
        SpellEffect {
            amplitude: 1_000,
            ..aura(AURA_PERIODIC_DAMAGE, 0, 10, 6)
        },
    );
    let mut still = bare(STILL, 5);
    still.casting_time_index = CAST_3S;
    still.interrupt_flags = 0x1;
    effect(&mut still, 0, damage(10));
    let mut restless = still.clone();
    restless.id = RESTLESS;
    restless.interrupt_flags = 0;
    insert(d, [stare, still, restless]);
}

/// `a`'s movement `dx` yards along x from `from`, facing `o`.
fn step(from: Vector3d, dx: f32, o: f32) -> MovementInfo {
    MovementInfo {
        flags: MovementInfo_MovementFlags::empty(),
        timestamp: 0,
        position: Vector3d {
            x: from.x + dx,
            ..from
        },
        orientation: o,
        fall_time: 0,
    }
}

/// Until the map has `a` at `x`: then what they cast.
async fn there(map: &MapHandle, a: Guid, x: f32, o: f32) -> Option<u32> {
    let deadline = tokio::time::Instant::now() + crate::game::harness::WAIT;
    loop {
        let (p, cast) = map
            .query(move |m| {
                let p = &m.players[&a].movement;
                ((p.position.x, p.orientation), casting(m, a))
            })
            .await
            .unwrap();
        if (p.0 - x).abs() < 1e-4 && (p.1 - o).abs() < 1e-4 {
            return cast;
        }
        assert!(tokio::time::Instant::now() < deadline, "never at {x}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// Thistle Tea's "turn-only movement cancels the channel and emits its
/// failure" and "small steps accumulate from the cast origin and respect
/// interrupt flags", as mangos's `Spell::update` measures from where the
/// cast began (`m_castOrientation`, `m_castPositionX`): a channel that a
/// turn breaks goes on through a twentieth of a radian, and ends on a turn
/// of a tenth and more from where it began, though no one packet turned
/// that far; a cast a move breaks ends at a step of a twentieth of a yard,
/// one that moving doesn't break goes on through a yard of such steps.
#[tokio::test]
async fn a_cast_is_broken_by_how_far_it_has_come_from_where_it_began() {
    let realm = fast_realm(data);
    let (c, a, map) = human(&realm, "P10CM1", "Fidget", Class::Warlock).await;
    let (from, o) = map
        .query(move |m| {
            kit::ready(m, a, 60, &[STARE, STILL, RESTLESS]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            assert!(cast_fresh(m, a, STARE, at(kobold)));
            let p = &m.players[&a].movement;
            (p.position, p.orientation)
        })
        .await
        .unwrap();
    // Two turns of 0.06 each: on after the first, off after the second.
    c.send(MSG_MOVE_SET_FACING_Client {
        info: step(from, 0.0, o + 0.06),
    })
    .await;
    assert_eq!(there(&map, a, from.x, o + 0.06).await, Some(STARE), "on");
    c.send(MSG_MOVE_SET_FACING_Client {
        info: step(from, 0.0, o + 0.12),
    })
    .await;
    assert_eq!(there(&map, a, from.x, o + 0.12).await, None, "off");
    // A cast a move breaks: a twentieth of a yard does.
    let o = o + 0.12;
    map.query(move |m| {
        let kobold = kit::dummy(m, a, 60, 5.0);
        assert!(cast_fresh(m, a, STILL, at(kobold)));
    })
    .await
    .unwrap();
    c.send(MSG_MOVE_HEARTBEAT_Client {
        info: step(from, 0.05, o),
    })
    .await;
    assert_eq!(there(&map, a, from.x + 0.05, o).await, None, "a small step");
    // One a move doesn't break: a yard of small steps.
    let from = Vector3d {
        x: from.x + 0.05,
        ..from
    };
    map.query(move |m| {
        let kobold = kit::dummy(m, a, 60, 5.0);
        assert!(cast_fresh(m, a, RESTLESS, at(kobold)));
    })
    .await
    .unwrap();
    for i in 1..=20 {
        c.send(MSG_MOVE_HEARTBEAT_Client {
            info: step(from, 0.05 * i as f32, o),
        })
        .await;
    }
    assert_eq!(
        there(&map, a, from.x + 1.0, o).await,
        Some(RESTLESS),
        "moving doesn't break it"
    );
}
