//! Warsong Gulch, as Thistle Tea's `WarsongGulch` runs it: each side
//! steals the other's flag and brings it home while its own is there;
//! three captures win. A carrier who falls, leaves or loses the flag drops
//! it where they are; their side picks it up again, the other side returns
//! it, and left alone it goes home after ten seconds. After a capture both
//! flags are down for 23 seconds, then stand at their bases again.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::{GameObjectSpawn, GameObjectTemplate};
use crate::game::map::Map;
use crate::game::map::auras;
use crate::game::map::battleground::{self as bg, Due, Phase};
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
/// A dropped flag left alone goes home after this; flags captured stand
/// again after that.
pub const FLAG_DROP: Duration = Duration::from_secs(10);
pub const FLAG_RESPAWN: Duration = Duration::from_secs(23);

// World states.
pub const WS_CAPTURES: [u32; 2] = [1581, 1582];
pub const WS_CAPTURES_MAX: u32 = 1601;
/// Each side's flag taken: 0 at its base, 1 carried, -1 on the ground.
pub const WS_TAKEN: [u32; 2] = [1545, 1546];
/// Each side's flag carried by the other (2) or not (1): the Alliance's
/// flag 2338, the Horde's 2339 (the carriers' side of the frame).
pub const WS_FLAG_STATE: [u32; 2] = [2338, 2339];

/// Bonus honor by bracket (10-19 to 60): a capture's, and a win's.
const CAPTURE_HONOR: [u32; 6] = [48, 82, 136, 226, 378, 396];
const WIN_HONOR: [u32; 6] = [24, 41, 68, 113, 189, 198];
/// Silverwing Sentinels and Warsong Outriders.
const REPUTATION: [u32; 2] = [890, 889];

// Broadcast texts, by the flag's side.
const PICKED_UP: [u32; 2] = [9804, 9807];
const DROPPED: [u32; 2] = [9805, 9806];
const RETURNED: [u32; 2] = [9808, 9809];
const WENT_HOME: [u32; 2] = [10022, 10023];
const CAPTURED: [u32; 2] = [9802, 9801];
const PLACED: u32 = 9803;
/// By the winner.
const WINS: [u32; 2] = [9843, 9842];
pub const COUNTDOWN: (u32, u32) = (10015, 10016);
const BEGINS: u32 = 10014;

// Sounds.
const SOUND_PICKED_UP: [u32; 2] = [8174, 8212];
const SOUND_CAPTURED: [u32; 2] = [8173, 8213];
const SOUND_RETURNED: u32 = 8192;
const SOUND_PLACED: u32 = 8232;

/// Scoreboard columns.
const CAPTURES: usize = 0;
const RETURNS: usize = 1;

/// Where a side's flag is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Flag {
    AtBase(Guid),
    Carried(Guid),
    Dropped(Guid),
    /// Captured: down until both stand again.
    Waiting,
}

pub fn world_states(m: &bg::Match) -> Vec<(u32, u32)> {
    let taken = |s: usize| match m.flags[s] {
        Some(Flag::Dropped(_)) => u32::MAX,
        Some(Flag::Carried(_)) => 1,
        _ => 0,
    };
    let carried = |s: usize| match m.flags[s] {
        Some(Flag::Carried(_)) => 2,
        _ => 1,
    };
    vec![
        (WS_CAPTURES[0], m.score[0]),
        (WS_CAPTURES[1], m.score[1]),
        (WS_TAKEN[0], taken(0)),
        (WS_TAKEN[1], taken(1)),
        (WS_CAPTURES_MAX, CAPTURES_TO_WIN),
        (WS_FLAG_STATE[1], carried(1)),
        (WS_FLAG_STATE[0], carried(0)),
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
    let counter = map.object_counter();
    let mut go = GameObject::from_spawn(
        &template,
        &GameObjectSpawn {
            guid: counter,
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

fn set_flag(map: &mut Map, s: usize, flag: Flag) {
    if let Some(m) = map.battleground.as_mut() {
        m.flags[s] = Some(flag);
    }
}

fn flag(map: &Map, s: usize) -> Option<Flag> {
    map.battleground.as_ref().and_then(|m| m.flags[s])
}

/// The flags go up, and the battle begins.
pub fn start(map: &mut Map) {
    for s in 0..2 {
        let flag = spawn_flag(map, s, None);
        set_flag(map, s, Flag::AtBase(flag));
    }
    bg::announce_text(map, None, BEGINS);
    bg::send_world_states(map);
}

/// The flag `g` carries, if they carry one.
fn carried_by(map: &Map, g: Guid) -> Option<usize> {
    let m = map.battleground.as_ref()?;
    (0..2).find(|s| m.flags[*s] == Some(Flag::Carried(g)))
}

/// A player leaving the match's map (or going offline): a flag they carry
/// drops where they are.
pub fn departing(map: &mut Map, g: Guid) {
    if carried_by(map, g).is_some() {
        drop_flag(map, g);
    }
}

/// `s`'s flag back on its stand: by a defender's hand (`by`), or left
/// alone.
fn return_flag(map: &mut Map, s: usize, by: Option<Guid>) {
    if let Some(Flag::Dropped(go)) = flag(map, s) {
        map.remove_gameobject(go);
    }
    let stand = spawn_flag(map, s, None);
    set_flag(map, s, Flag::AtBase(stand));
    match by {
        Some(g) => {
            if let Some(m) = map.battleground.as_mut() {
                m.scores.entry(g).or_default().objectives[RETURNS] += 1;
            }
            bg::announce_text_by(map, Some(s), RETURNED[s], g);
        }
        None => bg::announce_text(map, None, WENT_HOME[s]),
    }
    bg::play_sound(map, SOUND_RETURNED);
    bg::send_world_states(map);
}

fn carry(map: &mut Map, g: Guid, s: usize) {
    let go = match flag(map, s) {
        Some(Flag::AtBase(go) | Flag::Dropped(go)) => go,
        _ => return,
    };
    map.remove_gameobject(go);
    set_flag(map, s, Flag::Carried(g));
    crate::game::map::spells::cast_triggered(map, g, SPELL_FLAG_CARRIED[s], Some(g));
    bg::announce_text_by(map, Some(bg::other(s)), PICKED_UP[s], g);
    bg::play_sound(map, SOUND_PICKED_UP[s]);
    bg::send_world_states(map);
}

/// Whether `g` may take up or return a flag, as Thistle Tea's
/// `battleground_allowed?` has it: not immune to every school (Divine
/// Shield, Ice Block: mangos's `CanUseBattleGroundObject`), stunned,
/// feared or confused, feigning death, possessed, or a Spirit of
/// Redemption. One only partly immune (Blessing of Protection) may; one
/// mounted is dismounted first ([`use_object`]).
fn may_use_flag(map: &Map, g: Guid) -> bool {
    let Some(a) = auras::auras(map, g) else {
        return false;
    };
    let immune = a
        .effects()
        .filter(|(_, x)| x.kind == auras::AURA_SCHOOL_IMMUNITY)
        .fold(0u32, |mask, (_, x)| mask | x.misc as u32)
        & 0x7F
        == 0x7F;
    let held = [
        auras::AURA_MOD_STUN,
        auras::AURA_MOD_FEAR,
        auras::AURA_MOD_CONFUSE,
        auras::AURA_FEIGN_DEATH,
        auras::AURA_MOD_POSSESS,
    ]
    .iter()
    .any(|k| a.has(*k));
    !immune && !held && !crate::game::map::spells::redemption::spirit(map, g)
}

/// `CMSG_GAMEOBJ_USE` on a flag. Returns whether it was one.
pub fn use_object(map: &mut Map, g: Guid, object: Guid) -> bool {
    let Some(m) = map.battleground.as_ref() else {
        return false;
    };
    let flags = m.flags;
    let flag_object = flags
        .iter()
        .any(|f| matches!(f, Some(Flag::AtBase(go) | Flag::Dropped(go)) if *go == object));
    if m.phase != Phase::Playing {
        return flag_object;
    }
    let Some(mine) = m.team_of(g) else {
        return false;
    };
    let near = match (map.players.get(&g), map.gameobject(object)) {
        (Some(p), Some(o)) => crate::game::map::combat::dist(p.position(), o.position) <= 10.0,
        _ => false,
    };
    if !near || map.players.get(&g).is_none_or(|p| p.death != Death::Alive) {
        return flag_object;
    }
    if flag_object {
        if !may_use_flag(map, g) {
            return true;
        }
        // One mounted comes off the mount to take it (mangos's
        // `GameObject::Use`: a flag isn't `IsUsableMounted`, so
        // `RemoveSpellsCausingAura(SPELL_AURA_MOUNTED)`).
        crate::game::map::mounts::dismount(map, g);
        // Using it ends stealth and invisibility (mangos's pickup is no
        // triggered cast).
        auras::remove_where(map, g, |h| {
            h.has(auras::AURA_MOD_STEALTH) || h.has(auras::AURA_MOD_INVISIBILITY)
        });
    }
    for (s, flag) in flags.iter().enumerate() {
        match *flag {
            // The other side's flag, at its stand: taken.
            Some(Flag::AtBase(go)) if go == object && s != mine => {
                carry(map, g, s);
                return true;
            }
            // A flag on the ground: one's own home, the other's taken up.
            Some(Flag::Dropped(go)) if go == object => {
                if s == mine {
                    return_flag(map, s, Some(g));
                } else {
                    carry(map, g, s);
                }
                return true;
            }
            _ => {}
        }
    }
    flag_object
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
    let now = map.now;
    let (score, bracket) = {
        let m = map.battleground.as_mut().expect("checked above");
        m.score[mine] += 1;
        m.scores.entry(g).or_default().objectives[CAPTURES] += 1;
        m.flags[theirs] = Some(Flag::Waiting);
        (m.score[mine], usize::from(m.bracket).min(5))
    };
    bg::reward_team_honor(map, mine, CAPTURE_HONOR[bracket]);
    crate::game::map::auras::remove_spell(map, g, SPELL_FLAG_CARRIED[theirs], None);
    // Both stands come down until the flags are placed again.
    if let Some(Flag::AtBase(go)) = flag(map, mine) {
        map.remove_gameobject(go);
    }
    set_flag(map, mine, Flag::Waiting);
    bg::announce_text_by(map, Some(mine), CAPTURED[theirs], g);
    bg::reward_team_reputation(map, mine, REPUTATION[mine], 35);
    bg::play_sound(map, SOUND_CAPTURED[mine]);
    bg::send_world_states(map);
    if score >= CAPTURES_TO_WIN {
        bg::reward_team_honor(map, mine, WIN_HONOR[bracket]);
        bg::announce_text(map, None, WINS[mine]);
        bg::finish(map, mine);
    } else if let Some(m) = map.battleground.as_mut() {
        m.schedule(now + FLAG_RESPAWN, Due::FlagsPlaced);
    }
}

/// A carrier fell: the flag drops where they were.
pub fn died(map: &mut Map, g: Guid) {
    drop_flag(map, g);
}

/// The flag `g` carries falls where they are, and goes home in ten
/// seconds unless taken up.
fn drop_flag(map: &mut Map, g: Guid) {
    let Some(s) = carried_by(map, g) else {
        return;
    };
    let now = map.now;
    let Some(at) = map.players.get(&g).map(|p| p.position()) else {
        return;
    };
    let team = map.battleground.as_ref().and_then(|m| m.team_of(g));
    // Off first: its removal drops nothing more.
    set_flag(map, s, Flag::Waiting);
    crate::game::map::auras::remove_spell(map, g, SPELL_FLAG_CARRIED[s], None);
    let dropped = spawn_flag(map, s, Some(at));
    set_flag(map, s, Flag::Dropped(dropped));
    bg::announce_text_by(map, team, DROPPED[s], g);
    bg::send_world_states(map);
    if let Some(m) = map.battleground.as_mut() {
        m.schedule(
            now + FLAG_DROP,
            Due::FlagReturn {
                side: s as u8,
                object: dropped,
            },
        );
    }
}

/// The flags' timers: one left on the ground goes home; captured ones
/// stand again.
pub fn due(map: &mut Map, d: Due) {
    match d {
        Due::FlagReturn { side, object } => {
            let s = usize::from(side);
            if flag(map, s) == Some(Flag::Dropped(object)) {
                return_flag(map, s, None);
            }
        }
        Due::FlagsPlaced => {
            for s in 0..2 {
                if flag(map, s) == Some(Flag::Waiting) {
                    let stand = spawn_flag(map, s, None);
                    set_flag(map, s, Flag::AtBase(stand));
                }
            }
            bg::announce_text(map, None, PLACED);
            bg::play_sound(map, SOUND_PLACED);
            bg::send_world_states(map);
        }
        _ => {}
    }
}

/// Each update: a carrier whose flag aura is gone (cancelled, dispelled
/// by an immunity) drops it (Thistle Tea's `Flags.after_remove`).
pub fn update(map: &mut Map) {
    for (s, g) in carriers(map) {
        // Data without the aura's spell can't show it lost.
        let known = map.realm.data.spells.get(SPELL_FLAG_CARRIED[s]).is_some();
        let lost = known
            && map
                .players
                .get(&g)
                .is_some_and(|p| !p.auras.has_spell(SPELL_FLAG_CARRIED[s]));
        if lost {
            drop_flag(map, g);
        }
    }
}

fn carriers(map: &Map) -> Vec<(usize, Guid)> {
    map.battleground
        .as_ref()
        .map(|m| {
            (0..2)
                .filter_map(|s| match m.flags[s] {
                    Some(Flag::Carried(g)) => Some((s, g)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The match is over: carried flags' auras come off.
pub fn finished(map: &mut Map) {
    for (s, g) in carriers(map) {
        crate::game::map::auras::remove_spell(map, g, SPELL_FLAG_CARRIED[s], None);
    }
}
