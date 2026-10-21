//! A Scourge Invasion camp around its summoning circle, as vmangos's
//! `NecroticShard` and minion spawner AIs run it (Thistle Tea's
//! `ScourgeInvasion.Camp` and `NecroticCamp`), for as long as the zone's
//! attack event keeps the circle out.
//!
//! The circle raises the camp's Necrotic Shard. Every five seconds it calls
//! one to three minions of the camp's type out of the finders around it,
//! nearest first: a finder that calls one rests two and a half to three and
//! a third minutes, and none calls while a minion stands on it. One minion
//! in 217 is a rare, unless the camp has one. A minion flickers in and
//! roams within a yard of its finder for up to an hour.
//!
//! When the shard breaks a damaged one rises in its place; every hour it is
//! made whole and four Cultist Engineers come to channel into it, each that
//! falls striking it for a hundred. When the damaged shard falls (its bolt
//! to the necropolis is `invasion`'s) the cultists leave, and the circle and
//! its campfires, skull piles and summoner shields go with them.

use std::time::{Duration, Instant};

use ahash::{AHashMap, AHashSet};
use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::invasion::{DAMAGED_SHARD, NECROTIC_SHARD};
use crate::game::map::Map;

/// The summoning circle a camp is run from.
pub const CIRCLE: u32 = 181_136;
/// The Scourge Invasion Minion, finder: where a minion is called.
pub const FINDER: u32 = 16_356;
pub const CULTIST: u32 = 16_230;
/// The camp's objects, gone with it.
const DOODADS: [u32; 8] = [
    181_136, 181_173, 181_174, 181_191, 181_192, 181_193, 181_194, 181_142,
];
const UPKEEP: Duration = Duration::from_secs(5);
const BUTTRESS: Duration = Duration::from_secs(3_600);
const FINDER_REACH: f32 = 60.0;
const OCCUPIED_REACH: f32 = 5.0;
const DOODAD_REACH: f32 = 60.0;
const MINION_LIFETIME: Duration = Duration::from_secs(3_600);
const MINION_WANDER: f32 = 1.0;
const CORPSE: Duration = Duration::from_secs(30);
const REST_MS: std::ops::RangeInclusive<u64> = 150_000..=200_000;
const RARE_ODDS: u32 = 217;
const CULTIST_REACH: (f32, f32) = (6.95, 6.75);
/// mangos's summon kinds: timed or dead, a corpse's time, gone with it.
const TIMED_OR_DEAD: i32 = 1;
const CORPSE_TIMED: i32 = 6;
const DEAD_DESPAWN: i32 = 7;
const MINION_SPAWN_IN: u32 = 28_234;
const CREATE_SUMMONER_SHIELD: u32 = 28_132;
const DAMAGE_CRYSTAL: u32 = 28_041;

/// The camp's two kinds of minion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    GhostGhoul,
    GhostSkeleton,
    GhoulSkeleton,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::GhostGhoul, Kind::GhostSkeleton, Kind::GhoulSkeleton];

    /// Its minions: the Spectral Soldier (16298), the Ghoul Berserker
    /// (16141), the Skeletal Shocktrooper (16299).
    pub fn minions(self) -> [u32; 2] {
        match self {
            Kind::GhostGhoul => [16_298, 16_141],
            Kind::GhostSkeleton => [16_298, 16_299],
            Kind::GhoulSkeleton => [16_141, 16_299],
        }
    }

    /// Its rares: the Spirit of the Damned (16379), the Lumbering Horror
    /// (14697), the Bone Witch (16380).
    pub fn rares(self) -> [u32; 2] {
        match self {
            Kind::GhostGhoul => [16_379, 14_697],
            Kind::GhostSkeleton => [16_379, 16_380],
            Kind::GhoulSkeleton => [14_697, 16_380],
        }
    }
}

fn rare(entry: u32) -> bool {
    Kind::ALL.iter().any(|k| k.rares().contains(&entry))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Shard,
    Damaged,
    Fallen,
}

/// What a death in the camp brings about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fell {
    Shard,
    Camp,
    Cultist,
}

#[derive(Debug, Clone)]
pub struct Camp {
    pub kind: Kind,
    pub stage: Stage,
    /// Its shard, whole or damaged.
    pub shard: Option<Guid>,
    pub minions: AHashSet<Guid>,
    pub rares: AHashSet<Guid>,
    /// Finders resting, till when.
    pub resting: AHashMap<Guid, Instant>,
    pub cultists: AHashSet<Guid>,
    raised: bool,
    /// When it next calls minions, and next buttresses its damaged shard.
    pub upkeep_at: Instant,
    pub buttress_at: Option<Instant>,
}

impl Camp {
    pub fn new(kind: Kind, now: Instant) -> Self {
        Self {
            kind,
            stage: Stage::Shard,
            shard: None,
            minions: AHashSet::new(),
            rares: AHashSet::new(),
            resting: AHashMap::new(),
            cultists: AHashSet::new(),
            raised: false,
            upkeep_at: now + UPKEEP,
            buttress_at: None,
        }
    }

    /// Whether it still calls minions.
    pub fn spawning(&self) -> bool {
        matches!(self.stage, Stage::Shard | Stage::Damaged)
    }

    /// The finders it calls of `candidates` (each with its distance, and
    /// whether a minion stands on it): `count` of the nearest free, rested
    /// each for `rest()` from `now`.
    pub fn call(
        &mut self,
        mut candidates: Vec<(Guid, f32, bool)>,
        now: Instant,
        count: usize,
        mut rest: impl FnMut() -> Duration,
    ) -> Vec<Guid> {
        candidates.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.guid().cmp(&b.0.guid())));
        let called: Vec<Guid> = candidates
            .into_iter()
            .filter(|(g, _, occupied)| !occupied && self.resting.get(g).is_none_or(|t| *t <= now))
            .take(count)
            .map(|(g, ..)| g)
            .collect();
        for g in &called {
            self.resting.insert(*g, now + rest());
        }
        called
    }

    /// The minion it calls: a rare on a roll of 1 in 217 (`rare_roll`)
    /// while it has none, else one of its pair, by `pick` (0 or 1).
    pub fn minion_entry(&self, rare_roll: u32, pick: usize) -> u32 {
        let pair = if self.rares.is_empty() && rare_roll == 1 {
            self.kind.rares()
        } else {
            self.kind.minions()
        };
        pair[pick.min(1)]
    }

    pub fn summoned(&mut self, g: Guid, entry: u32) {
        match entry {
            NECROTIC_SHARD | DAMAGED_SHARD => self.shard = Some(g),
            CULTIST => {
                self.cultists.insert(g);
            }
            _ => {
                self.minions.insert(g);
                if rare(entry) {
                    self.rares.insert(g);
                }
            }
        }
    }

    /// `g` of `entry` died: what it brings about.
    pub fn died(&mut self, g: Guid, entry: u32) -> Option<Fell> {
        match (self.stage, entry) {
            (Stage::Shard, NECROTIC_SHARD) => {
                self.stage = Stage::Damaged;
                self.shard = None;
                Some(Fell::Shard)
            }
            (Stage::Damaged, DAMAGED_SHARD) => {
                self.stage = Stage::Fallen;
                self.shard = None;
                Some(Fell::Camp)
            }
            (stage, CULTIST) => {
                let own = self.cultists.remove(&g);
                (own && stage == Stage::Damaged).then_some(Fell::Cultist)
            }
            _ => {
                self.minions.remove(&g);
                None
            }
        }
    }

    /// `g` is gone: a rare's slot is free once its corpse is.
    pub fn despawned(&mut self, g: Guid) {
        self.minions.remove(&g);
        self.rares.remove(&g);
        self.cultists.remove(&g);
    }

    fn holds(&self, g: Guid) -> bool {
        self.shard == Some(g) || self.minions.contains(&g) || self.cultists.contains(&g)
    }
}

/// Four cultists about a shard at `at` facing `o`, each facing it.
pub fn cultist_positions(at: Vector3d, o: f32) -> [(Vector3d, f32); 4] {
    std::array::from_fn(|i| {
        let angle = i as f32 * std::f32::consts::FRAC_PI_2 + o;
        (
            Vector3d {
                x: at.x + CULTIST_REACH.0 * angle.cos(),
                y: at.y + CULTIST_REACH.1 * angle.sin(),
                z: at.z,
            },
            angle - std::f32::consts::PI,
        )
    })
}

/// An object came into the map: a circle's camp begins.
pub fn added(map: &mut Map, g: Guid, entry: u32) {
    if entry == CIRCLE {
        let kind = Kind::ALL[rand::rng().random_range(0..Kind::ALL.len())];
        map.camps.insert(g, Camp::new(kind, map.now));
    }
}

/// An object leaves the map: its camp, if it ran one, ends.
pub fn removed(map: &mut Map, g: Guid) {
    map.camps.remove(&g);
}

fn summon(
    map: &mut Map,
    circle: Guid,
    entry: u32,
    at: Vector3d,
    o: f32,
    despawn: (Duration, i32),
) -> Option<Guid> {
    let g = crate::game::map::scripts::summon(map, circle, entry, at, o, Some(despawn))?;
    if let Some(c) = map.camps.get_mut(&circle) {
        c.summoned(g, entry);
    }
    Some(g)
}

/// Each camp raises its shard, calls its minions, and buttresses its
/// damaged shard, as their times come.
pub fn update(map: &mut Map) {
    if map.camps.is_empty() {
        return;
    }
    let now = map.now;
    let circles: Vec<Guid> = map.camps.keys().copied().collect();
    for circle in circles {
        let Some(at) = map.gameobject(circle).map(|o| (o.position, o.orientation)) else {
            continue;
        };
        let Some(c) = map.camps.get_mut(&circle) else {
            continue;
        };
        if !c.raised {
            c.raised = true;
            summon(
                map,
                circle,
                NECROTIC_SHARD,
                at.0,
                at.1,
                (Duration::ZERO, DEAD_DESPAWN),
            );
            continue;
        }
        let due = c.spawning() && c.upkeep_at <= now;
        if due {
            c.upkeep_at = now + UPKEEP;
            upkeep(map, circle, at.0);
        }
        if map
            .camps
            .get(&circle)
            .is_some_and(|c| c.stage == Stage::Damaged && c.buttress_at.is_some_and(|t| t <= now))
        {
            buttress(map, circle, at);
        }
    }
}

/// The free finders about the circle at `at`: each with its distance, and
/// whether one of the camp's minions stands on it.
fn finders(map: &Map, circle: Guid, at: Vector3d) -> Vec<(Guid, f32, bool)> {
    let Some(camp) = map.camps.get(&circle) else {
        return Vec::new();
    };
    let minions: Vec<Vector3d> = camp
        .minions
        .iter()
        .filter_map(|g| map.creature(*g).map(|c| c.position))
        .collect();
    map.grid
        .within(at.x, at.y, FINDER_REACH)
        .filter_map(|g| map.creature(g))
        .filter(|c| c.entry == FINDER && c.alive())
        .filter_map(|c| {
            let d = crate::game::map::combat::dist(c.position, at);
            (d <= FINDER_REACH).then(|| {
                let occupied = minions
                    .iter()
                    .any(|m| crate::game::map::combat::dist(*m, c.position) <= OCCUPIED_REACH);
                (c.guid, d, occupied)
            })
        })
        .collect()
}

fn upkeep(map: &mut Map, circle: Guid, at: Vector3d) {
    let candidates = finders(map, circle, at);
    let now = map.now;
    let mut rng = rand::rng();
    let count = rng.random_range(1..=3);
    let Some(camp) = map.camps.get_mut(&circle) else {
        return;
    };
    let called = camp.call(candidates, now, count, || {
        Duration::from_millis(rand::rng().random_range(REST_MS))
    });
    for finder in called {
        let Some(home) = map.creature(finder).map(|c| c.position) else {
            continue;
        };
        let Some(entry) = map
            .camps
            .get(&circle)
            .map(|c| c.minion_entry(rng.random_range(1..=RARE_ODDS), rng.random_range(0..2)))
        else {
            return;
        };
        let o = rng.random_range(0.0..std::f32::consts::TAU);
        let Some(g) = summon(
            map,
            circle,
            entry,
            home,
            o,
            (MINION_LIFETIME, TIMED_OR_DEAD),
        ) else {
            continue;
        };
        // It roams within a yard of its finder.
        if let Some(c) = map.creature_mut(g) {
            c.home = home;
            c.movement_type = crate::game::map::creature::MOVEMENT_RANDOM;
            c.wander_distance = MINION_WANDER;
            c.motion = crate::game::map::creature::Motion::Idle { until: Some(now) };
        }
        crate::game::map::spells::cast_triggered(map, g, MINION_SPAWN_IN, Some(g));
    }
}

/// The damaged shard made whole, and four cultists to channel into it.
fn buttress(map: &mut Map, circle: Guid, at: (Vector3d, f32)) {
    let now = map.now;
    let Some(shard) = map.camps.get_mut(&circle).and_then(|c| {
        c.buttress_at = Some(now + BUTTRESS);
        c.shard
    }) else {
        return;
    };
    if let Some(f) = map.unit_fields_mut(shard) {
        let max = f.get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH);
        f.set_u32(crate::game::fields::UNIT_FIELD_HEALTH, max);
    }
    for (p, o) in cultist_positions(at.0, at.1) {
        if let Some(g) = summon(map, circle, CULTIST, p, o, (BUTTRESS, TIMED_OR_DEAD)) {
            crate::game::map::spells::cast_triggered(map, g, CREATE_SUMMONER_SHIELD, Some(g));
            crate::game::map::spells::cast_triggered(map, g, MINION_SPAWN_IN, Some(g));
        }
    }
}

/// `g` of `entry` died: what it brings about in its camp. Whether it was
/// a camp's whole shard, whose damaged one the camp raises.
pub fn died(map: &mut Map, g: Guid, entry: u32) -> bool {
    let Some((circle, fell)) = map
        .camps
        .iter_mut()
        .find(|(_, c)| c.holds(g))
        .map(|(circle, c)| (*circle, c.died(g, entry)))
    else {
        return false;
    };
    let Some(at) = map.gameobject(circle).map(|o| (o.position, o.orientation)) else {
        return fell == Some(Fell::Shard);
    };
    match fell {
        Some(Fell::Shard) => {
            summon(
                map,
                circle,
                DAMAGED_SHARD,
                at.0,
                at.1,
                (CORPSE, CORPSE_TIMED),
            );
            buttress(map, circle, at);
            true
        }
        Some(Fell::Cultist) => {
            if let Some(shard) = map.camps.get(&circle).and_then(|c| c.shard) {
                crate::game::map::spells::cast_triggered(map, shard, DAMAGE_CRYSTAL, Some(shard));
            }
            false
        }
        Some(Fell::Camp) => {
            let cultists: Vec<Guid> = map
                .camps
                .get(&circle)
                .map(|c| c.cultists.iter().copied().collect())
                .unwrap_or_default();
            for c in cultists {
                crate::game::map::scripts::despawn(map, c);
            }
            let doodads: Vec<Guid> = map
                .grid
                .within(at.0.x, at.0.y, DOODAD_REACH)
                .filter(|o| *o != circle)
                .filter(|o| {
                    map.gameobject(*o)
                        .is_some_and(|x| DOODADS.contains(&x.entry))
                })
                .collect();
            // Hidden until the map starts again (the event's end takes
            // them for good).
            for o in doodads {
                crate::game::map::objects::despawn_then(map, o, None);
            }
            crate::game::map::objects::despawn_then(map, circle, None);
            false
        }
        None => false,
    }
}

/// The circle `summoner`'s summon `g` went away.
pub fn despawned(map: &mut Map, summoner: Guid, g: Guid) {
    if let Some(c) = map.camps.get_mut(&summoner) {
        c.despawned(g);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(n: u32) -> Guid {
        crate::game::guid::unit(1, n)
    }

    fn camp() -> Camp {
        Camp::new(Kind::GhostSkeleton, Instant::now())
    }

    #[test]
    fn it_calls_the_nearest_free_finders_and_rests_them_and_skips_those_resting() {
        let now = Instant::now();
        let mut c = camp();
        let rest = Duration::from_millis(150_000);
        let called = c.call(
            vec![
                (g(1), 30.0, false),
                (g(2), 10.0, false),
                (g(3), 5.0, true),
                (g(4), 20.0, false),
            ],
            now,
            2,
            || rest,
        );
        assert_eq!(called, vec![g(2), g(4)]);
        assert_eq!(c.resting.len(), 2);
        assert_eq!(
            (c.resting[&g(2)], c.resting[&g(4)]),
            (now + rest, now + rest)
        );
        let mut c = camp();
        c.resting.insert(g(2), now + Duration::from_secs(4));
        let called = c.call(
            vec![(g(1), 30.0, false), (g(2), 10.0, false)],
            now,
            3,
            || rest,
        );
        assert_eq!(called, vec![g(1)]);
    }

    #[test]
    fn its_minions_are_its_kinds_pair_and_a_rare_only_while_it_has_none() {
        let mut c = camp();
        assert_eq!(c.kind, Kind::GhostSkeleton);
        assert_eq!(c.stage, Stage::Shard);
        assert_eq!(
            (c.minion_entry(2, 0), c.minion_entry(2, 1)),
            (16_298, 16_299)
        );
        assert_eq!(c.minion_entry(1, 0), 16_379);
        c.summoned(g(9), 16_379);
        assert_eq!(c.minion_entry(1, 0), 16_298);
        assert_eq!(Kind::GhostGhoul.minions(), [16_298, 16_141]);
        assert_eq!(Kind::GhoulSkeleton.rares(), [14_697, 16_380]);
        // A rare gone, its slot is free; a minion slain is forgotten.
        c.despawned(g(9));
        assert!(c.rares.is_empty() && c.minions.is_empty());
        c.summoned(g(7), 16_298);
        assert_eq!(c.died(g(7), 16_298), None);
        assert!(c.minions.is_empty());
    }

    #[test]
    fn its_shard_breaks_then_the_damaged_one_loses_it_and_only_its_cultists_strike() {
        let mut c = camp();
        c.summoned(g(1), NECROTIC_SHARD);
        assert_eq!(c.died(g(1), NECROTIC_SHARD), Some(Fell::Shard));
        assert_eq!((c.stage, c.shard), (Stage::Damaged, None));
        assert!(c.spawning());
        c.summoned(g(2), DAMAGED_SHARD);
        assert_eq!(c.shard, Some(g(2)));
        c.summoned(g(5), CULTIST);
        assert_eq!(c.died(g(5), CULTIST), Some(Fell::Cultist));
        assert!(c.cultists.is_empty());
        assert_eq!(c.died(g(6), CULTIST), None);
        assert_eq!(c.died(g(2), DAMAGED_SHARD), Some(Fell::Camp));
        assert_eq!(c.stage, Stage::Fallen);
        assert!(!c.spawning());
    }

    #[test]
    fn four_cultists_ring_the_shard_facing_it() {
        let p = cultist_positions(
            Vector3d {
                x: 100.0,
                y: 200.0,
                z: 5.0,
            },
            0.0,
        );
        assert_eq!(p.len(), 4);
        let (at, o) = p[0];
        assert!((at.x - 106.95).abs() < 0.001 && (at.y - 200.0).abs() < 0.001);
        assert_eq!(at.z, 5.0);
        assert!((o + std::f32::consts::PI).abs() < 0.001);
        assert!((p[1].0.y - 206.75).abs() < 0.001);
    }
}
