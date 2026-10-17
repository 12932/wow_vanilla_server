//! Spells: learning them, casting them (checks, cast time, power,
//! reagents, cooldowns and the global cooldown, interrupts), and what their
//! effects do.

use std::time::{Duration, Instant};

use ahash::AHashMap;
use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::items::Body;
use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras;
use crate::game::map::combat::{self, dist};

pub const SMSG_CAST_RESULT: u16 = 0x130;
/// `SPELL_FAILED_REAGENTS`, which wow_messages lacks.
pub const SPELL_FAILED_REAGENTS: u8 = 0x5c;

// Effects.
pub const EFFECT_INSTAKILL: u32 = 1;
pub const EFFECT_SCHOOL_DAMAGE: u32 = 2;
pub const EFFECT_DUMMY: u32 = 3;
pub const EFFECT_TELEPORT_UNITS: u32 = 5;
pub const EFFECT_APPLY_AURA: u32 = 6;
pub const EFFECT_POWER_DRAIN: u32 = 8;
pub const EFFECT_HEALTH_LEECH: u32 = 9;
pub const EFFECT_HEAL: u32 = 10;
pub const EFFECT_WEAPON_DAMAGE_NOSCHOOL: u32 = 17;
pub const EFFECT_RESURRECT: u32 = 18;
pub const EFFECT_CREATE_ITEM: u32 = 24;
pub const EFFECT_PERSISTENT_AREA_AURA: u32 = 27;
pub const EFFECT_LEAP: u32 = 29;
pub const EFFECT_ENERGIZE: u32 = 30;
pub const EFFECT_WEAPON_PERCENT_DAMAGE: u32 = 31;
pub const EFFECT_OPEN_LOCK: u32 = 33;
pub const EFFECT_APPLY_AREA_AURA_PARTY: u32 = 35;
pub const EFFECT_LEARN_SPELL: u32 = 36;
pub const EFFECT_DISPEL: u32 = 38;
pub const EFFECT_WEAPON_DAMAGE: u32 = 58;
pub const EFFECT_THREAT: u32 = 63;
pub const EFFECT_TRIGGER_SPELL: u32 = 64;
pub const EFFECT_HEAL_MAX_HEALTH: u32 = 67;
pub const EFFECT_INTERRUPT_CAST: u32 = 68;
pub const EFFECT_ADD_COMBO_POINTS: u32 = 80;
pub const EFFECT_DUEL: u32 = 83;
pub const EFFECT_ADD_EXTRA_ATTACKS: u32 = 19;
pub const EFFECT_CHARGE: u32 = 96;
pub const EFFECT_RESURRECT_NEW: u32 = 113;
pub const EFFECT_SKILL: u32 = 118;
pub const EFFECT_NORMALIZED_WEAPON_DMG: u32 = 121;

/// Every effect the server knows: those with an effect here, and the
/// passive and bookkeeping ones that need none (proficiencies, skills,
/// languages, dual wield, scripts and visuals it doesn't run yet).
pub fn effect_supported(effect: u32) -> bool {
    const KEPT: &[u32] = &[
        0, 4, 7, 11, 12, 13, 14, 15, 16, 19, 20, 21, 22, 23, 25, 26, 28, 32, 34, 37, 39, 40, 41,
        42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 59, 60, 61, 62, 65, 66, 69,
        70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93,
        94, 95, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 114,
        115, 116, 117, 119, 120, 122, 123, 124, 125, 126, 127, 128, 129,
    ];
    matches!(
        effect,
        EFFECT_INSTAKILL
            | EFFECT_SCHOOL_DAMAGE
            | EFFECT_DUMMY
            | EFFECT_TELEPORT_UNITS
            | EFFECT_APPLY_AURA
            | EFFECT_POWER_DRAIN
            | EFFECT_HEALTH_LEECH
            | EFFECT_HEAL
            | EFFECT_WEAPON_DAMAGE_NOSCHOOL
            | EFFECT_RESURRECT
            | EFFECT_CREATE_ITEM
            | EFFECT_PERSISTENT_AREA_AURA
            | EFFECT_LEAP
            | EFFECT_ENERGIZE
            | EFFECT_WEAPON_PERCENT_DAMAGE
            | EFFECT_OPEN_LOCK
            | EFFECT_APPLY_AREA_AURA_PARTY
            | EFFECT_LEARN_SPELL
            | EFFECT_DISPEL
            | EFFECT_WEAPON_DAMAGE
            | EFFECT_THREAT
            | EFFECT_TRIGGER_SPELL
            | EFFECT_HEAL_MAX_HEALTH
            | EFFECT_INTERRUPT_CAST
            | EFFECT_ADD_COMBO_POINTS
            | EFFECT_DUEL
            | EFFECT_CHARGE
            | EFFECT_RESURRECT_NEW
            | EFFECT_SKILL
            | EFFECT_NORMALIZED_WEAPON_DMG
    ) || KEPT.contains(&effect)
}

/// Implicit targets the server resolves.
pub fn target_supported(t: u32) -> bool {
    matches!(
        t,
        0 | 1
            | 2
            | 5
            | 6
            | 7
            | 8
            | 15
            | 16
            | 17
            | 18
            | 20
            | 21
            | 22
            | 23
            | 24
            | 25
            | 26
            | 27
            | 28
            | 30
            | 31
            | 32
            | 33
            | 34
            | 35
            | 36
            | 37
            | 38
            | 39
            | 40
            | 41
            | 42
            | 43
            | 44
            | 45
            | 46
            | 47
            | 53
            | 55
            | 56
            | 57
            | 60
            | 61
            | 63
    )
}

/// What a cast is aimed at.
#[derive(Debug, Clone, Copy, Default)]
pub struct Targets {
    pub unit: Option<Guid>,
    pub object: Option<Guid>,
    pub item: Option<Guid>,
    pub dest: Option<Vector3d>,
}

/// A cast under way.
#[derive(Debug, Clone)]
pub struct Casting {
    pub spell: u32,
    pub targets: Targets,
    pub ends: Instant,
    pub item: Option<Guid>,
    pub pushbacks: u8,
    pub channel_until: Option<Instant>,
}

/// A unit's spellcasting: the cast under way and the cooldowns.
#[derive(Debug, Clone, Default)]
pub struct Caster {
    pub casting: Option<Casting>,
    pub cooldowns: AHashMap<u32, Instant>,
    pub category_cooldowns: AHashMap<u32, Instant>,
    pub gcd_until: Option<Instant>,
    /// For a creature: when it next tries a spell from its list.
    pub next_spell_at: Option<Instant>,
    pub combo_target: Guid,
    pub combo_points: u8,
}

/// The spell learning `spell` gives: a trainer's spell may be one whose
/// effect teaches another.
pub fn learns(data: &GameData, spell: u32) -> u32 {
    data.spells
        .get(spell)
        .and_then(|s| {
            s.effects
                .iter()
                .find(|e| e.effect == EFFECT_LEARN_SPELL && e.trigger_spell != 0)
                .map(|e| e.trigger_spell)
        })
        .unwrap_or(spell)
}

/// `player` learns `spell`, and its passive auras take hold.
pub fn learn(map: &mut Map, player: Guid, spell: u32) {
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    if spell == 0 || p.spells.contains(&spell) {
        return;
    }
    p.spells.push(spell);
    p.outbound.send(&SMSG_LEARNED_SPELL { id: spell });
    apply_passive(map, player, spell);
}

/// Every passive spell a player knows takes hold, as on login.
pub fn apply_passives(map: &mut Map, player: Guid) {
    let spells = map
        .players
        .get(&player)
        .map(|p| p.spells.clone())
        .unwrap_or_default();
    for spell in spells {
        apply_passive(map, player, spell);
    }
}

/// A passive spell's effects on its owner.
pub fn apply_passive(map: &mut Map, player: Guid, spell: u32) {
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(spell) else {
        return;
    };
    if !s.passive() {
        return;
    }
    for (i, e) in s.effects.iter().enumerate() {
        if e.effect == EFFECT_APPLY_AURA && e.aura != 0 {
            let amount = points(s, i, 1);
            auras::apply(map, player, player, s, i, amount);
        }
        if e.effect == EFFECT_SKILL
            && let Some(p) = map.players.get_mut(&player)
        {
            crate::game::map::skills::learn_skill(p, e.misc_value.max(0) as u32, 1);
        }
    }
}

/// An effect's points at `level`: base, a die roll, and the per-level gain.
pub fn points(s: &SpellTemplate, index: usize, level: u32) -> i32 {
    let e = &s.effects[index];
    let level = level.min(if s.max_level > 0 {
        s.max_level
    } else {
        u32::MAX
    });
    let per_level = (e.real_points_per_level * level.saturating_sub(s.spell_level) as f32) as i32;
    let low = e.base_dice.max(1);
    let dice = if e.die_sides > low {
        rand::rng().random_range(low..=e.die_sides)
    } else {
        low
    };
    e.base_points + dice + per_level
}

fn send_cast_result(map: &Map, caster: Guid, spell: u32, result: Option<u8>) {
    let Some(p) = map.players.get(&caster) else {
        return;
    };
    let mut b = Body::default();
    b.u32(spell);
    match result {
        None => {
            b.u8(0);
        }
        Some(r) => {
            b.u8(2).u8(r);
        }
    }
    p.outbound.send_frame(b.frame(SMSG_CAST_RESULT));
}

fn power_index(s: &SpellTemplate) -> u16 {
    s.power_type.min(4) as u16
}

fn power_cost(map: &Map, caster: Guid, s: &SpellTemplate) -> u32 {
    let level = map.unit_view(caster).map(|u| u.level).unwrap_or(1);
    if s.mana_cost_percentage > 0 {
        let base_mana = map
            .players
            .get(&caster)
            .map(|p| p.fields.get_u32(UNIT_FIELD_BASE_MANA))
            .unwrap_or(0);
        return base_mana * s.mana_cost_percentage / 100;
    }
    s.mana_cost + s.mana_cost_per_level * level.saturating_sub(s.base_level)
}

fn targets_of(t: &SpellCastTargets) -> Targets {
    let f = &t.target_flags;
    Targets {
        unit: f.get_unit().map(|u| u.unit_target),
        object: f.get_gameobject().map(|g| match g {
            SpellCastTargets_SpellCastTargetFlags_Gameobject::Gameobject { gameobject }
            | SpellCastTargets_SpellCastTargetFlags_Gameobject::ObjectUnk {
                object_unk: gameobject,
            } => *gameobject,
        }),
        item: f.get_item().map(|i| match i {
            SpellCastTargets_SpellCastTargetFlags_Item::Item { item }
            | SpellCastTargets_SpellCastTargetFlags_Item::TradeItem { trade_item: item } => *item,
        }),
        dest: f.get_dest_location().map(|d| d.destination),
    }
}

pub fn wire_targets(t: &Targets) -> SpellCastTargets {
    let mut flags = SpellCastTargets_SpellCastTargetFlags::empty();
    if let Some(u) = t.unit {
        flags = flags.set_unit(SpellCastTargets_SpellCastTargetFlags_Unit { unit_target: u });
    }
    if let Some(d) = t.dest {
        flags = flags.set_dest_location(SpellCastTargets_SpellCastTargetFlags_DestLocation {
            destination: d,
        });
    }
    SpellCastTargets {
        target_flags: flags,
    }
}

/// Why `caster` can't cast `s` at `targets` now, if it can't.
fn check(
    map: &Map,
    caster: Guid,
    s: &SpellTemplate,
    t: &Targets,
    item: Option<Guid>,
) -> Option<u8> {
    let now = map.now;
    let data = &map.realm.data;
    let Some(me) = map.unit_view(caster) else {
        return Some(SpellCastResult::ErrorX.as_int());
    };
    if !me.alive {
        return Some(SpellCastResult::CasterDead.as_int());
    }
    if let Some(c) = map.caster(caster) {
        if c.casting.is_some() {
            return Some(SpellCastResult::SpellInProgress.as_int());
        }
        if c.cooldowns.get(&s.id).is_some_and(|t| *t > now)
            || (s.category != 0
                && c.category_cooldowns
                    .get(&s.category)
                    .is_some_and(|t| *t > now))
        {
            return Some(SpellCastResult::NotReady.as_int());
        }
        if s.start_recovery_category != 0 && c.gcd_until.is_some_and(|t| t > now) {
            return Some(SpellCastResult::NotReady.as_int());
        }
    }
    if let Some(p) = map.players.get(&caster) {
        if item.is_none() && !p.spells.contains(&s.id) {
            return Some(SpellCastResult::NotKnown.as_int());
        }
        for (reagent, count) in s.reagents.iter().filter(|(r, _)| *r != 0) {
            if p.count_of(*reagent) < *count {
                return Some(SPELL_FAILED_REAGENTS);
            }
        }
    }
    if auras::held(map, caster, true) {
        return Some(SpellCastResult::Stunned.as_int());
    }
    let cost = power_cost(map, caster, s);
    if cost > 0 {
        let have = map
            .unit_fields(caster)
            .map(|f| f.get_u32(UNIT_FIELD_POWER1 + power_index(s)))
            .unwrap_or(0);
        if have < cost {
            return Some(SpellCastResult::NoPower.as_int());
        }
    }
    // A target in range, alive, of the right side.
    let harmful = !auras::positive(s);
    let wants_unit = s
        .effects
        .iter()
        .any(|e| matches!(e.target_a, 6 | 21 | 25 | 57 | 53));
    if wants_unit {
        let target = t
            .unit
            .unwrap_or(if harmful { Guid::zero() } else { caster });
        let Some(u) = map.unit_view(target) else {
            return Some(SpellCastResult::BadTargets.as_int());
        };
        let resurrects = s
            .effects
            .iter()
            .any(|e| matches!(e.effect, EFFECT_RESURRECT | EFFECT_RESURRECT_NEW));
        if !u.alive && !resurrects {
            return Some(SpellCastResult::TargetsDead.as_int());
        }
        if harmful && target != caster && !combat::can_attack(map, caster, target) {
            return Some(SpellCastResult::TargetFriendly.as_int());
        }
        if target != caster {
            let (min, max) = data.spells.range(s);
            let d = dist(me.position, u.position);
            let reach = me.combat_reach + u.combat_reach;
            if d > max + reach + 1.0 {
                return Some(SpellCastResult::OutOfRange.as_int());
            }
            if min > 0.0 && d < min {
                return Some(SpellCastResult::TooClose.as_int());
            }
        }
    }
    None
}

impl Map {
    pub fn unit_fields(&self, g: Guid) -> Option<&Fields> {
        if let Some(p) = self.players.get(&g) {
            return Some(&p.fields);
        }
        self.creature(g).map(|c| &c.fields)
    }

    pub fn caster_mut(&mut self, g: Guid) -> Option<&mut Caster> {
        if self.players.contains_key(&g) {
            return self.players.get_mut(&g).map(|p| &mut p.caster);
        }
        self.creature_mut(g).map(|c| &mut c.caster)
    }

    pub fn caster(&self, g: Guid) -> Option<&Caster> {
        if let Some(p) = self.players.get(&g) {
            return Some(&p.caster);
        }
        self.creature(g).map(|c| &c.caster)
    }
}

/// `CMSG_CAST_SPELL`.
pub fn cast_request(map: &mut Map, caster: Guid, spell: u32, t: &SpellCastTargets) {
    let targets = targets_of(t);
    cast(map, caster, spell, targets, None);
}

/// Cast `spell`: checked, then begun (or done at once if instant).
pub fn cast(map: &mut Map, caster: Guid, spell: u32, mut targets: Targets, item: Option<Guid>) {
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(spell) else {
        send_cast_result(map, caster, spell, Some(SpellCastResult::NotKnown.as_int()));
        return;
    };
    if s.passive() {
        return;
    }
    if let Some(r) = check(map, caster, s, &targets, item) {
        send_cast_result(map, caster, spell, Some(r));
        return;
    }
    if targets.unit.is_none() && auras::positive(s) {
        targets.unit = Some(caster);
    }
    // A next-swing spell waits for the swing; an auto-repeat one repeats.
    if s.on_next_swing() {
        crate::game::map::combat::queue_next_swing(map, caster, spell);
        return;
    }
    if s.auto_repeat() {
        crate::game::map::ranged::start(map, caster, spell, targets);
        return;
    }
    let now = map.now;
    let cast_time = data.spells.cast_time(s);
    if let Some(frame) = crate::game::conn::encode(&SMSG_SPELL_START {
        cast_item: item.unwrap_or_default(),
        caster,
        spell,
        flags: SMSG_SPELL_START_CastFlags::empty(),
        timer: cast_time,
        targets: wire_targets(&targets),
    }) {
        map.send_to_observers(caster, &frame, true);
    }
    if s.start_recovery_category != 0
        && let Some(c) = map.caster_mut(caster)
    {
        c.gcd_until = Some(now + Duration::from_millis(u64::from(s.start_recovery_time.max(1000))));
    }
    if cast_time == 0 {
        execute(map, caster, s, targets, item);
        return;
    }
    if let Some(c) = map.caster_mut(caster) {
        c.casting = Some(Casting {
            spell,
            targets,
            ends: now + Duration::from_millis(u64::from(cast_time)),
            item,
            pushbacks: 0,
            channel_until: None,
        });
    }
}

/// Cast `spell` at once, without its cast time: a next-swing spell at its
/// swing. Whether it went off.
pub fn cast_now(map: &mut Map, caster: Guid, spell: u32, targets: Targets) -> bool {
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(spell) else {
        return false;
    };
    if let Some(r) = check(map, caster, s, &targets, None) {
        send_cast_result(map, caster, spell, Some(r));
        return false;
    }
    execute(map, caster, s, targets, None);
    true
}

/// A spell cast for free, as another spell or an aura's tick does.
pub fn cast_triggered(map: &mut Map, caster: Guid, spell: u32, target: Option<Guid>) {
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(spell) else {
        return;
    };
    let targets = Targets {
        unit: target.or(Some(caster)),
        ..Default::default()
    };
    execute_effects(map, caster, s, &targets);
}

/// A cast completes: power and reagents taken, cooldowns started, the
/// spell goes off.
fn execute(map: &mut Map, caster: Guid, s: &SpellTemplate, targets: Targets, item: Option<Guid>) {
    let now = map.now;
    let cost = power_cost(map, caster, s);
    if cost > 0 {
        let index = power_index(s);
        if let Some(f) = map.unit_fields_mut(caster) {
            let have = f.get_u32(UNIT_FIELD_POWER1 + index);
            f.set_u32(UNIT_FIELD_POWER1 + index, have.saturating_sub(cost));
        }
        if index == 0
            && let Some(p) = map.players.get_mut(&caster)
        {
            p.mana_spent_at = Some(now);
        }
    }
    for (reagent, count) in s.reagents.iter().filter(|(r, _)| *r != 0) {
        let mut left = *count;
        while left > 0 {
            let Some(i) = map
                .players
                .get(&caster)
                .and_then(|p| p.items.iter().position(|it| it.entry == *reagent))
            else {
                break;
            };
            let n = map.players[&caster].items[i].count().min(left);
            crate::game::map::inventory::take_item(map, caster, i, n);
            left -= n;
        }
    }
    if let Some(c) = map.caster_mut(caster) {
        if s.recovery_time > 0 {
            c.cooldowns.insert(
                s.id,
                now + Duration::from_millis(u64::from(s.recovery_time)),
            );
        }
        if s.category != 0 && s.category_recovery_time > 0 {
            c.category_cooldowns.insert(
                s.category,
                now + Duration::from_millis(u64::from(s.category_recovery_time)),
            );
        }
    }
    let hits: Vec<Guid> = targets.unit.into_iter().collect();
    if let Some(frame) = crate::game::conn::encode(&SMSG_SPELL_GO {
        cast_item: item.unwrap_or_default(),
        caster,
        spell: s.id,
        flags: SMSG_SPELL_GO_CastFlags::empty(),
        hits,
        misses: Vec::new(),
        targets: wire_targets(&targets),
    }) {
        map.send_to_observers(caster, &frame, true);
    }
    send_cast_result(map, caster, s.id, None);
    if s.channeled() {
        let level = map.unit_view(caster).map(|u| u.level).unwrap_or(1);
        let ms = map.realm.data.spells.duration(s, level).unwrap_or(0);
        if let Some(c) = map.caster_mut(caster) {
            c.casting = Some(Casting {
                spell: s.id,
                targets,
                ends: now + Duration::from_millis(u64::from(ms)),
                item,
                pushbacks: 0,
                channel_until: Some(now + Duration::from_millis(u64::from(ms))),
            });
        }
        if let Some(p) = map.players.get(&caster) {
            p.outbound.send(&MSG_CHANNEL_START_Server {
                spell: s.id,
                duration: ms,
            });
        }
    }
    if !auras::positive(s)
        && let Some(t) = targets.unit
        && t != caster
        && guid::is_unit(t)
    {
        // Casting at a creature is a fight.
        crate::game::map::ai::attacked(map, t, caster);
    }
    execute_effects(map, caster, s, &targets);
}

/// The units an effect lands on.
fn effect_targets(
    map: &Map,
    caster: Guid,
    s: &SpellTemplate,
    index: usize,
    t: &Targets,
) -> Vec<Guid> {
    let e = &s.effects[index];
    let radius = map.realm.data.spells.radius(e.radius_index).max(1.0);
    let center = t
        .dest
        .or_else(|| map.position_of(caster))
        .unwrap_or(Vector3d {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
    let enemies_around = |at: Vector3d| -> Vec<Guid> {
        map.grid
            .around(at.x, at.y, radius)
            .filter(|g| (guid::is_player(*g) || guid::is_unit(*g)) && *g != caster)
            .filter(|g| map.position_of(*g).is_some_and(|p| dist(p, at) <= radius))
            .filter(|g| combat::can_attack(map, caster, *g))
            .collect()
    };
    match e.target_a {
        0 | 1 | 5 | 20 | 27 | 30 | 31 | 32 | 33 | 34 | 35 | 37 | 39 | 41 | 42 | 43 | 44 | 61 => {
            vec![caster]
        }
        6 | 21 | 25 | 45 | 53 | 55 | 57 => t.unit.into_iter().collect(),
        2 | 8 | 15 | 16 | 28 | 36 => enemies_around(center),
        7 | 22 => {
            if matches!(e.target_b, 15 | 16 | 28 | 36) {
                enemies_around(center)
            } else {
                vec![caster]
            }
        }
        24 => {
            let Some(me) = map.unit_view(caster) else {
                return Vec::new();
            };
            enemies_around(me.position)
                .into_iter()
                .filter(|g| {
                    map.position_of(*g).is_some_and(|p| {
                        let a = (p.y - me.position.y).atan2(p.x - me.position.x);
                        let mut d = (a - me.orientation).rem_euclid(std::f32::consts::TAU);
                        if d > std::f32::consts::PI {
                            d = std::f32::consts::TAU - d;
                        }
                        d <= std::f32::consts::FRAC_PI_3
                    })
                })
                .collect()
        }
        _ => t.unit.into_iter().collect(),
    }
}

fn execute_effects(map: &mut Map, caster: Guid, s: &SpellTemplate, t: &Targets) {
    let level = map.unit_view(caster).map(|u| u.level).unwrap_or(1);
    for index in 0..3 {
        let e = s.effects[index];
        if e.effect == 0 {
            continue;
        }
        for target in effect_targets(map, caster, s, index, t) {
            let amount = points(s, index, level);
            effect(map, caster, target, s, index, amount);
        }
        if e.effect == EFFECT_OPEN_LOCK
            && let Some(object) = t.object
        {
            crate::game::map::loot::open_object(map, caster, object);
        }
    }
}

/// One effect on one target.
fn effect(map: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate, index: usize, amount: i32) {
    let e = s.effects[index];
    match e.effect {
        EFFECT_INSTAKILL => {
            let health = map
                .unit_fields(target)
                .map(|f| f.get_u32(UNIT_FIELD_HEALTH))
                .unwrap_or(0);
            combat::deal_damage(map, caster, target, health);
        }
        EFFECT_SCHOOL_DAMAGE | EFFECT_HEALTH_LEECH => {
            let damage = amount.max(0) as u32;
            spell_damage(map, caster, target, s, damage);
            if e.effect == EFFECT_HEALTH_LEECH {
                heal(map, caster, caster, s.id, damage, false);
            }
        }
        EFFECT_HEAL => heal(map, caster, target, s.id, amount.max(0) as u32, false),
        EFFECT_HEAL_MAX_HEALTH => {
            let max = map
                .unit_fields(target)
                .map(|f| f.get_u32(UNIT_FIELD_MAXHEALTH))
                .unwrap_or(0);
            heal(map, caster, target, s.id, max, false);
        }
        EFFECT_APPLY_AURA | EFFECT_APPLY_AREA_AURA_PARTY | EFFECT_PERSISTENT_AREA_AURA => {
            if map.unit_view(target).is_some_and(|u| u.alive) {
                auras::apply(map, caster, target, s, index, amount);
            }
        }
        EFFECT_ENERGIZE => {
            let power = e.misc_value.clamp(0, 4) as u16;
            if let Some(f) = map.unit_fields_mut(target) {
                let max = f.get_u32(UNIT_FIELD_MAXPOWER1 + power);
                let now = f.get_u32(UNIT_FIELD_POWER1 + power);
                f.set_u32(
                    UNIT_FIELD_POWER1 + power,
                    (now + amount.max(0) as u32).min(max),
                );
            }
        }
        EFFECT_POWER_DRAIN => {
            if let Some(f) = map.unit_fields_mut(target) {
                let now = f.get_u32(UNIT_FIELD_POWER1);
                f.set_u32(UNIT_FIELD_POWER1, now.saturating_sub(amount.max(0) as u32));
            }
        }
        EFFECT_WEAPON_DAMAGE
        | EFFECT_WEAPON_DAMAGE_NOSCHOOL
        | EFFECT_NORMALIZED_WEAPON_DMG
        | EFFECT_WEAPON_PERCENT_DAMAGE => {
            let weapon = map
                .unit_fields(caster)
                .map(|f| (f.get_f32(UNIT_FIELD_MINDAMAGE) + f.get_f32(UNIT_FIELD_MAXDAMAGE)) / 2.0)
                .unwrap_or(1.0);
            let damage = if e.effect == EFFECT_WEAPON_PERCENT_DAMAGE {
                weapon * amount as f32 / 100.0
            } else {
                weapon + amount as f32
            };
            spell_damage(map, caster, target, s, damage.max(1.0) as u32);
        }
        EFFECT_TELEPORT_UNITS => {
            let data = map.realm.data.clone();
            let to = match data.spells.target_positions.get(&s.id) {
                Some(&(m, x, y, z, o)) => Some((m, Vector3d { x, y, z }, o)),
                // The hearthstone and its kind: home.
                None => map
                    .players
                    .get(&target)
                    .and_then(|p| p.row.state.bind)
                    .map(|b| {
                        (
                            b.map,
                            Vector3d {
                                x: b.x,
                                y: b.y,
                                z: b.z,
                            },
                            0.0,
                        )
                    }),
            };
            if let Some((m, pos, o)) = to {
                map.teleport(target, crate::game::map::MapKey::world(m), pos, o);
            }
        }
        EFFECT_LEAP => {
            if let Some(me) = map.unit_view(caster) {
                let d = map.realm.data.spells.radius(e.radius_index).max(10.0);
                let to = Vector3d {
                    x: me.position.x + d * me.orientation.cos(),
                    y: me.position.y + d * me.orientation.sin(),
                    z: me.position.z,
                };
                map.teleport(caster, map.key, to, me.orientation);
            }
        }
        EFFECT_CHARGE => {
            if let (Some(me), Some(them)) = (map.unit_view(caster), map.unit_view(target)) {
                let d = dist(me.position, them.position).max(0.1);
                let back = 2.0 / d;
                let to = Vector3d {
                    x: them.position.x + (me.position.x - them.position.x) * back,
                    y: them.position.y + (me.position.y - them.position.y) * back,
                    z: them.position.z,
                };
                let o = (them.position.y - me.position.y).atan2(them.position.x - me.position.x);
                map.teleport(caster, map.key, to, o);
                combat::start_attack(map, caster, target);
            }
        }
        EFFECT_CREATE_ITEM if e.item_type != 0 => {
            let _ = crate::game::map::inventory::add_item(
                map,
                caster,
                e.item_type,
                amount.max(1) as u32,
                true,
            );
        }
        EFFECT_LEARN_SPELL if e.trigger_spell != 0 => learn(map, target, e.trigger_spell),
        EFFECT_TRIGGER_SPELL if e.trigger_spell != 0 => {
            cast_triggered(map, caster, e.trigger_spell, Some(target));
        }
        EFFECT_DISPEL => {
            let friendly = !combat::can_attack(map, caster, target);
            let gone: Vec<(u32, Guid)> = auras::dispellable(map, target, e.misc_value as u32, friendly)
                .into_iter()
                .take(amount.max(1) as usize)
                .collect();
            for (spell, by) in gone {
                auras::remove_spell(map, target, spell, Some(by));
            }
        }
        EFFECT_INTERRUPT_CAST => interrupt(map, target),
        EFFECT_THREAT => crate::game::map::ai::add_threat(map, target, caster, amount as f32),
        EFFECT_ADD_COMBO_POINTS => {
            if let Some(c) = map.caster_mut(caster) {
                if c.combo_target != target {
                    c.combo_target = target;
                    c.combo_points = 0;
                }
                c.combo_points = (c.combo_points + amount.max(1) as u8).min(5);
                let points = c.combo_points;
                if let Some(p) = map.players.get_mut(&caster) {
                    p.fields.set_guid(PLAYER_FIELD_COMBO_TARGET, target);
                    p.fields.set_byte(PLAYER_FIELD_BYTES, 1, points);
                }
            }
        }
        EFFECT_DUEL => crate::game::map::duel::challenge(map, caster, target),
        EFFECT_ADD_EXTRA_ATTACKS => {
            crate::game::map::combat::grant_extra_attacks(map, caster, s.id, amount.max(1) as u32);
        }
        EFFECT_RESURRECT | EFFECT_RESURRECT_NEW => {
            let name = map
                .players
                .get(&caster)
                .map(|p| p.name.to_string())
                .unwrap_or_default();
            if let Some(p) = map.players.get_mut(&target)
                && p.death != crate::game::map::death::Death::Alive
            {
                p.resurrect_offer = Some((caster, amount.clamp(1, 100) as u32));
                p.outbound.send(&SMSG_RESURRECT_REQUEST {
                    guid: caster,
                    name,
                    player: true,
                });
            }
        }
        EFFECT_SKILL => {
            let skill = e.misc_value.max(0) as u32;
            let step = amount.max(1) as u32;
            if let Some(p) = map.players.get_mut(&caster) {
                crate::game::map::skills::learn_skill(p, skill, step);
            }
        }
        _ => {}
    }
}

/// Spell damage from `caster` to `target`: logged, absorbed, dealt.
pub fn spell_damage(map: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate, damage: u32) {
    let (left, absorbed) = auras::absorb(map, target, damage, s.school_mask());
    if let Some(frame) = crate::game::conn::encode(&SMSG_SPELLNONMELEEDAMAGELOG {
        target,
        attacker: caster,
        spell: s.id,
        damage: left,
        school: SpellSchool::try_from(s.school as u8).unwrap_or(SpellSchool::Normal),
        absorbed_damage: absorbed,
        resisted: 0,
        periodic_log: false,
        unused: 0,
        blocked: 0,
        hit_info: HitInfo::NormalSwing,
        extend_flag: 0,
    }) {
        map.send_to_observers(caster, &frame, true);
        if let Some(p) = map.players.get(&target)
            && !p.known.contains(&caster)
        {
            p.outbound.send_frame(frame);
        }
    }
    // A blow on a caster pushes its cast back.
    pushback(map, target);
    if left > 0 {
        combat::deal_damage(map, caster, target, left);
    }
}

/// A periodic aura's tick of damage: logged, absorbed, dealt; a leech
/// heals its caster by what it took.
pub fn periodic_damage(
    map: &mut Map,
    h: &auras::Holder,
    target: Guid,
    amount: u32,
    leech: bool,
) {
    let (left, absorbed) = auras::absorb(map, target, amount, h.school);
    if let Some(frame) = crate::game::conn::encode(&SMSG_PERIODICAURALOG {
        target,
        caster: h.caster,
        spell: h.spell,
        auras: vec![AuraLog {
            aura_type: AuraLog_AuraType::PeriodicDamage {
                damage1: left,
                school: SpellSchool::try_from(h.school.trailing_zeros() as u8)
                    .unwrap_or(SpellSchool::Normal),
                absorbed,
                resisted: 0,
            },
        }],
    }) {
        map.send_to_observers(target, &frame, true);
    }
    if left > 0 {
        combat::deal_damage(map, h.caster, target, left);
    }
    if leech {
        heal(map, h.caster, h.caster, h.spell, left, true);
    }
}

/// Heal `target` by `amount`, as `spell`.
pub fn heal(map: &mut Map, caster: Guid, target: Guid, spell: u32, amount: u32, periodic: bool) {
    let Some(f) = map.unit_fields_mut(target) else {
        return;
    };
    let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
    let now = f.get_u32(UNIT_FIELD_HEALTH);
    if now == 0 {
        return;
    }
    f.set_u32(UNIT_FIELD_HEALTH, (now + amount).min(max));
    let frame = if periodic {
        crate::game::conn::encode(&SMSG_PERIODICAURALOG {
            target,
            caster,
            spell,
            auras: vec![AuraLog {
                aura_type: AuraLog_AuraType::PeriodicHeal { damage2: amount },
            }],
        })
    } else {
        crate::game::conn::encode(&SMSG_SPELLHEALLOG {
            victim: target,
            caster,
            id: spell,
            damage: amount,
            critical: false,
        })
    };
    if let Some(frame) = frame {
        map.send_to_observers(target, &frame, true);
    }
}

/// A unit's cast is interrupted.
pub fn interrupt(map: &mut Map, g: Guid) {
    let Some(c) = map.caster_mut(g) else {
        return;
    };
    let Some(casting) = c.casting.take() else {
        return;
    };
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&SMSG_SPELL_FAILURE {
            guid: g,
            spell: casting.spell,
            result: SpellCastResult::Interrupted,
        });
    }
    if let Some(frame) = crate::game::conn::encode(&SMSG_SPELL_FAILED_OTHER {
        caster: g,
        id: casting.spell,
    }) {
        map.send_to_observers(g, &frame, false);
    }
    if casting.channel_until.is_some() {
        auras::remove_spell(
            map,
            casting.targets.unit.unwrap_or(g),
            casting.spell,
            Some(g),
        );
    }
}

/// Damage taken while casting pushes the cast back half a second, twice
/// at most.
fn pushback(map: &mut Map, g: Guid) {
    if let Some(c) = map.caster_mut(g)
        && let Some(casting) = c.casting.as_mut()
        && casting.channel_until.is_none()
        && casting.pushbacks < 2
    {
        casting.ends += Duration::from_millis(500);
        casting.pushbacks += 1;
    }
}

/// `CMSG_CANCEL_CAST` and `CMSG_CANCEL_CHANNELLING`.
pub fn cancel(map: &mut Map, g: Guid) {
    interrupt(map, g);
    crate::game::map::combat::cancel_queued(map, g);
}

/// A player moved: a cast that movement interrupts stops.
pub fn moved(map: &mut Map, g: Guid) {
    let Some(spell) = map
        .caster(g)
        .and_then(|c| c.casting.as_ref())
        .map(|c| c.spell)
    else {
        return;
    };
    // Interrupt flag 0x1: movement.
    if map
        .realm
        .data
        .spells
        .get(spell)
        .is_some_and(|s| s.interrupt_flags & 0x1 != 0)
    {
        interrupt(map, g);
    }
}

/// Casts that finish now go off; channels that end, end.
pub fn update(map: &mut Map) {
    let now = map.now;
    let data = map.realm.data.clone();
    let mut done: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.caster.casting.as_ref().is_some_and(|c| c.ends <= now))
        .map(|p| p.guid)
        .collect();
    done.extend(
        map.creatures
            .iter()
            .filter(|(_, c)| c.caster.casting.as_ref().is_some_and(|x| x.ends <= now))
            .map(|(_, c)| c.guid),
    );
    for g in done {
        let Some(casting) = map.caster_mut(g).and_then(|c| c.casting.take()) else {
            continue;
        };
        if casting.channel_until.is_some() {
            continue;
        }
        let Some(s) = data.spells.get(casting.spell) else {
            continue;
        };
        execute(map, g, s, casting.targets, casting.item);
    }
}

/// `CMSG_RESURRECT_RESPONSE`: a resurrection offered is taken.
pub fn resurrect_response(map: &mut Map, player: Guid, from: Guid, accept: bool) {
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let Some((caster, percent)) = p.resurrect_offer.take() else {
        return;
    };
    if caster != from || !accept {
        return;
    }
    let to = map.position_of(caster);
    let fraction = (percent as f32 / 100.0).clamp(0.05, 1.0);
    crate::game::map::death::resurrect(map, player, fraction);
    if let Some(pos) = to {
        map.teleport(player, map.key, pos, 0.0);
    }
}

/// A creature in a fight tries a spell from its list.
pub fn creature_cast(map: &mut Map, creature: Guid, victim: Guid) {
    let now = map.now;
    let data = map.realm.data.clone();
    let Some(c) = map.creature(creature) else {
        return;
    };
    if c.caster.casting.is_some() || c.caster.next_spell_at.is_some_and(|t| t > now) {
        return;
    }
    let list = data
        .creature_template(c.entry)
        .map(|t| t.spell_list_id)
        .filter(|l| *l != 0)
        .and_then(|l| data.spells.creature_spells.get(&l));
    let Some(list) = list else {
        if let Some(c) = map.creature_mut(creature) {
            c.caster.next_spell_at = Some(now + Duration::from_secs(3600));
        }
        return;
    };
    let mut rng = rand::rng();
    let pick = list
        .iter()
        .filter(|s| s.probability == 0 || rng.random_range(0..100) < s.probability)
        .find(|s| data.spells.get(s.spell).is_some())
        .copied();
    let delay = pick
        .map(|s| s.delay_repeat)
        .filter(|(lo, hi)| *hi > 0 || *lo > 0)
        .map(|(lo, hi)| {
            let hi = hi.max(lo);
            Duration::from_millis(u64::from(if hi > lo {
                rng.random_range(lo..=hi)
            } else {
                lo
            }))
        })
        .unwrap_or(Duration::from_secs(8));
    if let Some(c) = map.creature_mut(creature) {
        c.caster.next_spell_at = Some(now + delay);
    }
    let Some(spell) = pick else {
        return;
    };
    // Target 0 and 1: the victim; anything else, for now: itself.
    let target = if spell.target <= 1 { victim } else { creature };
    let targets = Targets {
        unit: Some(target),
        ..Default::default()
    };
    cast(map, creature, spell.spell, targets, None);
}

/// Talent points a player has to spend: one a level from ten, less those
/// spent.
pub fn free_talent_points(p: &crate::game::map::player::Player, data: &GameData) -> u32 {
    let earned = u32::from(p.level()).saturating_sub(9);
    earned.saturating_sub(spent_talent_points(p, data, None))
}

/// Talent points spent, in all trees or in `tab`.
fn spent_talent_points(
    p: &crate::game::map::player::Player,
    data: &GameData,
    tab: Option<u32>,
) -> u32 {
    data.spells
        .talents
        .values()
        .filter(|t| tab.is_none_or(|tab| t.tab == tab))
        .map(|t| {
            t.ranks
                .iter()
                .rposition(|r| *r != 0 && p.spells.contains(r))
                .map_or(0, |i| i as u32 + 1)
        })
        .sum()
}

pub fn refresh_talent_points(p: &mut crate::game::map::player::Player, data: &GameData) {
    let free = free_talent_points(p, data);
    p.fields.set_u32(PLAYER_CHARACTER_POINTS1, free);
}

/// `CMSG_LEARN_TALENT`: the next rank of a talent, if the player has a
/// point, the row is open and the talent it needs is learned.
pub fn learn_talent(map: &mut Map, player: Guid, talent: u32, rank: u32) {
    let data = map.realm.data.clone();
    let Some(t) = data.spells.talents.get(&talent) else {
        return;
    };
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let have = t
        .ranks
        .iter()
        .rposition(|r| *r != 0 && p.spells.contains(r))
        .map_or(0, |i| i as u32 + 1);
    let Some(&spell) = t.ranks.get(rank as usize) else {
        return;
    };
    if rank != have || spell == 0 || free_talent_points(p, &data) == 0 {
        return;
    }
    if spent_talent_points(p, &data, Some(t.tab)) < t.row * 5 {
        return;
    }
    if t.depends_on != 0
        && let Some(d) = data.spells.talents.get(&t.depends_on)
        && !d
            .ranks
            .get(t.depends_rank as usize)
            .is_some_and(|r| p.spells.contains(r))
    {
        return;
    }
    learn(map, player, spell);
    if let Some(p) = map.players.get_mut(&player) {
        refresh_talent_points(p, &data);
    }
}

/// `CMSG_USE_ITEM`: an item's on-use spell.
pub fn use_item(map: &mut Map, player: Guid, bag: u8, slot: u8, t: &SpellCastTargets) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let Some(i) = p.item_at(bag, slot) else {
        return;
    };
    let item = &p.items[i];
    let item_guid = item.guid;
    let Some(template) = data.items.get(&item.entry) else {
        return;
    };
    // Trigger 0: on use.
    let Some(spell) = template.spells.iter().find(|s| s.id != 0 && s.trigger == 0) else {
        return;
    };
    let mut targets = targets_of(t);
    targets.item = targets.item.or(Some(item_guid));
    cast(map, player, spell.id, targets, Some(item_guid));
}
