# Bug hunt: social services and cross-map state (wvs, master at 0c5811f4)

Scope read: `src/game/social/*` (mod, mail, auction, groups, guilds, channels, friends), `src/game/instances.rs`,
`src/game/battlegrounds.rs`, `src/game/registry.rs`, `MapHandle::call`/`call_player` (`map/mod.rs`), the callers in
`map/social.rs`, `map/meeting_stones.rs`, `events.rs`, `weather.rs`. Checked against mangos-classic
(`Mails/MailHandler.cpp`, `Globals/ObjectMgr.cpp`, `AuctionHouse/AuctionHouseHandler.cpp`, `Chat/*`,
`Groups/GroupHandler.cpp`). Known items not repeated: R4-19 (now fixed), R4-21 (fixed: take_item holds the lock),
review 5's finding list, decision 50's "a lingering player's group sees no change until they go".

Lock order re-checked and still sound: groups -> instances -> registry; battlegrounds -> instances;
auctions -> mail; channel -> registry; lists taken before groups/guilds. No reverse pair found; no lock held across
an `.await`, `Maps::get` or `stop`.

## The bidder list compares every auction against every id the client sends, under the realm's auctions lock
- kind: robustness
- severity: high
- where: src/game/social/auction.rs:798-821 (`list_mine`), called from src/game/map/social.rs:565
- what: `CMSG_AUCTION_LIST_BIDDER_ITEMS` carries a client-chosen `u32[amount]` of "outbid" ids. `list_mine` sorts
  and dedups it, then for each auction in the market runs `asked.contains(&a.id)`, a linear scan, with the realm's
  single `auctions` mutex held, on the map's thread. A packet near the 64 KB client frame holds about 16,000 ids;
  against 20,000 auctions that is about 3.2e8 comparisons a packet, roughly a second. Every other map's auction
  list, bid, sell, cancel and the auction sweep wait on that lock for the whole time, and so does the sender's map
  (all its players). A client at an auctioneer can repeat it. mangos looks each asked id up in its map
  (`HandleAuctionListBidderItems`), costing the size of the list times log n.
- fix: `asked` is already sorted: use `asked.binary_search(&a.id).is_ok()`. Better still, cap `outbid` (mangos
  ties it to the packet size; 1.12's client sends at most the auctions it has seen) and look the asked ids up
  directly (`auctions.auctions.get(id)`) rather than scanning the market.
- fix size: trivial
- confidence: confirmed

## Built-in channel names with any suffix make channels that are never removed: unbounded growth, and every zone change and logout scans them all
- kind: robustness
- severity: medium
- where: src/game/social/channels.rs:99-108 (`built_in_row`), :175 (`gone`), :364-373 (`Channels::of`), :469
  (`leave_all`), :486 (`zone_changed`); join from src/game/map/social.rs:358
- what: `built_in_row` matches any name starting "General - ", "Trade - ", "LocalDefense - " and the rest. Such a
  channel gets the DBC id, so `constant()` is true and `gone()` is never true: once made, it stays in the directory
  forever, empty. A client sending `CMSG_JOIN_CHANNEL` with "General - 1", "General - 2" and so on adds one entry
  per packet, for good, with no cap (also no per-player cap on custom channels joined; the client's own limit is
  10). `Channels::of` locks every channel in the directory, and it runs on the map's thread at every zone change
  (`zone_changed`) and at every logout (`leave_all`), so the cost of each grows with the junk. mangos-classic has
  the same leak (`ChannelMgr::LeftChannel` keeps constant channels), but it doesn't scan all channels per zone
  change.
- fix: (1) let an empty built-in channel go too: it keeps no password or bans worth keeping, and the next join
  remakes it. Or accept a built-in name only with a real area's name, or "City", after the prefix. (2) Keep each
  player's channels on the player (or in an index by GUID) so `of` doesn't lock the whole directory. (3) Cap
  channels a player is in (10, the client's).
- fix size: small
- confidence: confirmed

## Any player can read any mail's body by id
- kind: bug
- severity: medium
- where: src/game/social/mail.rs:814-828 (`text_query`); `item_text_id` is the mail's id (`list`, :372)
- what: `CMSG_ITEM_TEXT_QUERY` answers `mails.texts[id]`, else `mails.mails[id].body`, with no check that the asker
  is the mail's receiver. Mail ids are realm-wide and sequential (`Mails::next_id`), so a client walking ids 1..N
  reads every letter in the realm, and every auction invoice (the other party's GUID, the bid, the buyout, the
  deposit and the cut: `auction::invoice`). Letters' texts (`KIND_TEXT`) share the id space, so they leak too.
  mangos-classic has the same hole, with a `TODO: some check needed` in `HandleItemTextQuery`; the
  `session::is_query` throttling doesn't help, since the ids are cheap to walk.
- fix: answer a mail's body only to its receiver (`m.receiver == counter(g)`, or the sender for a returned one);
  answer a letter's text only to one carrying an item whose data is `text:{id}` (or keep a text's owner).
  Otherwise an empty text.
- fix size: small
- confidence: confirmed

## A cash-on-delivery payment that expires goes back to the buyer, who keeps the item too
- kind: bug
- severity: medium
- where: src/game/social/mail.rs:879-891 (`sweep`), :644-659 (`take_item` posts the payment with `COD_PAYMENT`)
- what: the payment mail is `kind: NORMAL` with `flags: COD_PAYMENT` and `money: cod`. When it expires untaken (30
  days), the sweep's test `m.kind == NORMAL && m.flags & RETURNED == 0 && keeps` sends it back to its sender, the
  buyer. The buyer then has the item and the money; the seller loses both. mangos-classic deletes it instead
  (`ObjectMgr::ReturnOrDeleteOldMails`: "if it is mail from non-player, or if it's already return mail, it
  shouldn't be returned, but deleted", testing `MAIL_CHECK_MASK_COD_PAYMENT | MAIL_CHECK_MASK_RETURNED`). The same
  hole reaches a payment to a deleted seller: it sits in an orphan mailbox for 30 days and then goes back to the
  buyer.
- fix: also leave out `COD_PAYMENT` in the sweep's send-back test (`m.flags & (RETURNED | COD_PAYMENT) == 0`). Do
  the same in `return_to_sender` if the client can return one.
- fix size: trivial
- confidence: confirmed

## The mail sweep's expiry acts on a mail it doesn't check again: a mail returned meanwhile is deleted, item and all
- kind: bug
- severity: medium
- where: src/game/social/mail.rs:864-891 (`sweep`)
- what: the sweep collects the expired ids under the mail lock and lets it go. It sends `SMSG_RECEIVED_MAIL` to each
  due receiver, then takes the lock again for each id, and acts on whatever mail now has that id, without testing
  `expires_at <= now` again. Suppose that between the two, the receiver sends `CMSG_MAIL_RETURN_TO_SENDER` for a
  mail with an item or money that is expiring. `send_back` gives it a new receiver, `RETURNED` and 30 more days.
  The sweep then finds `RETURNED` set and takes the delete branch: the item or money is destroyed and its record
  deleted. The window is short, but the sweep runs every five seconds, and the client shows the expiry time, so a
  player returning a mail that is expiring is the natural case.
- fix: under the second lock, `continue` unless `m.expires_at <= now` (or do the expiry in the first lock, which
  already holds the set).
- fix size: trivial
- confidence: confirmed

## The group's roster at a logout shows the leaving member as online
- kind: bug
- severity: medium
- where: src/game/map/social.rs:598-616 (`went_offline` calls `logged_out` before `registry.go_offline`);
  src/game/social/groups.rs:1653-1683 (`logged_out`), :319-343 (`send_update`, `list_for`'s `is_online`)
- what: `groups::logged_out` sends every member `SMSG_GROUP_LIST`, with `is_online` computed as
  `realm.registry.online(m.guid).is_some()`. The leaving player is still in the registry at that point, because
  `go_offline` comes after. So the roster tells everyone the member is online. Only the
  `SMSG_PARTY_MEMBER_STATS` that follows (status 0) says otherwise. The raid UI's online flag
  (`GetRaidRosterInfo`) reads the roster, so a raid shows the departed member as connected until the next roster
  change. The bg-raid branch at :1678-1682 does the same. The leaver is also sent a roster and a
  `set_leader_flag` map call they don't need. No test checks the roster's online flag at logout.
- fix: take the player out of the registry first (or pass `g` to `send_update`/`list_for` as offline). Have
  `send_update` skip `g` itself.
- fix size: small
- confidence: confirmed (by reading; the client side follows mangos, which sends the update after the player is
  gone)

## A player in a battleground who joins or forms a group has their battleground raid roster overwritten
- kind: bug
- severity: low
- where: src/game/social/groups.rs:1009-1014 (`invite` makes a group for an inviter with none),
  :1072 (`accept`'s `set_home`), :714-720 (`set_home`), :319-329 (`send_update` skips only `away`)
- what: `send_update` keeps a member in a battleground's raid off their own group's roster only through
  `Group::away`, and only `bg_joined` adds to `away`. A player already in a battleground can still get an own
  group. They can invite someone (`invite` creates the group and `set_home` puts it in `original`), or accept an
  invite (`home` is `None` for one with no own group, so the invite is allowed; `accept` puts them in `original`).
  Neither adds them to `away`, so each roster change of the new group sends them that group's `SMSG_GROUP_LIST`,
  replacing their battleground raid's on the client. As leader, they also can't manage the group: `uninvite`,
  `loot_method`, `set_leader`, `convert_to_raid` and the subgroup commands look them up through `of` (the bg raid)
  or `mine()`, and are refused. mangos's `SendUpdate` skips any member whose `GetGroup() != this`, so it has
  neither problem.
- fix: in `set_home`, when `in_bg_raid(g)`, also push `g` to the group's `away`; or have `send_update` take the
  `of` map and skip members whose `of` isn't this group. Have the leader-only commands use `home()`.
- fix size: small
- confidence: confirmed

## `join_all` checks "already queued" under one lock and queues under another
- kind: bug
- severity: low
- where: src/game/battlegrounds.rs:471-482 (the check) and :505-544 (the queue or invite)
- what: review 5 noted this ("smaller, noted"), and it is still open. Take a player whose own join and whose
  leader's join for the group are handled on two maps at once (the leader elsewhere). Both pass the check before
  either queues, so the player is queued twice, `take_slot` gives them two slots for one battleground, and `fill`
  can invite them to two matches. The direct-instance path (`instance > 0`) races the same way.
- fix: do the checks and the queueing under one `battlegrounds` lock (the checks between them read only `who`).
- fix size: small
- confidence: confirmed (by reading; not reproduced)

## The owner list of a seller with many auctions is not sent at all
- kind: bug
- severity: low
- where: src/game/social/auction.rs:798-837 (`list_mine`, unpaged); src/game/conn.rs:28-36 (`encode` refuses a
  body past `MAX_FRAME_BODY`)
- what: review 5's finding 1 asked for the owner list to be paged. `encode` now refuses an oversized frame rather
  than wrapping its size, but the list is still unpaged and `sell` has no per-player cap. A seller with more than
  about a thousand auctions (or a bidder asking many ids) now gets nothing: `p.outbound.send` drops it with a
  `warn!`. Their "Auctions" tab stays empty.
- fix: stop at what fits (as `mail::list` does with `LIST_BYTES`), keeping `total_amount_of_auctions` the true
  count; or cap auctions per character.
- fix size: small
- confidence: confirmed

## The instance sweep tests every save against every online player, under the instances lock
- kind: perf
- severity: low
- where: src/game/instances.rs:1132-1158 (`sweep`'s `stale`), :519-525 (`admit_locked`), :552-570 (`in_use`)
- what: every five seconds, `stale` runs `online.iter().any(|o| o.map().key.instance == s.id)` for each save not a
  raid's, which is saves times online. Each `o.map()` locks the entry's mutex and clones a sender (atomics). With
  1,000 saves and 3,000 online that is 3 million mutex lock-and-clone operations, all under `instances`, which
  every dungeon entry and every group change (`group_changed`, under the groups lock) waits on. `admit_locked` and
  `in_use` also walk and allocate the whole online list on every entry into a copy.
- fix: build the set of occupied instance ids once (`AHashSet<u32>` from one `all_online()` pass) before the
  filter. Better: keep a count of players inside each copy, updated by `arrived` and a map's leave.
- fix size: trivial (the set) / small (the counts)
- confidence: confirmed

## A friend's login or logout scans every online player's friend list
- kind: perf
- severity: low
- where: src/game/social/friends.rs:118-125 (`Lists::watchers`), called by `logged_in` (:241), `told_offline`
  (:270), `logged_out` (:278); `forget` (:105) the same
- what: `watchers(g)` walks `friends`, which holds one set per online player, under the realm's `lists` lock. That
  is the lock every whisper, channel, party and guild line and every invite takes (`not_ignoring`, `ignores`). A
  login storm of N players costs N² set lookups under it, and so does a restart-reconnect or a load test's ramp.
  `ignores` already has its reverse index (`ignored_by`).
- fix: keep `friended_by: AHashMap<Guid, AHashSet<Guid>>` alongside `friends`, as `ignored_by` is kept beside
  `ignores`.
- fix size: small
- confidence: confirmed

## Every roster change does members-squared registry reads, and a map call per member, under the groups lock
- kind: perf
- severity: low
- where: src/game/social/groups.rs:319-343 (`send_update`, `list_for`), :386-396 (`set_leader_flag`)
- what: for each member online, `send_update` builds that member's list, calling `registry.online` for every other
  member (40 x 40 = 1,600 read-locks and `Arc` clones for a raid). It also sends a `call_player` closure to every
  member's map to set the leader flag and mark them visibility-dirty, even when the flag hasn't changed. All of it
  runs under `groups`, which maps take in their hot paths (`spells/targets.rs`, `combat.rs`, `loot.rs`, `tap.rs`,
  member stats). A raid's logins, logouts, subgroup drags and loot-rule changes each pay it.
- fix: look the members' online state up once a `send_update` (one `each_online` pass into a bitset), and build the
  frames after letting the lock go. Send the leader-flag call only to the old and new leader when the lead changes,
  and to a member when they join or leave.
- fix size: small
- confidence: confirmed

## Party member stats are given for any player, to anyone
- kind: improvement
- severity: low
- where: src/game/social/groups.rs:1829-1836 (`request_stats`)
- what: `CMSG_REQUEST_PARTY_MEMBER_STATS` answers with the full stats (health, power, zone, x/y position, auras) of
  any grouped player whose GUID is asked, including an enemy's, without checking that the asker shares a group.
  A client can track any grouped player's zone and position across the world. mangos-classic has the same lack of
  a check, so this is parity, not a regression.
- fix: answer with the full stats only when `others(me)` contains `of`; otherwise the offline status packet.
- fix size: trivial
- confidence: confirmed

## Money taken from mail past the cap is lost silently
- kind: improvement
- severity: low
- where: src/game/social/mail.rs:566-577 (`take_money`); src/game/map/inventory.rs:346-349 (`set_money` clamps)
- what: `take_money` zeroes the mail's money and sets the player's money to `have.saturating_add(money)`, which
  `set_money` then clamps at `MAX_MONEY`. A player near the 214k-gold cap who takes a large payment (an auction's
  takings, a refund) loses the excess, and the mail is saved empty. mangos-classic loses it the same way, but
  vendors (`vendor.rs:482`) and trades (`trade.rs:406`) in wvs refuse instead.
- fix: if `have + money > MAX_MONEY`, take only what fits and leave the rest in the mail, or refuse (as the
  vendor does).
- fix size: trivial
- confidence: confirmed

## Groups whose members are all gone for good are kept and swept forever
- kind: robustness
- severity: low
- where: src/game/social/groups.rs:1272-1294 (`replace_offline_leaders`), :619-644 (`load`)
- what: a group record is deleted only when its roster drops to one. A group whose members never come back stays
  in the store and in memory. Every five seconds the sweep checks its leader in the registry, and once the delay
  has passed it runs `next_leader`, which does a registry read per member, all under the groups lock. That cost
  grows with every group ever formed and abandoned. Since R4-19's fix kept groups across restarts, nothing ever
  removes them.
- fix: skip groups with no member online (keep a count of online members, or test `leader_away_since` once and
  stop re-running `next_leader` when no one is online). Optionally, expire a group none of whose members has logged
  in for N days.
- fix size: small
- confidence: confirmed (by reading)

## A failed bg-raid join leaves the player marked away from their own group
- kind: bug
- severity: low
- where: src/game/social/groups.rs:1709-1741 (`bg_joined`)
- what: `bg_joined` records the player's own group in `original` and pushes them into `own.away` before it checks
  that the side's raid exists and has room. Two cases return after that: `gr.full()`, and the raid gone while the
  lock was dropped for `bg_left` (:1711-1714). In both, `of[g]` still points at their own group, so `in_bg_raid`
  is false, but `own.away` contains them. They then get no roster updates from their own group: `send_update`
  skips `away`. `bg_left` returns early (not in a bg raid), so nothing clears it.
- fix: check the raid and its room first, and only then move `of` into `original` and push `away`; or undo both on
  the early returns.
- fix size: trivial
- confidence: likely (the full raid needs a side over 40, which `fill`'s counts should prevent; the dropped-lock
  case is a narrow race)

## A kick from one's own group while in a battleground sends `SMSG_GROUP_UNINVITE` to the battleground client
- kind: bug
- severity: low
- where: src/game/social/groups.rs:769-781 (`remove`, `kicked` with `away`)
- what: for a member in a battleground's raid (`away`), `remove` rightly leaves their roster alone (it is the
  raid's), but still sends `SMSG_GROUP_UNINVITE` when they are kicked. The client takes it as being put out of its
  current group, the battleground raid, and clears its raid UI until the next bg raid update.
- fix: send `SMSG_GROUP_UNINVITE` only when not `away`, or send the bg raid's roster again after it.
- fix size: trivial
- confidence: likely (the client's handling of `SMSG_GROUP_UNINVITE` is inferred, not checked with the client
  check)

## Stats kept for a player who just left their group
- kind: improvement
- severity: low
- where: src/game/map/social.rs:744-752 (`push_member_stats`'s second lock); src/game/social/groups.rs:778
  (`remove` clears `stats`)
- what: `push_member_stats` reads who is grouped under one lock, then writes `stats`/`seen_by` under a second.
  A `remove` (a kick or leave from another map) between the two has its `stats.remove(g)` undone, so an ungrouped
  player keeps a stats entry until they log out or join again. `request_stats` then serves that stale entry (see
  the finding above). The leak is small, one entry per player.
- fix: in the second lock, insert only for players still `group_of(g).is_some()`.
- fix size: trivial
- confidence: confirmed

## Dead code and misleading names in the auction paths
- kind: improvement
- severity: low
- where: src/game/social/auction.rs:557 and :594 (`let id = a.id; ... let _ = id;`), :467 and :588 (`fail(...)`
  used to send `R::Ok`); src/game/social/mail.rs:23-24 and :1 (`POSTAGE` comment says mangos-zero, and the module
  doc says "as mangos-zero runs it" while the rules now follow mangos-classic)
- what: `bid` computes `id` only to discard it, and reports success through a closure named `fail`. Both mislead a
  reader of the money path.
- fix: drop `id`; rename `fail` to `answer`.
- fix size: trivial
- confidence: confirmed

## Bids and cancels are taken on an auction that has expired but not been swept
- kind: bug
- severity: low
- where: src/game/social/auction.rs:475-484 (`bid`), :606-614 (`cancel`); `sweep` runs every 5 s
- what: `list` hides auctions with `expires_at <= now`, but `bid` and `cancel` don't test it. For up to five
  seconds after expiry, a bid or a buyout is accepted (the bidder wins an auction that ended unsold), and a seller
  can cancel one with no bid. Neither dupes anything, but it changes outcomes that should have been settled by the
  clock. A client sees the auction's time left and can aim for the window.
- fix: in `bid` and `cancel`, treat `a.expires_at <= mail::now()` as not found.
- fix size: trivial
- confidence: confirmed
