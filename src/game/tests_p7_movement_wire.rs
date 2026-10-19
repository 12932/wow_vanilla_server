//! Round 7's wire layouts, ported from Thistle Tea's
//! `test/game/network/message/*_test.exs` (docs/parity/matrix.csv).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

/// `MSG_PVP_LOG_DATA` with a Warsong Gulch row: the end and its winner,
/// then each player's GUID, rank, killing blows, honorable kills, deaths,
/// bonus honor, and the count of the battleground's own fields before them
/// (its captures and returns), as mangos-zero writes it and
/// `battleground::scoreboard_frame` sends it (Thistle Tea's "encodes the
/// WSG capture and return fields").
#[test]
fn the_scoreboard_carries_warsong_gulchs_captures_and_returns() {
    let frame = crate::game::conn::encode(&MSG_PVP_LOG_DATA_Server {
        status: MSG_PVP_LOG_DATA_Server_BattlegroundEndStatus::Ended {
            winner: BattlegroundWinner::Alliance,
        },
        players: vec![BattlegroundPlayer {
            player: Guid::new(42),
            rank: PvpRank::try_from(4).unwrap(),
            killing_blows: 1,
            honorable_kills: 2,
            deaths: 3,
            bonus_honor: 4,
            fields: vec![5, 6],
        }],
    })
    .unwrap();
    let mut want = vec![1u8, 1];
    want.extend(1u32.to_le_bytes());
    want.extend(42u64.to_le_bytes());
    for n in [4u32, 1, 2, 3, 4, 2, 5, 6] {
        want.extend(n.to_le_bytes());
    }
    assert_eq!(&frame[4..], &want[..]);
}
