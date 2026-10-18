# Decisions and questions, in full

The record behind [SITREP.md](SITREP.md): every question put to you, as it was written, and every decision made along
the way, with its reasons. SITREP.md keeps the short index; this keeps the detail.

## The questions, as asked

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

48. **Tracy, reworked** (your ask; `docs/PROFILING.md`). Set `WOW_TRACY=1` and connect Tracy v0.13.1.
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

## Left for later, and why
