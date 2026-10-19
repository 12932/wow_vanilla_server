//! Round 8's world tests, ported from Thistle Tea's visibility tests
//! (`test/game/world/visibility/`, `stealth_visibility_test.exs`;
//! docs/parity/matrix.csv): what each viewer is shown of what is near.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::npc::Quest;
use crate::game::guid;
use crate::game::harness::Client;
use crate::game::tests_p4_items_quests::{START, player, questgiver, realm_of, world};

const GIVER: u32 = 997_840;
const OFFERED: u32 = 997_841;
const ALTERNATIVE: u32 = 997_842;

/// The status `c` is told of `giver`, asked.
async fn status(c: &mut Client, giver: Guid) -> u8 {
    c.send(CMSG_QUESTGIVER_STATUS_QUERY { guid: giver }).await;
    c.wait_for("SMSG_QUESTGIVER_STATUS", |m| match m {
        ServerOpcodeMessage::SMSG_QUESTGIVER_STATUS(s) if s.guid == giver => {
            Some(s.status.as_int())
        }
        _ => None,
    })
    .await
}

/// Thistle Tea's `QuestGivers.refresh/1` "projects exclusive rewards
/// independently for each viewer": a giver's quest of an exclusive group
/// shows its "!" (5) to Bob, and none (0) to Alice, who was rewarded its
/// alternative of the group; asked again, each is told the same.
#[tokio::test]
async fn a_givers_exclusive_quest_shows_to_each_viewer_by_their_own_rewards() {
    let mut d = world();
    questgiver(&mut d, 0, GIVER, OFFERED, (START.0 + 2.0, START.1, START.2));
    d.npc.quests.get_mut(&OFFERED).unwrap().exclusive_group = OFFERED as i32;
    d.npc.quests.insert(
        ALTERNATIVE,
        Quest {
            id: ALTERNATIVE,
            title: "The other way".into(),
            min_level: 1,
            level: 1,
            exclusive_group: OFFERED as i32,
            ..Default::default()
        },
    );
    let realm = realm_of(d);
    let (mut a, ga, map) = player(&realm, "Alice").await;
    let (mut b, _gb, _) = player(&realm, "Bob").await;
    let giver = guid::unit(GIVER, GIVER);
    a.sees(giver).await;
    b.sees(giver).await;
    map.query(move |m| {
        m.players
            .get_mut(&ga)
            .unwrap()
            .journal
            .rewarded
            .insert(ALTERNATIVE);
    })
    .await
    .unwrap();
    assert_eq!(status(&mut b, giver).await, 5, "Bob: its '!'");
    assert_eq!(status(&mut a, giver).await, 0, "Alice: none");
    assert_eq!(status(&mut b, giver).await, 5, "Bob again");
    assert_eq!(status(&mut a, giver).await, 0, "Alice again");
}

/// Thistle Tea's `Visibility.reveal_nearby/1` "leaves units that are not
/// listed as hidden alone": Alice turns away from Bob, beside her, and
/// walks nine yards on, so what she sees is worked out again: she is not
/// told to forget him, nor told of him again.
#[tokio::test]
async fn a_second_look_leaves_who_she_sees_known() {
    use crate::game::tests_p3_entity::NONE_WITHIN;
    let realm = realm_of(world());
    let (mut a, _ga, map) = player(&realm, "Alice").await;
    let (_b, gb, _) = player(&realm, "Bob").await;
    a.heartbeat(START.0, START.1, START.2).await;
    a.sees(gb).await;
    a.send(MSG_MOVE_SET_FACING_Client {
        info: MovementInfo {
            flags: MovementInfo_MovementFlags::empty(),
            timestamp: 0,
            position: Vector3d {
                x: START.0,
                y: START.1,
                z: START.2,
            },
            orientation: std::f32::consts::PI,
            fall_time: 0,
        },
    })
    .await;
    a.heartbeat(START.0 + 9.0, START.1, START.2).await;
    for _ in 0..5 {
        crate::game::harness::an_update(&map).await;
    }
    let forgot = a
        .gets(NONE_WITHIN, |m| {
            crate::game::harness::forgotten(m)
                .contains(&gb)
                .then_some(())
        })
        .await;
    assert!(forgot.is_none(), "not forgotten");
    assert!(!a.sees_within(gb, NONE_WITHIN).await, "nor told of again");
}
