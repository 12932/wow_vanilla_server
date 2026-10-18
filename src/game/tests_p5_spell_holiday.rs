//! Phase 3, round 5, against Thistle Tea's `holiday_test.exs`: the
//! holidays' scripted spells (`spells::holiday`), each cast as a triggered
//! spell of made-up data with the holiday's ids, and what it casts in turn
//! read off the auras it leaves.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{fresh, holder, spawn};
use crate::game::data::spells::SpellEffect;
use crate::game::data::{CreatureTemplate, GameData};
use crate::game::fields::*;
use crate::game::map::auras::{self, AURA_DUMMY};
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::{Map, spells};

const TRICK: u32 = 24714;
const WANDS: [u32; 5] = [24717, 24718, 24719, 24720, 24737];
const CANDY: u32 = 24930;
const BAG_OF_CANDIES: u32 = 26678;
const SNOWBALL: u32 = 21343;
const KNOCKDOWN: u32 = 21167;
const RESISTANT: u32 = 21354;
const MISTLETOE: u32 = 26004;
const RESPONSE: u32 = 26005;
const GREATFATHER: u32 = 26218;
const WONDERVOLT: u32 = 26275;
const ELUNES_CANDLE: u32 = 26374;
const OMEN: u32 = 15467;
const MINION: u32 = 15466;

const MALE_TRICKS: [u32; 8] = [24708, 24711, 24712, 24735, 24723, 24732, 24740, 24753];
const FEMALE_TRICKS: [u32; 8] = [24709, 24710, 24713, 24736, 24723, 24732, 24740, 24753];
const CANDIES: [u32; 4] = [24924, 24925, 24926, 24927];
const HEART_CANDIES: [u32; 8] = [26668, 26670, 26671, 26672, 26673, 26674, 26675, 26676];
const GIFTS: [u32; 2] = [26206, 26207];
const VOLT_COSTUMES: [u32; 4] = [26272, 26157, 26273, 26274];
const OMEN_FLAMES: [u32; 5] = [26622, 26623, 26624, 26625, 26649];
const MINION_FLAME: u32 = 26624;
const SPARK: u32 = 26636;

/// Everything a holiday spell casts: a long dummy aura on its target.
fn outcomes() -> Vec<u32> {
    let mut v: Vec<u32> = [
        &MALE_TRICKS[..],
        &FEMALE_TRICKS,
        &CANDIES,
        &HEART_CANDIES,
        &GIFTS,
        &VOLT_COSTUMES,
        &OMEN_FLAMES,
        &[KNOCKDOWN, RESISTANT, RESPONSE, SPARK],
    ]
    .concat();
    v.sort();
    v.dedup();
    v
}

fn holidays(d: &mut GameData) {
    let mut spells = Vec::new();
    for id in outcomes() {
        let mut s = bare(id, 0);
        s.duration_index = LASTS_LONG;
        effect(&mut s, 0, aura(AURA_DUMMY, 0, 0, 25));
        spells.push(s);
    }
    let script = |id: u32, kind: u32, target: u32| {
        let mut s = bare(id, 0);
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: kind,
                target_a: target,
                ..Default::default()
            },
        );
        s
    };
    for id in [TRICK, CANDY, BAG_OF_CANDIES] {
        spells.push(script(id, 77, 1));
    }
    for id in WANDS
        .into_iter()
        .chain([MISTLETOE, GREATFATHER, WONDERVOLT])
    {
        spells.push(script(id, 77, 25));
    }
    for id in [SNOWBALL, ELUNES_CANDLE] {
        spells.push(script(id, 3, 25));
    }
    insert(d, spells);
    for entry in [OMEN, MINION] {
        let mut t = d.creature_templates[&KOBOLD].clone();
        t.entry = entry;
        t.name = format!("Creature {entry}");
        d.creature_templates.insert(entry, CreatureTemplate { ..t });
    }
}

/// Which of `among` `g` has on.
fn wearing(m: &Map, g: Guid, among: &[u32]) -> Vec<u32> {
    let a = auras::auras(m, g);
    among
        .iter()
        .copied()
        .filter(|s| a.is_some_and(|a| a.has_spell(*s)))
        .collect()
}

/// Cast `spell` from `caster` at `target` `n` times, each from a clean
/// slate; the outcomes among `among` each time.
fn tries(
    m: &mut Map,
    caster: Guid,
    target: Guid,
    spell: u32,
    among: &[u32],
    n: usize,
) -> Vec<Vec<u32>> {
    (0..n)
        .map(|_| {
            for g in [caster, target] {
                auras::remove_where(m, g, |h| outcomes().contains(&h.spell));
                fresh(m, g);
            }
            spells::cast_triggered(m, caster, spell, Some(target));
            wearing(m, target, among)
        })
        .collect()
}

fn set_female(m: &mut Map, g: Guid, female: bool) {
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_byte(UNIT_FIELD_BYTES_0, 2, u8::from(female));
    }
}

fn set_fighting(m: &mut Map, g: Guid, fighting: bool) {
    if let Some(f) = m.unit_fields_mut(g) {
        let flags = f.get_u32(UNIT_FIELD_FLAGS);
        let flags = if fighting {
            flags | UNIT_FLAG_IN_COMBAT
        } else {
            flags & !UNIT_FLAG_IN_COMBAT
        };
        f.set_u32(UNIT_FIELD_FLAGS, flags);
    }
}

/// Each try put one of `set` on, and over the tries more than one of it.
fn one_each_of(out: &[Vec<u32>], set: &[u32]) {
    for o in out {
        assert_eq!(o.len(), 1, "one at a time: {o:?}");
        assert!(set.contains(&o[0]), "{o:?} of {set:?}");
    }
    let mut seen: Vec<u32> = out.iter().map(|o| o[0]).collect();
    seen.sort();
    seen.dedup();
    assert!(seen.len() > 1, "picked at random: {seen:?}");
}

/// Trick dresses the trickster, by their gender, in one of eight costumes
/// at random; the Hallowed Wands do nothing to a player in combat or to a
/// creature; Hallow's End Candy and a Bag of Heart Candies have their
/// eater cast one of theirs (Thistle Tea's Trick, Hallowed Wands, and
/// candy tests).
#[tokio::test]
async fn hallows_end_tricks_wands_and_candies() {
    let realm = fast_realm(holidays);
    let (_a, a, map) = human(&realm, "P5HA", "Trickster", Class::Warrior).await;
    let (_b, b, _) = human(&realm, "P5HB", "Wandholder", Class::Warrior).await;
    let out = map
        .query(move |m| {
            set_female(m, a, false);
            let male = tries(m, a, a, TRICK, &outcomes(), 40);
            set_female(m, a, true);
            let female = tries(m, a, a, TRICK, &outcomes(), 40);
            set_female(m, a, false);
            set_fighting(m, a, true);
            let fighting: Vec<_> = WANDS
                .iter()
                .flat_map(|w| tries(m, b, a, *w, &outcomes(), 1))
                .collect();
            set_fighting(m, a, false);
            let calm = tries(m, b, a, WANDS[0], &outcomes(), 1);
            let kobold = spawn(m, b, KOBOLD, 60, 5.0);
            let creature: Vec<_> = WANDS
                .iter()
                .flat_map(|w| tries(m, b, kobold, *w, &outcomes(), 1))
                .collect();
            let candy = tries(m, a, a, CANDY, &outcomes(), 30);
            let bag = tries(m, a, a, BAG_OF_CANDIES, &outcomes(), 40);
            (male, female, fighting, calm, creature, candy, bag)
        })
        .await
        .unwrap();
    let (male, female, fighting, calm, creature, candy, bag) = out;
    one_each_of(&male, &MALE_TRICKS);
    one_each_of(&female, &FEMALE_TRICKS);
    assert!(fighting.iter().all(Vec::is_empty), "{fighting:?}");
    assert_eq!(calm, [vec![24708]], "the pirate's, out of combat");
    assert!(creature.iter().all(Vec::is_empty), "{creature:?}");
    one_each_of(&candy, &CANDIES);
    one_each_of(&bag, &HEART_CANDIES);
}

/// A snowball knocks down only a grouped player hit by another player,
/// and not one grown resistant; mistletoe has its target respond; the
/// Greatfather's gives mistletoe or holly (Thistle Tea's Winter Veil
/// tests).
#[tokio::test]
async fn winter_veils_snowballs_and_mistletoe() {
    let realm = fast_realm(holidays);
    let (_a, a, map) = human(&realm, "P5HC", "Thrower", Class::Warrior).await;
    let (_b, b, _) = human(&realm, "P5HD", "Target", Class::Warrior).await;
    let (_c, c, _) = human(&realm, "P5HE", "Stranger", Class::Warrior).await;
    let lo = realm.registry.online(a).unwrap();
    let bo = realm.registry.online(b).unwrap();
    assert!(crate::game::social::groups::join_directly(&realm, &lo, &bo));
    let out = map
        .query(move |m| {
            let all = outcomes();
            let grouped = tries(m, a, b, SNOWBALL, &all, 1);
            let stranger = tries(m, c, b, SNOWBALL, &all, 1);
            let own = tries(m, b, b, SNOWBALL, &all, 1);
            let kobold = spawn(m, a, KOBOLD, 60, 5.0);
            let creature = tries(m, kobold, b, SNOWBALL, &all, 1);
            // Resistant: the knockdown's resistance on before the throw.
            for g in [a, b] {
                auras::remove_where(m, g, |h| all.contains(&h.spell));
            }
            spells::cast_triggered(m, b, RESISTANT, Some(b));
            spells::cast_triggered(m, a, SNOWBALL, Some(b));
            let resistant = wearing(m, b, &[KNOCKDOWN]);
            let kissed = tries(m, a, b, MISTLETOE, &[RESPONSE], 1);
            let responder = holder(m, b, RESPONSE).map(|h| h.caster);
            let gifts = tries(m, kobold, b, GREATFATHER, &all, 30);
            (
                grouped, stranger, own, creature, resistant, kissed, responder, gifts,
            )
        })
        .await
        .unwrap();
    let (grouped, stranger, own, creature, resistant, kissed, responder, gifts) = out;
    assert_eq!(grouped, [vec![KNOCKDOWN]], "a party member knocked down");
    assert_eq!(stranger, [Vec::<u32>::new()], "not by a stranger");
    assert_eq!(own, [Vec::<u32>::new()], "nor by oneself");
    assert_eq!(creature, [Vec::<u32>::new()], "nor by a creature");
    assert!(resistant.is_empty(), "nor once resistant");
    assert_eq!(kissed, [vec![RESPONSE]]);
    assert_eq!(responder, Some(b), "the target responds itself");
    one_each_of(&gifts, &GIFTS);
}

/// The Winter Wondervolt swaps the costume of a player within a yard of
/// its heart (past both sizes) and leaves one further off as they were;
/// Elune's Candle sends one of five flames at Omen, its own at a minion,
/// and a spark at anything else (Thistle Tea's Wondervolt and Elune's
/// Candle tests).
#[tokio::test]
async fn the_wondervolt_and_elunes_candle() {
    let realm = fast_realm(holidays);
    let (_a, a, map) = human(&realm, "P5HF", "Volted", Class::Warrior).await;
    let out = map
        .query(move |m| {
            let all = outcomes();
            let near = spawn(m, a, KOBOLD, 60, 0.5);
            let far = spawn(m, a, KOBOLD, 60, 8.0);
            for g in [a, near, far] {
                if let Some(f) = m.unit_fields_mut(g) {
                    f.set_f32(UNIT_FIELD_BOUNDINGRADIUS, 0.0);
                }
            }
            // Dressed first in the Wondervolt's first costume.
            let mut swapped = Vec::new();
            for _ in 0..30 {
                auras::remove_where(m, a, |h| all.contains(&h.spell));
                spells::cast_triggered(m, a, VOLT_COSTUMES[0], Some(a));
                spells::cast_triggered(m, near, WONDERVOLT, Some(a));
                swapped.push(wearing(m, a, &all));
            }
            auras::remove_where(m, a, |h| all.contains(&h.spell));
            spells::cast_triggered(m, a, VOLT_COSTUMES[0], Some(a));
            spells::cast_triggered(m, far, WONDERVOLT, Some(a));
            let untouched = wearing(m, a, &all);
            let omen = spawn(m, a, OMEN, 60, 5.0);
            let minion = spawn(m, a, MINION, 60, 5.0);
            let at_omen = tries(m, a, omen, ELUNES_CANDLE, &all, 40);
            let at_minion = tries(m, a, minion, ELUNES_CANDLE, &all, 1);
            let at_kobold = tries(m, a, far, ELUNES_CANDLE, &all, 1);
            (swapped, untouched, at_omen, at_minion, at_kobold)
        })
        .await
        .unwrap();
    let (swapped, untouched, at_omen, at_minion, at_kobold) = out;
    one_each_of(&swapped, &VOLT_COSTUMES);
    assert_eq!(untouched, [VOLT_COSTUMES[0]], "too far from its heart");
    one_each_of(&at_omen, &OMEN_FLAMES);
    assert_eq!(at_minion, [vec![MINION_FLAME]]);
    assert_eq!(at_kobold, [vec![SPARK]]);
}
