# Code review: wow_vanilla_server and its loadtest (at 79b6def)

Six read-only reviews (networking and auth; world simulation; persistence, config and GM commands; the loadtest;
the race's navmesh workarounds; Thistle Tea compatibility), merged and deduplicated here. File and line references
are at 79b6def, relative to `src/`. The loadtest's own findings, and the plan for it, are in
[peon](https://github.com/12932/peon)'s [docs/PLAN.md](https://github.com/12932/peon/blob/master/docs/PLAN.md): the loadtest became peon.

**Baseline.** `cargo nextest run`: 129 of 129 pass. `cargo clippy --all-targets` fails:
- the `hot_loops` bench doesn't compile: it uses `World.cells` and `cell::CellKey`, both gone since d9a0131;
- one warning, an empty line after a doc comment.

AGENTS.md requires clippy to be clean.

One build setting raises the severity of every panic below: `[profile.release] panic = "abort"`. A panic anywhere,
in any task, kills the whole server with no final save.

## The ten to fix first

| # | Finding | Where | Effect |
|---|---|---|---|
| 1 | `listener.accept().await.unwrap()`, and no read timeouts or connection caps anywhere | `world/mod.rs:39-50`; auth reads in `auth.rs`; `character_screen_client.rs:182` | About 1k idle sockets, or fd exhaustion (documented at about 510 clients under the default ulimit), aborts the server. The auth accept loop busy-spins on EMFILE |
| 2 | Every session is a GM: any chat starting with `.` runs a command; `CMSG_WORLD_TELEPORT` is open; accounts are created for any name with password = name; listeners bind `0.0.0.0` | `opcode_handler.rs:237-256, 299-311`; `auth.rs:394-400`, `:173` | Anyone who can reach the port can teleport, spawn without limit, `.boom` and `.swifty`, and crash the server (next row) |
| 3 | GM commands crash the server: `.move` indexes `creatures()[0]` on a slab; `.mark` opens a file without `create(true)` and unwraps | `gm_command/mod.rs:160`; `file_utils.rs:5-9` | Two chat lines, `.go 0 0 0 30` then `.move`, abort the process |
| 4 | The same character can be in the world twice | `character_screen_handler/mod.rs:55-78`; `world/world/mod.rs:392-422, 2014-2048` | A reconnect over a half-open session overwrites `client_by_guid` and `PLAYER_REGISTRY`. Reaping the old copy then deletes the live player's index entries and saves the stale copy over it |
| 5 | A snapshot that fails to decode becomes an empty world at the next save; saves don't fsync; SIGTERM skips the final save | `world/mod.rs:190-193, 223-251`; `snapshot.rs:97-103` | A truncated file (crash during save), or any change to a snapshot struct, silently replaces every character |
| 6 | All maps tick one after another: each map task holds the global `db` and `maps` locks for its whole tick | `world/world/mod.rs:2315-2318`, through 2852 | Tick time is the sum over maps, not the max. The per-map design gains nothing, and load test numbers reflect this |
| 7 | Dead creatures keep walking, and each respawn can add a duplicate walking entry | `world/world/mod.rs:571-615, 1054-1071, 1250-1262` | Corpses move; duplicates move at double speed and broadcast twice, and farming makes it worse |
| 8 | Inbound packets aren't rate-limited, and several opcodes broadcast to the whole AOI on every packet | `world_opcode_handler/mod.rs:37-48`; `opcode_handler.rs:521-658`; `spell.rs:28` | One client can fill every observer's 1 MiB outbound budget, and dropped frames then desync those observers. `CMSG_ATTACKSWING` re-broadcasts a fake 1,337-damage crit each time; Frost Nova has no cooldown |
| 9 | An unknown client opcode desyncs the session: wow_messages has already read the body, and the reader reads `size` more | `character_screen_client.rs:187-195` | One unmodelled opcode turns the rest of the session into garbage. The loadtest has the same bug |
| 10 | Client positions are trusted: no finite, bounds or speed check | `opcode_handler.rs:104-106` and every `MSG_MOVE_*` arm | NaN makes `dist_sq > r²` false, so melee lands from anywhere while the attacker is invisible. Extreme coordinates saturate `cell_coord`, then `cx + dx` overflows. Spoofed positions force-load any ADT in the tick |

## Security and robustness

- **Character creation is unbounded.** No cap per account, no name rules, no uniqueness (`character_screen_handler/
  mod.rs:36-51`, `char_create.rs:12-73`, `database/mod.rs:75-85`).
  - Each character carries a 113-slot inventory, about 3.6 KB, and the save serializes them all.
  - Past 255 characters `SMSG_CHAR_ENUM`'s count wraps, and at about 64 KB its u16 size wraps.
  - Duplicate names break `PLAYER_NAME_INDEX`.
  - Fix: a limit of 10, 2–12 letters normalized, a name index, and a rate limit.
- **The teleport state machine** (`world/world/mod.rs:3491-3496`, `opcode_handler.rs:257-298`):
  - movement packets are applied mid-teleport;
  - `MSG_MOVE_TELEPORT_ACK` is unhandled, so the flag sticks;
  - the worldport arrival scans every creature on the map, and never seeds `visible_entities`.
- **Slow or half-open clients are never dropped** (`character_screen_client.rs:105`, `client/mod.rs:194-219`,
  `world/world/mod.rs:2690-2694`).
  - Budget exhaustion only counts, and the writer finishing isn't treated as stale.
  - There's no `set_nodelay` and no idle timeout.
  - Dropped creates or out-of-ranges leave ghost or invisible entities.
- **Dead players are stuck.** Logout is dropped while dead, there's no respawn, and the dead state persists
  (`opcode_handler.rs:84-93`).
- **The SRP cache grows without bound** under free logins (`auth.rs:88-115`).
- **Oversized packets wrap their u16 size header.** `OutOfRangeObjects` and `SMSG_CHAR_ENUM` aren't chunked
  (`aoi.rs:188`, `world/world/mod.rs:656-663, 730-737`).
- **Failure paths send no reply:** char delete, login failure, handshake failure, bad proof.
- **Packets after logout are still processed** (`world_opcode_handler/mod.rs:37-48`).
- **Account names aren't normalized** (`auth.rs:385, 448, 501`), so case variants are separate accounts.
- **GM input isn't validated.** `nan` and `inf` pass `parse_float` and get saved (`gm_command/parser.rs:260-265`).
  `.swifty` sends N² yells.

## World simulation

- **The killing blow isn't shown to the killer.** `kill_creature` broadcasts to `self.clients` while the attacker is
  taken out of the slab. No `SMSG_ATTACKSTOP` is sent when the target dies or vanishes (`world/world/mod.rs:2523-2527,
  566`).
- **Login and teleport send every in-range creature twice.**
  - Promotion doesn't add the creatures it sent to `visible_entities` (`world/world/mod.rs:2114-2149`).
  - Worldport ack updates neither side's set.
  - The result is about double the login payload under burst logins.
- **Every map does work proportional to its creature count every tick, even with no players.** The AOI snapshot
  copies every creature, and the root and corpse scans walk the whole slab (`world/world/mod.rs:1605-1648, 766-782,
  868-890`). Skip these when the map has no clients.
- **ADT loads block the tick.** `load_adt` does disk I/O and builds the mesh under the cell, db and maps locks.
  `line_of_sight` retries missing tiles on every call (`pathfinding_maps.rs:144-153, 200`).
- **Rayon inside tokio.** There is a fork-join per movement broadcast, and both pools are sized to every core, so the
  machine is oversubscribed (`aoi.rs:299-314`).
- **The heartbeat throttle counts map ticks, not time.** When backed off it throttles far more than intended, and it
  can drop a START/STOP coalesced under a heartbeat (`world/world/mod.rs:2309-2313, 705-720`).
- **Roots:**
  - creature roots resume walking for corpses and parked creatures, with no walk flag;
  - players are never unrooted.
- **Death and respawn are inconsistent:**
  - corpses keep the forward flag and a standing pose;
  - respawn doesn't reset behaviour;
  - melee resolves against corpses.
- **AggroChase teleports to the spline end.** It broadcasts per viewer, chases dead players, and scans clients
  linearly (`world/world/mod.rs:968-1028`).
- **`PLAYER_REGISTRY` positions go stale,** so `.go <name>` uses the login spot.
- **Banned patterns** (AGENTS.md) on hot paths:
  - per-viewer `send_message(msg.clone())` loops;
  - `CMSG_NAME_QUERY` scanning every character (`opcode_handler.rs:173-179`);
  - a linear PvP target lookup;
  - per-tick allocations despite the scratch buffers;
  - info-level `#[instrument]` spans on broadcast helpers.
- **Pacing:**
  - walking `dt` is clamped at 0.5 s while the pacer can back off to 1 s;
  - the slack makes a backed-off map tick at about 75% of its interval;
  - the slow-tick warning reports the first map in hash order, not the slowest.
- **Maintainability.** `world/world/mod.rs` is 3,955 lines, with a 640-line closure at 2211-2852. Split it into
  `map_state`, `cell_tick`, `combat`, `creature_ai`, `creature_life`, `aoi_tick`, `movement`, `login` and
  `player_object`. Do `cell_tick` and `combat` first: they are pure moves, and they make rows 6 and the killing-blow
  fix simpler.

## Persistence, config and build

- **postcard ignores `#[serde(default)]`,** and the test that claims to guard old payloads doesn't
  (`snapshot.rs:31, 77-80, 244-249`).
- **Saved enums (`Map`, `Area`, `RaceClass`) are stored as upstream variant indexes.** A `wow_messages` bump can
  silently remap every character. Store game ids.
- **A config parse error falls back to all defaults.** Nothing is validated:
  - `wander_idle_min_ms >= max` panics on `% 0`;
  - `target_interval_ms = 0` busy-loops;
  - a huge `save_interval_secs` panics at startup (`config.rs:47-70`; `world/world/mod.rs:1206-1208`).
- **Builds:**
  - `-Ctarget-cpu=native` can SIGILL on another host;
  - `option_env!("WOW_VANILLA_USE_MAPS")` bakes the build host's path into the binary, and panics if it's missing;
  - every test then builds navmeshes;
  - the bake cache is `/tmp/wow_vanilla_server`.
- **worlddb loader:**
  - `IN (?,…)` lists hit SQLite's parameter limit;
  - non-`Option` getters fail the whole load on one NULL;
  - `pathId` is ignored;
  - level and HP come from different columns.
- **Test gaps:**
  - no non-empty snapshot round trip;
  - no `restore_db_only`;
  - no worlddb fixture;
  - no GM parser arity tests;
  - no check that `config.toml` parses.
- **Docs drift.** AGENTS.md and the README describe:
  - 30 Hz (`config.toml` sets 10 Hz);
  - a 400 yd AOI (it's 200);
  - unbounded promotions (capped at 20);
  - simulated players, `unregister_creature` and `scratch_client_aabb`, none of which exist;
  - namigator, where rustigator is used now.
- **Cargo:**
  - `build.rs` links `user32` for a namigator that's gone;
  - `print-testcase` and `walkdir` are always on;
  - there's no `rust-version`, though let-chains need 1.88;
  - `test_support` ships in production.

## Navmesh and the race

- **The rustigator pin (fc45283) can't answer navmesh queries.** It built the navmesh with the east-west axis as "up"
  (fixed in rustigator 66dedc8, after the pin; this is the "off by 90 degrees" bug).
  - So the loadtest's `build_race_path()` fails, and the worker silently falls back to a 3-point polyline with no
    ground following (`loadtest/worker/mod.rs:83-89`).
  - At the pin, WMO collision also kept decorative triangles and lost a third of the arena's real ones.
  - Terrain also blocked line of sight.
- **The race workarounds predate rustigator.** They were written for namigator-rs (05-18 to 05-20).
  - Most were really loadtest bugs: a bad start point under the dock, and Z sampled without the bots' ±50 yd
    jitter.
  - On rustigator master, Booty Bay → Gurubashi routes (1,000 yd, steps under 2 yd), and Booty Bay → Stormwind routes
    too (5,410 yd).
- **Bumping to rustigator master is blocked at compile time.** rustigator e9cb2d6 moved `wow_world_base` to
  wow_messages 70abb9de; the server pins e8dc643, so `Map` and `Vector3d` become different types.
  - Fix in rustigator: pin `wow_world_base` back to e8dc643 (it built against it before e9cb2d6).
  - Or upgrade the server across 247 wow_messages commits, which rewrite much of `wow_world_base`.
  - After that the bump is a one-line rev change; the server's stale bakes are detected and rebuilt.
- **After the bump:**
  - **Remove:** the Booty Bay seabed special case and the race diagnostics.
  - **Change:** fail loudly when maps are configured but the path fails; use `PathOptions::reference(false)`; shrink
    the ±50 yd jitter, or give each bot its own path.
  - **Keep:** closest-to-hint height picking, the tile preload, the endpoint preload before line of sight, the
    collision-height raise for line of sight (fix its comment), and `snap_to_ground` on START/STOP.

## Strengths

- **SRP and ownership.** SRP is correct through wow_srp, the world proof can't be replayed, and character access is
  scoped to the owning account.
- **Outbound.** Broadcasts are serialized once into `Arc<[u8]>` with a per-client byte budget, and the tick never
  awaits a socket. Update objects are chunked under the u16 limit.
- **Locks.** No std mutex is held across `.await`, lock order is consistent, and no deadlock path was found.
- **AOI and indexes.** The AOI diff gives each rayon worker its observer's state, suppresses edge flapping, and caches
  CreateObject2s. The spatial index is maintained incrementally, with tests.
- **Persistence and config.** Saves write a temp file and rename it into place. Config uses `deny_unknown_fields`
  and `OnceLock`. Git dependencies are pinned by rev, and `Cargo.lock` is committed.
