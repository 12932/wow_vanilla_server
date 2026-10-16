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
    match std::fs::read_to_string(path) {
        Ok(s) => {
            let cfg = toml::from_str::<ServerConfig>(&s)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            tracing::info!("loaded config from {}", path.display());
            Ok(cfg)
        }
        Err(_) => {
            tracing::info!("no config file at {}; using defaults", path.display());
            Ok(ServerConfig::default())
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub network: NetworkConfig,
    pub world: WorldConfig,
    pub accounts: AccountsConfig,
    pub spawn: SpawnConfig,
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
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            outbound_channel_bytes: 4 * 1024 * 1024,
            handshake_timeout_secs: 10,
            client_idle_timeout_secs: 120,
            max_pending_connections: 1024,
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
    /// The directory with `vmangos.sqlite` and `dbc.sqlite`; without it the
    /// world is empty.
    pub data_dir: Option<PathBuf>,
    /// A rustigator bake (the directory with `Azeroth.map`); without it
    /// creatures walk straight lines and zones come from the client.
    pub nav_dir: Option<PathBuf>,
    /// Shown to every player at login.
    pub motd: String,
    pub max_characters_per_account: usize,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            visibility_yards: 100.0,
            save_interval_secs: 300,
            characters_db: PathBuf::from("characters.sqlite"),
            data_dir: None,
            nav_dir: None,
            motd: "Welcome to wow_vanilla_server.".to_owned(),
            max_characters_per_account: 10,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AccountsConfig {
    /// Every account may use GM commands. For development and load tests.
    pub everyone_gm: bool,
}

impl Default for AccountsConfig {
    fn default() -> Self {
        Self { everyone_gm: true }
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
