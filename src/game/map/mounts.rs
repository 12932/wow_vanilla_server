//! Mounting's rules, as Thistle Tea's `Spell.Mount` has them:
//!
//! - no mount on a ship's deck indoors, nor in Booty Bay, nor in a dungeon
//!   or raid (save Zul'Farrak, the Caverns of Time, Zul'Gurub and the Ruins
//!   of Ahn'Qiraj) unless the spell names its area, as the Qiraji battle
//!   tanks name the Temple of Ahn'Qiraj;
//! - no mount in a shapeshift form but the warrior's stances and Stealth,
//!   nor while transformed into a model that can't ride;
//! - nothing that breaks under water is begun swimming;
//! - the Black Qiraji Battle Tank's summons is the tank anywhere, and its
//!   Temple-only kind in a dungeon; cast mounted it dismounts;
//! - Reindeer Transformation turns a mount into a reindeer of its speed,
//!   and needs one.

use wow_world_messages::Guid;

use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, AURA_MOUNTED, INTERRUPT_UNDER_WATER};

pub const REINDEER_TRANSFORMATION: u32 = 25860;
const REINDEER: u32 = 25858;
const SWIFT_REINDEER: u32 = 25859;
pub const BLACK_QIRAJI: u32 = 26656;
const BLACK_QIRAJI_TANK: u32 = 26655;
const BLACK_QIRAJI_TANK_TEMPLE: u32 = 25863;
const AHN_QIRAJ_TEMPLE: u32 = 531;
/// The dungeons ridden in.
const RIDDEN_DUNGEONS: [u32; 4] = [209, 269, 309, 509];
const BOOTY_BAY: u32 = 35;
/// The forms a mount is had in: none, the warrior's stances, Shadowform
/// and Stealth.
const MOUNT_FORMS: [u8; 6] = [0, 17, 18, 19, 28, 30];
const DEFAULT_RUN_SPEED: f32 = 7.0;

pub const NO_MOUNTS_ALLOWED: u8 = 0x4b;
pub const ONLY_ABOVEWATER: u8 = 0x50;
pub const ONLY_MOUNTED: u8 = 0x53;
const NOT_SHAPESHIFT: u8 = 0x3d;

/// A spell that mounts.
pub fn mount_spell(s: &SpellTemplate) -> bool {
    s.id == BLACK_QIRAJI || s.has_aura(AURA_MOUNTED)
}

/// Whether mounts are ridden on this map.
pub fn mounts_allowed(map: &Map) -> bool {
    let kind = crate::game::instances::instance_type(&map.realm, map.key.map);
    !matches!(kind, 1 | 2) || RIDDEN_DUNGEONS.contains(&map.key.map)
}

fn mounted(map: &Map, g: Guid) -> bool {
    auras::auras(map, g).is_some_and(|a| a.has(AURA_MOUNTED))
}

pub fn dismount(map: &mut Map, g: Guid) {
    auras::remove_where(map, g, |h| h.has(AURA_MOUNTED));
}

/// The Black Qiraji Battle Tank cast mounted only dismounts: whether that
/// was all.
pub fn dismissed(map: &mut Map, caster: Guid, s: &SpellTemplate) -> bool {
    if s.id != BLACK_QIRAJI || !map.players.contains_key(&caster) || !mounted(map, caster) {
        return false;
    }
    dismount(map, caster);
    true
}

/// Why a player may not begin `s` here, as a mount.
pub fn check(map: &mut Map, caster: Guid, s: &SpellTemplate, triggered: bool) -> Option<u8> {
    let p = map.players.get(&caster)?;
    if s.id == REINDEER_TRANSFORMATION && !p.auras.has(AURA_MOUNTED) {
        return Some(ONLY_MOUNTED);
    }
    let swimming = p.movement.flags.get_swimming().is_some();
    if swimming && (s.id == BLACK_QIRAJI || s.aura_interrupt_flags & INTERRUPT_UNDER_WATER != 0) {
        return Some(ONLY_ABOVEWATER);
    }
    if !mount_spell(s) {
        return None;
    }
    let on_deck = p.movement.flags.get_on_transport().is_some();
    let at = p.position();
    let form = p.fields.get_byte(UNIT_FIELD_BYTES_1, 2);
    let (display, native) = (
        p.fields.get_u32(UNIT_FIELD_DISPLAYID),
        p.fields.get_u32(UNIT_FIELD_NATIVEDISPLAYID),
    );
    if on_deck
        && (s.id == BLACK_QIRAJI
            || crate::game::map::spells::environment::outdoors(map, caster) != Some(true))
    {
        return Some(NO_MOUNTS_ALLOWED);
    }
    if crate::game::map::world::zone_and_area(map, at).is_some_and(|(_, area)| area == BOOTY_BAY) {
        return Some(NO_MOUNTS_ALLOWED);
    }
    let data = &map.realm.data;
    if !mounts_allowed(map)
        && !triggered
        && crate::game::map::spells::area::required(data, s.id) == 0
        && !(s.id == BLACK_QIRAJI && map.key.map == AHN_QIRAJ_TEMPLE)
    {
        return Some(NO_MOUNTS_ALLOWED);
    }
    if !MOUNT_FORMS.contains(&form) {
        return Some(NOT_SHAPESHIFT);
    }
    if display != native && !data.display_info.get(&display).is_some_and(|d| d.can_mount) {
        return Some(NOT_SHAPESHIFT);
    }
    None
}

/// `SPELL_EFFECT_SCRIPT_EFFECT` of the Black Qiraji Battle Tank's summons:
/// off any mount, onto the tank, or in a dungeon its Temple-only kind.
pub fn summon_qiraji(map: &mut Map, caster: Guid) {
    dismount(map, caster);
    let tank = if mounts_allowed(map) {
        BLACK_QIRAJI_TANK
    } else {
        BLACK_QIRAJI_TANK_TEMPLE
    };
    crate::game::map::spells::cast_triggered(map, caster, tank, Some(caster));
}

/// Reindeer Transformation: the mount gives way to a reindeer, swift if
/// it was.
pub fn reindeer(map: &mut Map, caster: Guid) {
    let Some(p) = map.players.get(&caster) else {
        return;
    };
    if !p.auras.has(AURA_MOUNTED) {
        return;
    }
    let swift = p.speeds.run >= 2.0 * DEFAULT_RUN_SPEED;
    dismount(map, caster);
    let reindeer = if swift { SWIFT_REINDEER } else { REINDEER };
    crate::game::map::spells::cast_triggered(map, caster, reindeer, Some(caster));
}
