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

/// A cleared slot of the log is a `Logged` for quest 0: the quests after it
/// keep their slots, and the next quest taken goes in the lowest free one
/// (mangos's `FindQuestSlot(0)`, Thistle Tea's tombstones).
impl Journal {
    pub fn slot_of(&self, quest: u32) -> Option<usize> {
        if quest == 0 {
            return None;
        }
        self.log.iter().position(|l| l.quest == quest)
    }

    pub fn has(&self, quest: u32) -> bool {
        self.slot_of(quest).is_some()
    }

    /// The quests in the log.
    pub fn count(&self) -> usize {
        self.log.iter().filter(|l| l.quest != 0).count()
    }

    pub fn full(&self) -> bool {
        self.count() >= QUEST_LOG_SIZE
    }

    /// Into the lowest free slot: which, or none for a full log.
    pub fn add(&mut self, logged: Logged) -> Option<usize> {
        if let Some(slot) = self.log.iter().position(|l| l.quest == 0) {
            self.log[slot] = logged;
            return Some(slot);
        }
        if self.log.len() >= QUEST_LOG_SIZE {
            return None;
        }
        self.log.push(logged);
        Some(self.log.len() - 1)
    }

    /// Clear a slot, leaving the others where they are.
    pub fn clear(&mut self, slot: usize) {
        if let Some(l) = self.log.get_mut(slot) {
            *l = Logged::default();
        }
        self.trim();
    }

    /// Two slots of the log trade places, an empty one too; slots past the
    /// log's twenty are ignored (mangos's `HandleQuestLogSwapQuest`).
    pub fn swap(&mut self, a: usize, b: usize) {
        if a >= QUEST_LOG_SIZE || b >= QUEST_LOG_SIZE || a == b {
            return;
        }
        let len = self.log.len().max(a + 1).max(b + 1);
        self.log.resize(len, Logged::default());
        self.log.swap(a, b);
        self.trim();
    }

    fn trim(&mut self) {
        while self.log.last().is_some_and(|l| l.quest == 0) {
            self.log.pop();
        }
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
    can_take_but_level(p, q, data) && p.level() >= q.min_level
}

/// [`can_take`] but for the level, which Thistle Tea checks last.
fn can_take_but_level(p: &Player, q: &Quest, data: &GameData) -> bool {
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
    dependencies_met(p, q, data)
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

/// A quest under way: in the log, not failed, and not done and rewarded
/// before (mangos's `IsCurrentQuest`, Thistle Tea's `current?`).
fn current_quest(p: &Player, data: &GameData, quest: u32) -> bool {
    let Some(slot) = p.journal.slot_of(quest) else {
        return false;
    };
    let l = &p.journal.log[slot];
    !l.failed
        && !(p.journal.rewarded.contains(&quest)
            && data
                .npc
                .quests
                .get(&quest)
                .is_some_and(|q| complete(p, q, l)))
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
                current_quest(p, data, quest)
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

/// Whether `p` could take `q` but for being a few levels short: shown as
/// unavailable (Thistle Tea's `low_level`, its last check; mangos's
/// `CanSeeStartQuest`, within its `HighLevelHideDiff`).
fn soon(p: &Player, q: &Quest, data: &GameData) -> bool {
    p.level() < q.min_level && p.level() + 4 >= q.min_level && can_take_but_level(p, q, data)
}

/// The quests `giver` (a creature or object) starts and ends; an event's
/// quests are started and ended only while it runs, though they stay in
/// the log (mangos's `SetQuestActiveState`, Thistle Tea's
/// `event_active?`).
fn relations(map: &Map, giver: Guid) -> (Vec<u32>, Vec<u32>) {
    let npc = &map.realm.data.npc;
    let entry = guid::entry(giver);
    let (mut starts, mut ends) = if guid::is_unit(giver) {
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
    ends.retain(|q| crate::game::events::quest_offered(&map.realm, *q));
    (starts, ends)
}

/// Whether `player` meets a quest's `RequiredCondition`, with `giver` as
/// the source; a breadcrumb, its targets' too, and their events running
/// (Thistle Tea's `breadcrumb_conditions_met?`, and its `base_can_take`
/// of each target, which wants the target's event).
pub fn condition_ok(map: &Map, player: Guid, giver: Guid, quest: u32) -> bool {
    let data = &map.realm.data;
    let own = |quest: u32| match data.scripts.quest_condition.get(&quest) {
        Some(c) => crate::game::map::conditions::met(map, *c, Some(giver), Some(player)),
        None => true,
    };
    own(quest)
        && data
            .npc
            .quest_graph()
            .breadcrumb_targets
            .get(&quest)
            .is_none_or(|targets| {
                targets
                    .iter()
                    .all(|t| own(*t) && crate::game::events::quest_offered(&map.realm, *t))
            })
}

/// Where `p` is with a quest an ender takes back: done, under way, or
/// neither (not in the log, or failed, which an ender doesn't show).
fn turn_in_state(p: &Player, q: &Quest) -> Option<bool> {
    let l = &p.journal.log[p.journal.slot_of(q.id)?];
    (!l.failed).then(|| complete(p, q, l))
}

/// Whether an auto-complete quest is to be had from `giver`: one `player`
/// could take, with nothing to accept.
fn auto_offered(map: &Map, player: Guid, giver: Guid, q: &Quest) -> bool {
    q.auto_complete
        && map.players.get(&player).is_some_and(|p| {
            can_take(p, q, &map.realm.data) && condition_ok(map, player, giver, q.id)
        })
}

/// Whether `player` has `q` to turn in at `giver`, and whether it is
/// done: in the log, or an auto-complete quest they could take, done when
/// they carry what it asks for (mangos's `CanCompleteQuest`).
fn turn_in(map: &Map, player: Guid, giver: Guid, q: &Quest) -> Option<bool> {
    let p = map.players.get(&player)?;
    if let Some(slot) = p.journal.slot_of(q.id) {
        return Some(complete(p, q, &p.journal.log[slot]));
    }
    auto_offered(map, player, giver, q).then(|| delivered(p, q))
}

/// Whether `p` has what `q` asks for at its turn-in: its items, and the
/// money it costs (Thistle Tea's `QuestRequirements.delivered?`).
fn delivered(p: &Player, q: &Quest) -> bool {
    q.required_items
        .iter()
        .all(|(i, n)| *i == 0 || p.count_of(*i) >= *n)
        && p.money() >= q.reward_money.min(0).unsigned_abs()
}

/// The icon a quest giver shows `player`: the best of a quest done (a
/// yellow ?), one to take (a yellow !), one under way (a grey ?), one a
/// few levels off (a grey !), as mangos's `GetDialogStatus` and Thistle
/// Tea's `QuestDialogStatus.for_npc` rank them. An auto-complete quest
/// shows as done to anyone who could take it, blue when it repeats.
pub fn status(map: &Map, player: Guid, giver: Guid) -> QuestGiverStatus {
    let Some(p) = map.players.get(&player) else {
        return QuestGiverStatus::None;
    };
    let data = &map.realm.data;
    let (starts, ends) = relations(map, giver);
    let mut best = QuestGiverStatus::None;
    let mut consider = |s: QuestGiverStatus| {
        if s.as_int() > best.as_int() {
            best = s;
        }
    };
    for q in ends.iter().filter_map(|id| data.npc.quests.get(id)) {
        match turn_in_state(p, q) {
            Some(true) => consider(QuestGiverStatus::Reward2),
            Some(false) => consider(QuestGiverStatus::Incomplete),
            None => {}
        }
    }
    for q in starts.iter().filter_map(|id| data.npc.quests.get(id)) {
        if !condition_ok(map, player, giver, q.id) {
            continue;
        }
        if can_take(p, q, data) {
            consider(if !q.auto_complete {
                QuestGiverStatus::Available
            } else if q.special_flags & QUEST_SPECIAL_REPEATABLE != 0 {
                QuestGiverStatus::RewardRep
            } else {
                QuestGiverStatus::Reward2
            });
        } else if soon(p, q, data) {
            consider(QuestGiverStatus::Unavailable);
        }
    }
    best
}

/// The quests a giver lists for `player`, for a gossip menu or a quest
/// list, with the icons mangos's `PrepareQuestMenu` gives them: its quests
/// done (4) and under way (3) first, then those to take (5), and those
/// with nothing to accept as ready to turn in (4). The client sends for a
/// quest's request or its details by them.
pub fn giver_quests(map: &Map, player: Guid, giver: Guid) -> Vec<QuestItem> {
    let Some(p) = map.players.get(&player) else {
        return Vec::new();
    };
    let data = &map.realm.data;
    // A creature lists quests only while it is a quest giver (mangos's
    // `PrepareGossipMenu`): an escort under way isn't.
    if guid::is_unit(giver)
        && map.creature(giver).is_some_and(|c| {
            c.fields.get_u32(UNIT_NPC_FLAGS) & crate::game::map::npc::NPC_FLAG_QUESTGIVER == 0
        })
    {
        return Vec::new();
    }
    let (starts, ends) = relations(map, giver);
    let item = |q: &Quest, icon: u32| QuestItem {
        quest_id: q.id,
        quest_icon: icon,
        level: Level::new(q.level.max(0) as u8),
        title: q.title.clone(),
    };
    let mut out = Vec::new();
    for q in ends.iter().filter_map(|id| data.npc.quests.get(id)) {
        if let Some(done) = turn_in_state(p, q) {
            out.push(item(q, if done { 4 } else { 3 }));
        }
    }
    for q in starts.iter().filter_map(|id| data.npc.quests.get(id)) {
        if can_take(p, q, data)
            && condition_ok(map, player, giver, q.id)
            && !out.iter().any(|o| o.quest_id == q.id)
        {
            out.push(item(q, if q.auto_complete { 4 } else { 5 }));
        }
    }
    out
}

/// Whether `player` may deal with `giver` over a quest: a living player
/// within reach of a living quest giver, not hostile, not in flight nor
/// held (mangos's `CanInteractWithQuestGiver`, Thistle Tea's checks on
/// accepting and turning in); an object, within reach of its model
/// (Thistle Tea's `QuestGiver`).
fn near_giver(map: &Map, player: Guid, giver: Guid) -> bool {
    if guid::is_game_object(giver) {
        return alive(map, player) && crate::game::map::objects::within_reach(map, player, giver);
    }
    crate::game::map::npc::npc_entry(
        map,
        player,
        giver,
        crate::game::map::npc::NPC_FLAG_QUESTGIVER,
    )
    .is_some()
}

fn alive(map: &Map, player: Guid) -> bool {
    map.players
        .get(&player)
        .is_some_and(|p| p.death == crate::game::map::death::Death::Alive)
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
                map.batches.entry(g).or_default().push_for(o, &block);
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
        // One to turn in, or to take (mangos's `SendPreparedQuest`).
        let q = quests[0].quest_id;
        if quests[0].quest_icon != 5 {
            return complete_quest(map, player, giver, q);
        }
        return details(map, player, giver, q);
    }
    let (title, emote_delay, emote) = greeting(map, player, giver);
    if let Some(p) = map.players.get(&player) {
        p.outbound.send(&SMSG_QUESTGIVER_QUEST_LIST {
            npc: giver,
            title,
            emote_delay,
            emote,
            quest_items: quests,
        });
    }
}

/// A quest list's greeting, its emote's delay and the emote: the giver's
/// quest greeting, else its gossip menu's text but the default one (mangos's
/// `SendPreparedQuest`, Thistle Tea's `greeting`), else nothing.
fn greeting(map: &Map, player: Guid, giver: Guid) -> (String, u32, u32) {
    let npc = &map.realm.data.npc;
    if let Some(text) = npc.quest_greetings.get(&guid::entry(giver)) {
        return (text.clone(), 0, 0);
    }
    let Some(c) = map.creature(giver) else {
        return Default::default();
    };
    let menu = crate::game::map::event_ai::scripted(map, giver)
        .and_then(|s| s.gossip_menu)
        .or_else(|| {
            map.realm
                .data
                .creature_template(c.entry)
                .map(|t| t.gossip_menu_id)
        })
        .unwrap_or(0);
    let text = crate::game::map::npc::greeting(map, menu, giver, player);
    if text == crate::game::map::npc::DEFAULT_TEXT {
        return Default::default();
    }
    npc.npc_texts
        .get(&text)
        .and_then(|choices| npc.broadcast_texts.get(&choices[0].broadcast))
        .map(|b| {
            let said = if b.male.is_empty() {
                &b.female
            } else {
                &b.male
            };
            (said.clone(), b.emotes[0].0, b.emotes[0].1)
        })
        .unwrap_or_default()
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

/// `CMSG_QUESTGIVER_QUERY_QUEST`: a quest's details, from an item of the
/// player's that starts it, or a giver that starts or ends it (mangos's
/// `HasQuest` or `HasInvolvedQuest`); else the dialog closes.
pub fn query_quest(map: &mut Map, player: Guid, giver: Guid, quest: u32) {
    let data = map.realm.data.clone();
    let known = if guid::high(giver) == Some(guid::HighGuid::Item) {
        map.players.get(&player).is_some_and(|p| {
            p.item_by_guid(giver)
                .and_then(|i| data.item(p.items[i].entry))
                .is_some_and(|t| t.start_quest == quest)
        })
    } else {
        let (starts, ends) = relations(map, giver);
        (starts.contains(&quest) || ends.contains(&quest)) && map.position_of(giver).is_some()
    };
    if !known {
        return cancel(map, player);
    }
    details(map, player, giver, quest);
}

/// `CMSG_QUESTGIVER_CANCEL`, or a dialog with nothing for the player: it
/// closes (mangos's `CloseGossip`).
pub fn cancel(map: &Map, player: Guid) {
    if let Some(p) = map.players.get(&player) {
        p.outbound.send(&SMSG_GOSSIP_COMPLETE {});
    }
}

/// A hello with one quest, a chain's next, a shared quest: its details.
pub fn details(map: &mut Map, player: Guid, giver: Guid, quest: u32) {
    let data = map.realm.data.clone();
    let Some(q) = data.npc.quests.get(&quest) else {
        return;
    };
    if let Some(p) = map.players.get(&player) {
        p.outbound.send_frame(details_frame(&data, giver, q));
    }
}

/// Why `p` can't take `q`, as mangos's `SendCanTakeQuestResponse` tells
/// it: the race, a second timed quest, else what it asks for.
fn refusal(p: &Player, q: &Quest) -> QuestFailedReason {
    if q.required_races != 0 && q.required_races & (1 << (p.row.race - 1)) == 0 {
        QuestFailedReason::QuestFailedWrongRace
    } else if q.limit_time > 0 && p.journal.log.iter().any(|l| l.expires != 0 && !l.failed) {
        QuestFailedReason::QuestOnlyOneTimed
    } else {
        QuestFailedReason::DontHaveReq
    }
}

fn invalid(p: &Player, msg: QuestFailedReason) {
    p.outbound.send(&SMSG_QUESTGIVER_QUEST_INVALID { msg });
}

/// `CMSG_QUESTGIVER_ACCEPT_QUEST`: into the log; taken or not, the dialog
/// closes after (mangos's `HandleQuestgiverAcceptQuestOpcode`, Thistle
/// Tea's).
pub fn accept(map: &mut Map, player: Guid, giver: Guid, quest: u32) {
    accept_with(map, player, giver, quest, true);
    cancel(map, player);
}

/// A quest every member must accept, taken by one asked to (mangos's
/// `AddQuest` with no giver): its start script has already run.
pub fn accept_asked(map: &mut Map, player: Guid, asker: Guid, quest: u32) {
    accept_with(map, player, asker, quest, false);
}

fn accept_with(map: &mut Map, player: Guid, giver: Guid, quest: u32, script: bool) {
    let data = map.realm.data.clone();
    let Some(q) = data.npc.quests.get(&quest) else {
        return;
    };
    let (starts, _) = relations(map, giver);
    // From an item: one the player has, that starts the quest (mangos's
    // `Item::HasQuest`, Thistle Tea's `QuestItems.starter`).
    let from_item = guid::high(giver) == Some(guid::HighGuid::Item);
    // From an item or a player, the player must be alive (mangos's
    // `CanInteractWithQuestGiver`); a giver's own checks are
    // `near_giver`'s.
    if !alive(map, player) {
        return;
    }
    let starter = map.players.get(&player).and_then(|p| {
        let i = p.item_by_guid(giver)?;
        (data.item(p.items[i].entry)?.start_quest == quest).then_some(p.items[i].entry)
    });
    if from_item && starter.is_none() {
        return;
    }
    let shared = guid::is_player(giver)
        && crate::game::map::questshare::shared_by(map, player, giver, quest);
    if !from_item && !shared && (!starts.contains(&quest) || !near_giver(map, player, giver)) {
        return;
    }
    // A shared quest: still near its sharer, and the sharer's time left.
    let shared_expiry = if shared {
        let Some(expires) = crate::game::map::questshare::admit(map, player, giver, quest) else {
            return;
        };
        Some(expires)
    } else {
        None
    };
    let condition = condition_ok(map, player, giver, quest);
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    if p.journal.has(quest) {
        return invalid(p, QuestFailedReason::QuestAlreadyOn);
    }
    if !can_take_but_level(p, q, &data) || !condition {
        return invalid(p, refusal(p, q));
    }
    if p.level() < q.min_level {
        return invalid(p, QuestFailedReason::QuestFailedLowLevel);
    }
    if p.journal.full() {
        p.outbound.send(&SMSG_QUESTLOG_FULL {});
        return;
    }
    // The items the quest gives, but those already carried or banked
    // (Thistle Tea's `missing_source_count`): with no room for them the
    // quest isn't taken; one that can't be carried again is had already
    // (mangos's `CanAddQuest`).
    let source = if q.source_item != 0 {
        q.source_item_count
            .max(1)
            .saturating_sub(p.count_of(q.source_item))
    } else {
        0
    };
    let source = match (source > 0).then(|| inventory::room_for(p, &data, q.source_item, source)) {
        Some(Err(inventory::Fail::CantCarryMore)) | None => 0,
        Some(Ok(())) => source,
        Some(Err(fail)) => {
            inventory::send_fail(p, fail, Guid::zero());
            // A shared offer not taken goes, the sharer told (Thistle
            // Tea's `QuestSharing.accept`).
            if shared {
                crate::game::map::questshare::refused(map, player, giver);
            }
            return;
        }
    };
    p.journal.add(Logged {
        quest,
        expires: match shared_expiry {
            Some(expires) if q.limit_time > 0 && expires != 0 => expires,
            _ if q.limit_time > 0 => crate::game::social::mail::now() + i64::from(q.limit_time),
            _ => 0,
        },
        ..Default::default()
    });
    refresh_log(p, &data);
    if source > 0 {
        let _ = inventory::add_item(map, player, q.source_item, source, true);
    }
    check(map, player, quest);
    crate::game::map::questshare::accepted(map, player, giver, quest);
    crate::game::map::spell_areas::reconcile(map, player);
    if let Some(script) = data
        .scripts
        .quest_start_script
        .get(&quest)
        .copied()
        .filter(|_| script)
    {
        crate::game::map::scripts::start(
            map,
            crate::game::data::scripts::ScriptTable::QuestStart,
            script,
            giver,
            player,
        );
    }
    crate::game::map::escorts::accepted(map, player, quest);
    // The item that started it goes, unless the quest gives it or asks for
    // it (mangos-zero's `AddQuest`, Thistle Tea's `QuestItems.acceptance`).
    if let Some(entry) = starter
        && q.source_item != entry
        && !q.required_items.iter().any(|(i, _)| *i == entry)
        && let Some((i, count)) = map
            .players
            .get(&player)
            .and_then(|p| p.item_by_guid(giver).map(|i| (i, p.items[i].count())))
    {
        inventory::take_item(map, player, i, count);
    }
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
        // Six bits a counter, a count past them shown as 63 (Thistle Tea's
        // `slot_binary`).
        let mut state = 0u32;
        for (i, k) in l.kills.iter().enumerate() {
            state |= (*k).min(0x3F) << (i * 6);
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
        // A failed quest's timer is 1, as mangos's `FailQuest` sets it.
        let timer = if l.failed { 1 } else { l.expires.max(0) as u32 };
        p.fields.set_u32(base + 2, timer);
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

/// A quest under way fails (an escort lost, a script's say): not one
/// that isn't in the log, has failed already, or is done, as mangos's
/// `FailQuestForGroup` and Thistle Tea's `QuestLog.fail` have it. A timed
/// quest says so with its timer. Whether it failed.
pub fn fail(map: &mut Map, player: Guid, quest: u32) -> bool {
    let data = map.realm.data.clone();
    let Some(q) = data.npc.quests.get(&quest) else {
        return false;
    };
    let Some(p) = map.players.get_mut(&player) else {
        return false;
    };
    let Some(slot) = p.journal.slot_of(quest) else {
        return false;
    };
    if p.journal.log[slot].failed || complete(p, q, &p.journal.log[slot]) {
        return false;
    }
    let l = &mut p.journal.log[slot];
    l.failed = true;
    let timed = l.expires != 0;
    refresh_log(p, &data);
    if timed {
        p.outbound
            .send(&SMSG_QUESTUPDATE_FAILEDTIMER { quest_id: quest });
    } else {
        p.outbound
            .send(&SMSG_QUESTUPDATE_FAILED { quest_id: quest });
    }
    true
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
    let raid = in_raid(map, player);
    let allowed = |q: &Quest| !raid || raid_quest(q);
    credit(map, player, creature, 0, &allowed);
}

/// A creature's gossip closed by an option: talking to it, credit for the
/// quests asking for it with no spell, as mangos's TalkedToCreature and
/// Thistle Tea's `credit_talk` give it; not those waiting on an event.
pub fn talked(map: &mut Map, player: Guid, creature: Guid) {
    if guid::is_unit(creature) {
        credit(map, player, creature, 0, &|q: &Quest| !q.needs_event());
    }
}

/// A raid quest, by its type or its flags: its kills and drops count in a
/// raid.
fn raid_quest(q: &Quest) -> bool {
    q.kind == QUEST_TYPE_RAID || q.flags & QUEST_FLAGS_RAID != 0
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

/// Items went (or changed): the quests that need them, done or not now.
pub fn items_changed(map: &mut Map, player: Guid, item: u32) {
    items_added(map, player, item, 0);
}

/// `added` of `item` came: each quest under way that needs it is told how
/// many count, no more than it still wanted (mangos's
/// `ItemAddedQuestCheck`, Thistle Tea's delta), and done or not now.
/// Nothing is told of a loss.
pub fn items_added(map: &mut Map, player: Guid, item: u32, added: u32) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let quests: Vec<(u32, u32)> = p
        .journal
        .log
        .iter()
        .filter(|l| !l.failed)
        .filter_map(|l| data.npc.quests.get(&l.quest))
        .filter_map(|q| {
            q.required_items
                .iter()
                .find(|(i, _)| *i == item)
                .map(|(_, n)| (q.id, *n))
        })
        .collect();
    let before = p.count_of(item).saturating_sub(added);
    for (q, needed) in quests {
        let counted = needed.saturating_sub(before).min(added);
        if counted > 0
            && let Some(p) = map.players.get(&player)
        {
            p.outbound.send(&SMSG_QUESTUPDATE_ADD_ITEM {
                required_item_id: item,
                items_required: counted,
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

/// Whether `player` is in a raid group: only raid quests count there.
pub fn in_raid(map: &Map, player: Guid) -> bool {
    let groups = map.realm.social.groups.lock();
    groups.group_of(player).is_some_and(|g| g.raid)
}

/// The quest items `player`'s quests still need: they drop for them; in a
/// raid, only raid quests' (mangos's `HasQuestForItem`).
pub fn wanted(map: &Map, player: Guid) -> Vec<u32> {
    let raid = in_raid(map, player);
    map.players
        .get(&player)
        .map(|p| wanted_items(p, &map.realm.data, raid))
        .unwrap_or_default()
}

/// The quest items `p`'s quests under way still need.
pub fn wanted_items(p: &Player, data: &GameData, raid: bool) -> Vec<u32> {
    let mut out = Vec::new();
    for l in p.journal.log.iter().filter(|l| !l.failed) {
        let Some(q) = data.npc.quests.get(&l.quest) else {
            continue;
        };
        if raid && !raid_quest(q) {
            continue;
        }
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
    let Some(done) = turn_in(map, player, giver, q) else {
        return;
    };
    let Some(p) = map.players.get(&player) else {
        return;
    };
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
    if turn_in(map, player, giver, q) != Some(true) {
        return;
    }
    let Some(p) = map.players.get(&player) else {
        return;
    };
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
/// outlevels it), rounded up, as mangos's `Quest::XPValue` and Thistle
/// Tea's `Experience.quest_xp` round it.
pub fn quest_xp(q: &Quest, level: u8) -> u32 {
    let qlevel = q.level_for(level);
    let diff = i64::from(level) - i64::from(qlevel);
    let share = match diff {
        i64::MIN..=5 => 1.0,
        6 => 0.8,
        7 => 0.6,
        8 => 0.4,
        9 => 0.2,
        _ => 0.1,
    };
    (f64::from(q.reward_xp) * share).ceil() as u32
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
    if turn_in(map, player, giver, &q) != Some(true) {
        return;
    }
    let Some(p) = map.players.get(&player) else {
        return;
    };
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
    // What it takes and gives, as one change: the rewards in the room the
    // taken items leave, onto stacks, within their unique limits; what it
    // takes carried, not banked (Thistle Tea's turn-in plan, mangos's
    // "quest items in bank").
    let required: Vec<(u32, u32)> = q
        .required_items
        .iter()
        .copied()
        .filter(|(i, _)| *i != 0)
        .collect();
    let given: Vec<(u32, u32)> = rewards.iter().map(|(i, n)| (*i, (*n).max(1))).collect();
    match inventory::plan_change(p, &data, &required, &given) {
        Ok(()) => {}
        Err(inventory::Fail::InventoryFull) => {
            return invalid(p, QuestFailedReason::QuestFailedInventoryFull);
        }
        Err(f) => return inventory::send_fail(p, f, Guid::zero()),
    }
    // Take what the quest needed.
    for (item, count) in q.required_items.iter().filter(|(i, _)| *i != 0) {
        take_items(map, player, *item, *count, false);
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
        p.journal.clear(slot);
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
    // The chain's next quest, offered at once if this giver gives it and
    // it may be taken, its condition too (mangos's `GetNextQuest`).
    if q.next_in_chain != 0
        && relations(map, giver).0.contains(&q.next_in_chain)
        && condition_ok(map, player, giver, q.next_in_chain)
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

/// Take `count` of `item` from wherever the player carries it, the bank
/// too if `banked` (an abandoned quest's items, as Thistle Tea's
/// `QuestItems.abandon`); a turn-in's from what is carried alone
/// (mangos's `DestroyItemCount`).
fn take_items(map: &mut Map, player: Guid, item: u32, mut count: u32, banked: bool) {
    while count > 0 {
        let Some(p) = map.players.get(&player) else {
            return;
        };
        let Some(i) = p
            .items
            .iter()
            .position(|it| it.entry == item && (banked || it.carried()))
        else {
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
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let Some(quest) = p
        .journal
        .log
        .get(usize::from(slot))
        .map(|l| l.quest)
        .filter(|q| *q != 0)
    else {
        return;
    };
    // The items the quest gave go with it, from the bags and the bank
    // (mangos's `TakeQuestSourceItem`), unless they start it again
    // (Thistle Tea's `QuestItems.abandon`); a bag with something in it
    // can't go, and the quest stays.
    if let Some(q) = data.npc.quests.get(&quest)
        && q.source_item != 0
        && data
            .item(q.source_item)
            .is_none_or(|t| t.start_quest != quest)
    {
        if let Some(bag) = p
            .items
            .iter()
            .find(|i| i.entry == q.source_item && holds_items(p, i))
        {
            return inventory::send_fail(p, inventory::Fail::OnlyEmptyBags, bag.guid);
        }
        take_items(map, player, q.source_item, q.source_item_count.max(1), true);
    }
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    p.journal.clear(usize::from(slot));
    refresh_log(p, &data);
}

/// Whether `item` is a bag, worn or in the bank's bag slots, with
/// something in it.
fn holds_items(p: &Player, item: &crate::game::map::player::Item) -> bool {
    let bag_slot = item.bag == crate::game::store::BAG_PLAYER
        && ((inventory::BAG_START..inventory::BAG_END).contains(&item.slot)
            || (inventory::BANK_BAG_START..inventory::BANK_BAG_END).contains(&item.slot));
    bag_slot && p.items.iter().any(|i| i.bag == item.slot)
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
    b.u32(q.point.0)
        .f32(q.point.1)
        .f32(q.point.2)
        .u32(q.point.3);
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
