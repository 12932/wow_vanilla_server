# AGENTS.md

Guidance for AI coding assistants (Claude Code, Cursor, Aider, etc.) working in this repo. Pair with the developer's personal `~/.claude/CLAUDE.md` for global preferences (test runner, hashing crate, etc.).

## What this is

A WoW vanilla (1.12.2 client) server in Rust. Two binaries: `wow_vanilla_server` (the game server) and `loadtest` (an orchestrator + worker for spawning real-protocol bots against the server). Tokio async runtime. Single-threaded tick at **10 Hz** with rayon for one parallel phase (`tick_creature_ai`). Roughly 51 k creatures loaded from a mangos SQLite worlddb on boot. Tracy profiler is enabled.

## Quick commands

```sh
cargo check                              # fast feedback
cargo clippy --all-targets               # lint — must be clean before commit
cargo nextest run                        # test runner of choice (faster than `cargo test`)
cargo build --release --bin wow_vanilla_server
cargo build --release --bin loadtest
```

The `wowm-capture` cargo feature gates a hot-path debug scaffold that walks `../wow_messages` per outgoing packet. **Never enable it for a real run**:

```sh
cargo check --features wowm-capture      # only when expanding the wow_messages test corpus
```

## Architecture map

Where to look when adding things:

| Concern | File |
|---|---|
| Boot, snapshot, save loop, env (`dotenvy`) | `src/main.rs`, `src/world/mod.rs::run_world` |
| Auth listener, SRP6, realm list | `src/auth.rs` |
| Tick stages, AOI broadcast, login streaming | `src/world/world/mod.rs::tick` (~1500 lines) |
| Per-client state, network I/O | `src/world/world/client/mod.rs` (`PlayerSession`, `Player`, `Client`) |
| Per-client outbound writer task + channel | `src/world/world/client/character_screen_client.rs` |
| Pre-game (char screen) flow | `src/world/character_screen_handler/mod.rs` |
| Opcode dispatch | `src/world/world_opcode_handler/opcode_handler.rs` (big `match`, plan to split per-category) |
| GM commands | `src/world/world_opcode_handler/gm_command/` (parser + handlers) |
| World mutation queue | `src/world/command.rs` (`WorldCommand` + `CommandQueue`) |
| Creature data model + AI | `src/world/world_opcode_handler/creature.rs` |
| Server-side puppet horde | `src/world/world_opcode_handler/simulated_player.rs` |
| Entity lookups | `src/world/world_opcode_handler/entities.rs` |
| Worlddb SQLite loader | `src/world/world_db.rs` |
| AOI broadcast + serialize-once helpers | `src/world/aoi.rs` (`broadcast_within_aoi`, `broadcast_opcode_within_aoi`) |
| Compressed update objects | `src/world/update_object.rs` |
| Pathfinding / terrain queries | `src/world/world/pathfinding_maps.rs` |
| Numeric helpers | `src/numeric.rs` |
| Load-test orchestrator + worker | `src/loadtest/` |

## Conventions

### Command bus, not parallel `Vec`s

`World` mutation (spawn, kill, despawn) does not happen inline in handlers. Push a `WorldCommand` onto the `CommandQueue` instead:

```rust
commands.push(WorldCommand::SpawnCreature(creature));
commands.push(WorldCommand::KillCreature(guid));
```

`World::apply_commands` is the single place that mutates the slabs and broadcasts the matching protocol messages. Adding a new deferred action means one enum variant + one match arm — do not reintroduce `pending_*: &mut Vec<…>` arguments through the handler signatures.

### Entity guid lookups must use the indexes

`World` carries reverse `AHashMap<Guid, usize>` indexes for players, creatures, and simulated_players. `WorldDatabase` also carries `by_guid` and `by_account` indexes for characters. Use them via `Entities::find_player` / `find_creature` / `find_simulated` and `WorldDatabase::get_character_by_guid` / `get_characters_for_account` (all O(1) or O(chars-for-this-account)). **Do not write `slab.iter().find_map(|(_, c)| c.guid == g)` or linear `characters.iter().filter(|c| c.account == name)`** — at 51 k creatures or burst-login that's the dominant cost.

If you insert/remove from one of those slabs, update the matching reverse index in lockstep. Use `World::register_creature` / `World::unregister_creature` for creatures; `WorldDatabase::create_character_in_account` / `delete_character_by_guid` for characters (the latter handles the `swap_remove` index fixup).

### Per-client outbound channel

Each connected client owns:
- A `mpsc::Sender<Vec<u8>>` on `PlayerSession.outbound`. World-tick code calls `send_message` / `send_opcode` / `send_raw` / `try_queue_frame`, all of which serialize into a `Vec<u8>` and `try_send` non-blocking.
- A dedicated **writer task** (spawned in `CharacterScreenClient::new`) that owns the socket write half + ARC4 `EncrypterHalf`, drains the channel, and re-encrypts the 4-byte header per item.
- A `dropped_packets: Arc<AtomicU64>` counter — incremented when the channel is full (slow client). Log via the counter, never panic.

**Never `.await` a socket write from inside `World::tick`.** Always go through the channel. If the channel fills, drop the packet — that's by design.

### Per-tick movement coalescer

Every movement opcode handler in `opcode_handler.rs` queues into `entities.queue_movement(client, msg.into())` rather than broadcasting inline. After `per_client_loop` finishes, the `flush_movement_broadcasts` phase drains `World.pending_movement` (keyed by source `Guid`) and dispatches each entry via `aoi::broadcast_opcode_within_aoi(..., Some(source_guid), ...)`.

Two invariants:

1. **At most one movement broadcast per source per tick.** Same-tick opcodes for one source replace each other in the map. The latest `MovementInfo` is authoritative for observers; HEARTBEAT carries full flags + position.
2. **Always exclude the source guid.** Self-echo of `MSG_MOVE_*_Server` is read by the local WoW client as a server position correction → rubber-band. The flush phase passes the map key as the `exclude_guid` parameter.

### Numeric policy

Wire types match `wow_world_messages` exactly (`u32`/`u64`/`f32`). At boundaries:

1. **Narrowing casts**: never `as`. Use `i32::try_from(v).unwrap_or(i32::MAX)` or `.clamp()`.
2. **SQLite `i64` → narrow**: use the local helpers in `world_db.rs` (`i64_to_u32`, `i64_to_u8`, `i64_to_i32`). They clamp into range — a mangos row with `MinLevel = -1` becomes `0`, not `255`.
3. **`u64` random → `f32` in `[0, 1)`**: use `numeric::rand_unit_f32(r)`. Naive `r as f32 / u64::MAX as f32` collapses the top bit.
4. **`f32` → integer**: Rust 1.45+ float casts saturate (`NaN as uN == 0`, `+inf as uN == uN::MAX`). `.max(0.0)` guards negative input; explicit `is_finite()` check optional.

### Error handling

Per-tick / per-packet errors **never panic**. The policy:

- **Startup**: `.expect("explanatory message")` is fine for unrecoverable conditions.
- **Per-connection**: log at `debug` or `warn` and drop the connection. See `or_return!` macro in `src/auth.rs` and the pattern in `character_screen_inner`.
- **Per-packet**: log at `debug` and skip the packet. Client-supplied guids must be looked up fallibly (`db.get_character_by_guid` returns `Option`).
- **Per-tick**: log at `warn` and continue with the remaining work. `World::tick` has a slow-tick `WARN` that dumps per-phase wall time when the whole tick exceeds 100 ms.

### Async + locks

- The per-client writer task **owns** the socket write half — no `Arc<Mutex<…>>` indirection any more.
- `Arc<std::sync::Mutex<AHashMap<String, SrpServer>>>` for the SRP user map. Hold the lock briefly; never across `.await`.
- **Never hold a sync mutex across `.await`** — review will reject this.

### AOI broadcasts

Two helpers in `src/world/aoi.rs`:

- `broadcast_within_aoi<M: ServerMessage>(msg, anchor, map, clients)` — generic, serializes the concrete message body once and reuses the bytes across viewers. Use this for any code path holding a typed `MSG_MOVE_*_Server` / `SMSG_*` value.
- `broadcast_opcode_within_aoi(&ServerOpcodeMessage, anchor, map, exclude_guid, clients)` — opcode-enum variant. Used by `flush_movement_broadcasts` because the coalescer stores erased `ServerOpcodeMessage` values. Always pass `Some(source_guid)` to prevent self-echo.

**Do not write a per-viewer loop that calls `c.send_message(msg.clone())`.** It's the pattern these helpers replaced.

For batches of `Object` payloads (login streaming, spawn broadcasts) use `UpdateObject::from_objects(...)` — it picks `SMSG_COMPRESSED_UPDATE_OBJECT` vs plain based on payload shape.

### Per-tick scratch buffers

`tick_walking_creatures` and `tick_simulated_players` previously allocated `Vec`/`HashMap`/`HashSet` per tick. These are now held on `World` as `scratch_*` fields and reused tick-over-tick:

- `scratch_client_aabb: AHashMap<Map, (f32, f32, f32, f32)>` — per-map AOI bounding box.
- `scratch_walk_events: Vec<(usize, Vector3d, Map, CreatureMoveEvent)>`
- `scratch_to_park: Vec<(Instant, usize)>`
- `scratch_parked_set: AHashSet<usize>`
- `scratch_expired_roots: Vec<(Guid, Map, Vector3d, MovementInfo)>`

Pattern: take by `mem::take(&mut self.scratch_*)` at phase top, `.clear()`, fill, consume, store back at phase bottom so the inner allocations persist for the next tick.

### Snapshot / persistence

- Postcard, single `snapshot.bin` at the workspace root. Saved every **60 s**.
- Worlddb creatures are **not** persisted (SQLite is authoritative). Player characters + `next_guid` are.
- `WorldSnapshot::capture` is called with an empty creature slab. Do not pass `world.creatures()` again — you'll bloat the snapshot and shadow worlddb edits.
- Snapshot save log is at `debug` level. The startup "Restoring from snapshot.bin" log is at `info`.
- This project does **not** version snapshots. Adding a field to `CharacterSnapshot` will break existing saves. Fine in development; revisit before any external user has a save.

### `Character` field visibility

`Character` fields are `pub` today. Mutating health/max_health/level without an accessor is allowed but discouraged — invariants like `health <= max_health` are not enforced. If you're touching this area, prefer adding `Player::apply_damage(amount)` over direct field writes.

### Auto-attack timer is wall-clock-driven

`Character::update_auto_attack_timer(dt: f32)` takes a `dt` argument. `World::tick` passes the measured wall-clock duration since the previous tick (clamped to 1 s). The configured target interval `TARGET_INTERVAL` (currently 100 ms) is only used as the bootstrap value for the very first tick. **Do not** read tick-rate constants in game logic — the actual tick interval is adaptive (`TickPacer` in `src/world/mod.rs` may back off from 10 Hz to 2 Hz under sustained overload), and coupling combat pacing to it would mean combat slowed down whenever the server got busy.

### Visibility / API surface

- `pub` is the public surface; `pub(crate)` is for internal modules.
- The `OUTGOING_PACKETS` static is `pub(crate)`. External readers call `client::outgoing_packet_count()`.

## Performance posture

- Slab/HashMap capacities don't auto-shrink on remove. `World::shrink_periodic()` runs at every snapshot save; don't call it more often.
- AOI radius is **400 yards**. Larger than vanilla's 93 yd default by intent — leave it alone unless coordinating with the user.
- Creature ticks are bucketed by behavior: `walking_creature_keys` and `aggro_creature_keys` are pre-filtered Vecs maintained on `register_creature`/`unregister_creature`. `tick_walking_creatures` and `tick_creature_ai` iterate those, not the full slab.
- `creature_wake_at: BTreeMap<Instant, Vec<usize>>` parks idle creatures so the active walking iteration stays tiny.
- Promotions per tick are **unbounded**. There's no `MAX_PROMOTIONS_PER_TICK` cap; bursts of CMSG_PLAYER_LOGIN drain in one tick. The per-promotion `create_object` cache + the account-name reverse index handle the burst load.
- Tracy plots live in `World::tick` near the end. New plots go there.

## Environment

Set via shell env, `.cargo/config.toml` (cargo-launched only), or `.env` (loaded by `dotenvy` at startup):

| Var | Purpose |
|---|---|
| `WOW_REALM_ADDRESS` | `host:port` advertised in the realm list. Set to your public IP + `8085` when deploying to a server. Defaults to `localhost:8085`. |
| `WOW_AUTH_AUTO_CREATE` | If set (any value), unknown auth usernames are auto-created with `password = username`. Required for the loadtest binary to work. Off by default — production runs should leave it unset. |
| `WOW_VANILLA_WORLDDB` | Path to mangos SQLite. Without it the server boots with one test wolf only. Set in `.cargo/config.toml` for local dev. |
| `WOW_VANILLA_USE_MAPS` | Compile-time `option_env!`: path to vanilla client data for namigator builds. Enables terrain following. Without it, mobs walk in linear z. |
| `LOADTEST_NAME_PREFIX` | Optional override for the bot character-name prefix in the loadtest binary. Defaults to the worker host's hostname (via `gethostname` crate). |

A `.env.example` template is checked in; copy to `.env` and adjust.

## Load-test harness

`src/loadtest/` ships an orchestrator + worker for spawning real-protocol bots:

- **Worker** (`--role worker`) opens many SRP6/ARC4-encrypted sessions against the running server, auto-creates a character per session at Northshire, and runs a random-walk movement driver. Two tokio tasks per bot: a reader and a drive task.
- **Orchestrator** (`--role orchestrator`) is a TCP control plane on `:7100`. Workers register, receive `Spawn` / `Stop` / `Drain` commands, push `WorkerMetrics` every second.
- **Standalone**: `--clients N --ramp-up SECONDS` skips the orchestrator entirely.

Bots use deterministic per-username character profiles (`profile_for(username)` in `worker/world.rs`) so a given bot always logs into the same character. The character name format is `<HostPrefix><5RandomLowercase>` — the host prefix comes from the worker host, sourced via `gethostname` or the `LOADTEST_NAME_PREFIX` env override.

The server's `WOW_AUTH_AUTO_CREATE=1` env must be set for bots to authenticate. The server is otherwise unmodified — bots speak the exact protocol the 1.12.2 client speaks.

**Linux fd limit**: `ulimit -n` defaults to 1024 which caps the worker at ~510 bots during the auth-then-world transition. Raise to 65536 for any serious load test.

## Known scope-out items (don't fix unprompted)

- Snapshot versioning — explicitly punted ("this is development").
- Per-zone weather, world-DB game objects, full faction handling, respawn — listed but deferred.
- `Client` → `Slab<(PlayerSession, Player)>` proper split — `Client` is currently a composition shim. Don't unwind that without coordinating.
- Spatial index for AOI — linear scan over clients per broadcast is fine for the current clustered-bots workload. Revisit once spells with smaller AOIs land.

## Pull-request checklist

Before opening a PR:

- [ ] `cargo clippy --all-targets` clean (with and without `--features wowm-capture` if you touched those code paths).
- [ ] `cargo nextest run` clean.
- [ ] No new `pending_*: &mut Vec<…>` arguments. Use `CommandQueue`.
- [ ] No new `slab.iter().find_map(…)` guid lookups. Use the indexes.
- [ ] No new `as` casts at boundaries. Use `try_into` / `clamp`.
- [ ] No new `.unwrap()` on client-supplied data. Log + drop.
- [ ] No new `Vec::new()` / `HashMap::new()` allocations inside `World::tick` hot phases. Use the `scratch_*` fields.
- [ ] No new per-viewer `msg.clone()` broadcast loops. Use `aoi::broadcast_*_within_aoi`.
- [ ] No new socket writes from inside `World::tick`. Go through the per-client outbound channel.
- [ ] If you added a Slab or HashMap to `World`, add it to `World::shrink_periodic`.
