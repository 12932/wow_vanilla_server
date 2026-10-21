//! Round 11h: the meeting stones' queue as mangos-classic's LFG runs it
//! (Thistle Tea's `MeetingStoneTest` rows on roles and group changes): a
//! class that fills a role best goes first until another has waited half
//! an hour; a hybrid is placed in one role; the queue follows a party's
//! new leader, its leader leaving, a kick, a member leaving, an invitation
//! that makes it whole and a raid.

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
use crate::game::map::meeting_stones::{self, Queued, sweep};
use crate::game::registry::Team;

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const STONE: u32 = 178_824;
const AREA: u32 = 1581;

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
    for race in 1..=8 {
        for class in 1..=11 {
            d.start.insert(
                (race, class),
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
    }
    d.misc.meeting_stones.insert(
        STONE,
        MeetingStone {
            min_level: 1,
            max_level: 60,
            area: AREA,
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

/// The next queue status `c` is sent: its area and status.
async fn setqueue(c: &mut Client) -> (u32, MeetingStoneStatus) {
    c.wait_for("SMSG_MEETINGSTONE_SETQUEUE", |m| match m {
        ServerOpcodeMessage::SMSG_MEETINGSTONE_SETQUEUE(s) => Some((s.area.as_int(), s.status)),
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

/// `leader` queues their party at the stone, every member told so.
async fn queue_party(leader: &mut Client, members: &mut [&mut Client]) {
    leader.send(CMSG_MEETINGSTONE_JOIN { guid: stone() }).await;
    assert_eq!(
        setqueue(leader).await,
        (AREA, MeetingStoneStatus::JoinedQueue)
    );
    for m in members.iter_mut() {
        assert_eq!(setqueue(m).await, (AREA, MeetingStoneStatus::JoinedQueue));
    }
}

/// Players alone, queued at once (so no sweep runs among them half
/// queued), each `waited` that long.
fn queue_alone(realm: &Arc<Realm>, who: &[(Guid, u8, Duration)]) {
    let now = Instant::now();
    let mut stones = realm.social.meeting_stones.lock();
    for (i, (g, class, waited)) in who.iter().enumerate() {
        stones.queue.push(Queued {
            guid: *g,
            area: AREA,
            team: Team::Alliance,
            class: *class,
            leader: false,
            group: 0,
            // In the order given, a millisecond apart.
            since: now - *waited - Duration::from_millis(100 - i as u64),
            progress_at: now + meeting_stones::PROGRESS_EVERY,
        });
    }
}

fn members(realm: &Arc<Realm>, g: Guid) -> Vec<Guid> {
    realm
        .social
        .groups
        .lock()
        .group_of(g)
        .map(|gr| gr.members.iter().map(|m| m.guid).collect())
        .unwrap_or_default()
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

/// A party lacking a tank, a druid waiting longer than a warrior: which
/// of them joins, the druid having waited `druid_waited`.
async fn tank_for(druid_waited: Duration) -> (bool, bool) {
    let realm = harness::realm_with(data());
    let mut lead = player(&realm, "Pria", Race::Human, Class::Priest).await;
    let mut mage = player(&realm, "Maga", Race::Human, Class::Mage).await;
    let mut rogue = player(&realm, "Roga", Race::Human, Class::Rogue).await;
    let mut hunter = player(&realm, "Hunta", Race::Dwarf, Class::Hunter).await;
    let druid = player(&realm, "Druida", Race::NightElf, Class::Druid).await;
    let warrior = player(&realm, "Warra", Race::Human, Class::Warrior).await;
    party(&realm, lead.guid, &[mage.guid, rogue.guid, hunter.guid]);
    queue_party(&mut lead, &mut [&mut mage, &mut rogue, &mut hunter]).await;
    queue_alone(
        &realm,
        &[
            (druid.guid, 11, druid_waited),
            (warrior.guid, 1, Duration::ZERO),
        ],
    );
    sweep(&realm);
    let made = members(&realm, lead.guid);
    (made.contains(&druid.guid), made.contains(&warrior.guid))
}

/// Thistle Tea's "prefers class specialists until a player earns
/// long-wait priority" (mangos's `LFGQueue::FindRoleToGroup` with
/// `LFGMgr::GetPriority` and `hasQueuePriority`): a priest, a mage, a
/// rogue and a hunter lack a tank; a warrior, a tank first of all, is
/// taken over a druid queued before them, until the druid has waited
/// half an hour.
#[tokio::test]
async fn a_specialist_tanks_until_a_hybrid_has_waited_half_an_hour() {
    assert_eq!(
        tank_for(Duration::from_secs(60)).await,
        (false, true),
        "the warrior, a tank first of all"
    );
    assert_eq!(
        tank_for(meeting_stones::PRIORITY_AFTER + Duration::from_secs(1)).await,
        (true, false),
        "the druid, after half an hour"
    );
}

/// Thistle Tea's "assigns hybrids once and protects ordinary party
/// mutations" (mangos's `Group::CalculateLFGRoles`): a druid, a warrior
/// and a priest lack only damage, each placed once (the warrior tanks,
/// the druid heals); a druid leading a priest tanks, so their party takes
/// three damage dealers and no warrior, though the warrior waited
/// longest; made whole, it is complete, and the warrior is still queued
/// alone.
#[tokio::test]
async fn a_hybrid_fills_one_role_and_the_party_what_it_lacks() {
    use meeting_stones::{DAMAGE, HEALER, TANK, open_roles};
    assert_eq!(
        open_roles(&[11, 1, 5]),
        DAMAGE,
        "the druid heals, the warrior tanks"
    );
    assert_eq!(
        open_roles(&[11, 5]),
        DAMAGE,
        "the druid tanks, the priest heals"
    );
    assert_eq!(
        open_roles(&[8, 8, 8]),
        TANK | HEALER,
        "no fourth damage dealer"
    );

    let realm = harness::realm_with(data());
    let mut lead = player(&realm, "Druida", Race::NightElf, Class::Druid).await;
    let mut priest = player(&realm, "Pria", Race::Human, Class::Priest).await;
    let warrior = player(&realm, "Warra", Race::Human, Class::Warrior).await;
    let mage = player(&realm, "Maga", Race::Human, Class::Mage).await;
    let rogue = player(&realm, "Roga", Race::Human, Class::Rogue).await;
    let hunter = player(&realm, "Hunta", Race::Dwarf, Class::Hunter).await;
    party(&realm, lead.guid, &[priest.guid]);
    queue_party(&mut lead, &mut [&mut priest]).await;
    queue_alone(
        &realm,
        &[
            (warrior.guid, 1, Duration::from_secs(60)),
            (mage.guid, 8, Duration::ZERO),
            (rogue.guid, 4, Duration::ZERO),
            (hunter.guid, 3, Duration::ZERO),
        ],
    );
    sweep(&realm);
    let mut made = members(&realm, lead.guid);
    made.sort_by_key(|g| g.guid());
    let mut want = vec![lead.guid, priest.guid, mage.guid, rogue.guid, hunter.guid];
    want.sort_by_key(|g| g.guid());
    assert_eq!(made, want, "three damage dealers, no warrior");
    lead.wait_for("SMSG_MEETINGSTONE_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_MEETINGSTONE_COMPLETE).then_some(())
    })
    .await;
    assert_eq!(
        queued(&realm),
        vec![(warrior.guid, false)],
        "the warrior waits"
    );
}

/// Three in a party, its leader queued for it.
async fn three(realm: &Arc<Realm>) -> (Client, Client, Client) {
    let mut a = player(realm, "Alla", Race::Human, Class::Warrior).await;
    let mut b = player(realm, "Bella", Race::Human, Class::Priest).await;
    let mut c = player(realm, "Cella", Race::Human, Class::Mage).await;
    party(realm, a.guid, &[b.guid, c.guid]);
    queue_party(&mut a, &mut [&mut b, &mut c]).await;
    (a, b, c)
}

/// Thistle Tea's "cancels a departed leader's queue but preserves an
/// explicit leadership transfer" (mangos: `ChangeLeader` leaves the queue
/// be; `RemoveMember` with the leader changed withdraws the party): led
/// by another now, the party stays queued, a member asking is told so;
/// its new leader leaving withdraws it, each member told so.
#[tokio::test]
async fn a_new_leader_keeps_the_queue_and_one_leaving_ends_it() {
    let realm = harness::realm_with(data());
    let (mut a, mut b, mut c) = three(&realm).await;
    let (ga, gb) = (a.guid, b.guid);
    a.send(CMSG_GROUP_SET_LEADER { guid: gb }).await;
    a.wait_for("SMSG_GROUP_SET_LEADER", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_SET_LEADER(_)).then_some(())
    })
    .await;
    assert_eq!(
        queued(&realm),
        vec![(gb, true)],
        "queued under its new leader"
    );
    c.send(CMSG_MEETINGSTONE_INFO {}).await;
    assert_eq!(
        setqueue(&mut c).await,
        (AREA, MeetingStoneStatus::JoinedQueue)
    );

    // The leader goes.
    b.send(CMSG_GROUP_DISBAND {}).await;
    assert_eq!(setqueue(&mut b).await, (0, MeetingStoneStatus::None));
    for x in [&mut a, &mut c] {
        assert_eq!(setqueue(x).await, (0, MeetingStoneStatus::LeaveQueue));
    }
    assert!(queued(&realm).is_empty(), "withdrawn");
    assert!(
        members(&realm, ga).contains(&c.guid),
        "the rest still a party"
    );
    a.send(CMSG_MEETINGSTONE_INFO {}).await;
    assert_eq!(setqueue(&mut a).await, (0, MeetingStoneStatus::None));
}

/// Thistle Tea's "kicking a member withdraws the party and restores the
/// removed player's solo queue" (mangos's `RemoveMember` by a kick): the
/// rest hear the party removed, and are out of the queue; the one kicked
/// looks for a new party in the same dungeon's queue, alone.
#[tokio::test]
async fn a_kick_withdraws_the_party_and_queues_the_one_kicked_alone() {
    let realm = harness::realm_with(data());
    let (mut a, mut b, mut c) = three(&realm).await;
    let gc = c.guid;
    a.send(CMSG_GROUP_UNINVITE_GUID { guid: gc }).await;
    for x in [&mut a, &mut b] {
        assert_eq!(
            setqueue(x).await,
            (0, MeetingStoneStatus::PartyMemberRemovedPartyRemoved)
        );
        assert_eq!(setqueue(x).await, (0, MeetingStoneStatus::LeaveQueue));
    }
    assert_eq!(
        setqueue(&mut c).await,
        (AREA, MeetingStoneStatus::LookingForNewPartyInQueue)
    );
    assert_eq!(
        setqueue(&mut c).await,
        (AREA, MeetingStoneStatus::JoinedQueue)
    );
    assert_eq!(queued(&realm), vec![(gc, false)], "the one kicked, alone");
    c.send(CMSG_MEETINGSTONE_INFO {}).await;
    assert_eq!(
        setqueue(&mut c).await,
        (AREA, MeetingStoneStatus::JoinedQueue)
    );
    a.send(CMSG_MEETINGSTONE_INFO {}).await;
    assert_eq!(setqueue(&mut a).await, (0, MeetingStoneStatus::None));
}

async fn invite(leader: &mut Client, member: &mut Client, name: &str) {
    leader.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    member
        .wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
    member.send(CMSG_GROUP_ACCEPT {}).await;
}

/// Thistle Tea's "tracks removals, manual completion, and raid
/// conversion" (mangos's `RemoveMember` by leaving, `UpdateGroup` and
/// `RemoveGroupFromQueue`): one leaving is out, the rest searching on
/// without them; invited members join the search, the party made whole
/// is complete; a party made a raid is withdrawn.
#[tokio::test]
async fn the_queue_follows_a_member_leaving_an_invitation_and_a_raid() {
    let realm = harness::realm_with(data());
    let (mut a, mut b, mut c) = three(&realm).await;
    let (ga, gb) = (a.guid, b.guid);
    c.send(CMSG_GROUP_DISBAND {}).await;
    assert_eq!(setqueue(&mut c).await, (0, MeetingStoneStatus::None));
    for x in [&mut a, &mut b] {
        assert_eq!(
            setqueue(x).await,
            (AREA, MeetingStoneStatus::PartyMemberLeftLfg)
        );
    }
    assert_eq!(queued(&realm), vec![(ga, true)], "still searching");
    assert_eq!(members(&realm, ga), vec![ga, gb]);

    // Three invited: each joins the search, and the third makes it whole.
    let mut d = player(&realm, "Della", Race::Human, Class::Rogue).await;
    let mut e = player(&realm, "Ella", Race::Human, Class::Mage).await;
    let mut f = player(&realm, "Fella", Race::Human, Class::Warlock).await;
    invite(&mut a, &mut d, "Della").await;
    assert_eq!(
        setqueue(&mut d).await,
        (AREA, MeetingStoneStatus::JoinedQueue)
    );
    invite(&mut a, &mut e, "Ella").await;
    assert_eq!(
        setqueue(&mut e).await,
        (AREA, MeetingStoneStatus::JoinedQueue)
    );
    invite(&mut a, &mut f, "Fella").await;
    for x in [&mut a, &mut b, &mut d, &mut e, &mut f] {
        x.wait_for("SMSG_MEETINGSTONE_COMPLETE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_MEETINGSTONE_COMPLETE).then_some(())
        })
        .await;
        let s = setqueue(x).await;
        assert_eq!(s, (0, MeetingStoneStatus::None));
    }
    assert!(queued(&realm).is_empty(), "complete");

    // A party queued, then made a raid.
    let realm = harness::realm_with(data());
    let (mut a, mut b, mut c) = three(&realm).await;
    a.send(CMSG_GROUP_RAID_CONVERT {}).await;
    for x in [&mut a, &mut b, &mut c] {
        assert_eq!(setqueue(x).await, (0, MeetingStoneStatus::LeaveQueue));
    }
    assert!(queued(&realm).is_empty(), "a raid can't search");
}
