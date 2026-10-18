# Sitrep (2026-10-18)

Two days of work toward the goal you set: align wvs with Thistle Tea, using mangos-classic as a second reference;
fix the kobold's reach; focus on performance and robustness; write down every decision and question here; then
review the code. [STATE.md](STATE.md) describes where wvs stands now. [DECISIONS.md](DECISIONS.md) keeps every
question and decision in full; this page is the index.

## In short
- **Parity:** 5,110 of Thistle Tea's 8,380 applicable tests have a wvs test that checks the same behaviour, or an
  accepted difference (61%, from 21% when the matrix was first mapped). The rest are listed by area in
  [parity/matrix.csv](parity/matrix.csv).
- **The kobold:** fixed. A unit no longer swings from beyond its reach; two running units get the client's 8/3 yards.
- **Robustness:**
  - a panic costs one step, creature turn or packet, not the server;
  - a map that keeps panicking restarts, and a watchdog reports a hung one;
  - items and money that move between characters and records are saved in one transaction;
  - four code reviews fixed about 60 bugs.
- **Performance:**
  - CPU is down 38–77% across the measured scenarios;
  - the world loads ten times faster;
  - a whole-world load test and its benchmark exist;
  - Tracy is reworked.
- **Client crashes:**
  - the race crash (a 2-point creature spline) is fixed;
  - a harness now runs `WoW.exe`'s own packet parsers on what wvs sends, and reproduces every crash seen so far.
- **Your requests:**
  - fast tests: the unit suite runs in 25 s, and the extended one is separate;
  - configurable settings, with `WVS__` overrides and `.reload config`;
  - ghosts walk on water and see only near their corpse;
  - a 400-yard battleground view distance.

## Open questions
Each number is the question's number in [DECISIONS.md](DECISIONS.md), where it is written out with its options.
Until you answer, wvs keeps what the line says.

**Server and operations**
- **#53 The listen address:** login and world listen on every interface. With `creation = "auto"` and
  `everyone_gm = true`, anyone who reaches them gets an administrator. Default to localhost?
- **#3 A hung map:** should it abort the server, so systemd or Docker restarts it? Today the watchdog only logs it.
- **#2 A restarted map's players:** they're saved and sent to the character screen. Should they go straight back in
  instead?
- **#4 wow_messages' crash bugs:** guarded in wvs and fixed in your fork. Report them upstream, which would be public?
- **#54 Defaults that differ from mangos:**

  | Setting | wvs | mangos |
  |---|---|---|
  | Instance view distance | 100 | 170 |
  | Leash radius | 50 | 30 |
  | Assistance radius | 10 | 5 |
  | Rare corpse decay | wvs's own | mangos's |

  Switch?
- **#55 The world-awake fixes:** running now (`work/perf2`). Stop them, or let them land?
- **#49 Dungeon copies:** give each copy's creatures fresh guids? Today copies share them.
- **#26 Reconnect protocol:** both 2 and 3 are accepted. Which does the 1.12.1 client send?

**Combat and creatures**
- **#20 The player combat timer:** move to Thistle Tea's and mangos's model? That is no PvE timer, 5 s for PvP.
  Explained, waiting on your go.
- **#33 Teleports:** should a teleport within the map end the player's fight, as mangos and Thistle Tea do?
- **#28 A charm's end:** should it interrupt the freed creature's cast (Thistle Tea), or let it go on (mangos, wvs)?
- **#42 Interrupts:** should one lock out the school on a creature immune to silence? mangos does, Thistle Tea doesn't.
- **#43 Fears:** should a fear without the damage-break flag break by chance, as in mangos?
- **#9 Battleground creatures:** give them back a home guard of 60 yards?
- **#16 Eranikus:** is evading when he can't reach Remulos right?
- **#31 A creature-owned totem:** does it outlive its owner (Thistle Tea), or go with it (mangos, wvs)?
- **#32 A scripted follower's evade:** where it stands (Thistle Tea), by its leader (mangos), or its spawn (wvs)?
- **#45 Corpses:**
  - a fallen hound with no kin: lootable (wvs) or despawned at once (Thistle Tea)?
  - a dead pet's corpse: 60 s and 5 s (mangos), or an hour and 15 s (Thistle Tea)?
- **#50 Nerub'enkan's scarabs:** they vanish on arrival by vmangos's own EventAI. Keep that, or exempt them?
- **#1 Death's wear:** three small differences from Thistle Tea. Align them?

**Spells, auras and talents**
- **#38 vmangos's spell corrections:** keep them winning over the raw DBC Thistle Tea reads? Concussive Shot and
  Piercing Howl are dazes, for example.
- **#37 Lower spell ranks:** should lower ranks of rage and energy spells leave the spellbook, as in mangos?
- **#39 New weapon skills:** start them at 1, as in mangos and Thistle Tea? wvs starts them at their cap.
- **#30 Attack-speed slows:** keep them one at a time (Thistle Tea), or multiply them (mangos-classic)?
- **#29 Talent reset cost:** fall back to 1 gold after a month (Thistle Tea), or stay at 5 (mangos, wvs)?
- **#41 Stealth and Vanish:** should cancelling Stealth while Vanish is on remove both?
- **#46 Fade:** when it ends, give back only what it took (mangos), or its whole amount (Thistle Tea, wvs)?
- **#48 Druid form attack power:** which is 1.12's? Thistle Tea, mangos and wvs all differ.
- **#8 Failed openings:** hold back `SMSG_SPELL_GO` for one, as Thistle Tea does?
- **#13 Charged items:** they come back full after a relog, a mail or an auction, since charges aren't saved. Fix?
- **#11 Packet order:** an update's blocks go at its end, so a packet naming a new object can arrive first. Fix it
  generally? It touches the hot path.

**Pets**
- **#19 and #44 Stay:** should Stay from the action bar stop the pet's attack (mangos, wvs), or keep the fight
  (vmangos, Thistle Tea)?
- **#40 Healing:** how does a creature a player controls heal out of combat? Thistle Tea, mangos and wvs all differ.
- **#51 A creature-owned pet's death:** what does the killer get? Nothing (mangos, wvs), or three quarters of the XP
  (Thistle Tea)?

**Quests, items and the world**
- **#56 Unlearning a profession** (from 4d): should it drop that profession's quests, as Thistle Tea does? mangos and
  wvs don't.
- **#22 Abandoning a quest:** keep its collected items (mangos, wvs), or destroy them (Thistle Tea)?
- **#21 The grey quest "!":** show it up to 4 levels below (wvs), 7 (mangos), or at any gap (Thistle Tea)?
- **#23 An untapped creature's soul shard:** give one, as mangos does, or none, as Thistle Tea?
- **#35 A second login:** kick the first session (wvs), hand over the character (mangos), or refuse (Thistle Tea)?
- **#36 The Warsong flag:** may a mounted player pick it up? wvs refuses, as Thistle Tea does.
- **#47 Quest-log slots:** visible to everyone (wvs), the group (mangos), or no one (Thistle Tea)?
- **#52 `.honor week`:** it loses the rest of the week's honor. When should the week re-rank?
- **#6 Water:** keep breaking what water breaks by the terrain's liquid too (mangos), or by the swim flag alone
  (Thistle Tea)?
- **#7 Locked doors and buttons used by hand:** refuse them without the key, as Thistle Tea does?
- **#12 Gates of Ahn'Qiraj:** add zone 3478 to the Cenarion Favor's zones, as mangos does?
- **#14 A timed summon of despawn type 0:** wait out its fight (vmangos, wvs), or go at its time (Thistle Tea)?
- **#15 Script teleport with a player source:** teleport them (vmangos, wvs), or do nothing (Thistle Tea)?
- **#17 Quest 2078:** start its follower from the gossip option, as Thistle Tea does?
- **#24 Model heights:** dbc.sqlite's `CreatureModelData` columns are shifted by one. Is that known?

## Answered
- **#10** keep swinging on arrival.
- **#18** keep moving unusable gear off at login.
- **#27** snares stack, as mangos-classic has them.
- **#5** a knockback doesn't move a server-run creature.
- **#34** a bad deck offset is dropped, as mangos's `VerifyMovementInfo` drops it (review 2).
- **#25** the fork's crash bugs and layouts are fixed (round 2).
- **Passwords and accounts:** no lockout after wrong passwords, and `creation = "auto"` stays. Both deferred.
- **Ghosts:** water walking, and sight only within 45 yards of the corpse, for every kind of object.
- **Battlegrounds:** a 400-yard view distance.

## Progress
| Stage | Result | Commit |
|---|---|---|
| The kobold's reach | fixed | dd2d397 |
| Phase 1a: the matrix of Thistle Tea's tests, mapped | 1,781 of 8,429 covered (21%) | f9bad00 |
| Bugs found while mapping, batches 1 to 5 | merged | 25ecb91 … f09cc53 |
| Phase 2 (2a to 2i): spells, movement, combat, scripts, instances, world | merged | 099722e … ef1db26 |
| Robustness: supervision, guards, watchdog, chaos tests | merged | 5f53bef |
| wow_messages fork: stop layout, crash bugs, seven layouts | merged | 2bed07d, 3eacdd1 |
| Performance, round 1 | merged | 8775d80 |
| Phase 3, rounds 1 and 2 (3a to 3l) | 45.1% | b8e080f |
| Fast and extended test suites | merged | eef0000 |
| Phase 3, round 3 (3m to 3r) | 53.8% | 2d71df2 |
| Review 1: saves together, reconnect | merged | 70cd32f |
| Reviews 2, 3 and 4 | merged | f90d87f, 1e8c8a5, de5cf4a |
| The race crash (create splines) | fixed | 12c055a |
| The client-code harness | merged | eb9b73b |
| Round 4 (4a to 4c) | 58.6% | afc079c |
| Ghosts, battleground view distance | merged | 7aa9d29, c380d54 |
| Tracy reworked | merged | 479b728 |
| Configurable settings | merged | 287d03a |
| The whole-world-awake load test | merged | a1e8835 |
| Round 5 (5a, 5b so far) | 61.0% | 15aee3f, and 5b's merge |

**Still running as of this writing:**
- 4d, reconciling with 5a;
- 5c, 5d and 5e;
- perf2;
- flaky1.

Their results go into [DECISIONS.md](DECISIONS.md) and this table as they merge.
