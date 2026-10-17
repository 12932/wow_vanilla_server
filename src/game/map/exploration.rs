//! Exploring: an area's bit in `PLAYER_EXPLORED_ZONES` the first time a
//! player enters it, and vmangos's XP for it: the base for the area's
//! level, less for an area far below the player, and the player's level
//! plus five's for one far above.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::{Area, SMSG_EXPLORATION_EXPERIENCE};

use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::player::Player;

/// The 64 words of explored bits.
pub const WORDS: usize = 64;

/// The explored bits into the fields, from the stored character.
pub fn init(p: &mut Player) {
    p.row.state.explored_zones.resize(WORDS, 0);
    for (i, w) in p.row.state.explored_zones.iter().enumerate() {
        p.fields.set_u32(PLAYER_EXPLORED_ZONES_1 + i as u16, *w);
    }
}

/// XP for exploring an area of `area_level` at `level`.
pub fn xp(data: &GameData, level: u32, area_level: u32) -> u32 {
    let base = |l: u32| data.misc.exploration_xp.get(&l).copied().unwrap_or(0);
    if area_level == 0 {
        return 0;
    }
    let diff = level as i32 - area_level as i32;
    if diff < -5 {
        base(level + 5)
    } else if diff > 5 {
        let percent = (100 - (diff - 5) * 5).clamp(0, 100) as u32;
        base(area_level) * percent / 100
    } else {
        base(area_level)
    }
}

/// A player is in `area`: explored, with XP the first time. Not by the
/// dead or a ghost, nor from a flight (mangos's
/// `CheckAreaExploreAndOutdoor`, Thistle Tea's `discover_area`).
pub fn entered(map: &mut Map, g: Guid, area: u32) {
    let data = map.realm.data.clone();
    let Some(info) = data.area(area) else {
        return;
    };
    let bit = info.explore_bit;
    let (word, mask) = ((bit / 32) as usize, 1u32 << (bit % 32));
    if word >= WORDS {
        return;
    }
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if p.death != crate::game::map::death::Death::Alive || p.flight.is_some() {
        return;
    }
    p.row.state.explored_zones.resize(WORDS, 0);
    if p.row.state.explored_zones[word] & mask != 0 {
        return;
    }
    p.row.state.explored_zones[word] |= mask;
    let value = p.row.state.explored_zones[word];
    p.fields
        .set_u32(PLAYER_EXPLORED_ZONES_1 + word as u16, value);
    let level = u32::from(p.level());
    let gain = if level >= u32::from(crate::game::map::stats::MAX_LEVEL) {
        0
    } else {
        xp(&data, level, info.level)
    };
    p.outbound.send(&SMSG_EXPLORATION_EXPERIENCE {
        area: Area::try_from(area).unwrap_or_default(),
        experience: gain,
    });
    if gain > 0 {
        crate::game::map::stats::give_xp(map, g, gain, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exploring_gives_less_for_low_areas_and_capped_for_high() {
        let mut data = GameData::empty();
        for (l, x) in [(10u32, 100u32), (15, 150), (20, 200), (30, 300)] {
            data.misc.exploration_xp.insert(l, x);
        }
        assert_eq!(xp(&data, 10, 10), 100);
        // Ten levels below: 75%.
        assert_eq!(xp(&data, 30, 20), 150);
        // Far above: the player's level plus five.
        assert_eq!(xp(&data, 10, 30), 150);
        assert_eq!(xp(&data, 10, 0), 0);
    }
}
