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

    // Raise our own file-descriptor soft limit toward the hard limit so a
    // few thousand concurrent client sockets don't trip `EMFILE` on accept.
    // On Linux the default shell-inherited soft limit is usually 1024 while
    // the hard limit is 1 048 576 — a process can raise its own soft limit
    // up to the hard limit without root, which is exactly what we do here.
    // No-op on Windows (rlimit returns Ok with the requested value).
    match rlimit::increase_nofile_limit(1_048_576) {
        Ok(new_soft) => tracing::info!("raised fd soft limit to {new_soft}"),
        Err(e) => tracing::warn!(
            "could not raise fd limit (you may hit EMFILE under load): {e}"
        ),
    }

    let users = Arc::new(Mutex::new(AHashMap::new()));

    let auth_server = tokio::spawn(auth::auth(users.clone()));

    let world_server = tokio::spawn(world::world(users.clone()));

    let s = tokio::join!(auth_server, world_server);
    s.0.unwrap();
    s.1.unwrap();
}
