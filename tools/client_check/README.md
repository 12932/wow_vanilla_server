# client_check

Run the packets wvs sends through the 1.12.1 client's own parsing code
(`WoW.exe`, build 5875, in `tools/client_check/`) under the Unicorn CPU emulator, so a
packet that would fault the client fails a test before it reaches one. The
client's bytes are only ever executed inside the emulator.

```sh
# Prove it against the spline crash it was built for:
uv run --python 3.14 --with unicorn --with pefile python -I selftest.py

# Check a frame capture (the format Client::captured writes), or one packet:
uv run --python 3.14 --with unicorn --with pefile python -I check.py capture.bin
uv run ... python -I check.py --raw packet.bin        # opcode-first body
```

| File | What |
|---|---|
| `emu.py` | `WoW.exe` under Unicorn: image, stack, a bounds-checked heap, import/OOB/fault traps |
| `client.py` | the stubs every check shares (allocator, clock, `CDataStore`), and what is known of the client |
| `update.py` | `SMSG_UPDATE_OBJECT` / `SMSG_COMPRESSED_UPDATE_OBJECT`: the create pass, whole |
| `movement.py` | a create's movement block and its spline, and the copy into a unit (the `0x6194DD` crash) |
| `monster_move.py` | `SMSG_MONSTER_MOVE` / `..._TRANSPORT`: the spline reader |
| `spell_log_miss.py` | `SMSG_SPELLLOGMISS`: the handler, read exactly (a target is a GUID and a byte) |
| `check.py` | the command line: route each frame of a capture to its check |
| `selftest.py` | the proof against the spline crash, with no outside files |

See `docs/TESTING.md` ("The client-emulation check") for what it emulates,
what it stubs, its limits, and how to add a check for another handler.
