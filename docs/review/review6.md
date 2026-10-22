# Review 6: the bug hunt and the second mangos survey (2026-10-22)

Two passes over master after round 12:
- a **bug hunt**: seven areas of wvs read for bugs, exploits, races, leaks and hot-path costs, each finding checked
  against its callers and against mangos-classic;
- a **survey**: what is still missing against mangos-classic, two days after the first one, whose top 10 round 12
  fixed.

Every trivial or small finding was then fixed, with a test that fails without the fix, one commit a finding. The
architectural ones are deferred, and so are the ones that are a call for the user, below and in
[SITREP.md](../SITREP.md).

| Area | Findings | Fixed | Deferred | Not a bug | The hunt | What was done |
|---|---|---|---|---|---|---|
| Network, sessions, the store | 14 | 10 | 4 | 0 | [hunt](review6/hunt/net_store.md) | [fixes](review6/fixes/net_store.md) |
| The map's core | 13 | 10 (2 in part) | 4 | 0 | [hunt](review6/hunt/map_core.md) | [fixes](review6/fixes/map_core.md) |
| Spells and auras (and 2 of the survey's) | 17 | 19 parts | 7 parts | 5 | [hunt](review6/hunt/spells.md) | [fixes](review6/fixes/spells.md) |
| Combat, creatures, pets | 17 | 14 | 2 | 1 | [hunt](review6/hunt/combat_ai.md) | [fixes](review6/fixes/combat_ai.md) |
| Items, quests, loot | 15 | 13 (2 in part) | 2 | 0 | [hunt](review6/hunt/items_quests.md) | [fixes](review6/fixes/items_quests.md) |
| Scripts, instances, battlegrounds (and 2 of the survey's) | 16 | 11 | 4 | 1 | [hunt](review6/hunt/scripts_pvp.md) | [fixes](review6/fixes/scripts_pvp.md) |
| Social, mail, auctions | 20 | 15 (1 in part) | 5 | 0 | [hunt](review6/hunt/social.md) | [fixes](review6/fixes/social.md) |

peon, the load tester, had its own hunt (25 findings, 20 fixed), in peon's repository.

## The most important fixes
- **Exploits and dupes closed:**
  - a Mind Controlled creature could be made to cast any spell (Death Touch);
  - a possessed unit's moves weren't checked: a victim dragged anywhere, rooted or not;
  - trading with oneself gave a sold item back free;
  - the client chose its own zone (graveyards, city rest, channels);
  - banners and silithyst were taken with no Opening cast;
  - silithyst could be turned in twice through a logout;
  - a sold lockbox could still be opened, a sold reagent spent;
  - any mail's text could be read by its id.
- **Soft locks:**
  - Stratholme's Baron, locked for good if Ramstein went before he died;
  - Nefarius left hostile after a wipe;
  - Vaelastrasz never started again after a saved failure;
  - Razorgore's gate shut after a wipe before the fight;
  - escorts failing on their starter alone;
  - a battleground's end that didn't stop its fighting.
- **State lost or damaged:**
  - a rename racing another could delete the name's holder;
  - a failed account read reset an account's password (under `auto`) and let a ban through;
  - the memory guard could hold every client back for good;
  - the mail sweep could delete a mail returned meanwhile, item and all;
  - an expired COD payment went back to the buyer, who kept the item;
  - the battleground return point was lost at a relog.
- **1.12's rules:**
  - auras saved as mangos saves them;
  - a cast checked again as it ends (line of sight, silence);
  - persistent areas test line of sight each tick;
  - skinning only once the quest and free-for-all drops are taken;
  - a tap kept through the tapper's death;
  - a moderator logs out at once (mangos's `InstantLogout`).
- **Other players see** a player's emote state (dancing) and aura state (Execute's light).

Found alongside, and fixed:
- `MSG_MOVE_TELEPORT_ACK` is read with the eight-byte guid a 1.12 client sends. wow_messages read it packed, so a real
  client's answer to a teleport within its map (`.go`, Blink) was taken for another's, and its moves were dropped
  after.
- Zul'Farrak's pyramid waves face the altar, not north.

## Deferred, as architectural or not worth the risk
- every frame capped at the client's 32,766 bytes (mangos caps only updates and mail: a client check of a larger
  frame first);
- saves kept while the store can't write, bounded by bytes (it would weaken decision 33's all-or-none);
- the transports' clock wrapping after 49.7 days (as mangos's; the fix needs per-transport values sent);
- the rest of a knockback's flight (its ballistic end);
- a per-player dirty flag for item values;
- traps looked at only in a player's view;
- the travelling spell's template held by reference;
- an owner index for party auras;
- one cast check for creature spell lists (it changes when a cast is refused);
- the escort act loop's index;
- script runs whose source is gone;
- raid roster updates' registry reads.

## Calls for the user
Listed in [SITREP.md](../SITREP.md)'s "Since round 11" with the options and a recommendation for each.

## The survey: what is missing against mangos-classic
The protocol is essentially complete. Every opcode mangos handles in these areas has a handler, except
`CMSG_BUG`; every spell effect and aura type a 1.12 spell uses has one. What's left is content and a few systems,
in [survey/](review6/survey/):
- **Raid bosses with no AI:** 47 of 69. That is all of Naxxramas (20) and the Temple of Ahn'Qiraj (13), 9 of
  Zul'Gurub's 10 and 5 of the Ruins of Ahn'Qiraj's 6.
- **Dungeon bosses with no AI:** 14 of 213: Maraudon's three, two of Dire Maul's, Scholomance's Vectus.
- **Quest events mangos writes in code:**
  - the Dreadsteed of Xoroth;
  - Jail Break! (the Alliance's Onyxia attunement);
  - The Challenge (the T0.5 chain);
  - the Sunken Temple's Avatar of Hakkar.
- **No player limit or login queue.**
- **Shutdown and AFK:** no shutdown countdown, no 15-minute AFK logout, no raid reset warning.
- **Resistances unlike mangos's:** the low-level floor, whole resists of dots and of players' spells.
- **Creatures give no stealth alert.**
