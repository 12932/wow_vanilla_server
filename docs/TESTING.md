# Testing

All the tests are in the library, and run under [nextest](https://nexte.st), never `cargo test`: each test is a
process of its own. They are in two suites.

| Suite | Command | What | Time (16 cores, debug) |
|---|---|---|---|
| Fast | `cargo nextest run --lib` | every test that doesn't need the real world (about 1,950) | about 30 s |
| Extended | `cargo extended` | every test (about 2,340), the real world's too | about 7 minutes at 8 threads, 13 at 4 |

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

## The client-emulation check

Several times a packet wvs sent, one that `wow_messages` (and so every harness test) read without complaint, has
crashed the real 1.12.1 client in its own handling of it: a creature's two-point spline copied as a count of −1
(`0x6194DD`), an update naming an object just forgotten, a look with no texture. `tools/client_check/` runs what wvs
sends through `WoW.exe`'s own parsing code, so such a packet fails a test before it reaches a client.

- **How.** `emu.py` maps `WoW.exe` (1.12.1, build 5875; in `tools/client_check/`) under the [Unicorn](https://www.unicorn-engine.org)
  CPU emulator and calls the client's handlers on a `CDataStore` laid out as the network layer hands one over. The
  client's bytes are only ever run inside the emulator, never natively. A handler that faults, reads past the
  packet, or builds an absurd array is reported with the client address, like a crash log.
- **What is stubbed.** The allocator and the clock; the object manager (finding, making and forgetting objects is
  modelled in Python); and the renderer and world — a create's post-init, and the main pass that re-applies each
  block to the renderer. The checks drive the *parse*: the create pass (which reads every block), the movement
  reader and its spline, and the spline copy into a unit, which is where the `0x6194DD` crash was; and, for a
  unit created on a spline, the client's own decision whether to move it along it (`0x618940`: only one going
  somewhere, 0x20FF, is put among the movers; a create with the spline alone left a flier frozen in the air).
- **The checks** (`check.py` routes a frame by opcode): `SMSG_UPDATE_OBJECT` and `SMSG_COMPRESSED_UPDATE_OBJECT`
  (the whole create pass, plus each create's movement run through the unit copy and the movers' decision,
  `movement.py`);
  `SMSG_MONSTER_MOVE` / `..._TRANSPORT` (`monster_move.py`, the spline reader a flyer's hand-built move goes
  through); `SMSG_SPELLLOGMISS` (`spell_log_miss.py`, the handler `0x5E7E00`, which must read the packet
  exactly: a target is its GUID and a byte, two floats more when the packet's own byte is set); and `SMSG_ATTACKERSTATEUPDATE` (`attacker_state.py`, the
  melee handler `0x6255B0` up to the end of its parse, `0x625C60`: it must read the blow exactly, its hit info a set of
  flags of which only 0x1 changes the layout, and hold at most five damage lines); and `SMSG_INITIAL_SPELLS` and `SMSG_ACTION_BUTTONS` (`initial_spells.py`, the handlers `0x5E6510` and `0x5E6680`: each read exactly, and the spell list and buttons the client keeps, which it builds its spellbook and bars from at each create of its own player, the packet's). Adding a check for another handler means finding its address (the registration table at `0x465140` /
  `0x6035C0`, or `tools/client_check` notes; each handler is registered as `mov edx, handler; mov ecx, opcode;
  call 0x5AB650`, so the opcode's `B9` and its four bytes find it), driving it from a `Client`, and stubbing what it
  reaches outside the parse.
- **Its limits.** It drives parsing, not the renderer or the world, so it catches faults, over-reads, absurd counts
  and the spline copy, and whether a unit on a spline is moved along it, not a field wrongly applied. It does not model a fully initialised object, so a self-player
  create's private field groups are read short of the packet's end; an under-read is therefore a note, not a
  failure (each packet is its own buffer, so trailing bytes are harmless). The main-pass per-type apply is not run,
  nor is the field-group flush (`0x4667A0`) — the client's copy of a values block's changed field groups from the
  object's flat value array into its per-group renderer storage. That copy's offsets and byte counts come from the
  client's globals, never the packet, so it can reveal no packet fault, and it rests on the storage and descriptor
  list that `ClntObjMgrCreate` (`0x466e00`, stubbed) and the object's constructor build at the create, which the
  model does not: for an object created in this same packet it would walk an uninitialised list with a wild count
  and overrun (the `0x466816` report, which was the model's, not the client's). Stubbing it lets the real shape
  through — another player's create followed by a values block for them in one packet, what a ghost rising in view
  of a stranger gets — while the values parse above it (the mask read, bounded at 41 words, and the per-field reads
  off the packet) still catches a real fault, as the captured mask-overrun crashes do. A values block for an object
  of an earlier packet isn't run at all (the model knows only the packet's own objects).

### Running it

```sh
# Prove it against the spline crash (no outside files needed):
uv run --python 3.14 --with unicorn --with pefile python -I tools/client_check/selftest.py

# Check a frame capture, or a packet carved from a dump (opcode-first):
uv run --python 3.14 --with unicorn --with pefile python -I tools/client_check/check.py capture.bin
uv run ... python -I tools/client_check/check.py --raw packet.bin
```

A frame capture is server frames back to back, each `[size u16 BE][opcode u16 LE][body]`, as the writer task frames
one. `crash-analysis`' `upd.py` and friends carve a packet out of a `.dmp` for `--raw`.

### The extended test

`tests_client_emu::the_packets_wvs_sends_pass_the_client` plays out a scenario with movement in it — flyers on
their paths, a spell missing one every way one can, a crowd of players coming into view at once (so a create batch
arrives compressed), one leaving — keeps
every frame its clients receive (`Client::captured`), and runs the checker over them, failing if the client would.
It is `#[ignore = "extended"]` (it emulates x86 through `uv`, so it is slow) but needs no real world: it builds its
own. Without `tools/client_check/WoW.exe` or `uv` it fails loudly rather than passing quietly.
`a_corpse_and_its_bones_pass_the_client` does the same for a corpse, the bones it turns to at a reclaim, and bones
whose owner has logged out, told to a newcomer. `a_ghost_rising_before_a_stranger_passes_the_client` drives the
create-then-values shape on its own: a living stranger loses sight of a player's ghost and is told of her afresh
when she rises, her create and a values block for her in one packet. `tests_p8_movement_client::a_melee_blow_of_every_kind_passes_the_client`
runs a blow of each outcome both ways, and the flag combinations a fight can't make on its own (an off hand's, resisted,
absorbed), through the melee parse; the harness's client reads each too, its hit info a set of flags as the fork has
it.

## What the tests' realm changes

`harness::test_config` is the realm's defaults but for what the tests want quick (logouts, mail, a battleground's
start) and one rule: `[players] always_max_skill_for_level` is on, so a test's players have their weapon skills and
Defense at their caps at login and at every level a test gives them, where a live realm starts a new weapon skill at
1 (mangos's `AlwaysMaxSkillForLevel`). A skill learned mid-test is at 1 until the next level, as mangos has it. A test
of the skills' own rise sets the value it starts from (`skills::set_value`), or turns the rule off for its player
(`p.skills_maxed = false`) or its realm (`tests_rules_skills::live_realm`).

A player's fight is looked at for its end every 1.2 s (`combat::COMBAT_CHECK`). A test of it moves the map's clock in
a query (`m.now += ...`, then `combat::update_players`), or makes the look due (`next_check`). A combat flag a test
sets by hand, without `combat::enter`, is never looked at, so it stays until the test takes it off.

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

## The fight record

A fight seen in game (bots through a wall, a unit afloat, a druid that can't keep still) is gone over afterwards from
the **fight record** (`map/record.rs`, decision 105): what a map copy believes and tells its clients, a line of JSON an
event, a file per copy. It is off unless asked for:

- `[record] maps = [30]` records each copy of those maps from its start, into `[record] dir` (`record/`), as
  `map<id>-<copy>-<time>.jsonl`, sampling every `[record] every_ms` (100);
- `.record on [name]` (a game master's) records the copy they are in, `.record mark <text>` puts a mark in it (an
  attempt's start, its pull, its end: the file splits into attempts by them), `.record` says how it goes, `.record
  off` ends it;
- peon's `peon run <scenario> --record` sets the first for the scenario's map, its bots say the marks, and it writes
  each bot's own record beside it (peon's README, "Recording a run").

Each line has `t`, the Unix time in milliseconds (the bots' records' clock, to join them by), and `e`, what it is. Lines
come in the order written: a spline's `t` is when it began, up to a sample before the lines about it. GUIDs
are hex (`g`, and `by`, `tg`, `who` for the others in it); places are `[x, y, z]`; `sp` a spell's id.

| `e` | when | fields |
|---|---|---|
| `start`, `end` | the file's first and last | `map`, `instance`, `every_ms`, `why` (`config`, `gm`), `wvs` (its version) |
| `unit` | a unit, the first time it is written | `kind` (`player`, `creature`, `pet`), `entry`, `name`, `class`, `race`, `level`, `rank`, `faction`, `owner` |
| `pos` | each unit's sample (players, and the creatures awake), once it changed, or a second on | `p`, `o`, `fl` (its movement flags, every bit), `sp` (its speed as it moves now), `hp`, `mhp`, `pw`, `mpw` (its own power), `tg` (a player's selection, a creature's victim), `dead`, `dz` (its height over the ground: the navmesh's nearest surface under it, its real geometry's), `form`, `mount`, `stand`, `evade`, `spline` (the server's move under way) |
| `gone` | a unit left the copy | |
| `move` | a client's movement packet the server took | `op`, `p`, `o`, `fl`, `ct` (the client's time), `fall`, `dz` |
| `spline`, `halt` | a server-driven move begun (a chase, a way home, a charge, a fear, a fall), and ended | `id`, `kind`, `pts` (its whole way), `sp`, `ms`, `run`, `fly`; `halt`'s `p` and `early` (before its spline's end) |
| `cast`, `go`, `fail`, `channel`, `stop` | a cast begun, gone off, refused or failed (`why`, `why_s`), a channel begun, a cast or channel broken off (`how`) | `sp`, `tg`, `at`, `ms`, `trig` (a proc's or an aura's) |
| `aura` | an aura on (`on` 1, `refresh`) or off (`expired`) | `g` (the bearer), `by`, `sp`, `n` (stacks) |
| `dmg`, `out`, `heal` | damage to health (`n`, `left`, `sc`, `per`), how a blow or spell landed (`out`: hit, crit, miss, dodge, parry, block, glance, crush, resist, immune, evade, absorb, reflect; `sp` 0 a swing; `abs`, `res`, `blk`), healing (`n`, `over`, `crit`, `per`) | `by`, `g`, `sp` |
| `death` | a unit killed | `g`, `by`, `sp` |
| `threat` | an elite's threat list as it changed, at each sample | `l`: `[[guid, threat], ...]`, the highest ten |
| `victim`, `evade`, `notice`, `charge` | a creature's victim changed (`from`, `to`), its evade began or ended (`on`, `p`), it noticed someone (`who`, `yd`, `how`: `aggro`, `script`, `assist`), a charge (`tg`, `pts`) | |
| `summon`, `tele`, `mark` | a creature summoned (`by`, `entry`), a teleport (`from`, `to`, `map`), a run's mark (`by`, `text`) | |

The map's thread pushes the events onto a list (`Map::record`, a closure not run while it is off), taken at the
update's last step (`record`, in `WVS_STATS_FILE`'s step times) with the samples (a unit's spline, stamped with its
start, its victim and its evade are looked at with its sample), and sent down a channel to a thread
of the recorder's own, which writes the lines. A unit's sample is written once it changed or a second on (its
keyframe). The ground under each line (`dz`) is the recorder thread's to work out, on a navmesh of its own (the
map's bake, opened at its first line, its tiles read as they are needed: memory beside the map's), looked up again
only once a unit moved; the map's thread looks up none (a column's heights cost some 3 us). Off, it costs a field's
test at each event.

Tests: `tests_record` (a recorded copy's moves, chase, cast, aura, blast, damage and death in its file with their
fields; none written for a copy not recorded; a GM's `.record on`, `mark` and `off`; and, extended, the heights over
the ground on the real bake: a player kept at the ring's height over Drek'Thar's sunken middle is a yard over its
floor). peon's `scripts/record.py` goes
over a run's record: its moves across walls (`peon record walls`, on the real navmesh), its units over and under the
ground, their jitter, falls, abilities and deaths by attempt, and a timeline to watch.
