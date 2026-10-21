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
/// `SPELL_AURA_MOD_FACTION_REPUTATION_GAIN`: a kill's gain with one
/// faction (its misc value).
const AURA_MOD_FACTION_REPUTATION_GAIN: u32 = 190;

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

/// A forced reaction came or went: the player is told them all again
/// (mangos's `HandleForceReaction`, Thistle Tea's
/// `forced_reaction_events`).
pub fn send_forced_reactions(map: &Map, g: Guid) {
    if let Some(p) = map.players.get(&g) {
        p.outbound.send_frame(forced_reactions_frame(p));
    }
}

/// A forced reaction to `faction` at `forced` came (`applied`) or went:
/// the player is told them all again, and friendly now, forced so or by
/// their own standing once the force is gone, they and their pet stop
/// fighting the faction's units, and its units stop fighting them
/// (mangos's `HandleForceReaction` and `StopAttackFaction`, Thistle Tea's
/// `stop_attack_factions`).
pub fn forced_reaction_changed(map: &mut Map, g: Guid, faction: u32, forced: i32, applied: bool) {
    send_forced_reactions(map, g);
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let friendly = if applied {
        forced >= misc::RANK_FRIENDLY as i32
    } else {
        rank(p, &data, faction) >= misc::RANK_FRIENDLY
    };
    if friendly && faction != 0 {
        stop_attacking_faction(map, g, faction);
    }
}

fn stop_attacking_faction(map: &mut Map, g: Guid, faction: u32) {
    let data = map.realm.data.clone();
    let of_faction = |map: &Map, u: Guid| {
        map.unit_fields(u)
            .and_then(|f| {
                data.faction_templates
                    .get(&f.get_u32(UNIT_FIELD_FACTIONTEMPLATE))
            })
            .is_some_and(|t| t.faction == faction)
    };
    let pet = crate::game::map::pets::pet_of(map, g);
    for unit in std::iter::once(g).chain(pet) {
        let victim = map.combat(unit).filter(|c| c.attacking).map(|c| c.victim);
        if let Some(v) = victim
            && of_faction(map, v)
        {
            crate::game::map::combat::stop_attack(map, unit);
            crate::game::map::spells::interrupt(map, unit);
        }
        let fighting: Vec<Guid> = map
            .creatures
            .iter()
            .filter(|(_, c)| c.threat.iter().any(|(t, _)| *t == unit))
            .map(|(_, c)| c.guid)
            .filter(|c| of_faction(map, *c))
            .collect();
        crate::game::map::ai::forget_by(map, unit, fighting);
    }
}

/// The reputation packets a client gets at login.
pub fn send_login(p: &mut Player, data: &GameData) {
    init(p, data);
    p.outbound.send_frame(initialize_frame(p, data));
    p.outbound.send_frame(forced_reactions_frame(p));
    let watched = p.row.state.watched_faction.unwrap_or(u32::MAX);
    p.fields
        .set_u32(PLAYER_FIELD_WATCHED_FACTION_INDEX, watched);
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
    level_rate_with(kill, value, player_level, content_level, (0.2, 0.2))
}

/// [`level_rate`], a grey kill at `least.0` and a quest no lower than
/// `least.1` (`[rates] reputation_low_level_kill` and `_quest`, mangos's
/// `Rate.Reputation.LowLevel.*`).
pub fn level_rate_with(
    kill: bool,
    value: i32,
    player_level: u32,
    content_level: u32,
    least: (f32, f32),
) -> f32 {
    if value <= 0 {
        return 1.0;
    }
    if kill {
        if content_level <= crate::game::map::combat::gray_level(player_level) {
            least.0
        } else {
            1.0
        }
    } else {
        let rate: f32 = match player_level.saturating_sub(content_level + 5) {
            0 => 1.0,
            1 => 0.8,
            2 => 0.6,
            3 => 0.4,
            4 => 0.2,
            _ => 0.0,
        };
        rate.max(least.1)
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
    let mut shown = Vec::new();
    for (faction, delta) in changes {
        let Some(def) = data.misc.faction(faction) else {
            continue;
        };
        let (base, _) = def.base(p.row.race, p.row.class);
        let s = entry(p, def);
        let was = misc::rank(base + s.standing);
        let total = (base + s.standing + delta).clamp(misc::STANDING_MIN, misc::STANDING_MAX);
        s.standing = total - base;
        if s.flags & (FLAG_HIDDEN | FLAG_INVISIBLE_FORCED | FLAG_VISIBLE) == 0 {
            s.flags |= FLAG_VISIBLE;
            shown.push(def.index);
        }
        let now = misc::rank(total);
        if now <= misc::RANK_HOSTILE {
            s.flags |= FLAG_AT_WAR;
        } else if now >= misc::RANK_NEUTRAL
            && was < misc::RANK_NEUTRAL
            && s.flags & (FLAG_HIDDEN | FLAG_INVISIBLE_FORCED) == 0
        {
            // Up to Neutral from below, the war is over: else a player
            // risen from Hated with the Brood of Nozdormu would stay at war
            // with it, and its Anachronos would not speak to them
            // (mangos's `SetOneFactionReputation` unticks it; a hidden
            // faction's war can't change).
            s.flags &= !FLAG_AT_WAR;
        }
        sent.push((def.index, s.standing));
    }
    // A faction newly shown is told before its standing (mangos's
    // `SetOneFactionReputation` makes it visible first).
    for index in shown {
        let mut b = Body::default();
        b.u32(index);
        p.outbound.send_frame(b.frame(SMSG_SET_FACTION_VISIBLE));
    }
    if !sent.is_empty() {
        p.outbound.send_frame(standing_frame(&sent));
    }
    // Quests whose objective is a standing.
    let waiting: Vec<u32> = p
        .journal
        .log
        .iter()
        .map(|l| l.quest)
        .filter(|q| {
            data.npc
                .quests
                .get(q)
                .is_some_and(|q| q.rep_objective.0 != 0)
        })
        .collect();
    for q in waiting {
        crate::game::map::quests::check(map, g, q);
    }
}

/// What a gain is for: its `reputation_reward_rate` column.
#[derive(Debug, Clone, Copy)]
pub enum Source {
    Quest,
    Kill,
    Spell,
}

/// A faction's rate for gains of `source` (`reputation_reward_rate`; 1 if
/// none).
pub fn reward_rate(data: &GameData, faction: u32, source: Source) -> f32 {
    data.misc
        .reward_rates
        .get(&faction)
        .map_or(1.0, |r| match source {
            Source::Quest => r.0,
            Source::Kill => r.1,
            Source::Spell => r.2,
        })
}

/// A gain's size after its rate, the level rate, auras' bonuses and the
/// server's rate (`server_rate`, for gains only), dithered to a whole
/// point (Thistle Tea's `calculate_gain`).
fn gain(
    p: &Player,
    data: &GameData,
    value: i32,
    level_rate: f32,
    faction: u32,
    source: Source,
    server_rate: f32,
) -> i32 {
    let rate = reward_rate(data, faction, source);
    // A gain's auras, and a kill's for its faction alone (mangos's
    // `CalculateReputationGain`); a loss takes none (Thistle Tea's
    // `calculate_gain`).
    let bonus = if value > 0 {
        let kill = if matches!(source, Source::Kill) {
            p.auras
                .sum_misc(AURA_MOD_FACTION_REPUTATION_GAIN, faction as i32)
        } else {
            0
        };
        p.auras.sum(AURA_MOD_REPUTATION_GAIN) + kill
    } else {
        0
    };
    if rate <= 0.0 || 100 + bonus <= 0 {
        return 0;
    }
    let server_rate = if value > 0 { server_rate } else { 1.0 };
    let v = value as f32 * rate * level_rate * (100 + bonus) as f32 / 100.0 * server_rate;
    dither(v, rand::random())
}

/// `v` to a whole number by `roll` (in 0..1): a whole `v` as it is, a
/// fraction up as often as it is of a whole, so that on average it is
/// what it was (Thistle Tea's `Math.dither/2`); a loss as a gain is.
pub fn dither(v: f32, roll: f32) -> i32 {
    if v < 0.0 {
        -((v.abs() + roll).floor() as i32)
    } else {
        (v + roll).floor() as i32
    }
}

/// A creature killed by `tapper`: its reputation for them, and for their
/// group near the corpse.
pub fn creature_killed(map: &mut Map, tapper: Guid, creature: Guid) {
    let data = map.realm.data.clone();
    let server_rate = map.cfg.rates.reputation_gain;
    let least = (
        map.cfg.rates.reputation_low_level_kill,
        map.cfg.rates.reputation_low_level_quest,
    );
    let Some(c) = map.creature(creature) else {
        return;
    };
    let Some(rewards) = data.misc.kill_reputation.get(&c.entry) else {
        return;
    };
    let (at, level) = (c.position, c.fields.get_u32(UNIT_FIELD_LEVEL));
    let group = crate::game::map::tap::group_of(c, tapper);
    let members = crate::game::map::tap::recipients(&map.realm, tapper, group);
    for m in members {
        let Some(p) = map.players.get(&m) else {
            continue;
        };
        if m != tapper
            && crate::game::map::combat::dist(p.position(), at) > map.cfg.players.group_reward_yards
        {
            continue;
        }
        let team = match p.team {
            Team::Alliance => 1,
            Team::Horde => 2,
        };
        let plevel = u32::from(p.level());
        let mut grants = Vec::new();
        for r in rewards.iter().filter(|r| r.team == 0 || r.team == team) {
            let delta = gain(
                p,
                &data,
                r.value,
                level_rate_with(true, r.value, plevel, level, least),
                r.faction,
                Source::Kill,
                server_rate,
            );
            if rank(p, &data, r.faction) <= r.max_rank {
                grants.push((r.faction, delta, true));
            }
            if r.team_award
                && let Some(parent) = data
                    .misc
                    .faction(r.faction)
                    .map(|d| d.parent)
                    .filter(|x| *x != 0)
            {
                grants.push((parent, delta / 2, false));
            }
        }
        for (faction, delta, spill) in grants {
            modify(map, m, faction, delta, spill);
        }
    }
}

/// A spell's reputation (effect 103), at its faction's spell rate.
pub fn spell_reward(map: &mut Map, g: Guid, faction: u32, value: i32) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let delta = gain(
        p,
        &data,
        value,
        1.0,
        faction,
        Source::Spell,
        map.cfg.rates.reputation_gain,
    );
    modify(map, g, faction, delta, true);
}

/// A quest rewarded: its reputation.
pub fn quest_rewarded(map: &mut Map, g: Guid, q: &crate::game::data::npc::Quest) {
    let data = map.realm.data.clone();
    let server_rate = map.cfg.rates.reputation_gain;
    let least = (
        map.cfg.rates.reputation_low_level_kill,
        map.cfg.rates.reputation_low_level_quest,
    );
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let level = q.level_for(p.level());
    let plevel = u32::from(p.level());
    // A reward its mask marks spills nowhere.
    let grants: Vec<(u32, i32, bool)> = q
        .reputation_rewards
        .iter()
        .enumerate()
        .filter(|(_, (f, v))| *f != 0 && *v != 0)
        .map(|(i, (f, v))| {
            let rate = level_rate_with(false, *v, plevel, level, least);
            (
                *f,
                gain(p, &data, *v, rate, *f, Source::Quest, server_rate),
                q.reputation_no_spillover & (1 << i) == 0,
            )
        })
        .collect();
    for (faction, delta, spillover) in grants {
        modify(map, g, faction, delta, spillover);
    }
}

/// `SMSG_SET_FACTION_ATWAR`: a faction's at-war flag, by its list index.
const SMSG_SET_FACTION_ATWAR: u16 = 0x313;

fn at_war_frame(index: u32, war: bool) -> Arc<[u8]> {
    let mut b = Body::default();
    b.u32(index).u8(if war { FLAG_AT_WAR as u8 } else { 0 });
    b.frame(SMSG_SET_FACTION_ATWAR)
}

/// A creature took up a fight with a player: its faction, unless hidden,
/// forced to peace (and not hated) or at war already, is at war with them
/// for the fight (Thistle Tea's `set_temporary_at_war`).
pub fn temporary_war(map: &mut Map, player: Guid, creature: Guid) {
    if !crate::game::guid::is_unit(creature) {
        return;
    }
    let Some(faction) = faction_of(map, creature) else {
        return;
    };
    temporary_war_with(map, player, faction);
}

/// [`temporary_war`], with the creature's faction.
pub fn temporary_war_with(map: &mut Map, player: Guid, faction: u32) {
    let data = map.realm.data.clone();
    let now = map.now;
    let Some(def) = data.misc.faction(faction) else {
        return;
    };
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let hated = rank(p, &data, faction) == misc::RANK_HATED;
    let s = entry(p, def);
    if s.flags & (FLAG_AT_WAR | FLAG_HIDDEN | FLAG_INVISIBLE_FORCED) != 0
        || (s.flags & FLAG_PEACE_FORCED != 0 && !hated)
    {
        return;
    }
    s.flags |= FLAG_AT_WAR;
    p.temporary_war.push(faction);
    p.outbound.send_frame(at_war_frame(def.index, true));
    // Looked at with the player's fight: one its creature never fought
    // (a blow that put them on its list and killed it) ends too, once no
    // creature holds them.
    p.combat.next_check.get_or_insert(now);
}

/// A player's standings as they are kept: a fight's war is the fight's,
/// and not saved with them (else a save mid-fight would leave them at war
/// for good, the next login knowing nothing of the fight).
pub fn kept(p: &Player) -> Vec<Standing> {
    let mut out = p.row.state.reputation.clone();
    for s in out
        .iter_mut()
        .filter(|s| p.temporary_war.contains(&s.faction))
    {
        s.flags &= !FLAG_AT_WAR;
    }
    out
}

/// Out of the fight: the factions at war only for it are at peace again,
/// but for those now hated or hostile.
pub fn end_temporary_war(p: &mut Player, data: &GameData) {
    for faction in std::mem::take(&mut p.temporary_war) {
        let Some(def) = data.misc.faction(faction) else {
            continue;
        };
        let hostile = rank(p, data, faction) <= misc::RANK_HOSTILE;
        let s = entry(p, def);
        if s.flags & FLAG_AT_WAR != 0 && !hostile {
            s.flags &= !FLAG_AT_WAR;
            p.outbound.send_frame(at_war_frame(def.index, false));
        }
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

/// How `a` regards `b`, as a reputation rank (hated 0 to exalted 7), as
/// Thistle Tea's `reaction_rank` has it: friendly to its own side and its
/// group, hostile to a duel's opponent; a player as their standing with a
/// creature's faction says, and a creature as its faction's standing with
/// a player says; else as their faction templates have it. `None` when
/// either has no faction template.
pub fn reaction_rank(map: &Map, a: Guid, b: Guid) -> Option<u32> {
    let data = &map.realm.data;
    let template = |g: Guid| {
        let id = match map.unit_fields(g) {
            Some(f) => f.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
            None => map.gameobject(g)?.fields.get_u32(GAMEOBJECT_FACTION),
        };
        data.faction_templates.get(&id)
    };
    let (ta, tb) = (template(a)?, template(b)?);
    let (ma, mb) = (
        crate::game::map::pets::master(map, a),
        crate::game::map::pets::master(map, b),
    );
    if ma == mb {
        return Some(misc::RANK_FRIENDLY);
    }
    let (pa, pb) = (map.players.get(&ma), map.players.get(&mb));
    if pa.is_some() && pb.is_some() {
        if crate::game::map::duel::dueling(map, ma, mb) {
            return Some(misc::RANK_HOSTILE);
        }
        let groups = map.realm.social.groups.lock();
        if let (Some(x), Some(y)) = (groups.group_of(ma), groups.group_of(mb))
            && x.id == y.id
        {
            return Some(misc::RANK_FRIENDLY);
        }
    }
    let by_standing = match (pa, pb) {
        (Some(p), None) => standing_reaction(p, data, tb.faction, false),
        (None, Some(p)) => standing_reaction(p, data, ta.faction, true),
        _ => None,
    };
    Some(by_standing.unwrap_or(if ta.hostile_to(tb) {
        misc::RANK_HOSTILE
    } else if ta.friendly_to(tb) || tb.friendly_to(ta) {
        misc::RANK_FRIENDLY
    } else {
        misc::RANK_NEUTRAL
    }))
}

/// A player's standing with `faction` as a reaction: as an aura forces it;
/// else hostile at war and friendly not, or, for the faction's view of the
/// player, their rank, no better than neutral at war. `None` for a faction
/// with no standing.
fn standing_reaction(p: &Player, data: &GameData, faction: u32, of_faction: bool) -> Option<u32> {
    let forced = p
        .auras
        .effects()
        .find(|(_, a)| a.kind == AURA_FORCE_REACTION && a.misc > 0 && a.misc as u32 == faction)
        .map(|(_, a)| a.amount.clamp(0, 7) as u32);
    if forced.is_some() {
        return forced;
    }
    if faction == 0 || data.misc.faction(faction).is_none() {
        return None;
    }
    let war = at_war(p, data, faction);
    Some(match (of_faction, war) {
        (false, true) => misc::RANK_HOSTILE,
        (false, false) => misc::RANK_FRIENDLY,
        (true, true) => rank(p, data, faction).min(misc::RANK_NEUTRAL),
        (true, false) => rank(p, data, faction),
    })
}

/// A vendor's price for a player: a tenth off from honored up.
pub fn price(map: &Map, player: Guid, vendor: Guid, amount: u32) -> u32 {
    match percent(map, player, vendor) {
        100 => amount,
        pct => ((u64::from(amount) * u64::from(pct) + 50) / 100) as u32,
    }
}

/// The discount `vendor` gives `player` as a factor: a tenth off from
/// Honored, and another from the PvP rank of Sergeant at a friendly NPC.
pub fn discount(map: &Map, player: Guid, vendor: Guid) -> f32 {
    percent(map, player, vendor) as f32 / 100.0
}

/// What of a price `vendor` asks of `player`, in percent, as mangos's
/// `GetReputationPriceDiscount`: 10 off from Honored with its faction, and
/// 10 more from a PvP rank of Sergeant (the client's rank 3) at one whose
/// faction is friendly to the player's.
fn percent(map: &Map, player: Guid, vendor: Guid) -> u32 {
    let (Some(p), Some(f)) = (map.players.get(&player), faction_of(map, vendor)) else {
        return 100;
    };
    let data = &map.realm.data;
    let mut pct = 100;
    if data.misc.faction(f).is_some() && rank(p, data, f) >= misc::RANK_HONORED {
        pct -= 10;
    }
    let template = |g: Guid| {
        map.unit_fields(g).and_then(|x| {
            data.faction_templates
                .get(&x.get_u32(UNIT_FIELD_FACTIONTEMPLATE))
        })
    };
    const SERGEANT: u8 = 3;
    let visual = crate::game::map::pvp::current_rank(p).saturating_sub(4);
    if visual >= SERGEANT
        && let (Some(ours), Some(theirs)) = (template(player), template(vendor))
        && ours.friendly_to(theirs)
    {
        pct -= 10;
    }
    pct
}

/// Whether a player's standing meets an item's reputation requirement.
pub fn meets(p: &Player, data: &GameData, faction: u32, required_rank: u32) -> bool {
    faction == 0 || required_rank == 0 || rank(p, data, faction) >= required_rank
}

/// The player's choice of a faction to be at war with (`CMSG_SET_FACTION_ATWAR`).
pub fn set_at_war(map: &mut Map, g: Guid, index: u32, war: bool) {
    let data = map.realm.data.clone();
    let Some(def) = data
        .misc
        .faction_by_index
        .get(&index)
        .and_then(|f| data.misc.faction(*f))
    else {
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
    let Some(def) = data
        .misc
        .faction_by_index
        .get(&index)
        .and_then(|f| data.misc.faction(*f))
    else {
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

/// A player targets a unit: a friendly faction they have a standing with
/// shows in their pane.
pub fn reveal_target(map: &mut Map, g: Guid, target: Guid) {
    if target.is_zero() || target == g {
        return;
    }
    let Some(f) = faction_of(map, target) else {
        return;
    };
    if crate::game::map::combat::hostile(map, target, g) {
        return;
    }
    set_visible(map, g, f);
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
            p.fields.set_u32(
                PLAYER_FIELD_WATCHED_FACTION_INDEX,
                index.unwrap_or(u32::MAX),
            );
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
        // Other floors: a grey kill at its own, a quest's steps down to
        // its.
        assert_eq!(level_rate_with(true, 25, 30, 10, (0.5, 0.2)), 0.5);
        assert_eq!(level_rate_with(true, 25, 30, 30, (0.0, 0.2)), 1.0);
        assert_eq!(level_rate_with(false, 250, 23, 15, (0.2, 0.5)), 0.5);
        assert_eq!(level_rate_with(false, 250, 22, 15, (0.2, 0.5)), 0.6);
        assert_eq!(level_rate_with(false, 250, 60, 15, (0.2, 0.0)), 0.0);
        assert_eq!(level_rate_with(false, 250, 24, 15, (0.2, 0.0)), 0.2);
    }
}
