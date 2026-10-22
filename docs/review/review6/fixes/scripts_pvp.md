# Bug-hunt fixes: scripts, instances and battlegrounds (scripts_pvp)

Branch `fix/bughunt-scripts_pvp` (worktree `its worktree `), from master `0c5811f4`. Not merged or pushed.
Fast suite (3,413) and `cargo extended` (4,180) pass, and `cargo clippy --all-targets` is clean. The client check
wasn't run: no packet layout or create changed (only field values and existing frames).

## Findings in `hunt/scripts_pvp.md`

- Stratholme: Ramstein gone before he dies locks the Baron for the copy's life: **fixed**, `c0776639`. Ramstein is now a dead-despawn summon (mangos-classic's `TEMPSPAWN_DEAD_DESPAWN`), and `strat_settle` puts him back at his post when he was called but not beaten. Test: `tests_p9_core_scripts::ramstein_called_but_not_beaten_is_back_when_the_copy_loads_again`.
- BWL: a wipe in Nefarian's first stage leaves Lord Victor Nefarius hostile at his table: **fixed**, `e716b38b`. A new `at_table` gives him faction 35 as his own faction (not a temporary one), immune to players, combat movement back, gossip; it runs at Spawned and at the FAILED cleanup. Test: `tests_p12l_nefarian::nefarians_lair_lost_is_put_back` (extended) now checks the faction, the flag and the menu.
- BWL: Vaelastrasz can never be started again once a failed try is saved: **fixed**, `8d3de2f9`. `bwl::demote` resets his FAILED state to NOT_STARTED. Test: `map::bwl::tests::a_lost_vaelastrasz_starts_over_as_the_copy_loads`.
- AB and AV banners (and silithyst) taken by CMSG_GAMEOBJ_USE with no cast: **fixed**, `53a280c1`. `world::use_object` refuses a locked objective (`battleground::locked_objective`), as mangos's `HandleGameObjectUseOpcode` does; the Opening cast's `opened` path is unchanged. Test: `tests_p3_world_bg::a_locked_banner_is_claimed_by_its_opening_not_by_a_hand`. The test data's banners have no lock, so the older tests are unaffected.
- BWL Razorgore: a wipe before anyone fights him leaves the encounter IN_PROGRESS: **fixed**, `24a6de7a`. `T_DEFENSE` sets FAILED when no living non-GM player is left, which is mangos's leash condition. Test: `tests_p12l_bwl::razorgore_fails_when_the_raid_dies_before_he_is_fought` (extended).
- Escorts fail on the starter alone, during fights, and only for the starter: **fixed**, `af312ec9`. The starter or any group member in range keeps the escort going; there is no check while the escortee fights or is held; a failure fails the quest for the group members on the map (mangos also fails members on other maps; wvs can't reach them from the map). Test: `tests_world::an_escort_goes_on_with_its_group_near_and_fails_for_the_group`. The extended escort shards pass.
- Battleground end: the dead stay ghosts, fighting goes on, AV's NPCs keep killing: **fixed**, `156e1ba5`. `finish` resurrects the dead (their corpses become bones), takes the living out of combat and off threat lists, and makes every creature that isn't a player's immune to both sides and stunned. Not ported: mangos's removal of Spirit of Redemption and `BlockMovement`. Test: `tests_p3_world_bg::a_matchs_end_raises_its_dead_and_stops_its_fighting`.
- Invitations to a match survive its end: **fixed**, `aa428bab`. `battlegrounds::ended` drains the invitations, frees each slot and sends its none status. Test: `tests_p3_world_bg::a_matchs_end_takes_back_its_invitations`.
- Deadmines: the cannon's assault stalls if the copy unloads in its 3-second window: **fixed**, `04f6ae91`. `settle_spawns` schedules the alarm again when field 1 is 1. Test: `tests_p9_core_scripts::the_deadmines_alarm_comes_after_a_copy_loaded_again_in_its_window`.
- Gurubashi Arena: an outsider's pet may attack a fighter: **fixed**, `dca9763f`. `arena::can_attack` resolves both sides to their masters first. Test: `tests_arena::an_outsiders_pet_may_not_strike_a_fighter`.
- Escort run_acts indexes `map.escorts[i]` across calls that run arbitrary scripts: **deferred**. This is a suspicion with no bug today, since only `update` removes escorts. The fix is a refactor of `run_acts` (look the escort up again after each act, or apply the effects after the loop), not a local fix.
- Battleground arrival counts anyone who lands on the map as a player of the match: **fixed**, `11b6c16f`. `arrived` gives a score, a side, the start and the raid only to members of the match. Test: `tests_p3_world_bg::one_not_of_the_match_who_comes_in_plays_no_part`.
- Script runs whose source isn't a unit live on until their last step: **deferred**. It is bounded and has no gameplay effect. mangos skips a step whose source is missing and goes on, and dropping the run would change scripts whose later steps don't need their source. Not worth the risk.
- `instance_script::run_event` starts map event scripts from an arbitrary creature: **not a bug**. The function has no caller (dead code).
- WSG and silithyst rely on the carrier being in `map.players` to drop the flag: **fixed**, `07e48ec9`. `wsg::update` sends home a flag whose carrier is no longer in the map. Test: `tests_p3_world_bg::a_flag_whose_carrier_is_gone_goes_home`.
- Town guards: no cap on the guards one town summons at once: **deferred**. This is a design call: a per-town cap or a per-caller cooldown in `[guards]` is policy beyond mangos (decision 65 chose mangos's call-per-victim with 5-minute guards).

## Extra findings from `survey/pvp_social.md`

- No start-area check before the gates open: **fixed**, `8e2cda27`. Each second in Waiting or Starting, a member farther than mangos-classic's `StartMaxDist` from their side's start is teleported back. The distances are 100 for AV and 75 for WSG and AB, from `sql/base/mangos.sql`; vmangos's table has no such column, so they are constants. Test: `tests_p3_world_bg::before_the_gates_open_one_strayed_from_the_start_is_put_back`.
- The return point (`bg_return`) isn't kept across a relog or restart: **fixed**, `881c9673`. A new `CharacterState::bg_return` (map, x, y, z, o) defaults like the other fields, so no migration. It is set at port, used by `leave` when the in-memory point is gone, used by the login path when the match is gone (instead of the bind point), and cleared when the player strays. Tests: `tests_pvp4::the_way_back_from_a_match_outlasts_a_relog` and `tests_pvp4::one_who_logs_in_where_a_match_is_gone_comes_back_where_they_joined_it`.

`e9f087f9` is a clippy fix to the Nefarius test.

## Note
