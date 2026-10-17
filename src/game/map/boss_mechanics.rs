//! Encounter mechanics Thistle Tea runs in its aura and spell engine, by
//! spell, not in its creature scripts (so its scripts' import doesn't
//! bring them):
//!
//! - Golemagg's Trust: each second Golemagg fights, it strengthens the
//!   Core Ragers within 30 yards (`Aura.GolemaggsTrust`);
//! - Separation Anxiety: every five seconds, an add of Garr's or
//!   Majordomo's too far from him grows stronger
//!   (`Aura.SeparationAnxiety`);
//! - Baron Geddon's Inferno: its pulses hurt more each second
//!   (`Spell.Scripts.periodic_trigger_points`);
//! - Lord Kazzak's Shadow Bolt Whirl: eight cones in turn, each a
//!   quarter-turn on from the last (`Spell.Scripts.cone` and
//!   `periodic_trigger_spell_id`);
//! - the dragons' auras: Azuregos's and the Emerald Dream dragons' auras
//!   strip stealth and stun those they marked; dying under their marks
//!   leaves the mark (or Emeriss's mushroom) behind (`Aura.Script`'s death
//!   casts, `Spell.Scripts`' dream dragon aura);
//! - Azuregos's Arcane Vacuum and Darkmaster Gandling's Shadow Portal:
//!   the caught are pulled away, and their threat is wiped.

use std::time::{Duration, Instant};

use ahash::AHashMap;
use rand::RngExt;
use wow_world_messages::Guid;

use crate::game::data::GameData;
use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras::{self, Aura, Holder};
use crate::game::map::combat::dist;

const GOLEMAGGS_TRUST: u32 = 20556;
const GOLEMAGGS_TRUST_BUFF: u32 = 20553;
const CORE_RAGER: u32 = 11672;
const TRUST_RADIUS: f32 = 30.0;

/// Separation Anxiety: the add's aura, and what it grows to.
const SEPARATION_ANXIETY: [(u32, u32); 2] = [(21094, 21095), (23487, 23492)];
const SEPARATION_EVERY: Duration = Duration::from_secs(5);

const INFERNO: u32 = 19695;
const INFERNO_PULSE: u32 = 19698;
const INFERNO_DAMAGE: [i32; 8] = [500, 500, 1000, 1000, 2000, 2000, 3000, 5000];

const SHADOW_BOLT_WHIRL: u32 = 24834;
const WHIRL: [u32; 8] = [24820, 24821, 24822, 24823, 24835, 24836, 24837, 24838];
const WHIRL_CONE: i32 = 120;

/// A dragon's aura, and the marks it stuns: (mark, stun).
const DREAM_DRAGON_AURAS: [u32; 2] = [23185, 25044];
const MARK_STUNS: [(u32, u32); 2] = [(25040, 25043), (23182, 23186)];
/// Auras whose holders, dying, cast a spell where they fell.
const DEATH_CASTS: [(u32, u32); 3] = [(23183, 23182), (24906, 24904), (25042, 25040)];

const ARCANE_VACUUM: u32 = 21147;
const SUMMON_PLAYER: u32 = 21150;
const SHADOW_PORTAL: u32 = 17950;
const SHADOW_PORTAL_ROOMS: [u32; 6] = [17863, 17939, 17943, 17944, 17946, 17948];

/// What the mechanics keep on a map: when each Separation Anxiety next
/// looks (by its holder and spell).
#[derive(Debug, Default)]
pub struct State {
    separation: AHashMap<(Guid, u32), Instant>,
    next: Option<Instant>,
}

/// The data the mechanics need that the DBC leaves out: the whirl's cones
/// are a third of a circle.
pub fn fixups(data: &mut GameData) {
    for s in WHIRL {
        data.spells.cones.insert(s, WHIRL_CONE);
    }
}

/// A periodic trigger's tick a mechanic takes over. Whether it did.
pub fn periodic_trigger(map: &mut Map, target: Guid, h: &Holder, a: &Aura) -> bool {
    // The count of this tick, from one.
    let n = a.ticks as usize + 1;
    match h.spell {
        GOLEMAGGS_TRUST => {
            golemaggs_trust(map, target);
            true
        }
        INFERNO => {
            let damage = INFERNO_DAMAGE[n.clamp(1, INFERNO_DAMAGE.len()) - 1];
            crate::game::map::spells::scripts::cast_with_points(
                map,
                h.caster,
                INFERNO_PULSE,
                h.caster,
                [Some(damage), None, None],
            );
            true
        }
        SHADOW_BOLT_WHIRL => {
            let step = n % WHIRL.len();
            whirl(map, h.caster, WHIRL[step], step);
            true
        }
        _ => false,
    }
}

/// Golemagg, alive and fighting, strengthens the Core Ragers near him.
fn golemaggs_trust(map: &mut Map, golemagg: Guid) {
    let Some(c) = map.creature(golemagg) else {
        return;
    };
    // In a fight: whether or not he has a victim.
    let fighting = c.combat.attacking
        || c.fields.has_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        );
    if !c.alive() || !fighting {
        return;
    }
    let at = c.position;
    let ragers: Vec<Guid> = map
        .grid
        .around(at.x, at.y, TRUST_RADIUS)
        .filter(|g| guid::is_unit(*g))
        .filter(|g| {
            map.creature(*g).is_some_and(|r| {
                r.entry == CORE_RAGER && r.alive() && dist(r.position, at) <= TRUST_RADIUS
            })
        })
        .collect();
    let Some(s) = single_target(map, GOLEMAGGS_TRUST_BUFF) else {
        return;
    };
    for r in ragers {
        crate::game::map::spells::scripts::cast_template_with_points(
            map,
            golemagg,
            s.clone(),
            r,
            [None; 3],
        );
    }
}

/// A spell's template aimed at one unit, whatever area its data names.
fn single_target(map: &Map, spell: u32) -> Option<SpellTemplate> {
    let mut s = map.realm.data.spells.get(spell)?.clone();
    for e in s.effects.iter_mut().filter(|e| e.effect != 0) {
        e.target_a = 25;
        e.target_b = 0;
        e.radius_index = 0;
    }
    Some(s)
}

/// One cone of the whirl: cast with the caster turned `step` quarter
/// turns on, then turned back.
fn whirl(map: &mut Map, caster: Guid, spell: u32, step: usize) {
    let Some(facing) = map.creature(caster).map(|c| c.orientation) else {
        return;
    };
    let turned = facing + step as f32 * std::f32::consts::FRAC_PI_4;
    if let Some(c) = map.creature_mut(caster) {
        c.orientation = turned;
    }
    crate::game::map::spells::cast_triggered(map, caster, spell, Some(caster));
    if let Some(c) = map.creature_mut(caster) {
        c.orientation = facing;
    }
}

/// A dummy or script effect a mechanic owns. Whether it was one.
pub fn dummy(map: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate) -> bool {
    match s.id {
        id if DREAM_DRAGON_AURAS.contains(&id) => {
            // Stealth doesn't hide from it; its marked are stunned.
            auras::remove_where(map, target, |h| h.has(auras::AURA_MOD_STEALTH));
            for (mark, stun) in MARK_STUNS {
                if auras::auras(map, target).is_some_and(|a| a.has_spell(mark)) {
                    crate::game::map::spells::cast_triggered(map, target, stun, Some(target));
                }
            }
            true
        }
        ARCANE_VACUUM => {
            threat_wipe_cast(map, caster, target, SUMMON_PLAYER);
            true
        }
        SHADOW_PORTAL => {
            let room = SHADOW_PORTAL_ROOMS[rand::rng().random_range(0..SHADOW_PORTAL_ROOMS.len())];
            threat_wipe_cast(map, caster, target, room);
            true
        }
        _ => false,
    }
}

/// The caster forgets `target`, and casts `spell` on it.
fn threat_wipe_cast(map: &mut Map, caster: Guid, target: Guid, spell: u32) {
    if let Some(c) = map.creature_mut(caster) {
        for (t, v) in c.threat.iter_mut() {
            if *t == target {
                *v = 0.0;
            }
        }
    }
    crate::game::map::spells::cast_triggered(map, caster, spell, Some(target));
}

/// The auras a unit dying holds that cast something where it fell.
pub fn death_casts(map: &Map, g: Guid) -> Vec<u32> {
    auras::auras(map, g)
        .map(|a| {
            DEATH_CASTS
                .iter()
                .filter(|(aura, _)| a.has_spell(*aura))
                .map(|(_, cast)| *cast)
                .collect()
        })
        .unwrap_or_default()
}

/// What a unit that died under them casts where it fell: a mark goes
/// straight on the fallen; anything else is cast at its feet.
pub fn cast_on_death(map: &mut Map, g: Guid, casts: Vec<u32>) {
    let data = map.realm.data.clone();
    for spell in casts {
        let Some(s) = data.spells.get(spell) else {
            continue;
        };
        let auras: Vec<(usize, i32)> = s
            .effects
            .iter()
            .enumerate()
            .filter(|(_, e)| e.effect == crate::game::map::spells::EFFECT_APPLY_AURA)
            .map(|(i, e)| (i, e.base_points + 1))
            .collect();
        if !auras.is_empty() && auras.len() == s.effects.iter().filter(|e| e.effect != 0).count() {
            let _ = auras::apply_spell(map, g, g, s, &auras, Default::default());
        } else {
            crate::game::map::spells::cast_triggered(map, g, spell, Some(g));
        }
    }
}

/// The mechanics that look about on their own, about once a second.
pub fn update(map: &mut Map) {
    let now = map.now;
    if map.bosses.next.is_some_and(|t| t > now) {
        return;
    }
    map.bosses.next = Some(now + Duration::from_secs(1));
    separation_anxiety(map);
}

/// Every five seconds, an add too far from the boss whose aura it bears
/// grows stronger.
fn separation_anxiety(map: &mut Map) {
    let now = map.now;
    let data = map.realm.data.clone();
    // Only a creature with auras can bear it.
    let bearers = map.with_auras_where(|c| c.alive() && !c.auras.is_empty(), |k, _| k);
    let holders: Vec<(Guid, u32, Guid, u32)> = bearers
        .into_iter()
        .filter_map(|k| map.creatures.get(k))
        .flat_map(|c| {
            c.auras.holders.iter().filter_map(move |h| {
                SEPARATION_ANXIETY
                    .iter()
                    .find(|(s, _)| *s == h.spell)
                    .map(|(s, buff)| (c.guid, *s, h.caster, *buff))
            })
        })
        .collect();
    map.bosses
        .separation
        .retain(|(g, s), _| holders.iter().any(|(hg, hs, _, _)| hg == g && hs == s));
    for (g, spell, boss, buff) in holders {
        let due = map
            .bosses
            .separation
            .entry((g, spell))
            .or_insert(now + SEPARATION_EVERY);
        if *due > now {
            continue;
        }
        *due = now + SEPARATION_EVERY;
        let radius = data
            .spells
            .get(spell)
            .map_or(0.0, |s| data.spells.radius(s.effects[0].radius_index));
        let (Some(me), Some(him)) = (map.unit_view(g), map.unit_view(boss)) else {
            continue;
        };
        if !him.alive || radius <= 0.0 {
            continue;
        }
        let bounding = |x: Guid| {
            map.unit_fields(x)
                .map_or(0.0, |f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS).max(0.0))
        };
        if dist(me.position, him.position) >= radius + bounding(g) + bounding(boss) {
            crate::game::map::spells::cast_triggered(map, g, buff, Some(g));
        }
    }
}
