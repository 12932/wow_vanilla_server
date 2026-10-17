//! Server configuration, read once at start-up from `config.toml`.
//!
//! Every field has a default, so a missing file, or one that sets only a
//! few things, is fine. There is no reload: restart to apply a change.
//! Deployment settings (`WOW_REALM_ADDRESS`, `WOW_TRACY`, `RUST_LOG`) stay
//! environment variables.

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

static CONFIG: OnceLock<ServerConfig> = OnceLock::new();

/// The installed config; the defaults if none was installed (tests).
pub fn config() -> &'static ServerConfig {
    static FALLBACK: OnceLock<ServerConfig> = OnceLock::new();
    CONFIG
        .get()
        .unwrap_or_else(|| FALLBACK.get_or_init(ServerConfig::default))
}

/// Install `cfg` as the process's config. The first call wins.
pub fn install(cfg: ServerConfig) -> &'static ServerConfig {
    let _ = CONFIG.set(cfg);
    config()
}

/// Read `path` as TOML. A missing file gives the defaults; one that doesn't
/// parse is an error, so a typo isn't silently ignored.
pub fn load(path: &Path) -> Result<ServerConfig, String> {
    let mut cfg = match std::fs::read_to_string(path) {
        Ok(s) => {
            let cfg = toml::from_str::<ServerConfig>(&s)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            tracing::info!("loaded config from {}", path.display());
            cfg
        }
        Err(_) => {
            tracing::info!("no config file at {}; using defaults", path.display());
            ServerConfig::default()
        }
    };
    cfg.world.default_dirs();
    Ok(cfg)
}

/// Where the world's data is looked for when the config doesn't say: under
/// `data/` in the working directory, so a server run beside its data needs
/// no paths at all.
pub const DEFAULT_DATA_DIR: &str = "data/db";
pub const DEFAULT_NAV_DIR: &str = "data/nav";
pub const DEFAULT_TERRAIN_DIR: &str = "data/terrain";

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub network: NetworkConfig,
    pub world: WorldConfig,
    pub accounts: AccountsConfig,
    pub spawn: SpawnConfig,
    /// Gurubashi Arena's last man standing.
    pub arena: crate::game::map::arena::ArenaConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// Bytes a client's outbound queue may hold; a client that falls
    /// further behind is disconnected.
    pub outbound_channel_bytes: usize,
    /// Every read during auth and the world handshake must arrive within
    /// this.
    pub handshake_timeout_secs: u64,
    /// A client that sends nothing for this long is disconnected. Clients
    /// ping every 30 s.
    pub client_idle_timeout_secs: u64,
    /// Connections still in auth or the world handshake, at most.
    pub max_pending_connections: usize,
    /// Packets a client may send a second, sustained (three seconds' worth at
    /// once); a client that floods is disconnected.
    pub max_packets_per_sec: u32,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            outbound_channel_bytes: 4 * 1024 * 1024,
            handshake_timeout_secs: 10,
            client_idle_timeout_secs: 120,
            max_pending_connections: 1024,
            max_packets_per_sec: 100,
        }
    }
}

impl NetworkConfig {
    pub fn handshake_timeout(&self) -> Duration {
        Duration::from_secs(self.handshake_timeout_secs)
    }

    pub fn client_idle_timeout(&self) -> Duration {
        Duration::from_secs(self.client_idle_timeout_secs)
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorldConfig {
    /// How far players see, in yards.
    pub visibility_yards: f32,
    /// How often each online character is saved.
    pub save_interval_secs: u64,
    /// The character store.
    pub characters_db: PathBuf,
    /// The directory with `vmangos.sqlite` and `dbc.sqlite` (`data/db` if
    /// unset and there); without it the world is empty.
    pub data_dir: Option<PathBuf>,
    /// A rustigator bake, the directory with `Azeroth.map` (`data/nav` if
    /// unset and there); without it creatures walk straight lines and zones
    /// come from the client.
    pub nav_dir: Option<PathBuf>,
    /// Thistle Tea's terrain tiles, MaNGOS z1.5 `.map` files from its
    /// `nix run .#terrain-data` (`data/terrain` if unset and there); without
    /// them liquids have no kind outside WMOs: no deep-sea fatigue, no lava
    /// in the open.
    pub terrain_dir: Option<PathBuf>,
    /// Shown to every player at login.
    pub motd: String,
    pub max_characters_per_account: usize,
    /// Names no character may take.
    pub reserved_names: Vec<String>,
    /// A PvP realm: an account's characters are all of one side.
    pub pvp_realm: bool,
    /// Signatures a guild charter needs.
    pub petition_signatures: u32,
    /// How long mail with items or money takes between accounts.
    pub mail_delay_secs: u64,
    /// The weekday honor is ranked on, each week: 1 Monday to 7 Sunday.
    pub honor_reset_weekday: u8,
}

impl WorldConfig {
    /// The data directories the config leaves out: the defaults under
    /// `data/`, where they exist.
    pub fn default_dirs(&mut self) {
        for (dir, default) in [
            (&mut self.data_dir, DEFAULT_DATA_DIR),
            (&mut self.nav_dir, DEFAULT_NAV_DIR),
            (&mut self.terrain_dir, DEFAULT_TERRAIN_DIR),
        ] {
            if dir.is_none() && Path::new(default).is_dir() {
                tracing::info!("using {default}");
                *dir = Some(PathBuf::from(default));
            }
        }
    }
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            visibility_yards: 100.0,
            save_interval_secs: 300,
            characters_db: PathBuf::from("characters.sqlite"),
            data_dir: None,
            nav_dir: None,
            terrain_dir: None,
            motd: "Welcome to wow_vanilla_server.".to_owned(),
            max_characters_per_account: 10,
            reserved_names: Vec::new(),
            pvp_realm: false,
            petition_signatures: 9,
            mail_delay_secs: 3600,
            honor_reset_weekday: crate::game::honor::DEFAULT_RESET_WEEKDAY,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AccountsConfig {
    /// Every account is an administrator. For development and load tests.
    pub everyone_gm: bool,
    /// `auto`: an unknown name gets an account at its first login, with the
    /// name as its password. `existing`: only accounts already made.
    pub creation: crate::accounts::Creation,
    /// GM levels by account: 1 moderator, 2 game master, 3 administrator.
    pub gm: std::collections::HashMap<String, u8>,
}

impl Default for AccountsConfig {
    fn default() -> Self {
        Self {
            everyone_gm: false,
            creation: crate::accounts::Creation::Auto,
            gm: Default::default(),
        }
    }
}

/// Where new characters start, instead of their race's starting zone.
#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SpawnConfig {
    pub enabled: bool,
    pub map: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub orientation: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checked_in_config_parses() {
        let text =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("config.toml"))
                .unwrap();
        toml::from_str::<ServerConfig>(&text).unwrap();
    }

    #[test]
    fn an_unknown_key_is_an_error() {
        assert!(toml::from_str::<ServerConfig>("[world]\nvisibilty_yards = 3\n").is_err());
    }
}
