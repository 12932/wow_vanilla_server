//! The open world's respawn times, kept across a restart, as mangos's
//! `creature_respawn` and `gameobject_respawn` keep them for every map,
//! the continents too (`MapPersistentState::SaveCreatureRespawnTime`): a
//! world boss, a rare or a chest that is down when the realm stops is
//! still down when it starts, until its time. A copy's are in its own save
//! ([`crate::game::instances::InstanceSave`]).
//!
//! Each is a record of its own, keyed by its map and spawn, written as it
//! goes and deleted as it comes back; they are read once, at start-up,
//! and a map takes its own as it starts (a continent the supervisor builds
//! again too).

use std::collections::BTreeMap;

use ahash::AHashMap;
use serde::{Deserialize, Serialize};

use crate::game::Realm;

/// A creature's: when its spawn comes back.
pub const KIND_CREATURE: &str = "creature_respawn";
/// A game object's.
pub const KIND_OBJECT: &str = "object_respawn";

/// A record: when (Unix seconds) the spawn comes back.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Respawn {
    pub at: i64,
}

/// What of the open world is down, by (map, spawn): when each comes back.
#[derive(Debug, Default)]
pub struct World {
    pub creatures: AHashMap<(u32, u32), i64>,
    pub objects: AHashMap<(u32, u32), i64>,
    /// Records found past at start-up, for the first sweep to delete.
    pub stale: Vec<(&'static str, u64)>,
}

fn id(map: u32, spawn: u32) -> u64 {
    (u64::from(map) << 32) | u64::from(spawn)
}

impl World {
    /// Those still to come back at `now`; the rest are past, and are
    /// named in [`World::stale`].
    pub fn load(records: &[(String, u64, String)], now: i64) -> Self {
        let mut w = Self::default();
        for (kind, id, data) in records {
            let (into, kind) = match kind.as_str() {
                KIND_CREATURE => (&mut w.creatures, KIND_CREATURE),
                KIND_OBJECT => (&mut w.objects, KIND_OBJECT),
                _ => continue,
            };
            let Some(r) = crate::game::store::parse_record::<Respawn>(kind, *id, data) else {
                continue;
            };
            if r.at <= now {
                w.stale.push((kind, *id));
                continue;
            }
            into.insert(((id >> 32) as u32, *id as u32), r.at);
        }
        w
    }

    /// Map `map`'s, by spawn: its creatures', then its objects'.
    pub fn of(&self, map: u32) -> (BTreeMap<u32, i64>, BTreeMap<u32, i64>) {
        let pick = |m: &AHashMap<(u32, u32), i64>| {
            m.iter()
                .filter(|((m, _), _)| *m == map)
                .map(|((_, s), at)| (*s, *at))
                .collect()
        };
        (pick(&self.creatures), pick(&self.objects))
    }
}

/// A spawn of the open world went (`object`: a game object), back at
/// `at` (Unix seconds).
pub fn gone(realm: &Realm, object: bool, map: u32, spawn: u32, at: i64) {
    let kind = {
        let mut inst = realm.social.instances.lock();
        if object {
            inst.world.objects.insert((map, spawn), at);
            KIND_OBJECT
        } else {
            inst.world.creatures.insert((map, spawn), at);
            KIND_CREATURE
        }
    };
    realm
        .store
        .put_record(kind, id(map, spawn), &Respawn { at });
}

/// A spawn of the open world is back.
pub fn back(realm: &Realm, object: bool, map: u32, spawn: u32) {
    let (had, kind) = {
        let mut inst = realm.social.instances.lock();
        if object {
            (inst.world.objects.remove(&(map, spawn)), KIND_OBJECT)
        } else {
            (inst.world.creatures.remove(&(map, spawn)), KIND_CREATURE)
        }
    };
    if had.is_some() {
        realm.store.delete_record(kind, id(map, spawn));
    }
}

/// The records found past at start-up go from the store.
pub fn sweep(realm: &Realm) {
    let stale = std::mem::take(&mut realm.social.instances.lock().world.stale);
    for (kind, id) in stale {
        realm.store.delete_record(kind, id);
    }
}
