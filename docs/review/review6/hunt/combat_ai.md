# Bug hunt: combat, creatures and pets (wvs, master at 0c5811f4)

Scope: `src/game/map/{combat,creature_combat,ai,event_ai,creature,guards,elites}.rs`, threat (`spells/threat_sync.rs` as the
creatures use it), respawn, and `src/game/map/pets/`. Read only; every claim below was checked in the code and its callers,
and the 1.12 rules against mangos-classic (`mangos-classic/src/game`). Decisions in `docs/DECISIONS.md` (leash 74,
spawn grace 75, guards 64/70, combat timer 62, pets 59/71) are not reported.

## A player who possesses a creature can make it cast any spell in the game
- kind: bug (exploit)
- severity: critical
- where: src/game/map/pets/control.rs:322-330 (`cast`), reached from `CMSG_PET_CAST_SPELL` (control.rs:759) and `CMSG_PET_ACTION`'s spell branch (control.rs:542-556)
- what: `knows` is `s.spells.contains(&spell) || s.kind == Kind::Possessed`. For a possessed creature (a priest's Mind Control on any mob, an Eye of Kilrogg, the Gnomish Mind Control Cap) every spell id passes. `spells::checks::check` never asks whether the caster knows the spell, so a forged packet makes the creature cast whatever it can pay for. Spell 5 "Death Touch" is instant, costs no mana and instakills (effect 1). Spell 23965 "Instant Heal" and spell 1908 "Uber Heal Over Time" also cost nothing. A Mind Control is enough to one-shot players in PvP. mangos-classic's `HandlePetCastSpellOpcode` refuses a spell the unit doesn't have (`HasSpell`, its template spells). The bypass isn't needed either: `charmed()` already fills `s.spells` for a possessed unit (`summon::charm_spells`/`pick_charm_spells`, pets/mod.rs:1117-1123), and the eye gets the same through `summon_possessed`. It dates from cc5e8d39.
- fix: drop `|| s.kind == Kind::Possessed`. Add a harness test that sends `CMSG_PET_CAST_SPELL` with spell 5 from a Mind Controlled creature and expects no cast.
- fix size: trivial
- confidence: confirmed

## A possessed unit's movement is taken unchecked: teleport a Mind Controlled player (or mob) anywhere
- kind: bug (exploit)
- severity: high
- where: src/game/map/pets/possess.rs:255-321 (`moved`), called before any movement check from movement.rs:177 (`on_move`) and acks.rs:234
- what: when the client's active mover is a possessed unit, `moved` only runs `movement::finite`. Then it writes `info.position` straight into the victim player's `movement` (and resets their `move_check` to the new place) or into the creature's `position`. Nothing checks speed or distance (`move_guard`), and nothing stops a root or stun (it checks `driven`/`control` only, so fear and confusion, not `AURA_MOD_ROOT`/`AURA_MOD_STUN`). A priest's client can send one packet that puts the enemy player it Mind Controls 1,000 yards away, under the world, in lava or over a cliff, then let them go. The same works on a possessed mob (drag it into a town or onto a quest giver) or on an Eye of Kilrogg at any speed. mangos runs `VerifyMovementInfo` (and its anticheat) on the mover, whoever it is.
- fix: run the possessed unit's packets through the same guard as the player's own: a `MoveGuard` per controlled unit, or the controller's guard against the unit's speeds. Refuse a move while the unit is rooted or stunned.
- fix size: small
- confidence: confirmed

## A possessed or charmed creature that goes without dying leaves its master's client stuck on it
- kind: bug
- severity: medium
- where: src/game/map/pets/summon.rs:797-854 (`remove`), src/game/map/mod.rs:2717 (`remove_creature`), src/game/map/pets/ai.rs:96-122 (`step`, the dead branch)
- what: only `let_go` resets the master's side of a possession: `PLAYER_FARSIGHT`, `UNIT_FIELD_CHARM`, `owned.mover` and `SMSG_CLIENT_CONTROL_UPDATE{guid: master}`. `summon::remove` clears `owned.charm` alone. `Map::remove_creature` and `respawn()` reset nothing. Two cases hit this:
  - A Mind Controlled spawned creature despawned by a script (`scripts::despawn` → `quiet_death_for`, which never reaches `pets::died`): `pets::ai::step` sees it dead with no `dead_at` and calls `corpse_gone` → `remove`.
  - A Mind Controlled summon removed by an event's end, an invasion, an escort, a pool or `remove_for_good` (`remove_creature` straight away).

  Either way the priest's camera stays on a unit that isn't there and their client still moves it, so they can't move their own character. In the second case `owned.charm` also still names the gone unit, so `pets::check` answers `AlreadyHaveCharm` and `tame_check` answers `AnotherSummonActive` until they log out.
- fix: in `summon::remove` (and in `take_creature` for a creature whose `pet` is Charmed or Possessed by a player) call `let_go(map, owner, g, false)` first. Have `quiet_death_for` go through `pets::died` for a controlled creature.
- fix size: small
- confidence: likely (the code paths are confirmed; no harness test was run)

## The tap is lost when the tapper dies, feigns death, vanishes or leaves: a passer-by takes the kill
- kind: bug (kill credit)
- severity: medium
- where: src/game/map/ai.rs:299-316 (`forget_by`, `if cr.tapped_by == Some(g) { tap::clear(cr) }`). Callers: death.rs:226 (a player's death), auras.rs:2043 (Feign Death believed), spells/effects.rs:1306 (Vanish and the like), mod.rs:1760 (`take_player`)
- what: mangos-classic clears a creature's loot recipient only on evade (`Unit::TriggerEvadeEvents`, Unit.cpp:586) and at respawn (Creature.cpp:1926-1928), never when the tapper drops off its list. In wvs, take a group's tank who tapped a mob and dies, or a hunter who tapped it and feigns death so the pet tanks: the tap is gone while the creature fights on. The next player to land a blow (`ai::engage` → `tap::take`) takes it. That can be an outsider, who then gets the loot, XP and quest credit, and the creature turns grey for the group. A hunter's pet hitting it doesn't take it back, because `pets::drawn_in` refuses a master who is feigning.
- fix: don't clear the tap in `forget_by`. `evade` (ai.rs:2001) and respawn already clear it.
- fix size: trivial
- confidence: confirmed

## A creature is tapped by whoever it aggroes, not by who damages it
- kind: bug (1.12 rule)
- severity: medium
- where: src/game/map/ai.rs:149-153 (`engage`), reached from `aggro_scan` (ai.rs:1126), assistance (`ai::assist`), `creature_combat::zone_pulse`, taunts. Also pets/mod.rs:994-999 (`on_attacked`)
- what: `engage` taps the creature for the master of whoever it engages, even when that unit hasn't touched it: a player walking past who gets aggroed, or the enemy a helper is called against. mangos taps only in `Unit::DealDamage` (Unit.cpp:990-991: `if (!creatureVictim->HasLootRecipient()) SetLootRecipient(dealer)`). Scenario: a mob aggroes player A, who runs on, and player B kills it. The mob is A's, grey to B, and B gets no loot or XP. A helper pulled in by a call for help is likewise tapped by the puller before anyone hits it. Check this against Thistle Tea's engagement (its `leave/3` clears the tap, which may mean it taps on engage). If that's deliberate, it belongs in DECISIONS.
- fix: take the tap in `deal_damage_with` (combat.rs:3274, where `creature_combat::damaged` is) and in a harmful spell's hit, not in `engage`/`on_attacked`.
- fix size: small
- confidence: likely

## A creature that evades without walking (sessile, or a follower already at its place) never "reaches home"
- kind: bug
- severity: medium
- where: src/game/map/ai.rs:1970-2010 (`evade`) against ai.rs:2077-2108 (`evade_step`)
- what: for a sessile creature (or `there`), `path` is `vec![from]`, so `walk_at` returns `None` and `c.evading` is set false at once. `evade_step` never runs, so the creature misses three things:
  - `scripts::restore_faction(FACTION_RESTORE_REACH_HOME)`: a temporary faction that should end on reaching home stays for good.
  - `event_ai::reached_home`: EventAI `EVENT_REACHED_HOME` (21) never fires.
  - The turn back to its spawn's facing.

  mangos-classic's `MoveTargetedHome` → `HomeMovementGenerator` finalizes at once when the creature is already home, and calls `JustReachedHome`.
- fix: in `evade`, when `frame.is_none()`, run what `evade_step`'s arrival does (factor it into a `home_reached` helper and call it from both).
- fix size: small
- confidence: confirmed

## A creature left with only an unseen foe on its list stands in combat for good
- kind: bug (stuck in combat)
- severity: medium
- where: src/game/map/ai.rs:1157-1184 (`passive_fight`), entered for any creature not attacking with `IN_COMBAT` set, after `victim_died` (ai.rs:366-376)
- what: `pick_victim` picks through `valid_victim`, which includes `stealth::sees`. `passive_fight`'s `left` uses only `alive && can_attack`, and `can_attack` doesn't look at stealth. When a creature's victim dies and the only one left on its list is invisible to it (an Invisibility potion, a creature-side detection limit), `victim_died` finds no victim and leaves it not attacking. Every turn after that `passive_fight` finds `left == true` and neither attacks nor evades. `aggro_scan` won't take the foe either, because it needs `sees`. The creature stands in combat, and its threat list holds that player in combat (`leave_finished_fights`) until they go 31 yards from the fight's start and the leash's time passes, or until their client forgets the creature. `fight_step` would have evaded in the same state.
- fix: use `valid_victim` for `left` in `passive_fight` (a passive creature's list then empties as `fight_step`'s does), or call `evade` when a non-passive creature's `pick_victim` is `None`.
- fix size: trivial
- confidence: likely

## Every creature death scans all 66,000 spawns; every respawn scans its map's twice more
- kind: perf
- severity: medium
- where: src/game/map/ai.rs:428-452 (`respawn_delay`: `data.creature_spawns.values().flatten().find(..)` over every map), ai.rs:670-674 (`respawn`), ai.rs:758-761 (`spawn_creature_with`), event_ai.rs:902-906 (`spawn_movement`), scripts.rs:1137 (`despawn_with`)
- what: `die_rewarding` calls `respawn_delay` for each spawned creature that dies. That is a linear search of all 66,001 spawn rows (vmangos.sqlite), on the map's thread. Each respawn then searches its map's ~29,000 (map 1) twice. With hundreds of bots grinding this is a steady cost on the update and grows with the data. `GameData` has no index by spawn guid.
- fix: build `AHashMap<u32 /*spawn guid*/, (u32 /*map*/, usize /*index*/)>` once at load, plus a `GameData::creature_spawn(guid)` helper, and use it in these five places.
- fix size: small
- confidence: confirmed

## A charmed creature turns on its master as the master leaves the map, after they were forgotten
- kind: bug (threat leak)
- severity: low
- where: src/game/map/mod.rs:1760-1763 (`take_player`: `ai::forget` before `pets::owner_leaving`), src/game/map/pets/mod.rs:1305-1314 (`let_go`)
- what: `take_player` drops the leaving player from every threat list, then `owner_leaving` → `release` → `let_go(turn = true)` → `ai::attacked(unit, master)` + `add_threat(max health)`. That puts the departing player back on the creature's list, takes its tap for them and starts a temporary war, and they are removed from the map a moment later. The creature fights a guid that isn't there until its next turn prunes it and it evades. The departed player carries a `temporary_war` to the new map. It corrects itself, but it is a reference to a player no longer in the map.
- fix: in `owner_leaving`, release with `turn = false` (`let_go(map, owner, c, false)`), as a logout or a map change shouldn't start a fight.
- fix size: trivial
- confidence: confirmed

## Each change of victim from a forget calls for help again
- kind: bug (1.12 rule)
- severity: low
- where: src/game/map/ai.rs:318-344 (`turn_to_next`) → `attacked` → `engage` with `call = true` (ai.rs:228-230)
- what: `turn_to_next` sets `attacking = false` and calls `attacked`, so `engage` gets past its `if attacking { return }` and runs `assistance::drawn_in` again. Every Feign Death, Vanish, death of a group member or `lost_sight_of` makes the mob call its friends against the next on its list. mangos calls for help once, on the pull (`CallAssistanceOnPull` in the aggro event, Unit.cpp:579).
- fix: call `engage(map, c, t, false)` (as `ai::assist`) from `turn_to_next`.
- fix size: trivial
- confidence: likely

## A player's pet is noticed from as far as a player
- kind: bug (1.12 rule)
- severity: low
- where: src/game/map/ai.rs:84-106 (`attack_distance`)
- what: mangos-classic's `Unit::GetAttackDistance` (Unit.cpp:11802-11803) shortens the aggro radius to 65% for a player's pet that isn't client-controlled ("player pets do not aggro from so afar"). wvs uses the master's level for a pet but no factor. A hunter's pet on Follow or Stay pulls mobs from the full radius.
- fix: multiply by 0.65 when `t` is a creature whose master is a player and which is not the client's mover.
- fix size: trivial
- confidence: confirmed

## A pet out of combat scans every awake creature at each combat check
- kind: perf
- severity: low
- where: src/game/map/pets/ai.rs:1005-1048 (`calm`)
- what: each pet in combat with no victim runs `map.awake_where(..)` over all awake creatures every `COMBAT_CHECK`, and builds a `Vec` it only tests for emptiness. Players have the cached and shared scan (`leave_finished_fights`, review 5's combat 7); pets don't. With many hunters and warlocks in a busy zone that is O(pets × creatures) a second.
- fix: use `any` without collecting, or fold pets into `leave_finished_fights`' shared scan.
- fix size: small
- confidence: confirmed

## The leash's timeout parses its config strings on the hot path
- kind: perf
- severity: low
- where: src/game/map/creature_combat.rs:683-698 (`leash_timeout`), called from `should_evade` up to three times a fight step (ai.rs:1380-1387) and from `gone_from_fight`
- what: each call runs `split_once` and `parse` over the `leash_timeout_by_level` `Vec<String>`. The work is small, but it happens per creature per 20 Hz update whenever a creature is past its radius, and it repeats work that is the same until a reload.
- fix: parse once into a sorted `Vec<(u32, u64)>` when the config is taken (`map.cfg`), or cache it on the map per config generation.
- fix size: small
- confidence: confirmed

## Overkill damage counts in the players' and NPCs' shares of a kill
- kind: bug
- severity: low
- where: src/game/map/creature_combat.rs:443-455 (`damaged`), called from combat.rs:3274 before the health is clamped
- what: the full blow is counted, not what the creature had left. Say a player does 900 of a 1,000-health mob and a guard finishes it with a 5,000 swing. Then 65 × 900 < 35 × 5,000, so `loot_allowed` is false and `xp_share` is 0: the player loses the loot and XP of a kill that was 90% theirs. Overkill on the players' side likewise outweighs an NPC's real share. (mangos counts unclamped too, but measures `m_damageByOthers` against max health, where overkill matters less.)
- fix: count `damage.min(current health)` (pass the health before the blow from `deal_damage_with`).
- fix size: trivial
- confidence: confirmed (the code); the 1.12 effect is a suspicion as to what Thistle Tea does

## A possessed player's master can cast the victim's whole spellbook
- kind: bug
- severity: low
- where: src/game/map/pets/control.rs:322-324 (`cast`, `Some(v) => v.spells.contains(&spell)`)
- what: for a Mind Controlled player the possess bar is attack only (`possess::bar`, "possessed players don't have spells", after mangos's `InitPossessCreateSpells`). `CMSG_PET_CAST_SPELL` still accepts any spell the victim knows, so a priest's client can make an enemy mage Polymorph or Pyroblast their own group. The real client can't send this from its bar, so it takes a crafted packet.
- fix: refuse spells for a possessed player in `cast` (keep `COMMAND_ATTACK`).
- fix size: trivial
- confidence: likely

## A creature removed without dying leaves its borrowed threat behind
- kind: robustness (leak)
- severity: low
- where: src/game/map/spells/threat_sync.rs:31-38, against src/game/map/mod.rs:2717 (`remove_creature`) and scripts.rs:1117-1171 (`despawn`, `remove_for_good`); also duel.rs:264 and scripts.rs:2364/2786, which clear or retain `threat` without `threat_sync`
- what: `Map::spells.temp_threat` is keyed by `(creature, unit)` and cleared by `wiped` and `removed`. A summon removed for good (despawned, an event's end, `remove_creature`) while a taunt's borrowing or a Fade is held on it keeps its entries for good. The script command 73 path, Follow, and the duel's pet cleanup drop units from `threat` without `removed`. The map grows a little with each one, and `fade_changed`'s scan sees stale keys.
- fix: call `threat_sync::wiped(self, guid)` in `take_creature`, and `removed`/`wiped` where those three paths edit `threat`.
- fix size: trivial
- confidence: confirmed

## `die_rewarding`'s respawn time is written as a puzzle
- kind: improvement
- severity: low
- where: src/game/map/ai.rs:519-522
- what: `respawn_at: now + respawn.max(decay.min(respawn))` is always `now + respawn` (`min(decay, respawn) <= respawn`). The expression suggests a rule that isn't there.
- fix: `respawn_at: now + respawn`, with the comment saying the corpse lies `min(decay, respawn)`.
- fix size: trivial
- confidence: confirmed
