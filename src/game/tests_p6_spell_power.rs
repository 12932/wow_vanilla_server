//! Phase 3, round 6, against Thistle Tea's `spell_refunds_test.exs`: what
//! an avoided ability gives back of what it cost.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, has, power, set_power};
use crate::game::data::GameData;
use crate::game::data::spells::{
    ATTR_EX_DISCOUNT_POWER_ON_MISS, ATTR_EX_REQ_COMBO_POINTS, SpellEffect, SpellTemplate,
};
use crate::game::fields::*;
use crate::game::map::auras::{
    AURA_ADD_FLAT_MODIFIER, AURA_ADD_PCT_MODIFIER, AURA_SCHOOL_IMMUNITY,
};
use crate::game::map::spells::{self, mods};

/// A rogue's strike for 45 energy given back on a miss; one that adds a
/// combo point beside; a finisher for 35 that gives nothing back.
const STRIKE: u32 = 990_940;
const POINTED: u32 = 990_941;
const FINISHER: u32 = 990_942;
/// Five less energy for the next strike, then none, each one charge; a
/// shield of the physical school.
const THRIFT: u32 = 990_943;
const FREE: u32 = 990_944;
const SHIELD: u32 = 990_945;

fn rogue(d: &mut GameData) {
    let ability = |id: u32, cost: u32| {
        let mut s = bare(id, 0);
        s.dmg_class = 2;
        s.power_type = 3;
        s.mana_cost = cost;
        s.family_name = 8;
        s.family_flags = 0x1;
        s.attributes[1] |= ATTR_EX_DISCOUNT_POWER_ON_MISS;
        effect(&mut s, 0, damage(10));
        s
    };
    let mut pointed = ability(POINTED, 45);
    effect(
        &mut pointed,
        1,
        SpellEffect {
            effect: spells::EFFECT_ADD_COMBO_POINTS,
            base_points: 0,
            base_dice: 1,
            target_a: 6,
            ..Default::default()
        },
    );
    let mut finisher = ability(FINISHER, 35);
    finisher.attributes[1] = ATTR_EX_REQ_COMBO_POINTS;
    let modifier = |id: u32, kind: u32, amount: i32| {
        let mut s = bare(id, 0);
        s.family_name = 8;
        s.duration_index = LASTS_LONG;
        s.proc_charges = 1;
        effect(
            &mut s,
            0,
            SpellEffect {
                item_type: 0x1,
                ..aura(kind, mods::OP_COST as i32, amount, 1)
            },
        );
        s
    };
    let mut shield = bare(SHIELD, 1);
    shield.duration_index = LASTS_LONG;
    effect(&mut shield, 0, aura(AURA_SCHOOL_IMMUNITY, 1, 0, 1));
    let spells: Vec<SpellTemplate> = vec![
        ability(STRIKE, 45),
        pointed,
        finisher,
        modifier(THRIFT, AURA_ADD_FLAT_MODIFIER, -5),
        modifier(FREE, AURA_ADD_PCT_MODIFIER, -100),
        shield,
    ];
    insert(d, spells);
}

/// Spells on the global cooldown: 1.5 s of cast, an instant, one of the
/// category with no time of its own, one of neither; a modifier of 0.1 s
/// less, one of 0.1 s more, and one of a charge.
const SLOW: u32 = 990_950;
const QUICK: u32 = 990_951;
const CATEGORY_ONLY: u32 = 990_952;
const NEITHER: u32 = 990_953;
const SHORTER: u32 = 990_954;
const LONGER: u32 = 990_955;
const ONCE_SHORTER: u32 = 990_956;
const CAST_1500: u32 = 904;

fn cooldowns(d: &mut GameData) {
    d.spells.cast_times.insert(CAST_1500, 1_500);
    let on_gcd = |id: u32, cast: u32, category: u32, ms: u32| {
        let mut s = bare(id, 1);
        s.family_name = 6;
        s.family_flags = 0x1;
        s.casting_time_index = cast;
        s.start_recovery_category = category;
        s.start_recovery_time = ms;
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: 10,
                base_points: 9,
                base_dice: 1,
                target_a: 1,
                ..Default::default()
            },
        );
        s
    };
    let modifier = |id: u32, amount: i32, charges: u32| {
        let mut s = bare(id, 0);
        s.family_name = 6;
        s.duration_index = LASTS_LONG;
        s.proc_charges = charges;
        effect(
            &mut s,
            0,
            SpellEffect {
                item_type: 0x1,
                ..aura(
                    AURA_ADD_FLAT_MODIFIER,
                    mods::OP_GLOBAL_COOLDOWN as i32,
                    amount,
                    1,
                )
            },
        );
        s
    };
    let spells: Vec<SpellTemplate> = vec![
        on_gcd(SLOW, CAST_1500, 133, 1_500),
        on_gcd(QUICK, INSTANT, 133, 1_500),
        on_gcd(CATEGORY_ONLY, INSTANT, 133, 0),
        on_gcd(NEITHER, INSTANT, 0, 0),
        modifier(SHORTER, -100, 0),
        modifier(LONGER, 100, 0),
        modifier(ONCE_SHORTER, -100, 1),
    ];
    insert(d, spells);
}

/// The global cooldown is its spell's own time with its caster's
/// modifiers: a cast hastened by a third ends before it, at 1127 ms, and
/// it runs its 1.5 s; a creature's takes no modifiers; a spell of the
/// category with no time of its own takes a modifier's, one of no category
/// none; a charged modifier stays on a cast cancelled and goes with one
/// completed (Thistle Tea's `Cooldowns.gcd_duration` and `complete/2`;
/// mangos's `Player::AddGCD` and `GetSpellModOwner`).
#[tokio::test]
async fn the_global_cooldown_is_the_spells_own_with_a_players_modifiers() {
    use std::time::Duration;
    let realm = fast_realm(cooldowns);
    let (_c, a, map) = human(&realm, "P6PB", "Hasted", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[SLOW, QUICK, CATEGORY_ONLY, NEITHER]);
            let gcd = |m: &crate::game::map::Map, g| {
                let now = m.now;
                m.caster(g)
                    .and_then(|c| c.gcd_until)
                    .map(|t| t.saturating_duration_since(now).as_millis() as u64)
            };
            if let Some(f) = m.unit_fields_mut(a) {
                f.set_f32(UNIT_MOD_CAST_SPEED, 1.0 / 1.33);
            }
            cast_fresh(m, a, SLOW, at(a));
            let cast_ms = m
                .caster(a)
                .and_then(|c| c.casting.as_ref())
                .map(|c| c.duration);
            kit::pass(m, 1_127);
            let after_cast = (kit::casting(m, a), gcd(m, a));
            if let Some(f) = m.unit_fields_mut(a) {
                f.set_f32(UNIT_MOD_CAST_SPEED, 1.0);
            }
            // A creature under the same modifier as the player.
            let kobold = kit::dummy(m, a, 60, 5.0);
            kit::apply(m, kobold, kobold, SHORTER);
            kit::apply(m, a, a, SHORTER);
            kit::fresh(m, kobold);
            spells::cast(m, kobold, QUICK, at(kobold), None);
            cast_fresh(m, a, QUICK, at(a));
            let modified = (gcd(m, kobold), gcd(m, a));
            crate::game::map::auras::remove_spell(m, a, SHORTER, None);
            kit::apply(m, a, a, LONGER);
            cast_fresh(m, a, CATEGORY_ONLY, at(a));
            let category_only = gcd(m, a);
            cast_fresh(m, a, NEITHER, at(a));
            let neither = gcd(m, a);
            crate::game::map::auras::remove_spell(m, a, LONGER, None);
            kit::apply(m, a, a, ONCE_SHORTER);
            cast_fresh(m, a, SLOW, at(a));
            m.now += Duration::from_millis(500);
            spells::interrupt(m, a);
            let kept = has(m, a, ONCE_SHORTER);
            cast_fresh(m, a, SLOW, at(a));
            kit::pass(m, 1_500);
            (
                cast_ms,
                after_cast,
                modified,
                category_only,
                neither,
                kept,
                has(m, a, ONCE_SHORTER),
            )
        })
        .await
        .unwrap();
    let (cast_ms, after_cast, modified, category_only, neither, kept, spent) = out;
    assert_eq!(cast_ms, Some(1_127), "hastened by a third");
    assert_eq!(
        after_cast,
        (None, Some(373)),
        "cast, its cooldown still running"
    );
    assert_eq!(
        modified,
        (Some(1_500), Some(1_400)),
        "a creature's, a player's"
    );
    assert_eq!(category_only, Some(100), "a modifier's time");
    assert_eq!(neither, None);
    assert!(kept, "kept through a cancelled cast");
    assert!(!spent, "spent by a completed one");
}

/// A fire spell of 100 mana and 5 a level from its fifth, of the Fire
/// line; the same of no line; one of 100 and 5% of base mana, scaling with
/// its caster's level; one of 3; their modifiers.
const COSTLY: u32 = 990_960;
const LINELESS: u32 = 990_961;
const SHARED: u32 = 990_962;
const CHEAP: u32 = 990_963;
const COST_AURAS: u32 = 990_964;
const TENTH_OFF: u32 = 990_965;
const SKILL_UP: u32 = 990_966;
const TENTH_CHEAPER: u32 = 990_967;
const FIRE: u32 = 8;

fn costs(d: &mut GameData) {
    use crate::game::data::spells::ATTR_LEVEL_DAMAGE_CALCULATION;
    use crate::game::map::auras::{
        AURA_MOD_POWER_COST_SCHOOL, AURA_MOD_POWER_COST_SCHOOL_PCT, AURA_MOD_SKILL,
    };
    let fire = |id: u32, cost: u32, per_level: u32| {
        let mut s = bare(id, 2);
        s.mana_cost = cost;
        s.mana_cost_per_level = per_level;
        s.base_level = 5;
        s.family_name = 3;
        s.family_flags = 0x1;
        effect(&mut s, 0, damage(10));
        s
    };
    let mut shared = fire(SHARED, 100, 0);
    shared.mana_cost_percentage = 5;
    shared.spell_level = 20;
    shared.attributes[0] |= ATTR_LEVEL_DAMAGE_CALCULATION;
    let mut cost_auras = bare(COST_AURAS, 0);
    cost_auras.family_name = 3;
    cost_auras.duration_index = LASTS_LONG;
    let family = |kind: u32, amount: i32| SpellEffect {
        item_type: 0x1,
        ..aura(kind, mods::OP_COST as i32, amount, 1)
    };
    effect(
        &mut cost_auras,
        0,
        aura(AURA_MOD_POWER_COST_SCHOOL, 4, 10, 1),
    );
    effect(&mut cost_auras, 1, family(AURA_ADD_FLAT_MODIFIER, -5));
    effect(&mut cost_auras, 2, family(AURA_ADD_PCT_MODIFIER, -20));
    // The school's share, a second holder's.
    let mut tenth = bare(TENTH_OFF, 0);
    tenth.family_name = 3;
    tenth.duration_index = LASTS_LONG;
    effect(
        &mut tenth,
        0,
        aura(AURA_MOD_POWER_COST_SCHOOL_PCT, 4, 10, 1),
    );
    let mut cheaper = bare(TENTH_CHEAPER, 0);
    cheaper.family_name = 3;
    cheaper.duration_index = LASTS_LONG;
    effect(&mut cheaper, 0, family(AURA_ADD_PCT_MODIFIER, -10));
    let mut skill_up = bare(SKILL_UP, 0);
    skill_up.duration_index = LASTS_LONG;
    effect(&mut skill_up, 0, aura(AURA_MOD_SKILL, FIRE as i32, 5, 1));
    let spells: Vec<SpellTemplate> = vec![
        fire(COSTLY, 100, 5),
        fire(LINELESS, 100, 5),
        shared,
        fire(CHEAP, 3, 0),
        cost_auras,
        tenth,
        cheaper,
        skill_up,
    ];
    insert(d, spells);
    d.spells.spell_skills.insert(COSTLY, FIRE);
    d.spells.spell_skills.insert(SHARED, FIRE);
    d.spells.spell_skills.insert(CHEAP, FIRE);
}

/// A spell's cost by the level it is cast at: a creature's level in steps
/// of five, no higher than the spell's last; a player's skill in the
/// spell's line (with its bonuses), none for a spell of no line. Then, in
/// order, the school's flat change, the family's flat and percent
/// modifiers, the level's scaling and the school's percentage, each
/// truncated, and nothing below nothing (Thistle Tea's `CostScaling` and
/// `Resources.power_cost`; mangos's `CalculatePowerCost` and
/// `GetSpellRank`).
#[tokio::test]
async fn a_spells_cost_goes_by_its_rank_then_its_modifiers_in_order() {
    let realm = fast_realm(costs);
    let (_c, a, map) = human(&realm, "P6PC", "Costly", Class::Mage).await;
    let out = map
        .query(move |m| {
            fn cost_of(
                m: &mut crate::game::map::Map,
                g: wow_world_messages::Guid,
                s: &SpellTemplate,
            ) -> u32 {
                if let Some(p) = m.players.get_mut(&g) {
                    p.fields.set_u32(UNIT_FIELD_BASE_MANA, 1_000);
                }
                spells::spell_cost(m, g, s, &mods::for_spell(m, g, s))
            }
            let spell = |m: &crate::game::map::Map, id: u32, f: &dyn Fn(&mut SpellTemplate)| {
                let mut s = m.realm.data.spells.get(id).cloned().unwrap();
                f(&mut s);
                s
            };
            let costly = spell(m, COSTLY, &|_| {});
            let capped = spell(m, COSTLY, &|s| s.max_level = 8);
            let mut creatures = Vec::new();
            for (level, s) in [(50, &costly), (54, &costly), (55, &costly), (50, &capped)] {
                let kobold = kit::dummy(m, a, level, 5.0);
                creatures.push(cost_of(m, kobold, s));
            }
            kit::ready(m, a, 50, &[]);
            let p = m.players.get_mut(&a).unwrap();
            crate::game::map::skills::learn_skill(p, FIRE, 1);
            let slot = (0..128u16)
                .find(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) & 0xFFFF == FIRE)
                .unwrap();
            p.fields
                .set_u32(PLAYER_SKILL_INFO_1_1 + slot * 3 + 1, 249 | (250 << 16));
            let twentieth = spell(m, COSTLY, &|s| s.max_level = 20);
            let skilled = (cost_of(m, a, &costly), cost_of(m, a, &twentieth));
            kit::apply(m, a, a, SKILL_UP);
            crate::game::map::skills::set_bonuses(m.players.get_mut(&a).unwrap());
            let lineless = spell(m, LINELESS, &|_| {});
            let bonus = (cost_of(m, a, &costly), cost_of(m, a, &lineless));
            kit::apply(m, a, a, COST_AURAS);
            kit::apply(m, a, a, TENTH_OFF);
            let shared = spell(m, SHARED, &|_| {});
            let unscaled = spell(m, SHARED, &|s| s.spell_level = 0);
            let ordered = (cost_of(m, a, &shared), cost_of(m, a, &unscaled));
            crate::game::map::auras::remove_spell(m, a, COST_AURAS, None);
            crate::game::map::auras::remove_spell(m, a, TENTH_OFF, None);
            kit::apply(m, a, a, TENTH_CHEAPER);
            let cheap = spell(m, CHEAP, &|_| {});
            let scaled = spell(m, CHEAP, &|s| {
                s.attributes[0] |= crate::game::data::spells::ATTR_LEVEL_DAMAGE_CALCULATION;
                s.spell_level = 5;
            });
            let cheap = (cost_of(m, a, &cheap), cost_of(m, a, &scaled));
            (creatures, skilled, bonus, ordered, cheap)
        })
        .await
        .unwrap();
    let (creatures, skilled, bonus, ordered, cheap) = out;
    assert_eq!(
        creatures,
        [125, 125, 130, 115],
        "by fives, to its last level"
    );
    assert_eq!(
        skilled,
        (320, 175),
        "skill 249; capped at its twentieth level"
    );
    assert_eq!(bonus, (325, 75), "a skill bonus; no line");
    // 150, +10 the school, -5 then -20% the family: 124; by the level
    // (20 of 50): 394; +10% the school: 433. Unscaled: 136.
    assert_eq!(ordered, (433, 136));
    assert_eq!(cheap, (2, 0), "2.7 truncated; scaled below nothing");
}

/// An ability kept off by an immunity gives back 82% of what was paid for
/// it: of the cost a charged modifier cut, which is spent; nothing when a
/// modifier made it free, nor for a triggered one, which paid nothing. One
/// with two effects gives back once and adds no combo point. A finisher
/// without the attribute gives nothing back, and keeps its combo points
/// (Thistle Tea's paid cast feedback and `AttackFeedback`; mangos's
/// `SPELL_ATTR_EX_DISCOUNT_POWER_ON_MISS` and its combo points kept on a
/// miss).
#[tokio::test]
async fn an_avoided_ability_gives_back_what_was_paid() {
    let realm = fast_realm(rogue);
    let (_c, a, map) = human(&realm, "P6PA", "Refunded", Class::Rogue).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[STRIKE, POINTED, FINISHER]);
            if let Some(f) = m.unit_fields_mut(a) {
                f.set_u32(UNIT_FIELD_MAXPOWER1 + 3, 100);
            }
            let kobold = kit::dummy(m, a, 60, 2.0);
            kit::apply(m, kobold, kobold, SHIELD);
            let mut energy = Vec::new();
            let mut strike =
                |m: &mut crate::game::map::Map, spell: u32, from: u32, aura: Option<u32>| {
                    set_power(m, a, 3, from);
                    if let Some(x) = aura {
                        kit::apply(m, a, a, x);
                    }
                    cast_fresh(m, a, spell, at(kobold));
                    energy.push(power(m, a, 3));
                    aura.is_some_and(|x| has(m, a, x))
                };
            let thrift_left = strike(m, STRIKE, 100, Some(THRIFT));
            let free_left = strike(m, STRIKE, 50, Some(FREE));
            set_power(m, a, 3, 50);
            spells::cast_triggered(m, a, STRIKE, Some(kobold));
            let triggered = power(m, a, 3);
            strike(m, POINTED, 100, None);
            let points = m.caster(a).map(|c| c.combo_points);
            if let Some(c) = m.caster_mut(a) {
                c.combo_points = 5;
                c.combo_target = kobold;
            }
            strike(m, FINISHER, 100, None);
            let kept = m
                .caster(a)
                .map(|c| (c.combo_points, c.combo_target == kobold));
            (energy, thrift_left, free_left, triggered, points, kept)
        })
        .await
        .unwrap();
    let (energy, thrift_left, free_left, triggered, points, kept) = out;
    assert_eq!(
        energy,
        [93, 50, 92, 65],
        "40 paid, 33 back; free; 45 paid, 37 back once; a finisher's 35 kept"
    );
    assert!(!thrift_left && !free_left, "the charges spent");
    assert_eq!(triggered, 50, "a triggered strike paid nothing");
    assert_eq!(points, Some(0), "no combo point on an immune target");
    assert_eq!(kept, Some((5, true)), "the finisher keeps its points");
}
