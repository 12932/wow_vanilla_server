//! The battlegrounds against Thistle Tea, module by module, through the
//! protocol: Warsong Gulch's flags (a dropped flag going home, both down
//! after a capture, the carrier's leaving and lost aura, its world states
//! and messages), the status's bracket and times, the list, and the shared
//! rules (leaving dead, the waiting aura, Alterac's graveyards and mine
//! supplies, straying out of a match).

use std::time::{Duration, Instant};

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_pvp::{battleground, both_into, marker};
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;
use crate::game::map::battleground::{Flag, wsg};

const WSG: u32 = 489;

fn text(d: &mut GameData, id: u32, line: &str) {
    d.npc.broadcast_texts.insert(
        id,
        crate::game::data::npc::BroadcastText {
            male: line.into(),
            ..Default::default()
        },
    );
}

/// Move `c` within the match's map, at once, to a height.
async fn put(map: &MapHandle, c: &mut Client, x: f32, y: f32, z: f32) {
    let g = c.guid;
    map.query(move |m| m.teleport(g, m.key, Vector3d { x, y, z }, 0.0))
        .await;
    c.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
}

fn gulch() -> GameData {
    let mut d = GameData::empty();
    battleground(&mut d, WSG, 2, "PVPZone03", [769, 770]);
    for (s, id) in wsg::CAPTURE_TRIGGERS.into_iter().enumerate() {
        let (x, y, z, _) = wsg::FLAG_AT[s];
        d.area_triggers.insert(
            id,
            AreaTrigger {
                id,
                map: WSG,
                x,
                y,
                z,
                radius: 5.0,
                ..Default::default()
            },
        );
    }
    for (id, line) in [
        (9804, "The Alliance Flag was picked up by $n!"),
        (9807, "The Horde Flag was picked up by $n!"),
        (9806, "The Horde Flag was dropped by $n!"),
        (10023, "The Horde Flag has returned to its base!"),
        (9801, "$n captured the Horde Flag!"),
        (9803, "The flags are now placed at their bases."),
        (10015, "The battle for Warsong Gulch begins in 1 minute."),
    ] {
        text(&mut d, id, line);
    }
    marker(&mut d, wsg::SPELL_FLAG_CARRIED[1]);
    d
}

async fn said(c: &mut Client, containing: &str) -> String {
    c.wait_for(containing, |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(chat) if chat.message.contains(containing) => {
            Some(chat.message.clone())
        }
        _ => None,
    })
    .await
}

async fn state(c: &mut Client, field: u32, value: u32) {
    c.wait_for("the world state", |m| match m {
        ServerOpcodeMessage::SMSG_UPDATE_WORLD_STATE(w)
            if w.state.state == field && w.state.value == value =>
        {
            Some(())
        }
        _ => None,
    })
    .await
}

fn horde_flag(m: &crate::game::map::Map) -> Option<Flag> {
    m.battleground.as_ref().and_then(|b| b.flags[1])
}

/// The match's timers, due now.
async fn hurry(map: &MapHandle) {
    map.query(|m| {
        if let Some(b) = m.battleground.as_mut() {
            for t in b.timers.iter_mut() {
                t.0 = Instant::now();
            }
        }
    })
    .await;
    tokio::time::sleep(Duration::from_millis(120)).await;
}

/// Alice (Alliance) and Grom (Horde) in Warsong Gulch, Alice by the
/// Horde's flag with it in hand.
async fn alice_takes_the_horde_flag() -> (Client, Client, MapHandle, Guid) {
    let realm = harness::realm_with(gulch());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::WarsongGulch).await;
    let mut stand = None;
    for _ in 0..100 {
        stand = map
            .query(|m| match horde_flag(m) {
                Some(Flag::AtBase(go)) => Some(go),
                _ => None,
            })
            .await
            .flatten();
        if stand.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let stand = stand.expect("the Horde's flag up");
    let (x, y, z, _) = wsg::FLAG_AT[1];
    put(&map, &mut a, x, y, z).await;
    a.send(CMSG_GAMEOBJ_USE { guid: stand }).await;
    let alice = a.guid;
    (a, b, map, alice)
}

#[tokio::test]
async fn a_flag_taken_shows_on_its_takers_side_and_one_dropped_goes_home_in_ten_seconds() {
    let (mut a, _b, map, alice) = alice_takes_the_horde_flag().await;
    assert_eq!(
        said(&mut a, "picked up").await,
        "The Horde Flag was picked up by Alice!"
    );
    // Taken (1546 = 1), and carried on the Alliance's side (2339 = 2).
    state(&mut a, 1546, 1).await;
    state(&mut a, 2339, 2).await;
    // She falls: it's on the ground (1546 = -1) where she was.
    map.query(move |m| crate::game::map::battleground::died(m, alice, Guid::zero()))
        .await;
    assert_eq!(
        said(&mut a, "dropped").await,
        "The Horde Flag was dropped by Alice!"
    );
    state(&mut a, 1546, u32::MAX).await;
    assert!(matches!(
        map.query(|m| horde_flag(m)).await.flatten(),
        Some(Flag::Dropped(_))
    ));
    // Left alone ten seconds, it's home.
    hurry(&map).await;
    assert_eq!(
        said(&mut a, "returned").await,
        "The Horde Flag has returned to its base!"
    );
    assert!(matches!(
        map.query(|m| horde_flag(m)).await.flatten(),
        Some(Flag::AtBase(_))
    ));
}

#[tokio::test]
async fn after_a_capture_both_flags_are_down_until_placed_again() {
    let (mut a, _b, map, _alice) = alice_takes_the_horde_flag().await;
    state(&mut a, 2339, 2).await;
    let (x, y, z, _) = wsg::FLAG_AT[0];
    put(&map, &mut a, x, y, z).await;
    a.send(CMSG_AREATRIGGER {
        trigger_id: wsg::CAPTURE_TRIGGERS[0],
    })
    .await;
    assert_eq!(
        said(&mut a, "captured").await,
        "Alice captured the Horde Flag!"
    );
    let flags = map
        .query(|m| m.battleground.as_ref().unwrap().flags)
        .await
        .unwrap();
    assert_eq!(flags, [Some(Flag::Waiting), Some(Flag::Waiting)]);
    let stands = map
        .query(|m| {
            m.gameobjects
                .iter()
                .filter(|(_, o)| wsg::FLAG_STANDS.contains(&o.entry))
                .count()
        })
        .await
        .unwrap();
    assert_eq!(stands, 0, "both stands down");
    hurry(&map).await;
    said(&mut a, "placed at their bases").await;
    let flags = map
        .query(|m| m.battleground.as_ref().unwrap().flags)
        .await
        .unwrap();
    assert!(flags.iter().all(|f| matches!(f, Some(Flag::AtBase(_)))));
}

#[tokio::test]
async fn a_carrier_who_loses_the_flags_aura_or_leaves_drops_it() {
    let (mut a, _b, map, alice) = alice_takes_the_horde_flag().await;
    state(&mut a, 2339, 2).await;
    // The aura gone (cancelled): dropped.
    map.query(move |m| {
        crate::game::map::auras::remove_spell(m, alice, wsg::SPELL_FLAG_CARRIED[1], None)
    })
    .await;
    said(&mut a, "dropped").await;
    let dropped = map.query(|m| horde_flag(m)).await.flatten();
    let Some(Flag::Dropped(go)) = dropped else {
        panic!("dropped: {dropped:?}");
    };
    // She takes it up again, and leaves the match with it: dropped.
    a.send(CMSG_GAMEOBJ_USE { guid: go }).await;
    for _ in 0..100 {
        if matches!(
            map.query(|m| horde_flag(m)).await.flatten(),
            Some(Flag::Carried(_))
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    a.send(CMSG_LEAVE_BATTLEFIELD {
        map: Map::WarsongGulch,
    })
    .await;
    for _ in 0..100 {
        if matches!(
            map.query(|m| horde_flag(m)).await.flatten(),
            Some(Flag::Dropped(_))
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(matches!(
        map.query(|m| horde_flag(m)).await.flatten(),
        Some(Flag::Dropped(_))
    ));
}
