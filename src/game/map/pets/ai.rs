//! What a controlled creature does each update: follows its master, or
//! stays; fights what it was told to, or what its stance picks; casts its
//! autocast spells; heals and regains its power; and goes when its time is
//! up or its master has gone.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use super::*;
use crate::game::map::combat::{self, dist};
use crate::game::map::creature::Motion;

/// How often a pet thinks about its spells and what to fight.
const THINK_EVERY: Duration = Duration::from_millis(500);
/// How far from its master an aggressive pet looks for a fight.
const AGGRO_YARDS: f32 = 20.0;
/// A fight further than this from the master is given up.
const LEASH_YARDS: f32 = 80.0;
/// A chase paths again when its quarry has moved this far.
const REPATH_YARDS: f32 = 1.5;

/// One update of controlled creature `g`.
pub fn step(map: &mut Map, g: Guid, now: Instant) {
    let Some(c) = map.creature(g) else {
        return;
    };
    let Some(s) = c.pet.as_deref() else {
        return;
    };
    let (owner, kind, expires, dead_at) = (s.owner, s.kind, s.expires, s.dead_at);
    let alive = c.alive();
    // A player's pet goes when they have; a creature's when it has died.
    let owner_here =
        map.players.contains_key(&owner) || map.creature(owner).is_some_and(|o| o.alive());
    if !owner_here {
        summon::remove(map, g);
        return;
    }
    let owner_dead = map
        .players
        .get(&owner)
        .is_some_and(|p| p.death != crate::game::map::death::Death::Alive);
    // A guardian's time waits while it fights (Thistle Tea's
    // `TemporarySummon`), and it outlives a dead master while it fights,
    // or as a corpse (`Guardian.lifetime`).
    let fighting = kind == Kind::Guardian
        && map.unit_fields(g).is_some_and(|f| {
            f.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            )
        });
    let stays = kind == Kind::Guardian && (fighting || !alive);
    if owner_dead && matches!(kind, Kind::Totem | Kind::Guardian | Kind::Critter) && !stays {
        summon::remove(map, g);
        return;
    }
    mirror_flags(map, g, owner);
    if matches!(
        kind,
        Kind::Hunter | Kind::Demon | Kind::Guardian | Kind::Totem
    ) && alive
        && state(map, g).is_some_and(|s| s.inherit_at <= now)
    {
        if let Some(s) = state_mut(map, g) {
            s.inherit_at = now + inherit::EVERY;
        }
        inherit::sync(map, g);
    }
    if expires.is_some_and(|t| t <= now) && !fighting {
        // The Explosive Sheep's and the Goblin Bomb's time ends in a blast.
        if crate::game::map::item_scripts::guardian_expired(map, g) {
            return;
        }
        if matches!(kind, Kind::Possessed) {
            release(map, owner, g);
        }
        summon::remove(map, g);
        return;
    }
    if !alive {
        let stays = if kind == Kind::Hunter {
            CORPSE_STAYS
        } else {
            SUMMON_CORPSE_STAYS
        };
        let far = match (map.position_of(owner), map.position_of(g)) {
            (Some(a), Some(b)) => dist(a, b) > TELEPORT_DISTANCE * 2.0,
            _ => true,
        };
        if dead_at.is_none_or(|t| now.duration_since(t) >= stays) || far {
            summon::corpse_gone(map, g);
        }
        return;
    }
    crate::game::map::ai::advance_motion(map, g);
    match kind {
        Kind::Totem => totems::step(map, g, now),
        Kind::Possessed => {
            // The eye or mind control lasts as long as its channel or aura.
            let holding = state(map, g).map(|s| s.summon_spell).unwrap_or(0);
            let channel = map
                .caster(owner)
                .and_then(|c| c.casting.as_ref())
                .is_some_and(|c| c.spell == holding);
            let aura = crate::game::map::auras::auras(map, g).is_some_and(|a| {
                a.holders
                    .iter()
                    .any(|h| h.caster == owner && h.has(crate::game::map::auras::AURA_MOD_POSSESS))
            });
            if (holding != 0 && !channel) || (holding == 0 && !aura) {
                release(map, owner, g);
                if holding != 0 {
                    summon::remove(map, g);
                }
            }
        }
        Kind::Charmed => {
            let aura = crate::game::map::auras::auras(map, g).is_some_and(|a| {
                a.holders
                    .iter()
                    .any(|h| h.caster == owner && h.has(crate::game::map::auras::AURA_MOD_CHARM))
            });
            if !aura {
                release(map, owner, g);
                return;
            }
            regen(map, g, now);
            act(map, g, now);
        }
        Kind::Critter => follow(map, g, now),
        _ => {
            regen(map, g, now);
            if kind == Kind::Hunter {
                hunter::tick(map, g, now);
            }
            // Its master moves it (Eyes of the Beast): nothing of its own.
            if state(map, g).is_some_and(|s| s.eyes.is_some()) {
                return;
            }
            act(map, g, now);
        }
    }
}

/// A player's creature is flagged for PvP as its master is.
fn mirror_flags(map: &mut Map, g: Guid, owner: Guid) {
    let Some(pvp) = map.players.get(&owner).map(|p| {
        p.fields
            .has_flag(UNIT_FIELD_FLAGS, crate::game::map::player::UNIT_FLAG_PVP)
    }) else {
        return;
    };
    if let Some(c) = map.creature_mut(g)
        && c.fields
            .has_flag(UNIT_FIELD_FLAGS, crate::game::map::player::UNIT_FLAG_PVP)
            != pvp
    {
        c.fields.set_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_PVP,
            pvp,
        );
    }
}

/// Fight, or follow (or stay).
fn act(map: &mut Map, g: Guid, now: Instant) {
    let Some(s) = state(map, g) else {
        return;
    };
    let (owner, react) = (s.owner, s.react);
    let guardian = s.kind == Kind::Guardian;
    let current = map
        .combat(g)
        .filter(|c| c.attacking)
        .map(|c| c.victim)
        .filter(|v| still_fights(map, g, owner, *v));
    let victim = current.or_else(|| {
        if react == REACT_PASSIVE && !guardian {
            None
        } else {
            acquire(map, g, owner, react == REACT_AGGRESSIVE || guardian)
        }
    });
    match victim {
        Some(v) => fight(map, g, v, now),
        None => {
            if map.combat(g).is_some_and(|c| c.attacking) {
                combat::stop_attack(map, g);
            }
            calm(map, g, now);
            buff_self(map, g, now);
            let stay = state(map, g).and_then(|s| (s.command == COMMAND_STAY).then_some(s.stay_at));
            match stay {
                Some(Some(at)) => go_to(map, g, at, now),
                Some(None) => {}
                None => follow(map, g, now),
            }
        }
    }
}

/// Whether `v` is still worth fighting for `g`.
fn still_fights(map: &Map, g: Guid, owner: Guid, v: Guid) -> bool {
    let near_owner = match (map.position_of(owner), map.position_of(v)) {
        (Some(a), Some(b)) => dist(a, b) <= LEASH_YARDS,
        _ => false,
    };
    near_owner && map.unit_view(v).is_some_and(|u| u.alive) && combat::can_attack(map, g, v)
}

/// What a pet's stance makes it fight: whoever fights its master or it,
/// and (aggressive) what hostile comes near.
fn acquire(map: &Map, g: Guid, owner: Guid, aggressive: bool) -> Option<Guid> {
    let mut fighting: Vec<Guid> = map
        .combat(owner)
        .map(|c| c.attackers.iter().copied().collect())
        .unwrap_or_default();
    if let Some(c) = map.combat(g) {
        fighting.extend(c.attackers.iter().copied());
    }
    // A guardian joins its master's own fight too.
    if let Some(c) = map.combat(owner).filter(|c| c.attacking) {
        fighting.push(c.victim);
    }
    let pos = map.position_of(g)?;
    let pick = fighting
        .into_iter()
        .filter(|v| *v != g && *v != owner && still_fights(map, g, owner, *v))
        .filter(|v| {
            map.combat(*v).is_some_and(|c| {
                c.attackers.contains(&owner) || c.attackers.contains(&g) || c.attacking
            })
        })
        .min_by(|a, b| {
            let da = map.position_of(*a).map_or(f32::MAX, |p| dist(p, pos));
            let db = map.position_of(*b).map_or(f32::MAX, |p| dist(p, pos));
            da.total_cmp(&db)
        });
    if pick.is_some() || !aggressive {
        return pick;
    }
    map.grid
        .around(pos.x, pos.y, AGGRO_YARDS)
        .filter(|v| (guid::is_unit(*v) || guid::is_player(*v)) && *v != g && *v != owner)
        .filter(|v| {
            // Hostile creatures, and enemy players the rules let it fight;
            // never a civilian, whose death is a dishonorable kill
            // (Thistle Tea's `Acquisition.civilian_allowed?`).
            let foe = guid::is_player(*v)
                || (map.creature(*v).is_some_and(|c| {
                    c.alive()
                        && c.owner.is_none()
                        && !map
                            .realm
                            .data
                            .creature_template(c.entry)
                            .is_some_and(|t| t.civilian)
                }) && combat::hostile(map, *v, owner));
            foe && map
                .position_of(*v)
                .is_some_and(|p| dist(p, pos) <= AGGRO_YARDS)
                && combat::can_attack(map, g, *v)
        })
        .min_by(|a, b| {
            let da = map.position_of(*a).map_or(f32::MAX, |p| dist(p, pos));
            let db = map.position_of(*b).map_or(f32::MAX, |p| dist(p, pos));
            da.total_cmp(&db)
        })
}

/// The pet was attacked: it fights back unless passive or busy.
pub fn defend(map: &mut Map, g: Guid, attacker: Guid) {
    let Some(s) = state(map, g) else {
        return;
    };
    if s.react == REACT_PASSIVE && s.kind != Kind::Guardian {
        return;
    }
    if map
        .combat(g)
        .is_some_and(|c| c.attacking && c.victim != attacker)
        && map
            .combat(g)
            .is_some_and(|c| map.unit_view(c.victim).is_some_and(|u| u.alive))
    {
        return;
    }
    if combat::can_attack(map, g, attacker) {
        attack(map, g, attacker);
    }
}

/// Go for `target`.
pub fn attack(map: &mut Map, g: Guid, target: Guid) {
    if let Some(c) = map.creature_mut(g) {
        c.chase_target_at = None;
    }
    combat::start_attack(map, g, target);
    if let Some(c) = map.creature_mut(g) {
        c.combat.victim = target;
        c.combat.attacking = true;
        c.fields.set_guid(UNIT_FIELD_TARGET, target);
    }
}

/// The spell a pet would cast at `victim` now, of its harmful autocast
/// ones: the first ready, affordable and in range.
fn pick_spell(map: &Map, g: Guid, victim: Guid) -> Option<(u32, f32)> {
    let s = state(map, g)?;
    let data = &map.realm.data;
    let now = map.now;
    let caster = map.caster(g)?;
    if caster.casting.is_some() || caster.gcd_until.is_some_and(|t| t > now) {
        return None;
    }
    let f = map.unit_fields(g)?;
    let d = match (map.position_of(g), map.position_of(victim)) {
        (Some(a), Some(b)) => dist(a, b),
        _ => return None,
    };
    s.autocast.iter().copied().find_map(|id| {
        let t = data.spells.get(id)?;
        if crate::game::map::auras::positive(t) {
            return None;
        }
        if caster.cooldowns.get(&id).is_some_and(|c| *c > now)
            || (t.category != 0
                && caster
                    .category_cooldowns
                    .get(&t.category)
                    .is_some_and(|c| *c > now))
        {
            return None;
        }
        let power = t.power_type.min(4) as u16;
        if f.get_u32(UNIT_FIELD_POWER1 + power) < t.mana_cost {
            return None;
        }
        let (_, max) = data.spells.range(t);
        (d <= max + 2.0).then_some((id, max))
    })
}

/// `Attributes`: not in combat; it stops a swing; `AttributesEx`: no AI
/// autocasts it (Thistle Tea's `:not_in_combat`,
/// `:cancels_auto_attack_combat` and `:no_autocast_ai`).
const ATTR_NOT_IN_COMBAT: u32 = 0x1000_0000;
const ATTR_STOPS_ATTACK: u32 = 0x0010_0000;
const ATTR_EX_NO_AUTOCAST_AI: u32 = 0x0002_0000;

/// Out of a fight, a pet casts its autocast spells on itself that are
/// worth it (Thistle Tea's `autocast_self_spell`): Paranoia, Blood Pact.
fn buff_self(map: &mut Map, g: Guid, now: Instant) {
    if map.caster(g).is_some_and(|c| c.casting.is_some()) {
        return;
    }
    if !state(map, g).is_some_and(|s| s.next_think <= now) {
        return;
    }
    if let Some(s) = state_mut(map, g) {
        s.next_think = now + THINK_EVERY;
    }
    if let Some((spell, at)) = pick_positive(map, g, None) {
        crate::game::map::spells::cast(
            map,
            g,
            spell,
            Targets {
                unit: Some(at),
                ..Default::default()
            },
            None,
        );
    }
}

/// A positive autocast spell worth casting now and whom on, as Thistle
/// Tea's `Autocast` has it: on the pet itself, or (a single friend's) its
/// master or itself; out of a fight (no `victim`), only on itself and not
/// those for a fight. Dash and Dive while it isn't in reach, Fire Shield on
/// one under attack, a heal on the hurt, and a buff on one without it.
fn pick_positive(map: &Map, g: Guid, victim: Option<Guid>) -> Option<(u32, Guid)> {
    let s = state(map, g)?;
    let data = &map.realm.data;
    let now = map.now;
    let caster = map.caster(g)?;
    if caster.casting.is_some() || caster.gcd_until.is_some_and(|t| t > now) {
        return None;
    }
    let f = map.unit_fields(g)?;
    s.autocast.iter().copied().find_map(|id| {
        let t = data.spells.get(id)?;
        if !crate::game::map::auras::positive(t)
            || t.passive()
            || t.attr(1, ATTR_EX_NO_AUTOCAST_AI)
            || (victim.is_some() && t.attr(0, ATTR_STOPS_ATTACK))
            || (victim.is_none() && combat_only(map, t) && !t.attr(0, ATTR_NOT_IN_COMBAT))
        {
            return None;
        }
        if caster.cooldowns.get(&id).is_some_and(|c| *c > now)
            || (t.category != 0
                && caster
                    .category_cooldowns
                    .get(&t.category)
                    .is_some_and(|c| *c > now))
            || f.get_u32(UNIT_FIELD_POWER1 + t.power_type.min(4) as u16) < t.mana_cost
        {
            return None;
        }
        let on_self = t
            .effects
            .iter()
            .filter(|e| e.effect != 0)
            .all(|e| matches!(e.target_a, 0 | 1 | 20 | 22));
        let whom: &[Guid] = if on_self {
            &[g]
        } else if victim.is_some() {
            &[s.owner, g]
        } else {
            &[]
        };
        whom.iter()
            .copied()
            .find(|w| worth(map, g, t, *w, victim))
            .map(|w| (id, w))
    })
}

/// Fire Shield (Thistle Tea's `fire_shield?`).
fn fire_shield(t: &crate::game::data::spells::SpellTemplate) -> bool {
    t.spell_visual == 289 && t.family_name == 5 && t.family_flags & 0x0080_0000 != 0
}

/// A spell for a fight only (Thistle Tea's `combat_only?`): one that
/// kills or raises damage, or whose cooldown outlasts it.
fn combat_only(map: &Map, t: &crate::game::data::spells::SpellTemplate) -> bool {
    use crate::game::map::auras::AURA_MOD_DAMAGE_DONE;
    if fire_shield(t) {
        return false;
    }
    if t.spell_icon == 153 || t.has_effect(5) || t.has_aura(AURA_MOD_DAMAGE_DONE) {
        return true;
    }
    let cooldown = t.recovery_time.max(t.category_recovery_time);
    map.realm
        .data
        .spells
        .duration(t, 1)
        .is_some_and(|d| cooldown > d)
}

/// Whether `t` on `whom` does any good now (Thistle Tea's `shield_useful?`,
/// `heal_useful?` and `auras_useful?`).
fn worth(
    map: &Map,
    g: Guid,
    t: &crate::game::data::spells::SpellTemplate,
    whom: Guid,
    victim: Option<Guid>,
) -> bool {
    use crate::game::map::auras::*;
    if !map.unit_view(whom).is_some_and(|u| u.alive) {
        return false;
    }
    if fire_shield(t)
        && !map.combat(whom).is_some_and(|c| {
            c.attackers
                .iter()
                .any(|a| map.unit_view(*a).is_some_and(|x| x.alive))
        })
    {
        return false;
    }
    let heals = t.has_effect(10) || t.has_aura(AURA_PERIODIC_HEAL);
    if heals
        && map
            .unit_fields(whom)
            .is_some_and(|f| f.get_u32(UNIT_FIELD_HEALTH) >= f.get_u32(UNIT_FIELD_MAXHEALTH))
    {
        return false;
    }
    if t.has_effect(2) {
        return true;
    }
    let held = auras(map, whom).is_some_and(|a| a.has_spell(t.id));
    let in_reach = victim
        .and_then(|v| map.unit_view(v))
        .zip(map.unit_view(g))
        .is_some_and(|(v, me)| combat::in_melee_range(&me, &v));
    t.effects.iter().all(|e| match e.effect {
        6 if t.stack_amount <= 1 => {
            !held
                && !(e.aura == AURA_MOD_INCREASE_SPEED && in_reach)
                && !(e.aura == AURA_MOD_STUN && stunned(map, whom))
        }
        35 => !held,
        _ => true,
    })
}

fn stunned(map: &Map, g: Guid) -> bool {
    map.unit_fields(g)
        .is_some_and(|f| f.get_u32(UNIT_FIELD_FLAGS) & 0x0004_0000 != 0)
}

/// The longest range of the pet's autocast harmful spells it can afford:
/// a caster stands back and casts.
fn caster_range(map: &Map, g: Guid) -> Option<f32> {
    let s = state(map, g)?;
    let data = &map.realm.data;
    let f = map.unit_fields(g)?;
    s.autocast
        .iter()
        .filter_map(|id| data.spells.get(*id))
        .filter(|t| !crate::game::map::auras::positive(t))
        .filter(|t| t.has_effect(2) && t.casting_time_index > 1)
        .filter(|t| f.get_u32(UNIT_FIELD_POWER1 + t.power_type.min(4) as u16) >= t.mana_cost)
        .map(|t| data.spells.range(t).1)
        .filter(|r| *r > 8.0)
        .reduce(f32::max)
}

/// One step of a fight with `victim`.
fn fight(map: &mut Map, g: Guid, victim: Guid, now: Instant) {
    if !map
        .combat(g)
        .is_some_and(|c| c.attacking && c.victim == victim)
    {
        attack(map, g, victim);
    }
    let busy = map.caster(g).is_some_and(|c| c.casting.is_some())
        || crate::game::map::auras::held(map, g, false);
    if busy {
        stop(map, g);
        return;
    }
    // Its spells, now and then.
    let think = state(map, g).is_some_and(|s| s.next_think <= now);
    if think {
        if let Some(s) = state_mut(map, g) {
            s.next_think = now + THINK_EVERY;
        }
        let pick = pick_spell(map, g, victim)
            .map(|(spell, _)| (spell, victim))
            .or_else(|| pick_positive(map, g, Some(victim)));
        if let Some((spell, at)) = pick {
            crate::game::map::spells::cast(
                map,
                g,
                spell,
                Targets {
                    unit: Some(at),
                    ..Default::default()
                },
                None,
            );
            if map.caster(g).is_some_and(|c| c.casting.is_some()) {
                stop(map, g);
                face(map, g, victim);
                return;
            }
        }
    }
    let (Some(me), Some(them)) = (map.unit_view(g), map.unit_view(victim)) else {
        return;
    };
    let standoff = caster_range(map, g);
    let close_enough = match standoff {
        Some(r) => dist(me.position, them.position) <= r - 2.0,
        None => combat::in_melee_range(&me, &them),
    };
    if close_enough {
        stop(map, g);
        face(map, g, victim);
        if standoff.is_none() {
            combat::try_swing(map, g);
        }
        return;
    }
    let stale = map.creature(g).is_some_and(|c| {
        c.chase_target_at
            .is_none_or(|at| dist(at, them.position) > REPATH_YARDS)
            || !matches!(c.motion, Motion::Moving(_))
    });
    if !stale {
        return;
    }
    let reach = match standoff {
        Some(r) => (r - 4.0).max(5.0),
        None => combat::melee_range(&me, &them) * 0.8,
    };
    let to_me = Vector3d {
        x: me.position.x - them.position.x,
        y: me.position.y - them.position.y,
        z: 0.0,
    };
    let len = (to_me.x * to_me.x + to_me.y * to_me.y).sqrt().max(0.01);
    let goal = Vector3d {
        x: them.position.x + to_me.x / len * reach.min(len),
        y: them.position.y + to_me.y / len * reach.min(len),
        z: them.position.z,
    };
    walk(map, g, goal, now);
    if let Some(c) = map.creature_mut(g) {
        c.chase_target_at = Some(them.position);
    }
}

/// Out of the fight: the combat flag goes when nothing fights it.
fn calm(map: &mut Map, g: Guid, now: Instant) {
    let fighting = map.combat(g).is_some_and(|c| {
        c.attackers
            .iter()
            .any(|a| map.unit_view(*a).is_some_and(|u| u.alive))
    });
    let since = map
        .combat(g)
        .and_then(|c| c.last_action)
        .is_some_and(|t| now.duration_since(t) < combat::COMBAT_TIMEOUT);
    if !fighting
        && !since
        && map.unit_fields(g).is_some_and(|f| {
            f.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            )
        })
    {
        combat::leave_combat(map, g);
    }
}

fn face(map: &mut Map, g: Guid, other: Guid) {
    let (Some(a), Some(b)) = (map.position_of(g), map.position_of(other)) else {
        return;
    };
    if let Some(c) = map.creature_mut(g) {
        c.orientation = (b.y - a.y).atan2(b.x - a.x);
    }
}

/// Stop where it is.
pub fn stop(map: &mut Map, g: Guid) {
    let spline = map.next_spline_id;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    if !matches!(c.motion, Motion::Moving(_)) {
        return;
    }
    c.motion = Motion::Idle { until: None };
    let at = c.position;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let frame = crate::game::map::ai::stop_packet(g, at, spline);
    map.send_to_observers(g, &frame, false);
}

/// Walk (run) to `goal`, by the navmesh where there is one.
fn walk(map: &mut Map, g: Guid, goal: Vector3d, now: Instant) {
    let Some(from) = map.position_of(g) else {
        return;
    };
    let path = match map.nav.as_mut() {
        Some(nav) => nav.path(from, goal).unwrap_or_else(|| vec![from, goal]),
        None => vec![from, goal],
    };
    let spline = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    if let Some(c) = map.creature_mut(g)
        && let Some(frame) = c.walk(path, true, now, spline)
    {
        map.send_to_observers(g, &frame, false);
    }
}

fn destination(map: &Map, g: Guid) -> Option<Vector3d> {
    match &map.creature(g)?.motion {
        Motion::Moving(w) => Some(w.destination()),
        _ => None,
    }
}

/// Go to `at`, unless there or on the way.
fn go_to(map: &mut Map, g: Guid, at: Vector3d, now: Instant) {
    let Some(pos) = map.position_of(g) else {
        return;
    };
    if dist(pos, at) <= 1.0 || destination(map, g).is_some_and(|d| dist(d, at) <= 1.0) {
        return;
    }
    walk(map, g, at, now);
}

/// Keep at the master's side, as fast as they go; too far, and it jumps
/// to them.
fn follow(map: &mut Map, g: Guid, now: Instant) {
    let Some(s) = state(map, g) else {
        return;
    };
    let (owner, angle) = (s.owner, s.angle);
    let Some(o) = map.unit_view(owner) else {
        return;
    };
    let Some(pos) = map.position_of(g) else {
        return;
    };
    let a = o.orientation + angle;
    let spot = Vector3d {
        x: o.position.x + FOLLOW_DISTANCE * a.cos(),
        y: o.position.y + FOLLOW_DISTANCE * a.sin(),
        z: o.position.z,
    };
    let d = dist(pos, spot);
    if d > TELEPORT_DISTANCE {
        jump(map, g, spot, o.orientation);
        return;
    }
    if d <= FOLLOW_DISTANCE * 1.25 {
        return;
    }
    if destination(map, g).is_some_and(|dest| dist(dest, spot) <= FOLLOW_DISTANCE) {
        return;
    }
    // As fast as its master, or its own pace if faster.
    let owner_speed = map.players.get(&owner).map(|p| p.speeds.run).unwrap_or(7.0);
    if let Some(c) = map.creature_mut(g) {
        let own = if c.base.run_speed > 0.0 {
            c.base.run_speed
        } else {
            7.0
        };
        c.speeds.run = own.max(owner_speed);
    }
    walk(map, g, spot, now);
}

/// Put the pet at `at` at once.
fn jump(map: &mut Map, g: Guid, at: Vector3d, orientation: f32) {
    let spline = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let (anchor, old_cell) = (c.vis_anchor, c.cell);
    c.position = at;
    c.orientation = orientation;
    c.motion = Motion::Idle { until: None };
    let new_cell = crate::game::map::grid::cell_of(at.x, at.y);
    c.cell = new_cell;
    map.grid.relocate(g, old_cell, new_cell);
    crate::game::map::visibility::moved(map, g, anchor, at);
    let frame = crate::game::map::ai::stop_packet(g, at, spline);
    map.send_to_observers(g, &frame, false);
}

/// Health and power come back: health out of a fight, a demon's mana out
/// of a fight, a hunter's pet's focus all the time.
fn regen(map: &mut Map, g: Guid, now: Instant) {
    let due = state(map, g).is_some_and(|s| s.regen_at <= now);
    if !due {
        return;
    }
    let kind = state(map, g).map(|s| s.kind);
    let in_combat = map.unit_fields(g).is_some_and(|f| {
        f.has_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        )
    });
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    if let Some(s) = c.pet.as_deref_mut() {
        s.regen_at = now + Duration::from_secs(2);
    }
    let f = &mut c.fields;
    if !in_combat {
        let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
        let h = f.get_u32(UNIT_FIELD_HEALTH);
        f.set_u32(UNIT_FIELD_HEALTH, (h + (max / 20).max(1)).min(max));
        let mana_max = f.get_u32(UNIT_FIELD_MAXPOWER1);
        let mana = f.get_u32(UNIT_FIELD_POWER1);
        f.set_u32(
            UNIT_FIELD_POWER1,
            (mana + (mana_max / 20).max(1)).min(mana_max),
        );
    }
    if kind == Some(Kind::Hunter) {
        // vmangos's focus: 24 every four seconds.
        let max = f.get_u32(UNIT_FIELD_MAXPOWER1 + POWER_FOCUS);
        let focus = f.get_u32(UNIT_FIELD_POWER1 + POWER_FOCUS);
        f.set_u32(UNIT_FIELD_POWER1 + POWER_FOCUS, (focus + 12).min(max));
    }
}
