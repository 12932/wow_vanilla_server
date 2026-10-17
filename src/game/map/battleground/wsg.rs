//! Warsong Gulch: each side steals the other's flag and brings it home
//! while its own is there; three captures win. A carrier who falls drops
//! the flag; their side picks it up again, the other side returns it.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::{GameObjectSpawn, GameObjectTemplate};
use crate::game::map::Map;
use crate::game::map::battleground::{self as bg, Phase};
use crate::game::map::death::Death;
use crate::game::map::gameobject::GameObject;

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

// World states.
pub const WS_CAPTURES: [u32; 2] = [1581, 1582];
pub const WS_CAPTURES_MAX: u32 = 1601;
/// Each side's flag: 1 at its base, 2 carried.
pub const WS_FLAG_STATE: [u32; 2] = [2339, 2338];

/// Bonus honor by bracket (10-19 to 60): a capture's, and a win's.
const CAPTURE_HONOR: [u32; 6] = [48, 82, 136, 226, 378, 396];
const WIN_HONOR: [u32; 6] = [24, 41, 68, 113, 189, 198];
/// Silverwing Sentinels and Warsong Outriders.
const REPUTATION: [u32; 2] = [890, 889];

/// Scoreboard columns.
const CAPTURES: usize = 0;
const RETURNS: usize = 1;

/// Where a side's flag is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Flag {
    AtBase(Guid),
    Carried(Guid),
    Dropped(Guid),
}

pub fn world_states(m: &bg::Match) -> Vec<(u32, u32)> {
    let flag_state = |s: usize| match m.flags[s] {
        Some(Flag::Carried(_)) => 2,
        _ => 1,
    };
    vec![
        (WS_CAPTURES[0], m.score[0]),
        (WS_CAPTURES[1], m.score[1]),
        (WS_CAPTURES_MAX, CAPTURES_TO_WIN),
        (WS_FLAG_STATE[0], flag_state(0)),
        (WS_FLAG_STATE[1], flag_state(1)),
    ]
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

/// The flags go up.
pub fn start(map: &mut Map) {
    for s in 0..2 {
        let flag = spawn_flag(map, s, None);
        if let Some(m) = map.battleground.as_mut() {
            m.flags[s] = Some(Flag::AtBase(flag));
        }
    }
    bg::announce(map, None, "The battle for Warsong Gulch has begun!");
}

/// A player leaving the match's map: a flag they carry goes home.
pub fn departing(map: &mut Map, g: Guid) {
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    let carried = (0..2).find(|s| m.flags[*s] == Some(Flag::Carried(g)));
    if let Some(s) = carried {
        crate::game::map::auras::remove_spell(map, g, SPELL_FLAG_CARRIED[s], None);
        return_flag(map, s);
    }
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
    bg::world_state(map, WS_FLAG_STATE[s], 1);
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
    bg::world_state(map, WS_FLAG_STATE[s], 2);
    let name = name_of(map, g);
    let whose = ["Alliance", "Horde"][s];
    bg::announce(
        map,
        Some(bg::other(s)),
        &format!("The {whose} flag was picked up by {name}!"),
    );
}

fn name_of(map: &Map, g: Guid) -> String {
    map.players
        .get(&g)
        .map(|p| p.name.to_string())
        .unwrap_or_default()
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
    let Some(mine) = m.team_of(g) else {
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
                        m.scores.entry(g).or_default().objectives[RETURNS] += 1;
                    }
                    let name = name_of(map, g);
                    let whose = ["Alliance", "Horde"][s];
                    bg::announce(
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
    let Some(mine) = m.team_of(g) else {
        return;
    };
    let theirs = bg::other(mine);
    if m.phase != Phase::Playing
        || CAPTURE_TRIGGERS[mine] != trigger
        || m.flags[theirs] != Some(Flag::Carried(g))
        || !matches!(m.flags[mine], Some(Flag::AtBase(_)))
    {
        return;
    }
    crate::game::map::auras::remove_spell(map, g, SPELL_FLAG_CARRIED[theirs], None);
    let (score, bracket) = {
        let m = map.battleground.as_mut().expect("checked above");
        m.score[mine] += 1;
        m.scores.entry(g).or_default().objectives[CAPTURES] += 1;
        m.flags[theirs] = None;
        (m.score[mine], usize::from(m.bracket).min(5))
    };
    bg::reward_team_honor(map, mine, CAPTURE_HONOR[bracket]);
    bg::reward_team_reputation(map, mine, REPUTATION[mine], 35);
    bg::play_sound(map, [8173, 8213][mine]);
    return_flag(map, theirs);
    bg::world_state(map, WS_CAPTURES[mine], score);
    let name = name_of(map, g);
    let whose = ["Horde", "Alliance"][mine];
    bg::announce(
        map,
        Some(mine),
        &format!("{name} captured the {whose} flag!"),
    );
    if score >= CAPTURES_TO_WIN {
        bg::reward_team_honor(map, mine, WIN_HONOR[bracket]);
        bg::announce(
            map,
            Some(mine),
            ["The Alliance wins!", "The Horde wins!"][mine],
        );
        bg::finish(map, mine);
    }
}

/// A carrier fell: the flag drops where they were.
pub fn died(map: &mut Map, g: Guid) {
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
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
    bg::world_state(map, WS_FLAG_STATE[s], 1);
    let whose = ["Alliance", "Horde"][s];
    bg::announce(map, Some(s), &format!("The {whose} flag was dropped!"));
}
