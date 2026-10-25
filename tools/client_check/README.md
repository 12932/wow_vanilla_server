# client_check

Run the packets wvs sends through the 1.12.1 client's own parsing code
(`WoW.exe`, build 5875, in `tools/client_check/`) under the Unicorn CPU emulator, so a
packet that would fault the client fails a test before it reaches one. The
client's bytes are only ever executed inside the emulator.

```sh
# Prove it against the spline crash and the frozen flier it was built for:
uv run --python 3.14 --with unicorn --with pefile python -I selftest.py
# Prove the teleports' check on packets it builds (each flag's block, and
# packets cut short or a byte long):
uv run --python 3.14 --with unicorn --with pefile python -I teleport.py

# Check a frame capture (the format Client::captured writes), or one packet:
uv run --python 3.14 --with unicorn --with pefile python -I check.py capture.bin
uv run ... python -I check.py --raw packet.bin        # opcode-first body
```

| File | What |
|---|---|
| `emu.py` | `WoW.exe` under Unicorn: image, stack, a bounds-checked heap, import/OOB/fault traps |
| `client.py` | the stubs every check shares (allocator, clock, `CDataStore`), and what is known of the client |
| `update.py` | `SMSG_UPDATE_OBJECT` / `SMSG_COMPRESSED_UPDATE_OBJECT`: the create pass, whole |
| `movement.py` | a create's movement block and its spline, the copy into a unit (the `0x6194DD` crash), and whether the unit is moved along its spline (`0x618940`, the frozen flier) |
| `monster_move.py` | `SMSG_MONSTER_MOVE` / `..._TRANSPORT`: the spline reader |
| `spell_log_miss.py` | `SMSG_SPELLLOGMISS`: the handler, read exactly (a target is a GUID and a byte) |
| `attacker_state.py` | `SMSG_ATTACKERSTATEUPDATE`: the melee handler's parse, read exactly (five damage lines at most) |
| `initial_spells.py` | `SMSG_INITIAL_SPELLS` and `SMSG_ACTION_BUTTONS`: the handlers, read exactly, and the spell list and bars the client keeps to build its spellbook from at each create of its own player |
| `teleport.py` | `MSG_MOVE_TELEPORT` and `MSG_MOVE_TELEPORT_ACK` (a near teleport: Blink's, to the others and to the player's own client): the handlers up to their movement's read, read exactly, the GUID and counter as sent, the place a point of a map; run alone, its proof |
| `check.py` | the command line: route each frame of a capture to its check |
| `selftest.py` | the proof against the spline crash and the frozen flier, with no outside files |

See `docs/TESTING.md` ("The client-emulation check") for what it emulates,
what it stubs, its limits, and how to add a check for another handler.
