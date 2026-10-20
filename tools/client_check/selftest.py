#!/usr/bin/env python
"""Prove the checker against the faults it was built for, with no outside
files: build a creature create's movement block the ways wvs sent it and
the way it sends it now, and show the client's own code faults on, or
mishandles, each old one and is fine with the new.

    uv run --python 3.14 --with unicorn --with pefile python -I selftest.py

The crash (before `12c055a`): the 1.12 client counts a create's spline
segments as its points less three (`0x454770`, `lea esi, [eax - 3]`) and
copies that count unchecked (`0x619480`), so a two-point path became a copy
of -1 elements and wrote wild (`0x6194DD`). The checker runs that copy (see
movement.py) and catches it.

The frozen gryphons (2026-10-20): a create on a spline whose move flags had
nothing going anywhere (no MOVEFLAG_FORWARD) was never put among the
client's movers (`0x618940`), so the unit stood where the create put it, a
flier in the air. The checker runs that decision and catches it.

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
FORWARD = 0x1
EXE = Path(__file__).resolve().parent / "WoW.exe"


def padded(path):
    """create_spline_points (update.rs): a point before the first (the
    first less the step to the second), the path, its last again, so the
    array is never shorter than the client's three."""
    first, last = path[0], path[-1]
    second = path[1] if len(path) > 1 else first
    return [tuple(2 * a - b for a, b in zip(first, second)), *path, last]


def movement_block(path, pad, flying, move_flags=SPLINE_ENABLED | FORWARD):
    b = bytearray()
    b += struct.pack("<I", move_flags)             # move flags
    b += struct.pack("<I", 0)                      # timestamp
    b += struct.pack("<3f", *path[0])              # position
    b += struct.pack("<f", 0.0)                    # orientation
    b += struct.pack("<I", 0)                      # fall time
    for s in (2.5, 8.0, 4.5, 4.72, 2.5, 3.14):     # six speeds
        b += struct.pack("<f", s)
    spline_flags = 0x300 if flying else 0x100
    b += struct.pack("<I", spline_flags)           # spline flags (no final pt)
    b += struct.pack("<I", 0)                      # time passed
    b += struct.pack("<I", 1500)                   # duration
    b += struct.pack("<I", 777)                    # id
    pts = padded(path) if pad else list(path)
    b += struct.pack("<I", len(pts))
    for p in pts:
        b += struct.pack("<3f", *p)
    b += struct.pack("<3f", *path[-1])             # final destination
    return bytes(b)


def check(c, path, pad, flying, **kw):
    block = movement_block(path, pad, flying, **kw)
    _, problems = movement.check_block(c, b"\x00\x00" + block, 2)
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
        still = check(c, path, pad=True, flying=flying, move_flags=SPLINE_ENABLED)
        if bare:
            print(f"ok   a bare two-point {kind} faults the client: {bare[0].splitlines()[0]}")
        else:
            print(f"WRONG a bare two-point {kind} should have faulted the client")
            wrong += 1
        if not pad:
            print(f"ok   the padded {kind}, going forward (what wvs sends now), is fine")
        else:
            print(f"WRONG the padded {kind} should be fine: {pad}")
            wrong += 1
        if still:
            print(f"ok   a {kind} with the spline alone is never moved along: {still[0]}")
        else:
            print(f"WRONG a {kind} with the spline alone should be left standing")
            wrong += 1
    return wrong


if __name__ == "__main__":
    sys.exit(main())
