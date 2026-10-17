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
   - **Isolation:** each stage runs in its own git worktree (`~/wvs-wt/<stage>`, branch `parity/<stage>`).
   - **Merging:** I merge each stage into `master` when the full suite and clippy are clean, then push.
   - **Records:** the agents don't edit THISTLE_TEA_DIFFERENCES.md or this file. I do, at each merge, so the branches
     don't conflict.

## Progress

| Stage | State | Commit |
|---|---|---|
| The kobold's reach | done | |

## Left for later, and why
