//! Rested XP, as vmangos and Thistle Tea's `Core.Player.Rest` have it:
//! resting in an inn (a tavern area trigger) or a friendly capital fills a
//! pool at a level's XP / 1,152,000 a second, up to three quarters of a
//! level; logged out, it fills at that rate where the player rests and a
//! quarter of it elsewhere. Kill XP draws from the pool one for one, up to
//! doubling the kill.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use wow_world_messages::Guid;

use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::player::{PLAYER_FLAGS_RESTING, Player};
use crate::game::registry::Team;

/// Where a player rests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RestType {
    /// In the inn of this tavern trigger.
    Tavern(u32),
    City,
}

const ACCRUAL_DIVISOR: f32 = 1_152_000.0;
const POOL_CAP: f32 = 0.75;
/// `PLAYER_BYTES_2`'s rest state: rested, normal.
const STATE_RESTED: u8 = 1;
const STATE_NORMAL: u8 = 2;
/// `AreaTable` flag: a capital city.
pub const AREA_FLAG_CAPITAL: u32 = 0x100;

/// The XP to the next level, the pool's measure: none at the top level,
/// so nothing rests there (Thistle Tea's `Rest`, a 0 `next_level_xp`).
fn next_level_xp(p: &Player) -> f32 {
    if p.level() >= crate::game::map::stats::max_level() {
        return 0.0;
    }
    p.fields.get_u32(PLAYER_NEXT_LEVEL_XP) as f32
}

/// The pool, kept within its cap, and its client fields.
fn set_bonus(p: &mut Player, bonus: f32) {
    let bonus = bonus.clamp(0.0, next_level_xp(p) * POOL_CAP);
    p.row.state.rest_bonus = bonus;
    p.fields.set_u32(PLAYER_REST_STATE_EXPERIENCE, bonus as u32);
    let state = if bonus > 10.0 {
        STATE_RESTED
    } else if bonus <= 1.0 {
        STATE_NORMAL
    } else {
        p.fields.get_byte(PLAYER_BYTES_2, 3)
    };
    p.fields.set_byte(PLAYER_BYTES_2, 3, state);
}

/// Add to the pool (a GM's `.rested`).
pub fn add_bonus(p: &mut Player, amount: f32) {
    let bonus = p.row.state.rest_bonus + amount;
    set_bonus(p, bonus);
}

/// What resting since `since` has earned: counted every ten seconds, and
/// at a save, so a logout keeps the rest since the last count (Thistle
/// Tea's `Rest.logout` settles it).
pub fn accrue(p: &mut Player, now: Instant) {
    if let (Some(_), Some(since)) = (p.row.state.rest_type, p.misc.rest_since) {
        let secs = now.saturating_duration_since(since).as_secs_f32();
        let gained = secs * next_level_xp(p) / ACCRUAL_DIVISOR * p.misc.rest_rate.unwrap_or(1.0);
        let bonus = p.row.state.rest_bonus + gained;
        set_bonus(p, bonus);
        p.misc.rest_since = Some(now);
    }
}

/// At login: what the time logged out earned, at `rates`' offline
/// rates, and the fields.
pub fn restore(p: &mut Player, now: Instant, rates: &crate::config::RatesConfig) {
    p.misc.rest_rate = Some(rates.rest_in_game);
    let logged_out = p.row.logout_time;
    let wall = chrono::Utc::now().timestamp();
    if logged_out > 0 && wall > logged_out {
        let rate = if p.row.state.rest_type.is_some() {
            rates.rest_offline_in_tavern_or_city
        } else {
            0.25 * rates.rest_offline_in_wilderness
        };
        let gained = (wall - logged_out) as f32 * next_level_xp(p) / ACCRUAL_DIVISOR * rate;
        let bonus = p.row.state.rest_bonus + gained;
        set_bonus(p, bonus);
    } else {
        let bonus = p.row.state.rest_bonus;
        set_bonus(p, bonus);
    }
    let resting = p.row.state.rest_type.is_some();
    p.misc.rest_since = resting.then_some(now);
    p.fields
        .set_flag(PLAYER_FLAGS, PLAYER_FLAGS_RESTING, resting);
}

/// Start resting.
pub fn start(p: &mut Player, kind: RestType, now: Instant) {
    accrue(p, now);
    p.row.state.rest_type = Some(kind);
    p.misc.rest_since = Some(now);
    p.fields.set_flag(PLAYER_FLAGS, PLAYER_FLAGS_RESTING, true);
}

/// Stop resting.
pub fn stop(p: &mut Player, now: Instant) {
    accrue(p, now);
    p.row.state.rest_type = None;
    p.misc.rest_since = None;
    p.fields.set_flag(PLAYER_FLAGS, PLAYER_FLAGS_RESTING, false);
}

/// Kill XP's rested share: up to as much again from the pool.
pub fn spend(p: &mut Player, xp: u32, now: Instant) -> u32 {
    accrue(p, now);
    let bonus = p.row.state.rest_bonus;
    let spent = (bonus as u32).min(xp);
    if spent > 0 {
        set_bonus(p, bonus - spent as f32);
    }
    spent
}

/// A player stepped into an area trigger: an inn's starts their rest.
pub fn area_trigger(map: &mut Map, g: Guid, trigger: u32) {
    let now = map.now;
    if !map.realm.data.npc.taverns.contains(&trigger) {
        return;
    }
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    match p.row.state.rest_type {
        Some(RestType::City) => {}
        Some(RestType::Tavern(t)) if t == trigger => {}
        _ => start(p, RestType::Tavern(trigger), now),
    }
}

/// Whether a zone is a capital of the player's own side.
fn friendly_capital(data: &GameData, team: Team, zone: u32) -> bool {
    data.area(zone).is_some_and(|a| {
        a.flags & AREA_FLAG_CAPITAL != 0
            && match a.team {
                2 => team == Team::Alliance,
                4 => team == Team::Horde,
                _ => true,
            }
    })
}

/// The rest a player's place gives: a capital starts it, leaving the
/// capital or the inn stops it; and the pool's accrual.
pub fn update(map: &mut Map) {
    let now = map.now;
    let data = map.realm.data.clone();
    let rate = map.cfg.rates.rest_in_game;
    let mut left_inn = Vec::new();
    for p in map.players.values_mut() {
        if p.misc
            .rest_checked_at
            .is_some_and(|t| now.duration_since(t).as_secs() < 1)
        {
            continue;
        }
        p.misc.rest_checked_at = Some(now);
        // A reload's rate from now on; what was earned before at the old.
        if p.misc.rest_rate != Some(rate) {
            accrue(p, now);
            p.misc.rest_rate = Some(rate);
        }
        let capital = friendly_capital(&data, p.team, p.zone);
        match p.row.state.rest_type {
            Some(RestType::City) if !capital => stop(p, now),
            None if capital => start(p, RestType::City, now),
            Some(RestType::Tavern(t)) => {
                let inside = data.area_triggers.get(&t).is_some_and(|tr| {
                    tr.map == p.row.map && crate::game::map::world::inside(tr, p.position())
                });
                if !inside {
                    left_inn.push(p.guid);
                }
            }
            _ => {}
        }
        if p.misc
            .rest_since
            .is_some_and(|t| now.duration_since(t).as_secs() >= 10)
        {
            accrue(p, now);
        }
    }
    for g in left_inn {
        if let Some(p) = map.players.get_mut(&g) {
            stop(p, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pool_fills_at_a_level_per_thirteen_days() {
        // 1,152,000 seconds of rest is a level's XP.
        let rate = 1000.0 / ACCRUAL_DIVISOR;
        assert!((rate * 1_152_000.0 - 1000.0).abs() < 0.01);
    }
}
