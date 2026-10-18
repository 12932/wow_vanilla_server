//! Phase 3's aura syncs, Thistle Tea's `ModifierSyncTest` and
//! `DummyFlagsTest` ported: the spell modifier totals a client is told,
//! and Stoned's flag on a creature, not a player.

use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::auras::{self, *};
use crate::game::map::spells::mods;
use crate::game::tests_p3_aura2::{add_spells, aura_spell, put};
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};

/// Damage modifiers: flat of family bits 0 and 2, flat of bit 0, flat
/// of bit 0 that stacks to three, percent of bit 30.
const FIRST: u32 = 97_000;
const SECOND: u32 = 97_001;
const STACKING: u32 = 97_002;
const PERCENT: u32 = 97_003;
/// Stoned, its unselectable flag, and an immunity flag.
const STONED: u32 = 10255;
const NOT_SELECTABLE: u32 = 0x0200_0000;
const IMMUNE_FLAG: u32 = 0x0000_0100;

fn data() -> GameData {
    let mut d = crate::game::tests_p3_aura2::data();
    let op = mods::OP_DAMAGE as i32;
    let mut all = Vec::new();
    for (id, kind, mask, stacks) in [
        (FIRST, AURA_ADD_FLAT_MODIFIER, 0b101u64, 0),
        (SECOND, AURA_ADD_FLAT_MODIFIER, 0b001, 0),
        (STACKING, AURA_ADD_FLAT_MODIFIER, 0b001, 3),
        (PERCENT, AURA_ADD_PCT_MODIFIER, 1 << 30, 0),
    ] {
        let mut s = aura_spell(id, None, &[(kind, op, 0)]);
        s.family_name = 3;
        s.effects[0].item_type = mask;
        s.stack_amount = stacks;
        all.push(s);
    }
    all.push(aura_spell(STONED, None, &[(AURA_DUMMY, 0, 0)]));
    add_spells(&mut d, all);
    d
}

/// The next modifier `c` is told of: its bit, operation and amount.
async fn told(c: &mut Client, opcode: u16) -> (u8, u8, i32) {
    let b = c
        .wait_frame("SMSG_SET_SPELL_MODIFIER", opcode, |_| true)
        .await;
    (b[0], b[1], i32::from_le_bytes([b[2], b[3], b[4], b[5]]))
}

/// Thistle Tea's `ModifierSync`: a client is told each changed family
/// bit's total, the other modifiers on it summed in (-100 on bits 0 and 2,
/// then -300 on bit 0), each stack counted (-300 of three of -100), and
/// zero when the last goes.
#[tokio::test]
async fn the_client_is_told_each_bits_modifier_total() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Mage).await;
    let op = mods::OP_DAMAGE as u8;
    a.clear();
    map.query(move |m| put(m, ga, ga, FIRST, &[-100]).unwrap())
        .await
        .unwrap();
    assert_eq!(told(&mut a, 0x266).await, (0, op, -100));
    assert_eq!(told(&mut a, 0x266).await, (2, op, -100));
    map.query(move |m| put(m, ga, ga, SECOND, &[-200]).unwrap())
        .await
        .unwrap();
    assert_eq!(told(&mut a, 0x266).await, (0, op, -300), "the total");
    // Each stack counted: three of -100 make -300 on bit 0.
    let stacked = map
        .query(move |m| {
            auras::remove_where(m, ga, |_| true);
            for _ in 0..3 {
                put(m, ga, ga, STACKING, &[-100]).unwrap();
            }
            let mut s = m.realm.data.spells.get(FIRST).unwrap().clone();
            s.family_flags = 1;
            mods::for_spell(m, ga, &s).flat[mods::OP_DAMAGE]
        })
        .await
        .unwrap();
    assert_eq!(stacked, -300, "each stack counted");
    a.clear();
    map.query(move |m| {
        auras::remove_where(m, ga, |_| true);
        put(m, ga, ga, PERCENT, &[-100]).unwrap();
        auras::remove_spell(m, ga, PERCENT, None);
    })
    .await
    .unwrap();
    assert_eq!(told(&mut a, 0x267).await, (30, op, -100));
    assert_eq!(told(&mut a, 0x267).await, (30, op, 0), "zero as it goes");
}

/// Thistle Tea's `DummyFlags`: Stoned leaves a creature unselectable, one
/// already so (and immune) just as it was; a player it leaves selectable.
#[tokio::test]
async fn stoned_marks_a_creature_unselectable_and_not_a_player() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Mage).await;
    let got = map
        .query(move |m| {
            let flags = |m: &crate::game::map::Map, g| {
                m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_FLAGS))
            };
            let plain = spawn(m, ga, BRUTE, 20, 5.0);
            put(m, ga, plain, STONED, &[0]).unwrap();
            let statue = spawn(m, ga, BRUTE, 20, 5.0);
            if let Some(c) = m.creature_mut(statue) {
                let f = c.fields.get_u32(UNIT_FIELD_FLAGS);
                c.fields
                    .set_u32(UNIT_FIELD_FLAGS, f | NOT_SELECTABLE | IMMUNE_FLAG);
            }
            let before = flags(m, statue);
            put(m, ga, statue, STONED, &[0]).unwrap();
            let player_before = flags(m, ga);
            put(m, ga, ga, STONED, &[0]).unwrap();
            (
                flags(m, plain) & NOT_SELECTABLE,
                (before, flags(m, statue)),
                (player_before, flags(m, ga)),
            )
        })
        .await
        .unwrap();
    assert_eq!(got.0, NOT_SELECTABLE);
    assert_eq!(got.1.0, got.1.1, "as it was");
    assert_eq!(got.2.0, got.2.1, "a player's flags untouched");
}
