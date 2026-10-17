# Sitrep: the overnight run (2026-10-17 to 18)

What was asked:
- **First,** align wvs with Thistle Tea ([FULL_PARITY_PLAN.md](FULL_PARITY_PLAN.md)), committing and pushing as each
  phase lands. Load tests are capped at five minutes.
- **Then,** the kobold's reach, performance, and robustness ("Elixir's reliability").
- **Last,** a code review.

This file is where the decisions made along the way are written down, so you can override any of them, along with
the questions waiting for you.

## Questions for you

(None yet.)

## Decisions made (override any)

1. **The kobold's reach.** wvs let a unit swing from a yard beyond its melee range. Thistle Tea (`in_melee_range?`)
   and mangos-classic (`CanReachWithMeleeAttack`) allow none. That yard is the "couple of yards too far" you saw: the
   kobold hit you from where your own swing was out of range.
   - **What changed:** the yard is gone. Two units both running (not walking) get mangos-classic's `MELEE_LEEWAY` of
     8/3 yards, as the 1.12 client does.
   - **Where it goes beyond Thistle Tea:** Thistle Tea has no leeway. Without it, a player chasing a running creature
     would get "out of range" from the server where the client lets them swing.
   - **Distance:** stays 3D, as Thistle Tea measures it. mangos measures a creature's reach in 2D.
2. **How the work runs.**
   - **Isolation:** each stage runs in its own git worktree (`<worktree>/<stage>`, branch `parity/<stage>`).
   - **Merging:** I merge each stage into `master` when the full suite and clippy are clean, then push.
   - **Records:** the agents don't edit THISTLE_TEA_DIFFERENCES.md or this file. I do, at each merge, so the branches
     don't conflict.
3. **A whole-repo `cargo fmt`.** The toolchain's rustfmt (1.10) formats 67 files differently from how they were
   committed. That is one mechanical commit (`ec87804`), made before the stages branched, so each agent can run
   `cargo fmt` without churning files it never touched.
4. **The matrix's not-applicable rows.** Phase 1a lists all 8,850 Thistle Tea tests in `docs/parity/matrix.csv`.
   I marked 26 of them not applicable: the web server, telemetry, the developer seed, the namigator NIF, and the
   Elixir layering rules. The rest are being mapped by area.
5. **wow_messages misreads 1.12's stop packets.** A STOP `SMSG_MONSTER_MOVE` ends after its move type in 1.12
   (mangos-classic's `WriteStopMovement`), but the fork read on into spline flags. So wvs's test client and peon
   dropped every creature stop as unparsed; the real client was never affected. It is being fixed in the fork, then
   repinned in rustigator, wvs and peon together.

## Progress

| Stage | State | Commit |
|---|---|---|
| The kobold's reach | done | dd2d397 |
| 1a: the test matrix, listed | done | b50233b |
| 1a: the matrix, mapped (entity, ai, spell, combat) | running | |
| 2a+2h: spells, auras, PvP | running | |
| 2b: creature movement | running | |
| 2c: creature combat | running | |
| Robustness | running | |
| wow_messages stop layout | running | |

## Left for later, and why
