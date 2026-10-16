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
cargo test --lib                    # the harness tests run the real sessions and maps
cargo build --release --bin wow_vanilla_server
```

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
  outbound queue, session and map. Services that span maps go behind their own short locks.
- **The store** (`src/game/store.rs`): `characters.sqlite`, behind one thread, so reads see earlier saves. The schema
  is versioned (`MIGRATIONS`, `PRAGMA user_version`); a character's extra state is JSON whose fields all default.
  Saves at logout, every `save_interval_secs`, and for everyone on SIGTERM or Ctrl-C.
- **Game data** (`src/game/data/`): vmangos's `vmangos.sqlite` and the client's `dbc.sqlite`, loaded once from
  `[world] data_dir` into immutable tables. A template's row is its latest patch up to 10; a spawn is kept if its
  patch range has 10. Without the databases the server runs on `wow_world_base` and `wow_items`.
- **Navmesh** (`map/nav.rs`): a rustigator bake from `[world] nav_dir`, a map's tiles loaded as first needed. Zones
  and areas come from it, and with them the free-for-all flag of an arena.

## Conventions

- **Packets out are encoded once** and the frame shared: `conn::encode`, `Map::send_to_observers`. No per-viewer
  re-encoding.
- **Packets wow_messages types too strictly are encoded by hand** (`data::items::Body`): the item and game object
  query responses. Everything else uses wow_messages' types, so peon (which reads with wow_messages) reads it.
- **Values blocks always carry the object type**, and a GUID field's two halves are dirtied together: wow_messages
  can't read a block without them.
- **GUIDs are typed** (`src/game/guid.rs`): players by counter, items `0x4000`, creatures `0xF130` with their entry,
  game objects `0xF110`.
- **Never panic on client data**: look things up fallibly, log at `debug`, drop the packet.
- **Never block a map**: no `.await`, no file or database work in a handler; the store is fire-and-forget.
- **Tests go through the protocol** (`src/game/harness.rs`): connect clients to a realm, send real messages, and
  read what comes back with wow_messages.

## Configuration

`config.toml` (every key optional; an unknown key is an error), and the environment:

| Var | Purpose |
|---|---|
| `WOW_REALM_ADDRESS` | `host:port` in the realm list. Defaults to `localhost:8085`. |
| `WOW_TRACY` | `1` starts the Tracy profiler. |
| `RUST_LOG` | the log filter. |

## Pull-request checklist

- [ ] `cargo clippy --all-targets` clean, `cargo test --lib` passing.
- [ ] A harness test for each packet a change handles.
- [ ] A schema change is a new entry in `MIGRATIONS`, never an edit to an old one.
- [ ] Phase work updates [docs/ALIGNMENT_PLAN.md](docs/ALIGNMENT_PLAN.md)'s progress.
