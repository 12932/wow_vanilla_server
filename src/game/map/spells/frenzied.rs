//! Frenzied Regeneration (22842, 22895, 22896): each tick of its
//! periodic trigger, which names no spell, takes up to 10 of the druid's
//! rage and heals them its points a rage, as mangos-classic's
//! `Aura::TriggerSpell` casts 22845 for it.

use wow_world_messages::Guid;

use crate::game::fields::UNIT_FIELD_POWER1;
use crate::game::map::Map;

const RANKS: [u32; 3] = [22842, 22895, 22896];
const HEAL: u32 = 22845;
/// Rage's power, kept tenfold.
const RAGE: u16 = 1;

/// Whether `spell` is a rank of Frenzied Regeneration.
pub fn is(spell: u32) -> bool {
    RANKS.contains(&spell)
}

/// A tick on `target` of a rank whose points are `per_rage`.
pub fn tick(map: &mut Map, target: Guid, per_rage: i32) {
    let Some(rage) = map.unit_fields_mut(target).map(|f| {
        let have = f.get_u32(UNIT_FIELD_POWER1 + RAGE);
        let take = have.min(100);
        f.set_u32(UNIT_FIELD_POWER1 + RAGE, have - take);
        take
    }) else {
        return;
    };
    let heal = rage as i32 * per_rage / 10;
    super::scripts::cast_with_points(map, target, HEAL, target, [Some(heal), None, None]);
}
