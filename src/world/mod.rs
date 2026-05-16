pub mod aoi;
mod character_screen_handler;
pub mod command;
pub mod database;
pub mod update_object;
pub mod world_db;
#[allow(clippy::module_inception)]
mod world;
pub mod world_opcode_handler;

use crate::snapshot::{WorldSnapshot, SNAPSHOT_PATH};
use crate::world::database::WorldDatabase;
use crate::world::world::World;
use ahash::AHashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::time::sleep;
use tracing::{debug, info, warn};
use world::client::character_screen_client::CharacterScreenClient;
use wow_srp::normalized_string::NormalizedString;
use wow_srp::server::SrpServer;
use wow_srp::vanilla_header::ProofSeed;
use wow_world_messages::vanilla::tokio_expect_client_message;
use wow_world_messages::vanilla::*;

pub async fn world(users: Arc<Mutex<AHashMap<String, SrpServer>>>) {
    let listener = TcpListener::bind("0.0.0.0:8085").await.unwrap();
    info!("world: listening on 0.0.0.0:8085");
    let (world, clients_waiting_to_join) = mpsc::channel(32);

    tokio::spawn(run_world(clients_waiting_to_join));

    loop {
        let (stream, peer) = listener.accept().await.unwrap();
        debug!("world: accepted connection from {peer}");

        tokio::spawn(character_screen(stream, users.clone(), world.clone()));
    }
}

pub const DESIRED_TIMESTEP: f32 = 1.0 / 10.0;
const SAVE_INTERVAL: Duration = Duration::from_secs(60);

async fn run_world(clients_waiting_to_join: mpsc::Receiver<CharacterScreenClient>) {
    let mut db = match WorldSnapshot::load(SNAPSHOT_PATH) {
        Ok(Some(snap)) => {
            info!("Restoring characters from {SNAPSHOT_PATH}");
            snap.restore_db_only()
        }
        Ok(None) => {
            info!("No snapshot found; starting fresh");
            WorldDatabase::new()
        }
        Err(e) => {
            warn!("Failed to load {SNAPSHOT_PATH}: {e}; starting fresh");
            WorldDatabase::new()
        }
    };

    let creatures = match std::env::var("WOW_VANILLA_WORLDDB") {
        Ok(path) => match crate::world::world_db::load_creatures(&path) {
            Ok(slab) => slab,
            Err(e) => {
                warn!("worlddb load from '{path}' failed: {e}; starting with empty world");
                slab::Slab::new()
            }
        },
        Err(_) => {
            info!("WOW_VANILLA_WORLDDB unset; spawning legacy test creature only");
            let mut s = slab::Slab::new();
            s.insert(
                crate::world::world_opcode_handler::creature::Creature::new(
                    "Thing",
                    db.new_guid().into(),
                ),
            );
            s
        }
    };

    let mut world = World::with_creatures(clients_waiting_to_join, creatures);

    let shutdown = Arc::new(AtomicBool::new(false));
    {
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                info!("Ctrl-C received; world will save and exit after current tick");
                shutdown.store(true, Ordering::SeqCst);
            }
        });
    }

    let mut next_save = Instant::now() + SAVE_INTERVAL;

    loop {
        let before = Instant::now();

        world.tick(&mut db).await;

        let after = Instant::now();
        let tick_duration = after.duration_since(before);

        let final_save = shutdown.load(Ordering::SeqCst);
        if final_save || after >= next_save {
            world.sync_clients_to_db(&mut db);
            // Worlddb is authoritative for creatures — skip them in snapshot.
            let snap = WorldSnapshot::capture(&db, &slab::Slab::new());
            match snap.save(SNAPSHOT_PATH) {
                Ok(()) => tracing::debug!("Snapshot saved to {SNAPSHOT_PATH}"),
                Err(e) => warn!("Snapshot save failed: {e}"),
            }
            // Return excess slab / hashmap capacity after long-running churn.
            // This is outside the tick hot path so the cost is fine.
            world.shrink_periodic();
            next_save = after + SAVE_INTERVAL;
        }

        if final_save {
            info!("Shutdown complete");
            std::process::exit(0);
        }

        if tick_duration.as_secs_f32() < DESIRED_TIMESTEP {
            sleep(Duration::from_secs_f32(
                DESIRED_TIMESTEP - tick_duration.as_secs_f32(),
            ))
            .await;
        } else {
            warn!("Timestep took too long: '{}'", tick_duration.as_secs_f32());
        }
    }
}

async fn character_screen(
    stream: TcpStream,
    users: Arc<Mutex<AHashMap<String, SrpServer>>>,
    world: Sender<CharacterScreenClient>,
) {
    if let Err(e) = character_screen_inner(stream, users, world).await {
        // Per-connection errors are routine: clients disconnect mid-handshake,
        // send garbage, race the auth registration. Log and drop the socket.
        tracing::debug!("character_screen handshake aborted: {e}");
    }
}

async fn character_screen_inner(
    mut stream: TcpStream,
    users: Arc<Mutex<AHashMap<String, SrpServer>>>,
    world: Sender<CharacterScreenClient>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let seed = ProofSeed::new();

    SMSG_AUTH_CHALLENGE {
        server_seed: seed.seed(),
    }
    .tokio_write_unencrypted_server(&mut stream)
    .await?;

    let c = tokio_expect_client_message::<CMSG_AUTH_SESSION, _>(&mut stream).await?;
    let account_name = c.username;

    let session_key = {
        let mut server = users
            .lock()
            .map_err(|_| "users mutex poisoned".to_string())?;
        let Some(srp) = server.get_mut(&account_name) else {
            return Err(format!("unknown account '{account_name}'").into());
        };
        *srp.session_key()
    };

    let mut encryption = seed
        .into_server_header_crypto(
            &NormalizedString::new(&account_name)?,
            session_key,
            c.client_proof,
            c.client_seed,
        )
        .map_err(|e| format!("SRP handshake failed for '{account_name}': {e:?}"))?;

    SMSG_AUTH_RESPONSE {
        result: SMSG_AUTH_RESPONSE_WorldResult::AuthOk {
            billing_flags: 0,
            billing_rested: 0,
            billing_time: 0,
        },
    }
    .tokio_write_encrypted_server(&mut stream, encryption.encrypter())
    .await?;

    world
        .send(CharacterScreenClient::new(account_name, stream, encryption))
        .await
        .map_err(|e| format!("world receiver dropped: {e}"))?;
    Ok(())
}
