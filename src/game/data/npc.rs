//! What NPCs say and offer: gossip menus and texts, trainers' spells,
//! flight paths, quests and who gives them.

use ahash::{AHashMap, AHashSet};

/// A gossip menu: its texts (each with a condition) and options.
#[derive(Debug, Clone, Default)]
pub struct GossipMenu {
    pub texts: Vec<(u32, u32)>,
    pub options: Vec<GossipOption>,
}

#[derive(Debug, Clone, Default)]
pub struct GossipOption {
    pub id: u32,
    pub icon: u8,
    pub text: String,
    /// What it does: gossip 1, quest giver 2, vendor 3, taxi 4, trainer 5,
    /// spirit healer 6, innkeeper 8, banker 9, petitioner 10, tabard 11,
    /// battlemaster 12, auctioneer 13, stable 14, armorer 15.
    pub kind: u32,
    pub npc_flag: u32,
    pub action_menu: i32,
    pub action_poi: u32,
    pub coded: bool,
    pub box_money: u32,
    pub box_text: String,
    pub condition: u32,
    /// A gossip script it runs.
    pub script: u32,
}

/// One of an NPC text's eight choices: its broadcast text and probability.
#[derive(Debug, Clone, Copy, Default)]
pub struct NpcTextChoice {
    pub broadcast: u32,
    pub probability: f32,
}

#[derive(Debug, Clone, Default)]
pub struct BroadcastText {
    pub male: String,
    pub female: String,
    pub language: u32,
    pub emotes: [(u32, u32); 3],
    /// How a creature says it: 0 say, 1 yell, 2 emote, 3 boss emote, 4
    /// whisper, 5 boss whisper, 6 zone yell.
    pub chat_type: u32,
    pub sound: u32,
}

/// A spell a trainer teaches.
#[derive(Debug, Clone, Copy, Default)]
pub struct TrainerSpell {
    pub spell: u32,
    pub cost: u32,
    pub required_skill: u32,
    pub required_skill_value: u32,
    pub required_level: u8,
}

#[derive(Debug, Clone, Default)]
pub struct TaxiNode {
    pub id: u32,
    pub map: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub name: String,
    /// The creatures whose looks the mounts have: Alliance, Horde.
    pub mounts: [u32; 2],
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TaxiPathNode {
    pub map: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub flags: u32,
    pub delay: u32,
}

/// A quest, from `quest_template`.
#[derive(Debug, Clone, Default)]
pub struct Quest {
    pub id: u32,
    pub method: u32,
    pub zone_or_sort: i32,
    pub min_level: u8,
    pub level: i32,
    pub kind: u32,
    pub required_classes: u32,
    pub required_races: u32,
    pub required_skill: u32,
    pub required_skill_value: u32,
    pub prev_quest: i32,
    pub next_quest: i32,
    pub exclusive_group: i32,
    pub next_in_chain: u32,
    /// A breadcrumb leads to this quest, and isn't taken beside it.
    pub breadcrumb_for: u32,
    pub source_item: u32,
    pub source_item_count: u32,
    pub limit_time: u32,
    pub flags: u32,
    pub special_flags: u32,
    pub suggested_players: u32,
    pub title: String,
    pub details: String,
    pub objectives: String,
    pub offer_reward_text: String,
    pub request_items_text: String,
    pub end_text: String,
    pub objective_texts: [String; 4],
    /// Items to bring: (item, count).
    pub required_items: [(u32, u32); 4],
    /// Creatures to kill (positive) or objects to use (negative): (id,
    /// count).
    pub required_kills: [(i32, u32); 4],
    pub required_spell_casts: [u32; 4],
    /// Items that drop only for the quest's holders: (item, count).
    pub source_items: [(u32, u32); 4],
    pub choice_rewards: [(u32, u32); 6],
    pub rewards: [(u32, u32); 4],
    pub reputation_rewards: [(u32, i32); 5],
    /// A standing the player must have (at least, and below) to take it.
    pub required_min_rep: (u32, i32),
    pub required_max_rep: (u32, i32),
    /// A standing to reach: an objective.
    pub rep_objective: (u32, i32),
    pub reward_xp: u32,
    pub reward_money: i32,
    pub reward_money_max_level: u32,
    pub reward_spell: u32,
    pub reward_spell_cast: u32,
    pub point: (u32, f32, f32, u32),
    pub details_emotes: [(u32, u32); 4],
    pub incomplete_emote: u32,
    pub complete_emote: u32,
    pub offer_reward_emotes: [(u32, u32); 4],
}

impl Quest {
    /// Whether the quest has anything to do: kills, items or an
    /// exploration.
    pub fn has_objectives(&self) -> bool {
        self.required_items.iter().any(|(i, _)| *i != 0)
            || self.required_kills.iter().any(|(k, _)| *k != 0)
            || self.needs_event()
            || self.rep_objective.0 != 0
    }

    /// Whether the quest waits on an exploration or an event (a script,
    /// a spell): Thistle Tea's `exploration?`, `SpecialFlags` 0x2, of
    /// which `QuestFlags` 0x4 is a part.
    pub fn needs_event(&self) -> bool {
        self.special_flags & QUEST_SPECIAL_EXPLORATION_OR_EVENT != 0
            || self.flags & QUEST_FLAGS_EXPLORATION != 0
    }

    /// The level a quest is at for XP: its own, or the player's for one at
    /// any level (-1).
    pub fn level_for(&self, player_level: u8) -> u32 {
        if self.level < 0 {
            u32::from(player_level)
        } else {
            self.level as u32
        }
    }
}

/// `quest_template.QuestFlags`.
pub const QUEST_FLAGS_EXPLORATION: u32 = 0x0004;
/// `SpecialFlags`: repeatable.
pub const QUEST_SPECIAL_REPEATABLE: u32 = 0x1;
/// `SpecialFlags`: done by an exploration or an event.
pub const QUEST_SPECIAL_EXPLORATION_OR_EVENT: u32 = 0x2;

#[derive(Debug, Default)]
pub struct NpcData {
    pub gossip_menus: AHashMap<u32, GossipMenu>,
    pub npc_texts: AHashMap<u32, [NpcTextChoice; 8]>,
    pub broadcast_texts: AHashMap<u32, BroadcastText>,
    /// By creature entry, and by trainer template.
    pub trainers: AHashMap<u32, Vec<TrainerSpell>>,
    pub trainer_templates: AHashMap<u32, Vec<TrainerSpell>>,
    pub trainer_greetings: AHashMap<u32, String>,
    pub taxi_nodes: AHashMap<u32, TaxiNode>,
    /// (from, to) → (path, cost).
    pub taxi_paths: AHashMap<(u32, u32), (u32, u32)>,
    pub taxi_path_nodes: AHashMap<u32, Vec<TaxiPathNode>>,
    /// Bank bag slot → price.
    pub bank_slot_prices: AHashMap<u32, u32>,
    pub quests: AHashMap<u32, Quest>,
    /// Creature entry → the quests it starts, and those it ends.
    pub creature_starts: AHashMap<u32, Vec<u32>>,
    pub creature_ends: AHashMap<u32, Vec<u32>>,
    pub object_starts: AHashMap<u32, Vec<u32>>,
    pub object_ends: AHashMap<u32, Vec<u32>>,
    /// Area trigger → the quest it explores for.
    pub area_trigger_quests: AHashMap<u32, u32>,
    pub taverns: AHashSet<u32>,
    pub quest_greetings: AHashMap<u32, String>,
    /// The quests that need each one done first: quest → those after it.
    pub quests_after: AHashMap<u32, Vec<u32>>,
    /// The quests' dependencies, worked out from `quests` when first asked.
    pub graph: std::sync::OnceLock<QuestGraph>,
}

/// A quest that must be rewarded (or, `current`, in the log) first; with a
/// negative exclusive group, all of its group must be.
#[derive(Debug, Clone, Default)]
pub struct Prerequisite {
    pub quest: u32,
    pub current: bool,
    pub group: Vec<u32>,
}

/// What each quest depends on (Thistle Tea's `QuestGraph`, after mangos'
/// `prevQuests`): signed prerequisites, its own and those whose next quest
/// it is; the others of its exclusive group; the chain around it; and the
/// breadcrumbs that lead to it, or the quests it leads to.
#[derive(Debug, Clone, Default)]
pub struct QuestGraph {
    pub prerequisites: AHashMap<u32, Vec<Prerequisite>>,
    pub exclusive: AHashMap<u32, Vec<u32>>,
    /// Quest → the quests whose next in chain it is.
    pub previous_chain: AHashMap<u32, Vec<u32>>,
    /// Breadcrumb → the quests it leads to, nearest first.
    pub breadcrumb_targets: AHashMap<u32, Vec<u32>>,
    /// Quest → the breadcrumbs that lead to it.
    pub breadcrumbs_to: AHashMap<u32, Vec<u32>>,
    /// Breadcrumbs that lead nowhere, or round in a circle.
    pub invalid: AHashSet<u32>,
}

impl QuestGraph {
    pub fn compile(quests: &AHashMap<u32, Quest>) -> Self {
        let mut g = Self::default();
        let mut groups: AHashMap<i32, Vec<u32>> = AHashMap::new();
        let mut incoming: AHashMap<u32, Vec<&Quest>> = AHashMap::new();
        for q in quests.values() {
            if q.exclusive_group != 0 {
                groups.entry(q.exclusive_group).or_default().push(q.id);
            }
            if q.next_quest != 0 {
                incoming
                    .entry(q.next_quest.unsigned_abs())
                    .or_default()
                    .push(q);
            }
            if q.next_in_chain != 0 {
                g.previous_chain
                    .entry(q.next_in_chain)
                    .or_default()
                    .push(q.id);
            }
        }
        for v in groups.values_mut() {
            v.sort_unstable();
        }
        for q in quests.values() {
            // Where its breadcrumbs go, if it is one.
            if q.breadcrumb_for != 0 {
                let mut path = Vec::new();
                let mut seen = AHashSet::from([q.id]);
                let mut at = q.breadcrumb_for;
                loop {
                    let Some(t) = quests.get(&at).filter(|_| seen.insert(at)) else {
                        g.invalid.insert(q.id);
                        break;
                    };
                    path.push(at);
                    if t.breadcrumb_for == 0 {
                        break;
                    }
                    at = t.breadcrumb_for;
                }
                if !g.invalid.contains(&q.id) {
                    for t in &path {
                        g.breadcrumbs_to.entry(*t).or_default().push(q.id);
                    }
                    g.breadcrumb_targets.insert(q.id, path);
                }
            }
            let mut signed: Vec<i64> = Vec::new();
            if q.prev_quest != 0
                && quests
                    .get(&q.prev_quest.unsigned_abs())
                    .is_some_and(|p| p.breadcrumb_for == 0)
            {
                signed.push(i64::from(q.prev_quest));
            }
            let mut before = incoming.get(&q.id).cloned().unwrap_or_default();
            before.sort_by_key(|p| p.id);
            for p in before {
                let id = i64::from(p.id);
                signed.push(if p.next_quest < 0 { -id } else { id });
            }
            let mut seen = AHashSet::new();
            signed.retain(|s| seen.insert(*s));
            let prereqs: Vec<Prerequisite> = signed
                .into_iter()
                .filter_map(|s| {
                    let p = quests.get(&(s.unsigned_abs() as u32))?;
                    let group = if p.exclusive_group < 0 {
                        groups.get(&p.exclusive_group).cloned().unwrap_or_default()
                    } else {
                        vec![p.id]
                    };
                    Some(Prerequisite {
                        quest: p.id,
                        current: s < 0,
                        group,
                    })
                })
                .collect();
            if !prereqs.is_empty() {
                g.prerequisites.insert(q.id, prereqs);
            }
            if q.exclusive_group > 0 {
                let others: Vec<u32> = groups[&q.exclusive_group]
                    .iter()
                    .copied()
                    .filter(|o| *o != q.id)
                    .collect();
                g.exclusive.insert(q.id, others);
            }
        }
        for v in g.previous_chain.values_mut() {
            v.sort_unstable();
        }
        for v in g.breadcrumbs_to.values_mut() {
            v.sort_unstable();
        }
        g
    }
}

impl NpcData {
    /// The quests' dependencies.
    pub fn quest_graph(&self) -> &QuestGraph {
        self.graph.get_or_init(|| QuestGraph::compile(&self.quests))
    }
}

impl NpcData {
    /// What a trainer teaches: its entry's spells, then its template's.
    pub fn trainer_spells(&self, entry: u32, trainer_id: u32) -> Vec<TrainerSpell> {
        let mut out = self.trainers.get(&entry).cloned().unwrap_or_default();
        if trainer_id != 0
            && let Some(t) = self.trainer_templates.get(&trainer_id)
        {
            out.extend(t.iter().copied());
        }
        out
    }

    /// A broadcast text's line for a speaker of `gender`.
    pub fn broadcast_line(&self, id: u32, female: bool) -> Option<&str> {
        let b = self.broadcast_texts.get(&id)?;
        let line = if female && !b.female.is_empty() {
            &b.female
        } else {
            &b.male
        };
        Some(line.as_str())
    }
}
