# Review 5: speed, robustness and reliability (2026-10-20)

Six reviewers, one area each, read the code at 0.1.37 for what loses data, gets stuck, can be broken by a client
or bad data, or costs more than it must; each finding is verified (the path, and a scratch test where cheap). None
is fixed yet: [CODE_REVIEW.md](../CODE_REVIEW.md) lists them as open until you choose.

## The ones that matter most
| # | Severity | Area | Finding |
|---|---|---|---|
| 1 | high | world | A crafted express flight route overflows an update frame's size field and corrupts the packet stream of every player near |
| 2 | high | net | A connection's writer outlives its close while the client doesn't read, keeping its socket and its frames **Fixed.** |
| 3 | high | social | A long mail subject disconnects the receiver each time they open a mailbox |
| 4 | high | maps | A copy stopped from outside drops, unsaved, a player whose arrival is already in its queue |
| 5 | high | combat | A player pet's threat list is never pruned: whoever strikes a pet stays in combat while both live |
| 6 | high | data | A start on missing or broken world data wipes the skills of every character who logs in |
| 7 | medium | maps | One panic in a map's shutdown loses the saves of every player on it |
| 8 | medium | combat | A panic inside a death leaves its victim unkillable at no health |
| 9 | medium | combat | Triggered spells recurse without a bound (a spell that triggers itself aborts the server) |
| 10 | medium | data | A store that can't write loses every save while the realm plays on |
| 11 | medium | data, social | A deleted character's GUID, honor, tickets and raid binds go to the next character made after a restart |
| 12 | medium | net | An account can have any number of world sessions; a ban doesn't reach one on the character screen |
| 13 | medium | net | Account names keep the case typed: a lowercase login is another account in the world **Fixed.** |
| 14 | medium | social | A refund taken before its post is written comes back after a restart |
| 15 | medium | social | A client can have every mail in the realm scanned 1,000 times a second |
| 16 | medium | world | A possessed unit's position isn't validated (a NaN relayed to observers, saved) |
| 17 | medium | maps | After a restart, dungeon copies are swept at once, by the time of their last kill |
| 18 | medium | maps | A teleport during a logout's countdown leaves the player stunned |
| 19 | medium | data | A record that doesn't parse is dropped at start-up and its id given out again |
| 20 | medium | data | On a terminal, a start without the navmesh waits for an answer and never starts unattended |
| 21 | medium | data | A second server on the same store migrates it and runs its sweeps before it fails to bind |

The rest (about 35, low or speed) are in each area's section below, with the "checked and sound" lists.


## The network and sessions (`rev5-net`)

Worktree `<worktree>/rev5-net`, at master (22def61). Scope: `src/server.rs`, `src/auth.rs`,
`src/accounts.rs`, `src/game/session.rs`, `src/game/conn.rs`, `src/game/registry.rs`, `src/game/map/login.rs`
(`take_over`, the logout), the realm list, the rate limits and the outbound budget: a client's life from the TCP
accept to its logout.

Each finding was shown by a test in a scratch module (`src/game/tests_rev5_scratch.rs`, not committed, since
deleted from the worktree). A copy is beside this report, in `rev5-net-proofs.rs`; to run one, put it back as
`src/game/tests_rev5_scratch.rs` with `#[cfg(test)] mod tests_rev5_scratch;` in `src/game/mod.rs`, then
`cargo nextest run --lib tests_rev5_scratch --no-capture`. All eleven pass on master, which is what each finding
says. Timings are from a test build: wvs at `opt-level` 0, its dependencies at 2.

Three things this review found were also found by the other reviews running alongside it. They are at the end,
under "Found by the other reviews too", with what this review adds.

### Findings

#### 1. A connection's writer outlives its close while the client doesn't read, keeping its socket and its frames (high, reliability)
- **Where:** `src/game/conn.rs:572` (`write.write_all(&scratch).await`: no deadline, and nothing wakes it at a close),
  `conn.rs:363` (`close` marks the queue closed and queues an empty frame to wake the writer, which only helps a
  writer waiting on the queue), `src/game/map/login.rs:235-236` (`take_over` closes the old client's queue).
- **What:** a writer blocked in `write_all` on a client whose TCP window is zero isn't waiting on the queue, so a
  close doesn't reach it. It stays in the write until the socket fails. A peer that keeps acknowledging the
  zero-window probes keeps it from failing: no TCP user timeout or keepalive is set, and Linux retries a zero window
  for as long as it is answered. Meanwhile the writer holds:
  - the batch it took (up to 128 frames) and its `scratch` copy, up to the 4 MB budget (the budget is given back as
    frames are taken, before the write);
  - every frame still queued, since the queue's receiver lives in the writer: up to another 4 MB;
  - the socket and its kernel send buffer, up to `tcp_wmem`'s maximum (4 MB by default), which counts against the
    kernel's `tcp_mem` for every client.

  RELIABILITY.md says "a writer stuck on a client that doesn't read is bounded by the next item" (the budget). The
  budget closes the queue, but the writer stays.
- **How it shows:** `a_writer_on_a_client_that_doesnt_read_outlives_its_close`: a 16 KB frame to a peer that
  doesn't read, then `close`, then every sender dropped. Two seconds later the writer is still running; only the
  peer's going ends it. The path with one account: log in somewhere busy (Stormwind), with a small receive buffer,
  and read nothing. Log in again from a second connection: `take_over` closes the first connection's queue, and its
  writer, stuck, keeps what it holds. Repeat. Each round leaves up to about 8 MB of the server's memory and up to
  4 MB of the kernel's behind, for as long as the attacker keeps the dead connections open, until the process runs
  out of memory and everyone loses what they did since their last save. Without `take_over`, the over-budget close
  does the same once a connection (with `creation = auto`, any number of connections).
- **The fix:** the writer waits on the write or on the close, whichever comes first: a `tokio::sync::Notify` in
  `Shared` that `close` signals, and `tokio::select!` around `write_all`. A write that makes no progress for a bound
  (30 s, or `client_idle_timeout`) ends the connection too. On leaving, the writer drops its receiver (it does),
  so the queued frames go with it.
- **Effort:** easy to medium.

#### 2. An account can have any number of world sessions at once, and the character screen assumes one (medium, reliability)
- **Where:** `src/game/session.rs:116-160` (`handshake` takes the session key as often as it is offered),
  `session.rs:529-551` (`delete` asks `registry.online(g)`, then awaits the store), `session.rs:403-430`
  (`try_create` counts the characters, then creates), `session.rs:575` (`login`).
- **What:** one logon's session key opens as many world connections as a client asks for, all on the character
  screen together. mangos's `World::AddSession_` (`World.cpp:217-245`) kicks the account's other session when a new
  one arrives. wvs's character screen checks things and then awaits the store, which is right with one session per
  account and wrong with two. `login` puts the character online only after two store reads (`store.load` and
  `account_data::load`), so a delete in between finds them offline and goes ahead.
- **How it shows:**
  - `a_lowercase_login_and_many_world_sessions_from_one_key`: one login, then 50 world sessions open at once.
  - `a_character_deleted_as_it_logs_in_comes_back`: session A logs Lazarus in while session B deletes him. B is told
    `CharDeleteSuccess`, A enters the world, and A's next save (its first update, as `save_at` starts at now) writes
    Lazarus back: after A's disconnect the store lists him. On the way, the delete took the mail sent to him, his
    auctions with their items, his guild and his friends (`character_deleted` in each service). It also took his
    name from the registry, so name queries and whispers miss him, and he can't be logged into again
    (`CharLoginNoCharacter`) until a restart reloads the names from the store. It happened at the first try.
  - `two_sessions_of_an_account_create_past_its_limit`: two creates at once leave 11 characters with a limit of 10.
    The PvP realm's one-side rule (`session.rs:430-437`) passes the same way.

  A player gets there with two clients on one account, without trying to.
- **The fix:** one session an account, as mangos does it. The registry keeps each account's session (its control
  channel). A new world session, once its handshake passes, kicks the account's old one and waits for it to end
  before it starts its character screen. That also gives `.ban` a session to kick (finding 3).
- **Effort:** medium.

#### 3. A ban doesn't reach a session on the character screen, which can then log in (medium, robustness)
- **Where:** `src/game/map/gm.rs:84-86` (`.ban` kicks `online_for_account`, which is players in the world only),
  `src/game/session.rs:575` (`login` doesn't check the ban). Review 2's fix (#6) covers the reconnect and the world
  handshake, not a session already past them.
- **How it shows:** `a_banned_session_on_the_character_screen_still_logs_in`: EXILE is in the world on one session
  and on the character screen on another. `.ban EXILE perm` (as `gm.rs` does it) kicks the first, and the second
  logs the character straight back in. One session is enough: a player on the character screen when the ban comes
  (between characters, after a logout) picks a character and is in, banned.
- **The fix:** `login` refuses a banned account (`auth::banned`, then `SMSG_CHARACTER_LOGIN_FAILED` and a
  disconnect). With
  finding 2's one session an account, `.ban` kicks the session wherever it is, as mangos's ban kicks the account's
  session.
- **Effort:** easy.

#### 4. An account name is kept as the client typed it: a lowercase login is another account in the world (medium, robustness)
- **Where:** `src/auth.rs:488,529`, `:578,620`, `:647,691` (the session key is cached under `l.account_name` as
  sent, while the proof is made for `username.to_uppercase()`, `:538`), `src/game/session.rs:127`
  (`account = c.username`), and every exact comparison after: the registry's `by_account`, the store's
  `characters.account`, `gm.rs:84`'s kick by `name.to_uppercase()`.
- **What:** the 1.12 client sends its name uppercased (wow_srp's `normalized_string.rs` says so), but nothing makes
  another client do it. A logon as `casey` is proved against CASEY's verifier (the accounts table is `NOCASE`, and
  SRP uppercases), its key is cached as `casey`, and it enters the world as account `casey`: with its own character
  list, its own character limit and its own place online.
- **How it shows:** `a_lowercase_login_and_many_world_sessions_from_one_key`: after a login as `casey`, CASEY's
  character and casey's are online at once (`online_for_account` gives guid 1 for `CASEY` and 2 for `casey`). Each
  case variant of a name (2^n of them) is one more character online at once and ten more characters. `.ban casey`
  kicks only `CASEY`'s player; the `casey` one plays on (the world handshake refuses new ones, as `banned` is
  case-blind, but finding 3 lets a screen session in).
- **The fix:** uppercase the name at the logon and reconnect challenges, before anything uses it (the proof, the
  cache key), and in the world handshake (`c.username.to_uppercase()`), as mangos normalises it
  (`AccountMgr::normalizeString`).
- **Effort:** easy.

#### 5. Pings skip the flood guard, and a closed queue's check, and so keep a dead connection alive (medium-low, robustness)
- **Where:** `src/game/conn.rs:485-491`: the reader answers `CMSG_PING` and goes round (`continue`) before the
  session's rate (`session.rs:239`) and before its own `outbound.is_closed()` check (`conn.rs:494`).
- **What:**
  - A client may ping as fast as its link carries. Each ping is decrypted, decoded and answered with an encoded,
    queued `SMSG_PONG`, and none is counted. mangos kicks a player whose pings come less than 27 s apart more than
    `MaxOverspeedPings` times (`WorldSocket::HandlePing`, `WorldSocket.cpp:577-613`).
  - A queue closed for any reason (over budget, taken over, the player's map gone, `session.rs:278`) ends its
    connection only when the reader next passes on a packet that isn't a ping, or idles out. Nothing tells the
    session. A client that only pings keeps its player in the world, its session running and its queue closed,
    indefinitely: the idle timeout never fires.
- **How it shows:**
  - `pings_have_no_rate`: 5,000 pings in one burst get 5,000 pongs, and the session lives. 5,000 selections are cut
    off at the rate's burst.
  - `a_closed_connection_stays_in_the_world_while_it_pings`: the queue closed, and 3 s of pings later the reader,
    the online entry and the player in map 0 are all still there. The first packet that isn't a ping ends it.
- **The fix:** count pings in the reader (a bucket of their own, or mangos's 27 s rule with a few to spare) and
  disconnect past it. Check `is_closed` on every turn of the reader's loop, before the read. Have a close end the
  session at once: the session's `select!` waits on the same `Notify` as finding 1's writer.
- **Effort:** easy.

#### 6. The flood guard counts a packet as a packet, whatever it costs (low-medium, speed)
- **Where:** `src/game/session.rs:236-247` (one bucket for everything but the queries: 100 a second, 300 at once),
  `src/game/map/account_data.rs:114-137` (`parse_update` inflates up to 64 KB) and `:150-165` (`update` clones it
  into a store write), on the map's thread, and on the session's task on the character screen
  (`session.rs:553-567`); `src/game/map/social.rs:173-269` (`who`).
- **What:** every packet costs one token, whether it is a heartbeat or a 64 KB write. mangos puts `CMSG_WHO`,
  `CMSG_WHOIS` and `CMSG_INSPECT` on a 5 s cooldown each (`WorldSocket.cpp:71-80`), and drops what comes sooner.
- **How it shows:**
  - `account_data_uploads_are_64k_store_writes_at_the_packet_rate`: 299 `CMSG_UPDATE_ACCOUNT_DATA` of 92 bytes each
    (64 KB of zeros, zlib'd) took the map 53 ms, and wrote 18 MB to the store from 26 KB sent. At the rate, that is
    6.5 MB a second of writes a client, on the one store thread every save in the realm waits behind (the store
    keeps up, at about 350 MB a second on this disk with one key rewritten; a few dozen such clients don't). The
    burst is a 53 ms stall of the map: a missed update.
  - `a_who_costs_a_guild_lock_and_allocations_per_player_online`: a `/who` takes 204 µs on the asker's map with
    1,000 online and 584 µs with 3,000 (rev5-social #6 measured the same). At the burst, 300 of them stall the map
    175 ms; at the rate, they take 58 ms of each second.
- **The fix:** per-opcode cooldowns in the session, as mangos's: `/who`, whois and inspect at 5 s; an account-data
  upload of the same kind at a few seconds, and one equal to what's stored dropped before the store. `who`'s own
  costs are rev5-social #6's fix.
- **Effort:** easy.

#### 7. The connection caps are shared by everyone, so one host can hold every login slot (low-medium, robustness)
- **Where:** `src/auth.rs:194-215` and `src/game/mod.rs:1386-1410` (`max_pending_connections`, 1,024 permits for
  all; past them, `try_acquire_owned` fails and the connection is dropped); `auth.rs:250` (`expect_idle`: a login
  connection keeps its permit through the realm list, 120 s a request); `src/accounts.rs:206-219` (`login`).
- **What:**
  - A connection that sends nothing holds its permit for the handshake timeout (10 s). One host opening about 103
    connections a second keeps all 1,024 taken, and every other client is cut off at the accept: logins, and
    separately world handshakes. One account's logins hold them longer, polling the realm list.
  - With `creation = auto`, `Accounts::login` makes the account at the logon challenge, before any proof: an SRP
    verifier and a SQLite insert, on a tokio worker, under the accounts' mutex that the world's logins also take
    (`gm_level`). Logon challenges with new names alone fill the accounts table. (`auto` is documented as not for
    the internet; the order is still worth fixing.)
- **How it shows:** by reading. A test would bind 3724, which other worktrees' runs use.
- **The fix:** a cap per address beside the semaphore (a few pending connections per IP, in a small map), so one
  host can't take them all. Under `auto`, make the verifier from the name at the challenge but store the account
  only once its proof passes.
- **Effort:** easy to medium.

#### 8. Packets the old client sent still act on the player after another login takes them over (low, robustness)
- **Where:** `src/game/session.rs:263-281` (`MapEvent::Packet(*guid, msg)` and `handle_raw(g, ..)` carry the
  player's guid, not the session), `src/game/map/mod.rs:1042-1049` (the map handles a packet for whoever has the
  guid), `src/game/map/login.rs:235` (`take_over` tells the old session `TakenOver`).
- **What:** after `take_over`, the old session goes on forwarding its client's packets until it handles
  `TakenOver`. Its `select!` picks between its ready packets and its control message at random, so the packets
  already in its inbound queue (up to 256) can go first, about one more for each still queued, and the map applies
  them to the player the new client now has: a move, a cast, an item swap, a `CMSG_LOGOUT_REQUEST`. `Disconnect`
  carries its session for exactly this reason (`map/mod.rs:1051-1058`); packets don't.
- **How it shows:** by reading, not tested: it is a race between the old session's two channels.
- **The fix:** the session's packets carry its control channel (or an id kept in `Online`), and the map drops one
  whose sender isn't the player's session, as it does for `Disconnect`.
- **Effort:** easy.

### Found by the other reviews too

- **Frames over the 16-bit size** (rev5-social #1; rev5-world mentions it): `conn::encode` frames a message over
  65,535 bytes with its size wrapped. Shown here too (`a_mail_list_over_64k_is_framed_with_a_wrapped_size`: a frame
  of 80,153 bytes whose header says 14,617). What this review adds:
  - The guard belongs in `conn::encode` and `update::frame` (`update.rs:547` saturates at `u16::MAX` rather than
    wrapping, which misframes the client the same way), and its bound should be 32,766 bytes, not 65,533: mangos's
    `MAX_NETCLIENT_PACKET_SIZE` (`Common.h:109`) calls 32,767 the client's hard cap, and wvs's own update batches
    stop at 24 KB for the same reason (`update.rs`, `PACKET_LIMIT`).
  - One more list with no bound: a channel's member list (`social/channels.rs:379-394`, 9 bytes a member) passes
    32 KB at 3,641 members and the 16-bit size at 7,282.
- **A login in a dungeon or battleground starts that map's open-world copy, for good** (rev5-maps #5). Shown here
  too: `a_login_in_a_dungeon_sent_home_starts_the_dungeons_world_copy_for_good` (a character saved in the Deadmines
  and sent home starts map 36 instance 0, which is still running a second later).
- **A `/who` takes the guilds' lock for each player online** (rev5-social #6). The numbers are in finding 6.

### Smaller, noted
- `src/game/mod.rs:1381-1383`: the world's listener `expect`s its bind, so 8085 taken panics the process at
  start-up, where the login server (`auth.rs:184-192`) waits and tries again every 5 s.
- Neither the logon challenge nor the world handshake checks the client's build (`CMSG_AUTH_SESSION.build`); mangos
  refuses builds it doesn't serve. Another client version is let in and sent 1.12's packets.

### Checked and sound
- **The session's end:** `Session::drop` passes on a player handed over and not yet passed on, then disconnects; a
  transfer to a session that has gone goes on to the map and is saved there; a session gone between going online
  and reaching a map takes its online entry, but only its own (`same_channel`); a ghost found at login is reaped
  (`reap` checks `Arc::ptr_eq` before it takes anyone offline).
- **`take_over`:** the swap happens only while the registry still has the old entry (`registry.replace`, by
  `Arc::ptr_eq`) and the map has the player on it; the old session's `TakenOver` leaves the player where they are;
  a stale `Disconnect` from the old session is ignored (`same_channel`); the old client's held frames are let go.
- **The login's wait:** the row is read only once nobody of the account is online, and the store reads after the
  saves queued before it, so a fast relogin doesn't load an old row; `LOGIN_WAIT` and `GHOST_WAIT` bound it.
- **The logout:** refusals undo the countdown; the save is queued before `LoggedOut`; the session sends
  `SMSG_LOGOUT_COMPLETE` after switching its route; a disconnect during a logout finds no player and does nothing.
- **The registry:** locks taken in one order (`by_account`, then `online`) in `go_online`, `replace` and
  `go_offline`; `online_for_account` takes them one at a time; `go_offline` is only called by the player's own map
  or session, or after an identity check.
- **The world handshake:** one timeout over all of it; the auth session capped at 8 KB and parsed under a panic
  guard; the ban checked before the key; refusals sent in the clear.
- **The reader:** the header's size saturates, a body is at most 64 KB, raw opcodes are kept apart, a bad packet
  counts in a 10 s window, a wow_messages panic is a dropped packet, and the idle timeout is per packet. The inbound
  queue (256) pushes back to TCP.
- **The writer and the outbound queue:** ARC4 headers in order across bursts; the budget given back as frames are
  taken; frames queued at a close are dropped unwritten; `hold` and `release` order frames correctly across threads
  (the holding flag is checked again under the held lock); held frames count against the budget.
- **The rates:** `PacketRate` saturates time and caps its burst; `Recent` is bounded (2,048 entries); queries have a
  bucket of their own.
- **The login server:** every read has a timeout; a reconnect proof randomises the challenge after each try, so a
  proof can't be replayed; a banned account's reconnect is refused; the session-key cache is pruned by idle time,
  its lock recovers from poisoning. `now - self.idle_ttl` (`auth.rs:114`) can't panic on Windows with this
  toolchain: std (1.99) offsets Windows' `Instant` epoch so it doesn't start near zero.
- **The accounts:** the connection's lock recovers from poisoning; names are `NOCASE` in the table.
- **Shutdown:** the world's accept loop stops, the maps save within 15 s, those between maps are waited for, then the
  store; the login server is aborted last.
- **Character creation:** names are checked and capitalised; a look the client has no texture for is refused at
  creation and mended at login; a name taken at once by two sessions ends at the store's check.

### What wasn't checked
- What the 1.12 client does with a misframed or 32 to 64 KB packet: `tools/client_check` wasn't run (it has no
  handler for the mail list, rev5-social notes).
- Finding 1's memory figures are from the code (budget, batch, `tcp_wmem`), not measured on a stuck socket.
- Finding 7's flood was not run.
- The timings are from a test build; a release build would be several times faster, the same shape.

## The map actors and their lifecycle (`rev5-maps`)

Worktree `<worktree>/rev5-maps`, detached at master `22def61`. Line numbers are at that commit.
Scope: the map actors and their lifecycle (`src/game/map/mod.rs`: the event loop, the update, `after_events`,
`finish`, `deliver`, `hand_on`, `let_go`), `src/game/supervisor.rs` and `map/supervise.rs`, `Maps::get`/`stop`/`close`,
transfers between maps (`take_player`, `teleport_with`, `session::transfer`, the session's `Control` handling),
`src/game/instances.rs` and `map/eviction.rs`, the shutdown (`Realm::shutdown_within`), `src/game/transports.rs`, and
the store's queue (`src/game/store.rs`). Where a finding led there: `session.rs` (`enter`, `disconnect`, `reap`),
`map/login.rs`, `social/mod.rs`'s timers, `registry.rs`.

Nothing here repeats RELIABILITY.md, PERFORMANCE.md, CODE_REVIEW.md or reviews 2 to 4, except where noted as
worse than written. Each finding was proved by a scratch test, unless it says otherwise. The tests aren't committed
(the worktree is clean); their code is in [rev5-maps-tests.rs](rev5-maps-tests.rs), with how to run them. Each test
asserts the bug, so it passes while the bug is there.

### Findings

#### 1. A copy stopped from outside drops a player whose arrival is already in its queue
- **Severity:** high. **Kind:** reliability (a player dropped, unsaved).
- **Where:** `src/game/map/mod.rs:1066` (`MapEvent::Stop => return true`), with `finish` (`:834`) and `hand_on`
  (`:915`); reached from `src/game/instances.rs:878` and `:893` (`reset`) and `:998` and `:1017` (`expire`).
- **What:** `reset` and `expire` decide that a copy is empty from the registry,
  `online.iter().any(|o| o.map().key == key)`. That misses a player on their way in. `Online::map` is set only in
  `Map::arrive`, on the copy's thread (`mod.rs:1091`). From the old map's `take_player` until the copy handles the
  arrival, the player is "on their old map". A login into the copy is on `MapKey::world(map)` instead (finding 5).
  `instances` does know them: `admit_locked` put them in `on_the_way` (`:242`). But neither function looks there.
  `Maps::stop` then queues a `Stop` behind the `Arrive`. The copy handles the arrival: it inserts the player, sends
  `SMSG_NEW_WORLD` and points their online entry at itself. Then `Stop` ends the actor with the player inside.
  `finish` hands on only what is still in the queue, and `map.players` is dropped with the map. Review 2's #9 fixed
  the other order (the `Stop` before the `Arrive`). This one is still open.
- **How it shows:** the test `rev5_a_member_arriving_as_the_leader_resets_is_dropped`:
  1. Alice (the leader) and Bob are in a party. Alice makes the group's copy and comes out.
  2. Bob has 12,345 coins that he hasn't saved yet, and joins channel `peon`.
  3. The copy's actor is busy for 400 ms. In the test this stands for a long update, a burst of events, or a fresh
     copy still populating.
  4. Bob sets out for the copy, and Alice sends `CMSG_RESET_INSTANCES`. The server answers `SMSG_INSTANCE_RESET`
     rather than a failure.
  5. Bob is in no running map. His online entry names the copy's closed handle. When his client goes, he is taken
     offline without a save: the store has 0 coins, and he is still in `peon` (finding 6). The test passed 5 runs
     of 5.

  The reset is the group leader's to send at any time. The weekly raid reset (`sweep` → `expire`) and a GM's `.reset`
  do the same to anyone zoning into an empty raid copy. The window is the time the arrival waits in the copy's
  queue. The player loses everything since their last save (up to `save_interval_secs`, 5 minutes). Their client
  sits on the loading screen until its worldport acknowledgement finds the map gone and the connection closes.
- **The fix:**
  - (a) Make `Stop` safe in either order. In `handle_event`, on `Stop`, take each player still in the map and treat
    them as `hand_on` treats an arrival: into the copy started again, or, if the copy has expired, saved and let go.
    This alone closes the loss.
  - (b) Also have `reset` and `expire` count a copy as occupied while `inst.coming(key, ..)` has anyone, so the
    player doesn't land in a fresh copy with no save (review 2's #9 "Left").
- **Effort:** easy.

#### 2. One panic in a map's Shutdown loses the saves of every player on that map
- **Severity:** medium. **Kind:** reliability (saves missed).
- **Where:** `src/game/map/mod.rs:1067-1075` (the `Shutdown` arm), under the single guard of the event
  (`handle_guarded`, `:721`); also `src/game/map/supervise.rs:190-216` (`send_to_screen`'s fallback).
- **What:** the shutdown takes and saves every player in one loop, inside one event. `take_player` and
  `save_and_go_offline` run a good deal of code: threat lists, captured units, pets, single-target auras, the
  battleground, loot release, duels, trades and the social logout. A panic for any one player unwinds the whole
  loop. The players after them are neither saved nor taken offline, and the reply is dropped.
  1. The map then runs on.
  2. `shutdown_within` waits its whole limit (15 s) for `online_count() == 0` and logs "N still online".
  3. It flushes the store and exits.

  So everyone on that map loses what they did since their last save, at every shutdown while the bug lasts. The
  restart already guards each player (`send_to_screen`); the shutdown doesn't. That fallback has a gap of its own.
  When the panic comes in `save_and_go_offline`, after `take_player` has removed the player, `map.players.remove`
  finds nothing. The player is then not saved, not taken offline and not told `LoggedOut`. They stay a ghost until
  a later login reaps them. `handle_disconnect` has the same gap.
- **How it shows:** the test `rev5_one_panic_in_the_shutdown_loses_the_rest_of_the_maps_saves`, with a test-only
  fault hook at the top of `take_player`. Four players on map 0, past their first save, each get 777 coins. The
  first in the map's order panics as it is taken. After `shutdown_within(2 s)`, 0 of 4 are saved and all 4 are still
  online.
- **The fix:** guard each player with `catch(|| { take_player; save_and_go_offline })`. On a panic, fall back as
  `send_to_screen` does, and always send the reply. In the fallback, take the player offline when the registry
  still points at this map, even if `players` no longer has them.
- **Effort:** easy.

#### 3. After a restart, dungeon copies are swept at once, by the time of their last kill
- **Severity:** medium. **Kind:** reliability (state that should survive a restart).
- **Where:** `src/game/instances.rs:929-963` (`sweep`'s stale filter), `:1033` (`touch`, in memory only), `:131`
  (`Instances::load`), and `src/game/social/mod.rs:110` (the sweep's `interval`, whose first tick is at once).
- **What:** a dungeon save's `touched_at` reaches the store only when the save changes: a kill, a respawn, script
  data, or an entry through `key_for`. Review 2's #1 had the copy touch its save as it empties. But `touch` doesn't
  write the record, and a copy that is occupied at a shutdown never empties.

  At start-up the first sweep runs at once, before anyone can log in, because tokio's `interval` ticks first
  immediately. It deletes every dungeon save that has nobody online in it and whose last stored change is older than
  `max(instance_unload, 5 min)`, owner and all. Those who logged out inside, or were saved there by the shutdown, go
  to their bind point at their next login: `resume` → `last_entered` finds nothing. The downtime counts as well, so
  a restart that takes 5 minutes clears every dungeon copy. The header of instances.rs promises: "Every copy is
  saved ... so a restart finds it as it was."
- **How it shows:** the test `rev5_a_dungeon_copy_is_swept_at_a_restart_by_its_last_kill`, on a file-backed store:
  1. Alice, alone, enters the Deadmines, and the server shuts down.
  2. The test sets her copy's stored `touched_at` 11 minutes back: inside that long without a kill. Six minutes
     would do on a server, whose limit is 5; the test's realm uses 10.
  3. The server starts again on the same file. The save is loaded, then the sweep removes it.
  4. Alice logs into map 0, not her copy.

  The test's first run moved `touched_at` back too little for the sweep to take it. In that run Alice logged back
  into `map 36 instance 1`.
- **Context:** groups don't survive a restart (R4-19), and `load` takes a group's copy away from it. So the members
  of a group's copy go home after a restart regardless. This finding adds solo players' copies, and the copies that
  `adopt` would give back to the old leader's next group.
- **The fix:** in `Instances::load`, set each dungeon save's `touched_at` to now. A copy then has the same five
  minutes after a restart that it has after emptying. Alternatively, write the record in `touch` and touch every
  occupied copy at the shutdown.
- **Effort:** easy.

#### 4. A teleport during a logout's countdown leaves the player stunned
- **Severity:** medium. **Kind:** reliability (a stuck state).
- **Where:** `src/game/map/mod.rs:1480` (`teleport_with`: `p.logout_at = None`) and `src/game/transports.rs:852`
  (`teleport_aboard`, the same), against `src/game/map/login.rs:394-418` (`undo_logout`) and `:420-429`
  (`cancel_logout`).
- **What:** a far teleport drops the logout but doesn't undo what `request_logout` did. These stay: `UNIT_FLAG_STUNNED`,
  the logging-out byte of `PLAYER_FIELD_BYTES`, the sit, and `misc.logout_rooted`. The client isn't told either: no
  `SMSG_LOGOUT_CANCEL_ACK` is sent, so its countdown runs on to nothing.
  - On the new map, the client is created stunned.
  - A `CMSG_LOGOUT_CANCEL` does nothing: `logout_at` is already `None`, so there is no acknowledgement and no
    `undo_logout`.
  - While `logout_rooted` is set, the end of a later stun or root keeps the flag set (`auras.rs:2112-2121`).

  The only ways out are another logout request followed by a cancel, or a relog. mangos-classic keeps the logout
  going through a teleport, on the session's timer.
- **How it shows:** the test `rev5_a_teleport_during_a_logout_leaves_the_player_stunned`. With a 20 s logout delay,
  Alice asks to log out (stunned, rooted, counting down) and is then teleported to map 1.

  | Moment | Stunned | Logging out | Rooted |
  |---|---|---|---|
  | On arrival | true | false | true |
  | After `CMSG_LOGOUT_CANCEL` (no acknowledgement) | true | false | true |

  Teleports that can land in a 20-second countdown without the player's doing:
  - a boat or zeppelin crossing to the other continent (`transports::cross` → `teleport_aboard`);
  - the end of a copy's eviction grace;
  - a battleground's end sending everyone home;
  - a script's or a GM's teleport.
- **The fix:** either call `undo_logout` in both places, or keep the countdown going as mangos does: leave
  `logout_at` alone and let the new map complete it.
- **Effort:** easy.

#### 5. Logging into a copy starts that map's world copy, which never stops
- **Severity:** low. **Kind:** speed (CPU and memory for nothing).
- **Where:** `src/game/session.rs:692-700` (`enter`).
- **What:** `enter` calls `maps.get(MapKey::world(row.map))` before it knows where the player is going, and builds
  the `Online` with that handle. Only afterwards does `resume` (a dungeon or raid) or `rejoin` (a battleground) pick
  the copy or the bind point. `Maps::get` starts the actor at once, and it populates the whole map with every spawn.
  A battleground's world copy has no match, so it gets every event's spawns (`spawn_allowed` is true). A world map
  never unloads, because `unload_if_empty` returns for instance 0. So the first login of anyone who logged out in a
  dungeon, raid or battleground starts an empty, populated copy of that map that runs 20 updates a second for good.

  Over a server's life that can be all 30 instanced maps, from vmangos.sqlite at patch 10:

  | | Creature spawns | Object spawns |
  |---|---|---|
  | All 30 instanced maps | 12,408 | 6,043 |
  | Alterac Valley alone | 1,586 | 1,351 |
  | Eastern Kingdoms, for comparison | 24,572 | |

  PERFORMANCE.md puts a continent no one is on at about 0.02 cores. On that basis I estimate these copies at about
  0.01 cores, plus the memory of 18,000 objects, on a server that idles at 0.04. Review 2's L2 is the narrower case
  where the player also ends up in such a copy.
- **How it shows:** the test `rev5_a_login_into_a_copy_starts_the_maps_world_copy_for_good`. Alice enters her
  Deadmines copy, logs out and logs back in. She is back in `map 36 instance 1`, while `map 36` (instance 0) is
  running with no one in it (21 updates within a second).
- **The fix:** choose the key first, and call `maps.get` once, for that key. `resume` and `admit_copy` read the
  account from the registry, for the hourly count; that is why `go_online` comes first. Passing the account in
  removes that need.
- **Effort:** easy.

#### 6. Players let go or lost with a map go offline without the social logout
- **Severity:** low. **Kind:** reliability (stale state elsewhere).
- **Where:**
  - `src/game/map/mod.rs:901` (`let_go`);
  - `src/game/session.rs:341` (`disconnect`, when the map has gone);
  - `src/game/session.rs:799` (`reap`);
  - `src/game/map/supervise.rs:214` (`send_to_screen`'s fallback).
- **What:** each of these calls `registry.go_offline` and nothing else. `save_and_go_offline` also runs
  `map::social::logged_out` (`social.rs:508`), which:
  - leaves the chat channels;
  - tells friends;
  - takes the player out of battleground queues and invitations (`battlegrounds::logged_out`);
  - updates their guild's and group's status.

  Without it, the player stays a member of their channels (members are kept by GUID, and `Channels::of` scans them
  all at every logout). They stay queued for battlegrounds, so a match can be filled with them, and their group and
  friends see them online.
- **How it shows:** in finding 1's test, Bob, dropped and then offline, is still in channel `peon`:
  `channels.of(bob) == ["peon"]`.
- **The fix:** one function for "this player has gone" that does both the registry and the social parts, used by
  every path.
- **Effort:** easy.

#### 7. Summoned GUIDs repeat after 2^23 summons on a map
- **Severity:** low. **Kind:** reliability (two objects under one GUID).
- **Where:**
  - `src/game/map/scripts.rs:1081-1084` and `src/game/map/pets/summon.rs:56`, among the 21 sites that take
    `next_creature_counter`. Creatures and game objects share it: totems, guards' calls, bobbers, traps, outdoor
    PvP, escorts, events.
  - `src/game/guid.rs:45`, which cuts the counter to 24 bits.
  - `src/game/map/mod.rs:2027` and `:2044` (`by_guid.insert`).
- **What:** a GUID's counter is `next_creature_counter | 0x80_0000`, cut to 24 bits. It therefore repeats every 2^23
  (8.4 million) steps of the map's counter; a pet's counter (`| 0xA0_0000`) repeats every 2^22. The continents live
  as long as the server. When the counter comes round, a second object of the same entry gets the GUID of one still
  alive:
  - `creature_by_guid` (or `gameobject_by_guid`) now points at the new object;
  - the old object stays in the slab, can't be reached by its GUID, is never removed, and is still updated;
  - clients see two objects under one GUID.

  mangos-classic gives each map its own creature and game object counters over the 24-bit space. At overflow it
  stops the server ("guid overflow!! Can't continue") rather than reuse a GUID (`ObjectMgr.cpp:120-128`).
- **How it shows:** the test `rev5_summon_guids_repeat_after_two_to_the_twenty_three`. Two `scripts::summon` calls of
  the same entry, 2^23 counter steps apart, return the same GUID; the map then has 2 creatures and 1 index entry. At
  ten counter steps a second on a continent, summons repeat after 9.7 days and pets after 4.9 days; I didn't
  measure the real rate. A clash also needs the older object to still be alive (a pet kept out for days, or a
  long-lived summon), so it is rare.
- **The fix:** take the next counter whose GUID isn't in use (checking `creature_by_guid` and `gameobject_by_guid`),
  with separate counters for creatures and game objects, as mangos has.
- **Effort:** easy.

#### 8. Logging in aboard a ship that has crossed to the other continent leaves the player at sea
- **Severity:** low. **Kind:** robustness.
- **Where:** `src/game/transports.rs:575-589` (`place_aboard`).
- **What:** `if pose.map != p.row.map { return; }`. The transport state has already been taken (`take()`), and it is
  dropped. The player comes back at the world position they had when they logged out, which is where the ship was:
  on the crossing's open water, on their map's stretch of the route. mangos-classic's `Player::LoadFromDB`
  (`Player.cpp:14291-14336`) loads them onto the transport's other map, on deck, or sends them to their bind point
  if it can't.
- **How it shows:** by reading only. I didn't run it on the real routes. The two-map ships (Menethil to Theramore,
  Booty Bay to Ratchet, the zeppelins) cross once each way per round, so they spend part of every round on the other
  map.
- **The fix:** when the ship is on the other map, log the player in there, on deck. That is the session's choice,
  before `maps.get` (`session.rs:692`), so it belongs with finding 5's change. Otherwise, send them to their bind
  point.
- **Effort:** easy to medium.

#### 9. Full scans under the instances lock
- **Severity:** low. **Kind:** speed.
- **Where:** `src/game/instances.rs:422-447` (`admit_locked`), `:171` (`raid_bind`), `:622` (`belongs`), `:929-963`
  (`sweep`), `:872` (`reset`), `:986` (`expire`).
- **What:** the realm-wide instances mutex is held across scans of the whole realm:
  - Each entry into a copy walks every online player: `all_online()`, then `o.map().key` for each. That locks the
    entry's mutex and clones its map's sender (four atomic operations on that map's channel) just to read a key.
    `sweep` repeats this walk for each stale save.
  - `belongs` walks every raid bind twice, because `raid_bind` is a linear search over all raid saves' player
    lists. Eviction calls it once a second for every player in every copy. `send_saved` (each login and worldport)
    and `raid_infos` walk the binds the same way.

  With 3,000 online, a dungeon entry costs 3,000 mutex locks and sender clones. A week's raid binds (say 200 copies
  of 40) cost about 16,000 comparisons per player per second in raids. Every map's evictions, entries, kills and
  `expired` checks take this lock; `expired` runs at each update of each empty copy. The cost grows with the realm,
  not the copy. These figures are estimates; I didn't measure them.
- **The fix:**
  - keep each player's map key in `Online` as an atomic beside the handle;
  - count who is inside a copy at `arrived` and `take_player`;
  - index raid binds by player.
- **Effort:** medium.

### Checked and sound
- **`Maps::get`/`close`:**
  - `get` reads `closed` under the write lock that `close` takes, so no map starts unseen by the shutdown (M3's fix);
  - a key whose actor has gone is started again;
  - `let_go_all` saves anyone sent to a map asked for after the close.
- **`finish`:** it closes the queue and then reads it to its end, so a send racing the stop is either read and handed
  on, or refused and retried by `deliver` (up to 4 times, then `let_go`). The copy's own unload sets `stopping` in
  the update and breaks before it reads the queue again, so it can't have finding 1's ordering.
- **Session and map handover:**
  - the session's `Drop` closes its control queue before draining the `Transfer`s in it;
  - `session::transfer`'s fallback for a session that has gone delivers the player and disconnects them with their
    own session's channel, so they are saved;
  - `take_over` checks identity (`Arc::ptr_eq`, `registry.replace`), and a stale session's `Disconnect` is ignored
    through `same_channel`;
  - a ghost is reaped at the next login after 1 s.
- **Holds:** a held client is let go on every way out of a map: `take_player`, `take_over`, a step's panic (`Held`'s
  drop), `send_to_screen`'s fallback, and the map's own drop. Holds are bounded by the outbound budget.
- **Supervision:**
  - the step, turn and packet guards;
  - the restart: per-player catch, bones handed back, transports restored, handle kept;
  - `Strikes` pruned at 64 entries;
  - `Backoff` and `supervise` back-off;
  - the panic hook's `SEEN`, keyed by place, so bounded;
  - `packet_name`, which interns only opcode names (wow_messages's `Display` prints the name alone).
- **The store:**
  - one player's saves stay in order across maps, because each send happens-before the transfer that hands them on;
  - saves are written before a read that follows them;
  - a busy batch is retried, a failed one falls back to each save alone, and a save-together is never split;
  - the thread outlives a panic;
  - the shutdown's flush waits for what was queued before it.
- **Lock order:** groups → instances → registry (`key_for`, `resume`, `belongs`, `group_changed`), battlegrounds →
  instances (`fill`), and the registry's own `by_account` → `online`. I found no inversion, and nothing in my area
  holds a service lock across a send that can block.
- **Transports:**
  - `Route::build`, `Spline::new` and `pose_at` index within bounds for every route the builder accepts;
  - a crossing places the ship on the new map before the passengers' teleports are queued, and removes it from the
    old map after;
  - each route runs under its own `Backoff`;
  - `carry`'s per-route scan of the map's players costs microseconds.
- **Tile readers:** a job that panics costs itself, and the lock is let go before a job runs.
- **Arithmetic:** `next_beat`, `Strikes` (saturating), `next_reset`, `raid_infos`' `max(0)`, `spawn_died`'s `NEVER`,
  and populate's respawn waits.
- **The `Awake` set:** stale and repeated keys are deduplicated, and the debug builds' check against a full scan
  covers it.

### What I couldn't check
- **Rates under real load:** how often finding 1's window opens, and finding 7's counter rate. I ran no peon load.
- **Real routes and terrain for finding 8:** the extended suite wasn't run, as instructed. I didn't check whether the
  water there causes fatigue.
- **Finding 5's CPU and finding 9's lock hold times:** both are estimates; neither was measured in a release build.
- **Hangs:** RELIABILITY.md already records them as unreported, and I didn't look further.

### Verification
- The scratch tests ran with `cargo nextest run --lib rev5_` in the worktree. All 6 pass, which confirms each
  finding they cover (findings 1 to 7; 8 and 9 are by reading). Finding 1's test passed 5 runs of 5.
- No repository code was changed. The tests and the fault hook were removed afterwards, `git status` is clean, and
  nothing was committed.

## What a map sends: visibility, updates, movement (`rev5-world`)

Worktree `<worktree>/rev5-world` at master (`22def61`). Scope: `map/grid.rs`, `visibility.rs`,
`update.rs`, `fields.rs`, `movement.rs`, `player_motion.rs`, `creature_motion.rs`, `acks.rs`, the navmesh's use
(`nav.rs`, `tiles.rs`, `terrain.rs`), and the frame/flight paths those reach (`conn.rs`, `npc.rs` taxi,
`pets/possess.rs`, `tap.rs`). Findings I could not already find in `docs/CODE_REVIEW.md`, `review2`–`review4`,
`RELIABILITY.md` or `PERFORMANCE.md`. Each was proved with a scratch test (in this worktree, uncommitted, since
removed) or arithmetic over a traced path. Highest first.

---

### 1. HIGH — robustness/reliability. A client can corrupt every nearby player's packet stream with a crafted flight

**Where:** `src/game/map/npc.rs:1215` (`activate_route`), `:1031` (`flight_packet`); `src/game/update.rs:546-547`
(`frame`). Reached from `CMSG_ACTIVATETAXIEXPRESS` → `activate_express` (`npc.rs:1210`).

**What.** Two bugs compound into a remote stream-corruption that hits bystanders, not just the attacker:

1. **No cap on an express route.** `activate_route` walks the client's `nodes` list into hops, stitches every hop's
   path nodes into one `Vec<TaxiPathNode>`, and builds a single `Walk` and a single `SMSG_MONSTER_MOVE`
   (`flight_packet`) holding *all* of them. Nothing bounds the number of hops or the total node count. A client may
   send `nodes = [A, B, A, B, …]` for two adjacent nodes it has both visited; each `(A,B)` and `(B,A)` is a real
   `taxi_path`, so every hop passes the "visited"/"path exists" checks and the costs add up (saturating).
2. **The frame's size field is silently truncated.** `update::frame` computes
   `u16::try_from(body.len() + 2).unwrap_or(u16::MAX)` and writes that as the 1.12 two-byte big-endian size, then the
   writer (`conn::run_writer`) sends the *whole* body regardless. A body over 65,531 bytes is sent with a size field
   that is wrong (0xFFFF, or a wrapped value), so the client reads `size` bytes and then interprets the **rest of the
   body as the next packets' headers** — the connection is desynced from that point on (garbage opcodes, then a
   disconnect or client crash).

`flight_packet` writes every point raw (12 bytes each, no packing), so ~5,500 points overflow 64 KB. The frame is
sent with `map.send_to_observers(player, &frame, true)` — to the flier **and to everyone who sees them**, which at a
city flight master is a crowd.

**How it shows (proved).** Scratch test `tests_misc::scratch_an_express_route_back_and_forth_overflows_the_frame`:
a client sends a 400-entry express list over two 30-node hops; the server builds a flight of **11,175 points** lasting
**697 s**, and `flight_packet`'s body is **134,119 bytes** — the frame sent both to the flier and to a watching second
client. `conn::tests::scratch_a_frame_over_64k_desyncs_the_stream` builds a 70,004-byte `SMSG_MONSTER_MOVE`, reads its
header (size 0xFFFF), consumes the claimed body, and shows the *next* header the client reads is garbage (opcode 0x0,
size 11264) — the stream is out of step.
Cost in stock data is affordable: Stormwind↔Ironforge is 62 nodes/hop at 50 copper; ~180 hops (≈11,000 points) costs
under 1 gold, and both nodes are visited by any traveller. So a non-GM player at a busy flight master desyncs/crashes
every client that sees them take off.

**Fix.** (a) Cap the stitched node count (and hop count) in `activate_route`, as mangos's routes are bounded; refuse
with `NoSuchPath` past it. (b) In `update::frame` (and the hand-written `flight_packet`/`Walk::frame`/`Walk::packet`
monster-move builders), refuse to emit a frame whose body exceeds `u16::MAX - 2` — drop and log — rather than
truncate the size field; or split a long spline into several `SMSG_MONSTER_MOVE`s as mangos does. **Effort:** medium.

---

### 2. MEDIUM — reliability/robustness. A possessed unit's position is taken from the controller's client with no validation (NaN relayed to observers, saved)

**Where:** `src/game/map/pets/possess.rs:243` (`moved`), the player branch `:264` (`v.movement = *info`) and the
creature branch `:285` (`c.position = info.position`). Reached from `CMSG_*` movement while the controller is the
active mover of a Mind-Controlled player, an Eye of the Beast pet, or the Eye of Kilrogg.

**What.** `moved_self` (`movement.rs`) guards a player's own movement with `finite(&info)` (review 2 finding 3, which
added it) and the speed check. When a player's client moves a *possessed* unit, the packet is routed to
`pets::possess::moved` instead, which assigns `v.movement = *info` (player victim) or `c.position = info.position`
(creature) directly — **no `finite()` check, no speed check** — and relays it to observers with the given `relay`.
So the controller dictates the victim's position with none of the validation their own movement receives. A NaN or
20,000-yard position is taken, relayed to every client that sees the victim (the same client-crash risk review 2's
finding 3 fixed for self-movement), and written to the victim's row by `to_row`.

**How it shows (proved).** `tests_rev5_scratch::scratch_a_mind_controller_moves_the_victim_anywhere_and_to_nan`: a
priest Mind-Controls a target, sets it active mover, and sends one heartbeat 5,000 yd away and 3,000 yd under the
world — taken (`z` becomes `at.z - 3000`). A second heartbeat with `x = NaN` is relayed to a watching client as
`MSG_MOVE_HEARTBEAT` with a NaN position. (Save-through-logout of the NaN also happens but is timing-sensitive to
demonstrate, so it isn't asserted; the relay to observers is the clear harm, matching review 2's finding 3.)

**Fix.** Call `finite(&info)` at the top of `possess::moved` and drop the packet if it fails, as `moved_self` does;
consider the same speed check the controller's own movement gets (a possessed unit the server fears/confuses already
returns early). **Effort:** easy.

---

### 3. LOW–MEDIUM — correctness/robustness. The speed check pulls back server-initiated launches (knockbacks)

**Where:** `src/game/map/movement.rs:199-216` (the speed check in `moved_self`).

**What.** The allowance is `speed * speed_tolerance * (dt + latency_allowance_secs) + 3` horizontal yards, where
`speed = run.max(swim).max(7)`, with the only exemptions being GM and on-transport — **no exemption for a jumping or
falling packet.** But the server itself launches players with `SMSG_MOVE_KNOCK_BACK` (`extra_effects.rs`,
`spells/effects.rs::knock_back`) at a horizontal speed set by the spell: Wing Buffet (18500) is 250 → 25 yd/s, and
many others are 30–50 yd/s. A client flung at 25 yd/s covers ~12.5 yd between the ~500 ms heartbeats mangos/Thistle
Tea send, against an allowance of `7 * 1.25 * (0.5 + 0.5) + 3 = 11.75` yd — so the server teleports the victim back
(`teleport_within`) mid-flight, undoing a launch it sent.

**How it shows.** Arithmetic over the traced path (above). An integration scratch test reproduced the pull-back, but
its outcome depends on wall-clock `dt` between heartbeats (the known map-update race), so it isn't a stable artifact;
the per-heartbeat arithmetic is deterministic and sufficient. This is distinct from the **accepted** M1 (review 2),
which is about the check ignoring *height*; here the *horizontal* distance of a legitimate, server-sent launch
exceeds the allowance.

**Fix.** While a launch order is outstanding (the client has an unacknowledged `Order::KnockBack`, which `Outbound`
already tracks), or while the packet carries the jumping/falling-far flag, skip or widen the horizontal check for a
bounded window. **Effort:** medium (needs a tuned window so it can't be abused as a free teleport).

---

### 4. LOW — robustness. `next_creature_counter` wraps and collides after millions of spawns on a long-lived map

**Where:** `src/game/map/mod.rs` (`next_creature_counter: u32`, `+= 1` at every spawn site — ~20 of them: summons,
wild pets, totems, GM spawns, script/pool/battleground/outdoor objects), `src/game/guid.rs:39` (`make` masks the
counter to 24 bits), `src/game/map/pets/summon.rs:56` (`next_creature_counter | 0xA0_0000`).

**What.** The counter is a per-map `u32` that only ever increments while the map actor lives. `guid::make` cuts it to
24 bits for kinds that carry an entry (units, pets, game objects), and `pets::summon::spawn` ORs `0xA0_0000` on. So
pet GUIDs repeat every 2^21 (≈2.1 M) summons and creature/object GUIDs every 2^23 (≈8.4 M) spawns of the same entry.
A continent actor that lives for weeks with heavy summon/wild-pet/totem churn reaches this; the new object then shares
a GUID with a live one and overwrites it in `creature_by_guid` (the old object is stranded in the slab, its observers
and grid cell wrong, clients confused).

**How it shows (proved).** `tests_rev5_scratch::scratch_object_guids_repeat_as_the_counter_wraps_24_bits`: two pet
GUIDs 2^21 counters apart, and two unit GUIDs 2^23 apart, are equal. Reaching it needs millions of spawns in one map
uptime, so it is a weeks-of-uptime hazard, not an everyday one (and a map restart resets the counter).

**Fix.** Detect the wrap (skip a counter already live), or widen the identifier; at minimum log when the counter nears
2^21. **Effort:** medium.

---

### Checked and sound (so the next review needn't)

- **`update::frame_update` / `Batch::finish`** already cap *update-object* packets at `PACKET_LIMIT` (24 KB) and
  compress over 1 KB, and forgets go first in their own block — the oversized-frame bug (finding 1) is specific to the
  hand-built `SMSG_MONSTER_MOVE` path, not update objects.
- **`grid.rs`** clamps non-finite and out-of-world coordinates into a cell (`cell_of`), so a bad position can't panic
  cell arithmetic or (via `around`) blow up the loop (review 2 L6 was about `around`'s radius, still open but
  database-only); empty cells are pruned on remove.
- **`fields.rs`**: a GUID's two halves are always dirtied together; `is_dirty`/`below_max` shadow flags are
  `debug_assert`-checked against the real state; the public/group player masks match mangos's `updateVisualBits`;
  values blocks always carry the type word.
- **`movement::moved_self`**: `finite()` rejects NaN/≥20,000-yard world positions and bad transport offsets (review 2
  #3); the first move after login is seeded (`insert_player` sets `move_check`); dead bodies can't be walked
  horizontally; the on-transport speed-check exemption is the documented accepted gap (H1).
- **`acks.rs`**: every forced-movement ack settles only a pending order of the same unit *and* matching value
  (speed/impulse within `CLOSE`), and a possessed unit's ack settles without moving the unit.
- **`visibility.rs`**: the 10-yard margin and 8-yard (2-yard stealth) re-check thresholds; `part`/`introduce` keep
  `known`/`observers`/`forget` consistent; a flier's threshold scales with its sight. `create_spline_points` keeps a
  create's spline at ≥3 points (the 2026-10-18 client crash fix).
- **`tap.rs`**: `tell` builds at most two `Fields` clones per tapped creature per update (inside/outside views),
  not one per observer.
- **`nav.rs`/`tiles.rs`/`terrain.rs`**: background tile reads never block the map; a query over a loading/absent tile
  answers as a map with no navmesh (straight path, clear sight, unknown height), and `ready_at` guards every
  navmesh call; non-finite coordinates short-circuit to "ready" so no navmesh call sees NaN.

### Could not check

- **Real-data reachability of finding 1 at scale** (does a 180-hop Stormwind↔Ironforge express actually clear the
  client's `CMSG_ACTIVATETAXIEXPRESS` body limit and the "visited" bitmask for a fresh character?) — proved with
  crafted data and stock costs/node counts; I did not run it against the live `vmangos` taxi graph end-to-end.
- **The navmesh's own panic surface** (rustigator `find_path`/`line_of_sight` on degenerate tiles) — read the Rust
  wrappers (`nav.rs`) and rustigator's `map.rs` bounds (`MAX_PATH_HOPS`, node budgets), found them guarded, but did
  not fuzz the C++/parry3d layer.
- **Extended (real-world) tests** — not run for these findings; scope was the sending paths, provable with the fast
  suite and the harness.

## Combat, spells and auras (`rev5-combat`)

Scope: `map/combat.rs`, `map/spells/*` (with `outcome.rs`), `map/auras.rs`, `procs.rs`, `proc_scripts.rs`, threat
(`ai.rs`, `spells/threat_sync.rs`), `map/ai.rs`, `creature_combat.rs`, `event_ai.rs`, `scripts.rs`, and what they
reach where a finding led there (`pets/mod.rs`, `duel.rs`, `boss_mechanics.rs`). Detached worktree
`<worktree>/rev5-combat` at master `22def61`; line numbers are at that commit. Nothing was changed
in the repository: each proof was a scratch test in `src/game/tests_rev5_scratch.rs`, or a small program outside
the crate, deleted afterwards. The tests' code is quoted where it matters, so it can be run again.

Not repeated here: what `docs/CODE_REVIEW.md`, `review2.md` to `review4.md`, `RELIABILITY.md` and `PERFORMANCE.md`
already list. That includes R3-31 (heals, Fade and `ai::forget` looking through every creature), R3-32 (the aura
update's clones, `refresh_area_auras`), R3-16 (`apply_spell`'s stale index) and R4-39 (shifts by data values).

**About the timings.** The suite's build is a debug build with the crate at opt-level 0 (its dependencies at 2), so
the times below are many times what a release build takes. An optimized build of the crate would not fit this
agent's 8 GB: it was killed at opt-level 3, 2 and 1. So each speed finding gives:
- debug times, compared within one run (a threat list of 1 against one of 40);
- the count of calls;
- for the main one, a release model of the same work, built and run outside the crate.

### Findings

#### 1. A player pet's threat list is never pruned: whoever strikes a pet stays in combat while both live
- **Severity:** high. **Kind:** reliability (stuck state).
- **Where:**
  - `combat.rs:3036-3039`: `deal_damage_with` puts every attacker of a unit on its threat list, a pet's too.
  - `pets/mod.rs:767-773`: `on_attacked` hands a pet's fight to its own AI and returns before `ai::engage`.
  - `ai.rs:851-866`: `update_creatures` skips `c.pet.is_some()`, so a pet never runs `fight_step` or
    `prune_threat`. Nothing in `pets/ai.rs` touches `threat`.
  - `combat.rs:3497-3512`: `leave_finished_fights` holds every unit on the threat list of any living creature that
    has one, a pet included.
- **What:** a pet's list loses an entry only through `ai::forget_by` (that unit dies, feigns, vanishes or leaves the
  map) or `pet_fight_over`. So any player who damages a hunter's or warlock's pet, a totem or a charmed creature
  stays on its list. While both live, `held` contains that player, and `leave_finished_fights` never lets them out
  of combat.
  - In combat they can't log out (`login.rs:321`, `FailureInCombat`) and can't accept a summon (`misc.rs:420`).
    They can't mount, eat or drink either: mounts, Food and Drink carry `ATTR_NOT_IN_COMBAT` (`checks.rs:478`).
  - It spreads to healers. `heal_threat` (`combat.rs:2897-2918`) adds a healer to every creature, pets included,
    whose list holds the healed player and that can attack the healer. Healing such a player puts the healer on the
    pet's list too.
  - mangos-classic never has this. `Unit::CanHaveThreatList` (`Unit.cpp:8605-8630`) gives no threat list to a
    player-controlled pet (guardians aside), a player's totem, or a creature a player charms.
  - Paths:
    - **PvP** (battlegrounds, world PvP): any blow or area spell on an enemy pet.
    - **Duels:** only partly covered. `duel.rs:249-266` takes the opponent off the pet's list only if the pet is
      still attacking them when the duel ends.
- **How it shows:** scratch test `rev5_a_pet_holds_whoever_struck_it_in_combat`. Bob's blow on a tamed boar, as a
  swing deals it:

  ```rust
  on(&map, move |m| {
      combat::engage(m, gb, pet);
      combat::deal_damage(m, gb, pet, 1);
      combat::enter(m, gb, true);
  })
  .await;
  ```

  The PvP timer is 5 s and `COMBAT_CHECK` is 1.2 s. Polled each second for 9 s, the log shows the pet not attacking
  and Bob still in combat throughout:

  ```
  9s: Bob in combat true, pet's list [(Guid { guid: 2 }, 1.0)], pet attacking false
  ```

  The assertion "Bob leaves the fight once the PvP timer runs out" failed.
- **The fix:** as mangos, a creature with `owner` a player and `pet` set (not a guardian) keeps no threat list:
  - skip it in `add_threat`, `engage` and `heal_threat`;
  - clear it on charm (as now).

  Or, for the least change: leave pets out of `leave_finished_fights`' `held` set, which is what keeps the player in
  combat. Either way, a test with an enemy player and a pet, through the PvP timer.
- **Effort:** easy.

#### 2. A panic inside a death leaves its victim in `DYING` for good: unkillable at no health, on that worker thread
- **Severity:** medium. **Kind:** reliability (stuck state after a panic).
- **Where:** `combat.rs:3103-3130` (`kill_by`) and `:3132-3135` (the thread-local `DYING`). The same pattern is at
  `combat.rs:1803-1816`, where `in_extra_attack` is set around the extra swings.
- **What:** `kill_by` pushes the victim onto `DYING`, runs `kill_once`, then removes it with `retain`. A panic in
  between skips the removal. `kill_once` covers most of the game's hooks: procs, EventAI death and kill events and
  their scripts, `die_rewarding`, loot, quest credit, rewards, battleground and instance hooks, pets. The map's guards
  (`RELIABILITY.md`) catch that panic, but the victim stays in the worker thread's `DYING` until the process exits,
  and every later `kill_by` on it returns at once. Its health is already 0, so:
  - **A creature** stays alive at 0 health: no loot, no credit, never respawns. A boss so stuck blocks its encounter.
  - **A player** at 0 health can't die.

  Spawns respawn with the same GUID, and threads outlive map restarts, so neither clears it. It is R3-13 (the procs'
  depth, fixed with a guard) again, in `combat.rs`.
  - `in_extra_attack` has the same problem. A panic during an extra swing leaves it `true`, and `extra_attacks`
    (`combat.rs:2700`) then refuses every later grant. Windfury, Sword Specialization and Hand of Justice stop for
    that player until they log in again, because `leave_combat` doesn't reset the flag.
- **How it shows:** scratch test `rev5_a_panic_in_a_death_leaves_its_victim_unkillable`, current-thread runtime. A
  creature's `EVENT_DEATH` action casts (command 15) a spell whose `caster_aura_state` is 40. That panics in a debug
  build at `checks.rs:100` (`1 << (state - 1)`; see finding 8), inside `event_ai::death`, inside `kill_once`.
  - The first `deal_damage(.., 1_000_000)` panics; the query answers `None`.
  - A second `deal_damage` and a `combat::kill` follow. Then `(alive, health)` is `(true, 0)`, where `(false, 0)` was
    expected.
  - In a release build that shift doesn't panic, but any other panic under `kill_once` does the same.
- **The fix:** a guard that removes the victim on drop, as `procs::Deeper` does, so unwinding takes it off `DYING`.
  The same for `in_extra_attack`: set it back in a guard, or reset it in `leave_combat`.
- **Effort:** easy.

#### 3. Triggered spells recurse without a bound: a spell that triggers itself, or a ring of them, aborts the server
- **Severity:** medium (a bad data row kills every map; no shipped row does it). **Kind:** reliability.
- **Where:** `spells/mod.rs:1024-1036` (`cast_triggered`) → `:1176` (`execute`) → `:1643` (`land`) →
  `effects::run` → `effects.rs:1103` (`EFFECT_TRIGGER_SPELL`) → `cast_triggered`. Also `effects.rs:1796`
  (`EFFECT_TRIGGER_MISSILE`), `class_scripts::landed` and `boss_mechanics::cast_on_death`, all through `execute`.
- **What:** nothing counts how deep `execute` is.
  - Procs are limited to three deep (`procs::Deeper`), and script starts to 32 (`scripts::MAX_DEPTH`, R4-6), but a
    triggered spell neither enters a proc nor starts a script.
  - A self-targeted trigger never travels (`travels` needs another target), so nothing defers it.
  - A stack overflow is an abort, not a panic: every map goes down with what wasn't saved.
- **How it shows:** scratch test `rev5_a_chain_of_triggered_spells`, run on a 2 MiB thread (a tokio worker's stack,
  as `server.rs` builds the runtime). It casts a chain of N spells, each landing a hidden aura and triggering the
  next:
  - In debug, 20, 40 and 100 links land whole.
  - 200 links abort: `thread '<unknown>' has overflowed its stack ... SIGABRT`.
  - One spell that triggers itself (`REV5_LOOP=1`) aborts the same way.

  The shipped data, read with SQL:
  - 276 spells have trigger effects (32, 64);
  - there is no cycle;
  - the deepest chain is 20: Onyxia's Breath, 17086 → … → 17097 → 22267 → 22268 → 21132 → … → 21139.

  For comparison, a spell whose script casts it again (command 15, triggered) stops at 33 levels, and that fits:
  `rev5_a_spell_script_that_casts_its_spell` landed 33 deep and passed.
- **The fix:** a thread-local depth in `execute`, kept by a guard as `procs::Deeper` is, refusing past 32 or so and
  logging at `debug`. Optionally, refuse self-triggers at load.
- **Effort:** easy.

#### 4. A fighting creature checks every unit on its threat list two or three times an update, two of them for nothing
- **Severity:** medium. **Kind:** speed.
- **Where:** `ai.rs:1263-1278` (`fight_step`), `:1144` (`prune_threat`), `:1164` (`pick_victim`), `:1216-1261`
  (`overtake`, its `sorted` at `:1240`) and `:1120` (`valid_victim`). Also `combat.rs:816` (`can_attack`'s
  `match (regard(a, b), regard(b, a))`), `:998` (`forced_rank`) and `spells/stealth.rs:66-67`.
- **What:** each turn of an attacking creature with T on its list makes these calls to `valid_victim`:
  - `prune_threat`: T calls;
  - its current victim: 1;
  - `overtake` → `pick_victim`: T more;
  - when another unit tops the list, `sorted`: T more.

  Nothing changes between the prune and the other two passes (`overtake` takes `&Map`), so every one of their calls
  returns true. Each `valid_victim` is itself expensive:
  - a full `can_attack` of about 30 hash lookups;
  - both directions of `regard`, which a tuple evaluates eagerly. Each scans all of a player's aura effects for
    `AURA_FORCE_REACTION` (`forced_rank`), which almost no one has;
  - `stealth::sees`, two more full scans for stealth and invisibility;
  - in all, four scans of 70-odd holders for a raider, each holder a separate allocation.

  100 creatures holding 40 players come to 8,100 to 12,100 calls an update.
- **How it shows:** scratch test `rev5_bench_a_raid`:
  - 40 players, 70 auras each, 10 of them able to proc;
  - 100 creatures, each with the players on its list;
  - creatures 3 yards off, players in god mode;
  - timed inside a map query.

  | Debug build, per call | Threat list of 1 | Threat list of 40 |
  |---|---|---|
  | `ai::update_creatures` (one update's turns) | 9.6 ms | 175.6 ms |
  | `ai::overtake`, one creature | 21 µs | 1.63 ms |
  | `combat::can_attack(creature, player)` | 14.5 µs | 14.8 µs |
  | `stealth::sees(creature, player)` | 5.2 µs | 5.2 µs |
  | `Map::update`, whole | 11.3 ms | 175.2 ms |

  So 95% of the turns' time is the threat lists. A release model of `valid_victim`'s work, built with
  `-Ctarget-cpu=native`, costs 277 ns a call: 2.2 ms an update for two passes and 3.3 ms for three. It does:
  - about 35 AHashMap lookups, of 4.8 KB players and 2.5 KB creatures;
  - four scans of 70 holders of 264 B, each with a heap Vec of 72 B effects.

  The real code does more (reputation and template lookups), so 3 to 10 ms an update for one big pull is the likely
  range: 6% to 20% of the 50 ms budget. That is also the scaling limit for a large world-PvP fight against guards:
  50 guards holding 100 players make about 15,000 calls an update.
- **The fix:**
  1. After `prune_threat`, `pick_victim` and `overtake` take the list as valid (pass a flag, or pruned keys): a half
     to two thirds less.
  2. In `can_attack`, evaluate `regard(b, a)` only when `regard(a, b)` is neither hostile nor friendly.
  3. A per-unit bitset of the aura kinds present, kept where holders are added and removed. `has`, `forced_rank`
     and `sees` then return at once for a kind absent. This also speeds the 20-odd scans of every swing.
- **Effort:** easy for 1 and 2; medium for 3.

#### 5. Each proc event clones its candidate holders and works out every candidate's spell modifiers from scratch
- **Severity:** low. **Kind:** speed.
- **Where:** `procs.rs:426-547` (`run`), `:440-452` (the clones), `:511` and `:554-564` (`chance`), and
  `spells/mods.rs:144-186` (`for_spell`).
- **What:** `trigger` calls `run` for each side of every hit, tick and kill, once per target of an area spell. Each
  `run`:
  - makes two lookups for every holder (`spells.get` and `proc_events.get`);
  - clones each candidate, which is a heap allocation for its effects;
  - for each candidate that passes the filters, calls `chance` → `mods::for_spell`. That walks every holder again
    and does a `spells.get` before it looks at the effect kind. Only `AURA_ADD_FLAT_MODIFIER` and
    `AURA_ADD_PCT_MODIFIER` matter, and few holders are of those kinds.
- **How it shows:** `rev5_bench_a_raid`, `procs::trigger` for a player's melee hit, debug: 25 µs a call when the ten
  candidates' spells have no family, 159 µs when they have one (`family_name: 4`, as real talents do). Release is
  unmeasured. An area spell on 30 targets makes 30 such events.
- **The fix:**
  - keep candidates as indices or (spell, caster, item) keys;
  - in `for_spell`, test the effect kind before the spell lookup;
  - gather the owner's modifier holders once per `run`.
- **Effort:** easy.

#### 6. While any creature on the map casts, every update looks through every creature twice
- **Severity:** low. **Kind:** speed.
- **Where:** `spells/mod.rs:2648-2652` (`channels_in_reach`) and `:2696-2709` (`update`), both under
  `map.rare.casting`.
- **What:** the casting note (`PERFORMANCE.md`, item 7) skips these looks only while no creature casts. On a
  continent with players fighting, some creature is always casting. So every update runs two filters over all 23,000
  to 27,000 creatures, plus `combat::feigned`'s at each Feign Death.
- **How it shows:**
  - An extended scratch test on the real world: an idle continent has `rare.casting == false` and no creature
    casting, so idle is fine.
  - One such look over 23,345 and 27,375 creatures took 0.35 to 0.44 ms (debug).
  - At `PERFORMANCE.md`'s 15 ns a creature in release, that is about 0.8 ms an update, both looks, under load.
- **The fix:** go through `awake_where`, since a casting creature is awake or in a fight, or keep a set of the
  creatures casting, noted in `Map::caster_mut`.
- **Effort:** easy.

#### 7. `leave_finished_fights` builds a set of every fighting creature's whole threat list, nearly every update
- **Severity:** low. **Kind:** speed.
- **Where:** `combat.rs:3486-3537`, the `held` set at `:3497-3512`.
- **What:** whenever any player's `next_check` is due, the whole set is built. With 40 players on 1.2 s checks, that
  is about every update. For 100 creatures holding 40 each, it is about 4,000 AHashSet inserts to answer one or two
  players.
- **How it shows:** by count (the debug `combat::update_players` above: 9.7 µs with lists of 1, 71 µs with lists of
  40).
- **The fix:** for each due player, ask the creatures in their `attackers` and in their threat lists, or keep a
  per-player count of holders (the index R3-31 proposes would serve both).
- **Effort:** easy to medium.

#### 8. `checks::aura_state` and the form checks shift by a data value with no bound
- **Severity:** low. **Kind:** robustness.
- **Where:**
  - `spells/checks.rs:100`: `1 << (state - 1)` for `caster_aura_state` or `target_aura_state`;
  - `checks.rs:117` and `spells/mod.rs:474`: `1 << (form - 1)`.
- **What:** R4-39's class, at sites it doesn't list. A state over 32 panics in a debug build (it did, in finding 2's
  test) and tests the wrong bit in release. No shipped spell has one.
- **The fix:** `checked_shl(..).unwrap_or(0)`, as R4-39 proposes.
- **Effort:** easy.

#### 9. A chain spell runs a line-of-sight ray for every unit within a jump, before it picks the nearest
- **Severity:** low. **Kind:** speed.
- **Where:** `spells/targets.rs:520-579` (`chain`) through `units_around` (`:167-186`).
- **What:** `units_around` applies its whole predicate to every unit within the jump's radius before it sorts them:
  `can_attack_in_area` or the heal's four checks, `stealth::sees`, and `checks::sight`, a navmesh ray. A harmful
  chain then takes only the nearest. In a stacked pack of 100, Chain Lightning's three jumps are about 300 rays and
  300 `can_attack` calls. A Chain Heal in a stacked raid makes about 120 rays and takes the groups lock each jump.
  The cost of a ray wasn't measured.
- **The fix:** sort by distance first, then test the predicate lazily until one passes (harm). For heals, test sight
  only on the candidates in the heal's own order.
- **Effort:** easy.

### Checked and sound
- **Procs in procs:** `procs::Deeper` bounds them at three and restores on unwind. `target_triggers` runs only at
  depth 0 and never for its own spell. Damage shields raise no proc event, so two Thorns don't answer each other. A
  reflect is rolled once per landing, so it can't come back.
- **Script recursion:** every route to a script goes through `start_checked`'s depth of 32, spell scripts and EventAI
  actions included. A spell → script → cast loop stops at 33 levels and fits a 2 MiB stack (measured, debug).
  `scripts.depth` is reset at the update's start.
- **Other thread-locals:** `GATHERED` and `spell_areas`' depth are restored on unwind. `DYING` is the exception
  (finding 2).
- **Timers:**
  - Periodic auras clamp their interval to at least 1 ms and tick once per effect per update; nothing catches up in
    a loop.
  - Persistent areas tick at no less than 1,000 ms.
  - EventAI's `roll` guards empty ranges. A repeat with its minimum over its maximum disables the event, and 0/0
    fires once per 1 s tick. No loop catches up after a clock jump.
- **Random ranges** in scope are guarded: weapon damage, die rolls, respawn, spread, event and script picks.
- **Loops that end:** the reagent and script-command-40 removal loops end (`take_item` removes the stack or takes at
  least one; a stack of 0 is removed). Summon counts are clamped to 1–5, and a chain stops when no new unit is in
  reach.
- **Shipped data:** no trigger cycle; the deepest chain is 20. Combat dummies 4952 and 5652 are passive but immune to
  players, so they hold no one.
- **Indexing:** the `players[&..]` sites in `combat.rs` are behind `contains_key`, and `effects[a.effect]` is within
  0..3.
- **Locks:** combat takes only the groups mutex, briefly and never nested.
- **Threat, attackers and taps:** these are cleaned at death, evade and leaving the map (`take_player` → `forget`).
  Stale `attackers` entries last only until the creature's fight ends.
- **Combat logs and swings:**
  - Combat logs are encoded once and shared.
  - A swing does about 20 to 30 aura scans of the two units. At 3 to 5 swings an update, that is small next to
    finding 4.
- **Idle continents:** no creature casts, and `rare.casting` stays off (measured on both continents).

### Couldn't check
- Release timings inside the crate: three optimized builds were killed at the 8 GB cap. Finding 4's release figure is
  from a model built outside the crate. Findings 5, 6 and 9 have no release measurement.
- `DYING` on the multi-thread runtime. The test ran on a current-thread runtime. With several workers, a stuck victim
  can still die when its map runs on another worker, so it is unkillable only on the one thread.
- Whether any shipped EventAI or script sequence reaches a panic inside `kill_once` in release. Finding 2 needs some
  panic there; none was found on shipped data.
- A passive creature (template or script command 59) that no player sees gets no turn, so it doesn't prune its list
  either, and would hold players as in finding 1. That was read in the code, not tested, and is rare in shipped data:
  10 script rows set a creature passive, and the passive templates are triggers and dummies immune to players.

## Realm-wide state and its tasks (`rev5-social`)

Worktree `<worktree>/rev5-social`, detached at 22def61 (0.1.37); line numbers are at that commit.
Scope: `src/game/social/*` (channels only noted), `Realm` and its locks, the five-second sweeps (auctions, mail,
instances, battlegrounds, events, the two invasions, weather, meeting stones, honor), `champions.rs`, `cities.rs`
and `jaina.rs`, and what they reach in the maps where a finding led there. Nothing in the repository was changed.
The tests named below were scratch tests, run with `cargo nextest run --lib r5_` and then deleted from the worktree;
a copy is in [rev5-social-tests.rs](rev5-social-tests.rs) (it goes in `src/game/`, with a `mod` line in
`game/mod.rs`). All seven passed, each showing the fault it names.

Each finding: severity, kind, where, what and how it shows, the fix, its effort. Highest first.

### Findings

#### 1. A long mail subject cuts the receiver off each time they open a mailbox (high, robustness)
- **Where:** `src/game/social/mail.rs:285` (`send` keeps the subject and body as sent, `:390-391`), `mail.rs:228`
  (`list`: up to 254 mails, no size budget), `src/game/conn.rs:26` (`encode`) and `:562` (the writer).
- **What:** 1.12's server header holds a 16-bit size, and nothing refuses a message past it. wow_messages'
  `server_size()` is `size_without_header() as u16 + 4` (`traits/vanilla.rs:17`), so the header carries the size
  modulo 65,536 while the frame keeps every byte. The writer encrypts that header and writes the whole frame. The
  client reads the short size as the packet, then decrypts its next headers out of the body: the stream is lost from
  there.
- **How it shows:** `r5_a_long_subject_misframes_the_receivers_mail_list`. Alice sends Bob two mails with
  40,000-letter subjects (each fits its own `CMSG_SEND_MAIL`). Bob's `CMSG_GET_MAIL_LIST` is answered with an
  80,155-byte frame whose header says 14,619; the harness's own wow_messages read fails on it
  (`BufferSizeTooSmall`). So any player can cut off any player of their side, by name, each time that player opens a
  mailbox, for the 30 days the mails last, for 60 copper. Bob can't delete what he can't list. Other messages can
  pass 65,535 the same way:
  - a guild roster past about 680 members with full notes (96 bytes a member, `guilds.rs:492`), sent again at each
    note change (`:879`);
  - the owner list of more than 1,023 auctions (`auction.rs:750`, unpaged);
  - a `.ticket list` line of a ticket written at the packet's limit.

  Whether the client's own handler also faults on the cut list wasn't checked: `tools/client_check` has no handler
  for 0x23B.
- **Fix:** `encode` refuses (logs, returns `None`) a body over 65,533 bytes, so no message is ever misframed. Cap
  the subject at the client's 64 characters and the body at a few thousand, and list mails only while they fit, as
  mangos's `HandleGetMailList` stops at 32 KB (`MailHandler.cpp:565`). Page the owner list.
- **Effort:** easy.

#### 2. A deleted character's honor, ticket and raid binds go to the next character made after a restart (medium, reliability)
- **Where:** `src/game/store.rs:732` (the next counter is `MAX(guid)` of `characters` plus one, `game/mod.rs:1230`),
  `store.rs:1061` (a delete removes the row), `src/game/session.rs:529-547` (`delete` clears guilds, mail, auctions,
  the online friend lists and groups, and nothing else).
- **What:** what is keyed by a deleted character's counter stays:
  - its honor record (`honor.rs`, written by `pvp::sweep`, never deleted);
  - an open GM ticket (`Tickets::of`, `map/tickets.rs:82`);
  - its raid binds (`RaidSave.players`, `instances.rs:54`);
  - its place on offline characters' friend and ignore lists (`state.friends`, kept at their login because the
    name exists again, `friends.rs:113`);
  - mail it sent, which a return or an expiry sends back to it (`mail.rs:559`).

  When it was the highest counter, a restart hands it out again, and the next character made, by any account,
  inherits all of it.
- **How it shows:** `r5_a_new_character_takes_a_deleted_ones_honor`. A rank-18 character is deleted. After a restart
  the next character made has counter 1 again, and rank 18. Its fields, and the rank gear it may wear and buy, follow
  from the ledger at login. The last character made being the one deleted (a test alt) is the common case.
- **Fix:** never hand a counter out twice: keep the high-water mark in the store (a row written at each creation, by
  a new migration). Delete the character's honor record, tickets and raid places with it. Item counters are found
  the same way (the highest in bags, mail and auctions), so an item destroyed at the top is reused too; R4-22's
  leftover petitions, keyed by their charter's item, can meet one.
- **Effort:** easy (the high-water mark needs a migration).

#### 3. A refund taken before its post is written comes back after a restart (medium, reliability)
- **Where:** `mail.rs:141-154` (`post_in`); `auction.rs:527` and `:575` (`bid`), `:627` and `:638` (`cancel`),
  `:792-829` (the sweep), `:853-857` (`character_deleted`).
- **What:** `post_in` puts the mail in `Mails`, where any map can take from it, and releases the mail lock. Its
  record goes into the caller's `Records`, sent later. `take_money` and `take_item` send their save under the mail
  lock (`mail.rs:427-445`), so a take in that window is written first, and the post's older record then over it. In
  `bid` the window holds the bid notifications, a buyout's two more posts, and the bidder's `pets::sync_record` and
  `to_row` (`map/mod.rs:1260`). The outbid player, whose refund it is, is on another map, which runs in parallel.
- **How it shows:** `r5_a_refund_taken_before_its_post_is_written_comes_back_after_a_restart` does `bid`'s steps in
  that order: the refund posted, Bob takes it on his map, then the post's save. After a restart Bob has the 5,000 and
  the mail still holds them. A player can't learn the mail's id in time by hand. But ids are sequential across the
  realm (`Mails::next_id`) and taking needs no mailbox (finding 9), so a bot on the outbid side, asking for the next
  ids while its alt outbids it, can land in the window. It shows only after a restart, crash or clean. The auction
  sweep (a win with its item, an expiry) posts in the same order, with a window of a few microseconds.
- **Fix:** write the post under the mail lock: a `post_in` that takes `&mut Mails`, the auction paths taking the
  mail lock after the auctions lock (the order `character_deleted` already uses) until their `save_together` is
  sent. Or put the mail into `Mails` only after its store request has gone.
- **Effort:** easy to medium.

#### 4. One player queued alone at a meeting stone who logs out stops it grouping anyone alone (medium, reliability)
- **Where:** `src/game/map/meeting_stones.rs:207` (`leave`, documented "or gone offline", called only for
  `CMSG_MEETINGSTONE_LEAVE`, `:84`); `map/social.rs:508` (`logged_out` doesn't leave the queue); the sweep,
  `meeting_stones.rs:241-330`.
- **What:** the queue keeps the player who left. With no party queued, the longest waiting player alone leads
  (`:282`). Each player alone picked for them is taken out of `solos` and, the lead being offline, skipped (`:307`).
  So every sweep hands all the others to an offline lead and groups none of them.
- **How it shows:** `r5_one_queued_and_gone_stalls_a_meeting_stone`: a warrior, a priest and a mage queue for the
  Deadmines and the warrior logs out. After three sweeps he is still queued and the other two are not grouped. With
  all three online (`r5_control_a_meeting_stone_groups_those_online`) the first sweep groups them. It lasts until the
  one gone logs in and leaves, or a restart.
- **Fix:** `meeting_stones::leave` in `social::logged_out`, and a sweep that drops anyone offline.
- **Effort:** easy.

#### 5. R4-44, worse than written: a client can have every mail in the realm scanned 1,000 times a second (medium, speed)
- **Where:** `session.rs:1095` (`MSG_QUERY_NEXT_MAIL_TIME` in `is_query`), `mail.rs:678` (`next_mail_time`), `:112`
  (`Mails::of`), `:228` (`list`, for `CMSG_GET_MAIL_LIST`).
- **What:** R4-44 notes that `Mails::of` scans every mail under the realm's mail lock. `MSG_QUERY_NEXT_MAIL_TIME`
  runs it, and is counted among the cheap queries (`max_queries_per_sec`, 1,000 by default); `CMSG_GET_MAIL_LIST`
  runs it among the packets (100). Neither needs a mailbox.
- **How it shows:** `r5_next_mail_time_scans_the_realms_mail`: 1,000 of them over 100,000 mails take 1.51 s on the
  map's thread in a test build. A release build of the same scan (a micro-benchmark outside the repository, an
  `AHashMap` of mails shaped like wvs's) takes 288 ms for 100,000 mails and 15.5 ms for 10,000. So on a realm with
  100,000 mails one client at its query allowance takes 29% of its map's thread, and holds the mail lock, which every
  other map's mail and the sweep wait on, as long.
- **Fix:** the index R4-44 proposes (mail ids by receiver, kept up in `post_in`, `send_back` and removal). Until
  then, take `MSG_QUERY_NEXT_MAIL_TIME` out of `is_query`.
- **Effort:** easy (the rate) to medium (the index).

#### 6. A `/who` takes the guilds' lock once for each player online (low, speed)
- **Where:** `map/social.rs:173-274`: `guilds.lock().name_of` per player (`:226`); each player's name, guild and zone
  lowercased, and `words` rebuilt per player (`:239`).
- **How it shows:** `r5_who_costs_grow_with_everyone_online`: 100 `/who` with 3,000 online take 103 ms in a test
  build, about 1 ms each. `CMSG_WHO` is a packet (100 a second), so a client at its rate takes about a tenth of its
  map's thread in that build, and the guilds' lock 300,000 times a second. mangos's `HandleWhoOpcode` walks everyone
  too, so the walk is the protocol's; the lock and the allocations per player are wvs's.
- **Fix:** the guild names in one pass under one lock (or a guild id kept on `Online`), `words` once, names lowered
  once (the registry's `by_name` key already is).
- **Effort:** easy.

#### 7. A restart loses the world's running events (low, reliability)
- **Where:** `social/mod.rs:61` (`Social::load` restores guilds, mail, auctions, instances, tickets, the arena and
  honor only), `invasion.rs:116` (`State`), `elemental.rs:158`, `events.rs:119` (`force`, into `gm_events`),
  `mail.rs:75` (`Mails::load`).
- **What:**
  - the Scourge Invasion: a GM's `.invasion on`, its victories toward the 50, 100 and 150 milestones and the zones'
    rests. After a restart it is off, at 0.
  - the Elemental Invasion's stages and its two to four days' rest: it begins afresh at once (`!s.started`).
  - events a GM started or stopped follow the calendar again.
  - mail not yet due at the restart (an hour's delay) is never announced: `pending` isn't rebuilt at load, and the
    sweep announces only `pending` (`mail.rs:695`).
- **How it shows:** by reading: there is no record for any of them. Groups (R4-19) are the same, and already open.
- **Fix:** a record for the invasion's tallies, the elemental's stages and rest, and the GM's events with their end;
  `pending` rebuilt from the mails whose `deliver_at` is still ahead.
- **Effort:** easy to medium.

#### 8. What a service asks of a player's map while they change maps is lost (low, reliability)
- **Where:** `map/mod.rs:1091` (`Online::set_map` runs when the new map handles the arrival), `session.rs:296-306`
  and `:784-792` (`deliver`). The callers: `guilds.rs:240` (`set_fields`), `groups.rs:198` (`set_leader_flag`),
  `map/misc.rs:390` (a summons).
- **What:** between the old map's `take_player` and the new map's `arrive`, `o.map()` is still the old map. A call
  sent then finds no player there and is dropped.
- **How it shows:** by reading. A promotion, a demotion, a removal or a charter's founding (the signers' guild
  fields) as a member zones leaves their `PLAYER_GUILDID` or `PLAYER_GUILDRANK` as it was on their client, until the
  next change or a relog. A group's new leader keeps the old flag; a summons goes unheard.
- **Fix:** set the handle in `deliver`, as the arrival is sent. A map's queue is FIFO, so calls sent after it land
  after the arrival.
- **Effort:** easy.

#### 9. Mail is read, sent and taken without a mailbox (low, robustness)
- **Where:** `mail.rs`'s `list`, `send`, `take_money`, `take_item`, `delete`, `return_to_sender` and `make_letter`:
  none looks at the message's mailbox GUID. mangos checks `CheckMailBox` first in each (`MailHandler.cpp:98`, `:289`,
  `:319`, `:354`, `:411`, `:510`, `:545`, `:664`).
- **How it shows:** by reading; the harness's mail tests all send `mailbox: Guid::zero()`. It is what lets finding
  3's bot take from anywhere.
- **Fix:** a mailbox (a game object of type 19) within interaction range, as R4-15 did for auctions.
- **Effort:** easy.

#### 10. Two event sweeps at once can leave a map with the older events (low, reliability)
- **Where:** `events.rs:187-212` (`sweep`). It runs from the five-second sweep and from maps: each elemental invader
  slain (`elemental.rs:198`, `:207`), Omen (`omen.rs:64`, `:126`), a necropolis falling (`invasion.rs:309`), and
  `.event` or script command 53 (`events.rs:119`).
- **What:** `sweep` reads the driven events, works out the set, then takes the events lock to compare and store it,
  and fans `apply` out after releasing it. A sweep begun on older driven events can store its set after a newer one.
  The two fan-outs can also reach a map in either order, so a map can keep the older set while `social.events` holds
  the newer, and every later sweep returns early (`:194`) until some event changes. The Elemental Lords come with
  their events (72 to 75), so a map that missed one has no lord to kill, and the invasion waits at its last stage.
- **How it shows:** by reading; not reproduced (the window is microseconds).
- **Fix:** compute, compare, store and fan out under the events lock (the fan-out is channel sends).
- **Effort:** easy.

#### 11. A champion is lost until a restart if their city's map restarts during the farewell (low, reliability)
- **Where:** `champions.rs:896-897` (`Place::Returning` is realm-wide, but the time home is only in the city map's
  `Local`), `:782-788` (the only path that brings them home), `:828` (home's catch-up only keeps the home spawn
  away).
- **What:** a map restarted because it keeps panicking (RELIABILITY.md) loses its `Local`. Nothing then calls
  `bring_home`, and `call` wants `Place::Home`, so Jaina or Rexxar never come again.
- **How it shows:** by reading. The window is the farewell (1.5 s for Jaina's flash, up to 20 s for Rexxar's run).
- **Fix:** `Returning` gives way after `STALE` at home, as `Leaving` does (`:820-825`), and home brings the spawn
  back.
- **Effort:** easy.

#### 12. A guild is rewritten whole at every change (low, speed)
- **Where:** `guilds.rs:215` (`save`), from `set_note` (`:877`), `set_motd` (`:829`), `promote` (`:648`) and the
  rest.
- **What:** each note, MOTD or rank change serializes the guild with every member under the guilds' lock, and the
  store's single thread writes it.
- **How it shows:** a release micro-benchmark of the record's shape: a 1,000-member guild with full notes is 116 KB
  of JSON and takes 106 µs to encode. An officer's 100 notes a second put 11.6 MB/s through the store's thread, which
  every character's save waits behind. Each `set_note` also sends the whole roster back (finding 1).
- **Fix:** a record per member, or mark the guild changed and have the sweep write it.
- **Effort:** medium.

#### 13. Every map holds the realm's groups lock through its whole member-stats push (low, speed)
- **Where:** `map/social.rs:559-596`, on every map once a second (`map/mod.rs:1532`).
- **What:** the lock is held across every player in the map: each grouped player's stats, two `Vec`s, the
  `seen_by` copy, and a frame to each member out of sight. Everything else that takes the groups lock on any map
  (taps, group spells and totems, party chat, `instances::key_for`) waits behind it.
- **How it shows:** by reading; not measured.
- **Fix:** read the stats out of the map first, take the lock once to update `stats` and `seen_by` and decide what
  to send, and send after.
- **Effort:** easy.

#### Smaller, noted
- `auction.rs:306`: `auction_duration_in_minutes * 60` is worked out before the duration is checked. A huge value
  overflows: a debug build panics (the packet dropped, counted against the player), a release build wraps and then
  refuses it.
- `mail.rs:368`: money has no cap (mangos caps it at `MAX_MONEY_AMOUNT`), and the check above it saturates. A sender
  holding exactly `u32::MAX` who sends `u32::MAX - 10` underflows; in release they keep `u32::MAX - 19` while the mail
  carries the rest.
- `battlegrounds.rs:315-363`: `join_all` checks "already queued" under one lock and queues under another. A player's
  own join and their leader's join for the group, from two maps at once, queue them twice.
- Channels (another agent's area): `say` fans a line out to every member with the channels' and the lists' locks
  held (`channels.rs:336-376`). The lists' lock is the one every whisper, party and guild line and invite takes.

### Checked and sound
- **Lock order.** The nested pairs are channels then lists, groups then lists, guilds then lists, groups then
  instances (`group_changed`), battlegrounds then instances (a match's id), auctions then mail (a deleted seller's
  refunds), the registry's `by_account` then `online` and `names` then `by_name`, and `Maps::handles` then the config.
  None is taken in both orders, except mail then auctions in `Realm::new` (`game/mod.rs:1205`), which runs before
  the realm is shared. No lock is taken again while held: `instances::group_of` runs under neither groups nor
  instances, and `instances::reset` takes groups, then instances, one after the other. No social lock is held across
  an `.await` (parking_lot's guards aren't `Send`), across `Maps::get` or `stop`, or across a map call that waits.
- **Starting a map.** `Maps::get` holds `handles`' write lock across `Map::new`, which takes only the config's read
  lock; `world::populate` runs in the actor.
- **Auctions.** A bid, a buyout and a cancel are decided, paid for and saved under the auctions' lock, so two bidders
  on two maps can't both win, and the cut is worked out in `u64` (R4-7).
- **Mail takes.** `take_money`, `take_item`'s clearing and `make_letter` send their save under the mail lock; the
  gap between `take_item`'s clone and clear is R4-21's, still open.
- **Guilds and charters.** An invite, an accept, a signature and a turn-in are each checked and changed under one
  lock; the turn-in needs the charter held (R4-14); names, rank counts, MOTD, info, notes and rank names are bounded.
- **Groups.** `of`, `groups` and `invites` stay in step through invite, accept, decline, uninvite, leave, disband,
  and the logout of an unformed group's leader; `groups.groups[&id]` is only indexed with an id from `of`; parties
  and raids are capped at 5 and 40.
- **Instances.** A group's copies are left without an owner at load (`Instances::load`), so group ids handed out
  again after a restart don't give strangers someone's copy; `key_for` holds its lock from choosing to storing (M2).
- **Champions.** Each champion is in one place at a time, under its lock. A home map restarted while they are away,
  and a city map restarted while they are there, are both caught: the city's `watch` sees the copy gone, home keeps
  the spawn away. A call lost on the way gives way after 30 s.
- **Honor.** The ledger settles the weeks missed while the realm was down at load; a wall clock going back doesn't
  move the day back (`advance` only goes forward); honor not yet swept is written at shutdown (R4-17).
- **The sweeps.** Each runs under its own `Backoff`; the timer holds a `Weak` realm and ends with it.
- **Weather.** A GM's grade is clamped and finite; the sweep sends only zones that changed.
- **Battleground queues.** `take` keeps a party whole or leaves it; invitations lapse in the sweep and refill.

### Not checked
- What the 1.12 client's own mail-list handler does with the cut packet of finding 1 (crash or not):
  `tools/client_check` has no handler for `SMSG_MAIL_LIST_RESULT`. That the stream is lost after it follows from
  the framing.
- Finding 13's cost was not measured; finding 10's race was not reproduced.

## Start-up, data, config and the store (`rev5-data`)

Worktree `<worktree>/rev5-data`, detached at master `22def61`; line numbers are at that commit.
Scope: `src/game/data/*`, `src/config.rs`, `src/setup/*`, `src/game/store.rs`'s schema and migrations, and
`src/game/harness.rs` as far as tests reach production code. Where a finding led out of it: `game::run`'s start-up
order, `session.rs`'s character deletion, `honor.rs`'s load, `nav.rs`'s and `tiles.rs`'s tiles, `arena.rs`'s
schedule.

Nothing in the repository was changed. The proofs are scratch tests and runs, removed from the worktree afterwards;
the test file is kept at `/tmp/rev5-data/tests_rev5_scratch.rs.keep` (drop it in `src/game/` with
`#[cfg(test)] mod tests_rev5_scratch;` in `game/mod.rs`), the run scripts in `/tmp/rev5-data/run/`. The release
server was only ever run inside an isolated network namespace (`unshare -rn`), so nothing it bound was reachable.

Each finding: severity, kind, where, what, how it shows, the fix and its effort. Highest first.

### Findings

#### 1. A start on missing or broken world data wipes the skills of every character who logs in (high, reliability)
- **Where:** `src/game/mod.rs:1354-1362` (`run`: no data directory is a warning, then `GameData::empty()`);
  `src/game/data/load.rs:115-130` (`open`: an LFS pointer, or a database that won't open, is logged and skipped) and
  `:132-153` (`rows`: a table that won't read is a warning and nothing); `src/game/map/skills.rs:183-199` (`init`)
  and `:202-207` (`saved`); `src/game/map/player.rs:712` (`to_row`).
- **What:** the server starts and lets players in whatever state the data is in: no `data/db`, a `data_dir` that
  isn't there, a database that is a Git LFS pointer or won't open, tables that don't read. At login `skills::init`
  makes the skill slots from the data (`derive`: the skill effects of `spell_template`'s spells, from
  vmangos.sqlite, and `SkillLineAbility`'s lines, from dbc.sqlite), then puts each saved value back onto its slot.
  `set_value` does nothing for a skill with no slot, and `saved` lists only the slots. So on a world without the
  data the character is saved with no skills, at their logout or their next 5-minute save, and the values are gone
  from the store.
- **How it shows:** scratch test `rev5_a_login_on_missing_data_loses_the_saved_skills`. Alice's One-Handed Swords
  rise to 37 on a realm with the data. She logs in and out once on a realm over the same store with
  `GameData::empty()`, which is what `run` uses without data. Back on the data, her swords are at 1
  (`left: Some(1), right: Some(37)`). The same takes professions (their skills come from spells) and, with only
  dbc.sqlite missing, the languages and the other skills `SkillLineAbility` gives. The world is wrong too (no
  creatures without vmangos.sqlite, no factions without dbc.sqlite), but a restart on the right data mends that.
  The skills don't come back. mangos-classic refuses to start instead: a required DBC missing is `exit(1)`
  (`DBCStores.cpp:579-581`), and a world database it can't reach ends the start (`Master.cpp:357`).
- **Fix:** refuse to start (exit 2, as for a bad config) when `data_dir` is set, or `data/db` exists, and either
  database won't open, is an LFS pointer, or loads no creature templates or no spells. Keep the empty world only
  when no data was looked for and the store has no characters (tests, a bare checkout). And make `skills::saved`
  carry over what `p.row.state.skills` holds for skills the player has no slot for, so a skill the data doesn't
  know of is kept, not dropped.
- **Effort:** easy.

#### 2. A deleted character's guid goes to the next character made after a restart, with what is still kept under it (medium, reliability)
- **Where:** `src/game/store.rs:732-735` (`read_startup`: the highest guid in `characters`),
  `src/game/mod.rs:1230` (`next_character`), `src/game/session.rs:529-547` (`delete`).
- **What:** the next character guid is one past the highest in the table. Delete the newest character, restart, and
  the next character made, on any account, has its guid. Deletion cleans the mail waiting for the character, their
  auctions, their guild and group, and the friends lists of players online. It leaves their honor entry (`honor`
  records, by guid), their raid binds (`RaidSave.players`), the friends and ignore lists of players offline, and the
  mail they sent: one that expires unread is sent back to its sender's guid (`social/mail.rs:732-733`,
  `send_back`), which after the restart is the new character's, item and money included.
- **How it shows:** scratch test `rev5_a_deleted_characters_guid_is_given_again_after_a_restart`. Alice (account A,
  guid 1), with 5,000 honorable kills and highest rank 12, is deleted. After a restart Bob, on account B, is guid 1,
  and his honor entry is `(5000, 12)`: a new level 1 character with a Grand Marshal's highest rank. mangos-classic
  also takes `MAX(guid)` at start (`ObjectMgr::SetHighestGuids`), but its `Player::DeleteFromDB` deletes the friends
  rows both ways (`character_social`), the instance binds and the pets, and its highest rank and lifetime kills are
  columns of the `characters` row it deletes.
- **Fix:** never give a guid out twice: keep the high-water marks in the store (a `counters` table, migration 5,
  raised at each creation and read at start), for item guids as well. Cleaning every record kind at deletion is the
  other way, but each new kind keyed by a character would have to remember it.
- **Effort:** easy for the counter.

#### 3. A store that can't write loses every save and record while the realm plays on, and a lost record is never written again (medium, reliability)
- **Where:** `src/game/store.rs:895-931` (`write_saves`).
- **What:** a failure that isn't "busy" (a full disk, an I/O error, the file made read-only or replaced) has each
  save tried once on its own and then dropped, with an `error` line each. Nothing is kept to try again, logins go
  on, and no GM is told. A player online is saved whole at their next save, so they lose only the outage. A player
  who logs out during it loses their logout save. A record written once during it (a mail, an auction, a guild's
  change) is never written again, while the realm carries on as if it were. A `save_together` keeps the two sides
  of a move consistent with each other, but not with the later saves of the same characters. An auction bought out
  during the outage keeps its record in the store (its delete failed); the buyer takes the item from the mail once
  the disk is back, and that save succeeds. After a restart the auction is listed again: the item exists twice, and
  the seller's payment is gone.
- **How it shows:** from the code; not run (a full disk wasn't staged). RELIABILITY.md says a bad save loses only
  itself; under a failure that lasts, that is every save.
- **Fix:** keep what failed by key (a character's latest row; every record change by kind and id, in order) and
  write it with the next batch, ahead of newer saves of the same keys. While the store can't write, refuse new
  logins and tell the GMs; log once a minute, not once a save.
- **Effort:** medium.

#### 4. A record that doesn't parse is dropped at start-up and its id given out again; `.honor points nan` loses a character's honor at the next start (medium, reliability)
- **Where:** `src/game/honor.rs:133` (`Ledger::load`: `if let Ok`, without a word), `:345-347`
  (`set_rank_points`: `NaN.clamp(..)` is NaN), `src/game/map/pvp.rs:955` (`.honor points <n>`:
  `"nan".parse::<f32>()` is `Ok(NaN)`); `src/game/social/mail.rs:79-85`, `social/auction.rs:103-108`,
  `instances.rs:136-143` (`next_id` from the records that parsed).
- **What:** serde_json writes NaN as `null`, which an `f32` won't read back, so the whole honor entry fails and the
  load skips it. The character state's loader handles this (`parse_state`, `store.rs:1137`); the records' loaders
  don't. Mail, auction, guild and instance records that don't read are skipped too (some with a warning). Their ids
  aren't counted: if the record skipped was the highest, the next mail (auction, copy) takes its id, and
  `INSERT OR REPLACE` writes over it in the store, with any item it held. `max_mail_item` counts only the mail that
  parsed, so that item's guid can be handed out again as well.
- **How it shows:** scratch test `rev5_an_honor_entry_with_a_nan_is_dropped_at_load`: an entry with 5,000 kills and
  highest rank 12, given `set_rank_points(NaN)`, serialized (`"rank_points":null`) and loaded: it is gone. In the
  game, `.honor points nan` on a target marks the entry dirty, the sweep writes it, and the next start drops it.
  The records of real save files all parse today (see Checked).
- **Fix:** refuse non-finite values in `set_rank_points`, and wherever a float reaches a record. Load records field
  by field as `parse_state` does, log at `error` what doesn't parse, and leave the record in the store. Take each
  kind's `next_id` from the raw ids of `startup.records`, not the parsed ones.
- **Effort:** easy.

#### 5. On a terminal, a start without the navmesh waits for an answer, and never starts unattended (medium, reliability)
- **Where:** `src/setup/mod.rs:424-455` (`at_startup_with`), `src/setup/ui.rs:480-486` (`confirm`, no timeout),
  `src/server.rs:65` (before the accounts, the login server and the world).
- **What:** whenever the navmesh or the terrain is missing and stdin, stdout and stderr are terminals, the start
  asks "Build them now from your WoW 1.12.1 client? [Y/n]" and waits, with nothing started yet. A server run in tmux
  or screen, as private servers often are, and started again by a loop or a script after a crash, sits at the
  question until someone answers, every time, for as long as the data is missing: a "no" isn't remembered.
- **How it shows:** the release server under `script` (a pseudo-terminal), nobody typing, no navmesh or terrain:
  20 s later its output still ends at the question, with no login server and no "listening" line.
- **Fix:** remember a "no" (a note in `data/`, as `setup.json` is kept), or give the question a timeout (30 s, then
  start as it is), or ask only when started with `--setup`.
- **Effort:** easy.

#### 6. A second server on the same store migrates it and runs the realm's sweeps before it fails to bind (medium, reliability)
- **Where:** `src/game/mod.rs:1348` (`Store::open`: migrations, the bones sweep), `:1236-1237` (`Realm::new` starts
  the sweeps and the transports), `:1381-1383` (the world's bind, last, with `expect`);
  `src/game/social/mod.rs:104-120` (`tokio::time::interval` ticks at once); `src/game/instances.rs:929-960`.
- **What:** nothing stops two servers using one `characters.sqlite`. A second one (a service manager starting the
  next process before the last has saved and gone, a deploy that starts the new binary first, a second
  `cargo run`) opens the store, runs any migrations it has, deletes expired bones, loads the world, starts the
  social sweep, and only then finds port 8085 taken and panics. Its first sweep runs at once on its own copy of the
  records: it returns mail and ends auctions that came due, under mail ids the running server may have used since
  (`INSERT OR REPLACE` over theirs), and deletes the save of every dungeon copy it sees as empty (nobody is online in
  it) and untouched for 5 minutes, though the running server has players inside. A newer binary's migration changes
  the schema under the older server still writing to it.
- **How it shows:** two release servers in one network namespace on one store, the second started 6 s after the
  first. The second's log has the store opened, the world loaded, "started map 0", "9 transports running" and the
  event sweep's "event 13 (Elemental Invasion) begins", then `panic in thread main ... bind the world listener on
  0.0.0.0:8085: ... Address already in use`; its maps went on populating for 0.5 s after the panic. The harmful
  writes need records due in that second, which wasn't staged.
- **Fix:** bind the world's listener (and check the login port) before opening the store; hold an exclusive lock
  beside it (`characters.sqlite.lock`, `flock`) and refuse to start while another server holds it. And copy the
  store before migrating (`VACUUM INTO 'characters.sqlite.v{N}.bak'`), so a bad migration can be undone.
- **Effort:** easy.

#### 7. Every player in a harness test is an administrator, so most tests take the GMs' path, not the players' (medium, robustness of the tests)
- **Where:** `src/game/harness.rs:143` (`test_config`: `everyone_gm: true`), `src/game/session.rs:378-384`
  (`gm_level`), `src/game/registry.rs:75` (`gm` from the level).
- **What:** `Player::gm` is set at GM level 2 and up, and the harness makes every account level 3. A GM skips the
  reagents (`spells/checks.rs:572`, `:642`; none taken, `spells/mod.rs:1040`), the speed check
  (`movement.rs:203`), EventAI's choice of targets (`event_ai.rs:632` leaves GMs out), an area trigger's level
  (`world.rs:565`), eviction from a copy (`eviction.rs:48`), the bound-item rule of mail (`social/mail.rs:341`), and
  the faction checks of mail, groups, channels and friends. About 25 of the 482 test files turn it off, so a test
  of any of these passes a regression for players unless its author thought of it.
- **How it shows:** scratch test `rev5_the_harness_players_are_gms`: a player of `harness::realm()` is `gm` in
  their map.
- **Fix:** `everyone_gm: false` in `test_config`, with a GM realm for the tests of GM commands; or keep the command
  level apart from the bypasses (the bypasses only after `.gm on`, as mangos's GM mode). Some tests will then fail
  and need a GM or their reagents.
- **Effort:** medium, for the fallout.

#### 8. A partial bake is quiet, and a tile whose read panics is never read again (low, robustness)
- **Where:** `src/game/map/nav.rs:113-129` (`read`: a tile the header lists that is missing or doesn't decode is a
  `debug!`), `src/setup/plan.rs:362-366` (`has_navmesh`: any one map built counts), `src/game/map/tiles.rs:59`
  with `nav.rs:179-216` and `:302`.
- **What:** the start-up check asks only when no map at all has a navmesh, so one dungeon built passes for the
  continents (each continent then logs its own "no navmesh"). A bake whose header lists tiles that aren't there
  (copied in part, a disk that filled) logs each only at `debug`: creatures there walk straight lines, and nothing
  says so at `info` or above. A tile whose read panics is caught by the reader's `catch_unwind` and never sends its
  result: it stays `Loading`, `unload_idle_within` lets go only of tiles `In`, and it is never asked for again until
  the map restarts.
- **Fix:** at `Nav::open`, count the listed tiles with no file and warn once with the count; send `None` from a
  reader job that panics (a guard that sends on drop); check the continents by name at start-up.
- **Effort:** easy.

#### 9. A reload that finds no config file puts every live key back to its default (low, robustness)
- **Where:** `src/config.rs:768-771` (`load_full`: not found is the defaults), `src/game/mod.rs:1158-1166`
  (`reload_from_disk`), `:1262` (`Realm::reload`).
- **What:** at start a missing file means the defaults, which is right. At a reload it means the file has gone, or
  an editor that truncates before writing is saving it (an empty file reads as the defaults too). A SIGHUP or
  `.reload config` in that moment applies the defaults to every live key: the rates, `[gm] command_levels` (a
  command raised to administrators goes back to its own level), the guards, the MOTD, the save interval. The report
  lists them as applied.
- **Fix:** at a reload, a file not found, or empty while the running settings aren't the defaults, is an error and
  changes nothing.
- **Effort:** easy.

#### 10. `[arena]`'s times are unchecked, and a huge one panics the arena's step every update (low, robustness)
- **Where:** `src/config.rs:628-641` (`every_minutes`, `countdown_secs`, `max_minutes`, `ring_after_secs`, all
  `Any`), `src/game/map/arena.rs:598-603` and `:483`.
- **What:** `every_minutes * 60` overflows (a panic in debug, a wrap in release), and `now + every` panics on
  `Instant` overflow in release too for `every_minutes` above about 1.5e17; `countdown_secs` and `max_minutes` the
  same at a battle's start. The step is guarded, but one that panics every update has map 0 restarted within ten
  seconds (200 step panics a minute), and again after each restart. Only a typo in the config reaches it.
- **Fix:** give them `Clamp` ranges (minutes up to a week, seconds up to an hour) and add with `checked_add` where
  they meet an `Instant`.
- **Effort:** easy.

### Checked and sound
- **Start-up time and memory.** Release build, the real data, no navmesh or terrain: the data loads in 2.2 s
  (10,387 creature templates, 66,001 creature spawns, 8,847 object templates, 56,611 objects, 17,707 items, 22,381
  spells), each continent populates in 0.4 to 0.5 s, and the world listens 2.2 s after the start. Peak RSS 452 MB,
  395 MB steady. `GameData` is read-only behind an `Arc` and never copied (it doesn't derive `Clone`). The same load
  in the test profile takes 13.8 s. The navmesh's memory is PERFORMANCE.md's.
- **The loaders against the shipped databases.** With `Cols::value_ref` instrumented over a full `load`, no column
  the loaders ask for is missing, so the silent 0 for an absent column is only latent. Review 4's "no row panics the
  load" stands; a step error ends `rows`' iteration (rusqlite's `Rows` resets and stops after an error, no loop).
- **Migrations.** Each runs in one transaction with its `user_version`; a store newer than the binary is refused;
  migration 1 over existing tables fails rather than merging. `git log` of `MIGRATIONS` shows them only ever
  appended, never edited. Four real save files from earlier runs, at schemas 2 and 3 (2,553, 500, 101 and 100
  characters, with honor and arena records), migrate to 4 in 3 to 13 ms; every character reads back with its items
  and spells, and every record parses.
- **The store's order and counters.** A logout's save is queued before the map hands the player back, so a
  deletion that follows is written after it (no `INSERT OR REPLACE` bringing a deleted character back). The item
  counter counts mail and auction items (`max_mail_item`). Character creation's name check and the `NOCASE` unique
  index agree.
- **Config.** Unknown keys and variables are refused; an unreadable file is refused (review 2's fix); ranges clamp
  with a warning. A reload reads the file off the maps' threads, holds `source` across the swap (two reloads don't
  interleave), and bumps the generation after the new settings are in. Every key read from the start-up config
  (`config::config()`: the network's, `[spawn]`, `reserved_names`, `max_level`) is a restart key. A live
  `start_level` above the running `max_level` is clamped where it is used (`session.rs` `start_level`).
- **The setup.** rustigator writes every tile and header through a temporary file and a rename
  (`format::write_atomic`), so an interrupted build leaves no half file; `map_files_exist` refuses a header of
  another format, or one with no tile; the terrain count ignores the temporary names; `pe.rs` bounds-checks every
  read of `WoW.exe`.
- **A start that can't go on.** A store that won't open, accounts that won't open, a config that doesn't parse:
  each ends the start with a message (exit 2, or a panic naming the store).
- Not reported: the world's `0.0.0.0` bind is the open policy question #53.
