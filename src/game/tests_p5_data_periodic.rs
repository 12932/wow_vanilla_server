//! Phase 5's periodic damage, Thistle Tea's `PeriodicDamageTest` ported:
//! Curse of Agony's and Starshards' ramps (from their base dice, whatever
//! the amount), the fraction kept through the target's modifiers and then
//! rounded without bias, a flat reduction taking no more than half, and a
//! shield taking from the modified tick.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellTemplate;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::tests_p3_aura2::{add_spells, aura_spell, health, put};
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};

/// Curse of Agony's kind and Starshards', of 24 seconds and 6 a tick a
/// second, base dice of 30; the same of other families or flags; a half
/// more taken, ten more, a thousand less, all of it more; a shield of 10.
const AGONY: u32 = 97_000;
const STARSHARDS: u32 = 97_001;
const NOT_AGONY: u32 = 97_002;
const NOT_AGONY_FLAG: u32 = 97_003;
const NOT_STARSHARDS: u32 = 97_004;
const NOT_STARSHARDS_FLAG: u32 = 97_005;
const HALF_MORE: u32 = 97_010;
const TEN_MORE: u32 = 97_011;
const MUCH_LESS: u32 = 97_012;
const DOUBLE: u32 = 97_013;
const WARD: u32 = 97_014;
/// Agony's and Starshards' kinds with base dice of 7.
const AGONY_7: u32 = 97_015;
const STARSHARDS_7: u32 = 97_016;

fn ramp(id: u32, family: u32, flags: u64, ticks: u32) -> SpellTemplate {
    let mut s = aura_spell(id, Some(ticks * 1_000), &[(AURA_PERIODIC_DAMAGE, 0, 1_000)]);
    s.effects[0].target_a = 6;
    s.effects[0].base_points = 29;
    s.effects[0].base_dice = 1;
    // A share of one of what modifies it (Thistle Tea's fixture's).
    s.effects[0].bonus_coefficient = 1.0;
    s.school = 6;
    s.family_name = family;
    s.family_flags = flags;
    s
}

fn data() -> GameData {
    let mut d = crate::game::tests_p3_aura2::data();
    let sevens =
        [(AGONY_7, 5, 0x400, 12), (STARSHARDS_7, 6, 0x20_0000, 6)].map(|(id, f, flags, n)| {
            let mut s = ramp(id, f, flags, n);
            s.effects[0].base_points = 6;
            s
        });
    add_spells(&mut d, sevens);
    add_spells(
        &mut d,
        [
            ramp(AGONY, 5, 0x400, 12),
            ramp(STARSHARDS, 6, 0x20_0000, 6),
            ramp(NOT_AGONY, 0, 0x400, 12),
            ramp(NOT_AGONY_FLAG, 5, 0x1_0000, 12),
            ramp(NOT_STARSHARDS, 0, 0x20_0000, 12),
            ramp(NOT_STARSHARDS_FLAG, 6, 0x8000_0000, 12),
            aura_spell(HALF_MORE, None, &[(AURA_MOD_DAMAGE_PERCENT_TAKEN, 0x40, 0)]),
            aura_spell(TEN_MORE, None, &[(AURA_MOD_DAMAGE_TAKEN, 0x40, 0)]),
            aura_spell(MUCH_LESS, None, &[(AURA_MOD_DAMAGE_TAKEN, 0x40, 0)]),
            aura_spell(DOUBLE, None, &[(AURA_MOD_DAMAGE_PERCENT_TAKEN, 0x40, 0)]),
            aura_spell(WARD, None, &[(AURA_SCHOOL_ABSORB, 0x40, 0)]),
        ],
    );
    d
}

/// `g`'s next tick of `spell`, due now: the health it took.
fn tick(m: &mut Map, g: Guid, spell: u32) -> u32 {
    m.now += Duration::from_secs(1);
    let now = m.now;
    if let Some(a) = auras::auras_mut(m, g) {
        for h in a.holders.iter_mut().filter(|h| h.spell == spell) {
            for x in h.effects.iter_mut() {
                x.next_tick = Some(now);
            }
        }
    }
    let before = health(m, g);
    auras::update(m);
    before - health(m, g)
}

/// Thistle Tea's ramps: of 60 a tick and base dice of 30, Starshards'
/// six go 50, 50, 60, 60, 70, 70 and Agony's twelve 45 four times, 60,
/// then 75, whatever the amount (100: 90, 100, 110 at its first, third and
/// fifth); then they're gone. Another family's, or another flag's, stays
/// at its amount.
#[tokio::test]
async fn agony_and_starshards_ramp_from_their_base_dice() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            let brute = spawn(m, ga, BRUTE, 1, 3.0);
            let run = |m: &mut Map, spell: u32, amount: i32, ticks: u32| {
                let _ = put(m, ga, brute, spell, &[amount]);
                let out: Vec<u32> = (0..ticks).map(|_| tick(m, brute, spell)).collect();
                let left = auras::auras(m, brute).is_some_and(|a| a.has_spell(spell));
                auras::remove_spell(m, brute, spell, None);
                (out, left)
            };
            let starshards = run(m, STARSHARDS, 60, 6);
            let agony = run(m, AGONY, 60, 12);
            let big = run(m, STARSHARDS, 100, 5);
            let mut flat = Vec::new();
            for spell in [
                NOT_AGONY,
                NOT_AGONY_FLAG,
                NOT_STARSHARDS,
                NOT_STARSHARDS_FLAG,
            ] {
                flat.push(run(m, spell, 30, 12).0);
                auras::remove_spell(m, brute, spell, None);
            }
            (starshards, agony, big, flat)
        })
        .await
        .unwrap();
    let (starshards, agony, big, flat) = got;
    assert_eq!(starshards, (vec![50, 50, 60, 60, 70, 70], false));
    let mut want = vec![45; 4];
    want.extend([60; 4]);
    want.extend([75; 4]);
    assert_eq!(agony, (want, false));
    assert_eq!((big.0[0], big.0[2], big.0[4]), (90, 100, 110));
    for ticks in flat {
        assert_eq!(ticks, vec![30; 12]);
    }
}

/// Thistle Tea's rounding: Agony's first ticks of 7 (3.5 by the ramp)
/// come to 3 or 4 and 3.5 on the whole, rolled; Starshards' of 7 (4⅔)
/// half again taken is 7 whatever the roll, and ten more taken is 14⅔:
/// 15 with the rolls fixed. A thousand less takes only half of a tick of
/// 60. All of it more taken (100), a ward of 10 takes from that once.
#[tokio::test]
async fn a_ramps_fraction_takes_the_targets_modifiers_then_rounds_without_bias() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 1, 3.0);
            // Rolled: Agony's 3.5.
            m.spells.rolls = Some(true);
            let mut agony = Vec::new();
            for _ in 0..300 {
                let _ = put(m, ga, brute, AGONY_7, &[7]);
                agony.push(tick(m, brute, AGONY_7));
                auras::remove_spell(m, brute, AGONY_7, None);
            }
            m.spells.rolls = Some(false);
            let shards = |m: &mut Map, with: &[(u32, i32)]| {
                for (s, v) in with {
                    let _ = put(m, brute, brute, *s, &[*v]);
                }
                let _ = put(m, ga, brute, STARSHARDS_7, &[7]);
                let got = tick(m, brute, STARSHARDS_7);
                auras::remove_spell(m, brute, STARSHARDS_7, None);
                for (s, _) in with {
                    auras::remove_spell(m, brute, *s, None);
                }
                got
            };
            let half = shards(m, &[(HALF_MORE, 50)]);
            let ten = shards(m, &[(TEN_MORE, 10)]);
            // A thousand less on a tick of 60 (Starshards' third).
            let _ = put(m, brute, brute, MUCH_LESS, &[-1_000]);
            let _ = put(m, ga, brute, STARSHARDS, &[60]);
            let halved: Vec<u32> = (0..3).map(|_| tick(m, brute, STARSHARDS)).collect();
            auras::remove_spell(m, brute, STARSHARDS, None);
            auras::remove_spell(m, brute, MUCH_LESS, None);
            // Doubled, then warded.
            let _ = put(m, brute, brute, DOUBLE, &[100]);
            let _ = put(m, brute, brute, WARD, &[10]);
            let _ = put(m, ga, brute, STARSHARDS, &[60]);
            let warded = tick(m, brute, STARSHARDS);
            let ward_gone = !auras::auras(m, brute).is_some_and(|a| a.has_spell(WARD));
            (agony, half, ten, halved, warded, ward_gone)
        })
        .await
        .unwrap();
    let (agony, half, ten, halved, warded, ward_gone) = got;
    assert!(agony.iter().all(|t| *t == 3 || *t == 4), "{agony:?}");
    let mean = agony.iter().sum::<u32>() as f32 / agony.len() as f32;
    assert!((mean - 3.5).abs() < 0.15, "unbiased: {mean}");
    assert_eq!((half, ten), (7, 15));
    assert_eq!(halved[2], 30, "no more than half taken off");
    assert_eq!((warded, ward_gone), (90, true), "100 less the ward's 10");
}
