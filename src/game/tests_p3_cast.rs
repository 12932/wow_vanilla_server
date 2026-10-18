//! Phase 3's casting formulas, as Thistle Tea's tests of them have them:
//! the power coefficient, the magic hit and its resistances, the binary
//! spells, the frozen and Shatter, on made-up spells and the fast suite.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{apply_template, dummy, ready, set_health};
use crate::game::data::spells::{ATTR_EX_CHANNELED, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::*;
use crate::game::map::spells::mods::Mods;
use crate::game::map::spells::outcome::{self, Kind};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

/// The power coefficient of made-up spells: the template's own first, then
/// the cast time's share of 3.5 seconds (a damage over time's by its ticks,
/// a channel's by its duration), halved for areas and leeches, and five
/// percent less for each slow (Thistle Tea's `Coefficient.value`).
#[tokio::test]
async fn a_spells_coefficient_is_its_templates_or_its_cast_times() {
    let mut own = bare(990_100, 2);
    effect(
        &mut own,
        0,
        SpellEffect {
            bonus_coefficient: 0.8,
            ..damage(100)
        },
    );
    own.casting_time_index = CAST_3500;
    let mut dot = bare(990_101, 5);
    dot.spell_level = 30;
    dot.duration_index = LASTS_15S;
    effect(
        &mut dot,
        0,
        SpellEffect {
            amplitude: 3_000,
            ..aura(AURA_PERIODIC_DAMAGE, 0, 50, 6)
        },
    );
    let mut hybrid = dot.clone();
    hybrid.id = 990_102;
    hybrid.casting_time_index = CAST_2S;
    let periodic = hybrid.effects[0];
    effect(&mut hybrid, 0, damage(100));
    effect(&mut hybrid, 1, periodic);
    let mut area = bare(990_103, 2);
    area.spell_level = 30;
    area.casting_time_index = CAST_3500;
    effect(
        &mut area,
        0,
        SpellEffect {
            target_a: 15,
            ..damage(100)
        },
    );
    let mut leech = bare(990_104, 5);
    leech.spell_level = 30;
    leech.casting_time_index = CAST_3500;
    effect(
        &mut leech,
        0,
        SpellEffect {
            effect: 9,
            ..damage(100)
        },
    );
    let mut slowing = bare(990_105, 4);
    slowing.spell_level = 30;
    slowing.casting_time_index = CAST_3500;
    effect(&mut slowing, 0, damage(100));
    effect(&mut slowing, 1, aura(AURA_MOD_DECREASE_SPEED, 0, -50, 6));
    let mut channel = bare(990_106, 5);
    channel.spell_level = 30;
    channel.duration_index = LASTS_5S;
    channel.attributes[1] |= ATTR_EX_CHANNELED;
    effect(
        &mut channel,
        0,
        SpellEffect {
            amplitude: 1_000,
            ..aura(AURA_PERIODIC_DAMAGE, 0, 50, 6)
        },
    );
    let mut low = bare(990_107, 2);
    low.spell_level = 10;
    low.casting_time_index = CAST_3500;
    effect(&mut low, 0, damage(100));
    let mut long = bare(990_108, 2);
    long.spell_level = 30;
    long.casting_time_index = CAST_10S;
    effect(&mut long, 0, damage(100));
    let spells = [own, dot, hybrid, area, leech, slowing, channel, low, long];
    let all = spells.clone();
    let c = on_map(
        move |d| insert(d, all),
        move |m| {
            let c = |s: &SpellTemplate, i: usize, k: Kind| outcome::coefficient(m, s, i, k);
            [
                c(&spells[0], 0, Kind::Direct),
                c(&spells[1], 0, Kind::Dot),
                c(&spells[2], 0, Kind::Direct),
                c(&spells[2], 1, Kind::Dot),
                c(&spells[3], 0, Kind::Direct),
                c(&spells[4], 0, Kind::Direct),
                c(&spells[5], 0, Kind::Direct),
                c(&spells[6], 0, Kind::Dot),
                c(&spells[7], 0, Kind::Direct),
                c(&spells[8], 0, Kind::Direct),
            ]
        },
    )
    .await;
    assert!(close(c[0], 0.8), "the template's own: {}", c[0]);
    assert!(
        close(c[1], 0.2),
        "a damage over time's, over its five ticks: {}",
        c[1]
    );
    assert!(
        c[2] > 0.0 && c[2] < 2_000.0 / 3_500.0,
        "a hybrid's direct share: {}",
        c[2]
    );
    assert!(
        c[3] > 0.0 && c[3] < 0.2,
        "and its share over time: {}",
        c[3]
    );
    assert!(close(c[4], 0.5), "an area's half: {}", c[4]);
    assert!(close(c[5], 0.5), "a leech's half: {}", c[5]);
    assert!(close(c[6], 0.95), "a slow's five percent off: {}", c[6]);
    assert!(
        close(c[7], 5_000.0 / 3_500.0 / 5.0),
        "a channel's by its duration: {}",
        c[7]
    );
    assert!(close(c[8], 1.0 - 10.0 * 0.0375), "below level 20: {}", c[8]);
    assert!(
        close(c[9], 2.0),
        "a cast time held to seven seconds: {}",
        c[9]
    );
}

/// The magic hit chance, in hundredths of a percent: the level's, the
/// caster's hit, the target's mechanic resistance, then a binary spell's
/// resistance, held to 1% and 99% (Thistle Tea's `magic_hit_chance_bp`).
#[test]
fn the_magic_hit_chance_takes_hit_mechanic_and_binary_resistance_in_order() {
    let res = |r: i32| outcome::resist_chance(r, 60, false, 0);
    let h = outcome::magic_hit_chance;
    assert_eq!(h(60, 60, false, 0.0, 0.0, res(100)), 7_200);
    assert_eq!(h(60, 60, false, 0.0, 0.0, res(300)), 2_400);
    assert_eq!(h(60, 60, false, 10.0, 25.0, res(300)), 2_025);
    assert_eq!(h(60, 1, false, 0.0, 0.0, res(300)), 3_875);
    assert_eq!(h(60, 63, false, 0.0, 0.0, 0.0), 8_300);
    assert_eq!(h(60, 63, true, 0.0, 0.0, 0.0), 8_700);
    assert_eq!(h(60, 60, false, -100.0, 0.0, res(300)), 100);
    // A mechanic's resistance before the caps.
    assert_eq!(h(20, 20, true, 0.0, 25.0, 0.0), 7_100);
    assert_eq!(h(20, 20, true, 0.0, 100.0, 0.0), 100);
    assert_eq!(h(20, 20, true, 10.0, 25.0, 0.0), 8_100);
}

/// A partial resist's share: 75% at the top of the table, and a tick's
/// chance a tenth of a hit's (Thistle Tea's `resist_fraction` and
/// `resisted_amount`).
#[test]
fn a_partial_resist_takes_its_share_and_a_tick_a_tenth_of_the_chance() {
    let chance = outcome::resist_chance(300, 60, false, 0) * 100.0;
    assert!(close(chance, 75.0), "{chance}");
    assert_eq!(outcome::resist_fraction(chance, 0), 0.75);
    assert_eq!((100.0 * outcome::resist_fraction(chance, 0)) as u32, 75);
    // The same roll, a tick's tenth of the chance.
    assert_eq!(outcome::resist_fraction(chance, 5_000), 0.75);
    assert_eq!(outcome::resist_fraction(chance / 10.0, 5_000), 0.0);
}

fn binary_with(e: SpellEffect) -> SpellTemplate {
    let mut s = bare(990_120, 4);
    effect(&mut s, 0, e);
    s
}

/// Binary spells, resisted whole or not at all: magic with a control or
/// debuff aura, an interrupt or a knockback, and two by their ids; not
/// physical, not abilities, not plain damage or damage over time (Thistle
/// Tea's `Spell.binary?`).
#[test]
fn magic_control_is_binary_and_damage_is_not() {
    for a in [
        AURA_MOD_DECREASE_SPEED,
        AURA_MOD_FEAR,
        AURA_MOD_STUN,
        AURA_MOD_PACIFY,
        AURA_MOD_ROOT,
        AURA_MOD_SILENCE,
        AURA_MOD_DISARM,
        AURA_MOD_RESISTANCE,
        AURA_MOD_DAMAGE_TAKEN,
    ] {
        let s = binary_with(aura(a, 0, -10, 6));
        assert!(outcome::binary(&s), "aura {a}");
        for (school, class) in [(0, 1), (4, 2), (4, 3)] {
            let mut t = s.clone();
            t.school = school;
            t.dmg_class = class;
            assert!(
                !outcome::binary(&t),
                "aura {a}, school {school}, class {class}"
            );
        }
    }
    for e in [68, 98] {
        let s = binary_with(SpellEffect {
            effect: e,
            ..damage(1)
        });
        assert!(outcome::binary(&s), "effect {e}");
    }
    for id in [26_143, 26_478] {
        let mut s = bare(id, 3);
        s.id = id;
        assert!(outcome::binary(&s), "spell {id}");
    }
    for e in [
        damage(100),
        aura(AURA_PERIODIC_DAMAGE, 0, 10, 6),
        aura(AURA_PERIODIC_LEECH, 0, 10, 6),
    ] {
        assert!(!outcome::binary(&binary_with(e)));
    }
}

/// An aura on `g`, of a made-up spell of `school` with effects `e`, lasting.
fn put(
    m: &mut Map,
    caster: wow_world_messages::Guid,
    g: wow_world_messages::Guid,
    id: u32,
    school: u32,
    e: &[SpellEffect],
) {
    let mut s = bare(id, school);
    s.duration_index = LASTS_LONG;
    for (i, x) in e.iter().enumerate() {
        effect(&mut s, i, *x);
    }
    apply_template(m, caster, g, &s);
}

/// A unit is frozen by frost's roots and stuns, not another school's nor a
/// frost slow; and Shatter's ranks, by their override class scripts of the
/// mage's family, add 10 to 50 to a mage spell's crit against the frozen
/// (Thistle Tea's `Critical.snapshot` and `target_bonus`, and
/// `Aura.frozen?`).
#[tokio::test]
async fn shatter_crits_the_frozen_by_its_rank_and_frost_holds_alone_freeze() {
    // Shatter's ranks, another family's script and an unrelated one, as
    // talents in the data (Shatter reads a holder's family from it).
    let talent = |id: u32, family: u32, script: i32| {
        let mut t = bare(id, 0);
        t.family_name = family;
        t.duration_index = LASTS_LONG;
        effect(&mut t, 0, aura(AURA_OVERRIDE_CLASS_SCRIPTS, script, 0, 1));
        t
    };
    let talents = [
        talent(990_150, 3, 849),
        talent(990_151, 3, 910),
        talent(990_152, 3, 911),
        talent(990_153, 3, 912),
        talent(990_154, 3, 913),
        talent(990_155, 5, 913),
        talent(990_156, 3, 833),
    ];
    let realm = fast_realm(move |d| insert(d, talents));
    let (_c, mage, map) = human(&realm, "P3CF", "Shatterer", Class::Mage).await;
    let out = map
        .query(move |m| {
            let k = dummy(m, mage, 60, 5.0);
            let mut frozen = Vec::new();
            for (id, school, a) in [
                (990_130, 4, AURA_MOD_ROOT),
                (990_131, 4, AURA_MOD_STUN),
                (990_132, 3, AURA_MOD_ROOT),
                (990_133, 1, AURA_MOD_STUN),
                (990_134, 4, AURA_MOD_DECREASE_SPEED),
            ] {
                put(m, mage, k, id, school, &[aura(a, 0, -50, 6)]);
                frozen.push(outcome::frozen(m, k));
                remove_spell(m, k, id, None);
            }
            // The mage's spell crit 50%, and a frost hold on the kobold.
            let mut bolt = bare(990_140, 4);
            bolt.family_name = 3;
            effect(&mut bolt, 0, damage(100));
            let crits = |m: &mut Map| {
                if let Some(p) = m.players.get_mut(&mage) {
                    p.derived.spell_crit = [50.0; 7];
                }
                outcome::roll_crit(m, mage, k, &bolt, &Mods::default())
            };
            let unfrozen_crit = crits(m);
            put(m, mage, k, 990_130, 4, &[aura(AURA_MOD_ROOT, 0, 0, 6)]);
            let mut shatter = Vec::new();
            for id in 990_150..=990_154 {
                let t = data_spell(m, id);
                apply_template(m, mage, mage, &t);
                shatter.push(outcome::shatter(m, mage, &bolt));
                remove_spell(m, mage, id, None);
            }
            // Another family's script, or another script, adds nothing.
            for id in [990_155, 990_156] {
                let t = data_spell(m, id);
                apply_template(m, mage, mage, &t);
            }
            let none = outcome::shatter(m, mage, &bolt);
            // Shatter's top rank: 50 and 50, a sure crit on the frozen.
            let top = data_spell(m, 990_154);
            apply_template(m, mage, mage, &top);
            let frozen_crit = crits(m);
            set_health(m, k, 1);
            (frozen, unfrozen_crit, shatter, none, frozen_crit)
        })
        .await
        .unwrap();
    let (frozen, unfrozen_crit, shatter, none, frozen_crit) = out;
    assert_eq!(frozen, [true, true, false, false, false]);
    assert_eq!(shatter, [10.0, 20.0, 30.0, 40.0, 50.0]);
    assert_eq!(none, 0.0, "another family's script or another script");
    assert!(
        !unfrozen_crit,
        "half a chance doesn't come up in fixed rolls"
    );
    assert!(frozen_crit, "Shatter's fifty on the frozen makes it sure");
}

fn data_spell(m: &Map, id: u32) -> SpellTemplate {
    m.realm.data.spells.get(id).cloned().unwrap()
}

/// The hit chance against a target's area avoidance and its chance to
/// resist the school, which stack and go with their auras; a binary
/// spell's resistance after them; the caps; and a spell that always hits
/// (Thistle Tea's `context_hit_chance_bp` and `defense_snapshot`).
#[tokio::test]
async fn area_avoidance_and_resist_chance_take_from_the_hit_and_go_with_their_auras() {
    let realm = fast_realm(|_| {});
    let (_c, mage, map) = human(&realm, "P3CA", "Avoider", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let k = dummy(m, mage, 60, 5.0);
            let mut direct = bare(990_160, 4);
            effect(&mut direct, 0, damage(100));
            let mut area = direct.clone();
            area.id = 990_161;
            area.effects[0].target_a = 15;
            let chance = |m: &Map, s: &SpellTemplate| {
                outcome::spell_hit_chance(m, mage, k, s, &Mods::default())
            };
            let mut seen = vec![(chance(m, &area), chance(m, &direct))];
            put(
                m,
                mage,
                k,
                990_170,
                1,
                &[aura(AURA_MOD_AOE_AVOIDANCE, 0, 25, 1)],
            );
            seen.push((chance(m, &area), chance(m, &direct)));
            put(
                m,
                mage,
                k,
                990_171,
                1,
                &[aura(AURA_MOD_AOE_AVOIDANCE, 0, 10, 1)],
            );
            seen.push((chance(m, &area), chance(m, &direct)));
            remove_spell(m, k, 990_170, None);
            seen.push((chance(m, &area), chance(m, &direct)));
            remove_spell(m, k, 990_171, None);
            seen.push((chance(m, &area), chance(m, &direct)));
            // Avoidance, a resist chance for frost and a mechanic's
            // resistance, then frost resistance on a binary stun.
            let mut stun = area.clone();
            stun.id = 990_162;
            stun.mechanic = 12;
            effect(&mut stun, 1, aura(AURA_MOD_STUN, 0, 0, 15));
            put(
                m,
                mage,
                k,
                990_172,
                1,
                &[
                    aura(AURA_MOD_AOE_AVOIDANCE, 0, 25, 1),
                    aura(AURA_MOD_RESIST_CHANCE, 0x10, 4, 1),
                    aura(AURA_MOD_MECHANIC_RESISTANCE, 12, 5, 1),
                ],
            );
            if let Some(f) = m.unit_fields_mut(k) {
                f.set_u32(UNIT_FIELD_RESISTANCES + 4, 60);
            }
            let combined = chance(m, &stun);
            put(
                m,
                mage,
                k,
                990_173,
                1,
                &[aura(AURA_MOD_AOE_AVOIDANCE, 0, 200, 1)],
            );
            let floor = chance(m, &stun);
            remove_spell(m, k, 990_173, None);
            put(
                m,
                mage,
                k,
                990_174,
                1,
                &[aura(AURA_MOD_AOE_AVOIDANCE, 0, -200, 1)],
            );
            let ceiling = chance(m, &stun);
            (seen, combined, floor, ceiling)
        })
        .await
        .unwrap();
    let (seen, combined, floor, ceiling) = out;
    assert_eq!(
        seen,
        [
            (9_600, 9_600),
            (7_100, 9_600),
            (6_100, 9_600),
            (8_600, 9_600),
            (9_600, 9_600)
        ],
        "the area's chance with 25, 25 and 10, 10, and nothing"
    );
    // (9,600 - 2,500 - 400 - 500) x (1 - 60 x 0.15 / 60).
    assert_eq!(combined, (6_200.0 * (1.0 - 0.15)) as i32);
    assert_eq!(floor, 100, "never below 1%");
    assert_eq!(ceiling, 9_900, "nor above 99%");
}
