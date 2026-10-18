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
