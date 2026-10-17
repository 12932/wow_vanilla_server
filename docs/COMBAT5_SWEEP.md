# Combat, items, loot, professions, vendors and death: a parity sweep

A module-by-module comparison of Thistle Tea's `lib/game/core/{combat,item,
loot,profession,death,vendor,inventory}` with wvs, written down gap by gap
before each is fixed. Each entry names the Thistle Tea module, what differs,
and its state: **fixed**, or **left** with why.

## core/combat

1. `attack_table.ex`: the defender's `mod_attacker_melee_hit_chance` (184),
   `mod_attacker_ranged_hit_chance` (185), `mod_attacker_melee_crit_chance`
   (187) and `mod_attacker_ranged_crit_chance` (188) change the attacker's
   hit and crit chance. wvs ignored them.
2. `attack_table.ex`: a melee or ranged ability with a mechanic is resisted
   in the band just past the miss chance, as wide as the target's mechanic
   resistance (117). wvs's ability roll had no resist.
3. `attack_table.ex`: a player with weapons sheathed can't block.
4. `casting_combat.ex`: launching a cast whose interrupt flags hold 0x08,
   without `do_not_reset_combat_timers` (ex2 0x20000), restarts the melee
   swing timers; a spell with attribute 0x100000 stops its caster's attack
   as it finishes. wvs did neither.
5. `sheath.ex`: a sheath change interrupts a channel and takes off auras
   that sheathing breaks (0x200). wvs only set the byte.
6. `combat_control.ex`: Pacify-and-Silence pacifies too (auto attack held,
   the pacified flag set), as does Feign Death; wvs held only plain Pacify.
7. `healing_received.ex`: of percentage healing modifiers only the
   strongest reduction and the strongest increase apply; flat healing taken
   is by school and can't take more than half. wvs multiplied every
   percentage together and ignored the school.
8. `player/experience.ex`, `group_reward.ex`, `kill_credit.ex`:
   - elites are ranks 1, 2 and 3, and give 2.5 times in a five-man dungeon
     (2 elsewhere); wvs gave 2 for ranks 1 and 2 only;
   - the level factor below the player's level is a float rounded half to
     even, not integer division;
   - a group member below the highest non-grey level gets half its share
     of the level-weighted XP plus one; wvs gave half the whole kill;
   - groups above five (raids) share at 1 - 0.05 per member; wvs gave 1.0;
   - the original tagger counts in the level sum while dead;
   - a creature summoned as a critter, totem or untyped creature, or with a
     health multiplier at or below 0.1, gives no XP; nor does one a player
     controls.
9. `damage_origin.ex`: a creature tracks damage from players and from
   NPCs; unless players did more than 35% it drops no loot and its tap
   clears, and XP scales by the players' share. wvs had none of it.
10. `world/entity/mob/corpse.ex`: a corpse lies 300 seconds, an elite's
    (ranks 1-3) 600. wvs: 60 and 300.
10a. `assistance.ex`, `world/combat/call_for_help.ex`: a creature that
    calls for help, entering a fight, gathers the creatures of its faction
    within ten yards in sight whose faction answers calls (flag 0x01, not
    0x400), who are free and no friend of the enemy, and they come a second
    and a half on if still free; fighting more than ten yards from its spawn
    it calls each second as far as its call for help range, to friendly
    factions too. wvs called at once as far as its range, any of the same
    faction id, in sight or not, answering flag or not, and never again.
10b. `threat.ex`: a creature looks down its threat list from the top: past
    110% of its victim's threat one in melee range takes over, past 130%
    anyone; one at range between the two doesn't stop one in melee further
    down. A taunt raises the taunter to the victim's threat, not the top's.
    wvs looked at the top alone, and taunted to the top.

## core/loot

11. `loot.ex`: at most 16 items; references nest at most three deep; a
    quest drop (negative chance) in a group is picked by its chance like any
    other. wvs: no cap, four deep, and grouped quest drops never picked.
12. `world/entity/player/looting.ex`: money splits evenly among the group
    near the looter (74 yards), the remainder lost, every share announced
    (`SMSG_LOOT_MONEY_NOTIFY`, the looter's too, alone or not), from
    objects as well as corpses. wvs gave the looter the remainder, told no
    one alone, and split only corpses.
13. `loot_roll.ex`: a tie goes to the first roller by guid; the winner must
    be within 74 yards or the item goes free. wvs: the last, and anywhere.

## core/item, vendors and durability

14. `durability.ex`, `effect_resolver/durability.ex`: wear on death is 10%
    of each equipped item's maximum, at least a point; none in a
    battleground, nor dying to a player or their pet, and dying to the
    world counts. Each blow between players or with a creature wears one
    point off a random equipped item of each player involved, one time in
    two hundred. wvs wore down on death to a creature only (pets too), could
    wear nothing, and wore nothing per blow; broken gear kept its stats.
15. `durability.ex`: repairing all is all or nothing, priced
    `round(lost * cost * quality)` times the reputation discount, rounded
    half to even, at least 1 copper, over worn and carried items only. wvs
    repaired until the money ran out, truncated, without the discount, and
    took in the bank and buyback slots.
16. `vendor/buyback.ex`: a damaged item sells for its price less its repair
    cost (at least 1); a charged item for its share of charges left; only
    carried items are sold; a part of a stack sold is bought back as its
    own item; the slot is the next one, else the first free, else the
    oldest; its timestamp is seconds since login plus thirty hours; money
    stops at 2^31-1; buyback empties at logout. wvs differed on each.
17. `vendor/vendor_stock.ex`: limited stock comes back a buy count at a
    time, each restock interval after the last purchase, the interval
    scaled 80-120% for items flagged so. wvs refilled everything at once.
18. `equipment_transitions.ex`: in a fight only weapons, projectiles,
    shields, held-in-off-hand items and relics change, and weapons not
    while the weapon lock holds (spells5 had already brought this in);
    equipping an item with an on-use spell puts that spell on a 30-second
    cooldown, which wvs never started.

19b. `equipment_sets.ex`: a set whose `ItemSet` row asks a skill
    (`required_skill`, `required_skill_rank`) gives its bonuses only to one
    with that much of it. wvs gave them to anyone.

## core/profession

20. `skinning.ex`, `world/entity/mob/corpse.ex`: a skinning attempt rolls
    the skill (with its bonuses) from 25 below to 37 above, and fails
    ("try again") under the corpse's need unless the skill is 300; elites
    of rank 3 double the skill-up chance too. wvs always skinned, and
    doubled for ranks 1 and 2 only.
21. `loot/pickpocket.ex`: a creature's pockets are rolled once a life, but
    a later rogue still finds their own quest drops there. wvs refused any
    pick after the first ("no pockets").

## core/death

19. `world/entity/player/spirit_healer.ex`: a spirit healer's price is a
    quarter of the durability of everything carried, not only what's worn.
