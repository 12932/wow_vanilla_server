//! Meeting stones, as Thistle Tea's `Core.MeetingStone` runs them: a player
//! or a party's leader queues at a dungeon's stone; every few seconds each
//! dungeon's queue is matched by class, a party taking the players it
//! lacks (a tank, a healer, then damage) and players alone forming one.
//! The queue spans the realm.

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
const PARTY: usize = 5;

/// A party's needs: tanks, healers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Tank,
    Healer,
    Damage,
}

/// The role a class plays in a party, as Thistle Tea's `Roles` sees it.
pub fn role(class: u8) -> Role {
    match class {
        1 => Role::Tank,
        5 | 7 | 11 => Role::Healer,
        2 => Role::Healer,
        _ => Role::Damage,
    }
}

#[derive(Debug, Clone)]
pub struct Queued {
    pub guid: Guid,
    pub area: u32,
    pub team: Team,
    pub class: u8,
    /// A party's leader, queued for their party.
    pub leader: bool,
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

pub fn handle(map: &mut Map, g: Guid, msg: ClientOpcodeMessage) -> Option<ClientOpcodeMessage> {
    use ClientOpcodeMessage as C;
    match msg {
        C::CMSG_MEETINGSTONE_JOIN(m) => join(map, g, m.guid),
        C::CMSG_MEETINGSTONE_LEAVE => leave(&map.realm, g),
        C::CMSG_MEETINGSTONE_INFO => {
            let queued = map.realm.social.meeting_stones.lock().of(g).map(|q| q.area);
            match queued {
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
    let level = u32::from(p.level());
    if level < info.min_level || (info.max_level > 0 && level > info.max_level) {
        map.system_message(g, "You are not the right level for this dungeon.");
        return;
    }
    let (team, class) = (p.team, p.row.class);
    let party = {
        let groups = map.realm.social.groups.lock();
        groups.group_of(g).map(|gr| (gr.leader, gr.raid, gr.members.len()))
    };
    let leader = match party {
        Some((leader, raid, size)) => {
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
            true
        }
        None => false,
    };
    {
        let mut stones = map.realm.social.meeting_stones.lock();
        stones.queue.retain(|q| q.guid != g);
        stones.queue.push(Queued {
            guid: g,
            area: info.area,
            team,
            class,
            leader,
            since: Instant::now(),
            progress_at: Instant::now() + PROGRESS_EVERY,
        });
    }
    status(&map.realm, g, info.area, MeetingStoneStatus::JoinedQueue);
    if leader {
        let others = map.realm.social.groups.lock().others(g);
        for m in others {
            status(&map.realm, m, info.area, MeetingStoneStatus::JoinedQueue);
        }
    }
}

/// A script queues `g` for `area`'s meeting stone (`SCRIPT_COMMAND_MEETINGSTONE`).
pub fn queue(map: &mut Map, g: Guid, area: u32) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let (team, class) = (p.team, p.row.class);
    {
        let mut stones = map.realm.social.meeting_stones.lock();
        stones.queue.retain(|q| q.guid != g);
        stones.queue.push(Queued {
            guid: g,
            area,
            team,
            class,
            leader: false,
            since: Instant::now(),
            progress_at: Instant::now() + PROGRESS_EVERY,
        });
    }
    status(&map.realm, g, area, MeetingStoneStatus::JoinedQueue);
}

/// `CMSG_MEETINGSTONE_LEAVE`, or gone offline.
pub fn leave(realm: &Realm, g: Guid) {
    let was = {
        let mut stones = realm.social.meeting_stones.lock();
        let was = stones.of(g).is_some();
        stones.queue.retain(|q| q.guid != g);
        was
    };
    if was {
        status(realm, g, 0, MeetingStoneStatus::LeaveQueue);
    }
}

/// The roles a party has.
fn roles_of(realm: &Realm, members: &[Guid]) -> Vec<Role> {
    members
        .iter()
        .filter_map(|m| realm.registry.online(*m).map(|o| role(o.class)))
        .collect()
}

/// The queued player alone who best fills what `have` lacks.
fn best_for(have: &[Role], solos: &[&Queued]) -> Option<Guid> {
    let wants = |r: Role| match r {
        Role::Tank => !have.contains(&Role::Tank),
        Role::Healer => !have.contains(&Role::Healer),
        Role::Damage => have.iter().filter(|x| **x == Role::Damage).count() < 3,
    };
    solos
        .iter()
        .find(|q| wants(role(q.class)))
        .or(solos.first())
        .map(|q| q.guid)
}

/// Match each dungeon's queue: parties take who they lack, and players
/// alone make parties of their own.
pub fn sweep(realm: &Realm) {
    // A party still searching hears so, every five minutes.
    let now = Instant::now();
    let searching: Vec<Guid> = {
        let mut stones = realm.social.meeting_stones.lock();
        stones
            .queue
            .iter_mut()
            .filter(|q| q.leader && q.progress_at <= now)
            .map(|q| {
                q.progress_at = now + PROGRESS_EVERY;
                q.guid
            })
            .collect()
    };
    for leader in searching {
        let members: Vec<Guid> = {
            let groups = realm.social.groups.lock();
            groups
                .group_of(leader)
                .map(|g| g.members.iter().map(|m| m.guid).collect())
                .unwrap_or_else(|| vec![leader])
        };
        for m in members {
            send_to(realm, m, &SMSG_MEETINGSTONE_IN_PROGRESS {});
        }
    }
    let queue: Vec<Queued> = realm.social.meeting_stones.lock().queue.clone();
    let mut areas: Vec<(u32, Team)> = queue.iter().map(|q| (q.area, q.team)).collect();
    areas.sort_by_key(|(a, t)| (*a, *t as u8));
    areas.dedup();
    for (area, team) in areas {
        let here: Vec<&Queued> = queue
            .iter()
            .filter(|q| q.area == area && q.team == team)
            .collect();
        let mut solos: Vec<&Queued> = here.iter().copied().filter(|q| !q.leader).collect();
        solos.sort_by_key(|q| q.since);
        let mut leaders: Vec<Guid> = here.iter().filter(|q| q.leader).map(|q| q.guid).collect();
        // Players alone with no party to join: the longest waiting leads.
        if leaders.is_empty() && solos.len() >= 2 {
            leaders.push(solos.remove(0).guid);
        }
        for leader in leaders {
            loop {
                let members: Vec<Guid> = {
                    let groups = realm.social.groups.lock();
                    match groups.group_of(leader) {
                        Some(gr) => gr.members.iter().map(|m| m.guid).collect(),
                        None => vec![leader],
                    }
                };
                if members.len() >= PARTY || solos.is_empty() {
                    if members.len() >= PARTY {
                        complete(realm, &members);
                    }
                    break;
                }
                let have = roles_of(realm, &members);
                let Some(pick) = best_for(&have, &solos) else {
                    break;
                };
                solos.retain(|q| q.guid != pick);
                let (Some(lo), Some(po)) = (realm.registry.online(leader), realm.registry.online(pick))
                else {
                    continue;
                };
                if groups::join_directly(realm, &lo, &po) {
                    realm.social.meeting_stones.lock().queue.retain(|q| q.guid != pick);
                    let mut stones = realm.social.meeting_stones.lock();
                    // The leader now queues for a party.
                    if let Some(q) = stones.queue.iter_mut().find(|q| q.guid == leader) {
                        q.leader = true;
                    }
                    drop(stones);
                    for m in members.iter().chain([&pick]) {
                        send_to(realm, *m, &SMSG_MEETINGSTONE_MEMBER_ADDED { guid: pick });
                    }
                    status(realm, pick, area, MeetingStoneStatus::JoinedQueue);
                }
            }
        }
    }
}

/// A party made whole: out of the queue.
fn complete(realm: &Realm, members: &[Guid]) {
    realm
        .social
        .meeting_stones
        .lock()
        .queue
        .retain(|q| !members.contains(&q.guid));
    for m in members {
        send_to(realm, *m, &SMSG_MEETINGSTONE_COMPLETE {});
    }
}

/// How long a sweep waits between matches.
pub const MATCH_EVERY: Duration = Duration::from_secs(5);
