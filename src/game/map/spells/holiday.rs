//! The holidays' scripted spells, as Thistle Tea's `Spell.Holiday` (and
//! vmangos's `EffectDummy` and `EffectScriptEffect`) runs them, each from
//! the effect's target.
//!
//! Hallow's End: an innkeeper's Trick or Treat marks the player and either
//! bags them a treat or tricks them into a random costume, the Hallowed
//! Wands dress a friendly player out of combat in their costume, and
//! Hallow's End Candy turns its eater into something random. Feast of
//! Winter Veil: a snowball knocks down a party or raid member who hasn't
//! grown resistant, mistletoe makes its target respond, Greatfather
//! Winter's mistletoe gives mistletoe or holly, and the PX-238 Winter
//! Wondervolt swaps the costume of whoever stands at its heart. A Bag of
//! Heart Candies gives one of its candies. In the Lunar Festival, Elune's
//! Candle sends one of its flames at Omen, a chosen flame at his minions,
//! and a harmless spark at anything else.

use rand::RngExt;
use wow_world_messages::Guid;

use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras;
use crate::game::map::combat::dist;
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;

const TRICK_OR_TREAT: u32 = 24751;
const TRICKED_OR_TREATED: u32 = 24755;
const TRICK: u32 = 24714;
const TREAT: u32 = 24715;
const RANDOM_COSTUME: u32 = 24720;
const HALLOWS_END_CANDY: u32 = 24930;
const CANDY_TRANSFORMATIONS: [u32; 4] = [24924, 24925, 24926, 24927];
/// The Hallowed Wands, and the costume each gives.
const WANDS: [(u32, Costume); 4] = [
    (24717, Costume::Pirate),
    (24718, Costume::Ninja),
    (24719, Costume::LeperGnome),
    (24737, Costume::Ghost),
];
const TRICKS: [Costume; 8] = [
    Costume::Pirate,
    Costume::Ninja,
    Costume::LeperGnome,
    Costume::Ghost,
    Costume::Skeleton,
    Costume::Bat,
    Costume::Wisp,
    Costume::Critter,
];
const WAND_COSTUMES: [Costume; 7] = [
    Costume::Pirate,
    Costume::Ninja,
    Costume::LeperGnome,
    Costume::Skeleton,
    Costume::Bat,
    Costume::Ghost,
    Costume::Wisp,
];
const SNOWBALL: u32 = 21343;
const SNOWBALL_KNOCKDOWN: u32 = 21167;
const SNOWBALL_RESISTANT: u32 = 21354;
const MISTLETOE: u32 = 26004;
const MISTLETOE_RESPONSE: u32 = 26005;
const GREATFATHER_MISTLETOE: u32 = 26218;
const MISTLETOE_GIFTS: [u32; 2] = [26206, 26207];
const WONDERVOLT: u32 = 26275;
const WONDERVOLT_COSTUMES: [u32; 4] = [26272, 26157, 26273, 26274];
const WONDERVOLT_REACH: f32 = 1.0;
const BAG_OF_CANDIES: u32 = 26678;
const HEART_CANDIES: [u32; 8] = [26668, 26670, 26671, 26672, 26673, 26674, 26675, 26676];
const ELUNES_CANDLE: u32 = 26374;
const CANDLE_FIZZLE: u32 = 26636;
const OMEN: u32 = 15467;
const OMEN_CANDLES: [u32; 5] = [26622, 26623, 26624, 26625, 26649];
const MINION_OF_OMEN: u32 = 15466;
const MINION_CANDLE: u32 = 26624;

#[derive(Debug, Clone, Copy)]
enum Costume {
    Pirate,
    Ninja,
    LeperGnome,
    Ghost,
    Skeleton,
    Bat,
    Wisp,
    Critter,
}

impl Costume {
    /// Its spell for `female` or not.
    fn spell(self, female: bool) -> u32 {
        let (male, woman) = match self {
            Costume::Pirate => (24708, 24709),
            Costume::Ninja => (24711, 24710),
            Costume::LeperGnome => (24712, 24713),
            Costume::Ghost => (24735, 24736),
            Costume::Skeleton => (24723, 24723),
            Costume::Bat => (24732, 24732),
            Costume::Wisp => (24740, 24740),
            Costume::Critter => (24753, 24753),
        };
        if female { woman } else { male }
    }
}

fn pick<T: Copy>(choices: &[T]) -> T {
    choices[rand::rng().random_range(0..choices.len())]
}

fn female(map: &Map, g: Guid) -> bool {
    map.unit_fields(g)
        .is_some_and(|f| f.get_byte(UNIT_FIELD_BYTES_0, 2) == 1)
}

fn in_combat(map: &Map, g: Guid) -> bool {
    map.unit_fields(g)
        .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT))
}

fn grouped(map: &Map, a: Guid, b: Guid) -> bool {
    let groups = map.realm.social.groups.lock();
    groups
        .group_of(a)
        .is_some_and(|g| g.members.iter().any(|m| m.guid == b))
}

/// Run a holiday spell's script effect on `target`; whether `s` is one.
pub fn effect(map: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate) -> bool {
    let player = map.players.contains_key(&target);
    let trigger = |map: &mut Map, from: Guid, at: Guid, spell: u32| {
        super::cast_triggered(map, from, spell, Some(at));
    };
    match s.id {
        TRICK_OR_TREAT if player => {
            trigger(map, target, target, TRICKED_OR_TREATED);
            trigger(map, target, target, pick(&[TRICK, TREAT]));
        }
        TRICK if player && caster == target => {
            let costume = pick(&TRICKS).spell(female(map, target));
            trigger(map, caster, target, costume);
        }
        RANDOM_COSTUME if player && !in_combat(map, target) => {
            let costume = pick(&WAND_COSTUMES).spell(female(map, target));
            trigger(map, caster, target, costume);
        }
        id if WANDS.iter().any(|(w, _)| *w == id) => {
            if player && !in_combat(map, target) {
                let costume = WANDS.iter().find(|(w, _)| *w == id).map(|(_, c)| *c);
                if let Some(c) = costume {
                    trigger(map, caster, target, c.spell(female(map, target)));
                }
            }
        }
        HALLOWS_END_CANDY if caster == target => {
            trigger(map, caster, target, pick(&CANDY_TRANSFORMATIONS));
        }
        SNOWBALL if player && caster != target => {
            let thrower = map.players.contains_key(&caster);
            let resistant =
                auras::auras(map, target).is_some_and(|a| a.has_spell(SNOWBALL_RESISTANT));
            if thrower && !resistant && grouped(map, caster, target) {
                trigger(map, target, target, SNOWBALL_KNOCKDOWN);
            }
        }
        MISTLETOE => trigger(map, target, target, MISTLETOE_RESPONSE),
        GREATFATHER_MISTLETOE if player => {
            trigger(map, caster, target, pick(&MISTLETOE_GIFTS));
        }
        WONDERVOLT => {
            let radius = |g: Guid| {
                map.unit_fields(g)
                    .map_or(0.0, |f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS))
            };
            let reach = WONDERVOLT_REACH + radius(caster) + radius(target);
            let at_heart = match (map.position_of(caster), map.position_of(target)) {
                (Some(a), Some(b)) => dist(a, b) < reach,
                _ => false,
            };
            if at_heart {
                auras::remove_where(map, target, |h| WONDERVOLT_COSTUMES.contains(&h.spell));
                trigger(map, target, target, pick(&WONDERVOLT_COSTUMES));
            }
        }
        BAG_OF_CANDIES if caster == target => {
            trigger(map, caster, target, pick(&HEART_CANDIES));
        }
        ELUNES_CANDLE => {
            let flame = match map.creature(target).map(|c| c.entry) {
                Some(OMEN) => pick(&OMEN_CANDLES),
                Some(MINION_OF_OMEN) => MINION_CANDLE,
                _ => CANDLE_FIZZLE,
            };
            trigger(map, caster, target, flame);
        }
        TRICK_OR_TREAT
        | TRICK
        | RANDOM_COSTUME
        | HALLOWS_END_CANDY
        | SNOWBALL
        | GREATFATHER_MISTLETOE
        | BAG_OF_CANDIES => {}
        _ => return false,
    }
    true
}
