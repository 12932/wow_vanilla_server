# Plan: a server-agnostic loadtest

**Goal.** A load-testing tool that can be pointed at any WoW 1.12 server (wow_vanilla_server, Thistle Tea, and in
principle cmangos or vmangos) and gives numbers that measure the server, can be compared between servers, and can be
reproduced.

Today's `src/loadtest` is the starting point. Its findings are in [CODE_REVIEW.md](CODE_REVIEW.md) and summed up
below.

## Where it stands

The loadtest is already its own binary crate, imports nothing from the server, sends no GM commands and no chat. Its
coupling is behavioural and in the build.

**It can't drive Thistle Tea at all:**
1. **Auth hangs.** It logs in with protocol 2, so its proof is 74 bytes; Thistle Tea reads 75 (protocol 3's security
   flag) and waits. The bot has no auth timeout, so it hangs. wow_vanilla_server accepts 2, 3 and 8, which hid
   this. Build 5875 is 1.12.1, protocol 3.
2. **Movement is ignored.** It never sends `CMSG_SET_ACTIVE_MOVER`. Thistle Tea drops all movement until it does, and
   the bot never becomes visible.
3. **PvP bots disconnect.** A parse error ends the bot. Thistle Tea sends combined `HitInfo` flags (`0x82`, `0x4002`),
   which wow_messages' enum can't parse, so PvP bots drop at the first crit or miss.

**Assumptions that only hold on wow_vanilla_server:**
- **Accounts.** Password = username, created on the fly. Thistle Tea pre-seeds `BOT0000`–`BOT0999` and creates
  nothing at login.
- **Spawn point.** Every bot starts at a hard-coded `ANCHOR` in the Gurubashi pit, which must equal the server's
  `config.toml [spawn]`. Thistle Tea spawns by race, on map 0 or map 1.
- **Teleporting.** It teleports by heartbeat, for the race start and the random-walk bounce. That needs no
  anti-cheat, and the same map.
- **PvP model.** FFA anywhere, death when the summed damage reaches a mirrored `pvp_max_health = 100`, no respawn,
  swing re-sent every 2 s.
  - wow_vanilla_server's fake 1,337 crit on every `CMSG_ATTACKSWING` most likely counts toward a bot's damage sum,
    so death detection is probably already wrong there. Check this when the death model is replaced.
  - On Thistle Tea, PvP is FFA only in area 2177 (the pit), needs its maps for area detection, and uses real health
    and deaths.
- **No acknowledgements.** It never acks teleports, worldports, speed changes or roots, never logs out, and doesn't
  wait for `SMSG_LOGIN_VERIFY_WORLD`.
- **Build.** It lives in the server's package, so it inherits `panic = "abort"`, every server dependency, the
  wow_messages and rustigator pins, the compile-time `WOW_VANILLA_USE_MAPS`, and the server's
  `/tmp/wow_vanilla_server` bake.

**Bugs that hide behind the server it was written for:**
- **Skipping an unknown opcode desyncs the stream.** wow_messages has already read the body, and the bot reads it
  again.
- **The metrics don't measure the server:**
  - bytes in and out are never incremented;
  - there's no latency anywhere (`CMSG_PING` sends 0, and the pong is ignored);
  - "alive" and "world ok" are counted when `CMSG_PLAYER_LOGIN` is written;
  - rates assume one-second gaps.
- **Usernames collide across workers.** Every worker counts slots from 0, with the same default prefix.
- **The orchestrator's registry gets corrupted by a worker reconnecting.** Slab key reuse plus removal by key.
- **`drain_all` times out one bot at a time**, and `stop` never aborts.
- **The race can't work on the server's rustigator pin.** fc45283 used the east-west axis as "up" (fixed in 75e2af1),
  so path queries fail, and the race silently falls back to a 3-point line with no ground following.
  - Its other workarounds predate rustigator.
  - The real fixes: start on the dock rather than under it, and sample heights where the jittered bots are.
- **Load is less realistic than it looks:**
  - heartbeats every 250 ms (the client sends about 500);
  - synchronized bot phases when ramp-up is 0;
  - 20 wakeups a second per bot;
  - every packet decoded in full.

## Design

### Principles

1. **Behave like a real 1.12 client by default.** Anything server-specific lives in a profile, and anything
   unrealistic is an explicit stress option.
2. **Measure the server, not the tool.** The worker watches its own lag and CPU, and a saturated run is marked
   invalid.
3. **Reproducible.** A run is a plan, a profile and a seed; the report records them, with both sides' versions.
4. **Fail loudly.** No silent fallbacks: a missing navmesh, an account ceiling or an unsupported scenario is an error
   at startup.

### Shape

The loadtest gets its own repository: the history of `src/loadtest` is kept with `git filter-repo`. It becomes a
workspace:

| Crate | Holds |
|---|---|
| `proto` | The login client (protocol 3, timeouts). The world framer: decrypts headers itself, accepts Thistle Tea's 3-byte size header above 0x7FFF, sanity-checks sizes, decodes only the opcodes a bot needs and skips the rest raw, counting skips and parse failures |
| `bot` | The session state machine, the bot's view of the world, the acknowledgements, logout |
| `profile` | Server profiles (TOML) and the capability check |
| `scenario` | The `Scenario` trait and the scenarios |
| `nav` | Optional (feature `nav`): rustigator at master, runtime data and cache paths, a `bake` subcommand |
| `metrics` | HDR histograms, counters, worker health, merging, reports |
| `loadtest` | One binary: `worker`, `orchestrator`, `run <plan>`, `bake` |

Its own release profile unwinds on panic. Its wow_messages pin can follow rustigator master's (70abb9de), whatever
the server pins.

### The bot

The bot is a state machine with a deadline on every step, and shutdown can cancel any step:

`Connect → LogonChallenge → LogonProof → RealmList → WorldConnect → AuthSession (wait queue allowed) → CharEnum →
[CharCreate] → PlayerLogin → AwaitVerifyWorld → Ready → Placement → Scenario → Logout`.

- **Ready.**
  - Read the map and position from `SMSG_LOGIN_VERIFY_WORLD`.
  - Send `CMSG_SET_ACTIVE_MOVER{own guid}`, as the client does.
  - On `SMSG_TRIGGER_CINEMATIC`, send `CMSG_COMPLETE_CINEMATIC`.
  - Count world ok and alive only now, and record the time to ready.
- **Always handled, on any server:**
  - `MSG_MOVE_TELEPORT_ACK`: adopt the position and ack it.
  - `SMSG_TRANSFER_PENDING` / `SMSG_NEW_WORLD`: adopt the map and position, then `MSG_MOVE_WORLDPORT_ACK`.
  - `SMSG_FORCE_*_SPEED_CHANGE`: adopt the speed and ack.
  - Root and unroot, with acks.
  - `SMSG_PONG`, with round-trip time measured by sequence.
  - The bot's own health from `SMSG_UPDATE_OBJECT` values.
- **Movement:**
  - heartbeat every 500 ms by default, with a random phase offset per bot;
  - `MSG_MOVE_SET_FACING` for turns, and orientation normalized;
  - the timestamp is a client tick;
  - each bot sleeps until its next due action, rather than ticking at 20 Hz.
- **Exit.** `CMSG_LOGOUT_REQUEST`, then wait briefly for `SMSG_LOGOUT_COMPLETE`. Every exit records its reason.
- **Optional client-realism traffic (on by default):**
  - `CMSG_NAME_QUERY` and `CMSG_CREATURE_QUERY` for newly seen guids;
  - `CMSG_SET_SELECTION` before attacking.

  Real servers pay for these. wow_vanilla_server's name query scans every character, so a test without them misses a
  real hot spot.

### Server profiles

One TOML per server, validated at startup against the run plan. Each field is the setting for both servers:

| Field | wow_vanilla_server | Thistle Tea |
|---|---|---|
| `auth` | Protocol 3, connect and step timeouts | The same |
| `accounts` | `implicit` (password = name) | `preseeded`, prefix `BOT`, 0–999, password = name |
| | Global slots: the orchestrator assigns each worker a disjoint range; freed slots are reused; the plan fails if it needs more accounts than the profile has | The same |
| `realm` | First, or by name; `world_addr` override | The same |
| `character` | All 40 race/class pairs; names from the global slot, retried on `NameInUse` | The same; restrict to Eastern Kingdoms races (Human, Dwarf, Gnome, Undead) when a scenario needs map 0 and placement can't cross maps |
| `login` | Wait for verify, active mover, cinematic | The same, and setup commands: `.tgm`, `.character level 60` |
| `placement` | `server_spawns` (checks the server's spawn matches the scenario's target) | `chat_command`: `.go xyz {x} {y} {z} {map}`, waiting for the teleport or worldport ack |
| | Other strategies: `as_spawned`, `heartbeat_teleport` (same map only) | |
| `combat` | Death from update fields (or `damage_sum{max_hp}`); respawn `none`; swing `once_per_target`; PvP `ffa` | Death from update fields; respawn `.revive`; PvP `ffa_area`, area 2177 |
| `decode` | Default | Lenient `HitInfo`; 3-byte header supported |
| `keepalive` | Ping every 30 s | The same; Thistle Tea drops a connection after 60 s idle |

### Scenarios

Each scenario states what it needs from the server; a profile that can't provide it fails the plan at startup.

| Scenario | Exercises | Needs |
|---|---|---|
| `idle` | Logins, keepalive, idle cost per player | Nothing |
| `login_churn` | Repeated login and logout: auth, char screen, persistence | Logout |
| `wander` | Today's random walk without the teleport bounce: reflects at the radius | Placement |
| `cluster` | N bots moving within a small radius: AOI fan-out, roughly N² | Placement |
| `pvp_arena` | Today's PvP, with the battle started by an orchestrator command once every bot is ready | An FFA area or faction split, a death model |
| `race` | Booty Bay → Gurubashi on the navmesh | Map 0 placement, feature `nav` |

**The race, done properly:**
- `PathOptions::reference(false)`, so steep ground isn't taken as flat.
- A start per bot from a random navmesh point near the dock, with its own path, instead of ±50 yd jitter, which puts
  bots in the bay and in cliffs.
- Heights looked up where the bot actually is, picking the one closest to its previous height (keep this).
- The tile preload, with a margin.
- A shared read-only `PathfindMap`, queried through `&self` (`find_heights_into`, `find_path_with`), with no mutex.
- Remove the dock special case and the diagnostics. Stormwind can be the end again if wanted.

### Metrics

| Metric | How |
|---|---|
| Login funnel | Histograms of connect → auth ok → world ok → verify → first other player visible, and failure counts by step and reason |
| Responsiveness | Ping round-trip histogram |
| Movement propagation | Bot A records (guid, movement timestamp) → send time; bot B in the same worker records the time it receives A's `MSG_MOVE_*` carrying that timestamp. Both servers forward the client's `MovementInfo`. This measures the tick plus AOI fan-out. Same-worker only, since clocks differ between hosts |
| Combat | Swing → `SMSG_ATTACKERSTATEUPDATE` latency |
| Volume | Bytes and packets in and out at the socket, by opcode; parse skips; desyncs |
| Worker health | Driver lateness, tokio busy ratio, process CPU. A run above the thresholds is marked invalid |

Workers send interval histograms every 5 s, and the orchestrator merges them. Rates come from elapsed time, and
workers that go stale are expired.

The report is JSON plus a markdown summary per stage. It records the plan, the profile, the seed, the loadtest's
commit, and the server's build if the profile names it.

### Orchestrator

- **Registry.** Fix it with connection generations.
- **`Hello`** carries a protocol version and a shared token. The orchestrator binds 127.0.0.1 by default.
- **Slots.** The orchestrator assigns slot ranges in its reply to `Hello`.
- **Broadcasts** go through a task per worker, with write timeouts, so one stalled worker can't block the rest.
- **Run plans** for reproducible runs: `[[stage]] scenario="cluster" bots=500 ramp="60s" hold="300s"`, run in order.
  The REPL stays for ad hoc use.

### Running on ECS and Fargate

The worker ships as a minimal container image, so a fleet of small Fargate tasks can each run about 20 bots and
reach thousands together. That's closer to real players than a few big hosts, since each task has its own network
interface.

- **The image.** A multi-stage build: the Rust builder compiles `loadtest` in release, and copies it into
  `debian:trixie-slim`, which adds glibc and nothing else. It runs as a non-root user, with
  `ENTRYPOINT ["loadtest", "worker"]`.
  - The `nav` feature is left out: race bakes are gigabytes. A race fleet mounts them from EFS instead, with
    `--nav-cache`.
  - The image is tagged with the loadtest's commit, which the reports record.
- **Configuration by environment.** Every flag also reads an environment variable (`LOADTEST_TARGET`,
  `LOADTEST_CLIENTS`, `LOADTEST_ORCHESTRATOR`, `LOADTEST_PROFILE`, ...), so a task definition needs no command line.
- **Accounts across tasks.** Tasks are identical, so none can pick its own slot range. Each one connects to the
  orchestrator, which hands out disjoint ranges in its reply to `Hello`, and takes a range back when a task goes away.
- **The orchestrator runs a plan, not a REPL.** ECS has no stdin. It runs a TOML run plan (stages, bot counts, ramps,
  holds), and waits until the expected number of workers have joined.
  - It runs as its own task or service, reachable only inside the VPC.
  - Workers authenticate with the shared token in `Hello`.
- **Stopping.** ECS sends SIGTERM, then SIGKILL 30 s later. The worker treats SIGTERM like `drain`: it logs its bots
  out, sends its final metrics, and exits within the grace period.
- **Logs and metrics.** Logs go to stdout for the `awslogs` driver, in JSON with `--log-format json`. Metrics go to
  the orchestrator as usual; the report is written where the orchestrator runs (an EFS path or S3 later).
- **Not in this repository:** the AWS resources themselves (cluster, task definitions, VPC, security groups). A
  sample task definition and the run steps go in its README.

## Phases

The order puts the cheap fixes that decide whether any number means anything first, and the big redesign last.
Work happens on the `work/loadtest` branch of wow_vanilla_server, committed and pushed per phase; nothing is merged
until it is asked for.

### Phase 1: a correct client, in place

In `src/loadtest`, without moving it.

- [x] **Auth:** protocol 3 messages, version 1.12.1. Timeouts on connect, every read and every write. The handshake
  can be cancelled by `stop` and `drain`.
- [x] **Reader:** the unknown-opcode skip no longer reads the body twice. A parse error is skipped and counted; the
  body is already consumed.
- [x] **Login:**
  - Wait for `SMSG_LOGIN_VERIFY_WORLD`, skipping other packets until a deadline, and adopt its map and position.
  - Send `CMSG_SET_ACTIVE_MOVER`.
  - Answer `SMSG_TRIGGER_CINEMATIC` with `CMSG_COMPLETE_CINEMATIC`.
  - Count world ok and alive only at verify.
- [x] **Accounts:**
  - A global slot range for each worker: `--slot-offset`, and a default prefix from `--worker-id`.
  - `--password` (default: the username).
  - Freed slots are reused.
- [x] **Random mode** walks around the server's spawn, not the hard-coded anchor, and reflects at its radius instead
  of teleporting back. PvP and race still assume wow_vanilla_server's Gurubashi spawn, until Phase 5.
- [x] **Shutdown:**
  - `drain` notifies every bot at once, with one deadline, then aborts what's left.
  - `stop` aborts.
  - Dead bots leave the slab.
- [x] **Orchestrator:** registry entries carry a connection generation, so a reconnecting worker isn't evicted.

**Done when:**
- 50 bots in random mode reach the world on wow_vanilla_server, with no desyncs.
- 10 bots log in on Thistle Tea, and their movement is seen by another bot.
- `stop` and `drain` finish within their deadline.

**Result (debug builds, one host):**
- **wow_vanilla_server:** 50 of 50 bots in the world, 0 parse errors. With 20 bots, every packet received was another
  bot's movement (1,140 a second, which is 62 sent times 19 observers).
- **Thistle Tea** (0426f0d6, the fork's NIF): 10 of 10 bots in the world, receiving others' movement. Only bots that
  share a start zone meet, since Thistle Tea spawns by race; Phase 5's placement gathers them.
  - Its `SMSG_COMPRESSED_UPDATE_OBJECT` doesn't parse in wow_messages ("Missing object TYPE"). It is skipped and
    counted, and bots stay connected.
- **Shutdown:** `drain` took 53 ms for 50 bots.
- **Orchestrator:** `spawn 5`, `stop 2`, `spawn 2` ended at 5 alive, reusing `BOT0000` and `BOT0001`.

### Phase 2: server fixes that would abort or corrupt a run

In wow_vanilla_server itself (CODE_REVIEW rows 1, 4, 8 and 9).

- [x] **Listeners:** accept errors are logged with a back-off, not unwrapped. Pre-auth reads time out, the world
  reader has an idle timeout, and connections are capped.
- [x] **Double login:** a character already in the world is refused, and index removal only deletes entries that
  still point at the client being removed.
- [x] **Unknown opcodes:** the world reader no longer reads their bodies twice.
- [x] **`CMSG_ATTACKSWING`:** no fake 1,337 crit, and attack start broadcast only when the state or target changes.
- [x] **Clippy:** `cargo clippy --all-targets` clean again (the `hot_loops` bench, and one warning).

**Done when:** each fix has a test, and Phase 1's runs pass again.

**Result:**
- **Tests:** `cargo clippy --all-targets` is clean, and 136 of 136 tests pass. Each fix has a test:
  - a quiet auth client times out;
  - an unknown opcode is skipped and a ping after it is still answered;
  - a burst of bad packets disconnects;
  - a kicked session is dropped and saved;
  - repeating an attack swing changes nothing.
- **Live:** 50 bots in the world with no parse errors. A second worker logging the same five accounts in kicked
  the first worker's five sessions (45 and 5 alive, and five "logged in again" drops). An idle world socket is closed
  after the 10 s handshake timeout.
- **Double login:** the newest login wins, as on most servers. The old session is dropped as stale (saved, and
  unregistered) by its map's next tick, and the new one then enters.
- **New `[network]` settings:** `handshake_timeout_secs` (10), `client_idle_timeout_secs` (120),
  `max_pending_connections` (1024).
- **Side effect:** with the swing timer no longer reset by every `CMSG_ATTACKSWING`, repeated swings land at their
  real rate.

### Phase 3: rustigator master in wow_vanilla_server

- [x] rustigator pins `wow_world_base` back to the server's wow_messages commit (e8dc643).
- [x] wow_vanilla_server moves to rustigator master. Its bakes are format 12, and rebuild themselves.
- [x] **Race:**
  - Use `PathOptions::reference(false)`.
  - Fail loudly when maps are configured but no path is found.
  - Remove the dock special case and the diagnostics.
  - Small jitter, or a path per bot.

**Done when:** the race walks Booty Bay → Gurubashi on the navmesh, staying on the ground; the server's snap to
ground and line of sight work in Booty Bay and the arena.

**Result:**
- **Pin.** wow_vanilla_server pins rustigator 6a4d553, on rustigator's `work/wow-messages-pin` branch: master
  (5e9d92c) with `wow_world_base` back at e8dc643. rustigator passes 124 of 124 there. Merging that branch into
  rustigator's master is still to do.
- **Bake.** The server baked format 12 on startup: Eastern Kingdoms in 61 s, Kalimdor in 88 s.
- **Race.** 19 waypoints, 1,001 yd, with 3 ADT tiles loaded. 20 of 20 bots raced, and all 3,502 heartbeats took
  their height from the ground. The mean change was 0.30 yd a heartbeat (about 1.75 yd of travel), the largest
  2.45 yd.
- **The server's bake:**
  - heights: 24.75 on the dock; 21.857 in the pit, the configured spawn; 24.36 and 21.00 at the entrance;
  - line of sight across the pit: clear at chest height and at foot height (at fc45283, foot level reported
    "obstructed").
- **Race changes:**
  - `PathOptions::reference(false)`, so steep ground costs four times as much rather than counting as flat;
  - the map shared without a mutex, through `&self` height lookups;
  - jitter of ±5 yd, down from ±50;
  - an error, not a silent fallback, when maps are configured and the path fails;
  - the dock special case, the diagnostics and the stale comments removed.

### Phase 4: extract

- [ ] The loadtest moves to its own repository, history kept with `git filter-repo`. It gets its own workspace and
  release profile (unwinding), and no server dependencies.
- [ ] Runtime `--nav-data` and `--nav-cache` replace the compile-time variable and the server's bake directory.
- [ ] **The container image:**
  - a `Dockerfile`, multi-stage onto `debian:trixie-slim`, non-root, without `nav`;
  - every flag also readable from a `LOADTEST_*` environment variable;
  - SIGTERM drains like `drain`;
  - `--log-format json`.
- [ ] wow_vanilla_server drops `src/loadtest` and its `[[bin]]`, and its README points to the new repository.

**Done when:**
- The new repository builds and runs Phase 1's checks.
- The image is under 100 MB.
- `docker run` with only environment variables runs 20 bots against wow_vanilla_server.
- `docker stop` drains them within its grace period.

**Needs from you:** the new GitHub repository.

### Phase 5: profiles, placement and combat

- [ ] The profile schema and its startup validation, with profiles for wow_vanilla_server and Thistle Tea.
- [ ] Placement strategies: server spawns, as spawned, heartbeat teleport, chat command.
- [ ] Teleport, worldport, speed and root acks.
- [ ] Logout.
- [ ] Health from update fields; respawn policies; PvP modes (FFA, FFA area, faction split); swinging once per
  target. `SMSG_ATTACKERSTATEUPDATE` is read leniently, so Thistle Tea's combined `HitInfo` flags parse.
- [ ] A lenient reader for (compressed) update objects. wow_messages rejects Thistle Tea's with "Missing object
  TYPE", and health comes from them.

**Done when:**
- 200 bots gather in the Gurubashi pit on both servers and fight, with correct death detection.
- Plans a profile can't serve fail at startup.

### Phase 6: metrics, reports, run plans

- [ ] Latency histograms: login funnel, ping round trip, movement propagation, swing to damage.
- [ ] Bytes and packets at the socket.
- [ ] Worker health, so a saturated run is marked invalid.
- [ ] Orchestrator-assigned slot ranges, handed out in reply to `Hello` and taken back when a worker goes away.
- [ ] Merged reports, and run plans in TOML. The orchestrator runs a plan without stdin, for ECS, once enough
  workers have joined.
- [ ] A versioned `Hello` with a shared token. The orchestrator binds 127.0.0.1 unless told otherwise.
- [ ] A sample ECS task definition and the run steps for a Fargate fleet, of about 20 bots a task, in the README.

**Done when:**
- One plan produces comparable reports for both servers.
- A deliberately saturated worker marks its run invalid.
- Ten containers of 20 bots each, under one orchestrator, run a plan with no account collisions.

### Phase 7: scenarios and baselines

- [ ] `idle`, `login_churn`, `wander`, `cluster`, `pvp_arena`, `race`.
- [ ] Client-realism traffic: name and creature queries, selection.
- [ ] The same plan matrix on both servers on the same hardware, results kept in the repository, and the server
  findings that limit them filed.

**Done when:** baseline tables exist for both servers.

## Thistle Tea prerequisites

- **Maps baked on the server,** for area detection and so the Gurubashi FFA pit. Fork and rustigator bakes already
  exist locally.
- **A ceiling of 1,000 bots** from its seeded accounts.
- **Its 5 s internal call timeout** drops connections under overload. Count that as a server result, not a tool error.
- **It sends 3-byte size headers above 0x7FFF bytes,** which a real 1.12 client may not accept. Worth raising upstream.

## Decisions

Taken as defaults, to revisit at any phase:
1. **The loadtest gets its own repository** in Phase 4, once you have created it on GitHub. Its name is yours to
   choose.
2. **Servers:** designed for cmangos and vmangos too, but validated only on wow_vanilla_server and Thistle Tea.
3. **Thistle Tea stays at 1,000 bots,** its seeded accounts. Raising it is a one-line change in a local checkout.
4. **The rustigator bump** is unblocked from rustigator's side, by re-pinning its `wow_world_base`.
5. **Before the first baselines,** the four server fixes of Phase 2.
