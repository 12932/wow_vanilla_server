//! Who sees what. A player sees an object within the visibility radius
//! (on a flight master's mount, further: `[visibility] flight_yards`),
//! measured on the ground, and forgets it past the radius plus a margin,
//! so an object at the edge doesn't flicker. Visibility is worked out for an object when it has
//! moved some yards since the last time, or has just come into the map.
//!
//! **The quick path.** An object is *plain* when who sees it, and whom it
//! sees, turns on distance alone ([`plain_now`]): not stealthed or
//! invisible, not a ghost, not concealed by a script, not a spirit healer,
//! not a trap or a transport, not on a flight, not looking from elsewhere,
//! not between maps. Between two plain objects [`death::can_see`] is always
//! true. Each grid entry carries its object's anchor (where its visibility
//! was last worked out) and whether its own last refresh found it plain;
//! one is plain there only while it is within [`DRIFT`] of its anchor
//! ([`moved`] takes the flag off past it, as it does when the object stops
//! being plain), so its anchor is never further than that from where it is.
//!
//! The invariant the quick path keeps (call it K): of two plain objects
//! whose anchors are within `radius - DRIFT` of each other, the player
//! knows the other. Each refresh sets it up again for the object's pairs:
//! one whose anchors were that near stays known (a known pair is parted
//! only past the radius plus the margin, and the two are nearer than the
//! radius), and one whose new anchors are that near is within the radius,
//! the two being within [`DRIFT`] of their anchors, and so is introduced.
//! So when a plain object moves `delta` from its anchor, every plain
//! object whose anchor is within `radius - DRIFT - delta` of where it is
//! now was within `radius - DRIFT` of its old anchor: known, both ways,
//! and staying so. Its refresh skips those without looking them up, and a
//! whole cell of them (no object in it not plain, the cell's farthest
//! corner within `radius - 2 DRIFT - delta`) without looking at it. Those
//! not plain are always looked at, as is every object whose last refresh
//! was a whole one (`Kind::Full`: an arrival, a teleport, a change of
//! state), until its own refresh has set its pairs up again.
//!
//! What the quick path skips it counts as known, so that what a player
//! knows (or who knows an object) outside the window is looked for only
//! when the counts say there is some.
//!
//! **An update's refreshes** are taken oldest first, in chunks: each
//! chunk's side work (the zone, the tiles ahead, on a cell crossed), then
//! its decisions, worked out read-only across the visibility threads
//! (`[visibility] refresh_threads`), then applied in order on the map's
//! thread. A pair two of a chunk's refreshes share is decided by the
//! earlier, both ways. Once the update's refreshes have taken
//! `[visibility] refresh_budget_ms`, the rest wait for the next update,
//! oldest first: a chunk stops after the refresh at hand, and its rest go
//! back to the front of the line (one refresh at least an update). A
//! whole refresh asks [`death::can_see`] only of the pairs not both plain,
//! as the quick path does; among a crowd its looks are split across the
//! threads.
//!
//! **Its creates** for a crowd (an object coming into the sight of many,
//! who see it as a stranger does) go in packets shared among them at the
//! chunk's end, no client held for them ([`Crowds`]); the rest into each
//! observer's batch, the client held until it goes.

use std::collections::VecDeque;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ahash::AHashMap;
use rayon::prelude::*;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::fields::Visibility;
use crate::game::guid;
use crate::game::map::grid::{self, CellKey};
use crate::game::map::player::Player;
use crate::game::map::{Map, death};
use crate::game::update;

/// How far past the radius an object is kept before it is forgotten.
pub const MARGIN: f32 = 10.0;
/// How far an object moves before its visibility is worked out again.
pub const MOVE_THRESHOLD: f32 = 8.0;
/// How far a stealthed unit moves before who sees it is worked out again.
pub const STEALTH_THRESHOLD: f32 = 2.0;
/// How far a plain object may be from its anchor: twice the distance that
/// marks it, so that one marked is taken whole only if it goes on a long
/// way (a charge, a refresh put off by the budget) before its refresh.
pub const DRIFT: f32 = 2.0 * MOVE_THRESHOLD;
/// The refreshes worked out together, and between looks at the budget:
/// at most this many ...
const CHUNK: usize = 128;
/// ... and at least this many, a chunk being as many as what is left of
/// the budget has room for at what a refresh cost in the last (a crowd
/// coming into another's sight introduces thousands of pairs a refresh,
/// 2 to 4 ms each: a chunk of 16 such took 30 to 70 ms) ...
const MIN_CHUNK: usize = 1;
/// ... and at most twice the last chunk's worked out, or this many: one
/// grows into a costlier crowd a chunk at a time, not from one refresh's
/// cost to 128 refreshes' at once.
const GROW_FROM: usize = 4;
/// Fewer in a chunk than this are worked out on the map's thread alone ...
const PARALLEL_FROM: usize = 16;
/// ... unless they are to cost this much at what a refresh cost in the
/// last chunk (a charge's: a millisecond each among 1,600), when two or
/// more are worked out across the threads.
const PARALLEL_COST: Duration = Duration::from_micros(400);

/// Why an object's visibility is to be worked out.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Kind {
    /// It moved: the quick path may skip what moving can't change.
    Moved,
    /// Anything else (it arrived, was teleported, its state changed): every
    /// pair it is in is looked at.
    Full,
}

/// The objects whose visibility is to be worked out, oldest first, each
/// once, with why.
#[derive(Debug, Default)]
pub struct Dirty {
    order: VecDeque<(Guid, u64)>,
    marked: AHashMap<Guid, (Kind, u64)>,
    seq: u64,
    /// The radius the grid's plain flags were set for: at another, none is
    /// trusted.
    radius: f32,
    /// Work each out alone and whole, as before the quick path: what the
    /// tests and the benchmark measure it against.
    #[doc(hidden)]
    pub one_by_one: bool,
    /// Every create into its observer's batch, none shared among a crowd
    /// ([`Crowds`]): what the tests compare the shared ones with.
    #[doc(hidden)]
    pub creates_one_by_one: bool,
    /// What a refresh cost in the last chunk worked out, and how many were:
    /// the next chunk's size, against the budget.
    per_refresh: Duration,
    last_worked: usize,
    /// Since the map began: the pairs the quick path skipped without a
    /// look, and those looked at; and the creates shared among a crowd
    /// ([`Crowds`]).
    pub skipped: u64,
    pub looked: u64,
    pub shared_creates: u64,
}

impl Dirty {
    /// Mark `g` to be worked out whole. False if it was marked already.
    pub fn insert(&mut self, g: Guid) -> bool {
        self.mark(g, Kind::Full)
    }

    /// Mark `g` as moved (one marked whole stays so).
    pub fn moved(&mut self, g: Guid) {
        self.mark(g, Kind::Moved);
    }

    fn mark(&mut self, g: Guid, kind: Kind) -> bool {
        if let Some((k, _)) = self.marked.get_mut(&g) {
            *k = (*k).max(kind);
            return false;
        }
        self.seq += 1;
        self.marked.insert(g, (kind, self.seq));
        self.order.push_back((g, self.seq));
        true
    }

    pub fn remove(&mut self, g: &Guid) -> bool {
        self.marked.remove(g).is_some()
    }

    pub fn contains(&self, g: &Guid) -> bool {
        self.marked.contains_key(g)
    }

    pub fn kind(&self, g: &Guid) -> Option<Kind> {
        self.marked.get(g).map(|&(k, _)| k)
    }

    pub fn clear(&mut self) {
        self.order.clear();
        self.marked.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.marked.is_empty()
    }

    pub fn len(&self) -> usize {
        self.marked.len()
    }

    /// Refreshes taken and not worked out (the budget ran out in their
    /// chunk), back at the front in their order, as they were: before
    /// anything marked since. One marked again meanwhile keeps the later
    /// mark, and the greater kind.
    fn put_back(&mut self, back: &[(Guid, Kind, u64)]) {
        for &(g, kind, seq) in back.iter().rev() {
            if let Some((k, _)) = self.marked.get_mut(&g) {
                *k = (*k).max(kind);
                continue;
            }
            self.marked.insert(g, (kind, seq));
            self.order.push_front((g, seq));
        }
    }

    /// The oldest `n` marked by `last`, taken off.
    fn take(&mut self, n: usize, last: u64) -> Vec<(Guid, Kind, u64)> {
        let mut out = Vec::with_capacity(n.min(self.marked.len()));
        while out.len() < n {
            let Some(&(g, seq)) = self.order.front() else {
                break;
            };
            if seq > last {
                break;
            }
            self.order.pop_front();
            if let Some(&(kind, s)) = self.marked.get(&g)
                && s == seq
            {
                self.marked.remove(&g);
                out.push((g, kind, seq));
            }
        }
        out
    }
}

/// How far a player sees on `map`: a battleground's own distance, a
/// dungeon's or raid's, else the continents'.
pub fn radius(map: &Map) -> f32 {
    if crate::game::battlegrounds::bg_of_map(map.key.map).is_some() {
        map.cfg.battleground_visibility_yards
    } else if map.key.instance != 0 {
        map.cfg.instance_visibility_yards
    } else {
        map.cfg.visibility_yards
    }
}

/// How far `p` sees: on a flight master's mount, `[visibility]
/// flight_yards` (if further than the map's), else the map's.
pub fn sight(map: &Map, p: &Player) -> f32 {
    let r = radius(map);
    if p.flight.is_some() {
        r.max(map.cfg.flight_visibility_yards)
    } else {
        r
    }
}

/// [`sight`], and how far past it what `p` knows is kept, squared.
fn sight2(map: &Map, p: &Player) -> (f32, f32) {
    let r = sight(map, p);
    (r * r, (r + MARGIN) * (r + MARGIN))
}

/// Distances on the ground: height doesn't count.
fn dist2(a: Vector3d, b: Vector3d) -> f32 {
    (a.x - b.x).powi(2) + (a.y - b.y).powi(2)
}

/// The cell `g` is in, in the grid.
pub fn cell_of_object(map: &Map, g: Guid) -> Option<CellKey> {
    if guid::is_player(g) {
        map.players.get(&g).map(|p| p.cell)
    } else if guid::is_unit(g) {
        map.creature(g).map(|c| c.cell)
    } else if guid::is_game_object(g) {
        map.gameobject(g).map(|o| o.cell)
    } else if guid::is_corpse(g) {
        map.corpses.get(&g).map(|c| c.cell)
    } else if crate::game::map::spells::dynamic::is_dynamic_object(g) {
        map.spells.dynamic.get(&g).map(|d| d.cell)
    } else {
        None
    }
}

/// Whether `g` is stealthed or invisible.
fn stealthy(map: &Map, g: Guid) -> bool {
    use crate::game::map::auras::{AURA_MOD_INVISIBILITY, AURA_MOD_STEALTH};
    crate::game::map::auras::auras(map, g)
        .is_some_and(|a| a.has(AURA_MOD_STEALTH) || a.has(AURA_MOD_INVISIBILITY))
}

/// Whether who sees `g`, and whom it sees, turns on distance alone: then,
/// between it and another such, [`death::can_see`] is true. Not a ghost,
/// stealthed or invisible, concealed by a script, a spirit healer or
/// guide, a trap, a transport; a player not on a flight (who sees further
/// and moves further between looks), not looking from elsewhere (a
/// cinematic, a far sight), not between maps.
pub fn plain_now(map: &Map, g: Guid) -> bool {
    if map.globals.contains(&g) {
        return false;
    }
    if guid::is_player(g) {
        let Some(p) = map.players.get(&g) else {
            return false;
        };
        if p.awaiting_worldport
            || p.flight.is_some()
            || p.misc.view.is_some()
            || matches!(p.death, death::Death::Ghost { .. })
        {
            return false;
        }
        return !stealthy(map, g);
    }
    if guid::is_unit(g) {
        let Some(c) = map.creature(g) else {
            return false;
        };
        let spirit = c.fields.get_u32(crate::game::fields::UNIT_NPC_FLAGS)
            & (crate::game::map::npc::NPC_FLAG_SPIRITHEALER
                | crate::game::map::npc::NPC_FLAG_SPIRITGUIDE)
            != 0;
        if spirit || c.scripted.as_deref().is_some_and(|s| s.concealed) {
            return false;
        }
        return !stealthy(map, g);
    }
    if guid::is_game_object(g) {
        return map
            .gameobject(g)
            .is_some_and(|o| o.kind != crate::game::map::objects::GO_TRAP);
    }
    if guid::is_corpse(g) {
        return map.corpses.contains_key(&g);
    }
    crate::game::map::spells::dynamic::is_dynamic_object(g) && map.spells.dynamic.contains_key(&g)
}

/// Whether the grid takes `g` as plain.
fn flagged_plain(map: &Map, g: Guid) -> bool {
    cell_of_object(map, g)
        .and_then(|c| map.grid.entry(g, c))
        .is_some_and(|e| e.plain)
}

/// `g` is no longer taken as plain, until its refresh says it is.
fn unflag(map: &mut Map, g: Guid) {
    if let Some(c) = cell_of_object(map, g) {
        map.grid.update(g, c, |e| e.plain = false);
    }
}

/// Note that `object` moved to `position`: its visibility is worked out at
/// the next update if it went far enough. A flier goes further between
/// looks, in proportion to how much further it sees (32 yards at 400):
/// what comes into its sight is as late, for its distance, as for a
/// walker, and a flight across a continent takes a quarter of the looks
/// (each over a window sixteen times the size).
///
/// A stealthed unit is looked at every [`STEALTH_THRESHOLD`] yards: who
/// sees it turns on yards (Thistle Tea's stealthed walker, announced every
/// two).
///
/// One the grid takes as plain that is no longer (it has gone past
/// [`DRIFT`], or taken flight, stealth, a ghost's shape) is taken whole
/// from now on, and worked out whole.
pub fn moved(map: &mut Map, object: Guid, anchor: Vector3d, position: Vector3d) {
    let stealthed = || {
        crate::game::map::auras::auras(map, object)
            .is_some_and(|a| a.has(crate::game::map::auras::AURA_MOD_STEALTH))
    };
    let threshold = match map.players.get(&object) {
        Some(p) if p.flight.is_some() => MOVE_THRESHOLD * sight(map, p) / radius(map),
        _ if stealthed() => STEALTH_THRESHOLD,
        _ => MOVE_THRESHOLD,
    };
    let d2 = dist2(anchor, position);
    if (d2 > DRIFT * DRIFT || !plain_now(map, object)) && flagged_plain(map, object) {
        unflag(map, object);
        map.visibility_dirty.insert(object);
        return;
    }
    if d2 >= threshold * threshold {
        map.visibility_dirty.moved(object);
    }
}

/// A unit's stealth or invisibility changed: who sees it is worked out
/// again.
pub fn stealth_changed(map: &mut Map, g: Guid) {
    let _z = crate::zone!("visibility: stealth");
    refresh(map, g);
}

/// Work out the visibility of everything marked since the last update,
/// oldest first, until the budget is spent (`[visibility]
/// refresh_budget_ms`): the rest wait for the next.
pub fn refresh_dirty(map: &mut Map) {
    map.fresh.clear();
    if map.visibility_dirty.is_empty() {
        return;
    }
    // Another radius: K was for the old one.
    let r = radius(map);
    if map.visibility_dirty.radius != r {
        map.grid.unplain_all();
        map.visibility_dirty.radius = r;
    }
    // Those to be worked out whole, this update or later, are taken whole
    // by the others meanwhile: what changed them may change who sees them.
    let whole: Vec<Guid> = map
        .visibility_dirty
        .marked
        .iter()
        .filter(|(_, (k, _))| *k == Kind::Full)
        .map(|(g, _)| *g)
        .collect();
    for g in whole {
        unflag(map, g);
    }
    let started = Instant::now();
    let budget = map.cfg.visibility_budget;
    let last = map.visibility_dirty.seq;
    let z = crate::zone!("objects refreshed");
    let mut done = 0;
    loop {
        let one_by_one = map.visibility_dirty.one_by_one;
        let size = if one_by_one {
            1
        } else {
            let v = &map.visibility_dirty;
            let size = chunk_size(budget, started.elapsed(), v.per_refresh);
            if budget.is_zero() {
                size
            } else {
                size.min((2 * v.last_worked).max(GROW_FROM))
            }
        };
        let mut chunk = map.visibility_dirty.take(size, last);
        if chunk.is_empty() {
            break;
        }
        if one_by_one {
            chunk[0].1 = Kind::Full;
        }
        let began = Instant::now();
        // Past the budget, a chunk stops after the refresh at hand, the
        // update's first excepted, its rest waiting for the next update as
        // a chunk not begun would: two sides coming into each other's sight
        // make a refresh cost a hundred times what the last chunk's did.
        let until = (!budget.is_zero() && !one_by_one).then(|| started + budget);
        let first = done == 0;
        let parallel = chunk.len() >= PARALLEL_FROM
            || (chunk.len() >= 2
                && map.visibility_dirty.per_refresh * chunk.len() as u32 >= PARALLEL_COST);
        let worked = run_until(map, &chunk, until, first, parallel);
        done += worked;
        if worked < chunk.len() {
            map.visibility_dirty.put_back(&chunk[worked..]);
        }
        map.visibility_dirty.per_refresh = began.elapsed() / worked.max(1) as u32;
        map.visibility_dirty.last_worked = worked;
        if started.elapsed() >= budget {
            break;
        }
    }
    z.value(done as u64);
}

/// A chunk's size: what is left of `budget` after `spent`, at `per` a
/// refresh, between [`MIN_CHUNK`] and [`CHUNK`]. A budget of nothing is a
/// whole chunk an update.
fn chunk_size(budget: Duration, spent: Duration, per: Duration) -> usize {
    if budget.is_zero() || per.is_zero() {
        return CHUNK;
    }
    let left = budget.saturating_sub(spent).as_nanos();
    usize::try_from(left / per.as_nanos())
        .unwrap_or(CHUNK)
        .clamp(MIN_CHUNK, CHUNK)
}

/// Who should know `object` now, and, if it is a player, what it should
/// know: every pair it is in looked at.
pub fn refresh(map: &mut Map, object: Guid) {
    run_until(map, &[(object, Kind::Full, 0)], None, true, false);
}

/// One of a chunk's refreshes.
struct Item {
    g: Guid,
    full: bool,
    /// Its anchor before its side work, for one not worked out after all.
    anchor: Option<Vector3d>,
    /// Where it is, and how far from its anchor, once its side work is
    /// done; `None` if it has nothing to work out (gone, or between maps).
    at: Option<(Vector3d, f32)>,
}

/// A decision: `observer` is to learn of `object`, or to forget it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Introduce(Guid, Guid),
    Part(Guid, Guid),
}

/// A chunk of refreshes: which are whole, their side work, their decisions
/// (read-only, across the threads), then the decisions applied in order,
/// until `until` (after the first, if `first` is the update's first);
/// how many were worked out. Those after it are as they were before the
/// chunk (but one found to be taken whole, which stays so): a pair with
/// one of them that an earlier one shared was the earlier's to decide,
/// and was; the rest are theirs, when they are worked out.
fn run_until(
    map: &mut Map,
    chunk: &[(Guid, Kind, u64)],
    until: Option<Instant>,
    first: bool,
    parallel: bool,
) -> usize {
    let mut items: Vec<Item> = Vec::with_capacity(chunk.len());
    {
        let _z = crate::zone!("visibility: side work");
        for &(g, kind, _) in chunk {
            let full = kind == Kind::Full || !flagged_plain(map, g) || !plain_now(map, g);
            if full {
                // The others look at it whole until it is worked out.
                unflag(map, g);
            }
            items.push(Item {
                g,
                full,
                anchor: None,
                at: None,
            });
        }
        for it in &mut items {
            it.at = prepare(map, it.g, it.full, &mut it.anchor);
            if !it.full && it.at.is_some_and(|(_, delta)| delta > DRIFT) {
                it.full = true;
                unflag(map, it.g);
            }
        }
        // Each refresh deciding pairs, by its place: a pair two share is
        // the earlier's.
        for (i, it) in items.iter().enumerate() {
            if it.at.is_some()
                && !map.globals.contains(&it.g)
                && let Some(c) = cell_of_object(map, it.g)
            {
                map.grid.update(it.g, c, |e| e.batch = i as u32 + 1);
            }
        }
    }
    let decided = decide_all(map, &items, parallel);
    let z = crate::zone!("visibility: applied");
    let mut crowds = Crowds::new(map);
    let mut worked = 0;
    for (it, d) in items.iter().zip(decided) {
        if until.is_some_and(|t| Instant::now() >= t) && (worked > 0 || !first) {
            break;
        }
        worked += 1;
        map.visibility_dirty.skipped += d.skipped;
        map.visibility_dirty.looked += d.looked;
        apply(map, &d.acts, &mut crowds);
        if let Some((pos, _)) = it.at
            && let Some(c) = cell_of_object(map, it.g)
        {
            let plain = plain_now(map, it.g);
            map.grid.update(it.g, c, |e| {
                e.ax = pos.x;
                e.ay = pos.y;
                e.plain = plain;
                e.batch = 0;
            });
        }
    }
    drop(z);
    // Those not worked out: their anchors as before the chunk (one taken
    // whole stays so, until it is worked out).
    for it in &items[worked..] {
        if let Some(c) = cell_of_object(map, it.g) {
            map.grid.update(it.g, c, |e| e.batch = 0);
        }
        if let Some(a) = it.anchor {
            if let Some(p) = map.players.get_mut(&it.g) {
                p.vis_anchor = a;
            } else if let Some(c) = map.creature_mut(it.g) {
                c.vis_anchor = a;
            }
        }
    }
    // Before anything else is sent: the creates are due before what names
    // their objects.
    crowds.send(map);
    crate::game::map::creature_combat::out_of_sight(map);
    worked
}

/// What a refresh does before its decisions: the tiles ahead, the zone,
/// the anchor; the zone and the tiles only when it has crossed a cell
/// since they were last done (or it is worked out whole). Where it is,
/// and how far it has gone from its anchor; `None` if it has nothing to
/// work out. A player's or creature's anchor before it, into `was`.
fn prepare(
    map: &mut Map,
    g: Guid,
    full: bool,
    was: &mut Option<Vector3d>,
) -> Option<(Vector3d, f32)> {
    let pos = map.position_of(g)?;
    let cell = cell_of_object(map, g);
    let anchor = cell
        .and_then(|c| map.grid.entry(g, c))
        .map(|e| (e.ax, e.ay));
    let delta = anchor.map_or(f32::INFINITY, |(x, y)| {
        let d = ((x - pos.x).powi(2) + (y - pos.y).powi(2)).sqrt();
        if d.is_nan() { f32::INFINITY } else { d }
    });
    let side = |last: Option<CellKey>| full || last.is_none() || last != cell;
    // Arrived or moved on: the navmesh and terrain about them, ahead.
    if let Some(p) = map.players.get(&g)
        && side(p.side_cell)
    {
        let sees = sight(map, p);
        crate::game::map::tiles::ahead(map, pos, sees);
    }
    // A boat or zeppelin: everyone in the map knows it.
    if map.globals.contains(&g) {
        return Some((pos, delta));
    }
    if guid::is_player(g) {
        let p = map.players.get_mut(&g)?;
        if p.awaiting_worldport {
            return None;
        }
        *was = Some(std::mem::replace(&mut p.vis_anchor, pos));
        if side(p.side_cell) {
            p.side_cell = cell;
            crate::game::map::world::update_zone(map, g);
        }
    } else if let Some(c) = map.creature_mut(g) {
        *was = Some(std::mem::replace(&mut c.vis_anchor, pos));
        if side(c.side_cell) {
            c.side_cell = cell;
            crate::game::map::conditions::locate(map, g, true);
        }
    } else {
        crate::game::map::conditions::locate(map, g, true);
    }
    Some((pos, delta))
}

/// The pool the decisions are worked out on: `[visibility]
/// refresh_threads` of them (0: the machine's cores, up to 8), made at the
/// first chunk big enough to share.
pub(crate) fn pool(threads: usize) -> Option<&'static rayon::ThreadPool> {
    static POOL: OnceLock<Option<rayon::ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        let n = if threads == 0 {
            std::thread::available_parallelism().map_or(1, |n| n.get().min(8))
        } else {
            threads
        };
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .thread_name(|i| format!("visibility-{i}"))
            .build()
            .ok()
    })
    .as_ref()
}

/// Each item's decisions, in order: across the pool for a big chunk, else
/// here. The same either way: each is worked out from the map as the side
/// work left it, and nothing is changed until all are.
fn decide_all(map: &Map, items: &[Item], parallel: bool) -> Vec<Decided> {
    let _z = crate::zone!("visibility: decided");
    let threads = map.cfg.visibility_threads;
    if parallel
        && threads != 1
        && let Some(pool) = pool(threads)
    {
        return pool.install(|| {
            items
                .par_iter()
                .enumerate()
                .map(|(i, it)| decide(map, it, i as u32 + 1))
                .collect()
        });
    }
    items
        .iter()
        .enumerate()
        .map(|(i, it)| decide(map, it, i as u32 + 1))
        .collect()
}

/// A refresh's decisions, and the pairs it skipped and looked at.
#[derive(Default)]
struct Decided {
    acts: Vec<Act>,
    skipped: u64,
    looked: u64,
}

fn decide(map: &Map, it: &Item, place: u32) -> Decided {
    let mut d = Decided::default();
    let acts = &mut d.acts;
    let Some((pos, delta)) = it.at else {
        return d;
    };
    if map.globals.contains(&it.g) {
        for p in map.players.values() {
            if !p.awaiting_worldport && !p.known.contains(&it.g) {
                acts.push(Act::Introduce(p.guid, it.g));
            }
        }
    } else if it.full || !plain_now(map, it.g) {
        decide_whole(map, it.g, pos, acts);
    } else {
        (d.skipped, d.looked) = decide_moved(map, it.g, pos, delta, place, acts);
    }
    d
}

/// For the benchmarks: `object`'s whole refresh decided (not applied),
/// across the threads in parts or on this thread alone, as many decisions
/// as it made.
#[doc(hidden)]
pub fn decide_whole_alone(map: &mut Map, object: Guid, parts: bool) -> usize {
    let Some(pos) = map.position_of(object) else {
        return 0;
    };
    let cfg = map.cfg.clone();
    if !parts {
        let mut alone = (*cfg).clone();
        alone.visibility_threads = 1;
        map.cfg = std::sync::Arc::new(alone);
    }
    let mut acts = Vec::new();
    decide_whole(map, object, pos, &mut acts);
    map.cfg = cfg;
    acts.len()
}

/// Every pair `object` is in, looked at.
fn decide_whole(map: &Map, object: Guid, pos: Vector3d, acts: &mut Vec<Act>) {
    let keep = radius(map) + MARGIN;
    // The players near the object: should they know it? Each by how far it
    // sees; one further off that sees further (a flier) is told of it by
    // its own looks, which go as far as it sees.
    // Each with whether the grid takes it as plain: between it and the
    // object, if that is plain now too, [`death::can_see`] is true both
    // ways (the quick path's rule), and isn't asked.
    let nearby: Vec<(Guid, bool)> = map
        .grid
        .around_entries(pos.x, pos.y, keep)
        .map(|e| (e.guid, e.plain))
        .collect();
    let plain = plain_now(map, object);
    let sees = |a: Guid, b: Guid, both: bool| {
        debug_assert!(
            !both || death::can_see(map, a, b),
            "{a:?} and {b:?}, both plain, and one doesn't see the other"
        );
        both || death::can_see(map, a, b)
    };
    let observers = map.observers(object);
    in_parts(map, &nearby, acts, |part, acts| {
        for &(g, also) in part {
            let both = plain && also;
            if g == object || !guid::is_player(g) {
                continue;
            }
            let Some(p) = map.players.get(&g) else {
                continue;
            };
            if p.awaiting_worldport {
                continue;
            }
            let (r2, keep2) = sight2(map, p);
            let d2 = dist2(crate::game::map::cinematic::view_point(p), pos);
            // Whether they may see it is asked only where it decides: not
            // of one out of their sight, nor of one past their margin.
            if !p.known.contains(&object) {
                if d2 <= r2 && sees(g, object, both) {
                    acts.push(Act::Introduce(g, object));
                }
            } else if d2 > keep2 || !sees(g, object, both) {
                acts.push(Act::Part(g, object));
            }
        }
    });
    // Observers that have gone out of the grid window, and past their
    // sight.
    if let Some(observers) = observers {
        far(map, object, pos, observers, acts);
    }

    // A player: what should they know?
    let Some(me) = map.players.get(&object) else {
        return;
    };
    // What the player knows is what is near where they see from, as far
    // as they see.
    let view = crate::game::map::cinematic::view_point(me);
    let (r2, keep2) = sight2(map, me);
    let own = sight(map, me) + MARGIN;
    let (pos, nearby): (Vector3d, Vec<(Guid, bool)>) = if dist2(view, pos) > 0.01 || own > keep {
        // The cells the disc touches, not its square's corners (a third of
        // a flier's window): past `own`, nothing is introduced, and all
        // that is known is parted below.
        let within = map.grid.within_entries(view.x, view.y, own);
        (view, within.map(|e| (e.guid, e.plain)).collect())
    } else {
        (pos, nearby)
    };
    // What the player knows, and what has come into their sight: the rest
    // of the window, unknown and out of sight, has nothing to tell them.
    in_parts(map, &nearby, acts, |part, acts| {
        for &(g, also) in part {
            if g == object {
                continue;
            }
            let knows = me.known.contains(&g);
            if !map
                .position_of(g)
                .is_some_and(|other| knows || dist2(other, pos) <= r2)
            {
                continue;
            }
            if guid::is_player(g) && map.players.get(&g).is_some_and(|o| o.awaiting_worldport) {
                continue;
            }
            // Not known, it is in sight: the filter let it through for that.
            let sees = sees(object, g, plain && also);
            if !knows && sees {
                acts.push(Act::Introduce(object, g));
            } else if knows && !sees {
                acts.push(Act::Part(object, g));
            }
        }
    });
    for &g in &map.globals {
        if !me.known.contains(&g) {
            acts.push(Act::Introduce(object, g));
        }
    }
    gone(map, me, pos, keep2, acts);
}

/// A whole refresh's looks over this many objects at least are split
/// across the visibility threads ([`in_parts`]) ...
const PARTS_FROM: usize = 512;
/// ... in parts of this many.
const PART: usize = 128;

/// `look` over `all` in order, its decisions into `acts`: across the
/// visibility threads, in parts, for many (a stealthed unit's whole
/// refresh among a crowd of 1,600, every one of whom it is asked of, on
/// the map's thread alone: 0.45 ms, and 240 pets going into Prowl at once
/// 110 ms), each part's decisions after the last's, as in one pass; else
/// here. Within a chunk worked out across the threads, its parts are those
/// threads' too.
fn in_parts<T: Sync>(
    map: &Map,
    all: &[T],
    acts: &mut Vec<Act>,
    look: impl Fn(&[T], &mut Vec<Act>) + Sync,
) {
    let threads = map.cfg.visibility_threads;
    if all.len() >= PARTS_FROM
        && threads != 1
        && let Some(pool) = pool(threads)
    {
        let parts = || -> Vec<Vec<Act>> {
            all.par_chunks(PART)
                .map(|part| {
                    let mut acts = Vec::new();
                    look(part, &mut acts);
                    acts
                })
                .collect()
        };
        let parts = if pool.current_thread_index().is_some() {
            parts()
        } else {
            pool.install(parts)
        };
        for mut p in parts {
            acts.append(&mut p);
        }
        return;
    }
    look(all, acts);
}

/// Of `object`'s observers, those past their sight of it, parted.
fn far(
    map: &Map,
    object: Guid,
    pos: Vector3d,
    observers: &ahash::AHashSet<Guid>,
    acts: &mut Vec<Act>,
) {
    for &o in observers {
        let away = map.players.get(&o).is_none_or(|p| {
            dist2(crate::game::map::cinematic::view_point(p), pos) > sight2(map, p).1
        });
        if away {
            acts.push(Act::Part(o, object));
        }
    }
}

/// What `me` knows past its margin from `pos`, parted (transports, which
/// everyone knows, aside).
fn gone(map: &Map, me: &Player, pos: Vector3d, keep2: f32, acts: &mut Vec<Act>) {
    for &k in &me.known {
        if !map.globals.contains(&k)
            && map
                .position_of(k)
                .is_none_or(|other| dist2(other, pos) > keep2)
        {
            acts.push(Act::Part(me.guid, k));
        }
    }
}

/// A plain object that moved `delta` from its anchor: what moving can
/// change. The pairs K covers are skipped (counted as known); the rest
/// are looked at as [`decide_whole`] looks at them, but those a refresh
/// earlier in the chunk shares (`place`), which decided them both ways.
fn decide_moved(
    map: &Map,
    object: Guid,
    pos: Vector3d,
    delta: f32,
    place: u32,
    acts: &mut Vec<Act>,
) -> (u64, u64) {
    let r = radius(map);
    let keep = r + MARGIN;
    let (r2, keep2) = (r * r, keep * keep);
    let sq = |v: f32| if v > 0.0 { v * v } else { -1.0 };
    let skip2 = sq(r - DRIFT - delta);
    // An anchor further than this is of one out of sight now.
    let beyond2 = (r + DRIFT) * (r + DRIFT);
    // A whole cell: its objects are within DRIFT of their anchors.
    let cell_skip2 = sq(r - 2.0 * DRIFT - delta);
    let me = map.players.get(&object);
    let my_cell = cell_of_object(map, object);
    let (my_r2, my_keep2) = me.map_or((0.0, 0.0), |p| sight2(map, p));
    let observers = map.observers(object);
    // What the window holds that the object knows (transports aside), and
    // who in it knows the object.
    let (mut known, mut knowing) = (0usize, 0usize);
    let (mut skipped, mut looked) = (0u64, 0u64);
    let mut mine = Vec::new();
    for (key, cell) in map.grid.cells_around(pos.x, pos.y, keep) {
        // A cell wholly past the margin, of plain objects: none of them is
        // to be introduced either way, and one known (to be parted) shows
        // in the counts below.
        if cell.unplain() == 0 && grid::nearest2(key, pos.x, pos.y) > keep2 {
            continue;
        }
        if cell.unplain() == 0 && grid::farthest2(key, pos.x, pos.y) <= cell_skip2 {
            let here = usize::from(Some(key) == my_cell);
            if me.is_some() {
                known += cell.entries().len() - here;
                knowing += cell.players() as usize - here;
            } else {
                knowing += cell.players() as usize;
            }
            skipped += cell.entries().len() as u64;
            continue;
        }
        for e in cell.entries() {
            if e.guid == object {
                continue;
            }
            let is_player = guid::is_player(e.guid);
            if e.plain && (e.ax - pos.x).powi(2) + (e.ay - pos.y).powi(2) <= skip2 {
                known += usize::from(me.is_some());
                knowing += usize::from(is_player);
                skipped += 1;
                continue;
            }
            looked += 1;
            let knows = me.is_some_and(|p| p.known.contains(&e.guid));
            let known_by = is_player && observers.is_some_and(|o| o.contains(&e.guid));
            known += usize::from(knows && !(!e.plain && map.globals.contains(&e.guid)));
            knowing += usize::from(known_by);
            if e.batch != 0 && e.batch < place {
                continue;
            }
            if e.plain {
                // Two plain: each sees the other, from where it is, as far
                // as the map's radius. Past it by more than DRIFT, and
                // unknown both ways, nothing to look up.
                let anchor2 = (e.ax - pos.x).powi(2) + (e.ay - pos.y).powi(2);
                if !knows && !known_by && anchor2 > beyond2 {
                    continue;
                }
                let Some(other) = map.position_of(e.guid) else {
                    continue;
                };
                let d2 = dist2(other, pos);
                if is_player {
                    if !known_by && d2 <= r2 {
                        acts.push(Act::Introduce(e.guid, object));
                    } else if known_by && d2 > keep2 {
                        acts.push(Act::Part(e.guid, object));
                    }
                }
                if me.is_some() {
                    if !knows && d2 <= r2 {
                        mine.push(Act::Introduce(object, e.guid));
                    } else if knows && d2 > keep2 {
                        mine.push(Act::Part(object, e.guid));
                    }
                }
                continue;
            }
            // Should they know it?
            if is_player
                && let Some(o) = map.players.get(&e.guid)
                && !o.awaiting_worldport
            {
                let (r2, keep2) = sight2(map, o);
                let d2 = dist2(crate::game::map::cinematic::view_point(o), pos);
                if !known_by {
                    if d2 <= r2 && death::can_see(map, e.guid, object) {
                        acts.push(Act::Introduce(e.guid, object));
                    }
                } else if d2 > keep2 || !death::can_see(map, e.guid, object) {
                    acts.push(Act::Part(e.guid, object));
                }
            }
            // Should it know them?
            if me.is_some()
                && let Some(other) = map.position_of(e.guid)
            {
                let d2 = dist2(other, pos);
                let away = is_player
                    && map
                        .players
                        .get(&e.guid)
                        .is_some_and(|o| o.awaiting_worldport);
                if (knows || d2 <= my_r2) && !away {
                    let sees = death::can_see(map, object, e.guid);
                    if !knows && sees {
                        mine.push(Act::Introduce(object, e.guid));
                    } else if knows && !sees {
                        mine.push(Act::Part(object, e.guid));
                    }
                }
                if knows && d2 > my_keep2 && !map.globals.contains(&e.guid) {
                    mine.push(Act::Part(object, e.guid));
                }
            }
        }
    }
    // Who knows it from outside the window: only if the counts say so.
    if let Some(observers) = observers
        && knowing != observers.len()
    {
        far(map, object, pos, observers, acts);
    }
    acts.append(&mut mine);
    let Some(me) = me else {
        return (skipped, looked);
    };
    let mut transports = 0;
    for &g in &map.globals {
        if me.known.contains(&g) {
            transports += 1;
        } else {
            acts.push(Act::Introduce(object, g));
        }
    }
    // What it knows outside the window: only if the counts say so.
    if known + transports != me.known.len() {
        gone(map, me, pos, my_keep2, acts);
    }
    (skipped, looked)
}

/// The decisions, in order; each checked again as it is applied (an
/// earlier one may have done it). An introduction a crowd shares goes with
/// the chunk's ([`Crowds`]).
fn apply(map: &mut Map, acts: &[Act], crowds: &mut Crowds) {
    for &act in acts {
        match act {
            Act::Introduce(o, x) => {
                if map.players.get(&o).is_some_and(|p| !p.known.contains(&x))
                    && !crowds.take(map, o, x)
                {
                    introduce(map, o, x);
                }
            }
            Act::Part(o, x) => part(map, o, x),
        }
    }
}

/// A chunk's introductions of objects to those who see them as a stranger
/// does, in packets shared among them (`values::Shared`'s planning, as the
/// update's values are), broadcast at the chunk's end, before anything else
/// is sent: no client is held for them. A unit coming out of stealth in a
/// crowd of 1,600 had each of its observers' batches take its create and
/// each of them held, every broadcast to them until the step's end copied
/// into each hold (a volley's ticks: 30 ms), then each let go (20 ms); two
/// sides coming into each other's sight, each player's create copied into
/// a thousand batches.
///
/// Each client gets the same creates as one by one, each once, before
/// anything about their objects, and only the packets differ: the one by
/// one's go in its batch, after the update's forgets; these come where the
/// chunk ends. So a client with a forget of the object still to come in
/// this update (it lost sight of it and has it again) is told of it in its
/// batch, as are those who see it otherwise (its tap's outsiders, its
/// marker, a pet's master, a player's group with quests to show: `views`),
/// and a passenger, a corpse, a game object and a transport are always, and
/// a player to one who has the player's corpse or is to forget it.
/// Below [`values::SHARE_FROM`] of a crowd, the creates go into the
/// batches; so do the shares of them that only one client gets.
pub(crate) struct Crowds {
    on: bool,
    slots: Vec<Crowd>,
    /// Each object's slot, `None` if it is introduced one by one.
    of: AHashMap<Guid, Option<usize>>,
}

/// One object's crowd: its create, who sees it as a stranger does, and
/// those told of it.
struct Crowd {
    object: Guid,
    block: std::sync::Arc<[u8]>,
    strangers: crate::game::map::views::Strangers,
    /// Those told of it, and their clients.
    told: Vec<Guid>,
    to: crate::game::conn::Clients,
}

impl Crowds {
    fn new(map: &Map) -> Self {
        Self {
            on: !map.visibility_dirty.creates_one_by_one,
            slots: Vec::new(),
            of: AHashMap::new(),
        }
    }

    /// `observer` is told of `object` with the rest of its crowd: known,
    /// observing, its create to come. False if it is to be introduced on
    /// its own ([`introduce`]).
    fn take(&mut self, map: &mut Map, observer: Guid, object: Guid) -> bool {
        if !self.on {
            return false;
        }
        let slot = match self.of.get(&object) {
            Some(&s) => s,
            None => {
                let s = Self::open(map, object).map(|c| {
                    self.slots.push(c);
                    self.slots.len() - 1
                });
                self.of.insert(object, s);
                s
            }
        };
        let Some(s) = slot else {
            return false;
        };
        let crowd = &mut self.slots[s];
        // Its own view, or a forget of it still to come: on its own. So is a
        // player whose corpse the client has, or is to forget: the corpse's
        // forget goes first, in the batch, and the client never has both
        // (a corpse's create before its owner's forget crashed it).
        let forgets = |g: Guid| {
            map.forget.get(&observer).is_some_and(|f| f.contains(&g))
                || map.batches.get(&observer).is_some_and(|b| b.forgets(g))
        };
        let corpse = guid::is_player(object).then(|| guid::corpse(guid::counter(object)));
        if !crowd.strangers.includes(observer)
            || forgets(object)
            || corpse.is_some_and(|c| {
                forgets(c) || map.players.get(&observer).is_some_and(|p| p.known.contains(&c))
            })
        {
            return false;
        }
        debug_assert!(
            !(guid::is_unit(object) || guid::is_player(object))
                || crate::game::map::views::view_of(map, observer, object)
                    == crate::game::map::views::View::default(),
            "{observer:?} sees {object:?} as a stranger"
        );
        let Some(p) = map.players.get_mut(&observer) else {
            return false;
        };
        if p.awaiting_worldport || !p.known.insert(object) {
            return false;
        }
        // Made afresh: as no quest has it, until the next look.
        p.quest_objects.remove(&object);
        crowd.to.insert(map.clients.index_of(observer));
        crowd.told.push(observer);
        let first = crowd.told.len() == 1;
        map.add_observer(object, observer);
        if first {
            if map.creature_by_guid.contains_key(&object) {
                map.awake.insert(object);
            }
            crate::game::map::conditions::locate(map, object, false);
        }
        true
    }

    /// `object`'s crowd, if its create may be shared: a unit's, a player's
    /// not aboard a transport, a spell's area.
    fn open(map: &mut Map, object: Guid) -> Option<Crowd> {
        let shareable = if guid::is_player(object) {
            map.players
                .get(&object)
                .is_some_and(|p| p.movement.flags.get_on_transport().is_none())
        } else if guid::is_unit(object) {
            map.creature(object).is_some()
        } else {
            crate::game::map::spells::dynamic::is_dynamic_object(object)
                && map.spells.dynamic.contains_key(&object)
        };
        if !shareable || map.globals.contains(&object) {
            return None;
        }
        let block = map.public_create(object)?;
        Some(Crowd {
            object,
            block,
            strangers: crate::game::map::views::Strangers::of(map, object),
            told: Vec::new(),
            to: Default::default(),
        })
    }

    /// The chunk's creates: in packets shared among their crowds,
    /// broadcast; those for a few, or a share only one client gets, into
    /// the batches, those clients held.
    fn send(self, map: &mut Map) {
        if self.slots.iter().all(|c| c.told.is_empty()) {
            return;
        }
        let _z = crate::zone!("visibility: crowds' creates");
        let mut planner = crate::game::map::values::Shared::default();
        planner.one_by_one = map.shared_values.one_by_one;
        let mut rest: Vec<(Guid, Guid, Vec<u8>)> = Vec::new();
        for mut c in self.slots {
            if c.told.is_empty() {
                continue;
            }
            // One held already (for a create one by one, before or since its
            // take): its batch goes before what waits in its hold, so the
            // create goes into the batch with the rest, where a forget of it
            // later this update drops it, as one by one.
            let mut held = 0;
            for &o in &c.told {
                if map.players.get(&o).is_some_and(|p| p.outbound.is_holding()) {
                    if let Some(i) = map.clients.get(o) {
                        c.to.remove(i);
                    }
                    rest.push((o, c.object, c.block.to_vec()));
                    held += 1;
                }
            }
            map.visibility_dirty.shared_creates += (c.told.len() - held) as u64;
            let own = guid::is_player(c.object).then(|| map.clients.index_of(c.object));
            let cell = cell_of_object(map, c.object).unwrap_or_default();
            planner.tell(
                c.object,
                own,
                cell,
                c.block.to_vec(),
                std::sync::Arc::new(c.to),
            );
        }
        let pool = map.encode_pool();
        let packets = planner.plan_with(&map.clients, pool, |g, object, block: &[u8]| {
            rest.push((g, object, block.to_vec()));
        });
        for (frame, to) in &packets {
            map.broadcast(frame, to);
        }
        for (g, object, block) in rest {
            map.batches
                .entry(g)
                .or_default()
                .push_for(object, &block);
            // What the map sends it from now on follows the create.
            map.hold(g);
        }
    }
}

/// `observer`'s client learns of `object`: a player with quests in their
/// log as their group's members see them, with the log's ids (mangos's
/// `UF_FLAG_GROUP_ONLY`), else as anyone does.
pub fn introduce(map: &mut Map, observer: Guid, object: Guid) {
    // One aboard a boat the observer doesn't know yet: the boat first, so
    // the create naming it doesn't come before it.
    if let Some(t) = map
        .players
        .get(&object)
        .and_then(|p| p.movement.flags.get_on_transport())
        .map(|t| t.transport.guid)
        && map.gameobject(t).is_some()
        && map
            .players
            .get(&observer)
            .is_some_and(|p| !p.known.contains(&t))
    {
        introduce(map, observer, t);
    }
    // A unit or player as the observer sees it: a creature tapped by
    // another grey, a mark theirs tracked, a pet theirs or their Empathy's
    // with its owner-only fields, a group's member's quest log's ids
    // (`views`).
    let block = if guid::is_unit(object) || guid::is_player(object) {
        crate::game::map::views::create_for(map, observer, object)
    } else {
        map.public_create(object)
    };
    let Some(block) = block else {
        return;
    };
    let Some(p) = map.players.get_mut(&observer) else {
        return;
    };
    if !p.known.insert(object) {
        return;
    }
    // Made afresh: as no quest has it, until the next look.
    p.quest_objects.remove(&object);
    map.batches
        .entry(observer)
        .or_default()
        .push_for(object, &block);
    // What the map sends it from now on, about the object or not, follows
    // the create.
    map.hold(observer);
    map.add_observer(object, observer);
    if map.creature_by_guid.contains_key(&object) {
        map.awake.insert(object);
    }
    crate::game::map::conditions::locate(map, object, false);
    // A banner taken by attacking it starts each viewer afresh (Thistle
    // Tea's `reset_banner_interaction`), once they have it (the client is
    // held).
    if guid::is_game_object(object)
        && crate::game::map::objects::resets_on_sight(map, object)
        && let Some(p) = map.players.get(&observer)
    {
        p.outbound
            .send(&wow_world_messages::vanilla::SMSG_GAMEOBJECT_RESET_STATE { guid: object });
    }
}

/// `observer`'s client forgets `object`.
pub fn part(map: &mut Map, observer: Guid, object: Guid) {
    if let Some(p) = map.players.get_mut(&observer)
        && p.known.remove(&object)
    {
        map.forget.entry(observer).or_default().push(object);
        // A creature fighting them may give up on them: once the step is
        // done ([`run`]).
        if guid::is_unit(object) && map.cfg.creatures.evade_out_of_sight {
            map.lost_sight.push((object, observer));
        }
    }
    map.remove_observer(object, observer);
}

/// The create blocks a player gets for themselves and their items, sent at
/// once (at login, and after a worldport).
pub fn send_self(map: &mut Map, guid: Guid) {
    // Aboard a transport on the map: its create first, in the same packet,
    // so the client puts the player on its deck (mangos's `SendInitSelf`);
    // the player's own says where on it.
    let aboard = map
        .players
        .get(&guid)
        .and_then(|p| p.movement.flags.get_on_transport())
        .map(|t| t.transport.guid)
        .filter(|t| map.gameobject(*t).is_some());
    let transport = aboard.and_then(|t| Some((t, map.public_create(t)?)));
    if let Some((t, _)) = transport {
        map.add_observer(t, guid);
    }
    let Some(p) = map.players.get_mut(&guid) else {
        return;
    };
    let mut batch = update::Batch::new();
    if let Some((t, block)) = &transport {
        batch.push_for(*t, block);
        p.known.insert(*t);
    }
    for item in &p.items {
        batch.push(&update::create_block(&item.create(), Visibility::Owner));
    }
    batch.push(&update::create_block(&p.create(true), Visibility::Owner));
    for frame in batch.finish() {
        p.outbound.send_frame(frame);
    }
    // The client has every field now; nobody else knows the player yet.
    p.fields.clear_dirty();
    for item in &mut p.items {
        item.fields.clear_dirty();
    }
}

#[cfg(test)]
mod chunk_tests {
    use super::*;

    /// A chunk is as many refreshes as the budget left has room for, at
    /// what one cost, between the least and the most; with no budget, or
    /// no cost known, a whole chunk.
    #[test]
    fn a_chunk_is_sized_to_the_budget_left() {
        let ms = Duration::from_millis;
        let us = Duration::from_micros;
        assert_eq!(chunk_size(ms(10), ms(0), Duration::ZERO), CHUNK);
        assert_eq!(chunk_size(Duration::ZERO, ms(0), us(500)), CHUNK);
        assert_eq!(chunk_size(ms(10), ms(0), us(10)), CHUNK);
        assert_eq!(chunk_size(ms(10), ms(0), us(200)), 50);
        assert_eq!(chunk_size(ms(10), ms(6), us(100)), 40);
        assert_eq!(chunk_size(ms(10), ms(0), ms(2)), 5);
        assert_eq!(chunk_size(ms(10), ms(0), ms(30)), MIN_CHUNK);
        assert_eq!(chunk_size(ms(10), ms(12), us(100)), MIN_CHUNK);
    }
}
