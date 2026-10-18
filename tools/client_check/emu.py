"""The 1.12.1 client's own code (WoW.exe, build 5875) under Unicorn.

`Emu` maps WoW.exe's image at its base, gives the code a stack and a
bounds-checked heap, and calls the client's functions. Everything the
parsing code reaches outside itself is either stubbed (the allocator, the
clock: see `stub`) or a trap: an import, an unmapped address, a write or
read outside what was allocated. A trap ends the call as a `Fault`, laid out
like the client's crash log.

Nothing here runs WoW.exe natively: its bytes are only ever executed inside
the emulator.
"""

from __future__ import annotations

import bisect
import struct
from dataclasses import dataclass, field

import pefile
from unicorn import (
    UC_ARCH_X86,
    UC_HOOK_CODE,
    UC_HOOK_MEM_INVALID,
    UC_HOOK_MEM_READ,
    UC_HOOK_MEM_WRITE,
    UC_MEM_FETCH_UNMAPPED,
    UC_MEM_READ_UNMAPPED,
    UC_MEM_WRITE_UNMAPPED,
    UC_MEM_WRITE_PROT,
    UC_MEM_READ_PROT,
    UC_MEM_FETCH_PROT,
    UC_MEM_WRITE,
    UC_MODE_32,
    UC_PROT_ALL,
    UC_PROT_READ,
    UC_PROT_WRITE,
    Uc,
    UcError,
)
from unicorn.x86_const import (
    UC_X86_REG_EAX,
    UC_X86_REG_EBP,
    UC_X86_REG_EBX,
    UC_X86_REG_ECX,
    UC_X86_REG_EDI,
    UC_X86_REG_EDX,
    UC_X86_REG_EIP,
    UC_X86_REG_ESI,
    UC_X86_REG_ESP,
    UC_X86_REG_EFLAGS,
)

PAGE = 0x1000
STACK_TOP = 0x0FF00000
STACK_SIZE = 0x00100000
HEAP_BASE = 0x20000000
HEAP_LIMIT = 0x60000000
TRAPS = 0x70000000  # one 16-byte slot per import, and the return-to-host address
RETURN = TRAPS + 0xFFF0
# An allocation above this is absurd for anything a packet describes.
ABSURD_ALLOC = 16 << 20
STEPS = 20_000_000

REGS = [
    ("EAX", UC_X86_REG_EAX),
    ("EBX", UC_X86_REG_EBX),
    ("ECX", UC_X86_REG_ECX),
    ("EDX", UC_X86_REG_EDX),
    ("ESI", UC_X86_REG_ESI),
    ("EDI", UC_X86_REG_EDI),
    ("EBP", UC_X86_REG_EBP),
    ("ESP", UC_X86_REG_ESP),
    ("EIP", UC_X86_REG_EIP),
]


class Fault(Exception):
    """The client's code went wrong: what a crash log would say."""

    def __init__(self, kind: str, eip: int, detail: str, regs: dict, frames: list[int]):
        self.kind, self.eip, self.detail, self.regs, self.frames = kind, eip, detail, regs, frames
        super().__init__(self.report())

    def report(self) -> str:
        lines = [f"{self.kind} at {self.eip:08X}: {self.detail}"]
        r = self.regs
        if r:
            lines.append("  " + "  ".join(f"{n}={r[n]:08X}" for n, _ in REGS[:4]))
            lines.append("  " + "  ".join(f"{n}={r[n]:08X}" for n, _ in REGS[4:]))
        if self.frames:
            lines.append("  stack: " + " <- ".join(f"{a:08X}" for a in self.frames))
        return "\n".join(lines)


class Stop(Exception):
    """A stub ended the call on purpose (it reached what the check stops at)."""

    def __init__(self, at: int, why: str):
        self.at, self.why = at, why
        super().__init__(f"stopped at {at:08X}: {why}")


@dataclass
class Alloc:
    addr: int
    size: int
    tag: str
    live: bool = True


@dataclass
class Stub:
    """A function of the client's replaced: `fn(emu, args) -> eax`, popping
    `argc` stack arguments (stdcall/thiscall) or none (cdecl)."""

    name: str
    argc: int
    fn: object
    callee_pops: bool = True
    calls: list = field(default_factory=list)


class Emu:
    def __init__(self, exe: str):
        self.pe = pefile.PE(exe, fast_load=True)
        self.pe.parse_data_directories(
            directories=[pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_IMPORT"]]
        )
        self.base = self.pe.OPTIONAL_HEADER.ImageBase
        self.image = self.pe.get_memory_mapped_image()
        self.size = (self.pe.OPTIONAL_HEADER.SizeOfImage + PAGE - 1) & ~(PAGE - 1)
        self.uc = Uc(UC_ARCH_X86, UC_MODE_32)
        self.uc.mem_map(self.base, self.size, UC_PROT_ALL)
        self.pristine = bytes(self.image) + b"\0" * (self.size - len(self.image))
        self.uc.mem_write(self.base, self.pristine)
        self.uc.mem_map(STACK_TOP - STACK_SIZE, STACK_SIZE, UC_PROT_READ | UC_PROT_WRITE)
        self.uc.mem_map(TRAPS, 0x10000, UC_PROT_ALL)
        # Every slot a `ret`, so a trap that is allowed through returns.
        self.uc.mem_write(TRAPS, b"\xc3" * 0x10000)
        self.imports: dict[int, str] = {}
        self.named: dict[str, int] = {}
        self.jumps: dict[int, tuple[int, str]] = {}
        self._map_imports()
        self.stubs: dict[int, Stub] = {}
        self.stops: dict[int, str] = {}
        self.allocs: dict[int, Alloc] = {}
        self.starts: list[int] = []  # the allocations' addresses, in order
        self.chunks: list[tuple[int, int]] = []
        self.heap_next = HEAP_BASE
        self.fault: Fault | None = None
        self.stop: Stop | None = None
        self.trace_calls = False
        self.uc.hook_add(UC_HOOK_CODE, self._on_code, begin=TRAPS, end=TRAPS + 0xFFFF)
        self.uc.hook_add(UC_HOOK_MEM_INVALID, self._on_invalid)
        self.uc.hook_add(
            UC_HOOK_MEM_WRITE | UC_HOOK_MEM_READ, self._on_heap, begin=HEAP_BASE, end=HEAP_LIMIT
        )

    # ---- setup -------------------------------------------------------

    def _map_imports(self):
        slot = TRAPS
        for entry in getattr(self.pe, "DIRECTORY_ENTRY_IMPORT", []):
            dll = entry.dll.decode(errors="replace")
            for imp in entry.imports:
                name = imp.name.decode(errors="replace") if imp.name else f"#{imp.ordinal}"
                self.imports[slot] = f"{dll}!{name}"
                self.uc.mem_write(imp.address, struct.pack("<I", slot))
                slot += 16

    def reset(self):
        """Back to WoW.exe's own data and an empty heap, for the next call."""
        self.uc.mem_write(self.base, self.pristine)
        for start, length in self.chunks:
            self.uc.mem_unmap(start, length)
        self.chunks.clear()
        self.allocs.clear()
        self.starts.clear()
        self.heap_next = HEAP_BASE
        self.fault = self.stop = None
        for s in self.stubs.values():
            s.calls.clear()

    def stub(self, addr: int, name: str, argc: int, fn, callee_pops: bool = True) -> Stub:
        s = Stub(name, argc, fn, callee_pops)
        self.stubs[addr] = s
        self.uc.hook_add(UC_HOOK_CODE, self._on_stub, begin=addr, end=addr, user_data=s)
        return s

    def trap(self, name: str) -> int:
        """An address that, called, fails the run naming `name` (for the
        slots of a fake vtable)."""
        addr = self.named.get(name)
        if addr is None:
            addr = TRAPS + 0x8000 + 16 * len(self.named)
            assert addr < RETURN
            self.named[name] = addr
            self.imports[addr] = name
        return addr

    def jump(self, addr: int, to: int, why: str):
        """Skip the client's code from `addr` to `to` (what it does there is
        the object manager's business, not the packet's)."""

        def go(uc, address, size, _):
            uc.reg_write(UC_X86_REG_EIP, to)

        self.uc.hook_add(UC_HOOK_CODE, go, begin=addr, end=addr)
        self.jumps[addr] = (to, why)

    def stop_at(self, addr: int, why: str):
        self.stops[addr] = why
        self.uc.hook_add(UC_HOOK_CODE, self._on_stop, begin=addr, end=addr, user_data=why)

    # ---- memory ------------------------------------------------------

    def alloc(self, size: int, tag: str = "", data: bytes | None = None) -> int:
        """`size` bytes ending flush against an unmapped page, so a write or
        read one past the end faults; the bytes around it in its pages are
        checked by the heap hook."""
        if size > ABSURD_ALLOC:
            raise self._fault("ABSURD ALLOCATION", f"{size:#x} bytes ({tag})")
        pages = (max(size, 1) + 7 + PAGE - 1) // PAGE
        start = self.heap_next
        if start + (pages + 1) * PAGE >= HEAP_LIMIT:
            raise self._fault("OUT OF MEMORY", "the emulated heap is full")
        self.uc.mem_map(start, pages * PAGE, UC_PROT_READ | UC_PROT_WRITE)
        self.chunks.append((start, pages * PAGE))
        # 8-byte aligned, as the client's allocator returns; a size that
        # isn't a multiple of 8 leaves a few bytes the heap hook guards.
        addr = start + pages * PAGE - ((size + 7) & ~7)
        self.heap_next = start + (pages + 1) * PAGE  # a guard page after
        self.allocs[addr] = Alloc(addr, size, tag)
        self.starts.append(addr)
        if data:
            self.uc.mem_write(addr, data)
        return addr

    def free(self, addr: int):
        a = self.allocs.get(addr)
        if a is None or not a.live:
            raise self._fault("BAD FREE", f"{addr:#010x} was not allocated, or freed twice")
        a.live = False

    def owner(self, addr: int) -> Alloc | None:
        """The allocation `addr` is in (or one past the end of)."""
        i = bisect.bisect_right(self.starts, addr) - 1
        if i < 0:
            return None
        a = self.allocs[self.starts[i]]
        return a if addr <= a.addr + a.size else None

    def u8(self, a):
        return self.uc.mem_read(a, 1)[0]

    def u32(self, a):
        return struct.unpack("<I", self.uc.mem_read(a, 4))[0]

    def i32(self, a):
        return struct.unpack("<i", self.uc.mem_read(a, 4))[0]

    def f32(self, a):
        return struct.unpack("<f", self.uc.mem_read(a, 4))[0]

    def w32(self, a, v):
        self.uc.mem_write(a, struct.pack("<I", v & 0xFFFFFFFF))

    def read(self, a, n):
        return bytes(self.uc.mem_read(a, n))

    def write(self, a, b):
        self.uc.mem_write(a, b)

    # ---- calls -------------------------------------------------------

    def call(self, addr: int, ecx=0, edx=0, args=(), steps=STEPS) -> int:
        """Call the client's function at `addr` (thiscall/fastcall with ecx
        and edx; `args` pushed right to left) and return eax. Raises `Fault`
        if it faults or calls what it may not, `Stop` if a stop is reached."""
        self.fault = self.stop = None
        esp = STACK_TOP - 0x100
        for a in reversed(list(args)):
            esp -= 4
            self.w32(esp, a)
        esp -= 4
        self.w32(esp, RETURN)
        uc = self.uc
        uc.reg_write(UC_X86_REG_ESP, esp)
        uc.reg_write(UC_X86_REG_EBP, 0)
        uc.reg_write(UC_X86_REG_ECX, ecx)
        uc.reg_write(UC_X86_REG_EDX, edx)
        uc.reg_write(UC_X86_REG_EAX, 0)
        for r in (UC_X86_REG_EBX, UC_X86_REG_ESI, UC_X86_REG_EDI):
            uc.reg_write(r, 0)
        uc.reg_write(UC_X86_REG_EFLAGS, 0x202)
        try:
            uc.emu_start(addr, RETURN, count=steps)
        except UcError as e:
            if self.fault:
                raise self.fault from None
            if self.stop:
                raise self.stop from None
            raise self._fault("EMULATION ERROR", str(e)) from None
        if self.fault:
            raise self.fault
        if self.stop:
            raise self.stop
        if uc.reg_read(UC_X86_REG_EIP) != RETURN:
            raise self._fault("TIMEOUT", f"no return after {steps} instructions (a loop?)")
        return uc.reg_read(UC_X86_REG_EAX)

    def arg(self, i: int) -> int:
        """Stack argument `i` (0-based) of the function being entered."""
        return self.u32(self.uc.reg_read(UC_X86_REG_ESP) + 4 + 4 * i)

    # ---- hooks -------------------------------------------------------

    def _regs(self) -> dict:
        return {n: self.uc.reg_read(r) for n, r in REGS}

    def _frames(self) -> list[int]:
        out, ebp = [], self.uc.reg_read(UC_X86_REG_EBP)
        for _ in range(24):
            if not (STACK_TOP - STACK_SIZE <= ebp < STACK_TOP - 4):
                break
            ret = self.u32(ebp + 4)
            if ret == RETURN or not (self.base <= ret < self.base + self.size):
                break
            out.append(ret)
            ebp = self.u32(ebp)
        return out

    def _fault(self, kind: str, detail: str) -> Fault:
        f = Fault(kind, self.uc.reg_read(UC_X86_REG_EIP), detail, self._regs(), self._frames())
        self.fault = self.fault or f
        return f

    def _on_invalid(self, uc, access, address, size, value, _):
        what = {
            UC_MEM_READ_UNMAPPED: "read",
            UC_MEM_READ_PROT: "read",
            UC_MEM_WRITE_UNMAPPED: "written",
            UC_MEM_WRITE_PROT: "written",
            UC_MEM_FETCH_UNMAPPED: "executed",
            UC_MEM_FETCH_PROT: "executed",
        }.get(access, "touched")
        where = ""
        if HEAP_BASE <= address < HEAP_LIMIT:
            i = bisect.bisect_right(self.starts, address) - 1
            if i >= 0:
                a = self.allocs[self.starts[i]]
                where = (
                    f" ({address - a.addr - a.size:#x} past the end of the {a.size:#x}-byte"
                    f" {a.tag or 'allocation'} at {a.addr:#010x})"
                )
        self._fault(
            "ACCESS_VIOLATION",
            f'referenced memory at "{address:#010x}"{where}; the memory could not be "{what}"',
        )
        return False

    def _on_heap(self, uc, access, address, size, value, _):
        if self.fault:
            return
        i = bisect.bisect_right(self.starts, address) - 1
        a = self.allocs[self.starts[i]] if i >= 0 else None
        if a and address + size <= a.addr + a.size:
            if not a.live:
                self._fault("USE AFTER FREE", f"{address:#010x} in freed {a.tag or 'memory'}")
                uc.emu_stop()
            return
        what = "write" if access == UC_MEM_WRITE else "read"
        near = ""
        if a and address < a.addr + a.size + PAGE:
            near = f" ({address - a.addr:#x} into the {a.size:#x}-byte {a.tag or 'block'} at {a.addr:#010x})"
        elif i + 1 < len(self.starts):
            b = self.allocs[self.starts[i + 1]]
            near = f" ({b.addr - address:#x} before the {b.size:#x}-byte {b.tag or 'block'} at {b.addr:#010x})"
        self._fault("HEAP OVERRUN", f"{size}-byte {what} at {address:#010x} outside any allocation{near}")
        uc.emu_stop()

    def _on_code(self, uc, address, size, _):
        if address == RETURN:
            return
        name = self.imports.get(address, f"trap {address:#x}")
        caller = self.u32(uc.reg_read(UC_X86_REG_ESP))
        self._fault("UNSTUBBED CALL", f"the code called {name} (from {caller:08X})")
        uc.emu_stop()

    def _on_stub(self, uc, address, size, s: Stub):
        if self.fault or self.stop:
            return
        args = [self.arg(i) for i in range(s.argc)]
        s.calls.append(args)
        try:
            eax = s.fn(self, args)
        except (Fault, Stop) as e:
            if isinstance(e, Stop):
                self.stop = e
            uc.emu_stop()
            return
        esp = uc.reg_read(UC_X86_REG_ESP)
        ret = self.u32(esp)
        uc.reg_write(UC_X86_REG_ESP, esp + 4 + (4 * s.argc if s.callee_pops else 0))
        if isinstance(eax, tuple):  # a 64-bit result, edx:eax
            eax, edx = eax
            uc.reg_write(UC_X86_REG_EDX, edx & 0xFFFFFFFF)
        uc.reg_write(UC_X86_REG_EAX, (eax or 0) & 0xFFFFFFFF)
        uc.reg_write(UC_X86_REG_EIP, ret)

    def _on_stop(self, uc, address, size, why):
        if self.fault or self.stop:
            return
        self.stop = Stop(address, why)
        uc.emu_stop()
