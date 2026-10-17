//! Boss scripts: vmangos's C++ creature AIs (the `script_name` scripts) as
//! Thistle Tea ports them, written as EventAI events built in code. Each
//! scripted creature's events replace those of the world database, so they
//! run through the same engine ([`super::event_ai`]) and interpreter
//! ([`super::scripts`]).
//!
//! Ported: Molten Core's lieutenants and their guards (Lucifron, Gehennas,
//! Shazzrah, Garr and his Firesworn, Baron Geddon, Sulfuron, Golemagg and
//! his Core Ragers, Firewalkers, Firelords, Core Hounds and Lava Surgers)
//! and Onyxia in three phases. Shazzrah's blink, Garr's forced eruptions,
//! the hounds' linked revival and Onyxia's flight path are left out.

use crate::game::data::GameData;
use crate::game::data::scripts::*;

/// Ids for the events and scripts built here, clear of the world
/// database's.
const BASE: u32 = 90_000_000;

const TRIGGERED: u32 = 0x02;
const AURA_NOT_PRESENT: u32 = 0x20;

// Target types.
const T_PROVIDED: u32 = 0;
const T_VICTIM: u32 = 1;
const T_HOSTILE_RANDOM: u32 = 4;
const T_HOSTILE_FARTHEST: u32 = 7;
const T_NEAREST_WITH_ENTRY: u32 = 10;

struct Builder<'a> {
    data: &'a mut ScriptData,
    next: u32,
}

/// One step.
fn step(command: u32) -> ScriptStep {
    ScriptStep {
        command,
        ..Default::default()
    }
}

fn cast(spell: u32, target_type: u32, flags: u32) -> ScriptStep {
    ScriptStep {
        command: 15,
        datalong: [spell, flags, 0, 0],
        target_type,
        ..Default::default()
    }
}

fn cast_self(spell: u32, flags: u32) -> ScriptStep {
    ScriptStep {
        data_flags: SF_TARGET_SELF,
        ..cast(spell, T_PROVIDED, flags)
    }
}

fn talk(text: u32) -> ScriptStep {
    ScriptStep {
        command: 0,
        dataint: [text as i32, 0, 0, 0],
        ..Default::default()
    }
}

fn zone_combat() -> ScriptStep {
    step(49)
}

fn set_phase(phase: u32) -> ScriptStep {
    ScriptStep {
        command: 44,
        datalong: [phase, 0, 0, 0],
        ..Default::default()
    }
}

fn combat_movement(on: bool) -> ScriptStep {
    ScriptStep {
        command: 43,
        datalong: [u32::from(on), 0, 0, 0],
        ..Default::default()
    }
}

fn melee(on: bool) -> ScriptStep {
    ScriptStep {
        command: 42,
        datalong: [u32::from(on), 0, 0, 0],
        ..Default::default()
    }
}

fn clear_spell_list() -> ScriptStep {
    step(55)
}

fn summon(entry: u32, despawn_ms: u32) -> ScriptStep {
    ScriptStep {
        command: 10,
        datalong: [entry, despawn_ms, 0, 0],
        dataint: [0, 0, T_HOSTILE_RANDOM as i32, 2],
        ..Default::default()
    }
}

/// Only in these phases, as an inverse phase mask.
fn only_in(phases: &[u32]) -> u32 {
    let mut mask = u32::MAX;
    for p in phases {
        mask &= !(1 << p);
    }
    mask
}

impl Builder<'_> {
    fn script(&mut self, steps: Vec<ScriptStep>) -> u32 {
        self.next += 1;
        let id = self.next;
        let steps = steps.into_iter().map(|s| ScriptStep { id, ..s }).collect();
        self.data.ai_scripts.insert(id, steps);
        id
    }

    #[allow(clippy::too_many_arguments)]
    fn event(
        &mut self,
        entry: u32,
        kind: u32,
        params: [i32; 4],
        flags: u32,
        phases: u32,
        steps: Vec<ScriptStep>,
    ) {
        let script = self.script(steps);
        self.next += 1;
        let e = AiEvent {
            id: self.next,
            creature: entry,
            condition: 0,
            kind,
            inverse_phase_mask: phases,
            chance: 100,
            flags,
            params,
            actions: [script, 0, 0],
        };
        self.data.ai_events.entry(entry).or_default().push(e);
    }

    fn aggro(&mut self, entry: u32, steps: Vec<ScriptStep>) {
        self.event(entry, EVENT_AGGRO, [0; 4], 0, 0, steps);
    }

    /// A timer in combat: first after `first`, then every `repeat` (ms).
    fn every(&mut self, entry: u32, first: (i32, i32), repeat: (i32, i32), steps: Vec<ScriptStep>) {
        self.every_in(entry, first, repeat, 0, steps);
    }

    fn every_in(
        &mut self,
        entry: u32,
        first: (i32, i32),
        repeat: (i32, i32),
        phases: u32,
        steps: Vec<ScriptStep>,
    ) {
        self.event(
            entry,
            EVENT_TIMER_IN_COMBAT,
            [first.0, first.1, repeat.0, repeat.1],
            EF_REPEATABLE,
            phases,
            steps,
        );
    }

    /// Once below `pct` health.
    fn below(&mut self, entry: u32, pct: i32, phases: u32, steps: Vec<ScriptStep>) {
        self.event(entry, EVENT_HP, [pct, 0, 0, 0], 0, phases, steps);
    }
}

/// The scripted creatures' events, in place of the world database's.
pub fn install(data: &mut GameData) {
    if !data.is_loaded() {
        return;
    }
    let s = &mut data.scripts;
    for entry in ENTRIES {
        s.ai_events.remove(entry);
    }
    let mut b = Builder {
        data: s,
        next: BASE,
    };
    molten_core(&mut b);
    onyxia(&mut b);
}

const LUCIFRON: u32 = 12118;
const GEHENNAS: u32 = 12259;
const SHAZZRAH: u32 = 12264;
const GARR: u32 = 12057;
const FIRESWORN: u32 = 12099;
const GEDDON: u32 = 12056;
const SULFURON: u32 = 12098;
const GOLEMAGG: u32 = 11988;
const CORE_RAGER: u32 = 11672;
const CORE_HOUND: u32 = 11671;
const FIREWALKER: u32 = 11666;
const FIRELORD: u32 = 11668;
const LAVA_SURGER: u32 = 12101;
const ONYXIA: u32 = 10184;
const WHELP: u32 = 11262;

const ENTRIES: &[u32] = &[
    LUCIFRON,
    GEHENNAS,
    SHAZZRAH,
    GARR,
    FIRESWORN,
    GEDDON,
    SULFURON,
    GOLEMAGG,
    CORE_RAGER,
    CORE_HOUND,
    FIREWALKER,
    FIRELORD,
    LAVA_SURGER,
    ONYXIA,
    WHELP,
];

/// The script names these replace: a creature with one of them and no
/// port here keeps its world database events.
pub fn ported(entry: u32) -> bool {
    ENTRIES.contains(&entry)
}

fn molten_core(b: &mut Builder) {
    // Lucifron: Impending Doom, Lucifron's Curse, Shadow Shock.
    b.aggro(LUCIFRON, vec![zone_combat()]);
    b.every(
        LUCIFRON,
        (10_000, 10_000),
        (20_000, 20_000),
        vec![cast_self(19702, 0)],
    );
    b.every(
        LUCIFRON,
        (20_000, 20_000),
        (15_000, 15_000),
        vec![cast_self(19703, 0)],
    );
    b.every(
        LUCIFRON,
        (6_000, 6_000),
        (6_000, 6_000),
        vec![cast(19460, T_HOSTILE_RANDOM, 0)],
    );
    // Gehennas: Shadow Bolt, Gehennas' Curse, Rain of Fire, Fist of Ragnaros.
    b.aggro(GEHENNAS, vec![zone_combat()]);
    b.every(
        GEHENNAS,
        (6_000, 12_000),
        (6_000, 12_000),
        vec![cast(19717, T_HOSTILE_RANDOM, 0)],
    );
    b.every(
        GEHENNAS,
        (5_000, 10_000),
        (25_000, 30_000),
        vec![cast_self(19716, 0)],
    );
    b.every(
        GEHENNAS,
        (3_000, 6_000),
        (3_000, 6_000),
        vec![cast(19728, T_HOSTILE_RANDOM, 0)],
    );
    b.every(
        GEHENNAS,
        (3_000, 6_000),
        (3_000, 6_000),
        vec![cast(19729, T_VICTIM, 0)],
    );
    // Shazzrah: Arcane Explosion, Shazzrah's Curse, Magic Grounding,
    // Counterspell.
    b.every(
        SHAZZRAH,
        (2_000, 2_000),
        (3_000, 5_000),
        vec![cast(19712, T_VICTIM, 0)],
    );
    b.every(
        SHAZZRAH,
        (10_000, 10_000),
        (20_000, 20_000),
        vec![cast(19713, T_VICTIM, AURA_NOT_PRESENT)],
    );
    b.every(
        SHAZZRAH,
        (5_000, 5_000),
        (7_000, 14_000),
        vec![cast_self(19714, 0)],
    );
    b.every(
        SHAZZRAH,
        (15_000, 15_000),
        (16_000, 18_000),
        vec![cast(19715, T_VICTIM, 0)],
    );
    // Garr: Antimagic Pulse, Magma Shackles; his Firesworn's death enrages
    // him.
    b.aggro(GARR, vec![zone_combat()]);
    b.every(
        GARR,
        (15_000, 15_000),
        (15_000, 20_000),
        vec![cast_self(19492, 0)],
    );
    b.every(
        GARR,
        (10_000, 10_000),
        (10_000, 15_000),
        vec![cast_self(19496, 0)],
    );
    b.aggro(
        FIRESWORN,
        vec![cast_self(3391, TRIGGERED | AURA_NOT_PRESENT), zone_combat()],
    );
    b.every(
        FIRESWORN,
        (10_000, 10_000),
        (20_000, 20_000),
        vec![cast(15732, T_VICTIM, 0)],
    );
    b.event(
        FIRESWORN,
        EVENT_DEATH,
        [0; 4],
        0,
        0,
        vec![
            ScriptStep {
                target_param1: GARR,
                target_param2: 150,
                ..cast(19515, T_NEAREST_WITH_ENTRY, TRIGGERED)
            },
            cast_self(19497, TRIGGERED),
        ],
    );
    // Baron Geddon: Living Bomb, Ignite Mana, Inferno, and Armageddon at
    // five percent.
    let fighting = only_in(&[0]);
    b.aggro(GEDDON, vec![zone_combat()]);
    b.every_in(
        GEDDON,
        (15_000, 20_000),
        (12_000, 15_000),
        fighting,
        vec![cast(20475, T_HOSTILE_RANDOM, 0)],
    );
    b.every_in(
        GEDDON,
        (10_000, 15_000),
        (20_000, 30_000),
        fighting,
        vec![cast_self(19659, 0)],
    );
    b.every_in(
        GEDDON,
        (18_000, 24_000),
        (18_000, 24_000),
        fighting,
        vec![cast_self(19695, 0)],
    );
    b.below(
        GEDDON,
        5,
        fighting,
        vec![
            step(5),
            combat_movement(false),
            cast_self(20478, TRIGGERED),
            talk(8253),
            set_phase(1),
        ],
    );
    b.event(
        GEDDON,
        EVENT_EVADE,
        [0; 4],
        0,
        0,
        vec![set_phase(0), combat_movement(true)],
    );
    // Sulfuron: Demoralizing Shout, Inspire, Hand of Ragnaros, Flame Spear,
    // Knockdown.
    b.aggro(SULFURON, vec![zone_combat()]);
    b.every(
        SULFURON,
        (15_000, 15_000),
        (15_000, 20_000),
        vec![cast(19778, T_VICTIM, 0)],
    );
    b.every(
        SULFURON,
        (13_000, 13_000),
        (20_000, 26_000),
        vec![cast_self(19779, 0)],
    );
    b.every(
        SULFURON,
        (6_000, 6_000),
        (12_000, 15_000),
        vec![cast(19780, T_VICTIM, 0)],
    );
    b.every(
        SULFURON,
        (2_000, 2_000),
        (12_000, 16_000),
        vec![cast(19781, T_HOSTILE_RANDOM, 0)],
    );
    b.every(
        SULFURON,
        (10_000, 10_000),
        (15_000, 18_000),
        vec![cast(19777, T_VICTIM, 0)],
    );
    // Golemagg: Pyroblast, and below a tenth Enrage and Earthquakes.
    b.every(
        GOLEMAGG,
        (7_000, 7_000),
        (7_000, 7_000),
        vec![cast(20228, T_HOSTILE_RANDOM, 0)],
    );
    b.below(GOLEMAGG, 10, 0, vec![cast_self(20544, 0)]);
    b.event(
        GOLEMAGG,
        EVENT_HP,
        [10, 0, 5_000, 5_000],
        EF_REPEATABLE,
        0,
        vec![cast(19798, T_VICTIM, 0)],
    );
    // Core Ragers can't drop below half while Golemagg lives.
    b.event(
        CORE_RAGER,
        EVENT_SPAWNED,
        [0; 4],
        0,
        0,
        vec![ScriptStep {
            command: 52,
            datalong: [50, 1, 0, 0],
            ..Default::default()
        }],
    );
    b.every(
        CORE_RAGER,
        (7_000, 7_000),
        (10_000, 10_000),
        vec![cast(19820, T_VICTIM, 0)],
    );
    b.below(
        CORE_RAGER,
        50,
        0,
        vec![talk(7865), cast_self(17683, TRIGGERED)],
    );
    // Core Hounds call the raid, and bite.
    b.aggro(CORE_HOUND, vec![zone_combat()]);
    b.every(
        CORE_HOUND,
        (4_000, 7_000),
        (4_000, 7_000),
        vec![cast(19771, T_VICTIM, 0)],
    );
    // Firewalkers: Fire Blossoms and Melt Armor.
    b.aggro(FIREWALKER, vec![clear_spell_list(), zone_combat()]);
    b.every(
        FIREWALKER,
        (6_000, 6_000),
        (12_000, 12_000),
        vec![
            cast_self(19636, 0),
            ScriptStep {
                delay_ms: 1_000,
                ..cast(19637, T_HOSTILE_RANDOM, 0)
            },
            ScriptStep {
                delay_ms: 2_000,
                ..cast(19637, T_HOSTILE_RANDOM, 0)
            },
            ScriptStep {
                delay_ms: 3_000,
                ..cast(19637, T_HOSTILE_RANDOM, 0)
            },
        ],
    );
    b.every(
        FIREWALKER,
        (20_000, 20_000),
        (20_000, 20_000),
        vec![cast(19635, T_VICTIM, 0)],
    );
    // Firelords: Incinerate, Soul Burn, Lava Spawns.
    b.aggro(
        FIRELORD,
        vec![
            clear_spell_list(),
            cast_self(19396, TRIGGERED | AURA_NOT_PRESENT),
        ],
    );
    b.every(
        FIRELORD,
        (7_500, 12_500),
        (15_000, 20_000),
        vec![cast_self(19569, 0)],
    );
    b.every(
        FIRELORD,
        (4_000, 6_000),
        (3_000, 4_000),
        vec![cast(19393, T_HOSTILE_RANDOM, TRIGGERED)],
    );
    // Lava Surgers surge at their farthest attacker.
    b.aggro(LAVA_SURGER, vec![clear_spell_list()]);
    b.every(
        LAVA_SURGER,
        (1_000, 2_000),
        (5_000, 6_000),
        vec![cast(19196, T_HOSTILE_FARTHEST, 0)],
    );
}

fn onyxia(b: &mut Builder) {
    let ground = only_in(&[0, 3]);
    let flying = only_in(&[2]);
    b.aggro(ONYXIA, vec![talk(8286), set_phase(0)]);
    b.event(
        ONYXIA,
        EVENT_KILL,
        [0, 0, 0, 0],
        EF_REPEATABLE,
        0,
        vec![talk(8287)],
    );
    // On the ground: Flame Breath, Cleave, Wing Buffet, Knock Away, Tail
    // Sweep.
    b.every_in(
        ONYXIA,
        (10_000, 20_000),
        (10_000, 20_000),
        ground,
        vec![cast(18435, T_VICTIM, 0)],
    );
    b.every_in(
        ONYXIA,
        (5_000, 10_000),
        (5_000, 10_000),
        ground,
        vec![cast(19983, T_VICTIM, 0)],
    );
    b.every_in(
        ONYXIA,
        (10_000, 20_000),
        (15_000, 30_000),
        ground,
        vec![cast(18500, T_VICTIM, 0)],
    );
    b.every_in(
        ONYXIA,
        (20_000, 30_000),
        (15_000, 30_000),
        ground,
        vec![cast(19633, T_VICTIM, 0)],
    );
    b.every_in(
        ONYXIA,
        (15_000, 20_000),
        (15_000, 20_000),
        ground,
        vec![cast_self(15847, 0)],
    );
    // At 65 percent she takes to the air: Fireballs, and whelps.
    b.below(
        ONYXIA,
        65,
        only_in(&[0]),
        vec![
            talk(8288),
            set_phase(2),
            melee(false),
            combat_movement(false),
            cast_self(17131, TRIGGERED),
        ],
    );
    b.every_in(
        ONYXIA,
        (3_000, 5_000),
        (3_000, 5_000),
        flying,
        vec![cast(18392, T_HOSTILE_RANDOM, 0)],
    );
    b.every_in(
        ONYXIA,
        (5_000, 5_000),
        (30_000, 30_000),
        flying,
        vec![
            summon(WHELP, 120_000),
            summon(WHELP, 120_000),
            summon(WHELP, 120_000),
            summon(WHELP, 120_000),
        ],
    );
    // Below 40 percent she lands with a Bellowing Roar, and roars on.
    b.below(
        ONYXIA,
        40,
        only_in(&[2]),
        vec![
            talk(8290),
            ScriptStep {
                command: 14,
                datalong: [17131, 0, 0, 0],
                data_flags: SF_TARGET_SELF,
                ..Default::default()
            },
            set_phase(3),
            melee(true),
            combat_movement(true),
            cast_self(18431, TRIGGERED),
        ],
    );
    b.every_in(
        ONYXIA,
        (15_000, 30_000),
        (15_000, 30_000),
        only_in(&[3]),
        vec![cast_self(18431, TRIGGERED)],
    );
    b.event(
        ONYXIA,
        EVENT_EVADE,
        [0; 4],
        0,
        0,
        vec![set_phase(0), melee(true), combat_movement(true), step(56)],
    );
    b.aggro(WHELP, vec![zone_combat()]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phase_mask_leaves_the_named_phases_open() {
        let m = only_in(&[0, 3]);
        assert_eq!(m & 1, 0);
        assert_eq!(m & (1 << 3), 0);
        assert_ne!(m & (1 << 2), 0);
    }
}
