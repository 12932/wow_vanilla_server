# Sitrep: the overnight run (2026-10-17 to 18)

What was asked:
- **First,** align wvs with Thistle Tea ([FULL_PARITY_PLAN.md](FULL_PARITY_PLAN.md)), committing and pushing as each
  phase lands. Load tests are capped at five minutes.
- **Then,** the kobold's reach, performance, and robustness ("Elixir's reliability").
- **Last,** a code review.

This file is where the decisions made along the way are written down, so you can override any of them, along with
the questions waiting for you.

## Questions for you

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

27. **Snares** (from 3a). wvs now follows Thistle Tea and vmangos: one snare at a time, and a weaker or shorter
    one is passed over. mangos-classic lets snares coexist, with the strongest counting. Which?
28. **A charm or Mind Control ending** (from 3a). Should the freed creature's cast or channel be interrupted
    (Thistle Tea), or carry on (mangos's `Uncharm`, and wvs today)?
29. **Talent reset decay** (from 3d). After one reset, should the cost fall back to 1 gold after a month (Thistle
    Tea), or stay at 5 gold (mangos, and wvs)?
30. **Melee attack-speed slows** (from 3d). They don't stack, as vmangos and Thistle Tea have it (done); mangos-classic
    multiplies them. Keep?

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
| Performance | running | |
| 3a: matrix gaps, spells and auras (195 rows) | merged | |
| 3b: matrix gaps, combat and creature AI (259 rows) | merged | |
| 3f: matrix gaps, network and inbound (232 rows) | merged | |
| 3c: matrix gaps, items, quests, loot, professions (241 rows) | merged | |
| 3d: matrix gaps, player systems (311 rows) | merged | |
| 3e: matrix gaps, world systems (140 rows) | merged | |

## Left for later, and why
