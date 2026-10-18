//! Phase 3's fourth round of the network message tests, ported from
//! Thistle Tea's `test/game/network/message` (docs/parity/matrix.csv):
//! the auction house's notices and the group roster's end.

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::harness;
use crate::game::tests_p3_core_auction::{bid, data, goods, ok, player, sell, set_money};

/// The notices a bidder and a seller are told: (bid, outbid, bidder).
fn bidder_notice(m: &ServerOpcodeMessage) -> Option<(u32, u32, u64)> {
    match m {
        ServerOpcodeMessage::SMSG_AUCTION_BIDDER_NOTIFICATION(n) => {
            Some((n.won, n.out_bid, n.bidder.guid()))
        }
        _ => None,
    }
}

fn owner_notice(m: &ServerOpcodeMessage) -> Option<(u32, u32, u64)> {
    match m {
        ServerOpcodeMessage::SMSG_AUCTION_OWNER_NOTIFICATION(n) => {
            Some((n.bid, n.auction_out_bid, n.bidder.guid()))
        }
        _ => None,
    }
}

/// Thistle Tea's "encodes bidder, owner, and removal notices", as its
/// `ClientProjection.notice` and mangos-classic's
/// `SendAuctionBidderNotification` and `SendAuctionOwnerNotification`
/// fill them: the one outbid is told the bid they lost (not 0, which the
/// client shows as "You won an auction"), the winner is told 0, the
/// seller is told of a bid with its bidder and of the sale without one
/// (which the client shows as "Your auction sold"), and an auction gone
/// unsold is told removed; each names the item.
#[tokio::test]
async fn the_auction_houses_notices_say_what_happened() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "A", "Alice").await;
    let mut b = player(&realm, "B", "Bob").await;
    let mut c = player(&realm, "C", "Cid").await;
    let item = goods(&realm, a.guid).await;
    let (id, _) = sell(&mut a, item, 100, 1_000, 120).await;
    a.clear();
    assert!(ok(&bid(&mut b, id, 100).await));
    assert_eq!(
        a.wait_for("the bid's notice", owner_notice).await,
        (100, 5, b.guid.guid()),
        "a bid, by Bob"
    );
    b.clear();
    assert!(ok(&bid(&mut c, id, 105).await));
    let (lost, _, _) = b.wait_for("the outbid notice", bidder_notice).await;
    assert_eq!(lost, 100, "outbid, with the bid Bob lost");
    a.clear();
    set_money(&realm, c.guid, 10_000).await;
    assert!(ok(&bid(&mut c, id, 1_000).await), "bought out");
    let (won, _, _) = c.wait_for("the won notice", bidder_notice).await;
    assert_eq!(won, 0, "won");
    assert_eq!(
        a.wait_for("the sale's notice", owner_notice).await,
        (1_000, 0, 0),
        "sold, to no bidder named"
    );
    // One unsold, gone at its deadline.
    let unsold = goods(&realm, a.guid).await;
    let (id, _) = sell(&mut a, unsold, 100, 0, 120).await;
    a.clear();
    realm
        .social
        .auctions
        .lock()
        .auctions
        .get_mut(&id)
        .unwrap()
        .expires_at = crate::game::social::mail::now() - 1;
    crate::game::social::auction::sweep(&realm);
    let removed = a
        .wait_for("the removed notice", |m| match m {
            ServerOpcodeMessage::SMSG_AUCTION_REMOVED_NOTIFICATION(n) => {
                Some((n.item, n.item_template))
            }
            _ => None,
        })
        .await;
    assert_eq!(removed, (id, crate::game::tests_p3_core_auction::GOODS));
}

/// Thistle Tea's "ends after the leader guid when clearing a group": the
/// roster that tells one their group is gone is its type, flags, no
/// members and a leader of none, with no loot rules after (mangos-classic's
/// empty `SMSG_GROUP_LIST`).
#[tokio::test]
async fn a_groups_empty_roster_ends_at_its_leader() {
    let realm = harness::realm();
    let mut a = harness::Client::connect(&realm, "A");
    a.enter("Ana", Race::Human, Class::Warrior).await;
    let mut b = harness::Client::connect(&realm, "B");
    b.enter("Bea", Race::Human, Class::Warrior).await;
    a.send(CMSG_GROUP_INVITE { name: "Bea".into() }).await;
    b.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    b.wait_for("the roster", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
        _ => None,
    })
    .await;
    b.clear();
    a.send(CMSG_GROUP_DISBAND {}).await;
    let empty = b
        .wait_frame("the empty roster", 0x07D, |f| f[2..6] == [0, 0, 0, 0])
        .await;
    assert_eq!(empty, [0u8; 14], "type, flags, no members, no leader");
}
