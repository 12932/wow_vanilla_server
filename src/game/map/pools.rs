//! Spawn pools (`pool_*`), as Thistle Tea's `SpawnPool.Selection` picks
//! them (vmangos's way). A pool spawns up to its limit (at least one) of
//! its members, and only of its first kind: pools, else objects, else
//! creatures. With a limit of one, an explicitly chanced member comes by
//! its chance and the rest evenly; with more, any of them evenly. A pool of
//! pools spawns its picks' picks. A member that comes back from death or
//! gathering is picked again: another of its pool may come instead, or, in
//! a pool of pools, another pool of its mother's. Only the spawns the
//! events on allow are in a pool; when they change, the pool picks anew.

use ahash::{AHashMap, AHashSet};
use rand::RngExt;

use crate::game::data::scripts::ScriptData;
use crate::game::map::Map;

/// How deep pools of pools go before they are taken as broken.
const DEPTH: u32 = 8;

/// A pool's member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Member {
    Creature(u32),
    Object(u32),
    Pool(u32),
}

/// What a pool, from its root down, has picked: each pool's own picks,
/// and the spawns among them.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    direct: AHashMap<u32, Vec<Member>>,
    pub leaves: AHashSet<Member>,
}

/// A root pool in a map: its picks, and the spawns it may pick from.
#[derive(Debug, Clone, Default)]
pub struct PoolState {
    pub selection: Selection,
    pub available: AHashSet<Member>,
}

fn root(s: &ScriptData, mut pool: u32) -> u32 {
    for _ in 0..DEPTH {
        match s.pools.get(&pool) {
            Some(p) if p.mother != 0 && s.pools.contains_key(&p.mother) => pool = p.mother,
            _ => break,
        }
    }
    pool
}

/// The pool a spawn is in.
fn pool_of(s: &ScriptData, m: Member) -> Option<u32> {
    match m {
        Member::Creature(g) => s.creature_pool.get(&g).copied(),
        Member::Object(g) => s.object_pool.get(&g).copied(),
        Member::Pool(p) => Some(p),
    }
}

/// A pool's members of its first kind (pools, then objects, then
/// creatures), with their chances.
fn members(s: &ScriptData, pool: u32) -> Vec<(Member, f32)> {
    let Some(p) = s.pools.get(&pool) else {
        return Vec::new();
    };
    let chance = |c: f32| c.abs();
    if !p.pools.is_empty() {
        p.pools
            .iter()
            .map(|(g, c)| (Member::Pool(*g), chance(*c)))
            .collect()
    } else if !p.objects.is_empty() {
        p.objects
            .iter()
            .map(|(g, c)| (Member::Object(*g), chance(*c)))
            .collect()
    } else {
        p.creatures
            .iter()
            .map(|(g, c)| (Member::Creature(*g), chance(*c)))
            .collect()
    }
}

fn limit(s: &ScriptData, pool: u32) -> usize {
    s.pools
        .get(&pool)
        .map_or(1, |p| p.max_limit.max(1) as usize)
}

/// Whether a member may be picked: a spawn that is here and allowed, or
/// a pool with one in it.
fn available(s: &ScriptData, m: Member, avail: &AHashSet<Member>, depth: u32) -> bool {
    match m {
        Member::Pool(p) => {
            depth < DEPTH
                && s.pools.get(&p).is_some_and(|x| {
                    x.pools
                        .iter()
                        .any(|(c, _)| available(s, Member::Pool(*c), avail, depth + 1))
                        || x.objects
                            .iter()
                            .any(|(g, _)| avail.contains(&Member::Object(*g)))
                        || x.creatures
                            .iter()
                            .any(|(g, _)| avail.contains(&Member::Creature(*g)))
                })
        }
        spawn => avail.contains(&spawn),
    }
}

/// One more of `pool`'s members, not one of `excluded`: by chance for a
/// pool of one, else evenly.
fn choose(
    s: &ScriptData,
    pool: u32,
    avail: &AHashSet<Member>,
    excluded: &[Member],
    rng: &mut impl rand::Rng,
) -> Option<Member> {
    let candidates: Vec<(Member, f32)> = members(s, pool)
        .into_iter()
        .filter(|(m, _)| !excluded.contains(m) && available(s, *m, avail, 0))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    if limit(s, pool) > 1 {
        return Some(candidates[rng.random_range(0..candidates.len())].0);
    }
    let explicit: Vec<&(Member, f32)> = candidates.iter().filter(|(_, c)| *c > 0.0).collect();
    if !explicit.is_empty() {
        let mut roll = rng.random_range(0.0f32..100.0);
        for (m, c) in explicit {
            roll -= c;
            if roll < 0.0 {
                return Some(*m);
            }
        }
    }
    let equal: Vec<Member> = candidates
        .iter()
        .filter(|(_, c)| *c <= 0.0)
        .map(|(m, _)| *m)
        .collect();
    (!equal.is_empty()).then(|| equal[rng.random_range(0..equal.len())])
}

impl Selection {
    /// `root`'s picks from what is available.
    pub fn initialize(
        s: &ScriptData,
        root: u32,
        avail: &AHashSet<Member>,
        rng: &mut impl rand::Rng,
    ) -> Self {
        let mut sel = Selection::default();
        sel.fill(s, root, avail, rng, 0);
        sel
    }

    fn fill(
        &mut self,
        s: &ScriptData,
        pool: u32,
        avail: &AHashSet<Member>,
        rng: &mut impl rand::Rng,
        depth: u32,
    ) {
        if depth > DEPTH {
            return;
        }
        let limit = limit(s, pool);
        loop {
            let picked = self.direct.get(&pool).cloned().unwrap_or_default();
            if picked.len() >= limit {
                return;
            }
            let Some(m) = choose(s, pool, avail, &picked, rng) else {
                return;
            };
            self.add(s, pool, m, avail, rng, depth);
        }
    }

    fn add(
        &mut self,
        s: &ScriptData,
        pool: u32,
        m: Member,
        avail: &AHashSet<Member>,
        rng: &mut impl rand::Rng,
        depth: u32,
    ) {
        self.direct.entry(pool).or_default().push(m);
        match m {
            Member::Pool(child) => self.fill(s, child, avail, rng, depth + 1),
            spawn => {
                self.leaves.insert(spawn);
            }
        }
    }

    fn remove(&mut self, m: Member) {
        for picks in self.direct.values_mut() {
            picks.retain(|x| *x != m);
        }
        match m {
            Member::Pool(child) => self.remove_tree(child, 0),
            spawn => {
                self.leaves.remove(&spawn);
            }
        }
    }

    fn remove_tree(&mut self, pool: u32, depth: u32) {
        let picks = self.direct.remove(&pool).unwrap_or_default();
        for m in picks {
            match m {
                Member::Pool(child) if depth < DEPTH => self.remove_tree(child, depth + 1),
                Member::Pool(_) => {}
                spawn => {
                    self.leaves.remove(&spawn);
                }
            }
        }
    }

    /// `trigger` came back: its pick is made again, in its pool or, if its
    /// pool is in another, among its mother's (Thistle Tea's
    /// `Selection.replace`).
    pub fn replace(
        &mut self,
        s: &ScriptData,
        trigger: Member,
        avail: &AHashSet<Member>,
        rng: &mut impl rand::Rng,
    ) {
        let Some(leaf) = pool_of(s, trigger) else {
            return;
        };
        let mother = s
            .pools
            .get(&leaf)
            .map(|p| p.mother)
            .filter(|m| *m != 0 && s.pools.contains_key(m));
        let (pool, m) = match mother {
            Some(mother) => (mother, Member::Pool(leaf)),
            None => (leaf, trigger),
        };
        if !self.direct.get(&pool).is_some_and(|p| p.contains(&m)) {
            return;
        }
        self.remove(m);
        let picked = self.direct.get(&pool).cloned().unwrap_or_default();
        if let Some(next) = choose(s, pool, avail, &picked, rng) {
            self.add(s, pool, next, avail, rng, 0);
        }
    }
}

/// The pooled spawns of this map by their root pool: all of them, and
/// those the events on let spawn. One pass over the map's spawns.
fn pooled_here(map: &Map, on: &AHashSet<u32>) -> AHashMap<u32, (Vec<Member>, AHashSet<Member>)> {
    let data = &map.realm.data;
    let s = &data.scripts;
    let mut out: AHashMap<u32, (Vec<Member>, AHashSet<Member>)> = AHashMap::new();
    if s.pools.is_empty() {
        return out;
    }
    for c in data.creature_spawns.get(&map.key.map).into_iter().flatten() {
        let Some(pool) = s.creature_pool.get(&c.guid) else {
            continue;
        };
        let (all, available) = out.entry(root(s, *pool)).or_default();
        let m = Member::Creature(c.guid);
        all.push(m);
        if crate::game::events::spawned(
            crate::game::events::ties(&data.event_creatures, c.guid),
            on,
        ) {
            available.insert(m);
        }
    }
    for o in data
        .gameobject_spawns
        .get(&map.key.map)
        .into_iter()
        .flatten()
    {
        let Some(pool) = s.object_pool.get(&o.guid) else {
            continue;
        };
        let (all, available) = out.entry(root(s, *pool)).or_default();
        let m = Member::Object(o.guid);
        all.push(m);
        if !o.despawned
            && crate::game::events::spawned(
                crate::game::events::ties(&data.event_objects, o.guid),
                on,
            )
        {
            available.insert(m);
        }
    }
    out
}

/// A map's pools pick as it fills: the pooled spawns that don't spawn,
/// (creatures, objects).
pub fn populate(
    map: &mut Map,
    on: &AHashSet<u32>,
    rng: &mut impl rand::Rng,
) -> (AHashSet<u32>, AHashSet<u32>) {
    let data = map.realm.data.clone();
    let s = &data.scripts;
    let mut skip = (AHashSet::new(), AHashSet::new());
    for (root, (all, available)) in pooled_here(map, on) {
        let selection = Selection::initialize(s, root, &available, rng);
        for m in all {
            if selection.leaves.contains(&m) {
                continue;
            }
            match m {
                Member::Creature(g) => skip.0.insert(g),
                Member::Object(g) => skip.1.insert(g),
                Member::Pool(_) => false,
            };
        }
        map.pools.insert(
            root,
            PoolState {
                selection,
                available,
            },
        );
    }
    skip
}

/// Whether a spawn is in the map now: a creature alive or dead, an object
/// up or waiting to come back.
fn present(map: &Map, m: Member) -> bool {
    match m {
        Member::Creature(g) => crate::game::map::scripts::creature_of_spawn(map, g).is_some(),
        Member::Object(g) => {
            crate::game::map::scripts::object_of_spawn(map, g).is_some()
                || map.loot_state.respawns.iter().any(|(_, s)| *s == g)
        }
        Member::Pool(_) => true,
    }
}

/// A pooled spawn comes back (from death, or gathered): its pool picks
/// again. The spawns its picks now have that aren't in the map, to bring
/// in; `None` for a spawn in no pool, which comes back itself.
pub fn came_back(map: &mut Map, trigger: Member) -> Option<Vec<Member>> {
    let data = map.realm.data.clone();
    let s = &data.scripts;
    let root = root(s, pool_of(s, trigger)?);
    let mut state = map.pools.remove(&root)?;
    state
        .selection
        .replace(s, trigger, &state.available, &mut rand::rng());
    let bring: Vec<Member> = state
        .selection
        .leaves
        .iter()
        .copied()
        .filter(|m| !present(map, *m))
        .collect();
    map.pools.insert(root, state);
    Some(bring)
}

/// The events changed: a pool whose spawns the events allow changed puts
/// away what it had out and picks anew (Thistle Tea's `refresh`). The
/// spawns to bring in.
pub fn events_changed(map: &mut Map, on: &AHashSet<u32>) -> Vec<Member> {
    let data = map.realm.data.clone();
    let s = &data.scripts;
    let mut bring = Vec::new();
    for (root, (all, available)) in pooled_here(map, on) {
        if map
            .pools
            .get(&root)
            .is_some_and(|p| p.available == available)
        {
            continue;
        }
        for m in all {
            match m {
                Member::Creature(g) => {
                    if let Some(c) = crate::game::map::scripts::creature_of_spawn(map, g) {
                        map.remove_creature(c);
                    }
                }
                Member::Object(g) => {
                    if let Some(o) = crate::game::map::scripts::object_of_spawn(map, g) {
                        map.remove_gameobject(o);
                    }
                    map.loot_state.respawns.retain(|(_, x)| *x != g);
                }
                Member::Pool(_) => {}
            }
        }
        let selection = Selection::initialize(s, root, &available, &mut rand::rng());
        bring.extend(selection.leaves.iter().copied());
        map.pools.insert(
            root,
            PoolState {
                selection,
                available,
            },
        );
    }
    bring
}

/// A spawn's object comes into the map, unless a battleground bars it.
pub fn spawn_object(map: &mut Map, spawn: u32) {
    let data = map.realm.data.clone();
    let Some(s) = data
        .gameobject_spawns
        .get(&map.key.map)
        .and_then(|l| l.iter().find(|s| s.guid == spawn))
    else {
        return;
    };
    if !crate::game::map::battleground::spawn_allowed(map, false, spawn) {
        return;
    }
    if let Some(t) = data.gameobject_templates.get(&s.entry) {
        map.add_gameobject(crate::game::map::gameobject::GameObject::from_spawn(t, s));
    }
}

/// Whether a spawn is pooled (its coming and going are its pool's).
pub fn pooled(map: &Map, m: Member) -> bool {
    pool_of(&map.realm.data.scripts, m)
        .is_some_and(|p| map.realm.data.scripts.pools.contains_key(&p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::data::scripts::Pool;

    fn data(pools: Vec<Pool>) -> ScriptData {
        let mut s = ScriptData::default();
        for p in pools {
            for (g, _) in &p.creatures {
                s.creature_pool.insert(*g, p.entry);
            }
            for (g, _) in &p.objects {
                s.object_pool.insert(*g, p.entry);
            }
            s.pools.insert(p.entry, p);
        }
        let mothers: Vec<(u32, u32)> = s
            .pools
            .values()
            .flat_map(|p| p.pools.iter().map(move |(c, _)| (*c, p.entry)))
            .collect();
        for (child, mother) in mothers {
            if let Some(c) = s.pools.get_mut(&child) {
                c.mother = mother;
            }
        }
        s
    }

    fn all(s: &ScriptData) -> AHashSet<Member> {
        s.creature_pool
            .keys()
            .map(|g| Member::Creature(*g))
            .chain(s.object_pool.keys().map(|g| Member::Object(*g)))
            .collect()
    }

    /// A pack of four, each with a chance of 100, its limit four: all four
    /// come (Dire Maul's first packs), not one.
    #[test]
    fn a_limit_over_one_takes_chanced_members_evenly() {
        let s = data(vec![Pool {
            entry: 1,
            max_limit: 4,
            creatures: (1..=4).map(|g| (g, 100.0)).collect(),
            ..Default::default()
        }]);
        let sel = Selection::initialize(&s, 1, &all(&s), &mut rand::rng());
        assert_eq!(sel.leaves.len(), 4, "{:?}", sel.leaves);
    }

    /// A pool of one by chance: a chanced member by its chance; with none
    /// rolled, one of the rest.
    #[test]
    fn a_pool_of_one_rolls_its_chances() {
        let s = data(vec![Pool {
            entry: 1,
            max_limit: 1,
            creatures: vec![(1, 100.0), (2, 0.0)],
            ..Default::default()
        }]);
        for _ in 0..50 {
            let sel = Selection::initialize(&s, 1, &all(&s), &mut rand::rng());
            assert!(sel.leaves.contains(&Member::Creature(1)) && sel.leaves.len() == 1);
        }
    }

    /// One that comes back is picked again among its pool, chanced or not;
    /// in a pool of pools, among its mother's pools.
    #[test]
    fn a_member_coming_back_is_picked_again() {
        let s = data(vec![Pool {
            entry: 1,
            max_limit: 1,
            creatures: vec![(1, 50.0), (2, 50.0)],
            ..Default::default()
        }]);
        let avail = all(&s);
        let mut sel = Selection::initialize(&s, 1, &avail, &mut rand::rng());
        let mut seen = AHashSet::new();
        for _ in 0..40 {
            let m = *sel.leaves.iter().next().unwrap();
            seen.insert(m);
            sel.replace(&s, m, &avail, &mut rand::rng());
            assert_eq!(sel.leaves.len(), 1);
        }
        assert_eq!(seen.len(), 2, "both come in turn: {seen:?}");
        let s = data(vec![
            Pool {
                entry: 10,
                max_limit: 1,
                pools: vec![(11, 0.0), (12, 0.0)],
                ..Default::default()
            },
            Pool {
                entry: 11,
                max_limit: 1,
                creatures: vec![(1, 0.0)],
                ..Default::default()
            },
            Pool {
                entry: 12,
                max_limit: 1,
                creatures: vec![(2, 0.0)],
                ..Default::default()
            },
        ]);
        let avail = all(&s);
        let mut sel = Selection::initialize(&s, 10, &avail, &mut rand::rng());
        let mut seen = AHashSet::new();
        for _ in 0..40 {
            let m = *sel.leaves.iter().next().unwrap();
            seen.insert(m);
            sel.replace(&s, m, &avail, &mut rand::rng());
            assert_eq!(sel.leaves.len(), 1);
        }
        assert_eq!(seen.len(), 2, "the mother picks again: {seen:?}");
    }

    /// Only what is available is picked: an event's spawn not on isn't.
    #[test]
    fn only_the_available_are_picked() {
        let s = data(vec![Pool {
            entry: 1,
            max_limit: 2,
            creatures: vec![(1, 0.0), (2, 0.0), (3, 0.0)],
            ..Default::default()
        }]);
        let avail: AHashSet<Member> = [Member::Creature(1)].into_iter().collect();
        let sel = Selection::initialize(&s, 1, &avail, &mut rand::rng());
        assert_eq!(
            sel.leaves.iter().copied().collect::<Vec<_>>(),
            vec![Member::Creature(1)]
        );
    }
}
