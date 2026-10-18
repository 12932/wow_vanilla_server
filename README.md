# wow_vanilla_server

A WoW vanilla (1.12.1) server in Rust, on Tokio: an actor per map, packets handled as they arrive, a 20 Hz map
update, and vmangos's world. Load-tested with [peon](https://github.com/12932/peon).

A development project, not a production server; use vmangos or cmangos for a public realm. Where it stands against
Thistle Tea, and the plan to close the gap, is in [docs/ALIGNMENT_PLAN.md](docs/ALIGNMENT_PLAN.md); where it still
differs, in [docs/THISTLE_TEA_DIFFERENCES.md](docs/THISTLE_TEA_DIFFERENCES.md).

## Quick start

You need [Rust](https://rustup.rs), [Git LFS](https://git-lfs.com) (the world's databases are in it) and a WoW
1.12.1 client.

```sh
git lfs install
git clone https://github.com/12932/wow_vanilla_server
cd wow_vanilla_server
cargo run --release
```

The first start offers to build the navmesh and the terrain from your WoW client: answer Y, say where the client is,
and the server starts when they are done. All 36 maps take about 9 GB and, on 16 threads, three minutes;
the Eastern Kingdoms alone (Gurubashi Arena, Stormwind, the Undercity) about 3 GB and one minute. The terrain takes
seconds. To build without starting the server, or again later (more maps, another client):

```sh
cargo run --release --bin wvs-setup
cargo run --release --bin wvs-setup -- --yes --client "C:\Games\WoW 1.12" --maps azeroth    # in a script
```

The setup asks where the client is. Any of these will do, typed, pasted, or dragged onto the window:

- the client's folder, the one with `WoW.exe` and `Data` in it;
- `WoW.exe` itself, or a renamed `.exe` beside `Data` (its version is read);
- the `Data` folder, or any file in it: a copy of `Data` alone is enough.

A wrong answer is told what is wrong and asked again, and an empty line stops. The next time, the client used last is
offered: Enter takes it. A client the server can't use is turned away with the reason: The Burning Crusade or later,
or one not patched to 1.12.1. Another vanilla build is offered with a warning.

`--client <path>` (or `WOW_DIR`) gives it the client without asking, and `--yes` needs one. `--maps` takes `all`,
`azeroth`, or map names as the client names them (`Kalimdor,PVPZone03`), `--threads N` the cores to build with. Ctrl-C stops a build between tiles, keeping
what is finished, and the next run carries on from there. Without a terminal (a service, a script) the server asks
nothing: it logs how to build what is missing, and starts without it.

It listens on `:3724` (auth) and `:8085` (world). Point a 1.12.1 client's `realmlist.wtf` at the host. By default
an unknown username gets an account at its first login, with the username as its password (`[accounts] creation =
"auto"`): fine for development, not for the internet. With `creation = "existing"`, only accounts an administrator
made with `.account create` may log in. GM levels come from `.account gm` or the config's `[accounts] gm` table;
`everyone_gm = true` makes every account an administrator, as peon's runs need.

The world comes from three directories under `data/`, beside where the server runs (`cargo run --release` from
the checkout looks in the checkout's `data/`):

| Directory | Holds | Without it |
|---|---|---|
| `data/db` | `vmangos.sqlite` and `dbc.sqlite`, vmangos's database and the client's DBCs as SQLite (Thistle Tea's export). They ship with the repository, in Git LFS: install it before cloning, or run `git lfs pull` after (the server says so if they are LFS pointers) | the world is empty: no creatures, objects, spells or quests |
| `data/nav` | the navmesh, a rustigator bake (`Azeroth.map` and the rest), made by the setup from your client | creatures walk straight lines |
| `data/terrain` | the terrain tiles (`MMMMRRCC.map`, as mangos-zero's extractor writes them), made by the setup from your client | liquids outside WMOs have no kind (the deep sea doesn't tire swimmers, lava doesn't burn) |

Elsewhere, say so in `config.toml` (the setup builds into these too):

```toml
[world]
data_dir = "/path/to/db"
nav_dir = "/path/to/bake"
terrain_dir = "/path/to/terrain"
```

The tests that read the real world find it the same way, or under `WVS_DATA` if it is set; without it they pass
without running. How the setup came about is in [docs/DATA_SETUP_PLAN.md](docs/DATA_SETUP_PLAN.md).

## Configuration

`config.toml` next to the working directory; every key is optional and the defaults are in `src/config.rs`. An
unknown key is an error. The environment (or `.env`) sets `WOW_REALM_ADDRESS` (the realm list's `host:port`),
`WOW_TRACY=1` (the Tracy profiler) and `RUST_LOG`.

Characters are kept in `characters.sqlite` (`[world] characters_db`), saved at logout, every five minutes, and for
everyone on SIGTERM or Ctrl-C.

## GM commands

In chat, by GM level (1 moderator, 2 game master, 3 administrator). A player's `.` line is just chat.

| Command | Level | Does |
|---|---|---|
| `.gps` | 1 | Where you are: map, position, zone, area |
| `.online`, `.maps` | 1 | Who is online; each map's counters |
| `.save`, `.kick <name>`, `.announce <text>`, `.gm [on\|off]` | 1 | |
| `.go [xyz] <x> <y> <z> [map]` | 2 | Teleport |
| `.tele <place>` | 2 | Teleport to a place from vmangos's `game_tele` |
| `.speed <multiplier>` | 2 | Run speed |
| `.spawn <entry>` | 2 | Spawn a creature |
| `.die`, `.revive`, `.levelup [n]` | 2 | On the target, or oneself |
| `.arena start [countdown s] [max minutes]`, `.arena stop` | 2 | Gurubashi Arena's last man standing, with everyone alive on the pit's floor |
| `.arena status`, `.arena history` | 0 | The battle on, and the last champions |
| `.account create\|password <name> <password>`, `.account gm <name> <level>` | 3 | Accounts |
| `.ban <account> <minutes\|perm> [reason]`, `.unban <account>` | 3 | Bans (and kicks) |

## What it runs

The world of vmangos at patch 1.12: creatures with their AI, paths and spell lists; combat by vmangos's hit table;
spells and auras; items, loot, vendors, trainers, flight masters and quests; death and graveyards. Socially:
whispers, channels, friends and ignores, `/who`, groups and raids, guilds and charters, mail, the auction houses,
trade and duels. Dungeon and raid copies, Warsong Gulch, boats and zeppelins, seasonal events and weather.
Gurubashi Arena's Booty Run, and its last man standing: a countdown, the announcer calling every kill, a ring of fire
closing on the centre, and the champion crowned. What
is done and what is left, phase by phase, is in the plan's progress section.

## Development

```sh
cargo clippy --all-targets    # must be clean
cargo nextest run --lib       # the fast suite
cargo extended                # every test, on the real world (WVS_DATA); see docs/TESTING.md
```

[AGENTS.md](AGENTS.md) describes the design and the conventions.

## Credits

Built on [@gtker](https://github.com/gtker)'s [`wow_messages`](https://github.com/gtker/wow_messages) and
[`wow_srp`](https://github.com/gtker/wow_srp), and [rustigator](https://github.com/12932/rustigator) for the navmesh.
The world is [vmangos](https://github.com/vmangos/core)'s.
