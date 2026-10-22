# Re-survey: PvP and social, wvs (master 0c5811f4) vs mangos-classic

Sources: `the first survey (pvp)` (13 findings) and `the first survey (social)` (20 findings), re-checked against current
code. Rounds 12i (battleground shared rules) and 12j (chat and groups) fixed most of them. Of the 33, 22 are fixed,
1 partly fixed, 9 still open, and 1 (weekly decay) is now documented as deliberate. The opcode sweep found nothing
missing in these areas: every PvP and social opcode with a handler in mangos's `Server/Opcodes.cpp` is dispatched in
wvs (`map/pvp.rs::handle`, `map/social.rs::handle`, `map/tickets.rs`, `map/meeting_stones.rs`). The only opcodes
mangos handles that wvs never names are `CMSG_QUESTGIVER_QUEST_AUTOLAUNCH`, `CMSG_SET_TARGET_OBSOLETE`,
`CMSG_STABLE_REVIVE_PET` and `CMSG_WARDEN_DATA`, and none of them is PvP or social. Looking for whole systems that
are absent (honor fields, rank-gated items, `MSG_BATTLEGROUND_PLAYER_POSITIONS`, AV's summons and wing commanders,
AB's 4- and 5-base credit, WSG's flag rules, raid-only instances, raid warning rights, duel areas, ticket survey)
found everything in place. What is left is in the shared battleground framework and in the honor brackets.

## A match's end doesn't freeze it: creatures fight on, the dead stay dead, players keep moving
- status: open
- impact: medium
- who notices: AV players most (NPCs and towers keep killing during the 2-minute wait); every battleground player
  (ghosts wait out the end dead, fights go on)
- mangos: `BattleGround/BattleGround.cpp:EndBattleGround`: every creature that isn't client-controlled gets
  `SetImmuneToNPC/Player(true)` and `SetStunned(true)`. Each player gets Spirit of Redemption removed. The dead are
  resurrected with bones; the living get `CombatStop`, then `CombatStopWithPets` and `BlockMovement`
  (`UNIT_FLAG_CLIENT_CONTROL_LOST`).
- wvs: `src/game/map/battleground/mod.rs:finish` sets `Phase::Ended`, clears timers and the resurrection queue,
  casts the win and loss spells, clears buffs and sends the scoreboard. Nothing touches creatures, the dead or
  combat. `resurrect_wave` (mod.rs:820) stops at `Phase::Ended`, so a ghost stays dead until `battlegrounds::leave`
  sends them home.
- size: S

## Honor's weekly brackets aren't mangos's (fewer players reach the upper standings)
- status: open
- impact: medium
- who notices: every player working on a PvP rank; the size of each rank bracket sets their weekly RP
- mangos: `Tools/Formulas.h:GenerateScores` bracket fractions are 1.000, 0.858, 0.715, 0.587, 0.477, 0.377, 0.287,
  0.207, 0.137, 0.077, 0.037, 0.017, 0.007, 0.002.
- wvs: `src/game/honor.rs` `BRACKETS` are 1.0, 0.845, 0.697, 0.566, 0.436, 0.327, 0.228, 0.159, 0.100, 0.060,
  0.035, 0.020, 0.008, 0.003, used by `weekly_with`. The middle brackets (6 to 10) are smaller by up to 0.05 of the
  standing, so the same standing earns less RP, while the top bracket (R14) is larger (0.3% vs 0.2%).
- size: S
- notes: still undocumented in `docs/DECISIONS.md` (decision 73 covers only the decay of the unranked). Either adopt
  mangos's table or record why the later-vanilla table is kept.

## No start-area enforcement before the gates open
- status: open
- impact: medium
- who notices: battleground players (wall-jumping out of WSG's tunnels or AV's caves before the start)
- mangos: `BattleGround.cpp:Update` (`STATUS_WAIT_JOIN`): every `CHECK_PLAYER_POSITION_INVERVAL`, a player farther
  than the template's `StartMaxDist` from their side's start is teleported back.
- wvs: absent. `src/game/data/load.rs` loads `battleground_template` without a start distance;
  `battleground/mod.rs:update` runs only the countdown in `Phase::Starting`.
- size: S
- notes: if vmangos's `battleground_template` has no such column, mangos-classic's per-BG values can be used as
  constants.

## The return point isn't kept: after a relog or restart in a match, players go to their hearthstone
- status: open
- impact: medium
- who notices: battleground players who disconnect for longer than the 60 s linger, and everyone in a match during a
  restart. The 5-minute offline hold (fixed in 12i) makes rejoining after a drop common, and on return the trip
  home is lost.
- mangos: `Player::SetBattleGroundEntryPoint` at join, saved with the character (`character_battleground_data`);
  `RemovePlayerAtLeave` and a login into a finished match both teleport to it.
- wvs: `src/game/map/player.rs` `Player::bg_return` is in memory only (set in `battlegrounds.rs:port`).
  `battlegrounds.rs:leave` finds `None` after a relog and falls back to the bind point.
- size: S
- notes: a field in the character's JSON state (all fields default), with no migration.

## Accepting an invitation as a ghost, and leaving a match in combat
- status: open
- impact: low
- who notices: battleground players
- mangos: `BattleGroundHandler.cpp:HandleBattlefieldPortOpcode` resurrects a dead player (`ResurrectPlayer(1.0)`,
  bones) before porting. `HandleLeaveBattlefieldOpcode` returns without action while `IsInCombat()` and the match
  is on.
- wvs: `src/game/battlegrounds.rs:port` has no death check, and `battleground/mod.rs:arrived` clears AFK but
  doesn't resurrect. `battlegrounds.rs:leave` (from `map/pvp.rs` `CMSG_LEAVE_BATTLEFIELD`) has no combat check.
- size: S

## Queue feedback: no wait estimate, no invitation reminder, silent group-join refusals
- status: open
- impact: low
- who notices: battleground players; a group leader whose Join as Group is refused sees nothing happen
- mangos: `BattleGroundQueue::GetAverageQueueWaitTime` fills the queued status's estimate; `BgQueueInviteEvent`
  re-sends the invitation part-way through; `SendBattleGroundJoinError` / `BuildGroupJoinedBattlegroundPacket`
  report why a group can't join (offline member, level range, a deserter in the group, ...).
- wvs: `src/game/battlegrounds.rs:send_status` writes 0 for the estimate (`status_frame_at(..., WAIT_QUEUE, 0,
  elapsed)`), with no reminder. `src/game/map/pvp.rs:join_as` answers only `Refused::Deserter`; `NotLeader`,
  `MemberOffline` and level refusals are silent.
- size: S

## Warsong Gulch ignores Call to Arms weekends
- status: open
- impact: low
- who notices: WSG players on its holiday weekend
- mangos: `BattleGroundWS::Reset`: on `IsBgWeekend`, the capture reputation is 45 (not 35), the win bonus is 3
  kills' honor (not 1) and the map-complete bonus is 4 (not 2).
- wvs: `src/game/map/battleground/wsg.rs:area_trigger` always gives 35 reputation, `CAPTURE_HONOR` and
  `WIN_HONOR`. `Match::weekend` is set (mod.rs:527) and read only by `ab.rs` and `av.rs`.
- size: S

## Eastern Plaguelands tower captures give no honor
- status: open
- impact: low
- who notices: world PvP players in EPL
- mangos: `OutdoorPvP/OutdoorPvPEP.cpp` capture credit gives each capturer in range `KilledMonsterCredit` and
  `AddHonorCP(HONOR_REWARD_PLAGUELANDS = 189)` (marked "guessed").
- wvs: `src/game/map/outdoor_pvp.rs:capture_credit` gives only `quests::killed`.
- size: S

## SMSG_PVP_CREDIT shows an unranked victim as Private
- status: open
- impact: low
- who notices: players reading honorable-kill lines
- mangos: `Entities/Player.cpp:AddHonorCP` writes the victim's `GetHonorRankInfo().rank`, which is 0 when unranked.
- wvs: `src/game/honor.rs:player_kill` sends `rank_number(v.rank_points).max(5)`.
- size: S

## The guild info text is gated by the MOTD right, and there is no "modify guild info" right
- status: open
- impact: low
- who notices: guild leaders setting rank permissions
- mangos: `GuildHandler.cpp:HandleGuildChangeInfoTextOpcode` checks `GR_RIGHT_MODIFY_GUILD_INFO`; `GR_RIGHT_ALL`
  includes it.
- wvs: `src/game/social/guilds.rs:set_info` checks `RIGHT_SET_MOTD`; `RIGHT_ALL` is `0xF1FF`.
- size: S
- notes: check the 1.12 rank editor's checkboxes with the client before changing it.

## A meeting-stone player who loses their connection loses their place
- status: partly
- impact: low
- who notices: players queued alone at a meeting stone whose connection drops for more than a minute
- mangos: `LFG/LFGQueue.cpp:RestoreOfflinePlayer` keeps an offline player's entry (`m_offlinePlayers`) and
  restores it at login (`LFGHandler.cpp`).
- wvs: since 12b, a lost connection lingers 60 s before `social::logged_out`, so short drops keep the place. After
  that, `src/game/map/meeting_stones.rs:logged_out` drops the solo entry (`take_solo`). Parties stay queued.
- size: S

# Fixed since

- Three battleground queues at once, each status in its slot, a match taken leaving the other queues
  (`battlegrounds.rs:join_all`, `slots`, `port`; 7bbc8b54).
- Premature finish ([battleground] premature_finish_secs, `battleground/mod.rs` `Match::premature`; 7bbc8b54).
- AFK in a battleground leaves it as a deserter (`map/social.rs:away`, ToggleAFK); arriving clears AFK
  (`battleground/mod.rs:arrived`).
- A logout keeps the place 5 minutes, then Deserter at the next login (`battlegrounds.rs:logged_out`, bg_deserter
  record).
- A raid per side, with /bg and /bg leader, and party and raid chat to the original group (`social/groups.rs:chat`,
  `bg_left`; `map/social.rs:message`).
- General and LocalDefense follow the zone; Trade and GuildRecruitment named for a capital (`social/channels.rs`,
  2470ee91).
- Flood control skips addon lines and /afk, /dnd (6323d88d).
- Groups saved and loaded across a restart (`social/groups.rs` records, 376474fe).
- An offline leader replaced after 300 s; a leaving leader hands on to an online member, a raid's assistant first
  (`groups.rs`, `social::sweep`).
- Loot threshold at least Uncommon (`groups.rs:loot_method`).
- LocalDefense open to all, WorldDefense from honor rank 11 (`channels.rs:say`).
- Player commands (.help, .save, .start, .account password, ...) behind `[gm] player_commands` (d9a0c777).
- Guild invites check the invitee's ignore list (`guilds.rs:invite`).
- Guild roster shows an offline member's days away and zone (`guilds.rs:roster`).
- Party member stats carry the pet and the PvP/FFA flags (04b2791a).
- Cross-faction whisper answered with SMSG_CHAT_WRONG_FACTION (`map/social.rs:whisper`).
- Guild and officer chat in Universal (`guilds.rs:chat`).
- A charter's petition no longer reserves its name (R4-22, 2470ee91).
- GuildRecruitment refuses guild members (`channels.rs:join`).
- No group invite between two copies of one dungeon (fca4d4e0).
- Weekly decay of the unranked: now documented as deliberate, kept as 1.12 (`docs/DECISIONS.md` #73).
- Persistence of custom channels: matches mangos's default (off), so it isn't a gap.
