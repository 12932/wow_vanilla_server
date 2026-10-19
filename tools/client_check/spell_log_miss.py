"""SMSG_SPELLLOGMISS (0x24B) through the client's handler, `0x5E7E00`
(registered at `0x5E3754`), whole.

The handler reads, in order:
  * the spell (an i32, `0x418E30`) and the caster (a u64, `0x4190B0`);
  * a byte: nonzero for the debug layout, below (`0x418CB0`);
  * the target count (a u32, `0x418EB0`);
  * for each target: its GUID (a u64), then the miss reason as a single
    byte (`0x418CB0`, into a byte it zeroed first), and in the debug layout
    two floats more (`0x419130`), which it logs (`0x6301F0`).

So a target is 9 bytes (17 in the debug layout), as mangos-classic's
`Unit::SendSpellMiss` writes it (`uint8(missInfo)`). wow_messages' fork reads
the reason as its `SpellMissInfo`, a u32: 12 bytes a target, so it can't read
a packet the client and mangos agree on.

What the handler does with a target once read, in the plain layout, is the
combat log's and the floating text's (`0x494720`, `0x62BAB0`, the object's
lookup `0x468460` and `0x607140`), and it reaches them only with the spell's
record from the client's spell table (`0xC0D788`, filled at start-up from
the DBC, empty here): those are stubbed all the same, as no-ops. The check is
that the handler reads the packet exactly: past its end is an over-read, and
short of it means a target's layout isn't the client's.
"""

from __future__ import annotations

import struct

from client import Client, NOW
from emu import Fault, Stop

SMSG_SPELLLOGMISS = 0x24B
HANDLER = 0x5E7E00
# What the handler calls once a target is read: (address, what, stack
# arguments it pops).
AFTER_READ = [
    (0x6301F0, "the debug layout's log line", 4),
    (0x494720, "the combat log's miss text", 0),
    (0x62BAB0, "the combat log's entry", 5),
    (0x468460, "the object manager's lookup", 3),
    (0x607140, "the unit's floating miss text", 3),
]


class SpellLogMissCheck:
    def __init__(self, client: Client):
        self.c = client
        self.e = client.emu
        for at, what, argc in AFTER_READ:
            if at not in self.e.stubs:
                self.e.stub(at, what, argc, lambda emu, a: 0)

    def check(self, opcode: int, body: bytes) -> list[str]:
        e = self.e
        e.reset()
        packet = struct.pack("<H", SMSG_SPELLLOGMISS) + body
        ds = self.c.datastore(packet, 2)
        try:
            e.call(HANDLER, ecx=0, edx=SMSG_SPELLLOGMISS, args=(NOW, ds))
        except Stop as s:
            return [f"{s} (packet cursor {self.c.cursor(ds) - 2:#x} of {len(body):#x})"]
        except Fault as f:
            return [f"{f.report()} (packet cursor {self.c.cursor(ds) - 2:#x} of {len(body):#x})"]
        read = self.c.cursor(ds) - 2
        if read > len(body):
            return [f"the client read past the packet's {len(body):#x} bytes"]
        if read < len(body):
            return [
                f"the client read {read:#x} of the packet's {len(body):#x} bytes: "
                "a target's layout isn't the client's (9 bytes: a GUID and a byte)"
            ]
        return []
