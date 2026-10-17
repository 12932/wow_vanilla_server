//! The procs the spell data can't say, as Thistle Tea's `Aura.Script`,
//! `Aura.ProcSpell` and `Core.Class` script them: Ignite's growing burn,
//! Deep Wounds' bleed, mana returned (Master of Elements, Illumination,
//! Magic Absorption, Mana Drain), Pyroclasm's stun, Blessed Recovery's and
//! Persistent Shield's shares, Vampiric Embrace's healing, the judgements'
//! gifts to attackers, Combustion's charges, Sweeping Strikes and
//! Retaliation; and Lord Kazzak's Twisted Reflection and Mark of Kazzak.

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras::{self, Holder};
use crate::game::map::combat::{self, dist};
use crate::game::map::procs::{self, Event};

/// What a script made of a proc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Not a scripted proc: the aura's own effects go off.
    Unhandled,
    /// Done; a charge is spent as for any proc.
    Done,
    /// Done, and it spends its own charges (or none).
    DoneOwnCharges,
}

/// Ignite's talent ranks: each tick's share of a fire crit, in percent.
const IGNITE: &[(u32, i32)] = &[
    (11119, 4),
    (11120, 8),
    (12846, 12),
    (12847, 16),
    (12848, 20),
];
pub const IGNITE_BURN: u32 = 12654;
const IGNITE_MAX_STACKS: u8 = 5;
/// Deep Wounds' trigger spells and the share of the weapon's damage each
/// bleeds over its four ticks.
const DEEP_WOUNDS: &[(u32, u32)] = &[(12162, 20), (12850, 40), (12868, 60)];
const DEEP_WOUND_BLEED: u32 = 12721;
const DEEP_WOUND_TICKS: u32 = 4;
const MASTER_OF_ELEMENTS: &[u32] = &[29074, 29075, 29076];
const MASTER_OF_ELEMENTS_ENERGIZE: u32 = 29077;
const ILLUMINATION: &[u32] = &[20210, 20212, 20213, 20214, 20215];
const ILLUMINATION_ENERGIZE: u32 = 20272;
const MAGIC_ABSORPTION: &[u32] = &[29441, 29444, 29445, 29446, 29447];
const MAGIC_ABSORPTION_ENERGIZE: u32 = 29442;
const MANA_DRAIN: u32 = 27522;
const MANA_DRAIN_ENERGIZE: u32 = 29471;
const MANA_DRAIN_LEECH: u32 = 27526;
/// Pyroclasm's ranks and their chance over a whole Rain of Fire or
/// Hellfire.
const PYROCLASM: &[(u32, f32)] = &[(18096, 13.0), (18073, 26.0)];
const PYROCLASM_STUN: u32 = 18093;
const BLESSED_RECOVERY: &[(u32, u32)] = &[(27811, 27813), (27815, 27817), (27816, 27818)];
const PERSISTENT_SHIELD: u32 = 26467;
const PERSISTENT_SHIELD_ABSORB: u32 = 26470;
const VAMPIRIC_EMBRACE: u32 = 15286;
const VAMPIRIC_EMBRACE_HEAL: u32 = 15290;
/// The judgements on a target and what each gives the one who strikes it.
const JUDGEMENTS: &[(u32, u32)] = &[
    (20185, 20267),
    (20344, 20341),
    (20345, 20342),
    (20346, 20343),
    (20186, 20268),
    (20354, 20352),
    (20355, 20353),
];
const COMBUSTION: u32 = 11129;
const COMBUSTION_BUFF: u32 = 28682;
const SWEEPING_STRIKES: &[u32] = &[12292, 18765];
const SWEEPING_STRIKES_DAMAGE: u32 = 12723;
const SWEEPING_STRIKES_LOOP: &[u32] = &[12723, 26654];
const WHIRLWIND: u32 = 1680;
const BLADE_FLURRY: u32 = 13877;
const BLADE_FLURRY_DAMAGE: u32 = 22482;
const RETALIATION: u32 = 20230;
const RETALIATION_STRIKE: u32 = 22858;
const EYE_FOR_AN_EYE: &[u32] = &[9799, 25988];
const EYE_FOR_AN_EYE_DAMAGE: u32 = 25997;

/// Lord Kazzak's marks: blows on the reflected heal the one striking, and
/// the marked drained dry explodes.
const TWISTED_REFLECTION: u32 = 21063;
const TWISTED_REFLECTION_HEAL: u32 = 21064;
pub const MARK_OF_KAZZAK: u32 = 21056;
const MARK_OF_KAZZAK_EXPLOSION: u32 = 21058;

const SCHOOL_PHYSICAL: u32 = 1;
const SCHOOL_HOLY: u32 = 2;

/// A holder's dummy (or proc-trigger) effect's amount.
fn amount_of(h: &Holder, kinds: &[u32], default: i32) -> i32 {
    h.effects
        .iter()
        .find(|x| kinds.contains(&x.kind) && x.amount > 0)
        .map_or(default, |x| x.amount)
}

/// Power given, and logged.
pub fn energize(map: &mut Map, caster: Guid, target: Guid, spell: u32, power: u16, amount: u32) {
    if amount == 0 {
        return;
    }
    let Some(f) = map.unit_fields_mut(target) else {
        return;
    };
    let max = f.get_u32(UNIT_FIELD_MAXPOWER1 + power);
    let now = f.get_u32(UNIT_FIELD_POWER1 + power);
    f.set_u32(UNIT_FIELD_POWER1 + power, (now + amount).min(max));
    if let Some(frame) = crate::game::conn::encode(&SMSG_SPELLENERGIZELOG {
        victim: target,
        caster,
        spell,
        power: Power::try_from(power as u8).unwrap_or(Power::Mana),
        damage: amount,
    }) {
        map.send_to_observers(target, &frame, true);
    }
}

/// `spell`'s first effect as an aura on `target`, with `amount` for it.
fn aura_with(map: &mut Map, caster: Guid, target: Guid, spell: u32, amount: i32) {
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(spell) else {
        return;
    };
    let _ = auras::apply_spell(map, caster, target, s, &[(0, amount)], Default::default());
}

/// The triggering spell's base mana cost.
fn mana_cost(map: &Map, e: &Event) -> u32 {
    e.spell
        .and_then(|s| map.realm.data.spells.get(s))
        .map_or(0, |s| s.mana_cost)
}

/// Run a scripted proc of holder `h` on `owner`, against `other`.
pub fn fire(map: &mut Map, owner: Guid, other: Guid, h: &Holder, e: &Event) -> Outcome {
    let id = h.spell;
    let mut rng = rand::rng();
    if let Some((_, pct)) = IGNITE.iter().find(|(s, _)| *s == id) {
        if e.ex & procs::EX_CRITICAL_HIT != 0 {
            ignite(map, owner, other, e.damage as i32 * pct / 100);
        }
        return Outcome::Done;
    }
    if MASTER_OF_ELEMENTS.contains(&id) {
        let refund = mana_cost(map, e) as i32 * amount_of(h, &[auras::AURA_DUMMY], 0) / 100;
        energize(
            map,
            owner,
            owner,
            MASTER_OF_ELEMENTS_ENERGIZE,
            0,
            refund.max(0) as u32,
        );
        return Outcome::Done;
    }
    if ILLUMINATION.contains(&id) {
        energize(
            map,
            owner,
            owner,
            ILLUMINATION_ENERGIZE,
            0,
            mana_cost(map, e),
        );
        return Outcome::Done;
    }
    if MAGIC_ABSORPTION.contains(&id) {
        let max = map
            .unit_fields(owner)
            .map_or(0, |f| f.get_u32(UNIT_FIELD_MAXPOWER1));
        let restored = max as i64 * i64::from(amount_of(h, &[auras::AURA_DUMMY], 0)) / 100;
        energize(
            map,
            owner,
            owner,
            MAGIC_ABSORPTION_ENERGIZE,
            0,
            restored.max(0) as u32,
        );
        return Outcome::Done;
    }
    if id == MANA_DRAIN {
        crate::game::map::spells::cast_triggered(map, owner, MANA_DRAIN_ENERGIZE, Some(owner));
        crate::game::map::spells::cast_triggered(map, owner, MANA_DRAIN_LEECH, Some(other));
        return Outcome::Done;
    }
    if let Some((_, pct)) = PYROCLASM.iter().find(|(s, _)| *s == id) {
        let ticks = e
            .spell
            .and_then(|s| map.realm.data.spells.get(s))
            .and_then(|s| {
                if s.family_name == 5 && s.spell_icon == 184 && s.spell_visual == 2253 {
                    Some(1.0)
                } else if s.family_name == 5 && s.family_flags & 0x40 != 0 {
                    Some(15.0)
                } else if s.family_name == 5 && s.family_flags & 0x20 != 0 {
                    Some(4.0)
                } else {
                    None
                }
            });
        if let Some(ticks) = ticks
            && other != owner
            && map.unit_view(other).is_some_and(|u| u.alive)
            && rng.random_range(0.0f32..100.0) < pct / ticks
        {
            crate::game::map::spells::cast_triggered(map, owner, PYROCLASM_STUN, Some(other));
        }
        return Outcome::Done;
    }
    if let Some((_, heal)) = BLESSED_RECOVERY.iter().find(|(s, _)| *s == id) {
        let pct = amount_of(h, &[auras::AURA_PROC_TRIGGER_SPELL], 0);
        if e.damage > 0 && pct > 0 {
            let amount = e.damage as i32 * pct / 300 + i32::from(rng.random_bool(0.5));
            aura_with(map, owner, owner, *heal, amount);
        }
        return Outcome::Done;
    }
    if id == PERSISTENT_SHIELD {
        if e.damage > 0 && map.unit_view(e.target).is_some_and(|u| u.alive) {
            aura_with(
                map,
                owner,
                e.target,
                PERSISTENT_SHIELD_ABSORB,
                (e.damage * 15 / 100) as i32,
            );
        }
        return Outcome::Done;
    }
    if id == VAMPIRIC_EMBRACE {
        // The embraced take shadow damage from the priest: the priest and
        // their party heal a share.
        if other == h.caster && e.damage > 0 {
            let heal =
                (e.damage as i32 * amount_of(h, &[auras::AURA_DUMMY], 20) / 100).max(1) as u32;
            vampiric_embrace(map, h.caster, heal);
        }
        return Outcome::Done;
    }
    if let Some((_, gift)) = JUDGEMENTS.iter().find(|(s, _)| *s == id) {
        if guid::is_player(other) || map.creature(other).is_some() {
            crate::game::map::spells::cast_triggered(map, other, *gift, Some(other));
        }
        return Outcome::Done;
    }
    if id == COMBUSTION {
        combustion(map, owner, h, e);
        return Outcome::DoneOwnCharges;
    }
    if SWEEPING_STRIKES.contains(&id) {
        if e.damage > 1 && !e.spell.is_some_and(|s| SWEEPING_STRIKES_LOOP.contains(&s)) {
            let radius = if e.spell == Some(WHIRLWIND) { 8.0 } else { 5.0 };
            sweep(map, owner, other, e.damage, radius, SWEEPING_STRIKES_DAMAGE);
            return Outcome::Done;
        }
        return Outcome::DoneOwnCharges;
    }
    // Blade Flurry: the blow's damage on another enemy close by.
    if id == BLADE_FLURRY {
        if e.damage > 1 && e.spell != Some(BLADE_FLURRY_DAMAGE) {
            sweep(map, owner, other, e.damage, 5.0, BLADE_FLURRY_DAMAGE);
        }
        return Outcome::Done;
    }
    if id == RETALIATION {
        let in_front = match (map.unit_view(owner), map.unit_view(other)) {
            (Some(me), Some(them)) => combat::in_arc(&me, &them, std::f32::consts::PI),
            _ => false,
        };
        if in_front && !auras::auras(map, owner).is_some_and(|a| a.has(auras::AURA_MOD_STUN)) {
            crate::game::map::spells::cast_triggered(map, owner, RETALIATION_STRIKE, Some(other));
            return Outcome::Done;
        }
        return Outcome::DoneOwnCharges;
    }
    if id == TWISTED_REFLECTION {
        if other != owner {
            crate::game::map::spells::cast_triggered(
                map,
                owner,
                TWISTED_REFLECTION_HEAL,
                Some(other),
            );
        }
        return Outcome::Done;
    }
    if EYE_FOR_AN_EYE.contains(&id) {
        if e.ex & procs::EX_CRITICAL_HIT != 0 {
            let max = map
                .unit_fields(owner)
                .map_or(0, |f| f.get_u32(UNIT_FIELD_MAXHEALTH));
            let damage = (e.damage as i64 * i64::from(amount_of(h, &[auras::AURA_DUMMY], 0)) / 100)
                .min(i64::from(max / 2));
            procs::spell_damage(
                map,
                owner,
                other,
                EYE_FOR_AN_EYE_DAMAGE,
                damage.max(0) as u32,
                SCHOOL_HOLY,
            );
        }
        return Outcome::Done;
    }
    Outcome::Unhandled
}

/// A mana leech of holder `h` drained `target`'s `power`: Mark of Kazzak
/// explodes once there's none left, and is gone.
pub fn leeched(map: &mut Map, target: Guid, h: &Holder, power: u16) {
    if h.spell != MARK_OF_KAZZAK {
        return;
    }
    let dry = map
        .unit_fields(target)
        .is_some_and(|f| f.get_u32(UNIT_FIELD_POWER1 + power) == 0);
    if !dry {
        return;
    }
    crate::game::map::spells::cast_triggered(map, target, MARK_OF_KAZZAK_EXPLOSION, Some(target));
    auras::remove_spell(map, target, MARK_OF_KAZZAK, Some(h.caster));
}

/// Deep Wounds' trigger: the bleed, a share of the weapon's average blow
/// over four ticks. Whether `trigger` was one.
pub fn deep_wounds(map: &mut Map, owner: Guid, other: Guid, trigger: u32) -> bool {
    let Some((_, pct)) = DEEP_WOUNDS.iter().find(|(s, _)| *s == trigger) else {
        return false;
    };
    let average = map.unit_fields(owner).map_or(0.0, |f| {
        (f.get_f32(UNIT_FIELD_MINDAMAGE) + f.get_f32(UNIT_FIELD_MAXDAMAGE)) / 2.0
    });
    let tick = (average * *pct as f32 / 100.0 / DEEP_WOUND_TICKS as f32) as i32;
    if tick > 0 && map.unit_view(other).is_some_and(|u| u.alive) {
        aura_with(map, owner, other, DEEP_WOUND_BLEED, tick);
    }
    true
}

/// Ignite: a burn whose ticks grow with each fire crit, up to five, until
/// it runs out.
fn ignite(map: &mut Map, owner: Guid, target: Guid, tick: i32) {
    if tick <= 0 || !map.unit_view(target).is_some_and(|u| u.alive) {
        return;
    }
    let current = auras::auras(map, target).and_then(|a| {
        a.holders
            .iter()
            .find(|h| h.spell == IGNITE_BURN && h.caster == owner)
            .map(|h| (h.effects.first().map_or(0, |x| x.amount), h.combo))
    });
    let (amount, stacks) = match current {
        Some((amount, stacks)) if stacks < IGNITE_MAX_STACKS => (amount + tick, stacks + 1),
        Some((amount, stacks)) => (amount, stacks),
        None => (tick, 1),
    };
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(IGNITE_BURN) else {
        return;
    };
    let _ = auras::apply_spell(
        map,
        owner,
        target,
        s,
        &[(0, amount)],
        auras::Context {
            // The holder's combo count is Ignite's stack count.
            combo: stacks,
            ..Default::default()
        },
    );
}

/// Combustion: each fire spell adds to its crit; each fire crit spends a
/// charge, and the last takes it and its buff off.
fn combustion(map: &mut Map, owner: Guid, h: &Holder, e: &Event) {
    if e.ex & procs::EX_CRITICAL_HIT != 0 {
        if h.charges <= 1 {
            auras::remove_where(map, owner, |x| {
                x.spell == COMBUSTION || x.spell == COMBUSTION_BUFF
            });
        } else {
            auras::spend_charge(map, owner, COMBUSTION, h.caster);
        }
    } else {
        crate::game::map::spells::cast_triggered(map, owner, COMBUSTION_BUFF, Some(owner));
    }
}

/// Vampiric Embrace's healing for the priest and their party near them.
fn vampiric_embrace(map: &mut Map, priest: Guid, heal: u32) {
    let Some(at) = map.position_of(priest) else {
        return;
    };
    let members: Vec<Guid> = {
        let groups = map.realm.social.groups.lock();
        match groups.group_of(priest) {
            Some(g) => g.members.iter().map(|m| m.guid).collect(),
            None => vec![priest],
        }
    };
    for m in members {
        if m == priest || map.position_of(m).is_some_and(|p| dist(p, at) <= 30.0) {
            crate::game::map::spells::heal(map, priest, m, VAMPIRIC_EMBRACE_HEAL, heal, false);
        }
    }
}

/// Sweeping Strikes and Blade Flurry: the blow's damage, as `spell`, on
/// another enemy near the victim.
fn sweep(map: &mut Map, owner: Guid, victim: Guid, damage: u32, radius: f32, spell: u32) {
    let Some(at) = map.position_of(victim) else {
        return;
    };
    let next = map
        .grid
        .around(at.x, at.y, radius)
        .filter(|g| *g != victim && *g != owner && (guid::is_unit(*g) || guid::is_player(*g)))
        .filter(|g| map.position_of(*g).is_some_and(|p| dist(p, at) <= radius))
        .find(|g| combat::can_attack(map, owner, *g));
    if let Some(next) = next {
        procs::spell_damage(map, owner, next, spell, damage, SCHOOL_PHYSICAL);
    }
}
