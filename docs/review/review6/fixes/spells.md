# Bug-hunt fixes: spells and auras

Branch `fix/bughunt-spells` in `its worktree ` (not merged or pushed). New tests are in `src/game/tests_bughunt_spells.rs` unless another file is named.

- **A "cooldown on event" spell whose aura never lands is locked until relog (Riposte, Counterattack)**: **fixed in part**, 6389fb6e (test `a_waiting_cooldown_goes_with_an_aura_that_didnt_come_back`). At login, `restore` now drops the waiting cooldown of a saved aura that doesn't come back, as mangos's `_LoadAuras` does. The Riposte and Counterattack case is **not a bug** in 1.12: their 5875 rows don't have `SPELL_ATTR_COOLDOWN_ON_EVENT` (only builds up to 4695 do), and in the 1.12 data no on-event spell targets an enemy. The path where the aura's target is on another map is **deferred**: none of the 1.12 on-event spells puts its aura on another unit, and the fix would need a route through the registry.
- **Silithyst duplication through a logout**: **fixed** by the next finding's fix, 5bae6b13 (its aura has the leave-world interrupt flag, so it isn't saved; test `auras_mangos_doesnt_save_dont_come_back_at_login`).
- **Saved auras ignore mangos's save rules: map-leaving and single-target auras survive a relog**: **fixed**, 5bae6b13 (test `auras_mangos_doesnt_save_dont_come_back_at_login`). Not saved now: auras with `INTERRUPT_CHANGE_MAP`, single-target auras (all of them, as mangos does, self-cast too) and the Echoes of Lordaeron buffs.
- **A cast's line of sight isn't checked again as it ends: casts land through walls**: **fixed**, 601fe9c8 (test `a_cast_ends_only_in_sight_and_unrestrained`). The end-of-cast check now also covers line of sight, a player target on a flight and a silence or pacify on the caster.
- **Shadow Trance (and other charged cast-time modifiers) gained mid-cast is spent by the cast already under way**: **fixed**, af92f28d (test `a_trance_gained_mid_cast_is_kept_for_the_next`).
- **CMSG_CANCEL_CAST cancels whatever is being cast, and a queued strike, whatever spell it names**: **fixed**, 3decc1ad (test `a_cancel_stops_only_what_it_names`). `CMSG_CANCEL_CHANNELLING` now stops only a channel.
- **`tick_areas` walks stale indices when an area is cancelled during a tick**: **fixed**, c97b2920 (test `tests_p3_aura2_areas::an_area_ended_by_a_tick_doesnt_tick_another_early`). Areas are now matched by their dynamic object, so 22585ca9 gives each made-up area in `tests_spells5::dispels_by_the_stack_diminishing_recovery_and_shared_areas` an object of its own. They all shared `Guid::zero()`, which a real cast never does.
- **Every damage-over-time tick clones the spell's whole template**: **fixed** for the tick, 02df43de (perf, no new test). The full template clone into `InFlight` for each target of a travelling spell is **deferred**: changing that struct to hold an id or an `Arc` touches every caller of `land`.
- **Party auras' refresh clones every player's holders each second, and scans every creature**: **fixed** for the clones, bebdf75b (perf, no new test). The scan over every creature to find owned ones is **deferred**: it needs an index from owner to creatures in the pets registry.
- **CMSG_CANCEL_AURA lacks mangos's refusals**: **fixed**, 766563c6 (test `a_cancel_aura_spares_hidden_and_others_party_auras`), with follow-up d7c7bdfc: an aura whose spell isn't in the data can still be cancelled.
- **In-flight spells of a caster who left the map still land**: **fixed**, 00e9b007 (test `a_bolt_of_a_caster_gone_doesnt_land`).
- **Dead expression in `replaces`**: **fixed**, 130ca860 (a refactor that changes no behaviour, so no test).
- **Creature spell lists run the whole cast check twice per attempt**: **deferred**. The creature halts and turns to face its target between the two checks, and a failed first check resets the list's timer without rolling. Dropping one check changes when a cast is refused (facing especially), so this is not a refactor that keeps behaviour as it is.
- **Cannibalize's corpse search scans every creature on the map**: **fixed**, 612590f5 (perf; the existing Cannibalize tests pass, the extended ones included).
- **Review 3's spell and aura leftovers are still open**:
  - R3-16: **fixed**, aa91a76e (test `an_aura_gone_with_what_it_replaced_leaves_the_rest_alone`).
  - R3-18: **fixed**, 10f92373 (test `a_refreshed_aura_keeps_its_procs_cooldown`).
  - R3-20: its hit roll is **fixed**, 20a17484 (test `ground_ticks_roll_with_their_casters_hit`). The creature-type bonus that is scored against the caster is **deferred**: the amount would have to be worked out for each victim.
  - R3-22: **fixed**, 1287d8a6 (test `a_cast_at_a_target_gone_fails_for_its_watchers`).
  - R3-23: **fixed**, 5240666d (test `an_instant_failed_at_launch_gives_its_global_cooldown_back`).
  - R3-24: **fixed**, 1b43c338 (test `a_failed_cast_doesnt_dismount`).
  - R3-25: **not a bug**. mangos's `DoCreateItem` creates only what fits in the recipient's bags, and `CheckItems` checks only the caster's bags, so in mangos too a full recipient gets nothing and the reagents are spent.
  - R3-26: **fixed**, cd746873 (test `an_aura_taken_off_by_a_proc_doesnt_proc_after`).

Also fixed, from `survey/combat_spells.md`:
- **Persistent area spells tick through walls**: **fixed**, e820bcc6 (test `tests_p3_aura2_areas::a_ground_tick_doesnt_reach_through_a_wall`).
- **Damage shields can't miss, be resisted, or meet immunity**: immunity is **fixed**, 8e1428ab (test `a_damage_shield_meets_immunity`). That they can't miss or be resisted is **not a bug** by mangos-classic: `SpellHitResult` lets a positive effect mask through (the Thorns aura targets its caster), so mangos-classic doesn't roll a hit either.

Docs: review3.md now has a state line for R3-16 to R3-26, and CODE_REVIEW.md marks R3-16 as fixed.

Checks: `cargo clippy --all-targets` is clean. `cargo nextest run --lib` passes (3,417 tests). `cargo extended` with `WVS_DATA` set to the main checkout's `data/` passes (4,183 tests), after the test change above. I didn't run the client check: no packet's layout or create changed. The fixes only send existing packets (`SMSG_SPELL_FAILED_OTHER`, `SMSG_SPELLORDAMAGE_IMMUNE`) in more cases.
