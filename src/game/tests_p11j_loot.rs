//! Round 11j's loot, craft and weapon tests: what wvs already did, shown
//! through the protocol (Thistle Tea's gathering, item and weapon-proc
//! tests).

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::spells::Targets;
use crate::game::tests_loot::{alone, learn};

const ORE: u32 = 2770;
const MINING_PICK: u32 = 2901;
const MINING: u32 = 2575;
const VEIN: u32 = 1731;
const NONE_WITHIN: Duration = Duration::from_millis(400);

fn skill_of(m: &Map, g: Guid, skill: u32) -> u32 {
    m.players
        .get(&g)
        .and_then(|p| crate::game::map::skills::value(p, skill))
        .unwrap_or(0)
}

/// Mine `vein` until it opens (a low skill sometimes fails), each try a
/// fresh cast; whether it opened.
async fn mine(map: &crate::game::map::MapHandle, g: Guid, vein: Guid) -> bool {
    for _ in 0..60 {
        let opened = map
            .query(move |m| {
                m.now = Instant::now();
                if let Some(p) = m.players.get_mut(&g) {
                    p.caster.gcd_until = None;
                    p.caster.casting = None;
                }
                crate::game::map::spells::cast(
                    m,
                    g,
                    MINING,
                    Targets {
                        object: Some(vein),
                        ..Default::default()
                    },
                    None,
                );
                m.players.get(&g).is_some_and(|p| p.looting == Some(vein))
            })
            .await
            .unwrap_or(false);
        if opened {
            return true;
        }
    }
    false
}

/// Thistle Tea's "typed completion grants loot and one stored skill
/// point": a vein opened teaches a point; closed with its ore still in it
/// and opened again before it respawns, it teaches none (mangos's
/// `IsInSkillupList`, wvs's `loot_state.skilled`); its ore is taken once,
/// and the skill learnt is in the character's save.
#[tokio::test]
async fn a_vein_reopened_teaches_no_second_point_and_its_ore_is_taken_once() {
    let (realm, mut a, map) = alone("Alice", Class::Warrior).await;
    let g = a.guid;
    let vein = guid::game_object(VEIN, 50);
    a.sees(vein).await;
    map.query(move |m| {
        learn(m, g, MINING, 186);
        let _ = crate::game::map::inventory::add_item(m, g, MINING_PICK, 1, false);
    })
    .await;
    assert!(mine(&map, g, vein).await, "the vein opens");
    assert_eq!(map.query(move |m| skill_of(m, g, 186)).await, Some(2));
    // Closed with the ore left in it, and opened again.
    a.send(CMSG_LOOT_RELEASE { guid: vein }).await;
    a.wait_for("the window closed", |m| match m {
        ServerOpcodeMessage::SMSG_LOOT_RELEASE_RESPONSE(r) if r.guid == vein => Some(()),
        _ => None,
    })
    .await;
    assert!(mine(&map, g, vein).await, "the vein opens again");
    assert_eq!(
        map.query(move |m| skill_of(m, g, 186)).await,
        Some(2),
        "no second point from the same vein"
    );
    a.clear();
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == ORE => Some(()),
        _ => None,
    })
    .await;
    let ore = map.query(move |m| m.players[&g].count_of(ORE)).await.unwrap();
    assert!(ore >= 1);
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    let again = a
        .gets(NONE_WITHIN, |m| match m {
            ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == ORE => Some(()),
            _ => None,
        })
        .await;
    assert!(again.is_none(), "the ore is taken once");
    assert_eq!(
        map.query(move |m| m.players[&g].count_of(ORE)).await,
        Some(ore)
    );
    // The point is saved with the character.
    a.disconnect();
    crate::game::harness::offline(&realm, g).await;
    let saved = realm.store.load(guid::counter(g)).await.expect("saved");
    let state = &saved.state;
    assert!(
        state.skills.iter().flatten().any(|s| *s == (186, 2)),
        "{:?}",
        state.skills
    );
}
