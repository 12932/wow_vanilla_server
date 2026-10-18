use clap::Parser;
use wow_vanilla_server::server;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// A WoW 1.12.1 server. Its settings are config.toml, and WVS__SECTION__KEY
/// environment variables over it (see docs/CONFIG.md).
#[derive(Parser)]
#[command(name = "wow_vanilla_server")]
struct Args {
    /// Print the effective config (the defaults, config.toml over them, the
    /// environment over that) as commented TOML: every key, its value,
    /// default, range and meaning. Then exit.
    #[arg(long)]
    print_config: bool,
    /// With --print-config: the defaults alone, as config.example.toml.
    #[arg(long, requires = "print_config")]
    defaults: bool,
}

fn main() -> std::process::ExitCode {
    let args = Args::parse();
    if args.print_config {
        return server::print_config(args.defaults).into();
    }
    server::main(server::Start::Server).into()
}
