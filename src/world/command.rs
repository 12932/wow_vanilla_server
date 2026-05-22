//! World-mutating commands queued during opcode/GM handling and applied at the
//! end of each tick.
//!
//! Handlers do not mutate `World.creatures` directly; they push a
//! [`WorldCommand`] onto a [`CommandQueue`]. `World::tick` drains the queue
//! in `apply_commands`, which is the single place that performs spawns /
//! kills and the matching broadcasts.
//!
//! Why a command bus rather than direct mutation?
//! - Removes the parallel `pending_*` `Vec` arguments that used to thread
//!   through every handler signature.
//! - Lets future systems (scripting, instance manager, AI) produce commands
//!   without touching the tick loop.
//! - One place to add per-command Tracy zones, telemetry, batching, etc.

use crate::world::world_opcode_handler::creature::Creature;
use std::time::Instant;
use wow_world_messages::Guid;

/// Cell-agnostic state change applied to a unit by guid. Used by
/// `Entities::apply_effect` so handlers can mutate cross-cell
/// targets without knowing which cell owns them — the dispatcher
/// either applies locally (mutates the slab directly) or queues a
/// `CrossCellEffect` to the target's home cell inbox.
///
/// Add a variant here for every new effect type; the rest of the
/// plumbing (cross-cell routing + drain) reuses the same path.
#[derive(Debug, Clone)]
pub enum UnitEffect {
    /// Server-side root until `until`. Stops the unit from moving +
    /// drops incoming `MSG_MOVE_*` opcodes from real-client targets
    /// until the timer expires.
    Root { until: Instant },
    /// Subtract `amount` from the target's health (saturating). The
    /// receiving side checks `health == 0` after applying and
    /// queues a `KillCreature` command if so — the AoI broadcast +
    /// loot / corpse plumbing already exists in `apply_commands`.
    Damage { amount: u32 },
}

#[derive(Debug)]
// `SpawnCreature` dwarfs `KillCreature` size-wise; boxing it would add a
// heap alloc per spawn and the queue is short-lived (drained every tick),
// so we just eat the variant-size warning.
#[allow(clippy::large_enum_variant)]
pub enum WorldCommand {
    /// Spawn a new creature into the world and broadcast its create-object
    /// to viewers in AOI. Used by `.spawn` and the worlddb load path.
    SpawnCreature(Creature),
    /// Despawn the creature with the given guid (if alive) and broadcast a
    /// destroy to viewers in AOI.
    KillCreature(Guid),
}

#[derive(Debug, Default)]
pub struct CommandQueue {
    cmds: Vec<WorldCommand>,
}

impl CommandQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, cmd: WorldCommand) {
        self.cmds.push(cmd);
    }

    pub fn drain(&mut self) -> std::vec::Drain<'_, WorldCommand> {
        self.cmds.drain(..)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.cmds.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wow_world_messages::Guid;

    #[test]
    fn new_queue_is_empty() {
        let q = CommandQueue::new();
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn push_then_drain_returns_in_order() {
        let mut q = CommandQueue::new();
        q.push(WorldCommand::KillCreature(Guid::new(1)));
        q.push(WorldCommand::KillCreature(Guid::new(2)));
        q.push(WorldCommand::KillCreature(Guid::new(3)));
        assert_eq!(q.len(), 3);
        let collected: Vec<_> = q
            .drain()
            .map(|c| match c {
                WorldCommand::KillCreature(g) => g.guid(),
                _ => panic!("unexpected variant"),
            })
            .collect();
        assert_eq!(collected, vec![1, 2, 3]);
        assert!(q.is_empty());
    }
}
