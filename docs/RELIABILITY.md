# Reliability

Thistle Tea gets its reliability from OTP: a process per player and per creature, under supervisors. A process that
crashes takes down only itself. Its supervisor starts it again with a fresh state, and gives up on it if it keeps
crashing. wvs runs an actor per map instead, and a handful of other tasks and threads. This document lists each of
them and says what happens to the server, and to the players, when it panics, hangs or is overloaded. It also says
which of OTP's properties wvs now has, and which it does not.

The code is in `src/game/supervisor.rs` (catching panics, backoff, the panic hook and the watchdog) and in
`src/game/map/supervise.rs` (what a map does about its own panics). The tests are in `src/game/tests_robust.rs`.
Each test makes the failure happen and shows the server recover.

None of this costs anything while nothing panics. `catch_unwind` is free when there is no panic: in a release build
of a quick timing loop, a call through it costs the same 2.8 ns as a direct call. The strike counts exist only for
what has panicked. A map's pulse to the watchdog is two relaxed stores each time round its loop. There are no new
locks and no new allocation per tick.

## What runs

| Task or thread | Spawned in | How many |
|---|---|---|
| Map actor | `Maps::get` (`map/mod.rs`) | one per map and per instance copy |
| Session | `game::run`'s accept loop, `session::serve` | one per world connection |
| Connection reader and writer | `conn::start` | two per world connection |
| Store thread | `Store::open` (`store.rs`) | one (a `std` thread) |
| Social sweep | `social::start_timers` | one, every 5 s |
| Transports' clock | `transports::start` | one, every 250 ms |
| Watchdog | `supervisor::start_watchdog` | one (a `std` thread), every 1 s |
| Login server | `server::async_main`, `auth::auth` | one accept loop, and a task per login connection |
| World accept loop | `game::run` | one, in the main task |

These are shared state behind locks, not tasks, but they fail in their own ways: the registry (`registry.rs`), the
social services (`Realm::social`), the accounts connection (`accounts.rs`) and the login server's session keys
(`auth::UserCache`).

## Map actors

A map is a tokio task that owns everything in it. It handles its events (packets, arrivals, disconnects, calls from
services) as they arrive, and runs a 20 Hz update.

**A panic.** Before, `guarded` caught a panic for each event and for each update as a whole. That made a bad packet
cost one event. But a panic in one creature's AI cost the whole update, every tick: everything after it was skipped
on every tick, including visibility and the flushes, so the map froze for everyone in it while still "running".
Now:

- **Each step of the update** (spells, auras, creatures, visibility, the flushes and so on, 41 in all) runs under
  its own guard. A step that panics costs itself this once, and the rest of the update goes on.
  (`an_update_step_that_panics_costs_only_itself`)
- **Each creature's turn** (its timed events and its fight, its walk home or its walk) runs under a guard. This is
  OTP's restart of one child. The first panic costs the turn. At the second panic within 5 minutes, the creature is
  restarted from its spawn: it goes quietly, with no loot or credit, and is respawned fresh at once. At the fourth
  panic it is quarantined: despawned for good and logged, so it no longer spams the log every tick. Strikes are
  counted by spawn, so a respawn is the same creature. A summon has no spawn, so its first restart removes it, as
  Thistle Tea's summons are `restart: :temporary`. (`a_creature_that_keeps_panicking_is_restarted_then_quarantined`)
- **A player's packet** that panics is dropped, as before, and now counts against that player. At 10 panics within
  a minute, the player is saved and sent back to the character screen, so they come back with a fresh state. This
  matches Thistle Tea, where a player process that crashes takes its connection down.
  (`a_player_whose_packets_keep_panicking_goes_back_to_the_screen`)
- **Escalation.** If the update keeps panicking (200 step panics within a minute, that is a step that panics on
  every tick for ten seconds), the map is in a bad state, and it is restarted. Its players are saved
  (`login::complete_logout`) and sent to the character screen. The map is built again from the data (`Map::new`
  and `world::populate`), and the transports on it are put back (`transports::restore`). The map keeps its handle
  and its channel, so events already sent to the old map (an arrival, say) are handled by the new one, and the
  players can log straight back in. (`a_map_whose_update_keeps_panicking_is_restarted`)
- **Populating** is guarded too. Before, a panic while a map loaded its spawns killed the actor.

A restarted map loses what lives only in the map: creatures killed in the open world come back at once, a limited
vendor's stock is restocked, chests are back. A battleground, the outdoor PvP towers, an arena fight or an escort
under way starts again from the beginning. A dungeon or raid copy keeps its dead, because they are in the copy's
save.

**An actor that has gone** (its channel closed) used to stay in `Maps`, so every later `maps.get` handed out a dead
handle. A login to that map failed silently, and its players' sessions disconnected them without a save. Now
`Maps::get` starts the map again, and a login whose arrival can't be delivered fails with `CharLoginNoWorld` rather
than leaving an online entry behind. With every event, step and turn guarded, an actor should now end only when it
is told to. (`a_map_whose_actor_has_gone_is_started_again`)

**A hang** (an endless loop in an event or an update) can't be pre-empted. The map's players freeze, and the tokio
worker thread it runs on is lost to every other task. The watchdog (below) now logs it loudly within ten seconds,
once, and logs again when the map is going again. A hung map also stalls the shutdown (see below).

**Overload.** The mailbox is unbounded. The actor handles up to 513 events and then runs its update, so a flood of
events slows the update down but does not starve it. An update over 50 ms is logged. Now the watchdog also logs a
map with 10,000 events waiting, and logs again when it has caught up. There is still no backpressure on what is
sent to a map; each client's packets are limited by its session's rate.

## Sessions

A session is a tokio task per world connection. It runs the world handshake (with a timeout), the character screen,
and then routes the client's packets to its map.

**A panic**, or an aborted task, used to skip `disconnect`. The player stayed in their map as a ghost: never saved
again, still in the registry, with the outbound queue closed. Worse, the account couldn't log in again. Each new
login found the ghost, sent `Control::Kick` to a session that no longer existed, and failed with "duplicate
character" until the server was restarted. Now:

- `Session` disconnects in its `Drop`, so it runs however the task ends. The player leaves their map, which saves
  them. A player handed over by a map (a `Control::Transfer` still in the channel) is passed on to their new map
  first, and saved there. (`a_session_that_panics_saves_its_player_and_frees_the_account`)
- A session that ends between going online and reaching a map (a panic in the login) takes its online entry with it.
- A map that hands a player to a session that has gone (`session::transfer`) passes them on to the map they were
  bound for, where they arrive and leave at once, saved. Before, the player was dropped unsaved.
  (`a_transfer_to_a_session_that_has_gone_is_saved_there`)
- A login that finds the account online with a session that has gone (a ghost, however it was made) lets it go: the
  map saves and removes the player, or, if no map has them, the entry goes. The next login gets in.
  (`a_login_that_finds_a_ghost_lets_it_go`)
- If the player's map has gone when they disconnect, they are taken off the online list. Whatever happened since
  their last save is lost with the map, but the account is not locked out.

At shutdown the maps have saved everyone already, so a session dropped then does nothing.

**A hang.** The session awaits the store on the character screen (the character list, a creation, a deletion, a
login). A hung store hangs that client's character screen, with no timeout of its own. In the world, the session
only forwards packets and does not wait.

**Overload.** A client over its packet rate (`max_packets_per_sec`, a token bucket) is disconnected. The inbound
channel holds 256 packets. When it is full, the reader waits, stops reading the socket, and TCP pushes back on the
client.

## Connections

Each connection has a reader task (it decrypts headers, decodes with wow_messages and answers pings) and a writer
task (it encrypts headers and coalesces writes).

**An abort.** This was the worst failure the survey found. Some of wow_messages' readers reserve memory for a count
taken from the packet before they read what it counts. Those are a `/who`'s zones and search strings, an express
flight's nodes, a bidder's item list, a bug report's text, and the uncompressed size of an auth session's addon
data or a ticket's chat. An absurd count asks for gigabytes. A `/who` that claims 4 billion search strings asks for
96 GB, the allocation fails, and the process aborts (`memory allocation of 103079215080 bytes failed`). No guard can
catch that, so one packet from any logged-in client took the whole server down. The reader checked those counts
against the packet's length (`conn::fits`) and dropped the packet unread. Since 12932's wow_messages fork at
5b7f7f6, wow_messages reserves no more than the rest of the body could hold (decompressed data, no more than zlib
could inflate it to), so such a packet is a parse error, or is read for what it holds, and the check is gone. That
goes for the world handshake's `CMSG_AUTH_SESSION` too, so a client that hasn't authenticated yet can't make it
reserve 4 GB either. (`a_packet_that_would_abort_the_server_is_dropped`,
`packets_that_would_have_wow_messages_reserve_gigabytes_are_refused`,
`the_world_handshake_reads_an_auth_session_and_refuses_an_absurd_one`)

**A panic** in wow_messages' parser (compressed data that isn't zlib, or a body shorter than its own size field)
used to end the reader task, and with it the connection. Those are parse errors in the fork now; the parse is still
guarded, as a backstop, so a panic drops the packet and counts as a bad packet. Any other panic in the reader still ends the task. Its channel to the session is dropped,
so the session ends and disconnects the player, saved. A panic in the writer drops the socket's write half. The
client sees the connection close, and the reader then ends at EOF, which ends the session the same way.

**A hang.** The reader has an idle timeout (`client_idle_timeout_secs`). A writer stuck on a client that doesn't
read is bounded by the next item.

**Overload.** A client whose outbound queue is over its byte budget (`outbound_channel_bytes`) is disconnected
rather than sent a partial world. A client that sends 32 bad packets within 10 s is disconnected.

## The store thread

`characters.sqlite`, behind one `std` thread that takes requests in order. Saves are fire-and-forget and are batched
into one transaction.

**A panic** used to kill the thread silently. Every save after that was lost, since `send` errors are ignored.
Every read returned its default: an empty character list, "no character" at login, and character creation failed.
Now each request, and each batch of saves, runs under a guard, and the thread's loop outlives a panic anyway. The
request that panicked is lost (its caller gets the default), and nothing after it is. (`the_store_outlives_a_panic`)

**A save that fails** (an error or a panic) used to fail its whole batch's transaction, up to 512 saves at once,
with one warning. Now a batch that fails because the database was busy (another connection holding the lock past
SQLite's 5 s busy timeout) is tried again up to three times. Any other failure has each save written on its own, so
only the bad one is lost, and it is logged with what it was. (`a_bad_save_loses_only_itself`) A duplicate spell or
action button in a character used to be such a bad save: the `character_spells` and `character_actions` primary
keys failed the insert, and with it the whole batch, at every save. The second copy is now ignored (a spell) or
replaces the first (a button).

**A field that won't load** used to cost the character's whole state. A character's state beyond its columns is
JSON, and a float that was NaN is saved as `null`, which an `f32` won't take back. The whole state then failed to
parse and went back to its defaults: quests, reputation, pets, talents and the hearthstone's place. A GM's
`.rested nan` was enough. Now only the field that won't parse goes back to its default.
(`a_saved_field_that_wont_parse_costs_only_itself`)

**A hang** (a disk that doesn't answer) stalls every save and read behind it: character screens wait, saves queue
up. SQLite's busy timeout bounds lock waits at 5 s a statement. There is no watchdog on the store.

**Overload.** The queue is unbounded. Saves are batched up to 512 to a transaction.

## The social sweep

A tokio task that every 5 s sweeps what time drives outside the maps: auctions, mail, instance resets,
battleground queues, seasonal events, the invasions, weather, meeting stones and world PvP.

**A panic** used to end the task, and with it every sweep, for good and without a word. Auctions stopped expiring,
mail stopped coming, raids stopped resetting, battleground queues stopped popping, and weather and events stopped
changing. Now each sweep runs under its own `Backoff`. One that panics is logged and rests: a second, then twice as
long each time it panics again, up to a minute, and a run that doesn't panic clears the rest. The other sweeps go on
meanwhile. (`a_sweep_that_panics_rests_and_the_others_go_on`) The services' locks are `parking_lot`'s, which don't
poison. A sweep that panics while holding one releases it, though what it guards may be half changed.

**A hang** in a sweep hangs the task, and its worker thread. If the sweep holds a service's lock, every map that
takes that lock blocks too. The watchdog doesn't watch the sweep.

## Transports

A tokio task that moves the boats and zeppelins between maps, every 250 ms, carrying their passengers.

**A panic** used to end the task: every transport stopped where it was, for good. Now each route runs under its own
`Backoff`. (`the_transports_clock_outlives_a_panic`) A map that is restarted gets its transports back
(`transports::restore`).

## The watchdog

A `std` thread, so a map that has hung a tokio worker can't hang it too. Every second it looks at each map's pulse:
when the map last came round its loop, and how many events wait for it. It logs, once each:

- a map that hasn't come round its loop in 10 s ("it has hung (an endless loop?), and its players with it"), and
  when it is going again;
- a map with 10,000 events waiting ("it can't keep up"), and when it is down to a tenth of that.

A hung map can't be stopped, so the watchdog only tells you. (`the_watchdog_sees_a_map_hang_and_a_mailbox_swell`)
A GM's `.maps` shows each map's panics, quarantined creatures and restarts.

## The panic hook

`supervisor::install_panic_hook` (installed at start-up in `server.rs`) logs every panic through tracing at `error`:
the thread, the place (`file:line:column`), the message, and the first time at a place, a backtrace. Before, the
default hook wrote these to stderr, apart from the log, and with a backtrace only if `RUST_BACKTRACE` was set. Lines
for the same place after the third are logged only at powers of two, and so are the guards' own lines ("map 0: the
update's spells panicked: ...; the rest of the update goes on"). A panic every tick therefore doesn't flood the
log. (`the_panic_hook_logs_where_once_with_a_backtrace`)

## The login server

`auth::auth`: an accept loop, and a task per login connection.

**A panic** in a login connection's task loses that login; its permit is released as the task's future is dropped.
The session keys are in `auth::UserCache`, a `std` mutex shared with the world handshake. A panic while it was held
used to poison it for good, and then every login and every world handshake failed ("users mutex poisoned"): no one
could log in or enter the world until a restart. The lock now recovers from poisoning: each change to the cache is
a single insert, so what it holds is still good. (`a_panic_holding_the_session_keys_locks_no_one_out`) A panic in
the accept loop used to end the login server for good, while the world ran on with no way in. A port already taken
panicked it at start-up the same way. The login server is now supervised (`supervisor::supervise`): a panic is
logged and the server is started again, at once and then after a backoff. A taken port is now retried every 5 s.
(`a_supervised_task_that_panics_is_started_again`)

## The accounts connection

`accounts.rs`: a SQLite connection behind a `std` mutex, used by the login server (each login), by sessions (a GM
level at login) and by maps (GM commands). Each query blocks the tokio worker it runs on while it lasts. A write
waits up to SQLite's 5 s busy timeout if the store holds the lock. A panic while the mutex was held used to poison
it, and every login was then refused. It now recovers from poisoning.
(`accounts::tests::a_panic_while_the_connection_was_held_doesnt_lock_everyone_out`)

## The registry

`registry.rs`: who exists, and who is online with their outbound queue, session and map. Its locks are
`parking_lot`'s (no poisoning), are held briefly, and are always taken in the same order (`by_account`, then
`online`). The registry can't fail by itself, but it could be left wrong: an online entry with no live session or
no map. That is the ghost described under sessions, and the session's `Drop` and the reaping of ghosts at login now
prevent it.

## Shutdown

On SIGTERM or Ctrl-C, `Realm::shutdown` tells every map to save its players and stop, and waits for each to answer.
It then flushes the store. A hung map used to hang the shutdown for good, and the other maps' saves already queued
never reached the disk. It now waits at most 15 s for all the maps together, then flushes the store, and waits at
most 15 s for that too. (`a_shutdown_does_not_wait_for_a_hung_map`)

## The chaos test

`tests_robust::chaos` loads the real world and puts three characters at the human start, among Northshire's
creatures, NPCs and objects. It then has them send random, well-typed client messages in random orders. For each
client opcode wow_messages reads, it builds bodies from values the players might name: the guids around them, their
items, and the world's spell, item, quest, area trigger and map ids. Floats include NaN, infinity and 1e9, and
strings range from empty to long. A shape that parses is kept and filled afresh. Players who die are raised, those
sent elsewhere arrive, and those kicked log back in. After each round of 250 messages, every player's map has to
answer a name query, and the test asserts that no map panicked. `random_client_messages_never_panic_a_map` runs it as
players. `random_gm_commands_never_panic_a_map` runs it as GMs, with a quarter of the messages GM commands with random
arguments. `gm_commands_with_odd_arguments_never_panic` tries every GM command with numbers out of range, NaN and
infinities. The suite runs 40 rounds (10,000 messages) of each; `WVS_CHAOS_ROUNDS` and `WVS_CHAOS_SEED` run more or
others.

At 300 rounds (about 73,000 messages each, reaching about 300 opcodes and drawing about 110 kinds of answer), the
players' run found no panic in the game's own code. What it and the GMs' runs found:

- **wow_messages panics** on compressed data that isn't zlib (`CMSG_AUTH_SESSION`, `CMSG_GMTICKET_CREATE`), and
  overflows on bodies shorter than their size fields (`CMSG_WARDEN_DATA`, `CMSG_BUG`). Each used to end the
  connection. Reading those parsers led to the allocation that aborts the process (see Connections). Both are now
  dropped packets.
- **A creature walk to a place no finite distance off** (a GM's `.move nan nan nan`, or `.go 1e30 1e30 1e30` with
  creatures following) panicked while working out the walk's duration, on every turn of every creature walking it.
  The walk now takes no time, and a GM's numbers have to be finite and, for a place, in the world.
  (`a_walk_to_nowhere_finite_takes_no_time`, `a_gms_numbers_must_be_finite_and_in_the_world`)
- **A NaN saved** cost the character's whole state at the next login (see the store).

## What OTP has and wvs does not

- **Pre-emption.** An Erlang process in an endless loop is still scheduled out and can be killed. A Rust task can't
  be pre-empted or killed, so a hung map stays hung. The watchdog only tells you.
- **Isolation of state.** An Erlang process's crash can't corrupt another's state. A panic in a map step can leave
  that map's state half changed (a player's batch half sent, say). The guards limit how far the effects reach, but
  only a restart clears them.
- **Per-player and per-object isolation within a step.** Creatures' turns are isolated, but most other steps (auras,
  spells, scripts) iterate over every unit, and a panic there costs the step for all of them this tick. Only the
  map's restart deals with one that panics every tick.
- **Supervision of the store, the accounts and the registry's consistency** is by guards and recovery, not by a
  restart with fresh state. The store and the accounts connection keep their database connection.
