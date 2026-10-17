//! The world server: sessions, maps, and what they share.

pub mod battlegrounds;
pub mod conn;
pub mod data;
pub mod elemental;
pub mod events;
pub mod fields;
pub mod guid;
#[cfg(test)]
pub mod harness;
pub mod honor;
pub mod instances;
pub mod invasion;
pub mod map;
pub mod omen;
pub mod registry;
pub mod session;
pub mod social;
pub mod store;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_combat;
#[cfg(test)]
mod tests_combat2;
#[cfg(test)]
mod tests_combat3;
#[cfg(test)]
mod tests_items3;
#[cfg(test)]
mod tests_loot;
#[cfg(test)]
mod tests_objects;
#[cfg(test)]
mod tests_pets;
#[cfg(test)]
mod tests_pets2;
#[cfg(test)]
mod tests_taming;
#[cfg(test)]
mod tests_gifts;
#[cfg(test)]
mod tests_misc;
#[cfg(test)]
mod tests_misc2;
#[cfg(test)]
mod tests_pvp;
#[cfg(test)]
mod tests_scripts;
#[cfg(test)]
mod tests_spells;
#[cfg(test)]
mod tests_spells2;
#[cfg(test)]
mod tests_spells3;
#[cfg(test)]
mod tests_spells4;
#[cfg(test)]
mod tests_spells5;
#[cfg(test)]
mod tests_pvp2;
#[cfg(test)]
mod tests_pvp3;
#[cfg(test)]
mod tests_pvp4;
#[cfg(test)]
mod tests_pvp5;
#[cfg(test)]
mod tests_world;
#[cfg(test)]
mod tests_world5;
#[cfg(test)]
mod tests_arena;
#[cfg(test)]
mod tests_rules;
#[cfg(test)]
mod tests_terrain;
#[cfg(test)]
mod tests_scripts2;
#[cfg(test)]
mod tests_creatures4;
#[cfg(test)]
mod tests_bosses;
pub mod transports;
pub mod update;
pub mod weather;

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::Duration;

use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::game::data::GameData;
use crate::game::map::{MapEvent, Maps};
use crate::game::registry::{NameEntry, Registry};
use crate::game::store::{Startup, Store};

/// The settings maps and sessions read, copied out of the config.
#[derive(Debug, Clone)]
pub struct RealmConfig {
    pub visibility_yards: f32,
    pub save_interval: Duration,
    pub motd: String,
    pub everyone_gm: bool,
    pub max_characters_per_account: usize,
    pub nav_dir: Option<std::path::PathBuf>,
    /// Thistle Tea's terrain tiles (`MMMMRRCC.map`): liquid kinds and areas.
    pub terrain_dir: Option<std::path::PathBuf>,
    pub petition_signatures: u32,
    pub mail_delay: Duration,
    pub instance_unload: Duration,
    /// Players a side needs for a match, over the template's.
    pub bg_min_players: Option<u32>,
    pub bg_start_delay: Duration,
    pub pvp_realm: bool,
    /// Packets a client may send a second, sustained.
    pub max_packets_per_sec: u32,
    /// The weekday honor's week turns over on: 1 Monday to 7 Sunday.
    pub honor_reset_weekday: u8,
    /// Whether spells roll to hit, crit and resist (tests fix them).
    pub spell_rolls: bool,
    /// Whether bolts and missiles take time to arrive (tests land them at
    /// once).
    pub spell_travel: bool,
    pub arena: crate::game::map::arena::ArenaConfig,
}

impl RealmConfig {
    pub fn from_config(c: &crate::config::ServerConfig) -> Self {
        Self {
            visibility_yards: c.world.visibility_yards.clamp(20.0, 500.0),
            save_interval: Duration::from_secs(c.world.save_interval_secs.max(10)),
            motd: c.world.motd.clone(),
            everyone_gm: c.accounts.everyone_gm,
            max_characters_per_account: c.world.max_characters_per_account,
            nav_dir: c.world.nav_dir.clone(),
            terrain_dir: c.world.terrain_dir.clone(),
            petition_signatures: c.world.petition_signatures,
            mail_delay: Duration::from_secs(c.world.mail_delay_secs),
            instance_unload: crate::game::instances::UNLOAD_EMPTY_AFTER,
            bg_min_players: None,
            bg_start_delay: Duration::from_secs(120),
            pvp_realm: c.world.pvp_realm,
            max_packets_per_sec: c.network.max_packets_per_sec,
            honor_reset_weekday: c.world.honor_reset_weekday.clamp(1, 7),
            spell_rolls: true,
            spell_travel: true,
            arena: c.arena.clone(),
        }
    }
}

/// What every map and session shares.
pub struct Realm {
    pub config: RealmConfig,
    pub store: Store,
    pub data: Arc<GameData>,
    pub registry: Registry,
    pub maps: Maps,
    pub social: social::Social,
    pub next_character: AtomicU32,
    pub next_item: AtomicU32,
    /// The auth server's sessions, for the world handshake.
    pub users: Option<crate::auth::UserCache>,
}

impl Realm {
    pub fn new(
        config: RealmConfig,
        store: Store,
        startup: Startup,
        data: GameData,
        users: Option<crate::auth::UserCache>,
    ) -> Arc<Self> {
        let registry = Registry::new();
        let social = social::Social::load(&startup.records);
        social.honor.lock().set_weekday(config.honor_reset_weekday);
        let max_mail_item = social
            .mail
            .lock()
            .max_item()
            .max(social.auctions.lock().max_item());
        for n in startup.names {
            registry.add_name(NameEntry {
                guid: guid::player(n.guid),
                account: Arc::from(n.account.as_str()),
                name: Arc::from(n.name.as_str()),
                race: n.race,
                class: n.class,
                gender: n.gender,
                level: n.level.into(),
            });
        }
        let realm = Arc::new(Self {
            config,
            store,
            data: Arc::new(data),
            registry,
            maps: Maps::default(),
            social,
            next_character: AtomicU32::new(startup.max_character_guid + 1),
            next_item: AtomicU32::new(startup.max_item_guid.max(max_mail_item) + 1),
            users,
        });
        *realm.social.events.lock() = events::active(&realm.data, chrono::Utc::now().timestamp());
        social::start_timers(&realm);
        transports::start(&realm);
        realm
    }

    /// Save everyone in every map, and wait for the store to write it.
    pub async fn shutdown(&self) {
        let mut waits = Vec::new();
        for map in self.maps.all() {
            let (tx, rx) = tokio::sync::oneshot::channel();
            if map.send(MapEvent::Shutdown(tx)) {
                waits.push(rx);
            }
        }
        for w in waits {
            let _ = w.await;
        }
        self.store.flush().await;
    }
}

/// Run the world server on port 8085 until a shutdown signal, then save
/// everyone and return.
pub async fn run(users: crate::auth::UserCache) {
    let cfg = crate::config::config();
    let (store, startup) = Store::open(&cfg.world.characters_db).unwrap_or_else(|e| {
        panic!(
            "can't open the character store {}: {e}",
            cfg.world.characters_db.display()
        )
    });
    let data = match &cfg.world.data_dir {
        Some(dir) => data::load::load(dir),
        None => {
            info!("no data_dir configured: the world is empty");
            GameData::empty()
        }
    };
    let realm = Realm::new(
        RealmConfig::from_config(cfg),
        store,
        startup,
        data,
        Some(users),
    );
    // The continents start now, so the first player in doesn't wait for
    // their spawns.
    for m in [0, 1] {
        realm.maps.get(&realm, map::MapKey::world(m));
    }
    let listener = TcpListener::bind("0.0.0.0:8085")
        .await
        .expect("bind the world listener on 0.0.0.0:8085");
    info!("world: listening on 0.0.0.0:8085");
    let accept = {
        let realm = realm.clone();
        async move {
            let pending = Arc::new(tokio::sync::Semaphore::new(
                crate::config::config().network.max_pending_connections,
            ));
            loop {
                let (stream, peer) = match listener.accept().await {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("world: accept failed: {e}; retrying");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let Ok(permit) = pending.clone().try_acquire_owned() else {
                    tracing::debug!("world: too many connections in the handshake; closing {peer}");
                    continue;
                };
                let realm = realm.clone();
                // The permit covers the handshake only.
                tokio::spawn(session::serve(stream, realm, permit));
            }
        }
    };
    tokio::select! {
        _ = accept => {}
        _ = shutdown_signal() => {}
    }
    info!("shutting down: saving everyone");
    realm.shutdown().await;
    info!("saved; exiting");
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install the SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Milliseconds since the server started: the clock transports keep time
/// by.
pub fn uptime_ms() -> u32 {
    STARTED
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u32
}
