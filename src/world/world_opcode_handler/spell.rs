//! Player-cast spell handlers. Only Frost Nova (id 122) is wired today;
//! everything else from `CMSG_CAST_SPELL` is silently dropped at the
//! opcode dispatcher.

use crate::world::world::client::Client;
use crate::world::world_opcode_handler::entities::Entities;
use std::time::{Duration, Instant};
use wow_world_messages::vanilla::{
    MSG_MOVE_STOP_Server, Object, Object_UpdateType, SpellCastTargets, UpdateMask,
    UpdatePlayerBuilder, UpdateUnitBuilder, SMSG_FORCE_MOVE_ROOT, SMSG_SPELL_GO,
    SMSG_SPELL_GO_CastFlags, SMSG_UPDATE_OBJECT,
};

pub const SPELL_FROST_NOVA: u32 = 122;

const RADIUS: f32 = 14.0;
const ROOT_DURATION: Duration = Duration::from_secs(6);

const AFLAG_HARMFUL: u8 = 0x02;
const AFLAG_VISIBLE: u8 = 0x08;
const AFLAG_NOT_CANCELABLE: u8 = 0x20;
const AURA_FLAGS: u8 = AFLAG_HARMFUL | AFLAG_VISIBLE | AFLAG_NOT_CANCELABLE;

/// Cast Frost Nova from `client` against every creature + other client
/// within `RADIUS`. Shared between the `.nova` GM command and the
/// `CMSG_CAST_SPELL` player handler. No mana / cooldown / GCD checks —
/// the rest of the spell system isn't wired up yet.
pub(crate) async fn cast_frost_nova(client: &mut Client, entities: &mut Entities<'_>) {
    let caster_guid = client.character().guid;
    let caster_pos = client.character().info.position;
    let caster_map = client.character().map;
    let root_until = Instant::now() + ROOT_DURATION;

    // Cell-agnostic find: the snapshot spans the whole world, so
    // creatures + clients in neighbor cells across a boundary are
    // returned alongside local ones. Snapshot is one tick stale; at
    // 30 Hz / run speed that's ~0.2 yd of position drift, well under
    // the 14 yd nova radius.
    let creature_hits: Vec<wow_world_messages::Guid> = entities
        .creatures_in_radius(caster_pos, caster_map, RADIUS)
        .into_iter()
        .map(|v| v.guid)
        .collect();
    let client_hits: Vec<wow_world_messages::Guid> = entities
        .clients_in_radius(caster_pos, caster_map, RADIUS)
        .into_iter()
        .map(|t| t.guid)
        .filter(|g| *g != caster_guid)
        .collect();

    // Apply server-side root to every target — local or cross-cell.
    // `apply_effect` routes by guid: local mutation for same-cell
    // targets, queued `CrossCellEffect` for neighbor-cell targets
    // (drained on the target cell's next tick, ~33 ms lag).
    for g in creature_hits.iter().chain(client_hits.iter()) {
        entities.apply_effect(
            *g,
            crate::world::command::UnitEffect::Root { until: root_until },
        );
    }

    // Rooting only stops the SERVER forwarding the target's future move
    // packets. Observing clients keep dead-reckoning the last
    // START_FORWARD they saw until a stop arrives — so a rooted bot
    // visibly keeps running on everyone else's screen. `apply_effect`
    // already zeroed each target's movement flags; broadcast that
    // stopped state so observers halt the unit in place. Collect the
    // frozen MovementInfo first (immutable borrow) so the borrow is
    // released before `broadcast_within_aoi` takes `clients()` mutably.
    //
    // Local targets only: cross-cell roots are applied on the neighbor
    // cell's drain tick, which doesn't emit a stop yet — a rooted bot
    // across a cell boundary will still appear to run. Fine for the
    // Gurubashi loadtest (bots + caster share a cell); revisit if roots
    // need to be correct across boundaries.
    let mut stops: Vec<(wow_world_messages::Guid, _)> = Vec::new();
    for g in &creature_hits {
        if let Some(cr) = entities.find_creature(*g) {
            stops.push((*g, cr.info.clone()));
        }
    }
    for g in &client_hits {
        if let Some(c) = entities.find_player(*g) {
            stops.push((*g, c.character().info.clone()));
        }
    }
    for (guid, info) in stops {
        let pos = info.position;
        let stop = MSG_MOVE_STOP_Server { guid, info };
        // The caster is held outside the slab during their own cast, so
        // `broadcast_within_aoi` (which only reaches clients still in
        // `entities.clients()`) skips them. Send explicitly first — same
        // as the spell-go / aura visuals below — or the caster's own
        // client keeps dead-reckoning the rooted targets running.
        client.send_message(stop.clone()).await;
        crate::world::aoi::broadcast_within_aoi(stop, pos, caster_map, entities.clients()).await;
    }

    let hits: Vec<wow_world_messages::Guid> = creature_hits
        .iter()
        .chain(client_hits.iter())
        .copied()
        .collect();

    // Spell-go visual. `broadcast_within_aoi` already does cross-cell
    // post-fanout, so observers in neighbor cells also see the nova
    // land. Send to caster explicitly since `broadcast_within_aoi`
    // excludes the source from movement fan-out — for spell visuals
    // the caster needs to see it too.
    let spell_go = SMSG_SPELL_GO {
        cast_item: caster_guid,
        caster: caster_guid,
        spell: SPELL_FROST_NOVA,
        flags: SMSG_SPELL_GO_CastFlags::empty(),
        hits: hits.clone(),
        misses: vec![],
        targets: SpellCastTargets::default(),
    };
    client.send_message(spell_go.clone()).await;
    crate::world::aoi::broadcast_within_aoi(
        spell_go,
        caster_pos,
        caster_map,
        entities.clients(),
    )
    .await;

    // Aura visual for every hit. Unit (creature) and player builders
    // are separate update structs in the wire protocol but the
    // broadcast path treats them identically.
    for target_guid in &creature_hits {
        let aura_update = SMSG_UPDATE_OBJECT {
            has_transport: 0,
            objects: vec![Object {
                update_type: Object_UpdateType::Values {
                    guid1: *target_guid,
                    mask1: UpdateMask::Unit(
                        UpdateUnitBuilder::new()
                            .set_unit_aura(SPELL_FROST_NOVA as i32)
                            .set_unit_auraflags(AURA_FLAGS, 0, 0, 0)
                            .set_unit_auralevels(1, 0, 0, 0)
                            .set_unit_auraapplications(1, 0, 0, 0)
                            .finalize(),
                    ),
                },
            }],
        };
        client.send_message(aura_update.clone()).await;
        crate::world::aoi::broadcast_within_aoi(
            aura_update,
            caster_pos,
            caster_map,
            entities.clients(),
        )
        .await;
    }
    for target_guid in &client_hits {
        let aura_update = SMSG_UPDATE_OBJECT {
            has_transport: 0,
            objects: vec![Object {
                update_type: Object_UpdateType::Values {
                    guid1: *target_guid,
                    mask1: UpdateMask::Player(
                        UpdatePlayerBuilder::new()
                            .set_unit_aura(SPELL_FROST_NOVA as i32)
                            .set_unit_auraflags(AURA_FLAGS, 0, 0, 0)
                            .set_unit_auralevels(1, 0, 0, 0)
                            .set_unit_auraapplications(1, 0, 0, 0)
                            .finalize(),
                    ),
                },
            }],
        };
        client.send_message(aura_update.clone()).await;
        crate::world::aoi::broadcast_within_aoi(
            aura_update,
            caster_pos,
            caster_map,
            entities.clients(),
        )
        .await;
    }

    // SMSG_FORCE_MOVE_ROOT is only meaningful to real WoW clients (it
    // locks their movement input). Bots ignore it. For cross-cell
    // rooted clients we can't send it directly — they're in a
    // neighbor cell's slab — so the server-side root takes over via
    // apply_effect. Local rooted clients get the SMSG path too.
    for target_guid in &client_hits {
        if let Some(c) = entities.find_player_mut(*target_guid) {
            let root_msg = SMSG_FORCE_MOVE_ROOT {
                guid: *target_guid,
                counter: 0,
            };
            c.send_message(root_msg).await;
        }
    }
}
