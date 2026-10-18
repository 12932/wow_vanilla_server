"""SMSG_UPDATE_OBJECT (0x0A9) and SMSG_COMPRESSED_UPDATE_OBJECT (0x1F6) through
the client's handler, `0x4651A0`, whole.

The handler (registered at `0x465140`) reads the block count and the
transport byte, then:
  * if the first block is out-of-range (4), forgets its objects (`0x465EC0`);
  * a transports' create pass, if the transport byte says so, and the create
    pass (`0x466010`): every block from the first after that out-of-range one,
    by type: creates (2, 3) are made (`0x4660A0`), values (0), movement (1) and
    near-object (5) blocks skipped, and an out-of-range block (4) read as
    nothing at all, so a packet with one anywhere but first is misread from
    there on;
  * the main pass (`0x465236`), every block: values applied (`0x465330`),
    movement (`0x465EA0`), creates skipped (`0x465C50`), out-of-range and near
    objects (`0x465FD0`).

What is the object manager's is stubbed, against a model of the client's
objects (`Objects`): finding one by GUID, making one (`0x466E00`: a zeroed
object of its type's size, whose storage the client's own `0x466C70` sets
up), forgetting one, and its type's post-create (`0x466A20`, the renderer's
and the world's). A unit's movement is checked as the create left it: the
spline's init ran in the create, and the copy into the unit
(`0x619320`) is run on it after the handler (see `movement.py`).
"""

from __future__ import annotations

import struct
import zlib
from dataclasses import dataclass, field

from unicorn import UC_HOOK_CODE
from unicorn.x86_const import UC_X86_REG_EDI

from client import Client, NOW
from emu import Emu, Fault, Stop
import movement

SMSG_UPDATE_OBJECT = 0x0A9
SMSG_COMPRESSED_UPDATE_OBJECT = 0x1F6
HANDLER = 0x4651A0
OBJMGR = 0xB41414  # the client's object manager, a pointer

FIND_OR_PENDING = 0x464530  # (ecx: out flag; guid) -> object
FIND = 0x464890  # (guid) -> object
FORGET = 0x464920  # (guid): out of range
ACTIVE_PLAYER = 0x468550  # -> edx:eax the active player's GUID
CREATE = 0x466E00  # (ecx type; guid) -> a new object
POST_CREATE = 0x466A20  # (ecx; edx object; movement)
CREATE_LOG = 0x466BC0  # (ecx type): "Creating player" and so on
LINK = 0x4662A6, 0x4662E0  # the new object onto the object manager's list
# Right after the create pass (`0x466010`), before the main apply loop
# (`0x465236`). The create pass reads every block — creates, values,
# movement, out-of-range and near — so after it the whole packet has been
# parsed and the objects made (their movement snapshotted in the post-init
# stub). The main loop then re-applies each block to the renderer and the
# world, which is out of scope: we stop here. Every captured crash faults
# in the create pass or in the spline copy we run after it, before this.
CREATE_PASS_DONE = 0x465227
# A mask word about to be read into a 41-word array on the stack, and the
# register with its index: the create pass's values read (0x466590).
VALUES_MASKS = {0x4665C4: UC_X86_REG_EDI}

# The object's size, by type (0x466C70's storage and its saved copy end
# below it): object, item, container, unit, player, game object,
# dynamic object, corpse.
OBJECT_SIZE = [0x200, 0x500, 0xB00, 0x1500, 0x4600, 0x400, 0x300, 0x400]
TYPE_NAMES = ["object", "item", "container", "unit", "player", "game object", "dynamic object", "corpse"]


@dataclass
class Made:
    guid: int
    type: int
    addr: int
    movement: bytes | None = None
    arrays: dict = field(default_factory=dict)


class Objects:
    """The objects a client has been told of, as its object manager would
    hold them, kept across a stream's packets."""

    def __init__(self, player: int = 0):
        self.player = player
        self.known: dict[int, int] = {}  # guid -> type


class UpdateCheck:
    def __init__(self, client: Client):
        self.c = client
        e = self.e = client.emu
        e.stub(FIND_OR_PENDING, "ClntObjMgrObjectPtr", 2, self._find_or_pending)
        e.stub(FIND, "ClntObjMgrObjectPtr", 2, self._find)
        e.stub(FORGET, "ClntObjMgrForget", 2, self._forget)
        e.stub(ACTIVE_PLAYER, "ClntObjMgrGetActivePlayer", 0, self._active)
        e.stub(CREATE, "ClntObjMgrCreate", 2, self._create)
        e.stub(POST_CREATE, "CGObject_C::PostInit", 1, self._post_create)
        e.stub(CREATE_LOG, "the create's log line", 0, lambda emu, a: 0)
        e.jump(LINK[0], LINK[1], "the object manager's list of objects")
        for at, reg in VALUES_MASKS.items():
            e.uc.hook_add(UC_HOOK_CODE, self._mask_word, begin=at, end=at, user_data=reg)
        e.stop_at(CREATE_PASS_DONE, "the create pass is done")
        self.objects = Objects()
        self.made: list[Made] = []
        self.addrs: dict[int, int] = {}  # guid -> fake object, this packet

    # ---- stubs ---------------------------------------------------------

    @staticmethod
    def _guid(a) -> int:
        return a[0] | (a[1] << 32)

    def _find_or_pending(self, emu: Emu, a):
        from unicorn.x86_const import UC_X86_REG_ECX

        emu.w32(emu.uc.reg_read(UC_X86_REG_ECX), 0)
        return self._find(emu, a)

    def _find(self, emu: Emu, a):
        g = self._guid(a)
        return self.addrs.get(g, 0) if g in self.objects.known else 0

    def _forget(self, emu: Emu, a):
        g = self._guid(a)
        self.objects.known.pop(g, None)
        self.addrs.pop(g, None)
        return 0

    def _active(self, emu: Emu, a):
        p = self.objects.player
        return (p & 0xFFFFFFFF, p >> 32)

    def _create(self, emu: Emu, a):
        from unicorn.x86_const import UC_X86_REG_ECX

        t = emu.uc.reg_read(UC_X86_REG_ECX)
        g = self._guid(a)
        if t >= len(OBJECT_SIZE):
            raise emu._fault("BAD OBJECT TYPE", f"a create of {g:#x} with type {t}")
        obj = emu.alloc(OBJECT_SIZE[t], f"{TYPE_NAMES[t]} {g:#x}")
        vt = emu.alloc(0x100, f"{TYPE_NAMES[t]} vtable")
        for i in range(0x40):
            emu.w32(vt + 4 * i, emu.trap(f"the {TYPE_NAMES[t]}'s virtual function {i:#x}"))
        emu.w32(obj, vt)
        self.made.append(Made(g, t, obj))
        self.objects.known[g] = t
        self.addrs[g] = obj
        return obj

    def _post_create(self, emu: Emu, a):
        from unicorn.x86_const import UC_X86_REG_EDX

        obj = emu.uc.reg_read(UC_X86_REG_EDX)
        mv = a[0]
        made = next((m for m in self.made if m.addr == obj), None)
        if made and made.type in (3, 4):
            made.movement, made.arrays = movement.snapshot(emu, mv)
        return 0

    def _mask_word(self, uc, address, size, reg):
        i = uc.reg_read(reg)
        if i >= 0x29:
            self.e._fault(
                "STACK OVERRUN",
                f"update mask word {i} read into the client's 41-word mask array (on its stack)",
            )
            uc.emu_stop()

    # ---- the check -----------------------------------------------------

    def check(self, opcode: int, body: bytes) -> list[str]:
        """Run one packet through the handler. Returns the problems found
        (empty if none); raises nothing."""
        problems: list[str] = []
        if opcode == SMSG_COMPRESSED_UPDATE_OBJECT:
            if len(body) < 4:
                return ["a compressed update shorter than its size"]
            size = struct.unpack_from("<I", body)[0]
            try:
                raw = zlib.decompress(body[4:])
            except zlib.error as err:
                return [f"the compressed update doesn't inflate: {err}"]
            if len(raw) != size:
                problems.append(f"the compressed update says {size} bytes and inflates to {len(raw)}")
            body = raw
        e = self.e
        e.reset()
        self.made.clear()
        self.addrs = {}
        om = e.alloc(0x2000, "the object manager")
        e.w32(om + 0x24, 0xFFFFFFFF)
        e.w32(OBJMGR, om)
        packet = struct.pack("<H", SMSG_UPDATE_OBJECT) + body
        ds = self.ds = self.c.datastore(packet, 2)
        try:
            e.call(HANDLER, ecx=0, edx=SMSG_UPDATE_OBJECT, args=(NOW, ds))
        except Stop as s:
            if s.at != CREATE_PASS_DONE:
                return problems + [f"{s} (packet cursor {self.c.cursor(ds) - 2:#x} of {len(body):#x})"]
        except Fault as f:
            return problems + [f"{f.report()} (packet cursor {self.c.cursor(ds) - 2:#x} of {len(body):#x})"]
        else:
            problems.append("the create pass ran to the end without stopping where expected")
        cur = self.c.cursor(ds)
        if cur > len(packet):
            # The client read past the buffer: a real over-read (a bad count
            # or a block longer than it should be). An under-read is not a
            # fault — each packet is its own buffer, so trailing bytes are
            # ignored — and the client consumes a create's field values per
            # field group from object state the harness does not fully set
            # up (a self-player's private groups), so it is left as a note.
            problems.append(f"the create pass read past the packet's end ({len(body):#x} bytes)")
        for m in self.made:
            if m.movement is not None:
                p = movement.copy_into_unit(self.c, m.movement, m.arrays)
                if p:
                    problems.append(f"the {TYPE_NAMES[m.type]} {m.guid:#x}'s movement: {p}")
        return problems
