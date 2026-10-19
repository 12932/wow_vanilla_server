//! Spell modifiers: talents and auras that change a class's spells
//! (`SPELL_AURA_ADD_FLAT_MODIFIER` and `SPELL_AURA_ADD_PCT_MODIFIER`). Each
//! names an operation (its misc value) and the spells it touches: those of
//! its spell's family whose family flags meet its mask (`EffectItemType`).
//! A modifier with charges is spent by a cast it changes (Clearcasting,
//! Presence of Mind, Nature's Swiftness).

use wow_world_messages::Guid;

use crate::game::data::spells::{self as sd, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::{self, AURA_ADD_FLAT_MODIFIER, AURA_ADD_PCT_MODIFIER};

// 1.12's `SpellModOp` (mangos-classic's numbers, Thistle Tea's
// `Modifiers`): 3, 12 and 23 change an aura's attack power, speed and
// haste, where TBC's change an effect's points by its index.
pub const OP_DAMAGE: usize = 0;
pub const OP_DURATION: usize = 1;
pub const OP_THREAT: usize = 2;
pub const OP_ATTACK_POWER: usize = 3;
pub const OP_CHARGES: usize = 4;
pub const OP_RANGE: usize = 5;
pub const OP_RADIUS: usize = 6;
pub const OP_CRITICAL_CHANCE: usize = 7;
pub const OP_ALL_EFFECTS: usize = 8;
pub const OP_NOT_LOSE_CASTING_TIME: usize = 9;
pub const OP_CASTING_TIME: usize = 10;
pub const OP_COOLDOWN: usize = 11;
pub const OP_SPEED: usize = 12;
pub const OP_COST: usize = 14;
pub const OP_CRIT_DAMAGE_BONUS: usize = 15;
pub const OP_RESIST_MISS_CHANCE: usize = 16;
pub const OP_JUMP_TARGETS: usize = 17;
pub const OP_CHANCE_OF_SUCCESS: usize = 18;
pub const OP_ACTIVATION_TIME: usize = 19;
pub const OP_EFFECT_PAST_FIRST: usize = 20;
pub const OP_GLOBAL_COOLDOWN: usize = 21;
pub const OP_DOT: usize = 22;
pub const OP_HASTE: usize = 23;
pub const OP_SPELL_BONUS_DAMAGE: usize = 24;
pub const OP_MULTIPLE_VALUE: usize = 27;
pub const OP_RESIST_DISPEL_CHANCE: usize = 28;
pub const OPS: usize = 29;

pub const SMSG_SET_FLAT_SPELL_MODIFIER: u16 = 0x266;
pub const SMSG_SET_PCT_SPELL_MODIFIER: u16 = 0x267;

/// The modifiers on one spell from one caster.
#[derive(Debug, Clone, Default)]
pub struct Mods {
    pub flat: [i32; OPS],
    pub pct: [i32; OPS],
    /// Holders with charges whose modifiers this spell uses: (spell,
    /// caster, operation).
    pub charged: Vec<(u32, Guid, usize)>,
}

impl Mods {
    /// `base` with operation `op`'s flat and percent modifiers.
    pub fn apply(&self, op: usize, base: f32) -> f32 {
        (base + self.flat[op] as f32) * (100 + self.pct[op]).max(0) as f32 / 100.0
    }

    pub fn apply_i(&self, op: usize, base: i32) -> i32 {
        self.apply(op, base as f32).round() as i32
    }

    pub fn apply_ms(&self, op: usize, base: u32) -> u32 {
        self.apply(op, base as f32).max(0.0) as u32
    }

    /// The chance, in percent, that an aura these modifiers (its
    /// caster's) hold on resists a dispel: the flat chance by the percent,
    /// from none to certain.
    pub fn dispel_resistance(&self) -> f32 {
        self.apply(OP_RESIST_DISPEL_CHANCE, 0.0).clamp(0.0, 100.0)
    }

    pub fn any(&self, op: usize) -> bool {
        self.flat[op] != 0 || self.pct[op] != 0
    }

    /// Effect `index`'s points with all effects' modifiers, then those to
    /// its aura's speed, haste or attack power (mangos's
    /// `CalculateAuraEffectValue`).
    pub fn points(&self, s: &SpellTemplate, index: usize, points: i32) -> i32 {
        let v = self.apply(OP_ALL_EFFECTS, points as f32);
        match aura_op(s.effects[index].aura) {
            Some(op) => self.apply(op, v),
            None => v,
        }
        .round() as i32
    }
}

/// The operation that changes an aura of `kind`'s amount: its speed, its
/// haste or its attack power.
fn aura_op(kind: u32) -> Option<usize> {
    use crate::game::map::auras::*;
    match kind {
        AURA_MOD_INCREASE_SPEED | AURA_MOD_DECREASE_SPEED | AURA_MOD_INCREASE_SWIM_SPEED => {
            Some(OP_SPEED)
        }
        AURA_MOD_ATTACKSPEED
        | AURA_MOD_CASTING_SPEED
        | AURA_MOD_HASTE
        | AURA_MOD_RANGED_HASTE
        | AURA_MOD_RANGED_AMMO_HASTE => Some(OP_HASTE),
        AURA_MOD_ATTACK_POWER
        | AURA_MOD_ATTACK_POWER_PCT
        | AURA_MOD_RANGED_ATTACK_POWER
        | AURA_MOD_RANGED_ATTACK_POWER_PCT => Some(OP_ATTACK_POWER),
        _ => None,
    }
}

/// Whether a modifier with `mask` from a spell of `family` touches `s`.
pub fn touches(family: u32, mask: u64, s: &SpellTemplate) -> bool {
    family != 0 && family == s.family_name && mask & s.family_flags != 0
}

/// Whose modifiers `caster`'s spells take: a player's pet's or totem's are
/// its master's (mangos's `GetSpellModOwner`, Thistle Tea's pet's
/// `owner_spell_modifiers`); a charmed creature's are its own.
fn mod_owner(map: &Map, caster: Guid) -> Guid {
    use crate::game::map::pets::Kind;
    map.creature(caster)
        .and_then(|c| {
            let kind = c.pet.as_ref()?.kind;
            let owner = c.owner.filter(|o| crate::game::guid::is_player(*o))?;
            (!matches!(kind, Kind::Charmed | Kind::Possessed)).then_some(owner)
        })
        .unwrap_or(caster)
}

/// The modifiers `caster`'s auras (its master's, for a pet) put on `s`.
pub fn for_spell(map: &Map, caster: Guid, s: &SpellTemplate) -> Mods {
    let mut m = Mods::default();
    if s.attr(3, sd::ATTR_EX3_IGNORE_CASTER_MODIFIERS) || s.family_name == 0 {
        return m;
    }
    let data = &map.realm.data;
    let bearer = mod_owner(map, caster);
    let Some(a) = auras::auras(map, bearer) else {
        return m;
    };
    for h in &a.holders {
        let Some(family) = data.spells.get(h.spell).map(|t| t.family_name) else {
            continue;
        };
        for x in &h.effects {
            if !matches!(x.kind, AURA_ADD_FLAT_MODIFIER | AURA_ADD_PCT_MODIFIER) {
                continue;
            }
            let op = x.misc.clamp(0, OPS as i32 - 1) as usize;
            let used = used_by(op, s, data);
            // Damage's modifier holds for a script's amount too (a dummy's),
            // but only a direct amount spends its charge.
            if !touches(family, x.mask, s) || (!used && op != OP_DAMAGE) {
                continue;
            }
            let v = x.amount * h.stacks.max(1) as i32;
            if x.kind == AURA_ADD_FLAT_MODIFIER {
                m.flat[op] += v;
            } else {
                m.pct[op] += v;
            }
            if used
                && bearer == caster
                && h.charges > 0
                && !m.charged.iter().any(|c| c.0 == h.spell && c.1 == h.caster)
            {
                m.charged.push((h.spell, h.caster, op));
            }
        }
    }
    m
}

/// Whether operation `op` changes anything about `s` (a charge is spent
/// only on a spell it changes).
fn used_by(op: usize, s: &SpellTemplate, data: &crate::game::data::GameData) -> bool {
    match op {
        OP_COST => s.mana_cost > 0 || s.mana_cost_per_level > 0 || s.mana_cost_percentage > 0,
        OP_CASTING_TIME => data.spells.cast_time(s) > 0,
        OP_COOLDOWN => s.recovery_time > 0 || s.category_recovery_time > 0,
        OP_CRITICAL_CHANCE | OP_CRIT_DAMAGE_BONUS => s
            .effects
            .iter()
            .any(|e| matches!(e.effect, 2 | 9 | 10 | 17 | 31 | 58 | 121)),
        // A tick of damage or healing, as the amount over time takes it
        // (`effects::aura_amount`; mangos's `SPELLMOD_DOT` in the damage
        // and healing done): not a mana leech's or a trigger's.
        OP_DOT => s.effects.iter().any(|e| {
            e.amplitude > 0
                && matches!(
                    e.aura,
                    auras::AURA_PERIODIC_DAMAGE
                        | auras::AURA_PERIODIC_LEECH
                        | auras::AURA_PERIODIC_HEAL
                )
        }),
        // A direct amount: damage, a heal, a drain or a burn, not a gain of
        // power or a tick (Thistle Tea's `operation_used_by_spell?(:damage)`).
        OP_DAMAGE => s.effects.iter().any(|e| {
            matches!(e.effect, 2 | 8 | 9 | 10 | 17 | 31 | 58 | 62 | 121)
                || (e.effect == 6 && e.aura == auras::AURA_POWER_BURN_MANA)
        }),
        // A periodic aura's interval (Thistle Tea's
        // `operation_used_by_spell?(:activation_time)`).
        OP_ACTIVATION_TIME => s
            .effects
            .iter()
            .any(|e| auras::ticks(e.aura) && auras::interval(e, (0, 0)).is_some()),
        OP_SPEED | OP_HASTE | OP_ATTACK_POWER => {
            s.effects.iter().any(|e| aura_op(e.aura) == Some(op))
        }
        // Charges are an aura's (mangos asks for them only as its holder is
        // made).
        OP_CHARGES => s
            .effects
            .iter()
            .any(|e| matches!(e.effect, 6 | 35) && e.aura != 0),
        _ => true,
    }
}

/// Spend a charge of every modifier with charges `mods` used.
pub fn spend(map: &mut Map, caster: Guid, mods: &Mods) {
    for (spell, by, _) in &mods.charged {
        auras::spend_charge(map, caster, *spell, *by);
    }
}

/// Tell a player's client the modifiers their auras give: for each of the
/// family mask's 64 bits, the sum of each operation's flat and percent
/// values.
pub fn send(map: &Map, player: Guid) {
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let mut flat = [[0i32; OPS]; 64];
    let mut pct = [[0i32; OPS]; 64];
    for (h, x) in p.auras.effects() {
        if !matches!(x.kind, AURA_ADD_FLAT_MODIFIER | AURA_ADD_PCT_MODIFIER) {
            continue;
        }
        let op = x.misc.clamp(0, OPS as i32 - 1) as usize;
        for bit in 0..64 {
            if x.mask & (1 << bit) != 0 {
                let v = x.amount * h.stacks.max(1) as i32;
                if x.kind == AURA_ADD_FLAT_MODIFIER {
                    flat[bit][op] += v;
                } else {
                    pct[bit][op] += v;
                }
            }
        }
    }
    for (table, opcode) in [
        (&flat, SMSG_SET_FLAT_SPELL_MODIFIER),
        (&pct, SMSG_SET_PCT_SPELL_MODIFIER),
    ] {
        for (bit, row) in table.iter().enumerate() {
            for (op, v) in row.iter().enumerate().filter(|(_, v)| **v != 0) {
                let mut b = crate::game::data::items::Body::default();
                b.u8(bit as u8).u8(op as u8).u32(*v as u32);
                p.outbound.send_frame(b.frame(opcode));
            }
        }
    }
}

/// A modifier aura came or went: the client hears every bit it covered
/// again, with zeros where nothing is left.
pub fn changed(map: &Map, player: Guid, mask: u64, op: usize, pct: bool) {
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let kind = if pct {
        AURA_ADD_PCT_MODIFIER
    } else {
        AURA_ADD_FLAT_MODIFIER
    };
    for bit in 0..64 {
        if mask & (1 << bit) == 0 {
            continue;
        }
        let v: i32 = p
            .auras
            .effects()
            .filter(|(_, x)| x.kind == kind && x.misc == op as i32 && x.mask & (1 << bit) != 0)
            .map(|(h, x)| x.amount * h.stacks.max(1) as i32)
            .sum();
        let mut b = crate::game::data::items::Body::default();
        b.u8(bit as u8).u8(op as u8).u32(v as u32);
        p.outbound.send_frame(b.frame(if pct {
            SMSG_SET_PCT_SPELL_MODIFIER
        } else {
            SMSG_SET_FLAT_SPELL_MODIFIER
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_then_percent() {
        let mut m = Mods::default();
        m.flat[OP_CASTING_TIME] = -500;
        m.pct[OP_CASTING_TIME] = -10;
        assert_eq!(m.apply_ms(OP_CASTING_TIME, 3500), 2700);
        m.pct[OP_ALL_EFFECTS] = 10;
        assert_eq!(m.points(&SpellTemplate::default(), 0, 100), 110);
        let fireball = SpellTemplate {
            family_name: 3,
            family_flags: 0x4000_0001,
            ..Default::default()
        };
        assert!(touches(3, 1, &fireball));
        assert!(!touches(3, 2, &fireball));
        assert!(!touches(4, 1, &fireball));
    }
}
