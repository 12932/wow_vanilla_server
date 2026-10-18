# Thistle Tea, completely: a plan

[ALIGNMENT_PLAN.md](ALIGNMENT_PLAN.md) brought wvs close to Thistle Tea (pinned at `0426f0d6`), measured by three
audits and module-by-module sweeps. [THISTLE_TEA_DIFFERENCES.md](THISTLE_TEA_DIFFERENCES.md) lists what remains.
This plan closes it, and replaces audit judgement with numbers, so that "complete" can be checked rather than argued.
Thistle Tea's web server stays out of scope.

**Done means:**
- **Every Thistle Tea test is accounted for.** Each of its 8,850 tests (1,264 files) maps to a wvs test that checks
  the same behaviour and passes, or to a difference the user has accepted.
- **The two servers send the same packets.** On the same data and the same scripted journeys, server to client,
  nothing differs that isn't explained.
- **The differences list is only the accepted ones.**

## Where it stands (2026-10-18)

| Phase | State |
|---|---|
| 1a, the matrix | done: every test mapped ([parity/](parity/)) |
| 1b, the packet diff | not started |
| 1c, keeping up with Thistle Tea | the process is in [parity/README.md](parity/README.md); the pin is `0426f0d6` |
| 2, the known differences | done |
| 3, the matrix area by area | five rounds: 61% covered or accepted (5,110 of 8,380), from 21% |
| 4, the packet diff to zero | waits on 1b |
| 5, data and the protocol library | the fork is done; the data's provenance not started |
| 6, the accepted differences | the open questions in [SITREP.md](SITREP.md) |
| 7, sign-off | not started |

Of the 3,224 rows left:
- Most are work in the areas round 5 was on: the player, spells, combat, auras and loaders, scripts, pets.
- Some wait on a question in [SITREP.md](SITREP.md).
- Some wait on a system wvs doesn't have yet: Scourge camps, the Lunar Festival's fireworks, a movement
  acknowledgement system.

Each row's note says which. [STATE.md](STATE.md) has the rest.

## Phase 1: the measure

**1a, the test matrix.**
- **The list:** a script lists every Thistle Tea test (module, `describe`, `test` name, file and line) into
  `docs/parity/matrix.csv`, with columns for the wvs test that covers it and its status (covered, gap, accepted
  difference, not applicable: the web server, Elixir internals).
- **Where the tests are:** `test/game/core` 610 files, `test/game/world` 520, `game/network` 85, `game/inbound` 35,
  and a handful elsewhere.
- **Filling it:** agents take it module by module, starting from the wvs tests that already exist.
- **The number:** covered over applicable becomes the headline. Each phase below moves it.

**1b, the packet diff.**
- **The setup:** a harness runs Thistle Tea and wvs one at a time on the same `data/db`, navmesh and terrain. A
  deterministic scripted client drives each, built from peon's protocol code with a fixed seed and fixed timings,
  and both servers' traffic is recorded.
- **The comparison:** normalise what is meant to differ (GUID counters, timestamps, the server seed, ordering inside
  a tick), then diff message by message.
- **The journeys:**
  - a level 1 to 10 run for each race and class;
  - a dungeon (the Deadmines);
  - a raid boss (Ragnaros by GM commands);
  - one match in each battleground;
  - the auction house, mail, trade, a guild, a group with loot rolls;
  - professions from 1 to 75;
  - deaths, the corpse run, and the spirit healer;
  - Gurubashi's last man standing.
- **The output:** a report of diffs per journey, each to be fixed or explained.

**1c, keeping up with Thistle Tea.** The pin is recorded (`0426f0d6`). Moving it means diffing `lib/game` between
the two commits, regenerating `tt_scripts.json.gz` with `tools/dump_tt_scripts.exs`, and adding the new tests to the
matrix. The procedure is written down, so later work isn't against a moving target.

## Phase 2: the known differences

Each stage takes one section of THISTLE_TEA_DIFFERENCES.md, with a test for each item that fails before the fix.

- **2a, spells and auras:**
  - dispel passive auras;
  - hold equipped items' form-bound spells to their form;
  - the flat healing-taken bonus's spell coefficient;
  - healing-taken modifiers on a heal over time at each tick;
  - the killing spell's `no_durability_loss` reaching the death's wear (the kill knows its spell);
  - Net-o-Matic's 13139;
  - `retain_combo_points` (aura 148).
- **2b, creature movement** ([AI5_GAPS.md](AI5_GAPS.md)):
  - wander clamping (M2);
  - pools with limits, re-rolls and event spawns (M4);
  - charge (M5);
  - fear, confuse and flee splines (M8);
  - distract (M10);
  - formations' fidelity (M11);
  - scripted follow (M12);
  - the flying spline packet (M14);
  - the run flag (M18);
  - M19's small items.
- **2c, creature combat:**
  - harmful casts and stealth (C6);
  - melee range and the chase (C9);
  - emote and stand state in a fight (C10);
  - heal on leashing and the leash's extension (C11, C15);
  - combat facing packets (C12);
  - attack start and stop timing (C13);
  - spreading round a target (C14);
  - victim selection's finer rules (C18);
  - casters' line of sight (C19);
  - creatures charming players (C21);
  - spline speed packets only for possessed pets;
  - EventAI's smaller ordering items (E16).
- **2d, scripts and encounters:**
  - the open script commands (S2 to S4, S6 to S12, S14 to S18, S20 to S23);
  - summons' despawn types;
  - the unkillable flag;
  - `update_entry`;
  - the zone pulse command;
  - emote states;
  - D18, D20 and D25;
  - creature spell lists' force-cast flag, order, timed-out summons and guardians;
  - Molten Core's rune and its Quintessence: un-ignore `tests_ai5_instances::the_real_instances` and make it pass.
- **2e, instances:**
  - raid binds only for player killers;
  - re-raised chests' loot after a reload;
  - Razorfen Downs' gong across a reload;
  - lazy adoption;
  - escort hold and await steps.
- **2f, quests and events:**
  - The Nightmare Manifests (8736): the escort engine gains unit flags, phases, signals, facing and batch summons, and
    the map event its credit goes through;
  - walk the event quests not yet walked (2969, 5821 and 5943, 6403, 7622) and the 176 database event quests, one
    test each.
- **2g, the world and social systems:**
  - speed changes reach observers once;
  - release, revive and `.speed` relay at the change;
  - water interrupts from the terrain's liquid;
  - Bind's Hearthstone by the spell: a creature's create-item effect gives its target the item.
- **2h, PvP:** spell 30176.
- **2i, the second pass:** what the sweeps found matching but didn't read line by line (gathering, crafting,
  lockpicking, disenchanting, fishing, the corpse reclaim, self-resurrection, resurrection sickness), checked
  against Thistle Tea line by line.

## Phase 3: the matrix, area by area

Phase 1a's gaps, closed in order of Thistle Tea's own test counts, each Thistle Tea test ported as a wvs test (through
the protocol where it is a behaviour, as a unit test where it is a formula). Stages:
- **3a:** `core/aura` and `core/spell`.
- **3b:** `core/combat` and `core/ai`.
- **3c:** `core/quest`, `core/item`, loot and professions.
- **3d:** `world/entity/player`: stats, XP, skills, reputation, rest, the action bar.
- **3e:** `world/system`: battlegrounds, outdoor PvP, instances, game events, weather, transports.
- **3f:** `network` and `inbound`: every message's layout and edge cases (the layouts wvs writes by hand checked
  against the matrix's Thistle Tea expectations).

**The exit:** the matrix at 100% of applicable tests, every one covered or an accepted difference.

## Phase 4: the packet diff to zero

Phase 1b's report, fixed journey by journey. Each diff is closed or explained in THISTLE_TEA_DIFFERENCES.md, and each
journey becomes a regression check run before every merge.

## Phase 5: data and the protocol library

- **The wow_messages fork:** done. `12932/wow_messages`, branch `vanilla-1.12-layouts`, pinned by wvs, rustigator
  and peon, and wvs's test client decodes what the server sends. New layout fixes go there.
- **The data's provenance:** compare what Thistle Tea changes in code against `vmangos.sqlite` rows (its fixes and
  overrides), and carry them into wvs's loaders where they're missing.
- **The setup:** [DATA_SETUP_PLAN.md](DATA_SETUP_PLAN.md), so every tester has the same navmesh and terrain.

## Phase 6: the accepted differences

The user decides each remaining difference, to keep or align:
- the real honor rank on the scoreboard;
- the two-minute battleground invite;
- anything Phases 3 and 4 surface where Thistle Tea is wrong for 1.12.

What's kept is THISTLE_TEA_DIFFERENCES.md's only content.

## Phase 7: sign-off

- **A fourth independent audit,** given the matrix and the packet report, to find what both missed.
- **Peon's baselines on both servers,** same data, same scenarios: behaviour equal, and wvs's CPU and latency still
  a fraction of Thistle Tea's.
- **The proof:** the matrix at 100%, the packet diff at zero unexplained, and the differences list only the accepted
  ones.

## How it runs

Agents, each on its own branch from the current tip, as before:
- **Merging:** one stage per branch, merged as it lands, with the full suite (`cargo nextest run --lib`) and clippy
  clean at every merge.
- **Order:**
  - Phases 1a and 1b come first: everything after is measured by them.
  - Phase 2's stages are independent and run in parallel.
  - Phase 3 starts once 1a's matrix lists the gaps.
  - Phase 4 needs 1b.
  - Phase 5 can run beside any of them.
  - Phases 6 and 7 close.
