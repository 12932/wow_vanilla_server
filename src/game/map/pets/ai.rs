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
/// How far above or below an aggressive pet's sight goes, beyond both
/// reaches (mangos's `CREATURE_Z_ATTACK_RANGE_MELEE`).
const AGGRO_Z_YARDS: f32 = 3.0;
/// A unit a pet doesn't take on of its own: stunned, confused or fleeing
/// (mangos's `IsCrowdControlled`).
const CROWD_CONTROLLED: u32 = crate::game::map::player::UNIT_FLAG_STUNNED
    | crate::game::map::auras::UNIT_FLAG_CONFUSED
    | crate::game::map::auras::UNIT_FLAG_FLEEING;
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
    // A player's pet goes when they have; a creature's when it has.
    let owner_here = map.players.contains_key(&owner) || map.creature(owner).is_some();
    if !owner_here {
        summon::remove(map, g);
        return;
    }
    // A dead master's guardian fights on, or lies, as below; its idle one
    // goes (Thistle Tea's `CreaturePet.lifetime`, a player's or a
    // creature's alike).
    let owner_dead = map
        .players
        .get(&owner)
        .is_some_and(|p| p.death != crate::game::map::death::Death::Alive)
        || map.creature(owner).is_some_and(|o| !o.alive());
    // A guardian outlives a dead master while it fights, or as a corpse
    // (Thistle Tea's `Guardian.lifetime`).
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
    // Its time up, a living summon goes, in a fight or not: a player's
    // guardian (a Mechanical Dragonling), a creature's, a Target Dummy
    // (mangos-classic's `Pet::Update`, its `m_duration` run out mid-fight
    // too, where Thistle Tea's `TemporarySummon` waits for the fight's
    // end); a corpse lies its own time (`Guardian.lifetime`).
    if expires.is_some_and(|t| t <= now) && (alive || kind == Kind::Possessed) {
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
        // A totem struck down sinks at once, leaving no corpse and its slot
        // free (mangos's `Totem::Update`, Thistle Tea's totem stop).
        if kind == Kind::Totem {
            summon::remove(map, g);
            return;
        }
        // A hunter's pet lies as long as a creature of its template's
        // rank (mangos's `Creature::InitEntry`, `Corpse.Decay.*`); a
        // summoned one five seconds (`Pet::Create`'s `SetCorpseDelay(5)`).
        let stays = if kind == Kind::Hunter {
            let rank = map
                .creature(g)
                .and_then(|c| map.realm.data.creature_template(c.entry))
                .map_or(0, |t| t.rank);
            map.cfg.creatures.corpse_decay(rank)
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
    // A guardian's, a companion's or a creature's pet's own events (its
    // EventAI), before its pet AI's step; one of them may have taken it.
    crate::game::map::event_ai::owned_tick(map, g);
    if !map
        .creature(g)
        .is_some_and(|c| c.alive() && c.pet.is_some())
    {
        return;
    }
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
                return;
            }
            // Its swing, while it is sent at one: no AI moves it, but it
            // strikes on at its victim in reach, as a player's own swing
            // does (mangos's `PossessedAI::UpdateAI`, `DoMeleeAttackIfReady`).
            // Without, it struck once.
            combat::try_swing(map, g);
        }
        Kind::Charmed => {
            let aura = crate::game::map::auras::auras(map, g).is_some_and(|a| {
                a.holders.iter().any(|h| {
                    h.caster == owner
                        && (h.has(crate::game::map::auras::AURA_MOD_CHARM)
                            || h.has(crate::game::map::auras::AURA_AOE_CHARM))
                })
            });
            if !aura {
                release(map, owner, g);
                return;
            }
            regen(map, g, now);
            act(map, g, now);
        }
        // A companion follows, but where a script told it to stay (Thistle
        // Tea's mini-pet tree, its command state).
        Kind::Critter => {
            let stay = state(map, g).and_then(|s| (s.command == COMMAND_STAY).then_some(s.stay_at));
            match stay {
                Some(Some(at)) => go_to(map, g, at, now),
                Some(None) => {}
                None => follow(map, g, now),
            }
        }
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
    // A Target Dummy stands where it came and fights no one.
    if crate::game::map::item_scripts::target_dummy(map, g) {
        calm(map, g, now);
        return;
    }
    let (owner, react, returning) = (s.owner, s.react, s.returning);
    let guardian = s.kind == Kind::Guardian;
    // A victim put under crowd control since its master's last Attack it
    // lets be (mangos's `PetAI::UpdateAI`, `HasAuraPetShouldAvoidBreaking`).
    let current = map
        .combat(g)
        .filter(|c| c.attacking)
        .map(|c| c.victim)
        .filter(|v| still_fights(map, g, owner, *v))
        .filter(|v| !avoids_breaking(map, g, *v));
    // Called back, it takes nothing new up on the way.
    let victim = current.or_else(|| {
        if (react == REACT_PASSIVE && !guardian) || returning {
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
            // A cast under way (its own buff, or one its master told it)
            // is finished where it stands before it goes anywhere
            // (Thistle Tea's `:casting`, mangos's `PetAI::UpdateAI`).
            if map.caster(g).is_some_and(|c| c.casting.is_some()) {
                stop(map, g);
                return;
            }
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

/// What a pet's stance makes it fight: those fighting it, then its
/// master's victim, then those fighting its master (mangos's `AttackedBy`
/// and `getAttackerForHelper`, Thistle Tea's `TargetSelection.next`), the
/// nearest of each; and (aggressive) what hostile comes near. A pet told
/// to stay takes on of its own accord only what stands within its reach,
/// as Thistle Tea's `Targeting.within_stay_range?` has it.
fn acquire(map: &Map, g: Guid, owner: Guid, aggressive: bool) -> Option<Guid> {
    // A creature's own pet takes no fight up while its master goes home,
    // and takes its foes by its threat list first (Thistle Tea's
    // `TargetSelection.next` for NPC pets; mangos's `GuardianAI`, a
    // `CreatureEventAI` on its own list).
    let creatures = !guid::is_player(owner);
    if creatures && map.creature(owner).is_some_and(|c| c.evading) {
        return None;
    }
    let pick = creatures
        .then(|| by_threat(map, g, owner))
        .flatten()
        .or_else(|| acquire_any(map, g, owner, aggressive));
    let stays = state(map, g).is_some_and(|s| s.command == COMMAND_STAY);
    if !stays {
        return pick;
    }
    let me = map.unit_view(g)?;
    let within = |v: Guid| {
        map.unit_view(v)
            .is_some_and(|u| combat::in_melee_range(&me, &u))
    };
    match pick {
        Some(v) if within(v) => Some(v),
        // The nearest is out of reach; none nearer is in it.
        _ => None,
    }
}

/// The foe highest on a creature's pet's threat list it may fight (the
/// lower guid of two as high), but one under crowd control it leaves be.
fn by_threat(map: &Map, g: Guid, owner: Guid) -> Option<Guid> {
    map.creature(g)?
        .threat
        .iter()
        .copied()
        .filter(|(t, _)| *t != owner && still_fights(map, g, owner, *t))
        .filter(|(t, _)| !avoids_breaking(map, g, *t))
        .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.guid().cmp(&a.0.guid())))
        .map(|(t, _)| t)
}

/// Whether `v` is held by crowd control that damage would break
/// (Polymorph, Sap, Gouge): a pet leaves it be of its own accord, but an
/// aggressive guardian (Thistle Tea's `Targeting.crowd_control_allowed?`,
/// mangos's `HasAuraPetShouldAvoidBreaking`).
#[cfg(test)]
pub(crate) fn breakable_cc(map: &Map, v: Guid) -> bool {
    use crate::game::map::auras::{
        AURA_MOD_CONFUSE, AURA_MOD_STUN, AURA_TRANSFORM, INTERRUPT_DAMAGE,
    };
    crate::game::map::auras::auras(map, v).is_some_and(|a| {
        a.holders.iter().any(|h| {
            h.interrupt_flags & INTERRUPT_DAMAGE != 0
                && (h.has(AURA_MOD_CONFUSE) || h.has(AURA_MOD_STUN) || h.has(AURA_TRANSFORM))
        })
    })
}

/// Whether pet `g` leaves `v` be for its crowd control: one that damage
/// would break, put on since its master last sent it to attack, and not by
/// its own channel (Seduction). An aggressive guardian breaks it (mangos's
/// `HasAuraPetShouldAvoidBreaking`, Thistle Tea's
/// `Targeting.crowd_control_allowed?`).
fn avoids_breaking(map: &Map, g: Guid, v: Guid) -> bool {
    use crate::game::map::auras::{
        AURA_MOD_CONFUSE, AURA_MOD_STUN, AURA_TRANSFORM, INTERRUPT_DAMAGE,
    };
    let Some(s) = state(map, g) else {
        return false;
    };
    if s.kind == Kind::Guardian && s.react == REACT_AGGRESSIVE {
        return false;
    }
    // The override is for the victim the Attack named, not for the next
    // (Thistle Tea's "an explicit command does not carry its override into
    // a protected next target"; mangos applies the time to any).
    let since = s.commanded_at.filter(|_| s.commanded == Some(v));
    let channel = map
        .caster(g)
        .and_then(|c| c.casting.as_ref())
        .map(|c| c.spell);
    crate::game::map::auras::auras(map, v).is_some_and(|a| {
        a.holders.iter().any(|h| {
            let applied = match (h.expires, h.duration) {
                (Some(t), Some(ms)) => t.checked_sub(Duration::from_millis(u64::from(ms))),
                _ => None,
            };
            h.interrupt_flags & INTERRUPT_DAMAGE != 0
                && (h.has(AURA_MOD_CONFUSE) || h.has(AURA_MOD_STUN) || h.has(AURA_TRANSFORM))
                && Some(h.spell) != channel
                && since.is_none_or(|t| applied.is_none_or(|a| a >= t))
        })
    })
}

fn acquire_any(map: &Map, g: Guid, owner: Guid, aggressive: bool) -> Option<Guid> {
    let pos = map.position_of(g)?;
    let nearest = |list: Vec<Guid>| {
        list.into_iter()
            .filter(|v| *v != g && *v != owner && still_fights(map, g, owner, *v))
            .filter(|v| !avoids_breaking(map, g, *v))
            .filter(|v| {
                map.combat(*v).is_some_and(|c| {
                    c.attackers.contains(&owner) || c.attackers.contains(&g) || c.attacking
                })
            })
            .min_by(|a, b| {
                let da = map.position_of(*a).map_or(f32::MAX, |p| dist(p, pos));
                let db = map.position_of(*b).map_or(f32::MAX, |p| dist(p, pos));
                da.total_cmp(&db).then(a.guid().cmp(&b.guid()))
            })
    };
    let own: Vec<Guid> = map
        .combat(g)
        .map(|c| c.attackers.iter().copied().collect())
        .unwrap_or_default();
    let owners_victim: Vec<Guid> = map
        .combat(owner)
        .filter(|c| c.attacking)
        .map(|c| vec![c.victim])
        .unwrap_or_default();
    let owners: Vec<Guid> = map
        .combat(owner)
        .map(|c| c.attackers.iter().copied().collect())
        .unwrap_or_default();
    let pick = nearest(own)
        .or_else(|| nearest(owners_victim))
        .or_else(|| nearest(owners));
    if pick.is_some() || !aggressive {
        return pick;
    }
    // As far as its level lets it notice (mangos's `PetAI::MoveInLineOfSight`,
    // `GetAttackDistance`: its detection range, a yard more for each level
    // it has over its quarry, 25 at most).
    let search = map.creature(g).map_or(0.0, |c| {
        let detection = map
            .realm
            .data
            .creature_template(c.entry)
            .map_or(20.0, |t| t.detection_range);
        let modifier = c
            .auras
            .sum(crate::game::map::auras::AURA_MOD_DETECT_RANGE)
            .max(0) as f32;
        (detection + 25.0 + modifier).max(5.0) * map.cfg.creatures.aggro_rate
    });
    let reach = |v: Guid| {
        map.unit_fields(v)
            .map_or(0.0, |f| f.get_f32(UNIT_FIELD_COMBATREACH))
    };
    let own_reach = reach(g);
    map.grid
        .around(pos.x, pos.y, search)
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
            foe && map.position_of(*v).is_some_and(|p| {
                dist(p, pos) <= crate::game::map::ai::attack_distance(map, g, *v)
                    && (p.z - pos.z).abs() - own_reach - reach(*v) <= AGGRO_Z_YARDS
            })
                && combat::can_attack(map, g, *v)
                && !avoids_breaking(map, g, *v)
                // None held by a stun, fear or confusion (mangos's
                // `!who->IsCrowdControlled()`).
                && !map
                    .unit_fields(*v)
                    .is_some_and(|f| f.get_u32(UNIT_FIELD_FLAGS) & CROWD_CONTROLLED != 0)
                // Where its habitat lets it go (Thistle Tea's
                // `Acquisition.nearest`, `CreatureMovement.accessible?`).
                && crate::game::map::creature_motion::may_reach(map, g, *v)
                // In its sight (mangos's `PetAI::MoveInLineOfSight`,
                // `IsWithinLOSInMap`; Thistle Tea's obstructed units).
                && map.position_of(*v).is_some_and(|p| {
                    crate::game::map::spells::checks::sight(map, pos, p)
                })
        })
        // The nearest, the lower guid of two as near (Thistle Tea's).
        .min_by(|a, b| {
            let da = map.position_of(*a).map_or(f32::MAX, |p| dist(p, pos));
            let db = map.position_of(*b).map_or(f32::MAX, |p| dist(p, pos));
            da.total_cmp(&db).then(a.guid().cmp(&b.guid()))
        })
}

/// The pet was attacked: it fights back unless passive or busy.
pub fn defend(map: &mut Map, g: Guid, attacker: Guid) {
    let Some(s) = state(map, g) else {
        return;
    };
    if (s.react == REACT_PASSIVE && s.kind != Kind::Guardian)
        || s.returning
        || crate::game::map::item_scripts::target_dummy(map, g)
    {
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
    back(map, g);
    if let Some(c) = map.creature_mut(g) {
        c.chase_target_at = None;
    }
    // In reach, it turns to its victim before its first swing, which may
    // come at once (Thistle Tea's `SetFacing` before a pet's attack).
    if let (Some(me), Some(them)) = (map.unit_view(g), map.unit_view(target))
        && combat::in_melee_range(&me, &them)
    {
        face(map, g, target);
    }
    combat::start_attack(map, g, target);
    map.note_fighting(g);
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
        if helpful(t) {
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
        let max = reach(map, g, t);
        (d <= max + 2.0).then_some((id, max))
    })
}

/// The opener its master's bar sent it at `victim` with, once in its range
/// and sight; let go of with its victim nearer than its least range, and
/// kept while it is further than its most (mangos's `PickSpellWithTarget`).
fn opener(map: &mut Map, g: Guid, victim: Guid) -> Option<(u32, Guid)> {
    let (spell, min, max) = state(map, g)?.opener?;
    let (a, b) = (map.position_of(g)?, map.position_of(victim)?);
    let radius = |x: Guid| {
        map.unit_fields(x)
            .map_or(0.0, |f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS).max(0.0))
    };
    let d = dist(a, b) - radius(g) - radius(victim);
    if min > 0.0 && d < min {
        state_mut(map, g)?.opener = None;
        return None;
    }
    if d >= max || !crate::game::map::spells::checks::sight(map, a, b) {
        return None;
    }
    state_mut(map, g)?.opener = None;
    Some((spell, victim))
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
        if !helpful(t)
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
        // A dispel (Devour Magic) goes only where it has something to
        // take, and gives way to the next spell where it hasn't.
        if dispels(t) {
            let range = data.spells.range(t).1;
            return dispel_target(map, g, s.owner, t, victim, range).map(|w| (id, w));
        }
        let effects = || t.effects.iter().filter(|e| e.effect != 0);
        let on_self = effects().all(|e| matches!(e.target_a, 0 | 1 | 20 | 22));
        let on_master = effects().all(|e| e.target_a == TARGET_MASTER);
        let whom: Vec<Guid> = if on_self {
            vec![g]
        } else if on_master {
            vec![s.owner]
        } else {
            let mut whom = allies(map, g, s.owner, data.spells.range(t).1);
            whom.push(g);
            whom
        };
        whom.into_iter()
            .find(|w| worth(map, g, t, *w, victim))
            .map(|w| (id, w))
    })
}

/// A spell a pet casts for itself or a friend, not at its victim: a
/// helpful one, or one wholly on itself (Prowl, whose slow is its own), as
/// mangos's `IsPositiveEffectTargetMode` takes an effect on the caster
/// (Thistle Tea's Prowl among the self spells).
pub(super) fn helpful(t: &crate::game::data::spells::SpellTemplate) -> bool {
    crate::game::map::auras::positive(t)
        || t.effects
            .iter()
            .filter(|e| e.effect != 0)
            .all(|e| e.target_a == 1 && matches!(e.target_b, 0 | 1))
            && t.effects.iter().any(|e| e.effect != 0)
}

/// `TARGET_MASTER`: a spell on the pet's master alone.
const TARGET_MASTER: u32 = 27;

/// `SPELL_EFFECT_DISPEL`.
const EFFECT_DISPEL: u32 = 38;

/// Whether every effect of `t` is a dispel (Devour Magic).
fn dispels(t: &crate::game::data::spells::SpellTemplate) -> bool {
    let mut effects = t.effects.iter().filter(|e| e.effect != 0).peekable();
    effects.peek().is_some() && effects.all(|e| e.effect == EFFECT_DISPEL)
}

/// Whom a pet's dispel goes on, of those within `range` and in its sight:
/// an enemy with a buff it takes (its victim, its master's, then those
/// fighting either, the lower guid first), else a friend with an
/// affliction it takes (its master, then their party), as Thistle Tea's
/// `Autocast.candidates` has it for a spell at any unit, by the dispel
/// options it observes (mangos's `PetAI` tries its victim first, its
/// `CheckCast` refusing one with nothing to take). Never itself: Devour
/// Magic takes nothing of its caster's own (mangos's `DevourMagic`
/// script).
fn dispel_target(
    map: &Map,
    g: Guid,
    owner: Guid,
    t: &crate::game::data::spells::SpellTemplate,
    victim: Option<Guid>,
    range: f32,
) -> Option<Guid> {
    let me = map.position_of(g)?;
    let victim_of = |u: Guid| map.combat(u).filter(|c| c.attacking).map(|c| c.victim);
    let mut attackers: Vec<Guid> = [g, owner]
        .iter()
        .filter_map(|u| map.combat(*u))
        .flat_map(|c| c.attackers.iter().copied())
        .collect();
    attackers.sort_by_key(|a| a.guid());
    let near = |w: &Guid| {
        map.unit_view(*w).is_some_and(|u| {
            u.alive
                && dist(u.position, me) <= range + 2.0
                && crate::game::map::spells::checks::sight(map, me, u.position)
        })
    };
    let enemies = victim
        .or_else(|| victim_of(g))
        .into_iter()
        .chain(victim_of(owner))
        .chain(attackers)
        .filter(|e| !e.is_zero() && *e != g && combat::can_attack(map, g, *e) && near(e));
    let friends = allies(map, g, owner, range).into_iter();
    let takes = |w: &Guid| {
        t.effects
            .iter()
            .filter(|e| e.effect == EFFECT_DISPEL)
            .any(|e| !crate::game::map::spells::dispellable(map, g, *w, e.misc_value).is_empty())
    };
    enemies.chain(friends).find(|w| takes(w))
}

/// Whom of its master's a pet's spell for a friend may go on, as mangos's
/// `PetAI::UpdateAllies` and Thistle Tea's `pet_allies`: its master, then
/// the others of its master's party (in a raid, of their subgroup), each
/// here, alive, within `range` of it and in its sight.
fn allies(map: &Map, g: Guid, owner: Guid, range: f32) -> Vec<Guid> {
    let Some(me) = map.position_of(g) else {
        return Vec::new();
    };
    let mut out = vec![owner];
    if guid::is_player(owner) {
        out.extend(
            crate::game::map::spells::targets::members(map, owner, false)
                .into_iter()
                .filter(|m| *m != owner),
        );
    }
    out.retain(|a| {
        map.unit_view(*a).is_some_and(|u| {
            u.alive
                && dist(u.position, me) <= range.max(5.0)
                && crate::game::map::spells::checks::sight(map, me, u.position)
        })
    });
    out
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
    // Effect 1 is an instant kill (Sacrifice).
    if t.spell_icon == 153 || t.has_effect(1) || t.has_aura(AURA_MOD_DAMAGE_DONE) {
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
    // An effect held is judged by its index: one a holder of the spell
    // has lost (taken off alone) is worth putting back (mangos's
    // `PetAI::CanAutoCast`, `HasAura(id, index)`; Thistle Tea's
    // `held?` by spell and effect).
    let held = |i: usize| {
        auras(map, whom).is_some_and(|a| {
            a.holders
                .iter()
                .any(|h| h.spell == t.id && h.effects.iter().any(|x| usize::from(x.effect) == i))
        })
    };
    let in_reach = victim
        .and_then(|v| map.unit_view(v))
        .zip(map.unit_view(g))
        .is_some_and(|(v, me)| combat::in_melee_range(&me, &v));
    t.effects.iter().enumerate().all(|(i, e)| match e.effect {
        6 if t.stack_amount <= 1 => {
            !held(i)
                && !(e.aura == AURA_MOD_INCREASE_SPEED && in_reach)
                && !(e.aura == AURA_MOD_STUN && stunned(map, whom))
        }
        35 => !held(i),
        _ => true,
    })
}

fn stunned(map: &Map, g: Guid) -> bool {
    map.unit_fields(g)
        .is_some_and(|f| f.get_u32(UNIT_FIELD_FLAGS) & 0x0004_0000 != 0)
}

/// How far `g`'s spell `t` reaches: its range with the modifiers its cast
/// takes, a player's pet's its master's (mangos's `GetSpellModOwner`,
/// Thistle Tea's `observation_radius` with `owner_spell_modifiers`).
fn reach(map: &Map, g: Guid, t: &crate::game::data::spells::SpellTemplate) -> f32 {
    let (_, base) = map.realm.data.spells.range(t);
    crate::game::map::spells::mods::for_spell(map, g, t)
        .apply(crate::game::map::spells::mods::OP_RANGE, base)
        .max(0.0)
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
        .filter(|t| !helpful(t))
        .filter(|t| t.has_effect(2) && t.casting_time_index > 1)
        .filter(|t| f.get_u32(UNIT_FIELD_POWER1 + t.power_type.min(4) as u16) >= t.mana_cost)
        .map(|t| reach(map, g, t))
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
        let pick = opener(map, g, victim)
            .or_else(|| pick_spell(map, g, victim).map(|(spell, _)| (spell, victim)))
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
    // A caster stands back and casts (the imp: mangos's `PetAI`, its
    // attack distance 25 yards and no melee).
    if let Some(r) = caster_range(map, g) {
        stand_off(map, g, victim, &me, &them, r, now);
        return;
    }
    // Told to stay, it chases only the victim its master's Attack named;
    // another that leaves its reach it lets go where it stands (Thistle
    // Tea's `Targeting.within_stay_range?`).
    let stays =
        state(map, g).is_some_and(|s| s.command == COMMAND_STAY && s.commanded != Some(victim));
    // In melee it chases, stands and spreads round its victim as a
    // creature does, sharing its victim's spots with every other fighting
    // it, creatures and pets alike (`ai::chase_goal`, `ai::runs_on`,
    // `ai::spread`).
    if combat::in_melee_range(&me, &them) {
        let (moving, runs_on) = crate::game::map::ai::runs_on(map, g, &me, &them);
        if !runs_on {
            stop(map, g);
            face(map, g, victim);
        }
        if !moving && let Some(c) = map.creature_mut(g) {
            c.engagement.spreading = false;
        }
        // One whose template (or a script) bars its swing holds beside
        // its victim (Thistle Tea's `melee_enabled?`).
        if crate::game::map::event_ai::melee(map, g) {
            combat::try_swing(map, g);
        }
        if !moving && !stays {
            crate::game::map::ai::spread(map, g, victim);
        }
        return;
    }
    if stays {
        stop(map, g);
        face(map, g, victim);
        return;
    }
    let reach = combat::melee_range(&me, &them);
    let stale = map
        .creature(g)
        .is_some_and(|c| crate::game::map::ai::chase_stale(c, me.position, them.position, reach));
    if !stale {
        return;
    }
    let (goal, side) =
        crate::game::map::ai::chase_goal(map, g, victim, me.position, them.position, reach);
    walk(map, g, goal, now);
    crate::game::map::ai::chases(map, g, victim, side);
    if let Some(c) = map.creature_mut(g) {
        c.chase_target_at = Some(them.position);
        c.engagement.spread_tries = 0;
        c.engagement.spreading = false;
    }
}

/// A caster's step of a fight: within `range` (less two yards) it stands
/// and faces its victim; further, it closes to four yards inside it.
fn stand_off(
    map: &mut Map,
    g: Guid,
    victim: Guid,
    me: &combat::UnitView,
    them: &combat::UnitView,
    range: f32,
    now: Instant,
) {
    if dist(me.position, them.position) <= range - 2.0 {
        stop(map, g);
        face(map, g, victim);
        return;
    }
    let stays =
        state(map, g).is_some_and(|s| s.command == COMMAND_STAY && s.commanded != Some(victim));
    if stays {
        stop(map, g);
        face(map, g, victim);
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
    let reach = (range - 4.0).max(5.0);
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

/// Out of the fight, as a player is (your call on #20, mangos's
/// `CombatManager::Update` for a player-controlled unit, Thistle Tea's
/// `PetCombat.reconcile`): once no living creature here holds it on its
/// threat list or attacks it, and its PvP timer (5 s after its last blow
/// with another player's side) has run out; looked at every
/// [`combat::COMBAT_CHECK`], and at once as that timer ends. There is no
/// timer against creatures: a foe that died, went, evaded, left its fight
/// or came back anew holds it no more. Its own threat list goes with its
/// fight.
fn calm(map: &mut Map, g: Guid, now: Instant) {
    let in_combat = map.unit_fields(g).is_some_and(|f| {
        f.has_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        )
    });
    let Some(c) = map.combat(g) else {
        return;
    };
    if !in_combat
        || c.combat_until.is_some_and(|t| t > now)
        || c.next_check.is_some_and(|t| t > now)
    {
        return;
    }
    let held = map.awake_any(|c| {
        // In a fight of its own: one out of it holds nothing
        // (mangos's references go with its combat; p10d's test).
        c.alive()
            && !c.evading
            && c.fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            )
            && ((c.combat.attacking && c.combat.victim == g)
                || c.threat.iter().any(|(t, _)| *t == g))
    });
    if let Some(c) = map.combat_mut(g) {
        c.next_check = Some(now + combat::COMBAT_CHECK);
    }
    if held {
        return;
    }
    if let Some(c) = map.creature_mut(g) {
        c.threat.clear();
    }
    combat::leave_combat(map, g);
}

/// Turn to `other`, as a creature does in a fight: its observers are told
/// with a facing move (Thistle Tea's `SetFacing` before a pet's swing).
fn face(map: &mut Map, g: Guid, other: Guid) {
    crate::game::map::ai::face(map, g, other);
}

/// Stop where its walk has come to now, its watchers told.
pub fn stop(map: &mut Map, g: Guid) {
    crate::game::map::ai::halt(map, g);
}

/// Walk (run) to `goal`, by the navmesh where there is one: to the ground
/// there (a spot beside a victim on a slope is at its height). With no way
/// over the navmesh, a straight line only where nothing stands between and
/// the ground is a step's climb or drop (open ground the bake has a gap
/// in); else no walk, and false. Before, any goal the navmesh had no way to
/// was walked straight: through the floor of Drek'Thar's room to the hall
/// under it, and through his keep's walls (the watcher's `peon::seen`: 29
/// pets' falls in 5 attempts).
fn walk(map: &mut Map, g: Guid, mut goal: Vector3d, now: Instant) -> bool {
    let Some(from) = map.position_of(g) else {
        return false;
    };
    let path = match map.nav.as_mut() {
        Some(nav) => {
            if let Some(z) = nav.ground(goal.x, goal.y, goal.z) {
                goal.z = z;
            }
            match nav.path(from, goal) {
                Some(path) => path,
                None if (goal.z - from.z).abs() <= STRAIGHT_CLIMB
                    && crate::game::map::spells::checks::line_of_sight(map, from, goal) =>
                {
                    vec![from, goal]
                }
                None => return false,
            }
        }
        None => vec![from, goal],
    };
    let spline = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    if let Some(c) = map.creature_mut(g)
        && let Some(frame) = c.walk(path, true, now, spline)
    {
        map.send_to_observers(g, &frame, false);
    }
    true
}

/// The most a pet's straight walk, with no way over the navmesh, climbs or
/// drops: a step's.
const STRAIGHT_CLIMB: f32 = 2.5;

/// Its spot at its master's side, as mangos-classic's follow picks one
/// (`FollowMovementGenerator::_getLocation`, `WorldObject::GetNearPointAt`):
/// [`FOLLOW_DISTANCE`] out at `angle`, on the ground there, no further above
/// or below its master than that and both their radii, and in its master's
/// sight; else the same at angles further round each way; none, its
/// master's own spot. Before, the spot was taken where it fell, at its
/// master's height: by a wall or a doorway, in the wall or past it, its
/// ground the hall under Drek'Thar's room or the wall's top, and the pet
/// went there.
fn follow_spot(map: &mut Map, at: Vector3d, angle: f32, radii: f32) -> Vector3d {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};
    let most = FOLLOW_DISTANCE + radii;
    for turn in [
        0.0,
        FRAC_PI_4,
        -FRAC_PI_4,
        FRAC_PI_2,
        -FRAC_PI_2,
        3.0 * FRAC_PI_4,
        -3.0 * FRAC_PI_4,
        PI,
    ] {
        let a = angle + turn;
        let mut p = Vector3d {
            x: at.x + FOLLOW_DISTANCE * a.cos(),
            y: at.y + FOLLOW_DISTANCE * a.sin(),
            z: at.z,
        };
        if let Some(z) = map.nav.as_mut().and_then(|n| n.ground(p.x, p.y, at.z)) {
            p.z = z;
        }
        if (p.z - at.z).abs() < most && crate::game::map::spells::checks::line_of_sight(map, at, p)
        {
            return p;
        }
    }
    at
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
    let radius = |x: Guid| {
        map.unit_fields(x)
            .map(|f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS))
            .filter(|r| *r > 0.0)
            .unwrap_or(0.389)
    };
    let radii = radius(g) + radius(owner);
    let spot = follow_spot(map, o.position, o.orientation + angle, radii);
    let d = dist(pos, spot);
    if d > TELEPORT_DISTANCE {
        jump(map, g, spot, o.orientation);
        back(map, g);
        return;
    }
    if d <= FOLLOW_DISTANCE * 1.25 {
        back(map, g);
        return;
    }
    // Beside a master who stands, it stays, however they turn (Thistle
    // Tea's `Follow.step` settling by a leader at rest).
    let owner_moving = match map.players.get(&owner) {
        Some(p) => crate::game::map::spells::checks::moving(&p.movement.flags),
        None => map
            .creature(owner)
            .is_some_and(|c| matches!(c.motion, Motion::Moving(_))),
    };
    let slack = 1.4 * FOLLOW_DISTANCE + radii;
    if !owner_moving && destination(map, g).is_none() && dist(pos, o.position) <= slack {
        back(map, g);
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
    // No way there: put by its master at once, as mangos-classic's follow
    // unsticks one (`FollowMovementGenerator::_setTargetLocation`: no path,
    // relocated to its spot, which its master sees).
    if !walk(map, g, spot, now) {
        jump(map, g, spot, o.orientation);
        back(map, g);
    }
}

/// Back at its master's side: a recall is over.
fn back(map: &mut Map, g: Guid) {
    if let Some(s) = state_mut(map, g) {
        s.returning = false;
    }
}

/// Put the pet at `at` at once.
fn jump(map: &mut Map, g: Guid, at: Vector3d, orientation: f32) {
    if let Some(from) = map.recording().then(|| map.position_of(g)).flatten() {
        let on = map.key.map;
        map.record(|| crate::game::map::record::Ev::Tele {
            g,
            from,
            to: at,
            map: on,
        });
    }
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
    map.uncache_create(g);
    let frame = crate::game::map::ai::stop_packet(g, at, spline);
    map.send_to_observers(g, &frame, false);
}

/// How often a controlled creature's health comes back: every four
/// seconds for a pet or a guardian (mangos's `Pet::RegenerateAll`), five
/// for a charmed creature (`Creature::RegenerateAll`, `REGEN_TIME_FULL_UNIT`).
pub fn health_interval(kind: Kind) -> Duration {
    match kind {
        Kind::Charmed | Kind::Possessed => Duration::from_secs(5),
        _ => Duration::from_secs(4),
    }
}

/// Health and power come back: health out of a fight, a demon's mana out
/// of a fight, a hunter's pet's focus all the time. Health by its kind, as
/// mangos's: a hunter's or a summoned pet's four times its spirit's due
/// (`Pet::RegenerateHealth`), a guardian a twentieth, a charmed creature
/// 13% (`Creature::RegenerateHealth`, player controlled).
pub fn regen(map: &mut Map, g: Guid, now: Instant) {
    let Some((regen_at, health_at, kind)) =
        state(map, g).map(|s| (s.regen_at, s.health_at, s.kind))
    else {
        return;
    };
    if regen_at > now && health_at > now {
        return;
    }
    let kind = Some(kind);
    let in_combat = map.unit_fields(g).is_some_and(|f| {
        f.has_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        )
    });
    let polymorphed = crate::game::map::creature_combat::polymorphed(map, g);
    let rate = map.cfg.rates.health;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    // Health on a clock of its own (four seconds, a charmed creature's
    // five), power every two.
    let (mut health_due, mut power_due) = (false, false);
    if let Some(s) = c.pet.as_deref_mut() {
        if s.regen_at <= now {
            s.regen_at = now + Duration::from_secs(2);
            power_due = true;
        }
        if s.health_at <= now {
            s.health_at = now + health_interval(s.kind);
            health_due = true;
        }
    }
    let f = &mut c.fields;
    // Its health every four seconds, out of a fight or polymorphed in one:
    // a tenth polymorphed (Thistle Tea's controlled creature's); else a
    // pet or a charmed creature from its spirit as a player of its class,
    // four seconds' worth (mangos's `Pet::RegenerateHealth`:
    // `OCTRegenHPPerSpirit() * 4`, Thistle Tea's
    // `controlled_health_per_tick`); a guardian or a totem a twentieth.
    if health_due && (!in_combat || polymorphed) {
        let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
        let h = f.get_u32(UNIT_FIELD_HEALTH);
        let add = if polymorphed {
            max / 10
        } else if matches!(kind, Some(Kind::Charmed | Kind::Possessed)) {
            // A charmed creature heals as a creature, 13% every five seconds
            // (mangos's `Creature::RegenerateHealth`; your #40).
            (max as f32 * 0.13) as u32
        } else if kind.is_some_and(Kind::controlled) {
            let class = f.get_byte(UNIT_FIELD_BYTES_0, 1);
            let spirit = f.get_u32(UNIT_FIELD_STAT0 + 4) as f32;
            // Truncated once, after the rate and the four (mangos's
            // `OCTRegenHPPerSpirit() * HealthIncreaseRate * 4`).
            (crate::game::map::stats::health_regen(class, spirit) * rate * 4.0) as u32
        } else {
            (max / 20).max(1)
        };
        f.set_u32(UNIT_FIELD_HEALTH, (h + add).min(max));
    }
    if !power_due {
        return;
    }
    if !in_combat {
        let mana_max = f.get_u32(UNIT_FIELD_MAXPOWER1);
        let mana = f.get_u32(UNIT_FIELD_POWER1);
        f.set_u32(
            UNIT_FIELD_POWER1,
            (mana + (mana_max / 20).max(1)).min(mana_max),
        );
    }
    if kind == Some(Kind::Hunter) {
        // vmangos's focus: 24 every four seconds, as fast as the percent
        // auras for focus say (mangos's `Creature::RegeneratePower`).
        let rate = c.auras.product_misc(
            crate::game::map::auras::AURA_MOD_POWER_REGEN_PERCENT,
            POWER_FOCUS as i32,
        );
        let f = &mut c.fields;
        let max = f.get_u32(UNIT_FIELD_MAXPOWER1 + POWER_FOCUS);
        let focus = f.get_u32(UNIT_FIELD_POWER1 + POWER_FOCUS);
        let add = (12.0 * rate) as u32;
        f.set_u32(UNIT_FIELD_POWER1 + POWER_FOCUS, (focus + add).min(max));
    }
}
