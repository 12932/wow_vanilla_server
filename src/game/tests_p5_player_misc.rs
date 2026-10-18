//! Round 5's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/honor_test.exs` and others
//! (docs/parity/matrix.csv): honor inspection's refusals and layout.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;
use crate::game::tests_p3_combat::START;
use crate::game::tests_p3_entity::on;

const MSG_INSPECT_HONOR_STATS: u16 = 0x2D6;

/// The honor stats `a` is told of `of`, raw, if any.
async fn inspected(a: &mut Client, of: Guid) -> Option<Vec<u8>> {
    a.clear();
    a.send(MSG_INSPECT_HONOR_STATS_Client { guid: of }).await;
    tokio::time::timeout(
        Duration::from_millis(300),
        a.wait_frame("MSG_INSPECT_HONOR_STATS", MSG_INSPECT_HONOR_STATS, |_| true),
    )
    .await
    .ok()
}

/// Thistle Tea's `Honor.inspect_reply` "rejects distant, hostile, offline,
/// and other-instance targets" (with `honor_stats_are_inspected_only_near`
/// for the distant) and "dispatches inspection and encodes the build-5875
/// response": a friend's honor is told in 1.12's layout (the guid, the
/// highest rank, today's honorable and dishonorable kills as two halves,
/// yesterday's, last week's and this week's kills each padded to four
/// bytes, the lifetime kills, the contributions, last week's standing and
/// the rank bar); an enemy one could attack is told nothing (mangos's
/// `CanAttack` refusal), nor is one logged out.
#[tokio::test]
async fn honor_is_inspected_in_its_layout_from_friends_alone() {
    use crate::game::honor::{Day, Entry};
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0 + 2.0, START.1, START.2).await;
    let counter = crate::game::guid::counter(gb);
    let expected = {
        let mut h = realm.social.honor.lock();
        let day = h.day;
        let mut e = Entry {
            guid: counter,
            highest_rank: 9,
            lifetime_honorable_kills: 7,
            lifetime_dishonorable_kills: 8,
            last_week: Day {
                honorable_kills: 5,
                contribution: 10.0,
                ..Default::default()
            },
            last_standing: 12,
            ..Default::default()
        };
        e.days.insert(
            day,
            Day {
                honorable_kills: 3,
                dishonorable_kills: 2,
                ..Default::default()
            },
        );
        e.days.insert(
            day - 1,
            Day {
                honorable_kills: 4,
                contribution: 9.0,
                ..Default::default()
            },
        );
        h.entries.insert(counter, e);
        let p = h.project(counter);
        let mut want = gb.guid().to_le_bytes().to_vec();
        want.push(9);
        want.extend(3u16.to_le_bytes());
        want.extend(2u16.to_le_bytes());
        for kills in [4u32, 5, p.this_week_kills] {
            want.extend(kills.to_le_bytes());
        }
        for v in [7u32, 8, 9, 10, p.this_week_contribution, 12] {
            want.extend(v.to_le_bytes());
        }
        want.push(p.rank_bar);
        want
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(inspected(&mut a, gb).await, Some(expected), "a friend's");
    // A forsaken beside her, both flagged for a fight: one she could attack.
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Grom", Race::Undead, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let hostile = on(&map, move |m| {
        let at = m.players[&ga].position();
        let pvp = crate::game::map::player::UNIT_FLAG_PVP;
        for g in [ga, gc] {
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_flag(UNIT_FIELD_FLAGS, pvp, true);
        }
        m.players.get_mut(&gc).map(|p| p.movement.position = at);
        crate::game::map::combat::can_attack(m, ga, gc)
    })
    .await;
    assert!(hostile, "flagged, she could attack them");
    assert_eq!(inspected(&mut a, gc).await, None, "an enemy's");
    b.send(CMSG_LOGOUT_REQUEST {}).await;
    b.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(
            m,
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE
        )
        .then_some(())
    })
    .await;
    assert_eq!(inspected(&mut a, gb).await, None, "one logged out");
}
