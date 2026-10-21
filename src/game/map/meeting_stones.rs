//! Meeting stones, mangos-classic's LFG (`src/game/LFG`): a player or a
//! party's leader queues at a dungeon's stone; every few seconds each
//! dungeon's queue is matched by role, a party taking the players it lacks
//! (a tank, a healer, then damage; a hybrid placed in one role) and five
//! players alone forming one, a class that fills a role best ahead of one
//! that fills it less until the other has waited half an hour. The queue
//! follows a party's changes: a new leader keeps it queued; the leader
//! leaving, a kick, a raid or disbanding withdraws it; one made whole by
//! an invitation is complete. The queue spans the realm.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::registry::Team;
use crate::game::social::{groups, send_to};

/// How near the stone a player must be to queue.
pub const STONE_YARDS: f32 = 10.0;
const PARTY: usize = groups::PARTY_SIZE;

/// The roles of mangos's `LfgRoles`, as a mask.
pub const TANK: u8 = 0x1;
pub const HEALER: u8 = 0x2;
pub const DAMAGE: u8 = 0x4;
/// The order roles are filled in (mangos's `PotentialRoles`).
const ROLES: [u8; 3] = [TANK, HEALER, DAMAGE];
/// A party's damage dealers (`LFGMgr::GetMaximumDPSSlots`).
const DAMAGE_SLOTS: u32 = 3;
/// After this long in the queue a player is ahead of those who have
/// waited less, whatever their class (mangos's `hasQueuePriority`).
pub const PRIORITY_AFTER: Duration = Duration::from_secs(30 * 60);

/// Every role a class may fill (mangos's `LFGMgr::CalculateRoles`): a
/// druid or a paladin any, a warrior a tank or damage, a priest or a
/// shaman a healer or damage, the rest damage.
pub fn roles(class: u8) -> u8 {
    match class {
        2 | 11 => TANK | HEALER | DAMAGE,
        1 => TANK | DAMAGE,
        5 | 7 => HEALER | DAMAGE,
        3 | 4 | 8 | 9 => DAMAGE,
        _ => 0,
    }
}

/// How well a class fills a role (mangos's `LFGMgr::GetPriority`): 3 a
/// specialist, 2 a hybrid, 1 a priest dealing damage, 0 not at all.
pub fn priority(class: u8, role: u8) -> u8 {
    match (role, class) {
        (TANK, 1) => 3,
        (TANK, 2 | 11) => 2,
        (HEALER, 2 | 5 | 7 | 11) => 3,
        (DAMAGE, 3 | 4 | 8 | 9) => 3,
        (DAMAGE, 1 | 2 | 7 | 11) => 2,
        (DAMAGE, 5) => 1,
        _ => 0,
    }
}

/// The roles a party of `classes` still lacks, each member given the one
/// role they fill best of those (mangos's `Group::CalculateLFGRoles` and
/// `FillPremadeLFG`): a member takes the first role, tank, healer, damage,
/// that no member not yet placed fills better. A hybrid is placed once.
pub fn open_roles(classes: &[u8]) -> u8 {
    let mut open = TANK | HEALER | DAMAGE;
    let mut damage = 0;
    let mut placed = vec![false; classes.len()];
    for (i, &class) in classes.iter().enumerate() {
        for role in ROLES {
            if roles(class) & role == 0 || open & role == 0 {
                continue;
            }
            let mine = priority(class, role);
            let better = classes
                .iter()
                .enumerate()
                .any(|(j, &o)| j != i && !placed[j] && priority(o, role) > mine);
            if better {
                continue;
            }
            if role == DAMAGE {
                damage += 1;
                if damage >= DAMAGE_SLOTS {
                    open &= !DAMAGE;
                }
            } else {
                open &= !role;
            }
            placed[i] = true;
            break;
        }
    }
    open
}

/// The player alone who fills what a party lacks (`open`), the first role
/// first: who has waited long enough (`PRIORITY_AFTER`) ahead of who has
/// not, then the class that fills it best, then who has waited longest
/// (mangos's `LFGQueue::FindRoleToGroup`). None for a place filled: no
/// second tank or healer, no fourth damage dealer.
fn best_for(open: u8, solos: &[Queued], now: Instant) -> Option<Guid> {
    ROLES.iter().filter(|r| open & **r != 0).find_map(|&role| {
        solos
            .iter()
            .filter(|q| roles(q.class) & role != 0)
            .min_by_key(|q| {
                let waited = now.saturating_duration_since(q.since) >= PRIORITY_AFTER;
                (!waited, std::cmp::Reverse(priority(q.class, role)), q.since)
            })
            .map(|q| q.guid)
    })
}

#[derive(Debug, Clone)]
pub struct Queued {
    /// The player alone, or a party's leader.
    pub guid: Guid,
    pub area: u32,
    pub team: Team,
    pub class: u8,
    /// A party, queued by its leader.
    pub leader: bool,
    /// The party's group (0 for one alone): the queue follows the group,
    /// whoever leads it.
    pub group: u32,
    pub since: Instant,
    /// When a queued party is next told the search goes on.
    pub progress_at: Instant,
}

/// How often a queued party hears the search goes on (Thistle Tea's
/// `MeetingStone`).
pub const PROGRESS_EVERY: Duration = Duration::from_secs(300);

#[derive(Debug, Default)]
pub struct MeetingStones {
    pub queue: Vec<Queued>,
}

impl MeetingStones {
    pub fn of(&self, g: Guid) -> Option<&Queued> {
        self.queue.iter().find(|q| q.guid == g)
    }

    /// Group `id`'s entry, if the party is queued.
    pub fn party(&self, id: u32) -> Option<&Queued> {
        self.queue.iter().find(|q| q.leader && q.group == id)
    }

    fn take_party(&mut self, id: u32) -> Option<Queued> {
        let i = self.queue.iter().position(|q| q.leader && q.group == id)?;
        Some(self.queue.remove(i))
    }

    /// `g`'s entry alone, taken out.
    fn take_solo(&mut self, g: Guid) -> Option<Queued> {
        let i = self.queue.iter().position(|q| !q.leader && q.guid == g)?;
        Some(self.queue.remove(i))
    }
}

fn status(realm: &Realm, g: Guid, area: u32, s: MeetingStoneStatus) {
    send_to(
        realm,
        g,
        &SMSG_MEETINGSTONE_SETQUEUE {
            area: Area::try_from(area).unwrap_or_default(),
            status: s,
        },
    );
}

/// The dungeon `g` is queued for: their own, or their party's.
pub fn area_of(realm: &Realm, g: Guid) -> Option<u32> {
    let group = realm.social.groups.lock().group_of(g).map(|gr| gr.id);
    let stones = realm.social.meeting_stones.lock();
    match group {
        Some(id) => stones.party(id).map(|q| q.area),
        None => stones.of(g).map(|q| q.area),
    }
}

pub fn handle(map: &mut Map, g: Guid, msg: ClientOpcodeMessage) -> Option<ClientOpcodeMessage> {
    use ClientOpcodeMessage as C;
    match msg {
        C::CMSG_MEETINGSTONE_JOIN(m) => join(map, g, m.guid),
        C::CMSG_MEETINGSTONE_LEAVE => leave(&map.realm, g),
        C::CMSG_MEETINGSTONE_INFO => {
            // mangos's `HandleMeetingStoneInfoOpcode`: a party's members
            // see the party's queue.
            match area_of(&map.realm, g) {
                Some(area) => status(&map.realm, g, area, MeetingStoneStatus::JoinedQueue),
                None => status(&map.realm, g, 0, MeetingStoneStatus::None),
            }
        }
        C::CMSG_GROUP_SWAP_SUB_GROUP(m) => {
            if let Some(p) = map.players.get(&g) {
                let me = p.online.clone();
                groups::swap_subgroups(&map.realm, &me, &m.name, &m.swap_with_name);
            }
        }
        other => return Some(other),
    }
    None
}

/// `CMSG_MEETINGSTONE_JOIN`: into the stone's queue, alone or for one's
/// party.
fn join(map: &mut Map, g: Guid, stone: Guid) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let Some(o) = map.gameobject(stone) else {
        return;
    };
    let Some(info) = data.misc.meeting_stones.get(&guid::entry(stone)).copied() else {
        return;
    };
    if crate::game::map::combat::dist(p.position(), o.position) > STONE_YARDS {
        return;
    }
    // Not by the dead, one flying, or one moving another (mangos's
    // `IsSelfMover` and `GetGameObjectIfCanInteractWith`); not at a stone
    // that can't be used (Thistle Tea's `go_flags` 0x10).
    if p.death != crate::game::map::death::Death::Alive
        || p.flight.is_some()
        || p.owned.mover.is_some()
        || o.fields.get_u32(crate::game::fields::GAMEOBJECT_FLAGS)
            & crate::game::map::objects::GO_FLAG_NO_INTERACT
            != 0
    {
        return;
    }
    let level = u32::from(p.level());
    if level < info.min_level || (info.max_level > 0 && level > info.max_level) {
        map.system_message(g, "You are not the right level for this dungeon.");
        return;
    }
    let (team, class) = (p.team, p.row.class);
    let party = {
        let groups = map.realm.social.groups.lock();
        groups
            .group_of(g)
            .map(|gr| (gr.leader, gr.raid, gr.members.len()))
    };
    if let Some((leader, raid, size)) = party {
        let failure = if leader != g {
            Some(MeetingStoneFailure::MeetingstoneFailPartyleader)
        } else if raid {
            Some(MeetingStoneFailure::MeetingstoneFailRaidGroup)
        } else if size >= PARTY {
            Some(MeetingStoneFailure::MeetingstoneFailFullGroup)
        } else {
            None
        };
        if let Some(reason) = failure {
            send_to(&map.realm, g, &SMSG_MEETINGSTONE_JOINFAILED { reason });
            return;
        }
    }
    enqueue(&map.realm, g, info.area, team, class);
}

/// Into `area`'s queue (mangos's `LFGMgr::AddToQueue`): a party's leader
/// for their party, one alone for themselves, a member not at all.
fn enqueue(realm: &Realm, g: Guid, area: u32, team: Team, class: u8) {
    let party = {
        let groups = realm.social.groups.lock();
        groups.group_of(g).map(|gr| {
            (
                gr.id,
                gr.leader,
                gr.members.iter().map(|m| m.guid).collect::<Vec<_>>(),
            )
        })
    };
    let (group, members) = match party {
        Some((_, leader, _)) if leader != g => return,
        Some((id, _, members)) => (id, members),
        None => (0, vec![g]),
    };
    {
        let mut stones = realm.social.meeting_stones.lock();
        stones
            .queue
            .retain(|q| !members.contains(&q.guid) && (group == 0 || q.group != group));
        stones.queue.push(Queued {
            guid: g,
            area,
            team,
            class,
            leader: group != 0,
            group,
            since: Instant::now(),
            progress_at: Instant::now() + PROGRESS_EVERY,
        });
    }
    for m in members {
        status(realm, m, area, MeetingStoneStatus::JoinedQueue);
    }
}

/// A script queues `g` for `area`'s meeting stone (`SCRIPT_COMMAND_MEETINGSTONE`).
pub fn queue(map: &mut Map, g: Guid, area: u32) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let (team, class) = (p.team, p.row.class);
    enqueue(&map.realm, g, area, team, class);
}

/// `CMSG_MEETINGSTONE_LEAVE` (mangos's `HandleMeetingStoneLeaveOpcode`):
/// a party's leader withdraws it, each member hearing so; a member is only
/// told they aren't queued; one alone leaves.
pub fn leave(realm: &Realm, g: Guid) {
    let party = {
        let groups = realm.social.groups.lock();
        groups.group_of(g).map(|gr| {
            (
                gr.id,
                gr.leader,
                gr.members.iter().map(|m| m.guid).collect::<Vec<_>>(),
            )
        })
    };
    match party {
        Some((id, leader, members)) => {
            let taken = leader == g && realm.social.meeting_stones.lock().take_party(id).is_some();
            if taken {
                for m in members {
                    status(realm, m, 0, MeetingStoneStatus::LeaveQueue);
                }
            } else {
                status(realm, g, 0, MeetingStoneStatus::None);
            }
        }
        None => {
            let was = realm.social.meeting_stones.lock().take_solo(g).is_some();
            if was {
                status(realm, g, 0, MeetingStoneStatus::LeaveQueue);
            }
        }
    }
}

/// Gone offline (`social::logged_out`): one alone leaves the queue
/// unannounced, as mangos's `LogoutPlayer` takes them; a party stays
/// queued, as mangos's does.
pub fn logged_out(realm: &Realm, g: Guid) {
    realm.social.meeting_stones.lock().take_solo(g);
}

/// `g` joined group `id` by an invitation (`groups::accept`), `members`
/// all of it now. One queued alone who joins a party leaves their queue;
/// a queued party made whole by it is complete, as mangos's
/// `LFGQueue::UpdateGroup` has it; one joining a party still searching is
/// told so.
pub fn joined_group(realm: &Realm, id: u32, g: Guid, members: &[Guid]) {
    let (alone, party) = {
        let mut stones = realm.social.meeting_stones.lock();
        let alone: Vec<Guid> = members
            .iter()
            .copied()
            .filter(|m| stones.take_solo(*m).is_some())
            .collect();
        let party = stones.party(id).map(|q| q.area);
        if party.is_some() && members.len() >= PARTY {
            stones.take_party(id);
        }
        (alone, party)
    };
    match party {
        Some(_) if members.len() >= PARTY => complete(realm, members),
        Some(area) => {
            status(realm, g, area, MeetingStoneStatus::JoinedQueue);
            for a in alone.into_iter().filter(|a| *a != g) {
                status(realm, a, 0, MeetingStoneStatus::None);
            }
        }
        None => {
            for a in alone {
                status(realm, a, 0, MeetingStoneStatus::None);
            }
        }
    }
}

/// `g` out of group `id` (`groups::remove`), kicked or gone, `rest` those
/// who stay, `leader_left` if it was its leader; as mangos's
/// `Group::RemoveMember`. A kick withdraws the party (its members told so)
/// and queues the one kicked alone for the same dungeon; one leaving is
/// out of the queue, and the party searches on without them unless they
/// led it, when it is withdrawn.
pub fn left_group(realm: &Realm, id: u32, g: Guid, kicked: bool, leader_left: bool, rest: &[Guid]) {
    let party = {
        let mut stones = realm.social.meeting_stones.lock();
        if kicked || leader_left {
            stones.take_party(id)
        } else {
            stones.party(id).cloned()
        }
    };
    let Some(party) = party else {
        return;
    };
    if kicked {
        for m in rest {
            status(
                realm,
                *m,
                0,
                MeetingStoneStatus::PartyMemberRemovedPartyRemoved,
            );
            status(realm, *m, 0, MeetingStoneStatus::LeaveQueue);
        }
        if let Some(o) = realm.registry.online(g) {
            status(
                realm,
                g,
                party.area,
                MeetingStoneStatus::LookingForNewPartyInQueue,
            );
            enqueue(realm, g, party.area, party.team, o.class);
        }
        return;
    }
    status(realm, g, 0, MeetingStoneStatus::None);
    for m in rest {
        if leader_left {
            status(realm, *m, 0, MeetingStoneStatus::LeaveQueue);
        } else {
            status(
                realm,
                *m,
                party.area,
                MeetingStoneStatus::PartyMemberLeftLfg,
            );
        }
    }
}

/// Group `id` disbanded (mangos's `Group::Disband`): out of the queue,
/// each of `members` told they aren't queued.
pub fn group_disbanded(realm: &Realm, id: u32, members: &[Guid]) {
    if realm.social.meeting_stones.lock().take_party(id).is_none() {
        return;
    }
    for m in members {
        status(realm, *m, 0, MeetingStoneStatus::None);
    }
}

/// Group `id` led by `to` now: the party stays queued, under its new
/// leader.
pub fn leader_changed(realm: &Realm, id: u32, to: Guid) {
    let class = realm.registry.name(to).map(|n| n.class);
    let mut stones = realm.social.meeting_stones.lock();
    if let Some(q) = stones.queue.iter_mut().find(|q| q.leader && q.group == id) {
        q.guid = to;
        if let Some(class) = class {
            q.class = class;
        }
    }
}

/// Group `id` made a raid: a raid can't queue at a stone, so the party is
/// withdrawn, each of `members` told so.
pub fn raid_converted(realm: &Realm, id: u32, members: &[Guid]) {
    if realm.social.meeting_stones.lock().take_party(id).is_none() {
        return;
    }
    for m in members {
        status(realm, *m, 0, MeetingStoneStatus::LeaveQueue);
    }
}

/// A member's class, online or not.
fn class_of(realm: &Realm, g: Guid) -> u8 {
    realm.registry.name(g).map_or(0, |n| n.class)
}

/// Match each dungeon's queue: parties take who they lack, and players
/// alone make parties of their own.
pub fn sweep(realm: &Realm) {
    let now = Instant::now();
    // Those alone offline leave: one lost with their map had no logout.
    // Kept, the longest waiting of them would lead, and every player alone
    // be handed to them and grouped with no one. One alone now in a group
    // leaves too; a party whose group is gone, or a raid, is withdrawn.
    let queue: Vec<Queued> = realm.social.meeting_stones.lock().queue.clone();
    let (gone, grouped) = {
        let groups = realm.social.groups.lock();
        let mut gone = Vec::new();
        let mut grouped = Vec::new();
        for q in &queue {
            if q.leader {
                if groups.get(q.group).is_none_or(|gr| gr.raid) {
                    gone.push(q.guid);
                }
            } else if realm.registry.online(q.guid).is_none() {
                gone.push(q.guid);
            } else if groups.group_of(q.guid).is_some() {
                grouped.push(q.guid);
            }
        }
        (gone, grouped)
    };
    if !gone.is_empty() || !grouped.is_empty() {
        realm
            .social
            .meeting_stones
            .lock()
            .queue
            .retain(|q| !gone.contains(&q.guid) && !grouped.contains(&q.guid));
        for g in grouped {
            status(realm, g, 0, MeetingStoneStatus::None);
        }
    }
    // A party still searching hears so, every five minutes.
    let searching: Vec<u32> = {
        let mut stones = realm.social.meeting_stones.lock();
        stones
            .queue
            .iter_mut()
            .filter(|q| q.leader && q.progress_at <= now)
            .map(|q| {
                q.progress_at = now + PROGRESS_EVERY;
                q.group
            })
            .collect()
    };
    for id in searching {
        let members: Vec<Guid> = {
            let groups = realm.social.groups.lock();
            groups
                .get(id)
                .map(|g| g.members.iter().map(|m| m.guid).collect())
                .unwrap_or_default()
        };
        for m in members {
            send_to(realm, m, &SMSG_MEETINGSTONE_IN_PROGRESS {});
        }
    }
    let mut queue: Vec<Queued> = realm.social.meeting_stones.lock().queue.clone();
    queue.sort_by_key(|q| q.since);
    let mut areas: Vec<(u32, Team)> = queue.iter().map(|q| (q.area, q.team)).collect();
    areas.sort_by_key(|(a, t)| (*a, *t as u8));
    areas.dedup();
    for (area, team) in areas {
        let here = |q: &&Queued| q.area == area && q.team == team;
        let mut solos: Vec<Queued> = queue
            .iter()
            .filter(here)
            .filter(|q| !q.leader)
            .cloned()
            .collect();
        let mut parties: Vec<(Guid, u32)> = queue
            .iter()
            .filter(here)
            .filter(|q| q.leader)
            .map(|q| (q.guid, q.group))
            .collect();
        // Players alone with no party to join, five of them for the
        // dungeon (mangos-classic's `LFGQueue::Update`, `m_groupSize`:
        // the leader and four others in the area): the longest waiting
        // leads.
        if parties.is_empty() && solos.len() >= PARTY {
            parties.push((solos.remove(0).guid, 0));
        }
        for (mut leader, mut id) in parties {
            loop {
                let members: Vec<Guid> = if id == 0 {
                    vec![leader]
                } else {
                    let groups = realm.social.groups.lock();
                    let Some(gr) = groups.get(id) else {
                        break;
                    };
                    leader = gr.leader;
                    gr.members.iter().map(|m| m.guid).collect()
                };
                if members.len() >= PARTY {
                    realm.social.meeting_stones.lock().take_party(id);
                    complete(realm, &members);
                    break;
                }
                if solos.is_empty() {
                    break;
                }
                let classes: Vec<u8> = members.iter().map(|m| class_of(realm, *m)).collect();
                let Some(pick) = best_for(open_roles(&classes), &solos, now) else {
                    break;
                };
                solos.retain(|q| q.guid != pick);
                let (Some(lo), Some(po)) =
                    (realm.registry.online(leader), realm.registry.online(pick))
                else {
                    continue;
                };
                if !groups::join_directly(realm, &lo, &po) {
                    continue;
                }
                if id == 0 {
                    let Some(made) = realm.social.groups.lock().group_of(leader).map(|g| g.id)
                    else {
                        break;
                    };
                    id = made;
                }
                {
                    let mut stones = realm.social.meeting_stones.lock();
                    stones.queue.retain(|q| q.guid != pick);
                    // The leader now queues for a party.
                    if let Some(q) = stones.queue.iter_mut().find(|q| q.guid == leader) {
                        q.leader = true;
                        q.group = id;
                    }
                }
                for m in members.iter().chain([&pick]) {
                    send_to(realm, *m, &SMSG_MEETINGSTONE_MEMBER_ADDED { guid: pick });
                }
                status(realm, pick, area, MeetingStoneStatus::JoinedQueue);
            }
        }
    }
}

/// A party made whole: told so, and that it is out of the queue (mangos's
/// `RemoveGroupFromQueue` by the system).
fn complete(realm: &Realm, members: &[Guid]) {
    realm
        .social
        .meeting_stones
        .lock()
        .queue
        .retain(|q| !members.contains(&q.guid));
    for m in members {
        send_to(realm, *m, &SMSG_MEETINGSTONE_COMPLETE {});
        status(realm, *m, 0, MeetingStoneStatus::None);
    }
}

/// How long a sweep waits between matches.
pub const MATCH_EVERY: Duration = Duration::from_secs(5);
