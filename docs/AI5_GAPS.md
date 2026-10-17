# AI, scripts and instances: the parity sweep's gaps (2026-10-17)

A module-by-module sweep of Thistle Tea's `core/ai/**`, `core/instance/**`,
`core/instance_script/**` and its dungeon and raid scripts against this
server. Each gap is written down before it is fixed; its state is noted
beside it. Thistle Tea's paths are under `lib/game/`.

The import of Thistle Tea's scripts (`data/tt_scripts.rs`) leaves nothing
out at load (`tests_scripts2::the_real_thistle_tea_scripts_come_in_whole`),
so the gaps are in what the interpreters make of what comes in.

## Movement generators, formations and pools

- M1. Waypoints advance on setting off, not on arriving: a spawn's first
  point is skipped; a pulled or talked-to patroller runs to its spawn and
  skips the point it was bound for. TT: `world/loader/waypoint.ex:74`
  (the start is the point closest to the spawn), `core/ai/bt/mob.ex:1344`,
  `:573` (home is the last point reached).
- M2. Wander points aren't kept within `wander_distance`; no point means
  a random spot at the spawn's height, no path a straight line. TT
  clamps the point and otherwise idles 2-6 s.
- M3. Knockback's horizontal and vertical speeds are swapped.
- M4. Pools: a limit over 1 with chanced members spawns one; no re-roll
  on respawn for chanced or parent pools; event spawns ignore pools.
- M5. Charge teleports the caster; TT runs a path at up to 24 yd/s and
  swings on arrival.
- M6. Formation angles are read as degrees; they are radians.
- M7. Script command 47 (flee) always seeks help; `datalong` 0 is a plain
  7 s flee.
- M8. Fear, confusion and fleeing don't stop the spline under way; root
  and stun don't stop runners; confusion has no anchor; fear's point
  isn't checked against the navmesh.
- M9. Home from evading: no facing packet to the spawn's orientation;
  health comes back on arrival always, where TT heals on leashing and
  only creatures that regenerate.
- M10. Distract lasts its amount in ms, not seconds; no stop or facing
  packets; no exclusions.
- M11. Formations: heading from the leader's old facing; no speed
  matching; no new leader on the leader's death; homes from own spawns;
  respawn-together only on the leader's respawn; flags 0x10/0x20 absent;
  members pulled together call for help.
- M12. Scripted follow: current position, not 500 ms ahead; walks a
  walking leader; no facing; stops when the leader is missing.
- M13. Escorts: point ids taken as indices; straight-line walks; always an
  instant respawn at the end; combat home is the next point.
- M14. Flying splines are written packed (they must be whole points);
  flyers always run.
- M15. Waypoint orientation: 100 is "none" (0 is a facing).
- M16. Script waypoint commands: START_WAYPOINTS' point id read as an
  index; MOVEMENT type 2's repeat and start point ignored; movement inform
  gives the index, not the point id.
- M17. Routes on spawns whose movement type isn't 2 are ignored.
- M18. SMSG_MONSTER_MOVE: TT always sets the run flag 0x100 and packs
  offsets from the destination (to confirm before copying).
- M19. Minor: knockback doesn't interrupt a creature's cast; falling
  corpses uncapped; Stuck goes to the hearth; a follower 100 yd behind
  isn't brought up; idle-step leaders don't lead; first moves spread over
  0-8 s.

## Instances and instance scripts

- I1. A raid's reset with players inside stops its map: they're dropped
  unsaved and can't log back in. TT expires the copy, gives the 60 s
  grace, sends them home, then closes it.
- I2. An object opened by a spell (effect 59, Molten Core's runes,
  Stratholme's gate) never reaches the instance's object-use hook.
- I3. Raid lockouts don't follow the group: no group bind from the
  leader's saves, no bind on entering a bound copy, `belongs` by the
  copy's recorded owner only.
- I4. Players can open doors the scripts drive.
- I5. Doors are forced to their rule on load and every data write; TT
  only operates a door whose rule flips (and sets GO flag 0x1).
- I6. Scarlet Monastery: Mograine and Whitemane's stages are missing.
- I7. Molten Core: the Cache of the Firelord never appears.
- I8. Dire Maul: the Gordok tribute starts at 0, not 6.
- I9. A copy's player limit is hard-coded, not the map's.
- I10. Razorfen Downs: Belnistrasz's idol ritual is empty.
- I11. Scheduling a timer key adds a timer instead of replacing it.
- I12. Saved data reloads without what goes with it (in-progress fields,
  raised chests, kill lists); BRS has no door rule for field 5.
- I13. Group ids are reused after a restart while saved copies keep
  group owners.
- I14. Logging in inside a copy that's gone puts one in a fresh copy;
  TT sends one to their home bind.
- I15. An empty copy lasts 30 min; TT's 5.
- I16. Maraudon: Celebras the Redeemed shows before the Cursed dies.
- I17. Deadmines: the pirates' charge, the Gunpowder event.
- I18. Stratholme: Malown's despawn mid-fight; the Mindless Undead run.
- I19. Gnomeregan: spawned rockets removed an hour after Grubbis; the
  bombs share one walk timer.
- I20. Uldaman: the earthen wake isn't timed; Stoned isn't unselectable.
- I21. A group's leader's orphaned copies aren't adopted.
- I22. A refused portal sends chat, not SMSG_AREA_TRIGGER_MESSAGE, and
  makes up a level line.
- I23. Molten Core: Majordomo's summon (no summoner, despawn 6 at 10 s,
  field 9); Ragnaros's roar emote and zone pulse; Golemagg's reset.
- I24. Dire Maul and BRD text and attack timings.
- I25. Lockout packets: RAID_INSTANCE_INFO on save and expiry;
  RAID_GROUP_ONLY(0) when an eviction is lifted by a worldport.
- I26. Smaller: the reset's anchor; five-an-hour refresh; "saved to
  another" packet; raid bind only on a player's kill; object-use hook
  before the range and alive checks.

## EventAI and creature spell lists

- E1. Creature spell lists: one timer for the list, so only the first
  spell casts; no initial delay; repeat delays read as ms (they are
  seconds); cast targets 0/2/4/5 wrong; cast flags, target params and
  scripts not loaded; a failed cast spends the timer; main-ranged (0x08)
  doesn't hold at range.
- E2. A failed EventAI cast reads as cast, so check-result events never
  retry.
- E3. Creatures summoned by a creature's spell get no AI, and their
  summoner no summon events.
- E4. Health and mana thresholds compared as floats (TT truncates; mana
  with no mana pool is no event).
- E5. LEAVE_COMBAT never fires.
- E6. The death event's target is the tapper, not the killer.
- E7. FRIENDLY_HP's candidates and pick.
- E8. OOC_LOS has no line-of-sight test.
- E9. Timed events fire during the evade walk home.
- E10. Timed events tick every 500 ms; TT every second.
- E11. A script-set spell list survives an evade.
- E12. Aura stacks summed over casters; TT takes the most of any one.
- E13. Friendly missing-buff and friendly-CC candidates (in combat only).
- E14. Script cast flags read with cmangos meanings.
- E15. Pause while stunned: stun only, by the real time passed.
- E16. Smaller: group-member-died on the dead; movement inform when
  dead; summoned-unit order; summoned-despawn on every despawn; helper
  aggro order.

## Creature combat

- C1. Call for help: TT takes helpers within 10 yd that share the faction
  template, whose faction answers calls (flag 0x01, not 0x400), free,
  in sight; tells them 1.5 s later if the caller still fights; they don't
  call on; a 1 s pulse calls friends within `call_for_help_range` while
  more than 10 yd from home; 0x400 factions flee instead. wvs: same entry
  within `call_for_help_range`, at once, chaining.
- C2. Script command 47 always seeks help (M7).
- C3. Reaction modes: passive (flags_extra 0x80|0x20000, static 0x02000000)
  and defensive (flags_extra 0x2) creatures don't aggro on sight; passive
  ones take no victim.
- C4. No creature-against-creature (or pet) aggro on sight.
- C5. Reputation-gated hostility falls back to the faction template.
- C6. Any harmful cast puts the target in combat, Sap from stealth too.
- C7. Stunned or pacified idle creatures still aggro.
- C8. Mind Soothe reads the player's detect-range aura, not the creature's.
- C9. Melee range +1 yd leeway; chase and repath distances.
- C10. Emote and stand state not cleared on combat or restored at home.
- C11. Evade heals on arrival; TT on leashing.
- C12. No facing packets in combat (or home, M9).
- C13. ATTACKSTART at aggro, not the first swing; none on a victim switch;
  no ATTACKSTOP on death.
- C14. No attacker spreading.
- C15. Leash: DoTs extend it; control extends it; shared leash clocks;
  radius max(50, 1.5 × aggro radius).
- C16. Taunt auras leave permanent threat; ATTACK_ME raises to the top.
- C17. Aggro: polled every 500 ms; LOS only on the nearest; 45 yd clamp and
  detection 0 as 20; LOS eye heights +2/+1.
- C18. Victim selection: only the top candidate; a 60 yd-from-home filter.
- C19. A caster's stand-off ignores line of sight.
- C20. No contested-PvP guard reaction.
- C21. Creature charm of players: target and spell choice; ends with the
  charmer's combat.
- C22. Evade strips every aura (TT keeps positive ones for flags_extra
  0x1000 and players' timed ones).

## Conditions

- K1. Item (2) counts the bank.
- K2. Gossip greeting: highest allowed condition id, else text 68.
- K3. Quest taken (9) accepts a failed quest.
- K4. Argent Dawn commission (10) missing.
- K5. Area (4) for a creature source; EventAI with no target uses self.
- K6. Nearby creature (20): around the target; value4 excludes it; value3
  dead only.
- K7. Distance to position (54) missing; inline float values truncated.
- K8. Conditions on `spell_script_target` ignored.
- K9. Object fit (50) and object loot state (49) missing.
- K10. Two-valued logic under NOT and reverse; TT keeps unknown unknown.
- K11. Skill below (29) for an unknown skill.
- K12. Conditions on reference loot rows ignored.
- K13. DB guid (52) for a game object source.
- K14. Nearby player (56): values swapped.
- K15. PvP rank (51) on the wrong scale.
- K16. Reaction (44) missing.
- K17. Has pet (40) missing.
- K18. Minor: LOS eye heights; moving (39) for players; instance data
  outside a copy; escort (25) with no unit; map event targets gone; a
  missing script target is the owner; nearby object (21) around the
  target with the exact radius.

## Script commands

- S1. Flee (47) always seeks help (M7, C2).
- S2. respawn_game_object (9) can't bring in a despawned object, nor for
  its time.
- S3. data_flags: TARGET_SELF applied before SWAP_FINAL.
- S4. activate_object (13): traps fire, doors and buttons and goobers are
  used, the object as source.
- S5. Script casts always report success (E2), cast flags (E14).
- S6. turn_to (35) sends no facing packet.
- S7. Swap-initial with no target loses the buddy search.
- S8. despawn_game_object (81) is remove_object; it should despawn the
  spawn in datalong and bring it back after datalong2 s.
- S9. send_map_event (66): the id is datalong, the data datalong2,
  recipients by datalong3.
- S10. respawn_creature (71): the source, and datalong forces it alive.
- S11. play_custom_animation (89) sends an emote, not
  SMSG_GAMEOBJECT_CUSTOM_ANIM.
- S12. An object's gossip option scripts start (object, player); TT
  (player, object).
- S13. Scripts stop on the source's death and evade; TT's outlive both.
- S14. set_faction (22) ignores datalong2's restore flags.
- S15. zone_combat_pulse (49) bypasses the zone pulse (dungeons only,
  every 3 s).
- S16. Talk ignores the chat-type override; boss emote and zone yell
  ranges; boss whisper and zone emote kinds.
- S17. update_entry (27) is partial.
- S18. despawn (18) ignores the respawn delay.
- S19. remove_guardians (56) removes script summons, not guardians.
- S20. C_SET_REGENERATION has no effect.
- S21. set_equipment (19): 0 unequips.
- S22. edit_map_event (69): 0 clears.
- S23. Smaller: 92's map-wide and pets; 68's entry filter; 67's wander
  radius; 74's permanent flag; 10's unique-summon range and order; 6's
  who teleports; 20/19 distancing; 90 ungrouped; C_LOSE_HEALTH off by one;
  extra target the newest; area trigger alive check and cooldown; nested
  starts slipping a tick.

## Encounters (dungeon and raid scripts)

Root causes shared by many encounters:

- D-R1. Summon despawn types: TT's types 1, 2 and 4 time from the spawn,
  wait out a fight and restart after it; 2 and 5 vanish at death; 7 has
  no timer. wvs: only 2 waits, never restarts; none vanish at death.
  (Ragnaros's Sons, Wyrmthalak's and Nerub'enkan's adds, Willey's
  riflemen, Onyxia's whelps, Jandice's illusions, Angerforge's reserves,
  the Nightmare's druids, the SM trainees.)
- D-R2. A dying summon still counts alive when its summoner hears of it
  (Ragnaros's early emerge, Majordomo's Champion).
- D-R3. A fight's end cancels the creature's timed scripts and its path's
  hold (S13: Emi Shortfuse, the Disciple of Naralex, Herod).
- D-R4. static_flags1 0x8 (unkillable: a floor of 1 health) is ignored
  (Majordomo).
- D-R5. Creature-cast summons get no AI (E3: Lava Spawn, Dream Fog,
  Shades of Taerar, Doom'rel's voidwalkers, Zum'rah's skeletons).
- D-R6. update_entry partial (S17: Lathoric, Balnazzar keeps Dathrohan's
  events in TT).
- D-R7. zone_combat_pulse (S15).
- D-R8. Script emotes: TT sets the emote state for state emotes
  (Emotes.dbc spec_proc, or 0); wvs always sends SMSG_EMOTE.
- D-R9. Talk: the chat-type override (S16); wvs also plays the broadcast
  text's sound (vmangos does; TT doesn't) — kept.
- D-R10. Failed casts count as success (E2).

By encounter (beyond the root causes):

- D2. The last Core Hound's corpse: TT removes it (a TT quirk; vmangos
  keeps it) — kept.
- D5. Guards of dead bosses come back every 5 min (quiet_death's fixed
  respawn).
- D18. The Nightmare's druids may attack pets: the summon step's target
  params aren't copied to the attack target's selector.
- D20. "Nearest hostile player" with radius 0 defaults to 100 yd; TT 30.
- D21. Eranikus: Remulos's quest 8736 escort is missing, so the event
  can't start.
- D25. Mizzle: a script's walk home (movement 7) never fires reached
  home, so the Gordok tribute can't be had.
- D30. The Emperor's rally: friendly creatures in sight (C1).
