//! Library surface for the WoW vanilla server. The binaries (`src/main.rs`,
//! and `wvs-setup`) are thin entry points; the modules live here so that
//! tests can reach them.

pub mod accounts;
pub mod auth;
pub mod config;
pub mod game;
pub mod numeric;
#[cfg(feature = "pprof")]
pub mod profile;
pub mod server;
pub mod setup;
pub mod tracy;
