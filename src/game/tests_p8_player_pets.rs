//! Round 8's player: the stable.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::MapHandle;
use crate::game::map::pets;
use crate::game::tests_pets::{STABLE_MASTER, pet_bar, place, stable_result, tamed};

/// A stall bought at a stable master beside the hunter, with money to
/// spare: the master's guid.
async fn stall(h: &mut crate::game::harness::Client, map: &MapHandle, gh: Guid) -> Guid {
    let master = place(map, gh, STABLE_MASTER, 0x70_0004, 2.0).await;
    h.sees(master).await;
    map.query(move |m| m.players.get_mut(&gh).unwrap().set_money(100_000))
        .await
        .unwrap();
    h.send(CMSG_BUY_STABLE_SLOT { npc: master }).await;
    assert_eq!(stable_result(h).await, StableResult::SuccessBuySlot);
    master
}

/// Thistle Tea's pet_stable_test "captures final live state" and
/// "retrieval owns the replacement ... and permits immediate storage": a
/// pet stabled keeps its happiness, health, experience, training points
/// and stance as they were the moment it went in; taken out, it can go
/// back in at once.
#[tokio::test]
async fn a_pet_stabled_keeps_how_it_was_and_goes_back_in_at_once() {
    let (_realm, mut h, gh, map, pet, _wolf) = tamed().await;
    let master = stall(&mut h, &map, gh).await;
    let number = map
        .query(move |m| {
            let c = m.creature_mut(pet).unwrap();
            c.fields
                .set_u32(UNIT_FIELD_POWER1 + pets::POWER_HAPPINESS, 800_000);
            c.fields.set_u32(UNIT_FIELD_HEALTH, 73);
            c.fields.set_u32(UNIT_FIELD_PETEXPERIENCE, 123);
            let s = c.pet.as_deref_mut().unwrap();
            s.training_points = 17;
            s.react = 0;
            s.number
        })
        .await
        .unwrap();
    h.send(CMSG_STABLE_PET {
        stable_master: master,
    })
    .await;
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessStable);
    assert!(pet_bar(&mut h).await.is_zero(), "the bar goes with it");
    let kept = map
        .query(move |m| {
            let pets = &m.players[&gh].row.state.pets;
            pets.stable.first().map(|(_, r)| {
                (
                    r.number,
                    r.happiness,
                    r.health,
                    r.xp,
                    r.training_points,
                    r.react,
                )
            })
        })
        .await
        .unwrap();
    assert_eq!(kept, Some((number, 800_000, 73, 123, 17, 0)));
    h.send(CMSG_UNSTABLE_PET {
        stable_master: master,
        pet_number: number,
    })
    .await;
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessUnstable);
    let back = pet_bar(&mut h).await;
    assert!(!back.is_zero(), "out");
    h.send(CMSG_STABLE_PET {
        stable_master: master,
    })
    .await;
    assert_eq!(
        stable_result(&mut h).await,
        StableResult::SuccessStable,
        "in again"
    );
    let (out, stabled) = map
        .query(move |m| {
            let p = &m.players[&gh];
            (
                p.owned.pet,
                p.row
                    .state
                    .pets
                    .stable
                    .iter()
                    .map(|(_, r)| r.number)
                    .collect::<Vec<_>>(),
            )
        })
        .await
        .unwrap();
    assert_eq!((out, stabled), (None, vec![number]));
}

/// Thistle Tea's pet_stable_test "failed retrieval leaves both current pet
/// and stable unchanged": a swap for a stabled pet that can't be brought
/// out (its creature gone from the world's data) is refused before
/// anything moves, as mangos's `HandleStableSwapPet` checks the template
/// first: the pet out stays out, the stabled one stays stabled. Before,
/// the pet out went into the stall and nothing came out.
#[tokio::test]
async fn a_swap_that_cant_bring_its_pet_out_moves_nothing() {
    let (_realm, mut h, gh, map, pet, _wolf) = tamed().await;
    let master = stall(&mut h, &map, gh).await;
    map.query(move |m| {
        let p = m.players.get_mut(&gh).unwrap();
        p.row.state.pets.stable.push((
            1,
            crate::game::store::PetRecord {
                number: 77,
                entry: 99_999,
                kind: 1,
                name: "Ghost".into(),
                level: 1,
                ..Default::default()
            },
        ));
    })
    .await
    .unwrap();
    h.clear();
    h.send(CMSG_STABLE_SWAP_PET {
        npc: master,
        pet_slot: 77,
    })
    .await;
    assert_eq!(stable_result(&mut h).await, StableResult::ErrStable);
    let (out, stabled) = map
        .query(move |m| {
            let p = &m.players[&gh];
            (
                p.owned.pet,
                p.row
                    .state
                    .pets
                    .stable
                    .iter()
                    .map(|(_, r)| r.number)
                    .collect::<Vec<_>>(),
            )
        })
        .await
        .unwrap();
    assert_eq!((out, stabled), (Some(pet), vec![77]), "both as they were");
    let alive = map
        .query(move |m| m.creature(pet).is_some_and(|c| c.alive()))
        .await
        .unwrap();
    assert!(alive);
}
