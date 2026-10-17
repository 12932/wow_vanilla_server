//! Battleground resurrection waves as Thistle Tea raises them: only the
//! ghosts queued at a spirit guide (or by releasing), still Waiting to
//! Resurrect, rise.

use std::time::{Duration, Instant};

use wow_world_messages::vanilla::*;

use super::tests_pvp::{battleground, both_into, marker};
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::battleground::SPELL_WAITING_TO_RESURRECT;
use crate::game::map::death::{self, Death};

#[tokio::test]
async fn a_wave_raises_only_the_ghosts_waiting_to_resurrect() {
    let mut d = GameData::empty();
    battleground(&mut d, 489, 2, "PVPZone03", [769, 770]);
    marker(&mut d, SPELL_WAITING_TO_RESURRECT);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let grom = b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::WarsongGulch).await;
    // Both fall and release; Grom cancels Waiting to Resurrect.
    map.query(move |m| {
        for g in [alice, grom] {
            death::kill(m, g);
            death::release(m, g);
        }
        crate::game::map::auras::remove_spell(m, grom, SPELL_WAITING_TO_RESURRECT, None);
    })
    .await;
    // The wave, due now.
    map.query(|m| {
        if let Some(b) = m.battleground.as_mut() {
            b.resurrect_at = Instant::now();
        }
    })
    .await;
    let mut risen = false;
    for _ in 0..100 {
        risen = map
            .query(move |m| m.players[&alice].death == Death::Alive)
            .await
            .unwrap();
        if risen {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(risen, "Alice, waiting, rises");
    let (waiting, grom_dead) = map
        .query(move |m| {
            (
                m.players[&alice]
                    .auras
                    .has_spell(SPELL_WAITING_TO_RESURRECT),
                matches!(m.players[&grom].death, Death::Ghost { .. }),
            )
        })
        .await
        .unwrap();
    assert!(!waiting, "risen, no longer waiting");
    assert!(grom_dead, "Grom, no longer waiting, stays a ghost");
}
