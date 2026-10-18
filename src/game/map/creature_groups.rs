//! Creature groups (`creature_groups`) and links (`creature_linking`), as
//! vmangos has them and Thistle Tea ports them. A group fights, evades and
//! respawns together as its flags say, and walks in formation behind its
//! leader; a linked slave follows its master's aggro, evade and respawn.
//! Spawn pools are `pools`.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::GameData;
use crate::game::data::scripts::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::scripts::creature_of_spawn;

/// The spawn guid a creature came from.
fn spawn_of(map: &Map, g: Guid) -> Option<u32> {
    map.creature(g).and_then(|c| c.spawn_guid)
}

/// The leader's spawn of `g`'s group.
pub fn leader_of(map: &Map, g: Guid) -> Option<u32> {
    if let Some((leader, _, _, _)) = map.script_groups.get(&g) {
        return spawn_of(map, *leader).or(Some(0));
    }
    let spawn = spawn_of(map, g)?;
    map.realm.data.scripts.group_of.get(&spawn).copied()
}

pub fn is_leader(map: &Map, g: Guid) -> bool {
    spawn_of(map, g).is_some_and(|s| map.realm.data.scripts.groups.contains_key(&s))
        || map.script_groups.values().any(|(l, _, _, _)| *l == g)
}

/// The flags of `g`'s place in a group a script made: its own as a
/// member, its members' as a leader.
fn script_flags(map: &Map, g: Guid) -> u32 {
    if let Some((_, _, _, flags)) = map.script_groups.get(&g) {
        return *flags;
    }
    map.script_groups
        .values()
        .filter(|(l, _, _, _)| *l == g)
        .fold(0, |f, (_, _, _, x)| f | x)
}

fn group_flags(map: &Map, leader: u32) -> u32 {
    map.realm
        .data
        .scripts
        .groups
        .get(&leader)
        .map_or(0, |m| m.iter().fold(0, |f, x| f | x.flags))
}

/// The creatures of `g`'s group here, its leader first.
pub fn members_of(map: &Map, g: Guid) -> Vec<Guid> {
    if let Some((leader, _, _, _)) = map.script_groups.get(&g).copied() {
        let mut out = vec![leader];
        out.extend(
            map.script_groups
                .iter()
                .filter(|(_, (l, _, _, _))| *l == leader)
                .map(|(m, _)| *m),
        );
        return out;
    }
    if map.script_groups.values().any(|(l, _, _, _)| *l == g) {
        let mut out = vec![g];
        out.extend(
            map.script_groups
                .iter()
                .filter(|(_, (l, _, _, _))| *l == g)
                .map(|(m, _)| *m),
        );
        return out;
    }
    let Some(leader) = leader_of(map, g) else {
        return Vec::new();
    };
    let data = &map.realm.data.scripts;
    let mut out: Vec<Guid> = creature_of_spawn(map, leader).into_iter().collect();
    if let Some(members) = data.groups.get(&leader) {
        for m in members {
            if let Some(c) = creature_of_spawn(map, m.guid)
                && !out.contains(&c)
            {
                out.push(c);
            }
        }
    }
    out
}

/// The spawn leading `g`'s group, `g`'s own if it leads one a script
/// made (0 for a leader with no spawn); none out of a group.
pub fn group_leader(map: &Map, g: Guid) -> Option<u32> {
    if is_script_leader(map, g) {
        return spawn_of(map, g).or(Some(0));
    }
    leader_of(map, g)
}

/// Whether every other member of `g`'s group here is dead (Thistle Tea's
/// `CreatureGroup.dead?`, which leaves out the one asking); with no group,
/// it is, as vmangos's `CONDITION_CREATURE_GROUP_DEAD` has it.
pub fn all_dead(map: &Map, g: Guid) -> bool {
    members_of(map, g)
        .iter()
        .filter(|m| **m != g)
        .all(|m| map.creature(*m).is_none_or(|c| !c.alive()))
}

/// A script puts `g` in `leader`'s group, at a distance and angle (in
/// radians from the leader's facing), with group flags.
pub fn join_with(map: &mut Map, g: Guid, leader: Guid, distance: f32, angle: f32, flags: u32) {
    if g == leader || !guid::is_unit(leader) {
        return;
    }
    // A leader's own leader leads the lot.
    let leader = map
        .script_groups
        .get(&leader)
        .map_or(leader, |(l, _, _, _)| *l);
    map.script_groups
        .insert(g, (leader, distance, angle, flags));
}

pub fn leave(map: &mut Map, g: Guid) {
    map.script_groups.remove(&g);
}

/// `g` was attacked by `enemy`: its group and its links join in. Its
/// group's members join as told to (Thistle Tea's `CreatureGroup`
/// `:attack`), calling no one themselves.
pub fn aggro(map: &mut Map, g: Guid, enemy: Guid) {
    if leader_of(map, g).is_some_and(|l| group_flags(map, l) & GROUP_AGGRO_TOGETHER != 0)
        || script_flags(map, g) & GROUP_AGGRO_TOGETHER != 0
    {
        let members = members_of(map, g);
        join_all(map, g, members, enemy, false);
    }
    let mut join: Vec<Guid> = Vec::new();
    let Some(spawn) = spawn_of(map, g) else {
        return;
    };
    let data = map.realm.data.clone();
    // Slaves of this master.
    for (slave, (master, flags)) in &data.scripts.links {
        if *master == spawn
            && flags & LINK_AGGRO_ON_AGGRO != 0
            && let Some(s) = creature_of_spawn(map, *slave)
        {
            join.push(s);
        }
    }
    // Its own master.
    if let Some((master, flags)) = data.scripts.links.get(&spawn)
        && flags & LINK_TO_AGGRO_ON_AGGRO != 0
        && let Some(m) = creature_of_spawn(map, *master)
    {
        join.push(m);
    }
    attack_all(map, g, join, enemy);
}

fn attack_all(map: &mut Map, g: Guid, who: Vec<Guid>, enemy: Guid) {
    join_all(map, g, who, enemy, true);
}

/// Those of `who` free to join `g`'s fight against `enemy` do, on `g`'s
/// leash; `call`, and each calls for help as one attacked does.
fn join_all(map: &mut Map, g: Guid, who: Vec<Guid>, enemy: Guid, call: bool) {
    for c in who {
        if map
            .creature(c)
            .is_some_and(|x| x.alive() && !x.combat.attacking && !x.evading)
            && crate::game::map::combat::can_attack(map, c, enemy)
        {
            if call {
                crate::game::map::ai::attacked(map, c, enemy);
            } else {
                crate::game::map::ai::assist(map, c, enemy);
            }
            crate::game::map::creature_combat::share_leash(map, c, g);
        }
    }
}

/// `g` gave up its fight: its group, if they evade together, and its
/// links.
pub fn evaded(map: &mut Map, g: Guid) {
    let mut evade: Vec<Guid> = Vec::new();
    if leader_of(map, g).is_some_and(|l| group_flags(map, l) & GROUP_EVADE_TOGETHER != 0)
        || script_flags(map, g) & GROUP_EVADE_TOGETHER != 0
    {
        evade.extend(members_of(map, g).into_iter().filter(|m| *m != g));
    }
    let data = map.realm.data.clone();
    if let Some(spawn) = spawn_of(map, g) {
        for (slave, (master, flags)) in &data.scripts.links {
            if *master != spawn {
                continue;
            }
            let Some(s) = creature_of_spawn(map, *slave) else {
                continue;
            };
            let alive = map.creature(s).is_some_and(|c| c.alive());
            if alive && flags & LINK_DESPAWN_ON_EVADE != 0 {
                crate::game::map::scripts::despawn(map, s);
            } else if alive && flags & LINK_EVADE_ON_EVADE != 0 {
                evade.push(s);
            } else if !alive && flags & LINK_RESPAWN_ON_EVADE != 0 {
                crate::game::map::ai::respawn_now(map, s);
            }
        }
        if let Some((master, flags)) = data.scripts.links.get(&spawn)
            && flags & LINK_TO_RESPAWN_ON_EVADE != 0
            && let Some(m) = creature_of_spawn(map, *master)
            && map.creature(m).is_some_and(|c| !c.alive())
        {
            crate::game::map::ai::respawn_now(map, m);
        }
    }
    for c in evade {
        if map.creature(c).is_some_and(|x| x.combat.attacking) {
            crate::game::map::ai::enter_evade(map, c);
        }
    }
    // Its dead come back with the evade: on any member's (0x20), or on its
    // leader's, or one's while its leader lives and they evade together
    // (0x10).
    let flags = leader_of(map, g).map_or(0, |l| group_flags(map, l)) | script_flags(map, g);
    if flags & (GROUP_RESPAWN_ON_EVADE | GROUP_RESPAWN_ON_LEADER_EVADE) == 0 {
        return;
    }
    let members = members_of(map, g);
    let leader = leader_guid(map, g);
    let leader_alive = leader.is_some_and(|l| map.creature(l).is_some_and(|c| c.alive()));
    let leader_evades = leader == Some(g) || (flags & GROUP_EVADE_TOGETHER != 0 && leader_alive);
    if flags & GROUP_RESPAWN_ON_EVADE != 0
        || (flags & GROUP_RESPAWN_ON_LEADER_EVADE != 0 && leader_evades)
    {
        respawn_dead(map, &members, g);
    }
}

/// Whether `listener` hears of `dead`'s death, both of a group: its leader
/// when the group informs it of a member's, the rest when it informs them
/// of any (vmangos's `OPTION_INFORM_*`, Thistle Tea's `:member_died`).
pub fn hears_death(map: &Map, listener: Guid, dead: Guid) -> bool {
    let leader = leader_guid(map, dead);
    let flags = match leader {
        Some(l)
            if map.script_groups.contains_key(&dead)
                || map.script_groups.contains_key(&listener)
                || is_script_leader(map, l) =>
        {
            script_flags(map, l)
        }
        _ => leader_of(map, dead).map_or(0, |l| group_flags(map, l)),
    };
    if leader == Some(listener) {
        flags & GROUP_INFORM_LEADER_ON_MEMBER_DIED != 0
    } else {
        flags & GROUP_INFORM_MEMBERS_ON_ANY_DIED != 0
    }
}

fn is_script_leader(map: &Map, g: Guid) -> bool {
    map.script_groups.values().any(|(l, _, _, _)| *l == g)
}

/// The creature leading `g`'s group, if it is here.
fn leader_guid(map: &Map, g: Guid) -> Option<Guid> {
    if let Some((l, _, _, _)) = map.script_groups.get(&g) {
        return Some(*l);
    }
    if map.script_groups.values().any(|(l, _, _, _)| *l == g) {
        return Some(g);
    }
    leader_of(map, g).and_then(|s| creature_of_spawn(map, s))
}

/// The dead of `members` but `but` come back at once.
fn respawn_dead(map: &mut Map, members: &[Guid], but: Guid) {
    for m in members.iter().copied().filter(|m| *m != but) {
        if map.creature(m).is_some_and(|c| !c.alive()) {
            crate::game::map::ai::respawn_now(map, m);
        }
    }
}

/// `g` respawned: a group that respawns together, and links.
pub fn respawned(map: &mut Map, g: Guid) {
    leader_back(map, g);
    release_held(map);
    let data = map.realm.data.clone();
    let Some(spawn) = spawn_of(map, g) else {
        return;
    };
    // Any member back, the rest of its dead come with it.
    if leader_of(map, g).is_some_and(|l| group_flags(map, l) & GROUP_RESPAWN_TOGETHER != 0)
        || script_flags(map, g) & GROUP_RESPAWN_TOGETHER != 0
    {
        let members = members_of(map, g);
        respawn_dead(map, &members, g);
    }
    for (slave, (master, flags)) in &data.scripts.links {
        if *master != spawn {
            continue;
        }
        let Some(s) = creature_of_spawn(map, *slave) else {
            continue;
        };
        let alive = map.creature(s).is_some_and(|c| c.alive());
        if !alive && flags & LINK_RESPAWN_ON_RESPAWN != 0 {
            crate::game::map::ai::respawn_now(map, s);
        } else if alive && flags & LINK_DESPAWN_ON_RESPAWN != 0 {
            crate::game::map::scripts::despawn(map, s);
        }
    }
}

/// `g` was despawned alive: its living slaves flagged to go with it go
/// (vmangos's `LINKING_EVENT_DESPAWN`, Thistle Tea's `:despawn`).
pub fn despawned(map: &mut Map, g: Guid) {
    release_held(map);
    let Some(spawn) = spawn_of(map, g) else {
        return;
    };
    let data = map.realm.data.clone();
    for (slave, (master, flags)) in &data.scripts.links {
        if *master == spawn
            && flags & LINK_DESPAWN_ON_DESPAWN != 0
            && let Some(s) = creature_of_spawn(map, *slave)
            && s != g
            && map.creature(s).is_some_and(|c| c.alive())
        {
            crate::game::map::scripts::despawn(map, s);
        }
    }
}

/// The slaves held back by their master's life come in, those it now
/// allows.
fn release_held(map: &mut Map) {
    if map.held_slaves.is_empty() {
        return;
    }
    let free: Vec<u32> = map
        .held_slaves
        .iter()
        .copied()
        .filter(|s| may_spawn(map, *s))
        .collect();
    for s in free {
        map.held_slaves.remove(&s);
        crate::game::map::ai::spawn_creature_of(map, s);
    }
}

/// Whether a linked slave may spawn now: some won't while their master is
/// dead (or alive).
pub fn may_spawn(map: &Map, spawn: u32) -> bool {
    let Some((master, flags)) = map.realm.data.scripts.links.get(&spawn) else {
        return true;
    };
    let alive = creature_of_spawn(map, *master)
        .and_then(|m| map.creature(m))
        .map(|c| c.alive());
    let dead_saved = map.key.instance != 0
        && crate::game::instances::save_of(&map.realm, map.key.instance)
            .is_some_and(|s| s.dead.contains_key(master));
    // The first flag decides, with both (Naxxramas's trash): mangos's
    // `CanSpawn`, Thistle Tea's `spawn_allowed?`.
    if flags & LINK_CANT_SPAWN_IF_BOSS_DEAD != 0 {
        return !(alive == Some(false) || dead_saved);
    }
    if flags & LINK_CANT_SPAWN_IF_BOSS_ALIVE != 0 && alive == Some(true) {
        return false;
    }
    true
}

/// How far behind its leader a follower may fall before it is brought up
/// to it, and how much faster than its run it may go to keep up (Thistle
/// Tea's `BT.Formation`).
const BROUGHT_UP: f32 = 100.0;
const CATCH_UP: f32 = 1.3;

/// The group (by its leader's spawn) whose formation `g` leads: its own,
/// or one whose leader died that it leads meanwhile.
fn led_group(map: &Map, g: Guid) -> Option<u32> {
    if let Some((key, _)) = map.acting_leaders.iter().find(|(_, a)| **a == g) {
        return Some(*key);
    }
    let spawn = spawn_of(map, g)?;
    (map.realm.data.scripts.groups.contains_key(&spawn) && !map.acting_leaders.contains_key(&spawn))
        .then_some(spawn)
}

/// The followers that keep their places by `leader` as it walks: those a
/// script put in formation behind it, and its group's if the group walks
/// in formation; with their distance and angle.
fn formation_of(map: &Map, leader: Guid) -> Vec<(Guid, f32, f32)> {
    let mut out: Vec<(Guid, f32, f32)> = map
        .script_groups
        .iter()
        .filter(|(_, (l, _, _, f))| *l == leader && f & GROUP_FORMATION != 0)
        .map(|(m, (_, d, a, _))| (*m, *d, *a))
        .collect();
    let Some(key) = led_group(map, leader) else {
        return out;
    };
    let Some(members) = map.realm.data.scripts.groups.get(&key) else {
        return out;
    };
    if members.iter().fold(0, |f, m| f | m.flags) & GROUP_FORMATION == 0 {
        return out;
    }
    let own = spawn_of(map, leader);
    for m in members
        .iter()
        .filter(|m| m.guid != key && Some(m.guid) != own)
    {
        if let Some(c) = creature_of_spawn(map, m.guid) {
            out.push((c, m.distance, m.angle));
        }
    }
    out
}

/// `g` died: a formation it led on a patrol is led on along it by its
/// first member alive (by guid), from the point it was bound for, a
/// second later (Thistle Tea's `CreatureGroup.on_death`).
pub fn died(map: &mut Map, g: Guid) {
    release_held(map);
    let Some(key) = led_group(map, g) else {
        return;
    };
    let data = map.realm.data.clone();
    let Some(members) = data.scripts.groups.get(&key) else {
        return;
    };
    let path = data.waypoints.get(&key).filter(|p| !p.is_empty());
    let Some(path) = path.filter(|_| group_flags(map, key) & GROUP_FORMATION != 0) else {
        return;
    };
    let (bound_for, last) = map
        .creature(g)
        .map_or((0, None), |c| (c.waypoint, c.last_waypoint));
    let mut candidates: Vec<Guid> = members
        .iter()
        .filter(|m| m.guid != key)
        .filter_map(|m| creature_of_spawn(map, m.guid))
        .filter(|c| *c != g && map.creature(*c).is_some_and(|x| x.alive()))
        .collect();
    candidates.sort_by_key(|c| c.guid());
    let Some(next) = candidates.first().copied() else {
        map.acting_leaders.remove(&key);
        return;
    };
    map.acting_leaders.insert(key, next);
    let now = map.now;
    let points = std::sync::Arc::new(path.clone());
    let Some(c) = map.creature_mut(next) else {
        return;
    };
    let s = c.scripted.get_or_insert_with(Default::default);
    s.route = Some(crate::game::map::event_ai::Route {
        points,
        repeat: true,
    });
    c.movement_type = crate::game::map::creature::MOVEMENT_WAYPOINT;
    c.waypoint = bound_for;
    c.last_waypoint = last;
    if !c.combat.attacking && !c.evading {
        c.motion = crate::game::map::creature::Motion::Idle {
            until: Some(now + std::time::Duration::from_secs(1)),
        };
    }
}

/// `g` came back: the formation it leads, led meanwhile by another, is its
/// again, and the other falls in behind it.
fn leader_back(map: &mut Map, g: Guid) {
    let Some(spawn) = spawn_of(map, g) else {
        return;
    };
    let Some(acting) = map.acting_leaders.remove(&spawn) else {
        return;
    };
    let Some(c) = map.creature_mut(acting) else {
        return;
    };
    if let Some(s) = c.scripted.as_deref_mut() {
        s.route = None;
    }
    c.movement_type = crate::game::map::creature::MOVEMENT_IDLE;
    if !c.combat.attacking && !c.evading {
        c.motion = crate::game::map::creature::Motion::Idle { until: None };
    }
}

/// Whether `g` keeps its place in a formation rather than walking its own
/// way: one of a formation's members, not leading it meanwhile.
pub fn following(map: &Map, g: Guid) -> bool {
    let Some(c) = map.creature(g) else {
        return false;
    };
    follows(&map.realm.data, c.spawn_guid) && !map.acting_leaders.values().any(|a| *a == g)
}

/// Whether a follower is free to keep its place: alive, out of a fight,
/// not driven by fear, confusion or a flight, nor held by a root, stun or
/// distraction, and not rooted to its spot by its template.
fn free_to_follow(map: &Map, g: Guid) -> bool {
    use crate::game::map::auras::{AURA_MOD_ROOT, AURA_MOD_STUN};
    map.creature(g).is_some_and(|c| {
        c.alive()
            && !c.combat.attacking
            && !c.evading
            && c.engagement.control.is_none()
            && !c.auras.has(AURA_MOD_ROOT)
            && !c.auras.has(AURA_MOD_STUN)
    }) && !crate::game::map::creature_motion::held_by_distraction(map, g)
        && !crate::game::map::creature_motion::sessile(map, g)
}

/// A follower's place by a leader at `at` facing `heading`.
fn place(at: Vector3d, heading: f32, distance: f32, angle: f32) -> Vector3d {
    let a = heading + angle;
    Vector3d {
        x: at.x + distance * a.cos(),
        y: at.y + distance * a.sin(),
        z: at.z,
    }
}

/// A leader set off on a walk: its formation follows, as Thistle Tea's
/// `BT.Formation` has it, each to its place off where the walk ends, faced
/// along the walk's last leg, timed to arrive with it (but no faster than
/// 1.3 times its own run), and facing that way when there. One a hundred
/// yards or more from its leader is brought up to it at once.
pub fn leader_walked(map: &mut Map, leader: Guid) {
    let now = map.now;
    let Some(c) = map.creature(leader) else {
        return;
    };
    let crate::game::map::creature::Motion::Moving(w) = &c.motion else {
        return;
    };
    let dest = w.destination();
    let Some(prev) = w.points.iter().rev().find(|p| **p != dest).copied() else {
        return;
    };
    let heading = (dest.y - prev.y).atan2(dest.x - prev.x);
    let arrives = w.started + w.duration;
    let (at, facing) = (c.position, c.orientation);
    for (m, distance, angle) in formation_of(map, leader) {
        if !free_to_follow(map, m) {
            continue;
        }
        crate::game::map::ai::advance_motion(map, m);
        let Some(from) = map.creature(m).map(|c| c.position) else {
            continue;
        };
        if crate::game::map::combat::dist(from, at) >= BROUGHT_UP {
            crate::game::map::scripts::place(map, m, at, facing);
            continue;
        }
        let spot = place(dest, heading, distance, angle);
        let spot = map
            .nav
            .as_mut()
            .and_then(|n| n.ground(spot.x, spot.y, spot.z))
            .map(|z| Vector3d { z, ..spot })
            .unwrap_or(spot);
        if crate::game::map::combat::dist(from, spot) < 0.2 {
            continue;
        }
        let path = match map.nav.as_mut() {
            Some(nav) => match nav.path(from, spot) {
                Some(p) => p,
                None => continue,
            },
            None => vec![from, spot],
        };
        let length: f32 = path
            .windows(2)
            .map(|w| crate::game::map::combat::dist(w[0], w[1]))
            .sum();
        let left = arrives
            .saturating_duration_since(now)
            .max(std::time::Duration::from_millis(1));
        let fly = crate::game::map::creature_motion::habitat(map, m).fly;
        let spline_id = map.next_spline_id;
        map.next_spline_id = map.next_spline_id.wrapping_add(1);
        let Some(c) = map.creature_mut(m) else {
            continue;
        };
        let speed = (length / left.as_secs_f32()).min(c.speeds.run * CATCH_UP);
        let run = c.scripted.as_deref().is_some_and(|s| s.run);
        let mut walk = crate::game::map::creature::Walk::new(path, speed, now, spline_id, run);
        walk.fly = fly;
        walk.face = Some(heading);
        let frame = walk.frame(m, from);
        c.motion = crate::game::map::creature::Motion::Moving(walk);
        if let Some(s) = c.scripted.as_deref_mut() {
            s.scripted_move = None;
        }
        if let Some(frame) = frame {
            map.send_to_observers(m, &frame, false);
        }
    }
}

/// Where a follower goes home to after a fight: its place by its leader
/// as the leader now stands (Thistle Tea's `Formation.home_position`).
pub fn formation_home(map: &Map, g: Guid) -> Option<Vector3d> {
    let (leader, distance, angle) = match map.script_groups.get(&g) {
        Some((l, d, a, f)) if f & GROUP_FORMATION != 0 => (*l, *d, *a),
        _ => {
            let spawn = spawn_of(map, g)?;
            if !follows(&map.realm.data, Some(spawn)) {
                return None;
            }
            let leader = *map.realm.data.scripts.group_of.get(&spawn)?;
            let m = map
                .realm
                .data
                .scripts
                .groups
                .get(&leader)?
                .iter()
                .find(|m| m.guid == spawn)?;
            let leads = map
                .acting_leaders
                .get(&leader)
                .copied()
                .or_else(|| creature_of_spawn(map, leader))?;
            if leads == g {
                return None;
            }
            (leads, m.distance, m.angle)
        }
    };
    let l = map.creature(leader).filter(|l| l.alive())?;
    Some(place(l.position, l.orientation, distance, angle))
}

/// Whether a creature walks in a formation behind another, and so doesn't
/// walk its own way.
pub fn follows(data: &GameData, spawn: Option<u32>) -> bool {
    let Some(spawn) = spawn else {
        return false;
    };
    data.scripts.group_of.get(&spawn).is_some_and(|leader| {
        *leader != spawn
            && data
                .scripts
                .groups
                .get(leader)
                .is_some_and(|m| m.iter().fold(0, |f, x| f | x.flags) & GROUP_FORMATION != 0)
    })
}
