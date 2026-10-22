"""SMSG_INITIAL_SPELLS (0x12A) and SMSG_ACTION_BUTTONS (0x129) through the
client's handlers, whole.

SMSG_INITIAL_SPELLS' handler, `0x5E6510` (registered at `0x5E3138`), reads:
  * a byte (`0x418CB0`), then the spell count (a u16, `0x418DB0`);
  * for each spell, its id and a second u16 (`0x418DB0`, `0x418D30`), into
    the client's list of the spells it was sent (`0xC4D2A0`: its capacity,
    `+4` its count, `+8` the entries, four bytes each), grown to fit
    (`0x5F2410`, `0x5F2430`): no cap on the count;
  * the cooldown count (a u16), and for each cooldown its spell, its item and
    its category (u16s), its time and its category's (i32s, `0x418E30`), the
    category's top bit the "waits for its event" flag, handed to the
    cooldowns (`0x6E2C60`, fastcall, six arguments on the stack), stubbed here
    to record them.

The list is what the client builds its spellbook from whenever its own player
is created (`0x5DEA50`, at a login and after every far teleport): it empties
the book (`0x4B23A0`) and adds each entry. A spell learned in the world
(`SMSG_LEARNED_SPELL`, `0x5E61C0`) goes into the book alone, and into the list
only with no player of its own: so the server sends this again at every
worldport, as mangos's `SendInitialPacketsBeforeAddToMap`.

SMSG_ACTION_BUTTONS' handler, `0x5E6680`, reads 120 u32s into `0xC4C2B4`,
from which the bars are set as the player is created.

The checks are that each handler reads its packet exactly, and that what it
keeps is what the packet says.
"""

from __future__ import annotations

import struct

from unicorn.x86_const import UC_X86_REG_ECX, UC_X86_REG_EDX

from client import Client, NOW
from emu import Fault, Stop

SMSG_ACTION_BUTTONS = 0x129
SMSG_INITIAL_SPELLS = 0x12A
SPELLS_HANDLER = 0x5E6510
BUTTONS_HANDLER = 0x5E6680
LIST = 0xC4D2A0
BUTTONS = 0xC4C2B4
ADD_COOLDOWN = 0x6E2C60


def _parse(body: bytes):
    """The spells and cooldowns of an SMSG_INITIAL_SPELLS, or None if it
    runs short."""
    try:
        p = 1
        (n,) = struct.unpack_from("<H", body, p)
        p += 2
        spells = [struct.unpack_from("<HH", body, p + 4 * i) for i in range(n)]
        p += 4 * n
        (m,) = struct.unpack_from("<H", body, p)
        p += 2
        cds = [struct.unpack_from("<HHHii", body, p + 14 * i) for i in range(m)]
        p += 14 * m
    except struct.error:
        return None
    return spells, cds, p


class InitialSpellsCheck:
    def __init__(self, client: Client):
        self.c = client
        self.e = client.emu
        self.cooldowns: list[tuple] = []

        def add(emu, a):
            ecx = emu.uc.reg_read(UC_X86_REG_ECX)
            edx = emu.uc.reg_read(UC_X86_REG_EDX)
            self.cooldowns.append((ecx & 0xFFFF, edx & 0xFFFF, *a))
            return 0

        if ADD_COOLDOWN not in self.e.stubs:
            self.e.stub(ADD_COOLDOWN, "the cooldowns' add", 6, add)

    def check(self, opcode: int, body: bytes) -> list[str]:
        if opcode == SMSG_ACTION_BUTTONS:
            return self._buttons(body)
        e = self.e
        e.reset()
        self.cooldowns.clear()
        ds = self.c.datastore(struct.pack("<H", opcode) + body, 2)
        try:
            e.call(SPELLS_HANDLER, ecx=0, edx=opcode, args=(NOW, ds))
        except Stop as s:
            return [f"{s} (packet cursor {self.c.cursor(ds) - 2:#x} of {len(body):#x})"]
        except Fault as f:
            return [f"{f.report()} (packet cursor {self.c.cursor(ds) - 2:#x} of {len(body):#x})"]
        read = self.c.cursor(ds) - 2
        if read > len(body):
            return [f"the client read past the packet's {len(body):#x} bytes"]
        if read < len(body):
            return [f"the client read {read:#x} of the packet's {len(body):#x} bytes"]
        parsed = _parse(body)
        if parsed is None or parsed[2] != len(body):
            return ["the packet isn't laid out as the client reads it"]
        spells, cds, _ = parsed
        count = e.u32(LIST + 4)
        entries = e.u32(LIST + 8)
        kept = [struct.unpack("<HH", e.read(entries + 4 * i, 4)) for i in range(count)]
        problems = []
        if kept != spells:
            problems.append(
                f"the client kept {count} spell(s), not the packet's {len(spells)}: "
                f"{kept[:8]} against {spells[:8]}"
            )
        told = [(c[0], c[1]) for c in self.cooldowns]
        if told != [(s, item) for s, item, *_ in cds]:
            problems.append(f"the cooldowns handed on {told} aren't the packet's {cds}")
        return problems

    def _buttons(self, body: bytes) -> list[str]:
        e = self.e
        e.reset()
        ds = self.c.datastore(struct.pack("<H", SMSG_ACTION_BUTTONS) + body, 2)
        try:
            e.call(BUTTONS_HANDLER, ecx=0, edx=SMSG_ACTION_BUTTONS, args=(NOW, ds))
        except (Stop, Fault) as f:
            return [str(f) if isinstance(f, Stop) else f.report()]
        read = self.c.cursor(ds) - 2
        if read != len(body):
            return [f"the client read {read:#x} of the packet's {len(body):#x} bytes (120 u32s)"]
        kept = e.read(BUTTONS, 480)
        if kept != body:
            return ["the client's buttons aren't the packet's"]
        return []
