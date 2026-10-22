# Re-survey: world, protocol, the player and characters (wvs master 0c5811f4 against mangos-classic)

Sources re-checked: `the first survey (world_protocol)` (15 findings) and `the first survey (player)` (30 findings).
Round 12 (merges 12a, 12b, 12h, 12j, 12k, 12i) fixed most of them. 29 of the 45 are now fixed (listed at the
end). This file lists what is still open or partly fixed, and what was never implemented.

**Opcode table, compared again.** mangos-classic's `Server/Opcodes.cpp` gives a real handler to 298 client
opcodes. wvs's non-test sources name 294 of them. The 4 it doesn't name are `CMSG_QUESTGIVER_QUEST_AUTOLAUNCH`,
`CMSG_SET_TARGET_OBSOLETE` and `CMSG_STABLE_REVIVE_PET`, which do nothing in mangos either, and `CMSG_WARDEN_DATA`.
`CMSG_CHAR_RENAME` is handled now. The no-op arm in `map/handlers.rs` (`CMSG_REQUEST_ACCOUNT_DATA`,
`CMSG_MEETINGSTONE_INFO`, `CMSG_GMTICKET_*`, `CMSG_BATTLEFIELD_STATUS`, `CMSG_COMPLETE_CINEMATIC`) is only a
fallback: each of these is handled earlier in the chain (`account_data.rs`, `meeting_stones.rs`, `tickets.rs`,
`pvp.rs`, `misc.rs`). Only `CMSG_BUG` is really dropped.

I also compared the server messages mangos-classic sends with the ones wvs names. In these areas, wvs never sends
`SMSG_SERVER_MESSAGE`, `SMSG_RAID_INSTANCE_MESSAGE`, `SMSG_ADDON_INFO`, `SMSG_WARDEN_DATA`,
`SMSG_PAUSE_MIRROR_TIMER` or `SMSG_CHAT_RESTRICTED`. These are covered by the entries below.

---

## No player limit and no login queue
- status: never implemented
- impact: medium (high on a busy realm)
- who notices: everyone on a popular realm at launch or at peak: past what the hardware can carry, every map gets
  slow for everyone, instead of new players waiting in a queue
- mangos: `World::AddSession` with `PlayerLimit` (World.cpp:399), and `AddQueuedSession`: `SMSG_AUTH_RESPONSE`
  `AUTH_WAIT_QUEUE` with the player's place in the queue. GMs skip it.
- wvs: absent. `session.rs` sends only `AuthOk`, `AuthFailed` or `AuthBanned`. The only bound is
  `[network] max_pending_connections`, on handshakes. SITREP lists `PlayerLimit` as "left for you".
- size: M

## No shutdown or restart countdown
- status: open
- impact: medium
- who notices: every player online at a restart: raids mid-boss, players mid-trade or mid-auction
- mangos: `World::ShutdownServ` / `ShutdownMsg` sends `SMSG_SERVER_MESSAGE` (`SERVER_MSG_SHUTDOWN_TIME`,
  `RESTART_TIME`) at intervals, which the client shows as a countdown. GMs have `.server shutdown|restart <secs>`
  and `cancel`.
- wvs: `game/mod.rs` `shutdown_signal` goes straight to `Realm::shutdown` / `shutdown_within`. Nothing ever sends
  `SMSG_SERVER_MESSAGE`. The player commands added in 12j have `.server info|motd`, but nothing that shuts down.
- size: S

## A battleground's return point isn't saved: a relog or a restart sends the player to their hearthstone
- status: open
- impact: medium
- who notices: battleground players who disconnect past the 60 s linger, or are in a match at a restart. They land
  in their inn, not at the battlemaster where they queued.
- mangos: `Player::LoadFromDB` (~14205-14255) with `_LoadBGData` / `character_battleground_data`. When the match is
  gone, the player goes back to `GetBattleGroundEntryPoint()`.
- wvs: `map/player.rs` `Player::bg_return` is only in memory (set in `battlegrounds.rs` ~767). `session.rs`
  `Session::enter` (~1191-1206): when `battlegrounds::rejoin` fails, the player goes to `row.state.bind`.
- size: S (put it in `CharacterState`, a JSON field with a default, so no migration)

## Logging in to a copy that's gone puts you at your hearthstone, not the instance's entrance
- status: open
- impact: medium
- who notices: dungeon and raid players who log out inside and come back after the copy has expired or reset. A
  player who logged out deep in Blackrock Mountain wakes up in Ironforge or Orgrimmar.
- mangos: `Player::LoadFromDB` (~14346). When the instance's state is gone, or the player has been away more than
  15 minutes, they are moved to `GetMapEntranceTrigger(mapId)`.
- wvs: `session.rs` `Session::enter` (~1194-1205): when `instances::resume` returns `None`, the player goes to
  `row.state.bind`. 12a made this rarer, because a copy that loses its owner at a restart is now held for its
  members, but an expired copy still sends them home. The 15-minute rule is missing too.
- size: S (wvs already loads `areatrigger_teleport` and knows each map's way in)

## No AFK logout after 15 minutes
- status: partly (going AFK in a battleground now removes you; the AFK logout is still missing)
- impact: low-medium
- who notices: a busy realm's population. This matters more alongside the missing player limit: idle characters
  hold world capacity, and on a capped realm they would hold queue slots.
- mangos: `WorldSession::Update` → `ShouldAfkDisconnect` (`m_afkTime + 15 * MINUTE`) → `LogoutPlayer`.
- wvs: `map/social.rs` `away`. 12j added only the battleground rule (`ToggleAFK`, ~175). Nothing logs out an AFK
  player. The 120 s idle timeout (`[network] client_idle_timeout_secs`) catches only a silent socket, and an AFK
  1.12 client keeps pinging.
- size: S

## Raid copies reset with no warning to the players inside
- status: open
- impact: low-medium
- who notices: raids still in MC, BWL, ZG, AQ or Naxx at reset time
- mangos: `MapPersistentStateManager::Update` schedules `RESET_EVENT_INFORM_1..LAST` (60/15/5/1 min), and
  `DungeonMap::SendResetWarnings` → `Player::SendInstanceResetWarning` sends `SMSG_RAID_INSTANCE_MESSAGE`.
- wvs: `instances.rs` `sweep` → `expire` resets at `reset_at` with no warning first. `SMSG_RAID_INSTANCE_MESSAGE`
  appears nowhere in `src/`.
- size: S

## The realm's clocks disagree: events and the instance reset on UTC, the game clock and weather on local time
- status: open
- impact: low-medium
- who notices: players on any realm whose host isn't on UTC. Holidays and fireworks start at odd local hours, and
  the raid reset hour isn't the local hour an operator would expect.
- mangos: everything is local time: `game_event.start_time` (`UNIX_TIMESTAMP`), the game clock, the weather season
  and `Instance.ResetTimeHour`.
- wvs: UTC in `data/load.rs` ~1570 (`and_utc`), `events.rs` ~96 (`DateTime::from_timestamp`), `mod.rs` ~1680
  (`events::active(.., Utc::now())`), `instances.rs` `RESET_EPOCH` / `next_reset_at` and `honor.rs` ~107. Local
  time in `map/login.rs` ~302 (the game time), `weather.rs` ~203, `social/guilds.rs` ~267 and `map/conditions.rs`
  ~564. Only the weather's choice of local time is recorded (THISTLE_TEA_DIFFERENCES:56).
- size: S

## The world's event state is lost at a restart (the Scourge Invasion, the elemental invasion, GM-started events, world states)
- status: open
- impact: low
- who notices: players working towards the Argent Dawn's invasion milestones after a GM has turned the invasion on;
  everyone in a GM-run event
- mangos: `WorldState::Save` / `Load` (`world_state`: `SAVE_ID_SCOURGE_INVASION` and the others) and
  `GameEventMgr`'s `game_event_status`.
- wvs: `social/mod.rs` `Social::{world_states, driven_events, gm_events, invasion, elemental}` are mutexes in
  memory. No record kind exists for them (the kinds are mail, auction, guild, petition, group, ticket, raid,
  instance, arena and the respawns), and `Social::load` restores none of them.
- size: S

## Logging out is allowed during a duel
- status: open
- impact: low
- who notices: duellers, mostly during the countdown. A logout ends a duel the player is losing.
- mangos: `HandleLogoutRequestOpcode` refuses during combat, a duel (`GetPlayer()->duel`) or a fall.
- wvs: `map/login.rs` `request_logout` (~347-359) checks combat, airborne and GM freeze. `duel.rs` sets no combat
  flag, and nothing checks the duel.
- size: S

## Quests flagged "stay alive" don't fail on death
- status: open
- impact: low
- who notices: players on the handful of `QUEST_FLAGS_STAY_ALIVE` quests (647 "MacKreel's Moonshine", 1651, 1719
  "The Affray", 7622 and others)
- mangos: `Player::FailQuestsOnDeath`, called from `BuildPlayerRepop`.
- wvs: absent. Nothing on death fails quests. `quests::fail` is called only by escorts and scripts.
- size: S

## No Warden (client integrity checks)
- status: never implemented
- impact: low (it grows with the population)
- who notices: honest players on a realm that draws bots and Lua unlockers. The movement checks hardened in 12c are
  the only defence.
- mangos: `Anticheat/module/Warden`, `HandleWardenDataOpcode`; vmangos's `warden_scans` is in the data.
- wvs: absent. wvs never sends `SMSG_WARDEN_DATA`, and `CMSG_WARDEN_DATA` is never named outside the packet guards.
- size: XL

## Addon info isn't answered at login (`SMSG_ADDON_INFO`)
- status: never implemented
- impact: low
- who notices: probably no one. The 1.12 client loads addons without the answer. Listed because mangos-classic
  sends it (it is the packet that marks Blizzard's own addons as signed, and the hook its addon allow and ban
  lists use).
- mangos: `Addons/AddonHandler.cpp` `BuildAddonPacket`, sent from the auth session.
- wvs: absent. `session.rs`'s auth answers `SMSG_AUTH_RESPONSE` and nothing else.
- size: S

## The character screen never sends the guild
- status: never implemented
- impact: low
- who notices: players at the character list (the enum's guild id is always 0)
- mangos: `Player::BuildEnumData` writes `guild_member.guildid`.
- wvs: `session.rs` `character_entry` writes `guild_id: 0`.
- size: S

## Reserved names: a config list only, not vmangos's `reserved_name`
- status: partly (`[world] reserved_names`, checked in `session.rs` `valid_name`; empty by default)
- impact: low
- who notices: anyone who takes "Thrall" or another lore name on a realm whose operator didn't fill in the list
- mangos: `ObjectMgr::IsReservedName`, from the `reserved_name` table.
- wvs: the vmangos table (9 rows) isn't loaded. Rename (12k) uses the same `valid_name`.
- size: S

## `CMSG_BUG` (the client's bug and suggestion form) is dropped
- status: open
- impact: low
- who notices: players who send reports; the operators who never see them
- mangos: `HandleBugOpcode` inserts into `bugreport`.
- wvs: an empty arm in `map/handlers.rs` (~369).
- size: S

## Each creature death scans every spawn for its respawn time
- status: open
- impact: low
- who notices: no one directly. It is CPU on the map's thread during AoE farming and big pulls.
- mangos: a hash lookup by DB guid (`GetCreatureData`).
- wvs: `map/ai.rs` `respawn_delay` (~433): `creature_spawns.values().flatten().find(|s| s.guid == g)` over every
  spawn.
- size: S

## Free profession slots aren't shown to the client (`PLAYER_CHARACTER_POINTS2`)
- status: open
- impact: low
- who notices: players at a profession trainer, who see no "professions remaining" (the server still enforces the
  limit of two)
- mangos: `Player::SetFreePrimaryProfessions`.
- wvs: `fields.rs` defines `PLAYER_CHARACTER_POINTS2`, and nothing outside tests writes it.
- size: S

## Damaged items sell for less than their price
- status: open (Thistle Tea's rule; not recorded in DECISIONS or THISTLE_TEA_DIFFERENCES)
- impact: low
- who notices: everyone who sells worn gear
- mangos: `HandleSellItemOpcode` pays `SellPrice * count`, whatever the durability.
- wvs: `map/vendor.rs` `sale_price` (~414-420) takes the repair cost off.
- size: S

## Rage decay and out-of-combat health regen differ from mangos-classic
- status: open (the references disagree: vmangos and Thistle Tea match wvs)
- impact: low
- who notices: warriors and bear druids; everyone resting between pulls
- mangos: `Player::Regenerate(POWER_RAGE)` decays 25 every 2 s; `RegenerateHealth` adds twice the spirit formula.
- wvs: `map/stats.rs` `regen_player` decays 20 (~193) and adds `health_regen` once (~124).
- size: S (settle it against 1.12 data first)

## Ghost and wisp speed are hard-coded
- status: open
- impact: low
- who notices: night elves, and ghosts whose speed is recalculated by an aura change
- mangos: `BuildPlayerRepop` casts 8326 and 20584, and the speed comes from their auras.
- wvs: `map/death.rs` `GHOST_SPEED` 1.25 and `WISP_SPEED` 1.5 (~31-32), set by hand in `become_ghost`.
- size: S

## Smaller mismatches still open
- status: open
- impact: low
- who notices: the XP display, the level-up text, drinkers, players editing an opened container
- mangos: `SendLogXPGain` sends the real `groupRate`. `GiveLevel`'s `SMSG_LEVELUP_INFO` sends the base-stat
  deltas. `LoadFromDB` sobers a character by the time they were logged out. `CanEquipItem` and `CanUnequipItem`
  refuse an item whose loot window is open (`EQUIP_ERR_ALREADY_LOOTED`).
- wvs: `map/stats.rs` `xp_gain` writes `exp_group_bonus: 1.0` (~216). `set_level` (~301) sends max health and mana
  changes that include gear. `map/intoxication.rs` `restore` keeps the saved drunkenness. `map/inventory.rs`
  checks open loot only when merging stacks (`mergeable`), not when moving or equipping.
- size: S

---

## Fixed since
- Groups lost at a restart: `social/groups.rs` (`KIND_GROUP` records, loaded at start-up), 12j.
- Players lose their copy at a restart: `instances.rs` `belongs` / `key_for` hold a copy left without an owner for
  its members, 12a.
- Open-world respawn times not saved: `respawns.rs` (`creature_respawn`, `object_respawn` records), plus a copy's
  game objects, 12a.
- A disconnect removes the character at once: `map/mod.rs` `start_linger` / `disconnect_linger` (60 s, attackable,
  taken back by a login), 12b.
- An empty copy resets after 5 minutes: `[instances] unload_delay_secs` defaults to 1800 (`UNLOAD_EMPTY_AFTER`), 12a.
- Addon messages count towards flood control: `map/handlers.rs` (~276-283) skips `flooded` for `chat::ADDON` and
  AFK/DND, 12j.
- `CMSG_CHAR_RENAME`: `session.rs` `rename`, `AT_LOGIN_RENAME`, `CharacterFlags::RENAME`, `.rename`, 12k.
- A dead logout revives the player: 12b releases before the save (`LogoutPlayer`'s repop); the test that locked in
  rising at login was changed.
- Poisons skill never rises: `map/professions.rs` `SKILL_POISONS` raised by its recipes, 12h.
- A bag with items in it can't be moved: `map/inventory.rs` bag exchange (`ItemsCantBeSwapped`,
  `NONEMPTY_BAG_OVER_OTHER_BAG`), 12h.
- Mining and skinning halved every 75: `[rates] skill_chance_mining_steps` / `_skinning_steps` default 0
  (`professions.rs` ~425), 12h.
- Re-rolling a node's skill-up: one roll per spawn, none once its loot exists, 12h.
- Grey level at 60 is 51: `map/combat.rs` `gray_level` uses `IsTrivialLevelDifference` (47), 12h.
- Kill rewards need 74 yards in dungeons: `map/combat.rs` (~3642) skips the distance in a dungeon or raid copy, 12h.
- Fishing skill-ups: `professions.rs` `fishing_steps_needed` / `fishing_steps`, escapes count too, 12h.
- No 6-minute auto-release: `map/death.rs` `AUTO_RELEASE` and `PLAYER_FIELD_BYTE_RELEASE_TIMER`, 12h.
- The spirit healer revives in place: `map/death.rs` (~750) moves the player to the corpse's graveyard, 12h.
- Falling below the world: `map/death.rs` `VOID_Z` -500, a fall death and release, 12h.
- AFK in a battleground: `map/social.rs` (~175) makes the player leave as a deserter, 12j (the 15-minute AFK logout
  is still open, above).
- Gear changed while casting, stunned, dead or logging out: `map/inventory.rs` (`YouAreStunned`, ~1607-1618), 12h.
- A mounted player can melee: `map/combat.rs` (~1793), as `Unit::Attack`, 12h.
- Splitting onto a matching stack: `map/inventory.rs` `split` (~1893), 12h.
- No PvP rank 3 discount: `map/reputation.rs` (~707-714), 12k.
- Crafted items never show "Made by": `map/inventory.rs` (~1271) sets `ITEM_FIELD_CREATOR` and saves it, 12h.
- Weapon skills don't train on grey creatures: `map/combat.rs` `skill_gains` has no grey check now (critters,
  `NO_SKILL_GAINS` and player-controlled targets only), 12g.
- Repair-all is all or nothing: `map/vendor.rs` repairs item by item while the money lasts, 12k.
- Starter nodes go grey 4 points early: a need of 1 counts as 5, 12h.
- Rename at login: part of the "smaller mismatches" entry, fixed with `CMSG_CHAR_RENAME`, 12k.
- Combat-logging bypasses the logout rules: covered by the linger, 12b.
