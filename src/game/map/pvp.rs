//! PvP in a map: honorable and dishonorable kills, bonus honor, the honor
//! fields and `MSG_INSPECT_HONOR_STATS`; the PvP flag (`/pvp`, enemy
//! territory, the five minutes after); and the battleground opcodes that
//! aren't a battlemaster's: the battleground list and joining from it, the
//! spirit healer's queue, and the flag carriers' positions.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::AREA_FLAG_CAPITAL;
use crate::game::data::items::Body;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::honor::{self, Award, AwardKind, Projection};
use crate::game::map::Map;
use crate::game::map::death::Death;
use crate::game::map::player::{Player, UNIT_FLAG_PVP};
use crate::game::registry::Team;

pub const SMSG_PVP_CREDIT: u16 = 0x28C;
pub const MSG_INSPECT_HONOR_STATS: u16 = 0x2D6;
/// The PvP flag stays this long after it was last needed.
pub const PVP_LINGER: Duration = Duration::from_secs(300);
/// Damage older than this doesn't share a kill.
pub const DAMAGE_MEMORY: Duration = Duration::from_secs(60);
/// Players this near a kill share its honor.
pub const SHARE_YARDS: f32 = 74.0;

pub const PLAYER_FLAGS_PVP_DESIRED: u32 = 0x200;
/// Contested: guards of one's own side may attack.
pub const PLAYER_FLAGS_CONTESTED_PVP: u32 = 0x100;
/// How long attacking a player (or carrying silithyst) contests.
pub const CONTESTED_FOR: Duration = Duration::from_secs(30);
/// Honorless Target: one just arrived in PvP territory is worth no honor.
pub const SPELL_HONORLESS_TARGET: u32 = 2479;

/// A player's PvP flag: wanted (`/pvp`), required where they are, or
/// lingering for a while after.
#[derive(Debug, Clone, Copy, Default)]
pub struct PvpState {
    pub desired: bool,
    /// In enemy territory or a battleground.
    pub enforced: bool,
    /// The flag stays until then.
    pub until: Option<Instant>,
    /// The zone the territory was last worked out for.
    pub zone: u32,
    pub map: u32,
    /// Contested until then.
    pub contested_until: Option<Instant>,
    /// The area last worked out for them is free-for-all (an arena's).
    pub ffa_area: bool,
    /// Out of it, still free-for-all until then (`[arena]
    /// ffa_grace_secs`).
    pub ffa_until: Option<Instant>,
}

impl PvpState {
    pub fn holds(&self, now: Instant) -> bool {
        self.desired || self.enforced || self.until.is_some_and(|t| t > now)
    }
}

/// Damage a player took from other players, for sharing a kill's honor.
#[derive(Debug, Clone, Default)]
pub struct DamageHistory {
    pub by: Vec<(Guid, u32)>,
    pub last: Option<Instant>,
}

impl DamageHistory {
    pub fn current(&self, now: Instant) -> &[(Guid, u32)] {
        if self
            .last
            .is_some_and(|t| now.duration_since(t) <= DAMAGE_MEMORY)
        {
            &self.by
        } else {
            &[]
        }
    }
}

fn team_index(t: Team) -> u8 {
    match t {
        Team::Alliance => 0,
        Team::Horde => 1,
    }
}

/// The opcodes this module answers; others are passed on.
pub fn handle(map: &mut Map, g: Guid, msg: ClientOpcodeMessage) -> Option<ClientOpcodeMessage> {
    use ClientOpcodeMessage as C;
    match msg {
        C::CMSG_TOGGLE_PVP(m) => toggle(map, g, m.set.map(|s| s.enable_pvp)),
        C::MSG_INSPECT_HONOR_STATS(m) => inspect(map, g, m.guid),
        C::CMSG_BATTLEFIELD_LIST(m) => battlefield_list(map, g, Guid::zero(), m.map.as_int()),
        // Read by hand off the wire (`join_raw`); wow_messages' map alone
        // only from a test's client.
        C::CMSG_BATTLEFIELD_JOIN(m) => join_raw(map, g, &m.map.as_int().to_le_bytes()),
        C::CMSG_AREA_SPIRIT_HEALER_QUEUE(_) => {
            crate::game::map::battleground::queue_resurrection(map, g)
        }
        C::CMSG_AREA_SPIRIT_HEALER_QUERY(m) => {
            if let (Some(b), Some(p)) = (map.battleground.as_ref(), map.players.get(&g)) {
                let left = b.resurrect_at.saturating_duration_since(map.now);
                p.outbound.send(&SMSG_AREA_SPIRIT_HEALER_TIME {
                    guid: m.guid,
                    next_resurrect_time: left.as_millis() as u32,
                });
            }
        }
        C::MSG_BATTLEGROUND_PLAYER_POSITIONS => crate::game::map::battleground::positions(map, g),
        C::CMSG_BATTLEMASTER_JOIN(m) => battlemaster_join(
            map,
            g,
            m.guid,
            m.map.as_int(),
            m.join_as_group,
            m.instance_id,
        ),
        C::CMSG_BATTLEFIELD_PORT(m) => {
            crate::game::battlegrounds::port(map, g, m.action == BattlefieldPortAction::EnterBattle)
        }
        C::CMSG_LEAVE_BATTLEFIELD(_) => crate::game::battlegrounds::leave(map, g),
        C::CMSG_BATTLEFIELD_STATUS => {
            if let Some(me) = map.realm.registry.online(g) {
                crate::game::battlegrounds::send_status(&map.realm, &me);
            }
        }
        C::MSG_PVP_LOG_DATA => crate::game::map::battleground::scoreboard(map, g),
        other => return Some(other),
    }
    None
}

/// `CMSG_BATTLEMASTER_JOIN`: into the queue of the battlemaster's
/// battleground (a GM may join any from anywhere), alone or, for its
/// leader, with the whole group; into the match picked from the list, if
/// one was.
fn battlemaster_join(
    map: &mut Map,
    g: Guid,
    npc: Guid,
    map_id: u32,
    as_group: bool,
    instance: u32,
) {
    let realm = map.realm.clone();
    let gm = map.players.get(&g).is_some_and(|p| p.gm);
    let bg =
        crate::game::map::npc::npc_entry(map, g, npc, crate::game::map::npc::NPC_FLAG_BATTLEMASTER)
            .and_then(|e| realm.data.battlemasters.get(&e).copied())
            .or_else(|| {
                gm.then(|| crate::game::battlegrounds::bg_of_map(map_id))
                    .flatten()
            });
    if let Some(bg) = bg {
        join_as(map, g, bg, as_group, instance);
    }
}

pub const CMSG_BATTLEFIELD_JOIN: u16 = 0x23E;

/// `CMSG_BATTLEFIELD_JOIN`, read by hand: from a list shown with no
/// battlemaster, into the queue of the battleground on its map, alone or,
/// for its leader, with the whole group, into the match picked from the
/// list if one was. The 1.12.1 client (build 5875, `0x4AA00B`) sends the
/// map, the instance and the group's byte, as `CMSG_BATTLEMASTER_JOIN`
/// without its battlemaster (Thistle Tea's `CmsgBattlefieldJoin`, which
/// reads the map alone too as an instance of 0, alone); wow_messages wants
/// the map alone, and so read none the client sent.
pub fn join_raw(map: &mut Map, g: Guid, body: &[u8]) {
    let u32_at = |at: usize| {
        body.get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let Some(map_id) = u32_at(0) else {
        return;
    };
    let instance = u32_at(4).unwrap_or(0);
    let as_group = body.get(8).is_some_and(|b| *b != 0);
    if let Some(bg) = crate::game::battlegrounds::bg_of_map(map_id) {
        join_as(map, g, bg, as_group, instance);
    }
}

/// Into `bg`'s queue: a deserter is told so; the rest of what keeps one
/// out says nothing.
fn join_as(map: &mut Map, g: Guid, bg: u32, as_group: bool, instance: u32) {
    use crate::game::battlegrounds::{self as bgs, Refused};
    let result =
        entrants(map, g, as_group).and_then(|who| bgs::join_all(&map.realm, &who, bg, instance));
    if result == Err(Refused::Deserter)
        && let (Some(p), Some(frame)) = (map.players.get(&g), bgs::joined_frame(None))
    {
        p.outbound.send_frame(frame);
    }
}

/// Who joins: `g` alone, or (`as_group`, for its leader) their group, each
/// as they stand: on this map as they are, elsewhere as the registry and
/// their group's last stats have them.
fn entrants(
    map: &Map,
    g: Guid,
    as_group: bool,
) -> Result<Vec<crate::game::battlegrounds::Entrant>, crate::game::battlegrounds::Refused> {
    use crate::game::battlegrounds::{Entrant, Refused, bg_of_map};
    let here = |m: Guid| {
        map.players.get(&m).map(|p| Entrant {
            guid: m,
            team: p.team,
            level: p.level(),
            deserter: crate::game::battlegrounds::deserter(map, m),
            inside: map.battleground.is_some(),
        })
    };
    let me = here(g).ok_or(Refused::MemberOffline)?;
    if !as_group {
        return Ok(vec![me]);
    }
    let members: Vec<(Guid, Option<crate::game::social::groups::MemberStats>)> = {
        let groups = map.realm.social.groups.lock();
        match groups.group_of(g) {
            None => return Ok(vec![me]),
            Some(gr) if gr.leader != g => return Err(Refused::NotLeader),
            Some(gr) => gr
                .members
                .iter()
                .map(|m| (m.guid, groups.stats.get(&m.guid).copied()))
                .collect(),
        }
    };
    members
        .into_iter()
        .map(|(m, stats)| {
            if let Some(e) = here(m) {
                return Ok(e);
            }
            let o = map.realm.registry.online(m).ok_or(Refused::MemberOffline)?;
            let key = o.map().key;
            Ok(Entrant {
                guid: m,
                team: Team::of_race(o.race),
                level: o.level.load(std::sync::atomic::Ordering::Relaxed),
                deserter: stats.is_some_and(|s| {
                    s.auras
                        .contains(&crate::game::map::battleground::SPELL_DESERTER)
                }),
                inside: key.instance != 0 && bg_of_map(key.map).is_some(),
            })
        })
        .collect()
}

/// `SMSG_BATTLEFIELD_LIST`: a battleground's matches in the player's
/// bracket, from a battlemaster or the battleground window.
pub fn battlefield_list(map: &Map, g: Guid, battlemaster: Guid, map_id: u32) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let bg = crate::game::battlegrounds::bg_of_map(map_id).unwrap_or(0);
    // Alterac Valley's one bracket is the first (Thistle Tea's list).
    let number = crate::game::battlegrounds::bracket_for(bg, p.level());
    let bracket = match number {
        0 => BattlegroundBracket::Tens,
        1 => BattlegroundBracket::Twenties,
        2 => BattlegroundBracket::Thirties,
        3 => BattlegroundBracket::Fourties,
        4 => BattlegroundBracket::Fifties,
        _ => BattlegroundBracket::Sixty,
    };
    let instances = crate::game::battlegrounds::instances(&map.realm, bg, number);
    p.outbound.send(&SMSG_BATTLEFIELD_LIST {
        battlemaster,
        map: crate::game::map::movement::map_enum(map_id),
        bracket,
        battlegrounds: instances,
    });
}

/// A battleground's portal in the world: its list, for a player of its
/// side and levels; and out at its exit.
pub fn entrance(map: &mut Map, g: Guid, trigger: u32) {
    let data = map.realm.data.clone();
    let Some(e) = data.bg_entrances.get(&trigger) else {
        return;
    };
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let team_ok = match (e.team, p.team) {
        (469, Team::Alliance) | (67, Team::Horde) => true,
        (469 | 67, _) => false,
        _ => true,
    };
    let level = u32::from(p.level());
    let levels_ok = data
        .battlegrounds
        .get(&e.bg)
        .is_none_or(|t| level >= t.min_level && level <= t.max_level);
    if team_ok && levels_ok {
        battlefield_list(
            map,
            g,
            Guid::zero(),
            crate::game::battlegrounds::map_of(e.bg),
        );
    } else {
        let side = if e.team == 469 { "Alliance" } else { "Horde" };
        let min = data.battlegrounds.get(&e.bg).map_or(10, |t| t.min_level);
        map.system_message(
            g,
            &format!(
                "You must be in the {side} and at least {} level to enter.",
                ordinal(min)
            ),
        );
    }
    if e.exit_map == map.key.map {
        let (x, y, z, o) = e.exit;
        map.teleport(g, map.key, Vector3d { x, y, z }, o);
    }
}

/// A level as the portals' refusal names it: 1st, 2nd, 3rd, 11th, 51st
/// (Thistle Tea's `Entrance.ordinal`).
fn ordinal(n: u32) -> String {
    let suffix = match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// `CMSG_TOGGLE_PVP`: the player wants their flag, or not (it lingers five
/// minutes).
pub fn toggle(map: &mut Map, g: Guid, set: Option<bool>) {
    let now = map.now;
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let desired = set.unwrap_or(!p.pvp.desired);
    if p.pvp.desired && !desired {
        p.pvp.until = Some(now + PVP_LINGER);
    }
    p.pvp.desired = desired;
    p.fields
        .set_flag(PLAYER_FLAGS, PLAYER_FLAGS_PVP_DESIRED, desired);
    refresh_flag(p, now);
}

fn refresh_flag(p: &mut Player, now: Instant) {
    let holds = p.pvp.holds(now)
        || p.combat
            .last_pvp
            .is_some_and(|t| now.duration_since(t) <= PVP_LINGER);
    p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, holds);
}

/// Whether a zone obliges a player of `team` to be flagged: an enemy
/// capital, or on a PvP realm enemy and contested land; and any
/// battleground.
pub fn enforced(map: &Map, team: Team, zone: u32) -> bool {
    if crate::game::battlegrounds::bg_of_map(map.key.map).is_some() && map.key.instance != 0 {
        return true;
    }
    let Some(a) = map.realm.data.area(zone) else {
        return false;
    };
    let pvp_realm = map.cfg.pvp_realm;
    let capital = a.flags & AREA_FLAG_CAPITAL != 0;
    match (a.team, team) {
        // AreaTable's team: 2 the Alliance's, 4 the Horde's, 0 contested.
        (2, Team::Horde) | (4, Team::Alliance) => pvp_realm || capital,
        (0, _) => pvp_realm,
        _ => false,
    }
}

/// Every update: each player's flag in line with where they are, and how
/// long it lingers; and the free-for-all graces that ran out.
pub fn update(map: &mut Map) {
    let now = map.now;
    // Land flown over on a taxi isn't entered (Thistle Tea's territory):
    // where they land is worked out as they do.
    let changed: Vec<(Guid, Team, u32)> = map
        .players
        .values()
        .filter(|p| p.flight.is_none())
        .filter(|p| p.pvp.zone != p.zone || p.pvp.map != map.key.map + 1)
        .map(|p| (p.guid, p.team, p.zone))
        .collect();
    for (g, team, zone) in changed {
        let enforced = enforced(map, team, zone);
        let key = map.key.map + 1;
        if let Some(p) = map.players.get_mut(&g) {
            if p.pvp.enforced && !enforced {
                p.pvp.until = Some(now + PVP_LINGER);
            }
            p.pvp.enforced = enforced;
            p.pvp.zone = zone;
            p.pvp.map = key;
        }
    }
    for p in map.players.values_mut() {
        if p.pvp.contested_until.is_some_and(|t| t <= now) {
            p.pvp.contested_until = None;
            p.fields
                .set_flag(PLAYER_FLAGS, PLAYER_FLAGS_CONTESTED_PVP, false);
        }
        let flagged = p.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP);
        let holds = p.pvp.holds(now);
        if holds && !flagged {
            p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, true);
        } else if !holds && flagged && p.combat.last_pvp.is_none() {
            p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, false);
        }
    }
    ffa_graces(map);
}

/// Arrived somewhere (a worldport, a teleport, a flight's end): one who
/// must be flagged there is an Honorless Target a while, as Thistle Tea's
/// `Honor.Protection` has it. A battleground gives its own on arrival.
pub fn arrived(map: &mut Map, g: Guid) {
    if map.battleground.is_some() {
        return;
    }
    crate::game::map::world::update_zone(map, g);
    let Some(p) = map.players.get(&g) else {
        return;
    };
    if p.death != Death::Alive || p.flight.is_some() {
        return;
    }
    let (team, zone) = (p.team, p.zone);
    if enforced(map, team, zone) {
        crate::game::map::spells::cast_triggered(map, g, SPELL_HONORLESS_TARGET, Some(g));
    }
}

/// A player's free-for-all flag (`PLAYER_FLAGS_FFA_PVP`), which the client
/// goes by too: one may strike one of one's own side only when both carry
/// it. On in an arena's area (Gurubashi's pit), as mangos's `UpdateArea`
/// has it, and for a fighter in the pit's battle (the server knows the
/// pit's area only from the navmesh or the terrain; without them two of a
/// side could never fight). Walked out of the area, it stays `[arena]
/// ffa_grace_secs` (mangos drops it at once), so no one teases the edge;
/// back in within them, it carries on without a gap.
pub fn free_for_all(map: &mut Map, g: Guid) {
    let now = map.now;
    let grace = Duration::from_secs(map.cfg.arena.ffa_grace_secs);
    let fighting = crate::game::map::arena::fighting(map, g);
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let in_area = crate::game::map::world::area_is_ffa(&map.realm.data, p.area);
    if in_area {
        p.pvp.ffa_until = None;
    } else if p.pvp.ffa_area {
        p.pvp.ffa_until = Some(now + grace);
    }
    p.pvp.ffa_area = in_area;
    let graced = p.pvp.ffa_until.is_some_and(|t| t > now);
    if !graced {
        p.pvp.ffa_until = None;
    }
    p.fields.set_flag(
        PLAYER_FLAGS,
        crate::game::map::player::PLAYER_FLAGS_FFA_PVP,
        in_area || fighting || graced,
    );
}

/// A death ends the free-for-all grace: the flag is the area's (and the
/// battle's) again, and none starts.
pub fn died(map: &mut Map, g: Guid) {
    if let Some(p) = map.players.get_mut(&g) {
        p.pvp.ffa_until = None;
        p.pvp.ffa_area = false;
    }
    free_for_all(map, g);
}

/// A teleport away (one that ends the fight: not Blink's) ends the
/// free-for-all grace, and the flag is worked out again where they land,
/// as mangos's `UpdateArea` on arrival does; meanwhile only a fighter
/// keeps it.
pub fn teleported(map: &mut Map, g: Guid) {
    let fighting = crate::game::map::arena::fighting(map, g);
    if let Some(p) = map.players.get_mut(&g) {
        p.pvp.ffa_until = None;
        p.pvp.ffa_area = false;
        p.fields.set_flag(
            PLAYER_FLAGS,
            crate::game::map::player::PLAYER_FLAGS_FFA_PVP,
            fighting,
        );
    }
}

/// The free-for-all graces run out: where each player stands is worked
/// out again first, so one who stepped back in by less than the
/// visibility's step keeps the flag.
fn ffa_graces(map: &mut Map) {
    let now = map.now;
    let due: Vec<(Guid, bool)> = map
        .players
        .values()
        .filter(|p| p.pvp.ffa_until.is_some_and(|t| t <= now))
        .map(|p| (p.guid, p.awaiting_worldport))
        .collect();
    for (g, awaiting) in due {
        if !awaiting {
            crate::game::map::world::update_zone(map, g);
        }
        free_for_all(map, g);
    }
}

/// A player's honor known to the realm, and shown on their client.
pub fn logged_in(map: &mut Map, g: Guid) {
    register(map, g);
    refresh(map, g);
}

fn register(map: &Map, g: Guid) {
    if let Some(p) = map.players.get(&g) {
        map.realm
            .social
            .honor
            .lock()
            .register(guid::counter(g), team_index(p.team), p.level());
    }
}

/// The honor fields of `g`, from the ledger.
pub fn refresh(map: &mut Map, g: Guid) {
    let proj = map.realm.social.honor.lock().project(guid::counter(g));
    if let Some(p) = map.players.get_mut(&g) {
        apply(p, &proj);
    }
}

pub fn apply(p: &mut Player, h: &Projection) {
    let f = &mut p.fields;
    f.set_shorts(
        PLAYER_FIELD_SESSION_KILLS,
        h.today_honorable.min(0xFFFF) as u16,
        h.today_dishonorable.min(0xFFFF) as u16,
    );
    f.set_shorts(
        PLAYER_FIELD_YESTERDAY_KILLS,
        h.yesterday_kills.min(0xFFFF) as u16,
        0,
    );
    f.set_shorts(
        PLAYER_FIELD_LAST_WEEK_KILLS,
        h.last_week_kills.min(0xFFFF) as u16,
        0,
    );
    f.set_shorts(
        PLAYER_FIELD_THIS_WEEK_KILLS,
        h.this_week_kills.min(0xFFFF) as u16,
        0,
    );
    f.set_u32(
        PLAYER_FIELD_THIS_WEEK_CONTRIBUTION,
        h.this_week_contribution,
    );
    f.set_u32(PLAYER_FIELD_LIFETIME_HONORABLE_KILLS, h.lifetime_honorable);
    f.set_u32(
        PLAYER_FIELD_LIFETIME_DISHONORABLE_KILLS,
        h.lifetime_dishonorable,
    );
    f.set_u32(
        PLAYER_FIELD_YESTERDAY_CONTRIBUTION,
        h.yesterday_contribution,
    );
    f.set_u32(
        PLAYER_FIELD_LAST_WEEK_CONTRIBUTION,
        h.last_week_contribution,
    );
    f.set_u32(PLAYER_FIELD_LAST_WEEK_RANK, h.last_week_standing);
    // The rank, as a title: PLAYER_BYTES_3's last byte.
    f.set_byte(PLAYER_BYTES_3, 3, h.rank);
    f.set_byte(PLAYER_FIELD_BYTES, 3, h.highest_rank);
    f.set_byte(PLAYER_FIELD_BYTES2, 0, h.rank_bar);
}

/// How near a player must be to be inspected (mangos's
/// `INSPECT_DISTANCE`, the client's).
const INSPECT_YARDS: f32 = 28.0;

/// `MSG_INSPECT_HONOR_STATS`: a player's honor, for the inspect window: of
/// one near and not an enemy (mangos's `HandleInspectHonorStatsOpcode`,
/// Thistle Tea's `Inspection.available?`).
pub fn inspect(map: &Map, g: Guid, target: Guid) {
    let near = match (map.players.get(&g), map.players.get(&target)) {
        (Some(a), Some(b)) => {
            crate::game::map::combat::dist(a.position(), b.position()) <= INSPECT_YARDS
        }
        _ => false,
    };
    if !near || crate::game::map::combat::can_attack(map, g, target) {
        return;
    }
    let h = map.realm.social.honor.lock().project(guid::counter(target));
    let mut b = Body::default();
    b.u64(target.guid())
        .u8(h.highest_rank)
        .u32(h.today_honorable | (h.today_dishonorable << 16))
        .u16(h.yesterday_kills as u16)
        .u16(0)
        .u16(h.last_week_kills as u16)
        .u16(0)
        .u16(h.this_week_kills as u16)
        .u16(0)
        .u32(h.lifetime_honorable)
        .u32(h.lifetime_dishonorable)
        .u32(h.yesterday_contribution)
        .u32(h.last_week_contribution)
        .u32(h.this_week_contribution)
        .u32(h.last_week_standing)
        .u8(h.rank_bar);
    if let Some(p) = map.players.get(&g) {
        p.outbound.send_frame(b.frame(MSG_INSPECT_HONOR_STATS));
    }
}

/// `SMSG_PVP_CREDIT`: what a kill (or a dishonorable one) was worth.
fn credit(map: &Map, g: Guid, honor: i32, victim: Guid, rank: u32) {
    let mut b = Body::default();
    b.u32(honor as u32).u64(victim.guid()).u32(rank);
    if let Some(p) = map.players.get(&g) {
        p.outbound.send_frame(b.frame(SMSG_PVP_CREDIT));
    }
}

/// A player is contested for a while, and flagged for five minutes.
pub fn contest(map: &mut Map, g: Guid) {
    let now = map.now;
    if let Some(p) = map.players.get_mut(&g) {
        p.pvp.contested_until = Some(now + CONTESTED_FOR);
        p.pvp.until = Some(now + PVP_LINGER);
        p.fields
            .set_flag(PLAYER_FLAGS, PLAYER_FLAGS_CONTESTED_PVP, true);
        p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, true);
    }
}

/// A player was hurt: the damage counts toward sharing the kill, a
/// creature's (or the world's) under no one, so it dilutes the players'
/// shares and goes to none (Thistle Tea's `Contribution.record`); and
/// attacking a player contests the attacker.
pub fn damaged(map: &mut Map, attacker: Guid, victim: Guid, damage: u32) {
    let now = map.now;
    if attacker == victim || damage == 0 {
        return;
    }
    let attacker = if guid::is_player(attacker) {
        let duel = crate::game::map::duel::dueling(map, attacker, victim);
        let ffa = map.players.get(&attacker).is_some_and(|p| {
            p.fields
                .has_flag(PLAYER_FLAGS, crate::game::map::player::PLAYER_FLAGS_FFA_PVP)
        });
        if !duel && !ffa {
            contest(map, attacker);
        }
        attacker
    } else {
        Guid::zero()
    };
    let Some(p) = map.players.get_mut(&victim) else {
        return;
    };
    let h = &mut p.honor_damage;
    if h.last
        .is_some_and(|t| now.duration_since(t) > DAMAGE_MEMORY)
    {
        h.by.clear();
    }
    h.last = Some(now);
    match h.by.iter_mut().find(|(g, _)| *g == attacker) {
        Some((_, d)) => *d += damage,
        None => h.by.push((attacker, damage)),
    }
}

/// A player killed another: honor for those who did the damage, shared
/// by group, among those near and alive; with no damage known, the killer
/// takes it all.
pub fn player_killed(map: &mut Map, killer: Guid, victim: Guid) {
    let now = map.now;
    let Some(v) = map.players.get(&victim) else {
        return;
    };
    // Killed by an enemy; or by themselves or the world, when those who
    // hurt them lately share it (Thistle Tea's self-inflicted deaths).
    let by_enemy = map.players.get(&killer).is_some_and(|k| k.team != v.team);
    let own = killer == victim || !guid::is_player(killer);
    if (!by_enemy && !own) || v.auras.has(crate::game::map::auras::AURA_NO_PVP_CREDIT) {
        return;
    }
    let killer = if by_enemy { killer } else { Guid::zero() };
    let victim_team = v.team;
    let at = v.position();
    let history: Vec<(Guid, u32)> = v.honor_damage.current(now).to_vec();
    for g in [killer, victim] {
        if map.players.contains_key(&g) {
            register(map, g);
        }
    }
    let total: u32 = history.iter().map(|(_, d)| *d).sum();
    let mut shares: Vec<(Guid, f32)> = Vec::new();
    // The living of the other side near the victim (the killer however
    // far): the dead, and those away, earn nothing, and their share isn't
    // given to others.
    let eligible = |map: &Map, g: Guid| {
        map.players.get(&g).is_some_and(|p| {
            p.death == Death::Alive
                && p.team != victim_team
                && (g == killer || crate::game::map::combat::dist(p.position(), at) <= SHARE_YARDS)
        })
    };
    if total == 0 {
        if by_enemy {
            shares.push((killer, 1.0));
        }
    } else {
        // Damage by group (or alone), each group's share split among its
        // members near.
        let groups = map.realm.social.groups.lock();
        let mut by_group: Vec<(Vec<Guid>, u32)> = Vec::new();
        for (g, d) in &history {
            let members: Vec<Guid> = match groups.group_of(*g) {
                Some(gr) => gr.members.iter().map(|m| m.guid).collect(),
                None => vec![*g],
            };
            match by_group.iter_mut().find(|(m, _)| m.contains(g)) {
                Some((_, sum)) => *sum += d,
                None => by_group.push((members, *d)),
            }
        }
        drop(groups);
        for (members, d) in by_group {
            let rewarded: Vec<Guid> = members.into_iter().filter(|g| eligible(map, *g)).collect();
            let n = rewarded.len();
            if n == 0 {
                continue;
            }
            let each = d as f32 / total as f32 * honor::group_rate(n) / n as f32;
            for g in rewarded {
                shares.push((g, each));
            }
        }
    }
    if let Some(p) = map.players.get_mut(&victim) {
        p.honor_damage = DamageHistory::default();
    }
    let rate = map.cfg.rates.honor;
    for (g, share) in shares {
        register(map, g);
        // The server's rate scales the share, so the ledger keeps what is
        // given (mangos's `Rate.Honor`).
        let award = map
            .realm
            .social
            .honor
            .lock()
            .on(honor::today())
            .player_kill(guid::counter(g), guid::counter(victim), share * rate);
        if let Some(a) = award {
            credit(map, g, a.points as i32, victim, a.victim_rank);
            refresh(map, g);
        }
    }
}

/// A creature died at a player's (or their pet's) hand, as Thistle Tea's
/// `Honor.creature_award` has it: a civilian too low to matter (gray, or
/// worth no experience) is a dishonorable kill, an enemy leader an
/// honorable one, for the killer and their group's living members near.
/// A pet, or one Honorless, is worth nothing.
pub fn creature_killed(map: &mut Map, killer: Guid, creature: Guid) {
    let killer = crate::game::map::pets::master(map, killer);
    if !guid::is_player(killer) {
        return;
    }
    let Some(c) = map.creature(creature) else {
        return;
    };
    if c.owner.is_some() || c.auras.has(crate::game::map::auras::AURA_NO_PVP_CREDIT) {
        return;
    }
    let Some(t) = map.realm.data.creature_template(c.entry) else {
        return;
    };
    let (civilian, leader, at, entry) = (t.civilian, t.racial_leader, c.position, t.entry);
    let xp_free = t.xp_rate() == 0.0;
    let mob_level = c.fields.get_u32(UNIT_FIELD_LEVEL);
    let recipients: Vec<(Guid, u8)> = crate::game::map::loot::tap_group(map, killer, at)
        .into_iter()
        .filter_map(|g| {
            map.players
                .get(&g)
                .filter(|p| p.death == Death::Alive)
                .map(|p| (g, p.level()))
        })
        .collect();
    let rate = map.cfg.rates.honor;
    for (g, level) in recipients {
        let award = if civilian
            && (mob_level <= crate::game::map::combat::gray_level(u32::from(level)) || xp_free)
        {
            Some(Award {
                kind: AwardKind::Dishonorable,
                points: honor::dishonorable_points(level),
                victim: None,
                victim_rank: 0,
            })
        } else if leader {
            Some(Award {
                kind: AwardKind::Honorable,
                points: 488.0 * rate,
                victim: Some(u64::from(entry) << 32),
                victim_rank: 19,
            })
        } else {
            None
        };
        let Some(a) = award else {
            continue;
        };
        register(map, g);
        map.realm
            .social
            .honor
            .lock()
            .on(honor::today())
            .award(guid::counter(g), a);
        let shown = if a.kind == AwardKind::Dishonorable {
            -(a.points as i32)
        } else {
            a.points as i32
        };
        credit(map, g, shown, creature, a.victim_rank);
        refresh(map, g);
    }
}

/// Bonus honor: battleground objectives and wins, at the server's rate.
pub fn bonus_honor(map: &mut Map, g: Guid, amount: u32) {
    let amount = crate::config::scaled(amount, map.cfg.rates.honor);
    register(map, g);
    map.realm.social.honor.lock().on(honor::today()).award(
        guid::counter(g),
        Award {
            kind: AwardKind::Bonus,
            points: amount as f32,
            victim: None,
            victim_rank: 0,
        },
    );
    credit(map, g, amount as i32, Guid::zero(), 0);
    refresh(map, g);
}

/// Reputation with a battleground's faction, with its spillover.
pub fn reputation(map: &mut Map, g: Guid, faction: u32, amount: i32) {
    if amount != 0 {
        crate::game::map::reputation::modify(map, g, faction, amount, true);
    }
}

/// The highest rank a player has reached: items of a rank are worn by
/// those who ever reached it.
pub fn highest_rank(p: &Player) -> u8 {
    p.fields.get_byte(PLAYER_FIELD_BYTES, 3)
}

/// The rank a player holds now: rank rewards are bought at it.
pub fn current_rank(p: &Player) -> u8 {
    p.fields.get_byte(PLAYER_BYTES_3, 3)
}

pub fn rank_allows_use(p: &Player, t: &crate::game::data::ItemTemplate) -> bool {
    u32::from(highest_rank(p)) >= t.required_honor_rank
}

pub fn rank_allows_buy(p: &Player, t: &crate::game::data::ItemTemplate) -> bool {
    t.required_honor_rank == 0
        || (u32::from(current_rank(p)) >= t.required_honor_rank
            && u32::from(p.level()) >= t.required_level)
}

/// The realm's honor in time: the day moves on, each finished week is
/// ranked, and what changed is kept. Online players see their new fields.
pub fn sweep(realm: &crate::game::Realm) {
    let (settled, saves) = {
        let mut h = realm.social.honor.lock();
        let settled = h.advance(honor::today()) | std::mem::take(&mut h.week_turned);
        (settled, h.take_dirty())
    };
    for e in saves {
        realm.store.put_record(honor::KIND, u64::from(e.guid), &e);
    }
    if settled {
        refresh_everyone(realm);
    }
}

/// Every online player's honor fields again.
pub fn refresh_everyone(realm: &crate::game::Realm) {
    for map in realm.maps.all() {
        map.call(|m| {
            let gs: Vec<Guid> = m.players.keys().copied().collect();
            for g in gs {
                refresh(m, g);
            }
        });
    }
}

/// `.honor week`: rank the week now, as if it had ended.
pub fn settle_now(realm: &crate::game::Realm) {
    realm.social.honor.lock().settle_now();
    sweep(realm);
    refresh_everyone(realm);
}

/// `.honor`: the target's honor; `.honor points <n>` sets their rank
/// points; `.honor week` ranks the week now.
pub fn gm(map: &mut Map, g: Guid, args: &[&str]) -> String {
    let target = map
        .players
        .get(&g)
        .map(|p| p.fields.get_guid(UNIT_FIELD_TARGET))
        .filter(|t| map.players.contains_key(t))
        .unwrap_or(g);
    register(map, target);
    match args {
        ["week"] => {
            settle_now(&map.realm);
            "The week is ranked.".to_owned()
        }
        ["points", n] => match n.parse::<f32>() {
            Ok(points) => {
                map.realm
                    .social
                    .honor
                    .lock()
                    .set_rank_points(guid::counter(target), points);
                refresh(map, target);
                format!("Rank points set to {points}.")
            }
            Err(_) => "Use: .honor points <n>".to_owned(),
        },
        _ => {
            let h = map.realm.social.honor.lock();
            let p = h.project(guid::counter(target));
            let points = h
                .entries
                .get(&guid::counter(target))
                .map_or(0.0, |e| e.rank_points);
            format!(
                "Rank {} ({} points); today {} kills, this week {} kills and {} honor; last week {} kills, standing {}.",
                p.rank,
                points as u32,
                p.today_honorable,
                p.this_week_kills,
                p.this_week_contribution,
                p.last_week_kills,
                p.last_week_standing
            )
        }
    }
}
