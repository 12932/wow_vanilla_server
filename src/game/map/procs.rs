//! Procs: auras that go off when their unit hits or is hit. Melee swings,
//! ranged shots and spells report what happened through [`trigger`].

use wow_world_messages::Guid;

use crate::game::map::Map;

// `procFlags`: what happened, to the one who did it or had it done.
pub const PROC_KILLED: u32 = 0x1;
pub const PROC_KILL: u32 = 0x2;
pub const PROC_DONE_MELEE: u32 = 0x4;
pub const PROC_TAKEN_MELEE: u32 = 0x8;
pub const PROC_DONE_MELEE_SPELL: u32 = 0x10;
pub const PROC_TAKEN_MELEE_SPELL: u32 = 0x20;
pub const PROC_DONE_RANGED: u32 = 0x40;
pub const PROC_TAKEN_RANGED: u32 = 0x80;
pub const PROC_DONE_RANGED_SPELL: u32 = 0x100;
pub const PROC_TAKEN_RANGED_SPELL: u32 = 0x200;
pub const PROC_DONE_POSITIVE_AOE: u32 = 0x400;
pub const PROC_TAKEN_POSITIVE_AOE: u32 = 0x800;
pub const PROC_DONE_AOE: u32 = 0x1000;
pub const PROC_TAKEN_AOE: u32 = 0x2000;
pub const PROC_DONE_POSITIVE_SPELL: u32 = 0x4000;
pub const PROC_TAKEN_POSITIVE_SPELL: u32 = 0x8000;
pub const PROC_DONE_NEGATIVE_SPELL: u32 = 0x1_0000;
pub const PROC_TAKEN_NEGATIVE_SPELL: u32 = 0x2_0000;
pub const PROC_DONE_PERIODIC: u32 = 0x4_0000;
pub const PROC_TAKEN_PERIODIC: u32 = 0x8_0000;
pub const PROC_TAKEN_DAMAGE: u32 = 0x10_0000;
pub const PROC_TRAP: u32 = 0x20_0000;
pub const PROC_DONE_OFFHAND: u32 = 0x80_0000;

// `procEx`: how it went.
pub const EX_NORMAL_HIT: u32 = 0x1;
pub const EX_CRITICAL_HIT: u32 = 0x2;
pub const EX_MISS: u32 = 0x4;
pub const EX_RESIST: u32 = 0x8;
pub const EX_DODGE: u32 = 0x10;
pub const EX_PARRY: u32 = 0x20;
pub const EX_BLOCK: u32 = 0x40;
pub const EX_EVADE: u32 = 0x80;
pub const EX_IMMUNE: u32 = 0x100;
pub const EX_DEFLECT: u32 = 0x200;
pub const EX_ABSORB: u32 = 0x400;
pub const EX_REFLECT: u32 = 0x800;
pub const EX_INTERRUPT: u32 = 0x1000;

/// What happened between two units, for their procs.
#[derive(Debug, Clone, Copy, Default)]
pub struct Event {
    pub actor: Guid,
    pub target: Guid,
    /// `procFlags` on the actor's side, and on the target's.
    pub actor_flags: u32,
    pub target_flags: u32,
    /// `procEx`.
    pub ex: u32,
    /// The spell that did it; `None` for a melee swing or a shot.
    pub spell: Option<u32>,
    pub damage: u32,
    /// The school, as a mask.
    pub school: u32,
    /// The off hand's swing.
    pub offhand: bool,
}

/// Run the procs of both units for `e`.
pub fn trigger(map: &mut Map, e: Event) {
    // Weapons' enchantments and chance-on-hit spells.
    crate::game::map::enchant::weapon_procs(map, &e);
}
