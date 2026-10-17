# Where wow_vanilla_server differs from Thistle Tea

wow_vanilla_server (wvs) set out to do what [Thistle Tea](https://github.com/pikdum/thistle_tea) does, on the
same world, for a fraction of the CPU ([ALIGNMENT_PLAN.md](ALIGNMENT_PLAN.md)). This is where it stands on
2026-10-17, against Thistle Tea at `0426f0d6` (2026-10-11): what it does differently on purpose, and what it
doesn't do yet. Thistle Tea's web server is out of scope throughout.

It is not one for one. Three independent audits and a module-by-module sweep of every area of Thistle Tea's
`lib/game` found what is listed here; what isn't listed was compared and matches. Each sweep's own notes go into
more detail: [COMBAT5_SWEEP.md](COMBAT5_SWEEP.md) (combat, items, loot, professions, vendors, death) and
[AI5_GAPS.md](AI5_GAPS.md) (creature AI, scripts, instances).

## Where the two stand

| | wow_vanilla_server | Thistle Tea |
|---|---|---|
| Code | 119k lines of Rust, 499 tests (29k lines), most through the real protocol on the real data | 180k lines of Elixir, 8,800 tests |
| Client opcodes | every one Thistle Tea dispatches | 269, plus 22 movement opcodes |
| World | vmangos, from Thistle Tea's export, with Thistle Tea's creature scripts imported as data | vmangos |
| Systems | all of Thistle Tea's: spells and auras to 60, pets, procs, professions, group loot, quests and events, the three battlegrounds, honor, outdoor PvP, instances and their scripts, raid bosses' mechanics, transports, social | the same |
| Persistence | SQLite, versioned, saved at logout, on a timer and on SIGTERM | none: a restart loses every character |
| Security | SRP accounts, GM levels, bans, movement checks, rate limits | every session is a GM; positions trusted |
| Under load (peon, 100 bots; wvs at 3425257) | 0.16 cores idle, 0.43 in a crowd, 0.22 in the arena pit, 0.15 for the Gurubashi last man standing; every run valid | 6 cores in a crowd and in the pit |

## Different on purpose

- **The design.** An actor per map and per instance copy with a 20 Hz update, where Thistle Tea runs a process per
  player and per creature. The same behaviour, measured at a fraction of the CPU.
- **What wvs adds.** Persistence, accounts and GM levels, movement and rate checks (above); Gurubashi Arena's last
  man standing.
- **The battleground scoreboard** shows each player's real honor rank; Thistle Tea always shows 4.
- **Battleground invitations** last two minutes and show the time left; Thistle Tea sends a flat 80 seconds.

## Not yet matching

### Spells and auras

- A dispel skips passive auras, as vmangos does; Thistle Tea dispels them too.
- Equipped items' spells that name a form (feral attack power on weapons) aren't held to that form.
- The flat healing-taken bonus doesn't take the spell's coefficient, and a heal over time takes healing-taken
  modifiers when it lands rather than at each tick.
- A killing spell's `no_durability_loss` attribute doesn't reach the death's wear: the kill doesn't know its spell.
- Net-o-Matic's instant backfire aura (13139) isn't ported; 13119's trigger catches the thrower to the same end.
- Combo points kept by aura 148 (`retain_combo_points`) aren't taken back when it ends. No spell a player learns
  carries it.

### Combat, creatures and loot

- Creatures send `SMSG_SPLINE_SET_RUN_SPEED` when their speed changes; Thistle Tea does only for possessed pets.
- Gathering, crafting, lockpicking, disenchanting, fishing, the corpse reclaim, self-resurrection and resurrection
  sickness were compared and found matching, but not line by line.

### Creature AI, scripts and instances

[AI5_GAPS.md](AI5_GAPS.md) lists each by its number; in short:

- **Movement:**
  - wander points aren't clamped to the spawn's radius;
  - pools don't take limits over one, re-roll, or spawn for events;
  - creatures don't charge, and fear, confusion and fleeing don't stop, anchor or check their paths as Thistle
    Tea's do;
  - distract isn't handled;
  - formations and scripted following are close but not exact;
  - flyers' spline packets differ in format.
- **Creature combat:**
  - harmful casts and stealth;
  - melee range and the chase;
  - emote and stand state in a fight;
  - healing on leashing, and the leash's extension;
  - facing packets, and `SMSG_ATTACKSTART`/`STOP`'s timing;
  - creatures spreading round their target;
  - victim selection's finer rules, and casters' line of sight;
  - creatures charming players.
- **Script commands:**
  - respawning a despawned object, and despawning one (9, 81);
  - activating traps and goobers (13);
  - the swap and self-target data flags;
  - turning to face (35);
  - map events' ids and their clearing (66, 69);
  - respawning a creature (71), and a despawn's respawn delay (18);
  - custom animations (89);
  - object gossip scripts;
  - a faction's restore flags (22);
  - the zone pulse (49);
  - the chat-type override for boss emotes and zone yells;
  - `update_entry` (27), which is partial;
  - regeneration;
  - unequipping by 0 (19);
  - a few smaller ones.
- **Encounters:**
  - summons' despawn types;
  - the unkillable flag;
  - emote states;
  - The Nightmare's druids may attack pets;
  - "nearest hostile player" with no radius reaching 100 yards, not 30;
  - Mizzle's walk home never arriving.
- **Spell lists:**
  - the force-cast flag;
  - the order of `creature_spells_scripts`;
  - a timed-out summon despawns where Thistle Tea's dies;
  - creature-owned guardians don't follow their owner or go with it, and keep their loot.
- **Instances:**
  - Molten Core's runes: the real-data check that Aqual Quintessence douses one fails after the merge and is
    ignored (`tests_ai5_instances::the_real_instances`);
  - raid binds are given on any killer, where Thistle Tea wants a player;
  - chests raised again after a reload get fresh loot;
  - Razorfen Downs' gong is forgotten across a reload.

### Quests and events

Every quest that waits on an event (SpecialFlags 0x2, 212 of them) has a source that something triggers, as in
Thistle Tea, with these exceptions:
- **The Nightmare Manifests (8736)** can't be finished. Remulos's escort to the shrine isn't imported (the generator
  drops escorts Thistle Tea builds by function call), and porting it needs what wvs's escort engine lacks: unit flags
  and a script phase set on the way, a signal to Eranikus, facing, batch summons, and the map event its credit goes
  through.
- **The Glowing Shard (3366), Jail Break! (4322), Master Ryson's All Seeing Eye (6847, 6848) and A Pawn on the
  Eternal Board (8519)** can't be finished, on Thistle Tea either: nothing in the data or in Thistle Tea raises their
  event (vmangos scripts them in C++).
- Imported from Thistle Tea but not walked through end to end: Freedom for All Creatures (2969), the Gizelton
  Caravan (5821, 5943), The Great Masquerade (6403), The Balance of Light and Shadow (7622). About 176 world-database
  event quests rest on the generic area-trigger, script and escort paths, sampled rather than walked one by one.

### The world and the social systems

- Those watching a player may be told of a speed change twice: at the change and again at the client's
  acknowledgement. Thistle Tea tells them once. The speeds set by a ghost's release, a revive and `.speed` reach
  them only at the acknowledgement.
- Going into or out of water ends what it breaks only by the client's swimming flag, not by the terrain's liquid.
- An innkeeper's Hearthstone is given directly rather than by Bind's spell: in wvs a creature's create-item effect
  gives the item to the caster.

### PvP

- Spell 30176, which Thistle Tea has refresh a silithyst carrier's PvP flag, isn't handled; nothing casts it.

## Tests and tooling

- **wow_messages.** wvs pins `gtker/wow_messages` at `e8dc643`. Two layout fixes are committed in the local clone
  (`/local/home/wallenx/wow_messages`, branch `vanilla-1.12-layouts`) and not yet anywhere wvs can pin:
  `SMSG_BATTLEFIELD_STATUS`'s status is a u32 in 1.12 (`c9a8d912e`); and strings over 256 bytes, with
  `SMSG_QUESTGIVER_QUEST_DETAILS`'s reward items carrying their display id (`d5055998c`). The server builds both
  messages by hand and is unaffected. Until the clone is pushed to a fork and wvs repinned, the test client logs
  the battlefield status as unparsed and reads quest details itself, and other messages with strings over 256
  bytes (long reward texts, gossip) fail to decode in tests. Repinning also needs two small wvs changes, for the
  clone's other commits: `quests.rs`'s `choice_item_rewards` and `item_rewards`, and `SMSG_CAST_RESULT`'s new
  `reason` field in `tests.rs`.
- wow_messages types the client's fall time as a float; it is a u32 of milliseconds, so wvs reads the raw bits.
- **Load-sensitive tests.** Each real-data test loads the whole world. On a machine running several suites at once
  (load average 40 to 50 on 16 cores) a few time out; `tests_spells3::the_combat_log_packets` most often. They pass
  run alone. The harness waits 20 seconds for what a test expects.
- Not covered by a test of their own: the play-object-sound script command, the meeting stone's in-progress
  message, and Ritual of Summoning's dungeon and battleground refusals.
