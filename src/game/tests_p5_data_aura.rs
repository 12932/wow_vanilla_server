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

/// What a charmed brute offers: two spells and a charm of its own,
/// each slot always rolled.
const CHARM_ONE: u32 = 96_740;
const CHARM_TWO: u32 = 96_741;
const CHARM_CHARM: u32 = 96_742;

fn charm_data() -> GameData {
    let mut d = data();
    add_spells(
        &mut d,
        [
            aura_spell(CHARM_ONE, Some(1_000), &[(AURA_DUMMY, 0, 0)]),
            aura_spell(CHARM_TWO, Some(1_000), &[(AURA_DUMMY, 0, 0)]),
            aura_spell(CHARM_CHARM, Some(1_000), &[(AURA_MOD_CHARM, 0, 0)]),
        ],
    );
    let slot = |slot, spell| crate::game::data::pets::CharmSpell {
        slot,
        spell,
        availability: 100.0,
        cooldown_min_ms: 0,
        cooldown_max_ms: 0,
    };
    d.pets.charm_spells.insert(
        BRUTE,
        vec![slot(0, CHARM_ONE), slot(1, CHARM_CHARM), slot(2, CHARM_TWO)],
    );
    d
}

/// Thistle Tea's control: a creature taken on the move stops where it is;
/// it offers its rolled abilities in their slots' order, never a charm;
/// let go, it shows no player's control nor a pet's fight, and fights on
/// (its master, as mangos's creature turns on its charmer).
#[tokio::test]
async fn a_creature_taken_stops_offers_its_rolled_spells_and_let_go_is_its_own() {
    let (_realm, _a, ga, map) = setup_with(charm_data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 1, 5.0);
            let at = m.creature(brute).unwrap().position;
            crate::game::map::scripts::move_to(
                m,
                brute,
                Vector3d {
                    x: at.x + 20.0,
                    ..at
                },
                false,
            );
            let walking = matches!(
                m.creature(brute).unwrap().motion,
                crate::game::map::creature::Motion::Moving(_)
            );
            crate::game::map::pets::charmed(m, ga, brute, false);
            let stopped = matches!(
                m.creature(brute).unwrap().motion,
                crate::game::map::creature::Motion::Idle { .. }
            );
            let offered = crate::game::map::pets::state(m, brute).map(|s| s.spells.clone());
            crate::game::map::pets::uncharmed(m, ga, brute);
            let flags = m
                .unit_fields(brute)
                .map_or(0, |f| f.get_u32(UNIT_FIELD_FLAGS));
            let fights = m.creature(brute).map(|c| c.combat.victim);
            (walking, stopped, offered, flags, fights)
        })
        .await
        .unwrap();
    let (walking, stopped, offered, flags, fights) = got;
    assert!(walking, "on its way");
    assert!(stopped, "stopped where it was");
    assert_eq!(offered, Some(vec![CHARM_ONE, CHARM_TWO]));
    assert_eq!(flags & 0x808, 0, "no player's control nor pet's fight");
    assert_eq!(fights, Some(ga), "it turns on its master");
    assert_ne!(flags & 0x8_0000, 0, "and fights on");
}

/// Thistle Tea's mana user: every magic school's spells cost 100 less,
/// fire's 10 more twice over (two stacks) and half again; a rogue's talent
/// of half off its first family flag; spells of 200 mana in fire, of 20
/// and a tenth of base mana in frost, of 50 in frost, of all the mana, of
/// all the health, and of 150 energy in frost.
const COST_LESS: u32 = 96_750;
const FIRE_MORE: u32 = 96_751;
const FIRE_SHARE: u32 = 96_752;
const HALF_OFF: u32 = 96_753;
const FIRE_SPELL: u32 = 96_755;
const FIRE_FAMILY: u32 = 96_756;
const FIRE_OTHER: u32 = 96_757;
const FROST_SHARE: u32 = 96_758;
const FROST_CHEAP: u32 = 96_759;
const ALL_MANA: u32 = 96_760;
const ALL_HEALTH: u32 = 96_761;
const FROST_ENERGY: u32 = 96_762;

fn cost_data() -> GameData {
    let mut d = data();
    let mut more = aura_spell(
        FIRE_MORE,
        Some(10_000),
        &[(AURA_MOD_POWER_COST_SCHOOL, 4, 0)],
    );
    more.stack_amount = 2;
    let mut half = aura_spell(HALF_OFF, None, &[(AURA_ADD_PCT_MODIFIER, 14, 0)]);
    half.family_name = 8;
    half.effects[0].item_type = 1;
    let costs = |id: u32, school: u32, mana: u32, power: u32| {
        let mut s = SpellTemplate {
            id,
            name: format!("Costs {id}"),
            school,
            mana_cost: mana,
            power_type: power,
            equipped_item_class: -1,
            range_index: 1,
            ..Default::default()
        };
        // A dummy effect: nothing that works the caster's stats out again.
        s.effects[0] = crate::game::data::spells::SpellEffect {
            effect: 3,
            target_a: 1,
            base_dice: 1,
            ..Default::default()
        };
        s
    };
    let mut family = costs(FIRE_FAMILY, 2, 200, 0);
    family.family_name = 8;
    family.family_flags = 1;
    let mut other = family.clone();
    other.id = FIRE_OTHER;
    other.family_flags = 2;
    let mut share = costs(FROST_SHARE, 4, 20, 0);
    share.mana_cost_percentage = 10;
    let mut all_mana = costs(ALL_MANA, 2, 200, 0);
    all_mana.attributes[1] = crate::game::data::spells::ATTR_EX_USE_ALL_MANA;
    let mut all_health = all_mana.clone();
    all_health.id = ALL_HEALTH;
    all_health.power_type = u32::MAX - 1;
    add_spells(
        &mut d,
        [
            aura_spell(
                COST_LESS,
                Some(5_000),
                &[(AURA_MOD_POWER_COST_SCHOOL, 126, 0)],
            ),
            more,
            aura_spell(
                FIRE_SHARE,
                Some(10_000),
                &[(AURA_MOD_POWER_COST_SCHOOL_PCT, 4, 0)],
            ),
            half,
            costs(FIRE_SPELL, 2, 200, 0),
            family,
            other,
            share,
            costs(FROST_CHEAP, 4, 50, 0),
            all_mana,
            all_health,
            costs(FROST_ENERGY, 4, 150, 3),
        ],
    );
    d
}

/// Thistle Tea's costs, in mangos's order: the schools' flat changes, the
/// caster's spell modifiers, then the schools' shares (200 fire: 180; half
/// off by the talent: 90; another flag: 180); a share of base mana before
/// the flat change (20 and 100, less 100: 20); one cancelled and the rest
/// run out, the cost and the client's fields back (330, then 200 and
/// nothing shown). A free spell spends nothing and starts no five-second
/// rule; a paid one spends what it costs and does. One of all the mana or
/// health takes it all, nothing changing it; energy is changed by school
/// too, and starts no rule.
#[tokio::test]
async fn costs_go_by_school_then_spell_then_school_share() {
    let (_realm, _a, ga, map) = setup_with(cost_data(), Class::Priest).await;
    let got = map
        .query(move |m| {
            // Thistle Tea's mana user's pools (the stats worked out again
            // as auras change put them back).
            let pools = |m: &mut Map| {
                if let Some(f) = m.unit_fields_mut(ga) {
                    f.set_u32(UNIT_FIELD_HEALTH, 500);
                    f.set_u32(UNIT_FIELD_MAXHEALTH, 500);
                    f.set_u32(UNIT_FIELD_MAXPOWER1, 2_000);
                    f.set_u32(UNIT_FIELD_POWER1, 1_000);
                    f.set_u32(UNIT_FIELD_BASE_MANA, 1_000);
                    f.set_u32(UNIT_FIELD_MAXPOWER1 + 3, 100);
                    f.set_u32(UNIT_FIELD_POWER1 + 3, 100);
                }
            };
            let _ = put(m, ga, ga, COST_LESS, &[-100]);
            let _ = put(m, ga, ga, FIRE_MORE, &[10]);
            let _ = put(m, ga, ga, FIRE_MORE, &[10]);
            let _ = put(m, ga, ga, FIRE_SHARE, &[50]);
            let cost = |m: &Map, id: u32| {
                let s = m.realm.data.spells.get(id).unwrap().clone();
                let mods = spells::mods::for_spell(m, ga, &s);
                spells::spell_cost(m, ga, &s, &mods)
            };
            let shown = |m: &Map| {
                let f = m.unit_fields(ga).unwrap();
                (
                    (0..7)
                        .map(|i| f.get_u32(UNIT_FIELD_POWER_COST_MODIFIER + i) as i32)
                        .collect::<Vec<_>>(),
                    (0..7)
                        .map(|i| f.get_f32(UNIT_FIELD_POWER_COST_MULTIPLIER + i))
                        .collect::<Vec<_>>(),
                )
            };
            let fields = shown(m);
            let plain = cost(m, FIRE_SPELL);
            let _ = put(m, ga, ga, HALF_OFF, &[-50]);
            let talent = (cost(m, FIRE_FAMILY), cost(m, FIRE_OTHER));
            auras::remove_spell(m, ga, HALF_OFF, None);
            pools(m);
            let share = cost(m, FROST_SHARE);
            let free = cost(m, FROST_CHEAP);
            pools(m);
            let all = (cost(m, ALL_MANA), cost(m, ALL_HEALTH));
            // Paying.
            pools(m);
            let before = m.players[&ga].mana_spent_at;
            spells::cast(m, ga, FROST_CHEAP, spells::Targets::default(), None);
            let after_free = (
                m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_POWER1),
                m.players[&ga].mana_spent_at == before,
            );
            crate::game::tests_p3_spells_kit::fresh(m, ga);
            spells::cast(m, ga, FIRE_SPELL, spells::Targets::default(), None);
            let paid = (
                m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_POWER1),
                m.players[&ga].mana_spent_at == Some(m.now),
            );
            crate::game::tests_p3_spells_kit::fresh(m, ga);
            m.players.get_mut(&ga).unwrap().mana_spent_at = None;
            pools(m);
            spells::cast(m, ga, FROST_ENERGY, spells::Targets::default(), None);
            let energy = (
                m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_POWER1 + 3),
                m.players[&ga].mana_spent_at,
            );
            crate::game::tests_p3_spells_kit::fresh(m, ga);
            pools(m);
            spells::cast(m, ga, ALL_MANA, spells::Targets::default(), None);
            let drained = m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_POWER1);
            // One cancelled, then the rest run out.
            auras::remove_spell(m, ga, COST_LESS, None);
            let cancelled = cost(m, FIRE_SPELL);
            m.now += Duration::from_secs(10);
            auras::update(m);
            let expired = cost(m, FIRE_SPELL);
            let cleared = shown(m);
            (
                fields, plain, talent, share, free, all, after_free, paid, energy, drained,
                cancelled, expired, cleared,
            )
        })
        .await
        .unwrap();
    let (
        fields,
        plain,
        talent,
        share,
        free,
        all,
        after_free,
        paid,
        energy,
        drained,
        cancelled,
        expired,
        cleared,
    ) = got;
    assert_eq!(fields.0, vec![0, -100, -80, -100, -100, -100, -100]);
    assert_eq!(fields.1, vec![0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0]);
    assert_eq!(plain, 180);
    assert_eq!(talent, (90, 180));
    assert_eq!(share, 20);
    assert_eq!(free, 0);
    assert_eq!(all, (1_000, 500), "all of it, nothing changing it");
    assert_eq!(after_free, (1_000, true), "free: nothing spent, no rule");
    assert_eq!(paid, (820, true));
    assert_eq!(energy, (50, None), "energy less 100, no rule");
    assert_eq!(drained, 0);
    assert_eq!((cancelled, expired), (330, 200));
    assert_eq!(cleared, (vec![0; 7], vec![0.0; 7]));
}

/// A buff, permanent and not; stuns of each family's flags that count and
/// of the ranked lists' first ranks (Fireball, Polymorph, Curse of
/// Tongues, Corruption, Frostbolt, Death Coil).
const WORTH_BUFF: u32 = 96_770;
const WORTH_FOREVER: u32 = 96_771;
const WORTH_FAMILY: u32 = 96_772;
const WORTH_FAMILIES: [(u32, u64, u8); 9] = [
    (8, 0x8, 1),
    (8, 0x100, 2),
    (4, 0x20, 2),
    (7, 0x1000, 2),
    (4, 0x4000, 3),
    (8, 0x20_0000, 3),
    (7, 0x2000, 3),
    (4, 0x2_0000, 4),
    (7, 0x8, 4),
];
const WORTH_RANKS: [(u32, u8); 6] = [(133, 0), (118, 0), (1714, 1), (172, 2), (116, 3), (6136, 4)];

fn worth_data() -> GameData {
    let mut d = data();
    let mut all = vec![
        aura_spell(WORTH_BUFF, Some(10_000), &[(AURA_MOD_STAT, 0, 0)]),
        aura_spell(WORTH_FOREVER, None, &[(AURA_MOD_STAT, 0, 0)]),
    ];
    for (i, (family, flags, _)) in WORTH_FAMILIES.iter().enumerate() {
        let mut s = cc(WORTH_FAMILY + i as u32, Some(10_000));
        s.attributes[0] = 0;
        s.family_name = *family;
        s.family_flags = *flags;
        all.push(s);
    }
    for (id, _) in WORTH_RANKS {
        let mut s = cc(id, Some(10_000));
        s.attributes[0] = 0;
        all.push(s);
    }
    add_spells(&mut d, all);
    d
}

/// Thistle Tea's priorities: a buff of one's own is worth least, then one
/// from an item used, another's, and a permanent one most; a debuff's
/// family flags come before its rank and its kind (a stun, worth 4 by
/// kind, worth what its family says), and a ranked debuff keeps its rank's
/// worth when triggered.
#[tokio::test]
async fn an_auras_worth_goes_by_its_source_family_and_rank() {
    let (_realm, _a, ga, map) = setup_with(worth_data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let data = m.realm.data.clone();
            let brute = spawn(m, ga, BRUTE, 1, 5.0);
            let worth = |m: &Map, g: Guid, spell: u32| {
                auras::auras(m, g)
                    .and_then(|a| a.holders.iter().find(|h| h.spell == spell).cloned())
                    .map(|h| spells::capacity::worth(&data, g, &h))
            };
            let mut buffs = Vec::new();
            let _ = put(m, ga, ga, WORTH_BUFF, &[1]);
            buffs.push(worth(m, ga, WORTH_BUFF));
            auras::remove_spell(m, ga, WORTH_BUFF, None);
            let s = m.realm.data.spells.get(WORTH_BUFF).cloned().unwrap();
            let _ = auras::apply_spell(
                m,
                ga,
                ga,
                &s,
                &[(0, 1)],
                Context {
                    item: Some(Guid::new(0x4000_0000_0000_0012)),
                    ..Default::default()
                },
            );
            buffs.push(worth(m, ga, WORTH_BUFF));
            auras::remove_spell(m, ga, WORTH_BUFF, None);
            let _ = put(m, brute, ga, WORTH_BUFF, &[1]);
            buffs.push(worth(m, ga, WORTH_BUFF));
            let _ = put(m, ga, ga, WORTH_FOREVER, &[1]);
            buffs.push(worth(m, ga, WORTH_FOREVER));
            let mut families = Vec::new();
            for i in 0..WORTH_FAMILIES.len() as u32 {
                let _ = put(m, ga, brute, WORTH_FAMILY + i, &[0]);
                families.push(worth(m, brute, WORTH_FAMILY + i));
                auras::remove_spell(m, brute, WORTH_FAMILY + i, None);
            }
            let mut ranks = Vec::new();
            for (id, _) in WORTH_RANKS {
                let s = m.realm.data.spells.get(id).cloned().unwrap();
                let _ = auras::apply_spell(
                    m,
                    ga,
                    brute,
                    &s,
                    &[(0, 0)],
                    Context {
                        triggered: true,
                        ..Default::default()
                    },
                );
                ranks.push(worth(m, brute, id));
                auras::remove_spell(m, brute, id, None);
            }
            (buffs, families, ranks)
        })
        .await
        .unwrap();
    let (buffs, families, ranks) = got;
    assert_eq!(buffs, vec![Some(0), Some(1), Some(2), Some(3)]);
    assert_eq!(
        families,
        WORTH_FAMILIES.iter().map(|f| Some(f.2)).collect::<Vec<_>>()
    );
    assert_eq!(
        ranks,
        WORTH_RANKS.iter().map(|r| Some(r.1)).collect::<Vec<_>>()
    );
}

/// A rogue's proc of 10%, six a minute out and 7% in by its rule; talents
/// of 5 more, half again, 150 less and a hundred more to its chance; a
/// main hand of two seconds, an off hand of one, a bow of three.
const PROC_AURA: u32 = 96_780;
const CHANCE_MORE: u32 = 96_781;
const CHANCE_HALF: u32 = 96_782;
const CHANCE_LESS: u32 = 96_783;
const CHANCE_SURE: u32 = 96_784;
const MAIN: u32 = 96_785;
const OFF: u32 = 96_786;
const BOW: u32 = 96_787;

fn chance_data() -> GameData {
    let mut d = data();
    let mut aura = aura_spell(PROC_AURA, None, &[(AURA_DUMMY, 0, 0)]);
    aura.family_name = 8;
    aura.family_flags = 1;
    aura.proc_chance = 10;
    let talent = |id: u32, kind: u32| {
        let mut s = aura_spell(id, None, &[(kind, 18, 0)]);
        s.family_name = 8;
        s.effects[0].item_type = 1;
        s
    };
    add_spells(
        &mut d,
        [
            aura,
            talent(CHANCE_MORE, AURA_ADD_FLAT_MODIFIER),
            talent(CHANCE_HALF, AURA_ADD_PCT_MODIFIER),
            talent(CHANCE_LESS, AURA_ADD_FLAT_MODIFIER),
            talent(CHANCE_SURE, AURA_ADD_FLAT_MODIFIER),
        ],
    );
    d.spells.proc_events.insert(
        PROC_AURA,
        crate::game::data::spells::ProcEvent {
            ppm: 6.0,
            chance: 7.0,
            ..Default::default()
        },
    );
    for (entry, subclass, inventory_type, delay) in
        [(MAIN, 7, 13, 2000), (OFF, 7, 13, 1000), (BOW, 2, 15, 3000)]
    {
        d.items.insert(
            entry,
            crate::game::data::items::ItemTemplate {
                entry,
                class: 2,
                subclass,
                name: format!("Weapon {entry}"),
                display_id: 1000,
                inventory_type,
                delay,
                stackable: 1,
                ..Default::default()
            },
        );
    }
    d
}

/// Thistle Tea's proc chances: six a minute out is 20% by a main hand of
/// two seconds, 10% by an off hand of one, 30% by a bow of three, 10% in
/// cat form and 25% in bear or dire bear form; in, the rule's 7% (the
/// spell's 10% without it). The bearer's modifiers go on after, flat then
/// percent ((20 + 5) × 1.5, (7 + 5) × 1.5), uncapped first (60 a minute,
/// 200%, less 150: 50%), and as they are when it rolls; a pet's are its
/// master's.
#[tokio::test]
async fn a_procs_chance_goes_by_the_hand_or_form_then_the_bearers_modifiers() {
    use crate::game::map::combat::Hand;
    use crate::game::map::procs;
    let (_realm, _a, ga, map) = setup_with(chance_data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            crate::game::tests_p3_combat::equip(m, ga, MAIN, 15);
            crate::game::tests_p3_combat::equip(m, ga, OFF, 16);
            crate::game::tests_p3_combat::equip(m, ga, BOW, 17);
            let s = m.realm.data.spells.get(PROC_AURA).cloned().unwrap();
            let chance =
                |m: &Map, g: Guid, hand: Hand, out: bool| procs::chance(m, g, &s, hand, out);
            let hands = (
                chance(m, ga, Hand::Main, true),
                chance(m, ga, Hand::Off, true),
                chance(m, ga, Hand::Ranged, true),
                chance(m, ga, Hand::Main, false),
            );
            let mut forms = Vec::new();
            for form in [FORM_CAT, FORM_BEAR, FORM_DIREBEAR] {
                if let Some(f) = m.unit_fields_mut(ga) {
                    f.set_byte(UNIT_FIELD_BYTES_1, 2, form);
                }
                forms.push(chance(m, ga, Hand::Main, true));
            }
            if let Some(f) = m.unit_fields_mut(ga) {
                f.set_byte(UNIT_FIELD_BYTES_1, 2, 0);
            }
            let _ = put(m, ga, ga, CHANCE_MORE, &[5]);
            let _ = put(m, ga, ga, CHANCE_HALF, &[50]);
            let modified = (
                chance(m, ga, Hand::Main, true),
                chance(m, ga, Hand::Main, false),
            );
            auras::remove_spell(m, ga, CHANCE_MORE, None);
            auras::remove_spell(m, ga, CHANCE_HALF, None);
            // Sixty a minute: 200%, less 150.
            let uncapped = procs::ppm_chance(60.0, 2000);
            let _ = put(m, ga, ga, CHANCE_LESS, &[-150]);
            let less = m.realm.data.spells.get(PROC_AURA).map(|s| {
                crate::game::map::spells::mods::for_spell(m, ga, s).apply(
                    crate::game::map::spells::mods::OP_CHANCE_OF_SUCCESS,
                    uncapped,
                )
            });
            auras::remove_spell(m, ga, CHANCE_LESS, None);
            // A pet's are its master's, as they are now.
            let pet = spawn(m, ga, BRUTE, 1, 3.0);
            m.creature_mut(pet).unwrap().owner = Some(ga);
            m.creature_mut(pet).unwrap().pet =
                Some(Box::new(crate::game::map::pets::PetState::new(
                    crate::game::map::pets::Kind::Hunter,
                    ga,
                    m.now,
                )));
            let _ = put(m, ga, ga, CHANCE_SURE, &[100]);
            let pets = chance(m, pet, Hand::Main, false);
            auras::remove_spell(m, ga, CHANCE_SURE, None);
            let pets_after = chance(m, pet, Hand::Main, false);
            (hands, forms, modified, uncapped, less, pets, pets_after)
        })
        .await
        .unwrap();
    let (hands, forms, modified, uncapped, less, pets, pets_after) = got;
    assert_eq!(hands, (20.0, 10.0, 30.0, 7.0));
    assert_eq!(forms, vec![10.0, 25.0, 25.0]);
    assert_eq!(modified, (37.5, 18.0));
    assert_eq!((uncapped, less), (200.0, Some(50.0)));
    assert_eq!(
        (pets, pets_after),
        (107.0, 7.0),
        "the master's, as they are"
    );
}

/// Procs that cast a mark on their bearer, of two charges: one of twenty a
/// minute and five seconds' cooldown on ranged attacks; one of 10% in by
/// its rule on blows and spells taken; one of no chance of its own on
/// kills, blows and ranged attacks, a rogue's for the talents.
const RANGED_PROC: u32 = 96_790;
const TAKEN_PROC: u32 = 96_791;
const TALENTED_PROC: u32 = 96_792;
const PROC_MARK: u32 = 96_793;

fn charged_data() -> GameData {
    use crate::game::map::procs::*;
    let mut d = chance_data();
    let charged = |id: u32, flags: u32, chance: u32| {
        let mut s = crate::game::tests_p3_aura2::proc_spell(id, flags, PROC_MARK);
        s.proc_chance = chance;
        s.proc_charges = 2;
        s.family_name = 8;
        s.family_flags = 1;
        s
    };
    add_spells(
        &mut d,
        [
            aura_spell(PROC_MARK, Some(30_000), &[(AURA_DUMMY, 0, 0)]),
            charged(RANGED_PROC, PROC_DONE_RANGED, 0),
            charged(TAKEN_PROC, PROC_TAKEN_MELEE | PROC_TAKEN_NEGATIVE_SPELL, 0),
            charged(
                TALENTED_PROC,
                PROC_KILL | PROC_DONE_MELEE | PROC_DONE_RANGED,
                0,
            ),
        ],
    );
    d.spells.proc_events.insert(
        RANGED_PROC,
        crate::game::data::spells::ProcEvent {
            ppm: 20.0,
            cooldown: 5_000,
            ..Default::default()
        },
    );
    d.spells.proc_events.insert(
        TAKEN_PROC,
        crate::game::data::spells::ProcEvent {
            ppm: 0.01,
            chance: 10.0,
            cooldown: 5_000,
            ..Default::default()
        },
    );
    d
}

/// Thistle Tea's charges and cooldowns: a ranged attack procs twenty a
/// minute by the bow's three seconds (sure), a charge spent and five
/// seconds to wait; within them it doesn't, after them it does and the
/// aura is spent. Taken, a rate a minute counts not: the rule's 10% with
/// the bearer's live modifiers (90 more: sure; 10 less: never), for a blow
/// and a spell. A talent's chance gone, its proc of none goes off no more
/// and spends no charge.
#[tokio::test]
async fn procs_spend_their_charges_wait_their_cooldowns_and_follow_live_talents() {
    use crate::game::map::procs::{self, *};
    let (_realm, _a, ga, map) = setup_with(charged_data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            crate::game::tests_p3_combat::equip(m, ga, BOW, 17);
            let brute = spawn(m, ga, BRUTE, 1, 5.0);
            let marked = |m: &mut Map| {
                let had = auras::auras(m, ga).is_some_and(|a| a.has_spell(PROC_MARK));
                auras::remove_spell(m, ga, PROC_MARK, None);
                had
            };
            let charges = |m: &Map, spell: u32| {
                auras::auras(m, ga).and_then(|a| {
                    a.holders
                        .iter()
                        .find(|h| h.spell == spell)
                        .map(|h| h.charges)
                })
            };
            let event = |actor, target, actor_flags, target_flags, spell| Event {
                actor,
                target,
                actor_flags,
                target_flags,
                ex: EX_NORMAL_HIT,
                spell,
                damage: 30,
                school: 1,
                ranged: actor_flags & PROC_DONE_RANGED != 0,
                ..Default::default()
            };
            // Ranged.
            let _ = put(m, ga, ga, RANGED_PROC, &[0]);
            let shot = event(ga, brute, PROC_DONE_RANGED, 0, None);
            procs::trigger(m, shot);
            let first = (marked(m), charges(m, RANGED_PROC));
            m.now += Duration::from_secs(1);
            procs::trigger(m, shot);
            let cooling = marked(m);
            m.now += Duration::from_secs(4);
            procs::trigger(m, shot);
            let spent = (marked(m), charges(m, RANGED_PROC));
            // Taken.
            let mut taken = Vec::new();
            for (flags, spell) in [
                (PROC_TAKEN_MELEE, None),
                (PROC_TAKEN_NEGATIVE_SPELL, Some(133)),
            ] {
                let hit = event(brute, ga, 0, flags, spell);
                let _ = put(m, ga, ga, TAKEN_PROC, &[0]);
                let _ = put(m, ga, ga, CHANCE_SURE, &[90]);
                procs::trigger(m, hit);
                let sure = (marked(m), charges(m, TAKEN_PROC));
                auras::remove_spell(m, ga, CHANCE_SURE, None);
                auras::remove_spell(m, ga, TAKEN_PROC, None);
                let _ = put(m, ga, ga, TAKEN_PROC, &[0]);
                let _ = put(m, ga, ga, CHANCE_LESS, &[-10]);
                procs::trigger(m, hit);
                let never = (marked(m), charges(m, TAKEN_PROC));
                auras::remove_spell(m, ga, CHANCE_LESS, None);
                auras::remove_spell(m, ga, TAKEN_PROC, None);
                taken.push((sure, never));
            }
            // A talent's chance.
            let mut talented = Vec::new();
            for flags in [PROC_KILL, PROC_DONE_MELEE, PROC_DONE_RANGED] {
                let e = event(ga, brute, flags, 0, None);
                let _ = put(m, ga, ga, TALENTED_PROC, &[0]);
                let _ = put(m, ga, ga, CHANCE_SURE, &[100]);
                procs::trigger(m, e);
                let with = (marked(m), charges(m, TALENTED_PROC));
                auras::remove_spell(m, ga, CHANCE_SURE, None);
                procs::trigger(m, e);
                let without = (marked(m), charges(m, TALENTED_PROC));
                auras::remove_spell(m, ga, TALENTED_PROC, None);
                talented.push((with, without));
            }
            (first, cooling, spent, taken, talented)
        })
        .await
        .unwrap();
    let (first, cooling, spent, taken, talented) = got;
    assert_eq!(first, (true, Some(1)), "sure, a charge spent");
    assert!(!cooling, "within its cooldown");
    assert_eq!(spent, (true, None), "after it, the last charge");
    for (sure, never) in taken {
        assert_eq!(sure, (true, Some(1)));
        assert_eq!(never, (false, Some(2)));
    }
    for (with, without) in talented {
        assert_eq!(with, (true, Some(1)));
        assert_eq!(without, (false, Some(1)), "no proc, no charge");
    }
}

/// Health of 100 more; Last Stand's temporary health (300); bear form with
/// 400 more health.
const MORE_HEALTH: u32 = 96_795;
const LAST_STAND: u32 = 12_976;
const BEAR: u32 = 96_796;

fn health_data() -> GameData {
    let mut d = data();
    let mut bear = aura_spell(
        BEAR,
        None,
        &[
            (AURA_MOD_SHAPESHIFT, FORM_BEAR as i32, 0),
            (AURA_MOD_INCREASE_HEALTH, 0, 0),
        ],
    );
    bear.effects[1].base_points = 399;
    add_spells(
        &mut d,
        [
            aura_spell(
                MORE_HEALTH,
                Some(60_000),
                &[(AURA_MOD_INCREASE_HEALTH, 0, 0)],
            ),
            aura_spell(
                LAST_STAND,
                Some(20_000),
                &[(AURA_MOD_INCREASE_HEALTH, 0, 0)],
            ),
            bear,
        ],
    );
    d
}

/// Thistle Tea's health through auras: more maximum health heals nothing,
/// and its going takes none; Last Stand's health comes and goes with it,
/// leaving one injured below it at one, and the dead dead; bear form keeps
/// the share of health, ten times over without drifting.
#[tokio::test]
async fn health_follows_temporary_health_and_bear_form_but_not_a_buff() {
    let (_realm, _a, ga, map) = setup_with(health_data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let hp = |m: &Map| {
                let f = m.unit_fields(ga).unwrap();
                (
                    f.get_u32(UNIT_FIELD_HEALTH),
                    f.get_u32(UNIT_FIELD_MAXHEALTH),
                )
            };
            let set_health = |m: &mut Map, v: u32| {
                if let Some(f) = m.unit_fields_mut(ga) {
                    f.set_u32(UNIT_FIELD_HEALTH, v);
                }
            };
            let (_, max) = hp(m);
            set_health(m, max / 2);
            let start = hp(m);
            let _ = put(m, ga, ga, MORE_HEALTH, &[100]);
            let buffed = hp(m);
            auras::remove_spell(m, ga, MORE_HEALTH, None);
            let unbuffed = hp(m);
            let _ = put(m, ga, ga, LAST_STAND, &[300]);
            let stand = hp(m);
            set_health(m, 100);
            auras::remove_spell(m, ga, LAST_STAND, None);
            let injured = hp(m);
            set_health(m, start.0);
            let mut bear = Vec::new();
            for _ in 0..10 {
                let _ = put(m, ga, ga, BEAR, &[FORM_BEAR as i32, 400]);
                let shifted = hp(m);
                auras::remove_spell(m, ga, BEAR, None);
                bear.push((shifted, hp(m)));
            }
            // Dead with Last Stand on: dead after it.
            let _ = put(m, ga, ga, LAST_STAND, &[300]);
            set_health(m, 0);
            auras::remove_spell(m, ga, LAST_STAND, None);
            let dead = hp(m).0;
            (start, buffed, unbuffed, stand, injured, bear, dead)
        })
        .await
        .unwrap();
    let (start, buffed, unbuffed, stand, injured, bear, dead) = got;
    let (health, max) = start;
    assert_eq!(buffed, (health, max + 100), "a buff heals nothing");
    assert_eq!(unbuffed, start, "nor takes any going");
    assert_eq!(
        stand,
        (health + 300, max + 300),
        "Last Stand's health comes"
    );
    assert_eq!(injured, (1, max), "and goes, leaving the injured at one");
    for (shifted, back) in bear {
        let share = shifted.0 as f32 / shifted.1 as f32;
        assert!(
            (share - health as f32 / max as f32).abs() < 0.01,
            "{shifted:?}"
        );
        assert_eq!(back, start, "back as it was");
    }
    assert_eq!(dead, 0, "the dead stay dead");
}

/// Invisibility of kinds 0 and 1, its detection of kind 1, drunken
/// invisibility, an aura that losing sight ends, and invisibility that a
/// cast breaks.
const INVISIBLE: u32 = 96_797;
const SEES_KIND_1: u32 = 96_798;
const DRUNK_HIDDEN: u32 = 96_799;
const LOST_SIGHT: u32 = 96_800;
const CAST_BREAKS: u32 = 96_801;
const WORLD_BOSS: u32 = 96_802;

fn invisible_data() -> GameData {
    let mut d = data();
    let mut lost = aura_spell(LOST_SIGHT, Some(60_000), &[(AURA_DUMMY, 0, 0)]);
    lost.aura_interrupt_flags = 0x10_0000;
    let mut breaks = aura_spell(CAST_BREAKS, Some(60_000), &[(AURA_MOD_INVISIBILITY, 0, 0)]);
    breaks.aura_interrupt_flags = INTERRUPT_CAST;
    add_spells(
        &mut d,
        [
            aura_spell(
                INVISIBLE,
                None,
                &[(AURA_MOD_INVISIBILITY, 0, 0), (AURA_MOD_INVISIBILITY, 1, 0)],
            ),
            aura_spell(
                SEES_KIND_1,
                None,
                &[(AURA_MOD_INVISIBILITY_DETECTION, 1, 0)],
            ),
            aura_spell(DRUNK_HIDDEN, None, &[(AURA_MOD_INVISIBILITY, 6, 0)]),
            lost,
            breaks,
        ],
    );
    let mut boss = crate::game::tests_p3_combat::template(WORLD_BOSS, 7);
    boss.rank = 3;
    d.creature_templates.insert(WORLD_BOSS, boss);
    d
}

/// Thistle Tea's invisibility: seeing through one of its kinds is enough;
/// a world boss sees every kind; the drunk see drunken invisibility as
/// drunk as it, the sober not. Going invisible ends what losing sight
/// ends; a cast ends invisibility that a cast breaks.
#[tokio::test]
async fn invisibility_is_seen_by_any_kind_bosses_and_the_drunk() {
    use crate::game::map::spells::stealth::sees;
    let (_realm, _a, ga, map) = setup_with(invisible_data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let hidden = spawn(m, ga, BRUTE, 1, 3.0);
            let watcher = spawn(m, ga, BRUTE, 1, 4.0);
            let boss = spawn(m, ga, WORLD_BOSS, 1, 5.0);
            let _ = put(m, ga, ga, INVISIBLE, &[200, 1_000]);
            let unseen = sees(m, watcher, ga);
            let _ = put(m, watcher, watcher, SEES_KIND_1, &[1_000]);
            let one_kind = sees(m, watcher, ga);
            let by_boss = sees(m, boss, ga);
            auras::remove_spell(m, ga, INVISIBLE, None);
            let _ = put(m, hidden, hidden, DRUNK_HIDDEN, &[512]);
            let sober = sees(m, ga, hidden);
            m.players.get_mut(&ga).unwrap().drunk.value = 512;
            let drunk = sees(m, ga, hidden);
            m.players.get_mut(&ga).unwrap().drunk.value = 0;
            // Going invisible; then a cast.
            let _ = put(m, ga, ga, LOST_SIGHT, &[0]);
            let _ = put(m, ga, ga, CAST_BREAKS, &[100]);
            let sight_lost = !auras::auras(m, ga).is_some_and(|a| a.has_spell(LOST_SIGHT));
            auras::interrupt(m, ga, INTERRUPT_CAST);
            let broken = !auras::auras(m, ga).is_some_and(|a| a.has_spell(CAST_BREAKS));
            (unseen, one_kind, by_boss, sober, drunk, sight_lost, broken)
        })
        .await
        .unwrap();
    assert_eq!(got, (false, true, true, false, true, true, true));
}

/// Tracking: resources of kinds 2 and 7, creatures of 1 and 7, kinds
/// outside the field (0, −1, 33), and Track Hidden.
const TRACKING: u32 = 96_803;
const TRACKING_OUTSIDE: u32 = 96_804;
const TRACK_HIDDEN: u32 = 96_805;

fn tracking_data() -> GameData {
    let mut d = data();
    let tracking = aura_spell(
        TRACKING,
        None,
        &[
            (AURA_TRACK_RESOURCES, 2, 0),
            (AURA_TRACK_RESOURCES, 7, 0),
            (AURA_TRACK_CREATURES, 1, 0),
        ],
    );
    let second = aura_spell(TRACKING + 10, None, &[(AURA_TRACK_CREATURES, 7, 0)]);
    add_spells(
        &mut d,
        [
            tracking,
            second,
            aura_spell(
                TRACKING_OUTSIDE,
                None,
                &[
                    (AURA_TRACK_RESOURCES, 0, 0),
                    (AURA_TRACK_RESOURCES, -1, 0),
                    (AURA_TRACK_RESOURCES, 33, 0),
                ],
            ),
            aura_spell(TRACK_HIDDEN, None, &[(AURA_TRACK_STEALTHED, 0, 0)]),
        ],
    );
    d
}

/// Thistle Tea's tracking fields: the kinds' bits together (0x42, 0x41)
/// and cleared when they go; kinds outside the field none; Track Hidden's
/// flag beside the field's others, and back off.
#[tokio::test]
async fn tracking_auras_set_the_players_fields_bits_and_flag() {
    let (_realm, _a, ga, map) = setup_with(tracking_data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let fields = |m: &Map| {
                let f = m.unit_fields(ga).unwrap();
                (
                    f.get_u32(PLAYER_TRACK_RESOURCES),
                    f.get_u32(PLAYER_TRACK_CREATURES),
                    f.get_byte(PLAYER_FIELD_BYTES, 0),
                )
            };
            let _ = put(m, ga, ga, TRACKING, &[0, 0, 0]);
            let _ = put(m, ga, ga, TRACKING + 10, &[0]);
            let tracked = fields(m);
            auras::remove_spell(m, ga, TRACKING, None);
            auras::remove_spell(m, ga, TRACKING + 10, None);
            let cleared = fields(m);
            let _ = put(m, ga, ga, TRACKING_OUTSIDE, &[0, 0, 0]);
            let outside = fields(m);
            auras::remove_spell(m, ga, TRACKING_OUTSIDE, None);
            if let Some(f) = m.unit_fields_mut(ga) {
                f.set_byte(PLAYER_FIELD_BYTES, 0, 0x08);
            }
            let _ = put(m, ga, ga, TRACK_HIDDEN, &[0]);
            let hidden = fields(m).2;
            auras::remove_spell(m, ga, TRACK_HIDDEN, None);
            let shown = fields(m).2;
            (tracked, cleared, outside, hidden, shown)
        })
        .await
        .unwrap();
    let (tracked, cleared, outside, hidden, shown) = got;
    assert_eq!((tracked.0, tracked.1), (0x42, 0x41));
    assert_eq!((cleared.0, cleared.1), (0, 0));
    assert_eq!(outside.0, 0, "kinds outside the field");
    assert_eq!((hidden, shown), (0x0A, 0x08));
}

/// Cat form; a root of its mechanic with a tick of damage; a snare of
/// its mechanic; snares of the mechanics a form can't shake off; a root of
/// no mechanic.
const CAT: u32 = 96_806;
const ROOTS: u32 = 96_807;
const SNARE: u32 = 96_808;
const SNARES_KEPT: u32 = 96_810;
const KEPT_MECHANICS: [u32; 13] = [0, 1, 2, 5, 8, 12, 13, 18, 20, 23, 24, 27, 30];
const PLAIN_ROOT: u32 = 96_830;

fn form_data() -> GameData {
    let mut d = data();
    let harmful = |id: u32, effects: &[(u32, i32, u32)], mechanic: u32| {
        let mut s = aura_spell(id, Some(20_000), effects);
        s.mechanic = mechanic;
        for e in s.effects.iter_mut().filter(|e| e.effect != 0) {
            e.target_a = 6;
        }
        s
    };
    let mut all = vec![
        aura_spell(CAT, None, &[(AURA_MOD_SHAPESHIFT, FORM_CAT as i32, 0)]),
        harmful(
            ROOTS,
            &[(AURA_MOD_ROOT, 0, 0), (AURA_PERIODIC_DAMAGE, 0, 3_000)],
            7,
        ),
        harmful(SNARE, &[(AURA_MOD_DECREASE_SPEED, 0, 0)], 11),
        harmful(PLAIN_ROOT, &[(AURA_MOD_ROOT, 0, 0)], 0),
    ];
    for (i, m) in KEPT_MECHANICS.iter().enumerate() {
        all.push(harmful(
            SNARES_KEPT + i as u32,
            &[(AURA_MOD_DECREASE_SPEED, 0, 0)],
            *m,
        ));
    }
    add_spells(&mut d, all);
    d
}

/// Thistle Tea's form change: cat form shakes off a root of its mechanic
/// (its tick of damage with it) and a snare of its own; not a snare of a
/// mechanic a form can't escape (or of none), nor a root of no mechanic.
#[tokio::test]
async fn a_druid_form_shakes_off_roots_and_snares_of_their_mechanics() {
    let (_realm, _a, ga, map) = setup_with(form_data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 1, 3.0);
            let mut spells = vec![ROOTS, SNARE, PLAIN_ROOT];
            spells.extend((0..KEPT_MECHANICS.len() as u32).map(|i| SNARES_KEPT + i));
            for s in &spells {
                let amounts = if *s == ROOTS { vec![0, 5] } else { vec![-50] };
                let _ = put(m, brute, ga, *s, &amounts);
            }
            let before: Vec<bool> = spells
                .iter()
                .map(|s| auras::auras(m, ga).is_some_and(|a| a.has_spell(*s)))
                .collect();
            let _ = put(m, ga, ga, CAT, &[FORM_CAT as i32]);
            let after: Vec<(u32, bool)> = spells
                .iter()
                .map(|s| (*s, auras::auras(m, ga).is_some_and(|a| a.has_spell(*s))))
                .collect();
            (before, after)
        })
        .await
        .unwrap();
    let (before, after) = got;
    assert!(before.iter().all(|b| *b), "all on: {before:?}");
    for (spell, on) in after {
        let shaken = matches!(spell, ROOTS | SNARE);
        assert_eq!(on, !shaken, "{spell}");
    }
}
