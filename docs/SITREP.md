# Sitrep (2026-10-18)

Two days of work toward the goal you set: align wvs with Thistle Tea, using mangos-classic as a second reference;
fix the kobold's reach; focus on performance and robustness; write down every decision and question here; then
review the code. [STATE.md](STATE.md) describes where wvs stands now. [DECISIONS.md](DECISIONS.md) keeps every
question and decision in full; this page is the index.

## In short
- **Parity:** 5,656 of Thistle Tea's 8,373 applicable tests have a wvs test that checks the same behaviour, or an
  accepted difference (67.6%, from 21% when the matrix was first mapped). The rest are listed by area in
  [parity/matrix.csv](parity/matrix.csv).
- **The kobold:** fixed. A unit no longer swings from beyond its reach; two running units get the client's 8/3 yards.
- **Robustness:**
  - a panic costs one step, creature turn or packet, not the server;
  - a map that keeps panicking restarts;
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
- **#2 A restarted map's players:** they're saved and sent to the character screen. Should they go straight back in
  instead?
- **#4 wow_messages' crash bugs:** guarded in wvs and fixed in your fork. Report them upstream, which would be public?
- **#49 Dungeon copies:** give each copy's creatures fresh guids? Today copies share them.
- **#26 Reconnect protocol:** both 2 and 3 are accepted. Which does the 1.12.1 client send?

**Combat and creatures**
- **#28 A charm's end:** should it interrupt the freed creature's cast (Thistle Tea), or let it go on (mangos, wvs)?
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
- **#37 Lower spell ranks:** should lower ranks of rage and energy spells leave the spellbook, as in mangos?
- **#29 Talent reset cost:** fall back to 1 gold after a month (Thistle Tea), or stay at 5 (mangos, wvs)?
- **#48 Druid form attack power:** which is 1.12's? Thistle Tea, mangos and wvs all differ.
- **#8 Failed openings:** hold back `SMSG_SPELL_GO` for one, as Thistle Tea does?
- **#57 A passive party aura on its caster** (from 5d): shown on the caster too, as mangos-classic's
  `IsNeedVisibleSlot` has it, or hidden, as Thistle Tea and wvs have it? Which is 1.12's?
- **#58 Buffs on a shapeshifted target** (from 5d): refuse one that shapeshifting cancels (Water Walking), as Thistle
  Tea does? mangos-classic and wvs allow it.

**Pets**
- **#19 and #44 Stay:** should Stay from the action bar stop the pet's attack (mangos, wvs), or keep the fight
  (vmangos, Thistle Tea)?
- **#40 Healing:** how does a creature a player controls heal out of combat? Thistle Tea, mangos and wvs all differ.
- **#51 A creature-owned pet's death:** what does the killer get? Nothing (mangos, wvs), or three quarters of the XP
  (Thistle Tea)?

**Quests, items and the world**
- **#56 Unlearning a profession** (from 4d): should it drop that profession's quests, as Thistle Tea does? mangos and
  wvs don't.
- **#23 An untapped creature's soul shard:** give one, as mangos does, or none, as Thistle Tea?
- **#36 The Warsong flag:** may a mounted player pick it up? wvs refuses, as Thistle Tea does.
- **#52 `.honor week`:** it loses the rest of the week's honor. When should the week re-rank?
- **#6 Water:** keep breaking what water breaks by the terrain's liquid too (mangos), or by the swim flag alone
  (Thistle Tea)?
- **#12 Gates of Ahn'Qiraj:** add zone 3478 to the Cenarion Favor's zones, as mangos does?
- **#14 A timed summon of despawn type 0:** wait out its fight (vmangos, wvs), or go at its time (Thistle Tea)?
- **#15 Script teleport with a player source:** teleport them (vmangos, wvs), or do nothing (Thistle Tea)?
- **#17 Quest 2078:** start its follower from the gossip option, as Thistle Tea does?
- **#24 Model heights:** dbc.sqlite's `CreatureModelData` columns are shifted by one. Is that known?

## Your list of 2026-10-19 evening, and my calls
Being done by agent agents, four at a time; merged after the fast suite (the whole extended suite at most every 4
hours), the crate version bumped with each merge.
- **In progress:** Frost Nova on a Young Wolf, a critter's death not showing and `.kill`'s delays, stable masters for
  non-hunters, dead quest givers' icons (`fix/bugs1019`); a 400-yard view on flight paths, boats across continents
  (`feat/travel`); raid roles, buffs, strong raiders, healers' spots, druid forms, hunter pets (peon
  `feat/raidroles`); the fork's spell-miss layouts.
- **Next:** sitting to eat, bones (`feat/body`); human-like raid movement, the auction house wall (peon
  `feat/raidmove`); then the Thistle Tea alignment.
- **My calls:**
  - **Stable masters:** mangos's way: a non-hunter gets the stable master's greeting window, without the stable
    option, rather than no flag at all.
  - **Hunter pets (your item 11):** wvs already has taming, the stable and pet training (round 5), so I took it as
    the raid's hunters having pets and using them (and checking the server's pets work end to end on the way).
  - **Bones:** 18 hours as you asked, a setting; mangos's own figure goes here when the agent reports it.
  - **Strong raiders:** 1%, in their class's tier set at 60 (a new `.gear raid`), a setting.
  - **Raid AI:** two agents, one for roles, buffs and spells, one for movement and pathing, to keep their files apart.

**Merged from the evening list** (0.1.5, 0.1.6):
- **Boats (0.1.5):** the cause was the movement layout: the 1.12 client marks "on a transport" with flag
  0x02000000, the ship's full 8-byte GUID and no timestamp, where 12932's wow_messages fork expects 0x200, a packed
  GUID and a timestamp. wvs read no real player as aboard: walking on a deck tripped the speed check into the sea,
  and the crossing left them behind. wvs now converts the real layout at its edges (`movement_wire`), and crosses as
  mangos does (`SMSG_TRANSFER_PENDING` and `SMSG_NEW_WORLD` with the deck offset, the boat added before its
  passengers and removed after); passengers' server positions follow the deck; every two-map route crosses both
  ways. **Then (0.1.10):** the fork's vanilla movement is now 1.12's (`0f646a2`: the on-transport flag 0x02000000
  with the whole GUID and no timestamp, in both `MovementInfo` and the create's `MovementBlock`; and a u32 fall time,
  where it had a float: peon's landings had been telling servers about 1.1 billion ms), checked against the client's
  reader; `movement_wire` is gone, and wvs, peon and rustigator all pin the fork there. Left: logging in on a boat doesn't attach you to it; the Grom'Gol-Undercity zeppelin's in-map jump
  sends no near teleport.
- **Flight view (0.1.5):** `[visibility] flight_yards` 400, only on a flight master's mount, measured on the ground;
  others see the flier at their own distance.
- **Food (0.1.6):** an aura that standing ends seats a living player (mangos's `_AddSpellAuraHolder`); moving,
  turning, jumping or lying down ends the meal. Only players sit (mangos sits any unit). Jumping ends it as you asked,
  though mangos's mask doesn't include the jump.
- **Bones (0.1.6):** a risen player's corpse becomes bones (flags 0x05, gear slots cleared), seen by everyone, for
  `[players] bones_hours` (18; 0 for none); not saved across a restart, as mangos never saves them. **mangos's own
  numbers:** an unclaimed corpse turns to bones after 3 days (checked every 20 minutes); bones last until their grid
  unloads, about 5 minutes with no one near. wvs has no 3-day expiry for unclaimed corpses yet.
- **Raid movement (peon `cec3e2a`):** raiders jump, side-step, back up, pause, turn with a reaction delay and walk off
  the exact path line, each at its own seeded rhythm (`--raid-liveliness`, 0 for the old straight lines); none
  flagged by the speed check in 100-raider runs. **The auction house wall:** the bake has its walls and the door is
  the only way in, but its auctioneers stand on a dais the navmesh doesn't join to the floor, so the path failed and
  peon ran a straight line through the wall; now it goes as near as the navmesh does and never runs a line where the
  navmesh has no way. **Found for rustigator and the fork** (being fixed, `navfix`): the map builder doesn't shrink
  walkable areas by the agent's radius (polygons reach into walls; a re-bake will be needed), a few spots where the
  collision and the navmesh disagree (a stair foot at (-8783, 638)), and the fork's `clear_*` flag helpers wiping
  every flag.
- **Four in-game bugs (0.1.8):** the 1.12 client, within one update packet, runs a field's display handlers only for
  the last values block of an object (it compares each block with a copy taken before it), and wvs sent a creature's
  tap in a second block: so a dead Rabbit's frame stayed at 100% with no animation, and Frost Nova's ring never
  showed. Each viewer now gets one block, its tap view in it (mangos's and Thistle Tea's way). Frost Nova's wolf
  also left its fight at the root and was never told it was held: a root keeps the fight, a held creature stops, faces
  and swings in reach, and the client gets `SMSG_SPLINE_MOVE_ROOT`. Dead creatures lose their NPC flags until they
  respawn (no quest icon, gossip, vendor). A non-hunter at a stable master gets the default greeting window (text 68;
  mangos answers with nothing; vmangos has no stable text). **Left:** a game object's quest sparkle can go in a
  second block too (rare).
- **Raid AI, roles (0.1.7, peon `1bd7d04`):** each raider a tank, healer or damage from its seed (about 1 tank and
  2.3 healers per 10: warriors tank 45%, druids tank 30% and heal 40%, priests heal 90%, shamans 80%, paladins tank
  20% and heal 50%); warrior tanks in Defensive Stance with Taunt, Shield Block, Revenge, Sunder; druid tanks in bear
  form; buffs out of a fight to whoever lacks them (healers any time), Battle Shout and totems in one; 1% strong
  raiders (`--raid-strong`) in their class's tier 1 or 2 set at 60 (`.gear raid`), the best epics below; healers
  pick a spot every 2 s that reaches the most wounded in sight, out of melee; hunters back off 12 yards from a foe
  within 8, then shoot; druids in bear and cat form, leaving it below 30% to heal; hunters get a beast with a new
  `.pet` (no beasts of their level near Stormwind to tame), warlocks a Voidwalker. **Left:** raiders aren't grouped,
  so party buffs and totems reach only the caster; healers don't favour tanks; Shield Block needs `.gear` to hand a
  warrior a shield.
- **The navmesh, format 13 (0.1.14; you must re-bake):** rustigator now shrinks the walkable area by a player's
  radius (mangos's 0.533 yards), so paths keep off walls instead of grazing them; wvs, peon and rustigator pin the
  fork at `2c9fe11` (its `clear_*` helpers clear only their own flag; 1.12's walk speed) and rustigator at `921d510`.
  **Your part:** run `cargo run --release --bin wvs-setup` once (about 20 minutes for every map). It sees the old
  bake as stale and builds it again; until then wvs runs without a navmesh (straight-line movement, as with none).
  Peon's real-navmesh tests and wvs's navmesh tests pass on the new bake.
- **The possible client crash (0.1.9): not a crash.** `0x466816` is the client's per-field-group flush after a values
  block (`0x4667A0`), a copy from storage the real client set up at the create and the emulator never models; it reads
  nothing from the packet. The check now stubs it, as it stubs the renderer; the values block's parse is still checked,
  and all six real crash packets still fail it. A test covers a ghost rising before a stranger.

## Round 6: the Thistle Tea alignment, by area
Agent agents, one area each (6a player, 6b spells, 6c the world's loaders, 6d to 6j the rest), merged as they finish.
- **6a, the player (0.1.12):** tabard designers and charters, auction houses by faction, Evil Twin turning summonses
  away, a new character's home where it starts, a body-less login up at half health and mana, talents reset when the
  level is too low.
- **6b, spells (0.1.11):** a cast checked again as it ends, costs by rank, minimum ranges, threat from crits, ticks
  and heals, a cast's result before its `SMSG_SPELL_GO`, harmful casts' targets, buff ranks by level.
- **6c, the world's loaders (0.1.13):** `.tele` by an exact name, a prefix, then a part (mangos's `GetGameTele`);
  superseded ranks hidden; a creature's channel broken by damage; crowd control broken only by the spells whose
  immunities cancel it (Blink no longer works while silenced); Shield Bash and Hamstring missed whole when blocked
  (mangos's `CanBlockAbility`); `.start` casts Stuck (mangos's `HandleStartCommand`); the Threshwackonator (quest
  2078) follows from its own gossip. **My calls:**
  - **Stuck** still takes you to where you last stood on the ground, as Thistle Tea does. mangos's `EffectStuck`
    sends a ghost to its graveyard and uses the Hearthstone if it is ready; tell me if you want mangos's.
  - **Blink under a stun and a silence at once** is refused (mangos checks the silence after the stun); Thistle Tea
    allows it. `wrong-for-1.12`.
  - **Blessing of Protection** on someone with an aura that invulnerability cancels takes the aura off (mangos);
    Thistle Tea refuses the cast. `wrong-for-1.12`.
  - **A model's collision height** is `CreatureModelData`'s times the object's scale (mangos); Thistle Tea
    normalises it by the model's and display's scales. `wrong-for-1.12`.
  - **A spawn at 0% health** is at full health (mangos); Thistle Tea spawns it dead. No real spawn has 0.
  - **A creature's channel** stops on damage (mangos's `DealDamage`); Thistle Tea stops those with flag 0x1 when the
    creature enters a fight.
  - **Left as gaps:** casting a quest's `SrcSpell` on acceptance (Moonglade's teleport, 5921) and auto-rewarding
    flag quests (Naxxramas's attunement, 9378) were tried, had no tests, and were dropped.

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
- **#55** the world-awake fixes landed (perf2, DECISIONS 57).
- **2026-10-19, done:**
  - **#3** the watchdog is removed (DECISIONS 61).
  - **#13** item charges are saved (DECISIONS 60).
  - **#54** mangos's defaults: instance view 170, leash 30, assistance 5, rare corpses 900, 1200 and 3600 s
    (DECISIONS 61).
  - **#20** the player combat timer as mangos has it: no PvE timer, 5 s between players (DECISIONS 62).
  - **#33** a teleport within the map ends the fight, but for mangos's near teleports under 100 yards, Blink's
    among them (DECISIONS 62).
  - **#39** new weapon skills start at 1, and skills are now saved (DECISIONS 62).
  - **#11** nothing reaches a client about an object before its create (DECISIONS 63).
- **2026-10-19, guards:** mangos-classic's guard call, guards gone 5 minutes after a fight and wandering meanwhile,
  each town's alarm (Avoidance from 20, two elite from 40) and Jaina from Theramore to Stormwind from 60, her
  Teleport there to the Stockade's door and her respawn the standard day (DECISIONS 65). peon's `--mode raid` and
  wvs's `.gear` drive it (DECISIONS 66).
- **2026-10-19, dependencies:** every crate at its newest release, and `rayon`, `pprof`, `dotenvy`, the regex log
  filter and two unused features gone (DECISIONS 64). **Tracy now needs viewer v0.14.1.** No `.env` is read.
- **2026-10-19, second batch, done** (DECISIONS 69): #35 a second login takes the character over; #43 fears break
  by chance; #42 an interrupt locks a silence-immune creature's school (it already did); #30 attack-speed slows
  multiply; #46 Fade gives back only what it took; #41 cancelling Stealth with Vanish on removes both; #47 quest-log
  slots go to the group; #21 the grey "!" to 7 levels short (a setting); #7 locked doors and buttons need their key.
  Also: trainers offer a talent's ranks only with the talent, and `SMSG_SPELLLOGMISS` checked against the client.
- **#60** fixed in your fork (`27dfce0`): 1.12's spell misses are a byte reason, with `SMSG_SPELL_GO`'s reflect
  result and the miss log's debug floats; wvs, peon and rustigator all pin it (wvs 0.1.4).
- **2026-10-19, the raid's fixes:** queries have their own rate, `.gear` fills health and mana, `.character spells`,
  guards and land-bound swimmers wander, raiders on the ground and casting (DECISIONS 68).
- **2026-10-19, the review's findings ([CODE_REVIEW.md](CODE_REVIEW.md)):** H1 (the transport flag skips the
  speed check) and M1 (the speed check ignores height) are accepted as they are; M2 (two members entering a
  dungeon at once can get different copies) and M3 (a shutdown can miss a player between maps) are to fix, not
  started; M4 (a pool saved at 0 loads full) explained, waiting on you.
- **#59 Roots and silences break by chance** (from the second batch): mangos breaks roots (aura 26) and
  pacify-silence (aura 60) on damage by the same chance as fears; wvs does fears only. The same for those?
- **2026-10-19, kept as they are:** #53 login and world listen on every interface ("absolutely fine"); #38
  vmangos's spell corrections; #22 an abandoned quest's collected items.

## Progress
| Stage | Result | Commit |
|---|---|---|
| The kobold's reach | fixed | dd2d397 |
| Phase 1a: the matrix of Thistle Tea's tests, mapped | 1,781 of 8,429 covered (21%) | f9bad00 |
| Bugs found while mapping, batches 1 to 5 | merged | 25ecb91 … f09cc53 |
| Phase 2 (2a to 2i): spells, movement, combat, scripts, instances, world | merged | 099722e … ef1db26 |
| Robustness: supervision, guards, chaos tests (the watchdog since removed) | merged | 5f53bef |
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
| Dead bodies stay put; a dead GM commands by whisper | fixed | d22766c, c4496f8 |
| `.spawn` with its weapons and stats | fixed | 81706f1 |
| ahash's maps only | merged | 8a1e3ad |
| flaky1: the load-only failures | merged | d4ed4ad |
| 4d: items, quests, objects, events | 63.4% | cacd475 |
| Molten Core's fire after flaky1; Stratholme's flake | fixed | 040ee2f, and the next |
| 5c: combat | 64.5% | a8bb809 |
| perf2: the whole world awake, round two | merged | its merge |
| 5d: auras and objective carriers | 66.2% | ce71ae1 |
| 5e: pets and creature scripts | 67.5% | 145daf9 |
| 6b: spells | merged | b1deacf |
| 6a: the player | 69.5% | 99266b6 |
| 6c: the world's loaders | 70.2% | cd3605e |

**Running now:** round 6 (6d to 6g), and `fix/extended1019` for the four tests the 15:38 extended run failed.

Their results go into [DECISIONS.md](DECISIONS.md) and this table as they merge.
