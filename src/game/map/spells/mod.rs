//! Spells: learning them, casting them (checks, cast time, power,
//! reagents, cooldowns and the global cooldown, interrupts), and what their
//! effects do when they land ([`outcome`]), with the caster's talents'
//! modifiers ([`mods`]) and their targets ([`targets`]).

pub mod area;
pub mod aura_rank;
pub mod capacity;
pub mod checks;
pub mod class_scripts;
pub mod cooldowns;
pub mod dynamic;
mod effects;
pub use effects::{dr_aura_gone, release_retained_combo_points, retained_combo_points_expired};
pub mod environment;
pub mod health_sync;
pub mod heartbeat;
mod holiday;
pub mod immunity;
pub mod interrupts;
pub mod lists;
pub mod log;
pub mod mods;
pub mod outcome;
pub mod redemption;
pub mod scripts;
pub mod sight;
pub mod single_target;
pub mod stealth;
pub mod targets;
pub mod threat_sync;

use std::time::{Duration, Instant};

use ahash::AHashMap;
use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::items::Body;
use crate::game::data::spells::{self as sd, SpellTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras;
use crate::game::map::combat;

pub use checks::fail;
use mods::Mods;
use outcome::{Hit, Landing};

pub const SMSG_CAST_RESULT: u16 = 0x130;
/// `SPELL_FAILED_REAGENTS`, which wow_messages lacks.
pub const SPELL_FAILED_REAGENTS: u8 = 0x5c;

// Effects.
pub const EFFECT_INSTAKILL: u32 = 1;
/// Honor points: a battleground quest's reward.
pub const EFFECT_ADD_HONOR: u32 = 45;
pub const EFFECT_SCHOOL_DAMAGE: u32 = 2;
pub const EFFECT_DUMMY: u32 = 3;
pub const EFFECT_TELEPORT_UNITS: u32 = 5;
pub const EFFECT_APPLY_AURA: u32 = 6;
pub const EFFECT_ENVIRONMENTAL_DAMAGE: u32 = 7;
pub const EFFECT_POWER_DRAIN: u32 = 8;
pub const EFFECT_HEALTH_LEECH: u32 = 9;
pub const EFFECT_HEAL: u32 = 10;
pub const EFFECT_BIND: u32 = 11;
pub const EFFECT_QUEST_COMPLETE: u32 = 16;
pub const EFFECT_WEAPON_DAMAGE_NOSCHOOL: u32 = 17;
pub const EFFECT_RESURRECT: u32 = 18;
/// `SMSG_RESURRECT_REQUEST`, laid out by hand: wow_messages has its last
/// two bytes as one.
pub const SMSG_RESURRECT_REQUEST_OPCODE: u16 = 0x15B;
pub const EFFECT_CREATE_ITEM: u32 = 24;
pub const EFFECT_WEAPON: u32 = 25;
pub const EFFECT_PERSISTENT_AREA_AURA: u32 = 27;
pub const EFFECT_LEAP: u32 = 29;
pub const EFFECT_ENERGIZE: u32 = 30;
pub const EFFECT_WEAPON_PERCENT_DAMAGE: u32 = 31;
pub const EFFECT_TRIGGER_MISSILE: u32 = 32;
pub const EFFECT_OPEN_LOCK: u32 = 33;
pub const EFFECT_APPLY_AREA_AURA_PARTY: u32 = 35;
pub const EFFECT_LEARN_SPELL: u32 = 36;
pub const EFFECT_DISPEL: u32 = 38;
pub const EFFECT_TELEPORT_UNITS_FACE_CASTER: u32 = 43;
pub const EFFECT_SKILL_STEP: u32 = 44;
pub const EFFECT_WEAPON_DAMAGE: u32 = 58;
pub const EFFECT_PROFICIENCY: u32 = 60;
pub const EFFECT_ADD_FARSIGHT: u32 = 72;
pub const EFFECT_POWER_BURN: u32 = 62;
pub const EFFECT_THREAT: u32 = 63;
pub const EFFECT_TRIGGER_SPELL: u32 = 64;
pub const EFFECT_HEAL_MAX_HEALTH: u32 = 67;
pub const EFFECT_INTERRUPT_CAST: u32 = 68;
pub const EFFECT_DISTRACT: u32 = 69;
pub const EFFECT_HEAL_MECHANICAL: u32 = 75;
pub const EFFECT_SCRIPT_EFFECT: u32 = 77;
pub const EFFECT_ATTACK: u32 = 78;
pub const EFFECT_SANCTUARY: u32 = 79;
pub const EFFECT_ADD_COMBO_POINTS: u32 = 80;
pub const EFFECT_DUEL: u32 = 83;
pub const EFFECT_STUCK: u32 = 84;
pub const EFFECT_CHARGE: u32 = 96;
pub const EFFECT_KNOCK_BACK: u32 = 98;
pub const EFFECT_DISPEL_MECHANIC: u32 = 108;
pub const EFFECT_DURABILITY_DAMAGE: u32 = 111;
pub const EFFECT_RESURRECT_NEW: u32 = 113;
pub const EFFECT_ATTACK_ME: u32 = 114;
pub const EFFECT_DURABILITY_DAMAGE_PCT: u32 = 115;
pub const EFFECT_SKILL: u32 = 118;
pub const EFFECT_NORMALIZED_WEAPON_DMG: u32 = 121;
pub const EFFECT_MODIFY_THREAT_PERCENT: u32 = 125;

/// The effects this module runs.
pub const RUN: &[u32] = &[
    1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 16, 17, 18, 24, 25, 27, 29, 30, 31, 32, 33, 35, 36, 38, 43, 44,
    58, 60, 62, 63, 64, 67, 68, 69, 72, 75, 77, 78, 79, 80, 83, 84, 96, 98, 108, 111, 113, 114,
    115, 118, 121, 125,
];

/// Every effect the server knows: those with an effect here, and the
/// passive and bookkeeping ones that need none (proficiencies' skills,
/// languages, dual wield, defenses), and those other modules run (summons,
/// enchantments, gathering) or keep for later.
pub fn effect_supported(effect: u32) -> bool {
    const KEPT: &[u32] = &[
        0, 4, 12, 13, 14, 15, 19, 20, 21, 22, 23, 26, 28, 34, 37, 39, 40, 41, 42, 45, 46, 47, 48,
        49, 50, 51, 52, 53, 54, 55, 56, 57, 59, 61, 65, 66, 70, 71, 73, 74, 76, 81, 82, 85, 86, 87,
        88, 89, 90, 91, 92, 93, 94, 95, 97, 99, 100, 101, 102, 103, 104, 105, 106, 107, 109, 110,
        112, 116, 117, 119, 120, 122, 123, 124, 126, 127, 128, 129,
    ];
    RUN.contains(&effect) || KEPT.contains(&effect)
}

/// Implicit targets the server resolves.
pub fn target_supported(t: u32) -> bool {
    targets::supported(t)
}

/// What a cast is aimed at.
#[derive(Debug, Clone, Copy, Default)]
pub struct Targets {
    pub unit: Option<Guid>,
    pub object: Option<Guid>,
    pub item: Option<Guid>,
    pub dest: Option<Vector3d>,
}

/// A cast under way.
#[derive(Debug, Clone)]
pub struct Casting {
    pub spell: u32,
    pub targets: Targets,
    pub ends: Instant,
    pub item: Option<Guid>,
    pub pushbacks: u8,
    pub channel_until: Option<Instant>,
    /// The cast's full time, for a channel's pushback.
    pub duration: u32,
}

/// A unit's spellcasting: the cast under way and the cooldowns.
#[derive(Debug, Clone, Default)]
pub struct Caster {
    pub casting: Option<Casting>,
    pub cooldowns: AHashMap<u32, Instant>,
    pub category_cooldowns: AHashMap<u32, Instant>,
    pub gcd_until: Option<Instant>,
    /// For a creature: its spell list's timers in the fight it is in.
    pub spell_list: Option<Box<lists::ListState>>,
    pub combo_target: Guid,
    pub combo_points: u8,
    /// Spells whose cooldown starts when their aura goes (Stealth).
    pub pending_cooldowns: Vec<u32>,
    /// Schools an interrupt locks out: (school mask, until).
    pub school_lockout: Vec<(u32, Instant)>,
    /// A next-swing ability waiting for the next melee swing.
    pub next_swing: Option<u32>,
    /// Diminishing returns on this unit: (group, applications, resets at).
    pub diminishing: Vec<(u8, u8, Option<Instant>)>,
    /// A weapon put on in a fight locks weapon changes until then.
    pub weapon_change_until: Option<Instant>,
}

/// A map's spell state beyond each unit's.
#[derive(Debug, Clone, Default)]
pub struct MapSpells {
    /// Override for rolling outcomes; `None` takes the realm's setting.
    pub rolls: Option<bool>,
    /// Walls that block line of sight, beside the navmesh's (for tests).
    pub walls: Vec<(Vector3d, Vector3d)>,
    /// Persistent area auras: Blizzard, Rain of Fire, Consecration.
    pub areas: Vec<Area>,
    pub area_auras_at: Option<Instant>,
    /// Ground areas' and far sight's dynamic objects.
    pub dynamic: AHashMap<Guid, dynamic::DynamicObject>,
    pub next_dynamic: u32,
    /// Who looks from where (far sight, bind sight): viewer to object.
    pub viewpoints: AHashMap<Guid, Guid>,
    /// When and where each player's surroundings were last looked at.
    pub outdoor_checked: AHashMap<Guid, (Instant, Vector3d)>,
    /// Indoors or out, by player, for tests without a navmesh.
    pub outdoors_override: AHashMap<Guid, bool>,
    /// Spirits of redemption dying, whom the talent doesn't save again.
    pub redeemed: ahash::AHashSet<Guid>,
    /// Override for spells' travel time; `None` takes the realm's setting.
    pub travel: Option<bool>,
    /// Missiles and bolts on their way.
    pub in_flight: Vec<InFlight>,
    /// What units' auras last gave their health (Last Stand, bear form).
    pub health_seen: AHashMap<Guid, health_sync::State>,
    /// When each caster's ground spell (its effect) last ticked each unit.
    pub area_ticked: AHashMap<(Guid, u32, usize, Guid), Instant>,
    /// Threat held for a while: (creature, unit) to its amount (taunts,
    /// Fade).
    pub temp_threat: AHashMap<(Guid, Guid), f32>,
}

/// A persistent area aura on the ground.
#[derive(Debug, Clone)]
pub struct Area {
    pub caster: Guid,
    pub spell: u32,
    pub index: usize,
    pub amount: i32,
    pub at: Vector3d,
    pub radius: f32,
    pub ends: Instant,
    pub next_tick: Instant,
    pub period: Duration,
    /// The dynamic object clients see it by.
    pub object: Guid,
}

/// The spell learning `spell` gives: a trainer's spell may be one whose
/// effect teaches another.
pub fn learns(data: &GameData, spell: u32) -> u32 {
    data.spells
        .get(spell)
        .and_then(|s| {
            s.effects
                .iter()
                .find(|e| e.effect == EFFECT_LEARN_SPELL && e.trigger_spell != 0)
                .map(|e| e.trigger_spell)
        })
        .unwrap_or(spell)
}

/// `player` learns `spell`, and its passive auras take hold; the spells
/// that come with it (`spell_learn_spell`) come too.
pub fn learn(map: &mut Map, player: Guid, spell: u32) {
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    if spell == 0 || p.spells.contains(&spell) {
        return;
    }
    // A higher rank takes the lower's place on the bars; a talent's rank
    // takes it in the spellbook too.
    let (old, talent) = supersedes(&map.realm.data, &p.spells, spell);
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    if talent && let Some(old) = old {
        p.spells.retain(|s| *s != old);
    }
    p.spells.push(spell);
    if let Some(old) = old {
        log::superceded(map, player, old, spell);
    }
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    p.outbound.send(&SMSG_LEARNED_SPELL { id: spell });
    apply_passive(map, player, spell);
    let with = map
        .realm
        .data
        .spells
        .learn_with
        .get(&spell)
        .cloned()
        .unwrap_or_default();
    for s in with {
        learn(map, player, s);
    }
}

/// The known spell `spell` supersedes, as Thistle Tea's `SpellBook` finds
/// it (the skill line's `superseded_by`, or a talent's ranks), and whether
/// it is a talent's.
fn supersedes(data: &GameData, known: &[u32], spell: u32) -> (Option<u32>, bool) {
    let talent = data.spells.talents.values().find_map(|t| {
        let i = t.ranks.iter().position(|r| *r == spell)?;
        (i > 0 && known.contains(&t.ranks[i - 1])).then(|| t.ranks[i - 1])
    });
    if talent.is_some() {
        return (talent, true);
    }
    let line = known.iter().copied().find(|k| {
        data.spells
            .skill_abilities
            .get(k)
            .is_some_and(|abilities| abilities.iter().any(|a| a.superseded_by == spell))
    });
    (line, false)
}

/// Every passive spell a player knows takes hold, as on login, and the
/// auras and cooldowns they logged out with come back.
pub fn apply_passives(map: &mut Map, player: Guid) {
    let spells = map
        .players
        .get(&player)
        .map(|p| p.spells.clone())
        .unwrap_or_default();
    for spell in spells {
        apply_passive(map, player, spell);
    }
    cooldowns::restore(map, player);
    mods::send(map, player);
}

/// A passive spell's effects on its owner.
pub fn apply_passive(map: &mut Map, player: Guid, spell: u32) {
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(spell) else {
        return;
    };
    if !s.passive() || !passive_eligible(s, auras::form(map, player)) {
        return;
    }
    if s.attr(0, sd::ATTR_ONLY_OUTDOORS) && environment::outdoors(map, player) == Some(false) {
        return;
    }
    let level = map.unit_view(player).map(|u| u.level).unwrap_or(1);
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| {
            matches!(
                s.effects[*i].effect,
                EFFECT_APPLY_AURA | EFFECT_APPLY_AREA_AURA_PARTY
            ) && s.effects[*i].aura != 0
        })
        .map(|i| (i, points(s, i, level)))
        .collect();
    if !effects.is_empty() {
        let _ = auras::apply_spell(map, player, player, s, &effects, auras::Context::default());
    }
    for e in s.effects.iter() {
        if e.effect == EFFECT_SKILL
            && let Some(p) = map.players.get_mut(&player)
        {
            crate::game::map::skills::learn_skill(p, e.misc_value.max(0) as u32, 1);
        }
        if matches!(e.effect, EFFECT_WEAPON | EFFECT_PROFICIENCY) {
            effects::proficiency(map, player, s);
        }
    }
}

/// Allowed out of a form though it names forms.
const ATTR_EX2_NOT_NEED_SHAPESHIFT: u32 = 0x8_0000;

/// The forms a passive hangs on (Thistle Tea's `Passive.form_mask`: its
/// stances, and cat form for Feline Swiftness's 24864).
fn form_mask(s: &SpellTemplate) -> u32 {
    if s.id == 24_864 { 1 } else { s.stances }
}

/// Whether a learned passive holds in `form`, as Thistle Tea's
/// `Passive.eligible?` has it: in one of the forms it names, or anywhere if
/// it names none (or out of form, if allowed there).
pub fn passive_eligible(s: &SpellTemplate, form: u8) -> bool {
    let mask = form_mask(s);
    let out_of_form = s.attr(2, ATTR_EX2_NOT_NEED_SHAPESHIFT);
    let in_form = form > 0
        && (s.passive() || s.attr(0, sd::ATTR_HIDDEN))
        && mask & (1 << (form - 1)) != 0
        && !out_of_form;
    in_form || (s.passive() && (mask == 0 || (form == 0 && out_of_form)))
}

/// Whether leaving a form ends a unit's own aura of `s` (Thistle Tea's
/// `Passive.removed_on_shape_lost?`).
pub fn ends_with_form(s: &SpellTemplate) -> bool {
    form_mask(s) != 0
        && !s.attr(2, ATTR_EX2_NOT_NEED_SHAPESHIFT)
        && !s.attr(0, sd::ATTR_NOT_SHAPESHIFTED)
}

/// A player took a form or left one: the learned passives hanging on
/// forms that hold in this one take hold (Thistle Tea's
/// `PassiveSpells.restore` for forms), alive and off a flight.
pub fn restore_form_passives(map: &mut Map, player: Guid) {
    let Some(p) = map.players.get(&player) else {
        return;
    };
    if p.death != crate::game::map::death::Death::Alive || p.flight.is_some() {
        return;
    }
    let mut spells = p.spells.clone();
    spells.sort_unstable();
    let data = map.realm.data.clone();
    for spell in spells {
        if data.spells.get(spell).is_some_and(|s| form_mask(s) != 0)
            && !auras::auras(map, player).is_some_and(|a| a.has_spell(spell))
        {
            apply_passive(map, player, spell);
        }
    }
}

/// An effect's points at `level`: base, a die roll, and the per-level gain.
pub fn points(s: &SpellTemplate, index: usize, level: u32) -> i32 {
    points_with(s, index, level, 0)
}

/// An effect's points with `combo` combo points.
pub fn points_with(s: &SpellTemplate, index: usize, level: u32, combo: u8) -> i32 {
    let e = &s.effects[index];
    let level = level
        .min(if s.max_level > 0 {
            s.max_level
        } else {
            u32::MAX
        })
        .max(s.base_level);
    let units = level.saturating_sub(s.spell_level) as f32;
    let per_level = (e.real_points_per_level * units) as i32;
    let low = e.base_dice.max(1);
    let high = e.die_sides + (e.dice_per_level * units) as i32;
    let dice = if high > low {
        rand::rng().random_range(low..=high)
    } else {
        low
    };
    e.base_points + dice + per_level + (e.points_per_combo_point * f32::from(combo)) as i32
}

pub(crate) fn send_cast_result(map: &Map, caster: Guid, spell: u32, result: Option<u8>) {
    let Some(p) = map.players.get(&caster) else {
        return;
    };
    let mut b = Body::default();
    b.u32(spell);
    match result {
        None => {
            b.u8(0);
        }
        Some(r) => {
            b.u8(2).u8(r);
            // A spell focus failure names the focus.
            if r == checks::fail::REQUIRES_SPELL_FOCUS {
                let focus = map
                    .realm
                    .data
                    .spells
                    .get(spell)
                    .map_or(0, |s| s.requires_spell_focus);
                b.u32(focus);
            }
            // An area failure names the area.
            if r == checks::fail::REQUIRES_AREA {
                b.u32(area::required(&map.realm.data, spell));
            }
        }
    }
    p.outbound.send_frame(b.frame(SMSG_CAST_RESULT));
}

fn power_index(s: &SpellTemplate) -> u16 {
    s.power_type.min(4) as u16
}

/// What `s` costs `caster` before modifiers: its own cost, a share of base
/// mana, or all its mana; with school cost auras.
pub fn power_cost(map: &Map, caster: Guid, s: &SpellTemplate) -> u32 {
    let level = map.unit_view(caster).map(|u| u.level).unwrap_or(1);
    let f = map.unit_fields(caster);
    let mut cost = if s.attr(1, sd::ATTR_EX_USE_ALL_MANA) {
        f.map(|f| f.get_u32(UNIT_FIELD_POWER1 + power_index(s)))
            .unwrap_or(0)
    } else if s.mana_cost_percentage > 0 {
        let base = if s.power_type == 0 {
            f.map(|f| f.get_u32(UNIT_FIELD_BASE_MANA)).unwrap_or(0)
        } else {
            f.map(|f| f.get_u32(UNIT_FIELD_BASE_HEALTH)).unwrap_or(0)
        };
        base * s.mana_cost_percentage / 100
    } else {
        // The rank it is cast at goes no higher than its last level.
        let rank = if s.max_level > 0 {
            level.min(s.max_level)
        } else {
            level
        };
        s.mana_cost + s.mana_cost_per_level * rank.saturating_sub(s.base_level)
    } as i32;
    // A spell that scales with its caster's level costs as much more, as
    // vmangos's `CalculatePowerCost` (and Thistle Tea's `CostScaling`) has it.
    if s.attr(0, sd::ATTR_LEVEL_DAMAGE_CALCULATION) && s.spell_level > 0 && level > 0 {
        let divisor = 1.117 * s.spell_level as f32 / level as f32 - 0.1327;
        cost = if divisor.abs() < f32::EPSILON {
            0
        } else {
            (cost as f32 / divisor) as i32
        };
    }
    if let Some(a) = auras::auras(map, caster) {
        // The shares add, as the client's cost multiplier field does.
        cost += a.sum_mask(auras::AURA_MOD_POWER_COST_SCHOOL, s.school_mask());
        let share = a.sum_mask(auras::AURA_MOD_POWER_COST_SCHOOL_PCT, s.school_mask());
        cost = (cost as f32 * (1.0 + share as f32 / 100.0)) as i32;
    }
    cost.max(0) as u32
}

fn targets_of(t: &SpellCastTargets) -> Targets {
    let f = &t.target_flags;
    Targets {
        unit: f.get_unit().map(|u| u.unit_target),
        // A corpse (Remove Insignia's) goes with the objects.
        object: f
            .get_gameobject()
            .map(|g| match g {
                SpellCastTargets_SpellCastTargetFlags_Gameobject::Gameobject { gameobject }
                | SpellCastTargets_SpellCastTargetFlags_Gameobject::ObjectUnk {
                    object_unk: gameobject,
                } => *gameobject,
            })
            .or_else(|| {
                f.get_corpse().map(|c| match c {
                    SpellCastTargets_SpellCastTargetFlags_Corpse::Corpse { corpse }
                    | SpellCastTargets_SpellCastTargetFlags_Corpse::PvpCorpse {
                        pvp_corpse: corpse,
                    } => *corpse,
                })
            }),
        item: f.get_item().map(|i| match i {
            SpellCastTargets_SpellCastTargetFlags_Item::Item { item }
            | SpellCastTargets_SpellCastTargetFlags_Item::TradeItem { trade_item: item } => *item,
        }),
        dest: f.get_dest_location().map(|d| d.destination),
    }
}

pub fn wire_targets(t: &Targets) -> SpellCastTargets {
    let mut flags = SpellCastTargets_SpellCastTargetFlags::empty();
    if let Some(u) = t.unit {
        flags = flags.set_unit(SpellCastTargets_SpellCastTargetFlags_Unit { unit_target: u });
    }
    if let Some(d) = t.dest {
        flags = flags.set_dest_location(SpellCastTargets_SpellCastTargetFlags_DestLocation {
            destination: d,
        });
    }
    SpellCastTargets {
        target_flags: flags,
    }
}

impl Map {
    pub fn unit_fields(&self, g: Guid) -> Option<&Fields> {
        if let Some(p) = self.players.get(&g) {
            return Some(&p.fields);
        }
        self.creature(g).map(|c| &c.fields)
    }

    pub fn caster_mut(&mut self, g: Guid) -> Option<&mut Caster> {
        if self.players.contains_key(&g) {
            return self.players.get_mut(&g).map(|p| &mut p.caster);
        }
        self.creature_mut(g).map(|c| &mut c.caster)
    }

    pub fn caster(&self, g: Guid) -> Option<&Caster> {
        if let Some(p) = self.players.get(&g) {
            return Some(&p.caster);
        }
        self.creature(g).map(|c| &c.caster)
    }
}

/// `CMSG_CAST_SPELL`.
pub fn cast_request(map: &mut Map, caster: Guid, spell: u32, t: &SpellCastTargets) {
    let targets = targets_of(t);
    cast(map, caster, spell, targets, None);
}

/// The cast time of `s` for `caster`: its own, with modifiers, and the
/// caster's cast speed for spells (abilities don't haste).
fn cast_time(map: &Map, caster: Guid, s: &SpellTemplate, m: &Mods) -> u32 {
    let base = map.realm.data.spells.cast_time(s);
    if base == 0 {
        return 0;
    }
    let t = m.apply(mods::OP_CASTING_TIME, base as f32);
    let speed = if s.ability() || s.attr(0, sd::ATTR_TRADESKILL) {
        1.0
    } else {
        map.unit_fields(caster)
            .map(|f| f.get_f32(UNIT_MOD_CAST_SPEED))
            .filter(|v| *v > 0.0)
            .unwrap_or(1.0)
    };
    (t * speed).max(0.0) as u32
}

/// Cast `spell`: checked, then begun (or done at once if instant).
/// Whether it was.
pub fn cast(
    map: &mut Map,
    caster: Guid,
    spell: u32,
    mut targets: Targets,
    item: Option<Guid>,
) -> bool {
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(spell) else {
        send_cast_result(map, caster, spell, Some(fail::NOT_KNOWN));
        return false;
    };
    if s.passive() {
        return false;
    }
    // A buff too high for its target goes on at a rank that may.
    if item.is_none() && map.players.contains_key(&caster) {
        match aura_rank::chosen(map, caster, s, &targets) {
            Some(id) if id != spell => return cast(map, caster, id, targets, item),
            None => {
                send_cast_result(map, caster, spell, Some(fail::LOWLEVEL));
                return false;
            }
            _ => {}
        }
    }
    let m = mods::for_spell(map, caster, s);
    if crate::game::map::mounts::dismissed(map, caster, s) {
        return true;
    }
    if map.players.contains_key(&caster)
        && !s.attr(0, sd::ATTR_ALLOW_WHILE_MOUNTED)
        && !s.has_aura(auras::AURA_MOUNTED)
        && auras::auras(map, caster).is_some_and(|a| a.has(auras::AURA_MOUNTED))
    {
        auras::remove_where(map, caster, |h| h.has(auras::AURA_MOUNTED));
    }
    if let Some(r) = checks::check(map, caster, s, &targets, item, false, &m) {
        send_cast_result(map, caster, spell, Some(r));
        return false;
    }
    if targets.unit.is_none() && auras::positive(s) {
        targets.unit = Some(caster);
    }
    // A next-swing spell waits for the swing; an auto-repeat one repeats.
    if s.on_next_swing() {
        crate::game::map::combat::queue_next_swing(map, caster, spell);
        return true;
    }
    if s.auto_repeat() {
        crate::game::map::ranged::start(map, caster, spell, targets);
        return true;
    }
    let now = map.now;
    let time = cast_time(map, caster, s, &m);
    if let Some(frame) = crate::game::conn::encode(&SMSG_SPELL_START {
        cast_item: item.unwrap_or_default(),
        caster,
        spell,
        flags: SMSG_SPELL_START_CastFlags::empty(),
        timer: time,
        targets: wire_targets(&targets),
    }) {
        map.send_to_observers(caster, &frame, true);
    }
    if s.start_recovery_category != 0 {
        let gcd = m
            .apply_ms(mods::OP_GLOBAL_COOLDOWN, s.start_recovery_time.max(1000))
            .max(1000);
        if let Some(c) = map.caster_mut(caster) {
            c.gcd_until = Some(now + Duration::from_millis(u64::from(gcd)));
        }
    }
    if time == 0 {
        execute(map, caster, s, targets, item, false);
        return true;
    }
    // Casting breaks what casting breaks (Stealth, Invisibility).
    auras::interrupt(map, caster, auras::INTERRUPT_CAST);
    if let Some(c) = map.caster_mut(caster) {
        c.casting = Some(Casting {
            spell,
            targets,
            ends: now + Duration::from_millis(u64::from(time)),
            item,
            pushbacks: 0,
            channel_until: None,
            duration: time,
        });
    }
    true
}

/// Cast `spell` at once, without its cast time: a next-swing spell at its
/// swing. Whether it went off.
pub fn cast_now(map: &mut Map, caster: Guid, spell: u32, targets: Targets) -> bool {
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(spell) else {
        return false;
    };
    let m = mods::for_spell(map, caster, s);
    if let Some(r) = checks::check(map, caster, s, &targets, None, false, &m) {
        send_cast_result(map, caster, spell, Some(r));
        return false;
    }
    execute(map, caster, s, targets, None, false);
    true
}

/// A spell cast for free, as another spell or an aura's tick does.
pub fn cast_triggered(map: &mut Map, caster: Guid, spell: u32, target: Option<Guid>) {
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(spell) else {
        return;
    };
    let dest = target.and_then(|t| map.position_of(t));
    let targets = Targets {
        unit: target.or(Some(caster)),
        dest,
        ..Default::default()
    };
    execute(map, caster, s, targets, None, true);
}

/// A spell cast for free at `targets`: a trap's on whoever sprang it, at
/// the trap.
pub fn cast_triggered_with(map: &mut Map, caster: Guid, spell: u32, targets: Targets) {
    let data = map.realm.data.clone();
    if let Some(s) = data.spells.get(spell) {
        execute(map, caster, s, targets, None, true);
    }
}

/// A spell cast for free at a place.
pub fn cast_triggered_at(map: &mut Map, caster: Guid, spell: u32, at: Vector3d) {
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(spell) else {
        return;
    };
    let targets = Targets {
        unit: None,
        dest: Some(at),
        ..Default::default()
    };
    execute(map, caster, s, targets, None, true);
}

/// The units a cast lands on, in order, with the effects for each.
fn resolve(
    map: &Map,
    caster: Guid,
    s: &SpellTemplate,
    t: &Targets,
    m: &Mods,
) -> Vec<(Guid, [bool; 3])> {
    let mut out: Vec<(Guid, [bool; 3])> = Vec::new();
    for index in 0..3 {
        // A persistent area ticks on its own; it lands on no one at the cast.
        if matches!(s.effects[index].effect, 0 | EFFECT_PERSISTENT_AREA_AURA) {
            continue;
        }
        for g in targets::effect_targets(map, caster, s, index, t, m) {
            match out.iter_mut().find(|(x, _)| *x == g) {
                Some((_, e)) => e[index] = true,
                None => {
                    let mut e = [false; 3];
                    e[index] = true;
                    out.push((g, e));
                }
            }
        }
    }
    out
}

/// A cast completes: power and reagents taken, cooldowns started, the
/// spell goes off.
fn execute(
    map: &mut Map,
    caster: Guid,
    s: &SpellTemplate,
    targets: Targets,
    item: Option<Guid>,
    triggered: bool,
) {
    let now = map.now;
    let m = mods::for_spell(map, caster, s);
    // The item cast from, by its entry, before it is spent.
    let cast_entry = item.and_then(|g| {
        let p = map.players.get(&caster)?;
        p.item_by_guid(g).map(|i| p.items[i].entry)
    });
    // What the cast cost, for a refund if an ability is avoided.
    let mut paid = 0;
    if !triggered {
        let cost = m.apply_ms(mods::OP_COST, power_cost(map, caster, s));
        paid = cost;
        // A ranged ability (Arcane Shot, Multi-Shot, Aimed Shot) spends a
        // round of ammunition.
        if s.attr(0, sd::ATTR_RANGED) && s.dmg_class == 3 && !s.auto_repeat() {
            let _ = crate::game::map::ranged::pay_ammunition(map, caster);
        }
        if cost > 0 {
            let index = power_index(s);
            if let Some(f) = map.unit_fields_mut(caster) {
                let have = f.get_u32(UNIT_FIELD_POWER1 + index);
                f.set_u32(UNIT_FIELD_POWER1 + index, have.saturating_sub(cost));
            }
            if index == 0
                && let Some(p) = map.players.get_mut(&caster)
            {
                p.mana_spent_at = Some(now);
            }
            if index == 0
                && let Some(c) = map.creature_mut(caster)
            {
                c.engagement.mana_spent_at = Some(now);
            }
        }
        let gm = map.players.get(&caster).is_some_and(|p| p.gm);
        for (reagent, count) in s.reagents.iter().filter(|(r, _)| *r != 0 && !gm) {
            let mut left = *count;
            while left > 0 {
                let Some(i) = map.players.get(&caster).and_then(|p| {
                    p.items
                        .iter()
                        .position(|it| it.entry == *reagent && it.carried())
                }) else {
                    break;
                };
                let n = map.players[&caster].items[i].count().min(left);
                crate::game::map::inventory::take_item(map, caster, i, n);
                left -= n;
            }
        }
        cooldowns::start(map, caster, s, &m, item);
        // The item cast with: its charge spent, its cooldown, binding on use.
        if let Some(i) = item {
            crate::game::map::items::used(map, caster, i, s.id);
        }
    }
    // A finishing move's combo points, on the target they are on.
    let combo = map
        .caster(caster)
        .filter(|c| s.finisher() && targets.unit == Some(c.combo_target))
        .map_or(0, |c| c.combo_points);
    let targets = magnet(map, caster, s, targets);
    let resolved = resolve(map, caster, s, &targets, &m);
    let resolved = if !triggered && item.is_none() && map.players.contains_key(&caster) {
        aura_rank::recipients(map, caster, s, &targets, resolved)
    } else {
        resolved
    };
    let hits: Vec<Guid> = resolved.iter().map(|(g, _)| *g).collect();
    if let Some(frame) = crate::game::conn::encode(&SMSG_SPELL_GO {
        cast_item: item.unwrap_or_default(),
        caster,
        spell: s.id,
        flags: SMSG_SPELL_GO_CastFlags::empty(),
        hits,
        misses: Vec::new(),
        targets: wire_targets(&targets),
    }) {
        map.send_to_observers(caster, &frame, true);
    }
    if !triggered {
        send_cast_result(map, caster, s.id, None);
    }
    if s.channeled() {
        let level = map.unit_view(caster).map(|u| u.level).unwrap_or(1);
        let ms = m.apply_ms(
            mods::OP_DURATION,
            map.realm.data.spells.duration(s, level).unwrap_or(0),
        );
        if let Some(c) = map.caster_mut(caster) {
            c.casting = Some(Casting {
                spell: s.id,
                targets,
                ends: now + Duration::from_millis(u64::from(ms)),
                item,
                pushbacks: 0,
                channel_until: Some(now + Duration::from_millis(u64::from(ms))),
                duration: ms,
            });
        }
        if let Some(f) = map.unit_fields_mut(caster) {
            f.set_u32(UNIT_CHANNEL_SPELL, s.id);
            f.set_guid(UNIT_FIELD_CHANNEL_OBJECT, targets.unit.unwrap_or_default());
        }
        if let Some(p) = map.players.get(&caster) {
            p.outbound.send(&MSG_CHANNEL_START_Server {
                spell: s.id,
                duration: ms,
            });
        }
    }
    // A pet's feeding and abilities, and the professions' effects (which
    // run on their own and are skipped below).
    crate::game::map::pets::before_effects(map, caster, s, &targets);
    let professions = crate::game::map::professions::execute(map, caster, s, &targets, cast_entry);
    // The objects the cast is at, for the quests asking for it there.
    let at_objects = if triggered || crate::game::map::objects::opens_lock(s) {
        Vec::new()
    } else {
        crate::game::map::objects::cast_at(map, caster, s, &targets)
    };
    // Each target: the hit, then the effects.
    let mut chain = [1.0f32; 3];
    let mut landed_on_main = false;
    // A meteor's damage is shared among those it falls on (Thistle Tea's
    // `SharedDamage`).
    let shared = if s.script_name == "spell_meteor" {
        resolved.iter().filter(|(_, e)| e[0]).count() as u32
    } else {
        0
    };
    let launch = Launch {
        m: m.clone(),
        combo,
        item,
        targets,
        triggered,
        paid,
        shared,
    };
    let travels = travels(map) && s.speed > 0.0;
    let one_of = class_scripts::one_of(map, s);
    for (target, effects) in resolved {
        let mut effects = class_scripts::effects_for(s, target, &launch.targets, effects, one_of);
        for (i, e) in effects.iter_mut().enumerate() {
            if professions & (1 << i) != 0 {
                *e = false;
            }
        }
        if !effects.iter().any(|e| *e) {
            continue;
        }
        // A missile or a bolt lands when it gets there (Thistle Tea's
        // `SpellLaunch.delay`).
        let delay = if travels && target != caster {
            flight_ms(map, caster, target, s.speed)
        } else {
            0
        };
        if delay > 0 {
            if Some(target) == launch.targets.unit {
                landed_on_main = true;
            }
            map.spells.in_flight.push(InFlight {
                lands: now + Duration::from_millis(u64::from(delay)),
                caster,
                spell: Box::new(s.clone()),
                target,
                effects,
                chain,
                launch: launch.clone(),
            });
            continue;
        }
        if land(map, caster, s, target, effects, &mut chain, &launch)
            && Some(target) == launch.targets.unit
        {
            landed_on_main = true;
        }
    }
    let targets = launch.targets;
    // Effects with no unit: places, objects.
    effects::run_untargeted(map, caster, s, &targets, &m, combo, professions);
    crate::game::map::quests::cast_credit(map, caster, &at_objects, s.id);
    class_scripts::after_effects(map, caster, s);
    // Holy Nova's heal for a cast of it; Wolfshead Helm's energy however
    // its spell comes.
    if let Some(finish) = class_scripts::finish_trigger(s)
        && (!triggered || s.script_name == "spell_wolfshead_helm")
    {
        cast_triggered(map, caster, finish, None);
    }
    if combo > 0 && (landed_on_main || targets.unit.is_none()) {
        clear_combo_points(map, caster);
    }
    if !triggered {
        mods::spend(map, caster, &m);
        // Casting breaks what casting breaks, but the spell's own aura (and
        // stealth, for what stealth allows and a lucky Improved Sap).
        let stealthy = class_scripts::keeps_stealth(map, caster, s);
        let breaks = |h: &auras::Holder| {
            h.spell != s.id
                && h.interrupt_flags & auras::INTERRUPT_CAST != 0
                && !(stealthy && h.has(auras::AURA_MOD_STEALTH))
        };
        if auras::auras(map, caster).is_some_and(|a| a.holders.iter().any(breaks)) {
            auras::remove_where(map, caster, breaks);
        }
        crate::game::map::combat::spell_launched(map, caster, s);
    }
}

/// What a spell carries to each target it lands on.
#[derive(Debug, Clone)]
pub struct Launch {
    m: Mods,
    combo: u8,
    item: Option<Guid>,
    targets: Targets,
    triggered: bool,
    paid: u32,
    shared: u32,
}

/// A spell on its way to a target.
#[derive(Debug, Clone)]
pub struct InFlight {
    pub lands: Instant,
    pub caster: Guid,
    pub spell: Box<SpellTemplate>,
    pub target: Guid,
    effects: [bool; 3],
    chain: [f32; 3],
    launch: Launch,
}

/// Whether spells take time to get where they go (tests land them at
/// once).
fn travels(map: &Map) -> bool {
    map.spells.travel.unwrap_or(map.realm.config.spell_travel)
}

/// How long something at `speed` yards a second takes from `caster` to
/// `target`: at least five yards' worth.
fn flight_ms(map: &Map, caster: Guid, target: Guid, speed: f32) -> u32 {
    match (map.position_of(caster), map.position_of(target)) {
        (Some(a), Some(b)) => {
            let d = ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt();
            (d.max(5.0) / speed * 1000.0) as u32
        }
        _ => 0,
    }
}

/// The spells in flight that have got there land.
fn land_arrivals(map: &mut Map) {
    let now = map.now;
    if !map.spells.in_flight.iter().any(|f| f.lands <= now) {
        return;
    }
    let (due, waiting): (Vec<InFlight>, Vec<InFlight>) = std::mem::take(&mut map.spells.in_flight)
        .into_iter()
        .partition(|f| f.lands <= now);
    map.spells.in_flight = waiting;
    for mut f in due {
        if map.unit_view(f.target).is_none() {
            continue;
        }
        land(
            map,
            f.caster,
            &f.spell,
            f.target,
            f.effects,
            &mut f.chain,
            &f.launch,
        );
    }
}

/// `s` gets to `target`: the hit, then the effects. Whether it landed.
fn land(
    map: &mut Map,
    caster: Guid,
    s: &SpellTemplate,
    target: Guid,
    effects: [bool; 3],
    chain: &mut [f32; 3],
    l: &Launch,
) -> bool {
    let mut hit = outcome::roll_hit(map, caster, target, s, &l.m);
    // A creature immune to the spell (its template's masks).
    if hit.lands() && crate::game::map::creature_combat::spell_immune(map, target, caster, s) {
        hit = Hit::Immune;
    }
    let mut aim = target;
    if hit == Hit::Reflect {
        // Back at the caster.
        outcome::send_miss(map, caster, target, s.id, Hit::Reflect);
        aim = caster;
        hit = Hit::Hit;
    }
    if !hit.lands() {
        outcome::send_miss(map, caster, aim, s.id, hit);
        // An avoided ability gives back some of its rage or energy.
        if !l.triggered && outcome::physical(s) {
            crate::game::map::combat::ability_avoided(map, caster, s.id, hit.proc_ex(), l.paid);
        }
        let (done, taken) = outcome::proc_flags(s);
        crate::game::map::procs::trigger(
            map,
            crate::game::map::procs::Event {
                actor: caster,
                target: aim,
                actor_flags: done,
                target_flags: taken,
                ex: hit.proc_ex(),
                spell: Some(s.id),
                damage: 0,
                school: s.school_mask(),
                offhand: false,
                ranged: s.dmg_class == 3,
                extra_attack: false,
            },
        );
        contact(map, caster, aim, s, l, false);
        return false;
    }
    // Each harmful effect of a mechanic other than the spell's: immune, or
    // resisted on its own roll (Thistle Tea's `EffectImmunity` and
    // `MechanicResistance`).
    let mut effects = effects;
    if aim != caster && !auras::positive(s) {
        let before = effects.iter().filter(|e| **e).count();
        let mut immune = false;
        for (i, on) in effects.iter_mut().enumerate() {
            if !*on {
                continue;
            }
            let mechanic = s.effects[i].mechanic;
            if mechanic != 0
                && !s.attr(0, sd::ATTR_NO_IMMUNITIES)
                && auras::auras(map, aim).is_some_and(|a| a.mechanic_immune(mechanic))
            {
                *on = false;
                immune = true;
            } else if outcome::effect_resisted(map, aim, s, i) {
                *on = false;
            }
        }
        if before > 0 && !effects.iter().any(|e| *e) {
            let why = if immune { Hit::Immune } else { Hit::Resist };
            outcome::send_miss(map, caster, aim, s.id, why);
            contact(map, caster, aim, s, l, false);
            return false;
        }
    }
    let deals = s
        .effects
        .iter()
        .any(|e| matches!(e.effect, 2 | 7 | 9 | 10 | 17 | 31 | 58 | 67 | 75 | 121));
    let crit = deals
        && if outcome::physical(s) {
            outcome::melee_roll(map, caster, aim, s, &l.m).1
        } else {
            outcome::roll_crit(map, caster, aim, s, &l.m)
        };
    let landing = Landing {
        hit,
        crit,
        chain: 1.0,
        combo: l.combo,
        mods: l.m.clone(),
        reflected: aim != target,
        shared: l.shared,
        triggered: l.triggered,
    };
    // A quest's spell cast at what it hit (Thistle Tea's
    // `queue_quest_cast_credit`).
    if !l.triggered && !crate::game::map::objects::opens_lock(s) {
        crate::game::map::quests::cast_credit(map, caster, &[aim], s.id);
    }
    contact(map, caster, aim, s, l, true);
    effects::run(
        map, caster, aim, s, effects, &landing, chain, l.item, &l.targets,
    );
    class_scripts::landed(map, caster, aim, s);
    crate::game::map::event_ai::spell_hit(map, aim, caster, s.id, s.school_mask());
    // A script for the spell, and the events it sends.
    for (i, e) in s.effects.iter().enumerate() {
        if !effects[i] {
            continue;
        }
        if matches!(e.effect, EFFECT_DUMMY | EFFECT_SCRIPT_EFFECT)
            && map.realm.data.scripts.spell.contains_key(&s.id)
        {
            crate::game::map::scripts::start(
                map,
                crate::game::data::scripts::ScriptTable::Spell,
                s.id,
                caster,
                aim,
            );
        }
        if e.effect == 61 && e.misc_value > 0 {
            crate::game::map::scripts::start(
                map,
                crate::game::data::scripts::ScriptTable::Event,
                e.misc_value as u32,
                caster,
                aim,
            );
        }
    }
    true
}

/// Sap (the rogues' family, flag 0x80): it never starts a fight.
const SAP_FLAG: u64 = 0x80;
/// Seduction: a fight whether its caster is seen or not.
const SEDUCTION: u32 = 6358;
/// `AttributesEx`: no threat; threat only when it misses; a miss breaks
/// the caster's stealth. `AttributesEx2`: no threat as it lands.
const ATTR_EX_NO_THREAT: u32 = 0x400;
const ATTR_EX_THREAT_ONLY_ON_MISS: u32 = 0x20_0000;
const ATTR_EX_FAILURE_BREAKS_STEALTH: u32 = 0x1000;
const ATTR_EX2_NO_INITIAL_THREAT: u32 = 0x40_0000;

/// A harmful spell got to creature `target`, landing (`hit`) or not:
/// whether that puts it in the fight with its caster (Thistle Tea's
/// `Spell.Combat.decide`). Landing, it does if the creature saw the caster
/// (or it is Seduction), the cast was the caster's own (not an aura's or a
/// proc's, unless it travelled) or makes threat outright, and the spell
/// starts fights: not Sap, nor one with no threat, threat only on a miss,
/// or none as it lands. Missing, it does if the creature saw the caster or
/// the miss breaks stealth, and the cast was the caster's own with threat.
/// Damage puts it in the fight anyway (`outcome::deal`).
fn contact(map: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate, l: &Launch, hit: bool) {
    if target == caster
        || !guid::is_unit(target)
        || auras::positive(s)
        || !(guid::is_player(caster) || guid::is_unit(caster))
    {
        return;
    }
    let seen = stealth::sees(map, target, caster);
    let own = !l.triggered || s.speed > 0.0;
    let fights = if hit {
        let sap = s.family_name == 8 && s.family_flags & SAP_FLAG != 0;
        let threat = s.effects.iter().any(|e| {
            matches!(e.effect, EFFECT_THREAT | EFFECT_ATTACK_ME | 91) && e.base_points > 0
        });
        !sap && (seen || s.id == SEDUCTION)
            && (own || threat)
            && !s.attr(1, ATTR_EX_NO_THREAT)
            && !s.attr(1, ATTR_EX_THREAT_ONLY_ON_MISS)
            && !s.attr(2, ATTR_EX2_NO_INITIAL_THREAT)
    } else {
        let breaks = s.attr(1, ATTR_EX_FAILURE_BREAKS_STEALTH);
        (seen || breaks) && (breaks || (own && !s.attr(1, ATTR_EX_NO_THREAT)))
    };
    if fights {
        crate::game::map::ai::attacked(map, target, caster);
        crate::game::map::creature_combat::hostile_act(map, target);
    }
}

/// Whether a spell can be pulled aside by a spell magnet (Grounding
/// Totem), as Thistle Tea's `SpellMagnet.eligible?` says: a harmful magic
/// spell at one target, not a poison, an ability or a passive.
pub fn magnet_eligible(s: &SpellTemplate) -> bool {
    (s.dmg_class == 1 || s.spell_visual == 7250)
        && s.dispel != 4
        && !auras::positive(s)
        && !outcome::area(s)
        && !s.ability()
        && !s.passive()
        && !s.attr(1, 0x8)
        && !s.attr(3, sd::ATTR_EX3_SUPPRESS_TARGET_PROCS)
}

/// A spell at a unit under a spell magnet goes to the magnet's source
/// instead, and the magnet's one charge is spent for everyone it covered.
fn magnet(map: &mut Map, caster: Guid, s: &SpellTemplate, mut t: Targets) -> Targets {
    let Some(target) = t.unit else {
        return t;
    };
    if target == caster || !magnet_eligible(s) {
        return t;
    }
    let source = auras::auras(map, target).and_then(|a| {
        a.holders
            .iter()
            .find(|h| h.has(auras::AURA_SPELL_MAGNET) && h.caster != caster)
            .map(|h| (h.caster, h.spell))
    });
    let Some((source, spell)) = source else {
        return t;
    };
    if map.unit_view(source).is_none_or(|u| !u.alive) {
        return t;
    }
    let covered: Vec<Guid> = map
        .players
        .values()
        .filter(|p| {
            p.auras
                .holders
                .iter()
                .any(|h| h.spell == spell && h.caster == source)
        })
        .map(|p| p.guid)
        .chain(std::iter::once(source))
        .collect();
    for g in covered {
        auras::remove_spell(map, g, spell, Some(source));
    }
    t.unit = Some(source);
    t.dest = map.position_of(source);
    t
}

/// A rogue's or druid's combo points go, and any hold on them.
pub fn clear_combo_points(map: &mut Map, caster: Guid) {
    release_retained_combo_points(map, caster);
    if let Some(c) = map.caster_mut(caster) {
        c.combo_points = 0;
        c.combo_target = Guid::zero();
    }
    if let Some(p) = map.players.get_mut(&caster) {
        p.fields.set_guid(PLAYER_FIELD_COMBO_TARGET, Guid::zero());
        p.fields.set_byte(PLAYER_FIELD_BYTES, 1, 0);
    }
}

/// Threat a spell's damage (or effect) adds beyond the damage itself: its
/// `spell_threat`, the threat modifiers and the caster's threat auras.
pub fn threat(map: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate, damage: f32, m: &Mods) {
    if !guid::is_unit(target) || s.attr(1, sd::ATTR_EX_NO_THREAT) {
        return;
    }
    let (flat, mult) = map
        .realm
        .data
        .spells
        .threat
        .get(&s.id)
        .copied()
        .unwrap_or((0, 1.0));
    let pct = m.pct[mods::OP_THREAT]
        + auras::auras(map, caster)
            .map_or(0, |a| a.sum_mask(auras::AURA_MOD_THREAT, s.school_mask()));
    let total = (damage * mult.max(0.0) + flat as f32) * (100 + pct).max(0) as f32 / 100.0;
    let extra = total - damage;
    if extra.abs() > 0.01 {
        crate::game::map::ai::add_threat(map, target, caster, extra);
    }
}

/// Spell damage from `caster` to `target`, already worked out: logged,
/// absorbed, dealt.
pub fn spell_damage(map: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate, damage: u32) {
    let l = Landing::default();
    outcome::deal(
        map,
        caster,
        target,
        s,
        outcome::Damage {
            amount: damage,
            resisted: 0,
            blocked: 0,
        },
        &l,
    );
}

/// A periodic aura's tick of damage: the target's modifiers, a partial
/// resist (a tenth as likely as a direct hit's), absorbed, logged, dealt;
/// a leech heals its caster by what it took.
pub fn periodic_damage(map: &mut Map, h: &auras::Holder, target: Guid, amount: u32, leech: bool) {
    let data = map.realm.data.clone();
    let s = data
        .spells
        .get(h.spell)
        .cloned()
        .unwrap_or_else(|| SpellTemplate {
            id: h.spell,
            school: h.school.trailing_zeros(),
            ..Default::default()
        });
    let amount = outcome::taken(map, target, &s, amount as f32) as u32;
    let resisted = outcome::partial_resist(map, h.caster, target, &s, amount, true);
    let (left, absorbed) = auras::absorb(map, target, amount - resisted, h.school);
    if let Some(frame) = crate::game::conn::encode(&SMSG_PERIODICAURALOG {
        target,
        caster: h.caster,
        spell: h.spell,
        auras: vec![AuraLog {
            aura_type: AuraLog_AuraType::PeriodicDamage {
                damage1: left,
                school: SpellSchool::try_from(h.school.trailing_zeros().min(6) as u8)
                    .unwrap_or(SpellSchool::Normal),
                absorbed,
                resisted,
            },
        }],
    }) {
        map.send_to_observers(target, &frame, true);
    }
    crate::game::map::procs::trigger(
        map,
        crate::game::map::procs::Event {
            actor: h.caster,
            target,
            actor_flags: crate::game::map::procs::PROC_DONE_PERIODIC,
            target_flags: crate::game::map::procs::PROC_TAKEN_PERIODIC
                | crate::game::map::procs::PROC_TAKEN_DAMAGE,
            ex: crate::game::map::procs::EX_NORMAL_HIT,
            spell: Some(h.spell),
            damage: left,
            school: h.school,
            offhand: false,
            ranged: false,
            extra_attack: false,
        },
    );
    let items = outcome::death_items(map, target, left);
    if left > 0 {
        auras::interrupt(map, target, auras::INTERRUPT_DAMAGE);
        combat::deal_periodic_damage(map, h.caster, target, left, Some(h.spell), s.channeled());
    }
    outcome::give_death_items(map, target, items);
    if leech {
        let mult = s
            .effects
            .iter()
            .find(|e| e.aura == auras::AURA_PERIODIC_LEECH)
            .map(|e| e.multiple_value)
            .filter(|v| *v > 0.0)
            .unwrap_or(1.0);
        heal(
            map,
            h.caster,
            h.caster,
            h.spell,
            (left as f32 * mult) as u32,
            true,
        );
    }
}

/// Heal `target` by `amount`, as `spell`.
pub fn heal(map: &mut Map, caster: Guid, target: Guid, spell: u32, amount: u32, periodic: bool) {
    heal_crit(map, caster, target, spell, amount, periodic, false);
}

/// Heal, perhaps a crit; creatures fighting the target take half the
/// healing done as threat from the healer.
pub fn heal_crit(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    spell: u32,
    amount: u32,
    periodic: bool,
    crit: bool,
) {
    let Some(f) = map.unit_fields_mut(target) else {
        return;
    };
    let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
    let now = f.get_u32(UNIT_FIELD_HEALTH);
    if now == 0 {
        return;
    }
    let gained = amount.min(max.saturating_sub(now));
    f.set_u32(UNIT_FIELD_HEALTH, (now + amount).min(max));
    let frame = if periodic {
        crate::game::conn::encode(&SMSG_PERIODICAURALOG {
            target,
            caster,
            spell,
            auras: vec![AuraLog {
                aura_type: AuraLog_AuraType::PeriodicHeal { damage2: amount },
            }],
        })
    } else {
        crate::game::conn::encode(&SMSG_SPELLHEALLOG {
            victim: target,
            caster,
            id: spell,
            damage: amount,
            critical: crit,
        })
    };
    if let Some(frame) = frame {
        map.send_to_observers(target, &frame, true);
        if caster != target
            && let Some(p) = map.players.get(&caster)
            && !p.known.contains(&target)
        {
            p.outbound.send_frame(frame);
        }
    }
    if gained > 0 && !caster.is_zero() {
        let fighting: Vec<Guid> = map
            .creatures
            .iter()
            .filter(|(_, c)| c.alive() && c.threat.iter().any(|(t, _)| *t == target))
            .map(|(_, c)| c.guid)
            .collect();
        if !fighting.is_empty() {
            let share = gained as f32 * 0.5 / fighting.len() as f32;
            for c in fighting {
                crate::game::map::ai::add_threat(map, c, caster, share);
            }
        }
    }
}

/// A unit's cast is interrupted.
pub fn interrupt(map: &mut Map, g: Guid) {
    let Some(c) = map.caster_mut(g) else {
        return;
    };
    c.next_swing = None;
    let Some(casting) = c.casting.take() else {
        return;
    };
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&SMSG_SPELL_FAILURE {
            guid: g,
            spell: casting.spell,
            result: SpellCastResult::Interrupted,
        });
    }
    if let Some(frame) = crate::game::conn::encode(&SMSG_SPELL_FAILED_OTHER {
        caster: g,
        id: casting.spell,
    }) {
        map.send_to_observers(g, &frame, false);
    }
    if casting.channel_until.is_some() {
        end_channel(map, g, &casting);
    }
}

/// A channel ends: its auras from the caster go, its areas go, and the
/// channel fields clear.
fn end_channel(map: &mut Map, g: Guid, casting: &Casting) {
    let target = casting.targets.unit.unwrap_or(g);
    auras::remove_spell(map, target, casting.spell, Some(g));
    if target != g {
        auras::remove_spell(map, g, casting.spell, Some(g));
    }
    map.spells
        .areas
        .retain(|a| !(a.caster == g && a.spell == casting.spell));
    dynamic::remove_spell(map, g, casting.spell);
    if let Some(f) = map.unit_fields_mut(g) {
        f.set_u32(UNIT_CHANNEL_SPELL, 0);
        f.set_guid(UNIT_FIELD_CHANNEL_OBJECT, Guid::zero());
    }
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&MSG_CHANNEL_UPDATE_Server { time: 0 });
    }
    crate::game::map::objects::channel_ended(map, g, casting.spell);
    crate::game::map::professions::line_ended(map, g);
}

/// Damage taken while casting, as Thistle Tea's `CastPushback` has it: a
/// player's cast whose interrupt flags say so is pushed back half a second
/// (twice at most) or cancelled, and a channel cut short by a quarter or
/// cancelled; talents and auras may resist the pushback.
pub fn pushback(map: &mut Map, g: Guid) {
    let data = map.realm.data.clone();
    if !map.players.contains_key(&g) {
        return;
    }
    let Some((spell, channel)) = map
        .caster(g)
        .and_then(|c| c.casting.as_ref())
        .map(|c| (c.spell, c.channel_until.is_some()))
    else {
        return;
    };
    let Some(s) = data.spells.get(spell) else {
        return;
    };
    let (delays, cancels) = if channel {
        (
            s.channel_interrupt_flags & 0x4000 != 0,
            s.channel_interrupt_flags & 0x2 != 0,
        )
    } else {
        (s.interrupt_flags & 0x2 != 0, s.interrupt_flags & 0x10 != 0)
    };
    if cancels && !delays {
        interrupt(map, g);
        return;
    }
    if !delays {
        return;
    }
    let m = mods::for_spell(map, g, s);
    let resist = auras::auras(map, g).map_or(0, |a| {
        a.sum_mask(auras::AURA_RESIST_PUSHBACK, s.school_mask())
    }) + m.flat[mods::OP_NOT_LOSE_CASTING_TIME]
        + m.pct[mods::OP_NOT_LOSE_CASTING_TIME];
    if resist > 0 && outcome::chance(map, resist as f32) {
        return;
    }
    let now = map.now;
    let mut channel_left = None;
    let mut delayed = None;
    let mut pushed = None;
    if let Some(c) = map.caster_mut(g)
        && let Some(casting) = c.casting.as_mut()
    {
        // A second less a fifth for each hit before, to a fifth (Thistle
        // Tea's `take_pushback_delay`).
        let delay = Duration::from_millis(if casting.pushbacks < 5 {
            1_000 - u64::from(casting.pushbacks) * 200
        } else {
            200
        });
        casting.pushbacks = casting.pushbacks.saturating_add(1);
        match casting.channel_until.as_mut() {
            None => {
                // No further back than the cast's start.
                let left = casting.ends.saturating_duration_since(now);
                let full = Duration::from_millis(u64::from(casting.duration));
                let delta = delay.min(full.saturating_sub(left));
                casting.ends += delta;
                pushed = Some(delta);
            }
            Some(until) => {
                let before = *until;
                let cut = delay.min(until.saturating_duration_since(now));
                *until = until.checked_sub(cut).unwrap_or(now).max(now);
                casting.ends = *until;
                channel_left = Some(until.saturating_duration_since(now).as_millis() as u32);
                delayed = Some((
                    casting.spell,
                    casting.targets.unit.unwrap_or(g),
                    before - *until,
                ));
                pushed = Some(cut);
            }
        }
    }
    let Some(delta) = pushed.filter(|d| !d.is_zero()) else {
        return;
    };
    // The channel's auras end that much sooner too.
    if let Some((spell, target, cut)) = delayed
        && !cut.is_zero()
    {
        auras::delay_source(map, target, spell, g, cut);
    }
    let Some(p) = map.players.get(&g) else {
        return;
    };
    match channel_left {
        Some(left) => {
            p.outbound.send(&MSG_CHANNEL_UPDATE_Server { time: left });
        }
        None => {
            p.outbound
                .send(&wow_world_messages::vanilla::SMSG_SPELL_DELAYED {
                    guid: g,
                    delay_time: delta.as_millis() as u32,
                });
        }
    }
}

/// `CMSG_CANCEL_CAST` and `CMSG_CANCEL_CHANNELLING`.
pub fn cancel(map: &mut Map, g: Guid) {
    interrupt(map, g);
    crate::game::map::combat::cancel_queued(map, g);
}

/// A player moved: a cast that movement interrupts stops.
pub fn moved(map: &mut Map, g: Guid) {
    environment::moved(map, g);
    let Some((spell, channel)) = map
        .caster(g)
        .and_then(|c| c.casting.as_ref())
        .map(|c| (c.spell, c.channel_until.is_some()))
    else {
        return;
    };
    let breaks = map.realm.data.spells.get(spell).is_some_and(|s| {
        if channel {
            s.channel_interrupt_flags & 0x8 != 0 || s.interrupt_flags & 0x1 != 0
        } else {
            s.interrupt_flags & 0x1 != 0
        }
    });
    if breaks {
        interrupt(map, g);
    }
}

/// Casts that finish now go off; channels that end, end; areas tick; party
/// auras follow their casters.
pub fn update(map: &mut Map) {
    let now = map.now;
    let data = map.realm.data.clone();
    let mut done: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.caster.casting.as_ref().is_some_and(|c| c.ends <= now))
        .map(|p| p.guid)
        .collect();
    done.extend(
        map.creatures
            .iter()
            .filter(|(_, c)| c.caster.casting.as_ref().is_some_and(|x| x.ends <= now))
            .map(|(_, c)| c.guid),
    );
    for g in done {
        let Some(casting) = map.caster_mut(g).and_then(|c| c.casting.take()) else {
            continue;
        };
        if casting.channel_until.is_some() {
            // A channel run its course still has its last tick.
            let target = casting.targets.unit.unwrap_or(g);
            auras::tick_due(map, target, casting.spell, g);
            if target != g {
                auras::tick_due(map, g, casting.spell, g);
            }
            end_channel(map, g, &casting);
            // A taming rod's, run its course, tames its beast.
            crate::game::map::pets::rods::channel_done(map, g, casting.spell, casting.targets.unit);
            continue;
        }
        let Some(s) = data.spells.get(casting.spell) else {
            continue;
        };
        // The target must still be here.
        if let Some(t) = casting.targets.unit
            && t != g
            && map.unit_view(t).is_none()
        {
            if let Some(p) = map.players.get(&g) {
                p.outbound.send(&SMSG_SPELL_FAILURE {
                    guid: g,
                    spell: casting.spell,
                    result: SpellCastResult::BadTargets,
                });
            }
            continue;
        }
        execute(map, g, s, casting.targets, casting.item, false);
    }
    effects::tick_areas(map);
    effects::refresh_area_auras(map);
    land_arrivals(map);
    dynamic::update(map);
    sight::update(map);
    for p in map.players.values_mut() {
        p.caster.school_lockout.retain(|(_, until)| *until > now);
        p.caster
            .diminishing
            .retain(|(_, _, reset)| reset.is_none_or(|r| r > now));
    }
}

/// `CMSG_RESURRECT_RESPONSE`: a resurrection offered is taken.
pub fn resurrect_response(map: &mut Map, player: Guid, from: Guid, accept: bool) {
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let Some((caster, health, mana)) = p.resurrect_offer else {
        return;
    };
    // Declined, it is gone; accepted, it must be the one offered (Thistle
    // Tea's `Resurrection.respond`).
    if !accept {
        p.resurrect_offer = None;
        return;
    }
    if caster != from {
        return;
    }
    let to = map.position_of(caster);
    if crate::game::map::death::resurrect_with(map, player, health, mana)
        && let Some(pos) = to
    {
        map.teleport(player, map.key, pos, 0.0);
    }
}

/// A creature in a fight tries a spell from its list.
pub fn creature_cast(map: &mut Map, creature: Guid, victim: Guid) {
    lists::tick(map, creature, victim);
}

/// Talent points a player has to spend: one a level from ten, less those
/// spent.
pub fn free_talent_points(p: &crate::game::map::player::Player, data: &GameData) -> u32 {
    let earned = u32::from(p.level()).saturating_sub(9);
    earned.saturating_sub(spent_talent_points(p, data, None))
}

/// Talent points spent, in all trees or in `tab`.
fn spent_talent_points(
    p: &crate::game::map::player::Player,
    data: &GameData,
    tab: Option<u32>,
) -> u32 {
    data.spells
        .talents
        .values()
        .filter(|t| tab.is_none_or(|tab| t.tab == tab))
        .map(|t| {
            t.ranks
                .iter()
                .rposition(|r| *r != 0 && p.spells.contains(r))
                .map_or(0, |i| i as u32 + 1)
        })
        .sum()
}

pub fn refresh_talent_points(p: &mut crate::game::map::player::Player, data: &GameData) {
    let free = free_talent_points(p, data);
    p.fields.set_u32(PLAYER_CHARACTER_POINTS1, free);
}

/// `CMSG_LEARN_TALENT`: the next rank of a talent, if the player has a
/// point, the row is open and the talent it needs is learned.
pub fn learn_talent(map: &mut Map, player: Guid, talent: u32, rank: u32) {
    let data = map.realm.data.clone();
    let Some(t) = data.spells.talents.get(&talent) else {
        return;
    };
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let have = t
        .ranks
        .iter()
        .rposition(|r| *r != 0 && p.spells.contains(r))
        .map_or(0, |i| i as u32 + 1);
    let Some(&spell) = t.ranks.get(rank as usize) else {
        return;
    };
    if rank != have || spell == 0 || free_talent_points(p, &data) == 0 {
        return;
    }
    if spent_talent_points(p, &data, Some(t.tab)) < t.row * 5 {
        return;
    }
    if t.depends_on != 0
        && let Some(d) = data.spells.talents.get(&t.depends_on)
        && !d
            .ranks
            .iter()
            .skip(t.depends_rank as usize)
            .any(|r| *r != 0 && p.spells.contains(r))
    {
        return;
    }
    // A talent's new rank takes the old one's place.
    if rank > 0 {
        let old = t.ranks[rank as usize - 1];
        auras::remove_spell(map, player, old, Some(player));
    }
    learn(map, player, spell);
    if let Some(p) = map.players.get_mut(&player) {
        refresh_talent_points(p, &data);
    }
}

/// `CMSG_USE_ITEM`: an item's on-use spell.
pub fn use_item(map: &mut Map, player: Guid, bag: u8, slot: u8, t: &SpellCastTargets) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let Some(i) = p.item_at(bag, slot) else {
        return;
    };
    let item = &p.items[i];
    let item_guid = item.guid;
    let Some(template) = data.items.get(&item.entry) else {
        return;
    };
    // Trigger 0: on use.
    let Some((index, spell)) = template
        .spells
        .iter()
        .enumerate()
        .find(|(_, s)| s.id != 0 && s.trigger == 0)
    else {
        return;
    };
    if let Some(code) = crate::game::map::items::check_use(map, player, item_guid, index) {
        crate::game::map::items::send_failure(map, player, spell.id, code);
        return;
    }
    if cooldowns::item_on_cooldown(map, player, spell) {
        send_cast_result(map, player, spell.id, Some(fail::NOT_READY));
        return;
    }
    let mut targets = targets_of(t);
    targets.item = targets.item.or(Some(item_guid));
    cast(map, player, spell.id, targets, Some(item_guid));
}
