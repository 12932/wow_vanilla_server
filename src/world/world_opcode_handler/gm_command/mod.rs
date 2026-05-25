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
use tracing::info;
use wow_world_base::vanilla::position::Position;
use wow_world_base::vanilla::{HitInfo, Map, SpellSchool, SplineFlag, Vector3d};
use wow_world_messages::vanilla::{
    CompressedMove, CompressedMove_CompressedMoveOpcode, MonsterMove,
    MonsterMove_MonsterMoveType, Object,
    Object_UpdateType, SpellCastTargets, UpdateMask, UpdateUnitBuilder,
    SMSG_COMPRESSED_MOVES, SMSG_FORCE_RUN_SPEED_CHANGE,
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

            // Cell-agnostic target lookup. `creatures_in_radius`
            // spans the global AoI snapshot so creatures sitting
            // just past a cell boundary are still in scope.
            let targets: Vec<Guid> = entities
                .creatures_in_radius(caster_pos, caster_map, RADIUS)
                .into_iter()
                .map(|v| v.guid)
                .collect();

            // Apply damage to each. `apply_effect` routes locally for
            // same-cell creatures and via `CrossCellMsg::Effect`
            // for neighbor-cell ones (whose drain pushes the
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

            // Visuals go through the cross-cell-aware broadcast.
            // `broadcast_within_aoi` serializes once and posts the
            // frame into every neighbor cell's inbox via the
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
            // health for CROSS-CELL targets (snapshot has positions
            // only, not health). Send a synthetic "took DAMAGE" for
            // local kills the handler can observe; cross-cell
            // observers will see the HP via the neighbor cell's
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
            crate::world::world_opcode_handler::spell::cast_frost_nova(client, entities).await;
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
        GmCommand::Cells => {
            use ahash::AHashMap;

            // Per-MAP summary (concurrency is per continent). Players come
            // from `PLAYER_REGISTRY` (process-wide, also used by `.go`); the
            // requesting GM is transiently out of the registry during this
            // handler, so add them back for their own map. Creature counts
            // are from the GM's own map only (each MapState owns its creatures;
            // counting others would need locking their tasks).
            let mut player_counts: AHashMap<Map, usize> = AHashMap::new();
            if let Ok(reg) = crate::world::cell::PLAYER_REGISTRY.lock() {
                for entry in reg.values() {
                    *player_counts.entry(entry.map).or_default() += 1;
                }
            }
            *player_counts.entry(client.character().map).or_default() += 1;

            let mut creature_counts: AHashMap<Map, usize> = AHashMap::new();
            for (_, cr) in entities.creatures().iter() {
                *creature_counts.entry(cr.map).or_default() += 1;
            }

            let pacer_states: AHashMap<Map, crate::world::cell::PacerSnapshot> =
                crate::world::cell::PACER_STATES
                    .lock()
                    .ok()
                    .map(|g| g.clone())
                    .unwrap_or_default();

            let mut ranked: Vec<(Map, usize, usize)> = player_counts
                .iter()
                .map(|(m, &p)| (*m, p, creature_counts.get(m).copied().unwrap_or(0)))
                .collect();
            ranked.sort_by_key(|&(_, p, c)| std::cmp::Reverse((p, c)));

            client
                .send_system_message(format!("Maps: {} populated", ranked.len()))
                .await;
            for (m, p, c) in &ranked {
                let pacer = pacer_states
                    .get(m)
                    .map(|s| {
                        format!(
                            "{}ms last={}ms ema={:.2} streak={}",
                            s.current_interval_ms, s.last_tick_ms, s.slow_ema, s.healthy_streak,
                        )
                    })
                    .unwrap_or_else(|| "<no pacer state>".to_string());
                client
                    .send_system_message(format!(
                        "  {m:?}: players={p}, creatures={c}, pacer={pacer}"
                    ))
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

            // Raise each endpoint to its own unit's chest height before the
            // raycast — units sit exactly on the collision mesh, so a
            // foot-level ray grazes the floor and reports "obstructed".
            // Per-unit (model-aware) height mirrors mangos `IsWithinLOSInMap`
            // (source += source's GetCollisionHeight(), target += target's).
            // Players have object scale 1.0 (no scale auras modelled yet).
            let caster_h = maps.collision_height(client.character().display_id(), 1.0);
            let target_h = maps.collision_height(o.character().display_id(), 1.0);
            let mut from: Vector3d = pos.into();
            let mut to: Vector3d = other.into();
            from.z += caster_h;
            to.z += target_h;

            // `line_of_sight` lazily loads the endpoint ADT tiles; the raw
            // `VanillaMap::line_of_sight` fails open against unloaded tiles.
            let f = match maps.line_of_sight(pos.map, from, to) {
                Some(Ok(true)) => client.send_system_message(format!(
                    "Has line of sight to {}",
                    o.character().name
                )),
                Some(Ok(false)) => client.send_system_message(format!(
                    "Has no line of sight to {}",
                    o.character().name
                )),
                // rustigator raycasts can fail on degenerate input (e.g.
                // point outside the loaded map tile). Surface the error
                // to the GM instead of panicking the world task.
                Some(Err(e)) => client.send_system_message(format!(
                    "LOS check failed: {e:?}"
                )),
                None => client.send_system_message(format!(
                    "Unable to find map '{map}' in pathfinding maps",
                    map = pos.map
                )),
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
