//! Phase 3's auras, Thistle Tea's `AuraTest` ported (the rest of it): the
//! crowd control a pet leaves be, a victim's change to spells' chance to
//! hit it, the periodic auras' ticks (their cadence, catching up, leeches,
//! drains and energizes), procs (cast ends, cooldowns, kills, crits,
//! charges, dodges, blocks, helpful ticks), food and drink, and dispels.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{ProcEvent, SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::combat::{self, Outcome};
use crate::game::map::procs::{self, Event};
use crate::game::map::spells;
use crate::game::tests_p3_combat::{self as base, BRUTE, forced_swing, setup_with, spawn};

/// Markers a proc casts: on its owner, and on the other.
const MARK_SELF: u32 = 96_100;
const MARK_OTHER: u32 = 96_101;
/// A shadow DoT of a second; a leech of a second; a mana drain.
const DOT: u32 = 96_102;
const LEECH: u32 = 96_103;
const DRAIN: u32 = 96_104;
/// A periodic energize of mana (Drink's), of rage (Bloodrage's).
const ENERGIZE_MANA: u32 = 96_105;
const ENERGIZE_RAGE: u32 = 96_106;
/// A physical absorb.
const SHIELD: u32 = 96_107;
/// Blood Craze: a percentage of the maximum health a tick.
const BLOOD_CRAZE: u32 = 96_108;
/// A tenth of the maximum mana a tick for five seconds, with no
/// amplitude; one of four seconds' amplitude; one that stacks to three;
/// one broken by standing.
const RECOVERY: u32 = 96_112;
const RECOVERY_SLOW: u32 = 96_113;
const RECOVERY_STACKS: u32 = 96_114;
const RECOVERY_SEATED: u32 = 96_115;
/// Food and drink as Thistle Tea's fixtures: no amplitude, broken by
/// standing.
const FOOD: u32 = 96_109;
const DRINK: u32 = 96_110;
/// A victim's 2% off the spells of every school but the physical.
const SENSES: u32 = 96_111;
/// Procs: on a harmful spell (Clearcasting's), on its cast's end, with a
/// second's cooldown, on a kill (Spirit Tap's), on a crit heal
/// (Inspiration's), Divine Favor's charge, on a dodge or resist (Setup's),
/// on a block (Holy Shield's), Frost Armor's plain one, and on a helpful
/// tick.
const CLEARCASTING: u32 = 96_120;
const CAST_END: u32 = 96_121;
const COOLING: u32 = 96_122;
const SPIRIT_TAP: u32 = 96_123;
const INSPIRATION: u32 = 96_124;
const DIVINE_FAVOR: u32 = 96_125;
const SETUP: u32 = 96_126;
const HOLY_SHIELD: u32 = 96_127;
const FROST_ARMOR: u32 = 96_128;
const HOT_PROC: u32 = 96_129;
/// A frost bolt (the triggering spell), a heal, a heal over time.
const BOLT: u32 = 96_130;
const HEAL: u32 = 96_131;
const RENEW: u32 = 96_132;
/// A fire bolt cast at the victim, and a ward of every school's
/// resistance to it.
const FIREBALL: u32 = 96_133;
const RESIST_ALL: u32 = 96_134;
/// Two magic buffs, a curse's dispel and a magic one of two.
const BUFF_ONE: u32 = 96_135;
const BUFF_TWO: u32 = 96_136;
const DISPEL_CURSE: u32 = 96_137;
const DISPEL_TWO: u32 = 96_138;
/// The crowd control kinds, each breakable by damage and not.
const CC_BASE: u32 = 96_150;
/// Linked auras: a parent of 12 seconds holding a child of 20 (a fifth
/// less physical damage taken); its second rank, with none; a parent
/// whose child holds a grandchild; and a parent with a child of a second.
const PARENT: u32 = 96_400;
const CHILD: u32 = 96_401;
const PARENT_2: u32 = 96_402;
const NESTED: u32 = 96_403;
const NESTED_CHILD: u32 = 96_404;
const GRANDCHILD: u32 = 96_405;
const SHORT_PARENT: u32 = 96_406;
const SHORT_CHILD: u32 = 96_407;

/// The crowd control kinds Thistle Tea's test tries: the first three are
/// what a pet leaves be when damage would break them.
const CC_KINDS: [u32; 7] = [
    AURA_MOD_CONFUSE,
    AURA_MOD_STUN,
    AURA_TRANSFORM,
    AURA_MOD_FEAR,
    AURA_MOD_ROOT,
    AURA_MOD_STEALTH,
    AURA_MOD_STAT,
];

/// A spell of aura effects (each its aura, misc value and amplitude),
/// lasting `ms` (none for no end), on its caster.
pub(crate) fn aura_spell(id: u32, ms: Option<u32>, effects: &[(u32, i32, u32)]) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Aura {id}"),
        equipped_item_class: -1,
        range_index: 1,
        duration_index: ms.unwrap_or(0),
        ..Default::default()
    };
    for (i, (aura, misc, amplitude)) in effects.iter().enumerate() {
        s.effects[i] = SpellEffect {
            effect: 6,
            aura: *aura,
            misc_value: *misc,
            amplitude: *amplitude,
            base_points: 0,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        };
    }
    s
}

/// A proc aura of `flags` that casts `trigger`, always.
pub(crate) fn proc_spell(id: u32, flags: u32, trigger: u32) -> SpellTemplate {
    let mut s = aura_spell(id, None, &[(AURA_PROC_TRIGGER_SPELL, 0, 0)]);
    s.effects[0].trigger_spell = trigger;
    s.proc_flags = flags;
    s.proc_chance = 100;
    s
}

pub(crate) fn proc_event(ex: u32, cooldown: u32) -> ProcEvent {
    ProcEvent {
        ex,
        cooldown,
        ..Default::default()
    }
}

pub(crate) fn data() -> GameData {
    let mut d = base::data();
    // A human priest and warrior of level 1 with 20 in each stat.
    for class in [1u8, 5] {
        d.player_level_stats
            .insert((1, class, 1), [20, 20, 20, 20, 20]);
        d.player_class_stats
            .insert((class, 1), (60, if class == 1 { 0 } else { 100 }));
    }
    let mut all = vec![
        aura_spell(MARK_SELF, Some(30_000), &[(AURA_DUMMY, 0, 0)]),
        aura_spell(MARK_OTHER, Some(30_000), &[(AURA_DUMMY, 0, 0)]),
        aura_spell(DOT, Some(10_000), &[(AURA_PERIODIC_DAMAGE, 0, 1000)]),
        aura_spell(LEECH, Some(5_000), &[(AURA_PERIODIC_LEECH, 0, 1000)]),
        aura_spell(DRAIN, Some(5_000), &[(AURA_PERIODIC_MANA_LEECH, 0, 1000)]),
        aura_spell(
            ENERGIZE_MANA,
            Some(18_000),
            &[(AURA_PERIODIC_ENERGIZE, 0, 3000)],
        ),
        aura_spell(
            ENERGIZE_RAGE,
            Some(10_000),
            &[(AURA_PERIODIC_ENERGIZE, 1, 1000)],
        ),
        aura_spell(SHIELD, Some(30_000), &[(AURA_SCHOOL_ABSORB, 1, 0)]),
        aura_spell(BLOOD_CRAZE, Some(6_000), &[(AURA_OBS_MOD_HEALTH, 0, 3000)]),
        aura_spell(RECOVERY, Some(5_000), &[(AURA_OBS_MOD_MANA, 0, 0)]),
        aura_spell(RECOVERY_SLOW, Some(8_000), &[(AURA_OBS_MOD_MANA, 0, 4000)]),
        aura_spell(RECOVERY_STACKS, Some(5_000), &[(AURA_OBS_MOD_MANA, 0, 0)]),
        aura_spell(RECOVERY_SEATED, Some(5_000), &[(AURA_OBS_MOD_MANA, 0, 0)]),
        aura_spell(FOOD, Some(18_000), &[(AURA_MOD_REGEN, 0, 0)]),
        aura_spell(DRINK, Some(18_000), &[(AURA_MOD_POWER_REGEN, 0, 0)]),
        aura_spell(
            SENSES,
            None,
            &[(AURA_MOD_ATTACKER_SPELL_HIT_CHANCE, 0x7E, 0)],
        ),
        aura_spell(RESIST_ALL, None, &[(AURA_MOD_RESIST_CHANCE, 0x7F, 0)]),
        aura_spell(RENEW, Some(15_000), &[(AURA_PERIODIC_HEAL, 0, 3000)]),
    ];
    // The markers on the other are the proc's harmful spells at it.
    for s in all.iter_mut().filter(|s| s.id == MARK_OTHER) {
        s.effects[0].target_a = 6;
    }
    for s in all.iter_mut() {
        match s.id {
            DOT => s.school = 5,
            DRAIN => {
                s.school = 5;
                s.effects[0].multiple_value = 2.0;
            }
            FOOD | DRINK | RECOVERY_SEATED => s.aura_interrupt_flags = INTERRUPT_NOT_SEATED,
            RECOVERY_STACKS => s.stack_amount = 3,
            RENEW => s.school = 1,
            _ => {}
        }
    }
    let mut strikes = Vec::new();
    for (id, school) in [(BOLT, 4u32), (FIREBALL, 2)] {
        let mut s = SpellTemplate {
            id,
            name: format!("Bolt {id}"),
            school,
            dmg_class: 1,
            equipped_item_class: -1,
            range_index: 1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: spells::EFFECT_SCHOOL_DAMAGE,
            base_points: 9,
            base_dice: 1,
            target_a: 6,
            ..Default::default()
        };
        strikes.push(s);
    }
    let mut heal = SpellTemplate {
        id: HEAL,
        name: "Heal".into(),
        school: 1,
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    heal.effects[0] = SpellEffect {
        effect: spells::EFFECT_HEAL,
        base_points: 9,
        base_dice: 1,
        target_a: 21,
        ..Default::default()
    };
    strikes.push(heal);
    // Magic buffs, and dispels of a curse and of two magic auras.
    for id in [BUFF_ONE, BUFF_TWO] {
        let mut s = aura_spell(id, Some(30_000), &[(AURA_MOD_STAT, 0, 0)]);
        s.dispel = 1;
        all.push(s);
    }
    for (id, kind, count) in [(DISPEL_CURSE, 2, 1), (DISPEL_TWO, 1, 2)] {
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
            base_points: count - 1,
            base_dice: 1,
            target_a: 6,
            ..Default::default()
        };
        strikes.push(s);
    }
    // The procs.
    let mut clearcasting = proc_spell(CLEARCASTING, procs::PROC_DONE_NEGATIVE_SPELL, MARK_SELF);
    clearcasting.school = 4;
    all.extend([
        clearcasting,
        proc_spell(CAST_END, procs::PROC_DONE_NEGATIVE_SPELL, MARK_SELF),
        proc_spell(COOLING, procs::PROC_DONE_NEGATIVE_SPELL, MARK_SELF),
        proc_spell(SPIRIT_TAP, procs::PROC_KILL, MARK_SELF),
        proc_spell(INSPIRATION, 0x1_5550, MARK_OTHER),
        proc_spell(SETUP, 0x2_0028, MARK_OTHER),
        proc_spell(FROST_ARMOR, procs::PROC_TAKEN_MELEE, MARK_OTHER),
        proc_spell(HOT_PROC, 0x4_0000, MARK_OTHER),
    ]);
    let mut favor = aura_spell(
        DIVINE_FAVOR,
        Some(60_000),
        &[(AURA_MOD_SPELL_CRIT_CHANCE, 0, 0)],
    );
    favor.proc_flags = 0x1_4000;
    favor.proc_chance = 101;
    favor.proc_charges = 1;
    let mut holy_shield = aura_spell(
        HOLY_SHIELD,
        Some(10_000),
        &[(AURA_PROC_TRIGGER_SPELL, 0, 0)],
    );
    holy_shield.effects[0].trigger_spell = MARK_OTHER;
    holy_shield.proc_flags = 0x2A8;
    holy_shield.proc_chance = 100;
    all.extend([favor, holy_shield]);
    d.spells.proc_events.extend([
        (CAST_END, proc_event(procs::EX_CAST_END, 0)),
        (COOLING, proc_event(0, 1000)),
        (INSPIRATION, proc_event(procs::EX_CRITICAL_HIT, 0)),
        (SETUP, proc_event(procs::EX_DODGE | procs::EX_RESIST, 0)),
        (HOLY_SHIELD, proc_event(procs::EX_BLOCK, 0)),
        (HOT_PROC, proc_event(procs::EX_PERIODIC_POSITIVE, 0)),
    ]);
    // The crowd control kinds, breakable (even ids) and not (odd ones).
    for (i, kind) in CC_KINDS.iter().enumerate() {
        for breaks in [true, false] {
            let id = CC_BASE + 2 * i as u32 + u32::from(!breaks);
            let mut s = aura_spell(id, Some(1_000), &[(*kind, 0, 0)]);
            s.effects[0].target_a = 6;
            if breaks {
                s.aura_interrupt_flags = INTERRUPT_DAMAGE;
            }
            all.push(s);
        }
    }
    // Linked auras.
    let link = |id: u32, ms: u32, child: u32| {
        let mut s = aura_spell(id, Some(ms), &[(AURA_LINKED, 0, 0)]);
        s.effects[0].trigger_spell = child;
        s
    };
    let mut child = aura_spell(
        CHILD,
        Some(20_000),
        &[(AURA_MOD_DAMAGE_PERCENT_TAKEN, 1, 0)],
    );
    child.effects[0].base_points = -21;
    let mut nested_child = link(NESTED_CHILD, 20_000, GRANDCHILD);
    nested_child.effects[1] = child.effects[0];
    let mut short_child = child.clone();
    short_child.id = SHORT_CHILD;
    short_child.duration_index = 1_000;
    let mut grandchild = child.clone();
    grandchild.id = GRANDCHILD;
    all.extend([
        link(PARENT, 12_000, CHILD),
        child,
        aura_spell(PARENT_2, Some(12_000), &[(AURA_MOD_STAT, 0, 0)]),
        link(NESTED, 12_000, NESTED_CHILD),
        nested_child,
        grandchild,
        link(SHORT_PARENT, 12_000, SHORT_CHILD),
        short_child,
    ]);
    for (id, rank) in [(PARENT, 1), (PARENT_2, 2)] {
        d.spells.ranks.insert(
            id,
            crate::game::data::spells::SpellRank {
                first: PARENT,
                rank,
                ..Default::default()
            },
        );
    }
    add_spells(&mut d, all.into_iter().chain(strikes));
    d
}

/// Spells into the data, each duration index its milliseconds.
pub(crate) fn add_spells(d: &mut GameData, spells: impl IntoIterator<Item = SpellTemplate>) {
    for s in spells {
        if s.duration_index != 0 {
            d.spells.durations.insert(
                s.duration_index,
                SpellDuration {
                    base: s.duration_index as i32,
                    ..Default::default()
                },
            );
        }
        d.spells.spells.insert(s.id, s);
    }
}

/// Put `id` on `target` from `caster`, its effects each with an amount.
pub(crate) fn put(
    m: &mut Map,
    caster: Guid,
    target: Guid,
    id: u32,
    amounts: &[i32],
) -> Result<(), Refused> {
    let s = m.realm.data.spells.get(id).cloned().expect("the spell");
    let effects: Vec<(usize, i32)> = amounts.iter().copied().enumerate().collect();
    auras::apply_spell(m, caster, target, &s, &effects, Context::default())
}

pub(crate) fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

/// Whether `g` has `spell`, taken off as it is told.
pub(crate) fn took(m: &mut Map, g: Guid, spell: u32) -> bool {
    let had = has(m, g, spell);
    auras::remove_spell(m, g, spell, None);
    had
}

pub(crate) fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

pub(crate) fn set(m: &mut Map, g: Guid, index: u16, v: u32) {
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_u32(index, v);
    }
}

pub(crate) fn get(m: &Map, g: Guid, index: u16) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(index))
}

/// Time passes on the map: its auras run out and tick.
pub(crate) fn pass(m: &mut Map, ms: u64) {
    m.now += Duration::from_millis(ms);
    auras::update(m);
}

/// The next tick of `g`'s `spell`, from now.
pub(crate) fn next_tick(m: &Map, g: Guid, spell: u32) -> Option<Duration> {
    let now = m.now;
    auras::auras(m, g)?
        .holders
        .iter()
        .find(|h| h.spell == spell)?
        .effects
        .first()?
        .next_tick
        .map(|t| t.saturating_duration_since(now))
}

/// A spell `spell` of `actor` at `target` went as `ex` said.
fn spell_event(actor: Guid, target: Guid, spell: u32, flags: u32, ex: u32) -> Event {
    Event {
        actor,
        target,
        actor_flags: flags,
        spell: Some(spell),
        ex,
        ..Default::default()
    }
}

/// Thistle Tea's `breakable_crowd_control?/1`: a confusion, stun or
/// transform that damage breaks is what a pet leaves be; one damage
/// doesn't break isn't, nor any fear, root, stealth or stat change. It
/// tracks the aura: gone when taken off, run out, or its unit dead.
#[tokio::test]
async fn crowd_control_damage_breaks_is_left_be_while_it_lasts() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let (kinds, tracked) = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            let mut kinds = Vec::new();
            for i in 0..CC_KINDS.len() as u32 {
                for off in [0, 1] {
                    put(m, ga, brute, CC_BASE + 2 * i + off, &[0]).unwrap();
                    kinds.push(crate::game::map::pets::breakable_cc(m, brute));
                    auras::remove_where(m, brute, |_| true);
                }
            }
            let stun = CC_BASE + 2;
            let held = |m: &Map| crate::game::map::pets::breakable_cc(m, brute);
            put(m, ga, brute, stun, &[0]).unwrap();
            let applied = held(m);
            auras::remove_spell(m, brute, stun, None);
            let removed = held(m);
            put(m, ga, brute, stun, &[0]).unwrap();
            pass(m, 1_100);
            let expired = held(m);
            put(m, ga, brute, stun, &[0]).unwrap();
            combat::deal_damage(m, ga, brute, 1_000_000);
            let dead = held(m);
            (kinds, (applied, removed, expired, dead))
        })
        .await
        .unwrap();
    assert_eq!(
        kinds,
        [
            true, false, true, false, true, false, false, false, false, false, false, false, false,
            false
        ]
    );
    assert_eq!(tracked, (true, false, false, false));
}

/// Thistle Tea's `attacker_spell_hit_chance/1` and mangos's
/// `MagicSpellHitChance`: a victim's aura of 2% less for the schools of its
/// mask takes 2% from a fire spell's chance to hit it, and nothing from a
/// physical one's.
#[tokio::test]
async fn a_victims_aura_takes_from_its_schools_spells_chance_to_hit() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 1, 5.0);
            let data = m.realm.data.clone();
            let fire = data.spells.get(FIREBALL).unwrap().clone();
            let mut physical = fire.clone();
            physical.school = 0;
            let chance = |m: &Map, s: &SpellTemplate| {
                spells::outcome::spell_hit_chance(m, ga, brute, s, &Default::default())
            };
            let before = (chance(m, &fire), chance(m, &physical));
            put(m, ga, brute, SENSES, &[-2]).unwrap();
            (before, (chance(m, &fire), chance(m, &physical)))
        })
        .await
        .unwrap();
    assert_eq!(got, ((9_600, 9_600), (9_400, 9_600)));
}

/// Thistle Tea's `tick/2`: a periodic aura doesn't tick before its
/// amplitude has gone by, and does as it has. Reapplied, the interval
/// starts again from then. An update late by seven intervals ticks once,
/// on the aura's own cadence, the next tick due at once (mangos's
/// `Aura::Update`, one tick an update).
#[tokio::test]
async fn a_dot_ticks_on_its_amplitude_and_restarts_when_reapplied() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            put(m, ga, brute, DOT, &[10]).unwrap();
            pass(m, 999);
            let early = health(m, brute);
            pass(m, 1);
            let first = health(m, brute);
            pass(m, 7_000);
            let late = (health(m, brute), next_tick(m, brute, DOT));
            pass(m, 500);
            put(m, ga, brute, DOT, &[10]).unwrap();
            let restarted = next_tick(m, brute, DOT);
            (early, first, late, restarted)
        })
        .await
        .unwrap();
    assert_eq!(got.0, 100_000, "nothing before the amplitude");
    assert_eq!(got.1, 99_990);
    assert_eq!(
        got.2,
        (99_980, Some(Duration::ZERO)),
        "one tick an update, the next due"
    );
    assert_eq!(got.3, Some(Duration::from_millis(1000)));
}

/// Thistle Tea's periodic leech: its caster heals only what the tick took
/// of the victim's health, after the victim's absorbs and short of its
/// overkill: 20 into a shield of 20 heals nothing, into one of 15 heals 5,
/// and from a victim of 5 health heals 5.
#[tokio::test]
async fn a_leech_heals_only_the_health_it_took() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            for (life, shield) in [(100u32, 20), (100, 15), (5, 0)] {
                let brute = spawn(m, ga, BRUTE, 20, 5.0);
                set(m, brute, UNIT_FIELD_HEALTH, life);
                if shield > 0 {
                    put(m, brute, brute, SHIELD, &[shield]).unwrap();
                }
                set(m, ga, UNIT_FIELD_HEALTH, 10);
                put(m, ga, brute, LEECH, &[20]).unwrap();
                pass(m, 1_000);
                out.push((health(m, brute), health(m, ga) - 10));
                auras::remove_where(m, brute, |_| true);
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(got, [(100, 0), (95, 5), (0, 5)]);
}

/// Thistle Tea's periodic mana leech: its first tick an amplitude after
/// it lands, each taking no more than the mana there is, its caster
/// gaining twice that (the effect's multiple value): 20, then the last 10,
/// then nothing.
#[tokio::test]
async fn a_mana_drain_takes_no_more_than_there_is() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Priest).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            set(m, brute, UNIT_FIELD_MAXPOWER1, 100);
            set(m, brute, UNIT_FIELD_POWER1, 30);
            set(m, ga, UNIT_FIELD_POWER1, 0);
            put(m, ga, brute, DRAIN, &[20]).unwrap();
            let first = next_tick(m, brute, DRAIN);
            let mut out = Vec::new();
            for _ in 0..3 {
                pass(m, 1_000);
                out.push((
                    get(m, brute, UNIT_FIELD_POWER1),
                    get(m, ga, UNIT_FIELD_POWER1),
                ));
            }
            (first, out)
        })
        .await
        .unwrap();
    assert_eq!(got.0, Some(Duration::from_millis(1000)));
    assert_eq!(got.1, [(10, 40), (0, 60), (0, 60)]);
}

fn energize_log(m: &ServerOpcodeMessage) -> Option<(u32, u32, u32)> {
    match m {
        ServerOpcodeMessage::SMSG_PERIODICAURALOG(l) => match l.auras.first()?.aura_type {
            AuraLog_AuraType::PeriodicEnergize {
                damage3,
                misc_value1,
            } => Some((l.spell, damage3, misc_value1)),
            _ => None,
        },
        _ => None,
    }
}

/// Thistle Tea's periodic energize: Drink's tick gives its mana and is
/// logged as a periodic energize of mana; Bloodrage's gives its rage, is
/// logged as one of rage, and makes half of it its caster's threat on the
/// creatures fighting them.
#[tokio::test]
async fn periodic_energize_gives_its_power_and_is_logged() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Priest).await;
    a.clear();
    let mana = map
        .query(move |m| {
            set(m, ga, UNIT_FIELD_POWER1, 10);
            put(m, ga, ga, ENERGIZE_MANA, &[50]).unwrap();
            pass(m, 3_000);
            get(m, ga, UNIT_FIELD_POWER1)
        })
        .await
        .unwrap();
    assert_eq!(mana, 60);
    let told = a.wait_for("SMSG_PERIODICAURALOG", energize_log).await;
    assert_eq!(told, (ENERGIZE_MANA, 50, 0));

    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    a.clear();
    let (rage, threat) = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            crate::game::map::ai::add_threat(m, brute, ga, 1.0);
            set(m, ga, UNIT_FIELD_POWER1 + 1, 100);
            put(m, ga, ga, ENERGIZE_RAGE, &[10]).unwrap();
            pass(m, 1_000);
            let threat = m
                .creature(brute)
                .and_then(|c| c.threat.iter().find(|(t, _)| *t == ga).map(|(_, v)| *v));
            (get(m, ga, UNIT_FIELD_POWER1 + 1), threat)
        })
        .await
        .unwrap();
    assert_eq!(rage, 110);
    assert_eq!(threat, Some(6.0), "half the rage on what was there");
    let told = a.wait_for("SMSG_PERIODICAURALOG", energize_log).await;
    assert_eq!(told, (ENERGIZE_RAGE, 10, 1));
}

/// Thistle Tea's `obs_mod_health`: Blood Craze's tick heals a percentage
/// of the maximum health, 2 of 100.
#[tokio::test]
async fn a_percent_heal_ticks_a_share_of_the_maximum() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            put(m, ga, ga, BLOOD_CRAZE, &[2]).unwrap();
            set(m, ga, UNIT_FIELD_MAXHEALTH, 100);
            set(m, ga, UNIT_FIELD_HEALTH, 50);
            pass(m, 3_001);
            health(m, ga)
        })
        .await
        .unwrap();
    assert_eq!(got, 52);
}

/// Thistle Tea's food and drink: with no amplitude neither ticks as an
/// aura (the regeneration gives what they give), so nothing is healed,
/// given or logged by their auras; turning's interrupt doesn't take them
/// off, as they are broken by standing alone (turning stands a seated
/// eater up, which does: `tests_body`).
#[tokio::test]
async fn food_and_drink_tick_no_aura_and_turning_leaves_them() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Priest).await;
    a.clear();
    let got = map
        .query(move |m| {
            set(m, ga, UNIT_FIELD_HEALTH, 50);
            set(m, ga, UNIT_FIELD_POWER1, 10);
            put(m, ga, ga, FOOD, &[17]).unwrap();
            put(m, ga, ga, DRINK, &[42]).unwrap();
            pass(m, 5_000);
            let pools = (health(m, ga), get(m, ga, UNIT_FIELD_POWER1));
            auras::interrupt(m, ga, INTERRUPT_TURNING);
            (pools, has(m, ga, FOOD), has(m, ga, DRINK))
        })
        .await
        .unwrap();
    assert_eq!(got, ((50, 10), true, true));
    let logged = a
        .gets(Duration::from_millis(200), |m| match m {
            ServerOpcodeMessage::SMSG_PERIODICAURALOG(_) => Some(()),
            _ => None,
        })
        .await;
    assert_eq!(logged, None);
}

/// Thistle Tea's `reactions/3` on spells: a proc never goes off on its own
/// spell; a cast-end proc waits for the cast's end, not its hit; and a
/// proc's cooldown holds a second, then lets it go off again.
#[tokio::test]
async fn a_proc_waits_for_its_outcome_its_cooldown_and_another_spell() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            let negative = procs::PROC_DONE_NEGATIVE_SPELL;
            put(m, ga, ga, CLEARCASTING, &[0]).unwrap();
            procs::trigger(
                m,
                spell_event(ga, brute, CLEARCASTING, negative, procs::EX_NORMAL_HIT),
            );
            let own = took(m, ga, MARK_SELF);
            procs::trigger(
                m,
                spell_event(ga, brute, BOLT, negative, procs::EX_NORMAL_HIT),
            );
            let other = took(m, ga, MARK_SELF);
            auras::remove_where(m, ga, |_| true);

            put(m, ga, ga, CAST_END, &[0]).unwrap();
            procs::trigger(
                m,
                spell_event(ga, brute, BOLT, negative, procs::EX_NORMAL_HIT),
            );
            let on_hit = took(m, ga, MARK_SELF);
            procs::trigger(
                m,
                spell_event(ga, brute, BOLT, negative, procs::EX_CAST_END),
            );
            let on_end = took(m, ga, MARK_SELF);
            auras::remove_where(m, ga, |_| true);

            put(m, ga, ga, COOLING, &[0]).unwrap();
            let mut cooling = Vec::new();
            for ms in [0, 500, 600] {
                m.now += Duration::from_millis(ms);
                procs::trigger(
                    m,
                    spell_event(ga, brute, BOLT, negative, procs::EX_NORMAL_HIT),
                );
                cooling.push(took(m, ga, MARK_SELF));
            }
            ((own, other), (on_hit, on_end), cooling)
        })
        .await
        .unwrap();
    assert_eq!(got.0, (false, true), "not on its own spell");
    assert_eq!(got.1, (false, true), "on the cast's end alone");
    assert_eq!(got.2, [true, false, true], "a second's cooldown");
}

/// Thistle Tea's kill reactions: a kill proc (Spirit Tap's) goes off on a
/// kill that gives experience and not on a gray one's; a proc without the
/// kill flag doesn't on any.
#[tokio::test]
async fn kill_procs_go_off_on_a_kill_worth_experience() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            base::level(m, ga, 20);
            put(m, ga, ga, SPIRIT_TAP, &[0]).unwrap();
            put(m, ga, ga, CLEARCASTING, &[0]).unwrap();
            let worth = spawn(m, ga, BRUTE, 20, 5.0);
            combat::deal_damage(m, ga, worth, 1_000_000);
            let even = took(m, ga, MARK_SELF);
            let gray = spawn(m, ga, BRUTE, 1, 5.0);
            combat::deal_damage(m, ga, gray, 1_000_000);
            let gray = took(m, ga, MARK_SELF);
            auras::remove_spell(m, ga, SPIRIT_TAP, None);
            let worth = spawn(m, ga, BRUTE, 20, 5.0);
            combat::deal_damage(m, ga, worth, 1_000_000);
            (even, gray, took(m, ga, MARK_SELF))
        })
        .await
        .unwrap();
    assert_eq!(got, (true, false, false));
}

/// Thistle Tea's helpful procs: a crit-only proc (Inspiration's) goes off
/// at the healed on a crit heal and not a plain one; a proc on helpful
/// ticks goes off at the one a heal over time ticks on; and a trigger-less
/// proc with a charge (Divine Favor's) spends it on its first heal, and is
/// gone.
#[tokio::test]
async fn helpful_procs_go_off_on_crits_ticks_and_spend_their_charges() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Priest).await;
    let got = map
        .query(move |m| {
            let friend = spawn(m, ga, base::HELPER, 20, 5.0);
            let helpful = procs::PROC_DONE_POSITIVE_SPELL;
            put(m, ga, ga, INSPIRATION, &[0]).unwrap();
            procs::trigger(
                m,
                spell_event(ga, friend, HEAL, helpful, procs::EX_NORMAL_HIT),
            );
            let plain = took(m, friend, MARK_OTHER);
            procs::trigger(
                m,
                spell_event(ga, friend, HEAL, helpful, procs::EX_CRITICAL_HIT),
            );
            let crit = took(m, friend, MARK_OTHER);
            auras::remove_where(m, ga, |_| true);

            put(m, ga, ga, HOT_PROC, &[0]).unwrap();
            put(m, ga, friend, RENEW, &[10]).unwrap();
            pass(m, 3_000);
            let ticked = took(m, friend, MARK_OTHER);
            auras::remove_where(m, ga, |_| true);

            put(m, ga, ga, DIVINE_FAVOR, &[100]).unwrap();
            let before = has(m, ga, DIVINE_FAVOR);
            procs::trigger(
                m,
                spell_event(ga, friend, HEAL, helpful, procs::EX_CRITICAL_HIT),
            );
            ((plain, crit), ticked, (before, has(m, ga, DIVINE_FAVOR)))
        })
        .await
        .unwrap();
    assert_eq!(got.0, (false, true), "on the crit alone");
    assert!(got.1, "on a helpful tick");
    assert_eq!(got.2, (true, false), "the charge spent");
}

/// Thistle Tea's taken procs: Setup's goes off at the attacker on a dodge
/// and not a plain hit, and on a spell it resisted; Holy Shield's on a
/// block alone; Frost Armor's on a blow that landed (a partial block and a
/// crit too), not a dodge.
#[tokio::test]
async fn taken_procs_go_off_on_the_outcomes_they_name() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 2.0);
            let swing = |m: &mut Map, o: Outcome, proc: u32| {
                put(m, ga, ga, proc, &[0]).unwrap();
                // Blows of a point, that the warrior lives through.
                if let Some(c) = m.creature_mut(brute) {
                    c.fields.set_f32(UNIT_FIELD_MINDAMAGE, 1.0);
                    c.fields.set_f32(UNIT_FIELD_MAXDAMAGE, 1.0);
                }
                forced_swing(m, brute, ga, o);
                auras::remove_spell(m, ga, proc, None);
                took(m, brute, MARK_OTHER)
            };
            let setup = (
                swing(m, Outcome::Hit, SETUP),
                swing(m, Outcome::Dodge, SETUP),
            );
            // A fire bolt at the warrior, resisted.
            put(m, ga, ga, RESIST_ALL, &[1_000]).unwrap();
            put(m, ga, ga, SETUP, &[0]).unwrap();
            spells::cast_triggered(m, brute, FIREBALL, Some(ga));
            let resisted = took(m, brute, MARK_OTHER);
            auras::remove_where(m, ga, |_| true);
            let holy = (
                swing(m, Outcome::Hit, HOLY_SHIELD),
                swing(m, Outcome::Block, HOLY_SHIELD),
            );
            let armor = (
                swing(m, Outcome::Dodge, FROST_ARMOR),
                swing(m, Outcome::Block, FROST_ARMOR),
                swing(m, Outcome::Crit, FROST_ARMOR),
            );
            (setup, resisted, holy, armor)
        })
        .await
        .unwrap();
    assert_eq!(got.0, (false, true), "Setup on a dodge");
    assert!(got.1, "and on a resisted spell");
    assert_eq!(got.2, (false, true), "Holy Shield on a block");
    assert_eq!(got.3, (false, true, true), "Frost Armor on what landed");
}

/// Thistle Tea's `dispel/5`: a dispel of a kind the unit has none of takes
/// nothing; one of two takes two auras of its kind.
#[tokio::test]
async fn a_dispel_takes_its_count_of_its_kind_or_nothing() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Priest).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            put(m, brute, brute, BUFF_ONE, &[3]).unwrap();
            put(m, brute, brute, BUFF_TWO, &[3]).unwrap();
            spells::cast_triggered(m, ga, DISPEL_CURSE, Some(brute));
            let curse = (has(m, brute, BUFF_ONE), has(m, brute, BUFF_TWO));
            spells::cast_triggered(m, ga, DISPEL_TWO, Some(brute));
            (curse, (has(m, brute, BUFF_ONE), has(m, brute, BUFF_TWO)))
        })
        .await
        .unwrap();
    assert_eq!(got, ((true, true), (false, false)));
}

/// `g`'s holders: each spell, whether it has a slot, and whether it is
/// passive.
pub(crate) fn shown(m: &Map, g: Guid) -> Vec<(u32, bool, bool)> {
    auras::auras(m, g).map_or_else(Vec::new, |a| {
        a.holders
            .iter()
            .map(|h| (h.spell, h.slot.is_some(), h.passive))
            .collect()
    })
}

/// The share of physical and fire damage `g` takes.
fn taken_share(m: &Map, g: Guid) -> (f32, f32) {
    auras::auras(m, g).map_or((1.0, 1.0), |a| {
        (
            a.product_mask(AURA_MOD_DAMAGE_PERCENT_TAKEN, 1),
            a.product_mask(AURA_MOD_DAMAGE_PERCENT_TAKEN, SCHOOL_FIRE_MASK),
        )
    })
}

const SCHOOL_FIRE_MASK: u32 = 0x4;

/// Thistle Tea's `Linked`: a parent's child is hidden (no slot), passive,
/// and does its work (a fifth less physical damage, none less fire); a
/// refresh keeps one child, not two of its effect; a copy of the child's
/// spell on its own stays when the parent goes. A blocked parent gives no
/// child. The child goes as the parent goes, cancelled, run out (the
/// child's own 20 seconds longer than the parent's 12) or dead, and so do
/// nested children; a child taken off isn't made again until the parent
/// is.
#[tokio::test]
async fn a_linked_child_is_hidden_and_goes_with_its_parent() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            put(m, ga, ga, PARENT, &[0]).unwrap();
            out.push((shown(m, ga), taken_share(m, ga)));
            put(m, ga, ga, PARENT, &[0]).unwrap();
            out.push((shown(m, ga), taken_share(m, ga)));
            auras::remove_where(m, ga, |_| true);
            // A copy of its own stays.
            put(m, ga, ga, PARENT, &[0]).unwrap();
            put(m, ga, ga, CHILD, &[-20]).unwrap();
            auras::remove_spell(m, ga, PARENT, None);
            let copy = shown(m, ga);
            auras::remove_where(m, ga, |_| true);
            // A blocked parent.
            put(m, ga, ga, PARENT_2, &[5]).unwrap();
            let blocked = put(m, ga, ga, PARENT, &[0]);
            let after_block = shown(m, ga);
            auras::remove_where(m, ga, |_| true);
            // Cancelled, run out, dead.
            put(m, ga, ga, PARENT, &[0]).unwrap();
            auras::cancel(m, ga, PARENT);
            let cancelled = shown(m, ga);
            put(m, ga, ga, PARENT, &[0]).unwrap();
            pass(m, 12_000);
            let ran_out = shown(m, ga);
            put(m, ga, ga, NESTED, &[0]).unwrap();
            let nested = shown(m, ga);
            auras::remove_spell(m, ga, NESTED, None);
            let nested_gone = shown(m, ga);
            // Taken off: not again until the parent is.
            put(m, ga, ga, PARENT, &[0]).unwrap();
            auras::remove_spell(m, ga, CHILD, None);
            pass(m, 1_000);
            let not_again = shown(m, ga);
            put(m, ga, ga, PARENT, &[0]).unwrap();
            let again = shown(m, ga);
            auras::clear_on_death(m, ga);
            let dead = shown(m, ga);
            (
                out,
                copy,
                (blocked, after_block),
                (cancelled, ran_out),
                (nested, nested_gone),
                (not_again, again, dead),
            )
        })
        .await
        .unwrap();
    let (out, copy, blocked, gone, nested, again) = got;
    let pair = vec![(PARENT, true, false), (CHILD, false, true)];
    assert_eq!(out[0], (pair.clone(), (0.8, 1.0)), "hidden, and at work");
    assert_eq!(out[1], (pair.clone(), (0.8, 1.0)), "one child on a refresh");
    assert_eq!(copy, [(CHILD, true, false)], "the copy of its own stays");
    assert_eq!(
        blocked,
        (Err(Refused::Bounced), vec![(PARENT_2, true, false)]),
        "no child from a blocked parent"
    );
    assert_eq!(gone, (vec![], vec![]), "cancelled and run out");
    assert_eq!(
        nested.0,
        [
            (NESTED, true, false),
            (NESTED_CHILD, false, true),
            (GRANDCHILD, false, true)
        ]
    );
    assert_eq!(nested.1, [], "nested children leave with their root");
    assert_eq!(again.0, [(PARENT, true, false)], "not made again");
    assert_eq!(again.1, pair, "until the parent is");
    assert_eq!(again.2, [], "death takes both");
}

/// Thistle Tea's `Linked`: a child shorter than its parent runs out on
/// its own time and isn't made again while the parent lasts.
#[tokio::test]
async fn a_linked_child_runs_its_own_time_within_its_parents() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            put(m, ga, ga, SHORT_PARENT, &[0]).unwrap();
            let both = shown(m, ga);
            pass(m, 1_000);
            pass(m, 1_000);
            (both, shown(m, ga))
        })
        .await
        .unwrap();
    assert_eq!(
        got.0,
        [(SHORT_PARENT, true, false), (SHORT_CHILD, false, true)]
    );
    assert_eq!(got.1, [(SHORT_PARENT, true, false)]);
}

fn mana_log(m: &ServerOpcodeMessage) -> Option<u32> {
    match m {
        ServerOpcodeMessage::SMSG_PERIODICAURALOG(l) => match l.auras.first()?.aura_type {
            AuraLog_AuraType::ObsModMana { damage3, .. } => Some(damage3),
            _ => None,
        },
        _ => None,
    }
}

/// Thistle Tea's `PercentManaRecoveryTest`: with no amplitude a tick a
/// second, a tenth of the maximum mana of the moment, logged, half of
/// what it gave its caster's threat (none for nothing gained), capped at
/// the maximum; stacks multiply it, a percentage of none gives none, one
/// without mana or dead gets nothing; it ticks at its end before it goes;
/// an explicit amplitude restarts on a refresh; standing, cancelling and
/// death stop it.
#[tokio::test]
async fn percent_mana_recovery_ticks_a_share_of_the_maximum() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Priest).await;
    a.clear();
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            crate::game::map::ai::add_threat(m, brute, ga, 1.0);
            let threat = |m: &Map| {
                m.creature(brute)
                    .and_then(|c| c.threat.iter().find(|(t, _)| *t == ga).map(|(_, v)| *v))
                    .unwrap_or(0.0)
            };
            let mana = |m: &Map| get(m, ga, UNIT_FIELD_POWER1);
            // The aura on, then the pools (its going on works out the
            // player's maximum again).
            let fresh = |m: &mut Map, have: u32, spell: u32, amount: i32| {
                auras::remove_where(m, ga, |_| true);
                put(m, ga, ga, spell, &[amount]).unwrap();
                set(m, ga, UNIT_FIELD_MAXPOWER1, 1_000);
                set(m, ga, UNIT_FIELD_POWER1, have);
            };
            fresh(m, 100, RECOVERY, 10);
            pass(m, 999);
            let early = mana(m);
            pass(m, 1);
            let first = (mana(m), threat(m));
            set(m, ga, UNIT_FIELD_MAXPOWER1, 2_000);
            pass(m, 1_000);
            let bigger = mana(m);
            // Capped, the threat of what it gave.
            fresh(m, 970, RECOVERY, 10);
            let before = threat(m);
            pass(m, 1_000);
            let capped = (mana(m), threat(m) - before);
            let before = threat(m);
            pass(m, 1_000);
            let full = threat(m) - before;
            // Its last tick at its end, then gone: five ticks of the
            // priest's own maximum (the aura's going works it out again).
            auras::remove_where(m, ga, |_| true);
            put(m, ga, ga, RECOVERY, &[10]).unwrap();
            let own = get(m, ga, UNIT_FIELD_MAXPOWER1);
            set(m, ga, UNIT_FIELD_POWER1, 0);
            for _ in 0..5 {
                pass(m, 1_000);
            }
            let last = (mana(m) == own / 10 * 5, has(m, ga, RECOVERY));
            // Stacks, and nothing.
            fresh(m, 100, RECOVERY_STACKS, 10);
            put(m, ga, ga, RECOVERY_STACKS, &[10]).unwrap();
            set(m, ga, UNIT_FIELD_MAXPOWER1, 1_000);
            set(m, ga, UNIT_FIELD_POWER1, 100);
            pass(m, 1_000);
            let stacked = mana(m);
            let mut none = Vec::new();
            for amount in [0, -10] {
                fresh(m, 100, RECOVERY, amount);
                pass(m, 1_000);
                none.push(mana(m));
            }
            // A refresh of an explicit amplitude restarts it.
            fresh(m, 100, RECOVERY_SLOW, 10);
            pass(m, 3_000);
            put(m, ga, ga, RECOVERY_SLOW, &[10]).unwrap();
            set(m, ga, UNIT_FIELD_MAXPOWER1, 1_000);
            pass(m, 1_000);
            let refreshed = mana(m);
            pass(m, 3_000);
            let slow = mana(m);
            // Standing, cancelling.
            fresh(m, 100, RECOVERY_SEATED, 10);
            m.now += Duration::from_millis(500);
            auras::interrupt(m, ga, INTERRUPT_NOT_SEATED);
            pass(m, 500);
            let stood = (mana(m), has(m, ga, RECOVERY_SEATED));
            fresh(m, 100, RECOVERY, 10);
            m.now += Duration::from_millis(500);
            auras::cancel(m, ga, RECOVERY);
            pass(m, 500);
            let cancelled = mana(m);
            // No mana: nothing.
            fresh(m, 0, RECOVERY, 10);
            set(m, ga, UNIT_FIELD_MAXPOWER1, 0);
            pass(m, 1_000);
            let manaless = mana(m);
            (
                (early, first, bigger),
                (capped, full),
                last,
                (stacked, none),
                (refreshed, slow),
                (stood, cancelled, manaless),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        got.0,
        (100, (200, 51.0), 400),
        "a tenth a second, of the maximum"
    );
    assert_eq!(
        got.1,
        ((1_000, 15.0), 0.0),
        "capped, the threat of what it gave"
    );
    assert_eq!(got.2, (true, false), "five ticks, the last at its end");
    assert_eq!(
        got.3,
        (300, vec![100, 100]),
        "stacks, and nothing of nothing"
    );
    assert_eq!(got.4, (100, 200), "the refresh restarts its four seconds");
    assert_eq!(got.5, ((100, false), 100, 0), "stood, cancelled, manaless");
    let told = a.wait_for("SMSG_PERIODICAURALOG", mana_log).await;
    assert_eq!(told, 100);
}
