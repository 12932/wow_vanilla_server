//! Round 6's combat tests, damage done against a creature type (Thistle
//! Tea's `target_damage_test.exs`): aura 59's flat damage on a spell, a
//! weapon's blow, another ability and what ticks, and what takes none.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, *};
use crate::game::map::inventory::SLOT_MAIN_HAND;
use crate::game::map::spells::outcome::{self, Landing};
use crate::game::map::{Map, spells};
use crate::game::tests_p3_combat::{self as base, *};

/// Six more damage against beasts (creature type 1).
const VERSUS: u32 = 91_300;
/// Twenty holy spell power.
const HOLY_POWER: u32 = 91_301;
const HOLY: u32 = 1;

const BOLT: u32 = 91_310;
const STRIKE_MELEE: u32 = 91_311;
const STRIKE_RANGED: u32 = 91_312;
const WEAPON_MELEE: u32 = 91_313;
const LEECH: u32 = 91_314;
const DOT: u32 = 91_320;
const PHYSICAL_DOT: u32 = 91_321;
const RANGED_DOT: u32 = 91_322;
const LEECH_DOT: u32 = 91_323;
const HOT: u32 = 91_324;
const PERCENT_DOT: u32 = 91_325;
const FIXED: u32 = 91_330;
const UNMODIFIED: u32 = 91_331;
const HEAL: u32 = 91_332;

fn spell(id: u32, class: u32, effect: SpellEffect) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        range_index: 1,
        equipped_item_class: -1,
        dmg_class: class,
        school: HOLY,
        duration_index: 1,
        ..Default::default()
    };
    s.effects[0] = effect;
    s
}

/// A hundred of `effect` (or aura `aura`), at a coefficient of a half.
fn hundred(effect: u32, aura: u32) -> SpellEffect {
    SpellEffect {
        effect,
        aura,
        base_points: 99,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        bonus_coefficient: 0.5,
        multiple_value: 1.0,
        amplitude: if aura != 0 { 1000 } else { 0 },
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = base::data();
    let mut weapon = spell(
        WEAPON_MELEE,
        2,
        SpellEffect {
            effect: 121,
            base_points: 9,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            ..Default::default()
        },
    );
    weapon.school = 0;
    weapon.effects[1] = SpellEffect {
        effect: 31,
        base_points: 149,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    let mut fixed = spell(FIXED, 1, hundred(2, 0));
    fixed.custom_flags = crate::game::data::spells::CUSTOM_FIXED_DAMAGE;
    let mut unmodified = spell(UNMODIFIED, 1, hundred(2, 0));
    unmodified.attributes[3] |= crate::game::data::spells::ATTR_EX3_IGNORE_CASTER_MODIFIERS;
    let mut heal = spell(HEAL, 1, hundred(10, 0));
    heal.effects[0].target_a = 1;
    for s in [
        spell(BOLT, 1, hundred(2, 0)),
        spell(STRIKE_MELEE, 2, hundred(2, 0)),
        spell(STRIKE_RANGED, 3, hundred(2, 0)),
        weapon,
        spell(LEECH, 1, hundred(9, 0)),
        spell(DOT, 1, hundred(6, AURA_PERIODIC_DAMAGE)),
        spell(PHYSICAL_DOT, 2, hundred(6, AURA_PERIODIC_DAMAGE)),
        spell(RANGED_DOT, 3, hundred(6, AURA_PERIODIC_DAMAGE)),
        spell(LEECH_DOT, 1, hundred(6, AURA_PERIODIC_LEECH)),
        spell(HOT, 1, hundred(6, AURA_PERIODIC_HEAL)),
        spell(PERCENT_DOT, 1, hundred(6, AURA_PERIODIC_DAMAGE_PERCENT)),
        fixed,
        unmodified,
        heal,
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d.spells.durations.insert(
        1,
        SpellDuration {
            base: 3000,
            per_level: 0,
            max: 3000,
        },
    );
    d
}

fn setup_caster(m: &mut Map, g: Guid) {
    level(m, g, 60);
    aura_of(m, g, VERSUS, AURA_MOD_DAMAGE_DONE_CREATURE, 1, 6);
}

fn direct(m: &Map, g: Guid, target: Guid, spell: u32, crit: bool) -> u32 {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let l = Landing {
        crit,
        ..Default::default()
    };
    outcome::damage(m, g, target, &s, 0, 100.0, &l).amount
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn struck(m: &mut Map, g: Guid, target: Guid, spell: u32) -> u32 {
    let before = health(m, target);
    spells::cast_triggered(m, g, spell, Some(target));
    before - health(m, target)
}

/// The amount a periodic aura of `spell` on `target` holds.
fn holds(m: &Map, target: Guid, spell: u32) -> Option<i32> {
    auras::auras(m, target)?
        .holders
        .iter()
        .find(|h| h.spell == spell)
        .map(|h| h.effects[0].amount)
}

/// Thistle Tea's "adds flat magic damage separately from coefficient-scaled
/// spell power", "coefficient-scales non-weapon melee and ranged damage"
/// and "excludes fixed damage, ignored caster modifiers, Ignite, and
/// healing", as mangos's `SpellDamageBonusDone` and `MeleeDamageBonusDone`
/// have them: a spell adds damage done against a beast whole beside its
/// spell power by its coefficient, and its crit takes both; against
/// another type, none. A melee or ranged ability that isn't a weapon's
/// blow takes it by its coefficient. A fixed amount, a spell ignoring its
/// caster's modifiers and a heal take none.
#[tokio::test]
async fn a_spell_adds_damage_against_a_creature_type_whole_and_an_ability_by_its_coefficient() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            setup_caster(m, ga);
            aura_of(m, ga, HOLY_POWER, AURA_MOD_DAMAGE_DONE, 1 << HOLY, 20);
            crate::game::map::derive::unit(m, ga);
            let beast = spawn(m, ga, BEAST, 60, 3.0);
            let brute = spawn(m, ga, BRUTE, 60, 4.0);
            let bolt = [
                direct(m, ga, beast, BOLT, false),
                direct(m, ga, brute, BOLT, false),
                direct(m, ga, beast, BOLT, true),
            ];
            let abilities = [
                direct(m, ga, beast, STRIKE_MELEE, false),
                direct(m, ga, beast, STRIKE_RANGED, false),
            ];
            let none = [
                direct(m, ga, beast, FIXED, false),
                direct(m, ga, beast, UNMODIFIED, false),
            ];
            let f = &mut m.players.get_mut(&ga).unwrap().fields;
            f.set_u32(UNIT_FIELD_HEALTH, 500);
            spells::cast_triggered(m, ga, HEAL, Some(ga));
            (bolt, abilities, none, health(m, ga))
        })
        .await
        .unwrap();
    let (bolt, abilities, none, healed) = r;
    assert_eq!(
        bolt,
        [116, 110, 174],
        "100 + 20 x 0.5 + 6; no beast, no 6; crit"
    );
    assert_eq!(abilities, [103, 103], "100 + 6 x 0.5");
    assert_eq!(none, [100, 100], "fixed; ignoring its caster's modifiers");
    assert_eq!(healed, 600, "a heal takes none");
}

/// Thistle Tea's "adds weapon bonuses once after weapon percentage for
/// melee and ranged" and "leech transfers actual health lost including the
/// bonus", as mangos's `EffectWeaponDmg` and `MeleeDamageBonusDone` have
/// them: a weapon's blow of 150 percent takes the six against a beast once,
/// after its percentage; a leech gives back what it took, the six with it,
/// and no more than the target had.
#[tokio::test]
async fn a_weapons_blow_takes_it_after_its_percentage_and_a_leech_gives_it_back() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            setup_caster(m, ga);
            equip(m, ga, SWORD, SLOT_MAIN_HAND);
            crate::game::map::derive::unit(m, ga);
            let beast = spawn(m, ga, BEAST, 60, 3.0);
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            let weapon = (
                struck(m, ga, beast, WEAPON_MELEE),
                struck(m, ga, brute, WEAPON_MELEE),
            );
            m.players
                .get_mut(&ga)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_HEALTH, 100);
            let took = struck(m, ga, beast, LEECH);
            let gave = health(m, ga) - 100;
            let low = spawn(m, ga, BEAST, 60, 3.0);
            if let Some(c) = m.creature_mut(low) {
                c.fields.set_u32(UNIT_FIELD_HEALTH, 102);
            }
            m.players
                .get_mut(&ga)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_HEALTH, 100);
            spells::cast_triggered(m, ga, LEECH, Some(low));
            (weapon, (took, gave), health(m, ga) - 100)
        })
        .await
        .unwrap();
    let ((beast, brute), leech, capped) = r;
    assert_eq!(
        beast - brute,
        6,
        "{beast} against {brute}: once, after the 150%"
    );
    assert_eq!(leech, (106, 106), "100 + 6, and back");
    assert_eq!(capped, 102, "no more than it had");
}

/// Thistle Tea's "snapshots flat magic and scaled physical bonuses once per
/// periodic application", "does not increase periodic healing or percentage
/// health damage" and "cancellation, expiry, and death remove future
/// bonuses without changing old snapshots": damage over time holds the six
/// against a beast from its application, a spell's whole and an ability's
/// by its coefficient, and ticks it; the buff gone, it holds it still, and
/// cast again it holds none. A heal over time and damage by a percentage
/// of health take none.
#[tokio::test]
async fn damage_over_time_holds_the_bonus_from_its_application() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            setup_caster(m, ga);
            let beast = spawn(m, ga, BEAST, 60, 3.0);
            let held: Vec<Option<i32>> = [DOT, PHYSICAL_DOT, RANGED_DOT, LEECH_DOT]
                .into_iter()
                .map(|s| {
                    spells::cast_triggered(m, ga, s, Some(beast));
                    holds(m, beast, s)
                })
                .collect();
            // A tick of the magic one.
            let before = health(m, beast);
            let now = m.now;
            if let Some(h) = auras::auras_mut(m, beast)
                .and_then(|a| a.holders.iter_mut().find(|h| h.spell == DOT))
            {
                h.effects[0].next_tick = Some(now);
            }
            auras::tick_due(m, beast, DOT, ga);
            let ticked = before - health(m, beast);
            auras::remove_spell(m, ga, VERSUS, None);
            let kept = holds(m, beast, DOT);
            spells::cast_triggered(m, ga, DOT, Some(beast));
            let recast = holds(m, beast, DOT);
            spells::cast_triggered(m, ga, HOT, Some(ga));
            aura_of(m, ga, VERSUS, AURA_MOD_DAMAGE_DONE_CREATURE, 1, 6);
            spells::cast_triggered(m, ga, PERCENT_DOT, Some(beast));
            (
                held,
                ticked,
                (kept, recast),
                (holds(m, ga, HOT), holds(m, beast, PERCENT_DOT)),
            )
        })
        .await
        .unwrap();
    let (held, ticked, (kept, recast), others) = r;
    assert_eq!(
        held,
        [Some(106), Some(103), Some(103), Some(106)],
        "a spell's whole, an ability's by half; a leech's too"
    );
    assert_eq!(ticked, 106);
    assert_eq!(
        (kept, recast),
        (Some(106), Some(100)),
        "held, and none cast again"
    );
    assert_eq!(others, (Some(100), Some(100)), "a heal, a percentage: none");
}
