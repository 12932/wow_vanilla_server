"""What is known of WoW.exe 1.12.1 (build 5875), and the harness around it.

`Client` is an `Emu` with the stubs every check shares: the allocator, the
clock, and a `CDataStore` builder. The addresses are the client's own; each
is named where it is used.
"""

from __future__ import annotations

import struct

from emu import ABSURD_ALLOC, Emu, Fault

# CDataStore (vtable 0x7ff9e4): the client's packet reader.
#   +0x00 vtable  +0x04 buffer  +0x08 base (the buffer's first byte's offset)
#   +0x0C alloc (the buffer's length)  +0x10 size (the packet's)  +0x14 read cursor
# A read of n bytes at cursor c with c + n > size sets the cursor to size + 1
# (the overrun mark) and leaves its target as it was; it never faults.
DS_SIZE = 0x18
DS_READ_U8 = 0x418CB0
DS_READ_U16 = 0x418D30  # not yet confirmed
DS_READ_U32 = 0x418EB0
DS_READ_I32 = 0x418E30
DS_READ_U64 = 0x4190B0
DS_READ_F32 = 0x419130
DS_READ_BYTES = 0x419AC0  # (out: pointer into the buffer, n)
DS_READ_PACKED_GUID = 0x642ED0  # fastcall: ecx store, edx out

# The allocator: realloc(ptr, size, name, line, flags), alloc(size, name,
# line, flags), free(ptr, name, line, flags); all stdcall.
MEM_REALLOC = 0x646320
MEM_ALLOC = 0x6462E0
MEM_FREE = 0x646430
# The client's clock in ms (0x42b790 behind it reads the OS's counter).
CLOCK = 0x42C010
NOW = 0x0100_0000


class Client:
    def __init__(self, exe: str):
        self.emu = Emu(exe)
        e = self.emu
        e.stub(MEM_REALLOC, "SMemReAlloc", 5, self._realloc)
        e.stub(MEM_ALLOC, "SMemAlloc", 4, self._alloc)
        e.stub(MEM_FREE, "SMemFree", 4, self._free)
        e.stub(CLOCK, "OsGetAsyncTimeMs", 0, lambda emu, a: NOW)
        self.big_allocs: list[tuple[int, int]] = []

    # ---- the allocator -------------------------------------------------

    def _check_size(self, size: int, caller: str):
        if size > ABSURD_ALLOC:
            raise self.emu._fault(
                "ABSURD ALLOCATION",
                f"{size:#x} bytes asked for{caller} (a count from the packet, unchecked?)",
            )

    def _caller(self) -> str:
        from unicorn.x86_const import UC_X86_REG_ESP

        return f" from {self.emu.u32(self.emu.uc.reg_read(UC_X86_REG_ESP)):08X}"

    def _alloc(self, emu: Emu, a):
        size = a[0]
        self._check_size(size, self._caller())
        return emu.alloc(size, f"alloc{self._caller()}")

    def _realloc(self, emu: Emu, a):
        ptr, size = a[0], a[1]
        self._check_size(size, self._caller())
        new = emu.alloc(size, f"realloc{self._caller()}")
        if ptr:
            old = emu.allocs.get(ptr)
            if old is None or not old.live:
                raise emu._fault("BAD REALLOC", f"{ptr:#010x} was not allocated, or freed")
            emu.write(new, emu.read(ptr, min(old.size, size)))
            emu.free(ptr)
        return new

    def _free(self, emu: Emu, a):
        if a[0]:
            emu.free(a[0])
        return 0

    # ---- packets -------------------------------------------------------

    def datastore(self, body: bytes, cursor: int = 0) -> int:
        """A CDataStore over `body`, as the client's network layer hands a
        handler one: the whole packet in its buffer, the cursor at its body.
        Its vtable is all traps: the reads never need more data."""
        e = self.emu
        vt = e.alloc(0x40, "CDataStore vtable", struct.pack("<16I", *([0x70000000 + 0xFF00] * 16)))
        buf = e.alloc(len(body), "packet", body)
        ds = e.alloc(DS_SIZE, "CDataStore")
        e.write(ds, struct.pack("<6I", vt, buf, 0, len(body), len(body), cursor))
        return ds

    def cursor(self, ds: int) -> int:
        return self.emu.u32(ds + 0x14)

    def size(self, ds: int) -> int:
        return self.emu.u32(ds + 0x10)


def fault_of(e: Exception) -> str:
    return e.report() if isinstance(e, Fault) else str(e)
