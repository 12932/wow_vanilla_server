//! Phase 3, round 5, against Thistle Tea's `target_trigger_test.exs`: a
//! target-trigger aura (Relentless Strikes, Improved Scorch) rolls once a
//! landed impact of its family and mask, by its chance and the finisher's
//! combo points, and casts its spell at the one hit.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, fresh, has, pass, power, set_power};
use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_EX_CHANNELED, ATTR_EX_REQ_COMBO_POINTS, SpellEffect};
use crate::game::map::auras::{self, AURA_ADD_TARGET_TRIGGER, AURA_DUMMY, AURA_PERIODIC_DAMAGE};
use crate::game::map::{Map, spells};

/// The talent: family 8, a trigger of ENERGIZE for mask 1 at 20% a combo
/// point, and of MARK for mask 2 (bit 33 too) always.
const TALENT: u32 = 990_200;
const ENERGIZE: u32 = 990_201;
const MARK: u32 = 990_202;
/// A finisher of mask 1; a bolt of mask 2; one of bit 33 alone; a
/// two-effect strike; a channel of mask 1; one of another family.
const FINISHER: u32 = 990_203;
const BOLT: u32 = 990_204;
const HIGH: u32 = 990_205;
const DOUBLE: u32 = 990_206;
const CHANNEL: u32 = 990_207;
const OTHER_FAMILY: u32 = 990_208;
/// Immunity and reflection, for the kobold.
const IMMUNE: u32 = 990_209;
const REFLECT: u32 = 990_210;
/// A talent of its owner alone (Relentless Strikes' attribute), always,
/// for mask 4; a strike of mask 4 with a dummy at its caster too
/// (Eviscerate's), and one without.
const OWNER_ONLY: u32 = 990_211;
const WITH_DUMMY: u32 = 990_212;
const WITHOUT_DUMMY: u32 = 990_213;

fn kit(d: &mut GameData) {
    let mut talent = bare(TALENT, 0);
    talent.family_name = 8;
    talent.duration_index = LASTS_LONG;
    effect(
        &mut talent,
        0,
        SpellEffect {
            trigger_spell: ENERGIZE,
            item_type: 1,
            points_per_combo_point: 20.0,
            ..aura(AURA_ADD_TARGET_TRIGGER, 0, 0, 1)
        },
    );
    effect(
        &mut talent,
        1,
        SpellEffect {
            trigger_spell: MARK,
            item_type: 2 | (1 << 33),
            ..aura(AURA_ADD_TARGET_TRIGGER, 0, 100, 1)
        },
    );
    let mut energize = bare(ENERGIZE, 0);
    effect(
        &mut energize,
        0,
        SpellEffect {
            effect: 30,
            misc_value: 3,
            base_points: 24,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        },
    );
    let mut mark = bare(MARK, 0);
    mark.duration_index = LASTS_LONG;
    effect(&mut mark, 0, aura(AURA_DUMMY, 0, 0, 25));
    let rogue = |id: u32, flags: u64| {
        let mut s = bare(id, 0);
        s.family_name = 8;
        s.family_flags = flags;
        s.dmg_class = 2;
        effect(&mut s, 0, damage(10));
        s
    };
    let mut finisher = rogue(FINISHER, 1);
    finisher.attributes[1] |= ATTR_EX_REQ_COMBO_POINTS;
    let mut bolt = rogue(BOLT, 2);
    bolt.dmg_class = 1;
    bolt.school = 5;
    let high = rogue(HIGH, 1 << 33);
    let mut double = rogue(DOUBLE, 1);
    double.attributes[1] |= ATTR_EX_REQ_COMBO_POINTS;
    effect(&mut double, 1, damage(10));
    let mut channel = rogue(CHANNEL, 2);
    channel.dmg_class = 1;
    channel.school = 5;
    channel.attributes[1] |= ATTR_EX_CHANNELED;
    channel.duration_index = LASTS_5S;
    effect(
        &mut channel,
        0,
        SpellEffect {
            amplitude: 1_000,
            ..aura(AURA_PERIODIC_DAMAGE, 0, 5, 6)
        },
    );
    let mut other = rogue(OTHER_FAMILY, 1 | 2);
    other.family_name = 3;
    let mut immune = bare(IMMUNE, 0);
    immune.duration_index = LASTS_LONG;
    effect(&mut immune, 0, aura(auras::AURA_SCHOOL_IMMUNITY, 127, 0, 1));
    let mut reflect = bare(REFLECT, 0);
    reflect.duration_index = LASTS_LONG;
    effect(&mut reflect, 0, aura(auras::AURA_REFLECT_SPELLS, 0, 100, 1));
    let mut owner_only = bare(OWNER_ONLY, 0);
    owner_only.family_name = 8;
    owner_only.duration_index = LASTS_LONG;
    owner_only.attributes[4] |= crate::game::data::spells::ATTR_EX4_CLASS_TRIGGER_ONLY_ON_TARGET;
    effect(
        &mut owner_only,
        0,
        SpellEffect {
            trigger_spell: ENERGIZE,
            item_type: 4,
            ..aura(AURA_ADD_TARGET_TRIGGER, 0, 100, 1)
        },
    );
    let mut with_dummy = rogue(WITH_DUMMY, 4);
    effect(
        &mut with_dummy,
        1,
        SpellEffect {
            effect: 3,
            target_a: 1,
            ..Default::default()
        },
    );
    let without_dummy = rogue(WITHOUT_DUMMY, 4);
    insert(d, [owner_only, with_dummy, without_dummy]);
    insert(
        d,
        [
            talent, energize, mark, finisher, bolt, high, double, channel, other, immune, reflect,
        ],
    );
}

async fn rogue(
    account: &str,
    name: &str,
) -> (
    std::sync::Arc<crate::game::Realm>,
    Guid,
    Guid,
    crate::game::map::MapHandle,
    crate::game::harness::Client,
) {
    let realm = fast_realm(kit);
    let (c, g, map) = human(&realm, account, name, Class::Rogue).await;
    let kobold = map
        .query(move |m| {
            kit::ready(
                m,
                g,
                60,
                &[FINISHER, BOLT, HIGH, DOUBLE, CHANNEL, OTHER_FAMILY],
            );
            kit::apply(m, g, g, TALENT);
            kit::dummy(m, g, 60, 5.0)
        })
        .await
        .unwrap();
    (realm, g, kobold, map, c)
}

/// Energy from `n` casts of `spell` at `kobold`, each with `points`:
/// how many gave it.
fn energized(m: &mut Map, g: Guid, kobold: Guid, spell: u32, points: u8, n: usize) -> usize {
    (0..n)
        .filter(|_| {
            fresh(m, g);
            if points > 0 {
                spells::add_combo_points(m, g, kobold, points);
            }
            set_power(m, g, 3, 0);
            if spell == FINISHER {
                cast_fresh(m, g, spell, at(kobold));
            } else {
                spells::cast_triggered(m, g, spell, Some(kobold));
            }
            power(m, g, 3) > 0
        })
        .count()
}

/// Relentless Strikes' kind: the finisher's own combo points, spent by
/// the cast, give the chance (five, always; one, a fifth of the time);
/// both words of the family mask match, another family's spells don't; a
/// talent of its owner alone rolls only where the cast lands on its owner
/// (Thistle Tea's `snapshot/2` and `events/3`, mangos's
/// `CanExecuteTriggersOnHit`).
#[tokio::test]
async fn a_target_trigger_rolls_by_the_finishers_points_and_its_family_mask() {
    let (_realm, g, kobold, map, _c) = rogue("P5TA", "Relentless").await;
    let out = map
        .query(move |m| {
            let five = energized(m, g, kobold, FINISHER, 5, 20);
            let spent = m.caster(g).map(|c| c.combo_points);
            let one = energized(m, g, kobold, FINISHER, 1, 300);
            let mut marks = Vec::new();
            for spell in [BOLT, HIGH, OTHER_FAMILY, FINISHER] {
                auras::remove_spell(m, kobold, MARK, None);
                fresh(m, g);
                spells::cast_triggered(m, g, spell, Some(kobold));
                marks.push(has(m, kobold, MARK));
            }
            let other = energized(m, g, kobold, OTHER_FAMILY, 0, 10);
            // Its owner's alone: once, for the dummy on the rogue; none
            // without it.
            auras::remove_spell(m, g, TALENT, None);
            kit::apply(m, g, g, OWNER_ONLY);
            let owner = [WITH_DUMMY, WITHOUT_DUMMY].map(|s| {
                set_power(m, g, 3, 0);
                fresh(m, g);
                spells::cast_triggered(m, g, s, Some(kobold));
                power(m, g, 3)
            });
            (five, spent, one, marks, other, owner)
        })
        .await
        .unwrap();
    let (five, spent, one, marks, other, owner) = out;
    assert_eq!(five, 20, "five points: every time");
    assert_eq!(spent, Some(0), "the points are spent");
    assert!((30..=95).contains(&one), "one point, a fifth of 300: {one}");
    assert_eq!(
        marks,
        [true, true, false, false],
        "mask 2, bit 33; not another family nor mask 1"
    );
    assert_eq!(other, 0, "another family's spell");
    assert_eq!(owner, [25, 0], "its owner's alone: once, at the rogue");
}

/// One impact of two effects triggers once; an immune or a dead target
/// triggers nothing; a killing blow still does; a reflected spell triggers
/// at the one it lands on, its caster; a triggered spell's hit takes part
/// (Thistle Tea's `receive/4` tests).
#[tokio::test]
async fn a_target_trigger_rolls_once_a_landed_impact() {
    let (_realm, g, kobold, map, _c) = rogue("P5TB", "Relentlest").await;
    let out = map
        .query(move |m| {
            // Count by stacks of energy: two triggers would give fifty.
            fresh(m, g);
            spells::add_combo_points(m, g, kobold, 5);
            set_power(m, g, 3, 0);
            spells::cast_triggered(m, g, DOUBLE, Some(kobold));
            let double = power(m, g, 3);
            // Immune.
            kit::apply(m, kobold, kobold, IMMUNE);
            auras::remove_spell(m, kobold, MARK, None);
            spells::cast_triggered(m, g, BOLT, Some(kobold));
            let immune = has(m, kobold, MARK);
            auras::remove_spell(m, kobold, IMMUNE, None);
            // Reflected: the mark on the rogue, not the kobold.
            kit::apply(m, kobold, kobold, REFLECT);
            spells::cast_triggered(m, g, BOLT, Some(kobold));
            let reflected = (has(m, kobold, MARK), has(m, g, MARK));
            auras::remove_spell(m, kobold, REFLECT, None);
            auras::remove_spell(m, g, MARK, None);
            // A killing finisher still gives its energy.
            kit::set_health(m, kobold, 1);
            spells::add_combo_points(m, g, kobold, 5);
            set_power(m, g, 3, 0);
            spells::cast_triggered(m, g, FINISHER, Some(kobold));
            let killed = (m.unit_view(kobold).is_some_and(|u| u.alive), power(m, g, 3));
            // Already dead.
            auras::remove_spell(m, kobold, MARK, None);
            spells::cast_triggered(m, g, BOLT, Some(kobold));
            let dead = has(m, kobold, MARK);
            (double, immune, reflected, killed, dead)
        })
        .await
        .unwrap();
    let (double, immune, reflected, killed, dead) = out;
    assert_eq!(double, 25, "once for the impact, not each effect");
    assert!(!immune, "not at an immune target");
    assert_eq!(reflected, (false, true), "at the one it lands on");
    assert_eq!(killed, (false, 25), "a killing blow triggers");
    assert!(!dead, "a dead target doesn't");
}

/// A channel of the family triggers at its first impact and not at its
/// ticks (Thistle Tea's `start/5`); its five seconds of ticks did hurt.
#[tokio::test]
async fn a_channel_triggers_at_its_start_and_not_its_ticks() {
    let (_realm, g, kobold, map, _c) = rogue("P5TC", "Channeler").await;
    let out = map
        .query(move |m| {
            let began = cast_fresh(m, g, CHANNEL, at(kobold));
            let start = has(m, kobold, MARK);
            auras::remove_spell(m, kobold, MARK, None);
            let mut ticked = false;
            for _ in 0..6 {
                pass(m, 1_000);
                ticked |= has(m, kobold, MARK);
            }
            let still = auras::auras(m, kobold).is_some_and(|a| a.has_spell(CHANNEL));
            (began, start, ticked, still)
        })
        .await
        .unwrap();
    let (began, start, later, still) = out;
    assert!(began);
    assert!(start, "at its start");
    assert!(!later, "none from its ticks");
    assert!(!still, "the channel ran out");
}
