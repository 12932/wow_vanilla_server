//! Round 8's entity tests, ported from Thistle Tea's
//! `test/game/world/entity/*_test.exs` (docs/parity/matrix.csv): a
//! passenger's place on deck.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::harness::Client;
use crate::game::map::MapHandle;
use crate::game::tests_p3_combat::{START, data, setup_with};

/// A heartbeat aboard `transport` at `offset` on deck, standing at `at`.
async fn aboard(c: &Client, transport: Guid, offset: Vector3d, at: Vector3d) {
    c.send(MSG_MOVE_HEARTBEAT_Client {
        info: MovementInfo {
            flags: MovementInfo_MovementFlags::empty().set_on_transport(
                MovementInfo_MovementFlags_OnTransport {
                    transport: TransportInfo {
                        guid: transport,
                        position: offset,
                        orientation: 0.0,
                    },
                },
            ),
            timestamp: 0,
            position: at,
            orientation: 0.0,
            fall_time: 0,
        },
    })
    .await;
}

async fn deck(map: &MapHandle, g: Guid) -> (Option<Vector3d>, Vector3d) {
    map.query(move |m| {
        let p = &m.players[&g];
        (
            p.movement
                .flags
                .get_on_transport()
                .map(|t| t.transport.position),
            p.position(),
        )
    })
    .await
    .unwrap()
}

/// A place on deck farther than a deck goes (50 yards across, 100 up, as
/// mangos's `VerifyMovementInfo` has it) is refused: the movement isn't
/// taken and its sender doesn't board; one within it is (Thistle Tea's
/// "rejects out-of-bounds local coordinates without boarding"). wvs refuses
/// the same far on the other side too, where mangos looks at the positive
/// side only.
#[tokio::test]
async fn a_place_on_deck_beyond_a_deck_is_refused() {
    let (_realm, a, ga, map) = setup_with(data(), Class::Warrior).await;
    let boat = crate::game::guid::make(crate::game::guid::HighGuid::MoTransport, 0, 1);
    let here = Vector3d {
        x: START.0,
        y: START.1,
        z: START.2,
    };
    let there = Vector3d {
        x: START.0 + 2.0,
        ..here
    };
    for far in [
        Vector3d {
            x: 251.0,
            y: 0.0,
            z: 0.0,
        },
        Vector3d {
            x: 0.0,
            y: 51.0,
            z: 0.0,
        },
        Vector3d {
            x: 0.0,
            y: 0.0,
            z: 101.0,
        },
        Vector3d {
            x: -51.0,
            y: 0.0,
            z: 0.0,
        },
    ] {
        aboard(&a, boat, far, there).await;
    }
    // A heartbeat behind them to know they have all been seen to.
    a.heartbeat(here.x, here.y + 1.0, here.z).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let refused = loop {
        let (on, at) = deck(&map, ga).await;
        if (at.y - (here.y + 1.0)).abs() < 0.01 || tokio::time::Instant::now() > deadline {
            break (on, at);
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert_eq!(refused.0, None, "not aboard");
    assert!((refused.1.x - here.x).abs() < 0.01, "{:?}", refused.1);
    let near = Vector3d {
        x: 1.0,
        y: 2.0,
        z: 3.0,
    };
    aboard(&a, boat, near, there).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let taken = loop {
        let got = deck(&map, ga).await;
        if got.0.is_some() || tokio::time::Instant::now() > deadline {
            break got;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert_eq!(taken.0, Some(near), "aboard, at its place");
}
