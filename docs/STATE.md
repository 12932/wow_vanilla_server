# Where wow_vanilla_server stands (2026-10-18)

wvs is a WoW 1.12.1 server in Rust. It is being aligned with [Thistle Tea](https://github.com/pikdum/thistle_tea),
an Elixir server, as the behaviour to match, with mangos-classic as a second reference for what 1.12 does. This page
covers how it's built, how to run, test and measure it, what changed on 17–18 October 2026, and what's left.

## How it's built
- **One actor per map:** a continent, a dungeon copy, a battleground. Each has its own 20 Hz update. Sessions talk
  to their map through a mailbox.
- **The world's data:** vmangos's database and the client's DBCs (Thistle Tea's SQLite export), loaded once into
  immutable tables. With them, the navmesh (a rustigator bake) and the terrain's liquids.
- **Characters and records:** in SQLite, through a store thread. Saves batch into transactions, and a move between
  characters and records (a trade, mail, an auction) lands in one transaction.
- **Supervision, some of Erlang's in Rust** ([RELIABILITY.md](RELIABILITY.md)):
  - a panic costs the step, creature turn or packet it happened in;
  - a creature that keeps panicking is restarted, then quarantined;
  - a map that keeps panicking is rebuilt, its players saved;
  - a watchdog thread reports a map that hangs or can't keep up.
- **The update's hooks:** the update looks only at creatures a player sees or that are fighting, and at what
  changed. New code goes through the hooks ([PERFORMANCE.md](PERFORMANCE.md)); debug builds assert it.
- **Packet order:** a client gets an object's create before anything that names it. A client the map has queued
  creates for is held (`Outbound::hold`): what it is sent waits, in order, until the update's "held" step sends its
  creates and forgets, then what waited; its values follow in the update's batch. It is held as an object is
  introduced to it, as one comes into the map near it, and as it arrives or lands from a teleport.

## Running it
- **Data:** `cargo run --release --bin wvs-setup` builds the navmesh and terrain from a 1.12.1 client. The databases
  ship in Git LFS. See the README.
- **The server:** `cargo run --release` from the checkout. It listens on 3724 (login) and 8085 (world).
- **Settings:** `config.toml`, in sections named after mangos's keys. Any key can be set from the environment
  (`WVS__RATES__XP_KILL=2`), `--print-config` shows them all, and `.reload config` (or SIGHUP) puts in the live ones.
  See [CONFIG.md](CONFIG.md).
- **Accounts:** the shipped config makes an account at first login, with the name as its password, and everyone a GM.
  That suits development and load tests; see sitrep question #53 before exposing the ports.

## Testing it ([TESTING.md](TESTING.md))
- **The fast suite**, `cargo nextest run --lib`: about 1,640 tests in 25 seconds, none needing the real world.
- **The extended suite**, `cargo extended`: every test, about 1,990, in 6 to 8 minutes. It loads the real world from
  a snapshot and runs the long walks in parallel shards. Run it before merging or pushing.
- **Marking:** a test that reads the real world is marked `#[ignore = "extended"]`. An unmarked one fails in the
  fast suite, saying so.
- **The client check** (`tools/client_check/`): runs `WoW.exe`'s own packet parsers under an emulator on what wvs
  sends, and fails on a fault, an over-read or an absurd count. It reproduces every client crash seen so far, and
  runs in the extended suite on a crowd's capture.
- **Racing the map:** a harness test queries a map whose update runs on real time. Wait for what you check, in a
  poll up to a limit, or count the map's updates. The load-only failures seen were all this.

## Measuring it
- **Tracy** ([PROFILING.md](PROFILING.md)):
  - set `WOW_TRACY=1` and connect Tracy v0.13.1;
  - you get a frame per map update, a zone per step, packet and transaction, and plots once a second;
  - it costs nothing measurable when off.
- **pprof:** `--features pprof`, for flame graphs without a GUI.
- **Load tests:** with [peon](https://github.com/12932/peon), at most 5 minutes a run:
  - the baselines: idle, cluster, PvP arena, race;
  - `--spread world`, which puts bots all over both continents.
- **The world benchmark:** `wvs-world-bench` (`--features world-bench`) runs every map's update with observers at
  the cover points (`docs/world_points.csv`), with no network.
- **Where it stands:**
  - 100 bots in one place cost 0.2 to 0.3 cores, and an Eastern Kingdoms update about 3 ms.
  - With every creature on both continents awake, an update needs 25 to 29 ms at p50, and spikes past 50 ms. The
    fixes are under way (`work/perf2`; [PERFORMANCE.md](PERFORMANCE.md), "The whole world awake").

## What changed on 17–18 October
- **Parity:**
  - Thistle Tea's 8,850 tests were mapped to wvs tests ([parity/README.md](parity/README.md)).
  - Five rounds of stages ported them, fixing wvs where it differed, checked against mangos-classic.
  - The matrix went from 21% to 67.5% covered or accepted. Where Thistle Tea isn't 1.12, wvs keeps mangos's behaviour
    and the row says why (46 rows).
- **The kobold's reach:** fixed.
- **Robustness:** the supervision above, packet guards, and chaos tests. wow_messages' crash bugs were guarded in wvs
  and fixed in the fork.
- **Performance:** idle CPU −77%, arena −55%, crowd −38%; the world loads ten times faster; Tracy reworked; the
  whole-world benchmark.
- **The client crashes:**
  - bad character looks;
  - updates naming objects the client had just forgotten;
  - corpses made before their owners went;
  - creature splines of fewer than three points.

  All are fixed, and the client check guards against their kind.
- **Code review:** four reviews ([review/](review/), [CODE_REVIEW.md](CODE_REVIEW.md)). About 60 fixes, among them:
  - item and money duplication through the buyback, trades and mail;
  - a self-recursing script crash;
  - bans that didn't hold;
  - dungeon saves lost after a wipe;
  - bank bags lost at login.
- **Your requests:**
  - fast and extended test suites;
  - configurable settings;
  - ghosts that walk on water and see only near their corpse;
  - a 400-yard battleground view distance;
  - Tracy;
  - the client-code harness;
  - the whole-world load test;
  - dead bodies that stay put, and `.spawn` with its creature's weapons and stats;
  - ahash's maps only, enforced by clippy.

## What's left
- **Parity:** 2,652 rows are still gap. The biggest areas:
  - the world's entities (players, creatures and objects in the world), about 590;
  - creature AI and scripts, about 340;
  - spells, about 490 (core and world);
  - the world's loaders, about 240;
  - combat, about 210;
  - pets and creatures, about 155.

  Round 5 is in. Some rows wait on a question, and some on a system wvs doesn't have, such as Scourge camps, the
  Lunar Festival's fireworks, or a movement-acknowledgement system.
- **Open questions:** 50 or so, in [SITREP.md](SITREP.md). Each says what wvs does until you answer.
- **The hard findings** ([CODE_REVIEW.md](CODE_REVIEW.md)):
  - a client claiming a transport skips the speed check;
  - groups aren't saved;
  - round-robin loot locks others out of a corpse;
  - a death that despawns its creature cuts its own death short;
  - the threat-list scans of every creature;
  - the rest are listed there.
- **Performance:** perf2's whole-world fixes are in (staggered timers, the awake set once an update, tiles read off
  the map's thread and let go when idle). Left: BVHs built on the map's thread, a tile cap for the whole world awake
  (about 13 GB), and Kalimdor with the navmesh unmeasured ([PERFORMANCE.md](PERFORMANCE.md)).
- **Flaky tests:** flaky1 fixed the ones that failed under load ([TESTING.md](TESTING.md), "Under load"). One
  remains: `creatures_spread_round_their_victim`, about 1 run in 60–80 of the fast suite (Stratholme's bosses were
  a test looking before its boss was seen).

## How the work was run
- **Isolation:** each stage was an agent on its own git worktree and branch (`<worktree>/<stage>`). It worked from a
  brief in `runs/prompts/` and recorded its matrix rows in a CSV.
- **Merging:** each finished branch went into `master` only after the fast suite, the extended suite and clippy
  passed on the merge, and the push followed.
- **Records:** decisions and questions went into [DECISIONS.md](DECISIONS.md) as stages reported them.
- **Lessons for next time:**
  - **Memory:** too many agents compiling at once exhausted it. Cap each agent's memory and keep to four or five
    running at once.
  - **Sessions:** long agent sessions sometimes end without a report. Their commits carry what they did.
  - **Overlap:** agents working the same code from two sides (5a and 4d on purchases into a slot) write it twice.
    Give stages areas that don't share files, or merge more often.
