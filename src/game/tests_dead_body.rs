//! The dead lie where they fell, until they release or rise: in the user's
//! race a bot's body ran on along the road, its client still sending moves.
//! The dead are rooted, as mangos's `KillPlayer` roots them, and a dead
//! body takes no move but a fall straight down.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::death;

async fn rooted(c: &mut Client, g: Guid, root: bool) {
    c.wait_for(if root { "the root" } else { "the unroot" }, |m| match m {
        ServerOpcodeMessage::SMSG_FORCE_MOVE_ROOT(r) if root && r.guid == g => Some(()),
        ServerOpcodeMessage::SMSG_FORCE_MOVE_UNROOT(r) if !root && r.guid == g => Some(()),
        _ => None,
    })
    .await;
}

#[tokio::test]
async fn the_dead_lie_where_they_fell_until_they_release_or_rise() {
    let realm = harness::realm_with(GameData::empty());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    let kill = move |m: &mut crate::game::map::Map| {
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_HEALTH, 0);
        death::kill(m, g);
        m.players[&g].position()
    };
    let at = map.query(kill).await.unwrap();
    rooted(&mut a, g, true).await;
    // Ten yards along the road, and then a fall straight down: the body
    // stays, and lands.
    a.heartbeat(at.x + 10.0, at.y, at.z).await;
    a.heartbeat(at.x, at.y, at.z - 2.0).await;
    let mut landed = false;
    for _ in 0..100 {
        let p = map.query(move |m| m.players[&g].position()).await.unwrap();
        assert!(
            (p.x - at.x).abs() < 0.1,
            "the body moved: {p:?} from {at:?}"
        );
        if (p.z - (at.z - 2.0)).abs() < 0.1 {
            landed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(landed, "a fall straight down is taken");
    // Released, a ghost walks.
    a.send(CMSG_REPOP_REQUEST {}).await;
    rooted(&mut a, g, false).await;
    // Raised where they fell, they walk again.
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bea", Race::Human, Class::Warrior).await;
    let map_b = realm.registry.online(gb).unwrap().map();
    map_b.query(kill_of(gb)).await;
    rooted(&mut b, gb, true).await;
    map_b.query(move |m| death::resurrect(m, gb, 0.5)).await;
    rooted(&mut b, gb, false).await;
}

fn kill_of(g: Guid) -> impl FnOnce(&mut crate::game::map::Map) + Send + 'static {
    move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_HEALTH, 0);
        death::kill(m, g);
    }
}

/// The 1.12 client won't send a dead player's /say (its own
/// `ERR_CHAT_WHILE_DEAD`), so a dead GM's command comes some other way: a
/// "." line is a command on any chat type, a whisper to oneself too.
#[tokio::test]
async fn a_dead_gm_commands_by_whisper() {
    let realm = harness::realm_with(GameData::empty());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    map.query(kill_of(g)).await;
    rooted(&mut a, g, true).await;
    a.send(CMSG_MESSAGECHAT {
        chat_type: CMSG_MESSAGECHAT_ChatType::Whisper {
            target_player: "Alice".into(),
        },
        language: Language::Universal,
        message: ".revive".into(),
    })
    .await;
    let mut alive = false;
    for _ in 0..200 {
        alive = map
            .query(move |m| m.players[&g].death == death::Death::Alive)
            .await
            .unwrap_or(false);
        if alive {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(alive, "revived by a whispered .revive");
}

/// Alice's place, as the map has it.
async fn place_of(map: &crate::game::map::MapHandle, g: Guid) -> Vector3d {
    map.query(move |m| m.players[&g].position()).await.unwrap()
}

/// Waits until Alice's place is `want`, or says where she is.
async fn comes_to(
    map: &crate::game::map::MapHandle,
    g: Guid,
    want: Vector3d,
) -> Result<(), Vector3d> {
    let mut at = place_of(map, g).await;
    for _ in 0..100 {
        if crate::game::map::combat::dist(at, want) < 0.05 {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        at = place_of(map, g).await;
    }
    Err(at)
}

/// Killed in the air, a body comes down where its client's fall lands it,
/// as far as its jump carries it on, and lies there: those who see it are
/// told the landing, and its corpse at the release is on the ground
/// (mangos-classic's `KillPlayer`: "don't create corpse at this moment,
/// player might be falling"; its client's landing taken). A landing further
/// on than the jump goes is refused, as any run of the dead. Before
/// (decision 107), only a fall straight down was taken: a client whose
/// jump went on as it died landed nowhere, its body left in the air where
/// its last heartbeat had it, a yard and a half up, for every watcher.
#[tokio::test]
async fn one_killed_in_a_jump_comes_down_where_the_jump_lands() {
    use wow_world_messages::vanilla::{MovementInfo, MovementInfo_MovementFlags_Jumping};
    let realm = harness::realm_with(GameData::empty());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bea", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    let at = place_of(&map, g).await;
    b.sees(g).await;
    let jumping =
        MovementInfo_MovementFlags::new_forward().set_jumping(MovementInfo_MovementFlags_Jumping {
            z_speed: -7.96,
            cos_angle: 1.0,
            sin_angle: 0.0,
            xy_speed: 7.0,
        });
    let info = |flags, x: f32, z: f32, fall_time| MovementInfo {
        flags,
        timestamp: 0,
        position: Vector3d { x, y: at.y, z },
        orientation: 0.0,
        fall_time,
    };
    // A jump east at a run, its top 0.3 s on, a yard and a half up.
    a.send(MSG_MOVE_JUMP_Client {
        info: info(jumping, at.x, at.z, 0),
    })
    .await;
    let top = Vector3d {
        x: at.x + 2.1,
        z: at.z + 1.5,
        ..at
    };
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: info(jumping, top.x, top.z, 300),
    })
    .await;
    assert_eq!(comes_to(&map, g, top).await, Ok(()), "the jump's top taken");
    map.query(kill_of(g)).await;
    rooted(&mut a, g, true).await;
    // Too far on for its jump (8 yd in 0.4 s): refused, the body where it was.
    a.send(MSG_MOVE_FALL_LAND_Client {
        info: info(MovementInfo_MovementFlags::empty(), top.x + 8.0, at.z, 700),
    })
    .await;
    harness::an_update(&map).await;
    assert_eq!(
        place_of(&map, g).await,
        top,
        "a landing past the jump's reach"
    );
    // Its fall lands 2.8 yd on, 0.4 s at the jump's 7 yd a second.
    let landed = Vector3d {
        x: top.x + 2.8,
        ..at
    };
    a.send(MSG_MOVE_FALL_LAND_Client {
        info: info(MovementInfo_MovementFlags::empty(), landed.x, landed.z, 700),
    })
    .await;
    assert_eq!(comes_to(&map, g, landed).await, Ok(()), "the landing taken");
    let told = b
        .wait_for("Alice's landing", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_FALL_LAND(s) if s.guid == g => Some(s.info.position),
            _ => None,
        })
        .await;
    assert!(
        crate::game::map::combat::dist(told, landed) < 0.05,
        "Bea told of the landing: {told:?}"
    );
    // Landed, it lies there: a run on goes nowhere.
    a.heartbeat(landed.x + 3.0, landed.y, landed.z).await;
    harness::an_update(&map).await;
    assert_eq!(
        place_of(&map, g).await,
        landed,
        "the body lies where it landed"
    );
    a.send(CMSG_REPOP_REQUEST {}).await;
    rooted(&mut a, g, false).await;
    let corpse = map
        .query(move |m| {
            m.corpses
                .values()
                .find(|c| c.owner == g)
                .map(|c| c.position)
        })
        .await
        .unwrap()
        .expect("her corpse");
    assert!(
        crate::game::map::combat::dist(corpse, landed) < 0.05,
        "her corpse where she landed: {corpse:?}"
    );
}
