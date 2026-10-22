//! Loot: rolled from vmangos's tables when a creature dies or a chest is
//! opened, held on the corpse or object, and taken through the loot
//! window.
//!
//! A group's kill follows its loot method, as mangos-classic's `Loot`
//! does: under every method but free for all the group's next member near
//! is the current looter, the items under the threshold theirs alone till
//! they close the window, then anyone's it fell to (round robin's items at
//! the threshold too); master loot keeps the items at the threshold for
//! the master looter to give (group loot with no master near), group loot
//! and need before greed roll for them (60 seconds, need over greed).
//! Quest items, and free-for-all ones, drop for each member who may have
//! them; a drop no one it may fall to may have isn't rolled; gold is
//! split among those it fell to.

use crate::config::RatesConfig;
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
use crate::game::map::combat::dist;
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
/// How long a looted corpse lies at most, and one looked in at least
/// (mangos's `MINIMUM_LOOTING_TIME`).
pub const LOOTING_TIME: Duration = Duration::from_secs(120);
/// References nest this deep at most (Thistle Tea's `@max_reference_depth`).
const MAX_DEPTH: u32 = 3;
/// A loot window holds this many items at most.
pub const MAX_ITEMS: usize = 16;
/// `UNIT_FIELD_FLAGS`: the player has a loot window open.
const UNIT_FLAG_LOOTING: u32 = 0x400;

/// Loot slot types, as the window shows them (mangos's `LootSlotType`):
/// to take, to look at (rolled for, or kept for the master looter), the
/// master looter's to give.
const SLOT_ALLOW: u8 = 0;
const SLOT_VIEW: u8 = 1;
const SLOT_MASTER: u8 = 2;
/// `ITEM_FLAG_MULTI_DROP`: the item drops for each who may loot it
/// (mangos's `LootItem::freeForAll`).
pub const ITEM_FLAG_MULTI_DROP: u32 = 0x800;
/// `ITEM_EXTRA_IGNORE_QUEST_STATUS`: a quest-starting item that drops
/// whatever its quest's status.
const ITEM_EXTRA_IGNORE_QUEST_STATUS: u32 = 0x2;

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
    /// A roll's winner who couldn't carry it: theirs alone to take
    /// (Thistle Tea's need ownership).
    pub winner: Option<Guid>,
    pub quality: u32,
    /// A condition the viewer must meet to see it.
    pub condition: u32,
    /// Free for all (`ITEM_FLAG_MULTI_DROP`): each it fell to takes their
    /// own, as a quest drop (mangos's `freeForAll`).
    pub ffa: bool,
    /// The current looter closed the window on it, or a roll on it was
    /// all passed: anyone's it fell to (mangos's `isReleased`).
    pub released: bool,
    /// Not the current looter's to see: anyone's it fell to at once
    /// (mangos's `currentLooterPass`).
    pub looter_pass: bool,
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
    /// The group's current looter: the items under the threshold are
    /// theirs alone till they close the window (mangos's
    /// `m_currentLooterGuid`, for every method but free for all).
    pub assigned: Option<Guid>,
    /// Looted for all: nothing in it anyone it fell to may take, so the
    /// corpse may be skinned (mangos's `CREATURE_LOOT_STATUS_LOOTED`).
    pub done: bool,
    pub rolls: Vec<Roll>,
    pub kind: LootKind,
    /// Who has the window open.
    pub looters: AHashSet<Guid>,
    /// Behind a lock: those who opened it, till they close the window
    /// (Thistle Tea's `opened_by`).
    pub opened_by: AHashSet<Guid>,
    /// Those it fell to who would find nothing in it of theirs: its quest
    /// drops not for their quests, its conditions not met. It doesn't
    /// sparkle for them (mangos's per-viewer `Loot::CanLoot`, Thistle
    /// Tea's `visible?`), as their quests stood when it last changed.
    pub quiet: Vec<Guid>,
}

impl Loot {
    pub fn is_empty(&self) -> bool {
        self.done
            || (self.gold == 0
                && self.items.iter().all(|s| s.taken || s.quest || s.ffa)
                && self.rolls.is_empty())
    }

    /// Looted for all, so its corpse may be skinned (mangos's
    /// `IsLootedForAll`): a corpse's that fell to someone once
    /// [`Loot::done`], with every quest and free-for-all drop of theirs
    /// taken; any other once [`Loot::is_empty`].
    pub fn looted_for_all(&self) -> bool {
        if self.kind == LootKind::Corpse && (self.owner.is_some() || !self.allowed.is_empty()) {
            self.done
        } else {
            self.is_empty()
        }
    }

    /// Whether `g` has a right to it: its owner, one of the group it fell
    /// to (mangos's `m_ownerSet`), or one who opened its lock. What in it
    /// is theirs to take is [`Loot::slot_type`]'s.
    pub fn may_loot(&self, g: Guid) -> bool {
        if self.kind == LootKind::Opened && !self.opened_by.contains(&g) {
            return false;
        }
        if !self.allowed.is_empty() {
            return self.allowed.contains(&g);
        }
        self.owner.is_none_or(|o| o == g)
    }

    /// Whether `g` would find anything in it: its gold, or an item they
    /// see (mangos's `Loot::CanLoot`, by `GetLootStatusFor`).
    pub fn has_for(&self, g: Guid, sight: &Sight) -> bool {
        self.gold > 0 || (0..self.items.len()).any(|i| self.slot_type(i, g, sight).is_some())
    }

    /// How `g` sees slot `i`, if at all (mangos's
    /// `GetSlotTypeForSharedLoot`): to take, to look at, or to give.
    fn slot_type(&self, i: usize, g: Guid, sight: &Sight) -> Option<u8> {
        let s = self.items.get(i)?;
        let master_gives = self.master == Some(g) && s.blocked && self.rolls.is_empty();
        if s.taken
            || (s.condition != 0 && !sight.met.contains(&s.condition) && !master_gives)
            || s.winner.is_some_and(|w| w != g)
            || (sight.started.contains(&s.item) && self.master != Some(g))
        {
            return None;
        }
        if s.quest {
            return (sight.wanted.contains(&s.item) && !s.taken_by.contains(&g))
                .then_some(SLOT_ALLOW);
        }
        if s.ffa {
            return (!s.taken_by.contains(&g)).then_some(SLOT_ALLOW);
        }
        if s.winner == Some(g) {
            return Some(SLOT_ALLOW);
        }
        if s.blocked {
            return if self.rolls.iter().any(|r| r.slot == i) {
                Some(SLOT_VIEW)
            } else if master_gives {
                Some(SLOT_MASTER)
            } else {
                None
            };
        }
        match self.method {
            None | Some(GroupLootSetting::FreeForAll) => Some(SLOT_ALLOW),
            Some(_) => (s.released || s.looter_pass || self.assigned.is_none_or(|a| a == g))
                .then_some(SLOT_ALLOW),
        }
    }
}

/// What one viewer may see in a loot: the quest drops they need, the
/// conditions they meet, and the quest-starting items whose quest they
/// have begun (mangos's `AllowedForPlayer`).
#[derive(Debug, Clone, Default)]
pub struct Sight {
    pub wanted: Vec<u32>,
    pub met: Vec<u32>,
    pub started: Vec<u32>,
}

/// [`Sight`] of `viewer` on `loot`, on the corpse or object `source`.
pub fn sight(map: &Map, loot: &Loot, source: Guid, viewer: Guid) -> Sight {
    let data = &map.realm.data;
    let started = map.players.get(&viewer).map_or_else(Vec::new, |p| {
        loot.items
            .iter()
            .filter(|s| {
                data.item(s.item).is_some_and(|t| {
                    t.start_quest != 0
                        && t.extra_flags & ITEM_EXTRA_IGNORE_QUEST_STATUS == 0
                        && crate::game::map::quests::begun(p, t.start_quest)
                })
            })
            .map(|s| s.item)
            .collect()
    });
    Sight {
        wanted: crate::game::map::quests::wanted(map, viewer),
        met: conditions_met(map, loot, source, viewer),
        started,
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
    /// What a rogue left in a creature's pockets: theirs again when they
    /// pick them again, this life (mangos's kept loot, Thistle Tea's
    /// `Pockets`).
    pub pockets: ahash::AHashMap<(Guid, Guid), Loot>,
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
#[allow(clippy::too_many_arguments)]
fn reference(
    data: &GameData,
    e: &LootEntry,
    wanted: &[u32],
    rng: &mut impl rand::Rng,
    out: &mut Vec<LootSlot>,
    depth: u32,
    allowed: &dyn Fn(u32) -> bool,
    rates: &RatesConfig,
) {
    if e.condition != 0 && !allowed(e.condition) {
        return;
    }
    if let Some(r) = data.loot.reference.get(&(e.min.unsigned_abs())) {
        for _ in 0..e.max.max(1) {
            roll_rated(data, r, wanted, rng, out, depth + 1, allowed, rates);
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
        ffa: !is_quest_item(e)
            && data
                .item(e.item)
                .is_some_and(|t| t.flags & ITEM_FLAG_MULTI_DROP != 0),
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

/// The drop rates of 1.12 itself.
static AS_THEY_ARE: std::sync::LazyLock<RatesConfig> = std::sync::LazyLock::new(Default::default);

/// An ungrouped entry's chance at `rates` (mangos's `LootStoreItem::Roll`):
/// a reference's, a quest item's, or its quality's. A group's entries are
/// picked among at their own chances, as mangos picks them.
fn rated_chance(data: &GameData, e: &LootEntry, rates: &RatesConfig) -> f32 {
    let chance = e.chance.abs();
    if rates.drops_as_they_are() {
        return chance;
    }
    let rate = if e.min < 0 {
        rates.drop_item_referenced
    } else if is_quest_item(e) {
        rates.drop_item_quest
    } else {
        rates.drop_item(data.item(e.item).map_or(0, |t| t.quality))
    };
    chance * rate
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
    roll_rated(
        data,
        entries,
        wanted,
        rng,
        out,
        depth,
        allowed,
        &AS_THEY_ARE,
    );
}

/// [`roll_for`] at the server's drop rates (`[rates] drop_item_*`).
#[allow(clippy::too_many_arguments)]
pub fn roll_rated(
    data: &GameData,
    entries: &[LootEntry],
    wanted: &[u32],
    rng: &mut impl rand::Rng,
    out: &mut Vec<LootSlot>,
    depth: u32,
    allowed: &dyn Fn(u32) -> bool,
    rates: &RatesConfig,
) {
    let skip = |e: &LootEntry| is_quest_item(e) && !wanted.contains(&e.item);
    // An entry whose condition no one it may fall to meets isn't rolled,
    // and a group's roll falls to another (mangos's
    // `PlayerOrGroupFulfilsCondition`, in `Process` and `LootGroup::Roll`).
    let unmet = |e: &LootEntry| e.condition != 0 && !allowed(e.condition);
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
    for e in entries
        .iter()
        .filter(|e| e.group == 0 && !skip(e) && !unmet(e))
    {
        if !chance(rng, rated_chance(data, e, rates)) {
            continue;
        }
        if e.min < 0 {
            reference(data, e, wanted, rng, out, depth, allowed, rates);
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
        for e in members.iter().filter(|e| e.chance != 0.0 && !unmet(e)) {
            r -= e.chance.abs();
            if r < 0.0 {
                picked = Some(**e);
                break;
            }
        }
        if picked.is_none() {
            // The equally chanced in a random order, the first whose
            // condition holds; one already in the loot passed over half the
            // time (mangos's `IsItemAlreadyIn`).
            let mut equal: Vec<&LootEntry> = members
                .iter()
                .copied()
                .filter(|e| e.chance == 0.0 && !unmet(e))
                .collect();
            rand::seq::SliceRandom::shuffle(equal.as_mut_slice(), rng);
            picked = equal
                .iter()
                .find(|e| !out.iter().any(|s| s.item == e.item) || rng.random_bool(0.5))
                .map(|e| **e);
        }
        let Some(e) = picked.filter(|e| !skip(e)) else {
            continue;
        };
        if e.min < 0 {
            reference(data, &e, wanted, rng, out, depth, allowed, rates);
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
    map: &Map,
    table: &[LootEntry],
    owner: Guid,
    wanted: &[u32],
    kind: LootKind,
) -> Loot {
    let mut items = Vec::new();
    // Its conditions, the owner's to meet.
    let met = |c: u32| crate::game::map::conditions::met(map, c, None, Some(owner));
    roll_rated(
        &map.realm.data,
        table,
        wanted,
        &mut rand::rng(),
        &mut items,
        0,
        &met,
        &map.cfg.rates,
    );
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
    tapped_for(map, tapper, None, at)
}

/// [`tap_group`], of the group `group` a creature was tapped for while it
/// stands (`map::tap::recipients`).
fn tapped_for(
    map: &Map,
    tapper: Guid,
    group: Option<u32>,
    at: wow_world_messages::vanilla::Vector3d,
) -> Vec<Guid> {
    let members = crate::game::map::tap::recipients(&map.realm, tapper, group);
    let mut near: Vec<Guid> = vec![tapper];
    near.extend(members.into_iter().filter(|g| {
        *g != tapper
            && map
                .players
                .get(g)
                .is_some_and(|p| dist(p.position(), at) <= map.cfg.players.group_reward_yards)
    }));
    near
}

/// A creature died: its loot is rolled for its tapper and their group.
pub fn creature_died(map: &mut Map, creature: Guid, tapper: Guid) {
    let data = map.realm.data.clone();
    let cfg = map.cfg.clone();
    let Some((entry, at, tap_group)) = map.creature(creature).map(|c| {
        (
            c.entry,
            c.position,
            crate::game::map::tap::group_of(c, tapper),
        )
    }) else {
        return;
    };
    let Some(t) = data.creature_template(entry) else {
        return;
    };
    let tapped = guid::is_player(tapper);
    let group = if tapped {
        tapped_for(map, tapper, tap_group, at)
    } else {
        Vec::new()
    };
    let mut wanted = Vec::new();
    for g in &group {
        wanted.extend(crate::game::map::quests::wanted(map, *g));
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
        // A condition rolls if one it may fall to meets it.
        let met = |c: u32| {
            group
                .iter()
                .any(|g| crate::game::map::conditions::met(map, c, Some(creature), Some(*g)))
        };
        roll_rated(
            &data, table, &wanted, &mut rng, &mut items, 0, &met, &cfg.rates,
        );
    }
    let gold = if !allowed {
        0
    } else if t.gold_max > t.gold_min {
        rng.random_range(t.gold_min..=t.gold_max)
    } else {
        t.gold_min
    };
    let gold = crate::config::scaled(gold, cfg.rates.drop_money);
    let mut loot = Loot {
        items,
        gold,
        owner: tapped.then_some(tapper),
        ..Default::default()
    };
    // A kill that leaves nothing to loot sets no group's rules on it, nor
    // turns round robin on (Thistle Tea's `prepare_loot`: no session).
    if group.len() > 1 && allowed {
        setup_group(map, &mut loot, creature, tapper, tap_group, group);
    }
    let rolls: Vec<usize> = loot.rolls.iter().map(|r| r.slot).collect();
    forget_pockets(map, creature);
    // Its pockets, open as it dies, shut on those looking in them: the
    // corpse's loot is another (mangos's `Loot::~Loot`, `SendReleaseForAll`,
    // as `PrepareCorpseLoot` replaces the pickpocketing loot).
    let open: Vec<Guid> = map
        .creature(creature)
        .and_then(|c| c.loot.as_ref())
        .map(|l| l.looters.iter().copied().collect())
        .unwrap_or_default();
    for g in open {
        if let Some(p) = map.players.get_mut(&g)
            && p.looting == Some(creature)
        {
            p.looting = None;
            p.fields
                .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_LOOTING, false);
            p.outbound.send(&SMSG_LOOT_RELEASE_RESPONSE {
                guid: creature,
                unknown1: 1,
            });
        }
    }
    let Some(c) = map.creature_mut(creature) else {
        return;
    };
    c.loot = Some(loot);
    // Nothing on it anyone it fell to may take: it can be skinned at once
    // (mangos's `IsLootedForAll` at `CreateLoot`).
    let lootable = refresh_lootable(map, creature);
    let Some(c) = map.creature_mut(creature) else {
        return;
    };
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
fn setup_group(
    map: &mut Map,
    loot: &mut Loot,
    creature: Guid,
    tapper: Guid,
    tap_group: Option<u32>,
    near: Vec<Guid>,
) {
    let (method, master, threshold, assigned) = {
        let mut groups = map.realm.social.groups.lock();
        // The group it was tapped for, while it stands; else the tapper's.
        let id = tap_group
            .filter(|id| groups.get(*id).is_some())
            .or_else(|| groups.group_of(tapper).map(|g| g.id));
        let Some(g) = id.and_then(|id| groups.get_mut(id)) else {
            return;
        };
        // The next member near, after the last looter in the group's
        // order, those away passed over: the current looter, under every
        // method but free for all (mangos's `SetGroupLootRight`, Thistle
        // Tea's `update_looter/3`).
        let assigned = (g.loot_method != GroupLootSetting::FreeForAll).then(|| {
            let order: Vec<Guid> = g.members.iter().map(|m| m.guid).collect();
            let after = order
                .iter()
                .position(|m| *m == g.round_robin)
                .map_or(0, |i| i + 1);
            let next = (0..order.len())
                .map(|k| order[(after + k) % order.len()])
                .find(|m| near.contains(m))
                .unwrap_or(tapper);
            g.round_robin = next;
            next
        });
        (g.loot_method, g.looter, quality(g.threshold), assigned)
    };
    // Master loot with no master near is group loot (mangos's).
    let method = if method == GroupLootSetting::MasterLoot && !near.contains(&master) {
        GroupLootSetting::GroupLoot
    } else {
        method
    };
    loot.owner = None;
    loot.allowed = near.clone();
    loot.method = Some(method);
    loot.assigned = assigned;
    // What the current looter may not see is anyone's at once (mangos's
    // `currentLooterPass`).
    if let Some(looter) = assigned {
        let sight = sight(map, loot, creature, looter);
        for s in &mut loot.items {
            s.looter_pass = (s.condition != 0 && !sight.met.contains(&s.condition))
                || sight.started.contains(&s.item);
        }
    }
    let rollable: Vec<usize> = loot
        .items
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.quest && !s.ffa && s.quality >= threshold)
        .map(|(i, _)| i)
        .collect();
    match method {
        GroupLootSetting::MasterLoot => {
            loot.master = Some(master);
            for i in rollable {
                loot.items[i].blocked = true;
            }
        }
        GroupLootSetting::GroupLoot | GroupLootSetting::NeedBeforeGreed => {
            let now = map.now;
            let data = map.realm.data.clone();
            for i in rollable {
                let item = data.item(loot.items[i].item);
                let condition = loot.items[i].condition;
                // Those near who meet the item's condition (mangos's
                // `AllowedForPlayer`); for need before greed, who could
                // use it.
                let eligible: Vec<Guid> = near
                    .iter()
                    .copied()
                    .filter(|g| {
                        crate::game::map::conditions::met(map, condition, Some(creature), Some(*g))
                    })
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
        // Passed by all: anyone's (mangos's `Finish`, `isReleased`).
        if let Some(l) = loot_mut(map, object)
            && let Some(x) = l.items.get_mut(slot)
        {
            x.blocked = false;
            x.released = true;
        }
        finish_rolls(map, object);
        return;
    }
    let mut rng = rand::rng();
    let rolled: Vec<(Guid, u8)> = contenders
        .iter()
        .map(|g| (*g, rng.random_range(1..=100u8)))
        .collect();
    let (rolled, (winner, number)) = pick_winner(rolled).expect("there are contenders");
    award(map, object, slot, winner, number, kind, &rolled);
    let _ = r;
}

/// Who rolled what.
pub type Rolls = Vec<(Guid, u8)>;

/// The rolls in guid order, and their winner: the first of the highest
/// (Thistle Tea's `LootRoll.pick_winner`); none with no one rolling.
pub fn pick_winner(mut rolled: Rolls) -> Option<(Rolls, (Guid, u8))> {
    rolled.sort_unstable_by_key(|(g, _)| g.guid());
    let best = rolled
        .iter()
        .copied()
        .fold(None, |best: Option<(Guid, u8)>, x| match best {
            Some(b) if b.1 >= x.1 => Some(b),
            _ => Some(x),
        })?;
    Some((rolled, best))
}

/// The roll's winner gets the item, if it fits; if not, theirs to take
/// when they can. A winner gone too far has nothing: it's free.
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
        (Some(a), Some(b)) => dist(a, b) <= map.cfg.players.group_reward_yards,
        _ => false,
    };
    if near {
        if give(map, object, slot, winner).is_err()
            && let Some(x) = loot_mut(map, object).and_then(|l| l.items.get_mut(slot))
        {
            x.winner = Some(winner);
        }
    } else if let Some(l) = loot_mut(map, object)
        && let Some(x) = l.items.get_mut(slot)
    {
        x.blocked = false;
        x.released = true;
    }
    finish_rolls(map, object);
}

/// Slot `slot` of `object`'s loot goes into `to`'s bags; if it doesn't
/// fit, it's free for the taking.
fn give(map: &mut Map, object: Guid, slot: usize, to: Guid) -> Result<(), inventory::Fail> {
    let Some(s) = loot_ref(map, object).and_then(|l| l.items.get(slot).cloned()) else {
        return Err(inventory::Fail::NotFound);
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
            Ok(())
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
            Err(f)
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

/// Whether a corpse still has loot, and its sparkle: while one it fell
/// to would find anything in it (mangos's `IsLootedForAll`). A corpse
/// looted for all is [`Loot::done`]: it may be skinned, whatever is left
/// in it that no one may take.
pub fn refresh_lootable(map: &mut Map, object: Guid) -> bool {
    refresh_quiet(map, object);
    // A quest drop is still there while one it fell to hasn't taken theirs
    // (mangos's `IsLootedFor`).
    let open = |l: &Loot, s: &LootSlot| {
        (s.quest || s.ffa)
            && !s.taken
            && if !l.allowed.is_empty() {
                l.allowed.iter().any(|g| !s.taken_by.contains(g))
            } else {
                l.owner.is_none_or(|o| !s.taken_by.contains(&o))
            }
    };
    let Some(l) = map.creature(object).and_then(|c| c.loot.as_ref()) else {
        return false;
    };
    let rights: Vec<Guid> = if l.allowed.is_empty() {
        l.owner.into_iter().collect()
    } else {
        l.allowed.clone()
    };
    let lootable = if l.done {
        false
    } else if rights.is_empty() || l.kind != LootKind::Corpse {
        !l.is_empty() || l.items.iter().any(|s| open(l, s))
    } else {
        rights.iter().any(|g| !l.quiet.contains(g))
    };
    let done = !lootable && l.kind == LootKind::Corpse && !rights.is_empty();
    if let Some(c) = map.creature_mut(object) {
        c.fields
            .set_flag(UNIT_DYNAMIC_FLAGS, DYNFLAG_LOOTABLE, lootable);
        if let Some(l) = c.loot.as_mut()
            && done
        {
            l.done = true;
        }
    }
    lootable
}

/// A corpse lies `within` from now at most, or (`at_least`) at least.
fn corpse_within(map: &mut Map, creature: Guid, within: Duration, at_least: bool) {
    let until = map.now + within;
    if let Some(c) = map.creature_mut(creature)
        && let crate::game::map::creature::Life::Dead { corpse_until, .. } = &mut c.life
        && (if at_least {
            *corpse_until < until
        } else {
            *corpse_until > until
        })
    {
        *corpse_until = until;
    }
}

/// Who of those a corpse's loot fell to would find nothing in it of
/// theirs ([`Loot::quiet`]): no gold, no roll, and no slot they see.
pub fn refresh_quiet(map: &mut Map, object: Guid) {
    let Some(l) = map.creature(object).and_then(|c| c.loot.as_ref()) else {
        return;
    };
    let quiet: Vec<Guid> = if l.gold > 0 || !l.rolls.is_empty() {
        Vec::new()
    } else {
        let who: Vec<Guid> = if l.allowed.is_empty() {
            l.owner.into_iter().collect()
        } else {
            l.allowed.clone()
        };
        let mut quiet: Vec<Guid> = who
            .into_iter()
            .filter(|&g| !l.has_for(g, &sight(map, l, object, g)))
            .collect();
        quiet.sort_unstable();
        quiet
    };
    if let Some(l) = map.creature_mut(object).and_then(|c| c.loot.as_mut()) {
        l.quiet = quiet;
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
    let sight = Sight {
        wanted: wanted.to_vec(),
        met: met.to_vec(),
        started: Vec::new(),
    };
    response_for(data, g, loot, viewer, &sight)
}

/// [`response`], by `viewer`'s [`Sight`].
pub fn response_for(
    data: &GameData,
    g: Guid,
    loot: &Loot,
    viewer: Guid,
    sight: &Sight,
) -> Arc<[u8]> {
    let mut b = Body::default();
    b.u64(g.guid()).u8(loot.kind.wire()).u32(loot.gold);
    let shown: Vec<(usize, &LootSlot, u8)> = loot
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, s)| Some((i, s, loot.slot_type(i, viewer, sight)?)))
        .collect();
    b.u8(shown.len() as u8);
    for (i, s, kind) in shown {
        let display = data.item(s.item).map(|t| t.display_id).unwrap_or(0);
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

/// A creature's pockets are let go: picked or not, and what rogues left
/// in them. At its death, and as it leaves the map (despawned by a
/// script, a pool or an event, or come back as new).
pub fn forget_pockets(map: &mut Map, creature: Guid) {
    map.loot_state.picked.remove(&creature);
    map.loot_state.pockets.retain(|(c, _), _| *c != creature);
}

/// Loot on an item opens at once: a disenchanted item's dust, an opened
/// container's contents.
pub fn show_item_loot(map: &mut Map, player: Guid, item: Guid, loot: Loot) {
    map.loot_state.item_loot.insert(item, loot);
    show(map, player, item);
}

/// `CMSG_LOOT`: open the loot window on a corpse or object.
pub fn open(map: &mut Map, player: Guid, target: Guid) {
    let Some(p) = map.players.get(&player) else {
        return;
    };
    // Not the dead (mangos's `HandleLootOpcode`), the feared nor the
    // confused (Thistle Tea's `Looting.open`): the window shuts.
    use crate::game::map::auras::{AURA_MOD_CONFUSE, AURA_MOD_FEAR};
    if p.death != crate::game::map::death::Death::Alive
        || p.auras.has(AURA_MOD_FEAR)
        || p.auras.has(AURA_MOD_CONFUSE)
    {
        p.outbound.send(&SMSG_LOOT_RELEASE_RESPONSE {
            guid: target,
            unknown1: 1,
        });
        return;
    }
    let ppos = p.position();
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
    // A window open on something else shuts first, as the client is told
    // (Thistle Tea's `Looting.open`, its item loot settled before a
    // node's): its loot isn't left with this player among its lookers.
    if let Some(open) = map
        .players
        .get(&player)
        .and_then(|p| p.looting)
        .filter(|t| *t != target)
    {
        close(map, player, open);
    }
    let data = map.realm.data.clone();
    let Some(sight) = loot_ref(map, target).map(|l| sight(map, l, target, player)) else {
        return;
    };
    let Some(loot) = loot_mut(map, target) else {
        return;
    };
    // A corpse with nothing in it of theirs isn't theirs to open: the
    // window shuts (mangos's `ShowContentTo`, by `CanLoot`).
    let nothing = guid::is_unit(target)
        && loot.kind == LootKind::Corpse
        && !loot.items.is_empty()
        && !loot.has_for(player, &sight);
    if nothing && loot.may_loot(player) {
        if let Some(p) = map.players.get(&player) {
            p.outbound.send(&SMSG_LOOT_RELEASE_RESPONSE {
                guid: target,
                unknown1: 1,
            });
        }
        return;
    }
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
    // Looked in, a corpse lies long enough to loot (mangos's
    // `InspectingLoot`).
    if guid::is_unit(target) && loot.kind == LootKind::Corpse {
        corpse_within(map, target, LOOTING_TIME, true);
    }
    let Some(loot) = loot_ref(map, target) else {
        return;
    };
    let master_list = (loot.master == Some(player)).then(|| loot.allowed.clone());
    let frame = response_for(&data, target, loot, player, &sight);
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
    let Some(target) = map.players.get(&player).and_then(|p| p.looting) else {
        return;
    };
    let Some(sight) = loot_ref(map, target).map(|l| sight(map, l, target, player)) else {
        return;
    };
    let Some((item, count, property, quest)) = loot_ref(map, target).and_then(|l| {
        let i = usize::from(slot);
        let s = l.items.get(i)?;
        (l.may_loot(player) && l.slot_type(i, player, &sight) == Some(SLOT_ALLOW)).then_some((
            s.item,
            s.count,
            s.random_property,
            s.quest || s.ffa,
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
            if guid::is_unit(target) {
                refresh_lootable(map, target);
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
    let Some((gold, looters, allowed)) = loot_mut(map, target).map(|l| {
        let gold = if l.may_loot(player) {
            std::mem::take(&mut l.gold)
        } else {
            0
        };
        (
            gold,
            l.looters.iter().copied().collect::<Vec<_>>(),
            l.allowed.clone(),
        )
    }) else {
        return;
    };
    if gold == 0 {
        return;
    }
    // A group's kill's money is shared among those it fell to (mangos's
    // `SendGold`, by `m_ownerSet`: near the kill, on its map, alive or a
    // corpse near, as it fell), in even shares, the remainder lost as
    // mangos loses it; any other's (a lone kill, a chest, a pocket, a
    // container) is the looter's. Only those still on the map share it:
    // mangos pays one gone to another map, wvs can't reach them from here,
    // so their share is the others' rather than lost.
    let (sharers, share) = if allowed.is_empty() {
        (vec![player], gold)
    } else {
        let present: Vec<Guid> = allowed
            .into_iter()
            .filter(|g| map.players.contains_key(g))
            .collect();
        // The looter is one of them (`may_loot`), so there is at least one.
        let share = gold / present.len().max(1) as u32;
        (present, share)
    };
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
    if guid::is_unit(target) {
        refresh_lootable(map, target);
    }
    crate::game::map::instance_script::chest_changed(map, target);
}

/// `CMSG_LOOT_MASTER_GIVE`: the master looter gives a kept item to a
/// member near.
pub fn master_give(map: &mut Map, giver: Guid, object: Guid, slot: u8, to: Guid) {
    // To one of the group on the map, wherever on it they stand (mangos's
    // `HandleLootMasterGiveOpcode`: `IsInGroup`, `IsInMap`), it fell to
    // and meeting the item's condition (mangos's `IsAllowed`, Thistle
    // Tea's `reserve_master`). mangos tells the master why it can't; that
    // `SMSG_LOOT_RESPONSE` (loot type 0 and an error) wvs doesn't send, as
    // wow_messages reads it otherwise and no client check covers it.
    let ok = map.players.contains_key(&to)
        && loot_ref(map, object).is_some_and(|l| {
            l.master == Some(giver)
                && l.allowed.contains(&to)
                && l.items.get(usize::from(slot)).is_some_and(|s| {
                    s.blocked
                        && !s.taken
                        && crate::game::map::conditions::met(
                            map,
                            s.condition,
                            Some(object),
                            Some(to),
                        )
                })
        })
        && map
            .players
            .get(&giver)
            .is_some_and(|p| p.looting == Some(object))
        && map
            .realm
            .social
            .groups
            .lock()
            .group_of(giver)
            .is_some_and(|g| g.is_member(to));
    // What doesn't fit stays the master's to give (Thistle Tea's master
    // reservation).
    if ok
        && give(map, object, usize::from(slot), to).is_err()
        && let Some(x) = loot_mut(map, object).and_then(|l| l.items.get_mut(usize::from(slot)))
    {
        x.blocked = true;
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
/// Shut `player`'s window on `target`, an item's or another's, as
/// `CMSG_LOOT_RELEASE` does.
pub fn close(map: &mut Map, player: Guid, target: Guid) {
    if guid::high(target) == Some(guid::HighGuid::Item) {
        crate::game::map::items_misc::release(map, player, target);
        // A disenchant's leftovers go with its window (mangos's release of
        // an item's loot).
        map.loot_state.item_loot.remove(&target);
    } else {
        release(map, player, target);
    }
}

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
            // Only a window of one's own closes: another's release would
            // take a rogue's pockets from under them.
            if !l.looters.remove(&player) {
                return;
            }
            // Closed, a lock must be opened again.
            l.opened_by.remove(&player);
            // The current looter closes it: what's left that isn't rolled
            // for is anyone's it fell to (mangos's `Loot::Release`).
            if l.assigned == Some(player) && l.method.is_some() {
                for s in l.items.iter_mut().filter(|s| !s.blocked) {
                    s.released = true;
                }
            }
            (l.is_empty(), l.kind)
        }
        None => return,
    };
    if let Some(l) = map.loot_state.item_loot.remove(&target) {
        // What's left of a disenchant goes into the bags, what doesn't fit
        // lost (mangos's `Loot::Release`: `AutoStore`, "can be lost if no
        // space"; Thistle Tea's "automatically stores remaining
        // materials"); what's left on another item is lost with the
        // window.
        if kind == LootKind::Disenchant {
            for s in l
                .items
                .iter()
                .filter(|s| !s.taken && !s.quest && s.count > 0)
            {
                let _ = crate::game::map::inventory::add_item(map, player, s.item, s.count, true);
            }
        }
        return;
    }
    if guid::is_unit(target) {
        if kind == LootKind::Pickpocket {
            // The pockets are picked; what's left in them is the rogue's
            // to find again.
            if let Some(l) = map.creature_mut(target).and_then(|c| c.loot.take())
                && !empty
            {
                map.loot_state.pockets.insert((target, player), l);
            }
            return;
        }
        if kind == LootKind::Skinning {
            if empty {
                // Skinned: the corpse goes (mangos's
                // `CREATURE_LOOT_STATUS_SKINNED`).
                corpse_within(map, target, Duration::ZERO, false);
            } else {
                // A skin left is free for all to the skinner's group
                // (mangos's `Loot::Release`, `SKIN_AVAILABLE`).
                let group: Vec<Guid> = {
                    let groups = map.realm.social.groups.lock();
                    groups
                        .group_of(player)
                        .map(|g| g.members.iter().map(|m| m.guid).collect())
                        .unwrap_or_default()
                };
                if let Some(l) = loot_mut(map, target)
                    && !group.is_empty()
                {
                    l.owner = None;
                    l.allowed = group;
                    l.method = Some(GroupLootSetting::FreeForAll);
                }
                refresh_lootable(map, target);
            }
            return;
        }
        let lootable = refresh_lootable(map, target);
        let empty = empty || loot_ref(map, target).is_some_and(|l| l.done);
        if (empty || !lootable) && kind == LootKind::Corpse {
            // Looted for all with no skin on it, it goes sooner (mangos's
            // `ReduceCorpseDecayTimer`, as the window closes: at the kill,
            // its decay is set after).
            let skins = map
                .creature(target)
                .and_then(|c| map.realm.data.creature_template(c.entry))
                .is_some_and(|t| t.skinning_loot_id != 0);
            if !skins {
                corpse_within(map, target, LOOTING_TIME, false);
            }
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
    let wanted = crate::game::map::quests::wanted(map, player);
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
            roll_rated(
                &data,
                table,
                &wanted,
                &mut rand::rng(),
                &mut items,
                0,
                &allowed,
                &map.cfg.rates,
            );
        }
        let gold = if t.gold_max > t.gold_min {
            rand::rng().random_range(t.gold_min..=t.gold_max)
        } else {
            t.gold_min
        };
        let gold = crate::config::scaled(gold, map.cfg.rates.drop_money);
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
