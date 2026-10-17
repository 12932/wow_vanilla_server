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
    Vector3d,
};

use crate::game::conn::encode;
use crate::game::data::items::Body;
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

/// What the victim did.
pub const VICTIMSTATE_UNAFFECTED: u32 = 0;
pub const VICTIMSTATE_NORMAL: u32 = 1;
pub const VICTIMSTATE_DODGE: u32 = 2;
pub const VICTIMSTATE_PARRY: u32 = 3;
pub const VICTIMSTATE_BLOCKS: u32 = 5;
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

/// A player leaves combat this long after their last blow given or taken.
pub const COMBAT_TIMEOUT: Duration = Duration::from_secs(6);
/// A PvP flag lasts this long after the last PvP blow.
pub const PVP_TIMEOUT: Duration = Duration::from_secs(300);
/// An error for a swing that can't land is sent at most this often.
const SWING_ERROR_EVERY: Duration = Duration::from_millis(1000);
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
    pub last_action: Option<Instant>,
    pub last_pvp: Option<Instant>,
    pub swing_error_at: Option<Instant>,
    /// Units fighting this one.
    pub attackers: AHashSet<Guid>,
    /// A next-swing spell waiting for the main hand (Heroic Strike).
    pub queued: Option<u32>,
    /// Auto Shot, Shoot: repeating at the ranged weapon's speed.
    pub auto_repeat: Option<AutoRepeat>,
    /// Main-hand attacks owed at once, and Reckoning's banked ones.
    pub extra_attacks: u32,
    pub held_extra_attacks: u32,
    pub in_extra_attack: bool,
    /// After a dodge, parry or block: the window's target and its end.
    pub defense_window: Option<(Guid, Instant)>,
    pub hunter_parry_window: Option<(Guid, Instant)>,
    /// When the combo point a dodge gave a warrior (Overpower) runs out.
    pub dodge_combo_until: Option<Instant>,
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
    pub attacker_is_player: bool,
    pub victim_is_player: bool,
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

    fn capped_avoidance_skill_diff(&self) -> i32 {
        self.attacker_skill.min(self.attacker_level * 5) - self.victim_level * 5
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
        if self.ranged || self.avoidance_disabled || (self.victim_is_player && self.from_behind) {
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
        let d = self.capped_avoidance_skill_diff() as f32;
        let bonus = if d < -10.0 { d * 0.6 } else { d * 0.2 };
        bp(self.low_level(self.parry - bonus).max(0.0))
    }

    fn glancing(&self) -> i32 {
        if self.ranged || !self.attacker_is_player || self.victim_is_player || self.special {
            return 0;
        }
        bp((10.0 - self.capped_skill_diff() as f32 * 2.0).max(0.0)).min(4000)
    }

    fn block(&self) -> i32 {
        if self.ranged
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
        if self.attacker_is_player || self.special || self.ranged {
            return 0;
        }
        if self.always_crush {
            return 10_000;
        }
        let diff = self.skill_diff();
        if diff >= 15 { diff * 200 - 1500 } else { 0 }
    }
}

fn bp(chance: f32) -> i32 {
    (chance * 100.0) as i32
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
    if o.no_xp {
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
    let elite = if o.pet_victim || !o.elite {
        1.0
    } else if o.dungeon {
        2.5
    } else {
        2.0
    };
    let mut xp = base * elite;
    if o.pet_victim {
        xp *= 0.75;
    }
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
        pet_victim: c.pet.is_some(),
    }
}

/// Levels at or below which a kill gives nothing.
pub fn gray_level(level: u32) -> u32 {
    match level {
        0..=5 => 0,
        6..=39 => level - 5 - level / 10,
        40..=59 => level - 1 - level / 5,
        _ => level - 9,
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
    let l = level as f32;
    let c = 0.009_110_784 * l * l + 3.225_598 * l + 4.265_291;
    (damage as f32 / c * if taken { 2.5 } else { 7.5 } * 10.0) as u32
}

/// `SMSG_ATTACKERSTATEUPDATE`, encoded by hand: its hit info is a set of
/// flags, which wow_messages reads as one value. `damage` is what got
/// through, after absorbs; `school` its school's index.
#[allow(clippy::too_many_arguments)]
pub fn attacker_state_full(
    attacker: Guid,
    victim: Guid,
    hit_info: u32,
    damage: u32,
    victim_state: u32,
    blocked: u32,
    absorbed: u32,
    school: u32,
) -> Arc<[u8]> {
    let mut b = Body::default();
    b.u32(hit_info);
    crate::game::update::write_packed_guid(&mut b.0, attacker);
    crate::game::update::write_packed_guid(&mut b.0, victim);
    b.u32(damage)
        .u8(1)
        .u32(school)
        .f32(damage as f32)
        .u32(damage)
        .u32(absorbed)
        .u32(0)
        .u32(victim_state)
        .u32(if absorbed > 0 { u32::MAX } else { 0 })
        .u32(0)
        .u32(blocked);
    b.frame(SMSG_ATTACKERSTATEUPDATE)
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
    if a == b {
        return false;
    }
    let (Some(ua), Some(ub)) = (map.unit_view(a), map.unit_view(b)) else {
        return false;
    };
    if !ua.alive || !ub.alive {
        return false;
    }
    if let Some(c) = map.creature(b)
        && (c.evading || c.fields.get_u32(UNIT_FIELD_FLAGS) & 0x0200_0000 != 0)
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
        if ffa(pa) && ffa(pb) {
            return true;
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
    match (regard(map, a, b), regard(map, b, a)) {
        (Regard::Hostile, _) | (_, Regard::Hostile) => true,
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

/// Both units are in combat, now.
pub fn engage(map: &mut Map, a: Guid, b: Guid) {
    let now = map.now;
    for (x, y) in [(a, b), (b, a)] {
        if let Some(c) = map.combat_mut(x) {
            c.last_action = Some(now);
            c.attackers.insert(y);
        }
        if let Some(f) = map.unit_fields_mut(x) {
            f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, true);
        }
    }
    // A blow on a flagged unit flags the attacker; a flagged player's blow
    // flags the one struck. A pet's blows count as its master's.
    let (ma, mb) = (
        crate::game::map::pets::master(map, a),
        crate::game::map::pets::master(map, b),
    );
    pvp_contact(map, ma, mb, Contact::Attack);
    pvp_contact(map, mb, ma, Contact::Attacked);
    if guid::is_unit(b) {
        crate::game::map::ai::attacked(map, b, a);
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
    if map.players.contains_key(&g) {
        return 1 << 6;
    }
    map.creature(g)
        .and_then(|c| map.realm.data.creature_template(c.entry))
        .map(|t| t.creature_type)
        .filter(|t| *t > 0)
        .map_or(0, |t| 1 << (t - 1).min(31))
}

fn flags_extra(map: &Map, g: Guid) -> u32 {
    map.creature(g)
        .and_then(|c| map.realm.data.creature_template(c.entry))
        .map_or(0, |t| t.flags_extra)
}

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

/// The hit table's inputs for `attacker` against `victim` with `hand`.
pub fn roll_for(map: &Map, attacker: Guid, victim: Guid, hand: Hand) -> Option<Roll> {
    let (va, vb) = (map.unit_view(attacker)?, map.unit_view(victim)?);
    let (da, dv) = (derived(map, attacker), derived(map, victim));
    let or_level = |v: u32, level: u32| if v == 0 { level * 5 } else { v } as i32;
    let ranged = hand == Hand::Ranged;
    let held = |g: Guid| {
        map.caster(g).is_some_and(|c| c.casting.is_some())
            || map.unit_fields(g).is_some_and(|f| {
                f.has_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_STUNNED,
                )
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
    Some(Roll {
        attacker_skill: or_level(
            if ranged {
                da.ranged_skill
            } else {
                da.weapon_skill
            },
            va.level,
        ),
        victim_defense: or_level(dv.defense, vb.level),
        attacker_is_player: va.is_player,
        victim_is_player: vb.is_player,
        attacker_level: va.level as i32,
        victim_level: vb.level as i32,
        crit: if ranged { da.ranged_crit } else { da.crit } + attacker_crit,
        hit: if ranged { da.ranged_hit } else { da.hit },
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
    })
}

/// The school of a unit's blows, as a mask: a creature's may be magic.
pub fn melee_school(map: &Map, g: Guid) -> u32 {
    map.creature(g)
        .and_then(|c| map.realm.data.creature_template(c.entry))
        .map_or(1, |t| 1 << t.damage_school.min(6))
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
    let speed = f.get_u32(period).max(100) as f32 / 1000.0;
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
        let weapon = hand_weapon(map, attacker, hand);
        let is_player = map.players.contains_key(&attacker);
        damage += a
            .effects()
            .filter(|(h, x)| {
                x.kind == AURA_MOD_DAMAGE_DONE
                    && (x.misc as u32) & school != 0
                    && (!is_player || crate::game::map::derive::fits(h, weapon.as_ref()))
            })
            .map(|(h, x)| (x.amount * h.stacks.max(1) as i32) as f32)
            .sum::<f32>();
        let kind = creature_type_mask(map, victim);
        if kind != 0 {
            damage += a.sum_mask(AURA_MOD_DAMAGE_DONE_CREATURE, kind) as f32;
            let ap = a.sum_mask(
                if hand == Hand::Ranged {
                    AURA_MOD_RANGED_ATTACK_POWER_VERSUS
                } else {
                    AURA_MOD_MELEE_ATTACK_POWER_VERSUS
                },
                kind,
            );
            damage += ap as f32 / 14.0 * speed;
            damage *= a.product_mask(AURA_MOD_DAMAGE_DONE_VERSUS, kind);
        }
        // An armed creature, disarmed, hits for four tenths.
        if a.has(AURA_MOD_DISARM)
            && let Some(c) = map.creature(attacker)
            && c.fields.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY) != 0
            && hand == Hand::Main
        {
            damage *= 0.4;
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
        damage += v.sum_mask(AURA_MOD_DAMAGE_TAKEN, school) as f32 + v.sum(flat_kind) as f32;
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
    if !va.alive || !can_attack(map, attacker, victim) {
        send_to(map, attacker, encode(&SMSG_ATTACKSWING_CANT_ATTACK {}));
        return;
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
    // The off hand follows a moment after.
    if c.next_offhand.is_none_or(|t| t < now) {
        c.next_offhand = Some(now + Duration::from_millis(200));
    }
    if let Some(f) = map.unit_fields_mut(attacker) {
        f.set_guid(UNIT_FIELD_TARGET, victim);
    }
    if !same && let Some(frame) = encode(&SMSG_ATTACKSTART { attacker, victim }) {
        map.send_to_observers(attacker, &frame, true);
    }
    try_swing(map, attacker);
}

/// `CMSG_ATTACKSTOP`, or a fight that ends.
pub fn stop_attack(map: &mut Map, attacker: Guid) {
    let Some(c) = map.combat_mut(attacker) else {
        return;
    };
    if !c.attacking {
        return;
    }
    c.attacking = false;
    c.queued = None;
    let victim = c.victim;
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
    // A spell being cast, or a stun, holds the swing; a pacified unit
    // doesn't swing at all.
    if map.caster(attacker).is_some_and(|c| c.casting.is_some())
        || auras::held(map, attacker, false)
        || auras::auras(map, attacker).is_some_and(|a| {
            a.has(AURA_MOD_PACIFY) || a.has(AURA_MOD_PACIFY_SILENCE) || a.has(AURA_FEIGN_DEATH)
        })
    {
        return;
    }
    let error = if !va.standing {
        Some(encode(&SMSG_ATTACKSWING_NOTSTANDING {}))
    } else if !in_melee_range(&va, &vb) {
        Some(encode(&SMSG_ATTACKSWING_NOTINRANGE {}))
    } else if va.is_player && !facing(&va, &vb) {
        Some(encode(&SMSG_ATTACKSWING_BADFACING {}))
    } else {
        None
    };
    if let Some(frame) = error {
        let Some(c) = map.combat_mut(attacker) else {
            return;
        };
        if c.swing_error_at
            .is_none_or(|t| now.duration_since(t) >= SWING_ERROR_EVERY)
        {
            c.swing_error_at = Some(now);
            send_to(map, attacker, frame);
        }
        return;
    }
    if extra {
        let owed = map.combat(attacker).map_or(0, |c| c.extra_attacks);
        if let Some(c) = map.combat_mut(attacker) {
            c.extra_attacks = 0;
            c.in_extra_attack = true;
        }
        for _ in 0..owed {
            if !map.unit_view(victim).is_some_and(|u| u.alive) {
                break;
            }
            swing(map, attacker, victim, Hand::Main, true);
        }
        if let Some(c) = map.combat_mut(attacker) {
            c.in_extra_attack = false;
        }
    }
    // A script's melee spell stands in for the blow.
    if main_due && let Some(spell) = crate::game::map::event_ai::melee_spell(map, attacker) {
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
    if main_due && map.unit_view(victim).is_some_and(|u| u.alive) {
        swing(map, attacker, victim, Hand::Main, false);
    }
    if off_due && map.unit_view(victim).is_some_and(|u| u.alive) {
        swing(map, attacker, victim, Hand::Off, false);
    }
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
                Hand::Off => c.next_offhand = next,
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
            engage(map, attacker, victim);
            return;
        }
    }
    let Some(roll) = roll_for(map, attacker, victim, hand) else {
        return;
    };
    let mut rng = rand::rng();
    let outcome = roll_outcome(&roll, rng.random_range(0..10_000));
    let (raw, school) = weapon_damage(map, attacker, victim, hand, &mut rng);
    let mitigated = if school & 1 != 0 {
        armor_mitigate(map, attacker, victim, raw)
    } else {
        raw
    };
    let block_value = derived(map, victim).block_value;
    let caster_class = map
        .players
        .get(&attacker)
        .is_some_and(|p| matches!(p.row.class, 5 | 8 | 9));
    let (mut hit_info, state, damage, blocked) = match outcome {
        Outcome::Miss => (HITINFO_MISS, VICTIMSTATE_UNAFFECTED, 0, 0),
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
        Outcome::Glancing => {
            let gap = roll.victim_defense - roll.attacker_skill;
            let kept = glancing_factor(gap, caster_class, rng.random_range(0.0f32..1.0));
            (
                HITINFO_NORMALSWING2 | HITINFO_GLANCING,
                VICTIMSTATE_NORMAL,
                ((mitigated as f32 * kept) as u32).max(raw.min(1)),
                0,
            )
        }
        Outcome::Crit => (
            HITINFO_NORMALSWING2 | HITINFO_CRITICALHIT,
            VICTIMSTATE_NORMAL,
            (mitigated as f32 * crit_multiplier(map, attacker, victim)) as u32,
            0,
        ),
        Outcome::Crushing => (
            HITINFO_NORMALSWING2 | HITINFO_CRUSHING,
            VICTIMSTATE_NORMAL,
            mitigated * 3 / 2,
            0,
        ),
        Outcome::Hit => (HITINFO_NORMALSWING2, VICTIMSTATE_NORMAL, mitigated, 0),
    };
    // A blow on a unit its school can't touch does nothing.
    let immune = !matches!(outcome, Outcome::Miss | Outcome::Dodge | Outcome::Parry)
        && school_immune(map, victim, school);
    let (state, damage, blocked) = if immune {
        (VICTIMSTATE_IS_IMMUNE, 0, 0)
    } else {
        (state, damage, blocked)
    };
    if hand == Hand::Off {
        hit_info |= HITINFO_LEFTSWING;
    }
    let (left, absorbed) = if damage > 0 {
        auras::absorb(map, victim, damage, school)
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
    skill_gains(map, attacker, victim, hand);
    if left > 0 {
        // A blow on a caster pushes its cast back.
        crate::game::map::spells::pushback(map, victim);
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
/// two fifths of its period, to no less than a fifth.
pub fn parry_haste(map: &mut Map, g: Guid) {
    let now = map.now;
    let dual = dual_wields(map, g);
    let periods = map
        .unit_fields(g)
        .map(|f| {
            (
                f.get_u32(UNIT_FIELD_BASEATTACKTIME),
                f.get_u32(UNIT_FIELD_BASEATTACKTIME + 1),
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

/// A cast of `s` went off (Thistle Tea's `CastingCombat`): one that
/// interrupts the swing (interrupt flag 0x08) starts its caster's swings
/// over, unless it says not to; one that cancels auto attack ends it.
pub fn spell_launched(map: &mut Map, caster: Guid, s: &crate::game::data::spells::SpellTemplate) {
    let now = map.now;
    if s.interrupt_flags & 0x08 != 0 && !s.attr(2, 0x2_0000) {
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
    if s.attr(0, crate::game::data::spells::ATTR_CANCELS_AUTO_ATTACK)
        && map.combat(caster).is_some_and(|c| c.attacking)
    {
        stop_attack(map, caster);
    }
}

/// `UNIT_FIELD_AURASTATE` brought in line: the reactive windows still open,
/// and health below a fifth.
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
    if health > 0 && (health as u64) * 5 < max as u64 {
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
        p.fields.set_guid(PLAYER_FIELD_COMBO_TARGET, Guid::zero());
        p.fields.set_byte(PLAYER_FIELD_BYTES, 1, 0);
    }
    if changed {
        sync_aura_state(map, g);
    }
}

/// Rage for a blow dealt or taken, for a unit whose power is rage.
pub fn gain_rage(map: &mut Map, g: Guid, damage: u32, taken: bool) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if p.fields.get_byte(UNIT_FIELD_BYTES_0, 3) != 1 {
        return;
    }
    let mut gain = rage_for(damage, u32::from(p.level()), taken);
    if taken && p.auras.has_spell(SPELL_BERSERKER_RAGE) {
        gain = gain * 13 / 10;
    }
    let max = p.fields.get_u32(UNIT_FIELD_MAXPOWER1 + 1).max(1000);
    let rage = p.fields.get_u32(UNIT_FIELD_POWER1 + 1);
    p.fields
        .set_u32(UNIT_FIELD_POWER1 + 1, (rage + gain).min(max));
}

/// Raise a player's skill by a point, if below its cap.
fn raise_skill(p: &mut crate::game::map::player::Player, skill: u32) -> bool {
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
        p.fields.set_u32(base + 1, (value + 1) | (max << 16));
        return true;
    }
    false
}

/// Thistle Tea's weapon skill-up chance, in percent.
pub fn weapon_skill_chance(value: u32, cap: u32, intellect: u32) -> f32 {
    let (value, cap) = (value as f32, cap as f32);
    let base = if cap * 0.9 > value {
        (cap * 0.9 * 50.0 / value.max(1.0)).min(100.0)
    } else {
        let chance = (0.5 - 0.016_896_6 * value * (300.0 / cap) + 0.015_206_9 * 300.0) * 100.0;
        let diff = cap - value;
        if diff <= 3.0 {
            chance * (0.5 / (4.0 - diff))
        } else {
            chance
        }
    };
    (base + (0.02 * intellect as f32).min(10.0)).min(100.0)
}

/// Thistle Tea's defense skill-up chance, in percent.
pub fn defense_skill_chance(value: u32, cap: u32, player_level: u32, mob_level: u32) -> f32 {
    let mob = mob_level.min(player_level + 5);
    let diff = mob.saturating_sub(gray_level(player_level)).max(3) as f32;
    (3.0 * diff * cap.saturating_sub(value) as f32 / player_level.max(1) as f32).min(100.0)
}

/// Weapon and defense skills that a blow against a creature trains.
fn skill_gains(map: &mut Map, attacker: Guid, victim: Guid, hand: Hand) {
    let mut rng = rand::rng();
    let trains = |map: &Map, g: Guid| {
        map.creature(g)
            .is_some_and(|c| c.owner.is_none_or(|o| !guid::is_player(o)))
    };
    // The attacker's weapon.
    if map.players.contains_key(&attacker) && trains(map, victim) {
        let mob_level = map.unit_view(victim).map_or(1, |u| u.level);
        let skill = hand_weapon(map, attacker, hand)
            .map_or(162, |w| crate::game::map::derive::weapon_skill(w.subclass));
        let gained = map.players.get_mut(&attacker).is_some_and(|p| {
            let level = u32::from(p.level());
            if mob_level <= gray_level(level) {
                return false;
            }
            let Some(value) = crate::game::map::skills::value(p, skill) else {
                return false;
            };
            let cap = level * 5;
            if value >= cap {
                return false;
            }
            let intellect = p.fields.get_u32(UNIT_FIELD_STAT0 + 3);
            rng.random_range(0.0f32..100.0) < weapon_skill_chance(value, cap, intellect)
                && raise_skill(p, skill)
        });
        if gained {
            auras::recalc(map, attacker);
        }
    }
    // The victim's defense.
    if map.players.contains_key(&victim) && trains(map, attacker) {
        let mob_level = map.unit_view(attacker).map_or(1, |u| u.level);
        let skill = crate::game::map::derive::SKILL_DEFENSE;
        let gained = map.players.get_mut(&victim).is_some_and(|p| {
            let level = u32::from(p.level());
            let Some(value) = crate::game::map::skills::value(p, skill) else {
                return false;
            };
            let cap = level * 5;
            value < cap
                && rng.random_range(0.0f32..100.0)
                    < defense_skill_chance(value, cap, level, mob_level)
                && raise_skill(p, skill)
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
    let level = roll.victim_level as f32;
    let base = if level < 30.0 {
        0.65 * level + 0.5
    } else {
        20.0
    };
    let chance = (base + (roll.attacker_skill - roll.victim_defense) as f32 * 0.2).clamp(0.0, 40.0);
    if rand::rng().random_range(0.0f32..100.0) < chance {
        crate::game::map::spells::cast_triggered(map, attacker, SPELL_DAZED, Some(victim));
    }
}

/// A spell (Windfury, Sword Specialization, Hand of Justice) owes `count`
/// main-hand attacks at once; Reckoning banks one for the next swing. An
/// extra attack can't give more, nor can a batch stack.
pub fn grant_extra_attacks(map: &mut Map, g: Guid, spell: u32, count: u32) {
    let Some(c) = map.combat_mut(g) else {
        return;
    };
    if spell == SPELL_RECKONING {
        if c.extra_attacks + c.held_extra_attacks < RECKONING_CAP {
            c.held_extra_attacks += 1;
        }
        return;
    }
    if c.in_extra_attack || c.extra_attacks > 0 || count == 0 {
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
    let Some(f) = map.unit_fields(other) else {
        return;
    };
    if !f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP) {
        return;
    }
    let other_player = map.players.contains_key(&other);
    let other_fighting = f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT);
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
        Contact::Attacked => other_player,
        Contact::Assist => other_player || other_fighting,
    };
    if flags && let Some(p) = map.players.get_mut(&me) {
        p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, true);
        p.combat.last_pvp = Some(now);
    }
}

/// How a player touched another unit, for [`pvp_contact`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Contact {
    Attack,
    Attacked,
    Assist,
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

/// A heal makes threat for the healer on every creature fighting the
/// healed: half the health it gave back.
pub fn heal_threat(map: &mut Map, healer: Guid, healed: Guid, spell: u32, gained: u32) {
    pvp_contact(map, healer, healed, Contact::Assist);
    if gained == 0 {
        return;
    }
    let school = map
        .realm
        .data
        .spells
        .get(spell)
        .map_or(2, |s| s.school_mask());
    let threat = gained as f32 * 0.5 * threat_multiplier(map, healer, school);
    let fighting: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| {
            c.alive() && c.combat.attacking && c.threat.iter().any(|(t, _)| *t == healed)
        })
        .map(|(_, c)| c.guid)
        .collect();
    for c in fighting {
        if can_attack(map, c, healer) {
            crate::game::map::ai::add_threat(map, c, healer, threat);
        }
    }
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
    // A GM in god mode takes nothing.
    if crate::game::map::gm_dev::invulnerable(map, victim) {
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
    gain_rage(map, victim, damage, true);
    crate::game::map::creature_combat::damaged(map, victim, attacker, damage);
    crate::game::map::vendor::wear_from_blow(map, attacker, victim);
    if guid::is_unit(victim) {
        let threat = damage as f32 * threat_multiplier(map, attacker, school);
        crate::game::map::ai::add_threat(map, victim, attacker, threat);
    }
    // Thistle Tea's `break_on_damage`, whatever the source (a fall too):
    // confusion (Polymorph, Blind) and what damage interrupts (Sap, Gouge).
    auras::remove_where(map, victim, |h| {
        h.interrupt_flags & INTERRUPT_DAMAGE != 0 || (h.has(AURA_MOD_CONFUSE) && !h.positive)
    });
    if attacker != victim {
        crate::game::map::creature_combat::hostile_act(map, victim);
    }
    let Some(f) = map.unit_fields_mut(victim) else {
        return;
    };
    let left = f.get_u32(UNIT_FIELD_HEALTH).saturating_sub(damage);
    // A script can keep a creature from falling below a floor.
    let floor = crate::game::map::event_ai::invincible_floor(map, victim);
    let Some(f) = map.unit_fields_mut(victim) else {
        return;
    };
    let left = if floor > 0 {
        left.max(floor.min(f.get_u32(UNIT_FIELD_HEALTH)))
    } else {
        left
    };
    // A linked feign death keeps it down, not dead.
    if left == 0 && crate::game::map::event_ai::feign_death(map, victim) {
        return;
    }
    // A priest with Spirit of Redemption becomes a spirit instead.
    if left == 0 && crate::game::map::spells::redemption::saves(map, victim) {
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

/// `victim` dies at `killer`'s hand.
pub fn kill(map: &mut Map, killer: Guid, victim: Guid) {
    kill_by(map, killer, victim, None);
}

/// `victim` dies at `killer`'s hand, by `spell` if a spell killed them: one
/// with `no_durability_loss` (ex3 0x20: a Ritual of Doom's sacrifice), or
/// Suicide (27965), wears nothing down (Thistle Tea's
/// `Durability.death_loss?`).
pub fn kill_by(map: &mut Map, killer: Guid, victim: Guid, spell: Option<u32>) {
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
    // A kill a player's on-kill procs count (Thistle Tea's
    // `KillFeedback.eligible?`): not grey, and no pet, totem or creature
    // worth no XP; a creature's kills all count.
    let worth = if guid::is_player(killer) {
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
    } else {
        true
    };
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
        // Dying to a creature or the world wears gear down; to a player or
        // their pet, or in a battleground, it doesn't (Thistle Tea's
        // `Durability.resolve`).
        let battleground = crate::game::instances::instance_type(&map.realm, map.key.map) == 3;
        let spared = spell.is_some_and(|id| {
            id == crate::game::map::spells::redemption::SUICIDE
                || map.realm.data.spells.get(id).is_some_and(|s| {
                    s.attr(3, crate::game::data::spells::ATTR_EX3_NO_DURABILITY_LOSS)
                })
        });
        if !battleground && !spared && (killer == victim || !guid::is_player(by)) {
            crate::game::map::vendor::wear(map, victim, 0.10);
            if let Some(p) = map.players.get(&victim) {
                p.outbound
                    .send(&wow_world_messages::vanilla::SMSG_DURABILITY_DAMAGE_DEATH {});
            }
        }
    } else {
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
        if guid::is_player(tapper) {
            if allowed {
                reward(map, tapper, victim);
            } else {
                crate::game::map::quests::killed(map, tapper, victim);
            }
        }
        crate::game::map::local_defense::fallen(map, victim, killer);
        crate::game::map::ai::die_by(map, victim, tapper, killer);
    }
    if killer != victim {
        procs::trigger(
            map,
            procs::Event {
                actor: killer,
                target: victim,
                actor_flags: if worth { procs::PROC_KILL } else { 0 },
                target_flags: procs::PROC_KILLED,
                ex: procs::EX_NORMAL_HIT,
                ..Default::default()
            },
        );
    }
}

/// How near a corpse a member must be for a share of the kill.
pub const GROUP_REWARD_YARDS: f32 = 74.0;

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
/// spirit no quest credit.
fn reward(map: &mut Map, tapper: Guid, victim: Guid) {
    let Some(at) = map.creature(victim).map(|c| c.position) else {
        return;
    };
    let members: Vec<Guid> = {
        let groups = map.realm.social.groups.lock();
        match groups.group_of(tapper) {
            Some(g) => g.members.iter().map(|m| m.guid).collect(),
            None => vec![tapper],
        }
    };
    let near: Vec<(Guid, u32, Death)> = members
        .into_iter()
        .filter_map(|g| {
            let p = map.players.get(&g)?;
            (dist(p.position(), at) <= GROUP_REWARD_YARDS || g == tapper)
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
            share
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
    let data = map.realm.data.clone();
    let threatened: ahash::AHashSet<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| c.alive() && c.combat.attacking)
        .flat_map(|(_, c)| c.threat.iter().map(|(t, _)| *t))
        .collect();
    for p in map.players.values_mut() {
        if threatened.contains(&p.guid) && p.combat.last_action.is_some() {
            p.combat.last_action = Some(now);
        }
        if !p.combat.attacking
            && p.combat
                .last_action
                .is_some_and(|t| now.duration_since(t) > COMBAT_TIMEOUT)
        {
            p.combat.last_action = None;
            p.combat.attackers.clear();
            p.fields
                .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, false);
            crate::game::map::reputation::end_temporary_war(p, &data);
        }
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

/// The aura states of creatures in a fight: below a fifth of health
/// (Execute), and their reactive windows.
pub fn update_creatures(map: &mut Map) {
    let fighting: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| c.alive() && (c.combat.attacking || !c.combat.attackers.is_empty()))
        .map(|(_, c)| c.guid)
        .collect();
    for g in fighting {
        expire_windows(map, g);
        sync_aura_state(map, g);
    }
    crate::game::map::creature_combat::update(map);
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
    fn skill_ups_follow_thistle_tea() {
        // Far below the cap, a sure gain.
        assert_eq!(weapon_skill_chance(10, 100, 0), 100.0);
        assert!(weapon_skill_chance(99, 100, 0) < 20.0);
        assert!(defense_skill_chance(50, 100, 20, 20) > 0.0);
        assert_eq!(defense_skill_chance(100, 100, 20, 20), 0.0);
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
        );
        assert_eq!(full.len(), frame.len());
    }
}
