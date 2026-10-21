//! A pet's auras kept in its record while it is away (put away while its
//! master mounts, flies or dies, or teleports far; on another map; logged
//! out; dismissed or stabled), and given back when it comes again, with the
//! time it was away gone from them: mangos-classic's `Pet::_SaveAuras` and
//! `_LoadAuras` (`pet_aura`, its `remaintime` less the time since).

use wow_world_messages::Guid;

use crate::game::map::Map;
use crate::game::map::auras;
use crate::game::map::spells::cooldowns::SavedAura;

/// `SPELL_EFFECT_APPLY_AREA_AURA_PARTY` and `_PET`.
const AREA_AURA_EFFECTS: [u32; 2] = [35, 119];

/// The auras pet `pet` keeps while away: not passive ones or its gear's,
/// not a channel's, not stealth (Prowl), not an area aura's (its own or
/// another's, which the area gives again), not another's lasting ones;
/// its own kept as its own (caster 0), its next self having another GUID.
pub fn saved(map: &Map, pet: Guid) -> Vec<SavedAura> {
    let now = map.now;
    let data = &map.realm.data;
    let channel = map
        .creature(pet)
        .and_then(|c| c.caster.casting.as_ref())
        .filter(|c| c.channel_until.is_some())
        .map(|c| c.spell);
    let Some(a) = auras::auras(map, pet) else {
        return Vec::new();
    };
    a.holders
        .iter()
        .filter(|h| !h.passive && !h.equipped)
        .filter(|h| !(h.caster != pet && h.expires.is_none()))
        .filter(|h| Some(h.spell) != channel)
        .filter(|h| h.expires.is_none_or(|t| t > now))
        .filter(|h| !h.has(auras::AURA_MOD_STEALTH))
        .filter(|h| {
            !data.spells.get(h.spell).is_some_and(|s| {
                s.effects
                    .iter()
                    .any(|e| AREA_AURA_EFFECTS.contains(&e.effect))
            })
        })
        .map(|h| SavedAura {
            spell: h.spell,
            caster: if h.caster == pet { 0 } else { h.caster.guid() },
            item: h.item.map_or(0, |i| i.guid()),
            stacks: h.stacks,
            charges: h.charges,
            left: h
                .expires
                .map(|t| t.saturating_duration_since(now).as_millis() as u32),
            amounts: h.effects.iter().map(|x| (x.effect, x.amount)).collect(),
            combo: h.combo,
            triggered: h.triggered,
        })
        .collect()
}

/// Pet `pet`, come again, takes back the auras its record kept, `away_ms`
/// after they were kept: one whose time ran out while it was away is gone.
pub fn restore(map: &mut Map, pet: Guid, kept: &[SavedAura], away_ms: u64) {
    let data = map.realm.data.clone();
    for a in kept {
        let Some(s) = data.spells.get(a.spell) else {
            continue;
        };
        let left = match a.left {
            Some(ms) if u64::from(ms) <= away_ms => continue,
            Some(ms) => Some(ms - away_ms as u32),
            None => None,
        };
        let effects: Vec<(usize, i32)> = a
            .amounts
            .iter()
            .filter(|(i, _)| (*i as usize) < 3)
            .map(|(i, v)| (*i as usize, *v))
            .collect();
        if effects.is_empty() {
            continue;
        }
        let caster = if a.caster == 0 {
            pet
        } else {
            Guid::new(a.caster)
        };
        let item = (a.item != 0).then(|| Guid::new(a.item));
        let m = crate::game::map::spells::mods::for_spell(map, caster, s);
        let _ = auras::apply_spell(
            map,
            caster,
            pet,
            s,
            &effects,
            auras::Context {
                item,
                equipped: false,
                combo: a.combo,
                duration: Some(left),
                charges: Some(a.charges),
                triggered: a.triggered,
                linked: false,
                activation: (
                    m.flat[crate::game::map::spells::mods::OP_ACTIVATION_TIME],
                    m.pct[crate::game::map::spells::mods::OP_ACTIVATION_TIME],
                ),
                dr: None,
            },
        );
        auras::set_stacks(map, pet, a.spell, caster, item, a.stacks.max(1));
    }
}
