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

**Profiling.** `perf_event_paranoid` is 2 here, and neither `perf` nor `samply` will record, so the profiles below
were taken with a `pprof` cargo feature, a sampling profiler on SIGPROF, which needs no permissions: a flame graph and
the samples by function and by line, about 100 samples a CPU-second. The feature has since been removed, with
`pprof` itself; profiling is Tracy's now ([PROFILING.md](PROFILING.md)).

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

1. **An optional profiler** (the `pprof` feature, since removed), above.
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

Round two (at the end of this section) made four of the fixes this found: the 500 ms timers each on a phase of their
own, the awake set pruned once an update, navmesh and terrain tiles read off the map's thread, and idle tiles let go.
Since:

- **Standing, a fully awake continent keeps 50 ms.** Every point without the navmesh, the update's CPU at p99 is
  30 and 34 ms (the slowest update 35 and 38), from 44 and 42; all of Eastern Kingdoms with the navmesh, 32 and
  34 ms once its tiles are in, from 56 to 59. Kalimdor with the navmesh needs more memory than this round had.
- **The beat is gone**, and with it most updates over 50 ms. The median rises, by up to 3 ms at 100%: creatures now
  look for enemies every 500 ms, as the constant says, where on the beat they looked every 550 to 680 ms.
- **No read of a tile waits in the map**: logins, teleports and populating no longer stall. Taking a tile in and building its collision stay on the map's thread.
- **Walking at 100% nearly keeps it**: 0% to 2% of updates go over 50 ms of CPU, against 1% to 6% before, with p99
  at 42 to 55 ms.
- **Memory**: tiles no one is near are let go (`[world] tile_unload_secs`), so a walking crowd's memory stays level,
  but with every point awake every tile is near someone, and the whole world still takes about 13 GB.

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
continents took up to 16 s to populate under load, their players frozen meanwhile.

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

Before round two (below, with the answer since):

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

As this section first found them. Round two (below) made the first four fixes; 5 and 6 are open. Each one needs
the update's debug checks kept, or extended.

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

### Round two: the four fixes

Fixes 1 to 4 of the list above, one commit each, in this order: the timers, the awake set, the tiles read off the
map's thread, the idle tiles let go. Each was measured with `wvs-world-bench` before and after its commit.

**How this round measured.**

- **Without the navmesh, at every fraction.** This round's agent ran under a 10 GB memory cap (its cgroup's
  `memory.max`), and with the navmesh the benchmark passes 9 GB within 20 seconds of the logins at 10%, and needs
  13 GB at 100%. So the series at 10%, 25%, 50% and 100% ran with `nav_dir` unset (the terrain kept), at 0.5 to
  0.9 GB. The timers and the awake set don't depend on the navmesh, but the creatures' turns do: without it they
  walk straight, and a turn is cheaper. Standing at 100%, the median update is 22 to 26 ms of CPU without the
  navmesh, where round one's was 25 to 29 with it.
- **With the navmesh, at 3% and on one continent.** 194 observers (`--fraction 0.03`, 5.3 to 6.7 GB); and every
  point of Eastern Kingdoms with no one on Kalimdor (2,562 observers, the map-0 lines of `docs/world_points.csv`,
  6.4 to 6.9 GB),
  which is one continent fully awake. Kalimdor alone needs more than the cap allows.
- **In pairs.** The build before a commit and the build after it ran one after the other for each fraction and
  mode, so that both saw about the same machine. The load average was 7 to 39 over the round, and changed within
  minutes; each pair gives its own. At a load average over 30, a third to a half of an update's wall time is spent
  preempted, so the wall time's "over 50 ms" says more about the machine than the server there. The tables give the
  share over 50 ms of CPU as well, which the machine's load doesn't add to.
- **The scripts** are in `runs/perf2`: `bench.sh` (a run under a 9 GB RSS guard and the 5-minute cap), `ab.sh` (two
  builds alternated), `pair.py` (the tables below) and `beat.py` (the autocorrelation).

The tables give each continent's CPU an update at p50/p95/p99/max, before and after (in bold).

#### 1. The 500 ms timers, each on a phase of its own (`Map::next_beat`)

A creature's aggro check, a player's environment check and a civilian's watch for enemies (each second) set their
next time to the update's time plus their interval. An update takes every timer that came due since the last, and
sets each to the same next time, so timers only ever gather: once two fell in the same update, they stay together.
On a slow update more fall in, so the timers gathered on the slow updates, the beat that made every ninth or tenth
update cost twice the others.

Each now keeps to a phase of its own in its interval, from its guid (a Fibonacci hash of it, in milliseconds): its
next time is the first of its beats after now. Its period is still the interval. Its first check comes as before,
at once (a creature as it comes into view, a player as they arrive), and only the second may come sooner, by up to
the interval. mangos reacts to movement within its AI relocation delay (`Visibility.AIRelocationNotifyDelay`,
1000 ms), to a creature added or respawned within `CreatureRespawnAggroDelay` (5000 ms), and ticks its mirror
timers every 2000 ms, so a check up to 500 ms sooner is within what it allows.

The beat, as the autocorrelation of an update's CPU time at a lag of 10 updates, at 100% standing: +0.88 and +0.68
before, +0.26 and +0.13 after (a pair of profiled runs, load average 15). The series, one build after the other
(not alternated), load average 9 to 28:

| Observers | Walk | Eastern Kingdoms CPU p50/p95/p99/max ms | Kalimdor CPU p50/p95/p99/max ms | Over 50 ms, wall (EK, Kal) | Over 50 ms, CPU (EK, Kal) | Load |
|---|---|---|---|---|---|---|
| 647 | no | 6.5/12/14/15 → **5.7/6.6/7.1/7.6** | 7.4/12/14/16 → **6.2/7.2/7.7/8.3** | 1.1%, 1.7% → 0.0%, 0.0% | 0.0%, 0.0% → 0.0%, 0.0% | 20-25, 17-10 |
| 647 | yes | 5.8/8.7/10/47 → **5.6/7.1/7.7/9.2** | 6.4/9.1/11/42 → **6.0/7.6/8.5/10** | 0.1%, 0.3% → 0.0%, 0.0% | 0.0%, 0.0% → 0.0%, 0.0% | 25-26, 9-9 |
| 1,618 | no | 9.6/15/17/22 → **13/16/17/19** | 11/21/24/27 → **15/18/19/23** | 0.2%, 0.6% → 2.6%, 3.8% | 0.0%, 0.0% → 0.0%, 0.0% | 25-19, 9-19 |
| 1,618 | yes | 10/17/20/25 → **12/16/19/21** | 12/17/20/22 → **14/19/21/24** | 1.6%, 2.3% → 2.6%, 3.3% | 0.0%, 0.0% → 0.0%, 0.0% | 18-19, 19-22 |
| 3,236 | no | 16/39/42/48 → **20/25/27/33** | 20/33/36/42 → **23/28/31/35** | 4.9%, 4.6% → 8.2%, 13.3% | 0.0%, 0.0% → 0.0%, 0.0% | 21-22, 23-25 |
| 3,236 | yes | 16/29/33/51 → **19/27/32/92** | 19/30/34/38 → **22/30/35/70** | 2.9%, 4.2% → 12.1%, 16.8% | 0.1%, 0.0% → 0.2%, 0.1% | 22-21, 28-28 |
| 6,471 | no | 22/43/48/51 → **27/33/36/41** | 26/52/59/68 → **32/39/43/51** | 13.8%, 19.5% → 2.3%, 3.6% | 0.3%, 6.5% → 0.0%, 0.1% | 22-20, 28-27 |
| 6,471 | yes | 23/36/46/53 → **27/42/48/58** | 27/45/51/57 → **30/49/55/61** | 2.4%, 5.9% → 7.7%, 13.3% | 0.1%, 1.4% → 0.8%, 3.8% | 17-13, 26-26 |

At 100% the updates over 50 ms fall from 14% and 20% to 2% and 4%, and p99 from 48 and 59 ms to 36 and 43. The
median rises, by 5 or 6 ms here and 7 to 9 in the alternated pair below, and the profile says why: `aggro_scan`'s
samples rose by 35%. On the beat, an update long enough to gather the timers also delayed the next one, so creatures
looked every 550 to 680 ms; now they look every 500 ms, as `AGGRO_SCAN_EVERY` says. The same pair alternated at a
load average of 31 to 39:

| Observers | Walk | Eastern Kingdoms CPU p50/p95/p99/max ms | Kalimdor CPU p50/p95/p99/max ms | Over 50 ms, wall (EK, Kal) | Over 50 ms, CPU (EK, Kal) | Load |
|---|---|---|---|---|---|---|
| 3,236 | no | 16/37/40/43 → **18/20/21/27** | 19/40/45/50 → **20/24/25/28** | 13.6%, 23.6% → 0.0%, 0.0% | 0.0%, 0.1% → 0.0%, 0.0% | 37-34, 31-20 |
| 6,471 | no | 24/65/70/78 → **31/42/48/57** | 30/52/59/64 → **39/51/57/63** | 44.7%, 65.2% → 63.9%, 75.9% | 12.9%, 10.6% → 0.8%, 6.0% | 39-35, 34-33 |
| 6,471 | yes | 24/55/66/79 → **32/48/61/71** | 31/61/76/129 → **39/62/79/104** | 33.4%, 45.4% → 61.6%, 76.6% | 8.7%, 14.3% → 4.0%, 17.4% | 34-32, 33-38 |

Walking, the bench's observers send their heartbeats together every 250 ms, and the update after each batch works
out their visibility, a beat of 5 updates that is the bench's, not the server's (real clients aren't in step).

#### 2. The awake set pruned once an update (`Awake`)

`Map::awake` is now an `Awake`: the set, its keys sorted, and the guids added since. The update's first look lets go
of those no longer awake and sorts the keys; the four looks after it filter the same keys. What is added between
looks (a creature drawn into a fight by another's turn, a helper called, a summons) is kept aside and merged into the
keys at the next look, so every look still finds what a look through every creature would, in its order, which
debug builds still check at every look. A creature that comes back under a new key (its guid taken out of the map
and put back) is added again, whether it is awake then or later.

Alternated, load average 12 to 23:

| Observers | Walk | Eastern Kingdoms CPU p50/p95/p99/max ms | Kalimdor CPU p50/p95/p99/max ms | Over 50 ms, wall (EK, Kal) | Over 50 ms, CPU (EK, Kal) | Load |
|---|---|---|---|---|---|---|
| 647 | no | 6.1/7.4/8.0/9.1 → **5.8/6.9/7.6/8.5** | 6.6/8.2/9.1/9.7 → **6.3/7.5/8.3/9.5** | 0.0%, 0.0% → 0.0%, 0.1% | 0.0%, 0.0% → 0.0%, 0.0% | 14-17, 17-16 |
| 647 | yes | 5.6/7.2/8.0/9.4 → **5.6/7.0/7.8/8.5** | 6.0/7.5/8.3/11 → **6.0/7.5/8.4/10** | 0.0%, 0.0% → 0.0%, 0.0% | 0.0%, 0.0% → 0.0%, 0.0% | 15-12, 12-12 |
| 1,618 | no | 11/14/15/18 → **11/13/14/15** | 13/15/16/18 → **12/14/15/16** | 0.2%, 0.0% → 0.0%, 0.0% | 0.0%, 0.0% → 0.0%, 0.0% | 18-15, 15-17 |
| 1,618 | yes | 12/16/19/21 → **9.8/13/15/50** | 13/18/20/23 → **11/14/16/36** | 0.6%, 0.5% → 0.1%, 0.0% | 0.0%, 0.0% → 0.1%, 0.0% | 17-23, 21-15 |
| 3,236 | no | 18/21/22/24 → **16/19/21/25** | 21/24/25/28 → **19/22/24/28** | 0.0%, 0.2% → 0.0%, 0.3% | 0.0%, 0.0% → 0.0%, 0.0% | 14-15, 18-12 |
| 3,236 | yes | 17/24/27/29 → **16/23/26/30** | 19/27/29/33 → **18/27/30/33** | 0.8%, 0.8% → 1.5%, 2.1% | 0.0%, 0.0% → 0.0%, 0.0% | 12-15, 14-18 |
| 6,471 | no | 26/32/36/48 → **24/28/30/33** | 31/37/41/52 → **27/33/36/53** | 2.5%, 3.0% → 0.0%, 0.5% | 0.0%, 0.1% → 0.0%, 0.1% | 19-20, 19-18 |
| 6,471 | yes | 24/38/43/52 → **22/35/44/49** | 27/42/50/55 → **25/42/54/59** | 0.8%, 3.6% → 0.5%, 2.2% | 0.2%, 0.8% → 0.0%, 1.8% | 18-17, 18-14 |

At 100% standing, the creatures' combat step (the threat, the fights' aura states, the watch and the calls for help)
goes from 3.6 and 4.6 ms an update to 1.4 and 1.8, and p99 from 36 and 41 ms to 30 and 36.

#### 3. Tiles read off the map's thread (`map/tiles.rs`)

- **The readers.** Four threads read and decode the tiles (the navmesh's `.nav` with rustigator-pathfind's
  `NavFile::from_bytes`, the terrain's `.map`), and send them to the map, which takes them in at its next update
  (`PathfindMap::load_nav_file`), for at most 3 ms an update (at least one tile). The navmesh is now
  rustigator-pathfind's map, not rustigator's wrapper of it, which can't take a tile decoded elsewhere.
- **Ahead of the players.** As a player arrives, and each time their visibility is worked out again (every
  8 yards), the map asks for the tiles within what they see and 20 yards more. The creatures a player wakes are
  within that, and wander, chase and walk home about as far; one further off asks for its own tile as it goes.
- **No wait.** Until a tile is in, a query over it answers as a map with no navmesh would: a path is straight, a
  line of sight clear, heights and liquids unknown; a wanderer walks to a random point, and a feared or confused
  unit goes where it would. Once the tile is in, the next query uses it.
- **Tests** read tiles at once, as before (`RealmConfig::tiles_in_background`, off in `harness::test_config`), so
  what a test sees doesn't hang on the disk. A dungeon that is one WMO is still read whole as its copy starts.

The stalls, alternated, 194 observers with the navmesh:

| | Populated | Observers in place | Events over 200 ms, updates over 1 s | Peak RSS |
|---|---|---|---|---|
| Standing, before | 4.1 s | 14.0 s | 18 | 5.3 GB |
| Standing, after | 0.1 s | 0.1 s | 0 | 6.0 GB |
| Walking, before | 4.5 s | 20.1 s | 41 | 6.0 GB |
| Walking, after | 0.2 s | 0.2 s | 0 | 6.7 GB |

| Observers | Walk | Eastern Kingdoms CPU p50/p95/p99/max ms | Kalimdor CPU p50/p95/p99/max ms | Over 50 ms, wall (EK, Kal) | Over 50 ms, CPU (EK, Kal) | Load |
|---|---|---|---|---|---|---|
| 194 | no | 3.2/4.3/4.9/22 → **3.6/5.5/24/58** | 4.1/5.3/6.3/20 → **4.3/7.3/21/48** | 0.0%, 0.1% → 0.9%, 0.7% | 0.0%, 0.0% → 0.2%, 0.0% | 23-21, 21-29 |
| 194 | yes | 3.6/5.5/18/120 → **3.5/12/26/73** | 4.6/8.0/28/119 → **4.4/12/27/54** | 2.1%, 3.6% → 1.3%, 1.9% | 0.4%, 0.5% → 0.2%, 0.1% | 32-32, 32-38 |

Walking, the slowest update's wall time falls from 815 and 1,679 ms to 373 and 461. Two costs stay on the map's
thread, and show in the CPU's tail: taking a tile in (a few milliseconds for a busy one, more under load), and
building a tile's collision (its BVH, up to 27 ms) at the first query that needs it. Before, both came while the
observers were placed, before the measurement; now tiles arrive during it. The memory rises by 0.7 GB at 3%: the
tiles ahead of a player are read whether or not anything there asks. Without the navmesh (the terrain's tiles only)
there is no change past the noise.

A tile's costs, cold, from the real bake (a sample of 28 of Eastern Kingdoms' and 32 of Kalimdor's): reading it
83 ms on average and up to 250, decoding 3 ms (up to 12), taking it in 3 ms (up to 11), its BVH 5 ms (up to 27).

#### 4. Idle tiles let go (`[world] tile_unload_secs`)

Every 10 seconds the map marks the tiles around each player as used (a player standing still asks about nothing),
and lets go of the tiles, navmesh and terrain, that no one has used, asked about or stood near for
`tile_unload_secs` (300 by default, mangos's `GridCleanUpDelay`; 0 keeps them all; a reload changes it). An awake
creature far from every player keeps its tile by asking about it. A tile needed again is read again through the
readers of fix 3. Letting go of a busy tile takes milliseconds, so a sweep is spread over updates, 2 ms each.

A long walk shows it: 194 observers walking for 200 seconds with the navmesh, `tile_unload_secs` at 30:

| Build | Tiles in at the end (EK, Kal) | RSS at 40 / 80 / 120 / 160 / 200 s | Peak RSS | EK CPU p50/p95/p99/max ms | Kal CPU p50/p95/p99/max ms | Slowest update, wall (EK, Kal) | Load |
|---|---|---|---|---|---|---|---|
| Before (fix 3) | 239, 346 | 6.7 / 6.8 / 7.1 / 7.6 / 8.0 GB | 8.0 GB | 3.0/4.5/16/343 | 3.7/6.0/17/142 | 346, 164 ms | 21-18 |
| After | 156, 244 | 6.8 / 6.6 / 6.7 / 6.7 / 6.7 GB | 6.8 GB | 2.7/5.4/11/40 | 3.4/7.3/15/84 | 55, 106 ms | 20-7 |

At the default of 300 seconds no tile goes within the benchmark's minute, and the pair is as before:

| Observers | Walk | Eastern Kingdoms CPU p50/p95/p99/max ms | Kalimdor CPU p50/p95/p99/max ms | Over 50 ms, wall (EK, Kal) | Over 50 ms, CPU (EK, Kal) | Load |
|---|---|---|---|---|---|---|
| 194 | no | 3.1/4.7/19/44 → **3.1/4.5/25/70** | 3.7/5.8/14/59 → **3.7/9.1/24/37** | 0.0%, 0.2% → 0.6%, 0.2% | 0.0%, 0.2% → 0.3%, 0.0% | 11-16, 18-21 |
| 194 | yes | 3.2/5.1/12/56 → **3.0/5.1/15/84** | 3.9/7.2/19/68 → **3.7/7.1/23/193** | 0.1%, 0.2% → 0.2%, 0.4% | 0.1%, 0.2% → 0.2%, 0.4% | 20-20, 18-14 |

Dropping a tile whose pages were swapped out reads them back first: under this agent's cap, one sweep that let go of
dozens of tiles at once held an update for 2.2 s of wall time with 15 ms of CPU, which is why a sweep is now spread
over updates.

#### The start and the end

The build before round two and the build after it, alternated, without the navmesh:

| Observers | Walk | Eastern Kingdoms CPU p50/p95/p99/max ms | Kalimdor CPU p50/p95/p99/max ms | Over 50 ms, wall (EK, Kal) | Over 50 ms, CPU (EK, Kal) | Load |
|---|---|---|---|---|---|---|
| 647 | no | 5.5/11/13/15 → **6.3/7.9/8.5/10** | 6.1/12/13/15 → **6.8/8.6/9.6/11** | 0.1%, 0.0% → 0.3%, 0.3% | 0.0%, 0.0% → 0.0%, 0.0% | 17-19, 20-20 |
| 647 | yes | 5.5/8.5/9.9/11 → **6.3/8.5/9.7/12** | 6.1/9.4/11/13 → **6.8/9.2/10/12** | 0.0%, 0.0% → 1.2%, 1.2% | 0.0%, 0.0% → 0.0%, 0.0% | 20-17, 18-20 |
| 1,618 | no | 10/20/23/25 → **12/14/15/15** | 12/25/28/31 → **13/15/16/18** | 1.6%, 2.8% → 0.1%, 0.1% | 0.0%, 0.0% → 0.0%, 0.0% | 24-22, 22-19 |
| 1,618 | yes | 10/17/19/34 → **11/15/16/21** | 12/19/22/24 → **12/16/18/23** | 3.4%, 6.6% → 0.1%, 0.0% | 0.0%, 0.0% → 0.0%, 0.0% | 21-19, 19-16 |
| 3,236 | no | 14/30/33/34 → **18/23/26/31** | 17/26/28/29 → **20/27/31/37** | 0.0%, 0.0% → 5.6%, 7.2% | 0.0%, 0.0% → 0.0%, 0.0% | 13-10, 11-14 |
| 3,236 | yes | 19/34/39/44 → **19/28/33/38** | 22/37/45/54 → **21/31/37/50** | 33.3%, 35.7% → 21.1%, 20.0% | 0.0%, 0.2% → 0.0%, 0.1% | 16-21, 23-23 |
| 6,471 | no | 20/38/44/48 → **23/28/30/35** | 25/39/42/46 → **27/31/34/38** | 0.4%, 0.5% → 0.0%, 0.0% | 0.0%, 0.0% → 0.0%, 0.0% | 8-13, 12-15 |
| 6,471 | yes | 21/45/53/66 → **22/37/42/49** | 28/43/54/64 → **26/41/52/58** | 2.9%, 2.7% → 0.3%, 2.0% | 1.8%, 1.3% → 0.0%, 1.5% | 16-17, 15-13 |

At 100% the update rate goes from 18 a second to 19 or 20.

All of Eastern Kingdoms with the navmesh (2,562 observers, Kalimdor empty), alternated:

| Build | Walk | CPU p50/p95/p99/max ms | Over 50 ms, CPU | After the first 3 s: CPU | Over 50 ms, CPU | Over 50 ms, wall | Updates/s | Populated, in place | Peak RSS | Load |
|---|---|---|---|---|---|---|---|---|---|---|
| Before, first pair | no | 21/56/59/65 | 8.9% | 21/55/59/65 | 8.8% | 9.0% | 19.7 | 2.5 s, 13.5 s | 6.4 GB | 10-11 |
| After, first pair | no | 24/35/60/99 | 1.7% | 24/28/32/51 | 0.1% | 1.8% | 19.9 | 0.1 s, 1.6 s | 6.7 GB | 13-10 |
| After, second pair | no | 25/36/59/123 | 1.7% | 25/30/34/73 | 0.2% | 2.9% | 19.7 | 0.1 s, 1.8 s | 6.6 GB | 9-9 |
| Before, second pair | no | 20/54/56/61 | 9.8% | 20/54/56/61 | 9.6% | 9.8% | 19.8 | 3.3 s, 27.5 s | 6.4 GB | 10-9 |
| Before, first pair | yes | 22/49/60/201 | 4.4% | 22/50/60/201 | 4.4% | 5.2% | 19.2 | 1.1 s, 6.5 s | 6.8 GB | 10-10 |
| After, first pair | yes | 31/54/68/102 | 8.5% | 31/51/64/102 | 5.8% | 28.5% | 16.9 | 0.1 s, 2.0 s | 6.7 GB | 12-19 |
| After, second pair | yes | 22/46/60/116 | 2.8% | 22/44/55/116 | 1.7% | 2.8% | 19.6 | 0.1 s, 1.1 s | 7.2 GB | 12-8 |
| Before, second pair | yes | 21/51/61/124 | 5.6% | 21/51/63/124 | 5.7% | 6.3% | 19.1 | 0.9 s, 7.0 s | 6.7 GB | 7-8 |

The first pair ran with the round-two build before its sweep was spread over updates; no tile goes within these runs,
so that makes no difference. In the first pair's walking run the load average rose from 12 to 19 as it ran, and the
second pair, on a quieter machine, has the same build at 22/44/55/116. In the first 3 s of a window the last tiles
were still arriving: taking them in and building their BVHs made updates of 60 to 120 ms of CPU.

#### Does a fully awake continent keep 50 ms now?

**Standing, yes.** With every point awake and no navmesh, on a quiet machine (load average 8 to 15), p99 is 30 and
34 ms of CPU, the slowest update 35 and 38, and none goes over 50 ms of wall time; round one's build had p99 44 and
42. All of Eastern Kingdoms with the navmesh, once its tiles are in: p99 32 and 34 ms, 0.1% and 0.2% of updates over
50 ms of CPU, where round one's build had p99 56 to 59 and 9% to 10% over, the beat. The median is 24 to 25 ms, which
leaves a margin of half the budget at the median and a third at p99.

**Walking, nearly.** Without the navmesh, walking at 100%: p99 42 and 52 ms, the slowest update 49 and 58, 0% and
1.5% of updates over 50 ms of CPU (1.8% and 1.3% before). All of Eastern Kingdoms with the navmesh, walking, on the
quieter pair: p99 55 ms and 1.7% over 50 ms of CPU, against 63 ms and 5.7%. What is left in the tail is the walkers'
own work (visibility worked out for thousands of them in step, the bench's) and tiles arriving.

**Kalimdor with the navmesh** couldn't be measured whole here: it needs more than the 10 GB cap. Without the navmesh
its update is 3 to 4 ms dearer than Eastern Kingdoms', and in round one it was 4 to 6 ms dearer with it; by those, it
would be at about 37 to 40 ms at p99 standing, inside 50 ms, but that is an estimate.

#### What is left

- **Taking a tile in, and its BVH, on the map's thread.** Up to tens of milliseconds a tile, while tiles arrive. The
  BVH could be built on the readers' threads (rustigator's `warm_cell` takes `&self`, and the map is `Sync`), with the
  map shared behind an `Arc` and a tile counted in only once it is warm.
- **Memory with every point awake.** Every tile is near someone, so none goes, and the whole world is about 13 GB.
  A cap on the tiles in would bound it, at the cost of reading tiles again and creatures walking straight meanwhile.
- **The aggro scans' rate.** 500 ms exactly costs 2 to 4 ms an update at 100% over the gathered beat's 550 to
  680 ms. A longer interval (mangos's is event-driven, within 1000 ms) or a scan that skips creatures with no
  player within reach would take it back.
- **Walking at 100%**, which the creatures' own work and visibility bound (fixes 5 and 6 above).
- **Kalimdor, and the whole world, with the navmesh**: to be measured where 13 GB are free.

## Packet order: the hold (DECISIONS 11)

A client the map has queued creates for is held (`Outbound::hold`): what it is sent waits in a queue of its own
until the update's "held" step, which sends its creates and forgets and then what waited. What it costs:

- **Every send** reads one atomic flag. A held client's frames take a lock and go into its queue, and are moved
  to the writer's channel once, at the release.
- **Every introduction** (`visibility::introduce`) holds its observer: an atomic swap, and the observer noted once
  an update.
- **Every object added** (a creature, game object, corpse or dynamic object) looks up the players near it in the
  grid, when the map has any.
- **The "held" step** goes through the clients held. One that nothing waited for keeps its blocks for the one
  batch, so the packets per update are as before. One that had frames waiting gets its creates in a packet of
  their own, and its values in the update's batch, after.
- **After each event** (`Map::after_events`), with any client held: what the event brought into the map is seen
  at once, and every held client gets its creates and forgets, then what waited. The exception is a client near
  a creature whose first turn (its spawned events) is still to come: it is held to the update, which sees the
  creature after that turn.
- **Queries' answers** (name, creature, item) and the pong name nothing a create could be waiting for, and go
  past a hold (`Outbound::send_unheld`).

The first version held a client from a packet's handling to the next update. Under peon that cost latency, not
CPU: chat and name queries went from 1 ms at p99 to 47 to 91 ms, and movement relays from 2 ms to 6 to 10, in
pvp-arena and cluster, and pvp-arena's bots killed a fifth fewer of each other (310 and 321 deaths, against 385).
Letting the clients go at the end of a burst of events took most of that back; name queries near a fresh summons
(56 ms at p99) and the rest of the burst's wait (movement 1 ms more at p50) went with the last two points.

**peon**, the branch's start and its end alternated run after run, the same bots (slots 6000 and 6200), as
`runs/perf-srv/run.sh` runs them (a 60 s ramp, a 120 s hold, the server's CPU over 105 s of it), with the navmesh,
load average 2 to 6. The latencies are peon's, in ms:

| Run | Cores | Login p50/p99 | Movement p50/p99 | Chat p50/p99 | Name query p50/p99 | Deaths | Packets in/s |
|---|---|---|---|---|---|---|---|
| pvp-arena, end | 0.163 | 8.6 / 51.7 | 0.0 / 3.0 | 0.0 / 1.0 | 0.5 / 2.3 | 391 | 10,696 |
| pvp-arena, start | 0.162 | 8.3 / 20.7 | 0.0 / 4.0 | 0.0 / 2.0 | 0.5 / 1.3 | 369 | 10,183 |
| pvp-arena, start | 0.172 | 8.1 / 20.2 | 0.0 / 3.0 | 0.0 / 1.0 | 0.5 / 1.0 | 434 | 11,741 |
| pvp-arena, end | 0.161 | 8.6 / 19.7 | 1.0 / 3.0 | 1.0 / 1.0 | 0.5 / 2.0 | 392 | 10,915 |
| cluster, start | 0.330 | 8.1 / 20.7 | 0.0 / 2.0 | 0.0 / 1.0 | 0.4 / 2.5 | 0 | 28,225 |
| cluster, end | 0.334 | 8.3 / 19.7 | 0.0 / 2.0 | 0.0 / 2.0 | 0.5 / 1.6 | 0 | 28,209 |
| cluster, end | 0.331 | 8.6 / 20.2 | 0.0 / 2.0 | 0.0 / 1.0 | 0.5 / 2.2 | 0 | 28,271 |
| cluster, start | 0.331 | 9.1 / 21.2 | 0.0 / 3.0 | 0.0 / 1.0 | 0.5 / 1.1 | 0 | 28,151 |

The CPU, the packets and the latencies are the same within the runs' spread. peon's latencies are whole
milliseconds, so a median of 0.0 or 1.0 is a step of the histogram. The login's 51.7 ms p99 is one run; the other
end run has 19.7, and a login is auth, the character's creation and the world's handshake, none of which is held.

`wvs-world-bench`, without the navmesh, the branch's start (`26032a1`) and its end (`f5f59ed`) alternated run
after run (the 100% pairs twice, the second in the other order), load average 1 to 4:

| Observers | Walk | Eastern Kingdoms CPU p50/p95/p99/max ms | Kalimdor CPU p50/p95/p99/max ms | Over 50 ms, CPU (EK, Kal) | Load |
|---|---|---|---|---|---|
| 1,618 | no | 10/12/13/13 → **10/12/13/15** | 11/13/14/16 → **11/13/15/16** | 0.0%, 0.0% → 0.0%, 0.0% | 3-3, 4-3 |
| 1,618 | yes | 9.3/12/14/16 → **9.3/13/14/16** | 10/13/15/17 → **10/13/16/18** | 0.0%, 0.0% → 0.0%, 0.0% | 3-2, 2-1 |
| 6,471 | no | 22/26/27/27 → **22/27/28/30** | 25/31/33/34 → **25/32/34/36** | 0.0%, 0.0% → 0.0%, 0.0% | 2-1, 2-2 |
| 6,471 | no | 22/26/28/30 → **22/26/27/28** | 25/29/33/34 → **25/30/34/38** | 0.0%, 0.0% → 0.0%, 0.0% | 2-2, 2-2 |
| 6,471 | yes | 21/32/37/46 → **21/32/39/49** | 24/38/44/52 → **23/39/44/52** | 0.0%, 0.2% → 0.0%, 0.1% | 2-2, 2-3 |
| 6,471 | yes | 21/31/38/49 → **21/32/39/48** | 24/37/43/55 → **24/38/44/53** | 0.0%, 0.3% → 0.0%, 0.5% | 2-2, 2-2 |

The difference is inside the noise between pairs, but for one step. The frames a second are the same (38,299 and
38,294 before, 38,208 and 38,290 after, walking at 100%), so the split packets are rare. The visibility step,
where the introductions hold their observers, costs 0.12 to 0.15 ms more an update walking at 100% (2.15 and 2.16 ms
on Eastern Kingdoms before, 2.27 and 2.31 after; 2.76 and 2.77 on Kalimdor, 2.88 and 2.89), and the held step
0.01 ms. Standing, nothing changes past 0.02 ms. The cores are 1.16 before and 1.17 after, walking at 100%.

## The concurrency micro-benchmarks

`benches/sync.rs` settles "which channel" or "which lock" in five minutes, not in hours of load tests. It is
[divan](https://crates.io/crates/divan)'s (a dev-dependency, as are kanal, flume, crossbeam-channel and arc-swap;
wvs's own channels are all tokio's, DECISIONS 67). Each benchmark stands for a path in wvs, named in its doc comment,
and runs that path's shape (who sends, from where, who receives and how) with each candidate. What the paths do
besides is left out (the cipher and the socket, SQLite, the disk, a map's handling of an event), so a row is the
channel's or the lock's cost, and the harness's, which is the same for every candidate.

```sh
cargo bench --bench sync                     # everything: 76 cases, about five minutes
cargo bench --bench sync -- channels         # a group: channels, locks or live_config
cargo bench --bench sync -- channels::store  # one benchmark
cargo bench --bench sync -- Tokio            # the cases with Tokio in their path (a filter is a regular expression)
cargo bench --bench sync -- --max-time 0.2   # a quick look: each case 0.2 s
cargo bench --bench sync -- --list           # the cases, without running them
```

- **Each case runs 4 s** (`SECS`), whatever it costs, so the whole run takes the same time on any machine.
- **The first build compiles wvs in release** (the bench links the server's library, and the bench profile is
  release's: thin LTO, one codegen unit), about three and a half minutes on 4 jobs. `build.rs` names no file to be
  rerun on, so cargo reruns it on any change in the package, the bench's own file or a doc's, and builds the library
  again.
- **Run it on a quiet machine, twice.** Other work moves the numbers more than some candidates differ.

| Benchmark | wvs's path | The shape | Candidates, wvs's first |
|---|---|---|---|
| `channels::outbound` | `conn.rs`: `Outbound`, `run_writer` | 1, 4 or 16 producer threads send `Arc<[u8]>` frames (36 to 384 bytes) with a sync send; one writer task waits for a frame, takes up to 128 and copies them out. A round: 16,384 frames | `TokioRecvMany` (`recv_many`); `Kanal` (wvs's before), `Tokio` (drained as kanal was), `Flume` |
| `channels::mailbox` | `map/mod.rs`: `MapHandle`, `map::run` | 1 or 64 sender tasks send the real `MapEvent` (a movement packet) with a sync send; the map's task receives in `select!` beside its 50 ms ticker, then drains up to 512. A round: 4,096 events | `Tokio`; `Flume` in `select!`; `TokioNoSelect` and `KanalNoSelect` in a loop of their own (kanal's receive isn't safe in `select!`: a message can be lost) |
| `channels::tiles` | `map/tiles.rs`: `tiles::read` | one sender, 4 reader threads asleep in a blocking receive (a round waits 1 ms first, untimed). A round: 8 jobs | `Tokio`, whose one receiver the readers share behind parking_lot's mutex; `Kanal` (wvs's before), `Crossbeam`, `Flume`; `Std`, behind std's |
| `channels::store` | `store.rs`: `Store`, `serve_all` | 1 caller task, or 16, sends a request with a sync send and awaits the reply on tokio's `oneshot`; the store's thread waits in a blocking receive, takes up to 512 and answers each. A round: 256 round trips | `Tokio` (`blocking_recv`); `Std` (wvs's before), `Kanal`, `Flume`, `Crossbeam` |
| `locks::registry` | `registry.rs`: `Registry::online` | 1,000 players in a `RwLock<AHashMap<Guid, Arc<Online>>>`, looked up (the `Arc` cloned and dropped) from 1, 4, 16 or 32 threads; every 1,000th operation on a thread a write (a login) | `ParkingLot`; `Std`, `StdRecovering` (a poisoned lock taken anyway, as wvs would need) |
| `locks::social` | `social/mod.rs`: a service's `Mutex` | the friends lists of 1,000 players, a friend added or removed, from 1, 4 or 16 threads | `ParkingLot`; `Std`, `StdRecovering` |
| `live_config::*` | `game/mod.rs`: `Realm::config` | the real `RealmConfig`, read from 1, 4, 16 or 32 threads | `generation` (a map's look, once an update), `parking_lot_read_clone` (`Realm::config`); `std_read_clone`, `arc_swap_load` (`Realm::config` before), `arc_swap_load_full` |

crossbeam-channel and std's `mpsc` have no async receive, so they are candidates only where the receiver is a plain
thread: the tiles and the store.

### Reading divan's output

```text
sync                          fastest       │ slowest       │ median        │ mean          │ samples │ iters
├─ channels                                 │               │               │               │         │
│  ├─ outbound                              │               │               │               │         │
│  │  ├─ Kanal                              │               │               │               │         │
│  │  │  ├─ 1                 …             │ …             │ …             │ …             │ …       │ …
│  │  │  │                    … Mitem/s     │ … Mitem/s     │ … Mitem/s     │ … Mitem/s     │         │
…
╰─ locks                                    │               │               │               │         │
   ├─ registry                              │               │               │               │         │
   │  ├─ ParkingLot                         │               │               │               │         │
   │  │  ├─ t=1               …             │ …             │ …             │ …             │ …       │ …
```

- **The tree is the path**: the group, the benchmark, the candidate, then the producers, senders or callers (a
  number), or the threads (`t=N`).
- **The columns are over the samples**: the fastest, the slowest, the median and the mean of an iteration's time.
  Read the median. A busy machine pulls the mean and the slowest up, and the fastest is luck. `samples` and `iters`
  are how many there were.
- **A channel's iteration is a round**: it sends the round's items (16,384 frames, 4,096 events, 8 jobs or 256 round
  trips) and ends when the consumer has taken them all. Its time is the round's; the line under it (`item/s`) is the
  items through in a second. An item's cost is the round's time over its items: 16,384 frames in 1 ms are 61 ns a
  frame. A round has the same items whatever the producers, so a benchmark's rows compare, down a column.
- **A thread's iteration (`t=N`) is one operation**, as one of N threads sees it while the others run theirs (4,096
  operations a sample on each thread). N over it is what the N threads do together in a second; a time that grows
  with N is contention. 32 threads on 16 cores are two to a core, and some of that row is the scheduler's.
- **`Std` and `StdRecovering` differ only on a poisoned lock**, which there never is here: the gap between their rows
  is the noise.

### Channel chat's fan-out (`benches/chat.rs`)

A line said in a channel goes to every member, from the speaker's map task (`channels::say`). Before 2026-10-20,
`say` held `realm.social.channels`, one mutex over every channel in the realm, and the ignore lists' mutex, both
across the whole fan-out: a registry read lock and an `Arc` clone for each member, then its queue's send. All channel
chat in the realm took turns on those two locks, and a map waited for any other map's fan-out before starting its
own.

`benches/chat.rs` runs that path whole: a real `Realm` (an in-memory store, no game data), its registry, its
channels, each member's real `Outbound` drained by a writer task of its own (`run_writer` without the cipher and the
socket, as in `channels::outbound`), and the speakers as tasks on a tokio runtime, as maps are: 15 workers, the
sixteenth core left to the benchmark's thread, which spins on the round's end. A round is 40,000 deliveries whatever
the case, so a group's rows compare, down a column; the members are 100, 1,000 or 5,000, with 1 map saying lines, 8
maps saying lines in the same channel, or 8 maps each in a channel of its own (8 × 5,000 is 40,000 players online).

```sh
cargo bench --bench chat                        # 27 cases, about two minutes
cargo bench --bench chat -- said                # a group: delivered, said or pushed
WVS_BENCH_WORKERS=4 cargo bench --bench chat    # another count of tokio workers
```

- **`delivered`**: a round ends when the writers have taken every delivery. Deliveries a second, end to end.
- **`said`**: a round ends when the maps are done saying, the writers awake and working meanwhile; what they have left
  is drained before the next round, untimed. What a map hands out a second while its update isn't running.
- **`pushed`**: `said` with no writer at all, the queues emptied between rounds. The fan-out's own cost (the locks,
  the registry, each queue's budget and push), without waking anyone.

**Before and after** (2026-10-20; x86-64 server CPU, 16 cores; the old and new code's benchmark binaries alternately,
two runs each, each row the mean of the two runs' medians, in millions of deliveries a second). The machine was
shared: other agents (about seven, each mostly idle), no other build at the runs' starts, load averages 3.9 to 9.5
with the benchmark's own 16 threads in them. Busier, the `delivered` and `said` medians of one map move from about 2
to 16 within one binary; `pushed` moves by a tenth.

| Case | `delivered` before | after | `said` before | after | `pushed` before | after |
|---|---:|---:|---:|---:|---:|---:|
| 1 map, 100 members | 2.0 | 1.7 | 2.0 | 1.7 | 19.1 | 22.4 |
| 1 map, 1,000 members | 2.2 | 2.0 | 3.7 | 3.9 | 22.2 | 27.2 |
| 1 map, 5,000 members | 2.3 | 2.0 | 4.0 | 4.1 | 8.5 | 17.1 |
| 8 maps, one channel of 100 | 1.9 | 7.5 | 2.1 | 7.6 | 12.8 | 10.8 |
| 8 maps, one channel of 1,000 | 1.8 | 7.1 | 1.8 | 7.0 | 12.2 | 13.5 |
| 8 maps, one channel of 5,000 | 2.0 | 6.9 | 2.6 | 6.9 | 7.8 | 10.3 |
| 8 maps, 8 channels of 100 | 1.8 | 19.8 | 2.0 | 19.3 | 13.3 | 73.4 |
| 8 maps, 8 channels of 1,000 | 1.9 | 16.2 | 2.0 | 15.8 | 8.9 | 57.8 |
| 8 maps, 8 channels of 5,000 | 2.4 | 12.7 | 2.5 | 14.7 | 8.2 | 29.2 |

One map's rows with its writers awake are within the noise: run three times more each, alternately, `delivered` and
`said` gave 2.8 to 16 for both binaries, and `pushed` 23.0, 22.4 and 19.4 before against 28.9, 27.9 and 24.6 after
(100, 1,000 and 5,000 members).

**What changed** (`social/channels.rs`, `social/friends.rs`, `social/mod.rs`, `registry.rs`):

- **A lock a channel.** The directory (`Channels`) is a `RwLock` over `Arc<Mutex<Channel>>`, taken only to find a
  channel (a read), or to put one in or take one out (a write), and never while waiting for a channel's lock. A
  channel left empty is marked dropped under its own lock, then taken out; one who found it just before finds no
  channel, and a join makes a new one. A new channel is locked before it goes in, so no one finds it empty. Joins,
  leaves and commands lock their channel alone; their notices are still sent under it, in order with its changes.
- **The fan-out after the lock.** Under the channel's lock `say` does its checks and copies the members' GUIDs; then
  it encodes the line, asks the ignore lists once for those who ignore the speaker (`Lists::ignorers`, the lists the
  other way round, kept with them), and sends with no service's lock held (`social::send_frames`): the registry read
  once for 256 recipients (`Registry::each_online`), not once each.
- **Guild and group chat** (`guilds::chat`, `groups::chat`) the same way: the hearers copied under their lock, the
  ignore lists asked once, the fan-out after. Guild chat looked each member's rights up again by GUID, a scan of the
  members for each, N² for a guild of N; it reads each member's own rank now. `/who` stops at a full list (49): the
  rest it only looked at, taking the guilds' lock for each, to leave out. It holds no lock across its answer.

**What limits it now:**

- **The writers.** One map hands out 22 to 28 million deliveries a second when no writer is awake (35 to 45 ns a
  delivery: the registry, the budget's semaphore, the `mpsc` push), and 2 to 4 million when every recipient's writer
  is asleep on its queue (250 to 500 ns). The code is the same in both; the difference, 85 to 90 percent of a map's
  fan-out, is the writers being awake: tokio waking each one (its task's state, a run queue, and with workers idle,
  one unparked), and each taking frames off its queue while the map pushes more. Not profiled further (this machine
  allows no `perf` for its users). The server's writers also encrypt and make a write call a batch, which the
  benchmark leaves out. Neither lock was ever most of it for one map.
- **One channel's recipients.** Eight maps in one channel hand out 7 million a second together, under 1 each, and
  10 to 13 million with no writer awake, less than one map alone: every line goes to the same members' queues, so the
  maps queue on each recipient's semaphore, `mpsc` tail and waker instead of the channel's lock. Lines in one
  channel don't scale with maps; lines in different channels do (13 to 20 million awake, 29 to 73 million pushed).
- **Not the registry.** Its read lock is taken once for 256 recipients, and two maps' reads don't wait for each other.
- **The order of lines.** No longer under one lock, two maps' lines in the same channel can reach two members in
  different orders, and a line can arrive after the notice of a leave or a kick that came after it was said. One
  map's lines arrive in the order it said them.

## The movement drill (2026-10-21)

peon's `--mode drill` (`scripts/drill.sh` in peon; its results and the full table in peon's `results/2026-10-21/`):
N Alliance bots, level 30, geared, undying (`.god on`, a load test's god mode: blows land, health stays at 1 or more),
spread over a 60-yard disc of open sand in Tanaris's Waterspring Field, run north 10 s, back 10 s, then about at
random 120 s with jumps, side-steps and back-steps. The server on cores 0-15, on loopback (`[network] auth_address`,
`world_address`), with `WVS_STATS_FILE` (a JSON line a second: each update's time and steps, events, queues, bytes
out, disconnects, CPU, RSS; [PROFILING.md](PROFILING.md)); peon on 16-31, never past 46% of them. Kalimdor, at random:

| Bots | Update p50 / p99 / max ms | Broadcasts got / due | Latency p50 / p99 ms | Packets out a second | CPU | Map thread busy |
|---|---|---|---|---|---|---|
| 50 | 2.0 / 3.4 / 66 | 100.7% | 0 / 2 | 11,269 | 0.20 | 5% |
| 200 | 2.8 / 5.5 / 6.4 | 100.8% | 0 / 3 | 169,933 | 1.33 | 10% |
| 880 | 18.8 / 32.0 / 85 | 100.8% | 4 / 32 | 3.2 million | 9.14 | 78% |
| 1,000 | 23.6 / 39.4 / 115 | 100.8% | 11 / 49 | 4.1 million | 8.88 | 91% |
| 1,100 | 38.9 / 68.5 / 164 | 100.8% | 27 / 118 | 5.0 million | 8.85 | 99% |
| 1,200 | 56.6 / 297 / 482 | 96.4% | 4,391 / 6,357 | 5.7 million | 9.75 | 100% |
| 1,500 | 74.0 / 349 / 566 | 55.3% | 35,127 / 59,245 | 5.2 million | 9.32 | 100% |
| 2,000 | 109 / 891 / 1,191 | 27.3% | (over 10 s) | 4.5 million | 8.85 | 101% |

The knee is about 1,000 bots in one spot (the runs out and back, the crowd in step, break first: p99 237 ms at 1,000).
Kalimdor's one thread gives out, on two costs that grow with the square of the crowd: `visibility` (three quarters
of the update at 880; 1.22 ms a refresh of a player among 2,000, about 0.6 µs a player in sight, in the Tracy capture
at 2,000) and the movement handlers' fan-out to every observer (about 110 ns an observer a packet). Past it the map's
mailbox grows without bound (825,000 events at 2,000) and broadcasts fall a minute behind; at about 5 million packets a
second the outbound path (about 2 µs of CPU a packet) fills clients' queues to their budget. RSS stayed at 2.7 to
3.5 GB. The fixes, most valuable first: visibility refreshes budgeted per update and made cheaper (one pass a pair,
known pairs well inside sight skipped); the fan-out batched per observer or moved off the map's thread; back-pressure
on the mailbox (a mover's queued heartbeats coalesced); the writer's cost a packet; a crowd's creates and the login's
GM commands at arrival (placement held at about 45 a second).

## Visibility in a crowd (2026-10-22)

The drill's first cost, visibility's refresh, made cheap (`map/visibility.rs`, `map/grid.rs`). Before, each refresh
of a player looked at every player in its 7 by 7 cells both ways (do they see it; does it see them), with
`death::can_see` and several hash lookups a pair, then went through everything the player knew and everyone who knew
it, and re-ran the zone, the area conditions and the tiles ahead: about 0.1 µs a pair, and a crowd moving in step
refreshes all at once, so the square of the crowd in one update (92 ms at 880 on one core, 2.3 s at 3,000).

### What changed

1. **The quick path: what a move can't change is skipped.** An object is *plain* when who sees it, and whom it sees,
   turns on distance alone (`visibility::plain_now`: not stealthed or invisible, not a ghost, not concealed, not a
   spirit healer, a trap or a transport, not on a flight, not looking from elsewhere, not between maps); between two
   plain objects `can_see` is always true. Each grid entry now carries its object's *anchor* (where its visibility
   was last worked out) and whether its own last refresh found it plain; `visibility::moved` takes the flag off once
   the object strays more than `DRIFT` (16 yards, twice `MOVE_THRESHOLD`) from its anchor or stops being plain, and
   anything marked to be worked out whole (`visibility_dirty.insert`: an arrival, a teleport, a change of state) is
   taken as not plain by the others until its own refresh. The invariant (K, in the module's doc): of two plain
   objects whose anchors are within `radius - DRIFT` of each other, the player knows the other. So a plain object
   that moved `delta` skips, without a lookup, every plain object whose anchor is within `radius - DRIFT - delta` of
   it (75 yards at 100, as the brief's "sight less twice the threshold", with the drift's 16 yards spent on the
   anchors), counting them as known; and two plain objects in the ring beyond are decided without `can_see`.
2. **One decision a pair.** A chunk's refreshes are numbered in the grid; a pair two of them share is decided by the
   earlier, both ways.
3. **Whole cells.** A cell with nothing in it not plain, its farthest corner within `radius - 2 DRIFT - delta`, is
   skipped without a look at its objects (its counts kept per cell); one wholly past the margin likewise (anything
   known there shows in the counts, and only then are what the player knows, and who knows it, gone through).
4. **Side work at a cell crossed.** The zone (`world::update_zone`, with exploration, the area's auras and the
   free-for-all flag), the creatures' area for conditions and the tiles ahead are done when the object has left the
   50-yard cell they were last done in (`side_cell`), or its refresh is a whole one; not every 8 yards.
5. **Decisions across threads.** An update's refreshes go in chunks of 128: the side work on the map's thread, then
   the decisions, read only (`&Map` is `Sync`), on a rayon pool of `[visibility] refresh_threads` (0: the machine's
   cores, up to 8; chunks under 16 stay on the map's thread), then applied in order on the map's thread, each
   checked again as it is applied. The serial and the parallel give the same creates in the same packets and the
   same forgets, update after update (`tests_visibility::decisions_across_threads_are_the_ones_on_the_maps`).
6. **A budget an update.** `visibility_dirty` keeps its marks oldest first; past `[visibility] refresh_budget_ms`
   (10 ms) an update stops between chunks and leaves the rest, oldest first, for the next (at least a chunk an
   update). The stats file's `visibility_waiting` is the most an update of the second left.

Also: each grid entry is found by its slot, not by a scan of its cell (a crowd's cell has hundreds), and two triggers
were missing: `.gm on`/`off` changed what a game master sees only at their next steps, and a release to a ghost in
the same map only at the graveyard's teleport. Both are marked now.

What a client sees is as before, but for the side work's timing: a zone or area change (its world states, channels,
exploration, area auras) is noticed once the player crosses a cell, up to about 50 yards late, not within 8. Over the
budget, a refresh waits for a later update.

### Measured

`benches/visibility.rs` (`cargo bench --bench visibility`): a crowd on a disc 60 yards across, all in sight of each
other (the drill's), and on one 300 across (`wide`: each sees about half the others, many pairs at the edge of sight);
`in_step` refreshes the whole crowd after a 9-yard step, `random` one in 23 (an update at random), `one` one player.
`before` is each refresh alone and whole, as before. On 4 cores of a 32-core box (medians):

| Case | 100 | 880 | 2,000 | 3,000 |
|---|---|---|---|---|
| `in_step`, before | 0.77 ms | 89 ms | 970 ms | 2.54 s |
| `in_step`, quick, the map's thread | 0.042 ms | 0.89 ms | 3.7 ms | 7.9 ms |
| `in_step`, quick, 4 threads | 0.042 ms | 0.50 ms | 1.5 ms | 2.8 ms |
| `random`, before | 31 µs | 3.8 ms | 50 ms | 113 ms |
| `random`, quick, 4 threads | 3 µs | 36 µs | 109 µs | 179 µs |
| `one`, before | 8.4 µs | 144 µs | 629 µs | 1.0 ms |
| `one`, quick | 1.2 µs | 5.6 µs | 16 µs | 24 µs |
| `wide` in step, before | 0.50 ms | 41 ms | 276 ms | 801 ms |
| `wide` in step, quick, the map's thread | 0.11 ms | 8.4 ms | 62 ms | 146 ms |
| `wide` in step, quick, 4 threads | 0.08 ms | 2.5 ms | 15 ms | 35 ms |

By step, on the map's thread: whole cells halve `in_step` (1.65 to 0.92 ms at 880, 7.5 to 3.8 ms at 2,000); the
cells past the margin take a quarter off `wide` (85 to 62 ms at 2,000); one decision a pair is within the noise in
both (the pairs it saves are the cheap ones the quick path decides without `can_see`). What is left in step is the
skip test itself, about a nanosecond a pair in the crowd's own cells, and each refresh's own bookkeeping (about
0.5 µs). From 880 to 3,000 in step it grows 5.6 times for 3.4 times the crowd.

The drill at 200 bots (peon's `scripts/drill.sh 200 30`, the server on 4 cores, peon on 4 others, the box shared):
visibility's mean an update fell from 0.56 / 0.73 / 0.62 ms (out / back / random) to 0.070 / 0.073 / 0.062 ms, and
Kalimdor's update p50 / p99 from 3.2 / 6.4, 3.9 / 6.5 and 2.7 / 5.8 ms to 2.1 / 4.3, 2.1 / 4.4 and 2.1 / 4.7 ms;
broadcasts delivered and their latency as before (100.7 to 102.9%, p99 2 to 5 ms).

### How it is tested

`src/game/tests_visibility.rs`, on `vis_bench::Crowd` (a map with no task, the players put in directly): the quick
path against each refresh alone and whole, after every update of a 200-update walk of 120 players with three changes
of state an update (invisibility on and off, a ghost and risen, `.gm` on and off, teleports, a view from elsewhere),
three seeds, every player's knowledge equal; the threads against the map's thread (creates, packets and forgets);
the budget (a chunk an update, oldest first, the same knowledge at the end); and each change of state among a crowd
whose pairs the quick path would skip: invisibility, a ghost, `.gm`, a flight (noticed at its first move, nothing
marking its take-off), a teleport, a view from elsewhere, and a jump past `DRIFT`. The grid's slots have a test of
their own. The fast suite, the extended suite and the client check pass (`tests_p12l_nefarian`'s class call fails
before and after).

### Risks, and what is next

- **A change of state with no mark.** The quick path trusts that whatever makes an object stop being plain marks it
  (`visibility_dirty.insert`, `stealth_changed`) or happens as it moves (`moved` checks). Every such change found has
  one; one missed would leave a pair as it was until either moves past the quick path's reach.
- **The zone a cell late** (above).
- Next: the in-step crowd's remaining cost is the skip test over the crowd's own cells; anchors kept per cell as a
  bounding box would let more whole cells go. The fan-out of movement (the drill's second cost) is untouched.
## The outbound path (2026-10-22)

What a packet costs from a map's thread to a client's socket (`conn.rs`, `map/observers.rs`), measured with
`benches/outbound.rs` (`cargo bench --bench outbound`) on a 32-core box, the benchmark pinned to 8 cores that other
work shared, and with the drill at 200 bots (wvs on 4 cores, peon on 4).

What changed, in order:

1. **The writer** copies each frame into the burst's one buffer and encrypts its header there (ARC4 in order), one write
   a burst; the byte budget is a plain counter (one atomic add a frame) where it was a semaphore.
2. **The cork** (`conn::Cork`): while one is alive on a thread, what it queues for a client waits and goes in one push a
   client at its end. A map corks a run of movement packets and its update; any other event runs uncorked, what was
   corked pushed first, and so do `MapHandle::send`, `Map::take_player` and a session's control: a call's answer, a
   client handed on or a session told never overtakes what was sent before it. The budget is taken, a hold keeps what
   it keeps and a closed client takes nothing, at the send, as before.
3. **Broadcasts** (`conn::broadcast`): corked, a frame to an object's observers is kept once, with the set of them as
   bits in the map's numbering of its clients (`map::observers::ClientTable`); each observer's writer takes its share of
   the cork's broadcasts, in order with what was sent to it alone. An object's `Observers` keeps the set's bits until
   the set changes (any mutable use is a new generation; a debug build checks the kept bits against the set at every
   use). A held client's share goes into its hold at once. The map's cost is a packet's, not a packet's times its
   observers; the budget for a broadcast is counted at the push.
4. **Batches** (`[network] broadcast_batch_ms`, 10 by default): a map parks a run of movement's cork between bursts
   (`Cork::park`, `Cork::resume`), so a client gets one push (one wake of its writer, one write) a batch, not one an
   event. Pushed at the update, at any other event, or at the batch's end.
5. **Fewer heartbeats for distant observers** (`[visibility] distant_move_yards`, `distant_move_every`; off by
   default): past the yards, one in N of a mover's heartbeats; starts, stops, jumps, turns and landings always.

The microbenchmarks (deliveries or packets a second, the median; the before is the drill's merge, 0.1.93):

| Case | Before | After |
|---|---|---|
| writer, its queue full, to a sink | 62 M (16 ns a packet) | 65 to 74 M (14 to 15 ns) |
| writer, a cork's broadcasts (one client of 3,000 in each) | | 38 M (26 ns) |
| writer, a write a packet to a loopback socket | 150 k (6.6 µs) | the same: the system call |
| writer, 64 a write | 7.5 M | 7.5 M |
| the map's fan-out, 64 heartbeats to 100 / 880 / 2,000 / 3,000 observers | 41 / 40 / 40 / 40 M (25 ns) | 450 / 476 / 450 / 470 M (2.1 ns, broadcast) |
| the same, corked one by one | | 42 / 35 / 31 / 31 M |
| end to end to loopback sockets, 4 tokio workers, 100 / 880 / 2,000 / 3,000 | 12.2 / 12.7 / 10.8 / - M | 19.9 / 20.6 / 17.7 / 15.7 M |

End to end is the box's: 4 workers writing to 3,000 sockets whose reading threads share the same 8 cores. A packet's
write is about 6.6 µs a system call, so what counts is how many go in one: the batches.

The drill at 200 bots, the random phase:

| | Before | Cork and broadcasts, no batch | Batch 5 ms | Batch 10 ms |
|---|---|---|---|---|
| Kalimdor events, mean µs | 29.3 | 31.3 | 8.4 | 7.4 |
| Kalimdor update p50 / p99 ms | 3.22 / 6.13 | 2.67 / 5.19 | 2.48 / 4.88 | 2.64 / 5.33 |
| Kalimdor's thread busy | 9% | 8% | 6% | 6% |
| wvs CPU (cores) | 0.89 | 0.83 | 0.33 | 0.26 |
| broadcast latency p50 / p99 ms | 0 / 4 | 1 / 4 | 5.1 / 9.1 | 7 / 14 |
| delivered | 100.8% | 100.8% | 100.8% | 100.7% |

Without the batch an event cost what it did: at 200 bots the fan-out's pushes were cheap, and its cost was the wake of
each observer's writer, one an event. With it, the server's CPU falls by two thirds for 170,000 packets a second, for
7 ms more latency at the median. At 3,000 bots each moving as the drill's do (about 4 packets a second) the map's
broadcasts cost about 2 ns a delivery, 38 million deliveries a second, about 8% of its thread, where one by one they
would cost more than the thread; on top of that is a push to each client a batch (at 10 ms, about 360,000 pushes a
second for 3,000, each with a wake of its writer), which the drills past 1,000 should measure.

`SMSG_COMPRESSED_MOVES` (0x2FB), looked at for carrying other players' movement: mangos-classic never sends it
(`Handle_ServerSide`), and wow_messages lays it out for monster moves and splines only. In the 1.12.1 client its
handler reads a size, inflates the rest, and hands each inner packet (a byte of size, then the opcode and body) to the
same dispatch as a packet from the socket, whose table has `MSG_MOVE_HEARTBEAT`'s handler: by the code, it would take
players' movement. It isn't in the client check, and isn't sent: the batches already make one write of a client's
packets, and compressing each client's share would cost the deflate a client, not once.
## The mailbox and arrival (stage C)

What the drill showed past the knee: Kalimdor's mailbox growing without end (825,000 events at 2,000 bots, broadcasts
a minute late) and placement held to about 45 bots a second. The changes:

- **A mover's heartbeats coalesced** (`Session::heartbeat`, `map::Heartbeat`). A heartbeat waits for its map in a
  slot of its session's: while the map hasn't taken it and nothing else the client sent is queued behind it, the
  client's next heartbeat takes its place, if it has the same movement flags (a run's; a fall's last heartbeat and
  its landing, a swim's first, a jump's start are never folded). Every other packet keeps its place, and a heartbeat
  never goes ahead of one: one sent after another packet goes behind it, as any packet. The map takes the newest, and
  judges its speed from the last move it took and when (`move_guard`'s bank fills with time, so a longer step after a
  longer wait is no faster). Counted in the stats file (`heartbeats_coalesced`).
- **Each client's share of the mailbox bounded** (`[network] max_queued_packets`, 64). Every packet a session sends
  its map holds a permit until the map has handled it (the event carries it, so it is given back however the event
  ends: handled, dropped, handed on). With its share full the session waits, and reads no more of its client: the
  reader's queue fills and the socket with it. Nothing is dropped or reordered; a client that floods is cut off by the
  flood guard as before, as fast as its map takes its packets. The mailbox is at most the share times the clients
  (192,000 for 3,000). Counted (`packets_held_back`).
- **A burst of events runs at most 20 ms** (`EVENT_BURST`) before the map looks at its ticker again, not 512 events:
  a crowd's arrivals, a few ms each, had held Kalimdor to two or three updates a second.
- **Update packets framed once for everyone who gets them alike** (`update::SharedFrames`, `Batch::finish_shared`):
  every packet of the update's batches (and of the batches let go after an event) is looked up by its content, and
  one already built is shared, its frame and compression with it. A packet ends where its own blocks say (past 2 kB,
  after a block whose hash is one in 16; at 24 kB at most), so batches with a run of blocks alike cut it alike
  wherever it starts: a crowd told of a newcomer gets one frame (2,000 batches, two frames built), and newcomers to a
  crowd share most of its packets. Big batches come in packets of about 10 kB rather than 24 kB.
- **`.gear` looks through each slot's items** (`GameData::items_of_type`, built at its first use), not all 20,000
  for each of its twenty slots: 2.37 ms of the map's thread a `.gear` to 0.39.
- **An intro far from the character isn't followed** (`cinematic::begin`): with `[spawn]` a new human starts in
  Tanaris and its intro's camera flies over Northshire; the view went there, every player around was forgotten, and
  sent again as the client ended the intro (the crowd's creates, twice, for every newcomer).

`benches/crowd.rs` (`cargo bench --features world-bench --bench crowd`, with `WVS_DATA` for the real world's items)
puts a crowd on the drill's disc through the real sessions and map and measures both, on 8 cores of a 32-core box.
The probe now counts what the map does after an event (`after_events`: the holds let go, a newcomer's refresh) as an
event of its own.

**Movement** (every player a heartbeat each 250 ms, 10 s):

| Crowd | Mailbox after an update, p50 / max, before | after | Heartbeats the map handled a second, before / after |
|---|---|---|---|
| 880 | 0 / 888 | 0 / 883 | 3,534 / 3,460 |
| 2,000 | 19,350 / 49,984 | 1,487 / 2,036 | 3,006 / 1,793 (the newest of each mover's) |
| 3,000 | 20,355 / 99,755 | 2,807 / 3,058 | 1,488 / 872 |

With every second packet a facing (which isn't coalesced), 3,000 for 20 s: a share of 256 let the mailbox reach
194,083 and still grow; a share of 16 held it at 48,213 (3,000 times 16), 32,633 packets waiting in their sessions.
Past 1,000 the update is still saturated by visibility (stage A's): what this buys is a map that is behind by one
heartbeat a mover, not by a minute.

**Arrival** (newcomers as the drill's bots come: created, logged in at the disc, `.character level 30`, `.gear`,
`.god on`, `.gm off`, `.go`, the teleport acknowledged; placed a second):

| Crowd | 200 at once, before / after | 400 asked at 400 a second, before / after | Updates a second meanwhile, before / after |
|---|---|---|---|
| 100 | 269 / 560 | | |
| 880 | 111 / 264 | 101 / 261 | 2.0 / 15.0 |
| 2,000 | 77 / 134 | 52 / 123 | 1.3 / 12.6 |
| 3,000 | 36 / 75 | 46 / 67 | 0.7 / 11.8 |

The 52 a second asked at 2,000 is the drill's 44. The update's `batches` step at 2,000 and 400 newcomers went from
2,269 ms to 160 ms. What holds placement now is the refresh of each newcomer, both ways, at its login: about 4,000
introductions at 2,000, 3.4 ms at the end of the login's event, and 1.1 ms more after its `.go`: two thirds of the
map's time per newcomer, the visibility refresh's per-pair cost that stage A's quick path cuts. The drill at 200
bots (the server and peon on 4 cores each, a 1 s ramp, ramp-bound at 165 placed a second) is unchanged: update p99 5.2 ms at
random, every broadcast delivered within 3 ms p99; its arrival events took 172 µs on average, from 338.

## The drill, more like players (2026-10-22)

The Tanaris drill's bots are now more like a crowd of players (peon's `--mode drill`): every Alliance class, made
ready as a raider of it is (`.character spells`, the quests' spells and reagents), hunters with their pets out
(`.pet`, `Call Pet` if it isn't) and warlocks with their Voidwalkers (summoned as they gather, and at random standing
for it when theirs is gone); half of them (drawn from the seed) with a weapon and a chest enchantment for their class
(`.gear enchant`, below: Crusader, +15 agility, +30 spell damage or +55 healing; +100 health, +50 mana or +3 stats);
and over the random phase, priests, mages, druids and paladins buff the players within 28 yd who lack it (Power Word:
Fortitude, Arcane Intellect, Mark of the Wild; Blessing of Kings for paladins, shamans and druids, Might for warriors,
rogues and hunters, Wisdom for the rest): `CMSG_SET_SELECTION` and `CMSG_CAST_SPELL` at their highest rank at 30,
each bot's first at random in the phase's first half, then one every 1.6 to 5 s as the global cooldown, the server's
answer and its own mana let it, and again where one wears off. The phases and their measures are as they were; the
summary counts, a phase, the buffs cast, gone off and refused, the pets' calls and summons, and the pets out.

Two fixes on the server's side, for what the drill showed:

- **A unique item isn't worn twice.** A bot wore two of one unique ring: `.gear` fills each finger from the same pool,
  and the store's way in for a new item worn (`add_item_to` to an equipment slot, also a purchase into one) didn't
  look at `max_count`, 1.12's only uniqueness (no 1.12 item has mangos's unique-equipped flag, which
  `CanEquipUniqueItem` checks; mangos's `CanEquipNewItem` checks the count through `CanTakeMoreSimilarItems`).
  `may_equip_new` now refuses a unique one beside the one carried ("can't carry more"), as the way into the bags
  (`plan`, `plan_at`: `.additem`, loot and the rest) did. `.gear` also skips a unique item whose name is worn already: the
  battlegrounds' rings are one name at several levels, alike to see.
- **`.gear enchant <slot> <enchantment>`**, a GM's: the item worn in a slot (0 to 18, or head, chest, mainhand ...)
  given a lasting enchantment (`SpellItemEnchantment`'s id) in its permanent slot, through the enchanter's path
  (`enchant::enchant_item`: its stats, procs and glow).

Tested in `tests_gear_unique` (fast: `.gear` over unique rings, the refusals every way, the command) and
`tests_gear` (the real items at 28, ten `.gear`s, Crusader on the weapon); peon's tests drive a drill bot's casts.

At 200 bots (the server and peon on 4 cores each of a busy 32-core box, a 25 s ramp; three runs): 30 priests, 23 mages,
20 paladins and 4 druids cast 569 to 576 buffs in the random phase, every one gone off, none refused; 18 hunters and
19 warlocks had 30 to 33 pets out at the random phase's end (creatures kill some; they come again). The server, at
random:

| | Before (stage C) | Now |
|---|---|---|
| Kalimdor update p50 / p99 ms | 2.8 / 5.2 | 3.1 / 9.8 to 14.3 |
| Broadcasts got / due | 100.8% | 100.8% |
| Broadcast latency p50 / p99 ms | 0 / 3 | 1 / 7 to 14 |
| Packets out a second | 169,933 | 185,331 |
| CPU | 1.33 | 0.68 |

The update's dearest steps are now the creatures' (despawns about 1 ms an update, auras 0.7 to 1.0, values 0.5, pets
0.3, as the pets and the buffs' auras come in), not visibility; the before is the earlier drill on a quieter box (its
cores to itself), so the update's p99 and the latency are the box's as much as the crowd's.
## A path on the ground (2026-10-22)

The drill's crocolisks and basilisks went through the sand chasing a bot a little below them. A path from the
navmesh is its polygons' corners joined straight, each end kept at the height it was asked at where it lies over a
polygon; a chase's goal is beside its victim at the victim's height. A few yards up a slope from the victim that goal
is under the ground (3.6 yards on a dune south-west of Waterspring Field), and a straight leg over a dune goes through
it (0.7 yards in the Field). `Nav::path` now puts each point after the first on the navmesh's ground (its detail
mesh's height, `find_nearest_poly` and `poly_height`) where it is more than half a yard off it, and looks along each
leg every 2 yards: where the line leaves the ground by more than half a yard, a point on the ground goes in where it
leaves it most, and so on, at most 64 a path (mangos's smooth path puts one every 4 yards). A point on the ground is
left alone, so a path over flat ground is what it was. The test is `tests_chase_ground` (extended).

What it costs (`probe_path_on_the_ground`, release, one core): 2,000 chases of 5 to 40 yards about the Field, every
one a straight hop, took 0.6 µs each as the navmesh found them and 3.7 µs on the ground, with 4,550 points for
4,000.

The leash also changed: its radius is 31 yards (mangos's 30 and a yard), and a creature standing still in reach of its
victim, attacking, has its leash renewed every `[creatures] leash_standing_ms` (1.25 s): no cost to speak of, a
timestamp a creature fighting in reach.
## After the three merges: a test that raced the map, and Frost Nova's ranks (2026-10-22)

**`tests_p12l_bwl::ebonroc_fights_as_mangos_classic`**, failing every run alone since the visibility merge (first
at `086cdd1f`, its parent passing), was a test racing the map, not a regression in what a client sees. The test
pulls Ebonroc and makes his Shadow of Ebonroc, Shadow Flame, Thrash and then Wing Buffet due, each in a query: all
of it a few milliseconds, which can fall between two 20 Hz updates. Wing Buffet is a cone (target 24) at its
caster's facing, and a drake turns to his victim only on a fight turn of his own (mangos's chase does the same; its
`OnSpellCastStateChange` leaves a cone's caster as it faces). With no update between the pull and the buffet, he
still faced his spawn's way and the cone missed. The visibility merge moved the client's sight of the boss by an
update relative to the map's clock (it saw him a step sooner after its arrival), so the pull now always came just
after an update rather than just before one; loaded, as in the whole suite, it passed by luck. Firemaw and Flamegor
had the same race (Flamegor failed once alone). The fix is in the test (`buffet_halves_threat`): out of his last
cast, it waits for the drake to face his victim before it makes the buffet due. `tests_p12l` passed 10 runs of 10
on an 8-core share of a 32-core box.

**Frost Nova's ranks** (a wait of category 35's alone, 25 s, none of the spell's own): after rank 1 the client
showed the other ranks ready, though the server refused them. The 1.12 client (build 5875's `SMSG_SPELL_GO`
handler) starts its own cast's cooldown from its Spell.dbc, the spell's and its category's, only when the go's
first GUID, the cast item, is the caster's own; any other it takes for an item, and with no such item it starts
none. mangos writes the caster there when no item cast the spell (`Spell::SendSpellGo`, `SendSpellStart`); wvs
wrote an empty GUID, so the client started nothing, and the only cooldown it had was `SMSG_SPELL_COOLDOWN`'s, which
its handler takes as the spell's own (a time given there sets no category wait). Now the start and the go name the
caster when no item cast the spell, and a cast's `SMSG_SPELL_COOLDOWN` also names each other spell of the category
the player knows, with the category's time (mangos's `AddSpellAndCategoryCooldowns` holds each of them; Cold Snap's
reset already clears each). `.debug cooldowns` clears those too. The packets' layouts are unchanged; the fast and
the extended suites, the client check among them, pass.
## Area spells' reach (2026-10-22)

An area about its caster or at a place (Arcane Explosion, Frost Nova, Blizzard; friends' areas too) took the units
whose centres were within its radius, nothing more. Now (`spells::targets::area_units`) it reaches:

- **A creature's size past the radius, for a player's area** (its combat reach, 1.5 for most): mangos-classic's
  `SpellNotifierCreatureAndPlayer`, which adds it for the player-controlled at the not. A player in a player's area,
  and anyone in a creature's, gets none, as there.
- **A leeway past that when either side moves**: `[spells] area_leeway_moving_yards` (2, mangos-classic's
  `AOE_LEEWAY`) when the caster or the unit runs, forward or back; `area_leeway_strafing_yards` (8/3,
  mangos-classic's `MELEE_LEEWAY`) when either strafes. Walking gives none. mangos-classic gives its 2 yards for the
  caster's running alone; the unit's running and the strafe's larger leeway go beyond it (the user's call).

Cones (since given the same reach, below), script areas, party and raid areas are as they were.

The grid is asked for the radius, the strafe's leeway and 30 yards (the world's largest model reaches 26.25) as a
disc (`Grid::within`), so a 10-yard area still looks in at most nine cells. A unit past the radius costs a lookup of
its creature for its size and gait. Measured with 400 creatures in a 60-yard square about the caster, on 4 cores of
a 32-core box (release, one area's targets, 2,000 times):

| Radius | Before | After | Units taken, before / after |
|---|---|---|---|
| 8 | 4.2 µs | 8.2 µs | 22 / 38 |
| 10 | 4.5 µs | 8.4 µs | 38 / 46 |
| 20 | 6.2 µs | 10.4 µs | 138 / 162 |
| 30 | 9.6 µs | 13.5 µs | 316 / 336 |

Four microseconds more an area cast, in a crowd: nothing beside an update's 50 ms.

## A chase in melee, and blows at a creature on its way home (2026-10-22)

A creature chasing its victim jerked about it: it aimed half a yard inside its reach (Thistle Tea's
`chase_target_distance`), so every step its victim took back put it off the edge and sent it a new path, and it sent
a turn (an `SMSG_MONSTER_MOVE` to where it stood) at a bite whenever its victim had moved a degree. Now
(`ai::fight_step`) it chases as mangos-classic's `ChaseMovementGenerator` does:

- it aims at **half its reach** from its victim (`CHASE_DEFAULT_RANGE_FACTOR`, `combat::chase_distance`);
- it paths again only once its victim is **out of reach of where it is headed** (the walk's end, or where it
  stands): `RequiresNewPosition`, the combined reach from the spline's end; no more re-chase distance from where it
  last aimed, nor a path for every update it stood out of reach;
- on its way in it stops short only when its victim, moved since, has come nearer than where it is headed (no more
  stop at contact);
- it turns to a victim near its front without a word (mangos's `SetInFront`: the client turns a creature to its
  target itself); a turn is sent only when its victim is off its front (Thistle Tea's `face_melee_victim`, on bad
  facing).

Counted on the map's own clock at 20 Hz (`tests_chase_evade`), the moves one client is sent of a beast fighting it:

| Six seconds of | Before | After |
|---|---|---|
| a victim standing | 0 | 0 |
| a victim walking about a yard each way, the beast come in from 15 yards | 1 path, 1 turn | none |
| a victim walking away at 2.5 yards a second | 21 paths | 5 paths |

Each path is a navmesh query as well as a packet to every observer, so a crowd fighting a crowd paths about a
quarter as often as it did. Pets chase as they did.

A creature on its way home after a leash was refused as a target ("Invalid target", `SPELL_FAILED_BAD_TARGETS`, or
"You cannot attack that target"). Now (`combat::can_aim_at`) a player's spell at it goes off and misses as an evade
(`SPELL_MISS_EVADE` in the go), and a player's swing starts and each blow evades (`SMSG_ATTACKERSTATEUPDATE`, a miss
of no sound, `VICTIMSTATE_EVADES`, no damage), as mangos-classic's `MELEE_HIT_EVADE`; neither puts anyone in a fight
with it. Areas still pass it by, pets and creatures still let go of it, and its evade still stops those swinging at
it (mangos's `RemoveAllAttackers`). The evaded blows pass the client check.
## Cones' reach (2026-10-22)

A cone before its caster (Cone of Cold, Frost Breath, Flame Breath, a creature's breath) took the units whose
centres were within its radius and its angle. Now it reaches as an area about its caster does (`area_units`, the
same code): a creature's size past the radius for a player's cone, and `[spells]`'s leeways when either side runs
or strafes. The angle is as it was, from the caster's facing, at the unit's centre. A player in a player's cone still
gets no size; a creature's own running still gives its leeway. Script cones (target 60) are as they were.

The grid is asked for the larger disc an area's is, and a unit past the radius costs its creature's lookup, as for
areas. Measured as areas' were (400 creatures in a 60-yard square about the caster, on 4 cores of a 32-core box,
release, one cone's targets, 2,000 times; the time is the whole of `effect_targets`, the enemy checks included):

| Radius | Before | After | Units taken, before / after |
|---|---|---|---|
| 8 | 9.1 µs | 14.6 µs | 2 / 2 |
| 10 | 11.7 µs | 18.8 µs | 3 / 6 |
| 20 | 36.6 µs | 42.7 µs | 23 / 27 |
| 30 | 81.6 µs | 86.2 µs | 52 / 55 |

Five to seven microseconds more a cone cast, in a crowd, as for areas: nothing beside an update's 50 ms.
## The 2,000 drill, round two (2026-10-22)

peon's drill (above) on a 32-core box shared with other work: the server pinned to 4 of its cores, peon to 12
others, never past 45% of them. The goal: 880 under 20 ms p99 in every phase, 2,000 under 50 ms p99 with every
broadcast delivered within 100 ms p99, then 3,000. Where it stood (0.1.97): at 2,000, Kalimdor's thread 99% busy,
two thirds of the broadcasts delivered (the rest coalesced in the mailbox), a second late.

### Where a movement packet's time went

A Tracy capture of the 2,000 drill (`WOW_TRACY=1`, Tracy 0.14.1's `capture` and `csvexport`), with a zone for each
step of a move (`move: checks`, `taken`, `visibility`, `encode`, `broadcast`, `after`), the cork's broadcast and
push, and the writer's bursts ([PROFILING.md](PROFILING.md)). The mean of each, over the run's 785,000 movement
packets:

| Step | Mean |
|---|---|
| the gates, the root and the speed check (`move: checks`) | 1.1 µs |
| the move taken: the guard, the fall, a cast or a shot it breaks (`taken`) | 2.2 µs |
| the grid and visibility's mark (`visibility`) | 0.3 µs |
| the relay encoded (`encode`) | 0.5 µs |
| the broadcast to those who see the mover (`broadcast`) | **90 µs** |
| the stand, the auras it ends, the safe place (`after`) | 0.4 µs |

The whole of a heartbeat's event: 136 µs. Of the broadcast's 90 µs, 32 were the cork's: the first time in a batch
it reaches each of 2,000 clients it looks the client up and gives it a slot (44 µs a broadcast that reached anyone
new, in a zone of the capture's). The rest was a bug: a heartbeat that came through its session's slot (`MapEvent::Heartbeat`,
stage C's coalescing, most of them) wasn't corked (`map::corks` knew only `MapEvent::Packet`), so each went to every
observer's queue on its own, a push and a writer's wake each: 131,000 of the 299,000 broadcasts of a shorter capture,
2,000 hand-offs apiece. Besides the events: 1,000 cork pushes a second at 26 µs to 200 µs (each a hand-off to every
client reached), and, on the runtime's other threads, 25 million writer bursts at 2.3 µs.

After the fix (a heartbeat corked as any movement packet is): a heartbeat 9.3 µs, its broadcast 4.4 µs (the
first-reach 3.9 of it), 1,000 pushes a second at 26 µs. After the feed (below): a heartbeat 6.0 µs, its broadcast
0.7 µs, 60 publications a second at 46 µs, and the feed's waker 11,000 turns at 0.9 ms on its own thread.

Also: an object's observers keep their clients' bits as the set changes (`Observers::add`, `take`), not built again
at the next broadcast after any change (a crowd at the edge of sight changes its sets every few packets).

### The fan-out off the map's thread: a feed (`conn::Feed`)

Each map publishes its broadcasts in chunks that its clients' writers read themselves: a cork's run of movement
and its update are one chunk, published with one store and a word to the feed's waker, whatever the number of
clients. What the cork kept for one client alone goes in the chunk too, in its place among the broadcasts (by the
client's index: `Directs`); what anyone sends a client uncorked is stamped with the last chunk its feed had
published, and written after it. A client becomes a feed's reader the first time a broadcast reaches it (a `Sub` in
its queue, with the chunk it reads after) and stops when its index is let go (an `Unsub` with the last chunk it
reads): an index given out again is read by the new client only from where it got it. A held client's broadcasts
go into its hold, as before, the feed knowing who is held. The waker (a thread of its own) charges each client its
share of each chunk (the bytes of the broadcasts to it) and wakes those that have one: a client whose share and
queue pass its budget is disconnected, as before; the writer counts what it takes, so a reader that falls behind
by more than its budget is cut off, and a chunk is let go once every reader has passed it.

What a client is sent, and in what order, is as before: the tests in `conn.rs` (a direct send in its place among a
chunk's broadcasts; one from another thread after the chunks published before it; an index let go and given
again; a client moved to another map's feed; a writer woken by the waker), the fast suite, the extended suite and
the client check pass.

### Movement across the threads (`map::moves`, `[world] parallel_moves`)

A burst's movement packets kept until something else comes or the burst ends, then each mover's packets stepped in
order on its own player across the visibility's threads (the gates, the root and speed checks, the fall, the move,
the relay encoded), and what they tell the map (the grid, visibility's mark, the broadcast, the safe place) done on
its thread in the order the packets came. A packet that does more than move its own player (put back or pulled
back, a cast or a shot it breaks, an aura it ends, a stand, a hurting fall, a liquid left, the void) and every one
after it in the burst is handled one by one, and what any mover did past it is undone: before it, nothing that a
mover's check reads has changed but by its own packets, so the outcome is the same as one by one.
`tests_moves::a_burst_across_the_threads_comes_out_as_one_by_one` runs 40 bursts of 120 to 280 packets on two
crowds of 150 alike (runs, stops, turns, jumps and landings, long falls that hurt, leaps that are pulled back;
between bursts roots, stands, an aura that moving ends, deaths, game masters), and after each finds every player's
state, every client's packets and visibility's marks the same, with both ways taken. The fast and the extended
suites pass with it on.

It doesn't pay here, and is off by default. A crowd of 2,000's bursts of 400 steps, on 4 cores: 2.78 µs a packet one
by one, 2.83 across with one thread and 2.95 with four; most of a packet's time is what it tells the map, which stays
on its thread. In the drill (on the 4 cores the runtime's workers and the waker share) the step took 2.7 ms a
burst, 17 µs a packet, against 6 µs one by one: the threads' hand-off costs more than the checks it moves.

### A memory guard (`[network] memory_guard_fraction`)

Past that share of the machine's memory (0.8; the cgroup's limit if less), the sessions stop reading their clients
until the server is a twentieth under: what each has sent waits in its reader's queue and its socket, nothing is
dropped, and what the maps have drains. Logged as it engages and lets go; in the stats file (`memory_guard`). A test
holds a client back with it on and sees what it sent handled, in order, once it lets go. No drill reached it (3.4 GB
resident at 2,000, on a box of 128 GB).

### Niceness (`[server] nice`)

Set at start-up on every thread (Linux's niceness is a thread's); refused, it is logged with how to grant it. This
box has `sched_autogroup` on, which weighs a niceness only against the other processes of its session: here the
server's and peon's (one session), against other sessions' not at all. The 2,000 drill three times, random phase:

| | Update p50 / p99 ms | Broadcast latency p50 / p99 ms | Delivered | Thread busy |
|---|---|---|---|---|
| nice 0 | 14.3 / 63.5 | 17 / 85 | 100.7% | 41% |
| nice 0, the server under `sudo` | 13.8 / 54.4 | 17 / 75 | 100.7% | 40% |
| nice -10, the server under `sudo` | 14.1 / 54.0 | 17 / 75 | 100.7% | 41% |

No difference past the runs' own spread, in any phase (the out phase's latency p99: 178, 141, 211 ms): the server
and peon have cores of their own, and the server isn't short of CPU (2.8 of its 4 cores, its map's thread 40% busy);
what it shares its cores with runs in other sessions, which a niceness doesn't reach.

### Measured

The drill, the server on 4 cores (peon's binary changed under these runs, its drill the same):

| 2,000 bots, random | Before (0.1.97) | Heartbeats corked | The feed | All |
|---|---|---|---|---|
| Kalimdor update p50 / p99 ms | 12.6 / 30.5 | 15.4 / 54.7 | 15.7 / 57.7 | 14.3 / 63.5 |
| broadcasts delivered | 67.0% | 100.8% | 100.8% | 100.7% |
| broadcast latency p50 / p99 ms | 1,040 / 1,163 | 21 / 83 | 20 / 87 | 17 / 85 |
| events a second, mean µs | 5,493, 135 | 8,330, 15.3 | 8,339, 6.0 | 8,297, 6.0 |
| Kalimdor's thread busy | 99% | 52% | 45% | 41% |
| mailbox (most) | 6,545 | 912 | 824 | 972 |
| wvs CPU | 3.74 cores | 2.64 | 2.77 | 2.78 |

The runs together (out and back) at 2,000: update p99 31 and 37 ms to 28 and 32, every broadcast delivered (50%
before), latency p99 307 and 533 ms to 178 and 69. Before, the update's p99 was low because the map was behind by a
second: it now handles every packet as it comes, and its update is the whole of what is left.

| 3,000 bots | Before | After |
|---|---|---|
| random: update p50 / p99 ms | 26.0 / 130 | 34.9 / 104 |
| random: delivered, latency p50 / p99 ms | 88.2%, 7,143 / 9,044 | 100.7%, 83 / 244 |
| random: mailbox (most), thread busy | 104,892, 98% | 2,783, 77% |
| out: delivered, latency p99 ms | 12.3%, 504 | 101.1%, 381 |
| wvs CPU | 3.61 cores | 3.44 |

880 bots: update p99 15 ms out and back, 21 ms at random; latency p99 27, 26 and 48 ms.

### What is left

The update. At 2,000 at random its dearest steps are `values` (5.5 ms) and `batches` (3.2 ms): about fifty players'
public fields change an update (health and power at their ticks, auras, targets), and each one's values block goes
into each of 2,000 observers' batches, and each observer's batch is framed: the square of the crowd again, now in
the update. At 3,000: `values` 11.4, `visibility` 8.4, `batches` 7.6 ms, and the server at 3.4 of its 4 cores. A
values block shared through the feed (one chunk for a crowd's dirty players, as the broadcasts are) would take it
off the map's thread the same way.

## Food and drink's cooldown (2026-10-22)

The user: eating should not have a cooldown, only the first bite a second's wait, and the same for drink. Food and
drink items (Tough Jerky 117, Refreshing Spring Water 159) have no cooldown of their own and a second for their
category (11, 59); their spells (Food 433, Drink 430) wait a minute for the category in Spell.dbc, and have no
global cooldown. The server already held the category a second, and told the client a second in
`SMSG_SPELL_COOLDOWN`. What the client showed longer was `SMSG_ITEM_COOLDOWN`, sent twice on every use (once by
`cooldowns::start`, once by `items::used`). The 1.12 client's handler for it (build 5875's, the opcode's
registration at `0x6E71BA`, the handler `0x6E95D0`) finds the item by its GUID and starts thirty seconds on the
item's spell under its entry, whatever the item's times: it is the equip cooldown, which is the only place mangos
sends it (`Player::ApplyEquipCooldown`). So each bite put the food on thirty seconds. Neither the start nor
`SMSG_SPELL_COOLDOWN` added a global cooldown: the client's `SMSG_SPELL_COOLDOWN` handler (`0x6E9460`) starts the
spell's start-recovery wait only from Spell.dbc, nothing for Food's 0.

The fix: no `SMSG_ITEM_COOLDOWN` for a use (the go's cast item, the item's GUID, has the client start the item's own
times from its cache, `0x6E2D23`: an item spell's time of zero or more, else the spell's). And the server picks an
item's times as mangos's `AddCooldown` and the client do, each on its own (`cooldowns::times`): a time of zero or
more over the spell's, -1 the spell's, a category of 0 the spell's. Before, an item with neither time above zero
(319 on-use items, as `0, 0`) took its spell's times, and one with a single time set lost the spell's other.
`items::used` no longer starts cooldowns of its own, unmodified, over `start`'s. Tested by
`tests_p11g_cooldowns::food_and_drink_wait_a_second_for_their_category_and_no_more` (two foods and a drink: a second
for the category, the other food refused for it and eaten after it, the drink and a spell on the global cooldown
at once, no `SMSG_ITEM_COOLDOWN`), its real-world twin, and `times`' unit test. No packet's layout changed; one is
no longer sent. The fast and extended suites, the client check among them, pass, but for
`tests_p6_player_real::corina_steele_sells_and_repairs`, which fails before this change too.

## A spellbook lost at a far teleport (2026-10-22)

The user, playing a priest: `.tele Deadmines`, `.character spells` there, Mind Control on a bar, then out of the
Deadmines to Westfall (map 36 to map 0); after it, the spellbook had none of the class's spells. The server's own
list was whole across the teleport (saved, handed between maps, the same spells); nothing it sent was malformed.
What it didn't send was the cause. The 1.12 client (build 5875) keeps the last `SMSG_INITIAL_SPELLS` it was sent as
a list (its handler `0x5E6510`, the list at `0xC4D2A0`), and the last `SMSG_ACTION_BUTTONS` (`0x5E6680`, at
`0xC4C2B4`). Each time its own player is created (`0x5DEA50`: at a login, and after every far teleport, whose
`SMSG_NEW_WORLD` destroys every object, its own player too), it empties the spellbook (`0x4B23A0`) and fills it
from that list, and sets the bars from those buttons. A spell learned in the world (`SMSG_LEARNED_SPELL`,
`0x5E61C0`; `SMSG_SUPERCEDED_SPELL`, `0x5E6330`) goes into the book it shows alone, and into the list only when
it has no player of its own; a button the player sets goes into the bars alone. mangos sends both again at every
`MSG_MOVE_WORLDPORT_ACK` (`HandleMoveWorldportAckOpcode`, `SendInitialPacketsBeforeAddToMap`); wvs sent them only
at login, so a far teleport took back every spell learned, and every button set, since the login: the priest's
book went back to a level-1 priest's Smite and Lesser Heal among the general spells, and Mind Control off the bar
with it, as a spell the book no longer had.

The fix: the worldport sends `SMSG_INITIAL_SPELLS` (with the cooldowns running, as at login) and
`SMSG_ACTION_BUTTONS` before the player's own create (`login::send_spells_and_bars`, shared with the login). A near
teleport sends neither, as mangos. What changes on the wire is those two packets at each far teleport; nothing
else, and no layout. The client check gained a check of both (`tools/client_check/initial_spells.py`: the handlers
must read each packet exactly, and the list and buttons the client keeps must be the packet's), run by the client
emulation tests and by the new tests on what a map change sends. Tested by `tests_spellbook_lost`: a level 60
priest, mage and warrior, on the real world, taught in the Deadmines, a spell of theirs (Mind Control, Frostbolt,
Execute) on the first button, out through its exit (area trigger 119) to Westfall, with a model of the client's
book and bars over every frame received (`Book`), which must be the server's after; and, in the fast suite, a
spell learned and a button set after a login, kept across a far teleport and not told again at a near one. Without
the fix all four fail, the client's model book back to the login's. The fast and extended suites pass.
## A kill's impact (2026-10-22)

The report: Smite kills a level 1 rat at once, but its impact never plays on the rat. Nothing was changed: what wvs
sends for a kill is right, and in mangos-classic's order. Recorded by `tests_kill_visual`, a level 60 priest's Smite
(rank 8) on a level 1 Rat (a critter) and on a level 1 Young Wolf (a beast), in the real world, and a bolt of the
same shape (no speed, a cast time) on a critter and a beast in the fast suite. Her client gets `SMSG_SPELL_START`,
`SMSG_CAST_RESULT`, then `SMSG_SPELL_GO` with the victim in its hits (the caster in the cast item's place, the
victim in its targets), then `SMSG_SPELLNONMELEEDAMAGELOG`, `SMSG_ATTACKSTOP`, and only then, in the update's
values, the victim's health at 0. Nothing forgets the victim: the corpse stays, a critter's as a beast's. That is
mangos-classic's `Spell::cast` (`SendCastResult`, `SendSpellGo`, then `handle_immediate`'s `DoAllEffectOnTarget`,
whose damage log and `Unit::Kill` come before the map's object updates carry the death). Smite has no speed in
either database, so neither server holds its damage back.

The 1.12.1 client (build 5875) plays the impact from the go alone. `SMSG_SPELL_GO` is registered at `0x6E717A`,
handled by `0x6E7640` and read by `0x6E7A70`. For a spell of no speed in the client's Spell.dbc (Smite's is 0), it
calls `0x6E8BF0`, which looks up each unit in the go's hit list and starts the visual's impact kit on it right away
(`0x60EDF0`, kind 1). Smite's visual is 128, its impact kit 291, with `HolySmite_Low_Chest` on the chest attachment
(the Rat's model has that attachment, as the Wolf's does). Nothing on that path looks at the unit's health or
death. The go's cast flags only matter for the ammunition (0x20), how its hits and misses are merged (0x400), the combat log
(0x1, 0x1000) and a trade skill's recast (0x41). So the `0x100` mangos sets on a go, and the `0x2` it sets on a
start, which wvs leaves at 0, change nothing the client shows. The client is told about the hit while the rat is
alive and known to it. What becomes of the impact after the death arrives is the client's own rendering, and a
mangos-classic server would get the same result. The real-world tests run their packets through the client
check too.
## Out of sight, out of the fight (2026-10-22)

A creature gives up on a victim the moment that victim's client is told to forget it: past `[world]
visibility_yards` (100) and the 10-yard margin, by running or by a teleport. It no longer waits for the leash's 11
to 15 seconds. The visibility step's `part` notes each creature a player's client forgets (`Map::lost_sight`). Once
the step is done, `creature_combat::out_of_sight` looks at each one. If the creature has that player on its threat
list, and is no player's pet and no one's charm, the player comes off the list. A creature that was fighting them
turns to the next it may fight, or goes home evading if no one is left. That happens in the same update the client
forgets it. The victim then leaves combat at the next look at their fight (every 1.2 s) if nothing else holds them.
A creature that is hidden from the victim (its stealth, a script's concealment) is forgotten by their client too, but
it isn't out of their sight, so it fights on. So does one whose victim is themselves still within their sight
(`visibility_yards`) of it. A creature told of again in the same step (a despawn and respawn)
fights on as well. The setting is `[creatures] evade_out_of_sight`, on by default; off, the leash alone decides, as
before.

mangos-classic's `Player::BeforeVisibilityDestroy` does the same, but only outside dungeons and not for a creature
with combat-only stealth. Its leash is off in dungeons too. wvs applies the rule in dungeons as well, at the
user's ask. It also leaves the player's pet fighting the creature, where mangos stops the pet's attack.

Cost: one push per creature a client forgets, and a look at each once the step is done. Nothing measurable in
the update.

Tests (`tests_out_of_sight`, fast suite): a victim who outruns the brute's sight (150 yards off at once). In the
query that first finds the creature unknown to the client, it is evading with nothing on its list, and the victim is
out of combat within the next look. The same for a teleport of 150 yards: before this change, the brute chased her
for its leash's time. With Bob on the list too, still beside it, the brute drops Alice alone and fights Bob on.
With the setting off, the brute fights Alice on once forgotten, and she stays in combat. A creature a script
conceals is forgotten but fights on. A creature also fights on when the victim's camera, not the victim, has gone
away (an intro cinematic, Far Sight). The client forgets by its view point, so the rule also needs the player
themselves past their sight of the creature. That came from `the_real_scripted_bosses_fight_without_trouble`, which
failed about one run in three on the first version. A fresh character's intro camera was still being followed after
the test's teleport into Onyxia's lair. Her client forgot Onyxia five yards away, and Onyxia went home instead of
taking off. With the check, it passes 10 runs out of 10. The fast suite, the extended suite and the client check
pass.
