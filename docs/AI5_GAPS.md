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
