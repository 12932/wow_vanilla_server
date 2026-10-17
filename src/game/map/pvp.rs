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
        if self.last.is_some_and(|t| now.duration_since(t) <= DAMAGE_MEMORY) {
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
        C::CMSG_BATTLEFIELD_JOIN(m) => join(map, g, m.map.as_int()),
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
        C::MSG_BATTLEGROUND_PLAYER_POSITIONS => {
            crate::game::map::battleground::positions(map, g)
        }
        C::CMSG_BATTLEMASTER_JOIN(m) => battlemaster_join(map, g, m.guid, m.map.as_int()),
        C::CMSG_BATTLEFIELD_PORT(m) => crate::game::battlegrounds::port(
            map,
            g,
            m.action == BattlefieldPortAction::EnterBattle,
        ),
        C::CMSG_LEAVE_BATTLEFIELD(_) => crate::game::battlegrounds::leave(map, g),
        C::MSG_PVP_LOG_DATA => crate::game::map::battleground::scoreboard(map, g),
        other => return Some(other),
    }
    None
}

/// `CMSG_BATTLEMASTER_JOIN`: into the queue of the battlemaster's
/// battleground (a GM may join any from anywhere).
fn battlemaster_join(map: &mut Map, g: Guid, npc: Guid, map_id: u32) {
    let realm = map.realm.clone();
    let gm = map.players.get(&g).is_some_and(|p| p.gm);
    let bg = crate::game::map::npc::npc_entry(map, g, npc, crate::game::map::npc::NPC_FLAG_BATTLEMASTER)
        .and_then(|e| realm.data.battlemasters.get(&e).copied())
        .or_else(|| {
            gm.then(|| crate::game::battlegrounds::bg_of_map(map_id))
                .flatten()
        });
    let (Some(bg), Some(me)) = (bg, realm.registry.online(g)) else {
        return;
    };
    if let Some(p) = map.players.get(&g) {
        let (team, level) = (p.team, p.level());
        crate::game::battlegrounds::join(&realm, &me, team, level, bg);
    }
}

/// `CMSG_BATTLEFIELD_JOIN`: into the queue of the battleground on
/// `map_id`, from its list.
fn join(map: &mut Map, g: Guid, map_id: u32) {
    let realm = map.realm.clone();
    let (Some(bg), Some(me)) = (
        crate::game::battlegrounds::bg_of_map(map_id),
        realm.registry.online(g),
    ) else {
        return;
    };
    if let Some(p) = map.players.get(&g) {
        let (team, level) = (p.team, p.level());
        crate::game::battlegrounds::join(&realm, &me, team, level, bg);
    }
}

/// `SMSG_BATTLEFIELD_LIST`: a battleground's matches in the player's
/// bracket, from a battlemaster or the battleground window.
pub fn battlefield_list(map: &Map, g: Guid, battlemaster: Guid, map_id: u32) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let level = p.level();
    let bracket = match level {
        0..=19 => BattlegroundBracket::Tens,
        20..=29 => BattlegroundBracket::Twenties,
        30..=39 => BattlegroundBracket::Thirties,
        40..=49 => BattlegroundBracket::Fourties,
        50..=59 => BattlegroundBracket::Fifties,
        _ => BattlegroundBracket::Sixty,
    };
    let instances = crate::game::battlegrounds::instances(
        &map.realm,
        crate::game::battlegrounds::bg_of_map(map_id).unwrap_or(0),
        crate::game::battlegrounds::bracket_of(level),
    );
    p.outbound.send(&SMSG_BATTLEFIELD_LIST {
        battlemaster,
        map: crate::game::map::movement::map_enum(map_id),
        bracket,
        battlegrounds: instances,
    });
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
    let holds = p.pvp.holds(now) || p.combat.last_pvp.is_some_and(|t| now.duration_since(t) <= PVP_LINGER);
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
    let pvp_realm = map.realm.config.pvp_realm;
    let capital = a.flags & AREA_FLAG_CAPITAL != 0;
    match (a.team, team) {
        // AreaTable's team: 2 the Alliance's, 4 the Horde's, 0 contested.
        (2, Team::Horde) | (4, Team::Alliance) => pvp_realm || capital,
        (0, _) => pvp_realm,
        _ => false,
    }
}

/// Every update: each player's flag in line with where they are, and how
/// long it lingers.
pub fn update(map: &mut Map) {
    let now = map.now;
    let changed: Vec<(Guid, Team, u32)> = map
        .players
        .values()
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
        let flagged = p.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP);
        let holds = p.pvp.holds(now);
        if holds && !flagged {
            p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, true);
        } else if !holds && flagged && p.combat.last_pvp.is_none() {
            p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, false);
        }
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
    f.set_shorts(PLAYER_FIELD_YESTERDAY_KILLS, h.yesterday_kills.min(0xFFFF) as u16, 0);
    f.set_shorts(PLAYER_FIELD_LAST_WEEK_KILLS, h.last_week_kills.min(0xFFFF) as u16, 0);
    f.set_shorts(PLAYER_FIELD_THIS_WEEK_KILLS, h.this_week_kills.min(0xFFFF) as u16, 0);
    f.set_u32(PLAYER_FIELD_THIS_WEEK_CONTRIBUTION, h.this_week_contribution);
    f.set_u32(PLAYER_FIELD_LIFETIME_HONORABLE_KILLS, h.lifetime_honorable);
    f.set_u32(PLAYER_FIELD_LIFETIME_DISHONORABLE_KILLS, h.lifetime_dishonorable);
    f.set_u32(PLAYER_FIELD_YESTERDAY_CONTRIBUTION, h.yesterday_contribution);
    f.set_u32(PLAYER_FIELD_LAST_WEEK_CONTRIBUTION, h.last_week_contribution);
    f.set_u32(PLAYER_FIELD_LAST_WEEK_RANK, h.last_week_standing);
    // The rank, as a title: PLAYER_BYTES_3's last byte.
    f.set_byte(PLAYER_BYTES_3, 3, h.rank);
    f.set_byte(PLAYER_FIELD_BYTES, 3, h.highest_rank);
    f.set_byte(PLAYER_FIELD_BYTES2, 0, h.rank_bar);
}

/// `MSG_INSPECT_HONOR_STATS`: a player's honor, for the inspect window.
pub fn inspect(map: &Map, g: Guid, target: Guid) {
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

/// A player hurt another: the damage counts toward sharing the kill.
pub fn damaged(map: &mut Map, attacker: Guid, victim: Guid, damage: u32) {
    let now = map.now;
    if !guid::is_player(attacker) || attacker == victim || damage == 0 {
        return;
    }
    let Some(p) = map.players.get_mut(&victim) else {
        return;
    };
    let h = &mut p.honor_damage;
    if h.last.is_some_and(|t| now.duration_since(t) > DAMAGE_MEMORY) {
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
    let (Some(v), Some(k)) = (map.players.get(&victim), map.players.get(&killer)) else {
        return;
    };
    if k.team == v.team {
        return;
    }
    let victim_team = v.team;
    let at = v.position();
    let history: Vec<(Guid, u32)> = v.honor_damage.current(now).to_vec();
    for g in [killer, victim] {
        register(map, g);
    }
    let total: u32 = history.iter().map(|(_, d)| *d).sum();
    let mut shares: Vec<(Guid, f32)> = Vec::new();
    let eligible = |map: &Map, g: Guid| {
        map.players.get(&g).is_some_and(|p| {
            p.death == Death::Alive
                && p.team != victim_team
                && crate::game::map::combat::dist(p.position(), at) <= SHARE_YARDS
        }) || g == killer
    };
    if total == 0 {
        shares.push((killer, 1.0));
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
    for (g, share) in shares {
        register(map, g);
        let award = map.realm.social.honor.lock().player_kill(
            guid::counter(g),
            guid::counter(victim),
            share,
        );
        if let Some(a) = award {
            credit(map, g, a.points as i32, victim, a.victim_rank);
            refresh(map, g);
        }
    }
}

/// A creature died at a player's hand: killing a civilian is dishonorable
/// (unless it was worth nothing anyway), an enemy leader honorable.
pub fn creature_killed(map: &mut Map, killer: Guid, creature: Guid) {
    if !guid::is_player(killer) {
        return;
    }
    let Some(c) = map.creature(creature) else {
        return;
    };
    let Some(t) = map.realm.data.creature_template(c.entry) else {
        return;
    };
    let (civilian, leader) = (t.civilian, t.racial_leader);
    let xp_free = t.xp_multiplier == 0.0;
    let mob_level = c.fields.get_u32(UNIT_FIELD_LEVEL);
    let Some(level) = map.players.get(&killer).map(|p| p.level()) else {
        return;
    };
    let award = if civilian
        && (mob_level <= crate::game::map::combat::gray_level(u32::from(level)) || xp_free)
    {
        // A gray civilian costs nothing more than the kill.
        None
    } else if civilian {
        Some(Award {
            kind: AwardKind::Dishonorable,
            points: honor::dishonorable_points(level),
            victim: None,
            victim_rank: 0,
        })
    } else if leader {
        Some(Award {
            kind: AwardKind::Honorable,
            points: 488.0,
            victim: Some(u64::from(c.entry) << 32),
            victim_rank: 19,
        })
    } else {
        None
    };
    let Some(a) = award else {
        return;
    };
    register(map, killer);
    map.realm.social.honor.lock().award(guid::counter(killer), a);
    let shown = if a.kind == AwardKind::Dishonorable {
        -(a.points as i32)
    } else {
        a.points as i32
    };
    credit(map, killer, shown, creature, a.victim_rank);
    refresh(map, killer);
}

/// Bonus honor: battleground objectives and wins.
pub fn bonus_honor(map: &mut Map, g: Guid, amount: u32) {
    register(map, g);
    map.realm.social.honor.lock().award(
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

/// Reputation with a battleground's faction. The world keeps no
/// reputation yet, so this is where it will be given.
pub fn reputation(map: &mut Map, g: Guid, faction: u32, amount: i32) {
    let _ = map;
    tracing::trace!("{g}: {amount} reputation with faction {faction}");
}

/// The realm's honor in time: the day moves on, each finished week is
/// ranked, and what changed is kept. Online players see their new fields.
pub fn sweep(realm: &crate::game::Realm) {
    let (settled, saves) = {
        let mut h = realm.social.honor.lock();
        let settled = h.advance(honor::today());
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
