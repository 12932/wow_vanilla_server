use crate::world::world_opcode_handler::creature::{
    default_creature_health, initial_respawn_delay, Creature, CreatureBehavior,
};
use ahash::AHashMap;
use rusqlite::{Connection, OpenFlags};
use slab::Slab;
use std::time::{Duration, Instant};
use tracing::{info, warn};
use wow_world_base::vanilla::Map;
use wow_world_messages::vanilla::{MovementInfo, Vector3d};
use wow_world_messages::Guid;

/// Offset to keep mangos guids out of the player-guid namespace
/// (`db.new_guid()` returns small incrementing integers starting at 0).
const MANGOS_GUID_OFFSET: u64 = 0x1000_0000;

/// Bumped slightly past zero so wander mobs have a real distance to travel
/// before deciding to stop; mangos rows often record 0.0 even when the mob
/// should pace a little.
const MIN_WANDER_RADIUS: f32 = 1.0;

pub fn load_creatures(sqlite_path: &str) -> rusqlite::Result<Slab<Creature>> {
    let conn = Connection::open_with_flags(
        sqlite_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    let mut waypoint_spawn_ids: Vec<i64> = Vec::new();
    let mut waypoint_entry_ids: Vec<i64> = Vec::new();
    let mut raw_rows: Vec<RawRow> = Vec::with_capacity(64_000);

    {
        let mut stmt = conn.prepare(
            "SELECT c.guid, c.id, c.map, c.position_x, c.position_y, c.position_z, c.orientation, \
                    c.spawndist, c.MovementType, c.curhealth, c.modelid, \
                    ct.Name, ct.ModelId1, ct.ModelId2, ct.ModelId3, ct.ModelId4, \
                    ct.MinLevel, ct.FactionAlliance, \
                    ct.MinLevelHealth, ct.MaxLevelHealth \
             FROM creature c \
             JOIN creature_template ct ON c.id = ct.Entry \
             WHERE c.map IN (0, 1) AND c.DeathState = 0",
        )?;

        let iter = stmt.query_map([], |row| {
            Ok(RawRow {
                spawn_guid: row.get::<_, i64>(0)?,
                entry: i64_to_u32(row.get::<_, i64>(1)?),
                map_id: i64_to_u32(row.get::<_, i64>(2)?),
                x: row.get::<_, f64>(3)? as f32,
                y: row.get::<_, f64>(4)? as f32,
                z: row.get::<_, f64>(5)? as f32,
                orientation: row.get::<_, f64>(6)? as f32,
                spawndist: row.get::<_, f64>(7)? as f32,
                movement_type: i64_to_i32(row.get::<_, i64>(8)?),
                cur_health: i64_to_u32(row.get::<_, i64>(9)?),
                spawn_model: i64_to_u32(row.get::<_, i64>(10)?),
                name: row.get::<_, String>(11)?,
                template_models: [
                    i64_to_u32(row.get::<_, i64>(12)?),
                    i64_to_u32(row.get::<_, i64>(13)?),
                    i64_to_u32(row.get::<_, i64>(14)?),
                    i64_to_u32(row.get::<_, i64>(15)?),
                ],
                min_level: i64_to_u8(row.get::<_, i64>(16)?),
                faction_alliance: i64_to_u32(row.get::<_, i64>(17)?),
                min_health: i64_to_u32(row.get::<_, i64>(18)?),
                max_health: i64_to_u32(row.get::<_, i64>(19)?),
            })
        })?;

        for row in iter {
            let row = row?;
            if row.movement_type == 2 {
                waypoint_spawn_ids.push(row.spawn_guid);
                waypoint_entry_ids.push(row.entry as i64);
            }
            raw_rows.push(row);
        }
    }

    let spawn_paths = load_spawn_waypoints(&conn, &waypoint_spawn_ids)?;
    let entry_paths = load_entry_waypoints(&conn, &waypoint_entry_ids)?;

    let now = Instant::now();
    let mut slab: Slab<Creature> = Slab::with_capacity(raw_rows.len());
    let mut counts = [0_usize; 4]; // idle, wander, waypoint, skipped

    for row in raw_rows {
        let Ok(map) = Map::try_from(row.map_id) else {
            counts[3] += 1;
            continue;
        };

        let display_id = pick_display_id(row.spawn_model, &row.template_models);
        let Some(display_id) = display_id else {
            counts[3] += 1;
            continue;
        };

        let max_health = pick_health(row.max_health, row.min_health);
        let health = if row.cur_health == 0 {
            max_health
        } else {
            row.cur_health.min(max_health)
        };

        let position = Vector3d {
            x: row.x,
            y: row.y,
            z: row.z,
        };

        let behavior = match row.movement_type {
            1 => {
                counts[1] += 1;
                let radius = row.spawndist.max(MIN_WANDER_RADIUS);
                let jitter = row.spawn_guid as u64 % 4000;
                CreatureBehavior::RandomWander {
                    anchor: position,
                    radius,
                    target: None,
                    next_decision_at: now + Duration::from_millis(jitter),
                }
            }
            2 => {
                let path = spawn_paths
                    .get(&row.spawn_guid)
                    .or_else(|| entry_paths.get(&(row.entry as i64)));
                if let Some(path) = path {
                    counts[2] += 1;
                    CreatureBehavior::Waypoint {
                        waypoints: path.points.clone(),
                        waittimes_ms: path.waittimes.clone(),
                        current: 0,
                        idle_until: None,
                    }
                } else {
                    counts[0] += 1;
                    CreatureBehavior::Idle
                }
            }
            _ => {
                counts[0] += 1;
                CreatureBehavior::Idle
            }
        };

        let runtime_guid = Guid::new(MANGOS_GUID_OFFSET + row.spawn_guid as u64);

        slab.insert(Creature {
            name: row.name,
            guid: runtime_guid,
            info: MovementInfo {
                flags: Default::default(),
                timestamp: 0,
                position,
                orientation: row.orientation,
                fall_time: 0.0,
            },
            map,
            level: row.min_level.max(1),
            display_id: display_id as u16,
            entry: row.entry,
            faction_template: row.faction_alliance,
            health,
            max_health,
            root_until: None,
            behavior,
            last_advanced_at: now,
            last_heartbeat_at: now,
            spawn_position: position,
            spawn_orientation: row.orientation,
            life_state: crate::world::world_opcode_handler::creature::CreatureLifeState::Alive,
            last_alive_at: now,
            respawn_delay: initial_respawn_delay(),
        });
    }

    info!(
        "world_db: loaded {} creatures (idle {}, wander {}, waypoint {}, skipped {})",
        slab.len(),
        counts[0],
        counts[1],
        counts[2],
        counts[3]
    );
    if counts[3] > 0 {
        warn!(
            "world_db: {} rows skipped (unknown map or no usable display_id)",
            counts[3]
        );
    }

    Ok(slab)
}

struct RawRow {
    spawn_guid: i64,
    entry: u32,
    map_id: u32,
    x: f32,
    y: f32,
    z: f32,
    orientation: f32,
    spawndist: f32,
    movement_type: i32,
    cur_health: u32,
    spawn_model: u32,
    name: String,
    template_models: [u32; 4],
    min_level: u8,
    faction_alliance: u32,
    min_health: u32,
    max_health: u32,
}

struct Path {
    points: Vec<Vector3d>,
    waittimes: Vec<u32>,
}

fn load_spawn_waypoints(
    conn: &Connection,
    ids: &[i64],
) -> rusqlite::Result<AHashMap<i64, Path>> {
    if ids.is_empty() {
        return Ok(AHashMap::new());
    }
    let placeholders = repeat_placeholders(ids.len());
    let sql = format!(
        "SELECT id, point, position_x, position_y, position_z, waittime \
         FROM creature_movement WHERE id IN ({placeholders}) \
         ORDER BY id, point"
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
    let iter = stmt.query_map(params.as_slice(), |row| {
        Ok(WaypointRow {
            key: row.get::<_, i64>(0)?,
            _point: i64_to_u32(row.get::<_, i64>(1)?),
            x: row.get::<_, f64>(2)? as f32,
            y: row.get::<_, f64>(3)? as f32,
            z: row.get::<_, f64>(4)? as f32,
            waittime: i64_to_u32(row.get::<_, i64>(5)?),
        })
    })?;
    collect_paths(iter)
}

fn load_entry_waypoints(
    conn: &Connection,
    entries: &[i64],
) -> rusqlite::Result<AHashMap<i64, Path>> {
    if entries.is_empty() {
        return Ok(AHashMap::new());
    }
    let placeholders = repeat_placeholders(entries.len());
    let sql = format!(
        "SELECT entry, point, position_x, position_y, position_z, waittime \
         FROM creature_movement_template WHERE entry IN ({placeholders}) \
         ORDER BY entry, point"
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> =
        entries.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
    let iter = stmt.query_map(params.as_slice(), |row| {
        Ok(WaypointRow {
            key: row.get::<_, i64>(0)?,
            _point: i64_to_u32(row.get::<_, i64>(1)?),
            x: row.get::<_, f64>(2)? as f32,
            y: row.get::<_, f64>(3)? as f32,
            z: row.get::<_, f64>(4)? as f32,
            waittime: i64_to_u32(row.get::<_, i64>(5)?),
        })
    })?;
    collect_paths(iter)
}

struct WaypointRow {
    key: i64,
    _point: u32,
    x: f32,
    y: f32,
    z: f32,
    waittime: u32,
}

fn collect_paths<I>(iter: I) -> rusqlite::Result<AHashMap<i64, Path>>
where
    I: Iterator<Item = rusqlite::Result<WaypointRow>>,
{
    let mut paths: AHashMap<i64, Path> = AHashMap::new();
    for row in iter {
        let row = row?;
        let entry = paths.entry(row.key).or_insert_with(|| Path {
            points: Vec::new(),
            waittimes: Vec::new(),
        });
        entry.points.push(Vector3d {
            x: row.x,
            y: row.y,
            z: row.z,
        });
        entry.waittimes.push(row.waittime);
    }
    Ok(paths)
}

fn repeat_placeholders(n: usize) -> String {
    let mut s = String::with_capacity(n * 2);
    for i in 0..n {
        if i > 0 {
            s.push(',');
        }
        s.push('?');
    }
    s
}

fn pick_display_id(spawn_model: u32, template_models: &[u32; 4]) -> Option<u32> {
    if spawn_model != 0 {
        return Some(spawn_model);
    }
    template_models.iter().copied().find(|&m| m != 0)
}

/// SQLite always returns numeric columns as `i64`. The mangos schema uses
/// these helpers' target types for IDs / levels / health, but bad data (or
/// columns occasionally storing negative sentinels) shouldn't silently wrap
/// to huge unsigned values. Clamp into range.
fn i64_to_u32(v: i64) -> u32 {
    v.clamp(0, u32::MAX as i64) as u32
}

fn i64_to_i32(v: i64) -> i32 {
    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

fn i64_to_u8(v: i64) -> u8 {
    v.clamp(0, u8::MAX as i64) as u8
}

fn pick_health(max_health: u32, min_health: u32) -> u32 {
    if max_health > 0 {
        max_health
    } else if min_health > 0 {
        min_health
    } else {
        default_creature_health()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_display_id_prefers_spawn_override() {
        assert_eq!(pick_display_id(123, &[10, 20, 0, 0]), Some(123));
    }

    #[test]
    fn pick_display_id_falls_back_to_first_nonzero_template() {
        assert_eq!(pick_display_id(0, &[0, 0, 555, 999]), Some(555));
    }

    #[test]
    fn pick_display_id_returns_none_when_all_zero() {
        assert_eq!(pick_display_id(0, &[0, 0, 0, 0]), None);
    }

    #[test]
    fn pick_health_prefers_max_when_set() {
        assert_eq!(pick_health(2000, 1000), 2000);
    }

    #[test]
    fn pick_health_falls_back_to_min_when_max_zero() {
        assert_eq!(pick_health(0, 1000), 1000);
    }

    #[test]
    fn pick_health_falls_back_to_default_when_both_zero() {
        assert_eq!(pick_health(0, 0), default_creature_health());
    }

    #[test]
    fn repeat_placeholders_empty() {
        assert_eq!(repeat_placeholders(0), "");
    }

    #[test]
    fn repeat_placeholders_one() {
        assert_eq!(repeat_placeholders(1), "?");
    }

    #[test]
    fn repeat_placeholders_many() {
        assert_eq!(repeat_placeholders(4), "?,?,?,?");
    }

    #[test]
    fn i64_to_u32_clamps_negative_to_zero() {
        assert_eq!(i64_to_u32(-1), 0);
        assert_eq!(i64_to_u32(i64::MIN), 0);
    }

    #[test]
    fn i64_to_u32_clamps_huge_to_max() {
        assert_eq!(i64_to_u32(i64::MAX), u32::MAX);
        assert_eq!(i64_to_u32(u32::MAX as i64), u32::MAX);
    }

    #[test]
    fn i64_to_u32_passes_through_normal() {
        assert_eq!(i64_to_u32(0), 0);
        assert_eq!(i64_to_u32(12345), 12345);
    }

    #[test]
    fn i64_to_u8_clamps_to_range() {
        assert_eq!(i64_to_u8(-1), 0);
        assert_eq!(i64_to_u8(300), u8::MAX);
        assert_eq!(i64_to_u8(60), 60);
    }

    #[test]
    fn i64_to_i32_clamps_extremes() {
        assert_eq!(i64_to_i32(i64::MAX), i32::MAX);
        assert_eq!(i64_to_i32(i64::MIN), i32::MIN);
        assert_eq!(i64_to_i32(0), 0);
    }
}
