//! Quests: who offers what, the dialogs from hello to reward, the quest
//! log in the update fields, objectives (kills, items, exploration), and
//! rewards.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::npc::{QUEST_SPECIAL_REPEATABLE, Quest};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::inventory;
use crate::game::map::player::Player;

pub const QUEST_LOG_SIZE: usize = 20;
/// The quest log's second field: four 6-bit counters, then these.
const QUEST_STATE_COMPLETE: u32 = 1 << 24;
const QUEST_STATE_FAILED: u32 = 1 << 25;

pub use crate::game::store::{Journal, Logged};

impl Journal {
    pub fn slot_of(&self, quest: u32) -> Option<usize> {
        self.log.iter().position(|l| l.quest == quest)
    }

    pub fn has(&self, quest: u32) -> bool {
        self.slot_of(quest).is_some()
    }
}

/// Whether a quest's objectives are all met.
pub fn complete(p: &Player, q: &Quest, l: &Logged) -> bool {
    if l.failed {
        return false;
    }
    for (i, (id, count)) in q.required_kills.iter().enumerate() {
        if *id != 0 && l.kills[i] < *count {
            return false;
        }
    }
    for (item, count) in q.required_items {
        if item != 0 && p.count_of(item) < count {
            return false;
        }
    }
    if q.flags & crate::game::data::npc::QUEST_FLAGS_EXPLORATION != 0 && !l.explored {
        return false;
    }
    true
}

/// Whether `p` may take `q`.
pub fn can_take(p: &Player, q: &Quest, data: &GameData) -> bool {
    let j = &p.journal;
    if j.has(q.id) {
        return false;
    }
    if j.rewarded.contains(&q.id) && q.special_flags & QUEST_SPECIAL_REPEATABLE == 0 {
        return false;
    }
    if p.level() < q.min_level {
        return false;
    }
    if q.required_races != 0 && q.required_races & (1 << (p.row.race - 1)) == 0 {
        return false;
    }
    if q.required_classes != 0 && q.required_classes & (1 << (p.row.class - 1)) == 0 {
        return false;
    }
    if q.prev_quest > 0 && !j.rewarded.contains(&(q.prev_quest as u32)) {
        return false;
    }
    if q.prev_quest < 0 && !j.has(q.prev_quest.unsigned_abs()) {
        return false;
    }
    if q.exclusive_group > 0 {
        let taken = data.npc.quests.values().any(|o| {
            o.exclusive_group == q.exclusive_group
                && o.id != q.id
                && (j.rewarded.contains(&o.id) || j.has(o.id))
        });
        if taken {
            return false;
        }
    }
    true
}

/// Whether `p` is a few levels short of `q`: shown as unavailable.
fn soon(p: &Player, q: &Quest) -> bool {
    p.level() < q.min_level && p.level() + 4 >= q.min_level
}

/// The quests `giver` (a creature or object) starts and ends.
fn relations(map: &Map, giver: Guid) -> (Vec<u32>, Vec<u32>) {
    let npc = &map.realm.data.npc;
    if guid::is_unit(giver) {
        let entry = guid::entry(giver);
        (
            npc.creature_starts.get(&entry).cloned().unwrap_or_default(),
            npc.creature_ends.get(&entry).cloned().unwrap_or_default(),
        )
    } else {
        let entry = guid::entry(giver);
        (
            npc.object_starts.get(&entry).cloned().unwrap_or_default(),
            npc.object_ends.get(&entry).cloned().unwrap_or_default(),
        )
    }
}

/// The icon a quest giver shows `player`.
pub fn status(map: &Map, player: Guid, giver: Guid) -> QuestGiverStatus {
    let Some(p) = map.players.get(&player) else {
        return QuestGiverStatus::None;
    };
    let data = &map.realm.data;
    let (starts, ends) = relations(map, giver);
    let mut best = QuestGiverStatus::None;
    let rank = |s: QuestGiverStatus| match s {
        QuestGiverStatus::None => 0,
        QuestGiverStatus::Unavailable => 1,
        QuestGiverStatus::Incomplete => 3,
        QuestGiverStatus::Available => 5,
        QuestGiverStatus::Reward2 => 7,
        _ => 2,
    };
    let mut consider = |s: QuestGiverStatus| {
        if rank(s) > rank(best) {
            best = s;
        }
    };
    for q in ends.iter().filter_map(|id| data.npc.quests.get(id)) {
        if let Some(slot) = p.journal.slot_of(q.id) {
            consider(if complete(p, q, &p.journal.log[slot]) {
                QuestGiverStatus::Reward2
            } else {
                QuestGiverStatus::Incomplete
            });
        }
    }
    for q in starts.iter().filter_map(|id| data.npc.quests.get(id)) {
        if can_take(p, q, data) {
            consider(QuestGiverStatus::Available);
        } else if soon(p, q) {
            consider(QuestGiverStatus::Unavailable);
        }
    }
    best
}

/// The quests a giver lists for `player`, for a gossip menu or a quest
/// list.
pub fn giver_quests(map: &Map, player: Guid, giver: Guid) -> Vec<QuestItem> {
    let Some(p) = map.players.get(&player) else {
        return Vec::new();
    };
    let data = &map.realm.data;
    let (starts, ends) = relations(map, giver);
    let mut out = Vec::new();
    for q in ends.iter().filter_map(|id| data.npc.quests.get(id)) {
        if let Some(slot) = p.journal.slot_of(q.id) {
            let done = complete(p, q, &p.journal.log[slot]);
            out.push(QuestItem {
                quest_id: q.id,
                quest_icon: if done { 4 } else { 2 },
                level: Level::new(q.level.max(0) as u8),
                title: q.title.clone(),
            });
        }
    }
    for q in starts.iter().filter_map(|id| data.npc.quests.get(id)) {
        if can_take(p, q, data) && !out.iter().any(|o| o.quest_id == q.id) {
            out.push(QuestItem {
                quest_id: q.id,
                quest_icon: 2,
                level: Level::new(q.level.max(0) as u8),
                title: q.title.clone(),
            });
        }
    }
    out
}

fn near_giver(map: &Map, player: Guid, giver: Guid) -> bool {
    let (Some(p), Some(pos)) = (map.players.get(&player), map.position_of(giver)) else {
        return false;
    };
    crate::game::map::combat::dist(p.position(), pos) <= crate::game::map::vendor::NPC_YARDS + 2.0
}

/// `CMSG_QUESTGIVER_STATUS_QUERY`.
pub fn status_query(map: &Map, player: Guid, giver: Guid) {
    let s = status(map, player, giver);
    if let Some(p) = map.players.get(&player) {
        p.outbound.send(&SMSG_QUESTGIVER_STATUS {
            guid: giver,
            status: s,
        });
    }
}

/// `CMSG_QUESTGIVER_HELLO`: the giver's quests, or the one there is.
pub fn hello(map: &mut Map, player: Guid, giver: Guid) {
    if !near_giver(map, player, giver) {
        return;
    }
    let quests = giver_quests(map, player, giver);
    if quests.len() == 1 {
        let q = quests[0].quest_id;
        if map.players.get(&player).is_some_and(|p| p.journal.has(q)) {
            return complete_quest(map, player, giver, q);
        }
        return details(map, player, giver, q);
    }
    let greeting = map
        .realm
        .data
        .npc
        .quest_greetings
        .get(&guid::entry(giver))
        .cloned()
        .unwrap_or_default();
    if let Some(p) = map.players.get(&player) {
        p.outbound.send(&SMSG_QUESTGIVER_QUEST_LIST {
            npc: giver,
            title: greeting,
            emote_delay: 0,
            emote: 0,
            quest_items: quests,
        });
    }
}

fn rewards_of(q: &Quest) -> (Vec<QuestItemReward>, Vec<QuestItemReward>) {
    let pick = |v: &[(u32, u32)]| {
        v.iter()
            .filter(|(i, _)| *i != 0)
            .map(|(item, n)| QuestItemReward {
                item: *item,
                item_count: *n,
            })
            .collect()
    };
    (pick(&q.choice_rewards), pick(&q.rewards))
}

/// `CMSG_QUESTGIVER_QUERY_QUEST`, or a hello with one quest: its details.
pub fn details(map: &mut Map, player: Guid, giver: Guid, quest: u32) {
    let data = map.realm.data.clone();
    let Some(q) = data.npc.quests.get(&quest) else {
        return;
    };
    let (choice, items) = rewards_of(q);
    if let Some(p) = map.players.get(&player) {
        p.outbound.send(&SMSG_QUESTGIVER_QUEST_DETAILS {
            guid: giver,
            quest_id: quest,
            title: q.title.clone(),
            details: q.details.clone(),
            objectives: q.objectives.clone(),
            auto_finish: false,
            choice_item_rewards: choice,
            item_rewards: items,
            money_reward: (q.reward_money.max(0) as u32).into(),
            reward_spell: q.reward_spell,
            emotes: q
                .details_emotes
                .iter()
                .filter(|(e, _)| *e != 0)
                .map(|(e, d)| QuestDetailsEmote {
                    emote: *e,
                    emote_delay: std::time::Duration::from_millis(u64::from(*d)),
                })
                .collect(),
        });
    }
}

fn invalid(p: &Player, msg: QuestFailedReason) {
    p.outbound.send(&SMSG_QUESTGIVER_QUEST_INVALID { msg });
}

/// `CMSG_QUESTGIVER_ACCEPT_QUEST`: into the log.
pub fn accept(map: &mut Map, player: Guid, giver: Guid, quest: u32) {
    let data = map.realm.data.clone();
    let Some(q) = data.npc.quests.get(&quest) else {
        return;
    };
    let (starts, _) = relations(map, giver);
    let item_start = guid::high(giver) == Some(guid::HighGuid::Item)
        || (guid::is_player(giver)
            && crate::game::map::questshare::shared_by(map, player, giver, quest));
    if !item_start && (!starts.contains(&quest) || !near_giver(map, player, giver)) {
        return;
    }
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    if p.journal.has(quest) {
        return invalid(p, QuestFailedReason::QuestAlreadyOn);
    }
    if p.level() < q.min_level {
        return invalid(p, QuestFailedReason::QuestFailedLowLevel);
    }
    if !can_take(p, q, &data) {
        return invalid(p, QuestFailedReason::QuestFailedReqs);
    }
    if p.journal.log.len() >= QUEST_LOG_SIZE {
        p.outbound.send(&SMSG_QUESTLOG_FULL {});
        return;
    }
    p.journal.log.push(Logged {
        quest,
        ..Default::default()
    });
    refresh_log(p, &data);
    if q.source_item != 0 {
        let _ = inventory::add_item(map, player, q.source_item, q.source_item_count.max(1), true);
    }
    check(map, player, quest);
    crate::game::map::questshare::accepted(map, player, giver, quest);
}

/// The log's update fields, from the journal.
pub fn refresh_log(p: &mut Player, data: &GameData) {
    for slot in 0..QUEST_LOG_SIZE {
        let base = PLAYER_QUEST_LOG_1_1 + slot as u16 * QUEST_LOG_ENTRY_SIZE;
        let Some(l) = p.journal.log.get(slot).cloned() else {
            p.fields.set_u32(base, 0);
            p.fields.set_u32(base + 1, 0);
            p.fields.set_u32(base + 2, 0);
            continue;
        };
        let mut state = 0u32;
        for (i, k) in l.kills.iter().enumerate() {
            state |= (k & 0x3F) << (i * 6);
        }
        if let Some(q) = data.npc.quests.get(&l.quest)
            && complete(p, q, &l)
        {
            state |= QUEST_STATE_COMPLETE;
        }
        if l.failed {
            state |= QUEST_STATE_FAILED;
        }
        p.fields.set_u32(base, l.quest);
        p.fields.set_u32(base + 1, state);
    }
}

/// See whether a quest is now done, and say so.
pub fn check(map: &mut Map, player: Guid, quest: u32) {
    let data = map.realm.data.clone();
    let Some(q) = data.npc.quests.get(&quest) else {
        return;
    };
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let Some(slot) = p.journal.slot_of(quest) else {
        return;
    };
    let was = p
        .fields
        .get_u32(PLAYER_QUEST_LOG_1_1 + slot as u16 * QUEST_LOG_ENTRY_SIZE + 1)
        & QUEST_STATE_COMPLETE
        != 0;
    refresh_log(p, &data);
    let now = complete(p, q, &p.journal.log[slot]);
    if now && !was && q.has_objectives() {
        p.outbound
            .send(&SMSG_QUESTUPDATE_COMPLETE { quest_id: quest });
    }
}

/// A creature died: kill credit for the quests of the player it is
/// credited to.
pub fn killed(map: &mut Map, player: Guid, creature: Guid) {
    let entry = guid::entry(creature);
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let mut touched = Vec::new();
    for l in p.journal.log.iter_mut() {
        let Some(q) = data.npc.quests.get(&l.quest) else {
            continue;
        };
        for (i, (id, count)) in q.required_kills.iter().enumerate() {
            if *id > 0 && *id as u32 == entry && l.kills[i] < *count {
                l.kills[i] += 1;
                p.outbound.send(&SMSG_QUESTUPDATE_ADD_KILL {
                    quest_id: q.id,
                    creature_id: entry,
                    kill_count: l.kills[i],
                    required_kill_count: *count,
                    guid: creature,
                });
                touched.push(q.id);
            }
        }
    }
    for q in touched {
        check(map, player, q);
    }
}

/// Items came or went: the quests that need them.
pub fn items_changed(map: &mut Map, player: Guid, item: u32) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let quests: Vec<(u32, u32)> = p
        .journal
        .log
        .iter()
        .filter_map(|l| data.npc.quests.get(&l.quest))
        .filter_map(|q| {
            q.required_items
                .iter()
                .find(|(i, _)| *i == item)
                .map(|(_, n)| (q.id, *n))
        })
        .collect();
    let have = p.count_of(item);
    for (q, needed) in quests {
        if let Some(p) = map.players.get(&player) {
            p.outbound.send(&SMSG_QUESTUPDATE_ADD_ITEM {
                required_item_id: item,
                items_required: needed.saturating_sub(have),
            });
        }
        check(map, player, q);
    }
}

/// An area trigger: exploration for the quests it is for.
pub fn explored(map: &mut Map, player: Guid, trigger: u32) {
    let Some(&quest) = map.realm.data.npc.area_trigger_quests.get(&trigger) else {
        return;
    };
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let Some(slot) = p.journal.slot_of(quest) else {
        return;
    };
    p.journal.log[slot].explored = true;
    check(map, player, quest);
}

/// The quest items a looter's quests still need: they drop for them.
pub fn wanted_items(p: &Player, data: &GameData) -> Vec<u32> {
    let mut out = Vec::new();
    for l in &p.journal.log {
        let Some(q) = data.npc.quests.get(&l.quest) else {
            continue;
        };
        for (item, count) in q.required_items.iter().chain(q.source_items.iter()) {
            if *item != 0 && p.count_of(*item) < *count {
                out.push(*item);
            }
        }
    }
    out
}

/// `CMSG_QUESTGIVER_COMPLETE_QUEST`: the ender asks for what the quest
/// needs, or offers its reward.
pub fn complete_quest(map: &mut Map, player: Guid, giver: Guid, quest: u32) {
    let data = map.realm.data.clone();
    let Some(q) = data.npc.quests.get(&quest) else {
        return;
    };
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let Some(slot) = p.journal.slot_of(quest) else {
        return;
    };
    let done = complete(p, q, &p.journal.log[slot]);
    let needs_items = q.required_items.iter().any(|(i, _)| *i != 0) || q.reward_money < 0;
    if needs_items || !done {
        let required_items = q
            .required_items
            .iter()
            .filter(|(i, _)| *i != 0)
            .map(|(item, n)| QuestItemRequirement {
                item: *item,
                item_count: *n,
                item_display_id: data.item(*item).map(|t| t.display_id).unwrap_or(0),
            })
            .collect();
        p.outbound.send(&SMSG_QUESTGIVER_REQUEST_ITEMS {
            npc: giver,
            quest_id: quest,
            title: q.title.clone(),
            request_items_text: q.request_items_text.clone(),
            emote_delay: 0,
            emote: if done {
                q.complete_emote
            } else {
                q.incomplete_emote
            },
            auto_finish: false,
            required_money: (q.reward_money.min(0).unsigned_abs()).into(),
            required_items,
            unknown1: 0,
            completable: if done {
                QuestCompletable::Completeable
            } else {
                QuestCompletable::NotCompletable
            },
            flags2: 4,
            flags3: 8,
        });
        return;
    }
    offer_reward(map, player, giver, quest);
}

/// `CMSG_QUESTGIVER_REQUEST_REWARD`, or a completed quest without items:
/// the reward.
pub fn offer_reward(map: &mut Map, player: Guid, giver: Guid, quest: u32) {
    let data = map.realm.data.clone();
    let Some(q) = data.npc.quests.get(&quest) else {
        return;
    };
    let Some(p) = map.players.get(&player) else {
        return;
    };
    if !p
        .journal
        .slot_of(quest)
        .is_some_and(|s| complete(p, q, &p.journal.log[s]))
    {
        return;
    }
    let req = |v: &[(u32, u32)]| {
        v.iter()
            .filter(|(i, _)| *i != 0)
            .map(|(item, n)| QuestItemRequirement {
                item: *item,
                item_count: *n,
                item_display_id: data.item(*item).map(|t| t.display_id).unwrap_or(0),
            })
            .collect()
    };
    p.outbound.send(&SMSG_QUESTGIVER_OFFER_REWARD {
        npc: giver,
        quest_id: quest,
        title: q.title.clone(),
        offer_reward_text: q.offer_reward_text.clone(),
        auto_finish: false,
        emotes: q
            .offer_reward_emotes
            .iter()
            .filter(|(e, _)| *e != 0)
            .map(|(e, d)| NpcTextUpdateEmote {
                delay: *d,
                emote: *e,
            })
            .collect(),
        choice_item_rewards: req(&q.choice_rewards),
        item_rewards: req(&q.rewards),
        money_reward: (q.reward_money.max(0) as u32).into(),
        reward_spell: q.reward_spell,
        reward_spell_cast: q.reward_spell_cast,
    });
}

/// The XP a quest gives a player of `level` (vmangos: less as the player
/// outlevels it).
pub fn quest_xp(q: &Quest, level: u8) -> u32 {
    let qlevel = q.level_for(level);
    let diff = i64::from(level) - i64::from(qlevel);
    let pct = match diff {
        i64::MIN..=5 => 100,
        6 => 80,
        7 => 60,
        8 => 40,
        9 => 20,
        _ => 10,
    };
    q.reward_xp * pct / 100
}

/// `CMSG_QUESTGIVER_CHOOSE_REWARD`: the quest is done.
pub fn choose_reward(map: &mut Map, player: Guid, giver: Guid, quest: u32, choice: u32) {
    let data = map.realm.data.clone();
    let Some(q) = data.npc.quests.get(&quest).cloned() else {
        return;
    };
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let Some(slot) = p.journal.slot_of(quest) else {
        return;
    };
    if !complete(p, &q, &p.journal.log[slot]) {
        return;
    }
    if q.reward_money < 0 && p.money() < q.reward_money.unsigned_abs() {
        return;
    }
    // Room for the rewards first.
    let mut rewards: Vec<(u32, u32)> = q.rewards.iter().copied().filter(|(i, _)| *i != 0).collect();
    let choices: Vec<(u32, u32)> = q
        .choice_rewards
        .iter()
        .copied()
        .filter(|(i, _)| *i != 0)
        .collect();
    if !choices.is_empty() {
        match choices.get(choice as usize) {
            Some(c) => rewards.push(*c),
            None => return,
        }
    }
    let free = p
        .carry_slots(&data)
        .iter()
        .filter(|(b, s)| p.item_at(*b, *s).is_none())
        .count();
    if free < rewards.len() {
        return invalid(p, QuestFailedReason::QuestFailedInventoryFull);
    }
    // Take what the quest needed.
    for (item, count) in q.required_items.iter().filter(|(i, _)| *i != 0) {
        take_items(map, player, *item, *count);
    }
    for (item, count) in &rewards {
        let _ = inventory::add_item(map, player, *item, (*count).max(1), true);
    }
    let level = map.players.get(&player).map(|p| p.level()).unwrap_or(1);
    let xp = if level >= crate::game::map::stats::MAX_LEVEL {
        0
    } else {
        quest_xp(&q, level)
    };
    let money = if level >= crate::game::map::stats::MAX_LEVEL {
        q.reward_money.max(0) as u32 + q.reward_money_max_level
    } else {
        q.reward_money.max(0) as u32
    };
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let cost = q.reward_money.min(0).unsigned_abs();
    let after = p.money().saturating_add(money).saturating_sub(cost);
    p.set_money(after);
    if let Some(slot) = p.journal.slot_of(quest) {
        p.journal.log.remove(slot);
    }
    p.journal.rewarded.insert(quest);
    refresh_log(p, &data);
    p.outbound.send(&SMSG_QUESTGIVER_QUEST_COMPLETE {
        quest_id: quest,
        unknown: 3,
        experience_reward: xp,
        money_reward: money.into(),
        item_rewards: rewards
            .iter()
            .map(|(item, n)| QuestItemReward {
                item: *item,
                item_count: *n,
            })
            .collect(),
    });
    if xp > 0 {
        crate::game::map::stats::give_xp(map, player, xp, None);
    }
    if q.reward_spell != 0 {
        crate::game::map::spells::learn(map, player, q.reward_spell);
    }
    crate::game::map::reputation::quest_rewarded(map, player, &q);
    crate::game::social::mail::quest_mail(&map.realm, player, giver, quest);
    // The chain's next quest, offered at once.
    if q.next_in_chain != 0
        && map
            .players
            .get(&player)
            .and_then(|p| {
                data.npc
                    .quests
                    .get(&q.next_in_chain)
                    .map(|n| can_take(p, n, &data))
            })
            .unwrap_or(false)
    {
        details(map, player, giver, q.next_in_chain);
    }
}

/// Take `count` of `item` from wherever the player carries it.
fn take_items(map: &mut Map, player: Guid, item: u32, mut count: u32) {
    while count > 0 {
        let Some(p) = map.players.get(&player) else {
            return;
        };
        let Some(i) = p.items.iter().position(|it| it.entry == item) else {
            return;
        };
        let n = p.items[i].count().min(count);
        inventory::take_item(map, player, i, n);
        count -= n;
    }
}

/// `CMSG_QUESTLOG_REMOVE_QUEST`: abandoned.
pub fn abandon(map: &mut Map, player: Guid, slot: u8) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    if usize::from(slot) < p.journal.log.len() {
        p.journal.log.remove(usize::from(slot));
        refresh_log(p, &data);
    }
}

/// `CMSG_QUEST_QUERY`: a quest's text and objectives, for the log.
pub fn query(map: &Map, player: Guid, quest: u32) {
    let Some(q) = map.realm.data.npc.quests.get(&quest) else {
        return;
    };
    let reward = |(item, n): (u32, u32)| QuestItemReward {
        item,
        item_count: n,
    };
    let Some(p) = map.players.get(&player) else {
        return;
    };
    p.outbound.send(&SMSG_QUEST_QUERY_RESPONSE {
        quest_id: quest,
        quest_method: q.method,
        quest_level: Level::new(q.level.max(0) as u8),
        zone_or_sort: q.zone_or_sort as u32,
        quest_type: q.kind,
        reputation_objective_faction: Default::default(),
        reputation_objective_value: 0,
        required_opposite_faction: Default::default(),
        required_opposite_reputation_value: 0,
        next_quest_in_chain: q.next_in_chain,
        money_reward: (q.reward_money.max(0) as u32).into(),
        max_level_money_reward: q.reward_money_max_level.into(),
        reward_spell: q.reward_spell,
        source_item_id: q.source_item,
        quest_flags: q.flags,
        rewards: q.rewards.map(reward),
        choice_rewards: q.choice_rewards.map(reward),
        point_map_id: q.point.0,
        position: Vector2d {
            x: q.point.1,
            y: q.point.2,
        },
        point_opt: q.point.3,
        title: q.title.clone(),
        objective_text: q.objectives.clone(),
        details: q.details.clone(),
        end_text: q.end_text.clone(),
        objectives: std::array::from_fn(|i| QuestObjective {
            creature_id: q.required_kills[i].0 as u32,
            kill_count: q.required_kills[i].1,
            required_item_id: q.required_items[i].0,
            required_item_count: q.required_items[i].1,
        }),
        objective_texts: q.objective_texts.clone(),
    });
}
