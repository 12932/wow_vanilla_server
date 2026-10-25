#!/usr/bin/env python
"""MSG_MOVE_TELEPORT (0x0C5) and MSG_MOVE_TELEPORT_ACK (0x0C7) through the
client's handlers, up to the end of their parse.

    uv run --python 3.14 --with unicorn --with pefile python -I teleport.py

run alone, proves the check on packets built here (see `main`).

A near teleport (Blink, a teleport within the map) is told to the player's
own client as MSG_MOVE_TELEPORT_ACK (packed GUID, movement counter,
movement) and to the others in range as MSG_MOVE_TELEPORT (packed GUID,
movement), as mangos's `SendTeleportPacket` and wvs's
`movement::teleport_within` send them.

MSG_MOVE_TELEPORT is registered at `0x60348B` with the handler every other
unit's MSG_MOVE_* shares, `0x603BB0` (fastcall: the opcode in edx; the time
and the store on the stack). It reads the packed GUID (`0x642ED0`) and looks
the unit up (`0x468460`, the object manager, unit type mask 8); none found,
it skips the packet (its cursor to its size). Found, `0x601580` (thiscall on
the unit: opcode, time, store) reads the movement with `0x47EBA0` into a
0x50-byte struct of its frame, takes the clock, and by the opcode (its table
at `0x6018AC`/`0x601844`) hands it on: 0x0C5 to `0x602F90`, the unit's
teleport, which gives it to its CMovement (`0x6186B0`), the world's.

MSG_MOVE_TELEPORT_ACK is registered at `0x6035CB` with `0x603F90`, which
reads the packed GUID and looks the unit up as `0x603BB0` does (none found,
it returns, the rest unread: harmless, each packet is its own store). Found,
`0x602780` (thiscall: time, opcode, store), the handler of what the server
tells the unit a client controls, takes the clock, reads the movement
counter (a u32, `0x418EB0`) and by the opcode (its table at
`0x602A50`/`0x602A2C`) goes to `0x602FB0` for 0x0C7 (time, counter, store),
which reads the movement with `0x47EBA0` into a 0x68-byte local
(`[ebp-0x68]`); from `0x603000` (`0x468570`) on it moves the unit, or answers
the server with its own MSG_MOVE_TELEPORT_ACK (`0x5AB630`): the world's.

`0x47EBA0` (fastcall: the store in ecx, the movement in edx) reads, into the
movement struct:
  +0x10 the move flags (u32); +0x00 the time (u32, `0x418FB0`);
  +0x24/+0x28/+0x2C/+0x30 x, y, z and facing (f32);
  with 0x02000000 (on a transport): +0x08 its GUID, a plain u64 (`0x4190B0`,
  not packed), and +0x14 to +0x20 the place on it, x, y, z and facing;
  with 0x00200000 (swimming): +0x34 the pitch;
  +0x38 the fall time (u32), always;
  with 0x00002000 (jumping): +0x3C to +0x48 the jump's four floats;
  with 0x04000000 (spline elevation): +0x4C that float.
That is wow_messages' 1.12 `MovementInfo` (and mangos-zero's), field for
field.

The check runs each handler with the lookup finding a unit (a block of
memory nothing reads before the stop), stubs `0x602F90` (it records the
movement it is given) and stops at `0x603000`, and fails a packet the client
faults on, reads past, or reads short of (a layout that isn't the client's),
one whose GUID or counter the client reads otherwise than the packet's
bytes say, and one whose place is no point of a map (not finite, or outside
the ±17066.67 yards of a map's 64 tiles).
"""

from __future__ import annotations

import math
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from client import Client, NOW
from emu import Fault, Stop

MSG_MOVE_TELEPORT = 0x0C5
MSG_MOVE_TELEPORT_ACK = 0x0C7
MOVE_HANDLER = 0x603BB0
ACK_HANDLER = 0x603F90
LOOKUP = 0x468460
UNIT_TELEPORT = 0x602F90  # thiscall (unit): time, movement; the world's
ACK_READ = 0x603000  # in 0x602FB0, the movement read
ACK_MOVEMENT = 0x68  # its movement struct, at [ebp - 0x68]
MOVEMENT_SIZE = 0x50
UNIT_SIZE = 0xC00  # CGUnit_C up to past its CMovement (+0x9A8)
# A map is 64 tiles of 533.33 yards a side, about its middle.
MAP_HALF = 32 * 533.3333

ON_TRANSPORT = 0x02000000
SWIMMING = 0x00200000
JUMPING = 0x00002000
SPLINE_ELEVATION = 0x04000000


class TeleportCheck:
    def __init__(self, client: Client):
        self.c = client
        self.e = client.emu
        e = self.e
        if LOOKUP not in e.stubs:
            e.stub(LOOKUP, "the object manager's lookup", 3, lambda emu, a: 0)
        if UNIT_TELEPORT not in e.stubs:
            e.stub(UNIT_TELEPORT, "CGUnit_C's teleport (the world's)", 2, self._teleported)
        if ACK_READ not in e.stops:
            e.stop_at(ACK_READ, "the teleport's movement is read; what follows is the world's")
        self.moved: bytes | None = None

    def _teleported(self, emu, a):
        self.moved = emu.read(a[1], MOVEMENT_SIZE)
        return 0

    def check(self, opcode: int, body: bytes) -> list[str]:
        e = self.e
        e.reset()
        self.moved = None
        packet = struct.pack("<H", opcode) + body
        ds = self.c.datastore(packet, 2)
        unit = e.alloc(UNIT_SIZE, "the unit")
        lookup = e.stubs[LOOKUP]
        found = []

        def find(emu, a):
            found.append(a[0] | a[1] << 32)
            return unit

        old, lookup.fn = lookup.fn, find
        handler = MOVE_HANDLER if opcode == MSG_MOVE_TELEPORT else ACK_HANDLER
        where = lambda: f"(packet cursor {self.c.cursor(ds) - 2:#x} of {len(body):#x})"
        movement = counter = None
        try:
            e.call(handler, ecx=0, edx=opcode, args=(NOW, ds))
        except Stop:
            if opcode == MSG_MOVE_TELEPORT_ACK:
                from unicorn.x86_const import UC_X86_REG_EBP

                ebp = e.uc.reg_read(UC_X86_REG_EBP)
                movement = e.read(ebp - ACK_MOVEMENT, MOVEMENT_SIZE)
                counter = e.u32(ebp + 0xC)
        except Fault as f:
            return [f"{f.report()} {where()}"]
        finally:
            lookup.fn = old
        if opcode == MSG_MOVE_TELEPORT:
            movement = self.moved
        read = self.c.cursor(ds) - 2
        if read > len(body):
            return [f"the client read past the packet's {len(body):#x} bytes {where()}"]
        if movement is None:
            return [
                "the handler returned before its movement was read: "
                f"the opcode's case isn't the teleport's {where()}"
            ]
        problems = []
        if read < len(body):
            problems.append(
                f"the client read {read:#x} of the packet's {len(body):#x} bytes: "
                "the movement's layout isn't the client's"
            )
        guid, at = packed_guid(body, 0)
        if guid is not None and found and found[0] != guid:
            problems.append(f"the client looked up {found[0]:#x}, the packet names {guid:#x}")
        if opcode == MSG_MOVE_TELEPORT_ACK and at is not None and at + 4 <= len(body):
            sent = struct.unpack_from("<I", body, at)[0]
            if counter != sent:
                problems.append(f"the client read the counter as {counter}, the packet's is {sent}")
        x, y, z, o = struct.unpack_from("<4f", movement, 0x24)
        if not all(math.isfinite(v) for v in (x, y, z, o)):
            problems.append(f"the client is put at no point: ({x}, {y}, {z}), facing {o}")
        elif abs(x) > MAP_HALF or abs(y) > MAP_HALF:
            problems.append(f"the client is put off the map: ({x:.2f}, {y:.2f}, {z:.2f})")
        return problems


def packed_guid(body: bytes, at: int):
    """The packed GUID at `at` and the offset after it; (None, None) if the
    packet ends in it."""
    if at >= len(body):
        return None, None
    mask, at = body[at], at + 1
    guid = 0
    for i in range(8):
        if mask & 1 << i:
            if at >= len(body):
                return None, None
            guid |= body[at] << 8 * i
            at += 1
    return guid, at


# ---- the proof --------------------------------------------------------


def pack_guid(guid: int) -> bytes:
    mask, out = 0, bytearray()
    for i in range(8):
        b = guid >> 8 * i & 0xFF
        if b:
            mask |= 1 << i
            out.append(b)
    return bytes([mask]) + bytes(out)


def movement_info(flags=0, at=(-8913.2, -137.4, 82.1, 1.57), fall_time=0) -> bytes:
    """A 1.12 `MovementInfo`, as wow_messages writes it."""
    b = bytearray(struct.pack("<II4f", flags, 0x0012_3456, *at))
    if flags & ON_TRANSPORT:
        b += struct.pack("<Q4f", 0x1FC0_0000_0000_0042, 1.0, -2.0, 3.5, 0.0)
    if flags & SWIMMING:
        b += struct.pack("<f", -0.25)
    b += struct.pack("<I", fall_time)
    if flags & JUMPING:
        b += struct.pack("<4f", -7.9555, 1.0, 0.0, 7.0)
    if flags & SPLINE_ELEVATION:
        b += struct.pack("<f", 1.5)
    return bytes(b)


def main() -> int:
    exe = Path(__file__).resolve().parent / "WoW.exe"
    if not exe.exists():
        print(f"no WoW.exe at {exe}", file=sys.stderr)
        return 1
    check = TeleportCheck(Client(str(exe)))
    player = 0x0000_0000_0000_0007
    cases = [
        ("no flags", 0, True),
        ("jumping", JUMPING | 0x1, True),
        ("falling far", JUMPING | 0x4000, True),
        ("swimming", SWIMMING | 0x1, True),
        ("on a transport", ON_TRANSPORT, True),
        ("with a spline's elevation", SPLINE_ELEVATION, True),
        ("everything", ON_TRANSPORT | SWIMMING | JUMPING | SPLINE_ELEVATION, True),
    ]
    wrong = 0

    def expect(what: str, opcode: int, body: bytes, fine: bool):
        nonlocal wrong
        problems = check.check(opcode, body)
        name = "MSG_MOVE_TELEPORT" if opcode == MSG_MOVE_TELEPORT else "MSG_MOVE_TELEPORT_ACK"
        if fine and not problems:
            print(f"ok   {name}, {what}: read exactly")
        elif not fine and problems:
            print(f"ok   {name}, {what}: reported ({problems[0].splitlines()[0]})")
        else:
            wrong += 1
            print(f"WRONG {name}, {what}: {problems or 'should have been reported'}")

    for what, flags, fine in cases:
        info = movement_info(flags)
        expect(what, MSG_MOVE_TELEPORT, pack_guid(player) + info, fine)
        expect(what, MSG_MOVE_TELEPORT_ACK, pack_guid(player) + struct.pack("<I", 3) + info, fine)
    # What must be reported: a packet cut short, inside the movement and
    # inside the GUID; one with a byte too many; one whose jump block is
    # missing (its flag set, its floats not written); a place no map has.
    whole = pack_guid(player) + struct.pack("<I", 3) + movement_info(JUMPING)
    for opcode, body in (
        (MSG_MOVE_TELEPORT, pack_guid(player) + movement_info()),
        (MSG_MOVE_TELEPORT_ACK, pack_guid(player) + struct.pack("<I", 3) + movement_info()),
    ):
        expect("cut short by 3 bytes", opcode, body[:-3], False)
        expect("a byte too many", opcode, body + b"\0", False)
    expect("cut short in its GUID", MSG_MOVE_TELEPORT_ACK, b"\x03\x07", False)
    expect("its jump's floats left out", MSG_MOVE_TELEPORT_ACK, whole[:-16], False)
    nan = (float("nan"), 0.0, 0.0, 0.0)
    expect("at no point", MSG_MOVE_TELEPORT, pack_guid(player) + movement_info(at=nan), False)
    far = (20_000.0, 0.0, 0.0, 0.0)
    expect("off the map", MSG_MOVE_TELEPORT_ACK, pack_guid(player) + struct.pack("<I", 3) + movement_info(at=far), False)
    return wrong


if __name__ == "__main__":
    sys.exit(main())
