//! Phase 3, round 6, against Thistle Tea's `spell_target_resolver_test.exs`:
//! areas' recipients, their caps, friends' areas, and spells at a duel's
//! fighters.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at};
use super::tests_p6_wspell::{
    BEAST, DRAGONKIN, RANGE_30, creature, face_east, faction, off, put, sides, unit_at,
};
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::pets::Kind;
use crate::game::map::spells::{Targets, checks, fail, mods, targets};

/// A neutral creature, of a faction without a standing.
const NEUTRAL: u32 = 99_640;
const R5: u32 = 996;
const R10: u32 = 997;

/// Damage about the caster with a trigger and a dummy at the caster's own
/// place; a summon at the caster's place; damage about the caster, two at
/// most, for beasts; on the ground and in a cone, two at most; one about
/// the caster with an aura on the caster, one at most; a heal of friends
/// about the caster and at a place; a dispel of any unit; a heal of a
/// friend; a dismissal of the pet for beasts.
const BURST: u32 = 991_600;
const SUMMON: u32 = 991_601;
const BEASTS_TWO: u32 = 991_602;
const GROUND_TWO: u32 = 991_603;
const CONE_TWO: u32 = 991_604;
const AREA_AND_SELF: u32 = 991_605;
const FRIENDS_HERE: u32 = 991_606;
const FRIENDS_THERE: u32 = 991_607;
const DISPEL: u32 = 991_608;
const HEAL: u32 = 991_609;
const DISMISS: u32 = 991_610;
/// A magic blessing and a magic curse.
const BLESSED: u32 = 991_611;
const CURSED: u32 = 991_612;

fn data(d: &mut GameData) {
    sides(d);
    d.faction_templates.insert(7, faction(7, 7, 0, 0, 0));
    d.creature_templates
        .insert(NEUTRAL, creature(NEUTRAL, 7, 7));
    d.spells.radii.insert(R5, 5.0);
    d.spells.radii.insert(R10, 10.0);
    let spell = |id: u32, effects: &[SpellEffect]| {
        let mut s = bare(id, 0);
        s.range_index = RANGE_30;
        for (i, e) in effects.iter().enumerate() {
            effect(&mut s, i, *e);
        }
        s
    };
    let area = |a: u32, b: u32| SpellEffect {
        target_a: a,
        target_b: b,
        radius_index: R10,
        ..damage(10)
    };
    let heal = |a: u32, b: u32, r: u32| SpellEffect {
        effect: 10,
        base_points: 9,
        base_dice: 1,
        target_a: a,
        target_b: b,
        radius_index: r,
        ..Default::default()
    };
    let at_caster = |e: u32, target: u32| SpellEffect {
        effect: e,
        target_a: target,
        trigger_spell: if e == 64 { HEAL } else { 0 },
        misc_value: if e == 28 { BEAST as i32 } else { 0 },
        ..Default::default()
    };
    let mut beasts = spell(BEASTS_TWO, &[area(22, 15)]);
    beasts.max_affected_targets = 2;
    beasts.target_creature_type = 1;
    let mut ground = spell(GROUND_TWO, &[area(16, 0)]);
    ground.max_affected_targets = 2;
    let mut cone = spell(CONE_TWO, &[area(24, 0)]);
    cone.max_affected_targets = 2;
    let mut area_and_self = spell(AREA_AND_SELF, &[area(22, 15), aura(29, 0, 1, 1)]);
    area_and_self.max_affected_targets = 1;
    let mut dismiss = spell(
        DISMISS,
        &[SpellEffect {
            effect: crate::game::map::pets::EFFECT_DISMISS_PET,
            target_a: 5,
            ..Default::default()
        }],
    );
    dismiss.target_creature_type = 1;
    let magic = |id: u32, e: SpellEffect| {
        let mut s = bare(id, 5);
        s.duration_index = LASTS_LONG;
        s.dispel = 1;
        effect(&mut s, 0, e);
        s
    };
    let spells: Vec<SpellTemplate> = vec![
        magic(BLESSED, aura(29, 0, 1, 21)),
        magic(CURSED, aura(3, 0, 1, 6)),
        spell(BURST, &[area(22, 15), at_caster(64, 1), at_caster(3, 18)]),
        spell(SUMMON, &[at_caster(28, 18)]),
        beasts,
        ground,
        cone,
        area_and_self,
        spell(FRIENDS_HERE, &[heal(22, 30, R10)]),
        spell(FRIENDS_THERE, &[heal(31, 0, R5)]),
        spell(
            DISPEL,
            &[SpellEffect {
                effect: 38,
                misc_value: 1,
                base_points: 0,
                base_dice: 1,
                target_a: 25,
                ..Default::default()
            }],
        ),
        spell(HEAL, &[heal(21, 0, 0)]),
        dismiss,
    ];
    insert(d, spells);
}

/// Whom effect `index` of `spell` from `caster` at `t` lands on.
fn on(m: &Map, caster: Guid, spell: u32, index: usize, t: &Targets) -> Vec<Guid> {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let mods = mods::for_spell(m, caster, &s);
    let mut v = targets::effect_targets(m, caster, &s, index, t, &mods);
    v.sort_by_key(|g| g.guid());
    v
}

fn sorted(mut v: Vec<Guid>) -> Vec<Guid> {
    v.sort_by_key(|g| g.guid());
    v
}

/// An area's damage lands on the enemies about its caster, a trigger at
/// the caster on the caster, each apart, and a place's effect on no unit;
/// a summon at the caster's place on no unit, an enemy picked or not. An
/// area for beasts passes over the dead and the other types before it
/// takes its two, the dead one picked not among them; a ground area or a
/// cone takes two of those in it and not the one picked far off; an
/// effect at the caster takes no place of an area's one; a neutral
/// creature is an enemy to a player's area (Thistle Tea's target
/// resolver; mangos's `SetTargetMap` per effect and `FillAreaTargets`).
#[tokio::test]
async fn an_areas_effects_land_on_their_own_and_its_cap_on_what_it_may_hit() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6XN", "Arear", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            face_east(m, a);
            let enemy = unit_at(m, a, BEAST, 3.0, 0.0);
            let none = Targets::default();
            let burst = [0, 1, 2].map(|i| on(m, a, BURST, i, &at(enemy)));
            let summon = (on(m, a, SUMMON, 0, &none), on(m, a, SUMMON, 0, &at(enemy)));
            m.remove_creature(enemy);
            let eligible = unit_at(m, a, BEAST, 3.0, 0.0);
            let dead = unit_at(m, a, BEAST, 3.0, 1.0);
            crate::game::map::combat::kill(m, a, dead);
            let dragon = unit_at(m, a, DRAGONKIN, 3.0, -1.0);
            let beasts = on(m, a, BEASTS_TWO, 0, &at(dead));
            for g in [eligible, dead, dragon] {
                m.remove_creature(g);
            }
            let four: Vec<Guid> = (0..4)
                .map(|i| unit_at(m, a, BEAST, 3.0, i as f32 * 0.2))
                .collect();
            let distant = unit_at(m, a, BEAST, 50.0, 0.0);
            let there = Targets {
                unit: Some(distant),
                dest: Some(off(m, a, 3.0, 0.0)),
                ..Default::default()
            };
            let capped = [GROUND_TWO, CONE_TWO].map(|s| on(m, a, s, 0, &there));
            let with_self = (
                on(m, a, AREA_AND_SELF, 0, &none),
                on(m, a, AREA_AND_SELF, 1, &none),
            );
            for g in four.iter().copied().chain([distant]) {
                m.remove_creature(g);
            }
            let neutral = unit_at(m, a, NEUTRAL, 3.0, 0.0);
            let neutral_hit = on(m, a, BURST, 0, &none);
            (
                (enemy, burst, summon),
                (eligible, beasts),
                (four, capped, with_self),
                (neutral, neutral_hit),
            )
        })
        .await
        .unwrap();
    let ((enemy, burst, summon), (eligible, beasts), (four, capped, with_self), neutral) = out;
    assert_eq!(burst[0], [enemy], "the damage, the enemy");
    assert_eq!(burst[1], [a], "the trigger, the caster");
    assert!(burst[2].is_empty(), "the place's, no unit");
    assert!(
        summon.0.is_empty() && summon.1.is_empty(),
        "a summon, no unit"
    );
    assert_eq!(beasts, [eligible], "the living beast alone");
    for c in capped {
        assert_eq!(c.len(), 2, "two");
        assert!(c.iter().all(|g| four.contains(g)), "of those in it");
    }
    assert_eq!(with_self.0.len(), 1, "one enemy");
    assert_eq!(with_self.1, [a], "and the caster beside");
    assert_eq!(neutral.1, [neutral.0], "a neutral creature");
}

/// A friends' area about its caster takes the caster and friends in it,
/// not the dead, the far, or an enemy picked; one at a place, those about
/// the place, not about the caster or an enemy there (Thistle Tea's
/// friendly areas; mangos's `SPELL_TARGETS_ASSISTABLE`).
#[tokio::test]
async fn a_friends_area_takes_friends_about_its_centre() {
    let realm = fast_realm(data);
    let (_c, enemy, map) = human(&realm, "P6XO", "Intruder", Class::Mage).await;
    let out = map
        .query(move |m| {
            let source = unit_at(m, enemy, BEAST, 0.0, 10.0);
            let near = off(m, source, 1.0, 0.0);
            put(m, enemy, near);
            let ally = unit_at(m, source, BEAST, 3.0, 0.0);
            let dead = unit_at(m, source, BEAST, 2.0, 0.0);
            crate::game::map::combat::kill(m, enemy, dead);
            let distant = unit_at(m, source, BEAST, 11.0, 0.0);
            let here = on(m, source, FRIENDS_HERE, 0, &at(enemy));
            for g in [ally, dead, distant] {
                m.remove_creature(g);
            }
            let friend = unit_at(m, source, BEAST, 25.0, 0.0);
            let far = off(m, source, 25.0, 0.5);
            put(m, enemy, far);
            let place = Targets {
                unit: Some(enemy),
                dest: Some(off(m, source, 25.0, 0.0)),
                ..Default::default()
            };
            let there = on(m, source, FRIENDS_THERE, 0, &place);
            (source, ally, here, friend, there)
        })
        .await
        .unwrap();
    let (source, ally, here, friend, there) = out;
    assert_eq!(here, sorted(vec![source, ally]), "the caster and the ally");
    assert_eq!(there, [friend], "about the place");
}

/// In a duel, a dispel of any unit goes at the other side from the fighter
/// and their pet, and a pet's at its master; a heal at the other side is
/// refused (Thistle Tea's duel targets; mangos's `CanAssistSpell` for a
/// friend's target, no filter for any unit's). A pet's dismissal is for
/// the pet whatever its type.
#[tokio::test]
async fn a_duels_fighters_are_each_others_and_no_one_elses() {
    let realm = fast_realm(data);
    let (_c, owner, map) = human(&realm, "P6XP", "Duelist", Class::Priest).await;
    let (_d, target, _) = human(&realm, "P6XQ", "Opponent", Class::Priest).await;
    let (_e, outsider, _) = human(&realm, "P6XR", "Onlooker", Class::Priest).await;
    let out = map
        .query(move |m| {
            for g in [owner, target, outsider] {
                kit::ready(m, g, 60, &[DISPEL, HEAL, DISMISS]);
            }
            let to = off(m, owner, 1.0, 0.0);
            let pet = crate::game::map::pets::testing::spawn(
                m,
                owner,
                DRAGONKIN,
                Kind::Hunter,
                60,
                to,
                0.0,
            );
            m.players.get_mut(&owner).unwrap().owned.pet = Some(pet);
            use crate::game::map::duel::{Duel, Phase};
            let arbiter = Guid::new(0xF110_0000_0000_0001);
            for (me, them, initiator) in [(owner, target, true), (target, owner, false)] {
                m.players.get_mut(&me).unwrap().duel = Some(Duel {
                    opponent: them,
                    flag: arbiter,
                    phase: Phase::Fighting,
                    initiator,
                    out_of_bounds_since: None,
                });
            }
            // Something of each side's to dispel.
            for g in [owner, target] {
                kit::apply(m, g, g, BLESSED);
                kit::apply(m, outsider, g, CURSED);
            }
            let why = |m: &mut Map, caster: Guid, spell: u32, at_unit: Guid| {
                kit::fresh(m, caster);
                let s = m.realm.data.spells.get(spell).cloned().unwrap();
                let mods = mods::for_spell(m, caster, &s);
                checks::check(m, caster, &s, &at(at_unit), None, false, &mods)
            };
            let mut out = vec![
                why(m, owner, DISPEL, target),
                why(m, pet, DISPEL, target),
                why(m, pet, DISPEL, owner),
                why(m, outsider, DISPEL, target),
                why(m, owner, HEAL, target),
            ];
            m.unit_fields_mut(target)
                .unwrap()
                .set_flag(UNIT_FIELD_FLAGS, 0x100, true);
            out.push(why(m, pet, DISPEL, target));
            let dismissal = on(m, owner, DISMISS, 0, &Targets::default());
            (pet, out, dismissal)
        })
        .await
        .unwrap();
    let (pet, out, dismissal) = out;
    assert_eq!(out[0], None, "the fighter at the other side");
    assert_eq!(out[1], None, "their pet at the other side");
    assert_eq!(out[2], None, "the pet at its master");
    // A spell at any unit asks no side of its target (mangos's `CheckCast`
    // has no filter for `TARGET_UNIT`): one outside the duel may cast it,
    // and at one immune to players' side, as at any unit not unselectable.
    assert_eq!(out[3], None, "from outside the duel, at any unit");
    assert_eq!(
        out[4],
        Some(fail::TARGET_ENEMY),
        "no heal for the other side"
    );
    assert_eq!(out[5], None, "at any unit immune to players");
    assert_eq!(dismissal, [pet], "the pet, a dragonkin");
}
