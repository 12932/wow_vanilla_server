# Sitrep: the overnight run (2026-10-17 to 18)

What was asked:
- **First,** align wvs with Thistle Tea ([FULL_PARITY_PLAN.md](FULL_PARITY_PLAN.md)), committing and pushing as each
  phase lands. Load tests are capped at five minutes.
- **Then,** the kobold's reach, performance, and robustness ("Elixir's reliability").
- **Last,** a code review.

This file is where the decisions made along the way are written down, so you can override any of them, along with
the questions waiting for you.

## Questions for you

**Answered (2026-10-18):**
- **Wrong passwords:** no lockout. **Account creation:** keep `auto`, with the name as the password. Both are
  deferred until much later (review item 33).
- **#10:** keep swinging on arrival.
- **#18:** keep moving unusable gear at login.
- **#27:** snares stack, as mangos-classic has them: they coexist and the strongest counts. Done.
- **#20:** explained; waiting on your go to switch to Thistle Tea's and mangos's model.

1. **Death's wear, three small differences** (from 2a). Should they be aligned with Thistle Tea? Until you say so,
   they stay documented and unchanged.
   - Spirit of Redemption: wvs wears gear down when the spirit ends. Thistle Tea does at the fatal blow, and only if
     a player didn't strike it.
   - A death by one's own hand: wvs wears gear down, Thistle Tea doesn't.
   - The killing blow: in wvs it can also cost the one-in-200 wear of any blow. In Thistle Tea it can't.

2. **Robustness: what a restarted map does with its players.** Its players are saved and sent to the character
   screen. Should they go straight back into the fresh map instead?
3. **Robustness: should a hung map abort the server,** so systemd or Docker restarts the whole process (OTP's
   top-level "let it crash")? Today the watchdog only logs it, since a hung task can't be killed from inside.
4. **wow_messages' crash bugs.** The robustness work found that wow_messages reserves memory from counts in the
   packet before reading them: a `/who` claiming 4 billion strings asks for 96 GB and aborts the process. It also
   `unwrap`s zlib and underflows sizes in `CMSG_WARDEN_DATA` and `CMSG_BUG`. wvs now guards against all of these
   itself. Should they also be fixed in your fork, or reported to gtker upstream? That would be a public issue,
   which is why I'm asking first.

5. **Knockback on creatures** (settled by bugs3). A knockback no longer moves a creature the server runs: mangos
   ("no non player controlled pre-wotlk") and Thistle Tea agree, so 2b's teleporting creatures is gone. Nothing to
   answer unless you want them moved.

6. **Water** (from 2g). Thistle Tea ends what water breaks only by the client's swimming flag. wvs now also goes by
   the terrain's liquid, as mangos-classic does. Keep it, or match Thistle Tea exactly (revert `c99aab1`)?
7. **Locked doors and buttons used by hand** (from 2g). Thistle Tea refuses them without the key, wvs opens them,
   and mangos trusts the client. Follow Thistle Tea?
8. **Failed openings.** Thistle Tea holds back `SMSG_SPELL_GO` for a failed opening, which changes the cast
   pipeline. Should wvs do the same?

9. **Battleground creatures' home guard** (from 2c). wvs used to keep battleground creatures within 60 yards of
   home when choosing a victim. Thistle Tea has no such guard, so it's gone. Should battleground creatures keep one?
10. **A creature swings only once it has arrived** (from 2c). This goes beyond Thistle Tea, which swings as soon as
    it is in reach. The client shows a moving creature slightly behind where the server has it, so swinging on
    arrival keeps a creature from hitting from where the client still shows it out of reach: your kobold report.
    Keep it?

11. **Packet order in an update** (from bugs3). An update's blocks go at its end, after any packet sent during it.
    A packet naming an object created in that update (a monster move right after a player arrives) can reach the
    client first. A general fix (per-observer ordering, or flushing before direct sends) touches the hot path. Do
    it?
12. **Gates of Ahn'Qiraj and the Cenarion Favor** (from bugs3). Thistle Tea's zone list leaves out zone 3478;
    mangos-classic includes it. I followed Thistle Tea.
13. **Charged items** (found by bugs3). Spell charges aren't saved, so a part-used charged item comes back full after
    a relog, a mail or an auction. Fix it?

14. **Summons of despawn type 0 with a time** (from 2d). wvs keeps vmangos's meaning, so the summon waits out a
    fight. Thistle Tea despawns it at its time, even mid-fight. Follow Thistle Tea?
15. **Script teleport (6) with a player source** (from 2d). wvs teleports the player, as vmangos does; Thistle Tea
    does nothing. Keep vmangos's?

16. **Eranikus evades in The Nightmare Manifests** (from 2e). Under load, Eranikus evaded mid-fight, which failed
    the quest: his victim Remulos was out of his melee reach, and he doesn't move in combat. The test now beats him
    down quickly. Is wvs's evade-when-unreachable right for him? What Thistle Tea does here isn't confirmed.
17. **Quest 2078** (from 2e). Thistle Tea starts its follower from a gossip option ("[PH] Insert key"); wvs starts
    it on accept. It isn't an event quest, so I left it. Port it?

18. **Gear that existing characters can no longer use** (from bugs4). Equipping now needs the right skill:
    - proficiency comes from learned spells, not from level;
    - Dual Wield must be trained;
    - an item's required skill, rank and spell are checked.

    At login, worn gear a character can't use goes to their bags, or by mail if the bags are full, as mangos does.
    Nothing is destroyed, and a character can put it back on once they train the skill. Keep this, or leave such
    gear worn until it is taken off?
19. **A pet's Stay from the action bar** (from bugs4). It still stops the pet's attack, as mangos-classic and
    mangos-zero do; vmangos and Thistle Tea keep the fight (script command 88 now keeps it). Follow them?
20. **The player combat timer** (from 3b). wvs keeps a player in combat 6 s after their last action. Thistle Tea
    uses 5 s for PvP and for being hit, and 0 for hitting a creature. Move to Thistle Tea's model? It changes how
    soon players leave combat after a kill.

21. **The grey quest "!"** (from 3c). It shows for quests up to 4 levels below the player. Thistle Tea shows it at
    any gap; mangos's default is 7. 4, 7, or unlimited?
22. **Abandoning a quest** (from 3c) keeps its collected quest items, as mangos does; Thistle Tea destroys them.
    Which?
23. **An untapped creature's soul shard** (from 3c). Draining one still gives a shard, as mangos's
    `isAllowedToLoot` does; Thistle Tea gives none. Which?
24. **Model height from a mislabelled column** (from 3e). dbc.sqlite's `CreatureModelData` columns are shifted by one
    from 1.12's: `collision_height` holds the width. wvs now reads the height from `mount_height`, as mangos-classic
    does; Thistle Tea reads the width. Is the shift known? It affects anything else reading that table.
25. **Fixing the wow_messages fork** (from 3f and robustness). An agent is fixing the layouts wvs hand-encodes, and
    the crash bugs, in your fork. Nothing is reported upstream without your say.
26. **The reconnect protocol version** (from 3f). Both 2 and 3 are accepted. Which does the 1.12.1 client send?

27. **Snares** (from 3a). Answered: they stack, as mangos-classic has them (`6aec095`). Each stays on as long as it
    lasts, and the strongest slows.
28. **A charm or Mind Control ending** (from 3a). Should the freed creature's cast or channel be interrupted
    (Thistle Tea), or carry on (mangos's `Uncharm`, and wvs today)?
29. **Talent reset decay** (from 3d). After one reset, should the cost fall back to 1 gold after a month (Thistle
    Tea), or stay at 5 gold (mangos, and wvs)?
30. **Melee attack-speed slows** (from 3d). They don't stack, as vmangos and Thistle Tea have it (done); mangos-classic
    multiplies them. Keep? The snare answer left them as they are.

31. **A creature-owned totem when its owner dies** (from 3i). Should it outlive its owner until it expires (Thistle
    Tea), or go with it (mangos, and wvs)?
32. **A scripted follower's evade** (from 3i). Should it evade where it stands (Thistle Tea), or to its place by its
    leader (mangos)? wvs walks it to its spawn.

33. **A teleport within the map** (from 3l). Should one that isn't a spell's end the player's fight? mangos's
    `TeleportTo` does, unless `NOT_LEAVE_COMBAT` is set, and Thistle Tea does, holding the pet until the client
    acknowledges. wvs doesn't.
34. **A transport's deck offset** (from 3l). wvs takes any offset the client sends. mangos-classic ignores a movement
    whose offset passes 50, 50 or 100 yards (`VerifyMovementInfo`); Thistle Tea refuses the boarding. Which?
35. **A second login** while the character is still in the world (from 3l). wvs kicks the first session and logs in
    afresh. mangos-classic hands the character to the new session; Thistle Tea refuses with 0x41. Which?
36. **The Warsong flag on a mount** (from bugs5). Does the 1.12 client let a mounted player pick it up? wvs now
    refuses it, as Thistle Tea does; mangos-classic doesn't check.
37. **Lower ranks of rage and energy spells** (from 3j). Should they leave the spellbook when a higher one is
    learned, as in mangos? That needs a "known but inactive" spell state. wvs keeps them.
38. **vmangos's spell corrections** (from 3j). wvs applies vmangos's `spell_mod` and `spell_effect_mod`, where
    Thistle Tea reads the raw DBC. So Concussive Shot and Piercing Howl are dazes, not snares; Blessed Sunfruit
    Juice and Alterac Spring Water aren't Well Fed; and Blood Siphon and Blood Funnel are a heal and a hit, not
    funnels. Keep vmangos's?
39. **New weapon skills** (from 3j). wvs starts them at their cap; mangos and Thistle Tea start them at 1. Changing
    it means every level-60 combat test needs its skills raised. Change it?

40. **How a creature a player controls heals** out of combat (from 3p), whether charmed or a pet: Thistle Tea's spirit ×
    4, mangos-classic's 13% of its health, or wvs's current twentieth?

41. **Cancelling Stealth while Vanish is on** (from 3q). Should both come off, as Thistle Tea has it? mangos's
    `CMSG_CANCEL_AURA` takes only the spell named, so Vanish keeps the rogue stealthed. wvs follows mangos for now.

42. **An interrupt on a creature immune to silence** (from 3m). Should it still lock out the school? mangos-classic
    locks it; Thistle Tea, following vmangos, spares the creature. wvs locks it.
43. **A fear that damage may break** (from 3m). Should a fear without the damage-interrupt flag break on damage by
    chance, as mangos-classic does? wvs doesn't break it.

## Decisions made (override any)

1. **The kobold's reach.** wvs let a unit swing from a yard beyond its melee range. Thistle Tea (`in_melee_range?`)
   and mangos-classic (`CanReachWithMeleeAttack`) allow none. That yard is the "couple of yards too far" you saw: the
   kobold hit you from where your own swing was out of range.
   - **What changed:** the yard is gone. Two units both running (not walking) get mangos-classic's `MELEE_LEEWAY` of
     8/3 yards, as the 1.12 client does.
   - **Where it goes beyond Thistle Tea:** Thistle Tea has no leeway. Without it, a player chasing a running creature
     would get "out of range" from the server where the client lets them swing.
   - **Distance:** stays 3D, as Thistle Tea measures it. mangos measures a creature's reach in 2D.
2. **How the work runs.**
   - **Isolation:** each stage runs in its own git worktree (`<worktree>/<stage>`, branch `parity/<stage>`).
   - **Merging:** I merge each stage into `master` when the full suite and clippy are clean, then push.
   - **Records:** the agents don't edit THISTLE_TEA_DIFFERENCES.md or this file. I do, at each merge, so the branches
     don't conflict.
3. **A whole-repo `cargo fmt`.** The toolchain's rustfmt (1.10) formats 67 files differently from how they were
   committed. That is one mechanical commit (`ec87804`), made before the stages branched, so each agent can run
   `cargo fmt` without churning files it never touched.
4. **The matrix's not-applicable rows.** Phase 1a lists all 8,850 Thistle Tea tests in `docs/parity/matrix.csv`.
   I marked 26 of them not applicable: the web server, telemetry, the developer seed, the namigator NIF, and the
   Elixir layering rules. The rest are being mapped by area.
5. **wow_messages misreads 1.12's stop packets.** A STOP `SMSG_MONSTER_MOVE` ends after its move type in 1.12
   (mangos-classic's `WriteStopMovement`), but the fork read on into spline flags. So wvs's test client and peon
   dropped every creature stop as unparsed; the real client was never affected.
   - **The fix:** fork `e904e8e`, repinned together in rustigator `a1c3a82`, wvs `2bed07d` and peon `ca35164`, so
     they share one `wow_world_base`.
   - **Tests that changed with it:** three wvs tests waited for "a move" and had been passing because stops never
     reached them. They now skip stops.
   - **Left as it was:** `SMSG_MONSTER_MOVE_TRANSPORT` has the same problem and is still read the old way. Nothing in
     wvs sends one yet.
   - **Failures it didn't cause:** six `cmsg_update_account_data` tests in the fork fail with "incomplete deflate
     stream". Their code is unchanged by this fix.

6. **2a's choices, as Thistle Tea has them:**
   - Lay on Hands takes only the percentage healing modifiers; it used to take the flat bonus too.
   - Healing Way's gain multiplies the heal with its flat bonus, in Thistle Tea's order.
   - A dispelled passive from gear (a set bonus) comes back at the next gear sync.
   - A dispel now takes passive auras of its type. vmangos's own `EffectDispel` has no passive check either, so the
     old note that "vmangos skips them" was wrong.

7. **Creature size and reach** (found while mapping), aligned with Thistle Tea's mob builder.
   - **The size:** the template's for its display, else the display's own (CreatureDisplayInfo's scale times its
     model's).
   - **Reach and radius:** they grow from the display's own size. Before, they were multiplied by the whole scale,
     so Onyxia reached 47 yards rather than 26.
   - **Where mangos-classic differs:** it would give Onyxia 23.4, the model's reach at her template's own scale. I
     followed Thistle Tea.
   - **What you'll notice:** creatures whose template sets no scale (Hogger, Kobold Miners) now show at their
     display's size, 1.15, instead of 1. The kobold's reach didn't change; your kobold report was the yard of slack
     in decision 1.

8. **Robustness's thresholds**, all in `docs/RELIABILITY.md`:
   - **A creature** is restarted from its spawn at its 2nd panic within 5 minutes, and quarantined at its 4th.
     Quarantine means it lies dead for 10 years; a summon is simply removed.
   - **A player** whose packets panic 10 times in a minute is saved and sent to the character screen.
   - **A map** restarts at 200 update-step panics in a minute. A restart loses what only the map held:
     - the open world's dead respawn;
     - vendor stock refills;
     - battlegrounds, world PvP, the arena and escorts start over.

     Dungeon copies keep their dead.
   - **The packet guards:**
     - an auth session is at most 8 KiB;
     - a declared decompressed size is at most 0xFFFFF;
     - a GM command's coordinates must be finite and within ±100,000.

9. **2b's choices:**
   - **Run mode:** every monster move carries it, walking creatures too. mangos-classic always sets it ("client has
     strange issues without that flag"), and so does Thistle Tea.
   - **Points:** a move's points are offsets from its destination, which the old code got wrong for 1.12.
     Intermediate points within 0.7 yards of the destination are left out, as mangos-classic does.
   - **Flyers' splines** are hand-encoded, in whole points.
   - **Distract** affects creatures only.
   - **Charge:** a rooted player mid-charge gets the root at once; Thistle Tea defers it to arrival.
   - **Pools:** an event change re-picks a pool, removing members that are out even mid-fight, as Thistle Tea does.
   - **Healing on leashing:** a creature heals as it leashes, and only if it regenerates, rather than on getting
     home (M9 and C11).

10. **2g's choices:**
    - **Create-item targets:** a creature's create-item effect lands on the unit it is cast at. A player's own
      untargeted create-item stays theirs, even if the client names another unit, so a craft can't go to a selected
      friend. That is stricter than Thistle Tea.
    - **Stricter than before:**
      - a key carried in the bags opens a lock only when it is the item cast;
      - lock spells reach 5 yards;
      - ordinary chests despawn once emptied, rather than refilling at once.
    - **Speed acknowledgements** are no longer relayed to watchers, and no longer set the player's movement.
    - **Resurrection offers:** a second offer is ignored while one is pending.

11. **2c's choices:**
    - **Facing packets** go out only at a halt, a due swing, or bad facing, not every tick.
    - **A harmful spell** starts a fight as it lands on each target, so bolts start it on landing, and areas put
      every creature hit into the fight. Script-triggered casts start none.
    - **Evade and healing:** creatures that don't regenerate stay hurt after an evade.
    - **Leashes:** a summon shares its summoner's leash clock only if the summoner is already fighting.
    - **Charmed players:** whether one may cast goes by wvs's full cast check.
    - **Speed changes:** a possessed creature's speed acknowledgement is relayed to those who see it, as
      mangos-classic does. A player's own isn't (2g, as Thistle Tea).
    - **Merge note:** 2c's chase-facing (a target) and 2b's arrival facing (an angle) are now two fields of a walk.
      A move facing a target is sent as such; otherwise it is sent with its final angle.

12. **bugs2's choices:**
    - **Creature costs:** creatures don't pay rage, energy or mana they don't have, as in Thistle Tea. mangos-classic
      makes them pay, and refuses its rage and energy creatures.
    - **Periodic triggers** (Hellfire, Plague) aim about the unit the aura is on, credited to the aura's caster, as
      Thistle Tea does.
    - **Immune flags:** both immune-to-player (0x100) and immune-to-NPC (0x200) are checked. 1,053 templates, mostly
      town NPCs, can't be attacked by players now.
    - **Health regeneration auras:** percentage ones work only out of combat, as both references have it.

13. **bugs1's choices:**
    - **Unlearning skills** goes by the client's SkillRaceClassInfo, so First Aid, Cooking and Fishing can't be
      unlearned, as in mangos and Thistle Tea.
    - **An item that starts a quest** is taken on accept, unless the quest gives it or asks for it, as mangos-zero
      and Thistle Tea do (mangos-classic doesn't take it).
    - **A freed charmed creature** still tags itself for its former master, as Thistle Tea has it.
    - **Merge note:** traps now have their lock, as mangos's `GetLockId` gives it, which Disarm Trap needed after the
      lock rules from 2g.

14. **bugs3's choices:**
    - **`SMSG_GROUP_LIST`** ends with a byte, as Thistle Tea and mangos-classic send it (mangos-zero omits it).
    - **The equipped-item-class cast result** carries the real inventory-type mask, as mangos-zero does; Thistle Tea
      sends 0.
    - **Ghosts** see every player, as in Thistle Tea; mangos limits this. Ghosts see only creatures within 45 yards
      of their corpse.
    - **Single-target auras** end when their caster leaves the world, as mangos and Thistle Tea have it.
    - **Spawn flags:** disabled spawns (0x2) stay out, hidden ones (0x40) are concealed, and dead ones (0x80) lie as
      corpses.
    - **Merge note:** 2g's resurrection request sent a player caster's name. mangos-classic sends it empty (the
      client knows it), as bugs3 had it, so the name goes empty and 2g's test was corrected.

15. **2d's choices:**
    - **Object scripts:** `gameobject_scripts` start on the user, at the object, as vmangos and Thistle Tea do. For
      example, the Blastenheimer cannon's self-cast lands on the player.
    - **`update_entry`** keeps the level when it is in the new template's range, else rolls one.
    - **Emotes and whispers:** monster emotes, boss emotes and whispers carry the target's guid. The raid boss
      whisper is hand-encoded, since wow_messages lays it out without the speaker's name.
    - **Searches:** a buddy search with no radius looks 30 yards.
    - **Merge notes:**
      - **Object menus:** 2d and bugs1 both made them selectable. I kept bugs1's selection, which needs a living
        player in reach, and 2d's dispatch, which runs on the player.
      - **The Molten Core rune:** 2d and 2g both fixed the lock check for the rune and its Quintessence, and I kept
        2g's.
      - **`update_entry`'s size:** it now uses the shared creature-size fix.

16. **2e's choices:**
    - **Escort time limit:** every escort runs under its quest's map event, with Thistle Tea's 30 minutes, and fails
      past it. Before, wvs escorts had no limit.
    - **Tuten'kash's death** settles his encounter's field, as vmangos and mangos do; Thistle Tea never sets it.
    - **A raised chest emptied before a reload** comes back after its spawn's respawn delay, with fresh loot, as
      Thistle Tea's chests do.
    - **Instances by group:** a group takes over its leader's instances when it forms, as Thistle Tea's
      `group_changed` does; a disbanded group leaves them ownerless.
    - **Merge notes:**
      - **Passive creatures on a scripted walk** (Eris Havenfire's peasants in 7622) were stopped by 2c's combat
        rules. A peasant shot by the archers enters a passive fight, and when its foe died it walked home to its
        summon point, losing its walk, so only the first wave reached the light. Now such a creature ends its fight
        where it stands and walks on. Thistle Tea's exact behaviour here isn't confirmed; this is what makes the
        quest work.
      - **Keys cast at locks:** 2e and 2g both fixed them. The lock code now takes the object before the item, as
        vmangos's `EffectOpenLock` does, so a key cast at a door opens the door.
      - **Keys in the bags:** a key carried but not cast doesn't open a lock (2g's rule, as Thistle Tea has it).

17. **bugs4's choices:**
    - **No class starts with Dual Wield:** vmangos's trainers teach it, to rogues at 10 and to warriors and hunters at
      20.
    - **A test world without the Cloth spell** (9078) in its data treats everyone as proficient, as Thistle Tea's
      `Proficiency.all` does.
    - **Weather sounds** use vmangos's and Thistle Tea's thresholds (0.3, 0.6, 0.9).
    - **A dead master's guardian** is kept while it fights, and as a corpse.
    - **Extra attacks** swing during a cast, as vmangos and Thistle Tea do; mangos-classic holds them.

18. **Phase 3's choices** (3b, 3c, 3e, 3f; each agent's full list is in its merge commit):
    - **Unkillable:** one rule, Thistle Tea's, where a script's floor of 0 lets an unkillable creature die. 2d and 3b
      had each written one.
    - **Combat (3b):** auto-attack alone no longer keeps a player in combat. Block value and parry haste follow
      mangos. Units flagged unattackable (0x2, 0x10000, 0x02000000, or on a taxi) can't be attacked.
    - **Quests (3c):**
      - givers must be alive and carry the quest-giver flag, and the player must be alive;
      - auto-complete quests turn in without the log;
      - sharing needs a shareable quest;
      - error codes follow mangos.
    - **World (3e):**
      - the taxi mask is translated as the client reads it (node N at bit N-1), with stored masks unchanged;
      - new characters know their race's flight points;
      - the bank's seventh bag slot is gone;
      - environmental damage resists by vmangos's buckets.
    - **Network (3f):**
      - root packets now carry a packed guid; before, a real client was never rooted;
      - AUTH_FAILED is sent for an unknown account too, as Thistle Tea does;
      - several packets are hand-encoded, which wow_messages can't read yet.
    - **Merge notes:** escort quest walks now wait for the escortee to come back as a quest giver before turning in.
      3c's giver check needs the quest-giver flag, which an escort clears while it runs.

19. **3a's and 3d's choices:**
    - **Kick and Counterspell** stop only what a silence could (interrupt flag 0x8 and prevention 1, mangos's
      `IsInterruptible`), so they can't stop a Hearthstone. On an interrupt, the cast result goes before the failure,
      in Thistle Tea's order.
    - **Run speed** follows mangos's `UpdateSpeed`: when mounted, only the mount's bonus counts; the worst slow
      multiplies; a normal-speed aura caps the bonuses.
    - **Damage-over-time timing.** DoTs and ground spells keep their own cadence through late updates. Before, they
      lost their last tick: Corruption did 3 of 4, and Blizzard 7 waves of 8.
    - **Stealth's flag** goes in the visibility byte. Before, it was written into the form byte, so a prowling cat
      read as Travel Form.
    - **Parry** needs a weapon in hand, as mangos has it.
    - **AFK** is refused in combat, as mangos has it, and party members see AFK and DND.
    - **Pet names** may be in any one script, not ASCII only.
    - **Merge note:** the 3f, 3d and 3a tests that waited for a parsed unroot now look for its frame, since its
      packed guid is hand-encoded until the wow_messages repin.

20. **Performance's hook rules** (docs/PERFORMANCE.md). The update now looks only at creatures a player sees or that
    are fighting, and only at objects and auras that changed. New code must go through the hooks:
    - `combat_mut` or `note_fighting` to put a creature in a fight;
    - `map.rare.*` for charges, flights and calls for help;
    - `caster_mut`, `auras_mut` and `gameobject_mut`.

    Debug builds, and so every test, name a missed hook. Release builds would silently skip that creature.
    - **Other choices:** dependencies build at opt-level 2 in dev, which costs one rebuild. An optional `pprof`
      feature adds a profiler.
    - **Left for later:** four looks through every creature remain (despawn timers, regeneration, pets, the
      creature half of flushing values). So does the 2 KB `Creature` struct, whose split would make them cheap.
    - **peon's release binary** was built before the appearance fix, which is why 11-19% of bots failed character
      creation in these runs. Rebuild it.

21. **3i's choices:**
    - **EventAI actions** run their steps at once and ignore step delays, as Thistle Tea does. No row in the data
      has one.
    - **Command 83** now credits the step's creature or object. Before, it credited entry 0, so the 11 rows using it
      gave no quest credit.
    - **Pets** choose targets as mangos and Thistle Tea do: their own attackers, then their master's victim, then
      their master's attackers.
    - **A pet recalled to Follow** takes no new fight until it's back.

22. **3l's choices** (world entities):
    - **The known-spell check** is now on the client's cast request only. Scripts and a possessed player's AI may
      cast a spell the player hasn't learned.
    - **Kept mangos's over Thistle Tea's** (rows marked wrong-for-1.12):
      - a charmed summon still runs out of time;
      - a dead master's pet may still cast at their word;
      - a pet on Stay answers an attacker from afar (mangos's `PetAI::AttackedBy`), though of its own accord it takes
        on only what is in its reach.
    - **Merge note:** pets choose targets by 3i's order, with 3l's Stay reach, and pass over breakable crowd control.

23. **bugs5's choices:**
    - **Charged modifiers** (Clearcasting, Presence of Mind, Nature's Swiftness) are spent only by a cast they change,
      as Thistle Tea has it. Before, a harmful hit could use up a fresh Clearcasting.
    - **The Warsong flag** also refuses players who are mounted, crowd-controlled, feigning death, possessed or in
      Spirit of Redemption, as Thistle Tea does. mangos-classic leaves these to the client.
    - **Mana from a periodic energize** makes no threat, as in Thistle Tea. mangos-classic gives half of it.
    - **A dead target of a tick** runs none of its own procs, as mangos does.
    - **A totem** takes its shaman's modifiers when summoned, not a second later.

24. **3k's choices** (inventory, groups, social, guilds, trade, auctions, PvP, instances):
    - **Error codes:** where Thistle Tea and mangos send different codes for a refusal, wvs keeps mangos's.
    - **Turn-ins** plan the whole change, as Thistle Tea does: the slots that taken items free count, and items in the
      bank aren't taken. mangos checks each reward alone, before taking anything.
    - **A two-hander** may put the off hand into the backpack slot it just left (Thistle Tea). mangos checks for room
      first.
    - **A deleted guild rank's members** move to the lowest rank left (Thistle Tea). mangos leaves them on a rank that
      no longer exists.
    - **Duel bounds** are Thistle Tea's 75 and 70 yards, not mangos's 50 and 40.
    - **The dead:** creatures drop a dead player from their threat lists at once.
    - **Merge note:** a script's data add (mode 1) wraps as an unsigned 32 bits (3k, as mangos and Thistle Tea), and
      any mode other than set, add or take away is still refused (master, as Thistle Tea's `ScriptStep`).

25. **3j's choices** (the loader, on the real data):
    - **vmangos's spell corrections win** over the raw DBC (question 38).
    - **Healing Way's 18%** truncates in f32 (235, as mangos), not Thistle Tea's 236.
    - **Merge note:** 3j and an earlier stage both scaled a target's flat damage taken (Dampen and Amplify Magic) by
      the spell's coefficient. The merge keeps 3j's rules (a creature's spell takes the whole unless its table says
      otherwise, and Decimate's kind ignores the target's modifiers) and the earlier stage's cap: a cut takes at most
      half the damage, as Thistle Tea's `DamageReceived` has it. mangos has no cap.

26. **3h's choices** (encounter scripts):
    - **A script's evade** (command 33) acts only on a living creature, as Thistle Tea's does. Before, a Core Rager's
      leash found Golemagg's corpse, "evaded" it, and the instance reset both ragers.
    - **Summoned creatures** hold their template's weapons, as a spawn does.
    - **Ties in threat** keep the current victim, as Thistle Tea's `Threat.reselect` and mangos do. So Shazzrah keeps
      his victim after Blink's threat wipe, though his script says to attack a random player.

27. **The test suites** (your ask: unit tests in a minute or two). Details in `docs/TESTING.md`.
    - **Fast:** `cargo nextest run --lib` runs the 1,201 tests that don't need the real world, in about 25 s. It took
      about 10 minutes before.
    - **Extended:** `cargo extended` runs every test (1,450) in about 3 minutes. It runs before every merge and push.
    - **Marking:** a test that reads the real world is marked `#[ignore = "extended"]`. If one is left unmarked, it
      fails in the fast suite with a message saying so, so it can't slip in.
    - **The world's snapshot:** the real world loads once per test build and is shared through a snapshot, keyed on
      the databases and the test binary, so a test's load takes 0.9 s rather than 8.4 s. The long walks run in
      parallel shards.
    - **Left as they are:**
      - The walks still poll in real time, so a much faster machine could outrun them.
      - Compiling the crate at opt-level 1 for the extended suite would bring it to about 2 minutes. But a dozen walks
        fail at that speed, for the same reason, and it costs a second build.
    - **One assertion's timing changed:** Ragnaros's check waits for the map's next update, where his summoned
      events run. It failed every time on an idle machine.
28. **Quest 1560 (Tooga)** wasn't a server bug. The escort walk gave each walk a fixed number of looks, while Tooga's
    progress went by the clock, so on a quiet machine his 3,300 yards outlasted the walk. The walk now counts the
    map's updates, and a follower keeps up with its lead.
29. **an agent CLI** now runs the stages (your ask), on Opus 5.5.

30. **Round 3 so far (3m to 3r).** These rows keep mangos-classic's behaviour where Thistle Tea's isn't 1.12's (marked
    wrong-for-1.12):
    - **A weapon swap in combat** resets only the melee timers. The ranged timer and a queued Auto Shot run on.
    - **A scripted flight** with no flight master costs nothing.
    - **Abandoning a quest** doesn't give back the item that started it. Thistle Tea gives it back.
    - **A creature's Bear Form** is always model 902. Thistle Tea picks one by the creature's own model.
    - **Food** heals at its own period in a fight or out. Thistle Tea folds it into the 2 s tick, out of combat only.
    - **A pet's focus** comes back at 12 every 2 s, mangos's rate. Thistle Tea gives 25 every 4 s.
    - **A disarmed unit** can't parry, even with an off-hand weapon.
    - **Merge note:** 3q renews a parent's linked auras on a refresh, and 3o's test has a link back to its parent.
      Together they recursed until the stack overflowed. A refresh made by a link no longer renews its own links.
31. **The agents ran the machine out of memory** (30 GB). the orchestrator stopped all seven agent agents when memory ran
    short. Their committed work is merged above, and their branches and worktrees are kept. How to resume them, with
    fewer at once and a cap on each agent's memory, is waiting on you.

32. **Tests that race the map's own clock** (found merging round 3; for the code review). The harness's tests
    query a map while its own 20 Hz update runs on real time. Three tests failed today only because an update
    did or didn't land between two of their queries:
    - Ragnaros's spawned events;
    - the gear's auras after a login;
    - the lava test's health, which a stats resync reset.

    Each now waits for what it checks. The class remains, though, and a much faster or slower machine will find more
    of them. **The fix** would be a harness mode in which a test steps the map itself, with no update loop of its
    own. That is a few days' work across the harness and its tests, so I left it for your say.

33. **Code review, first findings** (auth, connections, sessions, the store):
    - **Item and money moves are saved apart** (fixed, `70cd32f`). A trade changes two characters, and mail and auctions
      move items between a character and a record. Each side is saved on its own time: the record at once, a
      character at its 5-minute save or logout. A crash in between, such as an abort, the OOM killer or a power cut,
      duplicates or loses the item. mangos saves both sides in one transaction, and so will wvs.
    - **A failed reconnect proof tells the client it is banned** (fixed). It is now answered as Thistle Tea answers it
      (`FailUnknown0`) and closed. mangos closes without an answer.
    - **No lockout after wrong passwords.** Nothing slows a password guesser beyond the cap on open connections.
      mangos has `WrongPass.MaxCount`, off by default. Add one, per account and per address, with a back-off? That
      is your call, as it's a policy.
    - **A save that fails is lost** until the character's next save, after three tries if the database was busy. In
      WAL mode with a single writer that should only happen with a full disk. A failed logout save loses the session
      since the last save. Keeping failed saves to retry is possible, but the right answer to a full disk is an
      operator alert, so I've left it.
    - **The fix for saves apart:** `Store::save_together` writes the characters and records involved in one
      transaction, retried whole and never split. It covers:
      - a trade;
      - mail sent or taken: its item, its money, a letter made of it, and cash on delivery;
      - an auction put up, bid on, bought out, cancelled or swept;
      - a charter or emblem bought.

      A test for each reads both sides back with no other save, and fails without the fix.
      - **Left, minor:** founding a guild saves the guild apart from taking the charter. A crash in between leaves a
        guild and an unused charter, but nothing of value is copied.
      - **Left, minor:** a deleted character's top bid is cleared without a refund, as before.
    - **GM account commands miss lowercase names** (fix queued). `.ban alice 10` answers "No account alice." because
      accounts are stored in capitals and the command matches exactly. `.unban` and `.account gm` have the same bug.
    - **`creation = "auto"` is the shipped default.** An unknown name gets an account whose password is the name.
      That's right for development and load tests, but an open server should set `existing`. Change the default and
      have the load tests set `auto`? It's your call, since it changes how a fresh server behaves.
    - **Sound as they are:**
      - the handshakes (timeouts, size caps, panic guards);
      - the outbound budget and backpressure;
      - the rate and bad-packet limits;
      - the session-key cache's bounds;
      - the supervision: restart, quarantine and back-off, the panic hook, and unwinding in release;
      - the registry's locks, always taken in the same order;
      - the damage, absorb, block and timer arithmetic, which is bounded wherever I looked.

34. **3p's choices** (combat and power, finished):
    - **A polymorphed player** heals a tenth of their health a tick, in a fight too, as Thistle Tea has it. mangos
      has no rule of its own; Polymorph's tooltip says the target "will regenerate very quickly".
    - **Damage sharing** (Blessing of Sacrifice, Soul Link) now takes its share of swings, shots and ticks too, after
      shields, as mangos's `CalculateDamageAbsorbAndResist` does. Before, only direct spells were shared.
    - **Burns and drains** take only a pool the target shows, from the living. A burn with a multiplier of 0 burns
      mana and does no damage. A drain gives nothing back for a power the caster doesn't use.
    - **Costs:** a percentage cost is added to the flat one instead of replacing it, and god mode pays nothing.
    - **Silence and pacify** go by the spell's prevention type, so a wand's Shoot works under silence.
    - **A flaky test, left as it was:** `tests_p3_ai_scripts::a_checked_event_whose_actions_fail_fires_again_until_they_succeed`
      failed 1 run in 12, at different points.

35. **3q's choices** (auras, finished):
    - **Periodic ticks** run one an update, on the aura's own cadence, as mangos's `Aura::Update` does. After a
      stalled update, the missed ticks follow over the next updates, so none is lost. Thistle Tea runs them all at
      once.
    - **A player's cast is checked again as it ends:** range with mangos's tolerance (a tenth, at most 3 yards),
      facing, life, and still a foe. One that fails there costs no mana and starts no cooldown.
    - **Ignite** keeps one burn a target, its first mage's, whoever crits after.
    - **A victim killed by a blow** procs only its death, as mangos's `canProcVictim` has it. So Blessed Recovery
      doesn't go off on a killing crit.
    - **Death clears diminishing returns**, as mangos's `ClearDiminishings` does.
36. **3n's choices** (the player, finished):
    - **The ammunition chosen** is kept over a logout, in the character's saved state.
    - **A vendor out of reach** is told so. A broken weapon's enchantments don't go off.
    - **A scripted route** is refused in a fight or far from its first node.
    - **A shared quest's offer** ends with the sharer's quest, group or presence.
    - **Starting items** come in whole stacks.
    - **The lava test's fragility had a cause:** a gear sync worked a player's stats out afresh every time. It now
      does so only when an item breaks or is mended.
37. **3o's choices** (the loaders, finished):
    - **A tamed beast** owes its spells' training points.
    - **A demon** keeps the name it drew, kept in its saved state.
    - **A player's guardian** keeps its template's flags.
    - **Not copied from Thistle Tea**, where mangos sets nothing: Ghost Wolf's 0.8 scale and reach, and a
      disguise's scale.

38. **3m's choices** (spells and classes, finished):
    - **Silence and pacify** go by the spell's prevention type, as mangos-classic has it. Pacify stops a wand's
      Shoot, and neither stops the Hearthstone.
    - **Lower ranks stay known** beside a higher one, as mangos keeps them. Thistle Tea drops them; this is
      question 37.
    - **A cooldown that waits for its aura** holds its category's other spells too.
    - **A heal's threat** goes by `spell_threat`, and a spell of no threat makes none.
    - **Combustion and its crit** come and go together.
    - **A wow_messages bug, found and left:** `CMSG_CAST_SPELL` reads a corpse's GUID after the location fields,
      where 1.12 sends it before them. Fixing it means reading the packet by hand, or a fork fix like the others.

## Progress

| Stage | State | Commit |
|---|---|---|
| The kobold's reach | done | dd2d397 |
| 1a: the test matrix, listed | done | b50233b |
| 1a: the matrix, mapped (entity, ai, spell, combat) | done: 573 of 4,481 applicable covered (13%); findings in docs/parity/FINDINGS.md | efcd9d3 |
| 1a: the matrix, mapped (core rest, world, network) | done: all 8,850 mapped, 1,781 of 8,429 applicable covered (21%) | f9bad00 |
| Creature size and reach | done | c5e1440 |
| Bugs from the mapping, batch 1 (23) | merged | 25ecb91 |
| Bugs from the mapping, batch 2 (18: spells, power, attack rules) | merged | d2dd886 |
| Bugs from the mapping, batch 3 (22 of 24; 2 kept) | merged | 65b7d7b |
| Bugs from the mapping, batch 4 (12: equipment, weather, leftovers) | merged | 7eda7b9 |
| 2a+2h: spells, auras, PvP | merged (all 8 items) | 099722e |
| 2b: creature movement (all 11 items) | merged | c2bb0e7 |
| 2c: creature combat (all 15 items) | merged | e557cf0 |
| 2d: scripts, encounters, spell lists, the Molten Core rune | merged | e089517 |
| 2e+2f: instances, quests, events (all 8 items) | merged | 9669fc0 |
| 2g+2i: world, social, second pass (3 items, and 8 systems line by line) | merged | ef1db26 |
| Robustness (all 8 items, and a remote abort fixed) | merged | 5f53bef |
| wow_messages stop layout | done | 2bed07d |
| wow_messages fork round 2 (crash bugs, 7 layouts), repinned everywhere | merged | 3eacdd1 |
| Performance (CPU −77% idle, −55% arena, −38% crowd; load ×10 faster) | merged | 8775d80 |
| 3a: matrix gaps, spells and auras (195 rows) | merged | 5e26640 |
| 3b: matrix gaps, combat and creature AI (259 rows) | merged | |
| 3f: matrix gaps, network and inbound (232 rows) | merged | |
| 3i: matrix gaps, creature AI, EventAI, scripts, pets (263 rows) | merged | 0877908 |
| 3c: matrix gaps, items, quests, loot, professions (241 rows) | merged | |
| 3d: matrix gaps, player systems (311 rows) | merged | 5e26640 |
| 3e: matrix gaps, world systems (140 rows) | merged | |
| 3l: matrix gaps, world entities | merged | 36db44d |
| Bugs, batch 5 (Holy Shield, the Warsong flag, op 19, mana ticks, procs, Nightfall) | merged | f09cc53 |
| Snares stack (your #27) | done | 6aec095 |
| 3k: matrix gaps, inventory, groups, social, guilds, trade, PvP, instances, Stratholme | merged | e904b5f |
| 3j: matrix gaps, the loader on the real data (124 rows) | merged | fab8aaa |
| 3h: matrix gaps, encounter scripts (70 rows) | merged | b8e080f |
| The matrix after the second round | 3,785 of 8,396 applicable covered or accepted (45.1%) | |
| Quest 1560's walk | merged | 8dc7a49 |
| Fast and extended test suites | merged | eef0000 |
| Round 3 (3m to 3r), the work committed before the agents stopped | merged | 208be0b |
| 3p: combat and power (179 rows) | merged | 1e01c85 |
| Review fix 1: trades, mail and auctions saved together; the reconnect answer | merged | 70cd32f |
| 3q: auras (167 rows) | merged | d437cd0 |
| 3n: the player (68 rows) | merged | a3d93c0 |
| 3o: the loaders (100 rows) | merged | b9f364f |
| 3m: spells and classes | merged | d70cd01 |
| The matrix after 3p, 3q, 3n and 3o | 4,425 of 8,385 applicable covered or accepted (52.8%) | |
| The matrix, partway through round 3 | 4,348 of 8,385 applicable covered or accepted (51.9%) | |

## Left for later, and why
