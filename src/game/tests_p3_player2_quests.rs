//! Phase 3's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/quest_sharing_test.exs`
//! (docs/parity/matrix.csv): what becomes of a shared quest's offer
//! between its sharing and its taking.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::npc::Quest;
use crate::game::harness::{self, Client};
use crate::game::map::questshare::QUEST_FLAGS_SHARABLE;
use crate::game::map::{Map, MapHandle, MapKey};
use crate::game::store::Logged;
use crate::game::tests_p3_combat::START;
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_player2_ammo::handled;

/// A quest to share; two of one exclusive group; one of an event's.
const PLAIN: u32 = 9_400;
const ONE: u32 = 9_401;
const OTHER: u32 = 9_402;
const FESTIVE: u32 = 9_403;
const FESTIVAL: u32 = 940;
/// A quest that gives a token, and a filler for the bags.
const SUPPLIED: u32 = 9_404;
const TOKEN: u32 = 64_500;
const FILLER: u32 = 64_501;

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    for (id, exclusive) in [
        (PLAIN, 0),
        (ONE, ONE as i32),
        (OTHER, ONE as i32),
        (FESTIVE, 0),
    ] {
        d.npc.quests.insert(
            id,
            Quest {
                id,
                title: format!("Quest {id}"),
                flags: QUEST_FLAGS_SHARABLE,
                exclusive_group: exclusive,
                min_level: 1,
                ..Default::default()
            },
        );
    }
    d.world.event_quests.insert(FESTIVE, FESTIVAL);
    d.npc.quests.insert(
        SUPPLIED,
        Quest {
            id: SUPPLIED,
            title: "Supplied".into(),
            flags: QUEST_FLAGS_SHARABLE,
            min_level: 1,
            source_item: TOKEN,
            source_item_count: 1,
            ..Default::default()
        },
    );
    for entry in [TOKEN, FILLER] {
        d.items.insert(
            entry,
            crate::game::data::ItemTemplate {
                entry,
                name: format!("Item {entry}"),
                display_id: 1000,
                stackable: 1,
                allowable_class: -1,
                allowable_race: -1,
                ..Default::default()
            },
        );
    }
    d
}

/// Alice with every quest under way, and Bob in her group beside her.
async fn party() -> (Arc<Realm>, Client, Guid, Client, Guid, MapHandle) {
    let realm = harness::realm_with(world());
    realm.social.events.lock().insert(FESTIVAL);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    b.heartbeat(START.0 + 1.0, START.1, START.2).await;
    a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
    b.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    handled(&mut b).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        for quest in [PLAIN, ONE, FESTIVE, SUPPLIED] {
            p.journal.add(Logged {
                quest,
                ..Default::default()
            });
        }
    })
    .await;
    (realm, a, ga, b, gb, map)
}

/// `a` shares `quest`; whether it is offered to `gb`.
async fn share(a: &mut Client, map: &MapHandle, gb: Guid, quest: u32) -> bool {
    a.send(CMSG_PUSHQUESTTOPARTY { quest_id: quest }).await;
    handled(a).await;
    a.clear();
    on(map, move |m| m.players[&gb].misc.quest_shared.is_some()).await
}

/// `b` takes `quest` from `ga`; whether it is in their log after.
async fn take(b: &mut Client, map: &MapHandle, ga: Guid, gb: Guid, quest: u32) -> bool {
    b.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: ga,
        quest_id: quest,
    })
    .await;
    handled(b).await;
    on(map, move |m| {
        let p = &m.players[&gb];
        p.journal.has(quest) && p.misc.quest_shared.is_none()
    })
    .await
}

fn offer(m: &Map, g: Guid) -> Option<(Guid, u32)> {
    m.players[&g].misc.quest_shared
}

/// What `a` heard of the share since it last looked.
async fn heard(a: &mut Client) -> Vec<QuestPartyMessage> {
    handled(a).await;
    let mut out = Vec::new();
    for m in std::mem::take(&mut a.backlog) {
        if let ServerOpcodeMessage::MSG_QUEST_PUSH_RESULT(r) = m {
            out.push(r.message);
        }
    }
    out
}

/// Thistle Tea's "declines only the pending offer and ignores claimed
/// client acceptance": a member's client saying it took the quest is not
/// passed on, and the offer stands; declining is, and the offer goes.
#[tokio::test]
async fn a_member_declines_an_offer_but_cannot_claim_to_take_it() {
    let (_realm, mut a, ga, mut b, gb, map) = party().await;
    assert!(share(&mut a, &map, gb, PLAIN).await);
    b.send(MSG_QUEST_PUSH_RESULT {
        guid: ga,
        message: QuestPartyMessage::AcceptQuest,
    })
    .await;
    handled(&mut b).await;
    assert_eq!(heard(&mut a).await, vec![], "a claimed acceptance");
    assert_eq!(on(&map, move |m| offer(m, gb)).await, Some((ga, PLAIN)));
    b.send(MSG_QUEST_PUSH_RESULT {
        guid: ga,
        message: QuestPartyMessage::DeclineQuest,
    })
    .await;
    handled(&mut b).await;
    assert_eq!(heard(&mut a).await, vec![QuestPartyMessage::DeclineQuest]);
    assert_eq!(on(&map, move |m| offer(m, gb)).await, None);
}

/// Thistle Tea's "rechecks exclusive choices acquired after a share
/// offer" and "rejects a shared offer when its event ends before
/// acceptance": a quest of an exclusive group the member took another of
/// since, or whose event has ended since, is not taken, and the sharer
/// hears it can't be.
#[tokio::test]
async fn a_shared_quest_is_judged_again_when_it_is_taken() {
    let (realm, mut a, ga, mut b, gb, map) = party().await;
    assert!(share(&mut a, &map, gb, ONE).await);
    on(&map, move |m| {
        m.players.get_mut(&gb).unwrap().journal.add(Logged {
            quest: OTHER,
            ..Default::default()
        });
    })
    .await;
    assert!(
        !take(&mut b, &map, ga, gb, ONE).await,
        "its other taken since"
    );
    assert_eq!(heard(&mut a).await, vec![QuestPartyMessage::CantTakeQuest]);
    assert_eq!(on(&map, move |m| offer(m, gb)).await, None);
    assert!(share(&mut a, &map, gb, FESTIVE).await);
    realm.social.events.lock().remove(&FESTIVAL);
    assert!(!take(&mut b, &map, ga, gb, FESTIVE).await, "its event over");
    assert_eq!(heard(&mut a).await, vec![QuestPartyMessage::CantTakeQuest]);
}

/// Thistle Tea's "invalidates abandoned quests and changed group
/// membership" and "monitors the sharer's owner": an offer of a quest the
/// sharer has since abandoned, from one no longer in the group, or from
/// one gone from the world, is taken by no one.
#[tokio::test]
async fn an_offer_goes_with_the_sharers_quest_group_and_presence() {
    let (_realm, mut a, ga, mut b, gb, map) = party().await;
    assert!(share(&mut a, &map, gb, PLAIN).await);
    a.send(CMSG_QUESTLOG_REMOVE_QUEST { slot: 0 }).await;
    handled(&mut a).await;
    assert!(!take(&mut b, &map, ga, gb, PLAIN).await, "abandoned");
    assert!(share(&mut a, &map, gb, ONE).await);
    a.disconnect();
    let gone = harness::WAIT;
    let deadline = tokio::time::Instant::now() + gone;
    while on(&map, move |m| m.players.contains_key(&ga)).await {
        assert!(tokio::time::Instant::now() < deadline, "never gone");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(!take(&mut b, &map, ga, gb, ONE).await, "the sharer gone");
}

/// Thistle Tea's "invalidates ... changed group membership": the member
/// leaves the group, and the offer is not taken.
#[tokio::test]
async fn an_offer_goes_when_the_member_leaves_the_group() {
    let (_realm, mut a, ga, mut b, gb, map) = party().await;
    assert!(share(&mut a, &map, gb, PLAIN).await);
    b.send(CMSG_GROUP_DISBAND {}).await;
    handled(&mut b).await;
    assert!(!take(&mut b, &map, ga, gb, PLAIN).await, "out of the group");
}

/// Fill `g`'s bags.
fn fill(m: &mut Map, g: Guid) {
    let d = m.realm.data.clone();
    let p = &m.players[&g];
    let free = p
        .carry_slots(&d)
        .iter()
        .filter(|(b, s)| p.item_at(*b, *s).is_none())
        .count() as u32;
    crate::game::map::inventory::add_item(m, g, FILLER, free, false).unwrap();
}

/// Thistle Tea's "accepts with full bags when the source item is already
/// in the bank" and "does not add a quest or acknowledge acceptance when
/// source items cannot fit", as mangos's `CanAddQuest` has them: with the
/// quest's token in the bank, a member with full bags takes it, given no
/// second token, and abandoning it takes the banked one; with no token
/// and no room, the quest isn't taken, the offer goes, and the sharer
/// hears it can't be.
#[tokio::test]
async fn a_shared_quests_item_is_given_only_if_wanted_and_room_is_had() {
    let (_realm, mut a, ga, mut b, gb, map) = party().await;
    on(&map, move |m| {
        crate::game::map::inventory::add_item(m, gb, TOKEN, 1, false).unwrap();
        let p = m.players.get_mut(&gb).unwrap();
        let i = p.items.iter().position(|i| i.entry == TOKEN).unwrap();
        p.items[i].slot = crate::game::map::inventory::BANK_START;
        fill(m, gb);
    })
    .await;
    assert!(share(&mut a, &map, gb, SUPPLIED).await);
    b.clear();
    assert!(
        take(&mut b, &map, ga, gb, SUPPLIED).await,
        "taken, bags full"
    );
    assert_eq!(on(&map, move |m| m.players[&gb].count_of(TOKEN)).await, 1);
    let pushed = b
        .gets(std::time::Duration::from_millis(1), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(_)).then_some(())
        })
        .await;
    assert!(pushed.is_none(), "no second token");
    assert_eq!(heard(&mut a).await, vec![QuestPartyMessage::AcceptQuest]);
    b.send(CMSG_QUESTLOG_REMOVE_QUEST { slot: 0 }).await;
    handled(&mut b).await;
    assert_eq!(
        on(&map, move |m| m.players[&gb].count_of(TOKEN)).await,
        0,
        "the banked token goes with the quest"
    );
    assert!(share(&mut a, &map, gb, SUPPLIED).await);
    b.clear();
    assert!(
        !take(&mut b, &map, ga, gb, SUPPLIED).await,
        "no room for it"
    );
    b.wait_for("the inventory failure", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(_)).then_some(())
    })
    .await;
    assert_eq!(
        on(&map, move |m| offer(m, gb)).await,
        None,
        "the offer gone"
    );
    assert_eq!(heard(&mut a).await, vec![QuestPartyMessage::CantTakeQuest]);
}
