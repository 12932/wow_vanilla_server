//! Phase 3's fifth round of combat tests, a group's kill (Thistle Tea's
//! `group_reward_test.exs` and `kill_reward_test.exs`): the dead not yet
//! released take no XP and leave the living's share whole, but a dead
//! original tagger still counts in the split.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey, combat};
use crate::game::tests_p3_combat::{self as base, *};

/// Alice and Bob, grouped, beside each other.
async fn group() -> (std::sync::Arc<crate::game::Realm>, Client, Client) {
    let realm = harness::realm_with(base::data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0, START.1, START.2).await;
    a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
    b.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    a.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
        _ => None,
    })
    .await;
    (realm, a, b)
}

fn xp_for(victim: Guid) -> impl FnMut(&ServerOpcodeMessage) -> Option<u32> {
    move |m| match m {
        ServerOpcodeMessage::SMSG_LOG_XPGAIN(x) if x.target == victim => Some(x.total_exp),
        _ => None,
    }
}

/// Bob killed by `brute`, not released.
fn kill_bob(m: &mut Map, gb: Guid, brute: Guid) {
    combat::deal_damage(m, brute, gb, 1_000_000);
}

/// Thistle Tea's "gives an unreleased member quest credit without reducing
/// living XP", "separates living XP, unreleased quest credit ..." and
/// "retains the original dead tagger in the XP denominator": with Bob dead
/// and not released, Alice's kill gives her the XP of one alone and Bob
/// none; with Bob dead but the one who tagged the creature, Alice's share
/// is the one of two, and Bob still has none.
#[tokio::test]
async fn the_unreleased_dead_take_no_share_unless_they_tagged_the_kill() {
    let (realm, mut a, mut b) = group().await;
    let (ga, gb) = (a.guid, b.guid);
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (first, second, alone, halved) = map
        .query(move |m| {
            let first = spawn(m, ga, BRUTE, 20, 3.0);
            let second = spawn(m, ga, BRUTE, 20, 3.0);
            let killer = spawn(m, ga, BRUTE, 20, 3.0);
            kill_bob(m, gb, killer);
            let o = combat::kill_options(m, first);
            let alone = combat::group_shares(&[1], 20, &o).0[0];
            let halved = combat::group_shares(&[1, 1], 20, &o).0[0];
            // Alice tags and kills the first; Bob had tagged the second.
            combat::deal_damage(m, ga, first, 1_000_000);
            m.creature_mut(second).unwrap().tapped_by = Some(gb);
            combat::deal_damage(m, ga, second, 1_000_000);
            (first, second, alone, halved)
        })
        .await
        .unwrap();
    assert_ne!(alone, halved);
    let got = a.wait_for("the first kill's XP", xp_for(first)).await;
    assert_eq!(got, alone, "the XP of one alone");
    let got = a.wait_for("the second kill's XP", xp_for(second)).await;
    assert_eq!(got, halved, "the dead tagger counted");
    for v in [first, second] {
        let none = b
            .gets(std::time::Duration::from_millis(100), xp_for(v))
            .await;
        assert_eq!(none, None, "the dead take none");
    }
}
