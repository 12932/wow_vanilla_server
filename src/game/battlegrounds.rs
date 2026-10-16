//! Battleground queues and matches, across maps: a queue per battleground
//! and level bracket, with a side for each team; a match (a copy of the
//! battleground's map) when both sides have enough, and later ones topped
//! up from the queue. The match's rules run on its map
//! ([`crate::game::map::battleground`]).

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
/// How long an invitation stands.
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
}

impl Match {
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
}

impl Battlegrounds {
    fn queued(&self, g: Guid) -> bool {
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
    let mut b = Body::default();
    b.u32(0);
    if status == 0 {
        b.u32(0);
    } else {
        b.u32(map).u8(0).u32(instance).u32(status);
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
        .config
        .bg_min_players
        .unwrap_or_else(|| t.map_or(10, |t| t.min_players.max(1)));
    let max = t.map_or(10, |t| t.max_players.max(1));
    let (lo, hi) = t.map_or((10, 60), |t| (t.min_level, t.max_level));
    (min, max, lo, hi)
}

/// `CMSG_BATTLEMASTER_JOIN`: into the queue for `bg`.
pub fn join(realm: &Realm, me: &Online, team: Team, level: u8, bg: u32) {
    let (_, _, lo, hi) = template(realm, bg);
    if u32::from(level) < lo || u32::from(level) > hi {
        return;
    }
    let bracket = bracket_of(level);
    {
        let mut bgs = realm.social.battlegrounds.lock();
        if bgs.queued(me.guid) || bgs.match_of(me.guid).is_some() {
            return;
        }
        bgs.queues.entry((bg, bracket)).or_default().push(Queued {
            guid: me.guid,
            team,
            since: Instant::now(),
        });
    }
    me.outbound
        .send_frame(status_frame(map_of(bg), 0, WAIT_QUEUE, 60_000, 0));
    fill(realm, bg, bracket);
}

/// Matches topped up from the queue, and a new one when both sides have
/// enough waiting.
pub fn fill(realm: &Realm, bg: u32, bracket: u8) {
    let (min, max, _, _) = template(realm, bg);
    let mut invites = Vec::new();
    {
        let mut bgs = realm.social.battlegrounds.lock();
        let Battlegrounds { queues, matches } = &mut *bgs;
        let queue = queues.entry((bg, bracket)).or_default();
        for m in matches
            .values_mut()
            .filter(|m| m.bg == bg && m.bracket == bracket && !m.ended)
        {
            for team in [Team::Alliance, Team::Horde] {
                while m.count(team) < max as usize {
                    let Some(i) = queue.iter().position(|q| q.team == team) else {
                        break;
                    };
                    let q = queue.remove(i);
                    m.invited
                        .insert(q.guid, (team, Instant::now() + INVITE_FOR));
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
            };
            for team in [Team::Alliance, Team::Horde] {
                for _ in 0..max {
                    let Some(i) = queue.iter().position(|q| q.team == team) else {
                        break;
                    };
                    let q = queue.remove(i);
                    m.invited
                        .insert(q.guid, (team, Instant::now() + INVITE_FOR));
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
            &status_frame(map_of(bg), id, WAIT_JOIN, INVITE_FOR.as_millis() as u32, 0),
        );
    }
}

/// `CMSG_BATTLEFIELD_PORT`: into the match one is invited to, or out of
/// the invitation.
pub fn port(map: &mut Map, g: Guid, enter: bool) {
    let realm = map.realm.clone();
    let (key, team, bg) = {
        let mut bgs = realm.social.battlegrounds.lock();
        let Some(m) = bgs
            .matches
            .values_mut()
            .find(|m| m.invited.contains_key(&g))
        else {
            return;
        };
        let (team, _) = m.invited.remove(&g).expect("found above");
        if enter {
            m.members.insert(g, team);
        }
        (m.key(), team, (m.bg, m.bracket))
    };
    if !enter {
        send_frame(&realm, g, &status_frame(0, 0, 0, 0, 0));
        fill(&realm, bg.0, bg.1);
        return;
    }
    let (at, facing) = start_of(&realm, bg.0, team);
    if let Some(p) = map.players.get_mut(&g) {
        p.bg_return = Some((map.key, p.position(), p.movement.orientation));
    }
    send_frame(
        &realm,
        g,
        &status_frame(key.map, key.instance, IN_PROGRESS, 0, 0),
    );
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

/// A member leaves the match (and the map): back where they came from.
/// Before the end, they are a deserter.
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
    let back = map.players.get_mut(&g).and_then(|p| p.bg_return.take());
    if let Some((key, pos, o)) = back {
        map.teleport(g, key, pos, o);
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

/// A player going offline leaves the queues and their invitations.
pub fn logged_out(realm: &Realm, g: Guid) {
    let mut refill = Vec::new();
    {
        let mut bgs = realm.social.battlegrounds.lock();
        for ((bg, bracket), q) in bgs.queues.iter_mut() {
            q.retain(|x| x.guid != g);
            let _ = (bg, bracket);
        }
        for m in bgs.matches.values_mut() {
            if m.invited.remove(&g).is_some() {
                refill.push((m.bg, m.bracket));
            }
            m.members.remove(&g);
        }
    }
    for (bg, bracket) in refill {
        fill(realm, bg, bracket);
    }
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
