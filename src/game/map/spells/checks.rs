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
use super::{Targets, spell_cost, unpayable};

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
    pub const NO_EDIBLE_CORPSES: u8 = 0x86;
    pub const NO_PET: u8 = 0x4c;
    pub const NO_POWER: u8 = 0x4d;
    pub const NOTHING_TO_DISPEL: u8 = 0x4e;
    pub const ONLY_BATTLEGROUNDS: u8 = 0x87;
    pub const ONLY_SHAPESHIFT: u8 = 0x56;
    pub const ONLY_STEALTHED: u8 = 0x57;
    pub const OUT_OF_RANGE: u8 = 0x59;
    pub const PACIFIED: u8 = 0x5a;
    pub const REAGENTS: u8 = 0x5c;
    pub const REQUIRES_AREA: u8 = 0x5d;
    pub const REQUIRES_SPELL_FOCUS: u8 = 0x5e;
    pub const SILENCED: u8 = 0x60;
    pub const SPELL_IN_PROGRESS: u8 = 0x61;
    pub const SPELL_LEARNED: u8 = 0x62;
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

/// [`line_of_sight`] on the tiles already read, for a look that can't ask
/// for one (a tile not in yet hides nothing).
pub fn sight(map: &Map, from: Vector3d, to: Vector3d) -> bool {
    if map
        .spells
        .walls
        .iter()
        .any(|(a, b)| crosses(from, to, *a, *b))
    {
        return false;
    }
    let eye = |p: Vector3d| Vector3d { z: p.z + 2.0, ..p };
    map.nav
        .as_ref()
        .is_none_or(|nav| nav.sight(eye(from), eye(to)))
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

/// Gouge's attributes: its target must face its caster.
const TARGET_FACING_CASTER: u32 = 0x15_0010;
/// The melee range index.
const RANGE_MELEE: u32 = 2;
/// Units this close face each other whatever their facing.
const OVERLAP_DISTANCE: f32 = 1.4;

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

/// Whether unit `g`'s type is one the spell may land on: a creature's
/// template's, a player's humanoid.
pub(crate) fn unit_type_allowed(map: &Map, s: &SpellTemplate, g: Guid) -> bool {
    if s.target_creature_type == 0 {
        return true;
    }
    let creature_type = match map.creature(g) {
        Some(c) => map
            .realm
            .data
            .creature_template(c.entry)
            .map_or(0, |t| t.creature_type),
        None => 7,
    };
    creature_type_allowed(s, creature_type)
}

/// Whether `caster` has the item `s` needs equipped.
pub(crate) fn equipped_ok(map: &Map, caster: Guid, s: &SpellTemplate) -> bool {
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

/// Whether `caster`'s item `item`, a spell's target, is of the kind `s`
/// wants (mangos's `IsFitToSpellRequirements`): its class and subclass,
/// and its inventory type for a spell at items. An item that isn't the
/// caster's (one in a trade window) is checked where it is.
fn target_item_fits(map: &Map, caster: Guid, s: &SpellTemplate, item: Guid) -> bool {
    const TARGET_FLAG_ITEM: u32 = 0x10;
    let Some(t) = map
        .players
        .get(&caster)
        .and_then(|p| p.item_by_guid(item).map(|i| p.items[i].entry))
        .and_then(|entry| map.realm.data.item(entry))
    else {
        return true;
    };
    (s.equipped_item_class < 0
        || (t.class as i32 == s.equipped_item_class
            && (s.equipped_item_subclass_mask <= 0
                || s.equipped_item_subclass_mask & (1 << t.subclass) != 0)))
        && (s.equipped_item_inventory_type_mask <= 0
            || s.targets & TARGET_FLAG_ITEM == 0
            || s.equipped_item_inventory_type_mask & (1 << t.inventory_type) != 0)
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

/// Whether a stand state isn't standing, as mangos's `IsStandState` has
/// it: sitting, in a chair, asleep or kneeling; the dead stand.
pub fn seated(state: u8) -> bool {
    !matches!(state, 0 | 7)
}

/// `PreventionType`: what silence keeps from being cast, and what pacify
/// does.
pub const PREVENTION_SILENCE: u32 = 1;
pub const PREVENTION_PACIFY: u32 = 2;

/// Whether a silence or a pacify on `caster` keeps `s` from being cast,
/// by its prevention type (the part of [`check`] a repeating shot meets
/// again at each shot, as mangos's `CheckCasterAuras` does).
pub fn prevented(map: &Map, caster: Guid, s: &SpellTemplate) -> bool {
    let kinds: &[u32] = match s.prevention_type {
        PREVENTION_SILENCE => &[AURA_MOD_SILENCE, AURA_MOD_PACIFY_SILENCE],
        PREVENTION_PACIFY => &[AURA_MOD_PACIFY, AURA_MOD_PACIFY_SILENCE],
        _ => return false,
    };
    let data = &map.realm.data;
    auras::auras(map, caster).is_some_and(|a| {
        a.holders.iter().any(|h| {
            h.effects
                .iter()
                .any(|x| kinds.contains(&x.kind) && !cancels(data, s, h, x))
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
    if !triggered && let Some(code) = crate::game::map::battleground::spell_check(map, caster, s) {
        return Some(code);
    }
    if !triggered && let Some(code) = crate::game::map::duel::check(map, caster, s, t.unit) {
        return Some(code);
    }
    if !triggered && let Some(code) = crate::game::map::objects::ritual_check(map, caster, s) {
        return Some(code);
    }
    let player = map.players.get(&caster);
    if let Some(p) = player {
        if !triggered && seated(p.stand_state()) && !s.attr(0, sd::ATTR_ALLOW_WHILE_SITTING) {
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
    if let Some(code) = crate::game::map::mounts::check(map, caster, s, triggered) {
        return Some(code);
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
            // A cooldown waiting for its event holds its category too, as
            // mangos's permanent cooldown and Thistle Tea's pending one do.
            || (s.category != 0
                && c.pending_cooldowns.iter().any(|p| {
                    map.realm
                        .data
                        .spells
                        .get(*p)
                        .is_some_and(|t| t.category == s.category)
                }))
        {
            return Some(if s.attr(0, 0x200_0000) {
                fail::DONT_REPORT
            } else {
                fail::NOT_READY
            });
        }
        if super::on_gcd(c, s.start_recovery_category, now) {
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
    // The caster's state. Breaking free (a trinket, Will of the Forsaken,
    // Divine Shield, Blink) works under what the spell's own immunities
    // cancel, and only that (mangos-classic's `CheckCasterAuras`).
    // A spell that ignores its caster's restrictions isn't held by them
    // (mangos-classic's `CheckCasterAuras` under
    // `SPELL_ATTR_EX3_IGNORE_CASTER_AND_TARGET_RESTRICTIONS`).
    if !triggered
        && !s.attr(3, sd::ATTR_EX3_IGNORE_CASTER_AND_TARGET_RESTRICTIONS)
        && let Some(a) = auras::auras(map, caster)
    {
        let data = &map.realm.data;
        let held = |kinds: &[u32]| {
            a.holders.iter().any(|h| {
                h.effects
                    .iter()
                    .any(|x| kinds.contains(&x.kind) && !cancels(data, s, h, x))
            })
        };
        if held(&[AURA_MOD_STUN]) {
            return Some(fail::STUNNED);
        }
        // Silence stops what its prevention type says silence stops, and
        // pacify what pacify does (mangos's `Spell::CheckCasterAuras`,
        // Thistle Tea's `CombatControl`): a wand's Shoot goes on under a
        // silence.
        if s.prevention_type == PREVENTION_SILENCE
            && held(&[AURA_MOD_SILENCE, AURA_MOD_PACIFY_SILENCE])
        {
            return Some(fail::SILENCED);
        }
        if s.prevention_type == PREVENTION_PACIFY
            && held(&[AURA_MOD_PACIFY, AURA_MOD_PACIFY_SILENCE])
        {
            return Some(fail::PACIFIED);
        }
        // A fear kept from taking hold (Recklessness) doesn't flee, as
        // mangos-classic's `SetFleeing` sets no fleeing under
        // `SPELL_AURA_PREVENTS_FLEEING`.
        if held(&[AURA_MOD_FEAR]) && !a.has(AURA_PREVENTS_FLEEING) {
            return Some(fail::FLEEING);
        }
        if held(&[AURA_MOD_CONFUSE]) {
            return Some(fail::CONFUSED);
        }
        // A charm's own AI casts for the one it holds (a creature's charm
        // of a player: Thistle Tea's `PlayerCharm`); nothing else does.
        let charm_ai = map
            .players
            .get(&caster)
            .and_then(|p| p.owned.possessed_by)
            .is_some_and(|pos| !pos.manual);
        if (a.has(AURA_MOD_POSSESS) || a.has(AURA_MOD_CHARM)) && !charm_ai {
            return Some(fail::NOT_IN_CONTROL);
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
        // A battleground's own spells (Thistle Tea's `Spell.Battleground`).
        if s.attr(3, sd::ATTR_EX3_BATTLEGROUND)
            && map.players.contains_key(&caster)
            && map.battleground.is_none()
        {
            return Some(fail::ONLY_BATTLEGROUNDS);
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
        // A stronger spell of a group that keeps its highest, on the unit a
        // helpful aura is cast at, the caster or another (Thistle Tea's
        // single-target upgrades; mangos's `CheckCast` refusing a weaker
        // aura than what is there). An area's is left to each it lands on.
        if auras::positive(s)
            && s.effects.iter().any(|e| e.effect == 6)
            && !super::outcome::area(s)
            && !s
                .effects
                .iter()
                .any(|e| e.effect == super::EFFECT_APPLY_AREA_AURA_PARTY)
            && auras::group_blocks(map, t.unit.unwrap_or(caster), s)
        {
            return Some(fail::AURA_BOUNCED);
        }
        if let Some(r) = crate::game::map::pets::check(map, caster, s, t) {
            return Some(r);
        }
        if let Some(r) = crate::game::map::professions::check(map, caster, s, t, item) {
            return Some(r);
        }
        if let Some(r) = crate::game::map::professions::fishing_spot(map, caster, s) {
            return Some(r);
        }
        // Power (none for an item's spell: mangos's `CheckPower`), reagents,
        // tools, focus, gear.
        let cost = if item.is_some() {
            0
        } else {
            spell_cost(map, caster, s, m)
        };
        if let Some(r) = unpayable(map, caster, s, cost) {
            return Some(r);
        }
        if let Some(p) = map.players.get(&caster)
            && !p.gm
        {
            // Carried, not banked (Thistle Tea's `count_entry`).
            for (reagent, count) in s.reagents.iter().filter(|(r, _)| *r != 0) {
                if p.carried_count_of(*reagent) < *count {
                    return Some(fail::REAGENTS);
                }
            }
            for tool in s.totems.iter().filter(|t| **t != 0) {
                if let Some(code) = crate::game::map::professions::tool_missing(p, s, *tool) {
                    return Some(code);
                }
            }
        }
        if let Some(code) = room_for_made(map, caster, s) {
            return Some(code);
        }
        if s.requires_spell_focus != 0
            && map.players.contains_key(&caster)
            && !crate::game::map::professions::focus_near(map, caster, s.requires_spell_focus)
        {
            return Some(fail::REQUIRES_SPELL_FOCUS);
        }
        // Cannibalize wants a body to eat before it costs anything.
        if s.script_name == "spell_cannibalize" && !super::scripts::edible_corpse(map, caster, s) {
            return Some(fail::NO_EDIBLE_CORPSES);
        }
        // A script's creature near the caster wants one there.
        if targets::script_creature_missing(map, caster, s, t, m) {
            return Some(fail::BAD_TARGETS);
        }
        // Without the weapon or item it wants, as mangos's `CheckItems`
        // and Thistle Tea answer it: at an item, that item is what must be
        // of the kind (an enchantment's bracer, not the gear its enchanter
        // wears); else something worn.
        let fits = match t.item {
            Some(item) => target_item_fits(map, caster, s, item),
            None => equipped_ok(map, caster, s),
        };
        if !fits {
            return Some(fail::EQUIPPED_ITEM_CLASS);
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

/// What a cast pays for, checked again as it goes off (mangos's
/// `CheckCast` at the cast, Thistle Tea's launch): the power it costs (a
/// caster drained while casting has none to pay), its reagents (carried,
/// where they are taken from: not banked meanwhile), and room
/// for what it makes. Why it fails, if it does.
pub fn at_launch(map: &Map, caster: Guid, s: &SpellTemplate, cost: u32) -> Option<u8> {
    if let Some(why) = super::unpayable(map, caster, s, cost) {
        return Some(why);
    }
    if let Some(p) = map.players.get(&caster)
        && !p.gm
        && s.reagents
            .iter()
            .any(|(r, n)| *r != 0 && p.carried_count_of(*r) < *n)
    {
        return Some(fail::REAGENTS);
    }
    room_for_made(map, caster, s)
}

/// Whether one of what `s` makes fits in its caster's bags (mangos's
/// `CheckItems`, which asks for one): if not, they are told why, and the
/// cast fails before it costs anything. What doesn't fit of more isn't
/// made.
fn room_for_made(map: &Map, caster: Guid, s: &SpellTemplate) -> Option<u8> {
    let p = map.players.get(&caster)?;
    let data = &map.realm.data;
    for e in &s.effects {
        if e.effect != super::EFFECT_CREATE_ITEM || e.item_type == 0 {
            continue;
        }
        let item =
            super::scripts::created_item(map, caster, s, u32::try_from(e.item_type).unwrap_or(0));
        if data.item(item).is_none() {
            continue;
        }
        if let Err(why) = crate::game::map::inventory::room_for(p, data, item, 1) {
            crate::game::map::inventory::send_fail(p, why, Guid::zero());
            return Some(fail::DONT_REPORT);
        }
    }
    None
}

/// Whether `s` brings back the dead.
/// A player's cast at a unit as it ends, before it costs anything: the
/// unit is still there, alive (unless the spell is for the dead), still a
/// foe for a harmful spell, in range with mangos's tolerance (a tenth of
/// the range, at most three yards, beyond the reaches) and in front if the
/// spell must be cast forward (mangos's `CheckCast(false)` and
/// `CheckRange(false)`, Thistle Tea's launch checks). Why not, if not.
pub fn at_finish(
    map: &mut Map,
    caster: Guid,
    s: &SpellTemplate,
    t: &Targets,
    m: &Mods,
) -> Option<u8> {
    if let Some(code) = caster_at_finish(map, caster, s) {
        return Some(code);
    }
    // A script's creature gone meanwhile (Thistle Tea's launch recheck,
    // mangos's targets filled again as the cast goes off).
    if map.players.contains_key(&caster) && targets::script_creature_missing(map, caster, s, t, m) {
        return Some(fail::BAD_TARGETS);
    }
    let target = t.unit.filter(|u| *u != caster)?;
    let Some(u) = map.unit_view(target) else {
        return Some(fail::BAD_TARGETS);
    };
    if !u.alive && !for_the_dead(s) && !resurrects(s) {
        return Some(fail::TARGETS_DEAD);
    }
    // A creature's too: a cast at an enemy made friendly or out of reach
    // of attack as it was cast fails, as mangos's `Spell::cast` checks it
    // again and Thistle Tea's `CastTarget.validate` does at launch (Keeper
    // Remulos's Starfire, begun at Eranikus, landed after his redemption
    // and drew him back into a fight). Only at an enemy, as mangos's
    // `CheckCast` holds a target to its effect's harmful filter alone: a
    // script's target needn't be one (Ragnaros's Elemental Fire, at the
    // Majordomo his summoning made friendly).
    let requires_enemy = s
        .effects
        .iter()
        .any(|e| matches!(targets::implicit(e.target_a), Implicit::Enemy) || e.target_a == 53);
    let harmful = requires_enemy
        || map.players.contains_key(&caster) && !auras::positive(s) && !resurrects(s);
    if harmful && !combat::can_attack(map, caster, target) && !s.has_effect(95) {
        return Some(fail::TARGET_FRIENDLY);
    }
    if !map.players.contains_key(&caster) {
        return None;
    }
    // A player's side gone from sight meanwhile (into stealth), as mangos's
    // `CheckCast(false)` has it "since the time of cast start".
    if harmful && player_side(map, target) && !super::stealth::sees(map, caster, target) {
        return Some(fail::BAD_TARGETS);
    }
    let me = map.unit_view(caster)?;
    let (_, base) = map.realm.data.spells.range(s);
    let max = m.apply(mods::OP_RANGE, base).max(0.0);
    let d = dist(me.position, u.position);
    let melee = s.melee_or_ranged() && max <= 5.0;
    let reach = if melee {
        combat::melee_range(&me, &u) + 1.0
    } else {
        max + me.combat_reach + u.combat_reach + (max * 0.1).min(3.0)
    } + moving_leeway(&me, &u, melee, target);
    if base > 0.0 && d > reach {
        return Some(fail::OUT_OF_RANGE);
    }
    let forward = !s.on_next_swing()
        && (s.range_index == RANGE_MELEE || s.custom(sd::CUSTOM_FACE_TARGET) || s.auto_repeat());
    if forward && d >= OVERLAP_DISTANCE && !facing(map, caster, target, std::f32::consts::PI) {
        return Some(fail::UNIT_NOT_INFRONT);
    }
    None
}

/// A player's own state as their cast ends, checked again before it costs
/// anything, as mangos's `Spell::cast` runs `CheckCast(false)` and Thistle
/// Tea's `Casting.complete` its requirements: still standing (unless the
/// spell allows sitting), not moving with food or drink, not on a flight,
/// still somewhere a mount may be had, still where the spell's area rules
/// allow, and its school not locked out meanwhile.
fn caster_at_finish(map: &mut Map, caster: Guid, s: &SpellTemplate) -> Option<u8> {
    let p = map.players.get(&caster)?;
    if seated(p.stand_state()) && !s.attr(0, sd::ATTR_ALLOW_WHILE_SITTING) {
        return Some(fail::NOT_STANDING);
    }
    if s.aura_interrupt_flags & INTERRUPT_NOT_SEATED != 0 && moving(&p.movement.flags) {
        return Some(fail::MOVING);
    }
    if p.flight.is_some() {
        return Some(fail::NOT_ON_TAXI);
    }
    let now = map.now;
    let locked = p
        .caster
        .school_lockout
        .iter()
        .any(|(mask, until)| *until > now && mask & s.school_mask() != 0);
    if locked && !s.melee_or_ranged() {
        return Some(fail::SILENCED);
    }
    if let Some(code) = crate::game::map::mounts::check(map, caster, s, false) {
        return Some(code);
    }
    if !super::area::allows(map, caster, s.id) {
        return Some(fail::REQUIRES_AREA);
    }
    // Its focus still by them (a campfire burnt out meanwhile), as mangos's
    // `CheckCast(false)` and Thistle Tea's launch revalidation have it.
    if s.requires_spell_focus != 0
        && !crate::game::map::professions::focus_near(map, caster, s.requires_spell_focus)
    {
        return Some(fail::REQUIRES_SPELL_FOCUS);
    }
    None
}

/// Whether `g` is a player or a player's (mangos's
/// `UNIT_FLAG_PLAYER_CONTROLLED`).
pub fn player_side(map: &Map, g: Guid) -> bool {
    guid::is_player(g) || crate::game::map::pets::owner_of(map, g).is_some_and(guid::is_player)
}

/// What a spell reaches beyond its range when caster and target both run
/// forward: mangos's `MELEE_LEEWAY`, for a melee spell or one at a player
/// (`GetMinMaxRange`; Thistle Tea's movement allowance).
fn moving_leeway(me: &combat::UnitView, u: &combat::UnitView, melee: bool, target: Guid) -> f32 {
    if me.running && u.running && (melee || guid::is_player(target)) {
        combat::MELEE_LEEWAY
    } else {
        0.0
    }
}

fn resurrects(s: &SpellTemplate) -> bool {
    s.effects.iter().any(|e| matches!(e.effect, 18 | 113))
}

/// Whether `s` may land on the dead: a resurrection, a spell that allows
/// a dead target, or skinning.
pub fn for_the_dead(s: &SpellTemplate) -> bool {
    resurrects(s) || s.attr(2, sd::ATTR_EX2_ALLOW_DEAD_TARGET) || s.has_effect(95)
}

/// The target's checks.
/// Whether every effect of `s` is a dispel at the unit picked, none an
/// area's.
fn dispels_only(s: &SpellTemplate) -> bool {
    let mut effects = s.effects.iter().filter(|e| e.effect != 0).peekable();
    effects.peek().is_some()
        && effects.all(|e| {
            e.effect == 38
                && e.radius_index == 0
                && matches!(
                    targets::implicit(e.target_a),
                    Implicit::Enemy | Implicit::Ally | Implicit::AnyUnit | Implicit::Caster
                )
        })
}

/// A cast charm's or possession's refusals (mangos-classic's `CheckCast`
/// for `SPELL_AURA_MOD_POSSESS` and `SPELL_AURA_MOD_CHARM`, Thistle Tea's
/// `PlayerPossession.validate`): not at oneself, nor at one charmed or
/// owned already, nor at one above the level its effect's points name
/// (Mind Control's rank).
fn charm_refused(
    map: &Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    level: u32,
) -> Option<u8> {
    use wow_world_messages::vanilla::SpellCastResult as R;
    let i = (0..3).find(|i| {
        s.effects[*i].effect == 6 && matches!(s.effects[*i].aura, AURA_MOD_POSSESS | AURA_MOD_CHARM)
    })?;
    if target == caster {
        return Some(fail::BAD_TARGETS);
    }
    if map
        .unit_fields(target)
        .is_some_and(|f| !f.get_guid(UNIT_FIELD_CHARMEDBY).is_zero())
    {
        return Some(R::Charmed.as_int());
    }
    if map.creature(target).is_some_and(|c| c.owner.is_some()) {
        return Some(R::CantBeCharmed.as_int());
    }
    let caster_level = map.unit_view(caster).map_or(1, |u| u.level);
    if i64::from(level) > i64::from(super::points(s, i, caster_level)) {
        return Some(fail::HIGHLEVEL);
    }
    None
}

fn check_target(
    map: &mut Map,
    caster: Guid,
    s: &SpellTemplate,
    t: &Targets,
    triggered: bool,
    m: &Mods,
) -> Option<u8> {
    let harmful = !auras::positive(s);
    let wants_unit = s.effects.iter().filter(|e| e.effect != 0).any(|e| {
        targets::wants_unit(e.target_a) || (e.target_a == 0 && matches!(e.effect, 18 | 113))
    });
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
            // Within range (a modifier can't take it below nothing), not too
            // close, and in sight, as mangos's `CheckRange` has a
            // destination: no combat reach.
            let (min, base) = map.realm.data.spells.range(s);
            let max = m.apply(mods::OP_RANGE, base).max(0.0);
            let far = dist(me.position, d);
            if base > 0.0 && far > max + 2.0 {
                return Some(fail::OUT_OF_RANGE);
            }
            if min > 0.0 && far < min {
                return Some(fail::TOO_CLOSE);
            }
            if !s.attr(2, sd::ATTR_EX2_IGNORE_LOS) && !line_of_sight(map, me.position, d) {
                return Some(fail::LINE_OF_SIGHT);
            }
        }
        // A spell at the caster's pet wants it out, alive, in range and in
        // sight, whatever unit the client named (mangos's `CheckCast` for
        // `TARGET_UNIT_CASTER_PET`, Thistle Tea's implicit pet range).
        if !triggered
            && s.effects
                .iter()
                .any(|e| e.effect != 0 && targets::implicit(e.target_a) == Implicit::Pet)
        {
            let Some(pet) = crate::game::map::pets::pet_of(map, caster) else {
                return Some(fail::NO_PET);
            };
            let (Some(me), Some(u)) = (map.unit_view(caster), map.unit_view(pet)) else {
                return Some(fail::NO_PET);
            };
            if !u.alive {
                return Some(fail::TARGETS_DEAD);
            }
            let (_, base) = map.realm.data.spells.range(s);
            let max = m.apply(mods::OP_RANGE, base).max(0.0);
            if base > 0.0 && dist(me.position, u.position) > max + me.combat_reach + u.combat_reach
            {
                return Some(fail::OUT_OF_RANGE);
            }
            if !s.attr(2, sd::ATTR_EX2_IGNORE_LOS) && !line_of_sight(map, me.position, u.position) {
                return Some(fail::LINE_OF_SIGHT);
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
    if resurrects(s) {
        if u.alive {
            return Some(fail::TARGET_NOT_DEAD);
        }
    } else if !u.alive && !for_the_dead(s) {
        return Some(fail::TARGETS_DEAD);
    }
    // A party member's spell is for the caster's party and pets, not the
    // caster (Thistle Tea's `validate_party_unit_query`).
    if s.effects
        .iter()
        .any(|e| e.effect != 0 && targets::implicit(e.target_a) == Implicit::PartyMember)
        && !targets::party_unit(map, caster, target)
    {
        return Some(fail::BAD_TARGETS);
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
        let attackable = if u.alive {
            combat::can_attack(map, caster, target)
        } else {
            combat::can_attack_dead(map, caster, target)
        };
        // A friend is TARGET_FRIENDLY; one who can't be attacked else (not
        // attackable, immune to the caster's side, a neutral of no war)
        // BAD_TARGETS, as mangos's harmful target filter and Thistle Tea's
        // `validate_target` have it.
        if (requires_enemy || harmful && !resurrects(s)) && !attackable && !s.has_effect(95) {
            return Some(if combat::friendly(map, caster, target) {
                fail::TARGET_FRIENDLY
            } else {
                fail::BAD_TARGETS
            });
        }
        // A player's harmful spell at a player's side it can't see (in
        // stealth, invisible), as mangos's `CheckCast` refuses it.
        if harmful
            && map.players.contains_key(&caster)
            && player_side(map, target)
            && !super::stealth::sees(map, caster, target)
        {
            return Some(fail::BAD_TARGETS);
        }
        if requires_friend && combat::can_attack(map, caster, target) {
            return Some(fail::TARGET_ENEMY);
        }
        if requires_friend && !combat::can_assist(map, caster, target) {
            return Some(fail::BAD_TARGETS);
        }
    } else if s
        .effects
        .iter()
        .any(|e| matches!(targets::implicit(e.target_a), Implicit::Enemy))
        && !triggered
        && map.players.contains_key(&caster)
    {
        // A spell at an enemy isn't cast at oneself (mangos's harmful target
        // filter: one can't attack oneself).
        return Some(fail::BAD_TARGETS);
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
    if !triggered && let Some(e) = charm_refused(map, caster, target, s, u.level) {
        return Some(e);
    }
    // A spell for the peaceful (Sap, Distract, Duel) refuses another in a
    // fight (mangos-classic's `CheckCast`: `SPELL_ATTR_EX_ONLY_PEACEFUL_TARGETS`
    // and `IsInCombat`, `SPELL_FAILED_TARGET_AFFECTING_COMBAT`).
    if target != caster
        && !triggered
        && s.attr(1, sd::ATTR_EX_ONLY_PEACEFUL_TARGETS)
        && map.unit_fields(target).is_some_and(|f| {
            f.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            )
        })
    {
        return Some(fail::TARGET_AFFECTING_COMBAT);
    }
    if s.target_aura_state != 0 && !aura_state(map, target, s.target_aura_state) {
        return Some(fail::TARGET_AURASTATE);
    }
    if let Some(e) = super::class_scripts::check_target(map, caster, s, target) {
        return Some(e);
    }
    // A spell that is all dispels at the unit (Dispel Magic, Cleanse, not
    // Shield Slam's nor Abolish Poison's, not an area's) wants something on
    // it for one of them to take (mangos's classic-only `CheckCast` check,
    // Thistle Tea's `nothing_to_dispel`).
    // A player's drain or burn of a power the target hasn't (Mana Burn at
    // a warrior), as mangos's `CheckCast` refuses it.
    if target != caster && map.players.contains_key(&caster) {
        let power = map
            .unit_fields(target)
            .map_or(0, |f| i32::from(f.get_byte(UNIT_FIELD_BYTES_0, 3)));
        if s.effects
            .iter()
            .any(|e| matches!(e.effect, 8 | 62) && e.misc_value != power)
        {
            return Some(fail::BAD_TARGETS);
        }
    }
    if !triggered && dispels_only(s) {
        let any =
            s.effects.iter().filter(|e| e.effect != 0).any(|e| {
                !super::effects::dispellable(map, caster, target, e.misc_value).is_empty()
            });
        if !any {
            return Some(fail::NOTHING_TO_DISPEL);
        }
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
    // A helpful spell whose every effect the target is immune to by its
    // mechanic (a shield on a Weakened Soul) is refused before it costs
    // anything, as mangos's `CheckCast` has it.
    if !harmful
        && !triggered
        && auras::auras(map, target).is_some_and(|a| {
            let mut effects = s.effects.iter().filter(|e| e.effect != 0).peekable();
            effects.peek().is_some()
                && effects.all(|e| {
                    let mechanic = if e.mechanic != 0 {
                        e.mechanic
                    } else {
                        s.mechanic
                    };
                    mechanic != 0 && a.mechanic_immune(mechanic)
                })
        })
    {
        return Some(fail::TARGET_AURASTATE);
    }
    if target != caster && !triggered {
        let me = map.unit_view(caster)?;
        let (min, max) = map.realm.data.spells.range(s);
        let max = m.apply(mods::OP_RANGE, max).max(0.0);
        let d = dist(me.position, u.position);
        let melee = s.melee_or_ranged() && max <= 5.0;
        let reach = if s.on_next_swing() {
            // A strike on the next swing is queued from afar, its swing to
            // reach (mangos's `GetMinMaxRange`, strict: a hundred yards).
            100.0
        } else if melee {
            combat::melee_range(&me, &u)
        } else {
            max + me.combat_reach + u.combat_reach
        };
        if d > reach + 1.0 + moving_leeway(&me, &u, melee, target) {
            return Some(fail::OUT_OF_RANGE);
        }
        // The minimum beyond both reaches, as the maximum is (mangos's
        // `GetMinMaxRange`, Thistle Tea's `Range`): a hunter's dead zone is
        // from its target's edge.
        if min > 0.0 && d < min + me.combat_reach + u.combat_reach {
            return Some(fail::TOO_CLOSE);
        }
        if s.from_behind() && facing(map, target, caster, std::f32::consts::PI) {
            return Some(fail::NOT_BEHIND);
        }
        // Gouge's kind: the target must face the caster.
        if s.attributes[0] == TARGET_FACING_CASTER
            && !facing(map, target, caster, std::f32::consts::PI)
        {
            return Some(fail::NOT_INFRONT);
        }
        // What must be cast forward, as Thistle Tea's `Facing` has it: a
        // melee-range spell, and a player's spell vmangos marks to face its
        // target or an auto-repeat one; within half a circle in front, or
        // close enough to overlap.
        let forward = !triggered
            && !s.on_next_swing()
            && (s.range_index == RANGE_MELEE
                || (map.players.contains_key(&caster)
                    && (s.custom(sd::CUSTOM_FACE_TARGET) || s.auto_repeat())));
        if forward
            && dist(me.position, u.position) >= OVERLAP_DISTANCE
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

/// `SPELL_ATTR_EX_IMMUNITY_PURGES_EFFECT`: the spell's immunities punch
/// through what they guard against.
const ATTR_EX_IMMUNITY_PURGES_EFFECT: u32 = 0x8000;

/// Whether `s` frees its caster from effect `x` of aura `h` (a stun, a
/// silence, a fear): mangos-classic's `SpellCancelsAuraEffect`. The spell
/// must purge what it is immune to, and have an immunity to the aura's
/// kind, its school, its dispel type or its mechanic (the spell's or the
/// effect's); an aura that ignores immunities holds anyway. Divine Shield
/// and Ice Block (every school) free from any; Blessing of Protection
/// (physical) from Kidney Shot, not Hammer of Justice's holy stun or a
/// fear; Blink (stun and root) from a stun, not a silence.
pub fn cancels(
    data: &crate::game::data::GameData,
    s: &SpellTemplate,
    h: &auras::Holder,
    x: &auras::Aura,
) -> bool {
    if !s.attr(1, ATTR_EX_IMMUNITY_PURGES_EFFECT) {
        return false;
    }
    let Some(t) = data.spells.get(h.spell) else {
        return false;
    };
    if t.attr(0, sd::ATTR_NO_IMMUNITIES) {
        return false;
    }
    let effect_mechanic = t
        .effects
        .get(usize::from(x.effect))
        .map_or(0, |e| e.mechanic);
    s.effects.iter().filter(|e| e.effect == 6).any(|e| {
        let misc = e.misc_value as u32;
        match e.aura {
            AURA_STATE_IMMUNITY => misc == x.kind,
            AURA_SCHOOL_IMMUNITY => {
                !t.attr(2, sd::ATTR_EX2_NO_SCHOOL_IMMUNITIES) && t.school_mask() & misc != 0
            }
            AURA_DISPEL_IMMUNITY => misc == t.dispel,
            AURA_MECHANIC_IMMUNITY => misc != 0 && (misc == t.mechanic || misc == effect_mechanic),
            _ => false,
        }
    })
}
