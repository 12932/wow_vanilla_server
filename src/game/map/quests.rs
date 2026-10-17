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
    if q.needs_event() && !l.explored {
        return false;
    }
    if q.rep_objective.0 != 0 && !l.rep_met {
        return false;
    }
    true
}

/// Whether `p` may take `q` (Thistle Tea's `QuestRequirements`, after
/// mangos' `CanTakeQuest`): not on it or done with it, no second timed
/// quest, the race, class, skill and standings, what it depends on, and
/// the level.
pub fn can_take(p: &Player, q: &Quest, data: &GameData) -> bool {
    let j = &p.journal;
    if j.has(q.id) || rewarded(p, data, q.id) {
        return false;
    }
    if q.limit_time > 0 && j.log.iter().any(|l| l.expires != 0 && !l.failed) {
        return false;
    }
    if q.required_races != 0 && q.required_races & (1 << (p.row.race - 1)) == 0 {
        return false;
    }
    if q.required_classes != 0 && q.required_classes & (1 << (p.row.class - 1)) == 0 {
        return false;
    }
    if q.required_skill != 0
        && crate::game::map::skills::value_with_bonuses(p, q.required_skill)
            < q.required_skill_value
    {
        return false;
    }
    // A standing at least, and below.
    let (min_f, min_v) = q.required_min_rep;
    if min_f != 0 && crate::game::map::reputation::standing(p, data, min_f) < min_v {
        return false;
    }
    let (max_f, max_v) = q.required_max_rep;
    if max_f != 0 && crate::game::map::reputation::standing(p, data, max_f) >= max_v {
        return false;
    }
    if !dependencies_met(p, q, data) {
        return false;
    }
    p.level() >= q.min_level
}

/// Rewarded, and not to be done again.
fn rewarded(p: &Player, data: &GameData, quest: u32) -> bool {
    p.journal.rewarded.contains(&quest)
        && data
            .npc
            .quests
            .get(&quest)
            .is_none_or(|q| q.special_flags & QUEST_SPECIAL_REPEATABLE == 0)
}

/// On it, or done with it for good.
fn taken(p: &Player, data: &GameData, quest: u32) -> bool {
    p.journal.has(quest) || rewarded(p, data, quest)
}

/// What `q` depends on: its prerequisites met; none of its exclusive group,
/// nor its chain's next, taken; its chain's previous, and any breadcrumb to
/// it, not on; and, a breadcrumb, the quests it leads to still to be had.
fn dependencies_met(p: &Player, q: &Quest, data: &GameData) -> bool {
    let g = data.npc.quest_graph();
    let j = &p.journal;
    if g.invalid.contains(&q.id) {
        return false;
    }
    if let Some(pre) = g.prerequisites.get(&q.id) {
        let state = |quest: u32, current: bool| {
            if current {
                j.has(quest)
            } else {
                j.rewarded.contains(&quest)
            }
        };
        // The first met decides, with all of its group.
        let met = pre
            .iter()
            .find(|x| state(x.quest, x.current))
            .is_some_and(|x| x.group.iter().all(|o| state(*o, x.current)));
        if !met {
            return false;
        }
    }
    if g.exclusive
        .get(&q.id)
        .is_some_and(|o| o.iter().any(|x| taken(p, data, *x)))
    {
        return false;
    }
    if q.next_in_chain != 0 && taken(p, data, q.next_in_chain) {
        return false;
    }
    let on = |v: Option<&Vec<u32>>| v.is_some_and(|v| v.iter().any(|x| j.has(*x)));
    if on(g.previous_chain.get(&q.id)) || on(g.breadcrumbs_to.get(&q.id)) {
        return false;
    }
    g.breadcrumb_targets.get(&q.id).is_none_or(|targets| {
        targets.iter().all(|t| {
            data.npc
                .quests
                .get(t)
                .is_some_and(|tq| can_take(p, tq, data))
        })
    })
}

/// Whether `p` is a few levels short of `q`: shown as unavailable.
fn soon(p: &Player, q: &Quest) -> bool {
    p.level() < q.min_level && p.level() + 4 >= q.min_level
}

/// The quests `giver` (a creature or object) starts and ends; an event's
/// quests are started only while it runs.
fn relations(map: &Map, giver: Guid) -> (Vec<u32>, Vec<u32>) {
    let npc = &map.realm.data.npc;
    let entry = guid::entry(giver);
    let (mut starts, ends) = if guid::is_unit(giver) {
        (
            npc.creature_starts.get(&entry).cloned().unwrap_or_default(),
            npc.creature_ends.get(&entry).cloned().unwrap_or_default(),
        )
    } else {
        (
            npc.object_starts.get(&entry).cloned().unwrap_or_default(),
            npc.object_ends.get(&entry).cloned().unwrap_or_default(),
        )
    };
    starts.retain(|q| crate::game::events::quest_offered(&map.realm, *q));
    (starts, ends)
}

/// The icon a quest giver shows `player`.
/// Whether `player` meets a quest's `RequiredCondition`, with `giver` as
/// the source.
pub fn condition_ok(map: &Map, player: Guid, giver: Guid, quest: u32) -> bool {
    match map.realm.data.scripts.quest_condition.get(&quest) {
        Some(c) => crate::game::map::conditions::met(map, *c, Some(giver), Some(player)),
        None => true,
    }
}

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
        if can_take(p, q, data) && condition_ok(map, player, giver, q.id) {
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
        if can_take(p, q, data)
            && condition_ok(map, player, giver, q.id)
            && !out.iter().any(|o| o.quest_id == q.id)
        {
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
    // An object: within reach of its model (Thistle Tea's `QuestGiver`).
    if guid::is_game_object(giver) {
        return crate::game::map::objects::within_reach(map, player, giver);
    }
    crate::game::map::npc::within_reach(map, player, giver)
}

/// Whether `giver`, beside `player`, takes `quest` back (mangos'
/// `HasInvolvedQuest`).
fn ends_here(map: &Map, player: Guid, giver: Guid, quest: u32) -> bool {
    relations(map, giver).1.contains(&quest) && near_giver(map, player, giver)
}

/// `GAMEOBJECT_DYN_FLAGS`: usable for a quest.
const GO_DYNFLAG_ACTIVATE: u32 = 0x1;
const GO_QUESTGIVER: u8 = 2;
const GO_GOOBER: u8 = 10;

/// Whether a quest object is usable to `player`: a quest giver with
/// something for them, a goober for a quest of theirs under way, or for
/// anyone (Thistle Tea's `QuestGivers`, mangos' `ActivateToQuest`).
fn activates(map: &Map, player: Guid, object: Guid, kind: u8, data: &[u32; 24]) -> bool {
    if kind == GO_QUESTGIVER {
        return !giver_quests(map, player, object).is_empty();
    }
    let Some(p) = map.players.get(&player) else {
        return false;
    };
    let quests = &map.realm.data.npc.quests;
    let under_way = |l: &Logged| quests.get(&l.quest).is_some_and(|q| !complete(p, q, l));
    let entry = guid::entry(object) as i32;
    data[1] as i32 == -1
        || p.journal.log.iter().any(|l| {
            under_way(l)
                && (l.quest == data[1]
                    || quests.get(&l.quest).is_some_and(|q| {
                        q.required_kills
                            .iter()
                            .enumerate()
                            .any(|(i, (id, n))| *id == -entry && l.kills[i] < *n)
                    }))
        })
}

/// Every few updates: each player's quest objects shown as usable to them
/// or not, as their quests stand.
pub fn update_objects(map: &mut Map) {
    if !map.stats.updates.is_multiple_of(10) {
        return;
    }
    let data = map.realm.data.clone();
    let players: Vec<Guid> = map.players.keys().copied().collect();
    for g in players {
        let known: Vec<Guid> = map.players[&g]
            .known
            .iter()
            .copied()
            .filter(|k| guid::is_game_object(*k))
            .collect();
        let mut changed = Vec::new();
        for o in known {
            let Some(t) = map
                .gameobject(o)
                .and_then(|x| data.gameobject_templates.get(&x.entry))
                .filter(|t| matches!(t.kind, GO_QUESTGIVER | GO_GOOBER))
            else {
                continue;
            };
            let on = activates(map, g, o, t.kind, &t.data);
            if on != map.players[&g].quest_objects.contains(&o) {
                changed.push((o, on));
            }
        }
        let Some(p) = map.players.get_mut(&g) else {
            continue;
        };
        p.quest_objects.retain(|o| p.known.contains(o));
        for (o, on) in changed {
            let flag = if on { GO_DYNFLAG_ACTIVATE } else { 0 };
            let mut f = Fields::new(TypeId::GameObject, o, guid::entry(o));
            f.set_u32(GAMEOBJECT_DYN_FLAGS, flag ^ GO_DYNFLAG_ACTIVATE);
            f.clear_dirty();
            f.set_u32(GAMEOBJECT_DYN_FLAGS, flag);
            if on {
                p.quest_objects.insert(o);
            } else {
                p.quest_objects.remove(&o);
            }
            if let Some(block) = crate::game::update::values_block(o, &f, Visibility::Public) {
                map.batches.entry(g).or_default().push(&block);
            }
        }
    }
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
    crate::game::map::creature_motion::talk_pause(map, giver);
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

/// `CMSG_GAMEOBJ_USE` on a quest giver (a present under the tree, a wanted
/// poster): its menu with its quests, or just its quests (mangos'
/// `PrepareGossipMenu`, then `SendPreparedGossip`).
pub fn object_hello(map: &mut Map, player: Guid, object: Guid) {
    let menu = map
        .gameobject(object)
        .and_then(|o| map.realm.data.gameobject_templates.get(&o.entry))
        .map_or(0, |t| t.data[3]);
    let menu = if map.realm.data.npc.gossip_menus.contains_key(&menu) {
        menu
    } else {
        0
    };
    if menu == 0 && !giver_quests(map, player, object).is_empty() {
        return hello(map, player, object);
    }
    if near_giver(map, player, object) {
        crate::game::map::npc::show_menu(map, player, object, menu, 0);
    }
}

/// `SMSG_QUESTGIVER_QUEST_DETAILS`'s opcode.
pub const SMSG_QUESTGIVER_QUEST_DETAILS_OPCODE: u16 = 0x188;

/// A quest's details as 1.12 lays them out (mangos-zero's, vmangos's and
/// Thistle Tea's): its Accept button on, each reward item with its display
/// id (which wow_messages' layout leaves out), the money it gives or asks
/// for, and its emotes up to the last.
pub fn details_frame(data: &GameData, giver: Guid, q: &Quest) -> std::sync::Arc<[u8]> {
    let mut b = crate::game::data::items::Body::default();
    b.u64(giver.guid())
        .u32(q.id)
        .cstr(&q.title)
        .cstr(&q.details)
        .cstr(&q.objectives)
        .u32(1);
    if q.flags & QUEST_FLAGS_HIDDEN_REWARDS != 0 {
        b.u32(0).u32(0).u32(0);
    } else {
        for list in [&q.choice_rewards[..], &q.rewards[..]] {
            let items: Vec<(u32, u32)> = list.iter().copied().filter(|(i, _)| *i != 0).collect();
            b.u32(items.len() as u32);
            for (item, n) in items {
                let display = data.items.get(&item).map_or(0, |t| t.display_id);
                b.u32(item).u32(n).u32(display);
            }
        }
        b.i32(q.reward_money);
    }
    b.u32(q.reward_spell);
    let emotes = q
        .details_emotes
        .iter()
        .rposition(|(e, _)| *e != 0)
        .map_or(&[][..], |last| &q.details_emotes[..=last]);
    b.u32(emotes.len() as u32);
    for (emote, delay) in emotes {
        b.u32(*emote).u32(*delay);
    }
    b.frame(SMSG_QUESTGIVER_QUEST_DETAILS_OPCODE)
}

/// `CMSG_QUESTGIVER_QUERY_QUEST`, or a hello with one quest: its details.
pub fn details(map: &mut Map, player: Guid, giver: Guid, quest: u32) {
    let data = map.realm.data.clone();
    let Some(q) = data.npc.quests.get(&quest) else {
        return;
    };
    if let Some(p) = map.players.get(&player) {
        p.outbound.send_frame(details_frame(&data, giver, q));
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
    let condition = condition_ok(map, player, giver, quest);
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    if p.journal.has(quest) {
        return invalid(p, QuestFailedReason::QuestAlreadyOn);
    }
    if p.level() < q.min_level {
        return invalid(p, QuestFailedReason::QuestFailedLowLevel);
    }
    if !can_take(p, q, &data) || !condition {
        return invalid(p, QuestFailedReason::QuestFailedReqs);
    }
    if p.journal.log.len() >= QUEST_LOG_SIZE {
        p.outbound.send(&SMSG_QUESTLOG_FULL {});
        return;
    }
    p.journal.log.push(Logged {
        quest,
        expires: if q.limit_time > 0 {
            crate::game::social::mail::now() + i64::from(q.limit_time)
        } else {
            0
        },
        ..Default::default()
    });
    refresh_log(p, &data);
    if q.source_item != 0 {
        let _ = inventory::add_item(map, player, q.source_item, q.source_item_count.max(1), true);
    }
    check(map, player, quest);
    crate::game::map::questshare::accepted(map, player, giver, quest);
    crate::game::map::spell_areas::reconcile(map, player);
    if let Some(script) = data.scripts.quest_start_script.get(&quest).copied() {
        crate::game::map::scripts::start(
            map,
            crate::game::data::scripts::ScriptTable::QuestStart,
            script,
            giver,
            player,
        );
    }
    crate::game::map::escorts::accepted(map, player, quest);
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
        p.fields.set_u32(base + 2, l.expires.max(0) as u32);
    }
}

/// Timed quests whose time is up fail.
pub fn update_timers(map: &mut Map) {
    let now = crate::game::social::mail::now();
    let data = map.realm.data.clone();
    for p in map.players.values_mut() {
        let failed: Vec<u32> = p
            .journal
            .log
            .iter_mut()
            .filter(|l| l.expires != 0 && l.expires <= now && !l.failed)
            .map(|l| {
                l.failed = true;
                l.quest
            })
            .collect();
        if failed.is_empty() {
            continue;
        }
        for quest in failed {
            p.outbound
                .send(&SMSG_QUESTUPDATE_FAILEDTIMER { quest_id: quest });
        }
        refresh_log(p, &data);
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
    if q.rep_objective.0 != 0 {
        let standing = crate::game::map::reputation::standing(p, &data, q.rep_objective.0);
        p.journal.log[slot].rep_met = standing >= q.rep_objective.1;
    }
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
/// credited to, for the objectives that ask for no spell (Thistle Tea's
/// `increment_kill`); in a raid, only for raid quests.
pub fn killed(map: &mut Map, player: Guid, creature: Guid) {
    let raid = {
        let groups = map.realm.social.groups.lock();
        groups.group_of(player).is_some_and(|g| g.raid)
    };
    let allowed = |q: &Quest| !raid || q.kind == QUEST_TYPE_RAID || q.flags & QUEST_FLAGS_RAID != 0;
    credit(map, player, creature, 0, &allowed);
}

/// The entry a creature or object is now (a script may have changed
/// it, as a cured beast's), not the one its GUID was made with.
pub fn entry_of(map: &Map, g: Guid) -> u32 {
    map.creature(g)
        .map(|c| c.entry)
        .or_else(|| map.gameobject(g).map(|o| o.entry))
        .unwrap_or_else(|| guid::entry(g))
}

/// A raid quest, by its type or its flags: killing counts in a raid.
const QUEST_TYPE_RAID: u32 = 62;
const QUEST_FLAGS_RAID: u32 = 0x40;

/// A finished cast: the quests asking for its spell cast at the
/// creatures and objects it hit (Thistle Tea's `credit_cast`).
pub fn cast_credit(map: &mut Map, player: Guid, targets: &[Guid], spell: u32) {
    if spell == 0 || !map.players.contains_key(&player) {
        return;
    }
    let mut seen = Vec::new();
    for t in targets {
        if !seen.contains(t) && (guid::is_unit(*t) || guid::is_game_object(*t)) {
            seen.push(*t);
            credit(map, player, *t, spell, &|_| true);
        }
    }
}

/// A game object used: credit for the quests that ask for its use (a
/// negative objective, with no spell); a quest object's goes to the group
/// near it too, for their shareable quests (Thistle Tea's
/// `credit_game_object_use`).
pub fn object_used(map: &mut Map, player: Guid, object: Guid, group: bool) {
    credit_object(map, player, object, 0, false);
    if !group {
        return;
    }
    let members: Vec<Guid> = {
        let groups = map.realm.social.groups.lock();
        groups
            .group_of(player)
            .map(|g| g.members.iter().map(|m| m.guid).collect())
            .unwrap_or_default()
    };
    let Some(at) = map.position_of(object) else {
        return;
    };
    for m in members.into_iter().filter(|m| *m != player) {
        let near = map.players.get(&m).is_some_and(|p| {
            !matches!(p.death, crate::game::map::death::Death::Ghost { .. })
                && crate::game::map::combat::dist(p.position(), at)
                    <= crate::game::map::combat::GROUP_REWARD_YARDS
        });
        if near {
            credit_object(map, m, object, 0, true);
        }
    }
}

/// A spell cast at a game object: the quests asking for it cast there.
pub fn cast_on_object(map: &mut Map, player: Guid, object: Guid, spell: u32) {
    if spell != 0 {
        credit(map, player, object, spell, &|_| true);
    }
}

fn credit_object(map: &mut Map, player: Guid, object: Guid, spell: u32, shareable: bool) {
    let share =
        |q: &Quest| !shareable || q.flags & crate::game::map::questshare::QUEST_FLAGS_SHARABLE != 0;
    credit(map, player, object, spell, &share);
}

/// The first objective of each of the player's quests under way (that
/// `eligible` lets count) asking for `target`, a creature (a positive id)
/// or an object (a negative one), by `spell` (0: killed or used), a step
/// on (Thistle Tea's `increment_entity_objective`).
fn credit(
    map: &mut Map,
    player: Guid,
    target: Guid,
    spell: u32,
    eligible: &dyn Fn(&Quest) -> bool,
) {
    let entry = entry_of(map, target);
    let object = guid::is_game_object(target);
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let mut touched = Vec::new();
    for i in 0..p.journal.log.len() {
        let Some(q) = data.npc.quests.get(&p.journal.log[i].quest) else {
            continue;
        };
        if !eligible(q) || complete(p, q, &p.journal.log[i]) {
            continue;
        }
        let Some(k) = q
            .required_kills
            .iter()
            .enumerate()
            .position(|(k, (id, _))| {
                let named = if object {
                    *id < 0 && id.unsigned_abs() == entry
                } else {
                    *id > 0 && *id as u32 == entry
                };
                named && q.required_spell_casts[k] == spell
            })
        else {
            continue;
        };
        let l = &mut p.journal.log[i];
        let required = q.required_kills[k].1;
        if l.kills[k] >= required {
            continue;
        }
        l.kills[k] += 1;
        p.outbound.send(&SMSG_QUESTUPDATE_ADD_KILL {
            quest_id: q.id,
            creature_id: if object { entry | 0x8000_0000 } else { entry },
            kill_count: l.kills[k],
            required_kill_count: required,
            guid: target,
        });
        touched.push(q.id);
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
    if !ends_here(map, player, giver, quest) {
        return;
    }
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
    if !ends_here(map, player, giver, quest) {
        return;
    }
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
    if !ends_here(map, player, giver, quest) {
        return;
    }
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
    // The spell the quest casts on its rewarded (honor, a buff, an item,
    // a lesson), else the one it shows: Thistle Tea's and vmangos' rule.
    let cast = if q.reward_spell_cast != 0 {
        q.reward_spell_cast
    } else {
        q.reward_spell
    };
    if cast != 0 {
        crate::game::map::spells::cast_triggered(map, player, cast, Some(player));
    }
    crate::game::map::reputation::quest_rewarded(map, player, &q);
    crate::game::map::battleground::quest_rewarded(map, player, quest);
    crate::game::social::mail::quest_mail(&map.realm, player, giver, quest);
    crate::game::map::spell_areas::reconcile(map, player);
    if let Some(script) = data.scripts.quest_complete_script.get(&quest).copied() {
        crate::game::map::scripts::start(
            map,
            crate::game::data::scripts::ScriptTable::QuestEnd,
            script,
            giver,
            player,
        );
    }
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

/// `SMSG_QUEST_QUERY_RESPONSE`'s opcode.
pub const SMSG_QUEST_QUERY_RESPONSE_OPCODE: u16 = 0x5D;
/// `QuestFlags`: the rewards aren't shown.
const QUEST_FLAGS_HIDDEN_REWARDS: u32 = 0x200;

/// `SMSG_QUEST_QUERY_RESPONSE` in 1.12's layout, as Thistle Tea and
/// mangos-zero send it: wow_messages gives the reputation objective's
/// faction 16 bits where 1.12 has 32, which moves everything after it.
pub fn query_response(q: &Quest) -> std::sync::Arc<[u8]> {
    let mut b = crate::game::data::items::Body::default();
    let hidden = q.flags & QUEST_FLAGS_HIDDEN_REWARDS != 0;
    b.u32(q.id)
        .u32(q.method)
        .u32(q.level.max(0) as u32)
        .i32(q.zone_or_sort)
        .u32(q.kind)
        .u32(q.rep_objective.0)
        .i32(q.rep_objective.1)
        .u32(0)
        .u32(0)
        .u32(q.next_in_chain)
        .i32(if hidden { 0 } else { q.reward_money })
        .u32(q.reward_money_max_level)
        .u32(q.reward_spell)
        .u32(q.source_item)
        .u32(q.flags);
    for (item, n) in q.rewards.iter().chain(q.choice_rewards.iter()) {
        let (item, n) = if hidden { (0, 0) } else { (*item, *n) };
        b.u32(item).u32(n);
    }
    b.u32(q.point.0).f32(q.point.1).f32(q.point.2).u32(q.point.3);
    b.cstr(&q.title)
        .cstr(&q.objectives)
        .cstr(&q.details)
        .cstr(&q.end_text);
    for i in 0..4 {
        let (id, count) = q.required_kills[i];
        // An object to use is sent with the top bit set.
        let id = if id < 0 {
            id.unsigned_abs() | 0x8000_0000
        } else {
            id as u32
        };
        b.u32(id)
            .u32(count)
            .u32(q.required_items[i].0)
            .u32(q.required_items[i].1);
    }
    for text in &q.objective_texts {
        b.cstr(text);
    }
    b.frame(SMSG_QUEST_QUERY_RESPONSE_OPCODE)
}

/// `CMSG_QUEST_QUERY`: a quest's text and objectives, for the log.
pub fn query(map: &Map, player: Guid, quest: u32) {
    let Some(q) = map.realm.data.npc.quests.get(&quest) else {
        return;
    };
    if let Some(p) = map.players.get(&player) {
        p.outbound.send_frame(query_response(q));
    }
}
