# Re-survey: creature AI, scripts and events, dungeons, raids and bosses; movement, maps, instances and transports (ai_world)

wvs at master `0c5811f4` (0.1.112), against mangos-classic `src/game` (ScriptDevAI's `scripts/`). Inputs:
`the first survey (ai_scripts)` and `the first survey (movement_maps)`. This was read only. The counts come from
`data/db/vmangos.sqlite` (opened read-only, patch 10 or lower), the Thistle Tea dump's `entries`
(`src/game/data/tt_scripts.json.gz`), every 4- or 5-digit number in non-test `src/game` code (each hit checked by
hand), and the `->Name = "..."` registrations under mangos-classic's `scripts/<continent>/<instance>/`.

Since the first survey, round 12 (`work/12a` to `12l`) has fixed the two biggest gaps in these areas: Blackwing Lair
(`map/bwl/`, all nine bosses and the instance script) and Rend's stadium in UBRS (`map/ubrs_stadium.rs`). It also
fixed most of the movement enforcement (`map/move_guard.rs`) and the fall into the void (`death::below_world`).
What is left is mostly the three later raids, plus a few map-level rules.

## Boss coverage, by instance

Each boss is put in one of three columns:
- **script**: a C++ AI that Thistle Tea ported, or hand-written Rust AI.
- **DB**: only vmangos EventAI (`creature_ai_events`) or a spell list (`creature_spells`). mangos-classic also runs
  most 5-man bosses from its DB's EventAI.
- **none**: no AI at all. The boss only melees, or never spawns.

The **mangos C++** column counts mangos-classic's `boss_*` scripts for that instance.

What counts as a boss:
- every mangos `boss_*` script, matched by `script_name` (by entry where vmangos has none);
- every rank-3 creature spawned in the map;
- every unique elite rank-1 or rank-2 spawn with loot.

Friendly NPCs, escorts, summons that belong to a boss fight and Rajaxx's captains are left out.

| Instance | bosses | script | DB | none | mangos C++ | the ones with nothing |
|---|---|---|---|---|---|---|
| Ragefire Chasm | 4 | 0 | 4 | 0 | 0 | |
| Deadmines | 8 | 0 | 8 | 0 | 1 | |
| Wailing Caverns | 7 | 0 | 5 | 2 | 0 | Kresh, Deviate Faerie Dragon (melee in mangos too) |
| Shadowfang Keep | 14 | 0 | 14 | 0 | 1 | |
| Stockade | 6 | 0 | 6 | 0 | 0 | |
| Blackfathom Deeps | 6 | 0 | 5 | 1 | 0 | Old Serra'kis |
| Gnomeregan | 5 | 1 | 4 | 0 | 1 | |
| Razorfen Kraul | 8 | 0 | 7 | 1 | 0 | Blind Hunter |
| Scarlet Monastery | 8 | 5 | 3 | 0 | 4 | |
| Razorfen Downs | 5 | 0 | 5 | 0 | 0 | |
| Uldaman | 10 | 2 | 8 | 0 | 1 | |
| Zul'Farrak | 13 | 1 | 12 | 0 | 1 | |
| Maraudon | 10 | 0 | 7 | **3** | 1 (Noxxion) | **Noxxion, Landslide, Celebras the Cursed** |
| Sunken Temple | 14 | 0 | 14 | 0 | 0 | |
| Blackrock Depths | 24 | 8 | 15 | 1 | 7 | Crest Killer |
| Blackrock Spire (LBRS + UBRS) | 22 | 8 | 11 | 3 | 3 | Spirestone Battle Lord, Spirestone Butcher, Ghok Bashguud |
| Dire Maul | 22 | 0 | 20 | **2** | 0 | **Alzzin the Wildshaper, Cho'Rush the Observer** |
| Scholomance | 14 | 8 | 5 | 1 | 2 | Vectus |
| Stratholme | 13 | 6 | 7 | 0 | 4 | |
| **5-man total** | **213** | **39** | **160** | **14** | **26** | |
| Molten Core | 10 | 9 | 1 | 0 | 10 | |
| Onyxia's Lair | 1 | 1 | 0 | 0 | 1 | |
| Blackwing Lair | 9 | **9** (new) | 0 | 0 | 9 | |
| Zul'Gurub | 10 | 0 | 1 (Thekal) | **9** | 8 | Jin'do, Mandokir, Venoxis, Mar'li, Arlokk (no spawn), Jeklik, Hakkar, Renataki (no spawn), Gahz'ranka |
| Ruins of Ahn'Qiraj | 6 | 0 | 1 (Rajaxx) | **5** | 5 | Kurinnaxx, Moam, Buru, Ayamiss, Ossirian |
| Temple of Ahn'Qiraj | 13 | 0 | 0 | **13** | 13 | Skeram, Kri, Yauj, Vem, Sartura, Fankriss, Viscidus, Huhuran, Vek'lor, Vek'nilash, Ouro (no spawn), Eye of C'Thun (no spawn), C'Thun |
| Naxxramas | 20 | 0 | 0 | **20** | 20 | all: Anub'Rekhan, Faerlina, Maexxna, Noth, Heigan, Loatheb, Razuvious, Gothik, the four Horsemen, Patchwerk, Grobbulus, Gluth, Thaddius, Feugen and Stalagg (no spawns), Sapphiron (no spawn), Kel'Thuzad |
| **Raid total** | **69** | **19** | **3** | **47** | **66** | |
| World bosses | 6 | 4 (Emerald dragons) | 2 (Azuregos, Kazzak, plus `boss_mechanics.rs`) | 0 | 6 | |

Some entries in the source match on number but give no AI. They are counted as **none**:
- Alzzin (`instance_adapters.rs` 2215, a death hook);
- Cho'Rush (`DM_TRIBUTE_GUARDS`, `DM_CHORUSH`, a list);
- Kurinnaxx (`aq20_creature`, his death brings Andorov);
- Celebras the Cursed (`instance_script.rs` 172, a door rule);
- Vectus's `10432` (a Lightning Shield rank in `procs.rs`, a false match).

Of the 66 boss scripts mangos-classic has for the raids, wvs now has 19. The 47 missing are all in ZG, AQ20, AQ40
and Naxxramas. Each of those four is a whole raid tier with no boss AI and no instance script.

---

## Naxxramas has no boss AI, no instance script, and every wing stops at its first boss
- status: open
- impact: critical
- who notices: raiders (tier 3, patch 10 content)
- mangos: `scripts/eastern_kingdoms/naxxramas/*`. It has 20 `boss_*` scripts, `instance_naxxramas` (wing doors,
  eye portals, `go_anub_door`, `go_sapphiron_birth`, Kel'Thuzad's gating), `at_naxxramas`, and the trash
  (`npc_living_poison`, `npc_web_wrap`, `npc_stoneskin_gargoyle`, `npc_diseased_maggot`).
- wvs: `instance_script::Script::by_name` still maps `instance_naxxramas` to `Unported`. None of the 20 bosses has
  AI (none is in the TT dump, none has EventAI or a spell list), so all of them only melee. Sapphiron, Feugen and
  Stalagg have no spawn. The doors flagged 0x30 stay shut, so each wing ends after its first boss.
- size: XL

## Temple of Ahn'Qiraj has no boss AI and no instance script
- status: open
- impact: critical
- who notices: raiders (AQ40)
- mangos: `scripts/kalimdor/temple_of_ahnqiraj/*`. It has 13 `boss_*` scripts (Skeram, the Bug Trio, Sartura,
  Fankriss, Viscidus, Huhuran, the Twin Emperors, Ouro, the Eye of C'Thun and C'Thun),
  `instance_temple_of_ahnqiraj`, `at_stomach_cthun`, `event_cthun_reset`, the Ouro spawners and mounds,
  `mob_anubisath_sentinel` and `mob_anubisath_defender`.
- wvs: absent (`Script::Unported`). All 13 bosses only melee. Ouro and the Eye of C'Thun never spawn, so C'Thun
  can't be fought. Trash scripts missing: `mob_qiraji_mindslayer`, `mob_vekniss_guardian`, `mob_twins_bug`,
  `mob_sartura_royal_guard`.
- size: XL

## Zul'Gurub's bosses have no AI and its instance script does nothing
- status: open
- impact: high
- who notices: raiders (the 20-man that guilds farm)
- mangos: `scripts/eastern_kingdoms/zulgurub/*`:
  - `instance_zulgurub` (Hakkar weakened by each high priest killed), `go_gong_of_bethekk`, `at_zulgurub`;
  - 8 boss scripts: Jeklik with her bat riders, Venoxis, Mar'li, Thekal with Lor'Khan and Zath, Mandokir with Ohgan,
    Arlokk, Renataki and Hakkar (Blood Siphon, Corrupted Blood).
- wvs: `Script::ZulGurub` has no arm in `instance_adapters.rs` (only `by_name` names it). Nine bosses have no AI
  (above). Thekal runs vmangos's 8 EventAI rows without the three-in-ten-seconds rule. Trash missing:
  `npc_hakkari_doctor`, `npc_gurubashi_berserker`, `npc_guru_bat_rider`.
- size: XL

## Ruins of Ahn'Qiraj: five of six bosses have no AI, and Rajaxx has no waves
- status: open
- impact: high
- who notices: raiders (AQ20)
- mangos: `scripts/kalimdor/ruins_of_ahnqiraj/*`:
  - 5 boss scripts: Kurinnaxx, Moam, Buru (with `npc_buru_egg`), Ayamiss (with `npc_hive_zara_larva`) and Ossirian
    (with `go_ossirian_crystal`);
  - `instance_ruins_of_ahnqiraj` (Rajaxx's army in waves);
  - `npc_general_andorov`, `npc_kaldorei_elite` and `mob_anubisath_guardian`.
- wvs: `instance_adapters.rs: aq20_creature` only loads Andorov's squad when Kurinnaxx dies, and steps 15471.
  Kurinnaxx, Moam, Buru, Ayamiss and Ossirian have no AI. Captain Tuubid (`boss_tuubid`) and nine AQ20 trash scripts
  are missing (`mob_qiraji_gladiator`, `mob_obsidian_destroyer`, `mob_hive_zara_*` and others).
- size: L

## AQ40's closed doors can be clicked open
- status: partly (fixed for BWL only)
- impact: medium (an exploit as long as AQ40 is unscripted)
- who notices: raiders (anyone can skip AQ40's gated rooms)
- mangos: the doors are driven by `instance_temple_of_ahnqiraj` (`DoUseOpenableObject`). The classic DB flags them
  against use.
- wvs: `world.rs: use_object` refuses only `GO_FLAG_NO_INTERACT` and `instance_script::scripted_door`. BWL's
  portcullises are now on `bwl::DOOR_ENTRIES`. AQ40's 180634 to 180636 (lock 0, no 0x10 flag) aren't on any list.
- size: S

## A raid or dungeon can be entered while a boss fight is under way
- status: open
- impact: medium
- who notices: raiders (a released ghost or a late joiner runs into a pull in progress)
- mangos: `Entities/Player.cpp: GetAreaTriggerLockStatus` returns `AREA_LOCKSTATUS_ZONE_IN_COMBAT` when
  `InstanceData::IsEncounterInProgress()`, and `SendTransferAbortedByLockStatus` sends `TRANSFER_ABORT_ZONE_IN_COMBAT`.
- wvs: absent. `instances.rs` raises only `NotFound`, `IsFull` and `TooManyInstances`, and nothing reads the copy's
  `IN_PROGRESS` fields at admission. BWL already keeps its main gate shut while a fight is on (`bwl::doors`), but
  entering is still allowed.
- size: S

## Closed doors and gates don't block line of sight
- status: open
- impact: medium
- who notices: dungeon and raid groups (casting and pulling through closed doors in BRD, Stratholme, MC and BWL);
  battleground players (casting through the starting gates)
- mangos: `Maps/Map.cpp: Map::IsInLineOfSight` checks the static vmaps and `m_dyn_tree`
  (`vmap/DynamicTree.cpp`, `GameObjectModel.cpp`). A closed door's model blocks line of sight and `GetHitPosition`.
- wvs: `spells/checks.rs: line_of_sight` and `sight` ask `nav.rs: line_of_sight` (the static bake), plus a test's
  `walls`. No game object's state is looked at.
- size: L (a first step is to test the segment against the boxes of closed door objects in the grid: M)

## Claiming to be on a transport skips the speed check (H1)
- status: open
- impact: medium (the easiest teleport hack on a live realm)
- who notices: everyone, against a cheater
- mangos: `HandleMoverRelocation` boards only a real transport with that GUID (`GetMap()->GetTransport`). The
  anticheat's `CHEAT_TYPE_TELE_TO_TRANSPORT` flags a boarding more than 100 yards from the last position.
- wvs: `movement.rs: moved_self_as` still skips `move_guard::too_fast` whenever `get_on_transport()` is set.
  `finite` bounds only the deck offset (50/100 yards). The GUID isn't checked against `Map::globals`' transports,
  nor the position against the transport's pose. DECISIONS 42 lists H1 under "Left for you, the hard ones", so it
  isn't a deliberate difference.
- size: S

## Height isn't checked, so flying, wall climbing and teleporting up all pass (M1)
- status: partly (the flags are now stripped)
- impact: medium
- who notices: everyone near a cheater; battleground flag runs
- mangos: the anticheat's speed and height tests in `Anticheat/module/Movement/movement.cpp`, including the Warsong
  Gulch heights.
- wvs: `move_guard::stripped` now takes off unearned levitate, can-fly, flying, water walking, slow fall and hover.
  `too_fast_with` measures only horizontal distance (`horizontal`). DECISIONS 73 also leaves a client that never
  acknowledges a flag's removal with that flag (no order timeout).
- size: M

## Maraudon: Noxxion, Landslide and Celebras the Cursed have no AI
- status: open
- impact: medium
- who notices: Maraudon groups (Legends of Maraudon, the Scepter of Celebras)
- mangos: `scripts/kalimdor/maraudon/boss_noxxion.cpp` (Toxic Volley, Uppercut, the summoning phase).
  vmangos's `boss_landslide` and `celebras_the_cursed` are C++ as well.
- wvs: none of the three is in the TT dump or has EventAI or a spell list. `instance_script.rs` only shows
  Celebras the Redeemed once the Cursed dies.
- size: S

## Dire Maul: Alzzin the Wildshaper and Cho'Rush the Observer have no AI
- status: open
- impact: medium
- who notices: groups in DM East (Alzzin) and DM North (Cho'Rush in the tribute run)
- mangos: no C++ for them (its DB's EventAI drives them). vmangos has `boss_alzzin_the_wildshaper` and `boss_chorush`.
- wvs: they are in Rust only as hooks: `instance_adapters.rs` `11492 => dm_set(map, 11, DONE)`, and
  `DM_TRIBUTE_GUARDS` / `DM_CHORUSH`. They have no EventAI or spells, so they only melee.
- size: S

## Warlock epic mount (Dreadsteed of Xoroth) ritual absent
- status: open
- impact: medium
- who notices: every level-60 warlock (the last step of the class quest chain)
- mangos: `scripts/kalimdor/dire_maul/dire_maul.cpp`: `event_spells_warlock_dreadsteed`, `npc_warlock_mount_ritual`
  and `go_dreadsteed_quest_objects` (candle, wheel and bell under demon waves, then Lord Hel'nurath).
- wvs: absent. Objects 179672 to 179674 and scripted events 8420 and 8428 (`scripted_event_id`) appear nowhere in
  `src/game`.
- size: M

## World and quest C++ scripts not imported
- status: partly (122 script names left, from 175)
- impact: medium
- who notices: levelling players on particular quests; Silithus and the war effort
- mangos: about 147 `npc_*`, 47 `go_*`, 25 `at_*` and 17 `event_*` scripts in `world/` and the zone files.
- wvs: of vmangos's 443 distinct creature `script_name`s, 122 spawned ones have no TT port and no reference in Rust
  (down from 175). Outside the raids, these include:
  - Silithus: `npc_colossus`, `npc_resonating_Crystal`, `npc_Geologist_Larksbane`, `npc_Krug_SkullSplit`, `npc_Shai`;
  - the war effort's soldiers: `npc_ironforge_infantry`, `npc_orgrimmar_infantry`, `npc_orgrimmar_rifleman`;
  - quest NPCs: `npc_dying_archaeologist`, `npc_mission_possible_but_not_probable`, `npc_treshala_fallowbrook`,
    `npc_sarilus_foulborne`, `npc_guard_didier`, `npc_caravan_mule`, `npc_argent_emissary`, `npc_terenthis`,
    `npc_archmage_tervosh`, `npc_lady_sylvanas_windrunner`, `npc_sirra_vonindi`, `commander_felstrom`;
  - BRD's jail NPCs: `npc_marshal_windsor`, `npc_dughal_stormwing`, `npc_tobias_seecher`, `npc_rocknot`.
- size: L (each one S to M)

## The trash scripts of the unscripted instances are missing
- status: partly (Willix's and Kernobee's escorts are in `escorts/catalog.rs`)
- impact: medium
- who notices: raid and dungeon groups
- mangos: the `mob_*` and `npc_*` scripts of each instance.
- wvs: still missing:
  - BWL: `npc_blackwing_technician`, `npc_death_talon`, `npc_death_talon_Captain`, `npc_corrupted_whelp`;
  - Stratholme: `mobs_spectral_ghostly_citizen`;
  - Scholomance: `npc_scholomance_student`, `npc_reanimated_corpse`, `npc_unstable_corpse`;
  - Uldaman: `mob_jadespine_basilisk`, `mob_annora`;
  - Dire Maul: `npc_knot_thimblejack`, `npc_gordok_brute`, `npc_arcane_aberration`, `npc_reste_mana`,
    `npc_residual_montruosity`;
  - and all of the AQ, ZG and Naxxramas trash.
- size: L

## No Warden (client integrity scans)
- status: open
- impact: medium
- who notices: everyone on a public realm (memory-edit hacks that stay inside the server-side limits)
- mangos: `Anticheat/module/Warden/` (`warden.cpp`, `wardenwin.cpp`, `wardenmac.cpp`, its scan and module managers).
- wvs: absent. `CMSG_WARDEN_DATA` is the only mangos handler opcode in these areas that wvs never names. It is only
  guarded against malformed bodies (DECISIONS 4).
- size: XL

## Relayed movement carries the mover's own client clock
- status: open (to check in the client)
- impact: medium
- who notices: every player watching others move under latency (jitter, sliding)
- mangos: `Server/WorldSession.cpp: SynchronizeMovement` relays `stime = ctime + m_clientTimeDelay +
  MOVEMENT_PACKET_TIME_DELAY`, which is server time.
- wvs: `movement.rs` relays the client's `MovementInfo` unchanged. `misc.rs` (`CMSG_MOVE_TIME_SKIPPED`) only adds
  the lag to `timestamp`.
- size: S

## Scholomance: Vectus has no AI, and Dawn's Gambit is absent
- status: open
- impact: low
- who notices: Scholomance groups; quest 4771
- mangos: `scholomance/scholomance.cpp: event_dawn_gambit`. vmangos has `boss_vectus`.
- wvs: Vectus (10432) has no AI. Neither item 12368 nor the event is handled. Door 175167 is only in an adapter's
  door list.
- size: S

## Sunken Temple: the Avatar of Hakkar event and Atal'alarion's statue order
- status: open
- impact: low
- who notices: quest 3528 (The God Hakkar); Atal'alarion's summon
- mangos: `sunken_temple.cpp`: `event_avatar_of_hakkar`, `event_antalarion_statue_activation`, `go_eternal_flame`.
- wvs: `st_set` and `st_creature` (`instance_adapters.rs`) cover Jammal'an's barrier and Eranikus only. 8443
  (the Avatar) and the statue order are nowhere in the code.
- size: M

## Blackrock Depths: the vault (relic coffers, Watchman Doomgrip) never opens
- status: open
- impact: low
- who notices: BRD groups (the Dark Coffer)
- mangos: `blackrock_depths.cpp: go_relic_coffer_door`, `instance_blackrock_depths` (`TYPE_VAULT`).
- wvs: nothing references the coffer doors 174554 to 174566 or Doomgrip (9476).
- size: S

## The AQ war effort is never run: the gates stand open from the start
- status: never implemented
- impact: low (it may be deliberate, as Thistle Tea's rule, but no document says so)
- who notices: a fresh realm's players (no resource drive, no Scarab Gong, no ten-hour war, no Scarab Lord)
- mangos: `World/WorldState.cpp` (`AQPhase`, `AQResources`, `SAVE_ID_AHN_QIRAJ`) and
  `scripts/world/war_effort.cpp` (the collectors, the gong, the war's waves).
- wvs: `events.rs: rules` always turns on 86 ("AQ War Effort (post-event)"). Events 84 and 85 never run.
- size: L

## No raid reset warnings (`SMSG_RAID_INSTANCE_MESSAGE`)
- status: open
- impact: low
- who notices: raiders
- mangos: `HandleMoveWorldportAckOpcode` calls `SendInstanceResetWarning`, and `MapPersistentStateMgr`'s scheduler
  warns players before a reset.
- wvs: absent (the opcode appears nowhere in `src`).
- size: S

## Fall damage goes by the client's falling-far flag and fall time, scaled by damage-taken auras
- status: open
- impact: low
- who notices: a cheater who never sets the flag takes no fall damage; warriors in a stance (plus or minus 10%)
- mangos: `Entities/Player.cpp: HandleFall` uses `m_lastFallZ - z` with no flag or time condition, unmodified.
- wvs: `movement.rs: landing` needs `fall_far` and `HURTING_FALL_MS` (1229). `fall_damage` applies
  `AURA_MOD_DAMAGE_PERCENT_TAKEN`. This follows Thistle Tea and vmangos.
- size: S

## A forged or wrong speed ack goes unpunished
- status: open (DECISIONS 73 names it as left)
- impact: low
- who notices: GMs and the anticheat record
- mangos: `HandleForceSpeedChangeAckOpcodes` kicks a player whose ack claims a speed higher than the server's.
- wvs: `acks.rs: speed` and `taken` drop a mismatched ack silently. It is harmless now that the speed check uses the
  server's speeds.
- size: S

## A battleground's fall below the world kills, where mangos puts the player back
- status: partly (fixed outside battlegrounds)
- impact: low
- who notices: battleground players who fall through the map
- mangos: `MovementHandler.cpp` calls `BattleGround::HandlePlayerUnderMap` in a match, which teleports the player
  to their graveyard alive. Outside a match it calls `EnvironmentalDamage(DAMAGE_FALL_TO_VOID)`.
- wvs: `death.rs: below_world` applies the void's full damage and releases the player everywhere, a match included.
- size: S

## EventAI: three of vmangos's event types aren't handled
- status: never implemented (4 rows)
- impact: low
- who notices: almost no one
- mangos: `AI/EventAI/CreatureEventAI.cpp`, e.g. `EVENT_T_TARGET_CASTING` (13).
- wvs: `data/scripts.rs` defines types 0 to 12, 14 to 18, 21 to 33 and 36. vmangos's DB uses 13 (1 row), 34 (1)
  and 35 (2), which wvs never fires. Every script command in the DB (0 to 92 and wvs's 200s) is in
  `scripts::command_supported`. Waypoints (`creature_movement`, `_template`, `_special`, `script_waypoint`),
  formations (`creature_groups`), linking, pools and game events are all loaded and run.
- size: S

## The world bosses run vmangos's EventAI plus spell mechanics, not mangos's C++
- status: open (unchanged; probably close)
- impact: low
- who notices: world-boss raiders
- mangos: `boss_azuregos.cpp`, `boss_kazzak.cpp`.
- wvs: Azuregos and Kazzak use EventAI, spell lists and `boss_mechanics.rs` (Arcane Vacuum, Shadow Bolt Whirl).
  The four Emerald dragons are Thistle Tea ports.
- size: S

Documented as by design, so skipped: line of sight and paths read as clear until a navmesh tile loads
(`AGENTS.md` "Navmesh"; `tiles::ahead` asks for tiles as a player arrives, not before the teleport).

---

## Fixed since
- Blackwing Lair: all nine bosses, Victor Nefarius and Nefarian's spawn, and the instance script with its doors
  (`map/bwl/mod.rs`, `razorgore.rs`, `vael.rs`, `chromaggus.rs`, `nefarian.rs`; `bwl::DOOR_ENTRIES`).
- UBRS: Rend's stadium (area trigger 2026, Nefarius's dialogue, the waves, Gyth summoned, Rend dropped) in
  `map/ubrs_stadium.rs`. Horde Onyxia attunement unblocked.
- BWL's clickable doors (now on `bwl::DOOR_ENTRIES` via `instance_script::scripted_door`).
- The speed check's allowance per packet: now a bank filled by time (`move_guard::too_fast_with`).
- Roots and stuns not enforced: `move_guard::held_back` refuses steering and side steps after a 1 s grace.
- Snares: the 7.0 floor is gone. The allowance uses the server's current run and swim speeds.
- Unearned movement flags (water walk, hover, levitate, fly, slow fall): `move_guard::stripped`.
- Knockbacks pulled back mid-flight: `move_guard` keeps the launch speed until landing (`launch`, `LAUNCH_LIMIT`).
- Falling into the void: `death::below_world` at `VOID_Z` (-500), with damage, release and graveyard.
- Raid encounter state saved: not a gap (BWL saves its fields and has a `demote` arm).
- Willix's and Kernobee's escorts: in `escorts/catalog.rs` (entries 4508, 7850).
