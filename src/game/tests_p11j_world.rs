//! Round 11j's world tests: a corpse's insignia, an outdoor PvP zone's
//! buffs, a resurrection offered at logout, a script's player across a
//! worldport, a trap disarmed, and a trainer's profession (Thistle Tea's
//! `insignia_test`, `outdoor_pvp_test`, `resurrection_test`,
//! `state_test`, `trap_disarming_test` and `spells_dbc_test`).

use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::battleground as bg;
use crate::game::tests_p3_entity::on;

/// Thistle Tea's "only one competing claim creates bones, and money can
/// be taken once": Alice's claim on a fallen Horde player's corpse from
/// out of reach (more than ten yards) is refused, the corpse kept; two
/// claims from beside it, one after the other in the map's turn, make one
/// set of bones and open its loot once; its gold is taken once.
#[tokio::test]
async fn one_claim_of_a_corpses_insignia_makes_bones_and_its_gold_is_taken_once() {
    let mut d = crate::game::tests_pvp2::valley();
    let mut s = crate::game::data::spells::SpellTemplate {
        id: 22027,
        name: "Remove Insignia".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0].effect = bg::insignia::EFFECT_SKIN_PLAYER_CORPSE;
    d.spells.spells.insert(22027, s);
    let realm = crate::game::harness::realm_with(d);
    let (mut a, alice, _b, grom, map) = crate::game::tests_pvp3::pair(&realm).await;
    let (corpse, at) = on(&map, move |m| {
        bg::died(m, grom, alice);
        crate::game::map::death::kill(m, grom);
        crate::game::map::death::release(m, grom);
        m.corpses
            .values()
            .find(|c| c.owner == grom)
            .map(|c| (c.guid, c.position))
            .expect("his corpse")
    })
    .await;
    let bones = move |m: &crate::game::map::Map| {
        m.corpses
            .values()
            .filter(|c| c.fields.get_u32(CORPSE_FIELD_FLAGS) & 0x01 != 0)
            .map(|c| c.guid)
            .collect::<Vec<_>>()
    };
    // Twelve yards off: refused.
    crate::game::tests_pvp3::put(&map, &mut a, at.x + 12.0, at.y, at.z).await;
    a.clear();
    let far = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, alice, 22027, Some(corpse));
        (m.corpses.contains_key(&corpse), bones(m).len())
    })
    .await;
    assert_eq!(far, (true, 0), "out of reach");
    crate::game::tests_pvp3::put(&map, &mut a, at.x + 1.0, at.y, at.z).await;
    let seen = a.captured().len();
    let made = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, alice, 22027, Some(corpse));
        let first = bones(m);
        // The second claim, on what is now bones.
        if let Some(b) = first.first().copied() {
            crate::game::map::spells::cast_triggered(m, alice, 22027, Some(b));
        }
        crate::game::map::spells::cast_triggered(m, alice, 22027, Some(corpse));
        (first.len(), bones(m).len(), m.corpses.contains_key(&corpse))
    })
    .await;
    assert_eq!(made, (1, 1, false), "one set of bones");
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    let windows = a.captured()[seen..]
        .iter()
        .filter(|f| u16::from_le_bytes([f[2], f[3]]) == 0x160)
        .count();
    assert_eq!(windows, 1, "its loot opened once");
    let before = on(&map, move |m| m.players[&alice].money()).await;
    a.send(CMSG_LOOT_MONEY {}).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    let once = on(&map, move |m| m.players[&alice].money()).await;
    assert!(once - before >= bg::insignia::gold(1, 50), "{before} {once}");
    a.send(CMSG_LOOT_MONEY {}).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    assert_eq!(
        on(&map, move |m| m.players[&alice].money()).await,
        once,
        "taken once"
    );
}
