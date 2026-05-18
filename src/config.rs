//! Server configuration loaded once at startup from `config.toml`.
//!
//! Every field has a code-side default so an empty / missing file just
//! runs the canonical values from before this module existed. Sections
//! and individual fields are all `#[serde(default)]`, so a partial file
//! is also fine — set only what you want to override.
//!
//! There is **no hot reload**. The config is read once during `main`,
//! stored in [`CONFIG`] (a `OnceLock`), and accessed everywhere via
//! [`config()`]. Restart the server to apply changes.
//!
//! Operator-host concerns (`WOW_REALM_ADDRESS`, `WOW_AUTH_AUTO_CREATE`,
//! `WOW_TRACY`, `RUST_LOG`) stay as env vars — they're deployment
//! topology, not gameplay behavior.

use serde::Deserialize;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;
use wow_world_base::vanilla::Map;

static CONFIG: OnceLock<ServerConfig> = OnceLock::new();

/// Global accessor. In production this returns the config installed by
/// [`install`] at the top of `main`. In tests / library use where
/// `install` wasn't called, returns a lazily-allocated default so
/// callers don't have to thread an `Option` through every read.
pub fn config() -> &'static ServerConfig {
    static FALLBACK: OnceLock<ServerConfig> = OnceLock::new();
    CONFIG
        .get()
        .unwrap_or_else(|| FALLBACK.get_or_init(ServerConfig::default))
}

/// Install `cfg` as the process-wide config. Subsequent calls are
/// no-ops — first writer wins. Returns the installed reference for
/// convenience.
pub fn install(cfg: ServerConfig) -> &'static ServerConfig {
    let _ = CONFIG.set(cfg);
    config()
}

/// Read `path` and parse it as TOML; on any failure (missing file,
/// parse error) fall back to defaults and warn so an operator can
/// see the file was ignored. Missing-file is INFO not WARN because
/// "no config file" is the legitimate dev case.
pub fn load_or_default(path: &Path) -> ServerConfig {
    match std::fs::read_to_string(path) {
        Ok(s) => match toml::from_str::<ServerConfig>(&s) {
            Ok(cfg) => {
                tracing::info!("loaded config from {}", path.display());
                cfg
            }
            Err(e) => {
                tracing::warn!(
                    "failed to parse {} ({e}); using defaults",
                    path.display()
                );
                ServerConfig::default()
            }
        },
        Err(_) => {
            tracing::info!(
                "no config file at {}; using defaults",
                path.display()
            );
            ServerConfig::default()
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub network: NetworkConfig,
    pub tick: TickConfig,
    pub combat: CombatConfig,
    pub respawn: RespawnConfig,
    pub creature: CreatureConfig,
    pub spawn: SpawnConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// Horizontal radius (yards) at which players are mutually visible.
    /// Z is ignored — vertical separation doesn't affect AOI.
    pub aoi_radius_yards: f32,
    /// Per-client outbound payload byte budget. Beyond this, queued
    /// frames for that client are dropped instead of being enqueued.
    /// 10 000 clients × 1 MiB ≈ 10 GiB worst-case buffer memory.
    pub outbound_channel_bytes: usize,
    /// Minimum interval (seconds) between per-guid AOI transitions
    /// for a given observer. Suppresses CreateObject/OutOfRangeObjects
    /// spam when a player parks on the AOI boundary and strafes to
    /// oscillate a target's range membership. 0 disables.
    pub aoi_flap_cooldown_secs: u64,
    /// Region size, in multiples of the creature spatial-grid cell
    /// (`CREATURE_GRID_CELL_YD`, currently 250 yd). Each region runs
    /// its own tick on a dedicated tokio task, paced independently.
    /// Default = 4 → 1000 yd regions: AOI radius (200 yd) is 20 % of
    /// the region, so ~36 % of broadcasts straddle a boundary and pay
    /// the cross-region channel cost. Lower values give more spatial
    /// isolation but more cross-region traffic. Must be ≥ 1.
    pub region_size_cells: u32,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            aoi_radius_yards: 200.0,
            outbound_channel_bytes: 1024 * 1024,
            aoi_flap_cooldown_secs: 3,
            region_size_cells: 4,
        }
    }
}

impl NetworkConfig {
    pub fn aoi_flap_cooldown(&self) -> Duration {
        Duration::from_secs(self.aoi_flap_cooldown_secs)
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TickConfig {
    /// Target world-tick period in milliseconds. 100 = 10 Hz.
    pub target_interval_ms: u64,
    /// Floor on the adaptive-pacing back-off. The tick interval will
    /// double under sustained slow ticks but stop at this value.
    pub max_interval_ms: u64,
    /// EMA coefficient for the "tick was slow" signal. Higher =
    /// faster reaction; lower = more averaging.
    pub slow_ema_alpha: f32,
    /// Slow-tick EMA threshold above which the pacer doubles the
    /// tick interval.
    pub backoff_threshold: f32,
    /// Required headroom (fraction of current interval) for a tick
    /// to count as healthy; prevents flapping at the boundary.
    pub recovery_hysteresis: f32,
    /// Consecutive healthy ticks needed before the interval halves
    /// back toward the target.
    pub recovery_healthy_streak: u32,
    /// How often the world snapshot is persisted to `snapshot.bin`.
    pub save_interval_secs: u64,
    /// Maximum number of `WaitingToLogIn` clients promoted into the world
    /// per tick. The promotion path scans every other in-AOI client +
    /// creature to build the joining player's initial visible-object
    /// bundle, so a login burst (300+ bots ramping at once) can pin
    /// `promote_logged_in` at hundreds of ms per tick. Capping spreads
    /// the burst across consecutive ticks at the cost of slightly slower
    /// ramp completion. Set to `0` to disable the cap (unbounded).
    pub max_promotions_per_tick: u32,
}

impl Default for TickConfig {
    fn default() -> Self {
        Self {
            // 30 Hz target. The adaptive pacer doubles `current_interval`
            // under sustained load, so a slow region falls through 33 →
            // 66 → 132 → 264 → 528 → 1000 ms (clamped at `max_interval_ms`),
            // i.e. 30 → 15 → 7.5 → 3.8 → 1.9 → 1 Hz. Per-region pacers
            // adapt independently — a hot region drops to 1 Hz while
            // adjacent regions stay at 30 Hz.
            target_interval_ms: 33,
            max_interval_ms: 1000,
            slow_ema_alpha: 0.2,
            backoff_threshold: 0.5,
            recovery_hysteresis: 0.6,
            recovery_healthy_streak: 30,
            save_interval_secs: 60,
            max_promotions_per_tick: 20,
        }
    }
}

impl TickConfig {
    pub fn target_interval(&self) -> Duration {
        Duration::from_millis(self.target_interval_ms)
    }
    pub fn max_interval(&self) -> Duration {
        Duration::from_millis(self.max_interval_ms)
    }
    pub fn save_interval(&self) -> Duration {
        Duration::from_secs(self.save_interval_secs)
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CombatConfig {
    /// Player HP cap under the simplified PvP rules.
    pub pvp_max_health: u32,
    /// cmangos `ATTACK_DISTANCE`. Floor on combined melee reach.
    pub attack_distance: f32,
    /// cmangos `BASE_MELEERANGE_OFFSET`. Static cushion added to
    /// the sum of both parties' combat reach.
    pub base_meleerange_offset: f32,
    /// cmangos `MELEE_LEEWAY`. Extra range granted when both
    /// parties are moving (compensates for heartbeat staleness).
    pub melee_leeway: f32,
    /// Combat reach value for player characters.
    pub player_combat_reach: f32,
    /// Combat reach value for creatures (until per-creature
    /// `creature_template.CombatReach` is wired up).
    pub creature_combat_reach: f32,
}

impl Default for CombatConfig {
    fn default() -> Self {
        Self {
            pvp_max_health: 100,
            attack_distance: 5.0,
            base_meleerange_offset: 1.333,
            melee_leeway: 8.0 / 3.0,
            player_combat_reach: 1.5,
            creature_combat_reach: 1.5,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RespawnConfig {
    /// How long a dead creature stays as a corpse before the
    /// despawn broadcast.
    pub corpse_despawn_secs: u64,
    /// Base respawn delay for creatures. Halves on each repeat-kill
    /// within the same window (existing live-stress behavior).
    pub initial_respawn_delay_secs: u64,
    /// Default HP for creatures that don't pull a value out of
    /// the worlddb.
    pub default_creature_health: u32,
}

impl Default for RespawnConfig {
    fn default() -> Self {
        Self {
            corpse_despawn_secs: 180,
            initial_respawn_delay_secs: 180,
            default_creature_health: 5000,
        }
    }
}

impl RespawnConfig {
    pub fn corpse_despawn(&self) -> Duration {
        Duration::from_secs(self.corpse_despawn_secs)
    }
    pub fn initial_respawn_delay(&self) -> Duration {
        Duration::from_secs(self.initial_respawn_delay_secs)
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CreatureConfig {
    /// Creature walk speed in yards/second.
    pub walk_speed: f32,
    /// Distance below which an aggro creature stops re-pathing to
    /// its target (yards). Avoids per-tick path jitter.
    pub re_path_threshold: f32,
    /// Stand-off distance for melee aggro creatures (yards).
    pub stand_off: f32,
    /// Maximum range at which an aggro creature acquires a target.
    pub max_follow_range: f32,
    /// Random-wander idle window minimum, milliseconds.
    pub wander_idle_min_ms: u64,
    /// Random-wander idle window maximum, milliseconds.
    pub wander_idle_max_ms: u64,
    /// Waypoint/wander creatures emit heartbeats this often.
    pub walking_heartbeat_ms: u128,
    /// Distance at which a walking creature marks a waypoint as
    /// reached and transitions to idle (yards).
    pub arrival_threshold: f32,
}

impl Default for CreatureConfig {
    fn default() -> Self {
        Self {
            walk_speed: 2.0,
            re_path_threshold: 0.5,
            stand_off: 3.0,
            max_follow_range: 60.0,
            wander_idle_min_ms: 3000,
            wander_idle_max_ms: 8000,
            walking_heartbeat_ms: 500,
            arrival_threshold: 0.4,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpawnConfig {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub orientation: f32,
    /// Map enum value — TOML string like `"EasternKingdoms"`. Deserialized
    /// by serde via `wow_world_base`'s own `Deserialize` impl.
    pub map: Map,
}

impl Default for SpawnConfig {
    fn default() -> Self {
        // Gurubashi Arena, Eastern Kingdoms — matches the prior
        // hardcoded value in `char_create.rs`.
        Self {
            x: -13206.0,
            y: 272.0,
            z: 21.857,
            orientation: 0.0,
            map: Map::EasternKingdoms,
        }
    }
}
