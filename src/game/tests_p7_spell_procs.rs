//! Round 7, against Thistle Tea's `cast_proc_test.exs` and
//! `stacking_proc_test.exs`: what procs on a cast's end (vmangos's
//! `PROC_EX_CAST_END`) rather than on its hits, once a cast: a charged
//! proc, Unstable Power's stacks and Ascendance's.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, has, holder, pass};
use crate::game::data::GameData;
use crate::game::data::spells::{
    ATTR_EX_CHANNELED, ATTR_EX3_NOT_A_PROC, ATTR_EX3_SUPPRESS_CASTER_PROCS, ProcEvent, SpellEffect,
};
use crate::game::map::Map;
use crate::game::map::auras::*;
use crate::game::map::procs::{self, EX_CAST_END};
use crate::game::map::spells::{self, Targets};
use crate::game::map::trinkets::{
    ASCENDANCE, ASCENDANCE_BONUS, UNSTABLE_POWER, UNSTABLE_POWER_BONUS,
};

/// A charged proc on a cast's end, and the mark it puts on its target; a
/// frost bolt, a frost nova of ten yards, a three-second frost cast, a
/// three-second frost channel ticking a second, a self buff, and a frost
/// bolt that procs nothing of its caster's.
const ON_CAST: u32 = 997_101;
const MARK: u32 = 997_102;
const BOLT: u32 = 997_103;
const NOVA: u32 = 997_104;
const SLOW_BOLT: u32 = 997_105;
const CHANNEL: u32 = 997_106;
const CHANNEL_TICK: u32 = 997_107;
const BUFF: u32 = 997_108;
const QUIET_BOLT: u32 = 997_109;
/// What a target can't be harmed by: every school.
const UNTOUCHABLE: u32 = 997_110;
/// A fire bolt, a fire nova, a fire heal, a fire dot, a strength buff of
/// fire, a physical strike, Searing Totem's look, and a script effect.
const FIRE_BOLT: u32 = 997_120;
const FIRE_NOVA: u32 = 997_121;
const FIRE_HEAL: u32 = 997_122;
const FIRE_DOT: u32 = 997_123;
const FIRE_STAT: u32 = 997_124;
const STRIKE: u32 = 997_125;
const SEARING: u32 = 997_126;
const FIRE_SCRIPT: u32 = 997_127;
/// A channel that is no proc's own (`SPELL_ATTR_EX3_NOT_A_PROC`).
const NOT_A_PROC_CHANNEL: u32 = 997_111;
const RADIUS_10: u32 = 997;

/// The cast-end rule of the four real ones (Unstable Power's, Ascendance's,
/// Elemental Focus's, the Blue Dragon's).
fn cast_end() -> ProcEvent {
    ProcEvent {
        ex: EX_CAST_END,
        ..Default::default()
    }
}

fn data(d: &mut GameData) {
    d.spells.radii.insert(RADIUS_10, 10.0);
    let mut on_cast = bare(ON_CAST, 0);
    on_cast.duration_index = LASTS_LONG;
    on_cast.proc_flags = procs::PROC_DONE_POSITIVE_SPELL
        | procs::PROC_DONE_NEGATIVE_SPELL
        | procs::PROC_DONE_AOE
        | procs::PROC_DONE_POSITIVE_AOE;
    on_cast.proc_chance = 100;
    on_cast.proc_charges = 3;
    effect(
        &mut on_cast,
        0,
        SpellEffect {
            trigger_spell: MARK,
            ..aura(AURA_PROC_TRIGGER_SPELL, 0, 0, 1)
        },
    );
    d.spells.proc_events.insert(ON_CAST, cast_end());
    let mut mark = bare(MARK, 0);
    mark.duration_index = LASTS_LONG;
    mark.stack_amount = 10;
    effect(&mut mark, 0, aura(AURA_DUMMY, 0, 0, 25));
    let mut bolt = bare(BOLT, 4);
    effect(&mut bolt, 0, damage(10));
    let mut nova = bare(NOVA, 4);
    effect(
        &mut nova,
        0,
        SpellEffect {
            target_a: 15,
            radius_index: RADIUS_10,
            ..damage(10)
        },
    );
    let mut slow = bolt.clone();
    slow.id = SLOW_BOLT;
    slow.casting_time_index = CAST_3S;
    let mut channel = bare(CHANNEL, 4);
    channel.attributes[1] |= ATTR_EX_CHANNELED;
    channel.duration_index = 902;
    d.spells.durations.insert(
        902,
        crate::game::data::spells::SpellDuration {
            base: 3_000,
            per_level: 0,
            max: 3_000,
        },
    );
    effect(
        &mut channel,
        0,
        SpellEffect {
            trigger_spell: CHANNEL_TICK,
            amplitude: 1_000,
            ..aura(AURA_PERIODIC_TRIGGER_SPELL, 0, 0, 6)
        },
    );
    let mut tick = bare(CHANNEL_TICK, 4);
    effect(&mut tick, 0, damage(10));
    let mut not_a_proc = channel.clone();
    not_a_proc.id = NOT_A_PROC_CHANNEL;
    not_a_proc.attributes[3] |= ATTR_EX3_NOT_A_PROC;
    let mut buff = bare(BUFF, 0);
    buff.duration_index = LASTS_LONG;
    effect(&mut buff, 0, aura(AURA_MOD_STAT, 0, 5, 1));
    let mut quiet = bolt.clone();
    quiet.id = QUIET_BOLT;
    quiet.attributes[3] |= ATTR_EX3_SUPPRESS_CASTER_PROCS;
    let mut untouchable = bare(UNTOUCHABLE, 0);
    untouchable.duration_index = LASTS_LONG;
    effect(&mut untouchable, 0, aura(AURA_SCHOOL_IMMUNITY, 127, 0, 1));
    insert(
        d,
        [
            on_cast,
            mark,
            bolt,
            nova,
            slow,
            channel,
            tick,
            buff,
            quiet,
            untouchable,
            not_a_proc,
        ],
    );
    trinkets(d);
}

/// Unstable Power and Ascendance as the real data has them: the parent's
/// dummy for two hours, the bonus of fire spell power (17 a stack, twelve
/// of them; 40 a stack for Ascendance), the cast-end rule on 24659 and
/// 28200 with every kind of spell done.
fn trinkets(d: &mut GameData) {
    let done = 0x15550;
    let mut unstable = bare(UNSTABLE_POWER, 0);
    unstable.duration_index = LASTS_LONG;
    effect(&mut unstable, 0, aura(AURA_DUMMY, 0, 0, 1));
    let mut unstable_bonus = bare(UNSTABLE_POWER_BONUS, 0);
    unstable_bonus.duration_index = LASTS_LONG;
    unstable_bonus.stack_amount = 12;
    unstable_bonus.proc_flags = done;
    unstable_bonus.proc_chance = 100;
    effect(
        &mut unstable_bonus,
        0,
        aura(AURA_MOD_DAMAGE_DONE, 0x7e, 17, 1),
    );
    d.spells
        .proc_events
        .insert(UNSTABLE_POWER_BONUS, cast_end());
    let mut ascendance = bare(ASCENDANCE, 0);
    ascendance.duration_index = LASTS_LONG;
    ascendance.proc_flags = done;
    ascendance.proc_chance = 100;
    effect(
        &mut ascendance,
        0,
        SpellEffect {
            trigger_spell: ASCENDANCE_BONUS,
            ..aura(AURA_PROC_TRIGGER_SPELL, 0, 0, 1)
        },
    );
    d.spells.proc_events.insert(ASCENDANCE, cast_end());
    let mut ascendance_bonus = bare(ASCENDANCE_BONUS, 0);
    ascendance_bonus.duration_index = LASTS_LONG;
    ascendance_bonus.stack_amount = 5;
    effect(
        &mut ascendance_bonus,
        0,
        aura(AURA_MOD_DAMAGE_DONE, 0x7e, 40, 1),
    );
    let fire = |id: u32, e: SpellEffect| {
        let mut s = bare(id, 2);
        effect(&mut s, 0, e);
        s
    };
    let mut strike = fire(STRIKE, damage(10));
    strike.school = 0;
    strike.dmg_class = 2;
    let mut searing = fire(
        SEARING,
        SpellEffect {
            effect: 87,
            target_a: 1,
            ..Default::default()
        },
    );
    searing.spell_visual = 319;
    searing.spell_icon = 1647;
    let mut heal = fire(
        FIRE_HEAL,
        SpellEffect {
            effect: 10,
            base_points: 9,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        },
    );
    heal.dmg_class = 1;
    let mut dot = fire(FIRE_DOT, aura(AURA_PERIODIC_DAMAGE, 0, 5, 6));
    dot.duration_index = LASTS_5S;
    dot.effects[0].amplitude = 1_000;
    let mut stat = fire(FIRE_STAT, aura(AURA_MOD_STAT, 0, 5, 1));
    stat.duration_index = LASTS_5S;
    insert(
        d,
        [
            unstable,
            unstable_bonus,
            ascendance,
            ascendance_bonus,
            fire(FIRE_BOLT, damage(10)),
            fire(
                FIRE_NOVA,
                SpellEffect {
                    target_a: 15,
                    radius_index: RADIUS_10,
                    ..damage(10)
                },
            ),
            heal,
            dot,
            stat,
            strike,
            searing,
            fire(
                FIRE_SCRIPT,
                SpellEffect {
                    effect: 77,
                    target_a: 6,
                    ..Default::default()
                },
            ),
        ],
    );
}

fn charges(m: &Map, g: Guid, spell: u32) -> u32 {
    holder(m, g, spell).map_or(0, |h| h.charges)
}

fn stacks(m: &Map, g: Guid, spell: u32) -> u32 {
    holder(m, g, spell).map_or(0, |h| h.stacks.max(1))
}

/// Marks on `g`: one a proc.
fn marks(m: &Map, g: Guid) -> u32 {
    holder(m, g, MARK).map_or(0, |h| h.stacks.max(1))
}

/// A charged proc on a cast's end goes off once a cast, however many it
/// lands on, and the hits after spend nothing more: a nova on two kobolds
/// marks the first named once, a charge gone; a bolt at a kobold immune to
/// it still goes off; a cast begun and cancelled spends nothing; a channel
/// goes off as it begins, not at its ticks nor as it ends; a bolt that
/// procs nothing of its caster's doesn't; a buff at no one marks its
/// caster (Thistle Tea's `Casting.complete`'s cast-completion procs).
#[tokio::test]
async fn a_cast_ends_once_whatever_it_lands_on() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7SP1", "Caster", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(
                m,
                a,
                60,
                &[BOLT, NOVA, SLOW_BOLT, CHANNEL, BUFF, QUIET_BOLT],
            );
            let first = kit::dummy(m, a, 60, 3.0);
            let second = kit::dummy(m, a, 60, 4.0);
            kit::apply(m, a, a, ON_CAST);
            let mut out = Vec::new();
            // An area's.
            cast_fresh(m, a, NOVA, at(first));
            out.push((charges(m, a, ON_CAST), marks(m, first), marks(m, second)));
            // A fully avoided one's.
            kit::apply(m, second, second, UNTOUCHABLE);
            cast_fresh(m, a, BOLT, at(second));
            out.push((charges(m, a, ON_CAST), marks(m, first), marks(m, second)));
            // Begun and cancelled.
            cast_fresh(m, a, SLOW_BOLT, at(first));
            pass(m, 1_000);
            spells::interrupt(m, a);
            pass(m, 3_000);
            out.push((charges(m, a, ON_CAST), marks(m, first), marks(m, second)));
            // A channel's: at its start only.
            cast_fresh(m, a, CHANNEL, at(first));
            let at_start = (charges(m, a, ON_CAST), marks(m, first));
            for _ in 0..4 {
                pass(m, 1_000);
            }
            out.push((charges(m, a, ON_CAST), marks(m, first), at_start.1));
            (out, at_start, kit::health(m, first) < 1_000_000 - 10)
        })
        .await
        .unwrap();
    let (out, at_start, ticked) = out;
    assert_eq!(out[0], (2, 1, 0), "an area: once, at the unit named");
    assert_eq!(out[1], (1, 1, 1), "avoided: still once");
    assert_eq!(out[2], (1, 1, 1), "cancelled: nothing");
    assert_eq!(at_start, (0, 2), "the channel's start: its last charge");
    assert_eq!(out[3], (0, 2, 2), "not at its ticks nor its end");
    assert!(ticked, "the channel ticked");
}

/// A spell that procs nothing of its caster's doesn't go off on its end;
/// a helpful cast at no one goes off at its caster; a helpful item's
/// doesn't, nor a triggered bolt, but a triggered channel that is no
/// proc's own does (Thistle Tea's "caster-suppressed spells cannot
/// trigger", "helpful casts with no unit target use the caster for their
/// proc", "positive item casts ... cannot trigger" and "triggered
/// channels honor aura origin and the NOT_A_PROC exception").
#[tokio::test]
async fn a_suppressed_cast_procs_nothing_and_a_buff_procs_at_its_caster() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7SP2", "Buffer", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[BUFF, QUIET_BOLT]);
            let kobold = kit::dummy(m, a, 60, 3.0);
            kit::apply(m, a, a, ON_CAST);
            cast_fresh(m, a, QUIET_BOLT, at(kobold));
            let quiet = (charges(m, a, ON_CAST), marks(m, kobold));
            cast_fresh(m, a, BUFF, Targets::default());
            let buff = (charges(m, a, ON_CAST), marks(m, a));
            // From an item, a helpful spell procs nothing.
            kit::fresh(m, a);
            spells::cast(
                m,
                a,
                BUFF,
                Targets::default(),
                Some(Guid::new(0x4000_0000_0000_0123)),
            );
            let item = charges(m, a, ON_CAST);
            // Triggered, a bolt procs nothing; a channel that is no
            // proc's own does.
            spells::cast_triggered(m, a, BOLT, Some(kobold));
            let triggered = charges(m, a, ON_CAST);
            spells::interrupt(m, a);
            spells::cast_triggered(m, a, NOT_A_PROC_CHANNEL, Some(kobold));
            (quiet, buff, item, triggered, charges(m, a, ON_CAST))
        })
        .await
        .unwrap();
    assert_eq!(out, ((3, 0), (2, 1), 2, 2, 1));
}

/// Unstable Power starts at twelve stacks and loses one a cast, a nova on
/// two at once one, until it is gone and its parent stays; its fire power
/// is had by every target of the cast that spends it (Thistle Tea's "one
/// stack per cast through exhaustion" and "all area recipients snapshot
/// Unstable Power before its one-stack reduction").
#[tokio::test]
async fn unstable_power_loses_a_stack_a_cast_until_its_gone() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7SP3", "Unstable", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[FIRE_BOLT, FIRE_NOVA]);
            let first = kit::dummy(m, a, 60, 3.0);
            let second = kit::dummy(m, a, 60, 4.0);
            spells::cast_triggered(m, a, UNSTABLE_POWER, Some(a));
            let full = stacks(m, a, UNSTABLE_POWER_BONUS);
            cast_fresh(m, a, FIRE_NOVA, at(first));
            let nova = (
                stacks(m, a, UNSTABLE_POWER_BONUS),
                1_000_000 - kit::health(m, first),
                1_000_000 - kit::health(m, second),
            );
            let mut each = Vec::new();
            for _ in 0..11 {
                cast_fresh(m, a, FIRE_BOLT, at(first));
                each.push(stacks(m, a, UNSTABLE_POWER_BONUS));
            }
            (full, nova, each, has(m, a, UNSTABLE_POWER))
        })
        .await
        .unwrap();
    let (full, (after_nova, first, second), each, parent) = out;
    assert_eq!(full, 12);
    assert_eq!(after_nova, 11, "one for the nova");
    assert_eq!(first, second, "both hit with the same power");
    assert!(first > 10, "with the bonus: {first}");
    assert_eq!(each, vec![10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0]);
    assert!(parent, "the parent stays");
}

/// Unstable Power is spent by a fire spell that harms or heals (a bolt, a
/// nova, a heal, a dot, Searing Totem's kind), not by a physical strike or
/// a fire buff of a stat (Thistle Tea's "damage, healing, periodic
/// spells, area casts and supported totems consume once").
#[tokio::test]
async fn unstable_power_is_spent_by_what_harms_or_heals() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7SP4", "Spender", Class::Mage).await;
    let out = map
        .query(move |m| {
            let all = [
                FIRE_BOLT, FIRE_NOVA, FIRE_HEAL, FIRE_DOT, SEARING, STRIKE, FIRE_STAT,
            ];
            kit::ready(m, a, 60, &all);
            let kobold = kit::dummy(m, a, 60, 3.0);
            all.map(|spell| {
                remove_where(m, a, |h| !h.passive);
                spells::cast_triggered(m, a, UNSTABLE_POWER, Some(a));
                let t = if matches!(spell, FIRE_HEAL | SEARING | FIRE_STAT) {
                    Targets::default()
                } else {
                    at(kobold)
                };
                cast_fresh(m, a, spell, t);
                stacks(m, a, UNSTABLE_POWER_BONUS)
            })
        })
        .await
        .unwrap();
    assert_eq!(out, [11, 11, 11, 11, 11, 12, 12]);
}

/// Ascendance holds six charges: each of five casts gives a stack of its
/// bonus, the sixth takes both away; an area spell or a script effect
/// counts for nothing, and its bonus doesn't go on without it (Thistle
/// Tea's "grows to five stacks and the sixth cast removes both holders",
/// "rejects area and script-effect casts", and "late triggered bonuses
/// cannot restore an ended parent").
#[tokio::test]
async fn ascendance_grows_five_casts_and_goes_at_the_sixth() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7SP5", "Ascendant", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[FIRE_BOLT, FIRE_NOVA, FIRE_SCRIPT]);
            let kobold = kit::dummy(m, a, 60, 3.0);
            spells::cast_triggered(m, a, ASCENDANCE_BONUS, Some(a));
            let orphan = has(m, a, ASCENDANCE_BONUS);
            spells::cast_triggered(m, a, UNSTABLE_POWER_BONUS, Some(a));
            let orphan = (orphan, has(m, a, UNSTABLE_POWER_BONUS));
            kit::apply(m, a, a, ASCENDANCE);
            let start = (charges(m, a, ASCENDANCE), has(m, a, ASCENDANCE_BONUS));
            cast_fresh(m, a, FIRE_NOVA, at(kobold));
            cast_fresh(m, a, FIRE_SCRIPT, at(kobold));
            let ignored = (charges(m, a, ASCENDANCE), has(m, a, ASCENDANCE_BONUS));
            let mut each = Vec::new();
            for _ in 0..5 {
                cast_fresh(m, a, FIRE_BOLT, at(kobold));
                each.push((charges(m, a, ASCENDANCE), stacks(m, a, ASCENDANCE_BONUS)));
            }
            cast_fresh(m, a, FIRE_BOLT, at(kobold));
            let sixth = (has(m, a, ASCENDANCE), has(m, a, ASCENDANCE_BONUS));
            (orphan, start, ignored, each, sixth)
        })
        .await
        .unwrap();
    let (orphan, start, ignored, each, sixth) = out;
    assert_eq!(orphan, (false, false), "no bonus without its trinket");
    assert_eq!(start, (6, false));
    assert_eq!(
        ignored,
        (6, false),
        "an area's and a script's count for nothing"
    );
    assert_eq!(each, vec![(5, 1), (4, 2), (3, 3), (2, 4), (1, 5)]);
    assert_eq!(sixth, (false, false), "the sixth takes both");
}

/// With the real data, Unstable Power (24659's cast-end rule) loses a
/// stack to a Fire Blast and one to a Frost Nova on two (the matrix's
/// "with the real data wvs never spends one").
#[tokio::test]
#[ignore = "extended"]
async fn with_the_real_data_unstable_power_loses_a_stack_a_cast() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (_c, a, map) = kit::player(&realm, "P7SP6", "Realpower", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[2136, 122]);
            let first = kit::dummy(m, a, 60, 3.0);
            let _second = kit::dummy(m, a, 60, 4.0);
            spells::cast_triggered(m, a, UNSTABLE_POWER, Some(a));
            let full = stacks(m, a, UNSTABLE_POWER_BONUS);
            cast_fresh(m, a, 2136, at(first));
            let blast = stacks(m, a, UNSTABLE_POWER_BONUS);
            cast_fresh(m, a, 122, Targets::default());
            (full, blast, stacks(m, a, UNSTABLE_POWER_BONUS))
        })
        .await
        .unwrap();
    assert_eq!(out, (12, 11, 10));
}
