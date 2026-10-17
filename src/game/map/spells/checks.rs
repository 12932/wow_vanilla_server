//! Whether a cast may begin, as Thistle Tea's `CastValidation` checks it:
//! the caster's state, form and aura state, cooldowns and power, reagents,
//! tools, spell focus and gear; then the target: alive or dead, friend or
//! foe, its creature type and aura state, range, facing and line of sight.
//! Each failure is a 1.12 `SpellCastResult`.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::spells::{self as sd, SpellTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::combat::{self, dist};

use super::mods::{self, Mods};
use super::targets::{self, Implicit};
use super::{Targets, power_cost, power_index};

/// `SpellCastResult` codes.
pub mod fail {
    pub const AFFECTING_COMBAT: u8 = 0x00;
    pub const AURA_BOUNCED: u8 = 0x07;
    pub const BAD_IMPLICIT_TARGETS: u8 = 0x09;
    pub const BAD_TARGETS: u8 = 0x0a;
    pub const CASTER_AURASTATE: u8 = 0x12;
    pub const CASTER_DEAD: u8 = 0x13;
    pub const CONFUSED: u8 = 0x16;
    pub const DONT_REPORT: u8 = 0x17;
    pub const EQUIPPED_ITEM: u8 = 0x18;
    pub const EQUIPPED_ITEM_CLASS: u8 = 0x19;
    pub const ERROR: u8 = 0x1c;
    pub const FLEEING: u8 = 0x1e;
    pub const HIGHLEVEL: u8 = 0x20;
    pub const IMMUNE: u8 = 0x22;
    pub const INTERRUPTED: u8 = 0x23;
    pub const ITEM_GONE: u8 = 0x26;
    pub const LINE_OF_SIGHT: u8 = 0x2a;
    pub const MAINHAND_EMPTY: u8 = 0x2d;
    pub const LOWLEVEL: u8 = 0x2b;
    pub const MOVING: u8 = 0x2e;
    pub const NOT_BEHIND: u8 = 0x33;
    pub const NOT_INFRONT: u8 = 0x36;
    pub const NOT_IN_CONTROL: u8 = 0x37;
    pub const NOT_KNOWN: u8 = 0x38;
    pub const NOT_MOUNTED: u8 = 0x39;
    pub const NOT_ON_TAXI: u8 = 0x3a;
    pub const NOT_READY: u8 = 0x3c;
    pub const NOT_SHAPESHIFT: u8 = 0x3d;
    pub const NOT_STANDING: u8 = 0x3e;
    pub const NO_COMBO_POINTS: u8 = 0x46;
    pub const NO_POWER: u8 = 0x4d;
    pub const NOTHING_TO_DISPEL: u8 = 0x4e;
    pub const ONLY_SHAPESHIFT: u8 = 0x56;
    pub const ONLY_STEALTHED: u8 = 0x57;
    pub const OUT_OF_RANGE: u8 = 0x59;
    pub const PACIFIED: u8 = 0x5a;
    pub const REAGENTS: u8 = 0x5c;
    pub const REQUIRES_AREA: u8 = 0x5d;
    pub const REQUIRES_SPELL_FOCUS: u8 = 0x5e;
    pub const SILENCED: u8 = 0x60;
    pub const SPELL_IN_PROGRESS: u8 = 0x61;
    pub const STUNNED: u8 = 0x64;
    pub const TARGETS_DEAD: u8 = 0x65;
    pub const TARGET_AFFECTING_COMBAT: u8 = 0x66;
    pub const TARGET_AURASTATE: u8 = 0x67;
    pub const TARGET_ENEMY: u8 = 0x69;
    pub const TARGET_FRIENDLY: u8 = 0x6b;
    pub const TARGET_NOT_DEAD: u8 = 0x6e;
    pub const TOO_CLOSE: u8 = 0x76;
    pub const TOTEMS: u8 = 0x78;
    pub const UNIT_NOT_INFRONT: u8 = 0x7c;
}

/// `AuraState`s spells ask for.
pub const AURA_STATE_DEFENSE: u32 = 1;
pub const AURA_STATE_HEALTHLESS_20: u32 = 2;
pub const AURA_STATE_BERSERKING: u32 = 3;
pub const AURA_STATE_JUDGEMENT: u32 = 5;
pub const AURA_STATE_HUNTER_PARRY: u32 = 7;
pub const AURA_STATE_HEALTHLESS_15: u32 = 13;

/// Forms that are stances, not shapes: battle, defensive, berserker,
/// shadowform, stealth, moonkin.
const STANCE_LIKE: &[u8] = &[17, 18, 19, 28, 30, 31];

/// Whether the aura state `state` is set on `g`.
pub fn aura_state(map: &Map, g: Guid, state: u32) -> bool {
    if state == 0 {
        return true;
    }
    let field = map
        .unit_fields(g)
        .map(|f| f.get_u32(UNIT_FIELD_AURASTATE))
        .unwrap_or(0);
    if field & (1 << (state - 1)) != 0 {
        return true;
    }
    // Health states are worked out here, as they change with every blow.
    let health_pct = map.unit_fields(g).map(|f| {
        let max = f.get_u32(UNIT_FIELD_MAXHEALTH).max(1);
        f.get_u32(UNIT_FIELD_HEALTH) * 100 / max
    });
    match state {
        AURA_STATE_HEALTHLESS_20 => health_pct.is_some_and(|p| p < 20),
        AURA_STATE_HEALTHLESS_15 => health_pct.is_some_and(|p| p < 15),
        _ => false,
    }
}

/// Why a form stops `s`, if it does.
pub fn form_error(s: &SpellTemplate, form: u8) -> Option<u8> {
    let mask = if form > 0 { 1u32 << (form - 1) } else { 0 };
    if mask & s.stances_not != 0 {
        return Some(fail::NOT_SHAPESHIFT);
    }
    if mask & s.stances != 0 {
        return None;
    }
    let shifted = form > 0 && !STANCE_LIKE.contains(&form);
    if shifted {
        if s.attr(0, sd::ATTR_NOT_SHAPESHIFTED) {
            return Some(fail::NOT_SHAPESHIFT);
        }
        if s.stances != 0 {
            return Some(fail::ONLY_SHAPESHIFT);
        }
        None
    } else if s.stances != 0 && s.attributes[2] & 0x8_0000 == 0 {
        Some(fail::ONLY_SHAPESHIFT)
    } else {
        None
    }
}

/// Whether `from` sees `to`: the navmesh's line of sight, and a test's
/// walls.
pub fn line_of_sight(map: &mut Map, from: Vector3d, to: Vector3d) -> bool {
    let blocked = map
        .spells
        .walls
        .iter()
        .any(|(a, b)| crosses(from, to, *a, *b));
    if blocked {
        return false;
    }
    let eye = |p: Vector3d| Vector3d {
        x: p.x,
        y: p.y,
        z: p.z + 2.0,
    };
    match map.nav.as_mut() {
        Some(nav) => nav.line_of_sight(eye(from), eye(to)),
        None => true,
    }
}

/// Whether segment p-q crosses segment a-b, on the ground.
fn crosses(p: Vector3d, q: Vector3d, a: Vector3d, b: Vector3d) -> bool {
    let side = |o: Vector3d, x: Vector3d, y: Vector3d| {
        (x.x - o.x) * (y.y - o.y) - (x.y - o.y) * (y.x - o.x)
    };
    let (d1, d2) = (side(a, b, p), side(a, b, q));
    let (d3, d4) = (side(p, q, a), side(p, q, b));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// Whether `a` faces `b` within `arc` radians.
pub fn facing(map: &Map, a: Guid, b: Guid, arc: f32) -> bool {
    let (Some(ua), Some(pb)) = (map.unit_view(a), map.position_of(b)) else {
        return true;
    };
    let angle = (pb.y - ua.position.y).atan2(pb.x - ua.position.x);
    let mut d = (angle - ua.orientation).rem_euclid(std::f32::consts::TAU);
    if d > std::f32::consts::PI {
        d = std::f32::consts::TAU - d;
    }
    d <= arc / 2.0
}

/// Whether a creature's type is one the spell may target.
fn creature_type_allowed(s: &SpellTemplate, creature_type: u32) -> bool {
    s.target_creature_type == 0
        || (creature_type > 0 && s.target_creature_type & (1 << (creature_type - 1).min(31)) != 0)
}

/// Whether `caster` has the item `s` needs equipped.
fn equipped_ok(map: &Map, caster: Guid, s: &SpellTemplate) -> bool {
    // No spell asks for a consumable (class 0): 0 and -1 are none.
    if s.equipped_item_class <= 0 {
        return true;
    }
    let Some(p) = map.players.get(&caster) else {
        return true;
    };
    let data = &map.realm.data;
    let fits = |subclass: u32| {
        s.equipped_item_subclass_mask <= 0 || s.equipped_item_subclass_mask & (1 << subclass) != 0
    };
    p.items
        .iter()
        .filter(|i| i.bag == crate::game::store::BAG_PLAYER && i.slot < 19)
        .filter(|i| {
            i.fields.get_u32(ITEM_FIELD_MAXDURABILITY) == 0
                || i.fields.get_u32(ITEM_FIELD_DURABILITY) > 0
        })
        .filter_map(|i| data.item(i.entry))
        .any(|t| t.class as i32 == s.equipped_item_class && fits(t.subclass))
}

/// Whether a spell focus object of `focus` is near `at`.
/// Whether a client's movement flags say it is going somewhere: walking,
/// strafing, jumping or falling.
pub fn moving(f: &wow_world_messages::vanilla::MovementInfo_MovementFlags) -> bool {
    f.get_forward()
        || f.get_backward()
        || f.get_strafe_left()
        || f.get_strafe_right()
        || f.get_jumping().is_some()
        || f.get_fallingfar()
}

fn near_focus(map: &Map, at: Vector3d, focus: u32) -> bool {
    let data = &map.realm.data;
    map.gameobjects.iter().any(|(_, g)| {
        g.kind == 8
            && data.gameobject_templates.get(&g.entry).is_some_and(|t| {
                t.data[0] == focus && dist(g.position, at) <= (t.data[1] as f32).max(10.0)
            })
    })
}

/// Why `caster` can't cast `s` at `t` now, if it can't.
pub fn check(
    map: &mut Map,
    caster: Guid,
    s: &SpellTemplate,
    t: &Targets,
    item: Option<Guid>,
    triggered: bool,
    m: &Mods,
) -> Option<u8> {
    let now = map.now;
    let Some(me) = map.unit_view(caster) else {
        return Some(fail::ERROR);
    };
    if !me.alive && !s.attr(0, sd::ATTR_CASTABLE_WHILE_DEAD) {
        return Some(fail::CASTER_DEAD);
    }
    let player = map.players.get(&caster);
    if let Some(p) = player {
        if item.is_none() && !triggered && !p.spells.contains(&s.id) {
            return Some(fail::NOT_KNOWN);
        }
        if !triggered && p.stand_state() != 0 && !s.attr(0, sd::ATTR_ALLOW_WHILE_SITTING) {
            return Some(fail::NOT_STANDING);
        }
        // Food and drink, which standing up ends, aren't had on the move
        // (Thistle Tea's `Posture`).
        if s.aura_interrupt_flags & INTERRUPT_NOT_SEATED != 0 && moving(&p.movement.flags) {
            return Some(fail::MOVING);
        }
        if p.flight.is_some() {
            return Some(fail::NOT_ON_TAXI);
        }
        if !triggered
            && p.fields.get_u32(UNIT_FIELD_MOUNTDISPLAYID) != 0
            && !s.attr(0, sd::ATTR_ALLOW_WHILE_MOUNTED)
            && !s.has_aura(AURA_MOUNTED)
        {
            return Some(fail::NOT_MOUNTED);
        }
    }
    if let Some(c) = map.caster(caster)
        && !triggered
    {
        if c.casting
            .as_ref()
            .is_some_and(|x| x.channel_until.is_none() || x.spell == s.id)
        {
            return Some(fail::SPELL_IN_PROGRESS);
        }
        if c.cooldowns.get(&s.id).is_some_and(|t| *t > now)
            || (s.category != 0
                && c.category_cooldowns
                    .get(&s.category)
                    .is_some_and(|t| *t > now))
            || c.pending_cooldowns.contains(&s.id)
        {
            return Some(if s.attr(0, 0x200_0000) {
                fail::DONT_REPORT
            } else {
                fail::NOT_READY
            });
        }
        if s.start_recovery_category != 0 && c.gcd_until.is_some_and(|t| t > now) {
            return Some(fail::NOT_READY);
        }
        // A school locked out by an interrupt.
        if c.school_lockout
            .iter()
            .any(|(mask, until)| *until > now && mask & s.school_mask() != 0)
            && !s.melee_or_ranged()
        {
            return Some(fail::SILENCED);
        }
    }
    // The caster's state.
    // Breaking free (a trinket, Will of the Forsaken) works under control.
    let frees = s.effects.iter().any(|e| {
        e.effect == 108
            || (e.effect == 6
                && matches!(e.aura, AURA_MECHANIC_IMMUNITY | AURA_MECHANIC_IMMUNITY_MASK))
    });
    if !triggered && let Some(a) = auras::auras(map, caster) {
        if a.has(AURA_MOD_STUN) && !frees {
            return Some(fail::STUNNED);
        }
        if a.has(AURA_MOD_CONFUSE) && !frees {
            return Some(fail::CONFUSED);
        }
        if a.has(AURA_MOD_FEAR) && !frees {
            return Some(fail::FLEEING);
        }
        if a.has(AURA_MOD_POSSESS) || a.has(AURA_MOD_CHARM) {
            return Some(fail::NOT_IN_CONTROL);
        }
        // Silence stops spells (anything of magic damage class, or not an
        // ability); pacify stops abilities.
        let spell_like = s.dmg_class == 1 || (!s.melee_or_ranged() && !s.ability());
        if spell_like && !frees && (a.has(AURA_MOD_SILENCE) || a.has(AURA_MOD_PACIFY_SILENCE)) {
            return Some(fail::SILENCED);
        }
        if s.melee_or_ranged() && (a.has(AURA_MOD_PACIFY) || a.has(AURA_MOD_PACIFY_SILENCE)) {
            return Some(fail::PACIFIED);
        }
    }
    if !triggered {
        if s.attr(0, sd::ATTR_NOT_IN_COMBAT)
            && map.unit_fields(caster).is_some_and(|f| {
                f.has_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                )
            })
        {
            return Some(fail::AFFECTING_COMBAT);
        }
        if s.attr(0, sd::ATTR_ONLY_STEALTHED)
            && !auras::auras(map, caster).is_some_and(|a| a.has(AURA_MOD_STEALTH))
        {
            return Some(fail::ONLY_STEALTHED);
        }
        if let Some(e) = form_error(s, auras::form(map, caster)) {
            return Some(e);
        }
        if let Some(e) = super::environment::check(map, caster, s) {
            return Some(e);
        }
        if !super::area::allows(map, caster, s.id) {
            return Some(fail::REQUIRES_AREA);
        }
        // A spirit of redemption only heals.
        if !super::redemption::allows(map, caster, s) {
            return Some(fail::NOT_SHAPESHIFT);
        }
        if s.caster_aura_state != 0 && !aura_state(map, caster, s.caster_aura_state) {
            return Some(fail::CASTER_AURASTATE);
        }
        // A stronger rank of a helpful aura already on the caster.
        if auras::positive(s)
            && t.unit.is_none_or(|u| u == caster)
            && s.effects.iter().any(|e| e.effect == 6)
            && let Some(a) = auras::auras(map, caster)
        {
            let data = &map.realm.data;
            let mine = data.spells.ranks.get(&s.id).map(|r| r.rank).unwrap_or(0);
            if a.holders.iter().any(|h| {
                h.spell != s.id
                    && !h.passive
                    && data.spells.same_kind(h.spell, s.id)
                    && data
                        .spells
                        .ranks
                        .get(&h.spell)
                        .is_some_and(|r| r.rank > mine)
            }) {
                return Some(fail::AURA_BOUNCED);
            }
        }
        if let Some(r) = crate::game::map::pets::check(map, caster, s, t) {
            return Some(r);
        }
        if let Some(r) = crate::game::map::professions::check(map, caster, s, t) {
            return Some(r);
        }
        // Power, reagents, tools, focus, gear.
        let cost = m.apply_ms(mods::OP_COST, power_cost(map, caster, s));
        if cost > 0 {
            let have = map
                .unit_fields(caster)
                .map(|f| f.get_u32(UNIT_FIELD_POWER1 + power_index(s)))
                .unwrap_or(0);
            if have < cost {
                return Some(fail::NO_POWER);
            }
        }
        if let Some(p) = map.players.get(&caster)
            && !p.gm
        {
            for (reagent, count) in s.reagents.iter().filter(|(r, _)| *r != 0) {
                if p.count_of(*reagent) < *count {
                    return Some(fail::REAGENTS);
                }
            }
            for tool in s.totems.iter().filter(|t| **t != 0) {
                if p.count_of(*tool) == 0 {
                    return Some(fail::TOTEMS);
                }
            }
        }
        if s.requires_spell_focus != 0
            && map.players.contains_key(&caster)
            && !near_focus(map, me.position, s.requires_spell_focus)
        {
            return Some(fail::REQUIRES_SPELL_FOCUS);
        }
        if !equipped_ok(map, caster, s) {
            return Some(if s.equipped_item_class == 2 {
                fail::MAINHAND_EMPTY
            } else {
                fail::EQUIPPED_ITEM_CLASS
            });
        }
        if s.equipped_item_class == 2
            && auras::auras(map, caster).is_some_and(|a| a.has(AURA_MOD_DISARM))
            && s.dmg_class == 2
        {
            return Some(fail::EQUIPPED_ITEM_CLASS);
        }
        // A finishing move needs combo points on its target.
        if s.finisher() && map.players.contains_key(&caster) {
            let c = map.caster(caster);
            let target = t.unit.unwrap_or_default();
            if c.is_none_or(|c| c.combo_points == 0 || c.combo_target != target) {
                return Some(fail::NO_COMBO_POINTS);
            }
        }
    }
    check_target(map, caster, s, t, triggered, m)
}

/// The target's checks.
fn check_target(
    map: &mut Map,
    caster: Guid,
    s: &SpellTemplate,
    t: &Targets,
    triggered: bool,
    m: &Mods,
) -> Option<u8> {
    let harmful = !auras::positive(s);
    let wants_unit = s
        .effects
        .iter()
        .filter(|e| e.effect != 0)
        .any(|e| targets::wants_unit(e.target_a));
    if !wants_unit {
        // A destination's range.
        if let (Some(d), Some(me)) = (t.dest, map.unit_view(caster))
            && s.effects.iter().any(|e| {
                matches!(
                    targets::implicit(e.target_a),
                    Implicit::AoeEnemyAtDest | Implicit::AoeAllyAtDest | Implicit::Location
                )
            })
            && !triggered
        {
            let (_, max) = map.realm.data.spells.range(s);
            let max = m.apply(mods::OP_RANGE, max);
            if max > 0.0 && dist(me.position, d) > max + 2.0 {
                return Some(fail::OUT_OF_RANGE);
            }
        }
        return None;
    }
    let target = t
        .unit
        .unwrap_or(if harmful { Guid::zero() } else { caster });
    if target.is_zero() {
        return Some(fail::BAD_IMPLICIT_TARGETS);
    }
    let Some(u) = map.unit_view(target) else {
        return Some(fail::BAD_TARGETS);
    };
    let resurrects = s.effects.iter().any(|e| matches!(e.effect, 18 | 113));
    if resurrects {
        if u.alive {
            return Some(fail::TARGET_NOT_DEAD);
        }
    } else if !u.alive && !s.attr(2, sd::ATTR_EX2_ALLOW_DEAD_TARGET) && !s.has_effect(95) {
        return Some(fail::TARGETS_DEAD);
    }
    if target != caster {
        let requires_enemy = s
            .effects
            .iter()
            .any(|e| matches!(targets::implicit(e.target_a), Implicit::Enemy) || e.target_a == 53);
        let requires_friend = s.effects.iter().any(|e| {
            matches!(
                targets::implicit(e.target_a),
                Implicit::Ally | Implicit::ChainHeal | Implicit::PartyMember
            )
        });
        if (requires_enemy || harmful && !resurrects)
            && !combat::can_attack(map, caster, target)
            && !s.has_effect(95)
        {
            return Some(fail::TARGET_FRIENDLY);
        }
        if requires_friend && combat::can_attack(map, caster, target) {
            return Some(fail::TARGET_ENEMY);
        }
    }
    if let Some(c) = map.creature(target) {
        let creature_type = map
            .realm
            .data
            .creature_template(c.entry)
            .map(|t| t.creature_type)
            .unwrap_or(0);
        if !creature_type_allowed(s, creature_type) {
            return Some(fail::BAD_TARGETS);
        }
    } else if s.target_creature_type != 0 && guid::is_player(target) {
        // Players are humanoids.
        if !creature_type_allowed(s, 7) {
            return Some(fail::BAD_TARGETS);
        }
    }
    if s.max_target_level > 0 && u.level > s.max_target_level {
        return Some(fail::HIGHLEVEL);
    }
    if s.target_aura_state != 0 && !aura_state(map, target, s.target_aura_state) {
        return Some(fail::TARGET_AURASTATE);
    }
    // Mechanic immunity of a harmful spell.
    if harmful
        && target != caster
        && auras::auras(map, target).is_some_and(|a| {
            let mechanics: Vec<u32> = std::iter::once(s.mechanic).filter(|m| *m != 0).collect();
            !mechanics.is_empty() && mechanics.iter().all(|m| a.mechanic_immune(*m))
        })
    {
        return Some(fail::IMMUNE);
    }
    if target != caster && !triggered {
        let me = map.unit_view(caster)?;
        let (min, max) = map.realm.data.spells.range(s);
        let max = m.apply(mods::OP_RANGE, max);
        let d = dist(me.position, u.position);
        let reach = if s.melee_or_ranged() && max <= 5.0 {
            combat::melee_range(&me, &u)
        } else {
            max + me.combat_reach + u.combat_reach
        };
        if d > reach + 1.0 {
            return Some(fail::OUT_OF_RANGE);
        }
        if min > 0.0 && d < min {
            return Some(fail::TOO_CLOSE);
        }
        if s.from_behind() && facing(map, target, caster, std::f32::consts::PI) {
            return Some(fail::NOT_BEHIND);
        }
        // Players face what they cast at; abilities and harmful spells
        // within half a circle in front.
        if map.players.contains_key(&caster)
            && harmful
            && (s.melee_or_ranged() || s.dmg_class == 1)
            && !facing(map, caster, target, std::f32::consts::PI)
        {
            return Some(fail::UNIT_NOT_INFRONT);
        }
        if !s.attr(2, sd::ATTR_EX2_IGNORE_LOS) {
            let (from, to) = (me.position, u.position);
            if !line_of_sight(map, from, to) {
                return Some(fail::LINE_OF_SIGHT);
            }
        }
    }
    None
}
