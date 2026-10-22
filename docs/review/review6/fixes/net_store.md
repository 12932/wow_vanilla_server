# Bug-hunt fixes: network, sessions and the store (net_store)

Branch `fix/bughunt-net_store` (worktree `its worktree `), off master `0c5811f4`. Not merged or pushed.
Clippy clean; fast suite 3,409 passed; extended filter for the areas touched (transports, instances, raids,
dungeons, store, rename, delete, battlegrounds, accounts, restarts, supervisor) 254 passed on the real world. No
packet or create changed, so the client check wasn't needed.

- A rename can delete another character (`INSERT OR REPLACE` on a unique name): **fixed**, `810f1d37`. A save is now
  an upsert on the GUID (`ON CONFLICT(guid) DO UPDATE`, built from `CHARACTER_COLUMNS`), so a name clash fails that
  one save instead of deleting the other row. A rename is its own `Store::rename` request: written and awaited on
  the store thread, with `NameInUse` on a UNIQUE failure, and kept saves of that character get the new name.
  `character_items`' `INSERT OR REPLACE` is left alone on purpose: moving an item to its newest owner is the right
  answer to a stale save. Test: `store::tests::taking_another_characters_name_deletes_no_one`.
- Accounts: a failed read counts as "no such account" (auto resets the password, a ban fails open): **fixed**,
  `a694c69b`. `Accounts::read` returns the error. A login refuses on it. Auto creation is
  `ON CONFLICT DO NOTHING`. `banned` fails closed. Test: `accounts::tests::an_account_that_cant_be_read_isnt_made_again`.
- Frames up to 65,531 bytes reach the client, whose cap is 32,766: **deferred**. mangos-classic applies the cap only
  to update batches and mail (`UpdateData.cpp`, `MailHandler.cpp`). It doesn't cap other packets, and its
  `SendChannelListResponse` has no cap either (the review was wrong about that). Lowering the global guard would
  refuse packets mangos sends (a big guild's roster), on a crash claim that rests on a comment. That should wait for
  a client check of a 33 KB frame, and then a decision on which lists to cap.
- The memory guard can stop every client for good: **fixed**, `ab100c25`. A hold now lasts 30 s at most. Then the
  guard logs at `error`, lets go, and rests (1 min, doubling to 10). A start-up warning fires when RSS is already
  near the line. docs/CONFIG.md updated. Test: `memory_guard::tests::a_guard_that_brings_nothing_down_lets_go_and_rests`.
  The 30 s and the rest are constants: say if you want them as config keys.
- Instance and raid records sent after their lock is let go, out of order: **fixed**, `055d87fc`. Every
  `put_record`/`delete_record` in `instances.rs` is now sent under the instances lock. There is no test: the race
  window is microseconds and can't be reproduced deterministically. The existing instance tests pass.
- Battleground copies write an instance save for every death: **fixed**, `ad24ce6c`. `change_save` does nothing
  for a battleground map, and every path that writes the save goes through it. Test:
  `tests_ai5_instances::a_battlegrounds_deaths_are_not_saved` (fails without the fix).
- A new character's name entry is at level 1: **fixed**, `37a010ad`. It now takes the row's start level. Test:
  `tests_config::a_new_character_starts_at_the_configs_level_and_money` (extended).
- `route_of` builds every transport's route on each call: **fixed**, `ee9195f3` (+ clippy `20dc8e5f`). The routes
  are built once into `GameData::transport_routes` (a `OnceLock`). `routes`/`route_of` return borrows. Test:
  `tests::transports::a_transports_route_is_built_once`.
- Kept saves bounded by count, not bytes; a save together never merged: **deferred**. Dropping a character's rows
  from older `Together`s weakens the together-saves' all-or-none guarantee (decision 33) when a batch falls back to
  one by one. A byte cap decides what an outage loses. Both are design calls, and this happens only during a
  long outage.
- A character's delete is four statements outside a transaction: **fixed**, `33966e4e`. The delete is now one
  transaction, which also deletes the character's `account_data` (`ACCOUNT#guid`) and drops its kept saves.
  Test: `store::tests::a_deleted_character_takes_its_caches`.
- The login server's other protocol versions are leftovers: **deferred**. Refusing builds other than 5875 (as
  mangos's realmd does; peon sends 5875) would in effect retire protocols 2 and 8. Whether to keep them, drop TBC's
  nine-realm list and fold the copies is your call. Sending the character count needs the store reachable from the
  login server.
- The login server's restart backoff never resets: **fixed**, `bd080a96`. A run that lasted `Backoff::MAX` before
  it panicked clears the rest. Test: `supervisor::tests::a_run_that_lasted_clears_the_rest`.
- The ping rule disconnects GMs, which mangos exempts: **fixed**, `d6b4571f`. `Outbound` carries a GM flag that
  the session sets after the handshake, and the reader then skips the kick. Tests:
  `conn::tests::a_gms_flood_of_pings_is_answered`, `tests_config::a_gms_connection_is_spared_the_ping_rule`.
- The transports' clock is a `u32` of milliseconds and wraps after 49.7 days: **deferred**. The same clock goes to
  the client as a `u32` (`GAMEOBJECT_LEVEL`, the create's transport time), and the client takes it modulo the
  period. A `u64` clock on the server would disagree with the client after the wrap, so a fix needs per-transport
  client values and a client check. mangos-classic wraps the same way.
