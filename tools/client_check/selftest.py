#!/usr/bin/env python
"""Prove the checker against the crash it was built for, with no outside
files: build a creature create's movement block both the way wvs sent it
before `12c055a` (a bare two-point path) and the way it sends it now
(mangos's padded array), and show the client's own code faults on the first
and is fine with the second.

    uv run --python 3.14 --with unicorn --with pefile python -I selftest.py

The crash: the 1.12 client counts a create's spline segments as its points
less three (`0x454770`, `lea esi, [eax - 3]`) and copies that count
unchecked (`0x619480`), so a two-point path became a copy of -1 elements
and wrote wild (`0x6194DD`). The checker runs that copy (see movement.py)
and catches it.

Exit status is the number of checks that came out wrong (0 when all pass).
"""

from __future__ import annotations

import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from client import Client
import movement

SPLINE_ENABLED = 0x400000
EXE = Path(__file__).resolve().parent / "WoW.exe"


def padded(path, flying):
    """create_spline_points (update.rs): a flyer gets an extrapolated point
    before its first, a one-point path is doubled, the last is always
    repeated, so the array is never shorter than the client's three."""
    first, last = path[0], path[-1]
    out = []
    if flying:
        second = path[1] if len(path) > 1 else first
        out.append(tuple(2 * a - b for a, b in zip(first, second)))
    out += list(path)
    if len(path) == 1:
        out.append(last)
    out.append(last)
    return out


def movement_block(path, pad, flying):
    b = bytearray()
    b += struct.pack("<I", SPLINE_ENABLED)        # move flags
    b += struct.pack("<I", 0)                      # timestamp
    b += struct.pack("<3f", *path[0])              # position
    b += struct.pack("<f", 0.0)                    # orientation
    b += struct.pack("<I", 0)                      # fall time
    for s in (2.5, 8.0, 4.5, 4.72, 2.5, 3.14):     # six speeds
        b += struct.pack("<f", s)
    b += struct.pack("<I", 0x100)                  # spline flags (no final pt)
    b += struct.pack("<I", 0)                      # time passed
    b += struct.pack("<I", 1500)                   # duration
    b += struct.pack("<I", 777)                    # id
    pts = padded(path, flying) if pad else list(path)
    b += struct.pack("<I", len(pts))
    for p in pts:
        b += struct.pack("<3f", *p)
    b += struct.pack("<3f", *path[-1])             # final destination
    return bytes(b)


def check(c, path, pad, flying):
    _, problems = movement.check_block(c, b"\x00\x00" + movement_block(path, pad, flying), 2)
    return problems


def main() -> int:
    if not EXE.exists():
        print(f"no WoW.exe at {EXE}", file=sys.stderr)
        return 1
    c = Client(str(EXE))
    path = [(-13718.6, 253.0, 29.7), (-13700.0, 260.0, 29.7)]
    wrong = 0
    for flying in (False, True):
        kind = "flight" if flying else "walk"
        bare = check(c, path, pad=False, flying=flying)
        pad = check(c, path, pad=True, flying=flying)
        if bare:
            print(f"ok   a bare two-point {kind} faults the client: {bare[0].splitlines()[0]}")
        else:
            print(f"WRONG a bare two-point {kind} should have faulted the client")
            wrong += 1
        if not pad:
            print(f"ok   the padded {kind} (what wvs sends now) is fine")
        else:
            print(f"WRONG the padded {kind} should be fine: {pad}")
            wrong += 1
    return wrong


if __name__ == "__main__":
    sys.exit(main())
