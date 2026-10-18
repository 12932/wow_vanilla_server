//! Phase 3's fifth round of combat tests, a defender's avoidance (Thistle
//! Tea's `defensive_capabilities_test.exs`): auras widen a parry or a block
//! a player has learned, and give none they haven't.

use wow_world_messages::vanilla::*;

use crate::game::map::auras;
use crate::game::map::inventory::SLOT_MAIN_HAND;
use crate::game::tests_p3_combat::*;

/// Thistle Tea's "weak opponents and avoidance auras cannot grant
/// unlearned abilities": a player who hasn't learned Parry or Block, armed
/// and with a shield, parries and blocks nothing for all their auras; once
/// learned, the auras count.
#[tokio::test]
async fn avoidance_auras_give_no_parry_or_block_a_player_has_not_learned() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            equip(m, ga, SWORD, SLOT_MAIN_HAND);
            equip(m, ga, SHIELD, OFF_HAND);
            let p = m.players.get_mut(&ga).unwrap();
            p.spells.retain(|s| {
                ![
                    crate::game::map::derive::SPELL_PARRY,
                    crate::game::map::derive::SPELL_BLOCK,
                ]
                .contains(s)
            });
            aura_of(m, ga, 90_910, auras::AURA_MOD_PARRY_PERCENT, 0, 25);
            aura_of(m, ga, 90_911, auras::AURA_MOD_BLOCK_PERCENT, 0, 25);
            let d = m.players[&ga].derived;
            let unlearned = (d.parry, d.block);
            let p = m.players.get_mut(&ga).unwrap();
            p.spells.push(crate::game::map::derive::SPELL_PARRY);
            p.spells.push(crate::game::map::derive::SPELL_BLOCK);
            auras::recalc(m, ga);
            let d = m.players[&ga].derived;
            (unlearned, (d.parry, d.block))
        })
        .await
        .unwrap();
    assert_eq!(r.0, (0.0, 0.0), "nothing unlearned");
    assert!(
        r.1.0 >= 25.0 && r.1.1 >= 25.0,
        "learned, the auras count: {:?}",
        r.1
    );
}
