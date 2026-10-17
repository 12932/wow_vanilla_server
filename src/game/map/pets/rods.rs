//! The Taming the Beast rods of the level-10 hunter quests. Each rod's
//! channel (a threat effect and a dummy aura) run its full course casts
//! the matching tame on its beast: a charm for a while, and the quest's
//! event. Thistle Tea's `Hunter.taming_rod_tame` and `Casting`'s
//! `queue_taming_rod_tame`, after vmangos' `Aura::HandleAuraDummy`. The
//! tame's ten-minute tick, Despawn Caster, sends the beast away.

use wow_world_messages::Guid;

use super::owner_of;
use crate::game::map::Map;

/// Each rod's channel, and the tame it casts.
const TAMES: [(u32, u32); 12] = [
    (19548, 19597),
    (19674, 19677),
    (19687, 19676),
    (19688, 19678),
    (19689, 19679),
    (19692, 19680),
    (19693, 19684),
    (19694, 19681),
    (19696, 19682),
    (19697, 19683),
    (19699, 19685),
    (19700, 19686),
];

/// The tames' periodic trigger: the beast goes.
pub const SPELL_DESPAWN_CASTER: u32 = 19614;

/// The tame rod channel `spell` casts, if it is one.
pub fn tame_of(spell: u32) -> Option<u32> {
    TAMES.iter().find(|(rod, _)| *rod == spell).map(|(_, t)| *t)
}

/// Whether `spell` is a rod's tame.
pub fn is_tame(spell: u32) -> bool {
    TAMES.iter().any(|(_, t)| *t == spell)
}

/// `caster`'s channel of `spell` at `target` ran its full course: a rod's
/// tames its beast, the two of them alive and the beast no one's. One
/// charm at a time.
pub fn channel_done(map: &mut Map, caster: Guid, spell: u32, target: Option<Guid>) {
    let Some(tame) = tame_of(spell) else {
        return;
    };
    let Some(beast) = target.filter(|t| *t != caster && map.creature(*t).is_some()) else {
        return;
    };
    if owner_of(map, beast).is_some() {
        return;
    }
    let alive = |g: Guid| map.unit_view(g).is_some_and(|u| u.alive);
    if !alive(caster) || !alive(beast) {
        return;
    }
    let charming = map
        .players
        .get(&caster)
        .and_then(|p| p.owned.charm)
        .is_some_and(|c| c != beast);
    if charming {
        use wow_world_messages::vanilla::SpellCastResult as R;
        crate::game::map::spells::send_cast_result(
            map,
            caster,
            tame,
            Some(R::AlreadyHaveCharm.as_int()),
        );
        return;
    }
    crate::game::map::spells::cast_triggered(map, caster, tame, Some(beast));
}

/// Despawn Caster: the creature casting it goes. An aura's trigger is cast
/// by the aura's caster here, so a tame's comes from its hunter; the beast
/// is their charm, held by the tame.
pub fn despawn(map: &mut Map, caster: Guid) {
    let beast = if map.creature(caster).is_some() {
        Some(caster)
    } else {
        map.players
            .get(&caster)
            .and_then(|p| p.owned.charm)
            .filter(|c| {
                crate::game::map::auras::auras(map, *c).is_some_and(|a| {
                    a.holders
                        .iter()
                        .any(|h| h.caster == caster && is_tame(h.spell))
                })
            })
    };
    let Some(beast) = beast else {
        return;
    };
    // It goes quietly, not at its master.
    if let Some(master) = owner_of(map, beast) {
        super::let_go(map, master, beast, false);
    }
    crate::game::map::scripts::despawn(map, beast);
}
