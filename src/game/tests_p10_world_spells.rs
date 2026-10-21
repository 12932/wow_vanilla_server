//! Round 10's spell tests of the world, ported from Thistle Tea's
//! `spell_units_test.exs`: a script target's inverse effect mask
//! (`spell_script_target.inverseEffectMask`, mangos-classic's
//! `CanNotHitWithSpellEffect`), which keeps a row's creatures off some of
//! a spell's effects, so that effects aimed alike choose apart, an area
//! whose rows all exclude it finds no one, and the living and the dead of
//! one entry are told apart per effect.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at};
use super::tests_p6_wspell::{BEAST, DRAGONKIN, RANGE_30, face_east, sides, unit_at};
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::spells::{Targets, mods, targets};

/// Two damages at a script's creature near the caster (30 and 40), the
/// first the beast's alone, the second the dragonkin's.
const MIXED: u32 = 991_800;
/// Two dummies at a script's beast near the caster: the first at the
/// living, the second at the dead.
const LIFE: u32 = 991_801;
/// A script area of ten yards about the caster whose only row (beasts)
/// keeps its one effect off.
const EXCLUDED: u32 = 991_802;
/// A script area of ten yards about the caster, of at most one target,
/// its first effect at living beasts, its second at dead ones.
const CAPPED: u32 = 991_803;
const R10: u32 = 999;
/// A demon (112) summoned where the cast was aimed (no implicit target,
/// as Inferno), a demon at its minion's spot (32) and a possessed eye
/// (73) there, five yards out.
const DEMON_THERE: u32 = 991_804;
const DEMON_AT_SPOT: u32 = 991_805;
const EYE: u32 = 991_806;
const R5: u32 = 997;

fn data(d: &mut GameData) {
    sides(d);
    d.spells.radii.insert(R10, 10.0);
    let at_script = |s: &mut SpellTemplate, i: usize, effect: u32, amount: i32, a: u32, b: u32| {
        crate::game::tests_p3_cast_kit::effect(
            s,
            i,
            SpellEffect {
                effect,
                base_points: amount - 1,
                base_dice: 1,
                target_a: a,
                target_b: b,
                radius_index: if b == 7 { R10 } else { 0 },
                ..Default::default()
            },
        );
    };
    let mut mixed = bare(MIXED, 0);
    mixed.range_index = RANGE_30;
    at_script(&mut mixed, 0, 2, 30, 38, 0);
    at_script(&mut mixed, 1, 2, 40, 38, 0);
    let mut life = bare(LIFE, 0);
    life.range_index = RANGE_30;
    at_script(&mut life, 0, 3, 1, 38, 0);
    at_script(&mut life, 1, 3, 1, 38, 0);
    let mut excluded = bare(EXCLUDED, 0);
    at_script(&mut excluded, 0, 3, 1, 22, 7);
    let mut capped = bare(CAPPED, 0);
    capped.max_affected_targets = 1;
    at_script(&mut capped, 0, 3, 1, 22, 7);
    at_script(&mut capped, 1, 3, 1, 22, 7);
    d.spells.radii.insert(R5, 5.0);
    let summon = |id: u32, effect: u32, target: u32| {
        let mut s = bare(id, 0);
        s.range_index = RANGE_30;
        crate::game::tests_p3_cast_kit::effect(
            &mut s,
            0,
            SpellEffect {
                effect,
                base_points: 0,
                base_dice: 1,
                target_a: target,
                radius_index: R5,
                misc_value: BEAST as i32,
                ..Default::default()
            },
        );
        s
    };
    let summons = [
        summon(DEMON_THERE, 112, 0),
        summon(DEMON_AT_SPOT, 112, 32),
        summon(EYE, 73, 32),
    ];
    insert(d, [mixed, life, excluded, capped]);
    insert(d, summons);
    let s = &mut d.spells;
    s.script_targets
        .insert(MIXED, vec![(1, BEAST), (1, DRAGONKIN)]);
    s.script_target_masks.insert((MIXED, 1, BEAST), 2);
    s.script_target_masks.insert((MIXED, 1, DRAGONKIN), 1);
    for spell in [LIFE, CAPPED] {
        s.script_targets.insert(spell, vec![(1, BEAST), (2, BEAST)]);
        s.script_target_masks.insert((spell, 1, BEAST), 2);
        s.script_target_masks.insert((spell, 2, BEAST), 1);
    }
    s.script_targets.insert(EXCLUDED, vec![(1, BEAST)]);
    s.script_target_masks.insert((EXCLUDED, 1, BEAST), 1);
}

fn aimed(m: &Map, caster: Guid, spell: u32, index: usize, t: &Targets) -> Vec<Guid> {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let mods = mods::for_spell(m, caster, &s);
    let mut v = targets::effect_targets(m, caster, &s, index, t, &mods);
    v.sort();
    v
}

fn lost(m: &Map, g: Guid) -> u32 {
    let f = m.unit_fields(g).unwrap();
    f.get_u32(UNIT_FIELD_MAXHEALTH) - f.get_u32(UNIT_FIELD_HEALTH)
}

/// Thistle Tea's `SpellUnits` "keeps each creature effect separate from
/// the caster's heal" and "mixed areas deliver each heal only to its own
/// recipients": a spell of two damages aimed alike at a script's creature
/// near its caster, its beast row kept off the second and its dragonkin
/// row off the first, deals the first's 30 to the beast and the second's
/// 40 to the dragonkin farther off, where the two shared the nearest (the
/// beast, 70); triggered, it does so again ("triggered casts preserve
/// per-effect recipients"). mangos-classic's `FillTargetMap` reuses an
/// effect's targets only where `CheckBoundsEqualForEffects` holds.
#[tokio::test]
async fn effects_aimed_alike_choose_apart_where_their_rows_keep_them_off() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P10WS1", "Mixer", Class::Mage).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[MIXED]);
            face_east(m, a);
            let beast = unit_at(m, a, BEAST, 1.0, 0.0);
            let dragon = unit_at(m, a, DRAGONKIN, 2.0, 0.0);
            let none = Targets::default();
            let each = (aimed(m, a, MIXED, 0, &none), aimed(m, a, MIXED, 1, &none));
            assert!(kit::cast_fresh(m, a, MIXED, none));
            let cast = (lost(m, beast), lost(m, dragon));
            crate::game::map::spells::cast_triggered(m, a, MIXED, None);
            let triggered = (lost(m, beast), lost(m, dragon));
            ((beast, dragon), each, cast, triggered)
        })
        .await
        .unwrap();
    let ((beast, dragon), each, hurt, triggered) = out;
    assert_eq!(each, (vec![beast], vec![dragon]), "each effect its own");
    assert_eq!(
        hurt,
        (30, 40),
        "the beast the first's, the dragonkin the second's"
    );
    assert_eq!(triggered, (60, 80), "triggered, alike");
}

/// Thistle Tea's `SpellUnits.resolve/3` "filters life states and inverse
/// effect masks independently", "an excluded selector does not turn into
/// an unrestricted area" and "retains life state and target caps per
/// effect": with a living beast's row kept off the second effect and a
/// dead beast's off the first, the first aims at the living beast (even
/// with the dead one picked) and the second at the dead one; an area whose
/// only row keeps its effect off finds no one, not every unit there; and a
/// script area of one target at most keeps the one picked of the living
/// for its first effect and the dead one for its second.
#[tokio::test]
async fn a_rows_effect_mask_splits_the_living_and_the_dead_per_effect() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P10WS2", "Splitter", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[LIFE, EXCLUDED, CAPPED]);
            face_east(m, a);
            let first = unit_at(m, a, BEAST, 1.0, 0.0);
            let dead = unit_at(m, a, BEAST, 2.0, 0.0);
            let picked = unit_at(m, a, BEAST, 3.0, 0.0);
            crate::game::map::combat::kill(m, a, dead);
            let none = Targets::default();
            let life = (
                aimed(m, a, LIFE, 0, &none),
                aimed(m, a, LIFE, 1, &none),
                aimed(m, a, LIFE, 0, &at(dead)),
            );
            let excluded = aimed(m, a, EXCLUDED, 0, &none);
            let capped = (
                aimed(m, a, CAPPED, 0, &at(picked)),
                aimed(m, a, CAPPED, 1, &at(picked)),
            );
            ((first, dead, picked), life, excluded, capped)
        })
        .await
        .unwrap();
    let ((first, dead, picked), life, excluded, capped) = out;
    assert_eq!(
        life,
        (vec![first], vec![dead], vec![first]),
        "the living for the first, the dead for the second"
    );
    assert!(excluded.is_empty(), "no one, not everyone: {excluded:?}");
    assert_eq!(capped, (vec![picked], vec![dead]), "capped per effect");
}

/// Where `caster`'s summon of `spell` stands, the newest.
fn summoned(m: &Map, spell: u32) -> Option<Vector3d> {
    m.creatures
        .iter()
        .filter(|(_, c)| c.fields.get_u32(UNIT_CREATED_BY_SPELL) == spell)
        .max_by_key(|(_, c)| c.guid.guid())
        .map(|(_, c)| c.position)
}

fn near(a: Option<Vector3d>, b: Vector3d) -> bool {
    a.is_some_and(|a| crate::game::map::combat::dist(a, b) < 0.01)
}

/// Thistle Tea's `SpellEffect.receive/4` "possessed and demon summons
/// retain resolved minion coordinates": a demon (effect 112) of no
/// implicit target summoned where the cast was aimed, ten yards on, stands
/// there (Inferno's infernal); one at its minion's spot (32) and a
/// possessed eye (73) there stand five yards out on the caster's front
/// left, where both stood two yards before her. mangos-classic's
/// `EffectSummonDemon` and `EffectSummonPossessed` summon at the cast's
/// destination.
#[tokio::test]
async fn demon_and_possessed_summons_stand_where_their_target_puts_them() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P10WS3", "Summoner", Class::Warlock).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[DEMON_THERE, DEMON_AT_SPOT, EYE]);
            face_east(m, a);
            let me = m.players[&a].position();
            let aimed = Vector3d {
                x: me.x + 10.0,
                y: me.y + 3.0,
                z: me.z,
            };
            let there = Targets {
                dest: Some(aimed),
                ..Default::default()
            };
            assert!(kit::cast_fresh(m, a, DEMON_THERE, there));
            let demon_there = near(summoned(m, DEMON_THERE), aimed);
            let a45 = std::f32::consts::FRAC_PI_4;
            let spot = Vector3d {
                x: me.x + 5.0 * a45.cos(),
                y: me.y + 5.0 * a45.sin(),
                z: me.z,
            };
            // (A second demon of the entry replaces the first.)
            assert!(kit::cast_fresh(m, a, DEMON_AT_SPOT, Targets::default()));
            let demon_spot = near(summoned(m, DEMON_AT_SPOT), spot);
            assert!(kit::cast_fresh(m, a, EYE, Targets::default()));
            (demon_there, demon_spot, near(summoned(m, EYE), spot))
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true, true), "(where aimed, the spot, the spot)");
}
