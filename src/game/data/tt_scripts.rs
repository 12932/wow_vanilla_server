//! Thistle Tea's creature scripts: its ports of vmangos's C++ creature AIs
//! (the `script_name` scripts: bosses, escorts, event creatures), which it
//! builds in code as EventAI events. `tools/dump_tt_scripts.exs` writes
//! them out of Thistle Tea as JSON (`tt_scripts.json.gz`); this makes
//! them this server's events, script steps and conditions. A scripted
//! creature's events replace the world database's, as a script name
//! stands over EventAI in vmangos.
//!
//! A code-built step carries what a database row names by id: an inline
//! condition, sub-scripts, texts. Each gets an id of its own here, from
//! [`BASE`] up. Its commands and target types that vmangos has no number
//! for get this server's ([`crate::game::map::scripts`]'s `C_*`). With
//! the events come the paths their creatures walk (`start_waypoints`
//! source 5), the gossip menus their C++ scripts build, and the steps
//! some add to quests' start and completion scripts.

use std::collections::BTreeMap;
use std::io::Read;

use serde_json::Value;
use tracing::{info, warn};

use super::GameData;
use super::npc::{GossipMenu, GossipOption};
use super::scripts::*;
use crate::game::map::scripts as cmd;

const DUMP: &[u8] = include_bytes!("tt_scripts.json.gz");

/// Ids for what is made here, clear of the world database's.
pub const BASE: u32 = 90_000_000;

/// A route's waypoints, by (creature entry, variant).
pub type Routes = ahash::AHashMap<(u32, u32), Vec<super::Waypoint>>;

struct Import<'a> {
    data: &'a mut GameData,
    next: u32,
    /// What has no meaning here, and how often.
    unknown: BTreeMap<String, u32>,
}

/// Thistle Tea's scripts, into the game data; what of them has no
/// meaning here, and how often.
pub fn install(data: &mut GameData) -> BTreeMap<String, u32> {
    if !data.is_loaded() {
        return BTreeMap::new();
    }
    let mut json = String::new();
    if let Err(e) = flate2::read::GzDecoder::new(DUMP).read_to_string(&mut json) {
        warn!("Thistle Tea's scripts: {e}");
        return BTreeMap::new();
    }
    let root: Value = match serde_json::from_str(&json) {
        Ok(v) => v,
        Err(e) => {
            warn!("Thistle Tea's scripts: {e}");
            return BTreeMap::new();
        }
    };
    let mut im = Import {
        data,
        next: BASE,
        unknown: BTreeMap::new(),
    };
    im.run(&root);
    if !im.unknown.is_empty() {
        info!("Thistle Tea's scripts: left out {:?}", im.unknown);
    }
    im.unknown
}

impl Import<'_> {
    fn id(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    fn unknown(&mut self, what: String) {
        *self.unknown.entry(what).or_default() += 1;
    }

    fn run(&mut self, root: &Value) {
        let mut events = 0;
        if let Some(by_entry) = root.get("events").and_then(Value::as_object) {
            for (entry, list) in by_entry {
                let Ok(entry) = entry.parse::<u32>() else {
                    continue;
                };
                let list: Vec<AiEvent> = list
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| self.event(entry, e))
                    .collect();
                events += list.len();
                // The script stands over the creature's own events.
                self.data.scripts.ai_events.insert(entry, list);
            }
        }
        for r in root
            .get("routes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            self.route(r);
        }
        if let Some(menus) = root.get("gossip").and_then(Value::as_object) {
            for (entry, g) in menus {
                if let Ok(entry) = entry.parse::<u32>() {
                    self.gossip(entry, g);
                }
            }
        }
        // Area triggers' and events' scripts stand in for the database's.
        if let Some(triggers) = root.get("area_triggers").and_then(Value::as_object) {
            for (id, steps) in triggers {
                if let Ok(id) = id.parse::<u32>() {
                    let steps = self.steps(steps);
                    self.data.scripts.areatrigger.insert(id, steps);
                    self.data.scripts.areatrigger_script.remove(&id);
                }
            }
        }
        if let Some(events) = root.get("event_scripts").and_then(Value::as_object) {
            for (id, steps) in events {
                if let Ok(id) = id.parse::<u32>() {
                    let steps = self.steps(steps);
                    self.data.scripts.event.insert(id, steps);
                }
            }
        }
        if let Some(objects) = root.get("object_scripts").and_then(Value::as_object) {
            for (entry, o) in objects {
                let Ok(entry) = entry.parse::<u32>() else {
                    continue;
                };
                if let Some(any) = o
                    .get("any")
                    .filter(|a| a.as_array().is_some_and(|a| !a.is_empty()))
                {
                    let id = self.generic(any);
                    self.data.scripts.object_hello_entry.insert(entry, id);
                }
                for (spawn, steps) in o
                    .get("spawns")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flatten()
                {
                    if let Ok(spawn) = spawn.parse::<u32>()
                        && steps.as_array().is_some_and(|a| !a.is_empty())
                    {
                        let id = self.generic(steps);
                        self.data.scripts.object_hello.insert(spawn, id);
                    }
                }
            }
        }
        for (key, start) in [("quest_start_steps", true), ("quest_end_steps", false)] {
            if let Some(quests) = root.get(key).and_then(Value::as_object) {
                for (quest, steps) in quests {
                    if let Ok(quest) = quest.parse::<u32>() {
                        self.quest_steps(quest, steps, start);
                    }
                }
            }
        }
        let creatures = root
            .get("entries")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        info!(
            "Thistle Tea's scripts: {events} events for {creatures} creatures, {} routes",
            self.data.scripts.routes.len()
        );
    }

    fn event(&mut self, entry: u32, v: &Value) -> Option<AiEvent> {
        let kind_atom = atom(v.get("event_type"));
        let Some(kind) = event_type(kind_atom) else {
            self.unknown(format!("event {kind_atom}"));
            return None;
        };
        let mut flags = 0;
        for (key, flag) in [
            ("repeatable?", EF_REPEATABLE),
            ("random_action?", EF_RANDOM_ACTION),
            ("not_casting?", EF_NOT_CASTING),
            ("check_result?", EF_CHECK_RESULT),
            ("victim_in_melee?", EF_VICTIM_IN_MELEE),
            ("pause_while_stunned?", EF_PAUSE_WHILE_STUNNED),
        ] {
            if boolean(v.get(key)) {
                flags |= flag;
            }
        }
        let condition = match v.get("condition").filter(|c| !c.is_null()) {
            Some(c) => self.condition(c),
            None => int(v.get("condition_id")) as u32,
        };
        let mut actions = [0; 3];
        for (i, steps) in v
            .get("actions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(3)
            .enumerate()
        {
            let id = self.id();
            let steps = self.steps(steps);
            self.data.scripts.ai_scripts.insert(id, steps);
            actions[i] = id;
        }
        Some(AiEvent {
            id: int(v.get("id")) as u32,
            creature: entry,
            condition,
            kind,
            inverse_phase_mask: int(v.get("inverse_phase_mask")) as u32,
            chance: v.get("chance").map_or(100, |c| int(Some(c)) as u32),
            flags,
            params: [
                int(v.get("param1")) as i32,
                int(v.get("param2")) as i32,
                int(v.get("param3")) as i32,
                int(v.get("param4")) as i32,
            ],
            actions,
            miss_repeat: None,
        })
    }

    /// A list of steps, sorted by delay as the tables are.
    fn steps(&mut self, v: &Value) -> Vec<ScriptStep> {
        let mut out: Vec<ScriptStep> = v
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| self.step(s))
            .collect();
        out.sort_by_key(|s| s.delay_ms);
        out
    }

    /// A generic script of `steps`, by a new id.
    fn generic(&mut self, steps: &Value) -> u32 {
        let id = self.id();
        let steps = self.steps(steps);
        self.data.scripts.generic.insert(id, steps);
        id
    }

    fn step(&mut self, v: &Value) -> ScriptStep {
        let command_atom = atom(v.get("command"));
        let command = command(command_atom).unwrap_or_else(|| {
            self.unknown(format!("command {command_atom}"));
            u32::MAX
        });
        let target_atom = v.get("target_type").map_or(":provided", |t| atom(Some(t)));
        let target_type = target_type(target_atom).unwrap_or_else(|| {
            self.unknown(format!("target {target_atom}"));
            u32::MAX
        });
        let mut data_flags = 0;
        for (key, flag) in [
            ("swap_initial?", SF_SWAP_INITIAL_TARGETS),
            ("swap_final?", SF_SWAP_FINAL_TARGETS),
            ("target_self?", SF_TARGET_SELF),
            ("abort_on_failure?", SF_ABORT_ON_FAILURE),
        ] {
            if boolean(v.get(key)) {
                data_flags |= flag;
            }
        }
        let [x, y, z, o] = position(v.get("position"));
        let mut step = ScriptStep {
            id: 0,
            delay_ms: int(v.get("delay_ms")).max(0) as u32,
            priority: int(v.get("priority")).max(0) as u32,
            command,
            datalong: ["datalong", "datalong2", "datalong3", "datalong4"]
                .map(|k| int(v.get(k)) as u32),
            dataint: ["dataint", "dataint2", "dataint3", "dataint4"].map(|k| int(v.get(k)) as i32),
            target_type,
            target_param1: int(v.get("target_param1")) as u32,
            target_param2: int(v.get("target_param2")) as u32,
            data_flags,
            x,
            y,
            z,
            o,
            condition: int(v.get("condition_id")) as u32,
            ext: None,
        };
        if let Some(c) = v.get("condition").filter(|c| !c.is_null()) {
            step.condition = self.condition(c);
        }
        // A sub-script a step names: a new generic script.
        let subs: BTreeMap<i64, u32> = v
            .get("sub_scripts")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(k, steps)| Some((k.parse::<i64>().ok()?, steps)))
            .map(|(k, steps)| (k, self.generic(steps)))
            .collect();
        let sub = |n: i64| subs.get(&n).copied().unwrap_or(0);
        match command {
            39 | 90 => {
                for d in step.datalong.iter_mut() {
                    *d = sub(i64::from(*d));
                }
            }
            10 => step.dataint[1] = sub(i64::from(step.dataint[1])) as i32,
            61 | 63 | 69 => {
                for i in [1, 3] {
                    if step.dataint[i] > 0 {
                        step.dataint[i] = sub(i64::from(step.dataint[i])) as i32;
                    }
                }
                for (key, i) in [("success_condition", 0), ("failure_condition", 2)] {
                    if let Some(c) = v.get(key).filter(|c| !c.is_null()) {
                        step.dataint[i] = self.condition(c) as i32;
                    }
                }
            }
            68 | 92 => step.datalong[0] = sub(i64::from(step.datalong[0])),
            cmd::C_HOLD_WAYPOINTS | cmd::C_LINKED_FEIGN_DEATH => {
                step.dataint[0] = sub(1) as i32;
                step.dataint[1] = sub(2) as i32;
            }
            _ => {}
        }
        if let Some(c) = v.get("termination_condition").filter(|c| !c.is_null()) {
            step.datalong[0] = self.condition(c);
        }
        if command == 19
            && let Some(items) = v.get("equipment_items").and_then(Value::as_array)
        {
            for (i, item) in items.iter().take(3).enumerate() {
                step.dataint[i] = int(Some(item)) as i32;
            }
        }
        let mut ext = StepExt::default();
        let mut extended = false;
        if let Some(texts) = v.get("texts").and_then(Value::as_array) {
            for t in texts {
                ext.texts.push(InlineText {
                    text: t
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    chat_type: chat_type(atom(t.get("chat_type"))),
                    language: int(t.get("language")) as u32,
                    emote: int(t.get("emote_id")) as u32,
                });
                extended = true;
            }
        }
        if let Some(ps) = v.get("positions").and_then(Value::as_array) {
            ext.positions = ps.iter().map(|p| position(Some(p))).collect();
            extended |= !ext.positions.is_empty();
        }
        if let Some(s) = v.get("scatter").and_then(Value::as_f64) {
            ext.scatter = s as f32;
            extended |= s > 0.0;
        }
        match v.get("count") {
            Some(Value::Number(n)) => {
                ext.count = SummonCount::Fixed(n.as_u64().unwrap_or(1) as u32);
                extended = true;
            }
            Some(t @ Value::Object(_)) => {
                let parts = tuple(Some(t));
                if parts.len() == 4 && atom(parts.first()) == ":threat_players" {
                    ext.count = SummonCount::ThreatPlayers(
                        float(parts.get(1)) as f32,
                        int(parts.get(2)) as u32,
                        int(parts.get(3)) as u32,
                    );
                    extended = true;
                }
            }
            _ => {}
        }
        if boolean(v.get("at_target?")) {
            ext.at_target = true;
            extended = true;
        }
        if boolean(v.get("formation_from_position?")) {
            ext.formation_from_position = true;
            extended = true;
        }
        if let Some(d) = v.get("minimum_target_distance_2d").and_then(Value::as_f64) {
            ext.min_target_distance_2d = Some(d as f32);
            extended = true;
        }
        if boolean(v.get("concealed?")) {
            ext.concealed = true;
            extended = true;
        }
        for key in ["buddy_guid", "game_object_spawn", "target_condition"] {
            if v.get(key)
                .is_some_and(|x| !x.is_null() && x != &Value::Bool(false))
            {
                self.unknown(format!("step {key}"));
            }
        }
        if v.get("creature_spell_lists")
            .and_then(Value::as_object)
            .is_some_and(|m| !m.is_empty())
        {
            self.unknown("step creature_spell_lists".into());
        }
        if extended {
            step.ext = Some(Box::new(ext));
        }
        step
    }

    /// A condition tree: its leaves and combinators get entries of their
    /// own.
    fn condition(&mut self, v: &Value) -> u32 {
        let type_atom = atom(v.get("type"));
        let Some(kind) = condition_type(type_atom) else {
            self.unknown(format!("condition {type_atom}"));
            // Unknown: never met, as Thistle Tea's evaluator has it.
            let id = self.id();
            self.data.scripts.conditions.insert(
                id,
                Condition {
                    entry: id,
                    kind: i32::MIN,
                    values: [0; 4],
                    flags: 0,
                },
            );
            return id;
        };
        let mut values = ["value1", "value2", "value3", "value4"].map(|k| int(v.get(k)) as i32);
        let floats = ["value1", "value2", "value3", "value4"].map(|k| float(v.get(k)) as f32);
        let children: Vec<u32> = v
            .get("children")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|c| self.condition(c))
            .collect();
        match kind {
            -1 | -2 if children.len() > 4 => {
                // More than four: nested, four at a time.
                let mut groups: Vec<u32> = Vec::new();
                for chunk in children.chunks(4) {
                    let id = self.id();
                    let mut vals = [0; 4];
                    for (i, c) in chunk.iter().enumerate() {
                        vals[i] = *c as i32;
                    }
                    self.data.scripts.conditions.insert(
                        id,
                        Condition {
                            entry: id,
                            kind,
                            values: vals,
                            flags: 0,
                        },
                    );
                    groups.push(id);
                }
                values = [0; 4];
                for (i, g) in groups.iter().take(4).enumerate() {
                    values[i] = *g as i32;
                }
            }
            -3..=-1 if !children.is_empty() => {
                values = [0; 4];
                for (i, c) in children.iter().take(4).enumerate() {
                    values[i] = *c as i32;
                }
            }
            47 | 50 if !children.is_empty() => values[1] = children[0] as i32,
            _ => {}
        }
        let mut flags = 0;
        if boolean(v.get("reverse?")) {
            flags |= CF_REVERSE_RESULT;
        }
        if boolean(v.get("swap_targets?")) {
            flags |= CF_SWAP_TARGETS;
        }
        let id = self.id();
        self.data.scripts.conditions.insert(
            id,
            Condition {
                entry: id,
                kind,
                values,
                flags,
            },
        );
        if floats.iter().any(|f| f.fract() != 0.0) {
            self.data.scripts.condition_floats.insert(id, floats);
        }
        id
    }

    /// A path a script walks: its own points, or the creature's
    /// `script_waypoint` rows, and the steps at its points.
    fn route(&mut self, v: &Value) {
        let entry = int(v.get("entry")) as u32;
        let variant = int(v.get("variant")) as u32;
        let mut points: Vec<super::Waypoint> = match v.get("path").and_then(Value::as_array) {
            Some(path) => path
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let t = tuple(Some(p));
                    super::Waypoint {
                        point: i as u32,
                        x: float(t.first()) as f32,
                        y: float(t.get(1)) as f32,
                        z: float(t.get(2)) as f32,
                        orientation: None,
                        wait_ms: int(t.get(3)).max(0) as u32,
                        script_id: 0,
                    }
                })
                .collect(),
            None => self
                .data
                .script_waypoints
                .get(&entry)
                .cloned()
                .unwrap_or_default(),
        };
        if points.is_empty() {
            self.unknown(format!("route of {entry} with no path"));
            return;
        }
        if let Some(at) = v.get("points").and_then(Value::as_object) {
            // By the point's number: a path's own from 0, else the
            // `script_waypoint` rows' `pointid`.
            for (point, steps) in at {
                let Ok(point) = point.parse::<u32>() else {
                    continue;
                };
                let id = self.id();
                let steps = self.steps(steps);
                self.data.scripts.movement.insert(id, steps);
                if let Some(p) = points.iter_mut().find(|p| p.point == point) {
                    p.script_id = id;
                }
            }
        }
        self.data.scripts.routes.insert((entry, variant), points);
    }

    /// A gossip menu built in code: it replaces the creature's own.
    fn gossip(&mut self, entry: u32, v: &Value) {
        let menu_id = self.id();
        let mut texts: Vec<(u32, u32)> = Vec::new();
        for t in v
            .get("texts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let text = int(t.get("text_id")) as u32;
            let condition = match t.get("condition").filter(|c| !c.is_null()) {
                Some(c) => self.condition(c),
                None => 0,
            };
            texts.push((text, condition));
        }
        let mut options = Vec::new();
        for (i, o) in v
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let condition = match o.get("condition").filter(|c| !c.is_null()) {
                Some(c) => self.condition(c),
                None => 0,
            };
            let script = self.id();
            let steps = self.steps(o.get("steps").unwrap_or(&Value::Null));
            self.data.scripts.gossip.insert(script, steps);
            let reply = int(o.get("reply_text_id")) as u32;
            let action_menu = if reply != 0 {
                let id = self.id();
                self.data.npc.gossip_menus.insert(
                    id,
                    GossipMenu {
                        texts: vec![(reply, 0)],
                        options: Vec::new(),
                    },
                );
                id as i32
            } else {
                -1
            };
            options.push(GossipOption {
                id: i as u32,
                icon: int(o.get("icon")) as u8,
                text: o
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                kind: 1,
                action_menu,
                condition,
                script,
                ..Default::default()
            });
        }
        self.data
            .npc
            .gossip_menus
            .insert(menu_id, GossipMenu { texts, options });
        if let Some(t) = self.data.creature_templates.get_mut(&entry) {
            t.gossip_menu_id = menu_id;
        }
    }

    /// Steps a script adds to a quest's start or completion script.
    fn quest_steps(&mut self, quest: u32, v: &Value, start: bool) {
        let steps = self.steps(v);
        if steps.is_empty() {
            return;
        }
        let (scripts, table) = if start {
            (
                &mut self.data.scripts.quest_start_script,
                &mut self.data.scripts.quest_start,
            )
        } else {
            (
                &mut self.data.scripts.quest_complete_script,
                &mut self.data.scripts.quest_end,
            )
        };
        let id = match scripts.get(&quest) {
            Some(id) => *id,
            None => {
                self.next += 1;
                scripts.insert(quest, self.next);
                self.next
            }
        };
        let list = table.entry(id).or_default();
        list.extend(steps);
        list.sort_by_key(|s| s.delay_ms);
    }
}

/// An atom, `":name"`.
fn atom(v: Option<&Value>) -> &str {
    v.and_then(Value::as_str).unwrap_or("")
}

fn int(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_u64().map(|u| u as i64))
            .or_else(|| n.as_f64().map(|f| f as i64))
            .unwrap_or(0),
        Some(Value::Bool(true)) => 1,
        _ => 0,
    }
}

fn float(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(0.0)
}

fn boolean(v: Option<&Value>) -> bool {
    v.and_then(Value::as_bool).unwrap_or(false)
}

/// A tuple, `{"t": [...]}`.
fn tuple(v: Option<&Value>) -> Vec<Value> {
    v.and_then(|t| t.get("t"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// `{x, y, z, o}`.
fn position(v: Option<&Value>) -> [f32; 4] {
    let t = tuple(v);
    [0, 1, 2, 3].map(|i| float(t.get(i)) as f32)
}

fn chat_type(a: &str) -> u32 {
    match a {
        ":yell" => cmd::CHAT_YELL,
        ":text_emote" | ":emote" => cmd::CHAT_TEXT_EMOTE,
        ":boss_emote" => cmd::CHAT_BOSS_EMOTE,
        ":whisper" => cmd::CHAT_WHISPER,
        ":boss_whisper" => cmd::CHAT_BOSS_WHISPER,
        ":zone_yell" => cmd::CHAT_ZONE_YELL,
        _ => cmd::CHAT_SAY,
    }
}

fn event_type(a: &str) -> Option<u32> {
    Some(match a {
        ":timer_in_combat" => EVENT_TIMER_IN_COMBAT,
        ":timer_ooc" => EVENT_TIMER_OOC,
        ":hp" => EVENT_HP,
        ":mana" => EVENT_MANA,
        ":aggro" => EVENT_AGGRO,
        ":kill" => EVENT_KILL,
        ":death" => EVENT_DEATH,
        ":evade" => EVENT_EVADE,
        ":hit_by_spell" => EVENT_HIT_BY_SPELL,
        ":range" => EVENT_RANGE,
        ":ooc_los" => EVENT_OOC_LOS,
        ":spawned" => EVENT_SPAWNED,
        ":target_hp" => EVENT_TARGET_HP,
        ":friendly_hp" => EVENT_FRIENDLY_HP,
        ":friendly_is_cc" => EVENT_FRIENDLY_IS_CC,
        ":friendly_missing_buff" => EVENT_FRIENDLY_MISSING_BUFF,
        ":summoned_unit" => EVENT_SUMMONED_UNIT,
        ":target_mana" => EVENT_TARGET_MANA,
        ":reached_home" => EVENT_REACHED_HOME,
        ":receive_emote" => EVENT_RECEIVE_EMOTE,
        ":aura" => EVENT_AURA,
        ":target_aura" => EVENT_TARGET_AURA,
        ":summoned_just_died" => EVENT_SUMMONED_JUST_DIED,
        ":summoned_just_despawn" => EVENT_SUMMONED_JUST_DESPAWN,
        ":missing_aura" => EVENT_MISSING_AURA,
        ":target_missing_aura" => EVENT_TARGET_MISSING_AURA,
        ":movement_inform" => EVENT_MOVEMENT_INFORM,
        ":leave_combat" => EVENT_LEAVE_COMBAT,
        ":script_event" => EVENT_SCRIPT_EVENT,
        ":group_member_died" => EVENT_GROUP_MEMBER_DIED,
        ":victim_rooted" => EVENT_VICTIM_ROOTED,
        ":spell_hit_target" => EVENT_SPELL_HIT_TARGET,
        _ => return None,
    })
}

/// vmangos's command numbers, and this server's for Thistle Tea's own.
fn command(a: &str) -> Option<u32> {
    Some(match a {
        ":talk" => 0,
        ":emote" => 1,
        ":move_to" => 3,
        ":modify_flags" => 4,
        ":interrupt_casts" => 5,
        // A code-built teleport moves its own creature.
        ":teleport_to" => cmd::C_TELEPORT_SELF,
        ":quest_explored" => 7,
        ":kill_credit" => 8,
        ":respawn_game_object" => 9,
        ":summon_creature" => 10,
        ":open_door" => 11,
        ":close_door" => 12,
        ":activate_object" => 13,
        ":remove_aura" => 14,
        ":cast_spell" => 15,
        ":play_sound" => 16,
        ":create_item" => 17,
        ":despawn" => 18,
        ":set_equipment" => 19,
        ":movement" => 20,
        ":set_faction" => 22,
        ":morph" => 23,
        ":mount" => 24,
        ":set_run" => 25,
        ":attack_start" => 26,
        ":update_entry" => 27,
        ":stand_state" => 28,
        ":modify_threat" => 29,
        ":send_taxi_path" => 30,
        ":terminate_script" => 31,
        ":terminate_condition" => 32,
        ":enter_evade" => 33,
        ":set_home_position" => 34,
        ":turn_to" => 35,
        ":meeting_stone" => 36,
        ":set_instance_data" => 37,
        ":start_script" => 39,
        ":remove_item" => 40,
        ":remove_object" => 41,
        ":set_melee_attack" => 42,
        ":set_combat_movement" => 43,
        ":set_phase" => 44,
        ":set_phase_random" => 45,
        ":set_phase_range" => 46,
        ":flee" => 47,
        ":deal_damage" => 48,
        ":zone_combat_pulse" => 49,
        ":call_for_help" => 50,
        ":set_sheath" => 51,
        ":invincibility" => 52,
        ":set_server_variable" => 54,
        ":creature_spells" => 55,
        ":remove_guardians" => 56,
        ":set_react_state" => 59,
        ":start_waypoints" => 60,
        ":start_map_event" => 61,
        ":end_map_event" => 62,
        ":add_map_event_target" => 63,
        ":remove_map_event_target" => 64,
        ":set_map_event_data" => 65,
        ":send_map_event" => 66,
        ":set_default_movement" => 67,
        ":start_script_for_all" => 68,
        ":edit_map_event" => 69,
        ":fail_quest" => 70,
        ":respawn_creature" => 71,
        ":combat_stop" => 73,
        ":add_aura" => 74,
        ":summon_object" => 76,
        ":set_fly" => 77,
        ":join_creature_group" => 78,
        ":leave_creature_group" => 79,
        ":set_game_object_state" => 80,
        ":despawn_game_object" => 81,
        ":load_game_object_spawn" => 82,
        ":quest_credit" => 83,
        ":set_gossip_menu" => 84,
        ":send_script_event" => 85,
        ":reset_door_or_button" => 87,
        ":set_command_state" => 88,
        ":play_custom_animation" => 89,
        ":start_script_on_group" => 90,
        ":load_creature_spawn" => 91,
        ":start_script_on_zone" => 92,
        ":stop_scripts" => cmd::C_STOP_SCRIPTS,
        ":hold_waypoints" => cmd::C_HOLD_WAYPOINTS,
        ":release_waypoints" => cmd::C_RELEASE_WAYPOINTS,
        ":set_regeneration" => cmd::C_SET_REGENERATION,
        ":set_caster_chase_distance" => cmd::C_SET_CASTER_CHASE_DISTANCE,
        ":set_health_pct" => cmd::C_SET_HEALTH_PCT,
        ":lose_health" => cmd::C_LOSE_HEALTH,
        ":set_concealed" => cmd::C_SET_CONCEALED,
        ":cast_credit" => cmd::C_CAST_CREDIT,
        ":clear_auras" => cmd::C_CLEAR_AURAS,
        ":linked_feign_death" => cmd::C_LINKED_FEIGN_DEATH,
        ":set_melee_spell" => cmd::C_SET_MELEE_SPELL,
        ":set_call_for_help_range" => cmd::C_SET_CALL_FOR_HELP_RANGE,
        ":battleground_event" => cmd::C_BATTLEGROUND_EVENT,
        _ => return None,
    })
}

fn target_type(a: &str) -> Option<u32> {
    Some(match a {
        ":provided" => 0,
        ":victim" => 1,
        ":hostile_second_aggro" => 2,
        ":hostile_last_aggro" => 3,
        ":hostile_random" => 4,
        ":hostile_random_not_top" => 5,
        ":hostile_nearest" => 6,
        ":hostile_farthest" => 7,
        ":owner_or_self" => 8,
        ":owner" => 9,
        ":nearest_creature_with_entry" => 10,
        ":creature_with_guid" => 11,
        ":creature_from_instance_data" => 12,
        ":nearest_game_object_with_entry" => 13,
        ":game_object_with_guid" => 14,
        ":friendly" => 16,
        ":friendly_injured" => 17,
        ":friendly_injured_except" => 18,
        ":friendly_missing_buff" => 19,
        ":friendly_missing_buff_except" => 20,
        ":map_event_source" => 22,
        ":map_event_target" => 23,
        ":map_event_extra_target" => 24,
        ":nearest_player" => 25,
        ":nearest_hostile_player" => 26,
        ":nearest_friendly_player" => 27,
        ":random_creature_with_entry" => 28,
        ":random_game_object_with_entry" => 29,
        ":owner_hostile_random" => cmd::T_OWNER_HOSTILE_RANDOM,
        _ => return None,
    })
}

fn condition_type(a: &str) -> Option<i32> {
    const TYPES: &[&str] = &[
        ":none",
        ":aura",
        ":item",
        ":item_equipped",
        ":area_id",
        ":reputation_rank_min",
        ":team",
        ":skill",
        ":quest_rewarded",
        ":quest_taken",
        ":argent_dawn_commission_aura",
        ":saved_variable",
        ":active_game_event",
        ":cannot_path_to_victim",
        ":race_class",
        ":level",
        ":source_entry",
        ":spell",
        ":instance_script",
        ":quest_available",
        ":nearby_creature",
        ":nearby_game_object",
        ":quest_none",
        ":item_with_bank",
        ":content_patch",
        ":escort",
        ":active_holiday",
        ":gender",
        ":is_player",
        ":skill_below",
        ":reputation_rank_max",
        ":has_flag",
        ":last_waypoint",
        ":map_id",
        ":instance_data",
        ":map_event_data",
        ":map_event_active",
        ":line_of_sight",
        ":distance_to_target",
        ":moving",
        ":has_pet",
        ":health_percent",
        ":mana_percent",
        ":in_combat",
        ":reaction",
        ":in_group",
        ":alive",
        ":map_event_targets",
        ":object_spawned",
        ":object_loot_state",
        ":object_fit_condition",
        ":pvp_rank",
        ":db_guid",
        ":local_time",
        ":distance_to_position",
        ":object_go_state",
        ":nearby_player",
        ":creature_group_member",
        ":creature_group_dead",
        ":area_explored",
    ];
    match a {
        ":not" => Some(-3),
        ":or" => Some(-2),
        ":and" => Some(-1),
        ":mini_pet" => Some(crate::game::map::conditions::MINI_PET),
        _ => TYPES.iter().position(|t| *t == a).map(|i| i as i32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dump_reads() {
        let mut json = String::new();
        flate2::read::GzDecoder::new(DUMP)
            .read_to_string(&mut json)
            .unwrap();
        let root: Value = serde_json::from_str(&json).unwrap();
        let entries = root["entries"].as_array().unwrap();
        assert!(entries.len() > 200, "{}", entries.len());
        // Onyxia, Ragnaros and Majordomo are among them.
        for e in [10184, 11502, 12018] {
            assert!(root["events"].get(e.to_string()).is_some(), "{e}");
        }
    }

    #[test]
    fn every_command_target_and_condition_has_a_number() {
        let mut json = String::new();
        flate2::read::GzDecoder::new(DUMP)
            .read_to_string(&mut json)
            .unwrap();
        let root: Value = serde_json::from_str(&json).unwrap();
        let mut missing = Vec::new();
        fn walk(v: &Value, missing: &mut Vec<String>) {
            match v {
                Value::Object(m) => {
                    match m.get("__struct__").and_then(Value::as_str) {
                        Some("ScriptStep") => {
                            let c = atom(m.get("command"));
                            if command(c).is_none() {
                                missing.push(c.to_owned());
                            }
                            let t = m.get("target_type").map_or(":provided", |t| atom(Some(t)));
                            if target_type(t).is_none() {
                                missing.push(t.to_owned());
                            }
                        }
                        Some("Condition") => {
                            let t = atom(m.get("type"));
                            if condition_type(t).is_none() {
                                missing.push(t.to_owned());
                            }
                        }
                        Some("AIEvent") => {
                            let t = atom(m.get("event_type"));
                            if event_type(t).is_none() {
                                missing.push(t.to_owned());
                            }
                        }
                        _ => {}
                    }
                    for x in m.values() {
                        walk(x, missing);
                    }
                }
                Value::Array(a) => {
                    for x in a {
                        walk(x, missing);
                    }
                }
                _ => {}
            }
        }
        walk(&root, &mut missing);
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "{missing:?}");
    }
}
