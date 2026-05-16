//! Real-network load test harness for the WoW vanilla server.
//!
//! Two roles share one binary:
//! * `--role worker` — opens N SRP6/ARC4-encrypted client sessions against
//!   the running server, creates one character per session at Northshire,
//!   and walks them randomly.
//! * `--role orchestrator` — accepts worker connections, broadcasts
//!   spawn/stop/drain commands from a stdin REPL, prints aggregated metrics.

mod orchestrator;
mod protocol;
mod worker;

use clap::{Parser, ValueEnum};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Role {
    Worker,
    Orchestrator,
}

#[derive(Debug, Parser)]
#[command(name = "loadtest", about = "WoW vanilla server load tester", version)]
struct Cli {
    #[arg(long, value_enum)]
    role: Role,

    /// Worker-only: auth server address (host:port).
    #[arg(long, default_value = "127.0.0.1:3724")]
    target: String,

    /// Worker-only: number of bots to spawn at startup.
    #[arg(long, default_value_t = 1)]
    clients: u32,

    /// Worker-only: seconds over which to ramp up a spawn batch. With
    /// `--clients 400 --ramp-up 60`, bots come online evenly over 60 seconds
    /// (so ~6.7 bots/sec). Default 0 spawns the whole batch as fast as the
    /// OS will let us — no inter-bot delay at all.
    #[arg(long, default_value_t = 0)]
    ramp_up: u32,

    /// Worker-only: stable identifier reported to the orchestrator.
    #[arg(long, default_value = "worker-1")]
    worker_id: String,

    /// Worker-only: username prefix, e.g. `BOT` → `BOT0001`, `BOT0002`, ...
    #[arg(long, default_value = "BOT")]
    username_prefix: String,

    /// Worker-only: orchestrator address; if omitted, worker runs standalone.
    #[arg(long)]
    orchestrator: Option<String>,

    /// Worker-only: override the world address returned by the auth realm
    /// list. Useful when auth advertises a public hostname but you want
    /// connections to localhost.
    #[arg(long)]
    world_addr: Option<String>,

    /// Orchestrator-only: bind address for the worker control plane.
    #[arg(long, default_value = "0.0.0.0:7100")]
    bind: String,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_target(false).with_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        ))
        .init();

    let cli = Cli::parse();
    match cli.role {
        Role::Worker => {
            let cfg = worker::WorkerConfig {
                worker_id: cli.worker_id,
                auth_addr: cli.target,
                world_addr_override: cli.world_addr,
                username_prefix: cli.username_prefix,
                initial_clients: cli.clients,
                ramp_up_secs: cli.ramp_up,
                orchestrator: cli.orchestrator,
            };
            worker::run(cfg).await
        }
        Role::Orchestrator => {
            let cfg = orchestrator::OrchestratorConfig { bind: cli.bind };
            orchestrator::run(cfg).await
        }
    }
}
