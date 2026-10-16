//! Library surface for the WoW vanilla server. The binary in `src/main.rs`
//! is a thin entry point; the modules live here so that tests can reach
//! them.

pub mod accounts;
pub mod auth;
pub mod config;
pub mod game;
pub mod numeric;
