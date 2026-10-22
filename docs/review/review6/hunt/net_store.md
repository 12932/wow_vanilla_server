# Bug hunt: network, sessions and persistence (net_store)

wvs at master `0c5811f4`. Scope: `src/*.rs`, `src/bin/`, `src/setup/`, and in `src/game/`: `session.rs`, `conn.rs`,
`store.rs`, `registry.rs`, `instances.rs`, `transports.rs`, `battlegrounds.rs`, `memory_guard.rs`, `supervisor.rs`,
`mod.rs` (start-up and shutdown). Read only. I checked review 5's net and data findings against the code first, and
left out what has been fixed: one session per account, upper-case account names, a ban reaching the character screen,
the writer's stall and close, the ping rule, the store lock, the backups before a migration, the GUID counters, kept
saves and parsing records field by field.

## A rename can delete another character: `INSERT OR REPLACE` on a table with a unique name
- kind: bug
- severity: medium
- where: src/game/session.rs:933-990 (`try_rename`), src/game/store.rs:1808-1841 (`save_row`), store.rs:34 (`name TEXT NOT NULL UNIQUE COLLATE NOCASE`)
- what: `try_rename` checks `guid_by_name(&name)`, then awaits `store.load(..)`, then sends a fire-and-forget
  `store.save(row)` with the new name. The name goes into the registry only after the await. `save_row` writes the row
  with `INSERT OR REPLACE INTO characters`. When the new row breaks the `UNIQUE name` constraint, SQLite's REPLACE
  deletes the *other* row that has the name. There is no error, so the batch's fallback never runs. Two ways to hit it:
  - Session A renames to "Bob" while session B (another account) creates "Bob". B's `name_in_use` and A's
    `guid_by_name` both pass. If B's `Create` reaches the store thread before A's `Save` (the order is decided by
    when each await comes back), B is told `CharCreateSuccess`. A's save then deletes B's new `characters` row and
    leaves its `character_items` behind as orphans. The registry ends with two entries for "Bob" (`by_name` points
    at A's), and B's character is gone from its list.
  - Two flagged characters on two accounts rename to the same name at once. Both pass the check and both saves go
    through, and the second deletes the first character entirely.

  `Create` does its own name check (`SELECT 1 .. WHERE name = ?`), but `Save` doesn't. The window is one store
  round trip, which is seconds when the store's queue is deep.
- fix: give rename a store request of its own that is awaited:
  `UPDATE characters SET name = ?2, at_login = ?3 WHERE guid = ?1`. It reports a UNIQUE failure as `NameInUse`.
  Reserve the name in the registry (`add_name` with a placeholder) before the await, and take it back on failure.
  Also make `save_row` `INSERT .. ON CONFLICT(guid) DO UPDATE SET ..`. Then a name clash from any path fails that
  one save, which gets logged, instead of silently deleting someone else's row. The same applies to
  `character_items`' `INSERT OR REPLACE`: an item GUID saved under two owners moves without a word.
- fix size: small
- confidence: confirmed (by reading; SQLite's REPLACE conflict resolution deletes the rows that conflict)

## Accounts: a failed read counts as "no such account". Under `auto` that resets the password to the name, and a ban fails open
- kind: bug
- severity: medium
- where: src/accounts.rs:141-162 (`find`: `.optional().ok().flatten()`), :234-246 (`login`), :165-183 (`set_password`: `ON CONFLICT(name) DO UPDATE SET salt, verifier`), :267-269 (`banned`)
- what: `find` maps every SQLite error to `None`: busy after the 5 s timeout, I/O, or a column that won't convert
  (a `gm_level` over 255 put in by hand fails `u8`). `login` under `Creation::Auto` treats `None` as a new name and
  calls `set_password(name, name)`, which is an upsert. So an existing account (an administrator made with
  `.account create`) gets its salt and verifier replaced, and its password becomes its name. From then on anyone can
  log in as it. The same `None` makes `banned()` false, so a ban is skipped in `auth::banned` (reconnect, world
  handshake, `login`), and `gm_level()` returns PLAYER. Making the account at the logon challenge, before any proof,
  was already noted (rev5-net #7). The upsert is what turns a transient read error into a takeover.
- fix: `find` returns `rusqlite::Result<Option<Account>>`, and `login` refuses on `Err`. Auto creation uses
  `INSERT .. ON CONFLICT(name) DO NOTHING` and then reads the row again, so it never overwrites. `banned` fails
  closed on an error.
- fix size: trivial
- confidence: likely (the code path is certain; the trigger is a rare database error or a hand-edited row)

## Frames up to 65,531 bytes reach the client, whose hard cap is 32,766
- kind: robustness
- severity: medium
- where: src/game/conn.rs:28-40 (`encode`), src/game/update.rs:732 (`MAX_FRAME_BODY = u16::MAX - 4`), used by `Body::frame`
- what: the only size guard on everything sent is the 16-bit size field. mangos-classic's `Common.h:109` says
  `MAX_NETCLIENT_PACKET_SIZE (32767 - 1) // Client hardcap: int16 with trailing zero space otherwise crash on
  memory free`. rev5-net noted this. The mail list got a local cap (`social/mail.rs` `LIST_BYTES = 32_766`), but the
  guard in `encode` and `update::frame` is still 65,531. Concrete case: `channels::list` (`CMSG_CHANNEL_LIST`, 9
  bytes a member, no cap) builds 32+ KB for a channel of about 3,640 members (World or LookingForGroup on a full
  realm), and it is sent. Every list built by hand without a cap is exposed the same way.
- fix: `MAX_FRAME_BODY = 32_766 - 4` in `update.rs` (update batches stop at 24 KB anyway), so `encode` and
  `Body::frame` refuse anything larger and log it. Then cap the lists that can grow (channel members, as mangos's
  `Channel::List` does by its size) the way the mail list is capped. Run `tools/client_check` on a 33 KB frame to
  confirm the crash.
- fix size: small
- confidence: likely (wvs's bound confirmed; the crash rests on mangos's comment)

## The memory guard can stop every client for good, with nothing that brings it back
- kind: robustness
- severity: medium
- where: src/game/memory_guard.rs:87-127, src/game/session.rs:335-343 (`inbound.recv(), if !held`)
- what: past `memory_guard_fraction` (0.8 by default) of RAM, or of the cgroup's limit, every session stops reading
  its client until RSS falls a twentieth under. That only helps when queues are what fills memory. When the steady
  state is over the line (a container with a 1 to 1.5 GB limit against about 450 MB of game data, navmesh tiles and
  the allocator's retained pages), RSS never drops. The guard stays on and the whole realm freezes:
  - no packet reaches a map;
  - pings are still answered by the reader until the 256-slot inbound queue fills, then not even those;
  - clients see a frozen world, and eventually disconnect.

  The only trace is one `warn` line. Nothing bounds how long the guard stays on, and nothing checks whether reading
  less can lower memory at all.
- fix: engage only when the bytes that reading less can drain (the sum of `queued_bytes` and the mailboxes) are a
  real share of RSS, or cap a continuous hold (say 30 s), after which the guard logs at `error`, lets go, and backs
  off. At start-up, warn when the baseline RSS is already near the limit.
- fix size: small
- confidence: likely

## Instance and raid records are sent to the store after their lock is let go, so two threads' writes can land out of order
- kind: bug
- severity: low
- where: src/game/instances.rs:824-845 (`change_save`), :572-600 (`entered`), :651-687 (`adopt`), :691-707 (`orphan`), :1185-1227 (`expire`)
- what: each function clones the `InstanceSave` under `social.instances`, drops the lock, then calls
  `store.put_record`. The writers run on different threads:
  - `entered` runs on the entering player's map thread (`key_for`, `resume`);
  - `spawn_died` and `data_changed` run on the copy's own map thread.

  Thread A clones S1 (a player entering). Thread B adds a dead boss, clones S2, and sends it. A then sends S1, and
  the store keeps S1, without the kill. Until the copy's next change rewrites it whole, a restart brings the boss
  back (or loses an `entered` or an owner). The same pattern exists between `bind_raid` and `entered` for `KIND_RAID`.
- fix: send the record while the lock is still held. `Store::send` is an unbounded channel's non-blocking send, so
  it's cheap. Or stamp each save with a version and write only a newer one.
- fix size: trivial
- confidence: confirmed (by reading; the window is microseconds, and a later write of the same copy heals it)

## Battleground copies write an instance save for every creature or object that dies or goes
- kind: robustness
- severity: low
- where: src/game/battlegrounds.rs:640 (a match's id from `instances.new_id()`), src/game/map/ai.rs:570, map/objects.rs:468, src/game/instances.rs:824-845 (`change_save`'s `or_insert_with`)
- what: a battleground copy's key has `instance != 0`, so `spawn_died` and `object_gone` go to `change_save`. That
  makes up an `InstanceSave` (owner `None`, map 30, 489 or 529) and `put_record`s it, again at every death. In
  Alterac Valley that is hundreds of record writes a match into the store's one thread, plus a record per match. A
  match ends by `maps.stop`, not by `unloaded`, so the records stay until the sweep removes them (30 minutes later)
  and survive a restart meanwhile. A boss there (`rank == 3`) is saved as `NEVER`.
- fix: in `spawn_died`, `spawn_back`, `object_gone`, `object_back`, `touch` and `change_save`, do nothing for a key
  whose map is a battleground (`battlegrounds::bg_of_map(key.map).is_some()`).
- fix size: trivial
- confidence: confirmed

## A new character's name entry is at level 1 whatever level it starts at
- kind: bug
- severity: low
- where: src/game/session.rs:879-887 (`level: 1.into()`), src/game/registry.rs:71 (`Online::new` copies it), src/game/map/social.rs:232 (`/who`), guilds.rs:544, insignia.rs:110
- what: with `[players] start_level` or `[gm] start_level` above 1, the row is made at that level, but its
  `NameEntry` says 1. Nothing at login sets the registry's level from the row; only a level-up, `.levelup` or a
  restart does. Until then `/who` (filtering and display), the guild roster and insignia scoring use level 1.
- fix: `level: row.level.into()`, taken before `row` moves into `store.create`. Better still, have the map's login
  call `registry.set_level(g, row.level)`.
- fix size: trivial
- confidence: confirmed

## `route_of` builds every transport's route, splines and all, on each call, on the map's thread
- kind: perf
- severity: low
- where: src/game/transports.rs:533-563 (`routes`, `route_of`), called from :583 (`aboard_at_login`), :601 (`place_aboard`), :779 (`settle_aboard`), :700 (`restore`)
- what: `route_of` is `routes(realm).into_iter().find(..)`, and `routes_in` runs `Route::build` (taxi nodes,
  spline segments and their lengths) for all nine boats and zeppelins each time. `settle_aboard` runs on the map's
  thread for every passenger's worldport acknowledgement. A full ship crossing means a full rebuild per passenger,
  and so does every login aboard. The routes never change once the data is loaded.
- fix: build them once (a `OnceLock<Vec<Route>>` in `Realm`, or a lazily built field in `GameData`) and return
  `&Route`.
- fix size: trivial
- confidence: confirmed

## Saves kept while the store can't write are bounded by count, not bytes, and a save together is never merged
- kind: robustness
- severity: low
- where: src/game/store.rs:1294-1320 (`MAX_KEPT = 100_000`, `keep`), :1274-1287 (`key`: `Together` has no key), :1358-1364 (`kept_row`)
- what: during a long outage every save is kept. A `Save` is merged with later saves of the same character, but a
  `Together` (trade, mail, auction) is kept whole every time, with *full* `CharacterRow`s (items, spells, state
  JSON, often tens of KB) for both sides. An auction house in use during an outage of an hour or more can keep
  gigabytes before the count cap applies. Each failed batch also rebuilds `pending` with a fresh hash set, and each
  login, list or load walks all of it (`kept_row`).
- fix: bound the kept saves by bytes (an estimate per row), and once a newer `Save` or `Together` holds a character,
  drop that character's rows from the older `Together`s (keep their record ops). Then log at `error` once the
  bound is near.
- fix size: small
- confidence: confirmed (by reading; no outage was staged)

## A character's delete is four statements outside a transaction, and leaves kept saves and caches behind
- kind: bug
- severity: low
- where: src/game/store.rs:1556-1580 (`Request::Delete`), session.rs:896-921
- what:
  - the `characters` row, then the items, spells and actions, are deleted in autocommit. A failure after the first
    (a full disk, an I/O error) leaves the row gone and the items orphaned. The client is told
    `CharDeleteFailed`, and the name stays in the registry until a restart.
  - a `Save` of that character still in `kept.pending` (its logout during an outage, if the small `DELETE` gets
    through while the big batch doesn't) is written afterwards and brings the character back (suspicion).
  - the character's `account_data` rows (`ACCOUNT#guid`) are never deleted.
- fix: one transaction (`unchecked_transaction`) around the four deletes and `DELETE FROM account_data WHERE
  account = ?`. In `serve(Delete)`, drop `kept.pending` saves and `Together` rows of that guid once it has deleted.
- fix size: trivial
- confidence: confirmed (the first and third); suspicion (the second)

## The login server's other protocol versions are leftovers: a TBC realm list of nine made-up realms, and no word for unknown ones
- kind: improvement
- severity: low
- where: src/auth.rs:762-789 (`print_version_8_realm_list`), :311-326 (`_ => {}`), :740-753, auth.rs:500-730
- what:
  - protocol 8 (2.x) gets nine realms named "0".."8", all at the world's address, which is a debugging leftover.
  - an unknown protocol version is closed without an answer, and neither the logon challenge nor
    `CMSG_AUTH_SESSION` checks the client's build (rev5-net "Smaller, noted"; still open). A non-1.12 client is
    let in and sent 1.12 packets.
  - the 1.12 list says `number_of_characters_on_realm: 0` (mangos sends the account's count).
  - `login_version_2`, `_3` and `_8` and the two reconnect versions are near copies (about 300 lines), so a fix to
    one (the name's case, the ban) has to be made several times.
- fix: refuse builds other than 5875 at the challenge (`FailVersionInvalid`) and in the handshake; drop protocol 8
  or give it the one realm; send the character count; fold the copies into one generic function over the versions'
  types, or a macro.
- fix size: small
- confidence: confirmed

## The login server's restart backoff never resets
- kind: robustness
- severity: low
- where: src/game/supervisor.rs:93-114 (`supervise`)
- what: `rest` doubles on each panic, up to 60 s, and is never cleared after a run that lasted. After six panics
  over a server's life (days apart), every later restart of "the login server" waits a full minute, and nobody can
  log in meanwhile. `Backoff` (the step version) does clear after a good run; `supervise` doesn't.
- fix: reset `rest` to zero when the task ran longer than, say, `Backoff::MAX` before it panicked.
- fix size: trivial
- confidence: confirmed

## The ping rule disconnects GMs, which mangos exempts
- kind: improvement
- severity: low
- where: src/game/conn.rs:1525-1529, :1706-1736 (`PingRate`)
- what: mangos's `WorldSocket::HandlePing` (WorldSocket.cpp:600-608) kicks for overspeed pings only when
  `GetSecurity() == SEC_PLAYER`. wvs's reader kicks anyone. GM tools and test clients that ping faster are cut off.
- fix: give the reader the account's GM level (it is known after the handshake) and skip the kick above PLAYER,
  as the session's cooldowns already do.
- fix size: trivial
- confidence: confirmed

## The transports' clock is a `u32` of milliseconds and wraps after 49.7 days
- kind: robustness
- severity: low
- where: src/game/mod.rs:1976-1984 (`uptime_ms`: `as_millis() as u32`), transports.rs:407 (`ms % self.period`), :652-697
- what: after 2^32 ms of uptime the clock goes back to 0. Each ship jumps to `0 % period` along its route; one that
  ends up on its other map makes the clock call `cross` and teleports its passengers mid-sea. mangos's `getMSTime()`
  wraps the same way, so this matches 1.12 servers; it only matters on servers that run that long.
- fix: keep the transports' time as `u64` and take `% period` from that. Only the client-facing values need to stay
  `u32`.
- fix size: trivial
- confidence: confirmed
