//! Auras that end the moment something happens, as Thistle Tea's aura
//! transition (and vmangos's interrupt flags) end them: mounting ends what
//! mounting interrupts and dismounting what needs a mount; stealth,
//! invisibility and a believed Feign Death end what losing sight of the
//! unit does; invulnerability ends what it does (a flag, silithyst);
//! leaving the map and teleporting end theirs; taking a flight ends
//! stealth, a form that can't fly, and the mount; and Discombobulate's
//! transform ends the mount.

use wow_world_messages::Guid;

use crate::game::map::Map;
use crate::game::map::auras::{self, *};

/// Flags vmangos's `AuraInterruptFlags` give these moments.
pub const IMMUNE_OR_LOST_SELECTION: u32 = 0x10_0000;
pub const INVULNERABLE: u32 = 0x20_0000;

/// The Discombobulator Ray's transform.
const DISCOMBOBULATE: u32 = 4060;

/// Forms a flight keeps: none, the warrior's stances, and the rogue's
/// stealth form and the like.
const FLIGHT_FORMS: [u8; 6] = [0, 17, 18, 19, 28, 30];

/// End `target`'s holders with any of `flags`, but `except`'s.
fn end(map: &mut Map, target: Guid, flags: u32, except: u32) {
    let any = auras::auras(map, target).is_some_and(|a| {
        a.holders
            .iter()
            .any(|h| h.spell != except && h.interrupt_flags & flags != 0)
    });
    if any {
        auras::remove_where(map, target, |h| {
            h.spell != except && h.interrupt_flags & flags != 0
        });
    }
}

/// `target` mounted (with `mount`): its banked swings go too (Thistle
/// Tea's `MountSync` clearing extra attacks).
pub fn mounted(map: &mut Map, target: Guid, mount: u32) {
    if let Some(c) = map.combat_mut(target) {
        c.extra_attacks = 0;
        c.held_extra_attacks = 0;
    }
    end(map, target, INTERRUPT_MOUNTING, mount);
}

/// Discombobulate's transform (its first effect) knocks `target` off its
/// mount (`MountSync.dismount_on_transform`).
pub fn transformed(map: &mut Map, target: Guid, spell: u32, effect: u8) {
    if spell == DISCOMBOBULATE && effect == 0 {
        auras::remove_where(map, target, |h| h.has(AURA_MOUNTED));
    }
}

/// `target` is off its mount.
pub fn dismounted(map: &mut Map, target: Guid) {
    end(map, target, INTERRUPT_NOT_MOUNTED, 0);
}

/// `target` went from sight (stealth, invisibility, a believed Feign
/// Death) by `spell`.
pub fn concealed(map: &mut Map, target: Guid, spell: u32) {
    end(map, target, IMMUNE_OR_LOST_SELECTION, spell);
}

/// `target` became invulnerable by `spell`.
pub fn invulnerable(map: &mut Map, target: Guid, spell: u32) {
    end(map, target, INVULNERABLE, spell);
}

/// A player leaves the map.
pub fn changed_map(map: &mut Map, target: Guid) {
    end(map, target, INTERRUPT_CHANGE_MAP, 0);
}

/// A player is teleported, in the map or out of it.
pub fn teleported(map: &mut Map, target: Guid) {
    end(map, target, INTERRUPT_TELEPORTED, 0);
}

/// A player takes a flight (Thistle Tea's `Taxi.prepare_auras` and
/// `Mount.dismount`): out of stealth, out of a form that can't fly, off
/// the mount, and what mounting ends.
pub fn taking_flight(map: &mut Map, target: Guid) {
    let form = auras::form(map, target);
    let unfit = !FLIGHT_FORMS.contains(&form);
    auras::remove_where(map, target, |h| {
        h.has(AURA_MOD_STEALTH)
            || h.has(AURA_MOUNTED)
            || (unfit && (h.has(AURA_MOD_SHAPESHIFT) || h.has(AURA_TRANSFORM)))
    });
    end(map, target, INTERRUPT_MOUNTING, 0);
}
