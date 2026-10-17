//! Creatures in a fight, beyond the swing, as Thistle Tea runs them:
//!
//! - the leash ([`should_evade`]): a creature fights on while near where
//!   the fight began, or while its victim is, or while blows still come;
//!   beyond its template's hard leash it always goes home;
//! - a victim it can't reach for 24 seconds makes it evade;
//! - fleeing: for help to the nearest of its kind, or in fear; fear and
//!   confusion move it about;
//! - regeneration: a third of its health and mana each five seconds out of
//!   a fight, and in one while polymorphed;
//! - its template's mechanic and school immunities.

use std::time::{Duration, Instant};

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::combat::dist;
use crate::game::map::creature::Motion;

/// The leash's reach around where the fight began.
pub const THREAT_RADIUS: f32 = 50.0;
/// Without a hostile act this long, a creature beyond its threat radius
/// goes home.
pub const LEASH_TIMEOUT: Duration = Duration::from_secs(12);
/// A victim it can't reach this long, and it goes home.
pub const UNREACHABLE_TIMEOUT: Duration = Duration::from_secs(24);
/// How long a creature flees, and how far it looks for help.
pub const FLEE_FOR: Duration = Duration::from_secs(7);
pub const FLEE_ASSISTANCE_RADIUS: f32 = 30.0;
/// How long it waits for help once it reaches it.
pub const ASSISTANCE_WAIT: Duration = Duration::from_millis(1500);
/// Creatures regenerate every five seconds.
pub const REGEN_EVERY: Duration = Duration::from_secs(5);

/// `flags_extra`: no leash at all; no evading an unreachable victim.
const EXTRA_NO_LEASH: u32 = 0x1;
const EXTRA_NO_UNREACHABLE_EVADE: u32 = 0x8;
/// `static_flags1`: it doesn't regenerate.
const STATIC_NO_AUTOMATIC_REGEN: u32 = 0x400;
/// The unit flags of a creature under control: stunned, confused, fleeing.
const CONTROLLED: u32 = 0x0004_0000 | 0x0040_0000 | 0x0080_0000;

/// What a creature's fight remembers.
#[derive(Debug, Clone, Default)]
pub struct Engagement {
    /// Where the fight began.
    pub origin: Option<Vector3d>,
    /// The last hostile act on or by it.
    pub last_hostile: Option<Instant>,
    /// Since when its victim has been out of reach.
    pub unreachable_since: Option<Instant>,
    pub flee: Option<Flee>,
    pub regen_at: Option<Instant>,
    /// When fear or confusion next moves it.
    pub next_move_at: Option<Instant>,
}

/// A creature running: from `from`, to `helper` if it found one.
#[derive(Debug, Clone, Copy)]
pub struct Flee {
    pub from: Guid,
    pub until: Instant,
    pub helper: Option<Guid>,
    /// When it reached its helper and called it.
    pub called_at: Option<Instant>,
}

fn template(map: &Map, g: Guid) -> Option<&crate::game::data::CreatureTemplate> {
    map.creature(g)
        .and_then(|c| map.realm.data.creature_template(c.entry))
}

/// A fight begins: it remembers where.
pub fn engaged(map: &mut Map, g: Guid) {
    let now = map.now;
    if let Some(c) = map.creature_mut(g) {
        c.engagement.origin = Some(c.position);
        c.engagement.last_hostile = Some(now);
        c.engagement.unreachable_since = None;
    }
}

/// A blow on or by the creature extends its leash.
pub fn hostile_act(map: &mut Map, g: Guid) {
    let now = map.now;
    if let Some(c) = map.creature_mut(g) {
        c.engagement.last_hostile = Some(now);
    }
}

/// The fight is over.
pub fn disengaged(map: &mut Map, g: Guid) {
    if let Some(c) = map.creature_mut(g) {
        c.engagement.origin = None;
        c.engagement.unreachable_since = None;
        c.engagement.flee = None;
    }
}

/// Whether it could reach its victim on its last path.
pub fn reach(map: &mut Map, g: Guid, reachable: bool) {
    let now = map.now;
    if let Some(c) = map.creature_mut(g) {
        if reachable {
            c.engagement.unreachable_since = None;
        } else if c.engagement.unreachable_since.is_none() {
            c.engagement.unreachable_since = Some(now);
        }
    }
}

/// The hard leash: the template's, if no shorter than its detection range.
fn hard_leash(map: &Map, g: Guid) -> Option<f32> {
    template(map, g)
        .map(|t| t.leash_range)
        .filter(|r| *r > 0.0)
        .filter(|r| template(map, g).is_none_or(|t| *r >= t.detection_range))
}

/// Thistle Tea's `CombatLeash.should_evade?` and `UnreachableTarget`:
/// whether a creature fighting `victim` should give up and go home.
pub fn should_evade(map: &Map, g: Guid, victim: Guid) -> bool {
    let Some(c) = map.creature(g) else {
        return true;
    };
    let now = map.now;
    let extra = template(map, g).map_or(0, |t| t.flags_extra);
    let origin = c.engagement.origin.unwrap_or(c.home);
    let d = dist(c.position, origin);
    if hard_leash(map, g).is_some_and(|r| d > r) {
        return true;
    }
    // A victim out of reach too long.
    let controlled = c.fields.get_u32(UNIT_FIELD_FLAGS) & CONTROLLED != 0
        || c.auras.has(AURA_MOD_ROOT)
        || c.owner.is_some();
    if extra & EXTRA_NO_UNREACHABLE_EVADE == 0
        && !controlled
        && c.engagement
            .unreachable_since
            .is_some_and(|t| now.duration_since(t) > UNREACHABLE_TIMEOUT)
    {
        return true;
    }
    // In a dungeon, or without a leash, it fights on.
    if map.key.instance != 0 || extra & EXTRA_NO_LEASH != 0 || controlled {
        return false;
    }
    if d <= THREAT_RADIUS {
        return false;
    }
    if map
        .position_of(victim)
        .is_some_and(|p| dist(p, origin) <= THREAT_RADIUS)
    {
        return false;
    }
    c.engagement
        .last_hostile
        .is_none_or(|t| now.duration_since(t) > LEASH_TIMEOUT)
}

/// The template's mechanic immunity: `mechanic` (1 to 31) is in its mask.
pub fn mechanic_immune(map: &Map, g: Guid, mechanic: u32) -> bool {
    (1..=31).contains(&mechanic)
        && template(map, g).is_some_and(|t| t.mechanic_immune_mask & (1 << (mechanic - 1)) != 0)
}

/// The template's school immunity, for a school mask.
pub fn school_immune(map: &Map, g: Guid, school: u32) -> bool {
    school != 0 && template(map, g).is_some_and(|t| t.school_immune_mask & school != 0)
}

/// Whether a creature's template stops `spell` from `caster`: its mechanic
/// or an effect's, or, if harmful, its school (Thistle Tea's
/// `CreatureImmunity.spell?`).
pub fn spell_immune(map: &Map, target: Guid, caster: Guid, spell: &SpellTemplate) -> bool {
    if target == caster
        || !guid::is_unit(target)
        || spell.attr(0, crate::game::data::spells::ATTR_NO_IMMUNITIES)
    {
        return false;
    }
    mechanic_immune(map, target, spell.mechanic)
        || spell
            .effects
            .iter()
            .any(|e| e.effect != 0 && mechanic_immune(map, target, e.mechanic))
        || (!auras::positive(spell) && school_immune(map, target, spell.school_mask()))
}

/// Whether fleeing is barred: an aura that prevents it, feigned death,
/// confusion, or a master's control.
fn flee_prevented(map: &Map, g: Guid) -> bool {
    map.creature(g).is_none_or(|c| {
        c.auras.has(AURA_PREVENTS_FLEEING)
            || c.auras.has(AURA_FEIGN_DEATH)
            || c.auras.has(AURA_MOD_CONFUSE)
            || c.owner.is_some()
            || !c.alive()
    })
}

/// vmangos's `DoFleeToGetAssistance` (EventAI's flee for assistance): to
/// the nearest creature of its faction not fighting, which joins in on
/// arrival; with none near, it runs in fear for seven seconds.
pub fn flee_for_assistance(map: &mut Map, g: Guid) {
    let now = map.now;
    if flee_prevented(map, g) {
        return;
    }
    let Some(c) = map.creature(g) else {
        return;
    };
    let from = c.combat.victim;
    if from.is_zero() {
        return;
    }
    let (pos, faction) = (c.position, c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE));
    let helper = map
        .grid
        .around(pos.x, pos.y, FLEE_ASSISTANCE_RADIUS)
        .filter(|h| guid::is_unit(*h) && *h != g)
        .filter_map(|h| {
            let hc = map.creature(h)?;
            (hc.alive()
                && !hc.combat.attacking
                && hc.owner.is_none()
                && hc.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE) == faction)
                .then(|| (h, dist(hc.position, pos)))
        })
        .filter(|(_, d)| *d <= FLEE_ASSISTANCE_RADIUS)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(h, _)| h);
    emote(map, g, "%s attempts to run away in fear!");
    crate::game::map::spells::interrupt(map, g);
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.engagement.flee = Some(Flee {
        from,
        until: now + FLEE_FOR,
        helper,
        called_at: None,
    });
    c.engagement.next_move_at = Some(now);
    c.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_FLEEING, true);
}

/// A creature's text emote, its name in place of `%s`.
fn emote(map: &Map, g: Guid, text: &str) {
    let Some(c) = map.creature(g) else {
        return;
    };
    let name = map
        .realm
        .data
        .creature_template(c.entry)
        .map(|t| t.name.clone())
        .unwrap_or_default();
    if let Some(frame) = crate::game::conn::encode(&SMSG_MESSAGECHAT {
        chat_type: SMSG_MESSAGECHAT_ChatType::MonsterEmote {
            monster: g,
            monster_name: name,
        },
        language: Language::Universal,
        message: text.to_owned(),
        tag: PlayerChatTag::None,
    }) {
        map.send_to_observers(g, &frame, false);
    }
}

/// Fear, confusion and fleeing move a creature themselves: the fight's own
/// step waits. Whether it did.
pub fn controlled_step(map: &mut Map, g: Guid) -> bool {
    let now = map.now;
    let Some(c) = map.creature(g) else {
        return false;
    };
    let feared = c.auras.has(AURA_MOD_FEAR);
    let confused = c.auras.has(AURA_MOD_CONFUSE);
    let flee = c.engagement.flee;
    if let Some(f) = flee
        && (f.until <= now || feared || confused)
    {
        end_flee(map, g);
        return feared || confused;
    }
    if !feared && !confused && flee.is_none() {
        return false;
    }
    // Stunned or rooted: it stands, held.
    if c.auras.has(AURA_MOD_STUN) || c.auras.has(AURA_MOD_ROOT) {
        return true;
    }
    if matches!(&c.motion, Motion::Moving(w) if !w.done(now)) {
        return true;
    }
    if c.engagement.next_move_at.is_some_and(|t| t > now) {
        return true;
    }
    let pos = c.position;
    let mut rng = rand::rng();
    let (goal, run, pause) = if let Some(f) = flee
        && let Some(helper) = f.helper
    {
        match map.position_of(helper) {
            Some(at) if dist(at, pos) > 3.0 => (at, true, Duration::from_millis(500)),
            Some(_) => {
                // Arrived: help is called, and given a moment.
                if f.called_at.is_none() {
                    if !f.from.is_zero() {
                        crate::game::map::ai::attacked(map, helper, f.from);
                    }
                    if let Some(c) = map.creature_mut(g)
                        && let Some(fl) = c.engagement.flee.as_mut()
                    {
                        fl.called_at = Some(now);
                        fl.until = fl.until.min(now + ASSISTANCE_WAIT);
                    }
                }
                return true;
            }
            None => {
                end_flee(map, g);
                return false;
            }
        }
    } else if confused {
        // Confusion: a stagger about where it stands.
        let a = rng.random_range(0.0f32..std::f32::consts::TAU);
        let r = rng.random_range(1.0f32..4.0);
        (
            Vector3d {
                x: pos.x + r * a.cos(),
                y: pos.y + r * a.sin(),
                z: pos.z,
            },
            false,
            Duration::from_millis(rng.random_range(500..1500)),
        )
    } else {
        // Fear: away from whoever it fears, this way and that.
        let source = flee.map(|f| f.from).or_else(|| {
            c.auras
                .holders
                .iter()
                .find(|h| h.has(AURA_MOD_FEAR))
                .map(|h| h.caster)
        });
        let away = source
            .and_then(|s| map.position_of(s))
            .map(|s| (pos.y - s.y).atan2(pos.x - s.x))
            .unwrap_or_else(|| rng.random_range(0.0f32..std::f32::consts::TAU));
        let a = away + rng.random_range(-0.8f32..0.8);
        let r = rng.random_range(6.0f32..10.0);
        (
            Vector3d {
                x: pos.x + r * a.cos(),
                y: pos.y + r * a.sin(),
                z: pos.z,
            },
            true,
            Duration::from_millis(200),
        )
    };
    let path = match map.nav.as_mut() {
        Some(nav) => nav.path(pos, goal).unwrap_or_else(|| vec![pos, goal]),
        None => vec![pos, goal],
    };
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let Some(c) = map.creature_mut(g) else {
        return true;
    };
    c.engagement.next_move_at = Some(now + pause);
    if let Some(frame) = c.walk(path, run, now, spline_id) {
        map.send_to_observers(g, &frame, false);
    }
    true
}

fn end_flee(map: &mut Map, g: Guid) {
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    if c.engagement.flee.take().is_some() && !c.auras.has(AURA_MOD_FEAR) {
        c.fields
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_FLEEING, false);
    }
    c.chase_target_at = None;
}

/// `static_flags1`: it calls the guards; `flags_extra`: it is a guard.
const STATIC_CALLS_GUARDS: u32 = 0x0800_0000;
const EXTRA_GUARD: u32 = 0x400;
/// How far a call for the guards carries.
pub const GUARD_CALL_RADIUS: f32 = 50.0;

/// Thistle Tea's `GuardCall`: a civilian that calls the guards, entering
/// a fight with a player, rouses the guards near it against them.
pub fn call_guards(map: &mut Map, caller: Guid, enemy: Guid) {
    if !guid::is_player(enemy)
        || !template(map, caller).is_some_and(|t| t.static_flags1 & STATIC_CALLS_GUARDS != 0)
    {
        return;
    }
    let Some(at) = map.position_of(caller) else {
        return;
    };
    let guards: Vec<Guid> = map
        .grid
        .around(at.x, at.y, GUARD_CALL_RADIUS)
        .filter(|g| guid::is_unit(*g) && *g != caller)
        .filter(|g| {
            map.creature(*g).is_some_and(|c| {
                c.alive()
                    && !c.combat.attacking
                    && dist(c.position, at) <= GUARD_CALL_RADIUS
                    && map
                        .realm
                        .data
                        .creature_template(c.entry)
                        .is_some_and(|t| t.flags_extra & EXTRA_GUARD != 0)
            })
        })
        .filter(|g| crate::game::map::combat::can_attack(map, *g, enemy))
        .collect();
    for g in guards {
        crate::game::map::ai::attacked(map, g, enemy);
    }
}

/// Whether a creature is polymorphed: a mage's confusion that transforms.
pub fn polymorphed(map: &Map, g: Guid) -> bool {
    let data = &map.realm.data;
    crate::game::map::auras::auras(map, g).is_some_and(|a| {
        a.holders.iter().any(|h| {
            h.has(AURA_MOD_CONFUSE)
                && data
                    .spells
                    .get(h.spell)
                    .is_some_and(|s| s.family_name == 3 && s.prevention_type == 1)
        })
    })
}

/// Creatures regenerate: a third of health and mana each tick out of a
/// fight, and health in one while polymorphed.
pub fn regenerate(map: &mut Map) {
    let now = map.now;
    let due: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| c.alive() && !c.evading && c.owner.is_none_or(|o| !guid::is_player(o)))
        .filter(|(_, c)| c.engagement.regen_at.is_none_or(|t| t <= now))
        .filter(|(_, c)| {
            c.fields.get_u32(UNIT_FIELD_HEALTH) < c.fields.get_u32(UNIT_FIELD_MAXHEALTH)
                || c.fields.get_u32(UNIT_FIELD_POWER1) < c.fields.get_u32(UNIT_FIELD_MAXPOWER1)
        })
        .map(|(_, c)| c.guid)
        .collect();
    for g in due {
        let regenerates =
            template(map, g).is_none_or(|t| t.static_flags1 & STATIC_NO_AUTOMATIC_REGEN == 0);
        let poly = polymorphed(map, g);
        let Some(c) = map.creature_mut(g) else {
            continue;
        };
        if c.engagement.regen_at.is_none() {
            c.engagement.regen_at = Some(now + REGEN_EVERY);
            continue;
        }
        c.engagement.regen_at = Some(now + REGEN_EVERY);
        if !regenerates {
            continue;
        }
        let fighting = c.combat.attacking || !c.combat.attackers.is_empty();
        let f = &mut c.fields;
        if !fighting || poly {
            let (health, max) = (
                f.get_u32(UNIT_FIELD_HEALTH),
                f.get_u32(UNIT_FIELD_MAXHEALTH),
            );
            f.set_u32(UNIT_FIELD_HEALTH, (health + (max / 3).max(1)).min(max));
        }
        if !fighting {
            let (mana, max) = (
                f.get_u32(UNIT_FIELD_POWER1),
                f.get_u32(UNIT_FIELD_MAXPOWER1),
            );
            if max > 0 {
                f.set_u32(UNIT_FIELD_POWER1, (mana + (max / 3).max(1)).min(max));
            }
        }
    }
}

/// The creatures' flight timers, and their regeneration.
pub fn update(map: &mut Map) {
    regenerate(map);
    let now = map.now;
    let over: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| c.engagement.flee.is_some_and(|f| f.until <= now))
        .map(|(_, c)| c.guid)
        .collect();
    for g in over {
        end_flee(map, g);
    }
}
