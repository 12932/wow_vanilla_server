//! Round 7's stealth aura tests, ported from Thistle Tea's
//! `stealth_detection_test.exs` and `stealth_sync_test.exs`
//! (docs/parity/matrix.csv): who sees through stealth, and the flags a
//! mark and Beast Lore show.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::spells::stealth;
use crate::game::map::{Map, auras};
use crate::game::tests_p3_combat::{BRUTE, level, setup_with, spawn};
use crate::game::tests_p3_entity::on;

const STEALTH: u32 = 97_200;
const DETECT: u32 = 97_201;
const DETECT_STACKED: u32 = 97_202;
const DETECT_INVISIBLE: u32 = 97_203;
const STUN: u32 = 97_204;
const MARK: u32 = 97_205;
const BEAST_LORE: u32 = 97_206;

fn spell(id: u32, aura: u32, misc: i32, stack: u32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        stack_amount: stack,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura,
        misc_value: misc,
        target_a: 1,
        ..Default::default()
    };
    s
}

fn data() -> crate::game::data::GameData {
    let mut d = crate::game::tests_p3_combat::data();
    for s in [
        spell(STEALTH, auras::AURA_MOD_STEALTH, 0, 0),
        spell(DETECT, auras::AURA_MOD_STEALTH_DETECT, 0, 0),
        spell(DETECT_STACKED, auras::AURA_MOD_STEALTH_DETECT, 0, 5),
        spell(DETECT_INVISIBLE, auras::AURA_MOD_STEALTH_DETECT, 1, 0),
        spell(STUN, auras::AURA_MOD_STUN, 0, 0),
        spell(MARK, auras::AURA_MOD_STALKED, 0, 0),
        spell(BEAST_LORE, auras::AURA_EMPATHY, 0, 0),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn put(m: &mut Map, caster: Guid, target: Guid, id: u32, amount: i32) {
    let s = m.realm.data.spells.spells[&id].clone();
    auras::apply_spell(
        m,
        caster,
        target,
        &s,
        &[(0, amount)],
        auras::Context::default(),
    )
    .expect("put on");
}

/// A creature `yards` before `g`, facing it, that detects stealth by 50,
/// and by 5 twice over, and invisibility (not stealth) by 200.
fn watcher(m: &mut Map, g: Guid, yards: f32) -> Guid {
    let c = spawn(m, g, BRUTE, 20, yards);
    put(m, c, c, DETECT, 50);
    put(m, c, c, DETECT_STACKED, 5);
    auras::set_stacks(m, c, DETECT_STACKED, c, None, 2);
    put(m, c, c, DETECT_INVISIBLE, 200);
    c
}

/// Thistle Tea's `StealthDetection` "publishes detection auras for the
/// stealth type and caster-specific marks" and "publishes level-scaled
/// skill when the flattened rank amount is stale": a creature of 20
/// detects stealth by its stealth detection auras, with their stacks, and
/// not by its invisibility detection (60: it sees a level 20 player
/// stealthed by a rank of 5, skill 100 from the level, at 10 yards, not at
/// 11.5); a level 30 one (skill 150) it sees at neither; stunned, it sees
/// none; one it marked, it sees.
#[tokio::test]
async fn stealth_is_seen_by_detection_with_its_stacks_skill_by_level() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Rogue).await;
    let out = on(&map, move |m| {
        level(m, ga, 20);
        let near = watcher(m, ga, 10.0);
        let far = watcher(m, ga, 11.5);
        put(m, ga, ga, STEALTH, 5);
        let at_20 = (stealth::sees(m, near, ga), stealth::sees(m, far, ga));
        level(m, ga, 30);
        let at_30 = (stealth::sees(m, near, ga), stealth::sees(m, far, ga));
        level(m, ga, 20);
        put(m, near, near, STUN, 0);
        put(m, far, ga, MARK, 0);
        let held = (stealth::sees(m, near, ga), stealth::sees(m, far, ga));
        (at_20, at_30, held)
    })
    .await;
    let (at_20, at_30, held) = out;
    assert_eq!(at_20, (true, false), "detection 60 against skill 100");
    assert_eq!(at_30, (false, false), "skill 150 at level 30");
    assert_eq!(held, (false, true), "(stunned, marking)");
}

/// Thistle Tea's `StealthSync` "projects stalked and empathy into DBC
/// dynamic flags": a unit marked (Hunter's Mark) is tracked and one under
/// Beast Lore shows its information (`UNIT_DYNFLAG_TRACK_UNIT` and
/// `_SPECIALINFO`, as mangos's `HandleAuraModStalked` and
/// `HandleAuraEmpathy` set them), its other flags kept, and both go with
/// their auras.
#[tokio::test]
async fn a_marked_unit_is_tracked_and_beast_lore_shows_its_information() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 5.0);
        let flags = |m: &Map| m.unit_fields(brute).unwrap().get_u32(UNIT_DYNAMIC_FLAGS);
        m.unit_fields_mut(brute)
            .unwrap()
            .set_u32(UNIT_DYNAMIC_FLAGS, 0x20);
        put(m, ga, brute, MARK, 0);
        put(m, ga, brute, BEAST_LORE, 0);
        let marked = flags(m);
        auras::remove_where(m, brute, |h| h.spell == MARK || h.spell == BEAST_LORE);
        (marked, flags(m))
    })
    .await;
    assert_eq!(out, (0x32, 0x20));
}
