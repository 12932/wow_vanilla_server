use crate::world::world::client::Client;
use crate::world::world::PendingMovement;
use crate::world::world_opcode_handler::creature::Creature;
use crate::world::world_opcode_handler::simulated_player::SimulatedPlayer;
use ahash::AHashMap;
use slab::Slab;
use wow_world_base::shared::Guid;
use wow_world_base::vanilla::position::Position;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;

pub(crate) enum Entity<'a> {
    Player(&'a Client),
    Creature(&'a Creature),
    Simulated(&'a SimulatedPlayer),
}

#[derive(Debug)]
pub(crate) struct Entities<'a> {
    clients: &'a mut Slab<Client>,
    creatures: &'a mut Slab<Creature>,
    creature_by_guid: &'a AHashMap<Guid, usize>,
    simulated_players: &'a mut Slab<SimulatedPlayer>,
    simulated_by_guid: &'a AHashMap<Guid, usize>,
    pending_movement: &'a mut AHashMap<Guid, PendingMovement>,
}

impl<'a> Entities<'a> {
    pub(crate) fn new(
        clients: &'a mut Slab<Client>,
        creatures: &'a mut Slab<Creature>,
        creature_by_guid: &'a AHashMap<Guid, usize>,
        simulated_players: &'a mut Slab<SimulatedPlayer>,
        simulated_by_guid: &'a AHashMap<Guid, usize>,
        pending_movement: &'a mut AHashMap<Guid, PendingMovement>,
    ) -> Self {
        Self {
            clients,
            creatures,
            creature_by_guid,
            simulated_players,
            simulated_by_guid,
            pending_movement,
        }
    }

    /// Queue a movement opcode broadcast for the source player. Replaces any
    /// previously-queued opcode from the same source this tick — the latest
    /// `MovementInfo` is the authoritative state for observers, and we'd
    /// rather emit one broadcast per source per tick than one per opcode.
    pub(crate) fn queue_movement(&mut self, source: &Client, msg: ServerOpcodeMessage) {
        let character = source.character();
        let anchor = character.info.position;
        let map = character.map;
        self.pending_movement.insert(
            character.guid,
            PendingMovement { msg, anchor, map },
        );
    }

    pub(crate) fn clients(&mut self) -> &mut Slab<Client> {
        self.clients
    }

    pub(crate) fn creatures(&mut self) -> &mut Slab<Creature> {
        self.creatures
    }

    pub(crate) fn simulated_players(&mut self) -> &mut Slab<SimulatedPlayer> {
        self.simulated_players
    }

    pub(crate) fn find_guid(&self, guid: Guid) -> Option<Entity<'_>> {
        if let Some(c) = self.find_player(guid) {
            Some(Entity::Player(c))
        } else if let Some(c) = self.find_creature(guid) {
            Some(Entity::Creature(c))
        } else {
            self.find_simulated(guid).map(Entity::Simulated)
        }
    }

    pub(crate) fn find_player(&self, guid: Guid) -> Option<&Client> {
        self.clients
            .iter()
            .find_map(|(_, c)| (c.character().guid == guid).then_some(c))
    }

    pub(crate) fn find_creature(&self, guid: Guid) -> Option<&Creature> {
        let key = *self.creature_by_guid.get(&guid)?;
        self.creatures.get(key)
    }

    pub(crate) fn find_simulated(&self, guid: Guid) -> Option<&SimulatedPlayer> {
        let key = *self.simulated_by_guid.get(&guid)?;
        self.simulated_players.get(key)
    }

    pub(crate) fn find_position(&self, guid: Guid) -> Option<Position> {
        self.find_guid(guid).map(|c| match c {
            Entity::Player(c) => c.position(),
            Entity::Creature(c) => c.position(),
            Entity::Simulated(s) => Position {
                map: s.map,
                x: s.info.position.x,
                y: s.info.position.y,
                z: s.info.position.z,
                orientation: s.info.orientation,
            },
        })
    }
}
