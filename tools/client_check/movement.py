"""A create's movement block, as the client reads it and copies it into a unit.

`0x47F050` (from the create, `0x466199`) reads the block into a movement
struct (0xD4 bytes, `0x469B70`; its spline at +0x68, `0x469BC0`):
  +0x10 move flags; +0x24 position and facing; +0x50 the six speeds; with
  0x400000 the spline: +0x80 its flags, +0x70 a final point (0x10000), +0x68
  a target (0x20000), +0x7C an angle (0x40000), +0x84 the start (now less the
  time passed), +0x8C duration, +0x90 id, then the nodes (`0x419AC0`, count x
  12 bytes) and their init (`0x453360`: `0x454770`, the segments, counted as
  the nodes less three; `0x4534F0` copies the nodes; `0x4532E0` the length),
  and +0xC0 the final destination.
The spline object is at +0x94: +0x98 its length, +0x9C/+0xA0/+0xA4 the
nodes (capacity, count, pointer), +0xAC/+0xB0/+0xB4 the segments' lengths.

`0x619320` copies the movement into the unit's (`CMovement`, at +0x9A8 in a
unit): the spline's arrays by `0x61ACA0` (nodes) and `0x619480` (segments).
The crash at `0x6194DD` was that copy, of a segment count of -1.

After the copy, `0x618C30` takes the block's move flags into the
CMovement's +0x40 (`0x618DE7`: those of mask 0x75A07DFF, over the sign bit
it sets first), and `0x618940` decides whether the unit moves: one going
somewhere (forward, back, sideways, turning, pitching or jumping, 0x20FF)
goes on the list of movers the world moves each frame (`0x619CA0`, as every
start of a move and a monster move's spline, `0x618808`, put it there);
any other is stopped (`0x619E80`). A unit created on a spline without one
of those flags never moves along it: one on a smooth (flying) spline stays
where the create put it (`0x617B00`, `0x7C61C0`), a gryphon frozen in the
air; one on the ground drops the spline and lands where it is.
"""

from __future__ import annotations

import struct

from client import Client
from emu import Emu, Fault, Stop

READ_MOVEMENT = 0x47F050  # fastcall: ecx store, edx movement
MOVEMENT_INIT = 0x469B70
SPLINE_INIT = 0x469BC0
MOVEMENT_SIZE = 0xD4
SPLINE = 0x68
SPLINE_SIZE = 0x6C
NODES = SPLINE + 0x34  # capacity, count, pointer
SEGMENTS = SPLINE + 0x44  # capacity, count, pointer, granularity
SPLINE_FLAGS = SPLINE + 0x18
MOVE_FLAGS = 0x10
SPLINE_ENABLED = 0x400000

UNIT_COPY = 0x619320  # thiscall (unit's CMovement; time, movement, flag)
UNIT_COPY_DONE = 0x619411  # the copies made: what follows is the world's
UNIT_COPY_NO_SPLINE = 0x61940C
CLEAR_SPLINE = 0x619300
UNIT_SPLINE = 0xA4  # CMovement's spline, a pointer
UNIT_FLAGS = 0x40  # CMovement's move flags
# What 0x618C30 takes of a block's move flags (0x618DEB), over the sign bit
# it sets on a CMovement's first (0x618C59).
FLAGS_TAKEN = 0x75A07DFF
FLAGS_FRESH = 0x80000000
SET_MOVING = 0x618940  # thiscall (CMovement): moving, or stopped
LINK_MOVER = 0x619CA0  # thiscall: onto the movers the world moves each frame
STOP_MOVER = 0x619E80  # thiscall (finish): off them
# A spline with more nodes than this is absurd (mangos sends a path's
# points, and a client path is at most a few hundred).
MAX_NODES = 1000


def install(c: Client):
    e = c.emu
    if CLEAR_SPLINE not in e.stubs:
        e.stub(CLEAR_SPLINE, "CMovement::ClearSpline", 1, lambda emu, a: 0)
        e.stop_at(UNIT_COPY_DONE, "the unit's movement copied")
        e.stop_at(UNIT_COPY_NO_SPLINE, "the unit's movement copied (no spline)")
        e.stub(LINK_MOVER, "CMovement: onto the movers", 0, lambda emu, a: 0)
        e.stub(STOP_MOVER, "CMovement: stopped", 1, lambda emu, a: 0)


def set_moving(c: Client, unit: int, flags: int) -> str | None:
    """`0x618940` on the unit's CMovement, its move flags as `0x618C30`
    leaves them after a create's copy: whether the unit on its spline is
    put among the movers. The problem, or None."""
    e = c.emu
    link = e.stubs[LINK_MOVER]
    link.calls.clear()
    e.w32(unit + UNIT_FLAGS, FLAGS_FRESH | (flags & FLAGS_TAKEN))
    try:
        e.call(SET_MOVING, ecx=unit)
    except Fault as f:
        return f.report()
    if not link.calls:
        return (
            f"the unit on its spline is never moved along it (move flags {flags:#x}:"
            " nothing of 0x20ff, so 0x618940 stops it; mangos's MoveSplineInit sets"
            " MOVEFLAG_FORWARD)"
        )
    return None


def new_movement(e: Emu) -> int:
    mv = e.alloc(MOVEMENT_SIZE, "movement")
    e.call(MOVEMENT_INIT, ecx=mv)
    e.call(SPLINE_INIT, ecx=mv + SPLINE)
    return mv


def sane(e: Emu, mv: int) -> list[str]:
    """What the client built that is out of reason: array counts negative or
    absurd, or an array shorter than its count."""
    out = []
    if not e.u32(mv + MOVE_FLAGS) & SPLINE_ENABLED:
        return out
    for name, at, width in (("node", NODES, 12), ("segment", SEGMENTS, 4)):
        count = e.i32(mv + at + 4)
        ptr = e.u32(mv + at + 8)
        if count < 0 or count > MAX_NODES:
            out.append(f"the spline's {name} count is {count}")
            continue
        if count:
            a = e.owner(ptr)
            have = (a.addr + a.size - ptr) // width if a else 0
            if have < count:
                out.append(f"the spline's {count} {name}s are in room for {have}")
    return out


def snapshot(e: Emu, mv: int) -> tuple[bytes, dict]:
    """The movement struct at `mv` and its spline's arrays, to copy into a
    unit after the create's own copy is gone."""
    raw = e.read(mv, MOVEMENT_SIZE)
    arrays = {}
    for at in (NODES, SEGMENTS):
        ptr = e.u32(mv + at + 8)
        a = e.owner(ptr) if ptr else None
        arrays[at] = e.read(ptr, a.addr + a.size - ptr) if a else None
    return raw, arrays


def copy_into_unit(c: Client, raw: bytes, arrays: dict) -> str | None:
    """`0x619320` on a movement struct (from `snapshot`), into a fresh
    unit's CMovement. The problem, or None."""
    e = c.emu
    install(c)
    mv = e.alloc(MOVEMENT_SIZE, "the create's movement", raw)
    for at, data in arrays.items():
        if data is None:
            e.w32(mv + at + 8, 0)
        else:
            e.w32(mv + at + 8, e.alloc(len(data), "a spline array", data) if data else e.alloc(0, "an empty spline array"))
    problems = sane(e, mv)
    unit = e.alloc(0x200, "the unit's CMovement")
    spline = e.alloc(SPLINE_SIZE, "the unit's spline")
    e.call(SPLINE_INIT, ecx=spline)
    e.w32(unit + UNIT_SPLINE, spline)
    try:
        e.call(UNIT_COPY, ecx=unit, args=(0, mv, 0))
    except Stop:
        pass
    except Fault as f:
        problems.append(f.report())
    flags = struct.unpack_from("<I", raw, MOVE_FLAGS)[0]
    if not problems and flags & SPLINE_ENABLED:
        p = set_moving(c, unit, flags)
        if p:
            problems.append(p)
    return "; ".join(problems) or None


def check_block(c: Client, packet: bytes, at: int) -> tuple[int, list[str]]:
    """Read the movement block at `at` in `packet` (opcode first, as the
    client's store has it) with `0x47F050`, then copy it into a unit.
    Returns the cursor after it and the problems."""
    e = c.emu
    ds = c.datastore(packet, at)
    mv = new_movement(e)
    try:
        e.call(READ_MOVEMENT, ecx=ds, edx=mv)
    except Fault as f:
        return c.cursor(ds), [f.report()]
    end = c.cursor(ds)
    problems = []
    if end > len(packet):
        problems.append("the movement block runs past the packet's end")
    raw, arrays = snapshot(e, mv)
    p = copy_into_unit(c, raw, arrays)
    if p:
        problems.append(p)
    return end, problems
