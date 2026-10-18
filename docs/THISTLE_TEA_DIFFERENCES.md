# Where wow_vanilla_server differs from Thistle Tea

wow_vanilla_server (wvs) set out to do what [Thistle Tea](https://github.com/pikdum/thistle_tea) does, on the
same world, for a fraction of the CPU ([ALIGNMENT_PLAN.md](ALIGNMENT_PLAN.md)). This is where it stands on
2026-10-17, against Thistle Tea at `0426f0d6` (2026-10-11): what it does differently on purpose, and what it
doesn't do yet. Thistle Tea's web server is out of scope throughout.

It is not one for one. Three independent audits and a module-by-module sweep of every area of Thistle Tea's
`lib/game` found what is listed here; what isn't listed was compared and matches. Each sweep's own notes go into
more detail: [COMBAT5_SWEEP.md](COMBAT5_SWEEP.md) (combat, items, loot, professions, vendors, death) and
[AI5_GAPS.md](AI5_GAPS.md) (creature AI, scripts, instances). The plan to close what is left is
[FULL_PARITY_PLAN.md](FULL_PARITY_PLAN.md).

## Where the two stand

| | wow_vanilla_server | Thistle Tea |
|---|---|---|
| Code | 119k lines of Rust, 499 tests (29k lines), most through the real protocol on the real data | 180k lines of Elixir, 8,800 tests |
| Client opcodes | every one Thistle Tea dispatches | 269, plus 22 movement opcodes |
| World | vmangos, from Thistle Tea's export, with Thistle Tea's creature scripts imported as data | vmangos |
| Systems | all of Thistle Tea's: spells and auras to 60, pets, procs, professions, group loot, quests and events, the three battlegrounds, honor, outdoor PvP, instances and their scripts, raid bosses' mechanics, transports, social | the same |
| Persistence | SQLite, versioned, saved at logout, on a timer and on SIGTERM | none: a restart loses every character |
| Security | SRP accounts, GM levels, bans, movement checks, rate limits | every session is a GM; positions trusted |
| Reliability | a panic costs its update step, creature turn or packet; a creature restarts from its spawn, and is quarantined if it keeps panicking; a map that keeps panicking restarts from the data; background loops back off and retry; a watchdog logs hung maps ([RELIABILITY.md](RELIABILITY.md)) | OTP supervisors: a process per player and creature, restarted when it crashes |
| Under load (peon, 100 bots; wvs at 3425257) | 0.16 cores idle, 0.43 in a crowd, 0.22 in the arena pit, 0.15 for the Gurubashi last man standing; every run valid | 6 cores in a crowd and in the pit |

## Different on purpose

- **The design.** An actor per map and per instance copy with a 20 Hz update, where Thistle Tea runs a process per
  player and per creature. The same behaviour, measured at a fraction of the CPU. A panic costs the step or creature
  turn it happened in, and a map that keeps panicking is restarted, as a supervisor restarts a child
  ([RELIABILITY.md](RELIABILITY.md)).
- **What wvs adds.** Persistence, accounts and GM levels, movement and rate checks (above); Gurubashi Arena's last
  man standing.
- **The battleground scoreboard** shows each player's real honor rank; Thistle Tea always shows 4.
- **Creatures' speed changes** go to those who see them as `SMSG_SPLINE_SET_RUN_SPEED`, as mangos-classic sends
  them; Thistle Tea sends nothing for a creature no player moves. Both send its run again at the new speed, and tell
  a possessed one's controller with `SMSG_FORCE_RUN_SPEED_CHANGE`.
- **A creature running in to strike** swings once it has got there, half a yard inside its reach, not as it crosses
  into reach: the client shows a creature on the move a little behind where it is. Thistle Tea swings at once.
- **Honor inspection** answers within 28 yards, as mangos and the client have it; Thistle Tea's reach is 10.
- **Every teleport stops a player's cast.** Thistle Tea spares its "combat teleports", which wvs doesn't tell apart.
- **A guild charter** is sold by any petitioner; Thistle Tea also wants the tabard designer's flag.
- **Blink's forward form** stops at the farthest point in sight; Thistle Tea walks the navmesh.
- **The tapped flag** follows the tapper's current group; Thistle Tea keeps the group as it was at the tap.
- **Update packets are compressed above 1,024 bytes** (Thistle Tea above 128, mangos above 100). The client reads
  either form at any size, so this only trades bandwidth against CPU.
- **A pet's Stay from its master's bar** stops its attack, as mangos-classic's does; Thistle Tea and vmangos keep the
  fight. Script command 88 keeps it, as they do.
- **An event's looks wait for a creature's fight to end,** and it is respawned in them; Thistle Tea changes the living
  creature in place.
- **Gear a character can't use comes off at login,** to the bags or else by mail, as mangos's `_LoadInventory` has
  it. Thistle Tea checks only at a talent reset.
- **The weather's season** follows the server's local date, as mangos does; Thistle Tea uses UTC.
- **Snares stack,** as mangos-classic has them: each stays on as long as it lasts, and the strongest slows. Thistle
  Tea and vmangos keep one, passing over a weaker or shorter one. (Attack-speed slows keep Thistle Tea's rule.)
- **Water's interrupts** also go by the terrain's liquid (deep enough to swim, as mangos-classic has it), not only by
  the client's swimming flag, which is all Thistle Tea goes by.
- **vmangos's spell corrections** (`spell_mod`, `spell_effect_mod`) win over the raw DBC Thistle Tea reads:
  Concussive Shot and Piercing Howl are dazes, not snares; Blessed Sunfruit Juice and Alterac Spring Water aren't
  Well Fed; Blood Siphon and Blood Funnel are a heal and a hit, not funnels.
- **New weapon skills** start at their cap; Thistle Tea and mangos start them at 1.
- **Lower ranks of rage and energy spells** stay in the spellbook; mangos takes them out.
- **Battleground invitations** last two minutes and show the time left; Thistle Tea sends a flat 80 seconds.
- **Pets, as mangos has them:** a pet on Stay answers an attacker from afar (`PetAI::AttackedBy`), though of its own
  accord it takes on only what is in its reach; a dead master's pet still casts at their word; and a charmed summon
  still runs out of time. Thistle Tea refuses all three.
- **A charm or Mind Control ending** leaves the freed creature's cast or channel running, as mangos's `Uncharm` does;
  Thistle Tea interrupts it and clears its global cooldown.
- **Kept as mangos-classic has them** (round 3):
  - Missed periodic ticks after a stalled update run one an update afterwards; Thistle Tea runs them at once.
  - An abandoned quest doesn't give back the item that started it.
  - A creature's Bear Form is model 902, whatever its own.
  - A charm's spells are rolled at each charm; Thistle Tea rolls them once a spawn.
  - Ghost Wolf and disguises set no scale or reach of their own.
  - Full bags at a vendor get mangos's inventory failure, not `BUY_FAILED cant_carry_more`.
- **A cast is checked again as it ends** (range with mangos's tolerance, facing, life, still a foe), and one that
  fails there costs nothing.
- **A refusal's error code** is mangos's where Thistle Tea sends another: a two-hander with nowhere for the off hand
  gets `InventoryFull`, not `cant_equip_with_twohanded`; a split into the wrong kind of bag, `ItemDoesntGoIntoBag`.
- **Kept as mangos-classic has them** (round 5):
  - A friend's school protection on a flag carrier lands, and drops the flag; Thistle Tea refuses it.
  - A charmed creature released stops only its auto-repeat and melee spells, not a prepared cast or a channel.
  - A spell modifier gives an aura whole points.
  - A victim of a persistent area aura's pure periodic damage gets a debuff slot.

## Not yet matching

### Spells and auras

- A dispel by mechanic (effect 108) skips passive auras and matches effects' mechanics. Thistle Tea matches only the
  spell's own mechanic, passive or not.
- Enchantments' spells aren't held to the outdoors or to forms; items' and sets' are, as Thistle Tea's
  `EquipmentSpells.eligible?` holds them.
- Extra attacks leave the swing timer alone; Thistle Tea and vmangos restart the main hand's after them.
- A dead master's guardian isn't removed for distance (Thistle Tea: beyond 120 yards).
- A channel's cost per second (Health Funnel's health, others' mana) isn't charged; Thistle Tea charges it each tick.
- The melee and ranged damage-taken auras' flat amount isn't scaled by an ability's coefficient, as Thistle Tea's
  `AttackDamageTaken.spell_amount` scales it.
- A damage over time's threat modifiers are read at each tick, not kept from the cast, and its `spell_threat`
  multiplier isn't applied.
- **Death's wear:**
  - Spirit of Redemption wears gear down when the spirit ends; Thistle Tea does at the fatal blow, and only if a
    player didn't strike it.
  - A death by one's own hand wears gear down; in Thistle Tea it doesn't.
  - The killing blow can cost the one-in-200 wear of any blow; in Thistle Tea it doesn't.

### Combat, creatures and loot

- **Gathering, crafting, lockpicking, disenchanting, fishing and resurrection,** compared line by line (Phase 2i).
  What is left:
  - **Failed openings.** A failed gathering, lockpicking or opening is told after its spell went off, with a cast
    result only. Thistle Tea holds back `SMSG_SPELL_GO` and sends `SMSG_SPELL_FAILURE` and
    `SMSG_SPELL_FAILED_OTHER`.
  - **Lock spells:**
    - their range is checked as the cast begins, without line of sight;
    - a key or charge is spent when the attempt fails;
    - a picked door doesn't close itself or spring its trap.
  - **Locked objects by hand.** A locked door or button opens by hand without its key, and a locked chest never does.
    Thistle Tea opens each with the key carried, and refuses without it.
  - **Crafts.** A craft's focus, tools and reagents aren't checked again when it completes. A craft that doesn't fit
    makes nothing and says nothing.
  - **Disenchanting.** What's left on a disenchanted item is lost only when its window is closed, and a second
    disenchant while it is open is refused; Thistle Tea stores it or shows it again.
  - **Unlearning a profession** doesn't check that it may be unlearned, nor drop the quests that need it.
  - **The spirit healer** confirms at a ghost's hello, with no gossip menu. Ghosts don't see Thistle Tea's ghost-only
    creatures, and the living see spirit healers.
  - **Corpses.** A corpse in a dungeon copy is checked by map, not copy. Death doesn't root, nor set the stand flag
    at release.
- **Where Thistle Tea is wrong for 1.12, wvs keeps mangos's behaviour:**
  - an off-hand weapon (inventory type 22) needs Dual Wield too;
  - short of an item's skill rank is `CANT_EQUIP_SKILL`;
  - a missing tool is 0x78 (`TOTEMS`; Thistle Tea sends 0x26);
  - fishing skill counts the poles' and hat's skill auras;
  - quest items in node loot don't keep a node from despawning.

  Bones, the spirit healer moving the player to a graveyard, a reclaim range that counts both bodies, Twisting
  Nether and Poisons skill-ups are in mangos, and in neither Thistle Tea nor wvs.

### Creature AI, scripts and instances

[AI5_GAPS.md](AI5_GAPS.md) lists each by its number; in short:

- **Movement:**
  - Distract stops and turns only creatures; Thistle Tea turns players too.
  - A monster move's intermediate point within about 0.7 yards of the destination is left out, as mangos-classic
    leaves it; Thistle Tea nudges its height instead.
- **Scripts:**
  - A summon of despawn type 0 with a time is vmangos's timed-or-dead (it waits out a fight); Thistle Tea despawns
    it at its time.
  - Script teleport (6) moves a player source too; Thistle Tea's moves only creatures.
  - Respawning a creature (71) with target type 10 still finds dead creatures; Thistle Tea's search finds only the
    living, so it could never respawn a dead one.
  - The Nightmare's druids may attack pets.
  - Silithus's wind stones aren't ported: `tt_scripts.json.gz` has their entries with empty steps.
  - Huldar and Miran's ambush (area trigger 171) can't spring: its first step explores quest 273 before the second
    asks for it incomplete.
- **Pets:**
  - A pet takes crowd-controlled targets, and doesn't hold off while on Stay.
  - Auto Shot isn't cancelled by pacify or range, and its dead zone is measured centre to centre.
- **Swings:** errors repeat each second, and the off hand rolls the main hand's weapon skill.

### Quests and events

Every quest that waits on an event (SpecialFlags 0x2, 212 of them) has a source that something triggers, as in
Thistle Tea, with these exceptions:
- **The Glowing Shard (3366), Jail Break! (4322), Master Ryson's All Seeing Eye (6847, 6848) and A Pawn on the
  Eternal Board (8519)** can't be finished, on Thistle Tea either: nothing in the data or in Thistle Tea raises their
  event (vmangos scripts them in C++).
- **What is walked.** Every event quest's source is in reach on the real world
  (`tests_parity_world::every_real_event_quest_has_its_source`).
  - **Walked end to end:**
    - The Nightmare Manifests, Freedom for All Creatures, the Gizelton Caravan, The Great Masquerade and The Balance
      of Light and Shadow;
    - every escort and follower event quest, except Celebras's (7046, which needs Celebras the Cursed dead) and
      Ancient Spirit (4261, whose giver has no spawn);
    - 84 of the 103 that the database raises by an area trigger, a gossip option or a start script.
  - **Not walked, listed in `tests_parity_world::UNWALKED`** (the other 19):
    - object gossip, and menus no spawned creature has;
    - Winter Veil's Metzen;
    - options that want an aura or item the walk doesn't give;
    - 4001, 4866, 6001 and 6002, still to look at.
  - **Checked only for a source in reach:** the roughly 80 raised by a spell, a generic script, an EventAI or a
    waypoint script.
- **Escorts.** The escort catalog leaves out its summons' own steps and their despawn kinds. 2078's follower starts on
  accept, not from its gossip option.

### Players and the world

- A teleport within the map doesn't end the player's fight; Thistle Tea clears it and holds the pet until the client
  acknowledges.
- A transport's deck offset is taken as the client sends it; Thistle Tea refuses a boarding out of range.
- A second login while the character is in the world kicks the first session; Thistle Tea refuses it with 0x41.
- Totems and dynamic objects leave without `SMSG_GAMEOBJECT_DESPAWN_ANIM`.

## Tests and tooling

- **wow_messages** comes from 12932's fork (`12932/wow_messages`, branch `vanilla-1.12-layouts`), as it does for
  rustigator and peon: gtker's `e8dc643` with 1.12's own layouts for the game object query, who, loot, the cast
  result, quest details (reward items with their display id, strings past 256 bytes) and the battlefield status.
- **Update blocks go at the end of an update,** after the packets sent during it. A packet naming an object made in
  that update can reach the client before its create.
- A flyer's `SMSG_MONSTER_MOVE` is laid out by hand, in whole points as the 1.12 client reads them; wow_messages
  (and so peon and the harness) can't read it.
- wow_messages types the client's fall time as a float; it is a u32 of milliseconds, so wvs reads the raw bits.
- **Load-sensitive tests.** Each real-data test loads the whole world. On a machine running several suites at once
  (load average 40 to 50 on 16 cores) a few can time out; they pass run alone. The harness waits 20 seconds for
  what a test expects.
- Not covered by a test of their own: Ritual of Summoning's dungeon and battleground refusals.
