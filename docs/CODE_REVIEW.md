# Code review (2026-10-18)

Four reviews of wvs on 18 October 2026, for correctness, robustness, reliability and speed, after the parity work of
17–18 October. Each fixed what it could show with a test that failed first, and wrote up the rest. The full findings
are in [review/](review/):
- review 1, the store and login: below;
- [review2.md](review/review2.md), the map core;
- [review3.md](review/review3.md), combat, spells and auras;
- [review4.md](review/review4.md), the world's systems.

The review of 79b6def that this file held before predates the per-map rewrite, and is in the git history.

## Review 5 (2026-10-20): open
Six reviewers, by area: [review/review5.md](review/review5.md), its first table the 21 that matter most (6 high: a
crafted flight route corrupting others' packet streams, a writer outliving its close, a long mail subject cutting
the reader off, a player dropped by a copy stopped from outside, a pet's threat list never pruned, skills wiped on
missing world data). None fixed yet; waiting on your choice.

- **#6** fixed: the start refuses data that was looked for and doesn't load (missing, an LFS pointer, unreadable, no
  creatures, spells, areas or skill lines), and no data at all over a store with characters (`game::world_data`); a
  skill the data gives no slot is saved as it was loaded (`skills::saved`).
- **#11** fixed: the next character and item GUIDs are kept in the store with every write (migration 5,
  `counters`; `Store::keep_counters`), so none is handed out twice; a deleted character's honor entry and raid binds
  go with them, as mangos's `DeleteFromDB`.
- **#19** fixed: records (honor, mail, auctions, guilds, charters, copies, tickets) load field by field as a
  character's state does (`store::parse_record`), one that doesn't read at all is logged at `error` and left in the
  store, and each kind's next id counts every record's id; `.honor points` and `set_rank_points` refuse a NaN.

## Fixed (about 60)
- **Items and money that could be duplicated or lost:**
  - **saves apart:** a trade, mail sent or taken, an auction, a charter or emblem bought were each saved on both
    sides at different times. They now go in one transaction (`Store::save_together`).
  - **the buyback:** a sold item could be taken back out of its slot for nothing.
  - **reagents:** banked mid-cast, they let the spell go off for free.
  - **bank bags:** a bag with items in it could be destroyed.
  - **bank bag slots:** those bought, and their money, were lost at every login.
- **Crashes and panics:**
  - a script starting itself with no delay overflowed the stack;
  - a panic in a proc left its depth raised for good;
  - a transport path with nothing to sail panicked at boot;
  - `.levelup` and `.ban` with huge numbers, and an auction's cut on a big bid, overflowed.
- **Bans, saves and copies:**
  - a ban didn't hold against the session the account already had;
  - a dungeon copy left empty lost its save within seconds;
  - a player sent into a stopping copy was lost;
  - honor earned just before a shutdown was lost;
  - an unreadable `config.toml` silently ran on the defaults.
- **Client input:** a NaN or far transport offset was relayed and saved, and the first move after a login went
  unchecked.
- **Game logic:**
  - **threat:**
    - a spell counted its caster's threat auras twice;
    - a creature switching victims moved its leash.
  - **leaving the map:** a player who logged out kept their channel and stayed on threat lists.
  - **casting:**
    - a cast during a channel didn't end it;
    - an interrupted cast was told twice.
  - **swings:**
    - an attacker a damage shield killed swung on;
    - a dual-wielder's hands swung in the same step.
  - **auras:**
    - restored stacks showed as one;
    - a replaced aura was never removed;
    - a pet's two-effect area aura gave one effect.
  - **loot:**
    - anyone could close another's loot window;
    - a stack added to wasn't told its count.
  - **events and quests:**
    - a spawn tied to several events kept one;
    - quests taken done weren't told complete;
    - an abandoned quest's area aura stayed.

  And more. Each fix is in its review's file, with its test.
- **The client's own crashes,** found from its crash dumps:
  - a character look it has no texture for;
  - an update naming what it had just forgotten;
  - corpses made before their owners went;
  - a creature spline of fewer than three points.

  A harness now runs the client's own parsers on what wvs sends (`tools/client_check/`, [TESTING.md](TESTING.md)).
- **Dead bodies:** a dead body no longer moves with a client that ignores its root.

## Left: the hard ones
**High**
- **H1, a transport-flag teleport** (review 2): a movement packet with the on-transport flag skips the speed check,
  and its transport GUID isn't checked, so a client can teleport anywhere. A fix checks the GUID against the map's
  transports, and the position against where that transport is on its route. **Accepted** (2026-10-19): you
  said it's fine as it is; no fix planned.

**Medium**
- **M1, height** (review 2): the speed check ignores height. A fix needs a vertical allowance that real falls pass.
  **Accepted** (2026-10-19): the speed check keeps ignoring height.
- **M2, copies** (review 2): two group members entering a dungeon at once can get different copies. `key_for`
  releases its lock between choosing an id and storing it. **Fixed** (2026-10-20, `fix/m2m3`): `key_for` holds
  the lock from choosing the copy to storing it, and those let in count against the copy's limit until they arrive.
- **M3, shutdown** (review 2): shutdown can miss a player between maps. **Fixed** (2026-10-20, `fix/m2m3`): a map
  asked for after the shutdown began, or one that has shut down, saves whoever arrives, and the shutdown waits
  (within its limit) until no one is online.
- **M4, empty pools** (review 2): health or mana saved at 0 loads as full, so a relog refills a mage. A fix needs a
  marker that a character has been saved. **Deferred** (2026-10-20: you rated it low; explained 2026-10-19): a pool saved at 0 (mana
  spent, or rage and energy at rest) loads full, as the loader can't tell spent from never saved, so a relog refills
  it.
- **R3-15, deaths that despawn** (review 3): a death action that despawns its creature cuts the death short, so
  loot and the summoner's and group's death events are skipped. A fix reorders the death's hooks, and needs the
  EventAI and boss tests to pass again.
- **R3-16, aura indexes** (review 3): a new aura's index can go stale after the auras it replaces end, and one
  early return skips the stats' recalculation.
- **R4-18, round-robin loot** (review 4): only the assigned looter can open the corpse, so the others' quest drops
  and the money wait. A fix needs mangos's released state.
- **R4-19, groups** (review 4): they aren't saved, so a restart dissolves them.
- **R4-20, Alterac Valley** (review 4): a general killed without a player's credit doesn't end the match.
- **R4-21, mail** (review 4): taking a mail's item can race its expiry sweep and duplicate it.
- **R4-22, charters** (review 4): a destroyed or sold charter keeps its name reserved.

**Speed**
- **R3-31, threat lists** (review 3): heals, Fade and logouts look through every creature on the map for the threat
  lists they're on, about 0.35 ms each. A per-unit index of who holds them would fix it.
- **R4-42 to R4-45** (review 4):
  - the quest-object pass;
  - the trap scan over every object;
  - full scans of mail and auctions;
  - every bag slot resent after any change.
- **The whole world awake** ([PERFORMANCE.md](PERFORMANCE.md)): staggered timers, the awake set worked out once per
  update, and navmesh tiles loaded off the map's thread and unloaded when unused. All four are in hand
  (`work/perf2`).

**Policy, for you** ([SITREP.md](SITREP.md)): the listen address (#53), wrong-password lockout (answered: none for
now), and account creation (answered: `auto` stays).

## Checked and sound
- **Connections:**
  - the handshakes' timeouts, size caps and panic guards;
  - the outbound budget and backpressure;
  - the bad-packet and rate limits;
  - the session-key cache's bounds.
- **Supervision:**
  - restart, quarantine and back-off;
  - the panic hook;
  - unwinding in release.
- **Locks:** the registry takes its locks in one order.
- **Arithmetic:** damage, absorb, block and timer arithmetic, wherever looked.
- **Weather and invasions:** the weather, the invasions' and Omen's timers.

## A correction
I reported that GM account commands missed lowercase names (`.ban alice`). Review 2 found the accounts table's
`name` is `COLLATE NOCASE`, so they never did. A test now keeps it so.
