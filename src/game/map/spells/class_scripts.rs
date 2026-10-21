//! The rest of Thistle Tea's class spell scripts (its `Spell.Scripts`,
//! `Class.*` and `Aura.Script`, after vmangos's), for what the spells'
//! data can't say:
//!
//! - what a spell leaves on whom it lands on: Power Word: Shield's Weakened
//!   Soul, the paladin bubbles' Forbearance, First Aid's Recently Bandaged;
//! - what it casts when it is done: Holy Nova's heal;
//! - damage from more than its points: Bloodthirst (a share of attack
//!   power), Ferocious Bite (attack power and the energy left, which it
//!   spends), Shield Slam (the block value), Judgement of Command (half,
//!   unless the target is stunned), Hammer of Wrath (a melee crit);
//! - heals and deaths: Swiftmend (eats a Rejuvenation or Regrowth),
//!   Demonic Sacrifice (a buff for the demon given);
//! - dummies: druid Enrage, Judgement of Command's seal, Readiness and
//!   Refocus, Flametongue Weapon's attack, Touch of Weakness by rank;
//! - Vanish: free of snares and roots, and stealthed;
//! - auras: Wyvern Sting's poison when the sleep ends, Bestial Wrath's
//!   immunities on the pet, Stoneform's disease immunity and Shadowmeld's
//!   stealth (both as extra effects of the spell);
//! - targets: Intimidating Shout's (its target cowers, the rest run),
//!   Mongoose Bite's and Counterattack's (the unit whose attack opened the
//!   window), Devour Magic's heal for the felhunter, Inferno's infernal;
//! - periodic timing: the totems that tick at once, and the auras whose
//!   rhythm a refresh keeps.

use rand::RngExt;
use wow_world_messages::Guid;

use crate::game::data::spells::{SpellData, SpellTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras::{self, AURA_MOD_STUN, AURA_PERIODIC_HEAL, Holder};
use crate::game::map::combat;
use crate::game::map::inventory::SLOT_MAIN_HAND;
use crate::game::store::BAG_PLAYER;

use super::outcome::Landing;
use super::{Targets, cast_triggered, fail};

pub const WEAKENED_SOUL: u32 = 6788;
const WOLFSHEAD_HELM_ENERGY: u32 = 29940;
/// Heart of the Wild's bear and cat form auras.
const HEART_OF_THE_WILD_FORMS: [u32; 2] = [24899, 24900];
/// Collecting Fallout's effects, one of which a cast keeps.
const ONE_OF: [usize; 2] = [0, 1];
const SHAPESHIFT_CLEANSE: u32 = 9033;
const CAPTURE_CREATURE: u32 = 15998;
const MERGE_OOZES: u32 = 16032;
const GARGANTUAN_OOZE: u32 = 9621;
const RESTORE_TO_LIFE: u32 = 8593;
const SPIRIT_SPAWN_OUT: u32 = 17680;
/// Mechanics a druid's form doesn't shake off.
const PROTECTED_MECHANICS: [u32; 12] = [1, 2, 5, 8, 12, 13, 18, 20, 23, 24, 27, 30];
/// Stealth, Vanish's and Prowl's icons: casting them keeps stealth.
const STEALTH_ICONS: [u32; 3] = [250, 103, 252];
const SAP_ICON: u32 = 249;
const IMPROVED_SAP: [(u32, u32); 3] = [(14095, 90), (14094, 60), (14076, 30)];
const CANNIBALIZE_EMOTE: u32 = 398;
pub const FORBEARANCE: u32 = 25771;
pub const RECENTLY_BANDAGED: u32 = 11196;
const ENRAGE_ARMOR: u32 = 25503;
const FLAMETONGUE_ATTACK: u32 = 10444;
const SHADOWMELD: u32 = 20580;
const SHADOWMELD_PASSIVE: u32 = 21009;
const BESTIAL_WRATH: u32 = 19574;
const BESTIAL_WRATH_IMMUNITIES: [u32; 4] = [24395, 24396, 24397, 26592];
const INFERNAL: u32 = 89;
const ENSLAVE_INFERNAL: u32 = 20882;
const INFERNAL_ROOT: u32 = 22707;
const INFERNAL_LANDING: u32 = 22703;
const DISPEL_DISEASE: i32 = 3;

const FAMILY_PRIEST: u32 = 6;
const FAMILY_WARLOCK: u32 = 5;
const FAMILY_DRUID: u32 = 7;
const FAMILY_ROGUE: u32 = 8;
const ROGUE_STEALTH: u64 = 0x0040_0000;
const FAMILY_HUNTER: u32 = 9;
const DRUID_REJUVENATION: u64 = 0x10;
const DRUID_REGROWTH: u64 = 0x40;
const HUNTER_REFOCUS: u64 = 0x0002_3800;

/// Holy Nova's damage ranks, and the heal each casts.
const HOLY_NOVA: [(u32, u32); 6] = [
    (15237, 23455),
    (15430, 23458),
    (15431, 23459),
    (27799, 27803),
    (27800, 27804),
    (27801, 27805),
];
/// Touch of Weakness's ranks, and the damage each does.
const TOUCH_OF_WEAKNESS: [(u32, u32); 6] = [
    (2652, 2943),
    (19261, 19249),
    (19262, 19251),
    (19264, 19252),
    (19265, 19253),
    (19266, 19254),
];
/// Wyvern Sting's ranks, and the poison each leaves.
const WYVERN_STING: [(u32, u32); 3] = [(19386, 24131), (24132, 24134), (24133, 24135)];
/// The demons Demonic Sacrifice gives up, and what each gives.
const SACRIFICE: [(u32, u32); 4] = [(416, 18789), (1860, 18790), (1863, 18791), (417, 18792)];
/// Devour Magic's ranks, and the heal each gives the felhunter.
const DEVOUR_MAGIC: [(u32, u32); 4] = [
    (19505, 19658),
    (19731, 19732),
    (19734, 19733),
    (19736, 19735),
];
/// Totems whose first pulse is at once.
const IMMEDIATE_PERIODIC: [u32; 8] = [8145, 6474, 8179, 8172, 8167, 8515, 10609, 10612];
/// More whose rhythm a refresh keeps.
const PRESERVED_PERIODIC: [u32; 7] = [13797, 14298, 14299, 14300, 14301, 23184, 25041];

fn script(s: &SpellTemplate, name: &str) -> bool {
    s.script_name == name
}

fn pair(table: &[(u32, u32)], id: u32) -> Option<u32> {
    table.iter().find(|(k, _)| *k == id).map(|(_, v)| *v)
}

fn family_flag(s: &SpellTemplate, family: u32, mask: u64) -> bool {
    s.family_name == family && s.family_flags & mask != 0
}

/// What `s` leaves on whom it lands on, beside its own aura.
pub fn apply_trigger(s: &SpellTemplate) -> Option<u32> {
    match s.script_name.as_str() {
        "spell_priest_power_word_shield" => Some(WEAKENED_SOUL),
        "spell_paladin_bubble" => Some(FORBEARANCE),
        "spell_first_aid" => Some(RECENTLY_BANDAGED),
        _ => None,
    }
}

/// What the caster casts when `s` is done.
pub fn finish_trigger(s: &SpellTemplate) -> Option<u32> {
    if script(s, "spell_wolfshead_helm") {
        return Some(WOLFSHEAD_HELM_ENERGY);
    }
    script(s, "spell_priest_holy_nova")
        .then(|| pair(&HOLY_NOVA, s.id))
        .flatten()
}

/// `s` landed on `target`: what it leaves there with its aura.
pub fn landed(map: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate) {
    let Some(trigger) = apply_trigger(s) else {
        return;
    };
    if auras::auras(map, target).is_some_and(|a| a.has_spell(s.id)) {
        cast_triggered(map, caster, trigger, Some(target));
    }
}

/// A school damage effect's base for the scripts that add to it.
pub fn damage_base(
    map: &mut Map,
    caster: Guid,
    s: &SpellTemplate,
    index: usize,
    base: f32,
    l: &Landing,
) -> f32 {
    let d = combat::derived(map, caster);
    let mut amount = base;
    if script(s, "spell_druid_ferocious_bite") {
        let energy = map
            .unit_fields(caster)
            .map_or(0, |f| f.get_u32(UNIT_FIELD_POWER1 + 3));
        amount += d.attack_power as f32 * f32::from(l.combo) * 0.03
            + energy as f32 * s.dmg_multiplier[index].max(0.0);
        // The energy left goes into the bite.
        if let Some(f) = map.unit_fields_mut(caster) {
            f.set_u32(UNIT_FIELD_POWER1 + 3, 0);
        }
    }
    if script(s, "spell_warrior_shield_slam") && index == 1 {
        amount += d.block_value as f32;
    }
    if script(s, "spell_warrior_bloodthirst") {
        amount = amount * d.attack_power as f32 / 100.0;
    }
    amount
}

/// A scripted multiplier on a school damage effect's damage.
pub fn damage_multiplier(map: &Map, target: Guid, s: &SpellTemplate) -> f32 {
    if script(s, "spell_paladin_judgement_of_command_damage")
        && !auras::auras(map, target).is_some_and(|a| a.has(AURA_MOD_STUN))
    {
        return 0.5;
    }
    1.0
}

/// Whether `s` crits as a melee attack does (Hammer of Wrath).
pub fn melee_crit(s: &SpellTemplate) -> bool {
    script(s, "spell_paladin_hammer_of_wrath")
}

fn swiftmend_hot(map: &Map, h: &Holder) -> bool {
    map.realm
        .data
        .spells
        .get(h.spell)
        .is_some_and(|t| family_flag(t, FAMILY_DRUID, DRUID_REJUVENATION | DRUID_REGROWTH))
        && h.has(AURA_PERIODIC_HEAL)
}

/// Why `caster` can't cast `s` at `target`, for the scripts that say.
pub fn check_target(map: &Map, caster: Guid, s: &SpellTemplate, target: Guid) -> Option<u8> {
    if script(s, "spell_druid_swiftmend")
        && !auras::auras(map, target)
            .is_some_and(|a| a.holders.iter().any(|h| swiftmend_hot(map, h)))
    {
        return Some(fail::TARGET_AURASTATE);
    }
    // Conflagrate wants the caster's Immolate on its target (Thistle Tea's
    // `check_warlock_target`).
    if script(s, "spell_warlock_conflagrate")
        && !auras::auras(map, target).is_some_and(|a| {
            a.holders.iter().any(|h| {
                h.caster == caster
                    && map
                        .realm
                        .data
                        .spells
                        .get(h.spell)
                        .is_some_and(|t| family_flag(t, FAMILY_WARLOCK, 0x4))
            })
        })
    {
        return Some(fail::TARGET_AURASTATE);
    }
    // Mongoose Bite and Counterattack are at whoever opened their window.
    let window = if script(s, "spell_hunter_mongoose_bite") {
        map.combat(caster).and_then(|c| c.defense_window)
    } else if script(s, "spell_hunter_counterattack") {
        map.combat(caster).and_then(|c| c.hunter_parry_window)
    } else {
        return None;
    };
    match window {
        Some((attacker, until)) if attacker == target && until > map.now => None,
        _ => Some(fail::BAD_TARGETS),
    }
}

/// Swiftmend eats `target`'s Rejuvenation or Regrowth (the one with the
/// least time left) for four or six of its ticks; the heal it adds.
pub fn swiftmend(map: &mut Map, target: Guid, s: &SpellTemplate) -> i32 {
    if !script(s, "spell_druid_swiftmend") {
        return 0;
    }
    let now = map.now;
    let eaten = auras::auras(map, target).and_then(|a| {
        a.holders
            .iter()
            .filter(|h| swiftmend_hot(map, h) && h.expires.is_none_or(|t| t > now))
            .min_by_key(|h| h.expires)
            .map(|h| {
                let tick = h
                    .effects
                    .iter()
                    .find(|x| x.kind == AURA_PERIODIC_HEAL)
                    .map_or(0, |x| x.amount);
                let regrowth = map
                    .realm
                    .data
                    .spells
                    .get(h.spell)
                    .is_some_and(|t| t.family_flags & DRUID_REGROWTH != 0);
                (h.spell, h.caster, tick * if regrowth { 6 } else { 4 })
            })
    });
    let Some((spell, by, heal)) = eaten else {
        return 0;
    };
    auras::remove_spell(map, target, spell, Some(by));
    heal
}

/// Demonic Sacrifice: the demon given up gives its warlock its buff.
pub fn sacrifice(map: &mut Map, caster: Guid, demon: Guid, s: &SpellTemplate) {
    if !script(s, "spell_warlock_demonic_sacrifice") {
        return;
    }
    if let Some(buff) = map.creature(demon).and_then(|c| pair(&SACRIFICE, c.entry)) {
        cast_triggered(map, caster, buff, Some(caster));
    }
}

/// A dummy or script effect a class script owns; whether it did it.
pub fn dummy(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    index: usize,
    amount: i32,
) -> bool {
    // Once a cast, at its first dummy or script effect: Summon Screecher
    // Spirit's is its second, its first the summon.
    let first = s
        .effects
        .iter()
        .position(|e| matches!(e.effect, super::EFFECT_DUMMY | super::EFFECT_SCRIPT_EFFECT));
    if first == Some(index) && id_dummy(map, caster, target, s) {
        return true;
    }
    match s.script_name.as_str() {
        "spell_druid_enrage" => {
            // Less armor: less in dire bear form.
            let form = auras::form(map, caster);
            let less = if form == auras::FORM_DIREBEAR {
                -16
            } else {
                -27
            };
            super::scripts::cast_with_points(
                map,
                caster,
                ENRAGE_ARMOR,
                caster,
                [None, Some(less), None],
            );
            true
        }
        "spell_paladin_judgement_of_command_dummy" => {
            if amount > 1 {
                cast_triggered(map, caster, amount as u32, Some(target));
            }
            true
        }
        "spell_hunter_readiness" => {
            super::scripts::reset_cooldowns(map, caster, s.id, |t| t.family_name == FAMILY_HUNTER);
            true
        }
        "spell_hunter_refocus" => {
            super::scripts::reset_cooldowns(map, caster, s.id, |t| {
                family_flag(t, FAMILY_HUNTER, HUNTER_REFOCUS)
            });
            true
        }
        "spell_shaman_flametongue_proc_dummy" => {
            flametongue(map, caster, target, s, index, amount);
            true
        }
        _ => false,
    }
}

/// Flametongue Weapon's attack: the proc's points and 3.85 times fire
/// spell power, a hundredth of it a second of the weapon's speed.
fn flametongue(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    _index: usize,
    amount: i32,
) {
    let data = map.realm.data.clone();
    let Some(attack) = data.spells.get(FLAMETONGUE_ATTACK) else {
        return;
    };
    let fire = super::outcome::power(map, caster, target, attack, false);
    let speed = map
        .players
        .get(&caster)
        .and_then(|p| {
            p.item_at(BAG_PLAYER, SLOT_MAIN_HAND)
                .map(|i| p.items[i].entry)
        })
        .and_then(|entry| data.item(entry))
        .map_or(2000, |t| t.delay.max(1));
    let damage = flametongue_damage(amount, fire, speed);
    let mut strike = attack.clone();
    // Its spell power is in already.
    strike.attributes[3] |= crate::game::data::spells::ATTR_EX3_IGNORE_CASTER_MODIFIERS;
    let _ = s;
    super::scripts::cast_template_with_points(
        map,
        caster,
        strike,
        target,
        [Some(damage.max(1)), None, None],
    );
}

/// Flametongue's attack of the proc's `points` and `fire` spell power, with
/// a weapon of `speed_ms`: vmangos's points and 3.85 times fire power, a
/// hundredth of it a second of the weapon's speed, rounded.
pub(crate) fn flametongue_damage(points: i32, fire: f32, speed_ms: u32) -> i32 {
    ((points as f32 + 3.85 * fire) * 0.01 * speed_ms as f32 / 1000.0).round() as i32
}

/// The spell an aura's proc casts: Touch of Weakness's damage by its rank.
pub fn proc_spell(aura_spell: u32, trigger: u32) -> u32 {
    pair(&TOUCH_OF_WEAKNESS, aura_spell).unwrap_or(trigger)
}

/// Intimidating Shout: the one it is at cowers (its first effect), the
/// rest flee (the others).
pub fn effects_for(
    s: &SpellTemplate,
    target: Guid,
    t: &Targets,
    effects: [bool; 3],
    chosen: Option<usize>,
) -> [bool; 3] {
    if let Some(keep) = chosen {
        let mut out = effects;
        for i in ONE_OF {
            out[i] = out[i] && i == keep;
        }
        return out;
    }
    if !script(s, "spell_warrior_intimidating_shout") {
        return effects;
    }
    let main = t.unit == Some(target);
    let mut out = effects;
    for (i, e) in out.iter_mut().enumerate() {
        *e = *e && ((i == 0) == main);
    }
    out
}

/// Which of Collecting Fallout's effects a cast keeps: one of its first
/// two.
pub fn one_of(map: &Map, s: &SpellTemplate) -> Option<usize> {
    if !script(s, "spell_gnomeregan_collecting_fallout") {
        return None;
    }
    Some(if super::outcome::rolling(map) {
        ONE_OF[rand::rng().random_range(0..ONE_OF.len())]
    } else {
        ONE_OF[0]
    })
}

/// The dummies vmangos scripts by spell id.
fn id_dummy(map: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate) -> bool {
    let dead = map.creature(target).is_some_and(|c| !c.alive());
    let now = map.now;
    match s.id {
        // Druid shapeshifting's cleanse: roots and snares go.
        SHAPESHIFT_CLEANSE => {
            let data = map.realm.data.clone();
            auras::remove_where(map, target, |h| shapeshift_removable(&data, h));
        }
        // A captured corpse goes in a moment, a captured creature at once.
        11885..=11889 | 12699 if dead => {
            if let Some(c) = map.creature_mut(target)
                && let crate::game::map::creature::Life::Dead { corpse_until, .. } = &mut c.life
            {
                *corpse_until = now + std::time::Duration::from_secs(1);
            }
        }
        11885..=11889 | 12699 => {}
        CAPTURE_CREATURE if map.creature(target).is_some() => {
            crate::game::map::scripts::despawn(map, target);
        }
        // Two oozes make a gargantuan one.
        MERGE_OOZES if map.creature(caster).is_some() && map.creature(target).is_some() => {
            let at = map.creature(caster).map(|c| (c.position, c.orientation));
            if let Some((at, o)) = at {
                let _ = crate::game::map::scripts::summon(
                    map,
                    caster,
                    GARGANTUAN_OOZE,
                    at,
                    o,
                    Some((std::time::Duration::from_secs(420), 1)),
                );
            }
            crate::game::map::scripts::despawn(map, target);
        }
        // A dead creature lives again, for two minutes.
        RESTORE_TO_LIFE if dead => {
            let data = map.realm.data.clone();
            if let Some(c) = map.creature_mut(target) {
                let max = c.fields.get_u32(UNIT_FIELD_MAXHEALTH);
                c.life = crate::game::map::creature::Life::Alive;
                c.despawned = false;
                c.fields.set_u32(UNIT_FIELD_HEALTH, max.max(1));
                c.fields.set_u32(UNIT_DYNAMIC_FLAGS, 0);
                // Its NPC flags, gone with its death, come back as its
                // template has them (mangos's `JUST_ALIVED`).
                let npc_flags = data.creature_template(c.entry).map_or(0, |t| t.npc_flags);
                c.fields.set_u32(UNIT_NPC_FLAGS, npc_flags);
                // Nothing left on it to loot, and no one's kill.
                c.loot = None;
                crate::game::map::tap::clear(c);
                let s = c.scripted.get_or_insert_with(Default::default);
                s.despawn_at = Some(now + std::time::Duration::from_secs(120));
                s.despawn_type = 1;
            }
            map.dead_creatures.retain(|d| *d != target);
            map.visibility_dirty.insert(target);
        }
        // A spirit out of combat goes.
        28091 => {
            let fighting = map.unit_fields(target).is_some_and(|f| {
                f.has_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                )
            });
            if map.creature(target).is_some() && !fighting {
                cast_triggered(map, target, SPIRIT_SPAWN_OUT, Some(target));
            }
        }
        28006 => cast_triggered(map, caster, 29296, Some(target)),
        28345 => cast_triggered(map, caster, 28281, Some(target)),
        _ => return false,
    }
    true
}

/// What druid shapeshifting shakes off, as Thistle Tea's
/// `Shapeshift.removable_spells` has it: roots, and snares but those of
/// mechanics a form can't escape.
pub fn shapeshift_removable(data: &crate::game::data::GameData, h: &Holder) -> bool {
    if h.positive || h.passive {
        return false;
    }
    let s = data.spells.get(h.spell);
    let mechanics: Vec<u32> = std::iter::once(h.mechanic)
        .chain(s.iter().flat_map(|s| s.effects.iter().map(|e| e.mechanic)))
        .filter(|m| (1..=31).contains(m))
        .collect();
    if mechanics.is_empty() {
        return false;
    }
    let icon = s.map_or(0, |s| s.spell_icon);
    h.has(auras::AURA_MOD_ROOT)
        || (h.has(auras::AURA_MOD_DECREASE_SPEED)
            && !mechanics.iter().any(|m| PROTECTED_MECHANICS.contains(m))
            && !(icon == 15 && h.dispel == 0 && !mechanics.contains(&11)))
}

/// Curse of Idiocy pulses until its caster's own target has lost 90
/// intellect and spirit to it (and never on its caster).
pub fn periodic_trigger_allowed(map: &Map, target: Guid, h: &Holder) -> bool {
    let idiocy = map
        .realm
        .data
        .spells
        .get(h.spell)
        .is_some_and(|t| script(t, "spell_warlock_curse_of_idiocy"));
    if !idiocy {
        return true;
    }
    if h.caster == target {
        return false;
    }
    let lost = |stat: i32| -> i32 {
        auras::auras(map, target).map_or(0, |a| {
            a.holders
                .iter()
                .filter(|x| x.spell == h.spell)
                .flat_map(|x| {
                    x.effects
                        .iter()
                        .filter(|e| e.kind == auras::AURA_MOD_STAT && e.misc == stat)
                        .map(move |e| e.amount * x.stacks.max(1) as i32)
                })
                .sum()
        })
    };
    !(lost(3) <= -90 && lost(4) <= -90)
}

/// A periodic trigger went off: the Frost Trap's aura counts as its trap
/// going off, for Entrapment.
pub fn periodic_triggered(map: &mut Map, target: Guid, h: &Holder) {
    let frost_trap = map
        .realm
        .data
        .spells
        .get(h.spell)
        .is_some_and(|t| script(t, "spell_hunter_frost_trap_aura"));
    if frost_trap && map.unit_view(target).is_some_and(|u| u.alive) {
        crate::game::map::procs::trigger(
            map,
            crate::game::map::procs::Event {
                actor: h.caster,
                target,
                actor_flags: crate::game::map::procs::PROC_TRAP,
                target_flags: 0,
                ex: crate::game::map::procs::EX_NORMAL_HIT,
                spell: Some(h.spell),
                damage: 0,
                school: h.school,
                offhand: false,
                ranged: false,
                extra_attack: false,
                origin: Default::default(),
            },
        );
    }
}

/// Whether stealth allows `s`: what says so, and stealth's own spells.
pub fn allows_stealth(s: &SpellTemplate) -> bool {
    STEALTH_ICONS.contains(&s.spell_icon)
        || s.attr(1, crate::game::data::spells::ATTR_EX_ALLOW_WHILE_STEALTHED)
}

/// Whether casting `s` keeps `caster`'s stealth: the spells stealth allows,
/// and Sap with Improved Sap's chance (Thistle Tea's `Spell.Stealth`).
pub fn keeps_stealth(map: &Map, caster: Guid, s: &SpellTemplate) -> bool {
    if allows_stealth(s) {
        return true;
    }
    if s.spell_icon != SAP_ICON {
        return false;
    }
    let known = |id: u32| {
        map.players
            .get(&caster)
            .is_some_and(|p| p.spells.contains(&id))
            || auras::auras(map, caster).is_some_and(|a| a.has_spell(id))
    };
    IMPROVED_SAP
        .iter()
        .find(|(id, _)| known(*id))
        .is_some_and(|(_, chance)| super::outcome::chance(map, *chance as f32))
}

/// A weapon put on in a fight costs the global cooldown (Thistle Tea's
/// `EquipmentTransitions.weapon_cooldown`).
pub fn weapon_equipped(map: &mut Map, player: Guid, entry: u32) {
    let Some(t) = map.realm.data.item(entry).cloned() else {
        return;
    };
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let fighting = p.fields.has_flag(
        UNIT_FIELD_FLAGS,
        crate::game::map::player::UNIT_FLAG_IN_COMBAT,
    );
    if !fighting || p.death != crate::game::map::death::Death::Alive {
        return;
    }
    if t.class != 2 && t.inventory_type != 28 {
        return;
    }
    let spell = if p.row.class == 4 { 6123 } else { 6119 };
    let Some(s) = map.realm.data.spells.get(spell).cloned() else {
        return;
    };
    if s.start_recovery_time == 0 {
        return;
    }
    let now = map.now;
    let gcd = std::time::Duration::from_millis(u64::from(s.start_recovery_time));
    let Some(c) = map.caster_mut(player) else {
        return;
    };
    // Weapons stay as they are until the lock is up (Thistle Tea's
    // `Cooldowns.start_weapon_change`).
    if c.weapon_change_until.is_some_and(|t| t > now) {
        return;
    }
    c.weapon_change_until = Some(now + gcd);
    if c.gcd_until.is_none_or(|t| t <= now) {
        c.gcd_until = Some(now + gcd);
    }
    super::cooldowns::send(map, player, &[(spell, 0)]);
}

/// Why a fighting player can't swap `old` for `new` in an equipment slot,
/// as Thistle Tea's `validate_combat_change` has it: only weapons,
/// ammunition, shields, held things and relics change in a fight, and a
/// weapon not while the last change's lock holds.
pub fn combat_change_refused(
    p: &crate::game::map::player::Player,
    data: &crate::game::data::GameData,
    old: Option<u32>,
    new: Option<u32>,
    now: std::time::Instant,
) -> Option<crate::game::map::inventory::Fail> {
    use crate::game::map::inventory::Fail;
    let fighting = p.fields.has_flag(
        UNIT_FIELD_FLAGS,
        crate::game::map::player::UNIT_FLAG_IN_COMBAT,
    );
    if !fighting {
        return None;
    }
    let changeable = |entry: Option<u32>| {
        entry
            .and_then(|e| data.item(e))
            .is_none_or(|t| matches!(t.class, 2 | 6) || matches!(t.inventory_type, 14 | 23 | 28))
    };
    if !changeable(old) || !changeable(new) {
        return Some(Fail::NotInCombat);
    }
    let weapon = new.and_then(|e| data.item(e)).is_some_and(|t| t.class == 2);
    if weapon && p.caster.weapon_change_until.is_some_and(|t| t > now) {
        return Some(Fail::CantDoRightNow);
    }
    None
}

/// Heart of the Wild's form auras (Thistle Tea's `Druid.form_aura_ids`):
/// in bear form its share more stamina, in cat form more strength, while
/// the form lasts.
pub fn heart_of_the_wild(map: &mut Map, g: Guid, form: u8) {
    for id in HEART_OF_THE_WILD_FORMS {
        auras::remove_spell(map, g, id, Some(g));
    }
    let data = map.realm.data.clone();
    let amount = auras::auras(map, g).and_then(|a| {
        a.effects()
            .find(|(h, x)| {
                x.kind == auras::AURA_MOD_TOTAL_STAT_PERCENTAGE
                    && x.misc == 3
                    && data
                        .spells
                        .get(h.spell)
                        .is_some_and(|t| t.spell_icon == 240)
            })
            .map(|(_, x)| x.amount)
    });
    let Some(amount) = amount else {
        return;
    };
    let id = match form {
        auras::FORM_CAT => HEART_OF_THE_WILD_FORMS[1],
        auras::FORM_BEAR | auras::FORM_DIREBEAR => HEART_OF_THE_WILD_FORMS[0],
        _ => return,
    };
    if let Some(t) = data.spells.get(id) {
        let effects: Vec<(usize, i32)> = (0..3)
            .filter(|i| t.effects[*i].effect == 6 && t.effects[*i].aura != 0)
            .map(|i| (i, amount))
            .collect();
        let _ = auras::apply_spell(map, g, g, t, &effects, auras::Context::default());
    }
}

/// A dispel took something: Devour Magic heals the felhunter.
pub fn dispelled(map: &mut Map, caster: Guid, s: &SpellTemplate) {
    if script(s, "spell_warlock_devour_magic")
        && let Some(heal) = pair(&DEVOUR_MAGIC, s.id)
    {
        cast_triggered(map, caster, heal, Some(caster));
    }
}

/// After `s`'s effects: Vanish frees the rogue and stealths them with
/// their best Stealth; Inferno's infernal is enslaved, rooted a moment,
/// and lands among its foes.
pub fn after_effects(map: &mut Map, caster: Guid, s: &SpellTemplate) {
    if script(s, "spell_rogue_vanish") {
        vanish(map, caster);
        return;
    }
    if !script(s, "spell_warlock_inferno") {
        return;
    }
    let infernal = map
        .creatures
        .iter()
        .filter(|(_, c)| c.owner == Some(caster) && c.entry == INFERNAL && c.alive())
        .map(|(_, c)| c.guid)
        .max_by_key(|g| g.guid());
    let Some(infernal) = infernal else {
        return;
    };
    cast_triggered(map, caster, ENSLAVE_INFERNAL, Some(infernal));
    cast_triggered(map, infernal, INFERNAL_ROOT, Some(infernal));
    cast_triggered(map, infernal, INFERNAL_LANDING, None);
}

/// How long after Vanish no one detects the rogue.
pub const VANISH_UNDETECTABLE: std::time::Duration = std::time::Duration::from_secs(1);

fn vanish(map: &mut Map, rogue: Guid) {
    // For a second no one detects the rogue's stealth, close up too
    // (mangos-classic's vanilla `SetCannotBeDetectedTimer(1000)` in
    // `EffectSanctuary`, Thistle Tea's `undetectable_until`).
    let until = map.now + VANISH_UNDETECTABLE;
    if let Some(p) = map.players.get_mut(&rogue) {
        p.misc.undetectable_until = Some(until);
    }
    auras::remove_where(map, rogue, |h| {
        !h.positive
            && (h.has(auras::AURA_MOD_STALKED)
                || h.has(auras::AURA_MOD_ROOT)
                || h.has(auras::AURA_MOD_DECREASE_SPEED))
    });
    let data = map.realm.data.clone();
    let stealth = map.players.get(&rogue).and_then(|p| {
        p.spells
            .iter()
            .filter_map(|id| data.spells.get(*id))
            .filter(|t| family_flag(t, FAMILY_ROGUE, ROGUE_STEALTH))
            .max_by_key(|t| data.spells.ranks.get(&t.id).map_or(0, |r| r.rank))
            .map(|t| t.id)
    });
    if let Some(stealth) = stealth {
        if let Some(c) = map.caster_mut(rogue) {
            c.cooldowns.remove(&stealth);
        }
        cast_triggered(map, rogue, stealth, Some(rogue));
    }
}

fn cannibalizing(map: &Map, h: &Holder) -> bool {
    map.realm
        .data
        .spells
        .get(h.spell)
        .is_some_and(|t| script(t, "spell_cannibalize_aura"))
}

/// An aura went on: Stoneform's disease immunity beside its poison one
/// (Thistle Tea's `extra_effects`, which the spell's three effects have no
/// room for), and Bestial Wrath's immunities on the pet.
pub fn aura_applied(map: &mut Map, target: Guid, h: &Holder) {
    if cannibalizing(map, h)
        && let Some(f) = map.unit_fields_mut(target)
    {
        f.set_u32(UNIT_NPC_EMOTESTATE, CANNIBALIZE_EMOTE);
    }
    let stoneform = map
        .realm
        .data
        .spells
        .get(h.spell)
        .is_some_and(|t| script(t, "spell_stoneform"));
    if stoneform {
        let spell = h.spell;
        if let Some(a) = auras::auras_mut(map, target)
            && let Some(holder) = a.holders.iter_mut().find(|x| x.spell == spell)
            && let Some(mut extra) = holder
                .effects
                .iter()
                .find(|x| x.kind == auras::AURA_DISPEL_IMMUNITY)
                .cloned()
        {
            extra.misc = DISPEL_DISEASE;
            holder.effects.push(extra);
        }
        auras::remove_where(map, target, |x| {
            !x.positive && !x.passive && x.dispel == DISPEL_DISEASE as u32
        });
    }
    let bestial = map
        .realm
        .data
        .spells
        .get(h.spell)
        .is_some_and(|t| t.id == BESTIAL_WRATH && t.family_name == FAMILY_HUNTER);
    if bestial {
        for id in BESTIAL_WRATH_IMMUNITIES {
            if let Some(t) = map.realm.data.spells.get(id).cloned() {
                let level = map.unit_view(h.caster).map_or(1, |u| u.level);
                let effects: Vec<(usize, i32)> = (0..3)
                    .filter(|i| t.effects[*i].effect == 6 && t.effects[*i].aura != 0)
                    .map(|i| (i, super::points(&t, i, level)))
                    .collect();
                let _ = auras::apply_spell(
                    map,
                    h.caster,
                    target,
                    &t,
                    &effects,
                    auras::Context {
                        duration: Some(h.duration),
                        ..Default::default()
                    },
                );
            }
        }
    }
}

/// An aura came off: Wyvern Sting's sleep leaves its poison (unless it was
/// death that ended it); Bestial Wrath's immunities go with it.
pub fn aura_gone(map: &mut Map, target: Guid, h: &Holder) {
    if cannibalizing(map, h)
        && let Some(f) = map.unit_fields_mut(target)
    {
        f.set_u32(UNIT_NPC_EMOTESTATE, 0);
    }
    if let Some(poison) = pair(&WYVERN_STING, h.spell)
        && map.unit_view(target).is_some_and(|u| u.alive)
        && map.unit_view(h.caster).is_some()
    {
        cast_triggered(map, h.caster, poison, Some(target));
    }
    if h.spell == BESTIAL_WRATH {
        for id in BESTIAL_WRATH_IMMUNITIES {
            auras::remove_spell(map, target, id, Some(h.caster));
        }
    }
    // Combustion and its crit go together, either way (mangos's
    // `RemoveSpellAuraHolder`): cancelling the crit takes Combustion.
    let other = match h.spell {
        COMBUSTION => COMBUSTION_CRIT,
        COMBUSTION_CRIT => COMBUSTION,
        _ => return,
    };
    if auras::auras(map, target).is_some_and(|a| a.has_spell(other)) {
        auras::remove_spell(map, target, other, None);
    }
}

const COMBUSTION: u32 = 11129;
const COMBUSTION_CRIT: u32 = 28682;

/// Improved Blizzard's chill by its script.
const BLIZZARD_CHILL: [(i32, u32); 3] = [(836, 12484), (988, 12485), (989, 12486)];
/// Holy Power's and Totemic Power's buffs: for mana users, spell casters,
/// fighters and warriors.
const HEALING_POWER: [(u32, [u32; 4]); 2] = [
    (28789, [28795, 28793, 28791, 28790]),
    (28823, [28824, 28825, 28826, 28827]),
];
/// Obsidian Armor's and Adaptive Warding's answers by school index.
const OBSIDIAN_ARMOR: u32 = 27539;
const ADAPTIVE_WARDING: u32 = 28764;
const REACTIVE_ARMOR: [(u32, [u32; 7]); 2] = [
    (
        OBSIDIAN_ARMOR,
        [0, 27536, 27533, 27538, 27534, 27535, 27540],
    ),
    (ADAPTIVE_WARDING, [0, 0, 28765, 28768, 28766, 28769, 28770]),
];

/// Whether `spell`'s procs are a script's answers alone: Holy Power's,
/// Totemic Power's and the reactive armors'.
pub fn answers(spell: u32) -> bool {
    HEALING_POWER.iter().any(|(id, _)| *id == spell)
        || REACTIVE_ARMOR.iter().any(|(id, _)| *id == spell)
}

/// A proc of `h` (on `owner`, `other` its other side) that a script says
/// more of, as Thistle Tea's `ClassScript`, `HealingPower` and
/// `ReactiveArmor` have it: Improved Blizzard's chill, Improved Mend Pet's
/// cleanse, Corrupted Healing, the druid set's Rejuvenation and Regrowth
/// gifts; Holy Power and Totemic Power for the healed one's class; and a
/// school's answer from Obsidian Armor or Adaptive Warding. Whether it
/// answered.
pub fn proc(
    map: &mut Map,
    owner: Guid,
    other: Guid,
    h: &Holder,
    spell: Option<u32>,
    outgoing: bool,
) -> bool {
    let data = map.realm.data.clone();
    let by = spell.and_then(|id| data.spells.get(id));
    if !outgoing {
        let Some(answers) = REACTIVE_ARMOR.iter().find(|(id, _)| *id == h.spell) else {
            return false;
        };
        let armored = h.spell != ADAPTIVE_WARDING
            || auras::auras(map, owner).is_some_and(|a| {
                a.holders.iter().any(|x| {
                    x.has(134)
                        && data
                            .spells
                            .get(x.spell)
                            .is_some_and(|t| family_flag(t, 3, 0x1000_0000))
                })
            });
        if let Some(by) = by
            && armored
            && let Some(&answer) = answers.1.get(by.school as usize)
            && answer != 0
        {
            cast_triggered(map, owner, answer, Some(owner));
            return true;
        }
        return false;
    }
    if !map.unit_view(other).is_some_and(|u| u.alive) {
        return false;
    }
    let victim = map.unit_fields(other).map(|f| {
        (
            f.get_byte(UNIT_FIELD_BYTES_0, 1),
            f.get_byte(UNIT_FIELD_BYTES_0, 3),
        )
    });
    if let Some((_, buffs)) = HEALING_POWER.iter().find(|(id, _)| *id == h.spell) {
        let buff = match victim.map(|v| v.0) {
            Some(2 | 5 | 7 | 11) => Some(buffs[0]),
            Some(8 | 9) => Some(buffs[1]),
            Some(3 | 4) => Some(buffs[2]),
            Some(1) => Some(buffs[3]),
            _ => None,
        };
        if let Some(buff) = buff {
            cast_triggered(map, owner, buff, Some(other));
            return true;
        }
        return false;
    }
    let mut answered = false;
    for x in h
        .effects
        .iter()
        .filter(|x| x.kind == auras::AURA_OVERRIDE_CLASS_SCRIPTS)
    {
        let trigger = match x.misc {
            836 | 988 | 989 if by.is_some_and(|b| b.spell_visual == 259) => BLIZZARD_CHILL
                .iter()
                .find(|(k, _)| *k == x.misc)
                .map(|(_, v)| *v),
            4086 | 4087
                if x.amount > 0
                    && (x.amount >= 100 || super::outcome::chance(map, x.amount as f32)) =>
            {
                Some(24406)
            }
            3656 if by
                .is_some_and(|b| b.effects.iter().any(|e| e.effect == super::EFFECT_HEAL)) =>
            {
                Some(23402)
            }
            4533 if by.is_some_and(|b| family_flag(b, FAMILY_DRUID, DRUID_REJUVENATION)) => {
                match victim.map(|v| v.1) {
                    Some(0) => Some(28722),
                    Some(1) => Some(28723),
                    Some(3) => Some(28724),
                    _ => None,
                }
            }
            4537 if by.is_some_and(|b| family_flag(b, FAMILY_DRUID, DRUID_REGROWTH)) => Some(28750),
            _ => None,
        };
        if let Some(trigger) = trigger {
            answered = true;
            cast_triggered(map, owner, trigger, Some(other));
        }
    }
    answered
}

/// `SMSG_FEIGN_DEATH_RESISTED`.
pub const SMSG_FEIGN_DEATH_RESISTED: u16 = 0x02B4;

/// Whether a creature fighting `g` (alive, not a player's, near enough to
/// notice) sees through its Feign Death `spell`: each rolls as against a
/// spell of `g`'s.
pub fn feign_death_resisted(map: &Map, g: Guid, spell: u32) -> bool {
    let Some(s) = map.realm.data.spells.get(spell) else {
        return false;
    };
    let Some(at) = map.position_of(g) else {
        return false;
    };
    let level = map.unit_view(g).map_or(1, |u| u.level);
    let mods = super::mods::for_spell(map, g, s);
    map.creatures
        .iter()
        .filter(|(_, c)| c.alive() && c.threat.iter().any(|(t, _)| *t == g))
        .filter(|(_, c)| {
            crate::game::map::pets::owner_of(map, c.guid).is_none_or(|o| !guid::is_player(o))
        })
        .filter(|(_, c)| {
            let reach =
                crate::game::map::ai::aggro_radius(20.0, c.fields.get_u32(UNIT_FIELD_LEVEL), level);
            combat::dist(c.position, at) <= reach
        })
        .any(|(_, c)| {
            !super::outcome::hits(
                map,
                super::outcome::spell_hit_chance(map, g, c.guid, s, &mods),
            )
        })
}

/// What periodic damage's `tick`th tick (from 1) adds to its points, as
/// Thistle Tea's `PeriodicDamage.ramp_offset` has it: Curse of Agony's
/// ticks go from half to one and a half times, Starshards' from two thirds
/// to four thirds.
pub fn ramp(data: &crate::game::data::GameData, spell: u32, tick: u32) -> f32 {
    let Some(s) = data.spells.get(spell) else {
        return 0.0;
    };
    if tick == 0 {
        return 0.0;
    }
    let agony = family_flag(s, FAMILY_WARLOCK, 0x400);
    let starshards = family_flag(s, FAMILY_PRIEST, 0x20_0000);
    let base = (s.effects[0].base_points + s.effects[0].base_dice) as f32;
    if agony {
        (((tick - 1) / 4) as f32 - 1.0) * base / 2.0
    } else if starshards {
        (((tick - 1) / 2) as f32 - 1.0) * base / 3.0
    } else {
        0.0
    }
}

/// Whether an aura's first pulse is at once.
pub fn immediate_periodic(spell: u32) -> bool {
    IMMEDIATE_PERIODIC.contains(&spell)
}

/// Whether refreshing an aura keeps its pulses' rhythm.
pub fn preserves_periodic_timer(s: &SpellTemplate) -> bool {
    s.stack_amount > 1
        || (s.spell_visual == 0 && s.spell_icon == 689)
        || IMMEDIATE_PERIODIC.contains(&s.id)
        || PRESERVED_PERIODIC.contains(&s.id)
}

/// What the scripts add to the spells' data: Shadowmeld's stealth from
/// its passive, as a spare effect.
pub fn patch(data: &mut SpellData) {
    let stealth = data
        .spells
        .get(&SHADOWMELD_PASSIVE)
        .and_then(|p| p.effects.iter().find(|e| e.effect != 0).copied());
    if let Some(stealth) = stealth
        && let Some(s) = data.spells.get_mut(&SHADOWMELD)
        && script(s, "spell_shadowmeld")
        && let Some(spare) = s.effects.iter().position(|e| e.effect == 0)
    {
        let mut e = stealth;
        e.target_a = s.effects[0].target_a;
        s.effects[spare] = e;
    }
}
