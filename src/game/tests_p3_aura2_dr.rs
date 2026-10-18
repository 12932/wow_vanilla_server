//! Phase 3's diminishing returns, Thistle Tea's `DiminishingReturnsTest`
//! and `DiminishingReturnsAuraTest` ported: stuns halved across casters,
//! the groups apart, recovery once the last of a group is gone, and what
//! spends no step.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::spells;
use crate::game::tests_p3_aura2::{add_spells, aura_spell, health, pass, put};
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};
use crate::game::tests_p3_spells_kit::missed;

/// Stuns of eight seconds: two of their own, one with a root beside it,
/// one of magic with a point of damage too, Impact's (a triggered stun)
/// and Kidney Shot's kind.
const STUN: u32 = 96_950;
const STUN_2: u32 = 96_951;
const STUN_ROOT: u32 = 96_952;
const STUN_HURTS: u32 = 96_953;
const IMPACT: u32 = 12355;
const KIDNEY: u32 = 96_954;
/// An immunity to stuns; a dispel of magic.
const STUN_IMMUNE: u32 = 96_955;
const DISPEL: u32 = 96_956;

fn stun(id: u32) -> SpellTemplate {
    let mut s = aura_spell(id, Some(8_000), &[(AURA_MOD_STUN, 0, 0)]);
    s.mechanic = 12;
    s.effects[0].target_a = 6;
    s
}

fn data() -> GameData {
    let mut d = crate::game::tests_p3_aura2::data();
    let mut root = stun(STUN_ROOT);
    root.effects[1] = root.effects[0];
    root.effects[1].aura = AURA_MOD_ROOT;
    let mut hurts = stun(STUN_HURTS);
    hurts.dispel = 1;
    hurts.effects[1] = SpellEffect {
        effect: spells::EFFECT_SCHOOL_DAMAGE,
        base_points: 9,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    };
    let mut impact = stun(IMPACT);
    impact.mechanic = 0;
    let mut kidney = stun(KIDNEY);
    kidney.family_name = 8;
    kidney.family_flags = 0x20_0000;
    let mut dispel = SpellTemplate {
        id: DISPEL,
        name: "Dispel".into(),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    dispel.effects[0] = SpellEffect {
        effect: spells::EFFECT_DISPEL,
        misc_value: 1,
        base_points: 0,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    add_spells(
        &mut d,
        [
            stun(STUN),
            stun(STUN_2),
            root,
            hurts,
            impact,
            kidney,
            dispel,
            aura_spell(STUN_IMMUNE, None, &[(AURA_MECHANIC_IMMUNITY, 12, 0)]),
        ],
    );
    d
}

/// `g`'s `spell`'s full duration, if on.
fn lasts(m: &Map, g: Guid, spell: u32) -> Option<u32> {
    auras::auras(m, g)?
        .holders
        .iter()
        .find(|h| h.spell == spell)
        .and_then(|h| h.duration)
}

/// `g`'s stuns' returns: how many have gone on, and when they recover.
fn returns(m: &Map, g: Guid) -> Vec<(u8, Option<Duration>)> {
    let now = m.now;
    m.caster(g).map_or_else(Vec::new, |c| {
        c.diminishing
            .iter()
            .map(|(_, n, at)| (*n, at.map(|t| t.saturating_duration_since(now))))
            .collect()
    })
}

/// Thistle Tea's stuns: shared across casters, eight seconds, then four,
/// then two, the player's client told each; the fourth is immune. A spell
/// of a stun and a root counts once.
#[tokio::test]
async fn stuns_halve_across_casters_and_the_fourth_is_immune() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    a.clear();
    let got = map
        .query(move |m| {
            let one = spawn(m, ga, BRUTE, 20, 5.0);
            let two = spawn(m, ga, BRUTE, 20, 5.0);
            let mut out = Vec::new();
            for (by, spell) in [(one, STUN_ROOT), (two, STUN), (one, STUN)] {
                spells::cast_triggered(m, by, spell, Some(ga));
                out.push(lasts(m, ga, spell));
            }
            spells::cast_triggered(m, two, STUN_2, Some(ga));
            out.push(lasts(m, ga, STUN_2));
            out
        })
        .await
        .unwrap();
    assert_eq!(got, [Some(8_000), Some(4_000), Some(2_000), None]);
    let mut told = Vec::new();
    for _ in 0..3 {
        told.push(
            a.wait_for("SMSG_UPDATE_AURA_DURATION", |m| match m {
                wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_UPDATE_AURA_DURATION(
                    d,
                ) => Some(d.aura_duration),
                _ => None,
            })
            .await,
        );
    }
    assert_eq!(told, [8_000, 4_000, 2_000]);
    assert_eq!(missed(&mut a, STUN_2).await, 7, "immune");
}

/// Thistle Tea's recovery: it waits for every stun on to go, then fifteen
/// seconds; a stun cast on one immune in that time doesn't put it off. A
/// dispel starts it as an ending does.
#[tokio::test]
async fn recovery_starts_as_the_last_stun_goes_and_immunity_doesnt_extend_it() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let one = spawn(m, ga, BRUTE, 20, 5.0);
            let two = spawn(m, ga, BRUTE, 20, 5.0);
            spells::cast_triggered(m, one, STUN, Some(ga));
            pass(m, 1_000);
            spells::cast_triggered(m, two, STUN_2, Some(ga));
            pass(m, 4_000);
            let one_left = returns(m, ga);
            pass(m, 3_000);
            let both_gone = returns(m, ga);
            pass(m, 14_999);
            spells::cast_triggered(m, one, STUN, Some(ga));
            let early = lasts(m, ga, STUN);
            auras::remove_where(m, ga, |_| true);
            // A third: immune until fifteen seconds after it, not after the
            // immune tries.
            pass(m, 15_000);
            for _ in 0..3 {
                spells::cast_triggered(m, one, STUN, Some(ga));
                auras::remove_where(m, ga, |_| true);
            }
            pass(m, 14_999);
            spells::cast_triggered(m, one, STUN, Some(ga));
            let tried = lasts(m, ga, STUN);
            pass(m, 1);
            spells::cast_triggered(m, one, STUN, Some(ga));
            let recovered = lasts(m, ga, STUN);
            // A dispel starts the recovery.
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            spells::cast_triggered(m, ga, STUN_HURTS, Some(brute));
            pass(m, 1_000);
            spells::cast_triggered(m, brute, DISPEL, Some(brute));
            (
                one_left,
                both_gone,
                early,
                tried,
                recovered,
                returns(m, brute),
            )
        })
        .await
        .unwrap();
    assert_eq!(got.0, [(2, None)], "no clock while one is on");
    assert_eq!(got.1, [(2, Some(Duration::from_secs(15)))]);
    assert_eq!(got.2, Some(2_000), "a millisecond early: the third");
    assert_eq!(got.3, None, "immune");
    assert_eq!(got.4, Some(8_000), "recovered on time");
    assert_eq!(got.5, [(1, Some(Duration::from_secs(15)))], "the dispel");
}

/// Thistle Tea's groups and exemptions: Impact's triggered stun and Kidney
/// Shot are groups of their own, each eight seconds after a stun; a stun
/// kept off by a mechanic immunity spends no step; death forgets them; a
/// stun refused for its returns still deals the damage beside it.
#[tokio::test]
async fn the_stun_groups_stand_apart_and_what_spends_no_step() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let one = spawn(m, ga, BRUTE, 20, 5.0);
            let mut groups = Vec::new();
            for s in [STUN, IMPACT, KIDNEY] {
                spells::cast_triggered(m, one, s, Some(ga));
                groups.push(lasts(m, ga, s));
            }
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            put(m, brute, brute, STUN_IMMUNE, &[0]).unwrap();
            spells::cast_triggered(m, ga, STUN, Some(brute));
            let immune = returns(m, brute);
            auras::remove_where(m, brute, |_| true);
            spells::cast_triggered(m, ga, STUN, Some(brute));
            auras::clear_on_death(m, brute);
            let dead = returns(m, brute);
            let other = spawn(m, ga, BRUTE, 20, 5.0);
            for _ in 0..3 {
                spells::cast_triggered(m, ga, STUN, Some(other));
                auras::remove_where(m, other, |_| true);
            }
            let before = health(m, other);
            spells::cast_triggered(m, ga, STUN_HURTS, Some(other));
            (
                groups,
                immune,
                dead,
                before - health(m, other),
                lasts(m, other, STUN_HURTS),
            )
        })
        .await
        .unwrap();
    assert_eq!(got.0, [Some(8_000); 3], "three groups");
    assert_eq!(got.1, [], "no step for the immune");
    assert_eq!(got.2, [], "death forgets");
    assert_eq!((got.3, got.4), (10, None), "the damage dealt, the stun not");
}
