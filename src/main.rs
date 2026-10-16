use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};
use wow_vanilla_server::{auth, config, game};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    // Rayon is for start-up work (loading game data); the world runs on
    // tokio, a task per map and per connection.
    rayon::ThreadPoolBuilder::new()
        .num_threads(cores)
        .thread_name(|i| format!("rayon-{i}"))
        .build_global()
        .expect("rayon global pool init");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(cores)
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(async_main());
}

async fn async_main() {
    // `.env` in the working directory, if there is one; the environment
    // wins over it.
    let _ = dotenvy::dotenv();

    // Tracy only with `WOW_TRACY=1`: without it the profiler stays dormant
    // and opens no port.
    let tracy_enabled = matches!(std::env::var("WOW_TRACY").as_deref(), Ok("1"));
    if tracy_enabled {
        let _ = tracy_client::Client::start();
    }
    let tracy_layer = tracy_enabled.then(tracing_tracy::TracyLayer::default);
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_filter(
                    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
                ),
        )
        .with(tracy_layer)
        .init();

    let cfg = match config::load(std::path::Path::new("config.toml")) {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!("{e}");
            std::process::exit(2);
        }
    };
    config::install(cfg);

    let users: auth::UserCache = Arc::new(Mutex::new(auth::UserCacheInner::new()));
    let auth_server = tokio::spawn(auth::auth(users.clone()));
    // The world returns once it has saved everyone after a shutdown signal.
    game::run(users).await;
    auth_server.abort();
}
