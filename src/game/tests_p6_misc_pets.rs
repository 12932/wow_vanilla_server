//! Round 6's tests of pets and the units players control, ported from
//! Thistle Tea's `test/game/network/message/pet_messages_test.exs`,
//! `test/game/core/pet` and others (docs/parity/matrix.csv).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::map::pets::{self, ACT_COMMAND, ACT_ENABLED, COMMAND_FOLLOW, COMMAND_STAY};
use crate::game::tests_p3_combat::{BRUTE, STRIKE, setup, spawn};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_player2_ammo::handled;

/// Thistle Tea's `CMSG_PET_SET_ACTION` "dispatches action-bar changes to a
/// charmed unit" and `CMSG_PET_ACTION` "dispatches commands to a charmed
/// unit": the unit a player charms takes their bar's edits (a spell's
/// button on, its autocast on) and their commands (stay, follow); another
/// player commands it nothing.
#[tokio::test]
async fn a_charmed_unit_takes_its_charmers_bar_and_commands() {
    let (realm, mut a, ga, map) = setup(Class::Warrior).await;
    let (mut b, _gb) = crate::game::tests_p3_entity::bob(&realm, ga).await;
    let brute = on(&map, move |m| {
        let g = spawn(m, ga, BRUTE, 20, 3.0);
        pets::charmed(m, ga, g, false);
        let s = pets::state_mut(m, g).unwrap();
        s.spells = vec![STRIKE];
        s.autocast.clear();
        g
    })
    .await;
    a.send(CMSG_PET_SET_ACTION {
        guid: brute,
        position1: 3,
        data1: pets::button(STRIKE, ACT_ENABLED),
        extra: None,
    })
    .await;
    handled(&mut a).await;
    let (autocast, button) = on(&map, move |m| {
        let s = pets::state(m, brute).unwrap();
        (s.autocast.clone(), s.bar[3])
    })
    .await;
    assert_eq!(autocast, vec![STRIKE], "its autocast on");
    assert_eq!(button, pets::button(STRIKE, ACT_ENABLED));
    let command = |c: u8| CMSG_PET_ACTION {
        pet: brute,
        data: pets::button(u32::from(c), ACT_COMMAND),
        target: Guid::zero(),
    };
    a.send(command(COMMAND_STAY)).await;
    handled(&mut a).await;
    let stays = on(&map, move |m| pets::state(m, brute).unwrap().command).await;
    assert_eq!(stays, COMMAND_STAY);
    b.send(command(COMMAND_FOLLOW)).await;
    handled(&mut b).await;
    let still = on(&map, move |m| pets::state(m, brute).unwrap().command).await;
    assert_eq!(still, COMMAND_STAY, "not Bob's to command");
    a.send(command(COMMAND_FOLLOW)).await;
    handled(&mut a).await;
    let follows = on(&map, move |m| pets::state(m, brute).unwrap().command).await;
    assert_eq!(follows, COMMAND_FOLLOW);
}
