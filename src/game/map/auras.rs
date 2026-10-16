//! Auras: a spell's lasting effects on a unit. Each spell's auras share one
//! of the unit's 48 visible slots (32 helpful, 16 harmful). Periodic ones
//! tick; stat and speed ones change the unit while they last; roots, stuns
//! and silences hold it.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::map::Map;

pub const AURA_PERIODIC_DAMAGE: u32 = 3;
pub const AURA_DUMMY: u32 = 4;
pub const AURA_MOD_CONFUSE: u32 = 5;
pub const AURA_MOD_FEAR: u32 = 7;
pub const AURA_PERIODIC_HEAL: u32 = 8;
pub const AURA_MOD_STUN: u32 = 12;
pub const AURA_DAMAGE_SHIELD: u32 = 15;
pub const AURA_OBS_MOD_HEALTH: u32 = 20;
pub const AURA_OBS_MOD_MANA: u32 = 21;
pub const AURA_MOD_RESISTANCE: u32 = 22;
pub const AURA_PERIODIC_TRIGGER_SPELL: u32 = 23;
pub const AURA_PERIODIC_ENERGIZE: u32 = 24;
pub const AURA_MOD_PACIFY: u32 = 25;
pub const AURA_MOD_ROOT: u32 = 26;
pub const AURA_MOD_SILENCE: u32 = 27;
pub const AURA_MOD_STAT: u32 = 29;
pub const AURA_MOD_INCREASE_SPEED: u32 = 31;
pub const AURA_MOD_INCREASE_MOUNTED_SPEED: u32 = 32;
pub const AURA_MOD_DECREASE_SPEED: u32 = 33;
pub const AURA_MOD_INCREASE_HEALTH: u32 = 34;
pub const AURA_MOD_SHAPESHIFT: u32 = 36;
pub const AURA_PROC_TRIGGER_SPELL: u32 = 42;
pub const AURA_TRACK_CREATURES: u32 = 44;
pub const AURA_TRACK_RESOURCES: u32 = 45;
pub const AURA_PERIODIC_LEECH: u32 = 53;
pub const AURA_SCHOOL_ABSORB: u32 = 69;
pub const AURA_MOUNTED: u32 = 78;
pub const AURA_MOD_REGEN: u32 = 84;
pub const AURA_MOD_POWER_REGEN: u32 = 85;
pub const AURA_MOD_ATTACK_POWER: u32 = 99;
pub const AURA_MOD_SPEED_ALWAYS: u32 = 129;

/// Every aura type the server knows: those with an effect here, and those
/// it keeps on the unit (shown to the client) with none.
pub fn supported(aura: u32) -> bool {
    // Kept and shown, with no effect of their own yet: skill and spell
    // modifiers, immunities, detection, threat, procs the client shows,
    // and the like.
    const KEPT: &[u32] = &[
        0, 1, 2, 6, 9, 10, 11, 13, 14, 16, 17, 18, 19, 28, 30, 35, 37, 38, 39, 40, 41, 43, 47, 49,
        50, 51, 52, 54, 55, 56, 57, 58, 59, 60, 61, 65, 66, 67, 68, 71, 72, 73, 74, 75, 77, 79, 80,
        81, 82, 83, 86, 87, 88, 89, 90, 91, 92, 93, 94, 97, 98, 100, 101, 102, 103, 104, 105, 106,
        107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124,
        125, 126, 127, 128, 130, 131, 132, 133, 134, 135, 136, 137, 138, 139, 140, 141, 142, 143,
        144, 145, 146, 147, 148, 149, 150, 151, 152, 153, 154, 155, 156, 157, 158, 159, 160, 161,
        162, 163, 165, 166, 167, 168, 169,
    ];
    matches!(
        aura,
        AURA_PERIODIC_DAMAGE
            | AURA_DUMMY
            | AURA_MOD_CONFUSE
            | AURA_MOD_FEAR
            | AURA_PERIODIC_HEAL
            | AURA_MOD_STUN
            | AURA_DAMAGE_SHIELD
            | AURA_OBS_MOD_HEALTH
            | AURA_OBS_MOD_MANA
            | AURA_MOD_RESISTANCE
            | AURA_PERIODIC_TRIGGER_SPELL
            | AURA_PERIODIC_ENERGIZE
            | AURA_MOD_PACIFY
            | AURA_MOD_ROOT
            | AURA_MOD_SILENCE
            | AURA_MOD_STAT
            | AURA_MOD_INCREASE_SPEED
            | AURA_MOD_INCREASE_MOUNTED_SPEED
            | AURA_MOD_DECREASE_SPEED
            | AURA_MOD_INCREASE_HEALTH
            | AURA_MOD_SHAPESHIFT
            | AURA_PROC_TRIGGER_SPELL
            | AURA_TRACK_CREATURES
            | AURA_TRACK_RESOURCES
            | AURA_PERIODIC_LEECH
            | AURA_SCHOOL_ABSORB
            | AURA_MOUNTED
            | AURA_MOD_REGEN
            | AURA_MOD_POWER_REGEN
            | AURA_MOD_ATTACK_POWER
            | AURA_MOD_SPEED_ALWAYS
    ) || KEPT.contains(&aura)
}

/// One effect of a spell lasting on a unit.
#[derive(Debug, Clone)]
pub struct Aura {
    pub spell: u32,
    pub caster: Guid,
    pub effect: u8,
    pub kind: u32,
    pub amount: i32,
    pub misc: i32,
    pub trigger: u32,
    pub positive: bool,
    pub expires: Option<Instant>,
    pub period: Option<Duration>,
    pub next_tick: Option<Instant>,
    /// What an absorb has left.
    pub left: i32,
}

/// A unit's auras and the visible slots they take.
#[derive(Debug, Clone, Default)]
pub struct Auras {
    pub list: Vec<Aura>,
    /// The spell (and caster) in each of the 48 slots.
    pub slots: Vec<Option<(u32, Guid)>>,
}

impl Auras {
    pub fn has(&self, kind: u32) -> bool {
        self.list.iter().any(|a| a.kind == kind)
    }

    pub fn sum(&self, kind: u32) -> i32 {
        self.list
            .iter()
            .filter(|a| a.kind == kind)
            .map(|a| a.amount)
            .sum()
    }

    pub fn sum_misc(&self, kind: u32, misc: i32) -> i32 {
        self.list
            .iter()
            .filter(|a| a.kind == kind && (a.misc == misc || a.misc == -1))
            .map(|a| a.amount)
            .sum()
    }

    /// The speed multiplier the auras give, as a percentage change.
    pub fn speed_percent(&self) -> i32 {
        let up = self
            .list
            .iter()
            .filter(|a| {
                matches!(
                    a.kind,
                    AURA_MOD_INCREASE_SPEED
                        | AURA_MOD_SPEED_ALWAYS
                        | AURA_MOD_INCREASE_MOUNTED_SPEED
                )
            })
            .map(|a| a.amount)
            .max()
            .unwrap_or(0);
        let down = self
            .list
            .iter()
            .filter(|a| a.kind == AURA_MOD_DECREASE_SPEED)
            .map(|a| a.amount)
            .min()
            .unwrap_or(0);
        up + down
    }

    fn slot_of(&self, spell: u32, caster: Guid) -> Option<usize> {
        self.slots.iter().position(|s| *s == Some((spell, caster)))
    }
}

/// Whether a spell is helpful: no harmful effects, aimed at a friend.
pub fn positive(s: &SpellTemplate) -> bool {
    !s.effects.iter().any(|e| {
        matches!(e.effect, 2 | 9 | 17 | 31 | 58 | 121)
            || matches!(
                e.aura,
                AURA_PERIODIC_DAMAGE
                    | AURA_MOD_STUN
                    | AURA_MOD_ROOT
                    | AURA_MOD_SILENCE
                    | AURA_MOD_DECREASE_SPEED
                    | AURA_MOD_FEAR
                    | AURA_MOD_CONFUSE
                    | AURA_MOD_PACIFY
                    | AURA_PERIODIC_LEECH
            )
            || (e.aura == AURA_MOD_STAT || e.aura == AURA_MOD_RESISTANCE) && e.base_points < 0
            || matches!(e.target_a, 6 | 15 | 16 | 22 | 24 | 28)
    })
}

/// The auras of `g`, mutably.
pub fn auras_mut(map: &mut Map, g: Guid) -> Option<&mut Auras> {
    if map.players.contains_key(&g) {
        return map.players.get_mut(&g).map(|p| &mut p.auras);
    }
    map.creature_mut(g).map(|c| &mut c.auras)
}

pub fn auras(map: &Map, g: Guid) -> Option<&Auras> {
    if let Some(p) = map.players.get(&g) {
        return Some(&p.auras);
    }
    map.creature(g).map(|c| &c.auras)
}

/// Put `spell`'s effect `index` on `target`.
pub fn apply(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    spell: &SpellTemplate,
    index: usize,
    amount: i32,
) {
    let now = map.now;
    let data = map.realm.data.clone();
    let level = map.unit_view(caster).map(|u| u.level).unwrap_or(1);
    let duration = data.spells.duration(spell, level);
    let e = spell.effects[index];
    let pos = positive(spell);
    let period = (e.amplitude > 0).then(|| Duration::from_millis(u64::from(e.amplitude)));
    let aura = Aura {
        spell: spell.id,
        caster,
        effect: index as u8,
        kind: e.aura,
        amount,
        misc: e.misc_value,
        trigger: e.trigger_spell,
        positive: pos,
        expires: duration.map(|ms| now + Duration::from_millis(u64::from(ms))),
        period,
        next_tick: period.map(|p| now + p),
        left: amount,
    };
    // Slots something else holds (a ghost's aura) stay as they are.
    let taken: Vec<bool> = map
        .unit_fields(target)
        .map(|f| {
            (0..48)
                .map(|i| f.get_u32(UNIT_FIELD_AURA + i) != 0)
                .collect()
        })
        .unwrap_or_else(|| vec![false; 48]);
    let Some(auras) = auras_mut(map, target) else {
        return;
    };
    if auras.slots.is_empty() {
        auras.slots = vec![None; 48];
    }
    // A spell's aura from the same caster refreshes rather than stacks.
    auras
        .list
        .retain(|a| !(a.spell == spell.id && a.caster == caster && a.effect == index as u8));
    auras.list.push(aura);
    let slot = match auras.slot_of(spell.id, caster) {
        Some(s) => Some(s),
        None => {
            let range = if pos { 0..32 } else { 32..48 };
            let free = range
                .clone()
                .find(|i| auras.slots[*i].is_none() && !taken[*i]);
            if let Some(i) = free {
                auras.slots[i] = Some((spell.id, caster));
            }
            free
        }
    };
    if let Some(slot) = slot {
        set_slot_fields(map, target, slot, spell.id, level);
        if let (Some(p), Some(ms)) = (map.players.get(&target), duration) {
            p.outbound.send(&SMSG_UPDATE_AURA_DURATION {
                aura_slot: slot as u8,
                aura_duration: ms,
            });
        }
    }
    started(map, target, e.aura, amount, e.misc_value);
}

fn set_slot_fields(map: &mut Map, target: Guid, slot: usize, spell: u32, level: u32) {
    let Some(f) = map.unit_fields_mut(target) else {
        return;
    };
    let slot = slot as u16;
    f.set_u32(UNIT_FIELD_AURA + slot, spell);
    let flags_word = UNIT_FIELD_AURAFLAGS + slot / 8;
    let shift = (slot % 8) * 4;
    let old = f.get_u32(flags_word);
    let nibble = if spell == 0 { 0 } else { 0x9 };
    f.set_u32(flags_word, (old & !(0xF << shift)) | (nibble << shift));
    let level_byte = (slot % 4) as u8;
    f.set_byte(
        UNIT_FIELD_AURALEVELS + slot / 4,
        level_byte,
        if spell == 0 { 0 } else { level.min(255) as u8 },
    );
}

/// Take every aura of `spell` from `caster` off `target`.
pub fn remove_spell(map: &mut Map, target: Guid, spell: u32, caster: Option<Guid>) {
    let Some(auras) = auras_mut(map, target) else {
        return;
    };
    let gone: Vec<Aura> = auras
        .list
        .iter()
        .filter(|a| a.spell == spell && caster.is_none_or(|c| a.caster == c))
        .cloned()
        .collect();
    if gone.is_empty() {
        return;
    }
    auras
        .list
        .retain(|a| !(a.spell == spell && caster.is_none_or(|c| a.caster == c)));
    let slots: Vec<usize> = auras
        .slots
        .iter()
        .enumerate()
        .filter(|(_, s)| s.is_some_and(|(sp, c)| sp == spell && caster.is_none_or(|cc| cc == c)))
        .map(|(i, _)| i)
        .collect();
    for i in &slots {
        auras.slots[*i] = None;
    }
    for i in slots {
        set_slot_fields(map, target, i, 0, 0);
    }
    for a in gone {
        ended(map, target, a.kind, a.amount, a.misc);
    }
}

/// An aura began: what it does at once.
fn started(map: &mut Map, target: Guid, kind: u32, amount: i32, misc: i32) {
    match kind {
        AURA_MOD_STUN | AURA_MOD_ROOT => {
            crate::game::map::combat::stop_attack(map, target);
            if kind == AURA_MOD_STUN
                && let Some(f) = map.unit_fields_mut(target)
            {
                f.set_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_STUNNED,
                    true,
                );
            }
            if let Some(p) = map.players.get(&target) {
                p.outbound.send(&SMSG_FORCE_MOVE_ROOT {
                    guid: target,
                    counter: 0,
                });
            }
            crate::game::map::spells::interrupt(map, target);
        }
        AURA_MOD_SILENCE => crate::game::map::spells::interrupt(map, target),
        AURA_MOD_SHAPESHIFT => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_byte(UNIT_FIELD_BYTES_1, 2, misc as u8);
            }
        }
        AURA_MOUNTED => {
            let display = map
                .realm
                .data
                .creature_template(misc as u32)
                .and_then(|t| t.display_ids.iter().copied().find(|d| *d != 0))
                .unwrap_or(0);
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_u32(UNIT_FIELD_MOUNTDISPLAYID, display);
            }
        }
        AURA_TRACK_CREATURES => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_u32(PLAYER_TRACK_CREATURES, 1 << (misc - 1).clamp(0, 31));
            }
        }
        AURA_TRACK_RESOURCES => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_u32(PLAYER_TRACK_RESOURCES, 1 << (misc - 1).clamp(0, 31));
            }
        }
        _ => {}
    }
    let _ = amount;
    if matches!(
        kind,
        AURA_MOD_STAT
            | AURA_MOD_RESISTANCE
            | AURA_MOD_INCREASE_HEALTH
            | AURA_MOD_ATTACK_POWER
            | AURA_MOD_INCREASE_SPEED
            | AURA_MOD_DECREASE_SPEED
            | AURA_MOD_SPEED_ALWAYS
            | AURA_MOD_INCREASE_MOUNTED_SPEED
    ) {
        recalc(map, target);
    }
}

/// An aura ended: undo what it did.
fn ended(map: &mut Map, target: Guid, kind: u32, _amount: i32, _misc: i32) {
    let still = |map: &Map, k: u32| auras(map, target).is_some_and(|a| a.has(k));
    match kind {
        AURA_MOD_STUN | AURA_MOD_ROOT => {
            let stunned = still(map, AURA_MOD_STUN);
            let rooted = stunned || still(map, AURA_MOD_ROOT);
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_STUNNED,
                    stunned,
                );
            }
            if !rooted && let Some(p) = map.players.get(&target) {
                p.outbound.send(&SMSG_FORCE_MOVE_UNROOT {
                    guid: target,
                    counter: 0,
                });
            }
        }
        AURA_MOD_SHAPESHIFT if !still(map, AURA_MOD_SHAPESHIFT) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_byte(UNIT_FIELD_BYTES_1, 2, 0);
            }
        }
        AURA_MOUNTED if !still(map, AURA_MOUNTED) => {
            if let Some(f) = map.unit_fields_mut(target) {
                f.set_u32(UNIT_FIELD_MOUNTDISPLAYID, 0);
            }
        }
        _ => {}
    }
    if matches!(
        kind,
        AURA_MOD_STAT
            | AURA_MOD_RESISTANCE
            | AURA_MOD_INCREASE_HEALTH
            | AURA_MOD_ATTACK_POWER
            | AURA_MOD_INCREASE_SPEED
            | AURA_MOD_DECREASE_SPEED
            | AURA_MOD_SPEED_ALWAYS
            | AURA_MOD_INCREASE_MOUNTED_SPEED
    ) {
        recalc(map, target);
    }
}

/// Recompute a unit's stats and speed with its auras.
pub fn recalc(map: &mut Map, target: Guid) {
    let data = map.realm.data.clone();
    if let Some(p) = map.players.get_mut(&target) {
        p.apply_stats(&data);
        crate::game::map::combat::player_damage(p, &data);
        apply_stat_auras(&mut p.fields, &p.auras.clone());
        let speed = (7.0 * (1.0 + p.auras.speed_percent() as f32 / 100.0)).max(0.5);
        let ghost = p.death != crate::game::map::death::Death::Alive;
        if !ghost && (speed - p.speeds.run).abs() > 0.01 {
            p.speeds.run = speed;
            p.outbound.send(&SMSG_FORCE_RUN_SPEED_CHANGE {
                guid: target,
                move_event: 0,
                speed,
            });
        }
        return;
    }
    if let Some(c) = map.creature_mut(target) {
        let list = c.auras.clone();
        apply_stat_auras(&mut c.fields, &list);
    }
}

fn apply_stat_auras(f: &mut Fields, auras: &Auras) {
    for a in auras.list.iter() {
        match a.kind {
            AURA_MOD_STAT => {
                let stats: Vec<u16> = if a.misc < 0 {
                    (0..5).collect()
                } else {
                    vec![a.misc.clamp(0, 4) as u16]
                };
                for s in stats {
                    let v = f.get_u32(UNIT_FIELD_STAT0 + s) as i32 + a.amount;
                    f.set_u32(UNIT_FIELD_STAT0 + s, v.max(0) as u32);
                    let field = if a.amount >= 0 {
                        PLAYER_FIELD_POSSTAT0
                    } else {
                        PLAYER_FIELD_NEGSTAT0
                    };
                    if f.type_id() == TypeId::Player {
                        let old = f.get_u32(field + s);
                        f.set_u32(field + s, old + a.amount.unsigned_abs());
                    }
                }
            }
            AURA_MOD_RESISTANCE if a.misc & 1 != 0 => {
                let v = f.get_u32(UNIT_FIELD_RESISTANCES) as i32 + a.amount;
                f.set_u32(UNIT_FIELD_RESISTANCES, v.max(0) as u32);
            }
            AURA_MOD_INCREASE_HEALTH => {
                let v = f.get_u32(UNIT_FIELD_MAXHEALTH) as i32 + a.amount;
                f.set_u32(UNIT_FIELD_MAXHEALTH, v.max(1) as u32);
            }
            AURA_MOD_ATTACK_POWER => {
                f.set_u32(UNIT_FIELD_ATTACK_POWER_MODS, a.amount.max(0) as u32);
            }
            _ => {}
        }
    }
    let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
    if f.get_u32(UNIT_FIELD_HEALTH) > max {
        f.set_u32(UNIT_FIELD_HEALTH, max);
    }
}

/// Expire auras, and tick the periodic ones.
pub fn update(map: &mut Map) {
    let now = map.now;
    let mut units: Vec<Guid> = map
        .players
        .values()
        .filter(|p| !p.auras.list.is_empty())
        .map(|p| p.guid)
        .collect();
    units.extend(
        map.creatures
            .iter()
            .filter(|(_, c)| !c.auras.list.is_empty())
            .map(|(_, c)| c.guid),
    );
    for g in units {
        let Some(a) = auras(map, g) else {
            continue;
        };
        let expired: Vec<(u32, Guid)> = a
            .list
            .iter()
            .filter(|x| x.expires.is_some_and(|t| t <= now))
            .map(|x| (x.spell, x.caster))
            .collect();
        let ticks: Vec<Aura> = a
            .list
            .iter()
            .filter(|x| x.next_tick.is_some_and(|t| t <= now))
            .cloned()
            .collect();
        for t in ticks {
            tick(map, g, &t);
            if let Some(a) = auras_mut(map, g)
                && let Some(x) = a
                    .list
                    .iter_mut()
                    .find(|x| x.spell == t.spell && x.caster == t.caster && x.effect == t.effect)
                && let Some(p) = x.period
            {
                x.next_tick = Some(now + p);
            }
        }
        for (spell, caster) in expired {
            remove_spell(map, g, spell, Some(caster));
        }
    }
}

fn tick(map: &mut Map, target: Guid, a: &Aura) {
    let alive = map.unit_view(target).is_some_and(|u| u.alive);
    if !alive {
        return;
    }
    match a.kind {
        AURA_PERIODIC_DAMAGE | AURA_PERIODIC_LEECH => {
            let damage = a.amount.max(0) as u32;
            if let Some(frame) = crate::game::conn::encode(&SMSG_PERIODICAURALOG {
                target,
                caster: a.caster,
                spell: a.spell,
                auras: vec![AuraLog {
                    aura_type: AuraLog_AuraType::PeriodicDamage {
                        damage1: damage,
                        school: SpellSchool::Normal,
                        absorbed: 0,
                        resisted: 0,
                    },
                }],
            }) {
                map.send_to_observers(target, &frame, true);
            }
            crate::game::map::combat::deal_damage(map, a.caster, target, damage);
            if a.kind == AURA_PERIODIC_LEECH {
                crate::game::map::spells::heal(map, a.caster, a.caster, a.spell, damage, true);
            }
        }
        AURA_PERIODIC_HEAL | AURA_OBS_MOD_HEALTH | AURA_MOD_REGEN => {
            let amount = if a.kind == AURA_OBS_MOD_HEALTH {
                let max = map.unit_view(target).map(|_| ()).and_then(|_| {
                    map.unit_fields_mut(target)
                        .map(|f| f.get_u32(UNIT_FIELD_MAXHEALTH))
                });
                max.unwrap_or(0) * a.amount.max(0) as u32 / 100
            } else {
                a.amount.max(0) as u32
            };
            crate::game::map::spells::heal(map, a.caster, target, a.spell, amount, true);
        }
        AURA_PERIODIC_ENERGIZE | AURA_OBS_MOD_MANA | AURA_MOD_POWER_REGEN => {
            if let Some(f) = map.unit_fields_mut(target) {
                let power = a.misc.clamp(0, 4) as u16;
                let max = f.get_u32(UNIT_FIELD_MAXPOWER1 + power);
                let add = if a.kind == AURA_OBS_MOD_MANA {
                    max * a.amount.max(0) as u32 / 100
                } else {
                    a.amount.max(0) as u32
                };
                let now = f.get_u32(UNIT_FIELD_POWER1 + power);
                f.set_u32(UNIT_FIELD_POWER1 + power, (now + add).min(max));
            }
        }
        AURA_PERIODIC_TRIGGER_SPELL if a.trigger != 0 => {
            crate::game::map::spells::cast_triggered(map, a.caster, a.trigger, Some(target));
        }
        _ => {}
    }
}

/// Damage a unit takes, less what its absorb auras soak.
pub fn absorb(map: &mut Map, target: Guid, damage: u32) -> u32 {
    let Some(a) = auras_mut(map, target) else {
        return damage;
    };
    let mut left = damage as i32;
    let mut used_up = Vec::new();
    for x in a.list.iter_mut().filter(|x| x.kind == AURA_SCHOOL_ABSORB) {
        let soak = x.left.min(left).max(0);
        x.left -= soak;
        left -= soak;
        if x.left <= 0 {
            used_up.push((x.spell, x.caster));
        }
        if left <= 0 {
            break;
        }
    }
    for (spell, caster) in used_up {
        remove_spell(map, target, spell, Some(caster));
    }
    left.max(0) as u32
}

/// Whether a unit is held: stunned, or (for `casting`) silenced.
pub fn held(map: &Map, g: Guid, casting: bool) -> bool {
    auras(map, g).is_some_and(|a| {
        a.has(AURA_MOD_STUN)
            || a.has(AURA_MOD_FEAR)
            || a.has(AURA_MOD_CONFUSE)
            || (casting && a.has(AURA_MOD_SILENCE))
    })
}

/// `CMSG_CANCEL_AURA`: a player takes a helpful aura off.
pub fn cancel(map: &mut Map, player: Guid, spell: u32) {
    let helpful = map
        .players
        .get(&player)
        .is_some_and(|p| p.auras.list.iter().any(|a| a.spell == spell && a.positive));
    if helpful {
        remove_spell(map, player, spell, None);
    }
}

/// On death every aura but the passive ones goes, and so does the cast.
pub fn clear_on_death(map: &mut Map, g: Guid) {
    crate::game::map::spells::interrupt(map, g);
    let data = map.realm.data.clone();
    let spells: Vec<u32> = auras(map, g)
        .map(|a| {
            a.list
                .iter()
                .filter(|x| !data.spells.get(x.spell).is_some_and(|s| s.passive()))
                .map(|x| x.spell)
                .collect()
        })
        .unwrap_or_default();
    for spell in spells {
        remove_spell(map, g, spell, None);
    }
}
