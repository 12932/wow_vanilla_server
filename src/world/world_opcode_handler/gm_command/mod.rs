mod parser;

use crate::world::database::WorldDatabase;
use crate::world::world;
use crate::world::world::client::Client;
use crate::world::world::pathfinding_maps::PathfindingMaps;
use crate::world::world_opcode_handler::creature::Creature;
use crate::world::world_opcode_handler::entities::{Entities, Entity};
use crate::world::world_opcode_handler::gm_command::parser::GmCommand;
use crate::world::world_opcode_handler::item::{award_item, Item};
use std::cell::Cell;
use std::time::{Duration, Instant};
use tracing::info;
use wow_world_base::vanilla::position::Position;
use wow_world_base::vanilla::{HitInfo, SpellSchool, SplineFlag, Vector3d};
use wow_world_messages::vanilla::{
    CompressedMove, CompressedMove_CompressedMoveOpcode, MonsterMove,
    MonsterMove_MonsterMoveType, Object,
    Object_UpdateType, SpellCastTargets, UpdateMask, UpdatePlayerBuilder, UpdateUnitBuilder,
    SMSG_COMPRESSED_MOVES, SMSG_FORCE_MOVE_ROOT, SMSG_FORCE_RUN_SPEED_CHANGE,
    SMSG_SPELLNONMELEEDAMAGELOG, SMSG_SPELL_GO, SMSG_SPELL_GO_CastFlags, SMSG_SPLINE_SET_RUN_SPEED,
    SMSG_UPDATE_OBJECT,
};
use wow_world_messages::Guid;

pub(crate) async fn gm_command(
    client: &mut Client,
    entities: &mut Entities<'_>,
    message: &str,
    db: &mut WorldDatabase,
    maps: &mut PathfindingMaps,
    commands: &mut crate::world::command::CommandQueue,
) {
    let command = match GmCommand::from_player_command(message, client, entities) {
        Ok(e) => e,
        Err(e) => {
            client.send_system_message(e).await;
            return;
        }
    };

    match command {
        GmCommand::WhereAmI => {
            client
                .send_system_message(format!(
                    "You are on '{map}' ({map_int}), x: {x}, y: {y}, z: {z}",
                    map = client.character().map,
                    map_int = client.character().map.as_int(),
                    x = client.character().info.position.x,
                    y = client.character().info.position.y,
                    z = client.character().info.position.z,
                ))
                .await;
        }
        GmCommand::Teleport(p) => {
            world::prepare_teleport(p, client).await;
        }
        GmCommand::SetRunSpeed(speed) => {
            client.character_mut().movement_speed = speed;
            client
                .send_message(SMSG_FORCE_RUN_SPEED_CHANGE {
                    guid: client.character().guid,
                    move_event: 0,
                    speed,
                })
                .await;

            for (_, c) in entities.clients().iter_mut() {
                c.send_message(SMSG_SPLINE_SET_RUN_SPEED {
                    guid: client.character().guid,
                    speed,
                })
                .await;
            }
        }
        GmCommand::Mark { names, p } => {
            use crate::file_utils::append_string_to_file;
            use std::fmt::Write;
            use std::path::Path;

            let mut msg = String::with_capacity(128);

            write!(
                msg,
                "RawPosition::new({}, {}, {}, {}, {}, vec![",
                p.map.as_int(),
                p.x,
                p.y,
                p.z,
                p.orientation,
            )
            .unwrap();

            for name in names {
                write!(msg, "\"{name}\",").unwrap();
            }

            writeln!(
                msg,
                "], ValidVersions::new(false, {tbc}, {vanilla})),",
                tbc = client.character().map.as_int() == 530,
                vanilla = client.character().map.as_int() == 571
                    || client.character().map.as_int() == 530,
            )
            .unwrap();

            info!("{} added {}", client.character().name, msg);
            append_string_to_file(&msg, Path::new("unadded_locations.txt"));

            let msg = format!("You added {}", msg);

            client.send_system_message(msg).await
        }
        GmCommand::RangeToTarget(range) => {
            client
                .send_system_message(format!("Range to target: '{}'", range))
                .await;
        }
        GmCommand::AddItem(item) => {
            const AMOUNT: u8 = 1;

            let item = Item::new(item, client.character().guid, AMOUNT, db);

            award_item(item, client, entities.clients()).await;
        }
        GmCommand::Spawn { display_id, name } => {
            let display_id = display_id.unwrap_or_else(random_display_id);
            let name = name.unwrap_or_else(random_name);
            let entry = db.new_guid() as u32;
            let guid = db.new_guid().into();
            let creature =
                Creature::with_display(name.clone(), guid, display_id, entry, client.position());
            commands.push(crate::world::command::WorldCommand::SpawnCreature(creature));
            client
                .send_system_message(format!("Spawned '{name}' (display {display_id})"))
                .await;
        }
        GmCommand::MoveNpc => {
            client
                .send_message(SMSG_COMPRESSED_MOVES {
                    moves: vec![CompressedMove {
                        opcode: CompressedMove_CompressedMoveOpcode::SmsgMonsterMove {
                            monster_move: MonsterMove {
                                spline_point: Vector3d {
                                    x: -8938.857,
                                    y: -131.36594,
                                    z: 83.57745,
                                },
                                spline_id: 0,
                                move_type: MonsterMove_MonsterMoveType::Normal {
                                    duration: 0,
                                    spline_flags: SplineFlag::empty(),
                                    splines: vec![Vector3d {
                                        x: -8937.863,
                                        y: -117.46813,
                                        z: 82.39997,
                                    }],
                                },
                            },
                        },
                        guid: entities.creatures()[0].guid,
                    }],
                })
                .await;
        }
        GmCommand::Boom => {
            const SPELL_ARCANE_EXPLOSION: u32 = 1449;
            const RADIUS: f32 = 10.0;
            const DAMAGE: u32 = 1332;

            let caster_guid = client.character().guid;
            let caster_pos = client.character().info.position;
            let caster_map = client.character().map;

            // Region-agnostic target lookup. `creatures_in_radius`
            // spans the global AoI snapshot so creatures sitting
            // just past a region boundary are still in scope.
            let targets: Vec<Guid> = entities
                .creatures_in_radius(caster_pos, caster_map, RADIUS)
                .into_iter()
                .map(|v| v.guid)
                .collect();

            // Apply damage to each. `apply_effect` routes locally for
            // same-region creatures and via `CrossRegionMsg::Effect`
            // for neighbor-region ones (whose drain pushes the
            // KillCreature itself).
            for target_guid in &targets {
                let outcome = entities.apply_effect(
                    *target_guid,
                    crate::world::command::UnitEffect::Damage { amount: DAMAGE },
                );
                if let crate::world::world_opcode_handler::entities::ApplyEffectResult::AppliedLocally {
                    creature_died: true,
                } = outcome
                {
                    commands.push(crate::world::command::WorldCommand::KillCreature(*target_guid));
                }
            }

            // Visuals go through the cross-region-aware broadcast.
            // `broadcast_within_aoi` serializes once and posts the
            // frame into every neighbor region's inbox via the
            // routing table — observers across the boundary see the
            // spell + damage log without needing local creature data.
            let spell_go = SMSG_SPELL_GO {
                cast_item: caster_guid,
                caster: caster_guid,
                spell: SPELL_ARCANE_EXPLOSION,
                flags: SMSG_SPELL_GO_CastFlags::empty(),
                hits: targets.clone(),
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

            for target_guid in &targets {
                let damage_log = SMSG_SPELLNONMELEEDAMAGELOG {
                    target: *target_guid,
                    attacker: caster_guid,
                    spell: SPELL_ARCANE_EXPLOSION,
                    damage: DAMAGE,
                    school: SpellSchool::Arcane,
                    absorbed_damage: 0,
                    resisted: 0,
                    periodic_log: false,
                    unused: 0,
                    blocked: 0,
                    hit_info: HitInfo::CriticalHit,
                    extend_flag: 0,
                };
                client.send_message(damage_log).await;
                crate::world::aoi::broadcast_within_aoi(
                    damage_log,
                    caster_pos,
                    caster_map,
                    entities.clients(),
                )
                .await;
            }

            // HP update broadcast — we don't know the post-damage
            // health for CROSS-REGION targets (snapshot has positions
            // only, not health). Send a synthetic "took DAMAGE" for
            // local kills the handler can observe; cross-region
            // observers will see the HP via the neighbor region's
            // own broadcast on its next tick. Acceptable lag.
            for target_guid in &targets {
                // Resolve local target's new health if we can; skip
                // otherwise.
                let new_health =
                    entities.find_creature(*target_guid).map(|c| c.health);
                let Some(new_health) = new_health else {
                    continue;
                };
                if new_health == 0 {
                    continue; // already queued KillCreature above
                }
                let hp_update = SMSG_UPDATE_OBJECT {
                    has_transport: 0,
                    objects: vec![Object {
                        update_type: Object_UpdateType::Values {
                            guid1: *target_guid,
                            mask1: UpdateMask::Unit(
                                UpdateUnitBuilder::new()
                                    .set_unit_health(i32::try_from(new_health).unwrap_or(i32::MAX))
                                    .finalize(),
                            ),
                        },
                    }],
                };
                client.send_message(hp_update.clone()).await;
                crate::world::aoi::broadcast_within_aoi(
                    hp_update,
                    caster_pos,
                    caster_map,
                    entities.clients(),
                )
                .await;
            }
        }
        GmCommand::Nova => {
            const SPELL_FROST_NOVA: u32 = 122;
            const RADIUS: f32 = 14.0;
            const ROOT_DURATION: Duration = Duration::from_secs(6);

            let caster_guid = client.character().guid;
            let caster_pos = client.character().info.position;
            let caster_map = client.character().map;
            let root_until = Instant::now() + ROOT_DURATION;

            // Region-agnostic find: the snapshot spans the whole world,
            // so creatures + clients in neighbor regions across a
            // boundary are returned alongside local ones. Snapshot is
            // one tick stale; at 30 Hz / run speed that's ~0.2 yd of
            // position drift, well under the 14 yd nova radius.
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

            // Apply server-side root to every target — local or
            // cross-region. `apply_effect` routes by guid: local
            // mutation for same-region targets, queued
            // `CrossRegionEffect` for neighbor-region targets (drained
            // on the target region's next tick, ~33 ms lag).
            for g in creature_hits.iter().chain(client_hits.iter()) {
                entities.apply_effect(*g, crate::world::command::UnitEffect::Root { until: root_until });
            }

            let hits: Vec<wow_world_messages::Guid> = creature_hits
                .iter()
                .chain(client_hits.iter())
                .copied()
                .collect();

            // Spell-go visual. `broadcast_within_aoi` already does
            // cross-region post-fanout (routes the frame through the
            // routing table to every neighbor inbox), so observers in
            // neighbor regions also see the nova land. Send to caster
            // explicitly since `broadcast_within_aoi` excludes the
            // source from the AOI fan-out for movement opcodes — but
            // for spell visuals the caster needs to see it too.
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

            const AFLAG_HARMFUL: u8 = 0x02;
            const AFLAG_VISIBLE: u8 = 0x08;
            const AFLAG_NOT_CANCELABLE: u8 = 0x20;
            const AURA_FLAGS: u8 = AFLAG_HARMFUL | AFLAG_VISIBLE | AFLAG_NOT_CANCELABLE;

            // Aura visual for every hit. One broadcast per target;
            // both unit (creature) and player builders apply the same
            // aura mask — they're separate update structs in the wire
            // protocol but the broadcast path doesn't care.
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

            // SMSG_FORCE_MOVE_ROOT is only meaningful to real WoW
            // clients (it locks their movement input). Bots ignore it.
            // For cross-region rooted clients we can't send it
            // directly — they're in a neighbor region's slab — so the
            // server-side root takes over instead via apply_effect.
            // Local rooted clients get the SMSG path too.
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
        GmCommand::WorldDbInfo => {
            let (mut idle, mut wander, mut waypoint, mut aggro) = (0, 0, 0, 0);
            for (_, c) in entities.creatures().iter() {
                match c.behavior {
                    crate::world::world_opcode_handler::creature::CreatureBehavior::Idle => {
                        idle += 1;
                    }
                    crate::world::world_opcode_handler::creature::CreatureBehavior::RandomWander { .. } => {
                        wander += 1;
                    }
                    crate::world::world_opcode_handler::creature::CreatureBehavior::Waypoint { .. } => {
                        waypoint += 1;
                    }
                    crate::world::world_opcode_handler::creature::CreatureBehavior::AggroChase => {
                        aggro += 1;
                    }
                }
            }
            let total = idle + wander + waypoint + aggro;
            client
                .send_system_message(format!(
                    "creatures: {total} (idle {idle}, wander {wander}, waypoint {waypoint}, aggro {aggro})"
                ))
                .await;
        }
        GmCommand::Players => {
            use crate::world::world_opcode_handler::creature::CreatureLifeState;
            use ahash::AHashMap;

            // Snapshot the caller's map first so the "near you" line is
            // measured against where the GM actually stands.
            let caller_map = client.character().map;
            let caller_pos = client.character().info.position;
            let aoi_radius = crate::config::config().network.aoi_radius_yards;

            // Player-side: total, per-map distribution, AOI-near-caller.
            let mut total_players = 0_usize;
            let mut per_map: AHashMap<wow_world_base::vanilla::Map, usize> =
                AHashMap::new();
            let mut near_me = 0_usize;
            for (_, c) in entities.clients().iter() {
                total_players += 1;
                *per_map.entry(c.character().map).or_default() += 1;
                if c.character().map == caller_map {
                    let dx = c.character().info.position.x - caller_pos.x;
                    let dy = c.character().info.position.y - caller_pos.y;
                    if dx * dx + dy * dy <= aoi_radius * aoi_radius {
                        near_me += 1;
                    }
                }
            }

            // Creature-side: life-state breakdown so the GM can sanity-
            // check why `creatures_active` in the slow-tick line moves.
            let (mut c_alive, mut c_corpse, mut c_respawning) = (0_usize, 0_usize, 0_usize);
            for (_, cr) in entities.creatures().iter() {
                match cr.life_state {
                    CreatureLifeState::Alive => c_alive += 1,
                    CreatureLifeState::Corpse { .. } => c_corpse += 1,
                    CreatureLifeState::Respawning { .. } => c_respawning += 1,
                }
            }
            let c_total = c_alive + c_corpse + c_respawning;

            client
                .send_system_message(format!(
                    "Players in-world: {total_players} (near you: {near_me} within {aoi_radius:.0}yd on '{map}')",
                    map = caller_map,
                ))
                .await;
            // Per-map line — sorted by descending count so the heaviest
            // map shows first. Cap at 6 entries to fit one chat line.
            let mut maps: Vec<(wow_world_base::vanilla::Map, usize)> =
                per_map.into_iter().collect();
            maps.sort_by_key(|b| std::cmp::Reverse(b.1));
            let map_str = maps
                .iter()
                .take(6)
                .map(|(m, n)| format!("{m}={n}"))
                .collect::<Vec<_>>()
                .join(", ");
            if !map_str.is_empty() {
                client
                    .send_system_message(format!("Per-map: {map_str}"))
                    .await;
            }
            client
                .send_system_message(format!(
                    "Creatures: {c_total} (alive {c_alive}, corpse {c_corpse}, respawning {c_respawning})"
                ))
                .await;
        }
        GmCommand::Regions => {
            use crate::world::region::RegionKey;
            use ahash::AHashMap;

            // Bin clients and creatures into the spatial regions the
            // Stage 3 sharding uses. Empty regions (zero players) are
            // dropped — a GM running `.regions` cares about hot spots,
            // not the long tail of empty buckets.
            //
            // Players come from `PLAYER_REGISTRY` (the process-wide
            // index also used by `.go PlayerName`) so the count is
            // cross-region accurate. The requesting GM has been
            // transiently removed from their region's slab + the
            // registry for the duration of this opcode handler — so
            // we explicitly add them back to the count for their own
            // region. Without this, `.regions` reported `players=0`
            // for a GM alone in the world.
            let mut player_counts: AHashMap<RegionKey, usize> = AHashMap::new();
            let mut creature_counts: AHashMap<RegionKey, usize> = AHashMap::new();
            if let Ok(reg) = crate::world::region::PLAYER_REGISTRY.lock() {
                for entry in reg.values() {
                    let key = RegionKey::from_position(
                        entry.map,
                        entry.position.x,
                        entry.position.y,
                    );
                    *player_counts.entry(key).or_default() += 1;
                }
            }
            {
                let ch = client.character();
                let me_key = RegionKey::from_position(
                    ch.map,
                    ch.info.position.x,
                    ch.info.position.y,
                );
                *player_counts.entry(me_key).or_default() += 1;
            }
            // Creatures are still counted from the requesting GM's
            // own region only — partitioning means each region owns
            // its creatures slab and locking neighbors mid-tick could
            // stall this region's tick. The visible side-effect:
            // `.regions` shows accurate creature counts for the
            // region the GM is in, and zero for the others. Acceptable
            // for a debug command; a cross-region creature-count
            // index is a future cleanup.
            for (_, cr) in entities.creatures().iter() {
                let key = RegionKey::from_position(cr.map, cr.info.position.x, cr.info.position.y);
                *creature_counts.entry(key).or_default() += 1;
            }

            // Filter to regions that have at least one player, then
            // sort by descending player count (creature count
            // breaks ties). A region with creatures but no players
            // is just terrain — not interesting for a `.regions`
            // peek.
            let mut ranked: Vec<(RegionKey, usize, usize)> = player_counts
                .iter()
                .map(|(k, &p)| {
                    let c = creature_counts.get(k).copied().unwrap_or(0);
                    (*k, p, c)
                })
                .collect();
            ranked.sort_by_key(|&(_, p, c)| std::cmp::Reverse((p, c)));

            // Snapshot per-region pacer state. The per-region tokio
            // tasks publish their pacer fields into this map at the
            // end of every tick; we lock briefly here and pull out
            // what the requested regions are doing.
            let pacer_states: ahash::AHashMap<RegionKey, crate::world::region::PacerSnapshot> =
                crate::world::region::PACER_STATES
                    .lock()
                    .ok()
                    .map(|g| g.clone())
                    .unwrap_or_default();

            const TOP_N: usize = 4;
            let total_populated = ranked.len();
            client
                .send_system_message(format!(
                    "Regions: {total_populated} populated ({:.0}-yd each); top {} by player count",
                    crate::world::region::region_size_yd(),
                    TOP_N.min(total_populated),
                ))
                .await;
            for (k, p, c) in ranked.iter().take(TOP_N) {
                let pacer = pacer_states
                    .get(k)
                    .map(|s| {
                        format!(
                            "{}ms last={}ms ema={:.2} streak={}",
                            s.current_interval_ms,
                            s.last_tick_ms,
                            s.slow_ema,
                            s.healthy_streak,
                        )
                    })
                    .unwrap_or_else(|| "<no pacer state>".to_string());
                client
                    .send_system_message(format!(
                        "  {k}: players={p}, creatures={c}, pacer={pacer}"
                    ))
                    .await;
            }
            if total_populated > TOP_N {
                client
                    .send_system_message(format!("  … {} more populated regions truncated", total_populated - TOP_N))
                    .await;
            }
        }
        GmCommand::Information(target) => {
            let info = if let Some(target) = entities.find_guid(target) {
                match target {
                    Entity::Player(c) => {
                        let name = c.character().name.as_str();
                        let guid = c.character().guid;
                        let race = c.character().race_class;
                        let gender = c.character().gender;
                        let level = c.character().level;

                        let map = c.character().map;
                        let Position { x, y, z, .. } = c.position();

                        format!("Player '{name}' ({guid})\nLevel {level} {gender} {race}\n{map} x: {x}, y: {y}, z: {z}")
                    }
                    Entity::Creature(c) => {
                        let name = c.name.as_str();
                        let guid = c.guid;

                        let map = c.map;
                        let Position { x, y, z, .. } = c.position();

                        format!("Creature '{name}' ({guid})\n{map} x: {x}, y: {y}, z: {z} (Client movement not supported)")
                    }
                }
            } else {
                client
                    .send_system_message(format!("Unable to find target '{target}'"))
                    .await;
                return;
            };

            client.send_system_message(info).await;
        }
        GmCommand::ShouldNotHaveLineOfSight(target) | GmCommand::ShouldHaveLineOfSight(target) => {
            let pos = client.position();
            let o = if let Some(other) = entities.find_player(target) {
                other
            } else {
                client
                    .send_system_message(format!("Unable to find player '{target}'"))
                    .await;
                return;
            };
            let other = o.position();

            let f = if let Some(map) = maps.get(&pos.map) {
                match map.line_of_sight(pos.into(), other.into()) {
                    Ok(true) => client.send_system_message(format!(
                        "Has line of sight to {}",
                        o.character().name
                    )),
                    Ok(false) => client.send_system_message(format!(
                        "Has no line of sight to {}",
                        o.character().name
                    )),
                    // namigator raycasts can fail on degenerate input (e.g.
                    // point outside the loaded map tile). Surface the error
                    // to the GM instead of panicking the world task.
                    Err(e) => client.send_system_message(format!(
                        "LOS check failed: {e:?}"
                    )),
                }
            } else {
                client.send_system_message(format!(
                    "Unable to find map '{map}' in pathfinding maps",
                    map = pos.map
                ))
            };

            f.await;
        }
        GmCommand::Swifty => {
            // Every connected player (including the GM caller) appears to
            // yell a randomized "SWIFTY INVASION" — pure visual gag for
            // stress tests and demos. Yells are visible to everyone
            // connected; we skip the usual YELL-range distance check by
            // design.
            //
            // Each yell is independently randomized so the chat log fills
            // with a chaotic mix of "SWIFTY INVASION", "SWIFTYYYY INVASION",
            // "SWIFTY INVASIONNNNN!!!!", etc.
            //
            // Snapshot the sender guids up front so we don't double-borrow
            // entities.clients() while iterating. The GM caller is held
            // outside the slab during per_client_loop, so we append their
            // guid to the snapshot and send to `client` separately on each
            // iteration.
            use wow_world_messages::vanilla::{
                Language, PlayerChatTag, SMSG_MESSAGECHAT, SMSG_MESSAGECHAT_ChatType,
            };
            let caller_guid = client.character().guid;
            let mut sender_guids: Vec<Guid> = entities
                .clients()
                .iter()
                .map(|(_, c)| c.character().guid)
                .collect();
            sender_guids.push(caller_guid);
            for sender in sender_guids {
                let msg = SMSG_MESSAGECHAT {
                    chat_type: SMSG_MESSAGECHAT_ChatType::Yell {
                        chat_credit: sender,
                        speech_bubble_credit: sender,
                    },
                    language: Language::Universal,
                    message: swifty_invasion_yell(),
                    tag: PlayerChatTag::None,
                };
                client.send_message(msg.clone()).await;
                for (_, c) in entities.clients().iter_mut() {
                    c.send_message(msg.clone()).await;
                }
            }
        }
    }
}

pub(crate) fn next_rand() -> u64 {
    thread_local! {
        static STATE: Cell<u64> = Cell::new({
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0xDEAD_BEEF_CAFE_BABE);
            nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1
        });
    }
    STATE.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        x
    })
}

fn random_display_id() -> u16 {
    (next_rand() % 11657 + 1) as u16
}

pub(crate) fn random_name() -> String {
    let len = (next_rand() % 6 + 5) as usize;
    (0..len)
        .map(|_| (b'a' + (next_rand() % 26) as u8) as char)
        .collect()
}

/// Pick one randomized "SWIFTY INVASION" variant for the `.swifty` GM
/// command. Independently rolls four knobs:
///
/// - elongate the trailing `Y` of `SWIFTY` (2..=10 extra Ys)
/// - elongate the trailing `N` of `INVASION` (2..=14 extra Ns)
/// - append exclamation marks (0..=24)
/// - rare ~1-in-8 chance of repeating the whole phrase twice ("SWIFTY
///   INVASION SWIFTY INVASION") for extra chaos in the chat log
///
/// All four knobs roll independently, so the same call can produce things
/// like `SWIFTY INVASION`, `SWIFTYYYY INVASION`, `SWIFTY INVASIONNNNN!!!!`,
/// `SWIFTYYYY INVASIONNNN!!!!!!!`, etc.
fn swifty_invasion_yell() -> String {
    let r = next_rand();
    let elongate_swifty = (r & 0b001) != 0;
    let elongate_invasion = (r & 0b010) != 0;
    let repeat_phrase = (r & 0b111_0000) == 0; // ~1/8

    let swifty = if elongate_swifty {
        let extra = 2 + (next_rand() % 9) as usize; // 2..=10
        format!("SWIFT{}", "Y".repeat(extra + 1))
    } else {
        "SWIFTY".to_string()
    };

    let invasion = if elongate_invasion {
        let extra = 2 + (next_rand() % 13) as usize; // 2..=14
        format!("INVASIO{}", "N".repeat(extra + 1))
    } else {
        "INVASION".to_string()
    };

    let exclamations = "!".repeat((next_rand() % 25) as usize); // 0..=24
    let base = format!("{swifty} {invasion}");

    if repeat_phrase {
        format!("{base} {base}{exclamations}")
    } else {
        format!("{base}{exclamations}")
    }
}
