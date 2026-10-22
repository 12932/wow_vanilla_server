# Bug hunt: items, quests, loot, vendors, professions and game data (wvs, master at 0c5811f4)

## A player can trade with themselves: a sold item comes back free, and bags can be orphaned
- kind: bug
- severity: high
- where: src/game/map/trade.rs:45 (initiate), src/game/map/trade.rs:344-435 (accept); src/game/map/social.rs:549
- what: `initiate` never checks `with != g` (mangos's `HandleInitiateTradeOpcode` answers `pOther == GetPlayer()` with Busy). A client that sends `CMSG_INITIATE_TRADE` with its own guid passes every check (it isn't trading yet, it's alive, the same team, 0 yards away, not ignoring itself) and gets `trade = Some(Trade { with: g })`. On `accept`, `both` is true at once and `mine == theirs` are the same index list. `take(map, g, mine)` detaches those items (`Vec::remove` shifts every later index down). Then `take(map, g, theirs)` detaches again at the same, now stale, indices, so it takes whatever item has slid into each one. That item gets no `tradable` check, and it can be anything: an item in the bank, an equipped item, a bag in a bag slot with its contents (they keep `bag = 19..22` and are orphaned), or an item in a buyback slot. All of them are then `attach_item`ed into the bags. Concrete gold dupe: sell X to a vendor (a whole sale keeps X's `Vec` position and only changes bag/slot to 69..80), then self-trade the item just before X in `p.items`. X comes back out of buyback while the player keeps the sale money, and this repeats. `plan_change_freeing` and `offer_holds` only ever look at the offered items.
- fix: in `initiate`, refuse `with == g` (`S::Busy`, as mangos does). As a second guard, have `accept` return early when `with == g`.
- fix size: trivial
- confidence: confirmed (the guard is missing and the index shift is clear in the code; whether a given item sits at index+1 depends on the order of `p.items`, which the player can steer)

## The disenchant loot window does nothing for a real client: the dust is lost and its loot entry leaks
- kind: bug
- severity: high
- where: src/game/map/items_misc.rs:149-158 (handle), :303-330 (window/take), :359-383 (release); src/game/map/professions.rs:1094; src/game/map/misc.rs:155 vs src/game/map/handlers.rs:86-92
- what: `disenchant` opens its loot with `loot::show_item_loot`. That stores the loot in `map.loot_state.item_loot` and sets `p.looting = Some(<the item's guid>)`, a `HighGuid::Item`. `misc::handle_all` (which runs `items_misc::handle`) comes before the handlers' loot match, and `items_misc::handle` claims `CMSG_AUTOSTORE_LOOT_ITEM`, `CMSG_LOOT_MONEY` and `CMSG_LOOT_RELEASE` whenever `looting` is an item guid. Its `take` and `take_money` only read `p.misc.item_loot` (the container window), which is `None` for a disenchant, so they return without doing anything. Its `release` clears `looting` and returns, so `loot::release` never runs and never auto-stores what is left. The result: the item is gone, nothing is received, and `loot_state.item_loot` keeps the entry for the life of the map. The tests don't see this. `tests_p3_items_professions.rs:443` and `tests_parity_misc.rs:1228` call `loot::take_item` directly. `tests_p10_player_items.rs:184` sends `CMSG_LOOT_RELEASE` twice: the first is swallowed by `items_misc`, and only the second reaches `loot::release`.
- fix: route by where the loot is, not by guid type. In `items_misc::handle`, add `&& p.misc.item_loot.as_ref().is_some_and(|(g, _)| Some(*g) == p.looting)` to `item_window`, or check `!map.loot_state.item_loot.contains_key(&target)`. Fix the test to send one release, and add one that takes a shard through `CMSG_AUTOSTORE_LOOT_ITEM`.
- fix size: small
- confidence: confirmed

## Lootable items can be opened from the buyback slots: sale money plus their contents
- kind: bug
- severity: medium
- where: src/game/map/items_misc.rs:178-300 (open_as), also :436 (wrap)
- what: `open_as` looks the item up with `p.item_at(bag, slot)` and has no `vendor::in_buyback` guard. The move, split, stow and use paths all have one (inventory.rs:1449/1797/1870, spells/mod.rs:3515). Sell a stack of clams, or an unlocked lockbox, then send `CMSG_OPEN_ITEM` for bag 255, slot 69+. The loot rolls and goes into the bags, and `release` takes one off the stack that is still in buyback. The buyback price stays at the full stack's, so the player keeps the sale money and also gets every container's loot. They can repeat this for each one in the stack. `wrap` also accepts paper or a target in buyback (take_item on the sold stack, no gain, but it is the same missing guard).
- fix: refuse `in_buyback(bag, slot)` in `open_as` and `wrap`, ideally through one helper that every slot-addressed item handler calls.
- fix size: trivial
- confidence: confirmed

## Spells cast in the trade window count and spend reagents from the bank and the buyback
- kind: bug
- severity: medium
- where: src/game/map/trade_spells.rs:228-243 (check: spare, totems), :427-440 (pay)
- what: `check` counts `spare` reagents over every item that isn't offered: banked, bought back, anywhere. `pay` then takes the first item of the entry at any index, also from the bank or a buyback slot. mangos's `HasItemCount` and `DestroyItemCount` only count carried items, and a normal cast here already uses `carried_count_of` (spells/checks.rs:605). An enchanter can enchant through trade with reagents left in the bank. Worse, they can sell their reagents to a vendor and have the trade enchant use up the buyback copies: they keep the vendor's money and spend no carried reagents. The totems check (`count_of`) also counts the bank.
- fix: count only `i.carried()` items in `spare` and `carried_count_of` for totems, and have `pay` skip anything not `carried()`, the same as the normal reagent path.
- fix size: trivial
- confidence: confirmed

## Skinning ignores untaken quest and free-for-all drops: they are lost for the rest of the group
- kind: bug
- severity: medium
- where: src/game/map/professions.rs:936, :671 (skinning_fails / skin, which use `Loot::is_empty`); src/game/map/loot.rs:176
- what: `skinning_fails` and `skin` let a corpse be skinned once `loot.is_empty()`. `is_empty` treats every `quest` and `ffa` slot as empty, taken or not. mangos lets a corpse be skinned only at `CREATURE_LOOT_STATUS_LOOTED` (Spell.cpp:5516, `IsLootedForAll`), which is what wvs's own `Loot::done` and `UNIT_FLAG_SKINNABLE` stand for. So when a group kills something whose drops include a quest item or a multi-drop item, the first skinner (in the group or not) can skin it as soon as the plain items and gold are gone. `skin` replaces `c.loot` with the skin's loot, so every member who hasn't yet taken their quest drop or free-for-all copy loses it.
- fix: require `l.done` (or the `UNIT_FLAG_SKINNABLE` flag that `corpse_looted` and `refresh_lootable` set) instead of `is_empty()` in both places.
- fix size: trivial
- confidence: confirmed (code against mangos's rule)

## Quest-object activation every half second does a global groups lock and allocations per player per known node
- kind: perf
- severity: medium
- where: src/game/map/quests.rs:530 (update_objects), :463 (activates), :492 (chest_for_quest), :1357 (in_raid)
- what: Every 10 updates, `update_objects` visits each player's known game objects, and herbs and veins are `GO_CHEST`. For each chest, `chest_for_quest` builds a fresh `Vec` of the table's quest items, following references. It then calls `wanted()`, which runs `in_raid()` (a `realm.social.groups.lock()`, shared by every map) and rebuilds the wanted list. Quest-giver objects run `giver_quests`, which clones the relation `Vec`s and locks `social.events` once per event quest. In a busy zone that adds up to thousands of global lock acquisitions and allocations a second, across every map, on the update path.
- fix: compute `wanted` (and `in_raid`) once per player per pass. Work out each gameobject loot table's quest items once, when the data loads (in `GameData`). Skip players whose quests and known set haven't changed since the last pass.
- fix size: small
- confidence: likely (the cost is clear from the code; it hasn't been measured)

## A party-accept quest is offered at 30 yards but refused past 10
- kind: bug
- severity: low
- where: src/game/map/questshare.rs:225-265 (accepted), :141-153 (admit), :274 (confirm)
- what: `accepted` sends `SMSG_QUEST_CONFIRM_ACCEPT` to members within `SHARE_YARDS * 3.0` (30 yards). When one confirms, `confirm` goes through `accept_asked` → `accept_with` → `questshare::admit`, which refuses with `TooFar` beyond `SHARE_YARDS` (10 yards). mangos's `HandleQuestConfirmAccept` checks no distance at all, only that the player is in the original player's party or group. So escort members standing 10 to 30 yards away are asked to join and then told they are too far.
- fix: in `admit`, use the confirm's range (or no range) when the offer came from `accepted`, for example by passing a range down from `confirm`.
- fix size: small
- confidence: confirmed

## The bank's packet gate misses CMSG_OPEN_ITEM
- kind: bug
- severity: low
- where: src/game/map/bank.rs:55-118 (refused); src/game/map/items_misc.rs:183
- what: The bank refuses item packets that touch it while no banker serves the player (a call of 2026-10-19), but `CMSG_OPEN_ITEM` isn't on the list. A clam or an unlocked lockbox in the bank can be opened from anywhere, and its loot goes into the bags.
- fix: add `C::CMSG_OPEN_ITEM(m) => (banked((m.bag_index, m.slot)), item(..))` to `refused`.
- fix size: trivial
- confidence: confirmed

## A group's loot money gives no share to members on another map, though the share is still counted for them
- kind: bug
- severity: low
- where: src/game/map/loot.rs:1478-1520 (take_money)
- what: The share is `gold / allowed.len()`, but only `allowed` members still in `map.players` are paid. One who has zoned out, logged out or gone through a portal since the kill loses their share. mangos's `Loot::SendGold` pays every owner found anywhere (`sObjectMgr.GetPlayer`).
- fix: pay sharers who are off the map through the registry or mail, or divide only among those present (choose one deliberately, and record it in DECISIONS).
- fix size: small
- confidence: confirmed (the divergence from mangos)

## Quest credit still goes to failed quests
- kind: bug
- severity: low
- where: src/game/map/quests.rs:1147-1200 (credit)
- what: `credit` skips a quest only when `!eligible(q) || complete(..)`, and `complete` is false for a failed quest. So kills, uses and casts still raise a failed quest's counters and send `SMSG_QUESTUPDATE_ADD_KILL`. mangos's `KilledMonsterCredit` and `CastedCreatureOrGO` skip any quest whose status isn't INCOMPLETE. Nothing can be turned in, but the client gets misleading updates and the saved counters change.
- fix: also skip `p.journal.log[i].failed`.
- fix size: trivial
- confidence: confirmed

## The pockets and picked sets leak for creatures that despawn without dying
- kind: robustness
- severity: low
- where: src/game/map/loot.rs:290-295, :627-628; src/game/map/professions.rs:1011-1020
- what: `loot_state.picked` and `loot_state.pockets` are cleared only in `creature_died`. A picked creature that despawns, or is removed by a script, pool or event without dying, keeps its entries for the life of the map. So do pockets left by rogues who have logged out.
- fix: clear both when a creature is removed from the map (the same place its other per-creature state is let go).
- fix size: trivial
- confidence: likely

## Picking a creature's pockets while another rogue has them open overwrites that rogue's loot
- kind: bug
- severity: low
- where: src/game/map/professions.rs:1041-1045 (pick_pocket)
- what: `pick_pocket` sets `c.loot = Some(loot)` without checking whether another rogue's pickpocket loot is open on the creature. The first rogue's window now shows loot they can't take (`may_loot` fails), and what they hadn't taken is gone. With `picked_before`, the second rogue gets only quest items, so both of them lose.
- fix: refuse (`FAIL_TARGET_NO_POCKETS` or "in use") while `c.loot` is open pickpocket loot with `looters` not empty, or keep pocket loot per rogue as `pockets` already does.
- fix size: small
- confidence: confirmed

## A tradable item's attach failure is silently ignored at trade completion
- kind: robustness
- severity: low
- where: src/game/map/trade.rs:430-435
- what: `let _ = attach_item(...)` drops any error. Room is planned by `plan_change_freeing`, but `attach_item` places items its own way (one free slot from `carry_slots`, stacks first). If the two ever disagree (a planner change, or a special bag only one of them considers), the detached item is gone with no log. mangos stores through the plan it checked.
- fix: on `Err`, log at `warn` and `mail_back` the row (as `inventory::mail_back` does elsewhere), so an item is never lost.
- fix size: trivial
- confidence: likely (no case where they disagree found today)

## MAX_MONEY is declared three times
- kind: improvement
- severity: low
- where: src/game/map/trade.rs:341, src/game/map/vendor.rs:41, src/game/map/inventory.rs:287
- what: The same constant appears three times, so a change to one won't follow in the others.
- fix: use `inventory::MAX_MONEY` everywhere.
- fix size: trivial
- confidence: confirmed

## The tests reach loot through direct calls that hide routing bugs
- kind: improvement
- severity: low
- where: src/game/tests_p3_items_professions.rs:443, src/game/tests_parity_misc.rs:1228, src/game/tests_p10_player_items.rs:184
- what: The disenchant tests call `loot::take_item` directly, or send release twice, so the bug in the second finding passes them. That goes against AGENTS.md's "tests go through the protocol".
- fix: send `CMSG_AUTOSTORE_LOOT_ITEM`, `CMSG_LOOT_MONEY` and one `CMSG_LOOT_RELEASE` for disenchant, container and pickpocket windows.
- fix size: small
- confidence: confirmed
