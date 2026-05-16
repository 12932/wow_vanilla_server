mod auth;
mod file_utils;
mod numeric;
mod snapshot;
mod world;

use ahash::AHashMap;
use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

#[tokio::main]
async fn main() {
    // Load `.env` from the working directory if present. Missing file is
    // fine — env-var-only deployments (systemd `Environment=`, container
    // `-e`, etc.) keep working. Existing process env always wins over the
    // file, so a CLI `export FOO=bar` still overrides `.env`.
    let _ = dotenvy::dotenv();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_filter(
                    EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| EnvFilter::new("info")),
                ),
        )
        .with(tracing_tracy::TracyLayer::default())
        .init();

    let users = Arc::new(Mutex::new(AHashMap::new()));

    let auth_server = tokio::spawn(auth::auth(users.clone()));

    let world_server = tokio::spawn(world::world(users.clone()));

    let s = tokio::join!(auth_server, world_server);
    s.0.unwrap();
    s.1.unwrap();
}
