//! Phase 3's casting at units, as Thistle Tea's tests of it have it:
//! reflection and its charges, a cone's arc, the target cap, on made-up
//! spells and the fast suite.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{
    apply_template, at, dummy, fresh, health, holder, ready, set_health,
};
use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_EX_NO_REFLECTION, ProcEvent, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::*;
use crate::game::map::spells;

const BOLT: u32 = 990_200;
const FROST_BOLT: u32 = 990_201;
const UNREFLECTABLE: u32 = 990_202;
const HEAL: u32 = 990_203;
const REFLECT: u32 = 990_210;
const FROST_REFLECT: u32 = 990_211;
const SOME_REFLECT: u32 = 990_212;
const IMMUNE: u32 = 990_213;

fn reflection(d: &mut GameData) {
    let bolt = |id: u32, school: u32| {
        let mut s = bare(id, school);
        effect(&mut s, 0, damage(100));
        s
    };
    let mut unreflectable = bolt(UNREFLECTABLE, 2);
    unreflectable.attributes[1] |= ATTR_EX_NO_REFLECTION;
    let mut heal = bare(HEAL, 2);
    effect(
        &mut heal,
        0,
        crate::game::data::spells::SpellEffect {
            effect: 10,
            ..damage(100)
        },
    );
    // A reflection of charges, spent by what it turns back (Sheen of
    // Zanza's `spell_proc_event`, of `PROC_EX_REFLECT`).
    let charged = |id: u32, aura_kind: u32, misc: i32, charges: u32| {
        let mut s = bare(id, 0);
        s.duration_index = LASTS_LONG;
        s.proc_flags = crate::game::map::procs::PROC_TAKEN_NEGATIVE_SPELL;
        s.proc_chance = 100;
        s.proc_charges = charges;
        effect(&mut s, 0, aura(aura_kind, misc, 100, 1));
        s
    };
    let mut some = bare(SOME_REFLECT, 0);
    some.duration_index = LASTS_LONG;
    effect(&mut some, 0, aura(AURA_REFLECT_SPELLS, 0, 3, 1));
    let mut immune = bare(IMMUNE, 0);
    immune.duration_index = LASTS_LONG;
    effect(&mut immune, 0, aura(AURA_SCHOOL_IMMUNITY, 0x4, 0, 1));
    insert(
        d,
        [
            bolt(BOLT, 2),
            bolt(FROST_BOLT, 4),
            unreflectable,
            heal,
            charged(REFLECT, AURA_REFLECT_SPELLS, 0, 2),
            charged(FROST_REFLECT, AURA_REFLECT_SPELLS_SCHOOL, 0x10, 2),
            some,
            immune,
        ],
    );
    for (id, school) in [(REFLECT, 0), (FROST_REFLECT, 0x10)] {
        d.spells.proc_events.insert(
            id,
            ProcEvent {
                school_mask: school,
                ex: crate::game::map::procs::EX_REFLECT,
                ..Default::default()
            },
        );
    }
}

fn template(m: &Map, id: u32) -> SpellTemplate {
    m.realm.data.spells.get(id).cloned().unwrap()
}

fn charges(m: &Map, g: Guid, id: u32) -> Option<u32> {
    holder(m, g, id).map(|h| h.charges)
}

/// `caster`'s `spell` at `target`, from a fresh start: the two units'
/// health lost.
fn bolt_at(m: &mut Map, caster: Guid, target: Guid, spell: u32) -> (u32, u32) {
    for g in [caster, target] {
        set_health(m, g, 1_000);
    }
    fresh(m, caster);
    spells::cast(m, caster, spell, at(target), None);
    (1_000 - health(m, caster), 1_000 - health(m, target))
}

/// Reflection: a sure one needs no roll and turns a fire bolt back on its
/// caster, spending a charge each time and going with its last; the
/// target's immunity comes first and spends nothing; an unreflectable
/// spell, a heal and a cast at oneself aren't reflected and spend nothing;
/// a school's reflection keeps its charges against another school's spell
/// (Thistle Tea's `spell_reflection_test`).
#[tokio::test]
async fn a_reflection_turns_a_spell_back_and_spends_a_charge_for_it() {
    let realm = fast_realm(reflection);
    let (_c, mage, map) = human(&realm, "P3CR", "Reflector", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let k = dummy(m, mage, 60, 3.0);
            for g in [mage, k] {
                if let Some(f) = m.unit_fields_mut(g) {
                    f.set_u32(UNIT_FIELD_MAXHEALTH, 10_000);
                }
            }
            let mut seen = Vec::new();
            // No reflection: the bolt lands.
            seen.push(bolt_at(m, k, mage, BOLT));
            let t = template(m, REFLECT);
            apply_template(m, mage, mage, &t);
            let t = template(m, FROST_REFLECT);
            apply_template(m, mage, mage, &t);
            let start = (charges(m, mage, REFLECT), charges(m, mage, FROST_REFLECT));
            // Immune to fire: immune, no charge spent.
            let t = template(m, IMMUNE);
            apply_template(m, mage, mage, &t);
            seen.push(bolt_at(m, k, mage, BOLT));
            let after_immune = charges(m, mage, REFLECT);
            remove_spell(m, mage, IMMUNE, None);
            // Unreflectable, a heal, the mage's own: nothing spent.
            seen.push(bolt_at(m, k, mage, UNREFLECTABLE));
            let after_unreflectable = charges(m, mage, REFLECT);
            // At oneself, triggered: a player casts no spell at an enemy at
            // themselves (round 6, mangos's harmful target filter).
            set_health(m, mage, 1_000);
            spells::cast_triggered(m, mage, BOLT, Some(mage));
            let own = 1_000 - health(m, mage);
            seen.push((own, own));
            let after_own = charges(m, mage, REFLECT);
            // Reflected twice, then gone; the frost reflection kept.
            seen.push(bolt_at(m, k, mage, BOLT));
            let after_one = (charges(m, mage, REFLECT), charges(m, mage, FROST_REFLECT));
            seen.push(bolt_at(m, k, mage, BOLT));
            let after_two = charges(m, mage, REFLECT);
            seen.push(bolt_at(m, k, mage, BOLT));
            // The frost reflection turns back frost.
            seen.push(bolt_at(m, k, mage, FROST_BOLT));
            let frost_after = charges(m, mage, FROST_REFLECT);
            (
                seen,
                start,
                after_immune,
                after_unreflectable,
                after_own,
                after_one,
                after_two,
                frost_after,
            )
        })
        .await
        .unwrap();
    let (seen, start, after_immune, after_unreflectable, after_own, after_one, after_two, frost) =
        out;
    assert_eq!(seen[0], (0, 100), "no reflection: the bolt lands");
    assert_eq!(start, (Some(2), Some(2)));
    assert_eq!(seen[1], (0, 0), "immune");
    assert_eq!(after_immune, Some(2), "and no charge spent");
    assert_eq!(seen[2], (0, 100), "an unreflectable spell lands");
    assert_eq!(after_unreflectable, Some(2), "and spends nothing");
    assert_eq!(seen[3], (100, 100), "one's own bolt isn't reflected");
    assert_eq!(after_own, Some(2));
    assert_eq!(seen[4], (100, 0), "reflected on its caster");
    assert_eq!(
        after_one,
        (Some(1), Some(2)),
        "a charge spent, the frost's kept"
    );
    assert_eq!(seen[5], (100, 0), "reflected again");
    assert_eq!(after_two, None, "the last charge took the reflection");
    assert_eq!(seen[6], (0, 100), "and the next lands");
    assert_eq!(seen[7], (100, 0), "frost's reflection turns frost back");
    assert_eq!(frost, Some(1));
}

/// A reflection's chance is the general one and the school's together,
/// rolled; a heal isn't rolled, nor anything once the reflection has gone
/// (Thistle Tea's `reflect_spell?` and its removal).
#[tokio::test]
async fn reflection_chances_add_by_school_and_go_with_their_auras() {
    let realm = fast_realm(reflection);
    let (_c, mage, map) = human(&realm, "P3CS", "Reflectwo", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let k = dummy(m, mage, 60, 3.0);
            for g in [mage, k] {
                if let Some(f) = m.unit_fields_mut(g) {
                    f.set_u32(UNIT_FIELD_MAXHEALTH, 10_000);
                }
            }
            // 3% general and 20% for fire: 23% of fire, 3% of frost.
            let t = template(m, SOME_REFLECT);
            apply_template(m, mage, mage, &t);
            let mut fire = bare(990_214, 0);
            fire.duration_index = LASTS_LONG;
            effect(&mut fire, 0, aura(AURA_REFLECT_SPELLS_SCHOOL, 0x4, 20, 1));
            apply_template(m, mage, mage, &fire);
            m.spells.rolls = Some(true);
            let (mut fire_back, mut frost_back) = (0, 0);
            for _ in 0..1_000 {
                if bolt_at(m, k, mage, BOLT).0 > 0 {
                    fire_back += 1;
                }
                if bolt_at(m, k, mage, FROST_BOLT).0 > 0 {
                    frost_back += 1;
                }
            }
            m.spells.rolls = None;
            // A heal on the mage isn't turned back.
            let healed = {
                set_health(m, mage, 500);
                fresh(m, k);
                spells::cast(m, k, HEAL, at(mage), None);
                health(m, mage)
            };
            remove_spell(m, mage, SOME_REFLECT, None);
            remove_spell(m, mage, 990_214, None);
            let mut gone = 0;
            m.spells.rolls = Some(true);
            for _ in 0..200 {
                if bolt_at(m, k, mage, BOLT).0 > 0 {
                    gone += 1;
                }
            }
            (fire_back, frost_back, healed, gone)
        })
        .await
        .unwrap();
    let (fire_back, frost_back, healed, gone) = out;
    assert!(
        (150..=320).contains(&fire_back),
        "23% of fire: {fire_back} of 1,000"
    );
    assert!(
        (5..=70).contains(&frost_back),
        "3% of frost: {frost_back} of 1,000"
    );
    assert!(healed > 500, "a heal lands: {healed}");
    assert_eq!(gone, 0, "nothing once the reflections have gone");
}

/// A sure reflection turns back magic of every school, physical too, on
/// its caster; not an ability, a passive, a spell past immunities, nor a
/// spell of another damage class (Thistle Tea's `reflectable?`, mangos's
/// `IsReflectableSpell`).
#[tokio::test]
async fn every_school_of_magic_is_reflected_and_abilities_are_not() {
    use crate::game::data::spells::{ATTR_ABILITY, ATTR_NO_IMMUNITIES, ATTR_PASSIVE};
    let schools: Vec<SpellTemplate> = (0..7)
        .map(|school| {
            let mut s = bare(990_220 + school, school);
            effect(&mut s, 0, damage(100));
            s
        })
        .collect();
    let mut kept: Vec<SpellTemplate> = Vec::new();
    for (i, attr) in [ATTR_ABILITY, ATTR_PASSIVE, ATTR_NO_IMMUNITIES]
        .into_iter()
        .enumerate()
    {
        let mut s = bare(990_230 + i as u32, 2);
        s.attributes[0] |= attr;
        effect(&mut s, 0, damage(100));
        kept.push(s);
    }
    for class in [0, 2, 3] {
        let mut s = bare(990_240 + class, 2);
        s.dmg_class = class;
        effect(&mut s, 0, damage(100));
        kept.push(s);
    }
    let ids: Vec<u32> = schools.iter().map(|s| s.id).collect();
    let kept_ids: Vec<u32> = kept.iter().map(|s| s.id).collect();
    let realm = fast_realm(move |d| {
        reflection(d);
        insert(d, schools);
        insert(d, kept);
        let mut sure = bare(990_250, 0);
        sure.duration_index = LASTS_LONG;
        effect(&mut sure, 0, aura(AURA_REFLECT_SPELLS, 0, 100, 1));
        insert(d, [sure]);
    });
    let (_c, mage, map) = human(&realm, "P3CT", "Reflecthree", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let k = dummy(m, mage, 60, 3.0);
            let t = template(m, 990_250);
            apply_template(m, mage, mage, &t);
            let back: Vec<bool> = ids
                .iter()
                .map(|id| bolt_at(m, k, mage, *id) == (100, 0))
                .collect();
            let not_back: Vec<bool> = kept_ids
                .iter()
                .map(|id| bolt_at(m, k, mage, *id).0 == 0)
                .collect();
            (back, not_back)
        })
        .await
        .unwrap();
    assert_eq!(out.0, [true; 7], "every school reflected whole");
    assert_eq!(
        out.1, [true; 6],
        "abilities, passives, past immunities, other classes"
    );
}

/// A made-up kobold at `x`, `y`, facing `o`.
fn kobold_at(m: &mut Map, x: f32, y: f32, o: f32) -> Guid {
    let t = m.realm.data.creature_template(KOBOLD).cloned().unwrap();
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let c = crate::game::map::creature::Creature::new(
        crate::game::guid::unit(KOBOLD, counter | 0x80_0000),
        &t,
        60,
        Vector3d { x, y, z: 0.0 },
        o,
    )
    .with_base();
    m.add_creature(c)
}

/// A cone: the caster's front, its width either side of its facing, the
/// angle wrapped across a full turn; a negative cone its back (Onyxia's
/// Tail Sweep, -120: mangos's `isInBack`, Thistle Tea's `Cone.contains?`).
#[tokio::test]
async fn a_cone_is_the_casters_front_and_a_negative_one_its_back() {
    use crate::game::map::spells::targets::in_cone;
    use std::f32::consts::PI;
    let out = on_map(
        |_| {},
        |m| {
            let east = kobold_at(m, 0.0, 0.0, 0.0);
            let near_turn = kobold_at(m, 0.0, 0.0, 2.0 * PI - 0.02);
            let west = kobold_at(m, 0.0, 0.0, PI);
            let at = |m: &mut Map, x: f32, y: f32| kobold_at(m, x, y, 0.0);
            let behind = at(m, -10.0, 0.0);
            let behind_wide = at(m, -10.0, 15.0);
            let behind_wider = at(m, -10.0, 20.0);
            let ahead = at(m, 10.0, 0.0);
            let just_ahead = at(m, 10.0, 0.2);
            (
                [
                    in_cone(m, east, behind, -120),
                    in_cone(m, east, behind_wide, -120),
                    in_cone(m, east, behind_wider, -120),
                    in_cone(m, east, ahead, -120),
                ],
                [
                    in_cone(m, near_turn, just_ahead, 7),
                    in_cone(m, west, behind, 60),
                    in_cone(m, west, ahead, 60),
                    in_cone(m, east, ahead, 60),
                ],
            )
        },
    )
    .await;
    assert_eq!(out.0, [true, true, false, false], "the rear arc");
    assert_eq!(out.1, [true, true, false, true], "the front, wrapped");
}

/// A spell's cap on its targets: duplicates go first, the target aimed at
/// is kept if it is among them, and never put in if not; under the cap,
/// all in their order (Thistle Tea's `TargetLimit.select`).
#[tokio::test]
async fn a_target_cap_keeps_the_aimed_at_and_counts_each_once() {
    use crate::game::map::spells::targets::limit;
    let out = on_map(
        |_| {},
        |m| {
            let g: Vec<Guid> = (0..6).map(|i| kobold_at(m, i as f32, 0.0, 0.0)).collect();
            let run = |m: &Map, list: &[usize], max: usize, primary: Option<usize>| {
                let mut v: Vec<Guid> = list.iter().map(|i| g[*i]).collect();
                limit(m, &mut v, max, primary.map(|i| g[i]));
                v.iter()
                    .map(|x| g.iter().position(|y| y == x).unwrap())
                    .collect::<Vec<_>>()
            };
            [
                run(m, &[2, 3, 4, 5], 2, Some(5)),
                run(m, &[2, 3, 4, 5], 1, Some(5)),
                run(m, &[2, 3, 4], 2, Some(0)),
                run(m, &[], 2, Some(0)),
                run(m, &[2, 2, 3, 4], 2, None),
                run(m, &[2, 2, 3, 4], 0, Some(4)),
                run(m, &[2, 3], 2, Some(2)),
            ]
        },
    )
    .await;
    assert_eq!(out[0], [2, 5]);
    assert_eq!(out[1], [5]);
    assert_eq!(out[2], [2, 3], "an ineligible primary isn't put in");
    assert!(out[3].is_empty());
    assert_eq!(out[4], [2, 3], "duplicates go before the count");
    assert_eq!(out[5], [2, 3, 4], "uncapped, in order");
    assert_eq!(out[6], [2, 3]);
}
