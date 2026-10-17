//! The DB script interpreter: vmangos's script commands (0 to 92, as
//! Thistle Tea runs them) for creature AI, generic, event, spell, quest,
//! gossip, movement, object and area trigger scripts. A script runs from a
//! source on a target; each step waits for its delay, may pick another
//! target (`target_type`), may swap the two, and runs if its condition is
//! met.

use std::time::{Duration, Instant};

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::combat::dist;
use crate::game::map::creature::{Creature, Motion};
use crate::game::map::event_ai;
use crate::game::map::{Map, MapKey};

/// A script under way.
#[derive(Debug, Clone)]
pub struct Run {
    pub table: ScriptTable,
    pub id: u32,
    pub next: usize,
    pub started: Instant,
    pub source: Guid,
    pub target: Guid,
}

/// The scripts running on a map.
#[derive(Debug, Default)]
pub struct Runner {
    pub runs: Vec<Run>,
}

/// Start script `id` of `table` from `source` on `target`; its first
/// steps with no delay run at once.
pub fn start(map: &mut Map, table: ScriptTable, id: u32, source: Guid, target: Guid) {
    if id == 0 || !map.realm.data.scripts.table(table).contains_key(&id) {
        return;
    }
    let now = map.now;
    map.scripts.runs.push(Run {
        table,
        id,
        next: 0,
        started: now,
        source,
        target,
    });
    let i = map.scripts.runs.len() - 1;
    advance(map, i);
    map.scripts.runs.retain(|r| r.next != usize::MAX);
}

/// Run what is due of every script.
pub fn update(map: &mut Map) {
    if map.scripts.runs.is_empty() {
        return;
    }
    let mut i = 0;
    while i < map.scripts.runs.len() {
        advance(map, i);
        i += 1;
    }
    map.scripts.runs.retain(|r| r.next != usize::MAX);
}

/// Run the due steps of run `i`; a finished run's `next` becomes
/// `usize::MAX`.
fn advance(map: &mut Map, i: usize) {
    let data = map.realm.data.clone();
    loop {
        let Some(run) = map.scripts.runs.get(i).cloned() else {
            return;
        };
        if run.next == usize::MAX {
            return;
        }
        let Some(steps) = data.scripts.table(run.table).get(&run.id) else {
            map.scripts.runs[i].next = usize::MAX;
            return;
        };
        let Some(step) = steps.get(run.next) else {
            map.scripts.runs[i].next = usize::MAX;
            return;
        };
        if run.started + Duration::from_millis(u64::from(step.delay_ms)) > map.now {
            return;
        }
        map.scripts.runs[i].next += 1;
        if !execute(map, step, run.source, run.target)
            && let Some(r) = map.scripts.runs.get_mut(i)
        {
            r.next = usize::MAX;
            return;
        }
        if map.scripts.runs.get(i).is_none_or(|r| r.next == usize::MAX) {
            return;
        }
    }
}

/// Stop the scripts `source` runs.
pub fn stop_for(map: &mut Map, source: Guid) {
    for r in map.scripts.runs.iter_mut().filter(|r| r.source == source) {
        r.next = usize::MAX;
    }
}

/// Whether a unit (player or creature) is here and alive.
fn alive(map: &Map, g: Guid) -> bool {
    map.unit_view(g).is_some_and(|u| u.alive)
}

/// Run one step from `source` on `target`. False ends the script.
pub fn execute(map: &mut Map, step: &ScriptStep, source: Guid, target: Guid) -> bool {
    let (mut source, mut target) = (source, target);
    if step.data_flags & SF_SWAP_INITIAL_TARGETS != 0 {
        std::mem::swap(&mut source, &mut target);
    }
    if step.target_type != 0 {
        match find_target(map, step, source, target) {
            Some(t) => target = t,
            None => return step.data_flags & SF_ABORT_ON_FAILURE == 0,
        }
    }
    if step.data_flags & SF_TARGET_SELF != 0 {
        target = source;
    }
    if step.data_flags & SF_SWAP_FINAL_TARGETS != 0 {
        std::mem::swap(&mut source, &mut target);
    }
    if step.condition != 0
        && !crate::game::map::conditions::met(map, step.condition, Some(source), Some(target))
    {
        return step.data_flags & SF_ABORT_ON_FAILURE == 0;
    }
    let ok = command(map, step, source, target);
    ok || step.data_flags & SF_ABORT_ON_FAILURE == 0
}

/// The threat list of a creature, highest first.
fn threat_list(map: &Map, g: Guid) -> Vec<Guid> {
    let Some(c) = map.creature(g) else {
        return Vec::new();
    };
    let mut t: Vec<(Guid, f32)> = c
        .threat
        .iter()
        .copied()
        .filter(|(u, _)| alive(map, *u))
        .collect();
    t.sort_by(|a, b| b.1.total_cmp(&a.1));
    t.into_iter().map(|(u, _)| u).collect()
}

/// The creature of spawn `spawn` on this map.
pub fn creature_of_spawn(map: &Map, spawn: u32) -> Option<Guid> {
    map.by_spawn
        .get(&spawn)
        .copied()
        .filter(|g| map.creature(*g).is_some())
}

pub fn object_of_spawn(map: &Map, spawn: u32) -> Option<Guid> {
    map.gameobjects
        .iter()
        .find(|(_, o)| o.spawn_guid == Some(spawn))
        .map(|(_, o)| o.guid)
}

/// Creatures of `entry` within `radius` of `at`, nearest first.
pub fn creatures_near(map: &Map, at: Vector3d, entry: u32, radius: f32, living: bool) -> Vec<Guid> {
    let radius = if radius > 0.0 { radius } else { 100.0 };
    let mut out: Vec<(f32, Guid)> = map
        .grid
        .around(at.x, at.y, radius)
        .filter(|g| guid::is_unit(*g))
        .filter_map(|g| {
            let c = map.creature(g)?;
            let d = dist(c.position, at);
            (c.entry == entry && d <= radius && (!living || c.alive())).then_some((d, g))
        })
        .collect();
    // A dead creature isn't in the grid once its corpse goes.
    if !living {
        for (_, c) in map.creatures.iter() {
            if c.entry == entry
                && c.despawned
                && dist(c.position, at) <= radius
                && !out.iter().any(|(_, g)| *g == c.guid)
            {
                out.push((dist(c.position, at), c.guid));
            }
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.into_iter().map(|(_, g)| g).collect()
}

fn objects_near(map: &Map, at: Vector3d, entry: u32, radius: f32) -> Vec<Guid> {
    let radius = if radius > 0.0 { radius } else { 100.0 };
    let mut out: Vec<(f32, Guid)> = map
        .gameobjects
        .iter()
        .filter(|(_, o)| o.entry == entry && dist(o.position, at) <= radius)
        .map(|(_, o)| (dist(o.position, at), o.guid))
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.into_iter().map(|(_, g)| g).collect()
}

fn players_near(map: &Map, at: Vector3d, radius: f32) -> Vec<Guid> {
    let radius = if radius > 0.0 { radius } else { 100.0 };
    let mut out: Vec<(f32, Guid)> = map
        .players
        .values()
        .filter(|p| p.death == crate::game::map::death::Death::Alive && !p.awaiting_worldport)
        .map(|p| (dist(p.position(), at), p.guid))
        .filter(|(d, _)| *d <= radius)
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.into_iter().map(|(_, g)| g).collect()
}

/// The target a step's `target_type` names, from `source`.
fn find_target(map: &Map, step: &ScriptStep, source: Guid, provided: Guid) -> Option<Guid> {
    let at = map.position_of(source)?;
    let (p1, p2) = (step.target_param1, step.target_param2);
    let mut rng = rand::rng();
    let pick = |v: Vec<Guid>, rng: &mut rand::rngs::ThreadRng| -> Option<Guid> {
        (!v.is_empty()).then(|| v[rng.random_range(0..v.len())])
    };
    match step.target_type {
        0 => (!provided.is_zero()).then_some(provided),
        1 => map
            .creature(source)
            .map(|c| c.combat.victim)
            .filter(|v| !v.is_zero() && alive(map, *v)),
        2 => threat_list(map, source).get(1).copied(),
        3 => threat_list(map, source).last().copied(),
        4 => pick(threat_list(map, source), &mut rng),
        5 => {
            let t = threat_list(map, source);
            if t.len() > 1 {
                pick(t[1..].to_vec(), &mut rng)
            } else {
                t.first().copied()
            }
        }
        6 => {
            let t = threat_list(map, source);
            t.into_iter().min_by(|a, b| {
                let da = map.position_of(*a).map_or(f32::MAX, |p| dist(p, at));
                let db = map.position_of(*b).map_or(f32::MAX, |p| dist(p, at));
                da.total_cmp(&db)
            })
        }
        7 => {
            let t = threat_list(map, source);
            t.into_iter().max_by(|a, b| {
                let da = map.position_of(*a).map_or(0.0, |p| dist(p, at));
                let db = map.position_of(*b).map_or(0.0, |p| dist(p, at));
                da.total_cmp(&db)
            })
        }
        8 => Some(
            map.creature(source)
                .and_then(|c| c.owner.or(c.scripted.as_deref().and_then(|s| s.summoner)))
                .unwrap_or(source),
        ),
        9 => map
            .creature(source)
            .and_then(|c| c.owner.or(c.scripted.as_deref().and_then(|s| s.summoner))),
        10 => creatures_near(map, at, p1, p2 as f32, step.command != 71)
            .into_iter()
            .find(|g| *g != source),
        11 => creature_of_spawn(map, p1),
        12 => crate::game::map::instance_script::guid_data(map, p1),
        13 => objects_near(map, at, p1, p2 as f32).into_iter().next(),
        14 => object_of_spawn(map, p1),
        16..=20 => {
            let radius = if p1 > 0 { p1 as f32 } else { 30.0 };
            let mut friends: Vec<Guid> = map
                .grid
                .around(at.x, at.y, radius)
                .filter(|g| guid::is_unit(*g) || guid::is_player(*g))
                .filter(|g| {
                    alive(map, *g)
                        && map.position_of(*g).is_some_and(|p| dist(p, at) <= radius)
                        && !crate::game::map::combat::can_attack(map, source, *g)
                })
                .collect();
            if matches!(step.target_type, 18 | 20) {
                friends.retain(|g| *g != source);
            }
            match step.target_type {
                17 | 18 => friends.into_iter().find(|g| {
                    map.unit_fields(*g).is_some_and(|f| {
                        let max = f.get_u32(UNIT_FIELD_MAXHEALTH).max(1);
                        f.get_u32(UNIT_FIELD_HEALTH) * 100 / max < 100u32.saturating_sub(p2)
                    })
                }),
                19 | 20 => friends.into_iter().find(|g| {
                    !crate::game::map::auras::auras(map, *g).is_some_and(|a| a.has_spell(p2))
                }),
                _ => pick(friends, &mut rng),
            }
        }
        25 => players_near(map, at, p1 as f32).into_iter().next(),
        26 => players_near(map, at, p1 as f32)
            .into_iter()
            .find(|g| crate::game::map::combat::hostile(map, source, *g)),
        27 => players_near(map, at, p1 as f32)
            .into_iter()
            .find(|g| !crate::game::map::combat::hostile(map, source, *g)),
        28 => pick(
            creatures_near(map, at, p1, p2 as f32, true)
                .into_iter()
                .filter(|g| *g != source)
                .collect(),
            &mut rng,
        ),
        29 => pick(objects_near(map, at, p1, p2 as f32), &mut rng),
        _ => None,
    }
}

/// The name a creature speaks with.
fn speaker_name(map: &Map, g: Guid) -> String {
    if let Some(c) = map.creature(g) {
        return map
            .realm
            .data
            .creature_template(c.entry)
            .map(|t| t.name.clone())
            .unwrap_or_default();
    }
    if let Some(o) = map.gameobject(g) {
        return map
            .realm
            .data
            .gameobject_templates
            .get(&o.entry)
            .map(|t| t.name.clone())
            .unwrap_or_default();
    }
    map.players
        .get(&g)
        .map(|p| p.name.to_string())
        .unwrap_or_default()
}

/// `$N` and the like in a line, for its target.
fn fill(map: &Map, text: &str, target: Guid) -> String {
    if !text.contains('$') {
        return text.to_owned();
    }
    let Some(p) = map.players.get(&target) else {
        return text.to_owned();
    };
    let race = match p.row.race {
        1 => "Human",
        2 => "Orc",
        3 => "Dwarf",
        4 => "Night Elf",
        5 => "Undead",
        6 => "Tauren",
        7 => "Gnome",
        8 => "Troll",
        _ => "",
    };
    let class = match p.row.class {
        1 => "Warrior",
        2 => "Paladin",
        3 => "Hunter",
        4 => "Rogue",
        5 => "Priest",
        7 => "Shaman",
        8 => "Mage",
        9 => "Warlock",
        11 => "Druid",
        _ => "",
    };
    text.replace("$N", &p.name)
        .replace("$n", &p.name)
        .replace("$R", race)
        .replace("$r", &race.to_lowercase())
        .replace("$C", class)
        .replace("$c", &class.to_lowercase())
}

/// Chat types of `broadcast_text`.
pub const CHAT_SAY: u32 = 0;
pub const CHAT_YELL: u32 = 1;
pub const CHAT_TEXT_EMOTE: u32 = 2;
pub const CHAT_BOSS_EMOTE: u32 = 3;
pub const CHAT_WHISPER: u32 = 4;
pub const CHAT_BOSS_WHISPER: u32 = 5;
pub const CHAT_ZONE_YELL: u32 = 6;

/// `speaker` says broadcast text `text` (to `target`, for a whisper or a
/// name), with the text's emote and sound.
pub fn talk(map: &mut Map, speaker: Guid, text: u32, target: Guid, chat_type: Option<u32>) {
    let data = map.realm.data.clone();
    let Some(b) = data.npc.broadcast_texts.get(&text) else {
        return;
    };
    let female = map
        .unit_fields(speaker)
        .is_some_and(|f| f.get_byte(UNIT_FIELD_BYTES_0, 2) == 1);
    let line = if female && !b.female.is_empty() {
        &b.female
    } else if !b.male.is_empty() {
        &b.male
    } else {
        &b.female
    };
    let line = fill(map, line, target);
    say(
        map,
        speaker,
        &line,
        chat_type.unwrap_or(b.chat_type),
        b.language,
        target,
    );
    if b.emotes[0].1 != 0
        && let Ok(emote) = Emote::try_from(b.emotes[0].1)
        && let Some(frame) = crate::game::conn::encode(&SMSG_EMOTE {
            emote,
            guid: speaker,
        })
    {
        map.send_to_observers(speaker, &frame, false);
    }
    if b.sound != 0
        && let Some(frame) = crate::game::conn::encode(&SMSG_PLAY_SOUND { sound_id: b.sound })
    {
        hearers(map, speaker, chat_type.unwrap_or(b.chat_type), target)
            .into_iter()
            .for_each(|g| {
                if let Some(p) = map.players.get(&g) {
                    p.outbound.send_frame(frame.clone());
                }
            });
    }
}

/// Who hears a creature's line of `chat_type`.
fn hearers(map: &Map, speaker: Guid, chat_type: u32, target: Guid) -> Vec<Guid> {
    let Some(at) = map.position_of(speaker) else {
        return Vec::new();
    };
    match chat_type {
        CHAT_WHISPER | CHAT_BOSS_WHISPER => vec![target],
        CHAT_BOSS_EMOTE | CHAT_ZONE_YELL => map.players.keys().copied().collect(),
        CHAT_YELL => map
            .players
            .values()
            .filter(|p| dist(p.position(), at) <= 300.0)
            .map(|p| p.guid)
            .collect(),
        _ => map
            .players
            .values()
            .filter(|p| dist(p.position(), at) <= 25.0)
            .map(|p| p.guid)
            .collect(),
    }
}

/// `speaker` says `line`.
pub fn say(map: &mut Map, speaker: Guid, line: &str, chat_type: u32, language: u32, target: Guid) {
    let name = speaker_name(map, speaker);
    let kind = match chat_type {
        CHAT_YELL | CHAT_ZONE_YELL => SMSG_MESSAGECHAT_ChatType::MonsterYell {
            sender1: speaker,
            sender_name: name,
            target,
        },
        CHAT_TEXT_EMOTE => SMSG_MESSAGECHAT_ChatType::MonsterEmote {
            monster: speaker,
            monster_name: name,
        },
        CHAT_BOSS_EMOTE => SMSG_MESSAGECHAT_ChatType::RaidBossEmote {
            monster: speaker,
            monster_name: name,
        },
        CHAT_WHISPER | CHAT_BOSS_WHISPER => SMSG_MESSAGECHAT_ChatType::MonsterWhisper {
            monster: speaker,
            monster_name: name,
        },
        _ => SMSG_MESSAGECHAT_ChatType::MonsterSay {
            sender1: speaker,
            sender_name: name,
            target,
        },
    };
    let Some(frame) = crate::game::conn::encode(&SMSG_MESSAGECHAT {
        chat_type: kind,
        language: Language::try_from(language).unwrap_or(Language::Universal),
        message: line.to_owned(),
        tag: PlayerChatTag::None,
    }) else {
        return;
    };
    for g in hearers(map, speaker, chat_type, target) {
        if let Some(p) = map.players.get(&g) {
            p.outbound.send_frame(frame.clone());
        }
    }
}

/// A creature summoned by `summoner` at `at`.
pub fn summon(
    map: &mut Map,
    summoner: Guid,
    entry: u32,
    at: Vector3d,
    orientation: f32,
    despawn: Option<(Duration, i32)>,
) -> Option<Guid> {
    let data = map.realm.data.clone();
    let template = data.creature_template(entry)?;
    let level = if template.level_max > template.level_min {
        rand::rng().random_range(template.level_min..=template.level_max)
    } else {
        template.level_min
    }
    .max(1);
    let counter = map.next_creature_counter;
    map.next_creature_counter += 1;
    let mut c = Creature::new(
        guid::unit(entry, counter | 0x80_0000),
        template,
        level,
        at,
        orientation,
    );
    if let Some(stats) = data.class_level_stats.get(&(template.unit_class, level)) {
        let health =
            crate::game::data::load::creature_health(stats.health, template.health_multiplier);
        c.fields.set_u32(UNIT_FIELD_MAXHEALTH, health);
        c.fields.set_u32(UNIT_FIELD_HEALTH, health);
        let mult = if template.damage_multiplier > 0.0 {
            template.damage_multiplier
        } else {
            1.0
        };
        let base = stats.melee_damage * mult;
        c.fields
            .set_f32(UNIT_FIELD_MINDAMAGE, (base * 0.86).max(1.0));
        c.fields
            .set_f32(UNIT_FIELD_MAXDAMAGE, (base * 1.14).max(1.0));
        c.fields
            .set_u32(UNIT_FIELD_ATTACK_POWER, stats.attack_power);
        c.fields.set_u32(UNIT_FIELD_RESISTANCES, stats.armor);
    }
    c.hostile_to_players =
        crate::game::map::ai::hostile_to_players(&data, template.faction, template.civilian);
    let mut c = c.with_base();
    let s = c.scripted.get_or_insert_with(Default::default);
    s.summoner = Some(summoner);
    if let Some((after, kind)) = despawn {
        s.despawn_at = Some(map.now + after);
        s.despawn_type = kind;
    }
    let g = map.add_creature(c);
    if let Some(s) = event_ai::scripted_mut(map, summoner) {
        s.summoned.push(g);
    }
    event_ai::summoned_unit(map, summoner, g);
    Some(g)
}

/// A creature goes: a summoned one for good, a spawned one until it
/// respawns.
pub fn despawn(map: &mut Map, g: Guid) {
    let Some(c) = map.creature(g) else {
        return;
    };
    let (spawn, entry) = (c.spawn_guid, c.entry);
    let summoner = c.scripted.as_deref().and_then(|s| s.summoner);
    crate::game::map::combat::leave_combat(map, g);
    stop_for(map, g);
    match spawn {
        None => {
            map.remove_creature(g);
        }
        Some(_) => {
            crate::game::map::ai::quiet_death(map, g);
        }
    }
    if let Some(s) = summoner {
        event_ai::summoned_despawned(map, s, g, entry);
    }
}

fn creature_mut(map: &mut Map, g: Guid) -> Option<&mut Creature> {
    map.creature_mut(g)
}

/// Move a creature to `to`, on the navmesh if there is one.
pub fn move_to(map: &mut Map, g: Guid, to: Vector3d, run: bool) {
    let now = map.now;
    let spline_id = map.next_spline_id;
    let Some(from) = map.creature(g).map(|c| c.position) else {
        return;
    };
    let path = match map.nav.as_mut() {
        Some(nav) => nav.path(from, to).unwrap_or_else(|| vec![from, to]),
        None => vec![from, to],
    };
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    if let Some(frame) = c.walk(path, run, now, spline_id) {
        map.send_to_observers(g, &frame, false);
    }
}

/// Set a game object's state (0 open, 1 closed, 2 active), with a reset
/// after `reset` if it has one.
pub fn set_object_state(map: &mut Map, object: Guid, state: u32) {
    if let Some(o) = map.gameobject_mut(object) {
        o.fields.set_u32(GAMEOBJECT_STATE, state);
    }
}

/// One command, from `source` on `target`. False if it couldn't be done.
fn command(map: &mut Map, step: &ScriptStep, source: Guid, target: Guid) -> bool {
    let data = map.realm.data.clone();
    let now = map.now;
    let [d1, d2, d3, d4] = step.datalong;
    let pos = Vector3d {
        x: step.x,
        y: step.y,
        z: step.z,
    };
    match step.command {
        // Talk: one of up to four broadcast texts.
        0 => {
            let texts: Vec<u32> = step
                .dataint
                .iter()
                .filter(|t| **t > 0)
                .map(|t| *t as u32)
                .collect();
            if texts.is_empty() {
                return false;
            }
            let text = texts[rand::rng().random_range(0..texts.len())];
            talk(map, source, text, target, None);
            true
        }
        // Emote: one of up to four.
        1 => {
            let emotes: Vec<u32> = std::iter::once(d1)
                .chain([d2, d3, d4].into_iter().take_while(|e| *e > 0))
                .collect();
            let e = emotes[rand::rng().random_range(0..emotes.len())];
            let Ok(emote) = Emote::try_from(e) else {
                return false;
            };
            if let Some(frame) = crate::game::conn::encode(&SMSG_EMOTE {
                emote,
                guid: source,
            }) {
                map.send_to_observers(source, &frame, true);
            }
            true
        }
        // Move to a point (or, with datalong4 & 1, the target).
        3 => {
            let to = if d3 & 0x2 != 0 || (step.x == 0.0 && step.y == 0.0 && step.z == 0.0) {
                map.position_of(target).unwrap_or(pos)
            } else {
                pos
            };
            let run = event_ai::scripted(map, source).is_some_and(|s| s.run) || d4 & 0x1 != 0;
            move_to(map, source, to, run);
            if step.o != 0.0
                && let Some(c) = creature_mut(map, source)
            {
                c.home_orientation = step.o;
            }
            true
        }
        // Modify a field's flags: toggle, set or clear.
        4 => {
            let field = d1 as u16;
            let Some(f) = map.unit_fields_mut(target) else {
                return false;
            };
            let old = f.get_u32(field);
            let new = match d3 {
                1 => old | d2,
                2 => old & !d2,
                _ => old ^ d2,
            };
            f.set_u32(field, new);
            true
        }
        5 => {
            crate::game::map::spells::interrupt(map, target);
            true
        }
        // Teleport a player.
        6 => {
            if !guid::is_player(target) {
                if let Some(c) = creature_mut(map, target) {
                    c.position = pos;
                    c.orientation = step.o;
                }
                return true;
            }
            map.teleport(target, MapKey::world(d1), pos, step.o);
            true
        }
        // A quest explored by a player near enough.
        7 => {
            let quest = d1;
            let near = d2 == 0
                || map
                    .position_of(source)
                    .zip(map.position_of(target))
                    .is_some_and(|(a, b)| dist(a, b) <= d2 as f32);
            if near {
                quest_explored(map, target, quest);
            }
            near
        }
        // Kill credit, to the player or their group.
        8 => {
            let credit = guid::unit(d1, 0);
            let players: Vec<Guid> = if d2 != 0 {
                group_here(map, target)
            } else {
                vec![target]
            };
            for p in players {
                crate::game::map::quests::killed(map, p, credit);
            }
            true
        }
        // Respawn an object.
        9 => {
            let o = object_of_spawn(map, d1).or(Some(target).filter(|t| guid::is_game_object(*t)));
            if let Some(o) = o {
                set_object_state(map, o, 1);
                map.visibility_dirty.insert(o);
            }
            o.is_some()
        }
        // Summon a creature.
        10 => {
            let at = if step.x == 0.0 && step.y == 0.0 && step.z == 0.0 {
                map.position_of(source).unwrap_or(pos)
            } else {
                pos
            };
            let flags = step.dataint[0] as u32;
            // Unique: not if one is already near.
            if flags & 0xC != 0
                && !creatures_near(map, at, d1, if d4 > 0 { d4 as f32 } else { 100.0 }, true)
                    .is_empty()
            {
                return false;
            }
            let despawn = (d2 > 0).then(|| {
                (
                    Duration::from_millis(u64::from(d2)),
                    if step.dataint[3] > 0 {
                        step.dataint[3]
                    } else {
                        1
                    },
                )
            });
            let Some(g) = summon(map, source, d1, at, step.o, despawn) else {
                return false;
            };
            if flags & 0x1 != 0
                && let Some(s) = event_ai::scripted_mut(map, g)
            {
                s.run = true;
            }
            if step.dataint[1] > 0 {
                start(map, ScriptTable::Generic, step.dataint[1] as u32, g, target);
            }
            // An attack target by target type; -1 for none.
            if step.dataint[2] >= 0 {
                let attack = ScriptStep {
                    target_type: step.dataint[2] as u32,
                    ..Default::default()
                };
                if let Some(t) = find_target(map, &attack, source, target)
                    && (guid::is_player(t) || guid::is_unit(t))
                    && crate::game::map::combat::can_attack(map, g, t)
                {
                    crate::game::map::ai::attacked(map, g, t);
                }
            }
            true
        }
        // Open or close a door.
        11 | 12 => {
            let door = object_of_spawn(map, d1)
                .or_else(|| (guid::is_game_object(target)).then_some(target))
                .or_else(|| (guid::is_game_object(source)).then_some(source));
            let Some(door) = door else {
                return false;
            };
            set_object_state(map, door, if step.command == 11 { 0 } else { 1 });
            if d2 > 0 {
                let back = if step.command == 11 { 1 } else { 0 };
                map.object_resets
                    .push((map.now + Duration::from_secs(u64::from(d2)), door, back));
            }
            true
        }
        // Use an object.
        13 => {
            if guid::is_game_object(target) {
                let state = map
                    .gameobject(target)
                    .map(|o| o.fields.get_u32(GAMEOBJECT_STATE))
                    .unwrap_or(1);
                set_object_state(map, target, if state == 0 { 1 } else { 0 });
                true
            } else {
                false
            }
        }
        14 => {
            crate::game::map::auras::remove_spell(map, target, d1, None);
            true
        }
        // Cast.
        15 => {
            let flags = d2;
            let (caster, aim) = if flags & 0x80 != 0 {
                (target, source)
            } else {
                (source, target)
            };
            let aim = if flags & 0x10 != 0 { caster } else { aim };
            cast(map, caster, aim, d1, flags)
        }
        16 => {
            if let Some(frame) = crate::game::conn::encode(&SMSG_PLAY_SOUND { sound_id: d1 }) {
                if d2 & 0x1 != 0 {
                    if let Some(p) = map.players.get(&target) {
                        p.outbound.send_frame(frame);
                    }
                } else {
                    map.send_to_observers(source, &frame, true);
                }
            }
            true
        }
        17 => crate::game::map::inventory::add_item(map, target, d1, d2.max(1), true).is_ok(),
        18 => {
            let g = if guid::is_unit(target) && d2 != 0 {
                target
            } else {
                source
            };
            if d1 > 0 {
                if let Some(s) = event_ai::scripted_mut(map, g) {
                    s.despawn_at = Some(now + Duration::from_millis(u64::from(d1)));
                    s.despawn_type = 1;
                }
            } else {
                despawn(map, g);
            }
            true
        }
        19 => {
            let items =
                [step.dataint[0], step.dataint[1], step.dataint[2]].map(|i| i.max(0) as u32);
            let items = if d1 != 0 {
                data.creature_template(map.creature(source).map_or(0, |c| c.entry))
                    .and_then(|t| data.creature_equipment.get(&t.equipment_id))
                    .copied()
                    .unwrap_or([0; 3])
            } else {
                items
            };
            if let Some(c) = creature_mut(map, source) {
                c.set_equipment(&data, items);
            }
            true
        }
        // Movement type: 0 idle, 1 random, 2 waypoint.
        20 => {
            if let Some(c) = creature_mut(map, source) {
                c.movement_type = d1 as u8;
                if d1 == 1 && d2 > 0 {
                    c.wander_distance = d2 as f32;
                }
                if d1 == 0 {
                    c.motion = Motion::Idle { until: None };
                } else {
                    c.motion = Motion::Idle { until: Some(now) };
                }
            }
            true
        }
        // Faction: set, or (0) the template's.
        22 => {
            let faction = if d1 == 0 {
                map.creature(source)
                    .and_then(|c| data.creature_template(c.entry))
                    .map_or(0, |t| t.faction)
            } else {
                d1
            };
            let Some(c) = creature_mut(map, source) else {
                return false;
            };
            c.fields.set_u32(UNIT_FIELD_FACTIONTEMPLATE, faction);
            let civilian = data.creature_template(c.entry).is_some_and(|t| t.civilian);
            c.hostile_to_players =
                crate::game::map::ai::hostile_to_players(&data, faction, civilian);
            true
        }
        // Morph to an entry's look (datalong2 0) or a display (1); 0 back.
        23 | 24 => {
            let display = if d1 == 0 {
                0
            } else if d2 == 0 {
                data.creature_template(d1)
                    .and_then(|t| t.display_ids.iter().copied().find(|d| *d != 0))
                    .unwrap_or(0)
            } else {
                d1
            };
            let Some(f) = map.unit_fields_mut(source) else {
                return false;
            };
            if step.command == 23 {
                let native = f.get_u32(UNIT_FIELD_NATIVEDISPLAYID);
                f.set_u32(
                    UNIT_FIELD_DISPLAYID,
                    if display == 0 { native } else { display },
                );
            } else {
                f.set_u32(UNIT_FIELD_MOUNTDISPLAYID, display);
            }
            true
        }
        25 => {
            if let Some(s) = event_ai::scripted_mut(map, source) {
                s.run = d1 != 0;
            }
            true
        }
        26 => {
            if guid::is_unit(source) && alive(map, target) {
                crate::game::map::ai::attacked(map, source, target);
                true
            } else {
                false
            }
        }
        // Become another entry: its look and faction.
        27 => {
            let Some(t) = data.creature_template(d1) else {
                return false;
            };
            let display = t.display_ids.iter().copied().find(|d| *d != 0).unwrap_or(0);
            let Some(c) = creature_mut(map, source) else {
                return false;
            };
            c.entry = d1;
            c.fields.set_u32(OBJECT_FIELD_ENTRY, d1);
            if display != 0 {
                c.fields.set_u32(UNIT_FIELD_DISPLAYID, display);
                c.fields.set_u32(UNIT_FIELD_NATIVEDISPLAYID, display);
            }
            if d2 == 0 {
                c.fields.set_u32(UNIT_FIELD_FACTIONTEMPLATE, t.faction);
            }
            c.fields.set_u32(UNIT_NPC_FLAGS, t.npc_flags);
            true
        }
        28 => {
            let g = if d2 != 0 { target } else { source };
            if let Some(p) = map.players.get_mut(&g) {
                p.set_stand_state(d1 as u8);
            } else if let Some(f) = map.unit_fields_mut(g) {
                f.set_byte(UNIT_FIELD_BYTES_1, 0, d1 as u8);
            }
            true
        }
        // Threat: by a percentage, on the target or everyone.
        29 => {
            let pct = step.dataint[0];
            if let Some(c) = creature_mut(map, source) {
                for (t, v) in c.threat.iter_mut() {
                    if d1 != 0 || *t == target {
                        *v = (*v * (100 + pct) as f32 / 100.0).max(0.0);
                    }
                }
            }
            true
        }
        // End the script, if a creature is (or, with dataint, isn't) near.
        31 => {
            if d1 == 0 {
                return false;
            }
            let at = map.position_of(source).unwrap_or(pos);
            let found = !creatures_near(map, at, d1, d2 as f32, true).is_empty();
            let end = if step.dataint[0] != 0 { !found } else { found };
            !end
        }
        // End the script if a condition holds.
        32 => !crate::game::map::conditions::met(map, d1, Some(source), Some(target)),
        33 => {
            crate::game::map::ai::enter_evade(map, source);
            true
        }
        34 => {
            let here = map.creature(source).map(|c| (c.position, c.orientation));
            if let Some(c) = creature_mut(map, source) {
                if d1 != 0 || (step.x == 0.0 && step.y == 0.0) {
                    if let Some((p, o)) = here {
                        c.home = p;
                        c.home_orientation = o;
                    }
                } else {
                    c.home = pos;
                    c.home_orientation = step.o;
                }
            }
            true
        }
        // Turn to the target, or to the step's orientation.
        35 => {
            let o = if d1 == 0 {
                step.o
            } else {
                match (map.position_of(source), map.position_of(target)) {
                    (Some(a), Some(b)) => (b.y - a.y).atan2(b.x - a.x),
                    _ => step.o,
                }
            };
            if let Some(c) = creature_mut(map, source) {
                c.orientation = o;
                map.visibility_dirty.insert(source);
            }
            true
        }
        // Instance data.
        37 => {
            crate::game::map::instance_script::set_data(map, d1, d2, d3);
            true
        }
        // Start one of up to four generic scripts, by chance.
        39 => {
            let options: Vec<(u32, i32)> = (0..4)
                .map(|i| (step.datalong[i], step.dataint[i]))
                .filter(|(s, _)| *s > 0)
                .collect();
            if options.is_empty() {
                return false;
            }
            let total: i32 = options.iter().map(|(_, c)| (*c).max(0)).sum();
            let script = if total <= 0 {
                options[rand::rng().random_range(0..options.len())].0
            } else {
                let mut r = rand::rng().random_range(0..total.max(1).max(100));
                let mut chosen = None;
                for (s, c) in &options {
                    if r < *c {
                        chosen = Some(*s);
                        break;
                    }
                    r -= c;
                }
                match chosen {
                    Some(s) => s,
                    None => return true,
                }
            };
            start(map, ScriptTable::Generic, script, source, target);
            true
        }
        40 => {
            let mut left = d2.max(1);
            while left > 0 {
                let Some(i) = map
                    .players
                    .get(&target)
                    .and_then(|p| p.items.iter().position(|it| it.entry == d1))
                else {
                    break;
                };
                let n = map.players[&target].items[i].count().min(left);
                crate::game::map::inventory::take_item(map, target, i, n);
                left -= n;
            }
            true
        }
        41 | 81 => {
            let o = if guid::is_game_object(target) {
                target
            } else {
                source
            };
            map.remove_gameobject(o).is_some()
        }
        42 => {
            if let Some(s) = event_ai::scripted_mut(map, source) {
                s.no_melee = d1 == 0;
            }
            if d1 == 0 {
                crate::game::map::combat::stop_attack(map, source);
            }
            true
        }
        43 => {
            if let Some(s) = event_ai::scripted_mut(map, source) {
                s.no_combat_movement = d1 == 0;
            }
            true
        }
        44 => {
            event_ai::set_phase(map, source, d1);
            true
        }
        45 => {
            let phases: Vec<u32> = step.datalong.iter().copied().filter(|p| *p > 0).collect();
            if !phases.is_empty() {
                let p = phases[rand::rng().random_range(0..phases.len())];
                event_ai::set_phase(map, source, p);
            }
            true
        }
        46 => {
            let (lo, hi) = (d1, d2.max(d1));
            event_ai::set_phase(map, source, rand::rng().random_range(lo..=hi));
            true
        }
        // Flee: run from the victim for a while.
        47 => {
            crate::game::map::ai::flee(map, source);
            true
        }
        // Damage the target: an amount or a percentage of its health.
        48 => {
            let amount = if d2 != 0 {
                map.unit_fields(target)
                    .map_or(0, |f| f.get_u32(UNIT_FIELD_MAXHEALTH) * d1 / 100)
            } else {
                d1
            };
            crate::game::map::combat::deal_damage(map, source, target, amount);
            true
        }
        // Everyone in the zone (here: the map) fights.
        49 => {
            let players: Vec<Guid> = map.players.keys().copied().collect();
            for p in players {
                crate::game::map::ai::attacked(map, source, p);
            }
            true
        }
        50 => {
            let radius = d1 as f32;
            crate::game::map::ai::call_for_help_radius(map, source, radius);
            true
        }
        51 => {
            if let Some(f) = map.unit_fields_mut(source) {
                f.set_byte(UNIT_FIELD_BYTES_2, 0, d1 as u8);
            }
            true
        }
        // Invincibility: a floor of health, flat or a percentage.
        52 => {
            let floor = if d2 != 0 {
                map.unit_fields(source)
                    .map_or(0, |f| f.get_u32(UNIT_FIELD_MAXHEALTH) * d1 / 100)
            } else {
                d1
            };
            if let Some(s) = event_ai::scripted_mut(map, source) {
                s.invincible_hp = floor;
            }
            true
        }
        // A creature spell list, from up to four by chance.
        55 => {
            let options: Vec<(u32, i32)> = (0..4)
                .map(|i| (step.datalong[i], step.dataint[i]))
                .filter(|(_, c)| *c > 0)
                .collect();
            let list = if options.is_empty() {
                d1
            } else {
                options[rand::rng().random_range(0..options.len())].0
            };
            if let Some(s) = event_ai::scripted_mut(map, source) {
                s.spell_list = Some(list);
            }
            if let Some(c) = creature_mut(map, source) {
                c.caster.next_spell_at = None;
            }
            true
        }
        56 => {
            let summoned = event_ai::scripted(map, source)
                .map(|s| s.summoned.clone())
                .unwrap_or_default();
            for g in summoned {
                if map.creature(g).is_some_and(|c| d1 == 0 || c.entry == d1) {
                    despawn(map, g);
                }
            }
            true
        }
        // React state: 0 passive, 1 defensive, 2 aggressive.
        59 => {
            if let Some(s) = event_ai::scripted_mut(map, source) {
                s.passive = d1 == 0;
            }
            if let Some(c) = creature_mut(map, source) {
                let template_hostile = data
                    .creature_template(c.entry)
                    .map(|t| {
                        crate::game::map::ai::hostile_to_players(
                            &data,
                            c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
                            t.civilian,
                        )
                    })
                    .unwrap_or(false);
                c.hostile_to_players = d1 == 2 || (d1 != 0 && template_hostile);
            }
            true
        }
        // Waypoints, again.
        60 => {
            if let Some(c) = creature_mut(map, source) {
                c.movement_type = crate::game::map::creature::MOVEMENT_WAYPOINT;
                c.waypoint = d2 as usize;
                c.motion = Motion::Idle { until: Some(now) };
            }
            true
        }
        // Default movement.
        67 => {
            if let Some(c) = creature_mut(map, source) {
                c.movement_type = d1 as u8;
                c.motion = Motion::Idle { until: Some(now) };
            }
            true
        }
        // A script for every player here.
        68 => {
            let players: Vec<Guid> = map.players.keys().copied().collect();
            for p in players {
                start(map, ScriptTable::Generic, d1, source, p);
            }
            true
        }
        70 => {
            if let Some(p) = map.players.get_mut(&target)
                && let Some(slot) = p.journal.slot_of(d1)
            {
                p.journal.log[slot].failed = true;
            }
            crate::game::map::quests::check(map, target, d1);
            true
        }
        71 => {
            let g = if guid::is_unit(target) {
                target
            } else {
                source
            };
            crate::game::map::ai::respawn_now(map, g);
            true
        }
        73 => {
            crate::game::map::combat::leave_combat(map, source);
            if let Some(c) = creature_mut(map, source) {
                c.threat.clear();
            }
            true
        }
        74 => {
            crate::game::map::spells::cast_triggered(map, target, d1, Some(target));
            true
        }
        // Summon an object.
        76 => {
            let Some(t) = data.gameobject_templates.get(&d1) else {
                return false;
            };
            let counter = map.next_creature_counter;
            map.next_creature_counter += 1;
            let mut go = crate::game::map::gameobject::GameObject::from_spawn(
                t,
                &crate::game::data::GameObjectSpawn {
                    guid: counter | 0x80_0000,
                    entry: d1,
                    map: map.key.map,
                    x: step.x,
                    y: step.y,
                    z: step.z,
                    orientation: step.o,
                    rotation: [0.0, 0.0, (step.o / 2.0).sin(), (step.o / 2.0).cos()],
                    state: 1,
                    anim_progress: 100,
                    ..Default::default()
                },
            );
            go.spawn_guid = None;
            let g = map.add_gameobject(go);
            if d2 > 0 {
                map.object_resets
                    .push((map.now + Duration::from_secs(u64::from(d2)), g, u32::MAX));
            }
            true
        }
        78 => {
            crate::game::map::creature_groups::join(map, source, target, d1 as f32, d2 as f32);
            true
        }
        79 => {
            crate::game::map::creature_groups::leave(map, source);
            true
        }
        80 => {
            let o = if guid::is_game_object(target) {
                target
            } else {
                source
            };
            set_object_state(map, o, d1);
            true
        }
        // Quest credit: kill credit or exploration.
        83 => {
            if d2 != 0 {
                quest_explored(map, target, d1);
            } else {
                let credit = guid::unit(d1, 0);
                crate::game::map::quests::killed(map, target, credit);
            }
            true
        }
        84 => {
            if let Some(s) = event_ai::scripted_mut(map, source) {
                s.gossip_menu = Some(d1);
            }
            true
        }
        85 => {
            let g = if guid::is_unit(target) {
                target
            } else {
                source
            };
            event_ai::script_event(map, g, d1, d2, Some(source));
            true
        }
        87 => {
            let o = object_of_spawn(map, d1).unwrap_or(target);
            set_object_state(map, o, 1);
            true
        }
        89 => {
            if let Ok(emote) = Emote::try_from(d1)
                && let Some(frame) = crate::game::conn::encode(&SMSG_EMOTE {
                    emote,
                    guid: source,
                })
            {
                map.send_to_observers(source, &frame, true);
            }
            true
        }
        // A script for each of a creature group, or everyone in the zone.
        90 => {
            let members = crate::game::map::creature_groups::members_of(map, source);
            for m in members {
                start(map, ScriptTable::Generic, d1, m, target);
            }
            true
        }
        92 => {
            let players: Vec<Guid> = map.players.keys().copied().collect();
            for p in players {
                start(map, ScriptTable::Generic, d1, source, p);
            }
            true
        }
        // Map events, server variables, meeting stones and the like are
        // left alone.
        _ => true,
    }
}

/// `player`'s group members in the map, or the player alone.
fn group_here(map: &Map, player: Guid) -> Vec<Guid> {
    let groups = map.realm.social.groups.lock();
    match groups.group_of(player) {
        Some(g) => g
            .members
            .iter()
            .map(|m| m.guid)
            .filter(|m| map.players.contains_key(m))
            .collect(),
        None => vec![player],
    }
}

/// A quest's exploration objective met for `player`.
pub fn quest_explored(map: &mut Map, player: Guid, quest: u32) {
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let Some(slot) = p.journal.slot_of(quest) else {
        return;
    };
    p.journal.log[slot].explored = true;
    crate::game::map::quests::check(map, player, quest);
}

/// A script's cast, with vmangos's cast flags: 0x1 interrupts the cast
/// under way, 0x2 is free and instant, 0x20 only if the aura isn't there.
pub fn cast(map: &mut Map, caster: Guid, target: Guid, spell: u32, flags: u32) -> bool {
    if spell == 0 {
        return false;
    }
    let target = if target.is_zero() { caster } else { target };
    if flags & 0x20 != 0
        && crate::game::map::auras::auras(map, target).is_some_and(|a| a.has_spell(spell))
    {
        return false;
    }
    let casting = map.caster(caster).is_some_and(|c| c.casting.is_some());
    if casting && flags & 0x1 == 0 && flags & 0x2 == 0 {
        return false;
    }
    if casting && flags & 0x1 != 0 {
        crate::game::map::spells::interrupt(map, caster);
    }
    if flags & 0x2 != 0 || guid::is_game_object(caster) {
        crate::game::map::spells::cast_triggered(map, caster, spell, Some(target));
    } else {
        crate::game::map::spells::cast(
            map,
            caster,
            spell,
            crate::game::map::spells::Targets {
                unit: Some(target),
                ..Default::default()
            },
            None,
        );
    }
    true
}

/// Objects whose state a script changed for a while go back.
pub fn update_objects(map: &mut Map) {
    if map.object_resets.is_empty() {
        return;
    }
    let now = map.now;
    let due: Vec<(Instant, Guid, u32)> = map
        .object_resets
        .iter()
        .copied()
        .filter(|(t, _, _)| *t <= now)
        .collect();
    map.object_resets.retain(|(t, _, _)| *t > now);
    for (_, g, state) in due {
        if state == u32::MAX {
            map.remove_gameobject(g);
        } else {
            set_object_state(map, g, state);
        }
    }
}

/// The scripts of a game object's use: `gameobject_scripts` for its spawn,
/// and the event its template sends.
pub fn object_used(map: &mut Map, player: Guid, object: Guid) {
    let Some(o) = map.gameobject(object) else {
        return;
    };
    let (spawn, entry, kind) = (o.spawn_guid, o.entry, o.kind);
    if let Some(s) = spawn {
        start(map, ScriptTable::GameObject, s, object, player);
    }
    let event = map
        .realm
        .data
        .gameobject_templates
        .get(&entry)
        .map(|t| match kind {
            // Door and button: data 3 (vmangos's linked event); goober:
            // data 2; camera: data 1; spell caster ... none.
            0 | 1 => 0,
            10 => t.data[2],
            13 => t.data[1],
            _ => 0,
        })
        .unwrap_or(0);
    if event != 0 {
        start(map, ScriptTable::Event, event, player, object);
    }
    crate::game::map::instance_script::object_used(map, entry);
}
