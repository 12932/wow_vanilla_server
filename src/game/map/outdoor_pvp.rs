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
            min_time: if t.data[16] > 0 {
                i64::from(t.data[16])
            } else {
                60
            },
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
    /// The banners by it, in its holder's colours.
    pub banners: Vec<Guid>,
}

/// The banners by each tower, in `TOWERS` order.
const BANNERS: [[(f32, f32, f32, f32); 2]; 4] = [
    [
        (1838.42, -3703.56, 167.713, 0.890117),
        (1877.6, -3716.76, 167.188, 1.74533),
    ],
    [
        (2539.61, -4801.55, 115.766, 2.00713),
        (2569.6, -4772.93, 115.399, 2.72271),
    ],
    [
        (3148.17, -4365.51, 145.029, 1.53589),
        (3188.76, -4358.5, 144.555, 1.97222),
    ],
    [
        (2975.5, -3060.36, 125.108, 5.23599),
        (2992.63, -3022.95, 125.593, 3.03684),
    ],
];
const TOWER_BANNER: u32 = 182106;

/// A tower's colours (art kit) and its banners' animation, by holder.
fn art_kit(owner: Option<usize>) -> (u32, u32) {
    match owner {
        None => (21, 2),
        Some(0) => (2, 1),
        Some(_) => (1, 0),
    }
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
/// How long silithyst lies where it fell.
pub const MOUND_LASTS: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, Default)]
pub struct Silithus {
    pub brought: [u32; 2],
    /// The side with the Cenarion Favor.
    pub controller: Option<usize>,
    /// Those carrying silithyst, and where they were last seen: it falls
    /// where its carrier lost it.
    pub carriers: AHashMap<Guid, Vector3d>,
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
    /// What the towers held put up: by tower and what it is.
    pub services: AHashMap<(usize, u8), (u32, Guid)>,
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
        MapKey {
            map: 0,
            instance: 0,
        } => Some(Outdoor::default()),
        MapKey {
            map: 1,
            instance: 0,
        } => Some(Outdoor {
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
        && !p
            .fields
            .has_flag(PLAYER_FLAGS, crate::game::map::player::PLAYER_FLAGS_GM)
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
    reconcile_buffs(map);
}

/// A zone's outdoor world states, for `SMSG_INIT_WORLD_STATES`: Silithus's
/// tallies, the Plaguelands' towers (and an idle slider).
pub fn zone_states(map: &Map, zone: u32) -> Vec<(u32, u32)> {
    let Some(o) = map.outdoor.as_ref() else {
        return Vec::new();
    };
    match (map.key.map, zone) {
        (1, SILITHUS) => o.silithus.as_ref().map(silithus_states).unwrap_or_default(),
        (0, PLAGUELANDS) => {
            let mut s = o.tower_states();
            s.extend([(2426, 0), (2428, 20), (2427, 50)]);
            s
        }
        _ => Vec::new(),
    }
}

/// Each second: the buffs world PvP gives, on those who should have them
/// and off everyone else (Thistle Tea's `OutdoorPvp.reconcile`): the
/// towers' in the Plaguelands' zones, by how many the side holds; the
/// Cenarion Favor in Silithus's, for the side that won it; neither for the
/// dead.
fn reconcile_buffs(map: &mut Map) {
    let Some(o) = map.outdoor.as_ref() else {
        return;
    };
    let counts = o.counts();
    let controller = o.silithus.as_ref().and_then(|si| si.controller);
    let on = map.key.map;
    let wanted: Vec<(Guid, [Option<u32>; 2])> = map
        .players
        .values()
        .map(|p| {
            let s = side(p.team);
            let alive = p.death == Death::Alive;
            let tower = (alive && on == 0 && PLAGUELANDS_BUFF_ZONES.contains(&p.zone))
                .then(|| {
                    (1..=4)
                        .contains(&counts[s])
                        .then(|| TOWER_BUFFS[s][counts[s] - 1])
                })
                .flatten();
            let favor = (alive
                && on == 1
                && SILITHUS_BUFF_ZONES.contains(&p.zone)
                && controller == Some(s))
            .then_some(SPELL_FAVOR);
            (p.guid, [tower, favor])
        })
        .collect();
    for (g, want) in wanted {
        let managed = TOWER_BUFFS.iter().flatten().copied().chain([SPELL_FAVOR]);
        for b in managed {
            let has = map.players.get(&g).is_some_and(|p| p.auras.has_spell(b));
            let keep = want.contains(&Some(b));
            if has && !keep {
                crate::game::map::auras::remove_spell(map, g, b, None);
            } else if keep && !has {
                crate::game::map::spells::cast_triggered(map, g, b, Some(g));
            }
        }
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
        go.fields.set_u32(GAMEOBJECT_ARTKIT, art_kit(None).0);
        let g = map.add_gameobject(go);
        let banners = BANNERS[i]
            .iter()
            .filter_map(|at| place_object(map, TOWER_BANNER, *at))
            .collect::<Vec<_>>();
        for b in &banners {
            if let Some(o) = map.gameobject_mut(*b) {
                o.fields.set_u32(GAMEOBJECT_ARTKIT, art_kit(None).0);
            }
        }
        towers.push(Tower {
            meter,
            object: Some(g),
            banners,
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
            (
                p.guid,
                side(p.team),
                (pos.x, pos.y, pos.z),
                eligible(map, p),
            )
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
    // Those come into the zone see its states; those gone out of it, none.
    for g in watching.difference(&before_watching) {
        send_states(map, *g, &after_states);
    }
    let cleared: Vec<(u32, u32)> = after_states
        .iter()
        .map(|(f, _)| (*f, 0))
        .chain([(2426, 0)])
        .collect();
    for g in before_watching.difference(&watching) {
        send_states(map, *g, &cleared);
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
        colours(map, i, now);
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
        // Heard within 250 yards of the tower, as Thistle Tea plays it.
        if let Some(sound) = sound {
            let at = TOWERS[i].at;
            for p in map.players.values() {
                let pos = p.position();
                if dist(at, (pos.x, pos.y, pos.z)) <= 250.0 {
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
    if before != after {
        reconcile_services(map);
    }
}

/// A tower and its banners take their new holder's colours.
fn colours(map: &mut Map, tower: usize, owner: Option<usize>) {
    let Some(t) = map.outdoor.as_ref().and_then(|o| o.towers.get(tower)) else {
        return;
    };
    let objects: Vec<Guid> = t
        .object
        .into_iter()
        .chain(t.banners.iter().copied())
        .collect();
    let (kit, animation) = art_kit(owner);
    for g in objects {
        if let Some(o) = map.gameobject_mut(g) {
            o.fields.set_u32(GAMEOBJECT_ARTKIT, kit);
        }
        if let Some(frame) = crate::game::conn::encode(&SMSG_GAMEOBJECT_CUSTOM_ANIM {
            guid: g,
            animation_id: animation,
        }) {
            map.send_to_observers(g, &frame, false);
        }
    }
}

/// What a tower's holder gets, by tower (Crown Guard, Eastwall, Northpass,
/// Plaguewood): a service spawn's slot, kind, entry, place, and for a
/// creature its faction and aura.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Service {
    slot: u8,
    creature: bool,
    entry: u32,
    at: (f32, f32, f32, f32),
    faction: u32,
    aura: u32,
}

const SQUAD: [(f32, f32, f32, f32); 5] = [
    (2532.85, -4764.92, 103.617, 2.35619),
    (2533.33, -4769.31, 104.396, 2.37365),
    (2537.34, -4773.92, 105.941, 2.21657),
    (2537.77, -4765.94, 104.432, 2.3911),
    (2542.57, -4770.22, 106.145, 2.42601),
];
/// Each tower's victory flare, the Alliance's and the Horde's.
const FLARES: [[(f32, f32, f32, f32); 2]; 4] = [
    [
        (1855.66, -3725.0, 197.044, 1.53589),
        (1853.12, -3722.62, 197.406, 0.628317),
    ],
    [
        (2563.26, -4795.15, 145.852, 1.81514),
        (2565.27, -4797.59, 147.846, 3.05433),
    ],
    [
        (3171.86, -4377.2, 174.898, 0.174532),
        (3169.76, -4375.13, 175.458, 2.70526),
    ],
    [
        (2971.41, -3038.36, 157.492, 5.35816),
        (2973.17, -3037.19, 156.443, 1.97222),
    ],
];
/// The Crown Guard graveyard (`game_graveyard_zone`), open to its holder.
pub const CROWN_GUARD_GRAVEYARD: u32 = 927;

fn services(tower: usize, phase: Phase) -> Vec<Service> {
    let mut out = Vec::new();
    if let Some(s) = phase.owner() {
        let faction = [774, 775][s];
        match tower {
            0 => {
                out.push(Service {
                    slot: 0,
                    creature: false,
                    entry: [180421, 180422][s],
                    at: (1985.47, -3653.88, 120.172, 1.46608),
                    faction: 0,
                    aura: 0,
                });
                out.push(Service {
                    slot: 1,
                    creature: true,
                    entry: 18039,
                    at: (1856.58, -3714.72, 194.637, 0.762214),
                    faction,
                    aura: [31954, 31951][s],
                });
            }
            1 => {
                for (i, at) in SQUAD.iter().enumerate() {
                    let entry = if i == 0 {
                        [17635, 17995][s]
                    } else {
                        [17647, 17996][s]
                    };
                    out.push(Service {
                        slot: i as u8,
                        creature: true,
                        entry,
                        at: *at,
                        faction: 0,
                        aura: 0,
                    });
                }
            }
            2 => {
                let at = [
                    (3167.72, -4355.91, 138.785, 1.69297),
                    (3167.5, -4356.25, 138.821, 1.69297),
                ][s];
                for (slot, entry) in [(0, [181682, 181955][s]), (1, [180100, 180101][s])] {
                    out.push(Service {
                        slot,
                        creature: false,
                        entry,
                        at,
                        faction: 0,
                        aura: 0,
                    });
                }
            }
            _ => out.push(Service {
                slot: 0,
                creature: true,
                entry: 17209,
                at: (2987.5, -3049.11, 120.126, 5.75959),
                faction,
                aura: [17327, 31309][s],
            }),
        }
    }
    if let Phase::Controlled(s) = phase {
        out.push(Service {
            slot: 9,
            creature: false,
            entry: [181852, 181853][s],
            at: FLARES[tower][s],
            faction: 0,
            aura: 0,
        });
    }
    out
}

/// The towers' services in line with who holds them.
fn reconcile_services(map: &mut Map) {
    let Some(o) = map.outdoor.as_ref() else {
        return;
    };
    let wanted: Vec<(usize, Service)> = o
        .towers
        .iter()
        .enumerate()
        .flat_map(|(i, t)| services(i, t.meter.phase).into_iter().map(move |s| (i, s)))
        .collect();
    let stale: Vec<((usize, u8), Guid)> = o
        .services
        .iter()
        .filter(|((i, slot), (entry, _))| {
            !wanted
                .iter()
                .any(|(wi, s)| wi == i && s.slot == *slot && s.entry == *entry)
        })
        .map(|(k, (_, g))| (*k, *g))
        .collect();
    let new: Vec<(usize, Service)> = wanted
        .into_iter()
        .filter(|(i, s)| {
            o.services
                .get(&(*i, s.slot))
                .is_none_or(|(entry, _)| *entry != s.entry)
        })
        .collect();
    for (k, g) in stale {
        if crate::game::guid::is_unit(g) {
            map.remove_creature(g);
        } else {
            map.remove_gameobject(g);
        }
        if let Some(o) = map.outdoor.as_mut() {
            o.services.remove(&k);
        }
    }
    for (i, s) in new {
        let g = if s.creature {
            summon(map, s.entry, s.at, s.faction)
        } else {
            place_object(map, s.entry, s.at)
        };
        let Some(g) = g else {
            continue;
        };
        if s.aura != 0 {
            crate::game::map::spells::cast_triggered(map, g, s.aura, Some(g));
        }
        if s.entry == SPIRIT_OF_VICTORY {
            walk_waypoints(map, g);
        }
        if let Some(o) = map.outdoor.as_mut() {
            o.services.insert((i, s.slot), (s.entry, g));
        }
        if i == EASTWALL && s.slot > 0 {
            link_soldier(map, g, usize::from(s.slot));
        }
    }
}

const EASTWALL: usize = 1;
const SPIRIT_OF_VICTORY: u32 = 18039;

/// The spirit healer walks its entry's waypoints, a second after it
/// comes, as Thistle Tea starts them.
fn walk_waypoints(map: &mut Map, g: Guid) {
    let now = map.now;
    if let Some(c) = map.creature_mut(g) {
        c.movement_type = crate::game::map::creature::MOVEMENT_WAYPOINT;
        c.waypoint = 0;
        c.motion = crate::game::map::creature::Motion::Idle {
            until: Some(now + std::time::Duration::from_secs(1)),
        };
    }
}

/// An Eastwall soldier joins its commander's squad, at its place beside
/// it: they fight together and give up together.
fn link_soldier(map: &mut Map, g: Guid, slot: usize) {
    let Some(leader) = map
        .outdoor
        .as_ref()
        .and_then(|o| o.services.get(&(EASTWALL, 0)))
        .map(|(_, l)| *l)
    else {
        return;
    };
    let (lx, ly, _, _) = SQUAD[0];
    let (x, y, _, o) = SQUAD[slot];
    let angle = (y - ly).atan2(x - lx) - o;
    // In formation, fighting and evading together.
    crate::game::map::creature_groups::join_with(
        map,
        g,
        leader,
        5.0,
        angle,
        crate::game::data::scripts::GROUP_FORMATION
            | crate::game::data::scripts::GROUP_AGGRO_TOGETHER
            | crate::game::data::scripts::GROUP_EVADE_TOGETHER,
    );
}

/// A creature put in the world by world PvP, of its template (with a
/// faction other than its own if given); it doesn't respawn.
pub fn summon(map: &mut Map, entry: u32, at: (f32, f32, f32, f32), faction: u32) -> Option<Guid> {
    let data = map.realm.data.clone();
    let counter = map.next_creature_counter;
    map.next_creature_counter += 1;
    let spawn = crate::game::data::CreatureSpawn {
        guid: counter | 0x80_0000,
        entries: vec![entry],
        map: map.key.map,
        x: at.0,
        y: at.1,
        z: at.2,
        orientation: at.3,
        ..Default::default()
    };
    let mut c = crate::game::map::creature::Creature::from_spawn(&data, &spawn, &mut rand::rng())?;
    c.spawn_guid = None;
    if faction != 0 {
        c.fields.set_u32(UNIT_FIELD_FACTIONTEMPLATE, faction);
        let civilian = data.creature_template(entry).is_some_and(|t| t.civilian);
        c.hostile_to_players = crate::game::map::ai::hostile_to_players(&data, faction, civilian);
    }
    Some(map.add_creature(c))
}

fn place_object(map: &mut Map, entry: u32, at: (f32, f32, f32, f32)) -> Option<Guid> {
    let data = map.realm.data.clone();
    let t = data.gameobject_templates.get(&entry)?;
    let counter = map.next_creature_counter;
    map.next_creature_counter += 1;
    let mut go = GameObject::from_spawn(
        t,
        &GameObjectSpawn {
            guid: counter | 0x80_0000,
            entry,
            map: map.key.map,
            x: at.0,
            y: at.1,
            z: at.2,
            orientation: at.3,
            state: 1,
            ..Default::default()
        },
    );
    go.spawn_guid = None;
    Some(map.add_gameobject(go))
}

/// Whether a graveyard is open to `faction` (469, 67): the Crown Guard
/// graveyard only to the side holding Crown Guard Tower.
pub fn graveyard_open(map: &Map, graveyard: u32, faction: u32) -> bool {
    if graveyard != CROWN_GUARD_GRAVEYARD {
        return true;
    }
    let owner = map
        .outdoor
        .as_ref()
        .and_then(|o| o.towers.first())
        .and_then(|t| t.meter.phase.owner());
    owner.is_some_and(|s| [469, 67][s] == faction)
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

/// Silithyst, at a geyser or where a carrier lost it, picked up (by
/// `CMSG_GAMEOBJ_USE`, or the client's Opening of it): carried, and the
/// carrier's side's aura with it; the carrier is contested. It's used up:
/// a mound goes, a geyser until it comes back. One carrying can't take
/// more. Returns whether it was silithyst.
pub fn use_object(map: &mut Map, g: Guid, object: Guid) -> bool {
    if map.outdoor.as_ref().is_none_or(|o| o.silithus.is_none()) {
        return false;
    }
    let Some(o) = map.gameobject(object) else {
        return false;
    };
    let (entry, spawn, at) = (o.entry, o.spawn_guid, o.position);
    if !matches!(entry, SILITHYST_MOUND | SILITHYST_GEYSER) {
        return false;
    }
    let Some(p) = map.players.get(&g) else {
        return true;
    };
    let near = crate::game::map::combat::dist(p.position(), at) <= 10.0;
    if !near || p.death != Death::Alive || p.auras.has_spell(SPELL_CARRYING) {
        return true;
    }
    let (s, pos) = (side(p.team), p.position());
    crate::game::map::spells::cast_triggered(map, g, SPELL_CARRYING, Some(g));
    crate::game::map::spells::cast_triggered(map, g, SPELL_TEAM_CARRYING[s], Some(g));
    // Carrying it flags the carrier, and contests them.
    crate::game::map::pvp::contest(map, g);
    if let Some(si) = map.outdoor.as_mut().and_then(|o| o.silithus.as_mut()) {
        si.carriers.insert(g, pos);
    }
    map.remove_gameobject(object);
    if let Some(spawn) = spawn {
        let secs = map
            .realm
            .data
            .gameobject_spawns
            .get(&map.key.map)
            .and_then(|l| l.iter().find(|s| s.guid == spawn))
            .map_or(180, |s| s.respawn_min.max(1));
        let back = map.now + Duration::from_secs(u64::from(secs));
        map.loot_state.respawns.push((back, spawn));
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
    // Home, not fallen: no longer a carrier before the aura goes.
    if let Some(si) = map.outdoor.as_mut().and_then(|o| o.silithus.as_mut()) {
        si.carriers.remove(&g);
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
        reconcile_buffs(map);
    }
}

fn silithus_states(si: &Silithus) -> Vec<(u32, u32)> {
    vec![
        (2313, si.brought[0]),
        (2314, si.brought[1]),
        (2317, SILITHYST_LIMIT),
    ]
}

/// Each second in Kalimdor, behind the aura's interrupts that end it at
/// the moment: carriers who mount, stealth or take a flight lose the
/// silithyst; silithyst lost any way but at the camp (fallen, mounted,
/// cancelled, gone from the map) falls where its carrier was, with their
/// side's aura off them (Thistle Tea's `Silithyst.after_remove`); those
/// who come into Silithus see its tallies, and those who leave it, none.
fn silithus_update(map: &mut Map) {
    use crate::game::map::auras::{AURA_MOD_INVISIBILITY, AURA_MOD_STEALTH};
    let Some(si) = map.outdoor.as_ref().and_then(|o| o.silithus.as_ref()) else {
        return;
    };
    let carriers: Vec<(Guid, Vector3d)> = si.carriers.iter().map(|(g, at)| (*g, *at)).collect();
    for (g, last) in carriers {
        let Some(p) = map.players.get(&g) else {
            lost(map, g, last);
            continue;
        };
        let at = p.position();
        let broken = p.fields.get_u32(UNIT_FIELD_MOUNTDISPLAYID) != 0
            || p.flight.is_some()
            || p.auras.has(AURA_MOD_STEALTH)
            || p.auras.has(AURA_MOD_INVISIBILITY);
        if broken {
            crate::game::map::auras::remove_spell(map, g, SPELL_CARRYING, None);
        }
        if !map
            .players
            .get(&g)
            .is_some_and(|p| p.auras.has_spell(SPELL_CARRYING))
        {
            lost(map, g, at);
        } else if let Some(si) = map.outdoor.as_mut().and_then(|o| o.silithus.as_mut()) {
            si.carriers.insert(g, at);
        }
    }
    let Some(si) = map.outdoor.as_ref().and_then(|o| o.silithus.as_ref()) else {
        return;
    };
    let states = silithus_states(si);
    let watching: AHashSet<Guid> = map
        .players
        .values()
        .filter(|p| p.zone == SILITHUS)
        .map(|p| p.guid)
        .collect();
    let before = map
        .outdoor
        .as_ref()
        .map(|o| o.watching.clone())
        .unwrap_or_default();
    for g in watching.difference(&before) {
        send_states(map, *g, &states);
    }
    let cleared: Vec<(u32, u32)> = states.iter().map(|(f, _)| (*f, 0)).collect();
    for g in before.difference(&watching) {
        send_states(map, *g, &cleared);
    }
    if let Some(o) = map.outdoor.as_mut() {
        o.watching = watching;
    }
}

/// The silithyst's aura came off `g` (mounted, stealthed, flown, dead,
/// cancelled, off the map): it falls where they are, at the moment
/// (Thistle Tea's `Silithyst.after_remove`); the second's check is the
/// backstop.
pub fn carrying_ended(map: &mut Map, g: Guid) {
    if let Some(at) = map.players.get(&g).map(|p| p.position()) {
        lost(map, g, at);
    }
}

/// A carrier lost the silithyst, not at the camp: it falls at `at`, once.
fn lost(map: &mut Map, g: Guid, at: Vector3d) {
    let Some(si) = map.outdoor.as_mut().and_then(|o| o.silithus.as_mut()) else {
        return;
    };
    if si.carriers.remove(&g).is_none() {
        return;
    }
    for a in SPELL_TEAM_CARRYING {
        crate::game::map::auras::remove_spell(map, g, a, None);
    }
    drop_silithyst(map, at);
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
    let g = map.add_gameobject(go);
    // A dropped mound lasts three minutes.
    map.object_resets.push((map.now + MOUND_LASTS, g, u32::MAX));
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
