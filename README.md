# wow_vanilla_server

A WoW vanilla (1.12.2 client) server in Rust. Single-process Tokio runtime, 10 Hz world tick, mangos worlddb SQLite for creature spawns, Tracy profiling baked in. Ships a load-test harness in the same workspace that spawns thousands of real-protocol bot clients for scale testing.

This is a development project, not a production server. It will lose data, change wire-protocol details, and break old snapshots. If you want to run a public realm, use cmangos / vmangos / TrinityCore.

## What works today

- SRP6 logon + ARC4-encrypted world handshake.
- Character create / delete / login (Northshire spawn).
- Position broadcasts (start/stop/strafe/heartbeat/turn/jump), per-tick movement coalescer, AOI broadcasts.
- Basic combat: melee auto-attack against creatures, damage and kill.
- Pathfinding via [namigator](https://github.com/gtker/namigator-rs) when a vanilla client data tree is pointed at via `WOW_VANILLA_USE_MAPS`.
- GM commands (chat-window `.command` syntax) for spawning, teleporting, marking locations, simulating puppet players, and assorted debug helpers.
- Character + account persistence to `snapshot.bin` (postcard, saved every 60 s).
- ~51 k creatures loaded from a mangos `world` DB dump (one-time SQLite import on boot).
- Real-protocol load-test harness (`loadtest` binary) for scaling tests.

## Quick start (local development)

```sh
git clone <this repo>
cd wow_vanilla_server

# Build both binaries
cargo build --release --bin wow_vanilla_server
cargo build --release --bin loadtest

# Run the server
./target/release/wow_vanilla_server
```

The server listens on:

- `0.0.0.0:3724` — auth (logon)
- `0.0.0.0:8085` — world

Point your 1.12.2 client's `realmlist.wtf` at the auth host (e.g. `set realmlist 127.0.0.1` for the loopback case). Auth replies with whatever `WOW_REALM_ADDRESS` is set to — see [Configuration](#configuration) below.

A default `admin` account isn't shipped; set `WOW_AUTH_AUTO_CREATE=1` and any username you type at the login screen will be created on the fly with password = username. That mode is for development only.

## Configuration

The server reads its config from environment variables. You can set them inline, via systemd `Environment=` directives, or by dropping a `.env` file next to the binary's working directory — `dotenvy` loads it at startup. A `.env.example` template is checked in.

| Variable | Default | Purpose |
|---|---|---|
| `WOW_REALM_ADDRESS` | `localhost:8085` | `host:port` advertised inside the realm-list payload. Must be reachable from every client. Set to your public IP + `8085` when deploying. |
| `WOW_AUTH_AUTO_CREATE` | unset | If set to any value, unknown usernames are auto-created on logon. Required for the `loadtest` binary. |
| `WOW_VANILLA_WORLDDB` | unset | Path to a mangos worlddb SQLite dump. Without it the server boots with one test wolf only. `.cargo/config.toml` sets this for local development. |
| `RUST_LOG` | `info` | Standard `tracing-subscriber` env filter. `RUST_LOG=info,wow_vanilla_server=debug` adds per-connection debug spam. |
| `WOW_TRACY` | unset | Set to `1` to start the Tracy profiler at boot. Leave unset for production — the `TracyLayer` queues trace events in memory when no GUI is connected and is the dominant memory-growth source at high client counts. Build flags don't change; same `--release` binary works either way. |
| `LOADTEST_NAME_PREFIX` | hostname | Loadtest worker: bot character-name prefix. Defaults to the worker host's hostname (via the `gethostname` crate). |

`WOW_VANILLA_USE_MAPS` is a **compile-time** `option_env!` for the namigator build — if set when you `cargo build`, the resulting binary will use the pointed-at vanilla client data for terrain following. Without it, mobs walk linearly through z.

## Deployment example

Deploying to a remote host (e.g. a Hetzner cloud box) — replace `YOUR.SERVER.IP` with your box's public IP:

```sh
# On the server, in the repo root:
cat > .env <<EOF
WOW_REALM_ADDRESS=YOUR.SERVER.IP:8085
WOW_AUTH_AUTO_CREATE=1
EOF

cargo build --release --bin wow_vanilla_server
./target/release/wow_vanilla_server
```

You should see:

```
INFO auth: advertising world server address 'YOUR.SERVER.IP:8085' in realm list
INFO world: listening on 0.0.0.0:8085
INFO Restoring characters from snapshot.bin   (or "No snapshot found; starting fresh")
```

If your `.env` isn't being picked up, the startup log will print `localhost:8085` instead — `dotenvy` looks in the **current working directory**, not next to the binary. Run from the repo root.

On the client side, set `realmlist.wtf` to `YOUR.SERVER.IP`. Open ports `3724/tcp` (auth) and `8085/tcp` (world) on your cloud firewall.

## Load-test harness

The `loadtest` binary opens many real SRP6/ARC4-encrypted client sessions against a running server. Each bot creates a character at Northshire and runs a random-walk movement driver indistinguishable from a real client to the server.

### Standalone worker (single machine)

```sh
./target/release/loadtest \
    --role worker \
    --target YOUR.SERVER.IP:3724 \
    --clients 400 \
    --ramp-up 30
```

Per-second metrics print to the worker's stdout:

```
[worker-1] t=12s | alive 87/400 | auth 87ok/0fail | world 87ok/0fail | msgs in/s 1240 out/s 612 | send_err 0
```

`--ramp-up 0` spawns the entire batch as fast as the OS will let you (limited by file-descriptor count, see below).

### Orchestrator + workers (multi-host)

Run one orchestrator process anywhere reachable from your worker hosts:

```sh
./target/release/loadtest --role orchestrator --bind 0.0.0.0:7100
```

Then run workers that register against it:

```sh
./target/release/loadtest --role worker \
    --orchestrator 127.0.0.1:7100 \
    --target YOUR.SERVER.IP:3724 \
    --worker-id us-east-1
```

The orchestrator has a stdin REPL — `spawn 100 all`, `stop 50 all`, `status`, `quit`.

### File-descriptor limit (Linux)

Default `ulimit -n` is 1024. The server uses one fd per connected client (briefly two during the auth→world handshake), so you'll cap at ~510 connections on a default Linux box. **The server does not raise its own fd limit** — raise it manually before starting both the server and any load-test workers:

```sh
ulimit -n 65536    # current shell only
./target/release/wow_vanilla_server
```

Or system-wide in `/etc/security/limits.conf`:

```
* soft nofile 65536
* hard nofile 65536
```

Under systemd, set `LimitNOFILE=65536` in the unit file. For testing thousands of bots, push it higher (e.g. `1048576`).

## Profiling

The server is wired up to [Tracy](https://github.com/wolfpld/tracy) via the `tracing-tracy` and `tracy-client` crates, but the profiler is **off by default**. Set `WOW_TRACY=1` in the environment before starting the server to enable it — `World::tick` and every phase (`per_client_loop`, `tick_walking_creatures`, `flush_movement_broadcasts`, etc.) are instrumented and will show up in the GUI. Same `--release` binary works either way; the gate is purely runtime. Start Tracy's GUI first (or shortly after) so it picks up the connection. Leave `WOW_TRACY` unset for production — when no GUI is attached, the layer queues trace events in memory and is the dominant memory-growth source at high client counts.

If a tick exceeds 100 ms, the server also logs a per-phase breakdown at `WARN`:

```
slow tick total=124.3ms drain=0.1 chrscreen=2.1 promote=87.6 per_client=12.0 apply=0.3 corpses=0.8 creatures=15.2 sims=0.0 logouts=0.0 | clients=400 sims_n=0 creatures_active=812
```

Useful for diagnosing without attaching the GUI.

## In-game GM commands

Type these in chat as a logged-in character.

| Command | What it does |
|---|---|
| `.whereami` | Print your map + coordinates. |
| `.tp <named-location>` | Teleport to a named position (Stormwind etc.). See `unadded_locations.txt` for candidate names. |
| `.go <x> <y> <z> [map]` | Teleport to coordinates. With no args, goes to your target's position. |
| `.north / .south / .east / .west` | Step 5 yards in a cardinal direction. |
| `.extend [dist]` | Step `dist` yards forward (default 5). |
| `.float [dist]` | Rise `dist` yards in z. |
| `.speed <multiplier>` | Set your run speed (1.0 = normal). |
| `.range` | Distance to your current target. |
| `.info [guid]` | Dump info for your target or the given guid. |
| `.additem <id-or-name>` | Add an item to your inventory. |
| `.spawn [display_id] [name]` | Spawn a creature at your position. |
| `.move` | Make your selected creature walk a short path. |
| `.boom` | Cast a damaging spell on your target. |
| `.nova` | Frost Nova: damage + root nearby clients. |
| `.simulate <N>` | Spawn N server-side puppet players that walk a route. |
| `.simclear` | Despawn all simulated players. |
| `.mark <name1>[,<name2>]` | Append the current position to `unadded_locations.txt` so it can be folded into `wow_world_base` later. |
| `.los` / `.nolos` | Check line of sight to your target. |
| `.worlddbinfo` | Print mangos worlddb load stats. |
| `.swifty` | Every connected player yells "swifty invasion". Demo/stress command. |

## Development workflow

```sh
cargo check                              # fastest feedback
cargo clippy --all-targets               # must be clean before commit
cargo nextest run                        # test runner (faster than cargo test)
cargo build --release --bin <name>       # for actual gameplay testing
```

Don't commit:

- `target/` (auto)
- `snapshot.bin` (runtime state)
- `mangos0.sqlite` (~30 MB, per-developer copy)
- `*.tracy` / `*.tracy.zstd` (CPU profiles)
- `.env` (host-specific config)

All of those are in `.gitignore`.

## Project layout

```
src/
  main.rs                          # bin entry, dotenvy, tracing init
  auth.rs                          # logon listener, SRP6, realm list
  snapshot.rs                      # postcard save/load
  numeric.rs                       # rand-unit-f32, narrowing helpers
  world/
    mod.rs                         # world TCP listener + accept loop, tick scheduler
    aoi.rs                         # broadcast_within_aoi (typed + opcode variants)
    command.rs                     # WorldCommand + CommandQueue
    update_object.rs               # SMSG_UPDATE_OBJECT / compressed batching
    world_db.rs                    # mangos SQLite loader
    character_screen_handler/      # CMSG_CHAR_ENUM/CREATE/DELETE/LOGIN
    world/
      mod.rs                       # World struct + tick implementation
      client/
        mod.rs                     # PlayerSession + Player + Client
        character_screen_client.rs # writer task spawn, reader task
      pathfinding_maps.rs          # namigator wrapper
    world_opcode_handler/
      mod.rs                       # send_to_all, write_*_test helpers
      opcode_handler.rs            # big opcode match
      entities.rs                  # Entities wrapper, queue_movement
      character.rs                 # Character struct, auto_attack_timer
      creature.rs                  # Creature behaviors (Wander/Waypoint/Aggro/Idle)
      chat.rs                      # CMSG_MESSAGECHAT (say/yell)
      inventory.rs, item.rs        # inventory + item handling
      simulated_player.rs          # server-side puppet horde
      gm_command/                  # `.command` parser + handlers
  loadtest/
    main.rs                        # CLI, role dispatch
    protocol.rs                    # orchestrator ↔ worker frames
    orchestrator/mod.rs            # TCP control plane, registry, REPL
    worker/
      mod.rs                       # spawn_n, drain, metrics tick
      bot.rs                       # per-bot state machine
      auth.rs                      # SRP6 client, realm list parse
      world.rs                     # ARC4 handshake, char create, login
      movement.rs                  # random-walk driver, heartbeat
      metrics.rs                   # atomic counters
```

## Credits

Built on top of [@gtker](https://github.com/gtker)'s suite:
- [`wow_messages`](https://github.com/gtker/wow_messages) — generated wire types for every WoW version.
- [`wow_srp`](https://github.com/gtker/wow_srp) — SRP6 + ARC4 header crypto.
- [`namigator`](https://github.com/gtker/namigator-rs) — Rust bindings for the namigator pathfinding library.

The mangos worlddb dump (`mangos0.sqlite`) originates from the [cmangos](https://github.com/cmangos) project's `mangos-0.21` branch.

## License

Same terms as the upstream `wow_vanilla_server` project this was forked from. Treat the worlddb dump and any client data files according to their respective licenses.
