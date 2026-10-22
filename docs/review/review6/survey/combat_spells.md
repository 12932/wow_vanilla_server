# Re-survey: combat, spells and auras (wvs master at 0c5811f4 against mangos-classic)

Sources re-checked: `the first survey (combat)`, `spells.md` and `auras.md`. Round 12 (work/12e, 12f and 12g, and calls12) fixed most
of them. I then looked again for anything never implemented:
- a coverage check of every `SPELL_EFFECT_*` and `SPELL_AURA_*` mangos handles, weighted by use in the 1.12 data (22,381 spells
  at build 5875 or earlier: 8,371 player, 1,893 creature and 5,210 item spells, with what they trigger);
- every client opcode in these areas, and every server message mangos sends in them;
- mangos's class scripts and `case <id>:` scripts, checked against wvs's source.

**Coverage, in short.** Every effect mangos really handles has a working handler in wvs, effect 92 included. Every aura type it
handles that 1.12 data uses has a reader: only 152 (`MOD_DETECTED_RANGE`) and 178 (`MOD_DEBUFF_RESISTANCE`) have none, and no
1.12 spell uses either. Every client opcode mangos really handles in these areas is handled. What remains is rule details,
per-spell scripts inside handled types, and a few server messages.

## Persistent area spells tick through walls
- status: partly (round 12 added line of sight for a cast's area targets, but not for a persistent area's ticks)
- impact: high
- who notices: everyone, in PvP and PvE. Blizzard, Rain of Fire, Hurricane, Volley, Consecration and Flamestrike's burn hit and
  pull units round corners and through walls. In battlegrounds, pillars give no cover from them.
- mangos: `Grids/GridNotifiersImpl.h` `DynamicObjectUpdater::VisitHelper`, line 264: `if (!HasAttribute(SPELL_ATTR_EX2_IGNORE_LINE_OF_SIGHT) && !i_dynobject.IsWithinLOSInMap(target)) return;`.
  A dynamic object's tick hits only the units it can see.
- wvs: `map/spells/targets.rs` `area_sight` says outright that "a persistent area ... isn't tested as it ticks, as mangos's
  `DynamicObjectUpdater` doesn't". That misreads mangos. `effects.rs` `tick_areas` has no line-of-sight test.
- size: S (the same `checks::sight` test from the dynamic object's position; on a big pack, it costs navmesh queries every tick)

## Proc charges are spent when mangos's per-aura conditions say they shouldn't be
- status: open
- impact: medium
- who notices:
  - priests: Inner Fire loses a charge on a hit that is fully absorbed;
  - users of Hand of Edward the Odd (Focus 18803), which an instant cast eats;
  - users of Insight (18820), which a free spell eats;
  - Fury warriors and Enhancement shamans: a Cleave's or Sweeping Strikes' second target spends Flurry;
  - spell reflection of one school is spent by a spell of another school.
- mangos: `UnitAuraProcHandler.cpp`. These handlers return `SPELL_AURA_PROC_FAILED` (no charge spent) when their condition
  isn't met:
  - `HandleModCastingSpeedNotStackAuraProc` (aura 65): instant casts;
  - `HandleModPowerCostSchoolAuraProc` (72 and 73): free spells and wrong schools;
  - `HandleReflectSpellsSchoolAuraProc` (74): wrong schools;
  - `HandleModResistanceAuraProc` (22): no damage taken;
  - `HandleHasteAuraProc` (138): any target but the main one.
- wvs: `map/procs.rs` `run` spends a charge whenever the proc flags and `spell_proc_event` match
  (`fired || answered || h.charges > 0 && !answers_alone`). There is no check on the aura's kind.
- size: S

## Resistances: the low-level formula, DoTs and full resists differ from mangos
- status: open
- impact: medium
- who notices: casters against resistant mobs and players, low-level characters against elementals, PvP players with
  resistance gear
- mangos:
  - `Unit::CalculateEffectiveMagicResistancePercent` divides resistance by `max(attacker level*5, 100)`;
  - `IsBinarySpell` makes pure-aura DoTs binary;
  - `CalculateSpellResistChance` lets players' and pets' spells resist fully in the 100% bucket;
  - NPC spells over 54% resistance get an extra bucket.
- wvs: `map/spells/outcome.rs`:
  - `resist_chance` is `r*0.15/caster_level` with no 100 floor, so below level 20 each point counts double;
  - `binary` covers only control and debuff auras, so Corruption, Shadow Word: Pain and similar DoTs are never resisted whole;
  - `resist_fraction` folds the 100% bucket into 75% and has no NPC cut-off.
- size: M

## Damage shields can't miss, be resisted, or meet immunity
- status: open
- impact: medium
- who notices:
  - druids (Thorns), shamans (Lightning Shield) and Retribution paladins;
  - raids: fire-shield items and auras hurt Molten Core's fire-immune mobs;
  - PvP: nature resistance never resists Thorns.
- mangos: `Unit::DealMeleeDamage` runs each damage shield through `SpellHitResult` (a miss is sent with `SendSpellMiss`). It also
  checks `IsImmuneToDamage` and applies `SPELLMOD_DAMAGE`.
- wvs: `map/procs.rs` `damage_shields` sends `SMSG_SPELLDAMAGESHIELD` for the full amount, then calls
  `combat::deal_damage_school`. There is no hit roll, no resistance and no immunity check.
- size: S

## Creatures don't react to a stealthed player at the edge of detection
- status: never implemented
- impact: medium
- who notices: rogues and Prowling druids. mangos's mobs turn to the player and show the "?" alert before they detect them;
  wvs's mobs give no warning.
- mangos: `AI/BaseAI/UnitAI.cpp` `MoveInLineOfSight` with `CanTriggerStealthAlert`. The creature sends
  `SendAIReaction(AI_REACTION_ALERT)`, turns to face the player, pauses through `MoveDistract(TIME_INTERVAL_LOOK)` and calls
  `OnStealthAlert`.
- wvs: absent (`ai.rs` and `creature_motion.rs`; `heeds_distraction` is used only by Distract).
- size: S-M

## Loatheb's Corrupted Mind doesn't stop healing
- status: never implemented (an encounter rule, shared with the raid survey)
- impact: medium for Naxxramas raiders
- who notices: Naxxramas raids. Healers heal freely through Loatheb, which removes the fight's point.
- mangos: `Spell::CheckCast`, class script 4327 (29184, 29195). Heals, HoTs and dispels from priests, druids, shamans and
  paladins fail.
- wvs: absent. Neither Loatheb nor 29184 nor 4327 appears in `src/` outside unrelated tests.
- size: S (the cast check; the encounter itself is the raid survey's)

## No SMSG_CANCEL_COMBAT when the server ends a player's attack
- status: open
- impact: low-medium
- who notices: rogues (Gouge, Blind, Vanish), hunters (Scatter Shot) and anyone whose attack the server stops. The auto-attack
  button may stay lit.
- mangos: `Unit::AttackStop` calls `SendAttackSwingCancelAttack` for any stop the client didn't start. `Spell.cpp` (around line
  3801) sends it for `SPELL_ATTR_CANCELS_AUTO_ATTACK_COMBAT`.
- wvs: `map/combat.rs` `stop_attack` and `spell_launched` send only `SMSG_ATTACKSTOP`. The opcode isn't referenced anywhere.
- size: S (the client check should show whether 1.12 needs it)

## Set, trinket and relic procs mangos scripts that are still missing
- status: partly (round 12's `set_procs.rs` added Oracle, Faith, Dreamwalker, Longevity, Flowing Water, Ten Storms, Mana Surge,
  Clean Escape and Cheat Death)
- impact: low
- who notices: raiders with these items
- mangos:
  - Aegis of Preservation (23780, trigger 23781): `HandleProcTriggerSpellAuraProc` fills in the trigger;
  - Battlegear of Wrath's Warrior's Wrath (21977 → 21887): `HandleDummyAuraProc`;
  - Plagueheart's cheaper Life Tap (28830): `Warlock.cpp`;
  - Darkmoon Card: Twisting Nether's 10% self-resurrection (23701 → 23700): `Player::GetResurrectionSpellId`.
- wvs: none of these ids appears in `src/`. `misc.rs` `capture_self_res` knows only soulstones and Reincarnation.
- size: S each

## Crit chance against creatures counts weapon skill twice
- status: open
- impact: low
- who notices: melee with weapon-skill bonuses: Humans with swords or maces, Orcs with axes, users of Edgemaster's
- mangos: `CalculateEffectiveCritChance`. Against an NPC, the attacker's skill is `GetSkillMaxForLevel()` ("weapon skill does not
  benefit crit% vs NPCs"), at 0.2% a point either way.
- wvs:
  - `map/derive.rs` `player` adds `(weapon_skill - max)*0.04` to `Derived.crit` (also the ranged one);
  - `map/combat.rs` `Roll::crit_chance` then adds `skill_diff*0.04` again when the difference is positive;
  - below the mob's level it uses 0.2 of the capped difference, as mangos does.
- size: S

## Drains from two casters stack on a player
- status: open
- impact: low
- who notices: PvP warlocks and priests (two Drain Life or two Drain Mana on one target)
- mangos: `SpellStacker` (`IsStackableAuraEffect`). Drain Life-type (aura 53) and Drain Mana-type (64) auras from different
  casters don't stack on a player-controlled target.
- wvs: `map/auras.rs` `cross_caster_coexist` returns true for any channeled spell, and the drains are channeled.
- size: S

## Improved Scorpid Sting does nothing
- status: open
- impact: low
- who notices: PvP hunters with the talent
- mangos: `SpellAuras.cpp` `HandleAuraModStat`. Scorpid Sting with class scripts 2388-2390 casts 19486, which takes 10%, 20% or
  30% of the strength loss as stamina.
- wvs: absent (no 2388-2390 and no 19486).
- size: S

## Shield Slam's dispel always succeeds
- status: open
- impact: low
- who notices: PvP Protection warriors and their targets
- mangos: `Spell::EffectDispel`. A warrior's Shield Slam (family flag 0x100000000) dispels only on a 50% roll.
- wvs: `map/spells/effects.rs` `dispel` has no Shield Slam case.
- size: S

## Smaller class-spell rules: Dispel Magic's threat, Power Infusion with Arcane Power, Flametongue Weapon's speed
- status: open
- impact: low
- who notices: priests (threat in raids), mages and priests, and Enhancement shamans who dual-wield
- mangos:
  - `Priest.cpp` `DispelMagicPriest::OnInit` halves Dispel Magic's helpful threat;
  - `Priest.cpp` `PowerInfusion::OnCheckCast` refuses Power Infusion on a target with Arcane Power (12042) as
    `SPELL_FAILED_AURA_BOUNCED`;
  - `SpellEffects.cpp` `EffectDummy` scales Flametongue Weapon's proc by the speed of the weapon that procced.
- wvs:
  - nothing halves Dispel Magic's threat;
  - 12042 isn't referenced;
  - `map/spells/class_scripts.rs` `flametongue` always uses the main hand's speed.
- size: S

## A spell's cancel ignores its id and also drops a queued next-swing ability
- status: partly
- impact: low
- who notices: warriors and druids, possibly. A cancel may un-queue Heroic Strike, Maul or Cleave; it isn't confirmed what the
  1.12 client sends.
- mangos: `SpellHandler.cpp` `HandleCancelCastOpcode` calls `InterruptNonMeleeSpells(false, spellId)`. Queued melee is left alone.
- wvs: `map/spells/mod.rs` `cancel` calls `interrupt` and then `combat::cancel_queued`, with no spell-id check.
- size: S

## Raid encounter spell scripts outside Blackwing Lair
- status: partly (round 12l scripted BWL: Razorgore, Vaelastrasz, Chromaggus and Nefarian's class calls; Golemagg's Trust is in
  `boss_mechanics.rs`)
- impact: low here (medium for raiders; it belongs to the raid survey)
- who notices: Molten Core, BWL, AQ and Naxxramas raiders
- mangos: `EffectDummy` and `EffectScriptEffect`. Shazzrah's Gate (23138: teleport to a random player and reset threat),
  Chromaggus's Hourglass Sand cure (23645), Kurinnaxx, Buru, the Obsidian Nullifiers and Eradicators, Kel'Thuzad's minions.
- wvs: 23138 and 23645 are absent, and so are the AQ and Naxxramas ids.
- size: L in total

## Glancing blows: no caster-class chance
- status: partly (the level-10 floor is fixed)
- impact: low
- who notices: priests, mages and warlocks in melee or with wands
- mangos: `CalculateEffectiveGlanceChance` gives a caster class `level + 2*gap` (from 30 + 2*gap at level 30) and no 40% cap.
- wvs: `map/combat.rs` `Roll::glancing` is `10 - 2*capped_diff` for every class, capped at 40%. The caster rule exists only for
  the damage (`glancing_factor`).
- size: S

## An elemental's full-resist share doesn't become a miss
- status: partly (magic-school blows are now unblockable; the partial resist was already there)
- impact: low
- who notices: tanks wearing resistance gear against elementals
- mangos: `CalculateEffectiveMissChance` adds the 100% resist bucket of the blow's school to the miss chance.
- wvs: `map/combat.rs` `Roll::miss` doesn't look at the school.
- size: S

## Ranged attacks can't be partially blocked
- status: open
- impact: low
- who notices: hunters against shield-bearing mobs and players
- mangos: `CalculateAbsorbResistBlock` rolls `RollAbilityPartialBlockOutcome` for ranged abilities, Auto Shot included.
- wvs: `map/combat.rs` `Roll::block` returns 0 for `ranged`, and `ranged.rs` always sends `blocked: 0`.
- size: S

## World bosses aren't treated as their target's level + 3
- status: open
- impact: low
- who notices: players below 60 against Kazzak, the Emerald dragons and Azuregos
- mangos: `Creature::GetLevelForTarget` uses the target's level + `WORLD_BOSS_LEVEL_DIFF` (3) for skill, glancing, daze and
  spell-miss calculations.
- wvs: absent. The template level is used.
- size: S

## Creatures swing whichever way they face
- status: open
- impact: low
- who notices: players circling or kiting a mob
- mangos: `Unit::UpdateMeleeAttackingState` requires `HasInArc(victim, 2π/3)` of every unit.
- wvs: `map/combat.rs` `try_swing` refuses a bad facing only for players (`va.is_player && !facing`).
- size: S

## The miss table's floor and its skill steps differ slightly
- status: open
- impact: low
- who notices: melee who hit-cap for raids
- mangos: `CalculateEffectiveMissChance`. +1% for the first 10 points of skill gap, 0.6% a point beyond that, and a 1% floor
  when the gap is over 10.
- wvs: `map/combat.rs` `Roll::miss` (vmangos's rule): 0.1 or 0.2% a point, the first point of hit dropped, clamped at 0.
- size: S

## Class base melee crit and dodge differ from mangos
- status: open (wvs's values may be the better match for 1.12's character sheet; listed for completeness)
- impact: low
- who notices: casters and hybrids reading their character sheet
- mangos: `GetMeleeCritFromAgility` has no class base; `PLAYER_BASE_DODGE` gives each class its own base dodge.
- wvs: `map/derive.rs` `class_base_bonus` adds the same per-class constant to both crit and dodge.
- size: S

## Combat-log and feedback messages wvs never sends
- status: never implemented
- impact: low
- who notices: groups and combat-log readers; hunters and warlocks (pet sounds)
- mangos:
  - `Unit::Kill`: `SMSG_PARTYKILLLOG` ("X has slain Y");
  - `Unit::Attack`: `SMSG_AI_REACTION` (the aggro cue);
  - `HandleProcTriggerDamageAuraProc`: `SMSG_PROCRESIST`;
  - `Item.cpp` `SendEnchantmentLog`: `SMSG_ENCHANTMENTLOG`;
  - `Spell.cpp` (around line 4277): `SMSG_SPELL_UPDATE_CHAIN_TARGETS`, a channel's extra targets;
  - `SendPetTalk` and `SendPetAIReaction`: `SMSG_PET_ACTION_SOUND`;
  - `Pet.cpp` `SendPetDismiss`: `SMSG_PET_DISMISS_SOUND`.
- wvs: none of these is referenced in `src/`.
- size: S each

## Energize amounts that fall with level, and threat that scales with level
- status: never implemented
- impact: low
- who notices:
  - high-level users of Restore Energy (9512), Burst of Energy (24532) and the Blood Fury item (24571), who get the full amount;
  - players below 60 using Arcane Shroud (26400) or the Eye of Diminution (28862). At 60 nothing changes.
- mangos: `EffectEnergize` and `HandleModThreat` per-id cases.
- wvs: absent.
- size: S

# Fixed since

From combat.md:
- Crushing blows count defense only up to the victim's level cap: `combat.rs` `Roll::crushing` (12g).
- Pets and charms are on the player side of the table (no crushes, glancing, the player factors, 7% a level spell miss):
  `Roll.attacker_is_player`, `victim_is_player` and `no_crush` (12g).
- A paladin's melee hits refresh their judgements: `DealMeleeDamage`'s `RefreshHolder` (12g).
- Parry against creatures uses the whole weapon skill (0.6, 0.1 and 0.04 a point): `Roll::parry` (12g).
- Feared and confused victims don't dodge, parry or block (`CanReactInCombat`) (12g).
- Weapon and defense skill-ups follow mangos's `UpdateCombatSkills` curve, and gray mobs train: `combat.rs` `skill_gains` (12g).
- Glancing has its level-10 victim floor: `Roll::glancing` (12g).
- Magic-school melee can't be blocked: `Roll::block` `magic` (12g).
- The off-hand's first swing comes at half its period: `combat.rs`, around line 1831 (12g).
- A disarmed creature hits for 50%: `combat.rs` `weapon_damage` (calls12).

From spells.md:
- Area spells test line of sight for a cast's targets: `targets.rs` `area_sight` (12e). The persistent-area ticks are still open, above.
- Effect 92 (`ENCHANT_HELD_ITEM`) works, with Improved Weapon Totems, so the Windfury and Flametongue Totems and the Firestone work (12e).
- Paladin auras are one per paladin (12e).
- Winter's Chill (aura 179) is read in the spell crit roll (12f).
- Seal of the Crusader divides the main hand's damage by 1.4 (12f).
- Mark and Gift of the Wild replace each other, as do Shadow Protection and its Prayer (12e).
- Roots (aura 26) and pacify-silence (60) break by chance on damage: `procs.rs` `fear_break_chance` (12f).
- Cooldowns and harmful auras run down while logged out (12k).
- Charge is refused when rooted or with no path, before it costs anything: `checks.rs` `charge_check` (12e). The rooted refusal
  for leaps and teleports wasn't re-checked.
- Polymorph, Mind Control and Enslave Demon can't be cast on someone else's tapped creature (`CANNOT_CAST_ON_TAPPED`) (12e).
- Blind has its own DR group, for creatures too (12e).
- Seal of Righteousness uses mangos's formula with spell power, cast as the rank's spell (12e).
- Frenzied Regeneration works: `spells/frenzied.rs` (12e).
- Ruthlessness's combo point is added after the finisher clears its own (12e).
- Inspiration and Ancestral Fortitude don't stack (12e).
- A new aura of a DR group replaces the old one (calls12).
- Flametongue Totem procs as mangos's `EffectDummy` (16368, the weapon's speed, no spell power) (12e).
- Mana Burn, Drain Mana and Viper Sting are refused at a target without that power (BAD_TARGETS) (12e).
- Curse of Doom is refused at players and has its 5% Doomguard (12e).
- Divine Intervention is refused at a flag carrier (12e).
- Instant spells report misses in `SMSG_SPELL_GO` (calls11).

From auras.md:
- Onyxia Scale Cloak protects from Shadow Flame (22682 isn't applied with 22683) (12f).
- Mind Control and Dominate Mind can be dispelled off a charmed friend: `effects.rs` `dispellable` (12f).
- A pet keeps its auras across map changes, mounting, death, the stable and logout, less the time it was away (12f).
- Set and relic procs, partly: Garments of the Oracle, Vestments of Faith, Dreamwalker, Idol of Longevity, Totem of Flowing Water,
  The Ten Storms, Mana Surge, Clean Escape, and Cheat Death with its health check: `set_procs.rs` (12f). The rest are open, above.
- Golemagg's Trust: `boss_mechanics.rs` `golemaggs_trust`.
