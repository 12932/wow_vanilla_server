//! Thistle Tea's creature scripts for dungeon bosses this server imports
//! (`data/tt_scripts.rs`), on the real world: Stratholme's, Blackrock
//! Depths', Zul'Farrak's and the Wailing Caverns', each pulled in Molten
//! Core's empty arena, its events made due and what came of them seen.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use crate::game::Realm;
use crate::game::harness::Client;
use crate::game::map::{Map, MapHandle, ai, event_ai};

/// Ragnaros's lair, in a copy of Molten Core: nothing about.
const ARENA: Vector3d = Vector3d {
    x: 840.0,
    y: -812.0,
    z: -229.6,
};

/// A boss of `entry` `dx` yards east of the arena's middle, standing its
/// ground and lasting.
async fn boss(map: &MapHandle, entry: u32, dx: f32) -> Guid {
    map.query(move |m| {
        let g = summon(m, entry, off(ARENA, dx, 0.0), std::f32::consts::PI);
        tough(m, g);
        if let Some(s) = event_ai::scripted_mut(m, g) {
            s.no_combat_movement = true;
        }
        g
    })
    .await
    .unwrap()
}

async fn arena(realm: &Arc<Realm>, name: &str) -> (Client, Guid, MapHandle) {
    hero_in(realm, name, 409, ARENA, 120.0, &[]).await
}

/// The id of `entry`'s event whose first parameter is `p1` (and, if
/// given, whose kind is `kind`).
fn event_with(m: &Map, entry: u32, p1: i32) -> u32 {
    m.realm.data.scripts.ai_events[&entry]
        .iter()
        .find(|e| e.params[0] == p1)
        .map(|e| e.id)
        .unwrap_or_else(|| panic!("{entry}: an event at {p1}"))
}

/// The living creatures of `entry` that `of` summoned.
fn summoned_by(m: &Map, of: Guid, entry: u32) -> Vec<Guid> {
    living(m, entry)
        .into_iter()
        .filter(|g| event_ai::scripted(m, *g).and_then(|s| s.summoner) == Some(of))
        .collect()
}

/// Nerub'enkan raises a swarm of four, six or eight crypt scarabs, or one
/// undead scarab, about him (ten yards), each set on his attacker. A crypt
/// scarab's own EventAI (the world database's, which Thistle Tea runs too)
/// despawns it once within five yards of its victim: from beside him, a
/// swarm is gone as it reaches one, which is why it seemed none came.
#[tokio::test]
#[ignore = "extended"]
async fn nerubenkan_raises_his_scarabs() {
    const NERUBENKAN: u32 = 10437;
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = arena(&realm, "Scarabs").await;
    // Thirty yards off, so a swarm doesn't reach his attacker at once.
    let g = boss(&map, NERUBENKAN, 30.0).await;
    map.query(move |m| ai::attacked(m, g, me)).await;
    let id = map
        .query(move |m| event_with(m, NERUBENKAN, 3000))
        .await
        .unwrap();
    // His events started (at the update after his spawn and pull).
    assert!(until(&map, 2000, move |m| left(m, g, id).is_some()).await);
    let mut seen = std::collections::BTreeSet::new();
    for round in 0..8 {
        let n = map
            .query(move |m| {
                if !fighting(m, g) {
                    ai::attacked(m, g, me);
                }
                crate::game::map::spells::interrupt(m, g);
                if let Some(cr) = m.caster_mut(g) {
                    cr.gcd_until = None;
                }
                due(m, g, id);
                let crypt = summoned_by(m, g, 10577);
                let undead = summoned_by(m, g, 10876);
                let near = crypt.iter().chain(&undead).all(|s| {
                    m.position_of(*s)
                        .zip(m.position_of(g))
                        .is_some_and(|(a, b)| crate::game::map::combat::dist(a, b) <= 10.5)
                });
                assert!(near, "about him");
                (crypt.len(), undead.len())
            })
            .await
            .unwrap();
        assert!(
            matches!(n, (4 | 6 | 8, 0) | (0, 1)),
            "round {round}: {n:?} scarabs"
        );
        seen.insert(n);
        let at_me = until(&map, 1000, move |m| {
            let all: Vec<Guid> = summoned_by(m, g, 10577)
                .into_iter()
                .chain(summoned_by(m, g, 10876))
                .collect();
            !all.is_empty() && all.iter().all(|s| victim(m, *s) == me)
        })
        .await;
        assert!(at_me, "round {round}: set on his attacker");
        // A crypt scarab that reaches him goes.
        if n.0 > 0 {
            map.query(move |m| {
                let s = summoned_by(m, g, 10577)[0];
                let at = m.position_of(me).unwrap();
                if let Some(c) = m.creature_mut(s) {
                    c.position = off(at, 2.0, 0.0);
                }
            })
            .await;
            assert!(
                until(&map, 2000, move |m| summoned_by(m, g, 10577).len() < n.0).await,
                "a crypt scarab gone as it reaches its victim"
            );
        }
        map.query(move |m| {
            for e in [10577, 10876] {
                for s in summoned_by(m, g, e) {
                    crate::game::map::scripts::despawn(m, s);
                }
            }
        })
        .await;
    }
    eprintln!("nerub'enkan's swarms: {seen:?}");
}
