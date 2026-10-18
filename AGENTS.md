# AGENTS.md

Guidance for AI coding assistants working in this repo.

## What this is

A WoW vanilla (1.12.1) server in Rust, being brought up to what Thistle Tea does: the plan, its phases and its
progress are in [docs/ALIGNMENT_PLAN.md](docs/ALIGNMENT_PLAN.md), and what Thistle Tea does is in
[docs/THISTLE_TEA_INVENTORY.md](docs/THISTLE_TEA_INVENTORY.md). The load tester is its own repository,
[peon](https://github.com/12932/peon).

## Quick commands

```sh
cargo check
cargo clippy --all-targets          # must be clean before a commit
cargo nextest run --lib             # the fast suite: every test that doesn't need the real world
cargo extended                      # the extended suite: every test, before a merge or a push
cargo build --release --bin wow_vanilla_server
cargo run --release --bin wvs-setup  # data/nav and data/terrain from a WoW client
```

Tests run under nextest, never `cargo test`. The fast suite takes about 25 s on 16 cores. `cargo extended` is an
alias for `cargo nextest run --lib --profile extended --run-ignored all`; it takes about 3 minutes, and needs the
real world: `WVS_DATA` points at a `data/` with `db` (`vmangos.sqlite`, `dbc.sqlite`), `nav` and `terrain` (else the
checkout's `data/`, whose databases are Git LFS pointers until pulled). A test that reads the real world (through
`harness::real_data`, `harness::load_real` or `load::real_data()`) is marked `#[ignore = "extended"]`; unmarked, it
fails in the fast suite and says so. Details in [docs/TESTING.md](docs/TESTING.md).

## The design

There is no world tick.

- **Sessions** (`src/game/session.rs`): a task per connection runs the world handshake and the character screen, at
  once, against the character store. In the world, it routes the client's packets to its map. It switches its route
  before the client can act on a change: it sends `SMSG_LOGOUT_COMPLETE` itself, and hands a player to their new map
  before that map sends `SMSG_NEW_WORLD`.
- **Connections** (`src/game/conn.rs`): a reader task decodes packets (and answers pings); a writer task encrypts and
  coalesces writes. Everything sends through an `Outbound`. A client over its byte budget is disconnected, not sent
  a partial world.
- **Maps** (`src/game/map/`): each map is an actor (a task) that owns its players, creatures, game objects, grid and
  navmesh, with no locks. Packets are handled as they arrive (`handlers.rs`). A 20 Hz update (`Map::update`) runs
  what time drives, works out visibility, and flushes changed fields.
- **Update fields** (`src/game/fields.rs`, `src/game/update.rs`): each object's fields are a flat array with dirty
  bits. A values block is encoded once and shared by every observer; a player's own client gets its private fields
  in a second block. Blocks are batched per observer per update; big batches are compressed.
- **Visibility** (`visibility.rs`): a 50-yard grid, an exact distance test at `[world] visibility_yards` (100), and
  a 10-yard margin before an object is forgotten. Worked out for an object when it has moved 8 yards, or arrived.
  A creature no player sees doesn't move.
- **Shared state**: `Registry` (`src/game/registry.rs`) has every character's name and everyone online, with their
  outbound queue, session and map. Services that span maps go behind their own short locks, in `Realm::social`
  (`src/game/social/`): channels, groups, friend and ignore lists, guilds and charters, mail, auctions, instances,
  battleground queues, events and weather. They reach a player elsewhere through the registry, and change what a
  map owns with `MapHandle::call`. Mail, auctions, instances, battleground invitations, events and weather are swept
  every five seconds (`social::sweep`).
- **Copies** (`src/game/instances.rs`): a dungeon, raid or battleground copy is a map actor keyed by its map and an
  instance id; `Map::teleport` picks the copy. Battleground queues are in `src/game/battlegrounds.rs`, a match's
  rules in `map/battleground.rs`.
- **Transports** (`src/game/transports.rs`): boats and zeppelins on their paths, moved between maps by a clock of
  their own; every player in a map knows them (`Map::globals`).
- **Accounts** (`src/accounts.rs`): SRP verifiers, GM levels and bans in the store's file, on a connection of their
  own, shared by the login server and the world. GM commands are gated by level (`gm::needed_level`).
- **The store** (`src/game/store.rs`): `characters.sqlite`, behind one thread, so reads see earlier saves. The schema
  is versioned (`MIGRATIONS`, `PRAGMA user_version`); a character's extra state is JSON whose fields all default.
  Saves at logout, every `save_interval_secs`, and for everyone on SIGTERM or Ctrl-C.
- **Game data** (`src/game/data/`): vmangos's `vmangos.sqlite` and the client's `dbc.sqlite`, loaded once from
  `[world] data_dir` into immutable tables. A template's row is its latest patch up to 10; a spawn is kept if its
  patch range has 10. Without the databases the server runs on `wow_world_base` and `wow_items`.
- **Navmesh** (`map/nav.rs`, `map/tiles.rs`): a rustigator bake from `[world] nav_dir`. Its tiles are read off the
  map's thread, ahead of the players, and let go once idle (`[world] tile_unload_secs`); until a tile lands, queries
  answer as if there were no navmesh. Tests read them at once (`RealmConfig::tiles_in_background` off). Zones and areas
  come from it, and with them the free-for-all flag of an arena.
- **The data setup** (`src/setup/`, also the `wvs-setup` binary; `src/server.rs` starts both): asks where the
  player's WoW client is and checks it (`discover.rs`), works out what of the navmesh and terrain is left to build (`plan.rs`, with the
  estimates in `maps.rs`) and builds it through a `Builder`: rustigator's, or `Fake` in the tests. The questions and
  bars are `ui.rs`. The server offers it at start-up only on a terminal (`setup::at_startup`).

## Conventions

- **Packets out are encoded once** and the frame shared: `conn::encode`, `Map::send_to_observers`. No per-viewer
  re-encoding.
- **Packets wow_messages lays out unlike 1.12 are encoded by hand** (`data::items::Body`), as mangos-zero lays
  them out: the item and game object queries, the loot window, `SMSG_CAST_RESULT`, channel notices, friend status,
  `/who`, party member stats, raid icons and the battlefield status. Everything else uses wow_messages' types, so
  peon (which reads with wow_messages) reads it. mangos-zero's source is the reference for 1.12's layouts.
- **Values blocks always carry the object type**, and a GUID field's two halves are dirtied together: wow_messages
  can't read a block without them.
- **GUIDs are typed** (`src/game/guid.rs`): players by counter, items `0x4000`, creatures `0xF130` with their entry,
  game objects `0xF110`.
- **Never panic on client data**: look things up fallibly, log at `debug`, drop the packet. (A panic that slips
  through costs one event, update step or creature turn; see [docs/RELIABILITY.md](docs/RELIABILITY.md).)
- **Never block a map**: no `.await`, no file or database work in a handler; the store is fire-and-forget.
- **Tests go through the protocol** (`src/game/harness.rs`): connect clients to a realm, send real messages, and
  read what comes back with wow_messages.
- **The fast suite stays fast**: a test that needs the real world is `#[ignore = "extended"]`; one that waits seconds
  in real time skips the map's clock instead where it can. A long walk is split into tests nextest runs at once.
- **Don't race the map**: a harness test queries a map whose own update runs on real time. Wait for what you check,
  with a short poll up to a limit, or count the map's updates (`m.stats.updates`). Never assume an update has or
  hasn't happened between two queries. Every test that failed only under load was this.
- **The client mustn't crash**: a change to a packet or a create runs the client check (`tools/client_check/`,
  [docs/TESTING.md](docs/TESTING.md)), which runs `WoW.exe`'s own parsers on what wvs sends.

## Where things are written down

- [docs/STATE.md](docs/STATE.md): where wvs stands, how to run, test and measure it, and what's left.
- [docs/SITREP.md](docs/SITREP.md): the open questions for the user, by subject. [docs/DECISIONS.md](docs/DECISIONS.md)
  has each in full, with every decision made and why.
- [docs/CODE_REVIEW.md](docs/CODE_REVIEW.md) and [docs/review/](docs/review/): what the reviews fixed and left.
- [docs/parity/](docs/parity/): the matrix of Thistle Tea's tests; [docs/THISTLE_TEA_DIFFERENCES.md](docs/THISTLE_TEA_DIFFERENCES.md).
- [docs/RELIABILITY.md](docs/RELIABILITY.md), [docs/PERFORMANCE.md](docs/PERFORMANCE.md),
  [docs/PROFILING.md](docs/PROFILING.md), [docs/TESTING.md](docs/TESTING.md), [docs/CONFIG.md](docs/CONFIG.md).

## Configuration

`config.toml` (every key optional; an unknown key is an error), with the environment over it, as described in
[docs/CONFIG.md](docs/CONFIG.md). Each key is declared once, in a `settings!` block in `src/config.rs` (its type,
default, range, whether a reload changes it, its mangos name and its meaning); `config.example.toml` is generated from
those (`cargo run --bin wow_vanilla_server -- --print-config --defaults > config.example.toml`), and a test fails when
it drifts. Maps read the live settings as `map.cfg` (taken once an update); elsewhere, `realm.config()`. A tunable
that is policy, not 1.12's rule, goes there rather than in a constant.

| Var | Purpose |
|---|---|
| `WVS__SECTION__KEY` | any config key, over the file: `WVS__RATES__XP_KILL=2`. An unknown one is an error. |
| `WOW_REALM_ADDRESS` | `host:port` in the realm list. Defaults to `localhost:8085`. |
| `WOW_TRACY` | `1` starts Tracy: a frame per map update, a zone per step, packet and save ([docs/PROFILING.md](docs/PROFILING.md)). |
| `RUST_LOG` | the log filter. |
| `WVS_DATA` | the tests' real world: a `data/` with `db`, `nav` and `terrain` (the extended suite's). |

## Pull-request checklist

- [ ] `cargo clippy --all-targets` clean, `cargo nextest run --lib` and `cargo extended` passing.
- [ ] A harness test for each packet a change handles.
- [ ] A schema change is a new entry in `MIGRATIONS`, never an edit to an old one.
- [ ] Phase work updates [docs/ALIGNMENT_PLAN.md](docs/ALIGNMENT_PLAN.md)'s progress.
