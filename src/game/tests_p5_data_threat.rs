//! Phase 5's taunts, Thistle Tea's `Aura.TauntTest` ported: a brute
//! fighting Alice (500 threat), Bob (50) and Carol (600), taunted by a
//! challenge of six seconds: what is borrowed and given back, what holds
//! the brute, and what it falls back on.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::harness::Client;
use crate::game::map::Map;
use crate::game::map::ai;
use crate::game::map::auras::{self, *};
use crate::game::map::spells::threat_sync;
use crate::game::tests_p3_aura2::{add_spells, aura_spell, put};
use crate::game::tests_p3_combat::{BRUTE, START, setup_with, spawn};

const CHALLENGE: u32 = 96_890;
const CHALLENGE_2: u32 = 96_891;

fn data() -> GameData {
    let mut d = crate::game::tests_p3_aura2::data();
    let challenge = |id| {
        let mut s = aura_spell(id, Some(6_000), &[(AURA_MOD_TAUNT, 0, 0)]);
        s.effects[0].target_a = 6;
        s
    };
    add_spells(&mut d, [challenge(CHALLENGE), challenge(CHALLENGE_2)]);
    d
}

/// The brute's threat on `g`, and what of it is borrowed.
fn threat(m: &Map, brute: Guid, g: Guid) -> (Option<f32>, Option<f32>) {
    (
        m.creature(brute)
            .and_then(|c| c.threat.iter().find(|(t, _)| *t == g).map(|(_, v)| *v)),
        m.spells.temp_threat.get(&(brute, g)).copied(),
    )
}

/// A brute fighting the three of them, Alice its target.
fn fight(m: &mut Map, alice: Guid, bob: Guid, carol: Guid) -> Guid {
    let brute = spawn(m, alice, BRUTE, 1, 3.0);
    let c = m.creature_mut(brute).unwrap();
    c.threat = vec![(alice, 500.0), (bob, 50.0), (carol, 600.0)];
    c.combat.victim = alice;
    c.combat.attacking = true;
    m.note_fighting(brute);
    brute
}

async fn three() -> (
    std::sync::Arc<crate::game::Realm>,
    [Client; 3],
    [Guid; 3],
    crate::game::map::MapHandle,
) {
    let (realm, a, alice, map) = setup_with(data(), Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let bob = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0, START.1, START.2).await;
    let mut c = Client::connect(&realm, "C");
    let carol = c.enter("Carol", Race::Human, Class::Warrior).await;
    c.heartbeat(START.0, START.1, START.2).await;
    (realm, [a, b, c], [alice, bob, carol], map)
}

/// Thistle Tea's loan: Bob's challenge borrows the 450 that bring him
/// level with Alice, and holds the brute; threat he earns meanwhile stays
/// when the challenge ends (75), and the brute goes for the top (Carol).
/// A refresh borrows no more. A loan stands beside a temporary threat
/// already there (Fade's −20) without taking its place; a match for good
/// (Taunt's attack-me) outlasts the challenge.
#[tokio::test]
async fn a_challenge_borrows_level_threat_and_gives_it_back() {
    let (_realm, _c, [alice, bob, carol], map) = three().await;
    let got = map
        .query(move |m| {
            let brute = fight(m, alice, bob, carol);
            let _ = put(m, bob, brute, CHALLENGE, &[0]);
            let lent = threat(m, brute, bob);
            let held = ai::overtake(m, brute, alice);
            m.now += Duration::from_secs(1);
            let _ = put(m, bob, brute, CHALLENGE, &[0]);
            let refreshed = threat(m, brute, bob);
            if let Some(c) = m.creature_mut(brute)
                && let Some((_, t)) = c.threat.iter_mut().find(|(g, _)| *g == bob)
            {
                *t += 25.0;
            }
            m.now += Duration::from_secs(6);
            auras::update(m);
            let returned = threat(m, brute, bob);
            let after = ai::overtake(m, brute, bob);
            // Fade's −20 on Bob first: the challenge borrows nothing more.
            let faded = fight(m, alice, bob, carol);
            threat_sync::set_temporary(m, faded, bob, -20.0);
            let _ = put(m, bob, faded, CHALLENGE, &[0]);
            let beside = (threat(m, faded, bob), ai::overtake(m, faded, alice));
            // Taunt's match, then a challenge run out.
            let matched = fight(m, alice, bob, carol);
            threat_sync::matched(m, matched, bob);
            let _ = put(m, bob, matched, CHALLENGE, &[0]);
            m.now += Duration::from_secs(6);
            auras::update(m);
            let kept = threat(m, matched, bob);
            (lent, held, refreshed, returned, after, beside, kept)
        })
        .await
        .unwrap();
    let (lent, held, refreshed, returned, after, beside, kept) = got;
    assert_eq!(lent, (Some(500.0), Some(450.0)));
    assert_eq!(held, bob, "the challenge holds it");
    assert_eq!(refreshed, lent, "no second loan");
    assert_eq!(returned, (Some(75.0), None), "the loan back, the 25 kept");
    assert_eq!(after, carol, "the top again");
    assert_eq!(beside, ((Some(30.0), Some(-20.0)), bob));
    assert_eq!(kept, (Some(500.0), None), "the match for good");
}

/// Thistle Tea's choice: the newest challenge holds, a refreshed one anew
/// though it went on first; of two at once, the later. The newest's caster
/// gone, the one before holds; the last gone, the threat's choice, what
/// it lent gone with it. A dead brute lends nothing.
#[tokio::test]
async fn the_newest_challenge_holds_and_an_older_one_after_it() {
    let (_realm, _c, [alice, bob, carol], map) = three().await;
    let got = map
        .query(move |m| {
            let brute = fight(m, alice, bob, carol);
            let _ = put(m, bob, brute, CHALLENGE, &[0]);
            m.now += Duration::from_secs(1);
            let _ = put(m, carol, brute, CHALLENGE_2, &[0]);
            let newest = ai::overtake(m, brute, alice);
            m.now += Duration::from_secs(1);
            let _ = put(m, bob, brute, CHALLENGE, &[0]);
            let refreshed = ai::overtake(m, brute, alice);
            // At once: the later.
            let together = fight(m, alice, bob, carol);
            let _ = put(m, bob, together, CHALLENGE, &[0]);
            let _ = put(m, carol, together, CHALLENGE_2, &[0]);
            let later = ai::overtake(m, together, alice);
            // Carol, the newest, dies: Bob's holds; Bob dies too: the
            // threat's choice, his loan gone.
            let killer = spawn(m, alice, BRUTE, 1, 3.0);
            crate::game::map::combat::deal_damage(m, killer, carol, 10_000_000);
            let older = ai::overtake(m, together, carol);
            let carol_gone = threat(m, together, carol).0.is_none();
            crate::game::map::combat::deal_damage(m, killer, bob, 10_000_000);
            let plain = ai::overtake(m, together, bob);
            let loan = threat(m, together, bob);
            // A dead brute.
            let dead = fight(m, alice, bob, carol);
            crate::game::map::combat::deal_damage(m, alice, dead, 10_000_000);
            threat_sync::taunted(m, dead, alice);
            let lent = m.spells.temp_threat.get(&(dead, alice)).copied();
            (
                newest, refreshed, later, older, carol_gone, plain, loan, lent,
            )
        })
        .await
        .unwrap();
    let (newest, refreshed, later, older, carol_gone, plain, loan, lent) = got;
    assert_eq!(newest, carol);
    assert_eq!(refreshed, bob, "refreshed, it is the newest");
    assert_eq!(later, carol, "of two at once, the later");
    assert_eq!(older, bob, "the older one holds");
    assert!(carol_gone, "the dead off the list");
    assert_eq!(plain, alice, "the threat's choice");
    assert_eq!(loan, (None, None), "nothing lent left");
    assert_eq!(lent, None, "a dead brute lends nothing");
}

/// Thistle Tea's endings: the challenge taken off, dispelled, or gone with
/// its caster's death, the loan comes back; a fight forgotten (an evade),
/// a challenge running out after takes nothing from what was earned since.
#[tokio::test]
async fn a_loan_comes_back_however_it_ends_and_never_twice() {
    let (_realm, _c, [alice, bob, carol], map) = three().await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            for how in 0..2 {
                let brute = fight(m, alice, bob, carol);
                let _ = put(m, bob, brute, CHALLENGE, &[0]);
                match how {
                    0 => auras::remove_spell(m, brute, CHALLENGE, None),
                    _ => {
                        // Dispelled.
                        auras::remove_where(m, brute, |h| h.spell == CHALLENGE);
                    }
                }
                out.push(threat(m, brute, bob));
            }
            // Evaded, then fought again: the challenge's end takes nothing.
            let brute = fight(m, alice, bob, carol);
            let _ = put(m, bob, brute, CHALLENGE, &[0]);
            ai::enter_evade(m, brute);
            if let Some(c) = m.creature_mut(brute) {
                c.threat = vec![(bob, 10.0)];
            }
            m.now += Duration::from_secs(6);
            auras::update(m);
            let evaded = threat(m, brute, bob);
            (out, evaded)
        })
        .await
        .unwrap();
    let (out, evaded) = got;
    assert_eq!(out, vec![(Some(50.0), None); 2]);
    assert_eq!(evaded, (Some(10.0), None));
}
