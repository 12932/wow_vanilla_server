//! Procs: auras that go off when their unit hits or is hit. Melee swings,
//! ranged shots and spells report what happened through [`trigger`].
//!
//! A holder procs when its spell's `procFlags` (or `spell_proc_event`'s)
//! name what happened, the outcome and the triggering spell's school and
//! family pass `spell_proc_event`'s rules, its cooldown is over, and its
//! chance rolls: a flat chance, or procs a minute against the weapon's
//! speed. Proc-trigger auras then cast their spell, proc-damage auras deal
//! theirs, and a charge is spent. Weapons proc their chance-on-hit spells
//! and their enchantments' (Crusader, Fiery, poisons, Windfury), damage
//! shields hit back, and target-trigger auras (Frostbite, Improved Scorch)
//! go off on their spells. As in Thistle Tea's `Spell.Proc`.

use std::cell::Cell;

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{self as sd, ProcEvent, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, Holder};
use crate::game::map::combat::{self, Hand};

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
/// `spell_proc_event`'s: always, whatever the outcome.
pub const EX_TRIGGER_ALWAYS: u32 = 0x1_0000;
pub const EX_PERIODIC_POSITIVE: u32 = 0x4_0000;
/// The event is a cast ending, not a hit.
pub const EX_CAST_END: u32 = 0x8_0000;

/// Item spells' trigger: chance on hit.
/// Enchantment effect type: a spell cast in combat.
const EFFECT_ADD_EXTRA_ATTACKS: u32 = 19;

/// Lightning Shield's and Shadowguard's ranks, and the spell each strikes
/// back with.
const RETALIATION: &[(u32, u32)] = &[
    (324, 26364),
    (325, 26365),
    (905, 26366),
    (945, 26367),
    (8134, 26369),
    (10431, 26370),
    (10432, 26363),
    (18137, 28377),
    (19308, 28378),
    (19309, 28379),
    (19310, 28380),
    (19311, 28381),
    (19312, 28382),
];

/// Seal of Righteousness's ranks and the holy strike each deals.
const RIGHTEOUSNESS: &[(u32, u32)] = &[
    (20154, 25742),
    (21084, 25742),
    (20287, 25740),
    (20288, 25739),
    (20289, 25738),
    (20290, 25737),
    (20291, 25736),
    (20292, 25735),
    (20293, 25713),
];
/// Nightfall's ranks give Shadow Trance.
const NIGHTFALL: &[u32] = &[18094, 18095];
const SHADOW_TRANCE: u32 = 17941;
const SCHOOL_HOLY: u32 = 2;

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
    /// The spell that did it; `None` for a melee swing.
    pub spell: Option<u32>,
    pub damage: u32,
    /// The school, as a mask.
    pub school: u32,
    /// The off hand's swing.
    pub offhand: bool,
    /// A ranged attack.
    pub ranged: bool,
    /// An extra attack's: it can't give more.
    pub extra_attack: bool,
}

thread_local! {
    /// How deep procs are in procs: a proc's spell procs only what allows
    /// it, and not without end.
    static DEPTH: Cell<u8> = const { Cell::new(0) };
}

/// A level deeper in procs, while it lives: the depth before is put back
/// as it goes, a panic's unwinding too (the depth is the thread's, and a
/// worker's thread runs every map on it).
struct Deeper(u8);

impl Deeper {
    /// A level deeper, unless procs are already as deep as they go.
    fn enter() -> Option<Self> {
        let depth = DEPTH.get();
        if depth >= 3 {
            return None;
        }
        DEPTH.set(depth + 1);
        Some(Self(depth))
    }
}

impl Drop for Deeper {
    fn drop(&mut self) {
        DEPTH.set(self.0);
    }
}

/// Run the procs of both units for `e`.
pub fn trigger(map: &mut Map, e: Event) {
    let Some(deeper) = Deeper::enter() else {
        return;
    };
    let from_proc = deeper.0 > 0;
    if e.actor_flags != 0 {
        run(map, e.actor, e.target, e.actor_flags, &e, true, from_proc);
    }
    // A victim the blow killed procs nothing but its being killed (mangos's
    // `ProcDamageAndSpell`: `canProcVictim` wants it alive): no Blessed
    // Recovery from a killing crit.
    let target_flags = if map.unit_view(e.target).is_some_and(|u| u.alive) {
        e.target_flags
    } else {
        e.target_flags & PROC_KILLED
    };
    if target_flags != 0 && e.target != e.actor {
        run(map, e.target, e.actor, target_flags, &e, false, from_proc);
    }
    if landed(e.ex) && !from_proc {
        // Weapons' enchantments and chance-on-hit spells.
        crate::game::map::enchant::weapon_procs(map, &e);
        target_triggers(map, &e);
    }
    drop(deeper);
}

fn landed(ex: u32) -> bool {
    ex & (EX_NORMAL_HIT | EX_CRITICAL_HIT | EX_BLOCK) != 0 && ex & EX_CAST_END == 0
}

/// The hand an event was dealt with.
fn hand_of(e: &Event) -> Hand {
    if e.ranged {
        Hand::Ranged
    } else if e.offhand {
        Hand::Off
    } else {
        Hand::Main
    }
}

/// A weapon's speed in milliseconds, for procs a minute, before haste: a
/// druid's paws in cat form (a second) or bear form (two and a half),
/// else the hand's weapon (Thistle Tea's `AttackSpeed.base_ms`).
fn weapon_speed(map: &Map, g: Guid, hand: Hand) -> u32 {
    if hand != Hand::Ranged && map.players.contains_key(&g) {
        match auras::form(map, g) {
            auras::FORM_CAT => return 1000,
            auras::FORM_BEAR | auras::FORM_DIREBEAR => return 2500,
            _ => {}
        }
    }
    combat::hand_weapon(map, g, hand)
        .map(|w| w.delay)
        .filter(|d| *d > 0)
        .unwrap_or_else(|| {
            map.unit_fields(g)
                .map(|f| f.get_u32(UNIT_FIELD_BASEATTACKTIME))
                .unwrap_or(2000)
        })
}

/// A chance from procs a minute and a weapon's speed: past 100 when the
/// weapon is slow enough, so a chance modifier takes from all of it before
/// a roll counts 100 or more as sure (Thistle Tea's `Proc.chance`).
pub fn ppm_chance(ppm: f32, speed_ms: u32) -> f32 {
    ppm * speed_ms as f32 / 600.0
}

/// The `procFlags` a holder procs on. A seal of righteousness strikes on
/// abilities' blows too.
fn proc_flags(s: &SpellTemplate, pe: Option<&ProcEvent>) -> u32 {
    let flags = match pe.filter(|p| p.flags != 0) {
        Some(p) => p.flags,
        None => s.proc_flags,
    };
    if RIGHTEOUSNESS.iter().any(|(r, _)| *r == s.id) {
        flags | PROC_DONE_MELEE_SPELL
    } else {
        flags
    }
}

/// `spell_proc_event`'s outcome rule, or by default a hit or crit that
/// isn't a cast ending.
pub fn outcome_allowed(pe: Option<&ProcEvent>, ex: u32) -> bool {
    match pe.filter(|p| p.ex != 0) {
        Some(p) => {
            (p.ex & EX_CAST_END) == (ex & EX_CAST_END)
                && (p.ex & EX_TRIGGER_ALWAYS != 0 || p.ex & ex != 0)
        }
        None => ex & EX_CAST_END == 0 && ex & (EX_NORMAL_HIT | EX_CRITICAL_HIT) != 0,
    }
}

/// Whether the triggering spell passes `spell_proc_event`'s school and
/// family rules.
pub(crate) fn spell_allowed(
    pe: Option<&ProcEvent>,
    spell: Option<&SpellTemplate>,
    school: u32,
) -> bool {
    let Some(pe) = pe else {
        return true;
    };
    if pe.school_mask != 0 {
        let school = spell.map_or(if school == 0 { 1 } else { school }, |s| s.school_mask());
        if pe.school_mask & school == 0 {
            return false;
        }
    }
    if pe.family != 0 && spell.is_none_or(|s| s.family_name != pe.family) {
        return false;
    }
    if pe.family_mask.iter().any(|m| *m != 0) {
        let Some(s) = spell else {
            return false;
        };
        if !pe.family_mask.iter().any(|m| m & s.family_flags != 0) {
            return false;
        }
    }
    true
}

fn run(
    map: &mut Map,
    owner: Guid,
    other: Guid,
    flags: u32,
    e: &Event,
    outgoing: bool,
    from_proc: bool,
) {
    let data = map.realm.data.clone();
    let now = map.now;
    let Some(a) = auras::auras(map, owner) else {
        return;
    };
    let candidates: Vec<Holder> = a
        .holders
        .iter()
        .filter(|h| {
            data.spells
                .get(h.spell)
                .is_some_and(|s| proc_flags(s, data.spells.proc_events.get(&h.spell)) & flags != 0)
        })
        .cloned()
        .collect();
    if candidates.is_empty() {
        return;
    }
    let trigger_spell = e.spell.and_then(|id| data.spells.get(id));
    let mut rng = rand::rng();
    for h in candidates {
        let Some(s) = data.spells.get(h.spell) else {
            continue;
        };
        let pe = data.spells.proc_events.get(&h.spell);
        if e.spell == Some(h.spell) {
            continue;
        }
        if from_proc && !s.attr(3, sd::ATTR_EX3_CAN_PROC_FROM_PROCS) {
            continue;
        }
        if let Some(t) = trigger_spell {
            let suppress = if outgoing {
                sd::ATTR_EX3_SUPPRESS_CASTER_PROCS
            } else {
                sd::ATTR_EX3_SUPPRESS_TARGET_PROCS
            };
            if t.attr(3, suppress) {
                continue;
            }
        }
        if !spell_allowed(pe, trigger_spell, e.school) || !outcome_allowed(pe, e.ex) {
            continue;
        }
        // A mechanic immunity procs only on a spell of its mechanic (mangos's
        // `HandleMechanicImmuneResistanceAuraProc`): Fear Ward isn't spent
        // by a Frostbolt; what it keeps off spends it.
        let immunity = h
            .effects
            .iter()
            .find(|x| x.kind == auras::AURA_MECHANIC_IMMUNITY);
        if let Some(x) = immunity
            && !trigger_spell.is_some_and(|t| {
                t.mechanic == x.misc as u32
                    || t.effects
                        .iter()
                        .any(|f| f.effect != 0 && f.mechanic == x.misc as u32)
            })
        {
            continue;
        }
        let hand = hand_of(e);
        if outgoing
            && h.item_class == 2
            && map.players.contains_key(&owner)
            && !crate::game::map::derive::fits(&h, combat::hand_weapon(map, owner, hand).as_ref())
        {
            continue;
        }
        if h.proc_ready_at.is_some_and(|t| t > now) {
            continue;
        }
        // An extra attack's swing can't give more.
        if e.extra_attack && grants_extra_attacks(&data, &h) {
            continue;
        }
        let chance = chance(map, owner, s, hand, outgoing);
        if chance <= 0.0 || (chance < 100.0 && rng.random_range(0.0f32..100.0) >= chance) {
            continue;
        }
        // The cooldown starts before the proc goes off, so what it casts
        // can't set it off again, and is taken back if nothing came of it.
        if let Some(p) = pe.filter(|p| p.cooldown > 0) {
            let ready = now + std::time::Duration::from_millis(u64::from(p.cooldown));
            set_ready(map, owner, &h, Some(ready));
        }
        if crate::game::map::trinkets::proc(map, owner, &h, e) {
            continue;
        }
        let script = crate::game::map::proc_scripts::fire(map, owner, other, &h, e);
        let went_off = script != crate::game::map::proc_scripts::Outcome::Unhandled || {
            let fired = fire(map, owner, other, &h);
            let answered = crate::game::map::spells::class_scripts::proc(
                map, owner, other, &h, e.spell, outgoing,
            );
            fired || answered || (h.charges > 0 && !answers_alone(&h))
        };
        // A proc that came to nothing (a class script's dead victim, an
        // armor with no answer to the school) spends neither its cooldown
        // nor a charge, as mangos's `SPELL_AURA_PROC_FAILED` and Thistle
        // Tea's reactions have it.
        if !went_off {
            set_ready(map, owner, &h, h.proc_ready_at);
            continue;
        }
        if h.charges > 0 && script != crate::game::map::proc_scripts::Outcome::DoneOwnCharges {
            auras::spend_charge(map, owner, h.spell, h.caster);
        }
    }
}

/// The chance, in percent, that `owner`'s aura of `s` procs on what it
/// does (`outgoing`) with `hand` or has done to it: an outgoing rate a
/// minute by the hand's speed, else `spell_proc_event`'s chance or the
/// spell's own; then the bearer's modifiers to it, flat then percent,
/// uncapped (Thistle Tea's `ProcChance`).
pub fn chance(map: &Map, owner: Guid, s: &SpellTemplate, hand: Hand, outgoing: bool) -> f32 {
    let pe = map.realm.data.spells.proc_events.get(&s.id);
    let chance = match pe {
        Some(p) if p.ppm > 0.0 && outgoing => ppm_chance(p.ppm, weapon_speed(map, owner, hand)),
        Some(p) if p.chance > 0.0 => p.chance,
        _ => s.proc_chance as f32,
    };
    crate::game::map::spells::mods::for_spell(map, owner, s)
        .apply(crate::game::map::spells::mods::OP_CHANCE_OF_SUCCESS, chance)
}

/// When `h`'s proc may go off again.
fn set_ready(map: &mut Map, owner: Guid, h: &Holder, at: Option<std::time::Instant>) {
    if let Some(a) = auras::auras_mut(map, owner)
        && let Some(x) = a
            .holders
            .iter_mut()
            .find(|x| x.spell == h.spell && x.caster == h.caster && x.item == h.item)
    {
        x.proc_ready_at = at;
    }
}

/// Whether a holder's charges go only when it answers: a class script's,
/// a healing power's or a reactive armor's, or a spell modifier's (spent
/// by the casts it changes), as Thistle Tea's
/// `spends_charge_without_trigger?` leaves them out. Any other holder with
/// charges spends one whenever it procs (Fear Ward's by the fear it keeps
/// off).
fn answers_alone(h: &Holder) -> bool {
    crate::game::map::spells::class_scripts::answers(h.spell)
        || h.effects.iter().any(|x| {
            matches!(
                x.kind,
                auras::AURA_OVERRIDE_CLASS_SCRIPTS
                    | auras::AURA_ADD_FLAT_MODIFIER
                    | auras::AURA_ADD_PCT_MODIFIER
            )
        })
}

fn grants_extra_attacks(data: &crate::game::data::GameData, h: &Holder) -> bool {
    h.effects.iter().any(|x| {
        x.trigger != 0
            && data
                .spells
                .get(x.trigger)
                .is_some_and(|t| t.has_effect(EFFECT_ADD_EXTRA_ATTACKS))
    })
}

/// A holder went off: its trigger spells cast, its damage dealt, and the
/// class scripts the data can't say (Thistle Tea's `Core.Class`). Whether
/// anything came of it.
fn fire(map: &mut Map, owner: Guid, other: Guid, h: &Holder) -> bool {
    let mut fired = false;
    for x in &h.effects {
        match x.kind {
            auras::AURA_DUMMY => {
                if let Some((_, strike)) = RIGHTEOUSNESS.iter().find(|(s, _)| *s == h.spell)
                    && x.effect == 0
                {
                    fired = true;
                    let speed = (weapon_speed(map, owner, Hand::Main) as f32 / 1000.0).max(1.5);
                    let amount = (x.amount * h.stacks.max(1) as i32) as f32;
                    let damage = amount / 87.0
                        + (amount / 25.0 - amount / 87.0) * ((speed.min(4.0) - 1.5) / 2.5);
                    spell_damage(
                        map,
                        owner,
                        other,
                        *strike,
                        damage.max(0.0) as u32,
                        SCHOOL_HOLY,
                    );
                }
            }
            // Its victim must be alive, as a class script's (mangos's
            // `HandleOverrideClassScriptAuraProc`): a killing tick gives
            // no trance.
            auras::AURA_OVERRIDE_CLASS_SCRIPTS
                if NIGHTFALL.contains(&h.spell)
                    && map.unit_view(other).is_some_and(|u| u.alive) =>
            {
                fired = true;
                crate::game::map::spells::cast_triggered(map, owner, SHADOW_TRANCE, Some(owner));
            }
            auras::AURA_PROC_TRIGGER_SPELL => {
                let trigger = RETALIATION
                    .iter()
                    .find(|(s, _)| *s == h.spell)
                    .map_or(x.trigger, |(_, t)| *t);
                let trigger = crate::game::map::spells::class_scripts::proc_spell(h.spell, trigger);
                if trigger != 0 {
                    fired = true;
                    if !crate::game::map::proc_scripts::deep_wounds(map, owner, other, trigger) {
                        crate::game::map::spells::cast_triggered(map, owner, trigger, Some(other));
                    }
                }
            }
            auras::AURA_PROC_TRIGGER_DAMAGE => {
                fired = true;
                let amount = (x.amount * h.stacks.max(1) as i32).max(0);
                proc_damage(map, owner, other, h, x.effect as usize, amount);
            }
            _ => {}
        }
    }
    fired
}

/// A proc aura's damage, as Thistle Tea's `ProcDamage` makes it: a school
/// damage spell of its own effect's points with its spell power share,
/// never a crit or reflected. Its hit is rolled only for a harmful magic
/// aura's.
fn proc_damage(map: &mut Map, owner: Guid, other: Guid, h: &Holder, index: usize, amount: i32) {
    let data = map.realm.data.clone();
    let Some(t) = data.spells.get(h.spell).filter(|_| index < 3) else {
        spell_damage(map, owner, other, h.spell, amount.max(0) as u32, h.school);
        return;
    };
    let mut s = t.clone();
    let mut e = s.effects[index];
    e.effect = crate::game::map::spells::EFFECT_SCHOOL_DAMAGE;
    e.aura = 0;
    e.trigger_spell = 0;
    e.target_a = 6;
    e.target_b = 0;
    s.effects = [Default::default(); 3];
    s.effects[0] = e;
    s.attributes[0] &= !sd::ATTR_PASSIVE;
    s.attributes[1] |= sd::ATTR_EX_NO_REFLECTION;
    s.attributes[2] |= sd::ATTR_EX2_CANT_CRIT;
    // A helpful aura's damage (Holy Shield's, Blessing of Sanctuary's), or
    // one not of magic (a shield spike's), can't miss, though immunities
    // still keep it off: Thistle Tea's `ProcDamage.hit?`, and mangos's
    // `SpellHitResult` with the aura's own spell.
    if t.dmg_class != 1 || auras::positive(t) {
        s.attributes[3] |= sd::ATTR_EX3_ALWAYS_HIT;
    }
    s.speed = 0.0;
    crate::game::map::spells::scripts::cast_template_with_points(
        map,
        owner,
        s,
        other,
        [Some(amount.max(1)), None, None],
    );
}

/// Spell damage a proc deals: logged, absorbed, dealt.
pub fn spell_damage(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    spell: u32,
    damage: u32,
    school: u32,
) {
    if !map.unit_view(target).is_some_and(|u| u.alive) {
        return;
    }
    let (left, absorbed) = auras::absorb_and_share(map, caster, target, damage, school);
    if let Some(frame) = crate::game::conn::encode(&SMSG_SPELLNONMELEEDAMAGELOG {
        target,
        attacker: caster,
        spell,
        damage: left,
        school: SpellSchool::try_from(school.trailing_zeros() as u8).unwrap_or(SpellSchool::Normal),
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
    if left > 0 {
        combat::deal_damage_by(map, caster, target, left, school, Some(spell));
    }
}

/// Target-trigger auras (Frostbite, Improved Scorch, Healing Way): their
/// owner's spells of the family and mask they name may cast their spell on
/// the target. The chance is the aura's, and its effect's points a combo
/// point for each a finisher spends (Relentless Strikes' 20%), as Thistle
/// Tea's `TargetTrigger` rolls it.
fn target_triggers(map: &mut Map, e: &Event) {
    let data = map.realm.data.clone();
    let Some(spell) = e.spell.and_then(|id| data.spells.get(id)) else {
        return;
    };
    let combo = map
        .caster(e.actor)
        .filter(|c| spell.finisher() && c.combo_target == e.target)
        .map_or(0, |c| c.combo_points);
    let Some(a) = auras::auras(map, e.actor) else {
        return;
    };
    let triggers: Vec<(u32, f32)> = a
        .holders
        .iter()
        .filter(|h| {
            data.spells
                .get(h.spell)
                .is_some_and(|s| s.family_name == spell.family_name)
        })
        .flat_map(|h| {
            let per_combo = |index: u8| {
                data.spells.get(h.spell).map_or(0.0, |s| {
                    s.effects[usize::from(index).min(2)].points_per_combo_point
                })
            };
            h.effects
                .iter()
                .filter(|x| {
                    x.kind == auras::AURA_ADD_TARGET_TRIGGER
                        && x.trigger != 0
                        && x.mask & spell.family_flags != 0
                })
                .map(|x| {
                    let chance = x.amount + (per_combo(x.effect) * f32::from(combo)) as i32;
                    (x.trigger, chance as f32)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let mut rng = rand::rng();
    for (trigger, chance) in triggers {
        if chance >= 100.0 || rng.random_range(0.0f32..100.0) < chance {
            crate::game::map::spells::cast_triggered(map, e.actor, trigger, Some(e.target));
        }
    }
}

/// Damage shields (Thorns, Retribution Aura) on `victim` strike `attacker`
/// back for a melee blow that landed.
pub fn damage_shields(map: &mut Map, attacker: Guid, victim: Guid) {
    if attacker == victim || !map.unit_view(attacker).is_some_and(|u| u.alive) {
        return;
    }
    let shields: Vec<(u32, u32)> = auras::auras(map, victim)
        .map(|a| {
            a.effects()
                .filter(|(_, x)| x.kind == auras::AURA_DAMAGE_SHIELD)
                .map(|(h, x)| ((x.amount * h.stacks.max(1) as i32).max(0) as u32, h.school))
                .collect()
        })
        .unwrap_or_default();
    for (damage, school) in shields {
        if damage == 0 {
            continue;
        }
        if let Some(frame) = crate::game::conn::encode(&SMSG_SPELLDAMAGESHIELD {
            victim: attacker,
            caster: victim,
            damage,
            school: SpellSchool::try_from(school.trailing_zeros() as u8)
                .unwrap_or(SpellSchool::Normal),
        }) {
            map.send_to_observers(victim, &frame, true);
            if let Some(p) = map.players.get(&attacker)
                && !p.known.contains(&victim)
            {
                p.outbound.send_frame(frame);
            }
        }
        combat::deal_damage_school(map, victim, attacker, damage, school);
        if !map.unit_view(attacker).is_some_and(|u| u.alive) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_in_a_proc_leaves_the_depth_as_it_was() {
        let deeper = || {
            let _outer = Deeper::enter().expect("the first level");
            let _inner = Deeper::enter().expect("the second");
            panic!("a proc's spell panics");
        };
        for _ in 0..3 {
            assert!(std::panic::catch_unwind(deeper).is_err());
            assert_eq!(DEPTH.get(), 0, "the depth is put back");
        }
        // And procs still go three deep.
        let levels: Vec<_> = (0..4).map_while(|_| Deeper::enter()).collect();
        assert_eq!(levels.len(), 3);
        // Out again innermost first, as calls return.
        levels.into_iter().rev().for_each(drop);
        assert_eq!(DEPTH.get(), 0);
    }

    #[test]
    fn procs_a_minute_scale_with_weapon_speed() {
        // Seal of Command's 7 a minute on a 3.6-second weapon: 42%.
        assert!((ppm_chance(7.0, 3600) - 42.0).abs() < 0.01);
        assert_eq!(ppm_chance(60.0, 4000), 400.0);
    }

    #[test]
    fn the_outcome_rules_follow_spell_proc_event() {
        // By default a hit or a crit.
        assert!(outcome_allowed(None, EX_NORMAL_HIT));
        assert!(!outcome_allowed(None, EX_DODGE));
        // Flurry's talent: crits only.
        let crit = ProcEvent {
            ex: EX_CRITICAL_HIT,
            ..Default::default()
        };
        assert!(outcome_allowed(Some(&crit), EX_CRITICAL_HIT));
        assert!(!outcome_allowed(Some(&crit), EX_NORMAL_HIT));
        // Flurry's buff: whatever happens.
        let always = ProcEvent {
            ex: EX_TRIGGER_ALWAYS,
            ..Default::default()
        };
        assert!(outcome_allowed(Some(&always), EX_MISS));
        // Redoubt: a block.
        let block = ProcEvent {
            ex: EX_BLOCK,
            ..Default::default()
        };
        assert!(outcome_allowed(Some(&block), EX_BLOCK));
        assert!(!outcome_allowed(Some(&block), EX_NORMAL_HIT));
    }
}
