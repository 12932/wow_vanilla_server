# Re-survey: quests and the economy, wvs (master 0c5811f4) against mangos-classic

Scope: quests and their scripts (escorts, quest-started events, scripted credit), loot, items, vendors, the auction
house, mail, banks, trade, professions, and the economy. Read-only: I read both codebases, the Thistle Tea script dump
(`src/game/data/tt_scripts.json.gz`, decompressed to /tmp), and `data/db/vmangos.sqlite` (opened read-only).

Round 12 (`p12d loot`, `12k`, `p12h`) fixed most of the first survey: 25 of its 37 findings are fixed, 3 are partly
fixed, 7 are still open, and 2 I skip because DECISIONS documents them:
- the grey "!" with no `LowLevelHideDiff` (decision 69, #21);
- need before greed rolling only among those who can use the item, a lone one getting it (decision 73).

The opcode comparison found nothing missing in these areas. Every handler in mangos's `Opcodes.cpp` is matched in wvs
except `CMSG_QUESTGIVER_QUEST_AUTOLAUNCH` and `CMSG_STABLE_REVIVE_PET`, which mangos-classic only logs. The new gaps are
quest content that mangos-classic scripts in C++ and that neither vmangos's data nor Thistle Tea's dump covers.

## Dreadsteed of Xoroth (warlock epic mount) can't be done
- status: never implemented
- impact: high
- who notices: every level-60 warlock. Quest 7631 is the last step of the epic mount chain.
- mangos:
  - `ScriptDevAI/scripts/kalimdor/dire_maul/dire_maul.cpp` (`npc_warlock_mount_ritual`'s `UpdateAI`) and
    `instance_dire_maul.cpp:ProcessDreadsteedRitualStart` run the event:
    - the Wheel of the Black March, the Doomsday Candle and the Bell of Dethmoora (`go_ritual_wheel`, `go_ritual_candle`
      and `go_ritual_bell`, objects 179672 to 179674) must be kept up;
    - nine waves of Xorothian imps and dreadlords come;
    - the Dreadsteed Portal opens and the Xorothian Dreadsteed (14502, `boss_xorothian_dreadsteed`) comes out;
    - `TYPE_DREADSTEED` fails the event if the ritual lapses.
- wvs: absent.
  - `map/instance_adapters.rs`'s Dire Maul (`dm_set`, `dm_creature`) covers the generators, Immol'thar, Zevrim,
    Alzzin and the tribute.
  - Nothing in `src/` or the Thistle Tea dump names 179672 to 179674, 179681, 14482 or 14502.
- size: M

## Escorts follow only their starter, not the group
- status: open
- impact: medium
- who notices: groups doing escort quests (169 party-accept quests: the OOX escorts, Keeshan and others)
- mangos:
  - `npc_escortAI::IsPlayerOrGroupInRange` keeps the escort going while any member is in range. It is checked on a
    timer and skipped while the escortee fights.
  - `FailQuestForGroup` fails the quest for every member who has it.
- wvs:
  - `map/escorts/mod.rs:step` measures only `escorts[i].player`, every update, mid-fight too.
  - `escorts::fail` calls `quests::fail` for the starter alone. Only the credit (`escorts::credit`) reaches the group.
  - If the starter dies, logs out or is knocked away, the escort fails beside the rest of the group, and the members
    are left with a quest that is neither failed nor finishable.
- size: S

## Jail Break! (BRD, 4322) can't be finished
- status: never implemented
- impact: medium
- who notices: Alliance players in Blackrock Depths. The quest leads on to Stormwind Rendezvous and the Onyxia
  attunement.
- mangos: `scripts/eastern_kingdoms/blackrock_depths/blackrock_depths.cpp`:
  - `QuestAccept_npc_marshal_windsor` starts an escort through the prison;
  - Dughal Stormwing and Tobias Seecher are freed by gossip (`GossipSelect_npc_dughal_stormwing`,
    `GossipSelect_npc_tobias_seecher`);
  - Crest Killer's waves attack on the way;
  - `GroupEventHappens` gives the credit.
- wvs: absent. 9023, 9022, 9679 and 9682 appear nowhere in `src/`. `docs/THISTLE_TEA_DIFFERENCES.md` (Quests and
  events) lists it as unfinishable "on Thistle Tea either". That is a shared gap, not a design choice, and
  mangos-classic does script it.
- size: M

## The Challenge (BRD, 9015): no Theldren event at the Ring of Law
- status: never implemented
- impact: medium
- who notices: players doing the Dungeon Set 2 (T0.5) chain. The Banner of Provocation is meant to bring Theldren and
  his party into the Ring of Law.
- mangos: `blackrock_depths.cpp`:
  - placing the banner (181058) during the sentence sets `DATA_BANNER_BEFORE_EVENT`;
  - the ring's last wave is then Theldren (16059) and four of eight adds (`MAX_THELDREN_ADDS`);
  - the kill credit is 16166 (`NPC_THELDREN_QUEST_CREDIT`).
- wvs: the Ring of Law runs (`instance_script.rs`, BRD adapter), but 181058, 16059 and 16166 appear nowhere in `src/`.
  The banner does nothing, so 9015 can't be completed.
- size: S

## The Sunken Temple: no Avatar of Hakkar event (quest 3528) and no Atal'ai statue puzzle
- status: never implemented (likely; the instance script's fields show none of it)
- impact: medium
- who notices: groups in the Sunken Temple on The God Hakkar, which needs the filled Egg of Hakkar, and anyone who
  wants Atal'alarion
- mangos:
  - `scripts/eastern_kingdoms/sunken_temple/sunken_temple.cpp`: `TYPE_AVATAR` drives the Shade of Hakkar's summoning,
    the Hakkari Bloodkeepers, the eternal flames (`DoUpdateFlamesFlags`) and the Avatar of Hakkar (8443). A Bloodkeeper
    reaching its end fails it.
  - `TYPE_ATALARION` runs the statue order (events 3094 to 3100) that brings Atal'alarion.
  - `sunken_templeScripts.cpp` handles quest 3528's egg.
- wvs: `map/instance_adapters.rs:st_set` and `st_creature` keep only the six protectors, Jammal'an's barrier and
  Eranikus. Fields 0 and 4 fall through to `settle`, and nothing names 8443 or the statue events. vmangos's
  `event_scripts` 3094 to 3100 and 8502 hold one step each and leave the rest to C++.
- size: M

## Grey-kill reputation is a flat 20%, not mangos's sliding scale
- status: open
- impact: medium
- who notices: level-60 reputation grinders (Timbermaw, Cenarion Circle, Argent Dawn)
- mangos: `Entities/Player.cpp:CalculateReputationGain` gives `max(minRate, 1 - 0.2*(level - greenRange - creatureLevel))`
  for a trivial kill.
- wvs: `map/reputation.rs:level_rate_with` gives `least.0` (0.2) to every grey kill. A creature one level into grey
  gives about 80% in mangos and 20% in wvs.
- size: S

## Trade initiation skips mangos's refusals, including trading with yourself
- status: open
- impact: low (but the self-trade can lose items)
- who notices: crafted packets; players who are stunned or on a flight path
- mangos: `TradeHandler.cpp:HandleInitiateTradeOpcode` refuses:
  - an initiator who is stunned, logging out or on a taxi;
  - a target who is stunned or on a taxi;
  - yourself as the target.
- wvs: `map/trade.rs:initiate` checks death, faction, distance, the target logging out and ignore, but none of those.
  `with == g` opens a trade with yourself.
- size: S

## Group-loot chests (data15) are free for the opener
- status: open
- impact: low
- who notices: Naxxramas raiders (the Four Horsemen Chest), Arena Spoils
- mangos: `Loot::SetGroupLootRight` applies the group's method to a chest whose template sets `groupLootRules`
  (data15).
- wvs: `map/loot.rs:open_object` never reads `data[15]` and never calls `setup_group`. A raid boss's whole loot goes to
  whoever clicks first.
- size: S

## Quests flagged STAY_ALIVE don't fail on death
- status: open
- impact: low
- who notices: paladins (1651), warriors (1719), priests (7622), and quests 647, 1090 and 1699
- mangos: `Player::FailQuestsOnDeath` fails each incomplete quest with `QUEST_FLAGS_STAY_ALIVE` (0x1).
- wvs: absent. Nothing reads quest flag 0x1.
- size: S

## Banked items count toward a quest, but the turn-in takes only carried items
- status: open
- impact: low
- who notices: players who bank quest items
- mangos: progress counts the bank (`GetItemCount(id, true)`), but `CanRewardQuest` (Player.cpp:12397) checks carried
  items only, so the turn-in refuses with "missing items".
- wvs: `map/quests.rs:complete`, `delivered` and `wanted_items` all use `Player::count_of`, which includes the bank.
  `choose_reward` then takes items and fails partway.
  `inventory.rs:carried_count_of` exists but isn't used here.
- size: S

## Rolls start at the kill, and a lone eligible player under group loot gets the item at once
- status: partly
- impact: low
- who notices: raids
- mangos: `Loot::ShowContentTo` runs `GroupCheck()` at the corpse's first opening. A roll with one candidate leaves
  the item for normal looting.
- wvs: `loot.rs:creature_died` calls `start_roll` at death, so the 60-second timers run down mid-fight. `start_roll`
  awards a lone eligible player as a need, under group loot too. Decision 73 covers need before greed only.
- size: S

## Master loot gives no error back
- status: partly (the recipient rule and the threshold are fixed)
- impact: low
- who notices: raid leaders
- mangos: `HandleLootMasterGiveOpcode` replies `LOOT_ERROR_MASTER_INV_FULL`, `MASTER_UNIQUE_ITEM` or `MASTER_OTHER`.
- wvs: `loot.rs:master_give` keeps the item blocked silently (its comment says the reply isn't sent because no client
  check covers it).
- size: S

## The auction search and the sweeps still scan everything under realm-wide locks (R4-45)
- status: partly (040369e7 made the search lighter)
- impact: low to medium on a busy realm
- who notices: a populated realm; full-scan addons
- mangos: `BuildListAuctionItems` runs on the session thread.
- wvs:
  - `social/auction.rs:list` still walks every auction under `auctions.lock()` on the map thread, for every page.
    The names are now pre-lowercased and the results unsorted, so each match costs less.
  - `auction::sweep` and `mail::sweep` scan all auctions and all mail every 5 s.
- size: M

## Quest money ignores Rate.Drop.Money; a complete repeatable quest shows a yellow "?"
- status: open
- impact: low
- who notices: custom-rate realms; players turning in repeatable quests
- mangos:
  - `Quest::GetRewOrReqMoney` scales a positive reward by `RATE_DROP_MONEY`;
  - `GetDialogStatus` (QuestHandler.cpp:670) gives `DIALOG_STATUS_REWARD_REP` for a complete repeatable quest.
- wvs:
  - `quests.rs:choose_reward` and `auto_reward` use `q.reward_money` unscaled;
  - `status` gives `Reward2` for every complete quest in the log.
- size: S

## Rocknot's Ale and The Love Potion: no follow-on events in the Grim Guzzler
- status: never implemented
- impact: low
- who notices: BRD groups (Rocknot's walk to the kegs, Nagmara and Rocknot's scene, which opens a shortcut)
- mangos: `blackrock_depths.cpp:QuestRewarded_npc_rocknot` (`TYPE_ROCKNOT`, counted beers, the escort to the kegs)
  and `QuestRewarded_npc_mistress_nagmara` with `GossipSelect_npc_mistress_nagmara`.
- wvs: absent. 9503 and 9500 have no script (the quests turn in, nothing follows).
- size: S

## Outdoor group kills give each member full reputation
- status: open (check against 1.12)
- impact: low
- who notices: groups grinding reputation outdoors
- mangos: `Group::RewardGroupAtKill_helper` gives `1/count` outdoors.
- wvs: `reputation.rs:creature_killed` gives every member within reward distance the full amount.
- size: S

## A pickpocketed creature never restocks
- status: open
- impact: low
- who notices: rogues farming pickpocket loot
- mangos: `Creature::StartPickpocketRestockTimer` lets the creature be picked again after a delay.
- wvs: `map/professions.rs` pickpocket: `loot_state.picked` holds the creature for its whole life. A second pick gives
  only the rogue's quest drops.
- size: S

## Fixed since
- Group loot: a current looter under every method but free for all, `released`, `looter_pass`, and round robin no
  longer locking the corpse (`loot.rs:setup_group`, `Loot::slot_type`; 0c8775ad).
- A guild master's charter and crest options open their windows (`npc.rs:select`; ec321330).
- Quest MaxLevel is loaded and checked (`quests.rs:over_level`, used in `can_take`; 0f4fac70).
- Free-for-all drops (`ITEM_FLAG_MULTI_DROP`) drop once for each member (`loot.rs:slot_for`, `slot_type`'s `ffa`).
- Loot conditions are checked at the roll, and a grouped roll falls to another entry (`loot.rs:roll_rated`'s `unmet`;
  `creature_died` passes `conditions::met` over the group).
- Gossip options are filtered per player as `PrepareGossipMenu` does (`npc.rs:offered`).
- A flight path is learned as its flight master's menu is shown (`npc.rs:show_menu` with `discover_node`).
- Read mail keeps its 30 days (`social/mail.rs:mark_read`; d68a2c39).
- Mail opcodes require a mailbox within reach (`map/social.rs:at_mailbox`).
- Taking a mail's item holds the lock from the read to the clear (R4-21; `mail.rs:take_item`).
- The PvP rank discount is in: Sergeant gives another 10% at a friendly NPC (`reputation.rs:percent`; 9d55e891).
- Auction mails carry the invoice body and the random property in the subject (`auction.rs:settle`, `invoice`;
  7bacdc7f).
- Quest-starting drops are hidden from players who have or have done the quest (`loot.rs:sight`'s `started`,
  `quests::begun`).
- Vendors hide other races' items and other classes' Bind-on-Pickup items, and won't sell them (`vendor.rs` mask
  checks; 9d55e891).
- Duplicates in a loot group are re-picked half the time (`roll_rated`'s `IsItemAlreadyIn` analogue).
- Gold is split among those the kill fell to (`loot.rs:take_money` over `allowed`).
- An item under a roll shows as view-only (`slot_type` returns `SLOT_VIEW`).
- Corpse timers after looting and skinning, and a leftover skin free for all (99b76b34).
- The master looter gives to anyone in the group on the map (3ac47555), and the threshold is clamped to Uncommon or
  higher (`groups.rs:threshold`).
- Postage is 30c flat (`mail.rs:send`).
- A COD mail can't be deleted (`mail.rs:delete`).
- Conjured and timed items can't be mailed (`mail.rs:send`).
- "Repair All" repairs item by item, worn gear first (`vendor.rs:repair`).
- Sold-auction money is paid at once (`auction.rs:settle`).
- Money is capped at `MAX_MONEY` (`inventory.rs:set_money`), and delayed mail is announced after a restart
  (`Mails::load` rebuilds `pending`).
