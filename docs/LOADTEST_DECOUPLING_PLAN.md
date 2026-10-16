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
- **The race can't work on the server's rustigator pin.** fc45283 used the east-west axis as "up" (fixed in 66dedc8),
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

## Phases

| # | Deliverable | Done when |
|---|---|---|
| 1. Extract | The new repository, history kept; own workspace and release profile; no server dependencies; runtime `--nav-data` / `--nav-cache`; wow_vanilla_server drops `src/loadtest` and its `[[bin]]`, with a README pointer | It builds and runs today's modes against wow_vanilla_server unchanged |
| 2. A correct client | The framer and selective decode; the state machine with timeouts and cancellation; protocol 3; ready (verify, active mover, cinematic); acks; logout; global slots; orchestrator fixes; `drain_all` concurrency | 1 bot logs in, becomes visible to a second bot and moves, on both servers; 500 bots `wander` on wow_vanilla_server with no desyncs; `stop` and `drain` finish within their deadline |
| 3. Profiles and placement | Profile schema and validation; the two profiles; placement strategies; combat model from update fields | 200 bots gather in the Gurubashi pit on both servers and fight to the death with correct death detection; plans the profile can't serve fail at startup |
| 4. Metrics and reports | Histograms, worker health, merged reports, run plans | One plan produces comparable reports for both servers; a deliberately saturated worker marks its run invalid |
| 5. Scenarios | `idle`, `login_churn`, `wander`, `cluster`, `pvp_arena`, and `race` on rustigator master | Every scenario runs on both servers, or fails at startup with a stated reason |
| 6. Baselines | The same plan matrix on both servers on the same hardware, results kept in the repository | Baseline tables, and the server findings that limit them, filed |

## Server-side prerequisites

**wow_vanilla_server.** Fix what would abort or corrupt a run before taking baselines; measure the performance issues
with the new tool, then fix them:
- **Before baselines:**
  - the `accept()` unwrap and the missing timeouts (review row 1);
  - double login (row 4);
  - the server's own unknown-opcode desync (row 9);
  - the fake crit on `CMSG_ATTACKSWING` (row 8).
- **To measure, then fix:**
  - maps ticking serially (row 6);
  - creatures walking while dead, and duplicates (row 7);
  - creatures sent twice at login;
  - idle maps doing full work.
- **The rustigator bump.** The server's own snap-to-ground and line of sight are wrong until it moves past fc45283.
  That needs a matching `wow_world_base` first (CODE_REVIEW, "Navmesh and the race").

**Thistle Tea:**
- **Maps baked on the server,** for area detection and so the Gurubashi FFA pit. The fork bakes and rustigator bakes
  already exist locally.
- **A ceiling of 1,000 bots** from the seeded accounts. Raise it in a local checkout (`lib/application.ex`, one
  range), or register over HTTP.
- **Its 5 s internal call timeout** drops connections under overload. Count it as a server result, not a tool error.
- **It sends 3-byte size headers,** which a real 1.12 client may not accept. Worth raising upstream.

## Decisions for you

1. **Where the loadtest lives:** its own repository (recommended: no shared pins or release profile), or a workspace
   crate in wow_vanilla_server. Also its name.
2. **Which servers to target:** design for cmangos and vmangos too, but validate only wow_vanilla_server and Thistle
   Tea.
3. **Thistle Tea above 1,000 bots:** patch its seeding locally, or stay at 1,000.
4. **wow_vanilla_server's rustigator bump:** re-pin rustigator's `wow_world_base` to e8dc643, or upgrade the server's
   wow_messages.
5. **Which server fixes come before the first baselines:** the four above, or more.
