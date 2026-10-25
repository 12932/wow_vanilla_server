# Decisions and questions, in full

The record behind [SITREP.md](SITREP.md): every question put to you, as it was written, and every decision made along
the way, with its reasons. SITREP.md keeps the short index; this keeps the detail.

## The questions, as asked

**2026-10-21:** every question still open below was settled by mangos-classic (agent 12q, 0.1.67) except #4;
the answers, with mangos's functions, are in SITREP.md's **Answered**. The text below is as it was asked.

**Answered (2026-10-18):**
- **Battleground view distance:** 400 yards, against the world's 100, done in `c380d54` (`[world]
  battleground_visibility_yards`).
- **Ghosts:** a ghost walks on water, and sees only what is within 45 yards of its corpse, players too, as mangos has
  it. This replaces bugs3's "ghosts see every player". A agent stage (`fix/ghost`) is on it.
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
   top-level "let it crash")? Answered 2026-10-19: the watchdog is removed completely, so a hung map is no longer
   reported (decision 61).
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
    it means every level-60 combat test needs its skills raised. Change it? Answered 2026-10-19: yes (decision 62).

40. **How a creature a player controls heals** out of combat (from 3p), whether charmed or a pet: Thistle Tea's spirit ×
    4, mangos-classic's 13% of its health, or wvs's current twentieth?

41. **Cancelling Stealth while Vanish is on** (from 3q). Should both come off, as Thistle Tea has it? mangos's
    `CMSG_CANCEL_AURA` takes only the spell named, so Vanish keeps the rogue stealthed. wvs follows mangos for now.

42. **An interrupt on a creature immune to silence** (from 3m). Should it still lock out the school? mangos-classic
    locks it; Thistle Tea, following vmangos, spares the creature. wvs locks it.
43. **A fear that damage may break** (from 3m). Should a fear without the damage-interrupt flag break on damage by
    chance, as mangos-classic does? wvs doesn't break it.

44. **A staying pet's fight** (from 3r). Should a pet told to Stay keep fighting its current victim, as Thistle Tea
    has it? This turns on #19, so it's left as it is. A staying pet now chases only the victim its master's
    Attack named.
45. **Corpses** (from 3r). Two cases differ from Thistle Tea:
    - A fallen hound with no kin left: wvs leaves a lootable corpse; Thistle Tea despawns it at once.
    - A dead pet's corpse: wvs keeps a hunter pet's 60 s and a summoned pet's 5 s, as mangos does; Thistle Tea keeps
      them an hour and 15 s.

    Which should wvs follow?

46. **Fade's threat given back** (from review 3, R3-3). When Fade ends, wvs gives back its whole amount, as Thistle
    Tea does, even if it took less: 100 threat can come back as 820. mangos gives back only what Fade took. Follow
    mangos? The fix is one line, but it changes a parity test that pins Thistle Tea's behaviour.

47. **Quest-log slot visibility** (from 4b). Every observer can see the ids of a player's quest-log slots in wvs.
    mangos-classic shows them to the group only, and Thistle Tea to no one. Follow mangos, which needs group-only
    update fields, or Thistle Tea?
48. **Druid form attack power** (from 4b). Which is 1.12's?
    - Thistle Tea gives no share by level.
    - mangos-classic gives level × 3 for every druid.
    - wvs now gives level × 2 plus agility in Cat Form, and level × 3 in Bear Form.

49. **Fresh creature guids for each dungeon copy?** (from 4a). wvs builds a copy's creature guids from the database's,
    so two copies' creatures share guids. No client sees two copies at once. Thistle Tea gives each copy fresh ones.
50. **Nerub'enkan's scarabs** (from 4a). vmangos's own EventAI (1057701) despawns a Crypt Scarab within 5 yards of its
    victim, so his swarm vanishes on arrival, in Thistle Tea too. Keep it, or exempt his summons?
51. **A creature-owned pet's death** (from 4c). What does its killer get? Nothing (mangos-classic), or three quarters
    of the XP and the credits (Thistle Tea, vmangos)? wvs gives nothing.

52. **`.honor week`** (from review 4, R4-23). The GM command loses the rest of that week's honor. When should the
    week re-rank: at the real week's end, which decays rank points twice, or the day after the command?

53. **The listen address** (from the config stage). Login (3724) and the world (8085) listen on every interface,
    hard-coded. With the shipped `creation = "auto"` and `everyone_gm = true`, anyone who can reach those ports gets
    an administrator, and now `.reload config` too. Add `[network] bind_address`?
    - **Defaulting to 127.0.0.1** is safe, but your remote client then needs `0.0.0.0` set in `config.toml`.
    - **Defaulting to 0.0.0.0** keeps today's behaviour.
54. **Defaults that differ from mangos** (from the config stage). Kept as wvs had them:
    - instance view distance: 100, mangos 170;
    - leash radius: 50, mangos 30;
    - assistance radius: 10, mangos 5;
    - corpse decay for rares, rare elites and bosses.

    Switch to mangos's?
55. **The whole world awake** (from the load test). With every creature on both continents awake, from 6,471
    observers, a continent's update needs 25 to 29 ms of CPU at p50, but 81 to 87 ms at p99, and 20 to 25% of
    updates go over 50 ms. The fixes, cheapest first:
    1. **Stagger the 500 ms aggro and environment timers,** which run in step and spike every tenth update. Hours.
    2. **Prune the awake set once per update,** saving about 11%. A day.
    3. **Load navmesh tiles ahead of players, on a background thread,** where they now stall the map's thread. A few
       days.
    4. **Unload unused tiles,** as memory reaches 13 GB at 100%. A day or two.

    1, 2 and 3 should bring a fully awake continent to 27 to 34 ms, evenly spread. Past that, a continent needs
    regions with their own update loops, or creature turns run in parallel: weeks, and not needed for a realm of a
    few thousand mostly in cities. Do 1 to 4?


56. **Unlearning a profession** (from 4d). In 1.12, does unlearning a profession drop its quests, active and rewarded,
    and their source items? Thistle Tea does. wvs and mangos-classic's `HandleUnlearnSkillOpcode` don't. Three matrix
    rows are left gap on this.

57. **A passive party aura on its caster** (from 5d, `priority:69`). mangos-classic's `IsNeedVisibleSlot` shows a
    passive party aura on its caster too; Thistle Tea and wvs hide it. Which is 1.12's?

58. **Buffs on a shapeshifted target** (from 5d, `shapeshift:124`, `:134`). Thistle Tea refuses a buff that
    shapeshifting cancels (Water Walking) on a shapeshifted target; mangos-classic has no such refusal, and wvs allows
    it. Should wvs refuse?

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
   - **Distract** affects creatures only. **Overridden 2026-10-20:** it turns players too (your call, mangos's `EffectDistract`).
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
    - **Ghosts** (replaced at your word, `fix/ghost`): a ghost sees only what is within 45 yards of its corpse. The
      exceptions are its group, its side's ghosts, spirit healers, its own corpse, and boats and elevators. A ghost
      walks on water. Its group's living members see it, as in mangos.
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
      feature added a profiler; it was removed on 2026-10-19 (decision 64), Tracy covering it.
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
    - **GM account commands and lowercase names: not a bug.** I reported that `.ban alice 10` found no account; review 2
      found the accounts table's `name` is `COLLATE NOCASE`, so it always did. I had misread the code. A test now
      keeps it so.
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

39. **3r's choices** (creatures, pets and world systems, finished):
    - **Sessile creatures** stand in a fight unless a script lets them chase, and `NO_MELEE_FLEE` ones don't swing,
      as mangos and Thistle Tea have it. You'll notice it on repair bots, combat dummies and water guardians.
    - **A template's effect-mechanic immunity** keeps off only the effect with that mechanic. So a blow with a stun
      now hurts a stun-immune creature, where it used to be refused whole.
    - **A creature without an addon** has its weapons out, as mangos's `InitEntry` sets it.
    - **A moderator's channel message** reaches members who ignore them.
    - **A react state a script set** is kept over a respawn.
40. **A 500-bot race, here** (checking your crash):
    - **Load:** 492 of 500 bots alive, about 236,000 relayed moves a second.
    - **The packets:** none that wow_messages couldn't read, and every bot's look valid for the client.
    - **The map's update:** with 490 players on Booty Bay's docks, it ran over its 50 ms only 8 times, at most 79 ms.
    - **Your client's crash** was at the same spot as the earlier bad-look ones (`0x006194DD`, texture compositing).
      It is waiting on its `.dmp` and the build's commit.

41. **The client crash at `0x006194DD`** (your race, 2026-10-18; fixed in `12c055a`). A creature walking a two-point
    path that came into view crashed the client.
    - **Why:** the 1.12 client counts a create's spline segments as its points less three, then copies that count
      unchecked. mangos sends its spline's padded array: a linear path's last point again, and a smooth path's
      extrapolated first point too. wvs sent the bare path, so two points became a count of -1.
    - **The fix:** creates now carry mangos's array.
    - **Next:** a agent stage (`work/clientemu`) is building your idea, a test that runs what wvs sends through
      `WoW.exe`'s own parsing code under an emulator, so the next such packet fails a test before it reaches a client.
42. **Review 2, the map core** (findings in `runs/review/review2.md`). Fixed, each with a test:
    - **Copies and bags:**
      - an empty dungeon copy lost its save in seconds, so a group back after a wipe got a fresh copy;
      - bank bag slots bought were lost at every login.
    - **Bans:**
      - a `.ban`'s kick let the client straight back in on its cached session key;
      - `.ban` and `.levelup` with huge numbers overflowed;
      - a test that GM account commands find a name in any case (they always did; my report was wrong).
    - **Movement:**
      - a NaN or far transport offset was relayed and saved;
      - the first move after a login went unchecked.
    - **Config:** an unreadable `config.toml` silently ran on the defaults.
    - **Copies stopping:** a player sent into a stopping copy was lost.
    - **Left for you, the hard ones:**
      - **H1. Transport-flag teleport:** a client claiming to be on a transport skips the speed check, so it can
        teleport. A fix checks the transport's GUID and where it is on its route.
      - **M1. Height:** the speed check ignores height.
      - **M2. Copy race:** two group members entering at once can get different copies.
      - **M3. Shutdown:** shutdown can miss a player mid-transfer.
      - **M4. Empty pools:** health or mana saved at 0 load as full, so a relog refills a mage.
      - **Eleven minor ones** are in the findings file.

43. **Review 3, combat, spells and auras** (findings in `runs/review/review3.md`). Fixed, each with a test:
    - **Spells:**
      - reagents banked mid-cast let the spell go off for free, an item duplication;
      - an interrupted cast was told twice;
      - a cast during a channel didn't end the channel.
    - **Threat:** a spell counted its caster's threat auras twice (160 rather than 130 under +30%).
    - **Leaving the map:** a player who logged out kept their channel and stayed on creatures' threat lists.
    - **Robustness:** a panic in a proc left the proc depth raised for good.
    - **Creatures:**
      - one switching victims moved its leash origin;
      - an attacker killed by a damage shield swung on from its corpse;
      - a creature down in a linked feign death ran its timers;
      - one that forgot its victim could turn to one it can't attack.
    - **Combat:** a dual-wielder's hands swung in the same step.
    - **Auras:**
      - restored stacks showed as one;
      - a replaced aura was never wholly removed, so a cooldown waiting for it never started;
      - a pet area aura of two effects gave the pet only its last.
    - **Left, the harder ones:**
      - **R3-15:** a death action that despawns its creature cuts the death short, so loot and its summoner's and
        group's death events are skipped.
      - **R3-16:** a new aura's index can go stale after the auras it replaces end.
      - **R3-31 (speed):** heals, Fade and logouts walk all 25,000 creatures to find the threat lists they're on. A
        per-unit index of who holds them would fix it.
      - **Twelve minor ones** are in the findings file.

44. **4b's choices** (the core: stats, skills, wands, movement):
    - **A percentage of damage done that wants an item** counts only for a hand whose weapon fits it, and never for
      spells, as mangos has it. Before, Wand Specialization raised every spell by 25%.
    - **A wand's Shoot** stops for any cast that isn't triggered, or when its target goes out of range. Auto Shot
      waits through both.
    - **Animal forms** make a player a beast: cat, bear, travel, aquatic and Ghost Wolf.
    - **Rounding:**
      - an ability's armor cut rounds to a whole number, as Thistle Tea has it;
      - a fall after Slow Fall counts from where the player last was.
    - **Creatures** carry 0x10 in `UNIT_FIELD_BYTES_2`.

45. **The client-code harness** (your idea; `tools/client_check/`, `docs/TESTING.md`).
    - **What it is:** `WoW.exe` itself, under the Unicorn emulator. It runs the client's own parsers on what wvs
      sends: update objects and their creates, movement blocks and splines, monster moves.
    - **What fails it:** a fault, a read past a packet's end, or an absurd count.
    - **The proof:** it reproduces every crash captured so far. The gorilla's spline gives -1 at `0x6194DD`, the
      forgotten object a null read at `0x45353C`, plus a 41-word mask overrun and a read past an end. The padded
      spline passes, and live traffic gives no false positives.
    - **The test:** the extended suite's `tests_client_emu::the_packets_wvs_sends_pass_the_client` runs it on a
      crowd's capture.
    - **Out of its reach:** the bad-look crash, which happens in drawing rather than in parsing.
46. **4a's choices** (encounter scripts and instances, 171 rows):
    - **Blackrock Spire's alcoves** follow mangos-classic's roster (`DoSortRoomEventMobs`): a spawn by a rune that
      rolled a summoner or veteran guards it too.
    - **Stage 3h's leads:** Doom'rel's failure was the test racing the map, and Nerub'enkan's missing scarabs are the
      data's own EventAI (question 50).
47. **4c's choices** (world entities and the network):
    - **Chairs** seat one, on the nearest seat at the chair's height (`SIT_LOW_CHAIR` and up), from 3 yards. Before,
      a chair sat everyone at its centre on the ground.
    - **Guardians:** a second cast of a guardian sends its kind away, and a triggered one adds to them.
    - **Wild summons** (a Field Repair Bot, fireworks) are creatures of their own.
    - **Auction notices:** the outbid were told "You won". The client also read past the end of a refused shot's cast
      result. Both are fixed.
    - **Skinning:** a skin with an empty loot table is skinned once.
    - **Spell hits:** a harmful spell of damage class 0 never misses, as in mangos.
    - **Rifts** replace an invader that despawned.
    - **Demonic Sacrifice** ends with a new demon.

48. **Tracy, reworked** (your ask; `docs/PROFILING.md`). Set `WOW_TRACY=1` and connect Tracy v0.13.1 (v0.14.1 since decision 64).
    - **What you see:**
      - a frame per map update, named by the map;
      - a zone per update step, packet (by opcode), store transaction and load stage;
      - plots once a second, of each map's players, creatures awake, mailbox and outbound, and the realm's;
      - warnings and errors as messages.
    - **Off:** it costs nothing measurable. It records only while a profiler is connected, and listens on localhost
      only.
    - **Your calls:**
      - The `tracy` feature is on by default, so a release build can be profiled with no rebuild. That costs a C++
        build once.
      - System tracing stays: Tracy's sampling comes with it, and it needs administrator rights to do anything.
    - **What 100 bots showed:**
      - An Eastern Kingdoms update takes 2.8 ms, most of it the despawn steps' three looks through every creature
        (1 ms).
      - The worst hitches are navmesh tiles loaded during an update, at 20 to 116 ms each. Loading them ahead, or off
        the map's thread, is the next performance lead.

49. **Review 4, the world's systems** (findings in `runs/review/review4.md`). Fixed, each with a test:
    - **Money from nothing:** an item sold to a vendor could be moved, used or mailed out of its buyback slot, the
      money kept.
    - **A crash:** a script starting itself with no delay recursed until the stack overflowed.
    - **Items and loot:**
      - stacks added to weren't told their new count;
      - a bank bag with items in it could be destroyed;
      - any player could close another's loot window, taking a rogue's pick-pocket loot, and leaving didn't close
        one.
    - **Auctions:**
      - a big bid's cut overflowed;
      - a deleted seller's bidders lost their money;
      - an auction could be cancelled away from an auctioneer.
    - **Channels:** empty locked channels were never removed.
    - **Event spawns:** a spawn tied to several events kept only one (849 in the shipped data).
    - **Quests:**
      - a quest taken already done wasn't told complete;
      - an abandoned quest's area aura stayed.
    - **Objects:** a scripted object lost its respawn.
    - **Guilds:** a charter could be turned in unheld.
    - **Buyback:** it skipped an item's unique limit.
    - **Honor:** honor in a shutdown's last 5 s was lost.
    - **Left, the harder ones:**
      - **R4-18:** round-robin loot lets only the assigned looter open the corpse, so the others' quest drops and gold
        wait.
      - **R4-19:** groups aren't saved, so a restart dissolves them.
      - **R4-20:** a general killed without a player's credit doesn't end Alterac Valley.
      - **R4-21:** a race between taking a mail's item and its expiry sweep can duplicate the item.
      - **R4-22:** a destroyed charter keeps its name reserved.
      - **Minor ones,** and four speed leads, are in the findings file.

50. **The config stage** (your ask; `docs/CONFIG.md`).
    - **The file:** `config.toml` in sections (`[network]`, `[world]`, `[visibility]`, `[rates]`, `[creatures]`,
      `[players]`, `[battleground]`, `[anticheat]`, `[accounts]`, `[gm]`, `[spawn]`, `[arena]`). Keys are named after
      mangos's where it has them (`Rate.XP.Kill` is `[rates] xp_kill`).
    - **Overrides:** any key from the environment, as `WVS__RATES__XP_KILL=2`.
    - **Reference:** `--print-config`, and a `config.example.toml` generated from the code.
    - **Hot reload:** `.reload config` or SIGHUP for rates, distances, timers and the MOTD; ports and paths need a
      restart.
    - **Moved in at their old defaults:** XP, drops by quality, money, honor, reputation, rest, respawn and corpse
      decay, skill gains, aggro, leash and assistance, start level and money, the level cap, battleground timers,
      the speed check, and GM command levels.
    - **Old keys still load.**
51. **The whole-world-awake load test** (`docs/PERFORMANCE.md`, "The whole world awake").
    - **The tools:**
      - `tools/world_points.py` (cover points, `docs/world_points.csv`);
      - `wvs-world-bench` (behind the `world-bench` feature);
      - peon's `--spread world --points` (peon `b4fe387`).
    - **peon:** it drove 350 bots across the world on this machine at 0.24 cores of server. Memory, not CPU, was the
      limit (question 55).

52. **5a's changes** (the player, 70 rows). Its session ended before its report, so these are from its commits:
    - **Warriors log in in Battle Stance** when they had none, as mangos's `_LoadAuras` has it.
    - **NPCs serve no one while fighting or charmed.** A stable master serves hunters alone.
    - **A purchase dropped on a slot goes there,** spills into that bag, or is worn.
    - **Items:** one is used only by who may use it, and a book teaching what is known is refused, not spent.
    - **Talents and languages:**
      - a talent rank learned past the next unlearns the others;
      - a language's spell gives its language at once.
    - **Reincarnation** isn't offered without a self-resurrection.
    - **Companions** have their template's level and flags.

53. **5b's changes** (spells and classes, 139 rows; from its commits, as its session ended before its report):
    - **Target triggers** roll once per cast's impact on each target, after its effects.
    - **A send-event effect** with no target of its own is aimed as mangos aims it.
    - **An item's spell** costs no power and starts no five-second rule.
    - **A dispel** wants something to take, else "nothing to dispel".
    - **A channel's last tick** runs while it is still the caster's channel.
    - **Pets:** Soul Link, and a pet's own area aura on it and its master.
    - **Blessing of Light** adds by the heal's coefficient.
    - **Weapon enchantments** take their wielder's chance modifiers (Improved Poisons).
    - **The Gnomish transporter** does nothing for the dead.

54. **flaky1** (the tests that failed only under load). Two were wvs bugs:
    - **A creature attacked keeps fighting.** One with an empty threat list fights whoever is attacking it, and puts
      them on its list, as mangos's `SelectHostileTarget` has it. This departs from Thistle Tea, which has no such
      rule: Remulos evaded when his last phantasm died before Eranikus's breath reached him.
    - **A creature's cast at a target it may no longer attack fails** as it ends, as a player's does and as mangos
      and Thistle Tea have it. Remulos's Starfire had landed on the redeemed Eranikus.
    - The rest were tests budgeting by looks or seconds; `docs/TESTING.md`'s "Under load" has the rules.

55. **4d's choices** (items, quests, Alterac Valley, objects, honor, professions and events; 234 rows):
    - **Purchases** are refused with mangos's codes (`DoesntGoIntoBag`, and `InventoryFull` for a two-hander bought
      with full bags).
    - **Item changes:**
      - a change into a two-hander puts the off hand away, as Thistle Tea does (mangos refuses);
      - an item may replace itself though unique;
      - a failed change sends an inventory failure, where mangos is silent.
    - **Gear spells:** only passive equip spells (519 of 562) outlast death and can't be cancelled; a non-passive one
      such as Argent Dawn Commission still drops at death. Enchantments' equip spells aren't held to forms or the
      outdoors.
    - **Enchanting:** items saved bound by an earlier enchantment stay bound, with no migration.
    - **Racial leaders** are worth 488 honor, as Thistle Tea has it; mangos has 398, with a comment doubting it.
    - **Captured animals** follow Thistle Tea, as mangos has no such mechanic: when the player dies the animal goes
      after 2 s, not 1, and leaving the map ends a capture at once.
    - **Using objects:**
      - the immunity bar applies to quest givers and quest objects only, not doors or buttons;
      - a newer state cancels an older reset timer, as Thistle Tea has it (in mangos a scripted state change doesn't
        cancel a door's reset).
    - **Engineering:** the gadgets keep Thistle Tea's outcome weights where mangos differs (the Universal Remote,
      the Mind Control Cap, the Net-o-Matic). The second-disenchant refusal comes when the cast goes off, not at its
      check.
    - **Omen** is a two-hour summon, so he leaves at two hours even mid-fight.
    - **Left as gaps:**
      - unlearning a profession and its quests (question 56);
      - the cheer speaker's fireworks (about 480 lines of launch sites);
      - the Scourge Invasion's camps, which wvs doesn't have at all (shards, finders, minions, rares, cultists);
      - auto-rewarded quests (flag 0x400; the data has one, 9378).
    - **A lead:** mangos adds the Death Ray's damage left after absorbs to its charge; wvs, like Thistle Tea, adds
      the base amount.
    - **Merged with a fix:** flaky1's rule for a creature's cast failing at a target it may no longer attack stopped
      Ragnaros's Elemental Fire at the Majordomo he had made friendly. It now applies only where the effect must
      target an enemy, as mangos's `CheckCast` filters it (`040ee2f`).

56. **5c's choices** (combat, 102 rows). Where Thistle Tea and mangos differ it follows mangos, and the rows are
    `wrong-for-1.12`:
    - **Swing timers:** only a weapon put in the main or off hand resets its swing; taking one off, a break, a ranged
      swap, a form or a disarm reset nothing.
    - **Pet kills** proc the master's on-kill auras as well as the pet's; Thistle Tea procs the pet's alone.
    - **A feral main hand** trains unarmed; Thistle Tea trains nothing.
    - **Stopping an attack** doesn't clear a player's selection.
    - **Owed extra attacks** are kept until swung; Thistle Tea clears them on an attack stop, a death or a mount.
    - **Seated crits:** a player with no crit chance at all doesn't crit a seated one (mangos's `CanCrit`).
    - **A struck creature** stands when it enters the fight, not at each blow (mangos stands it unless stunned).
    - **Beyond Thistle Tea's tests:** critters train no weapon skill, and spell damage now goes taken modifiers,
      then crit, then armor (armor came first), which moves every physical ability's numbers.
    - **Left open:** the real-data rows (Spirit Tap, Shadowguard, Lightning Shield, Deep Wounds, the racials, death
      items), proximity, feign death, the shared leash clocks, assistance, disarm, and most of kill and group
      rewards.

57. **perf2** (the whole world awake, round two; [PERFORMANCE.md](PERFORMANCE.md)). Standing, a fully awake continent
    now keeps 50 ms an update; walking, nearly.
    - **Aggro scans run at exactly 500 ms,** as the constant says; on the old beat they ran every 550–680 ms. That
      costs 2–4 ms an update with everything awake; a longer interval, or skipping creatures with no player in
      reach, would take it back.
    - **The navmesh uses `rustigator-pathfind` directly** (the same repository and revision), as rustigator's
      wrapper can't take a tile decoded elsewhere.
    - **Tiles are asked for within the view distance and 20 yards** of each player (+0.7 GB at 3% of the world
      awake), and let go after `[world] tile_unload_secs` (300, mangos's `GridCleanUpDelay`).
    - **A dungeon copy that is one WMO** is still read whole when it starts.
    - **Left:** BVHs still built on the map's thread (up to 27 ms a tile); with the whole world awake nothing
      unloads (about 13 GB), so a tile cap would bound it; Kalimdor with the navmesh unmeasured; fixes 5 and 6.


58. **5d's choices** (auras and objective carriers, 151 rows; 13 fixes):
    - **The dead's speeds:** a player lying dead takes back the speeds their slows and swim bonuses changed; before,
      they stayed until release.
    - **Restless Strength's** flat damage still goes on each blow, not into the hands' damage fields the client
      shows (`depleting_aura:32` left gap).
    - **Tracking fields** combine every tracking aura's bits; mangos sets one aura's at a time, and in play only one
      is on at once.
    - **Following mangos where Thistle Tea isn't 1.12** (`wrong-for-1.12`): a friend's school protection on a flag
      carrier lands and drops the flag; releasing a charmed creature interrupts only its auto-repeat and melee
      spells; a spell modifier gives an aura whole points; a victim of a persistent area aura's pure periodic damage
      gets a debuff slot.
    - **Merged with a fix:** 5b's rule that an item's spell costs no power wraps 5d's cost in mangos's order.
    - **Left:** about 290 aura rows (breakable control, capacity, caster modifiers, dispel, linked, persistent area,
      stealth detection and more) and all the world loader rows.

59. **5e's choices** (pets, pet behaviour trees and creature scripts, 116 rows):
    - **Pet stats:** a hunter's or warlock's pet's stat auras raise its stats: 10 health a stamina point over its
      level's, 15 mana an intellect point, as mangos-zero has it (mangos-classic softens the first 20 stamina).
    - **Crowd control and a pet's attack:** a pet lets its victim be once crowd control that damage would break is
      put on it after its master's Attack; Thistle Tea's narrower rule is kept, that the Attack overrides crowd
      control only on the target it named.
    - **A dead pet** is refused the stable, as mangos-classic has it; Thistle Tea stables it dead.
    - **A totem or ward** goes with its owner's death, a creature owner's too (mangos-classic's `Totem::Update`;
      question 31 stands).
    - **Happiness** scales only a hunter pet's main-hand weapon damage and its flat damage buffs, not its abilities
      or periodic damage (three rows `wrong-for-1.12`).
    - **A script's cast of a passive spell** puts it on, as vmangos's `DoCastSpellIfCan` does.
    - **Left:** Obsidion's Smash and Knockback never went in its runs (row left gap); Bly's Revenge's cast isn't
      asserted; about 320 rows untouched (the Eranikus, Gizelton, Stormwind Rendezvous, Alterac Valley and Scourge
      Invasion walks, mob and distancing behaviour trees, charmed and possessed players, guardians).

60. **Item charges saved** (question 13; `fix/charges`). Each item's five spell charges are saved wherever an item is:
    carried, in bags, the bank, the buyback, mail and auctions.
    - **The store:** migration 3 adds `character_items.charges`, five numbers separated by spaces as mangos-classic's
      `item_instance.charges` has them. A row saved before it loads with its template's charges. Mail and auctions
      keep their items as JSON, so they carry the field without a migration.
    - **Used up:** an item is used up by its last charged spell, as mangos's `TakeCastItem` and Thistle Tea's
      `spend_charge` have it; wvs destroyed it when any charged spell reached 0.

61. **The watchdog removed, and mangos's defaults** (questions 3 and 54; `chore/ops`).
    - **The watchdog** is gone: its thread, pulses, alarms and test. It never had config keys. Supervision stays (a
      panic costs one step, creature or packet; a map that keeps panicking restarts with its back-off). A hung map is
      no longer reported.
    - **Defaults:** `[visibility] instances_yards` 170, `[creatures] leash_radius` 30, `assistance_radius` 5, and
      rare, rare elite and boss corpses 900, 1200 and 3600 s. The harness takes its view distances from the defaults.
    - **Left as wvs has it:** a corpse is cut short at its whole respawn, where mangos cuts it at 90%, and mangos's
      own per-template `CorpseDelay` and encounter bosses' times aren't read.

62. **The combat timer, teleports and weapon skills** (questions 20, 33 and 39; `fix/rules`). Where mangos-classic and
    Thistle Tea differ, mangos's.
    - **Combat (#20):** every 1.2 s a player leaves combat when no creature holds them on its threat list, their
      timer is 0 and Bloodrage isn't on. The 5 s timer runs only between player-controlled units, duels included;
      a creature's blow sets none. A healer joins the fight, held 5 s only for a PvP-flagged target. Spells between
      players now put both in combat. When nothing holds the player, their attack stops too (mangos's `CombatStop`).
    - **Teleports (#33):** `Map::teleport` ends the fight; another map's always does. mangos's near teleports under
      100 yards keep it (`Map::near_teleport`: Blink, teleport to caster, a spell's teleport within the map, a chair).
      The creatures keep the player on their threat lists, so their next blow brings them back in.
    - **Weapon skills (#39):** weapon skills and Defense start at 1 and rise by use; Dual Wield stays at its cap.
      **Skills are now saved**: wvs never saved them. A character saved before keeps its weapon skills at their caps.
      `[players] always_max_skill_for_level` (mangos's `AlwaysMaxSkillForLevel`, off by default) caps them at login
      and each level; the harness turns it on.
    - **You might override:** pets keep their own 6 s window (in mangos a pet leaves combat with its master); Fist
      Weapons train as other weapons (mangos makes them a 1/1 skill counted as Unarmed).
    - **Left:** mangos holds a spell's caster in combat from launch for its travel and 500 ms (wvs, when it lands);
      a pet unsummoned by a same-map teleport past 50 yards; skill values kept for forgotten talents.

63. **The packet order** (question 11; `fix/order`). A client gets an object's create before anything naming it.
    - **The hold:** a client is held (`Outbound::hold`, one atomic flag a send checks) when an object is introduced
      to it, added within its view (`Map::hold_near`), or as it logs in, lands from a worldport or teleports within
      the map. What is sent meanwhile waits, in order. It is let go at the end of the event that held it
      (`Map::after_events`, which works out visibility for what the event added first), else at the update's "held"
      step, or when it leaves the map; creates and forgets go first. A client nothing waited for keeps its one batch.
    - **Exceptions:** a creature whose spawned events haven't run is still seen at the update (seen sooner, it could
      be attacked before its events started); a pong and the name, creature and item queries' answers skip a hold
      (`send_unheld`); a client awaiting a worldport is never held, and its pet's bar is sent at the acknowledgement.
    - **Forgets first:** a packet about an object forgotten in the same update reaches the client after the forget,
      and is dropped; the other order would put a corpse's create before its owner's forget, which crashed it.
    - **Cost:** within the noise, in `wvs-world-bench` (1,618 and 6,471 observers, standing and walking) and peon's
      arena and cluster. A first version that held clients to the next update cost no CPU but added latency (chat
      p99 47–91 ms); the release at each event's end took it away.
    - **Left:** packets sent by distance (a yell) to a player who learns of the object later by their own move.

64. **Dependencies** (2026-10-19; `chore/deps`, and peon's). Every crate at its newest release.
    - **The majors:** rusqlite 0.40 (SQLite 3.53.2), md-5 0.11, tracy-client 0.19 with tracy-client-sys pinned at
      0.30, so **Tracy's viewer is v0.14.1** (protocol 82), and changes only with that pin.
    - **Removed:** rayon (nothing in wvs used it), the `wowm-capture` feature, wow_world_messages' `print-testcase`
      (its `message_name()` replaced by a name from `Display`, kept once per opcode), pprof and `src/profile.rs`,
      dotenvy (no `.env` is read), and the `env-filter` log filter (`RUST_LOG` now takes a bare level and
      `target=level` entries, through `Targets`; regex and its kin leave the build). wow_srp is without its TBC and
      Wrath headers (hmac goes).
    - **Not taken:** wow_srp 0.7 (`25ffab6`): wow_world_messages pins 0.6, and their encryption types must match,
      so it waits on the fork. A flate2 change (a zlib stream ending before its checksum is now an error) is met in
      `account_data::parse_update`.
    - **The tree:** 211 crates before, 212 after. Gone: dotenvy, hmac, matchers, regex-automata, regex-syntax.
      Added: md-5 0.11's digest stack beside the 0.10 that wow-mpq and wow_srp still use. Clean release build
      3m 19s to 3m 18s; binary 20.9 to 20.3 MB. peon: 110 to 107 crates, `rust-version` 1.92 (rustigator's).
    - **Next:** rustigator's dependencies (wow-mpq's rsa and old rand, parry3d's second glam, the old digest), and
      the parking_lot investigation.

65. **Guards, the alarm and Jaina** (2026-10-19, your asks; `feat/guards`).
    - **mangos-classic's call** (`CreatureAI::AttackStart`, `HandleSpawnGuard`) in place of vmangos's posts: a
      civilian with `CALLS_GUARDS` (bit 0x08000000 of `static_flags1`, 2,431 templates) calls one guard each time it
      takes a victim (a fight's start, a threat switch, a taunt), at a reachable point within 30 yards, attacking its
      attacker. Your changes to mangos: gone 5 minutes out of combat or at death, and wandering 8 yards meanwhile.
      No calls on sight, no rousing. A civilian shouts at most once in 10 s.
    - **The table:** mangos's, with vmangos's zones and two-sided areas and Razor Hill's line. Shadowglen keeps 12160
      (mangos's 4844 is a Badlands surveyor, a typo); Goldshire, Undercity, Hammerfall and the Sepulcher follow
      mangos; a caller's area is matched, then its zone.
    - **The alarm,** per town, halving each 5 minutes without a call (`[guards]`): from 20, Avoidance (aura 160, as
      spell 23198) 5% a step of 10 to 50%, kept through an evade; from 40, two elite a call (Royal Guard 1756,
      Kor'kron Elite 14304, Royal Dreadguard 13839, Honor Guard 3083; Ironforge and Darnassus have none, so two of
      their guards); each threshold logged at `info`.
    - **Jaina,** from 60 in `jaina_city` (Stormwind): alive and home, she speaks and flashes out of Theramore (12980),
      and a copy flashes in near the caller, her flash and line after her create. She fights the attackers, leashed
      to the city; 60 s at peace, she says farewell and goes home. Dead in the city, her home spawn returns after
      `jaina_respawn_secs` (86,400, your choice). Her combat script came with the Thistle Tea import; in Stormwind her
      Teleport sends its victim to the Stockade's exit (area trigger 503), her 40-yard rule about that spot.
    - **Left:** a lost "home" message leaves her "returning"; a restart forgets where she was (her spawn returns).

66. **The raid that drives it** (`feat/gear`, peon's `feat/raid`).
    - **`.gear`** (GM level 2): random uncommon and rare items of the target's class and level in every slot,
      replacing what is worn (destroyed, not bagged), teaching the skills they need, with a wand or ranged weapon and
      ammo; the shirt and tabard are left alone. Armour kinds and dual wield are fixed per class.
    - **peon `--mode raid`:** Horde bots, levelled from `--raid-levels` (50-60), geared, out of GM mode, placed at
      `--raid-at` (the Trade District); each fights its attacker, else the nearest Alliance civilian within 40 yards,
      else a guard, leashed 60 yards from the point, and is revived after `--raid-revive-secs` (30). Movement is a
      straight line, without the navmesh.
    - **Matrix:** the rows that named the removed post tests, and those of #27 and #20's removed tests, are
      `accepted`, each with its reason.

67. **Channels, locks and the live config, measured** (2026-10-19; `cargo bench --bench sync`, a quiet machine).
    - **Channels: tokio's everywhere** (your call; `chore/channels`). kanal won only the outbound queue with one
      sender (33 ns a frame against tokio's 109, 79 with `recv_many`), about 0.6% of a core at 400 players, and lost
      under contention (16 senders: 487 ns against 188; a 64-sender mailbox: 1,388 ns against 217). Its async
      receive is unsafe in `select!` by its own docs. The store's and the tiles' differences are microseconds
      against milliseconds of SQLite and tile reads.
    - **parking_lot stays** (your call): no clear winner. The registry's `RwLock` is about 20% faster with it at 16
      and 32 threads; a contended `Mutex` is 1.8x faster at 4 threads and 2.3x slower at 16; uncontended, the same.
      std with poison recovery costs nothing over std.
    - **arc-swap gone** (`bbafa39`): a map's update reads the reload counter, 0.14 ns, against arc-swap's 4 ns; the
      rare reads elsewhere take the lock (8.5 ns alone, about 1 µs at 32 threads).

68. **What the user's 400-bot raid showed** (2026-10-19; wvs `fix/raidfix`, peon `fix/raidfix`).
    - **"Sends packets too fast":** a client arriving in a crowded city sends a burst of queries, about 140 for the
      place and one name query per player in view, some 540 in a crowd of 400; a real client would too. Read-only
      queries (name, creature, game object, item, page, NPC text, quest and quest-giver status, time, played time,
      next mail, pet name, guild, petition, item text, taxi node, party member stats, corpse and spirit healer) now
      have their own bucket, `[network] max_queries_per_sec` (1,000, 3 s of burst); the 100/s guard covers the rest.
      A session cut off logs, once, what it sent in its last 5 s by opcode. peon asks each thing once and paces its
      queries (5 a second), and `--send-limit` (60) holds a bot under wvs's limit.
    - **Half health:** `.gear` raised the maximums and left the current values; it now fills health and mana (a GM
      convenience). `.character level` already did, as mangos's `GiveLevel`.
    - **Abilities:** `.character spells` teaches the class trainers' spells up to the level, rank by rank, talent
      ranks only with the talent (quest-given spells, such as stances, totems and forms, aren't taught; neither are
      weapon skills). peon's raiders cast a rotation per class, area spells among them, and heal.
    - **Half-sunk raiders:** peon places and moves them on the navmesh's ground (`--nav-data`).
    - **Guards standing still:** Stormwind's guards can swim, and a random-moving swimmer looked only for water
      about its home; on land it found none and idled. A swimmer now swims only when its wander centre is in water
      (as Thistle Tea), so 4,655 of vmangos's 41,927 random-moving spawns now walk on land. `[guards] wander_yards`
      is 10. Most of Stormwind's own spawned guards are stationary in vmangos, so they stand at their posts.
    - **Jaina:** her regression test, with her real no-aggro flag.

69. **The second batch of answers** (2026-10-19; `fix/answers2`).
    - **#21:** the grey "!" for quests up to 7 levels above the player (`[quests] high_level_hide_diff`, mangos's
      `Quests.HighLevelHideDiff`; -1 any gap). mangos's `LowLevelHideDiff` (5) isn't added.
    - **#46:** Fade holds `max(-threat, amount)` and gives back exactly that (mangos's `threatTemporaryFade`).
    - **#41:** cancelling any stealth removes every stealth aura and the form, so Vanish's too.
    - **#30:** attack-speed slows stack and multiply (mangos's `ApplyAttackTimePercentMod`); Thunder Clap is no
      longer exclusive.
    - **#43:** a fear whose spell's proc flags name the blow breaks by chance: the ceiling is 25 × the feared unit's
      level − 150 past level 8 (else 50), the chance damage ÷ ceiling (mangos's `HandleRemoveByDamageChanceProc`; at
      60, 135 damage is 10%). Roots and silences by the same rule are question 59.
    - **#42:** wvs already locked the school; a test pins it (mangos spares only `NO_INTERRUPT_SCHOOL_COOLDOWN`, which
      no creature has).
    - **#7:** a locked door or button opens by hand only with one of its lock's keys carried; a script's unlock lifts
      the check and its lock bars hands; GMs open anything. Locked chests still never open by hand.
    - **#47:** quest-log ids go to the player and their group alone (`Visibility::Group`, as mangos's
      `UF_FLAG_GROUP_ONLY`); someone joining a group gets them at the next change.
    - **#35:** a second login of the same character takes it over (`login::take_over`, as mangos's
      `HandlePlayerReconnect`): the old connection closed, the client's view reset and the world resent, the group,
      friends, guild and pet bar, a flight resumed; another character of the account is refused and the first kicked.
      Breath timers and a cast in progress aren't resent.
    - **Trainers:** a talent's trained rank (Mortal Strike 2 to 4) is red without the talent, the rank before named as
      required (mangos's `GetTrainerSpellState`).
    - **`SMSG_SPELLLOGMISS`:** wvs's layout is mangos's and the client's (`0x5E7E00`); the fork's `SpellMissInfo` is
      a u32 (question 60). `tools/client_check/spell_log_miss.py` checks it, every kind of miss in the scenario.

70. **The Stormwind Elite Guard** (your design, 2026-10-19; `feat/elites`, `map/elites.rs`).
    - **The unit:** entry 101756, made in code, level 62 elite, the Royal Guard's model, faction, speeds, notice,
      resistances, guard flag and loot, and the Tower Point Warmaster's toughness (health ×20, 64,740; damage ×13.5,
      about 1,690 a main-hand blow, 2 s). Two swords: 21573 (the Alliance PvP longsword, display 34128) and 23456
      (Grand Marshal's Swiftblade, 36066); the off hand swings at half.
    - **Its fight,** the warmaster's (`av_scripts::warmaster()`): Charge, Cleave, Demoralizing Shout, Whirlwind
      twice 2 s apart, Enrage once below 30%. **The charge** goes to a random hostile healer (priest, druid, shaman,
      paladin) in sight 8 to 25 yards off, one who healed in the last 5 s first, else its victim at that range; the
      one charged is raised to the top of its threat.
    - **When:** in Jaina's city (Stormwind), two (`elite_count`) as the alarm rises through `elite_from` (50), once
      a crossing, as guards (5 minutes out of combat, wandering); four (`jaina_elites`) with Jaina, `jaina_elites_yards`
      (40) out at random angles, walking to her side and fighting on the way, leaving with her flash (or staying as
      guards if she dies).
    - **You might override:** Enrage once a fight (warmasters recast it each 2 minutes); the warmaster's immunities
      not copied; Elite Guards not counted in the alarm; they take the alarm's Avoidance; the 5 s heal window a
      constant; Jaina's four walk in rather than flash.
    - **A fix to Alterac Valley:** the warmasters' second Whirlwind went off with the first (an EventAI action's
      delay is ignored); it is now a timed sub-script, and their scripts stop at an evade, as Thistle Tea.

71. **Round 11's calls** (2026-10-21, your answers to the triage of the last 178 gap rows; stages `parity/11a` to
    `11j`):
    - **A pet's harmful bar spell:** mangos-classic's `HandlePetAction`. In range and sight, the pet casts it and
      starts no attack; out of range or sight, it runs to the target, attacks, and casts it in range (the spell
      opener). wvs had started a melee attack after every harmful cast, and failed one out of range.
    - **Spell range, on a unit and on a destination alike:** Thistle Tea's (vmangos's) leeway: players 1.25 yards
      at the start and 6.25 at launch, creatures 0 and 2.25, with the bounding radii. It replaces decision 35's
      mangos tolerance at the end of a cast; you wanted ground and unit spells to match.
    - **Spells 15958, 16447 and 24973 without their object** (Collect Rookery Egg, Spawn Challenge to Urok, Clean
      Up Stink Bomb): mangos-classic, unchanged; the script target is optional, and they go without it.
    - **Casting on a transport:** Thistle Tea's: a cast's movement is measured on deck while its caster is aboard,
      so a turn on a moving ship no longer breaks it. mangos compares world positions and has the same bug wvs had.
    - **A creature's pet as a kill** (question 51, reopened): mangos-classic's whole rule: no XP (`XP::Gain`'s
      `IsPet()`), but reputation and quest credit (`RewardSinglePlayerAtKill`). wvs had given a pet nothing and a
      wild guardian full XP.
    - **The meeting stones:** a player's queue goes at logout, as mangos-classic (`m_offlinePlayers` is never filled);
      Thistle Tea's offline tickets are `wrong-for-1.12`.

72. **Round 11's questions, answered** (2026-10-21, your answers; `fix/calls11`, 0.1.79):
    - **Meeting stones:** players alone make a party only when five are queued for the dungeon (mangos-classic's
      `m_groupSize`); a party short of five is topped up as before. The role order stays as 11h left it (a
      half-hour wait, then the class best at the role, then the longest wait), and a raid conversion withdraws it.
    - **An instant spell's bonus and its charge:** kept, the bonus before the charge is spent (Thistle Tea).
    - **Misses in `SMSG_SPELL_GO`:** every spell lists its misses (and a reflection, as 11) in the go, as mangos's
      `WriteSpellGoTargets`; `SMSG_SPELLLOGMISS` is no longer sent for a miss the go told. Left: whole-effect and
      mechanic immunities still go in the miss log, and `tools/client_check` has no `SMSG_SPELL_GO` parser yet.
    - **A player's guardian:** it goes at its time, even mid-fight (mangos-classic's `Pet::Update`). Left: a
      player's wild summons (effect 41) still wait out their fight.
    - **A map's zone without navmesh or terrain:** `Map.dbc`'s first; only Sunken Temple changes (1417 to 1477).
    - **A cast item banked mid-cast** fails the cast, and **the 45 scripted-object spells with no target rows** cast:
      both kept.

73. **Round 12: the survey against mangos-classic, and its calls** (2026-10-21; `work/12a` to `12l`, 0.1.80 to 0.1.90;
    the survey and its ranking in `<runs>/survey/`):
    - **Your answers:** a disarmed creature hits for 50% (mangos-classic; `fix/calls12`); crowd control replaces
      another of its diminishing-returns group (mangos-classic; `fix/calls12`); need before greed stays Thistle
      Tea's (only those who can use an item roll, and a lone one gets it); Battlegear of Wrath's Warrior's Wrath is
      left out (mangos gives it only as a side effect of re-equipping on each stance change).
    - **Kept as 1.12 was, over mangos-classic:** weekly honor decays a player with fewer than 15 kills (mangos leaves
      them); a battleground raid counts for quest kills.
    - **The agents' calls, each following mangos-classic where it has an answer:**
      - a disconnected character lingers 60 s (a constant, as mangos), a kick or a flood removes it at once;
      - the speed check's allowance is a bank filled by time (`[anticheat] latency_allowance_secs`, now 2 s deep);
        the 1.12 root flag is uncertain, so a rooted client is held by its steering and side steps, not the flag;
      - an empty copy lasts 30 minutes; a world boss in the open world respawns at its own time;
      - groups are kept as store records (no migration); a battleground side's raid isn't kept;
      - mining and skinning skill-ups don't decrease (`mangosd.conf`'s 0); a dungeon's rewards reach its whole map;
      - player commands are on (`[gm] player_commands`); Trade and GuildRecruitment are named for a capital;
      - a charm is dispellable off a friend as mangos's `IsCharmAura` means (its code reads the wrong way round);
      - two HoTs of one family stand together (Thistle Tea's Swiftmend test); Blind has its own DR group.
    - **Left, named by the agents:** a cheat client that never acknowledges a flag's removal keeps it (no order
      timeout); forged speed acks go unpunished; the client check has no parser for `SMSG_SPELL_GO`, party member
      stats or the loot window; a lingering player's group sees no change until they go.

74. **The leash, your calls** (2026-10-22; `fix/x2`, 0.1.99): the radius is **31 yards** (`[creatures] leash_radius`,
    over decision 61's mangos default of 30); a creature standing still in reach and swinging renews its leash every
    `[creatures] leash_standing_ms` (1250). mangos-classic's `AttackerStateUpdate` renews it on a swing only after 3 s
    standing and with its victim standing too; wvs's rule leaves the victim's movement out.
    The leash's time goes by the creature's level, 1.12's brackets (`[creatures] leash_timeout_by_level`: 11 s to
    level 29, 12 to 39, 13 to 44, 14 to 49, 15 from 50), and a creature looks at its leash every
    `[creatures] leash_check_ms` (1000) from its fight's start, so it goes home up to a second past its time, never
    before: your read of the variation as the server's coarse check, not a random offset.

75. **Your calls of 2026-10-22, the second batch:**
    - **A spawn's grace:** a hostile creature just come into the world from its spawn (its first, or a respawn; a summon, a script's or a GM's has none) looks for no one
      to attack for `[creatures] spawn_grace_ms` (6000), unless attacked: a respawn mustn't catch a player by
      surprise. mangos-classic has none.
    - **A GM's `/who`:** a GM's account isn't held to `/who`'s, whois's and inspect's 5 s cooldown (mangos's
      `InitOpcodeCooldowns`, kept for players).
    - **Area reach:** areas and cones count a creature's size for a player's spell (mangos-classic), and reach
      further when either side runs (`[spells] area_leeway_moving_yards`, 2) or strafes (`area_leeway_strafing_yards`,
      8/3); a player's size doesn't count against a player's area, and a creature caster's own running gives it
      leeway.
    - **Chase and evade:** a creature in melee chases as mangos-classic's `ChaseMovementGenerator`; an evading creature
      may be attacked, and what hits it evades.

76. **Broadcast batching, your call** (2026-10-22): the window adapts per map (`perf/adaptive-batch`: steered by the
    map's load and the server's CPU, between `[network] broadcast_batch_min_ms` and `broadcast_batch_max_ms`), but its
    least is **10 ms** (the old fixed window): you'd rather keep the CPU down than have the lower latency (2 ms halved
    880 bots' p99 to ~17 ms at twice the CPU). It still widens to 40 ms under load. Lower the minimum for latency.

77. **Hyperspawn, your design** (2026-10-22; `feat/hyperspawn`, `map/hyperspawn.rs`, `[hyperspawn]`): in the races'
    starting areas, the faster creatures die, the faster they respawn. mangos-classic and Thistle Tea have nothing
    like it.
    - **Your choices:** the starting subzones only, as a list of area ids; kill rate the only signal (no player
      counts); creatures' respawns and quest objects' (by their own rate of use) brought forward, no spawns added; a
      floor of 2 s at peak demand, the global `[rates] respawn_time` and its 5 s floor unchanged elsewhere.
    - **The areas**, checked against `dbc.sqlite`'s `AreaTable` and against what the navmesh gives the starting
      spawns' places (`tests_hyperspawn`, extended): your six ids were right (9 Northshire Valley, 132 Coldridge
      Valley, 188 Shadowglen, 363 the Valley of Trials, 221 Camp Narache, 154 Deathknell), but the navmesh puts much
      of the starting areas in their sibling subzones, so the defaults add them: 34 Echo Ridge Mine (Northshire's
      kobolds) and 59 Northshire Vineyards (its Defias and Milly's Harvest); 256 Aldrassil and 257 Shadowthread Cave
      (Shadowglen's tree and spider cave); 365 Burning Blade Coven (the Valley of Trials' cave); 220 Red Cloud Mesa
      and 358 Brambleblade Ravine (Camp Narache's 4 spawns are its quest givers: Red Cloud Mesa has the 223 starting
      creatures, the ravine Sharptusk's quilboar); 155 Night Web's Hollow (Deathknell's spiders). Coldridge Pass
      (800), the road out to Kharanos, is left out. The navmesh gives the deep ends of Echo Ridge Mine and of the
      Burning Blade Coven their zones' ids (Elwynn Forest's 12, Durotar's 14): their 8 Kobold Laborers and 5 Vile
      Familiars aren't sped up, as the whole zone would be with them.
    - **Pockets:** a square of the map, 50 yards (the grid's cell; `pocket_yards`), keyed by the spawn's home, not
      where it died. Each keeps a score that a kill adds one to and that decays as e^(-t/120 s) (`window_secs`),
      worked out lazily from its last time: O(1) a death, nothing swept. Kills a minute are the score × 60 / window;
      over the pocket's spawns (counted once per map, at its first death in an area: those hyperspawn would speed
      up), kills a minute per spawn.
    - **The curve**, linear: `factor = 1 + gain × max(0, rate − quiet_rate)`, capped at `max_factor`; the respawn is
      `base / factor`, no less than `floor_secs`, no more than `base` (`base` the normal one, the global rate and
      its 5 s floor in it). Defaults: `quiet_rate` 0.2 kills a minute per spawn (a spawn killed every five minutes,
      what a 5-minute respawn allows without hyperspawn, so the first pressure starts it), `gain` 40, `max_factor`
      150 (a 5-minute respawn to its 2 s floor). A pocket of 8 at a 5-minute base: 0 kills a minute, factor 1,
      300 s; 10 a minute (1.25 a spawn), factor 43, 7.0 s; 30 a minute (3.75 a spawn), factor 143, 2.1 s; the floor
      from 31.4 a minute. Starting creatures' own respawns are mostly 3 minutes (vmangos), so 30 a minute gives them
      2 s too. The kills feed back (faster respawns, more kills) until the players' demand is met, and decay when
      they leave: two windows after 80 kills in a pocket of 8, its factor is 20, after four, 1.
    - **Fixed at the death:** a creature already waiting keeps its time. Shortening the waiting ones would mean
      finding a pocket's dead, a scan; left out.
    - **The corpse:** wvs respawns a creature in its corpse's place, so a 2 s respawn would take a kill's loot from
      under its looter. A hastened creature's corpse lies as long as it would have (its decay, no longer than its
      normal respawn), at most `loot_wait_secs` (30) past its hastened time, and holds its respawn back while it has
      loot or a skin on it, or someone looking in it: looted clean, it comes back at once. (A new key, not in your
      list: the only way to keep both the floor and the loot.)
    - **Exempt:** a template's rank (elites 1, rare elites 2, bosses 3, rares 4); quest givers (`npc_flags` 2);
      scripted creatures (a `script_name`); named creatures, taken as a template with one spawn on its map
      (Garrick Padfoot, Grik'nir the Cold, Sarkoth, Chief Sharptusk Thornmantle are rank 0); summons, pets, owned
      units and anything without a world spawn; copies (instance maps). Exempt kills don't count in the pocket.
    - **Quest objects** (`objects`, on): a chest whose loot has a quest item (Milly's Harvest, Cactus Apples,
      Moonpetal Lilies, Hyacinth Mushrooms, Webwood Eggs, Equipment Boxes), or a goober a quest uses; their own
      score per pocket, over the pocket's quest objects, on the same curve. Herbs, veins and treasure chests aren't
      quest objects.
    - **Seeing it:** `.hyperspawn` (moderator) gives the area where the GM stands, whether it is a hyperspawn area,
      and its pocket's spawns, kills a minute, factor and what a 5-minute respawn comes to, for creatures and quest
      objects. `WVS_STATS_FILE`'s line has each map's `hot_pockets` (factor 2 or more).
    - **Where the area comes from:** the navmesh (or the terrain tiles) at the spawn's place, kept per spawn once
      given; without either on a continent, what the map last worked out of the creature's or object's place, if
      anything. So a server without its navmesh and terrain has hyperspawn only where that is known.

78. **`.durability off|on`, your design** (2026-10-22): a GM's (level 2) toggle for load tests, which mangos-classic
    hasn't. Off mends everything the character owns, free, by the repair-all's own loop (`vendor::mend`): worn,
    carried, the keyring and the bank, broken gear giving its stats and auras again; what was sold isn't. While it
    is off nothing takes durability off their gear, each path asking `vendor::wears`: death's tenth (and its
    `SMSG_DURABILITY_DAMAGE_DEATH`, not sent), a fall's, a spirit healer's quarter (resurrection sickness is still
    had), a blow taken or dealt, a thrown weapon's throw, and a spell's wear (its mending still mends);
    `.debug durability` says it can't. On wears as usual again and mends nothing. The flag is the character's
    (`no_wear` in its extra state, false by default), kept across a relog. peon's bots turn it off with their gear.

79. **The calls of review 6, your answers** (2026-10-23; `fix/calls-tap`, `fix/calls-loot`, `fix/calls-social`,
    `fix/calls-server`; the questions in [review/review6.md](review/review6.md)'s calls):
    - **1, the tap:** only damage taps a creature, as mangos-classic (`Unit::DealDamage` → `SetLootRecipient`;
      `map/tap.rs`'s `damaged`): its first damage that gets through, melee, a spell or a tick, a player's or their
      pet's, guardian's or possessed's, for them and their group. An aggro, a call for help, a taunt, a heal's threat,
      a miss or a blow absorbed whole taps no one; the tap outlasts its tapper's death, feign or vanish. Thistle Tea
      taps on combat entry (THISTLE_TEA_DIFFERENCES).
    - **2, overkill in a kill's damage shares:** kept, as mangos-classic and Thistle Tea count it.
    - **3, a possessed player's spellbook:** kept, mangos's (`HandlePetCastSpellOpcode` checks `HasSpell`).
    - **4, a group's gold:** split only among those it fell to who are still on the map (wvs's near-the-kill set, mangos's
      `m_ownerSet`); each gets `gold / n`, the remainder dropped as mangos's `SendGold` drops it.
    - **5, two rogues at one pocket:** a second rogue's Pick Pocket at pockets another has open is refused,
      `SPELL_FAILED_CHEST_IN_USE` ("That is already being used"; mangos's `LOOT_ERROR_LOCKED`), the effect refusing too;
      per-rogue pockets and `picked_before` kept.
    - **6, mail money past the cap:** takes what fits, leaves the rest on the mail (saved, the list sent again); with no
      room, nothing taken and "internal error" (mangos's for a mail it can't take; 1.12 has no gold-cap error).
    - **7, abandoned groups:** a group records when its last online member left (in the store); the sweep deletes one empty
      for `[groups] expire_offline_hours` (24, your 1 day); a login or join clears it.
    - **8, group commands in a battleground:** as mangos: in a battleground raid every command acts on the raid
      (`GetGroup()`) but invite, which acts on one's own group; kick, leave and convert are refused there. Not the
      "own group" the question assumed: mangos has no command reach the own group from a battleground but invite.
    - **9, town guards:** no cap. Town raids are disruptive events; 50 attackers calling 50 guards is meant to be
      hard (your words).
    - **10, the memory guard's timings:** `[network] memory_guard_most_held_secs` (30), `memory_guard_rest_secs` (60),
      `memory_guard_rest_max_secs` (600), all Live (the guard reads them at every look; a longest rest below the first
      takes the first).
    - **11, other client builds' logins:** still answered, for now (your call).
    - **12, a knockback's flight:** ends at its arc, `2·vz/g` (19.29 yd/s²) + 1.5 s, at most 10 s; within it the player
      is no further from the launch than its horizontal speed × the time (×1.25, + 3 yd); past it, the flight lasts only
      while the client's height matches a fall along the arc (2 s for a late packet), so a fall off a ledge goes on with
      no correction, navmesh or not. Stricter than mangos, your call.
    - **13, a far teleport never acknowledged:** kicked as `.kick` does after `[world] worldport_ack_timeout_secs` (60,
      Live, 0 never) from `SMSG_NEW_WORLD`; one whose connection is gone is left to the 60 s linger.

80. **Pets spread round their victim, your ask** (2026-10-23; `feat/pet-fanout`): twenty hunter pets on one unit should
    fan out as mobs on a player do, and other melee pets, the voidwalker, with them. Every melee pet of a player (a hunter's, a
    warlock's Voidwalker, Succubus and Felhunter, a guardian, a charmed creature) chases, stands and spreads as a
    creature does (`ai::chase_goal`, `ai::runs_on`, `ai::spread`, Thistle Tea's `approach_angle_offset` and
    `maybe_spread`), its victim's spots shared by every one fighting it, creatures and pets alike. The imp stands off
    and casts, as before (mangos's `PetAI`, 25 yards and no melee).
    - **mangos-classic** fans out only those chasing a **player** (`ChaseMovementGenerator::Backpedal`, whose
      `FanOut` steps one off another standing within a yard, 36 to 45 degrees round its target); pets chase from
      any side (`PetAttackFromBehind = 0`, the default; 1 sends them behind). Twenty pets on a creature stand on
      each other there. wvs fans out at any victim, as it already did for creatures (Thistle Tea's), and pets
      chase from their own side, not behind.
    - **What the spread now does more than Thistle Tea's,** for creatures too: a step aside goes to a place clear of
      where every other attacker of its victim stands or is headed, so a crowd ends apart in a step or two and then
      stands; two are on each other within their radii combined (Thistle Tea's 1.12 yards, at least); there is no
      turn aside at a victim on the move, any victim (Thistle Tea's, at a player), so those following a walking
      creature keep their sides; and a step's end is on the ground there, not at its victim's height.
    - **An arc, not all round, your report** (`fix/fan-out-arc`): sixteen mobs from your left should fan out over
      100 to 140 degrees on the left, never all the way round; and stand at their attack distance, not at assorted
      ones. Before, Thistle Tea's turn aside as a chaser came in grew with the count past a half turn either way
      (16 attackers, up to 9 radians), and a step aside turned 0.4 to 4 radians from where it stood, three times,
      so a crowd walked round its victim; a step aside also went out 0.8 to 4 yards past its size, and one inside
      its victim backed off to another distance, so they stood at varied depths. Now:
      - **The arc** (`ai::arc_of`, `[creatures] fan_out_arc_degrees`, 120, live; 360 is all round) is centred on
        the mean side its group came from: each melee attacker's bearing from its victim as it last came at it
        (`Engagement::side`, set at each chase; a player's, where it stands), its victim's attackers and the
        chasers still on their way (`Combat::chasers`), so a group's arc doesn't move as the rest arrive. The sides
        are cut into groups where two neighbours are more than 90 degrees apart, so a pack from the left and one
        from the right each keep to their own; with no such gap (fought from all round) there is no arc. The arc
        follows the attackers, not the victim's facing, and is worked out afresh as they chase it again.
      - **One ring:** every attacker of a crowd stands half its reach out (`ai::ring`, mangos-classic's chase,
        `CHASE_DEFAULT_RANGE_FACTOR`, the distance it chased to), out past a victim bigger than that; one off it
        (or any standing inside its victim) steps onto it, and every step ends on it. One alone elsewhere in reach
        stands, as mangos's does.
      - **The approach's turn aside** is Thistle Tea's, no wider than the arc's half, and its end is brought into
        the arc; one that came in before the rest of its group and stands out of the arc steps into it.
      - **A step aside** turns round the ring as mangos-classic's `FanOut` does, 36 to 45 degrees from where it
        stands, then 18 to 30, 50 to 70 and 8 to 16, each way, no further than the arc's edge; it takes the first
        place clear of the others. Where the arc is full (more than about five on a player at 120 degrees), it takes
        the middle of the widest gap left in the arc, or the clearest of those looked at, side by side and
        overlapping a little, if that is clearer than where it stands; else it stays. Three tries, as before.
      - Pets use the same rule; twenty pets from one side fill the arc, side by side.

81. **`.gear green|blue`, for peon's arena** (2026-10-23): `.gear`'s random pick of the uncommon and rare items of the
    character's level (the ten levels up to theirs) narrowed to one quality, a fresh level's greens or a dungeon's
    blues, so a load test's crowd can be dressed as a real server's is; a slot with none of that quality is filled
    from both, as `.gear` fills it. A GM's (level 2), not 1.12's; `.gear` and `.gear raid` are unchanged.

82. **`.gear` fills a hunter's quiver, your ask** (2026-10-23; `feat/gear-quiver`): a hunter gets the best quiver
    (bow, crossbow) or ammo pouch (gun) of its level on the bag bar, filled with the best ammunition, its slots ×
    200 (3,200 at 40 in a Quickdraw Quiver), the ammunition chosen; four stacks loose only when no bag can go on.
    `.gear` is wvs's own, so these are calls of mine:
    - **The bag:** the most slots, then the most haste (the Harpy Hide Quiver or Gnoll Skin Bandolier at 60, the
      epic Ancient Sinew Wrapped Lamina for `.gear raid`). One with no required level (the Quiver of the Night
      Watch, 12 slots, item level 25; the Hunting Quiver) is taken at five under its item level, not at level 1.
      Kept if as good, of the weapon's kind; else it goes where the quiver or pouch worn is (a pouch's bullets with
      it, when a bow comes; arrows kept in a better quiver), else in an empty bag slot, else in place of the
      smallest bag, whose items move elsewhere or, with no room, the bag stays and the reply says so.
    - **The ammunition in it** is all the chosen kind: other ammunition in the quiver goes (as the gear `.gear`
      replaces goes), loose stacks of the chosen kind are moved in, and what is missing is given, so a `.gear`
      again gives nothing (peon's bots say it at every reconnect).
    - **The ammunition chosen** goes by the damage it adds, then level, among what anyone may buy or make for
      `.gear`, `green` and `blue` (a vendor's, a craft's: Explosive Arrows at 37 and Thorium Shells at 52), and
      for `.gear raid` also a boss's drop or a quest's reward (Doomshot from Shadow Hunter Vosh'gajin,
      Miniature Cannon Balls from Cannon Master Willey, Rockshard Pellets from Lord Roccor, Thorium Headed Arrows
      from Alterac Valley's quests). The Ice Threaded Arrows and Bullets (sold only at Honored with Alterac
      Valley's sides) are neither's; nor what is deprecated or no one gives (the "Depricated" arrows, Elven
      Arrows), nor a vendor that is nowhere (vmangos's Jimmy McWeaksauce's Thorium arrows). Before, the highest
      required level won: Jagged Arrows at 40 over the better Explosive.

83. **The Booty Run, scripted, and game events on the server's clock, your ask** (2026-10-23; `feat/booty-run-walk`):
    Short John Mithril should walk down to the arena floor, set the chest down and run back, every third hour on the
    hour from midnight.
    - **His walk is vmangos's own:** `creature_movement_template` has 31 points for 14508 (round the stands' walkway, down a
      stair, the floor by the chest's place at point 14, and back), with
      `creature_movement_scripts` 1450816 at point 14 (run; yell 9731 "Let the Bloodletting Begin!"; put out chest
      12029) and 1450833 at home (walk, stop, gossip and quests back). mangos-classic starts the same path
      (`MoveWaypoint` on `GAME_EVENT_GURUBASHI_ARENA`); vmangos's event-start spell (33013, in
      `game_event_creature_data`) isn't in its spell data, so wvs does what it must: yell 9730, put the gossip and
      quests away, set him walking. The path is walked on the navmesh between its points (the extended
      `short_johns_path_is_walkable_down_to_the_pit_floor`). Mine: he **kneels** at point 14 (`EMOTE_ONESHOT_KNEEL`)
      and stays down 3 s, so the chest comes out while he is down, not as he runs off (vmangos's wait is 1 s, its
      chest at 2 s).
    - **He is kept awake** on his run, so it happens with no one watching; if he can't put the chest down (dead,
      gone, no path), it comes out at the event's start plus his walk (and 13 s to spare). He is faction 35,
      friendly to all: no one in the pit's free-for-all can strike him, so only a GM kills him. A map that comes up
      during the run has him at home and the chest out. An event stopped while he is on his way sends him home at a
      run.
    - **Game events are on the server's local clock,** as vmangos's `GameEventMgr` reads `game_event` (`mktime`), and
      as game time and the weather here already were; before, wvs read the rows as UTC. The schedule is worked out on
      the wall clock (the local date and time as if zone-less), so an event keeps its local hour through summer
      time, where vmangos's, counted in seconds from its start, moves an hour. The Darkmoon Faire's days and the
      holidays' fireworks go by the local date and night too. A GM's `.event start|stop` lasts its minutes, as
      before.
    - **The Booty Run's hours are a setting,** since vmangos's row (23:00) bakes in its UTC+1 server: `[events]
      booty_run_first_hour` (0) and `booty_run_every_minutes` (180), Live, so 00:00, 03:00, ... 21:00 local, two
      hours each (the row's length). Other rows' hours are vmangos's, as they were: on a server not in UTC+1 those
      with that zone baked in (the holidays starting at 23:00 or 22:00, say) start at that local hour.
    - **`.arena booty`** (a GM's) starts it now, as `.event start 16` does.
    - **Raid resets are left on UTC** (`[instances] reset_hour`, 04:00 UTC): their times go to clients as absolute
      times, and a change of clock would move every saved lockout.

84. **A client given its control back is checked from where the server left it** (2026-10-23;
    `fix/fear-position`): at a fear's, a confusion's or a creature's charm's end, a possession's release and a
    charge's end, the speed check's last place is where the server has the player, and its bank holds only
    `distance_allowance_yards` (3) ahead of its speed, not the whole 2 s allowance (about 20 yards) it had
    before the fear. Until a move is taken this holds a GM too, though a GM's moves are otherwise unchecked:
    every player of a GM realm is one (peon's arena runs with `everyone_gm`), and a client that never followed
    the server's run (peon's bots, before their fix) sent its movement from where it stood before, which was taken,
    so everyone watching saw it slide back across the arena. Now that move is pulled back to the fear's end (a
    teleport to its own client). mangos-classic has no speed check here (its anticheat is a stub); a real client
    follows the server's `SMSG_MONSTER_MOVE` for its own guid, which wvs sends it, so it never meets this.

85. **Alterac Valley's world copy, and Drek'Thar's leash** (2026-10-23; `feat/alterac-allin`, for peon's `run av`,
    40 of a side going all in at a general from outside his keep, on map 30 outside any match):
    - **The world copy holds a match's start.** A battleground's map at instance 0 (a GM's `.go` there) spawned
      every event's spawns at once: each of the valley's guards in all four of its upgrades on its spot, both
      sides' graveyard defenders on theirs, all eight of each general's marshals or warmasters, every banner state.
      Now a spawn bound to events comes as at a match's start (`battleground::initial_events`): Drek'Thar with the
      four warmasters of the Horde's towers, Vanndar with the four marshals of the Alliance's bunkers, one guard a
      spot. Warsong Gulch's and Arathi Basin's world copies the same. Nothing else of a match runs there: no
      towers, graveyards, reinforcements, routes, riders or rituals.
    - **The leaders hold their keeps there too.** Their once-a-second sweep (drawn too far, home; fighting, their
      guards called; Drek'Thar's wolves raised after a fight) ran only in a match; it runs on the world copy now,
      its state on the map (`Map::valley`).
    - **Drek'Thar's leash is a line** (your memory of retail: "he will de-aggro if he is pulled any further east than
      x -1358.81, y -254.89, z 99.37 and run back into the keep"; his warmasters can be pulled beyond it). East is
      -y. The point is in the mouth of the one way out of his hall, a corridor 13 yards wide there (x -1364 to
      -1351) running east to the stairs down to the gate; the hall's other walls are closed. So the line is
      y = -254.89 across the corridor, and he goes home once east of it, his spawn 35 yards west. It replaces
      vmangos's 33 yards about his spawn (`npc_DrekThar`'s `GetDistance2d`), which reached the corridor's mouth
      at y -250, five yards short; his whole hall is within 33 yards of his spawn. His warmasters and wolves have
      no leash of his: the wolves are neither his summons nor linked to him (no `creature_linking` or group in
      vmangos's data), so they keep the creatures' own. mangos-classic scripts neither general (their EventAI is
      the world database's, with no such check), so Vanndar keeps vmangos's 35 yards, as do Balinda and Galvangar
      their 45. The extended `drek_thars_line_is_across_the_one_way_out_of_his_hall_on_the_real_navmesh` checks the
      line on the bake: the way from his spawn to the gate crosses it once, his hall's corners are reached
      without crossing it.

86. **A creature leaves the immune be while it has anyone else to fight** (2026-10-23; `fix/skip-immune-target`,
    your rule: "Drek'thar (or any other mob for that matter) should never attack players who are immune to attacks,
    like paladin bubble, IF they have other players in their aggro table"). mangos-classic's rule, followed: one on a
    creature's threat list immune to the school of its blows (Divine Shield, Ice Block, Blessing of Protection against
    a physical creature, not one whose blows are magic) is held below everyone else on the list, their threat kept
    (`STATE_SUPPRESSED`, sorted after the rest in `ThreatContainer::update`). A creature whose victim is held turns at
    its next look to the top of the rest at once, with no tenths to pass; among the rest, and among the held when no
    one else is left, the tenth near and three tenths far decide as before; a taunt holds over it. Alone on its list,
    the immune one stays its victim: it swings into the immunity and doesn't go home. Once the immunity ends they stay
    held until their threat on it changes (a heal, a blow) or another on the list is newly held
    (`m_suppresabilityToggle`, `ClearSuppressed`). A script's or EventAI's pick walks the list as sorted: the second on
    it is the second of those it may hurt; one at random may be anyone, as mangos's. Pets keep their own choice
    (mangos's pets have no threat list and its `PetAI` no such test).
    - **Where it departs from mangos:** mangos holds a target as their immunity goes on, for the creatures that have
      them on their lists then; one that puts them on its list afterwards (a bubbled paladin's heal pulling a new pack)
      treats them as anyone. wvs looks before each choice of victim, so they are held there too, as your rule has it.
    - **Not taken:** mangos also holds the confused, the feared and those in a stun that damage breaks
      (`IsSuppressedTarget`); wvs holds only the immune, your rule. Nor mangos's slip in `ATTACKING_TARGET_BOTTOMAGGRO`
      (it walks from the bottom but returns the top): the last on the list is the last, a held one if there is one.

87. **`.ready save|load` and `.respawn <radius>`, a raid attempt's reset** (2026-10-23; `feat/gm-ready`, your goal:
    "reduce the time it takes to get the raid ready... new GM commands or perhaps have some 'ready' state where the
    players are all buffed and full HP that they can be quickly reset back to"). mangos-classic has neither; both are
    game masters' (level 2), as `.durability`.
    - **`.ready save`** keeps the GM's own ready state on the character (`CharacterState::ready`, in the store's
      JSON, so it survives a relog and a restart): the map, the place and the facing; each helpful aura a cast put on
      (the player's own or another's: Fortitude, Mark of the Wild, a blessing, a warlock's armor, a form or a stance,
      a mount), with its caster, amounts, stacks and charges as they stand, and its full time; and whether a pet is
      out. Not kept: passives, gear's, another's lasting aura of an area (it comes back by itself), an aura another
      aura put on, a channel's, and any harmful aura. A dead GM can't save.
    - **`.ready load`** puts the selected player (or the GM) back in their own saved state. On another map, it sends
      them there and does nothing else: load again there. Else, in order: risen if dead or a ghost (no sickness); the
      cast stopped; out of combat, and every creature forgets them (its threat gone, turning to the next or going
      home); at the saved place; every aura off but passives, gear's and another's lasting area aura (debuffs, and
      buffs had since, a potion's or a trinket's); every cooldown cleared, the client told: spells', items'
      (potions, healthstones), categories', the global ones, those waiting for an aura's end, an interrupt's
      lockouts; each saved buff put on again at its **full** time, not what was left, with its saved amounts, stacks
      and charges, from its saved caster if that player (or creature) is on the map, else from the player
      themselves, so a blessing or a stack stays its caster's; health, mana, energy (and a pet's focus) full, rage
      none, no combo points; the pet, if one was out, revived (a hunter's or a demon's record) at full health, out of
      combat, forgotten, its harmful auras gone and its cooldowns clear. A buff's amounts are the caster's when saved,
      so every attempt starts identical.
    - **`.respawn`** alone keeps what it did: the selected creature dead or alive comes back at its spawn as new at
      the next update. **`.respawn <radius>`** (0 to 500 yards) does it now for every spawned creature within the
      radius of the GM, by where it is or where it spawned (one drawn off is caught), not a player's pet, totem or
      charmed unit: the dead back, the living taken out and spawned again (home, whole, out of combat, its EventAI
      and script state new, its tap gone; a pool picks again). Their summons, and the summons' summons, go first
      (`scripts::remove_for_good`), so the new summoners hear nothing of them. On Alterac Valley (its world copy or a
      match) the leaders' sweep forgets the fights of those respawned (Drek'Thar raises no wolves for a fight ended
      so) and looks again at the next update. Objects within the radius in use (a door, a button, a goober waiting
      to go back) go back as they rest now, and objects gone (used, or a looted chest or node) come back at the next
      update. Not reset: an instance script's encounter data, nor creatures summoned by no one there (a GM's
      `.spawn`).
    - **For peon**, between attempts: one GM `.respawn <radius>` by the keep, then each bot `.ready load`; no wait
      is needed between them, and none after but the next update (50 ms) for the respawned creatures' creates.

88. **The warmasters' and marshals' Charge** (2026-10-23; `fix/av-warmasters`, your question: "the warmasters in the
    room shouldn't be able to charge to targets beside them ... I'm used to them dashing across the room to knock down
    healers when they are casting"). Charge (22911) is range 95, 8 to 25 yards, past both combat reaches as mangos's
    `GetMinMaxRange` measures it (11 to 28 yards between a warmaster's centre and a player's); its blow is the weapon's
    and 75, and it stuns 2 s. vmangos's `npc_WarMaster` (Thistle Tea's port) charges its victim only, from 8 to 25
    yards; no source (vmangos's script, its unused EventAI rows, mangos-classic) charges a caster or a random target,
    so none is invented: a healer who takes the top of its list by healing is charged across the room, as you
    remember, and one beside it never is. What was wrong, fixed:
    - **A refused Charge spent its timer.** The event fired from 8 yards between centres, the cast was refused below
      11 (too close), and the warmaster then waited 12 to 18 s, past a victim who had stepped into the band. Now its
      timer starts over only once the Charge goes (vmangos's `DoCastSpellIfCan(..) == CAST_OK`), and it looks again
      at its next tick.
    - **EventAI's range event** (`EVENT_T_RANGE`) measured between centres; it is mangos's `IsInRange(victim, min,
      max, true, true)` now: past both bounding radii, from the least, short of the most.
    - **The Elite Guards' healer pick** (your twist on this Charge) measured 8 to 25 yards between centres,
      and could pick a healer the cast then refused, every second; it measures past both combat reaches now.
    - Already right: the cast checks refuse a creature's cast inside a spell's least as a player's
      (`SPELL_FAILED_TOO_CLOSE`), and a script's random pick for a cast is one in the spell's band.

89. **Improved Scorpid Sting** (2026-10-23; `fix/talents-server`, the hunter audit). A hunter's Scorpid Sting going on
    casts 19486 on the stung: 10, 20 or 30% (the talent's override scripts 2388 to 2390) of the strength the sting
    takes, as stamina, for the sting's time, and it goes with the sting, as mangos's `HandleAuraModStat` and
    `RemoveAurasTriggeredBySpell`. Two departures from mangos-classic: it looks for the script on the **hunter**
    (mangos looks on the stung, who never has the hunter's talent, so its talent never works), and it takes the
    share of the **strength** effect alone (mangos's handler runs for each stat effect, so its agility effect's cast
    would replace the strength one's; the tooltip says strength). Thistle Tea has no handler.

90. **Improved Firestone on the Firestone's fire damage** (2026-10-23; `fix/talents-server`, the warlock audit). A
    worn item's aura now takes its wearer's spell modifiers, as mangos's `CalculateSpellEffectValue` (all effects'),
    and a bonus to damage done (`SPELL_AURA_MOD_DAMAGE_DONE`) takes their damage modifiers too. That second part is
    not mangos-classic's: it modifies no aura's amount by `SPELLMOD_DAMAGE`, so the half of Improved Firestone that
    names the Firestones' equip auras ("Increase Fire Dam", family flag 0x200000, operation 0) does nothing there.
    With it, a Major Firestone's 21 fire damage is 27 at rank 2 (30%). The Firestone's proc already took the
    talent. A modifier coming or going puts the worn auras it touches on again.

91. **Improved Drain Mana** (2026-10-23; `fix/talents-server`, the warlock audit). Each tick of a warlock's Drain
    Mana deals 15 or 30% of the mana it took (the talent's override scripts 2028 and 2029) as the drain's own Shadow
    damage, through the target's modifiers and resistances as a tick's. Neither mangos-classic nor Thistle Tea has
    it, and the talent's auras carry no amount, so the shares are the tooltip's.

92. **Divine Favor's mask and charge** (2026-10-23; `fix/talents-server`, the paladin audit). mangos-classic takes
    20216's mask from the DBC (0x206000: Holy Shock's 0x200000 and the script heals 19968 and 19993 at 0x4000 and
    0x2000). vmangos's `spell_effect_mod` makes every Holy Light rank a direct heal with its own flag 0x80000000, so
    with the DBC's mask Divine Favor never touched Holy Light. wvs ORs 0xC0000000 (Holy Light and Flash of Light)
    into the mask, as mangos's old `spell_affect` 0xC0200000 did and as 1.12's tooltip says. Its one charge is also
    spent by the heal or strike a script casts in the cast's place (Flash of Light's 19993, Holy Shock's 25912 and
    25914 and their ranks), as mangos's `RemoveSpellMods` does for the modifiers a spell applied; wvs otherwise
    spends charges only on casts not triggered. Before, a Holy Shock crit for as long as Divine Favor stayed up,
    which was for good.

93. **A warmaster and Drek'Thar stand while they whirl** (2026-10-23; `fix/av-fidelity`, a fidelity finding: wvs
    followed Thistle Tea's port, whose moduledoc says "whirlwinding leaders keep moving"). vmangos's C++
    (`battleground_alterac.cpp`) is the reference, followed:
    - **The warmasters and marshals** (`npc_WarMaster`): its first Whirlwind (13736, from 12 s) sets
      `m_uiIsInWhirlwind_Timer` to 4 s and the next Whirlwind 2 s on; the second sets it 11 to 13 s on. While the
      timer runs it has `UNIT_STATE_ROOT`: it fights in reach but doesn't chase. Each cast sets a script global
      cooldown (Whirlwind 2 s, Cleave, Shout and Enrage 1 s, Charge 250 ms) under which nothing else is cast and
      the other timers stay due. So for the 4 s it neither moves nor Charges, Cleaves, Shouts or Enrages. wvs's
      warmaster kept chasing, and could Charge into the ranged and whirl among them. Now its Whirlwind event turns
      combat movement off and puts it in a phase of its own (`av_scripts::WHIRLING`, 2, clear of a general's 0 and
      1), which its other events skip; EventAI looks at an event's phase before its timer starts over, so one due
      meanwhile goes at the first look (a second at most) after. Two scripts of their own follow: the second
      Whirlwind at 2 s, and at 4 s combat movement on and phase 0 (apart, so a refused second cast, a stun, can't
      leave it rooted). Its evade, which stops its scripts, puts movement and phase back. The Elite Guards are the
      same fight (`elites.rs`), and stand too.
    - **Drek'Thar** (`npc_DrekThar`): his Whirlwind (8 to 12 s, then 12 to 16) roots him 2 s, under a 2 s global
      cooldown, so no Knockdown (19128) or Frenzy (28747) in it: the same, at 2 s and one cast. His evade stops the
      end of the whirl and moves him again; one mid-whirl takes his phase back to fighting before his lines, so a
      wipe still says his wipe line, and a leash his (the leash sets its own phase first).
    - Combat movement on, as the end of the whirl sets it, overrides a sessile template's standing
      (`event_ai::combat_movement`); none of these creatures is sessile (vmangos's `static_flags1`, the Elite
      Guards' looks' too), so it is what they had.
    - **Whirlwind's blow, checked, right:** 13736 is a 2 s aura whose 2 s period triggers 15589, a weapon blow
      (effect 17) of 300 and 6 a level from spell level 1, no most level: weapon damage and 654 at 60 (the
      warmasters'), 666 at 62 (Drek'Thar's, the Elite Guards'), as vmangos's `CalculateSpellEffectValue`; wvs's
      `spells::points` gives the same (the extended `the_real_whirlwind_is_a_weapon_blow_and_654_at_sixty`).
    - **Not taken:** the global cooldown outside the whirl (Cleave's, Shout's and Enrage's 1 s, Charge's 250 ms
      holding the others back); EventAI looks once a second, so two due together go together. And a Charge with no
      victim in its 8 to 25 yards: vmangos's waits 5 to 10 s before looking again; wvs's looks each tick, as Thistle
      Tea's and mangos's EventAI range event do (decision 88).

94. **A reset leaves nothing of the last pull** (2026-10-23; `fix/reset-inflight`, your bug: "in-flight abilities
    like imp Firebolts can still aggro after .reset"). A spell on its way (`InFlight`, an Auto Shot's arrow) was
    kept through `.ready load` and through a respawn: `.ready load` stopped only the cast under way, and only the
    player's and a saved pet's; and a respawned creature keeps its spawn's GUID, so a bolt cast at the old life
    landed on the new one and pulled it. mangos-classic has no such reset; it is put together from what its pieces
    do:
    - **`.ready load`**, for the player and all that fights for them (pet, charmed unit, totems, guardians), saved
      with a pet or not: their totems and guardians unsummoned (a Searing Totem or an Infernal would pull the next
      creature it saw), and their traps; every cast and channel stopped (`InterruptNonMeleeSpells`), the auto-repeat
      shot too; what they cast that is on its way lost, and what is on its way to them (`Spell` events killed);
      their ground areas and dynamic objects gone (`RemoveAllDynObjects`: Blizzard, Rain of Fire, Consecration);
      every harmful aura of theirs on another unit gone (a damage over time ticking on a creature would bring it
      back); out of combat and forgotten. The pet stands down as mangos's `AttackStop(true, true)` and a Follow:
      no victim, no target, no Attack order or opener, its chase stopped, on its way back. `.ready save` now keeps
      the pet's stance (passive, defensive, aggressive), and `.ready load` puts it back; a state saved before keeps
      the stance the pet has.
    - **A creature leaving the world or coming back as new** (respawned by `.respawn`, its timer or a script,
      its corpse gone, despawned, removed for good): what it cast that is on its way, and what is on its way to it,
      is lost, and its ground areas go (mangos's `RemoveFromWorld`). A spell or a shot that lands also checks the
      creature's spawn time against its own cast: one at, or from, a creature come back as new since is lost.
    - **Not taken:** a player's other summoned objects (a portal, a campfire) stay; a quest object's spell landing
      on its user, Alterac Valley's scripted timers (`av_scripts`), and helpful auras on others are left alone;
      `.respawn` takes no player's ground area there (each bot's `.ready load` does).

95. **A warmaster's Charge and the linked guards, as vmangos's C++** (2026-10-23; `fix/av-fidelity-2`, round three of
    the AV all-in, taking up what decision 93 left; amends decision 85's guards). vmangos's
    `battleground_alterac.cpp` is the reference, followed:
    - **The Charge** (`npc_WarMasterAI`): due from the fight's start, its timer runs out and, with no global cooldown,
      it looks for `GetVictimInRange(8, 25)`: the first of its threat list, the most threat first, from 8 to 25
      yards between bounding radii (`IsInRange`, 3D), its victim or not. A cast that goes starts its timer over 12 to
      18 s on; one refused (in range, short of the spell's least past both combat reaches) leaves it due, looked at
      again at its next tick; with none in range it waits 5 to 10 s. wvs had mangos's EventAI range event at its
      victim alone, looked at each tick (Thistle Tea's). Now an event of wvs's own (`EVENT_THREAT_IN_RANGE`, 1000:
      the first of the threat list in range, fired at that one) and a wait of its own when its test fails with a
      victim (`AiEvent::miss_repeat`, set only by `av_scripts`; no database has it). The marshals and the four
      warmasters by Drek'Thar alike (they share the script); the Elite Guards keep their own pick (`ChargeAt::Picked`).
      So a warmaster on a tank with a healer high on its list 8 to 25 yards off charges the healer.
    - **The linked guards** (`npc_alterac_bossHelper::AggroLinkedMobsIfNeeded`): each of the general's linked
      guards within 100 yards, alive and out of a fight, gets `AttackStart` at every unit on his threat list, the
      most threat first; each adds its target to its own list with no threat (`AddThreat` of 0) and switches its
      victim, so it ends on the last, the least of his list, and the first who does it harm or heals against it takes
      it (anything past none). wvs's went at his victim alone (Thistle Tea's choice, in 85). Now they take on his
      whole list, none with threat of its own, fighting its last; with an empty list, his victim. The world copy and
      a match alike.
    - Tests: `drek_thars_warmaster_takes_on_his_whole_threat_list_and_fights_its_last`,
      `a_warmaster_with_no_one_to_charge_looks_again_five_to_ten_seconds_on`, and the real valley's
      `a_real_warmaster_charges_the_one_it_fights_across_the_hall_not_the_one_beside_it` (Bob, top of its list
      across the hall but short of taking it from Alice, is charged) and the marshals'
      `a_warmasters_charge_waits_past_the_spells_least_range_and_goes_at_once_beyond`, updated.

96. **Two warlocks' soulstones on one friend, as vmangos** (2026-10-23; `fix/av-fidelity-2`, the user's question for the
    AV all-in). Soulstone Resurrection (20707, 20762 to 20765) is a dummy aura of no class (family 0), all five ranks
    of one icon (92) and visual (99). vmangos (`Unit::AddSpellAuraHolder`, `RemoveNoStackAurasDueToAuraHolder`,
    `SpellMgr::IsNoStackSpellDueToSpell`): the same rank from two casters stands together (a dummy from another caster
    is allowed to stack); another rank, from anyone, takes the first one's place (its "more generic checks": one icon
    and visual, one family, alike helpful, not a modifier, don't stack; family 0's ranks are no ranks there, so none
    is kept for being higher). wvs let both ranks stand. Now `auras::replaces` has vmangos's generic rule for spells of
    family 0 whose auras are all dummies (kept that narrow: the case wvs had otherwise); one rank from two warlocks
    still stands together. Test: `another_warlocks_soulstone_of_another_rank_takes_the_first_ones_place`. peon's
    warlocks keep to one stone a friend anyway (none on one with a stone from anyone).

97. **`.ready load` ends a spirit of redemption** (2026-10-23; `fix/av-fidelity-2`, found by the AV all-in's
    telemetry: priests "killed by nothing" 1 to 12 s into an attempt, 9 deaths in 5 of 66 attempts). A priest who
    died into a spirit near an attempt's end came back from `.ready load` whole but still with the spirit's state
    auras: 27792 and 27795 are passive, and the load clears all but the passive and the gear's; 27795's end killed
    him (`redemption::aura_gone`). Now the load ends a spirit first (`redemption::end_without_death`, as leaving a
    battleground does), the talent kept. Test: `ready_load_ends_a_spirit_of_redemption`.

98. **No rage from an avoided swing; an avoided ability's refund as mangos-classic** (your call, 2026-10-24;
    `fix/warrior-calls`, from the warriors' damage round). Departs from Thistle Tea, which wvs followed until now.
    - **White swings:** wvs gave a dodged or parried swing three quarters of the rage its damage would have given
      (Thistle Tea), a miss none. mangos-classic gives rage only from damage dealt: `Unit::DealDamage` calls
      `Player::RewardRage(damage, true)` with the damage done, which an avoided swing has none of. Your call was "give
      none": now a miss, a dodge and a parry give the attacker no rage. A blocked swing still gives rage from what
      got through, as before. The victim's side is unchanged: none from a blow it dodged or parried (mangos's
      `hitOutCome != MELEE_HIT_DODGE && != MELEE_HIT_PARRY`), and none from nothing taken.
    - **Abilities:** mangos-classic's `Spell::finish` gives back `int32(float(m_powerCost) * 0.8f)` of the spell's
      own power for each target that a spell with `SPELL_ATTR_EX_DISCOUNT_POWER_ON_MISS` missed, or that dodged,
      parried or deflected it (`SPELL_MISS_MISS`, `_DODGE`, `_PARRY`, `_DEFLECT`); nothing on an immunity, a block, a
      resist, an evade or a reflect. wvs had Thistle Tea's: 82%, rounded; rage only on a dodge or a parry, energy also
      on a miss and an immunity, mana never. Now mangos's: 80%, truncated (15 rage back 12, 40 energy back 32), on a
      miss, dodge, parry or deflect, in whatever power the ability costs (no mana spell in 1.12's data has the
      attribute). Overpower and Execute carry the attribute in `Spell.dbc`, so they are refunded too: Execute 80% of
      its base cost, and the rest of the rage is kept (mangos's `WarriorExecuteDamage` empties it only on a hit).
      Not followed: mangos rolls Execute twice, the dummy (5308, whose roll decides the refund) and the strike it
      casts (20647, whose roll decides the damage); wvs rolls once, for both.
      Whirlwind and Cleave have no attribute and get nothing back, as before.
    - Tests: `a_swing_gives_rage_when_it_lands_and_none_when_missed_dodged_or_parried`, `a_dodged_swing_gives_no_rage`
      (through the protocol), `avoided_blows_and_abilities_give_power_back`,
      `an_avoided_ability_gives_back_its_rage_or_energy_as_its_outcome_allows` and
      `a_dodged_rage_ability_gives_back_four_fifths_of_its_cost`.

99. **Spirit of Redemption lasts 10 s** (2026-10-24; `fix/redemption-10s`, your call: "take the client's version, 10
    seconds"). wvs had Thistle Tea's 15 s; the client's Spell.dbc gives the spirit (27827) duration index 1, 10,000 ms,
    as mangos-classic reads it. Test: `spirit_of_redemption_heals_on_then_dies`.

100. **An ability's crit is its crit chance of its hits** (2026-10-24; `fix/rogue-dps`, from the rogues' damage check).
     A melee or ranged ability rolled its hit on the table (`outcome::melee_roll`), then, as it landed, rolled the
     whole table again and crit only if that second roll came up a crit: its miss, dodge (and a creature's parry
     and block from the front) were taken off its crit chance a second time. At a level-63 boss from behind that is
     7 to 15% of the chance lost: a raid-geared Combat rogue's Sinister Strike crit 11.8% of its hits for its 13.6%
     (the test's, without Precision). mangos-classic rolls the crit once, past the hit (`Spell.cpp`'s
     `target->isCrit = Unit::RollSpellCritOutcome(...)`: `roll_chance_combat(CalculateSpellCritChance)`, a sitting
     player crit by a melee ability). Now a hit (not a partial block) crits on `outcome::melee_crit`'s single roll,
     a sitting player always. Every class's abilities: Sinister Strike, Backstab, Eviscerate, Bloodthirst, Mortal
     Strike, Aimed Shot; and Seal Fate's points with them. Test: `an_abilitys_crit_is_its_crit_chance_of_its_hits`;
     the rogue's numbers checked beside it (`a_rogues_blows_and_energy_are_1_12s`,
     `deadly_poison_stacks_to_five_on_a_boss`,
     `a_rogues_threat_is_its_damages_71_feint_and_vanish_take_it_off`, `seal_fate_adds_a_point_on_a_crit`).

101. **A flight master's guardians are two at a time** (2026-10-24; `fix/flightmaster-guards`, from your report:
     "only 2 should ever be active at a time, even if attacked on separate occasions"). A flight master's aggro event
     (`creature_ai_events`, flags 0: once a fight) runs a script of two summon steps (Dungar Longdrink's 35201: two
     Enraged Gryphons, 9526; Gringer's two Enraged Wyverns, 9297; Vesprystus's two Enraged Hippogryphs, 9527; Michael
     Garrett's two Enraged Felbats, 9521), each set on the master's victim (attack target type 1). Three things in wvs
     let them pile up:
     - **Gryphons and wyverns never went** (a bug, now vmangos's). Their steps have no time (`datalong2` 0) and
       despawn type 4 (`TEMPSUMMON_TIMED_DESPAWN_OUT_OF_COMBAT`); wvs gave a summon with no time no timer at all, so
       they lived on after every fight. vmangos's `TemporarySummon::Summon` sets the timer to 0, and its `Update`
       unsummons one of type 1, 2 or 4 the moment it is out of combat, alive or dead. Now a summon of those types
       with no time goes as its fight ends (89 rows of `creature_ai_scripts`, every flight master's, and 6 of
       `generic_scripts`).
     - **A creature pulled again by a blow fired no aggro events** (a bug). An evade left `combat_start` set, and a
       blow puts the creature in combat before it hears of its attacker, so its next fight wasn't taken as a new
       one: no aggro event, no leash begun. vmangos's `CreatureEventAI::EnterCombat` fires `EVENT_T_AGGRO` at every
       fight's start. Now an evade clears it.
     - **Not vmangos's (yours): a once-a-fight event's summons are capped at one firing's.** vmangos re-arms the
       event every fight and summons two more each time: the gryphons go with the fight, so two is the most there,
       but the felbats and hippogryphs (type 1, 30 s) wait 30 s past their fight, and a pull within that has four;
       a master pulled again while its gryphons still fight someone has four too. Now a summon step run by an
       EventAI event that isn't repeatable (`EFLAG_REPEATABLE` clear) calls no more of its creature than that
       script's summon steps bring at once, counting those it summoned earlier that live; at the cap it keeps one,
       and sets one out of its fight on the new victim, as a new summon would be. One killed is replaced at the next
       pull. Generic: any once-a-fight summon, a boss's at a health mark too; a repeatable event's summons (adds on a
       timer) are uncapped, as vmangos's.
     - Tests: `a_flight_masters_gryphons_are_two_however_often_it_is_attacked`,
       `felbats_waiting_out_their_time_are_called_back_not_called_again_summon` (through the protocol), and
       `vmangos_flight_masters_summon_two_guardians_a_fight` (extended: Dungar Longdrink and Vesprystus).

102. **A crowd's update packets** (2026-10-24; `perf/crowd-1600`, the user's goal: 800 a side fighting in one spot
    in Hillsbrad, the map's update within its 50 ms; numbered after master's 101). Made without the user; none
    changes a rule of the game (nothing a crowded client is sent is capped, no relay is slowed), so none has a key
    of its own. docs/PERFORMANCE.md, "A battle of 1,600 in one spot".
    - **Values blocks a crowd gets alike, in packets shared among it** (`map::values`). mangos-classic builds one
      `SMSG_UPDATE_OBJECT` per player per update with every block for them (`UpdateData`); wvs did the same, a batch
      per observer. Now an object's public values block for 8 or more observers goes in packets broadcast to those
      who get every block in them, after each client's own batch (its forgets, creates and own fields); the rest as
      before. Each client gets the same blocks, each once, and nothing for an object before its create: only the
      packets differ (no more of them: about 75 update packets a second a client at 1,600, against 91). Fewer than
      8 observers keep the one packet (a create and its values together, as the client check has them).
    - **A crowd's creates compressed across visibility's threads** (`[visibility] refresh_threads`, which now
      compress the update's packets no two clients share as well as working out visibility's decisions): each
      packet compressed from its own bytes alone, so what is sent is the same; the key's 1 keeps both on the map's
      thread.
    - **Visibility's chunk sized to its budget** (`[visibility] refresh_budget_ms`): as many refreshes as what is
      left of the budget has room for at what one cost in the last, 16 to 128 (a budget of 0 is still a whole chunk
      of 128 an update). In a charge some refreshes wait for the next update where before a chunk of 128 ran over
      the budget by 20 to 50 ms; over the budget they waited before too, oldest first.
    - **A client held in an update step before visibility's is let go at the step's end** (`Map::let_go_waiting`)
      if frames wait for it, as at an event's end: its creates first, then what waited. Before, it waited for the
      update's "held" step, every broadcast meanwhile copied into its hold. Each client's packets come in the same
      order.
    - Tests: `map::values`' own, `tests_crowd_values`, `tests_held_steps`,
      `update::batches_framed_across_a_pool_are_framed_as_one_by_one`, `visibility::a_chunk_is_sized_to_the_budget_left`,
      `tests_client_emu::a_crowds_shared_values_pass_the_client`; benches `battle` and `values`.

103. **Vanndar's global cooldown, and the AV scripts' notice radii, as vmangos's C++** (your call, 2026-10-24;
     `fix/vanndar-fidelity`, from the Horde at Vanndar: "Yes wait 1s between his three spells" and "Match vmangos
     here, agreed"). vmangos's `src/scripts/battlegrounds/battleground_alterac.cpp` (development, `a9fa07aa`) is the
     reference, followed:
     - **A general's global cooldown** (`npc_VanndarAI::UpdateAI`): each of Avatar (19135), Storm Bolt (19136) and
       Thunderclap (15588) is cast only with `m_uiGlobalCooldown` at 0, and a cast that goes (`DoCastSpellIfCan(..)
       == CAST_OK`) starts its own timer over and sets the cooldown to 1 s; while he casts it is held
       (`IsNonMeleeSpellCasted`). One due meanwhile stays due. wvs let all three go in the same EventAI tick. Now an
       event may carry a C++ script's global cooldown (`AiEvent::gcd_ms`, wvs's own, set only by `av_scripts`): due,
       it waits while the creature's (`Scripted::gcd_until`) runs or it casts; its cast that goes sets it; one
       refused sets none and stays due (`EF_CHECK_RESULT`, the cast step `SF_ABORT_ON_FAILURE`). A fight's start
       clears it, as `Reset`. Due together, they go in the script's order a second apart: Avatar, Storm Bolt,
       Thunderclap. **Drek'Thar** (`npc_DrekTharAI`) has the same mechanism, so has it too: Whirlwind 2 s (its
       whirl already held the others for its 2 s; decision 93), Knockdown and Frenzy 1 s. Not the marshals and
       warmasters: their 1 s and Charge's 250 ms stay as decision 93 left them (not asked; EventAI looks once a
       second, so a 250 ms cooldown would be a whole tick).
     - **Notice radii** (`MoveInLineOfSight`): with no victim, `npc_Vanndar` attacks a hostile unit at
       `pWho->GetDistance(m_creature) < 23.0f`, `npc_DrekThar` `< 22.0f`, and `npc_WarMaster` (all 16 marshals and
       warmasters, 14762 to 14777) `< 22.0f` at a valid target, then casts Charge (22911) at it, its Charge timer 12 s
       on if it goes. `GetDistance` is 3D past both bounding radii (`SizeFactor::BoundingRadius` by default,
       `Object.cpp`), with no level difference, no detect-range aura, no line of sight, no height or way-there check
       (those are `BasicAI::MoveInLineOfSight`'s, which the scripts replace; it measures between centres at
       `GetAttackDistance`). The stealth check stays (`CallAIMoveLOS`'s `IsVisibleForOrDetect`). wvs used the
       database's detection by level for all of them: 20 yards at an even level between centres, so 22 for the
       level-62 generals at 60 and 20 for the marshals. Now `ScriptData::notice` (wvs's own, set by `av_scripts`)
       gives an entry a radius of its script's, and `ai::script_notice` looks with it in place of `aggro_scan`'s
       range; an `Opener` casts a spell at the one noticed and puts the event it names off. So **Drek'Thar** keeps
       22 yards (vmangos's, the same as wvs had at 60, but now past both radii, so about a yard and a half further
       between centres, at any level), and the warmasters by him and the marshals by Vanndar notice at 22 and charge.
       One departure: `[creatures] aggro_rate` still scales the radius (vmangos's scripts don't take
       `CONFIG_FLOAT_RATE_CREATURE_AGGRO`; at its 1, the same).
     - **Not taken, found:** Balinda and Galvangar have the same override at `< 28.0f` (`npc_Balinda`,
       `npc_Galvangar`); not asked, so they keep the database's (20, so 21 at 60); one line in `av_scripts::leaders`
       each. And vmangos links Drek'Thar's guards as 12121, 12122 and 14772 to 14777 (`AddLinkedMob`), where wvs
       (decision 85) has 14770 to 14777, two more warmasters.
     - Tests: `vanndars_three_spells_are_at_least_a_second_apart`,
       `vanndar_notices_a_player_at_twenty_two_and_a_half_yards_not_at_twenty_three_and_a_half` and
       `a_marshal_notices_at_twenty_two_yards_and_charges_the_first_it_notices` (`tests_av_vanndar`, through the
       protocol; the first skips the map's clock). `respawn_around_resets_drek_thars_keep_his_warmasters_back`
       now makes Alice a god before she lands in his hall: his warmasters charge her as she lands, as vmangos's
       would, and killed her (level 1) before the test's first look about 4 times in 5.

104. **The AV scripts' notice needs sight** (2026-10-25; `fix/av-notice-los`, your call: "there's supposed to be line
     of sight"). Decision 103 took vmangos's scripted notice for Vanndar, Drek'Thar and the marshals and warmasters
     (23 and 22 yards past both radii), and with it its bare distance test: vmangos's scripts override the default
     notice (`BasicAI::MoveInLineOfSight`, which tests `IsWithinLOSInMap`), and its visibility path tests line of
     sight only for a stealthed unit, so there they notice and charge through walls. Drek'Thar's warmasters charged a
     raid standing outside his hall's wall. Now a scripted notice takes only one in sight, by the test every
     creature's aggro uses (`assistance::sees`). Departs from vmangos. Test: `vanndar_notices_no_one_behind_a_wall`.

105. **The fight record** (2026-10-25; `feat/fight-record`, asked for: "record every character's position, every
     ability ever used even by npcs and players, all positions, etc so it can be debugged and saved"; the calls in it
     not yet yours). A map copy can write what it believes and tells its clients (`map/record.rs`, `[record] maps`,
     `every_ms`, `dir`, a GM's `.record on|off|mark`; docs/TESTING.md, "The fight record"), and peon's `--record`
     turns it on for a run. Mangos and Thistle Tea have nothing like it. The calls:
     - **JSON lines, not a binary format.** A line an event, its time and kind first, readable with `grep` and `jq`
       and by peon's script without a converter. Its cost is the writer thread's, not the map's: the map pushes the
       events as they are (an enum's copy on a list), and the thread formats them.
     - **Who is sampled:** every player, and the creatures awake (seen by a player or in a fight: a creature no one
       sees doesn't move); a unit's line only once something in it changed, or a second on, so a still crowd costs
       little. Between samples, a unit's moves and casts are the events'.
     - **The ground** under a unit is the navmesh's surface nearest it (`Nav::ground`: the bake's real geometry, its
       terrain's, WMOs' and doodads' upward faces, not the navmesh's polygons), worked out by the recorder's thread
       on a navmesh of its own (the map's bake; memory for the tiles it reads), so the map's thread looks up none:
       at 3 us a column, a moving crowd's grounds were most of the recorder's cost. `dz` is signed, so a unit sunk
       into a floor reads under it rather than over the floor below.
     - **Server-driven moves** are found at each sample by their spline's id (a creature's walk, a player's
       charge or fear), not at each of the seven places that start one, and stamped with when they began: two
       begun within one sample's 100 ms show as the second, and a halt is the spline's end before its time. So are a
       creature's victim and its evade, to the sample.
     - **Threat** is written for elites and above (`rank` 1 to 3), their highest ten, when it changed at a sample;
       every creature's victim as it changes, and every notice with how (`aggro`, `script`, `assist`) and how far.
     - **A GM's file name** is letters, digits, `-`, `_` and `.`, never starting with a dot: the record stays in its
       directory. Recording is a game master's (`gm::needed_level`), as the other world-changing tools.

106. **On their floors in Drek'Thar's keep: pets beside their masters, creatures' walks and a charge's end**
     (2026-10-25; `fix/av-bugs`, from your reports at Drek'Thar: "some players are floating slightly elevated in
     drek's room", and pets falling through his room's floor; found and proved with the fight record, decision 105,
     and peon's watcher, `peon::seen`). Three rules, the calls in them mine:
     - **A pet's place beside its master** is mangos-classic's (`FollowMovementGenerator::_getLocation`,
       `WorldObject::GetNearPointAt`): two yards out at its angle, on the ground there, no further above or below
       its master than that and both their radii, and in its master's sight; else the same at angles further round
       each way (45° steps); none, its master's own spot. wvs took the place where it fell, at its master's height
       onto the ground nearest that: by a wall or a doorway the ground was the terrain under the keep, 8 yards down,
       or a wall's top, and the pet walked there, down through the floor and back up (29 times in 5 attempts of the
       all-in, the record's "server splines" across a step). **And with no way over the navmesh** a pet walks
       straight only where nothing stands between and the ground is a step's climb (2.5 yd); else a follower is
       put at its place at once, as mangos-classic's follow unsticks one (`_setTargetLocation`: no path, relocated
       to its place, which its master sees), and a chaser stands. wvs walked any such way straight: through the
       keep's walls. Test: `a_pet_in_drek_thars_keep_keeps_to_its_masters_floor` (extended, the real valley).
     - **A creature's walk is on the floor itself**: each point of its path (and the points put in where a leg
       leaves the ground, every 2 yards) is at the bake's geometry's surface (terrain, WMOs', doodads' upward faces)
       nearest the navmesh polygon's height within 1.5 yards, not at the polygon's height, and a leg may leave it
       by 0.15 yards, not 0.5. The polygons are a quarter yard and more off the floor, and by a step the polygon
       nearest a point is the one above (the ring round his room's sunken middle, a yard up): his guards walked the
       middle at the ring's height, up to a yard in the air, standing there up to three quarters of a yard up (the
       record: 134 of 505 walk points over a third of a yard). Grounding a path costs 7.2 to 7.8 µs against 3.7 to
       3.8 µs before (`probe_path_on_the_ground`, 2,000 paths in Tanaris; 5,641 points against 4,550). Test:
       `drek_thars_guards_walk_his_room_on_its_floor`.
     - **A charge ends on the floor**: its last leg, cut short of its target, had the height between its ends; now
       the floor's there (within 2 yards). A charger into his room's middle stood a third of a yard in the air
       (peon now lands its raiders where the server leaves them, as a client drops onto its floor). Test:
       `a_charge_down_into_drek_thars_room_ends_on_its_floor`.
     - **The record, before and after** (ten attempts each of peon's all-in at 40, seeds 1 and 2; peon's own fixes
       with them): server splines across a step or a wall 25 to 0; creatures in his room over the floor past a third
       of a yard, of their time, 9.6% to 0.6% (moving, p90 +0.47 to +0.06 yd, p99 +0.77 to +0.29); their spline
       points over a third of a yard 25% to 2% of walks', 14% to 0% of chases', 27% to 0% of charges'; pets in his
       room moving, p99 +1.13 to +0.24 yd, the most +5.67 to +0.45; falls through a floor 5 to 1 (a Frostwolf
       Legionnaire at the record's start, its spawn). Drek'Thar killed 10 of 10 both.

107. **On the ground outside Drek'Thar's keep, and a body killed in the air** (2026-10-25; `fix/av-floating`, from
     your report of floating at Drek'Thar: bots killed in a jump lying in the air, and the valley's creatures afloat
     as they walk its slopes; found with the fight record, decision 105). The calls in it mine:
     - **A formation's followers walk over the navmesh**: the navmesh stays in the map through a creature's step
       (`world::step_creature`), so what the step sets off (its formation following it, a script at a waypoint)
       finds its way over it too. It was taken out for the step, and a leader's followers (the Frostwolf Legionnaires
       and their Bloodhounds behind a patrol's leader) walked straight to their places at their leader's height, up
       to 1.5 yd in the air over the valley's slopes and 0.75 under them, and across what the navmesh doesn't join:
       most of the creatures' time over the ground outside the keep. mangos-classic's followers path to each place
       and put every point on the ground (`FormationMovementGenerator::BuildPath`: `PathFinder::calculate`,
       `UpdateAllowedPositionZ`).
     - **A path's corners on the ground**: a corner is on its polygon's edge, over none of them to the float, so its
       ground was none and it kept the navmesh's own height (a yeti 1.17 yd over its cave's floor, round a rock); now
       the ground of the nearest polygon's nearest edge, as Detour's `closestPointOnPoly` has it. A path's first point
       is the walker's own place, as mangos-classic's `MoveSplineInit::Launch` puts it (`args.path[0] =
       real_position`), not the navmesh's on its polygon's edge (a Legionnaire set off on a tile's edge 2.8 yd up). The
       floor is looked for within a step's climb of the navmesh (2.5 yd, was 1.5: by Tower Point the detail mesh is
       1.64 yd over the terrain), and a leg is looked at every yard, not two (at two, legs left the ground by up to
       0.31 yd between their samples, at one and a half 0.44; at one, 0.20 at most). mangos-classic's smooth path has
       a point every 4 yd at the detail mesh's height (`findSmoothPath`, `SMOOTH_PATH_STEP_SIZE`), on the ground only
       with its `PathFinder.NormalizeZ`, off by default (`NormalizePath`, `UpdateAllowedPositionZ`); wvs's are on the
       ground always, as decision 106 has them.
     - **A walk the navmesh has no way for goes straight on the ground** (`Nav::straight`): its end on its floor and
       points put in where the line leaves the ground, as a path's legs; a point off the navmesh takes the surface
       nearest it. It was the line itself: a chase to a goal under a doodad's overhang on a Tanaris dune (two
       surfaces 3 yd apart, its polygon joined to nothing) went through the dune and ended at its victim's height,
       1.03 yd off the slope: the flaky `a_chase_down_a_slope_keeps_to_the_ground` (after its first chase, its
       victim's earlier attackers spread its goal's side at random), neither a tile's loading nor a long leg.
       mangos-classic's chase goal is on the ground (`GetNearPoint`'s `UpdateAllowedPositionZ`) and its shortcut its
       two points (`BuildShortcut`); every straight walk of wvs's (a chase's, a way home, a script's move, a fear's or
       a confusion's, a pet's, a waypoint the navmesh stops short of) is on the ground, out of the water.
     - **A player killed in the air comes down where its client's fall lands it** (`movement::dead_reach`), as far as
       its jump carries it on since its last move (its jump's speed over the ground, at most a run's, for the fall's
       time between), and lies there; a landing further on is refused, as any run of the dead. wvs took only a move
       straight down, so a client whose answer to its root, or its landing, came from where its jump had gone on
       since its last heartbeat (the bot's, 1.4 yd on) was refused, and its body stayed where that heartbeat had it,
       a yard and a half up, for every watcher. mangos-classic's `KillPlayer` leaves the body to its client ("don't
       create corpse at this moment, player might be falling") and takes its landing (`VerifyMovementInfo`: a rooted
       mover's packet with no run in it), the corpse made at the release where the body lies (`BuildPlayerRepop`,
       `CreateCorpse`). peon's bot now falls as a client does (peon `fix/av-floating`): its root's answer where it is
       in the air, its landing in its jump's time; before, its answer put it on the ground at once.
     - **The navmesh's steep polygons are a creature's to walk**, as they were: the record's "edge" crossings left
       are a Frostwolf Guardian's and an Alterac Yeti's walks over steep polygons (Ground and Steep both), which a
       creature's path takes (rustigator's `find_path`, every polygon at unit cost; mangos-classic's `createFilter`:
       `NAV_GROUND | NAV_GROUND_STEEP` for a creature that walks) and peon's check finds no way over, as it measures by
       a player's (steep left out). On the navmesh, not off its edge; left so.
     - **Cost**: grounding a path, 13.6 µs against 7.3 µs on master (`probe_path_on_the_ground`, 2,000 paths in
       Tanaris, the same machine and load; 5,660 points against 5,641), from the samples every yard.
     - **A path's end reaches its goal over the ground** (`nav::reaches`: within its yards over the ground and a
       step's climb of its height), for Rexxar's start (`champions::out_of_sight`, as his run in counts its
       arrival over the ground) and a guard's reachable point: with a path's end on the floor, Orgrimmar's square, a
       platform 1.45 yd over the floor where the navmesh's way to it ends 2.7 yd off, was 3.05 yd from its path's
       end, and Rexxar found no start (`each_citys_spots_are_on_the_navmesh`).
     - **The record, before and after** (peon's all-in at 40, seeds 1 and 2, two runs of five attempts each way, 20
       attempts in all; peon's fall with them): creatures' time over the ground past 0.3 yd outside the keep 1.07% to
       0.01% (moving 5.36% to 0.03%, under it 3.17% to 0.00%), in the keep's yard moving 0.94% to 0.02% (under 5.20%
       to 0.00%), in his room 0.91% to 0.18% (moving 0.92% to 0.44%); their height moving, p99 outside +0.92 to
       +0.12 yd (the most +1.54 to +0.70), in the yard +0.30 to +0.10, in his room +0.30 to
       +0.21; walks' spline points over 0.3 yd 191 of 10,702 to 0 of 12,686, chases' 10 of 5,001 to 0 of 6,428;
       bodies over the ground a second after their death 1 of 341 (a pet) to 0 of 320, the players killed in a jump
       2 and 11, each landed in its jump's time (one answered its root 1.4 yd on from its last heartbeat, which the
       old rule refused); crossings 42 to 29, of them the steep polygons' "edge" 25 to 28 (a wall's 1 to 0), the
       runs and snaps a watcher is shown across a step 14 to 1. Drek'Thar killed 18 of 20 both (9 and 9 before, 8
       and 10 after).

110. **A charge's spikes** (2026-10-25; `perf/charge-spikes`, your go: "remove the map update's spikes in a charge's
     first seconds", a charge at 1,600 with p99 under 50 ms and no update over about 100 ms, each client seeing what
     1.12 would show it; numbered after master's 109). docs/PERFORMANCE.md, "A charge of 1,600". Two of these are
     changes of timing, said so here; the rest change only which packets carry what.
     - **A crowd's creates in packets shared among it, no client held for them** (`visibility::Crowds`). mangos-classic
       builds each player's creates into its own `UpdateData`; wvs put each create into its observer's batch and held
       the client until it went, every broadcast meanwhile copied into its hold. Now a chunk's introductions of an
       object to those who see it as a stranger does go in packets planned as the values are (decision 102's
       planner) and broadcast at the chunk's end, before anything else is sent. The rest as before, each client held:
       one with a view of its own (a marker, a pet's master, a tap's outsider, a group shown quests), one with a
       forget of the object still to come this update, one held already (its batch goes before its hold), a player
       to one who has the player's corpse or is to forget it, a passenger, a corpse, a game object, a transport, a
       share one client alone gets. Each client gets the same creates, each once, before anything naming their objects; a
       client's creates may come in another order among themselves, and in a packet of their own rather than its
       batch. No key: no rule changes.
     - **A spell's area is seen as it is put down** (`spells::dynamic::spawn`), not at its event's end (or at the
       update's visibility, for one put down in an update): the timing of its create, now after the cast's go and
       before the rest of the event's packets, as mangos-classic's `Map::Add` shows a dynamic object to those in
       range at once. No one is held for it. A far sight's focus is as before.
     - **Visibility's budget holds** (`[visibility] refresh_budget_ms`, the key it was; its meaning written again): a
       chunk stops where the budget is spent, its rest first next update, one refresh at least an update; a chunk is 1
       to twice the last's (at least 4), where it was 16 at least and ran 10 to 60 ms over the budget in a charge.
       A change of timing: a charge's sides are in each other's sight about 0.4 s later at 1,600 (each player's
       sight whole at 9.2 s from the charge's start, where it was 8.8 s). The budget is yours to raise; I kept 10.
     - **Member stats spread over the second** (`[groups] member_stats_spread`, new, on): each grouped player's sent
       to the members out of their sight once a second, as before, each at a moment of the second of their own (its
       guid's) rather than all at one update (a battle of 1,600: 7 ms in it); as each player's own update in mangos
       sends theirs. Off is the old timing.
     - **Not changes of rule**: a whole refresh asks `death::can_see` only of pairs not both plain (the quick path's
       rule); its looks over a crowd across the visibility threads; a unit's values by view share the strangers'
       block; the planners' packets framed across the threads; an area's navmesh sight asked last of its filters;
       the areas' tick record pruned once a second (its gap capped at the 30 s kept, so what ticks is the same).
     - Tests: `tests_visibility::shared_creates_leave_each_client_as_one_by_one` (each client's frames replayed over a
       walk with every change of state, shared and one by one: the same views, update by update),
       `a_crowds_shared_create_comes_before_what_names_its_object`, `a_held_client_gets_a_crowds_create_in_its_batch`,
       `a_spell_area_is_seen_at_once_its_create_shared`,
       `a_chunk_stops_where_the_budget_runs_out`, `chunk_tests`, `social::spread_tests`,
       `tests_crowd_values::shared_values_are_the_blocks_each_client_had_one_by_one` (marks now), the client check
       `tests_client_emu::a_crowds_shared_creates_pass_the_client` and
       `a_crowd_sees_one_rise_from_her_corpse_and_passes_the_client`; benches `battle` (its new knobs) and `visibility`
       (`whole`; it runs again: it lacked `harness = false`).

## Left for later, and why
