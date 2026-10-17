//! The combat log's spell packets Thistle Tea sends: what an effect did
//! (`SMSG_SPELLLOGEXECUTE`: a drain, extra attacks, an interrupt, a
//! durability loss), a dispel's work and its failures, a tick the target
//! was immune to, a self-inflicted instant kill, and a spell's rank that
//! takes another's place.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::items::Body;
use crate::game::map::Map;

/// `SMSG_SUPERCEDED_SPELL`.
pub const SMSG_SUPERCEDED_SPELL: u16 = 0x012C;

fn broadcast<M: wow_world_messages::vanilla::ServerMessage>(map: &mut Map, around: Guid, m: &M) {
    if let Some(frame) = crate::game::conn::encode(m) {
        map.send_to_observers(around, &frame, true);
    }
}

/// What one effect of `spell` did, for the combat log.
pub fn execute(map: &mut Map, caster: Guid, spell: u32, effect: SpellLog_SpellEffect) {
    broadcast(
        map,
        caster,
        &SMSG_SPELLLOGEXECUTE {
            caster,
            spell,
            logs: vec![SpellLog { effect }],
        },
    );
}

/// A drain of `amount` of `power` from `target`.
pub fn power_drain(
    map: &mut Map,
    caster: Guid,
    spell: u32,
    target: Guid,
    amount: u32,
    power: u16,
    multiplier: f32,
) {
    let power = Power::try_from(power as u8).unwrap_or(Power::Mana);
    execute(
        map,
        caster,
        spell,
        SpellLog_SpellEffect::PowerDrain {
            amount,
            multiplier,
            power,
            target1: target,
        },
    );
}

/// `count` extra attacks for `target`.
pub fn extra_attacks(map: &mut Map, caster: Guid, spell: u32, target: Guid, count: u32) {
    execute(
        map,
        caster,
        spell,
        SpellLog_SpellEffect::AddExtraAttacks {
            extra_attacks: count,
            target4: target,
        },
    );
}

/// `target`'s cast of `interrupted` was cut short.
pub fn interrupt(map: &mut Map, caster: Guid, spell: u32, target: Guid, interrupted: u32) {
    execute(
        map,
        caster,
        spell,
        SpellLog_SpellEffect::InterruptCast {
            interrupted_spell: interrupted,
            target5: target,
        },
    );
}

/// `target`'s item `entry` lost durability.
pub fn durability(map: &mut Map, caster: Guid, spell: u32, target: Guid, entry: u32) {
    execute(
        map,
        caster,
        spell,
        SpellLog_SpellEffect::DurabilityDamage {
            item_to_damage: entry,
            target6: target,
            unknown5: u32::MAX,
        },
    );
}

/// `SMSG_SPELLDISPELLOG`: what `caster`'s dispel took from `victim`.
pub fn dispelled(map: &mut Map, caster: Guid, victim: Guid, spells: Vec<u32>) {
    if spells.is_empty() {
        return;
    }
    broadcast(
        map,
        caster,
        &SMSG_SPELLDISPELLOG {
            victim,
            caster,
            spells,
        },
    );
}

/// `SMSG_DISPEL_FAILED`: what `victim`'s auras held against the dispel.
pub fn dispel_failed(map: &mut Map, caster: Guid, victim: Guid, spells: Vec<u32>) {
    if spells.is_empty() {
        return;
    }
    broadcast(
        map,
        caster,
        &SMSG_DISPEL_FAILED {
            caster,
            target: victim,
            spells,
        },
    );
}

/// `SMSG_SPELLORDAMAGE_IMMUNE`: `target` shrugged off `spell`.
pub fn immune(map: &mut Map, caster: Guid, target: Guid, spell: u32) {
    broadcast(
        map,
        target,
        &SMSG_SPELLORDAMAGE_IMMUNE {
            caster,
            target,
            id: spell,
            debug_log_format: false,
        },
    );
}

/// `SMSG_SPELLINSTAKILLLOG`: `target` killed itself with `spell` (the
/// client logs others' kills from the cast).
pub fn instakill(map: &mut Map, target: Guid, spell: u32) {
    broadcast(map, target, &SMSG_SPELLINSTAKILLLOG { target, spell });
}

/// `SMSG_SUPERCEDED_SPELL`: `new` takes `old`'s place on the player's bars,
/// as mangos-zero writes it (the old rank first).
pub fn superceded(map: &Map, player: Guid, old: u32, new: u32) {
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let mut b = Body::default();
    b.u16(old as u16).u16(new as u16);
    p.outbound.send_frame(b.frame(SMSG_SUPERCEDED_SPELL));
}
