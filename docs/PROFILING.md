# Profiling

[Tracy](https://github.com/wolfpld/tracy), a frame profiler with a GUI, to see where a map's 50 ms go, update by
update. It is in every build and costs nothing until it is asked for. Without a GUI, Tracy's own command-line tools
record and summarize a capture (below).

## The stats file (`WVS_STATS_FILE`)

For a load test, without Tracy's tools: `WVS_STATS_FILE=stats.jsonl` has the server write a JSON line a second
(`src/game/stats.rs`). Each line has the realm's players online, its packets in and out and bytes out, the heartbeats
coalesced and the packets held back in their sessions (`[network] max_queued_packets`), the clients disconnected for
their byte budget, for flooding and for stalling, the process's CPU seconds and RSS (counters since
start-up: take differences), and for each map its players, creatures awake, events waiting, every update's time that
second (`updates_us`), the events it handled and their time, its biggest and total outbound queue, and the most
objects an update left for the next to work out the visibility of (`visibility_waiting`: over `[visibility]
refresh_budget_ms`, [PERFORMANCE.md](PERFORMANCE.md)). Off, it costs
a relaxed load an update and an event burst. peon's `scripts/drill.sh` reads it.

## Tracy

### Running it

1. **The profiler.** Download Tracy **v0.14.1**, the release whose protocol the server speaks (the server's
   `tracy-client` 0.19.0 is built on `tracy-client-sys` 0.30.0, pinned in `Cargo.toml`: Tracy 0.14.1, protocol 82;
   another release, 0.13.1 included, refuses to connect, or mis-reads what it is sent). From
   [the release](https://github.com/wolfpld/tracy/releases/tag/v0.14.1): on Windows, `windows-0.14.1.zip` has
   `tracy-profiler.exe`; on Linux, `linux-0.14.1.zip` has `tracy-profiler-x86_64.AppImage` (`chmod +x` it, and run
   it). Both zips have the command-line tools, `tracy-capture` and `tracy-csvexport`, below. To build it from the
   release's source instead: `cmake -B build -S profiler -DCMAKE_BUILD_TYPE=Release && cmake --build build -j` (it
   wants Wayland or, with `-DLEGACY=ON`, X11 and GLFW).
2. **The server, with `WOW_TRACY=1`** in the environment:

   ```sh
   WOW_TRACY=1 cargo run --release                        # Linux
   ```

   ```powershell
   $env:WOW_TRACY = "1"; cargo run --release              # Windows, PowerShell
   ```

   ```bat
   :: Windows, cmd
   set WOW_TRACY=1
   cargo run --release
   ```

3. **Connect.** In the profiler, connect to `127.0.0.1`. The server records only while a profiler is connected
   (Tracy's on-demand mode), so a server left with `WOW_TRACY=1` and nobody watching keeps nothing in memory. To see
   the start-up (the world's load, the continents' spawns), click Connect before starting the server: the profiler
   waits for it, and records from the moment it connects, a fraction of a second in (the first stage or two may be
   missed).

The server listens for the profiler on **127.0.0.1:8086 only**: a profile holds call stacks and the log's warnings,
which are no one else's. To watch a server on another machine, tunnel to it (`ssh -L 8086:127.0.0.1:8086 host`) and
connect to `127.0.0.1`.

**Without the GUI.** Tracy's `capture` tool (`tracy-capture`, in the release's zips) records to a file:
`tracy-capture -o run.tracy -a 127.0.0.1 -s 120` for two minutes. `csvexport` turns one into zone statistics:
`tracy-csvexport run.tracy` (each zone's total, count, mean and maximum), `-e` for self times, `-u` for every zone
and plot point.

**Sampling.** Tracy also samples call stacks, which shows where the time goes inside a zone. Sampling is part of
Tracy's system tracing, and needs the server run as an administrator on Windows (where it is the only sampling
profiler at hand), or as root on Linux. Without those rights there are no samples, and everything
else works. `TRACY_NO_SYS_TRACE=1` in the environment keeps it off even with the rights. Tracy samples at 8 kHz on
Windows and 10 kHz on Linux.

### What it shows

**Frames: one set a map.** Each map's updates are a set of frames named by the map: `Eastern Kingdoms`, `Kalimdor`,
and a copy by its name and instance id, `The Deadmines #3`. A frame is one update, from its start to its end (Tracy's
discontinuous frames), so a set's frame graph is that map's update times, and its frame count over time the update
rate (20 a second, unless a map falls behind). Pick a set in the frame list at the top to see its graph.

**Zones.** The map's thread shows, for each update:

- `update`, the whole update; its value is the map's id.
  - A zone each update step, named by the step (`players`, `gear`, `auras`, `despawns`, `creatures`, `visibility`,
    `values`, `batches`, ...), from `Map::step`. Every step is one, so a slow update is a slow step, by name.
  - Inside the steps where an update's time goes (a capture of 100 bots, below):
    - `despawns`: `scripts' despawns`, `orphaned guardians` and `despawn timers`, each a look through every creature.
    - `values`: `values: players`, `values: game objects`, `values: creatures` (every creature, for its dirty flag).
    - `creatures' combat`: `regeneration`, `leashes`, `watching for enemies`, `assistance`.
    - `auras`: `units with auras`, its value the number of units looked at.
    - `creatures`: `creatures' turns`, one zone for all their turns, its value how many took one.
    - `visibility`: `objects refreshed`, its value how many.
  - `navmesh tile` and `terrain tile`: a tile read from disk the first time it is needed, wherever that is (an
    update step, a packet, the continent's spawns). A long one in an update is a hitch.

Between updates, the map handles what it is sent, a zone each:

- a packet from a client, named by its opcode (`MSG_MOVE_HEARTBEAT`, `CMSG_CAST_SPELL`, ...), so a costly handler
  stands out in the statistics;
  - inside a movement packet, its steps: `move: checks` (the gates, the root and the speed check), `move: taken`
    (the guard, the fall, a cast or shot it breaks), `move: visibility` (the grid, visibility's mark),
    `move: encode` (the relay), `move: broadcast` (with `observers: clients` and `cork: broadcast` in it), and
    `move: after` (the stand, the auras it ends, the safe place);
  - with `[world] parallel_moves`, a burst's movement: `moves: stepped` (across the threads) and `moves: told`
    (what it tells the map), a packet handled alone by its opcode as above;
- `cork: push`: a cork's chunk published to the map's feed, and what it kept for clients of no feed pushed;
- `arrival` (a player logging in or coming from another map), `disconnect`, and `call` (another map or service
  changing something this map owns).

Elsewhere:

- the writers: `writer: burst`, a client's frames copied and their headers encrypted for one write, its value
  how many; the feed's waker: `feed: wake`, a feed's new chunks charged to its clients and their writers woken
  (`conn::Feed`);
- the store's thread: `store transaction`, a zone per transaction written, its value how many saves it held; and
  `store request` for each read (a character list, a login's load).
- start-up: `load: the character store`, then `load` (the world's data) with a zone each stage (`load: dbc`,
  `load: vmangos`, `load: npcs`, `load: spells`, ...); then each map's `populate`, its spawns.

Tracy's statistics window (the zone list, by total or by count) is the place to start: sort by total time to see
where the time went, and by mean to see what is slow each time.

**Plots,** once a second:

| Plot | |
|---|---|
| `<map>: players` | players in the map |
| `<map>: creatures awake` | creatures in the update's active set (`Map::awake`), those some player sees or in a fight |
| `<map>: mailbox` | events waiting for the map: a map that can't keep up grows it |
| `<map>: outbound queued` | bytes queued to the map's players and not yet written: a client too slow to read grows it, and is disconnected at `outbound_channel_bytes` |
| `realm: players online` | everyone in the world |
| `realm: packets in a second`, `realm: packets out a second` | all clients' packets |
| `realm: store queue` | requests waiting for the store's thread |

**Messages:** the log's warnings (yellow) and errors (red), on the timeline where they happened: an update that took
too long, a panic, a lost save. The log's other lines stay out.

### Without Tracy: the world benchmark's probe

With the `world-bench` feature, while the maps' probe records (`map::bench_probe::enable`), every zone on a map's
thread is timed too, Tracy off: its count and wall time by name, a map's alone with `bench_probe::zones_of`, taken
with `bench_probe::take_zones`. `benches/battle.rs` prints them (`BATTLE_ZONES=40`: the 40 dearest), beside each
step's CPU time and the slowest updates' dearest steps (`BATTLE_SLOWEST`). A zone added to look inside a step shows
there at the next run, with no profiler and no rights (this machine's `perf_event_paranoid` is 2). Zones on other
threads (the visibility pool's, the writers') aren't counted.

### Adding a zone

`src/tracy.rs` has the helpers. A zone named by a literal lasts to the end of its scope:

```rust
let _z = crate::zone!("pathing");          // `_z`, not `_`, which would end it at once
```

with a number shown with it:

```rust
let z = crate::zone!("units with auras");
z.value(units.len() as u64);
```

One named at run time, from a `&'static str` (each distinct name is made once, and kept):

```rust
let _z = crate::tracy::Zone::named("packet", packet_name(&msg));
```

A zone isn't `Send`: it is for synchronous code, and can't be held over an `.await` (Tracy wants a zone to end on
the thread it began on; a map's update and handlers never await). Keep zones to what runs a few thousand times a
second at most: one around a loop over creatures, with their count as its value, not one for each.

`Plot::new("name", Format::Number)` and `plot.set(v)` add a plot, `FrameName` and `Frame` a set of frames; see
`MapProfile` in `src/game/map/mod.rs`. A test can check what is emitted: `tracy::record::start()` turns the helpers on
without Tracy, and `record::take()` returns every zone, frame and plot since (`src/tracy.rs`'s tests).

### What it costs

**Off** (no `WOW_TRACY=1`), every helper is a relaxed load of a flag and a branch: no Tracy thread, no port, nothing
recorded. Built with `--no-default-features`, they are nothing at all, and Tracy's C++ isn't built.

Measured as [PERFORMANCE.md](PERFORMANCE.md#how-it-was-measured) measures (the server's CPU time from `/proc`,
over 60 s), release builds of this branch's start (no zones) and of its zones, with `WOW_TRACY` unset, run after
run, alternating, on this shared machine (load average 5 to 21):

| Tracy off | Before (no zones) | After (the zones) | |
|---|---|---|---|
| Idle, both continents | 0.092, 0.094, 0.094, 0.094 cores | 0.063, 0.087, 0.095, 0.093, 0.086 cores | 0.094 and 0.085 on average |
| cluster, 100 bots | 0.366, 0.349 cores | 0.364, 0.358 cores | 0.357 and 0.361 |

The two are within each other's noise: no cost to be measured. The fast suite's
`helpers_off_emit_nothing_and_cost_a_flag` checks it at the scale of one zone (a million zones off, unoptimized,
under a microsecond each).

**On, with a profiler connected,** a zone costs what Tracy's own do, tens of nanoseconds at most (not measured
here); an update has about 60, and a map handles a few hundred packets a second, each one zone. A zone named at run time also takes a short lock to find its
name's location.

The `tracy` feature is on by default, so the release binary a player builds can be profiled by setting `WOW_TRACY=1`,
on Windows as on Linux, without a rebuild, at the cost of a flag's load per zone while off.

### What a capture showed

A capture of peon's cluster scenario (`--mode random --spread cluster --mix wanderer=1`), 100 bots ramped over 60 s,
of which 96 to 99 logged in, all in one spot on Eastern Kingdoms, with Kalimdor empty; Tracy's `capture` for 230 s,
then `csvexport -u` and a script that charges each zone to its map by the `update` zone around it. Over the 110 s
of the hold, each continent ran its 20 updates a second (2,200 each):

| Eastern Kingdoms, 96 players | per update | | Kalimdor, no one | per update | |
|---|---|---|---|---|---|
| **the update** | **2.81 ms** (p99 4.0, max 13.2) | | **the update** | **2.30 ms** (p99 3.0, max 6.9) | |
| `despawns` | 0.99 ms | 35% | `despawns` | 1.09 ms | 47% |
| &nbsp;&nbsp;`scripts' despawns` | 0.40 ms | | &nbsp;&nbsp;`scripts' despawns` | 0.44 ms | |
| &nbsp;&nbsp;`orphaned guardians` | 0.31 ms | | &nbsp;&nbsp;`orphaned guardians` | 0.35 ms | |
| &nbsp;&nbsp;`despawn timers` | 0.29 ms | | &nbsp;&nbsp;`despawn timers` | 0.30 ms | |
| `values: creatures` | 0.45 ms | 16% | `values: creatures` | 0.52 ms | 23% |
| `auras` (1,711 units with auras) | 0.39 ms | 14% | `auras` (2,283 units) | 0.38 ms | 17% |
| `pets` | 0.27 ms | 10% | `pets` | 0.00 ms | |
| `creatures' combat` (of it, `regeneration`) | 0.22 ms (0.21) | 8% | `creatures' combat` (`regeneration`) | 0.23 ms (0.23) | 10% |
| `visibility` (2.7 objects refreshed) | 0.16 ms | 6% | | | |
| `objects` (the traps) | 0.15 ms | 5% | | | |
| the other 33 steps | 0.17 ms | 6% | the other steps | 0.07 ms | 3% |

So most of an update, crowded or empty, is the looks through every creature of the continent that are left (23,350
and 27,361 of them): the three of `despawns` (each a deref of every scripted creature's box), the creatures' dirty
flags, regeneration, and the auras' set. They cost the same with no one on: the crowded continent's update is only
0.5 ms longer than the empty one's, mostly the pets' look (NPCs' pets are out there), visibility and the traps.
Between updates, Eastern Kingdoms' actor handled 293 heartbeats a second at 46 µs each (13 ms a second), and the
other movement packets 2 ms a second; everything else (queries, chat, emotes) under 0.1 ms a second.

Outside the hold: the first updates of each continent were the slowest of the run, 791 ms and 346 ms (and 165 ms
when the first bots arrived), and the zones say why: 15 `navmesh tile` reads of 20 to 116 ms each, under `despawn
timers`, `pets` and `visibility`, the first time a creature or player needed a tile. Before them, each continent's
`populate` took 3.1 to 3.3 s, 101 tile reads of it; and the world's data 0.9 s (`load: vmangos` 457 ms, `load:
spells` 324 ms, `load: npcs` 67 ms, `load: scripts` 40 ms). The store wrote 127 transactions, 1.6 ms on average
(78 ms at most), on its own thread.

The capture (2.8 MB, 572,832 zones) was waiting before the server started, but Tracy's on-demand mode records
from the moment the connection is made, a fraction of a second in, so the first stages (the character store's
opening, `load: dbc`) are not in it. In a later, shorter capture they were: the connection beat them.

### What was dropped

The earlier setup opened no zone of its own, and sent every log line to Tracy, through `tracing-tracy`. Now:

- `tracing-tracy` is gone: the zones are `tracy-client`'s, through `src/tracy.rs`, and a small `tracing` layer sends
  warnings and errors only.
- `broadcast` (announcing the server on the network), `code-transfer` and `context-switch-tracing` are off.
  `system-tracing` stays, as Tracy's sampling is part of it: inert without an administrator's (or root's) rights.
  (The captures above were taken without it; a later one with it, unprivileged, was the same.)
- `ondemand` and `only-localhost` are on, for the reasons above.
- The `pprof` feature (a sampling profiler on SIGPROF, writing a flame graph, with `WVS_PPROF`) is gone: the sampling
  profiler is Tracy's, which needs root (or an administrator) where pprof needed nothing, and a capture without the
  GUI is `tracy-capture`'s.

