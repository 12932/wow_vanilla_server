# Testing

All the tests are in the library, and run under [nextest](https://nexte.st), never `cargo test`: each test is a
process of its own. They are in two suites.

| Suite | Command | What | Time (16 cores, debug) |
|---|---|---|---|
| Fast | `cargo nextest run --lib` | every test that doesn't need the real world (1,201) | about 25 s |
| Extended | `cargo extended` | every test (1,448), the real world's too | about 3 minutes |

`cargo extended` is an alias (`.cargo/config.toml`) for `cargo nextest run --lib --profile extended --run-ignored
all`. Run the fast suite as you work, and the extended one before a merge or a push. Both use the same debug build,
so the extended suite builds nothing more.

## The real world

The extended suite's tests read the real world, from `WVS_DATA` if it is set, else the checkout's `data/`:

- `db`: `vmangos.sqlite` and `dbc.sqlite` (in the checkout, Git LFS pointers until `git lfs pull`);
- `nav`: a rustigator bake; `terrain`: Thistle Tea's tiles (`wvs-setup` builds both).

A test whose data isn't there returns at once and passes, as before; so the extended suite without `WVS_DATA` (or the
pulled databases) is the fast suite plus a few hundred tests that check nothing. Set it:

```sh
export WVS_DATA=/path/to/wow_vanilla_server/data
cargo extended
```

## Marking a test extended

A test that reads the real world is marked so:

```rust
#[tokio::test]
#[ignore = "extended"]
async fn the_real_conditions() {
```

The real world's only entry points are `harness::real_data(what)` (the `db`, `nav` and `terrain` directories),
`harness::load_real(dir)` (the loaded world) and `load::real_data()` (one shared, read-only load). Under nextest, in
any profile but `extended`, `harness::real_data` panics, naming the mark to add. So a test that reads the real world
and isn't marked fails the fast suite at once; it doesn't land there quietly, and doesn't pass there without running.
A marked test is skipped in the fast suite, and counted as skipped.

`cargo nextest run --lib --profile extended` without `--run-ignored all` runs only the fast tests (and says so,
in its skipped count): use the alias.

Hand-run probes, which print and check nothing, are `#[ignore = "a probe, run by hand"]` and named `probe_*`: the
extended profile leaves them out (`default-filter` in `.config/nextest.toml`). There are two:
`tests_terrain::probe_the_tiles` and `tests_arena::probe_the_pit`. Run one with
`cargo nextest run --lib --profile extended --run-ignored only --ignore-default-filter probe_the_pit --no-capture`.

To run one extended test: `cargo extended the_real_conditions`.

## Why it is fast

Before this split, `cargo nextest run --lib` ran all 1,421 tests in about 10 minutes (8 threads), or 476 s on 16:
2,917 CPU-seconds, of which the 221 tests on the real world were most. Each loaded the world again in its own
process, 8.4 s in a debug build, and a few walked for minutes: the escorts (340 s), the database's event quests
(159 s), the imported event quests (139 s), the Gizelton Caravan (115 s), `tests_events`' two (100 s and 86 s).

- **The split.** The fast suite is the 1,201 tests that don't need the real world: 315 test-seconds, 25 s of wall
  time on 16 cores (load average 2 to 3). Its slowest tests, at 4 to 9 s, are synthetic ones that wait in real time
  (`a_chased_rogue_is_struck_only_from_within_reach`, `onyxia_takes_to_the_air_and_lands_again`): they time the
  server against the client's clock, so the map's simulated clock (`skip`) can't stand in for the wait.
- **The world's snapshot.** `harness::load_real` loads the real world once per test binary and databases, and writes
  it (`postcard`, 30 MB) beside the test binary as `target/debug/deps/wvs-real-world-<key>.bin`; the other tests read
  it, in 0.9 s rather than 8.4 s. The key hashes the two databases' paths, sizes and modification times and the test
  binary's, so any rebuild (a change to the loader among them) or a new database is a new snapshot: one can't serve
  stale data. The first test to want a missing snapshot makes it, under a file lock, while the others wait; older
  snapshots are removed. `GameData` and its types derive serde's traits in test builds only
  (`cfg_attr(test, ...)`); `NpcData::graph`, a cache worked out on first use, is left out.
- **The long walks split.** The escorts and the database's event quests are eight shards each
  (`the_real_escort_quests_are_walked::shard_1` to `shard_8`), each walking every eighth on a realm of its own; the
  imported event quests and `tests_events`' walks are a test each. The assertions are the same, per shard. The
  extended profile starts them first (`priority`), so that the suite doesn't wait on one at its end.
- **The result.** The extended suite went from 476 s to about 170 s on 16 cores. What is left of its wall time is
  its longest tests: the escort shard with quest 1560 (Tooga's Quest, which fails and waits out its walk: 144 s),
  the Gizelton Caravan (116 s, one walk of two legs, one after the other) and The Great Masquerade (97 s).

## What wasn't done

- **The crate at opt-level 1 for the extended suite** (a cargo profile of its own): the world loads in 1.4 s and the
  suite runs in 117 s, but a dozen walks fail, the escorts' among them. They poll a fixed number of times, skipping
  the map's clock, with a 10 ms sleep between, and the map's real-time updates are what moves an escortee between
  polls: at opt-level 1 the polls come too fast for it. It would also be a second build, 3.5 minutes from clean.
- **Real-time waits in the walks.** The walks lean on the map's 20 Hz update running between a test's polls, so a
  machine fast enough (as opt-level 1 showed) fails them. `molten_cores_encounters_run_as_thistle_tea_scripts_them`
  did the same on this machine when idle: it checked Ragnaros the moment he was summoned, before the update that runs
  his spawned events (`event_ai::spawned`). It now waits up to a second for that update. The walks' polls would want
  the same: each waiting on a map update rather than on 10 ms.

## Under load

The extended suite runs on a shared machine, beside other suites (a load average of 15 to 40), where a map's 20 Hz
update and a client's reads come late. A test that budgets by looks or by seconds passes on an idle machine and fails
there. So:

- **A walk counts the map's updates** (`m.stats.updates`), not its looks, and hurries the timers by each update since
  its last look, not by each look: the Nightmare walk (`the_nightmare_manifests`) and the escorts' (`walk_escort`).
- **What a look keeps safe, it keeps safe between looks too**: the player in god mode (`p.misc.god`) rather than only
  healed at each look; a creature with a floor of health (`invincible_hp`).
- **A look leaves a fight alone once it matters**: the Nightmare walk took Remulos out of his fights while he walked,
  and on a late look, out of the one with Eranikus too.
- **A wait for what the server sends ends when it comes**: up to `harness::WAIT` (20 s), or, for what a map does at
  once, `Client::sees_on`, which waits for `harness::UPDATES` (400) of the map's updates as well. A wait for nothing
  keeps its short window.
