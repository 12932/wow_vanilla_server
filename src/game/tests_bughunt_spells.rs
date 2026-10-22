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

/// `CMSG_CANCEL_CAST` stops only the spell it names, and takes back only a
/// queued strike of that spell; `CMSG_CANCEL_CHANNELLING` stops only a
/// channel (mangos's `HandleCancelCastOpcode` and `HandleCancelChanneling`).
/// A late cancel of a cast already done doesn't kill the next.
#[tokio::test]
async fn a_cancel_stops_only_what_it_names() {
    use super::tests_p3_spells_kit::{at, cast_fresh, casting};
    use crate::game::map::spells::{self, checks::fail};
    const SLOW: u32 = 990_910;
    const QUICK: u32 = 990_911;
    const STRIKE: u32 = 990_912;
    let realm = fast_realm(|d| {
        let made = |id: u32, cast: u32| {
            let mut s = bare(id, 0);
            s.casting_time_index = cast;
            s.duration_index = LASTS_5S;
            effect(&mut s, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
            s
        };
        insert(d, [made(SLOW, CAST_10S), made(QUICK, INSTANT)]);
    });
    let mut c = Client::connect(&realm, "BHCC");
    let g = c.enter("Canceller", Race::Human, Class::Mage).await;
    let map = world(&realm);
    let started = map
        .query(move |m| {
            ready(m, g, 60, &[SLOW, QUICK]);
            let started = cast_fresh(m, g, SLOW, at(g));
            crate::game::map::combat::queue_next_swing(m, g, STRIKE);
            started
        })
        .await
        .unwrap();
    assert!(started, "the slow cast began");
    c.clear();
    c.send(CMSG_CANCEL_CAST { id: QUICK }).await;
    c.send(CMSG_CANCEL_CHANNELLING { id: SLOW }).await;
    c.send(CMSG_CAST_SPELL {
        spell: QUICK,
        targets: spells::wire_targets(&at(g)),
    })
    .await;
    c.wait_frame("the quick cast refused", spells::SMSG_CAST_RESULT, |b| {
        b[..4] == QUICK.to_le_bytes() && b[4] == 2 && b[5] == fail::SPELL_IN_PROGRESS
    })
    .await;
    let kept = map
        .query(move |m| (casting(m, g), m.players[&g].combat.queued))
        .await
        .unwrap();
    assert_eq!(kept, (Some(SLOW), Some(STRIKE)), "nothing named stopped");
    c.send(CMSG_CANCEL_CAST { id: SLOW }).await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        let now = map
            .query(move |m| (casting(m, g), m.players[&g].combat.queued))
            .await
            .unwrap();
        if now == (None, Some(STRIKE)) {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "{now:?}");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    c.send(CMSG_CANCEL_CAST { id: STRIKE }).await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while map
        .query(move |m| m.players[&g].combat.queued.is_some())
        .await
        .unwrap()
    {
        assert!(tokio::time::Instant::now() < deadline, "the strike stays");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}
