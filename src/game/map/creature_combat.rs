//! Creatures in a fight, beyond the swing, as Thistle Tea runs them:
//!
//! - the leash ([`should_evade`]): a creature fights on while near where
//!   the fight began, or while its victim is, or while blows still come;
//!   beyond its template's hard leash it always goes home;
//! - a victim it can't reach for 24 seconds makes it evade;
//! - fleeing: for help to the nearest of its kind, or in fear; fear and
//!   confusion move it about;
//! - regeneration: a third of its health and mana each five seconds out of
//!   a fight, health in one while polymorphed, and mana by its spirit;
//! - a dungeon's boss fighting the whole dungeon at once;
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
/// `static_flags2`: it fights everyone in its dungeon.
const STATIC2_FORCE_RAID_COMBAT: u32 = 0x2;
/// `flags_extra`: it keeps no threat list.
const EXTRA_NO_THREAT_LIST: u32 = 0x800;
/// How often one fighting its whole dungeon draws in who isn't fighting.
pub const ZONE_PULSE: Duration = Duration::from_secs(3);
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
    /// When it last spent mana, for the five-second rule.
    pub mana_spent_at: Option<Instant>,
    /// When it next draws its dungeon into the fight.
    pub zone_pulse_at: Option<Instant>,
    /// Its call for the guards, answered.
    pub guard_call: Option<GuardCall>,
    /// When a civilian next looks about for enemies.
    pub watch_at: Option<Instant>,
    /// When a frightened critter's escape ends.
    pub escape_at: Option<Instant>,
    /// Damage it took from players' side, and from NPCs', this fight.
    pub damage_by_players: u64,
    pub damage_by_npcs: u64,
    /// Its calls for help on their way.
    pub assist: Vec<crate::game::map::assistance::Assist>,
    /// When it next calls its friends, fighting away from its spawn.
    pub pulse_at: Option<Instant>,
}

/// A civilian's call for the guards, answered: it makes one at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardCall {
    /// A post's guard came: it is quiet until that guard is gone.
    Summoned(Guid),
    /// A guard near was roused (or none was): quiet until it respawns.
    Held,
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
        c.engagement.zone_pulse_at = None;
        c.engagement.damage_by_players = 0;
        c.engagement.damage_by_npcs = 0;
        c.engagement.assist.clear();
        c.engagement.pulse_at = None;
    }
}

/// `static_flags1`: its corpse is the raid's, whoever fought it.
const STATIC_CORPSE_RAID: u32 = 0x1000;

/// Damage `amount` to creature `g` from `by` (Thistle Tea's
/// `DamageOrigin.record`): a player's side's, or its own, counts as the
/// players'; anything else as the NPCs'.
pub fn damaged(map: &mut Map, g: Guid, by: Guid, amount: u32) {
    if amount == 0 || !guid::is_unit(g) {
        return;
    }
    let players = by == g || guid::is_player(crate::game::map::pets::master(map, by));
    if let Some(c) = map.creature_mut(g) {
        if players {
            c.engagement.damage_by_players += u64::from(amount);
        } else {
            c.engagement.damage_by_npcs += u64::from(amount);
        }
    }
}

/// Whether a creature's corpse is lootable, and its kill rewarded: players
/// dealt more than 35% of its damage, or its corpse is the raid's.
pub fn loot_allowed(map: &Map, g: Guid) -> bool {
    let Some(c) = map.creature(g) else {
        return true;
    };
    let (p, n) = (c.engagement.damage_by_players, c.engagement.damage_by_npcs);
    template(map, g).is_some_and(|t| t.static_flags1 & STATIC_CORPSE_RAID != 0)
        || p + n == 0
        || 65 * p > 35 * n
}

/// The share of a kill's XP its damage earns: the players' part of it.
pub fn xp_share(map: &Map, g: Guid) -> f32 {
    let Some(c) = map.creature(g) else {
        return 1.0;
    };
    let (p, n) = (c.engagement.damage_by_players, c.engagement.damage_by_npcs);
    if template(map, g).is_some_and(|t| t.static_flags1 & STATIC_CORPSE_RAID != 0) || p + n == 0 {
        1.0
    } else if loot_allowed(map, g) {
        p as f32 / (p + n) as f32
    } else {
        0.0
    }
}

/// Whether `g` fights its whole dungeon (Thistle Tea's `ZoneCombat`): its
/// template says so, it keeps a threat list, and no player owns it.
fn zone_fighter(map: &Map, g: Guid) -> bool {
    let (Some(c), Some(t)) = (map.creature(g), template(map, g)) else {
        return false;
    };
    t.static_flags2 & STATIC2_FORCE_RAID_COMBAT != 0
        && t.flags_extra & EXTRA_NO_THREAT_LIST == 0
        && c.alive()
        && !c.owner.is_some_and(guid::is_player)
        && matches!(
            crate::game::instances::instance_type(&map.realm, map.key.map),
            1 | 2
        )
}

/// A fight begins for `g` against `source`: one that fights its whole
/// dungeon draws in everyone there, and their pets, if a player's side
/// began it.
pub fn entered(map: &mut Map, g: Guid, source: Guid) {
    if !zone_fighter(map, g) {
        return;
    }
    let now = map.now;
    if let Some(c) = map.creature_mut(g) {
        c.engagement.zone_pulse_at = Some(now + ZONE_PULSE);
    }
    if guid::is_player(crate::game::map::pets::master(map, source)) {
        zone_pulse(map, g, true);
    }
}

/// Everyone in the dungeon, and their pets, fight `g`: at first all of
/// them, afterwards those not already fighting.
fn zone_pulse(map: &mut Map, g: Guid, initial: bool) {
    let players: Vec<Guid> = map.players.keys().copied().collect();
    for p in players {
        let fighting = map
            .combat(p)
            .is_some_and(|c| c.attacking || !c.attackers.is_empty());
        if fighting && !initial {
            continue;
        }
        let pet = map.players.get(&p).and_then(|x| x.owned.pet);
        for t in std::iter::once(p).chain(pet) {
            if map.unit_view(t).is_some_and(|u| u.alive)
                && crate::game::map::combat::can_attack(map, g, t)
            {
                crate::game::map::combat::engage(map, t, g);
            }
        }
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

/// A creature flees (script command 47): seeking help (`seek`), to the
/// nearest creature of its faction within 30 yards in sight that answers
/// calls, which it calls on arrival; else, or with none, it runs in fear
/// for seven seconds (Thistle Tea's `Flee.start`).
pub fn flee_for_assistance(map: &mut Map, g: Guid, seek: bool) {
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
    let helper = if seek {
        crate::game::map::assistance::nearest_helper(map, g)
    } else {
        None
    };
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
                        crate::game::map::assistance::drawn_in(map, g, f.from);
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
        // Fear: Thistle Tea's run at a random angle, longer the nearer
        // the one it fears.
        let source = flee.map(|f| f.from).or_else(|| {
            c.auras
                .holders
                .iter()
                .find(|h| h.has(AURA_MOD_FEAR))
                .map(|h| h.caster)
        });
        let distance = source
            .and_then(|s| map.position_of(s))
            .map_or(0.0, |s| dist(s, pos));
        (
            crate::game::map::player_motion::fear_destination(pos, distance, &mut rng),
            true,
            Duration::from_millis(rng.random_range(800..1500)),
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
    let frame = c.walk(path, run, now, spline_id);
    // The pause comes after the run.
    let run_for = match &c.motion {
        Motion::Moving(w) => w.duration,
        Motion::Idle { .. } => Duration::ZERO,
    };
    c.engagement.next_move_at = Some(now + run_for + pause);
    if let Some(frame) = frame {
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
/// How often a civilian looks about, and how far above or below it sees.
const WATCH_EVERY: Duration = Duration::from_secs(1);
const WATCH_HEIGHT: f32 = 3.0;

/// Whether a civilian may call the guards now: it calls them, it's its
/// own, and it has no call answered (or its guard has gone).
fn guard_call_ready(map: &Map, caller: Guid) -> bool {
    let Some(c) = map.creature(caller) else {
        return false;
    };
    let ready = match c.engagement.guard_call {
        None => true,
        Some(GuardCall::Summoned(g)) => map.creature(g).is_none_or(|g| !g.alive()),
        Some(GuardCall::Held) => false,
    };
    ready
        && c.alive()
        && c.owner.is_none()
        && template(map, caller).is_some_and(|t| t.static_flags1 & STATIC_CALLS_GUARDS != 0)
}

/// Thistle Tea's `GuardCall`: a civilian that calls the guards, entering
/// a fight with a player or seeing one of the enemy, shouts and has its
/// town's post send a guard of the other side five yards off to fight them
/// for two minutes; with no post for its area it rouses the nearest guard.
/// A post spent or cooling down refuses, and it may call again.
pub fn call_guards(map: &mut Map, caller: Guid, enemy: Guid) {
    if !guid::is_player(enemy) || !guard_call_ready(map, caller) {
        return;
    }
    let Some(c) = map.creature(caller) else {
        return;
    };
    let (at, facing, faction) = (
        c.position,
        c.orientation,
        c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
    );
    let area = map
        .nav
        .as_mut()
        .and_then(|n| n.zone_and_area(at))
        .map(|(_, area)| area);
    if let Some(area) = area.filter(|a| crate::game::map::guard_posts::posted(*a)) {
        let key = (map.key.map, map.key.instance, area);
        if !crate::game::map::guard_posts::take(key, map.now) {
            return;
        }
        if let Some(text) = crate::game::map::guard_posts::text(area, faction) {
            crate::game::map::scripts::talk(map, caller, text, enemy, None);
        }
        let enemy_alliance = map
            .players
            .get(&enemy)
            .is_some_and(|p| p.team == crate::game::registry::Team::Alliance);
        let guard = crate::game::map::guard_posts::guard(area, !enemy_alliance)
            .and_then(|entry| summon_guard(map, caller, entry, at, facing, enemy));
        answered(
            map,
            caller,
            guard.map_or(GuardCall::Held, GuardCall::Summoned),
        );
        return;
    }
    rouse_nearest_guard(map, caller, at, enemy);
    answered(map, caller, GuardCall::Held);
}

fn answered(map: &mut Map, caller: Guid, answer: GuardCall) {
    if let Some(c) = map.creature_mut(caller) {
        c.engagement.guard_call = Some(answer);
    }
}

/// Civilians that call the guards look about for enemy players: within
/// their detection range, level with them, in sight and seen; in a fight,
/// only for whom they fight.
fn watch_for_enemies(map: &mut Map) {
    let now = map.now;
    let data = map.realm.data.clone();
    let watching: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| {
            c.alive()
                && !c.evading
                && c.pet.is_none()
                && !c.observers.is_empty()
                && (c.combat.attacking || !c.hostile_to_players)
                && c.engagement.watch_at.is_none_or(|t| t <= now)
                && data
                    .creature_template(c.entry)
                    .is_some_and(|t| t.static_flags1 & STATIC_CALLS_GUARDS != 0)
        })
        .map(|(_, c)| c.guid)
        .collect();
    for g in watching {
        if let Some(c) = map.creature_mut(g) {
            c.engagement.watch_at = Some(now + WATCH_EVERY);
        }
        if !guard_call_ready(map, g) {
            continue;
        }
        if let Some(enemy) = sighted_enemy(map, g) {
            call_guards(map, g, enemy);
        }
    }
}

/// The nearest enemy player a civilian sees.
fn sighted_enemy(map: &mut Map, g: Guid) -> Option<Guid> {
    let c = map.creature(g)?;
    let (at, fighting, victim) = (c.position, c.combat.attacking, c.combat.victim);
    let range = template(map, g).map_or(20.0, |t| t.detection_range);
    let enemy = map
        .players_near(at.x, at.y, range)
        .into_iter()
        .filter(|p| !fighting || *p == victim)
        .filter_map(|p| {
            let pl = map.players.get(&p)?;
            let pos = pl.position();
            (pl.death == crate::game::map::death::Death::Alive
                && !pl
                    .fields
                    .has_flag(PLAYER_FLAGS, crate::game::map::player::PLAYER_FLAGS_GM)
                && !pl.awaiting_worldport
                && (pos.z - at.z).abs() <= WATCH_HEIGHT
                && dist(pos, at) <= range
                && crate::game::map::spells::stealth::sees(map, g, p)
                && crate::game::map::combat::hostile(map, g, p)
                && crate::game::map::combat::can_attack(map, g, p))
            .then(|| (p, pos, dist(pos, at)))
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))?;
    let in_sight = match map.nav.as_mut() {
        Some(nav) => {
            let eye = |v: Vector3d| Vector3d { z: v.z + 1.5, ..v };
            nav.line_of_sight(eye(at), eye(enemy.1))
        }
        None => true,
    };
    in_sight.then_some(enemy.0)
}

/// A post's guard, five yards in front of the civilian, facing it.
pub fn summon_guard(
    map: &mut Map,
    caller: Guid,
    entry: u32,
    at: Vector3d,
    facing: f32,
    enemy: Guid,
) -> Option<Guid> {
    let spot = Vector3d {
        x: at.x + 5.0 * facing.cos(),
        y: at.y + 5.0 * facing.sin(),
        z: at.z,
    };
    let spot = match map.nav.as_mut() {
        Some(nav) => nav
            .path(at, spot)
            .and_then(|p| p.last().copied())
            .unwrap_or(spot),
        None => spot,
    };
    // Despawn type 1: after its time, or dead.
    let lifetime = crate::game::map::guard_posts::GUARD_LIFETIME;
    let guard = crate::game::map::scripts::summon(
        map,
        caller,
        entry,
        spot,
        facing + std::f32::consts::PI,
        Some((lifetime, 1)),
    )?;
    crate::game::map::ai::attacked(map, guard, enemy);
    Some(guard)
}

/// Whether two creatures are on one side: one faction, or factions
/// friendly to each other.
fn friendly(map: &Map, a: Guid, b: Guid) -> bool {
    let faction = |g: Guid| {
        map.unit_fields(g)
            .map(|f| f.get_u32(UNIT_FIELD_FACTIONTEMPLATE))
    };
    let (fa, fb) = (faction(a), faction(b));
    if fa.is_some() && fa == fb {
        return true;
    }
    let templates = &map.realm.data.faction_templates;
    match (
        fa.and_then(|f| templates.get(&f)),
        fb.and_then(|f| templates.get(&f)),
    ) {
        (Some(x), Some(y)) => x.friendly_to(y),
        _ => false,
    }
}

/// The nearest guard of the civilian's side within fifty yards joins in.
fn rouse_nearest_guard(map: &mut Map, caller: Guid, at: Vector3d, enemy: Guid) {
    let guard = map
        .grid
        .around(at.x, at.y, GUARD_CALL_RADIUS)
        .filter(|g| guid::is_unit(*g) && *g != caller)
        .filter_map(|g| {
            let c = map.creature(g)?;
            (c.alive()
                && !c.combat.attacking
                && map
                    .realm
                    .data
                    .creature_template(c.entry)
                    .is_some_and(|t| t.flags_extra & EXTRA_GUARD != 0)
                && friendly(map, g, caller)
                && crate::game::map::combat::can_attack(map, g, enemy))
            .then(|| (g, dist(c.position, at)))
        })
        .filter(|(_, d)| *d <= GUARD_CALL_RADIUS)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(g, _)| g);
    if let Some(g) = guard {
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

/// A creature's mana each tick in a fight, from its class's spirit and
/// intellect (Thistle Tea's `creature_combat_mana_per_tick`).
pub fn combat_mana_per_tick(map: &Map, g: Guid) -> u32 {
    let Some(c) = map.creature(g) else {
        return 0;
    };
    let data = &map.realm.data;
    let Some(t) = data.creature_template(c.entry) else {
        return 0;
    };
    let level = c.fields.get_u32(UNIT_FIELD_LEVEL).min(255) as u8;
    let Some(stats) = data.class_level_stats.get(&(t.unit_class, level)) else {
        return 0;
    };
    let spirit = stats.spirit as f32;
    let per_tick = match t.unit_class {
        8 | 5 => spirit / 4.0 + 12.5,
        7 => spirit / 5.0 + 17.0,
        2 | 3 | 9 | 11 => spirit / 5.0 + 15.0,
        _ => 0.0,
    };
    let intellect = (stats.intellect as f32).max(1.0);
    ((per_tick / 2.0 + 0.6 * intellect.sqrt() / 5.0) * 5.0).max(0.0) as u32
}

/// Creatures regenerate: a third of health and mana each tick out of a
/// fight, health in one while polymorphed, and mana in one by spirit and
/// intellect outside the five-second rule.
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
    for &g in &due {
        let regenerates =
            template(map, g).is_none_or(|t| t.static_flags1 & STATIC_NO_AUTOMATIC_REGEN == 0);
        let poly = polymorphed(map, g);
        let combat_mana = combat_mana_per_tick(map, g);
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
        let (mana, max) = (
            f.get_u32(UNIT_FIELD_POWER1),
            f.get_u32(UNIT_FIELD_MAXPOWER1),
        );
        // Mana spent within five seconds holds it back.
        let cast_lately = c
            .engagement
            .mana_spent_at
            .is_some_and(|t| now.duration_since(t) < crate::game::map::stats::FIVE_SECOND_RULE);
        let gain = if max == 0 {
            0
        } else if !fighting {
            (max / 3).max(1)
        } else if cast_lately {
            0
        } else {
            combat_mana
        };
        c.fields.set_u32(UNIT_FIELD_POWER1, (mana + gain).min(max));
    }
    // Health back: wounds slow it no more.
    for g in due {
        crate::game::map::combat::sync_aura_state(map, g);
    }
}

/// The creatures' flight timers, and their regeneration.
pub fn update(map: &mut Map) {
    regenerate(map);
    watch_for_enemies(map);
    crate::game::map::assistance::update(map);
    let now = map.now;
    let pulses: Vec<(Guid, Guid)> = map
        .creatures
        .iter()
        .filter(|(_, c)| c.engagement.zone_pulse_at.is_some_and(|t| t <= now))
        .map(|(_, c)| (c.guid, c.combat.victim))
        .collect();
    for (g, victim) in pulses {
        let fighting = map.creature(g).is_some_and(|c| c.combat.attacking);
        if let Some(c) = map.creature_mut(g) {
            c.engagement.zone_pulse_at = fighting.then_some(now + ZONE_PULSE);
        }
        if fighting && zone_fighter(map, g) && !should_evade(map, g, victim) {
            zone_pulse(map, g, false);
        }
    }
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
