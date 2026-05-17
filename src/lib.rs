//! Library surface for the WoW vanilla server. The binary in `src/main.rs`
//! is a thin entry point; the actual modules live here so that integration
//! tests and `benches/*.rs` can reach internal types via
//! `use wow_vanilla_server::world`.

pub mod auth;
pub mod config;
pub mod file_utils;
pub mod numeric;
pub mod snapshot;
pub mod world;
