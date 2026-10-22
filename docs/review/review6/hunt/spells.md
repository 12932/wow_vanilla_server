# Bug hunt: spells and auras (wvs, master at 0c5811f4)

Scope read: `src/game/map/spells/` (mod, checks, cooldowns, effects, mods, interrupts, single_target, capacity,
aura_rank, heartbeat, dynamic, sight, lists, parts of scripts/class_scripts/targets/outcome), `src/game/map/auras.rs`,
`src/game/map/procs.rs`, the head of `proc_scripts.rs`. Checked against mangos-classic (`mangos-classic/`) and,
where useful, Thistle Tea. Data facts below are from `data/db/vmangos.sqlite` (opened read-only).

## A "cooldown on event" spell whose aura never lands is locked until relog (Riposte, Counterattack)
- kind: bug
- severity: high
- where: src/game/map/spells/cooldowns.rs:60-70 (`start`), src/game/map/auras.rs:2502 (`removed`, the only
  release for a spell's aura), src/game/map/spells/checks.rs:406-416 (`check` refuses while pending)
- what: for a spell with `SPELL_ATTR_COOLDOWN_ON_EVENT` (0x2000000), `execute` calls `cooldowns::start` before the
  spell resolves, which pushes the spell (and blocks its category) in `pending_cooldowns`. Only `aura_gone`
  (a holder of that spell from that caster coming off), an object's end or a summon's end lets it go. If the aura
  never goes on, nothing ever releases it, and `check` answers NOT_READY for good. Not saved, so a relog clears it.
  In 1.12 data this includes harmful spells at an enemy: Riposte (14251: weapon damage + disarm aura 67) and
  Counterattack (19306/20909/20910: damage + root aura 26). A Riposte that is dodged, parried, misses, or lands on a
  disarm-immune boss (or one whose disarm is resisted by `effect_resisted`) puts no aura on: Riposte is then unusable
  for the rest of the session. Same for Counterattack against root-immune targets. Two other ways to strand it:
  the aura's target leaves the map before the aura ends (the removal runs on the other map, where
  `caster_mut(h.caster)` is `None`); and `cooldowns::load` (cooldowns.rs:475-486) pends a saved aura's spell before
  `restore` knows if the aura comes back (`left_after` drops an expired harmful one; `apply_spell` may refuse it).
- fix: after `execute` runs a cooldown-on-event spell, if no holder of (spell, caster) landed anywhere (an aura on
  the caster or on a target), call `cooldowns::release(map, caster, spell, true)` at once. In `restore`, release
  the pend if the aura wasn't put back. For a target on another map, release when the caster's aura on a unit
  leaves the map (`take_player`) or route the release through the registry.
- fix size: small
- confirmed: confirmed (no release path exists for a missed or refused aura; the attribute is on those spells in
  `spell_template`)

## Silithyst duplication through a logout
- kind: bug
- severity: high
- where: src/game/map/spells/cooldowns.rs:346-374 (`saved_auras`), :569-623 (`restore`);
  src/game/map/outdoor_pvp.rs:1140-1150 (`silithus_update`), :1082 (`area_trigger`)
- what: a carrier's `Sillithus Flag (DND)` (29519) is self-cast, has no duration and isn't passive, so
  `saved_auras` saves it. When the carrier logs out, `take_player` drops them from the map. The next
  `silithus_update` finds no player for the carrier and calls `lost`, which drops a silithyst mound where they
  were. At login `restore` puts 29519 back on them. The turn-in (`area_trigger`) checks only
  `p.auras.has_spell(SPELL_CARRYING)`, so the one silithyst is turned in by the returning player and picked up again
  from the mound: the rewards, the Cenarion reputation and a `brought` count toward the zone's capture, twice.
  mangos doesn't save it: the aura's interrupt flags (3801088) include `AURA_INTERRUPT_FLAG_LEAVE_WORLD` (0x80000),
  which `SpellAuraHolder::IsSaveToDbHolder` refuses.
- fix: the next finding's (don't save LEAVE_WORLD auras). Belt and braces: take 29519 and the team auras off in
  `outdoor_pvp` on `take_player`, or check `si.carriers` at the turn-in.
- fix size: trivial
- confirmed: confirmed (by reading the save, the restore, `silithus_update` and the turn-in)

## Saved auras ignore mangos's save rules: map-leaving and single-target auras survive a relog
- kind: bug
- severity: medium
- where: src/game/map/spells/cooldowns.rs:346-374 (`saved_auras`)
- what: `saved_auras` keeps every non-passive, non-gear aura that isn't another caster's lasting one and isn't a
  channel. mangos's `IsSaveToDbHolder` (SpellAuras.cpp:5868) also refuses auras with `AuraInterruptFlags & 0x80000`
  (leave world/teleported), single-target tracked auras, and the Echoes of Lordaeron world buffs. In 1.12 data, 36
  aura spells carry 0x80000, among them Polymorph (118, 12824-12826, 28270-28272), Warsong/Silverwing Flag
  (23333/23335), Silithyst (29519), Thaddius's Positive/Negative Charge (28059/28084/29659/29660), Ascendance
  (28200/28204), Cenarion Favor (30754) and The Baron's Ultimatum (27861). So a player who logs out polymorphed, or
  with a Thaddius charge, gets it back at login; a periodic or crash save keeps a flag carrier's flag. Saved
  auras from other casters' single-target spells come back with the caster absent (level 1, as R3-19 noted for
  level). wvs drops 0x80000 auras on teleport (`interrupts::changed_map`), but never at a logout.
- fix: in `saved_auras`, skip holders with `interrupt_flags & INTERRUPT_CHANGE_MAP != 0`, those whose spell is
  `CUSTOM_SINGLE_TARGET_AURA` (`single_target::limited`) unless self-cast, and the Echoes ids.
- fix size: trivial
- confirmed: confirmed

## A cast's line of sight isn't checked again as it ends: casts land through walls
- kind: bug
- severity: medium
- where: src/game/map/spells/checks.rs:777-878 (`at_finish`), called from src/game/map/spells/mod.rs:3247
- what: `at_finish` checks the target's presence, life, side, range, facing and stealth again, but not line of
  sight. mangos runs `CheckCast(false)` at `Spell::cast` (Spell.cpp:3201), and its LOS test (Spell.cpp:4850,
  `!IsIgnoreLosSpell && !m_IsTriggeredSpell && !IsWithinLOSInMap(target)`) isn't gated by `strict`. So in mangos a
  Fireball, Polymorph or heal whose target steps behind a pillar during the cast fails with LINE_OF_SIGHT; in wvs
  it lands. That breaks PvP line-of-sight play, and it lets creature casters hit through walls too. The same
  recheck in mangos also refuses a target that took a flight (`IsTaxiFlying`, Spell.cpp:4838) and the caster's own
  restraints (`CheckCasterAuras`: a pacify begun mid-cast doesn't stop a PREVENTION_PACIFY cast such as Slam, since
  only stun, fear, confusion and silence interrupt as they land).
- fix: in `at_finish`, after the range test: `if !s.attr(2, ATTR_EX2_IGNORE_LOS) && !line_of_sight(map, me.position,
  u.position) { return Some(fail::LINE_OF_SIGHT) }` (it already takes `&mut Map`); also refuse a player target on a
  flight, and check `prevented(map, caster, s)`.
- fix size: trivial
- confirmed: confirmed

## Shadow Trance (and other charged cast-time modifiers) gained mid-cast is spent by the cast already under way
- kind: bug
- severity: medium
- where: src/game/map/spells/mod.rs:1437 (`execute` recomputes `mods::for_spell` at launch), :1744
  (`mods::spend`); src/game/map/spells/mods.rs:175-181 and :192 (`used_by(OP_CASTING_TIME)` is
  `cast_time(s) > 0`)
- what: the cast time is worked out from the modifiers at `cast()` (mod.rs:1050), but the modifiers whose charges
  are spent are collected again at `execute`. A charged casting-time modifier that came on during the cast is
  counted as "used" because the spell has a cast time, and its charge is spent, though it changed nothing. The
  usual case: an affliction warlock casting a 2.5 s Shadow Bolt while Corruption ticks. Nightfall procs Shadow Trance
  (17941, one charge, -100% cast time) mid-cast, and the completing bolt spends it, so the next bolt isn't instant.
  mangos applies the cast-time mod at `prepare` (`m_appliedMods`) and `RemoveSpellMods` takes only those applied,
  so the trance survives for the next cast. Netherwind Focus (22008) behaves the same.
- fix: remember in `Casting` the charged holders that `cast()`'s `m` used for OP_CASTING_TIME (or the whole
  `m.charged` at start). At `execute` for a cast that had a cast time, drop from `m.charged` any OP_CASTING_TIME
  entry that wasn't there at the start.
- fix size: small
- confirmed: confirmed (by reading; no test covers a trance gained mid-cast)

## CMSG_CANCEL_CAST cancels whatever is being cast, and a queued strike, whatever spell it names
- kind: bug
- severity: medium
- where: src/game/map/handlers.rs:249-252; src/game/map/spells/mod.rs:3041-3044 (`cancel`), :2764-2769
  (`interrupt` clears `next_swing`)
- what: both `CMSG_CANCEL_CAST` and `CMSG_CANCEL_CHANNELLING` go to `cancel`, which interrupts the current cast or
  channel and cancels a queued next-swing ability, ignoring the packet's spell id. mangos's `HandleCancelCastOpcode`
  interrupts only when the current spell is the one named (`InterruptNonMeleeSpells(false, spellId)`), and
  `HandleCancelChanneling` stops only the channel. With latency, a client's cancel of a cast that already finished
  arrives after the next cast began and kills that one (its GCD given back), and a CANCEL_CHANNELLING interrupts a
  normal cast. It also drops a queued Heroic Strike or Maul.
- fix: pass the packet's spell id. CANCEL_CAST interrupts only a non-channel cast of that spell (and unqueues
  `next_swing` only if it is that spell). CANCEL_CHANNELLING interrupts only a channel.
- fix size: trivial
- confirmed: confirmed (mismatch with mangos by reading; the race needs latency to show)

## `tick_areas` walks stale indices when an area is cancelled during a tick
- kind: bug
- severity: low
- where: src/game/map/spells/effects.rs:2100-2113 (`due` indices), :2336-2344 (`areas_cancelled`'s `retain`)
- what: `due` is a list of indices into `map.spells.areas` taken before the loop. A tick's damage can kill or
  interrupt a unit that is itself channelling a ground spell (`periodic_damage` → death → `clear_on_death` →
  `interrupt` → `end_channel` → `areas_cancelled`, which `retain`s the vector), for example a mage channelling
  Blizzard who dies to a warlock's Rain of Fire tick. Later indices then point at other areas: an area that isn't
  due is ticked and has its `next_tick` pushed a period on, and the one that was due waits an update.
  (`area_ticked` usually hides the double hit; the cadence still shifts.)
- fix: collect the due areas' `object` GUIDs and find each again by GUID in the loop, or work on a snapshot and
  write `next_tick` back by GUID.
- fix size: trivial
- confirmed: confirmed (reachable by reading); rare in play

## Every damage-over-time tick clones the spell's whole template
- kind: perf
- severity: low
- where: src/game/map/spells/mod.rs:2509-2518 (`periodic_damage_of`); also :1680-1689 (`InFlight.spell:
  Box::new(s.clone())` for every target of a travelling spell)
- what: `periodic_damage_of` does `data.spells.get(h.spell).cloned()`. `SpellTemplate` holds three `String`s (name,
  rank, script_name) and about 200 bytes of fields, so every DoT tick in a raid allocates four times. It also runs
  `mods::for_spell` (a scan of the caster's holders) for the threat multiplier. `data` is an `Arc` already held, so
  a borrow would do. Each target of a travelling spell (bolts, Arcane Missiles, Multi-Shot) boxes a full clone the
  same way, plus a `Launch` clone with its `Mods`.
- fix: borrow `&SpellTemplate` from the held `data` (fall back to a stack default only when missing); keep the
  spell id (or an `Arc<SpellTemplate>`) in `InFlight` and look it up as it lands.
- fix size: small
- confirmed: confirmed

## Party auras' refresh clones every player's holders each second, and scans every creature
- kind: perf
- severity: low
- where: src/game/map/spells/effects.rs:2406-2424 (`refresh_area_auras`)
- what: once a second per map, `units` clones the whole `holders` vector (each with its `effects` Vec) of every
  player and every owned creature, and walks all `map.creatures` (calling `pets::owner_of` on each one with auras),
  only to find party-aura sources. With hundreds of bots that's thousands of holder clones a second for a handful
  of paladin auras and totems. The orphan pass then rescans every player's holders.
- fix: iterate the holders in place, collecting only (caster, spell, effects, expires) of holders whose spell has
  `EFFECT_APPLY_AREA_AURA_PARTY`. Find owned creatures through the pets registry rather than all creatures.
- fix size: small
- confirmed: confirmed

## CMSG_CANCEL_AURA lacks mangos's refusals
- kind: bug
- severity: low
- where: src/game/map/auras.rs:3682-3703 (`cancel`)
- what: mangos's `HandleCancelAuraOpcode` also refuses (a) auras with `SPELL_ATTR_EX_NO_AURA_ICON` (hidden ones,
  other than tracking), (b) any cancel while the player is possessed or charmed, and (c) another caster's area aura.
  wvs checks only positive, not passive, and `ATTR_CANT_CANCEL`. A client can cancel hidden helpful auras the UI
  never offers (e.g. a hidden Ghost 20584, linked children aren't reachable as they're passive, but non-passive
  hidden scripted buffs are). It can also strip itself of a party aura (the next second's refresh puts it back).
- fix: add the three checks before `remove_where`.
- fix size: trivial
- confirmed: confirmed (mismatch by reading); real impact depends on which hidden helpful auras matter

## In-flight spells of a caster who left the map still land
- kind: bug
- severity: low
- where: src/game/map/spells/mod.rs:1957-1986 (`land_arrivals`), src/game/map/mod.rs:1749-1758 (`take_player`
  interrupts the cast but leaves `map.spells.in_flight`)
- what: `land_arrivals` checks only that the target lives. A bolt from a player who logged out, or teleported
  away, mid-flight lands with its caster absent: `contact` calls `ai::attacked(target, caster)` and
  `combat::struck` for a unit not on the map, and damage taps for them. mangos kills a unit's pending spell events
  as it leaves the world (a caster's death doesn't stop the bolt, leaving does).
- fix: in `take_player` (and on map change) drop `in_flight` and `shots` entries whose caster is leaving; or skip
  arrivals whose caster has no `unit_view`.
- fix size: trivial
- confirmed: likely (the landing path with an absent caster wasn't traced end to end)

## Dead expression in `replaces`
- kind: improvement
- severity: low
- where: src/game/map/auras.rs:1507-1510
- what: `return old.item != new.item && old.item.is_some() && new.item.is_some() && false;` is always `false`. It
  reads as if a same-spell same-caster holder from another item is replaced, but it never is (the comment says
  "Refreshed in place, unless from another item"). Two copies of one item-sourced aura from two items stand
  together, as they should by mangos's rules, but the code says otherwise.
- fix: `return false;` with the comment saying why.
- fix size: trivial
- confirmed: confirmed

## Creature spell lists run the whole cast check twice per attempt
- kind: perf
- severity: low
- where: src/game/map/spells/lists.rs:196-200 then :235 (`super::cast` runs `checks::check` again)
- what: `attempt` calls `checks::check`, and `cast` calls it again. That's two navmesh line-of-sight raycasts (and
  for a charge, two path queries) per creature spell attempt, on every list tick of every fighting creature.
- fix: let `attempt` go straight to `cast` and read the failure from it (have `cast` return the code), or pass a
  flag so `cast` skips the second check.
- fix size: small
- confirmed: confirmed

## Cannibalize's corpse search scans every creature on the map
- kind: perf
- severity: low
- where: src/game/map/spells/scripts.rs:567-576 (`edible_corpse`), called from checks.rs:625
- what: `map.creatures.iter()` over the whole map (tens of thousands of creatures on a continent) for each
  Cannibalize check, though the range is about 5 yards.
- fix: `map.grid.around(at.x, at.y, range + margin)`.
- fix size: trivial
- confirmed: confirmed

## Review 3's spell and aura leftovers are still open
- kind: bug
- severity: low
- where: as listed in docs/review/review3.md
- what: re-checked on master. These are still as described there:
  - R3-16: `apply_spell`'s holder index is still computed after the replaced holders' `ended`/`removed`
    (auras.rs:1175-1176). It still uses `a.holders[index]` and `.expect("the unit is here")` (auras.rs:1187-1188),
    and `len() - 1` can underflow if `removed`'s `source` cascade takes the new holder too.
  - R3-18: a refresh still resets `proc_ready_at` (auras.rs:1124-1128 takes `holder.clone()`'s `None`).
  - R3-20: ground ticks still roll with `Mods::default()` (effects.rs:2165).
  - R3-22: the at-finish BAD_TARGETS branch still tells only the caster and keeps the GCD (mod.rs:3252-3259).
  - R3-23: `launch_failed` still keeps the GCD (mod.rs:1930-1942).
  - R3-24: a mounted caster is still dismounted before `checks::check` (mod.rs:1001-1007).
  - R3-25: a created item for another player with full bags is still lost (effects.rs:1202-1218).
  - R3-26: proc candidates are still a snapshot (procs.rs:490-499).

  R3-19 (offline time) and R3-21 (Drain Life's healing taken) are fixed.
- fix: as in review3.md.
- fix size: small (each)
- confirmed: confirmed
