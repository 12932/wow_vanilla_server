//! Phase 3's meeting stones, Thistle Tea's `MeetingStoneTest` ported: who
//! may queue (a party's leader, not a raid nor a full party), one entry a
//! player, the leader's to withdraw; five alone made a party of its roles,
//! no fourth damage dealer, each side apart; a party's search heard every
//! five minutes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::misc::MeetingStone;
use crate::game::data::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::meeting_stones::sweep;

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const STONE: u32 = 178_824;

fn data() -> GameData {
    let mut d = GameData::empty();
    d.gameobject_templates.insert(
        STONE,
        GameObjectTemplate {
            entry: STONE,
            kind: 23,
            display_id: 1,
            name: "Meeting Stone".into(),
            size: 1.0,
            ..Default::default()
        },
    );
    d.gameobject_spawns.insert(
        0,
        vec![GameObjectSpawn {
            guid: 30,
            entry: STONE,
            map: 0,
            x: START.0 - 2.0,
            y: START.1,
            z: START.2,
            rotation: [0.0, 0.0, 0.0, 1.0],
            state: 1,
            ..Default::default()
        }],
    );
    // Everyone, an orc too, starts by the stone.
    for key in [(1, 1), (1, 4), (1, 5), (1, 8), (1, 9), (2, 1)] {
        d.start.insert(
            key,
            StartInfo {
                map: 0,
                zone: 12,
                x: START.0,
                y: START.1,
                z: START.2,
                orientation: 0.0,
                items: Vec::new(),
                spells: Vec::new(),
                actions: Vec::new(),
            },
        );
    }
    d.misc.meeting_stones.insert(
        STONE,
        MeetingStone {
            min_level: 1,
            max_level: 60,
            area: 1581,
        },
    );
    d
}

fn stone() -> Guid {
    guid::game_object(STONE, 30)
}

async fn player(realm: &Arc<Realm>, name: &str, race: Race, class: Class) -> Client {
    let mut c = Client::connect(realm, name);
    c.enter(name, race, class).await;
    c.sees(stone()).await;
    c.clear();
    c
}

async fn handled(c: &mut Client) {
    c.send(CMSG_NAME_QUERY { guid: c.guid }).await;
    c.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
}

/// Queue at the stone: the refusal, if any.
async fn join(c: &mut Client) -> Option<MeetingStoneFailure> {
    c.clear();
    c.send(CMSG_MEETINGSTONE_JOIN { guid: stone() }).await;
    handled(c).await;
    c.gets(Duration::from_millis(10), |m| match m {
        ServerOpcodeMessage::SMSG_MEETINGSTONE_JOINFAILED(f) => Some(f.reason),
        _ => None,
    })
    .await
}

/// `leader`'s party made of `members` at once.
fn party(realm: &Arc<Realm>, leader: Guid, members: &[Guid]) {
    let lo = realm.registry.online(leader).unwrap();
    for m in members {
        let mo = realm.registry.online(*m).unwrap();
        assert!(crate::game::social::groups::join_directly(realm, &lo, &mo));
    }
}

fn queued(realm: &Arc<Realm>) -> Vec<(Guid, bool)> {
    let mut q: Vec<(Guid, bool)> = realm
        .social
        .meeting_stones
        .lock()
        .queue
        .iter()
        .map(|q| (q.guid, q.leader))
        .collect();
    q.sort_by_key(|(g, _)| g.guid());
    q
}

/// Thistle Tea's `join/5` and `leave/3`: a member, a raid's leader and a
/// full party's leader refused; a party's leader queues it once, a member
/// can't withdraw it and the leader can; one alone queued twice is in the
/// queue once.
#[tokio::test]
async fn a_stone_queues_a_partys_leader_once() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice", Race::Human, Class::Warrior).await;
    let mut b = player(&realm, "Bob", Race::Human, Class::Priest).await;
    let others = [
        player(&realm, "Cid", Race::Human, Class::Mage).await,
        player(&realm, "Dee", Race::Human, Class::Rogue).await,
        player(&realm, "Eve", Race::Human, Class::Warlock).await,
    ];
    let (ga, gb) = (a.guid, b.guid);
    party(&realm, ga, &[gb]);
    assert_eq!(
        join(&mut b).await,
        Some(MeetingStoneFailure::MeetingstoneFailPartyleader)
    );
    assert_eq!(join(&mut a).await, None);
    assert_eq!(join(&mut a).await, None);
    assert_eq!(queued(&realm), vec![(ga, true)], "the party once");
    b.send(CMSG_MEETINGSTONE_LEAVE {}).await;
    handled(&mut b).await;
    assert_eq!(
        queued(&realm),
        vec![(ga, true)],
        "not the member's to withdraw"
    );
    a.send(CMSG_MEETINGSTONE_LEAVE {}).await;
    handled(&mut a).await;
    assert!(queued(&realm).is_empty());

    // Full, and a raid.
    party(
        &realm,
        ga,
        &others.iter().map(|c| c.guid).collect::<Vec<_>>(),
    );
    assert_eq!(
        join(&mut a).await,
        Some(MeetingStoneFailure::MeetingstoneFailFullGroup)
    );
    a.send(CMSG_GROUP_RAID_CONVERT {}).await;
    handled(&mut a).await;
    assert_eq!(
        join(&mut a).await,
        Some(MeetingStoneFailure::MeetingstoneFailRaidGroup)
    );
    assert!(queued(&realm).is_empty());
}

/// Thistle Tea's `refresh/4`: of six alone, the longest waiting leads a
/// party of a tank, a healer and three damage dealers, the sixth left
/// waiting; one of the other side, at the same stone, waits apart; and a
/// party with its three damage dealers takes no fourth.
#[tokio::test]
async fn those_alone_make_a_party_of_its_roles() {
    let realm = harness::realm_with(data());
    let mut solos = Vec::new();
    for (name, class) in [
        ("Alice", Class::Warrior),
        ("Cid", Class::Mage),
        ("Dee", Class::Rogue),
        ("Bob", Class::Priest),
        ("Eve", Class::Warlock),
        ("Fay", Class::Mage),
    ] {
        let mut c = player(&realm, name, Race::Human, class).await;
        assert_eq!(join(&mut c).await, None);
        solos.push(c);
    }
    let mut orc = player(&realm, "Grom", Race::Orc, Class::Warrior).await;
    assert_eq!(join(&mut orc).await, None);
    sweep(&realm);
    let made = realm
        .social
        .groups
        .lock()
        .group_of(solos[0].guid)
        .map(|g| g.members.iter().map(|m| m.guid).collect::<Vec<_>>())
        .unwrap();
    let want: Vec<Guid> = solos[..5].iter().map(|c| c.guid).collect();
    let mut have = made.clone();
    have.sort_by_key(|g| g.guid());
    let mut want_sorted = want.clone();
    want_sorted.sort_by_key(|g| g.guid());
    assert_eq!(have, want_sorted, "a tank, a healer and three damage");
    assert_eq!(
        queued(&realm),
        vec![(solos[5].guid, false), (orc.guid, false)],
        "the sixth, and the orc apart"
    );

    // A warrior and three damage dealers: no fourth damage dealer, though
    // two wait.
    let realm = harness::realm_with(data());
    let mut lead = player(&realm, "Alice", Race::Human, Class::Warrior).await;
    let mut dps = Vec::new();
    for name in ["Cid", "Dee", "Eve", "Fay", "Gil"] {
        dps.push(player(&realm, name, Race::Human, Class::Mage).await);
    }
    party(&realm, lead.guid, &[dps[0].guid, dps[1].guid, dps[2].guid]);
    assert_eq!(join(&mut lead).await, None);
    for c in &mut dps[3..] {
        assert_eq!(join(c).await, None);
    }
    sweep(&realm);
    let size = realm
        .social
        .groups
        .lock()
        .group_of(lead.guid)
        .map(|g| g.members.len());
    assert_eq!(size, Some(4), "no fourth damage dealer");
}

/// Thistle Tea's progress: a party still searching hears so every five
/// minutes, each of its members.
#[tokio::test]
async fn a_partys_search_is_heard_every_five_minutes() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice", Race::Human, Class::Warrior).await;
    let mut b = player(&realm, "Bob", Race::Human, Class::Priest).await;
    party(&realm, a.guid, &[b.guid]);
    assert_eq!(join(&mut a).await, None);
    b.clear();
    sweep(&realm);
    let early = b
        .gets(Duration::from_millis(50), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_MEETINGSTONE_IN_PROGRESS).then_some(())
        })
        .await;
    assert!(early.is_none(), "not yet");
    for q in realm.social.meeting_stones.lock().queue.iter_mut() {
        q.progress_at = Instant::now() - Duration::from_secs(1);
    }
    sweep(&realm);
    for c in [&mut a, &mut b] {
        c.wait_for("SMSG_MEETINGSTONE_IN_PROGRESS", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_MEETINGSTONE_IN_PROGRESS).then_some(())
        })
        .await;
    }
}
