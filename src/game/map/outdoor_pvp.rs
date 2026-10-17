//! World PvP, as Thistle Tea has it: the four towers of the Eastern
//! Plaguelands, taken by standing at them, and the silithyst of Silithus,
//! carried home for the side's favor.
//!
//! A tower's capture meter moves with how many of each side stand within
//! its reach, from neutral through contested and in progress to held. The
//! side holding more towers has its zone's buff; each capture is
//! announced across the continent and credited to the side's players near.
//! In Silithus, silithyst picked up at a geyser is carried to the side's
//! camp; 200 brought home give the side the Cenarion Favor.

use std::time::{Duration, Instant};

use ahash::{AHashMap, AHashSet};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::{GameObjectSpawn, GameObjectTemplate};
use crate::game::fields::*;
use crate::game::map::battleground::side;
use crate::game::map::death::Death;
use crate::game::map::gameobject::GameObject;
use crate::game::map::{Map, MapKey};

/// How often the meters move.
pub const TICK: Duration = Duration::from_secs(1);

/// The Eastern Plaguelands, and the zones that see its towers' buff.
pub const PLAGUELANDS: u32 = 139;
const PLAGUELANDS_BUFF_ZONES: [u32; 3] = [139, 2017, 2057];

/// A tower: its capture point's object, where it stands, its world-state
/// fields by phase, who is credited for it, and its announcements.
struct TowerDef {
    entry: u32,
    at: (f32, f32, f32, f32),
    rotation: [f32; 4],
    states: [u32; 7],
    credit: u32,
    credit_at: [(f32, f32, f32); 2],
    announcements: [u32; 2],
}

const TOWERS: [TowerDef; 4] = [
    // Crown Guard.
    TowerDef {
        entry: 182096,
        at: (1860.85, -3731.23, 196.716, -2.53214),
        rotation: [0.033967, -0.131914, 0.944741, -0.298177],
        states: [2355, 2374, 2375, 2376, 2377, 2378, 2379],
        credit: 17689,
        credit_at: [(1860.59, -3730.8, 197.854), (1860.48, -3731.34, 197.778)],
        announcements: [13632, 13633],
    },
    // Eastwall.
    TowerDef {
        entry: 182097,
        at: (2574.51, -4794.89, 144.704, -1.45003),
        rotation: [-0.097056, 0.095578, -0.656229, 0.742165],
        states: [2361, 2359, 2360, 2357, 2358, 2354, 2356],
        credit: 17690,
        credit_at: [(2574.12, -4795.33, 145.871), (2574.0, -4794.79, 145.881)],
        announcements: [13631, 13636],
    },
    // Northpass.
    TowerDef {
        entry: 181899,
        at: (3181.08, -4379.36, 174.123, -2.03472),
        rotation: [-0.065392, 0.119494, -0.842275, 0.521553],
        states: [2352, 2362, 2363, 2364, 2365, 2372, 2373],
        credit: 17696,
        credit_at: [(3180.54, -4379.31, 175.275), (3180.48, -4379.07, 174.995)],
        announcements: [13630, 13635],
    },
    // Plaguewood.
    TowerDef {
        entry: 182098,
        at: (2962.71, -3042.31, 154.789, 2.08426),
        rotation: [-0.074807, -0.113837, 0.855928, 0.49883],
        states: [2353, 2366, 2367, 2368, 2369, 2370, 2371],
        credit: 17698,
        credit_at: [(2962.6, -3041.96, 155.835), (2963.02, -3041.9, 155.965)],
        announcements: [13629, 13634],
    },
];

/// The buffs of holding one to four towers: the Alliance's, the Horde's.
const TOWER_BUFFS: [[u32; 4]; 2] = [[11413, 11414, 11415, 1386], [30880, 30683, 30682, 29520]];
const VICTORY: [u32; 2] = [13638, 13637];

/// A capture meter's phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Neutral,
    Contested(usize),
    Progress(usize),
    Controlled(usize),
}

impl Phase {
    /// The phase's index among a tower's world-state fields.
    fn index(self) -> usize {
        match self {
            Phase::Neutral => 0,
            Phase::Contested(s) => 1 + s,
            Phase::Progress(s) => 3 + s,
            Phase::Controlled(s) => 5 + s,
        }
    }

    pub fn owner(self) -> Option<usize> {
        match self {
            Phase::Progress(s) | Phase::Controlled(s) => Some(s),
            _ => None,
        }
    }
}

/// A capture meter, from its object's template (type 29).
#[derive(Debug, Clone, Copy)]
pub struct Meter {
    pub radius: f32,
    pub display_state: u32,
    pub position_state: u32,
    pub neutral_state: u32,
    pub neutral_percent: i64,
    pub min_time: i64,
    pub max_time: i64,
    /// Signed progress, in thousandths of a second of capture: the
    /// Alliance's way positive.
    pub progress: i64,
    pub difference: i64,
    pub phase: Phase,
}

impl Meter {
    pub fn from_template(t: &GameObjectTemplate) -> Option<Self> {
        if t.kind != 29 || t.data[0] == 0 || t.data[17] == 0 {
            return None;
        }
        Some(Self {
            radius: t.data[0] as f32,
            display_state: t.data[2],
            position_state: t.data[3],
            neutral_state: t.data[13],
            neutral_percent: i64::from(t.data[12]).clamp(0, 100),
            min_time: if t.data[16] > 0 { i64::from(t.data[16]) } else { 60 },
            max_time: i64::from(t.data[17]),
            progress: 0,
            difference: 0,
            phase: Phase::Neutral,
        })
    }

    /// `elapsed` milliseconds with `alliance` and `horde` at the point.
    pub fn advance(&mut self, alliance: i64, horde: i64, elapsed: i64) {
        if elapsed <= 0 {
            return;
        }
        let difference = alliance - horde;
        let maximum = self.max_time * 1000;
        let ceiling = maximum * elapsed / self.min_time.max(1);
        let shift = (difference * elapsed).clamp(-ceiling, ceiling);
        self.progress = (self.progress + shift).clamp(-maximum, maximum);
        self.difference = difference;
        let neutral = maximum * self.neutral_percent / 100;
        self.phase = if self.progress == maximum {
            Phase::Controlled(0)
        } else if self.progress == -maximum {
            Phase::Controlled(1)
        } else if self.progress > 0 && self.progress >= neutral {
            Phase::Progress(0)
        } else if self.progress < 0 && self.progress <= -neutral {
            Phase::Progress(1)
        } else if difference > 0 {
            Phase::Contested(0)
        } else if difference < 0 {
            Phase::Contested(1)
        } else {
            Phase::Neutral
        };
    }

    /// The client's slider, 0 (the Horde's) to 100 (the Alliance's).
    pub fn slider(&self) -> u32 {
        let d = self.max_time * 2000;
        let n = (self.progress + self.max_time * 1000) * 100;
        ((n + d - 1) / d.max(1)).clamp(0, 100) as u32
    }
}

#[derive(Debug, Clone)]
pub struct Tower {
    pub meter: Meter,
    pub object: Option<Guid>,
}

/// Silithus: silithyst brought home by each side, toward 200.
pub const SILITHUS: u32 = 1377;
const SILITHUS_BUFF_ZONES: [u32; 3] = [1377, 3428, 3429];
pub const SPELL_CARRYING: u32 = 29519;
const SPELL_TEAM_CARRYING: [u32; 2] = [29894, 29895];
const SPELL_FAVOR: u32 = 30754;
const SILITHYST_MOUND: u32 = 181597;
const SILITHYST_GEYSER: u32 = 181598;
const TURN_IN: [u32; 2] = [4162, 4168];
const TURN_IN_REWARDS: [u32; 3] = [29534, 31420, 31247];
const TURN_IN_CREDIT: [u32; 2] = [17090, 18199];
pub const SILITHYST_LIMIT: u32 = 200;

#[derive(Debug, Clone, Default)]
pub struct Silithus {
    pub brought: [u32; 2],
    /// The side with the Cenarion Favor.
    pub controller: Option<usize>,
    /// Those carrying silithyst, for when they fall.
    pub carriers: AHashSet<Guid>,
}

/// A continent's world PvP.
#[derive(Debug, Clone, Default)]
pub struct Outdoor {
    pub towers: Vec<Tower>,
    /// Who stands at which tower.
    pub members: AHashMap<Guid, usize>,
    /// Players in the zone, who see its states.
    pub watching: AHashSet<Guid>,
    pub next_tick: Option<Instant>,
    pub last_tick: Option<Instant>,
    pub silithus: Option<Silithus>,
}

impl Outdoor {
    pub fn counts(&self) -> [usize; 2] {
        let mut c = [0, 0];
        for t in &self.towers {
            if let Some(s) = t.meter.phase.owner() {
                c[s] += 1;
            }
        }
        c
    }

    /// The Plaguelands' world states: towers held, and each tower's phase.
    pub fn tower_states(&self) -> Vec<(u32, u32)> {
        let c = self.counts();
        let mut out = vec![(2327, c[0] as u32), (2328, c[1] as u32)];
        for (i, def) in TOWERS.iter().enumerate() {
            let phase = self
                .towers
                .get(i)
                .map_or(Phase::Neutral, |t| t.meter.phase)
                .index();
            for (j, field) in def.states.iter().enumerate() {
                out.push((*field, u32::from(j == phase)));
            }
        }
        out
    }
}

/// World PvP for the continents.
pub fn for_map(key: MapKey) -> Option<Outdoor> {
    match key {
        MapKey { map: 0, instance: 0 } => Some(Outdoor::default()),
        MapKey { map: 1, instance: 0 } => Some(Outdoor {
            silithus: Some(Silithus::default()),
            ..Default::default()
        }),
        _ => None,
    }
}

/// Whether a player takes part: alive, not a GM, not on a flight, flagged
/// for PvP (or on a PvP realm), and seen.
fn eligible(map: &Map, p: &crate::game::map::player::Player) -> bool {
    p.death == Death::Alive
        && !p.fields.has_flag(PLAYER_FLAGS, crate::game::map::player::PLAYER_FLAGS_GM)
        && p.flight.is_none()
        && (map.realm.config.pvp_realm || p.pvp.desired)
        && !p.auras.has(crate::game::map::auras::AURA_MOD_STEALTH)
        && !p.auras.has(crate::game::map::auras::AURA_MOD_INVISIBILITY)
}

/// Every update: the meters move each second.
pub fn update(map: &mut Map) {
    let now = map.now;
    let Some(o) = map.outdoor.as_ref() else {
        return;
    };
    if o.next_tick.is_some_and(|t| t > now) {
        return;
    }
    let last = o.last_tick.unwrap_or(now);
    if let Some(o) = map.outdoor.as_mut() {
        o.next_tick = Some(now + TICK);
        o.last_tick = Some(now);
    }
    if map.key.map == 0 {
        if map.outdoor.as_ref().is_some_and(|o| o.towers.is_empty()) {
            place_towers(map);
        }
        towers(map, now.saturating_duration_since(last).as_millis() as i64);
    }
    if map.key.map == 1 {
        silithus_update(map);
    }
}

/// The towers' capture points go up.
fn place_towers(map: &mut Map) {
    let data = map.realm.data.clone();
    let mut towers = Vec::new();
    for (i, def) in TOWERS.iter().enumerate() {
        let Some(t) = data.gameobject_templates.get(&def.entry) else {
            continue;
        };
        let Some(meter) = Meter::from_template(t) else {
            continue;
        };
        let mut go = GameObject::from_spawn(
            t,
            &GameObjectSpawn {
                guid: 0x7F_0000 + i as u32,
                entry: def.entry,
                map: 0,
                x: def.at.0,
                y: def.at.1,
                z: def.at.2,
                orientation: def.at.3,
                rotation: def.rotation,
                state: 1,
                anim_progress: 255,
                ..Default::default()
            },
        );
        go.spawn_guid = None;
        let g = map.add_gameobject(go);
        towers.push(Tower {
            meter,
            object: Some(g),
        });
    }
    if let Some(o) = map.outdoor.as_mut() {
        o.towers = towers;
    }
}

fn send_states(map: &Map, g: Guid, states: &[(u32, u32)]) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    for (state, value) in states {
        p.outbound.send(&SMSG_UPDATE_WORLD_STATE {
            state: WorldState {
                state: *state,
                value: *value,
            },
        });
    }
}

/// The Plaguelands' meters for `elapsed` milliseconds.
fn towers(map: &mut Map, elapsed: i64) {
    let Some(o) = map.outdoor.as_ref() else {
        return;
    };
    if o.towers.is_empty() {
        return;
    }
    // Who is in the zone, and who stands at each tower.
    let in_zone: Vec<(Guid, usize, Vector3dLike, bool)> = map
        .players
        .values()
        .filter(|p| p.zone == PLAGUELANDS)
        .map(|p| {
            let pos = p.position();
            (p.guid, side(p.team), (pos.x, pos.y, pos.z), eligible(map, p))
        })
        .collect();
    let mut members: AHashMap<Guid, usize> = AHashMap::new();
    for (g, _, pos, ok) in &in_zone {
        if !ok {
            continue;
        }
        let nearest = o
            .towers
            .iter()
            .enumerate()
            .map(|(i, t)| (i, dist(TOWERS[i].at, *pos), t.meter.radius))
            .filter(|(_, d, r)| d < r)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, _, _)) = nearest {
            members.insert(*g, i);
        }
    }
    let before: Vec<Phase> = o.towers.iter().map(|t| t.meter.phase).collect();
    let before_states = o.tower_states();
    let before_counts = o.counts();
    let before_sliders: Vec<u32> = o.towers.iter().map(|t| t.meter.slider()).collect();
    let before_members = o.members.clone();
    let before_watching = o.watching.clone();
    let mut counts = vec![[0i64; 2]; o.towers.len()];
    for (g, i) in &members {
        if let Some((_, s, _, _)) = in_zone.iter().find(|x| x.0 == *g) {
            counts[*i][*s] += 1;
        }
    }
    let watching: AHashSet<Guid> = in_zone.iter().map(|x| x.0).collect();
    let Some(o) = map.outdoor.as_mut() else {
        return;
    };
    for (i, t) in o.towers.iter_mut().enumerate() {
        t.meter.advance(counts[i][0], counts[i][1], elapsed);
    }
    o.members = members.clone();
    o.watching = watching.clone();
    let after: Vec<Phase> = o.towers.iter().map(|t| t.meter.phase).collect();
    let after_states = o.tower_states();
    let after_counts = o.counts();
    let meters: Vec<Meter> = o.towers.iter().map(|t| t.meter).collect();
    // Those come into the zone see its states.
    for g in watching.difference(&before_watching) {
        send_states(map, *g, &after_states);
    }
    if after_states != before_states {
        for g in &watching {
            send_states(map, *g, &after_states);
        }
    }
    // The sliders of those at a tower.
    for (g, i) in &members {
        let m = &meters[*i];
        if before_members.get(g) != Some(i) {
            send_states(
                map,
                *g,
                &[
                    (m.display_state, 1),
                    (m.neutral_state, m.neutral_percent as u32),
                    (m.position_state, m.slider()),
                ],
            );
        } else if before_sliders[*i] != m.slider() || before[*i] != after[*i] {
            send_states(map, *g, &[(m.position_state, m.slider())]);
        }
    }
    for (g, i) in &before_members {
        if !members.contains_key(g) {
            send_states(map, *g, &[(meters[*i].display_state, 0)]);
        }
    }
    // Ownership changes: the word, the credit, the buffs.
    for i in 0..after.len() {
        let (was, now) = (before[i].owner(), after[i].owner());
        if was == now {
            continue;
        }
        if let Some(s) = now {
            defense(map, TOWERS[i].announcements[s]);
            capture_credit(map, i, s);
        }
        let sound = match (before[i], after[i]) {
            (_, Phase::Controlled(0)) => Some(8455),
            (_, Phase::Controlled(_)) => Some(8454),
            (Phase::Controlled(0), Phase::Progress(0)) => Some(8332),
            (Phase::Controlled(1), Phase::Progress(1)) => Some(8333),
            (_, Phase::Progress(0)) => Some(8173),
            (_, Phase::Progress(_)) => Some(8213),
            _ => None,
        };
        if let Some(sound) = sound {
            for g in &watching {
                if let Some(p) = map.players.get(g) {
                    p.outbound.send(&SMSG_PLAY_SOUND { sound_id: sound });
                }
            }
        }
    }
    for s in 0..2 {
        if after_counts[s] == 4 && before_counts[s] != 4 {
            defense(map, VICTORY[s]);
        }
    }
    if after_counts != before_counts {
        tower_buffs(map, after_counts);
    }
}

type Vector3dLike = (f32, f32, f32);

fn dist(a: (f32, f32, f32, f32), b: Vector3dLike) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) + (a.2 - b.2).powi(2)).sqrt()
}

/// A defense message to everyone on the continent.
fn defense(map: &Map, text: u32) {
    let Some(line) = map.realm.data.npc.broadcast_line(text, false) else {
        return;
    };
    let msg = SMSG_DEFENSE_MESSAGE {
        area: Area::try_from(PLAGUELANDS).unwrap_or_default(),
        message: line.to_owned(),
    };
    for p in map.players.values() {
        p.outbound.send(&msg);
    }
}

/// A side's players near the tower they took get its credit.
fn capture_credit(map: &mut Map, tower: usize, s: usize) {
    let at = TOWERS[tower].credit_at[s];
    let near: Vec<Guid> = map
        .players
        .values()
        .filter(|p| side(p.team) == s && eligible(map, p))
        .filter(|p| {
            let pos = p.position();
            ((pos.x - at.0).powi(2) + (pos.y - at.1).powi(2) + (pos.z - at.2).powi(2)).sqrt()
                <= 100.0
        })
        .map(|p| p.guid)
        .collect();
    for g in near {
        crate::game::map::quests::killed(map, g, crate::game::guid::unit(TOWERS[tower].credit, 0));
    }
}

/// The buff of the towers held, for each side's players in the zones.
fn tower_buffs(map: &mut Map, counts: [usize; 2]) {
    let players: Vec<(Guid, usize)> = map
        .players
        .values()
        .filter(|p| PLAGUELANDS_BUFF_ZONES.contains(&p.zone))
        .map(|p| (p.guid, side(p.team)))
        .collect();
    for (g, s) in players {
        apply_tower_buff(map, g, s, counts[s]);
    }
}

fn apply_tower_buff(map: &mut Map, g: Guid, s: usize, count: usize) {
    let want = (1..=4).contains(&count).then(|| TOWER_BUFFS[s][count - 1]);
    for b in TOWER_BUFFS[s] {
        if Some(b) != want {
            crate::game::map::auras::remove_spell(map, g, b, None);
        }
    }
    if let Some(b) = want
        && !map.players.get(&g).is_some_and(|p| p.auras.has_spell(b))
    {
        crate::game::map::spells::cast_triggered(map, g, b, Some(g));
    }
}

/// `CMSG_GAMEOBJ_USE` on silithyst: picked up. Returns whether it was.
pub fn use_object(map: &mut Map, g: Guid, object: Guid) -> bool {
    if map.outdoor.as_ref().is_none_or(|o| o.silithus.is_none()) {
        return false;
    }
    let Some(o) = map.gameobject(object) else {
        return false;
    };
    let (entry, dropped) = (o.entry, o.spawn_guid.is_none());
    if !matches!(entry, SILITHYST_MOUND | SILITHYST_GEYSER) {
        return false;
    }
    let Some(p) = map.players.get(&g) else {
        return true;
    };
    if p.death != Death::Alive || p.auras.has_spell(SPELL_CARRYING) {
        return true;
    }
    let s = side(p.team);
    crate::game::map::spells::cast_triggered(map, g, SPELL_CARRYING, Some(g));
    crate::game::map::spells::cast_triggered(map, g, SPELL_TEAM_CARRYING[s], Some(g));
    // Carrying it flags the carrier.
    let now = map.now;
    if let Some(p) = map.players.get_mut(&g) {
        p.pvp.until = Some(now + crate::game::map::pvp::PVP_LINGER);
    }
    if let Some(si) = map.outdoor.as_mut().and_then(|o| o.silithus.as_mut()) {
        si.carriers.insert(g);
    }
    // A mound a carrier dropped is gone once picked up.
    if dropped {
        map.remove_gameobject(object);
    }
    true
}

/// An area trigger: a carrier home with silithyst.
pub fn area_trigger(map: &mut Map, g: Guid, trigger: u32) {
    if map.outdoor.as_ref().is_none_or(|o| o.silithus.is_none()) {
        return;
    }
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let s = side(p.team);
    if TURN_IN[s] != trigger || p.death != Death::Alive || !p.auras.has_spell(SPELL_CARRYING) {
        return;
    }
    crate::game::map::auras::remove_spell(map, g, SPELL_CARRYING, None);
    crate::game::map::auras::remove_spell(map, g, SPELL_TEAM_CARRYING[s], None);
    for r in TURN_IN_REWARDS {
        crate::game::map::spells::cast_triggered(map, g, r, Some(g));
    }
    crate::game::map::quests::killed(map, g, crate::game::guid::unit(TURN_IN_CREDIT[s], 0));
    let Some(si) = map.outdoor.as_mut().and_then(|o| o.silithus.as_mut()) else {
        return;
    };
    si.carriers.remove(&g);
    si.brought[s] += 1;
    let captured = si.brought[s] >= SILITHYST_LIMIT;
    if captured {
        si.brought = [0, 0];
        si.controller = Some(s);
    }
    let states = silithus_states(si);
    let watching: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.zone == SILITHUS)
        .map(|p| p.guid)
        .collect();
    for w in watching {
        send_states(map, w, &states);
    }
    if captured {
        favor(map);
    }
}

fn silithus_states(si: &Silithus) -> Vec<(u32, u32)> {
    vec![
        (2313, si.brought[0]),
        (2314, si.brought[1]),
        (2317, SILITHYST_LIMIT),
    ]
}

/// The Cenarion Favor for the controlling side in the zones, and off the
/// other.
fn favor(map: &mut Map) {
    let controller = map
        .outdoor
        .as_ref()
        .and_then(|o| o.silithus.as_ref())
        .and_then(|si| si.controller);
    let players: Vec<(Guid, usize, bool)> = map
        .players
        .values()
        .map(|p| (p.guid, side(p.team), SILITHUS_BUFF_ZONES.contains(&p.zone)))
        .collect();
    for (g, s, in_zone) in players {
        let has = map
            .players
            .get(&g)
            .is_some_and(|p| p.auras.has_spell(SPELL_FAVOR));
        if in_zone && controller == Some(s) {
            if !has {
                crate::game::map::spells::cast_triggered(map, g, SPELL_FAVOR, Some(g));
            }
        } else if has {
            crate::game::map::auras::remove_spell(map, g, SPELL_FAVOR, None);
        }
    }
}

/// Each second in Kalimdor: a fallen carrier drops the silithyst, those
/// come into Silithus see its states, and the favor follows the zones.
fn silithus_update(map: &mut Map) {
    let Some(si) = map.outdoor.as_ref().and_then(|o| o.silithus.as_ref()) else {
        return;
    };
    let dropped: Vec<(Guid, Vector3d)> = si
        .carriers
        .iter()
        .filter_map(|g| match map.players.get(g) {
            Some(p) if p.auras.has_spell(SPELL_CARRYING) => None,
            Some(p) => Some((*g, p.position())),
            None => Some((*g, Vector3d { x: 0.0, y: 0.0, z: 0.0 })),
        })
        .collect();
    let states = silithus_states(si);
    let entering: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.zone == SILITHUS && !map.outdoor.as_ref().is_some_and(|o| o.watching.contains(&p.guid)))
        .map(|p| p.guid)
        .collect();
    let watching: AHashSet<Guid> = map
        .players
        .values()
        .filter(|p| p.zone == SILITHUS)
        .map(|p| p.guid)
        .collect();
    for g in &entering {
        send_states(map, *g, &states);
    }
    for (g, at) in dropped {
        if let Some(si) = map.outdoor.as_mut().and_then(|o| o.silithus.as_mut()) {
            si.carriers.remove(&g);
        }
        // Only one who fell (rather than left) leaves it behind.
        let fell = map.players.get(&g).is_some_and(|p| p.death != Death::Alive);
        if fell {
            drop_silithyst(map, at);
        }
    }
    if let Some(o) = map.outdoor.as_mut() {
        o.watching = watching;
    }
    favor(map);
}

fn drop_silithyst(map: &mut Map, at: Vector3d) {
    let data = map.realm.data.clone();
    let Some(t) = data.gameobject_templates.get(&SILITHYST_MOUND) else {
        return;
    };
    let counter = map.next_creature_counter;
    map.next_creature_counter += 1;
    let mut go = GameObject::from_spawn(
        t,
        &GameObjectSpawn {
            guid: counter | 0x80_0000,
            entry: SILITHYST_MOUND,
            map: map.key.map,
            x: at.x,
            y: at.y,
            z: at.z,
            state: 1,
            ..Default::default()
        },
    );
    go.spawn_guid = None;
    map.add_gameobject(go);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meter() -> Meter {
        let mut t = GameObjectTemplate {
            entry: 182097,
            kind: 29,
            ..Default::default()
        };
        t.data[0] = 80;
        t.data[2] = 2426;
        t.data[3] = 2427;
        t.data[12] = 20;
        t.data[13] = 2428;
        t.data[16] = 480;
        t.data[17] = 1200;
        Meter::from_template(&t).unwrap()
    }

    #[test]
    fn a_tower_moves_with_who_stands_at_it() {
        let mut m = meter();
        assert_eq!(m.slider(), 50);
        // One Alliance player a minute: progress, but contested until a
        // fifth of the way.
        m.advance(1, 0, 60_000);
        assert_eq!(m.phase, Phase::Contested(0));
        assert_eq!(m.progress, 60_000);
        // Enough of them, long enough: the Alliance's.
        m.advance(10, 0, 480_000);
        assert_eq!(m.phase, Phase::Controlled(0));
        assert_eq!(m.slider(), 100);
        // The Horde comes back: still the Alliance's in progress, then
        // theirs.
        m.advance(0, 10, 60_000);
        assert_eq!(m.phase, Phase::Progress(0));
        m.advance(0, 10, 200_000);
        assert_eq!(m.phase, Phase::Controlled(1));
        assert_eq!(m.slider(), 0);
    }
}
