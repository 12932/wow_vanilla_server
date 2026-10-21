//! The server's start, for both binaries: the logs, the config, the data
//! setup (offered, or run as `wvs-setup`), the accounts, then the login
//! server and the world.

use std::sync::{Arc, Mutex};

use tracing_subscriber::Layer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

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
    // The world runs on tokio, a task per map and per connection.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(cores)
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(async_main(start))
}

async fn async_main(start: Start) -> u8 {
    // Tracy only with `WOW_TRACY=1`: without it the profiler stays dormant
    // and opens no port (docs/PROFILING.md).
    crate::tracy::start_from_env();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                // Past the setup's progress bars while they are drawn.
                .with_writer(setup::ui::LogWriter)
                // Recast's complaints, by the thousand in a navmesh build,
                // and a line an archive each time the client is opened.
                .with_filter(log_filter("info,landmark=off,wow_mpq=warn")),
        )
        .with(crate::tracy::messages())
        .init();
    // Panics in the log, with where they happened, once.
    crate::game::supervisor::install_panic_hook();

    let mut cfg = match crate::config::load(std::path::Path::new(crate::config::PATH)) {
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
    // Before anything touches the store: one server a store, and the
    // world's port ours. A second server (the next process started before
    // the last has gone) stops here, not after it has migrated the store
    // and swept the realm's records on its own copy of them.
    let _lock = match crate::game::store::lock(&cfg.world.characters_db) {
        Ok(lock) => lock,
        Err(e) => {
            tracing::error!("{e}");
            return 2;
        }
    };
    let world_address = cfg.network.world_address.as_str();
    let world = match tokio::net::TcpListener::bind(world_address).await {
        Ok(listener) => listener,
        Err(e) => {
            tracing::error!("can't listen for the world on {world_address}: {e}");
            return 2;
        }
    };
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
    let auth_server = {
        let users = users.clone();
        tokio::spawn(crate::game::supervisor::supervise(
            "the login server",
            move || crate::auth::auth(users.clone()),
        ))
    };
    // The world returns once it has saved everyone after a shutdown signal.
    let code = crate::game::run(users, world).await;
    auth_server.abort();
    code
}

/// The log's filter: `RUST_LOG` if it is set and reads, else `default`.
/// Its syntax is `Targets`': a comma-separated list of `target=level` and
/// at most one bare `level`, the default (`trace`, `debug`, `info`, `warn`,
/// `error` or `off`); a target is a module path's prefix
/// (`wow_vanilla_server::game::map=debug`). No span or field filters.
pub fn log_filter(default: &str) -> Targets {
    filter_from(std::env::var("RUST_LOG").ok().as_deref(), default)
}

fn filter_from(rust_log: Option<&str>, default: &str) -> Targets {
    let default = || default.parse().expect("the default log filter reads");
    match rust_log {
        Some(s) if !s.trim().is_empty() => s.parse().unwrap_or_else(|e| {
            eprintln!("RUST_LOG={s:?} doesn't read ({e}): the default filter instead");
            default()
        }),
        _ => default(),
    }
}

/// `--print-config`: the effective config (or with `defaults`, the
/// reference) on stdout, as commented TOML; what was clamped on stderr.
pub fn print_config(defaults: bool) -> u8 {
    if defaults {
        print!("{}", crate::config::reference());
        return 0;
    }
    match crate::config::load_full(std::path::Path::new(crate::config::PATH)) {
        Ok(loaded) => {
            for w in &loaded.warnings {
                eprintln!("warning: {w}");
            }
            print!(
                "{}",
                crate::config::render(
                    &loaded.config,
                    "wow_vanilla_server's effective settings: the defaults, config.toml over them,\n\
                     and WVS__SECTION__KEY environment variables over that.\n\
                     \n\
                     Generated by `wow_vanilla_server --print-config`. See docs/CONFIG.md."
                )
            );
            0
        }
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
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

#[cfg(test)]
mod tests {
    use super::filter_from;
    use tracing::Level;

    const DEFAULT: &str = "info,landmark=off,wow_mpq=warn";

    /// Without `RUST_LOG`, the server's default: `info`, Recast's
    /// `landmark` off and the archives' `wow_mpq` at `warn`.
    #[test]
    fn the_default_log_filter_is_info_without_landmark_and_with_wow_mpqs_warnings() {
        for unset in [None, Some(""), Some("  ")] {
            let f = filter_from(unset, DEFAULT);
            assert!(f.would_enable("wow_vanilla_server::game::map", &Level::INFO));
            assert!(!f.would_enable("wow_vanilla_server::game::map", &Level::DEBUG));
            assert!(!f.would_enable("landmark", &Level::ERROR));
            assert!(f.would_enable("wow_mpq::archive", &Level::WARN));
            assert!(!f.would_enable("wow_mpq::archive", &Level::INFO));
        }
    }

    /// `RUST_LOG` takes a bare default level and `target=level`s, a target
    /// being a module path's prefix; what doesn't read leaves the default.
    #[test]
    fn rust_log_takes_a_level_and_target_levels_and_what_doesnt_read_is_the_default() {
        let f = filter_from(Some("warn,wow_vanilla_server::game::map=debug"), DEFAULT);
        assert!(f.would_enable("wow_vanilla_server::game::map::ai", &Level::DEBUG));
        assert!(!f.would_enable("wow_vanilla_server::game::session", &Level::INFO));
        assert!(f.would_enable("wow_vanilla_server::game::session", &Level::WARN));
        let f = filter_from(Some("debug"), DEFAULT);
        assert!(
            f.would_enable("landmark", &Level::DEBUG),
            "a bare level is all"
        );
        let f = filter_from(Some("off"), DEFAULT);
        assert!(!f.would_enable("wow_vanilla_server", &Level::ERROR));
        // A span filter is env-filter's, not Targets': its brackets are part
        // of a target no module has.
        let f = filter_from(Some("wow_vanilla_server[login]=debug"), DEFAULT);
        assert!(!f.would_enable("wow_vanilla_server", &Level::ERROR));
        // A level that isn't one doesn't read: the default instead.
        let f = filter_from(Some("wow_vanilla_server=loud"), DEFAULT);
        assert!(f.would_enable("wow_vanilla_server", &Level::INFO));
        assert!(!f.would_enable("wow_vanilla_server", &Level::DEBUG));
        assert!(!f.would_enable("landmark", &Level::ERROR));
    }
}
