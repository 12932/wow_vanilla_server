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
