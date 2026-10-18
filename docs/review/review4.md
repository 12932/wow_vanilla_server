# Review 4: the world's systems

Branch `fix/review4` in `<worktree>/review4`, from 3fe6640. Line numbers are at that commit.
Scope: `src/game/social/`, `map/inventory.rs`, `loot.rs`, `quests.rs`, `npc.rs`, `vendor.rs`, `professions.rs`,
`objects.rs`, `scripts.rs`, `battlegrounds.rs`, `map/battleground/`, `arena.rs`, `outdoor_pvp.rs`, `honor.rs`, and
`data/load.rs`. Each finding: where, what is wrong, how it shows, severity, easy or hard, and its state (fixed in
a commit with its test, or left with what a fix would take). The tests are in `src/game/tests_review4.rs`.

## Fixed

### R4-1. A sold item can be taken back out of its buyback slot for nothing
- **Where:** `src/game/map/inventory.rs:993` (`move_item`), `:1193` (`stow`, behind `CMSG_AUTOSTORE_BAG_ITEM` and the
  bank's), `:1262` (`split`); `map/spells/mod.rs:2379` (`use_item`); `social/mail.rs:331` (`send`).
- **What:** `slot_exists` and `may_place` check only the destination. The source `(255, 69..81)`, a buyback slot,
  is found by `item_at` like any other, so an item sold to a vendor can be moved, split, auto-stored, equipped,
  used, or mailed out of it. The sale's money stays, and `PLAYER_FIELD_BUYBACK_PRICE_n` stays on an empty slot.
  mangos's handlers call `IsValidPos(srcbag, srcslot, true)` first, which leaves out the buyback slots.
- **How it shows:** `CMSG_SELL_ITEM`, then `CMSG_SWAP_ITEM {255, 69 -> 255, 28}`: the item is back in the
  backpack and the money is kept. Repeat to make money from nothing.
- **Severity:** high (money duplication). **Easy.**
- **State:** fixed in 4640e3e (test `a_sold_item_leaves_its_buyback_slot_only_bought_back`, which covers the
  move, split, auto-store, auto-equip and mail). The `use_item` guard has no test of its own.

### R4-2. Items added onto a stack never tell the client the stack's new count
- **Where:** `src/game/map/inventory.rs:849-855` (`store`).
- **What:** after a stack's count is raised (`set_count`), the loop meant to clear the new items' dirty bits clears
  every item of that entry's, the stacks joined among them. `flush_values` sends only dirty item fields, so the
  stack's `ITEM_FIELD_STACK_COUNT` never goes out.
- **How it shows:** carry 5 widgets and buy, loot, craft or be rewarded one more. The server holds 6, but the
  client shows 5 until the stack changes again.
- **Severity:** medium (the client is wrong about every stack added onto). **Easy.**
- **State:** fixed in ed1061f (test `a_stack_added_onto_shows_its_new_count`).

### R4-3. A bank bag with items in it can be destroyed, orphaning them
- **Where:** `src/game/map/inventory.rs:1320-1345` (`destroy`).
- **What:** the "bag with items" test covers only the bag bar (19-22), not the bank's bag slots (63-68), which
  `move_item`'s own test does cover. mangos's `HandleDestroyItemOpcode` refuses any `IsNotEmptyBag`.
- **How it shows:** `CMSG_DESTROYITEM {255, 63}` on a full bank bag: the bag goes, its items stay with `bag = 63`,
  unseen, and come back inside the next bag put there (or past its end, if it is smaller).
- **Severity:** medium. **Easy.**
- **State:** fixed in 8ac48f2 (test `a_full_bank_bag_is_not_destroyed`).

### R4-4. Anyone can close another player's loot window, and a rogue's pockets move to them
- **Where:** `src/game/map/loot.rs:1193` (`release`).
- **What:** `CMSG_LOOT_RELEASE` is acted on for any target, whether or not the sender has its window open. For
  pick-pocket loot it takes the creature's loot and files it in `pockets` under `(target, sender)`. For any loot
  it also clears `opened_by`, removes an item's loot, and runs `object_looted`/`corpse_looted`.
- **How it shows:** a rogue picks a creature's pockets; any player on the map sends `CMSG_LOOT_RELEASE {that
  creature}`. The rogue's window now has nothing behind it, and the pockets are the sender's to pick.
- **Severity:** medium. **Easy.**
- **State:** fixed in 2a0360a (test `another_players_release_leaves_a_rogues_pockets`): only a looter of that
  loot (`looters.remove` true) closes it.

### R4-5. Leaving the map with a loot window open doesn't close it
- **Where:** `src/game/map/mod.rs:785` (`take_player`); only `login::request_logout` released loot.
- **What:** a disconnect, or a map change, leaves the player in `Loot::looters`, a locked chest's `opened_by`
  (they can reopen it later without its lock), a pick-pocket window's leftovers unfiled, and a disenchanted
  item's or opened container's loot in `loot_state.item_loot` for good (keyed by an item that may be gone).
- **How it shows:** disenchant an item and drop the connection before looting: the loot stays on the map.
- **Severity:** low to medium. **Easy.**
- **State:** fixed in 88c4bb3 (test `a_loot_window_closes_with_its_looter_leaving`). A master looter who leaves
  still leaves the kept items unassignable (nobody else may `master_give`); mangos keeps them the master's too,
  so that is left.

### R4-6. A script that starts itself with no delay recurses until the stack overflows
- **Where:** `src/game/map/scripts.rs:115` (`start_checked`, which runs the due steps at once through `advance`),
  with command 39 at `:1840`, command 68 at `:2153`, and an object's scripts on its use.
- **What:** `start_checked` pushes the run and calls `advance` straight away, which runs every step already due;
  a step 39 (relay) naming its own script, or a ring of them, with no delay, starts it again inside itself, with
  no limit. A stack overflow is an abort, not a panic, so the map's catch (`docs/RELIABILITY.md`) can't save
  the process: every map goes down, and what wasn't saved is lost.
- **How it shows:** a `generic_scripts` row `(id A, delay 0, command 39, datalong A)`. Not in the shipped data
  (no generic script relays to itself at once), so it is a bad row's hazard.
- **Severity:** medium (a data row kills the server). **Easy.**
- **State:** fixed in 01b3de7 (test `a_script_starting_itself_at_once_stops`): a start 32 deep is dropped, logged
  at `debug`.

### R4-7. The auction house's cut overflows a `u32` on a big bid
- **Where:** `src/game/social/auction.rs:316` (`settle`) and `:489` (`cancel`'s fee).
- **What:** `a.bid * cut / 100` in `u32`: past 286,331,153 copper (28,633 gold) at the goblins' 15%, or 85,899
  gold at a faction's 5%, it overflows. Bids are limited only by the bidder's money. A debug build panics (in the
  realm's sweep, or in the map at a buyout); a release build wraps and takes a cut of almost nothing.
- **How it shows:** a 30,000 gold buyout at Gadgetzan.
- **Severity:** medium. **Easy.**
- **State:** fixed in 249a28b (test `a_big_bids_cut_doesnt_overflow`): the cut in `u64`, the next bid's minimum
  and the seller's payment saturating.

### R4-8. Deleting a seller loses the money bid on their auctions
- **Where:** `src/game/social/auction.rs:715-736` (`character_deleted`).
- **What:** the deleted owner's auctions are removed with their records, but a bidder's money (taken at the bid)
  is never mailed back, as `cancel` mails it (`CANCELLED_TO_BIDDER`).
- **How it shows:** B bids 50 gold on A's auction; A deletes the character. B's 50 gold is gone.
- **Severity:** medium (money lost). **Easy.**
- **State:** fixed in 70be8d0 (test `a_deleted_sellers_bidder_is_paid_back`), the refund in one transaction with
  the auction's deletion.

### R4-9. Empty custom channels with a password or a ban are never removed
- **Where:** `src/game/social/channels.rs:302-305` (`leave`), and `kick` (`:489`), which never removed an emptied
  channel.
- **What:** `leave` drops an empty channel only if it has no password and no bans. mangos's
  `ChannelMgr::LeftChannel` and Thistle Tea's `Channels.put` drop any empty channel. So a client can `/join x`,
  `/password x p`, `/leave x`, for any number of names: each stays in `by_key` for good, ownerless and locked,
  and `Channels::of` (every logout) scans them all.
- **How it shows:** the loop above; the channels map grows by one per name.
- **Severity:** medium (client-driven unbounded growth). **Easy.**
- **State:** fixed in 6de5bd0 (test `an_empty_locked_channel_goes`).

### R4-10. A spawn tied to more than one event keeps only one tie
- **Where:** `src/game/data/load.rs:1451-1461`; the maps' type in `data/mod.rs:524`; read by `events::spawned`.
- **What:** `game_event_creature` and `game_event_gameobject` are keyed by (guid, event) in vmangos, but loaded
  into `AHashMap<u32, i32>`: a later row overwrites the earlier one (row order is unspecified). In the shipped
  data 153 creature spawns and 696 object spawns have more than one.
- **How it shows:** objects 363477 and on are tied to 34 (New Year's Eve) and 38 (Lunar Festival): only one is
  kept, so they miss the other. Creatures 80360 and on are tied to -23 and -4 (the Faire's building, the Faire):
  only one is kept, so they stand through the other.
- **Severity:** medium (the real world's events are wrong). **Easy.**
- **State:** fixed in 7c78008 (tests `a_spawn_tied_to_two_events_follows_both`, and
  `the_real_spawns_keep_every_event_they_are_tied_to` in the extended suite). The ties are a `Vec<i32>`; a spawn
  is out if it is tied to no event or any of its positive events is on, and none of its negative ones is.

### R4-11. A quest taken already done is never told complete
- **Where:** `src/game/map/quests.rs:807-812` (`accept_with`) with `:936-964` (`check`).
- **What:** `accept_with` calls `refresh_log`, which sets the slot's `QUEST_STATE_COMPLETE`, and then `check`, which
  reads that bit as what the quest was before: "was complete", so `SMSG_QUESTUPDATE_COMPLETE` is never sent.
  Thistle Tea's `force_accept` sends it when the quest evaluates complete at once.
- **How it shows:** take a quest for an item you already carry: the log shows it done, but there is no "objective
  complete" message.
- **Severity:** low. **Easy.**
- **State:** fixed in ca7a26c (test `a_quest_taken_done_is_told_complete`): `check` refreshes the log itself.

### R4-12. Abandoning a quest leaves the area auras its being taken gave
- **Where:** `src/game/map/quests.rs:1495-1535` (`abandon`).
- **What:** taking and rewarding a quest call `spell_areas::reconcile`; abandoning doesn't, so an aura granted
  while `quest_start_active` and the quest is in the log stays until something else reconciles (an area change).
- **How it shows:** a `spell_area` row with `quest_start_active`; take the quest, abandon it: the aura stays.
- **Severity:** low. **Easy.**
- **State:** fixed in 572ca9c (test `an_abandoned_quests_area_aura_goes`).

### R4-13. A used-up object a script brings out loses its respawn
- **Where:** `src/game/map/scripts.rs:1462-1487` (command 9) and `:3208-3227` (`update_objects`), with
  `objects.rs:1518-1538` (`update`'s respawns).
- **What:** command 9 brings out an object spawn that is absent for `datalong2` seconds, then removes it with
  `remove_gameobject`. If the spawn was used up (a looted chest, a gathered node) and its respawn comes due while
  the script holds it out, `objects::update` drops the respawn (the object is out), and the script's removal
  then takes it for good. `remove_gameobject` also skips `objects::forget`, so a trap's spent charges, a ritual
  or a pending close stay keyed by the object's GUID, which a spawn's next copy has again; for command 76's
  summoned traps, each leaves its `traps` entry behind.
- **How it shows:** a chest looted (back in 5 minutes), brought out by a script for 10 minutes: once the script
  takes it in, the chest never returns until the map restarts.
- **Severity:** medium. **Easy.**
- **State:** fixed in e6797c1 (test `a_spawn_a_script_brought_out_still_comes_back`). Command 9 keeps the pending
  respawn aside (`Objects::brought_out`) and puts it back when the script takes the object in, if it wasn't used
  up again meanwhile; the removal goes through `despawn_then`; and `despawn_then` keeps one respawn a spawn.

### R4-14. A guild charter is turned in without being held
- **Where:** `src/game/social/guilds.rs:1360-1428` (`petition_turn_in`).
- **What:** the petition is looked up by the GUID the client sends, and the charter item is taken only `if` it is
  still there. mangos's `HandleTurnInPetitionOpcode` returns at once without the item.
- **How it shows:** sign a charter fully, sell it to a vendor (or destroy it), send `CMSG_TURN_IN_PETITION` with
  its old GUID: the guild is founded.
- **Severity:** low. **Easy.**
- **State:** fixed: the test `a_charter_not_held_founds_no_guild` is in 1494611, but the fix itself went into 48e1d6d (with R4-15's) by a staging slip.

### R4-15. An auction can be cancelled away from the auction house
- **Where:** `src/game/social/auction.rs:478` (`cancel`).
- **What:** every other auction command checks `house_at` (an auctioneer, near); `cancel` doesn't. mangos's
  `HandleAuctionRemoveItem` does.
- **How it shows:** `CMSG_AUCTION_REMOVE_ITEM` from anywhere returns the item by mail.
- **Severity:** low. **Easy.**
- **State:** fixed in 48e1d6d (test `an_auction_is_cancelled_only_at_an_auctioneer`).

### R4-16. Buying back skips the unique limit
- **Where:** `src/game/map/vendor.rs:535-592` (`buyback`).
- **What:** buyback checks money, stack room and a free slot, but not `max_count`; `count_of` leaves the buyback
  slots out, so a unique item sold doesn't count while another is had. mangos's `HandleBuybackItem` stores
  through `CanStoreItem`, which applies the limit.
- **How it shows:** sell a unique item, get another (a vendor, a quest), buy the first back: two.
- **Severity:** low. **Easy.**
- **State:** fixed in c75b6cb (test `a_unique_item_bought_back_keeps_its_limit`).

### R4-17. Honor earned in the last seconds before a shutdown is lost
- **Where:** `src/game/mod.rs:555-582` (`shutdown_within`), with `map/pvp.rs:791` (`sweep`, the only writer of the
  honor ledger's records, every five seconds from `social::SWEEPS`).
- **What:** the shutdown saves every map's players and flushes the store, but the ledger's dirty entries wait for
  the next sweep, which never comes. The character row is saved, but `pvp::refresh` reads honor from the ledger.
- **How it shows:** win a battleground (its bonus honor) or make a kill, then SIGTERM within five seconds: after
  the restart the honor is gone. A crash loses the same window, which is the design's.
- **Severity:** medium (lost state at every clean shutdown). **Easy.**
- **State:** fixed in ea3db83 (test `honor_earned_just_before_a_shutdown_is_kept`).

## Left, by severity

### Medium

#### R4-18. Round robin: only the assigned looter may open a corpse, so others' quest drops and the gold are out of reach
- **Where:** `src/game/map/loot.rs:145-156` (`Loot::may_loot`), used by `show`, `take_item` and `take_money`.
- **What:** under round robin, `may_loot` is false for every allowed member but the one assigned. `creature_died`
  rolls quest items for each member's own quests and `visible` shows a member theirs, but a member not assigned
  gets no window at all; the corpse keeps sparkling for them. mangos's `Loot::CanLoot` lets any member with
  rights take FFA items (quest drops) and the gold whatever the method, and when the current looter releases the
  window (`Loot::Release`) the leftover items are released to the group. wvs never clears `assigned`, so leftovers
  stay the assigned looter's until the corpse goes.
- **How it shows:** two players on round robin, both on a quest with a drop: the one not assigned can never loot
  their drop from that corpse.
- **Severity:** medium. **Hard-ish** (a design point, not a one-liner).
- **What a fix takes:** let any allowed member past `may_loot`, and filter in `visible`/`take_item` by kind: quest
  slots their own, the gold anyone's (mangos's `LOOT_STATUS_CONTAIN_GOLD`), other items only the assigned looter's
  until released; add a `released` flag set when the assigned looter closes the window (`release`), as mangos's
  `isReleased`. Check Thistle Tea's round robin before choosing (its `Looting` module).

#### R4-19. Groups are lost at every restart
- **Where:** `src/game/social/groups.rs` (`Groups`).
- **What:** no record kind or store write exists for groups. A restart (or crash) dissolves every party and raid:
  leader, loot method, subgroups and assistants, and the group-keyed instance links `instances::group_changed`
  keeps. mangos persists them (`groups`, `group_member`).
- **Severity:** medium (state that should survive a restart). **Hard** (a new record kind, written on every
  change, loaded with offline members; the instance links reconciled on load).

#### R4-20. Alterac Valley: a general killed with no player credit doesn't end the match
- **Where:** `src/game/map/battleground/av.rs:547-583` (`creature_died`).
- **What:** the side is the tapper's (`m.team_of(killer)`, from the scores), so a general (events 61/62) killed by
  a creature alone, or tapped by someone not in the match, records nothing and the match goes on. The general
  respawns at his time and must be killed again. In this AV the NPC assaults (Ivus, Lokholar, the riders, the
  cavalry, the ground troops) fight in the enemy's base, so it is reachable. How `die_by` sets the tapper when
  only creatures did damage wasn't traced.
- **Severity:** medium. **Easy**: when the tapper has no side, take it from the event (an Alliance leader's death is
  the Horde's, and the reverse), at least for 61/62; the scoreboard credit then goes to nobody.

### Low to medium

#### R4-21. A mail's item can be duplicated if the mail expires as it is taken
- **Where:** `src/game/social/mail.rs:454-543` (`take_item`).
- **What:** the item is cloned under the mail lock, the lock dropped, `attach_item` run, and the mail re-locked to
  clear it. `list` doesn't filter expired mail, so an expired mail can be taken from; if the realm's sweep (another
  thread, every 5 s) sends it back in that window, `own_mail` no longer finds it: the taker keeps the item, the
  returned mail keeps it too, and a COD payment is still posted.
- **Severity:** low to medium (a duplication, through a narrow race). **Easy to medium**: take the item out of the
  mail under the lock first (`m.item.take()`), attach it, and put it back if `attach_item` fails.

#### R4-22. A charter destroyed or sold keeps its petition, and its name, for good
- **Where:** `src/game/social/guilds.rs:199-208` (`name_taken` counts petitions), `:1119` (`petition_buy`).
- **What:** a petition goes only at turn-in or when its owner is deleted. Destroying, selling or mailing the
  charter leaves it, persisted (`KIND_PETITION`): the name stays taken for everyone, and a client can reserve
  names (and grow the store) at 10 silver each.
- **Severity:** low to medium. **Medium**: delete the petition where its charter item leaves the character
  (destroy, sell, and the item's deletion at a mail's or auction's end), or let `name_taken` ignore a petition
  whose charter its owner no longer holds (checkable only for the online).

#### R4-23. `.honor week` loses the rest of that week's honor
- **Where:** `src/game/honor.rs:181-188` (`settle_now`), `:206-209` (`settle_week`'s skip).
- **What:** the forced settle marks every entry `last_settled_week = start`; at the real week's end `advance` skips
  every entry for that week, so kills and honor after the GM's command are never ranked.
- **Severity:** low (a GM command). **Easy**, but a choice: don't mark the week settled when forcing (the week's
  end then ranks the rest again, decaying rank points a second time), or start the next week the day after.

### Low

- **R4-24. Mail money near the cap is lost.** `mail.rs:426-452` (`take_money`) zeroes the mail's money and
  `saturating_add`s it. mangos-classic's `HandleMailTakeMoney` loses it the same way (`ModifyMoney` caps); left.
- **R4-25. A guild leader passing leadership to themselves is demoted to officer.** `guilds.rs:738-775`
  (`set_leader`): `target` accepts oneself, so the leader ends at rank 1 with an officer's rights. mangos does the
  same; an early return when `t == me.guid` would fix it.
- **R4-26. Looting has no distance check after the window opens.** `loot.rs:1052`, `:1106` (`take_item`,
  `take_money`) check only `p.looting`. mangos's `HandleAutostoreLootItemOpcode` checks
  `IsWithinDistInMap(INTERACTION_DISTANCE)` and releases the loot otherwise. Easy.
- **R4-27. A corpse's gold is shared by the looter's group now, near the looter.** `loot.rs:1125-1131`
  (`tap_group(map, player, at)`), not the members allowed at the kill (`Loot::allowed`): one who joined after the
  kill shares, one who left doesn't. Easy.
- **R4-28. A battleground's scoreboard keeps those who left.** `battleground/mod.rs:514` adds a score in `arrived`;
  nothing removes it, so `team_of` still gives a deserter a side and `MSG_PVP_LOG_DATA` lists them. mangos's
  `RemovePlayerAtLeave` erases the score. Easy (remove on a leave before the end).
- **R4-29. A non-member on a closed battleground copy is never sent home.** `battleground/mod.rs:588-594` calls
  `battlegrounds::leave` for every player each update after the close; `leave` returns at once for one not in the
  match (a GM's `.tele`), so the copy never empties or unloads, and the map takes the battlegrounds lock for them
  20 times a second. Easy: fall back to the bind point.
- **R4-30. A match nobody accepts stays listed.** `battlegrounds.rs:465-485`: a `Match` made by `fill` whose
  invitations all lapse has no copy, so `closed` never removes it; it stays in `SMSG_BATTLEFIELD_LIST` and is
  topped up first. Bounded (reused). Easy: drop it in `sweep` with no members and no invitations.
- **R4-31. Alterac Valley's ground assault never re-arms after its commander dies while the quartermaster lives.**
  `av_quests.rs:944-950` sets `March::Dead`; only `back_from_the_dead` resets it, and only after the
  quartermaster died. Supplies still count, but the orders are never offered again. Not checked against
  Thistle Tea.
- **R4-32. Opening a second loot window doesn't release the first.** `loot.rs:1029` (`show`) overwrites
  `p.looting`; the first loot keeps the player in `looters` (and a lock's `opened_by`). Easy: release the old one.
- **R4-33. A ritual helper's cast is overwritten.** `objects.rs:1374-1402` (`start_channel`) sets
  `Caster::casting` over any cast under way, without interrupting it. Easy: `spells::interrupt` first, or refuse.
- **R4-34. Selling part of a stack loses the item's creator, flags and time.** `vendor.rs:465-487` makes the sold
  part with `Item::new`, unlike `split`, which copies them (mangos's `CloneItem`); bought back, a timed or made item
  comes back fresh. Easy.
- **R4-35. Letters' texts are never deleted.** `mail.rs:614-658` (`make_letter`) stores a text record for good,
  and leaks it in memory when `attach_item` fails. Slow store growth.

### Latent in the data (no failure on the shipped data)

- **R4-36.** `load.rs:1358-1368`: `gameobject_questrelation` and `gameobject_involvedrelation` ignore their patch
  range (`relation_any`), unlike the creatures' (`relation`); every shipped row covers patch 10. Easy.
- **R4-37.** `load.rs:2417`: `mail_loot_template` checks `patch_min` only, and a reference row (negative
  `mincountOrRef`) is mailed as one of its `item`; all 72 shipped rows are plain and in range. Easy.
- **R4-38.** `load.rs:2181-2199`: `pet_spell_data` keeps the last row read, not the latest build (59 lists have
  several); right today because rowids ascend with build. Easy: `ORDER BY entry, build`.
- **R4-39.** `1 << subclass` / `1 << inventory_type` (`derive.rs:215`, `professions.rs:498-500`,
  `trade_spells.rs:250-252`, `spells/checks.rs:224`): a value of 32 or more panics in a debug build and wraps in
  release; the shipped maximums are 20 and 28. Easy: `checked_shl(..).unwrap_or(0)`.
- **R4-40.** `t.power_type.min(4)` (`trade_spells.rs:221`, `:406`, `pets/ai.rs:390`, `:462`, `:574`,
  `combat.rs:2291`) maps POWER_HEALTH (-2, read as 4294967294) to happiness; harmless unless a trade or pet spell
  costs health (not confirmed). Easy: `spells::power_index`.
- **R4-41.** `outdoor_pvp.rs:459-494`: a tower whose template is missing is skipped, so `o.towers[i]` no longer
  lines up with `TOWERS[i]` (positions, banners, announcements). Easy: keep the index.

### Speed

- **R4-42.** `quests.rs:464-513` (`update_objects`): every 10 updates, for every player, every quest object they
  know is re-evaluated (`activates` → `giver_quests`, cloned relations, the quest graph's checks), most of it
  unchanged. Medium: recompute on a journal or known-set change.
- **R4-43.** `objects.rs:1541-1567`: the trap scan walks every game object on the map 5 times a second while
  anyone is on it, cloning each trap; on a continent tens of thousands. Easy: keep the traps' GUIDs as a set.
- **R4-44.** `mail.rs:352-362` (`send`'s cap) and `Mails::of`: a scan of every mail in the realm under the global
  lock; `auction::list` formats and lowercases every auction's name per search under the auctions' lock. Medium:
  per-receiver index, names lowered once.
- **R4-45.** `Player::refresh_inventory` sets each bag's 36 slot fields to zero and back, so every occupied slot is
  dirty and resent after any inventory change. Easy: set each slot once.

## Checked and not reported

- Arena `b.fighters.len() - b.alive as usize` (`arena.rs:1289`): `alive` starts at `fighters.len()` and only
  `fall` lowers it, once a fighter; no underflow.
- AB and AV capture timers (`ControlPoint` revisions), towers burning and paying once, mines' reclaim revisions,
  `finish` running once, the win honor paid once; AV's lieutenants (events 68/69) have no once-guard on purpose:
  six a side, each counted on its own death, with 5-day respawns.
- WSG's flag carrier on death, logout, leaving the map, losing the aura, the capture and the end.
- Ritual helpers who log out are taken out of the ritual: `take_player` interrupts their channel, whose end runs
  `objects::channel_ended`.
- Quest rewards are all or nothing (`plan_change` before any change), a choice index is bounds-checked, a rewarded
  quest can't be rewarded again; trainer, bank slot and taxi payments check money first.
- Guild ranks, friend and ignore caps, petition signers, group invites and raid icons are bounded and checked.
- `load.rs`'s `Cols` saturates every read, `rows` drops a bad row, and no row panics the load.
