//! A battleground match, on its copy of the map. What every battleground
//! shares lives here: the countdown and the doors, the events that put each
//! spawn in or out of the match (vmangos's `creature_battleground` and
//! `gameobject_battleground`), resurrection waves at the spirit guides,
//! scores and the scoreboard, bonus honor, the end and the close. Each
//! battleground's rules are its own: Warsong Gulch ([`wsg`]), Arathi Basin
//! ([`ab`]) and Alterac Valley ([`av`]), as Thistle Tea models them.

pub mod ab;
pub mod av;
pub mod av_quests;
pub mod av_routes;
pub mod av_scripts;
pub mod insignia;
pub mod wsg;

use std::time::{Duration, Instant};

use ahash::{AHashMap, AHashSet};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::battlegrounds;
use crate::game::data::items::Body;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::creature::Creature;
use crate::game::map::death::Death;
use crate::game::map::gameobject::GameObject;
use crate::game::map::{Map, MapKey};
use crate::game::registry::Team;

pub use wsg::{
    CAPTURE_TRIGGERS, CAPTURES_TO_WIN, FLAG_AT, FLAG_STANDS, FLAGS_DROPPED, Flag,
    SPELL_FLAG_CARRIED, WS_CAPTURES, WS_CAPTURES_MAX, WS_FLAG_STATE,
};

pub const SPELL_DESERTER: u32 = 26013;
/// Waiting to Resurrect: a ghost in the match, for the next wave.
pub const SPELL_WAITING_TO_RESURRECT: u32 = 2584;
pub const RESURRECT_EVERY: Duration = Duration::from_secs(30);
/// The winner of a match that ended even (mangos's `WINNER_NONE`).
pub const NO_WINNER: usize = 2;

/// Battlegrounds, by `battleground_template` id.
pub const BG_AV: u32 = 1;
pub const BG_WSG: u32 = 2;
pub const BG_AB: u32 = 3;

/// The events every battleground has: its ghost gates and its doors.
pub const EVENT_GHOST_GATES: u8 = 253;
pub const EVENT_DOORS: u8 = 254;

/// The map of the battleground a battlemaster sends players to.
pub fn map_for_battlemaster(map: &Map, npc: Guid) -> u32 {
    let bg = map
        .realm
        .data
        .battlemasters
        .get(&guid::entry(npc))
        .copied()
        .unwrap_or(BG_WSG);
    battlegrounds::map_of(bg)
}

pub fn side(team: Team) -> usize {
    match team {
        Team::Alliance => 0,
        Team::Horde => 1,
    }
}

pub fn other(s: usize) -> usize {
    1 - s
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Waiting,
    Starting { at: Instant },
    Playing,
    Ended { winner: usize, at: Instant },
}

/// A player's part in the match, for the scoreboard.
#[derive(Debug, Clone, Copy, Default)]
pub struct Score {
    pub team: usize,
    pub killing_blows: u32,
    pub honorable_kills: u32,
    pub deaths: u32,
    pub bonus_honor: u32,
    /// The battleground's own columns: Warsong Gulch's captures and
    /// returns; Arathi Basin's bases assaulted and defended; Alterac
    /// Valley's graveyards and towers assaulted and defended, mines
    /// captured, leaders killed and secondary objectives.
    pub objectives: [u32; 7],
}

/// Something the match does later.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Due {
    /// A node's banner goes up in its new state.
    Banner { node: u8, revision: u32 },
    /// An assault on a node succeeds, if still under way.
    Capture { node: u8, revision: u32 },
    /// An Alterac mine goes back to its own.
    MineReclaim { mine: u8, revision: u32 },
    /// An Alterac captain rallies their side.
    CaptainBuff(usize),
    /// A pickup comes back.
    Buff(usize),
    /// A Warsong flag left on the ground goes home, if still there.
    FlagReturn { side: u8, object: Guid },
    /// Warsong's flags stand again after a capture.
    FlagsPlaced,
    /// A second into the battle: everyone's status and the scoreboard.
    StatusRefresh,
}

/// A pickup's place: what's there now, and since when it's been gone.
#[derive(Debug, Clone, Copy)]
pub struct BuffSlot {
    pub at: (f32, f32, f32, f32),
    pub object: Option<Guid>,
}

#[derive(Debug, Clone)]
pub struct Match {
    pub bg: u32,
    pub phase: Phase,
    /// Each side's score: Warsong Gulch's captures, Arathi Basin's
    /// resources.
    pub score: [u32; 2],
    /// Warsong Gulch: each side's flag.
    pub flags: [Option<Flag>; 2],
    pub scores: AHashMap<Guid, Score>,
    pub resurrect_at: Instant,
    /// Ghosts waiting for the next wave.
    pub resurrect_queue: AHashSet<Guid>,
    /// Each event's state; `None` for off.
    pub events: AHashMap<u8, Option<u8>>,
    pub timers: Vec<(Instant, Due)>,
    pub started_at: Option<Instant>,
    /// Countdown announcements made: a minute, half a minute.
    pub countdown: u8,
    pub bracket: u8,
    /// A Call to Arms weekend for this battleground.
    pub weekend: bool,
    pub ab: Option<ab::Ab>,
    pub av: Option<av::Av>,
    pub buffs: Vec<BuffSlot>,
    /// A side short of players: when the match ends for it, and what was
    /// left of that at the last update (mangos's premature finish).
    pub premature: Option<Instant>,
    pub premature_left: Duration,
}

impl Match {
    pub fn new(bg: u32, now: Instant) -> Self {
        Self {
            bg,
            phase: Phase::Waiting,
            score: [0, 0],
            flags: [None, None],
            scores: AHashMap::new(),
            resurrect_at: now + RESURRECT_EVERY,
            resurrect_queue: AHashSet::new(),
            events: initial_events(bg),
            timers: Vec::new(),
            started_at: None,
            countdown: 0,
            bracket: 5,
            weekend: false,
            ab: (bg == BG_AB).then(ab::Ab::default),
            av: (bg == BG_AV).then(av::Av::new),
            buffs: Vec::new(),
            premature: None,
            premature_left: Duration::ZERO,
        }
    }

    pub fn world_states(&self) -> Vec<WorldState> {
        let states: Vec<(u32, u32)> = match self.bg {
            BG_AB => ab::world_states(self),
            BG_AV => av::world_states(self),
            _ => wsg::world_states(self),
        };
        states
            .into_iter()
            .map(|(state, value)| WorldState { state, value })
            .collect()
    }

    pub fn playing(&self) -> bool {
        self.phase == Phase::Playing
    }

    pub fn team_of(&self, g: Guid) -> Option<usize> {
        self.scores.get(&g).map(|s| s.team)
    }

    pub fn schedule(&mut self, at: Instant, due: Due) {
        self.timers.push((at, due));
    }
}

/// The events a battleground starts with.
pub fn initial_events(bg: u32) -> AHashMap<u8, Option<u8>> {
    let mut e = AHashMap::new();
    match bg {
        BG_AB => {
            for node in 0..5 {
                e.insert(node, Some(0));
            }
        }
        BG_AV => return av::initial_events(),
        // Warsong Gulch's flags are the match's own; its spirit guides are
        // event 2.
        _ => {
            e.insert(2, Some(0));
        }
    }
    e.insert(EVENT_GHOST_GATES, Some(0));
    e.insert(EVENT_DOORS, Some(0));
    e
}

/// The Call to Arms holiday of each battleground, by `game_event`.
fn weekend_event(bg: u32) -> u32 {
    match bg {
        BG_AV => 18,
        BG_WSG => 19,
        _ => 20,
    }
}

/// The match a battleground copy plays.
pub fn for_map(key: MapKey, now: Instant) -> Option<Match> {
    if key.instance == 0 {
        return None;
    }
    let bg = battlegrounds::bg_of_map(key.map)?;
    Some(Match::new(bg, now))
}

/// Whether a spawn belongs in the match as its events stand: one with no
/// events always does; one with events while any of them is on.
pub fn spawn_allowed(map: &Map, creature: bool, spawn: u32) -> bool {
    let Some(m) = map.battleground.as_ref() else {
        return true;
    };
    let data = &map.realm.data;
    let bindings = if creature {
        data.bg_creature_events.get(&spawn)
    } else {
        data.bg_object_events.get(&spawn)
    };
    match bindings {
        None => true,
        Some(b) => b.iter().any(|(e, s)| m.events.get(e) == Some(&Some(*s))),
    }
}

/// The events of a spawn.
pub fn bindings(map: &Map, creature: bool, spawn: Option<u32>) -> Vec<(u8, u8)> {
    let Some(spawn) = spawn else {
        return Vec::new();
    };
    let data = &map.realm.data;
    let b = if creature {
        data.bg_creature_events.get(&spawn)
    } else {
        data.bg_object_events.get(&spawn)
    };
    b.cloned().unwrap_or_default()
}

/// `event` goes to `state` (`None`: off): the spawns of the event that
/// now belong come, and those that don't go.
pub fn set_event(map: &mut Map, event: u8, state: Option<u8>) {
    let Some(m) = map.battleground.as_mut() else {
        return;
    };
    m.events.insert(event, state);
    let data = map.realm.data.clone();
    if let Some(spawns) = data.creature_spawns.get(&map.key.map) {
        for s in spawns {
            let Some(b) = data.bg_creature_events.get(&s.guid) else {
                continue;
            };
            if !b.iter().any(|(e, _)| *e == event) {
                continue;
            }
            let allowed = spawn_allowed(map, true, s.guid);
            let present: Vec<Guid> = map
                .creatures
                .iter()
                .filter(|(_, c)| c.spawn_guid == Some(s.guid))
                .map(|(_, c)| c.guid)
                .collect();
            if allowed && present.is_empty() {
                if let Some(c) = Creature::from_spawn(&data, s, &mut rand::rng()) {
                    map.add_creature(c);
                }
            } else if !allowed {
                for g in present {
                    map.remove_creature(g);
                }
            }
        }
    }
    if let Some(spawns) = data.gameobject_spawns.get(&map.key.map) {
        for s in spawns {
            let Some(b) = data.bg_object_events.get(&s.guid) else {
                continue;
            };
            if !b.iter().any(|(e, _)| *e == event) {
                continue;
            }
            let allowed = spawn_allowed(map, false, s.guid);
            let present: Vec<Guid> = map
                .gameobjects
                .iter()
                .filter(|(_, g)| g.spawn_guid == Some(s.guid))
                .map(|(_, g)| g.guid)
                .collect();
            if allowed && present.is_empty() {
                if let Some(t) = data.gameobject_templates.get(&s.entry) {
                    map.add_gameobject(GameObject::from_spawn(t, s));
                }
            } else if !allowed {
                for g in present {
                    map.remove_gameobject(g);
                }
            }
        }
    }
}

/// An event goes off without taking its spawns away: those alive stay, the
/// dead don't come back.
pub fn stop_respawns(map: &mut Map, event: u8) {
    if let Some(m) = map.battleground.as_mut() {
        m.events.insert(event, None);
    }
}

/// A battleground message to everyone in the match, or one side.
pub fn announce(map: &Map, s: Option<usize>, text: &str) {
    announce_by(map, s, text, Guid::zero());
}

/// A battleground message, from the player it tells of.
pub fn announce_by(map: &Map, s: Option<usize>, text: &str, actor: Guid) {
    let chat_type = match s {
        Some(0) => SMSG_MESSAGECHAT_ChatType::BgSystemAlliance { sender2: actor },
        Some(_) => SMSG_MESSAGECHAT_ChatType::BgSystemHorde { sender2: actor },
        None => SMSG_MESSAGECHAT_ChatType::BgSystemNeutral { sender2: actor },
    };
    let Some(frame) = crate::game::conn::encode(&SMSG_MESSAGECHAT {
        chat_type,
        language: Language::Universal,
        message: text.to_owned(),
        tag: PlayerChatTag::None,
    }) else {
        return;
    };
    for p in map.players.values() {
        p.outbound.send_frame(frame.clone());
    }
}

/// A broadcast text as a battleground message, its `$1776W`-style world
/// states filled in.
pub fn announce_text(map: &Map, s: Option<usize>, id: u32) {
    announce_text_by(map, s, id, Guid::zero());
}

/// A broadcast text as a battleground message about `actor`: their name
/// for its `$n`, and they its sender.
pub fn announce_text_by(map: &Map, s: Option<usize>, id: u32, actor: Guid) {
    let Some(line) = map.realm.data.npc.broadcast_line(id, false) else {
        return;
    };
    let mut text = fill_world_states(map, line);
    if !actor.is_zero() && text.contains("$n") {
        let name = map
            .players
            .get(&actor)
            .map(|p| p.name.to_string())
            .unwrap_or_else(|| "Unknown".into());
        text = text.replace("$n", &name).replace("$N", &name);
    }
    announce_by(map, s, &text, actor);
}

fn fill_world_states(map: &Map, line: &str) -> String {
    let Some(m) = map.battleground.as_ref() else {
        return line.to_owned();
    };
    let states = m.world_states();
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() && tail[digits.len()..].starts_with('W') {
            let id: u32 = digits.parse().unwrap_or(0);
            let v = states.iter().find(|w| w.state == id).map_or(0, |w| w.value);
            out.push_str(&v.to_string());
            rest = &tail[digits.len() + 1..];
        } else {
            out.push('$');
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

pub fn world_state(map: &Map, state: u32, value: u32) {
    let Some(frame) = crate::game::conn::encode(&SMSG_UPDATE_WORLD_STATE {
        state: WorldState { state, value },
    }) else {
        return;
    };
    for p in map.players.values() {
        p.outbound.send_frame(frame.clone());
    }
}

/// Every world state the match shows, sent again.
pub fn send_world_states(map: &Map) {
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    for w in m.world_states() {
        world_state(map, w.state, w.value);
    }
}

pub fn play_sound(map: &Map, sound: u32) {
    let Some(frame) = crate::game::conn::encode(&SMSG_PLAY_SOUND { sound_id: sound }) else {
        return;
    };
    for p in map.players.values() {
        p.outbound.send_frame(frame.clone());
    }
}

/// The players of a side in the match.
pub fn players_of(map: &Map, s: usize) -> Vec<Guid> {
    let Some(m) = map.battleground.as_ref() else {
        return Vec::new();
    };
    let mut out: Vec<Guid> = map
        .players
        .keys()
        .copied()
        .filter(|g| m.team_of(*g) == Some(s))
        .collect();
    out.sort();
    out
}

/// A spell cast on every player of a side.
pub fn team_spell(map: &mut Map, s: usize, spell: u32) {
    for g in players_of(map, s) {
        crate::game::map::spells::cast_triggered(map, g, spell, Some(g));
    }
}

/// Bonus honor for every player of a side.
pub fn reward_team_honor(map: &mut Map, s: usize, amount: u32) {
    if amount == 0 {
        return;
    }
    for g in players_of(map, s) {
        if let Some(m) = map.battleground.as_mut()
            && let Some(score) = m.scores.get_mut(&g)
        {
            // As the honor itself, at the server's rate.
            score.bonus_honor += crate::config::scaled(amount, map.cfg.rates.honor);
        }
        crate::game::map::pvp::bonus_honor(map, g, amount);
    }
}

/// Reputation for every player of a side: the battleground's faction.
pub fn reward_team_reputation(map: &mut Map, s: usize, faction: u32, amount: i32) {
    for g in players_of(map, s) {
        crate::game::map::pvp::reputation(map, g, faction, amount);
    }
}

/// Kill credit (for a quest) for every player of a side.
pub fn team_kill_credit(map: &mut Map, s: usize, entry: u32) {
    for g in players_of(map, s) {
        crate::game::map::quests::killed(map, g, guid::unit(entry, 0));
    }
}

/// A player in the match's map.
pub fn arrived(map: &mut Map, g: Guid) {
    let now = map.now;
    let delay = map.cfg.bg_start_delay;
    let key = map.key;
    let realm = map.realm.clone();
    let Some(team) = map.players.get(&g).map(|p| side(p.team)) else {
        return;
    };
    let Some(m) = map.battleground.as_mut() else {
        return;
    };
    m.scores.entry(g).or_insert(Score {
        team,
        ..Default::default()
    });
    m.resurrect_queue.remove(&g);
    if m.phase == Phase::Waiting {
        m.phase = Phase::Starting { at: now + delay };
        if let Some(b) = realm.social.battlegrounds.lock().matches.get(&key.instance) {
            m.bracket = b.bracket;
        }
        m.weekend = realm.social.events.lock().contains(&weekend_event(m.bg));
    }
    let states = m.world_states();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    p.fields.set_flag(
        UNIT_FIELD_FLAGS,
        crate::game::map::player::UNIT_FLAG_PVP,
        true,
    );
    // No longer AFK (mangos's `AddPlayer`), and into their side's raid.
    use crate::game::map::player::PLAYER_FLAGS_AFK;
    if p.fields.has_flag(PLAYER_FLAGS, PLAYER_FLAGS_AFK) {
        p.fields.set_flag(PLAYER_FLAGS, PLAYER_FLAGS_AFK, false);
        p.online
            .flags
            .fetch_and(!PLAYER_FLAGS_AFK, std::sync::atomic::Ordering::Relaxed);
    }
    let name = p.online.name.clone();
    if realm
        .social
        .battlegrounds
        .lock()
        .matches
        .get(&key.instance)
        .is_some_and(|b| b.members.contains_key(&g))
    {
        crate::game::social::groups::bg_joined(&realm, g, name, key.instance, team as u8);
    }
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    p.outbound.send(&SMSG_INIT_WORLD_STATES {
        map: crate::game::map::movement::map_enum(key.map),
        area: Area::try_from(p.zone).unwrap_or_default(),
        states,
    });
    // Just arrived: worth no honor for a moment.
    crate::game::map::spells::cast_triggered(
        map,
        g,
        crate::game::map::pvp::SPELL_HONORLESS_TARGET,
        Some(g),
    );
    if let Some(frame) = crate::game::conn::encode(&SMSG_BATTLEGROUND_PLAYER_JOINED { player: g }) {
        for (o, other) in map.players.iter() {
            if *o != g {
                other.outbound.send_frame(frame.clone());
            }
        }
    }
}

/// A player leaving the match's map: a flag they carry goes home.
pub fn departing(map: &mut Map, g: Guid) {
    if map.battleground.is_none() {
        return;
    }
    wsg::departing(map, g);
    if let Some(m) = map.battleground.as_mut() {
        m.resurrect_queue.remove(&g);
    }
    if let Some(frame) = crate::game::conn::encode(&SMSG_BATTLEGROUND_PLAYER_LEFT { guid: g }) {
        for (o, other) in map.players.iter() {
            if *o != g {
                other.outbound.send_frame(frame.clone());
            }
        }
    }
}

/// The match's clock: the countdown and the start, its timers, the
/// resurrection waves, and the close.
pub fn update(map: &mut Map) {
    let now = map.now;
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    match m.phase {
        Phase::Starting { at } => {
            let left = at.saturating_duration_since(now);
            countdown(map, left);
            if at <= now {
                start(map);
            }
        }
        Phase::Ended { at, .. } if now.duration_since(at) >= map.cfg.bg_auto_leave => {
            let everyone: Vec<Guid> = map.players.keys().copied().collect();
            for g in everyone {
                battlegrounds::leave(map, g);
            }
        }
        _ => {}
    }
    run_timers(map);
    if map.battleground.as_ref().is_some_and(|m| m.playing()) {
        match map.battleground.as_ref().map(|m| m.bg) {
            Some(BG_AB) => ab::update(map),
            Some(BG_AV) => av::update(map),
            _ => wsg::update(map),
        }
        buffs(map);
    }
    premature(map);
    resurrect_wave(map);
}

/// A side short of players while the battle is on (mangos's
/// `BattleGround::Update`, its balance system): told each minute, then
/// each 15 seconds, and at `[battleground] premature_finish_secs` the
/// match ends, won by the side ahead, else the larger, else no one.
fn premature(map: &mut Map) {
    let now = map.now;
    let after = map.cfg.bg_premature_finish;
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    let bg = m.bg;
    let short = m.playing() && !after.is_zero() && {
        let min = battlegrounds::min_players(&map.realm, bg) as usize;
        battlegrounds::sides(&map.realm, map.key)
            .iter()
            .any(|n| *n < min)
    };
    let Some(m) = map.battleground.as_mut() else {
        return;
    };
    if !short {
        m.premature = None;
        return;
    }
    let Some(at) = m.premature else {
        m.premature = Some(now + after);
        m.premature_left = after;
        return;
    };
    if at <= now {
        m.premature = None;
        let sides = battlegrounds::sides(&map.realm, map.key);
        let winner = premature_winner(map.battleground.as_ref().expect("above"), sides);
        match winner {
            0 => announce(map, None, "The Alliance wins!"),
            1 => announce(map, None, "The Horde wins!"),
            _ => {}
        }
        finish(map, winner);
        return;
    }
    let (old, new) = (m.premature_left.as_millis(), (at - now).as_millis());
    m.premature_left = at - now;
    let text = if new > 60_000 {
        (new / 60_000 != old / 60_000).then(|| {
            format!(
                "Not enough players. This game will close in {} mins.",
                old / 60_000
            )
        })
    } else {
        (new / 15_000 != old / 15_000).then(|| {
            format!(
                "Not enough players. This game will close in {} seconds.",
                old / 1000
            )
        })
    };
    if let Some(text) = text {
        for p in map.players.values() {
            crate::game::map::chat::send_system(&p.outbound, &text);
        }
    }
}

/// Who wins a match ended early: the side ahead (Warsong Gulch's
/// captures, Arathi Basin's resources; Alterac Valley keeps no score),
/// else the side with more players, else no one (mangos's
/// `GetPrematureWinner`).
fn premature_winner(m: &Match, sides: [usize; 2]) -> usize {
    let [a, h] = if m.bg == BG_AV { [0, 0] } else { m.score };
    if a != h {
        return usize::from(h > a);
    }
    match sides[0].cmp(&sides[1]) {
        std::cmp::Ordering::Greater => 0,
        std::cmp::Ordering::Less => 1,
        std::cmp::Ordering::Equal => NO_WINNER,
    }
}

/// The countdown's announcements, a minute and half a minute before.
fn countdown(map: &mut Map, left: Duration) {
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    let (minute, half) = match m.bg {
        BG_AB => (10477, 10478),
        BG_AV => (10638, 10639),
        _ => wsg::COUNTDOWN,
    };
    let done = m.countdown;
    let say = if left <= Duration::from_secs(30) && done < 2 {
        Some((half, 2))
    } else if left <= Duration::from_secs(60) && left > Duration::from_secs(30) && done < 1 {
        Some((minute, 1))
    } else {
        None
    };
    if let Some((text, step)) = say {
        // A start too soon for either is announced only when it comes.
        if left > Duration::ZERO {
            announce_text(map, None, text);
        }
        if let Some(m) = map.battleground.as_mut() {
            m.countdown = step;
        }
    }
}

fn run_timers(map: &mut Map) {
    let now = map.now;
    let due: Vec<Due> = {
        let Some(m) = map.battleground.as_mut() else {
            return;
        };
        if m.timers.iter().all(|(at, _)| *at > now) {
            return;
        }
        let (due, later): (Vec<_>, Vec<_>) = m.timers.drain(..).partition(|(at, _)| *at <= now);
        m.timers = later;
        due.into_iter().map(|(_, d)| d).collect()
    };
    for d in due {
        if !map.battleground.as_ref().is_some_and(|m| m.playing()) {
            continue;
        }
        match d {
            Due::Buff(slot) => spawn_buff(map, slot),
            Due::StatusRefresh => {
                battlegrounds::status_to_members(&map.realm, map.key);
                scoreboard_all(map);
            }
            _ => match map.battleground.as_ref().map(|m| m.bg) {
                Some(BG_AB) => ab::due(map, d),
                Some(BG_AV) => av::due(map, d),
                _ => wsg::due(map, d),
            },
        }
    }
}

/// The doors open and the battle begins.
fn start(map: &mut Map) {
    let now = map.now;
    // The doors of event 254 open; with none, every door does.
    let doors: Vec<Guid> = map
        .gameobjects
        .iter()
        .filter(|(_, g)| {
            let b = bindings(map, false, g.spawn_guid);
            if b.is_empty() {
                g.kind == 0
            } else {
                b.iter().any(|(e, _)| *e == EVENT_DOORS)
            }
        })
        .map(|(_, g)| g.guid)
        .collect();
    for d in doors {
        if let Some(g) = map.gameobject_mut(d) {
            g.fields.set_u32(GAMEOBJECT_STATE, 0);
        }
    }
    set_event(map, EVENT_GHOST_GATES, None);
    if let Some(m) = map.battleground.as_mut() {
        m.phase = Phase::Playing;
        m.started_at = Some(now);
        m.schedule(now + Duration::from_secs(1), Due::StatusRefresh);
    }
    battlegrounds::began(&map.realm, map.key);
    match map.battleground.as_ref().map(|m| m.bg) {
        Some(BG_AB) => ab::start(map),
        Some(BG_AV) => av::start(map),
        _ => wsg::start(map),
    }
}

/// Resurrection waves: every 30 seconds the dead waiting rise at their
/// graveyard.
fn resurrect_wave(map: &mut Map) {
    let now = map.now;
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    if m.resurrect_at > now || matches!(m.phase, Phase::Ended { .. }) {
        return;
    }
    let queued = m.resurrect_queue.clone();
    if let Some(m) = map.battleground.as_mut() {
        m.resurrect_at = now + RESURRECT_EVERY;
        m.resurrect_queue.clear();
    }
    // Those queued at a spirit guide (or by releasing) rise, if still dead
    // and Waiting to Resurrect: one who cancelled the aura has left the
    // queue (Thistle Tea's `Resurrection.ready?`).
    let known = map
        .realm
        .data
        .spells
        .get(SPELL_WAITING_TO_RESURRECT)
        .is_some();
    let mut ghosts: Vec<Guid> = map
        .players
        .values()
        .filter(|p| {
            queued.contains(&p.guid)
                && p.death != Death::Alive
                && (!known || p.auras.has_spell(SPELL_WAITING_TO_RESURRECT))
        })
        .map(|p| p.guid)
        .collect();
    ghosts.sort();
    for g in ghosts {
        let Some((at, o)) = graveyard(map, g) else {
            continue;
        };
        crate::game::map::death::resurrect(map, g, 1.0);
        map.teleport(g, map.key, at, o);
    }
}

/// A player fell in the match: their insignia to take, and [`defeated`].
pub fn died(map: &mut Map, g: Guid, killer: Guid) {
    insignia::fallen(map, g);
    defeated(map, g, killer, true);
}

/// A player of the match was defeated, while the battle is on: their death
/// (`count_death`; not a priest's Spirit of Redemption, which dies later),
/// the killing blow for an enemy of theirs in the match, and the honorable
/// kill for that enemy and their side near: within 74 yards of the fall,
/// themselves or, dead, their corpse (Thistle Tea's `Roster.
/// update_death_scores` and `KillReward.in_range?`, mangos's
/// `IsAtGroupRewardDistance`); a flag they carried drops. One not in the
/// match, a fall before the gates open or after the win, counts for
/// nothing.
pub fn defeated(map: &mut Map, g: Guid, killer: Guid, count_death: bool) {
    let pos = map.position_of(g);
    let Some(m) = map.battleground.as_mut() else {
        return;
    };
    let Some(victim_team) = m.team_of(g).filter(|_| m.playing()) else {
        return;
    };
    if count_death && let Some(s) = m.scores.get_mut(&g) {
        s.deaths += 1;
    }
    let killer_team = m
        .team_of(killer)
        .filter(|t| *t != victim_team && guid::is_player(killer));
    if let Some(kt) = killer_team
        && map.players.contains_key(&killer)
    {
        if let Some(s) = map
            .battleground
            .as_mut()
            .and_then(|m| m.scores.get_mut(&killer))
        {
            s.killing_blows += 1;
        }
        let reach = map.cfg.players.group_reward_yards;
        let near =
            |at: Vector3d| pos.is_some_and(|p| crate::game::map::combat::dist(p, at) <= reach);
        let credited: Vec<Guid> = map
            .players
            .values()
            .filter(|p| {
                p.guid == killer
                    || near(p.position())
                    || (p.death != Death::Alive
                        && map
                            .corpses
                            .get(&guid::corpse(guid::counter(p.guid)))
                            .is_some_and(|c| near(c.position)))
            })
            .map(|p| p.guid)
            .collect();
        if let Some(m) = map.battleground.as_mut() {
            for n in credited {
                if let Some(s) = m.scores.get_mut(&n)
                    && s.team == kt
                {
                    s.honorable_kills += 1;
                }
            }
        }
    }
    if map.battleground.as_ref().is_some_and(|m| m.bg == BG_WSG) {
        wsg::died(map, g);
    }
}

/// A quest turned in during the match.
pub fn quest_rewarded(map: &mut Map, g: Guid, quest: u32) {
    if map.battleground.as_ref().is_some_and(|m| m.bg == BG_AV) {
        av_quests::quest_rewarded(map, g, quest);
    }
}

/// `CMSG_GOSSIP_HELLO` with an NPC of the match's own: whether it was one.
pub fn gossip_hello(map: &mut Map, g: Guid, npc: Guid) -> bool {
    map.battleground.as_ref().is_some_and(|m| m.bg == BG_AV) && av_quests::gossip_hello(map, g, npc)
}

/// `CMSG_GOSSIP_SELECT_OPTION` on such an NPC's menu.
pub fn gossip_select(map: &mut Map, g: Guid, npc: Guid, option: u32) -> bool {
    map.battleground.as_ref().is_some_and(|m| m.bg == BG_AV)
        && av_quests::gossip_select(map, g, npc, option)
}

/// A creature fell in the match.
/// A battleground creature's script reports `event` (Thistle Tea's
/// `battleground_event`), as Alterac Valley's air, cavalry and ground
/// assaults count them: the hook their rules fill in.
pub fn creature_event(_map: &mut Map, _creature: Guid, _event: u32) {}

pub fn creature_died(map: &mut Map, creature: Guid, killer: Guid) {
    if map.battleground.as_ref().is_some_and(|m| m.bg == BG_AV) {
        av::creature_died(map, creature, killer);
    }
}

/// An area trigger: Warsong Gulch's captures, and each battleground's way
/// out.
pub fn area_trigger(map: &mut Map, g: Guid, trigger: u32) {
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    let team = m.team_of(g);
    let exit = matches!(
        (m.bg, trigger, team),
        (BG_AB, 3948, Some(0))
            | (BG_AB, 3949, Some(1))
            | (BG_AV, 2608, Some(0))
            | (BG_AV, 2606, Some(1))
    );
    if exit {
        battlegrounds::leave(map, g);
        return;
    }
    if m.bg == BG_WSG {
        wsg::area_trigger(map, g, trigger);
    }
}

/// `CMSG_GAMEOBJ_USE` in the match: a flag or a banner. Returns whether it
/// was one.
pub fn use_object(map: &mut Map, g: Guid, object: Guid) -> bool {
    let Some(m) = map.battleground.as_ref() else {
        // A mine's supplies with no match here are no one's.
        return av::supplies_barred(map, None, object);
    };
    let near = match (map.players.get(&g), map.gameobject(object)) {
        (Some(p), Some(o)) => crate::game::map::combat::dist(p.position(), o.position) <= 10.0,
        _ => false,
    };
    let alive = map.players.get(&g).is_some_and(|p| p.death == Death::Alive);
    match m.bg {
        BG_AB => near && alive && ab::use_object(map, g, object),
        BG_AV => {
            // A mine's supplies are its holders' alone.
            if av::supplies_barred(map, m.team_of(g), object) {
                return true;
            }
            near && alive
                && (av_scripts::use_ritual(map, g, object)
                    // A beacon is disarmed by the Attacking cast.
                    || av_quests::is_beacon(map, object)
                    || av::use_object(map, g, object))
        }
        _ => wsg::use_object(map, g, object),
    }
}

/// A lock opened by a spell (the client's Opening and Attacking casts):
/// in the match, a banner assaulted, a beacon disarmed; out of one,
/// silithyst picked up. Returns whether it was one.
pub fn opened(map: &mut Map, g: Guid, object: Guid) -> bool {
    let Some(m) = map.battleground.as_ref() else {
        // Out of a match: Silithus's silithyst.
        return crate::game::map::outdoor_pvp::use_object(map, g, object);
    };
    if m.bg == BG_AV && av_quests::use_beacon(map, g, object) {
        return true;
    }
    use_object(map, g, object)
}

/// The spells that plant Alterac Valley's beacons, by the side they're
/// the Alliance's or the Horde's.
const BEACON_PLANTS: [(u32, usize); 6] = [
    (21728, 0),
    (21729, 0),
    (21730, 0),
    (21355, 1),
    (21370, 1),
    (21371, 1),
];
/// Alterac Valley's own: its recalls and battle standards.
const VALLEY_SPELLS: [u32; 4] = [22563, 22564, 23538, 23539];
const ATTR_EX3_ONLY_BATTLEGROUNDS: u32 = 0x800;
const FAIL_REQUIRES_AREA: u8 = 0x5d;
const FAIL_ONLY_BATTLEGROUNDS: u8 = 0x87;

/// Whether `s` plants a beacon.
pub fn plants(s: &crate::game::data::spells::SpellTemplate) -> bool {
    BEACON_PLANTS.iter().any(|(id, _)| *id == s.id)
}

/// Why `caster` can't cast a battleground's spell here, as Thistle Tea's
/// `Spell.Battleground` has it: a beacon only in Alterac Valley's battle
/// and by its side, the valley's own spells only there, and the rest
/// marked for battlegrounds only in one.
pub fn spell_check(
    map: &Map,
    caster: Guid,
    s: &crate::game::data::spells::SpellTemplate,
) -> Option<u8> {
    if !guid::is_player(caster) {
        return None;
    }
    let m = map.battleground.as_ref();
    let valley = m.filter(|m| m.bg == BG_AV);
    if let Some((_, side)) = BEACON_PLANTS.iter().find(|(id, _)| *id == s.id) {
        let ok = valley.is_some_and(|m| m.playing() && m.team_of(caster) == Some(*side));
        return (!ok).then_some(FAIL_REQUIRES_AREA);
    }
    if VALLEY_SPELLS.contains(&s.id) {
        let ok = valley.is_some_and(|m| m.playing() || matches!(m.phase, Phase::Ended { .. }));
        return (!ok).then_some(FAIL_REQUIRES_AREA);
    }
    if s.attr(3, ATTR_EX3_ONLY_BATTLEGROUNDS) && m.is_none() {
        return Some(FAIL_ONLY_BATTLEGROUNDS);
    }
    None
}

/// A beacon planted where `caster` stands: Alliance's or the Horde's, no
/// one's own, and it outlasts its planter.
pub fn plant(map: &mut Map, caster: Guid, entry: u32) {
    let Some((at, o)) = map
        .players
        .get(&caster)
        .map(|p| (p.position(), p.movement.orientation))
    else {
        return;
    };
    let data = map.realm.data.clone();
    let Some(t) = data.gameobject_templates.get(&entry) else {
        return;
    };
    let side = usize::from(!matches!(entry, 178724..=178726));
    let counter = map.object_counter();
    let mut go = GameObject::from_spawn(
        t,
        &crate::game::data::GameObjectSpawn {
            guid: counter,
            entry,
            map: map.key.map,
            x: at.x,
            y: at.y,
            z: at.z,
            orientation: o,
            rotation: [0.0, 0.0, (o / 2.0).sin(), (o / 2.0).cos()],
            state: 1,
            anim_progress: 100,
            ..Default::default()
        },
    );
    go.spawn_guid = None;
    go.fields.set_u32(GAMEOBJECT_FACTION, [83, 84][side]);
    go.fields.set_u32(GAMEOBJECT_LEVEL, 0);
    map.add_gameobject(go);
}

/// Where a ghost goes in a match: its side's start before the battle,
/// then the battleground's graveyard for it.
pub fn graveyard(map: &Map, g: Guid) -> Option<(Vector3d, f32)> {
    let m = map.battleground.as_ref()?;
    let team = map.players.get(&g)?.team;
    if matches!(m.phase, Phase::Waiting | Phase::Starting { .. }) {
        return Some(battlegrounds::start_of(&map.realm, m.bg, team));
    }
    let pos = map.position_of(g)?;
    let near = match m.bg {
        BG_AB => ab::graveyard(map, side(team), pos),
        BG_AV => av::graveyard(map, side(team), pos),
        _ => None,
    };
    near.or_else(|| team_graveyard(map, m.bg, side(team)))
        .or_else(|| Some(battlegrounds::start_of(&map.realm, m.bg, team)))
}

/// A side's own graveyard in a battleground (`WorldSafeLocs`).
pub fn team_graveyard(map: &Map, bg: u32, s: usize) -> Option<(Vector3d, f32)> {
    let id = match (bg, s) {
        (BG_AV, 0) => 611,
        (BG_AV, _) => 610,
        (BG_WSG, 0) => 771,
        (BG_WSG, _) => 772,
        (BG_AB, 0) => 898,
        (BG_AB, _) => 899,
        _ => return None,
    };
    safe_loc(map, id)
}

pub fn safe_loc(map: &Map, id: u32) -> Option<(Vector3d, f32)> {
    map.realm
        .data
        .safe_locs
        .get(&id)
        .map(|&(_, x, y, z)| (Vector3d { x, y, z }, 0.0))
}

/// The nearest of `ids` to `pos`.
pub fn nearest_safe_loc(map: &Map, ids: &[u32], pos: Vector3d) -> Option<(Vector3d, f32)> {
    ids.iter()
        .filter_map(|id| safe_loc(map, *id))
        .min_by(|a, b| {
            let d = |v: Vector3d| (v.x - pos.x).powi(2) + (v.y - pos.y).powi(2);
            d(a.0).total_cmp(&d(b.0))
        })
}

/// `MSG_PVP_LOG_DATA`: the scoreboard.
pub fn scoreboard(map: &Map, g: Guid) {
    let Some(frame) = scoreboard_frame(map) else {
        return;
    };
    if let Some(p) = map.players.get(&g) {
        p.outbound.send_frame(frame);
    }
}

/// The scoreboard to everyone in the match.
pub fn scoreboard_all(map: &Map) {
    let Some(frame) = scoreboard_frame(map) else {
        return;
    };
    for p in map.players.values() {
        p.outbound.send_frame(frame.clone());
    }
}

fn scoreboard_frame(map: &Map) -> Option<std::sync::Arc<[u8]>> {
    let m = map.battleground.as_ref()?;
    let columns = match m.bg {
        BG_AV => 7,
        _ => 2,
    };
    let status = match m.phase {
        Phase::Ended { winner, .. } => MSG_PVP_LOG_DATA_Server_BattlegroundEndStatus::Ended {
            winner: match winner {
                0 => BattlegroundWinner::Alliance,
                1 => BattlegroundWinner::Horde,
                _ => BattlegroundWinner::None,
            },
        },
        _ => MSG_PVP_LOG_DATA_Server_BattlegroundEndStatus::NotEnded,
    };
    let mut rows: Vec<(&Guid, &Score)> = m.scores.iter().collect();
    rows.sort_by_key(|(g, _)| **g);
    let honor = map.realm.social.honor.lock();
    let players = rows
        .into_iter()
        .map(|(g, s)| BattlegroundPlayer {
            player: *g,
            rank: PvpRank::try_from(honor.rank(guid::counter(*g))).unwrap_or(PvpRank::NoRank),
            killing_blows: s.killing_blows,
            honorable_kills: s.honorable_kills,
            deaths: s.deaths,
            bonus_honor: s.bonus_honor,
            fields: s.objectives[..columns].to_vec(),
        })
        .collect();
    drop(honor);
    crate::game::conn::encode(&MSG_PVP_LOG_DATA_Server { status, players })
}

/// The match is won (by [`NO_WINNER`], ended even): the end's
/// announcement, the win and loss spells, the scoreboard, and two minutes
/// until everyone goes home.
pub fn finish(map: &mut Map, winner: usize) {
    let now = map.now;
    let bg = map.battleground.as_ref().map_or(BG_WSG, |m| m.bg);
    if map
        .battleground
        .as_ref()
        .is_none_or(|m| matches!(m.phase, Phase::Ended { .. }))
    {
        return;
    }
    if bg == BG_WSG {
        wsg::finished(map);
    }
    if let Some(m) = map.battleground.as_mut() {
        m.phase = Phase::Ended { winner, at: now };
        m.timers.clear();
        m.resurrect_queue.clear();
    }
    battlegrounds::ended(&map.realm, map.key);
    // Everyone hears how long until they're sent home.
    battlegrounds::status_to_members(&map.realm, map.key);
    let template = map.realm.data.battlegrounds.get(&bg).cloned();
    let everyone: Vec<(Guid, usize)> = map
        .players
        .values()
        .map(|p| (p.guid, side(p.team)))
        .collect();
    for (g, s) in &everyone {
        if let Some(t) = &template {
            let spell = if *s == winner {
                t.win_spells[*s]
            } else {
                t.lose_spells[*s]
            };
            if spell != 0 {
                crate::game::map::spells::cast_triggered(map, *g, spell, Some(*g));
            }
        }
    }
    clear_buffs(map);
    scoreboard_all(map);
}

/// Arathi Basin's pickups: speed, restoration and berserking, one at each
/// place, back three minutes after it's taken.
pub const BUFF_ENTRIES: [u32; 3] = [179871, 179904, 179905];
pub const BUFF_RESPAWN: Duration = Duration::from_secs(180);
const BUFF_RADIUS: f32 = 3.0;

pub fn start_buffs(map: &mut Map, places: &[(f32, f32, f32, f32)]) {
    if let Some(m) = map.battleground.as_mut() {
        m.buffs = places
            .iter()
            .map(|at| BuffSlot {
                at: *at,
                object: None,
            })
            .collect();
    }
    for i in 0..places.len() {
        spawn_buff(map, i);
    }
}

fn spawn_buff(map: &mut Map, slot: usize) {
    use rand::RngExt;
    let Some(at) = map
        .battleground
        .as_ref()
        .and_then(|m| m.buffs.get(slot))
        .filter(|b| b.object.is_none())
        .map(|b| b.at)
    else {
        return;
    };
    let entry = BUFF_ENTRIES[rand::rng().random_range(0..BUFF_ENTRIES.len())];
    let data = map.realm.data.clone();
    let Some(t) = data.gameobject_templates.get(&entry) else {
        return;
    };
    let counter = map.object_counter();
    let mut go = GameObject::from_spawn(
        t,
        &crate::game::data::GameObjectSpawn {
            guid: counter,
            entry,
            map: map.key.map,
            x: at.0,
            y: at.1,
            z: at.2,
            orientation: at.3,
            state: 1,
            ..Default::default()
        },
    );
    go.spawn_guid = None;
    let g = map.add_gameobject(go);
    if let Some(m) = map.battleground.as_mut()
        && let Some(b) = m.buffs.get_mut(slot)
    {
        b.object = Some(g);
    }
}

/// A living player on a pickup takes it: its spell, and it's gone for a
/// while.
fn buffs(map: &mut Map) {
    let now = map.now;
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    if m.buffs.is_empty() {
        return;
    }
    let slots: Vec<(usize, Guid, Vector3d)> = m
        .buffs
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            let g = b.object?;
            map.gameobject(g).map(|o| (i, g, o.position))
        })
        .collect();
    for (i, object, at) in slots {
        let taker = map
            .players
            .values()
            .filter(|p| p.death == Death::Alive)
            .find(|p| crate::game::map::combat::dist(p.position(), at) <= BUFF_RADIUS)
            .map(|p| p.guid);
        let Some(taker) = taker else {
            continue;
        };
        let spell = map
            .gameobject(object)
            .and_then(|o| map.realm.data.gameobject_templates.get(&o.entry))
            .map(|t| t.data[3])
            .unwrap_or(0);
        map.remove_gameobject(object);
        if let Some(m) = map.battleground.as_mut() {
            m.buffs[i].object = None;
            m.schedule(now + BUFF_RESPAWN, Due::Buff(i));
        }
        if spell != 0 {
            crate::game::map::spells::cast_triggered(map, taker, spell, Some(taker));
        }
    }
}

fn clear_buffs(map: &mut Map) {
    let objects: Vec<Guid> = map
        .battleground
        .as_mut()
        .map(|m| m.buffs.drain(..).filter_map(|b| b.object).collect())
        .unwrap_or_default();
    for g in objects {
        map.remove_gameobject(g);
    }
}

/// `CMSG_AREA_SPIRIT_HEALER_QUEUE`: a ghost waits for the next wave,
/// Waiting to Resurrect.
pub fn queue_resurrection(map: &mut Map, g: Guid) {
    let dead = map.players.get(&g).is_some_and(|p| p.death != Death::Alive);
    let queued = if let Some(m) = map.battleground.as_mut()
        && dead
        && m.scores.contains_key(&g)
    {
        m.resurrect_queue.insert(g);
        true
    } else {
        false
    };
    if queued {
        waiting(map, g);
    }
}

/// Waiting to Resurrect on a ghost, put straight on as Thistle Tea does
/// (`Aura.apply_spell`): a cast wouldn't land on the dead.
fn waiting(map: &mut Map, g: Guid) {
    if map
        .players
        .get(&g)
        .is_none_or(|p| p.auras.has_spell(SPELL_WAITING_TO_RESURRECT))
    {
        return;
    }
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(SPELL_WAITING_TO_RESURRECT) else {
        return;
    };
    let level = map.unit_view(g).map(|u| u.level).unwrap_or(1);
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].effect == 6 && s.effects[*i].aura != 0)
        .map(|i| (i, crate::game::map::spells::points(s, i, level)))
        .collect();
    let _ = crate::game::map::auras::apply_spell(map, g, g, s, &effects, Default::default());
}

/// A player released their spirit in the match: Waiting to Resurrect, as
/// Thistle Tea gives it, and in the queue for the next wave.
pub fn released(map: &mut Map, g: Guid) {
    if let Some(m) = map.battleground.as_mut()
        && m.scores.contains_key(&g)
    {
        m.resurrect_queue.insert(g);
    }
    waiting(map, g);
}

/// `MSG_BATTLEGROUND_PLAYER_POSITIONS`: where the flag carriers are.
pub fn positions(map: &Map, g: Guid) {
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    let carriers: Vec<Guid> = m
        .flags
        .iter()
        .filter_map(|f| match f {
            Some(Flag::Carried(c)) => Some(*c),
            _ => None,
        })
        .collect();
    let mut b = Body::default();
    b.u32(0);
    let shown: Vec<(Guid, Vector3d)> = carriers
        .iter()
        .filter_map(|c| map.players.get(c).map(|p| (*c, p.position())))
        .collect();
    b.u8(shown.len() as u8);
    for (c, pos) in shown {
        b.u64(c.guid()).f32(pos.x).f32(pos.y);
    }
    if let Some(p) = map.players.get(&g) {
        p.outbound
            .send_frame(b.frame(MSG_BATTLEGROUND_PLAYER_POSITIONS));
    }
}

pub const MSG_BATTLEGROUND_PLAYER_POSITIONS: u16 = 0x2E9;

/// A node's state, as both Arathi Basin and Alterac Valley keep it: who
/// owns it, who is taking it, until when, and a revision that tells stale
/// timers from current ones.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ControlPoint {
    pub owner: Option<usize>,
    pub assaulting: Option<usize>,
    pub capture_at: Option<Instant>,
    pub revision: u32,
}

/// What an assault did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assault {
    Unchanged,
    /// Taken from its owner, or claimed from neutral: captured in time.
    Assaulted,
    /// Its owner took it back.
    Defended,
}

impl ControlPoint {
    pub fn assault(&mut self, team: usize, now: Instant, capture: Duration) -> Assault {
        if self.assaulting == Some(team) || (self.owner == Some(team) && self.assaulting.is_none())
        {
            return Assault::Unchanged;
        }
        self.revision += 1;
        if self.owner == Some(team) {
            self.assaulting = None;
            self.capture_at = None;
            return Assault::Defended;
        }
        self.assaulting = Some(team);
        self.capture_at = Some(now + capture);
        Assault::Assaulted
    }

    /// The assault's time is up: the assaulter owns it.
    pub fn capture(&mut self, revision: u32, now: Instant) -> bool {
        match (self.assaulting, self.capture_at) {
            (Some(team), Some(at)) if self.revision == revision && now >= at => {
                self.owner = Some(team);
                self.assaulting = None;
                self.capture_at = None;
                self.revision += 1;
                true
            }
            _ => false,
        }
    }

    pub fn controlled_by(&self) -> Option<usize> {
        if self.assaulting.is_some() {
            None
        } else {
            self.owner
        }
    }
}
