//! Melee combat, for players and creatures alike: who may attack whom, the
//! swing timer, vmangos's single-roll hit table, damage and armor, death,
//! and what a kill is worth.

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
use crate::game::map::death::{self, Death};
use crate::game::map::player::{PLAYER_FLAGS_FFA_PVP, UNIT_FLAG_IN_COMBAT, UNIT_FLAG_PVP};

pub const SMSG_ATTACKERSTATEUPDATE: u16 = 0x14A;

/// `SMSG_ATTACKERSTATEUPDATE` hit flags.
pub const HITINFO_NORMALSWING2: u32 = 0x02;
pub const HITINFO_MISS: u32 = 0x10;
pub const HITINFO_CRITICALHIT: u32 = 0x80;
pub const HITINFO_GLANCING: u32 = 0x4000;
pub const HITINFO_CRUSHING: u32 = 0x8000;

/// What the victim did.
pub const VICTIMSTATE_UNAFFECTED: u32 = 0;
pub const VICTIMSTATE_NORMAL: u32 = 1;
pub const VICTIMSTATE_DODGE: u32 = 2;
pub const VICTIMSTATE_PARRY: u32 = 3;
pub const VICTIMSTATE_BLOCKS: u32 = 5;

/// A player leaves combat this long after their last blow given or taken.
pub const COMBAT_TIMEOUT: Duration = Duration::from_secs(6);
/// A PvP flag lasts this long after the last PvP blow.
pub const PVP_TIMEOUT: Duration = Duration::from_secs(300);
/// An error for a swing that can't land is sent at most this often.
const SWING_ERROR_EVERY: Duration = Duration::from_millis(1000);
/// The attacker faces the victim within this arc.
const FACING_ARC: f32 = 2.0 * std::f32::consts::PI / 3.0;

/// A unit's fight: whom it swings at, and when the next swing is due.
#[derive(Debug, Clone, Default)]
pub struct Combat {
    pub victim: Guid,
    pub attacking: bool,
    pub next_swing: Option<Instant>,
    pub last_action: Option<Instant>,
    pub last_pvp: Option<Instant>,
    pub swing_error_at: Option<Instant>,
    /// Units fighting this one.
    pub attackers: AHashSet<Guid>,
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
#[derive(Debug, Clone, Copy)]
pub struct Roll {
    pub attacker_skill: i32,
    pub victim_defense: i32,
    pub attacker_is_player: bool,
    pub victim_is_player: bool,
    pub attacker_level: i32,
    pub victim_level: i32,
    /// In percent.
    pub crit: f32,
    pub dodge: f32,
    pub parry: f32,
    pub block: f32,
    pub from_behind: bool,
}

/// vmangos's single roll (`RollMeleeOutcomeAgainst`), `roll` in hundredths
/// of a percent.
pub fn roll_outcome(r: &Roll, roll: i32) -> Outcome {
    let skill_diff = r.attacker_skill - r.victim_defense;
    let mut sum = 0;
    // Miss: 5%, more against a higher defense.
    let mut miss = 500 - skill_diff * if skill_diff < -10 { 20 } else { 4 };
    if r.attacker_is_player && !r.victim_is_player && skill_diff <= 0 {
        miss += 100;
    }
    sum += miss.clamp(0, 6000);
    if roll < sum {
        return Outcome::Miss;
    }
    sum += ((r.dodge * 100.0) as i32 - skill_diff * 4).max(0);
    if roll < sum {
        return Outcome::Dodge;
    }
    if !r.from_behind {
        sum += ((r.parry * 100.0) as i32 - skill_diff * 4).max(0);
        if roll < sum {
            return Outcome::Parry;
        }
    }
    if r.attacker_is_player && !r.victim_is_player && r.victim_level >= r.attacker_level {
        sum += (1000 + (r.victim_defense - r.attacker_skill) * 200).clamp(0, 4000);
        if roll < sum {
            return Outcome::Glancing;
        }
    }
    if !r.from_behind {
        sum += ((r.block * 100.0) as i32 - skill_diff * 4).max(0);
        if roll < sum {
            return Outcome::Block;
        }
    }
    sum += ((r.crit * 100.0) as i32 + skill_diff * 4).max(0);
    if roll < sum {
        return Outcome::Crit;
    }
    if !r.attacker_is_player && r.attacker_level >= r.victim_level + 4 {
        sum += (skill_diff * 200 - 1500).max(0);
        if roll < sum {
            return Outcome::Crushing;
        }
    }
    Outcome::Hit
}

/// Armor's share of physical damage stopped, up to 75%.
pub fn armor_reduction(armor: u32, attacker_level: u32) -> f32 {
    let armor = armor as f32;
    (armor / (armor + 400.0 + 85.0 * attacker_level as f32)).clamp(0.0, 0.75)
}

/// XP for a kill (vmangos's `MaNGOS::XP::BaseGain`, vanilla content).
pub fn kill_xp(player_level: u32, mob_level: u32, elite: bool, multiplier: f32) -> u32 {
    let base = if mob_level >= player_level {
        let diff = (mob_level - player_level).min(4);
        ((player_level * 5 + 45) * (20 + diff) / 10).div_ceil(2)
    } else if mob_level <= gray_level(player_level) {
        0
    } else {
        let zd = zero_difference(player_level);
        (player_level * 5 + 45) * (zd + mob_level - player_level) / zd
    };
    let m = if multiplier > 0.0 { multiplier } else { 1.0 };
    (base as f32 * m * if elite { 2.0 } else { 1.0 }).round() as u32
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
/// flags, which wow_messages reads as one value.
pub fn attacker_state(
    attacker: Guid,
    victim: Guid,
    hit_info: u32,
    damage: u32,
    victim_state: u32,
    blocked: u32,
) -> Arc<[u8]> {
    let mut b = Body::default();
    b.u32(hit_info);
    crate::game::update::write_packed_guid(&mut b.0, attacker);
    crate::game::update::write_packed_guid(&mut b.0, victim);
    b.u32(damage)
        .u8(1)
        .u32(1)
        .f32(damage as f32)
        .u32(damage)
        .u32(0)
        .u32(0)
        .u32(victim_state)
        .u32(0)
        .u32(0)
        .u32(blocked);
    b.frame(SMSG_ATTACKERSTATEUPDATE)
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
    if ua.is_player && ub.is_player {
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
        return pa.team != pb.team
            && (pb.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP)
                || pa.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP));
    }
    let data = &map.realm.data;
    match (
        data.faction_templates.get(&ua.faction),
        data.faction_templates.get(&ub.faction),
    ) {
        (Some(fa), Some(fb)) => !fa.friendly_to(fb),
        // Without faction data, a player may attack any creature.
        _ => true,
    }
}

/// Whether `a` is hostile to `b`, and so attacks it on sight.
pub fn hostile(map: &Map, a: Guid, b: Guid) -> bool {
    let (Some(ua), Some(ub)) = (map.unit_view(a), map.unit_view(b)) else {
        return false;
    };
    let data = &map.realm.data;
    match (
        data.faction_templates.get(&ua.faction),
        data.faction_templates.get(&ub.faction),
    ) {
        (Some(fa), Some(fb)) => fa.hostile_to(fb),
        _ => false,
    }
}

pub fn dist(a: Vector3d, b: Vector3d) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

/// The distance two units may swing at each other from: both reaches and
/// four thirds of a yard, at least five.
pub fn melee_range(a: &UnitView, b: &UnitView) -> f32 {
    (a.combat_reach + b.combat_reach + 4.0 / 3.0).max(5.0)
}

pub fn in_melee_range(a: &UnitView, b: &UnitView) -> bool {
    dist(a.position, b.position) <= melee_range(a, b) + 1.0
}

fn facing(a: &UnitView, b: &UnitView) -> bool {
    let angle = (b.position.y - a.position.y).atan2(b.position.x - a.position.x);
    let mut diff = (angle - a.orientation).rem_euclid(std::f32::consts::TAU);
    if diff > std::f32::consts::PI {
        diff = std::f32::consts::TAU - diff;
    }
    diff <= FACING_ARC / 2.0
}

/// Both units are in combat, now.
fn engage(map: &mut Map, a: Guid, b: Guid) {
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
    // A blow between the factions flags the attacker for PvP.
    if let (Some(pa), Some(pb)) = (map.players.get(&a), map.players.get(&b))
        && pa.team != pb.team
        && let Some(p) = map.players.get_mut(&a)
    {
        p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, true);
        p.combat.last_pvp = Some(now);
    }
    if guid::is_unit(b) {
        crate::game::map::ai::attacked(map, b, a);
    }
}

fn send_to(map: &Map, g: Guid, frame: Option<Arc<[u8]>>) {
    if let (Some(p), Some(f)) = (map.players.get(&g), frame) {
        p.outbound.send_frame(f);
    }
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
    let victim = c.victim;
    if let Some(frame) = encode(&SMSG_ATTACKSTOP {
        player: attacker,
        enemy: victim,
        unknown1: 0,
    }) {
        map.send_to_observers(attacker, &frame, true);
    }
}

/// Swing if the timer is due and the swing can land; an error, now and
/// then, if it can't.
pub fn try_swing(map: &mut Map, attacker: Guid) {
    let now = map.now;
    let Some(c) = map.combat(attacker) else {
        return;
    };
    if !c.attacking || c.next_swing.is_some_and(|t| t > now) {
        return;
    }
    let victim = c.victim;
    let (Some(va), Some(vb)) = (map.unit_view(attacker), map.unit_view(victim)) else {
        stop_attack(map, attacker);
        return;
    };
    if !vb.alive || !va.alive {
        stop_attack(map, attacker);
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
    swing(map, attacker, victim);
}

/// One melee swing, landed or not.
fn swing(map: &mut Map, attacker: Guid, victim: Guid) {
    let now = map.now;
    let (Some(va), Some(vb)) = (map.unit_view(attacker), map.unit_view(victim)) else {
        return;
    };
    let attack_time = map
        .unit_fields_mut(attacker)
        .map(|f| f.get_u32(UNIT_FIELD_BASEATTACKTIME))
        .unwrap_or(2000)
        .max(500);
    if let Some(c) = map.combat_mut(attacker) {
        c.next_swing = Some(now + Duration::from_millis(u64::from(attack_time)));
    }
    let mut rng = rand::rng();
    let roll = Roll {
        attacker_skill: (va.level * 5) as i32,
        victim_defense: (vb.level * 5) as i32,
        attacker_is_player: va.is_player,
        victim_is_player: vb.is_player,
        attacker_level: va.level as i32,
        victim_level: vb.level as i32,
        crit: crit_chance(map, attacker),
        dodge: dodge_chance(map, victim),
        parry: if parries(map, victim) { 5.0 } else { 0.0 },
        block: 0.0,
        from_behind: !facing(&vb, &va),
    };
    let outcome = roll_outcome(&roll, rng.random_range(0..10_000));
    let (min, max) = damage_range(map, attacker);
    let rolled = if max > min {
        rng.random_range(min..=max)
    } else {
        min
    };
    let armor = map
        .unit_fields_mut(victim)
        .map(|f| f.get_u32(UNIT_FIELD_RESISTANCES))
        .unwrap_or(0);
    let mitigated = (rolled as f32 * (1.0 - armor_reduction(armor, va.level))) as u32;
    let (hit_info, state, damage) = match outcome {
        Outcome::Miss => (HITINFO_MISS, VICTIMSTATE_UNAFFECTED, 0),
        Outcome::Dodge => (HITINFO_NORMALSWING2, VICTIMSTATE_DODGE, 0),
        Outcome::Parry => (HITINFO_NORMALSWING2, VICTIMSTATE_PARRY, 0),
        Outcome::Block => (HITINFO_NORMALSWING2, VICTIMSTATE_BLOCKS, mitigated / 2),
        Outcome::Glancing => (
            HITINFO_NORMALSWING2 | HITINFO_GLANCING,
            VICTIMSTATE_NORMAL,
            (mitigated as f32 * rng.random_range(0.65f32..0.95)) as u32,
        ),
        Outcome::Crit => (
            HITINFO_NORMALSWING2 | HITINFO_CRITICALHIT,
            VICTIMSTATE_NORMAL,
            mitigated * 2,
        ),
        Outcome::Crushing => (
            HITINFO_NORMALSWING2 | HITINFO_CRUSHING,
            VICTIMSTATE_NORMAL,
            mitigated * 3 / 2,
        ),
        Outcome::Hit => (HITINFO_NORMALSWING2, VICTIMSTATE_NORMAL, mitigated),
    };
    let damage = match outcome {
        Outcome::Miss | Outcome::Dodge | Outcome::Parry => 0,
        _ => damage.max(1),
    };
    let frame = attacker_state(attacker, victim, hit_info, damage, state, 0);
    map.send_to_observers(attacker, &frame, true);
    // A creature's victim hears of the blow even if it doesn't see it yet.
    if !va.is_player
        && let Some(p) = map.players.get(&victim)
        && !p.known.contains(&attacker)
    {
        p.outbound.send_frame(frame);
    }
    engage(map, attacker, victim);
    if damage > 0 {
        deal_damage(map, attacker, victim, damage);
    }
}

fn crit_chance(map: &Map, g: Guid) -> f32 {
    match map.players.get(&g) {
        Some(p) => 5.0 + p.fields.get_u32(UNIT_FIELD_STAT0 + 1) as f32 / 20.0,
        None => 5.0,
    }
}

fn dodge_chance(map: &Map, g: Guid) -> f32 {
    match map.players.get(&g) {
        Some(p) => p.fields.get_u32(UNIT_FIELD_STAT0 + 1) as f32 / 20.0,
        None => 5.0,
    }
}

fn parries(map: &Map, g: Guid) -> bool {
    match map.players.get(&g) {
        // Warriors, paladins, hunters and rogues learn parry.
        Some(p) => matches!(p.row.class, 1..=4),
        None => true,
    }
}

/// A swing's weapon damage, before armor.
fn damage_range(map: &Map, g: Guid) -> (u32, u32) {
    let f = if let Some(p) = map.players.get(&g) {
        &p.fields
    } else if let Some(c) = map.creature(g) {
        &c.fields
    } else {
        return (1, 2);
    };
    let min = f.get_f32(UNIT_FIELD_MINDAMAGE).max(1.0);
    let max = f.get_f32(UNIT_FIELD_MAXDAMAGE).max(min);
    (min as u32, max as u32)
}

/// Damage a unit: rage, threat, health, death.
pub fn deal_damage(map: &mut Map, attacker: Guid, victim: Guid, damage: u32) {
    let damage = if guid::is_player(victim) && guid::is_player(attacker) {
        crate::game::map::duel::soften(map, attacker, victim, damage)
    } else {
        damage
    };
    if damage == 0 {
        return;
    }
    for (g, taken) in [(attacker, false), (victim, true)] {
        if let Some(p) = map.players.get_mut(&g)
            && p.power_type() == 1
        {
            let gain = rage_for(damage, u32::from(p.level()), taken);
            let rage = p.fields.get_u32(UNIT_FIELD_POWER1 + 1);
            p.fields
                .set_u32(UNIT_FIELD_POWER1 + 1, (rage + gain).min(1000));
        }
    }
    if guid::is_unit(victim) {
        crate::game::map::ai::add_threat(map, victim, attacker, damage as f32);
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
    f.set_u32(UNIT_FIELD_HEALTH, left);
    if left == 0 {
        kill(map, attacker, victim);
    }
}

/// `victim` dies at `killer`'s hand.
pub fn kill(map: &mut Map, killer: Guid, victim: Guid) {
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
        }
    }
    stop_attack(map, victim);
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
        honor(map, killer, victim);
        crate::game::map::battleground::died(map, victim, killer);
        death::kill(map, victim);
        // Dying to a creature wears gear down; to a player, it doesn't.
        if guid::is_unit(killer) {
            crate::game::map::vendor::wear(map, victim, 0.10);
        }
    } else {
        let tapper = map
            .creature(victim)
            .and_then(|c| c.tapped_by)
            .unwrap_or(killer);
        if guid::is_player(tapper) {
            reward(map, tapper, victim);
        }
        crate::game::map::ai::die(map, victim, tapper);
    }
}

/// How near a corpse a member must be for a share of the kill.
pub const GROUP_REWARD_YARDS: f32 = 74.0;

/// vmangos's group XP bonus: three, four and five share more.
pub fn group_xp_rate(count: usize, raid: bool) -> f32 {
    if raid {
        return 1.0;
    }
    match count {
        0..=2 => 1.0,
        3 => 1.166,
        4 => 1.3,
        _ => 1.4,
    }
}

/// XP and kill credit for a creature's death: the tapper's, or shared with
/// their group near the corpse, as vmangos shares them.
fn reward(map: &mut Map, tapper: Guid, victim: Guid) {
    let Some(at) = map.creature(victim).map(|c| c.position) else {
        return;
    };
    let (members, raid) = {
        let groups = map.realm.social.groups.lock();
        match groups.group_of(tapper) {
            Some(g) => (g.members.iter().map(|m| m.guid).collect::<Vec<_>>(), g.raid),
            None => (vec![tapper], false),
        }
    };
    let near: Vec<(Guid, u32, bool)> = members
        .into_iter()
        .filter_map(|g| {
            let p = map.players.get(&g)?;
            (dist(p.position(), at) <= GROUP_REWARD_YARDS || g == tapper)
                .then(|| (g, u32::from(p.level()), p.death == Death::Alive))
        })
        .collect();
    if near.len() <= 1 {
        kill_reward(map, tapper, victim);
        crate::game::map::quests::killed(map, tapper, victim);
        return;
    }
    let template = map
        .creature(victim)
        .and_then(|c| map.realm.data.creature_template(c.entry));
    let no_xp = template.is_some_and(|t| t.static_flags1 & 0x2 != 0);
    let elite = template.is_some_and(|t| t.rank == 1 || t.rank == 2);
    let mult = template.map(|t| t.xp_multiplier).unwrap_or(1.0);
    let mob_level = map
        .creature(victim)
        .map(|c| c.fields.get_u32(UNIT_FIELD_LEVEL))
        .unwrap_or(1);
    let alive: Vec<&(Guid, u32, bool)> = near.iter().filter(|m| m.2).collect();
    let sum_level: u32 = alive.iter().map(|m| m.1).sum();
    let top = alive.iter().map(|m| m.1).max().unwrap_or(0);
    let top_not_gray = alive
        .iter()
        .map(|m| m.1)
        .filter(|l| mob_level > gray_level(*l))
        .max();
    if let Some(level) = top_not_gray.filter(|_| !no_xp && sum_level > 0) {
        let xp = kill_xp(level, mob_level, elite, mult) as f32;
        let rate = group_xp_rate(near.len(), raid);
        for m in alive.iter().filter(|m| m.1 <= level) {
            let share = if top == level {
                xp * rate * m.1 as f32 / sum_level as f32
            } else {
                xp * rate / 2.0 + 1.0
            };
            crate::game::map::stats::give_xp(map, m.0, share as u32, Some(victim));
        }
    }
    // Quest credit for the living, and the dead not yet released.
    for (g, _, _) in near {
        if map
            .players
            .get(&g)
            .is_some_and(|p| matches!(p.death, Death::Alive | Death::Dead { .. }))
        {
            crate::game::map::quests::killed(map, g, victim);
        }
    }
}

/// XP for a creature's death.
fn kill_reward(map: &mut Map, player: Guid, victim: Guid) {
    let Some(c) = map.creature(victim) else {
        return;
    };
    let template = map.realm.data.creature_template(c.entry);
    // No XP for a creature flagged so.
    if template.is_some_and(|t| t.static_flags1 & 0x2 != 0) {
        return;
    }
    let elite = template.is_some_and(|t| t.rank == 1 || t.rank == 2);
    let mult = template.map(|t| t.xp_multiplier).unwrap_or(1.0);
    let mob_level = c.fields.get_u32(UNIT_FIELD_LEVEL);
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let xp = kill_xp(u32::from(p.level()), mob_level, elite, mult);
    crate::game::map::stats::give_xp(map, player, xp, Some(victim));
}

/// An honorable kill: an enemy player not too far below the killer.
fn honor(map: &mut Map, killer: Guid, victim: Guid) {
    let (Some(k), Some(v)) = (map.players.get(&killer), map.players.get(&victim)) else {
        return;
    };
    if k.team == v.team || u32::from(v.level()) <= gray_level(u32::from(k.level())) {
        return;
    }
    let victim_level = v.level();
    let Some(k) = map.players.get_mut(&killer) else {
        return;
    };
    let (today, yesterday) = {
        let v = k.fields.get_u32(PLAYER_FIELD_SESSION_KILLS);
        (v & 0xFFFF, v >> 16)
    };
    k.fields.set_shorts(
        PLAYER_FIELD_SESSION_KILLS,
        (today + 1) as u16,
        yesterday as u16,
    );
    let life = k.fields.get_u32(PLAYER_FIELD_LIFETIME_HONORABLE_KILLS);
    k.fields
        .set_u32(PLAYER_FIELD_LIFETIME_HONORABLE_KILLS, life + 1);
    k.outbound
        .send(&wow_world_messages::vanilla::SMSG_PVP_CREDIT {
            honor_points: u32::from(victim_level),
            victim,
            rank: wow_world_messages::vanilla::PvpRank::NoRank,
        });
}

/// Run the players' swings, and expire their combat and PvP flags.
pub fn update_players(map: &mut Map) {
    let now = map.now;
    let swinging: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.combat.attacking)
        .map(|p| p.guid)
        .collect();
    for g in swinging {
        try_swing(map, g);
    }
    for p in map.players.values_mut() {
        if !p.combat.attacking
            && p.combat
                .last_action
                .is_some_and(|t| now.duration_since(t) > COMBAT_TIMEOUT)
        {
            p.combat.last_action = None;
            p.combat.attackers.clear();
            p.fields
                .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, false);
        }
        if p.combat
            .last_pvp
            .is_some_and(|t| now.duration_since(t) > PVP_TIMEOUT)
        {
            p.combat.last_pvp = None;
            p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, false);
        }
    }
}

/// A unit stops fighting: out of combat, attacking nothing.
pub fn leave_combat(map: &mut Map, g: Guid) {
    stop_attack(map, g);
    if let Some(c) = map.combat_mut(g) {
        c.attackers.clear();
        c.last_action = None;
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
            block: 0.0,
            from_behind: false,
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
        let behind = Roll {
            from_behind: true,
            ..r
        };
        assert_eq!(roll_outcome(&behind, 1000), Outcome::Crit);
    }

    #[test]
    fn players_glance_off_creatures_of_their_level() {
        let r = Roll {
            victim_is_player: false,
            ..even(10)
        };
        // Miss 6%, dodge 5, parry 5, then glancing.
        assert_eq!(roll_outcome(&r, 1700), Outcome::Glancing);
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
    }
}
