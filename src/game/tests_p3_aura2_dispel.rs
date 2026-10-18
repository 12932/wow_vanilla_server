//! Phase 3's dispels, mechanic resistance and percentage stats, Thistle
//! Tea's `DispelTest`, `DispelResistanceTest`, `MechanicResistanceTest`
//! and `PercentStatTest` ported.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::spells::{self, mods};
use crate::game::tests_p3_aura2::{add_spells, aura_spell, get, has, health, pass, put};
use crate::game::tests_p3_combat::{BRUTE, HELPER, setup_with, spawn};

/// Harmful auras of each dispel type: magic, curse, disease, poison,
/// stealth (5), invisibility (6) and enrage (9).
const AFFLICTION: u32 = 96_800;
const AFFLICTION_TYPES: [u32; 7] = [1, 2, 3, 4, 5, 6, 9];
/// A helpful magic aura and a helpful curse.
const HELPFUL_MAGIC: u32 = 96_810;
const HELPFUL_CURSE: u32 = 96_811;
/// Dispels of ten: all, magic, curse.
const DISPEL_ALL: u32 = 96_812;
const DISPEL_MAGIC: u32 = 96_813;
const REMOVE_CURSE: u32 = 96_814;
/// A mage's magic heal over time (family 3, flag 0x1; two casters' stand
/// side by side) and a talent holding it on whole (its dispel resisted).
const MAGE_BUFF: u32 = 96_815;
const HOLD_ON: u32 = 96_816;
/// Resistances to stuns of 25 and 15, and to fears of 100; a magic stun.
const STUN_RESIST_25: u32 = 96_817;
const STUN_RESIST_15: u32 = 96_818;
const FEAR_RESIST: u32 = 96_819;
const MAGIC_STUN: u32 = 96_820;
/// A fifth more of every stat, and three quarters less (sickness).
const MIGHTY: u32 = 96_821;
const SICK: u32 = 96_822;

fn dispel(id: u32, kind: i32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Dispel {id}"),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: spells::EFFECT_DISPEL,
        misc_value: kind,
        base_points: 9,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    };
    s
}

fn data() -> GameData {
    let mut d = crate::game::tests_p3_aura2::data();
    let mut all = Vec::new();
    for (i, kind) in AFFLICTION_TYPES.iter().enumerate() {
        let mut s = aura_spell(
            AFFLICTION + i as u32,
            Some(30_000),
            &[(AURA_MOD_STAT, 0, 0)],
        );
        s.dispel = *kind;
        s.attributes[0] |= crate::game::data::spells::ATTR_DEBUFF;
        all.push(s);
    }
    for (id, kind) in [(HELPFUL_MAGIC, 1), (HELPFUL_CURSE, 2)] {
        let mut s = aura_spell(id, Some(30_000), &[(AURA_MOD_STAT, 0, 0)]);
        s.dispel = kind;
        all.push(s);
    }
    let mut buff = aura_spell(MAGE_BUFF, Some(30_000), &[(AURA_PERIODIC_HEAL, 0, 3000)]);
    buff.dispel = 1;
    buff.family_name = 3;
    buff.family_flags = 0x1;
    let mut hold = aura_spell(
        HOLD_ON,
        None,
        &[(
            AURA_ADD_FLAT_MODIFIER,
            mods::OP_RESIST_DISPEL_CHANCE as i32,
            0,
        )],
    );
    hold.family_name = 3;
    hold.effects[0].item_type = 0x1;
    hold.attributes[0] |= crate::game::data::spells::ATTR_PASSIVE;
    let mut stun = aura_spell(MAGIC_STUN, Some(10_000), &[(AURA_MOD_STUN, 0, 0)]);
    stun.mechanic = 12;
    stun.school = 6;
    stun.effects[0].target_a = 6;
    all.extend([
        buff,
        hold,
        stun,
        dispel(DISPEL_ALL, 7),
        dispel(DISPEL_MAGIC, 1),
        dispel(REMOVE_CURSE, 2),
        aura_spell(
            STUN_RESIST_25,
            None,
            &[(AURA_MOD_MECHANIC_RESISTANCE, 12, 0)],
        ),
        aura_spell(
            STUN_RESIST_15,
            None,
            &[(AURA_MOD_MECHANIC_RESISTANCE, 12, 0)],
        ),
        aura_spell(FEAR_RESIST, None, &[(AURA_MOD_MECHANIC_RESISTANCE, 5, 0)]),
        aura_spell(MIGHTY, Some(1_000), &[(AURA_MOD_PERCENT_STAT, -1, 0)]),
        aura_spell(SICK, Some(1_000), &[(AURA_MOD_PERCENT_STAT, -1, 0)]),
    ]);
    add_spells(&mut d, all);
    d
}

/// The dispel types of `g`'s afflictions.
fn afflictions(m: &Map, g: Guid) -> Vec<u32> {
    AFFLICTION_TYPES
        .iter()
        .enumerate()
        .filter(|(i, _)| has(m, g, AFFLICTION + *i as u32))
        .map(|(_, k)| *k)
        .collect()
}

/// Thistle Tea's `Dispel.matches?`: a dispel of all takes the four
/// standard types (magic, curse, disease, poison) and not stealth,
/// invisibility or enrage; magic and poison keep to their polarity (a
/// friend's dispel takes a friend's harmful magic, not its helpful;
/// an enemy's helpful magic is an offensive dispel's), a curse doesn't
/// (a friend's helpful curse goes to Remove Curse).
#[tokio::test]
async fn a_dispel_takes_its_types_by_their_polarity() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Priest).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            let friend = spawn(m, ga, HELPER, 20, 5.0);
            for i in 0..AFFLICTION_TYPES.len() as u32 {
                put(m, brute, ga, AFFLICTION + i, &[-1]).unwrap();
            }
            spells::cast_triggered(m, ga, DISPEL_ALL, Some(ga));
            let all = afflictions(m, ga);
            put(m, brute, friend, AFFLICTION, &[-1]).unwrap();
            put(m, friend, friend, HELPFUL_MAGIC, &[1]).unwrap();
            put(m, friend, friend, HELPFUL_CURSE, &[1]).unwrap();
            spells::cast_triggered(m, ga, DISPEL_MAGIC, Some(friend));
            let friendly = (has(m, friend, AFFLICTION), has(m, friend, HELPFUL_MAGIC));
            spells::cast_triggered(m, ga, REMOVE_CURSE, Some(friend));
            let curse = has(m, friend, HELPFUL_CURSE);
            put(m, brute, brute, HELPFUL_MAGIC, &[1]).unwrap();
            spells::cast_triggered(m, ga, DISPEL_MAGIC, Some(brute));
            (all, friendly, curse, has(m, brute, HELPFUL_MAGIC))
        })
        .await
        .unwrap();
    assert_eq!(got.0, [5, 6, 9], "the four standard types taken");
    assert_eq!(
        got.1,
        (false, true),
        "a friend's harmful magic, not its helpful"
    );
    assert!(!got.2, "a curse whatever its polarity");
    assert!(!got.3, "an enemy's helpful magic");
}

/// Thistle Tea's `DispelResistance`: an aura its caster's talent holds on
/// (a modifier to its dispel chance, of its family and mask) resists the
/// dispel, and isn't tried again in the same cast: of two such buffs from
/// two mages, a dispel of two takes the unprotected one alone.
#[tokio::test]
async fn a_held_aura_resists_its_dispel_once_and_the_next_goes() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Priest).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            let other = spawn(m, ga, BRUTE, 20, 5.0);
            put(m, brute, brute, HOLD_ON, &[100]).unwrap();
            put(m, brute, brute, MAGE_BUFF, &[1]).unwrap();
            put(m, other, brute, MAGE_BUFF, &[1]).unwrap();
            let two = dispel(DISPEL_MAGIC + 100, 1);
            let mut two = two;
            two.effects[0].base_points = 1;
            crate::game::map::spells::scripts::cast_template_with_points(
                m,
                ga,
                two,
                brute,
                [None, None, None],
            );
            auras::auras(m, brute).map(|a| {
                a.holders
                    .iter()
                    .filter(|h| h.spell == MAGE_BUFF)
                    .map(|h| h.caster == brute)
                    .collect::<Vec<_>>()
            })
        })
        .await
        .unwrap();
    assert_eq!(got, Some(vec![true]), "the protected one stays");
}

/// Thistle Tea's `MechanicResistance`: resistances to one mechanic sum
/// (25 and 15 to stuns, 40), one to another mechanic counts for nothing;
/// a spell's chance to hit loses them.
#[tokio::test]
async fn mechanic_resistances_of_one_mechanic_sum_into_the_hit() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 1, 5.0);
            let s = m.realm.data.spells.get(MAGIC_STUN).unwrap().clone();
            let chance =
                |m: &Map| spells::outcome::spell_hit_chance(m, ga, brute, &s, &Default::default());
            let before = chance(m);
            put(m, brute, brute, STUN_RESIST_25, &[25]).unwrap();
            put(m, brute, brute, STUN_RESIST_15, &[15]).unwrap();
            put(m, brute, brute, FEAR_RESIST, &[100]).unwrap();
            (before, chance(m))
        })
        .await
        .unwrap();
    assert_eq!(got, (9_600, 5_600));
}

/// `g`'s five stats.
fn stats(m: &Map, g: Guid) -> [u32; 5] {
    std::array::from_fn(|i| get(m, g, UNIT_FIELD_STAT0 + i as u16))
}

/// Thistle Tea's `PercentStatTest`: a fifth more of every stat, refreshed,
/// doesn't compound; it runs out, is cancelled or goes at death, and the
/// stats are the player's own again. Three quarters less clamps health
/// and mana to the smaller pools, and its end doesn't refill them.
#[tokio::test]
async fn a_percentage_of_the_stats_comes_and_goes_without_compounding() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Priest).await;
    let got = map
        .query(move |m| {
            let own = stats(m, ga);
            put(m, ga, ga, MIGHTY, &[20]).unwrap();
            let on = stats(m, ga);
            put(m, ga, ga, MIGHTY, &[20]).unwrap();
            let refreshed = stats(m, ga);
            pass(m, 1_000);
            let expired = stats(m, ga);
            put(m, ga, ga, MIGHTY, &[20]).unwrap();
            auras::cancel(m, ga, MIGHTY);
            let cancelled = stats(m, ga);
            // Sickness clamps; its end doesn't refill.
            let full = (
                get(m, ga, UNIT_FIELD_MAXHEALTH),
                get(m, ga, UNIT_FIELD_MAXPOWER1),
            );
            put(m, ga, ga, SICK, &[-75]).unwrap();
            let sick = (health(m, ga), get(m, ga, UNIT_FIELD_POWER1));
            let sick_max = (
                get(m, ga, UNIT_FIELD_MAXHEALTH),
                get(m, ga, UNIT_FIELD_MAXPOWER1),
            );
            pass(m, 1_000);
            let after = (health(m, ga), get(m, ga, UNIT_FIELD_POWER1));
            (
                own, on, refreshed, expired, cancelled, full, sick, sick_max, after,
            )
        })
        .await
        .unwrap();
    let (own, on, refreshed, expired, cancelled, full, sick, sick_max, after) = got;
    assert_eq!(own, [20; 5]);
    assert_eq!(on, [24; 5], "a fifth more");
    assert_eq!(refreshed, on, "not compounded");
    assert_eq!((expired, cancelled), (own, own));
    assert!(sick_max.0 < full.0 && sick_max.1 < full.1);
    assert_eq!(sick, sick_max, "clamped to the smaller pools");
    assert_eq!(after, sick, "not refilled");
}
