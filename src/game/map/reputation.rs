//! Reputation: a player's standing with each faction of `Faction.dbc` that
//! has a reputation index, as Thistle Tea's `Core.Reputation` keeps it. A
//! standing is stored as the offset from the race's and class's base; kills
//! and quests change it, with spillover to related factions; the client
//! toggles at war, inactive and the watched faction. Forced reactions come
//! from auras.
//!
//! wow_messages lays the faction packets out with a 16-bit faction id where
//! 1.12 has a 32-bit reputation index, so they are encoded (and the two
//! toggles decoded) here by hand.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;

use crate::game::data::GameData;
use crate::game::data::items::Body;
use crate::game::data::misc::{self, FactionDef};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::player::Player;
use crate::game::registry::Team;

pub const SMSG_INITIALIZE_FACTIONS: u16 = 0x122;
pub const SMSG_SET_FACTION_VISIBLE: u16 = 0x123;
pub const SMSG_SET_FACTION_STANDING: u16 = 0x124;
pub const SMSG_SET_FORCED_REACTIONS: u16 = 0x2A5;
pub const CMSG_SET_FACTION_ATWAR: u16 = 0x125;
pub const CMSG_SET_FACTION_INACTIVE: u16 = 0x317;
pub const CMSG_SET_WATCHED_FACTION: u16 = 0x318;

// `FactionFlag`.
pub const FLAG_VISIBLE: u32 = 0x01;
pub const FLAG_AT_WAR: u32 = 0x02;
pub const FLAG_HIDDEN: u32 = 0x04;
pub const FLAG_INVISIBLE_FORCED: u32 = 0x08;
pub const FLAG_PEACE_FORCED: u32 = 0x10;
pub const FLAG_INACTIVE: u32 = 0x20;

/// The aura that forces a faction's reaction (`SPELL_AURA_FORCE_REACTION`).
const AURA_FORCE_REACTION: u32 = 139;
/// `SPELL_AURA_MOD_REPUTATION_GAIN`.
const AURA_MOD_REPUTATION_GAIN: u32 = 156;
/// How near a corpse a group member must be for the kill's reputation.
const GROUP_YARDS: f32 = 74.0;

/// A player's standing with one faction: the offset from their base.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Standing {
    pub faction: u32,
    pub standing: i32,
    pub flags: u32,
}

/// A player's standing with `faction` and its flags, made from the base if
/// they have none yet.
fn entry<'a>(p: &'a mut Player, def: &FactionDef) -> &'a mut Standing {
    let list = &mut p.row.state.reputation;
    match list.iter().position(|s| s.faction == def.id) {
        Some(i) => &mut list[i],
        None => {
            let (_, flags) = def.base(p.row.race, p.row.class);
            list.push(Standing {
                faction: def.id,
                standing: 0,
                flags,
            });
            list.last_mut().expect("just pushed")
        }
    }
}

/// A player's total standing with `faction`: base and offset.
pub fn standing(p: &Player, data: &GameData, faction: u32) -> i32 {
    let Some(def) = data.misc.faction(faction) else {
        return 0;
    };
    let (base, _) = def.base(p.row.race, p.row.class);
    base + p
        .row
        .state
        .reputation
        .iter()
        .find(|s| s.faction == faction)
        .map_or(0, |s| s.standing)
}

/// A player's rank with `faction`.
pub fn rank(p: &Player, data: &GameData, faction: u32) -> u32 {
    misc::rank(standing(p, data, faction))
}

/// Whether a player is at war with `faction`: by their choice, or hated or
/// hostile.
pub fn at_war(p: &Player, data: &GameData, faction: u32) -> bool {
    p.row
        .state
        .reputation
        .iter()
        .find(|s| s.faction == faction)
        .is_some_and(|s| s.flags & FLAG_AT_WAR != 0)
        || (data.misc.faction(faction).is_some() && rank(p, data, faction) <= misc::RANK_HOSTILE)
}

/// Every faction a player could have a standing with gets one, and the
/// hostile ones are at war.
pub fn init(p: &mut Player, data: &GameData) {
    let mut defs: Vec<&FactionDef> = data.misc.factions.values().collect();
    defs.sort_by_key(|d| d.index);
    for def in defs {
        let (base, _) = def.base(p.row.race, p.row.class);
        let s = entry(p, def);
        if misc::rank(base + s.standing) <= misc::RANK_HOSTILE {
            s.flags |= FLAG_AT_WAR;
        }
    }
}

/// `SMSG_INITIALIZE_FACTIONS`: the 64 slots' flags and standings.
pub fn initialize_frame(p: &Player, data: &GameData) -> Arc<[u8]> {
    let mut b = Body::default();
    b.u32(64);
    for index in 0..64u32 {
        let s = data
            .misc
            .faction_by_index
            .get(&index)
            .and_then(|f| p.row.state.reputation.iter().find(|s| s.faction == *f));
        match s {
            Some(s) => {
                b.u8(s.flags as u8).u32(s.standing as u32);
            }
            None => {
                b.u8(0).u32(0);
            }
        }
    }
    b.frame(SMSG_INITIALIZE_FACTIONS)
}

/// `SMSG_SET_FORCED_REACTIONS`: the factions auras force, and to what
/// rank.
pub fn forced_reactions_frame(p: &Player) -> Arc<[u8]> {
    let mut reactions: Vec<(u32, u32)> = p
        .auras
        .effects()
        .filter(|(_, a)| a.kind == AURA_FORCE_REACTION && a.misc > 0)
        .map(|(_, a)| (a.misc as u32, a.amount.clamp(0, 7) as u32))
        .collect();
    reactions.sort_unstable();
    reactions.dedup_by_key(|r| r.0);
    let mut b = Body::default();
    b.u32(reactions.len() as u32);
    for (faction, rank) in reactions {
        b.u32(faction).u32(rank);
    }
    b.frame(SMSG_SET_FORCED_REACTIONS)
}

/// The reputation packets a client gets at login.
pub fn send_login(p: &mut Player, data: &GameData) {
    init(p, data);
    p.outbound.send_frame(initialize_frame(p, data));
    p.outbound.send_frame(forced_reactions_frame(p));
    let watched = p.row.state.watched_faction.unwrap_or(u32::MAX);
    p.fields.set_u32(PLAYER_FIELD_WATCHED_FACTION_INDEX, watched);
}

fn standing_frame(changes: &[(u32, i32)]) -> Arc<[u8]> {
    let mut b = Body::default();
    b.u32(changes.len() as u32);
    for (index, standing) in changes {
        b.u32(*index).u32(*standing as u32);
    }
    b.frame(SMSG_SET_FACTION_STANDING)
}

/// The rate a gain is made at for the content's level: a kill of a grey
/// creature gives a fifth, a quest well below the player less.
pub fn level_rate(kill: bool, value: i32, player_level: u32, content_level: u32) -> f32 {
    if value <= 0 {
        return 1.0;
    }
    if kill {
        if content_level <= crate::game::map::combat::gray_level(player_level) {
            0.2
        } else {
            1.0
        }
    } else {
        match player_level.saturating_sub(content_level + 5) {
            0 => 1.0,
            1 => 0.8,
            2 => 0.6,
            3 => 0.4,
            _ => 0.2,
        }
    }
}

/// Change a player's standing with `faction` by `delta`, with spillover;
/// the client is told of what changed.
pub fn modify(map: &mut Map, g: Guid, faction: u32, delta: i32, spillover: bool) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let mut changes = Vec::new();
    if spillover && let Some(spills) = data.misc.spillovers.get(&faction) {
        for s in spills {
            if rank(p, &data, s.faction) <= s.max_rank {
                changes.push((s.faction, (delta as f32 * s.rate) as i32));
            }
        }
    }
    changes.push((faction, delta));
    let mut sent = Vec::new();
    for (faction, delta) in changes {
        let Some(def) = data.misc.faction(faction) else {
            continue;
        };
        let (base, _) = def.base(p.row.race, p.row.class);
        let s = entry(p, def);
        let total = (base + s.standing + delta).clamp(misc::STANDING_MIN, misc::STANDING_MAX);
        s.standing = total - base;
        if s.flags & (FLAG_HIDDEN | FLAG_INVISIBLE_FORCED) == 0 {
            s.flags |= FLAG_VISIBLE;
        }
        if misc::rank(total) <= misc::RANK_HOSTILE {
            s.flags |= FLAG_AT_WAR;
        }
        sent.push((def.index, s.standing));
    }
    if !sent.is_empty() {
        p.outbound.send_frame(standing_frame(&sent));
    }
}

/// A gain's size after its rate, the level rate and auras' bonuses,
/// dithered to a whole point.
fn gain(p: &Player, value: i32, level_rate: f32, _faction: u32) -> i32 {
    let bonus = if value > 0 {
        p.auras.sum(AURA_MOD_REPUTATION_GAIN)
    } else {
        0
    };
    if 100 + bonus <= 0 {
        return 0;
    }
    let v = value as f32 * level_rate * (100 + bonus) as f32 / 100.0;
    let dither: f32 = rand::random();
    if v < 0.0 {
        -((v.abs() + dither).floor() as i32)
    } else {
        (v + dither).floor() as i32
    }
}

/// A creature killed by `tapper`: its reputation for them, and for their
/// group near the corpse.
pub fn creature_killed(map: &mut Map, tapper: Guid, creature: Guid) {
    let data = map.realm.data.clone();
    let Some(c) = map.creature(creature) else {
        return;
    };
    let Some(rewards) = data.misc.kill_reputation.get(&c.entry) else {
        return;
    };
    let (at, level) = (c.position, c.fields.get_u32(UNIT_FIELD_LEVEL));
    let members: Vec<Guid> = {
        let groups = map.realm.social.groups.lock();
        match groups.group_of(tapper) {
            Some(g) => g.members.iter().map(|m| m.guid).collect(),
            None => vec![tapper],
        }
    };
    for m in members {
        let Some(p) = map.players.get(&m) else {
            continue;
        };
        if m != tapper && crate::game::map::combat::dist(p.position(), at) > GROUP_YARDS {
            continue;
        }
        let team = match p.team {
            Team::Alliance => 1,
            Team::Horde => 2,
        };
        let plevel = u32::from(p.level());
        let mut grants = Vec::new();
        for r in rewards.iter().filter(|r| r.team == 0 || r.team == team) {
            let delta = gain(p, r.value, level_rate(true, r.value, plevel, level), r.faction);
            if rank(p, &data, r.faction) <= r.max_rank {
                grants.push((r.faction, delta, true));
            }
            if r.team_award
                && let Some(parent) = data.misc.faction(r.faction).map(|d| d.parent).filter(|x| *x != 0)
            {
                grants.push((parent, delta / 2, false));
            }
        }
        for (faction, delta, spill) in grants {
            modify(map, m, faction, delta, spill);
        }
    }
}

/// A quest rewarded: its reputation.
pub fn quest_rewarded(map: &mut Map, g: Guid, q: &crate::game::data::npc::Quest) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let level = q.level_for(p.level());
    let plevel = u32::from(p.level());
    let grants: Vec<(u32, i32)> = q
        .reputation_rewards
        .iter()
        .filter(|(f, v)| *f != 0 && *v != 0)
        .map(|(f, v)| (*f, gain(p, *v, level_rate(false, *v, plevel, level), *f)))
        .collect();
    for (faction, delta) in grants {
        modify(map, g, faction, delta, true);
    }
}

/// The faction a unit's faction template belongs to.
pub fn faction_of(map: &Map, unit: Guid) -> Option<u32> {
    let template = map.unit_fields(unit)?.get_u32(UNIT_FIELD_FACTIONTEMPLATE);
    map.realm
        .data
        .faction_templates
        .get(&template)
        .map(|t| t.faction)
        .filter(|f| *f != 0)
}

/// A vendor's price for a player: a tenth off from honored up.
pub fn price(map: &Map, player: Guid, vendor: Guid, amount: u32) -> u32 {
    let (Some(p), Some(f)) = (map.players.get(&player), faction_of(map, vendor)) else {
        return amount;
    };
    let data = &map.realm.data;
    if data.misc.faction(f).is_some() && rank(p, data, f) >= misc::RANK_HONORED {
        (amount * 9 + 5) / 10
    } else {
        amount
    }
}

/// Whether a player's standing meets an item's reputation requirement.
pub fn meets(p: &Player, data: &GameData, faction: u32, required_rank: u32) -> bool {
    faction == 0 || required_rank == 0 || rank(p, data, faction) >= required_rank
}

/// The player's choice of a faction to be at war with (`CMSG_SET_FACTION_ATWAR`).
pub fn set_at_war(map: &mut Map, g: Guid, index: u32, war: bool) {
    let data = map.realm.data.clone();
    let Some(def) = data.misc.faction_by_index.get(&index).and_then(|f| data.misc.faction(*f)) else {
        return;
    };
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let hated = rank(p, &data, def.id) == misc::RANK_HATED;
    let s = entry(p, def);
    if s.flags & (FLAG_HIDDEN | FLAG_INVISIBLE_FORCED) != 0
        || (war && s.flags & FLAG_PEACE_FORCED != 0 && !hated)
    {
        return;
    }
    if war {
        s.flags |= FLAG_AT_WAR;
    } else {
        s.flags &= !FLAG_AT_WAR;
    }
}

/// `CMSG_SET_FACTION_INACTIVE`.
pub fn set_inactive(map: &mut Map, g: Guid, index: u32, inactive: bool) {
    let data = map.realm.data.clone();
    let Some(def) = data.misc.faction_by_index.get(&index).and_then(|f| data.misc.faction(*f)) else {
        return;
    };
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let s = entry(p, def);
    if inactive
        && (s.flags & (FLAG_HIDDEN | FLAG_INVISIBLE_FORCED) != 0 || s.flags & FLAG_VISIBLE == 0)
    {
        return;
    }
    if inactive {
        s.flags |= FLAG_INACTIVE;
    } else {
        s.flags &= !FLAG_INACTIVE;
    }
}

/// A faction the player meets is shown in their reputation pane.
pub fn set_visible(map: &mut Map, g: Guid, faction: u32) {
    let data = map.realm.data.clone();
    let Some(def) = data.misc.faction(faction) else {
        return;
    };
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let s = entry(p, def);
    if s.flags & (FLAG_HIDDEN | FLAG_INVISIBLE_FORCED | FLAG_VISIBLE) != 0 {
        return;
    }
    s.flags |= FLAG_VISIBLE;
    let mut b = Body::default();
    b.u32(def.index);
    p.outbound.send_frame(b.frame(SMSG_SET_FACTION_VISIBLE));
}

/// The reputation opcodes wow_messages reads.
pub fn handle(_map: &mut Map, _g: Guid, msg: ClientOpcodeMessage) -> Option<ClientOpcodeMessage> {
    match msg {
        // Laid out wrong by wow_messages; these come raw.
        ClientOpcodeMessage::CMSG_SET_FACTION_ATWAR(_)
        | ClientOpcodeMessage::CMSG_SET_FACTION_INACTIVE(_)
        | ClientOpcodeMessage::CMSG_SET_WATCHED_FACTION(_) => {}
        other => return Some(other),
    }
    None
}

/// The raw reputation opcodes: (index u32, flag u8), or the watched index
/// (-1 for none).
pub fn handle_raw(map: &mut Map, g: Guid, opcode: u16, body: &[u8]) -> bool {
    if opcode == CMSG_SET_WATCHED_FACTION {
        if body.len() >= 4
            && let Some(p) = map.players.get_mut(&g)
        {
            let id = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
            let index = (id < 64).then_some(id);
            p.row.state.watched_faction = index;
            p.fields
                .set_u32(PLAYER_FIELD_WATCHED_FACTION_INDEX, index.unwrap_or(u32::MAX));
        }
        return true;
    }
    if !matches!(opcode, CMSG_SET_FACTION_ATWAR | CMSG_SET_FACTION_INACTIVE) {
        return false;
    }
    if body.len() < 5 {
        return true;
    }
    let index = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    let flag = body[4];
    if opcode == CMSG_SET_FACTION_ATWAR {
        set_at_war(map, g, index, flag != 0);
    } else {
        set_inactive(map, g, index, flag != 0);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gains_shrink_for_grey_kills_and_old_quests() {
        assert_eq!(level_rate(true, 25, 30, 30), 1.0);
        assert_eq!(level_rate(true, 25, 30, 10), 0.2);
        assert_eq!(level_rate(true, -25, 30, 10), 1.0);
        assert_eq!(level_rate(false, 250, 20, 15), 1.0);
        assert_eq!(level_rate(false, 250, 22, 15), 0.6);
        assert_eq!(level_rate(false, 250, 60, 15), 0.2);
    }
}
