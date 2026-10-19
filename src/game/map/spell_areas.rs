//! `spell_area`: auras that belong to places. A rule's spell is cast on a
//! player who matches it where it holds (its area, quest, race, gender and
//! aura), and an aura of a spell with rules goes when none of them holds,
//! as Thistle Tea's `SpellAreas` reconciles them.

use wow_world_messages::Guid;

use crate::game::data::scripts::SpellArea;
use crate::game::map::Map;

fn matches(map: &Map, g: Guid, r: &SpellArea) -> bool {
    let Some(p) = map.players.get(&g) else {
        return false;
    };
    if r.area != 0 && r.area != p.zone && r.area != p.area {
        return false;
    }
    if r.gender != 2 && u32::from(p.row.gender) != r.gender {
        return false;
    }
    if r.race_mask != 0 && r.race_mask & (1 << (p.row.race.max(1) - 1)) == 0 {
        return false;
    }
    if r.quest_start != 0
        && !(p.journal.rewarded.contains(&r.quest_start)
            || (r.quest_start_active && p.journal.has(r.quest_start)))
    {
        return false;
    }
    if r.quest_end != 0 && p.journal.rewarded.contains(&r.quest_end) {
        return false;
    }
    if r.aura_spell != 0 {
        let has = p.auras.has_spell(r.aura_spell.unsigned_abs());
        if has != (r.aura_spell > 0) {
            return false;
        }
    }
    true
}

/// A player's place-bound auras, in line with where they are and what
/// they have done.
pub fn reconcile(map: &mut Map, g: Guid) {
    let data = map.realm.data.clone();
    let rules = &data.scripts.spell_areas;
    if rules.is_empty() || !map.players.contains_key(&g) {
        return;
    }
    let alive = map
        .players
        .get(&g)
        .is_some_and(|p| p.death == crate::game::map::death::Death::Alive);
    // Auras whose rules no longer hold.
    let stale: Vec<u32> = map
        .players
        .get(&g)
        .map(|p| {
            p.auras
                .holders
                .iter()
                .map(|h| h.spell)
                .filter(|s| {
                    let mine: Vec<&SpellArea> = rules.iter().filter(|r| r.spell == *s).collect();
                    !mine.is_empty() && !mine.iter().any(|r| matches(map, g, r))
                })
                .collect()
        })
        .unwrap_or_default();
    for s in stale {
        crate::game::map::auras::remove_spell(map, g, s, None);
    }
    if !alive {
        return;
    }
    let mut cast: Vec<u32> = rules
        .iter()
        .filter(|r| r.autocast && matches(map, g, r))
        .map(|r| r.spell)
        .collect();
    cast.sort_unstable();
    cast.dedup();
    for s in cast {
        let has = map.players.get(&g).is_some_and(|p| p.auras.has_spell(s));
        if !has {
            crate::game::map::spells::cast_triggered(map, g, s, Some(g));
        }
    }
}

/// Whether `spell` may be cast by `g` where they are: a spell with rules
/// only where one holds.
pub fn allowed(map: &Map, g: Guid, spell: u32) -> bool {
    let rules: Vec<&SpellArea> = map
        .realm
        .data
        .scripts
        .spell_areas
        .iter()
        .filter(|r| r.spell == spell)
        .collect();
    if rules.is_empty() {
        return true;
    }
    // A pet's or a totem's are its master's (mangos's
    // `GetBeneficiaryPlayer`, Thistle Tea's controlling player's facts),
    // where its master is.
    let master = crate::game::map::pets::master(map, g);
    if master != g && map.players.contains_key(&master) {
        return rules.iter().any(|r| matches(map, master, r));
    }
    if !crate::game::guid::is_player(g) {
        // No player to hold to a race, gender, quest or aura: a rule asking
        // one fails, as mangos's `SpellArea::IsFitToRequirements` without a
        // player and Thistle Tea's `Area.matches?` have it. (A creature's
        // place isn't worked out here; its area is taken to hold.)
        return rules.iter().any(|r| !wants_player(r));
    }
    rules.iter().any(|r| matches(map, g, r))
}

/// Whether a rule asks something only a player has.
fn wants_player(r: &SpellArea) -> bool {
    r.gender != 2 || r.race_mask != 0 || r.quest_start != 0 || r.quest_end != 0 || r.aura_spell != 0
}
