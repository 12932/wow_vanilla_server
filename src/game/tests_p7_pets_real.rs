//! Round 7's pet tests on the real world, ported from Thistle Tea's
//! `test/game/core/pet/` (docs/parity/matrix.csv): a hunter's talents on
//! her pet's passives.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::fields::*;
use crate::game::map::{Map, MapHandle, auras, pets};

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

/// The tamed pets' health passive, which Endurance Training changes.
const TAMED_PET_PASSIVE: u32 = 19_581;
const ENDURANCE_TRAINING_1: u32 = 19_583;
const ENDURANCE_TRAINING_5: u32 = 19_587;

/// The pet's maximum health, and the amount of its health passive.
fn health(m: &Map, pet: Guid) -> (u32, Option<i32>) {
    let max = m
        .creature(pet)
        .map_or(0, |c| c.fields.get_u32(UNIT_FIELD_MAXHEALTH));
    let amount =
        holder(m, pet, TAMED_PET_PASSIVE).and_then(|h| h.effects.first().map(|e| e.amount));
    (max, amount)
}

/// Wait for the pet's health and its passive's amount to be `want`'s.
async fn health_becomes(
    map: &MapHandle,
    pet: Guid,
    want: impl Fn(u32, Option<i32>) -> bool + Send + Copy + 'static,
) -> (u32, Option<i32>) {
    let mut last = (0, None);
    for _ in 0..300 {
        last = on(map, move |m| health(m, pet)).await;
        if want(last.0, last.1) {
            return last;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the pet's health stayed {last:?}");
}

/// Thistle Tea's `PetSpellModifiers` with the real Endurance Training
/// (`pet_spell_modifiers_dbc_test.exs`) and its "updates existing
/// passives on rank replacement and removal without stacking",
/// "restore_passives applies inherited modifiers when rebuilding a pet"
/// and "does not resurrect a dead pet": a hunter of 50's wolf of 49 has
/// the tamed pets' health passive at nothing; her Endurance Training's
/// first rank makes it 3% and its fifth 15%, one aura and not stacked,
/// its health 15% more. Called again after a dismissal, the pet has the
/// 15% at once; and the talent gone while it lies dead, its health to come
/// is its own again, and it stays dead.
#[tokio::test]
#[ignore = "extended"]
async fn endurance_training_raises_the_pets_health_passive_by_its_rank() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P7PET1", "Endurer", Race::Dwarf, Class::Hunter).await;
    let pet = on(&map, move |m| {
        ready(m, g, 50, &[883, 2641]);
        let data = m.realm.data.clone();
        let entry = pets::beast_for(&data, 49).expect("a beast of 49");
        pets::give_pet(m, g, entry, 49).expect("a pet");
        m.players[&g].owned.pet
    })
    .await
    .expect("a pet out");
    let (base, amount) = health_becomes(&map, pet, |max, a| max > 0 && a.is_some()).await;
    assert_eq!(amount, Some(0), "the passive at nothing");
    on(&map, move |m| {
        crate::game::map::spells::learn(m, g, ENDURANCE_TRAINING_1);
    })
    .await;
    let (rank1, _) = health_becomes(&map, pet, |_, a| a == Some(3)).await;
    assert_eq!(rank1, base * 103 / 100, "3% more");
    on(&map, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.spells.retain(|s| *s != ENDURANCE_TRAINING_1);
        auras::remove_spell(m, g, ENDURANCE_TRAINING_1, None);
        crate::game::map::spells::learn(m, g, ENDURANCE_TRAINING_5);
    })
    .await;
    let (rank5, _) = health_becomes(&map, pet, |_, a| a == Some(15)).await;
    assert_eq!(rank5, base * 115 / 100, "15% more");
    let passives = on(&map, move |m| {
        auras::auras(m, pet).map_or(0, |a| {
            a.holders
                .iter()
                .filter(|h| h.spell == TAMED_PET_PASSIVE)
                .count()
        })
    })
    .await;
    assert_eq!(passives, 1, "one, not stacked");
    // Called again, it has the talent's at once.
    let called = on(&map, move |m| {
        pets::dismiss(m, pet);
        crate::game::map::spells::cast_triggered(m, g, 883, Some(g));
        let pet = m.players[&g].owned.pet?;
        Some((pet, health(m, pet)))
    })
    .await
    .expect("called back");
    assert_eq!(called.1, (rank5, Some(15)), "rebuilt with the talent's");
    let pet = called.0;
    // Dead, the talent's going takes its 15% and raises it not.
    on(&map, move |m| {
        crate::game::map::combat::kill(m, g, pet);
        m.players
            .get_mut(&g)
            .unwrap()
            .spells
            .retain(|s| *s != ENDURANCE_TRAINING_5);
        auras::remove_spell(m, g, ENDURANCE_TRAINING_5, None);
    })
    .await;
    let (dead, _) = health_becomes(&map, pet, |_, a| a == Some(0)).await;
    assert_eq!(dead, base, "its own health to come");
    let alive = on(&map, move |m| m.creature(pet).map(|c| c.alive())).await;
    assert_eq!(alive, Some(false), "not raised");
}
