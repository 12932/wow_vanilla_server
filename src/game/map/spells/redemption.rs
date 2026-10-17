//! Spirit of Redemption, as Thistle Tea's `Entity` and aura scripts run it:
//! a priest with the talent who would die becomes a spirit instead, at full
//! health and mana, rooted and unattackable, free to heal for fifteen
//! seconds; then the spirit's end kills them.

use wow_world_messages::Guid;

use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras;

pub const TALENT: u32 = 20711;
pub const FORM: u32 = 27827;
pub const STATE: [u32; 2] = [27792, 27795];
pub const SUICIDE: u32 = 27965;
pub const DURATION_MS: u32 = 15_000;

/// Whether `g` is a spirit of redemption.
pub fn spirit(map: &Map, g: Guid) -> bool {
    auras::auras(map, g).is_some_and(|a| a.has_spell(FORM))
}

/// `g` would die: a priest with the talent becomes a spirit instead.
/// Returns whether they did (and so don't die).
pub fn saves(map: &mut Map, g: Guid) -> bool {
    let Some(p) = map.players.get(&g) else {
        return false;
    };
    if !p.spells.contains(&TALENT) || spirit(map, g) || map.spells.redeemed.contains(&g) {
        return false;
    }
    let data = map.realm.data.clone();
    if let Some(f) = map.unit_fields_mut(g) {
        let max = f.get_u32(UNIT_FIELD_MAXHEALTH).max(1);
        f.set_u32(UNIT_FIELD_HEALTH, max);
        let mana = f.get_u32(UNIT_FIELD_MAXPOWER1);
        f.set_u32(UNIT_FIELD_POWER1, mana);
    }
    crate::game::map::combat::leave_combat(map, g);
    crate::game::map::ai::forget(map, g);
    let max = map
        .unit_fields(g)
        .map(|f| f.get_u32(UNIT_FIELD_MAXHEALTH) as i32)
        .unwrap_or(1);
    for id in std::iter::once(FORM).chain(STATE) {
        let Some(s) = data.spells.get(id) else {
            continue;
        };
        let effects: Vec<(usize, i32)> = (0..3)
            .filter(|i| s.effects[*i].effect == 6 && s.effects[*i].aura != 0)
            .map(|i| {
                let points = if id == FORM && i == 0 {
                    max
                } else {
                    super::points(s, i, 60)
                };
                (i, points)
            })
            .collect();
        let _ = auras::apply_spell(
            map,
            g,
            g,
            s,
            &effects,
            auras::Context {
                duration: Some(Some(DURATION_MS)),
                ..Default::default()
            },
        );
    }
    true
}

/// A spirit's state ran out: the spirit dies.
pub fn aura_gone(map: &mut Map, g: Guid, spell: u32, expired: bool) {
    if spell != STATE[1] || !expired {
        return;
    }
    // The spirit dies (the talent doesn't save them twice); death takes
    // whatever of the spirit's auras are left.
    map.spells.redeemed.insert(g);
    let health = map
        .unit_fields(g)
        .map(|f| f.get_u32(UNIT_FIELD_HEALTH))
        .unwrap_or(0);
    crate::game::map::combat::deal_damage(map, g, g, health.max(1));
    map.spells.redeemed.remove(&g);
}

/// While a spirit, only healing spells may be cast.
pub fn allows(map: &Map, g: Guid, s: &crate::game::data::spells::SpellTemplate) -> bool {
    !spirit(map, g)
        || s.effects
            .iter()
            .any(|e| matches!(e.effect, 10 | 67) || e.aura == 8)
}
