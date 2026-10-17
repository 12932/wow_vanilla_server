//! Arathi Basin: five bases to assault and defend. An assault takes a
//! minute; until then the base is contested and its owner may take it
//! back. Bases held earn resources, faster the more a side holds; 2,000
//! win. Players rise at the nearest base their side holds.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::map::Map;
use crate::game::map::battleground::{self as bg, Assault, ControlPoint, Due};

/// The banners players click: the neutral ones of each base, then the
/// contested and owned ones.
pub const BANNERS: [u32; 9] = [
    180087, 180088, 180089, 180090, 180091, 180058, 180059, 180060, 180061,
];
/// Each base's own world state, and its neutral one.
const NODE_FIELDS: [u32; 5] = [1767, 1782, 1772, 1792, 1787];
const NEUTRAL_FIELDS: [u32; 5] = [1842, 1846, 1845, 1844, 1843];
pub const NODES: [&str; 5] = ["Stables", "Blacksmith", "Farm", "Lumber Mill", "Gold Mine"];
/// The graveyard at each base (`WorldSafeLocs`).
const NODE_GRAVEYARDS: [u32; 5] = [895, 894, 893, 897, 896];
pub const LIMIT: u32 = 2000;
pub const CAPTURE: Duration = Duration::from_secs(60);
/// Bonus honor by bracket (10-19 to 60), per 330 resources and for a win.
const HONOR: [u32; 6] = [0, 41, 68, 113, 189, 198];
/// The League of Arathor and the Defilers.
const REPUTATION: [u32; 2] = [509, 510];
/// The pickups' places.
const BUFFS: [(f32, f32, f32, f32); 5] = [
    (1185.566, 1184.629, -56.36329, 2.303831),
    (989.939, 1008.75, -42.60327, 0.8203033),
    (818.0089, 842.3543, -56.54062, 3.176533),
    (808.8463, 1185.417, 11.92161, 5.619962),
    (1147.091, 816.8362, -98.39896, 6.056293),
];

/// Scoreboard columns.
const ASSAULTED: usize = 0;
const DEFENDED: usize = 1;

#[derive(Debug, Clone, Default)]
pub struct Ab {
    pub nodes: [ControlPoint; 5],
    /// Milliseconds each side has held toward its next resources.
    pub elapsed: [u32; 2],
    pub last_resources: Option<Instant>,
    /// Resources toward the next honor and reputation.
    pub honor_progress: [u32; 2],
    pub reputation_progress: [u32; 2],
    pub near_victory: bool,
}

impl Ab {
    /// A base's state: 0 neutral, 1 Alliance contested, 2 Horde
    /// contested, 3 Alliance owned, 4 Horde owned.
    pub fn state(&self, node: usize) -> u8 {
        let p = &self.nodes[node];
        match (p.assaulting, p.owner) {
            (Some(0), _) => 1,
            (Some(_), _) => 2,
            (None, Some(0)) => 3,
            (None, Some(_)) => 4,
            (None, None) => 0,
        }
    }

    pub fn controlled(&self, s: usize) -> usize {
        self.nodes
            .iter()
            .filter(|n| n.controlled_by() == Some(s))
            .count()
    }
}

pub fn world_states(m: &bg::Match) -> Vec<(u32, u32)> {
    let Some(ab) = m.ab.as_ref() else {
        return Vec::new();
    };
    let mut out = vec![
        (1776, m.score[0]),
        (1777, m.score[1]),
        (1778, ab.controlled(1) as u32),
        (1779, ab.controlled(0) as u32),
        (1780, LIMIT),
        (1955, 1800),
        (1861, 2),
    ];
    for node in 0..5 {
        out.extend(node_states(ab, node));
    }
    out
}

fn node_states(ab: &Ab, node: usize) -> Vec<(u32, u32)> {
    let state = ab.state(node);
    let base = NODE_FIELDS[node];
    let mut out = vec![(NEUTRAL_FIELDS[node], u32::from(state == 0))];
    for (status, offset) in [(1, 2), (2, 3), (3, 0), (4, 1)] {
        out.push((base + offset, u32::from(state == status)));
    }
    out
}

/// The battle begins: the pickups go out.
pub fn start(map: &mut Map) {
    let now = map.now;
    if let Some(ab) = map.battleground.as_mut().and_then(|m| m.ab.as_mut()) {
        ab.last_resources = Some(now);
    }
    bg::start_buffs(map, &BUFFS);
    bg::announce_text(map, None, 10479);
    bg::send_world_states(map);
}

/// Every update: resources for the bases held.
pub fn update(map: &mut Map) {
    advance_resources(map);
}

pub fn due(map: &mut Map, d: Due) {
    let now = map.now;
    match d {
        Due::Capture { node, revision } => {
            advance_resources(map);
            let captured = map
                .battleground
                .as_mut()
                .filter(|m| m.playing())
                .and_then(|m| m.ab.as_mut())
                .is_some_and(|ab| ab.nodes[usize::from(node)].capture(revision, now));
            if captured {
                changed(map, usize::from(node), Change::Captured, None);
            }
        }
        Due::Banner { node, revision } => {
            let state = map
                .battleground
                .as_ref()
                .and_then(|m| m.ab.as_ref())
                .filter(|ab| ab.nodes[usize::from(node)].revision == revision)
                .map(|ab| ab.state(usize::from(node)));
            if let Some(state) = state {
                bg::set_event(map, node, Some(state));
            }
        }
        _ => {}
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    Claimed,
    Assaulted,
    Defended,
    Captured,
}

/// `CMSG_GAMEOBJ_USE` on a banner: an assault, or a defense.
pub fn use_object(map: &mut Map, g: Guid, object: Guid) -> bool {
    let now = map.now;
    let Some(o) = map.gameobject(object) else {
        return false;
    };
    if !BANNERS.contains(&o.entry) {
        return false;
    }
    let binding = bg::bindings(map, false, o.spawn_guid)
        .into_iter()
        .find(|(e, _)| *e < 5);
    let Some(m) = map.battleground.as_ref() else {
        return true;
    };
    let (Some(team), Some((node, state)), Some(ab)) = (m.team_of(g), binding, m.ab.as_ref()) else {
        return true;
    };
    let node = usize::from(node);
    if !m.playing() || ab.state(node) != state {
        return true;
    }
    advance_resources(map);
    let Some(ab) = map.battleground.as_mut().and_then(|m| m.ab.as_mut()) else {
        return true;
    };
    let was = ab.state(node);
    let change = match ab.nodes[node].assault(team, now, CAPTURE) {
        Assault::Unchanged => return true,
        Assault::Assaulted if was == 0 => Change::Claimed,
        Assault::Assaulted => Change::Assaulted,
        Assault::Defended => Change::Defended,
    };
    if let Some(m) = map.battleground.as_mut() {
        let column = if change == Change::Defended {
            DEFENDED
        } else {
            ASSAULTED
        };
        m.scores.entry(g).or_default().objectives[column] += 1;
    }
    // The base's quest credit: Arathi Basin's "Control" objectives.
    crate::game::map::quests::killed(map, g, crate::game::guid::unit(15001 + node as u32, 0));
    changed(map, node, change, Some(g));
    true
}

/// A base changed hands: its banner, the word, the world states, and the
/// rewards of holding four or five.
fn changed(map: &mut Map, node: usize, change: Change, by: Option<Guid>) {
    let now = map.now;
    let Some((point, holding)) = map
        .battleground
        .as_ref()
        .and_then(|m| m.ab.as_ref())
        .map(|ab| (ab.nodes[node], (ab.controlled(0), ab.controlled(1))))
    else {
        return;
    };
    let team = point.assaulting.or(point.owner).unwrap_or(0);
    // The banner comes down, and goes up again in its new state.
    bg::set_event(map, node as u8, None);
    let delay = if matches!(change, Change::Assaulted | Change::Claimed) {
        Duration::from_secs(1)
    } else {
        Duration::from_secs(5)
    };
    if let Some(m) = map.battleground.as_mut() {
        m.schedule(
            now + delay,
            Due::Banner {
                node: node as u8,
                revision: point.revision,
            },
        );
        if matches!(change, Change::Assaulted | Change::Claimed) {
            m.schedule(
                now + CAPTURE,
                Due::Capture {
                    node: node as u8,
                    revision: point.revision,
                },
            );
        }
    }
    let name = NODES[node];
    let faction = ["Alliance", "Horde"][team];
    let actor = by
        .and_then(|g| map.players.get(&g).map(|p| p.name.to_string()))
        .unwrap_or_default();
    let text = match change {
        Change::Claimed => format!(
            "{actor} claims the {name}! If left unchallenged, the {faction} will control it in 1 minute!"
        ),
        Change::Assaulted => format!("{actor} has assaulted the {name}!"),
        Change::Defended => format!("{actor} has defended the {name}!"),
        Change::Captured => format!("The {faction} has taken the {name}!"),
    };
    bg::announce(map, Some(team), &text);
    let sound = match (change, point.assaulting, point.owner) {
        (Change::Claimed, _, _) => 8192,
        (Change::Assaulted, Some(0), _) => 8212,
        (Change::Assaulted, _, _) => 8174,
        (_, _, Some(0)) => 8173,
        _ => 8213,
    };
    bg::play_sound(map, sound);
    bg::send_world_states(map);
    if matches!(change, Change::Captured | Change::Defended) {
        let count = [holding.0, holding.1][team];
        if count >= 4 {
            bg::team_spell(map, team, 24061);
        }
        if count == 5 {
            bg::team_spell(map, team, 24064);
        }
    }
}

/// Resources for the time since they were last counted.
fn advance_resources(map: &mut Map) {
    let now = map.now;
    let Some(m) = map.battleground.as_mut() else {
        return;
    };
    if !m.playing() {
        return;
    }
    let weekend = m.weekend;
    let bracket = usize::from(m.bracket).min(5);
    let Some(ab) = m.ab.as_mut() else {
        return;
    };
    let last = ab.last_resources.unwrap_or(now);
    let elapsed = now.saturating_duration_since(last).as_millis() as u32;
    if elapsed == 0 {
        return;
    }
    ab.last_resources = Some(now.max(last));
    let mut gained = [0u32; 2];
    for (s, gain) in gained.iter_mut().enumerate() {
        let count = ab.controlled(s);
        if count == 0 {
            continue;
        }
        let interval = match count {
            1 => 12_000,
            2 => 9_000,
            3 => 6_000,
            4 => 3_000,
            _ => 1_000,
        };
        let per_tick = if count == 5 { 30 } else { 10 };
        let accumulated = ab.elapsed[s] + elapsed;
        let remaining_ticks = (LIMIT - m.score[s].min(LIMIT)).div_ceil(per_tick);
        let ticks = (accumulated / interval).min(remaining_ticks);
        ab.elapsed[s] = accumulated - ticks * interval;
        if ticks > 0 {
            *gain = ticks * per_tick;
            m.score[s] = (m.score[s] + *gain).min(LIMIT);
        }
    }
    let mut honor = [0u32; 2];
    let mut reputation = [0i32; 2];
    let (honor_every, rep_every) = if weekend { (200, 150) } else { (330, 200) };
    for s in 0..2 {
        if gained[s] == 0 {
            continue;
        }
        let h = ab.honor_progress[s] + gained[s];
        ab.honor_progress[s] = h % honor_every;
        honor[s] = h / honor_every * HONOR[bracket];
        let r = ab.reputation_progress[s] + gained[s];
        ab.reputation_progress[s] = r % rep_every;
        reputation[s] = (r / rep_every * 10) as i32;
    }
    let near = !ab.near_victory && (m.score[0] > 1800 || m.score[1] > 1800);
    if near {
        ab.near_victory = true;
    }
    let score = m.score;
    for s in 0..2 {
        if gained[s] > 0 {
            bg::world_state(map, [1776, 1777][s], score[s]);
        }
        bg::reward_team_honor(map, s, honor[s]);
        if reputation[s] > 0 {
            bg::reward_team_reputation(map, s, REPUTATION[s], reputation[s]);
        }
    }
    if near {
        let s = if score[0] > 1800 { 0 } else { 1 };
        bg::announce_text(map, None, [10598, 10599][s]);
        bg::play_sound(map, [8457, 8456][s]);
    }
    if let Some(winner) = (0..2).find(|s| score[*s] >= LIMIT) {
        let bonus = HONOR[bracket] * if weekend { 2 } else { 1 };
        bg::reward_team_honor(map, winner, bonus);
        bg::announce_text(map, None, [10633, 10634][winner]);
        bg::finish(map, winner);
    }
}

/// The nearest base a side holds with a graveyard.
pub fn graveyard(map: &Map, s: usize, pos: Vector3d) -> Option<(Vector3d, f32)> {
    let ab = map.battleground.as_ref()?.ab.as_ref()?;
    let held: Vec<u32> = (0..5)
        .filter(|n| ab.nodes[*n].controlled_by() == Some(s))
        .map(|n| NODE_GRAVEYARDS[n])
        .collect();
    bg::nearest_safe_loc(map, &held, pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_base_is_contested_then_held() {
        let now = Instant::now();
        let mut ab = Ab::default();
        assert_eq!(ab.state(0), 0);
        assert_eq!(ab.nodes[0].assault(0, now, CAPTURE), Assault::Assaulted);
        assert_eq!(ab.state(0), 1);
        let rev = ab.nodes[0].revision;
        assert!(!ab.nodes[0].capture(rev, now), "not before the minute");
        assert!(ab.nodes[0].capture(rev, now + CAPTURE));
        assert_eq!(ab.state(0), 3);
        assert_eq!(ab.controlled(0), 1);
        // The Horde assaults; the Alliance defends at once.
        assert_eq!(ab.nodes[0].assault(1, now, CAPTURE), Assault::Assaulted);
        assert_eq!(ab.state(0), 2);
        let stale = ab.nodes[0].revision;
        assert_eq!(ab.nodes[0].assault(0, now, CAPTURE), Assault::Defended);
        assert_eq!(ab.state(0), 3);
        assert!(!ab.nodes[0].capture(stale, now + CAPTURE), "a stale timer");
    }
}
