//! A crowd for the visibility benchmark (`benches/visibility.rs`) and the
//! tests that compare its ways of working: N players on a disc, all in
//! sight of each other, as the movement drill has them (docs/PERFORMANCE.md),
//! in a map of its own on an empty world, with no task running it.
//! Not part of the server.

use std::sync::Arc;
use std::sync::atomic::AtomicU8;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::conn::{Frames, Outbound};
use crate::game::data::GameData;
use crate::game::map::player::Player;
use crate::game::map::{Map, MapHandle, MapKey, grid, visibility};
use crate::game::registry::{NameEntry, Online};
use crate::game::store::{CharacterRow, Store};
use crate::game::{Realm, RealmConfig, guid};

/// The disc's middle: inside a cell, away from the grid's origin.
pub const CENTRE: (f32, f32) = (1_025.0, 1_025.0);

pub struct Crowd {
    pub map: Map,
    pub guids: Vec<Guid>,
    /// Each player's queue, emptied after each refresh so that nothing
    /// piles up.
    frames: Vec<Frames>,
    /// Which way the crowd goes next: east, then back west.
    east: bool,
}

/// A spot on a disc of `radius` for the `i`th of `n`: a sunflower's
/// spiral, even over the disc and the same every run.
fn spot(i: usize, n: usize, radius: f32) -> (f32, f32) {
    let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
    let r = radius * ((i as f32 + 0.5) / n as f32).sqrt();
    let a = i as f32 * golden;
    (CENTRE.0 + r * a.cos(), CENTRE.1 + r * a.sin())
}

impl Crowd {
    /// `n` players on a disc of `radius` yards, each knowing all the others
    /// (a first refresh done, its creates sent and dropped). Call within a
    /// tokio runtime.
    pub fn new(n: usize, radius: f32) -> Self {
        Self::with_config(RealmConfig::default(), n, radius)
    }

    /// [`Crowd::new`] in a realm of `cfg`.
    pub fn with_config(cfg: RealmConfig, n: usize, radius: f32) -> Self {
        let (store, startup) = Store::open(":memory:").expect("an in-memory store");
        let realm = Realm::new(cfg, store, startup, GameData::empty(), None);
        Self::in_realm(&realm, n, radius)
    }

    pub fn in_realm(realm: &Arc<Realm>, n: usize, radius: f32) -> Self {
        let key = MapKey::world(1);
        let map = Map::new(key, realm.clone(), MapHandle::detached(key));
        let mut crowd = Self {
            map,
            guids: Vec::new(),
            frames: Vec::new(),
            east: true,
        };
        crowd.add(n, radius);
        crowd
    }

    /// `n` more players, a hundred at a time, each hundred's creates
    /// dropped before the next: a crowd of thousands never holds millions
    /// of creates at once.
    fn add(&mut self, n: usize, radius: f32) {
        let total = self.guids.len() + n;
        while self.guids.len() < total {
            let k = (total - self.guids.len()).min(100);
            self.add_some(k, total, radius);
            self.refresh();
        }
    }

    /// `k` more players at the next of the `n` spots.
    fn add_some(&mut self, k: usize, n: usize, radius: f32) {
        let data = self.map.realm.data.clone();
        let start = self.guids.len();
        for i in start..start + k {
            let map = &mut self.map;
            let id = i as u32 + 1;
            let (x, y) = spot(i, n, radius);
            let entry = NameEntry {
                guid: guid::player(id),
                account: Arc::from(format!("A{id}")),
                name: Arc::from(format!("P{id}")),
                race: 1,
                class: 1,
                gender: 0,
                level: AtomicU8::new(30),
            };
            let (outbound, f) = Outbound::detached();
            let (session, _) = tokio::sync::mpsc::unbounded_channel();
            let online = Arc::new(Online::new(
                &entry,
                0,
                outbound,
                session,
                map.handle.clone(),
            ));
            let row = CharacterRow {
                guid: id,
                account: format!("A{id}"),
                name: format!("P{id}"),
                race: 1,
                class: 1,
                level: 30,
                map: 1,
                x,
                y,
                z: 10.0,
                ..CharacterRow::default()
            };
            let p = Player::from_row(&data, row, online, map.now);
            let g = p.guid;
            map.insert_player(p);
            map.visibility_dirty.insert(g);
            self.guids.push(g);
            self.frames.push(f);
        }
    }

    /// Move every `every`th player (from the `from`th) `yards` east or
    /// west, by turns, marking them as movement does.
    pub fn shift(&mut self, every: usize, from: usize, yards: f32) {
        let dx = if self.east { yards } else { -yards };
        self.east = !self.east;
        for &g in self.guids.iter().skip(from).step_by(every.max(1)) {
            move_by(&mut self.map, g, dx, 0.0);
        }
    }

    /// The update's visibility, then what it made sent and dropped
    /// (untimed by a caller that cares: see [`Crowd::drain`]).
    pub fn refresh(&mut self) {
        visibility::refresh_dirty(&mut self.map);
        self.drain();
    }

    /// What the update's visibility made: each observer's creates (its
    /// batch's packets) and forgets (sorted), by observer. Taken off, as
    /// [`Crowd::drain`] would drop them.
    pub fn sent(&mut self) -> Vec<(Guid, Vec<Vec<u8>>, Vec<Guid>)> {
        let mut batches: Vec<_> = std::mem::take(&mut self.map.batches).into_iter().collect();
        batches.sort_unstable_by_key(|(g, _)| *g);
        let mut forget = std::mem::take(&mut self.map.forget);
        let mut out: Vec<(Guid, Vec<Vec<u8>>, Vec<Guid>)> = batches
            .into_iter()
            .map(|(g, b)| {
                let frames = b.finish().iter().map(|f| f.to_vec()).collect();
                (g, frames, forget.remove(&g).unwrap_or_default())
            })
            .collect();
        out.extend(forget.into_iter().map(|(g, f)| (g, Vec::new(), f)));
        out.sort_unstable_by_key(|(g, _, _)| *g);
        // A forget's place in its list is a hash set's order (what was
        // known, who knew it): not the client's concern.
        for (_, _, f) in &mut out {
            f.sort_unstable();
        }
        out
    }

    /// The creates and forgets made, and the clients held for them, let go
    /// and emptied.
    pub fn drain(&mut self) {
        self.map.batches.clear();
        self.map.forget.clear();
        for (_, out) in std::mem::take(&mut self.map.holding) {
            out.release(Vec::new());
        }
        for f in &mut self.frames {
            while f.try_recv().is_some() {}
        }
    }

    /// What the `i`th player's client has been sent since the last look,
    /// each frame's bytes.
    pub fn take_frames(&mut self, i: usize) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| self.frames[i].try_recv())
            .map(|f| f.to_vec())
            .collect()
    }

    /// Every player's known set, sorted: what two ways of working must
    /// agree on.
    pub fn knowledge(&self) -> Vec<(Guid, Vec<Guid>)> {
        let mut all: Vec<(Guid, Vec<Guid>)> = self
            .map
            .players
            .values()
            .map(|p| {
                let mut k: Vec<Guid> = p.known.iter().copied().collect();
                k.sort_unstable();
                (p.guid, k)
            })
            .collect();
        all.sort_unstable();
        all
    }
}

/// Move player `g` by `(dx, dy)`, as a movement packet does: its position,
/// its cell and its mark for visibility.
pub fn move_by(map: &mut Map, g: Guid, dx: f32, dy: f32) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let mut at = p.movement.position;
    at.x += dx;
    at.y += dy;
    move_to(map, g, at);
}

/// Put player `g` at `at`, as a movement packet does.
pub fn move_to(map: &mut Map, g: Guid, at: Vector3d) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    p.movement.position = at;
    let (anchor, old) = (p.vis_anchor, p.cell);
    let new = grid::cell_of(at.x, at.y);
    p.cell = new;
    map.grid.relocate(g, old, new);
    visibility::moved(map, g, anchor, at);
}
