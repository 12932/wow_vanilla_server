use arc_swap::ArcSwapOption;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wow_world_base::movement::{DEFAULT_RUNNING_SPEED, DEFAULT_TURN_SPEED};
use wow_world_base::vanilla::position::{position, Position, PositionIdentifier};
use wow_world_base::vanilla::Map;
use wow_world_messages::vanilla::UpdateMask;
use wow_world_messages::vanilla::{
    MovementBlock, MovementBlock_MovementFlags, MovementBlock_UpdateFlag,
    MovementBlock_UpdateFlag_Living, MovementInfo, Object, ObjectType, Object_UpdateType,
    UpdateUnitBuilder, Vector3d,
};
use wow_world_messages::Guid;

// Creature health / corpse / respawn knobs live in `[respawn]` of
// `config.toml`. Helpers below read them from the global config.

pub fn default_creature_health() -> u32 {
    crate::config::config().respawn.default_creature_health
}

/// How long a corpse stays visible after death before it gets despawned.
pub fn corpse_despawn() -> Duration {
    crate::config::config().respawn.corpse_despawn()
}

/// Initial respawn delay applied to every freshly-spawned creature. After
/// the first death, this halves on every kill where the mob lived for less
/// than the current respawn delay — a "the faster you kill it, the faster it
/// comes back" feedback loop with no floor (can reach sub-second).
pub fn initial_respawn_delay() -> Duration {
    crate::config::config().respawn.initial_respawn_delay()
}

/// Where a creature is in its life cycle.
#[derive(Debug, Clone, Copy)]
pub enum CreatureLifeState {
    Alive,
    /// Killed; the body is still visible in-world until `CORPSE_DESPAWN`
    /// elapses from `died_at`. Not iterated by the AI ticks.
    Corpse { died_at: Instant },
    /// Corpse has decayed; waiting until `respawn_at` to come back alive.
    Respawning { respawn_at: Instant },
}

/// Creature walking speed, served from `[creature] walk_speed` in
/// config (default 2.0 yd/s). `wow_world_base`'s canonical vanilla
/// walking is 2.5 but that looks like a brisk march for patrolling
/// guards — 2.0 is the sweet spot.
pub fn walk_speed() -> f32 {
    crate::config::config().creature.walk_speed
}

#[derive(Debug)]
pub enum CreatureBehavior {
    AggroChase,
    Idle,
    RandomWander {
        anchor: Vector3d,
        radius: f32,
        target: Option<Vector3d>,
        next_decision_at: Instant,
    },
    Waypoint {
        waypoints: Vec<Vector3d>,
        waittimes_ms: Vec<u32>,
        current: usize,
        idle_until: Option<Instant>,
    },
}

#[derive(Debug)]
pub struct Creature {
    pub name: String,
    pub guid: Guid,
    pub info: MovementInfo,
    pub map: Map,
    pub level: u8,
    pub display_id: u16,
    pub entry: u32,
    pub faction_template: u32,
    pub health: u32,
    pub max_health: u32,
    pub root_until: Option<Instant>,
    pub behavior: CreatureBehavior,
    pub last_advanced_at: Instant,
    pub last_heartbeat_at: Instant,
    /// Position the creature returns to on respawn — its original placement.
    /// Distinct from `info.position` which tracks the live (possibly-wandered)
    /// location.
    pub spawn_position: Vector3d,
    pub spawn_orientation: f32,
    pub life_state: CreatureLifeState,
    /// Set on (re)spawn; used to compute "alive duration" at next death.
    pub last_alive_at: Instant,
    /// Dynamic respawn delay. Starts at `INITIAL_RESPAWN_DELAY`, halves every
    /// time the creature dies within its own respawn delay window.
    pub respawn_delay: Duration,

    /// Cache of the wire-format `CreateObject2` for this creature.
    /// Built lazily on first observer-AOI entry; cleared by every
    /// mutator that affects the resulting wire bits (position,
    /// orientation, flags, health, display_id, level, faction, entry).
    /// Idle creatures cache forever across ticks; walkers re-bake
    /// roughly per move tick. The `Arc<Object>` is share-counted into
    /// the global AoI snapshot so cross-cell observers entering AOI of
    /// this creature pay an `Arc::clone` instead of a fresh builder
    /// pass.
    ///
    /// `ArcSwapOption` (not `RefCell`) so `Creature: Sync` — the AoI
    /// diff pass uses rayon `par_iter` and creatures must be sharable
    /// across worker threads.
    pub cached_create_object: ArcSwapOption<Object>,
}

impl Creature {
    pub fn is_rooted(&self) -> bool {
        self.root_until
            .map(|t| t > Instant::now())
            .unwrap_or(false)
    }

    pub fn new(name: impl Into<String>, guid: Guid) -> Self {
        Self::with_display(
            name,
            guid,
            646,
            69,
            position(PositionIdentifier::HumanStartZone),
        )
    }

    pub fn with_display(
        name: impl Into<String>,
        guid: Guid,
        display_id: u16,
        entry: u32,
        position: Position,
    ) -> Self {
        Self {
            name: name.into(),
            guid,
            info: MovementInfo {
                flags: Default::default(),
                timestamp: 0,
                position: Vector3d {
                    x: position.x,
                    y: position.y,
                    z: position.z,
                },
                orientation: position.orientation,
                fall_time: 0.0,
            },
            map: position.map,
            level: 1,
            display_id,
            entry,
            faction_template: 16,
            health: default_creature_health(),
            max_health: default_creature_health(),
            root_until: None,
            behavior: CreatureBehavior::AggroChase,
            last_advanced_at: Instant::now(),
            last_heartbeat_at: Instant::now(),
            spawn_position: Vector3d {
                x: position.x,
                y: position.y,
                z: position.z,
            },
            spawn_orientation: position.orientation,
            life_state: CreatureLifeState::Alive,
            last_alive_at: Instant::now(),
            respawn_delay: initial_respawn_delay(),
            cached_create_object: ArcSwapOption::from(None),
        }
    }

    pub fn position(&self) -> Position {
        Position {
            map: self.map,
            x: self.info.position.x,
            y: self.info.position.y,
            z: self.info.position.z,
            orientation: self.info.orientation,
        }
    }

    fn living_block_flags(&self) -> MovementBlock_MovementFlags {
        if self.info.flags.get_forward() {
            MovementBlock_MovementFlags::new_forward().set_walk_mode()
        } else {
            MovementBlock_MovementFlags::empty()
        }
    }

    /// Return the cached wire-format `CreateObject2`, building (and
    /// caching) it on the first call since the last invalidation.
    /// Returns a shared `Arc<Object>` so multiple observers entering
    /// AOI of this creature in the same tick share one allocation.
    pub fn cached_create_object(&self) -> Arc<Object> {
        if let Some(arc) = self.cached_create_object.load_full() {
            return arc;
        }
        let obj = Arc::new(self.build_create_object());
        self.cached_create_object.store(Some(Arc::clone(&obj)));
        obj
    }

    /// Backwards-compat helper that yields an owned `Object` clone for
    /// callers that need to drop it into a `Vec<Object>` (which the
    /// wire encoder requires by value). Internally still goes through
    /// the cache.
    pub fn to_create_object(&self) -> Object {
        (*self.cached_create_object()).clone()
    }

    /// Drop the cached `CreateObject2`. Must be called by every
    /// mutator that touches a field the wire payload depends on
    /// (position, orientation, flags, health, display_id, level,
    /// faction_template, entry). Missing an invalidation produces a
    /// 1-tick visual snap when an observer enters AOI; not a crash,
    /// but still a bug.
    #[inline]
    pub fn invalidate_object_cache(&self) {
        self.cached_create_object.store(None);
    }

    fn build_create_object(&self) -> Object {
        Object {
            update_type: Object_UpdateType::CreateObject2 {
                guid3: self.guid,
                mask2: UpdateMask::Unit(
                    UpdateUnitBuilder::new()
                        .set_unit_health(i32::try_from(self.health).unwrap_or(i32::MAX))
                        .set_unit_maxhealth(i32::try_from(self.max_health).unwrap_or(i32::MAX))
                        .set_object_guid(self.guid)
                        .set_unit_displayid(self.display_id.into())
                        .set_object_scale_x(1.0)
                        .set_unit_level(self.level.into())
                        .set_unit_factiontemplate(self.faction_template as i32)
                        .set_object_entry(self.entry as i32)
                        .finalize(),
                ),
                movement2: MovementBlock {
                    update_flag: MovementBlock_UpdateFlag::new_living(
                        MovementBlock_UpdateFlag_Living::Living {
                            backwards_running_speed: 0.0,
                            backwards_swimming_speed: 0.0,
                            fall_time: 0.0,
                            flags: self.living_block_flags(),
                            living_orientation: self.info.orientation,
                            living_position: self.info.position,
                            running_speed: DEFAULT_RUNNING_SPEED,
                            swimming_speed: 0.0,
                            timestamp: 0,
                            turn_rate: DEFAULT_TURN_SPEED,
                            walking_speed: walk_speed(),
                        },
                    ),
                },
                object_type: ObjectType::Unit,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_create_object_returns_same_arc_on_consecutive_calls() {
        // Cache hit must yield the exact same allocation — not a
        // semantically-equal new Object. We compare Arc pointers
        // directly: a fresh build would produce a distinct allocation.
        let c = Creature::new("test", Guid::new(1));
        let a = c.cached_create_object();
        let b = c.cached_create_object();
        assert!(
            Arc::ptr_eq(&a, &b),
            "second call should hit the cache, not rebuild",
        );
    }

    #[test]
    fn invalidate_object_cache_forces_rebuild() {
        // After invalidation the next call must produce a fresh
        // allocation (distinct Arc pointer). Mutation sites rely on
        // this contract to make new state observable.
        let c = Creature::new("test", Guid::new(1));
        let a = c.cached_create_object();
        c.invalidate_object_cache();
        let b = c.cached_create_object();
        assert!(
            !Arc::ptr_eq(&a, &b),
            "invalidation should drop the cached Arc",
        );
    }

    #[test]
    fn fresh_creature_has_no_cached_object() {
        // A freshly-built Creature must not pre-populate the cache —
        // first call lazy-builds. Guards against accidental
        // pre-baking in `Creature::new` / `with_display`.
        let c = Creature::new("test", Guid::new(1));
        assert!(
            c.cached_create_object.load_full().is_none(),
            "cache must start empty so lazy build path is exercised",
        );
    }

    #[test]
    fn is_rooted_returns_false_when_root_until_is_none() {
        let c = Creature::new("test", Guid::new(1));
        assert!(!c.is_rooted());
    }

    #[test]
    fn is_rooted_returns_false_for_past_root_until() {
        let mut c = Creature::new("test", Guid::new(1));
        c.root_until = Instant::now().checked_sub(Duration::from_secs(1));
        assert!(!c.is_rooted());
    }

    #[test]
    fn is_rooted_returns_true_for_future_root_until() {
        let mut c = Creature::new("test", Guid::new(1));
        c.root_until = Some(Instant::now() + Duration::from_secs(5));
        assert!(c.is_rooted());
    }
}
