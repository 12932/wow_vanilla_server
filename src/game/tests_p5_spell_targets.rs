//! Phase 3, round 5, against Thistle Tea's `spell_target_test.exs`: whom
//! each of a spell's implicit targets picks (`spells::targets`), with
//! enemies placed about the caster and the unit picked, a party member
//! and a stranger.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, has};
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::AURA_DUMMY;
use crate::game::map::spells::{self, Targets, mods::Mods, outcome, targets};

const TEN_YARDS: u32 = 900;
const AREA_AURA: u32 = 990_400;

fn kit(d: &mut GameData) {
    d.spells.radii.insert(TEN_YARDS, 10.0);
    d.spells.ranges.insert(1, (0.0, 40.0));
    let mut area = bare(AREA_AURA, 0);
    area.duration_index = LASTS_15S;
    effect(
        &mut area,
        0,
        SpellEffect {
            effect: 27,
            aura: AURA_DUMMY,
            target_a: 28,
            radius_index: TEN_YARDS,
            amplitude: 1_000,
            ..Default::default()
        },
    );
    insert(d, [area]);
}

/// A spell of one effect at `a` and `b`, ten yards about.
fn at(a: u32, b: u32) -> SpellTemplate {
    let mut s = bare(990_499, 0);
    effect(
        &mut s,
        0,
        SpellEffect {
            effect: 2,
            base_points: 1,
            target_a: a,
            target_b: b,
            radius_index: TEN_YARDS,
            ..Default::default()
        },
    );
    s
}

fn picked(m: &Map, caster: Guid, s: &SpellTemplate, t: &Targets) -> Vec<Guid> {
    let mut v = targets::effect_targets(m, caster, s, 0, t, &Mods::default());
    v.sort();
    v
}

fn sorted(mut v: Vec<Guid>) -> Vec<Guid> {
    v.sort();
    v
}

/// The areas: at the destination, or the unit picked when there is none
/// (16, 28; mangos's `setUnitTarget` makes the unit's place the
/// destination); about the caster whatever is picked (15), and in a cone
/// before them (24); about the caster's own place even with a destination
/// (18 with 16, a triggered Hellfire Effect); a unit target, that unit
/// alone (Thistle Tea's `aim_at_unit/3` and `target_query/2`).
#[tokio::test]
async fn areas_are_about_the_destination_the_unit_picked_or_the_caster() {
    let realm = fast_realm(kit);
    let (_a, a, map) = human(&realm, "P5GA", "Aimer", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let near = kit::dummy(m, a, 60, 5.0);
            let far = kit::dummy(m, a, 60, 25.0);
            let behind = kit::dummy(m, a, 60, -5.0);
            let far_at = m.position_of(far);
            let unit = Targets {
                unit: Some(far),
                ..Default::default()
            };
            // As a triggered cast aims: the unit and its place.
            let triggered = Targets {
                unit: Some(far),
                dest: far_at,
                ..Default::default()
            };
            let at_caster = Targets {
                dest: m.position_of(a),
                ..Default::default()
            };
            (
                (near, far, behind),
                [
                    picked(m, a, &at(16, 0), &unit),
                    picked(m, a, &at(28, 0), &unit),
                    picked(m, a, &at(16, 0), &at_caster),
                    picked(m, a, &at(15, 0), &unit),
                    picked(m, a, &at(24, 0), &unit),
                    picked(m, a, &at(18, 16), &triggered),
                    picked(m, a, &at(6, 0), &unit),
                ],
            )
        })
        .await
        .unwrap();
    let ((near, far, behind), got) = out;
    assert_eq!(got[0], [far], "about the unit picked");
    assert_eq!(got[1], [far], "a channel's, the same");
    assert_eq!(got[2], sorted(vec![near, behind]), "about a destination");
    assert_eq!(got[3], sorted(vec![near, behind]), "about the caster");
    assert_eq!(got[4], [near], "the cone before them");
    assert_eq!(got[5], sorted(vec![near, behind]), "the caster's own place");
    assert_eq!(got[6], [far], "the unit alone");
    let area = |a: u32, b: u32| outcome::area(&at(a, b));
    assert!(area(15, 0) && area(16, 0) && area(24, 0) && area(18, 16));
    assert!(!area(6, 0) && !area(1, 0), "a unit's or the caster's");
}

/// The groups: the caster's party about them whatever is picked (20), the
/// raid about them (56), the picked member's party (37), and a party
/// member's spell (35) only at a member (Thistle Tea's party, raid and
/// party-member `target_query/2` tests).
#[tokio::test]
async fn group_spells_take_the_party_or_raid_and_a_member_spell_only_a_member() {
    let realm = fast_realm(kit);
    let (_a, a, map) = human(&realm, "P5GB", "Leader", Class::Warrior).await;
    let (_b, b, _) = human(&realm, "P5GC", "Member", Class::Warrior).await;
    let (_c, c, _) = human(&realm, "P5GD", "Stranger", Class::Warrior).await;
    let lo = realm.registry.online(a).unwrap();
    let bo = realm.registry.online(b).unwrap();
    assert!(crate::game::social::groups::join_directly(&realm, &lo, &bo));
    let out = map
        .query(move |m| {
            let kobold = kit::dummy(m, a, 60, 5.0);
            let unit = |g| Targets {
                unit: Some(g),
                ..Default::default()
            };
            [
                picked(m, a, &at(20, 0), &unit(kobold)),
                picked(m, a, &at(20, 0), &Targets::default()),
                picked(m, a, &at(56, 0), &unit(c)),
                picked(m, a, &at(37, 0), &unit(b)),
                picked(m, a, &at(35, 0), &unit(b)),
                picked(m, a, &at(35, 0), &unit(c)),
                picked(m, a, &at(35, 0), &Targets::default()),
            ]
        })
        .await
        .unwrap();
    let party = sorted(vec![a, b]);
    assert_eq!(out[0], party, "the party, not the kobold picked");
    assert_eq!(out[1], party, "nothing picked");
    assert_eq!(
        out[2],
        [b],
        "the raid, not the stranger picked, nor for harm its caster"
    );
    assert_eq!(out[3], party, "the member's party");
    assert_eq!(out[4], [b], "a member");
    assert!(out[5].is_empty(), "not a stranger");
    assert!(out[6].is_empty(), "nor no one");
}

/// A persistent area lands on no one at the cast: it is placed, and its
/// ticks are left to put its aura on (Thistle Tea's ground-only aura effects and
/// `spell_effect_test`'s persistent areas).
#[tokio::test]
async fn a_persistent_area_lands_on_no_one_at_the_cast() {
    let realm = fast_realm(kit);
    let (_a, a, map) = human(&realm, "P5GE", "Grounder", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let at = m.position_of(kobold).unwrap();
            spells::cast_triggered_at(m, a, AREA_AURA, at);
            let at_once = has(m, kobold, AREA_AURA);
            let placed = m.spells.areas.iter().any(|x| x.spell == AREA_AURA);
            (at_once, placed)
        })
        .await
        .unwrap();
    assert_eq!(out, (false, true));
}

const TRIG_ENEMY: u32 = 990_410;
const TRIG_FRIEND: u32 = 990_411;
const PULSE: u32 = 990_412;
const PULSE_FRIEND: u32 = 990_413;
const PULSE_CHANNEL: u32 = 990_414;

fn pulses(d: &mut GameData) {
    kit(d);
    // A mark at an enemy, and one at a friend.
    let mark = |id: u32, target: u32| {
        let mut s = bare(id, 0);
        s.duration_index = LASTS_LONG;
        effect(&mut s, 0, aura(AURA_DUMMY, 0, 0, target));
        s
    };
    let pulse = |id: u32, trigger: u32| {
        let mut s = bare(id, 0);
        s.duration_index = LASTS_LONG;
        effect(
            &mut s,
            0,
            SpellEffect {
                trigger_spell: trigger,
                amplitude: 1_000,
                ..aura(
                    crate::game::map::auras::AURA_PERIODIC_TRIGGER_SPELL,
                    0,
                    0,
                    1,
                )
            },
        );
        s
    };
    let mut channel = pulse(PULSE_CHANNEL, TRIG_ENEMY);
    channel.attributes[1] |= crate::game::data::spells::ATTR_EX_CHANNELED;
    channel.duration_index = LASTS_15S;
    insert(
        d,
        [
            mark(TRIG_ENEMY, 6),
            mark(TRIG_FRIEND, 21),
            pulse(PULSE, TRIG_ENEMY),
            pulse(PULSE_FRIEND, TRIG_FRIEND),
            channel,
        ],
    );
}

/// A periodic trigger on its own caster: a spell at an enemy goes at the
/// channel's target, else the unit the caster has targeted, else nowhere
/// (not at the caster); one at a friend stays on the caster; on another
/// unit, at that unit (Thistle Tea's `redirect_trigger_target/3`).
#[tokio::test]
async fn a_periodic_trigger_aims_at_the_channels_target_the_target_or_nothing() {
    use crate::game::fields::UNIT_FIELD_TARGET;
    let realm = fast_realm(pulses);
    let (_a, a, map) = human(&realm, "P5GF", "Pulser", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[PULSE_CHANNEL]);
            let one = kit::dummy(m, a, 60, 5.0);
            let two = kit::dummy(m, a, 60, 8.0);
            let marks = |m: &Map| {
                (
                    has(m, a, TRIG_ENEMY),
                    has(m, one, TRIG_ENEMY),
                    has(m, two, TRIG_ENEMY),
                )
            };
            let clear = |m: &mut Map| {
                for g in [a, one, two] {
                    crate::game::map::auras::remove_where(m, g, |h| {
                        [TRIG_ENEMY, TRIG_FRIEND].contains(&h.spell)
                    });
                }
            };
            // Nothing targeted: nowhere.
            kit::apply(m, a, a, PULSE);
            kit::tick_now(m, a, PULSE);
            let alone = marks(m);
            clear(m);
            // The unit targeted.
            if let Some(f) = m.unit_fields_mut(a) {
                f.set_guid(UNIT_FIELD_TARGET, two);
            }
            kit::tick_now(m, a, PULSE);
            let targeted = marks(m);
            clear(m);
            crate::game::map::auras::remove_spell(m, a, PULSE, None);
            // A channel at `one`, while `two` is targeted.
            let began = kit::cast_fresh(m, a, PULSE_CHANNEL, kit::at(one));
            kit::tick_now(m, a, PULSE_CHANNEL);
            let channelled = marks(m);
            clear(m);
            crate::game::map::spells::interrupt(m, a);
            // At a friend: on the caster.
            kit::apply(m, a, a, PULSE_FRIEND);
            kit::tick_now(m, a, PULSE_FRIEND);
            let friend = has(m, a, TRIG_FRIEND);
            // On another unit: at it.
            kit::apply(m, a, one, PULSE);
            kit::tick_now(m, one, PULSE);
            let other = marks(m);
            (alone, targeted, began, channelled, friend, other)
        })
        .await
        .unwrap();
    let (alone, targeted, began, channelled, friend, other) = out;
    assert_eq!(alone, (false, false, false), "nowhere, not the caster");
    assert_eq!(targeted, (false, false, true), "the unit targeted");
    assert!(began);
    assert_eq!(channelled, (false, true, false), "the channel's target");
    assert!(friend, "a friend's on the caster");
    assert_eq!(other, (false, true, false), "the unit it is on");
}
