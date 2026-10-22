#!/usr/bin/env python
"""Run wvs's packets through the 1.12.1 client's own parsing code.

    uv run --python 3.14 --with unicorn --with pefile python -I check.py FILE...

Each FILE is a frame capture: server frames back to back, each as the writer
task lays one out before it encrypts the header,
`[size u16 BE][opcode u16 LE][body]` with size = len(body) + 2. The wvs test
harness writes them (see `Client::dump_frames`), and `--raw` reads a single
body with a leading `[opcode u16 LE]` instead (a packet carved from a dump).

Every frame of a kind a check knows is run through `WoW.exe`'s handler under
the emulator (emu.py); a fault, an over-read or an absurd count is a problem,
reported with the client address like a crash log. The exit status is the
number of frames with a problem, so a test or a shell can branch on it.

`WoW.exe` is found in `tools/client_check/`, or at `--exe PATH`.
"""

from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path

# `python -I` (isolated) does not put the script's own directory on the
# path, so the sibling modules are added by hand.
sys.path.insert(0, str(Path(__file__).resolve().parent))

from client import Client
from update import SMSG_COMPRESSED_UPDATE_OBJECT, SMSG_UPDATE_OBJECT, UpdateCheck
from monster_move import SMSG_MONSTER_MOVE, MonsterMoveCheck
from spell_log_miss import SMSG_SPELLLOGMISS, SpellLogMissCheck
from attacker_state import SMSG_ATTACKERSTATEUPDATE, AttackerStateCheck
from initial_spells import SMSG_ACTION_BUTTONS, SMSG_INITIAL_SPELLS, InitialSpellsCheck

DEFAULT_EXE = Path(__file__).resolve().parent / "WoW.exe"


def frames_of(data: bytes):
    """The frames in a capture, as (opcode, body)."""
    i = 0
    while i + 4 <= len(data):
        size = struct.unpack_from(">H", data, i)[0]
        opcode = struct.unpack_from("<H", data, i + 2)[0]
        body = data[i + 4 : i + 2 + size]
        yield opcode, body
        i += 2 + size


def run(paths, exe: Path, raw: bool, verbose: bool) -> int:
    c = Client(str(exe))
    checks = {
        SMSG_UPDATE_OBJECT: UpdateCheck(c),
        SMSG_COMPRESSED_UPDATE_OBJECT: None,  # filled below, same instance
        SMSG_MONSTER_MOVE: MonsterMoveCheck(c),
        SMSG_SPELLLOGMISS: SpellLogMissCheck(c),
        SMSG_ATTACKERSTATEUPDATE: AttackerStateCheck(c),
    }
    checks[SMSG_COMPRESSED_UPDATE_OBJECT] = checks[SMSG_UPDATE_OBJECT]
    checks[SMSG_INITIAL_SPELLS] = InitialSpellsCheck(c)
    checks[SMSG_ACTION_BUTTONS] = checks[SMSG_INITIAL_SPELLS]

    seen = {}
    bad = 0
    for path in paths:
        data = Path(path).read_bytes()
        if raw:
            frames = [(struct.unpack_from("<H", data)[0], data[2:])]
        else:
            frames = list(frames_of(data))
        for opcode, body in frames:
            check = checks.get(opcode)
            seen[opcode] = seen.get(opcode, 0) + 1
            if check is None:
                continue
            problems = check.check(opcode, body)
            tag = f"{Path(path).name} opcode {opcode:#05x} ({len(body)} bytes)"
            if problems:
                bad += 1
                print(f"FAIL {tag}")
                for p in problems:
                    print("    " + p.replace("\n", "\n    "))
            elif verbose:
                print(f"ok   {tag}")

    checked = sum(n for op, n in seen.items() if checks.get(op) is not None)
    skipped = sum(n for op, n in seen.items() if checks.get(op) is None)
    print(
        f"\n{checked} frame(s) checked, {bad} with a problem, "
        f"{skipped} of other kinds skipped.",
        file=sys.stderr,
    )
    return bad


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("files", nargs="+", help="frame captures, or a raw packet with --raw")
    ap.add_argument("--exe", type=Path, default=DEFAULT_EXE, help="WoW.exe (1.12.1, build 5875)")
    ap.add_argument("--raw", action="store_true", help="each file is one body, opcode-first")
    ap.add_argument("-v", "--verbose", action="store_true", help="name the frames that pass too")
    a = ap.parse_args()
    if not a.exe.exists():
        print(f"no WoW.exe at {a.exe}; pass --exe", file=sys.stderr)
        return 2
    return run(a.files, a.exe, a.raw, a.verbose)


if __name__ == "__main__":
    sys.exit(main())
