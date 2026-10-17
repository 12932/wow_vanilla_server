//! Loot: rolled from vmangos's tables when a creature dies or a chest is
//! opened, held on the corpse or object, and taken through the loot
//! window.
//!
//! A group's kill follows its loot method, as Thistle Tea's loot session
//! does: free for all, round robin (the next member's), master loot (items
//! at the threshold kept for the master looter to give), and group loot or
//! need before greed (items at the threshold rolled for, 60 seconds, need
//! over greed). Quest items drop for each member who needs them; gold is
//! split among the members near.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ahash::AHashSet;
use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::{
    GroupLootSetting, ItemQuality, SMSG_LOOT_CLEAR_MONEY, SMSG_LOOT_RELEASE_RESPONSE,
    SMSG_LOOT_REMOVED,
};

use crate::game::data::items::Body;
use crate::game::data::{GameData, ItemTemplate, LootEntry};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::combat::{GROUP_REWARD_YARDS, dist};
use crate::game::map::inventory;
use crate::game::map::player::Player;

/// `UNIT_DYNAMIC_FLAGS`: there is loot on the corpse.
pub const DYNFLAG_LOOTABLE: u32 = 0x1;
pub const SMSG_LOOT_RESPONSE: u16 = 0x160;
pub const SMSG_LOOT_ALL_PASSED: u16 = 0x29E;
pub const SMSG_LOOT_ROLL_WON: u16 = 0x29F;
pub const SMSG_LOOT_START_ROLL: u16 = 0x2A1;
pub const SMSG_LOOT_ROLL: u16 = 0x2A2;
pub const SMSG_LOOT_MASTER_LIST: u16 = 0x2A4;
pub const SMSG_LOOT_MONEY_NOTIFY: u16 = 0x163;
/// How far from a corpse a player may loot it.
pub const LOOT_YARDS: f32 = 6.0;
/// How long a roll stands.
pub const ROLL_FOR: Duration = Duration::from_secs(60);
/// References nest this deep at most (Thistle Tea's `@max_reference_depth`).
const MAX_DEPTH: u32 = 3;
/// A loot window holds this many items at most.
pub const MAX_ITEMS: usize = 16;
/// `UNIT_FIELD_FLAGS`: the player has a loot window open.
const UNIT_FLAG_LOOTING: u32 = 0x400;

/// Loot slot types, as the window shows them.
const SLOT_ALLOW: u8 = 0;
const SLOT_MASTER: u8 = 2;

/// Votes, as the client sends them.
pub const VOTE_PASS: u8 = 0;
pub const VOTE_NEED: u8 = 1;
pub const VOTE_GREED: u8 = 2;

/// What the loot is of: the window's loot type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LootKind {
    #[default]
    Corpse,
    Pickpocket,
    Fishing,
    Disenchant,
    Skinning,
    /// Behind a lock: a chest picked or gathered, a lockbox unlocked.
    Opened,
}

impl LootKind {
    fn wire(self) -> u8 {
        match self {
            LootKind::Corpse => 1,
            // The client has no skinning type: pickpocketing's window, as
            // a lock opened shows too.
            LootKind::Pickpocket | LootKind::Skinning | LootKind::Opened => 2,
            LootKind::Fishing => 3,
            // An item's loot shows as Thistle Tea and mangos show it.
            LootKind::Disenchant => 2,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LootSlot {
    pub item: u32,
    pub count: u32,
    pub taken: bool,
    pub random_property: u32,
    /// A quest item: each who needs it takes their own.
    pub quest: bool,
    pub taken_by: Vec<Guid>,
    /// Kept for the master looter, or rolled for.
    pub blocked: bool,
    pub quality: u32,
    /// A condition the viewer must meet to see it.
    pub condition: u32,
}

/// A roll under way for one slot.
#[derive(Debug, Clone)]
pub struct Roll {
    pub slot: usize,
    pub eligible: Vec<Guid>,
    pub votes: Vec<(Guid, u8)>,
    pub ends: Instant,
}

/// What a corpse or object holds.
#[derive(Debug, Clone, Default)]
pub struct Loot {
    pub items: Vec<LootSlot>,
    pub gold: u32,
    /// Who may take it alone; `None` for anyone (or the group in `allowed`).
    pub owner: Option<Guid>,
    /// The group members it fell to; empty for one player or anyone.
    pub allowed: Vec<Guid>,
    pub method: Option<GroupLootSetting>,
    pub master: Option<Guid>,
    /// Round robin's looter.
    pub assigned: Option<Guid>,
    pub rolls: Vec<Roll>,
    pub kind: LootKind,
    /// Who has the window open.
    pub looters: AHashSet<Guid>,
    /// Behind a lock: those who opened it, till they close the window
    /// (Thistle Tea's `opened_by`).
    pub opened_by: AHashSet<Guid>,
}

impl Loot {
    pub fn is_empty(&self) -> bool {
        self.gold == 0 && self.items.iter().all(|s| s.taken || s.quest) && self.rolls.is_empty()
    }

    /// Whether `g` may open it.
    pub fn may_loot(&self, g: Guid) -> bool {
        if self.kind == LootKind::Opened && !self.opened_by.contains(&g) {
            return false;
        }
        if !self.allowed.is_empty() {
            return self.allowed.contains(&g)
                && (self.method != Some(GroupLootSetting::RoundRobin)
                    || self.assigned.is_none_or(|a| a == g));
        }
        self.owner.is_none_or(|o| o == g)
    }

    /// Whether `g` sees slot `s`, needing `wanted` and meeting the
    /// conditions `met`.
    fn visible(&self, s: &LootSlot, g: Guid, wanted: &[u32], met: &[u32]) -> bool {
        if s.taken || (s.condition != 0 && !met.contains(&s.condition)) {
            return false;
        }
        if s.quest {
            return wanted.contains(&s.item) && !s.taken_by.contains(&g);
        }
        !s.blocked || (self.master == Some(g) && self.rolls.is_empty())
    }
}

/// A map's loot work between updates: corpses with rolls under way, and
/// gathered nodes waiting to come back.
#[derive(Debug, Default)]
pub struct MapLoot {
    pub rolling: Vec<Guid>,
    /// (when, the spawn's guid).
    pub respawns: Vec<(Instant, u32)>,
    /// When temporary enchantments last ran down.
    pub enchant_tick_at: Option<Instant>,
    /// When timed items last ran down.
    pub items_tick_at: Option<Instant>,
    /// A node's successful gatherings since it spawned.
    pub node_uses: ahash::AHashMap<Guid, u32>,
    /// Those whose skill an object's lock has raised since it spawned:
    /// once each (Thistle Tea's `skilled_players`).
    pub skilled: ahash::AHashMap<Guid, AHashSet<Guid>>,
    /// Creatures whose pockets are picked this life.
    pub picked: AHashSet<Guid>,
    pub bobbers: Vec<crate::game::map::professions::Bobber>,
    /// Where each player's line will land, chosen as the cast began.
    pub fishing_spots: ahash::AHashMap<Guid, wow_world_messages::vanilla::Vector3d>,
    /// How many catches a school of fish holds this spawn, rolled at its
    /// first.
    pub hole_uses: ahash::AHashMap<Guid, u32>,
    /// Loot on an item: a disenchanted one, an opened container.
    pub item_loot: ahash::AHashMap<Guid, Loot>,
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

/// A reference rolled into, if its condition holds for the looter.
fn reference(
    data: &GameData,
    e: &LootEntry,
    wanted: &[u32],
    rng: &mut impl rand::Rng,
    out: &mut Vec<LootSlot>,
    depth: u32,
    allowed: &dyn Fn(u32) -> bool,
) {
    if e.condition != 0 && !allowed(e.condition) {
        return;
    }
    if let Some(r) = data.loot.reference.get(&(e.min.unsigned_abs())) {
        for _ in 0..e.max.max(1) {
            roll_for(data, r, wanted, rng, out, depth + 1, allowed);
        }
    }
}

fn slot_for(data: &GameData, e: &LootEntry, rng: &mut impl rand::Rng) -> LootSlot {
    LootSlot {
        item: e.item,
        count: count(rng, e),
        random_property: crate::game::map::enchant::roll_random_property(data, e.item, rng),
        quest: is_quest_item(e),
        quality: data.item(e.item).map_or(0, |t| t.quality),
        condition: e.condition,
        ..Default::default()
    }
}

/// Roll a table, vmangos's way: each ungrouped entry on its own chance;
/// one entry per group (the explicitly chanced in order, else one of the
/// equally chanced); references rolled into as many times as they say.
/// Quest items drop only if they are `wanted`. A reference with a
/// condition isn't rolled with no looter to judge it, as in Thistle Tea.
pub fn roll(
    data: &GameData,
    entries: &[LootEntry],
    wanted: &[u32],
    rng: &mut impl rand::Rng,
    out: &mut Vec<LootSlot>,
    depth: u32,
) {
    roll_for(data, entries, wanted, rng, out, depth, &|_| false);
}

/// [`roll`], a reference with a condition rolled if `allowed` says the
/// condition holds.
pub fn roll_for(
    data: &GameData,
    entries: &[LootEntry],
    wanted: &[u32],
    rng: &mut impl rand::Rng,
    out: &mut Vec<LootSlot>,
    depth: u32,
    allowed: &dyn Fn(u32) -> bool,
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
            reference(data, e, wanted, rng, out, depth, allowed);
        } else {
            out.push(slot_for(data, e, rng));
        }
    }
    for g in groups {
        // A quest drop is picked by its chance as any other; one no one
        // wants then drops for no one (Thistle Tea's `roll_group`).
        let members: Vec<&LootEntry> = entries.iter().filter(|e| e.group == g).collect();
        let mut r = rng.random_range(0.0f32..100.0);
        let mut picked = None;
        for e in members.iter().filter(|e| e.chance != 0.0) {
            r -= e.chance.abs();
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
        let Some(e) = picked.filter(|e| !skip(e)) else {
            continue;
        };
        if e.min < 0 {
            reference(data, &e, wanted, rng, out, depth, allowed);
        } else {
            out.push(slot_for(data, &e, rng));
        }
    }
    if depth == 0 {
        out.truncate(MAX_ITEMS);
    }
}

/// Personal loot rolled from `table` for `owner`: pickpocketing,
/// skinning, fishing, disenchanting.
pub fn personal(
    data: &GameData,
    table: &[LootEntry],
    owner: Guid,
    wanted: &[u32],
    kind: LootKind,
) -> Loot {
    let mut items = Vec::new();
    roll(data, table, wanted, &mut rand::rng(), &mut items, 0);
    Loot {
        items,
        owner: Some(owner),
        kind,
        ..Default::default()
    }
}

/// The group members of `tapper` near `at` on this map, the tapper first;
/// just the tapper alone.
pub fn tap_group(map: &Map, tapper: Guid, at: wow_world_messages::vanilla::Vector3d) -> Vec<Guid> {
    let members: Vec<Guid> = {
        let groups = map.realm.social.groups.lock();
        match groups.group_of(tapper) {
            Some(g) => g.members.iter().map(|m| m.guid).collect(),
            None => Vec::new(),
        }
    };
    let mut near: Vec<Guid> = vec![tapper];
    near.extend(members.into_iter().filter(|g| {
        *g != tapper
            && map
                .players
                .get(g)
                .is_some_and(|p| dist(p.position(), at) <= GROUP_REWARD_YARDS)
    }));
    near
}

/// A creature died: its loot is rolled for its tapper and their group.
pub fn creature_died(map: &mut Map, creature: Guid, tapper: Guid) {
    let data = map.realm.data.clone();
    let Some((entry, at)) = map.creature(creature).map(|c| (c.entry, c.position)) else {
        return;
    };
    let Some(t) = data.creature_template(entry) else {
        return;
    };
    let tapped = guid::is_player(tapper);
    let group = if tapped {
        tap_group(map, tapper, at)
    } else {
        Vec::new()
    };
    let mut wanted = Vec::new();
    for g in &group {
        if let Some(p) = map.players.get(g) {
            wanted.extend(crate::game::map::quests::wanted_items(p, &data));
        }
    }
    let mut rng = rand::rng();
    let mut items = Vec::new();
    // Killed mostly by NPCs, it carries nothing (Thistle Tea's
    // `DamageOrigin.loot_allowed?`); nor does a creature's guardian (its
    // `loot: nil`).
    let guardian = crate::game::map::event_ai::scripted(map, creature).is_some_and(|s| s.guardian);
    let allowed = !guardian && crate::game::map::creature_combat::loot_allowed(map, creature);
    if allowed
        && t.loot_id != 0
        && let Some(table) = data.loot.creature.get(&t.loot_id)
    {
        roll(&data, table, &wanted, &mut rng, &mut items, 0);
    }
    let gold = if !allowed {
        0
    } else if t.gold_max > t.gold_min {
        rng.random_range(t.gold_min..=t.gold_max)
    } else {
        t.gold_min
    };
    let mut loot = Loot {
        items,
        gold,
        owner: tapped.then_some(tapper),
        ..Default::default()
    };
    if group.len() > 1 {
        setup_group(map, &mut loot, tapper, group);
    }
    let lootable = !loot.is_empty() || loot.items.iter().any(|s| s.quest);
    let rolls: Vec<usize> = loot.rolls.iter().map(|r| r.slot).collect();
    map.loot_state.picked.remove(&creature);
    let Some(c) = map.creature_mut(creature) else {
        return;
    };
    c.loot = Some(loot);
    c.fields
        .set_flag(UNIT_DYNAMIC_FLAGS, DYNFLAG_LOOTABLE, lootable);
    // Nothing on it: it can be skinned at once.
    if !lootable && t.skinning_loot_id != 0 {
        c.fields.set_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::professions::UNIT_FLAG_SKINNABLE,
            true,
        );
    }
    if !rolls.is_empty() {
        map.loot_state.rolling.push(creature);
        for slot in rolls {
            start_roll(map, creature, slot);
        }
    }
}

/// The group's loot method on a kill.
fn setup_group(map: &mut Map, loot: &mut Loot, tapper: Guid, near: Vec<Guid>) {
    let data = map.realm.data.clone();
    let (method, master, threshold, assigned) = {
        let mut groups = map.realm.social.groups.lock();
        let Some(g) = groups.group_of_mut(tapper) else {
            return;
        };
        let assigned = (g.loot_method == GroupLootSetting::RoundRobin).then(|| {
            let order: Vec<Guid> = g
                .members
                .iter()
                .map(|m| m.guid)
                .filter(|m| near.contains(m))
                .collect();
            let next = order
                .iter()
                .position(|m| *m == g.round_robin)
                .map(|i| order[(i + 1) % order.len()])
                .unwrap_or(order[0]);
            g.round_robin = next;
            next
        });
        (g.loot_method, g.looter, quality(g.threshold), assigned)
    };
    loot.owner = None;
    loot.allowed = near.clone();
    loot.method = Some(method);
    loot.assigned = assigned;
    let rollable: Vec<usize> = loot
        .items
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.quest && s.quality >= threshold)
        .map(|(i, _)| i)
        .collect();
    match method {
        GroupLootSetting::MasterLoot if near.contains(&master) => {
            loot.master = Some(master);
            for i in rollable {
                loot.items[i].blocked = true;
            }
        }
        GroupLootSetting::GroupLoot | GroupLootSetting::NeedBeforeGreed => {
            let now = map.now;
            for i in rollable {
                let item = data.item(loot.items[i].item);
                let eligible: Vec<Guid> = near
                    .iter()
                    .copied()
                    .filter(|g| {
                        method != GroupLootSetting::NeedBeforeGreed
                            || match (map.players.get(g), item) {
                                (Some(p), Some(t)) => usable(p, &data, t),
                                _ => false,
                            }
                    })
                    .collect();
                if eligible.is_empty() {
                    continue;
                }
                loot.items[i].blocked = true;
                loot.rolls.push(Roll {
                    slot: i,
                    eligible,
                    votes: Vec::new(),
                    ends: now + ROLL_FOR,
                });
            }
        }
        _ => {}
    }
}

fn quality(q: ItemQuality) -> u32 {
    u32::from(q.as_int())
}

/// Whether `p` could use an item: their class, race and proficiency, as
/// they have learned it (Thistle Tea's `ItemEligibility`).
pub fn usable(p: &Player, data: &GameData, t: &ItemTemplate) -> bool {
    let class_ok = t.allowable_class <= 0 || t.allowable_class & (1 << (p.row.class - 1)) != 0;
    let race_ok = t.allowable_race <= 0 || t.allowable_race & (1 << (p.row.race - 1)) != 0;
    class_ok && race_ok && inventory::proficient(p, &inventory::Proficiency::of(p, data), t).is_ok()
}

fn send_to(map: &Map, players: &[Guid], frame: &Arc<[u8]>) {
    for g in players {
        if let Some(p) = map.players.get(g) {
            p.outbound.send_frame(frame.clone());
        }
    }
}

fn roll_of(map: &Map, object: Guid, slot: usize) -> Option<(Roll, LootSlot)> {
    let loot = loot_ref(map, object)?;
    let r = loot.rolls.iter().find(|r| r.slot == slot)?.clone();
    Some((r, loot.items.get(slot)?.clone()))
}

/// `SMSG_LOOT_START_ROLL` to those who may roll.
fn start_roll(map: &mut Map, object: Guid, slot: usize) {
    let Some((r, s)) = roll_of(map, object, slot) else {
        return;
    };
    if r.eligible.len() == 1 {
        // Only one may have it: theirs, as a need.
        award(map, object, slot, r.eligible[0], 100, VOTE_NEED, &[]);
        return;
    }
    let mut b = Body::default();
    b.u64(object.guid())
        .u32(slot as u32)
        .u32(s.item)
        .u32(0)
        .u32(s.random_property)
        .u32(ROLL_FOR.as_millis() as u32);
    send_to(map, &r.eligible, &b.frame(SMSG_LOOT_START_ROLL));
}

/// `SMSG_LOOT_ROLL`: a member's vote, or their number.
fn roll_frame(
    object: Guid,
    slot: usize,
    s: &LootSlot,
    who: Guid,
    number: u8,
    kind: u8,
) -> Arc<[u8]> {
    let mut b = Body::default();
    b.u64(object.guid())
        .u32(slot as u32)
        .u64(who.guid())
        .u32(s.item)
        .u32(0)
        .u32(s.random_property)
        .u8(number)
        .u8(kind);
    b.frame(SMSG_LOOT_ROLL)
}

/// `CMSG_LOOT_ROLL`: a vote on a roll.
pub fn vote(map: &mut Map, player: Guid, object: Guid, slot: u32, vote: u8) {
    let slot = slot as usize;
    let complete = {
        let Some(loot) = loot_mut(map, object) else {
            return;
        };
        let Some(r) = loot.rolls.iter_mut().find(|r| r.slot == slot) else {
            return;
        };
        if !r.eligible.contains(&player) || r.votes.iter().any(|(g, _)| *g == player) {
            return;
        }
        let vote = if vote > VOTE_GREED { VOTE_PASS } else { vote };
        r.votes.push((player, vote));
        r.votes.len() >= r.eligible.len()
    };
    let Some((r, s)) = roll_of(map, object, slot) else {
        return;
    };
    // The echo every roller sees: need, greed or a pass.
    let (number, kind) = match vote {
        VOTE_NEED => (0, 0),
        VOTE_GREED => (128, 2),
        _ => (128, 128),
    };
    send_to(
        map,
        &r.eligible,
        &roll_frame(object, slot, &s, player, number, kind),
    );
    if complete {
        resolve(map, object, slot);
    }
}

/// A roll ends: need over greed, the highest number winning.
fn resolve(map: &mut Map, object: Guid, slot: usize) {
    let Some((r, s)) = roll_of(map, object, slot) else {
        return;
    };
    if let Some(l) = loot_mut(map, object) {
        l.rolls.retain(|x| x.slot != slot);
    }
    let needs: Vec<Guid> = r
        .votes
        .iter()
        .filter(|(_, v)| *v == VOTE_NEED)
        .map(|(g, _)| *g)
        .collect();
    let greeds: Vec<Guid> = r
        .votes
        .iter()
        .filter(|(_, v)| *v == VOTE_GREED)
        .map(|(g, _)| *g)
        .collect();
    let (contenders, kind) = if !needs.is_empty() {
        (needs, VOTE_NEED)
    } else {
        (greeds, VOTE_GREED)
    };
    if contenders.is_empty() {
        let mut b = Body::default();
        b.u64(object.guid())
            .u32(slot as u32)
            .u32(s.item)
            .u32(s.random_property)
            .u32(0);
        send_to(map, &r.eligible, &b.frame(SMSG_LOOT_ALL_PASSED));
        if let Some(l) = loot_mut(map, object)
            && let Some(x) = l.items.get_mut(slot)
        {
            x.blocked = false;
        }
        finish_rolls(map, object);
        return;
    }
    // In guid order, the first of the highest wins (Thistle Tea's
    // `LootRoll.pick_winner`).
    let mut contenders = contenders;
    contenders.sort_unstable_by_key(|g| g.guid());
    let mut rng = rand::rng();
    let rolled: Vec<(Guid, u8)> = contenders
        .iter()
        .map(|g| (*g, rng.random_range(1..=100u8)))
        .collect();
    let (winner, number) = rolled
        .iter()
        .copied()
        .fold(None, |best: Option<(Guid, u8)>, x| match best {
            Some(b) if b.1 >= x.1 => Some(b),
            _ => Some(x),
        })
        .expect("there are contenders");
    award(map, object, slot, winner, number, kind, &rolled);
    let _ = r;
}

/// The roll's winner gets the item, if it fits; if not, it's free again.
fn award(
    map: &mut Map,
    object: Guid,
    slot: usize,
    winner: Guid,
    number: u8,
    kind: u8,
    rolled: &[(Guid, u8)],
) {
    let Some(loot) = loot_ref(map, object) else {
        return;
    };
    let Some(s) = loot.items.get(slot).cloned() else {
        return;
    };
    let eligible: Vec<Guid> = loot
        .rolls
        .iter()
        .find(|r| r.slot == slot)
        .map(|r| r.eligible.clone())
        .unwrap_or_else(|| loot.allowed.clone());
    for (g, n) in rolled {
        send_to(map, &eligible, &roll_frame(object, slot, &s, *g, *n, kind));
    }
    let mut b = Body::default();
    b.u64(object.guid())
        .u32(slot as u32)
        .u32(s.item)
        .u32(0)
        .u32(s.random_property)
        .u64(winner.guid())
        .u8(number)
        .u8(kind);
    send_to(map, &eligible, &b.frame(SMSG_LOOT_ROLL_WON));
    if let Some(l) = loot_mut(map, object) {
        l.rolls.retain(|r| r.slot != slot);
    }
    // A winner gone too far has nothing; it's free for the taking.
    let near = match (
        map.players.get(&winner).map(|p| p.position()),
        position_of(map, object),
    ) {
        (Some(a), Some(b)) => dist(a, b) <= GROUP_REWARD_YARDS,
        _ => false,
    };
    if near {
        give(map, object, slot, winner);
    } else if let Some(l) = loot_mut(map, object)
        && let Some(x) = l.items.get_mut(slot)
    {
        x.blocked = false;
    }
    finish_rolls(map, object);
}

/// Slot `slot` of `object`'s loot goes into `to`'s bags; if it doesn't
/// fit, it's free for the taking.
fn give(map: &mut Map, object: Guid, slot: usize, to: Guid) -> bool {
    let Some(s) = loot_ref(map, object).and_then(|l| l.items.get(slot).cloned()) else {
        return false;
    };
    match inventory::add_item_with_property(map, to, s.item, s.count, true, s.random_property) {
        Ok(()) => {
            let looters: Vec<Guid> = match loot_mut(map, object) {
                Some(l) => {
                    if let Some(x) = l.items.get_mut(slot) {
                        x.taken = true;
                        x.blocked = false;
                    }
                    l.looters.iter().copied().collect()
                }
                None => Vec::new(),
            };
            for g in looters {
                if let Some(p) = map.players.get(&g) {
                    p.outbound.send(&SMSG_LOOT_REMOVED { slot: slot as u8 });
                }
            }
            true
        }
        Err(f) => {
            if let Some(p) = map.players.get(&to) {
                inventory::send_fail(p, f, Guid::zero());
            }
            if let Some(l) = loot_mut(map, object)
                && let Some(x) = l.items.get_mut(slot)
            {
                x.blocked = false;
            }
            false
        }
    }
}

fn finish_rolls(map: &mut Map, object: Guid) {
    let done = loot_ref(map, object).is_none_or(|l| l.rolls.is_empty());
    if done {
        map.loot_state.rolling.retain(|g| *g != object);
    }
    refresh_lootable(map, object);
}

fn refresh_lootable(map: &mut Map, object: Guid) {
    let lootable = loot_ref(map, object)
        .is_some_and(|l| !l.is_empty() || l.items.iter().any(|s| s.quest && !s.taken));
    if let Some(c) = map.creature_mut(object) {
        c.fields
            .set_flag(UNIT_DYNAMIC_FLAGS, DYNFLAG_LOOTABLE, lootable);
    }
}

/// Rolls whose time is up end; gathered nodes come back.
pub fn update(map: &mut Map) {
    let now = map.now;
    let rolling = map.loot_state.rolling.clone();
    for object in rolling {
        let due: Vec<usize> = loot_ref(map, object)
            .map(|l| {
                l.rolls
                    .iter()
                    .filter(|r| r.ends <= now)
                    .map(|r| r.slot)
                    .collect()
            })
            .unwrap_or_default();
        if loot_ref(map, object).is_none() {
            map.loot_state.rolling.retain(|g| *g != object);
            continue;
        }
        for slot in due {
            resolve(map, object, slot);
        }
    }
    crate::game::map::professions::respawn_nodes(map);
}

/// The conditions of `loot`'s slots that `viewer` meets, from the
/// corpse or object `source`.
pub fn conditions_met(map: &Map, loot: &Loot, source: Guid, viewer: Guid) -> Vec<u32> {
    let mut ids: Vec<u32> = loot
        .items
        .iter()
        .map(|s| s.condition)
        .filter(|c| *c != 0)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids.retain(|c| crate::game::map::conditions::met(map, *c, Some(source), Some(viewer)));
    ids
}

/// `SMSG_LOOT_RESPONSE` in 1.12's layout for `viewer`: each item they may
/// see with its count, look and random property.
pub fn response(
    data: &GameData,
    g: Guid,
    loot: &Loot,
    viewer: Guid,
    wanted: &[u32],
    met: &[u32],
) -> Arc<[u8]> {
    let mut b = Body::default();
    b.u64(g.guid()).u8(loot.kind.wire()).u32(loot.gold);
    let shown: Vec<(usize, &LootSlot)> = loot
        .items
        .iter()
        .enumerate()
        .filter(|(_, s)| loot.visible(s, viewer, wanted, met))
        .collect();
    b.u8(shown.len() as u8);
    for (i, s) in shown {
        let display = data.item(s.item).map(|t| t.display_id).unwrap_or(0);
        let kind = if s.blocked { SLOT_MASTER } else { SLOT_ALLOW };
        b.u8(i as u8)
            .u32(s.item)
            .u32(s.count)
            .u32(display)
            .u32(0)
            .u32(s.random_property)
            .u8(kind);
    }
    b.frame(SMSG_LOOT_RESPONSE)
}

/// The loot on `g` (a creature, game object or item).
pub fn loot_ref(map: &Map, g: Guid) -> Option<&Loot> {
    if let Some(l) = map.loot_state.item_loot.get(&g) {
        return Some(l);
    }
    if guid::is_unit(g) {
        map.creature(g).and_then(|c| c.loot.as_ref())
    } else if guid::is_game_object(g) {
        map.gameobject(g).and_then(|o| o.loot.as_ref())
    } else {
        None
    }
}

/// The loot on `g` (a creature, game object or item), mutably.
pub fn loot_mut(map: &mut Map, g: Guid) -> Option<&mut Loot> {
    if map.loot_state.item_loot.contains_key(&g) {
        return map.loot_state.item_loot.get_mut(&g);
    }
    if guid::is_unit(g) {
        map.creature_mut(g).and_then(|c| c.loot.as_mut())
    } else if guid::is_game_object(g) {
        map.gameobject_mut(g).and_then(|o| o.loot.as_mut())
    } else {
        None
    }
}

/// Loot on an item opens at once: a disenchanted item's dust, an opened
/// container's contents.
pub fn show_item_loot(map: &mut Map, player: Guid, item: Guid, loot: Loot) {
    map.loot_state.item_loot.insert(item, loot);
    show(map, player, item);
}

/// `CMSG_LOOT`: open the loot window on a corpse or object.
pub fn open(map: &mut Map, player: Guid, target: Guid) {
    let Some(ppos) = map.players.get(&player).map(|p| p.position()) else {
        return;
    };
    let (tpos, reach) = if guid::is_unit(target) {
        match map.creature(target) {
            Some(c)
                if !c.alive()
                    || c.loot
                        .as_ref()
                        .is_some_and(|l| l.kind == LootKind::Pickpocket) =>
            {
                (c.position, LOOT_YARDS)
            }
            _ => return,
        }
    } else if let Some(o) = map.gameobject(target) {
        // A locked object (a chest, a vein, a herb) is looked in only by
        // one who opened it, as Thistle Tea's `Chest.authorized?` has it.
        // The window shuts on one who didn't (Thistle Tea's).
        let lock = crate::game::map::professions::object_lock(&map.realm.data, o.entry);
        if lock != 0
            && !o
                .loot
                .as_ref()
                .is_some_and(|l| l.opened_by.contains(&player))
        {
            if let Some(p) = map.players.get(&player) {
                p.outbound.send(&SMSG_LOOT_RELEASE_RESPONSE {
                    guid: target,
                    unknown1: 1,
                });
            }
            return;
        }
        // A bobber is looted from the shore.
        let reach = if o.kind == crate::game::map::professions::GO_FISHING_NODE {
            30.0
        } else {
            LOOT_YARDS
        };
        (o.position, reach)
    } else {
        return;
    };
    if dist(ppos, tpos) > reach {
        return;
    }
    show(map, player, target);
}

/// Open `target`'s loot window for `player`, distance aside.
pub fn show(map: &mut Map, player: Guid, target: Guid) {
    let data = map.realm.data.clone();
    let wanted = map
        .players
        .get(&player)
        .map(|p| crate::game::map::quests::wanted_items(p, &data))
        .unwrap_or_default();
    let met = loot_ref(map, target)
        .map(|l| conditions_met(map, l, target, player))
        .unwrap_or_default();
    let Some(loot) = loot_mut(map, target) else {
        return;
    };
    if !loot.may_loot(player) {
        // A lock not opened by them: the window shuts (Thistle Tea's).
        if loot.kind == LootKind::Opened
            && let Some(p) = map.players.get(&player)
        {
            p.outbound.send(&SMSG_LOOT_RELEASE_RESPONSE {
                guid: target,
                unknown1: 1,
            });
        }
        return;
    }
    loot.looters.insert(player);
    let master_list = (loot.master == Some(player)).then(|| loot.allowed.clone());
    let frame = response(&data, target, loot, player, &wanted, &met);
    if let Some(p) = map.players.get_mut(&player) {
        p.looting = Some(target);
        p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_LOOTING, true);
        p.outbound.send_frame(frame);
    }
    if let Some(list) = master_list {
        let near: Vec<Guid> = list
            .into_iter()
            .filter(|g| map.players.contains_key(g))
            .collect();
        let mut b = Body::default();
        b.u8(near.len() as u8);
        for g in &near {
            b.u64(g.guid());
        }
        if let Some(p) = map.players.get(&player) {
            p.outbound.send_frame(b.frame(SMSG_LOOT_MASTER_LIST));
        }
    }
}

/// `CMSG_AUTOSTORE_LOOT_ITEM`: take an item from the open window.
pub fn take_item(map: &mut Map, player: Guid, slot: u8) {
    let data = map.realm.data.clone();
    let Some(target) = map.players.get(&player).and_then(|p| p.looting) else {
        return;
    };
    let wanted = map
        .players
        .get(&player)
        .map(|p| crate::game::map::quests::wanted_items(p, &data))
        .unwrap_or_default();
    let met = loot_ref(map, target)
        .map(|l| conditions_met(map, l, target, player))
        .unwrap_or_default();
    let Some((item, count, property, quest)) = loot_ref(map, target).and_then(|l| {
        let s = l.items.get(usize::from(slot))?;
        (l.may_loot(player) && l.visible(s, player, &wanted, &met) && !s.blocked).then_some((
            s.item,
            s.count,
            s.random_property,
            s.quest,
        ))
    }) else {
        return;
    };
    match inventory::add_item_with_property(map, player, item, count, true, property) {
        Ok(()) => {
            let looters: Vec<Guid> = match loot_mut(map, target) {
                Some(l) => {
                    if let Some(s) = l.items.get_mut(usize::from(slot)) {
                        if quest {
                            s.taken_by.push(player);
                        } else {
                            s.taken = true;
                        }
                    }
                    if quest {
                        vec![player]
                    } else {
                        l.looters.iter().copied().collect()
                    }
                }
                None => Vec::new(),
            };
            for g in looters {
                if let Some(p) = map.players.get(&g) {
                    p.outbound.send(&SMSG_LOOT_REMOVED { slot });
                }
            }
            crate::game::map::instance_script::chest_changed(map, target);
        }
        Err(f) => {
            if let Some(p) = map.players.get(&player) {
                inventory::send_fail(p, f, Guid::zero());
            }
        }
    }
}

/// `CMSG_LOOT_MONEY`: the gold, split among the group near the corpse.
pub fn take_money(map: &mut Map, player: Guid) {
    let Some(target) = map.players.get(&player).and_then(|p| p.looting) else {
        return;
    };
    let Some((gold, looters, kind)) = loot_mut(map, target).map(|l| {
        let gold = if l.may_loot(player) {
            std::mem::take(&mut l.gold)
        } else {
            0
        };
        (gold, l.looters.iter().copied().collect::<Vec<_>>(), l.kind)
    }) else {
        return;
    };
    if gold == 0 {
        return;
    }
    // A corpse's or an object's money is the group's near the looter, in
    // even shares, the remainder lost; a pocket's or a container's is the
    // looter's (Thistle Tea's `Looting.split_gold`).
    let at = map.players.get(&player).map(|p| p.position());
    let shared =
        kind != LootKind::Pickpocket && (guid::is_unit(target) || guid::is_game_object(target));
    let sharers = match at {
        Some(at) if shared => tap_group(map, player, at),
        _ => vec![player],
    };
    let share = gold / sharers.len().max(1) as u32;
    for g in &sharers {
        if let Some(p) = map.players.get_mut(g) {
            let money = p.money().saturating_add(share);
            p.set_money(money);
            let mut b = Body::default();
            b.u32(share);
            p.outbound.send_frame(b.frame(SMSG_LOOT_MONEY_NOTIFY));
        }
    }
    for g in looters {
        if let Some(p) = map.players.get(&g) {
            p.outbound.send(&SMSG_LOOT_CLEAR_MONEY {});
        }
    }
    crate::game::map::instance_script::chest_changed(map, target);
}

/// `CMSG_LOOT_MASTER_GIVE`: the master looter gives a kept item to a
/// member near.
pub fn master_give(map: &mut Map, giver: Guid, object: Guid, slot: u8, to: Guid) {
    let ok = loot_ref(map, object).is_some_and(|l| {
        l.master == Some(giver)
            && l.allowed.contains(&to)
            && l.items
                .get(usize::from(slot))
                .is_some_and(|s| s.blocked && !s.taken)
    }) && map
        .players
        .get(&giver)
        .is_some_and(|p| p.looting == Some(object));
    let near = match (map.players.get(&to), position_of(map, object)) {
        (Some(p), Some(at)) => dist(p.position(), at) <= GROUP_REWARD_YARDS,
        _ => false,
    };
    if ok && near {
        give(map, object, usize::from(slot), to);
    }
}

fn position_of(map: &Map, g: Guid) -> Option<wow_world_messages::vanilla::Vector3d> {
    if guid::is_unit(g) {
        map.creature(g).map(|c| c.position)
    } else {
        map.gameobject(g).map(|o| o.position)
    }
}

/// `CMSG_LOOT_RELEASE`: the window closes. An emptied corpse is no longer
/// lootable; an emptied node is gathered.
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
    let (empty, kind) = match loot_mut(map, target) {
        Some(l) => {
            l.looters.remove(&player);
            // Closed, a lock must be opened again.
            l.opened_by.remove(&player);
            (l.is_empty(), l.kind)
        }
        None => return,
    };
    if map.loot_state.item_loot.contains_key(&target) {
        // What's left on an item is lost with the window.
        map.loot_state.item_loot.remove(&target);
        return;
    }
    if guid::is_unit(target) {
        if kind == LootKind::Pickpocket {
            // The pockets are picked; the creature keeps no window.
            if let Some(c) = map.creature_mut(target) {
                c.loot = None;
            }
            return;
        }
        refresh_lootable(map, target);
        if empty && kind == LootKind::Corpse {
            crate::game::map::professions::corpse_looted(map, target);
        }
    } else {
        // A bobber goes with its window, caught or left (Thistle Tea's).
        if empty
            || map
                .gameobject(target)
                .is_some_and(|o| o.kind == crate::game::map::professions::GO_FISHING_NODE)
        {
            crate::game::map::professions::object_looted(map, player, target);
        }
        crate::game::map::instance_script::chest_changed(map, target);
    }
}

/// A chest or node is opened: its loot, rolled now if it hasn't been.
pub fn open_object(map: &mut Map, player: Guid, object: Guid) {
    let data = map.realm.data.clone();
    let wanted = map
        .players
        .get(&player)
        .map(|p| crate::game::map::quests::wanted_items(p, &data))
        .unwrap_or_default();
    let Some(o) = map.gameobject(object) else {
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
            let allowed =
                |c: u32| crate::game::map::conditions::met(map, c, Some(object), Some(player));
            roll_for(
                &data,
                table,
                &wanted,
                &mut rand::rng(),
                &mut items,
                0,
                &allowed,
            );
        }
        let gold = if t.gold_max > t.gold_min {
            rand::rng().random_range(t.gold_min..=t.gold_max)
        } else {
            t.gold_min
        };
        // A chest behind a lock shows as one opened (Thistle Tea's
        // `Gathering`, mangos's chests).
        let kind = if t.kind == crate::game::map::professions::GO_CHEST && t.data[0] != 0 {
            LootKind::Opened
        } else {
            LootKind::Corpse
        };
        if let Some(o) = map.gameobject_mut(object) {
            o.loot = Some(Loot {
                items,
                gold,
                kind,
                ..Default::default()
            });
        }
        crate::game::map::instance_script::chest_changed(map, object);
    }
    // Opened, by lock or by hand: theirs to look at.
    if let Some(l) = map.gameobject_mut(object).and_then(|o| o.loot.as_mut()) {
        l.opened_by.insert(player);
    }
    open(map, player, object);
}

/// `CMSG_LOOT_ROLL` and `CMSG_LOOT_MASTER_GIVE`.
pub fn handle(map: &mut Map, g: Guid, msg: ClientOpcodeMessage) -> Option<ClientOpcodeMessage> {
    match msg {
        ClientOpcodeMessage::CMSG_LOOT_ROLL(m) => {
            vote(map, g, m.item, m.item_slot, m.vote.as_int());
            None
        }
        ClientOpcodeMessage::CMSG_LOOT_MASTER_GIVE(m) => {
            master_give(map, g, m.loot, m.slot_id, m.player);
            None
        }
        m => Some(m),
    }
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
