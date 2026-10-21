//! Round 12e: class spells as mangos-classic runs them: Blind's
//! diminishing returns, Seal of Righteousness's proc, Frenzied
//! Regeneration, Ruthlessness's combo point after its finisher.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh};
use crate::game::data::GameData;
use crate::game::map::auras::AURA_MOD_CONFUSE;

/// A rogue's Blind, ten seconds (family flags `0x1000000`).
const BLIND: u32 = 997_901;
const RANGE_10: u32 = 990_901;

fn rogue(d: &mut GameData) {
    d.spells.ranges.insert(RANGE_10, (0.0, 10.0));
    let mut blind = bare(BLIND, 1);
    blind.family_name = 8;
    blind.family_flags = 0x100_0000;
    blind.range_index = RANGE_10;
    blind.duration_index = 990_901;
    d.spells.durations.insert(
        990_901,
        crate::game::data::spells::SpellDuration {
            base: 10_000,
            per_level: 0,
            max: 10_000,
        },
    );
    effect(&mut blind, 0, aura(AURA_MOD_CONFUSE, 0, 0, 6));
    insert(d, [blind]);
}

/// A rogue's Blind on a kobold, four times, each after the last is taken
/// off: ten seconds, five, two and a half, then not at all, as mangos's
/// `DIMINISHING_BLIND`, which counts for creatures too. Before, Blind
/// had no group and lasted ten seconds each time.
#[tokio::test]
async fn blind_diminishes_on_a_creature_too() {
    let realm = fast_realm(rogue);
    let (_c, r, map) = human(&realm, "P12EZ", "Blinder", Class::Rogue).await;
    let lasts = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, r, 60, &[BLIND]);
            let kobold = kit::dummy(m, r, 60, 3.0);
            (0..4)
                .map(|_| {
                    cast_fresh(m, r, BLIND, at(kobold));
                    let d = kit::holder(m, kobold, BLIND).and_then(|h| h.duration);
                    crate::game::map::auras::remove_spell(m, kobold, BLIND, None);
                    d
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    assert_eq!(
        lasts,
        vec![Some(10_000), Some(5_000), Some(2_500), None],
        "a half each time, then immune"
    );
}
