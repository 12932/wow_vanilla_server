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
//! with room. A player who goes offline in a match keeps their place a
//! while, and logging in again takes them back in.

use std::time::{Duration, Instant};

use ahash::AHashMap;
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
    AlreadyQueued,
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
    pub started: Instant,
    pub ended: bool,
    /// When the battle began, and ended.
    pub began: Option<Instant>,
    pub ended_at: Option<Instant>,
}

impl Match {
    /// Its `SMSG_BATTLEFIELD_STATUS` for one inside: the time left before
    /// all are sent home (`leave_after` the end), and since the battle
    /// began.
    pub fn in_progress_frame(&self, now: Instant, leave_after: Duration) -> std::sync::Arc<[u8]> {
        let leave = self.ended_at.map_or(0, |t| {
            (t + leave_after).saturating_duration_since(now).as_millis() as u32
        });
        let since = self
            .began
            .map_or(0, |t| now.saturating_duration_since(t).as_millis() as u32);
        status_frame_in(
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
}

impl Battlegrounds {
    pub fn queued(&self, g: Guid) -> bool {
        self.queues.values().flatten().any(|q| q.guid == g)
    }

    pub fn match_of(&self, g: Guid) -> Option<&Match> {
        self.matches
            .values()
            .find(|m| m.members.contains_key(&g) || m.invited.contains_key(&g))
    }
}

/// `SMSG_BATTLEFIELD_STATUS` in 1.12's layout: none (map 0), or the map,
/// the match and how long.
pub fn status_frame(
    map: u32,
    instance: u32,
    status: u32,
    t1: u32,
    t2: u32,
) -> std::sync::Arc<[u8]> {
    status_frame_in(0, map, instance, status, t1, t2)
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
    let mut b = Body::default();
    b.u32(0);
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

fn template(realm: &Realm, bg: u32) -> (u32, u32, u32, u32) {
    let t = realm.data.battlegrounds.get(&bg);
    let min = realm
        .config()
        .bg_min_players
        .unwrap_or_else(|| t.map_or(10, |t| t.min_players.max(1)));
    let max = t.map_or(10, |t| t.max_players.max(1));
    let (lo, hi) = t.map_or((10, 60), |t| (t.min_level, t.max_level));
    (min, max, lo, hi)
}

/// `CMSG_BATTLEFIELD_STATUS`: where a player stands: queued, invited, in a
/// match, or none.
pub fn send_status(realm: &Realm, me: &Online) {
    let now = Instant::now();
    let frame = {
        let bgs = realm.social.battlegrounds.lock();
        let queued = bgs.queues.iter().find_map(|((bg, bracket), q)| {
            q.iter()
                .find(|x| x.guid == me.guid)
                .map(|x| (*bg, *bracket, x.since))
        });
        if let Some(m) = bgs.match_of(me.guid) {
            match m.invited.get(&me.guid) {
                Some((_, until)) => status_frame_in(
                    m.bracket,
                    map_of(m.bg),
                    m.id,
                    WAIT_JOIN,
                    until.saturating_duration_since(now).as_millis() as u32,
                    0,
                ),
                None => m.in_progress_frame(now, realm.config().bg_auto_leave),
            }
        } else if let Some((bg, bracket, since)) = queued {
            status_frame_in(
                bracket,
                map_of(bg),
                0,
                WAIT_QUEUE,
                0,
                now.saturating_duration_since(since).as_millis() as u32,
            )
        } else {
            status_frame(0, 0, 0, 0, 0)
        }
    };
    me.outbound.send_frame(frame);
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
/// where they stand.
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
    {
        let bgs = realm.social.battlegrounds.lock();
        if who
            .iter()
            .any(|e| bgs.queued(e.guid) || bgs.match_of(e.guid).is_some())
        {
            return Err(Refused::AlreadyQueued);
        }
    }
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
            Some(m.id)
        } else {
            bgs.next_party += 1;
            let party = if who.len() > 1 { bgs.next_party } else { 0 };
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
    if let Some(id) = direct {
        for e in who {
            send_frame(
                realm,
                e.guid,
                &status_frame_in(
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
        send_frame(
            realm,
            g,
            &status_frame_in(
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

/// `CMSG_BATTLEFIELD_PORT`: into the match one is invited to, or out of
/// the invitation.
pub fn port(map: &mut Map, g: Guid, enter: bool) {
    let realm = map.realm.clone();
    // A deserter can't take it up: it's declined for them.
    let deserter = enter && deserter(map, g);
    let enter = enter && !deserter;
    if deserter && let Some(frame) = joined_frame(None) {
        send_frame(&realm, g, &frame);
    }
    let (key, team, bg) = {
        let mut bgs = realm.social.battlegrounds.lock();
        let Some(m) = bgs
            .matches
            .values_mut()
            .find(|m| m.invited.contains_key(&g))
        else {
            // Declined while still queued: out of the queue.
            let queued = bgs.queued(g);
            if !enter && queued {
                for q in bgs.queues.values_mut() {
                    q.retain(|x| x.guid != g);
                }
                drop(bgs);
                send_frame(&realm, g, &status_frame(0, 0, 0, 0, 0));
            }
            return;
        };
        let (team, _) = m.invited.remove(&g).expect("found above");
        if enter {
            m.members.insert(g, team);
        }
        (
            m.key(),
            team,
            (
                m.bg,
                m.bracket,
                m.in_progress_frame(Instant::now(), realm.config().bg_auto_leave),
            ),
        )
    };
    if !enter {
        send_frame(&realm, g, &status_frame(0, 0, 0, 0, 0));
        fill(&realm, bg.0, bg.1);
        return;
    }
    let inside = bg.2;
    let (at, facing) = start_of(&realm, bg.0, team);
    if let Some(p) = map.players.get_mut(&g) {
        p.bg_return = Some((map.key, p.position(), p.movement.orientation));
    }
    send_frame(&realm, g, &inside);
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

/// A member leaves the match (and the map): back where they came from
/// (or, logged in again inside, home). Before the end, they are a
/// deserter.
pub fn leave(map: &mut Map, g: Guid) {
    let realm = map.realm.clone();
    let ended = {
        let mut bgs = realm.social.battlegrounds.lock();
        let Some(m) = bgs
            .matches
            .values_mut()
            .find(|m| m.members.contains_key(&g))
        else {
            return;
        };
        m.members.remove(&g);
        m.ended
    };
    send_frame(&realm, g, &status_frame(0, 0, 0, 0, 0));
    if !ended {
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
    let ended = {
        let mut bgs = realm.social.battlegrounds.lock();
        let Some(m) = bgs
            .matches
            .values_mut()
            .find(|m| m.members.contains_key(&g) && m.key() != map.key)
        else {
            return;
        };
        m.members.remove(&g);
        m.ended
    };
    send_frame(&realm, g, &status_frame(0, 0, 0, 0, 0));
    if !ended {
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
    let (frame, members) = {
        let bgs = realm.social.battlegrounds.lock();
        let Some(m) = bgs.matches.get(&key.instance) else {
            return;
        };
        (
            m.in_progress_frame(Instant::now(), realm.config().bg_auto_leave),
            m.members.keys().copied().collect::<Vec<_>>(),
        )
    };
    for g in members {
        send_frame(realm, g, &frame);
    }
}

/// The match's copy has gone.
pub fn closed(realm: &Realm, key: MapKey) {
    realm
        .social
        .battlegrounds
        .lock()
        .matches
        .remove(&key.instance);
}

/// A player going offline leaves the queues and their invitations; one in
/// a match keeps their place for an invitation's while, as Thistle Tea
/// keeps it, to come back to.
pub fn logged_out(realm: &Realm, g: Guid) {
    let mut refill = Vec::new();
    {
        let mut bgs = realm.social.battlegrounds.lock();
        for ((bg, bracket), q) in bgs.queues.iter_mut() {
            q.retain(|x| x.guid != g);
            let _ = (bg, bracket);
        }
        for m in bgs.matches.values_mut() {
            if let Some(team) = m.members.remove(&g) {
                if !m.ended {
                    m.invited
                        .insert(g, (team, Instant::now() + realm.config().bg_invite_for));
                }
            } else if m.invited.remove(&g).is_some() {
                refill.push((m.bg, m.bracket));
            }
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
    let m = bgs.matches.values_mut().find(|m| {
        map_of(m.bg) == map
            && !m.ended
            && (m.members.contains_key(&g) || m.invited.contains_key(&g))
    })?;
    if let Some((team, _)) = m.invited.remove(&g) {
        m.members.insert(g, team);
    }
    Some(m.key())
}

/// Invitations not taken in time lapse.
pub fn sweep(realm: &Realm) {
    let now = Instant::now();
    let mut lapsed = Vec::new();
    {
        let mut bgs = realm.social.battlegrounds.lock();
        for m in bgs.matches.values_mut() {
            m.invited.retain(|g, (_, until)| {
                let keep = *until > now;
                if !keep {
                    lapsed.push((*g, m.bg, m.bracket));
                }
                keep
            });
        }
    }
    for (g, bg, bracket) in lapsed {
        send_frame(realm, g, &status_frame(0, 0, 0, 0, 0));
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
