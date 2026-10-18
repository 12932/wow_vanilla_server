//! Phase 5's auras, Thistle Tea's `Aura.HeartbeatTest` ported: which crowd
//! control breaks early, a player's break and a creature's rolls (a pet's
//! or a charmed one's both), the chance snapshotted as it went on, the
//! breaks diminishing returns bring sooner, a late roll, and a break
//! before the aura's own tick.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_EX3_ALWAYS_HIT, CUSTOM_SINGLE_TARGET_AURA, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::spells::{self, heartbeat::Heartbeat};
use crate::game::tests_p3_aura2::{add_spells, aura_spell, health, put};
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};

/// `SPELL_ATTR_HEARTBEAT_RESIST_CHECK`.
const HEARTBEAT: u32 = 0x4000_0000;

/// A stun of twenty seconds that checks; one of ten; one with no end; one
/// that doesn't check; a helpful one that would.
const CC: u32 = 96_700;
const CC_SHORT: u32 = 96_701;
const CC_ENDLESS: u32 = 96_702;
const CC_UNCHECKED: u32 = 96_703;
const BUFF: u32 = 96_704;
/// The Gnomish Mind Control Cap and Reckless Charge: they check without
/// the attribute.
const MIND_CONTROL_CAP: u32 = 13_181;
const RECKLESS_CHARGE: u32 = 13_327;
/// The stun with fear, a root, pacify and silence, confusion beside it.
const CC_WITH: u32 = 96_710;
/// A shadow stun, one that always hits, one aimed at an area.
const SHADOW_CC: u32 = 96_720;
const SURE_CC: u32 = 96_721;
const AREA_CC: u32 = 96_722;
/// 4% more spell hit; a hundred shadow resistance gone through; a victim's
/// 10% off shadow spells and 25% off areas.
const HIT: u32 = 96_723;
const PENETRATION: u32 = 96_724;
const DEFENSE: u32 = 96_725;
/// A stun with a tick of shadow damage at five seconds, single-target.
const CC_DOT: u32 = 96_730;

fn cc(id: u32, ms: Option<u32>) -> SpellTemplate {
    let mut s = aura_spell(id, ms, &[(AURA_MOD_STUN, 0, 0)]);
    s.mechanic = 12;
    s.school = 5;
    s.dmg_class = 1;
    s.attributes[0] = HEARTBEAT;
    s.effects[0].target_a = 6;
    s
}

fn data() -> GameData {
    let mut d = crate::game::tests_p3_aura2::data();
    let mut unchecked = cc(CC_UNCHECKED, Some(20_000));
    unchecked.attributes[0] = 0;
    let mut buff = aura_spell(BUFF, Some(20_000), &[(AURA_MOD_STAT, 0, 0)]);
    buff.attributes[0] = HEARTBEAT;
    buff.effects[0].base_points = 2;
    let mut all = vec![
        cc(CC, Some(20_000)),
        cc(CC_SHORT, Some(10_000)),
        cc(CC_ENDLESS, None),
        unchecked,
        buff,
    ];
    for id in [MIND_CONTROL_CAP, RECKLESS_CHARGE] {
        let mut s = cc(id, Some(20_000));
        s.attributes[0] = 0;
        all.push(s);
    }
    for (i, kind) in [
        AURA_MOD_FEAR,
        AURA_MOD_ROOT,
        AURA_MOD_PACIFY_SILENCE,
        AURA_MOD_CONFUSE,
    ]
    .into_iter()
    .enumerate()
    {
        let mut s = cc(CC_WITH + i as u32, Some(20_000));
        s.effects[1] = s.effects[0];
        s.effects[1].aura = kind;
        all.push(s);
    }
    let mut shadow = cc(SHADOW_CC, Some(20_000));
    shadow.school = 5;
    let mut sure = shadow.clone();
    sure.id = SURE_CC;
    sure.attributes[3] = ATTR_EX3_ALWAYS_HIT;
    let mut area = shadow.clone();
    area.id = AREA_CC;
    area.effects[0].target_a = 22;
    area.effects[0].target_b = 15;
    let hit = aura_spell(HIT, None, &[(AURA_MOD_SPELL_HIT_CHANCE, 0, 0)]);
    let pen = aura_spell(PENETRATION, None, &[(AURA_MOD_TARGET_RESISTANCE, 32, 0)]);
    let defense = aura_spell(
        DEFENSE,
        Some(60_000),
        &[
            (AURA_MOD_ATTACKER_SPELL_HIT_CHANCE, 32, 0),
            (AURA_MOD_AOE_AVOIDANCE, 0, 0),
        ],
    );
    let mut dot = cc(CC_DOT, Some(20_000));
    dot.custom_flags = CUSTOM_SINGLE_TARGET_AURA;
    dot.effects[1] = dot.effects[0];
    dot.effects[1].aura = AURA_PERIODIC_DAMAGE;
    dot.effects[1].amplitude = 5_000;
    all.extend([shadow, sure, area, hit, pen, defense, dot]);
    add_spells(&mut d, all);
    d
}

fn beat(m: &Map, g: Guid, spell: u32) -> Option<Heartbeat> {
    auras::auras(m, g)?
        .holders
        .iter()
        .find(|h| h.spell == spell)
        .and_then(|h| h.heartbeat)
}

/// Milliseconds from `from` to `at`.
fn ms(from: Instant, at: Option<Instant>) -> Option<u64> {
    at.map(|t| t.saturating_duration_since(from).as_millis() as u64)
}

/// When `g`'s `spell` breaks and next rolls, from `from`, and the chance it
/// holds by.
fn when(m: &Map, from: Instant, g: Guid, spell: u32) -> Option<(Option<u64>, Option<u64>, i32)> {
    beat(m, g, spell).map(|b| (ms(from, b.break_at), ms(from, b.next_check), b.hit_chance))
}

fn level(m: &mut Map, g: Guid, level: u32) {
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_LEVEL, level);
    }
}

/// Thistle Tea's exclusions: a helpful aura, one with no end, one of ten
/// seconds and one whose spell doesn't check never break early, on a
/// player or a creature. The engineering controls check on a player
/// without the attribute, not on a creature; fear, roots, pacify and
/// silence, and confusion don't roll on a creature, but break on a player.
#[tokio::test]
async fn crowd_control_breaks_early_only_when_harmful_long_and_checked() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            let brute = spawn(m, ga, BRUTE, 1, 5.0);
            let now = m.now;
            let mut none = Vec::new();
            for spell in [BUFF, CC_ENDLESS, CC_SHORT, CC_UNCHECKED] {
                for (by, on) in [(brute, ga), (ga, brute)] {
                    let _ = put(m, by, on, spell, &[3]);
                    none.push((
                        spell,
                        auras::auras(m, on).is_some_and(|a| a.has_spell(spell)),
                        beat(m, on, spell),
                    ));
                    auras::remove_spell(m, on, spell, None);
                }
            }
            let mut engineering = Vec::new();
            for spell in [MIND_CONTROL_CAP, RECKLESS_CHARGE] {
                let _ = put(m, brute, ga, spell, &[0]);
                let _ = put(m, ga, brute, spell, &[0]);
                engineering.push((when(m, now, ga, spell), when(m, now, brute, spell)));
                auras::remove_spell(m, ga, spell, None);
                auras::remove_spell(m, brute, spell, None);
            }
            let mut kinds = Vec::new();
            for i in 0..4 {
                let spell = CC_WITH + i;
                let _ = put(m, brute, ga, spell, &[0, 0]);
                let _ = put(m, ga, brute, spell, &[0, 0]);
                kinds.push((when(m, now, ga, spell), when(m, now, brute, spell)));
                auras::remove_spell(m, ga, spell, None);
                auras::remove_spell(m, brute, spell, None);
            }
            (none, engineering, kinds)
        })
        .await
        .unwrap();
    let (none, engineering, kinds) = got;
    for (spell, on, b) in none {
        assert!(on, "{spell} went on");
        assert_eq!(b, None, "{spell} doesn't break early");
    }
    for (player, creature) in engineering {
        assert_eq!(player.map(|p| (p.0, p.1)), Some((Some(12_000), None)));
        assert_eq!(creature, None);
    }
    for (player, creature) in kinds {
        assert_eq!(player.map(|p| (p.0, p.1)), Some((Some(12_000), None)));
        assert_eq!(creature, None, "not rolled on a creature");
    }
}

/// Thistle Tea's player-controlled creatures: a player's pet and a creature
/// a player charmed both break at the player's time and roll as a creature
/// does; a creature's pet only rolls.
#[tokio::test]
async fn a_players_pet_or_charmed_creature_both_breaks_and_rolls() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            let now = m.now;
            let pet = spawn(m, ga, BRUTE, 1, 5.0);
            let charmed = spawn(m, ga, BRUTE, 1, 5.0);
            let master = spawn(m, ga, BRUTE, 1, 5.0);
            let npc_pet = spawn(m, ga, BRUTE, 1, 5.0);
            m.creature_mut(pet).unwrap().owner = Some(ga);
            m.creature_mut(npc_pet).unwrap().owner = Some(master);
            if let Some(f) = m.unit_fields_mut(charmed) {
                f.set_guid(UNIT_FIELD_CHARMEDBY, ga);
            }
            let mut out = Vec::new();
            for g in [pet, charmed, npc_pet] {
                let _ = put(m, master, g, CC, &[0]);
                out.push(when(m, now, g, CC).map(|w| (w.0, w.1)));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(got[0], Some((Some(12_000), Some(5_000))), "a pet");
    assert_eq!(got[1], Some((Some(12_000), Some(5_000))), "a charmed one");
    assert_eq!(got[2], Some((None, Some(5_000))), "a creature's pet");
}

/// Thistle Tea's snapshot: the creature's roll holds by the spell's chance
/// as it went on, the caster's hit and a binary resistance in it (75%),
/// the caster's penetration through that resistance (99%, the cap); a
/// spell that always hits holds by all of it, and rolled, it holds. An
/// area's roll has the victim's change to the school's spells and its
/// avoidance of areas in it (96% less 10, less 25: 61%).
#[tokio::test]
async fn a_creatures_roll_holds_by_the_chance_snapshotted_as_it_went_on() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            level(m, ga, 60);
            let resists = spawn(m, ga, BRUTE, 60, 5.0);
            let avoids = spawn(m, ga, BRUTE, 60, 5.0);
            level(m, resists, 60);
            level(m, avoids, 60);
            if let Some(f) = m.unit_fields_mut(resists) {
                f.set_u32(UNIT_FIELD_RESISTANCES + 5, 100);
            }
            let _ = put(m, ga, ga, HIT, &[4]);
            let _ = put(m, ga, resists, SHADOW_CC, &[0]);
            let resisted = beat(m, resists, SHADOW_CC).map(|b| b.hit_chance);
            auras::remove_spell(m, resists, SHADOW_CC, None);
            let _ = put(m, ga, ga, PENETRATION, &[-100]);
            let _ = put(m, ga, resists, SHADOW_CC, &[0]);
            let pierced = beat(m, resists, SHADOW_CC).map(|b| b.hit_chance);
            auras::remove_spell(m, resists, SHADOW_CC, None);
            auras::remove_spell(m, ga, PENETRATION, None);
            let _ = put(m, ga, resists, SURE_CC, &[0]);
            let sure = beat(m, resists, SURE_CC).map(|b| b.hit_chance);
            m.spells.rolls = Some(true);
            m.now += Duration::from_secs(5);
            auras::update(m);
            let held = auras::auras(m, resists).is_some_and(|a| a.has_spell(SURE_CC));
            m.spells.rolls = Some(false);
            auras::remove_spell(m, ga, HIT, None);
            let _ = put(m, avoids, avoids, DEFENSE, &[-10, 25]);
            let _ = put(m, ga, avoids, AREA_CC, &[0]);
            let area = beat(m, avoids, AREA_CC).map(|b| b.hit_chance);
            (resisted, pierced, sure, held, area)
        })
        .await
        .unwrap();
    assert_eq!(
        got,
        (Some(7_500), Some(9_900), Some(10_000), true, Some(6_100))
    );
}

/// Thistle Tea's diminishing returns: a player's stun refreshed keeps its
/// drawn delay and breaks at half of it from the second, a quarter from
/// the third (with the aura's own end); then it breaks, and the returns
/// recover fifteen seconds after.
#[tokio::test]
async fn diminishing_returns_bring_a_players_break_sooner() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            let brute = spawn(m, ga, BRUTE, 1, 5.0);
            let start = m.now;
            let mut out = Vec::new();
            for _ in 0..3 {
                spells::cast_triggered(m, brute, CC, Some(ga));
                let h = auras::auras(m, ga)
                    .and_then(|a| a.holders.iter().find(|h| h.spell == CC).cloned());
                out.push(h.map(|h| {
                    (
                        ms(start, h.heartbeat.and_then(|b| b.break_at)),
                        ms(start, h.expires),
                    )
                }));
                m.now += Duration::from_secs(1);
            }
            // At five seconds in, it breaks.
            m.now = start + Duration::from_millis(4_999);
            auras::update(m);
            let early = auras::auras(m, ga).is_some_and(|a| a.has_spell(CC));
            m.now = start + Duration::from_secs(5);
            auras::update(m);
            let broken = !auras::auras(m, ga).is_some_and(|a| a.has_spell(CC));
            let stunned = m
                .unit_fields(ga)
                .is_some_and(|f| f.get_u32(UNIT_FIELD_FLAGS) & 0x4_0000 != 0);
            let returns = m.caster(ga).map(|c| {
                c.diminishing
                    .iter()
                    .map(|(_, n, at)| (*n, at.map(|t| ms(start, Some(t)))))
                    .collect::<Vec<_>>()
            });
            (out, early, broken, stunned, returns)
        })
        .await
        .unwrap();
    let (out, early, broken, stunned, returns) = got;
    assert_eq!(
        out,
        vec![
            Some((Some(12_000), Some(20_000))),
            Some((Some(7_000), Some(11_000))),
            Some((Some(5_000), Some(7_000))),
        ]
    );
    assert!(
        early && broken && !stunned,
        "broken at five seconds, not before"
    );
    assert_eq!(returns, Some(vec![(3, Some(Some(20_000)))]));
}

/// Thistle Tea's late tick: a creature's roll due at five seconds, looked
/// at only at twelve, rolls once and next at fifteen; failed then, the
/// stun is gone and nothing more is due.
#[tokio::test]
async fn a_late_roll_rolls_once_and_moves_past_now() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            let brute = spawn(m, ga, BRUTE, 1, 5.0);
            let start = m.now;
            let _ = put(m, ga, brute, CC, &[0]);
            m.now = start + Duration::from_secs(12);
            auras::update(m);
            let next = beat(m, brute, CC).and_then(|b| ms(start, b.next_check));
            // Every roll fails from here.
            if let Some(a) = auras::auras_mut(m, brute)
                && let Some(h) = a.holders.iter_mut().find(|h| h.spell == CC)
                && let Some(b) = h.heartbeat.as_mut()
            {
                b.hit_chance = 0;
            }
            m.now = start + Duration::from_millis(14_999);
            auras::update(m);
            let held = auras::auras(m, brute).is_some_and(|a| a.has_spell(CC));
            m.now = start + Duration::from_secs(15);
            auras::update(m);
            let gone = !auras::auras(m, brute).is_some_and(|a| a.has_spell(CC));
            (next, held, gone)
        })
        .await
        .unwrap();
    assert_eq!(got, (Some(15_000), true, true));
}

/// Thistle Tea's order: a stun that fails its roll at five seconds is gone
/// before its tick of damage due then, and its single-target claim with
/// it. Dispelled or its victim dead, nothing of it is left to break.
#[tokio::test]
async fn a_break_comes_before_the_auras_own_tick_and_nothing_outlives_the_aura() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            let brute = spawn(m, ga, BRUTE, 1, 5.0);
            let start = m.now;
            let _ = put(m, ga, brute, CC_DOT, &[0, 10]);
            if let Some(a) = auras::auras_mut(m, brute)
                && let Some(h) = a.holders.iter_mut().find(|h| h.spell == CC_DOT)
                && let Some(b) = h.heartbeat.as_mut()
            {
                b.hit_chance = 0;
            }
            let before = health(m, brute);
            m.now = start + Duration::from_secs(5);
            auras::update(m);
            let gone = !auras::auras(m, brute).is_some_and(|a| a.has_spell(CC_DOT));
            let hurt = before - health(m, brute);
            // A player's stun dispelled, another's on one who dies.
            let other = spawn(m, ga, BRUTE, 1, 5.0);
            let _ = put(m, brute, ga, CC, &[0]);
            auras::remove_spell(m, ga, CC, None);
            let _ = put(m, ga, other, CC, &[0]);
            crate::game::map::combat::deal_damage(m, ga, other, 1_000_000);
            m.now = start + Duration::from_secs(20);
            auras::update(m);
            let left = [ga, other].iter().any(|g| {
                auras::auras(m, *g).is_some_and(|a| a.holders.iter().any(|h| h.heartbeat.is_some()))
            });
            (gone, hurt, left)
        })
        .await
        .unwrap();
    assert_eq!(got, (true, 0, false));
}
