//! Alterac Valley: graveyards to take, bunkers and towers to burn, mines
//! to win from their keepers, captains and commanders to kill, and a
//! general on each side whose death ends it. A graveyard or tower assault
//! takes five minutes; a burnt tower is gone for good. Each objective gives
//! its side honor and reputation, and at the end the side's surviving
//! towers, graveyards, mines and captain do too (Thistle Tea's model; its
//! quest-driven extras, the armor upgrades, summons and air, cavalry and
//! ground assaults, are not here).

use std::time::{Duration, Instant};

use ahash::{AHashMap, AHashSet};
use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::map::Map;
use crate::game::map::battleground::{self as bg, Assault, ControlPoint, Due};

pub const BANNERS: [u32; 9] = [
    178364, 178365, 178925, 178940, 178943, 179286, 179287, 179435, 180418,
];
pub const CAPTURE: Duration = Duration::from_secs(300);
pub const MINE_RECLAIM: Duration = Duration::from_secs(1200);
/// Stormpike Guard and Frostwolf Clan.
const REPUTATION: [u32; 2] = [730, 729];
/// The graveyard at each graveyard node (`WorldSafeLocs`).
const NODE_GRAVEYARDS: [u32; 7] = [751, 689, 729, 169, 749, 690, 750];

/// Scoreboard columns.
const GRAVEYARDS_ASSAULTED: usize = 0;
const GRAVEYARDS_DEFENDED: usize = 1;
const TOWERS_ASSAULTED: usize = 2;
const TOWERS_DEFENDED: usize = 3;
const MINES_CAPTURED: usize = 4;
const LEADERS_KILLED: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Graveyard,
    Tower,
}

/// The nodes: name, kind, home side, and world-state fields.
const NODES: [(&str, Kind, Option<usize>, [u32; 4]); 15] = [
    (
        "Stormpike Aid Station",
        Kind::Graveyard,
        Some(0),
        [1326, 1325, 1328, 1327],
    ),
    (
        "Stormpike Graveyard",
        Kind::Graveyard,
        Some(0),
        [1335, 1333, 1336, 1334],
    ),
    (
        "Stonehearth Graveyard",
        Kind::Graveyard,
        Some(0),
        [1304, 1302, 1303, 1301],
    ),
    (
        "Snowfall Graveyard",
        Kind::Graveyard,
        None,
        [1343, 1341, 1344, 1342],
    ),
    (
        "Iceblood Graveyard",
        Kind::Graveyard,
        Some(1),
        [1348, 1346, 1349, 1347],
    ),
    (
        "Frostwolf Graveyard",
        Kind::Graveyard,
        Some(1),
        [1339, 1337, 1340, 1338],
    ),
    (
        "Frostwolf Relief Hut",
        Kind::Graveyard,
        Some(1),
        [1331, 1329, 1332, 1330],
    ),
    (
        "Dun Baldar South Bunker",
        Kind::Tower,
        Some(0),
        [1375, 1361, 1378, 1370],
    ),
    (
        "Dun Baldar North Bunker",
        Kind::Tower,
        Some(0),
        [1374, 1362, 1379, 1371],
    ),
    (
        "Icewing Bunker",
        Kind::Tower,
        Some(0),
        [1376, 1363, 1380, 1372],
    ),
    (
        "Stonehearth Bunker",
        Kind::Tower,
        Some(0),
        [1377, 1364, 1381, 1373],
    ),
    (
        "Iceblood Tower",
        Kind::Tower,
        Some(1),
        [1390, 1368, 1395, 1385],
    ),
    (
        "Tower Point",
        Kind::Tower,
        Some(1),
        [1389, 1367, 1394, 1384],
    ),
    (
        "East Frostwolf Tower",
        Kind::Tower,
        Some(1),
        [1388, 1366, 1393, 1383],
    ),
    (
        "West Frostwolf Tower",
        Kind::Tower,
        Some(1),
        [1387, 1365, 1392, 1382],
    ),
];

/// The events whose creatures' deaths are objectives: the mines'
/// keepers, then the Alliance's and the Horde's leaders.
const ALLIANCE_EVENTS: [u8; 8] = [48, 52, 53, 54, 55, 61, 66, 68];
const HORDE_EVENTS: [u8; 8] = [49, 56, 57, 58, 59, 62, 67, 69];
const CAPTAIN_DELAYS: [u64; 5] = [120, 180, 240, 300, 360];

#[derive(Debug, Clone, Copy, Default)]
pub struct Node {
    pub point: ControlPoint,
    pub destroyed: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Mine {
    pub owner: Option<usize>,
    pub revision: u32,
}

impl Mine {
    /// 0 Alliance, 1 Horde, 2 its own.
    pub fn state(&self) -> u8 {
        match self.owner {
            Some(0) => 0,
            Some(_) => 1,
            None => 2,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Av {
    pub nodes: [Node; 15],
    pub mines: [Mine; 2],
    /// Objective events whose creature has fallen.
    pub defeated: AHashSet<u8>,
    /// The quest-driven extras: armor, offerings, the air, cavalry and
    /// ground assaults.
    pub extras: crate::game::map::battleground::av_quests::Extras,
}

impl Av {
    pub fn new() -> Self {
        let mut av = Self::default();
        for (i, n) in NODES.iter().enumerate() {
            av.nodes[i].point.owner = n.2;
        }
        av
    }

    /// A node's event state: 0 and 2 assaulted by the Alliance and the
    /// Horde, 1 and 3 theirs, 5 neutral.
    pub fn event_state(&self, id: usize) -> u8 {
        let p = &self.nodes[id].point;
        match (p.assaulting, p.owner) {
            (Some(0), _) => 0,
            (Some(_), _) => 2,
            (None, Some(0)) => 1,
            (None, Some(_)) => 3,
            (None, None) => 5,
        }
    }

    pub fn controlled_by(&self, id: usize) -> Option<usize> {
        if self.nodes[id].destroyed {
            None
        } else {
            self.nodes[id].point.controlled_by()
        }
    }

    /// The defenders of a node, by event: its own and its defender
    /// quest's.
    pub fn defender_events(&self, id: usize) -> Vec<(u8, u8)> {
        let p = &self.nodes[id].point;
        let team = |o: Option<usize>| match o {
            Some(0) => 0u8,
            Some(_) => 1,
            None => 2,
        };
        let upgrade = p.owner.map_or(0, |s| self.extras.armor[s].tier);
        match NODES[id].1 {
            Kind::Graveyard if p.owner.is_none() => vec![(15 + id as u8, 8)],
            Kind::Graveyard => vec![(15 + id as u8, team(p.owner) * 4 + upgrade)],
            Kind::Tower => vec![
                (15 + id as u8, team(p.owner) * 2 + 1),
                (23 + id as u8, team(p.owner) * 4 + upgrade),
            ],
        }
    }

    fn node_states(&self, id: usize) -> Vec<(u32, u32)> {
        let state = self.event_state(id);
        let fields = NODES[id].3;
        let mut out: Vec<(u32, u32)> = fields
            .iter()
            .zip([1u8, 0, 3, 2])
            .map(|(f, s)| (*f, u32::from(state == s)))
            .collect();
        if id == 3 {
            out.insert(0, (1966, u32::from(state == 5)));
        }
        out
    }

    fn mine_states(&self, id: usize) -> Vec<(u32, u32)> {
        let first = if id == 0 { 1358 } else { 1355 };
        let state = self.mines[id].state();
        (0..3)
            .map(|s| (first + u32::from(s), u32::from(state == s)))
            .collect()
    }
}

/// The events the valley starts with.
pub fn initial_events() -> AHashMap<u8, Option<u8>> {
    let av = Av::new();
    let mut e = AHashMap::new();
    for id in 0..15 {
        e.insert(id as u8, Some(av.event_state(id)));
        for (ev, s) in av.defender_events(id) {
            e.insert(ev, Some(s));
        }
    }
    for id in 0..2u8 {
        e.insert(46 + id, Some(2));
        e.insert(50 + id, Some(2));
    }
    for ev in [48u8, 49].into_iter().chain(52..=62).chain(65..=69).chain([
        100,
        101,
        bg::EVENT_GHOST_GATES,
        bg::EVENT_DOORS,
    ]) {
        e.insert(ev, Some(0));
    }
    for ev in (80u8..=87).chain(90..=97) {
        e.insert(ev, Some(2));
    }
    e
}

pub fn world_states(m: &bg::Match) -> Vec<(u32, u32)> {
    let Some(av) = m.av.as_ref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for id in 0..15 {
        out.extend(av.node_states(id));
    }
    for id in 0..2 {
        out.extend(av.mine_states(id));
    }
    out
}

pub fn start(map: &mut Map) {
    bg::announce_text(map, None, 10640);
    bg::send_world_states(map);
    let now = map.now;
    if let Some(m) = map.battleground.as_mut() {
        for s in 0..2 {
            let delay = CAPTAIN_DELAYS[rand::rng().random_range(0..CAPTAIN_DELAYS.len())];
            m.schedule(now + Duration::from_secs(delay), Due::CaptainBuff(s));
        }
    }
}

pub fn update(map: &mut Map) {
    crate::game::map::battleground::av_quests::update(map);
}

pub fn due(map: &mut Map, d: Due) {
    let now = map.now;
    match d {
        Due::Capture { node, revision } => {
            let id = usize::from(node);
            let captured = map
                .battleground
                .as_mut()
                .and_then(|m| m.av.as_mut())
                .is_some_and(|av| {
                    let ok = !av.nodes[id].destroyed && av.nodes[id].point.capture(revision, now);
                    if ok && NODES[id].1 == Kind::Tower {
                        av.nodes[id].destroyed = true;
                    }
                    ok
                });
            if captured {
                changed(map, id, Assault::Unchanged, None);
            }
        }
        Due::Banner { node, revision } => {
            let state = map
                .battleground
                .as_ref()
                .and_then(|m| m.av.as_ref())
                .filter(|av| av.nodes[usize::from(node)].point.revision == revision)
                .map(|av| av.event_state(usize::from(node)));
            if let Some(state) = state {
                bg::set_event(map, node, Some(state));
            }
        }
        Due::MineReclaim { mine, revision } => {
            let reclaimed = map
                .battleground
                .as_mut()
                .and_then(|m| m.av.as_mut())
                .is_some_and(|av| {
                    let m = &mut av.mines[usize::from(mine)];
                    let ok = m.revision == revision && m.owner.is_some();
                    if ok {
                        m.owner = None;
                        m.revision += 1;
                    }
                    ok
                });
            if reclaimed {
                mine_changed(map, usize::from(mine));
            }
        }
        Due::CaptainBuff(s) => captain_buff(map, s),
        _ => {}
    }
}

fn faction(s: usize) -> &'static str {
    ["Alliance", "Horde"][s]
}

/// `CMSG_GAMEOBJ_USE` on a banner: a graveyard or tower assaulted or
/// defended.
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
        .find(|(e, _)| *e < 15);
    let Some(m) = map.battleground.as_mut() else {
        return true;
    };
    let (Some(team), Some((id, state))) = (m.team_of(g), binding) else {
        return true;
    };
    let id = usize::from(id);
    let playing = m.playing();
    let Some(av) = m.av.as_mut() else {
        return true;
    };
    if !playing || av.event_state(id) != state || av.nodes[id].destroyed {
        return true;
    }
    let previous_owner = av.nodes[id].point.owner;
    let action = av.nodes[id].point.assault(team, now, CAPTURE);
    if action == Assault::Unchanged {
        return true;
    }
    let column = match (NODES[id].1, action) {
        (Kind::Graveyard, Assault::Assaulted) => GRAVEYARDS_ASSAULTED,
        (Kind::Graveyard, _) => GRAVEYARDS_DEFENDED,
        (Kind::Tower, Assault::Assaulted) => TOWERS_ASSAULTED,
        (Kind::Tower, _) => TOWERS_DEFENDED,
    };
    m.scores.entry(g).or_default().objectives[column] += 1;
    if action == Assault::Assaulted && previous_owner.is_some() {
        let entry = if NODES[id].1 == Kind::Tower {
            13778
        } else {
            13756
        };
        crate::game::map::quests::killed(map, g, crate::game::guid::unit(entry, 0));
    }
    changed(map, id, action, Some(g));
    true
}

/// A node changed: its banner, its defenders, the word and the rewards.
/// `action` is the assault's; `Unchanged` for a capture.
fn changed(map: &mut Map, id: usize, action: Assault, _by: Option<Guid>) {
    let now = map.now;
    let Some((point, defenders)) = map
        .battleground
        .as_ref()
        .and_then(|m| m.av.as_ref())
        .map(|av| (av.nodes[id].point, av.defender_events(id)))
    else {
        return;
    };
    let captured = action == Assault::Unchanged;
    bg::set_event(map, id as u8, None);
    let delay = if action == Assault::Assaulted {
        Duration::from_secs(1)
    } else {
        Duration::from_secs(5)
    };
    if let Some(m) = map.battleground.as_mut() {
        m.schedule(
            now + delay,
            Due::Banner {
                node: id as u8,
                revision: point.revision,
            },
        );
        if action == Assault::Assaulted {
            m.schedule(
                now + CAPTURE,
                Due::Capture {
                    node: id as u8,
                    revision: point.revision,
                },
            );
        }
    }
    for (event, state) in defenders {
        if action == Assault::Assaulted {
            bg::stop_respawns(map, event);
        } else {
            bg::set_event(map, event, Some(state));
        }
    }
    let team = point.assaulting.or(point.owner);
    let name = NODES[id].0;
    let text = match (NODES[id].1, action, team) {
        (_, _, None) => String::new(),
        (Kind::Tower, Assault::Unchanged, Some(t)) => {
            format!("The {} has destroyed {name}!", faction(t))
        }
        (_, Assault::Assaulted, Some(t)) => format!("The {} has assaulted {name}!", faction(t)),
        (_, Assault::Defended, Some(t)) => format!("The {} has defended {name}!", faction(t)),
        (_, Assault::Unchanged, Some(t)) => format!("The {} has taken {name}!", faction(t)),
    };
    if !text.is_empty() {
        bg::announce(map, team, &text);
    }
    let sound = match (action, point.assaulting, NODES[id].1, point.owner) {
        (Assault::Assaulted, Some(0), _, _) => 8212,
        (Assault::Assaulted, _, _, _) => 8174,
        (Assault::Defended, _, Kind::Tower, _) => 8192,
        (_, _, _, Some(0)) => 8173,
        _ => 8213,
    };
    bg::play_sound(map, sound);
    let states = map
        .battleground
        .as_ref()
        .and_then(|m| m.av.as_ref())
        .map(|av| av.node_states(id))
        .unwrap_or_default();
    for (s, v) in states {
        bg::world_state(map, s, v);
    }
    if captured
        && NODES[id].1 == Kind::Tower
        && let Some(owner) = point.owner
    {
        objective(map, owner, Reward::Tower, 1);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reward {
    General,
    Captain,
    Commander,
    Tower,
    SurvivingTower,
    SurvivingCaptain,
    Graveyard,
    Mine,
}

/// An objective's honor and reputation for a side: (kills of 198 honor,
/// reputation, on a weekend); some scale with the match's length.
fn objective(map: &mut Map, s: usize, kind: Reward, count: u32) {
    let now = map.now;
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    let (kills, rep, weekend_rep) = match kind {
        Reward::General => (6, 350, 525),
        Reward::Captain => (3, 125, 185),
        Reward::Commander => (1, 12, 18),
        Reward::Tower => (2, 12, 18),
        Reward::SurvivingTower => (3, 12, 18),
        Reward::SurvivingCaptain => (3, 125, 175),
        Reward::Graveyard => (1, 12, 18),
        Reward::Mine => (1, 24, 36),
    };
    let scaled = matches!(
        kind,
        Reward::General | Reward::SurvivingTower | Reward::Graveyard | Reward::Mine
    );
    let multiplier = if scaled {
        let hours = m.started_at.map_or(0.0, |t| {
            now.saturating_duration_since(t).as_secs_f32() / 3600.0
        });
        60f32.powf(hours.min(1.0) - 1.0)
    } else {
        1.0
    };
    let honor = (198.0 * kills as f32 * count as f32 * multiplier) as u32;
    let reputation = count as i32 * if m.weekend { weekend_rep } else { rep };
    bg::reward_team_honor(map, s, honor);
    bg::reward_team_reputation(map, s, REPUTATION[s], reputation);
}

/// A creature fell: a mine's keeper, a captain, a commander, an
/// explosives expert, a lieutenant or a general.
pub fn creature_died(map: &mut Map, creature: Guid, killer: Guid) {
    crate::game::map::battleground::av_quests::creature_died(map, creature);
    let spawn = map.creature(creature).and_then(|c| c.spawn_guid);
    let bindings = bg::bindings(map, true, spawn);
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    let Some(team) = m.team_of(killer) else {
        return;
    };
    let Some(av) = m.av.as_ref() else {
        return;
    };
    if !m.playing() {
        return;
    }
    let found = bindings.into_iter().find(|(event, state)| {
        if matches!(*event, 46 | 47) {
            let mine = &av.mines[usize::from(*event - 46)];
            return mine.state() == *state && mine.owner != Some(team);
        }
        let enemy = (team == 0 && HORDE_EVENTS.contains(event))
            || (team == 1 && ALLIANCE_EVENTS.contains(event));
        *state == 0 && enemy && !av.defeated.contains(event)
    });
    let Some((event, _)) = found else {
        return;
    };
    if let Some(m) = map.battleground.as_mut() {
        let column = if matches!(event, 46 | 47) {
            Some(MINES_CAPTURED)
        } else if matches!(event, 48 | 49 | 61 | 62 | 68 | 69) || (52..=59).contains(&event) {
            Some(LEADERS_KILLED)
        } else {
            None
        };
        if let Some(c) = column {
            m.scores.entry(killer).or_default().objectives[c] += 1;
        }
    }
    defeated(map, event, team);
}

fn record(map: &mut Map, event: u8) {
    if let Some(av) = map.battleground.as_mut().and_then(|m| m.av.as_mut()) {
        av.defeated.insert(event);
    }
}

fn defeated(map: &mut Map, event: u8, team: usize) {
    let now = map.now;
    match event {
        46 | 47 => {
            let id = usize::from(event - 46);
            let revision = map
                .battleground
                .as_mut()
                .and_then(|m| m.av.as_mut())
                .map(|av| {
                    let mine = &mut av.mines[id];
                    mine.owner = Some(team);
                    mine.revision += 1;
                    mine.revision
                });
            if let Some(revision) = revision {
                if let Some(m) = map.battleground.as_mut() {
                    m.schedule(
                        now + MINE_RECLAIM,
                        Due::MineReclaim {
                            mine: id as u8,
                            revision,
                        },
                    );
                }
                mine_changed(map, id);
            }
        }
        61 | 62 => {
            record(map, event);
            objective(map, team, Reward::General, 1);
            survivors(map);
            bg::stop_respawns(map, event);
            bg::team_spell(map, team, 23658);
            bg::announce_text(map, None, [7335, 7336][team]);
            bg::finish(map, team);
        }
        48 | 49 => {
            record(map, event);
            objective(map, team, Reward::Captain, 1);
            bg::stop_respawns(map, event);
            bg::set_event(map, event + 15, Some(0));
        }
        52..=59 => {
            record(map, event);
            objective(map, team, Reward::Commander, 1);
            bg::stop_respawns(map, event);
            match event {
                54 => bg::team_kill_credit(map, team, 13320),
                57 => bg::team_kill_credit(map, team, 13154),
                _ => {}
            }
        }
        66 | 67 => {
            record(map, event);
            bg::stop_respawns(map, event);
            bg::set_event(map, if event == 66 { 100 } else { 101 }, None);
            bg::team_kill_credit(map, team, if event == 66 { 13598 } else { 13597 });
        }
        68 | 69 => objective(map, team, Reward::Commander, 1),
        _ => {}
    }
}

/// The match's end: each side's surviving towers, graveyards (but its
/// own last ones), mines and captain, and on a weekend a bonus.
fn survivors(map: &mut Map) {
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    let Some(av) = m.av.as_ref() else {
        return;
    };
    let winner = (0..2).find(|s| av.defeated.contains(&[62u8, 61][*s]));
    let weekend = m.weekend;
    let mut rewards = Vec::new();
    for s in 0..2 {
        let towers = (7..15)
            .filter(|id| av.controlled_by(*id) == Some(s))
            .count() as u32;
        let graves = (1..6).filter(|id| av.controlled_by(*id) == Some(s)).count() as u32;
        let mines = av.mines.iter().filter(|mine| mine.owner == Some(s)).count() as u32;
        let captain = u32::from(!av.defeated.contains(&[48u8, 49][s]));
        for (kind, count) in [
            (Reward::SurvivingTower, towers),
            (Reward::SurvivingCaptain, captain),
            (Reward::Graveyard, graves),
            (Reward::Mine, mines),
        ] {
            if count > 0 {
                rewards.push((s, kind, count));
            }
        }
    }
    for (s, kind, count) in rewards {
        objective(map, s, kind, count);
    }
    if weekend {
        for s in 0..2 {
            bg::reward_team_honor(map, s, 1584 + if winner == Some(s) { 396 } else { 0 });
        }
    }
}

fn mine_changed(map: &mut Map, id: usize) {
    let Some((owner, state, states)) = map
        .battleground
        .as_ref()
        .and_then(|m| m.av.as_ref())
        .map(|av| (av.mines[id].owner, av.mines[id].state(), av.mine_states(id)))
    else {
        return;
    };
    bg::set_event(map, 46 + id as u8, Some(state));
    bg::set_event(map, 50 + id as u8, Some(state));
    for (s, v) in states {
        bg::world_state(map, s, v);
    }
    let name = if id == 0 {
        "Irondeep Mine"
    } else {
        "Coldtooth Mine"
    };
    match owner {
        Some(t) => {
            bg::play_sound(map, if t == 0 { 8173 } else { 8213 });
            bg::announce(
                map,
                Some(t),
                &format!("The {} has taken {name}!", faction(t)),
            );
            bg::team_kill_credit(map, t, 13796);
        }
        None => bg::announce(
            map,
            None,
            &format!("{name} has been reclaimed by its original inhabitants!"),
        ),
    }
}

fn captain_buff(map: &mut Map, s: usize) {
    let now = map.now;
    let alive = map
        .battleground
        .as_ref()
        .and_then(|m| m.av.as_ref())
        .is_some_and(|av| !av.defeated.contains(&[48u8, 49][s]));
    if !alive {
        return;
    }
    let (name, spell, sound) = if s == 0 {
        ("Balinda Stonehearth", 23693, 8232)
    } else {
        ("Galvangar", 22751, 8333)
    };
    bg::team_spell(map, s, spell);
    bg::announce(
        map,
        Some(s),
        &format!("{name} rallies the {} troops!", faction(s)),
    );
    bg::play_sound(map, sound);
    if let Some(m) = map.battleground.as_mut() {
        let delay = CAPTAIN_DELAYS[rand::rng().random_range(0..CAPTAIN_DELAYS.len())];
        m.schedule(now + Duration::from_secs(delay), Due::CaptainBuff(s));
    }
}

/// The nearest of a side's graveyards: its base's, and those it holds
/// (Thistle Tea's, the base's among them).
pub fn graveyard(map: &Map, s: usize, pos: Vector3d) -> Option<(Vector3d, f32)> {
    let av = map.battleground.as_ref()?.av.as_ref()?;
    let held: Vec<u32> = [[611u32, 610][s]]
        .into_iter()
        .chain(
            (0..7)
                .filter(|id| av.controlled_by(*id) == Some(s))
                .map(|id| NODE_GRAVEYARDS[id]),
        )
        .collect();
    bg::nearest_safe_loc(map, &held, pos)
}

/// The mines' supplies, Irondeep's then Coldtooth's.
const SUPPLIES: [[u32; 3]; 2] = [[178785, 178788, 178789], [178784, 178786, 178787]];

/// The mine whose supplies object `entry` is: 0 Irondeep, 1 Coldtooth.
pub fn supply_mine(entry: u32) -> Option<usize> {
    SUPPLIES.iter().position(|e| e.contains(&entry))
}

/// Whether `object` is a mine's supplies the side `s` may not take: only
/// the side holding the mine may, while the battle is on (Thistle Tea's
/// `Mine.supply_allowed?`).
pub fn supplies_barred(map: &Map, s: Option<usize>, object: Guid) -> bool {
    let Some(entry) = map.gameobject(object).map(|o| o.entry) else {
        return false;
    };
    let Some(mine) = supply_mine(entry) else {
        return false;
    };
    // With no match here, no one may (Thistle Tea's `ActorFactory`, which
    // denies supplies without an active match reservation).
    let Some(m) = map.battleground.as_ref() else {
        return true;
    };
    let owner = m.av.as_ref().and_then(|av| av.mines[mine].owner);
    !m.playing() || owner.is_none() || owner != s
}

#[allow(dead_code)]
fn _instant(_: Instant) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn towers_burn_and_graveyards_change_hands() {
        let now = Instant::now();
        let mut av = Av::new();
        assert_eq!(av.event_state(3), 5, "Snowfall starts neutral");
        assert_eq!(av.event_state(0), 1);
        assert_eq!(av.event_state(11), 3);
        // The Alliance assaults Iceblood Tower.
        assert_eq!(
            av.nodes[11].point.assault(0, now, CAPTURE),
            Assault::Assaulted
        );
        assert_eq!(av.event_state(11), 0);
        assert_eq!(av.controlled_by(11), None);
        let rev = av.nodes[11].point.revision;
        assert!(av.nodes[11].point.capture(rev, now + CAPTURE));
        av.nodes[11].destroyed = true;
        assert_eq!(av.event_state(11), 1);
        assert_eq!(
            av.controlled_by(11),
            None,
            "a burnt tower counts for no one"
        );
        // Defenders: a tower's own event, and its defender quest's.
        assert_eq!(av.defender_events(7), vec![(22, 1), (30, 0)]);
        assert_eq!(av.defender_events(3), vec![(18, 8)]);
        let e = initial_events();
        assert_eq!(e.get(&46), Some(&Some(2)));
        assert_eq!(e.get(&61), Some(&Some(0)));
        assert_eq!(e.get(&63), None, "the captain's death isn't yet");
    }
}
