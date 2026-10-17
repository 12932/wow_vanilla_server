# Aligning wow_vanilla_server with Thistle Tea

What wow_vanilla_server needs to do what Thistle Tea does, and the order to build it in. It is measured with
[peon](https://github.com/12932/peon) on both servers, as in peon's
[BASELINES.md](https://github.com/12932/peon/blob/master/docs/BASELINES.md).

## Where the two stand (2026-10-16)

| | wow_vanilla_server | Thistle Tea |
|---|---|---|
| Code | 13k lines of Rust | 180k lines of Elixir (1,879 files), 8,800 tests |
| Client opcodes handled | 46 | 269, plus 22 movement opcodes |
| World | creatures from a cmangos database, only when one is configured; no game objects | vmangos: 66,001 creatures, 56,611 game objects, every loot table, 4,433 quests |
| Model | one world tick (10 Hz) that every in-world packet waits for; maps tick one after another under two global locks | a process per player and per creature, no global tick; replies at once |
| Units | 100 HP for everyone; no stats, levels, XP or power | vmangos stats, XP, regen, talents, skills |
| Combat | a swing loop with fake damage | vmangos's hit table, armor, threat, dual wield, ranged |
| Spells | Frost Nova, no cooldown | the casting pipeline, 98 effects, 172 aura types, cooldowns, power |
| Content | none | loot, items, vendors, trainers, gossip, quests, taxis, banks, mail, auction house, trade, guilds, groups, raids, duels, battlegrounds, instances, transports, pets, professions, honor, weather, events |
| Persistence | `snapshot.bin` every 60 s: no fsync, no versioning, no save on SIGTERM | none: a restart loses every character |
| Security | every session is a GM; positions trusted | every session is a GM; positions trusted |
| Under load (peon, 100 bots) | under a core; replies on the tick: movement 55 ms, name query 100 ms, login 356 ms | replies in 1 ms with long tails; 13.5 cores for 200 bots in a crowd |

Thistle Tea's own lessons, read from its code ([THISTLE_TEA_INVENTORY.md](THISTLE_TEA_INVENTORY.md)): its CPU
goes on a 875-yard visibility square, full update blocks re-sent on every change and encoded once per recipient,
deflate set up per packet, proximity announcements fanned out to every unit every 2 yards, and a large metadata
projection rebuilt after every packet. Doing what it does without those costs is the point of the
design below.

## Where they stand after the plan (2026-10-16)

| | wow_vanilla_server | Thistle Tea |
|---|---|---|
| Code | 32k lines of Rust, 132 tests through the protocol (and the real data's checks) | 180k lines of Elixir, 8,800 tests |
| Client opcodes handled | 240, 39 of them movement and acknowledgements | 269, plus 22 movement opcodes |
| World | vmangos, from Thistle Tea's export: the same creatures, objects, loot, quests and events | vmangos |
| Model | an actor per map and per copy; packets handled as they arrive; a 20 Hz update | a process per player and per creature |
| Units and combat | vmangos stats, XP, regen, the hit table, threat, AI with paths, leashes and spell lists | the same, with more class scripts |
| Spells | the casting pipeline, the effects and auras trainers' spells use to level 20, talents, skills | 98 effects, 172 aura types, procs, pets |
| Content | loot, items, vendors, trainers, gossip, quests, taxis, banks; whispers, channels, friends, `/who`, groups and raids, guilds, mail, auctions, trade, duels; dungeon and raid copies, Warsong Gulch, boats and zeppelins, events, weather | the same, with Arathi Basin, Alterac Valley, pets, professions' skill-ups and honor |
| Persistence | SQLite: characters, and guilds, charters, mail, auctions and raid binds as records; versioned, saved at logout, on a timer and on SIGTERM | none: a restart loses every character |
| Security | accounts with SRP verifiers, GM levels and bans; movement checks; packet and chat rate limits | every session is a GM; positions trusted |
| Under load (peon, 100 bots) | 0.33 cores for a crowd, 0.10 for the pit's fight; login 8 ms; ping, movement, chat and name query under 2 ms at p99 | 6.1 and 6.0 cores; login 45 to 110 ms; ping p99 220 to 420 ms, name query p99 to 1.3 s |

## Goal

**Capability:** a client can do on wow_vanilla_server what it can do on Thistle Tea, phase by phase in the order
below, each phase checked by tests through the real protocol and by peon.

**Cost:** at the same load, a fraction of Thistle Tea's CPU, and replies as quick as its own (a millisecond or two)
without its tails.

**Measured by:** peon's baseline scenarios on both servers, on the same world (Phase 12), with every behaviour
switched on in wow_vanilla_server's profile; and by an opcode coverage count against Thistle Tea's.

## The design

The world tick goes. What replaces it:

### Sessions

A task per connection runs the session: the handshake, then the character screen, served at once from the
character store (enum, create, delete, login), off any tick. At login it loads the character and hands a `Player`
to its map. From then on it routes the client's packets to that map, and back to itself after a logout. A reader
task decodes packets and a writer task encrypts and coalesces writes (both kept from today's code).

### Maps

Each map, and later each instance, is an actor: a task that owns everything in it (players, creatures, game
objects, corpses, its navmesh) with no locks.

- **Packets are handled as they arrive**: a movement packet is relayed, a chat line delivered and a query answered
  in the same wake-up, never on a later tick.
- **A 20 Hz update** (50 ms, run on measured `dt`) does what time drives: creature AI and movement, swing timers,
  regen, auras, respawns, logout timers. Timers due later are parked in a timer queue, so idle creatures cost
  nothing; only grids near players are active (as cmangos and vmangos do).
- **Maps run in parallel**: Eastern Kingdoms, Kalimdor and each instance on their own task.

### Objects and updates

Each object keeps its update fields in a flat array with a dirty bit per field.

- At the end of each update, an object with dirty fields encodes one values block for public fields, once, and
  shares the bytes with every observer; its owner gets a second block with the private fields.
- Each observer's blocks are batched into one `SMSG_UPDATE_OBJECT` per update; big ones (logins, crowds coming into
  view) are compressed.
- Movement relays are encoded once and shared, and are sent at once.

### Visibility

- **A grid of cells**, with an exact distance test at a radius of 100 yards by default (vmangos's), configurable,
  with hysteresis so units at the edge don't flicker.
- **A player's view is recomputed** when they have moved some yards since the last time, or when something near
  them spawns or goes. Not every tick.
- **Aggro and assistance are found from the grid** in the creature's update, instead of announcements.

### Shared services

Some things span maps: the names of all characters, who is online, chat channels, groups, guilds, friends, mail,
the auction house. They live in services the maps call directly, each behind its own short lock, never held across
an `.await`. A service reaches a player through the registry of online players: their outbound queue, and a handle
to their map for anything that must change their state there.

### Persistence

- **Characters are kept in SQLite** (`characters.sqlite`), with a versioned schema migrated at start-up. That covers
  characters, items, spells, action bars, quests, reputation, social lists, mail, auctions and guilds.
- **A writer thread** applies saves in transactions: at logout, every few minutes for each player, and for everyone
  on SIGTERM or Ctrl-C before the process exits.
- `snapshot.bin` goes.

### Game data

The game data comes from vmangos's database and the client's DBCs, exported to SQLite as Thistle Tea does
(`vmangos.sqlite`, `dbc.sqlite`). It is loaded once at start-up into immutable tables shared by every map.

- **Patch:** for templates with a `patch` column, the row with the highest patch up to 10 (1.12) is taken; spawns
  are taken if their patch range includes 10.
- **Without the databases** (in tests), the server runs with an empty world.

### Tests

- **An in-process harness** runs a map with sessions on channels. A test sends real `ClientOpcodeMessage`s and
  parses what comes back with wow_messages, which checks the server's encoding as well.
- **peon** runs the end-to-end measure.

## Phases

Each phase ends in a commit on `work/loadtest`, pushed. Each lists what it adds and when it is done.

### Phase 1: the new core

**Adds:**
- Sessions, map actors, the registry of online players, the update-field model with dirty masks, the visibility
  grid, the SQLite character store with its writer thread and save on exit, and the test harness.
- Ported from the old world:
  - login (the full login packet set) and logout;
  - movement relay and teleports, within a map and between maps;
  - chat (say, yell, emote, text emotes);
  - name, creature and item queries;
  - stand state (sit), the 20 s logout timer;
  - the GM commands worth keeping.
- The old world module, `snapshot.bin` and the cmangos loader are deleted.

**Done when:**
- the harness tests pass, with at least one test for each opcode the phase handles;
- characters survive a restart, and a SIGTERM;
- peon's `idle`, `wander`, `cluster`, `race`, `login-storm` and `login-churn` run clean at 100 bots, with replies
  off the tick: movement, chat and name query p50 under 5 ms, login p50 under 100 ms.

### Phase 2: the world

**Adds:**
- The vmangos and DBC loader, with patch selection.
- Creatures:
  - template, model and scale, equipment, faction and NPC flags;
  - level ranges;
  - random movement within `wander_distance`;
  - waypoints from `creature_movement`.
- Game objects, with their template, state and rotation.
- Zone and area from rustigator's `get_zone_and_area`; area flags from `AreaTable`, among them the free-for-all
  ones (Gurubashi Arena).
- Area triggers, and their teleports.
- Creature and game object queries from the templates.
- The navmesh for every map in the bake.

**Done when:**
- the world holds vmangos's patch-10 spawns, with the same creature and game object counts as Thistle Tea;
- peon's NPC errands find NPCs at the hotspots;
- the pit is free-for-all.

### Phase 3: units, stats and death

**Adds:**
- **Players:**
  - base stats from `player_levelstats` and `player_classlevelstats`;
  - health, mana, rage and energy, with vmangos's regen;
  - levels and XP from `player_xp_for_level`, with rested XP;
  - creation from `playercreateinfo` (position, spells, action bars, items).
- **Creatures:** stats from `creature_classlevelstats` and the template's multipliers.
- **Death:** release, ghost, corpse, the nearest graveyard (`game_graveyard_zone`, `WorldSafeLocs`), reclaim delay,
  spirit healers and resurrection sickness.

**Done when:**
- a created character has vmangos's starting stats, spells, action bar and items;
- a player who dies can release, run back and reclaim, or take the spirit healer.

### Phase 4: combat and creature AI

**Adds:**
- **Melee:**
  - the hit table (vmangos's `RollMeleeOutcomeAgainst`: miss, dodge, parry, glancing, block, crit, crushing);
  - weapon damage, armor, attack power;
  - swing timers that start ready;
  - attack errors (not in range, facing, not standing, dead target, can't attack).
- **Hostility:** faction hostility from `FactionTemplate`; PvP flags, contested and free-for-all areas.
- **Creature AI:**
  - threat;
  - aggro radius by level difference;
  - chase on the navmesh, leash and evade;
  - call for help and assistance.
- **After a kill:** respawn from `spawntimesecsmin`/`max`, corpse decay, tapping, XP for kills, honorable kills.

**Done when:**
- peon's `pvp-arena` and its hunts work, and a swing's first result comes on the swing, not 1.8 s later;
- creatures fight back, chase, leash and respawn;
- kills give vmangos's XP.

### Phase 5: items, loot, money and vendors

**Adds:**
- **Items:**
  - item templates;
  - item instances, with an inventory of backpack, bags, bank and bank bags;
  - equip rules (class, race, level, proficiency, two-hand, unique);
  - swap, split, destroy, auto-equip, auto-store.
- **Loot:** loot from every vmangos loot table (creature, game object, fishing, pickpocket, skinning, item,
  reference), with groups and quest-only drops.
- **Money and vendors:**
  - money, and loot windows;
  - vendors: list, buy, sell, buy back, limited stock;
  - repair, and durability.
- **Gathering:** herbs, veins and chests, as looted game objects.

**Done when:**
- a killed creature's loot can be taken;
- vendor buy, sell and buy back work;
- equipment changes what other players see and the player's stats;
- inventories survive a restart.

### Phase 6: NPC services and quests

**Adds:**
- **NPC services:**
  - gossip menus with options, conditions and NPC text;
  - trainers (list, learn, cost, requirements);
  - innkeepers (bind, hearthstone);
  - bankers and bank slots;
  - flight masters (discovery, node lists, flights on `TaxiPathNode` splines);
  - battlemaster lists;
  - the auctioneer's hello.
- **Quests:**
  - quest givers (creatures, game objects and items), with status icons;
  - the dialog from hello to reward;
  - objectives for kills, items, exploration and spell casts;
  - prerequisites and exclusive groups;
  - timed, repeatable and party-shared quests;
  - the quest log in the update fields.

**Done when:**
- peon's NPC errands get a real reply from every kind of NPC;
- a starting zone's quest chain can be done start to end;
- flights land.

### Phase 7: spells and auras

**Adds:**
- **Spell data** from `spell_template`, with ranges, durations and cast times from the DBCs.
- **Casting:**
  - validation: range, LOS, facing, power, reagents, cooldown, GCD, state;
  - cast times, pushback, interrupts;
  - channels, and auto-repeat.
- **Effects:** damage, heal, apply aura, energize, weapon damage, teleport, create item, learn spell, trigger spell,
  dispel, resurrect, summon, and the class scripts that need code.
- **Auras:** periodic, stat and speed mods, root, stun, silence, absorbs, procs and stacking.
- **Talents.**
- **Creature spell lists** (`creature_spells`).
- **Hunter and warlock pets.**
- **Crafting**, as create-item spells.

**Done when:**
- every spell a trainer teaches up to level 20 uses only effects, auras and targets the server supports (checked by
  a test, as Thistle Tea checks its support matrix);
- every class can fight with its starting spells;
- creatures cast.

### Phase 8: social

**Adds:**
- Whispers, with AFK and DND.
- Chat channels: join, leave, list, moderation, the default zone channels.
- Friends and ignore lists, persisted.
- `/who` with its filters.
- Inspect.
- Groups and raids: invite, accept, decline, kick, leader, loot method, subgroups, ready check, target icons, and
  party member stats, rate-limited.
- Party chat, raid chat and raid warnings.
- Random roll, and minimap pings.

**Done when:**
- peon's `channel`, `whisper`, `party`, `who` and `inspect` behaviours get their replies;
- its profile switches them on.

### Phase 9: guilds, mail, the auction house, trade and duels

**Adds:**
- **Guilds and petitions:** charters, ranks, MOTD, roster, events, guild chat.
- **Mail:** money, COD, items, expiry and returns.
- **The auction house:** faction and neutral houses, deposits, bids, buyouts, expiry, settlement by mail.
- **Trade.**
- **Duels.**

**Done when** each has a harness test from start to end, and survives a restart where it should.

### Phase 10: instances, battlegrounds, transports and the world around

**Adds:**
- **Instances:**
  - an actor per copy;
  - entry through area triggers, with level requirements;
  - binds, resets, `CMSG_RESET_INSTANCES`.
- **Battlegrounds:**
  - queues;
  - Warsong Gulch, Arathi Basin and Alterac Valley;
  - scoreboards, world states, deserter.
- **Transports:** boats, zeppelins and elevators.
- **The world around:** weather, game events (seasonal spawns and quests), world states.

**Done when:**
- a group can enter a dungeon and get its own copy;
- a Warsong Gulch match runs to a win;
- a boat crossing carries its passengers.

### Phase 11: security and operations

**Adds:**
- **GM rights:** GM levels per account, every GM command and opcode gated by them.
- **Accounts:**
  - account creation by policy, not for any name;
  - name rules, and faction rules on a PvP realm;
  - one session per account (a second login kicks the first);
  - bans.
- **Movement validation:** speed, teleport and fall checks, and finite, in-bounds positions.
- **Rate limits** on packets and chat.
- **Fixes** for the crashes the code review found.
- **Metrics:** per-map update time, players, creatures and queue depths.

**Done when:**
- a non-GM can't run a GM command;
- a teleporting client is pulled back;
- a packet flood is cut off;
- peon's profile still runs (its bots are GMs where they need to be).

### Phase 12: the re-baseline

**Adds:** peon's baselines on both servers on the same world, with every behaviour switched on in wow_vanilla_server's
profile, and peon's BASELINES.md updated.

**Done when** the reports are committed in peon and the findings are written up.

### Phase 13: spells to level 60

Of the 3,497 spells a player can have (trainers' to 60, talents, racials, and what they trigger), 1,377 run every
effect they have. The rest lean on talents' spell modifiers (769 spells), procs (260), class scripts, and the
auras of crit, haste, avoidance and damage taken.

**Adds:**
- **Spell modifiers** (auras 107 and 108): talents change a spell's points, duration, cost, cast time, cooldown,
  range, radius, crit chance and damage, threat and resist chance; the client is told.
- **Outcomes:** spell hit and resist by level, crits for damage and heals, partial resists from resistance, school
  and mechanic immunities, spell power and healing power with each spell's coefficient, and the misses in
  `SMSG_SPELL_GO`.
- **The auras of combat:** crit, hit, dodge, parry and block chance; haste and attack speed; damage done and taken,
  flat and by percent; stats and resistances by percent; threat; regen while casting; block value; damage and crit
  against a creature type.
- **Procs:** proc-trigger auras (spell and damage) with their flags, chance and charges; damage shields; on-hit
  target and caster triggers.
- **Melee and ranged:** next-swing spells, auto-repeat (Auto Shot, Shoot, Throw) with the ranged weapon and ammo,
  finishing moves that scale with and spend combo points, stacking auras, line of sight and facing.
- **Effects:** the class scripts that need code; enchanting items (permanent and temporary: poisons, imbues,
  stones); mana burn; totems; traps; portals and rituals; dispelling by mechanic; vanish; skinning; pick pocket;
  distract; far sight; stuck; weapon proficiencies; languages.
- **Stealth and invisibility**, and their detection.
- **Pets:** taming, calling and dismissing; warlock demons; summoned guardians; a pet's AI (follow, attack, stay;
  aggressive, defensive, passive), its action bar and spells, a hunter pet's XP and happiness; pets kept across a
  logout.
- Auras and cooldowns kept across a logout.

**Done when:**
- at least 90% of the player spells run every effect they have (a test counts them);
- each class's core at 60 works in a test: a mage's Fireball crits and Ignites, a rogue's finishers scale with
  combo points, a hunter auto-shoots with a pet;
- peon's parse errors for the game object query, `/who`, loot and cast results are gone.

### Phase 14: Arathi Basin, Alterac Valley and honor

**Adds:**
- **Arathi Basin:** five bases to assault and defend, resources by bases held, graveyards at bases, 2,000 to win.
- **Alterac Valley:** graveyards and towers to take, reinforcements, captains and generals, the win on a general's
  death.
- **Honor:** honorable and dishonorable kills, contribution points, the weekly rank by vmangos's formula, rank titles
  and rewards, battleground honor and marks.

**Done when** a test plays each battleground to its end, and a week's kills give the rank vmangos would.

### Phase 15: group loot and professions

**Adds:**
- **Group loot:** free for all, round robin, master looter, group loot and need before greed, with the quality
  threshold and the rolls.
- **Professions:** crafting with spell focus, tools and reagents; skill-ups by colour; gathering (herbs, mining,
  skinning) by skill; fishing, with its bobber and zone loot; recipes; disenchanting.

**Done when** a group rolls on an item in a test, and a skill rises by crafting and by gathering.

### Phase 16: instance scripts and saved boss kills

**Adds:**
- **EventAI** (`creature_ai_scripts`): timers, health and mana thresholds, aggro, death, evade, spell hits, ranges,
  and their actions (cast, summon, say and yell, phases, flee, despawn).
- **The database scripts** for gossip, quests, spells and events.
- **Boss kills** kept with the instance: a copy's dead stay dead across a restart, and a raid's lockout holds them.

**Done when** a dungeon boss's EventAI runs in a test and a copy's kills survive a restart.

## What is out of reach, and why

Thistle Tea has years of content behind it. Some of it is listed in these phases at a lower depth than its own:

- **Its 61 hand-ported boss scripts and 19 instance scripts.** Phase 10 brings instances and their doors and
  bosses, as the database scripts drive them. Thistle Tea itself has none for BWL, ZG or AQ.
- **Its EventAI and script coverage** (6,220 of 6,256 event rows).
- **Class scripts beyond what the starting spells need.**

What each phase leaves for later is written into this plan as the phase is done.

## Progress

Each phase's commit, and what it measured, as it lands.

### Phases 1 to 4 (2026-10-16)

The new core, the world, units and combat landed together: the world module they replaced was deleted in one go,
and the later phases build on the core's types.

**Phase 1, the core.** Sessions, map actors, the registry, update fields with dirty bits, the visibility grid, the
SQLite store (migrated, saved at logout, every five minutes and on SIGTERM), and the harness. peon's baselines at
100 bots, before (the old tick) and after, p50 / p99 ms:

| Scenario | Login | Movement | Chat | Name query |
|---|---|---|---|---|
| idle | 356 / 406 → 3.3 / 18.2 | 42 / 99 → 0.0 / 1.0 | 54 / 95 → 0.0 / 1.0 | 101 / 101 → 0.1 / 0.3 |
| wander | 356 / 406 → 3.2 / 17.2 | 53 / 137 → 0.0 / 1.0 | 42 / 97 → 0.0 / 1.0 | 101 / 103 → 0.1 / 0.2 |
| cluster | 356 / 406 → 3.2 / 18.7 | 59 / 141 → 0.0 / 1.0 | 42 / 101 → 0.0 / 1.0 | 3 / 307 → 0.4 / 0.8 |
| race | 356 / 406 → 3.2 / 19.2 | 59 / 141 → 0.0 / 1.0 | 51 / 93 → 0.0 / 1.0 | 97 / 203 → 0.1 / 0.5 |
| login-storm | 356 / 406 → 3.1 / 15.0 | 51 / 112 → 0.0 / 1.0 | 41 / 97 → 0.0 / 1.0 | 2 / 110 → 0.1 / 0.3 |
| login-churn | 195 / 406 → 1.7 / 16.3 | 51 / 126 → 0.0 / 1.0 | 42 / 93 → 0.0 / 1.0 | 1 / 101 → 0.1 / 0.2 |

These ran without world data, as the old baselines did. The reports are in peon's `results/2026-10-16/wvs-phase1/`.

**Phase 2, the world.** vmangos's patch-10 world loads in 2.3 s: 66,001 creatures and 56,611 game objects (Thistle
Tea's counts), 10,381 creature templates, 17,707 items. Eastern Kingdoms and Kalimdor populate in 25 ms each, at
boot. Creatures wander near home and walk their waypoints on the navmesh, but only when a player sees them. Zones and
areas come from rustigator, and with them Gurubashi Arena's free-for-all: in a 30-bot `pvp-arena` smoke test,
same-faction bots killed each other 28 times. peon's NPC errands and hunts find their targets.

**Phase 3, units and death.** Characters start with vmangos's position, stats, spells, action bars and gear (placed by
inventory type). Health, mana, rage and energy regenerate by vanilla's class formulas; levels and XP follow
`player_xp_for_level`, with `SMSG_LEVELUP_INFO`. A dead player releases to the nearest graveyard of their zone and
faction as a ghost, which the living don't see, and comes back at their corpse after 30 s, or at a spirit healer.

**Phase 4, combat and creature AI.**
- **Melee:** vmangos's single-roll hit table, weapon damage from attack power, armor, rage, and the attack errors.
  A swing starts at once: `pvp-arena`'s first-result p50 fell from 1.8 s to 0.25 s.
- **Hostility:** faction templates.
- **Creatures:** they notice hostile players at vmangos's aggro radius (with line of sight), call for help, chase on
  the navmesh, leash and evade, die, decay and respawn on their spawn's timer.
- **Rewards:** kills give vmangos's XP; enemy players give honorable kills.

**Left for later:**
- Rested XP, ghost speed by race, resurrection sickness and durability loss (they need auras and items: Phases 5
  and 7).
- Parry and dodge chances by class and level.
- Ranged attacks.
- Contested areas.
- Honor ranks.
- peon reads `SMSG_GAMEOBJECT_QUERY_RESPONSE` with wow_messages, whose layout (five names, six data words) isn't
  1.12's (four names, 24 data words); wow_vanilla_server sends 1.12's, so peon counts those as parse errors.

### Phase 5 (2026-10-16)

**Items:** vmangos's item templates, items as objects with typed GUIDs, and bags as containers. Starting gear
is worn by inventory type and the rest carried. Swap, split, destroy, auto-equip and auto-store, with the equip
rules (class, race, level, proficiency by class, two-handers, dual wield). Gear's stats and armor feed the
player's stats, and others see what a player wears.

**Loot:** rolled vmangos's way from every loot table (ungrouped chances, one per group, references), with gold, for
the creature's tapper. The loot window is in 1.12's layout. Chests without a lock open, doors and buttons flip, and
chairs are sat on.

**Vendors:** their lists (an entry's and its vendor template's), limited stock that restocks, buy, sell (to twelve
buyback slots), buy back, and repair at DBC durability costs. Gear wears 10% on a death to a creature and 25% at a
spirit healer.

**Left for later:**
- Bank slots and bankers, and gathering (herbs, veins, skinning, fishing need skills and spells): Phases 6 and 7.
- Quest drops: Phase 6.
- Random properties and enchantments.
- peon reads the loot window with wow_messages, whose layout isn't 1.12's (it has six bytes an item where 1.12 has
  twenty-two), so its bots don't take loot.

### Phase 6 (2026-10-16)

**NPC services:**
- **Gossip** menus from vmangos (options by the NPC's flags) with their NPC texts and broadcast texts, and the
  options' actions: submenus, quests, vendor, trainer, taxi, bank, innkeeper, battlemaster, auctioneer, armorer.
  An NPC without a menu opens its one service.
- **Trainers** list their spells (green, red, gray by level and what is known) and teach them for money.
- **Innkeepers** bind; **bankers** open the bank and sell bank bag slots at the DBC prices.
- **Flight masters** learn their node, list the known ones, and fly the player along `TaxiPathNode` on a mount,
  as a flying spline.
- **Battlemasters** answer with their battleground; **auctioneers** with their house.

**Quests:**
- All of vmangos's quests and who starts and ends them; questgiver icons.
- The dialogs: hello, list, details, accept, request items, offer reward, choose reward.
- Prerequisites (previous quest, exclusive groups, level, race and class), the log in the update fields, abandon.
- Kill credit, items counted from the bags, exploration by area trigger, and quest items that drop only for those
  on the quest.
- Rewards: items (with a choice), money, XP scaled down as the player outlevels the quest, and spells. The chain's
  next quest is offered at once.
- The journal is kept with the character.

**Left for later:**
- Conditions on gossip options and loot.
- Timed quests, repeatable and daily handling beyond the flag, party sharing (Phase 8), reputation rewards.
- Quest start and end scripts, escort quests.
- Multi-hop express flights.

### Phase 7 (2026-10-16)

**Spell data:** vmangos's `spell_template` (the latest build up to 5875), with cast times, durations, ranges and
radii from the DBCs, `spell_target_position`, talents, `SkillLineAbility` and `creature_spells`.

**Casting:**
- **Checks:** known, a cast already going, the spell's and its category's cooldowns, the global cooldown,
  reagents, stunned, feared or silenced, power, and a target that is there, alive, on the right side and in range
  (with a minimum for ranged spells).
- **Cast times**, pushback (half a second a blow, twice at most), and interrupts by moving, stuns, silences and
  interrupt effects.
- **Channels**, which end, or are broken, with their aura.
- `SMSG_SPELL_START`, `SMSG_SPELL_GO`, and `SMSG_CAST_RESULT` in 1.12's layout. The result goes to the caster on
  success too, as Thistle Tea does.
- Items' on-use spells.

**Effects:** instakill, school damage, leech, heal (to full too), apply aura (party and persistent area too),
energize, power drain, the weapon damages, teleport (to the spell's place, or the hearthstone's home), leap,
charge, create item, learn spell, trigger spell, dispel by type, interrupt, threat, combo points, resurrect (with
the request and the answer), and skills. Area targets (around the caster, the target or a place, and the cone in
front) find the enemies there.

**Auras:** one slot each in the unit's 48 (32 helpful, 16 harmful), shown with their durations, refreshed rather
than stacked from the same caster. Periodic damage, heals, leech, energize and triggered spells tick. Stat,
resistance, health and attack-power mods, speed up and down, root, stun, silence, fear and confusion (which hold),
shapeshifts, mounts, tracking and absorbs work. The rest are kept on the unit with no effect. Passive spells hold
from login and from learning. Death takes every aura but the passive ones.

**Skills:** spells with a skill effect teach them; weapon and class skills cap at five a level, languages at 300,
professions at 75 a step.

**Talents:** a point a level from ten. A row opens at five points a row in its tree, and the talent it needs must be
learned.

**Creatures** cast from their `creature_spells` lists (chance, repeat delay, at the victim or themselves), standing
still while they cast. An evade takes their auras and casts away.

**Done when:**
- A test against the real data finds every spell a trainer teaches up to level 20 uses only effects, auras and
  targets the server knows. "Knows" includes the ones it keeps without an effect, listed in `effect_supported` and
  `auras::supported`.
- Another test casts each class's starting spells at a kobold. Every harmful one hurts it; every helpful one does
  something.
- Creatures cast (tested).

**Left for later:**
- Line of sight and facing checks.
- Auto-repeat. Auto Shot, Shoot and Throw are single casts, using melee weapon damage.
- Next-swing spells (Heroic Strike, Raptor Strike) strike at once.
- Procs: seals and proc-trigger auras are kept on the unit but do nothing.
- Stacks, and spell power coefficients.
- Spell hits, crits and resists.
- Finishing moves don't scale with combo points.
- Dummy-effect class scripts.
- Summons and pets.
- Spell focus, tools and skill-ups for crafting.
- Auras and cooldowns aren't saved across a logout.
- wow_messages reads `SMSG_CAST_RESULT`'s status the wrong way round, so peon counts it as a parse error. It does
  the same on Thistle Tea, which sends the same bytes. peon doesn't cast yet.

### Phase 8 (2026-10-16)

Social state that spans maps lives in `game::social`, each part behind its own lock. Players elsewhere are reached
through the registry.

**Chat:**
- **Whispers** go to the player of that name on the same side, with the echo to the speaker. Those not found get
  `SMSG_CHAT_PLAYER_NOT_FOUND`. AFK lets a whisper through and answers with its message; DND holds it back.
  Someone ignoring the speaker answers with `Ignored`.
- `/afk` and `/dnd` set the player flag and the message.
- **Party, raid, raid-leader and raid-warning chat.** Raid chat needs a raid; the leader's lines and warnings need
  the leader or an assistant. In a raid, party chat goes to the subgroup.

**Channels:** the built-in ones from `ChatChannels.dbc` and custom ones, one set per team. Join (with password and
bans), leave, the list, the owner (shown and passed on), moderators, mutes, kick, ban and unban, announcements and
moderation, and invites. Lines skip those who ignore the speaker. LocalDefense and WorldDefense are for GMs, as in
mangos without ranks. The notices are laid out as mangos-zero's.

**Friends and ignore lists:** 50 friends and 25 ignored, kept with the character. They are sent at login with each
friend's status, area, level and class. Logins and logouts are told to those with the player as a friend.

**`/who`:** those online on the same side, filtered by level, name, race, class, zones and up to four words
(names and zone names), 49 at most, in 1.12's layout.

**Inspect:** the target hears of it; what they wear is in their public fields. The honor tab answers with none.

**Groups and raids:**
- Invite, accept, decline, kick (a member, or an invite back), leave, a new leader, and the loot method.
- Convert to raid, subgroups, assistants, the ready check and its answers, and target icons.
- A party of two that loses one is disbanded.
- Members stay in the group across a logout, shown offline.
- **Stats:** each map sends its grouped players' changed stats once a second to the members who can't see them.
  A member just out of sight gets all of them. A request gets the full stats.
- **Random roll** to the group (or the roller), and **minimap pings** to the rest of the group.
- **A kill is shared** with the members within 74 yards of the corpse, as vmangos shares it. XP is split by
  level with the group bonus (1.166, 1.3 and 1.4 for three, four and five), and goes only to those not above the
  highest member who doesn't find the creature gray. Every member gets the quest credit.

**Done when:** harness tests cover each of these. peon's channel, whisper, party, who and inspect behaviours are on
in the run's profile.

**Left for later:**
- The group loot rules: round robin, master looter, need before greed. The tapper loots, as before.
- Meeting stones and the LFG channel's tools.
- Groups and channel bans aren't kept across a restart.

### Phase 9 (2026-10-16)

Guilds, charters, mail and auctions are kept in the store's new `records` table (schema version 2), one JSON record
each, and loaded at start-up. Mail and auctions are swept every 5 seconds for what has come due.

**Guilds:**
- Five default ranks with mangos's rights, up to ten.
- Invite (same team), accept, decline, leave, remove, promote and demote (within one's own rank), a new leader,
  and disband.
- The MOTD and info, public and officer notes, rank names and rights.
- The roster (officer notes for those allowed), `SMSG_GUILD_INFO`, and the guild query with ranks and emblem.
- The emblem, bought for 10 gold at a tabard vendor.
- Guild and officer chat by rights, and the events: joined, left, removed, promotions, the MOTD, leader changes,
  disbanding, tabard, signing on and off.
- The guild fields on every member, in whatever map they are.
- `/who` shows the guild.
- Creating a guild outright is for GMs.

**Charters:** bought for 10 silver from a guild master (item 5863). Offered, signed (one signature an account, not
by those in a guild) or declined, renamed, and turned in once signed enough (9 by default, `petition_signatures`).
The owner leads, and the signers join at the lowest rank.

**Mail:**
- Money, one item, and cash on delivery, for 30 copper postage (60 with an item).
- With money or an item, it takes `mail_delay_secs` (an hour by default) between accounts.
- Taking the money and the item; cash on delivery is paid to the sender by mail.
- Reading, returning, deleting, and copying the text onto a letter. The text is kept.
- `SMSG_RECEIVED_MAIL` when mail arrives (now, or when it comes due), and the next-mail-time query.
- Mail lasts 30 days (cash on delivery 3), then goes back to a player who sent money or an item, or away.
- The auction house and its settlement use the same mail.

**The auction house:**
- Three markets (Alliance, Horde, goblins) by the auctioneer's faction.
- `AuctionHouse.dbc`'s deposits (mangos's formula, counted in 2-hour periods) and cuts (5%, or 15% at the goblins).
- 2, 8 and 24 hour auctions.
- Bids with mangos's 5% increment. The one outbid gets their money back by mail and a notification.
- Buyouts. Cancelling pays the cut if there is a bid, and the bidder is paid back.
- Searches by name, level, slot, class, subclass and quality, 50 a page, and the owner's and bidder's lists.
- When an auction runs out, the winner gets the item and the seller the money (after the mail delay). An unsold
  item comes home.

**Trade:** with a player within 11 yards on the same side.
- Six slots, money, and the seventh slot that isn't traded. Soulbound and worn items stay out.
- Any change takes both acceptances back.
- When both accept, it goes through at once if each can pay and has room, and not at all otherwise.
- Death, logout or leaving the map cancels it.

**Duels:**
- The Duel spell's effect plants a flag between the two.
- The challenged accepts; three seconds' countdown; then the duel fields, and the two may fight whatever their
  factions.
- A blow that would fell one leaves them at 1 health: they kneel (Grovel) and lose.
- Cancelling yields once the fight is on.
- Out of bounds (75 yards from the flag) for 10 seconds flees.
- Logging out or leaving the map flees; dying to something else interrupts.
- `SMSG_DUEL_WINNER` goes to those near, and the flag goes.

**Done when:** a harness test runs each from start to end: guild, charter, mail with cash on delivery, auction to
settlement, trade, duel, and expiry sweeps. Another restarts the store and finds the guild and mail (with its
item) kept.

**Left for later:**
- Guild bank (not in 1.12) and guild rank permissions over the roster's columns.
- Deleting a character doesn't yet clean up their guild, mail and auctions.
- Item enchantments, random properties and charges in mail, auctions and trade.
- Auction search's "usable" filter.
- Mail from creatures and quests.

### Phase 10 (2026-10-16)

**Instances:**
- Each dungeon and raid copy is a map actor of its own, keyed by the map and an instance id. Any teleport into an
  instanced map (area triggers, GM commands, death's way back) picks the copy.
- A group shares its copies; a player alone has theirs.
- Raids need a raid group (GMs excepted).
- A copy holds 5 (10 for Blackrock Spire, 20 for Zul'Gurub and the Ruins, 40 for other raids).
- A dungeon copy unloads after half an hour empty (`instance_unload`), so the next visit is fresh.
  `CMSG_RESET_INSTANCES` resets the player's (or the leader's group's) empty copies.
- Raids bind those who enter until the raid's reset (weekly, 5 days for Onyxia, 3 for Zul'Gurub and the Ruins,
  counted from a Tuesday 04:00 UTC). The binds are kept as records and listed by `CMSG_REQUEST_RAID_INFO`.

**Battlegrounds:**
- Queues per battleground and level bracket from `battleground_template` (players per side, levels, start
  locations) and the battlemasters' `battlemaster_entry`. GMs may queue from anywhere.
- A match when both sides have enough; later ones are topped up from the queue. Invitations last two minutes.
- `SMSG_BATTLEFIELD_STATUS` is in 1.12's layout through queued, invited and in progress.
- Warsong Gulch's rules on the match's copy:
  - The countdown (`bg_start_delay`, two minutes), then the doors open and the flags go up.
  - Each side takes the other's flag, with its aura, and captures at home while its own flag is there.
  - A carrier who falls drops it: their own side returns it; the other side picks it up again.
  - Three captures win, with the template's win and loss spells.
  - World states for the captures and the flags; announcements; the scoreboard (`MSG_PVP_LOG_DATA`).
  - The fallen rise at their side's start every 30 seconds, with the spirit healer's timer.
  - Two minutes after the end, everyone goes home.
  - Leaving before the end gives Deserter.

**Transports:**
- The boats and zeppelins of `transports` follow their `TaxiPathNode` paths in their periods, at a steady speed
  with the paths' stops.
- They are created with the server's clock (`UPDATEFLAG_TRANSPORT`), and everyone in their map knows them,
  however far.
- Where a path crosses to the other continent, the transport goes there, and those aboard (by their movement's
  transport GUID) go with it at the same place on deck. `SMSG_TRANSFER_PENDING` carries the transport.
- Elevators (type 11) carry the server's clock too.

**The world around:**
- Seasonal events from `game_event` (its start, end, occurrence and length). Hard-coded and disabled ones are left
  out. Creatures and objects tied to an event spawn while it runs (or, with a negative id, while it doesn't), and
  each map brings them into line as events start and end.
- Weather from `game_weather`: every ten minutes, a zone keeps its weather three times in ten, or turns to rain,
  snow, a storm or fair by the season's chances. Those in the zone are told, and so is anyone entering it.

**Done when:** harness tests check that:
- a group shares a copy and a player alone has another;
- an empty copy unloads, and a reset starts afresh;
- a raid needs a raid group and binds;
- a Warsong Gulch match runs to a win;
- a boat crossing carries its passenger to the other continent;
- event spawns come and go;
- the weather changes and the zone hears.

A test against the real data checks the routes, including the Booty Bay boat's crossing, and the events,
weather and battleground templates.

**Left for later:**
- Arathi Basin and Alterac Valley: queues and copies work, but not their rules (bases, resources, captains).
- Instance boss kills, kept across a restart (and so the raid lockout's content).
- The transport's turn: offsets on deck are kept, not rotated.
- Event quests, and the scripted events: Darkmoon Faire, invasions.
- Zone world states outside battlegrounds.

### Phase 11 (2026-10-16)

**Accounts** (`accounts.rs`) live in the character store's file (table `accounts`): the name, the SRP salt and
verifier, a GM level and a ban. The login server and the world share them.
- `[accounts] creation`:
  - `auto` makes an account at its first login, with its name as its password (development and peon);
  - `existing` admits only accounts made with `.account create`.
- The login server answers an unknown account, a banned one, and a wrong password, instead of going quiet.
- `everyone_gm` is now off by default; the run's config turns it on for peon.

**GM levels:** 1 moderator, 2 game master, 3 administrator. They come from the store (`.account gm`) or the
config's `gm` table.
- Moderators look and talk: `.gps`, `.online`, `.maps`, `.save`, `.announce`, `.kick`, `.gm`.
- Game masters change the world: teleports, spawns, levels, deaths and revivals, `CMSG_WORLD_TELEPORT`, making
  guilds, queueing for a battleground from anywhere.
- Administrators run accounts: `.account create|password|gm`, `.ban <account> <minutes|perm> [reason]` (which
  kicks them too), `.unban`.
- A player's `.` line is just chat.

**Characters:**
- Names are 2 to 12 ASCII letters, capitalised. Three of a letter in a row are allowed, as in mangos-zero:
  the re-baseline found a peon name ("Devdachhh") that a stricter rule turned away.
- None of the config's `reserved_names`.
- On a `pvp_realm`, an account's characters are all of one side.
- One session per account was already the rule: a second login kicks the first.

**Movement:**
- Positions must be finite and on the map.
- A move faster than the player's speed (a quarter's tolerance, half a second's lag, three yards) pulls them back
  where they were, as a teleport. GMs and those on transports are exempt.
- Falls past vmangos's safe 14.57 yards hurt by its formula, with `SMSG_ENVIRONMENTAL_DAMAGE_LOG`.

**Rate limits:**
- A token bucket per session: `max_packets_per_sec` sustained (100), three seconds' worth at once. A client that
  floods is disconnected.
- mangos's chat flood control: ten lines each within a second of the last mute for ten seconds.

**Crashes:** the code review's crashes were in the old core, which is gone. On top of that:
- A panic in a map's event or update is logged and costs that one event or update; the map goes on (tested).
- Lookups that could panic on a player who had left now check.

**Metrics:** every minute each busy map logs its players, creatures and objects, its average and slowest update,
and the events waiting in its queue. `.maps` shows the same on demand.

**Done when:** harness tests check that:
- a player's `.go` is only chat, and a moderator can look but not teleport;
- a client moving too fast is pulled back;
- a packet flood is cut off, and a chat flood muted;
- names follow the rules;
- a long fall hurts;
- a panic on a map costs one event.

Unit tests cover the accounts (auto and existing creation, bans, GM levels) and the rate bucket. peon runs with
its bots as administrators (`everyone_gm` in the run's config).

**Left for later:**
- IP bans, and account locks.
- Fly and climb checks beyond speed, and checks on movement timestamps.
- Per-opcode limits beyond the overall rate.
- A metrics endpoint: the numbers are in the log.

### Phase 12 (2026-10-16)

peon's seven baseline scenarios ran on both servers, on the same vmangos world, each on a fresh server, with every
behaviour switched on in wow_vanilla_server's profile (which now places bots with `.go xyz` and revives them with
`.revive`, as Thistle Tea's does). A second `cluster` and `pvp-arena` on each sampled the server's CPU. The reports
and the write-up are in peon: `results/2026-10-16/aligned/`, `results/2026-10-16/aligned-cpu/` and
[BASELINES.md](https://github.com/12932/peon/blob/master/docs/BASELINES.md).

**What it found:**
- **Replies:** login p50 under 8 ms (Thistle Tea's 45 to 110). Ping, movement, chat and name query are under 2 ms
  at p99 in every scenario, where Thistle Tea's tails run to 220 to 420 ms for ping and 1.3 s for name queries.
- **CPU:** 0.33 cores for 100 wanderers in one crowd and 0.10 for the pit's fight, against Thistle Tea's 6.1 and
  6.0: a twentieth, or less. Map updates average 0.9 to 1.4 ms with 100 players among 23,000 to 27,000 creatures.
- **Sent:** Thistle Tea sends more in a fight (28,288 packets a second against 7,919) and to idle bots, and about
  the same in a crowd.
- **Creatures:** wow_vanilla_server's kill bots placed among them, as vmangos's would (one level-1 bot at the
  Westfall hotspot died every ten seconds). Thistle Tea's never killed a bot outside the pit.
- **Two fixes came of it:**
  - The new name rules refused three of a letter in a row, and peon's `Devdachhh` was turned away. The server now
    allows it, as mangos-zero does.
  - peon handed the refused slot to every later bot, which starved `idle`'s fleet. It now retires the slot, and
    `idle` was rerun.
- **Parse errors:** peon can't read wow_vanilla_server's `SMSG_GAMEOBJECT_QUERY_RESPONSE` and `SMSG_WHO` with
  wow_messages: both are sent as 1.12 (and mangos-zero, and Thistle Tea) lay them out.

**Done when:** the reports are committed in peon and the findings written up there, and the comparison above
(Where they stand after the plan) records the result.

### Phase 13's base (2026-10-17)

- **Auras are holders:** one per spell, caster and item on a unit, with stacks, charges, an item source and the
  spell's weapon restrictions.
  - Thistle Tea's stacking rules: a caster's refresh, ranks, `spell_group` rules 1 and 3, and the exclusive kinds
    (seals, blessings, curses, stings, aspects, armors, forms, mounts, food and drink, elixirs).
  - Passive and hidden auras take no slot. Slot flags, levels and applications follow Thistle Tea's.
- **Derived stats** (`derive.rs`), worked out again on every gear, level or aura change:
  - stats with percent auras, health and power, armor and resistances;
  - attack power and ranged attack power, weapon damage for both hands and ranged;
  - haste and cast speed;
  - crit, dodge, parry and block by vmangos's class rates, spell crit and spell power.
  - Creatures keep their spawn values as a base. Gear's on-equip spells are auras that come and go with the item.
- **Spell data:** weapon restrictions, aura states, focus and totems; vmangos's `spell_mod` and `spell_effect_mod`
  corrections; `spell_proc_event`, `spell_chain`, `spell_threat`, `spell_group` and `spell_elixir`; and the item
  enchantments.
- wow_messages, patched in a local branch, reads 1.12's game object query, `/who`, loot window and cast result;
  peon against it counted no parse errors in 30 bots' 100 seconds. The game object query response was missing its
  fifth name, which the client needs.
- The rest of phases 13 to 16 was fanned out to seven agents, each on its own branch, merged as each finishes.

### Phase 13: melee, ranged and procs (2026-10-17)

**The hit table** reads the derived stats, as Thistle Tea's `attack_table.ex`:
- Weapon skill against defense, the dual wielder's 19% more misses on white swings, avoidance from behind, sitting
  players always crit, and creatures below level 10 avoid less.
- Glancing and crushing blows. Blocks take the block value off. Creatures' no-parry, no-block and always-crush flags.
- Ranged attacks only miss or crit.
- Damage done and taken by school, flat and by percent, and against creature types; armor less penetration; the
  crit damage bonus. `SMSG_ATTACKERSTATEUPDATE` is now mangos-zero's full layout, absorbs included.

**Fighting:**
- Two hands with their own swing timers, and parry haste.
- Extra attacks (Windfury, Sword Specialization; Reckoning banks up to four), creatures' daze from behind, disarm.
- The reactive states for 4 seconds, as Thistle Tea has them: Revenge, Riposte, Counterattack, Execute, and a
  warrior's Overpower.
- Rage only from white blows and damage taken. Weapon and defense skills rise by Thistle Tea's chances.
- Threat: its auras, Fade, taunts, vmangos's 110% and 130% overtake, and healing's threat.
- Damage breaks the auras damage breaks.
- Next-swing spells (Heroic Strike, Cleave, Maul, Raptor Strike) take the main hand's swing.
- Auto Shot and Shoot (`ranged.rs`): ammo, wands' schools and crits, and moving's delays and cancels.

**Procs** (`procs.rs`):
- Proc-trigger and proc-damage auras, by `procFlags` or `spell_proc_event`, with charges and procs-from-procs rules.
- Items' chance-on-hit spells and weapon enchantments' procs (Crusader, Fiery, poisons, Windfury).
- Lightning Shield, damage shields, target triggers, kill procs, and the scripted procs (Seal of Righteousness,
  Nightfall, Eye for an Eye).

### Phase 13: pets, guardians, totems and charm (2026-10-17)

`map/pets/` (summon, control, AI, hunter, totems) and `data/pets.rs`.

- **A pet** is a creature with an owner: the owner's faction, a pet number and name. A hunter's pet also has XP,
  focus, happiness, loyalty and training points. Its bar goes out in `SMSG_PET_SPELLS`.
- **Its behaviour:**
  - it follows (and jumps to its master past 60 yards), stays, and attacks on command or by its stance, defensive and
    aggressive picking their fights;
  - it casts what's set to autocast, and an imp keeps its distance to cast;
  - it goes while its master is mounted, dead or flying and comes back after, and is kept across a logout and a map
    change.
- **The opcodes:** the 11 pet opcodes and the 5 stable ones. Stable masters by their NPC flag and gossip, and
  untraining by gossip.
- **Hunters:**
  - taming through its channel, with Thistle Tea's checks;
  - Call, Dismiss and Revive Pet;
  - feeding by food type and level;
  - pet XP and levels, happiness and loyalty;
  - spellbooks by the pet's skill lines;
  - training as Thistle Tea checks it.
- **Warlocks:** demons with their create and Grimoire spells at the right ranks, and generated names. Enslave Demon
  charms, and the demon turns on its warlock when it's let go. Mind Control and Eye of Kilrogg possess: far sight,
  the client in control, the unit moving with it.
- **Totems:** one of each element, a new one replacing the old with its auras. A totem holds its party aura on the
  group members in range.
- **Guardians** fight for the spell's duration. **Companions** follow, and casting the same one again sends it away.

### Phase 14 (2026-10-17)

The battlegrounds share one layer, `map/battleground/{mod,wsg,ab,av}.rs`:
- vmangos's battleground events gate the spawns;
- the countdown, doors and ghost gates;
- 30-second spirit guide waves;
- each battleground's scoreboard columns, bonus honor, win and lose spells, and the close after 120 seconds.

- **Arathi Basin:** five bases assaulted or claimed by their banners, taken after 60 seconds or defended at once.
  - vmangos's resource rates to 2,000, its world states and announcements, graveyards at the bases held.
  - Honor every 330 resources and on the win, the four- and five-base spells, the near-victory warning, and the speed,
    restoration and berserk pickups.
- **Alterac Valley:** 15 graveyards and towers, taken after 5 minutes; a burnt tower stays burnt, and the defenders
  follow their events.
  - The mines are won by killing their keeper and reclaimed after 20 minutes.
  - Captains, commanders, explosives experts and lieutenants. The generals end it.
  - Honor for objectives and for surviving, by the match's length. The captains buff their side now and then.
- **Honor** (`game/honor.rs`, kept as records):
  - honorable kills, a tenth less for each repeat of the same victim in a day, as Thistle Tea has it;
  - credit shared by the last 60 seconds' player damage, and by group;
  - dishonorable kills of civilians; racial leaders count;
  - the weekly ranking by standing, with decay and level caps;
  - the honor fields, rank titles and highest rank, `MSG_INSPECT_HONOR_STATS`, `SMSG_PVP_CREDIT` with real values, and
    Honorless Target on arrival;
  - the week turns on a configured day (Wednesday); `.honor week` and `.honor points`.
- **The opcodes:** battlefield list, join and status, the area spirit healer's query and queue, flag carriers'
  positions, and `/pvp`. A battleground portal shows its list.
- **The PvP flag:** `/pvp`, enemy capitals, and on a PvP realm enemy and contested land, lasting 5 minutes after.
- **World PvP** (`map/outdoor_pvp.rs`):
  - the four Plaguelands towers, with capture meters, world states, defense messages, credit and the buffs for towers
    held;
  - Silithus's silithyst, carried to camp: 200 give the Cenarion Favor, and a carrier who falls drops it.

### Phase 13: reputation, rest and the remaining opcodes (2026-10-17)

- **Raw packets:** wow_messages reads a 16-bit faction where 1.12 sends a 32-bit index in the three faction opcodes,
  so a real client's were dropped. The connection now hands them on raw.
- **Reputation:**
  - standings and gains from kills (with the group) and quests, with the level rates and spillover;
  - forced reactions, at war, inactive and watched;
  - a faction shown when targeted;
  - a tenth off at vendors from honored, and items behind reputation hidden;
  - at war makes a faction's units attackable; hated or hostile ones attack.
- **Rested XP:** inns and friendly capitals, accrued online and off (a quarter outside rest), doubling kill XP from
  the pool. Inns give an instant logout.
- **Quests:** sharing with its results, escort confirmations and swapping the log. Template mail on quests. Taking a
  quest "from a player" now needs it shared.
- **GM tickets**, kept as records, with `.ticket`.
- **Items:** opening (lockboxes and clams with their loot), reading and page text, gift wrapping, the bank's auto
  moves, and equipping to a slot.
- **Express flights**, talent resets at the rising price, unlearning a skill, swapping raid subgroups, meeting stones
  (matched by class role), the LFG stub, soulstones and Reincarnation, summons, far sight, the mount animation,
  cancelling growth auras, and the speed, knockback and time-skip acknowledgements.
- **Player systems:**
  - resurrection sickness at the spirit healer;
  - ghost speed by race (a night elf's wisp is faster);
  - breath, fatigue, lava and slime with their mirror timers;
  - exploration XP, which was missing;
  - deleting a character cleans up their guild, charters, mail, auctions and friends (a guild leader with members is
    refused).

### Phase 15 (2026-10-17)

**Group loot** (`loot.rs`), as Thistle Tea's:
- Free for all, round robin (only the member whose turn it is may open), master loot (items at the threshold wait
  for the master), group loot and need before greed (need only for those who can use the item).
- Rolls start at the death and last 60 seconds. Need beats greed, the winner gets the item in their bags, and if
  everyone passes it's free to take.
- Quest items drop for each member who needs them, each a copy of their own. Gold is split among the members within
  74 yards.
- The loot window says its kind and each item's random property. The roll and money packets are encoded by hand, as
  mangos-zero lays them out.

**Professions** (`professions.rs`):
- Herbalism, mining and lockpicking by the Lock DBC, with Thistle Tea's chances of success and skill-ups.
- Skinning (a corpse with nothing left on it is skinnable at once), pick pocket (once a life, from stealth),
  disenchanting.
- Fishing with a bobber, bites, the zone's skill and fishing pools.
- Crafting needs its spell focus and tools, and its skill rises by the recipe's colour.
- Nodes and chests despawn and come back on their spawn's timer; veins may give again.

**Enchantments** (`enchant.rs`):
- Permanent and temporary slots, kept in the item's stored data. Their stats, resistances and weapon damage reach
  the derived stats, and their equip spells become gear auras.
- Item set bonuses by the count worn. Random properties rolled from `item_enchantment_template`.
- Temporary enchantments run down and expire, with the client told the time.
- Weapon procs: an enchantment's combat spell by chance or procs a minute, poisons' charges spent until they're
  gone, and items' chance-on-hit spells.

**Left for later:**
- Loot conditions (`condition_id`), once there is a conditions evaluator.
- Item charges and cooldowns on use, conjured and timed items.
- The bobber lands at the caster's height, with no check for water or line of sight.
- Skill bonuses from enchantments.

## Where the plan leaves the gap

Each phase's "Left for later" lists what remains. The largest:
- Arathi Basin's and Alterac Valley's rules.
- Pets, procs and auto-repeat spells.
- Group loot rules.
- Professions' skill-ups.
- Honor ranks.
- Instance and boss scripts.
- Instance progress kept across a restart.
