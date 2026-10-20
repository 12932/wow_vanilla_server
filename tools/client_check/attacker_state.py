"""SMSG_ATTACKERSTATEUPDATE (0x14A) through the client's handler, `0x6255B0`
(registered at `0x625549`), up to the end of its parse.

`0x6255B0` handles the melee messages 0x143 to 0x1FC through a jump table
(`0x625B0C`, `0x625AEC`); 0x14A's case (`0x62578D`) builds a blow on its
stack (`0x6263A0`) and reads it from the packet (`0x625C60`):
  * the hit info (a u32), the attacker and the victim (packed GUIDs), the
    total damage (an i32);
  * the count of damage lines (a byte), then each line's school (an i32), its
    damage as a float and as an i32, its absorb and its resist (i32s), into
    five-slot arrays: the count is not checked, so more than five writes over
    the blow's other fields (and, far enough, past the handler's frame);
  * the victim state, a u32 (the "usually -1, 0 or 1000" of mangos), the
    spell (u32s) and the blocked amount (an i32);
  * with hit info's 0x1 (HITINFO_UNK0), a debug trailer: a u32, eight floats,
    five pairs of floats and a u32.

The hit info is a set of flags: the parse branches on 0x1 alone, so any other
combination (a miss of the main hand is 0x12, a crushing blow 0x8002) is read
alike. The check stops at `0x6257FD`, the parse done: what follows (finding
the attacker, the animation, the combat log and the floating text) is the
world's and the renderer's. The object lookup the parse ends with (`0x468460`)
is stubbed to find nothing.
"""

from __future__ import annotations

import struct

from client import Client, NOW
from emu import Fault, Stop

SMSG_ATTACKERSTATEUPDATE = 0x14A
HANDLER = 0x6255B0
PARSE_DONE = 0x6257FD
LOOKUP = 0x468460
# The blow's damage lines live in five-slot arrays.
MAX_LINES = 5


class AttackerStateCheck:
    def __init__(self, client: Client):
        self.c = client
        self.e = client.emu
        self.e.stop_at(PARSE_DONE, "the blow is read; what follows is the world's")
        if LOOKUP not in self.e.stubs:
            self.e.stub(LOOKUP, "the object manager's lookup", 3, lambda emu, a: 0)

    def check(self, opcode: int, body: bytes) -> list[str]:
        lines = _lines(body)
        if lines is None:
            return ["the blow's GUIDs run past the packet"]
        if lines > MAX_LINES:
            return [
                f"{lines} damage lines: the client's blow holds {MAX_LINES}; "
                "more write over its other fields, and past its frame"
            ]
        e = self.e
        e.reset()
        ds = self.c.datastore(struct.pack("<H", SMSG_ATTACKERSTATEUPDATE) + body, 2)
        try:
            e.call(HANDLER, ecx=0, edx=SMSG_ATTACKERSTATEUPDATE, args=(NOW, ds))
        except Stop:
            pass
        except Fault as f:
            return [f"{f.report()} (packet cursor {self.c.cursor(ds) - 2:#x} of {len(body):#x})"]
        else:
            return ["the handler returned before its parse was done: the opcode's case isn't the blow's"]
        read = self.c.cursor(ds) - 2
        if read > len(body):
            return [f"the client read past the packet's {len(body):#x} bytes"]
        if read < len(body):
            return [
                f"the client read {read:#x} of the packet's {len(body):#x} bytes: "
                "the blow's layout isn't the client's"
            ]
        return []


def _lines(body: bytes):
    """The count of damage lines, after the hit info, two packed GUIDs and
    the total damage."""
    p = 4
    for _ in range(2):
        if p >= len(body):
            return None
        mask = body[p]
        p += 1 + bin(mask).count("1")
    p += 4
    return body[p] if p < len(body) else None
