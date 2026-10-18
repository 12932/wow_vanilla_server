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

**Profiling.** `perf_event_paranoid` is 2 here, and neither `perf` nor `samply` will record, so there is now a
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
