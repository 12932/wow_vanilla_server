# wow_vanilla_server

A WoW vanilla (1.12.2) server in Rust. Single-process Tokio runtime, 10 Hz world tick, mangos worlddb SQLite for creature spawns. Ships a load-test harness in the same workspace that opens thousands of real-protocol bot clients for scale testing.

Development project — not a production server. Use cmangos / vmangos / TrinityCore if you want a public realm.

## What works

- SRP6 logon + ARC4-encrypted world handshake
- Character create / delete / login, Gurubashi Arena spawn
- Movement broadcasts (start/stop/strafe/heartbeat/turn/jump) with per-tick coalescing + AOI transitions
- Basic PvP melee combat (server-side range check with movement leeway, FFA flag)
- Pathfinding via [namigator](https://github.com/gtker/namigator-rs) when client maps are pointed at via `WOW_VANILLA_USE_MAPS` at build time
- `.command` GM chat commands (spawn, teleport, simulate, etc.)
- Character + account persistence to `snapshot.bin` (postcard, every 60 s)
- ~51 k creatures from a mangos worlddb dump
- Real-protocol load-test harness (`loadtest` binary)

## Quick start

```sh
cargo build --release --bin wow_vanilla_server
./target/release/wow_vanilla_server
```

Listens on `:3724` (auth) and `:8085` (world). Point your 1.12.2 `realmlist.wtf` at the auth host. With `WOW_AUTH_AUTO_CREATE=1`, any username at the login screen is created on the fly (password = username).

For remote deploys set `WOW_REALM_ADDRESS=YOUR.SERVER.IP:8085` in `.env`. On Linux, raise the fd limit (`ulimit -n 65536`) before testing at scale — default 1024 caps you around 510 connections.

## Configuration

`.env` next to the working dir, or normal env vars / systemd `Environment=`. Template in `.env.example`.

| Variable | Default | Purpose |
|---|---|---|
| `WOW_REALM_ADDRESS` | `localhost:8085` | host:port advertised in the realm list |
| `WOW_AUTH_AUTO_CREATE` | unset | auto-create unknown usernames on logon (required by loadtest) |
| `WOW_VANILLA_WORLDDB` | unset | path to mangos worlddb SQLite dump |
| `WOW_TRACY` | unset | set to `1` to enable the Tracy profiler |
| `RUST_LOG` | `info` | `tracing-subscriber` env filter |
| `LOADTEST_NAME_PREFIX` | hostname | loadtest worker character-name prefix |

`WOW_VANILLA_USE_MAPS` is a **compile-time** `option_env!` consumed by the namigator build — set it before `cargo build` to enable terrain following.

### Behavior tuning (`config.toml`)

Gameplay knobs (AOI radius, tick rate, combat numbers, respawn delays, spawn point, etc.) live in `config.toml` next to the binary. Every key is optional — omit the file entirely to get the defaults. See `config.toml.example` for the full list. No hot reload; restart the server to apply changes.

## Load test

```sh
# Standalone single-host worker
./target/release/loadtest --role worker --target IP:3724 --clients 400 --ramp-up 30

# Multi-host: orchestrator + workers
./target/release/loadtest --role orchestrator --bind 0.0.0.0:7100
./target/release/loadtest --role worker --orchestrator HOST:7100 --target IP:3724 --worker-id us-east-1
```

Orchestrator REPL: `spawn N all`, `stop N all`, `status`, `quit`. Add `--pvp` to a worker for the Gurubashi free-for-all bot driver.

## Profiling

`WOW_TRACY=1 ./target/release/wow_vanilla_server` enables the [Tracy](https://github.com/wolfpld/tracy) profiler. Without the env var the profiler is fully off — no listener, no broadcast. Same `--release` binary works either way.

If a tick exceeds 100 ms, the server also logs a phase-by-phase breakdown at `WARN`.

## GM commands

In-game chat (`.command` syntax):

| Command | Effect |
|---|---|
| `.whereami` | Print map + coordinates |
| `.tp <name>` / `.go x y z [map]` | Teleport (named / coordinates) |
| `.north/.south/.east/.west`, `.extend [d]`, `.float [d]` | Step in a direction |
| `.speed <mul>` | Run-speed multiplier |
| `.range`, `.info [guid]` | Info about target |
| `.additem <id-or-name>` | Add item to inventory |
| `.spawn [display_id] [name]` | Spawn a creature |
| `.boom`, `.nova` | Damage spells |
| `.mark <name>` | Append current pos to `unadded_locations.txt` |
| `.los`, `.nolos`, `.worlddbinfo`, `.swifty` | Misc utilities |

## Development

```sh
cargo clippy --all-targets    # must be clean
cargo nextest run             # faster than cargo test
```

`.gitignore` covers runtime state (snapshot, mangos sqlite, tracy captures, `.env`).

## Credits

Built on [@gtker](https://github.com/gtker)'s suite: [`wow_messages`](https://github.com/gtker/wow_messages), [`wow_srp`](https://github.com/gtker/wow_srp), [`namigator`](https://github.com/gtker/namigator-rs). Worlddb dump from [cmangos](https://github.com/cmangos) `mangos-0.21`.
