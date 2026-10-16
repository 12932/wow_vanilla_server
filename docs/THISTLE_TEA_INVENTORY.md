# Thistle Tea: an inventory

What Thistle Tea (0426f0d6) does, system by system, read from its code for [ALIGNMENT_PLAN.md](ALIGNMENT_PLAN.md) on
2026-10-16. Paths are relative to its repository. Nothing was run for this; the CPU explanations in section 4 are
read from the code, not profiled.

**Size:**
- `lib/` has 1,879 `.ex` files, about 180k lines.
- `lib/game/inbound/` has 275 client-message modules.
- `dispatch.ex` maps 269 named opcodes plus 22 movement opcodes. All 22 go to one module, `Message.MsgMove`.
- `docs/` has 329 files; 319 of them are `*-playtest.md`.
- `test/` has 1,264 `*_test.exs` files.

Design rules are in `AGENTS.md` (`CLAUDE.md` is a symlink to it): a pure core with an effectful boundary, enforced by the `boundary` compiler; effects returned as data; behaviour trees for all AI and action logic; and no disk persistence on purpose.

---

## 1. Architecture

### Process model (everything is a process except the map)

**Connection.** One ThousandIsland handler process per TCP socket.
- World: `lib/game/network/server.ex`, port 8085. Auth: `lib/auth/auth_server.ex`, port 3724.
- It frames packets and decrypts headers (`network/connection.ex`, `connection/crypto.ex`).
- It also encodes outbound packets in `network/send.ex`: zlib compression, header encryption, and one `Socket.send` per packet.
- After login it holds no gameplay state (`network/connection_state.ex`).

**Player.** One GenServer per logged-in character: `world/entity/player.ex` (2,526 lines) plus about 150 helper modules in `world/entity/player/*.ex`.
- Runs under `DynamicSupervisor PlayerSupervisor` with `restart: :temporary`.
- Registered by guid in `World.Entity.Registry` (an Elixir `Registry`).
- The connection forwards every client message with a synchronous `GenServer.call` (`Player.handle_message/2`, `player.ex:207`).

**Creature.** One GenServer per spawned creature: `world/entity/mob.ex` (2,497 lines) plus `world/entity/mob/*`.
- Runs under `EntitySupervisor`, a PartitionSupervisor of 8 DynamicSuperviors (`world/entity/entity_supervisor.ex`).
- Every spawn also has a SpawnPool owner GenServer (`world/system/spawn_pool.ex`, also 8 partitions). That is about 2 processes per creature; the bench saw 1,384 processes for 385 mobs, 66 GOs and 12 players.

**Other entity processes.** Every game object (`world/entity/game_object.ex`), corpse (`entity/corpse.ex`), dynamic object (`entity/dynamic_object.ex`) and transport (`entity/transport.ex`) is also its own process. Transports tick every 50 ms.

**No map, grid or cell processes.** Cells are only ETS buckets and process-group keys.
- `world/system/cell_activator.ex` is a singleton that loads and unloads cells.
- It allows 4 concurrent loads, sweeps every 10 s, unloads a cell after a 90 s grace period, and tears down an empty open world after 300 s.

**Singleton system GenServers**, all started in `lib/application.ex`:
- Party (which also runs Meeting Stones), Guild, Petition, ChatChannels, Duel, Trade, PostOffice (mail), Auction, VendorStock, Honor.
- Instance, Battleground (plus one `Match` process per BG copy under `system/battleground/supervisor.ex`), OutdoorPvp, Weather, GameEvent, ScriptedEvent.
- CreatureGroups, CombatLeashes, LocalDefense, GuardPosts, AreaEffects, SpellMagnets, SingleTargetAuras, GmTickets, and three invasion systems.
- There are also a Phoenix endpoint (LiveView map, LiveDashboard, `POST /register`) and Telemetry.

**Supervision.** Top level is `one_for_one` (`lib/application.ex`). Loaders init their ETS tables before the supervisor starts. After it starts, most catalogs are bulk-loaded (`load_all`), transports are started and DevSeed runs; most of this is skipped in test.

### World partitioning
- A world copy is `Core.WorldRef{map_id, instance_id}` (`core/world_ref.ex`). The open world uses `instance_id: nil`; dungeon and BG copies get ids.
- `world/spatial_hash.ex` defines the ETS tables:
  - `:players`, `:mobs`, `:game_objects`, `:corpses`, `:dynamic_objects`: duplicate_bag tables mapping cell to guid.
  - `:entities`: guid to `{world, x, y, z}`.
  - `:entity_movement`: spline and client-motion projections, used for dead reckoning.
- Cells are 125 yd (`core/spatial_grid.ex`), keyed `{WorldRef, cx, cy}`.
- Positions are computed lazily from projections (`world/position.ex`).

### Visibility, proximity and presence

**Visibility** (`world/visibility.ex`)
- Uses the `group` hex library (`{Group, name: Groups}`; join and leave are synchronous calls to shard processes).
- Every entity joins key `"cell/<map>/<inst>/<cx>/<cy>"`.
- Each player monitors every cell within `@range 250`. `SpatialGrid.cells_in_range` turns that into ±3 cells: a 7×7 block, about 875×875 yd.
- On `:joined`/`:left` events the player asks the entity for a create (`Entity.request_update_from` → `{:send_update_to, pid}` → the entity casts its create block back) or sends itself `SMSG_DESTROY_OBJECT`.
- Per-viewer filters: ghost/spirit (`visibility/filter.ex`), stealth detection, owned/grouped, plus GO, tap and quest-giver personalisation.
- State kept per player: `tracked_entities` (MapSet) and `player_guids` / `mob_guids` / `game_object_guids` lists.

**Proximity** (`world/proximity.ex`, `core/combat/proximity.ex`)
- Aggro is announcement-based, not polled. Units join `"proximity/..."` keys.
- A unit announces `{:proximity, ann}` to every unit in the cells that overlap its reach (≥80 yd sight or 75 yd aggro, and a path extent of up to 250 yd). It announces after moving 2 yd, starting a path, spawning, or changing reaction facts.
- Listeners decide for themselves, or schedule a `:proximity_due` contact check.
- Stealthed units also join `"hidden/..."` keys; traps join `"owned/<guid>"`.

**Presence and Metadata** (`world/presence.ex`, `world/metadata.ex`)
- Each owner publishes a large denormalised ETS row per guid (faction, level, alive, aura projections, reputation, and more).
- Other processes read those rows instead of making IPC calls.
- Writes use compare-and-swap via `select_replace`, and are skipped when the row is unchanged.

### Ticks and timers
- **No global world tick.** Each process schedules itself.
- `core/ai/tick.ex` and `tick_plan.ex` collect typed deadlines (behaviour status, auras, regen, breath, rest, pvp, totem, movement completion, …) and arm only the earliest one. The fallback is 100 ms. A dormant plan arms no timer at all.
- Players: `world/entity/player/tick_scheduler.ex` (`:erlang.start_timer(:player_tick)`). A player ticks only while `Tick.needs_tick?` holds: casting, in combat, has any aura, regenerating, breath, rest, etc.
- Mobs: `Process.send_after({:ai_tick, token})` in `mob.ex:2093`. Idle mobs keep no timer (`test/game/world/entity/mob/dormancy_test.exs`).
- Common cadences:
  - regen: players 2 s, creatures 5 s (`core/power/regen.ex`)
  - EventAI 1 s; creature spell lists 1.2 s; chase 1 s; formation 200 ms
  - totems 200 ms; pets 500 ms; trap game objects poll every 200 ms (`game_object.ex:579`)
  - rest 10 s; duel bounds 1 s; meeting stones 1 s; weather 600 s

### Inbound packet flow
1. `Server.handle_data` buffers bytes and frames packets.
2. `Inbound.Session.decode` (`inbound/session.ex`) calls `Dispatch.to_message`, which runs the module's `from_binary`. Unknown opcodes are logged as "Unimplemented" and dropped.
3. Before a player process exists, the message is handled on the connection state through the `World.ClientInput` protocol (`world/client_input.ex`). Every `use Inbound.ClientMessage` module implements it.
4. After login, it goes by `GenServer.call` to the player, which runs `Input.handle` → `ClientInput.handle` and then `maybe_broadcast_update` (`player.ex:1919`).

### Outbound packet flow
1. Callers use `World.Outbound.send_packet(msg, pid | guid, opts)`. For a guid it does a Registry lookup plus `Process.alive?`, then casts `{:send_packet, …}` to the player process.
2. In the player process, `player/packet_sink.ex`:
   - drops the message if the source isn't tracked;
   - batches updates with `network/update_batcher.ex`: drains up to 100 queued UpdateObject casts, keeps the newest values block per guid, folds values into a create, and stops at the first other send;
   - personalises per viewer (Tap, QuestGivers);
   - stamps movement-ack counters (MovementControl);
   - encodes the packet, then casts `{:write_packet, packet}` to the connection.
3. `Send.send_packet` turns an `SMSG_UPDATE_OBJECT` larger than 128 bytes into `SMSG_COMPRESSED_UPDATE_OBJECT` (zlib level 1), uses a 3-byte size header above 0x7FFF, encrypts the header and calls `Socket.send`.

`World.broadcast_packet` (`world.ex:186`) picks recipients in one of three ways:
- an explicit list;
- `tracking_players`: every player in the 49 cells, with no distance filter (the recipient's PacketSink then drops it if not tracked);
- `range: r`: an exact-distance query.

### Update objects
- Field layout is declared per component with `core/entity/update_mask.ex`. Components are in `core/entity/component/{object,item,container,unit,player,game_object,dynamic_object,corpse}.ex`.
- Fields are tagged by audience: 199 `:private` (self only) and 11 `:special_info` (self and Beast Lore/empathy viewers); untagged fields are public.
- Encoder: `network/update_object.ex`.
- **There is no dirty-field mask.** A values block contains every non-nil field the recipient may see. An owner sets `internal.broadcast_update?` (134 call sites of `Entity.mark_broadcast_update`), and `do_broadcast_update` (`player.ex:2006`) re-broadcasts the full snapshot.

### State storage and persistence
- Everything is ETS or GenServer memory. **Nothing is written to disk.** Accounts are re-seeded at boot (`application.ex:186`: test/test, test1..10, BOT0000..0999).
- Stores: `CharacterStore` (the whole `Core.Entity.Character` struct by id), `ItemStore` (item instances by guid), `SocialStore`, `MailStore`, `AuctionStore`, `HonorStore`, `AccountDataStore`, `Account`, `SessionKey`, `Metadata`, `ServerVariables`, `VendorStockStore`, `ChaseWatch`, `Party`, `InstanceData`, `Transports`, and about 60 loader caches. All `:ets.new` sites are under `lib/game/world/**`.
- "Saving" means calling `CharacterStore.put/1`. It happens at logout (`player/state.ex:163`) and at many transaction points.

### Game data sources
- `db/vmangos.sqlite` through Ecto (`ThistleTea.DB.Mangos.Repo`, 124 schemas in `lib/db/mangos`).
- `db/dbc.sqlite`, generated from the user's own client (`ThistleTea.DB.DBC`, 39 schemas).
- Loaders (`lib/game/world/loader/*`) cache data in ETS, mostly at boot. The exception: creature and gameobject spawn rows are queried live from sqlite each time a cell activates (`loader/mob.ex`, `loader/mob/batch.ex`).
- Navmesh: the namigator NIF (`lib/native/namigator.ex`, `c_src/namigator_ex.cpp`, built with Fine).
  - 36 maps are loaded at boot into `:persistent_term`.
  - **Every NIF function is `ERL_NIF_DIRTY_JOB_CPU_BOUND`, and every query is preceded by a separate `load_adt_at` NIF call** (`world/pathfinding.ex:251`, which has a TODO).
  - Tiles are never unloaded. There is no path cache, and pathfinding runs synchronously inside the entity process.
- Liquid and terrain data: `world/terrain.ex` reads `maps/terrain`.

---

## 2. System-by-system inventory

**General notes:**
- There are only 6 TODOs in all of `lib/`. Gaps are tracked in `core/spell/support_matrix.ex`, `{:unsupported, id}` markers, coverage docs (`docs/event-ai-coverage.md`, `quest-scripting-coverage.md`, `condition-coverage.md`) and the playtest docs.
- Almost every feature has a `docs/<feature>-playtest.md` with vmangos references and a "Native acceptance" section checked with a real 5875 client.

**Login, auth, realm**
- Implemented: SRP6 logon and reconnect proofs and the realm list (`lib/auth/{auth_server,srp,account,session_key}.ex`); SMSG_AUTH_CHALLENGE → CMSG_AUTH_SESSION → SMSG_AUTH_RESPONSE; vanilla header crypto.
- There is a single hard-coded realm (`core/realm.ex`, type from config, default 8 = PvP rules) with character count 0.
- Missing: build and CRC checks, bans, queue, `SMSG_ADDON_INFO`, MOTD, duplicate-session kick, and any world-side reconnect or linger (a disconnect leaves the world immediately).
- **Bug:** `network/connection.ex:20` has `seed: :crypto.strong_rand_bytes(4)` as a defstruct default. That is evaluated at compile time, so every connection gets the same auth seed.
- Opcodes: CMSG_AUTH_SESSION, CMSG_PING.

**Character create, delete, enum, login, logout**
- Files: `inbound/cmsg_char_{create,delete,enum}.ex`, `world/loader/character.ex`, `player/characters.ex`.
- Create data comes from `playercreateinfo` (position), `_spell`, `_action`, `_item`, ChrRaces (display, faction, taxi mask, cinematic), `player_levelstats` and `player_classlevelstats`.
- Create checks only duplicate names and the 10-per-account limit. There is no name validation, no faction rule, and **an invalid race/class combination crashes the connection**.
- Delete releases guild, party, mail, items and social state, and refuses for guild leaders.
- Login sequence (`player/login.ex:145`):
  1. SMSG_LOGIN_VERIFY_WORLD
  2. instance ownership, init world states, corpse delay, account-data MD5, rest, bindpoint, tutorial flags
  3. SMSG_INITIAL_SPELLS, action buttons, proficiency, factions, forced reactions, time speed
  4. cinematic (first login only), owned item creates, the player's own create
  5. friend list, guild MOTD, group list
- The player becomes `ready` on CMSG_SET_ACTIVE_MOVER.
- Logout (`player/logout.ex`): 20 s countdown that sits and roots the player; instant in an inn or on a taxi; refused in combat. Rested XP follows vmangos (`core/player/rest.ex`).

**Movement**
- Path: the 22 MSG_MOVE_* opcodes → `player/movement.ex`, which parses `MovementBlock`, then:
  - Transports reconcile, falling, breath, fatigue, lava, liquid spells;
  - aura, cast and auto-repeat interrupts;
  - Presence relocate (750 ms projection), ChaseWatch, Proximity sync, exploration, visibility refresh;
  - relays the raw payload unchanged to `player_guids`.
- Server-initiated movement changes are acked by counter (`player/movement_control.ex`): root/unroot, run, run-back, swim and swim-back speed, water walk, feather fall, knockback, teleport, worldport. Wrong or missing acks are ignored silently. Walk speed, turn rate and hover acks are not implemented.
- **No anti-cheat at all**: no speed, teleport, fly or climb checks, no timestamp checks. The only validation is transport-bounds checking (`world/transports.ex:138`).
- Fall damage uses the vmangos formula (`core/movement/falling.ex`), based on client fall time. Breath, ocean fatigue, lava and slime spells live in `core/environment/*`.
- Teleports: SMSG_TRANSFER_PENDING/NEW_WORLD → MSG_MOVE_WORLDPORT_ACK, and same-map MSG_MOVE_TELEPORT_ACK (`player.ex:910`, `player/travel.ex`).
- Area triggers (`player/area_triggers.ex`, 5 yd tolerance): BG, outdoor PvP, quest exploration, scripted triggers, taverns, `areatrigger_teleport` with level and condition checks.
- Zone resolution is server-side from terrain (`player/zone.ex`). Exploration XP is implemented.

**Transports** (boats, zeppelins, elevators)
- Routes: `loader/transport.ex` (vmangos `transports` + TaxiPathNode; type-11 animations).
- Pose math: `core/travel/transport.ex` (674 lines).
- One process per transport with a 50 ms tick, all started at boot (`Transports.start_all`).
- Passengers get pose snapshots and recompute their world position (`player.ex:1078`). Map-crossing legs worldport the passenger (`player.ex:2450`).
- No CMSG_MOVE_CHNG_TRANSPORT handler; NPC crew is unclear.

**Update fields and objects**
- Covered in §1. Also: per-recipient field audience, one leading out-of-range block, and item objects sent only to their owner in a fixed order (quest progress, destroy, create, item values, player values; see `player/inventory_update.ex`).
- No dirty masks, and no party-member field flags.

**Stats, levels, XP, talents, skills**
- Derived-stat recompute: `core/stats.ex` and `core/stats/*` (base inputs + equipment bonuses + auras).
- XP follows vmangos float math (`core/player/experience.ex`): elite, dungeon, group rates 1.166/1.3/1.4, 74 yd range, gray levels, quest XP decay, rested XP.
- Level-up sends SMSG_LEVELUP_INFO; the rage/energy deltas in it are always 0.
- Talents: `core/player/talents.ex` (ported from vmangos LearnTalent); reset with escalating cost via MSG_TALENT_WIPE_CONFIRM.
- Skills: `core/skills.ex`, 128 slots.

**Combat** (`core/combat.ex`, `core/combat/*`: 44 files, 5.3k lines; swing loop in `core/ai/bt/combat.ex`, shared by players and mobs)
- Hit table: single-roll port of vmangos `RollMeleeOutcomeAgainst` (`attack_table.ex`, 608 lines). Covers miss, dodge, parry, glancing, block, crit and crushing; weapon vs defense skill; attacks from behind; sitting targets get crit; creature no-parry/no-block/always-crush flags; ranged can only miss or crit.
- Dual wield (+19% miss on white swings), parry haste, extra attacks (Windfury, Reckoning), daze, disarm, reactive abilities (Overpower, Revenge, Riposte), weapon skill-ups, rage generation and refunds.
- Armor uses the vmangos formula, 75% cap. Spell resist, partial resists, binary spells, penetration.
- Threat (`threat.ex`): vmangos ThreatManager, 110%/130% overtake.
- Auto Shot, wands and ammo (`core/spell/auto_repeat.ex`, `player/ammunition.ex`). Mobs have no ranged auto-attack; they use spell lists.
- Missing server opcodes: ATTACKSWING_NOTSTANDING/DEADTARGET/CANT_ATTACK, SPELLDAMAGESHIELD, PROCRESIST, PARTYKILLLOG, AI_REACTION.
- Tests: `test/game/core/combat`, 53 files, 622 tests.
- Opcodes: CMSG_ATTACKSWING/ATTACKSTOP/SETSHEATHED/SET_AMMO/SET_SELECTION.

**Spells**
- Pipeline: `player/spellcasting.ex` → `core/spell/casting.ex` (1,603 lines).
- Validation (`cast_validation.ex`): range, LOS, facing, reagents, tools, focus, stance, indoor/outdoor, crowd-control states, cooldowns. Also pushback, interrupts with school lockouts, channels, auto-repeat, diminishing returns, reflect, Grounding Totem/magnets.
- Target resolution goes through the `Core.Spell.TargetResolver` port, wired in `config/config.exs`.
- Effects: `core/spell/spell_effect.ex` dispatches to 9 handler modules. 98 effect ids are mapped and 86 effect atoms are routed. Unmapped ids are silent no-ops: 4, 12–15, 20, 21, 26, 32, 37, 47–49, 51, 52, 65, 66, 70, 78, 81, 82, 91, 93, 110, 117 (spirit heal), 120 (teleport graveyard), 122, and 125 upward. `:enchant_held_item` is mapped but has no handler.
- Auras: 172 of 192 non-zero aura ids mapped, handled across about 52 `core/aura/*` modules (6.7k lines). Covers stacking groups, 32-buff/16-debuff capacity, periodic ticks, dispel, procs (DBC flags + `spell_proc_event`) and absorbs.
- Cooldowns: spell, category, item, deferred; GCD is not hasted, on purpose (`core/spell/cooldowns.ex`).
- Power: `core/power/*` (five-second rule, energy 20 per tick, rage decay).
- Data: DBC Spell tables plus vmangos `spell_*` tables (`world/loader/spell.ex`, 1,158 lines). 64-bit family-mask modifiers.
- About 45–60 hand-written class/racial scripts (`core/spell/scripts.ex`, `core/class/*`).
- There is no percentage of working class abilities. The guard is `test/game/core/spell/support_matrix_vmangos_test.exs` (tagged `:dbc_db`, not run in CI): every trainable spell and talent rank must use only known or deferred effect, aura and target ids.
- Tests: about 1,700 across `core/spell`, `core/aura`, `core/power`, `core/class`, `world/spell`.

**Death and resurrection** (`core/death.ex`, `player/{corpses,spirit_healer,resurrection,self_resurrection}.ex`, `world/graveyards.ex`)
- Release spirit → corpse process, ghost auras, reclaim delay escalating 30/60/120 s, nearest graveyard by faction.
- MSG_CORPSE_QUERY, including dungeon entrances. Reclaim within 39 yd at 50% health.
- Spirit healer: resurrection sickness and 25% durability loss.
- Spell resurrection works across maps. Soulstone and Reincarnation via CMSG_SELF_RES. 10% PvE durability loss on death.
- BG area spirit healers resurrect in 30 s waves.
- Gaps: corpses are deleted instead of turning into bones, and ordinary corpses don't decay.

**Creatures**
- Spawning: cells activate lazily (49-cell window per player), with live sqlite spawn queries per cell (§1). Pools follow vmangos pool selection (`spawn_pool/*`).
- Respawn timer runs in the mob process (`mob/respawn.ex`, default 120 s). Corpse decay is 300 s, or 600 s for elites.
- Creature groups, formations and links: `world/system/creature_groups.ex`, `core/creature/*`.
- Behaviour trees: `core/ai/bt.ex` (selector, sequence, condition, action).
  - Mob tree in `bt/mob.ex:89-213`: feign/link → EventAI → CC → OOC casting/assist/flee → aggro → combat (victim, tether/evade, spell lists, melee, chase) → point/follow/formation → waypoints → wander → idle.
  - Separate trees for pet, guardian, totem, mini-pet and critter.
- Aggro radius follows vmangos (`core/combat/aggro.ex`). Call-for-help and assistance, soft and hard leash, unreachable-target evade, ChaseWatch-triggered repaths. Waypoints come from `creature_movement` / `creature_movement_template`.
- EventAI (`core/ai/event_ai.ex`): every event type except 13, 34 and 35. The coverage doc reports 6,220 of 6,256 event rows and 7,250 of 7,409 action rows, and is partly stale (more is now supported).
- Script interpreter (`core/ai/script.ex`, 2.5k lines) handles DB script commands 0–92 except about 9. It reads creature_ai, movement, generic, event, GO, quest, gossip, spell and area-trigger scripts.
- 61 hand-ported C++ creature scripts (`core/ai/creature_script/*`, 11.9k lines): Molten Core bosses, Onyxia, Dragons of Nightmare, BRD/BRS/Strat/Scholo/SM/ZF/DM/Uldaman/Gnomeregan/WC bosses, AV, invasions, escorts. **None for BWL, ZG or AQ.**
- Pathing: NIF `find_path`, LOS, heights, liquid, random point, zone/area. Synchronous, no cache. Splines interpolated linearly (`core/movement.ex`); SMSG_MONSTER_MOVE sent from `event_sink/movement.ex`.
- Opcodes: CMSG_CREATURE_QUERY, CMSG_GAMEOBJECT_QUERY, CMSG_GAMEOBJ_USE.

**Loot**
- Every vmangos loot template is loaded: creature, GO, fishing, pickpocket, skinning, disenchant, item, reference (`loader/loot.ex`). Groups, conditions, per-player quest drops.
- Tapping; free-for-all, round-robin, master looter and group/need-before-greed with 60 s rolls (`core/loot/loot_session.ex`, `loot_roll.ex`, `mob/corpse.ex`); gold split.
- Chests, herbs and veins, fishing, pickpocket, skinning, lockboxes.
- Opcodes: CMSG_LOOT, AUTOSTORE_LOOT_ITEM, LOOT_MONEY, LOOT_RELEASE, LOOT_METHOD, LOOT_ROLL, LOOT_MASTER_GIVE.

**Items, inventory, bank**
- Pure inventory model (`core/inventory.ex`, 1.5k lines): equipment, 4 bags, backpack, 24 bank slots, 6 bank bags, a level-scaled keyring, specialised bags.
- Atomic multi-item changes via `Inventory.Batch → plan → ChangeSet`, committed once in `player/inventory_update.ex`.
- Equip rules: class, race, level, honor rank, proficiency, reputation, dual wield, two-hand. Unique limits, binding, durability and repair, item sets, permanent and temporary enchants, random properties, item use with cooldowns and charges, open/wrap/read, timed and conjured items, ammo.
- Missing: CMSG_CANCEL_TEMP_ENCHANTMENT.
- Opcodes: about 20 CMSG item and bank opcodes (AUTOEQUIP*, SWAP*, SPLIT, DESTROY, BANKER_ACTIVATE, BUY_BANK_SLOT, AUTOBANK/AUTOSTORE*, USE/OPEN/WRAP/READ_ITEM, ITEM_QUERY_SINGLE, ITEM_NAME_QUERY, REPAIR_ITEM).

**Quests**
- All 4,433 `quest_template` rows (`loader/quest.ex`). Prerequisite, exclusive-group and breadcrumb graph (`core/quest/quest_graph.ex`).
- Objectives: kill, item, explore/event, cast, talk, reputation. Timed quests; repeatables; auto-complete quests; party sharing; item-started quests; game-object questgivers; quest reward spells.
- Escort quests (30 ported) and follower quests (7).
- Coverage: start/end script rows 2,351 of 2,355; RequiredCondition 1,661 of 1,776 conditions evaluable. There is no count of fully working quests.
- Files: `player/quests.ex` (1.2k lines), `core/quest/*`.
- Opcodes: all 14 questgiver/questlog opcodes plus CMSG_AREATRIGGER.

**NPC services** (reach check 5 yd plus bounding radii, `player/npc_reach.ex`)
- Gossip: `player/gossip.ex` with conditions, gossip scripts and POIs. Option types 2, 7 and 15 are not loaded.
- Vendors: shared limited stock, buyback, reputation and rank gates, 10% Honored discount.
- Trainers: purchase failures are silent; no SMSG_TRAINER_BUY_FAILED.
- Repair, innkeeper/hearthstone, banker, tabard vendor, petitions.
- Taxi (`player/taxi.ex`): node discovery, multi-hop express routes stitched into one spline, resume after relog.
- Opcodes: GOSSIP_HELLO/SELECT_OPTION, NPC_TEXT_QUERY, LIST_INVENTORY, BUY/BUY_IN_SLOT/SELL/BUYBACK, TRAINER_LIST/BUY_SPELL, UNLEARN_SKILL, BINDER_ACTIVATE, TAXINODE_STATUS_QUERY, TAXIQUERYAVAILABLENODES, ACTIVATETAXI, ACTIVATETAXIEXPRESS, PETITION_*, TABARDVENDOR.

**Reputation and factions** (`core/reputation.ex`, `player/reputation.ex`, `core/combat/hostility.ex`, a port of vmangos GetReactionTo / IsValidAttackTarget)
- Spillover, kill and quest gains, at-war/inactive/watched flags, forced reactions.
- Opcodes: CMSG_SET_FACTION_ATWAR/INACTIVE, SET_WATCHED_FACTION.

**Professions**
- Gathering, skinning, fishing, crafting skill-ups, spell focus, recipe books, enchanting and disenchanting, first aid, engineering gadgets.
- Cooking, alchemy, blacksmithing and the rest have no special code; they use the generic create-item path.

**Chat** (`world/chat.ex`; channels in `world/system/chat_channels.ex`)
- Say and emote within 25 yd, yell within 300 yd, whisper with AFK/DND replies, party, raid, raid leader, raid warning, guild, officer, channel, AFK, DND.
- 13 languages with comprehension and Curse of Tongues.
- Channels: all moderation commands; per-faction; defaults from ChatChannels.dbc. No zone validation.
- **Missing: battleground chat.** Text emotes come from DBC.
- Opcodes: MESSAGECHAT, JOIN/LEAVE_CHANNEL, 16 CHANNEL_* opcodes, TEXT_EMOTE, EMOTE, CHAT_IGNORED.

**Groups and raids** (`core/party.ex`, `player/groups.ex`)
- 5-man parties and 40-man raids with subgroups, assistants, target icons, ready check, loot methods, meeting stones (class-based matchmaking).
- **SMSG_PARTY_MEMBER_STATS is sent unthrottled on every values broadcast** (`system/party/notifier.ex:81`). It carries only 8 fields: no position, auras or pet.
- MSG_LOOKING_FOR_GROUP is a stub.
- Opcodes: 11 CMSG_GROUP_* opcodes, RAID_TARGET_UPDATE, RAID_READY_CHECK, REQUEST_PARTY_MEMBER_STATS, MEETINGSTONE_*.

**Guilds** (`core/guild.ex`, `player/guilds.ex`)
- Ranks and rights, MOTD, notes, charter or petition flow (9 signatures), emblem, guild events.
- CMSG_GUILD_CREATE lets anyone create a guild with no GM check.
- Opcodes: 20 CMSG_GUILD_*, 9 PETITION opcodes, TABARD.

**Friends, ignore, /who** (`core/social.ex`, `core/who.ex`)
- 50 friends and 25 ignores; online/offline status pushes only.
- /who filters for the same team, 49 results max, and scans every character on each query.

**Mail** (`core/mail.ex`, `player/mail.ex`, `system/post_office.ex`)
- Money, COD, one item per mail, 1 h delay for item or money mail, text items, NPC and quest mail.
- **Expiry is never processed**: no auto-return, and attachments are orphaned.
- Opcodes: 9 CMSG_MAIL / SEND_MAIL opcodes, ITEM_TEXT_QUERY, MSG_QUERY_NEXT_MAIL_TIME.

**Auction house** (`core/auction*`, `system/auction.ex`)
- Faction and neutral markets, DBC deposit and cut, 2/8/24 h durations, bids, buyout, cancel, expiry, mail settlement, full search filters.
- Opcodes: 7.

**Trade** (`core/trade.ex`, `system/trade.ex`): 6 slots plus gold plus an enchant/lockpick slot; atomic commit. Opcodes: 10.

**Duels** (`system/duel.ex`, `core/duel*`)
- Started by the duel spell effect, with a flag game object. 3 s countdown; out of bounds at 75 yd triggers a 10 s forfeit; Grovel on defeat.
- Opcodes: CMSG_DUEL_ACCEPTED/CANCELLED.

**PvP flags, honor, outdoor PvP**
- PvP flag (`core/pvp.ex`): 5 min flag, contested flag, enemy and capital territory, FFA areas.
- Honor (`core/honor/*`, `system/honor.ex`): vanilla kill formula, diminishing returns, dishonorable kills, weekly rank calculation, rank fields used as titles.
- Outdoor PvP: Eastern Plaguelands towers and Silithus only.
- Opcodes: TOGGLE_PVP, INSPECT_HONOR_STATS, INSPECT.

**Battlegrounds**
- WSG, AB and AV are fully modelled (`core/battleground/*`; AV is in 10+ sub-modules). Queue per bracket and team, with backfill. One Match process per copy; scoreboard; world states; deserter debuff; spirit-healer waves.
- Missing: no automatic BG raid group, no BG chat.
- Opcodes: BATTLEMASTER_HELLO/JOIN, 5 BATTLEFIELD_*, AREA_SPIRIT_HEALER_*, PVP_LOG_DATA, BATTLEGROUND_PLAYER_POSITIONS.

**Instances** (`system/instance.ex`, `core/instance/*`)
- Per-group copies via WorldRef. Admission: raid requirement, player cap, 5 copies per hour per account. Raid lockouts, 04:00 resets, CMSG_RESET_INSTANCES, REQUEST_RAID_INFO.
- 19 instance scripts (doors, encounter state).
- No adapters for RFC, Stockade, Onyxia's Lair, ZG, BWL, AQ40 or Naxx.

**Pets and stables** (`core/pet/*`: 26 files)
- Hunter pets: tame, happiness, loyalty, XP, training, naming, 2 stable slots.
- Warlock demons, charm and possession, guardians, totems, mini-pets, creature-owned pets.
- Opcodes: 11 CMSG_PET_*, REQUEST_PET_INFO, 5 stable opcodes.

**Weather, game events, time**
- Weather: vmangos seasonal model, a per-zone GenServer.
- Game events: recurrence from the `game_event` table, gated spawns, seasonal quests, Darkmoon Faire, fireworks, three invasion systems.
- Server variables, CMSG_QUERY_TIME, CMSG_PLAYED_TIME, world states (not re-sent on zone change within a map).

**GM and dev commands** (`player/dev_commands.ex`, 1,750 lines)
- Dot-commands in any chat type. **There are no permission levels**: anyone can run them, and no CMSG_GM_* opcode is handled.
- Commands: .additem, .addquest, .learn, .levelup, .tgm, .go xyz, .tele, .modify, .speed, .kill, .respawn, .die/.revive, .weather, .invasion, .bg, .instance, .ticket, and many .debug subcommands.
- GM tickets: in-memory (`system/gm_tickets.ex`, 5 CMSG_GMTICKET_* opcodes). GM surveys and bug reports are only logged.

**Misc**
- Name, creature, GO, item, page, NPC-text and quest queries.
- Account data (zlib-inflated) and tutorials, both stored per account.
- Cinematic camera tracking, inspect, minimap ping, random roll, action bars, helm/cloak toggles.

---

## 3. Testing
- **Volume:**
  - 1,264 `*_test.exs` files and about 8,800 `test "…"` cases: core 610 files / 5,673; world 520 / 2,721; network 85 / 267; inbound 35 / 120; plus auth, db, native, web and the architecture test.
  - Last recorded full run (`docs/server-optimization.md`): `mix test.all` passed 8,859.
- **Kinds:**
  - Pure core unit tests. Time and randomness are injected (`Core.Rolls`, `now`), so they are deterministic.
  - World tests that start real player and mob GenServers on ETS.
  - Inbound decode/handle tests.
  - Byte-exact server message encoder tests (`test/game/network/message/*`, `test/update_object_test.exs`).
  - Loader tests against the real databases.
- **Integration tiers** (excluded by default in `test/test_helper.exs`):
  - `:vmangos_db` (104 files)
  - `:dbc_db` (204 files)
  - `:namigator_maps` (23 files)
  - Tags are mutually exclusive.
- **CI** (`.github/workflows/ci.yml`): `nix flake check`, `mix test`, then generate `vmangos.sqlite` and run `--only vmangos_db`. DBC and map tests never run in CI.
- **Architecture enforcement:** `test/game/architecture/dependency_test.exs` (core never touches World/DB, no clock reads, an ambient-random allowlist), the `boundary` compiler with `--warnings-as-errors`, and `credo --strict`.
- **Behaviour validation:** almost every feature has a `docs/*-playtest.md` with vmangos source references and a "Native acceptance" pass using a real 1.12.1 (build 5875) client. Coverage docs and a `mix condition.coverage` task (`lib/mix/tasks`) quantify how much DB scripting is handled.
- **Benchmarks:** `bench/{hot_paths,population,spatial_grid}.exs`, with results in `bench/results/*.json`, write-ups in `docs/server-optimization.md` and `docs/refactor-hardening.md`. The population harness uses 12–48 players and 80–320 mobs, completion-paced, with a discard sink. **Nothing has been measured at hundreds of real connections.**

---

## 4. Performance design, and why the crowd tests burn cores

**What is already designed for performance:**
- dormant AI with no timers;
- announcement-based aggro instead of per-tick scans;
- Metadata reads instead of IPC, and unchanged Metadata writes are skipped;
- update coalescing in the mailbox;
- zlib above 128 bytes;
- lazy cell activation;
- 8-way partitioned supervisors.

**Likely causes of ~13.5 cores for 200 players in one spot.** Items 1–7 are what the code does; how much CPU each one costs is my inference, because I didn't profile anything.

1. **Visibility is about 875×875 yd per player.**
   - `visibility.ex` `@range 250` becomes ±3 cells of 125 yd (`spatial_grid.ex` `cells_in_range`). `cell_activator.ex` `@visibility_cell_radius 3` uses the same window.
   - vmangos defaults to about 90–100 yd. So every player tracks and receives traffic from far more entities, and in a crowd everyone tracks everyone.
2. **Every change re-sends the full field set, with no dirty mask.**
   - `update_object.ex` `packet_body` → `recipient_fields` → `UpdateMask.values` drops only nil fields.
   - Any `mark_broadcast_update` triggers a full values snapshot to every tracking player, plus party stats (`player.ex:2006`). There are 134 call sites, including regen every 2 s, rest every 10 s, target change (`inbound/cmsg_set_selection.ex`), emote and stand state, attack timers, combat flags, and aura changes. `player/movement.ex` `interrupt_auras` broadcasts a full block whenever movement removes an aura.
   - Arithmetic: 199 visible players × about 2.5 flagged changes per second each is roughly 500 update packets per second per observer, which matches what you saw.
3. **Encoding and compression happen once per recipient, not once per change.**
   - The struct is cast to each recipient process. Each one runs `Visibility.can_see?` (a full `Metadata.get` row copy, a party ETS lookup, and `StealthDetection.target_metadata` recomputed), personalises, and calls `UpdateObject.to_packet`.
   - The connection then compresses it in `Send.deflate`, which does `:zlib.open` + `deflateInit` + deflate + close **for every packet**. Nearly every player values block is over 128 bytes.
4. **Batching is opportunistic.**
   - `UpdateBatcher` merges only updates already sitting in the mailbox, and stops at the first non-update send. Interleaved MSG_MOVE relays break it, so most changes go out as their own packet.
   - Its `receive` loop is a selective receive. It scans past non-matching messages (proximity, group events, timers) on every iteration, so the cost grows with mailbox depth under load.
5. **Each outbound packet costs about 3 process hops and one syscall.**
   - The path is broadcaster → Registry lookup + `Process.alive?` → recipient player process → connection process → `Socket.send`. There is no write coalescing or corking.
   - Each send also bumps 3 shared ETS counter rows (`lib/telemetry/telemetry.ex:75`, `update_counter`). At about 100k packets/s server-wide those rows are contended.
6. **Proximity fan-out is O(N²) in a crowd.**
   - Every unit, players included, announces every 2 yd (`core/combat/proximity.ex` `@movement_step 2.0`; reach ≥80 yd, so up to about 9 cells) to every unit in those cells.
   - Each listener runs `handle_info({:proximity, …})` → `Proximity.hear` + `Visibility.hear` + `QuestGivers.hear` (`player.ex:1115`).
   - Arithmetic: 200 runners at about 3.5 announcements per second × 200 listeners is about 140k messages per second, even though player-to-player announcements are ultimately ignored.
7. **Heavy work for every inbound packet, every heartbeat included.**
   - The connection→player `GenServer.call` is followed by the whole `maybe_broadcast_update` pipeline (`player.ex:1919`): about 12 reconcilers, then `sync_character_metadata` (`player.ex:2025`), which builds a projection of about 35 keys (aura sources, stacks, effects, dispel options, spell threat, reputation, defense snapshot…), writes it with Presence/Metadata CAS, then runs `QuestGivers.sync` and `Proximity.sync`.
   - Movement also does: `Presence.relocate_client`, whose metadata write always changes because of `last_move_at`; visibility refresh and `reveal_nearby`; and environment queries (`query_liquid_surface`, `wmo_liquid`, sometimes `find_heights`). Each of those is two dirty-scheduler NIF calls because of `load_adt_at`.
   - Then the raw MSG_MOVE is relayed to every `player_guid`.

**Capital cities (about 9 cores for 44 players): likely causes.** These are inferences from the code.
- Each player's 49-cell window holds hundreds of NPCs, each with its own process plus a SpawnPool process.
- Patrolling guards and waypoint NPCs keep sending SMSG_MONSTER_MOVE and proximity announcements.
- Guards and civilians have `watch` ranges (`GuardCall.watch_range`). Every player announcement makes each NPC aggressor or watcher in reach run `engages?`/`watches?`, which means two full `Metadata.get` row copies plus Hostility checks.
- **Quest-giver markers (`visibility/quest_givers.ex`).**
  - `sync` runs on every message and tick. `hear` runs on every group or proximity event, and does a full refresh when the watch is "spatial" — when any visible quest condition depends on environment, the fingerprint includes the player's position.
  - A refresh walks every tracked entity, does a `Metadata.query` (a full row copy) for `npc_flags` on each, and computes `Quests.dialog_status`. Cities are full of quest givers.
- Resting players also do a full values broadcast every 10 s (`core/player/rest.ex` `@update_interval`).

**Minor O(n) scans:** `CharacterStore.get_by_name/all`, `/who` (every query), `SocialStore.followers` (a `foldl` over the whole table).

**Design lessons for the Rust plan, read from the code:**
- Keep a per-object dirty mask, and build each update block once per change, shared across recipients. Only fields that differ by audience need per-recipient variants.
- Batch per observer per tick and coalesce socket writes. Reuse a deflate stream or context per connection.
- Use a visibility radius of about 100 yd with an exact distance test, not cell-quantised ±3 cells.
- Rate-limit party stats with a changed-field mask.
- Make interest management a cheap per-cell pass inside a map or grid update, instead of message fan-out to every unit per 2 yd.
