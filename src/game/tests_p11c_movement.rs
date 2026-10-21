//! Round 11c: a cast's movement measured on deck (Thistle Tea's
//! `cast_movement_test.exs`, "uses transport-relative distance and
//! orientation"; the user's call): aboard a boat, where the caster stands
//! and faces is the deck's, so the ship's own way and turns break nothing;
//! a step on deck, another ship or the shore does.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, cast_fresh};
use crate::game::data::spells::SpellTemplate;
use crate::game::harness::Client;
use crate::game::map::MapHandle;
use crate::game::map::spells::Targets;

/// A three-second cast that movement breaks, and a channel that turning
/// breaks.
const MOVE_BREAKS: u32 = 998_960;
const TURN_BREAKS: u32 = 998_961;

const SHIP: u64 = 0x1FC0_0000_0000_0077;
const OTHER_SHIP: u64 = 0x1FC0_0000_0000_0078;

fn data(d: &mut crate::game::data::GameData) {
    let mut cast = bare(MOVE_BREAKS, 0);
    cast.casting_time_index = CAST_3S;
    cast.interrupt_flags = 0x1;
    let mut channel: SpellTemplate = bare(TURN_BREAKS, 0);
    channel.attributes[1] |= crate::game::data::spells::ATTR_EX_CHANNELED;
    channel.duration_index = LASTS_15S;
    channel.channel_interrupt_flags = 0x10;
    insert(d, [cast, channel]);
}

/// A heartbeat at `world`, facing `o`, on deck of `ship` at `deck` facing
/// `deck_o` (none ashore); waited for until the map has it.
async fn step(
    c: &Client,
    map: &MapHandle,
    g: Guid,
    world: (f32, f32, f32),
    o: f32,
    ship: Option<(u64, (f32, f32, f32), f32)>,
) {
    let v = |(x, y, z): (f32, f32, f32)| Vector3d { x, y, z };
    let mut flags = MovementInfo_MovementFlags::empty();
    if let Some((guid, deck, deck_o)) = ship {
        flags = flags.set_on_transport(MovementInfo_MovementFlags_OnTransport {
            transport: TransportInfo {
                guid: Guid::new(guid),
                position: v(deck),
                orientation: deck_o,
            },
        });
    }
    let info = MovementInfo {
        flags,
        timestamp: 0,
        position: v(world),
        orientation: o,
        fall_time: 0,
    };
    c.send(MSG_MOVE_HEARTBEAT_Client { info }).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let there = map
            .query(move |m| {
                m.players.get(&g).is_some_and(|p| {
                    p.movement.position == info.position
                        && p.movement.orientation == info.orientation
                        && p.movement.flags.get_on_transport() == info.flags.get_on_transport()
                })
            })
            .await
            .unwrap();
        if there {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the move never came"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

async fn begin(map: &MapHandle, g: Guid, spell: u32) {
    let began = map
        .query(move |m| {
            kit::ready(m, g, 60, &[spell]);
            cast_fresh(m, g, spell, Targets::default());
            kit::casting(m, g)
        })
        .await
        .unwrap();
    assert_eq!(began, Some(spell));
}

async fn casting(map: &MapHandle, g: Guid) -> Option<u32> {
    map.query(move |m| kit::casting(m, g)).await.unwrap()
}

#[tokio::test]
async fn a_cast_aboard_is_measured_on_deck() {
    let realm = fast_realm(data);
    let (c, g, map) = human(&realm, "P11CMV", "Sailor", Class::Mage).await;
    let start = map
        .query(move |m| {
            let p = m.players[&g].position();
            (p.x, p.y, p.z)
        })
        .await
        .unwrap();
    let away = (start.0 + 40.0, start.1 + 30.0, start.2);
    let deck = (1.0, 2.0, 3.0);
    // Aboard, the ship carrying the caster off and turning it: the cast
    // goes on.
    step(&c, &map, g, start, 0.0, Some((SHIP, deck, 0.0))).await;
    begin(&map, g, MOVE_BREAKS).await;
    step(&c, &map, g, away, 1.0, Some((SHIP, deck, 0.0))).await;
    assert_eq!(casting(&map, g).await, Some(MOVE_BREAKS), "the ship's way");
    // A step on deck breaks it.
    step(&c, &map, g, start, 0.0, Some((SHIP, (1.5, 2.0, 3.0), 0.0))).await;
    assert_eq!(casting(&map, g).await, None, "a step on deck");
    // So does another ship, at the same spot of its deck.
    begin(&map, g, MOVE_BREAKS).await;
    step(
        &c,
        &map,
        g,
        away,
        0.0,
        Some((OTHER_SHIP, (1.5, 2.0, 3.0), 0.0)),
    )
    .await;
    assert_eq!(casting(&map, g).await, None, "another ship");
    // And going ashore.
    let shore = (away.0 + 0.5, away.1, away.2);
    begin(&map, g, MOVE_BREAKS).await;
    step(&c, &map, g, shore, 0.0, None).await;
    assert_eq!(casting(&map, g).await, None, "ashore");
    // Ashore, as before: a step from where it began breaks it.
    begin(&map, g, MOVE_BREAKS).await;
    step(&c, &map, g, (shore.0 + 1.0, shore.1, shore.2), 0.0, None).await;
    assert_eq!(casting(&map, g).await, None, "a step ashore");
}

#[tokio::test]
async fn a_channel_aboard_turns_with_its_deck() {
    let realm = fast_realm(data);
    let (c, g, map) = human(&realm, "P11CMT", "Helmsman", Class::Mage).await;
    let start = map
        .query(move |m| {
            let p = m.players[&g].position();
            (p.x, p.y, p.z)
        })
        .await
        .unwrap();
    let deck = (1.0, 2.0, 3.0);
    step(&c, &map, g, start, 0.0, Some((SHIP, deck, 0.0))).await;
    begin(&map, g, TURN_BREAKS).await;
    // The ship turning the caster in the world: the channel goes on.
    step(&c, &map, g, start, 1.5, Some((SHIP, deck, 0.0))).await;
    assert_eq!(casting(&map, g).await, Some(TURN_BREAKS), "the ship's turn");
    // A turn on deck stops it.
    step(&c, &map, g, start, 1.7, Some((SHIP, deck, 0.2))).await;
    assert_eq!(casting(&map, g).await, None, "a turn on deck");
}
