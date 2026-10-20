# Sitrep (2026-10-20)

Two days of work toward the goal you set: align wvs with Thistle Tea, using mangos-classic as a second reference;
fix the kobold's reach; focus on performance and robustness; write down every decision and question here; then
review the code. [STATE.md](STATE.md) describes where wvs stands now. [DECISIONS.md](DECISIONS.md) keeps every
question and decision in full; this page is the index.

## In short
- **Parity:** 7,317 of Thistle Tea's 8,330 applicable tests have a wvs test that checks the same behaviour, or an
  accepted difference (87.8%, from 21% when the matrix was first mapped, 67.5% before rounds 6 to 8). 814 are still
  gap, listed by area in [parity/matrix.csv](parity/matrix.csv); paused before round 9 (your word, 2026-10-20).
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

**Pets**
- **#19 and #44 Stay:** should Stay from the action bar stop the pet's attack (mangos, wvs), or keep the fight
  (vmangos, Thistle Tea)?
- **#40 Healing:** how does a creature a player controls heal out of combat? Thistle Tea, mangos and wvs all differ.

**Quests, items and the world**
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
  `[players] bones_hours` (18; 0 for none); **kept across a restart since 0.1.35** (your ask: the store's `bones`
  table; open-world bones only, a copy's or a battleground's not), where mangos never saves them. **mangos's own
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

## Rounds 6 to 8: the Thistle Tea alignment, by area
Round 7's 7g to 7m and round 8's 8a to 8f went out together as **0.1.28** (the memory stop held the tests back).
**The full extended suite at 0.1.28** (23:05, 873 s): 3,114 of 3,117 passed; the three that failed (Balnazzar's,
Windsor's and Lady Jaina's scripts) pass alone, timing under load, queued to be made to wait on what they check.

### Round 6
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
- **6g, the world's systems (0.1.15):** a trade is called off as the partners walk apart; an addon's line is never a
  GM command; the first honor after midnight goes on the new day; gaining or losing detection is seen at once
  (mangos's `HandleInvisibilityDetect`); a stealthed walker is looked at every 2 yards; the Scourge invasion's world
  states show only while it runs; ending a map event that isn't under way fails; a chain spell grounded by a totem
  stops at it (mangos's `CheckAndAddMagnetTarget`). **My calls:**
  - **A trade** is called off past 11 yards (wvs's opening distance), checked every map update. mangos opens at
    11.11 yards and cancels on movement past 5; Thistle Tea uses 10 for both.
  - **Addon-language lines** are never commands: mangos would parse them, but an addon's prefix isn't one.
  - **Stealthed units** (not invisible ones) are looked at every 2 yards of movement, everyone else every 8.
  - **Left as gaps:** a trap isn't looked at again when its owner's PvP flag or duel changes (until someone moves);
    visibility isn't checked on a turn (mangos doesn't either). 137 rows in the area are still open, mostly the
    real-navmesh pathfinding (38) and positions (20).
- **6d, combat (0.1.16):** each hand's own skill and hit, and players' caps against players; a warrior in a stance
  or a rogue in Stealth dual wields and parries (only cat, bear, travel and aquatic forms put weapons away); a
  fishing pole trains nothing; damage against a creature type added flat to spells, after the percentage on a
  weapon blow, by the coefficient on other abilities; Deep Wounds bleeds by the hand that crit; kill procs before
  the death takes the auras (Improved Drain Soul); a killing Shadowburn leaves its shard; the dead open no
  defensive window. **My calls** (mangos's unless said):
  - **Deep Wounds** uses the hand that crit (Thistle Tea's; mangos guesses it from the swing timers); the off hand's
    amount is its damage as it stands, where Thistle Tea halves it again.
  - **Weapon procs:** a wholly absorbed blow procs, a whole block doesn't; Auto Shot, Aimed Shot, Throw and Shoot
    proc the ranged weapon; a proc spell on cooldown stops the enchant rolls too (Thistle Tea goes on to them).
  - **Ghost Wolf** trains weapon skills (its form has no "no weapon" flag); **a fishing pole** still swings at
    the unarmed skill, as round 4 chose (mangos would use Fishing).
  - **The spell damage log** carries the damage less the absorb, and god mode logs the blow; Thistle Tea folds the
    absorb in and logs 0.
  - **A melee or ranged tick** takes the target's damage-taken flats whole; Thistle Tea scales them.
  - **You might override:** spell damage "up to" a creature type (aura 180) stays as spell power, as Thistle Tea;
    mangos adds it flat.
  - **Left:** a spell's procs run once per damaging effect, where both references run them once per target; 119
    rows open (fleeing for help, proximity aggro, leashes, pet combat, kill credit, Feign Death and more).
- **6e, spells as the world runs them (0.1.17):** a chain jumps in sight of the last target; group spells take pets
  (a pet's group is its master's); a harmful raid spell (Plague) spares its carrier; a duel's two sides can't help
  each other; drains and Health Funnel do nothing while their caster is dead; party auras by each member's level;
  Cannibalize from both edges and not on a flight; a spell at the pet wants it out, alive and in sight (`NO_PET`);
  spell foci looked for as far as their kind reaches; a leap with no radius stays put; script targets within the
  effect's radius, else the spell's range, else 100 yards (mangos's); script areas naming no creature hit every unit
  (Coarse Dynamite and Cozy Fire hit no one before). It also fixed the extended Charge test (6b's minimum range now
  counts from both edges, so Charge at 10 yards is too close). **My calls:**
  - **A healing chain** goes to the most health missing, the last target's group first (Thistle Tea's and mangos's
    `ChainHealingOrder`); newer mangos ranks by health percentage.
  - **`wrong-for-1.12`, mangos's way:** no level filter on raid areas; Cannibalize won't eat the body of a player
    who is offline; no projection of the caster's movement for areas about the caster; spells at any unit
    (dispels) may be cast at duellists by others and at units immune to players.
  - **No `SMSG_CLEAR_COOLDOWN`** when Cannibalize is refused (mangos sends none).
  - **A pet's spell area rules** are its master's race, gender, quests, auras and place.
  - **Left:** per-effect script target masks (four 1.12 spells, all on objects), summons and teleports to a unit's
    place, Void Zone's lift, game objects' own spells; about 60 rows in the area untouched.
- **The 15:38 extended run's four failures (0.1.17, `fix/extended1019`):** none a crash. Round 6b's minimum range
  (beyond both units' reaches, mangos's `GetMinMaxRange`) put Charge's test and the starting spells' kobold too
  close; the tests moved, and the Blink test now expects fear and Polymorph to hold (Blink cancels stuns and roots
  only, round 6c). **A real bug:** a standing raised to Neutral never ended its war, so with 6a's interaction rule a
  player raised from Hated with the Brood of Nozdormu couldn't take Anachronos's quest; it now ends, as mangos's
  `SetOneFactionReputation` (Thistle Tea never clears it). Triage's test was racing the doctor's one-a-second pace
  on a loaded machine; it waits by the clock.
- **6f, creature scripts (0.1.18):** 73 rows, every one a test of a ported script: the Affray, Riggle Bassbait's
  week, Lady Jaina in Theramore, Unbagwa's waves, the Scourge cauldrons and watchtowers, Naralex's waking, Zul'Farrak
  (Zum'rah, Weegli, Bly), the Stave of the Ancients, the Scourge Invasion, and a dozen smaller ones. **A real bug:** a
  script's damage to the creature itself (command 48) stopped at its invincibility floor, so a necropolis never
  fell (and Reginald Windsor never died); it now goes past it, as Thistle Tea's `lose_health` (vmangos's intent).
  **My calls:** Big Will stops at the Affray ring's edge on the navmesh, 14 to 16 yards from its middle; I left it.
  **Left:** 65 rows (Alterac Valley 16, Gnomeregan 9, the Gizelton Caravan 9, Eris Havenfire 8, the Stormwind
  Rendezvous 8, Eranikus 7, Kindal 7). **To look into:** a test hero was at war with Booty Bay before any fight
  (the agent worked around it); it may be the reputation fix above or a creature's temporary war.
- **6h, creature AI (0.1.19):** a repeat shot (Auto Shot, wands) is checked again each shot: pacify, Hawk Eye's
  range, the dead zone from both reaches, sight, a stealthed player-side target; a swing error is told once while it
  lasts (mangos's `LastSwingErrorMsg`); no swing at a victim gone unseen; a pet faces before its first swing; 30 of
  Thistle Tea's imported trigger, object and event scripts tested on the real world. 6d and 6h both gave each hand
  its own weapon skill; I kept 6d's (it also has the skill against players). **My calls:**
  - **Auto Shot stops when its target goes out of range** (mangos's `_UpdateAutoRepeatSpell`), reversing round 4b's
    "Auto Shot waits through both".
  - **The stealth check on shots** is for player-side targets only (mangos's `CheckCast`); Thistle Tea checks every
    target.
  - **Restarting an attack clears the swing error** (Thistle Tea; mangos clears it on a landed swing).
  - **A failed swing** leaves the hands due and retries at the next update, keeping a later main-hand deadline
    (Thistle Tea); mangos sets both to 100 ms.
  - **Melee swings don't check line of sight** (mangos); Thistle Tea does.
  - **Facing within 1.4 yards** isn't forgiven (mangos's `HasInArc`); Thistle Tea's is `wrong-for-1.12`.
  - **Left:** about 150 rows (distancing, formations and following, spreading attackers, pet autocast, waypoints);
    Silithus's wind stones have no steps in Thistle Tea's scripts and would be ported by hand.
- **6i, entities in the world (0.1.20):** any spell of a player's that reaches another player's flagged side (the
  player, their pet or totem) flags the caster, hit or miss (mangos's `Spell::FillTargetMap`): heals, Sap and marks
  did not; a threat-making heal on someone contested contests the healer. A creature's Summon Pet now makes its
  demon (NPC warlocks had none); a pet's instant spell starts its fight. **My calls:**
  - **PvP flagging is mangos's:** Sap, a no-threat harmful spell (Beast Lore, Mind Vision) and a "threat only on a
    miss" spell that hits all flag the caster; Thistle Tea flags no one for them (`wrong-for-1.12`). Sap still
    starts no fight.
  - **A creature's demon is a guardian** (mangos): defensive, several on recasts, kept after the summoner dies and
    gone when it leaves the world; Thistle Tea gives one aggressive pet (`wrong-for-1.12`).
  - **Left:** 133 rows (game objects' casts and summons, event sinks, corpse group loot); the force-move counter is
    always 0 where mangos counts up per session; spells with mangos's PvP attribute should flag the victim too.
- **7a, the player, round 7 (0.1.21):** 58 rows. Fixes: gear bound to another map closes its open loot window
  (mangos's `Loot::~Loot`); a place's spell auras are judged again when an aura they depend on comes or goes, and at
  a resurrection (mangos); meeting stones refuse the dead, the flying, one controlling another and an unusable
  stone. New GM commands: `.debug position [guid]` with the facing, and `.instance data [field]` (read only).
  **My calls:**
  - **Meeting stones** refuse the dead (Thistle Tea; mangos doesn't check), the flying and a controller (mangos).
  - **Remote bank access:** mangos checks for a banker only when buying a slot, so a client can reach its bank from
    anywhere; Thistle Tea refuses without a banker near. I chose **Thistle Tea's check** (it is how the real
    server behaved, and it stops a remote-bank exploit); it goes to the next player round. Tell me if you'd rather
    keep mangos's.
  - **Left:** 241 rows, mostly bigger designs: movement counters and acks, Charge as server movement, cross-map and
    corpse-targeted resurrection, a game object's gossip option, quests' source spells, auto-rewarded event quests.
- **6j, the core's other areas (0.1.22):** 46 rows, 9 fixes: one with an invitation pending can't invite (mangos's
  `HandleGroupInviteOpcode`); a new guild charter ends the owner's older petition; addon auras' ids parsed right; a
  duel's end takes off the harmful auras each side and its pet put on the other, and the combo points (an opponent's
  DoT could kill the loser); the arbiter set at the challenge; a blow leaving exactly 1 health ends the duel; a
  creature is the kill of the group it was tapped for (mangos's `SetLootRecipient`); a mind-controlled player's tap
  is their controller's; a script's faction cleared over a respawn goes back to the creature's own; a refreshed
  transform shows again. **My calls:**
  - **A pet's harmful auras** come off at a duel's end too (Thistle Tea; mangos only the opponent's own).
  - **A duel's end leaves the target** selected (mangos; Thistle Tea clears it).
  - **A tap whose group has disbanded** rewards the tapper's current group (Thistle Tea; mangos the tapper alone).
  - **Your own reflected spell** can kill you in a duel and interrupts it (mangos); Thistle Tea stops at 1 health.
  - **No experience for killing any pet,** an NPC's included (mangos-classic's `XP::Gain`); Thistle Tea, after
    vmangos, gives three quarters. `wrong-for-1.12` unless you take vmangos as the reference here.
  - **Left:** about 539 rows in its wide area: pets (~70), creatures (~65), auras (~55), movement (~49), stats (34),
    and movement acks (~20, a missing feature).
- **7b, spells, round 7 (0.1.23):** cast-end procs (nothing raised them, so Unstable Power and Ascendance never
  spent their stacks and Elemental Focus never procced); a fear that Recklessness's kind stops doesn't block casts;
  a spell's teleport leaves a flier on their flight (mangos's `IsTaxiFlying`); a heal's tick procs only what a rule
  asks (vmangos); a channel ends past 1.5 times its range, on a jump, on a turn if it says so, and as it begins if
  its target avoids it whole (mangos's `WriteSpellGoTargets`); spell groups refuse whole and bounce a weaker buff
  at the check (`AURA_BOUNCED`); Sap, Distract and Duel refuse a target in a fight. **My calls:**
  - **A channel's range** is mangos's 1.5 times for every channel; Thistle Tea uses vmangos's 1.3 for hostile
    targets (`wrong-for-1.12`).
  - **Triggered casts** raise no cast-end proc (Thistle Tea; mangos raises one for every cast); the proc runs after
    the hits, so every target of an area cast gets Unstable Power's full bonus (Thistle Tea).
  - **Any facing change** is a turn; mangos allows 0.1 radians, and more for channels that track their target.
  - **Left:** Thistle Tea's proc origins (spells triggered by auras don't proc), deferred cooldowns, summons by
    compass offset, traps and the Transporter.
- **7c, the loaders, round 7 (0.1.25, with 7d):** 71 rows. Fixes: a weather chance over 100% or below 0 is 25, as mangos's
  `LoadWeatherZoneChances` repairs it; a flag quest (0x400) is rewarded when its event happens (the Naxxramas
  attunement, mangos's `AreaExploredOrEventHappens`); a quest's source spell is cast on accepting it (a druid taking
  Moonglade learns its teleport; round 6c had tried and dropped both); a swing ends the attacker's auras that
  attacking interrupts (Honorless Target, Shadowmeld; mangos's `AttackerStateUpdate`). **My calls:**
  - **Miran's raiders** say mangos's line (512, "Get him!") as they attack; Thistle Tea's first raider says 1936.
  - **A soulstone's health** is capped at the player's maximum (mangos).
  - **vmangos's `spell_effect_mod`** stays applied: 396 wild summons where the client's data has 372, and Summon
    Doomguard a wild summon.
  - **Left:** 104 rows: proc rules, spell modifiers, combat-control attributes wvs doesn't read, pets' helpful
    autocast, dispel resistance (Vile Poisons), Plagueheart's critical threat, captures by muzzle and collar.
- **7d, combat, round 7 (0.1.25):** 65 rows. **Feign Death:** a believed one hides the hunter from creatures' notice
  and picks (mangos's `CanAttackOnSight`, `IsOfflineTarget`), interrupts casts at her, stops Auto Shot either way;
  a pet's blows no longer pull a dead or feigning master back in. **Kill credit:** a creature a player controlled
  is worth nothing (its death had freed it before the reward). **Leashes:** a summon fighting before its summoner
  joins the summoner's clock. **Fleeing for help:** shows no target while it runs, ends when its victim changes or
  goes, stops for a distraction, runs on at once when a root ends, and isn't stalled by a speed change. **My calls**
  (mangos's):
  - **Feign Death with a pet fighting** ends the hunter's fight, and she can't be pulled back in; Thistle Tea holds
    her in it 6 s (`wrong-for-1.12`). **A creature's Feign Death** still ends its fight (Thistle Tea's; mangos keeps
    it).
  - **A fleeing creature** stops every cast (mangos); Thistle Tea only those movement breaks.
  - **A passive creature** joins a dungeon-wide pulse (mangos has no passive check).
  - **Left:** 55 rows (damage origin, posture, pet combat, disarm, extra attacks, elemental swings); fleeing with no
    navmesh path runs straight.
- **7e, the world, round 7 (0.1.26):** a random point (a script's random walk, a summon's or an escort's scatter)
  stays within its radius (the navmesh's own pick could land 11 yards out of a 2-yard circle); the real navmesh's
  sight, indoors, zones and paths tested at Thistle Tea's points; Arathi Basin's pickups, a match's countdown home,
  Alterac's captains' rallies, separate copies for teams matched apart. 7e and 6j both made a kill the tap group's;
  I kept 6j's (`map::tap`). **My calls:**
  - **No projection of a player's motion** between packets, and "moving" read from the movement flags (mangos);
    Thistle Tea projects (`wrong-for-1.12`).
  - **Quest-giver status is answered, not pushed** (mangos; the 1.12 client asks again on its own).
  - **An NPC's pet gives no experience** in play (`pets::died`), as 6j found; 7e's ¾ test checks only the formula,
    so the row stays `wrong-for-1.12`.
  - **The bake's sea surface** is 0.19 where Thistle Tea's is 0.268 (the test allows 0.1).
  - **Noticed, not changed:** a script group's members keep pointing at a leader who left (Thistle Tea disbands
    it). **Spells' hit chance** keeps vmangos's 22% floor (`magic_hit_chance`), as Thistle Tea has it; mangos-classic
    has none (a miss of 1% to 100%, `CalculateSpellMissChance`), which matters only far above the caster's level.
- **7f, creature AI, round 7 (0.1.27):** a wander in reach stops as the fight begins; a swimmer chases its victim's
  depth; the victim selector works for a player source after a swap; hostile selectors measure between the two
  reaches, minimum included; a pet finishes its cast before it walks, settles by a master at rest, stops walking and
  casting on Passive, leaves a fight where it stands when its last foe vanishes; death clears a pet's recall and
  orders; a guardian's corpse lies its time, and a dead creature's fighting guardians stay; script command 88 (pet
  commands) fixed; a rooted creature doesn't back away; a retreat's end frees the spell list. **My calls:**
  - **Hostile selectors** use combat distance (Thistle Tea and mangos); the old 5-yard leeway is gone.
  - **Passive** also stops the pet's cast (mangos's `AttackStop(true, true)`).
  - **A pet whose last foe vanishes** leaves the fight at once, not after its 6-second calm.
  - **A creature's death** leaves its fighting guardians and their corpses; its totems and companions go.
  - **A pet settles** within 1.4 × 2 yards plus both radii of a master at rest (Thistle Tea's follow slack).
  - **Left:** 81 rows: formations, scripted follow, the rest of distancing, pets' ally autocast and Devour Magic,
    targeting by threat and PvP flag, guardians dropped by distance, totems' last pulse, scripted waypoint routes.
- **7h, entities, round 7:** forced movements (roots, speeds, water walk, feather fall, hover, knockbacks,
  the teleport ack) carry each client's order counter, counting up from 0 (mangos's `GetOrderCounter`), where wvs
  sent 0 (the acks aren't checked against it, so older clients and peon are unaffected); a possessed creature's
  root goes to the client moving it; a kill that leaves no loot turns no round robin; an object's spell puts no
  object on a threat list. **My calls:**
  - **Game objects cast through a stand-in** (mangos's `GameObject::Use`): a trap's spell by its owner, else the
    one it springs on; a quest object's by its owner, else its user.
  - **`wrong-for-1.12`, mangos's way:** with one roller or none, no roll and the item stays on the corpse; a chest
    with no loot table opens an empty window; a creature's pet's level offset cuts toward zero and never below
    the template's.
  - **Left:** 130 rows (triggered hits, the AI's environment, event sinks, formations, wild summons); Lunar
    Festival fireworks, cheer speakers and the Ghost Magnet don't exist in wvs (content work).
- **7g, creature scripts, round 7 (with 7h):** 50 rows, every one a test of a ported script, step by step: the
  Gizelton Caravan, Freedom for All Creatures (Kindal), the Stormwind Rendezvous and the Great Masquerade, the
  Balance of Light and Shadow (Eris's waves), the Nightmare Manifests (Eranikus, Tyrande, Remulos), Gnomeregan
  (Emi, Grubbis, Thermaplugg), Mizzle, Balnazzar. No fixes needed. **Found, for the next round:** a creature's line
  aimed at another creature keeps a literal `$n`; the test client couldn't parse some melee blows (HitInfo 32770
  and 18) in Eranikus's fight, to go through the client check. **Left:** Alterac Valley's 15 rows (wvs builds them
  in `av_scripts.rs`, not from Thistle Tea's events).
- **7i, pets, round 7:** a pet's use teaches its master by 10 chances in 101 (mangos's `CheckLearning`); a pet's
  passives take the master's modifiers and are put on again as one comes or goes (Endurance Training did nothing);
  totems' time takes the shaman's talents; a pet's bar comes back without what it can't use; a wild summon comes at
  its spell's place and dies as its fight ends; **engineering guardians (Dragonling, sheep, land mine) start their
  item's cooldown at their death** (they had none after the first use); Mind Control refused above its rank, at the
  charmed and at oneself; a held player's bar is Attack alone and their own client plays nothing but chat; a
  creature-charmed caster casts her best spells. **My calls:**
  - **A companion** comes 2 yards front-left facing its owner (Thistle Tea; mangos puts it on top of them).
  - **Eyes of the Beast:** a possessed pet still teaches (mangos). **Revive Pet** from another map works from the
    stored pet (mangos's `LoadPetFromDB`); Thistle Tea refuses.
  - **`wrong-for-1.12`, mangos's way:** a hunter pet's corpse lasts 60 s (Thistle Tea an hour); Rebirth raises
    players only; Beast Lore's flag is seen by everyone.
  - **Found:** a reconnect within milliseconds of a disconnect can load the character before the disconnect's save
    lands and lose recent state (the pet); queued for the next player round.
- **7j, creatures, round 7:** an entry change tells its speeds, keeps its scale auras' growth, lands a flyer and
  swaps a script's spell list; a template's school at a DoT's ticks; a sessile creature stands under fear and
  leaves its fight; a critter held from fleeing stands until let go; **an event's looks now change a living
  creature where it stands** (Thistle Tea's way; they had waited for its fight to end), leave a corpse to its
  respawn, and cast their start and end spells (the holiday costumes); a charmed creature casts its bar. 7i and 7j
  both loosened the "not in control" check; both are kept (a creature under a charm casts, and a player held by a
  creature's charm casts through the charm's AI). **My calls** (`wrong-for-1.12`, mangos's):
  - **A script's entry change** keeps the react state and auras; the new template's auras aren't put on.
  - **Sessile creatures** take fear, roots and confusion but don't move under them.
  - **A resisted hostile spell** still sends a critter running (mangos counts it an attack).
  - **A script's weapon reset** goes to the template's weapons, not an event's.
  - **`NO_SPELL_DEFENSE`** as Thistle Tea (mangos names the flag but never reads it).
  - **Left:** Lunar Festival fireworks (a feature to port); scale auras grow a creature's size but not its reach;
    an entry change doesn't update a creature's maximum mana.
- **7k, auras, round 7:** a transform takes its creature's size, reach and weapons, a growth grows the reach too,
  and the unit's own come back after; a marked unit is tracked (`UNIT_DYNFLAG_TRACK_UNIT`); a buff refreshed at the
  limit counts as the newest, so the next new one pushes out the next-oldest (the refreshed one was pushed out);
  Golemagg's Trust pulses only from his own aura; a lasting aura triggered by a timed one goes with it (Insight of
  the Qiraji with the Badge of the Swarmguard); dispel resistance is its flat chance scaled by its percent (mangos's
  `ApplySpellMod`). **My calls:**
  - **A form keeps the unit's own size** (mangos); Thistle Tea shrinks Ghost Wolf, cat, travel and aquatic to 0.8.
  - **Stacked transforms:** taking off a later one puts the earlier one's weapons back (Thistle Tea); a transform's
    size is its own even at 1.0; several growths multiply (Thistle Tea; mangos adds them).
  - **Left:** ground areas deal their damage from the area, not through auras on those inside (four rows);
    Thistle Tea's 1 s of no detection after Vanish isn't in wvs or mangos.
- **7l, movement and the network, round 7:** **forced movements now wait for their acknowledgement** (roots,
  speeds, water walk, Slow Fall, hover, knockbacks, teleports): each ack settles the order it names by counter,
  unit and change, and any other changes nothing (Thistle Tea's `MovementControl`, mangos's `GetOrderCounter`); a
  teleport drops the orders waiting before it. Peon already echoes every counter. Fixed on the way: a root ack sent
  before a teleport put the player back where they had been. Also: a pet left 50 yards behind by a teleport goes and
  comes back at the ack (mangos's `TeleportTo`); a player dying on a charge lies where they fell; what prevents
  fleeing (Recklessness) holds a fear back until it goes; a unit a player controls is taken from their client while
  feared or confused; a knockback goes to the client that moves the unit. **Merging 7j and 7l** left two arms for
  "prevents fleeing" where the first shadowed the second (a critter's held run, a held fear taking hold); I merged
  them into one. **My calls** (`wrong-for-1.12`, mangos's): a spirit's release teleports at once; roots and stuns
  caught on a charge land at once; a refreshed or second fear doesn't restart the run; a fear drops the creature's
  target. **To ask you again:** Distract turning players (mangos's `EffectDistract` turns them; DECISIONS.md said
  not to).
- **7m, the core's other areas, round 7:** a player's death frees what they charm; a pet told to follow or made
  passive lets go of its target and cast; pick pocket can be resisted, and a caught one breaks stealth and starts
  the fight; a quest chest shows as usable to one whose quest wants its loot; a Spirit of Redemption leaving a
  battleground goes home a living priest; a profession's lower rank read after a higher one keeps the higher step
  and cap (mangos's `SetSkillStep`); an auction search by the chest slot finds robes; an accepted resurrection
  brings the dead where the caster stood as they cast it (mangos's `setResurrectRequestData`). **My calls:**
  - **Quest chests** show as usable only to one whose quest wants their loot (mangos); Thistle Tea flags them all.
  - **A forgotten weapon skill** starts again at 1 when relearned (mangos; `wrong-for-1.12` for Thistle Tea's
    keeping the old value).
  - **To ask you:** an emote ends Feign Death (Thistle Tea's, kept); mangos refuses the emote instead
    (`UNIT_FLAG_PREVENT_ANIM`).
  - **Left:** a create strips jump and spline-elevation data where both references send it; riding skill caps
    don't follow `SkillTiers`; the Scourge camps and fireworks; Stratholme's Baron and Aurius.
- **8b, spells, round 8:** **what a cast breaks is mangos's now:** wvs took off the caster's auras with flag 0x1
  (mangos's HOSTILE_ACTION, which 1.12's Stealth has) at the cast's start, so **applying a poison or disarming a
  trap broke Stealth**; now ACTION (0x4) goes at the start and ACTION_LATE at the launch, the spell's own aura and
  spells that allow Stealth or Invisibility keep them, and NOT_AN_ACTION breaks nothing. Pick Pocket's failure
  breaks stealth; a health funnel takes spell power and its modifiers (mangos); a DoT's coefficient shares by
  mangos's tick count (`GetSpellAuraMaxTicks`); Deep Wounds adds flat damage done; a refreshed Discombobulate
  dismounts again. **My calls:** a cast now ends Feign Death too (its flags include ACTION, mangos); a harmful spell
  at a dead target still rolls to hit (mangos). **Found:** a cast at a corpse with a destination is misread
  (`CMSG_CAST_SPELL`'s targets), queued for the fork.
- **8c, loaders, round 8:** aura amounts with percent modifiers in whole numbers (Bonescythe made Slice and Dice's
  30 into 32; mangos's `ApplySpellMod<int32>` gives 31); a spell's two ground areas join one aura holder, so Flare
  removes invisibility and keeps stealth off; target 22 (the caster's own spot) is neutral, so 244 spells (Healing
  Auras, totem pulses) are helpful as in mangos; Shadow Trance and Netherwind Focus are spent on use (every Shadow
  Bolt had stayed instant); Pick Pocket rolls to hit (7m, 8b and 8c all fixed it; 7m's and 8b's kept). **My calls:**
  modifiers in whole numbers (mangos; Thistle Tea keeps 31.8); Arcane Intellect procs the Blue Dragon (mangos);
  cat form ends Water Walking and Moonkin keeps it (Thistle Tea; mangos ends it on every form). **Found on master**
  (round 7 and 8's merges, untested together): two of 7j's size tests against 7k's transforms, and Symbol of Life
  no longer reviving Henze Faulk; `fix/master1019` is on them, with the two flaky tests.
- **8a, the player, round 8:** **a login waits for the account's character to finish leaving** (up to 5 s, 1 s
  for a ghost) before reading it, so a fast reconnect no longer loads it from before its last save (7i's find);
  **the bank moves items only at the banker it was opened at** (my call of 7a, Thistle Tea's `Bank.authorize`; the
  NPC check's 5 yards past both bodies); worn or mended gear works the stats out again; worn gear sells (mangos), a
  sale ends a cast; a spell's taxi from a node with no mount flies unmounted; a stable swap for a pet that can't come
  out moves nothing; a kill's reputation takes its faction's aura bonus (190, mangos); GM commands' limits.
  **My calls:** any cast ends at a sale (mangos; Thistle Tea only the sold item's); reputation losses take no aura
  bonus (wvs's, kept; mangos shrinks them); `wrong-for-1.12`: taking a weapon off doesn't reset the swing timer, a
  saved flight resumes at login, abandoning a quest doesn't give its starter item back.
- **8e, the world, round 8:** a spell at its database place (target 17) lands on that place's map: its summons,
  guardian, object and ground area; a teleport at a unit's place (63, a creature's Blink) goes there and moves a
  creature on its map (mangos's `SetTargetMap`, `EffectTeleportUnits`); a druid in an animal form is a beast to a
  spell's creature types as to an aura's; far sight's camera comes home and goes back (mangos's
  `HandleFarSightOpcode`); a script's leave takes a member out of its database group, and its leader's disbands it
  (as Thistle Tea, 7e's find; it stays left for the map's life, vmangos). **My calls** (`wrong-for-1.12`, mangos's):
  leash clocks per creature (Thistle Tea shares one between a caller and its helpers); no combat hold for a fighting
  pet under Feign Death; group reward measured from the member, not the corpse; no Void Zone lift; stealth checked
  again on moving, not on turning; a leech's heal makes no threat.
- **8f, creature AI, round 8:** a retreat (movement 19) needs its point on the ground, seen and reached, and breaks
  only a cast that moving breaks; a scripted flight runs for its time; a pet's helpful autocast goes on its master,
  then their party (a raid's subgroup) near, alive and in sight, then itself (mangos's `PetAI::UpdateAllies`; it
  had cast only on itself); a fighting leader no longer drags its formation; a scripted follower leaves a fight
  where it stands; a list's cast that takes time halts the walk and faces the target. **My calls:** a door script
  with `datalong2` 0 stays open (the Emberseer, Chromaggus and Mur'Alzzin doors use it; mangos would close it after
  15 s); a retreat reaches its point within 0.5 yards (navmesh ends; Thistle Tea 0.1); a script's follower evades in
  place (Thistle Tea; mangos walks it back).
- **8d, combat, round 8** (its session ended before its report; from its commits): **a spell's procs go off once a
  target, after its effects** (mangos; they had run once per damaging effect, 6d's leftover); a creature's cast not
  for a fight stops as an attack puts it in one (mangos's `SetInCombatState`); pacify holds a unit's blows without
  ending its fight; an elemental's blow is resisted in part by its school's resistance less penetration; a soul
  shard only from a creature that was mostly the players' kill (Thistle Tea's damage share); the attack table's
  chances rounded to the hundredth (mangos's `chance_u`), not cut; a player logging in under a fifth of her health
  keeps it (mangos); a weapon cast disarmed or without its weapon as it ends fails; a player's owed attacks land
  within 1.4 yards whatever her facing (Thistle Tea's overlap); a unit immune to players takes no help from them
  (mangos's `CanAssist`).
- **Rounds 7 and 8 merged together, then fixed (`fix/master1019`):** the machine ran short of memory at 19:42 and
  my test runs were stopped, so rounds 7 and 8 were merged without the fast suite between them. What broke: 7k
  saves a unit's own size while a growth or transform is on and 7j changes it on an entry change or an event's
  model, so the size came back as the old entry's; now an entry change or an event's model resets the unit's own
  size and fits the growths and transform again (mangos's `GetNativeScale` is the current entry's). Henze Faulk's
  test cast Symbol of Life from beyond the spell's range (6e's range rule, mangos's). The trap and bound-gear
  flakes were test races (a fight left running killed the hero; the map's once-a-second item check), now fixed.
- **8g, creature scripts, round 8:** **a fight's temporary war ends once nothing holds the player, and is never
  saved** (it leaked: a character could stay at war with Booty Bay with no fight, 6f's find); a creature's line at
  another creature names it for `$n` (Kindal's "Quickly! $n attacks!"); Alterac Valley's 15 rows: the war riders'
  spells wait for a victim within 30 yards, Balinda's casting spot needs sight, a ritual needs its living invoker;
  the cavalry, infantry, lords and wing commanders tested. **My calls:** the temporary war is Thistle Tea's (mangos
  has none), without the leak; `$r` and `$c` stay as written for a creature (mangos gives no rule).
- **8i, pets, round 8:** a master who can't join (unattackable, unselectable, flying) or owns a summon with
  vmangos's `NO_OWNER_THREAT` (Eye of Kilrogg, Sentry Totem) isn't drawn into its fight; a blow at an evading
  creature stops; **a creature whose victim (a pet) died turns to the next on its threat list** (it had stood idle,
  keeping the hunter in combat); a creature-charmed caster keeps her foe; **the engineers' Target Dummies** stand,
  answer no blow and fall at 15 seconds (vmangos's `npc_target_dummy`; they had followed and fought for good); a
  charm of many (aura 177, Chains of Kel'Thuzad) charms. **My calls** (mangos's): a creature taking on a pet brings
  its master in at once (Thistle Tea waits for a blow); the master is held in by his own threat entry; at the pet's
  death the foe turns on the master.
- **Your two answers (fix/userdec):** an emote (and a posture change) is refused while feigning, Feign Death
  staying (mangos's `UNIT_FLAG_PREVENT_ANIM`); Distract turns a player to its point with a facing move to them and
  their watchers, no control change and no pause, except in a fight, stunned, feigning, logging out, moved by the
  server, flying or aboard a transport (mangos's `!IsInCombat` and `UNIT_STAT_CAN_NOT_REACT`; aboard would need a
  transport-relative move). Client check passed. **Left:** mangos sets the same flag from 97 other spells' attribute
  (stuns, Polymorph, Sap, Sleep), refusing emotes under them too; wvs only for Feign Death, queued.
- **M2 and M3 fixed (`fix/m2m3`):** `key_for` holds the copies' lock from choosing a copy to storing it, and
  those let into a copy count against its limit until they arrive (two of a group entering at once now always go
  into one copy, and a copy's last place goes to one; mangos has the same limit gap); a refused entry spends no
  copy id. Shutdown: a map asked for after the close is a stand-in that saves arrivals; a map that has shut down
  keeps saving late arrivals; the shutdown waits (within its limit) until no one is online before the store's last
  flush; a session ending during the shutdown still hands on a player it was moving. **Found:** a copy that unloads
  normally can lose a player sent to it just before it stopped; queued.
- **8j, creatures, round 8:** **the Lunar Festival's firework guys** (vmangos's `npc_pats_firework_guy`, new
  `fireworks.rs`): a rocket or cluster used at a launcher calls its guy up there; spell target 46 now puts wild
  summons and creature guardians at the nearest such spot (Raise Dead, Summon Atal'ai Skeleton too); a core hound with
  no kin fighting dies and is gone at once (vmangos's `Kill_Self`); a creature group's leader row is no longer counted
  as a member (60 real groups' flags change, vmangos's `AddMember`); a flyer's corpse falls under gravity on the
  server as the client sees it; a transform's weapons stay over an event's looks. **My calls:** rockets go off at
  once (vmangos's sniffs; Thistle Tea keeps them 5 s); the small coloured clusters send their colour's rockets
  (Thistle Tea; vmangos's guy sends nothing for them); a transform's weapons win over an event's own equipment
  (Thistle Tea; mangos puts the event's on).
- **8h, entities, round 8:** a harmful spell's hostile act flags both sides and contests the caster, damage or
  not (mangos's `CasterHitTargetWithSpell`, `SetInCombatWithVictim`); **the Ghost Magnet** calls its eight Magrami
  Spectres for two minutes (vmangos); **the Lunar Festival's cheer speakers** run their cities' fireworks shows
  (vmangos's `go_cheer_speaker`); target dummies take their summoner's side (in vmangos's data they are wild
  summons) and a dragonling arrives at its full health; a rogue's open pockets shut as the creature dies (mangos's
  `Loot::~Loot`). **Merged with 8j:** both had ported the firework guys and target dummies; 8j's fireworks were kept
  (rockets go off at once, vmangos's sniffs; Omen's launcher), and both dummy paths (as a guardian, 8i; as a wild
  summon, 8h). **My calls:** the Ghost Magnet, fireworks and speakers follow vmangos (mangos-classic's magnet is one
  spectre every 15 s); an owned trap springs on a flagged enemy though its owner isn't flagged (mangos); items held in
  an open pickpocket window are lost at the creature's death (mangos).
- **8k, auras, round 8:** every spell with `SPELL_ATTR_EX_PREVENTS_ANIM` (stuns, Polymorph, Sap, Sleep, Dazed, Feign
  Death: 103) blocks emotes and postures (mangos; a dazed player can't `/sit`); a periodic trigger's spell pays its
  own reagents; **a player's flat physical damage done goes in the hands' damage fields under their percentages**
  (mangos's `HandleModDamageDone`: normalized abilities such as Mortal Strike and Backstab had lost the hands'
  percentages and Hunter's Mark); for a second after Vanish no one detects the rogue (mangos sets the timer;
  Thistle Tea reads it); a ground channel pushed back cuts its areas short. **My calls:** the off hand gets half the
  flat bonus (mangos; Thistle Tea all of it); a ground area's caster is read as it hits (mangos), not at the cast.
- **8l, movement and the network, round 8:** **a player sent to a copy as it stops isn't lost** (it closes its queue
  and reads it to its end; a late arrival goes to the copy started again, or is saved: m2m3's find); the client
  check now reads melee blows (`0x6255B0`): wvs's layout of HitInfo 32770 and 18 was right, they are flags (a
  crushing blow 0x8002, a main-hand miss 0x12), and the fork reads the field as one value; **an auction error's
  reason is a u32** (the fork writes a byte, so the client showed its own default error; now by hand, as
  mangos-zero); **`CMSG_BATTLEFIELD_JOIN` as 1.12.1 sends it** (map, instance, group byte; the fork took only the
  map, so a join without a battlemaster was refused; now read by hand); a pet's ground spell keeps its place.
  **For the fork (your call):** three of its 1.12 layouts are wrong: `HitInfo` as an enum, not flags;
  `SMSG_AUCTION_COMMAND_RESULT`'s inventory error a u8, not a u32; `CMSG_BATTLEFIELD_JOIN` without its instance and
  group byte. wvs works around the last two by hand.

## Your list of 2026-10-20
1. **Done (`feat/cities`): Jaina for Stormwind, Ironforge and Darnassus.** The capitals are data (`cities.rs`), one
   champion flow for both sides (`champions.rs`). Her Teleport in Ironforge goes to the Mystic Ward's upper floor
   (-4598, -925, 524.7), in Darnassus to the Temple of the Moon's lower hall (9625, 2525, 1331.6). An Elite Guard
   for each: the Ironforge Elite Guard (an Ironforge Guard's look, double axe and Stormhammer), the Darnassus Elite
   Sentinel (a Sentinel's, a glaive in each hand). Ironforge read as Dun Morogh on the navmesh (the city is inside
   the mountain), so its civilians called no guards; a box round its interior fixes that for guards and champions
   (players' zone there is still Dun Morogh, queued).
2. **Done (`feat/cities`): the Horde's city protection.** Orgrimmar, the Undercity and Thunder Bluff already had
   mangos's guards per victim; now the alarm, an Elite Guard each (Orgrimmar Elite Grunt, Undercity Elite
   Deathguard, Thunder Bluff Elite Bluffwatcher) and **Rexxar**, one in the world: he says a line in Desolace, runs
   40 yards with Misha and is gone; he appears about 110 yards from the fight out of view, on the navmesh, and runs
   in with Misha and four Elite Guards; he fights with his template's spells (Thrash, Knock Away, Sundering Cleave,
   Puncture, Frenzy below 30%); leaving, "I must go.", a 40-yard run at raised speed, and gone. **My reading:** your
   "when summoned he says I must go" as his leaving (Jaina's flash home's counterpart). **My calls:** a city called
   while its champion is out goes unanswered, and its next call after they're home brings them; a champion's
   elites are the city's; Rexxar's respawn a day, a setting of its own. **Settings:** `jaina_city` is now
   `jaina_cities = [1519, 1537, 1657]` (a config with the old key is refused: yours didn't set it), and
   `rexxar_cities`, `rexxar_from`, `rexxar_stays_secs`, `rexxar_elites`, `rexxar_arrive_yards`, `rexxar_run_yards`.
   **For peon:** its raid knows only Stormwind's guards and Jaina; queued.
3. and 4. Peon's healers target whom they heal and cancel half the time when the target dies; shamans' Chain Heal
   (peon `feat/heals`, running).
5. **Done (`fix/bugs1020`): patrols and held items.** Officer Jaxon (14423) and Thomas Miller (3518) are movement
   type 2 with no path of their own: their paths are by entry (`creature_movement_template`, 112 and 46 points),
   which wvs never fell back to (mangos's `GetDefaultPath`); and their `creature_addon` has `equipment_id = -1`,
   which in vmangos means "the template's weapons", where wvs read it as none (and 0, none, as the template's).
   Both fixed: **185 spawns now walk their entry's path** (18 formation leaders, such as Princess), and **2,517
   spawns now hold their weapons** (Nethergarde Miners, Venture Co. Workers, Defias Miners); creatures with an
   off hand now swing both (mangos). A creature home from a fight walks its entry's path again.
6. **Done (`fix/bugs1020`): corpses sliding.** Your theory was right for players: a player dying on the run kept
   their forward flag and watchers got no stop, so their clients carried the body on. Now death clears the run
   flags and sends watchers a stop at the body (mangos's `SetDeathState`, `StopMoving`); a dead client's late
   movement is passed on without them. Creatures already got a stop, but at the last update's place, so the corpse
   jumped back a little; now at where the walk had reached. Client check passed. **Left:** 38 places put a creature
   at rest without a stop (each could let a client run it to the end of its walk); not audited yet.
- **Chain Heal jumps 12.5 yards** between friends (your call): wvs's heal chains reach 12.5, harmful chains 10
   (mangos's); peon's shamans aim by the same 12.5.
7. **Done (`feat/arenagrace`):** walking out of an arena's area keeps free-for-all for 20 seconds (`[arena]
   ffa_grace_secs`, live); back in within it carries on; death or a far teleport ends it; a Blink out keeps it.
   **My calls:** everyone gets it (not only those recently in PvP, AzerothCore's rule); only walking out starts it
   (a battle's end still lets fighters go at once); it covers every arena-flagged area (the Rumble Cage, the Maul).
   mangos has no such timer (its flag goes at once); AzerothCore's is about 60 s for the recently flagged.
- **Settings for what was hard-coded (`feat/cfgreview`, your ask):** about 40 new keys at today's values, most with
  mangos's name: chat's say, yell and emote ranges and flood control (`[chat]`), health, mana, rage, energy, fall
  damage and auction rates (`[rates]`), the low-level reputation floors, the group reward range (one, 74 yards, for
  XP, loot, quest credit, reputation and honor), death sickness's level, primary professions (2), breath, fatigue
  and lava (`[environment]`), instances' unload delay, reset hour and entries an hour (`[instances]`), Deserter,
  outdoor PvP, the elemental and Scourge invasions, the weather's interval, the realm's name, the movement check's
  allowance, the arena's timers. All live but the realm name and outdoor PvP. **Its calls:** the `[arena]` keys are
  live now (the arena reads them as it uses them); **the realm list shows PvP when `[players] pvp_realm` is on** (it
  said normal on every realm: the one behaviour change); a flood count of 0 turns flood control off, as mangos.
  **Left for you** (in its report): mangos's `PlayerLimit`, `InstantLogout`, `Rate.Creature.*`, ghost speed, mail
  expiry and the 100-mail box, the invasions' script numbers, ports and bind address, and more.
- **Chat's fan-out (`perf/chat`, your ask):** a lock per channel behind a directory never held across one; a line is
  fanned out after its channel and the ignore lists are let go (the ignore lists gained a reverse index); guild and
  group lines likewise, and guild chat no longer looks each member up again (it was N²); `/who` stops at a full
  list. **Measured** (`benches/chat.rs`, millions of deliveries a second): one map about the same (2), 8 maps in one
  channel 1.9 → 7.2, 8 maps in 8 channels 2 → 13 to 20. **What limits it now** is tokio waking each recipient's
  writer (85–90% of a map's fan-out), and in one channel the shared recipients' queues. **Its calls:** lines from
  two maps in one channel may reach members in different orders (one map's stay in order); channel notices stay
  under the channel's lock, in order. Numbers in `docs/PERFORMANCE.md`. **Also fixed:** the reputation test's race
  (the map's update ending a fight's war between two queries).
- **Review 5's fixes, social (`fix/fixsocial`):** #3: a mail's subject is cut to the client's 64 letters and its
  body to 8,000 bytes, and the mail list stops at 32,766 bytes (mangos's `HandleGetMailList`), so no frame
  outgrows its size field; #14: a post holds the mail's lock until its save is sent, so a refund taken as it is
  written comes back no more; #15: mail is indexed by receiver (the next-mail-time query over 100,000 mails, 1,000
  times, 1.5 s → 1 ms); a meeting stone's queue is left at a logout. **Its calls:** mails cut, not refused (a
  client's box takes 500 letters, an addon may send more).
- **Review 5's fixes, maps (`fix/fixmaps`):** #4 (high): a copy counts as in use while anyone let in is on their
  way, so a reset or a raid's expiry no longer drops an arrival in its queue; as a backstop, a copy stopped with
  players inside saves each and sends them to the character screen; #7: a map's shutdown saves each player under a
  guard of their own (one panic no longer loses the rest); #17: copies loaded at start-up get their five minutes;
  #18: a far teleport keeps a logout's countdown going (mangos), the client rooted again on the new map; maps 6:
  every way out of the world leaves channels, queues, friends, guild and group. **Its calls:** players in a copy at
  its stop go to the character screen, not into the restarted copy; a reset refused for someone on their way says
  "zoning" (mangos). **Found:** a session can keep sending to a stopped copy's old handle (the player then goes
  offline unsaved); queued for after `fixnet`.
- **Review 5's fixes, network (`fix/fixnet`):** #2 (high): a close now ends a writer stuck on a client that doesn't
  read, and a write that moves nothing for `client_idle_timeout_secs` (120) ends it; #12: **one session an account**
  (mangos's `AddSession_`): a new one replaces the old, which ends first (a player it left in the world is taken
  over as before, #35); `.ban` reaches the account's session wherever it is, and a banned login is refused; #13:
  account names in capitals at every handshake (old rows in another case merged at start-up); pings counted as
  mangos's `HandlePing` does, more than `[network] max_overspeed_pings` (2) under 27 s apart disconnecting (peon
  and the real client ping every 30 s). **Its calls:** an old connection is cut as soon as the new one is through
  the handshake (mangos); GMs aren't exempt from the ping rule (the reader doesn't know the GM level).
- **Review 5's fixes, world and combat (`fix/fixplay`):** #1 (high): an express route passes a node once (the
  client's is always a shortest), 128 nodes at most, and a frame too big for its size field is refused, never cut
  short, everywhere frames are built; #16: a possessed unit's movement from the controller's client is checked
  valid (no NaN relayed or saved; no speed check yet); #5 (high): a pet, totem, guardian or charmed creature keeps
  no threat list, so whoever strikes one leaves the fight once its timer runs out, and a passive creature lets go
  of whoever struck it and went (30 yards, 12 s, the leash's settings); #8: a death's guard takes its victim off the
  dying list as a panic unwinds, and an extra swing's flag is reset; #9: spells nest 32 deep at most on a thread
  (a self-triggering spell no longer aborts the server). **Its calls:** a frame too big refused, not split;
  guardians and creature-owned pets keep no threat list either (mangos keeps one, wvs never read it).
- **Review 5's fixes, data and the store (`fix/fixdata`):** #6 (high): **a start with world data missing or broken
  refuses** ("run `git lfs pull`") rather than running an empty world that wiped the skills of whoever logged in (an
  empty world is still allowed with no data and no characters); #10: saves the store can't write (a full disk, I/O,
  read-only) are kept, the newest of each, and written once it can (100,000 at most; logins refused meanwhile, GMs
  told); #11: the next character and item GUIDs are kept in the store (`counters`, migration 5), so a deleted
  character's GUID, honor, tickets and binds go to no one; #19: records read field by field, one that doesn't read
  left in the store with its id kept; #20: the start-up navmesh question waits 30 s, then starts (a "no" or no answer
  remembered in `data/setup-declined.txt`); #21: a second server on the same store stops at its lock
  (`characters.sqlite.lock`) and its port before touching it. **Migration 5 backs the store up first**
  (`characters.sqlite.v4.bak`). **Its calls:** the 30 s a constant; saves still kept at a shutdown are lost, logged.
  Merging with fixnet: a test reconnected so fast it took its player over (one session an account) where it looked
  for a logout; it waits for the logout now.
- **Your flight fix (`fix/flight-splines`, 12932's):** merged as it was; the fast suite, the taxi tests and the
  client check pass with it, and nothing needed fixing.
- **A session's stale map handle (`fix/stalehandle`, found by fixmaps):** a copy that stops and hands its arrival
  to its restarted copy now has that copy tell the player's session its new handle, so their packets reach it and
  they're saved on logout (they had been dropped unsaved at their next packet); a player whose client goes while
  handed on is saved there.
- **Leftovers, the capitals (`fix/cities2`):** a player inside Ironforge is in Ironforge, zone and area (the box
  round its inside, asked first by every lookup; the gate's steps stay Dun Morogh); a script group whose leader
  leaves the map is disbanded (Thistle Tea keeps it leaderless, mangos promotes a new leader). **peon:** the raid
  knows every capital's guards, Elite Guards and champions; `--raid-at stormwind|ironforge|darnassus|orgrimmar|
  undercity|thunderbluff` (or `map,x,y,z`); `--raid-side alliance` sends Alliance raiders at a Horde city (naming
  a Horde city picks it). Merged with your peon flight path storm (`--mode flight`).
- **Gryphons frozen in the air (`fix/flightbug`, your bug):** a create on a spline carried `SPLINE_ENABLED` and no
  direction; the 1.12 client moves a unit each frame only if its create has it going somewhere (`0x618940`), so a
  newcomer's client froze the flier where it was told she was (before your fix, with no spline, it dropped her to
  the ground). Now a create on a spline goes FORWARD, walking unless it runs (mangos's `MoveSplineInit::Launch`),
  for every unit; and a walk's spline is laid out as the client reads it (a point before the first: it had dropped
  a walk's first segment). Ruled out with tests: a missed next leg (a flight is one walk in wvs), relayed movement
  after the create, the frame's size. The client check now runs the client's own `0x618940` on each create.
  **Left:** a player under fear, confusion or a charge is created for a newcomer without their spline.
- **The fork's four 1.12 layouts (`fix/fork`, pushed to your fork at `361d2ed`):** `HitInfo` is flags (a crushing
  blow 0x8002, a main-hand miss 0x12 now read; the debug trailer under 0x1 too); `SMSG_AUCTION_COMMAND_RESULT`'s
  inventory error a u32 (the client's `0x4CC460`); `CMSG_BATTLEFIELD_JOIN` the map, instance and group byte
  (`0x4AA00B`); a cast's targets in the client's order (one GUID for unit, corpse or object first, `0x7E4E10`;
  mangos's `SpellCastTargets::read` has the corpse last). TBC's layouts untouched. wvs's hand encoder and reader
  for the two are gone, its melee blow through the fork's type. **Pins:** rustigator `732cec2`, wvs and peon at the
  fork's `361d2ed` and rustigator's `732cec2`. **Its call:** a `CMSG_BATTLEFIELD_JOIN` of the map alone is refused
  (Thistle Tea takes it; the client never sends it).
- **Review 5's lows, the world, combat and realm-wide state (`fix/lowsb`):** faster: a creature's turn no longer
  re-checks its threat list after pruning it (100 creatures holding 40 players: 130 → 32 ms an update), the casters
  kept in a set (20,001 creatures, one casting: 691 → 1.9 µs), a player's combat check stops at the first creature
  holding them (702 → 20 µs), a chain's sight rays only until one passes (1.3 → 0.4 ms); fixed: an aura state, form
  or creature type past 32 matches nothing (mangos's mask), what a service changes on a player between maps is
  forwarded, two event sweeps can't leave a map the older set, a champion whose farewell was lost comes home,
  the member-stats push holds the groups lock briefly. **Left:** world 3 and 4, social 7, 9 and 12 (queued).
- **Leftovers, entities (`fix/entity`):** each viewer gets their own view of a unit (`map/views.rs`, one block a
  distinct view, as the tap's): a Hunter's Mark's tracking flag for its hunter alone; creatures' fields by mangos's
  masks: strangers the public ones, the master the owner's too, an Empathy caster damage and resistances; a corpse
  sparkles only for those who may open it. A creature put at rest on a walk now stops where the walk has come to,
  its watchers told (`ai::rest`): 8 places sent no stop (formations, escorts, Eyes of the Beast, script commands 60
  and 67, the script wander), 3 sent it from the last update's place. (The PvP flag items were already done on
  master.) Client check passed. **Its calls:** marks per viewer on players too (mangos's intent); strangers lose the
  whole mangos owner mask (stats, attack power, base health and mana, costs, pet experience).
- **Review 5's lows, the network, maps and data (`fix/lowsa`):** `/who`, whois and inspect at most once in 5 s
  each (mangos's `InitOpcodeCooldowns`), and a cache upload unchanged isn't written; a client's packet carries its
  session; a login into a copy starts no world copy of its map; logging in aboard a ship that crossed to the other
  continent puts you on its deck there (mangos's `LoadFromDB`); what a map makes itself gets a counter that
  doesn't collide; a navmesh header listing missing tiles warns; a reload finding no config file keeps the running
  settings; `[arena]`'s times bounded. **Left:** net 7, maps 9, data 7 (queued).
- **The flaky tests (`fix/flaky`):** nine, each from 5 to 16 failures in 30 or 40 runs to none: fights that ended
  at the next update, charms with no aura that went free, timers read late, an unseeded roll (Jaina's elementals,
  Balnazzar's Domination window), a route's last point gone by the next update (`came_to`); and tests that
  reconnected so fast they took their player over (`harness::offline`).

## Round 9: the last gaps
Phase 1, the rows where wvs was said to do it already: six agents by area; tests, and small fixes where a test
showed a difference. Phase 2 takes what is left.
- **9creatures:** #57 done (a party aura shows on its caster too, as mangos's `IsNeedVisibleSlot`); a direct power
  burn takes the target's damage-taken modifiers (mangos's `CalculateSpellDamage`); an aggressive pet takes on
  no one out of sight (`PetAI::MoveInLineOfSight`); a totem's pulse due at or after its end goes with it; a dead
  unit can't be put back in a fight. Four rows where mangos does as wvs and Thistle Tea is stricter are
  `wrong-for-1.12` (an unflagged player's pet at a flagged enemy, autocast under a stun, a pet kept at any
  distance, a partial path's point). **For phase 2:** EventAI for owned creatures (`event_ai::fire` skips every
  creature with an owner), how long a pet stays in its fight, guardians' distance and map rules, Devour Magic.
- **9entity:** a summon's going is told to its summoner while it is still there (its despawn event's condition on
  it can be met); a new pet comes in at its passives' full health (an imp had 60 of 160); a demon keeps its class
  armor over a level row with none (mangos-zero's). **Its calls:** a creature notices a pet at its master's level
  (Thistle Tea; mangos the unit's own); `SMSG_SPELL_GO` lists every target as hit, misses apart (1.12's layout).
  **For phase 2:** summon spots unchecked against terrain and sight; triggered casts' cast-end procs (Arcane
  Missiles spends one Unstable Power stack a channel); `inverseEffectMask` unloaded; script command 27's level; **a
  player's wild summon isn't tapped by its creator and can give XP.**
- **9player:** a lock spell's object not in this world is no target, and a cast ending runs the profession checks
  again (mangos's `CheckCast(false)`); a coating's last charge spent stops its other spells; a spawn's own greeting
  (`npc_gossip`) where it has no menu (mangos's `SendPreparedGossip`); at login a saved flight is taken up before
  the pet is asked for; a held player's mover stays through a not-active-mover; a battleground portal's refusal is
  an area trigger's message; a talent's rank learned takes the rank below's aura with it. Four rows where wvs
  follows mangos are `wrong-for-1.12` (a dead player taking from an open item-loot window, a node's window left
  open on walking away, a random property rolled for a new item, only the caster's form checked).

## Answered
- **#57, #58, #51, #56** (2026-10-20): mangos's for each: a passive party aura shown on its caster too (to do); a buff that shapeshifting cancels allowed on a shapeshifted target; a creature-owned pet's death gives nothing; unlearning a profession keeps its quests.
- **M2 and M3** (2026-10-20): to fix now (`fix/m2m3`); **M4** deferred, low severity.
- **Feign Death and emotes** (2026-10-20): mangos: an emote is refused while feigning (`UNIT_FLAG_PREVENT_ANIM`).
- **Distract** (2026-10-20): it turns players too, as mangos's `EffectDistract`.
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
| 6g: the world's systems | 71.0% | 2febcb2 |
| 6d: combat | 72.0% | fd2fa71 |
| 6e: spells in the world | 72.9% | fce32da |
| The 15:38 extended run's four failures | fixed | 37a0656 |
| 6f: creature scripts | 73.8% | dd31c4f |
| 6h: creature AI | 74.5% | 4738983 |
| 6i: entities in the world | 75.1% | cabc8d6 |
| 7a: the player, round 7 | 75.7% | 5b07acc |
| 6j: the core's other areas | 76.2% | dfcd6e7 |
| 7b: spells, round 7 | 76.9% | a69e06e |
| 7c: the loaders, round 7 | merged | 2df117c |
| 7d: combat, round 7 | 78.5% | 5890d1f |
| 7e: the world, round 7 | 79.2% | 0c99ac6 |
| 7f: creature AI, round 7 | 80.0% | 427b0c3 |
| 7h: entities, round 7 | merged | c7f8585 |
| 7g: creature scripts, round 7 | merged | 1a96144 |
| 7i: pets, round 7 | 81.5% | 296f744 |
| 7j: creatures, round 7 | 82.1% | 63a5e39 |
| 7k: auras, round 7 | 82.6% | 462b7fb |
| 7l: movement and the network | 83.3% | d68535e |
| 7m: the core\'s other areas, round 7 | merged | aace0cc |
| Round 8 (8a to 8f) and fix/master1019 | 86.7% | 0.1.28 |
| 8g, 8i and your two answers (fix/userdec) | 87.1% | 0.1.29 |
| M2 and M3 (fix/m2m3) | merged | 0.1.30 |
| 8h, 8j, 8k and 8l: round 8 done | 87.8% | 0.1.31 |

**Running now:** nothing: paused before round 9 (your word, 2026-10-20). Round 8 is in.

Their results go into [DECISIONS.md](DECISIONS.md) and this table as they merge.
