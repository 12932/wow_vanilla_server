# Performance

wvs was already far cheaper than Thistle Tea: about 0.2 to 0.4 cores for peon's 100 bots, where Thistle Tea takes
6 ([THISTLE_TEA_DIFFERENCES.md](THISTLE_TEA_DIFFERENCES.md)). This is a pass over where that CPU went after the
parity work, and what was done about it: the scenarios and how they were measured, the numbers before and after, the
profiles' top entries, what changed, and what is left.

In short: most of the server's CPU, idle or under load, went on the map update looking at every creature of a
continent (23,000 on Eastern Kingdoms, 27,000 on Kalimdor) a dozen times an update, twenty updates a second, for
states that few or none of them were in. Those looks now skip what can't be found, or go through the few creatures
that could be it. And the world's data now loads in a tenth of the time, which every server start and every
real-data test gains.

| | Before | After | |
|---|---|---|---|
| Idle (no one on, both continents) | 0.17 to 0.19 cores | 0.04 cores | 77% less |
| cluster, 100 bots | 0.38 to 0.41 cores | 0.23 to 0.26 cores | 38% less |
| pvp-arena, 100 bots | 0.25 to 0.26 cores | 0.11 to 0.12 cores | 55% less |
| The world's data, `load()`, release | 7.6 to 9.9 s | 0.9 to 1.2 s | a tenth |
| The world's data in a debug test | 34.4 s | 9.1 to 9.6 s | 73% less |
| The test suite (`--test-threads 6`) | 595 s (the tests' own time 3,445 s) | 215 s (1,159 s) | 64% less |

The suite's two runs were one after the other, at load averages of 15 to 28 (the start's) and 24 to 16 (the end's),
the end's with its dependencies built afresh first (2 minutes, once).

## How it was measured

**The server.** `cargo build --release --bin wow_vanilla_server`, run from a directory of its own outside the
worktree (`runs/perf-srv`), with a copy of `config.toml` whose `[world]` points at the real data: `data_dir` at
`wow_vanilla_server/data/db`, `nav_dir` at the Thistle Tea 36-map bake (`runs/tt-maps/rustigator`) and
`terrain_dir` at `runs/tt-data/terrain`. Each run starts on an empty `characters.sqlite`. The binaries compared are
copies, so a build in the worktree never changes what is being measured.

**The load.** peon's baseline plans (`plans/baseline/*.toml`) with the ramp kept at 60 s and the hold cut to 120 s,
one worker, as `scripts/baselines.sh` runs them: `cluster` is `--mode random --spread cluster` (100 wanderers in one
spot), `pvp-arena` is `--mode pvp` (100 bots fighting in Gurubashi Arena's pit, revived with `.revive`). Idle is the
same server with no one on.

**The CPU.** The server's `utime + stime` from `/proc/<pid>/stat`, read 10 s into the hold and 15 s before its end
(105 s), divided by the time; idle, over 60 s from 20 s after the server listens. The load average is noted with each
run. This 16-core machine is shared with other agents (load average 4 to 25 during these runs), so wall-clock
latencies are noise. CPU time per unit of work is fairer, but not wholly fair: the update's looks are bound by memory,
and a busy machine's caches make them dearer. So the final comparison alternates the two builds, run after run.

**The same bots.** peon gives each bot a random look from its seed and slot, and the server refuses a character whose
look the client has no texture for (`appearances.valid`): 11 to 19 of 100 bots a run never get in, which ones
depending on the slots. With a fresh character store each run, the final comparison gives both builds the same slots,
so the same bots: 84 or 85 in pvp-arena, 86 in cluster.

**Profiling.** (Tracy, the frame profiler, and this pprof feature are both described in
[PROFILING.md](PROFILING.md).) `perf_event_paranoid` is 2 here, and neither `perf` nor `samply` will record, so there is now a
`pprof` cargo feature (off by default; `src/profile.rs`). It is driven by SIGPROF, which needs no permissions:

```sh
CARGO_PROFILE_RELEASE_DEBUG=line-tables-only cargo build --release --features pprof \
  --bin wow_vanilla_server --target-dir target/prof
WVS_PPROF=/tmp/cluster WVS_PPROF_DELAY=75 WVS_PPROF_SECS=100 target/prof/release/wow_vanilla_server
```

It writes `/tmp/cluster.svg`, a flame graph, and `/tmp/cluster.txt`: the samples by function and by line, the self
time charged to the innermost line of wvs's own source, and the inclusive time of wvs's functions. SIGPROF comes at
the kernel's tick, so whatever `WVS_PPROF_HZ` asks, a process gets about 100 samples a CPU-second: a 100 s profile of
0.3 cores is about 3,000 samples, enough for the top entries to a percent or two.

## Where the time went

The profiles before (the branch's start, with only the loader's change, which is over before a profile begins),
inclusive, by wvs's functions:

| | idle | cluster | pvp-arena |
|---|---|---|---|
| `creature_combat::update` (regeneration, watch, assistance, zone pulses, flights) | 43.9% | 35.8% | 55.4% |
| `ai::update_creatures` (whose turn it is) | 14.8% | 9.4% | 6.0% |
| `regenerate`'s reads of health and mana (`Fields::get_u32`) | 13.6% | 7.7% | 12.1% |
| `assistance::update` | 10.8% | 11.0% | 16.1% |
| `watch_for_enemies` | 10.6% | 3.6% | 14.6% |
| `combat::update_players` (whom creatures threaten) | 8.2% | 3.2% | 10.7% |
| `charge::update` | 7.0% | | 3.0% |
| `update_despawns` | 5.3% | | 3.7% |
| `flush_values` (of it, `Fields::is_dirty`) | 4.6% | 11.9% (11.1%) | 3.0% |
| `pets::update` | 4.5% | | 3.0% |
| `auras::update` | 3.8% | 3.0% | 3.4% |
| packets: `on_move`, `send_to_observers`, the writers | | 7.1%, 4.6%, 8.3% | about 1% each |

Nearly all of it is one thing: `map.creatures.iter().filter(...)` over 23,000 or 27,000 creatures, each a 2,088-byte
struct, for one or two fields of each. Each look touches a different line of each creature, so each is tens of
thousands of cache misses, and there were a dozen an update: about 4.5 ms an update for an empty continent, some
15 ns a creature a look. Two of the looks went further, to the heap: `regenerate` read every creature's health and
mana from its fields' values, and `flush_values` every object's dirty mask, each a vector of its own. Under load the
same looks were most of the CPU: the update was 77% of the cluster's samples and 97% of pvp-arena's, and the packets
(moving, relaying, writing) about a fifth of the cluster's.

## What changed

Each is its own commit, with its measurement; the numbers there are from runs taken one after the other, before
and after that commit.

1. **An optional profiler** (`src/profile.rs`, the `pprof` feature), above.
2. **The loader reads columns by index** (`data/load.rs`). Every value the loader read by name went through
   rusqlite's lookup by name, a scan of the statement's columns comparing names without regard to case: for
   `spell_template`'s 180-odd columns and 30,000 rows, tens of thousands of comparisons a row. Each statement's names
   are now mapped to their indices once, with rusqlite's rules (ASCII case ignored, the first of two alike wins), and
   numbers are read from the borrowed value. A test pins the rules down, and passes on the old reader too. `load()`:
   7.6 to 9.9 s to 0.9 s in release; 34.4 s to 13.3 s in a debug test.
3. **Fields say whether they changed** (`fields.rs`, `update.rs`): a flag beside the dirty mask, so the update's
   question to every object needn't read the mask on the heap; a clean mask isn't cleared again, and `values_block`
   says nothing for fields with nothing changed (a player's every item, every update). Idle 0.185 to 0.165 cores.
4. **Rare states skip their looks** (`RareStates`, `Map::rare`): a charge, a flight, a dungeon-wide pulse (withdrawn
   in 9), a call for help on its way. The update looks at every creature for one only while one may be in it; what
   puts a creature in one notes it (`add_creature` notes what a creature arrives with), and a look that finds none
   clears it. Whom creatures threaten is looked for only while some player is in a fight. Idle 0.165 to 0.124.
5. **The awake creatures** (`Map::awake`, `Map::awake_where`). Five looks were for what only a creature some player
   sees, or one in a fight, could be: a creature's turn, a civilian watching for enemies, a call to friends from far
   from home, whom creatures fighting threaten, and the aura states of those fighting. The map keeps the creatures that
   may be awake: a player coming to know one adds it (`visibility::introduce`, the only place observers are added), as
   do the places a fight begins (`combat_mut`, `ai::attacked`, a pet's attack, an evade, a feign death), and
   `awake_where` lets go of those no longer awake as it answers, in the order a look through every creature would find
   them. Idle 0.124 to 0.082; pvp-arena 0.259 to 0.163 (an early pair of runs, on other bots).
6. **Regeneration's look reads no heap** (`Fields::below_max`): fields keep whether a unit's health or mana is below
   its maximum, as those four fields change, and the look asks that first. Idle 0.086 to 0.078; pvp-arena 0.158 to
   0.143.
7. **Casts, despawn timers (withdrawn in 9) and pets join the rare states**; every cast begins in `Map::caster_mut`,
   which notes it. Idle 0.073 to 0.055.
8. **Traps and crowd control** (`objects.rs`, `auras.rs`): a trap nobody owns springs only on players, so it no longer
   builds a unit view of every creature near it to learn whether it lives; and a unit whose auras have nothing due
   skips the second look, for crowd control due. pvp-arena 0.122 to 0.100.
9. **Two notes withdrawn**, for safety. master has moved on in despawn timers and zone pulses since this branch began
   (a script's own zone pulse; a summon's time started again after its fight), setting them where no note would be
   taken, and a missed note is a timer that never fires. The despawn timers' look is back as it was (idle 0.045 to
   0.056), and zone pulses are looked for on a dungeon's or a raid's map only, as only there does a creature pulse its
   map (`zone_fighter` and the script's pulse both ask the map's type), whoever sets the timer.
10. **The dev profile optimizes the dependencies** (`[profile.dev.package."*"] opt-level = 2`). The crate stays at
    opt-level 0, so an edit builds as quickly as before (26 s for an incremental test build after a change to a
    function, either way); SQLite, tokio, wow_messages, the navmesh and compression run optimized in the tests. The
    suite 277 s to 189 s; the tests' own time 1,638 s to 1,095 s.
11. **Only changed game objects are flushed.** Every change to a game object goes through `Map::gameobject_mut` (the
    only other mutable access is the flush), or comes with `add_gameobject`, so both note its key, and `flush_values`
    looks at those instead of all 8,500 of a continent. Idle 0.057 to 0.054.
12. **The creatures with auras** (`Map::with_auras`, `with_auras_where`). An aura is only ever added through
    `auras::auras_mut`, which now adds its creature to the set, as `add_creature` adds one arriving with auras; the
    aura update, a single-target aura's caster dying or casting another, and Separation Anxiety go through it, in key
    order. cluster 0.228 to 0.169; idle 0.046 to 0.038.

**Exactness.** None of this changes what the server does: each shortcut finds what the look it replaces found, in the
same order. A note missed would break that, so debug builds check every shortcut against the look it replaces, every
time: a skipped look with a `debug_assert!` that no creature is in the state, each `awake_where` and
`with_auras_where` with a `debug_assert_eq!` against the look through every creature, the flush with one that no game
object is left dirty, and the two field flags against the fields. The suite runs with them on, and so did a run of it
on this branch merged with master (see below).

**For whoever changes this code.** Putting a creature in a fight (`attacking`, `attackers`, `evading`, a feign death)
goes through `Map::combat_mut` or calls `Map::note_fighting`. Starting a charge, a flight or a call for help sets
`map.rare.charge`, `.flee` or `.assist` first. A cast goes through `Map::caster_mut`, an aura through
`auras::auras_mut`, a game object's change through `Map::gameobject_mut`; making a creature a pet sets
`map.rare.pet`; and a creature's observers are added only by `visibility::introduce`. A debug build names the note
missing, and the map, at the first update that would have gone wrong.

## After

The final comparison: the branch's start against its end, alternated run after run, the same bots for both.

| | The start (`c2bb0e7`) | The end | Change | The work |
|---|---|---|---|---|
| Idle | 0.171, 0.172 cores | 0.039, 0.041 cores | 77% less | |
| pvp-arena | 0.262, 0.252 cores | 0.114, 0.117 cores | 55% less | 84 and 85 bots; 314 and 327 deaths to 346 and 343; 7,288 and 7,775 packets a second to 7,979 and 8,029 |
| cluster | 0.412, 0.377 cores | 0.262, 0.227 cores | 38% less | 86 bots; 22,333 and 22,104 packets a second to 22,043 and 22,204 |

The load average was 8 to 14 for the idle and pvp-arena runs, 10 to 21 for cluster's. An earlier comparison, before
the last two commits, gave idle 0.197 and 0.184 to 0.056 and 0.069, pvp-arena 0.248 and 0.252 to 0.119 and 0.129, and
cluster 0.384 and 0.353 to 0.244 and 0.261. The bots do as much against the end, or more (more deaths in pvp-arena),
and peon's latencies are as before: login 8 to 13 ms at the median, a ping's round trip 0.1 ms, a movement's relay 1 to
2 ms (one cluster run's login p99 was 2.5 s, at a load average of 21). peon's reports are in each run's directory under
`runs/perf-srv/out/h-*`.

The profiles after, inclusive, by wvs's functions:

| | idle (361 samples) | cluster (612) | pvp-arena (410) |
|---|---|---|---|
| `update_despawns` | 36.3% | 14.7% | 20.0% |
| `regenerate` | 30.5% | 17.5% | 10.0% |
| `pets::update` (pets are out, some NPCs') | 16.1% | 10.3% | 11.5% |
| `auras::update` | 10.0% | 9.8% | 7.8% |
| `flush_values` | 4.4% | 8.5% | 8.3% |
| `visibility::refresh` | | 2.3% | 9.3% |
| the traps (`objects::update`) | | 4.7% | 7.1% |
| packets: `on_move`, `send_to_observers`, the writers | | 7.5%, 5.1%, 11.4% | 0.7%, 1.5%, 5.6% |

What is left is the same kind of look, fewer of them: the four remaining looks through every creature
(`update_despawns`, `regenerate`, `pets::update`, and `flush_values`' creatures) are most of an idle server, and half of
a loaded one; under load the rest is the work the bots ask for. The server is now about 0.02 cores a continent when
no one is on.

## What is left

- **Looks through every creature that remain.** `update_despawns` (a box's deref for each scripted creature),
  `regenerate` (now one line of each creature) and `flush_values`'s creatures (every one, for its dirty flag) are most
  of what idle is now; `pets::update` while any pet is out; the Elemental Invasion's. Each could go the way of the
  ones above, but a note per state is a note someone can forget. Despawn timers are set in a dozen places, by code
  other agents are changing now, and creatures are changed through `creature_mut` and a few direct borrows of the
  slab, with no one place to note a change.
- **Despawn timers**: their skip saved 0.011 cores idle. Done again after the merge, with every setter in view (or one
  setter function), it is worth having.
- **The creature struct.** At 2,088 bytes, a look at one field of every creature is a cache miss a creature. A hot part
  kept apart (alive, in a fight, seen, the timers) would make each remaining look a fraction of what it is, but it
  changes the most-used struct of the codebase.
- **The debug checks.** Checking one update in eight, rather than every one, made the heavy tests 7 to 17% faster.
  They check every update: a check skipped is a missed note not caught.
- **The crate at opt-level 1 for tests.** With the dependencies at 2 as well, the heavy real-data tests took half the
  time (the world's load 1.6 s; the scripted bosses 26 s to 12 s; the encounters 34 s to 19 s), but an incremental
  test build after a change went from 26 s to 56 to 67 s. Not done: an agent editing and testing pays that on every
  build.
- **The slowest test**, `event_quests_with_their_map_events_are_done`, is most of the suite's wall time (166 s of 189
  with the dependencies optimized): one test that goes through triage, two escorts, defenses and a brawl on the real
  continents, each step a query that looks through every creature. Split in five, it would run on five threads.
- **A flaky test**: `flat_healing_taken_goes_by_the_spells_coefficient` failed once in five suite runs here, and once
  in twelve runs alone on the code before this branch (Flash Heal for 284, its bound 280). It isn't this work's, but
  it will fail now and then.
- **peon's bots**: 11 to 19 of every 100 fail character creation on a look the server refuses (`CharCreateFailed`),
  so a scenario's 100 bots are 81 to 89. peon could pick its looks from the same table.

## Merging

This branch began at `c2bb0e7`. Merged (in a scratch copy, not committed) with master at `20b62a1`, it has two
conflicts, both of the keep-both kind: `data/load.rs`'s imports (master's `AHashSet`, this branch's `ValueRef`), and
`flush_values`, where master's `let mut taps = Vec::new();` goes after this branch's check that no game object is left
dirty. So merged, the suite ran 1,084 tests: 1,077 passed and 7 failed.

- Five fail on master alone too: `spell_logs_are_laid_out_as_1_12_has_them`,
  `a_flight_masters_node_and_map_are_laid_out_as_1_12_has_them`,
  `a_quest_too_high_is_refused_and_hidden_rewards_are_not_shown`, `the_imported_event_quests_are_done_end_to_end` and
  `the_nightmare_manifests_is_done_end_to_end`.
- Two are the awake check doing its work: `tests_p3_combat2`'s `fighting` helper, new on master, puts a creature in a
  fight by setting `combat.attacking` itself, and a creature no one sees in a fight that the map wasn't told of is one
  whose turn a release build would skip. With `m.note_fighting(brute);` after the helper sets it, the file's eight
  tests pass.

An earlier merge, with master at `a2a2322`, had no conflicts, and its suite passed whole (774 tests).

## The whole world awake

The map update looks only at creatures a player sees or that are fighting, which keeps an idle world cheap. This
section puts players everywhere at once, so that every creature of Eastern Kingdoms (23,363 spawned) and Kalimdor
(27,476) is awake, and asks whether one continent's single 20 Hz loop keeps its 50 ms.

In short:

- **Up to 25% of the points (1,618 observers, 9,000 to 10,000 awake creatures a continent), both continents keep
  50 ms.** Their CPU per update is 12 to 14 ms at the median and 31 ms at p99.
- **With the whole world awake, they don't.** A continent's update averages 30 ms of CPU (Eastern Kingdoms) and 38 ms
  (Kalimdor). Every tenth update costs about twice that, and 9% to 25% of updates run past 50 ms.
- **Observers that walk** make it worse: navmesh tiles are read from disk on the map's thread, and at 100% the update
  rate falls to 9 a second.
- **Memory** is the other limit: 13 GB at 100%, nearly all of it navmesh.

### The points

`tools/world_points.py` is a greedy set cover of the creature spawns of maps 0 and 1 (vmangos's `creature` rows in
patch 10). Each point is a spawn, chosen as the one whose 90-yard radius takes in the most spawns not yet covered (ties
go to the lowest guid). Points are added until every spawn is covered. The z comes from the spawn. The points are then
put in one seeded order over both continents, so that the first N lines of the file are a fair sample of the world.
The script uses the standard library only, and a rerun writes the same file.

```sh
uv run --python 3.14 python -I tools/world_points.py $WVS_DATA/db/vmangos.sqlite docs/world_points.csv
```

| | Spawns | 100-yard cells with a spawn | Points |
|---|---|---|---|
| Eastern Kingdoms (0) | 24,572 | 5,006 | 2,562 |
| Kalimdor (1) | 28,971 | 7,644 | 3,909 |
| Both | 53,543 | 12,650 | 6,471 (`docs/world_points.csv`, 160 kB) |

Coverage is the share of each continent's spawns within 90 yards of the first N points of the file:

| The file's first | Points | Eastern Kingdoms | Kalimdor |
|---|---|---|---|
| 10% | 647 (269 on 0, 378 on 1) | 15.0% | 13.9% |
| 25% | 1,618 (670, 948) | 34.7% | 33.4% |
| 50% | 3,236 (1,286, 1,950) | 62.3% | 61.3% |
| 100% | 6,471 (2,562, 3,909) | 100% | 100% |

Coverage grows faster than the share of points, because a point picked early in the cover takes in a dense cluster.
The 6,471 points are about a quarter of the 23,000 a bot every 100 yards would need, and twice a real 1.12 realm's
3,000 players. They are all in the open world, each with a crowd of creatures around it, so this is the worst case of
the update's looks, not a realistic evening.

### The benchmark

`wvs-world-bench` (`src/bin/wvs-world-bench.rs`) runs the server alone in one process, behind the `world-bench`
feature. Without the feature the binary isn't built, and the maps carry no probe, so the normal build and the test
suites are unchanged. The benchmark works like this:

1. **Load.** It reads the server's `config.toml` for the world's directories, loads the world as `game::run` does,
   starts both continents' actors, and waits for them to populate.
2. **Log in.** It logs in N new characters through the real session over channels, as the tests' harness does (no
   sockets, no encryption). Each one:
   - ends its intro (until then a client sees from the intro's camera);
   - says `.gm on` (creatures look for it and leave it be), then `.go xyz` to its point;
   - answers the teleport or worldport.
3. **Run.** The observers either stand, or walk (`--walk`): a heartbeat every 250 ms at running speed, turning now
   and then, at the point's height. A thread drains their queues every 50 ms.
4. **Measure.** After `--settle` seconds, it records every update of both continents for `--secs` seconds.
   `map/bench_probe.rs` collects, for each update:
   - its wall and thread CPU time;
   - each step's CPU time;
   - the creatures awake at its end;
   - its context switches.

   It also collects the CPU time of the events between updates. For the whole process it records the CPU in cores
   and peak RSS from `/proc/self`, plus major faults and disk reads. An update over 1 s, or an event over 200 ms, is
   printed with its dearest steps.

```sh
cargo build --release --features world-bench --bin wvs-world-bench --target-dir target/bench
cd runs/worldawake        # a config.toml whose [world] has data_dir, nav_dir and terrain_dir, as above
wvs-world-bench --config config.toml --points world_points.csv --fraction 0.25 --secs 60 --settle 10 \
  --logins 256 [--walk] [--json out.json]
# the profile of the measurement:
CARGO_PROFILE_RELEASE_DEBUG=line-tables-only cargo build --release --features world-bench,pprof \
  --bin wvs-world-bench --target-dir target/benchprof
WVS_PPROF=/tmp/world WVS_PPROF_SECS=30 WVS_PPROF_HZ=100 wvs-world-bench ... --fraction 1.0 --secs 30
```

A run takes 1 to 2.5 minutes: 2 to 16 s to populate, 30 to 60 s for the logins, then the settle and the
measurement.

**Columns.** The tables below use these:

- **CPU** is the map thread's CPU time over the update. It is the fairer number on this machine: about 30% of what
  the wall time adds is preemption by other agents' work, and on walking runs, waits.
- **Over 50 ms** is wall time.
- **Busy** is the share of the run the map's thread spent in updates and events.
- **Off CPU** is the share of the updates' wall time spent off the CPU.

Updates under 20 a second mean the loop fell behind: the ticker waits for an update to end.

**Load.** The machine is shared with other agents. The load average is given for each run: 17 to 31 for the main
series, 12 to 17 for the two repeats at 100%. None of these were taken at a quiet time.

The main series, one after another:

| Observers | Walk | Map | Updates/s | Wall p50 / p95 / p99 / max ms | CPU p50 / p95 / p99 / max ms | Over 50 ms | Awake mean / max | Busy | Off CPU | Load |
|---|---|---|---|---|---|---|---|---|---|---|
| 647 | no | 0 | 19.8 | 6.8 / 13.5 / 21.5 / 324 | 6.8 / 12.7 / 13.9 / 46 | 0.1% | 3,910 / 3,934 | 15% | 9% | 24-20 |
| 647 | no | 1 | 19.8 | 7.8 / 15.5 / 23.4 / 92 | 7.7 / 14.1 / 15.9 / 19 | 0.1% | 4,239 / 4,287 | 16% | 8% | 24-20 |
| 647 | yes | 0 | 17.0 | 7.3 / 52.9 / 222 / 803 | 7.0 / 12.9 / 19.3 / 80 | 5.3% | 3,352 / 4,014 | 14% | 53% | 17-22 |
| 647 | yes | 1 | 15.3 | 8.4 / 51.5 / 326 / 1,356 | 8.1 / 15.4 / 46.7 / 120 | 5.2% | 3,669 / 4,335 | 17% | 52% | 17-22 |
| 1,618 | no | 0 | 18.9 | 12.0 / 29.2 / 41.5 / 1,051 | 11.7 / 27.0 / 31.3 / 166 | 0.5% | 8,894 / 8,938 | 26% | 15% | 23-21 |
| 1,618 | no | 1 | 19.5 | 13.8 / 30.0 / 37.7 / 165 | 13.6 / 28.3 / 31.3 / 137 | 0.6% | 9,980 / 10,079 | 30% | 8% | 23-21 |
| 1,618 | yes | 0 | 15.4 | 12.5 / 65.9 / 217 / 2,555 | 11.9 / 24.8 / 36.7 / 61 | 7.0% | 7,431 / 8,776 | 23% | 47% | 21-24 |
| 1,618 | yes | 1 | 15.4 | 14.5 / 72.1 / 222 / 485 | 14.0 / 27.7 / 42.1 / 113 | 7.3% | 8,376 / 10,015 | 27% | 37% | 21-24 |
| 3,236 | no | 0 | 17.1 | 23.7 / 64.6 / 106 / 225 | 19.4 / 47.6 / 53.4 / 58 | 11.3% | 15,406 / 15,474 | 39% | 26% | 26-30 |
| 3,236 | no | 1 | 17.3 | 27.8 / 71.5 / 130 / 287 | 23.0 / 52.4 / 60.5 / 68 | 15.9% | 17,724 / 17,868 | 46% | 24% | 26-30 |
| 3,236 | yes | 0 | 14.9 | 24.7 / 82.1 / 248 / 1,321 | 20.5 / 43.7 / 60.2 / 86 | 12.2% | 12,886 / 15,333 | 39% | 38% | 31-31 |
| 3,236 | yes | 1 | 14.6 | 28.9 / 89.8 / 329 / 1,465 | 24.0 / 49.3 / 67.6 / 152 | 15.4% | 14,759 / 17,441 | 45% | 36% | 31-31 |
| 6,471 | no | 0 | 15.8 | 28.1 / 89.7 / 153 / 638 | 25.0 / 70.1 / 81.0 / 114 | 19.9% | 23,362 / 23,395 | 48% | 23% | 31-31 |
| 6,471 | no | 1 | 16.6 | 32.3 / 84.3 / 134 / 251 | 29.3 / 74.2 / 86.7 / 111 | 25.1% | 27,436 / 27,463 | 58% | 18% | 31-31 |
| 6,471 | yes | 0 | 9.3 | 28.3 / 91.8 / 411 / 7,173 | 27.2 / 64.0 / 87.0 / 193 | 18.1% | 19,934 / 22,287 | 35% | 54% | 24-30 |
| 6,471 | yes | 1 | 9.6 | 32.3 / 150 / 1,931 / 3,978 | 31.7 / 70.0 / 136 / 217 | 21.6% | 23,377 / 26,154 | 42% | 53% | 24-30 |

The 100% standing runs, repeated at a lower load. The second ran with the profiler on, sampling at 100 Hz:

| Run | Map | Updates/s | Wall p50 / p95 / p99 / max ms | CPU p50 / p95 / p99 / max ms | Over 50 ms | Off CPU | Load |
|---|---|---|---|---|---|---|---|
| repeat | 0 | 15.8 | 29.9 / 93.3 / 183 / 286 | 25.4 / 65.4 / 72.8 / 84 | 22.1% | 26% | 12-16 |
| repeat | 1 | 14.8 | 35.7 / 116 / 192 / 356 | 31.5 / 76.6 / 87.2 / 105 | 24.9% | 24% | 12-16 |
| profiled | 0 | 18.2 | 24.4 / 51.3 / 55.8 / 60 | 24.4 / 51.3 / 55.7 / 60 | 8.6% | 0% | 17-15 |
| profiled | 1 | 18.1 | 29.3 / 69.6 / 74.6 / 256 | 29.2 / 69.4 / 72.8 / 83 | 11.0% | 2% | 17-15 |

The process as a whole (the main series):

| Observers | Walk | Cores | Peak RSS | To the observers | Major faults/s | Disk read | Movement events/s (0, 1) |
|---|---|---|---|---|---|---|---|
| 647 | no | 0.32 | 8.9 GB | 36 kB/s | 0 | 0.2 MB/s | |
| 647 | yes | 0.32 | 10.0 GB | 102 kB/s | 33 | 7.7 MB/s | 1,043, 1,420 |
| 1,618 | no | 0.56 | 11.3 GB | 93 kB/s | 13 | 0.6 MB/s | |
| 1,618 | yes | 0.53 | 11.9 GB | 288 kB/s | 62 | 5.6 MB/s | 2,577, 3,646 |
| 3,236 | no | 0.87 | 12.1 GB | 195 kB/s | 0 | 0 | |
| 3,236 | yes | 0.88 | 12.7 GB | 672 kB/s | 66 | 3.9 MB/s | 4,626, 6,558 |
| 6,471 | no | 1.09 | 12.8 GB | 397 kB/s | 3 | 0.1 MB/s | |
| 6,471 | yes | 0.92 | 13.0 GB | 1,407 kB/s | 62 | 3.1 MB/s | 7,254, 9,437 |

The two repeats at 100% took 1.07 and 1.17 cores, with peak RSS of 12.4 and 12.7 GB. With 20 observers the process is
2.7 GB. A run with 300 observers, after the merge with master that brought Tracy's zones, took 0.19 cores and peaked
at 6.4 GB, with the same update times as before for its size. The walking runs' cores are lower than the standing
ones' because their loop falls behind: it runs fewer updates a second.

### Where the time goes

**The steps.** The mean CPU per update of each step, with the whole world standing (the main series' 100% run). Each
step is also a Tracy zone of the same name (docs/PROFILING.md):

| | Eastern Kingdoms (30.6 ms) | Kalimdor (35.1 ms) |
|---|---|---|
| `creatures` (the creatures' turns) | 18.4 ms | 20.0 ms |
| `creatures' combat` (regeneration, leashes, watching, assistance) | 5.1 | 6.0 |
| `misc` (rest, cinematics, the environment) | 1.2 | 1.7 |
| `auras` | 1.2 | 1.6 |
| `values` | 0.9 | 1.2 |
| `despawns` | 0.9 | 1.2 |
| `visibility` (standing; walking: 4.1 and 4.5) | 0.6 | 0.6 |

The creatures' turns are 0.75 to 0.8 µs per awake creature per update (18.4 ms over 23,362 creatures, 20.0 ms over
27,436), and cost grows in proportion to the awake creatures, run after run. The update's fixed cost is the looks
through every creature, about 2.5 ms per continent: `despawns`, `values`, regeneration and `pets`, the same as with
no one on (above).

**The profile at 100%.** 2,660 samples over 30 s, standing, load average 15 to 17
(`runs/worldawake/prof-100-stand.{txt,svg}`). Inclusive figures for wvs's functions:

| | |
|---|---|
| `ai::update_creatures`, of which the turns (`turn`) | 57.5%, 51.8% |
| the walk: `step_creature_key`, `step_creature`, `wander_point`, `nav::random_point` | 21.1%, 13.8%, 4.5%, 3.3% |
| `combat::update_creatures` (whom creatures threaten) | 16.9% |
| `creature_combat::update` | 11.9% |
| `aggro_scan` | 11.7% |
| `map::awake`, the awake set's own re-check, self time | 11.4% |
| `assistance::update`, `auras::update`, `watch_for_enemies`, `maintain_leash` | 4.4%, 4.4%, 3.7%, 3.7% |
| `misc::update` (the environment's liquid look: `nav::liquid_surface`) | 3.9% (2.3%) |

Most of the cost is in the creatures' own work: they wander, pathing on the navmesh, look for enemies, keep their
leashes, and run their events. A tenth of it is bookkeeping: `members_where` re-checks every member of `Map::awake`
(23,000 to 27,000 of them) with `awake()`, and sorts their keys, once for each look that goes through it: the turns,
the threat, the assistance, the watch and the auras, five times an update.

**The beat every 500 ms.** At 100%, every ninth or tenth update costs twice the mean. The autocorrelation of an
update's CPU time peaks at a lag of 9 to 10 updates: 0.17 on Eastern Kingdoms, 0.38 on Kalimdor. Those updates take 60
to 83 ms of CPU, of which the turns are 42 to 60 ms and `misc` 6 to 10 ms. Three timers have a 500 ms period:

- `aggro_scan` sets `aggro_check_at = now + AGGRO_SCAN_EVERY` (500 ms);
- the environment checks every player every 500 ms (`environment::CHECK_EVERY`);
- `watch_for_enemies` has its own interval.

Creatures that came awake in the same update stay in step with one another, and so do the players. Most of the
updates over 50 ms are these beats. Between them, the CPU per update is 22 to 35 ms.

**Walking.** Standing, the update is on the CPU 74% to 100% of its wall time. Walking at 100%, it is on the CPU for
only 46% of it, and the update rate falls to 9 a second. The time off the CPU is spent waiting on the disk: the walking
runs have 33 to 66 major faults a second and 3 to 8 MB/s of reads, against 0 to 13 faults standing. An update over 1 s
shows 84 to 1,352 voluntary waits. A navmesh tile (`Nav::ensure`, rustigator's `ensure_adt_loaded_at`) or a terrain
tile is read from disk on the map's thread the first time anything needs it, and a walker reaches a new one now and
then. At 100%, a few hundred observers reach one in the same second. With 30 GB shared by several agents, the page
cache doesn't keep the 6.8 GB of the continents' navmesh, so the reads go to the disk. The same thing makes the logins
slow: a worldport's or teleport's acknowledgement took 200 to 450 ms while its first visibility loaded tiles. The
continents took up to 16 s to populate under load, and the watchdog called them hung meanwhile.

**Memory.** A tile, once loaded, stays loaded. The two continents' navmesh is 2.8 GB (Azeroth, 687 tiles) and 4.0 GB
(Kalimdor, 1,018) on disk, and the process holds 2.7 GB with 20 observers. Peak RSS was 6.4 GB with 300 observers, 8.9 GB
with 647 and 12.8 GB with 6,471: the tiles where the first few hundred observers stand are most of the world's.

### peon

peon has a new spread for this: `--spread world --points <file>` (branch `work/worldawake`).

- Bot slot `n` goes to point `n`, with `--placement command` (`.go xyz {x} {y} {z} {map}`).
- `--before-place ".gm on"` is said first, so the bots are in GM mode as in the benchmark.
- With the `wanderer` persona the bots walk at random within 60 yards of their point, at a constant z.
- `plans/baseline/world.toml` and a `world` case in `scripts/baselines.sh` (which needs `PEON_POINTS`) run it.

The runs went against a release `wow_vanilla_server` built from this branch after the merge with master (Tracy is in
the build and stayed off). Each run used a fresh `characters.sqlite` in `runs/worldawake/srv`, with one worker, a
60 s ramp and a 120 s hold. The server's CPU is from `/proc/<pid>/stat`, from 10 s into the hold to 15 s before its
end (95 s):

```sh
peon worker --orchestrator 127.0.0.1:7299 --clients 0 --target 127.0.0.1:3724 --profile wow-vanilla-server \
  --seed 1 --mode random --spread world --placement command --points docs/world_points.csv --before-place ".gm on"
# with: peon orchestrator --plan <a stage of N bots, ramp 60, hold 120, mix "wanderer=1"> --slots-per-worker 20000
```

| Bots | Alive | Placement failures | Server | peon | Server peak RSS | Updates over 50 ms | Login p50 / p99 | Movement relay p50 / p99 | Packets in/s | Load |
|---|---|---|---|---|---|---|---|---|---|---|
| 200 | 197 | 0 | 0.158 cores | 0.056 cores | 6.6 GB | 13 in the run | 7.9 / 32 ms | 0.0 / 4.0 ms | 336 | 5.1-5.7 |
| 350 | 343 | 0 | 0.240 cores | 0.084 cores | 7.9 GB | 30 in the run | 8.0 / 59 ms | 0.0 / 34 ms | 623 | 4.6-14.6 |
| 450 | | | | | stopped at 8 GB | | | | | |

The deaths (28 and 32) are the wanderers' own hunts. The slow updates were mostly during the ramp, while tiles
loaded: the slowest were 742 ms and 477 ms, with all but a few of the rest between 60 and 300 ms.

peon itself is not the limit: 0.084 cores for 350 bots means one worker could run thousands. The limit was memory.
This agent was capped at 10 GB, and the server needs about 8 GB for 400 bots spread this way, so the run stopped at
350. The benchmark's 100% run needed 13 GB, with peon's sockets and encryption on top of that.

### Does a continent keep 50 ms?

**Up to a quarter of the points, yes.** That is 670 to 948 players a continent and 9,000 to 10,000 awake creatures,
at p99 31 ms of CPU.

**At half the points, mostly.** The median is 19 to 23 ms of CPU and p95 is 48 to 52 ms; 11% to 16% of updates go
over 50 ms.

**With the whole world awake, no.** Eastern Kingdoms averages 30 ms of CPU an update and Kalimdor 38 ms; p95 is 65 to
77 ms. 9% to 11% of updates go over 50 ms in the quietest run, and 20% to 25% at a load average of 12 to 31. Each
continent takes about half a core: the work is single-threaded, and the CPU is there for it. Walking makes the misses
worse, because the disk reads happen inside the update.

**Margins.** Kalimdor at 100% is 3,909 players, more than a whole real realm, each one with a crowd of creatures in
view. Even so, the margin is thin:

- the mean of 30 to 38 ms leaves 12 to 20 ms;
- a creature costs 0.75 to 0.8 µs an update awake, so 27,000 of them are 20 ms before anything else;
- a busier machine, or more per-creature work (more scripts, more fights), takes the rest.

### The bottlenecks, and what each fix would cost

None of these is done here. Each one needs the update's debug checks kept, or extended.

1. **The 500 ms beat** (the spikes, and most updates over 50 ms). A creature's first aggro check, and a player's first
   environment check, could come at a random point of the 500 ms interval rather than all at once. The timers would
   then stay spread, and the beat's 60 to 83 ms updates would fall to about the mean.
   - Cost: hours.
   - Catch: it changes when a creature first looks, by up to 500 ms, so the tests that wait on a first aggro need
     to allow for that.
2. **Navmesh and terrain tiles read on the map's thread** (walking's waits, the logins' and populations' stalls).
   - The fix: load the tiles around a player before they need them, on a thread of the map's own. The map would take
     a loaded tile at the next update.
   - Cost: a few days. Rustigator's map is used through `&mut`, so a tile has to be built apart from it and handed
     over.
   - It would also stop the watchdog's false "hung" alarms during population.
3. **Memory: tiles never unloaded** (13 GB at 100%; it bounds a load test on a shared machine, and a server's host).
   - The fix: unload a tile that no player or awake creature has been near for some minutes. Rustigator has
     `unload_adt`.
   - Cost: a day or two. A creature on a patrol needs its tile, so "near" has to count the creatures.
4. **The awake set re-checked on every look** (11% of the update at 100%).
   - The fix: prune `Map::awake` once an update, into a sorted list of keys. The five looks would then filter that
     list, instead of each re-checking and sorting 27,000 members. The debug check against the look through every
     creature would stay.
   - Cost: a day. It saves about a tenth of the update.
5. **The creatures' own work** (75% of the update): wandering on the navmesh (21%), the threat look (17%), aggro scans
   (12%), leashes, events.
   - There is no single fix. Each part could be made cheaper: wander points picked without a navmesh query each time,
     and an aggro scan that skips creatures with no player within reach (one grid lookup for players only). Together
     these might take a third off.
   - Cost: days, part by part.
6. **One loop per continent.** With fixes 1, 2 and 4, a continent's update at 100% would be about 27 to 34 ms of CPU,
   evenly spread. It would keep 50 ms on a quiet machine, with a margin of a third. To go beyond that, the work has to
   go to more than one core:
   - **Regions of a continent with loops of their own:** for example by ADT blocks, each an actor. Every
     cross-region case needs a protocol: visibility, combat, a creature or player crossing, a spell's area. Cost:
     weeks. This is a design decision for the user.
   - **The creatures' turns in parallel within an update:** creatures far apart would be partitioned to threads,
     their turns reading the map and their changes applied after. Cost: weeks too, because a turn today mutates the
     map freely.

   Neither is needed at a real realm's 3,000 players, who are mostly in cities and instances.
