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

pub const DEFAULT_CREATURE_HEALTH: u32 = 5000;

/// How long a corpse stays visible after death before it gets despawned.
pub const CORPSE_DESPAWN: Duration = Duration::from_secs(180);

/// Initial respawn delay applied to every freshly-spawned creature. After
/// the first death, this halves on every kill where the mob lived for less
/// than the current respawn delay — a "the faster you kill it, the faster it
/// comes back" feedback loop with no floor (can reach sub-second).
pub const INITIAL_RESPAWN_DELAY: Duration = Duration::from_secs(180);

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

/// `wow_world_base::movement::DEFAULT_WALKING_SPEED` is 1.0 yd/s, which looks
/// unnaturally slow in-client. Canonical vanilla walking is 2.5, but that
/// looks like a brisk march for patrolling guards — 2.0 is the sweet spot.
pub const WALK_SPEED: f32 = 2.0;

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
            health: DEFAULT_CREATURE_HEALTH,
            max_health: DEFAULT_CREATURE_HEALTH,
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
            respawn_delay: INITIAL_RESPAWN_DELAY,
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

    pub fn to_create_object(&self) -> Object {
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
                            walking_speed: WALK_SPEED,
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
