//! World-mutating commands queued during opcode/GM handling and applied at the
//! end of each tick.
//!
//! Handlers do not mutate `World.creatures`, `World.simulated_players`, etc.
//! directly; they push a [`WorldCommand`] onto a [`CommandQueue`]. `World::tick`
//! drains the queue in `apply_commands`, which is the single place that
//! performs spawns / kills / sim instantiation and the matching broadcasts.
//!
//! Why a command bus rather than direct mutation?
//! - Removes the four parallel `pending_*` `Vec` arguments that used to thread
//!   through every handler signature.
//! - Lets future systems (scripting, instance manager, AI) produce commands
//!   without touching the tick loop.
//! - One place to add per-command Tracy zones, telemetry, batching, etc.

use crate::world::world_opcode_handler::creature::Creature;
use crate::world::world_opcode_handler::simulated_player::SimulatedPlayer;
use wow_world_messages::Guid;

#[derive(Debug)]
pub enum WorldCommand {
    /// Spawn a new creature into the world and broadcast its create-object
    /// to viewers in AOI. Used by `.spawn` and the worlddb load path.
    SpawnCreature(Creature),
    /// Despawn the creature with the given guid (if alive) and broadcast a
    /// destroy to viewers in AOI.
    KillCreature(Guid),
    /// Spawn a server-side puppet horde player and broadcast its
    /// create-object + `MSG_MOVE_START_FORWARD` to viewers in AOI.
    SpawnSimulant(SimulatedPlayer),
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
