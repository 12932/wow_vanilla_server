//! Round 8's combat tests, damage shared (Thistle Tea's
//! `damage_sharing_test.exs`): a taker on a flight takes her share
//! absorbed whole.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, AURA_SPLIT_DAMAGE_PCT};
use crate::game::tests_p3_combat::{self as p3, BRUTE};

const SACRIFICE: u32 = 97_200;

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

/// Thistle Tea's "taxi flight absorbs transfers without affecting health",
/// as mangos keeps one on a flight from harm: Alice's thirty percent of a
/// hundred struck at the brute she shares with is logged to her as thirty
/// absorbed, her health as it was; the brute takes seventy.
#[tokio::test]
async fn a_taker_on_a_flight_takes_her_share_absorbed_whole() {
    let (_realm, mut a, ga, map) = p3::setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            p3::level(m, ga, 60);
            let attacker = p3::spawn(m, ga, BRUTE, 20, 3.0);
            let target = p3::spawn(m, ga, BRUTE, 20, 4.0);
            let mut s = SpellTemplate {
                id: SACRIFICE,
                name: "Sacrifice".into(),
                equipped_item_class: -1,
                ..Default::default()
            };
            s.effects[0] = SpellEffect {
                effect: 6,
                aura: AURA_SPLIT_DAMAGE_PCT,
                misc_value: 127,
                base_points: 29,
                base_dice: 1,
                target_a: 1,
                ..Default::default()
            };
            let _ = auras::apply_spell(m, ga, target, &s, &[(0, 30)], auras::Context::default());
            let pos = m.players[&ga].position();
            let far = Vector3d {
                x: pos.x + 1000.0,
                ..pos
            };
            m.players.get_mut(&ga).unwrap().flight = Some(crate::game::map::npc::Flight {
                walk: crate::game::map::creature::Walk::new(vec![pos, far], 1.0, m.now, 0, true),
                destination: 0,
                mount: 0,
            });
            let before = health(m, ga);
            let (left, absorbed) = auras::absorb_and_share(m, attacker, target, 100, 1);
            (left, absorbed, before, health(m, ga))
        })
        .await
        .unwrap();
    let (left, absorbed, before, after) = r;
    assert_eq!((left, absorbed), (70, 30), "the brute's seventy");
    assert_eq!(after, before, "her health as it was");
    let b = a
        .wait_frame(
            "her share's log",
            crate::game::map::spells::outcome::SMSG_SPELLNONMELEEDAMAGELOG,
            |b| {
                let mut i = 0;
                for _ in 0..2 {
                    i += 1 + b[i].count_ones() as usize;
                }
                b[i..i + 4] == SACRIFICE.to_le_bytes()
            },
        )
        .await;
    let mut i = 0;
    for _ in 0..2 {
        i += 1 + b[i].count_ones() as usize;
    }
    let u32_at = |at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
    // The spell, the damage, the school, then what was absorbed.
    assert_eq!((u32_at(i + 4), u32_at(i + 9)), (0, 30), "(dealt, absorbed)");
}
