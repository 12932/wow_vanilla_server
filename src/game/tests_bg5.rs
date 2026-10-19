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
pub(super) async fn put(map: &MapHandle, c: &mut Client, x: f32, y: f32, z: f32) {
    let g = c.guid;
    map.query(move |m| m.teleport(g, m.key, Vector3d { x, y, z }, 0.0))
        .await;
    c.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: c.teleport_order(),
        time: Default::default(),
    })
    .await;
}

pub(super) fn gulch() -> GameData {
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

pub(super) async fn said(c: &mut Client, containing: &str) -> String {
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

pub(super) fn horde_flag(m: &crate::game::map::Map) -> Option<Flag> {
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
pub(super) async fn alice_takes_the_horde_flag() -> (Client, Client, MapHandle, Guid) {
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

#[tokio::test]
async fn one_who_leaves_dead_goes_home_alive_and_a_stray_from_the_match_is_a_deserter() {
    let mut d = gulch();
    marker(
        &mut d,
        crate::game::map::battleground::SPELL_WAITING_TO_RESURRECT,
    );
    marker(&mut d, crate::game::map::battleground::SPELL_DESERTER);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let grom = b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::WarsongGulch).await;
    // Alice falls and releases: Waiting to Resurrect.
    map.query(move |m| {
        crate::game::map::death::kill(m, alice);
        crate::game::map::death::release(m, alice);
    })
    .await;
    let waiting = map
        .query(move |m| {
            m.players[&alice]
                .auras
                .has_spell(crate::game::map::battleground::SPELL_WAITING_TO_RESURRECT)
        })
        .await
        .unwrap();
    assert!(waiting);
    // She leaves a ghost: home alive and whole, no corpse.
    a.send(CMSG_LEAVE_BATTLEFIELD {
        map: Map::WarsongGulch,
    })
    .await;
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(alice).await;
    let home = realm.registry.online(alice).unwrap().map();
    let (alive, corpse, full) = home
        .query(move |m| {
            let p = &m.players[&alice];
            (
                p.death == crate::game::map::death::Death::Alive,
                p.row.state.corpse.is_some(),
                p.fields.get_u32(crate::game::fields::UNIT_FIELD_HEALTH)
                    == p.fields.get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH),
            )
        })
        .await
        .unwrap();
    assert_eq!((alive, corpse, full), (true, false, true));
    // Grom is sent out another way (a hearthstone, say): no longer in it,
    // and a deserter.
    map.query(move |m| {
        m.teleport(
            grom,
            crate::game::map::MapKey::world(1),
            Vector3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
        )
    })
    .await;
    b.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    b.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    b.sees(grom).await;
    let mut member = true;
    for _ in 0..100 {
        member = realm.social.battlegrounds.lock().match_of(grom).is_some();
        if !member {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!member);
    let there = realm.registry.online(grom).unwrap().map();
    let deserter = there
        .query(move |m| {
            m.players[&grom]
                .auras
                .has_spell(crate::game::map::battleground::SPELL_DESERTER)
        })
        .await
        .unwrap();
    assert!(deserter);
}

#[tokio::test]
async fn alteracs_base_graveyard_is_a_choice_and_its_mines_supplies_are_their_holders() {
    let mut d = super::tests_pvp::alterac();
    // Stormpike Aid Station's graveyard, far off; the Alliance's own, near.
    d.safe_locs.insert(751, (30, 500.0, 500.0, 0.0));
    super::tests_pvp::banner(&mut d, 178785, 3);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::AlteracValley).await;
    let (grave, supplies) = map
        .query(move |m| {
            let grave = crate::game::map::battleground::av::graveyard(
                m,
                0,
                Vector3d {
                    x: 1.0,
                    y: 1.0,
                    z: 0.0,
                },
            );
            let t = m.realm.data.gameobject_templates[&178785].clone();
            let at = m.players[&alice].position();
            let mut go = crate::game::map::gameobject::GameObject::from_spawn(
                &t,
                &GameObjectSpawn {
                    guid: 0x80_0201,
                    entry: 178785,
                    map: 30,
                    x: at.x + 1.0,
                    y: at.y,
                    z: at.z,
                    state: 1,
                    ..Default::default()
                },
            );
            go.spawn_guid = None;
            (grave.map(|g| g.0), m.add_gameobject(go))
        })
        .await
        .unwrap();
    let grave = grave.expect("a graveyard");
    assert!(grave.x.abs() < 0.01 && grave.y.abs() < 0.01, "{grave:?}");
    // Irondeep is no one's: its supplies aren't Alice's.
    let barred = map
        .query(move |m| crate::game::map::battleground::av::supplies_barred(m, Some(0), supplies))
        .await
        .unwrap();
    assert!(barred);
    map.query(|m| {
        m.battleground.as_mut().unwrap().av.as_mut().unwrap().mines[0].owner = Some(0);
    })
    .await;
    let (alliance, horde) = map
        .query(move |m| {
            (
                crate::game::map::battleground::av::supplies_barred(m, Some(0), supplies),
                crate::game::map::battleground::av::supplies_barred(m, Some(1), supplies),
            )
        })
        .await
        .unwrap();
    assert_eq!((alliance, horde), (false, true));
}

#[test]
fn alterac_has_one_bracket_and_the_status_says_which() {
    use crate::game::battlegrounds::{bracket_for, status_frame_in};
    assert_eq!(bracket_for(1, 51), 0);
    assert_eq!(bracket_for(1, 60), 0);
    assert_eq!(bracket_for(2, 60), 5);
    assert_eq!(bracket_for(2, 25), 1);
    // The bracket byte, after the queue slot and the map.
    let frame = status_frame_in(3, 489, 7, 1, 0, 1234);
    let body = &frame[4..];
    assert_eq!(body[8], 3);
    assert_eq!(&body[13..17], &1u32.to_le_bytes(), "waiting in the queue");
    assert_eq!(&body[17..21], &0u32.to_le_bytes(), "no average wait");
}
