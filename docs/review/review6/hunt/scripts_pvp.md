# Bug hunt: scripts, instances, bosses and PvP maps (wvs)

Read at master `0c5811f4`. Reference: mangos-classic `mangos-classic/src/game`; data checked read-only in
`data/db/vmangos.sqlite` and `dbc.sqlite`.

## Stratholme: Ramstein gone before he dies locks the Baron for the copy's life
- kind: bug
- severity: high
- where: src/game/map/instance_adapters.rs:2793 (strat_creature, the last abomination's death), :2864 (strat_settle), :2815 (Evade); src/game/map/instance_script.rs:361 (demote)
- what: Ramstein is a summon with despawn type 1 and 30 minutes (`Some((1_800_000, 1))`). Once he is summoned, `S_RAMSTEIN_SUMMONED` is set and `STRAT_RAMSTEIN_EVENT` is saved as DONE, so he is never summoned again. Only his death sets `STRAT_RAMSTEIN` to DONE, and only that death (through `T_STRAT_BLACK_GUARD`) clears `STRAT_BARON_LOCKED` from Baron Rivendare. Two ways to hit it: (a) the group wipes on Ramstein or leaves him alone, and his 30 minutes run out out of combat, so he despawns; (b) the copy unloads (empty for `instance_unload`) or the server restarts while he is alive. In both cases summons are not reloaded, `strat_settle` sees `RAMSTEIN_EVENT == DONE` and keeps `S_RAMSTEIN_SUMMONED = 1`, and the abominations stay dead. Ramstein never comes back, and the Baron stays not-selectable and immune for good. That also means the Baron run quest (Ysida) can't be done.
- fix: in `strat_settle`, and when Ramstein despawns while not DONE, summon him again if `RAMSTEIN_EVENT == DONE && RAMSTEIN != DONE`. Or demote `RAMSTEIN_EVENT` to NOT_STARTED with `S_RAMSTEIN_SUMMONED` cleared when Ramstein isn't dead, and have the death-count check run once the copy has settled.
- fix size: small
- confidence: confirmed

## BWL: a wipe in Nefarian's first stage leaves Lord Victor Nefarius hostile at his table
- kind: bug
- severity: high
- where: src/game/map/bwl/nefarian.rs:244 (rises), :443 (Spawned), :555-575 (the REACHED_HOME event); src/game/map/scripts.rs:1781 (command 22), :1203 (restore_faction)
- what: On Spawned, Nefarius is made friendly with `step 22 [35, 0]`. That records a temporary faction whose `original` is the template's 14 (hostile monster, checked in the data). `rises` then sets faction 103 with restore flags `0x1|0x4`. Because a temporary faction is already tracked, the new record keeps `original = 14`, not 35. After a wipe in the drakonid stage, Nefarius walks home, and `evade_step` calls `restore_faction(REACH_HOME)`, which sets faction 14. The REACHED_HOME script then sets FAILED, adds the gossip flag back and makes him selectable. It does not set 35 again, and it doesn't put back `UNIT_FLAG_IMMUNE_TO_PLAYER` or `no_combat_movement`. The result is a hostile Nefarius at his table, whom the client won't gossip with. The fight can't be started again until he respawns, and nothing respawns him: `cleanup` only respawns a dead Nefarius. The test `nefarians_lair_lost_is_put_back` checks only the npc flag.
- fix: in the REACHED_HOME event (or in `cleanup` on FAILED), do what Spawned does: faction 35 with `datalong2 0`, `temp_faction` cleared, IMMUNE_TO_PLAYER set again, `no_combat_movement` false. Have the test check the faction.
- fix size: small
- confidence: likely (the faction path is confirmed by reading; the in-game effect is inferred)

## BWL: Vaelastrasz can never be started again once a failed try is saved
- kind: bug
- severity: high
- where: src/game/map/bwl/vael.rs:106 (area_trigger), :236 (Spawned); src/game/map/bwl/mod.rs:208 (demote)
- what: A wipe on Vael sets VAELASTRASZ to FAILED (home_fails), and the value is saved. `demote` only resets IN_PROGRESS and SPECIAL. When the copy reloads (empty for `instance_unload`, or a restart), Vael's Spawned event lays him down (stand state dead) and clears gossip, because the field isn't SPECIAL. The intro trigger 3626 needs `== NOT_STARTED`, and FAILED isn't. Vael lies there with no gossip, not hostile, and his door (`Open::Done(VAELASTRASZ)`) never opens, so the rest of the lair is shut until the raid's lockout resets. mangos-classic has the same latent bug (its trigger also wants NOT_STARTED and its `Load` keeps FAIL), but there Vael stays standing and hostile after a wipe as long as the map lives.
- fix: in `demote`, also reset FAILED to NOT_STARTED for VAELASTRASZ, or let the trigger accept FAILED as well.
- fix size: trivial
- confidence: confirmed

## Arathi Basin and Alterac Valley banners (and silithyst) are taken by CMSG_GAMEOBJ_USE with no cast
- kind: bug (exploit)
- severity: high
- where: src/game/map/world.rs:698 (use_object); src/game/map/battleground/mod.rs:983 (use_object), ab.rs (use_object), av.rs:369 (use_object); outdoor_pvp.rs:1032
- what: The AB banners (e.g. 180087, a goober with lock 1479: lock type 17 "Slow Open", whose spell 21651 "Opening" takes 10 s) are meant to be opened by that interruptible 10-second cast. The cast path does exist (`professions.rs:877` calls `battleground::opened`). But `world::use_object` passes any CMSG_GAMEOBJ_USE straight to `battleground::use_object`, and that assaults or defends the base after only a 10-yard and alive check. A modified client can therefore take a base instantly, with no cast to interrupt, and while stealthed (no stealth break on this path). The same holds for AV's graveyards and towers, and for Silithus's silithyst mounds (lock 1635, "Quick Open"). mangos only handles these banners in `EffectOpenLock`. The real client sends a cast for a locked object, not a use. The tests (`tests_p3_world_bg::use_banner`) send CMSG_GAMEOBJ_USE, which hides this.
- fix: in `use_object`, refuse an object whose template has a lock (goober `data[0]` / button `data[1]` non-zero) when it comes from CMSG_GAMEOBJ_USE. Keep the battleground handling for `opened` only, and move the tests to a cast of the opening spell (or to a helper that calls `opened`).
- fix size: small
- confidence: confirmed

## BWL Razorgore: a wipe before anyone fights Razorgore leaves the encounter IN_PROGRESS and the main gate shut
- kind: bug
- severity: medium
- where: src/game/map/bwl/razorgore.rs:224 (timer T_DEFENSE), :116 (creature_event); bwl/mod.rs:95 (main_gate_open)
- what: Grethok's pull sets RAZORGORE to IN_PROGRESS, and the only ways to FAILED are Razorgore reaching home from a fight or a death he is kept from. If the raid wipes before Razorgore is ever in combat (killed Grethok, then died to the defenders before using the orb), neither happens. The defenders are zone-pulsed against players only. `T_DEFENSE` keeps summoning them every 15 s up to 52, and the field stays IN_PROGRESS, so `main_gate_open` keeps DOOR_RAZORGORE_ENTER shut against the raid running back. It only clears when the copy empties and unloads; if anyone stays inside, it never clears. mangos gives Razorgore a leashing check that evades him when no living player is in the map (`SetLeashingCheck(... GetPlayerInMap(true,false) == nullptr)`), and that ends in FAIL.
- fix: in `T_DEFENSE` (or a 1 s watch), set FAILED when no living, non-GM player is in the copy, as mangos's leash does.
- fix size: small
- confidence: likely

## Escorts fail on the starter alone, during fights, and only for the starter
- kind: bug (1.12 rule)
- severity: medium
- where: src/game/map/escorts/mod.rs:1174 (step), :1018 (fail)
- what: `strayed` measures only `player` (the one who accepted). The check runs before the `fighting || moving` early return, so an escortee that chases a mob past `max_distance` fails the quest mid-fight. mangos's `npc_escortAI::UpdateAI` checks `IsPlayerOrGroupInRange()`: any group member in range keeps the escort going. It skips the check while the escortee has a victim, while it is returning, and while it is paused. On failure it fails the quest for the player and their group (`FailQuestForPlayerAndGroup`). In wvs a group escort fails when the starter alone dies, logs out or falls behind with the rest beside it, and only the starter's quest is failed. The others keep an incomplete escort quest they can't restart without abandoning it, because `accepted` only fires on accept.
- fix: count the escort as near when any group member on the map is within `max_distance`. Skip the check while the escortee fights or is held. Fail the quest for the group, as `credit` already walks the group.
- fix size: small
- confidence: confirmed

## Battleground end: the dead stay ghosts, fighting goes on, AV's NPCs keep killing
- kind: bug (1.12 rule)
- severity: medium
- where: src/game/map/battleground/mod.rs:1219 (finish), :815 (resurrect_wave)
- what: mangos's `EndBattleGround` resurrects every dead player (`ResurrectPlayer(1.0)` plus bones), stops their combat, blocks movement, and makes every creature immune and stunned ("else in av some creatures will kill the players at the end"). wvs's `finish` does none of this, and `resurrect_wave` returns early in `Ended`. So anyone dead at the end stays a ghost for the whole `bg_auto_leave` (2 minutes): resurrection waves have stopped, and `leave` revives them only on the way out. Players can also keep fighting each other, and AV's guards and bosses keep attacking.
- fix: in `finish`, resurrect the dead (and convert their corpses), `leave_combat` everyone, and in AV set the creatures immune to players (or evade them).
- fix size: small
- confidence: confirmed

## Invitations to a match survive its end, and taking one ports into a finished battleground
- kind: bug
- severity: low
- where: src/game/battlegrounds.rs:900 (ended), :710 (port)
- what: `ended` only flags the match. `invited` entries issued shortly before the end stay valid until they lapse (`bg_invite_for`). `port` finds the invite, makes the player a member and teleports them in, where they see the final scoreboard and are sent home with the `Ended` timer. A player can be pulled out of the world (or out of the queue for a live match) to land in a finished match.
- fix: in `ended`, clear `invited`, freeing their slots and sending each the none status. Or refuse in `port` when `m.ended`.
- fix size: trivial
- confidence: confirmed

## Deadmines: the cannon's assault stalls for good if the copy unloads in its 3-second window
- kind: bug
- severity: low
- where: src/game/map/instance_script.rs:904 (object_used 16398), :1021 (T_DEADMINES_ALARM); demote has no Deadmines case
- what: Firing the cannon sets field 1 to 1 and schedules the alarm 3 s later. Timers aren't saved, and `demote` leaves Deadmines alone. If the copy goes in between (a restart, say), field 1 stays 1 for good: the alarm never fires, the pirates never charge, and the cannon can't be fired again (`== 0` is required).
- fix: in `demote`, map field 1's value 1 to 2 (the alarm given), or schedule the alarm again in `settle_spawns` when it reads 1.
- fix size: trivial
- confidence: confirmed

## Gurubashi Arena: an outsider's pet may attack a fighter during the battle
- kind: bug
- severity: low
- where: src/game/map/arena.rs:1208 (can_attack)
- what: When one side is a fighter and the other is a non-fighter's creature (a pet from the stands), `can_attack` returns `None` (`guid::is_unit(a) || guid::is_unit(b)`), so the normal free-for-all rules apply. A player in the stands can't strike a fighter (`Some(false)`), but their pet can, and so can a totem or guardian, so a battle's outcome can be decided from outside.
- fix: resolve each side to its master first, then apply the fighter-vs-outsider rule to the masters. Return `None` only when neither master is a fighter.
- fix size: trivial
- confidence: likely

## Escort run_acts indexes `map.escorts[i]` across calls that run arbitrary scripts
- kind: robustness
- severity: low
- where: src/game/map/escorts/mod.rs (run_acts, summon_for, hold)
- what: `run_acts` holds the index `i` while calling `script_step`, `cast_triggered`, `instance_script::set_data`, `deal_damage` (Act::Die) and summons. Today only `update` removes from `map.escorts`, so the index stays valid. But any future path that ends an escort synchronously (an escortee death handler, a map-event end that removes rather than flags) would make `map.escorts[i]` hit another escort or panic. The panic would be caught, and the escort lost.
- fix: look the escort up by `(creature, quest)` after each act that can run a script, or collect the effects and apply them after the loop.
- fix size: small
- confidence: suspicion

## Battleground arrival counts anyone who lands on the map as a player of the match
- kind: robustness
- severity: low
- where: src/game/map/battleground/mod.rs (arrived)
- what: `arrived` inserts a `Score` for anyone who arrives, by their side, without checking that they are a member of the match (`battlegrounds::Match.members`). A GM's `.tele`, a summon, or a login put back onto the map gets onto the scoreboard, becomes a `players_of` target for team honor, reputation and kill credit, can take flags and banners, and counts toward the match starting (`Waiting` to `Starting`).
- fix: score only those in `members` of the match on `key.instance`, and keep the rest out of `team_of`.
- fix size: small
- confidence: likely

## Script runs whose source isn't a unit live on until their last step
- kind: robustness
- severity: low
- where: src/game/map/scripts.rs:179 (advance)
- what: A run is dropped when its source is a player or creature no longer present, but not when the source is a game object, a corpse or zero (`run_event` falls back to the target). Such a run with long delays (event scripts span minutes) keeps running after its object has despawned, and its object steps fail one at a time. It's bounded, but it also keeps `runs` from shrinking.
- fix: also drop runs whose game-object source is gone, unless the step's target type doesn't need a source (`NULL_SOURCE_TARGET_TYPES`).
- fix size: trivial
- confidence: likely

## `instance_script::run_event` starts map event scripts from an arbitrary creature
- kind: improvement
- severity: low
- where: src/game/map/instance_script.rs (run_event)
- what: The source is `map.creatures.iter().next()`, the first slab entry. That can be a dead creature, one far away, or a pet, so a step whose target type is relative to the source (nearest creature, say, or players within X) behaves differently from copy to copy.
- fix: use the target as the source (as vmangos's map events run from the instance), or the nearest living creature to the target.
- fix size: trivial
- confidence: likely

## WSG and silithyst rely on the carrier being in `map.players` to drop the flag
- kind: robustness
- severity: low
- where: src/game/map/battleground/wsg.rs (drop_flag, update)
- what: `drop_flag` returns without changing the flag's state when the carrier isn't in `map.players`, and `update` checks for a lost aura only for carriers who are present. Today `take_player` calls `departing` before removing the player, so the order holds. But a carrier removed by any other path (a panic in `take_player` before `departing`, a future removal) leaves `Flag::Carried(g)` for good: no flag, no capture, and the match can only end prematurely. Silithus handles the absent carrier (`lost(map, g, last)`); WSG doesn't.
- fix: in `wsg::update`, treat an absent carrier as lost and return the flag home (or drop it at its last known spot).
- fix size: trivial
- confidence: suspicion

## Town guards: no cap on the guards one town summons at once
- kind: robustness
- severity: low
- where: src/game/map/guards.rs (attack_started, call)
- what: Each victim change of a guard-calling civilian summons a guard (two at a high alarm) for `despawn_secs` (5 minutes) after its fight. A civilian whose victim alternates between attackers calls a guard on every switch. This is as mangos has it, but mangos's guards go after 30 s, while wvs's stay ten times as long. Only the shout is rate-limited (`SHOUT_EVERY`). A group farming civilians in a capital can grow a town's live guards into the hundreds, each awake and pathing, on that map's thread.
- fix: a per-town (or per-caller) cooldown or live cap in `[guards]`, such as at most one call per caller every N seconds, or N live summoned guards per town.
- fix size: small
- confidence: confirmed (unbounded by design; the load impact is inferred)
