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

pub(crate) use effects::dispellable;
pub(crate) use effects::first_clear;
#[cfg(test)]
pub(crate) use effects::leap_end;
#[cfg(test)]
pub(crate) use effects::{Dr, dr_group};
pub use effects::{
    add_combo_points, combo_duration, dr_aura_gone, release_retained_combo_points,
    retained_combo_points_expired, shown_power,
};
pub mod environment;
pub mod frenzied;
pub mod health_sync;
pub mod heartbeat;
pub mod held_item;
mod holiday;
pub mod immunity;
pub mod interrupts;
pub mod lists;
pub mod log;
pub mod mods;
pub mod outcome;
pub mod redemption;
pub mod righteousness;
pub mod script_checks;
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
    /// Whom areas and groups about the caster are about, when not the
    /// caster: the unit an aura that triggers the spell is on.
    pub origin: Option<Guid>,
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
    /// Where the caster stood and faced as it began: a move from there, or
    /// a turn of a tenth of a radian, is what breaks it (mangos's
    /// `m_castPositionX` and `m_castOrientation`).
    pub at: Option<Stance>,
}

/// Where a caster stands and faces, for [`Casting::at`]: aboard a boat or
/// a zeppelin, on its deck, in the transport's own frame (Thistle Tea's
/// `CastMovement.anchor`, the user's call): the ship's own way and turns
/// aren't the caster's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stance {
    pub position: Vector3d,
    pub orientation: f32,
    /// The transport it is measured on, if any.
    pub transport: Option<Guid>,
}

/// Where a caster stands and faces, for [`Casting::at`].
pub fn stance(map: &Map, g: Guid) -> Option<Stance> {
    if let Some(t) = map
        .players
        .get(&g)
        .and_then(|p| p.movement.flags.get_on_transport())
    {
        return Some(Stance {
            position: t.transport.position,
            orientation: t.transport.orientation,
            transport: Some(t.transport.guid),
        });
    }
    map.unit_view(g).map(|u| Stance {
        position: u.position,
        orientation: u.orientation,
        transport: None,
    })
}

/// A unit's spellcasting: the cast under way and the cooldowns.
#[derive(Debug, Clone, Default)]
pub struct Caster {
    pub casting: Option<Casting>,
    pub cooldowns: AHashMap<u32, Instant>,
    pub category_cooldowns: AHashMap<u32, Instant>,
    /// The global cooldown of the spells' category (133).
    pub gcd_until: Option<Instant>,
    /// Global cooldowns of other categories: (category, until).
    pub other_gcds: Vec<(u32, Instant)>,
    /// For a creature: its spell list's timers in the fight it is in.
    pub spell_list: Option<Box<lists::ListState>>,
    pub combo_target: Guid,
    pub combo_points: u8,
    /// Spells whose cooldown starts when their aura goes (Stealth).
    pub pending_cooldowns: Vec<u32>,
    /// The times those wait to start, as the cast had them (an item's
    /// own, for an engineer's Mechanical Dragonling): (spell, recovery,
    /// category, the category's recovery), milliseconds.
    pub pending_times: Vec<(u32, u32, u32, u32)>,
    /// Where each cooldown came from, for the client at login: (spell,
    /// (the item's entry or 0, the category it started)), as mangos's
    /// cooldown entries keep their item and category.
    pub cooldown_sources: AHashMap<u32, (u32, u32)>,
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
    /// Rolls a test lines up for the outcomes' chances, in hundredths of a
    /// percent (0 to 9,999), each taken once before any other.
    #[cfg(test)]
    pub queued_rolls: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<i32>>>,
    /// Walls that block line of sight, beside the navmesh's (for tests).
    pub walls: Vec<(Vector3d, Vector3d)>,
    /// Items' temporary enchantments' modifiers (Improved Weapon Totems'
    /// on a weapon totem's): item to the enchantment and its percent.
    pub enchant_modifiers: AHashMap<Guid, (u32, i32)>,
    /// Combo points owed to a caster whose finisher is landing, given
    /// once it has taken its own: caster to (target, points).
    pub deferred_combo: AHashMap<Guid, Vec<(Guid, u8)>>,
    /// Persistent area auras: Blizzard, Rain of Fire, Consecration.
    pub areas: Vec<Area>,
    pub area_auras_at: Option<Instant>,
    /// Ground areas' and far sight's dynamic objects.
    pub dynamic: AHashMap<Guid, dynamic::DynamicObject>,
    pub next_dynamic: u32,
    /// Who looks from where (far sight, bind sight): viewer to object.
    pub viewpoints: AHashMap<Guid, Guid>,
    /// The viewers whose client brought its camera back to themselves
    /// (`CMSG_FAR_SIGHT` 0), their viewpoint kept for its asking again.
    pub cameras_home: ahash::AHashSet<Guid>,
    /// When and where each player's surroundings were last looked at.
    pub outdoor_checked: AHashMap<Guid, (Instant, Vector3d)>,
    /// Indoors or out, by player, for tests without a navmesh.
    pub outdoors_override: AHashMap<Guid, bool>,
    /// Spirits of redemption dying, whom the talent doesn't save again.
    pub redeemed: ahash::AHashSet<Guid>,
    /// Casters whose gathering, lockpicking or skinning took its roll as
    /// the cast ended, before its go: its effect doesn't roll again.
    pub openings_rolled: ahash::AHashSet<Guid>,
    /// The charged cast-time modifiers (spell, caster) the cast now ending
    /// began with, for its `execute`: only those are spent.
    pub ending_began_with: Option<Vec<(u32, Guid)>>,
    /// A rider whose cast is checked as if off their mount, which it takes
    /// them off if it may be cast.
    pub dismounting: Option<Guid>,
    /// By caster, the charged cast-time modifiers (spell, caster) its cast
    /// under way had its time worked out with (Shadow Trance's), if any:
    /// only those are spent as it ends.
    pub began_with: AHashMap<Guid, Vec<(u32, Guid)>>,
    /// Override for spells' travel time; `None` takes the realm's setting.
    pub travel: Option<bool>,
    /// Missiles and bolts on their way.
    pub in_flight: Vec<InFlight>,
    /// Auto-repeat shots on their way (Auto Shot's arrows, a wand's bolt).
    pub shots: Vec<crate::game::map::ranged::Shot>,
    /// What units' auras last gave their health (Last Stand, bear form).
    pub health_seen: AHashMap<Guid, health_sync::State>,
    /// When each caster's ground spell (its effect) last ticked each unit.
    pub area_ticked: AHashMap<(Guid, u32, usize, Guid), Instant>,
    /// Threat held for a while: (creature, unit) to its amount (taunts,
    /// Fade).
    pub temp_threat: AHashMap<(Guid, Guid), f32>,
    /// When each channel's caster next pays its spell's cost a second
    /// (Health Funnel's health).
    pub channel_costs: AHashMap<Guid, Instant>,
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
    let skills_before = crate::game::map::skills::ids(p);
    // A higher rank takes the lower's place on the bars; a talent's rank
    // takes it in the spellbook too.
    let data = map.realm.data.clone();
    let (old, talent) = supersedes(&data, &p.spells, spell);
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    if talent && old.is_some() {
        let ranks: Vec<u32> = data
            .spells
            .talents
            .values()
            .find(|t| t.ranks.contains(&spell))
            .map(|t| t.ranks.to_vec())
            .unwrap_or_default();
        let gone: Vec<u32> = p
            .spells
            .iter()
            .copied()
            .filter(|s| *s != spell && ranks.contains(s))
            .collect();
        p.spells.retain(|s| *s == spell || !ranks.contains(s));
        // What came with them (Feline Swiftness's hidden dodge) goes too,
        // unless the new rank brings it (mangos's `removeSpell` takes a
        // spell's dependents).
        let keep = data
            .spells
            .learn_with
            .get(&spell)
            .cloned()
            .unwrap_or_default();
        let dependents: Vec<u32> = gone
            .iter()
            .flat_map(|r| data.spells.learn_with.get(r).cloned().unwrap_or_default())
            .filter(|d| !keep.contains(d) && p.spells.contains(d))
            .collect();
        // And their passive auras with them: the ranks never stack (mangos's
        // `removeSpell` of the rank below, Thistle Tea's "without duplicate
        // ranks").
        for r in gone {
            auras::remove_spell(map, player, r, Some(player));
        }
        crate::game::map::misc::forget_spells(map, player, &dependents);
    }
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    p.spells.push(spell);
    if let Some(old) = old {
        log::superceded(map, player, old, spell);
    }
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    p.outbound.send(&SMSG_LEARNED_SPELL { id: spell });
    apply_passive(map, player, spell);
    let data = map.realm.data.clone();
    if let Some(p) = map.players.get_mut(&player) {
        crate::game::map::skills::learn_from_spell(p, &data, spell);
        crate::game::map::skills::learn_from_line(p, &data, spell);
        crate::game::map::skills::learn_weapon_lines(p, &data, spell);
        crate::game::map::skills::learn_language(p, &data, spell);
    }
    // The spells a skill it brought comes with (Mining's Find Minerals and
    // Smelt Copper, Bows' Shoot Bow).
    let rewards: Vec<u32> = map.players.get(&player).map_or_else(Vec::new, |p| {
        crate::game::map::skills::ids(p)
            .into_iter()
            .filter(|s| !skills_before.contains(s))
            .flat_map(|s| crate::game::map::skills::rewards(&data, s, p.row.race, p.row.class))
            .collect()
    });
    for r in rewards {
        learn(map, player, r);
    }
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
    // A talent's rank: the highest other rank known, below or above it
    // (mangos's `addSpell` unlearns every other rank).
    let talent = data.spells.talents.values().find_map(|t| {
        t.ranks.iter().position(|r| *r == spell)?;
        t.ranks
            .iter()
            .rev()
            .find(|r| **r != 0 && **r != spell && known.contains(r))
            .copied()
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

/// Whether known `spell` is a rank a known higher one has taken the place
/// of in the spellbook: its skill line's `superseded_by` is known. The
/// client's data names one only for ranks that don't stack in the
/// spellbook (a rage or energy spell's, not a mage's Frostbolt). Such a
/// rank is known (a trainer counts it) but not shown at login nor cast at
/// the client's request: mangos-classic's inactive spell (`addSpell`,
/// `GetSpellBookSuccessorSpellId`, `HasActiveSpell`).
pub fn superseded_in_book(data: &GameData, known: &[u32], spell: u32) -> bool {
    data.spells.skill_abilities.get(&spell).is_some_and(|v| {
        v.iter()
            .any(|a| a.superseded_by != 0 && known.contains(&a.superseded_by))
    })
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
    default_stance(map, player);
    mods::send(map, player);
}

/// Battle Stance, the warrior's first.
pub const BATTLE_STANCE: u32 = 2457;

/// A warrior in no stance at login takes Battle Stance, if they know it
/// (mangos's `_LoadAuras`, Thistle Tea's `apply_default_auras`): a new
/// one, or one whose stance was lost; one in another keeps it.
fn default_stance(map: &mut Map, player: Guid) {
    let Some(p) = map.players.get(&player) else {
        return;
    };
    if p.row.class != 1
        || !p.spells.contains(&BATTLE_STANCE)
        || p.auras.has(auras::AURA_MOD_SHAPESHIFT)
        || p.death != crate::game::map::death::Death::Alive
    {
        return;
    }
    cast_triggered(map, player, BATTLE_STANCE, Some(player));
}

/// A passive spell's effects on its owner.
pub fn apply_passive(map: &mut Map, player: Guid, spell: u32) {
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(spell) else {
        return;
    };
    // A passive, or a hidden spell of the form the player is in (Feline
    // Swiftness's dodge: mangos's `IsNeedCastSpellAtFormApply` takes
    // `SPELL_ATTR_PASSIVE` or `SPELL_ATTR_DO_NOT_DISPLAY`).
    let form = auras::form(map, player);
    let hidden = s.attr(0, sd::ATTR_HIDDEN) && form > 0;
    if !(s.passive() || hidden) || !passive_eligible(s, form) {
        return;
    }
    if s.attr(0, sd::ATTR_ONLY_OUTDOORS) && environment::outdoors(map, player) == Some(false) {
        return;
    }
    let level = map.unit_view(player).map(|u| u.level).unwrap_or(1);
    // Its points with the player's spell modifiers to it, as any spell's
    // (mangos's `CalculateSpellEffectValue`).
    let m = mods::for_spell(map, player, s);
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| {
            matches!(
                s.effects[*i].effect,
                EFFECT_APPLY_AURA | EFFECT_APPLY_AREA_AURA_PARTY
            ) && s.effects[*i].aura != 0
        })
        .map(|i| (i, m.points(s, i, points(s, i, level))))
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
        && mask & checks::bit(u32::from(form)) != 0
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

/// The projectile a player's spell of the ranged slot shows (Aimed Shot,
/// a throw), for its start and its go.
fn ranged_ammo(map: &Map, caster: Guid, s: &SpellTemplate) -> Option<(u32, u32)> {
    if !s.attr(0, crate::game::data::spells::ATTR_RANGED) {
        return None;
    }
    crate::game::map::ranged::ammo_shown(map, caster)
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
            // A gear failure names the gear: its class, subclasses and
            // inventory types (mangos-zero's `SendCastResult`).
            if r == checks::fail::EQUIPPED_ITEM_CLASS {
                let s = map.realm.data.spells.get(spell);
                b.i32(s.map_or(-1, |s| s.equipped_item_class))
                    .i32(s.map_or(0, |s| s.equipped_item_subclass_mask))
                    .i32(s.map_or(0, |s| s.equipped_item_inventory_type_mask));
            }
        }
    }
    p.outbound.send_frame(b.frame(SMSG_CAST_RESULT));
}

/// Power type -2: a cost paid in health (Life Tap, Health Funnel,
/// Bloodrage).
pub const POWER_HEALTH: u32 = 0xFFFF_FFFE;

/// The power `s` costs, as an offset from `UNIT_FIELD_POWER1`; none for a
/// cost in health.
fn power_index(s: &SpellTemplate) -> Option<u16> {
    (s.power_type < 5).then_some(s.power_type as u16)
}

/// What `s` costs `caster` before modifiers: its own cost and a share of
/// base mana (of base health, or of the power's maximum), or all it has;
/// with school cost auras.
pub fn power_cost(map: &Map, caster: Guid, s: &SpellTemplate) -> u32 {
    spell_cost(map, caster, s, &mods::Mods::default())
}

/// What `s` costs `caster`, in mangos's `CalculatePowerCost`'s order: all
/// the power there is for a spell that takes it all, nothing changing it;
/// else its own cost and share, the school's flat change, the caster's
/// spell modifiers `m`, the level's scaling, then the school's share.
pub fn spell_cost(map: &Map, caster: Guid, s: &SpellTemplate, m: &mods::Mods) -> u32 {
    let level = map.unit_view(caster).map(|u| u.level).unwrap_or(1);
    let f = map.unit_fields(caster);
    let field = |field: u16| f.map(|f| f.get_u32(field)).unwrap_or(0);
    if s.attr(1, sd::ATTR_EX_USE_ALL_MANA) {
        return power_index(s).map_or_else(
            || field(UNIT_FIELD_HEALTH),
            |i| field(UNIT_FIELD_POWER1 + i),
        );
    }
    let a = auras::auras(map, caster);
    let mut cost = {
        // The rank it is cast at: a creature's level, a player's skill in
        // the spell's line (none without one), no higher than its last
        // level's; its cost per level by the rank's fifths past its first
        // level (mangos's `CalculatePowerCost` and `GetSpellRank`, Thistle
        // Tea's `CostScaling.base_cost`).
        let rank = match map.players.get(&caster) {
            Some(p) => map
                .realm
                .data
                .spells
                .spell_skills
                .get(&s.id)
                .map_or(0, |skill| {
                    crate::game::map::skills::value_with_bonuses(p, *skill)
                }),
            None => level,
        };
        let rank = if s.max_level > 0 {
            rank.min(s.max_level * 5)
        } else {
            rank
        };
        let flat = s.mana_cost as i32
            + s.mana_cost_per_level as i32 * (rank as i32 / 5 - s.base_level as i32);
        // A share is on top of the flat cost (mangos's
        // `Spell::CalculatePowerCost`, Thistle Tea's `Resources`).
        let share = if s.mana_cost_percentage > 0 {
            let base = match power_index(s) {
                Some(0) => field(UNIT_FIELD_BASE_MANA),
                Some(i) => field(UNIT_FIELD_MAXPOWER1 + i),
                None => match field(UNIT_FIELD_BASE_HEALTH) {
                    0 => field(UNIT_FIELD_MAXHEALTH),
                    base => base,
                },
            };
            base * s.mana_cost_percentage / 100
        } else {
            0
        };
        flat + share as i32
    };
    if let Some(a) = a {
        cost += a.sum_mask(auras::AURA_MOD_POWER_COST_SCHOOL, s.school_mask());
    }
    cost = m.apply(mods::OP_COST, cost as f32) as i32;
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
    if let Some(a) = a {
        // The shares add, as the client's cost multiplier field does.
        let share = a.sum_mask(auras::AURA_MOD_POWER_COST_SCHOOL_PCT, s.school_mask());
        cost = (cost as f32 * (1.0 + share as f32 / 100.0)) as i32;
    }
    cost.max(0) as u32
}

/// Why `caster` can't pay `cost` of `s`'s power, as Thistle Tea's
/// `Resources.validate_cast_cost` and mangos's `Spell::CheckPower` have it:
/// a cost in health needs more health than it takes (`CASTER_AURASTATE`),
/// any other as much power (`NO_POWER`). An ordinary creature, not a pet,
/// pays no rage, focus, energy or happiness, nor mana when it has none.
pub(crate) fn unpayable(map: &Map, caster: Guid, s: &SpellTemplate, cost: u32) -> Option<u8> {
    if cost == 0 || crate::game::map::gm_dev::invulnerable(map, caster) {
        return None;
    }
    if power_index(s) == Some(0) && crate::game::map::gm_dev::infinite_mana_of(map, caster) {
        return None;
    }
    let f = map.unit_fields(caster)?;
    let Some(index) = power_index(s) else {
        return (f.get_u32(UNIT_FIELD_HEALTH) <= cost).then_some(fail::CASTER_AURASTATE);
    };
    if let Some(c) = map.creature(caster)
        && c.pet
            .as_ref()
            .is_none_or(|p| p.kind == crate::game::map::pets::Kind::Charmed)
        && (index > 0 || c.base.max_mana == 0)
    {
        return None;
    }
    (f.get_u32(UNIT_FIELD_POWER1 + index) < cost).then_some(fail::NO_POWER)
}

/// `caster` pays `cost` of `s`'s power: health (never the last point), or
/// the power, as much of it as there is.
fn pay(map: &mut Map, caster: Guid, s: &SpellTemplate, cost: u32) {
    let now = map.now;
    // A GM in god mode pays nothing (Thistle Tea's `Resources.spend_power`).
    if crate::game::map::gm_dev::invulnerable(map, caster) {
        return;
    }
    // `.infinitemana`: no mana taken.
    if power_index(s) == Some(0) && crate::game::map::gm_dev::infinite_mana_of(map, caster) {
        return;
    }
    let Some(index) = power_index(s) else {
        if let Some(f) = map.unit_fields_mut(caster) {
            let health = f.get_u32(UNIT_FIELD_HEALTH);
            f.set_u32(UNIT_FIELD_HEALTH, health.saturating_sub(cost).max(1));
        }
        combat::sync_aura_state(map, caster);
        return;
    };
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

/// The unit a cast is aimed at, if any: the 1.12.1 client writes one
/// object's guid for a unit, a game object or a corpse, and only the
/// first is a unit.
pub(crate) fn unit_of(t: &SpellCastTargets) -> Option<Guid> {
    match t.target_flags.get_unit()? {
        SpellCastTargets_SpellCastTargetFlags_Unit::Unit { unit_target } => Some(*unit_target),
        _ => None,
    }
}

pub(crate) fn targets_of(t: &SpellCastTargets) -> Targets {
    use SpellCastTargets_SpellCastTargetFlags_Unit as U;
    let f = &t.target_flags;
    Targets {
        unit: unit_of(t),
        // A corpse (Remove Insignia's) goes with the objects.
        object: f.get_unit().and_then(|o| match o {
            U::Unit { .. } => None,
            U::Gameobject { gameobject: o }
            | U::Corpse { corpse: o }
            | U::PvpCorpse { pvp_corpse: o } => Some(*o),
        }),
        item: f.get_item().map(|i| match i {
            SpellCastTargets_SpellCastTargetFlags_Item::Item { item }
            | SpellCastTargets_SpellCastTargetFlags_Item::TradeItem { trade_item: item } => *item,
        }),
        // The ground a cast is at: its destination, else the source the
        // client sent (mangos's `m_srcPos`, Thistle Tea's
        // `Target.ground_location`).
        dest: f
            .get_dest_location()
            .map(|d| d.destination)
            .or_else(|| f.get_source_location().map(|s| s.source)),
        origin: None,
    }
}

pub fn wire_targets(t: &Targets) -> SpellCastTargets {
    let mut flags = SpellCastTargets_SpellCastTargetFlags::empty();
    if let Some(u) = t.unit {
        flags = flags.set_unit(SpellCastTargets_SpellCastTargetFlags_Unit::Unit { unit_target: u });
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
        // Every cast begins here (see `Map::casting`).
        if self.creature_by_guid.contains_key(&g) {
            self.casting.insert(g);
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

/// `CMSG_CAST_SPELL`: a spell the player knows. Only the client's request
/// asks that (mangos's `HandleCastSpellOpcode`); a script's or an item's
/// cast is the player's whether they know it or not.
pub fn cast_request(map: &mut Map, caster: Guid, spell: u32, t: &SpellCastTargets) {
    let data = &map.realm.data;
    if map
        .players
        .get(&caster)
        .is_some_and(|p| !p.spells.contains(&spell) || superseded_in_book(data, &p.spells, spell))
    {
        send_cast_result(map, caster, spell, Some(fail::NOT_KNOWN));
        return;
    }
    let targets = targets_of(t);
    cast(map, caster, spell, targets, None);
}

/// The cast time of `s` for `caster`: its own, with modifiers, and the
/// caster's cast speed for spells (abilities don't haste).
pub(crate) fn cast_time(map: &Map, caster: Guid, s: &SpellTemplate, m: &Mods) -> u32 {
    let base = map.realm.data.spells.cast_time(s);
    if base == 0 {
        return 0;
    }
    let t = m.apply(mods::OP_CASTING_TIME, base as f32);
    let speed = if s.attr(0, sd::ATTR_RANGED) && !s.auto_repeat() {
        // A ranged ability (Aimed Shot, Multi-Shot) by the ranged haste
        // (mangos's `GetSpellCastTime`: `m_modAttackSpeedPct[RANGED]`).
        auras::auras(map, caster).map_or(1.0, |a| {
            crate::game::map::derive::period_multiplier(
                a,
                &[
                    auras::AURA_MOD_RANGED_HASTE,
                    auras::AURA_MOD_ATTACKSPEED,
                    auras::AURA_MOD_RANGED_AMMO_HASTE,
                ],
            )
        })
    } else if s.ability() || s.attr(0, sd::ATTR_TRADESKILL) {
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
    // A finisher aimed at no one, or at its caster (Slice and Dice), is at
    // the unit its combo points are on.
    if s.finisher()
        && targets.unit.is_none_or(|u| u == caster)
        && let Some(c) = map.caster(caster)
        && c.combo_points > 0
        && map.unit_view(c.combo_target).is_some()
    {
        targets.unit = Some(c.combo_target);
    }
    let m = mods::for_spell(map, caster, s);
    if crate::game::map::mounts::dismissed(map, caster, s) {
        return true;
    }
    // A rider is taken off their mount by a cast that may be cast once off
    // it, and only then: one that fails its checks (on cooldown, out of
    // range) leaves them mounted. The checks take them as off it.
    let dismount = map.players.contains_key(&caster)
        && !s.attr(0, sd::ATTR_ALLOW_WHILE_MOUNTED)
        && !s.has_aura(auras::AURA_MOUNTED)
        && auras::auras(map, caster).is_some_and(|a| a.has(auras::AURA_MOUNTED));
    if dismount {
        map.spells.dismounting = Some(caster);
    }
    let refused = checks::check(map, caster, s, &targets, item, false, &m);
    map.spells.dismounting = None;
    if let Some(r) = refused {
        send_cast_result(map, caster, spell, Some(r));
        return false;
    }
    if dismount {
        auras::remove_where(map, caster, |h| h.has(auras::AURA_MOUNTED));
    }
    // At oneself, unless aimed at a body to raise.
    if targets.unit.is_none()
        && auras::positive(s)
        && !((s.has_effect(EFFECT_RESURRECT) || s.has_effect(EFFECT_RESURRECT_NEW))
            && crate::game::map::corpses::aimed_at(&targets).is_some())
    {
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
    // A spell cast while channelling ends the channel, unless either may
    // be cast with another (mangos's `SetCurrentCastedSpell`).
    let channel = map
        .caster(caster)
        .and_then(|c| c.casting.as_ref())
        .filter(|c| c.channel_until.is_some())
        .map(|c| c.spell);
    if let Some(channel) = channel
        && !s.attr(4, sd::ATTR_EX4_ALLOW_CAST_WHILE_CASTING)
        && !data
            .spells
            .get(channel)
            .is_some_and(|c| c.attr(4, sd::ATTR_EX4_ALLOW_CAST_WHILE_CASTING))
    {
        interrupt(map, caster);
    }
    // Another spell cast stops a wand (mangos's `SetCurrentCastedSpell`,
    // `SPELL_ATTR_EX3_CASTING_CANCELS_AUTOREPEAT` on Shoot); Auto Shot
    // goes on.
    crate::game::map::ranged::cast_begun(map, caster);
    let now = map.now;
    let time = cast_time(map, caster, s, &m);
    // A spell of the ranged slot shows its projectile (mangos's
    // CAST_FLAG_AMMO, set for `SPELL_ATTR_USES_RANGED_SLOT`).
    let mut flags = SMSG_SPELL_START_CastFlags::empty();
    if let Some((display, inventory_type)) = ranged_ammo(map, caster, s) {
        flags = flags.set_ammo(SMSG_SPELL_START_CastFlags_Ammo {
            ammo_display_id: display,
            ammo_inventory_type: inventory_type,
        });
    }
    // Its caster in the item's place when no item cast it, as the go.
    if let Some(frame) = crate::game::conn::encode(&SMSG_SPELL_START {
        cast_item: item.unwrap_or(caster),
        caster,
        spell,
        flags,
        timer: time,
        targets: wire_targets(&targets),
    }) {
        map.send_to_observers(caster, &frame, true);
    }
    // The global cooldown is the spell's start time with its modifiers:
    // none for one without (vmangos, mangos's `Player::AddGCD`, Thistle
    // Tea's `Cooldowns.gcd_duration`).
    // A creature's is its spell's own (mangos's `GetSpellModOwner` is
    // nobody for it, Thistle Tea's creatures keep their spells' timing).
    let gcd = if guid::is_player(caster) {
        m.apply_ms(mods::OP_GLOBAL_COOLDOWN, s.start_recovery_time)
    } else {
        s.start_recovery_time
    };
    if s.start_recovery_category != 0
        && gcd > 0
        && let Some(c) = map.caster_mut(caster)
    {
        let until = now + Duration::from_millis(u64::from(gcd));
        if s.start_recovery_category == GCD_SPELLS {
            c.gcd_until = Some(until);
        } else {
            c.other_gcds
                .retain(|(cat, t)| *cat != s.start_recovery_category && *t > now);
            c.other_gcds.push((s.start_recovery_category, until));
        }
    }
    if time == 0 {
        // An instant's launch is its end: its gathering's roll too, before
        // its go (mangos's `CheckCast(false)` in `Spell::cast`).
        if let Some(code) =
            crate::game::map::professions::roll_at_finish(map, caster, s, &targets, item)
        {
            launch_failed(map, caster, spell, code);
            return false;
        }
        execute(map, caster, s, targets, item, false);
        map.spells.openings_rolled.remove(&caster);
        return true;
    }
    // Beginning a cast breaks what an action breaks at once; 1.12's Stealth
    // and Invisibility go only at its launch (mangos-classic's
    // `AURA_INTERRUPT_FLAG_ACTION` at `Spell::prepare`).
    let stealthy = class_scripts::allows_stealth(s);
    action_breaks(map, caster, s, auras::INTERRUPT_ACTION, stealthy);
    let at = stance(map, caster);
    if let Some(c) = map.caster_mut(caster) {
        c.casting = Some(Casting {
            spell,
            targets,
            ends: now + Duration::from_millis(u64::from(time)),
            item,
            pushbacks: 0,
            channel_until: None,
            duration: time,
            at,
        });
    }
    let began: Vec<(u32, Guid)> = m
        .charged
        .iter()
        .filter(|c| c.2 == mods::OP_CASTING_TIME)
        .map(|c| (c.0, c.1))
        .collect();
    if began.is_empty() {
        map.spells.began_with.remove(&caster);
    } else {
        map.spells.began_with.insert(caster, began);
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

thread_local! {
    /// Whether the cast starting now is an aura's (see [`cast_by_aura`]).
    static BY_AURA: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The next cast an aura's, while it lives: put back as it goes, a panic's
/// unwinding too.
struct ByAura(bool);

impl ByAura {
    fn enter() -> Self {
        Self(BY_AURA.replace(true))
    }
}

impl Drop for ByAura {
    fn drop(&mut self) {
        BY_AURA.set(self.0);
    }
}

/// A spell an aura casts for free: its periodic trigger's (Arcane
/// Missiles' missiles, Hellfire's flames), a target trigger's (Frostbite).
/// It procs and starts fights as an aura's, not a cast's (mangos's
/// `m_triggeredByAuraSpell`, Thistle Tea's aura origin); a proc's spell is
/// one too, cast while the proc goes off.
pub fn cast_by_aura(map: &mut Map, caster: Guid, spell: u32, target: Option<Guid>) {
    let _aura = ByAura::enter();
    cast_triggered(map, caster, spell, target);
}

/// [`cast_by_aura`] at `targets`.
pub fn cast_by_aura_with(map: &mut Map, caster: Guid, spell: u32, targets: Targets) {
    let _aura = ByAura::enter();
    cast_triggered_with(map, caster, spell, targets);
}

/// A spell cast for free, as another spell or an aura's tick does.
pub fn cast_triggered(map: &mut Map, caster: Guid, spell: u32, target: Option<Guid>) {
    cast_triggered_from(map, caster, spell, target, None);
}

/// [`cast_triggered`], of the item a spell was cast from: its go names
/// the item, as mangos's `CastSpell(.., TRIGGERED_OLD_TRIGGERED,
/// m_CastItem)` passes it on (an engineer's trinket's summon). Nothing of
/// the item is spent.
pub fn cast_triggered_from(
    map: &mut Map,
    caster: Guid,
    spell: u32,
    target: Option<Guid>,
    item: Option<Guid>,
) {
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(spell) else {
        return;
    };
    cast_template_from(map, caster, s, target, item);
}

/// [`cast_triggered_from`] of a template a script made.
pub fn cast_template_from(
    map: &mut Map,
    caster: Guid,
    s: &SpellTemplate,
    target: Option<Guid>,
    item: Option<Guid>,
) {
    let dest = target.and_then(|t| map.position_of(t));
    let targets = Targets {
        unit: target.or(Some(caster)),
        dest,
        ..Default::default()
    };
    execute(map, caster, s, targets, item, true);
}

/// `s`'s reagents taken from what `caster` carries (none from a GM).
fn take_reagents(map: &mut Map, caster: Guid, s: &SpellTemplate) {
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
}

/// Whether a player's `trigger`, set off by an aura of `by`, may go: one
/// whose aura spell has no reagent pays its own, and doesn't go without
/// them, as mangos's `IgnoreItemRequirements` has it (a triggered spell's
/// reagents are its trigger's to pay when that has some in its first slot;
/// Thistle Tea's `pays_reagents?`). Will Drain Festive Mug's drain takes
/// the Filled Festive Mug it empties; a wind stone's summon, the Twilight
/// set. Paid as it goes.
pub fn trigger_reagents(map: &mut Map, caster: Guid, by: &SpellTemplate, trigger: u32) -> bool {
    let data = map.realm.data.clone();
    let Some(t) = data.spells.get(trigger) else {
        return true;
    };
    if by.reagents[0].0 != 0 || t.reagents.iter().all(|(r, _)| *r == 0) {
        return true;
    }
    let Some(p) = map.players.get(&caster).filter(|p| !p.gm) else {
        return true;
    };
    let carried = t
        .reagents
        .iter()
        .filter(|(r, _)| *r != 0)
        .all(|(r, n)| p.carried_count_of(*r) >= *n);
    if carried {
        take_reagents(map, caster, t);
    }
    carried
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

/// A cast of an effect at a script's place near its caster (46) is at
/// that place, the first such effect's, whatever place the cast was sent
/// (mangos-classic's `CheckScriptTargeting` setting the destination; a
/// later effect at the same target is ignored, `SpellTargetMgr`'s
/// `ignoredTargets`): its summons, objects and areas are there, and its
/// go says so. With no place found, the cast keeps its own.
fn script_dest(map: &Map, caster: Guid, s: &SpellTemplate, mut t: Targets) -> Targets {
    let place = (0..3)
        .find(|i| {
            let e = &s.effects[*i];
            e.effect != 0 && (e.target_a == 46 || e.target_b == 46)
        })
        .and_then(|i| targets::script_place(map, caster, s, i, &t));
    if let Some(p) = place {
        t.dest = Some(p);
    }
    t
}

/// A cast of an effect at its spell's database place (17) is at that
/// place, if its `spell_target_position` row is on this map, whatever
/// place the cast was sent or a trigger gave it: its summons, its ground
/// and its areas are there, and its go says so (mangos's `SetTargetMap`
/// for `TARGET_LOCATION_DATABASE`, Thistle Tea's database locations). A
/// row on another map is a teleport's own; with none, the cast keeps its
/// place.
fn database_place(map: &Map, s: &SpellTemplate, mut t: Targets) -> Targets {
    if s.effects
        .iter()
        .any(|e| e.effect != 0 && (e.target_a == 17 || e.target_b == 17))
        && let Some(&(m, x, y, z, _)) = map.realm.data.spells.target_positions.get(&s.id)
        && m == map.key.map
    {
        t.dest = Some(Vector3d { x, y, z });
    }
    t
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
    let mut chosen: Vec<(usize, Vec<Guid>)> = Vec::new();
    for index in 0..3 {
        // A persistent area ticks on its own; it lands on no one at the cast.
        if matches!(s.effects[index].effect, 0 | EFFECT_PERSISTENT_AREA_AURA) {
            continue;
        }
        // Effects aimed alike share one choice of their targets, so a
        // limit picks the same few for each (mangos's `FillTargetMap`
        // reusing an earlier effect's map, Thistle Tea's one capped
        // selection).
        let e = &s.effects[index];
        let alike = chosen.iter().find(|(j, _)| {
            let o = &s.effects[*j];
            e.target_a != 0
                && (o.target_a, o.target_b, o.radius_index, o.chain_target)
                    == (e.target_a, e.target_b, e.radius_index, e.chain_target)
                && targets::script_masks_alike(&map.realm.data, s.id, *j, index)
        });
        let these = match alike {
            Some((_, v)) => v.clone(),
            None => targets::effect_targets(map, caster, s, index, t, m),
        };
        chosen.push((index, these.clone()));
        for g in these {
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

/// The most spells going off inside one another on a thread: one lands and
/// triggers the next ([`EFFECT_TRIGGER_SPELL`], a missile, a class script,
/// a death's cast). 1.12's deepest chain is 20 (Onyxia's Breath); a spell
/// that triggers itself, or a ring of them, would overflow the stack and
/// abort the server. As deep as a script's starts go (`scripts::MAX_DEPTH`).
const MAX_DEPTH: u8 = 32;

thread_local! {
    /// How many spells are going off inside one another now, on this thread.
    static DEPTH: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

/// A spell deeper, while it lives: the depth before is put back as it goes,
/// a panic's unwinding too (as procs' `Deeper`).
struct Deeper(u8);

impl Deeper {
    /// A spell deeper, unless spells are already as deep as they go.
    fn enter() -> Option<Self> {
        let depth = DEPTH.get();
        if depth >= MAX_DEPTH {
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
    // A cast ending: the charged cast-time modifiers it began with.
    let began_with = map.spells.ending_began_with.take();
    let instant = began_with.is_none();
    // An aura's cast, or a proc's; what it casts in turn is its own.
    let by_aura = BY_AURA.replace(false) || crate::game::map::procs::in_proc();
    // What a triggered cast is checked for still (mangos-classic's
    // `CheckCast` and `CheckScriptTargeting` for a triggered spell too):
    // told its caster unless an aura set it off.
    if triggered && let Some(code) = checks::triggered(map, caster, s) {
        if !by_aura {
            send_cast_result(map, caster, s.id, Some(code));
        }
        return;
    }
    let Some(_deeper) = Deeper::enter() else {
        tracing::debug!("spell {} not cast: {MAX_DEPTH} spells deep", s.id);
        return;
    };
    let now = map.now;
    let mut m = mods::for_spell(map, caster, s);
    // A charged cast-time modifier that came on during the cast changed
    // nothing of it, and is kept for the next (mangos's `m_appliedMods`,
    // taken at `prepare`): a Shadow Trance from a Corruption tick mid-cast
    // isn't spent by the Shadow Bolt under way.
    if let Some(began) = began_with {
        m.charged
            .retain(|c| c.2 != mods::OP_CASTING_TIME || began.contains(&(c.0, c.1)));
    }
    // The item cast from, by its entry, before it is spent.
    let cast_entry = item.and_then(|g| {
        let p = map.players.get(&caster)?;
        p.item_by_guid(g).map(|i| p.items[i].entry)
    });
    // What the cast cost, for a refund if an ability is avoided.
    let mut paid = 0;
    if !triggered {
        // An item's spell costs no power (mangos's `TakePower`, Thistle
        // Tea's item casts): no mana spent, so no five-second rule.
        let cost = if item.is_some() {
            0
        } else {
            spell_cost(map, caster, s, &m)
        };
        paid = cost;
        // An instant that fails here never went off: its global cooldown
        // is given back, as mangos's checks refuse it before one starts
        // (Arcane Shot with no ammunition).
        if let Some(r) = checks::at_launch(map, caster, s, cost) {
            if instant {
                give_back_gcd(map, caster, s.id);
            }
            return launch_failed(map, caster, s.id, r);
        }
        // A ranged ability (Arcane Shot, Multi-Shot, Aimed Shot) spends a
        // round of ammunition; a player's doesn't go off without it.
        if s.attr(0, sd::ATTR_RANGED) && s.dmg_class == 3 && !s.auto_repeat() {
            let ammunition = crate::game::map::ranged::pay_ammunition(map, caster);
            if let Err(r) = ammunition
                && map.players.contains_key(&caster)
            {
                if instant {
                    give_back_gcd(map, caster, s.id);
                }
                return launch_failed(map, caster, s.id, r);
            }
        }
        if cost > 0 {
            pay(map, caster, s, cost);
        }
        take_reagents(map, caster, s);
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
    let aimed = targets.unit;
    let targets = magnet(map, caster, s, targets);
    let targets = database_place(map, s, targets);
    let targets = script_dest(map, caster, s, targets);
    let resolved = resolve(map, caster, s, &targets, &m);
    // A spell a magnet took stops there: a chain jumps on to no one from
    // it (mangos's `CheckAndAddMagnetTarget`, Thistle Tea's resolver), and
    // what the spell does to its caster stays with the caster.
    let resolved = if targets.unit != aimed {
        resolved
            .into_iter()
            .filter(|(g, _)| Some(*g) == targets.unit || *g == caster)
            .collect()
    } else {
        resolved
    };
    let resolved = if !triggered && item.is_none() && map.players.contains_key(&caster) {
        aura_rank::recipients(map, caster, s, &targets, resolved)
    } else {
        resolved
    };
    // Each unit's hit is rolled as the spell goes, and kept: its misses
    // (a reflection among them) are the go's, and what lands is what was
    // rolled, whatever the target's chances to be hit become in flight
    // (mangos's `Spell::AddUnitTarget`, `missCondition = SpellHitResult`,
    // written by `WriteSpellGoTargets` and applied at
    // `DoAllEffectOnTarget`; Thistle Tea's saved launch result), an
    // instant spell's as a travelling one's.
    let travels = travels(map) && s.speed > 0.0;
    let flights: Vec<(u32, Option<Hit>)> = resolved
        .iter()
        .map(|(target, _)| {
            let delay = if travels && *target != caster {
                flight_ms(map, caster, *target, s.speed)
            } else {
                0
            };
            (delay, Some(launch_roll(map, caster, *target, s, &m)))
        })
        .collect();
    let missed = |hit: &Option<Hit>| hit.is_some_and(|h| !h.lands());
    let hits: Vec<Guid> = resolved
        .iter()
        .zip(&flights)
        .filter(|(_, (_, hit))| !missed(hit))
        .map(|((g, _), _)| *g)
        .collect();
    let misses: Vec<SpellMiss> = resolved
        .iter()
        .zip(&flights)
        .filter(|(_, (_, hit))| missed(hit))
        .filter_map(|((g, _), (_, hit))| {
            Some(SpellMiss {
                target: *g,
                miss_info: miss_info((*hit)?),
            })
        })
        .collect();
    // The cast's result before its go, as mangos's `Spell::cast` sends them
    // (and Thistle Tea queues them).
    if !triggered {
        send_cast_result(map, caster, s.id, None);
    }
    let mut flags = SMSG_SPELL_GO_CastFlags::empty();
    if let Some((display, inventory_type)) = ranged_ammo(map, caster, s) {
        flags = flags.set_ammo(SMSG_SPELL_GO_CastFlags_Ammo {
            ammo_display_id: display,
            ammo_inventory_type: inventory_type,
        });
    }
    // Not cast from an item, the go names its caster in the item's place
    // (mangos's `SendSpellGo`): a player's client, seeing its own cast
    // there, starts the spell's cooldown and its category's from its own
    // Spell.dbc; any other GUID there it takes for an item, and with no
    // such item it starts none.
    if let Some(frame) = crate::game::conn::encode(&SMSG_SPELL_GO {
        cast_item: item.unwrap_or(caster),
        caster,
        spell: s.id,
        flags,
        hits,
        misses,
        targets: wire_targets(&targets),
    }) {
        map.send_to_observers(caster, &frame, true);
    }
    if !triggered {
        // A heal cast: an Elite Guard's Charge looks for healers.
        if heals(s)
            && let Some(p) = map.players.get_mut(&caster)
        {
            p.misc.healed_at = Some(now);
        }
    }
    if s.channeled() {
        let level = map.unit_view(caster).map(|u| u.level).unwrap_or(1);
        // A channel of no end (a duration of -1: Blackfathom Channeling, a
        // ribbon pole's) runs until something stops it, its start telling
        // the client -1, as mangos-classic's channel with no timer.
        let spells = &map.realm.data.spells;
        let ms = match spells.duration(s, level) {
            Some(d) => m.apply_ms(mods::OP_DURATION, d),
            None if spells
                .durations
                .get(&s.duration_index)
                .is_some_and(|d| d.base < 0) =>
            {
                u32::MAX
            }
            None => 0,
        };
        let at = stance(map, caster);
        if let Some(c) = map.caster_mut(caster) {
            c.casting = Some(Casting {
                spell: s.id,
                targets,
                ends: now + Duration::from_millis(u64::from(ms)),
                item,
                pushbacks: 0,
                channel_until: Some(now + Duration::from_millis(u64::from(ms))),
                duration: ms,
                at,
            });
        }
        let object = channel_object(caster, s, &targets, &resolved);
        if let Some(f) = map.unit_fields_mut(caster) {
            f.set_u32(UNIT_CHANNEL_SPELL, s.id);
            f.set_guid(UNIT_FIELD_CHANNEL_OBJECT, object);
        }
        if let Some(p) = map.players.get(&caster) {
            p.outbound.send(&MSG_CHANNEL_START_Server {
                spell: s.id,
                duration: ms,
            });
        }
        if s.mana_per_second > 0 || s.mana_per_second_per_level > 0 {
            map.spells
                .channel_costs
                .insert(caster, now + Duration::from_secs(1));
        }
    }
    // A pet's feeding and abilities, and the professions' effects (which
    // run on their own and are skipped below).
    crate::game::map::pets::before_effects(map, caster, s, &targets);
    let professions = crate::game::map::professions::execute(map, caster, s, &targets, cast_entry);
    // The objects the cast is at, for the quests asking for it there: a
    // lock's too, credited once it is opened, after the effects (mangos's
    // `DoAllEffectOnTarget(GOTargetInfo*)` and `RewardPlayerAndGroupAtCast`
    // for every cast not a channel's; Thistle Tea's deferred credit).
    let at_objects = if triggered {
        Vec::new()
    } else {
        crate::game::map::objects::cast_at(map, caster, s, &targets)
    };
    // Each target: the hit, then the effects.
    let mut chain = [1.0f32; 3];
    let mut landed_on_main = false;
    let mut main_missed = false;
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
        by_aura,
        paid,
        shared,
    };
    let one_of = class_scripts::one_of(map, s);
    // Combo points a finisher's landing gives (Ruthlessness's) wait until
    // it has taken its own (mangos-classic's `AddTriggeredSpell`).
    if combo > 0 {
        map.spells.deferred_combo.entry(caster).or_default();
    }
    for ((target, effects), (delay, hit)) in resolved.into_iter().zip(flights) {
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
        if delay > 0 {
            if Some(target) == launch.targets.unit {
                landed_on_main = true;
                launch_hold(map, caster, s, target, delay);
            }
            map.spells.in_flight.push(InFlight {
                lands: now + Duration::from_millis(u64::from(delay)),
                launched: now,
                caster,
                spell: Box::new(s.clone()),
                target,
                effects,
                chain,
                launch: launch.clone(),
                hit,
            });
            continue;
        }
        let landed = land(map, caster, s, target, effects, &mut chain, &launch, hit);
        if landed && Some(target) == launch.targets.unit {
            landed_on_main = true;
        }
        main_missed |= !landed && Some(target) == launch.targets.unit && target != caster;
    }
    // A channel whose unit target resisted it, reflected it or was immune
    // to all of it stops at once: mangos-classic's `m_duration = 0` for a
    // channel's missed unit target (`WriteSpellGoTargets`), Thistle Tea's
    // "stops a target-dependent channel when its only target resists". One
    // whose selection it didn't aim at (a self aura's) goes on.
    if s.channeled()
        && main_missed
        && let Some(casting) = map
            .caster_mut(caster)
            .and_then(|c| c.casting.take_if(|c| c.spell == s.id))
    {
        end_channel(map, caster, &casting);
    }
    let targets = launch.targets;
    // Effects with no unit: places, objects.
    effects::run_untargeted(
        map,
        caster,
        s,
        &targets,
        &m,
        combo,
        professions,
        triggered,
        item,
    );
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
    if combo > 0
        && let Some(owed) = map.spells.deferred_combo.remove(&caster)
    {
        for (target, n) in owed {
            add_combo_points(map, caster, target, n);
        }
    }
    if !triggered {
        mods::spend(map, caster, &m);
        // Casting breaks what an action breaks, late or at once (an
        // instant's start and its launch are one), but what the spell
        // allows: stealth for a lucky Improved Sap too (mangos-classic's
        // `AURA_INTERRUPT_FLAG_ACTION_LATE` at `Spell::cast`).
        let stealthy = class_scripts::keeps_stealth(map, caster, s);
        action_breaks(
            map,
            caster,
            s,
            auras::INTERRUPT_ACTION | auras::INTERRUPT_ACTION_COMPLETE,
            stealthy,
        );
        // A failure of one that says so breaks stealth whatever it allows
        // (Pick Pocket's; mangos-classic's `FAILURE_BREAKS_STEALTH` in
        // `CasterHitTargetWithSpell`).
        if main_missed && s.attr(1, sd::ATTR_EX_FAILURE_BREAKS_STEALTH) {
            auras::remove_where(map, caster, |h| h.has(auras::AURA_MOD_STEALTH));
        }
    }
    cast_end_procs(map, caster, s, &targets, item);
    crate::game::map::combat::spell_launched(map, caster, s, triggered);
}

/// What `caster`'s action of casting `s` breaks: its auras whose interrupt
/// flags meet `flags`, but its own of `s`, stealth if `stealthy`, and an
/// invisibility if `s` allows that; nothing for a spell that is no action
/// (mangos-classic's `RemoveAurasOnCast`, `SPELL_ATTR_EX2_NOT_AN_ACTION`;
/// Thistle Tea's "an invisibility-compatible cast preserves early and late
/// invisibility").
fn action_breaks(map: &mut Map, caster: Guid, s: &SpellTemplate, flags: u32, stealthy: bool) {
    if s.attr(2, sd::ATTR_EX2_NOT_AN_ACTION) {
        return;
    }
    let invisible = s.attr(2, sd::ATTR_EX2_ALLOW_WHILE_INVISIBLE);
    let data = map.realm.data.clone();
    let breaks = |h: &auras::Holder| {
        h.interrupt_flags & flags != 0
            && !(h.spell == s.id && h.caster == caster)
            && !(stealthy && h.has(auras::AURA_MOD_STEALTH))
            && !(invisible
                && (h.has(auras::AURA_MOD_INVISIBILITY)
                    || data
                        .spells
                        .get(h.spell)
                        .is_some_and(|t| t.dispel == DISPEL_INVISIBILITY)))
    };
    if auras::auras(map, caster).is_some_and(|a| a.holders.iter().any(breaks)) {
        auras::remove_where(map, caster, breaks);
    }
}

/// `DISPEL_INVISIBILITY`, a spell's dispel type.
const DISPEL_INVISIBILITY: u32 = 6;

/// The cast's end, once, after whatever it landed on (vmangos's
/// `PROC_EX_CAST_END`, Thistle Tea's `cast_complete_procs`): what procs on
/// a spell cast rather than on its hits (Unstable Power and Ascendance
/// spend a stack a cast, Elemental Focus and the Blue Dragon may come of
/// one), whether it hit, was resisted or landed on many. At its unit
/// target, else its caster. A triggered cast's too, as mangos's
/// `Spell::cast` raises it for every cast with no spell to ask whether it
/// was triggered: each of Arcane Missiles' missiles spends a stack of
/// Unstable Power (Thistle Tea raises none for a triggered cast). Not a
/// helpful item's.
fn cast_end_procs(map: &mut Map, caster: Guid, s: &SpellTemplate, t: &Targets, item: Option<Guid>) {
    if item.is_some() && auras::positive(s) {
        return;
    }
    let (done, _) = outcome::proc_flags(s);
    crate::game::map::procs::trigger(
        map,
        crate::game::map::procs::Event {
            actor: caster,
            target: t.unit.unwrap_or(caster),
            actor_flags: done,
            ex: crate::game::map::procs::EX_CAST_END,
            spell: Some(s.id),
            school: s.school_mask(),
            ranged: s.dmg_class == 3,
            ..Default::default()
        },
    );
}

/// A mechanic immunity with charges kept `s` off `target`: a charge of it
/// goes, as Thistle Tea's `EffectImmunity` spends Fear Ward's.
fn spend_immunity(map: &mut Map, target: Guid, s: &SpellTemplate) {
    let mechanics: Vec<u32> = std::iter::once(s.mechanic)
        .chain(
            s.effects
                .iter()
                .filter(|e| e.effect != 0)
                .map(|e| e.mechanic),
        )
        .filter(|m| *m != 0)
        .collect();
    let charged = auras::auras(map, target).and_then(|a| {
        a.holders
            .iter()
            .find(|h| {
                h.charges > 0
                    && h.effects.iter().any(|x| {
                        mechanics.iter().any(|m| match x.kind {
                            auras::AURA_MECHANIC_IMMUNITY => x.misc as u32 == *m,
                            auras::AURA_MECHANIC_IMMUNITY_MASK => {
                                (x.misc as u32) & (1 << (m - 1).min(31)) != 0
                            }
                            _ => false,
                        })
                    })
            })
            .map(|h| (h.spell, h.caster))
    });
    if let Some((spell, caster)) = charged {
        auras::spend_charge(map, target, spell, caster);
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
    /// An aura's or a proc's cast (see [`cast_by_aura`]).
    by_aura: bool,
    paid: u32,
    shared: u32,
}

/// A spell on its way to a target.
#[derive(Debug, Clone)]
pub struct InFlight {
    pub lands: Instant,
    /// When it was cast: a target come back as new since isn't hit.
    pub launched: Instant,
    pub caster: Guid,
    pub spell: Box<SpellTemplate>,
    pub target: Guid,
    effects: [bool; 3],
    chain: [f32; 3],
    launch: Launch,
    /// Its hit, rolled as it went.
    hit: Option<Hit>,
}

/// A spell's hit on `target` as it goes: its roll, or the target's
/// template immunity to it.
fn launch_roll(map: &Map, caster: Guid, target: Guid, s: &SpellTemplate, m: &Mods) -> Hit {
    let hit = outcome::roll_hit(map, caster, target, s, m);
    if hit.lands() && crate::game::map::creature_combat::spell_immune(map, target, caster, s) {
        Hit::Immune
    } else {
        hit
    }
}

/// How a go names a miss.
fn miss_info(hit: Hit) -> SpellMiss_SpellMissInfo {
    match hit {
        Hit::Hit => SpellMiss_SpellMissInfo::None,
        Hit::Miss => SpellMiss_SpellMissInfo::Miss,
        Hit::Resist => SpellMiss_SpellMissInfo::Resist,
        Hit::Dodge => SpellMiss_SpellMissInfo::Dodge,
        Hit::Parry => SpellMiss_SpellMissInfo::Parry,
        Hit::Block | Hit::Blocked => SpellMiss_SpellMissInfo::Block,
        Hit::Evade => SpellMiss_SpellMissInfo::Evade,
        Hit::Immune => SpellMiss_SpellMissInfo::Immune,
        Hit::Deflect => SpellMiss_SpellMissInfo::Deflect,
        Hit::Absorb => SpellMiss_SpellMissInfo::Absorb,
        Hit::Reflect => SpellMiss_SpellMissInfo::Reflect {
            reflect_result: SpellMissInfo::None,
        },
    }
}

/// Whether spells take time to get where they go (tests land them at
/// once).
pub(crate) fn travels(map: &Map) -> bool {
    map.spells.travel.unwrap_or(map.cfg.spell_travel)
}

/// A cast that can't go off as it ends (its power, ammunition, reagents
/// or room gone): why, and its bar stops for everyone (mangos's
/// `SendCastResult` and `SendInterrupted`).
fn launch_failed(map: &Map, caster: Guid, spell: u32, code: u8) {
    send_cast_result(map, caster, spell, Some(code));
    if let Some(p) = map.players.get(&caster) {
        p.outbound.send(&SMSG_SPELL_FAILURE {
            guid: caster,
            spell,
            result: SpellCastResult::try_from(code).unwrap_or(SpellCastResult::Interrupted),
        });
    }
    if let Some(frame) = crate::game::conn::encode(&SMSG_SPELL_FAILED_OTHER { caster, id: spell }) {
        map.send_to_observers(caster, &frame, false);
    }
}

/// How long something at `speed` yards a second takes from `caster` to
/// `target`: at least five yards' worth.
pub(crate) fn flight_ms(map: &Map, caster: Guid, target: Guid, speed: f32) -> u32 {
    match (map.position_of(caster), map.position_of(target)) {
        (Some(a), Some(b)) => {
            let d = ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt();
            (d.max(5.0) / speed * 1000.0) as u32
        }
        _ => 0,
    }
}

/// Whether `g` is a creature come into the world since `t`: a respawn
/// keeps a spawn's GUID, and the new life is no target of what was cast
/// at the old (mangos-classic's new `Creature` object at a respawn).
pub(crate) fn new_since(map: &Map, g: Guid, t: Instant) -> bool {
    guid::is_unit(g)
        && map
            .creature(g)
            .and_then(|c| c.spawned_at)
            .is_some_and(|s| s > t)
}

/// `g` left the world, or is to come back into it as new (a creature
/// respawned, despawned or gone for good): what it cast that is on its
/// way, and what is on its way to it, is lost, and its ground areas go,
/// as mangos-classic's `Unit::RemoveFromWorld` ends a unit's spell events
/// (`m_Events.KillAllEvents`) and takes its dynamic objects
/// (`RemoveAllDynObjects`).
pub fn unit_gone(map: &mut Map, g: Guid) {
    lose_pending(map, &[g]);
    remove_dynamic_of(map, g);
}

/// What `units` cast that is on its way, and what is on its way to them,
/// lost: missiles and bolts, auto-repeat shots, combo points owed a
/// landing finisher, and a channel's next payment.
pub fn lose_pending(map: &mut Map, units: &[Guid]) {
    let involved = |a: Guid, b: Guid| units.contains(&a) || units.contains(&b);
    map.spells
        .in_flight
        .retain(|f| !involved(f.caster, f.target));
    map.spells.shots.retain(|s| !involved(s.shooter, s.target));
    for u in units {
        map.spells.deferred_combo.remove(u);
        map.spells.channel_costs.remove(u);
    }
}

/// Take away `caster`'s ground areas (Blizzard, Rain of Fire,
/// Consecration) and their dynamic objects, and its other dynamic objects
/// (far sight's): mangos-classic's `Unit::RemoveAllDynObjects`.
pub fn remove_dynamic_of(map: &mut Map, caster: Guid) {
    map.spells.areas.retain(|a| a.caster != caster);
    map.spells
        .area_ticked
        .retain(|(c, _, _, u), _| *c != caster && *u != caster);
    let gone: Vec<Guid> = map
        .spells
        .dynamic
        .values()
        .filter(|d| d.caster == caster)
        .map(|d| d.guid)
        .collect();
    for g in gone {
        dynamic::remove(map, g);
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
        // A target that died while it flew is missed, unless the spell is
        // for the dead: Thistle Tea keeps only what works on a corpse. One
        // whose caster left the map (logged out, teleported) is lost, as
        // mangos ends a unit's spell events as it leaves the world (a
        // caster's death doesn't stop it), and as a shot is. A creature
        // come back as new since (respawned under its GUID) is another
        // life: neither hit nor hitting.
        if map
            .unit_view(f.target)
            .is_none_or(|u| !u.alive && !checks::for_the_dead(&f.spell))
            || map.unit_view(f.caster).is_none()
            || new_since(map, f.target, f.launched)
            || new_since(map, f.caster, f.launched)
        {
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
            f.hit,
        );
    }
}

/// `s` gets to `target`: the hit (`rolled` as it went, and told in the go,
/// else rolled now), then the effects. Whether it landed. A miss the go
/// told isn't told again; one that comes about as it lands (an immunity
/// gained in flight) is, in `SMSG_SPELLLOGMISS` (mangos's
/// `DoSpellHitOnUnit`, `SendSpellMiss`).
#[allow(clippy::too_many_arguments)]
fn land(
    map: &mut Map,
    caster: Guid,
    s: &SpellTemplate,
    target: Guid,
    effects: [bool; 3],
    chain: &mut [f32; 3],
    l: &Launch,
    rolled: Option<Hit>,
) -> bool {
    let told = rolled;
    let mut hit = rolled.unwrap_or_else(|| outcome::roll_hit(map, caster, target, s, &l.m));
    // A creature immune to the spell (its template's masks), as it lands.
    if hit.lands() && crate::game::map::creature_combat::spell_immune(map, target, caster, s) {
        hit = Hit::Immune;
    }
    // A charged immunity that keeps it off is spent (Fear Ward).
    if hit == Hit::Immune {
        spend_immunity(map, target, s);
    }
    // A melee or ranged ability trains its weapon as a swing does.
    if outcome::physical(s)
        && matches!(
            hit,
            Hit::Hit | Hit::Block | Hit::Blocked | Hit::Miss | Hit::Dodge | Hit::Parry
        )
    {
        crate::game::map::combat::ability_skill_gains(map, caster, target, s);
    }
    let mut aim = target;
    if hit == Hit::Reflect {
        // Back at the caster; the reflector's procs see it reflected, and
        // a reflection of charges spends one (Sheen of Zanza's first, as
        // mangos's victim proc of `PROC_EX_REFLECT`).
        if told != Some(Hit::Reflect) {
            outcome::send_miss(map, caster, target, s.id, Hit::Reflect);
        }
        let (_, taken) = outcome::proc_flags(s);
        crate::game::map::procs::trigger(
            map,
            crate::game::map::procs::Event {
                actor: caster,
                target,
                actor_flags: 0,
                target_flags: taken,
                ex: crate::game::map::procs::EX_REFLECT,
                spell: Some(s.id),
                damage: 0,
                school: s.school_mask(),
                offhand: false,
                ranged: false,
                extra_attack: false,
                origin: crate::game::map::procs::Origin::of(s, l.by_aura, l.item.is_some()),
            },
        );
        aim = caster;
        hit = Hit::Hit;
    }
    if !hit.lands() {
        if told != Some(hit) {
            outcome::send_miss(map, caster, aim, s.id, hit);
        }
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
                origin: crate::game::map::procs::Origin::of(s, l.by_aura, l.item.is_some()),
            },
        );
        contact(map, caster, aim, s, l, false);
        return false;
    }
    // Each harmful effect of a mechanic other than the spell's: immune, or
    // resisted on its own roll (Thistle Tea's `EffectImmunity` and
    // `MechanicResistance`).
    let mut effects = effects;
    // An effect of a kind the target is immune to is kept off it, helpful
    // or harmful (mangos's `AddUnitTarget` asking `IsImmuneToSpellEffect`
    // of every one): a flag carrier isn't summoned, a mechanical one held
    // off its repair isn't repaired. Kept off whole, it says so.
    if !s.attr(0, sd::ATTR_NO_IMMUNITIES) {
        let before = effects.iter().filter(|e| **e).count();
        for (i, on) in effects.iter_mut().enumerate() {
            if *on && crate::game::map::extra_effects::effect_immune(map, aim, s, i) {
                *on = false;
            }
        }
        if before > 0 && !effects.iter().any(|e| *e) {
            outcome::send_miss(map, caster, aim, s.id, Hit::Immune);
            contact(map, caster, aim, s, l, false);
            return false;
        }
    }
    if aim != caster && !auras::positive(s) {
        let before = effects.iter().filter(|e| **e).count();
        let mut immune = false;
        for (i, on) in effects.iter_mut().enumerate() {
            if !*on {
                continue;
            }
            let mechanic = s.effects[i].mechanic;
            if mechanic != 0
                && ((!s.attr(0, sd::ATTR_NO_IMMUNITIES)
                    && auras::auras(map, aim).is_some_and(|a| a.mechanic_immune(mechanic)))
                    || crate::game::map::creature_combat::effect_immune(map, aim, caster, mechanic))
            {
                *on = false;
                immune = true;
            } else if !s.attr(0, sd::ATTR_NO_IMMUNITIES)
                && (auras::auras(map, aim).is_some_and(|a| outcome::state_immune(a, &s.effects[i]))
                    || crate::game::map::extra_effects::effect_immune(map, aim, s, i))
            {
                // An aura of a kind kept off (mangos's `IsImmuneToSpellEffect`).
                *on = false;
                immune = true;
            } else if outcome::effect_resisted(map, aim, s, i) {
                *on = false;
            }
        }
        if immune {
            spend_immunity(map, aim, s);
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
        .any(|e| matches!(e.effect, 2 | 7 | 9 | 10 | 17 | 31 | 58 | 62 | 67 | 75 | 121));
    // Damage that ignores its caster's modifiers ignores their crit too
    // (mangos's `CalculateCritAmount`); a heal's may still crit.
    let unmodified = s.attr(3, sd::ATTR_EX3_IGNORE_CASTER_MODIFIERS)
        && !s.effects.iter().any(|e| matches!(e.effect, 10 | 67 | 75));
    let crit = deals
        && !unmodified
        && if outcome::physical(s) {
            // A hit's crit on a roll of its own, past the hit (the second
            // of a special attack's two, `combat::roll_special`): a second
            // roll of the whole table took its miss and dodge off the crit
            // chance. A block (in part) doesn't crit.
            hit == Hit::Hit && outcome::melee_crit(map, caster, aim, s, &l.m)
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
        by_aura: l.by_aura,
        item: l.item.is_some(),
    };
    // A quest's spell cast at what it hit (Thistle Tea's
    // `queue_quest_cast_credit`).
    if !l.triggered && !crate::game::map::objects::opens_lock(s) {
        crate::game::map::quests::cast_credit(map, caster, &[aim], s.id);
    }
    contact(map, caster, aim, s, l, true);
    let alive_before = map.unit_view(aim).is_some_and(|u| u.alive);
    effects::run(
        map, caster, aim, s, effects, &landing, chain, l.item, &l.targets,
    );
    if alive_before {
        crate::game::map::procs::target_triggers(map, caster, aim, s.id, l.combo);
    }
    class_scripts::landed(map, caster, aim, s);
    crate::game::map::event_ai::spell_hit(map, aim, caster, s.id, s.school_mask());
    // A script for the spell, and the events it sends.
    for (i, e) in s.effects.iter().enumerate() {
        if !effects[i] {
            continue;
        }
        // A pet aura's dummy is the pet's to answer, not a script's
        // (Soul Link's old link, Thistle Tea's preference).
        if matches!(e.effect, EFFECT_DUMMY | EFFECT_SCRIPT_EFFECT)
            && map.realm.data.scripts.spell.contains_key(&s.id)
            && !map
                .realm
                .data
                .pets
                .owner_auras
                .iter()
                .any(|(spell, ..)| *spell == s.id)
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
            // An event with no target of its own lands once, on its caster,
            // and is aimed at the object, else the unit picked, else at
            // nothing (vmangos's `EffectSendEvent`, Thistle Tea's
            // `SendEvent.target`).
            let at = if e.target_a == 0 && e.target_b == 0 {
                [l.targets.object, l.targets.unit]
                    .into_iter()
                    .flatten()
                    .find(|g| !g.is_zero() && *g != caster)
                    .unwrap_or(Guid::zero())
            } else {
                aim
            };
            crate::game::map::scripts::start(
                map,
                crate::game::data::scripts::ScriptTable::Event,
                e.misc_value as u32,
                caster,
                at,
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
/// Stealth's dispel type.
const DISPEL_STEALTH: u32 = 5;
const ATTR_EX2_NO_INITIAL_THREAT: u32 = 0x40_0000;

/// A harmful spell got to creature `target`, landing (`hit`) or not:
/// whether that puts it in the fight with its caster (Thistle Tea's
/// `Spell.Combat.decide`). Landing, it does if the creature saw the caster
/// (or it is Seduction), the cast was the caster's own (not an aura's or a
/// proc's, unless it travelled; another spell's trigger is: Thistle Tea's
/// direct triggered cast, as mangos's `CasterHitTargetWithSpell` for every
/// one, so a trap's spell pulls) or makes threat outright, and the spell
/// starts fights: not Sap, nor one with no threat, threat only on a miss,
/// or none as it lands. Missing, it does if the creature saw the caster or
/// the miss breaks stealth, and the cast was the caster's own with threat.
/// Damage puts it in the fight anyway (`outcome::deal`).
///
/// The players' sides of a fight it starts are in it too, a player struck
/// as well as one striking (`combat::struck`); and a helpful spell landing
/// on one in a fight puts its caster in it (`combat::assisted`), unless it
/// makes no threat (mangos's `CasterHitTargetWithSpell`).
///
/// A harmful one that starts a fight between two players' sides, or that
/// is PvP-enabling (Hunter's Mark) and seen, flags them as a blow does,
/// a pet's spell its master (`combat::hostile_spell_pvp`).
fn contact(map: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate, l: &Launch, hit: bool) {
    if target == caster
        || !(guid::is_unit(target) || guid::is_player(target))
        || !(guid::is_player(caster) || guid::is_unit(caster))
    {
        return;
    }
    // Any spell reaching another player's side flagged for PvP flags its
    // caster, hit or not (mangos's `Spell::FillTargetMap`).
    combat::pvp_contact(map, caster, target, combat::Contact::Touch);
    if auras::positive(s) {
        if hit {
            combat::assist_contact(map, caster, target, s);
            if !s.attr(1, ATTR_EX_NO_THREAT) && !s.attr(2, ATTR_EX2_NO_INITIAL_THREAT) {
                combat::assisted(map, caster, target);
            }
        }
        return;
    }
    // Only at one it may attack (mangos's `CasterHitTargetWithSpell` asks
    // `CanAttack` first), and not the world's harm (a campfire's).
    if !combat::can_attack(map, caster, target) || s.has_effect(EFFECT_ENVIRONMENTAL_DAMAGE) {
        return;
    }
    let seen = stealth::sees(map, target, caster);
    let own = !l.by_aura || s.speed > 0.0;
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
    let enabling = s.attr(3, sd::ATTR_EX3_PVP_ENABLING)
        && (seen || (!hit && s.attr(1, ATTR_EX_FAILURE_BREAKS_STEALTH)));
    if fights || enabling {
        combat::hostile_spell_pvp(map, caster, target);
    }
    // Caught at it (a resisted Pick Pocket), the caster's stealth ends
    // (mangos's `CasterHitTargetWithSpell`: `RemoveAurasWithDispelType
    // (DISPEL_STEALTH)` for `SPELL_ATTR_EX_FAILURE_BREAKS_STEALTH`).
    if fights && !hit && s.attr(1, ATTR_EX_FAILURE_BREAKS_STEALTH) {
        auras::remove_where(map, caster, |h| h.dispel == DISPEL_STEALTH);
    }
    if fights {
        if guid::is_unit(target) {
            crate::game::map::ai::attacked(map, target, caster);
            crate::game::map::creature_combat::hostile_act(map, target);
        }
        combat::struck(map, caster, target);
    }
}

/// A player's harmful spell flying at the unit it was aimed at, one it
/// may attack and is seen by, holds the player (or a pet's master) in a
/// fight for its flight and half a second, before it lands (mangos's
/// `Spell::cast` for a delayed spell: `SetInCombatWithVictim` and
/// `TriggerCombatTimer(timeDelay + 500)`; Thistle Tea's `LaunchCombat`).
/// Not one of no threat, threat only on a miss, or none as it lands, nor
/// a creature's.
pub(crate) fn launch_hold(
    map: &mut Map,
    caster: Guid,
    s: &SpellTemplate,
    target: Guid,
    delay_ms: u32,
) {
    let master = crate::game::map::pets::master(map, caster);
    if target == caster
        || !guid::is_player(master)
        || auras::positive(s)
        || s.attr(1, ATTR_EX_NO_THREAT)
        || s.attr(1, ATTR_EX_THREAT_ONLY_ON_MISS)
        || s.attr(2, ATTR_EX2_NO_INITIAL_THREAT)
        || !combat::can_attack(map, caster, target)
        || !stealth::sees(map, target, caster)
    {
        return;
    }
    let until = map.now + Duration::from_millis(u64::from(delay_ms) + 500);
    for g in [caster, master] {
        combat::enter(map, g, false);
        if let Some(p) = map.players.get_mut(&g) {
            p.combat.combat_until = p.combat.combat_until.max(Some(until));
        }
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

/// A rogue's or druid's combo points go, and any hold on them. The field
/// still names the target they were on, its points at 0 (mangos's
/// `ClearComboPoints`, Thistle Tea's `put_points`).
pub fn clear_combo_points(map: &mut Map, caster: Guid) {
    release_retained_combo_points(map, caster);
    if let Some(c) = map.caster_mut(caster) {
        c.combo_points = 0;
        c.combo_target = Guid::zero();
    }
    // The field keeps the last target, its points none, as mangos's
    // ClearComboPoints sends them and Thistle Tea encodes them.
    if let Some(p) = map.players.get_mut(&caster) {
        p.fields.set_byte(PLAYER_FIELD_BYTES, 1, 0);
    }
}

/// Threat a spell's damage (or effect) adds beyond what the damage itself
/// made (its school's threat auras on it, `deal_damage`'s): its
/// `spell_threat`, the threat modifiers, the auras on those too, as Thistle
/// Tea's `SpellThreat.multiplier` has it.
pub fn threat(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    damage: f32,
    m: &Mods,
    crit: bool,
) {
    if !guid::is_unit(target) || s.attr(1, sd::ATTR_EX_NO_THREAT) {
        return;
    }
    // A critical hit's threat by its school's critical threat auras
    // (mangos's `CalcThreat`, `SPELL_AURA_MOD_CRITICAL_THREAT`).
    let critical = if crit {
        auras::auras(map, caster).map_or(1.0, |a| {
            a.product_mask(auras::AURA_MOD_CRITICAL_THREAT, s.school_mask())
        })
    } else {
        1.0
    };
    let (flat, mult) = map
        .realm
        .data
        .spells
        .threat
        .get(&s.id)
        .copied()
        .unwrap_or((0, 1.0));
    let auras = crate::game::map::combat::threat_multiplier(map, caster, s.school_mask());
    let pct = m.pct[mods::OP_THREAT];
    let total =
        (damage * mult.max(0.0) + flat as f32) * auras * critical * (100 + pct).max(0) as f32
            / 100.0;
    let extra = total - damage * auras;
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
    periodic_damage_of(map, h, target, amount as f32, leech);
}

/// [`periodic_damage`] of an amount with a fraction: the target's
/// modifiers on it, then rounded without bias (a half when the rolls are
/// fixed), as Thistle Tea's `PeriodicDamage` dithers it.
pub fn periodic_damage_of(
    map: &mut Map,
    h: &auras::Holder,
    target: Guid,
    amount: f32,
    leech: bool,
) {
    let data = map.realm.data.clone();
    // Borrowed from the data, not cloned each tick.
    let made_up;
    let s = match data.spells.get(h.spell) {
        Some(s) => s,
        None => {
            made_up = SpellTemplate {
                id: h.spell,
                school: h.school.trailing_zeros(),
                ..Default::default()
            };
            &made_up
        }
    };
    let index = s
        .effects
        .iter()
        .position(|e| {
            matches!(
                e.aura,
                auras::AURA_PERIODIC_DAMAGE
                    | auras::AURA_PERIODIC_LEECH
                    | auras::AURA_PERIODIC_DAMAGE_PERCENT
                    // A burn's tick takes its own effect's coefficient
                    // (Thistle Tea's `PowerBurn.damage_effect`).
                    | auras::AURA_POWER_BURN_MANA
            )
        })
        .unwrap_or(0);
    let amount = outcome::taken_of(
        map,
        h.caster,
        target,
        s,
        index,
        outcome::Kind::Dot,
        h.stacks,
        amount,
    );
    let roll = if outcome::rolling(map) {
        rand::rng().random_range(0.0f32..1.0)
    } else {
        0.5
    };
    let amount = crate::game::map::reputation::dither(amount, roll).max(0) as u32;
    let resisted = outcome::partial_resist(map, h.caster, target, s, amount, true);
    let (left, absorbed) =
        auras::absorb_and_share(map, h.caster, target, amount - resisted, h.school);
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
    let items = outcome::death_items(map, target, h.caster, left);
    // A leech takes no more than the health there was (mangos's
    // `PeriodicTick`: `min(pdamage, victim's health)`).
    let had = map
        .unit_fields(target)
        .map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH));
    if left > 0 {
        auras::interrupt(map, target, auras::INTERRUPT_DAMAGE);
        // Its threat is of its school, and has the spell's threat
        // modifiers (Shadow Affinity) and its `spell_threat` multiplier, as
        // Thistle Tea's periodic damage's and mangos's `DealDamage` for a
        // DOT (`GetSpellThreatMultiplier`, then `CalcThreat`).
        let multiplier = data.spells.threat.get(&s.id).map_or(1.0, |t| t.1.max(0.0));
        let by_mods =
            mods::for_spell(map, h.caster, s).apply(mods::OP_THREAT, 100.0) / 100.0 * multiplier;
        let threat = left as f32 * combat::threat_multiplier(map, h.caster, h.school);
        combat::deal_periodic_damage(
            map,
            h.caster,
            target,
            left,
            h.school,
            Some(h.spell),
            s.channeled(),
        );
        // A critter's escape runs on from each tick (Thistle Tea's DoT
        // refreshing it).
        if guid::is_unit(target) {
            crate::game::map::creature_motion::critter_hit(map, target, h.caster);
        }
        if (by_mods - 1.0).abs() > f32::EPSILON && map.creature(target).is_some_and(|c| c.alive()) {
            crate::game::map::ai::add_threat(map, target, h.caster, threat * (by_mods - 1.0));
        }
    }
    outcome::give_death_items(map, target, items);
    // The tick's procs come after its damage, as mangos's `PeriodicTick`
    // has them: a killing tick's victim is dead to them (Nightfall gives no
    // trance), and has none of its own.
    let lives = map.unit_view(target).is_some_and(|u| u.alive);
    crate::game::map::procs::trigger(
        map,
        crate::game::map::procs::Event {
            actor: h.caster,
            target,
            actor_flags: crate::game::map::procs::PROC_DONE_PERIODIC,
            target_flags: if lives {
                crate::game::map::procs::PROC_TAKEN_PERIODIC
                    | crate::game::map::procs::PROC_TAKEN_DAMAGE
            } else {
                0
            },
            ex: crate::game::map::procs::EX_NORMAL_HIT,
            spell: Some(h.spell),
            damage: left,
            school: h.school,
            offhand: false,
            ranged: false,
            extra_attack: false,
            origin: Default::default(),
        },
    );
    if leech {
        let mult = s
            .effects
            .iter()
            .find(|e| e.aura == auras::AURA_PERIODIC_LEECH)
            .map(|e| e.multiple_value)
            .filter(|v| *v > 0.0)
            .unwrap_or(1.0);
        // What it gives back is healing the caster takes, by their
        // modifiers (mangos's `SpellHealingBonusTaken` on the beneficiary).
        let gain = outcome::spell_healing_taken(
            map,
            h.caster,
            left.min(had) as f32 * mult,
            s,
            index,
            outcome::Kind::Dot,
            h.stacks,
            1.0,
        ) as u32;
        leech_heal(map, h.caster, h.spell, gain, true);
    }
}

/// Whether a spell heals: directly, to the full, or over time.
pub fn heals(s: &SpellTemplate) -> bool {
    s.effects.iter().any(|e| {
        matches!(e.effect, EFFECT_HEAL | EFFECT_HEAL_MAX_HEALTH)
            || (e.effect != 0 && e.aura == auras::AURA_PERIODIC_HEAL)
    })
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
    heal_with(map, caster, target, spell, amount, periodic, crit, true);
}

/// A leech's heal of its caster: no threat for it (mangos's `PeriodicTick`
/// and `EffectHealthLeech`, which leave `threatAssist` out).
pub fn leech_heal(map: &mut Map, caster: Guid, spell: u32, amount: u32, periodic: bool) {
    heal_with(map, caster, caster, spell, amount, periodic, false, false);
}

#[allow(clippy::too_many_arguments)]
fn heal_with(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    spell: u32,
    amount: u32,
    periodic: bool,
    crit: bool,
    threat: bool,
) {
    // Down in a linked feign death, nothing heals it (Thistle Tea's
    // fallen pose).
    if crate::game::map::event_ai::feigning(map, target) {
        return;
    }
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
    if threat {
        combat::heal_threat(map, caster, target, spell, gained);
    }
}

/// The category most spells' global cooldown is of.
pub const GCD_SPELLS: u32 = 133;

/// Whether `c` is within the global cooldown of `category`.
pub fn on_gcd(c: &Caster, category: u32, now: Instant) -> bool {
    if category == 0 {
        return false;
    }
    if category == GCD_SPELLS {
        return c.gcd_until.is_some_and(|t| t > now);
    }
    c.other_gcds
        .iter()
        .any(|(cat, t)| *cat == category && *t > now)
}

/// A unit's cast is interrupted. One still being cast gives back its
/// global cooldown and tells its caster it was interrupted (mangos's
/// `Spell::cancel`: `ResetGCD`, `SendCastResult`).
pub fn interrupt(map: &mut Map, g: Guid) {
    let Some(c) = map.caster_mut(g) else {
        return;
    };
    c.next_swing = None;
    let Some(casting) = c.casting.take() else {
        return;
    };
    if casting.channel_until.is_none() {
        give_back_gcd(map, g, casting.spell);
    }
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
    // The caster's client is told why too, and lets the button go, but
    // for a channel (mangos's `Spell::cancel`, Thistle Tea's interrupted
    // cast result).
    if casting.channel_until.is_some() {
        end_channel(map, g, &casting);
    } else {
        send_cast_result(map, g, casting.spell, Some(fail::INTERRUPTED));
    }
}

/// What a channel is at, for its caster's channel object: the unit it was
/// aimed at when the cast lands on it; else the first other unit it lands
/// on (mangos's `SendChannelStart`, "select first not resisted target from
/// target list for _0_ effect", a target not the caster): a Mend Pet or
/// Health Funnel cast with its caster or an enemy selected is at the pet,
/// a channel cast on oneself whose aura goes on a scripted creature at the
/// creature (Thistle Tea's "keeps pet channels active when the client
/// selects the caster" and "aims a channel cast on oneself at the scripted
/// creature its aura lands on"); else the unit it was aimed at, as Thistle
/// Tea's selected unit for a channel of its caster's own aura. A channel
/// whose first effect is a persistent area's is at the unit it was aimed
/// at, as it was.
fn channel_object(
    caster: Guid,
    s: &SpellTemplate,
    t: &Targets,
    resolved: &[(Guid, [bool; 3])],
) -> Guid {
    let aimed = t.unit.filter(|u| *u != caster);
    if s.effects[0].effect == EFFECT_PERSISTENT_AREA_AURA
        || aimed.is_some_and(|u| resolved.iter().any(|(g, _)| *g == u))
    {
        return t.unit.unwrap_or_default();
    }
    resolved
        .iter()
        .map(|(g, _)| *g)
        .find(|g| *g != caster)
        .or(t.unit)
        .unwrap_or_default()
}

/// A channel ends: its auras from the caster go (on the unit it was aimed
/// at, on its channel object, on the caster), its areas go, and the
/// channel fields clear.
fn end_channel(map: &mut Map, g: Guid, casting: &Casting) {
    let target = casting.targets.unit.unwrap_or(g);
    auras::remove_spell(map, target, casting.spell, Some(g));
    if target != g {
        auras::remove_spell(map, g, casting.spell, Some(g));
    }
    let object = map
        .unit_fields(g)
        .map(|f| f.get_guid(UNIT_FIELD_CHANNEL_OBJECT))
        .filter(|o| !o.is_zero() && *o != target && *o != g);
    if let Some(o) = object {
        auras::remove_spell(map, o, casting.spell, Some(g));
    }
    // A channel at the pet (Mend Pet, Health Funnel) ends on the pet,
    // whatever the client aimed it at.
    let at_pet = map.realm.data.spells.get(casting.spell).is_some_and(|s| {
        s.effects
            .iter()
            .any(|e| targets::implicit(e.target_a) == targets::Implicit::Pet)
    });
    if at_pet
        && let Some(pet) = crate::game::map::pets::pet_of(map, g)
        && pet != target
    {
        auras::remove_spell(map, pet, casting.spell, Some(g));
    }
    effects::areas_cancelled(map, g, casting.spell, target);
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

/// Damage taken while casting, from no one in particular: [`pushback_by`].
pub fn pushback(map: &mut Map, g: Guid) {
    pushback_by(map, g, Guid::zero());
}

/// Damage taken while casting, dealt by `from`, as Thistle Tea's
/// `CastPushback` and mangos's `InterruptOrDelaySpell` have it: a player's
/// cast whose interrupt flags say so is cancelled (a mount, the
/// Hearthstone, gathering) or else pushed back, and a channel cut short or
/// else cancelled (a bandage); talents and auras may resist the pushback.
/// A channel isn't cut short by its caster's own harm, the world's
/// included (falling, lava).
pub fn pushback_by(map: &mut Map, g: Guid, from: Guid) {
    let data = map.realm.data.clone();
    if !map.players.contains_key(&g) {
        // A creature's channel that damage breaks stops (Destroy Spear's,
        // Blackfathom Channeling's), as mangos-classic's `DealDamage` has
        // it for any unit; nothing of a creature's is pushed back.
        let breaks = map
            .caster(g)
            .and_then(|c| c.casting.as_ref())
            .filter(|c| c.channel_until.is_some())
            .and_then(|c| data.spells.get(c.spell))
            .is_some_and(|s| s.channel_interrupt_flags & 0x2 != 0);
        if breaks {
            interrupt(map, g);
        }
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
    if channel {
        // A channel cut short isn't cancelled too.
        let delays = s.channel_interrupt_flags & 0x4000 != 0;
        if delays && from == g {
            return;
        }
        if !delays && s.channel_interrupt_flags & 0x2 != 0 {
            interrupt(map, g);
            return;
        }
        if !delays {
            return;
        }
    } else {
        // A cast that damage cancels isn't pushed back too.
        if s.interrupt_flags & 0x10 != 0 {
            interrupt(map, g);
            return;
        }
        if s.interrupt_flags & 0x2 == 0 {
            return;
        }
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
    // The channel's auras end that much sooner too, and its areas on the
    // ground with what they hold on those inside.
    if let Some((spell, target, cut)) = delayed
        && !cut.is_zero()
    {
        auras::delay_source(map, target, spell, g, cut);
        effects::delay_areas(map, g, spell, cut);
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

/// `Attributes`: not to be cast in a fight.
const ATTR_NOT_IN_COMBAT: u32 = 0x1000_0000;

/// `g` went into a fight: a cast or a channel that isn't for one (the
/// Hearthstone, a mount) stops, as mangos's `SetInCombatState` and Thistle
/// Tea's `CombatState.enter` have it; an instant one is already done.
pub fn combat_began(map: &mut Map, g: Guid) {
    let data = map.realm.data.clone();
    let stops = map
        .caster(g)
        .and_then(|c| c.casting.as_ref())
        .filter(|c| c.channel_until.is_some() || c.duration > 0)
        .and_then(|c| data.spells.get(c.spell))
        .is_some_and(|s| s.attr(0, ATTR_NOT_IN_COMBAT));
    if stops {
        interrupt(map, g);
    }
}

/// Whatever the unit casts or channels stops, and a next-swing strike
/// waiting is taken back.
pub fn cancel(map: &mut Map, g: Guid) {
    interrupt(map, g);
    crate::game::map::combat::cancel_queued(map, g);
}

/// The global cooldown `spell` started is given back (mangos's
/// `ResetGCD`): a cast that never went off.
fn give_back_gcd(map: &mut Map, g: Guid, spell: u32) {
    let category = map
        .realm
        .data
        .spells
        .get(spell)
        .map_or(0, |s| s.start_recovery_category);
    if let Some(c) = map.caster_mut(g) {
        if category == GCD_SPELLS {
            c.gcd_until = None;
        } else {
            c.other_gcds.retain(|(cat, _)| *cat != category);
        }
    }
}

/// `CMSG_CANCEL_CAST`: the cast or channel of `spell` stops, if that is
/// what is under way, and a next-swing strike of it waiting is taken back
/// (mangos's `InterruptNonMeleeSpells(false, spellId)`; 0 names any). A
/// cancel that comes after its cast is done stops nothing of the next.
pub fn cancel_cast(map: &mut Map, g: Guid, spell: u32) {
    let named = |id: u32| spell == 0 || id == spell;
    if map
        .caster(g)
        .and_then(|c| c.casting.as_ref())
        .is_some_and(|c| named(c.spell))
    {
        interrupt(map, g);
    }
    if map.combat(g).and_then(|c| c.queued).is_some_and(named) {
        crate::game::map::combat::cancel_queued(map, g);
    }
}

/// `CMSG_CANCEL_CHANNELLING`: the channel alone, whatever it names
/// (mangos's `InterruptSpell(CURRENT_CHANNELED_SPELL)`).
pub fn cancel_channel(map: &mut Map, g: Guid) {
    if map
        .caster(g)
        .and_then(|c| c.casting.as_ref())
        .is_some_and(|c| c.channel_until.is_some())
    {
        interrupt(map, g);
    }
}

/// How far from where a cast began a caster may be before it has moved:
/// float noise alone (mangos compares the position exactly).
const MOVED_FROM_CAST: f32 = 0.01;

/// A player moved: a cast that movement interrupts stops, once the caster
/// is away from where it began, however small the steps (mangos's
/// `Spell::update` against `m_castPositionX`).
pub fn moved(map: &mut Map, g: Guid) {
    environment::moved(map, g);
    let here = stance(map, g);
    let Some((spell, channel, from)) = map
        .caster(g)
        .and_then(|c| c.casting.as_ref())
        .map(|c| (c.spell, c.channel_until.is_some(), c.at))
    else {
        return;
    };
    // On the deck it began on, measured there; off it, or onto another,
    // moved.
    if let (Some(from), Some(here)) = (from, here)
        && from.transport == here.transport
        && combat::dist(from.position, here.position) <= MOVED_FROM_CAST
    {
        return;
    }
    // A Stuck (its first effect) goes on through a long fall (mangos's
    // `Spell::update`: `SPELL_EFFECT_STUCK` and `MOVEFLAG_FALLINGFAR`).
    let falling = map
        .players
        .get(&g)
        .is_some_and(|p| p.movement.flags.get_fallingfar());
    let breaks = map.realm.data.spells.get(spell).is_some_and(|s| {
        if falling && s.effects[0].effect == EFFECT_STUCK {
            false
        } else if channel {
            s.channel_interrupt_flags & 0x8 != 0 || s.interrupt_flags & 0x1 != 0
        } else {
            s.interrupt_flags & 0x1 != 0
        }
    });
    if breaks {
        interrupt(map, g);
    }
}

/// `AURA_INTERRUPT_FLAG_TURNING` among a channel's interrupt flags.
const CHANNEL_INTERRUPT_TURNING: u32 = 0x10;
/// How far a player may turn from where a channel began (mangos's
/// "client server mismatch").
const TURN_LEEWAY: f32 = 0.10;
/// `SPELL_ATTR_EX_TRACK_TARGET_IN_CHANNEL`: the client turns to its target
/// as the channel starts.
const ATTR_EX_TRACK_TARGET_IN_CHANNEL: u32 = 0x4000;

/// A player turned or jumped: a channel stops on a jump, and on a turn of
/// more than a tenth of a radian from where it began if its channel
/// interrupt flags say so, more for one the client turns to its target in
/// its first 1.2 s (mangos-classic's `Spell::update` for
/// `SPELL_STATE_CHANNELING`: `MOVEFLAG_JUMPING`, and
/// `AURA_INTERRUPT_FLAG_TURNING` against `m_castOrientation`).
pub fn channel_turned(map: &mut Map, g: Guid, turned: bool, jumped: bool) {
    let now = map.now;
    let Some(c) = map
        .caster(g)
        .and_then(|c| c.casting.as_ref())
        .filter(|c| c.channel_until.is_some())
        .cloned()
    else {
        return;
    };
    let facing = stance(map, g).map_or(0.0, |u| u.orientation);
    let turns = turned
        && map.realm.data.spells.get(c.spell).is_some_and(|s| {
            if s.channel_interrupt_flags & CHANNEL_INTERRUPT_TURNING == 0 {
                return false;
            }
            let Some(from) = c.at.map(|a| a.orientation) else {
                return true;
            };
            let mut leeway = TURN_LEEWAY;
            if s.attr(1, ATTR_EX_TRACK_TARGET_IN_CHANNEL) {
                let began = c
                    .channel_until
                    .map(|u| u - Duration::from_millis(u64::from(c.duration)));
                let elapsed = began.map_or(0, |b| now.saturating_duration_since(b).as_millis());
                leeway += std::f32::consts::PI * (1200u128.saturating_sub(elapsed) as f32 / 1200.0);
            }
            let pi = std::f32::consts::PI;
            let apart = pi - ((from - facing).abs() - pi).abs();
            apart > leeway
        });
    if jumped || turns {
        interrupt(map, g);
    }
}

/// A channel at another unit stops once that unit is half its range again
/// beyond it, past both their reaches (mangos-classic's `Spell::update` for
/// `SPELL_STATE_CHANNELING`: `IsWithinCombatDistInMap(channelTarget,
/// m_maxRange * 1.5f)`, the range with its modifiers).
fn channels_in_reach(map: &mut Map) {
    let at_other = |g: Guid, c: &Caster| {
        let x = c.casting.as_ref()?;
        x.channel_until?;
        let t = x.targets.unit.filter(|u| *u != g)?;
        Some((g, x.spell, t))
    };
    let mut channels: Vec<(Guid, u32, Guid)> = map
        .players
        .values()
        .filter_map(|p| at_other(p.guid, &p.caster))
        .collect();
    channels.extend(
        map.casting_where(
            |c| at_other(c.guid, &c.caster).is_some(),
            |_, c| at_other(c.guid, &c.caster),
        )
        .into_iter()
        .flatten(),
    );
    if channels.is_empty() {
        return;
    }
    let data = map.realm.data.clone();
    for (g, spell, target) in channels {
        let Some(s) = data.spells.get(spell) else {
            continue;
        };
        let (_, base) = data.spells.range(s);
        if base <= 0.0 {
            continue;
        }
        let max = mods::for_spell(map, g, s)
            .apply(mods::OP_RANGE, base)
            .max(0.0);
        let (Some(me), Some(u)) = (map.unit_view(g), map.unit_view(target)) else {
            continue;
        };
        let apart = combat::dist(me.position, u.position) - me.combat_reach - u.combat_reach;
        if apart > max * 1.5 {
            interrupt(map, g);
        }
    }
}

/// Casts that finish now go off; channels that end, end; areas tick; party
/// auras follow their casters.
pub fn update(map: &mut Map) {
    let now = map.now;
    let data = map.realm.data.clone();
    // Areas first: a channel's area has its last tick before the channel,
    // run out, takes it away.
    {
        let _z = crate::zone!("spells: areas");
        effects::tick_areas(map);
    }
    {
        let _z = crate::zone!("spells: channels");
        channel_costs(map);
        channels_in_reach(map);
    }
    let _z = crate::zone!("spells: casts ending");
    let mut done: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.caster.casting.as_ref().is_some_and(|c| c.ends <= now))
        .map(|p| p.guid)
        .collect();
    done.extend(map.casting_where(
        |c| c.caster.casting.as_ref().is_some_and(|x| x.ends <= now),
        |_, c| c.guid,
    ));
    for g in done {
        // The Death Ray's hurt has its last tick while its channel is
        // still under way.
        crate::game::map::item_scripts::channel_running_out(map, g);
        // A channel run its course still has its last tick, while it is
        // still the caster's channel (its trigger aims at its target).
        if let Some(casting) = map
            .caster(g)
            .and_then(|c| c.casting.clone())
            .filter(|c| c.channel_until.is_some())
        {
            let target = casting.targets.unit.unwrap_or(g);
            auras::tick_due(map, target, casting.spell, g);
            if target != g {
                auras::tick_due(map, g, casting.spell, g);
            }
        }
        let Some(casting) = map.caster_mut(g).and_then(|c| c.casting.take()) else {
            continue;
        };
        if casting.channel_until.is_some() {
            end_channel(map, g, &casting);
            // A taming rod's, run its course, tames its beast.
            crate::game::map::pets::rods::channel_done(map, g, casting.spell, casting.targets.unit);
            continue;
        }
        let Some(s) = data.spells.get(casting.spell) else {
            continue;
        };
        // The target must still be here; a player's still alive, a foe, in
        // range and in front as the cast ends, or it fails before it costs.
        let m = mods::for_spell(map, g, s);
        // And its profession's: the lock, node or lockbox still there, in
        // reach, nobody else's and not beyond the skill, its tools and
        // focus by (mangos's `CheckCast(false)` at `Spell::cast`, Thistle
        // Tea's launch recheck), failed before it costs or spends a charge.
        let at_finish =
            checks::at_finish(map, g, s, &casting.targets, casting.item, &m).or_else(|| {
                crate::game::map::professions::check(map, g, s, &casting.targets, casting.item)
            });
        if let Some(code) = at_finish {
            if code == checks::fail::BAD_TARGETS {
                if let Some(p) = map.players.get(&g) {
                    p.outbound.send(&SMSG_SPELL_FAILURE {
                        guid: g,
                        spell: casting.spell,
                        result: SpellCastResult::BadTargets,
                    });
                }
                // Those who saw it begin see it fail (mangos's
                // `SendInterrupted`); a target gone gives the global
                // cooldown back, as mangos's `cancel` at a lost target.
                if let Some(frame) = crate::game::conn::encode(&SMSG_SPELL_FAILED_OTHER {
                    caster: g,
                    id: casting.spell,
                }) {
                    map.send_to_observers(g, &frame, false);
                }
                if casting
                    .targets
                    .unit
                    .is_some_and(|u| u != g && map.unit_view(u).is_none())
                {
                    give_back_gcd(map, g, casting.spell);
                }
            } else {
                launch_failed(map, g, casting.spell, code);
            }
            continue;
        }
        // A player's buff too high for its target now, as the cast ends,
        // fails before it costs (mangos's `CheckCast` again at
        // `Spell::cast`, `SPELL_FAILED_LOWLEVEL`).
        let low = casting.targets.unit.filter(|u| *u != g).is_some_and(|t| {
            casting.item.is_none()
                && map.players.contains_key(&g)
                && map
                    .unit_view(t)
                    .is_some_and(|u| u.level < aura_rank::minimum_level(&data, s))
        });
        if low {
            launch_failed(map, g, casting.spell, checks::fail::LOWLEVEL);
            continue;
        }
        // A gathering, lockpicking or skinning that misses its roll fails
        // here, before its go and its costs (mangos's `CheckCast(false)` at
        // `Spell::cast`: `SPELL_FAILED_TRY_AGAIN`, then `StopCast`).
        if let Some(code) =
            crate::game::map::professions::roll_at_finish(map, g, s, &casting.targets, casting.item)
        {
            launch_failed(map, g, casting.spell, code);
            continue;
        }
        map.spells.ending_began_with = Some(map.spells.began_with.remove(&g).unwrap_or_default());
        execute(map, g, s, casting.targets, casting.item, false);
        map.spells.openings_rolled.remove(&g);
    }
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

/// Channels that cost a second pay it, as mangos's `SpellAuraHolder`
/// update has the caster pay its spell's per-second cost: Health Funnel's
/// health, a channel without the health for it ending.
fn channel_costs(map: &mut Map) {
    let now = map.now;
    let due: Vec<Guid> = map
        .spells
        .channel_costs
        .iter()
        .filter(|(_, t)| **t <= now)
        .map(|(g, _)| *g)
        .collect();
    let data = map.realm.data.clone();
    for g in due {
        let spell = map
            .caster(g)
            .and_then(|c| c.casting.as_ref())
            .filter(|c| c.channel_until.is_some())
            .and_then(|c| data.spells.get(c.spell));
        let Some(s) = spell else {
            map.spells.channel_costs.remove(&g);
            continue;
        };
        map.spells
            .channel_costs
            .insert(g, now + Duration::from_secs(1));
        let level = map.unit_view(g).map(|u| u.level).unwrap_or(1);
        let cost = s.mana_per_second + s.mana_per_second_per_level * level;
        if cost == 0 {
            continue;
        }
        let health = map
            .unit_fields(g)
            .map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH));
        if s.power_type == POWER_HEALTH && health <= cost {
            map.spells.channel_costs.remove(&g);
            if let Some(casting) = map.caster_mut(g).and_then(|c| c.casting.take()) {
                end_channel(map, g, &casting);
            }
            continue;
        }
        pay(map, g, s, cost);
    }
}

/// `CMSG_RESURRECT_RESPONSE`: a resurrection offered is taken.
///
/// Taken from a player, it brings the dead to where its caster stood as
/// they cast it, and they rise as they land: at the teleport's
/// acknowledgement, or the new map's (mangos's
/// `ResurrectUsingRequestDataInit`, `DELAYED_RESURRECT_PLAYER`). One who
/// can't go there (a raid's copy, for a raid group only) rises where they
/// are. A creature's raises them where they are.
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
    // Where the caster stood as they cast it, not where they are now
    // (mangos's `ResurrectUsingRequestData`, Thistle Tea's offer), in the
    // map they were in.
    let key = p.resurrect_map.take().unwrap_or(map.key);
    let offered_at = p.resurrect_at.take();
    let orientation = p.movement.orientation;
    p.resurrect_offer = None;
    let to = offered_at.or_else(|| (key == map.key).then(|| map.position_of(caster)).flatten());
    let Some(to) = to.filter(|_| guid::is_player(caster)) else {
        crate::game::map::death::resurrect_with(map, player, health, mana);
        return;
    };
    if let Some(p) = map.players.get_mut(&player) {
        p.resurrect_on_arrival = Some((health, mana));
    }
    // Another map by its id: the player's own copy of it, if they may
    // enter (mangos's `TeleportTo` by the map, `CanEnter`).
    let far = key.map != map.key.map;
    let dest = if far {
        crate::game::map::MapKey::world(key.map)
    } else {
        map.key
    };
    map.teleport(player, dest, to, orientation);
    // Refused (the copy turned them away and said why): risen where they
    // are, their corpse there bones.
    if far
        && let Some(p) = map.players.get_mut(&player)
        && let Some((health, mana)) = p.resurrect_on_arrival.take()
    {
        crate::game::map::death::resurrect_with(map, player, health, mana);
        crate::game::map::corpses::bones_in(map, key, player);
    }
}

/// A resurrection taken waiting for its teleport to land: risen now.
pub fn resurrect_on_arrival(map: &mut Map, player: Guid) {
    if let Some((health, mana)) = map
        .players
        .get_mut(&player)
        .and_then(|p| p.resurrect_on_arrival.take())
    {
        crate::game::map::death::resurrect_with(map, player, health, mana);
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
pub(crate) fn spent_talent_points(
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

/// `CMSG_LEARN_TALENT`: a rank of a talent of the player's class above
/// the one they have, if they have the points to reach it (a point a rank,
/// as mangos's `LearnTalent` and Thistle Tea's `Talents.validate` count
/// a jump), the row is open and the talent it needs is learned.
pub fn learn_talent(map: &mut Map, player: Guid, talent: u32, rank: u32) {
    let data = map.realm.data.clone();
    let Some(t) = data.spells.talents.get(&talent) else {
        return;
    };
    let Some(p) = map.players.get(&player) else {
        return;
    };
    if let Some(classes) = data.spells.talent_tab_classes.get(&t.tab)
        && classes & (1 << (p.row.class.max(1) - 1)) == 0
    {
        return;
    }
    let have = t
        .ranks
        .iter()
        .rposition(|r| *r != 0 && p.spells.contains(r))
        .map_or(0, |i| i as u32 + 1);
    let Some(&spell) = t.ranks.get(rank as usize) else {
        return;
    };
    if rank < have || spell == 0 || free_talent_points(p, &data) < rank + 1 - have {
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
    if have > 0 {
        let old = t.ranks[have as usize - 1];
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
    // Not one sold, in the vendor's buyback slots.
    let Some(i) = p
        .item_at(bag, slot)
        .filter(|_| !crate::game::map::vendor::in_buyback(bag, slot))
    else {
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
    // Gear is used worn; and an item by one who may use it: of its class
    // and race, level, skill and spell, standing and rank (mangos's
    // `HandleUseItemOpcode` and `CanUseItem`).
    let worn = item.bag == crate::game::store::BAG_PLAYER
        && item.slot < crate::game::map::inventory::EQUIPMENT_END;
    let usable = if template.inventory_type != 0 && !worn {
        Err(crate::game::map::inventory::Fail::NotFound)
    } else {
        crate::game::map::inventory::can_use(p, &data, template)
    };
    if let Err(f) = usable {
        crate::game::map::inventory::send_fail(p, f, item_guid);
        return;
    }
    // A book that teaches only what is known is refused, not spent.
    let teaches: Vec<u32> = data
        .spells
        .get(spell.id)
        .map(|s| {
            s.effects
                .iter()
                .filter(|e| e.effect == EFFECT_LEARN_SPELL && e.trigger_spell != 0)
                .map(|e| e.trigger_spell)
                .collect()
        })
        .unwrap_or_default();
    if !teaches.is_empty() && teaches.iter().all(|t| p.spells.contains(t)) {
        send_cast_result(map, player, spell.id, Some(fail::SPELL_LEARNED));
        return;
    }
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
