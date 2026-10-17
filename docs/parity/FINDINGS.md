# What mapping the matrix found

Each area's mapping ([README.md](README.md)) ends with its biggest gaps and the wvs bugs noticed on the way. They seed
Phase 3. When something here is fixed, its line gets the commit.

## Bugs found while mapping

### Read and confirmed in the code

- **A dead unit can be killed again.** `combat::deal_damage_school` and the spell landing path (`spells/mod.rs`
  `land`, `land_arrivals`) never check that the victim is alive. A missile landing on a corpse runs `kill` again:
  the reward, the quest credit, and the battleground death and honor all count twice.
- **Container loot is duplicated.** `items_misc::open` always rolls fresh loot, while `release` keeps a part-looted
  container to be opened again.
- **Druid forms get no mana back.** `stats::regen_player` regenerates only the power the player is shown.
  mangos-classic's `Player::RegenerateAll` regenerates mana in every form.
- **Any item GUID starts any quest.** `quests::accept` skips the giver and distance checks for any GUID whose high
  part is Item.
- **Ritual of Doom (18540) can never be cast again.** Its cooldown waits for an aura to go (`spells/cooldowns.rs`),
  and the spell has none.
- **A pet set to autocast a positive spell casts nothing.** `pets/ai.rs` `pick_spell` returns `None` for every
  positive spell: Dash, Dive, Fire Shield, Paranoia.

### Reported, not yet confirmed

**Object gossip:**
- `quests::object_hello` and `objects::show_readable` pass NPC flags 0, which hides every option.
- `npc::select` takes only creatures.

**Loot and crafting:**
- Disarm Trap opens a loot window (`professions.rs`).
- `CMSG_LOOT` skips the lock and in-use checks on an object whose loot is already rolled.
- Crafting into full bags takes the reagents and still gives the skill gain.

**Casting:**
- A ranged ability with no ammo still fires.
- Power is taken at completion with `saturating_sub`, so a caster drained during the cast pays nothing.
- Combat doesn't interrupt a not-in-combat cast (attribute 0x10000000).
- A teleport doesn't interrupt a cast.

**PvP and reputation:**
- Ghosts explore and get the XP.
- `SMSG_SET_FORCED_REACTIONS` is sent only at login.
- `MSG_INSPECT_HONOR_STATS` is answered for any target at any distance.
- Healing threat isn't split among the creatures in the fight.

**Social:**
- The ready check isn't sent to the player who started it.
- A charter can be bought with no petitioner near.
- A party member's accept of a shared quest runs its start script again, and a client can fake a share accept.

**Movement and control:**
- `undo_logout` always unroots and unstuns, even under a root or stun aura.
- A crafted `CMSG_UNLEARN_SKILL` can drop weapon or defense skills.
- A freed charmed creature turns on its controller with 0 threat, and tags itself for them.
- Script command 88 (stay) stops the attack. Thistle Tea keeps the fight.

**Pets and creatures:**
- A dead player's fighting guardian is removed at once. Thistle Tea keeps it until its fight ends.
- Extra attacks owed by procs wait out a cast. Thistle Tea swings them and keeps the cast going.
- Pets acting on their own don't spare civilians, which risks dishonorable kills.
- A game event starting or ending replaces a creature mid-fight.

**Tests that assert less than they claim:**
- `tests_scripts::a_pool_spawns_its_limit_and_a_condition_gates_a_step`
- `tests_ai5::reaction_modes`

## The biggest gaps, by feature

### Creature AI (`core/ai`, `core/creature`: 160 of 1,071 covered)

- **Facing, the chase and spreading** (C9, C12, C14). wvs sends no facing packets at melee contact, before a timed
  cast or at home. It has no spreading, and doesn't use Thistle Tea's chase or escape distances.
- **Evade and home** (C10, C11, C22, M9, D25):
  - wvs heals on arrival rather than on leashing, and heals creatures that don't regenerate;
  - it strips positive auras, where Thistle Tea keeps them;
  - stand state, emote state and facing aren't restored at home;
  - the reached-home event never fires.
- **Keeping distance** (movement 19). There are no tests. It doesn't check line of sight or the ground, doesn't hold
  casts while backing off, isn't cancelled by root, stun, fear or death, and ignores the target's size.
- **Script commands** (S2 to S23). About 90 of `script_test`'s 122 rows are gaps, and `script_failure` and
  `script_run` (22 rows) are untested.
- **Summon despawn types** (D-R1), behind Ragnaros's Sons, Wyrmthalak's and Nerub'enkan's adds, and others.
- **Static flags:**
  - unkillable (static_flags1 0x8);
  - a sessile creature that still takes fear and roots;
  - no defense, and no spell defense;
  - player- and NPC-immune;
  - combat dummies.
- **Pet AI** (1 of 46 covered):
  - only harmful spells are autocast;
  - there is no target priority;
  - Stay, Follow and Attack are untested;
  - guardians', totems' and companions' lifetimes.
- **Creature linking and group flags** (M11, M12). They are untested; flags 0x10 to 0x80 are absent, and no new
  leader is picked.
- **EventAI events with code but no test:**
  - timers out of combat;
  - aura, target-aura and target-mana events;
  - a rooted victim;
  - line of sight out of combat;
  - a spell hit;
  - receiving an emote;
  - a script event;
  - reaching home;
  - evading;
  - conditions;
  - pausing while stunned;
  - a group member dying.
- **Linked feign death** (Core Hounds) and **critters** have code but no tests.
- **About 265 imported encounter and NPC scripts** have no scenario test. That spans Molten Core, Onyxia, BRD and
  BRS, Stratholme, the Nightmare dragons, and world events.
- **Missing altogether:** the Lunar Festival's firework launchers and Silithus's wind stones.
- **Event creatures** are respawned in their event looks, which loses a fight. There are no per-event spell lists.
- **Charmed creatures** get the whole spell list; Thistle Tea rolls charm spell slots.
- **Wands** are untested. **Aggro's finer rules** have code but no tests.

### World entities (`world/entity`: 146 of 1,251 covered)

- **Spells:**
  - charge is a teleport;
  - nothing is checked again when the cast completes;
  - a travelling spell's hit is rolled on landing, not at launch;
  - spells don't make PvP contact;
  - on-event cooldowns are released only when an aura goes.
- **Objects:**
  - objects cast through their target, not as themselves;
  - disarming a trap;
  - wild summons (effect 41) are made guardians;
  - chairs have no seats or height, and use a 10-yard range where Thistle Tea uses 3.
- **Movement:**
  - acknowledgement counters are always 0 and every ack is taken;
  - possessed units' controllers aren't told of roots, speeds or knockbacks;
  - a creature's teleport doesn't use `MSG_MOVE_TELEPORT`.
- **Quests:**
  - item-started quests: the item isn't checked, taken or given back;
  - refusal reasons;
  - `SMSG_QUESTUPDATE_ADD_ITEM`'s count;
  - talk-to credit;
  - the sharing rules.
- **Pets and creatures:**
  - creature-owned pets;
  - summon despawn types;
  - Stay as melee only;
  - waiting out crowd control.
- **Loot, items and vendors:**
  - group loot's sole member and full bags;
  - the bank never checks for a banker;
  - leftover container loot;
  - a bag-room check before create-item;
  - vendors: buy-in-slot, selling equipped items, range failures, `CMSG_SET_AMMO`.
- **Death, travel and PvP:**
  - resurrection across maps;
  - scripted and spell taxis' fares and checks;
  - battleground credit for dead teammates;
  - honor shared by the roster;
  - soul shards to the tap group.
- **Player state:**
  - talent reset's checks;
  - a stanceless warrior isn't put in Battle Stance at login;
  - the group leader flag;
  - `.instance data` and `.debug transport`;
  - reputation's spillover, aura 190 and `SMSG_SET_FACTION_VISIBLE`.
