//! A battleground match, on its copy of the map. Warsong Gulch's rules:
//! the doors open after the countdown, each side steals the other's flag
//! and brings it home while its own is there, and three captures win. A
//! carrier who falls drops the flag; their side picks it up again, the
//! other side returns it. The fallen rise at their side's start every 30
//! seconds. Two minutes after the end, everyone goes home.

use std::time::{Duration, Instant};

use ahash::AHashMap;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::battlegrounds;
use crate::game::data::{GameObjectSpawn, GameObjectTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::death::Death;
use crate::game::map::gameobject::GameObject;
use crate::game::registry::Team;

pub const SPELL_DESERTER: u32 = 26013;
/// The flags' auras on their carriers: the Alliance's flag, the Horde's.
pub const SPELL_FLAG_CARRIED: [u32; 2] = [23335, 23333];
/// Flag stands and dropped flags: the Alliance's, the Horde's.
pub const FLAG_STANDS: [u32; 2] = [179830, 179831];
pub const FLAGS_DROPPED: [u32; 2] = [179785, 179786];
/// Where each side's flag stands.
pub const FLAG_AT: [(f32, f32, f32, f32); 2] = [
    (1540.42, 1481.33, 351.83, 3.09),
    (916.02, 1434.40, 345.41, 0.02),
];
/// Where each side's carriers capture: the area triggers at the
/// Alliance's and the Horde's bases.
pub const CAPTURE_TRIGGERS: [u32; 2] = [3646, 3647];
pub const CAPTURES_TO_WIN: u32 = 3;
pub const RESURRECT_EVERY: Duration = Duration::from_secs(30);
pub const CLOSE_AFTER: Duration = Duration::from_secs(120);

// World states.
pub const WS_CAPTURES: [u32; 2] = [1581, 1582];
pub const WS_CAPTURES_MAX: u32 = 1601;
/// Each side's flag: 1 at its base, 2 carried.
pub const WS_FLAG_STATE: [u32; 2] = [2339, 2338];

/// The map of the battleground a battlemaster sends players to.
pub fn map_for_battlemaster(map: &Map, npc: Guid) -> u32 {
    let bg = map
        .realm
        .data
        .battlemasters
        .get(&guid::entry(npc))
        .copied()
        .unwrap_or(2);
    battlegrounds::map_of(bg)
}

fn side(team: Team) -> usize {
    match team {
        Team::Alliance => 0,
        Team::Horde => 1,
    }
}

fn other(s: usize) -> usize {
    1 - s
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Waiting,
    Starting { at: Instant },
    Playing,
    Ended { winner: usize, at: Instant },
}

/// Where a side's flag is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Flag {
    AtBase(Guid),
    Carried(Guid),
    Dropped(Guid),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Score {
    pub killing_blows: u32,
    pub deaths: u32,
    pub captures: u32,
    pub returns: u32,
}

#[derive(Debug, Clone)]
pub struct Match {
    pub bg: u32,
    pub phase: Phase,
    pub score: [u32; 2],
    /// Each side's flag.
    pub flags: [Option<Flag>; 2],
    pub scores: AHashMap<Guid, Score>,
    pub resurrect_at: Instant,
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
        }
    }

    pub fn world_states(&self) -> Vec<WorldState> {
        let flag_state = |s: usize| match self.flags[s] {
            Some(Flag::Carried(_)) => 2,
            _ => 1,
        };
        vec![
            WorldState {
                state: WS_CAPTURES[0],
                value: self.score[0],
            },
            WorldState {
                state: WS_CAPTURES[1],
                value: self.score[1],
            },
            WorldState {
                state: WS_CAPTURES_MAX,
                value: CAPTURES_TO_WIN,
            },
            WorldState {
                state: WS_FLAG_STATE[0],
                value: flag_state(0),
            },
            WorldState {
                state: WS_FLAG_STATE[1],
                value: flag_state(1),
            },
        ]
    }
}

/// The match a battleground copy plays.
pub fn for_map(key: crate::game::map::MapKey, now: Instant) -> Option<Match> {
    if key.instance == 0 {
        return None;
    }
    let bg = battlegrounds::bg_of_map(key.map)?;
    Some(Match::new(bg, now))
}

fn team_of(map: &Map, g: Guid) -> Option<usize> {
    map.players.get(&g).map(|p| side(p.team))
}

fn announce(map: &Map, s: Option<usize>, text: &str) {
    let chat_type = match s {
        Some(0) => SMSG_MESSAGECHAT_ChatType::BgSystemAlliance {
            sender2: Guid::zero(),
        },
        Some(_) => SMSG_MESSAGECHAT_ChatType::BgSystemHorde {
            sender2: Guid::zero(),
        },
        None => SMSG_MESSAGECHAT_ChatType::BgSystemNeutral {
            sender2: Guid::zero(),
        },
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

fn world_state(map: &Map, state: u32, value: u32) {
    let Some(frame) = crate::game::conn::encode(&SMSG_UPDATE_WORLD_STATE {
        state: WorldState { state, value },
    }) else {
        return;
    };
    for p in map.players.values() {
        p.outbound.send_frame(frame.clone());
    }
}

/// A flag object: a side's stand, or its dropped flag.
fn spawn_flag(map: &mut Map, s: usize, dropped: Option<Vector3d>) -> Guid {
    let entry = if dropped.is_some() {
        FLAGS_DROPPED[s]
    } else {
        FLAG_STANDS[s]
    };
    let data = map.realm.data.clone();
    let template = data
        .gameobject_templates
        .get(&entry)
        .cloned()
        .unwrap_or_else(|| GameObjectTemplate {
            entry,
            kind: if dropped.is_some() { 26 } else { 24 },
            display_id: [5912, 5913][s],
            size: 2.5,
            name: ["Silverwing Flag", "Warsong Flag"][s].into(),
            ..Default::default()
        });
    let (x, y, z, o) = match dropped {
        Some(at) => (at.x, at.y, at.z, 0.0),
        None => FLAG_AT[s],
    };
    let counter = map.next_creature_counter;
    map.next_creature_counter += 1;
    let mut go = GameObject::from_spawn(
        &template,
        &GameObjectSpawn {
            guid: counter | 0x80_0000,
            entry,
            map: map.key.map,
            x,
            y,
            z,
            orientation: o,
            state: 1,
            ..Default::default()
        },
    );
    go.spawn_guid = None;
    map.add_gameobject(go)
}

/// A player in the match's map.
pub fn arrived(map: &mut Map, g: Guid) {
    let now = map.now;
    let delay = map.realm.config.bg_start_delay;
    let Some(m) = map.battleground.as_mut() else {
        return;
    };
    m.scores.entry(g).or_default();
    if m.phase == Phase::Waiting {
        m.phase = Phase::Starting { at: now + delay };
    }
    let states = m.world_states();
    let key = map.key;
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    p.fields.set_flag(
        UNIT_FIELD_FLAGS,
        crate::game::map::player::UNIT_FLAG_PVP,
        true,
    );
    p.outbound.send(&SMSG_INIT_WORLD_STATES {
        map: crate::game::map::movement::map_enum(key.map),
        area: Area::try_from(p.zone).unwrap_or_default(),
        states,
    });
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
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    let carried = (0..2).find(|s| m.flags[*s] == Some(Flag::Carried(g)));
    if let Some(s) = carried {
        return_flag(map, s);
    }
}

/// The match's clock: the start, resurrection waves, and the close.
pub fn update(map: &mut Map) {
    let now = map.now;
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    match m.phase {
        Phase::Starting { at } if at <= now => start(map),
        Phase::Ended { at, .. } if now.duration_since(at) >= CLOSE_AFTER => {
            let everyone: Vec<Guid> = map.players.keys().copied().collect();
            for g in everyone {
                battlegrounds::leave(map, g);
            }
        }
        _ => {}
    }
    if !map
        .battleground
        .as_ref()
        .is_some_and(|m| m.resurrect_at <= now)
    {
        return;
    }
    if let Some(m) = map.battleground.as_mut() {
        m.resurrect_at = now + RESURRECT_EVERY;
    }
    let ghosts: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.death != Death::Alive)
        .map(|p| p.guid)
        .collect();
    for g in ghosts {
        let Some((at, o)) = graveyard(map, g) else {
            continue;
        };
        crate::game::map::death::resurrect(map, g, 1.0);
        map.teleport(g, map.key, at, o);
    }
}

/// The doors open and the flags go up.
fn start(map: &mut Map) {
    let doors: Vec<Guid> = map
        .gameobjects
        .iter()
        .filter(|(_, g)| g.kind == 0)
        .map(|(_, g)| g.guid)
        .collect();
    for d in doors {
        if let Some(g) = map.gameobject_mut(d) {
            g.fields.set_u32(GAMEOBJECT_STATE, 0);
        }
    }
    for s in 0..2 {
        let flag = spawn_flag(map, s, None);
        if let Some(m) = map.battleground.as_mut() {
            m.flags[s] = Some(Flag::AtBase(flag));
        }
    }
    if let Some(m) = map.battleground.as_mut() {
        m.phase = Phase::Playing;
    }
    announce(map, None, "The battle for Warsong Gulch has begun!");
}

fn return_flag(map: &mut Map, s: usize) {
    if let Some(Flag::Dropped(go) | Flag::AtBase(go)) =
        map.battleground.as_ref().and_then(|m| m.flags[s])
    {
        map.remove_gameobject(go);
    }
    let flag = spawn_flag(map, s, None);
    if let Some(m) = map.battleground.as_mut() {
        m.flags[s] = Some(Flag::AtBase(flag));
    }
    world_state(map, WS_FLAG_STATE[s], 1);
}

fn carry(map: &mut Map, g: Guid, s: usize) {
    let go = match map.battleground.as_ref().and_then(|m| m.flags[s]) {
        Some(Flag::AtBase(go) | Flag::Dropped(go)) => go,
        _ => return,
    };
    map.remove_gameobject(go);
    if let Some(m) = map.battleground.as_mut() {
        m.flags[s] = Some(Flag::Carried(g));
    }
    crate::game::map::spells::cast_triggered(map, g, SPELL_FLAG_CARRIED[s], Some(g));
    world_state(map, WS_FLAG_STATE[s], 2);
    let name = map
        .players
        .get(&g)
        .map(|p| p.name.to_string())
        .unwrap_or_default();
    let whose = ["Alliance", "Horde"][s];
    announce(
        map,
        Some(other(s)),
        &format!("The {whose} flag was picked up by {name}!"),
    );
}

/// `CMSG_GAMEOBJ_USE` on a flag. Returns whether it was one.
pub fn use_object(map: &mut Map, g: Guid, object: Guid) -> bool {
    let Some(m) = map.battleground.as_ref() else {
        return false;
    };
    if m.phase != Phase::Playing {
        return false;
    }
    let flags = m.flags;
    let Some(mine) = team_of(map, g) else {
        return false;
    };
    let near = match (map.players.get(&g), map.gameobject(object)) {
        (Some(p), Some(o)) => crate::game::map::combat::dist(p.position(), o.position) <= 10.0,
        _ => false,
    };
    if !near || map.players.get(&g).is_none_or(|p| p.death != Death::Alive) {
        return false;
    }
    for (s, flag) in flags.iter().enumerate() {
        match *flag {
            // The other side's flag, at its stand: taken.
            Some(Flag::AtBase(go)) if go == object && s != mine => {
                carry(map, g, s);
                return true;
            }
            Some(Flag::Dropped(go)) if go == object => {
                if s == mine {
                    // One's own flag, dropped: home again.
                    return_flag(map, s);
                    if let Some(m) = map.battleground.as_mut() {
                        m.scores.entry(g).or_default().returns += 1;
                    }
                    let name = map
                        .players
                        .get(&g)
                        .map(|p| p.name.to_string())
                        .unwrap_or_default();
                    let whose = ["Alliance", "Horde"][s];
                    announce(
                        map,
                        Some(s),
                        &format!("The {whose} flag was returned to its base by {name}!"),
                    );
                } else {
                    carry(map, g, s);
                }
                return true;
            }
            _ => {}
        }
    }
    false
}

/// An area trigger: a carrier home while their own flag is there
/// captures.
pub fn area_trigger(map: &mut Map, g: Guid, trigger: u32) {
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    let Some(mine) = team_of(map, g) else {
        return;
    };
    let theirs = other(mine);
    if m.phase != Phase::Playing
        || CAPTURE_TRIGGERS[mine] != trigger
        || m.flags[theirs] != Some(Flag::Carried(g))
        || !matches!(m.flags[mine], Some(Flag::AtBase(_)))
    {
        return;
    }
    crate::game::map::auras::remove_spell(map, g, SPELL_FLAG_CARRIED[theirs], None);
    let score = {
        let m = map.battleground.as_mut().expect("checked above");
        m.score[mine] += 1;
        m.scores.entry(g).or_default().captures += 1;
        m.flags[theirs] = None;
        m.score[mine]
    };
    return_flag(map, theirs);
    world_state(map, WS_CAPTURES[mine], score);
    let name = map
        .players
        .get(&g)
        .map(|p| p.name.to_string())
        .unwrap_or_default();
    let whose = ["Horde", "Alliance"][mine];
    announce(
        map,
        Some(mine),
        &format!("{name} captured the {whose} flag!"),
    );
    if score >= CAPTURES_TO_WIN {
        win(map, mine);
    }
}

fn win(map: &mut Map, winner: usize) {
    let now = map.now;
    let bg = map.battleground.as_ref().map_or(2, |m| m.bg);
    if let Some(m) = map.battleground.as_mut() {
        m.phase = Phase::Ended { winner, at: now };
    }
    announce(
        map,
        Some(winner),
        ["The Alliance wins!", "The Horde wins!"][winner],
    );
    battlegrounds::ended(&map.realm, map.key);
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
        scoreboard(map, *g);
    }
}

/// `MSG_PVP_LOG_DATA`: the scoreboard.
pub fn scoreboard(map: &Map, g: Guid) {
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    let status = match m.phase {
        Phase::Ended { winner, .. } => MSG_PVP_LOG_DATA_Server_BattlegroundEndStatus::Ended {
            winner: if winner == 0 {
                BattlegroundWinner::Alliance
            } else {
                BattlegroundWinner::Horde
            },
        },
        _ => MSG_PVP_LOG_DATA_Server_BattlegroundEndStatus::NotEnded,
    };
    let players = m
        .scores
        .iter()
        .map(|(p, s)| BattlegroundPlayer {
            player: *p,
            rank: PvpRank::NoRank,
            killing_blows: s.killing_blows,
            honorable_kills: s.killing_blows,
            deaths: s.deaths,
            bonus_honor: 0,
            fields: vec![s.captures, s.returns],
        })
        .collect();
    if let Some(p) = map.players.get(&g) {
        p.outbound
            .send(&MSG_PVP_LOG_DATA_Server { status, players });
    }
}

/// A player fell in the match: counted, and a flag they carried dropped.
pub fn died(map: &mut Map, g: Guid, killer: Guid) {
    let Some(m) = map.battleground.as_mut() else {
        return;
    };
    m.scores.entry(g).or_default().deaths += 1;
    if guid::is_player(killer) && killer != g {
        m.scores.entry(killer).or_default().killing_blows += 1;
    }
    let carried = (0..2).find(|s| m.flags[*s] == Some(Flag::Carried(g)));
    let Some(s) = carried else {
        return;
    };
    crate::game::map::auras::remove_spell(map, g, SPELL_FLAG_CARRIED[s], None);
    let Some(at) = map.players.get(&g).map(|p| p.position()) else {
        return;
    };
    let dropped = spawn_flag(map, s, Some(at));
    if let Some(m) = map.battleground.as_mut() {
        m.flags[s] = Some(Flag::Dropped(dropped));
    }
    world_state(map, WS_FLAG_STATE[s], 1);
    let whose = ["Alliance", "Horde"][s];
    announce(map, Some(s), &format!("The {whose} flag was dropped!"));
}

/// Where a ghost goes in a match: their side's start.
pub fn graveyard(map: &Map, g: Guid) -> Option<(Vector3d, f32)> {
    let m = map.battleground.as_ref()?;
    let team = map.players.get(&g)?.team;
    Some(battlegrounds::start_of(&map.realm, m.bg, team))
}
