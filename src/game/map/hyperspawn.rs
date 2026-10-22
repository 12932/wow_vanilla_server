//! Hyperspawn (`[hyperspawn]`): in the races' starting areas, the faster
//! creatures die in a pocket of the map, the faster they come back, and so
//! do the quest objects there, by the rate they are used at.
//!
//! A pocket is a square of the map `pocket_yards` across (the grid's 50 by
//! default), keyed by where a spawn stands. It keeps a score that each kill
//! adds one to and that decays exponentially over `window_secs`, worked out
//! when it is next touched: a death costs a hash lookup, and nothing is
//! swept. The score over the window is its kills a minute; over the
//! pocket's spawns (counted once per map, of those hyperspawn would speed
//! up), its kills a minute per spawn: ten kills a minute mean more to a
//! pocket of three than to one of thirty. The factor a respawn is divided
//! by is 1 at or under `quiet_rate`, and rises by `gain` for each kill a
//! minute per spawn over it, to `max_factor`; the respawn is never under
//! `floor_secs`, and never longer than its own. It is fixed at the death: a
//! creature already waiting keeps its time.
//!
//! A hastened creature's corpse holds its respawn back while it has loot
//! or a skin on it, for `loot_wait_secs` at most: a two-second respawn
//! mustn't take a kill's loot from under its looter (wvs respawns a
//! creature in its corpse's place, so a respawn ends the corpse).

use std::time::{Duration, Instant};

use ahash::{AHashMap, AHashSet};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::config::HyperspawnConfig;
use crate::game::data::{CreatureTemplate, GameData, GameObjectTemplate};
use crate::game::fields::{UNIT_DYNAMIC_FLAGS, UNIT_FIELD_FLAGS};
use crate::game::map::Map;
use crate::game::map::gameobject::GameObject;

/// A pocket's square on its map.
pub type PocketKey = (i32, i32);

const NPC_FLAG_QUESTGIVER: u32 = crate::game::map::npc::NPC_FLAG_QUESTGIVER;
const GO_CHEST: u8 = 3;
const GO_GOOBER: u8 = 10;
/// A factor at which a pocket counts as hot, for the stats line.
const HOT: f32 = 2.0;

/// A map's hyperspawn state.
#[derive(Debug, Default)]
pub struct Hyperspawn {
    creatures: AHashMap<PocketKey, Pocket>,
    objects: AHashMap<PocketKey, Pocket>,
    /// The spawns each pocket has, worked out at its map's first death in
    /// a hyperspawn area.
    census: Option<Census>,
    /// The area of each spawn's place, once the navmesh has given it.
    areas: AHashMap<(Kind, u32), u32>,
    /// Creatures whose respawn hyperspawn brought forward, until they
    /// respawn: their corpses hold it back while they have loot on them.
    hastened: AHashSet<Guid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Creature,
    Object,
}

/// A pocket's kills (or uses), decaying.
#[derive(Debug, Default, Clone, Copy)]
struct Pocket {
    score: f64,
    at: Option<Instant>,
}

impl Pocket {
    /// Its score at `now`, decayed over `window` seconds since it was last
    /// touched.
    fn score_at(&self, now: Instant, window: f64) -> f64 {
        self.at.map_or(0.0, |at| {
            let dt = now.saturating_duration_since(at).as_secs_f64();
            self.score * (-dt / window).exp()
        })
    }

    fn add(&mut self, now: Instant, window: f64) {
        self.score = self.score_at(now, window) + 1.0;
        self.at = Some(now);
    }

    /// Kills a minute, as its score has them.
    fn per_minute(&self, now: Instant, window: f64) -> f64 {
        self.score_at(now, window) * 60.0 / window
    }
}

#[derive(Debug)]
struct Census {
    yards: f32,
    creatures: AHashMap<PocketKey, u32>,
    objects: AHashMap<PocketKey, u32>,
    /// Templates with one spawn on the map: named creatures.
    named: AHashSet<u32>,
}

fn pocket_of(x: f32, y: f32, yards: f32) -> PocketKey {
    let v = |c: f32| {
        if c.is_finite() {
            (c.clamp(-20_000.0, 20_000.0) / yards).floor() as i32
        } else {
            0
        }
    };
    (v(x), v(y))
}

/// The factor a respawn is divided by, at `per_spawn` kills a minute per
/// spawn of its pocket.
pub fn factor(per_spawn: f64, cfg: &HyperspawnConfig) -> f32 {
    let over = (per_spawn - f64::from(cfg.quiet_rate)).max(0.0);
    let f = 1.0 + over * f64::from(cfg.gain);
    (f as f32).clamp(1.0, cfg.max_factor.max(1.0))
}

/// `base` divided by `factor`, no less than the floor, no more than itself.
pub fn hastened(base: Duration, factor: f32, cfg: &HyperspawnConfig) -> Duration {
    let floor = Duration::from_secs_f32(cfg.floor_secs.max(0.0));
    base.div_f32(factor.max(1.0)).max(floor).min(base)
}

/// A creature hyperspawn may speed up, by its template: a normal one
/// (not elite, rare or a boss), no quest giver, no script's.
fn creature_eligible(t: &CreatureTemplate) -> bool {
    t.rank == 0 && t.npc_flags & NPC_FLAG_QUESTGIVER == 0 && t.script_name.is_empty()
}

/// A quest object: a chest with a quest's item in its loot, or a goober a
/// quest uses.
fn quest_object(data: &GameData, t: &GameObjectTemplate) -> bool {
    match t.kind {
        GO_CHEST => data.loot.gameobject.get(&t.data[1]).is_some_and(|l| {
            l.iter().any(|e| {
                if e.min < 0 {
                    data.loot
                        .reference
                        .get(&e.item)
                        .is_some_and(|r| r.iter().any(|x| x.chance < 0.0 && x.min >= 0))
                } else {
                    e.chance < 0.0
                }
            })
        }),
        GO_GOOBER => t.data[1] != 0 && t.data[1] as i32 != -1,
        _ => false,
    }
}

/// The map's census, worked out if it isn't, or its pockets' size changed.
fn census(map: &mut Map, yards: f32) -> &Census {
    if map
        .hyperspawn
        .census
        .as_ref()
        .is_none_or(|c| c.yards != yards)
    {
        let data = map.realm.data.clone();
        let id = map.key.map;
        let mut per_entry: AHashMap<u32, u32> = AHashMap::new();
        let spawns = data
            .creature_spawns
            .get(&id)
            .map_or(&[][..], |v| v.as_slice());
        for s in spawns {
            for e in &s.entries {
                *per_entry.entry(*e).or_default() += 1;
            }
        }
        let named: AHashSet<u32> = per_entry
            .iter()
            .filter(|(_, n)| **n == 1)
            .map(|(e, _)| *e)
            .collect();
        let mut creatures: AHashMap<PocketKey, u32> = AHashMap::new();
        for s in spawns {
            let counts = s.entries.first().is_some_and(|e| {
                !named.contains(e) && data.creature_template(*e).is_some_and(creature_eligible)
            });
            if counts {
                *creatures.entry(pocket_of(s.x, s.y, yards)).or_default() += 1;
            }
        }
        let mut objects: AHashMap<PocketKey, u32> = AHashMap::new();
        for s in data.gameobject_spawns.get(&id).into_iter().flatten() {
            let counts = !s.despawned
                && data
                    .gameobject_templates
                    .get(&s.entry)
                    .is_some_and(|t| quest_object(&data, t));
            if counts {
                *objects.entry(pocket_of(s.x, s.y, yards)).or_default() += 1;
            }
        }
        map.hyperspawn.census = Some(Census {
            yards,
            creatures,
            objects,
            named,
        });
    }
    map.hyperspawn.census.as_ref().expect("just worked out")
}

/// The area of a spawn's place: the navmesh's (kept once given), else what
/// the map knows of the object's.
fn area_of(
    map: &mut Map,
    kind: Kind,
    spawn: u32,
    at: Vector3d,
    known: Option<(u32, u32)>,
) -> Option<u32> {
    if let Some(a) = map.hyperspawn.areas.get(&(kind, spawn)) {
        return Some(*a);
    }
    if let Some((_, a)) = crate::game::map::world::zone_and_area(map, at) {
        map.hyperspawn.areas.insert((kind, spawn), a);
        return Some(a);
    }
    known.map(|(_, a)| a)
}

/// Whether hyperspawn is on in this map at all.
fn on(map: &Map) -> bool {
    let cfg = &map.cfg.hyperspawn;
    cfg.enabled && !cfg.areas.is_empty() && map.key.instance == 0
}

/// A creature died, its respawn `base` from now and its corpse to lie
/// `corpse`: both as hyperspawn has them. Its kill counts in its pocket if
/// it is one hyperspawn speeds up, in one of its areas.
pub fn creature_died(
    map: &mut Map,
    g: Guid,
    base: Duration,
    corpse: Duration,
) -> (Duration, Duration) {
    if !on(map) {
        return (base, corpse);
    }
    let Some(c) = map.creature(g) else {
        return (base, corpse);
    };
    let summoned = c.summoned_at.is_some()
        || c.owner.is_some()
        || c.pet.is_some()
        || c.scripted.as_deref().is_some_and(|s| s.summoner.is_some());
    let Some(spawn) = c.spawn_guid.filter(|_| !summoned) else {
        return (base, corpse);
    };
    let (entry, home, known) = (c.entry, c.home, c.zone_area);
    let cfg = map.cfg.clone();
    let h = &cfg.hyperspawn;
    if !map
        .realm
        .data
        .creature_template(entry)
        .is_some_and(creature_eligible)
    {
        return (base, corpse);
    }
    if !area_of(map, Kind::Creature, spawn, home, known).is_some_and(|a| h.areas.contains(&a)) {
        return (base, corpse);
    }
    let key = pocket_of(home.x, home.y, h.pocket_yards);
    let census = census(map, h.pocket_yards);
    if census.named.contains(&entry) {
        return (base, corpse);
    }
    let spawns = census.creatures.get(&key).copied().unwrap_or(0).max(1);
    let (now, window) = (map.now, h.window_secs.max(1) as f64);
    let pocket = map.hyperspawn.creatures.entry(key).or_default();
    pocket.add(now, window);
    let f = factor(pocket.per_minute(now, window) / f64::from(spawns), h);
    let respawn = hastened(base, f, h);
    if respawn >= base {
        return (base, corpse);
    }
    map.hyperspawn.hastened.insert(g);
    let wait = Duration::from_secs(h.loot_wait_secs);
    (respawn, corpse.min(respawn.max(wait)))
}

/// Whether a creature whose respawn is due comes back now: one hyperspawn
/// hastened waits while its corpse lies with loot or a skin on it, or
/// someone looking in it.
pub fn respawn_due(map: &mut Map, g: Guid, corpse_until: Instant) -> bool {
    if !map.hyperspawn.hastened.contains(&g) {
        return true;
    }
    let waits = corpse_until > map.now
        && map.creature(g).is_some_and(|c| {
            !c.despawned
                && (c.fields.get_u32(UNIT_DYNAMIC_FLAGS) & crate::game::map::loot::DYNFLAG_LOOTABLE
                    != 0
                    || c.fields.get_u32(UNIT_FIELD_FLAGS)
                        & crate::game::map::professions::UNIT_FLAG_SKINNABLE
                        != 0
                    || c.loot.as_ref().is_some_and(|l| !l.looters.is_empty()))
        });
    if !waits {
        map.hyperspawn.hastened.remove(&g);
    }
    !waits
}

/// Where a used-up object was, for [`object_used`].
#[derive(Debug, Clone, Copy)]
pub struct Spot {
    pub entry: u32,
    pub spawn: Option<u32>,
    pub at: Vector3d,
    pub zone_area: Option<(u32, u32)>,
}

impl Spot {
    pub fn of(o: &GameObject) -> Self {
        Self {
            entry: o.entry,
            spawn: o.spawn_guid,
            at: o.position,
            zone_area: o.zone_area,
        }
    }
}

/// A spawned object was used up (looted, or used), to come back after
/// `base`: as hyperspawn has it, a quest object in one of its areas
/// counting in its pocket.
pub fn object_used(map: &mut Map, spot: Spot, base: Duration) -> Duration {
    if !on(map) || !map.cfg.hyperspawn.objects {
        return base;
    }
    let Some(spawn) = spot.spawn else {
        return base;
    };
    let data = map.realm.data.clone();
    if !data
        .gameobject_templates
        .get(&spot.entry)
        .is_some_and(|t| quest_object(&data, t))
    {
        return base;
    }
    let cfg = map.cfg.clone();
    let h = &cfg.hyperspawn;
    if !area_of(map, Kind::Object, spawn, spot.at, spot.zone_area)
        .is_some_and(|a| h.areas.contains(&a))
    {
        return base;
    }
    let key = pocket_of(spot.at.x, spot.at.y, h.pocket_yards);
    let spawns = census(map, h.pocket_yards)
        .objects
        .get(&key)
        .copied()
        .unwrap_or(0)
        .max(1);
    let (now, window) = (map.now, h.window_secs.max(1) as f64);
    let pocket = map.hyperspawn.objects.entry(key).or_default();
    pocket.add(now, window);
    let f = factor(pocket.per_minute(now, window) / f64::from(spawns), h);
    hastened(base, f, h)
}

/// A pocket's numbers where someone stands, for `.hyperspawn`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    pub spawns: u32,
    pub per_minute: f64,
    pub factor: f32,
}

/// The creatures' and the quest objects' pockets at `at`.
pub fn reading(map: &mut Map, at: Vector3d) -> (PocketKey, Reading, Reading) {
    let cfg = map.cfg.clone();
    let h = &cfg.hyperspawn;
    let key = pocket_of(at.x, at.y, h.pocket_yards);
    let census = census(map, h.pocket_yards);
    let counts = (
        census.creatures.get(&key).copied().unwrap_or(0),
        census.objects.get(&key).copied().unwrap_or(0),
    );
    let (now, window) = (map.now, h.window_secs.max(1) as f64);
    let read = |p: Option<&Pocket>, spawns: u32| {
        let per_minute = p.map_or(0.0, |p| p.per_minute(now, window));
        Reading {
            spawns,
            per_minute,
            factor: factor(per_minute / f64::from(spawns.max(1)), h),
        }
    };
    (
        key,
        read(map.hyperspawn.creatures.get(&key), counts.0),
        read(map.hyperspawn.objects.get(&key), counts.1),
    )
}

/// `.hyperspawn`: the pocket where `g` stands.
pub fn report(map: &mut Map, g: Guid) -> Option<String> {
    let at = map.players.get(&g)?.position();
    let cfg = map.cfg.clone();
    let h = &cfg.hyperspawn;
    let area = crate::game::map::world::zone_and_area(map, at)
        .map(|(_, a)| a)
        .or_else(|| map.players.get(&g).map(|p| p.area))
        .unwrap_or(0);
    let name = map
        .realm
        .data
        .area(area)
        .map_or_else(|| "unknown".to_owned(), |a| a.name.clone());
    let state = if !h.enabled {
        "hyperspawn is off"
    } else if map.key.instance != 0 || !h.areas.contains(&area) {
        "not a hyperspawn area"
    } else {
        "a hyperspawn area"
    };
    let (key, c, o) = reading(map, at);
    let five = |r: &Reading| hastened(Duration::from_secs(300), r.factor, h).as_secs_f32();
    Some(format!(
        "Area {area} ({name}): {state}. Pocket ({}, {}) of {:.0} yards: creatures {} spawns, \
         {:.1} kills a minute ({:.2} a spawn), factor {:.1}, a 5-minute respawn in {:.1} s; \
         quest objects {} spawns, {:.1} uses a minute, factor {:.1}{}.",
        key.0,
        key.1,
        h.pocket_yards,
        c.spawns,
        c.per_minute,
        c.per_minute / f64::from(c.spawns.max(1)),
        c.factor,
        five(&c),
        o.spawns,
        o.per_minute,
        o.factor,
        if h.objects { "" } else { " (off)" },
    ))
}

/// The map's pockets whose factor is 2 or more now, for `WVS_STATS_FILE`.
pub fn hot_pockets(map: &Map) -> usize {
    let Some(census) = map.hyperspawn.census.as_ref() else {
        return 0;
    };
    let h = &map.cfg.hyperspawn;
    let (now, window) = (map.now, h.window_secs.max(1) as f64);
    let hot = |pockets: &AHashMap<PocketKey, Pocket>, counts: &AHashMap<PocketKey, u32>| {
        pockets
            .iter()
            .filter(|(k, p)| {
                let spawns = counts.get(k).copied().unwrap_or(0).max(1);
                factor(p.per_minute(now, window) / f64::from(spawns), h) >= HOT
            })
            .count()
    };
    hot(&map.hyperspawn.creatures, &census.creatures)
        + hot(&map.hyperspawn.objects, &census.objects)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curve_is_one_when_quiet_and_capped_at_its_most() {
        let cfg = HyperspawnConfig::default();
        assert_eq!(factor(0.0, &cfg), 1.0);
        assert_eq!(factor(f64::from(cfg.quiet_rate), &cfg), 1.0);
        assert!((factor(1.25, &cfg) - 43.0).abs() < 1e-3);
        assert_eq!(factor(100.0, &cfg), cfg.max_factor);
        let five = Duration::from_secs(300);
        assert_eq!(hastened(five, 1.0, &cfg), five);
        assert_eq!(hastened(five, 150.0, &cfg), Duration::from_secs(2));
        // Never longer than its own, under the floor as it is.
        assert_eq!(
            hastened(Duration::from_secs(1), 150.0, &cfg),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn a_pocket_decays_over_its_window() {
        let now = Instant::now();
        let mut p = Pocket::default();
        for _ in 0..10 {
            p.add(now, 120.0);
        }
        assert!((p.per_minute(now, 120.0) - 5.0).abs() < 1e-9);
        let later = now + Duration::from_secs(120);
        assert!((p.score_at(later, 120.0) - 10.0 / std::f64::consts::E).abs() < 1e-9);
    }
}
