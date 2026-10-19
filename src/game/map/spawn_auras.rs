//! The auras a creature is given by its spawn's addon (or, naming none,
//! its template): on as it comes into the world, and back on once it
//! has evaded home (Thistle Tea's `Mob.apply_addon_auras` and
//! `restore_addon_auras`). Golemagg's Trust, a stealther's stealth and a
//! warrior's Battle Stance are among them.

use wow_world_messages::Guid;

use crate::game::map::Map;
use crate::game::map::auras;
use crate::game::map::spells::{self, EFFECT_APPLY_AREA_AURA_PARTY, EFFECT_APPLY_AURA};

/// The Explosive Sheep: its passive and its suicide are its fuse's, not
/// its addon's (Thistle Tea's `Engineering.guardian_addon_auras`).
const EXPLOSIVE_SHEEP: u32 = 2675;
const SHEEP_SKIPS: [u32; 2] = [4051, 8327];

/// The creature's spawn auras, each once; an engineered guardian's own
/// passive comes another way.
pub fn spells(map: &Map, g: Guid) -> Vec<u32> {
    let Some(c) = map.creature(g) else {
        return Vec::new();
    };
    let data = &map.realm.data;
    let ids = match c
        .spawn_guid
        .and_then(|s| data.creature_addons.get(&s))
        .filter(|a| !a.auras.is_empty())
    {
        Some(addon) => &addon.auras,
        None => match data.creature_templates.get(&c.entry) {
            Some(t) => &t.auras,
            None => return Vec::new(),
        },
    };
    let passive = crate::game::map::item_scripts::guardian_passive(c.entry);
    let skips =
        |s: u32| (c.entry == EXPLOSIVE_SHEEP && SHEEP_SKIPS.contains(&s)) || Some(s) == passive;
    let mut once = Vec::with_capacity(ids.len());
    for s in ids {
        if !once.contains(s) && !skips(*s) {
            once.push(*s);
        }
    }
    once
}

/// A creature come into the world takes its spawn auras.
pub fn apply(map: &mut Map, g: Guid) {
    for spell in spells(map, g) {
        put_on(map, g, spell);
    }
}

/// A creature home from evading takes back the spawn auras it lost.
pub fn restore(map: &mut Map, g: Guid) {
    crate::game::map::guards::restore(map, g);
    for spell in spells(map, g) {
        if !auras::auras(map, g).is_some_and(|a| a.has_spell(spell)) {
            put_on(map, g, spell);
        }
    }
}

/// `spell`'s aura effects on the creature, cast by itself.
fn put_on(map: &mut Map, g: Guid, spell: u32) {
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(spell) else {
        return;
    };
    let level = map.unit_view(g).map_or(1, |u| u.level);
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| {
            matches!(
                s.effects[*i].effect,
                EFFECT_APPLY_AURA | EFFECT_APPLY_AREA_AURA_PARTY
            ) && s.effects[*i].aura != 0
        })
        .map(|i| (i, spells::points(s, i, level)))
        .collect();
    if !effects.is_empty() {
        let _ = auras::apply_spell(map, g, g, s, &effects, auras::Context::default());
    }
}
