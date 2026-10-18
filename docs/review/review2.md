# Review 2: the map core

Branch `fix/review2` in `<worktree>/review2`. Scope: `src/game/map/mod.rs`, `update.rs`, `fields.rs`,
`visibility.rs`, `grid.rs`, `movement.rs`, `login.rs`, `player.rs`, `registry.rs`, `instances.rs`, `transports.rs`,
`src/game/mod.rs`, `src/server.rs`, `src/accounts.rs`, `src/config.rs`, `src/game/map/gm.rs`, and what they reach
(the session's handshake, `auth.rs`'s reconnect) where a finding led there.

Each entry: where, what is wrong, how it shows, severity, and easy or hard. Fixed ones name their commit's test.

## The finding given to fix first

### 0. GM account commands and case: does not reproduce
- **Where:** `src/game/map/gm.rs` `account`, `ban`, `unban` → `src/accounts.rs` `set_gm_level`, `ban`.
- **What was said:** `.ban alice 10` finds no account, since names are stored uppercase and matched exactly.
- **What is so:** the `accounts` table's `name` is `TEXT PRIMARY KEY COLLATE NOCASE` (`accounts.rs` `SCHEMA`, and so
  since the table was made, commit 871ef7e). SQLite compares `name = ?1` with the column's collation, so `alice`
  finds `ALICE`. The kick after a ban looks up the registry with `name.to_uppercase()`, which is what clients send.
- **Done:** a test that keeps it so, `accounts::tests::a_gm_command_finds_an_account_in_any_case` (ban, unban and
  GM level in lower and mixed case). It passed before any change. Nothing else was changed for it.
- What the ban did miss is finding 6 below: the ban didn't hold against the session the account already had.

## Fixed (each with a test that failed before)

### 1. A dungeon copy left empty loses its save within 5 s if it had no kill in 5 minutes (high, easy)
- **Where:** `src/game/instances.rs` `sweep` (the `touched_at + unload < now` filter) and `touch`, which nothing
  called; `src/game/map/mod.rs` `unload_if_empty`.
- **What:** the realm's 5-second sweep deletes a dungeon save (and the owner's `owned` entry) once nobody is inside
  and `touched_at` is more than `max(instance_unload, 5 min)` old. `touched_at` was only set on entering and on a
  save change (a kill, script data), so it measured time since the last kill, not time empty.
- **How it shows:** a group fights a boss for 6 minutes, wipes and releases (the copy empties), or zones out to
  repair. Within 5 s the save goes; on return they get a fresh copy, progress gone, while the old copy's actor runs
  on for its own 5 minutes.
- **Fix:** the copy touches its save when it empties (`unload_if_empty`, the moment `empty_since` is set), so the
  sweep's clock and the copy's agree.
- **Test:** `tests_review2::a_dungeon_copy_left_is_kept_however_long_since_it_changed`.

### 2. Bank bag slots bought are lost at every login, with the money paid (high, easy)
- **Where:** `src/game/map/player.rs` `init_fields` (`PLAYER_BYTES_2`'s third byte always 0) and `to_row` (never
  saved); `src/game/store.rs` `CharacterState`.
- **How it shows:** buy a bank bag slot, relog: none bought. A bag already in the slot is stranded
  (`MustPurchaseBagSlot` to move one in).
- **Fix:** `CharacterState::bank_bag_slots` (serde default, so no migration), saved and restored (at most 6).
- **Test:** `tests_review2::bank_bag_slots_bought_survive_a_relog`.

### 3. A movement packet's transport offset isn't checked: NaN relayed to other clients and saved (medium-high, easy)
- **Where:** `src/game/map/movement.rs` `finite`.
- **What:** only the world position and orientation were checked. The on-transport block's offset and orientation
  were taken as sent: NaN or a mile off.
- **How it shows:** a client sends `MSG_MOVE_HEARTBEAT` with the on-transport flag and a NaN offset. Every observer's
  client is sent it (a likely crash of theirs), and it is saved in `state.transport`; at the next login
  `transports::place_aboard` puts the character at a NaN position, each login, until the store is edited.
- **Fix:** as mangos's `VerifyMovementInfo`: an offset over 50 yards across or 100 up, or not finite, drops the
  packet.
- **Test:** `tests_review2::a_place_on_deck_off_any_deck_is_dropped` (an observer is relayed only the good one).

### 4. The first move after a login or a map change goes unchecked (medium, easy)
- **Where:** `src/game/map/mod.rs` `insert_player`; `movement.rs` `on_move` checks only `if let Some(..) =
  p.move_check`, which starts as `None` (and `arrive` set it so for a transfer).
- **How it shows:** a non-GM's first heartbeat after logging in or zoning can be anywhere in the world under 20,000
  yards; it is taken and the speed check measures from there.
- **Fix:** `insert_player` seeds `move_check` with where the player came in.
- **Test:** `tests_review2::the_first_move_after_logging_in_is_checked` (a first heartbeat 500 yards off is pulled
  back).

### 5. An unreadable config.toml silently runs the server on the defaults (medium, easy)
- **Where:** `src/config.rs` `load`: any read error fell back to `ServerConfig::default()`.
- **How it shows:** `config.toml` without read permission, not UTF-8, or a directory: the server starts on the
  defaults, which means another `characters.sqlite`, `creation = auto` and no GM levels, logging only "no config
  file".
- **Fix:** only `NotFound` is the defaults; any other error is an error (the server exits 2, as for a parse error).
- **Test:** `config::tests::a_config_that_cant_be_read_is_an_error`.

### 6. A ban doesn't hold against the session the account already has (high, easy)
- **Where:** `src/auth.rs` `reconnect_version_2` and `_8`, and the world handshake in `src/game/session.rs`
  `handshake`. Reached from `.ban` (`gm.rs`).
- **What:** the ban is checked only at a logon challenge. The session key from the account's last logon stays in the
  user cache, and neither the reconnect challenge nor the world's `CMSG_AUTH_SESSION` looked at the ban.
- **How it shows:** `.ban alice perm` kicks Alice; her 1.12 client reconnects (the reconnect challenge, opcode 2, with
  its cached key) and is let in, then into the world. Banning an offline account whose session key is still cached
  is the same.
- **Fix:** a banned account's reconnect challenge is answered `FailBanned`, as a logon is, and the world answers
  `AUTH_BANNED` (in the clear), as mangos's `HandleAuthSession` does.
- **Test:** `tests_p3_network_auth::a_ban_outlasts_the_session_made_before_it` (protocols 3 and 8, and the world).

### 7. `.ban <name> <huge minutes>` says banned but bans no one (medium, easy)
- **Where:** `src/game/map/gm.rs` `ban`: `now + m * 60`.
- **How it shows:** `.ban alice 9223372036854775807` overflows: a panic in a debug build; in release the end wraps
  into the past and the account can log in, though the GM was told "ALICE is banned."
- **Fix:** checked arithmetic; past the end of time is for good (-1).
- **Test:** `tests_review2::a_ban_too_long_to_count_is_for_good`.

### 8. `.levelup <huge>` overflows (low, easy)
- **Where:** `src/game/map/gm.rs` `levelup`: `i32::from(level) + n`.
- **How it shows:** `.levelup 2147483647`: a panic in debug (caught, the command lost), level 1 in release.
- **Fix:** `saturating_add`.
- **Test:** `tests_review2::levelup_by_too_many_is_the_last_level`.

### 9. A player sent into a copy as it stops is lost (medium, easy once seen)
- **Where:** `src/game/map/mod.rs` `run`: on `Stop`, or `stopping` after the update (an empty copy unloading), the
  actor returned and its queue, with any `Arrive` in it, was dropped.
- **How it shows:** a member zones into a dungeon as the leader's `CMSG_RESET_INSTANCES` stops it (`reset` counts only
  those already in the copy), or as the copy's empty time runs out; or an arrival at a map just after its shutdown
  event. The `Player` is dropped: unsaved since their last save, still in the registry (the account can't log in
  again until the ghost is reaped), the client on a loading screen.
- **Fix:** `hand_on`: what was queued goes to the map started again for the key; on shutdown, or for a copy its raid
  reset took (which would stop again at once), an arriving player is saved and let go, and their session kicked.
- **Test:** `tests_review2::a_player_on_the_way_into_a_copy_that_stops_arrives`.
- **Left:** a dungeon copy started again this way has lost its save and owner (`unloaded` ran), so the player may be
  evicted after the grace minute. That is still better than lost.

### 10. A transport path with no stretch to sail panics at boot (low, easy)
- **Where:** `src/game/transports.rs` `Route::build` and `pose_at` (`self.splines[...]` with none).
- **How it shows:** `build` made a route with two frames when the first was a map change (nodes: normal, transition,
  skipped, normal, end), and no spline. `pose_at` then panics, outside the clock's per-route guard (`start` at boot,
  `place_aboard` at a login aboard). The shipped data doesn't have such a path.
- **Fix:** no spline, no route.
- **Test:** `transports::tests::a_path_with_no_stretch_to_sail_is_no_route`.

## Left

### High

#### H1. The on-transport flag skips the speed check: a client can teleport anywhere (hard)
- **Where:** `src/game/map/movement.rs:127` (`info.flags.get_on_transport().is_none()` in the speed check).
- **What:** a packet with the on-transport flag is never speed-checked, and nothing checks that its transport GUID is
  a transport on this map, or that the world position is near it. The position is taken, relayed, and becomes the
  next check's start.
- **How it shows:** one heartbeat with the flag set, any GUID, and a world position 5,000 yards away; the next packet
  without the flag goes on from there.
- **What a fix takes:** for the GUID, a game object of the map that is a transport (`map.globals`, or an elevator,
  type 11). For the place, the transport's pose now: boats' objects don't follow their route in the map (the client
  moves them), so it means `Route::pose_at(uptime_ms())` (the routes would need caching, see L9) or the elevator's
  animation position, then the world position within the deck bound (finding 3) of `passenger_world(offset, pose)`.
  Options: check only the GUID (cheap, stops the free form of it), or the full check. mangos checks only the bound.

### Medium

#### M1. The speed check ignores height (easy to change, hard to tune)
- **Where:** `movement.rs:132` compares `horizontal(last, position)`.
- **How it shows:** a heartbeat 3,000 yards up or down is taken: under the world, or over it.
- **Fix options:** a vertical bound for a non-falling packet (falls legitimately cover 60 yd/s; jumps less). Needs a
  tuned allowance, so it's left.

#### M2. `key_for` releases the instances lock between choosing a new id and storing it (medium, hard to test)
- **Where:** `src/game/instances.rs:316`: `inst.owned.entry((owner, map)).or_insert(id)`'s result is ignored and the
  caller's own `id` returned.
- **How it shows:** two members of a group, on different maps, entering the same dungeon at once both see no copy;
  each makes an id; the second is sent into a copy its group doesn't own, and evicted after the grace minute.
- **Fix:** hold the lock across choosing and storing, or use what `or_insert` returns and redo `admit_copy` for it.
  `admit_copy`'s player count has the same window (a copy over its limit).

#### M3. Shutdown misses a player between maps (hard)
- **Where:** `src/game/mod.rs:499` `shutdown_within`, `src/game/map/mod.rs:223` `Maps::get`.
- **What:** shutdown sends `Shutdown` to the maps running then. A player taken from one map (`take_player`) whose
  session hands them to a map started after it (`Maps::get` after `close`) is in a map that never hears of the
  shutdown; they aren't saved on SIGTERM. (An arrival queued at a map behind its `Shutdown` is now saved, finding 9.)
- **Fix options:** `Maps::get` after `close` returns a handle that saves arrivals (as `hand_on` does), or the shutdown
  waits on the registry until no one is online.

#### M4. Mana and health stored as 0 come back full at login (medium, easy, needs a choice)
- **Where:** `src/game/map/player.rs:473` (health 0 and not dead is full) and `:487` (mana 0 is full).
- **How it shows:** a mage logs out at 0 mana and back in to a full bar; relogging is a free drink.
- **Why left:** 0 stands for "never saved" (a new character). A fix needs a marker of a saved state (a
  `CharacterState` field, defaulting false) and a choice for characters saved before it.

### Low

#### L1. A ghost's logout countdown drops its moves (low)
- **Where:** `movement.rs:120` drops every move while `logout_at` is set; `login.rs` roots only the living and free.
  A ghost (or a feared player) asking to log out keeps moving on its client while the server ignores it.
- **Fix:** root every delayed logout, or cancel the logout on a move, as mangos cancels it.

#### L2. Logging into a battleground with no match to rejoin and no bind point makes a world copy of the battleground map
- **Where:** `src/game/session.rs:587`: on `Err(())`, only a bind point redirects; otherwise `MapKey::world(row.map)`.
- **Fix:** home to the race's start when there is no bind.

#### L3. `uptime_ms` wraps at 49.7 days (low, as mangos's `getMSTime`)
- **Where:** `src/game/mod.rs:623`; `supervisor.rs:235` `now.saturating_sub(beat)`.
- **How it shows:** after the wrap, a map that hung before it reads as beating 0 ms ago, so the watchdog never
  reports it. Transports' server-side pose jumps (their client clock is a u32 too, so that part is protocol).
- **Fix:** `wrapping_sub` in the watchdog.

#### L4. The create cache outlives the handler that filled it (low)
- **Where:** `src/game/map/mod.rs:1215` `public_create`, cleared only at the update's end (`:1035`).
- **How it shows:** a stealth change in a packet handler (`visibility::stealth_changed` → `introduce`) caches the
  object's create; an observer introduced in the next update gets that placement (a player's old position, a
  creature's spline from then) until its next move. Fields are right: what changed since is dirty and sent.
- **Fix:** clear the cache at the update's start too.

#### L5. `flush_forgets` is quadratic in an observer's forgets (low, speed)
- **Where:** `mod.rs:1085`, with `Batch::creates_of`, `forget` and `drop_for` (`update.rs`) each a scan.
- **How it shows:** a player teleporting out of a crowded city forgets hundreds of objects in one update: forgets ×
  (forgets + spans) comparisons on the map's task, about a few million for 800.
- **Fix:** count forgets and creates per object in one pass (a map), and index spans by object.

#### L6. `Grid::around` with a huge radius (low)
- **Where:** `src/game/map/grid.rs:67`: `(radius / CELL_YARDS).ceil() as i32` saturates; `cx - r` overflows (a panic in
  debug), or the loop is billions of lookups.
- **Reached by:** script and trap radii from the database, not the client; `visibility_yards` is clamped.
- **Fix:** clamp `r` to the world's cells (400).

#### L7. An object at a NaN position is never forgotten by distance (low)
- **Where:** `visibility.rs:19` `dist2`; every `d2 > keep2` is false for NaN. Client positions are checked; a
  server-made NaN (a spline, a script move) would keep the object known until it is removed.
- **Fix:** `!(d2 <= keep2)` where it parts.

#### L8. Fixed: see 10 above.

#### L9. `transports::route_of` builds every route on each call (low, speed)
- **Where:** `transports.rs:551`, from each login aboard (`place_aboard`) and each map restore. The routes could be
  built once in the realm.

#### L10. Zero values in `[network]` aren't refused (low)
- **Where:** `src/config.rs`. `max_pending_connections = 0` (no one can connect), `handshake_timeout_secs = 0`,
  `client_idle_timeout_secs = 0`, `outbound_channel_bytes = 0` each make the server unusable with no error.

#### L11. Out of scope, noted: a creature restored to life after its corpse despawned isn't put back in the grid
- **Where:** `src/game/map/spells/class_scripts.rs:473` (`RESTORE_TO_LIFE`) sets it alive without `grid.insert`
  (its despawn took it out). Players arriving later don't find it by `grid.around`; nor do aggro and scripts.

## Checked and found sound
- A GUID's halves are dirtied together; dirty bits cleared after one shared encode; the public mask matches mangos's
  `updateVisualBits`; values blocks carry the type; forgets go first; the 10-yard margin and 8-yard re-check.
- Every remove path takes the object out of the grid and tells its observers; empty cells are pruned.
- Registry lock order (`by_account` then `online`) is the same everywhere; no deadlock.
- A duplicate login kicks the other session; `arrive` has no early return between making the player and inserting
  them.
- Character name races at creation end at the store's unique index (`CreateError::NameInUse`).

## Verification
- `cargo clippy --all-targets`: clean. `cargo fmt`: run.
- Fast suite: 1,363 of 1,364 passed. The one failure, `tests_p3_combat2::the_gm_threat_list_runs_highest_first_and_an_evade_empties_it`
  (a wolf's threat dropped by the map's own update), passed 3 of 3 runs alone. An earlier run's
  `tests_p3_creature2::a_fallen_hound_dies_without_a_fighting_kin_in_reach` (a 20 s timeout under load) also passed 3 of 3 alone.
  Both are the known race with the map's real-time update.
- `cargo extended --no-fail-fast --test-threads 4` (`WVS_DATA` the main checkout's data): 1,654 run, 1,654 passed.
