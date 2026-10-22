//! The spells and auras bug hunt's findings, each shown on made-up data in
//! the fast suite.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{apply_template, has, ready};
use crate::game::harness::{self, Client};
use crate::game::map::auras;

/// A lasting aura that ends as its unit leaves the world (a polymorph, a
/// flag, Silithyst), a single-target one and a world buff aren't saved at a
/// logout, as mangos's `IsSaveToDbHolder`; a plain one is.
#[tokio::test]
async fn auras_mangos_doesnt_save_dont_come_back_at_login() {
    const PLAIN: u32 = 990_900;
    const LEAVES: u32 = 990_901;
    const SINGLE: u32 = 990_902;
    const ECHOES: u32 = 1386;
    let realm = fast_realm(|d| {
        let made = |id: u32| {
            let mut s = bare(id, 0);
            s.duration_index = LASTS_LONG;
            effect(&mut s, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
            s
        };
        let mut leaves = made(LEAVES);
        leaves.aura_interrupt_flags = auras::INTERRUPT_CHANGE_MAP;
        let mut single = made(SINGLE);
        single.custom_flags = crate::game::data::spells::CUSTOM_SINGLE_TARGET_AURA;
        insert(d, [made(PLAIN), leaves, single, made(ECHOES)]);
    });
    let mut c = Client::connect(&realm, "BHSA");
    let g = c.enter("Saver", Race::Human, Class::Mage).await;
    let map = world(&realm);
    let on = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            for id in [PLAIN, LEAVES, SINGLE, ECHOES] {
                let s = m.realm.data.spells.get(id).unwrap().clone();
                apply_template(m, g, g, &s);
            }
            [PLAIN, LEAVES, SINGLE, ECHOES].map(|id| has(m, g, id))
        })
        .await
        .unwrap();
    assert_eq!(on, [true; 4], "all on");
    c.disconnect();
    harness::offline(&realm, g).await;
    let mut back = Client::connect(&realm, "BHSA");
    back.login(g).await;
    harness::an_update(&map).await;
    let back_on = map
        .query(move |m| [PLAIN, LEAVES, SINGLE, ECHOES].map(|id| has(m, g, id)))
        .await
        .unwrap();
    assert_eq!(back_on, [true, false, false, false], "only the plain one back");
}
