//! Seal of Righteousness's holy strike on a melee hit, as mangos-classic's
//! `Unit::HandleDummyAuraProc` (paladin, family flag `0x8000000`) works it
//! out: by the main hand's speed, a two-hander's formula or a
//! one-hander's, 3% of the weapon's base damage, and holy spell power
//! times a share of the speed and the seal's level penalty; cast as its
//! rank's spell (25742 and on), so its modifiers and crits apply.

use wow_world_messages::Guid;

use crate::game::map::Map;
use crate::game::map::inventory::SLOT_MAIN_HAND;
use crate::game::store::BAG_PLAYER;

/// `INVTYPE_2HWEAPON`.
const TWO_HANDED: u32 = 17;
/// mangos's `BASE_ATTACK_TIME`, and a player's base damage unarmed.
const BASE_ATTACK_MS: u32 = 2000;
const UNARMED: (f32, f32) = (1.0, 2.0);

/// The strike's points for a seal of `seal` whose dummy is `amount`, of
/// `owner` hitting `victim`; `None` for no player.
pub fn points(map: &Map, owner: Guid, victim: Guid, seal: u32, amount: i32) -> Option<i32> {
    let p = map.players.get(&owner)?;
    let data = &map.realm.data;
    let weapon = p
        .item_at(BAG_PLAYER, SLOT_MAIN_HAND)
        .and_then(|i| data.item(p.items[i].entry));
    let speed = weapon.map_or(BASE_ATTACK_MS, |w| w.delay.max(1)) as f32 / 1000.0;
    let amount = amount as f32;
    let (base, coeff) = if weapon.is_some_and(|w| w.inventory_type == TWO_HANDED) {
        (
            1.20 * amount * 1.2 * 1.03 * speed / 100.0 + 1.0,
            0.108 * speed,
        )
    } else {
        (
            0.85 * (amount * 1.2 * 1.03 * speed / 100.0).ceil() - 1.0,
            0.092 * speed,
        )
    };
    let (min, max) = weapon.map_or(UNARMED, |w| (w.damages[0].0, w.damages[0].1));
    let mut damage = (base + 0.03 * (min + max) / 2.0) as i32 + 1;
    if damage >= 0
        && let Some(s) = data.spells.get(seal)
    {
        let power = super::outcome::power(map, owner, victim, s, false);
        damage += (power * coeff * super::outcome::level_penalty(s)) as i32;
    }
    Some(damage)
}
