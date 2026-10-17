# Making the world's data: a plan

**Goal:** from a fresh clone, `cargo run --release` gives the whole world. When the navmesh or the terrain is
missing, the server finds the player's WoW 1.12.1 client, asks once, builds what's missing with a progress bar, and
starts. No Docker, no Nix, no C++ toolchain beyond what Rust already needs, on Windows, Linux and macOS alike.

## What the world needs, and where it comes from today

| Directory | Holds | Today | After this plan |
|---|---|---|---|
| `data/db` | `vmangos.sqlite`, `dbc.sqlite` | committed (Git LFS, 89f42be) | the same |
| `data/nav` | the navmesh: `<Map>.map`, `<Map>_XX_YY.nav`, `models/` | rustigator's `tools/replay build` (Rust) | built by the setup, in-process |
| `data/terrain` | MaNGOS z1.5 tiles, `MMMMRRCC.map` | Thistle Tea's `nix run .#terrain-data`, which runs mangos-zero's C++ `map-extractor` | a Rust extractor, in-process |

Both builds read only the client's `Data/*.MPQ`.

## Stage 1: a Rust terrain extractor

Port the part of mangos-zero's `map-extractor` (`-e 1`, maps only) that wvs reads (`map/terrain.rs`) into rustigator
(`rustigator-mapbuild::terrain`, beside the navmesh builder; it already reads WDTs, ADTs and DBCs):

- **Per ADT:** each cell's area id, the ground heights (V9/V8, or the flat form), holes, and the liquid: MCLQ's
  heights, its type mapped to MaNGOS's flags (water, ocean, magma, slime), deep water as mangos marks it, and the
  `LiquidType.dbc` entry where a WMO holds it.
- **Written** in the same z1.5 layout, so `Terrain::open` doesn't change.

Done when the Rust tiles match mangos-zero's field for field on the whole 5875 client: the 2,429 reference tiles we
have, and the copy of the client in rustigator's LFS as the golden input. wvs's `tests_terrain` pass on them.

## Stage 2: progress and resumption in rustigator's builds

- `build_map` and `build_bvh` take a progress sink: tiles done of the total (the WDT gives the total), with the map's
  name.
- Cancelling (Ctrl-C) stops between tiles. The header is already written last, so an interrupted map isn't taken as
  current, and the previous bake stays usable.
- A map that is already current (`map_files_exist`) is skipped, so a second run carries on where the first stopped.

## Stage 3: finding the client

A `setup` module in wvs (also `cargo run --release --bin wvs-setup` on its own):

- **Where to look, fastest first:**
  1. a path given (`--client <dir>`) or `WOW_DIR`;
  2. the Windows registry (`HKLM\SOFTWARE\WOW6432Node\Blizzard Entertainment\World of Warcraft`, `InstallPath`);
  3. common folders: `Program Files (x86)`, `C:\Games`, `D:\Games` and other drives' `Games`, the user's Desktop,
     Downloads and Documents; on Linux and macOS, `~/Games`, Wine, Lutris, Bottles and Proton prefixes,
     `/Applications`.
- **Then a full search,** if nothing is found or the user asks:
  - every fixed drive, skipping system and package folders (`Windows`, `ProgramData`, `$Recycle.Bin`, `node_modules`,
    `.git`, `target`);
  - a spinner shows the folder being searched and the count so far;
  - a time limit is offered, and Esc stops it.
- **What counts as a client:**
  - a folder with `Data/` holding 1.12's MPQs (`dbc.MPQ`, `terrain.MPQ`, `model.MPQ`, `texture.MPQ`, `wmo.MPQ`,
    `patch.MPQ`);
  - `WoW.exe`'s version 1.12.1.5875 read from its version resource, where `WoW.exe` is there.
- **What is turned away, with the reason:** TBC and later (`common.MPQ`, `expansion.MPQ`, `lichking.MPQ`), and other
  vanilla builds, which are offered with a warning.
- **Before trusting it:** rustigator's `preflight` (every map's files resolve) and the free disk space against the
  build's estimate.

## Stage 4: the interactive flow

```
Found World of Warcraft 1.12.1 (5875) at C:\Games\WoW 1.12 (3.6 GB)
Use it to build the navmesh and terrain? [Y/n] y
Build what?
> Everything: 36 maps, about 9 GB
  Eastern Kingdoms only (Gurubashi Arena, Stormwind, the Undercity): about 3 GB
  Pick maps...
Terrain   [####################] 2,429 / 2,429 tiles
Navmesh   [#########-----------] Azeroth 312 / 689 tiles   ETA 4m 10s
Done: data/nav (2.8 GB), data/terrain (258 MB) in 6m 02s.
Start the server now? [Y/n]
```

- `dialoguer` for the questions and `indicatif` for the bars: an overall bar, and one for each map.
- Non-interactive, for scripts: `--yes --client <dir> --maps all|azeroth|<names>`.
- Errors say what to do: no client found (where to point it), not 1.12.1, not enough disk, an unreadable MPQ (which
  one).

## Stage 5: in the server

- **At start-up, with a terminal:** if `data/nav` or `data/terrain` is missing, the server offers the setup and saying
  no starts it without them, as now: "No navmesh: creatures will walk straight lines. Find your WoW client and build
  it now? [Y/n]". Without a terminal (a service), it logs the hint and starts.
- **`data/db` holding Git LFS pointers** (a clone without LFS) is caught by name: "data/db/vmangos.sqlite is a Git LFS
  pointer: run `git lfs pull`", rather than SQLite's error.
- **The README's quick start becomes:** clone, `cargo run --release`, answer Y.

## Stage 6: the last of it

- **`build.rs`** passes `user32.lib` by MSVC's name, which breaks the GNU target; it should pass it only for
  `target_env = "msvc"` (`-luser32` otherwise).
- Timings for one map and for all 36 go in the README.

## Order and size

Stages 1 and 2 are in rustigator, and independent of each other; Stage 3 can start beside them. Stage 4 needs 2 and 3;
Stage 5 needs 4; Stage 6 closes it. Stage 1 is the largest: a careful port with a golden
comparison. The rest are small.
