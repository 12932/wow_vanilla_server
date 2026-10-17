//! Creature groups (`creature_groups`), links (`creature_linking`) and
//! spawn pools (`pool_*`), as vmangos has them and Thistle Tea ports them.
//! A group fights, evades and respawns together as its flags say, and
//! walks in formation behind its leader; a linked slave follows its
//! master's aggro, evade and respawn; a pool spawns at most its limit of
//! its members, picked by their chances.

use ahash::AHashSet;
use rand::RngExt;
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
    if let Some((leader, _, _)) = map.script_groups.get(&g) {
        return spawn_of(map, *leader).or(Some(0));
    }
    let spawn = spawn_of(map, g)?;
    map.realm.data.scripts.group_of.get(&spawn).copied()
}

pub fn is_leader(map: &Map, g: Guid) -> bool {
    spawn_of(map, g).is_some_and(|s| map.realm.data.scripts.groups.contains_key(&s))
        || map.script_groups.values().any(|(l, _, _)| *l == g)
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
    if let Some((leader, _, _)) = map.script_groups.get(&g).copied() {
        let mut out = vec![leader];
        out.extend(
            map.script_groups
                .iter()
                .filter(|(_, (l, _, _))| *l == leader)
                .map(|(m, _)| *m),
        );
        return out;
    }
    if map.script_groups.values().any(|(l, _, _)| *l == g) {
        let mut out = vec![g];
        out.extend(
            map.script_groups
                .iter()
                .filter(|(_, (l, _, _))| *l == g)
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

/// Whether every member of `g`'s group is dead.
pub fn all_dead(map: &Map, g: Guid) -> bool {
    let members = members_of(map, g);
    !members.is_empty()
        && members
            .iter()
            .all(|m| map.creature(*m).is_none_or(|c| !c.alive()))
}

/// A script puts `g` in `leader`'s group, at a distance and angle.
pub fn join(map: &mut Map, g: Guid, leader: Guid, distance: f32, angle: f32) {
    if g != leader && guid::is_unit(leader) {
        map.script_groups
            .insert(g, (leader, distance, angle.to_radians()));
    }
}

pub fn leave(map: &mut Map, g: Guid) {
    map.script_groups.remove(&g);
}

/// `g` was attacked by `enemy`: its group and its links join in.
pub fn aggro(map: &mut Map, g: Guid, enemy: Guid) {
    let mut join: Vec<Guid> = Vec::new();
    if let Some(leader) = leader_of(map, g)
        && (group_flags(map, leader) & GROUP_AGGRO_TOGETHER != 0
            || map.script_groups.contains_key(&g))
    {
        join.extend(members_of(map, g));
    }
    let Some(spawn) = spawn_of(map, g) else {
        attack_all(map, join, enemy);
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
    attack_all(map, join, enemy);
}

fn attack_all(map: &mut Map, who: Vec<Guid>, enemy: Guid) {
    for c in who {
        if map
            .creature(c)
            .is_some_and(|x| x.alive() && !x.combat.attacking && !x.evading)
            && crate::game::map::combat::can_attack(map, c, enemy)
        {
            crate::game::map::ai::attacked(map, c, enemy);
        }
    }
}

/// `g` gave up its fight: its group, if they evade together, and its
/// links.
pub fn evaded(map: &mut Map, g: Guid) {
    let mut evade: Vec<Guid> = Vec::new();
    if let Some(leader) = leader_of(map, g)
        && group_flags(map, leader) & GROUP_EVADE_TOGETHER != 0
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
}

/// `g` respawned: a group that respawns together, and links.
pub fn respawned(map: &mut Map, g: Guid) {
    let data = map.realm.data.clone();
    let Some(spawn) = spawn_of(map, g) else {
        return;
    };
    if data.scripts.groups.contains_key(&spawn)
        && group_flags(map, spawn) & GROUP_RESPAWN_TOGETHER != 0
    {
        for m in members_of(map, g).into_iter().filter(|m| *m != g) {
            if map.creature(m).is_some_and(|c| !c.alive()) {
                crate::game::map::ai::respawn_now(map, m);
            }
        }
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
    if flags & LINK_CANT_SPAWN_IF_BOSS_DEAD != 0 && (alive == Some(false) || dead_saved) {
        return false;
    }
    if flags & LINK_CANT_SPAWN_IF_BOSS_ALIVE != 0 && alive == Some(true) {
        return false;
    }
    true
}

/// A leader starts a walk to `to`: its formation follows, each at its
/// place behind it.
pub fn leader_walked(map: &mut Map, leader: Guid, to: Vector3d, facing: f32, run: bool) {
    let Some(spawn) = spawn_of(map, leader) else {
        return;
    };
    let data = map.realm.data.clone();
    let Some(members) = data.scripts.groups.get(&spawn) else {
        return;
    };
    if members.iter().fold(0, |f, m| f | m.flags) & GROUP_FORMATION == 0 {
        return;
    }
    for m in members.iter().filter(|m| m.guid != spawn) {
        let Some(c) = creature_of_spawn(map, m.guid) else {
            continue;
        };
        let ok = map
            .creature(c)
            .is_some_and(|x| x.alive() && !x.combat.attacking && !x.evading);
        if !ok {
            continue;
        }
        let angle = facing + m.angle.to_radians();
        let place = Vector3d {
            x: to.x + m.distance * angle.cos(),
            y: to.y + m.distance * angle.sin(),
            z: to.z,
        };
        let place = map
            .nav
            .as_mut()
            .and_then(|n| n.ground(place.x, place.y, place.z))
            .map(|z| Vector3d { z, ..place })
            .unwrap_or(place);
        crate::game::map::scripts::move_to(map, c, place, run);
    }
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

/// The pooled spawns that don't spawn this time: (creatures, objects).
/// Pools spawn at most their limit, an explicitly chanced member by its
/// chance, the rest evenly; a pool of pools spawns its picks' picks.
pub fn unpicked(
    data: &GameData,
    map: u32,
    rng: &mut impl rand::Rng,
) -> (AHashSet<u32>, AHashSet<u32>) {
    let s = &data.scripts;
    let mut out = (AHashSet::new(), AHashSet::new());
    if s.pools.is_empty() {
        return out;
    }
    let on_map_creature: AHashSet<u32> = data
        .creature_spawns
        .get(&map)
        .map(|v| {
            v.iter()
                .map(|c| c.guid)
                .filter(|g| s.creature_pool.contains_key(g))
                .collect()
        })
        .unwrap_or_default();
    let on_map_object: AHashSet<u32> = data
        .gameobject_spawns
        .get(&map)
        .map(|v| {
            v.iter()
                .map(|o| o.guid)
                .filter(|g| s.object_pool.contains_key(g))
                .collect()
        })
        .unwrap_or_default();
    if on_map_creature.is_empty() && on_map_object.is_empty() {
        return out;
    }
    // Every pooled spawn here starts unpicked; the picks are taken out.
    out.0 = on_map_creature.clone();
    out.1 = on_map_object.clone();
    let mut roots: AHashSet<u32> = AHashSet::new();
    for g in &on_map_creature {
        roots.insert(root(s, s.creature_pool[g]));
    }
    for g in &on_map_object {
        roots.insert(root(s, s.object_pool[g]));
    }
    for r in roots {
        pick(s, r, rng, &mut out, 0);
    }
    out
}

fn root(s: &ScriptData, mut pool: u32) -> u32 {
    for _ in 0..8 {
        match s.pools.get(&pool) {
            Some(p) if p.mother != 0 && s.pools.contains_key(&p.mother) => pool = p.mother,
            _ => break,
        }
    }
    pool
}

#[derive(Clone, Copy)]
enum Member {
    Creature(u32),
    Object(u32),
    Pool(u32),
}

fn pick(
    s: &ScriptData,
    pool: u32,
    rng: &mut impl rand::Rng,
    out: &mut (AHashSet<u32>, AHashSet<u32>),
    depth: u32,
) {
    let Some(p) = s.pools.get(&pool) else {
        return;
    };
    if depth > 8 {
        return;
    }
    let members: Vec<(Member, f32)> = p
        .creatures
        .iter()
        .map(|(g, c)| (Member::Creature(*g), *c))
        .chain(p.objects.iter().map(|(g, c)| (Member::Object(*g), *c)))
        .chain(p.pools.iter().map(|(g, c)| (Member::Pool(*g), *c)))
        .collect();
    if members.is_empty() {
        return;
    }
    let limit = if p.max_limit == 0 {
        members.len()
    } else {
        p.max_limit as usize
    };
    let mut chosen: Vec<Member> = Vec::new();
    // One of the explicitly chanced, by its chance.
    let explicit: Vec<&(Member, f32)> = members.iter().filter(|(_, c)| *c > 0.0).collect();
    if !explicit.is_empty() {
        let mut roll = rng.random_range(0.0f32..100.0);
        for (m, c) in explicit {
            roll -= c;
            if roll < 0.0 {
                chosen.push(*m);
                break;
            }
        }
    }
    // The rest evenly, up to the limit.
    let mut equal: Vec<Member> = members
        .iter()
        .filter(|(_, c)| *c <= 0.0)
        .map(|(m, _)| *m)
        .collect();
    while chosen.len() < limit && !equal.is_empty() {
        let i = rng.random_range(0..equal.len());
        chosen.push(equal.swap_remove(i));
    }
    for m in chosen {
        match m {
            Member::Creature(g) => {
                out.0.remove(&g);
            }
            Member::Object(g) => {
                out.1.remove(&g);
            }
            Member::Pool(child) => pick(s, child, rng, out, depth + 1),
        }
    }
}

/// A pooled creature died: the spawn that comes back may be another of its
/// pool, one not up now.
pub fn respawn_choice(map: &Map, spawn: u32) -> u32 {
    let s = &map.realm.data.scripts;
    let Some(pool) = s.creature_pool.get(&spawn).and_then(|p| s.pools.get(p)) else {
        return spawn;
    };
    if pool.creatures.len() < 2 || pool.creatures.iter().any(|(_, c)| *c > 0.0) {
        return spawn;
    }
    let free: Vec<u32> = pool
        .creatures
        .iter()
        .map(|(g, _)| *g)
        .filter(|g| *g == spawn || creature_of_spawn(map, *g).is_none())
        .collect();
    if free.is_empty() {
        return spawn;
    }
    free[rand::rng().random_range(0..free.len())]
}
