# Bug hunt: the map actor's core (`map_core`)

Scope: `src/game/map/` mod.rs, visibility.rs, observers.rs, grid.rs, views.rs, derive.rs, handlers.rs, movement.rs,
moves.rs, move_guard.rs, acks.rs, objects.rs, nav.rs, tiles.rs, world.rs (zones, triggers), plus `src/game/fields.rs`
and `src/game/update.rs`. Read at master `0c5811f4`. Already-known items (review 2's H1/M1, review 5's world 1-4,
which are fixed or left on purpose) are not repeated, except where a new angle is noted.

## CMSG_ZONEUPDATE is trusted: a client picks its own zone (graveyards, city rest, channels)
- kind: bug
- severity: high
- where: src/game/map/handlers.rs:310-331 (`handle_packet`, the `CMSG_ZONEUPDATE` arm); its readers: src/game/map/death.rs:308 and :902 (`repop_at_graveyard`, the graveyard `pick(gy.zone == zone)`), src/game/map/rest.rs:178 (`friendly_capital(.., p.zone)`), `social::channels::zone_changed`, `world_states::send_init`, outdoor_pvp.rs:433
- what: the handler writes `m.area` straight into `p.zone` (and `online.zone`), joins that zone's channels and sends its world states. The server's own zone (`world::update_zone`, from the navmesh) only runs when the player crosses a 50-yard cell in a visibility refresh, so a client that stands still, or sends the packet again after each crossing, keeps any zone it likes. mangos-classic's `HandleZoneUpdateOpcode` only sets `SetDelayedZoneUpdate`, and `Player::Update` checks the zone against `GetZoneAndAreaId` ("Client sent invalid zoneId") and uses the server's. Three ways to use it: (1) die anywhere on a continent, send ZONEUPDATE with Stormwind's zone, then `CMSG_REPOP_REQUEST`, and `repop_at_graveyard` picks a graveyard in that zone, a free trip across the continent; (2) claim a friendly capital's zone and `rest::update` starts City rest anywhere (rested XP); (3) join any city's Trade or LocalDefense channel, get another zone's world states, or get the Plaguelands tower buff away from the Plaguelands.
- fix: in the ZONEUPDATE arm, call `world::update_zone(map, guid)` and only fall back to the client's value when `zone_and_area` has no answer (no navmesh or terrain, as `misc::handle_all` already does for exploration). Log a mismatch at `debug`, as mangos does.
- fix size: small
- confidence: confirmed

## A knockback gives a 10-second window with no flag check, no root check and the launch's speed
- kind: bug
- severity: medium
- where: src/game/map/move_guard.rs:37 (`LAUNCH_LIMIT`), :74 (`held_back_with`), :106 (`stripped`), :176-186 (`too_fast_with`), :203 (`launched`), :217 (`taken_with`); src/game/map/acks.rs:209-228 (`knock_back`)
- what: once a `CMSG_MOVE_KNOCK_BACK_ACK` settles, `guard.launch = (xy_speed, now)` and the bank is refilled. `taken_with` only clears the launch on a packet with neither JUMPING nor FALLINGFAR, so a client that keeps setting JUMPING stays "launched" for the whole `LAUNCH_LIMIT` (10 s). Meanwhile: `stripped` returns the info untouched (CAN_FLY, FLYING, LEVITATING, water walking and hover all accepted), `held_back` ignores roots and stuns, and the speed check allows `xy_speed * 1.25`, with a bank of `xy_speed * 1.25 * 2 + 3` available at once. A real knockback's flight lasts about 1 s. With a 50 yd/s launch, a cheater gets 128 yards at once and then 62 yd/s in any direction, flying, for 10 s. Mob and boss knockbacks (Wing Buffet, Knock Away, Ragnaros) are easy to come by, so this skips trash, crosses walls and escapes roots.
- fix: end the launch at its ballistic time (from the ordered impulse's `z_speed`: about `2*vz/g` plus a fall margin, capped at about 3 s), not after 10 s. Keep stripping CAN_FLY, FLYING and LEVITATING during a launch, since no knockback grants them. Don't refill the bank to the launch's full cap: allow only `xy_speed * elapsed`.
- fix size: small
- confidence: confirmed

## A client that never sends MSG_MOVE_WORLDPORT_ACK acts on the new map, unseen
- kind: bug
- severity: medium
- where: src/game/map/mod.rs:1700-1727 (`arrive`, `ArrivalKind::Transfer`); src/game/map/handlers.rs:19-392 (`handle_packet`, no gate); src/game/map/movement.rs:630 (`worldport_ack`); visibility.rs:250/573/600/651 (those awaiting are skipped both ways); ai.rs:1108 (creatures don't aggro them)
- what: after a transfer, `awaiting_worldport` stays true until the client acks, with no timeout. Meanwhile the player is on the map at the destination, but no client is told of them and creatures don't aggro them. Every non-movement packet is still handled: `CMSG_CAST_SPELL`, `CMSG_ATTACKSWING`, `CMSG_GAMEOBJ_USE`, loot, vendors, quests. A modified client that withholds the ack after a portal, battleground entry or summons is invisible and untargetable by other players, and immune to aggro, but can still cast on targets whose GUIDs it knows (creature GUIDs are predictable: `0xF130 | entry | spawn`). In mangos the player isn't added to the map until the ack, so in-world opcodes don't reach it.
- fix: while `awaiting_worldport`, drop every packet but the worldport ack, pings and time queries, logout, and chat. Also kick (save and send to the character screen) a client that hasn't acked within about 60 s.
- fix size: small
- confidence: likely (the code path is confirmed; real clients always ack)

## Other players never see a player's emote state (/dance, Cannibalize) or aura state (Execute, Hammer of Wrath)
- kind: bug
- severity: medium
- where: src/game/fields.rs:667-695 (`player_public_mask`); set at src/game/map/chat.rs:268-276 (`play_animation`: a state emote sets `UNIT_NPC_EMOTESTATE` and sends no `SMSG_EMOTE`), spells/class_scripts.rs:842 (Cannibalize), combat.rs:2618 (`sync_aura_state`: `AURA_STATE_HEALTHLESS_20`), derive.rs:519 (Judgement)
- what: the public player mask leaves out `UNIT_FIELD_AURASTATE` (125), `UNIT_CREATED_BY_SPELL`, `UNIT_NPC_FLAGS` and `UNIT_NPC_EMOTESTATE` (146-148): it sets `UNIT_DYNAMIC_FLAGS..=UNIT_MOD_CAST_SPEED` and stops there. mangos-classic's `UpdateFields.cpp`, the authority, flags all four `UF_FLAG_PUBLIC`, and Thistle Tea keeps dancing for observers ("retains dancing for observers", emotes_test.exs). The mask follows mangos-zero's `updateVisualBits`, which reviews 2 and 5 checked against. Result: a dancing player looks like they're standing to everyone else, and so does a Cannibalizing one. Execute and Hammer of Wrath (`target_aura_state` 2) against a player target under 20%: the server sets the bit, but the attacker's client never sees it, so the button stays unlit, and the 1.12 client checks target aura state before it sends the cast.
- fix: add `UNIT_FIELD_AURASTATE` and `UNIT_CREATED_BY_SPELL..=UNIT_NPC_EMOTESTATE` to `player_public_mask`. Add a harness test: a second client sees a dancer's `UNIT_NPC_EMOTESTATE`, and a target's `AURASTATE` bit 2. Run the client check (`tools/client_check/`).
- fix size: trivial
- confidence: confirmed for emote states; likely for Execute and Hammer of Wrath (depends on the client's own pre-cast check)

## A possessed unit's movement skips the speed check, roots and flag stripping: a Mind Controller can drag a victim anywhere
- kind: bug
- severity: medium
- where: src/game/map/pets/possess.rs:255-320 (`moved`)
- what: review 5's world 2 added `finite()` here. The rest of the controller's own checks are still missing: no `too_fast`, no `held_back` (a stunned or rooted victim can still be moved), no `stripped`. A priest who Mind Controls an enemy player can send a single heartbeat that puts the victim up to 20,000 yards away, or under the terrain at sea, or on an unreachable roof, with any flags. `v.move_check` is then set to that spot, so the victim's own client is checked from there when the charm ends. A Mind Controlled creature (a boss's add, an Eye of the Beast pet) can be moved anywhere too.
- fix: run the victim's `move_guard::held_back`, `stripped` and `too_fast` (with the victim's guard and speeds) before taking the position, and `teleport_within` back on failure, as `moved_self_as` does. The Eye of Kilrogg keeps its own speed from its fields.
- fix size: small
- confidence: confirmed

## A creature despawned while marked for visibility is introduced again, and stays stale past its respawn
- kind: bug
- severity: medium
- where: src/game/map/ai.rs:632-645 (`despawn_from_view`), :676-686 (`respawn`); src/game/map/visibility.rs:469 (`prepare`), :586 (`decide_whole`); callers: ai.rs:847 (`quiet_death_for`), scripts.rs:1144 (a script's despawn), instance_adapters.rs:278, instance_script.rs:721
- what: `despawn_from_view` takes the creature out of the grid and out of every client's `known`, but leaves it in `visibility_dirty`. `lie_dead` does remove it. If the creature was marked (a script's `update_entry` or `place` in the same run, scripts.rs:1055/1318, a refresh left over by the visibility budget, a summon despawned in the step it came), the update's refresh still finds it: `position_of` answers, its grid entry is gone so it is taken whole, and `decide_whole` sends `Act::Introduce` to every nearby player. The client is sent a create for a creature the server thinks is gone. Worse, `respawn` then removes the old creature straight from the slab (it is `despawned`, so `despawn_from_view` is skipped), leaving those players' `known` holding its GUID. The respawned creature has the same GUID, so `apply` skips the introduce (`known.contains`), and those players keep the stale corpse until they walk away.
- fix: in `despawn_from_view`, `map.visibility_dirty.remove(&g)`. Also make `prepare` return `None` for an object with no grid entry, so nothing out of the grid is ever introduced.
- fix size: trivial
- confidence: likely (the path is confirmed by reading; which scripts mark and then despawn in one run wasn't enumerated)

## Forgets are quadratic: one hearth within a continent costs millions of comparisons on the map's thread
- kind: perf
- severity: medium
- where: src/game/map/mod.rs:2188-2208 (`settle_forgets`); src/game/update.rs:518 (`creates_of`), :526 (`Batch::forget`), :534 (`drop_for`)
- what: for each distinct forgotten GUID, `settle_forgets` counts its repeats by scanning the whole list (`guids.iter().filter(..).count()`) and checks `seen` linearly. Then it calls `creates_of` and `drop_for`, which each scan every span in the batch, and `Batch::forget`, which scans the forget list. A Hearthstone from Stormwind to Ironforge (both map 0, so `teleport_within`) forgets about 1,500 objects and creates about 1,500 in the same update: about 1,500 × (1,500 + 3,000 + 3,000 + 750) ≈ 12M comparisons, several milliseconds for one player. A raid hearthing at once is worse. A GM `.tele` or a mage's Teleport within a continent does the same.
- fix: build `AHashMap<Guid, u32>` counts of the forgets once, give `Batch` a `HashMap<Guid, SmallVec<span idx>>` (or a creates-count map), and make `forget` a set (or dedupe once in `packets()`).
- fix size: small
- confidence: confirmed (by reading; not timed)

## Tiles ahead: corner tiles are missed, and the reach is the continents' sight on every map
- kind: robustness
- severity: low
- where: src/game/map/tiles.rs:93-113 (`tiles_around`), :159 and :190 (`reach = visibility_yards + 20`)
- what: (1) nine sample points (the centre and eight on the circle) don't find every 533-yard tile a 120-yard disc touches. A centre 100 yards west and 10 yards south of a tile corner has the corner within 101 yards, but no sample lands in the diagonal tile ((d,d) falls 15 yards short in x). Those tiles are never asked for ahead, and `keep_near_players` doesn't keep them, so they unload while a player stands about 100 yards away. (2) `reach` always uses `[visibility] continents_yards` (100). Instances see 170, battlegrounds 400 and fliers 400 (`visibility::sight`), so creatures woken at 120-400 yards walk with no navmesh (straight paths, unknown ground) until their own query brings the tile in.
- fix: list every tile whose square intersects the disc (`tile_of` at the corners of the disc's bounding box, then filter by nearest-point distance). Use `visibility::sight(map, p) + AHEAD_OF_PLAYERS` (or `radius(map)` in `keep_near_players`).
- fix size: small
- confidence: confirmed (geometry by hand)

## Dead and looser duplicate arms in handlers.rs
- kind: improvement
- severity: low
- where: src/game/map/handlers.rs:338-366 (`CMSG_TUTORIAL_FLAG`, `_CLEAR`, `_RESET`, `CMSG_UPDATE_ACCOUNT_DATA`), :366-374 (`CMSG_REQUEST_ACCOUNT_DATA`, `CMSG_COMPLETE_CINEMATIC` in the ignore list)
- what: `misc::handle_all` (account_data.rs:237-282, misc.rs:161) handles all of these first, so these arms never run. The `CMSG_UPDATE_ACCOUNT_DATA` arm is the dangerous one: it has no `kind < TYPES` or `MAX_SIZE` check, writes to the store directly, and skips `account_data::update`. One reorder of the chain would open an unbounded write from any client.
- fix: delete the arms (or make them `unreachable`-logged no-ops).
- fix size: trivial
- confidence: confirmed

## Traps: every 200 ms the update walks every game object on the map
- kind: perf
- severity: low
- where: src/game/map/objects.rs:1806-1830 (`update`'s trap look); also :408-420 (`despawn`) and :1779-1786 (respawns), which search the map's spawn list linearly (`v.iter().find(|x| x.guid == s)`)
- what: with any player on the map, `update` filters all of `map.gameobjects` (tens of thousands on a continent) for `GO_TRAP` five times a second. Then, for every trap, near a player or not, it clones it and asks the grid. The despawn and respawn lookups are O(spawns) per object.
- fix: keep a set of trap GUIDs (add in `add_gameobject`, remove in `remove_gameobject`), and look only at those within a player's view (`grid.around` from each player, or the trap's observers being non-empty). Index spawns by GUID in `GameData` (`AHashMap<u32, idx>`).
- fix size: small
- confidence: confirmed

## A creature's speed change scans every player for its controller
- kind: perf
- severity: low
- where: src/game/map/derive.rs:1304 (`speed`: `map.players.values().find(|p| p.owned.mover == Some(g))`)
- what: every run-speed change of a creature (each wound threshold at 16, 11 and 6%, every slow or haste aura) is O(players on the map). In a crowded city fight that is thousands per change, each update.
- fix: use `movement::controller(map, g)`, which checks only the charmer, summoner, creator and owner.
- fix size: trivial
- confidence: confirmed

## The values flush walks every item of every player, every update
- kind: perf
- severity: low
- where: src/game/map/mod.rs:2327-2338 (`flush_values`)
- what: for each player, every item (about 100 with bags and bank) gets a `values_block` call and `clear_dirty`, every 50 ms, even when nothing changed. That is about 200k calls per update at 2,000 players, before the player's own dirty check.
- fix: keep a per-player "an item is dirty" flag, set where item fields change (`Item::fields` behind a setter, or a touched list as game objects have), and skip the loop otherwise.
- fix size: small
- confidence: confirmed (not timed)

## `distant_move_yards`: each heartbeat builds a client set and takes the table lock once per observer
- kind: perf
- severity: low
- where: src/game/map/mod.rs:2857-2884 (`send_move`)
- what: with the setting on, every heartbeat not on the every-Nth beat allocates a `Clients`, does a distance test and takes `clients.index_of` (a mutex) per observer, and the result isn't cached like `Observers::clients`. The setting also turns off `parallel_moves` (`moves::parallel`). It is off by default, so this only bites whoever turns it on to save bandwidth in a crowd, which is exactly the case it's meant for.
- fix: take the indices once (`observers.clients(&self.clients)`), filter that bitset by distance without the lock, and reuse a scratch `Clients`.
- fix size: small
- confidence: confirmed
