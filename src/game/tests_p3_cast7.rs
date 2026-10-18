//! Phase 3's proc rules, as Thistle Tea's `proc_test` has them: what
//! outcomes and spells a `spell_proc_event` rule lets set a proc off.
//! Formulas, in the fast suite.

use crate::game::data::spells::{ProcEvent, SpellTemplate};
use crate::game::map::procs::{
    EX_ABSORB, EX_BLOCK, EX_CAST_END, EX_CRITICAL_HIT, EX_MISS, EX_NORMAL_HIT, EX_RESIST,
    EX_TRIGGER_ALWAYS, outcome_allowed, spell_allowed,
};

fn rule(ex: u32) -> ProcEvent {
    ProcEvent {
        ex,
        ..Default::default()
    }
}

/// A cast's end and a hit are told apart, by the rule's own cast-end bit
/// and even with its always-trigger: no rule, a hit's or always's, take a
/// hit and not a cast's end; a cast-end rule takes its end and nothing
/// else (Thistle Tea's `Proc.eligible?`).
#[test]
fn a_casts_end_and_a_hit_are_procs_apart() {
    for r in [
        None,
        Some(rule(EX_NORMAL_HIT)),
        Some(rule(EX_TRIGGER_ALWAYS)),
    ] {
        let r = r.as_ref();
        assert!(outcome_allowed(r, EX_NORMAL_HIT));
        assert!(!outcome_allowed(r, EX_CAST_END));
        assert!(!outcome_allowed(r, EX_CAST_END | EX_NORMAL_HIT));
    }
    for flags in [
        EX_CAST_END,
        EX_CAST_END | EX_TRIGGER_ALWAYS,
        EX_CAST_END | EX_NORMAL_HIT,
    ] {
        let r = Some(rule(flags));
        assert!(outcome_allowed(r.as_ref(), EX_CAST_END), "{flags:#x}");
        for ex in [EX_NORMAL_HIT, EX_CRITICAL_HIT, EX_MISS, EX_RESIST] {
            assert!(!outcome_allowed(r.as_ref(), ex), "{flags:#x} on {ex:#x}");
        }
    }
}

/// A partial block, a hit and a block (absorbed or not), meets a hit's
/// rule and a block's, not a crit's; a whole block only a block's; an
/// absorb rule wants the absorb, whatever else (Thistle Tea's blocks and
/// absorbs).
#[test]
fn a_partial_block_is_a_hit_and_a_block_and_an_absorb_rule_wants_an_absorb() {
    for ex in [
        EX_NORMAL_HIT | EX_BLOCK,
        EX_NORMAL_HIT | EX_BLOCK | EX_ABSORB,
    ] {
        for r in [None, Some(rule(EX_NORMAL_HIT)), Some(rule(EX_BLOCK))] {
            assert!(outcome_allowed(r.as_ref(), ex), "{ex:#x}");
        }
        assert!(!outcome_allowed(Some(&rule(EX_CRITICAL_HIT)), ex));
    }
    assert!(
        !outcome_allowed(None, EX_BLOCK),
        "a whole block isn't a hit"
    );
    assert!(outcome_allowed(Some(&rule(EX_BLOCK)), EX_BLOCK));
    let absorb = Some(rule(EX_ABSORB));
    for ex in [
        EX_ABSORB | EX_NORMAL_HIT,
        EX_ABSORB | EX_CRITICAL_HIT,
        EX_ABSORB | EX_BLOCK | EX_NORMAL_HIT,
    ] {
        assert!(outcome_allowed(absorb.as_ref(), ex), "{ex:#x}");
    }
    for ex in [
        EX_NORMAL_HIT,
        EX_CRITICAL_HIT,
        EX_BLOCK,
        EX_BLOCK | EX_NORMAL_HIT,
    ] {
        assert!(!outcome_allowed(absorb.as_ref(), ex), "{ex:#x}");
    }
}

fn spell(school: u32, family: u32, flags: u64) -> SpellTemplate {
    SpellTemplate {
        school,
        family_name: family,
        family_flags: flags,
        ..Default::default()
    }
}

/// A rule's school: a fire rule takes fire, not frost; a swing is
/// physical, so a rule of physical (alone or among others) takes it and
/// a magic one doesn't. Its family masks, the upper 32 bits too: a trap
/// talent's (the hunter's 0x14) takes its traps, not another trap nor
/// another family's (Thistle Tea's school and family rules).
#[test]
fn a_rules_school_and_family_mask_pick_the_spells_it_takes() {
    let fire = ProcEvent {
        school_mask: 0x4,
        ..Default::default()
    };
    assert!(spell_allowed(Some(&fire), Some(&spell(2, 0, 0)), 0x4));
    assert!(!spell_allowed(Some(&fire), Some(&spell(4, 0, 0)), 0x10));
    for mask in [1, 5, 127] {
        let r = ProcEvent {
            school_mask: mask,
            ..Default::default()
        };
        assert!(spell_allowed(Some(&r), None, 0), "a swing, {mask}");
        assert!(spell_allowed(Some(&r), None, 1), "a swing, {mask}");
    }
    let magic = ProcEvent {
        school_mask: 0x4,
        ..Default::default()
    };
    assert!(!spell_allowed(Some(&magic), None, 0), "a swing isn't fire");
    let high = ProcEvent {
        family: 3,
        family_mask: [0x4 << 32, 0, 0],
        ..Default::default()
    };
    assert!(spell_allowed(
        Some(&high),
        Some(&spell(2, 3, 0x4 << 32)),
        0x4
    ));
    assert!(!spell_allowed(
        Some(&high),
        Some(&spell(2, 3, 0x8 << 32)),
        0x4
    ));
    let trap = ProcEvent {
        family: 9,
        family_mask: [0x14, 0, 0],
        ..Default::default()
    };
    for flags in [0x4, 0x10] {
        assert!(spell_allowed(Some(&trap), Some(&spell(2, 9, flags)), 0x4));
    }
    assert!(!spell_allowed(Some(&trap), Some(&spell(2, 9, 0x8)), 0x4));
    assert!(!spell_allowed(Some(&trap), Some(&spell(2, 3, 0x4)), 0x4));
    assert!(!spell_allowed(Some(&trap), None, 0), "nor a swing");
}
