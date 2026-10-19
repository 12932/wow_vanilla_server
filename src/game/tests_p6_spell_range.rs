//! Phase 3, round 6, against Thistle Tea's `range_test.exs` and
//! `destination_test.exs`: a spell's minimum range, a range modified to
//! nothing, running targets, and ground targets.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at};
use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_EX2_IGNORE_LOS, SpellEffect, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::{AURA_ADD_FLAT_MODIFIER, AURA_ADD_PCT_MODIFIER};
use crate::game::map::spells::{Targets, checks, fail, mods};

/// Ranges: 8 to 30 yards, and 30.
const RANGE_8_TO_30: u32 = 950;
const RANGE_30: u32 = 951;
/// A bolt of 8 to 30 yards; a heal of 30 at a friend; a ground spell of 8
/// to 30, and one that ignores line of sight.
const BOLT: u32 = 990_970;
const HEAL: u32 = 990_971;
const GROUND: u32 = 990_972;
const GROUND_LOS: u32 = 990_973;
/// A hundred yards less range; a fifth more.
const SHORTER: u32 = 990_974;
const WIDER: u32 = 990_975;
const BOLT_30: u32 = 990_976;

fn data(d: &mut GameData) {
    d.spells.ranges.insert(RANGE_8_TO_30, (8.0, 30.0));
    d.spells.ranges.insert(RANGE_30, (0.0, 30.0));
    let spell = |id: u32, range: u32, e: SpellEffect| {
        let mut s = bare(id, 2);
        s.range_index = range;
        s.family_name = 3;
        s.family_flags = 0x1;
        effect(&mut s, 0, e);
        s
    };
    let heal = SpellEffect {
        effect: 10,
        base_points: 9,
        base_dice: 1,
        target_a: 21,
        ..Default::default()
    };
    let ground = SpellEffect {
        target_a: 16,
        radius_index: 0,
        ..damage(10)
    };
    let mut ignores = spell(GROUND_LOS, RANGE_8_TO_30, ground);
    ignores.attributes[2] |= ATTR_EX2_IGNORE_LOS;
    let modifier = |id: u32, kind: u32, amount: i32| {
        let mut s = bare(id, 0);
        s.family_name = 3;
        s.duration_index = LASTS_LONG;
        effect(
            &mut s,
            0,
            SpellEffect {
                item_type: 0x1,
                ..aura(kind, mods::OP_RANGE as i32, amount, 1)
            },
        );
        s
    };
    let spells: Vec<SpellTemplate> = vec![
        spell(BOLT, RANGE_8_TO_30, damage(10)),
        spell(HEAL, RANGE_30, heal),
        spell(BOLT_30, RANGE_30, damage(10)),
        spell(GROUND, RANGE_8_TO_30, ground),
        ignores,
        modifier(SHORTER, AURA_ADD_FLAT_MODIFIER, -100),
        modifier(WIDER, AURA_ADD_PCT_MODIFIER, 20),
    ];
    insert(d, spells);
}

fn why(m: &mut Map, caster: Guid, spell: u32, t: Targets) -> Option<u8> {
    kit::fresh(m, caster);
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let mods = mods::for_spell(m, caster, &s);
    checks::check(m, caster, &s, &t, None, false, &mods)
}

/// `g` put `d` yards in front of `from`.
fn place(m: &mut Map, from: Guid, g: Guid, d: f32) {
    let me = m.unit_view(from).unwrap();
    let at = Vector3d {
        x: me.position.x + d * me.orientation.cos(),
        y: me.position.y + d * me.orientation.sin(),
        z: me.position.z,
    };
    if let Some(c) = m.creature_mut(g) {
        c.position = at;
    } else if let Some(p) = m.players.get_mut(&g) {
        p.movement.position = at;
    }
}

fn reaches(m: &Map, a: Guid, b: Guid) -> f32 {
    m.unit_view(a).unwrap().combat_reach + m.unit_view(b).unwrap().combat_reach
}

/// A spell's minimum range is beyond both units' reaches, as its maximum
/// is; a range modified below nothing is nothing, the reaches and a yard
/// left, not unlimited; a caster and a player target both running reach
/// mangos's leeway further, one standing doesn't (Thistle Tea's
/// `Range.validate_target` and movement allowance; mangos's
/// `GetMinMaxRange`).
#[tokio::test]
async fn the_minimum_is_beyond_the_reaches_and_running_reaches_further() {
    use wow_world_messages::vanilla::MovementInfo_MovementFlags as Flags;
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6RA", "Ranger", Class::Mage).await;
    let (_d, b, _) = human(&realm, "P6RB", "Runner", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[BOLT, BOLT_30, HEAL]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let r = reaches(m, a, kobold);
            let mut minimum = Vec::new();
            for d in [8.0 + r - 0.1, 8.0 + r + 0.1] {
                place(m, a, kobold, d);
                minimum.push(why(m, a, BOLT, at(kobold)));
            }
            kit::apply(m, a, a, SHORTER);
            let mut floored = Vec::new();
            for d in [r + 0.9, r + 1.1] {
                place(m, a, kobold, d);
                floored.push(why(m, a, BOLT_30, at(kobold)));
            }
            crate::game::map::auras::remove_spell(m, a, SHORTER, None);
            // The heal at a running friend, 2 yards beyond.
            let r = reaches(m, a, b);
            place(m, a, b, 30.0 + r + 1.0 + 2.0);
            let standing = why(m, a, HEAL, at(b));
            for g in [a, b] {
                m.players.get_mut(&g).unwrap().movement.flags = Flags::empty().set_forward();
            }
            let running = why(m, a, HEAL, at(b));
            m.players.get_mut(&b).unwrap().movement.flags = Flags::empty();
            let one_running = why(m, a, HEAL, at(b));
            m.players.get_mut(&a).unwrap().movement.flags = Flags::empty();
            (minimum, floored, standing, running, one_running)
        })
        .await
        .unwrap();
    let (minimum, floored, standing, running, one_running) = out;
    assert_eq!(
        minimum,
        [Some(fail::TOO_CLOSE), None],
        "8 yards past the reaches"
    );
    assert_eq!(
        floored,
        [None, Some(fail::OUT_OF_RANGE)],
        "the reaches and a yard"
    );
    let out_of_range = Some(fail::OUT_OF_RANGE);
    assert_eq!(
        (standing, running, one_running),
        (out_of_range, None, out_of_range),
        "both running"
    );
}

/// A ground spell's place: within its range, with its range modifiers and
/// mangos's two yards, the caster's reach no help; not nearer than its
/// minimum; in sight, unless the spell ignores sight. A spell at a unit
/// leaves a place sent with it to the unit's checks (Thistle Tea's
/// `Destination.validate`; mangos's `CheckRange` for a destination).
#[tokio::test]
async fn a_ground_spell_wants_its_place_in_range_and_in_sight() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6RC", "Grounded", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[BOLT, GROUND, GROUND_LOS]);
            let ahead = |m: &Map, d: f32| {
                let me = m.unit_view(a).unwrap();
                Vector3d {
                    x: me.position.x + d * me.orientation.cos(),
                    y: me.position.y + d * me.orientation.sin(),
                    z: me.position.z,
                }
            };
            let there = |m: &Map, d: f32| Targets {
                dest: Some(ahead(m, d)),
                ..Default::default()
            };
            let seen: Vec<Option<u8>> = [5.0, 20.0, 31.9, 32.1]
                .into_iter()
                .map(|d| {
                    let t = there(m, d);
                    why(m, a, GROUND, t)
                })
                .collect();
            kit::apply(m, a, a, WIDER);
            if let Some(f) = m.unit_fields_mut(a) {
                f.set_f32(crate::game::fields::UNIT_FIELD_COMBATREACH, 100.0);
            }
            let wider: Vec<Option<u8>> = [37.9, 38.1]
                .into_iter()
                .map(|d| {
                    let t = there(m, d);
                    why(m, a, GROUND, t)
                })
                .collect();
            if let Some(f) = m.unit_fields_mut(a) {
                f.set_f32(crate::game::fields::UNIT_FIELD_COMBATREACH, 1.5);
            }
            // A wall across the way, ten yards on.
            let (l, r) = {
                let me = m.unit_view(a).unwrap();
                let side = me.orientation + std::f32::consts::FRAC_PI_2;
                let mid = ahead(m, 10.0);
                let off = |k: f32| Vector3d {
                    x: mid.x + k * side.cos(),
                    y: mid.y + k * side.sin(),
                    z: mid.z,
                };
                (off(-5.0), off(5.0))
            };
            m.spells.walls.push((l, r));
            let t = there(m, 20.0);
            let walled = (why(m, a, GROUND, t), why(m, a, GROUND_LOS, t));
            m.spells.walls.clear();
            let kobold = kit::dummy(m, a, 60, 20.0);
            let with_place = Targets {
                unit: Some(kobold),
                dest: Some(ahead(m, 200.0)),
                ..Default::default()
            };
            (seen, wider, walled, why(m, a, BOLT, with_place))
        })
        .await
        .unwrap();
    let (seen, wider, walled, with_place) = out;
    let out_of_range = Some(fail::OUT_OF_RANGE);
    assert_eq!(
        seen,
        [Some(fail::TOO_CLOSE), None, None, out_of_range],
        "too close, in range, two yards over, beyond"
    );
    assert_eq!(wider, [None, out_of_range], "a fifth more, no reach");
    assert_eq!(walled, (Some(fail::LINE_OF_SIGHT), None), "a wall between");
    assert_eq!(with_place, None, "the unit's checks");
}

/// Ten-yard spells: a nova about the caster, one of another family mask,
/// a blizzard on the ground; five yards and half again more radius, one
/// charge of it; twenty yards less.
const NOVA: u32 = 990_980;
const OTHER_NOVA: u32 = 990_981;
const BLIZZARD: u32 = 990_982;
const BROADER: u32 = 990_983;
const NARROWER: u32 = 990_984;
const RADIUS_10: u32 = 950;

fn radii(d: &mut GameData) {
    use crate::game::map::auras::AURA_PERIODIC_DAMAGE;
    data(d);
    d.spells.radii.insert(RADIUS_10, 10.0);
    let area = |id: u32, flags: u64, e: SpellEffect| {
        let mut s = bare(id, 4);
        s.family_name = 11;
        s.family_flags = flags;
        s.range_index = RANGE_30;
        s.duration_index = LASTS_5S;
        effect(
            &mut s,
            0,
            SpellEffect {
                radius_index: RADIUS_10,
                ..e
            },
        );
        s
    };
    let nova = SpellEffect {
        target_a: 15,
        ..damage(10)
    };
    let blizzard = SpellEffect {
        effect: crate::game::map::spells::EFFECT_PERSISTENT_AREA_AURA,
        amplitude: 1_000,
        ..aura(AURA_PERIODIC_DAMAGE, 0, 10, 16)
    };
    let modifier = |id: u32, effects: &[(u32, i32)], charges: u32| {
        let mut s = bare(id, 0);
        s.family_name = 11;
        s.duration_index = LASTS_LONG;
        s.proc_charges = charges;
        for (i, (kind, amount)) in effects.iter().enumerate() {
            effect(
                &mut s,
                i,
                SpellEffect {
                    item_type: 0x1,
                    ..aura(*kind, mods::OP_RADIUS as i32, *amount, 1)
                },
            );
        }
        s
    };
    let spells: Vec<SpellTemplate> = vec![
        area(NOVA, 0x1, nova),
        area(OTHER_NOVA, 0x2, nova),
        area(BLIZZARD, 0x1, blizzard),
        modifier(
            BROADER,
            &[(AURA_ADD_FLAT_MODIFIER, 5), (AURA_ADD_PCT_MODIFIER, 50)],
            1,
        ),
        modifier(NARROWER, &[(AURA_ADD_FLAT_MODIFIER, -20)], 0),
    ];
    insert(d, spells);
}

/// An area's radius takes its family's flat then percent modifiers, ten
/// yards to 22.5, and another mask's spell's doesn't; a radius cut below
/// nothing is nothing; a ground spell keeps the radius its charged
/// modifier gave it once the charge is spent (Thistle Tea's `Radius` and
/// the area's snapshot; mangos's `SPELLMOD_RADIUS`).
#[tokio::test]
async fn a_radius_takes_its_modifiers_and_keeps_them_past_their_charge() {
    let realm = fast_realm(radii);
    let (_c, a, map) = human(&realm, "P6RD", "Wide", Class::Mage).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[NOVA, OTHER_NOVA, BLIZZARD]);
            let kobold = kit::dummy(m, a, 60, 20.0);
            let struck = |m: &mut Map, spell: u32| {
                kit::set_health(m, kobold, 1_000_000);
                kit::cast_fresh(m, a, spell, Targets::default());
                kit::health(m, kobold) < 1_000_000
            };
            let plain = struck(m, NOVA);
            kit::apply(m, a, a, BROADER);
            let other = struck(m, OTHER_NOVA);
            let broadened = struck(m, NOVA);
            let spent = kit::has(m, a, BROADER);
            kit::apply(m, a, a, BROADER);
            let ground = Targets {
                dest: m.position_of(kobold),
                ..Default::default()
            };
            kit::cast_fresh(m, a, BLIZZARD, ground);
            let area = m
                .spells
                .areas
                .iter()
                .find(|x| x.spell == BLIZZARD)
                .map(|x| x.radius);
            let blizzard_spent = kit::has(m, a, BROADER);
            kit::apply(m, a, a, NARROWER);
            let s = m.realm.data.spells.get(NOVA).cloned().unwrap();
            let narrowed =
                crate::game::map::spells::targets::radius(m, &s, 0, &mods::for_spell(m, a, &s));
            (
                plain,
                other,
                broadened,
                spent,
                area,
                blizzard_spent,
                narrowed,
            )
        })
        .await
        .unwrap();
    let (plain, other, broadened, spent, area, blizzard_spent, narrowed) = out;
    assert!(!plain, "ten yards don't reach twenty");
    assert!(!other, "another mask's");
    assert!(broadened, "22.5 yards do");
    assert!(!spent, "its charge spent");
    assert_eq!(area, Some(22.5), "the blizzard's");
    assert!(!blizzard_spent);
    assert_eq!(narrowed, 0.0);
}
