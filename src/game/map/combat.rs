//! Melee combat, for players and creatures alike: who may attack whom, the
//! swing timers of both hands, vmangos's single-roll hit table as Thistle
//! Tea ports it (weapon skill against defense, from behind, glancing and
//! crushing blows, blocks), damage with its modifiers and armor, parry
//! haste, extra attacks, reactive windows (Overpower, Revenge, Riposte),
//! next-swing spells, rage, skill gains, threat, death, and what a kill is
//! worth. Ranged auto-repeat is in [`crate::game::map::ranged`], procs in
//! [`crate::game::map::procs`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use ahash::AHashSet;
use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    SMSG_ATTACKSTART, SMSG_ATTACKSTOP, SMSG_ATTACKSWING_BADFACING, SMSG_ATTACKSWING_CANT_ATTACK,
    SMSG_ATTACKSWING_DEADTARGET, SMSG_ATTACKSWING_NOTINRANGE, SMSG_ATTACKSWING_NOTSTANDING,
    SMSG_STANDSTATE_UPDATE, UnitStandState, Vector3d,
};

use crate::game::conn::encode;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::death::{self, Death};
use crate::game::map::player::{PLAYER_FLAGS_FFA_PVP, UNIT_FLAG_IN_COMBAT, UNIT_FLAG_PVP};
use crate::game::map::procs;

pub const SMSG_ATTACKERSTATEUPDATE: u16 = 0x14A;

/// `SMSG_ATTACKERSTATEUPDATE` hit flags.
pub const HITINFO_NORMALSWING2: u32 = 0x02;
pub const HITINFO_LEFTSWING: u32 = 0x04;
pub const HITINFO_MISS: u32 = 0x10;
pub const HITINFO_ABSORB: u32 = 0x20;
pub const HITINFO_RESIST: u32 = 0x40;
pub const HITINFO_CRITICALHIT: u32 = 0x80;
pub const HITINFO_GLANCING: u32 = 0x4000;
pub const HITINFO_CRUSHING: u32 = 0x8000;
pub const HITINFO_SWINGNOHITSOUND: u32 = 0x8_0000;

/// What the victim did.
pub const VICTIMSTATE_UNAFFECTED: u32 = 0;
pub const VICTIMSTATE_NORMAL: u32 = 1;
pub const VICTIMSTATE_DODGE: u32 = 2;
pub const VICTIMSTATE_PARRY: u32 = 3;
pub const VICTIMSTATE_BLOCKS: u32 = 5;
pub const VICTIMSTATE_EVADES: u32 = 6;
pub const VICTIMSTATE_IS_IMMUNE: u32 = 7;

/// A creature's `flags_extra`: it never parries, never blocks, or always
/// crushes.
pub const FLAG_EXTRA_NO_PARRY: u32 = 0x4;
pub const FLAG_EXTRA_NO_BLOCK: u32 = 0x10;
pub const FLAG_EXTRA_ALWAYS_CRUSH: u32 = 0x2000;

/// `UNIT_FIELD_AURASTATE` bits: after a dodge, parry or block (Revenge,
/// Riposte); below a fifth of health (Execute); after a hunter parries
/// (Counterattack, Mongoose Bite).
pub const AURA_STATE_DEFENSE: u32 = 1 << 0;
pub const AURA_STATE_HEALTHLESS_20: u32 = 1 << 1;
pub const AURA_STATE_HUNTER_PARRY: u32 = 1 << 6;
/// How long a reactive window stays open.
pub const REACTIVE_WINDOW: Duration = Duration::from_millis(4000);

/// A player in a fight with another player's side (a player, their pet, in
/// a duel too) stays in it at least this long after the last blow either
/// way (mangos's `TriggerCombatTimer(true)`, Thistle Tea's
/// `@pvp_combat_ms`). Against creatures there is no timer: a player is in
/// the fight while a creature holds them on its threat list.
pub const PVP_COMBAT: Duration = Duration::from_secs(5);
/// How often a player's fight is looked at for its end, the first time at
/// the next update after it began (mangos's `COMBAT_MANAGER_TICK`, and
/// `OnCombatStart` priming it).
pub const COMBAT_CHECK: Duration = Duration::from_millis(1200);
/// Bloodrage's rage over time: while it lasts the warrior stays in combat
/// (mangos's `Bloodrage` aura script, `SetForcedCombat`).
pub const SPELL_BLOODRAGE_REGEN: u32 = 29131;
/// A PvP flag lasts this long after the last PvP blow.
pub const PVP_TIMEOUT: Duration = Duration::from_secs(300);
/// The swing errors, as a player is last told of one: not standing, out
/// of range, facing away.
const SWING_ERROR_NOT_STANDING: u8 = 1;
const SWING_ERROR_NOT_IN_RANGE: u8 = 2;
const SWING_ERROR_BAD_FACING: u8 = 3;
/// The attacker faces the victim within this arc.
const FACING_ARC: f32 = 2.0 * std::f32::consts::PI / 3.0;
/// Reckoning banks up to this many extra attacks.
const RECKONING_CAP: u32 = 4;
pub const SPELL_RECKONING: u32 = 20178;
pub const SPELL_DAZED: u32 = 1604;
pub const SPELL_BERSERKER_RAGE: u32 = 18499;

/// A hand a blow comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hand {
    Main,
    Off,
    Ranged,
}

/// A unit's fight: whom it swings at, and when each hand's next swing is
/// due.
#[derive(Debug, Clone, Default)]
pub struct Combat {
    pub victim: Guid,
    pub attacking: bool,
    pub next_swing: Option<Instant>,
    pub next_offhand: Option<Instant>,
    /// When it last entered or was kept in a fight ([`enter`]).
    pub last_action: Option<Instant>,
    /// A player's PvP combat timer: in the fight at least until then.
    pub combat_until: Option<Instant>,
    /// When a player's fight is next looked at for its end.
    pub next_check: Option<Instant>,
    pub last_pvp: Option<Instant>,
    /// The swing error the player was last told of, while it lasts: told
    /// once each time it changes (mangos's `LastSwingErrorMsg`, Thistle
    /// Tea's `last_swing_error`), cleared by a swing that lands and by
    /// stopping the attack.
    pub last_swing_error: Option<u8>,
    /// Units fighting this one.
    pub attackers: AHashSet<Guid>,
    /// Melee chasers that have come at it (`ai::chases`), for the fan-out's
    /// arc: those on their way, not yet among its attackers, count too.
    /// Let go of once they chase another or leave the fight.
    pub chasers: Vec<Guid>,
    /// A next-swing spell waiting for the main hand (Heroic Strike).
    pub queued: Option<u32>,
    /// Auto Shot, Shoot: repeating at the ranged weapon's speed.
    pub auto_repeat: Option<AutoRepeat>,
    /// When the ranged weapon is ready again after its last shot, kept
    /// though the repeat stops.
    pub ranged_ready_at: Option<Instant>,
    /// Main-hand attacks owed at once, and Reckoning's banked ones.
    pub extra_attacks: u32,
    pub held_extra_attacks: u32,
    pub in_extra_attack: bool,
    /// After a dodge, parry or block: the window's target and its end.
    pub defense_window: Option<(Guid, Instant)>,
    pub hunter_parry_window: Option<(Guid, Instant)>,
    /// When the combo point a dodge gave a warrior (Overpower) runs out.
    pub dodge_combo_until: Option<Instant>,
    /// The victim a creature's `SMSG_ATTACKSTART` went out for: it goes
    /// out at its first swing in reach, and again for each new victim
    /// (Thistle Tea's `attack_started`).
    pub announced: Option<Guid>,
    /// A Feign Death believed holds: no creature notices it or picks it
    /// while it lasts ([`feigned`]).
    pub feigning: bool,
}

/// A repeating ranged attack.
#[derive(Debug, Clone, Copy)]
pub struct AutoRepeat {
    pub spell: u32,
    pub target: Guid,
    pub next_at: Instant,
}

/// The outcome of a swing's roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Miss,
    Dodge,
    Parry,
    Glancing,
    Block,
    Crit,
    Crushing,
    Hit,
}

/// What the hit table is rolled from.
#[derive(Debug, Clone, Copy, Default)]
pub struct Roll {
    pub attacker_skill: i32,
    pub victim_defense: i32,
    /// On the players' side of the table: a player, or a creature under a
    /// player's control (mangos's `CanGlance`, `UNIT_FLAG_PLAYER_CONTROLLED`
    /// not charmed by a creature).
    pub attacker_is_player: bool,
    /// A player or a player's pet or charm (mangos's
    /// `UNIT_FLAG_PLAYER_CONTROLLED`): the players' factors, never glanced,
    /// the only victims crushed.
    pub victim_is_player: bool,
    /// The victim is a creature for all that: it dodges from behind.
    pub victim_creature: bool,
    /// A creature on the creatures' side that deals no crushing blows: one
    /// a player masters, or `NO_CRUSHING_BLOWS` (mangos's `CanCrushInCombat`).
    pub no_crush: bool,
    pub attacker_level: i32,
    pub victim_level: i32,
    /// In percent.
    pub crit: f32,
    pub hit: f32,
    pub dodge: f32,
    pub parry: f32,
    pub block: f32,
    pub from_behind: bool,
    /// The victim stands (a sitting player is crit).
    pub standing: bool,
    /// The victim casts or is stunned: it can't dodge, parry or block.
    pub avoidance_disabled: bool,
    /// The attacker wields two weapons: white swings miss more.
    pub dual_wield: bool,
    /// A spell's swing (Heroic Strike), not a white one.
    pub special: bool,
    pub ranged: bool,
    pub can_crit: bool,
    pub no_parry: bool,
    pub no_block: bool,
    pub always_crush: bool,
    /// What the victim's auras take off its attackers' hit chance (in
    /// percent; negative helps them).
    pub attacker_hit: f32,
    /// A player victim with weapons sheathed doesn't block.
    pub sheathed: bool,
    /// The blow is of a magic school (an elemental's): it can't be blocked
    /// (mangos's pre-WotLK `CanBlockInCombat`).
    pub magic: bool,
}

impl Roll {
    /// The miss chance, in hundredths of a percent.
    pub fn miss_bp(&self) -> i32 {
        self.miss()
    }

    fn skill_diff(&self) -> i32 {
        self.attacker_skill - self.victim_defense
    }

    fn capped_skill_diff(&self) -> i32 {
        self.attacker_skill.min(self.attacker_level * 5) - self.victim_defense
    }

    fn avoidance_skill_diff(&self) -> i32 {
        self.attacker_skill - self.victim_level * 5
    }

    /// A creature under level ten avoids less.
    fn low_level(&self, chance: f32) -> f32 {
        if !self.victim_is_player && self.victim_level < 10 {
            chance * self.victim_level as f32 / 10.0
        } else {
            chance
        }
    }

    fn miss(&self) -> i32 {
        if !self.standing {
            return 0;
        }
        let diff = self.skill_diff() as f32;
        let skill_bonus = if self.victim_is_player {
            diff * 0.04
        } else if diff < -10.0 {
            diff * 0.2
        } else {
            diff * 0.1
        };
        let penalty = if self.dual_wield && !self.special && !self.ranged {
            19.0
        } else {
            0.0
        };
        let hit = if diff < -10.0 && self.hit > 0.0 {
            self.hit - 1.0
        } else {
            self.hit
        };
        bp(
            (self.low_level(5.0 + penalty - skill_bonus) - hit - self.attacker_hit)
                .clamp(0.0, 60.0),
        )
    }

    fn sitting_crit(&self) -> bool {
        self.victim_is_player
            && !self.standing
            && self.can_crit
            && (self.crit > 0.0 || !self.attacker_is_player)
    }

    fn dodge(&self) -> i32 {
        if self.ranged
            || self.avoidance_disabled
            || (self.victim_is_player && !self.victim_creature && self.from_behind)
        {
            return 0;
        }
        let bonus = if self.victim_is_player {
            self.avoidance_skill_diff() as f32 * 0.04
        } else {
            self.skill_diff() as f32 * 0.1
        };
        bp(self.low_level(self.dodge - bonus).max(0.0))
    }

    fn parry(&self) -> i32 {
        if self.ranged || self.parry <= 0.0 || self.avoidance_disabled || self.from_behind {
            return 0;
        }
        if self.victim_is_player {
            return bp((self.parry - self.avoidance_skill_diff() as f32 * 0.04).max(0.0));
        }
        if self.no_parry {
            return 0;
        }
        // The attacker's whole weapon skill, past its cap too: a creature
        // above it by more than ten parries 0.6% a point, by up to ten
        // 0.1%, and below it 0.04% (mangos's
        // `CalculateEffectiveParryChance`).
        let d = (self.victim_level * 5 - self.attacker_skill) as f32;
        let factor = if d > 10.0 {
            0.6
        } else if d > 0.0 {
            0.1
        } else {
            0.04
        };
        bp(self.low_level(self.parry + d * factor).max(0.0))
    }

    fn glancing(&self) -> i32 {
        // A victim of level ten or less is never glanced (mangos's
        // `CanGlanceInCombat`).
        if self.ranged
            || !self.attacker_is_player
            || self.victim_is_player
            || self.special
            || self.victim_level <= 10
        {
            return 0;
        }
        bp((10.0 - self.capped_skill_diff() as f32 * 2.0).max(0.0)).min(4000)
    }

    fn block(&self) -> i32 {
        if self.ranged
            || self.magic
            || self.avoidance_disabled
            || self.from_behind
            || self.block <= 0.0
            || (self.victim_is_player && self.sheathed)
        {
            return 0;
        }
        if self.victim_is_player {
            return bp((self.block - self.avoidance_skill_diff() as f32 * 0.04).max(0.0));
        }
        if self.no_block {
            return 0;
        }
        bp(self
            .low_level((self.block - self.skill_diff() as f32 * 0.1).min(5.0))
            .max(0.0))
    }

    fn crit_chance(&self) -> i32 {
        if !self.can_crit {
            return 0;
        }
        let term = if self.victim_is_player || self.skill_diff() > 0 {
            self.skill_diff() as f32 * 0.04
        } else {
            self.capped_skill_diff() as f32 * 0.2
        };
        bp((self.crit + term).max(0.0))
    }

    fn crushing(&self) -> i32 {
        if self.attacker_is_player || self.no_crush || self.special || self.ranged {
            return 0;
        }
        if self.always_crush {
            return 10_000;
        }
        // Only a player or a player's is crushed, its defense counted no
        // higher than its level's most: past that, defense takes no
        // crushing blows off (mangos's `CanCrushInCombat` and
        // `CalculateEffectiveCrushChance`).
        if !self.victim_is_player {
            return 0;
        }
        let diff = self.attacker_skill - self.victim_defense.min(self.victim_level * 5);
        if diff >= 15 { diff * 200 - 1500 } else { 0 }
    }
}

fn bp(chance: f32) -> i32 {
    // To the nearest hundredth, as mangos's `chance_u` has it: 4.8% is 480,
    // not the 479 its float would be cut to.
    (chance * 100.0).round() as i32
}

/// Walk the table's steps with a roll in hundredths of a percent.
fn walk(steps: &[(Outcome, i32)], roll: i32, otherwise: Outcome) -> Outcome {
    let mut sum = 0;
    for (outcome, chance) in steps {
        if *chance >= 10_000 {
            return *outcome;
        }
        if *chance > 0 && roll < sum + chance {
            return *outcome;
        }
        sum += (*chance).max(0);
    }
    otherwise
}

/// vmangos's single roll (`RollMeleeOutcomeAgainst`): miss, a sitting
/// victim's crit, dodge, parry, glancing, block, crit, crushing, `roll` in
/// hundredths of a percent.
pub fn roll_outcome(r: &Roll, roll: i32) -> Outcome {
    let sitting = if r.sitting_crit() { 10_000 } else { 0 };
    walk(
        &[
            (Outcome::Miss, r.miss()),
            (Outcome::Crit, sitting),
            (Outcome::Dodge, r.dodge()),
            (Outcome::Parry, r.parry()),
            (Outcome::Glancing, r.glancing()),
            (Outcome::Block, r.block()),
            (Outcome::Crit, r.crit_chance()),
            (Outcome::Crushing, r.crushing()),
        ],
        roll,
        Outcome::Hit,
    )
}

/// A melee or ranged ability's roll: miss, dodge, parry and block first,
/// then the crit on a second roll.
pub fn roll_special(r: &Roll, roll: i32, crit_roll: i32) -> Outcome {
    let r = Roll {
        special: true,
        ..*r
    };
    let sitting = if r.sitting_crit() { 10_000 } else { 0 };
    let first = walk(
        &[
            (Outcome::Miss, r.miss()),
            (Outcome::Crit, sitting),
            (Outcome::Dodge, r.dodge()),
            (Outcome::Parry, r.parry()),
            (Outcome::Block, r.block()),
        ],
        roll,
        Outcome::Hit,
    );
    if first == Outcome::Hit && crit_roll < r.crit_chance() {
        Outcome::Crit
    } else {
        first
    }
}

/// What a glancing blow keeps of the damage, for a defense `gap` above the
/// attacker's skill; casters glance harder.
pub fn glancing_factor(gap: i32, caster: bool, roll: f32) -> f32 {
    let gap = gap as f32;
    let (low_cut, low_cap, high_cut) = if caster {
        (0.7, 0.6, 0.3)
    } else {
        (0.0, 0.91, 0.0)
    };
    let low = (1.3 - 0.05 * gap - low_cut).clamp(0.01, low_cap);
    let high = (1.2 - 0.03 * gap - high_cut).clamp(0.2, 0.99);
    low + (high - low) * roll.clamp(0.0, 1.0)
}

/// Armor's share of physical damage stopped, up to 75%.
pub fn armor_reduction(armor: u32, attacker_level: u32) -> f32 {
    let armor = armor as f32;
    (armor / (armor + 400.0 + 85.0 * attacker_level as f32)).clamp(0.0, 0.75)
}

/// XP for a kill (vmangos's `MaNGOS::XP::BaseGain`, vanilla content).
pub fn kill_xp(player_level: u32, mob_level: u32, elite: bool, multiplier: f32) -> u32 {
    let o = KillXp {
        elite,
        multiplier,
        ..Default::default()
    };
    kill_xp_with(player_level, player_level, mob_level, &o)
}

/// What a kill's XP depends on beyond the levels (Thistle Tea's
/// `Experience.kill_options`).
#[derive(Debug, Clone, Copy)]
pub struct KillXp {
    pub elite: bool,
    /// In a five-man dungeon, an elite is worth more.
    pub dungeon: bool,
    /// The template's multiplier (`CreatureTemplate::xp_rate`): 0 gives
    /// nothing.
    pub multiplier: f32,
    /// The share of its damage players dealt (`DamageOrigin`).
    pub damage_share: f32,
    pub no_xp: bool,
    /// The victim was a pet.
    pub pet_victim: bool,
}

impl Default for KillXp {
    fn default() -> Self {
        KillXp {
            elite: false,
            dungeon: false,
            multiplier: 1.0,
            damage_share: 1.0,
            no_xp: false,
            pet_victim: false,
        }
    }
}

/// Thistle Tea's `Experience.kill_xp`: the base from `owner_level` (a
/// hunter's for their pet), the level factor from `unit_level`, in
/// single-precision floats as the client counts, rounded half to even.
pub fn kill_xp_with(owner_level: u32, unit_level: u32, mob_level: u32, o: &KillXp) -> u32 {
    // A pet is worth nothing, a creature's as a player's (mangos-classic's
    // `XP::Gain`: `IsPet()`; not Thistle Tea's and vmangos's three
    // quarters).
    if o.no_xp || o.pet_victim {
        return 0;
    }
    let factor = if mob_level >= unit_level {
        1.0 + 0.05f32 * (mob_level - unit_level).min(4) as f32
    } else if mob_level > gray_level(unit_level) {
        let zd = zero_difference(unit_level) as f32;
        (zd + mob_level as f32 - unit_level as f32) / zd
    } else {
        0.0
    };
    let base = (owner_level * 5 + 45) as f32 * factor;
    let elite = if !o.elite {
        1.0
    } else if o.dungeon {
        2.5
    } else {
        2.0
    };
    let mut xp = base * elite;
    // A multiplier of nothing gives nothing; only one below that is
    // taken for one (Thistle Tea's `experience_multiplier`).
    xp *= if o.multiplier >= 0.0 {
        o.multiplier
    } else {
        1.0
    };
    xp *= o.damage_share;
    round_half_even(xp)
}

/// Rounded to the nearest, a half to the even.
fn round_half_even(x: f32) -> u32 {
    let whole = x.trunc();
    if x - whole == 0.5 {
        (whole as u32) + (whole as u32) % 2
    } else {
        x.round().max(0.0) as u32
    }
}

/// The options of a kill of creature `victim` (Thistle Tea's
/// `Experience.kill_options`): elites are ranks one to three; nothing for a
/// creature flagged so, one a player controls, or a summoned critter,
/// totem or the like.
pub fn kill_options(map: &Map, victim: Guid) -> KillXp {
    let Some(c) = map.creature(victim) else {
        return KillXp {
            no_xp: true,
            ..Default::default()
        };
    };
    let t = map.realm.data.creature_template(c.entry);
    let controlled = c.owner.is_some_and(guid::is_player);
    let summoned = c.fields.get_u32(UNIT_CREATED_BY_SPELL) != 0
        && t.is_some_and(|t| matches!(t.creature_type, 8 | 10 | 11) || t.health_multiplier <= 0.1);
    KillXp {
        elite: t.is_some_and(|t| matches!(t.rank, 1..=3)),
        dungeon: crate::game::instances::instance_type(&map.realm, map.key.map) == 1,
        multiplier: t.map_or(1.0, |t| t.xp_rate()),
        damage_share: crate::game::map::creature_combat::xp_share(map, victim),
        no_xp: controlled || summoned || t.is_some_and(|t| t.static_flags1 & 0x2 != 0),
        // A pet, a totem or a guardian: a creature's guardian made a
        // creature of its own (`pets::wild`) is mangos's `GUARDIAN_PET`.
        pet_victim: c.pet.is_some() || c.scripted.as_deref().is_some_and(|s| s.guardian),
    }
}

/// Levels at or below which a kill gives nothing: mangos's
/// `IsTrivialLevelDifference`, which all its rewards use (XP, honor,
/// reputation, procs); at 60 that is a gap over 12, 47 and below, not the
/// colour's `GetGrayLevel` (51), which nothing rewards by.
pub fn gray_level(level: u32) -> u32 {
    match level {
        0..=5 => 0,
        6..=39 => level - 5 - level / 10,
        40..=59 => level - 1 - level / 5,
        _ => level - 13,
    }
}

fn zero_difference(level: u32) -> u32 {
    match level {
        0..=7 => 5,
        8..=9 => 6,
        10..=11 => 7,
        12..=15 => 8,
        16..=19 => 9,
        20..=29 => 11,
        30..=39 => 12,
        40..=44 => 13,
        45..=49 => 14,
        50..=54 => 15,
        55..=59 => 16,
        _ => 17,
    }
}

/// Rage a warrior gets for dealing (or, with `taken`, taking) `damage`, in
/// tenths, as the field holds it.
pub fn rage_for(damage: u32, level: u32, taken: bool) -> u32 {
    rage_scaled(damage, level, taken, 1.0)
}

/// [`rage_for`], scaled by `by` before it is rounded down (mangos's
/// `Player::RewardRage`, Berserker Rage's 1.3).
pub fn rage_scaled(damage: u32, level: u32, taken: bool, by: f32) -> u32 {
    let l = level as f32;
    let c = 0.009_110_784 * l * l + 3.225_598 * l + 4.265_291;
    (damage as f32 / c * if taken { 2.5 } else { 7.5 } * by * 10.0) as u32
}

/// `SMSG_ATTACKERSTATEUPDATE`: its hit info a set of flags, one damage
/// line. `damage` is what got through, after absorbs and resists; `school`
/// its school's index. `hit_info` never has the debug flag (0x1), whose
/// trailer the client would then read.
#[allow(clippy::too_many_arguments)]
pub fn attacker_state_full(
    attacker: Guid,
    victim: Guid,
    hit_info: u32,
    damage: u32,
    victim_state: u32,
    blocked: u32,
    absorbed: u32,
    resisted: u32,
    school: u32,
) -> Arc<[u8]> {
    use wow_world_messages::vanilla::{
        DamageInfo, SMSG_ATTACKERSTATEUPDATE, SMSG_ATTACKERSTATEUPDATE_HitInfo,
    };
    encode(&SMSG_ATTACKERSTATEUPDATE {
        hit_info: SMSG_ATTACKERSTATEUPDATE_HitInfo::new(hit_info, None),
        attacker,
        target: victim,
        total_damage: damage,
        damages: vec![DamageInfo {
            spell_school_mask: school,
            damage_float: damage as f32,
            damage_uint: damage,
            absorb: absorbed,
            resist: resisted,
        }],
        damage_state: victim_state,
        unknown1: if absorbed > 0 { u32::MAX } else { 0 },
        spell_id: 0,
        blocked_amount: blocked,
    })
    .expect("a blow encodes")
}

pub fn attacker_state(
    attacker: Guid,
    victim: Guid,
    hit_info: u32,
    damage: u32,
    victim_state: u32,
    blocked: u32,
) -> Arc<[u8]> {
    attacker_state_full(
        attacker,
        victim,
        hit_info,
        damage,
        victim_state,
        blocked,
        0,
        0,
        0,
    )
}

/// What the map knows of a unit, for a fight.
#[derive(Debug, Clone, Copy)]
pub struct UnitView {
    pub guid: Guid,
    pub position: Vector3d,
    pub orientation: f32,
    pub level: u32,
    pub faction: u32,
    pub combat_reach: f32,
    pub alive: bool,
    pub is_player: bool,
    pub standing: bool,
    /// Going somewhere at a run (not walking): two running units swing at
    /// each other with a leeway.
    pub running: bool,
}

impl Map {
    pub fn unit_view(&self, g: Guid) -> Option<UnitView> {
        if let Some(p) = self.players.get(&g) {
            return Some(UnitView {
                guid: g,
                position: p.position(),
                orientation: p.movement.orientation,
                level: u32::from(p.level()),
                faction: p.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
                combat_reach: p.fields.get_f32(UNIT_FIELD_COMBATREACH),
                alive: p.death == Death::Alive,
                is_player: true,
                standing: p.stand_state() == 0,
                running: crate::game::map::spells::checks::moving(&p.movement.flags)
                    && !p.movement.flags.get_walk_mode(),
            });
        }
        let c = self.creature(g)?;
        Some(UnitView {
            guid: g,
            position: c.position,
            orientation: c.orientation,
            level: c.fields.get_u32(UNIT_FIELD_LEVEL),
            faction: c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
            combat_reach: c.fields.get_f32(UNIT_FIELD_COMBATREACH),
            alive: c.alive(),
            is_player: false,
            standing: true,
            running: matches!(&c.motion, crate::game::map::creature::Motion::Moving(w) if w.run),
        })
    }

    pub fn unit_fields_mut(&mut self, g: Guid) -> Option<&mut Fields> {
        if self.players.contains_key(&g) {
            return self.players.get_mut(&g).map(|p| &mut p.fields);
        }
        self.creature_mut(g).map(|c| &mut c.fields)
    }

    pub fn combat_mut(&mut self, g: Guid) -> Option<&mut Combat> {
        if self.players.contains_key(&g) {
            return self.players.get_mut(&g).map(|p| &mut p.combat);
        }
        // What changes a creature's fight may put it in one.
        self.note_fighting(g);
        self.creature_mut(g).map(|c| &mut c.combat)
    }

    pub fn combat(&self, g: Guid) -> Option<&Combat> {
        if let Some(p) = self.players.get(&g) {
            return Some(&p.combat);
        }
        self.creature(g).map(|c| &c.combat)
    }
}

/// Whether `a` may attack `b`.
pub fn can_attack(map: &Map, a: Guid, b: Guid) -> bool {
    can_attack_alive_or_not(map, a, b, false)
}

/// [`can_attack`], of a spell at `b` that may be cast at the dead: a
/// corpse of an enemy is one still (mangos's `CanAttackSpell` with
/// `SPELL_ATTR_EX2_ALLOW_DEAD_TARGET`).
pub fn can_attack_dead(map: &Map, a: Guid, b: Guid) -> bool {
    can_attack_alive_or_not(map, a, b, true)
}

/// Whether `a` may aim an attack at `b`, a swing or a spell: as
/// [`can_attack`], but a creature on its way home may be aimed at, and what
/// lands on it evades (mangos-classic's `CanAttack`, which doesn't ask its
/// evade, and its `VICTIMSTATE_EVADES` and `SPELL_MISS_EVADE`). Not
/// "Invalid target".
pub fn can_aim_at(map: &Map, a: Guid, b: Guid) -> bool {
    can_attack_of(map, a, b, false, true)
}

fn can_attack_alive_or_not(map: &Map, a: Guid, b: Guid, dead_ok: bool) -> bool {
    can_attack_of(map, a, b, dead_ok, false)
}

fn can_attack_of(map: &Map, a: Guid, b: Guid, dead_ok: bool, evading_ok: bool) -> bool {
    if a == b {
        return false;
    }
    let (Some(ua), Some(ub)) = (map.unit_view(a), map.unit_view(b)) else {
        return false;
    };
    if !ua.alive || (!ub.alive && !dead_ok) {
        return false;
    }
    // One no one may attack: not attackable, made so by an aura, not
    // selectable, or on a flight (mangos's `Unit::CanAttack`, Thistle
    // Tea's `targetable?`); and a creature on its way home.
    if map
        .unit_fields(b)
        .is_some_and(|f| f.get_u32(UNIT_FIELD_FLAGS) & UNIT_FLAGS_NOT_ATTACKABLE != 0)
        || (!evading_ok && map.creature(b).is_some_and(|c| c.evading))
    {
        return false;
    }
    // Gurubashi Arena's battle has its own rules for its fighters.
    if let Some(allowed) = crate::game::map::arena::can_attack(map, a, b) {
        return allowed;
    }
    // A pet, totem or guardian fights on its master's side; a possessed
    // player on their possessor's.
    let (ma, mb) = (
        crate::game::map::pets::master(map, a),
        crate::game::map::pets::master(map, b),
    );
    if ma == mb {
        return false;
    }
    let (player_a, player_b) = (map.players.contains_key(&ma), map.players.contains_key(&mb));
    // A unit immune to players' side (0x100) or to creatures' (0x200)
    // neither attacks it nor is attacked by it (Thistle Tea's
    // `attack_flags_allow?`, mangos's `Unit::CanAttack`).
    let immune = |g: Guid, to_player: bool| {
        let flag = if to_player {
            UNIT_FLAG_IMMUNE_TO_PLAYER
        } else {
            UNIT_FLAG_IMMUNE_TO_NPC
        };
        map.unit_fields(g)
            .is_some_and(|f| f.get_u32(UNIT_FIELD_FLAGS) & flag != 0)
    };
    if immune(b, player_a) || immune(a, player_b) {
        return false;
    }
    if player_a && player_b {
        let (a, b) = (ma, mb);
        if crate::game::map::duel::dueling(map, a, b) {
            return true;
        }
        let (pa, pb) = (&map.players[&a], &map.players[&b]);
        let ffa = |p: &crate::game::map::player::Player| {
            p.fields.has_flag(PLAYER_FLAGS, PLAYER_FLAGS_FFA_PVP)
        };
        // Free for all, but not for one's own group (mangos's
        // `GetReactionTo`: a group is friendly first).
        if ffa(pa) && ffa(pb) {
            let groups = map.realm.social.groups.lock();
            return !matches!(
                (groups.group_of(a), groups.group_of(b)),
                (Some(x), Some(y)) if x.id == y.id
            );
        }
        // The one attacked must be flagged (contested is flagged too), as
        // mangos's `Unit::CanAttack` and Thistle Tea's
        // `pvp_attack_allowed?` have it: a flag doesn't let one attack.
        return pa.team != pb.team && pb.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP);
    }
    let data = &map.realm.data;
    // Without faction data, anything may be attacked.
    if data.faction_templates.get(&ua.faction).is_none()
        || data.faction_templates.get(&ub.faction).is_none()
    {
        return true;
    }
    // Hostile either way, or for a player's side a neutral creature whose
    // faction keeps no standing (or one an aura forces): a faction with
    // standings is friendly but at war (Thistle Tea's
    // `attack_reaction_allows?`, mangos's `GetReactionTo`). Two creatures
    // must be enemies.
    // The other way only if this way isn't hostile (review 5's combat 4:
    // each scans a player's auras for a forced reaction).
    let ab = regard(map, a, b);
    if ab == Regard::Hostile {
        return true;
    }
    match (ab, regard(map, b, a)) {
        (_, Regard::Hostile) => true,
        (Regard::Friendly, _) | (_, Regard::Friendly) => false,
        _ if player_a != player_b => {
            let (creature, player) = if player_a { (b, ma) } else { (a, mb) };
            crate::game::map::reputation::faction_of(map, creature).is_none_or(|f| {
                data.misc.faction(f).is_none() || forced_rank(&map.players[&player], f).is_some()
            })
        }
        _ => false,
    }
}

/// Whether an area spell of `a`'s may hit `b`: as [`can_attack`], but a
/// player's area spares players unless the caster is flagged too, they
/// duel, or both fight free for all; so no one is flagged by an area they
/// didn't mean (mangos's `CanAttackSpell` for an area, Thistle Tea's
/// `can_attack_without_flagging?`).
pub fn can_attack_in_area(map: &Map, a: Guid, b: Guid) -> bool {
    if !can_attack(map, a, b) {
        return false;
    }
    let (ma, mb) = (
        crate::game::map::pets::master(map, a),
        crate::game::map::pets::master(map, b),
    );
    let (Some(pa), Some(pb)) = (map.players.get(&ma), map.players.get(&mb)) else {
        return true;
    };
    let ffa = |p: &crate::game::map::player::Player| {
        p.fields.has_flag(PLAYER_FLAGS, PLAYER_FLAGS_FFA_PVP)
    };
    crate::game::map::duel::dueling(map, ma, mb)
        || (ffa(pa) && ffa(pb))
        || pa.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP)
}

/// Whether `a` may help `b` (a heal, a buff): no one outside a duel helps
/// its fighters, nor they anyone outside it, as mangos's `Unit::CanAssist`
/// has it by the duel's arbiter (Thistle Tea's `can_assist?`).
pub fn can_assist(map: &Map, a: Guid, b: Guid) -> bool {
    let (ma, mb) = (
        crate::game::map::pets::master(map, a),
        crate::game::map::pets::master(map, b),
    );
    if ma == mb {
        return true;
    }
    // One immune to players' side takes no help from it, though a
    // creature's it takes (Thistle Tea's `can_assist?`; mangos's
    // `CanAssist` helps no such NPC of a player's either).
    if guid::is_player(ma)
        && map
            .unit_fields(b)
            .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_PLAYER))
    {
        return false;
    }
    // The duel's arbiter and side: a duel's two sides help neither each
    // other nor anyone outside it (mangos's `PLAYER_DUEL_ARBITER` and
    // `PLAYER_DUEL_TEAM`).
    let arbiter = |g: Guid| {
        map.players.get(&g).and_then(|p| p.duel).and_then(|d| {
            (d.phase == crate::game::map::duel::Phase::Fighting).then_some((d.flag, d.initiator))
        })
    };
    match (map.players.contains_key(&ma), map.players.contains_key(&mb)) {
        (true, true) => arbiter(ma) == arbiter(mb),
        _ => true,
    }
}

/// The unit flags no one attacks through: not attackable, not attackable
/// by an aura, not selectable, on a flight.
pub const UNIT_FLAGS_NOT_ATTACKABLE: u32 = 0x2 | 0x1_0000 | 0x0200_0000 | 0x0010_0000;

/// The unit flags that make a unit immune to players' side and to
/// creatures'.
pub const UNIT_FLAG_IMMUNE_TO_PLAYER: u32 = 0x0000_0100;
pub const UNIT_FLAG_IMMUNE_TO_NPC: u32 = 0x0000_0200;

/// How one unit regards another, as Thistle Tea ranks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Regard {
    Hostile,
    Neutral,
    Friendly,
}

/// Whether `a` is hostile to `b`, and so attacks it on sight.
pub fn hostile(map: &Map, a: Guid, b: Guid) -> bool {
    regard(map, a, b) == Regard::Hostile
}

/// Whether `a` is friendly to `b`.
pub fn friendly(map: &Map, a: Guid, b: Guid) -> bool {
    regard(map, a, b) == Regard::Friendly
}

/// How `a` regards `b` (Thistle Tea's `Hostility.reaction_rank`): units of
/// one master are friends; between a creature and a player (or one of a
/// player's), a rank an aura forces, a guard of contested ground's on one
/// flagged for it, else the player's standing with the creature's faction
/// if it has one; else their faction templates.
pub fn regard(map: &Map, a: Guid, b: Guid) -> Regard {
    use crate::game::data::misc::{RANK_FRIENDLY, RANK_HOSTILE, RANK_NEUTRAL};
    let (Some(ua), Some(ub)) = (map.unit_view(a), map.unit_view(b)) else {
        return Regard::Neutral;
    };
    let (ma, mb) = (
        crate::game::map::pets::master(map, a),
        crate::game::map::pets::master(map, b),
    );
    if ma == mb {
        return Regard::Friendly;
    }
    let data = &map.realm.data;
    let of_rank = |rank: u32| {
        if rank <= RANK_HOSTILE {
            Regard::Hostile
        } else if rank >= RANK_FRIENDLY {
            Regard::Friendly
        } else {
            Regard::Neutral
        }
    };
    let (pa, pb) = (map.players.get(&ma), map.players.get(&mb));
    // A creature and a player's side: the player's standing decides.
    let creature_player = match (pa, pb) {
        (None, Some(p)) if !ua.is_player => Some((a, p, false)),
        (Some(p), None) if !ub.is_player => Some((b, p, true)),
        _ => None,
    };
    if let Some((creature, p, player_first)) = creature_player
        && let Some(faction) = crate::game::map::reputation::faction_of(map, creature)
    {
        if let Some(rank) = forced_rank(p, faction) {
            return of_rank(rank);
        }
        let template = map
            .unit_view(creature)
            .and_then(|u| data.faction_templates.get(&u.faction));
        if template.is_some_and(|t| t.flags & FACTION_ATTACKS_CONTESTED != 0)
            && p.fields.has_flag(
                PLAYER_FLAGS,
                crate::game::map::pvp::PLAYER_FLAGS_CONTESTED_PVP,
            )
        {
            return Regard::Hostile;
        }
        if data.misc.faction(faction).is_some() {
            let at_war = crate::game::map::reputation::at_war(p, data, faction);
            if player_first {
                return if at_war {
                    Regard::Hostile
                } else {
                    Regard::Friendly
                };
            }
            let rank = crate::game::map::reputation::rank(p, data, faction);
            // At war while friendly or better: neutral.
            return if at_war && rank >= RANK_FRIENDLY {
                of_rank(RANK_NEUTRAL)
            } else {
                of_rank(rank)
            };
        }
    }
    match (
        data.faction_templates.get(&ua.faction),
        data.faction_templates.get(&ub.faction),
    ) {
        (Some(fa), Some(fb)) if fa.hostile_to(fb) => Regard::Hostile,
        (Some(fa), Some(fb)) if fa.friendly_to(fb) || fb.friendly_to(fa) => Regard::Friendly,
        _ => Regard::Neutral,
    }
}

/// A faction template's guards attack players flagged on contested ground.
const FACTION_ATTACKS_CONTESTED: u32 = 0x1000;

/// The rank a player's auras force with `faction`, if any.
fn forced_rank(p: &crate::game::map::player::Player, faction: u32) -> Option<u32> {
    p.auras
        .effects()
        .find(|(_, a)| {
            a.kind == crate::game::map::auras::AURA_FORCE_REACTION && a.misc == faction as i32
        })
        .map(|(_, a)| a.amount.clamp(0, 7) as u32)
}

pub fn dist(a: Vector3d, b: Vector3d) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

/// The distance two units may swing at each other from: both reaches and
/// four thirds of a yard, at least five.
pub fn melee_range(a: &UnitView, b: &UnitView) -> f32 {
    (a.combat_reach + b.combat_reach + 4.0 / 3.0).max(5.0)
}

/// What two units both running may swing from beyond their reach, as
/// mangos (`MELEE_LEEWAY`) and the client have it.
pub const MELEE_LEEWAY: f32 = 8.0 / 3.0;

/// Whether `a` reaches `b` with a swing: within their melee range, as the
/// client measures it (Thistle Tea's `in_melee_range?`), and a leeway when
/// both run. A creature swinging from further than that hits a player
/// whose own swing the client says is out of range.
pub fn in_melee_range(a: &UnitView, b: &UnitView) -> bool {
    let leeway = if a.running && b.running {
        MELEE_LEEWAY
    } else {
        0.0
    };
    dist(a.position, b.position) <= melee_range(a, b) + leeway
}

/// How far from its victim a creature chases to, for a melee range of
/// `reach`: half of it (mangos-classic's `CHASE_DEFAULT_RANGE_FACTOR`), so
/// that a victim walking on has half the reach to go before it is out of
/// reach again. Aimed at the reach's edge (Thistle Tea's half a yard
/// inside it), every half yard its victim stepped sent it a new path.
pub fn chase_distance(reach: f32) -> f32 {
    (reach * 0.5).max(0.0)
}

/// Within this of its victim, an attack owed needn't face it (Thistle
/// Tea's overlap distance).
const OVERLAP_YARDS: f32 = 1.4;

/// Whether `a` faces `b` within `arc` radians.
pub fn in_arc(a: &UnitView, b: &UnitView, arc: f32) -> bool {
    let angle = (b.position.y - a.position.y).atan2(b.position.x - a.position.x);
    let mut diff = (angle - a.orientation).rem_euclid(std::f32::consts::TAU);
    if diff > std::f32::consts::PI {
        diff = std::f32::consts::TAU - diff;
    }
    diff <= arc / 2.0
}

pub fn facing(a: &UnitView, b: &UnitView) -> bool {
    in_arc(a, b, FACING_ARC)
}

/// Whether `attacker` stands behind `victim`: outside its front half.
pub fn behind(victim: &UnitView, attacker: &UnitView) -> bool {
    !in_arc(victim, attacker, std::f32::consts::PI)
}

/// `g` is in combat, now: its flag, and what a fight's start stops. A
/// player-controlled unit fighting another player's side (`pvp`) has the
/// PvP timer set again; a player's fight is looked at for its end at the
/// next update after it began. Only the living fight (mangos's
/// `SetInCombatState`).
pub fn enter(map: &mut Map, g: Guid, pvp: bool) {
    let now = map.now;
    // Only the living are in a fight (mangos's `SetInCombatState`): a
    // blow that comes after a death doesn't put the corpse in one (Thistle
    // Tea's `die/1`).
    if !map.unit_view(g).is_some_and(|u| u.alive) {
        return;
    }
    let Some(f) = map.unit_fields_mut(g) else {
        return;
    };
    let fresh = !f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT);
    f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, true);
    if let Some(c) = map.combat_mut(g) {
        c.last_action = Some(now);
        if pvp {
            c.combat_until = Some(now + PVP_COMBAT);
        }
        if fresh || c.next_check.is_none() {
            c.next_check = Some(now);
        }
    }
    if fresh {
        crate::game::map::spells::combat_began(map, g);
    }
}

/// Whether a fight between `a` and `b` is one between two players' sides:
/// it runs the PvP timer (mangos's `SetInCombatWith`, both units
/// `UNIT_FLAG_PLAYER_CONTROLLED`).
fn pvp_fight(map: &Map, a: Guid, b: Guid) -> bool {
    let (ma, mb) = (
        crate::game::map::pets::master(map, a),
        crate::game::map::pets::master(map, b),
    );
    guid::is_player(ma) && guid::is_player(mb) && ma != mb
}

/// A harmful act of `attacker`'s landed on `victim`: the players' sides of
/// it are in the fight, a pet's master with it (mangos's
/// `SetInCombatWithAggressor` and `SetInCombatWithVictim`, the controller
/// joining in `SetInCombatState`). A creature's own side is its AI's
/// ([`crate::game::map::ai::attacked`]).
pub fn struck(map: &mut Map, attacker: Guid, victim: Guid) {
    if attacker == victim {
        return;
    }
    let pvp = pvp_fight(map, attacker, victim);
    // A master who can't join it (lying in a believed Feign Death, out of
    // every fight) isn't drawn in by its pet's blows (mangos's
    // `CanJoinInAttacking`).
    let mut sides = vec![attacker, victim];
    sides.extend(crate::game::map::pets::drawn_in(map, attacker));
    sides.extend(crate::game::map::pets::drawn_in(map, victim));
    sides.sort_unstable();
    sides.dedup();
    for g in sides {
        if guid::is_player(crate::game::map::pets::master(map, g)) {
            enter(map, g, pvp);
        }
    }
}

/// `helper` helped `helped` (a heal, a buff): if `helped` is in a fight,
/// so is the helper, and on the PvP timer if `helped` is another player's
/// side flagged for PvP (mangos's `SetInCombatWithAssisted`). What keeps
/// the helper there after is the threat its help made.
pub fn assisted(map: &mut Map, helper: Guid, helped: Guid) {
    if helper == helped
        || !guid::is_player(crate::game::map::pets::master(map, helper))
        || !map
            .unit_fields(helped)
            .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT))
    {
        return;
    }
    let pvp = pvp_fight(map, helper, helped)
        && map
            .unit_fields(helped)
            .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP));
    enter(map, helper, pvp);
    // Helping one contested contests the helper: the guards come for a
    // healer of the one they hunt (mangos's `SetInCombatWithAssisted`,
    // `UpdatePvPContested`).
    let side = crate::game::map::pets::master(map, helped);
    let contested = map.players.get(&side).is_some_and(|p| {
        p.fields.has_flag(
            PLAYER_FLAGS,
            crate::game::map::pvp::PLAYER_FLAGS_CONTESTED_PVP,
        )
    });
    if pvp && contested && guid::is_player(helper) {
        crate::game::map::pvp::contest(map, helper);
    }
}

/// `g`'s fight ends, and its pet's, guardians' and charmed unit's
/// (mangos's `CombatStopWithPets`).
pub fn stop_with_pets(map: &mut Map, g: Guid) {
    leave_combat(map, g);
    let owned: Vec<Guid> = map
        .players
        .get(&g)
        .map(|p| {
            p.owned
                .pet
                .into_iter()
                .chain(p.owned.charm)
                .chain(p.owned.guardians.iter().copied())
                .collect()
        })
        .unwrap_or_default();
    for u in owned {
        leave_combat(map, u);
    }
}

/// Both units are in combat, now.
pub fn engage(map: &mut Map, a: Guid, b: Guid) {
    let pvp = pvp_fight(map, a, b);
    let (ma, mb) = (
        crate::game::map::pets::master(map, a),
        crate::game::map::pets::master(map, b),
    );
    for (x, y) in [(a, b), (b, a)] {
        // The dead take no foe.
        if !map.unit_view(x).is_some_and(|u| u.alive) {
            continue;
        }
        if let Some(c) = map.combat_mut(x) {
            c.attackers.insert(y);
        }
        enter(map, x, pvp);
    }
    // A pet's fight is its master's (mangos's `SetInCombatState`), but not
    // one who can't join it (feigning death, out of every fight).
    for (u, m) in [(a, ma), (b, mb)] {
        if m != a && m != b && crate::game::map::pets::drawn_in(map, u) == Some(m) {
            enter(map, m, pvp);
        }
    }
    // A blow on a flagged unit flags the attacker; a flagged player's blow
    // flags the one struck. A pet's blows count as its master's.
    pvp_contact(map, ma, mb, Contact::Attack);
    pvp_contact(map, mb, ma, Contact::Attacked);
    if guid::is_unit(b) {
        crate::game::map::ai::attacked(map, b, a);
        // A blow, landed or not, keeps it on its leash (Thistle Tea's
        // contact on `receive_attack`).
        crate::game::map::creature_combat::hostile_act(map, b);
    }
}

fn send_to(map: &Map, g: Guid, frame: Option<Arc<[u8]>>) {
    if let (Some(p), Some(f)) = (map.players.get(&g), frame) {
        p.outbound.send_frame(f);
    }
}

/// A unit's derived stats.
pub fn derived(map: &Map, g: Guid) -> crate::game::map::derive::Derived {
    if let Some(p) = map.players.get(&g) {
        return p.derived;
    }
    map.creature(g).map(|c| c.derived).unwrap_or_default()
}

/// The unit's creature type as a mask, for the auras against one: a player
/// is a humanoid.
pub fn creature_type_mask(map: &Map, g: Guid) -> u32 {
    match creature_type(map, g) {
        0 => 0,
        t => 1 << (t - 1).min(31),
    }
}

/// The unit's creature type (0 for none): a player is a humanoid (7), but
/// in a form `SpellShapeshiftForm` makes a beast (1: cat, bear, dire bear,
/// travel, aquatic, ghost wolf), as mangos's `GetCreatureType` and Thistle
/// Tea's have it.
pub fn creature_type(map: &Map, g: Guid) -> u32 {
    if let Some(p) = map.players.get(&g) {
        let form = p.fields.get_byte(UNIT_FIELD_BYTES_1, 2);
        return if matches!(form, 1 | 3 | 4 | 5 | 8 | 14 | 15 | 16) {
            1
        } else {
            7
        };
    }
    map.creature(g)
        .and_then(|c| map.realm.data.creature_template(c.entry))
        .map_or(0, |t| t.creature_type)
}

fn flags_extra(map: &Map, g: Guid) -> u32 {
    map.creature(g)
        .and_then(|c| map.realm.data.creature_template(c.entry))
        .map_or(0, |t| t.flags_extra)
}

/// A creature template's `static_flags1`.
fn static_flags(map: &Map, g: Guid) -> u32 {
    map.creature(g)
        .and_then(|c| map.realm.data.creature_template(c.entry))
        .map_or(0, |t| t.static_flags1)
}

/// `static_flags1`: it can't be killed, only brought to one health; it has
/// no defense skill.
pub const STATIC_UNKILLABLE: u32 = 0x8;
pub const STATIC_NO_DEFENSE: u32 = 0x4000;

/// Whether a unit wields two weapons: a player who can, or a creature
/// whose off hand holds a weapon (not a shield or a held item).
pub fn dual_wields(map: &Map, g: Guid) -> bool {
    if let Some(p) = map.players.get(&g) {
        return p.derived.dual_wield;
    }
    map.creature(g).is_some_and(|c| {
        let info = c.fields.get_u32(UNIT_VIRTUAL_ITEM_INFO + 2);
        let (class, inventory_type) = (info & 0xFF, info >> 24);
        c.fields.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + 1) != 0
            && class == 2
            && !matches!(inventory_type, 14 | 23)
            && !c.auras.has(AURA_MOD_DISARM)
    })
}

/// Whether a school (a mask) can't touch a unit: its auras' immunities
/// (Divine Shield, Ice Block) or a creature's template's.
pub fn school_immune(map: &Map, g: Guid, school: u32) -> bool {
    auras::auras(map, g).is_some_and(|a| a.school_immune(school))
        || crate::game::map::creature_combat::school_immune(map, g, school)
}

/// The weapon in a player's hand.
pub fn hand_weapon(map: &Map, g: Guid, hand: Hand) -> Option<crate::game::map::derive::Weapon> {
    let p = map.players.get(&g)?;
    let gear = crate::game::map::derive::gear(p, &map.realm.data);
    match hand {
        Hand::Main => gear.main,
        Hand::Off => gear.off,
        Hand::Ranged => gear.ranged,
    }
}

/// Whether `g` is a player or under a player's control: a pet, a guardian,
/// a charm (mangos's `UNIT_FLAG_PLAYER_CONTROLLED`).
pub fn player_controlled(map: &Map, g: Guid) -> bool {
    guid::is_player(g)
        || map.unit_fields(g).is_some_and(|f| {
            f.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::pets::UNIT_FLAG_PLAYER_CONTROLLED,
            )
        })
        || crate::game::map::pets::owner_of(map, g).is_some_and(guid::is_player)
}

/// Whether a creature charms `g`.
fn charmed_by_creature(map: &Map, g: Guid) -> bool {
    map.unit_fields(g).is_some_and(|f| {
        let by = f.get_guid(UNIT_FIELD_CHARMEDBY);
        !by.is_zero() && !guid::is_player(by)
    })
}

/// Whether `g`'s blows may glance: a player's side's, not a creature's
/// charm (mangos's `CanGlance`).
pub fn glances(map: &Map, g: Guid) -> bool {
    player_controlled(map, g) && !charmed_by_creature(map, g)
}

/// Whether `g`'s blows may crush: the creatures' side's, not one a player
/// masters, nor one of `NO_CRUSHING_BLOWS` (mangos's `CanCrush` and
/// `CanCrushInCombat`).
pub fn crushes(map: &Map, g: Guid) -> bool {
    let side = if player_controlled(map, g) {
        charmed_by_creature(map, g)
    } else {
        !guid::is_player(crate::game::map::pets::master(map, g))
    };
    side && map
        .creature(g)
        .and_then(|c| map.realm.data.creature_template(c.entry))
        .is_none_or(|t| t.static_flags2 & STATIC2_NO_CRUSHING_BLOWS == 0)
}

/// `static_flags2`: it deals no crushing blows; it trains no weapon skill.
pub const STATIC2_NO_CRUSHING_BLOWS: u32 = 0x10;
pub const STATIC2_NO_SKILL_GAINS: u32 = 0x4000_0000;

/// The hit table's inputs for `attacker` against `victim` with `hand`.
pub fn roll_for(map: &Map, attacker: Guid, victim: Guid, hand: Hand) -> Option<Roll> {
    let (va, vb) = (map.unit_view(attacker)?, map.unit_view(victim)?);
    let (da, dv) = (derived(map, attacker), derived(map, victim));
    let or_level = |v: u32, level: u32| if v == 0 { level * 5 } else { v } as i32;
    let ranged = hand == Hand::Ranged;
    // Casting, or crowd controlled: stunned, confused or fleeing (mangos's
    // `CanReactInCombat`, `IsCrowdControlled`).
    let held = |g: Guid| {
        map.caster(g).is_some_and(|c| c.casting.is_some())
            || map.unit_fields(g).is_some_and(|f| {
                f.get_u32(UNIT_FIELD_FLAGS)
                    & (crate::game::map::player::UNIT_FLAG_STUNNED
                        | auras::UNIT_FLAG_CONFUSED
                        | auras::UNIT_FLAG_FLEEING)
                    != 0
            })
    };
    let extra = flags_extra(map, victim);
    // The victim's auras on those attacking it (Thistle Tea's
    // `attacker_hit_debuff` and `attacker_crit_bonus`).
    let (hit_kind, crit_kind) = if ranged {
        (
            AURA_MOD_ATTACKER_RANGED_HIT_CHANCE,
            AURA_MOD_ATTACKER_RANGED_CRIT_CHANCE,
        )
    } else {
        (
            AURA_MOD_ATTACKER_MELEE_HIT_CHANCE,
            AURA_MOD_ATTACKER_MELEE_CRIT_CHANCE,
        )
    };
    let (attacker_hit, attacker_crit) = auras::auras(map, victim).map_or((0.0, 0.0), |a| {
        (a.sum(hit_kind) as f32, a.sum(crit_kind) as f32)
    });
    // A player's weapons put away can't block.
    let sheathed = map
        .players
        .get(&victim)
        .is_some_and(|p| p.fields.get_byte(UNIT_FIELD_BYTES_2, 0) == 0);
    // Each hand's own skill; against a player, a player's skills are at
    // least their caps (mangos's `GetWeaponSkillValue` and
    // `GetDefenseSkillValue`). A player's skill is as it is, 0 too.
    let pvp = va.is_player && vb.is_player;
    let skill = match (hand, pvp) {
        (Hand::Main, false) => da.weapon_skill,
        (Hand::Main, true) => da.pvp_weapon_skill,
        (Hand::Off, false) => da.off_skill,
        (Hand::Off, true) => da.pvp_off_skill,
        (Hand::Ranged, false) => da.ranged_skill,
        (Hand::Ranged, true) => da.pvp_ranged_skill,
    };
    let defense = if pvp { dv.pvp_defense } else { dv.defense };
    Some(Roll {
        attacker_skill: if va.is_player {
            skill as i32
        } else {
            or_level(skill, va.level)
        },
        // A creature without defense meets every blow with none (Thistle
        // Tea's `Skills.defense_value`).
        victim_defense: if static_flags(map, victim) & STATIC_NO_DEFENSE != 0 {
            0
        } else if vb.is_player {
            defense as i32
        } else {
            or_level(defense, vb.level)
        },
        attacker_is_player: glances(map, attacker),
        victim_is_player: player_controlled(map, victim),
        victim_creature: !vb.is_player,
        no_crush: !crushes(map, attacker),
        attacker_level: va.level as i32,
        victim_level: vb.level as i32,
        crit: if ranged { da.ranged_crit } else { da.crit } + attacker_crit,
        hit: match hand {
            Hand::Main => da.hit,
            Hand::Off => da.off_hit,
            Hand::Ranged => da.ranged_hit,
        },
        dodge: dv.dodge,
        parry: dv.parry,
        block: dv.block,
        from_behind: behind(&vb, &va),
        standing: vb.standing,
        avoidance_disabled: held(victim),
        dual_wield: !ranged
            && dual_wields(map, attacker)
            && map.combat(attacker).is_some_and(|c| c.queued.is_none()),
        special: false,
        ranged,
        can_crit: true,
        no_parry: extra & FLAG_EXTRA_NO_PARRY != 0,
        no_block: extra & FLAG_EXTRA_NO_BLOCK != 0,
        always_crush: flags_extra(map, attacker) & FLAG_EXTRA_ALWAYS_CRUSH != 0,
        attacker_hit,
        sheathed,
        magic: !ranged && melee_school(map, attacker) & 1 == 0,
    })
}

/// The school of a unit's blows, as a mask: a creature's may be magic.
pub fn melee_school(map: &Map, g: Guid) -> u32 {
    map.creature(g)
        .and_then(|c| map.realm.data.creature_template(c.entry))
        .map_or(1, |t| 1 << t.damage_school.min(6))
}

/// The flat damage `attacker` does more against `victim`'s creature type
/// (aura 59: a weapon's "+damage against beasts").
pub fn damage_versus(map: &Map, attacker: Guid, victim: Guid) -> f32 {
    let kind = creature_type_mask(map, victim);
    if kind == 0 {
        return 0.0;
    }
    auras::auras(map, attacker).map_or(0.0, |a| {
        a.effects()
            .filter(|(_, x)| x.kind == AURA_MOD_DAMAGE_DONE_CREATURE && (x.misc as u32) & kind != 0)
            .map(|(h, x)| (x.amount * h.stacks.max(1) as i32) as f32)
            .sum()
    })
}

/// A player's physical damage done percentage for `hand`, the weapon in it
/// fitting and the off hand's half and its bonus with it, as its flat
/// bonuses take it (mangos's hand `TOTAL_PCT`); 1 for a creature.
pub fn hand_damage_pct(map: &Map, attacker: Guid, hand: Hand) -> f32 {
    if !map.players.contains_key(&attacker) {
        return 1.0;
    }
    let Some(a) = auras::auras(map, attacker) else {
        return 1.0;
    };
    let weapon = hand_weapon(map, attacker, hand);
    let mut pct = crate::game::map::derive::physical_pct_for(a, weapon.as_ref());
    if hand == Hand::Off {
        pct *= 0.5 * (1.0 + a.sum(AURA_MOD_OFFHAND_DAMAGE_PCT) as f32 / 100.0);
    }
    pct
}

/// A blow's weapon damage before armor, with what the attacker deals more
/// and the victim takes more: (damage, school mask).
pub fn weapon_damage(
    map: &Map,
    attacker: Guid,
    victim: Guid,
    hand: Hand,
    rng: &mut impl rand::Rng,
) -> (u32, u32) {
    let Some(f) = map.unit_fields(attacker) else {
        return (1, 1);
    };
    let (lo, hi, period) = match hand {
        Hand::Main => (
            UNIT_FIELD_MINDAMAGE,
            UNIT_FIELD_MAXDAMAGE,
            UNIT_FIELD_BASEATTACKTIME,
        ),
        // A creature's off hand strikes for half its main hand.
        Hand::Off if !map.players.contains_key(&attacker) => (
            UNIT_FIELD_MINDAMAGE,
            UNIT_FIELD_MAXDAMAGE,
            UNIT_FIELD_BASEATTACKTIME + 1,
        ),
        Hand::Off => (
            UNIT_FIELD_MINOFFHANDDAMAGE,
            UNIT_FIELD_MAXOFFHANDDAMAGE,
            UNIT_FIELD_BASEATTACKTIME + 1,
        ),
        Hand::Ranged => (
            UNIT_FIELD_MINRANGEDDAMAGE,
            UNIT_FIELD_MAXRANGEDDAMAGE,
            UNIT_FIELD_RANGEDATTACKTIME,
        ),
    };
    let min = f.get_f32(lo).max(1.0);
    let max = f.get_f32(hi).max(min);
    // Attack power's bonuses go by the hand's speed before haste: its
    // weapon's, or a creature's own (mangos's `GetAPMultiplier` of
    // `GetAttackTime`, the field over `m_modAttackSpeedPct`; Thistle Tea's
    // "offhand speed, penalty, and damage modifiers without haste").
    let unhasted = hand_weapon(map, attacker, hand)
        .map(|w| w.delay)
        .or_else(|| {
            map.creature(attacker)
                .filter(|_| hand != Hand::Ranged)
                .map(|c| c.base.attack_time)
        })
        .filter(|t| *t > 0)
        .unwrap_or_else(|| f.get_u32(period));
    let speed = unhasted.max(100) as f32 / 1000.0;
    let rolled = if max > min {
        rng.random_range(min..=max)
    } else {
        min
    };
    let school = if hand == Hand::Ranged {
        1
    } else {
        melee_school(map, attacker)
    };
    let mut damage = rolled;
    if hand == Hand::Off && !map.players.contains_key(&attacker) {
        damage *= 0.5;
    }
    if let Some(a) = auras::auras(map, attacker) {
        // A blow of a magic school (an elemental's) takes its school's
        // share of damage done, not the physical share its fields hold
        // (Thistle Tea's `attack_damage_multipliers`).
        let school_pct = if school & 1 == 0 {
            let physical = a.product_mask(AURA_MOD_DAMAGE_PERCENT_DONE, 1);
            if physical > 0.0 {
                damage /= physical;
            }
            a.product_mask(AURA_MOD_DAMAGE_PERCENT_DONE, school)
        } else {
            1.0
        };
        let weapon = hand_weapon(map, attacker, hand);
        let is_player = map.players.contains_key(&attacker);
        // A player's hand holds its flat physical damage done in its
        // fields already (`derive::physical_flat_for`), but a wand's.
        let in_fields = is_player
            && !weapon.as_ref().is_some_and(|w| {
                hand == Hand::Ranged && w.subclass == crate::game::map::derive::SUBCLASS_WAND
            });
        // A hunter's pet's mood scales its flat damage done with its
        // weapon's, as its fields already have it (mangos's
        // `Pet::GetConditionalTotalPhysicalDamageModifier` over the
        // `TOTAL_VALUE` the flat auras hold).
        let mood = if hand == Hand::Main {
            crate::game::map::pets::happiness_damage(map, attacker)
        } else {
            1.0
        };
        let mut flat = a
            .effects()
            .filter(|(h, x)| {
                x.kind == AURA_MOD_DAMAGE_DONE
                    && (x.misc as u32) & school != 0
                    && !(in_fields && x.misc & 1 != 0)
                    && (!is_player || crate::game::map::derive::fits(h, weapon.as_ref()))
            })
            .map(|(h, x)| (x.amount * h.stacks.max(1) as i32) as f32)
            .sum::<f32>()
            * mood;
        let kind = creature_type_mask(map, victim);
        if kind != 0 {
            flat += a.sum_mask(AURA_MOD_DAMAGE_DONE_CREATURE, kind) as f32;
            let ap = a.sum_mask(
                if hand == Hand::Ranged {
                    AURA_MOD_RANGED_ATTACK_POWER_VERSUS
                } else {
                    AURA_MOD_MELEE_ATTACK_POWER_VERSUS
                },
                kind,
            );
            flat += ap as f32 / 14.0 * speed;
        }
        // A player's flat bonuses take the hand's percentages, as its
        // weapon's damage in the fields has: damage done, and the off
        // hand's half and its bonus (mangos's `TOTAL_VALUE` under the
        // hand's `TOTAL_PCT`, and `MeleeDamageBonusDone`'s target bonuses).
        if is_player {
            let mut pct = crate::game::map::derive::physical_pct_for(a, weapon.as_ref());
            if hand == Hand::Off {
                pct *= 0.5 * (1.0 + a.sum(AURA_MOD_OFFHAND_DAMAGE_PCT) as f32 / 100.0);
            }
            flat *= pct;
        }
        damage += flat;
        if kind != 0 {
            damage *= a.product_mask(AURA_MOD_DAMAGE_DONE_VERSUS, kind);
        }
        damage *= school_pct;
        // An armed creature, disarmed, hits for half (mangos-classic's
        // `Creature::GetConditionalTotalPhysicalDamageModifier`: a weapon
        // in the hand it can't use, `0.5f`; Thistle Tea's is four tenths).
        if a.has(AURA_MOD_DISARM)
            && let Some(c) = map.creature(attacker)
            && c.fields.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY) != 0
            && hand == Hand::Main
        {
            damage *= 0.5;
        }
    }
    if let Some(v) = auras::auras(map, victim) {
        let (flat_kind, pct_kind) = if hand == Hand::Ranged {
            (
                AURA_MOD_RANGED_DAMAGE_TAKEN,
                AURA_MOD_RANGED_DAMAGE_TAKEN_PCT,
            )
        } else {
            (AURA_MOD_MELEE_DAMAGE_TAKEN, AURA_MOD_MELEE_DAMAGE_TAKEN_PCT)
        };
        // Attack power against a marked target (Hunter's Mark).
        let marked = v.sum(if hand == Hand::Ranged {
            AURA_RANGED_ATTACK_POWER_ATTACKER_BONUS
        } else {
            AURA_MELEE_ATTACK_POWER_ATTACKER_BONUS
        });
        damage += marked.max(0) as f32 / 14.0 * speed;
        // A magic school's flat cut (Dampen Magic) takes at most half of a
        // blow, as it does of a spell's (`outcome::taken_of`, Thistle Tea's
        // `AttackDamageTaken.swing_amount`).
        let mut school_flat = v.sum_mask(AURA_MOD_DAMAGE_TAKEN, school) as f32;
        if school & 1 == 0 {
            school_flat = school_flat.max(-damage.max(0.0) / 2.0);
        }
        damage += school_flat + v.sum(flat_kind) as f32;
        damage *= v.product_mask(AURA_MOD_DAMAGE_PERCENT_TAKEN, school) * v.product(pct_kind);
    }
    (damage.max(0.0) as u32, school)
}

/// What armor leaves of physical damage from `attacker` on `victim`.
pub fn armor_mitigate(map: &Map, attacker: Guid, victim: Guid, damage: u32) -> u32 {
    if damage == 0 {
        return 0;
    }
    let armor = map
        .unit_fields(victim)
        .map_or(0, |f| f.get_u32(UNIT_FIELD_RESISTANCES) as i32);
    let penetration =
        auras::auras(map, attacker).map_or(0, |a| a.sum_mask(AURA_MOD_TARGET_RESISTANCE, 1));
    let level = map.unit_view(attacker).map_or(1, |u| u.level);
    let cut = armor_reduction((armor + penetration).max(0) as u32, level);
    damage.saturating_sub((damage as f32 * cut) as u32).max(1)
}

/// A crit's multiplier: double, more against the creature types the
/// attacker's auras name.
pub fn crit_multiplier(map: &Map, attacker: Guid, victim: Guid) -> f32 {
    let kind = creature_type_mask(map, victim);
    auras::auras(map, attacker).map_or(2.0, |a| {
        2.0 + (a.sum_mask(AURA_MOD_CRIT_PERCENT_VERSUS, kind)
            + a.sum(AURA_MOD_CRIT_DAMAGE_BONUS_MELEE)) as f32
            / 100.0
    })
}

/// `CMSG_ATTACKSWING`: `attacker` starts on `victim`, with the first swing
/// at once if it can land.
pub fn start_attack(map: &mut Map, attacker: Guid, victim: Guid) {
    let now = map.now;
    let Some(va) = map.unit_view(attacker) else {
        return;
    };
    let Some(vb) = map.unit_view(victim) else {
        send_to(map, attacker, encode(&SMSG_ATTACKSWING_CANT_ATTACK {}));
        return;
    };
    if !vb.alive {
        send_to(map, attacker, encode(&SMSG_ATTACKSWING_DEADTARGET {}));
        return;
    }
    if !va.alive || !can_aim_at(map, attacker, victim) {
        send_to(map, attacker, encode(&SMSG_ATTACKSWING_CANT_ATTACK {}));
        return;
    }
    // A player can't fight mounted (mangos's `Unit::Attack`: "player
    // cannot attack in mount state").
    if map
        .players
        .get(&attacker)
        .is_some_and(|p| p.fields.get_u32(UNIT_FIELD_MOUNTDISPLAYID) != 0)
    {
        send_to(map, attacker, encode(&SMSG_ATTACKSWING_CANT_ATTACK {}));
        return;
    }
    // Another victim: the old one hears the attack stop, and a strike
    // queued for it goes (mangos's `Unit::Attack`, Thistle Tea's
    // `start_melee_attack`).
    if map
        .combat(attacker)
        .is_some_and(|c| c.attacking && c.victim != victim && !c.victim.is_zero())
    {
        stop_attack(map, attacker);
    }
    let Some(c) = map.combat_mut(attacker) else {
        return;
    };
    let same = c.attacking && c.victim == victim;
    c.victim = victim;
    c.attacking = true;
    if c.next_swing.is_none_or(|t| t < now) {
        c.next_swing = Some(now);
    }
    // A new attack's off hand waits half its period (mangos's
    // `Unit::Attack`), or longer if it was waiting longer: a new victim
    // doesn't hasten it.
    if !same {
        let period = map
            .unit_fields(attacker)
            .map_or(2000, |f| f.get_u32(UNIT_FIELD_BASEATTACKTIME + 1))
            .max(400);
        let half = now + Duration::from_millis(u64::from(period / 2));
        if let Some(c) = map.combat_mut(attacker) {
            c.next_offhand = Some(c.next_offhand.map_or(half, |t| t.max(half)));
        }
    }
    if let Some(f) = map.unit_fields_mut(attacker) {
        f.set_guid(UNIT_FIELD_TARGET, victim);
    }
    if !same && let Some(frame) = encode(&SMSG_ATTACKSTART { attacker, victim }) {
        map.send_to_observers(attacker, &frame, true);
    }
    try_swing(map, attacker);
}

/// `CMSG_ATTACKSTOP`, or a fight that ends. A strike queued for the next
/// swing is interrupted, and its caster told (mangos's `MeleeAttackStop`
/// interrupting the melee spell, Thistle Tea's `stop_melee_attack`).
pub fn stop_attack(map: &mut Map, attacker: Guid) {
    let Some(c) = map.combat_mut(attacker) else {
        return;
    };
    if !c.attacking {
        return;
    }
    c.attacking = false;
    // Attacking again is told its errors afresh (Thistle Tea's
    // `clear_auto_attack`).
    c.last_swing_error = None;
    let queued = c.queued.take();
    let victim = c.victim;
    if let Some(spell) = queued {
        crate::game::map::spells::send_cast_result(
            map,
            attacker,
            spell,
            Some(crate::game::map::spells::checks::fail::INTERRUPTED),
        );
    }
    if let Some(frame) = encode(&SMSG_ATTACKSTOP {
        player: attacker,
        enemy: victim,
        unknown1: 0,
    }) {
        map.send_to_observers(attacker, &frame, true);
    }
}

/// Swing if a hand's timer is due and the swing can land; an error, now
/// and then, if it can't. Extra attacks owed go first.
pub fn try_swing(map: &mut Map, attacker: Guid) {
    let now = map.now;
    let Some(c) = map.combat(attacker) else {
        return;
    };
    if !c.attacking {
        return;
    }
    // A creature barred from melee (its template, a script) swings at no
    // one, however its fight began (a pet sent in, a guardian).
    if guid::is_unit(attacker) && !crate::game::map::event_ai::melee(map, attacker) {
        return;
    }
    let victim = c.victim;
    let dual = dual_wields(map, attacker);
    let main_due = c.next_swing.is_none_or(|t| t <= now);
    let off_due = dual && c.next_offhand.is_none_or(|t| t <= now);
    let extra = c.extra_attacks > 0;
    if !main_due && !off_due && !extra {
        return;
    }
    let (Some(va), Some(vb)) = (map.unit_view(attacker), map.unit_view(victim)) else {
        stop_attack(map, attacker);
        return;
    };
    if !vb.alive || !va.alive {
        stop_attack(map, attacker);
        return;
    }
    // A pet or a creature lets go of a victim on its way home (mangos's
    // `PetAI`: "do not continue attacking if victim is moving home"); a
    // player swings on, and the blows evade (in `swing`).
    if !va.is_player && map.creature(victim).is_some_and(|c| c.evading) {
        stop_attack(map, attacker);
        return;
    }
    // A player's victim gone from their sight (into stealth, invisible)
    // isn't swung at: their client has forgotten it, and so stops (Thistle
    // Tea's `target_valid_same_map?`). A creature's AI lets go of one
    // itself.
    if va.is_player && !crate::game::map::spells::stealth::sees(map, attacker, victim) {
        stop_attack(map, attacker);
        return;
    }
    // A stun holds the swing; a pacified unit doesn't swing at all. A
    // spell being cast holds the regular swings, but not the extra attacks
    // a proc owes: they go, and the cast goes on (vmangos's
    // `AttackerStateUpdate` skips the cast for an extra swing, and Thistle
    // Tea's `extra_attacks_step` comes before its casting).
    if auras::held(map, attacker, false)
        || auras::auras(map, attacker).is_some_and(|a| {
            a.has(AURA_MOD_PACIFY) || a.has(AURA_MOD_PACIFY_SILENCE) || a.has(AURA_FEIGN_DEATH)
        })
    {
        return;
    }
    let casting = map.caster(attacker).is_some_and(|c| c.casting.is_some());
    if casting && !extra {
        return;
    }
    // The extra attacks owed go in reach, a player's facing their victim
    // or so close that facing doesn't count (Thistle Tea's overlap, 1.4
    // yards; mangos's `DoExtraAttacks` asks reach alone).
    let close = (va.position.x - vb.position.x).powi(2) + (va.position.y - vb.position.y).powi(2)
        <= OVERLAP_YARDS * OVERLAP_YARDS;
    let extra_done = extra
        && va.standing
        && in_melee_range(&va, &vb)
        && (!va.is_player || close || facing(&va, &vb));
    if extra_done {
        let owed = map.combat(attacker).map_or(0, |c| c.extra_attacks);
        if let Some(c) = map.combat_mut(attacker) {
            c.extra_attacks = 0;
            c.in_extra_attack = true;
        }
        // Put back however the swings end, a panic's unwinding too: left
        // set, it would refuse every extra attack after (Windfury, Sword
        // Specialization, Hand of Justice) until a new login.
        let swings = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for _ in 0..owed {
                if !still_swinging(map, attacker, victim) {
                    break;
                }
                #[cfg(test)]
                map.faults.check(&format!("extra swing of {attacker}"));
                swing(map, attacker, victim, Hand::Main, true);
            }
        }));
        if let Some(c) = map.combat_mut(attacker) {
            c.in_extra_attack = false;
        }
        if let Err(panic) = swings {
            std::panic::resume_unwind(panic);
        }
    }
    // Casting, the regular swings wait without a word.
    if casting || (extra_done && !main_due && !off_due) {
        return;
    }
    let error = if !va.standing {
        Some(SWING_ERROR_NOT_STANDING)
    } else if !in_melee_range(&va, &vb) {
        Some(SWING_ERROR_NOT_IN_RANGE)
    } else if va.is_player && !facing(&va, &vb) {
        Some(SWING_ERROR_BAD_FACING)
    } else {
        None
    };
    if let Some(error) = error {
        let Some(c) = map.combat_mut(attacker) else {
            return;
        };
        // Told once while it lasts, again when it changes (mangos's
        // `UpdateMeleeAttackingState`); the hands stay due, and try again
        // at the next update.
        if c.last_swing_error != Some(error) {
            c.last_swing_error = Some(error);
            let frame = match error {
                SWING_ERROR_NOT_STANDING => encode(&SMSG_ATTACKSWING_NOTSTANDING {}),
                SWING_ERROR_NOT_IN_RANGE => encode(&SMSG_ATTACKSWING_NOTINRANGE {}),
                _ => encode(&SMSG_ATTACKSWING_BADFACING {}),
            };
            send_to(map, attacker, frame);
        }
        return;
    }
    if let Some(c) = map.combat_mut(attacker) {
        c.last_swing_error = None;
    }
    // A script's melee spell stands in for the blow; not for one under
    // its master's control, which swings as a pet does (Thistle Tea's
    // "controlled creatures use ordinary swings": its own AI is set
    // aside, as mangos's `PetAI` takes a charmed creature's).
    if main_due
        && map
            .creature(attacker)
            .is_some_and(crate::game::map::event_ai::runs_own)
        && let Some(spell) = crate::game::map::event_ai::melee_spell(map, attacker)
    {
        let period = map
            .unit_fields(attacker)
            .map_or(2000, |f| f.get_u32(UNIT_FIELD_BASEATTACKTIME))
            .max(100);
        if let Some(c) = map.combat_mut(attacker) {
            c.next_swing = Some(now + Duration::from_millis(u64::from(period)));
        }
        crate::game::map::spells::cast_triggered(map, attacker, spell, Some(victim));
        return;
    }
    if main_due && still_swinging(map, attacker, victim) {
        swing(map, attacker, victim, Hand::Main, false);
    }
    // The main hand's swing puts the off-hand's a moment later, which it
    // waits for (mangos asks `isAttackReady(OFF_ATTACK)` after it).
    let off_due = off_due
        && map
            .combat(attacker)
            .is_some_and(|c| c.next_offhand.is_none_or(|t| t <= now));
    if off_due && still_swinging(map, attacker, victim) {
        swing(map, attacker, victim, Hand::Off, false);
    }
}

/// Whether `attacker` goes on swinging at `victim` after a blow: both
/// alive (a damage shield's answer may have killed the striker), and the
/// attack not stopped.
fn still_swinging(map: &Map, attacker: Guid, victim: Guid) -> bool {
    map.unit_view(victim).is_some_and(|u| u.alive)
        && map.unit_view(attacker).is_some_and(|u| u.alive)
        && map
            .combat(attacker)
            .is_some_and(|c| c.attacking && c.victim == victim)
}

/// One swing of `hand`, landed or not. An extra attack leaves the timer
/// alone.
fn swing(map: &mut Map, attacker: Guid, victim: Guid, hand: Hand, extra: bool) {
    let now = map.now;
    if !extra {
        let field = if hand == Hand::Off {
            UNIT_FIELD_BASEATTACKTIME + 1
        } else {
            UNIT_FIELD_BASEATTACKTIME
        };
        let period = map
            .unit_fields(attacker)
            .map(|f| f.get_u32(field))
            .unwrap_or(2000)
            .max(100);
        if let Some(c) = map.combat_mut(attacker) {
            let next = Some(now + Duration::from_millis(u64::from(period)));
            match hand {
                // The hands keep a moment apart, the main one waiting too
                // (mangos's `ATTACK_DISPLAY_DELAY`).
                Hand::Off => {
                    c.next_offhand = next;
                    if c.next_swing
                        .is_some_and(|t| t < now + Duration::from_millis(200))
                    {
                        c.next_swing = Some(now + Duration::from_millis(200));
                    }
                }
                _ => {
                    c.next_swing = next;
                    // Reckoning's banked attacks arm on a regular swing.
                    c.extra_attacks += std::mem::take(&mut c.held_extra_attacks);
                    // The hands keep a moment apart.
                    if c.next_offhand
                        .is_some_and(|t| t < now + Duration::from_millis(200))
                    {
                        c.next_offhand = Some(now + Duration::from_millis(200));
                    }
                }
            }
        }
    }
    // A swing ends what attacking does (Honorless Target; mangos's
    // `AttackerStateUpdate`, `AURA_INTERRUPT_FLAG_ATTACKING`).
    auras::interrupt(map, attacker, auras::INTERRUPT_ATTACK);
    // A next-swing spell takes the main hand's swing.
    if hand == Hand::Main
        && !extra
        && let Some(spell) = map.combat_mut(attacker).and_then(|c| c.queued.take())
    {
        let targets = crate::game::map::spells::Targets {
            unit: Some(victim),
            ..Default::default()
        };
        if crate::game::map::spells::cast_now(map, attacker, spell, targets) {
            if !map.creature(victim).is_some_and(|c| c.evading) {
                engage(map, attacker, victim);
            }
            return;
        }
    }
    // A creature on its way home evades the blow: a miss of no sound, and
    // no fight from it, no damage, no proc (mangos-classic's
    // `MELEE_HIT_EVADE`, `VICTIMSTATE_EVADES`, and `DealMeleeDamage`
    // skipping a unit in evade).
    if map.creature(victim).is_some_and(|c| c.evading) {
        let hand_info = if hand == Hand::Off {
            HITINFO_NORMALSWING2 | HITINFO_LEFTSWING
        } else {
            HITINFO_NORMALSWING2
        };
        let frame = attacker_state(
            attacker,
            victim,
            hand_info | HITINFO_MISS | HITINFO_SWINGNOHITSOUND,
            0,
            VICTIMSTATE_EVADES,
            0,
        );
        map.send_to_observers(attacker, &frame, true);
        return;
    }
    let Some(roll) = roll_for(map, attacker, victim, hand) else {
        return;
    };
    let mut rng = rand::rng();
    let outcome = rolled_outcome(&roll, &mut rng);
    let (raw, school) = weapon_damage(map, attacker, victim, hand, &mut rng);
    let mitigated = mitigate(map, attacker, victim, raw, school);
    let kept = if outcome == Outcome::Glancing {
        let caster_class = map
            .players
            .get(&attacker)
            .is_some_and(|p| matches!(p.row.class, 5 | 8 | 9));
        let gap = roll.victim_defense - roll.attacker_skill;
        glancing_factor(gap, caster_class, rng.random_range(0.0f32..1.0))
    } else {
        1.0
    };
    let crit = if outcome == Outcome::Crit {
        crit_multiplier(map, attacker, victim)
    } else {
        2.0
    };
    let (mut hit_info, state, damage, blocked) = blow(
        outcome,
        raw,
        mitigated,
        block_value(map, victim),
        crit,
        kept,
    );
    // A blow on a unit its school can't touch does nothing.
    let immune = !matches!(outcome, Outcome::Miss | Outcome::Dodge | Outcome::Parry)
        && school_immune(map, victim, school);
    let (state, mut damage, blocked) = if immune {
        (VICTIMSTATE_IS_IMMUNE, 0, 0)
    } else {
        (state, damage, blocked)
    };
    if hand == Hand::Off {
        hit_info |= HITINFO_LEFTSWING;
    }
    // A blow of a magic school (an elemental's) is resisted in part, as a
    // spell of it is, before a ward of its school soaks the rest (mangos's
    // `CalculateDamageAbsorbAndResist` for each of a blow's schools).
    let resisted = if school & 1 == 0 {
        crate::game::map::spells::outcome::resisted_of_school(
            map,
            attacker,
            victim,
            school.trailing_zeros() as usize,
            damage,
            false,
        )
    } else {
        0
    };
    if resisted > 0 {
        damage -= resisted;
        hit_info |= HITINFO_RESIST;
    }
    let (left, absorbed) = if damage > 0 {
        auras::absorb_and_share(map, attacker, victim, damage, school)
    } else {
        (0, 0)
    };
    if absorbed > 0 {
        hit_info |= HITINFO_ABSORB;
    }
    let frame = attacker_state_full(
        attacker,
        victim,
        hit_info,
        left,
        state,
        blocked,
        absorbed,
        resisted,
        school.trailing_zeros(),
    );
    map.send_to_observers(attacker, &frame, true);
    // A creature's victim hears of the blow even if it doesn't see it yet.
    if !guid::is_player(attacker)
        && let Some(p) = map.players.get(&victim)
        && !p.known.contains(&attacker)
    {
        p.outbound.send_frame(frame);
    }
    engage(map, attacker, victim);
    after_outcome(map, attacker, victim, outcome);
    let landed = !immune
        && matches!(
            outcome,
            Outcome::Hit | Outcome::Crit | Outcome::Glancing | Outcome::Crushing | Outcome::Block
        );
    if landed && damage > 0 {
        gain_rage(map, attacker, damage, false);
    }
    // A dodged or parried blow still gives three quarters of its rage.
    if matches!(outcome, Outcome::Dodge | Outcome::Parry) {
        gain_rage(map, attacker, mitigated * 3 / 4, false);
    }
    // A blow on the immune trains nothing (Thistle Tea's `CombatSkills`).
    if !immune {
        skill_gains(map, attacker, victim, hand);
        stand_struck(map, victim);
    }
    if left > 0 {
        // A player's blow that hurts renews their judgements on the victim.
        refresh_judgements(map, attacker, victim);
        // A blow on a caster pushes its cast back.
        crate::game::map::spells::pushback_by(map, victim, attacker);
        deal_damage_school(map, attacker, victim, left, school);
    }
    let mut ex = if immune {
        procs::EX_IMMUNE
    } else {
        outcome_ex(outcome)
    };
    if outcome == Outcome::Block && damage > 0 {
        ex |= procs::EX_NORMAL_HIT;
    }
    if absorbed > 0 {
        ex |= procs::EX_ABSORB;
    }
    procs::trigger(
        map,
        procs::Event {
            actor: attacker,
            target: victim,
            actor_flags: procs::PROC_DONE_MELEE,
            target_flags: procs::PROC_TAKEN_MELEE,
            ex,
            spell: None,
            damage: left,
            school,
            offhand: hand == Hand::Off,
            extra_attack: extra,
            ..Default::default()
        },
    );
    if landed && (left > 0 || outcome == Outcome::Block) {
        procs::damage_shields(map, attacker, victim);
    }
    if left > 0 {
        daze(map, attacker, victim, &roll);
    }
}

/// A player's white blow that did damage: every aura on `victim` of their
/// paladin spells that always hit (the judgements) starts its full duration
/// again (mangos's `DealMeleeDamage`, `RefreshHolder`).
pub fn refresh_judgements(map: &mut Map, attacker: Guid, victim: Guid) {
    if !guid::is_player(attacker) {
        return;
    }
    let data = map.realm.data.clone();
    let now = map.now;
    let Some(a) = auras::auras_mut(map, victim) else {
        return;
    };
    let mut shown = Vec::new();
    for h in a.holders.iter_mut().filter(|h| h.caster == attacker) {
        let judgement = data.spells.get(h.spell).is_some_and(|s| {
            s.family_name == 10 && s.attr(3, crate::game::data::spells::ATTR_EX3_ALWAYS_HIT)
        });
        let Some(ms) = h.duration.filter(|_| judgement && h.expires.is_some()) else {
            continue;
        };
        h.expires = Some(now + Duration::from_millis(u64::from(ms)));
        if let Some(slot) = h.slot {
            shown.push((slot, ms));
        }
    }
    if let Some(p) = map.players.get(&victim) {
        for (slot, ms) in shown {
            p.outbound
                .send(&wow_world_messages::vanilla::SMSG_UPDATE_AURA_DURATION {
                    aura_slot: slot as u8,
                    aura_duration: ms,
                });
        }
    }
}

/// What a white blow's `outcome` makes of its damage, `raw` before armor
/// and `mitigated` after: its hit info, the victim's state, the damage and
/// what a block stopped. A block stops up to `block_value` of the damage
/// after armor; a crit multiplies it by `crit`; a glancing blow keeps
/// `kept` of it, a crushing one half again.
pub fn blow(
    outcome: Outcome,
    raw: u32,
    mitigated: u32,
    block_value: u32,
    crit: f32,
    kept: f32,
) -> (u32, u32, u32, u32) {
    match outcome {
        // A swing's own flag with the miss, as mangos's
        // `CalculateMeleeDamage` starts every swing (Thistle Tea's 0x12).
        Outcome::Miss => (
            HITINFO_NORMALSWING2 | HITINFO_MISS,
            VICTIMSTATE_UNAFFECTED,
            0,
            0,
        ),
        Outcome::Dodge => (HITINFO_NORMALSWING2, VICTIMSTATE_DODGE, 0, 0),
        Outcome::Parry => (HITINFO_NORMALSWING2, VICTIMSTATE_PARRY, 0, 0),
        Outcome::Block => {
            let blocked = block_value.min(mitigated);
            let state = if blocked >= mitigated {
                VICTIMSTATE_BLOCKS
            } else {
                VICTIMSTATE_NORMAL
            };
            (HITINFO_NORMALSWING2, state, mitigated - blocked, blocked)
        }
        Outcome::Glancing => (
            HITINFO_NORMALSWING2 | HITINFO_GLANCING,
            VICTIMSTATE_NORMAL,
            ((mitigated as f32 * kept) as u32).max(raw.min(1)),
            0,
        ),
        Outcome::Crit => (
            HITINFO_NORMALSWING2 | HITINFO_CRITICALHIT,
            VICTIMSTATE_NORMAL,
            (mitigated as f32 * crit) as u32,
            0,
        ),
        Outcome::Crushing => (
            HITINFO_NORMALSWING2 | HITINFO_CRUSHING,
            VICTIMSTATE_NORMAL,
            mitigated * 3 / 2,
            0,
        ),
        Outcome::Hit => (HITINFO_NORMALSWING2, VICTIMSTATE_NORMAL, mitigated, 0),
    }
}

#[cfg(test)]
thread_local! {
    /// A test's outcome for the white swings it makes, in place of the
    /// roll.
    pub static FORCED_OUTCOME: std::cell::Cell<Option<Outcome>> =
        const { std::cell::Cell::new(None) };
}

/// A white swing's roll on the table (or a test's outcome for it).
fn rolled_outcome(r: &Roll, rng: &mut impl rand::Rng) -> Outcome {
    #[cfg(test)]
    if let Some(o) = FORCED_OUTCOME.with(|f| f.get()) {
        return o;
    }
    roll_outcome(r, rng.random_range(0..10_000))
}

/// What armor leaves of a blow of `school` (a mask): only a physical one
/// is mitigated.
pub fn mitigate(map: &Map, attacker: Guid, victim: Guid, raw: u32, school: u32) -> u32 {
    if school & 1 != 0 {
        armor_mitigate(map, attacker, victim, raw)
    } else {
        raw
    }
}

/// What a unit's block stops: a player's shield's value, or a creature's
/// half its level and a twentieth of its class's strength at that level
/// (mangos's `Creature::GetShieldBlockValue`, Thistle Tea's
/// `CombatRatings.block_value`).
pub fn block_value(map: &Map, g: Guid) -> u32 {
    let strength = map
        .creature(g)
        .and_then(|c| {
            let t = map.realm.data.creature_template(c.entry)?;
            let level = c.fields.get_u32(UNIT_FIELD_LEVEL).min(255) as u8;
            map.realm.data.class_level_stats.get(&(t.unit_class, level))
        })
        .map_or(0, |s| s.strength);
    derived(map, g).block_value + strength / 20
}

/// The `procEx` an outcome is.
pub fn outcome_ex(outcome: Outcome) -> u32 {
    match outcome {
        Outcome::Miss => procs::EX_MISS,
        Outcome::Dodge => procs::EX_DODGE,
        Outcome::Parry => procs::EX_PARRY,
        Outcome::Block => procs::EX_BLOCK,
        Outcome::Crit => procs::EX_CRITICAL_HIT,
        Outcome::Hit | Outcome::Glancing | Outcome::Crushing => procs::EX_NORMAL_HIT,
    }
}

/// What a dodge, parry or block opens: the defender's reactive window
/// (Revenge, Riposte; a hunter's Counterattack), a warrior attacker's
/// combo point for Overpower, and a parry's haste.
pub fn after_outcome(map: &mut Map, attacker: Guid, victim: Guid, outcome: Outcome) {
    let now = map.now;
    if !matches!(outcome, Outcome::Dodge | Outcome::Parry | Outcome::Block) {
        return;
    }
    // The dead open nothing (mangos's `ProcSkillsAndReactives` wants a
    // living victim).
    if !map.unit_view(victim).is_some_and(|u| u.alive) {
        return;
    }
    let class = map.players.get(&victim).map(|p| p.row.class);
    if let Some(class) = class
        && let Some(c) = map.combat_mut(victim)
    {
        let window = Some((attacker, now + REACTIVE_WINDOW));
        match (class, outcome) {
            // A rogue's dodge opens nothing.
            (4, Outcome::Dodge) => {}
            (3, Outcome::Parry) => c.hunter_parry_window = window,
            _ => c.defense_window = window,
        }
    }
    if outcome == Outcome::Dodge
        && let Some(p) = map.players.get_mut(&attacker)
        && p.row.class == 1
    {
        p.caster.combo_target = victim;
        p.caster.combo_points = 1;
        p.combat.dodge_combo_until = Some(now + REACTIVE_WINDOW);
        p.fields.set_guid(PLAYER_FIELD_COMBO_TARGET, victim);
        p.fields.set_byte(PLAYER_FIELD_BYTES, 1, 1);
    }
    if outcome == Outcome::Parry {
        parry_haste(map, victim);
    }
    sync_aura_state(map, victim);
}

/// A parry hastens the parrier's next swing: the earlier hand loses up to
/// two fifths of its period, to no less than a fifth. The period is the
/// weapon's own, before hastes (mangos's `GetAttackTime`, Thistle Tea's
/// base attack time).
pub fn parry_haste(map: &mut Map, g: Guid) {
    let now = map.now;
    let dual = dual_wields(map, g);
    let haste = auras::auras(map, g)
        .map_or(1.0, crate::game::map::derive::melee_haste)
        .max(0.01);
    let unhasted = |p: u32| (p as f32 / haste).round() as u32;
    let periods = map
        .unit_fields(g)
        .map(|f| {
            (
                unhasted(f.get_u32(UNIT_FIELD_BASEATTACKTIME)),
                unhasted(f.get_u32(UNIT_FIELD_BASEATTACKTIME + 1)),
            )
        })
        .unwrap_or((2000, 2000));
    let Some(c) = map.combat_mut(g) else {
        return;
    };
    if !c.attacking {
        return;
    }
    let main = c.next_swing.filter(|t| *t > now);
    let off = c.next_offhand.filter(|t| *t > now && dual);
    let (slot, deadline, period) = match (main, off) {
        (Some(m), Some(o)) if o < m => (&mut c.next_offhand, o, periods.1),
        (Some(m), _) => (&mut c.next_swing, m, periods.0),
        (None, Some(o)) => (&mut c.next_offhand, o, periods.1),
        (None, None) => return,
    };
    let remaining = deadline.duration_since(now).as_millis() as f32;
    let period = period.max(1) as f32;
    let shortened = remaining.min((period * 0.2).max(remaining - period * 0.4));
    *slot = Some(now + Duration::from_millis(shortened as u64));
}

/// A weapon put in the main or off hand in a fight starts that hand's
/// swing over, a whole period of the new weapon away, hasted as the hand
/// is (mangos's `Player::EquipItem` and `resetAttackTimer`, Thistle Tea's
/// `AttackTimers.equipment_changed`). The other hand, the ranged slot and a
/// weapon taken off leave the timers as they are, as mangos has them.
pub fn weapon_put_in_hand(map: &mut Map, g: Guid, slot: u8) {
    use crate::game::map::inventory::{SLOT_MAIN_HAND, SLOT_OFF_HAND};
    if slot != SLOT_MAIN_HAND && slot != SLOT_OFF_HAND {
        return;
    }
    let now = map.now;
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let fighting = p.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT);
    let weapon = p
        .item_at(crate::game::store::BAG_PLAYER, slot)
        .and_then(|i| data.item(p.items[i].entry))
        .is_some_and(|t| t.class == 2);
    if !fighting || !weapon || p.death != Death::Alive {
        return;
    }
    let field = if slot == SLOT_OFF_HAND {
        UNIT_FIELD_BASEATTACKTIME + 1
    } else {
        UNIT_FIELD_BASEATTACKTIME
    };
    let period = p.fields.get_u32(field).max(100);
    let at = Some(now + Duration::from_millis(u64::from(period)));
    if let Some(c) = map.combat_mut(g) {
        if slot == SLOT_OFF_HAND {
            c.next_offhand = at;
        } else {
            c.next_swing = at;
        }
    }
}

/// `CMSG_SETSHEATHED` (Thistle Tea's `Sheath.request`): weapons out or
/// away; a channel and the auras that sheathing breaks end first.
pub fn sheathe(map: &mut Map, g: Guid, state: u8) {
    if state > 2 {
        return;
    }
    let channel = map
        .caster(g)
        .and_then(|c| c.casting.as_ref())
        .filter(|c| c.channel_until.is_some())
        .map(|c| c.spell);
    if channel
        .and_then(|s| map.realm.data.spells.get(s))
        .is_some_and(|s| s.channel_interrupt_flags & INTERRUPT_SHEATHING != 0)
    {
        crate::game::map::spells::interrupt(map, g);
    }
    auras::interrupt(map, g, INTERRUPT_SHEATHING);
    if let Some(p) = map.players.get_mut(&g) {
        p.fields.set_byte(UNIT_FIELD_BYTES_2, 0, state);
    }
}

/// A cast of `s` went off (Thistle Tea's `CastingCombat`, mangos's
/// `Spell::cast`): one cast, not triggered, that interrupts the swing
/// (interrupt flag 0x08) starts its caster's swings over, unless it says
/// not to; one that cancels auto attack ends it, triggered or not, and a
/// ranged one of those ends Auto Shot too (mangos's "blizzlike order").
pub fn spell_launched(
    map: &mut Map,
    caster: Guid,
    s: &crate::game::data::spells::SpellTemplate,
    triggered: bool,
) {
    let now = map.now;
    if !triggered && s.interrupt_flags & 0x08 != 0 && !s.attr(2, 0x2_0000) {
        let dual = dual_wields(map, caster);
        let periods = map.unit_fields(caster).map_or((2000, 2000), |f| {
            (
                f.get_u32(UNIT_FIELD_BASEATTACKTIME),
                f.get_u32(UNIT_FIELD_BASEATTACKTIME + 1),
            )
        });
        if let Some(c) = map.combat_mut(caster)
            && c.attacking
        {
            c.next_swing = Some(now + Duration::from_millis(u64::from(periods.0.max(1))));
            if dual {
                c.next_offhand = Some(now + Duration::from_millis(u64::from(periods.1.max(1))));
            }
        }
    }
    if s.attr(0, crate::game::data::spells::ATTR_CANCELS_AUTO_ATTACK) {
        if s.attr(0, crate::game::data::spells::ATTR_RANGED) {
            crate::game::map::ranged::cancel(map, caster, true);
        }
        if map.combat(caster).is_some_and(|c| c.attacking) {
            stop_attack(map, caster);
        }
    }
}

/// `UNIT_FIELD_AURASTATE` brought in line: the reactive windows still open,
/// and health below a fifth.
/// Alive under a fifth of its health: Execute's and Hammer of Wrath's
/// state (mangos's `AURA_STATE_HEALTHLESS_20_PERCENT`).
pub fn healthless(health: u32, max: u32) -> bool {
    health > 0 && (health as u64) * 5 < max as u64
}

pub fn sync_aura_state(map: &mut Map, g: Guid) {
    let now = map.now;
    let (defense, parry) = map.combat(g).map_or((false, false), |c| {
        (
            c.defense_window.is_some_and(|(_, t)| t > now),
            c.hunter_parry_window.is_some_and(|(_, t)| t > now),
        )
    });
    let Some(f) = map.unit_fields_mut(g) else {
        return;
    };
    let health = f.get_u32(UNIT_FIELD_HEALTH);
    let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
    let old = f.get_u32(UNIT_FIELD_AURASTATE);
    let mut bits = old & !(AURA_STATE_DEFENSE | AURA_STATE_HUNTER_PARRY | AURA_STATE_HEALTHLESS_20);
    if defense {
        bits |= AURA_STATE_DEFENSE;
    }
    if parry {
        bits |= AURA_STATE_HUNTER_PARRY;
    }
    if healthless(health, max) {
        bits |= AURA_STATE_HEALTHLESS_20;
    }
    // A creature's health bands under 15, 10 and 5 percent.
    if guid::is_unit(g) {
        bits = (bits & !0x700) | crate::game::map::creature_motion::band(health, max).1;
    }
    if bits != old {
        f.set_u32(UNIT_FIELD_AURASTATE, bits);
    }
    crate::game::map::creature_motion::health_changed(map, g);
}

/// Windows that closed, and a warrior's Overpower point that ran out.
fn expire_windows(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(c) = map.combat_mut(g) else {
        return;
    };
    let mut changed = false;
    if c.defense_window.is_some_and(|(_, t)| t <= now) {
        c.defense_window = None;
        changed = true;
    }
    if c.hunter_parry_window.is_some_and(|(_, t)| t <= now) {
        c.hunter_parry_window = None;
        changed = true;
    }
    let combo_gone = c.dodge_combo_until.is_some_and(|t| t <= now);
    if combo_gone {
        c.dodge_combo_until = None;
    }
    if combo_gone && let Some(p) = map.players.get_mut(&g) {
        p.caster.combo_points = 0;
        p.caster.combo_target = Guid::zero();
        p.fields.set_byte(PLAYER_FIELD_BYTES, 1, 0);
    }
    if changed {
        sync_aura_state(map, g);
    }
}

/// Rage for a blow dealt or taken, for a unit whose power is rage.
pub fn gain_rage(map: &mut Map, g: Guid, damage: u32, taken: bool) {
    // `[rates] rage_income`, mangos's `Rate.Rage.Income`.
    let rate = map.cfg.rates.rage_income;
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if p.fields.get_byte(UNIT_FIELD_BYTES_0, 3) != 1 {
        return;
    }
    let by = if taken && p.auras.has_spell(SPELL_BERSERKER_RAGE) {
        1.3
    } else {
        1.0
    };
    let gain = crate::config::scaled(rage_scaled(damage, u32::from(p.level()), taken, by), rate);
    let max = p.fields.get_u32(UNIT_FIELD_MAXPOWER1 + 1).max(1000);
    let rage = p.fields.get_u32(UNIT_FIELD_POWER1 + 1);
    p.fields
        .set_u32(UNIT_FIELD_POWER1 + 1, (rage + gain).min(max));
}

/// Raise a player's skill by a point, if below its cap.
fn raise_skill(p: &mut crate::game::map::player::Player, skill: u32, by: u32) -> bool {
    for i in 0..128u16 {
        let base = PLAYER_SKILL_INFO_1_1 + i * 3;
        if p.fields.get_u32(base) & 0xFFFF != skill {
            continue;
        }
        let v = p.fields.get_u32(base + 1);
        let (value, max) = (v & 0xFFFF, v >> 16);
        if value >= max {
            return false;
        }
        p.fields
            .set_u32(base + 1, (value + by.max(1)).min(max) | (max << 16));
        return true;
    }
    false
}

/// The chance, in percent, that a blow trains a weapon skill (with the
/// player's `intellect`) or defense (`None`) at `value` of `max` at `level`
/// (mangos-classic's `UpdateCombatSkills`): a logarithmic catch-up seven or
/// more points short of the cap, then a quadratic fall to it; a weapon's
/// up to a tenth more for intellect, the most at 750 by level 60.
pub fn combat_skill_chance(value: u32, max: u32, level: u32, intellect: Option<u32>) -> f32 {
    if max == 0 || level == 0 || value >= max {
        return 0.0;
    }
    let (room, max_f, level_f) = (f64::from(max - value), f64::from(max), f64::from(level));
    let growth = 0.22 * (1.0 + 12.0 / level_f);
    let base = if room >= 7.0 {
        7.0 / max_f + growth * ((room + 5.0) / 12.0).ln()
    } else {
        room * room / (7.0 * max_f)
    }
    .clamp(0.0, 1.0);
    let bonus = intellect.map_or(0.0, |int| {
        let (bracket, most) = match level {
            ..=60 => (60.0, 750.0),
            61..=70 => (70.0, 1500.0),
            71..=80 => (80.0, 3000.0),
            _ => (255.0, 6000.0),
        };
        (0.10 * (f64::from(int) / most) * (bracket / level_f)).clamp(0.0, 0.10)
    });
    ((base + bonus).clamp(0.0, 1.0) * 100.0) as f32
}

/// A critter's creature type.
pub const CREATURE_TYPE_CRITTER: u32 = 8;

/// A fishing pole's weapon subclass.
pub const SUBCLASS_FISHING_POLE: u32 = 20;

/// A melee or ranged ability that hit, or was missed, dodged, parried or
/// blocked, trains the skill of the weapon it was made with and the
/// target's defense, once, as a swing does (mangos's
/// `ProcSkillsAndReactives` on a melee-based proc; Thistle Tea's
/// `CombatSkills.resolve` for abilities).
pub fn ability_skill_gains(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    s: &crate::game::data::spells::SpellTemplate,
) {
    let hand = if s.dmg_class == 3 {
        Hand::Ranged
    } else if s.attr(3, crate::game::data::spells::ATTR_EX3_REQ_OFFHAND) {
        Hand::Off
    } else {
        Hand::Main
    };
    skill_gains(map, caster, target, hand);
}

/// Weapon and defense skills that a blow between a player and a creature
/// trains, whatever the creature's level; a critter trains nothing, a
/// player's pet or charm no weapon skill, nor a creature of
/// `NO_SKILL_GAINS` (mangos's `ProcSkillsAndReactives`,
/// `UpdateCombatSkills` and `UpdateWeaponSkill`).
fn skill_gains(map: &mut Map, attacker: Guid, victim: Guid, hand: Hand) {
    let mut rng = rand::rng();
    // With outcomes fixed, not rolled (the tests), a blow trains whenever
    // it may.
    let sure = !crate::game::map::spells::outcome::rolling(map);
    let mut skill_roll = move || {
        if sure {
            0.0
        } else {
            rng.random_range(0.0f32..100.0)
        }
    };
    let (weapon_by, defense_by) = (
        map.cfg.rates.skill_gain_weapon,
        map.cfg.rates.skill_gain_defense,
    );
    let trains = |map: &Map, g: Guid| {
        map.creature(g).is_some_and(|c| {
            map.realm
                .data
                .creature_template(c.entry)
                .is_none_or(|t| t.creature_type != CREATURE_TYPE_CRITTER)
        })
    };
    let teaches_weapons = |map: &Map, g: Guid| {
        !player_controlled(map, g)
            && map
                .creature(g)
                .and_then(|c| map.realm.data.creature_template(c.entry))
                .is_none_or(|t| t.static_flags2 & STATIC2_NO_SKILL_GAINS == 0)
    };
    // The attacker's weapon.
    if map.players.contains_key(&attacker) && trains(map, victim) && teaches_weapons(map, victim) {
        // The weapon it can use: none in cat or bear form, nor in a
        // disarmed main hand, which trains unarmed; an empty off hand or
        // ranged slot trains nothing (mangos's `GetWeaponSkillIdForAttack`
        // and `CanUseEquippedWeapon`).
        let (feral, disarmed) = map.players.get(&attacker).map_or((false, false), |p| {
            (
                matches!(
                    p.fields.get_byte(UNIT_FIELD_BYTES_1, 2),
                    auras::FORM_CAT | auras::FORM_BEAR | auras::FORM_DIREBEAR
                ),
                p.auras.has(AURA_MOD_DISARM),
            )
        });
        let usable = !feral && !(hand == Hand::Main && disarmed);
        // A fishing pole trains nothing (mangos's `UpdateWeaponSkill`).
        let skill = match hand_weapon(map, attacker, hand).filter(|_| usable) {
            Some(w) if w.subclass == SUBCLASS_FISHING_POLE => None,
            Some(w) => Some(crate::game::map::derive::weapon_skill(w.subclass)),
            None if hand == Hand::Main => Some(162),
            None => None,
        };
        let gained = skill.is_some_and(|skill| {
            map.players.get_mut(&attacker).is_some_and(|p| {
                let level = u32::from(p.level());
                let Some(value) = crate::game::map::skills::value(p, skill) else {
                    return false;
                };
                let cap = level * 5;
                let intellect = p.fields.get_u32(UNIT_FIELD_STAT0 + 3);
                skill_roll() < combat_skill_chance(value, cap, level, Some(intellect))
                    && raise_skill(p, skill, weapon_by)
            })
        });
        if gained {
            auras::recalc(map, attacker);
        }
    }
    // The victim's defense.
    if map.players.contains_key(&victim) && trains(map, attacker) {
        let skill = crate::game::map::derive::SKILL_DEFENSE;
        let gained = map.players.get_mut(&victim).is_some_and(|p| {
            let level = u32::from(p.level());
            let Some(value) = crate::game::map::skills::value(p, skill) else {
                return false;
            };
            let cap = level * 5;
            skill_roll() < combat_skill_chance(value, cap, level, None)
                && raise_skill(p, skill, defense_by)
        });
        if gained {
            auras::recalc(map, victim);
        }
    }
}

/// A creature striking a player from behind may daze them.
fn daze(map: &mut Map, attacker: Guid, victim: Guid, roll: &Roll) {
    if roll.ranged || !roll.from_behind || !map.players.contains_key(&victim) {
        return;
    }
    if !map.creature(attacker).is_some_and(|c| c.owner.is_none()) {
        return;
    }
    // Not a blow that killed, nor one on the invulnerable.
    if !map.unit_view(victim).is_some_and(|u| u.alive)
        || crate::game::map::gm_dev::invulnerable(map, victim)
    {
        return;
    }
    let chance = daze_chance(roll.victim_level, roll.attacker_skill, roll.victim_defense);
    if daze_roll() < chance {
        crate::game::map::spells::cast_triggered(map, attacker, SPELL_DAZED, Some(victim));
    }
}

/// The chance, in percent, a creature's blow from behind dazes a player of
/// `level` with `defense` (mangos's `CalculateEffectiveDazeChance`): a
/// fifth at 30 and up, less below; a fifth of a point for each point of
/// skill over defense; no more than two fifths.
pub fn daze_chance(level: i32, skill: i32, defense: i32) -> f32 {
    let level = level as f32;
    let base = if level < 30.0 {
        0.65 * level + 0.5
    } else {
        20.0
    };
    (base + (skill - defense) as f32 * 0.2).clamp(0.0, 40.0)
}

#[cfg(test)]
thread_local! {
    /// A test's roll for a daze, in percent, in place of the dice.
    pub static FORCED_DAZE_ROLL: std::cell::Cell<Option<f32>> =
        const { std::cell::Cell::new(None) };
}

fn daze_roll() -> f32 {
    #[cfg(test)]
    if let Some(r) = FORCED_DAZE_ROLL.with(|f| f.get()) {
        return r;
    }
    rand::rng().random_range(0.0f32..100.0)
}

/// A spell (Windfury, Sword Specialization, Hand of Justice) owes `count`
/// main-hand attacks at once; Reckoning banks one for the next swing. An
/// extra attack can't give more, nor can a batch stack.
pub fn grant_extra_attacks(map: &mut Map, g: Guid, spell: u32, count: u32) {
    // The dead are owed nothing.
    if !map.unit_view(g).is_some_and(|u| u.alive) {
        return;
    }
    let Some(c) = map.combat_mut(g) else {
        return;
    };
    if spell == SPELL_RECKONING {
        if c.extra_attacks + c.held_extra_attacks < RECKONING_CAP {
            c.held_extra_attacks += 1;
        }
        return;
    }
    // Reckoning's bank holds off another batch until a swing arms it
    // (Thistle Tea's `ExtraAttacks.grant`).
    if c.in_extra_attack || c.extra_attacks > 0 || c.held_extra_attacks > 0 || count == 0 {
        return;
    }
    c.extra_attacks = count;
}

/// Thistle Tea's `AttackFeedback`: an ability that pays rage or energy and
/// discounts its power on a miss gives back 82% of what it cost when it is
/// avoided: rage on a dodge or parry, energy on a miss, dodge, parry or
/// immunity.
pub fn ability_avoided(map: &mut Map, caster: Guid, spell: u32, outcome_ex: u32, cost: u32) {
    let refundable = map.realm.data.spells.get(spell).is_some_and(|s| {
        s.attr(1, crate::game::data::spells::ATTR_EX_DISCOUNT_POWER_ON_MISS)
            && match s.power_type {
                1 => outcome_ex & (procs::EX_DODGE | procs::EX_PARRY) != 0,
                3 => {
                    outcome_ex
                        & (procs::EX_MISS | procs::EX_DODGE | procs::EX_PARRY | procs::EX_IMMUNE)
                        != 0
                }
                _ => false,
            }
    });
    if !refundable {
        return;
    }
    let power = map
        .realm
        .data
        .spells
        .get(spell)
        .map_or(0, |s| s.power_type.min(4) as u16);
    let refund = (cost as f32 * 0.82).round() as u32;
    if let Some(f) = map.unit_fields_mut(caster) {
        let max = f.get_u32(UNIT_FIELD_MAXPOWER1 + power);
        let now = f.get_u32(UNIT_FIELD_POWER1 + power);
        f.set_u32(UNIT_FIELD_POWER1 + power, (now + refund).min(max));
    }
}

/// Thistle Tea's `Pvp.contact`: a player who touches a PvP-flagged unit
/// is flagged in turn: attacking it, being attacked by a flagged player,
/// or helping a flagged player (or a flagged unit in a fight). Duelists and
/// free-for-all opponents aren't.
pub fn pvp_contact(map: &mut Map, me: Guid, other: Guid, role: Contact) {
    let now = map.now;
    if me == other || !map.players.contains_key(&me) {
        return;
    }
    // Another player's pet, totem or guardian is their side: its flag is
    // theirs (mangos's `GetBeneficiaryPlayer`).
    let owner = crate::game::map::pets::master(map, other);
    let side = if owner != other && map.players.contains_key(&owner) {
        owner
    } else {
        other
    };
    if side == me {
        return;
    }
    let Some(f) = map.unit_fields(side) else {
        return;
    };
    if !f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP) {
        return;
    }
    let other_player = map.players.contains_key(&side);
    let other_fighting = map
        .unit_fields(other)
        .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT));
    let other = side;
    if other_player && crate::game::map::duel::dueling(map, me, other) {
        return;
    }
    let ffa = |g: Guid| {
        map.players
            .get(&g)
            .is_some_and(|p| p.fields.has_flag(PLAYER_FLAGS, PLAYER_FLAGS_FFA_PVP))
    };
    if ffa(me) && ffa(other) {
        return;
    }
    let flags = match role {
        Contact::Attack => true,
        Contact::Attacked | Contact::Touch => other_player,
        Contact::Assist => other_player || other_fighting,
    };
    if flags && let Some(p) = map.players.get_mut(&me) {
        p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, true);
        p.combat.last_pvp = Some(now);
    }
}

/// A harmful spell of `attacker`'s side got to `victim`'s, as a hostile
/// act (mangos's `CasterHitTargetWithSpell`, a fight's or a PvP-enabling
/// spell's touch): the victim's player is flagged if the attacker's side
/// is (`SetInCombatWithAggressor`), then the attacker's player, a pet's
/// master for the pet, flagged and contested if the victim's side is
/// (`SetInCombatWithVictim`, by `GetBeneficiaryPlayer`). Duelists aren't;
/// a free-for-all attacker isn't contested (as [`crate::game::map::pvp::damaged`]).
pub fn hostile_spell_pvp(map: &mut Map, attacker: Guid, victim: Guid) {
    let (ma, mv) = (
        crate::game::map::pets::master(map, attacker),
        crate::game::map::pets::master(map, victim),
    );
    if ma == mv || !map.players.contains_key(&ma) || !map.players.contains_key(&mv) {
        return;
    }
    pvp_contact(map, mv, ma, Contact::Attacked);
    let flagged = map.players[&mv]
        .fields
        .has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP);
    if !flagged || crate::game::map::duel::dueling(map, ma, mv) {
        return;
    }
    pvp_contact(map, ma, mv, Contact::Attack);
    let ffa = map.players[&ma]
        .fields
        .has_flag(PLAYER_FLAGS, PLAYER_FLAGS_FFA_PVP);
    if !ffa {
        crate::game::map::pvp::contest(map, ma);
    }
}

/// How a player touched another unit, for [`pvp_contact`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Contact {
    Attack,
    Attacked,
    Assist,
    /// A spell of theirs reached it, helpful or harmful, hit or missed:
    /// another player's side flagged for PvP flags the caster (mangos's
    /// `Spell::FillTargetMap`, `UpdatePvP(true)` for a target whose
    /// beneficiary player is flagged).
    Touch,
}

/// A helpful spell `s` of `caster`'s landed on `target`: helping another
/// player's flagged side flags the helper; helping a flagged creature in a
/// fight does too, unless the help makes no threat (Thistle Tea's
/// `spell_contacts`, assistance with `combat?` only for a threat-making
/// spell).
pub fn assist_contact(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    s: &crate::game::data::spells::SpellTemplate,
) {
    use crate::game::data::spells as sd;
    let threat = !s.attr(1, sd::ATTR_EX_NO_THREAT) && !s.attr(2, sd::ATTR_EX2_NO_INITIAL_THREAT);
    if threat || guid::is_player(crate::game::map::pets::master(map, target)) {
        pvp_contact(map, caster, target, Contact::Assist);
    }
}

/// A next-swing spell (Heroic Strike, Cleave, Maul, Raptor Strike) waits
/// for the main hand's swing.
pub fn queue_next_swing(map: &mut Map, g: Guid, spell: u32) {
    if let Some(c) = map.combat_mut(g) {
        c.queued = Some(spell);
    }
}

/// `CMSG_CANCEL_CAST` takes a waiting next-swing spell back.
pub fn cancel_queued(map: &mut Map, g: Guid) {
    if let Some(c) = map.combat_mut(g) {
        c.queued = None;
    }
}

/// A heal makes threat for the healer on the creatures fighting the
/// healed that may fight the healer: half the health it gave back, by the
/// healer's threat, split among them (mangos's `threatAssist`, Thistle
/// Tea's `heal_threat`).
pub fn heal_threat(map: &mut Map, healer: Guid, healed: Guid, spell: u32, gained: u32) {
    use crate::game::data::spells as sd;
    if gained == 0 || healer.is_zero() {
        return;
    }
    let data = map.realm.data.clone();
    let s = data.spells.get(spell);
    // A spell of no threat, none at first, or none for its help, gives
    // none (mangos's `threatAssist`).
    if s.is_some_and(|s| {
        s.attr(1, sd::ATTR_EX_NO_THREAT)
            || s.attr(2, sd::ATTR_EX2_NO_INITIAL_THREAT)
            || s.attr(4, sd::ATTR_EX4_NO_HELPFUL_THREAT)
    }) {
        return;
    }
    let school = s.map_or(2, |s| s.school_mask());
    // `spell_threat`'s multiplier, as mangos's `DealHeal` takes it.
    let spell_mult = data
        .spells
        .threat
        .get(&spell)
        .map_or(1.0, |(_, m)| m.max(0.0));
    let fighting: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| c.alive() && c.threat.iter().any(|(t, _)| *t == healed))
        .map(|(_, c)| c.guid)
        .filter(|c| can_attack(map, *c, healer))
        .collect();
    if fighting.is_empty() {
        return;
    }
    // The spell's threat modifiers too (mangos's `threatAssist` through
    // `CalcThreat`: `SPELLMOD_THREAT`).
    let by_mods = s.map_or(1.0, |s| {
        crate::game::map::spells::mods::for_spell(map, healer, s)
            .apply(crate::game::map::spells::mods::OP_THREAT, 1.0)
            .max(0.0)
    });
    let threat =
        gained as f32 * 0.5 * spell_mult * by_mods * threat_multiplier(map, healer, school)
            / fighting.len() as f32;
    for c in fighting {
        crate::game::map::ai::add_threat(map, c, healer, threat);
    }
}

/// A seated, sleeping or kneeling player struck stands up, and what wants
/// them seated ends (food, drink): any blow, a miss or one absorbed whole
/// too, but no periodic tick (mangos's `DealDamage`, since 1.5; Thistle
/// Tea's `Emote.on_damage`). A creature stands as it enters the fight
/// ([`crate::game::map::creature_combat::stand_up`]).
pub fn stand_struck(map: &mut Map, victim: Guid) {
    if crate::game::map::gm_dev::invulnerable(map, victim) {
        return;
    }
    let Some(p) = map.players.get_mut(&victim) else {
        return;
    };
    // Standing, or dead (7).
    if matches!(p.stand_state(), 0 | 7) || p.death != Death::Alive {
        return;
    }
    p.set_stand_state(0);
    p.outbound.send(&SMSG_STANDSTATE_UPDATE {
        state: UnitStandState::Stand,
    });
    auras::interrupt(map, victim, INTERRUPT_NOT_SEATED);
}

/// Damage a unit: rage, threat, health, death.
pub fn deal_damage(map: &mut Map, attacker: Guid, victim: Guid, damage: u32) {
    deal_damage_school(map, attacker, victim, damage, 1);
}

/// Threat a unit's blows of `school` make, as a multiplier.
pub fn threat_multiplier(map: &Map, g: Guid, school: u32) -> f32 {
    auras::auras(map, g).map_or(1.0, |a| a.product_mask(AURA_MOD_THREAT, school))
}

/// Damage of `school` (a mask) on a unit: the rage it gives, threat, the
/// auras damage breaks, health, death.
pub fn deal_damage_school(map: &mut Map, attacker: Guid, victim: Guid, damage: u32, school: u32) {
    deal_damage_by(map, attacker, victim, damage, school, None);
}

/// Damage of `school` on a unit, by `spell` if a spell's: a death it causes
/// is laid to the spell (see [`kill_by`]).
pub fn deal_damage_by(
    map: &mut Map,
    attacker: Guid,
    victim: Guid,
    damage: u32,
    school: u32,
    spell: Option<u32>,
) {
    if map.unit_view(victim).is_some_and(|u| u.alive) {
        stand_struck(map, victim);
    }
    deal_damage_with(map, attacker, victim, damage, school, spell, true);
}

/// A periodic aura's damage: [`deal_damage_by`], but only a channel's keeps
/// a creature on its leash (Thistle Tea's `CombatLeash.on_damage`).
pub fn deal_periodic_damage(
    map: &mut Map,
    attacker: Guid,
    victim: Guid,
    damage: u32,
    school: u32,
    spell: Option<u32>,
    channel: bool,
) {
    deal_damage_with(map, attacker, victim, damage, school, spell, channel);
}

fn deal_damage_with(
    map: &mut Map,
    attacker: Guid,
    victim: Guid,
    damage: u32,
    school: u32,
    spell: Option<u32>,
    extends: bool,
) {
    // A GM in god mode takes nothing, and the dead nothing more (mangos's
    // `DealMeleeDamage` and `DealSpellDamage`): a blow or a tick that gets
    // to a corpse doesn't kill it again.
    if crate::game::map::gm_dev::invulnerable(map, victim)
        || map.unit_view(victim).is_none_or(|u| !u.alive)
    {
        return;
    }
    // A spirit of redemption takes nothing, unattackable (mangos's
    // `SPELL_AURA_MOD_UNATTACKABLE`, Thistle Tea's absorbing it all), but
    // the death its end brings.
    if crate::game::map::spells::redemption::spirit(map, victim)
        && !map.spells.redeemed.contains(&victim)
    {
        return;
    }
    // A pet's blows count as its master's for duels and honor.
    let by = crate::game::map::pets::master(map, attacker);
    let damage = if guid::is_player(victim) && guid::is_player(by) {
        crate::game::map::duel::soften(map, by, victim, damage)
    } else {
        damage
    };
    if guid::is_player(victim) {
        crate::game::map::pvp::damaged(map, by, victim, damage);
        crate::game::map::arena::damaged(map, by, victim, damage);
    }
    if damage == 0 || crate::game::map::event_ai::feigning(map, victim) {
        return;
    }
    // The first damage that gets through taps a creature (one absorbed
    // whole doesn't, as `damage` is what is left).
    crate::game::map::tap::damaged(map, victim, attacker);
    // No rage from one's own harm, the world's (mangos's `dealer !=
    // victim`).
    if attacker != victim {
        gain_rage(map, victim, damage, true);
    }
    crate::game::map::creature_combat::damaged(map, victim, attacker, damage);
    if guid::is_unit(victim) {
        let threat = damage as f32 * threat_multiplier(map, attacker, school);
        crate::game::map::ai::add_threat(map, victim, attacker, threat);
    }
    // Thistle Tea's `break_on_damage`, whatever the source (a fall too):
    // confusion (Polymorph, Blind) and what damage interrupts (Sap, Gouge).
    auras::remove_where(map, victim, |h| {
        h.interrupt_flags & INTERRUPT_DAMAGE != 0 || (h.has(AURA_MOD_CONFUSE) && !h.positive)
    });
    if attacker != victim && extends {
        crate::game::map::creature_combat::hostile_act(map, victim);
    }
    let Some(f) = map.unit_fields_mut(victim) else {
        return;
    };
    let left = f.get_u32(UNIT_FIELD_HEALTH).saturating_sub(damage);
    // A linked feign death keeps it down, not dead: a killing blow, before
    // any floor holds it up (Thistle Tea's `LinkedFeignDeath.prevent`, ahead
    // of the invincibility threshold).
    if left == 0 && crate::game::map::event_ai::feign_death(map, victim) {
        return;
    }
    // A script can keep a creature from falling below a floor, and an
    // unkillable one (vmangos's `CREATURE_STATIC_FLAG_UNKILLABLE`) never
    // falls below one health unless a script's floor lets it (Thistle
    // Tea's invincibility threshold).
    let floor = crate::game::map::event_ai::invincible_floor(map, victim);
    // An undying player (`.god`) stops at 1.
    let floor = if crate::game::map::gm_dev::undying(map, victim) {
        floor.max(1)
    } else {
        floor
    };
    let Some(f) = map.unit_fields_mut(victim) else {
        return;
    };
    let prevented = floor > 0 && left == 0;
    let left = if floor > 0 {
        left.max(floor.min(f.get_u32(UNIT_FIELD_HEALTH)))
    } else {
        left
    };
    // A death the floor kept off, which a script may hear of (mangos's
    // `JustPreventedDeath`).
    if prevented {
        crate::game::map::bwl::death_prevented(map, victim);
    }
    // A blow that doesn't kill may wear a point off each player's gear; a
    // killing blow doesn't (mangos's `HandleDamageDealt`, which `Kill`
    // takes the place of).
    if left > 0 {
        crate::game::map::vendor::wear_from_blow(map, attacker, victim);
    }
    // A priest with Spirit of Redemption becomes a spirit instead: the kill's
    // honor is had now, the spirit's end giving none (mangos's `Unit::Kill`
    // rewards before the spirit, Thistle Tea's honor at the initial
    // defeat); in a battleground, defeated all the same, the death to come
    // (Thistle Tea's `count_death?: false`). The fatal blow's wear is had
    // now too, the spirit's end (by its own hand) wearing nothing.
    if left == 0 && crate::game::map::spells::redemption::saves(map, victim) {
        let by = crate::game::map::pets::master(map, attacker);
        honor(map, by, victim);
        crate::game::map::battleground::defeated(map, victim, by, false);
        death_wear(map, victim, by, spell);
        return;
    }
    let Some(f) = map.unit_fields_mut(victim) else {
        return;
    };
    f.set_u32(UNIT_FIELD_HEALTH, left);
    sync_aura_state(map, victim);
    if left == 0 {
        kill_by(map, attacker, victim, spell);
    }
}

/// A player's fatal blow (their death, or the Spirit of Redemption it
/// becomes) wears their gear down a tenth, if no player was behind it,
/// `by` being the killer's master: not by a player or their pet, nor by
/// their own hand (the world's harm too, but a fall's, which
/// `movement::fall_damage` wears), nor in a battleground, nor by Suicide or
/// a spell flagged `no_durability_loss` (mangos's `Unit::Kill`, its
/// `responsiblePlayer`).
pub fn death_wear(map: &mut Map, victim: Guid, by: Guid, spell: Option<u32>) {
    let battleground = crate::game::instances::instance_type(&map.realm, map.key.map) == 3;
    let spared =
        spell.is_some_and(|id| {
            id == crate::game::map::spells::redemption::SUICIDE
                || map.realm.data.spells.get(id).is_some_and(|s| {
                    s.attr(3, crate::game::data::spells::ATTR_EX3_NO_DURABILITY_LOSS)
                })
        });
    if guid::is_player(victim) && !battleground && !spared && !guid::is_player(by) {
        wear_at_death(map, victim);
    }
}

/// A tenth off everything worn, and the client told (mangos's
/// `DurabilityLossAll(0.10f)`, `SMSG_DURABILITY_DAMAGE_DEATH`).
pub fn wear_at_death(map: &mut Map, victim: Guid) {
    // Gear that doesn't wear (`.durability off`) loses nothing, and the
    // client isn't told it did.
    if !map
        .players
        .get(&victim)
        .is_some_and(crate::game::map::vendor::wears)
    {
        return;
    }
    crate::game::map::vendor::wear(map, victim, 0.10);
    if let Some(p) = map.players.get(&victim) {
        p.outbound
            .send(&wow_world_messages::vanilla::SMSG_DURABILITY_DAMAGE_DEATH {});
    }
}

/// `victim` dies at `killer`'s hand, once: the dead aren't killed again
/// (Thistle Tea's `Entity.kill`).
pub fn kill(map: &mut Map, killer: Guid, victim: Guid) {
    kill_by(map, killer, victim, None);
}

/// `victim` dies at `killer`'s hand, by `spell` if a spell killed them: one
/// with `no_durability_loss` (ex3 0x20: a Ritual of Doom's sacrifice), or
/// Suicide (27965), wears nothing down (Thistle Tea's
/// `Durability.death_loss?`).
pub fn kill_by(map: &mut Map, killer: Guid, victim: Guid, spell: Option<u32>) {
    if map.unit_view(victim).is_none_or(|u| !u.alive)
        || DYING.with(|d| d.borrow().contains(&victim))
    {
        return;
    }
    // An undying player (`.god`) isn't killed: left at 1 health, at least.
    if crate::game::map::gm_dev::undying(map, victim) {
        if let Some(f) = map.unit_fields_mut(victim)
            && f.get_u32(UNIT_FIELD_HEALTH) == 0
        {
            f.set_u32(UNIT_FIELD_HEALTH, 1);
        }
        return;
    }
    // The creatures fighting it, to fight on after.
    let fighting: Vec<Guid> = map
        .combat(victim)
        .map(|c| c.attackers.iter().copied().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain([killer])
        .filter(|a| {
            guid::is_unit(*a)
                && map
                    .combat(*a)
                    .is_some_and(|c| c.attacking && c.victim == victim)
        })
        .collect();
    // Being killed, it isn't killed again meanwhile (by a kill's proc).
    {
        let _dying = Dying::enter(victim);
        kill_once(map, killer, victim, spell);
    }
    for a in fighting {
        crate::game::map::ai::victim_died(map, a);
    }
}

thread_local! {
    /// The units being killed now, on this thread.
    static DYING: std::cell::RefCell<Vec<Guid>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A unit on [`DYING`] while it lives: taken off as it goes, a panic's
/// unwinding too. Left on, the unit could never be killed on that thread
/// again (a worker's thread runs every map on it), at no health.
struct Dying(Guid);

impl Dying {
    fn enter(victim: Guid) -> Self {
        DYING.with(|d| d.borrow_mut().push(victim));
        Self(victim)
    }
}

impl Drop for Dying {
    fn drop(&mut self) {
        DYING.with(|d| d.borrow_mut().retain(|g| *g != self.0));
    }
}

fn kill_once(map: &mut Map, killer: Guid, victim: Guid, spell: Option<u32>) {
    #[cfg(test)]
    map.faults.check(&format!("death of {victim}"));
    let attackers: Vec<Guid> = map
        .combat(victim)
        .map(|c| c.attackers.iter().copied().collect())
        .unwrap_or_default();
    for a in attackers.into_iter().chain([killer]) {
        if map
            .combat(a)
            .is_some_and(|c| c.attacking && c.victim == victim)
        {
            stop_attack(map, a);
        }
        if let Some(c) = map.combat_mut(a) {
            c.attackers.remove(&victim);
            if c.auto_repeat.is_some_and(|r| r.target == victim) {
                c.auto_repeat = None;
            }
        }
    }
    stop_attack(map, victim);
    // Combo points on the dead go with them, and the dead's own (mangos's
    // `ClearComboPointHolders`, Thistle Tea's `clear_combo_target`).
    let holders: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.caster.combo_target == victim)
        .map(|p| p.guid)
        .chain(guid::is_player(victim).then_some(victim))
        .collect();
    for h in holders {
        crate::game::map::spells::clear_combo_points(map, h);
    }
    // The dead's reactive windows close with their bits (mangos's
    // `ClearAllReactives` at death).
    if let Some(c) = map.combat_mut(victim) {
        c.defense_window = None;
        c.hunter_parry_window = None;
        c.dodge_combo_until = None;
    }
    if let Some(f) = map.unit_fields_mut(victim) {
        let bits = f.get_u32(UNIT_FIELD_AURASTATE);
        let cleared =
            bits & !(AURA_STATE_DEFENSE | AURA_STATE_HUNTER_PARRY | AURA_STATE_HEALTHLESS_20);
        if cleared != bits {
            f.set_u32(UNIT_FIELD_AURASTATE, cleared);
        }
    }
    let worth = kill_procs(map, killer, victim);
    // A pet's kill is its master's too (mangos's `Unit::Kill` procs the
    // killer's owner as well). The killer's procs of the kill come before
    // the victim's death takes its auras (mangos's `Unit::Kill`, before
    // `RemoveAllAurasOnDeath`): a Drain Soul ended by it still procs.
    let master = crate::game::map::pets::master(map, killer);
    let master_worth = master != killer && kill_procs(map, master, victim);
    if killer != victim {
        for (by, counts) in [(master, master_worth && master != victim), (killer, worth)] {
            if counts {
                procs::trigger(
                    map,
                    procs::Event {
                        actor: by,
                        target: victim,
                        actor_flags: procs::PROC_KILL,
                        ex: procs::EX_NORMAL_HIT,
                        ..Default::default()
                    },
                );
            }
        }
    }
    if guid::is_unit(killer) {
        crate::game::map::event_ai::kill(map, killer, victim);
    }
    if guid::is_player(victim) {
        tracing::debug!(
            "{} killed by {}",
            map.players
                .get(&victim)
                .map_or_else(|| "?".to_owned(), |p| p.name.to_string()),
            map.creature(killer)
                .map(|c| format!("creature {} at {:?}", c.entry, c.position))
                .or_else(|| map.players.get(&killer).map(|p| p.name.to_string()))
                .unwrap_or_else(|| format!("{killer}")),
        );
        let by = crate::game::map::pets::master(map, killer);
        honor(map, by, victim);
        crate::game::map::battleground::died(map, victim, by);
        crate::game::map::arena::died(map, victim, by);
        death::kill(map, victim);
        death_wear(map, victim, by, spell);
        crate::game::map::arena::after_kill(map);
    } else {
        // A creature a player charmed or possessed is worth nothing to
        // anyone, though its death lets it go first (Thistle Tea's
        // `KillCredit.capture`, before the blow releases the control).
        let controlled = map
            .creature(victim)
            .is_some_and(|c| c.owner.is_some_and(guid::is_player));
        // A creature's pet, guardian or totem: no loot and no XP, but its
        // killer's reputation and quest credit (mangos-classic's
        // `RewardSinglePlayerAtKill`, for a victim not
        // `UNIT_FLAG_PLAYER_CONTROLLED`).
        let creatures = crate::game::map::pets::state(map, victim).is_some_and(|s| {
            !matches!(
                s.kind,
                crate::game::map::pets::Kind::Charmed | crate::game::map::pets::Kind::Possessed
            ) && !guid::is_player(crate::game::map::pets::master(map, s.owner))
        }) && map.unit_fields(victim).is_some_and(|f| {
            !f.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::pets::UNIT_FLAG_PLAYER_CONTROLLED,
            )
        });
        if creatures {
            let tapper = map
                .creature(victim)
                .and_then(|c| c.tapped_by)
                .unwrap_or_else(|| crate::game::map::pets::master(map, killer));
            if guid::is_player(tapper) {
                reward(map, tapper, victim);
                crate::game::map::reputation::creature_killed(map, tapper, victim);
            }
        }
        if crate::game::map::pets::died(map, killer, victim) {
            return;
        }
        // Mostly NPCs' doing, it isn't the tapper's: the killer's side has
        // the quest credit alone (Thistle Tea's `KillReward.selection`).
        let allowed = crate::game::map::creature_combat::loot_allowed(map, victim);
        let tapper = map
            .creature(victim)
            .and_then(|c| c.tapped_by)
            .filter(|_| allowed)
            .unwrap_or_else(|| crate::game::map::pets::master(map, killer));
        if guid::is_player(tapper) && !controlled {
            if allowed {
                reward(map, tapper, victim);
            } else {
                crate::game::map::quests::killed(map, tapper, victim);
            }
        }
        crate::game::map::local_defense::fallen(map, victim, killer);
        crate::game::map::ai::die_rewarding(map, victim, tapper, killer, !controlled);
    }
    if killer != victim {
        procs::trigger(
            map,
            procs::Event {
                actor: killer,
                target: victim,
                target_flags: procs::PROC_KILLED,
                ex: procs::EX_NORMAL_HIT,
                ..Default::default()
            },
        );
    }
}

/// Whether `killer`'s on-kill procs count the kill of `victim` (Thistle
/// Tea's `KillFeedback.eligible?`, mangos's `isHonorOrXPTarget`): for a
/// player, a victim not grey and no pet, totem or creature worth no XP; a
/// creature's kills all count.
fn kill_procs(map: &Map, killer: Guid, victim: Guid) -> bool {
    if !guid::is_player(killer) {
        return true;
    }
    let (kl, vl) = (
        map.unit_view(killer).map_or(1, |u| u.level),
        map.unit_view(victim).map_or(1, |u| u.level),
    );
    let rewarding = map.creature(victim).is_none_or(|c| {
        c.pet.is_none()
            && map
                .realm
                .data
                .creature_template(c.entry)
                .is_none_or(|t| t.xp_rate() != 0.0 && t.static_flags1 & 0x2 == 0)
    });
    rewarding && vl > gray_level(kl)
}

/// How near a corpse a member must be for a share of the kill, by
/// default (`[players] group_reward_yards`).
pub const GROUP_REWARD_YARDS: f32 = 74.0;

/// Whether a member at `member` is near enough `source` for a share of a
/// group's reward: anywhere in a dungeon or raid copy, else within
/// `[players] group_reward_yards` (mangos's `IsAtGroupRewardDistance`:
/// `GetMap()->IsDungeon()`, which a battleground isn't).
pub fn at_reward_distance(map: &Map, member: Vector3d, source: Vector3d) -> bool {
    matches!(
        crate::game::instances::instance_type(&map.realm, map.key.map),
        1 | 2
    ) || dist(member, source) <= map.cfg.players.group_reward_yards
}

/// Thistle Tea's `Experience.group_rate`: three, four and five share more;
/// a raid's members less the more of them.
pub fn group_xp_rate(count: usize) -> f32 {
    match count {
        3 => 1.166,
        4 => 1.3,
        5 => 1.4,
        n if n > 5 => (1.0 - n as f32 * 0.05).max(0.01),
        _ => 1.0,
    }
}

/// Each member's share of a kill's XP (Thistle Tea's
/// `Experience.group_rewards`), from their levels: a level-weighted share
/// of the highest non-grey level's XP, halved plus one when a member above
/// that level is there, and nothing for those above it. Also the highest
/// non-grey level, for the pets.
pub fn group_shares(levels: &[u32], mob_level: u32, o: &KillXp) -> (Vec<u32>, Option<u32>) {
    let sum: u32 = levels.iter().sum();
    let top = levels.iter().copied().max().unwrap_or(0);
    let top_not_gray = levels
        .iter()
        .copied()
        .filter(|l| mob_level > gray_level(*l))
        .max();
    let Some(level) = top_not_gray.filter(|_| sum > 0) else {
        return (vec![0; levels.len()], None);
    };
    let base = kill_xp_with(level, level, mob_level, o) as f32;
    let rate = group_xp_rate(levels.len());
    let shares = levels
        .iter()
        .map(|l| {
            if base <= 0.0 {
                return 0;
            }
            let share = base * rate * *l as f32 / sum as f32;
            if top == level {
                share as u32
            } else {
                (share / 2.0) as u32 + 1
            }
        })
        .collect();
    (shares, Some(level))
}

/// XP and kill credit for a creature's death: the tapper's, or shared with
/// their group near the corpse (Thistle Tea's `GroupReward`): the living
/// and the original tapper count; the dead earn nothing, and a released
/// spirit no quest credit. The group is the one the tapper was in as they
/// tapped it, while it holds together, the tapper too if they have left
/// it, and not a group they have joined since (mangos's
/// `GetGroupLootRecipient`, Thistle Tea's `KillReward.selection`); else
/// the tapper's own.
fn reward(map: &mut Map, tapper: Guid, victim: Guid) {
    let Some(at) = map.creature(victim).map(|c| c.position) else {
        return;
    };
    let group = map
        .creature(victim)
        .and_then(|c| crate::game::map::tap::group_of(c, tapper));
    let members = crate::game::map::tap::recipients(&map.realm, tapper, group);
    let near: Vec<(Guid, u32, Death)> = members
        .into_iter()
        .filter_map(|g| {
            let p = map.players.get(&g)?;
            (at_reward_distance(map, p.position(), at) || g == tapper)
                .then(|| (g, u32::from(p.level()), p.death))
        })
        .collect();
    let o = kill_options(map, victim);
    let mob_level = map
        .creature(victim)
        .map(|c| c.fields.get_u32(UNIT_FIELD_LEVEL))
        .unwrap_or(1);
    let sharing: Vec<&(Guid, u32, Death)> = near
        .iter()
        .filter(|m| m.2 == Death::Alive || m.0 == tapper)
        .collect();
    let levels: Vec<u32> = sharing.iter().map(|m| m.1).collect();
    let (shares, top) = group_shares(&levels, mob_level, &o);
    let solo = sharing.len() <= 1;
    for (m, share) in sharing.iter().zip(shares) {
        if m.2 != Death::Alive {
            continue;
        }
        let xp = if top.is_some_and(|t| m.1 <= t) {
            crate::config::scaled(share, map.cfg.rates.xp_kill)
        } else {
            0
        };
        crate::game::map::stats::give_xp(map, m.0, xp, Some(victim));
        let pet = if solo {
            crate::game::map::pets::KillShare::Solo
        } else {
            crate::game::map::pets::KillShare::Group {
                xp: share,
                max_level: top.unwrap_or(0),
            }
        };
        crate::game::map::pets::owner_killed(map, m.0, victim, pet);
    }
    // Quest credit for the living, and the dead not yet released.
    for (g, _, death) in near {
        if matches!(death, Death::Alive | Death::Dead { .. }) {
            crate::game::map::quests::killed(map, g, victim);
        }
    }
}

/// An honorable kill: honor for the killer and those who helped.
fn honor(map: &mut Map, killer: Guid, victim: Guid) {
    crate::game::map::pvp::player_killed(map, killer, victim);
}

/// Run the players' swings and shots, close their reactive windows, and
/// expire their combat and PvP flags.
pub fn update_players(map: &mut Map) {
    let now = map.now;
    let swinging: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.combat.attacking || p.combat.extra_attacks > 0)
        .map(|p| p.guid)
        .collect();
    for g in swinging {
        try_swing(map, g);
    }
    crate::game::map::ranged::update(map);
    let windows: Vec<Guid> = map
        .players
        .values()
        .filter(|p| {
            p.combat.defense_window.is_some()
                || p.combat.hunter_parry_window.is_some()
                || p.combat.dodge_combo_until.is_some()
        })
        .map(|p| p.guid)
        .collect();
    for g in windows {
        expire_windows(map, g);
    }
    let fighting: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.combat.last_action.is_some())
        .map(|p| p.guid)
        .collect();
    for g in fighting {
        sync_aura_state(map, g);
    }
    leave_finished_fights(map);
    for p in map.players.values_mut() {
        if p.combat
            .last_pvp
            .is_some_and(|t| now.duration_since(t) > PVP_TIMEOUT)
        {
            p.combat.last_pvp = None;
            if !p.fields.has_flag(
                PLAYER_FLAGS,
                crate::game::map::player::PLAYER_FLAGS_PVP_DESIRED,
            ) {
                p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, false);
            }
        }
    }
}

/// Players whose fight is over leave it, looked at every
/// [`COMBAT_CHECK`] (mangos's `CombatManager::Update`): one is over when
/// no living creature holds the player on its threat list or attacks them,
/// the PvP timer has run out, and no Bloodrage holds them in it. Swinging
/// at no one in reach, or meaning to, holds no one: the attack stops with
/// the fight (mangos's `HandleExitCombat`, its `CombatStop`).
fn leave_finished_fights(map: &mut Map) {
    let now = map.now;
    let due: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.combat.next_check.is_some_and(|t| t <= now))
        .map(|p| p.guid)
        .collect();
    if due.is_empty() {
        return;
    }
    // The creatures in a fight, looked for once, and only if a due
    // player's answer needs them; each such player asks them in turn,
    // stopping at the first that holds them, where a set of everyone each
    // holds was built for one or two players (review 5's combat 7).
    let mut fighting: Option<Vec<usize>> = None;
    let data = map.realm.data.clone();
    for g in due {
        let Some(p) = map.players.get(&g) else {
            continue;
        };
        let in_combat = p.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT);
        // In a fight, what holds them besides a creature (the PvP timer,
        // Bloodrage); out of one, a war a creature's fight began.
        let asks = if in_combat {
            let timed = p.combat.combat_until.is_some_and(|t| t > now);
            let forced = p.auras.has_spell(SPELL_BLOODRAGE_REGEN);
            !timed && !forced
        } else {
            !p.temporary_war.is_empty()
        };
        let held = asks && {
            let keys = fighting.get_or_insert_with(|| {
                map.awake_where(
                    |c| c.alive() && (c.combat.attacking || !c.threat.is_empty()),
                    |k, _| k,
                )
            });
            keys.iter().filter_map(|&k| map.creatures.get(k)).any(|c| {
                (c.combat.attacking && c.combat.victim == g)
                    || c.threat.iter().any(|(t, _)| *t == g)
            })
        };
        let Some(p) = map.players.get_mut(&g) else {
            continue;
        };
        p.combat.next_check = Some(now + COMBAT_CHECK);
        if !in_combat {
            // Out of combat, with a war a creature's fight began (it took
            // them on its list, and never fought them): over once none
            // holds them.
            if !held {
                p.combat.next_check = None;
                crate::game::map::reputation::end_temporary_war(p, &data);
            }
            continue;
        }
        if held || !asks {
            continue;
        }
        leave_combat(map, g);
        // What it changed, the next asks afresh.
        fighting = None;
    }
}

/// The aura states of creatures in a fight: below a fifth of health
/// (Execute), and their reactive windows.
pub fn update_creatures(map: &mut Map) {
    let fighting: Vec<Guid> = map.awake_where(
        |c| c.alive() && (c.combat.attacking || !c.combat.attackers.is_empty()),
        |_, c| c.guid,
    );
    for g in fighting {
        expire_windows(map, g);
        sync_aura_state(map, g);
    }
    crate::game::map::creature_combat::update(map);
}

/// `g` feigned death and was believed (mangos's `SetFeignDeath` with
/// `UNIT_STAT_FEIGN_DEATH`, Thistle Tea's `FeignDeath.target_lost`): while
/// it lasts no creature notices it or picks it (mangos's
/// `CanAttackOnSight` and `IsOfflineTarget`), and now every cast at it is
/// interrupted, and the players swinging or shooting at it stop, in their
/// fight still. Its own fight is the aura's to end.
pub fn feigned(map: &mut Map, g: Guid) {
    if let Some(c) = map.combat_mut(g) {
        c.feigning = true;
    }
    let at_it = |caster: &crate::game::map::spells::Caster| {
        caster
            .casting
            .as_ref()
            .is_some_and(|c| c.targets.unit == Some(g))
    };
    let mut casting: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.guid != g && at_it(&p.caster))
        .map(|p| p.guid)
        .collect();
    casting.extend(map.casting_where(|c| c.guid != g && at_it(&c.caster), |_, c| c.guid));
    for c in casting {
        crate::game::map::spells::interrupt(map, c);
    }
    let attacking: Vec<Guid> = map
        .players
        .values()
        .filter(|p| {
            p.guid != g
                && ((p.combat.attacking && p.combat.victim == g)
                    || p.combat.auto_repeat.is_some_and(|a| a.target == g))
        })
        .map(|p| p.guid)
        .collect();
    for p in attacking {
        stop_attack(map, p);
        crate::game::map::ranged::cancel(map, p, true);
    }
}

/// Whether a believed Feign Death holds `g` (see [`feigned`]).
pub fn feigning(map: &Map, g: Guid) -> bool {
    map.combat(g).is_some_and(|c| c.feigning)
}

/// A unit stops fighting: out of combat, attacking nothing.
pub fn leave_combat(map: &mut Map, g: Guid) {
    stop_attack(map, g);
    let data = map.realm.data.clone();
    if let Some(p) = map.players.get_mut(&g) {
        crate::game::map::reputation::end_temporary_war(p, &data);
    }
    if let Some(c) = map.combat_mut(g) {
        c.attackers.clear();
        c.last_action = None;
        c.combat_until = None;
        c.next_check = None;
        c.auto_repeat = None;
    }
    if let Some(f) = map.unit_fields_mut(g) {
        f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn even(level: i32) -> Roll {
        Roll {
            attacker_skill: level * 5,
            victim_defense: level * 5,
            attacker_is_player: true,
            victim_is_player: true,
            attacker_level: level,
            victim_level: level,
            crit: 5.0,
            dodge: 5.0,
            parry: 5.0,
            standing: true,
            can_crit: true,
            ..Default::default()
        }
    }

    #[test]
    fn the_hit_table_rolls_in_order() {
        let r = even(10);
        assert_eq!(roll_outcome(&r, 0), Outcome::Miss);
        assert_eq!(roll_outcome(&r, 499), Outcome::Miss);
        assert_eq!(roll_outcome(&r, 500), Outcome::Dodge);
        assert_eq!(roll_outcome(&r, 1000), Outcome::Parry);
        assert_eq!(roll_outcome(&r, 1500), Outcome::Crit);
        assert_eq!(roll_outcome(&r, 2000), Outcome::Hit);
        // From behind, a player can't dodge or parry.
        let behind = Roll {
            from_behind: true,
            ..r
        };
        assert_eq!(roll_outcome(&behind, 500), Outcome::Crit);
        // A sitting player is crit, and never missed.
        let sitting = Roll {
            standing: false,
            ..r
        };
        assert_eq!(roll_outcome(&sitting, 0), Outcome::Crit);
    }

    #[test]
    fn players_glance_off_creatures_of_their_level() {
        let r = Roll {
            victim_is_player: false,
            ..even(20)
        };
        // Miss 5%, dodge 5, parry 5, then glancing's 10%.
        assert_eq!(roll_outcome(&r, 1499), Outcome::Parry);
        assert_eq!(roll_outcome(&r, 1500), Outcome::Glancing);
        assert_eq!(roll_outcome(&r, 2499), Outcome::Glancing);
        assert_eq!(roll_outcome(&r, 2500), Outcome::Crit);
    }

    #[test]
    fn dual_wielding_misses_nineteen_percent_more_on_white_swings() {
        let r = Roll {
            dual_wield: true,
            ..even(20)
        };
        assert_eq!(roll_outcome(&r, 2399), Outcome::Miss);
        assert_eq!(roll_outcome(&r, 2400), Outcome::Dodge);
        // An ability's swing doesn't pay it.
        assert_eq!(roll_special(&r, 500, 9999), Outcome::Dodge);
    }

    #[test]
    fn creatures_far_above_crush_and_rangers_only_miss_or_crit() {
        let r = Roll {
            attacker_is_player: false,
            attacker_skill: 100,
            victim_defense: 75,
            attacker_level: 20,
            victim_level: 15,
            ..even(15)
        };
        // A skill gap of 25: miss, dodge and parry 4% each, crit 6%, then
        // crushing's 35%.
        assert_eq!(roll_outcome(&r, 1799), Outcome::Crit);
        assert_eq!(roll_outcome(&r, 1800), Outcome::Crushing);
        assert_eq!(roll_outcome(&r, 5299), Outcome::Crushing);
        assert_eq!(roll_outcome(&r, 5300), Outcome::Hit);
        let ranged = Roll {
            ranged: true,
            ..even(20)
        };
        assert_eq!(roll_outcome(&ranged, 600), Outcome::Crit);
        assert_eq!(roll_outcome(&ranged, 1100), Outcome::Hit);
        let no_parry = Roll {
            victim_is_player: false,
            no_parry: true,
            ..even(20)
        };
        assert_eq!(roll_outcome(&no_parry, 1100), Outcome::Glancing);
    }

    #[test]
    fn skill_ups_follow_mangos_and_glancing_keeps_its_share() {
        // Far below the cap, a likely gain; at it, none.
        assert!(combat_skill_chance(10, 100, 20, None) > 75.0);
        assert!(combat_skill_chance(99, 100, 20, None) < 1.0);
        assert_eq!(combat_skill_chance(100, 100, 20, Some(100)), 0.0);
        let low = glancing_factor(15, false, 0.0);
        let high = glancing_factor(15, false, 1.0);
        assert!(
            (low - 0.55).abs() < 0.001 && (high - 0.75).abs() < 0.001,
            "{low} {high}"
        );
    }

    #[test]
    fn kill_xp_matches_vmangos() {
        // Level 1 killing level 1: ((5 + 45) * 20 / 10 + 1) / 2 = 50.
        assert_eq!(kill_xp(1, 1, false, 1.0), 50);
        assert_eq!(kill_xp(10, 2, false, 1.0), 0);
        assert_eq!(kill_xp(10, 10, true, 1.0), 2 * kill_xp(10, 10, false, 1.0));
    }

    #[test]
    fn armor_stops_up_to_three_quarters() {
        assert!((armor_reduction(400, 0) - 0.5).abs() < 0.001);
        assert_eq!(armor_reduction(1_000_000, 60), 0.75);
    }

    #[test]
    fn a_plain_hit_parses_in_wow_messages() {
        let frame = attacker_state(Guid::new(1), Guid::new(2), HITINFO_NORMALSWING2, 7, 1, 0);
        let m =
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::read_unencrypted(&frame[..]);
        assert!(m.is_ok(), "{m:?}");
        // The full layout is as long as the plain one.
        let full = attacker_state_full(
            Guid::new(1),
            Guid::new(2),
            HITINFO_NORMALSWING2 | HITINFO_LEFTSWING | HITINFO_ABSORB,
            7,
            1,
            3,
            5,
            0,
            0,
        );
        assert_eq!(full.len(), frame.len());
    }
}
