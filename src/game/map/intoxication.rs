//! Drink, as Thistle Tea's `Player.Intoxication` keeps it: each inebriating
//! sip (spell effect 100) adds its points times 256 to a player's drunk
//! value, at most 65535; every ten seconds 256 wears off. The client reads
//! the value from `PLAYER_BYTES_3` (beside the gender bit) for its
//! staggering, blur and "you feel tipsy" messages; drunk enough, a player
//! sees what hides behind drunken invisibility. Death sobers.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;

use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::death::Death;
use crate::game::map::player::Player;

/// How often a drink wears off, and how much each time.
pub const SOBER_EVERY: Duration = Duration::from_secs(10);
pub const SOBER_STEP: u32 = 256;
pub const MAXIMUM: u32 = 65_535;
/// The invisibility only the drunk see through.
pub const DRUNK_INVISIBILITY: i32 = 6;

/// A player's drunk value, and when it next wears off.
#[derive(Debug, Clone, Copy, Default)]
pub struct Drunk {
    pub value: u32,
    pub sober_at: Option<Instant>,
}

/// How drunk a player is.
pub fn value(map: &Map, g: Guid) -> u32 {
    map.players.get(&g).map_or(0, |p| p.drunk.value)
}

/// `g` drinks `amount` points: alive, it gets drunker (or, for a
/// negative amount, soberer).
pub fn drink(map: &mut Map, g: Guid, amount: i32) {
    let now = map.now;
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if p.death != Death::Alive || amount == 0 {
        return;
    }
    tick(p, now);
    let value = (i64::from(p.drunk.value) + i64::from(amount) * 256).clamp(0, i64::from(MAXIMUM));
    let deadline = p.drunk.sober_at.unwrap_or(now + SOBER_EVERY);
    put(p, value as u32, deadline);
}

/// At login: as drunk as at logout, sobering from the first tick (Thistle
/// Tea keeps `drunk_value` with the character, mangos its `drunk` column).
pub fn restore(p: &mut Player) {
    p.drunk = Drunk {
        value: p.row.state.drunk.min(MAXIMUM),
        sober_at: None,
    };
    write(p);
}

/// Sober at once (death).
pub fn clear(map: &mut Map, g: Guid) {
    if let Some(p) = map.players.get_mut(&g)
        && p.drunk.value > 0
    {
        put(p, 0, map.now);
    }
}

/// The drunk wear off a little.
pub fn update(map: &mut Map) {
    let now = map.now;
    for p in map.players.values_mut().filter(|p| p.drunk.value > 0) {
        tick(p, now);
    }
}

fn tick(p: &mut Player, now: Instant) {
    if p.drunk.value == 0 {
        return;
    }
    match p.drunk.sober_at {
        None => p.drunk.sober_at = Some(now + SOBER_EVERY),
        Some(t) if t <= now => {
            let value = p.drunk.value.saturating_sub(SOBER_STEP);
            put(p, value, now + SOBER_EVERY);
        }
        Some(_) => {}
    }
}

fn put(p: &mut Player, value: u32, deadline: Instant) {
    p.drunk = Drunk {
        value,
        sober_at: (value > 0).then_some(deadline),
    };
    write(p);
}

/// `PLAYER_BYTES_3`'s low half: the gender bit, and the drunk value above it.
pub fn write(p: &mut Player) {
    let gender = p.row.gender & 1;
    let low = (p.drunk.value & 0xFFFE) as u16 | u16::from(gender);
    p.fields.set_byte(PLAYER_BYTES_3, 0, low as u8);
    p.fields.set_byte(PLAYER_BYTES_3, 1, (low >> 8) as u8);
}
