//! Phase 3, round 5, against Thistle Tea's `cast_validation_test.exs`:
//! the checks of a cast's target that it had no test of, dispels with
//! nothing to take and burns of a power the target hasn't.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, fresh};
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::{AURA_MOD_STAT, AURA_PERIODIC_DAMAGE};
use crate::game::map::spells::{checks, fail, mods::Mods};

const CLEANSE: u32 = 990_600;
const PURGE: u32 = 990_601;
const SLAM: u32 = 990_602;
const MASS: u32 = 990_603;
const ALL: u32 = 990_604;
const MANA_BURN: u32 = 990_605;
/// Auras of a dispel type: a magic buff, a magic debuff, a disease, a
/// poison, an enrage.
const MAGIC_BUFF: u32 = 990_610;
const MAGIC_DEBUFF: u32 = 990_611;
const DISEASE: u32 = 990_612;
const POISON: u32 = 990_613;
const ENRAGE: u32 = 990_614;

fn dispels(d: &mut GameData) {
    d.spells.radii.insert(900, 15.0);
    let spell = |id: u32, effects: &[SpellEffect]| {
        let mut s = bare(id, 1);
        for (i, e) in effects.iter().enumerate() {
            effect(&mut s, i, *e);
        }
        s
    };
    let dispel = |kind: i32, target: u32| SpellEffect {
        effect: 38,
        misc_value: kind,
        base_points: 0,
        base_dice: 1,
        target_a: target,
        ..Default::default()
    };
    let lasting = |id: u32, dispel: u32, e: SpellEffect| {
        let mut s = bare(id, 0);
        s.dispel = dispel;
        s.duration_index = LASTS_LONG;
        effect(&mut s, 0, e);
        s
    };
    let mut slam = spell(SLAM, &[dispel(1, 6), damage(10)]);
    slam.dmg_class = 2;
    let mut mass = spell(MASS, &[dispel(1, 21)]);
    mass.effects[0].radius_index = 900;
    let spells: Vec<SpellTemplate> = vec![
        spell(CLEANSE, &[dispel(4, 21), dispel(3, 21), dispel(1, 21)]),
        spell(PURGE, &[dispel(1, 6)]),
        slam,
        mass,
        spell(ALL, &[dispel(7, 21)]),
        spell(
            MANA_BURN,
            &[SpellEffect {
                effect: 62,
                base_points: 99,
                base_dice: 1,
                target_a: 6,
                ..Default::default()
            }],
        ),
        lasting(MAGIC_BUFF, 1, aura(AURA_MOD_STAT, 0, 5, 1)),
        lasting(MAGIC_DEBUFF, 1, aura(AURA_PERIODIC_DAMAGE, 0, 1, 6)),
        lasting(DISEASE, 3, aura(AURA_PERIODIC_DAMAGE, 0, 1, 6)),
        lasting(POISON, 4, aura(AURA_PERIODIC_DAMAGE, 0, 1, 6)),
        lasting(ENRAGE, 9, aura(AURA_PERIODIC_DAMAGE, 0, 1, 6)),
    ];
    insert(d, spells);
}

fn why(m: &mut Map, caster: Guid, spell: u32, target: Guid) -> Option<u8> {
    fresh(m, caster);
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    checks::check(m, caster, &s, &at(target), None, false, &Mods::default())
}

fn only(m: &mut Map, from: Guid, on: Guid, aura: Option<u32>) {
    crate::game::map::auras::remove_where(m, on, |h| (MAGIC_BUFF..=ENRAGE).contains(&h.spell));
    if let Some(a) = aura {
        kit::apply(m, from, on, a);
    }
}

/// A spell all of dispels at a unit wants something there one of them
/// takes: Cleanse a friend with nothing, or only a buff, and it is refused
/// NOTHING_TO_DISPEL; with a disease, it goes. A purge wants an enemy's
/// buff. An attack with a dispel beside (Shield Slam) and an area's dispel
/// want nothing. A dispel of all takes the four kinds, not an enrage
/// (Thistle Tea's dispel validation, mangos's classic `CheckCast`).
#[tokio::test]
async fn a_dispel_with_nothing_to_take_is_refused() {
    let realm = fast_realm(dispels);
    let (_a, a, map) = human(&realm, "P5VA", "Cleanser", Class::Paladin).await;
    let (_b, b, _) = human(&realm, "P5VB", "Afflicted", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[CLEANSE, PURGE, SLAM, MASS, ALL]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            only(m, kobold, b, None);
            let clean = why(m, a, CLEANSE, b);
            only(m, b, b, Some(MAGIC_BUFF));
            let buffed = why(m, a, CLEANSE, b);
            only(m, kobold, b, Some(DISEASE));
            let diseased = why(m, a, CLEANSE, b);
            only(m, kobold, b, Some(MAGIC_DEBUFF));
            let cursed = why(m, a, CLEANSE, b);
            only(m, kobold, kobold, None);
            let bare_enemy = why(m, a, PURGE, kobold);
            only(m, kobold, kobold, Some(MAGIC_BUFF));
            let buffed_enemy = why(m, a, PURGE, kobold);
            only(m, kobold, kobold, None);
            let slam = why(m, a, SLAM, kobold);
            only(m, kobold, b, None);
            let mass = why(m, a, MASS, b);
            let all: Vec<_> = [MAGIC_DEBUFF, DISEASE, POISON, ENRAGE]
                .into_iter()
                .map(|x| {
                    only(m, kobold, b, Some(x));
                    why(m, a, ALL, b)
                })
                .collect();
            (
                clean,
                buffed,
                diseased,
                cursed,
                bare_enemy,
                buffed_enemy,
                slam,
                mass,
                all,
            )
        })
        .await
        .unwrap();
    let nothing = Some(fail::NOTHING_TO_DISPEL);
    let (clean, buffed, diseased, cursed, bare_enemy, buffed_enemy, slam, mass, all) = out;
    assert_eq!(clean, nothing, "nothing on them");
    assert_eq!(buffed, nothing, "a friend's buff isn't for taking");
    assert_eq!(diseased, None);
    assert_eq!(cursed, None, "a magic debuff");
    assert_eq!(bare_enemy, nothing, "a purge of an enemy with no buff");
    assert_eq!(buffed_enemy, None);
    assert_eq!(slam, None, "an attack with a dispel beside");
    assert_eq!(mass, None, "an area's");
    assert_eq!(all, [None, None, None, nothing], "all but an enrage");
}

/// A player's Mana Burn at one of another power is refused BAD_TARGETS
/// (Thistle Tea's power burn validation, mangos's `CheckCast`).
#[tokio::test]
async fn a_burn_of_a_power_the_target_hasnt_is_refused() {
    let realm = fast_realm(dispels);
    let (_a, a, map) = human(&realm, "P5VC", "Burner", Class::Priest).await;
    let (_b, w, _) = human(&realm, "P5VD", "Ragey", Class::Warrior).await;
    let (_c, p, _) = human(&realm, "P5VE", "Manay", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[MANA_BURN]);
            // The players' powers, as their classes have them; a creature's
            // made rage as the warrior's is.
            let s = m.realm.data.spells.get(MANA_BURN).cloned().unwrap();
            let kobold = kit::dummy(m, a, 60, 5.0);
            let creature = why(m, a, MANA_BURN, kobold);
            let powers = [w, p].map(|g| {
                m.unit_fields(g)
                    .map(|f| f.get_byte(crate::game::fields::UNIT_FIELD_BYTES_0, 3))
            });
            // A rage creature: as the warrior is.
            if let Some(f) = m.unit_fields_mut(kobold) {
                f.set_byte(crate::game::fields::UNIT_FIELD_BYTES_0, 3, 1);
            }
            let ragey = checks::check(m, a, &s, &at(kobold), None, false, &Mods::default());
            (creature, powers, ragey)
        })
        .await
        .unwrap();
    let (creature, powers, ragey) = out;
    assert_eq!(creature, None, "a mana creature");
    assert_eq!(powers, [Some(1), Some(0)], "rage and mana");
    assert_eq!(ragey, Some(fail::BAD_TARGETS), "one of rage");
}
