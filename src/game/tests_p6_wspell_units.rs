//! Phase 3, round 6, against Thistle Tea's `spell_units_test.exs`: the
//! creatures a scripted spell (`spell_script_target`) aims at near its
//! caster, and its refusal without one.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at};
use super::tests_p6_wspell::{BEAST, DRAGONKIN, RANGE_30, face_east, off, put, sides, unit_at};
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::spells::{Targets, checks, fail, mods, targets};

/// A dummy at a script's living beast near the caster, within the range
/// (thirty yards); one within a radius of three; one at a dead beast; one
/// cast in two seconds.
const NEAR: u32 = 991_700;
const CLOSE: u32 = 991_701;
const CORPSE: u32 = 991_702;
const SLOW: u32 = 991_703;
const R3: u32 = 998;
const R10: u32 = 999;
/// Script areas of ten yards: about the caster naming no one; at the
/// destination naming no one; about the caster naming beasts; a cone of
/// beasts; a summon at the destination; about the caster's own place.
const ALL_HERE: u32 = 991_704;
const ALL_THERE: u32 = 991_705;
const BEASTS_HERE: u32 = 991_706;
const BEAST_CONE: u32 = 991_707;
const SUMMON_THERE: u32 = 991_708;
const ALL_AT_ME: u32 = 991_709;

fn data(d: &mut GameData) {
    sides(d);
    d.spells.radii.insert(R3, 3.0);
    let scripted = |id: u32, radius: u32| {
        let mut s = bare(id, 0);
        s.range_index = RANGE_30;
        s.mana_cost = 10;
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: 3,
                target_a: 38,
                radius_index: radius,
                ..Default::default()
            },
        );
        s
    };
    d.spells.radii.insert(R10, 10.0);
    let area = |id: u32, a: u32, b: u32, effect_id: u32| {
        let mut s = bare(id, 0);
        s.range_index = RANGE_30;
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: effect_id,
                target_a: a,
                target_b: b,
                radius_index: R10,
                misc_value: if effect_id == 41 { BEAST as i32 } else { 0 },
                ..Default::default()
            },
        );
        s
    };
    for (spell, kind) in [(BEASTS_HERE, 1), (BEAST_CONE, 1)] {
        d.spells.script_targets.insert(spell, vec![(kind, BEAST)]);
    }
    let mut slow = scripted(SLOW, 0);
    slow.casting_time_index = CAST_2S;
    for (spell, kind) in [(NEAR, 1), (CLOSE, 1), (CORPSE, 2), (SLOW, 1)] {
        d.spells.script_targets.insert(spell, vec![(kind, BEAST)]);
    }
    let spells: Vec<SpellTemplate> = vec![
        area(ALL_HERE, 22, 7, 3),
        area(ALL_THERE, 8, 0, 3),
        area(BEASTS_HERE, 22, 7, 3),
        area(BEAST_CONE, 60, 0, 3),
        area(SUMMON_THERE, 8, 0, 41),
        area(ALL_AT_ME, 18, 8, 3),
        scripted(NEAR, 0),
        scripted(CLOSE, R3),
        scripted(CORPSE, 0),
        slow,
    ];
    insert(d, spells);
}

fn aimed(m: &Map, caster: Guid, spell: u32, t: &Targets) -> Vec<Guid> {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let mods = mods::for_spell(m, caster, &s);
    targets::effect_targets(m, caster, &s, 0, t, &mods)
}

fn why(m: &mut Map, caster: Guid, spell: u32) -> Option<u8> {
    kit::fresh(m, caster);
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let mods = mods::for_spell(m, caster, &s);
    checks::check(m, caster, &s, &Targets::default(), None, false, &mods)
}

fn size(m: &Map, g: Guid) -> f32 {
    m.unit_fields(g)
        .map_or(0.0, |f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS))
}

/// A scripted spell aims at the creature picked if it is one its script
/// names, else the nearest of those; a dead one only for a corpse's row;
/// within the effect's radius, else the spell's range, of both edges, high
/// above too far; with none there it is refused, and one gone as the cast
/// ends fails it before it costs (Thistle Tea's `SpellUnits`; mangos's
/// `CheckScriptTargeting`).
#[tokio::test]
async fn a_scripted_spell_aims_at_its_creature_near_and_is_refused_without_one() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6XS", "Scripter", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[NEAR, CLOSE, CORPSE, SLOW]);
            face_east(m, a);
            let near = unit_at(m, a, BEAST, 1.0, 0.0);
            let far = unit_at(m, a, BEAST, 5.0, 0.0);
            let wrong = unit_at(m, a, DRAGONKIN, 0.5, 0.0);
            let picks = [
                aimed(m, a, NEAR, &Targets::default()),
                aimed(m, a, NEAR, &at(far)),
                aimed(m, a, NEAR, &at(wrong)),
            ];
            let corpse_alive = why(m, a, CORPSE);
            crate::game::map::combat::kill(m, a, near);
            let corpse_dead = (why(m, a, CORPSE), aimed(m, a, CORPSE, &Targets::default()));
            for g in [near, far, wrong] {
                m.remove_creature(g);
            }
            let none = why(m, a, NEAR);
            // High above, beyond thirty yards; then within three yards of
            // both edges and beyond.
            let up = unit_at(m, a, BEAST, 0.0, 0.0);
            let mut high = off(m, a, 0.0, 0.0);
            high.z += 31.0 + size(m, a) + size(m, up);
            put(m, up, high);
            let above = why(m, a, NEAR);
            let reach = 3.0 + size(m, a) + size(m, up);
            let within = off(m, a, reach - 0.05, 0.0);
            put(m, up, within);
            let close = why(m, a, CLOSE);
            let beyond = off(m, a, reach + 0.05, 0.0);
            put(m, up, beyond);
            let too_far = why(m, a, CLOSE);
            // Gone as the cast ends.
            let before = kit::power(m, a, 0);
            assert!(kit::cast_fresh(m, a, SLOW, Targets::default()));
            m.remove_creature(up);
            kit::pass(m, 2_100);
            let gone = (kit::casting(m, a), before - kit::power(m, a, 0));
            (
                (near, far, picks),
                (corpse_alive, corpse_dead),
                none,
                (above, close, too_far),
                gone,
            )
        })
        .await
        .unwrap();
    let ((near, far, picks), (corpse_alive, corpse_dead), none, ranges, gone) = out;
    let refused = Some(fail::BAD_TARGETS);
    assert_eq!(
        picks,
        [vec![near], vec![far], vec![near]],
        "nearest, the picked, not another's"
    );
    assert_eq!(corpse_alive, refused, "a corpse's row: not the living");
    assert_eq!(corpse_dead, (None, vec![near]), "the dead one");
    assert_eq!(none, refused, "none near");
    assert_eq!(
        ranges,
        (refused, None, refused),
        "high above; within three; beyond"
    );
    assert_eq!(gone, (None, 0), "gone as it ends: failed, unpaid");
}

fn sorted(mut v: Vec<Guid>) -> Vec<Guid> {
    v.sort_by_key(|g| g.guid());
    v
}

/// A script's area naming no one takes every unit there, the caster too,
/// and may be empty; naming beasts, the beasts alone. A cone of them is
/// sixty degrees before the caster, as they turn. A summon at its place
/// lands on no unit there. One about the caster's own place is about the
/// caster, whatever place was sent (Thistle Tea's script unit areas;
/// mangos's `TARGET_ENUM_UNITS_SCRIPT_AOE_AT_SRC_LOC`, `_DEST_LOC` and
/// `_IN_CONE_60`).
#[tokio::test]
async fn a_script_area_takes_those_named_or_every_unit_there() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6XT", "Arealist", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            face_east(m, a);
            let beast = unit_at(m, a, BEAST, 2.0, 0.0);
            let dragon = unit_at(m, a, DRAGONKIN, 3.0, 0.0);
            let none = Targets::default();
            let here = sorted(aimed(m, a, ALL_HERE, &none));
            let far = Targets {
                dest: Some(off(m, a, 100.0, 0.0)),
                ..Default::default()
            };
            let there_empty = aimed(m, a, ALL_THERE, &far);
            let near_place = Targets {
                dest: Some(off(m, a, 2.0, 0.0)),
                ..Default::default()
            };
            let there = sorted(aimed(m, a, ALL_THERE, &near_place));
            let beasts = aimed(m, a, BEASTS_HERE, &none);
            let summon = aimed(m, a, SUMMON_THERE, &near_place);
            let at_me = sorted(aimed(m, a, ALL_AT_ME, &far));
            for g in [beast, dragon] {
                m.remove_creature(g);
            }
            let front = unit_at(m, a, BEAST, 5.0, 0.0);
            let edge = unit_at(m, a, BEAST, 4.0, 2.0);
            let side = unit_at(m, a, BEAST, 4.0, 3.0);
            let rear = unit_at(m, a, BEAST, -5.0, 0.0);
            let cone = sorted(aimed(m, a, BEAST_CONE, &at(rear)));
            m.players.get_mut(&a).unwrap().movement.orientation = std::f32::consts::PI;
            let turned = aimed(m, a, BEAST_CONE, &none);
            m.players.get_mut(&a).unwrap().movement.orientation = std::f32::consts::TAU;
            let wrapped = sorted(aimed(m, a, BEAST_CONE, &none));
            (
                (
                    beast,
                    dragon,
                    here,
                    there_empty,
                    there,
                    beasts,
                    summon,
                    at_me,
                ),
                (front, edge, side, rear, cone, turned, wrapped),
            )
        })
        .await
        .unwrap();
    let ((beast, dragon, here, there_empty, there, beasts, summon, at_me), cones) = out;
    let all = sorted(vec![a, beast, dragon]);
    assert_eq!(here, all, "every unit about the caster, the caster too");
    assert!(there_empty.is_empty(), "an empty place");
    assert_eq!(there, all, "every unit about the place");
    assert_eq!(beasts, [beast], "those named alone");
    assert!(summon.is_empty(), "a summon is the place's");
    assert_eq!(
        at_me, all,
        "about the caster's own place, not the place sent"
    );
    let (front, edge, side, rear, cone, turned, wrapped) = cones;
    assert_eq!(cone, sorted(vec![front, edge]), "sixty degrees before");
    assert!(!cone.contains(&side) && !cone.contains(&rear));
    assert_eq!(turned, [rear], "turned round");
    assert_eq!(wrapped, cone, "a full turn on");
}
