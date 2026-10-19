//! Round 7, against Thistle Tea's `proc_origin_test.exs`,
//! `proc_origin_logic_test.exs` and `proc_test.exs`: which side of a blow
//! procs, on which kind of blow, and what a proc spends.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, holder, tick_now};
use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_EX3_SUPPRESS_TARGET_PROCS, ProcEvent, SpellEffect};
use crate::game::map::Map;
use crate::game::map::auras::*;
use crate::game::map::procs::{self, Event};
use crate::game::map::spells;

/// A proc on spells dealt and taken and periodic damage dealt (Thistle
/// Tea's 0x74010), of three charges and three seconds' cooldown, and the
/// mark it gives; a bolt, the same sparing its target's procs, and a dot.
const ON_SPELLS: u32 = 997_301;
const MARK: u32 = 997_302;
const BOLT: u32 = 997_303;
const SPARING_BOLT: u32 = 997_304;
const DOT: u32 = 997_305;
/// A proc on ranged blows dealt and taken (0x3C0); one on periodic
/// effects done with vmangos's rule for heals' ticks; one on them without
/// a rule; a heal over time.
const ON_RANGED: u32 = 997_306;
const ON_HOT_TICKS: u32 = 997_307;
const ON_TICKS: u32 = 997_308;
const HOT: u32 = 997_309;

fn proc_aura(id: u32, flags: u32, charges: u32) -> crate::game::data::spells::SpellTemplate {
    let mut s = bare(id, 0);
    s.duration_index = LASTS_LONG;
    s.proc_flags = flags;
    s.proc_chance = 100;
    s.proc_charges = charges;
    effect(
        &mut s,
        0,
        SpellEffect {
            trigger_spell: MARK,
            ..aura(AURA_PROC_TRIGGER_SPELL, 0, 0, 1)
        },
    );
    s
}

fn data(d: &mut GameData) {
    d.spells.proc_events.insert(
        ON_SPELLS,
        ProcEvent {
            cooldown: 3_000,
            ..Default::default()
        },
    );
    d.spells.proc_events.insert(
        ON_HOT_TICKS,
        ProcEvent {
            flags: procs::PROC_DONE_PERIODIC,
            ex: procs::EX_PERIODIC_POSITIVE,
            ..Default::default()
        },
    );
    let mut mark = bare(MARK, 0);
    mark.duration_index = LASTS_LONG;
    mark.stack_amount = 10;
    effect(&mut mark, 0, aura(AURA_DUMMY, 0, 0, 25));
    let mut bolt = bare(BOLT, 4);
    effect(&mut bolt, 0, damage(10));
    let mut sparing = bolt.clone();
    sparing.id = SPARING_BOLT;
    sparing.attributes[3] |= ATTR_EX3_SUPPRESS_TARGET_PROCS;
    let mut dot = bare(DOT, 5);
    dot.duration_index = LASTS_15S;
    effect(
        &mut dot,
        0,
        SpellEffect {
            amplitude: 3_000,
            ..aura(AURA_PERIODIC_DAMAGE, 0, 10, 6)
        },
    );
    let mut hot = bare(HOT, 3);
    hot.duration_index = LASTS_15S;
    effect(
        &mut hot,
        0,
        SpellEffect {
            amplitude: 3_000,
            ..aura(AURA_PERIODIC_HEAL, 0, 10, 1)
        },
    );
    insert(
        d,
        [
            proc_aura(ON_SPELLS, 0x74010, 3),
            mark,
            bolt,
            sparing,
            dot,
            proc_aura(ON_RANGED, 0x3c0, 10),
            proc_aura(ON_HOT_TICKS, procs::PROC_DONE_PERIODIC, 10),
            proc_aura(ON_TICKS, procs::PROC_DONE_PERIODIC, 10),
            hot,
        ],
    );
}

fn charges(m: &Map, g: Guid, spell: u32) -> u32 {
    holder(m, g, spell).map_or(0, |h| h.charges)
}

/// How long until `g`'s proc of `spell` may go off again, in seconds.
fn cooling(m: &Map, g: Guid, spell: u32) -> Option<u64> {
    holder(m, g, spell)
        .and_then(|h| h.proc_ready_at)
        .map(|t| t.saturating_duration_since(m.now).as_secs())
}

/// A bolt's hit spends a charge on both sides and starts both cooldowns,
/// a triggered one's as a cast's; within the cooldown another spends
/// nothing; a bolt that spares its target's procs spends the caster's
/// alone; a dot's tick procs its caster's (Thistle Tea's "ordinary and
/// exempt casts consume both sides' charges", "ordinary casts and
/// effect-triggered spells can proc", "target suppression preserves
/// caster procs", and "periodic damage does not inherit the applying aura
/// cast's suppression").
#[tokio::test]
async fn a_spells_hit_spends_both_sides_charges_once_a_cooldown() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7PS1", "Bolter", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[BOLT, SPARING_BOLT, DOT]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let fresh = |m: &mut Map| {
                for g in [a, kobold] {
                    remove_where(m, g, |h| h.spell == ON_SPELLS || h.spell == MARK);
                    kit::apply(m, g, g, ON_SPELLS);
                }
            };
            let both = |m: &Map| (charges(m, a, ON_SPELLS), charges(m, kobold, ON_SPELLS));
            fresh(m);
            cast_fresh(m, a, BOLT, at(kobold));
            let cast = (
                both(m),
                cooling(m, a, ON_SPELLS),
                cooling(m, kobold, ON_SPELLS),
            );
            cast_fresh(m, a, BOLT, at(kobold));
            let cooled = both(m);
            fresh(m);
            spells::cast_triggered(m, a, BOLT, Some(kobold));
            let triggered = both(m);
            fresh(m);
            cast_fresh(m, a, SPARING_BOLT, at(kobold));
            let sparing = both(m);
            fresh(m);
            spells::cast_triggered(m, a, DOT, Some(kobold));
            fresh(m);
            tick_now(m, kobold, DOT);
            (cast, cooled, triggered, sparing, both(m))
        })
        .await
        .unwrap();
    let (cast, cooled, triggered, sparing, tick) = out;
    assert_eq!(
        cast,
        ((2, 2), Some(3), Some(3)),
        "both, cooling three seconds"
    );
    assert_eq!(cooled, (2, 2), "nothing within the cooldown");
    assert_eq!(triggered, (2, 2), "a triggered bolt as a cast");
    assert_eq!(sparing, (2, 3), "the caster's alone");
    assert_eq!(tick.0, 2, "a dot's tick procs its caster's");
}

/// A proc on ranged blows dealt and taken goes off on a ranged attack and
/// a ranged ability, on either side, not on a melee ability (Thistle
/// Tea's "maps ranged attack and ability proc flags on both sides").
#[tokio::test]
async fn a_ranged_proc_goes_off_on_either_side_of_a_ranged_blow() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7PS2", "Archer", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            for g in [a, kobold] {
                kit::apply(m, g, g, ON_RANGED);
            }
            let blow = |m: &mut Map, done: u32, taken: u32, ex: u32| {
                procs::trigger(
                    m,
                    Event {
                        actor: a,
                        target: kobold,
                        actor_flags: done,
                        target_flags: taken,
                        ex,
                        spell: Some(BOLT),
                        school: 1,
                        damage: 10,
                        ranged: true,
                        ..Default::default()
                    },
                );
                (charges(m, a, ON_RANGED), charges(m, kobold, ON_RANGED))
            };
            [
                blow(
                    m,
                    procs::PROC_DONE_RANGED,
                    procs::PROC_TAKEN_RANGED,
                    procs::EX_NORMAL_HIT,
                ),
                blow(
                    m,
                    procs::PROC_DONE_RANGED_SPELL,
                    procs::PROC_TAKEN_RANGED_SPELL,
                    procs::EX_CRITICAL_HIT,
                ),
                blow(
                    m,
                    procs::PROC_DONE_MELEE_SPELL,
                    procs::PROC_TAKEN_MELEE_SPELL,
                    procs::EX_NORMAL_HIT,
                ),
            ]
        })
        .await
        .unwrap();
    assert_eq!(out, [(9, 9), (8, 8), (8, 8)]);
}

/// A heal over time's tick procs what vmangos's rule for heals' ticks
/// names (`PROC_EX_PERIODIC_POSITIVE`: Improved Mend Pet, the
/// Dreamwalker's Rejuvenation), not a periodic proc with no rule; a dot's
/// tick procs the one with no rule, not the heals' (Thistle Tea's
/// "requires the VMangos positive-periodic flag for HoT procs"); a heal
/// over time from an item ticks so too (its "a periodic heal applied by an
/// item retains normal tick eligibility").
#[tokio::test]
async fn a_heals_tick_procs_only_what_asks_for_it() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7PS3", "Healer", Class::Priest).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            kit::apply(m, a, a, ON_HOT_TICKS);
            kit::apply(m, a, a, ON_TICKS);
            let both = |m: &Map| (charges(m, a, ON_HOT_TICKS), charges(m, a, ON_TICKS));
            spells::cast_triggered(m, a, HOT, Some(a));
            tick_now(m, a, HOT);
            let heal = both(m);
            spells::cast_triggered(m, a, DOT, Some(kobold));
            tick_now(m, kobold, DOT);
            let dot = both(m);
            // From an item: the holder knows its item.
            remove_spell(m, a, HOT, None);
            spells::cast_triggered(m, a, HOT, Some(a));
            if let Some(x) = auras_mut(m, a)
                && let Some(h) = x.holders.iter_mut().find(|h| h.spell == HOT)
            {
                h.item = Some(Guid::new(0x4000_0000_0000_0077));
            }
            m.now += Duration::from_millis(1);
            tick_now(m, a, HOT);
            (heal, dot, both(m))
        })
        .await
        .unwrap();
    assert_eq!(out, ((9, 10), (9, 9), (8, 9)));
}
