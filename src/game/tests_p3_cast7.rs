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

/// `CMSG_CAST_SPELL`'s targets as the client lays them out, read as the
/// session reads them: the spell, then the flags and each field in its
/// order.
fn read_targets(flags: u16, fields: &[u8]) -> Option<crate::game::map::spells::Targets> {
    use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
    let mut body = 116u32.to_le_bytes().to_vec();
    body.extend(flags.to_le_bytes());
    body.extend(fields);
    let mut plain = ((body.len() + 4) as u16).to_be_bytes().to_vec();
    plain.extend(0x12Eu32.to_le_bytes());
    plain.extend(body);
    match ClientOpcodeMessage::read_unencrypted(&plain[..]).ok()? {
        ClientOpcodeMessage::CMSG_CAST_SPELL(c) => {
            Some(crate::game::map::spells::targets_of(&c.targets))
        }
        _ => None,
    }
}

/// A packed GUID's bytes.
fn packed(g: u64) -> Vec<u8> {
    let mut mask = 0u8;
    let mut out = vec![0];
    for (i, b) in g.to_le_bytes().iter().enumerate() {
        if *b != 0 {
            mask |= 1 << i;
            out.push(*b);
        }
    }
    out[0] = mask;
    out
}

fn floats(v: [f32; 3]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// The targets of a cast as the client sends them: an item's GUID, a
/// destination's three floats, a unit and a destination in their order,
/// a corpse alone as an object; and what the server writes of a unit and a
/// destination reads back as it was (Thistle Tea's `TargetCodec`).
#[test]
fn a_casts_targets_read_from_the_packet_and_round_trip() {
    use wow_world_messages::Guid;
    let item = read_targets(0x10, &packed(0x4000_00AA)).unwrap();
    assert_eq!(item.item, Some(Guid::new(0x4000_00AA)));
    let dest = read_targets(0x40, &floats([1.0, 2.0, 3.0])).unwrap();
    let d = dest.dest.unwrap();
    assert_eq!((d.x, d.y, d.z), (1.0, 2.0, 3.0));
    let mut both = packed(0xAABBCC);
    both.extend(floats([1.0, 2.0, 3.0]));
    let both = read_targets(0x42, &both).unwrap();
    assert_eq!(both.unit, Some(Guid::new(0xAABBCC)));
    assert!(
        both.dest
            .is_some_and(|d| (d.x, d.y, d.z) == (1.0, 2.0, 3.0))
    );
    // A corpse alone (wow_messages reads a corpse's GUID after the
    // locations, where 1.12 sends it before them).
    for flags in [0x0200u16, 0x8000] {
        let corpse = 0xF000_0000_0000_002A;
        let t = read_targets(flags, &packed(corpse)).unwrap();
        assert_eq!(t.object, Some(Guid::new(corpse)), "a corpse, {flags:#x}");
    }
    let back = crate::game::map::spells::targets_of(&crate::game::map::spells::wire_targets(&both));
    assert_eq!(back.unit, both.unit);
    assert!(
        back.dest
            .is_some_and(|d| (d.x, d.y, d.z) == (1.0, 2.0, 3.0))
    );
}
