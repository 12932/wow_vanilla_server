//! Builds the server's navmesh and terrain from the player's WoW 1.12.1
//! client, asking where it is and a question or two; then starts the
//! server, if wanted.

use std::path::PathBuf;

use clap::Parser;
use wow_vanilla_server::server;
use wow_vanilla_server::setup::{Options, Selection};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Build the navmesh (data/nav) and terrain (data/terrain) from your WoW
/// 1.12.1 client. With no flags, it asks where the client is.
#[derive(Parser)]
#[command(name = "wvs-setup")]
struct Args {
    /// The client: its folder, its WoW.exe or its Data folder (or set
    /// WOW_DIR).
    #[arg(long, value_name = "PATH")]
    client: Option<PathBuf>,
    /// What to build: all, azeroth (the Eastern Kingdoms), or map names
    /// separated by commas (Kalimdor,PVPZone03).
    #[arg(long, value_name = "MAPS", value_parser = Selection::parse)]
    maps: Option<Selection>,
    /// Ask nothing, for scripts: needs --client (or WOW_DIR), and builds all
    /// the maps unless --maps says. It doesn't start the server.
    #[arg(long, short)]
    yes: bool,
    /// Build afresh what another client built, without asking (the old
    /// folder is kept beside it, as nav.old).
    #[arg(long)]
    fresh: bool,
    /// Threads to build with; every core by default.
    #[arg(long, value_name = "N")]
    threads: Option<u32>,
    /// Build nothing, into a temporary folder: to try the questions and bars.
    #[arg(long, hide = true)]
    fake: bool,
}

fn main() -> std::process::ExitCode {
    let args = Args::parse();
    let opts = Options {
        client: args.client,
        maps: args.maps,
        yes: args.yes,
        fresh: args.fresh,
        threads: args.threads,
    };
    server::main(server::Start::Setup {
        opts,
        fake: args.fake,
    })
    .into()
}
