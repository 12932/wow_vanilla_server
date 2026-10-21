# Where wow_vanilla_server stands (2026-10-21)

wvs is a WoW 1.12.1 server in Rust. It is being aligned with [Thistle Tea](https://github.com/pikdum/thistle_tea),
an Elixir server, as the behaviour to match, with mangos-classic as a second reference for what 1.12 does. This page
covers how it's built, how to run, test and measure it, what changed on 17–21 October 2026, and what's left.

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
  - a map that keeps panicking is rebuilt, its players saved.
- **The update's hooks:** the update looks only at creatures a player sees or that are fighting, and at what
  changed. New code goes through the hooks ([PERFORMANCE.md](PERFORMANCE.md)); debug builds assert it.
- **Packet order:** a client gets an object's create before anything that names it. A client the map has queued
  creates for is held (`Outbound::hold`): what it is sent waits, in order, behind its creates and forgets. It is
  held as an object is introduced to it, as one comes into the map near it, and as it arrives or lands from a
  teleport. It is let go at the end of the event that held it (`Map::after_events`, which sees what the event
  brought in), or at the update's "held" step; its values follow in the update's batch. Forgets still go first:
  a packet about an object sent before the object was forgotten, in the same update, reaches the client after the
  forget, and the client drops it.
- **Sight:** 100 yards on the continents, measured on the ground; a flight master's passenger sees 400
  (`[visibility] flight_yards`), each observer by its own, and sees the map's distance again on landing.
- **Boats and zeppelins:** each sails its vmangos keyframes on the server's clock. At a crossing it goes to the
  other map first; then each passenger follows it (`SMSG_TRANSFER_PENDING` with the ship, `SMSG_NEW_WORLD` with
  their place on deck, as mangos's `TeleportTo`), and gets the ship's create before their own, aboard. A
  passenger's movement on the wire is 1.12's (flag `0x0200_0000`, the ship's whole GUID, no timestamp), as the
  wow_messages fork's vanilla `MovementInfo` and `MovementBlock` read and write it.

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
- **The fast suite**, `cargo nextest run --lib`: about 3,140 tests in 24 seconds on 32 cores, none needing the real world.
- **The extended suite**, `cargo extended`: every test, about 3,860 (721 of them marked extended), in about 3½ minutes. It loads the real world from
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
  - set `WOW_TRACY=1` and connect Tracy v0.14.1 (the protocol `tracy-client-sys` 0.30.0 speaks);
  - you get a frame per map update, a zone per step, packet and transaction, and plots once a second;
  - it costs nothing measurable when off.
- **Without a GUI:** Tracy's `tracy-capture` and `tracy-csvexport`.
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

## Guards and Jaina (19 October)
- **Guards as mangos-classic calls them** (`map/guards.rs`, replacing vmangos's posts): a guard each time a civilian
  takes a victim, at a reachable point within 30 yards, gone five minutes out of combat or at its death, wandering
  about where it came meanwhile. No charges, no cooldown; no guard roused where the table names no town.
- **Each town's alarm** (`[guards]`, [CONFIG.md](CONFIG.md)): the guards called there lately, halved for each five
  minutes without a call. From 20, guards come with Avoidance (5% per ten, to 50%); from 40, a call brings two of
  the capital's elite; from 60 in Stormwind, Jaina Proudmoore comes from Theramore (`jaina.rs`), fights, goes home a
  minute after the fight, and dead, is back in Theramore a day later. Each threshold crossed, and each of her moves,
  is logged at `info`.
- **The Stormwind Elite Guard** (`map/elites.rs`, entry 101756, defined in code over the data): the Royal Guard's
  looks and side at level 62, elite, with the Tower Point Warmaster's multipliers (health 20, damage 13.5) and fight,
  its Charge going for a healer in sight (one who healed in the last 5 s first), else its victim; a sword in each
  hand (the Alliance PvP blade and the Grand Marshal's Swiftblade), swinging both. Two come as Stormwind's alarm
  rises through 50 (`elite_from`), once a crossing, as guards come and go; four come 40 yards around Jaina, walk to
  her side fighting what they meet, guard her and leave with her.

## Rounds 6 to 8 (19–20 October)
Thirty-odd agent agents, one area each, took the parity matrix from 67.5% to 87.8% (7,317 of 8,330 applicable
Thistle Tea tests matched or accepted), with the user's evening list, review 2's M2 and M3, and the navmesh's
erosion (format 13) on the way. Every stage's fixes and calls are in [SITREP.md](SITREP.md), "Rounds 6 to 8".
Among what players notice: a poison or a trap's disarming no longer breaks Stealth; engineering guardians have
their cooldowns; Shadow Trance and Netherwind Focus are spent; spells' procs go off once a target; forced movements
wait for the client's acknowledgement; a fast reconnect no longer loads a character before its last save; the bank
works only at its banker; the Lunar Festival's fireworks, cheer speakers and the Ghost Magnet are in; Alterac
Valley's war riders, Balinda and the rituals are scripted; two of a group entering a dungeon at once share a copy,
and a shutdown saves a player between maps.

## Rounds 9 and 10, and the questions (20–21 October)
Round 9 (six stages: the player, spells, the core, entities, creatures, the world) and round 10 (eight: 10a to 10f,
10h, 10m) took the matrix from 87.8% to 94.2%: 7,824 of 8,304 applicable Thistle Tea tests covered (7,741)
or accepted (83). Of the rest, 302 are kept as mangos-classic where Thistle Tea isn't 1.12 (`wrong-for-1.12`), and
178 are gap. Among what changed: proc origins and
cast-end procs by mangos, the creature stat model, owned creatures' EventAI and a pet's fight as its master's, the
wild-summon tap, formations led on past a dead leader, the Scourge camps, healing by school, PvP timers in a fight.
Alongside them: `fix/flaky` made nine load-flaky tests deterministic, `fix/lowsa` took review 5's lows, and
`fix/entity` gave each viewer its own view of a unit (`map::views`). Then `fix/questions` (0.1.67) settled the
open questions by mangos-classic (#1, #8, #9, #12, #16, #19/#44, #23, #28, #32, #36, #40, #45, #48, #52), the rest
confirmed.

## Round 11, the matrix closed (21 October)
The last 178 gaps were triaged by six agents (82 already done, needing a test; 38 a note; 53 a fix; 5 your call),
then closed by ten Opus agents on their own worktrees (`parity/11a` to `11j`), merged one at a time with the fast
suite after each and the extended suite every few: 96.0% covered or accepted, the rest `wrong-for-1.12`, none gap
(0.1.77). What changed is in [ALIGNMENT_PLAN.md](ALIGNMENT_PLAN.md), "The parity matrix closed"; your calls are
decision 71.

## What's left
- **Parity:** no row is gap (round 11, 2026-10-21). What's left are the workers' calls in [SITREP.md](SITREP.md)
  ("Round 11's questions"), and the engineering items they named: the per-update create cache can serve a create
  made earlier in the same update (`Map::public_create`); peon must acknowledge the teleport for an accepted
  resurrection to finish; instant spells still report misses only in `SMSG_SPELLLOGMISS`. The non-matrix leftovers
  of earlier rounds are in `<runs>/prompts/_carryover.md`.
- **The fork (12932/wow_messages):** its 1.12 `HitInfo` is flags (with the debug trailer under 0x1),
  `SMSG_AUCTION_COMMAND_RESULT`'s inventory error a u32, `CMSG_BATTLEFIELD_JOIN` the map, the instance and the group
  byte, and a spell's targets the client's (a corpse's guid before the locations), so wvs encodes and reads them
  with its types.
- **Open questions:** settled by mangos-classic on 2026-10-21 (`fix/questions`), but for #4 (reporting wow_messages' crash
  bugs upstream) and six calls where wvs keeps Thistle Tea's or vmangos's behaviour over mangos's, in [SITREP.md](SITREP.md).
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
- **Flaky tests:** flaky1 fixed the ones that failed under load ([TESTING.md](TESTING.md), "Under load"). Two
  remain: `creatures_spread_round_their_victim`, about 1 run in 60–80 of the fast suite, and
  `tests_items3::the_elixir_and_the_fish_turn_their_eater_into_one_thing_each_time`, which failed once under load on
  2026-10-19 and not in 40 runs alone (Stratholme's bosses were a test looking before its boss was seen).

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
  - **Overlap:** agents working the same code from two sides (5a and 4d on purchases into a slot; 8h and 8j on the
    fireworks; 6d and 6h on each hand's weapon skill) write it twice. Give stages areas that don't share files, or
    merge more often.
  - **Merging untested:** on 2026-10-19 the machine ran short of memory and rounds 7 and 8 merged without the fast
    suite between them; two branches' changes broke each other's tests (7j's sizes against 7k's transforms) and two
    `match` arms shadowed each other silently. Run `cargo check` after each hand-resolved merge and the fast suite
    before each push; keep at most three agents up for the full extended suite.
