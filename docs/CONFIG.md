# Configuration

The server's settings are `config.toml`, in its working directory, with the environment over it. Every key has a
default in the code, so a missing file, or one that sets a single key, is fine.
[config.example.toml](../config.example.toml) has every key at its default, with its range and meaning; it is
generated, and a test fails when it drifts. Deployment settings that aren't keys stay environment variables of their
own: `WOW_REALM_ADDRESS` (the realm list's `host:port`), `WOW_TRACY` (`1` starts the Tracy profiler) and `RUST_LOG`.

## Precedence

1. The code's defaults (`src/config.rs`).
2. `config.toml`, over them.
3. The environment, over the file: `WVS__SECTION__KEY=value`, from the process or from `.env` (the process wins over
   `.env`).

`WVS__RATES__XP_KILL=2` sets `[rates] xp_kill`; `WVS__VISIBILITY__BATTLEGROUND_YARDS=400` sets `[visibility]
battleground_yards`. The section and key are case-insensitive. A value is read as its key's type: a number, `true` or
`false` (or `1`, `0`, `yes`, `no`, `on`, `off`), a string as it is (no quotes needed), and a list or table as TOML
writes it (`WVS__WORLD__RESERVED_NAMES='["Thrall", "Jaina"]'`). A key in a table goes one level further:
`WVS__ACCOUNTS__GM__ADMIN=3` and `WVS__GM__COMMAND_LEVELS__KICK=2`.

## Checks

The config is checked once, at load, and again at each reload.

- **Unknown keys are errors:** an unknown section or key in the file, and an unknown `WVS__` variable, stop the
  server with a message that names it. A value of the wrong type does too.
- **Ranges:** a number outside its key's range is clamped to the nearest end, and a warning names the key, its range
  and the value used: `[visibility] continents_yards = 5000 is outside its range, 20 to 500: using 500`.
- **Across keys:** `[players] start_level` and `[gm] start_level` are at most `[players] max_level`. A level in
  `[gm] command_levels` above 3 is refused.

## Printing it

`wow_vanilla_server --print-config` prints the effective config (the defaults, the file, the environment) as
commented TOML: every key with its value, default, range, whether a reload changes it, its mangos name and its
meaning. The output reads back to the same config. `--print-config --defaults` prints the reference, which is
`config.example.toml`:

```sh
cargo run --bin wow_vanilla_server -- --print-config --defaults > config.example.toml
```

## Hot reload

`.reload config` (GM level 3) and SIGHUP (on Unix) read `config.toml` and the environment again, off the maps'
threads. A config that doesn't load is reported and changes nothing. Otherwise:

- **What changes live:** every key marked *live* below takes its new value. These are the rates, distances, timers,
  the MOTD, new characters' settings, the battleground, anticheat and GM settings.
- **How:** the realm keeps its settings behind an atomically swapped pointer (`arc-swap`). A map takes the pointer
  once, at the start of each update (`Map::cfg`), so a reload is seen whole, between updates, never mid-step. Code
  outside a map reads `Realm::config()`, one atomic load.
- **What needs a restart:** keys marked *restart*: the network's limits, the paths, the character store, the
  accounts, the level cap, the PvP realm type, the honor week, the start position and the arena. A reload leaves
  them as they run.
- **The reply:** the command says which keys changed and which of those need a restart:
  `Config reloaded: [rates] xp_kill. Needs a restart: [world] characters_db.` SIGHUP logs the same.

## Compatibility

Keys that moved are still read at their old places, from the file and from the environment:

| Old | New |
|---|---|
| `[world] visibility_yards` | `[visibility] continents_yards` and `[visibility] instances_yards` |
| `[world] battleground_visibility_yards` | `[visibility] battleground_yards` |
| `[world] max_characters_per_account` | `[players] characters_per_realm` |
| `[world] pvp_realm` | `[players] pvp_realm` |

The old name is logged with its new one. Setting both the old and the new name is an error, since which value is
meant isn't clear. Every other key that shipped stayed where it was.

## mangos names

Where mangos-classic has the same setting (`mangosd.conf`, read in `World.cpp`), the key is mangos's name in snake
case, less what its section already says: `Rate.XP.Kill` is `[rates] xp_kill`, `Corpse.Decay.ELITE` is `[creatures]
corpse_decay_elite`, `StartPlayerLevel` is `[players] start_level`. The tables below and `config.example.toml` give
the mangos key for each. A default that differs from mangos's keeps this server's behaviour, and the key's meaning
says so (`[visibility] instances_yards` is 100 here and 170 in mangos).

## Sections and keys

*Live* keys change at a reload; *restart* keys need a restart.

### `[network]`

Connections: queues, timeouts and limits.

| Key | Meaning | Default | Range | Reload | mangos |
|---|---|---|---|---|---|
| `outbound_channel_bytes` | Bytes a client's outbound queue may hold; a client that falls further behind is disconnected, not sent a partial world. | `4194304` | 65536 to 1073741824, clamped | restart |  |
| `handshake_timeout_secs` | Seconds every read of the login and the world handshake must arrive within. | `10` | 1 to 600, clamped | restart |  |
| `client_idle_timeout_secs` | Seconds a client may send nothing before it is disconnected. Clients ping every 30. | `120` | 10 to 3600, clamped | restart |  |
| `max_pending_connections` | Connections still in the login or the world handshake, at most. | `1024` | 1 to 1000000, clamped | restart |  |
| `max_packets_per_sec` | Packets a client may send a second, sustained (three seconds' worth at once); a client that floods is disconnected. | `100` | 10 to 100000, clamped | restart |  |

### `[world]`

The world: saves, the message of the day, the character store and the data's paths.

| Key | Meaning | Default | Range | Reload | mangos |
|---|---|---|---|---|---|
| `save_interval_secs` | Seconds between saves of each online character. | `300` | 10 to 86400, clamped | live | `PlayerSave.Interval` |
| `characters_db` | The character store (with the accounts in it). | `"characters.sqlite"` |  | restart |  |
| `data_dir` | The directory with `vmangos.sqlite` and `dbc.sqlite`, as Thistle Tea exports them; `data/db` if unset and there. Without it the world is empty. | `unset` |  | restart | `DataDir` |
| `nav_dir` | A rustigator bake, the directory with `Azeroth.map`; `data/nav` if unset and there. Without it creatures walk straight lines and zones come from the client. | `unset` |  | restart |  |
| `terrain_dir` | Thistle Tea's terrain tiles (`nix run .#terrain-data`); `data/terrain` if unset and there. Without them liquids have no kind outside WMOs: no deep-sea fatigue, no lava in the open. | `unset` |  | restart |  |
| `motd` | Shown to every player at login. | `"Welcome to wow_vanilla_server."` |  | live | `Motd` |
| `reserved_names` | Names no character may take. | `[]` |  | restart |  |
| `petition_signatures` | Signatures a guild charter needs. | `9` | 0 to 9, clamped | live | `MinPetitionSigns` |
| `mail_delay_secs` | Seconds mail with money or an item takes between accounts. | `3600` | 0 to 2592000, clamped | live | `MailDeliveryDelay` |
| `honor_reset_weekday` | The weekday honor is ranked on, each week: 1 Monday to 7 Sunday (Tuesday, as Thistle Tea). | `2` | 1 to 7, clamped | restart |  |

### `[visibility]`

How far players see, in yards. Further costs the server more and the client more.

| Key | Meaning | Default | Range | Reload | mangos |
|---|---|---|---|---|---|
| `continents_yards` | On the continents. | `100.0` | 20 to 500, clamped | live | `Visibility.Distance.Continents` |
| `instances_yards` | In dungeons and raids (mangos's default is 170; here as far as the continents, as before this key). | `100.0` | 20 to 500, clamped | live | `Visibility.Distance.Instances` |
| `battleground_yards` | In battlegrounds: further, so a battle's far side is in view. | `400.0` | 20 to 500, clamped | live | `Visibility.Distance.BGArenas` |

### `[rates]`

Multipliers on what players earn, and the skill-ups' sizes and chances. 1 is 1.12's own.

| Key | Meaning | Default | Range | Reload | mangos |
|---|---|---|---|---|---|
| `xp_kill` | XP from kills. | `1.0` | 0 to 100, clamped | live | `Rate.XP.Kill` |
| `xp_quest` | XP from quests. | `1.0` | 0 to 100, clamped | live | `Rate.XP.Quest` |
| `xp_explore` | XP from discovering an area. | `1.0` | 0 to 100, clamped | live | `Rate.XP.Explore` |
| `drop_item_poor` | The drop chance of poor (grey) items. | `1.0` | 0 to 100, clamped | live | `Rate.Drop.Item.Poor` |
| `drop_item_normal` | The drop chance of normal (white) items. | `1.0` | 0 to 100, clamped | live | `Rate.Drop.Item.Normal` |
| `drop_item_uncommon` | The drop chance of uncommon (green) items. | `1.0` | 0 to 100, clamped | live | `Rate.Drop.Item.Uncommon` |
| `drop_item_rare` | The drop chance of rare (blue) items. | `1.0` | 0 to 100, clamped | live | `Rate.Drop.Item.Rare` |
| `drop_item_epic` | The drop chance of epic (purple) items. | `1.0` | 0 to 100, clamped | live | `Rate.Drop.Item.Epic` |
| `drop_item_legendary` | The drop chance of legendary (orange) items. | `1.0` | 0 to 100, clamped | live | `Rate.Drop.Item.Legendary` |
| `drop_item_artifact` | The drop chance of artifacts. | `1.0` | 0 to 100, clamped | live | `Rate.Drop.Item.Artifact` |
| `drop_item_referenced` | The chance a table's reference to another table is rolled. | `1.0` | 0 to 100, clamped | live | `Rate.Drop.Item.Referenced` |
| `drop_item_quest` | The drop chance of quest items, for those on the quest. | `1.0` | 0 to 100, clamped | live | `Rate.Drop.Item.Quest` |
| `drop_money` | Money dropped by creatures and found in chests. | `1.0` | 0 to 100, clamped | live | `Rate.Drop.Money` |
| `honor` | Honor from kills and battlegrounds (a dishonorable kill costs what it costs). | `1.0` | 0 to 100, clamped | live | `Rate.Honor` |
| `reputation_gain` | Reputation gained from kills, quests and spells (losses are as they are). | `1.0` | 0 to 100, clamped | live | `Rate.Reputation.Gain` |
| `rest_in_game` | Rested XP earned resting in game. | `1.0` | 0 to 100, clamped | live | `Rate.Rest.InGame` |
| `rest_offline_in_tavern_or_city` | Rested XP earned logged out in an inn or a city. | `1.0` | 0 to 100, clamped | live | `Rate.Rest.Offline.InTavernOrCity` |
| `rest_offline_in_wilderness` | Rested XP earned logged out elsewhere (a quarter of an inn's at 1). | `1.0` | 0 to 100, clamped | live | `Rate.Rest.Offline.InWilderness` |
| `respawn_time` | Creatures' respawn times (each at least five seconds). | `1.0` | 0 to 100, clamped | live |  |
| `skill_gain_crafting` | Points a crafting skill rises by at a skill-up. | `1` | 1 to 100, clamped | live | `SkillGain.Crafting` |
| `skill_gain_gathering` | Points a gathering skill (herbalism, mining, skinning, lockpicking) rises by at a skill-up. | `1` | 1 to 100, clamped | live | `SkillGain.Gathering` |
| `skill_gain_weapon` | Points a weapon skill rises by at a skill-up. | `1` | 1 to 100, clamped | live | `SkillGain.Weapon` |
| `skill_gain_defense` | Points defense rises by at a skill-up. | `1` | 1 to 100, clamped | live | `SkillGain.Defense` |
| `skill_chance_orange` | The chance, in percent, an orange recipe raises its skill. | `100` | 0 to 100, clamped | live | `SkillChance.Orange` |
| `skill_chance_yellow` | The chance, in percent, a yellow recipe raises its skill. | `75` | 0 to 100, clamped | live | `SkillChance.Yellow` |
| `skill_chance_green` | The chance, in percent, a green recipe raises its skill. | `25` | 0 to 100, clamped | live | `SkillChance.Green` |
| `skill_chance_grey` | The chance, in percent, a grey recipe raises its skill. | `0` | 0 to 100, clamped | live | `SkillChance.Grey` |

### `[creatures]`

Creatures: how near they notice players, how far they chase, how they call for help, how long their corpses lie.

| Key | Meaning | Default | Range | Reload | mangos |
|---|---|---|---|---|---|
| `aggro_rate` | Multiplies the distance at which creatures notice players. | `1.0` | 0 to 10, clamped | live | `Rate.Creature.Aggro` |
| `leash_radius` | Yards from where a fight began that a creature chases before it may give up (mangos's default is 30). | `50.0` | 5 to 500, clamped | live | `LeashRadius` |
| `leash_timeout_secs` | Seconds beyond its leash without a hostile act before a creature goes home. | `12` | 1 to 600, clamped | live |  |
| `unreachable_timeout_secs` | Seconds a creature tries for a victim it can't reach before it goes home. | `24` | 1 to 600, clamped | live |  |
| `assistance_radius` | Yards a creature drawn into a fight calls its kind from (mangos's default is 5). | `10.0` | 0 to 100, clamped | live | `CreatureFamilyAssistanceRadius` |
| `assistance_delay_ms` | Milliseconds the called take to come. | `1500` | 0 to 60000, clamped | live | `CreatureFamilyAssistanceDelay` |
| `flee_assistance_radius` | Yards a fleeing creature looks for help within. | `30.0` | 0 to 100, clamped | live | `CreatureFamilyFleeAssistanceRadius` |
| `corpse_decay_normal` | Seconds a normal creature's corpse lies (no longer than its respawn). | `300` | 1 to 86400, clamped | live | `Corpse.Decay.NORMAL` |
| `corpse_decay_rare` | Seconds a rare's corpse lies (mangos's default is 900). | `300` | 1 to 86400, clamped | live | `Corpse.Decay.RARE` |
| `corpse_decay_elite` | Seconds an elite's corpse lies. | `600` | 1 to 86400, clamped | live | `Corpse.Decay.ELITE` |
| `corpse_decay_rare_elite` | Seconds a rare elite's corpse lies (mangos's default is 1200). | `600` | 1 to 86400, clamped | live | `Corpse.Decay.RAREELITE` |
| `corpse_decay_world_boss` | Seconds a boss's corpse lies (mangos's default is 3600). | `600` | 1 to 86400, clamped | live | `Corpse.Decay.WORLDBOSS` |

### `[players]`

New characters, the level cap, and the realm's type.

| Key | Meaning | Default | Range | Reload | mangos |
|---|---|---|---|---|---|
| `start_level` | The level new characters start at (no higher than `max_level`). | `1` | 1 to 60, clamped | live | `StartPlayerLevel` |
| `max_level` | The highest level a player can reach. | `60` | 1 to 60, clamped | restart | `MaxPlayerLevel` |
| `start_money` | Copper new characters start with. | `0` | 0 to 2147483647, clamped | live | `StartPlayerMoney` |
| `characters_per_realm` | Characters an account may have on the realm. | `10` | 1 to 10, clamped | live | `CharactersPerRealm` |
| `pvp_realm` | A PvP realm: an account's characters are all of one side, and everyone is flagged for PvP. | `false` |  | restart | `GameType` |

### `[battleground]`

Battlegrounds' queues and matches.

| Key | Meaning | Default | Range | Reload | mangos |
|---|---|---|---|---|---|
| `start_delay_secs` | Seconds from a match's first arrival to its start. | `120` | 0 to 600, clamped | live |  |
| `min_players` | Players a side needs for a match; 0 for each battleground's own. | `0` | 0 to 40, clamped | live |  |
| `auto_leave_secs` | Seconds after a match ends before everyone is sent home. | `120` | 10 to 600, clamped | live |  |
| `invitation_secs` | Seconds an invitation to a match stands. | `120` | 10 to 600, clamped | live |  |

### `[anticheat]`

The movement check: a player who moves faster than they can is pulled back. GMs are never checked.

| Key | Meaning | Default | Range | Reload | mangos |
|---|---|---|---|---|---|
| `speed_check` | Check players' movement at all. | `true` |  | live |  |
| `speed_tolerance` | How much faster than their speed a player may seem to go. | `1.25` | 1 to 10, clamped | live |  |
| `latency_allowance_secs` | Seconds of lag a movement packet may carry. | `0.5` | 0 to 5, clamped | live |  |

### `[accounts]`

Who may log in, and who is a GM.

| Key | Meaning | Default | Range | Reload | mangos |
|---|---|---|---|---|---|
| `everyone_gm` | Every account is an administrator. For development and load tests only. | `false` |  | restart |  |
| `creation` | `auto`: an unknown name gets an account at its first login, with the name as its password (development and load tests). `existing`: only accounts made with `.account create`. | `"auto"` |  | restart |  |
| `gm` | GM levels by account: 1 moderator, 2 game master, 3 administrator, as `{ ADMIN = 3 }`. | `{}` |  | restart |  |

### `[gm]`

GM commands and GMs' characters.

| Key | Meaning | Default | Range | Reload | mangos |
|---|---|---|---|---|---|
| `start_level` | The level a GM's new characters start at, if above `[players] start_level`. | `1` | 1 to 60, clamped | live | `GM.StartLevel` |
| `command_levels` | The GM level a command needs, by its name, over the server's own (0 player to 3 administrator), as `{ kick = 2, announce = 2 }`. | `{}` |  | live |  |

### `[spawn]`

Where new characters start, instead of their race's starting zone: peon's heartbeat placement assumes Gurubashi Arena's pit.

| Key | Meaning | Default | Range | Reload | mangos |
|---|---|---|---|---|---|
| `enabled` | Start new characters here. | `false` |  | restart |  |
| `map` | The map (0 Eastern Kingdoms, 1 Kalimdor). | `0` |  | restart |  |
| `x` | Where on the map, in yards. | `0.0` |  | restart |  |
| `y` | Where on the map, in yards. | `0.0` |  | restart |  |
| `z` | Where on the map, in yards. | `0.0` |  | restart |  |
| `orientation` | Facing, in radians. | `0.0` |  | restart |  |

### `[arena]`

Gurubashi Arena's last man standing: `.arena start [countdown s] [max minutes]` starts one with everyone alive on the pit's floor.

| Key | Meaning | Default | Range | Reload | mangos |
|---|---|---|---|---|---|
| `every_minutes` | Minutes between scheduled battles; 0 for only those a GM starts. | `0` |  | restart |  |
| `countdown_secs` | Seconds of countdown before a battle. | `30` |  | restart |  |
| `ring_after_secs` | Seconds into the fight the ring of fire starts closing. | `120` |  | restart |  |
| `ring_every_secs` | Seconds between the ring's steps. | `20` | 1 to 3600, clamped | restart |  |
| `ring_step_yards` | Yards the ring closes by at each step. | `5.0` |  | restart |  |
| `ring_min_yards` | The ring's smallest radius. | `6.0` |  | restart |  |
| `max_minutes` | The longest a battle runs: then the ring is at its smallest and burns its hardest. | `20` |  | restart |  |
| `reward_copper` | The champion's prize in copper. | `250000` |  | restart |  |
| `reward_item` | The champion's item (Arena Grand Master). | `19024` |  | restart |  |
| `champion_spell` | The champion's buff (Spirit of Zandalar). | `24425` |  | restart |  |
| `min_fighters` | Fighters a battle needs to start. | `2` |  | restart |  |


## What is configurable, and what is not

These tunables were constants and are now keys, at the same defaults:

- **Rates (`[rates]`):** XP from kills, quests and exploration; drop chances by quality, of references and of quest
  items; money from creatures, chests and items; honor; reputation gains; rested XP in game and offline; creatures'
  respawn times; skill-up sizes for crafting, gathering, weapons and defense; recipes' skill-up chances by colour.
- **Visibility (`[visibility]`):** the continents', dungeons' and battlegrounds' distances.
- **Creatures (`[creatures]`):** the aggro range factor, the leash radius, the leash and unreachable timeouts, the
  call-for-help radius and delay, the fleeing creature's search radius, and the corpse decay by rank.
- **Players (`[players]`):** the start level and money, the level cap, the characters per account, the PvP realm.
- **Battlegrounds (`[battleground]`):** the start delay, the minimum players, the auto-leave and invitation times.
- **Anticheat (`[anticheat]`):** the speed check, its tolerance and its lag allowance.
- **GM (`[gm]`):** a GM's start level, and each command's GM level.

These stay constants, because they are the client's or 1.12's rules, the game's formulas, or the engine's own
tuning:

- **The client's and 1.12's rules:** packet layouts and opcodes; the XP formula, its grey levels and group bonuses;
  the honor brackets, weekly earnings and minimum kills; rest's accrual rate and pool cap (the rates scale them); the
  6-second combat and 5-minute PvP timeouts; the 5-second regeneration; chat ranges (say 25, yell 300 yards); the
  corpse reclaim delays and range; the 74-yard group reward range; the logout's 20 seconds; battlegrounds' 30-second
  resurrection waves, their buffs' and flags' timers; spell rules and the combat table.
- **The engine's tuning:** the aggro scan's half second, the leash sweep's ten, the call-for-help pulse, visibility's
  margin and move threshold (see [PERFORMANCE.md](PERFORMANCE.md)), an empty instance's unload time, the shutdown's
  wait, and the 5-second floor on a respawn.
- **Pets:** their leash and level cap follow their master's.
- **The listeners:** the login server's port 3724 and the world's 8085, on all interfaces. Making them keys is a
  change to who can reach the server, so it is left for its own decision.
