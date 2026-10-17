//! The server's start, for both binaries: the logs, the config, the data
//! setup (offered, or run as `wvs-setup`), the accounts, then the login
//! server and the world.

use std::sync::{Arc, Mutex};

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use crate::setup;

/// What the process is for.
pub enum Start {
    /// The server, offering the setup if the navmesh or terrain is missing.
    Server,
    /// `wvs-setup`: the setup, then the server if the player wants it.
    /// `fake` builds nothing, into a temporary folder.
    Setup { opts: setup::Options, fake: bool },
}

/// Run until shut down; returns the process's exit code.
pub fn main(start: Start) -> u8 {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    // Rayon is for start-up work (loading game data); the world runs on
    // tokio, a task per map and per connection.
    if let Err(e) = rayon::ThreadPoolBuilder::new()
        .num_threads(cores)
        .thread_name(|i| format!("rayon-{i}"))
        .build_global()
    {
        eprintln!("rayon's global pool: {e}");
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(cores)
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(async_main(start))
}

async fn async_main(start: Start) -> u8 {
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
                // Past the setup's progress bars while they are drawn.
                .with_writer(setup::ui::LogWriter)
                .with_filter(
                    // Recast's complaints, by the thousand in a navmesh build,
                    // and a line an archive each time the client is opened.
                    EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| EnvFilter::new("info,landmark=off,wow_mpq=warn")),
                ),
        )
        .with(tracy_layer)
        .init();
    // Panics in the log, with where they happened, once.
    crate::game::supervisor::install_panic_hook();

    let mut cfg = match crate::config::load(std::path::Path::new("config.toml")) {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!("{e}");
            return 2;
        }
    };
    match start {
        Start::Server => {
            if !tokio::task::block_in_place(|| setup::at_startup(&mut cfg.world)) {
                return 0;
            }
        }
        Start::Setup { opts, fake } => {
            if let Some(code) = tokio::task::block_in_place(|| run_setup(&opts, fake, &cfg.world)) {
                return code;
            }
            cfg.world.default_dirs();
        }
    }
    crate::config::install(cfg);
    let cfg = crate::config::config();
    match crate::accounts::Accounts::open(
        &cfg.world.characters_db,
        cfg.accounts.creation,
        cfg.accounts.everyone_gm,
        cfg.accounts.gm.clone().into_iter().collect(),
    ) {
        Ok(a) => crate::accounts::install(a),
        Err(e) => {
            tracing::error!(
                "can't open the accounts in {}: {e}",
                cfg.world.characters_db.display()
            );
            return 2;
        }
    }

    let users: crate::auth::UserCache = Arc::new(Mutex::new(crate::auth::UserCacheInner::new()));
    let auth_server = tokio::spawn(crate::auth::auth(users.clone()));
    // The world returns once it has saved everyone after a shutdown signal.
    crate::game::run(users).await;
    auth_server.abort();
    0
}

/// `wvs-setup`: `None` to go on and start the server, else the exit code.
fn run_setup(opts: &setup::Options, fake: bool, world: &crate::config::WorldConfig) -> Option<u8> {
    if !opts.yes && !setup::terminal() {
        eprintln!(
            "wvs-setup: there is no terminal to ask on. In a script, give it --yes, with --client \
             <folder> and --maps all|azeroth|<names>."
        );
        return Some(2);
    }
    let (dirs, builder): (setup::Dirs, Box<dyn setup::Builder>) = if fake {
        let tmp = std::env::temp_dir().join("wvs-setup-fake");
        eprintln!("Building nothing (--fake), into {}.", tmp.display());
        let fake = setup::Fake {
            delay: std::time::Duration::from_millis(3),
            ..Default::default()
        };
        let dirs = setup::Dirs {
            nav: tmp.join("nav"),
            terrain: tmp.join("terrain"),
        };
        (dirs, Box::new(fake))
    } else {
        (setup::Dirs::of(world), Box::new(setup::Rustigator))
    };
    match setup::run(opts, &dirs, builder.as_ref()) {
        Ok(setup::Outcome::Built) if !opts.yes && !fake => {
            match setup::ui::confirm("Start the server now?", true) {
                Some(true) => None,
                _ => Some(0),
            }
        }
        Ok(setup::Outcome::Built) => {
            if !fake {
                eprintln!("Start the server with `cargo run --release`.");
            }
            Some(0)
        }
        Ok(setup::Outcome::Declined) => Some(0),
        Ok(setup::Outcome::Stopped) => Some(130),
        Err(e) => {
            eprintln!("wvs-setup: {e}.");
            Some(1)
        }
    }
}
