//! The third code review's findings (combat, spells, auras, threat and
//! creature AI), each shown on made-up data in the fast suite.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{at, dummy, health, pass, ready};
use crate::game::data::ItemTemplate;
use crate::game::map::inventory::BANK_START;
use crate::game::map::spells;
use crate::game::store::BAG_PLAYER;

const REAGENT: u32 = 990_390;

fn reagent() -> ItemTemplate {
    ItemTemplate {
        entry: REAGENT,
        name: "Made-up Reagent".into(),
        allowable_class: -1,
        allowable_race: -1,
        stackable: 20,
        buy_count: 1,
        ..Default::default()
    }
}

/// A cast's reagents are checked again as it goes off where it takes them
/// from, the bags: put in the bank meanwhile, they fail the cast, rather
/// than it going off for nothing.
#[tokio::test]
async fn reagents_banked_during_a_cast_fail_it() {
    const SPELL: u32 = 990_301;
    let realm = fast_realm(|d| {
        d.items.insert(REAGENT, reagent());
        let mut s = bare(SPELL, 2);
        s.casting_time_index = CAST_2S;
        s.reagents[0] = (REAGENT, 1);
        effect(&mut s, 0, damage(100));
        insert(d, [s]);
    });
    let (_c, mage, map) = human(&realm, "R3RG", "Reagents", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[SPELL]);
            let k = dummy(m, mage, 60, 3.0);
            crate::game::map::inventory::add_item(m, mage, REAGENT, 1, false).unwrap();
            // A GM pays no reagents.
            m.players.get_mut(&mage).unwrap().gm = false;
            let before = health(m, k);
            let began = spells::cast(m, mage, SPELL, at(k), None);
            // Into the bank while the cast goes on.
            let p = m.players.get_mut(&mage).unwrap();
            let i = p.items.iter().position(|i| i.entry == REAGENT).unwrap();
            p.items[i].bag = BAG_PLAYER;
            p.items[i].slot = BANK_START;
            pass(m, 2_100);
            let p = &m.players[&mage];
            (began, before - health(m, k), p.count_of(REAGENT))
        })
        .await
        .unwrap();
    assert!(out.0, "the cast began with the reagent in the bags");
    assert_eq!(out.1, 0, "it doesn't go off with the reagent in the bank");
    assert_eq!(out.2, 1, "and the reagent is still there");
}
