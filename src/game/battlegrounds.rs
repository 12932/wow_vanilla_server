//! Battleground queues and matches, across maps: a queue per battleground
//! and level bracket, with a side for each team; a match (a copy of the
//! battleground's map) when both sides have enough, and later ones topped
//! up from the queue. The match's rules run on its map
//! ([`crate::game::map::battleground`]).
//!
//! Joining is Thistle Tea's: a player or their group's leader for the
//! whole group, every one of the same side and bracket, within the
//! battleground's levels, no deserter and no one already in a match; a
//! group goes in together, to the match picked from the list or the first
//! with room. A player may be in three queues at once, each in a slot of
//! their own that every status names (mangos's
//! `PLAYER_MAX_BATTLEGROUND_QUEUES`); the slot is held until they leave
//! the queue, the invitation or the match. A player who goes offline in a
//! match keeps their place five minutes (mangos's `MAX_OFFLINE_TIME`), and
//! logging in again takes them back in; not back by then, they are out,
//! and a deserter at their next login (`AT_LOGIN_ADD_BG_DESERTER`).

use std::time::{Duration, Instant};

use ahash::{AHashMap, AHashSet};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::Realm;
use crate::game::data::items::Body;
use crate::game::map::{Map, MapKey};
use crate::game::registry::{Online, Team};
use crate::game::social::send_frame;

pub const SMSG_BATTLEFIELD_STATUS: u16 = 0x2D4;
/// How long an invitation stands, by default (`[battleground]
/// invitation_secs`).
pub const INVITE_FOR: Duration = Duration::from_secs(120);
/// How long a player gone offline in a match keeps their place (mangos's
/// `MAX_OFFLINE_TIME`).
pub const OFFLINE_FOR: Duration = Duration::from_secs(300);
/// The queues a player may be in at once (mangos's
/// `PLAYER_MAX_BATTLEGROUND_QUEUES`).
pub const QUEUE_SLOTS: usize = 3;
/// The store's record of a character who stayed away from their match too
/// long: Deserter at their next login.
pub const DESERTER_KIND: &str = "bg_deserter";

// Statuses.
const WAIT_QUEUE: u32 = 1;
const WAIT_JOIN: u32 = 2;
const IN_PROGRESS: u32 = 3;

/// The map each battleground is played on.
pub fn map_of(bg: u32) -> u32 {
    match bg {
        1 => 30,
        3 => 529,
        _ => 489,
    }
}

pub fn bg_of_map(map: u32) -> Option<u32> {
    match map {
        30 => Some(1),
        489 => Some(2),
        529 => Some(3),
        _ => None,
    }
}

/// The level bracket: 10-19 is 0, and so on to 60.
pub fn bracket_of(level: u8) -> u8 {
    (level / 10).saturating_sub(1).min(5)
}

/// A battleground's bracket for `level`: Alterac Valley has the one,
/// 51 to 60 (Thistle Tea's `Battleground.bracket/2`).
pub fn bracket_for(bg: u32, level: u8) -> u8 {
    if bg == 1 { 0 } else { bracket_of(level) }
}

/// How long after the end everyone is sent home, by default
/// (`[battleground] auto_leave_secs`).
pub const AUTO_LEAVE: Duration = Duration::from_secs(120);

fn side(team: Team) -> usize {
    match team {
        Team::Alliance => 0,
        Team::Horde => 1,
    }
}

#[derive(Debug, Clone)]
pub struct Queued {
    pub guid: Guid,
    pub team: Team,
    pub since: Instant,
    /// The group queued together (0 for one alone): it goes in whole.
    pub party: u32,
}

/// One who joins: their side, level, and what would keep them out.
#[derive(Debug, Clone, Copy)]
pub struct Entrant {
    pub guid: Guid,
    pub team: Team,
    pub level: u8,
    pub deserter: bool,
    /// Already in a battleground.
    pub inside: bool,
}

/// Why a join was refused, as Thistle Tea's `BattlegroundSystem` has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    Empty,
    Duplicate,
    /// Queued for this battleground already, invited to it or in it.
    AlreadyQueued,
    /// In three queues already.
    NoFreeSlot,
    Deserter,
    AlreadyInside,
    LevelRestricted,
    InvalidTeam,
    MixedBracket,
    GroupTooLarge,
    InvalidInstance,
    NotLeader,
    MemberOffline,
}

#[derive(Debug, Clone)]
pub struct Match {
    pub id: u32,
    pub bg: u32,
    pub bracket: u8,
    pub invited: AHashMap<Guid, (Team, Instant)>,
    pub members: AHashMap<Guid, Team>,
    /// Members gone offline, and until when their place is kept.
    pub offline: AHashMap<Guid, Instant>,
    pub started: Instant,
    pub ended: bool,
    /// When the battle began, and ended.
    pub began: Option<Instant>,
    pub ended_at: Option<Instant>,
}

impl Match {
    /// Its `SMSG_BATTLEFIELD_STATUS` for one inside, in their queue
    /// `slot`: the time left before all are sent home (`leave_after` the
    /// end), and since the battle began.
    pub fn in_progress_frame(
        &self,
        slot: u32,
        now: Instant,
        leave_after: Duration,
    ) -> std::sync::Arc<[u8]> {
        let leave = self.ended_at.map_or(0, |t| {
            (t + leave_after).saturating_duration_since(now).as_millis() as u32
        });
        let since = self
            .began
            .map_or(0, |t| now.saturating_duration_since(t).as_millis() as u32);
        status_frame_at(
            slot,
            self.bracket,
            map_of(self.bg),
            self.id,
            IN_PROGRESS,
            leave,
            since,
        )
    }

    fn count(&self, team: Team) -> usize {
        self.invited.values().filter(|(t, _)| *t == team).count()
            + self.members.values().filter(|t| **t == team).count()
    }

    /// The members of a side, those offline with their place kept among
    /// them (mangos's `GetPlayersCountByTeam`).
    pub fn members_of(&self, team: Team) -> usize {
        self.members.values().filter(|t| **t == team).count()
    }

    pub fn key(&self) -> MapKey {
        MapKey {
            map: map_of(self.bg),
            instance: self.id,
        }
    }
}

#[derive(Debug, Default)]
pub struct Battlegrounds {
    queues: AHashMap<(u32, u8), Vec<Queued>>,
    pub matches: AHashMap<u32, Match>,
    next_party: u32,
    /// Each player's queue slots: the battleground each holds, 0 for free.
    slots: AHashMap<Guid, [u32; QUEUE_SLOTS]>,
    /// Characters (by counter) who stayed away from their match too long:
    /// Deserter at their next login.
    pub deserters: AHashSet<u32>,
}

impl Battlegrounds {
    /// The deserters the store kept.
    pub fn load(records: &[(String, u64, String)]) -> Self {
        Self {
            deserters: records
                .iter()
                .filter(|r| r.0 == DESERTER_KIND)
                .map(|r| r.1 as u32)
                .collect(),
            ..Default::default()
        }
    }

    pub fn queued(&self, g: Guid) -> bool {
        self.queues.values().flatten().any(|q| q.guid == g)
    }

    /// Whether `g` is queued for `bg`.
    pub fn queued_for(&self, g: Guid, bg: u32) -> bool {
        self.queues
            .iter()
            .any(|((b, _), q)| *b == bg && q.iter().any(|x| x.guid == g))
    }

    pub fn match_of(&self, g: Guid) -> Option<&Match> {
        self.matches
            .values()
            .find(|m| m.members.contains_key(&g) || m.invited.contains_key(&g))
    }

    /// The match of `bg` that `g` is invited to or in.
    pub fn match_of_bg(&self, g: Guid, bg: u32) -> Option<&Match> {
        self.matches
            .values()
            .find(|m| m.bg == bg && (m.members.contains_key(&g) || m.invited.contains_key(&g)))
    }

    /// The queue slot `g` holds for `bg`.
    pub fn slot_of(&self, g: Guid, bg: u32) -> Option<u32> {
        self.slots
            .get(&g)
            .and_then(|s| s.iter().position(|b| *b == bg))
            .map(|i| i as u32)
    }

    /// `g`'s slot for `bg`: the one held, or the first free.
    fn take_slot(&mut self, g: Guid, bg: u32) -> Option<u32> {
        if let Some(s) = self.slot_of(g, bg) {
            return Some(s);
        }
        let slots = self.slots.entry(g).or_default();
        let free = slots.iter().position(|b| *b == 0)?;
        slots[free] = bg;
        Some(free as u32)
    }

    /// `g` lets their slot for `bg` go: the one it was, to tell them.
    fn free_slot(&mut self, g: Guid, bg: u32) -> Option<u32> {
        let s = self.slot_of(g, bg)?;
        if let Some(slots) = self.slots.get_mut(&g) {
            slots[s as usize] = 0;
            if slots.iter().all(|b| *b == 0) {
                self.slots.remove(&g);
            }
        }
        Some(s)
    }

    /// Whether `g` has a slot free for `bg`, or holds one for it.
    fn has_slot(&self, g: Guid, bg: u32) -> bool {
        self.slots
            .get(&g)
            .is_none_or(|s| s.iter().any(|b| *b == 0 || *b == bg))
    }

    /// The slots `g` holds, and for which battlegrounds.
    pub fn slots(&self, g: Guid) -> Vec<(u32, u32)> {
        self.slots
            .get(&g)
            .map(|s| {
                s.iter()
                    .enumerate()
                    .filter(|(_, b)| **b != 0)
                    .map(|(i, b)| (i as u32, *b))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// `SMSG_BATTLEFIELD_STATUS` in 1.12's layout, in queue slot 0: none (map
/// 0), or the map, the match and how long.
pub fn status_frame(
    map: u32,
    instance: u32,
    status: u32,
    t1: u32,
    t2: u32,
) -> std::sync::Arc<[u8]> {
    status_frame_at(0, 0, map, instance, status, t1, t2)
}

/// [`status_frame`], with the match's bracket.
pub fn status_frame_in(
    bracket: u8,
    map: u32,
    instance: u32,
    status: u32,
    t1: u32,
    t2: u32,
) -> std::sync::Arc<[u8]> {
    status_frame_at(0, bracket, map, instance, status, t1, t2)
}

/// [`status_frame_in`], in queue slot `slot` (mangos's
/// `BuildBattleGroundStatusPacket`'s `queueSlot`).
pub fn status_frame_at(
    slot: u32,
    bracket: u8,
    map: u32,
    instance: u32,
    status: u32,
    t1: u32,
    t2: u32,
) -> std::sync::Arc<[u8]> {
    let mut b = Body::default();
    b.u32(slot);
    if status == 0 {
        b.u32(0);
    } else {
        b.u32(map).u8(bracket).u32(instance).u32(status);
        match status {
            WAIT_QUEUE | IN_PROGRESS => {
                b.u32(t1).u32(t2);
            }
            WAIT_JOIN => {
                b.u32(t1);
            }
            _ => {}
        }
    }
    b.frame(SMSG_BATTLEFIELD_STATUS)
}

/// No status, in queue slot `slot`.
pub fn none_frame(slot: u32) -> std::sync::Arc<[u8]> {
    status_frame_at(slot, 0, 0, 0, 0, 0, 0)
}

fn template(realm: &Realm, bg: u32) -> (u32, u32, u32, u32) {
    let t = realm.data.battlegrounds.get(&bg);
    let min = min_players(realm, bg);
    let max = t.map_or(10, |t| t.max_players.max(1));
    let (lo, hi) = t.map_or((10, 60), |t| (t.min_level, t.max_level));
    (min, max, lo, hi)
}

/// The players a side of `bg` needs to begin, and to go on (mangos's
/// `GetMinPlayersPerTeam`).
pub fn min_players(realm: &Realm, bg: u32) -> u32 {
    let t = realm.data.battlegrounds.get(&bg);
    realm
        .config()
        .bg_min_players
        .unwrap_or_else(|| t.map_or(10, |t| t.min_players.max(1)))
}

/// `CMSG_BATTLEFIELD_STATUS`: where a player stands in each of their queue
/// slots: queued, invited or in a match (mangos's
/// `HandleBattlefieldStatusOpcode`); none, in none.
pub fn send_status(realm: &Realm, me: &Online) {
    let now = Instant::now();
    let frames = {
        let bgs = realm.social.battlegrounds.lock();
        let mut frames = Vec::new();
        for (slot, bg) in bgs.slots(me.guid) {
            let queued = bgs.queues.iter().find_map(|((b, bracket), q)| {
                (*b == bg)
                    .then(|| q.iter().find(|x| x.guid == me.guid))
                    .flatten()
                    .map(|x| (*bracket, x.since))
            });
            let frame = if let Some(m) = bgs.match_of_bg(me.guid, bg) {
                match m.invited.get(&me.guid) {
                    Some((_, until)) => status_frame_at(
                        slot,
                        m.bracket,
                        map_of(m.bg),
                        m.id,
                        WAIT_JOIN,
                        until.saturating_duration_since(now).as_millis() as u32,
                        0,
                    ),
                    None => m.in_progress_frame(slot, now, realm.config().bg_auto_leave),
                }
            } else if let Some((bracket, since)) = queued {
                status_frame_at(
                    slot,
                    bracket,
                    map_of(bg),
                    0,
                    WAIT_QUEUE,
                    0,
                    now.saturating_duration_since(since).as_millis() as u32,
                )
            } else {
                none_frame(slot)
            };
            frames.push(frame);
        }
        if frames.is_empty() {
            frames.push(none_frame(0));
        }
        frames
    };
    for f in frames {
        me.outbound.send_frame(f);
    }
}

/// The matches of `bg` in `bracket` still on with room for either side,
/// for the list.
pub fn instances(realm: &Realm, bg: u32, bracket: u8) -> Vec<u32> {
    let (_, max, _, _) = template(realm, bg);
    let bgs = realm.social.battlegrounds.lock();
    let mut out: Vec<u32> = bgs
        .matches
        .values()
        .filter(|m| m.bg == bg && m.bracket == bracket && !m.ended)
        .filter(|m| m.count(Team::Alliance) < max as usize || m.count(Team::Horde) < max as usize)
        .map(|m| m.id)
        .collect();
    out.sort_unstable();
    out
}

/// `SMSG_GROUP_JOINED_BATTLEGROUND`: queued for `bg`, or (`None`) not, a
/// deserter.
pub fn joined_frame(bg: Option<u32>) -> Option<std::sync::Arc<[u8]>> {
    use wow_world_messages::vanilla::{BgTypeId, SMSG_GROUP_JOINED_BATTLEGROUND};
    let id = match bg {
        Some(1) => BgTypeId::QueuedForAv,
        Some(3) => BgTypeId::QueuedForAb,
        Some(_) => BgTypeId::QueuedForWsg,
        None => BgTypeId::RemoveFromQueue,
    };
    crate::game::conn::encode(&SMSG_GROUP_JOINED_BATTLEGROUND { id })
}

/// Into `bg`'s queue, or straight into the match `instance` (0 for the
/// first with room), together: those who join hear they're queued, and
/// where they stand. One already queued for `bg` (or invited to it, or in
/// it), or with their three slots taken, is refused, as mangos's
/// `HandleBattlemasterJoinOpcode` ignores them.
pub fn join_all(realm: &Realm, who: &[Entrant], bg: u32, instance: u32) -> Result<(), Refused> {
    let (_, max, lo, hi) = template(realm, bg);
    if who.is_empty() {
        return Err(Refused::Empty);
    }
    let mut guids: Vec<Guid> = who.iter().map(|e| e.guid).collect();
    guids.sort_by_key(|g| g.guid());
    guids.dedup();
    if guids.len() != who.len() {
        return Err(Refused::Duplicate);
    }
    let bracket = bracket_for(bg, who[0].level);
    let team = who[0].team;
    // Asked again under the lock that queues them: a player's own join and
    // their leader's for the group, on two maps at once, both passed this
    // before either queued, and had two slots for one battleground.
    let taken = |bgs: &Battlegrounds| {
        if who
            .iter()
            .any(|e| bgs.queued_for(e.guid, bg) || bgs.match_of_bg(e.guid, bg).is_some())
        {
            return Err(Refused::AlreadyQueued);
        }
        if who.iter().any(|e| !bgs.has_slot(e.guid, bg)) {
            return Err(Refused::NoFreeSlot);
        }
        Ok(())
    };
    taken(&realm.social.battlegrounds.lock())?;
    if who.iter().any(|e| e.deserter) {
        return Err(Refused::Deserter);
    }
    if who.iter().any(|e| e.inside) {
        return Err(Refused::AlreadyInside);
    }
    if who
        .iter()
        .any(|e| u32::from(e.level) < lo || u32::from(e.level) > hi)
    {
        return Err(Refused::LevelRestricted);
    }
    if who.iter().any(|e| e.team != team) {
        return Err(Refused::InvalidTeam);
    }
    if who.iter().any(|e| bracket_for(bg, e.level) != bracket) {
        return Err(Refused::MixedBracket);
    }
    if who.len() > max as usize {
        return Err(Refused::GroupTooLarge);
    }
    let now = Instant::now();
    let direct = {
        let mut bgs = realm.social.battlegrounds.lock();
        taken(&bgs)?;
        if instance > 0 {
            // The match picked from the list, if it has room for them all.
            let Some(m) = bgs.matches.get_mut(&instance).filter(|m| {
                m.bg == bg
                    && m.bracket == bracket
                    && !m.ended
                    && m.count(team) + who.len() <= max as usize
            }) else {
                return Err(Refused::InvalidInstance);
            };
            for e in who {
                m.invited
                    .insert(e.guid, (team, now + realm.config().bg_invite_for));
            }
            let id = m.id;
            let slots: Vec<(Guid, u32)> = who
                .iter()
                .map(|e| (e.guid, bgs.take_slot(e.guid, bg).unwrap_or(0)))
                .collect();
            Some((id, slots))
        } else {
            bgs.next_party += 1;
            let party = if who.len() > 1 { bgs.next_party } else { 0 };
            for e in who {
                bgs.take_slot(e.guid, bg);
            }
            let queue = bgs.queues.entry((bg, bracket)).or_default();
            for e in who {
                queue.push(Queued {
                    guid: e.guid,
                    team,
                    since: now,
                    party,
                });
            }
            None
        }
    };
    if let Some((id, slots)) = direct {
        for (g, slot) in slots {
            send_frame(
                realm,
                g,
                &status_frame_at(
                    slot,
                    bracket,
                    map_of(bg),
                    id,
                    WAIT_JOIN,
                    realm.config().bg_invite_for.as_millis() as u32,
                    0,
                ),
            );
        }
    } else {
        fill(realm, bg, bracket);
    }
    for e in who {
        if let Some(o) = realm.registry.online(e.guid) {
            if let Some(frame) = joined_frame(Some(bg)) {
                o.outbound.send_frame(frame);
            }
            send_status(realm, &o);
        }
    }
    Ok(())
}

/// Up to `room` of `team` from the queue, first come first: a group whole,
/// or not at all.
fn take(queue: &mut Vec<Queued>, team: Team, room: usize) -> Vec<Queued> {
    let mut out = Vec::new();
    let mut skipped: Vec<u32> = Vec::new();
    let mut i = 0;
    while i < queue.len() && out.len() < room {
        let q = &queue[i];
        if q.team != team || skipped.contains(&q.party) {
            i += 1;
            continue;
        }
        if q.party == 0 {
            out.push(queue.remove(i));
            continue;
        }
        let party = q.party;
        let size = queue.iter().filter(|x| x.party == party).count();
        if out.len() + size <= room {
            let (theirs, rest): (Vec<Queued>, Vec<Queued>) =
                queue.drain(..).partition(|x| x.party == party);
            *queue = rest;
            out.extend(theirs);
        } else {
            skipped.push(party);
            i += 1;
        }
    }
    out
}

/// Matches topped up from the queue, and a new one when both sides have
/// enough waiting.
pub fn fill(realm: &Realm, bg: u32, bracket: u8) {
    let (min, max, _, _) = template(realm, bg);
    let mut invites = Vec::new();
    {
        let mut bgs = realm.social.battlegrounds.lock();
        let Battlegrounds {
            queues, matches, ..
        } = &mut *bgs;
        let queue = queues.entry((bg, bracket)).or_default();
        let mut ids: Vec<u32> = matches
            .values()
            .filter(|m| m.bg == bg && m.bracket == bracket && !m.ended)
            .map(|m| m.id)
            .collect();
        ids.sort_unstable();
        for id in ids {
            let Some(m) = matches.get_mut(&id) else {
                continue;
            };
            for team in [Team::Alliance, Team::Horde] {
                let room = (max as usize).saturating_sub(m.count(team));
                for q in take(queue, team, room) {
                    m.invited.insert(
                        q.guid,
                        (team, Instant::now() + realm.config().bg_invite_for),
                    );
                    invites.push((q.guid, m.bg, m.id));
                }
            }
        }
        let waiting = |t: Team| queue.iter().filter(|q| q.team == t).count();
        if waiting(Team::Alliance) >= min as usize && waiting(Team::Horde) >= min as usize {
            let id = realm.social.instances.lock().new_id();
            let mut m = Match {
                id,
                bg,
                bracket,
                invited: AHashMap::new(),
                members: AHashMap::new(),
                offline: AHashMap::new(),
                started: Instant::now(),
                ended: false,
                began: None,
                ended_at: None,
            };
            for team in [Team::Alliance, Team::Horde] {
                for q in take(queue, team, max as usize) {
                    m.invited.insert(
                        q.guid,
                        (team, Instant::now() + realm.config().bg_invite_for),
                    );
                    invites.push((q.guid, bg, id));
                }
            }
            matches.insert(id, m);
        }
    }
    for (g, bg, id) in invites {
        let slot = realm
            .social
            .battlegrounds
            .lock()
            .slot_of(g, bg)
            .unwrap_or(0);
        send_frame(
            realm,
            g,
            &status_frame_at(
                slot,
                bracket,
                map_of(bg),
                id,
                WAIT_JOIN,
                realm.config().bg_invite_for.as_millis() as u32,
                0,
            ),
        );
    }
}

/// `CMSG_BATTLEFIELD_PORT`: into the match of the battleground on `map_id`
/// one is invited to, or out of the invitation (or the queue). One in a
/// match already leaves it for the new one, no deserter for it (mangos's
/// battleground "jumping", `RemovePlayerAtLeave` with no teleport), and
/// goes home from the new one to where they came from first.
pub fn port(map: &mut Map, g: Guid, map_id: u32, enter: bool) {
    let realm = map.realm.clone();
    let Some(bg_id) = bg_of_map(map_id) else {
        return;
    };
    // A deserter can't take it up: it's declined for them.
    let deserter = enter && deserter(map, g);
    let enter = enter && !deserter;
    if deserter && let Some(frame) = joined_frame(None) {
        send_frame(&realm, g, &frame);
    }
    let mut jumped = None;
    let (key, team, bg, slot) = {
        let mut bgs = realm.social.battlegrounds.lock();
        let Some(m) = bgs
            .matches
            .values_mut()
            .find(|m| m.bg == bg_id && m.invited.contains_key(&g))
        else {
            // Declined while still queued: out of the queue.
            let queued = bgs.queued_for(g, bg_id);
            if !enter && queued {
                for ((b, _), q) in bgs.queues.iter_mut() {
                    if *b == bg_id {
                        q.retain(|x| x.guid != g);
                    }
                }
                let slot = bgs.free_slot(g, bg_id).unwrap_or(0);
                drop(bgs);
                send_frame(&realm, g, &none_frame(slot));
            }
            return;
        };
        let (team, _) = m.invited.remove(&g).expect("found above");
        if enter {
            m.members.insert(g, team);
        }
        let (key, bg) = (m.key(), (m.bg, m.bracket));
        let slot = if enter {
            bgs.slot_of(g, bg_id).unwrap_or(0)
        } else {
            bgs.free_slot(g, bg_id).unwrap_or(0)
        };
        let inside = bgs
            .matches
            .get(&key.instance)
            .map(|m| m.in_progress_frame(slot, Instant::now(), realm.config().bg_auto_leave));
        // Out of the match they're in, for this one.
        if enter
            && let Some(old) = bgs
                .matches
                .values_mut()
                .find(|o| o.id != key.instance && o.members.contains_key(&g))
        {
            old.members.remove(&g);
            old.offline.remove(&g);
            let old_bg = old.bg;
            jumped = bgs.free_slot(g, old_bg);
        }
        (key, team, (bg.0, bg.1, inside), slot)
    };
    if !enter {
        send_frame(&realm, g, &none_frame(slot));
        fill(&realm, bg.0, bg.1);
        return;
    }
    if let Some(old_slot) = jumped {
        send_frame(&realm, g, &none_frame(old_slot));
        crate::game::social::groups::bg_left(&realm, g);
    }
    let (at, facing) = start_of(&realm, bg.0, team);
    if map.battleground.is_none()
        && let Some(p) = map.players.get_mut(&g)
    {
        p.bg_return = Some((map.key, p.position(), p.movement.orientation));
    }
    if let Some(inside) = bg.2 {
        send_frame(&realm, g, &inside);
    }
    map.teleport(g, key, at, facing);
}

/// Where a team starts, and rises again.
pub fn start_of(realm: &Realm, bg: u32, team: Team) -> (Vector3d, f32) {
    let loc = realm
        .data
        .battlegrounds
        .get(&bg)
        .and_then(|t| realm.data.safe_locs.get(&t.starts[side(team)]).copied());
    match loc {
        Some((_, x, y, z)) => (Vector3d { x, y, z }, 0.0),
        // Warsong Gulch's tunnels.
        None if team == Team::Alliance => (
            Vector3d {
                x: 1519.53,
                y: 1481.87,
                z: 352.02,
            },
            std::f32::consts::PI,
        ),
        None => (
            Vector3d {
                x: 933.33,
                y: 1433.72,
                z: 345.54,
            },
            0.0,
        ),
    }
}

/// Whether `g` is a deserter still.
pub fn deserter(map: &Map, g: Guid) -> bool {
    crate::game::map::auras::auras(map, g)
        .is_some_and(|a| a.has_spell(crate::game::map::battleground::SPELL_DESERTER))
}

/// `g` out of the match they are a member of (not the one on `not_on`):
/// whether it was over, and the queue slot it held. Out of their side's
/// raid too.
fn out_of_match(realm: &Realm, g: Guid, not_on: Option<MapKey>) -> Option<(bool, u32)> {
    let out = {
        let mut bgs = realm.social.battlegrounds.lock();
        let m = bgs
            .matches
            .values_mut()
            .find(|m| m.members.contains_key(&g) && Some(m.key()) != not_on)?;
        m.members.remove(&g);
        m.offline.remove(&g);
        let (ended, bg) = (m.ended, m.bg);
        let slot = bgs.free_slot(g, bg).unwrap_or(0);
        (ended, slot)
    };
    crate::game::social::groups::bg_left(realm, g);
    Some(out)
}

/// A member leaves the match (and the map): back where they came from
/// (or, logged in again inside, home). Before the end, they are a
/// deserter.
pub fn leave(map: &mut Map, g: Guid) {
    let realm = map.realm.clone();
    let Some((ended, slot)) = out_of_match(&realm, g, None) else {
        return;
    };
    send_frame(&realm, g, &none_frame(slot));
    // `[battleground] cast_deserter`, mangos's `Battleground.CastDeserter`.
    if !ended && map.cfg.bg_cast_deserter {
        crate::game::map::spells::cast_triggered(
            map,
            g,
            crate::game::map::battleground::SPELL_DESERTER,
            Some(g),
        );
    }
    // A spirit of redemption goes home a living priest, not to die there
    // (mangos's `RemovePlayerAtLeave`, Thistle Tea's `leave_battleground`).
    if crate::game::map::spells::redemption::spirit(map, g) {
        crate::game::map::spells::redemption::end_without_death(map, g);
    }
    // The dead and the ghosts go home alive and whole, their corpse gone
    // (Thistle Tea's exit, as vmangos does it); the living as they are.
    if map
        .players
        .get(&g)
        .is_some_and(|p| p.death != crate::game::map::death::Death::Alive)
    {
        crate::game::map::death::resurrect(map, g, 1.0);
    }
    let back = map.players.get_mut(&g).and_then(|p| p.bg_return.take());
    if let Some((key, pos, o)) = back {
        map.teleport(g, key, pos, o);
    } else if let Some(b) = map.players.get(&g).and_then(|p| p.row.state.bind) {
        map.teleport(
            g,
            MapKey::world(b.map),
            Vector3d {
                x: b.x,
                y: b.y,
                z: b.z,
            },
            0.0,
        );
    }
}

/// `g` arrived on `map` while still in a match elsewhere (a hearthstone,
/// a teleport): they've left it, a deserter if it was still on (Thistle
/// Tea's `world_left`).
pub fn strayed(map: &mut Map, g: Guid) {
    let realm = map.realm.clone();
    let Some((ended, slot)) = out_of_match(&realm, g, Some(map.key)) else {
        return;
    };
    send_frame(&realm, g, &none_frame(slot));
    // `[battleground] cast_deserter`, mangos's `Battleground.CastDeserter`.
    if !ended && map.cfg.bg_cast_deserter {
        crate::game::map::spells::cast_triggered(
            map,
            g,
            crate::game::map::battleground::SPELL_DESERTER,
            Some(g),
        );
    }
}

/// The match on `key` is over.
pub fn ended(realm: &Realm, key: MapKey) {
    if let Some(m) = realm
        .social
        .battlegrounds
        .lock()
        .matches
        .get_mut(&key.instance)
    {
        m.ended = true;
        m.ended_at = Some(Instant::now());
    }
}

/// The battle on `key` began.
pub fn began(realm: &Realm, key: MapKey) {
    if let Some(m) = realm
        .social
        .battlegrounds
        .lock()
        .matches
        .get_mut(&key.instance)
    {
        m.began = Some(Instant::now());
    }
}

/// Everyone in the match on `key` hears where it stands: the time since
/// it began, and (after the end) before they're sent home.
pub fn status_to_members(realm: &Realm, key: MapKey) {
    let frames = {
        let bgs = realm.social.battlegrounds.lock();
        let Some(m) = bgs.matches.get(&key.instance) else {
            return;
        };
        let now = Instant::now();
        m.members
            .keys()
            .map(|g| {
                let slot = bgs.slot_of(*g, m.bg).unwrap_or(0);
                (
                    *g,
                    m.in_progress_frame(slot, now, realm.config().bg_auto_leave),
                )
            })
            .collect::<Vec<_>>()
    };
    for (g, frame) in frames {
        send_frame(realm, g, &frame);
    }
}

/// The members of each side of the match on `key`, those offline with
/// their place kept among them.
pub fn sides(realm: &Realm, key: MapKey) -> [usize; 2] {
    let bgs = realm.social.battlegrounds.lock();
    bgs.matches.get(&key.instance).map_or([0, 0], |m| {
        [m.members_of(Team::Alliance), m.members_of(Team::Horde)]
    })
}

/// The match's copy has gone: everyone's slot for it with it.
pub fn closed(realm: &Realm, key: MapKey) {
    let mut bgs = realm.social.battlegrounds.lock();
    if let Some(m) = bgs.matches.remove(&key.instance) {
        for g in m.members.keys().chain(m.invited.keys()) {
            bgs.free_slot(*g, m.bg);
        }
    }
    drop(bgs);
    crate::game::social::groups::bg_closed(realm, key.instance);
}

/// A player going offline leaves the queues and their invitations; one in
/// a match keeps their place five minutes, to come back to (mangos's
/// `EventPlayerLoggedOut`).
pub fn logged_out(realm: &Realm, g: Guid) {
    let mut refill = Vec::new();
    {
        let mut bgs = realm.social.battlegrounds.lock();
        let mut freed = Vec::new();
        for ((bg, _), q) in bgs.queues.iter_mut() {
            if q.iter().any(|x| x.guid == g) {
                q.retain(|x| x.guid != g);
                freed.push(*bg);
            }
        }
        for m in bgs.matches.values_mut() {
            if m.members.contains_key(&g) {
                m.offline.insert(g, Instant::now() + OFFLINE_FOR);
            } else if m.invited.remove(&g).is_some() {
                refill.push((m.bg, m.bracket));
                freed.push(m.bg);
            }
        }
        for bg in freed {
            bgs.free_slot(g, bg);
        }
    }
    for (bg, bracket) in refill {
        fill(realm, bg, bracket);
    }
}

/// Logging in on `map`, a battleground's: the match `g` kept their place
/// in, still on, which takes them back. `None` sends them home.
pub fn rejoin(realm: &Realm, g: Guid, map: u32) -> Option<MapKey> {
    let mut bgs = realm.social.battlegrounds.lock();
    let found = bgs.matches.values_mut().find(|m| {
        map_of(m.bg) == map
            && !m.ended
            && (m.members.contains_key(&g) || m.invited.contains_key(&g))
    });
    let Some(m) = found else {
        // Nothing to go back to: no slot held for it.
        if let Some(bg) = bg_of_map(map)
            && bgs.match_of_bg(g, bg).is_none()
        {
            bgs.free_slot(g, bg);
        }
        return None;
    };
    if let Some((team, _)) = m.invited.remove(&g) {
        m.members.insert(g, team);
    }
    m.offline.remove(&g);
    Some(m.key())
}

/// At login: one who stayed away from their match too long is a deserter
/// now (mangos's `AT_LOGIN_ADD_BG_DESERTER`).
pub fn logged_in(map: &mut Map, g: Guid) {
    let counter = crate::game::guid::counter(g);
    let was = map
        .realm
        .social
        .battlegrounds
        .lock()
        .deserters
        .remove(&counter);
    if !was {
        return;
    }
    map.realm
        .store
        .delete_record(DESERTER_KIND, u64::from(counter));
    crate::game::map::spells::cast_triggered(
        map,
        g,
        crate::game::map::battleground::SPELL_DESERTER,
        Some(g),
    );
}

/// Invitations not taken in time lapse; members offline past their time
/// are out of the match, deserters if it is still on (mangos's
/// `BattleGround::Update`, `m_offlineQueue`).
pub fn sweep(realm: &Realm) {
    let now = Instant::now();
    // Invitations lapsed: who, the battleground, its bracket, the slot.
    let mut lapsed: Vec<(Guid, u32, u8, u32)> = Vec::new();
    // Members gone too long, and whether they deserted a match still on.
    let mut gone: Vec<(Guid, bool)> = Vec::new();
    {
        let mut bgs = realm.social.battlegrounds.lock();
        let mut freed: Vec<(Guid, u32, Option<u8>)> = Vec::new();
        for m in bgs.matches.values_mut() {
            let (bg, bracket) = (m.bg, m.bracket);
            m.invited.retain(|g, (_, until)| {
                let keep = *until > now;
                if !keep {
                    freed.push((*g, bg, Some(bracket)));
                }
                keep
            });
            let away: Vec<Guid> = m
                .offline
                .iter()
                .filter(|(_, until)| **until <= now)
                .map(|(g, _)| *g)
                .collect();
            for g in away {
                m.offline.remove(&g);
                m.members.remove(&g);
                freed.push((g, bg, None));
                gone.push((g, !m.ended));
            }
        }
        for (g, bg, bracket) in freed {
            let slot = bgs.free_slot(g, bg).unwrap_or(0);
            if let Some(bracket) = bracket {
                lapsed.push((g, bg, bracket, slot));
            }
        }
        for (g, deserts) in &gone {
            if *deserts {
                bgs.deserters.insert(crate::game::guid::counter(*g));
            }
        }
    }
    for (g, deserts) in gone {
        crate::game::social::groups::bg_left(realm, g);
        if deserts {
            realm.store.put_record(
                DESERTER_KIND,
                u64::from(crate::game::guid::counter(g)),
                &true,
            );
        }
    }
    for (g, bg, bracket, slot) in lapsed {
        send_frame(realm, g, &none_frame(slot));
        fill(realm, bg, bracket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brackets_go_by_tens() {
        assert_eq!(bracket_of(10), 0);
        assert_eq!(bracket_of(19), 0);
        assert_eq!(bracket_of(20), 1);
        assert_eq!(bracket_of(60), 5);
        assert_eq!(map_of(2), 489);
        assert_eq!(bg_of_map(529), Some(3));
    }
}
