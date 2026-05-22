use crate::world::aoi::{BroadcastTarget, CreatureView, GlobalAoiSnapshot};
use crate::world::command::UnitEffect;
use crate::world::world::client::Client;
use crate::world::world::PendingMovement;
use crate::world::world_opcode_handler::creature::Creature;
use ahash::AHashMap;
use slab::Slab;
use wow_world_base::shared::Guid;
use wow_world_base::vanilla::position::Position;
use wow_world_base::vanilla::Map;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::Vector3d;

pub(crate) enum Entity<'a> {
    Player(&'a Client),
    Creature(&'a Creature),
}

#[derive(Debug)]
pub(crate) struct Entities<'a> {
    clients: &'a mut Slab<Client>,
    client_by_guid: &'a AHashMap<Guid, usize>,
    creatures: &'a mut Slab<Creature>,
    creature_by_guid: &'a AHashMap<Guid, usize>,
    pending_movement: &'a mut AHashMap<Guid, PendingMovement>,
    /// Cross-cell view built once per tick. Lets `creatures_in_radius`
    /// / `clients_in_radius` / `apply_effect` span cells transparently.
    aoi_snapshot: &'a GlobalAoiSnapshot,
}

impl<'a> Entities<'a> {
    pub(crate) fn new(
        clients: &'a mut Slab<Client>,
        client_by_guid: &'a AHashMap<Guid, usize>,
        creatures: &'a mut Slab<Creature>,
        creature_by_guid: &'a AHashMap<Guid, usize>,
        pending_movement: &'a mut AHashMap<Guid, PendingMovement>,
        aoi_snapshot: &'a GlobalAoiSnapshot,
    ) -> Self {
        Self {
            clients,
            client_by_guid,
            creatures,
            creature_by_guid,
            pending_movement,
            aoi_snapshot,
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

    pub(crate) fn find_guid(&self, guid: Guid) -> Option<Entity<'_>> {
        if let Some(c) = self.find_player(guid) {
            Some(Entity::Player(c))
        } else {
            self.find_creature(guid).map(Entity::Creature)
        }
    }

    pub(crate) fn find_player(&self, guid: Guid) -> Option<&Client> {
        let key = *self.client_by_guid.get(&guid)?;
        self.clients.get(key)
    }

    pub(crate) fn find_player_mut(&mut self, guid: Guid) -> Option<&mut Client> {
        let key = *self.client_by_guid.get(&guid)?;
        self.clients.get_mut(key)
    }

    pub(crate) fn find_creature(&self, guid: Guid) -> Option<&Creature> {
        let key = *self.creature_by_guid.get(&guid)?;
        self.creatures.get(key)
    }

    /// Locate any entity by guid, regardless of which cell it
    /// lives in. Reads from the global AoI snapshot, so the
    /// position is one tick stale — acceptable for combat range
    /// checks (~0.2 yd creature drift per tick at run speed).
    /// Returns `None` for unknown guids or if no snapshot is wired
    /// (e.g. tests that bypass `build_global_aoi_snapshot`).
    ///
    /// Currently unused — combat reads the snapshot directly to
    /// avoid extending the `entities` mut borrow during the swing
    /// block. Kept as the documented public API for handlers that
    /// don't have the borrow-conflict problem.
    #[allow(dead_code)]
    pub(crate) fn locate_entity(&self, guid: Guid) -> Option<crate::world::aoi::EntityLocation> {
        self.aoi_snapshot.entity_locations.get(&guid).copied()
    }

    pub(crate) fn find_position(&self, guid: Guid) -> Option<Position> {
        self.find_guid(guid).map(|c| match c {
            Entity::Player(c) => c.position(),
            Entity::Creature(c) => c.position(),
        })
    }

    /// All creatures within `radius` of `center` on `map`, across the
    /// whole world (not just the local cell). Reads from the global
    /// AoI snapshot built once at the top of `World::tick`, so this is
    /// O(grid_cells × creatures_per_grid_cell). For a 14-yd nova radius
    /// that's a single 250-yd cell window — handful of comparisons.
    pub(crate) fn creatures_in_radius(
        &self,
        center: Vector3d,
        map: Map,
        radius: f32,
    ) -> Vec<CreatureView> {
        use crate::world::world::CREATURE_GRID_CELL_YD;
        let r_sq = radius * radius;
        let grid_cell_x = (center.x / CREATURE_GRID_CELL_YD).floor() as i32;
        let grid_cell_y = (center.y / CREATURE_GRID_CELL_YD).floor() as i32;
        // 3×3 cell window — same as the AoI diff scan. Sufficient for
        // any radius up to one cell (250 yd); larger radii would need
        // a wider window. Frost nova / .swifty / any near-melee AoE
        // is well inside this.
        let mut out = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(views) =
                    self.aoi_snapshot.creature_grid_cells.get(&(map, grid_cell_x + dx, grid_cell_y + dy))
                else {
                    continue;
                };
                for view in views {
                    let ddx = view.position.x - center.x;
                    let ddy = view.position.y - center.y;
                    let ddz = view.position.z - center.z;
                    if ddx * ddx + ddy * ddy + ddz * ddz <= r_sq {
                        out.push(*view);
                    }
                }
            }
        }
        out
    }

    /// All clients within `radius` of `center` on `map`, across the
    /// whole world. Returns `BroadcastTarget` so the caller can both
    /// identify by guid AND send packets through the same handle.
    pub(crate) fn clients_in_radius(
        &self,
        center: Vector3d,
        map: Map,
        radius: f32,
    ) -> Vec<&BroadcastTarget> {
        let r_sq = radius * radius;
        self.aoi_snapshot
            .broadcast_view
            .iter()
            .filter(|t| t.map == map)
            .filter(|t| {
                let dx = t.position.x - center.x;
                let dy = t.position.y - center.y;
                let dz = t.position.z - center.z;
                dx * dx + dy * dy + dz * dz <= r_sq
            })
            .collect()
    }

    /// Apply a state change to a unit by guid. Routes transparently:
    /// - If the target lives in the local cell (i.e. in this cell's
    ///   `clients` / `creatures` slab), the effect is applied
    ///   immediately by mutating the slab.
    /// - If the target lives in a neighbor cell, a
    ///   `CrossCellEffect` is dispatched through the routing table
    ///   to the target's cell inbox. The receiving cell drains
    ///   the effect during its next tick (~33 ms lag at 30 Hz).
    ///
    /// Returns the outcome so callers can react to LOCAL deaths
    /// (e.g. push `WorldCommand::KillCreature` for the corpse +
    /// loot pipeline). Cross-cell kills are detected by the
    /// neighbor's drain code and handled there.
    pub(crate) fn apply_effect(&mut self, guid: Guid, effect: UnitEffect) -> ApplyEffectResult {
        // Local: creatures.
        if let Some(&ck) = self.creature_by_guid.get(&guid)
            && let Some(cr) = self.creatures.get_mut(ck)
        {
            let died = apply_effect_to_creature(cr, &effect);
            return ApplyEffectResult::AppliedLocally { creature_died: died };
        }
        // Local: clients.
        if let Some(&pk) = self.client_by_guid.get(&guid)
            && let Some(c) = self.clients.get_mut(pk)
        {
            apply_effect_to_client(c, &effect);
            return ApplyEffectResult::AppliedLocally { creature_died: false };
        }
        // Cross-cell: route to the target's home cell inbox.
        let Some(&home) = self.aoi_snapshot.home_cell_by_guid.get(&guid) else {
            return ApplyEffectResult::Unknown;
        };
        let table = crate::world::cell::routing().load();
        let Some(inbox) = table.inboxes.get(&home) else {
            return ApplyEffectResult::Unknown;
        };
        let msg = crate::world::cell::CrossCellMsg::Effect(
            crate::world::cell::CrossCellEffect {
                target_guid: guid,
                effect,
            },
        );
        if inbox.cross_cell_tx.try_send(msg).is_ok() {
            ApplyEffectResult::QueuedCrossCell
        } else {
            ApplyEffectResult::Unknown
        }
    }
}

/// Outcome of [`Entities::apply_effect`]. The handler uses
/// `AppliedLocally { creature_died: true }` to queue a follow-up
/// `KillCreature` command (corpse / loot / AoI broadcast already
/// plumbed in `apply_commands`). Cross-cell kills are detected
/// inside the neighbor cell's effect-inbox drain.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ApplyEffectResult {
    /// Effect mutated the local slab. `creature_died` is true if
    /// the target was a creature whose health hit zero from this
    /// effect.
    AppliedLocally { creature_died: bool },
    /// Effect was sent to the target's home cell inbox.
    QueuedCrossCell,
    /// Target guid not found anywhere (stale lookup, target
    /// logged out / despawned between snapshot build and call).
    Unknown,
}

/// Apply a `UnitEffect` to a creature directly. Only called when the
/// creature is owned by the local cell (so we have the mutable
/// borrow). Used by `Entities::apply_effect` and by the per-cell
/// effect-inbox drain.
///
/// Returns `true` if the creature died from this effect (health
/// dropped to zero). Caller uses this to push a follow-up
/// `WorldCommand::KillCreature` for the corpse + loot + AoI
/// broadcast pipeline.
pub(crate) fn apply_effect_to_creature(cr: &mut Creature, effect: &UnitEffect) -> bool {
    use wow_world_messages::vanilla::MovementInfo_MovementFlags;
    match effect {
        UnitEffect::Root { until } => {
            cr.root_until = Some(*until);
            // Freeze authoritative movement state so the next broadcast
            // emits a stopped unit. Caller is responsible for emitting
            // the MSG_MOVE_STOP_Server and the visual aura, since those
            // need cell-context (broadcast_within_aoi).
            cr.info.flags = MovementInfo_MovementFlags::default();
            false
        }
        UnitEffect::Damage { amount } => {
            cr.health = cr.health.saturating_sub(*amount);
            cr.health == 0
        }
    }
}

/// Apply a `UnitEffect` to a client. Mirrors `apply_effect_to_creature`.
/// Damage on a client is a no-op for now — player-vs-player damage
/// goes through the dedicated combat path (`SMSG_ATTACKERSTATEUPDATE`)
/// which already exists; routing player damage through this generic
/// path would skip melee leeway / parry / dodge resolution.
pub(crate) fn apply_effect_to_client(c: &mut Client, effect: &UnitEffect) {
    use wow_world_messages::vanilla::MovementInfo_MovementFlags;
    match effect {
        UnitEffect::Root { until } => {
            c.character_mut().info.flags = MovementInfo_MovementFlags::default();
            c.character_mut().root_until = Some(*until);
        }
        UnitEffect::Damage { .. } => {
            // Intentional no-op — see doc.
        }
    }
}
