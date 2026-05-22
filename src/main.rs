use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};
use wow_vanilla_server::{auth, config, world};

// mimalloc: faster general-purpose allocator than the Windows default
// heap or glibc's ptmalloc. Our hot paths allocate frequently — Arc<[u8]>
// broadcast frames, AHashMap rehashes, transient Vec scratch buffers —
// and the allocator becomes a non-trivial cost. Swap is one line; we
// keep secure / encrypted / etc. features off to minimize binary size.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    // Both tokio and rayon are sized to the full core count. They share
    // the cores rather than carve them up: rayon's work-stealing parks
    // idle threads, and tokio workers also park when no tasks are
    // runnable, so the realized thread count tracks actual work — not
    // the configured pool size. The earlier static half/half split
    // assumed "one tokio worker drives the whole tick," but the world
    // tick now fans out into one tokio task per cell (see
    // `world/world/mod.rs` Stage 3) and each of those tasks calls
    // `par_iter` into rayon. A small rayon pool would then serialize
    // cells that tokio thought were running in parallel.
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    let rayon_threads = cores;
    let tokio_workers = cores;

    rayon::ThreadPoolBuilder::new()
        .num_threads(rayon_threads)
        .thread_name(|i| format!("rayon-{i}"))
        .build_global()
        .expect("rayon global pool init");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(tokio_workers)
        .enable_all()
        .build()
        .expect("tokio runtime");

    runtime.block_on(async move {
        async_main(cores, rayon_threads, tokio_workers).await;
    });
}

async fn async_main(cores: usize, rayon_threads: usize, tokio_workers: usize) {
    // Load `.env` from the working directory if present. Missing file is
    // fine — env-var-only deployments (systemd `Environment=`, container
    // `-e`, etc.) keep working. Existing process env always wins over the
    // file, so a CLI `export FOO=bar` still overrides `.env`.
    let _ = dotenvy::dotenv();

    // Tracy is gated on `WOW_TRACY=1` so a default `--release` run carries
    // zero profiler overhead AND zero network presence. Both
    // `tracy-client` and `tracing-tracy` are compiled with
    // `manual-lifetime`, which keeps the profiler dormant until
    // `tracy_client::Client::start()` is called. Without that call the
    // discovery broadcast endpoint never opens, so a Tracy GUI can't
    // even find the process. We also drop the `broadcast` default
    // feature so there's no UDP advertisement either.
    //
    // When the var IS set: start the client, then add the tracing layer
    // so spans/events route into Tracy. Important: the layer attached
    // when no GUI is listening will *queue trace events in memory*,
    // which at 1000-client fan-out can climb tens of MB per minute —
    // keep Tracy off unless a profiler is actively attached.
    let tracy_enabled = matches!(std::env::var("WOW_TRACY").as_deref(), Ok("1"));
    if tracy_enabled {
        // Idempotent — repeated calls are no-ops. Required under
        // `manual-lifetime` to bring the profiler up.
        let _ = tracy_client::Client::start();
    }
    let tracy_layer = tracy_enabled.then(tracing_tracy::TracyLayer::default);

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_filter(
                    EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| EnvFilter::new("info")),
                ),
        )
        .with(tracy_layer)
        .init();

    if tracy_enabled {
        tracing::info!("Tracy profiler enabled (WOW_TRACY=1); attach a Tracy GUI to collect");
    }

    tracing::info!(
        "thread pools: {cores} cores total -> tokio_workers={tokio_workers}, rayon_threads={rayon_threads}"
    );

    // Load behavior config (AOI radius, tick rate, combat numbers, etc.)
    // after tracing init so the load log messages are visible. Missing
    // file is fine — defaults match the prior hardcoded constants.
    config::install(config::load_or_default(std::path::Path::new("config.toml")));

    let users: auth::UserCache = Arc::new(Mutex::new(auth::UserCacheInner::new()));

    let auth_server = tokio::spawn(auth::auth(users.clone()));

    let world_server = tokio::spawn(world::world(users.clone()));

    let s = tokio::join!(auth_server, world_server);
    s.0.unwrap();
    s.1.unwrap();
}
