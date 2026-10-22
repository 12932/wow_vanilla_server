//! Library surface for the WoW vanilla server. The binaries (`src/main.rs`,
//! and `wvs-setup`) are thin entry points; the modules live here so that
//! tests can reach them.

pub mod accounts;
pub mod auth;
pub mod config;
pub mod game;
pub mod numeric;
pub mod priority;
pub mod server;
pub mod setup;
pub mod tracy;

/// The server's version and the commit it was built from, as
/// `0.1.35 (c31f3df)`: the crate's version and git's short hash
/// (`build.rs`).
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("WVS_GIT_HASH"), ")");
