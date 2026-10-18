"""SMSG_MONSTER_MOVE (0x0DD) and SMSG_MONSTER_MOVE_TRANSPORT (0x2AE) through
the client's spline reader, `0x6018F0` (reached from the handler `0x603F00`,
registered at `0x6035EB`).

The handler reads the mover's packed GUID, finds the unit, and calls
`0x6018F0` on its `CMovement` (at unit+0x9A8). That reader reads, from the
body after the GUID:
  * the start point (three floats) and the spline id (a u32);
  * the move type, a byte: 0 normal, 1 stop, 2 to face a point (three
    floats), 3 to face a target (a packed... a u64 GUID), 4 to face an angle
    (a float);
  * for anything but a stop: the spline flags (a u32), a time (a u32) and the
    node count (a u32), then the nodes: whole points when the flags have
    0x200 (a flyer's, `0x601B72`), else the destination whole and the rest as
    packed 11/11/10-bit offsets from it (`0x601D03`, unpacked by `0x602130`).

The reader puts the nodes in a buffer it takes off its own stack (`0x40A280`,
`_alloca`), so an absurd count walks the stack down until it faults, as it
would the client's. We stop before the world and render apply (`0x601F37`):
what the packet drives is the read and the node copy.

Stubbed: the `CMovement` stores the apply would make, and the clock. The
unit is a bare object with a position block at +0x118 (its +0x10 the current
point, which the packed form is relative to) and room for the `CMovement`.
"""

from __future__ import annotations

import struct

from unicorn.x86_const import UC_X86_REG_ECX

from client import Client, NOW
from emu import Emu, Fault, Stop

SMSG_MONSTER_MOVE = 0x0DD
SMSG_MONSTER_MOVE_TRANSPORT = 0x2AE
HANDLER = 0x603F00
READER = 0x6018F0
PARSE_DONE = 0x601F37  # the reads are done; what follows is the apply
UNIT_SIZE = 0xB00
POSITION_BLOCK = 0x118  # unit+0x118 -> a block whose +0x10 is the position
CMOVEMENT = 0x9A8
MAX_NODES = 2000


class MonsterMoveCheck:
    def __init__(self, client: Client):
        self.c = client
        self.e = client.emu
        self.e.stop_at(PARSE_DONE, "the move is read; the apply is the renderer's")

    def check(self, opcode: int, body: bytes) -> list[str]:
        """`body` is the whole packet body after the opcode: a packed GUID,
        then (for the transport move) a transport GUID and a byte, then the
        move. We read the GUID ourselves to find where the move starts, and
        drive `0x6018F0` from there, as the handler does."""
        e = self.e
        e.reset()
        p = _skip_packed_guid(body, 0)
        if opcode == SMSG_MONSTER_MOVE_TRANSPORT:
            p = _skip_packed_guid(body, p) + 1  # transport guid, then a seat byte
        if p is None or p > len(body):
            return ["the move's GUID runs past the packet"]
        move = body[p:]

        unit = e.alloc(UNIT_SIZE, "the unit")
        pos = e.alloc(0x40, "the unit's position block")
        e.write(pos + 0x10, struct.pack("<3f", 0.0, 0.0, 0.0))
        e.w32(unit + POSITION_BLOCK, pos)
        # A vtable of traps, so a stray virtual call is caught, not a wild jump.
        vt = e.alloc(0x100, "the unit's vtable")
        for i in range(0x40):
            e.w32(vt + 4 * i, e.trap(f"the unit's virtual function {i:#x}"))
        e.w32(unit, vt)
        self._stub_apply()

        ds = self.c.datastore(b"\x00\x00" + move, 2)
        try:
            e.call(READER, ecx=unit, args=(ds, opcode))
        except Stop:
            pass
        except Fault as f:
            return [f"{f.report()} (move cursor {self.c.cursor(ds) - 2:#x} of {len(move):#x})"]
        problems = []
        cur = self.c.cursor(ds) - 2
        if cur > len(move):
            problems.append(f"the client read {cur:#x} of the move's {len(move):#x} bytes (past its end)")
        return problems

    def _stub_apply(self):
        # The CMovement stores and helpers the reader calls as it goes; none
        # read from the packet, so a no-op keeps the parse honest.
        for at in (0x618750, 0x618840, 0x618870, 0x6188A0, 0x6187A0):
            if at not in self.e.stubs:
                self.e.stub(at, f"a CMovement apply ({at:#x})", 0, lambda emu, a: 0, callee_pops=False)


def _skip_packed_guid(body: bytes, p: int):
    if p >= len(body):
        return None
    mask = body[p]
    p += 1
    for i in range(8):
        if mask & (1 << i):
            p += 1
    return p
