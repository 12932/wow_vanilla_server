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

/// Thistle Tea's commands, which vmangos has no number for.
pub const C_STOP_SCRIPTS: u32 = 200;
pub const C_HOLD_WAYPOINTS: u32 = 201;
pub const C_RELEASE_WAYPOINTS: u32 = 202;
pub const C_SET_REGENERATION: u32 = 203;
pub const C_SET_CASTER_CHASE_DISTANCE: u32 = 204;
pub const C_SET_HEALTH_PCT: u32 = 205;
pub const C_LOSE_HEALTH: u32 = 206;
pub const C_SET_CONCEALED: u32 = 207;
pub const C_CAST_CREDIT: u32 = 208;
pub const C_CLEAR_AURAS: u32 = 209;
pub const C_LINKED_FEIGN_DEATH: u32 = 210;
pub const C_SET_MELEE_SPELL: u32 = 211;
pub const C_SET_CALL_FOR_HELP_RANGE: u32 = 212;
pub const C_BATTLEGROUND_EVENT: u32 = 213;
pub const C_TELEPORT_SELF: u32 = 214;

/// A flying creature's movement flags, as Thistle Tea sets them.
const FLIGHT_FLAGS: u32 = 0x0180_0000;

/// The commands that do something here (the rest are skipped, as
/// Thistle Tea skips them).
pub fn command_supported(command: u32) -> bool {
    matches!(
        command,
        0..=20
            | 22..=29
            | 31..=35
            | 37
            | 39..=52
            | 54..=56
            | 59..=71
            | 73
            | 74
            | 76..=85
            | 87
            | 89..=92
            | 21
            | 30
            | 36
            | 38
            | 53
            | 57
            | 58
            | 72
            | 75
            | 86
            | 88
            | C_STOP_SCRIPTS..=C_TELEPORT_SELF
    )
}

/// A script under way.
#[derive(Debug, Clone)]
pub struct Run {
    pub table: ScriptTable,
    pub id: u32,
    pub next: usize,
    pub started: Instant,
    pub source: Guid,
    pub target: Guid,
    /// The EventAI event (its creature and index) that checks this run's
    /// result: if it fails, the event may fire again at once.
    pub checked: Option<(Guid, usize)>,
}

/// The scripts running on a map.
#[derive(Debug, Default)]
pub struct Runner {
    pub runs: Vec<Run>,
}

/// Start script `id` of `table` from `source` on `target`; its first
/// steps with no delay run at once.
pub fn start(map: &mut Map, table: ScriptTable, id: u32, source: Guid, target: Guid) {
    start_checked(map, table, id, source, target, None);
}

/// [`start`], for an EventAI event that checks the result.
pub fn start_checked(
    map: &mut Map,
    table: ScriptTable,
    id: u32,
    source: Guid,
    target: Guid,
    checked: Option<(Guid, usize)>,
) {
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
        checked,
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
        if step.command == C_STOP_SCRIPTS {
            for (j, r) in map.scripts.runs.iter_mut().enumerate() {
                if j != i && r.source == run.source {
                    r.next = usize::MAX;
                }
            }
        }
        if !execute(map, step, run.source, run.target)
            && let Some(r) = map.scripts.runs.get_mut(i)
        {
            r.next = usize::MAX;
            if let Some((g, event)) = r.checked {
                event_ai::retry(map, g, event);
            }
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
    // With no target, a step's condition is judged on its source.
    let judged = if target.is_zero() { source } else { target };
    if step.condition != 0
        && !crate::game::map::conditions::met(map, step.condition, Some(source), Some(judged))
    {
        return step.data_flags & SF_ABORT_ON_FAILURE == 0;
    }
    if let Some(min) = step.ext.as_ref().and_then(|e| e.min_target_distance_2d)
        && !surface_distance_2d(map, source, target).is_some_and(|d| d > min)
    {
        return step.data_flags & SF_ABORT_ON_FAILURE == 0;
    }
    let ok = command(map, step, source, target);
    // The terminating commands end the script whatever its flags.
    if matches!(step.command, 31 | 32) {
        return ok;
    }
    ok || step.data_flags & SF_ABORT_ON_FAILURE == 0
}

/// The flat distance between two units' edges.
fn surface_distance_2d(map: &Map, a: Guid, b: Guid) -> Option<f32> {
    let (pa, pb) = (map.position_of(a)?, map.position_of(b)?);
    let radius = |g| {
        map.unit_fields(g)
            .map_or(0.0, |f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS))
    };
    let (dx, dy) = (pa.x - pb.x, pa.y - pb.y);
    Some(((dx * dx + dy * dy).sqrt() - radius(a) - radius(b)).max(0.0))
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

/// Thistle Tea's target type for one its owner fights.
pub const T_OWNER_HOSTILE_RANDOM: u32 = 200;

/// Whether a hostile target type may pick `g`: `target_param1`'s flags
/// (0x1 in sight, 0x2 or 0x200 a player, 0x4 a mana user), and for a
/// cast, the spell's range.
fn hostile_allowed(map: &Map, step: &ScriptStep, source: Guid, g: Guid) -> bool {
    let flags = step.target_param1;
    if flags & 0x1 != 0
        && let (Some(a), Some(b)) = (map.position_of(source), map.position_of(g))
        && !map.nav.as_ref().is_none_or(|n| {
            n.sight(
                Vector3d { z: a.z + 1.5, ..a },
                Vector3d { z: b.z + 1.5, ..b },
            )
        })
    {
        return false;
    }
    if flags & 0x202 != 0 && !guid::is_player(g) {
        return false;
    }
    if flags & 0x4 != 0
        && !map
            .unit_fields(g)
            .is_some_and(|f| f.get_byte(UNIT_FIELD_BYTES_0, 3) == 0)
    {
        return false;
    }
    if step.command == 15
        && let Some(s) = map.realm.data.spells.spells.get(&step.datalong[0])
    {
        let (_, max) = map.realm.data.spells.range(s);
        if max > 0.0
            && let (Some(a), Some(b)) = (map.position_of(source), map.position_of(g))
            && dist(a, b) > max + 5.0
        {
            return false;
        }
    }
    true
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
        2..=7 => {
            let mut t = threat_list(map, source);
            if matches!(step.target_type, 2 | 5) && !t.is_empty() {
                t.remove(0);
            }
            t.retain(|g| hostile_allowed(map, step, source, *g));
            match step.target_type {
                2 => t.first().copied(),
                3 => t.last().copied(),
                4 | 5 => pick(t, &mut rng),
                6 => t.into_iter().min_by(|a, b| {
                    let da = map.position_of(*a).map_or(f32::MAX, |p| dist(p, at));
                    let db = map.position_of(*b).map_or(f32::MAX, |p| dist(p, at));
                    da.total_cmp(&db)
                }),
                _ => t.into_iter().max_by(|a, b| {
                    let da = map.position_of(*a).map_or(0.0, |p| dist(p, at));
                    let db = map.position_of(*b).map_or(0.0, |p| dist(p, at));
                    da.total_cmp(&db)
                }),
            }
        }
        // Thistle Tea's: one at random its owner fights, not its own
        // victim.
        T_OWNER_HOSTILE_RANDOM => {
            let owner = map
                .creature(source)
                .and_then(|c| c.owner.or(c.scripted.as_deref().and_then(|s| s.summoner)))?;
            let victim = map.combat(source).map(|c| c.victim).unwrap_or_default();
            let mut t = map
                .combat(owner)
                .map(|c| c.attackers.iter().copied().collect::<Vec<_>>())
                .unwrap_or_default();
            if let Some(c) = map.creature(owner) {
                t.extend(c.threat.iter().map(|(g, _)| *g));
            }
            t.sort();
            t.dedup();
            t.retain(|g| *g != victim && alive(map, *g) && hostile_allowed(map, step, source, *g));
            pick(t, &mut rng)
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
        22..=24 => crate::game::map::map_events::target_of(map, step.target_type, p1, p2),
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
        // Its mana, as a spawn's, for its list's spells.
        let mana = (stats.mana as f32 * template.mana_multiplier.max(0.0)).round() as u32;
        if mana > 0 {
            c.fields.set_u32(UNIT_FIELD_MAXPOWER1, mana);
            c.fields.set_u32(UNIT_FIELD_POWER1, mana);
        }
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
    move_with(
        map,
        g,
        to,
        Move {
            run,
            pathfind: true,
            ..Default::default()
        },
    );
}

/// How a script moves a creature.
#[derive(Debug, Clone, Copy, Default)]
pub struct Move {
    pub run: bool,
    pub fly: bool,
    /// Along the navmesh, not straight.
    pub pathfind: bool,
    /// Taking this long, whatever its speed.
    pub time: Option<Duration>,
    /// Facing this way when it gets there.
    pub face: Option<f32>,
    /// The point its arrival tells of.
    pub point: Option<u32>,
}

/// Move a creature as `how` says, remembering the move so its arrival
/// can be told.
pub fn move_with(map: &mut Map, g: Guid, to: Vector3d, how: Move) -> bool {
    let now = map.now;
    let spline_id = map.next_spline_id;
    let Some(from) = map.creature(g).map(|c| c.position) else {
        return false;
    };
    let path = match map.nav.as_mut() {
        Some(nav) if how.pathfind && !how.fly => {
            nav.path(from, to).unwrap_or_else(|| vec![from, to])
        }
        _ => vec![from, to],
    };
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let Some(c) = map.creature_mut(g) else {
        return false;
    };
    let frame = c.walk_at(path, how.run, how.fly, how.time, now, spline_id);
    let moved = matches!(&c.motion, Motion::Moving(w) if w.spline_id == spline_id);
    if let Some(s) = c.scripted.as_deref_mut() {
        s.scripted_move = None;
    }
    if moved {
        let s = c.scripted.get_or_insert_with(Default::default);
        s.scripted_move = Some(event_ai::ScriptedMove {
            spline_id,
            to,
            point: how.point,
            face: how.face,
        });
    } else {
        // Already there.
        if let Some(o) = how.face {
            c.orientation = o;
            map.visibility_dirty.insert(g);
        }
        if let Some(point) = how.point {
            event_ai::movement_inform(map, g, event_ai::POINT_MOTION, point);
        }
    }
    if let Some(frame) = frame {
        map.send_to_observers(g, &frame, false);
    }
    if moved {
        let facing = (to.y - from.y).atan2(to.x - from.x);
        crate::game::map::creature_groups::leader_walked(map, g, to, facing, how.run);
    }
    true
}

/// The bounding radius Thistle Tea takes for a unit with none.
const DEFAULT_BOUNDING_RADIUS: f32 = 0.389;

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
            if let Some(lines) = step
                .ext
                .as_ref()
                .map(|e| &e.texts)
                .filter(|t| !t.is_empty())
            {
                let line = &lines[rand::rng().random_range(0..lines.len())];
                if matches!(line.chat_type, CHAT_WHISPER | CHAT_BOSS_WHISPER)
                    && !guid::is_player(target)
                {
                    return true;
                }
                let text = fill(map, &line.text, target);
                say(map, source, &text, line.chat_type, line.language, target);
                if line.emote != 0
                    && let Ok(emote) = Emote::try_from(line.emote)
                    && let Some(frame) = crate::game::conn::encode(&SMSG_EMOTE {
                        emote,
                        guid: source,
                    })
                {
                    map.send_to_observers(source, &frame, false);
                }
                return true;
            }
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
        // Move: to a point (datalong 0), offset from the target (1), at a
        // distance from it (2), or to a random point around one (3).
        // datalong2 is the travel time, datalong3 the options (1
        // pathfind, 2 walk, 4 run, 8 fly), datalong4 & 1 moves even a
        // held creature and & 2 tells of point dataint on arrival.
        3 => {
            if !guid::is_unit(source)
                || !alive(map, source)
                || (d4 & 0x1 == 0 && crate::game::map::auras::held(map, source, false))
            {
                return false;
            }
            let Some(here) = map.position_of(source) else {
                return false;
            };
            let (to, face) = match d1 {
                1 => {
                    let Some(t) = map.position_of(target) else {
                        return false;
                    };
                    (
                        Vector3d {
                            x: t.x + step.x,
                            y: t.y + step.y,
                            z: t.z + step.z,
                        },
                        step.o,
                    )
                }
                2 => {
                    let Some(t) = map.position_of(target) else {
                        return false;
                    };
                    let angle = if step.o < 0.0 {
                        (here.y - t.y).atan2(here.x - t.x)
                    } else {
                        rand::rng().random_range(0.0..std::f32::consts::TAU)
                    };
                    let radius = map
                        .unit_fields(target)
                        .map(|f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS))
                        .filter(|r| *r > 0.0)
                        .unwrap_or(DEFAULT_BOUNDING_RADIUS);
                    let d = step.x + radius;
                    (
                        Vector3d {
                            x: t.x + d * angle.cos(),
                            y: t.y + d * angle.sin(),
                            z: t.z,
                        },
                        step.o,
                    )
                }
                3 => {
                    let to = map
                        .nav
                        .as_mut()
                        .and_then(|n| n.random_point(pos, step.o))
                        .unwrap_or(pos);
                    (to, -10.0)
                }
                _ => (pos, step.o),
            };
            let run = if d3 & 0x4 != 0 {
                true
            } else if d3 & 0x2 != 0 {
                false
            } else {
                event_ai::scripted(map, source).is_some_and(|s| s.run)
            };
            let fly = d3 & 0x8 != 0 || event_ai::scripted(map, source).is_some_and(|s| s.fly);
            move_with(
                map,
                source,
                to,
                Move {
                    run,
                    fly,
                    pathfind: d3 & 0x1 != 0,
                    time: (d2 > 0).then(|| Duration::from_millis(u64::from(d2))),
                    face: (face > 0.0).then_some(face),
                    point: (d4 & 0x2 != 0).then_some(step.dataint[0].max(0) as u32),
                },
            )
        }
        // Modify a field's flags: set, clear, or toggle (set if none of
        // them is, else clear).
        4 => {
            let field = d1 as u16;
            let f = if guid::is_game_object(source) {
                map.gameobject_mut(source).map(|o| &mut o.fields)
            } else {
                map.unit_fields_mut(source)
            };
            let Some(f) = f else {
                return false;
            };
            let old = f.get_u32(field);
            let new = match d3 {
                1 => old | d2,
                2 => old & !d2,
                _ if old & d2 == 0 => old | d2,
                _ => old & !d2,
            };
            f.set_u32(field, new);
            true
        }
        5 => {
            let casting = map
                .caster(source)
                .and_then(|c| c.casting.as_ref())
                .map(|c| c.spell);
            if casting.is_some_and(|s| d2 == 0 || s == d2) {
                crate::game::map::spells::interrupt(map, source);
            }
            true
        }
        // Teleport the player of the two, or the creature.
        6 => {
            if let Some(p) = script_player(source, target) {
                map.teleport(p, MapKey::world(d1), pos, step.o);
                return true;
            }
            if !guid::is_unit(source) {
                return false;
            }
            place(map, source, pos, step.o);
            true
        }
        // A quest explored by a player near enough.
        7 => {
            let Some(player) = script_player(source, target) else {
                return false;
            };
            let other = if player == target { source } else { target };
            let quest = d1;
            let near = d2 == 0
                || map
                    .position_of(other)
                    .zip(map.position_of(player))
                    .is_some_and(|(a, b)| dist(a, b) <= d2 as f32);
            if near {
                let players = if d3 != 0 {
                    group_here(map, player)
                } else {
                    vec![player]
                };
                for p in players {
                    quest_explored(map, p, quest);
                }
            }
            near
        }
        // Kill credit, to the player or their group.
        8 => {
            let Some(player) = script_player(source, target) else {
                return false;
            };
            let credit = guid::unit(d1, 0);
            let players: Vec<Guid> = if d2 != 0 {
                group_here(map, player)
            } else {
                vec![player]
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
        // Summon a creature: datalong the entry, datalong2 the despawn
        // delay, datalong3 and datalong4 how many may be near and how
        // near (with dataint's unique flags 0x4 or 0x8), dataint 0x1 to
        // run, dataint2 a script for it, dataint3 a target type to attack
        // (-1 none), dataint4 the despawn type.
        10 => {
            let count = match step.ext.as_ref().map(|e| e.count) {
                Some(SummonCount::Fixed(n)) => n,
                Some(SummonCount::ThreatPlayers(ratio, least, most)) => {
                    let players = threat_list(map, source)
                        .into_iter()
                        .filter(|g| guid::is_player(*g))
                        .count();
                    ((players as f32 * ratio) as u32).max(least).min(most)
                }
                None => 1,
            };
            let mut any = false;
            for _ in 0..count {
                any |= summon_step(map, step, source, target);
            }
            any
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
            crate::game::map::auras::remove_spell(map, source, d1, None);
            true
        }
        // Cast, with vmangos's cast flags.
        15 => cast(map, source, target, d1, d2),
        // A sound by its distance from the source (Thistle Tea's flag 2),
        // or the same for all.
        16 if d2 & 0x2 != 0 => {
            if let Some(frame) = crate::game::conn::encode(&SMSG_PLAY_OBJECT_SOUND {
                sound_id: d1,
                guid: source,
            }) {
                map.send_to_observers(source, &frame, true);
            }
            true
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
        17 => script_player(source, target).is_some_and(|p| {
            crate::game::map::inventory::add_item(map, p, d1, d2.max(1), true).is_ok()
        }),
        // Despawn, after datalong ms.
        18 => {
            let g = source;
            if !guid::is_unit(g) {
                return false;
            }
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
        // Movement: 0 idle, 1 wander x yards (around its spawn, or with
        // datalong2 where it is), 2 waypoints, 7 home, 10 flee, 15 follow
        // the target at x yards and angle o, 19 keep x yards from it.
        20 => movement(map, step, source, target),
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
        // Threat, by x percent: on everyone (datalong 8) or the unit
        // datalong names as a target type.
        29 => {
            let pct = step.x;
            let only = if d1 == 8 {
                None
            } else {
                let pick = ScriptStep {
                    target_type: d1,
                    ..Default::default()
                };
                let Some(t) = (if d1 == 0 {
                    Some(target)
                } else {
                    find_target(map, &pick, source, target)
                }) else {
                    return false;
                };
                Some(t)
            };
            if let Some(c) = creature_mut(map, source) {
                for (t, v) in c.threat.iter_mut() {
                    if only.is_none_or(|o| o == *t) {
                        *v = (*v * (100.0 + pct) / 100.0).max(0.0);
                    }
                }
            }
            true
        }
        // End the script: at once, or if creature datalong isn't (or,
        // with datalong3, is) alive within datalong2 yards.
        31 => {
            if d1 == 0 {
                return false;
            }
            let at = map.position_of(source).unwrap_or(pos);
            let radius = if d2 > 0 { d2 as f32 } else { 30.0 };
            let found = !creatures_near(map, at, d1, radius, true).is_empty();
            let end = if d3 != 0 { found } else { !found };
            !end
        }
        // End the script if a condition holds (or, with datalong3 & 1,
        // doesn't), failing quest datalong2 for the player.
        32 => {
            use crate::game::map::conditions::{Truth, check};
            let judged = if target.is_zero() { source } else { target };
            let end = match check(map, d1, Some(source), Some(judged)) {
                Truth::Met => d3 & 0x1 == 0,
                Truth::Unmet => d3 & 0x1 != 0,
                Truth::Unknown => false,
            };
            if end
                && d2 > 0
                && let Some(p) = script_player(source, target)
            {
                for p in group_here(map, p) {
                    fail_quest(map, p, d2);
                }
            }
            !end
        }
        33 => {
            crate::game::map::ai::enter_evade(map, source);
            true
        }
        // Home: the step's place (0), where it stands (1), or its spawn's
        // (2).
        34 => {
            let Some(c) = map.creature(source) else {
                return false;
            };
            let home = match d1 {
                0 => Some((pos, step.o)),
                1 => Some((c.position, c.orientation)),
                _ => c
                    .spawn_guid
                    .and_then(|g| {
                        data.creature_spawns
                            .get(&map.key.map)
                            .and_then(|v| v.iter().find(|s| s.guid == g))
                    })
                    .map(|s| {
                        (
                            Vector3d {
                                x: s.x,
                                y: s.y,
                                z: s.z,
                            },
                            s.orientation,
                        )
                    }),
            };
            let Some((p, o)) = home else {
                return false;
            };
            if let Some(c) = creature_mut(map, source) {
                c.home = p;
                c.home_orientation = o;
            }
            true
        }
        // Turn to the target (datalong 0), or to the step's orientation.
        35 => {
            let o = if d1 == 0 {
                match (map.position_of(source), map.position_of(target)) {
                    (Some(a), Some(b)) if target != source => (b.y - a.y).atan2(b.x - a.x),
                    _ => return false,
                }
            } else {
                step.o
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
            if let Some(script) = choose_script(step) {
                start(map, ScriptTable::Generic, script, source, target);
            }
            true
        }
        40 => {
            let Some(target) = script_player(source, target) else {
                return false;
            };
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
        // Melee on or off: off, it stays in the fight, but doesn't swing.
        42 => {
            if let Some(s) = event_ai::scripted_mut(map, source) {
                s.no_melee = d1 == 0;
            }
            true
        }
        43 => {
            if let Some(s) = event_ai::scripted_mut(map, source) {
                s.no_combat_movement = d1 == 0;
            }
            true
        }
        // Phase: set, or (datalong2 1) raised or (2) lowered by datalong.
        44 => {
            let phase = event_ai::phase(map, source);
            let to = match d2 {
                1 => phase.saturating_add(d1),
                2 => phase.saturating_sub(d1),
                _ => d1,
            };
            event_ai::set_phase(map, source, to);
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
        // Flee: in fear for a while, or (datalong) to the nearest friend
        // for help.
        47 => {
            crate::game::map::creature_combat::flee_for_assistance(map, source, d1 != 0);
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
            let players: Vec<Guid> = map
                .players
                .values()
                .filter(|p| p.death == crate::game::map::death::Death::Alive)
                .map(|p| p.guid)
                .collect();
            for p in players {
                crate::game::map::ai::attacked(map, source, p);
            }
            true
        }
        // Call for help within x yards.
        50 => {
            let radius = if step.x > 0.0 { step.x } else { d1 as f32 };
            if radius <= 0.0 {
                return false;
            }
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
        // A creature spell list, from up to four by their chances out of a
        // hundred; none if the roll passes them all, as Thistle Tea has it.
        55 => {
            let roll = rand::rng().random_range(1..=100);
            let mut sum = 0;
            let list = (0..4)
                .map(|i| (step.datalong[i], step.dataint[i]))
                .filter(|(_, c)| *c > 0)
                .find(|(_, c)| {
                    let hit = roll > sum && roll <= sum + c;
                    sum += c;
                    hit
                })
                .map_or(0, |(l, _)| l);
            crate::game::map::spells::lists::set_list(map, source, list);
            true
        }
        // The source's guardians (of entry datalong, or all) go.
        56 => {
            crate::game::map::pets::remove_guardians(map, source, d1);
            true
        }
        // React state: 0 passive, 1 defensive, 2 aggressive.
        59 => {
            if let Some(s) = event_ai::scripted_mut(map, source) {
                use crate::game::map::assistance::Reaction;
                s.passive = d1 == 0;
                s.reaction = Some(match d1 {
                    0 => Reaction::Passive,
                    1 => Reaction::Defensive,
                    _ => Reaction::Aggressive,
                });
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
        // Waypoints: from the spawn's or the entry's path (datalong 0),
        // the spawn's (1), the entry's (2), the entry's special one (3),
        // or a script's (5, variant dataint3); from point datalong2, after
        // datalong3 ms, round again with datalong4. dataint and dataint2
        // stand in for the spawn and entry.
        60 => {
            let Some(c) = map.creature(source).filter(|c| c.alive()) else {
                return false;
            };
            let spawn = if step.dataint[0] > 0 {
                Some(step.dataint[0] as u32)
            } else {
                c.spawn_guid
            };
            let entry = if step.dataint[1] > 0 {
                step.dataint[1] as u32
            } else {
                c.entry
            };
            let by_spawn = || spawn.and_then(|g| data.waypoints.get(&g));
            let points = match d1 {
                0 => by_spawn().or_else(|| data.waypoints_by_entry.get(&entry)),
                1 => by_spawn(),
                2 => data.waypoints_by_entry.get(&entry),
                3 => data.waypoints_special.get(&entry),
                5 => data
                    .scripts
                    .routes
                    .get(&(entry, step.dataint[2].max(0) as u32)),
                _ => None,
            };
            let Some(points) = points.filter(|p| !p.is_empty()).cloned() else {
                return false;
            };
            // From the point numbered datalong2 if it has one, else its
            // first.
            let start = points
                .iter()
                .position(|p| d2 > 0 && p.point == d2)
                .unwrap_or(0);
            if let Some(s) = event_ai::scripted_mut(map, source) {
                s.route = Some(event_ai::Route {
                    points: std::sync::Arc::new(points),
                    repeat: d4 != 0,
                });
                s.follow = None;
            }
            if let Some(c) = creature_mut(map, source) {
                c.movement_type = crate::game::map::creature::MOVEMENT_WAYPOINT;
                c.waypoint = start;
                c.last_waypoint = None;
                c.motion = Motion::Idle {
                    until: Some(now + Duration::from_millis(u64::from(d3))),
                };
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
        // Script datalong for everything near of a kind (datalong2: 0
        // objects, 1 units, 2 creatures, 3 players) and entry
        // (datalong3, 0 any) within datalong4 yards.
        68 => {
            let Some(at) = map.position_of(source) else {
                return false;
            };
            let radius = d4 as f32;
            let mut found: Vec<Guid> = Vec::new();
            if matches!(d2, 1 | 2) {
                found.extend(
                    map.grid
                        .around(at.x, at.y, radius)
                        .filter(|g| guid::is_unit(*g))
                        .filter(|g| {
                            map.creature(*g).is_some_and(|c| {
                                (d3 == 0 || c.entry == d3) && dist(c.position, at) <= radius
                            })
                        }),
                );
            }
            if matches!(d2, 1 | 3) {
                found.extend(
                    map.players
                        .values()
                        .filter(|p| dist(p.position(), at) <= radius)
                        .map(|p| p.guid),
                );
            }
            if d2 == 0 {
                found.extend(
                    map.gameobjects
                        .iter()
                        .filter(|(_, o)| {
                            (d3 == 0 || o.entry == d3) && dist(o.position, at) <= radius
                        })
                        .map(|(_, o)| o.guid),
                );
            }
            found.sort();
            found.dedup();
            for g in found {
                start(map, ScriptTable::Generic, d1, g, target);
            }
            true
        }
        70 => {
            let Some(p) = script_player(source, target) else {
                return false;
            };
            for p in group_here(map, p) {
                fail_quest(map, p, d1);
            }
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
            crate::game::map::spells::cast_triggered(map, source, d1, Some(source));
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
        // Join the target's group at x yards and angle o (or, Thistle
        // Tea's, where it stands from it), with flags datalong.
        78 => {
            if !guid::is_unit(target) || target == source {
                return false;
            }
            let (distance, angle) = if step.ext.as_ref().is_some_and(|e| e.formation_from_position)
            {
                let (Some(me), Some(them)) = (map.creature(source), map.creature(target)) else {
                    return false;
                };
                (
                    dist(me.position, them.position),
                    (me.position.y - them.position.y).atan2(me.position.x - them.position.x)
                        - them.orientation,
                )
            } else {
                (step.x.max(0.0), step.o)
            };
            crate::game::map::creature_groups::join_with(map, source, target, distance, angle, d1);
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
            let Some(target) = script_player(source, target) else {
                return false;
            };
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
        // A script event for the source, from the target.
        85 => {
            if !guid::is_unit(source) {
                return false;
            }
            event_ai::script_event(map, source, d1, d2, Some(target).filter(|t| !t.is_zero()));
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
        // One of up to four scripts, by chance, for each of a creature
        // group.
        90 => {
            let Some(script) = choose_script(step) else {
                return false;
            };
            let members = crate::game::map::creature_groups::members_of(map, source);
            for m in members {
                start(map, ScriptTable::Generic, script, m, target);
            }
            true
        }
        // Script datalong for everyone in zone datalong2.
        92 => {
            let players: Vec<Guid> = map
                .players
                .values()
                .filter(|p| d2 == 0 || p.zone == d2)
                .map(|p| p.guid)
                .collect();
            for p in players {
                start(map, ScriptTable::Generic, d1, p, target);
            }
            true
        }
        // Set a field to a value.
        2 => {
            let Some(f) = map.unit_fields_mut(source) else {
                return false;
            };
            f.set_u32(d1 as u16, d2);
            true
        }
        54 => {
            set_server_variable(d1, d2);
            true
        }
        61..=66 | 69 => crate::game::map::map_events::command(map, step, source, target),
        // A spawn the world database keeps back until a script loads it.
        82 => {
            if object_of_spawn(map, d1).is_some() {
                return true;
            }
            let spawn = data
                .gameobject_spawns
                .get(&map.key.map)
                .and_then(|v| v.iter().find(|s| s.guid == d1))
                .cloned();
            let Some(s) = spawn else {
                return false;
            };
            let Some(t) = data.gameobject_templates.get(&s.entry) else {
                return false;
            };
            map.add_gameobject(crate::game::map::gameobject::GameObject::from_spawn(t, &s));
            true
        }
        91 => {
            if creature_of_spawn(map, d1).is_some() {
                return true;
            }
            let spawn = data
                .creature_spawns
                .get(&map.key.map)
                .and_then(|v| v.iter().find(|s| s.guid == d1))
                .cloned();
            let Some(s) = spawn else {
                return false;
            };
            match Creature::from_spawn(&data, &s, &mut rand::rng()) {
                Some(c) => {
                    map.add_creature(c);
                    true
                }
                None => false,
            }
        }
        // Kept updating with no one near: every map here updates anyway.
        21 => true,
        30 => {
            if guid::is_player(target) {
                crate::game::map::npc::fly_path(map, target, d1);
                true
            } else {
                false
            }
        }
        36 => {
            let player = if guid::is_player(target) {
                target
            } else {
                source
            };
            if !guid::is_player(player) {
                return false;
            }
            crate::game::map::meeting_stones::queue(map, player, d1);
            true
        }
        // A guid the instance script keeps: the target's (0) or the source's.
        38 => {
            let g = if d2 == 0 { target } else { source };
            map.instance.guids.insert(d1, g);
            true
        }
        // Start (1) or stop (0) a game event.
        53 => {
            let realm = map.realm.clone();
            crate::game::events::force(&realm, d1, d2 != 0);
            true
        }
        // A cooldown on (57) or off (58) the source's spell.
        57 => {
            let until = now + Duration::from_secs(u64::from(d2));
            if let Some(c) = map.caster_mut(source) {
                c.cooldowns.insert(d1, until);
            }
            true
        }
        58 => {
            if let Some(c) = map.caster_mut(source) {
                c.cooldowns.remove(&d1);
            }
            if let Some(p) = map.players.get(&source) {
                p.outbound.send(&SMSG_CLEAR_COOLDOWN {
                    id: d1,
                    target: source,
                });
            }
            true
        }
        // Help the target with its fight.
        72 => {
            let foe = map
                .combat(target)
                .map(|c| c.victim)
                .filter(|v| !v.is_zero() && alive(map, *v));
            match foe {
                Some(f) if guid::is_unit(source) => {
                    crate::game::map::ai::attacked(map, source, f);
                    true
                }
                _ => false,
            }
        }
        75 => {
            if guid::is_unit(source) && alive(map, target) {
                crate::game::map::ai::add_threat(map, source, target, d1 as f32);
                crate::game::map::ai::attacked(map, source, target);
                true
            } else {
                false
            }
        }
        // A player's PvP flag on (1) or off.
        86 => {
            let Some(p) = script_player(target, source).and_then(|g| map.players.get_mut(&g))
            else {
                return false;
            };
            p.fields.set_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_PVP,
                d1 != 0,
            );
            true
        }
        // A pet's command: stay, follow, attack the target, dismiss.
        88 => {
            if crate::game::map::pets::state(map, source).is_none() {
                return false;
            }
            match d1 as u8 {
                crate::game::map::pets::COMMAND_STAY => {
                    let at = map.position_of(source);
                    crate::game::map::combat::stop_attack(map, source);
                    crate::game::map::pets::stop_where(map, source);
                    if let Some(s) = crate::game::map::pets::state_mut(map, source) {
                        s.command = crate::game::map::pets::COMMAND_STAY;
                        s.stay_at = at;
                    }
                }
                crate::game::map::pets::COMMAND_FOLLOW => {
                    crate::game::map::combat::stop_attack(map, source);
                    if let Some(s) = crate::game::map::pets::state_mut(map, source) {
                        s.command = crate::game::map::pets::COMMAND_FOLLOW;
                        s.stay_at = None;
                    }
                }
                crate::game::map::pets::COMMAND_ATTACK => {
                    if !crate::game::map::combat::can_attack(map, source, target) {
                        return false;
                    }
                    crate::game::map::pets::attack_target(map, source, target);
                }
                crate::game::map::pets::COMMAND_DISMISS => {
                    crate::game::map::pets::dismiss(map, source)
                }
                _ => return false,
            }
            true
        }
        // Fly, or come down.
        77 => {
            let on = d1 != 0;
            if let Some(s) = event_ai::scripted_mut(map, source) {
                s.fly = on;
            }
            if let Some(c) = creature_mut(map, source) {
                if on {
                    c.move_flags |= FLIGHT_FLAGS;
                } else {
                    c.move_flags &= !FLIGHT_FLAGS;
                }
            }
            true
        }
        // Thistle Tea's: the rest of the source's scripts end (in
        // `advance`).
        C_STOP_SCRIPTS => true,
        // Hold its path for datalong ms: until its summons since are gone,
        // or (datalong2 1) until released. dataint is the release's
        // script, dataint2 the one when it runs out unreleased.
        C_HOLD_WAYPOINTS => {
            let earlier: Vec<Guid> = event_ai::scripted(map, source)
                .map(|s| {
                    s.summoned
                        .iter()
                        .copied()
                        .filter(|g| map.creature(*g).is_some_and(|c| c.alive()))
                        .collect()
                })
                .unwrap_or_default();
            let Some(s) = event_ai::scripted_mut(map, source) else {
                return false;
            };
            s.hold = Some(event_ai::Hold {
                until: now + Duration::from_millis(u64::from(d1)),
                ready_at: now + Duration::from_millis(1),
                signal: d2 == 1,
                signaled: false,
                earlier,
                release_script: step.dataint[0].max(0) as u32,
                expired_script: step.dataint[1].max(0) as u32,
            });
            true
        }
        C_RELEASE_WAYPOINTS => {
            if let Some(h) = event_ai::scripted_mut(map, source).and_then(|s| s.hold.as_mut()) {
                h.signaled |= now < h.until;
                h.until = h.until.min(now);
            }
            true
        }
        C_SET_REGENERATION => {
            if let Some(s) = event_ai::scripted_mut(map, source) {
                s.regenerate = Some(d1);
            }
            true
        }
        C_SET_CASTER_CHASE_DISTANCE => {
            if let Some(s) = event_ai::scripted_mut(map, source) {
                s.caster_chase_distance = (d1 > 0).then_some(d1 as f32);
            }
            true
        }
        C_SET_HEALTH_PCT => {
            if !(1..=100).contains(&d1) || !alive(map, source) {
                return false;
            }
            let Some(f) = map.unit_fields_mut(source) else {
                return false;
            };
            let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
            f.set_u32(UNIT_FIELD_HEALTH, (max * d1 / 100).max(1));
            true
        }
        // Lose datalong health: dying of it if it is all there is.
        C_LOSE_HEALTH => {
            if d1 == 0 || !alive(map, source) {
                return false;
            }
            let health = map
                .unit_fields(source)
                .map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH));
            if health > d1 + 1 {
                if let Some(f) = map.unit_fields_mut(source) {
                    f.set_u32(UNIT_FIELD_HEALTH, health - d1);
                }
            } else {
                crate::game::map::combat::deal_damage(map, source, source, health);
            }
            true
        }
        C_SET_CONCEALED => {
            let Some(s) = event_ai::scripted_mut(map, source) else {
                return false;
            };
            s.concealed = d1 != 0;
            crate::game::map::visibility::stealth_changed(map, source);
            true
        }
        // The player has cast spell datalong on the source, for their
        // quests.
        C_CAST_CREDIT => {
            let Some(player) = script_player(source, target).filter(|p| *p != source) else {
                return false;
            };
            cast_credit(map, player, d1, source);
            true
        }
        // The auras an evade would shed.
        C_CLEAR_AURAS => {
            let held: Vec<u32> = map
                .creature(source)
                .map(|c| c.auras.holders.iter().map(|a| a.spell).collect())
                .unwrap_or_default();
            for spell in held {
                crate::game::map::auras::remove_spell(map, source, spell, None);
            }
            true
        }
        // Linked feign death: get up after datalong ms unless every
        // creature datalong2 within datalong3 yards is down too, under
        // aura datalong4; scripts dataint (on falling) and dataint2 (on
        // getting up).
        C_LINKED_FEIGN_DEATH => {
            let Some(s) = event_ai::scripted_mut(map, source) else {
                return false;
            };
            s.feign = Some(event_ai::Feign {
                delay: Duration::from_millis(u64::from(d1)),
                entry: d2,
                radius: d3 as f32,
                pacify: d4,
                fall_script: step.dataint[0].max(0) as u32,
                revive_script: step.dataint[1].max(0) as u32,
                down_at: None,
            });
            true
        }
        C_SET_MELEE_SPELL => {
            if let Some(s) = event_ai::scripted_mut(map, source) {
                s.melee_spell = (d1 > 0).then_some(d1);
            }
            true
        }
        C_SET_CALL_FOR_HELP_RANGE => {
            if let Some(s) = event_ai::scripted_mut(map, source) {
                s.call_for_help_range = Some(d1 as f32);
            }
            true
        }
        // A battleground's creature event.
        C_BATTLEGROUND_EVENT => {
            if !guid::is_unit(source) {
                return false;
            }
            crate::game::map::battleground::creature_event(map, source, d1);
            true
        }
        // The source to the step's place, or to its target.
        C_TELEPORT_SELF => {
            if !guid::is_unit(source) || !alive(map, source) {
                return false;
            }
            let (at, o) = if step.ext.as_ref().is_some_and(|e| e.at_target) {
                let Some(at) = map.position_of(target) else {
                    return false;
                };
                (at, map.creature(source).map_or(0.0, |c| c.orientation))
            } else {
                (pos, step.o)
            };
            place(map, source, at, o);
            true
        }
        _ => true,
    }
}

/// One creature of a summon step.
fn summon_step(map: &mut Map, step: &ScriptStep, source: Guid, target: Guid) -> bool {
    let [d1, d2, d3, d4] = step.datalong;
    let mut at = Vector3d {
        x: step.x,
        y: step.y,
        z: step.z,
    };
    let mut o = step.o;
    let ext = step.ext.as_deref();
    if let Some(e) = ext {
        if e.at_target {
            let Some(p) = map.position_of(target) else {
                return false;
            };
            at = p;
            o = map.creature(source).map_or(o, |c| c.orientation);
        } else if !e.positions.is_empty() {
            let vacant: Vec<[f32; 4]> = e
                .positions
                .iter()
                .copied()
                .filter(|p| {
                    let here = Vector3d {
                        x: p[0],
                        y: p[1],
                        z: p[2],
                    };
                    !map.grid
                        .around(here.x, here.y, 40.0)
                        .filter(|g| guid::is_unit(*g))
                        .any(|g| {
                            map.creature(g)
                                .is_some_and(|c| c.alive() && dist(c.position, here) < 1.0)
                        })
                })
                .collect();
            if vacant.is_empty() {
                return false;
            }
            let p = vacant[rand::rng().random_range(0..vacant.len())];
            at = Vector3d {
                x: p[0],
                y: p[1],
                z: p[2],
            };
            o = p[3];
        }
    }
    if at.x == 0.0 && at.y == 0.0 && at.z == 0.0 {
        let Some(here) = map.position_of(source) else {
            return false;
        };
        at = here;
    }
    if let Some(e) = ext
        && e.scatter > 0.0
    {
        at = map
            .nav
            .as_mut()
            .and_then(|n| n.random_point(at, e.scatter))
            .unwrap_or(at);
    }
    let flags = step.dataint[0] as u32;
    // Unique: not if as many as allowed are near already.
    if flags & 0xC != 0
        && creatures_near(map, at, d1, if d4 > 0 { d4 as f32 } else { 100.0 }, true).len()
            >= d3.max(1) as usize
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
    let Some(g) = summon(map, source, d1, at, o, despawn) else {
        return false;
    };
    if let Some(s) = event_ai::scripted_mut(map, g) {
        if flags & 0x1 != 0 {
            s.run = true;
        }
        if despawn.is_none() && step.dataint[3] > 0 {
            s.despawn_type = step.dataint[3];
        }
        if ext.is_some_and(|e| e.concealed) {
            s.concealed = true;
        }
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

/// A player cast `spell` on `creature`: the quests that ask for it.
fn cast_credit(map: &mut Map, player: Guid, spell: u32, creature: Guid) {
    let entry = crate::game::map::quests::entry_of(map, creature);
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
            if q.required_spell_casts[i] == spell
                && *id > 0
                && *id as u32 == entry
                && l.kills[i] < *count
            {
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
        crate::game::map::quests::check(map, player, q);
    }
}

/// The player of a step's two: the target if it is one, else the source.
fn script_player(source: Guid, target: Guid) -> Option<Guid> {
    if guid::is_player(target) {
        Some(target)
    } else {
        guid::is_player(source).then_some(source)
    }
}

/// One of a step's up to four scripts (datalong) by their chances
/// (dataint): a roll of 1 to 100 falls in one's share, or in none.
fn choose_script(step: &ScriptStep) -> Option<u32> {
    let options: Vec<(u32, i32)> = (0..4)
        .map(|i| (step.datalong[i], step.dataint[i]))
        .filter(|(s, _)| *s > 0)
        .collect();
    if options.is_empty() {
        return None;
    }
    if options.iter().all(|(_, c)| *c <= 0) {
        return Some(options[rand::rng().random_range(0..options.len())].0);
    }
    let roll = rand::rng().random_range(1..=100);
    let mut sum = 0;
    for (s, c) in options {
        if roll > sum && roll <= sum + c {
            return Some(s);
        }
        sum += c;
    }
    None
}

/// A quest of a player's failed.
fn fail_quest(map: &mut Map, player: Guid, quest: u32) {
    if let Some(p) = map.players.get_mut(&player)
        && let Some(slot) = p.journal.slot_of(quest)
    {
        p.journal.log[slot].failed = true;
    }
    crate::game::map::quests::check(map, player, quest);
}

/// A creature put at `at` at once, as vmangos's `NearTeleportTo`.
pub fn place(map: &mut Map, g: Guid, at: Vector3d, orientation: f32) {
    let now = map.now;
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.motion = Motion::Idle {
        until: (c.movement_type != crate::game::map::creature::MOVEMENT_IDLE).then_some(now),
    };
    c.position = at;
    c.orientation = orientation;
    if let Some(s) = c.scripted.as_deref_mut() {
        s.scripted_move = None;
    }
    let (anchor, old_cell) = (c.vis_anchor, c.cell);
    let new_cell = crate::game::map::grid::cell_of(at.x, at.y);
    c.cell = new_cell;
    map.grid.relocate(g, old_cell, new_cell);
    crate::game::map::visibility::moved(map, g, anchor, at);
    let frame = crate::game::map::ai::stop_packet(g, at, spline_id);
    map.send_to_observers(g, &frame, false);
}

/// `SCRIPT_COMMAND_MOVEMENT`.
fn movement(map: &mut Map, step: &ScriptStep, source: Guid, target: Guid) -> bool {
    let now = map.now;
    let [d1, d2, d3, _] = step.datalong;
    if !map.creature(source).is_some_and(|c| c.alive()) {
        return false;
    }
    if let Some(s) = event_ai::scripted_mut(map, source) {
        s.follow = None;
    }
    match d1 {
        0 => {
            let at = map.position_of(source);
            if let Some(c) = creature_mut(map, source) {
                c.movement_type = crate::game::map::creature::MOVEMENT_IDLE;
                if matches!(c.motion, Motion::Moving(_)) {
                    c.motion = Motion::Idle { until: None };
                    if let Some(at) = at {
                        let spline_id = map.next_spline_id;
                        map.next_spline_id = map.next_spline_id.wrapping_add(1);
                        let frame = crate::game::map::ai::stop_packet(source, at, spline_id);
                        map.send_to_observers(source, &frame, false);
                    }
                } else {
                    c.motion = Motion::Idle { until: None };
                }
            }
        }
        1 => {
            let here = map.position_of(source);
            if let Some(c) = creature_mut(map, source) {
                c.movement_type = crate::game::map::creature::MOVEMENT_RANDOM;
                c.wander_distance = step.x.max(0.0);
                if d2 != 0
                    && let Some(h) = here
                {
                    c.home = h;
                }
                c.motion = Motion::Idle { until: Some(now) };
            }
        }
        // Its path (the spawn's, else the entry's), as START_WAYPOINTS
        // with the start point datalong3 and round again with datalong2.
        2 => {
            let walk = ScriptStep {
                command: 60,
                datalong: [0, d3, 0, d2],
                dataint: [0, 0, step.dataint[2], step.dataint[3]],
                ext: None,
                ..step.clone()
            };
            return command(map, &walk, source, target);
        }
        7 => {
            let Some((home, o)) = map.creature(source).map(|c| (c.home, c.home_orientation)) else {
                return false;
            };
            move_with(
                map,
                source,
                home,
                Move {
                    pathfind: true,
                    face: Some(o),
                    ..Default::default()
                },
            );
        }
        10 => {
            let from = if d2 == 0 {
                target
            } else {
                map.combat(source).map(|c| c.victim).unwrap_or_default()
            };
            if from.is_zero() || from == source {
                return false;
            }
            crate::game::map::ai::flee_from(
                map,
                source,
                from,
                (d3 > 0).then(|| Duration::from_millis(u64::from(d3))),
            );
        }
        15 => {
            if !guid::is_unit(target) && !guid::is_player(target) || target == source {
                return false;
            }
            let angle = if step.o < 0.0 {
                rand::rng().random_range(0.0..std::f32::consts::TAU)
            } else {
                step.o
            };
            if let Some(s) = event_ai::scripted_mut(map, source) {
                s.follow = Some((target, step.x.max(0.0), angle));
            }
            if let Some(c) = creature_mut(map, source) {
                c.movement_type = crate::game::map::creature::MOVEMENT_IDLE;
            }
            event_ai::follow_step(map, source);
        }
        19 => {
            let mana_ok = d3 == 0
                || map.unit_fields(source).is_some_and(|f| {
                    let max = f.get_u32(UNIT_FIELD_MAXPOWER1);
                    max == 0 || f.get_u32(UNIT_FIELD_POWER1) * 100 >= d3 * max
                });
            let victim = map.combat(source).map(|c| c.victim).unwrap_or_default();
            let from = if victim.is_zero() { target } else { victim };
            if !mana_ok || from.is_zero() || from == source {
                return true;
            }
            let (Some(here), Some(them)) = (map.position_of(source), map.position_of(from)) else {
                return false;
            };
            let angle = (here.y - them.y).atan2(here.x - them.x);
            let to = Vector3d {
                x: them.x + step.x * angle.cos(),
                y: them.y + step.x * angle.sin(),
                z: them.z,
            };
            move_with(
                map,
                source,
                to,
                Move {
                    run: true,
                    pathfind: true,
                    ..Default::default()
                },
            );
        }
        _ => {}
    }
    true
}

static SERVER_VARIABLES: std::sync::LazyLock<parking_lot::Mutex<ahash::AHashMap<u32, u32>>> =
    std::sync::LazyLock::new(Default::default);

/// A server-wide variable scripts set and conditions read.
pub fn server_variable(index: u32) -> u32 {
    SERVER_VARIABLES.lock().get(&index).copied().unwrap_or(0)
}

pub fn set_server_variable(index: u32, value: u32) {
    SERVER_VARIABLES.lock().insert(index, value);
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
/// under way, 0x2 is free and instant, and the rest as a spell list's
/// ([`crate::game::map::spells::lists::flags_allow`]). Whether it went: a
/// creature's that its checks stopped didn't, as Thistle Tea has it.
pub fn cast(map: &mut Map, caster: Guid, target: Guid, spell: u32, flags: u32) -> bool {
    if spell == 0 {
        return false;
    }
    let target = if target.is_zero() { caster } else { target };
    if !crate::game::map::spells::lists::flags_allow(map, caster, flags, spell, target) {
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
        return true;
    }
    let went = crate::game::map::spells::cast(
        map,
        caster,
        spell,
        crate::game::map::spells::Targets {
            unit: Some(target),
            ..Default::default()
        },
        None,
    );
    went || map.players.contains_key(&caster)
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
    // A code-built script: on the player, at the object.
    let scripts = &map.realm.data.scripts;
    let hello = spawn
        .and_then(|s| scripts.object_hello.get(&s))
        .or_else(|| scripts.object_hello_entry.get(&entry))
        .copied();
    if let Some(id) = hello {
        start(map, ScriptTable::Generic, id, player, object);
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
