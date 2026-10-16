//! Loot: rolled from vmangos's tables when a creature dies or a chest is
//! opened, held on the corpse or object, and taken through the loot
//! window.

use std::sync::Arc;

use ahash::AHashSet;
use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    SMSG_LOOT_CLEAR_MONEY, SMSG_LOOT_RELEASE_RESPONSE, SMSG_LOOT_REMOVED,
};

use crate::game::data::items::Body;
use crate::game::data::{GameData, LootEntry};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::combat::dist;
use crate::game::map::inventory;

/// `UNIT_DYNAMIC_FLAGS`: there is loot on the corpse.
pub const DYNFLAG_LOOTABLE: u32 = 0x1;
pub const SMSG_LOOT_RESPONSE: u16 = 0x160;
/// How far from a corpse a player may loot it.
pub const LOOT_YARDS: f32 = 6.0;
/// References nest this deep at most.
const MAX_DEPTH: u32 = 4;
/// `UNIT_FIELD_FLAGS`: the player has a loot window open.
const UNIT_FLAG_LOOTING: u32 = 0x400;

#[derive(Debug, Clone, Default)]
pub struct LootSlot {
    pub item: u32,
    pub count: u32,
    pub taken: bool,
}

/// What a corpse or object holds.
#[derive(Debug, Clone, Default)]
pub struct Loot {
    pub items: Vec<LootSlot>,
    pub gold: u32,
    /// Who may take it; `None` for anyone.
    pub owner: Option<Guid>,
    /// Who has the window open.
    pub looters: AHashSet<Guid>,
}

impl Loot {
    pub fn is_empty(&self) -> bool {
        self.gold == 0 && self.items.iter().all(|s| s.taken)
    }
}

fn chance(rng: &mut impl rand::Rng, percent: f32) -> bool {
    percent >= 100.0 || rng.random_range(0.0f32..100.0) < percent
}

fn count(rng: &mut impl rand::Rng, e: &LootEntry) -> u32 {
    let min = e.min.max(1) as u32;
    let max = e.max.max(min);
    if max > min {
        rng.random_range(min..=max)
    } else {
        min
    }
}

fn is_quest_item(e: &LootEntry) -> bool {
    e.chance < 0.0 && e.min >= 0
}

fn reference(
    data: &GameData,
    e: &LootEntry,
    wanted: &[u32],
    rng: &mut impl rand::Rng,
    out: &mut Vec<LootSlot>,
    depth: u32,
) {
    if let Some(r) = data.loot.reference.get(&(e.min.unsigned_abs())) {
        for _ in 0..e.max.max(1) {
            roll(data, r, wanted, rng, out, depth + 1);
        }
    }
}

/// Roll a table, vmangos's way: each ungrouped entry on its own chance;
/// one entry per group (the explicitly chanced in order, else one of the
/// equally chanced); references rolled into as many times as they say.
/// Quest items drop only if they are `wanted`.
pub fn roll(
    data: &GameData,
    entries: &[LootEntry],
    wanted: &[u32],
    rng: &mut impl rand::Rng,
    out: &mut Vec<LootSlot>,
    depth: u32,
) {
    let skip = |e: &LootEntry| is_quest_item(e) && !wanted.contains(&e.item);
    if depth > MAX_DEPTH {
        return;
    }
    let mut groups: Vec<u8> = entries
        .iter()
        .map(|e| e.group)
        .filter(|g| *g != 0)
        .collect();
    groups.sort_unstable();
    groups.dedup();
    for e in entries.iter().filter(|e| e.group == 0 && !skip(e)) {
        if !chance(rng, e.chance.abs()) {
            continue;
        }
        if e.min < 0 {
            reference(data, e, wanted, rng, out, depth);
        } else {
            out.push(LootSlot {
                item: e.item,
                count: count(rng, e),
                taken: false,
            });
        }
    }
    for g in groups {
        let members: Vec<&LootEntry> = entries
            .iter()
            .filter(|e| e.group == g && !skip(e))
            .collect();
        let mut r = rng.random_range(0.0f32..100.0);
        let mut picked = None;
        for e in members.iter().filter(|e| e.chance > 0.0) {
            r -= e.chance;
            if r < 0.0 {
                picked = Some(**e);
                break;
            }
        }
        if picked.is_none() {
            let equal: Vec<&&LootEntry> = members.iter().filter(|e| e.chance == 0.0).collect();
            if !equal.is_empty() {
                picked = Some(**equal[rng.random_range(0..equal.len())]);
            }
        }
        let Some(e) = picked else {
            continue;
        };
        if e.min < 0 {
            reference(data, &e, wanted, rng, out, depth);
        } else {
            out.push(LootSlot {
                item: e.item,
                count: count(rng, &e),
                taken: false,
            });
        }
    }
}

/// A creature died: its loot is rolled for its tapper.
pub fn creature_died(map: &mut Map, creature: Guid, tapper: Guid) {
    let data = map.realm.data.clone();
    let wanted = map
        .players
        .get(&tapper)
        .map(|p| crate::game::map::quests::wanted_items(p, &data))
        .unwrap_or_default();
    let Some(c) = map.creature_mut(creature) else {
        return;
    };
    let Some(t) = data.creature_template(c.entry) else {
        return;
    };
    let mut rng = rand::rng();
    let mut items = Vec::new();
    if t.loot_id != 0
        && let Some(table) = data.loot.creature.get(&t.loot_id)
    {
        roll(&data, table, &wanted, &mut rng, &mut items, 0);
    }
    let gold = if t.gold_max > t.gold_min {
        rng.random_range(t.gold_min..=t.gold_max)
    } else {
        t.gold_min
    };
    let loot = Loot {
        items,
        gold,
        owner: guid::is_player(tapper).then_some(tapper),
        looters: AHashSet::new(),
    };
    let lootable = !loot.is_empty();
    c.loot = Some(loot);
    c.fields
        .set_flag(UNIT_DYNAMIC_FLAGS, DYNFLAG_LOOTABLE, lootable);
}

/// `SMSG_LOOT_RESPONSE` in 1.12's layout: each item with its count, look
/// and random properties.
pub fn response(data: &GameData, g: Guid, loot: &Loot) -> Arc<[u8]> {
    let mut b = Body::default();
    b.u64(g.guid()).u8(1).u32(loot.gold);
    let shown: Vec<(usize, &LootSlot)> = loot
        .items
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.taken)
        .collect();
    b.u8(shown.len() as u8);
    for (i, s) in shown {
        let display = data.item(s.item).map(|t| t.display_id).unwrap_or(0);
        b.u8(i as u8)
            .u32(s.item)
            .u32(s.count)
            .u32(display)
            .u32(0)
            .u32(0)
            .u8(0);
    }
    b.frame(SMSG_LOOT_RESPONSE)
}

/// The loot on `g` (a creature or game object), mutably.
fn loot_mut(map: &mut Map, g: Guid) -> Option<&mut Loot> {
    if guid::is_unit(g) {
        map.creature_mut(g).and_then(|c| c.loot.as_mut())
    } else if guid::is_game_object(g) {
        map.gameobject_mut(g).and_then(|o| o.loot.as_mut())
    } else {
        None
    }
}

/// `CMSG_LOOT`: open the loot window on a corpse or object.
pub fn open(map: &mut Map, player: Guid, target: Guid) {
    let data = map.realm.data.clone();
    let Some(ppos) = map.players.get(&player).map(|p| p.position()) else {
        return;
    };
    let tpos = if guid::is_unit(target) {
        match map.creature(target) {
            Some(c) if !c.alive() => c.position,
            _ => return,
        }
    } else if let Some(o) = map.gameobject(target) {
        o.position
    } else {
        return;
    };
    if dist(ppos, tpos) > LOOT_YARDS {
        return;
    }
    let Some(loot) = loot_mut(map, target) else {
        return;
    };
    if loot.owner.is_some_and(|o| o != player) {
        return;
    }
    loot.looters.insert(player);
    let frame = response(&data, target, loot);
    if let Some(p) = map.players.get_mut(&player) {
        p.looting = Some(target);
        p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_LOOTING, true);
        p.outbound.send_frame(frame);
    }
}

/// `CMSG_AUTOSTORE_LOOT_ITEM`: take an item from the open window.
pub fn take_item(map: &mut Map, player: Guid, slot: u8) {
    let Some(target) = map.players.get(&player).and_then(|p| p.looting) else {
        return;
    };
    let Some((item, count)) = loot_mut(map, target).and_then(|l| {
        l.items
            .get(usize::from(slot))
            .filter(|s| !s.taken)
            .map(|s| (s.item, s.count))
    }) else {
        return;
    };
    match inventory::add_item(map, player, item, count, true) {
        Ok(()) => {
            let looters: Vec<Guid> = match loot_mut(map, target) {
                Some(l) => {
                    if let Some(s) = l.items.get_mut(usize::from(slot)) {
                        s.taken = true;
                    }
                    l.looters.iter().copied().collect()
                }
                None => Vec::new(),
            };
            for g in looters {
                if let Some(p) = map.players.get(&g) {
                    p.outbound.send(&SMSG_LOOT_REMOVED { slot });
                }
            }
        }
        Err(f) => {
            if let Some(p) = map.players.get(&player) {
                inventory::send_fail(p, f, Guid::zero());
            }
        }
    }
}

/// `CMSG_LOOT_MONEY`.
pub fn take_money(map: &mut Map, player: Guid) {
    let Some(target) = map.players.get(&player).and_then(|p| p.looting) else {
        return;
    };
    let Some((gold, looters)) = loot_mut(map, target).map(|l| {
        let gold = std::mem::take(&mut l.gold);
        (gold, l.looters.iter().copied().collect::<Vec<_>>())
    }) else {
        return;
    };
    if gold == 0 {
        return;
    }
    if let Some(p) = map.players.get_mut(&player) {
        let money = p.money().saturating_add(gold);
        p.set_money(money);
    }
    for g in looters {
        if let Some(p) = map.players.get(&g) {
            p.outbound.send(&SMSG_LOOT_CLEAR_MONEY {});
        }
    }
}

/// `CMSG_LOOT_RELEASE`: the window closes. An emptied corpse is no longer
/// lootable.
pub fn release(map: &mut Map, player: Guid, target: Guid) {
    if let Some(p) = map.players.get_mut(&player) {
        p.looting = None;
        p.fields
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_LOOTING, false);
        p.outbound.send(&SMSG_LOOT_RELEASE_RESPONSE {
            guid: target,
            unknown1: 1,
        });
    }
    let empty = match loot_mut(map, target) {
        Some(l) => {
            l.looters.remove(&player);
            l.is_empty()
        }
        None => return,
    };
    if empty && let Some(c) = map.creature_mut(target) {
        c.fields
            .set_flag(UNIT_DYNAMIC_FLAGS, DYNFLAG_LOOTABLE, false);
    }
}

/// A chest or node is opened: its loot, rolled now if it hasn't been.
pub fn open_object(map: &mut Map, player: Guid, object: Guid) {
    let data = map.realm.data.clone();
    let Some(o) = map.gameobject_mut(object) else {
        return;
    };
    if o.loot.is_none() {
        let Some(t) = data.gameobject_templates.get(&o.entry) else {
            return;
        };
        // A chest's loot table is its second data field.
        let loot_id = t.data[1];
        let mut items = Vec::new();
        if let Some(table) = data.loot.gameobject.get(&loot_id) {
            roll(&data, table, &[], &mut rand::rng(), &mut items, 0);
        }
        let gold = if t.gold_max > t.gold_min {
            rand::rng().random_range(t.gold_min..=t.gold_max)
        } else {
            t.gold_min
        };
        o.loot = Some(Loot {
            items,
            gold,
            owner: None,
            looters: AHashSet::new(),
        });
    }
    open(map, player, object);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn a_group_drops_one_and_a_sure_drop_always_drops() {
        let data = GameData::empty();
        let entry = |item, chance, group| LootEntry {
            item,
            chance,
            group,
            min: 1,
            max: 1,
            condition: 0,
        };
        let entries = vec![
            entry(1, 100.0, 0),
            entry(2, 0.0, 1),
            entry(3, 0.0, 1),
            entry(4, -50.0, 0),
        ];
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        for _ in 0..100 {
            let mut out = Vec::new();
            roll(&data, &entries, &[], &mut rng, &mut out, 0);
            assert!(out.iter().any(|s| s.item == 1));
            assert_eq!(out.iter().filter(|s| s.item == 2 || s.item == 3).count(), 1);
            assert!(
                out.iter().all(|s| s.item != 4),
                "no quest items without the quest"
            );
        }
    }
}
