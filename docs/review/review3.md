# Review 3: combat, spells, auras, threat and creature AI

Branch `fix/review3` in `<worktree>/review3`, from 9641e3c. Line numbers are at that commit.
Each finding: where, what is wrong, how it shows, severity, easy or hard, and its state (fixed in a commit, or
left with what a fix would take).

## Correctness

### R3-1. A cast's reagents are checked at launch across the bank, but taken only from the bags
- **Where:** `src/game/map/spells/checks.rs:466` (`at_launch`), against `spells/mod.rs:1000-1015` (`execute`'s take loop).
- **What:** `at_launch` counts reagents with `Player::count_of`, which counts bank slots and bank bags. The cast's
  start (`check`) counts with `carried_count_of`, and `execute` takes reagents only from carried items; when it runs
  out it `break`s and the spell goes off anyway.
- **How it shows:** start a craft with a cast time (a potion, a bandage) at a banker, move the reagents into the
  bank before the cast ends. The cast finishes, makes the item and takes nothing. Repeat: items from nothing.
- **Severity:** high (item duplication). **Easy.**
- **State:** fixed in bae55c8 (test `reagents_banked_during_a_cast_fail_it`).

### R3-2. A spell's damage counts the caster's threat auras twice
- **Where:** `src/game/map/spells/mod.rs:1617-1637` (`threat`), with `spells/outcome.rs:1156-1160` and
  `combat.rs:2364` (`threat_multiplier`).
- **What:** `outcome::deal` calls `deal_damage_by(.., school 1, ..)`, which already adds `damage *
  threat_multiplier(caster, 1)` (the product of the caster's `AURA_MOD_THREAT` auras for physical). `spells::threat`
  then adds `total - damage`, where `total` includes the sum of the same auras again, for the spell's school. So an
  all-school threat aura counts twice, and a school-only one (Righteous Fury's holy) once by luck. Thistle Tea and
  mangos multiply: damage x spell_threat's multiplier x the auras' product x OP_THREAT, plus the flat threat x the
  same multiplier.
- **How it shows:** Defensive Stance (+30%, all schools): a 100-damage spell gives 160 threat, not 130. Blessing of
  Salvation (-30%): 40, not 70.
- **Severity:** medium. **Easy.**
- **State:** fixed in 610214c (test `a_spells_threat_takes_the_threat_auras_once`). `outcome::deal` now passes the spell's school to `deal_damage_by`, and `spells::threat` multiplies by the auras' product and adds only what goes beyond `damage x product`.

### R3-3. Fade (a temporary threat cut) clamped at none hands back more threat than it took
- **Where:** `src/game/map/spells/threat_sync.rs:15-21` (`modify`) and `:34-62` (`set_temporary`).
- **What:** the cut is applied clamped at 0, but the whole amount is stored as what was taken, and given back in
  full when the aura ends. mangos's `HostileRefManager::threatTemporaryFade` stores `max(-current, amount)`, what
  was actually taken. (Thistle Tea has the same clamp; it is wrong there too.)
- **How it shows:** a priest with 100 threat casts Fade (-820): threat 0, then 820 when Fade ends.
- **Severity:** medium. **Easy.**
- **State:** left, deliberately. The fix (store the change actually made) is a one-liner, but `tests_p3_combat2::fades_threat_is_held_apart_and_given_back_whole` pins Thistle Tea's "given back whole" behaviour as intended parity. A decision for the parity owner: follow mangos (`threatTemporaryFade` stores `max(-current, amount)`) and change that test, or keep Thistle Tea's.

### R3-4. A cast while channelling replaces the channel without ending it
- **Where:** `src/game/map/spells/checks.rs:307-312` lets a different spell through during a channel;
  `spells/mod.rs` `cast` (about 873) and `execute` (about 1063) overwrite `Caster::casting`.
- **What:** mangos's `Unit::SetCurrentCastedSpell` interrupts the channel when a generic spell starts (unless either
  has `SPELL_ATTR_EX4_ALLOW_CAST_WHILE_CASTING`). wvs never calls `end_channel` for the old one: its channel fields
  stay, `MSG_CHANNEL_UPDATE` is never sent, its auras on the target and its persistent areas (Blizzard) run to their
  full time. An instant cast during a channel leaves the channel running as well.
- **How it shows:** channel Blizzard, cast Frostbolt: Blizzard keeps hitting, `UNIT_CHANNEL_SPELL` stays set.
- **Severity:** medium. **Easy.**
- **State:** fixed in ee07a08 (test `a_cast_during_a_channel_ends_the_channel`). Adds `ATTR_EX4_ALLOW_CAST_WHILE_CASTING`. Triggered channels in `execute` still overwrite a cast; left, as mangos sets no current spell for most triggered spells.

### R3-5. `interrupt` tells the caster twice that a cast was interrupted
- **Where:** `src/game/map/spells/mod.rs:1884` and `:1907`.
- **What:** for a cast that isn't a channel, `SMSG_CAST_RESULT(INTERRUPTED)` is sent inside the first `if`, and again
  in the `else` at the end.
- **How it shows:** move during a Fireball: two failure packets, two error messages.
- **Severity:** low. **Easy.**
- **State:** fixed in 1235db2 (test `an_interrupted_cast_is_told_once`).

### R3-6. A player leaving the map (logout, disconnect) keeps their cast, channel and threat
- **Where:** `src/game/map/mod.rs:745-770` (`take_player`), from `login.rs:312` and `supervise.rs:186`.
- **What:** a teleport interrupts the cast before `take_player`; a logout or a disconnect doesn't, and nothing takes
  the player off creatures' threat lists (mangos's `RemoveFromWorld` deletes the hostile references; death here calls
  `ai::forget`). A channel's auras on its target outlive the caster; a passive creature they hit, seen by no one
  else, keeps `UNIT_FLAG_IN_COMBAT` and its threat list until someone comes near (it isn't awake to notice).
- **How it shows:** disconnect while channelling Drain Life on a creature, or after hitting a passive one.
- **Severity:** medium. **Easy.**
- **State:** fixed in 161b6dd (test `a_player_leaving_the_map_ends_their_channel_and_their_threat`).

### R3-7. A dead attacker swings on in the same `try_swing`; the off-hand swings with the main hand
- **Where:** `src/game/map/combat.rs:1523-1564` (`try_swing`).
- **What:** the attacker's life and `attacking` are read once at the top. A swing can kill its attacker (a damage
  shield: Thorns, Retribution Aura, Lightning Shield, through `procs::damage_shields`), and the next extra attack or
  the off-hand still swings from the corpse: it logs a blow, deals damage, procs, and `engage` puts the dead attacker
  back in combat. Separately, `off_due` is worked out before the main hand's swing, which pushes the off-hand 200 ms
  on (mangos's `ATTACK_DISPLAY_DELAY`): mangos asks again, so the two never land in one step; wvs swings both.
- **How it shows:** a dual-wielder low on health hits a Thorns-buffed target when both hands are due; or a paladin's
  banked Reckoning attacks.
- **Severity:** medium (the corpse's blows), low (the off-hand's timing). **Easy.**
- **State:** fixed in 0955dc3, the corpse's swings (test `a_striker_killed_by_a_damage_shield_swings_no_more`); 31b97d8, the off-hand (test `the_hands_dont_swing_together`).

### R3-8. A creature re-engaged mid-fight moves its leash's origin to where it stands
- **Where:** `src/game/map/ai.rs:181-183` (`engage`).
- **What:** for a creature not attacking, `combat_start` and `engagement.origin` are set every time, not only on a
  fight's first engagement (the passive branch above guards them with `first`). `forget_by` (Feign Death, Vanish)
  and `feign_reset` stop the attack and call `attacked` for the next target, so each such switch moves the origin
  that `should_evade` measures the leash from.
- **How it shows:** a hunter kites a creature away and feigns death; the creature turns on the next player with its
  leash measured from where it is, so a group can drag it further from home than the leash allows.
- **Severity:** medium. **Easy.**
- **State:** fixed in 823e32e (test `a_creature_turning_mid_fight_keeps_its_leash_origin`).

### R3-9. A creature down in a linked feign death runs its timed events
- **Where:** `src/game/map/ai.rs:767-768` (`turn`), before the feign check at `:774-779`.
- **What:** `event_ai::tick` runs before the feigning creature's early return. `feign_death` only stops the attack;
  `UNIT_FLAG_IN_COMBAT` stays, so in-combat timers keep firing, and an HP event matches 1 health (0%).
- **How it shows:** a creature with a linked feign death (Scarlet Monastery's Mograine and Whitemane pairs, or any
  `Scripted.feign`) casts and yells its in-combat events while lying "dead".
- **Severity:** medium. **Easy.**
- **State:** fixed in d8f4c07 (test `a_creature_down_in_a_linked_feign_death_runs_no_timed_events`).

### R3-10. Restored auras get their stacks after `apply_spell`, so their slot and stats show one stack
- **Where:** `src/game/map/spells/cooldowns.rs:367-373` (`restore`).
- **What:** `apply_spell` writes the slot's applications byte and runs `recalc` with one stack; the saved stacks are
  then set on the holder with neither a slot rewrite nor a recalc.
- **How it shows:** log out with a 5-stack stat aura, log in: the bar shows 1, and the stats count 1 until something
  else recalculates.
- **Severity:** medium. **Easy.**
- **State:** fixed in 2d5bab1 (test `a_restored_aura_shows_and_gives_its_stacks`). New `auras::set_stacks`; it also matches the holder by its item, which the old `find` didn't.

### R3-11. A pet area aura of two effects (Soul Link, Stalker's Ally) keeps only its last
- **Where:** `src/game/map/pets/mod.rs:419-424`, through `auras::apply` (`auras.rs:802`).
- **What:** each `EFFECT_APPLY_AREA_AURA_PET` effect is applied on its own; the second finds the first's holder (same
  spell and caster), refreshes it and replaces its effect list with its own one effect.
- **How it shows:** Soul Link (25228): the pet gets the 30% damage split but not the 3% damage done; Stalker's Ally
  (28758) gives the pet one of its two bonuses.
- **Severity:** medium. **Easy.**
- **State:** fixed in 3c18ba5 (test `a_pet_area_aura_of_two_effects_puts_both_on_the_pet`). `effects::run` gathers the effects and `pets::area_aura` applies them as one holder.

### R3-12. Holders an aura replaces are ended but never `removed`
- **Where:** `src/game/map/auras.rs:1071-1078` (`apply_spell`).
- **What:** `remove_where` calls `removed()` for each holder it takes off; the replace path calls only `ended()`. So
  a replaced holder's diminishing returns never start to recover, a "cooldown when the aura ends" never starts (the
  spell stays pending for good), a channel whose aura was replaced isn't interrupted, and the trinket, item, class
  script and redemption cleanups don't run.
- **How it shows:** a second priest's Mind Control replaces the first's: the first priest goes on channelling
  nothing until the channel's time is up.
- **Severity:** medium. **Easy** (call `removed`).

## Robustness
- **State:** fixed in 9b2031c (test `an_aura_replaced_starts_the_cooldown_waiting_for_it`).

### R3-13. A panic in a proc leaves the thread's proc depth raised for good
- **Where:** `src/game/map/procs.rs:133-166` (`trigger`).
- **What:** the thread-local `DEPTH` is raised and put back by hand; a panic in between (which `supervise` catches
  per step, turn and packet) skips the reset. The thread is a tokio worker shared by every map on it: at depth 1 or 2
  every proc after counts as a proc's proc (only `CAN_PROC_FROM_PROCS` holders, no weapon procs), and after three
  such panics nothing procs on that thread until a restart.
- **Severity:** medium. **Easy** (a guard that restores it on drop).
- **State:** fixed in aff6a32 (test `procs::tests::a_panic_in_a_proc_leaves_the_depth_as_it_was`).

### R3-14. A creature that forgets its victim turns to one it may not fight
- **Where:** `src/game/map/ai.rs:262-265` (`forget_by`).
- **What:** the next victim is the raw highest threat on the list, without `valid_victim` (alive, attackable,
  seen), which the fight's own choice (`pick_victim`) applies. The creature targets and starts on a unit it can't
  attack (stealthed, immune to creatures) until its next turn prunes it.
- **How it shows:** the tank feigns death with a stealthed rogue next on the list.
- **Severity:** low. **Easy.**
- **State:** fixed in 694a75e (test `a_creature_forgetting_its_victim_turns_to_one_it_may_fight`).

## Left: findings not fixed

Each with what a fix would take. Ordered by severity.

### R3-15. A death action that despawns cuts `die_by` short, or queues the corpse twice (medium, hard)
- **Where:** `src/game/map/ai.rs:358` (`event_ai::death` is fired) before `:371-373` and `:419`.
- **What:** `scripts::start_checked` runs a script's delay-0 steps at once. An `EVENT_DEATH` action that despawns:
  a summoned creature is removed (`remove_creature`), and `die_by` returns at `creature_mut(..) else return`, so
  `creature_groups::died`, `death_heard` (a summoner's `SUMMONED_JUST_DIED`, a group's `GROUP_MEMBER_DIED`), loot,
  the battleground's and the instance's death hooks are skipped. A spawned one goes through `quiet_death_for`, which
  sees it still alive, sets it dead and pushes it on `dead_creatures`; `die_by` then sets its life again and pushes
  it a second time.
- **How it shows:** a boss counting dead adds by `SUMMONED_JUST_DIED`, whose adds' death event despawns them, never
  advances.
- **Fix:** set `life` to dead before firing `EVENT_DEATH` (so `quiet_death_for`'s alive check sees the death), or
  queue the death's events until `die_by` has run its hooks; and push to `dead_creatures` only if not already there.
  It reorders the death's hooks, so the EventAI and boss tests need a full pass.

### R3-16. `apply_spell`'s index of the new holder can go stale, and one early return skips `recalc` (medium-low, moderate)
- **Where:** `src/game/map/auras.rs:1077-1081` (the index), used at `:1092`, `:1140`, `:1147`; the early returns at
  `:1105-1111`, `:1117`, `:1133`.
- **What:** the index of the new or refreshed holder is worked out after the replaced holders' `ended` (and now
  `removed`) ran, which can add holders (a shapeshift's end gives back form passives) or remove others (`AURA_LINKED`
  children, a channel's end). The index then points at the wrong holder (`started` runs on it, the new one never
  starts) or past the end (a panic at `holders[index]`). Separately, when the full bar's least-worth holder is the
  new one itself, the function returns before `recalc`, so the replaced holders' stats stay in the fields until the
  next recalc.
- **How it shows:** I found no stock spell that triggers the stale index; the recalc skip needs a full bar of 32
  helpful (or 16 harmful) auras.
- **Fix:** find the new holder again by (spell, caster, item) after the replaced ones are ended, with `get`; call
  `recalc` on every return after something was replaced.

### R3-17. Out-of-combat timers fire at once after a fight that ends without an evade (low-medium, easy)
- **Where:** `src/game/map/event_ai.rs:290-300` (`reset_for_combat` leaves the OOC slots `None`) and `:860-864`.
- **What:** only `evade` re-rolls the OOC timers. A fight that ends otherwise (`stand_down` for a passive creature on
  a script's walk, a script's combat stop) leaves them `None`, "due now", and the same tick that fires
  `LEAVE_COMBAT` fires every OOC timer.
- **Fix:** in `tick`'s `was && !combat` branch, call `reset_ooc` as `evade` does. I didn't check what mangos does for
  a combat stop without an evade, so I left it.

### R3-18. Refreshing an aura resets its proc's internal cooldown (low-medium, easy)
- **Where:** `src/game/map/auras.rs:1027-1031`: `*old = Holder { stacks, slot, ..holder.clone() }` takes the new
  holder's `proc_ready_at: None`.
- **What:** a proc's cooldown (`spell_proc_event.cooldown`) lives on its holder; re-applying the aura (a seal cast
  again, a weapon buff renewed) makes the proc ready at once. mangos keeps proc cooldowns on the owner, by spell.
- **Fix:** keep `old.proc_ready_at` on refresh, as `slot` and the heartbeat are kept. I couldn't confirm where Thistle
  Tea keeps the cooldown, so I left it.

### R3-19. Saved auras' durations stand still while their player is offline (low, easy with a logout time)
- **Where:** `src/game/map/spells/cooldowns.rs:243-271` (`saved_auras`) and `:341-366` (`restore`).
- **What:** `left` is restored as saved. mangos-classic's `_LoadAuras` takes the time since logout off harmful auras
  (Forbearance, Weakened Soul, Recently Bandaged, a debuff from a fight). The saved caster's level isn't kept either:
  a caster not online makes the restored holder level 1.
- **Fix:** save the logout time (the row may have it) and take the time since off harmful auras' `left`, dropping
  those gone; save `level` in `SavedAura` (a `serde(default)` field, no migration).

### R3-20. Ground areas roll their hit without their caster's modifiers, and score creature-type bonuses on the caster (low, easy)
- **Where:** `src/game/map/spells/effects.rs` `tick_areas` (`roll_hit(.., &Mods::default())`) and `run_untargeted`
  (`aura_amount(map, caster, caster, ..)`).
- **What:** Blizzard, Rain of Fire, Consecration and Volley ticks miss more than they should for a caster with
  OP_RESIST_MISS_CHANCE talents (Arcane Focus, Suppression); "damage versus undead" bonuses are read against the
  caster's own creature type (a player's falls back to humanoid).
- **Fix:** keep the cast's `Mods` in `Area` and roll with them; leave the type bonus out of the stored amount and add
  it per victim, or drop it.

### R3-21. Drain Life's heal skips its caster's healing modifiers (low, easy)
- **Where:** `src/game/map/spells/effects.rs` `EFFECT_HEALTH_LEECH`: `heal(map, caster, caster, s.id, taken * mult)`;
  `spells::heal` applies nothing.
- **What:** mangos's `EffectHealthLeech` passes the gain through healing done and taken (`SpellHealingBonusDone`,
  `SpellHealingBonusTaken`): Mortal Strike's -50% halves a warlock's Drain Life heal there, not here.
- **Fix:** `outcome::healing_taken(map, caster, ..)` on the gain before `heal`.

### R3-22. A cast that fails at its end for a target gone tells no one but its caster (low, easy)
- **Where:** `src/game/map/spells/mod.rs` `update`, the `BAD_TARGETS` branch.
- **What:** only the caster gets `SMSG_SPELL_FAILURE`; observers, who saw `SMSG_SPELL_START`, get no
  `SMSG_SPELL_FAILED_OTHER`, so their cast bar runs out. The global cooldown isn't given back either (`interrupt`
  does both).
- **Fix:** send `SMSG_SPELL_FAILED_OTHER` to observers there, and clear the GCD as `interrupt` does.

### R3-23. An instant cast that fails at launch keeps its global cooldown and its `SMSG_SPELL_START` (low, easy)
- **Where:** `src/game/map/spells/mod.rs` `cast`: the GCD is set and the start broadcast before `execute`, whose
  `at_launch` and ammunition checks can fail (`launch_failed`).
- **How it shows:** Arcane Shot with no ammunition: the hunter is locked out for 1.5 s for a shot that didn't go.
- **Fix:** give the GCD back in `launch_failed`, or check the ammunition in `checks::check`.

### R3-24. A mounted player is dismounted by a cast that then fails its checks (low, easy)
- **Where:** `src/game/map/spells/mod.rs` `cast`: `remove_where(.. AURA_MOUNTED)` runs before `checks::check`.
- **How it shows:** casting a spell on cooldown, or out of range, while mounted dismounts.
- **Fix:** dismount after the checks pass.

### R3-25. A created item for another player is lost when their bags are full (low, easy)
- **Where:** `src/game/map/spells/effects.rs` `EFFECT_CREATE_ITEM`: `fitting(..).ok()...unwrap_or(amount)`; room is
  checked only for the caster (`checks::room_for_made`).
- **What:** the reagents are spent and `add_item` fails as a whole for a recipient who isn't the caster.
- **Fix:** check room for the recipient in `room_for_made`, or make a count of 0 skip the effect.

### R3-26. Procs run from a snapshot of the holders taken before any of them fires (low, easy)
- **Where:** `src/game/map/procs.rs:263-275`.
- **What:** a proc that removes or replaces another candidate (a dispel, an absorb used up, its owner's death) still
  lets the later one fire from its clone. mangos skips a deleted holder (`IsDeleted`).
- **Fix:** before firing, look the holder up again by (spell, caster, item) and skip it if gone.

### R3-27. An evade whose walk home makes no frame skips the reach-home work (low, easy)
- **Where:** `src/game/map/ai.rs:1611-1614` and `:1647-1650`.
- **What:** with `walk` returning no frame, `evading` is cleared and only `rest` and the spawn auras run; the
  `FACTION_RESTORE_REACH_HOME` restore and `EVENT_REACHED_HOME` that `evade_step` runs don't. Rare: the path has two
  points or more.
- **Fix:** run the same three calls as `evade_step` there.

### R3-28. Persistent areas of unlimited duration vanish at once (low, easy)
- **Where:** `src/game/map/spells/effects.rs` `run_untargeted`: `aura_duration(..).unwrap_or(0)`.
- **What:** an area of duration index 21 (-1) gets `ends == now` and is swept the next update. Three spells in the
  data (22011, 22997, 23467), none a player's.
- **Fix:** a far-future `ends` for `None`.

### R3-29. Hunter's Mark is lost on normalized weapon damage (low, easy)
- **Where:** `src/game/map/spells/effects.rs` `weapon_damage`: the attacker-power bonus is added to `base`, which the
  normalized branch then overwrites.
- **Fix:** add the bonus after the normalized roll.

### R3-30. Phase kept through a death, for a creature revived in place (low)
- **Where:** `src/game/map/event_ai.rs:1080-1084` (`death`).
- **What:** mangos's `JustDied` resets the EventAI and its phase. A respawn here makes a new `Creature`, so this
  matters only for one brought back in place (a corpse revived by a spell or a script).
- **Fix:** reset the phase and the disabled flags after the death events.

## Speed

### R3-31. Every heal and every Fade looks at every creature on the map (low-medium)
- **Where:** `src/game/map/combat.rs:2331-2357` (`heal_threat`, from every direct heal and every heal over time's
  tick), `spells/threat_sync.rs` `fade_changed`, `ai::forget` (now also on every logout and map change, R3-6).
- **What:** `map.creatures.iter()` over 23,000 to 27,000 creatures (about 15 ns each, PERFORMANCE.md) per heal: some
  0.35 ms a heal, on the map's thread. A healer with three HoTs ticking in pvp-arena is three of these a second.
- **Fix:** keep, per unit, the creatures holding it on their threat lists (added in `add_threat`/`engage`, removed in
  `forget_by`, `prune_threat`, evade and death), or look through `awake_where` once every creature with a threat
  list is sure to be awake (a passive creature in a fight with no observer isn't, today).

### R3-32. The aura update allocates and clones holders every step (low)
- **Where:** `src/game/map/auras.rs:2395-2433` (`update`, `due_ticks`), `spells/effects.rs` `refresh_area_auras`.
- **What:** every 50 ms, a `Vec` of every player with an aura (all of them: passives) and every creature with one;
  each due tick clones its whole `Holder` (with its effects) twice. Every second, `refresh_area_auras` clones every
  holder of every player and owned creature before looking for party auras.
- **Fix:** a per-unit "next tick or expiry" instant to skip units with nothing due; clone only (spell, caster, item,
  effect) keys; filter holders for area auras before cloning.

## Checked, not findings

- A mana-per-second channel going on with its caster out of mana: mangos's `SpellAuraHolder::Update` does the same
  (`ModifyPower`, clamped; only a health cost ends it).
- A mana leech's `to - now` can't underflow in stock play: `derive` clamps mana to its maximum; no stock leech drains
  energy.
- `gain_rage`'s `rage + gain`: `gain` overflows only for damage near 7e8.
- Client-reachable panics: `CMSG_CAST_SPELL` with an unknown spell or target, an item slot out of range, a swing at
  nothing are all looked up fallibly. No unguarded `u32` subtraction found on client data in the scope.
- derive.rs's health, mana, armor, attack power, crit and block formulas match mangos-classic's `StatSystem.cpp`.

## Verification

- `cargo clippy --all-targets`: clean. `cargo fmt`: run.
- `cargo nextest run --lib`: 1,368 passed. Two tests failed once each in a full run and passed alone (several runs):
  `tests_p3_entity_objects::a_trap_springs_on_a_foe_as_its_charges_and_cooldown_allow` and
  `tests_p3_combat2::the_gm_threat_list_runs_highest_first_and_an_evade_empties_it`, both of the known kind (a
  query, then the map's real-time update, then a check: the second's brute prunes the wolf it can't fight between
  the query and the `.threat` command). `tests_pvp5::silithus_tallies_go_to_those_in_it_and_the_favor_to_its_living_winners`
  likewise once.
- `cargo extended --no-fail-fast --test-threads 4` (WVS_DATA set): 1,657 passed, 0 failed.
