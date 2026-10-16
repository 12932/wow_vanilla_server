# wow_vanilla_server

A WoW vanilla (1.12.1) server in Rust, on Tokio: an actor per map, packets handled as they arrive, a 20 Hz map
update, and vmangos's world. Load-tested with [peon](https://github.com/12932/peon).

A development project, not a production server; use vmangos or cmangos for a public realm. Where it stands against
Thistle Tea, and the plan to close the gap, is in [docs/ALIGNMENT_PLAN.md](docs/ALIGNMENT_PLAN.md).

## Quick start

```sh
cargo build --release --bin wow_vanilla_server
./target/release/wow_vanilla_server
```

It listens on `:3724` (auth) and `:8085` (world). Point a 1.12.1 client's `realmlist.wtf` at the host. Any username
is created at login (password = username) and every account may use GM commands: fine for development, not for
the internet.

For the world, point `config.toml` at vmangos's database and the client's DBCs as SQLite (Thistle Tea's export:
`vmangos.sqlite` and `dbc.sqlite` in one directory), and at a rustigator bake for the navmesh:

```toml
[world]
data_dir = "/path/to/db"
nav_dir = "/path/to/bake"
```

Without them the world is empty and creatures walk straight lines.

## Configuration

`config.toml` next to the working directory; every key is optional and the defaults are in `src/config.rs`. An
unknown key is an error. The environment (or `.env`) sets `WOW_REALM_ADDRESS` (the realm list's `host:port`),
`WOW_TRACY=1` (the Tracy profiler) and `RUST_LOG`.

Characters are kept in `characters.sqlite` (`[world] characters_db`), saved at logout, every five minutes, and for
everyone on SIGTERM or Ctrl-C.

## GM commands

In chat:

| Command | Does |
|---|---|
| `.go [xyz] <x> <y> <z> [map]` | Teleport |
| `.tele <place>` | Teleport to a place from vmangos's `game_tele` |
| `.gps` | Where you are: map, position, zone, area |
| `.speed <multiplier>` | Run speed |
| `.spawn <entry>` | Spawn a creature |
| `.online`, `.maps` | Who is online; a map's counters |
| `.save`, `.kick <name>`, `.announce <text>` | |

## Development

```sh
cargo clippy --all-targets    # must be clean
cargo test --lib
```

[AGENTS.md](AGENTS.md) describes the design and the conventions.

## Credits

Built on [@gtker](https://github.com/gtker)'s [`wow_messages`](https://github.com/gtker/wow_messages) and
[`wow_srp`](https://github.com/gtker/wow_srp), and [rustigator](https://github.com/12932/rustigator) for the navmesh.
The world is [vmangos](https://github.com/vmangos/core)'s.
