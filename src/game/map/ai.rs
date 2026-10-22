//! Creature AI: noticing a hostile player, threat, chasing on the navmesh,
//! leashing home, calling for help, death, corpse decay and respawn.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::items::Body;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::combat::{self, dist};
use crate::game::map::creature::{Life, Motion};
use crate::game::map::death::Death;
use crate::game::map::player::{PLAYER_FLAGS_GM, UNIT_FLAG_IN_COMBAT};

/// How often an idle hostile creature looks for players.
const AGGRO_SCAN_EVERY: Duration = Duration::from_millis(500);
/// The shortest wait before a creature respawns.
const MIN_RESPAWN: Duration = Duration::from_secs(5);
/// `UNIT_DYNAMIC_FLAGS`: the creature is tapped by someone else.
pub const DYNFLAG_TAPPED: u32 = 0x4;

pub const SMSG_MONSTER_MOVE: u16 = 0x0DD;

/// A short `SMSG_MONSTER_MOVE`: the creature stops where it is.
pub fn stop_packet(g: Guid, at: Vector3d, spline_id: u32) -> Arc<[u8]> {
    let mut b = Body::default();
    crate::game::update::write_packed_guid(&mut b.0, g);
    b.f32(at.x).f32(at.y).f32(at.z).u32(spline_id).u8(1);
    b.frame(SMSG_MONSTER_MOVE)
}

/// A unit turning on the spot to `angle` (a facing-angle monster move of
/// 100 ms, as Thistle Tea turns one); a player's client turns itself by
/// one for its own guid.
pub fn face_packet(g: Guid, at: Vector3d, angle: f32, spline_id: u32) -> Arc<[u8]> {
    let mut b = Body::default();
    crate::game::update::write_packed_guid(&mut b.0, g);
    b.f32(at.x)
        .f32(at.y)
        .f32(at.z)
        .u32(spline_id)
        .u8(4)
        .f32(angle);
    b.u32(0).u32(100).u32(1).f32(at.x).f32(at.y).f32(at.z);
    b.frame(SMSG_MONSTER_MOVE)
}

/// A creature turning on the spot to face `target` (Thistle Tea's
/// `set_facing({:target, _})`: a facing-target monster move of 100 ms).
pub fn face_target_packet(g: Guid, at: Vector3d, target: Guid, spline_id: u32) -> Arc<[u8]> {
    let mut b = Body::default();
    crate::game::update::write_packed_guid(&mut b.0, g);
    b.f32(at.x).f32(at.y).f32(at.z).u32(spline_id).u8(3);
    b.u64(target.guid());
    b.u32(0).u32(100).u32(1).f32(at.x).f32(at.y).f32(at.z);
    b.frame(SMSG_MONSTER_MOVE)
}

/// The radius at which a creature notices a unit (vmangos's
/// `GetAttackDistance`): its detection range at an even level, a yard less
/// for each level the unit is above it (more for up to 25 below), at least
/// five yards (or its range, if less); none with a range under a yard.
pub fn aggro_radius(detection: f32, creature_level: u32, player_level: u32) -> f32 {
    aggro_reach(detection, creature_level, player_level, 0.0)
}

/// `aggro_radius` with the creature's own detect-range auras (Mind Soothe).
pub fn aggro_reach(detection: f32, creature_level: u32, unit_level: u32, modifier: f32) -> f32 {
    if detection < 1.0 {
        return 0.0;
    }
    let diff = (unit_level as i32 - creature_level as i32).max(-25);
    (detection - diff as f32 + modifier).max(detection.min(5.0))
}

/// How near `t` `g` notices it (Thistle Tea's `Acquisition.radius`): its
/// aggro radius at `t`'s level (a pet's, its master's), with its own
/// detect-range auras, at the server's rate (`[creatures] aggro_rate`).
pub fn attack_distance(map: &Map, g: Guid, t: Guid) -> f32 {
    use crate::game::map::auras::AURA_MOD_DETECT_RANGE;
    let Some(c) = map.creature(g) else {
        return 0.0;
    };
    let detection = map
        .realm
        .data
        .creature_template(c.entry)
        .map_or(20.0, |t| t.detection_range);
    let modifier = c.auras.sum(AURA_MOD_DETECT_RANGE) as f32;
    let level = c.fields.get_u32(UNIT_FIELD_LEVEL);
    let master = crate::game::map::pets::master(map, t);
    let who = if map.players.contains_key(&master) {
        master
    } else {
        t
    };
    let lv = map
        .unit_fields(who)
        .map_or(1, |f| f.get_u32(UNIT_FIELD_LEVEL));
    // A player's pet no client moves is noticed from nearer (mangos-
    // classic's `GetAttackDistance`: "player pets do not aggro from so
    // afar").
    let pet =
        who != t && guid::is_unit(t) && crate::game::map::movement::controller(map, t).is_none();
    let pet_rate = if pet { PET_AGGRO_RATE } else { 1.0 };
    aggro_reach(detection, level, lv, modifier) * pet_rate * map.cfg.creatures.aggro_rate
}

/// How much nearer a player's pet is noticed than its master.
const PET_AGGRO_RATE: f32 = 0.65;

/// The unit flags that put a unit out of any fight: not attackable (both),
/// not selectable, on a flight.
const UNTARGETABLE: u32 = 0x2 | 0x1_0000 | 0x0200_0000 | 0x0010_0000;

/// `creature` was attacked by `attacker`: it fights back, and calls for
/// help.
pub fn attacked(map: &mut Map, creature: Guid, attacker: Guid) {
    engage(map, creature, attacker, true);
}

/// `creature` comes to another's help against `enemy`: it fights, calling
/// no one itself.
pub fn assist(map: &mut Map, creature: Guid, enemy: Guid) {
    engage(map, creature, enemy, false);
}

fn engage(map: &mut Map, creature: Guid, attacker: Guid, call: bool) {
    if crate::game::map::pets::on_attacked(map, creature, attacker) {
        return;
    }
    let now = map.now;
    let Some(c) = map.creature_mut(creature) else {
        return;
    };
    if !c.alive() || c.evading {
        return;
    }
    // The player it fights is in the fight too, as it begins and as it
    // goes on (mangos's `EngageInCombatWith`, `SetInCombatWithVictim`).
    // Not a pet's master who can't join it, though (lying in a believed
    // Feign Death, out of every fight: [`pets::drawn_in`]).
    let master = crate::game::map::pets::master(map, attacker);
    if guid::is_player(master)
        && (master == attacker || crate::game::map::pets::drawn_in(map, attacker) == Some(master))
    {
        combat::enter(map, master, false);
    }
    // A fight taps no one: the first damage does ([`tap::damaged`]).
    let Some(c) = map.creature_mut(creature) else {
        return;
    };
    let new = c.threat.iter().all(|(g, _)| *g != attacker);
    if new {
        c.threat.push((attacker, 0.0));
    }
    let attacking = c.combat.attacking;
    if new && guid::is_player(attacker) {
        crate::game::map::reputation::temporary_war(map, attacker, creature);
    }
    // A critter runs, from each hit anew.
    let critter = crate::game::map::creature_motion::critter(map, creature);
    if critter {
        crate::game::map::creature_motion::critter_hit(map, creature, attacker);
    }
    let passive = !critter
        && crate::game::map::assistance::reaction(map, creature)
            == crate::game::map::assistance::Reaction::Passive;
    map.note_fighting(creature);
    let Some(c) = map.creature_mut(creature) else {
        return;
    };
    if attacking {
        return;
    }
    let fresh = !c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT);
    let first = c.combat_start.is_none() || fresh;
    // A passive one is in the fight, but takes no victim and calls no one.
    if passive && c.owner.is_none() {
        if first {
            c.combat_start = Some(now);
            c.engagement.origin = Some(c.position);
        }
        c.engagement.last_hostile = Some(now);
        c.fields
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, true);
        if fresh {
            crate::game::map::spells::combat_began(map, creature);
        }
        if first {
            crate::game::map::creature_combat::leash_begins(map, creature);
            crate::game::map::creature_combat::stand_up(map, creature);
            crate::game::map::event_ai::aggro(map, creature, attacker);
        }
        return;
    }
    // A fight's start and its leash's origin are where it began, not
    // where it turned to another (its victim feigning death, vanishing).
    if first {
        c.combat_start = Some(now);
        c.engagement.origin = Some(c.position);
    }
    c.engagement.last_hostile = Some(now);
    c.combat.victim = attacker;
    c.combat.attacking = true;
    c.combat.next_swing.get_or_insert(now);
    // A critter runs with no one selected (Thistle Tea's `target: 0`).
    if !critter {
        c.fields.set_guid(UNIT_FIELD_TARGET, attacker);
    }
    c.fields
        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, true);
    // What a fight's start stops: a cast not for one (mangos's
    // `SetInCombatState`, for every unit).
    if fresh {
        crate::game::map::spells::combat_began(map, creature);
    }
    if first {
        crate::game::map::creature_combat::leash_begins(map, creature);
        crate::game::map::creature_combat::stand_up(map, creature);
    }
    // Its SMSG_ATTACKSTART waits for its first swing in reach (`announce`).
    // A critter calls no one, and fights no one.
    if critter {
        return;
    }
    if call {
        crate::game::map::assistance::drawn_in(map, creature, attacker);
    }
    if first {
        crate::game::map::event_ai::aggro(map, creature, attacker);
        crate::game::map::creature_combat::entered(map, creature, attacker);
    }
    crate::game::map::guards::attack_started(map, creature, attacker);
}

/// Damage from `from` raises its threat on `creature`; a cut (Feint)
/// lowers it, never below none, and puts no one on the list. A creature
/// holds none on itself (mangos's `ThreatManager::addThreat` and
/// `HostileReference::addThreat`, Thistle Tea's `Threat.add` and
/// `Threat.change`).
pub fn add_threat(map: &mut Map, creature: Guid, from: Guid, amount: f32) {
    // An object's spell (a script's cast from it) makes it no foe: it is
    // never on a threat list (Thistle Tea's object damage, no combat
    // opponent).
    if from == creature || from.is_zero() || guid::is_game_object(from) {
        return;
    }
    let Some(c) = map.creature_mut(creature).filter(|c| keeps_threat(c)) else {
        return;
    };
    match c.threat.iter_mut().find(|(g, _)| *g == from) {
        Some((_, t)) => *t = (*t + amount).max(0.0),
        None if amount < 0.0 => {}
        None => {
            c.threat.push((from, amount));
            map.note_fighting(creature);
            if guid::is_player(from) {
                crate::game::map::reputation::temporary_war(map, from, creature);
            }
        }
    }
}

/// Whether a creature keeps a threat list: not a pet, totem, guardian or
/// charmed one, whose own AI picks its foes from those fighting it and
/// its master, and which no fight's turn would ever prune (mangos's
/// `Unit::CanHaveThreatList`, which keeps one for a guardian, whose AI
/// there reads it). On one, whoever struck it was held in their fight
/// while both lived.
pub fn keeps_threat(c: &crate::game::map::creature::Creature) -> bool {
    use crate::game::map::pets::Kind;
    // A creature's own pet or guardian keeps one, and fights by it
    // (mangos's `CanHaveThreatList`: "unless they are controlled by a
    // creature"; Thistle Tea's NPC pets' threat order).
    c.pet
        .as_deref()
        .is_none_or(|s| !guid::is_player(s.owner) && !matches!(s.kind, Kind::Totem | Kind::Critter))
}

/// Creatures forget `g` (feigning death, vanishing): its threat goes, and
/// one fighting it turns to the next on its list. A pet fighting it, with
/// no list, stops too (mangos's `CombatStop`, its `RemoveAllAttackers`).
pub fn forget(map: &mut Map, g: Guid) {
    let fighting: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| {
            c.threat.iter().any(|(t, _)| *t == g)
                || (!keeps_threat(c) && c.combat.attacking && c.combat.victim == g)
        })
        .map(|(_, c)| c.guid)
        .collect();
    forget_by(map, g, fighting);
}

/// [`forget`], by the creatures `fighting` alone.
pub fn forget_by(map: &mut Map, g: Guid, fighting: Vec<Guid>) {
    for c in fighting {
        crate::game::map::spells::threat_sync::removed(map, c, g);
        {
            let Some(cr) = map.creature_mut(c) else {
                continue;
            };
            // Its tap stays: a creature's loot recipient goes only as it
            // evades or respawns (mangos-classic's `TriggerEvadeEvents`,
            // `Creature::Respawn`; Thistle Tea's `Engagement.leave`), not
            // as its tapper dies, feigns or vanishes and it fights on.
            cr.threat.retain(|(t, _)| *t != g);
            if cr.combat.victim != g {
                continue;
            }
        }
        turn_to_next(map, c);
    }
}

/// `g`'s victim is gone from its list: it fights the next it may (not one
/// it can't reach or see), or, with none, its fight is over.
fn turn_to_next(map: &mut Map, c: Guid) {
    let next = pick_victim(map, c, false);
    combat::stop_attack(map, c);
    let pet = map.creature(c).is_some_and(|cr| cr.pet.is_some());
    match next {
        // A pet takes up the next itself (its own AI's choice); with
        // none, its fight is over where it stands, whole as it is
        // (Thistle Tea's `drop_threat` for a pet): no evade home.
        Some(_) if pet => {}
        None if pet => pet_fight_over(map, c),
        Some(t) => {
            if let Some(cr) = map.creature_mut(c) {
                cr.combat.attacking = false;
            }
            // It called for help as its fight began, and calls no one
            // again for a new victim in it (mangos-classic's
            // `m_AlreadyCallAssistance`, reset only at `CombatStop`).
            assist(map, c, t);
            map.note_fighting(c);
            if let Some(cr) = map.creature_mut(c) {
                cr.combat.victim = t;
                cr.combat.attacking = true;
                cr.fields.set_guid(UNIT_FIELD_TARGET, t);
            }
        }
        None => evade(map, c),
    }
}

/// `g` gives up on `victim`, gone out of their sight
/// ([`creature_combat::out_of_sight`](crate::game::map::creature_combat::out_of_sight)):
/// they come off its list (its tap kept, if it fights on), and, if it was
/// fighting them, it turns to the next, or goes home.
pub fn lost_sight_of(map: &mut Map, g: Guid, victim: Guid) {
    crate::game::map::spells::threat_sync::removed(map, g, victim);
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.threat.retain(|(t, _)| *t != victim);
    if c.combat.victim == victim {
        turn_to_next(map, g);
    }
}

/// `g`'s victim died (its attack stopped with it): a creature fights on
/// with the next on its list it may fight, as mangos's
/// `SelectHostileTarget` has it at its next update (one that killed a pet
/// turns on its master, his entry on its list). With none, its next turn
/// sees its fight over. A pet's own AI takes up its next.
pub fn victim_died(map: &mut Map, g: Guid) {
    if !map
        .creature(g)
        .is_some_and(|c| c.pet.is_none() && c.alive() && !c.combat.attacking && !c.evading)
    {
        return;
    }
    if let Some(t) = pick_victim(map, g, false) {
        assist(map, g, t);
    }
}

/// A pet's last foe gone: it leaves the fight where it is, its health and
/// auras as they are, its master's Attack over; it follows or stays as it
/// was told.
fn pet_fight_over(map: &mut Map, g: Guid) {
    if let Some(c) = map.creature_mut(g) {
        c.threat.clear();
        c.chase_target_at = None;
        c.fields.set_guid(UNIT_FIELD_TARGET, Guid::zero());
        if let Some(s) = c.pet.as_deref_mut() {
            s.commanded = None;
        }
    }
    combat::leave_combat(map, g);
}

/// A taunt: `by` is `creature`'s victim while the taunt lasts, and level
/// with its old target's threat meanwhile.
pub fn taunted(map: &mut Map, creature: Guid, by: Guid) {
    // One with no list isn't taunted (mangos's `TauntApply`).
    let Some(c) = map.creature_mut(creature).filter(|c| keeps_threat(c)) else {
        return;
    };
    // On the threat list, and borrowing what brings it level with the
    // creature's target for the taunt (Thistle Tea's `apply_taunt`).
    if !c.threat.iter().any(|(g, _)| *g == by) {
        c.threat.push((by, 0.0));
        map.note_fighting(creature);
    }
    crate::game::map::spells::threat_sync::taunted(map, creature, by);
    let Some(c) = map.creature_mut(creature) else {
        return;
    };
    let switched = c.combat.attacking && c.combat.victim != by;
    c.combat.victim = by;
    c.fields.set_guid(UNIT_FIELD_TARGET, by);
    attacked(map, creature, by);
    if switched {
        crate::game::map::guards::attack_started(map, creature, by);
    }
}

/// A taunt ended: what it borrowed goes back, and the victim is the
/// threat's choice again, or an older taunt's (`taunter`).
pub fn taunt_ended(map: &mut Map, creature: Guid, by: Guid) {
    crate::game::map::spells::threat_sync::taunt_ended(map, creature, by);
}

/// How long spawn `spawn` waits to come back: its time, rolled in its
/// range (five minutes for none), at `rate` (`[rates] respawn_time`), at
/// least five seconds.
pub fn respawn_delay(
    data: &crate::game::data::GameData,
    spawn: Option<u32>,
    rate: f32,
) -> Duration {
    spawn
        .and_then(|g| data.creature_spawn(g))
        .map(|s| {
            let (lo, hi) = (s.respawn_min, s.respawn_max.max(s.respawn_min));
            let secs = if hi > lo {
                rand::rng().random_range(lo..=hi)
            } else {
                lo
            };
            Duration::from_secs(u64::from(secs))
        })
        .unwrap_or(Duration::from_secs(300))
        .mul_f32(rate)
        .max(MIN_RESPAWN)
}

/// A creature dies.
pub fn die(map: &mut Map, creature: Guid, tapper: Guid) {
    die_by(map, creature, tapper, tapper);
}

/// It died to `killer`, `tapper` holding its tap. Its scripts run on to
/// its corpse's end, as Thistle Tea's do.
pub fn die_by(map: &mut Map, creature: Guid, tapper: Guid, killer: Guid) {
    die_rewarding(map, creature, tapper, killer, true);
}

/// [`die_by`]; with `reputation` false, its kill is worth no standing to
/// anyone (one a player controlled: Thistle Tea's `KillCredit`).
pub fn die_rewarding(map: &mut Map, creature: Guid, tapper: Guid, killer: Guid, reputation: bool) {
    // Who a raid's bind is for, as Thistle Tea's `raid_killer` has it: the
    // killer, or the player whose pet it is, or, for one that killed
    // itself, who tapped it. Only a player binds.
    let binder = if killer == creature {
        tapper
    } else {
        crate::game::map::pets::master(map, killer)
    };
    if reputation && guid::is_player(tapper) {
        crate::game::map::reputation::creature_killed(map, tapper, creature);
    }
    // Honor sees the auras it died with (Honorless Target), before death
    // takes them.
    crate::game::map::pvp::creature_killed(map, tapper, creature);
    crate::game::map::auras::clear_on_death(map, creature);
    crate::game::map::event_ai::death(map, creature, killer);
    crate::game::map::event_ai::combat_stopped(map, creature, true);
    crate::game::map::instance_script::creature_event(
        map,
        creature,
        crate::game::map::instance_script::Event::Death,
    );
    if let Some((entry, at)) = map.creature(creature).map(|c| (c.entry, c.position)) {
        let camps_shard = crate::game::map::necrotic_camps::died(map, creature, entry);
        crate::game::invasion::creature_died(map, entry, at, camps_shard);
        crate::game::champions::creature_died(map, creature);
        crate::game::elemental::creature_died(map, creature, entry);
        crate::game::ghost_magnet::creature_died(map, creature, entry);
        crate::game::omen::creature_died(map, entry);
    }
    let now = map.now;
    let data = map.realm.data.clone();
    let spline_id = map.next_spline_id;
    let cfg = map.cfg.clone();
    crate::game::map::spells::threat_sync::wiped(map, creature);
    // A run stops where it has come to now, not where the map's last
    // update left it (mangos's `MoveSplineInit::Stop` computes the
    // spline's position): its watchers' clients would have it slide back.
    advance_motion(map, creature);
    let Some(c) = map.creature_mut(creature) else {
        return;
    };
    let template = data.creature_template(c.entry);
    // How long its corpse lies, by its rank (Thistle Tea's
    // `@corpse_decay_ms`, an elite's twice a normal's; mangos's
    // `Corpse.Decay.*`).
    let decay = cfg.creatures.corpse_decay(template.map_or(0, |t| t.rank));
    let respawn = respawn_delay(&data, c.spawn_guid, cfg.rates.respawn_time);
    // Sooner in a busy pocket of a starting area (`[hyperspawn]`).
    let (respawn, corpse) =
        crate::game::map::hyperspawn::creature_died(map, creature, respawn, decay.min(respawn));
    let Some(c) = map.creature_mut(creature) else {
        return;
    };
    // Its corpse lies its decay, but no longer than until it comes back.
    c.life = Life::Dead {
        corpse_until: now + corpse,
        respawn_at: now + respawn,
    };
    let moving = matches!(c.motion, Motion::Moving(_));
    c.motion = Motion::Idle { until: None };
    c.threat.clear();
    c.combat = Default::default();
    c.evading = false;
    c.fields.set_u32(UNIT_FIELD_HEALTH, 0);
    c.fields.set_guid(UNIT_FIELD_TARGET, Guid::zero());
    c.fields
        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, false);
    // A corpse is no one's quest giver, vendor or trainer: its NPC flags
    // go, and its respawn has its template's (mangos's
    // `Creature::SetDeathState`). A dead quest giver kept its "!".
    c.fields.set_u32(UNIT_NPC_FLAGS, 0);
    // A run ends with its death: a critter's escape, a flight for help
    // (Thistle Tea's `Critter.clear` and `flee_until` at death).
    c.engagement.flee = None;
    c.engagement.escape_at = None;
    c.fields.set_flag(
        UNIT_FIELD_FLAGS,
        crate::game::map::auras::UNIT_FLAG_FLEEING,
        false,
    );
    // The kill is its tapper's, for the group it was tapped for; another's
    // (the tap not theirs) for their group now.
    if c.tapped_by != Some(tapper) {
        let realm = map.realm.clone();
        let Some(c) = map.creature_mut(creature) else {
            return;
        };
        crate::game::map::tap::take(&realm, c, tapper);
    }
    let Some(c) = map.creature_mut(creature) else {
        return;
    };
    let pos = c.position;
    let (spawn, locks_raid) = (
        c.spawn_guid,
        template.is_some_and(|t| {
            t.static_flags2 & crate::game::instances::STATIC_FLAG2_LOCKS_RAID != 0
        }),
    );
    let boss = template.is_some_and(|t| t.rank == 3) || locks_raid;
    map.dead_creatures.push(creature);
    if let Some(spawn) = spawn {
        // A boss in a copy never comes back; one in the open world (a
        // world boss) at its time.
        let never = boss && map.key.instance != 0;
        crate::game::instances::spawn_died(&map.realm, map.key, spawn, (!never).then_some(respawn));
    }
    if locks_raid && guid::is_player(binder) {
        crate::game::instances::bind_raid(map);
    }
    if moving {
        map.next_spline_id = map.next_spline_id.wrapping_add(1);
        let frame = stop_packet(creature, pos, spline_id);
        map.send_to_observers(creature, &frame, false);
    }
    // A flyer falls to the ground; a formation it led goes on without it.
    crate::game::map::creature_motion::fall_dead(map, creature);
    crate::game::map::creature_groups::died(map, creature);
    crate::game::map::event_ai::death_heard(map, creature);
    crate::game::map::loot::creature_died(map, creature, tapper);
    crate::game::map::battleground::creature_died(map, creature, tapper);
    // A summon of a type that vanishes at death goes now.
    crate::game::map::event_ai::summon_died(map, creature);
}

/// Corpses that decay and creatures that respawn.
pub fn update_dead(map: &mut Map) {
    if map.dead_creatures.is_empty() {
        return;
    }
    let now = map.now;
    let dead = std::mem::take(&mut map.dead_creatures);
    let mut still = Vec::with_capacity(dead.len());
    for g in dead {
        // A flyer's corpse falling.
        crate::game::map::creature_motion::fall_on(map, g);
        let Some(c) = map.creature_mut(g) else {
            continue;
        };
        let Life::Dead {
            corpse_until,
            respawn_at,
        } = c.life
        else {
            continue;
        };
        if respawn_at <= now && crate::game::map::hyperspawn::respawn_due(map, g, corpse_until) {
            respawn(map, g);
            continue;
        }
        let Some(c) = map.creature_mut(g) else {
            continue;
        };
        if corpse_until <= now && !c.despawned {
            c.despawned = true;
            let (entry, summoner) = (c.entry, c.scripted.as_deref().and_then(|s| s.summoner));
            despawn_from_view(map, g);
            crate::game::map::scripts::stop_for(map, g);
            // A summon's corpse gone, its summoner hears it has gone.
            if let Some(s) = summoner {
                crate::game::map::event_ai::summoned_despawned(map, s, g, entry);
            }
        }
        still.push(g);
    }
    map.dead_creatures.extend(still);
}

/// Take a creature out of every client's view and out of the grid; it
/// stays in the map until it respawns. A refresh it was marked for goes
/// too: out of the grid, it would be taken whole and introduced again.
fn despawn_from_view(map: &mut Map, g: Guid) {
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let (cell, entry) = (c.cell, c.entry);
    let observers: Vec<Guid> = c.observers.drain().collect();
    map.grid.remove(g, cell);
    map.visibility_dirty.remove(&g);
    crate::game::omen::left(map, g, entry);
    for o in observers {
        if let Some(p) = map.players.get_mut(&o) {
            p.known.remove(&g);
            map.forget.entry(o).or_default().push(g);
        }
    }
}

/// Bring a dead creature back at its spawn, as new.
fn respawn(map: &mut Map, g: Guid) {
    let Some(old) = map.creature(g) else {
        return;
    };
    let spawn_guid = old.spawn_guid;
    let despawned = old.despawned;
    let (entry, summoner) = (old.entry, old.scripted.as_deref().and_then(|s| s.summoner));
    // A faction a script set stays over the respawn, unless its flags
    // say the respawn ends it.
    let faction = old
        .scripted
        .as_deref()
        .and_then(|s| s.temp_faction)
        .filter(|t| t.flags & crate::game::map::event_ai::FACTION_RESTORE_RESPAWN == 0);
    // So does a reaction a script set (Thistle Tea's `CreatureReaction`
    // over `Mob.respawn`; mangos's AI keeps its react state).
    let reaction = old
        .scripted
        .as_deref()
        .and_then(|s| s.reaction.map(|r| (r, s.passive)));
    let data = map.realm.data.clone();
    let spawn = spawn_guid.and_then(|sg| data.creature_spawn_in(map.key.map, sg));
    if !despawned {
        despawn_from_view(map, g);
        if let Some(s) = summoner {
            crate::game::map::event_ai::summoned_despawned(map, s, g, entry);
        }
    }
    crate::game::map::scripts::stop_for(map, g);
    if let Some(c) = map
        .creature_by_guid
        .remove(&g)
        .and_then(|k| map.creatures.try_remove(k))
    {
        drop(c);
    }
    crate::game::map::loot::forget_pockets(map, g);
    let Some(spawn) = spawn else {
        return;
    };
    // A battleground's creature whose event has gone stays gone.
    if !crate::game::map::battleground::spawn_allowed(map, true, spawn.guid) {
        return;
    }
    // A pooled spawn's pool picks again: it, or others of the pool, come.
    use crate::game::map::pools::{self, Member};
    let back = match pools::came_back(map, Member::Creature(spawn.guid)) {
        Some(picked) => {
            crate::game::instances::spawn_back(&map.realm, map.key, spawn.guid);
            picked
        }
        None => vec![Member::Creature(spawn.guid)],
    };
    for m in back {
        match m {
            // The same spawn back keeps the faction a script left it.
            Member::Creature(s) => {
                let same = s == spawn.guid;
                spawn_creature_with(map, s, faction.filter(|_| same));
                // A follower comes back in its place by its leader, as it
                // now stands (or by its spawn, it dead), not at its own
                // spawn (Thistle Tea's `Formation.respawn_position`).
                if let Some(g) = crate::game::map::scripts::creature_of_spawn(map, s)
                    && let Some((at, o)) = crate::game::map::creature_groups::respawn_place(map, g)
                {
                    crate::game::map::scripts::place(map, g, at, o);
                }
                if same
                    && let Some((r, passive)) = reaction
                    && let Some(g) = crate::game::map::scripts::creature_of_spawn(map, s)
                {
                    let hostile = map.creature(g).is_some_and(|c| c.hostile_to_players);
                    if let Some(sc) = crate::game::map::event_ai::scripted_mut(map, g) {
                        sc.reaction = Some(r);
                        sc.passive = passive;
                    }
                    // As the script's command left it (scripts.rs, 59).
                    use crate::game::map::assistance::Reaction;
                    if let Some(c) = map.creature_mut(g) {
                        c.hostile_to_players = match r {
                            Reaction::Passive => false,
                            Reaction::Defensive => hostile,
                            Reaction::Aggressive => true,
                        };
                    }
                }
            }
            Member::Object(s) => pools::spawn_object(map, s),
            Member::Pool(_) => {}
        }
    }
}

/// A spawn's creature comes into the map anew, unless a battleground or a
/// link bars it.
pub fn spawn_creature_of(map: &mut Map, spawn_guid: u32) {
    spawn_creature_with(map, spawn_guid, None);
}

/// [`spawn_creature_of`], in a faction a script set it.
fn spawn_creature_with(
    map: &mut Map,
    spawn_guid: u32,
    faction: Option<crate::game::map::event_ai::TempFaction>,
) {
    let data = map.realm.data.clone();
    let Some(spawn) = data.creature_spawn_in(map.key.map, spawn_guid) else {
        return;
    };
    if !crate::game::map::battleground::spawn_allowed(map, true, spawn.guid) {
        return;
    }
    crate::game::instances::spawn_back(&map.realm, map.key, spawn.guid);
    if !crate::game::map::creature_groups::may_spawn(map, spawn.guid) {
        map.held_slaves.insert(spawn.guid);
        return;
    }
    map.held_slaves.remove(&spawn.guid);
    let on = crate::game::events::current(&map.realm);
    if let Some(c) = crate::game::events::spawn_creature(&data, spawn, &on, &mut rand::rng()) {
        let g = map.add_creature(c);
        lie_dead_if_spawned_so(map, g, spawn);
        crate::game::events::came_in(map, g);
        if let Some(mut t) = faction {
            // Cleared, it is its respawned self's again (mangos's
            // `ClearTemporaryFaction`, the template's): not the faction it
            // had under an entry a script changed.
            if let Some(c) = map.creature(g) {
                t.original = c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE);
            }
            if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
                s.temp_faction = Some(t);
            }
            crate::game::map::scripts::set_faction(map, g, t.value);
        }
        crate::game::map::creature_groups::respawned(map, g);
    }
}

/// A spawn flagged dead (`SPAWN_FLAG_DEAD`) comes out a corpse, its death
/// settled: nothing on it, no respawn of its own, until a spell brings it
/// back (Thistle Tea's `Mob.spawn_dead`, mangos-zero's dead by default).
pub fn lie_dead_if_spawned_so(map: &mut Map, g: Guid, spawn: &crate::game::data::CreatureSpawn) {
    if spawn.spawn_flags & crate::game::data::SPAWN_FLAG_DEAD == 0 {
        return;
    }
    let now = map.now;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let never = now + Duration::from_secs(10 * 365 * 86_400);
    c.life = Life::Dead {
        corpse_until: never,
        respawn_at: never,
    };
    c.fields.set_u32(UNIT_FIELD_HEALTH, 0);
    c.fields.set_u32(UNIT_NPC_FLAGS, 0);
    map.dead_creatures.push(g);
}

/// A dead creature (or one never spawned, like a pooled spawn's other
/// pick) comes back at once.
pub fn respawn_now(map: &mut Map, g: Guid) {
    respawn_forced(map, g, false);
}

/// A spawned creature comes back new at once: a dead one, or (`even_alive`)
/// one alive too, as Thistle Tea's `Respawn.force`.
pub fn respawn_forced(map: &mut Map, g: Guid, even_alive: bool) {
    let Some(c) = map.creature(g) else {
        return;
    };
    if c.alive() && !(even_alive && c.spawn_guid.is_some()) {
        return;
    }
    if c.alive() {
        combat::leave_combat(map, g);
    }
    map.dead_creatures.retain(|d| *d != g);
    respawn(map, g);
}

/// A creature goes as though it died, with no loot or credit, and comes
/// back in five minutes.
pub fn quiet_death(map: &mut Map, g: Guid) {
    quiet_death_for(map, g, Duration::from_secs(300));
}

/// A creature goes as though it died, with no loot or credit, and comes
/// back after `respawn`; a corpse goes from view, to come back then too
/// (Thistle Tea's `Respawn.despawn`).
pub fn quiet_death_for(map: &mut Map, g: Guid, respawn: Duration) {
    let now = map.now;
    if let Some(c) = map.creature_mut(g)
        && !c.alive()
    {
        if c.despawned {
            return;
        }
        c.life = Life::Dead {
            corpse_until: now,
            respawn_at: now + respawn,
        };
        c.despawned = true;
        despawn_from_view(map, g);
        return;
    }
    crate::game::map::auras::clear_on_death(map, g);
    crate::game::map::spells::threat_sync::wiped(map, g);
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.life = Life::Dead {
        corpse_until: now,
        respawn_at: now + respawn,
    };
    c.threat.clear();
    c.combat = Default::default();
    c.evading = false;
    c.fields.set_u32(UNIT_FIELD_HEALTH, 0);
    // Its selection goes with it, as at a death (mangos's
    // `Creature::SetDeathState`, Thistle Tea's `Respawn.despawn`).
    c.fields.set_guid(UNIT_FIELD_TARGET, Guid::zero());
    c.despawned = true;
    map.dead_creatures.push(g);
    despawn_from_view(map, g);
}

/// A creature spawned dead, waiting `respawn_in` to come back, as a copy
/// saved it.
pub fn lie_dead(map: &mut Map, g: Guid, respawn_in: Option<Duration>) {
    let now = map.now;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.life = Life::Dead {
        corpse_until: now,
        respawn_at: now + respawn_in.unwrap_or(Duration::from_secs(10 * 365 * 86_400)),
    };
    c.fields.set_u32(UNIT_FIELD_HEALTH, 0);
    c.despawned = true;
    let cell = c.cell;
    map.grid.remove(g, cell);
    map.visibility_dirty.remove(&g);
    map.dead_creatures.push(g);
}

/// Run the creatures some player sees, or that are fighting.
pub fn update_creatures(map: &mut Map) {
    let now = map.now;
    let keys: Vec<usize> = map.awake_where(
        |c| {
            c.pet.is_none()
                && c.alive()
                && (!c.observers.is_empty()
                    || c.keep_awake
                    || c.combat.attacking
                    // A passive one in a fight, that no one sees: its turn
                    // lets go of those gone from it.
                    || !c.threat.is_empty()
                    || c.evading
                    || c.scripted
                        .as_deref()
                        .is_some_and(|s| s.feign.is_some_and(|f| f.down_at.is_some())))
        },
        |k, _| k,
    );
    // One zone for every turn, with their count: a zone a creature would
    // be thousands an update.
    let z = crate::zone!("creatures' turns");
    z.value(keys.len() as u64);
    for key in keys {
        let Some(g) = map.creatures.get(key).map(|c| c.guid) else {
            continue;
        };
        // Under its own guard: a creature that panics costs its turn, not
        // the update.
        crate::game::map::supervise::creature_turn(map, g, |map| turn(map, key, g, now));
    }
}

/// A creature's turn: its timed events, and its fight, its way home or
/// its walk.
fn turn(map: &mut Map, key: usize, g: Guid, now: Instant) {
    // Under control, its leash is kept.
    crate::game::map::creature_combat::maintain_leash(map, g);
    // Its timed events wait while it walks home, and while it lies in a
    // linked feign death (its fight's flag kept, they would fire on).
    if !map.creatures.get(key).is_some_and(|c| {
        c.evading
            || c.scripted
                .as_deref()
                .is_some_and(|s| s.feign.is_some_and(|f| f.down_at.is_some()))
    }) {
        crate::game::map::event_ai::tick(map, g);
    }
    let Some(c) = map.creatures.get(key).filter(|c| c.guid == g && c.alive()) else {
        return;
    };
    // Down in a linked feign death, or holding its path.
    if c.scripted
        .as_deref()
        .is_some_and(|s| s.feign.is_some_and(|f| f.down_at.is_some()))
    {
        crate::game::map::event_ai::feign_tick(map, g);
        return;
    }
    let (holding, evading) = (
        c.scripted.as_deref().is_some_and(|s| s.hold.is_some()),
        c.evading,
    );
    let escaped = c.engagement.escape_at.is_some_and(|t| t <= now);
    if holding {
        crate::game::map::event_ai::hold_tick(map, g);
    }
    // A critter's escape runs out whatever holds it: a root or a stun
    // doesn't keep it in the fight (Thistle Tea's escape timer).
    if !evading && escaped && crate::game::map::creature_motion::critter_step(map, g) {
        return;
    }
    // Feared, confused or fleeing: it moves as that has it.
    if !evading && crate::game::map::creature_combat::controlled_step(map, g) {
        return;
    }
    let Some(c) = map.creatures.get(key).filter(|c| c.guid == g && c.alive()) else {
        return;
    };
    if c.evading {
        evade_step(map, g);
    } else if c.combat.attacking {
        fight_step(map, g);
    } else {
        if c.aggro_check_at <= now && !c.observers.is_empty() {
            aggro_scan(map, g);
        }
        // Passive in a fight: it waits it out until no one it could
        // fight is left.
        passive_fight(map, g);
        // A distraction holds it where it stands, and so do a root and a
        // stun.
        if !map
            .creature(g)
            .is_some_and(|c| c.combat.attacking || c.evading)
            && !crate::game::map::creature_motion::distracted(map, g)
            && !crate::game::map::creature_combat::immobile(map, g)
        {
            crate::game::map::world::step_creature_key(map, key);
        }
    }
    crate::game::map::event_ai::retreat_step(map, g);
    let (moving, follows, fighting) = map
        .creature(g)
        .map(|c| {
            let s = c.scripted.as_deref();
            (
                s.is_some_and(|s| s.scripted_move.is_some()),
                s.is_some_and(|s| s.follow.is_some()),
                c.combat.attacking || c.evading,
            )
        })
        .unwrap_or_default();
    if moving {
        crate::game::map::event_ai::scripted_move_ended(map, g);
    }
    if follows && !fighting && !crate::game::map::creature_motion::held_by_distraction(map, g) {
        crate::game::map::event_ai::follow_step(map, g);
    }
}

/// Look for an enemy to attack, as Thistle Tea's creatures look: only an
/// aggressive one, in a state to fight, at the nearest enemy (player,
/// creature or pet) within its reach of it that it sees and can get at.
fn aggro_scan(map: &mut Map, g: Guid) {
    use crate::game::map::auras::{
        AURA_FEIGN_DEATH, AURA_MOD_CONFUSE, AURA_MOD_DETECT_RANGE, AURA_MOD_FEAR, AURA_MOD_PACIFY,
        AURA_MOD_PACIFY_SILENCE, AURA_MOD_STUN,
    };
    let now = map.now;
    // A critter looks for no one.
    if crate::game::map::creature_motion::critter(map, g) {
        return;
    }
    let data = map.realm.data.clone();
    // On its own phase of the interval: creatures that came into view
    // together don't all look again in the same update.
    let next = crate::game::map::next_beat(map.started, g, AGGRO_SCAN_EVERY, now);
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.aggro_check_at = next;
    // Just come into the world from its spawn (a respawn, or the world's
    // first), it lets those about it be for a while: a respawn mustn't
    // catch a player by surprise (`[creatures] spawn_grace_ms`). A summon
    // (Jaina's elementals, a script's or a GM's) has none: it comes to
    // fight. Attacked meanwhile, it fights as ever.
    let grace = Duration::from_millis(map.cfg.creatures.spawn_grace_ms);
    if map
        .creature(g)
        .filter(|c| c.spawn_guid.is_some())
        .and_then(|c| c.spawned_at)
        .is_some_and(|at| now.saturating_duration_since(at) < grace)
    {
        return;
    }
    if crate::game::map::assistance::reaction(map, g)
        != crate::game::map::assistance::Reaction::Aggressive
    {
        return;
    }
    let Some(c) = map.creature(g) else {
        return;
    };
    // Stunned, pacified, feigning or under control, or itself out of reach
    // of a fight: it starts none.
    let held = [
        AURA_MOD_STUN,
        AURA_MOD_PACIFY,
        AURA_MOD_PACIFY_SILENCE,
        AURA_FEIGN_DEATH,
        AURA_MOD_FEAR,
        AURA_MOD_CONFUSE,
    ]
    .iter()
    .any(|a| c.auras.has(*a))
        || c.engagement.flee.is_some();
    if held || c.fields.get_u32(UNIT_FIELD_FLAGS) & UNTARGETABLE != 0 {
        return;
    }
    let (pos, level) = (c.position, c.fields.get_u32(UNIT_FIELD_LEVEL));
    let detection = data
        .creature_template(c.entry)
        .map(|t| t.detection_range)
        .unwrap_or(20.0);
    let modifier = c.auras.sum(AURA_MOD_DETECT_RANGE) as f32;
    let search = aggro_reach(detection, level, 0, modifier) * map.cfg.creatures.aggro_rate;
    if search <= 0.0 {
        return;
    }
    let mut near: Vec<(Guid, f32)> = map
        .grid
        .around(pos.x, pos.y, search)
        .filter(|t| *t != g && (guid::is_player(*t) || guid::is_unit(*t)))
        .filter_map(|t| map.position_of(t).map(|at| (t, dist(at, pos))))
        .filter(|(_, d)| *d <= search)
        .collect();
    near.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.guid().cmp(&b.0.guid())));
    let eligible: Vec<Guid> = near
        .into_iter()
        .filter(|(t, d)| {
            let player_ok = map.players.get(t).is_none_or(|pl| {
                pl.death == Death::Alive
                    && !pl.fields.has_flag(PLAYER_FLAGS, PLAYER_FLAGS_GM)
                    && !pl.awaiting_worldport
            });
            player_ok
                && map.unit_view(*t).is_some_and(|u| u.alive)
                && *d <= attack_distance(map, g, *t)
                && !fooled(map, g, *t)
                && combat::hostile(map, g, *t)
                && combat::can_attack(map, g, *t)
                && crate::game::map::spells::stealth::sees(map, g, *t)
                && crate::game::map::creature_motion::may_reach(map, g, *t)
        })
        .map(|(t, _)| t)
        .collect();
    for t in eligible {
        let Some(to) = map.position_of(t) else {
            continue;
        };
        if crate::game::map::assistance::sees(map, pos, to) {
            attacked(map, g, t);
            return;
        }
    }
}

/// A passive creature in a fight takes no victim; when none on its threat
/// list is left to fight, the fight is over.
fn passive_fight(map: &mut Map, g: Guid) {
    let Some(c) = map.creature(g) else {
        return;
    };
    if c.combat.attacking || !c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT) {
        return;
    }
    // One gone far from its fight comes off its list, as one a fighting
    // creature would go home from (mangos's `CombatManager` drops an
    // immobile creature's references out of range): else whoever struck
    // it and walked away was held in their fight while both lived.
    let gone: Vec<Guid> = c
        .threat
        .iter()
        .map(|(t, _)| *t)
        .filter(|t| crate::game::map::creature_combat::gone_from_fight(map, g, *t))
        .collect();
    for t in gone {
        crate::game::map::spells::threat_sync::removed(map, g, t);
        if let Some(c) = map.creature_mut(g) {
            c.threat.retain(|(x, _)| *x != t);
        }
    }
    let Some(c) = map.creature(g) else {
        return;
    };
    // Left to fight as [`fight_step`] has it: one it doesn't see (gone
    // invisible) or believes dead holds it in no fight.
    let left = c.threat.iter().any(|(t, _)| valid_victim(map, g, *t));
    // With none left, it doesn't go home while a creature it may fight is
    // attacking it, as in [`fight_step`]: that one is put on its list.
    // (Keeper Remulos, his phantasm dead and his attack stopped, waited
    // here.)
    if !left && let Some(a) = attacker_of(map, g) {
        if let Some(c) = map.creature_mut(g) {
            c.threat.push((a, 0.0));
        }
        return;
    }
    if !left {
        // One on a script's walk (a peasant fleeing for Eris Havenfire's
        // light under the Scourge's arrows) leaves the fight where it is
        // and walks on, rather than going home and losing its way.
        if scripted_moving(map, g) {
            stand_down(map, g);
        } else {
            evade(map, g);
        }
    }
}

/// A passive creature's fight ends where it is: no threat, no combat, its
/// walk kept.
fn stand_down(map: &mut Map, g: Guid) {
    crate::game::map::creature_combat::disengaged(map, g);
    crate::game::map::event_ai::combat_stopped(map, g, false);
    crate::game::map::spells::threat_sync::wiped(map, g);
    if let Some(c) = map.creature_mut(g) {
        c.threat.clear();
        c.combat_start = None;
    }
    combat::leave_combat(map, g);
}

/// A unit's threat on a creature, never below none. Fade's share is in the
/// threat itself, for those fighting the priest when it went on
/// (`threat_sync`).
fn effective_threat(threat: f32) -> f32 {
    threat.max(0.0)
}

/// Whether `g` may fight `t`: alive, attackable, seen through any stealth
/// or invisibility (Thistle Tea's `valid_victim?`), and not lying in a
/// Feign Death `g` believes (mangos's `IsOfflineTarget`).
fn valid_victim(map: &Map, g: Guid, t: Guid) -> bool {
    map.unit_view(t).is_some_and(|u| u.alive)
        && combat::can_attack(map, g, t)
        && crate::game::map::spells::stealth::sees(map, g, t)
        && !fooled(map, g, t)
}

/// `static_flags2`'s `IGNORE_FEIGN_DEATH`.
const STATIC2_IGNORE_FEIGN_DEATH: u32 = 0x100;

/// Whether `t` lies in a believed Feign Death that `g` doesn't see
/// through (mangos's `IsIgnoringFeignDeath`): a creature of no player's,
/// as Thistle Tea's `targetable_by?` lets players and their pets at it.
fn fooled(map: &Map, g: Guid, t: Guid) -> bool {
    combat::feigning(map, t)
        && !guid::is_player(crate::game::map::pets::master(map, g))
        && !map
            .creature(g)
            .and_then(|c| map.realm.data.creature_template(c.entry))
            .is_some_and(|x| x.static_flags2 & STATIC2_IGNORE_FEIGN_DEATH != 0)
}

/// Those on `g`'s threat list it may no longer fight come off it, and
/// what they held there (Thistle Tea's `Threat.reselect`).
fn prune_threat(map: &mut Map, g: Guid) {
    let gone: Vec<Guid> = map
        .creature(g)
        .map(|c| {
            c.threat
                .iter()
                .map(|(t, _)| *t)
                .filter(|t| !valid_victim(map, g, *t))
                .collect()
        })
        .unwrap_or_default();
    for t in gone {
        crate::game::map::spells::threat_sync::removed(map, g, t);
        if let Some(c) = map.creature_mut(g) {
            c.threat.retain(|(x, _)| *x != t);
        }
    }
}

/// The highest threat still worth fighting, if any; every one on the list
/// is, if it was `pruned` just before ([`prune_threat`]), and isn't asked
/// again.
fn pick_victim(map: &Map, g: Guid, pruned: bool) -> Option<Guid> {
    let c = map.creature(g)?;
    c.threat
        .iter()
        .filter(|(t, _)| pruned || valid_victim(map, g, *t))
        .max_by(|a, b| effective_threat(a.1).total_cmp(&effective_threat(b.1)))
        .map(|(t, _)| *t)
}

/// A creature attacking `g` that `g` may fight, the first by key: what
/// keeps `g` in its fight when its threat list has no one left.
fn attacker_of(map: &Map, g: Guid) -> Option<Guid> {
    map.creatures
        .iter()
        .filter(|(_, c)| c.guid != g && c.combat.attacking && c.combat.victim == g)
        .map(|(_, c)| c.guid)
        .filter(|a| {
            valid_victim(map, g, *a) && !crate::game::map::creature_combat::should_evade(map, g, *a)
        })
        .min_by_key(|a| a.guid())
}

/// The caster of the newest taunt on `g` it may fight (Thistle Tea's
/// `taunt_caster`): an older one holds when the newest can't.
fn taunter(map: &Map, g: Guid) -> Option<Guid> {
    // Newest by when each went on, a refreshed one anew (mangos puts a
    // refreshed holder on again, last); as old, the later on the list.
    let mut taunts: Vec<(Option<std::time::Instant>, usize, Guid)> =
        map.creature(g)?
            .auras
            .holders
            .iter()
            .enumerate()
            .filter(|(_, h)| h.has(crate::game::map::auras::AURA_MOD_TAUNT))
            .map(|(i, h)| {
                let applied = h.expires.zip(h.duration).and_then(|(t, ms)| {
                    t.checked_sub(std::time::Duration::from_millis(u64::from(ms)))
                });
                (applied, i, h.caster)
            })
            .collect();
    taunts.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    taunts
        .into_iter()
        .map(|(_, _, t)| t)
        .find(|t| valid_victim(map, g, *t))
}

/// vmangos's `ThreatManager`: a taunter while the taunt lasts; otherwise
/// the current victim, until another passes its threat by a tenth in
/// melee range or three tenths beyond. Wherever they are: the leash, not
/// the choice, keeps a creature near home.
pub fn overtake(map: &Map, g: Guid, current: Guid) -> Guid {
    overtake_of(map, g, current, false)
}

/// [`overtake`], for a list `pruned` just before: everyone on it may be
/// fought, and isn't asked again (review 5's combat 4: a turn asked two or
/// three times of each, a full `can_attack` and `stealth::sees` each).
fn overtake_of(map: &Map, g: Guid, current: Guid, pruned: bool) -> Guid {
    let Some(c) = map.creature(g) else {
        return current;
    };
    if let Some(t) = taunter(map, g) {
        return t;
    }
    let Some(best) = pick_victim(map, g, pruned) else {
        return current;
    };
    if best == current {
        return current;
    }
    let threat_of = |t: Guid| {
        c.threat
            .iter()
            .find(|(x, _)| *x == t)
            .map(|(_, v)| effective_threat(*v))
    };
    let Some(mine) = threat_of(current) else {
        return best;
    };
    // Down the list from the top (Thistle Tea's `Threat.decide`): past a
    // tenth more, one in melee range takes over; past three tenths, anyone.
    let mut sorted: Vec<(Guid, f32)> = c
        .threat
        .iter()
        .filter(|(t, _)| pruned || valid_victim(map, g, *t))
        .map(|(t, v)| (*t, effective_threat(*v)))
        .collect();
    sorted.sort_by(|a, b| b.1.total_cmp(&a.1));
    let me = map.unit_view(g);
    for (t, theirs) in sorted {
        if t == current || theirs <= mine * 1.1 {
            return current;
        }
        let near = match (&me, map.unit_view(t)) {
            (Some(me), Some(them)) => combat::in_melee_range(me, &them),
            _ => false,
        };
        if theirs > mine * 1.3 || near {
            return t;
        }
    }
    current
}

fn fight_step(map: &mut Map, g: Guid) {
    let now = map.now;
    // Standing in reach attacking, it goes on standing only where this
    // step finds it so again.
    let stood = map
        .creature_mut(g)
        .and_then(|c| c.engagement.standing_since.take());
    if crate::game::map::creature_motion::critter_step(map, g) {
        return;
    }
    // A charge runs its course.
    if crate::game::map::charge::step(map, g) {
        return;
    }
    prune_threat(map, g);
    let Some(c) = map.creature(g) else {
        return;
    };
    let current = c.combat.victim;
    let victim = if valid_victim(map, g, current)
        && !crate::game::map::creature_combat::should_evade(map, g, current)
    {
        Some(overtake_of(map, g, current, true))
    } else {
        pick_victim(map, g, true)
    };
    let victim = victim.filter(|v| !crate::game::map::creature_combat::should_evade(map, g, *v));
    // With no one left on its list, it doesn't go home while a creature
    // it may fight is attacking it, one that hasn't harmed it yet and so
    // isn't on the list (mangos's `Unit::SelectHostileTarget`, whose last
    // case keeps a creature in its fight while it has attackers): it
    // fights that one, put on its list. (Keeper Remulos, his phantasm
    // dead before Eranikus's first breath reached him, went home, and
    // Eranikus, his victim gone, evaded and failed The Nightmare
    // Manifests.)
    let victim = match victim {
        Some(v) => Some(v),
        None => attacker_of(map, g).inspect(|a| {
            if let Some(c) = map.creature_mut(g) {
                c.threat.push((*a, 0.0));
            }
        }),
    };
    let Some(victim) = victim else {
        evade(map, g);
        return;
    };
    if victim != current
        && let Some(c) = map.creature_mut(g)
    {
        c.combat.victim = victim;
        c.fields.set_guid(UNIT_FIELD_TARGET, victim);
        // A new victim, a new clock for reaching it (Thistle Tea's
        // `UnreachableTarget.record` by victim).
        c.engagement.unreachable_since = None;
        // Another victim: one that calls the guards calls one (mangos's
        // `AttackStart` from `SelectHostileTarget`).
        crate::game::map::guards::attack_started(map, g, victim);
    }
    advance_motion(map, g);
    // A script's move goes on until it gets there; there, its arrival is
    // told before the fight moves it on (a retreat's end frees its spell
    // list, a point is informed).
    if scripted_moving(map, g) {
        return;
    }
    if map.creature(g).is_some_and(|c| {
        c.scripted
            .as_deref()
            .is_some_and(|s| s.scripted_move.is_some())
    }) {
        crate::game::map::event_ai::scripted_move_ended(map, g);
    }
    // Stunned, or casting: it stands where it is.
    crate::game::map::spells::creature_cast(map, g, victim);
    let busy = map.creature(g).is_some_and(|c| c.caster.casting.is_some())
        || crate::game::map::auras::held(map, g, false);
    if busy {
        stop_moving(map, g);
        return;
    }
    let (Some(me), Some(them)) = (map.unit_view(g), map.unit_view(victim)) else {
        return;
    };
    // A caster with a chase distance stands off within it, where it sees
    // its victim (Thistle Tea's `CasterChase.in_range?`).
    let caster = crate::game::map::event_ai::scripted(map, g).and_then(|s| s.caster_chase_distance);
    let sight = caster.is_some()
        && crate::game::map::spells::checks::line_of_sight(map, me.position, them.position);
    let stand_off = sight && caster.is_some_and(|d| dist(me.position, them.position) <= d + 0.01);
    // Rooted, it fights where it stands, turning to its victim (mangos's
    // chase waits out `UNIT_STAT_ROOT`).
    let rooted = crate::game::map::creature_combat::immobile(map, g);
    let chase = crate::game::map::event_ai::combat_movement(map, g) && !stand_off && !rooted;
    let swing = crate::game::map::event_ai::melee(map, g);
    let in_reach = combat::in_melee_range(&me, &them);
    if in_reach || !chase {
        // In reach, or held where it stands, it isn't failing to reach
        // its victim (Thistle Tea's `UnreachableTarget.maintain`: a
        // control pause keeps no failure).
        if in_reach || rooted {
            crate::game::map::creature_combat::reach(map, g, true);
        }
        // On its way in, a chaser runs on to where it aimed, inside its
        // reach: it doesn't stop on the reach's edge. It stops short only
        // once its victim, moved since, has come nearer it than where it
        // is headed (mangos-classic's `ChaseMovementGenerator`, its
        // destination past its target in reach: no running on behind
        // them). A step about its victim goes on to its end. A walk that
        // isn't a chase (a wander, its path) stops as the fight begins in
        // reach (`interrupt_idle_movement`).
        let (moving, spreading, chasing) = map.creature(g).map_or((false, false, false), |c| {
            let chasing = match (&c.motion, c.chase_target_at) {
                (Motion::Moving(w), Some(aimed)) => {
                    let ahead = dist(me.position, w.destination());
                    aimed == them.position || ahead <= dist(me.position, them.position)
                }
                _ => false,
            };
            (
                matches!(c.motion, Motion::Moving(_)),
                c.engagement.spreading,
                chasing,
            )
        });
        let runs_on = chase && moving && (spreading || chasing);
        // Turning to its victim a little it sends nothing: the client
        // turns a creature to its target itself (mangos's `SetInFront` as
        // it fights, "client change orientation by self"). Only one that
        // has its victim off its front is told turned (Thistle Tea's
        // `face_melee_victim`, on bad facing alone). A move for each turn,
        // at every bite the victim had stepped for, jerked it.
        if !runs_on && moving {
            stop_moving(map, g);
            turn_to(map, g, victim, faces(&me, &them));
        }
        if !moving && let Some(c) = map.creature_mut(g) {
            c.engagement.spreading = false;
        }
        if swing && in_reach && seen_in_reach(map, g, &me, &them) {
            announce(map, g, victim);
            let due = map
                .creature(g)
                .is_some_and(|c| c.combat.next_swing.is_none_or(|t| t <= now));
            if !runs_on && (due || !faces(&me, &them)) {
                turn_to(map, g, victim, faces(&me, &them));
            }
            combat::try_swing(map, g);
            // Still in reach, attacking: its stand renews its leash.
            if !runs_on {
                crate::game::map::creature_combat::stood_attacking(map, g, stood.unwrap_or(now));
            }
        } else if !runs_on && !faces(&me, &them) {
            face(map, g, victim);
        }
        if in_reach && chase && !moving {
            spread(map, g, victim);
        }
        return;
    }
    // Chase: path again once the victim is out of reach of where it is
    // headed, or of where it stands (mangos-classic's
    // `ChaseMovementGenerator::RequiresNewPosition`, the combined reach
    // from the spline's end): not for every step its victim takes.
    let Some(c) = map.creature(g) else {
        return;
    };
    let reach = combat::melee_range(&me, &them);
    let habitat = crate::game::map::creature_motion::habitat(map, g);
    let flies = habitat.fly;
    let headed = match &c.motion {
        Motion::Moving(w) if c.chase_target_at.is_some() => Some(w.destination()),
        Motion::Moving(_) => None,
        Motion::Idle { .. } => Some(me.position),
    };
    let stale = headed.is_none_or(|at| dist(at, them.position) > reach);
    if !stale {
        return;
    }
    let goal = match caster.filter(|_| sight) {
        // In sight, a caster closes only to its distance (Thistle Tea's
        // `CasterChase.destination`); out of it, it comes on as any.
        Some(d) => {
            let from = if flies {
                me.position
            } else {
                Vector3d {
                    z: them.position.z,
                    ..me.position
                }
            };
            let apart = dist(from, them.position);
            if apart > d {
                let f = d / apart;
                Vector3d {
                    x: them.position.x + (from.x - them.position.x) * f,
                    y: them.position.y + (from.y - them.position.y) * f,
                    z: them.position.z + (from.z - them.position.z) * f,
                }
            } else {
                from
            }
        }
        None => {
            // From the side it comes, turned aside by the others fighting
            // its victim (Thistle Tea's `approach_angle_offset`).
            let to = combat::chase_distance(reach);
            let angle = side_of(me.position, them.position) + approach_offset(map, g, victim);
            Vector3d {
                x: them.position.x + to * angle.cos(),
                y: them.position.y + to * angle.sin(),
                z: them.position.z,
            }
        }
    };
    // As its habitat lets it: through the air, the water, or only to the
    // edge of what it can't enter.
    let routed = crate::game::map::creature_motion::route(map, g, me.position, goal);
    crate::game::map::creature_combat::reach(map, g, routed.as_ref().is_some_and(|r| r.1));
    let (path, fly) = match routed {
        Some((path, _, fly)) => (path, fly),
        None if map.nav.is_some() => return,
        None => (vec![me.position, goal], false),
    };
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.chase_target_at = Some(them.position);
    c.engagement.spread_tries = 0;
    c.engagement.spreading = false;
    if let Some(frame) = c.walk_facing(path, true, fly, victim, now, spline_id) {
        map.send_to_observers(g, &frame, false);
    }
}

/// How often a creature fighting in reach looks to step about its victim.
const SPREAD_EVERY_MS: (u64, u64) = (2500, 3500);
/// How often it steps aside from one of its kind before its victim moves.
const SPREAD_TRIES: u8 = 3;
/// The bounding radius of a unit with none.
const DEFAULT_BOUNDING: f32 = 0.389;

fn bounding(map: &Map, g: Guid) -> f32 {
    map.unit_fields(g)
        .map(|f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS))
        .filter(|r| *r > 0.0)
        .unwrap_or(DEFAULT_BOUNDING)
}

/// The angle from `them` to `me`, any if they stand on each other.
fn side_of(me: Vector3d, them: Vector3d) -> f32 {
    let (dx, dy) = (me.x - them.x, me.y - them.y);
    if dx.abs() < 1e-4 && dy.abs() < 1e-4 {
        rand::rng().random_range(0.0..std::f32::consts::TAU)
    } else {
        dy.atan2(dx)
    }
}

/// How many fight `t`.
fn attackers(map: &Map, t: Guid) -> usize {
    map.combat(t).map_or(0, |c| c.attackers.len())
}

/// Whether `t` is on the move.
fn on_the_move(map: &Map, t: Guid) -> bool {
    match map.players.get(&t) {
        Some(p) => crate::game::map::spells::checks::moving(&p.movement.flags),
        None => map
            .creature(t)
            .is_some_and(|c| matches!(c.motion, Motion::Moving(_))),
    }
}

/// A chaser's turn aside as it comes at a victim others fight already, so
/// they don't all stand in one place: none at a player on the move
/// (Thistle Tea's `approach_angle_offset`).
fn approach_offset(map: &Map, g: Guid, victim: Guid) -> f32 {
    let moving_player = guid::is_player(victim) && on_the_move(map, victim);
    let count = if moving_player {
        0
    } else {
        attackers(map, victim).saturating_sub(1)
    };
    if count == 0 {
        return 0.0;
    }
    let size = bounding(map, g) + bounding(map, victim);
    let size = if size < 0.1 { DEFAULT_BOUNDING } else { size };
    let spread = std::f32::consts::FRAC_PI_2 - std::f32::consts::PI * rand::rng().random::<f32>();
    spread * count as f32 / size * 0.3
}

/// Standing in reach, a creature every three seconds or so steps back off
/// a victim it stands in, or (three times, until its victim moves) aside
/// from another standing on it, at a walk and facing its victim (Thistle
/// Tea's `maybe_spread`).
fn spread(map: &mut Map, g: Guid, victim: Guid) {
    let now = map.now;
    let mut rng = rand::rng();
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    if c.engagement.spread_at.is_some_and(|t| t > now) {
        return;
    }
    let wait = rng.random_range(SPREAD_EVERY_MS.0..=SPREAD_EVERY_MS.1);
    c.engagement.spread_at = Some(now + Duration::from_millis(wait));
    if on_the_move(map, victim) {
        if let Some(c) = map.creature_mut(g) {
            c.engagement.spread_tries = 0;
            c.engagement.spreading = false;
        }
        return;
    }
    let (Some(me), Some(them)) = (map.unit_view(g), map.unit_view(victim)) else {
        return;
    };
    let reach = combat::melee_range(&me, &them);
    let (mine, theirs) = (bounding(map, g), bounding(map, victim));
    let apart = planar(me.position, them.position);
    // Back off one it stands in.
    if apart < (0.5 * mine.min(theirs) + 1.0).sqrt() {
        let d = 1.0 + mine + theirs;
        if d < reach {
            let angle = side_of(me.position, them.position);
            step_about(map, g, victim, them.position, angle, d, false);
        }
        return;
    }
    let tries = map.creature(g).map_or(0, |c| c.engagement.spread_tries);
    if tries >= SPREAD_TRIES {
        return;
    }
    // Aside from the nearest of its kind standing on it.
    let neighbour = map
        .grid
        .around(me.position.x, me.position.y, 2.0)
        .filter(|n| *n != g && *n != victim && guid::is_unit(*n))
        .filter_map(|n| {
            let c = map.creature(n).filter(|c| c.alive())?;
            let d = planar(c.position, me.position);
            let stacked = (mine.max(bounding(map, n)).min(0.25) + 1.0).sqrt();
            (d < stacked).then_some((c.position, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1));
    let Some((at, _)) = neighbour else {
        return;
    };
    let mine_angle = side_of(me.position, them.position);
    let his = (at.y - them.position.y).atan2(at.x - them.position.x);
    let delta = rng.random_range(0.4f32..1.0);
    let turn = if (his - mine_angle).sin().atan2((his - mine_angle).cos()) > 0.0 {
        -delta
    } else {
        delta
    };
    let widest = if attackers(map, victim) > 5 { 4.0 } else { 2.0 };
    let d = mine + rng.random_range(0.8f32..widest);
    if d < reach {
        step_about(map, g, victim, them.position, mine_angle + turn, d, true);
    }
}

/// `g` walks to `d` yards from its victim at `angle`, facing it.
fn step_about(
    map: &mut Map,
    g: Guid,
    victim: Guid,
    them: Vector3d,
    angle: f32,
    d: f32,
    aside: bool,
) {
    let now = map.now;
    let Some(from) = map.position_of(g) else {
        return;
    };
    let to = Vector3d {
        x: them.x + d * angle.cos(),
        y: them.y + d * angle.sin(),
        z: them.z,
    };
    let (path, fly) = match crate::game::map::creature_motion::route(map, g, from, to) {
        Some((path, true, fly)) => (path, fly),
        _ if map.nav.is_some() => return,
        _ => (vec![from, to], false),
    };
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let Some(frame) = c.walk_facing(path, false, fly, victim, now, spline_id) else {
        return;
    };
    c.engagement.spreading = true;
    if aside {
        c.engagement.spread_tries += 1;
    }
    map.send_to_observers(g, &frame, false);
}

fn planar(a: Vector3d, b: Vector3d) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// Whether `me` faces `them` well enough to swing: within a third of a
/// circle, or so near it doesn't matter (Thistle Tea's `facing_target?`).
fn faces(me: &combat::UnitView, them: &combat::UnitView) -> bool {
    planar(me.position, them.position) <= 1.4 || combat::facing(me, them)
}

/// Whether the client, which shows a creature on the move a little behind
/// where it is, sees `me` in reach of `them`: one standing is where it is;
/// one on the move is if the way it came is all in reach (a step about its
/// victim), or both run (the leeway is for that). A creature running in
/// swings once it is there.
fn seen_in_reach(map: &Map, g: Guid, me: &combat::UnitView, them: &combat::UnitView) -> bool {
    let Some(Motion::Moving(w)) = map.creature(g).map(|c| &c.motion) else {
        return true;
    };
    if me.running && them.running {
        return true;
    }
    let reach = combat::melee_range(me, them);
    w.passed(map.now).all(|p| dist(p, them.position) <= reach)
}

/// The creature's `SMSG_ATTACKSTART` on `victim`, once for each victim, at
/// its first swing in reach (Thistle Tea's `maybe_start_melee_attack`).
fn announce(map: &mut Map, g: Guid, victim: Guid) {
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    if c.combat.announced == Some(victim) {
        return;
    }
    c.combat.announced = Some(victim);
    if let Some(frame) = crate::game::conn::encode(&wow_world_messages::vanilla::SMSG_ATTACKSTART {
        attacker: g,
        victim,
    }) {
        map.send_to_observers(g, &frame, false);
        if let Some(p) = map.players.get(&victim)
            && !p.known.contains(&g)
        {
            p.outbound.send_frame(frame);
        }
    }
}

/// A creature fighting turns to `target`: telling no one if it `faced` it
/// already, near enough (its client turns it to its target itself,
/// mangos's `SetInFront`); else as [`face`] does.
fn turn_to(map: &mut Map, g: Guid, target: Guid, faced: bool) {
    if !faced {
        face(map, g, target);
        return;
    }
    let (Some(at), Some(to)) = (map.position_of(g), map.position_of(target)) else {
        return;
    };
    if planar(at, to) < 0.01 {
        return;
    }
    if let Some(c) = map.creature_mut(g) {
        c.orientation = (to.y - at.y).atan2(to.x - at.x);
    }
}

/// A creature standing in a fight turns to face `target`, and tells its
/// observers, if it doesn't already (Thistle Tea's `face_target`).
pub(crate) fn face(map: &mut Map, g: Guid, target: Guid) {
    let (Some(at), Some(to)) = (map.position_of(g), map.position_of(target)) else {
        return;
    };
    if planar(at, to) < 0.01 {
        return;
    }
    let angle = (to.y - at.y).atan2(to.x - at.x);
    let spline_id = map.next_spline_id;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let off = (angle - c.orientation).sin().abs() > 0.01 || (angle - c.orientation).cos() < 0.0;
    if !off {
        return;
    }
    c.orientation = angle;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    map.send_to_observers(g, &face_target_packet(g, at, target, spline_id), false);
}

/// Whether a script's move is under way.
fn scripted_moving(map: &Map, g: Guid) -> bool {
    map.creature(g).is_some_and(|c| {
        matches!(
            (&c.motion, c.scripted.as_deref().and_then(|s| s.scripted_move)),
            (Motion::Moving(w), Some(m)) if w.spline_id == m.spline_id
        )
    })
}

/// `g` backs away from `enemy` to stand `distance` yards off it (and its
/// reach), as Thistle Tea's distancing moves it.
pub fn back_away(map: &mut Map, g: Guid, enemy: Guid, distance: f32) {
    let (Some(me), Some(e)) = (map.unit_view(g), map.unit_view(enemy)) else {
        return;
    };
    if g == enemy || !e.alive {
        return;
    }
    let angle = (me.position.y - e.position.y).atan2(me.position.x - e.position.x);
    let bounding = map
        .unit_fields(enemy)
        .map_or(0.0, |f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS).max(0.0));
    let r = distance + bounding;
    let to = Vector3d {
        x: e.position.x + r * angle.cos(),
        y: e.position.y + r * angle.sin(),
        z: e.position.z,
    };
    crate::game::map::spells::interrupt(map, g);
    crate::game::map::scripts::move_to(map, g, to, true);
}

/// A script's `enter evade`.
pub fn enter_evade(map: &mut Map, g: Guid) {
    evade(map, g);
}

/// A script's call for help: those of `g`'s friends within `radius` that
/// answer join its fight (a radius of none calls no one).
pub fn call_for_help_radius(map: &mut Map, g: Guid, radius: f32) {
    let Some(enemy) = map.creature(g).map(|c| c.combat.victim) else {
        return;
    };
    crate::game::map::assistance::call_friends(map, g, enemy, radius);
}

/// Run from `enemy` in fear for `run_for` (seven seconds without),
/// whatever its fight, as a script's movement 10 has it (Thistle Tea's
/// `flee_from` and `flee_until`): it runs as a fear runs, one way then
/// another, until its time is up or the one it runs from is dead or gone.
pub fn flee_from(map: &mut Map, g: Guid, enemy: Guid, run_for: Option<Duration>) {
    let now = map.now;
    let flee_for = Duration::from_millis(map.cfg.creatures.flee_delay_ms);
    if !map.creature(g).is_some_and(|c| c.alive()) || map.unit_view(enemy).is_none() {
        return;
    }
    map.rare.flee = true;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.engagement.flee = Some(crate::game::map::creature_combat::Flee {
        from: enemy,
        until: now + run_for.unwrap_or(flee_for),
        helper: None,
        called_at: None,
        scripted: true,
    });
    c.engagement.next_move_at = Some(now);
    c.fields.set_flag(
        UNIT_FIELD_FLAGS,
        crate::game::map::auras::UNIT_FLAG_FLEEING,
        true,
    );
}

/// Give up the fight and run home.
fn evade(map: &mut Map, g: Guid) {
    // Where its fight began, before the fight is forgotten.
    let began = map.creature(g).and_then(|c| c.engagement.origin);
    crate::game::map::creature_combat::disengaged(map, g);
    crate::game::map::event_ai::combat_stopped(map, g, false);
    crate::game::map::spells::threat_sync::wiped(map, g);
    let now = map.now;
    let spline_id = map.next_spline_id;
    let Some(c) = map.creature(g) else {
        return;
    };
    // A follower goes back to its place by its leader; a patroller to the
    // last point of its path it reached.
    let from = c.position;
    // One a script set following someone goes back to where its fight
    // began and follows on from there (mangos's `FollowerAI::EnterEvadeMode`,
    // its combat start position; a follow not its main movement is
    // dropped by `MoveTargetedHome` for the same place): not where it
    // stands (Thistle Tea), nor its spawn.
    let follows = c
        .scripted
        .as_deref()
        .and_then(|s| s.follow)
        .is_some_and(|(leader, _, _)| map.unit_view(leader).is_some_and(|u| u.alive));
    let home = if follows {
        began.unwrap_or(from)
    } else {
        crate::game::map::creature_groups::formation_home(map, g)
            .or_else(|| patrol_home(map, g))
            .unwrap_or(c.home)
    };
    // One that never moves leaves its fight where it stands (mangos's
    // home movement waits out its AI's root; Thistle Tea's sessile
    // evade).
    // A flyer flies home, as it chased (Thistle Tea's airborne home
    // movement).
    let there = follows && dist(from, home) < 0.5;
    let (path, fly) = if there || crate::game::map::creature_motion::sessile(map, g) {
        (vec![from], false)
    } else if crate::game::map::creature_motion::habitat(map, g).fly {
        match crate::game::map::creature_motion::route(map, g, from, home) {
            Some((path, _, fly)) => (path, fly),
            None => (vec![from, home], true),
        }
    } else {
        let path = match map.nav.as_mut() {
            Some(nav) => nav.path(from, home).unwrap_or_else(|| vec![from, home]),
            None => vec![from, home],
        };
        (path, false)
    };
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let attackers: Vec<Guid> = map
        .creature(g)
        .map(|c| c.threat.iter().map(|(t, _)| *t).collect())
        .unwrap_or_default();
    combat::stop_attack(map, g);
    crate::game::map::spells::interrupt(map, g);
    shed_on_evade(map, g);
    map.note_fighting(g);
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.evading = true;
    c.threat.clear();
    c.combat = Default::default();
    c.caster = Default::default();
    crate::game::map::tap::clear(c);
    c.chase_target_at = None;
    c.fields.set_guid(UNIT_FIELD_TARGET, Guid::zero());
    c.fields
        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, false);
    c.fields.set_flag(UNIT_DYNAMIC_FLAGS, DYNFLAG_TAPPED, false);
    let frame = c.walk_at(path, true, fly, None, now, spline_id);
    if frame.is_none() {
        c.evading = false;
    }
    if let Some(frame) = frame {
        map.send_to_observers(g, &frame, false);
    }
    for a in attackers {
        if map.combat(a).is_some_and(|c| c.attacking && c.victim == g) {
            combat::stop_attack(map, a);
        }
        if let Some(c) = map.combat_mut(a) {
            c.attackers.remove(&g);
        }
    }
    // Its scripts run on: a script that wants them stopped says so.
    crate::game::map::spells::lists::left_combat(map, g);
    crate::game::map::event_ai::evade(map, g);
    // Whole again as it leashes, if it regenerates (Thistle Tea's
    // `heal_to_full`), not when it gets home.
    if crate::game::map::creature_combat::regenerates_health(map, g)
        && let Some(c) = map.creature_mut(g)
        && c.alive()
    {
        let max = c.fields.get_u32(UNIT_FIELD_MAXHEALTH);
        if c.fields.get_u32(UNIT_FIELD_HEALTH) < max {
            c.fields.set_u32(UNIT_FIELD_HEALTH, max);
            combat::sync_aura_state(map, g);
            crate::game::map::creature_motion::health_changed(map, g);
        }
    }
    // Already where it goes (sessile, or a follower at its place): home at
    // once, as mangos-classic's `HomeMovementGenerator` finalizes.
    if map.creature(g).is_some_and(|c| !c.evading) {
        home_reached(map, g);
    }
}

/// What an evade takes from a creature, as Thistle Tea's `remove_on_evade`
/// has it: all its auras but what vmangos marks to stay, a player's timed
/// buffs, and (for a creature whose template says so) its buffs.
pub fn shed_on_evade(map: &mut Map, g: Guid) {
    let data = map.realm.data.clone();
    let keeps_buffs = map
        .creature(g)
        .and_then(|c| data.creature_template(c.entry))
        .is_some_and(|t| t.flags_extra & 0x1000 != 0);
    let shed: Vec<(u32, Guid)> = map
        .creature(g)
        .map(|c| {
            c.auras
                .holders
                .iter()
                .filter(|h| {
                    let stays =
                        data.spells.get(h.spell).is_some_and(|s| {
                            s.custom(crate::game::data::spells::CUSTOM_NOT_REMOVED_ON_EVADE)
                        }) || (h.positive && h.expires.is_some() && guid::is_player(h.caster))
                            || (keeps_buffs && h.positive);
                    !stays
                })
                .map(|h| (h.spell, h.caster))
                .collect()
        })
        .unwrap_or_default();
    for (spell, caster) in shed {
        crate::game::map::auras::remove_spell(map, g, spell, Some(caster));
    }
}

/// An evading creature walks home, and rests there as it did.
fn evade_step(map: &mut Map, g: Guid) {
    let now = map.now;
    advance_motion(map, g);
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    if matches!(c.motion, Motion::Idle { .. }) {
        c.evading = false;
        c.motion = Motion::Idle {
            until: Some(now + Duration::from_secs(2)),
        };
        home_reached(map, g);
    }
}

/// An evading creature is home: it turns to its spawn's facing, rests,
/// has its own faction back if it was to on reaching home, and its
/// `EVENT_REACHED_HOME` runs (mangos-classic's `JustReachedHome`).
fn home_reached(map: &mut Map, g: Guid) {
    let Some(c) = map.creature(g) else {
        return;
    };
    // Home, it turns to its spawn's facing (a patroller as it is).
    let (at, o) = (c.position, c.home_orientation);
    let turned = c.orientation != o;
    if turned && patrol_home(map, g).is_none() {
        let spline_id = map.next_spline_id;
        map.next_spline_id = map.next_spline_id.wrapping_add(1);
        if let Some(c) = map.creature_mut(g) {
            c.orientation = o;
        }
        map.send_to_observers(g, &face_packet(g, at, o, spline_id), false);
    }
    crate::game::map::creature_combat::rest(map, g);
    crate::game::map::spawn_auras::restore(map, g);
    crate::game::map::scripts::restore_faction(
        map,
        g,
        crate::game::map::event_ai::FACTION_RESTORE_REACH_HOME,
    );
    crate::game::map::event_ai::reached_home(map, g);
}

/// Where a creature walking a path goes back to after a fight: the last
/// point it reached (none until it reaches one).
fn patrol_home(map: &Map, g: Guid) -> Option<Vector3d> {
    let c = map.creature(g)?;
    if c.movement_type != crate::game::map::creature::MOVEMENT_WAYPOINT {
        return None;
    }
    let i = c.last_waypoint?;
    let w = *crate::game::map::creature_motion::waypoints(map, g)?.get(i)?;
    Some(Vector3d {
        x: w.x,
        y: w.y,
        z: w.z,
    })
}

/// Bring a moving creature's position up to now, and its grid cell and
/// visibility with it.
pub fn advance_motion(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let Motion::Moving(w) = &c.motion else {
        return;
    };
    let done = w.done(now);
    let facing = w.face_target.filter(|_| done);
    let (pos, o) = if done {
        (w.destination(), w.face.unwrap_or(c.orientation))
    } else if w.falling {
        // A fall keeps the way it faced.
        (w.position_at(now).0, c.orientation)
    } else {
        w.position_at(now)
    };
    // A walk that faces a unit ends facing it.
    let o = match facing.and_then(|t| map.position_of(t)) {
        Some(t) if planar(t, pos) > 0.01 => (t.y - pos.y).atan2(t.x - pos.x),
        _ => o,
    };
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.position = pos;
    c.orientation = o;
    if done {
        c.motion = Motion::Idle { until: None };
    }
    let (anchor, old_cell) = (c.vis_anchor, c.cell);
    let new_cell = crate::game::map::grid::cell_of(pos.x, pos.y);
    c.cell = new_cell;
    map.grid.relocate(g, old_cell, new_cell);
    crate::game::map::visibility::moved(map, g, anchor, pos);
}

/// Whether a creature's faction is hostile to the players' factions, so it
/// looks for them.
pub fn hostile_to_players(
    data: &crate::game::data::GameData,
    faction: u32,
    civilian: bool,
) -> bool {
    if civilian {
        return false;
    }
    let Some(f) = data.faction_templates.get(&faction) else {
        return false;
    };
    // 1: Alliance players' group, 2: Horde's ... the player masks.
    f.enemy_mask & 0x7 != 0
        || f.enemies
            .iter()
            .any(|e| matches!(*e, 469 | 67 | 1 | 2 | 3 | 4 | 5 | 6 | 8 | 115 | 116))
}

/// A creature on the move stops where it is now along its walk, its
/// watchers told.
pub fn halt(map: &mut Map, g: Guid) {
    advance_motion(map, g);
    stop_moving(map, g);
}

/// A creature comes to rest, standing until `until` (for good with
/// `None`): a walk it is on stops where it has come to now, its watchers
/// told with a stop there ([`halt`], as mangos's `MoveSplineInit::Stop`
/// and death's stop), as every place that lets a walk go does. Without
/// it, a client that saw the walk set off ran the creature on to its end,
/// and the next walk snapped it back. A fall goes on.
pub fn rest(map: &mut Map, g: Guid, until: Option<Instant>) {
    halt(map, g);
    if let Some(c) = map.creature_mut(g)
        && !matches!(&c.motion, Motion::Moving(w) if w.falling)
    {
        c.motion = Motion::Idle { until };
    }
}

/// A creature on the move stops where it is.
fn stop_moving(map: &mut Map, g: Guid) {
    let spline_id = map.next_spline_id;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    // Nothing stops a fall.
    if !matches!(&c.motion, Motion::Moving(w) if !w.falling) {
        return;
    }
    c.motion = Motion::Idle { until: None };
    let at = c.position;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let frame = stop_packet(g, at, spline_id);
    map.send_to_observers(g, &frame, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_aggro_radius_shrinks_as_the_player_outlevels() {
        assert_eq!(aggro_radius(20.0, 10, 10), 20.0);
        assert_eq!(aggro_radius(20.0, 10, 15), 15.0);
        assert_eq!(aggro_radius(20.0, 10, 40), 5.0);
        assert_eq!(aggro_radius(20.0, 60, 1), 45.0);
    }

    #[test]
    fn a_stop_packet_is_short() {
        let f = stop_packet(
            guid::unit(1, 1),
            Vector3d {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            4,
        );
        // Header, packed guid (4 bytes here), position, id, type.
        assert_eq!(f.len(), 4 + 5 + 12 + 4 + 1);
    }

    /// Bug hunt (creatures): a death's respawn time searched every spawn
    /// of every map, and a respawn its map's twice more. A spawn is looked
    /// up by its guid, on any map or on one, and its time is its own.
    #[test]
    fn a_spawn_is_found_by_its_guid_and_its_respawn_time_is_its_own() {
        use crate::game::data::{CreatureSpawn, GameData};
        let mut d = GameData::empty();
        let spawn = |guid, map, secs| CreatureSpawn {
            guid,
            map,
            respawn_min: secs,
            respawn_max: secs,
            ..Default::default()
        };
        d.creature_spawns
            .insert(0, vec![spawn(1, 0, 60), spawn(2, 0, 90)]);
        d.creature_spawns.insert(1, vec![spawn(3, 1, 120)]);
        assert_eq!(d.creature_spawn(3).map(|s| s.map), Some(1));
        assert_eq!(d.creature_spawn_in(0, 2).map(|s| s.respawn_min), Some(90));
        assert!(d.creature_spawn_in(0, 3).is_none(), "not on that map");
        assert!(d.creature_spawn(4).is_none());
        assert_eq!(respawn_delay(&d, Some(3), 1.0), Duration::from_secs(120));
        assert_eq!(respawn_delay(&d, None, 1.0), Duration::from_secs(300));
    }
}
