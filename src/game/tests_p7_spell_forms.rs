//! Round 7, against Thistle Tea's `passive_spells_test.exs`: what leaving
//! a form ends.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, has};
use crate::game::data::GameData;
use crate::game::map::auras::*;

/// Cat form; a buff for cat form; the same that doesn't need the form
/// (`SPELL_ATTR_EX2_NOT_NEED_SHAPESHIFT`); the same from another.
const CAT: u32 = 997_801;
const CAT_BUFF: u32 = 997_802;
const KEPT_BUFF: u32 = 997_803;
const OTHERS_BUFF: u32 = 997_804;

fn data(d: &mut GameData) {
    let mut cat = bare(CAT, 0);
    cat.duration_index = LASTS_LONG;
    effect(
        &mut cat,
        0,
        aura(AURA_MOD_SHAPESHIFT, i32::from(FORM_CAT), 0, 1),
    );
    let buff = |id: u32| {
        let mut s = bare(id, 0);
        s.duration_index = LASTS_LONG;
        s.stances = 1;
        effect(&mut s, 0, aura(AURA_MOD_STAT, 0, 5, 1));
        s
    };
    let mut kept = buff(KEPT_BUFF);
    kept.attributes[2] |= 0x8_0000;
    insert(d, [cat, buff(CAT_BUFF), kept, buff(OTHERS_BUFF)]);
}

/// Leaving cat form ends its own buffs that hang on the form, not one
/// that doesn't need it nor another's (Thistle Tea's "only self-cast
/// form-bound effects end on leaving the form").
#[tokio::test]
async fn leaving_a_form_ends_only_ones_own_buffs_that_need_it() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7SF1", "Shifter", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let kobold = kit::dummy(m, a, 60, 3.0);
            kit::apply(m, a, a, CAT);
            let shifted = form(m, a);
            kit::apply(m, a, a, CAT_BUFF);
            kit::apply(m, a, a, KEPT_BUFF);
            kit::apply(m, kobold, a, OTHERS_BUFF);
            remove_spell(m, a, CAT, None);
            (
                shifted,
                form(m, a),
                has(m, a, CAT_BUFF),
                has(m, a, KEPT_BUFF),
                has(m, a, OTHERS_BUFF),
            )
        })
        .await
        .unwrap();
    assert_eq!(out, (FORM_CAT, 0, false, true, true));
}
