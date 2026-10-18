//! Round 5's pet tests, ported from Thistle Tea's `test/game/core/pet/`
//! (docs/parity/matrix.csv): the stable's slots and swaps, training and
//! untraining a hunter's pet, its happiness and loyalty, and Revive Pet.
//! The pet is the hunter's boar of `tests_pets::tamed`, on data of this
//! file's own where it needs more.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::pets::PetAbility;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::pets::testing as hunter;
use crate::game::map::pets::{self, Kind};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_pets::{
    self, BOAR, CALL_PET, DISMISS_PET, STABLE_MASTER, TAME_BEAST, WOLF, cast, enter, learn,
    pet_bar, place, stable_result,
};

/// A hunter with a tamed boar near a wolf, on `d`: their client, GUID,
/// map, the pet and the wolf.
pub(crate) async fn tamed_with(
    d: GameData,
) -> (std::sync::Arc<Realm>, Client, Guid, MapHandle, Guid, Guid) {
    let realm = harness::realm_with(d);
    let (mut h, gh, map) = enter(&realm, "H", "Hunter", Race::Dwarf, Class::Hunter).await;
    learn(&map, gh, &[TAME_BEAST, CALL_PET, DISMISS_PET]);
    let boar = place(&map, gh, BOAR, 0x70_0001, 4.0).await;
    let wolf = place(&map, gh, WOLF, 0x70_0002, 14.0).await;
    h.sees(boar).await;
    cast(&h, TAME_BEAST, Some(boar)).await;
    let pet = pet_bar(&mut h).await;
    assert!(!pet.is_zero(), "the tamed boar's bar");
    (realm, h, gh, map, pet, wolf)
}

/// The pet `g` has out, and its pet number.
async fn out(map: &MapHandle, g: Guid) -> Option<(Guid, u32)> {
    map.query(move |m| {
        let pet = m.players[&g].owned.pet?;
        Some((pet, m.creature(pet)?.fields.get_u32(UNIT_FIELD_PETNUMBER)))
    })
    .await
    .flatten()
}

/// The stalls `g` has filled: each slot and the pet number in it, by slot.
async fn stalls(map: &MapHandle, g: Guid) -> Vec<(u8, u32)> {
    map.query(move |m| {
        let mut s: Vec<(u8, u32)> = m.players[&g]
            .row
            .state
            .pets
            .stable
            .iter()
            .map(|(slot, r)| (*slot, r.number))
            .collect();
        s.sort_unstable();
        s
    })
    .await
    .unwrap()
}

fn money(map: &MapHandle, g: Guid, amount: u32) {
    map.call(move |m| {
        if let Some(p) = m.players.get_mut(&g) {
            p.set_money(amount);
        }
    });
}

/// The two stalls cost 50 silver and 5 gold, each charged once, and there
/// is no third; one too dear is refused for the money. A pet goes into the
/// first free stall, comes out only with none out (a swap otherwise, into
/// the stall it leaves), and keeps its pet number throughout, and through
/// Dismiss Pet and Call Pet (Thistle Tea's `PetStable`, mangos's
/// `HandleBuyStableSlot`, `HandleStablePet`, `HandleStableSwapPet`).
#[tokio::test]
async fn the_stable_sells_two_stalls_and_swaps_pets_into_the_stall_left() {
    let (_realm, mut h, gh, map, _pet, _wolf) = tamed_with(tests_pets::data()).await;
    let master = place(&map, gh, STABLE_MASTER, 0x70_0004, 2.0).await;
    h.sees(master).await;
    money(&map, gh, 50_400);
    h.send(CMSG_BUY_STABLE_SLOT { npc: master }).await;
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessBuySlot);
    h.send(CMSG_BUY_STABLE_SLOT { npc: master }).await;
    assert_eq!(
        stable_result(&mut h).await,
        StableResult::ErrMoney,
        "4,99 gold for a 5 gold stall"
    );
    money(&map, gh, 50_000);
    h.send(CMSG_BUY_STABLE_SLOT { npc: master }).await;
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessBuySlot);
    h.send(CMSG_BUY_STABLE_SLOT { npc: master }).await;
    assert_eq!(stable_result(&mut h).await, StableResult::ErrStable, "no third");
    let (slots, left) = map
        .query(move |m| {
            let p = &m.players[&gh];
            (p.row.state.pets.stable_slots, p.money())
        })
        .await
        .unwrap();
    assert_eq!((slots, left), (2, 0), "each charged once");

    // The boar is dismissed and called back: the same pet number.
    let (_, first) = out(&map, gh).await.unwrap();
    cast(&h, DISMISS_PET, None).await;
    assert_eq!(pet_bar(&mut h).await, Guid::zero());
    cast(&h, CALL_PET, None).await;
    assert!(!pet_bar(&mut h).await.is_zero());
    assert_eq!(out(&map, gh).await.map(|o| o.1), Some(first), "Call Pet");

    // Into the first stall; a second pet, of the same kind, is put in the
    // other by hand, and taken out.
    h.send(CMSG_STABLE_PET {
        stable_master: master,
    })
    .await;
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessStable);
    assert_eq!(stalls(&map, gh).await, [(1, first)]);
    let second = first + 100;
    map.query(move |m| {
        let pets = &mut m.players.get_mut(&gh).unwrap().row.state.pets;
        let mut r = pets.stable[0].1.clone();
        r.number = second;
        r.level = 12;
        pets.stable.push((2, r));
    })
    .await;
    h.send(CMSG_UNSTABLE_PET {
        stable_master: master,
        pet_number: second,
    })
    .await;
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessUnstable);
    assert_eq!(out(&map, gh).await.map(|o| o.1), Some(second));
    // With one out, the other doesn't come out; a swap it must be.
    h.send(CMSG_UNSTABLE_PET {
        stable_master: master,
        pet_number: first,
    })
    .await;
    assert_eq!(stable_result(&mut h).await, StableResult::ErrStable);
    h.send(CMSG_STABLE_SWAP_PET {
        npc: master,
        pet_slot: 999,
    })
    .await;
    assert_eq!(
        stable_result(&mut h).await,
        StableResult::ErrStable,
        "no such pet"
    );
    h.send(CMSG_STABLE_SWAP_PET {
        npc: master,
        pet_slot: first,
    })
    .await;
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessUnstable);
    assert_eq!(out(&map, gh).await.map(|o| o.1), Some(first));
    assert_eq!(
        stalls(&map, gh).await,
        [(1, second)],
        "into the stall it left"
    );
    let level = map
        .query(move |m| m.players[&gh].row.state.pets.stable[0].1.level)
        .await
        .unwrap();
    assert_eq!(level, 12, "the stabled pet is itself, not a copy");
    // The first free stall is the second now.
    h.send(CMSG_STABLE_PET {
        stable_master: master,
    })
    .await;
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessStable);
    assert_eq!(stalls(&map, gh).await, [(1, second), (2, first)]);
}

/// A dead pet isn't stabled (mangos's `HandleStablePet`: "can't place in
/// stable dead pet"; Thistle Tea keeps it dead in the stall).
#[tokio::test]
async fn a_dead_pet_stays_out_of_the_stable() {
    let (_realm, mut h, gh, map, pet, wolf) = tamed_with(tests_pets::data()).await;
    let master = place(&map, gh, STABLE_MASTER, 0x70_0004, 2.0).await;
    h.sees(master).await;
    money(&map, gh, 1_000);
    h.send(CMSG_BUY_STABLE_SLOT { npc: master }).await;
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessBuySlot);
    map.query(move |m| crate::game::map::ai::die(m, pet, wolf))
        .await;
    h.send(CMSG_STABLE_PET {
        stable_master: master,
    })
    .await;
    assert_eq!(stable_result(&mut h).await, StableResult::ErrStable);
    assert!(stalls(&map, gh).await.is_empty());
}

// The training data: Gore's second rank, two ranks of a stamina passive,
// a free ability, four more to fill the active families, one of another
// family's, and a teaching of a spell that isn't.
const GORE: u32 = tests_pets::GORE;
const GORE_2: u32 = 9102;
const STAMINA_1: u32 = 9201;
const STAMINA_2: u32 = 9202;
const GROWL: u32 = 9301;
const BITE: u32 = 9401;
const CLAW: u32 = 9402;
const DASH: u32 = 9403;
const HOWL: u32 = 9404;
const SCREECH: u32 = 9501;
const NOTHING: u32 = 9999;
/// The teaching of `spell`.
const fn teaching(spell: u32) -> u32 {
    if spell == GORE {
        tests_pets::TRAIN_GORE
    } else {
        spell + 100_000
    }
}

fn training_data() -> GameData {
    let mut d = tests_pets::data();
    let none = SpellEffect::default();
    let mut add = |id: u32, name: &str, effect: SpellEffect, cost: u32, skill: u32, level: u32| {
        let mut s = tests_pets::spell(id, name, [effect, none, none]);
        s.base_level = level.max(1) + u32::from(id == STAMINA_2);
        s.spell_level = level;
        if effect.effect == 6 {
            s.attributes[0] |= crate::game::data::spells::ATTR_PASSIVE;
        }
        d.spells.spells.insert(id, s);
        d.pets.abilities.entry(skill).or_default().push(PetAbility {
            spell: id,
            skill,
            cost,
            ..Default::default()
        });
        let mut e = tests_pets::effect(36, 0, 5);
        e.trigger_spell = id;
        let mut t = tests_pets::spell(teaching(id), name, [e, none, none]);
        t.spell_level = level;
        d.spells.spells.insert(teaching(id), t);
        d.pets.teaching.insert(id, teaching(id));
    };
    let hit = tests_pets::effect(2, 3, 6);
    let stamina = |points| {
        let mut e = tests_pets::effect(6, points - 1, 1);
        e.aura = 29;
        e.misc_value = 2;
        e
    };
    add(GORE_2, "Gore", hit, 4, 211, 10);
    add(STAMINA_1, "Great Stamina", stamina(5), 2, 270, 1);
    add(STAMINA_2, "Great Stamina", stamina(10), 4, 270, 1);
    add(GROWL, "Growl", hit, 0, 270, 1);
    add(BITE, "Bite", hit, 0, 270, 1);
    add(CLAW, "Claw", hit, 0, 270, 1);
    add(DASH, "Dash", hit, 0, 270, 1);
    add(HOWL, "Howl", hit, 0, 270, 1);
    add(SCREECH, "Screech", hit, 0, 999, 1);
    for (rank, first) in [(GORE_2, GORE), (STAMINA_2, STAMINA_1)] {
        d.spells.ranks.insert(
            rank,
            crate::game::data::spells::SpellRank {
                first,
                ..Default::default()
            },
        );
    }
    let mut e = tests_pets::effect(36, 0, 5);
    e.trigger_spell = NOTHING;
    d.spells.spells.insert(
        teaching(NOTHING),
        tests_pets::spell(teaching(NOTHING), "Nothing", [e, none, none]),
    );
    d
}

/// Why the hunter's pet can't be taught `spell` now, if it can't.
async fn refused(map: &MapHandle, g: Guid, spell: u32) -> Option<SpellCastResult> {
    map.query(move |m| {
        let t = m.realm.data.spells.get(teaching(spell)).cloned().unwrap();
        hunter::train_check(m, g, &t).map(|r| SpellCastResult::try_from(r).unwrap())
    })
    .await
    .unwrap()
}

/// Teach the pet `spell`, by its master's cast, and wait for it to know it.
async fn teach(h: &Client, map: &MapHandle, gh: Guid, pet: Guid, spell: u32) {
    let t = teaching(spell);
    map.query(move |m| {
        if let Some(p) = m.players.get_mut(&gh)
            && !p.spells.contains(&t)
        {
            p.spells.push(t);
        }
    })
    .await;
    cast(h, t, None).await;
    for _ in 0..100 {
        if map
            .query(move |m| pets::state(m, pet).is_some_and(|s| s.spells.contains(&spell)))
            .await
            .unwrap()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the pet never learned {spell}");
}

fn set_pet(map: &MapHandle, pet: Guid, f: impl FnOnce(&mut Map) + Send + 'static) {
    let _ = pet;
    map.call(f);
}

/// The points, spells, bar and autocast of `pet`.
async fn book(map: &MapHandle, pet: Guid) -> (i32, Vec<u32>, Vec<u32>, Vec<u32>, u32) {
    map.query(move |m| {
        let s = pets::state(m, pet).unwrap();
        let bar: Vec<u32> = s
            .bar
            .iter()
            .map(|b| b & 0x00FF_FFFF)
            .filter(|a| *a > 10)
            .collect();
        let shown = m
            .creature(pet)
            .unwrap()
            .fields
            .get_u32(UNIT_TRAINING_POINTS);
        (
            s.training_points,
            s.spells.clone(),
            bar,
            s.autocast.clone(),
            shown,
        )
    })
    .await
    .unwrap()
}

/// A higher rank costs only what it costs over the rank the pet knows,
/// takes that rank's place in its spellbook, on its bar and in its
/// autocast, and is worked out from the spellbook it came back with after
/// Call Pet; a rank it knows, or a lower one, it is told it knows. An
/// ability that costs nothing is taught even to a pet in debt, one that
/// costs points isn't (Thistle Tea's `PetTraining.learn`).
#[tokio::test]
async fn a_pet_pays_only_an_upgrades_difference_and_learns_free_abilities_in_debt() {
    let (_realm, mut h, gh, map, pet, _wolf) = tamed_with(training_data()).await;
    h.sees(pet).await;
    set_pet(&map, pet, move |m| {
        let s = pets::state_mut(m, pet).unwrap();
        s.training_points = 10;
        if let Some(c) = m.creature_mut(pet) {
            c.fields.set_u32(UNIT_FIELD_LEVEL, 10);
        }
    });
    teach(&h, &map, gh, pet, GORE).await;
    set_pet(&map, pet, move |m| {
        let s = pets::state_mut(m, pet).unwrap();
        s.autocast = vec![GORE];
        pets::sync_record(m, s_owner(m, pet));
    });
    // Away and back: the spellbook is the saved one.
    cast(&h, DISMISS_PET, None).await;
    while !pet_bar(&mut h).await.is_zero() {}
    cast(&h, CALL_PET, None).await;
    let pet = pet_bar(&mut h).await;
    h.sees(pet).await;
    map.call(move |m| {
        if let Some(c) = m.creature_mut(pet) {
            c.fields.set_u32(UNIT_FIELD_LEVEL, 10);
        }
    });
    teach(&h, &map, gh, pet, GORE_2).await;
    let (points, spells, bar, autocast, _) = book(&map, pet).await;
    assert_eq!(points, 6, "1 for Gore, then 3 more for its second rank");
    assert!(spells.contains(&GORE_2) && !spells.contains(&GORE));
    assert_eq!(
        bar.iter().filter(|b| [GORE, GORE_2].contains(b)).count(),
        1,
        "{bar:?}"
    );
    assert!(bar.contains(&GORE_2), "{bar:?}");
    assert_eq!(autocast, [GORE_2]);
    assert_eq!(refused(&map, gh, GORE_2).await, Some(SpellCastResult::SpellLearned));
    assert_eq!(refused(&map, gh, GORE).await, Some(SpellCastResult::SpellLearned));

    // In debt: Growl, free, is taught; Great Stamina, for 2, isn't.
    map.call(move |m| {
        pets::state_mut(m, pet).unwrap().training_points = -10;
    });
    teach(&h, &map, gh, pet, GROWL).await;
    let (points, _, _, _, shown) = book(&map, pet).await;
    assert_eq!((points, shown), (-10, 10), "still owing 10, shown as owed");
    assert_eq!(
        refused(&map, gh, STAMINA_1).await,
        Some(SpellCastResult::TrainingPoints)
    );
}

fn s_owner(m: &Map, pet: Guid) -> Guid {
    pets::state(m, pet).unwrap().owner
}

/// A passive takes hold the moment it is taught, and its second rank
/// replaces the first rather than stacking on it (Thistle Tea's
/// `PetTraining.learn` and `restore_passives`).
#[tokio::test]
async fn a_trained_passive_takes_hold_at_once_and_its_rank_replaces_the_last() {
    let (_realm, mut h, gh, map, pet, _wolf) = tamed_with(training_data()).await;
    h.sees(pet).await;
    map.call(move |m| pets::state_mut(m, pet).unwrap().training_points = 10);
    let stamina = || async {
        map.query(move |m| {
            let c = m.creature(pet).unwrap();
            (
                c.fields.get_u32(UNIT_FIELD_STAT0 + 2),
                c.fields.get_u32(UNIT_FIELD_MAXHEALTH),
                [STAMINA_1, STAMINA_2].map(|s| c.auras.has_spell(s)),
            )
        })
        .await
        .unwrap()
    };
    let (base, health, _) = stamina().await;
    teach(&h, &map, gh, pet, STAMINA_1).await;
    assert_eq!(
        stamina().await,
        (base + 5, health + 50, [true, false]),
        "ten health a point"
    );
    teach(&h, &map, gh, pet, STAMINA_2).await;
    assert_eq!(
        stamina().await,
        (base + 10, health + 100, [false, true]),
        "not 15"
    );
    let (points, spells, ..) = book(&map, pet).await;
    assert_eq!(points, 6, "2, then 2 more");
    assert!(spells.contains(&STAMINA_2) && !spells.contains(&STAMINA_1));
}

/// Training is refused for a pet that isn't a hunter's (a warlock's imp),
/// a dead one, an ability of another family, one above the pet's level,
/// a spell that isn't, one it hasn't the points for, and a fifth active
/// ability; a higher rank and a passive are still taught beside four
/// (Thistle Tea's `PetTraining.validate`).
#[tokio::test]
async fn training_is_refused_for_the_wrong_pet_family_level_points_and_a_fifth_ability() {
    let (realm, mut h, gh, map, pet, wolf) = tamed_with(training_data()).await;
    h.sees(pet).await;
    let (mut w, gw, _) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&map, gw, &[tests_pets::SUMMON_IMP]);
    cast(&w, tests_pets::SUMMON_IMP, None).await;
    assert!(!pet_bar(&mut w).await.is_zero(), "the imp");
    assert_eq!(refused(&map, gw, GROWL).await, Some(SpellCastResult::NoPet));

    map.call(move |m| pets::state_mut(m, pet).unwrap().training_points = 0);
    assert_eq!(refused(&map, gh, SCREECH).await, Some(SpellCastResult::BadTargets));
    assert_eq!(refused(&map, gh, GROWL).await, None, "a free ability of any pet's");
    assert_eq!(refused(&map, gh, GORE_2).await, Some(SpellCastResult::Lowlevel));
    assert_eq!(refused(&map, gh, NOTHING).await, Some(SpellCastResult::NotKnown));
    assert_eq!(
        refused(&map, gh, STAMINA_1).await,
        Some(SpellCastResult::TrainingPoints)
    );

    map.call(move |m| {
        let s = pets::state_mut(m, pet).unwrap();
        s.spells = vec![GORE, BITE, CLAW, DASH];
        s.training_points = 20;
        if let Some(c) = m.creature_mut(pet) {
            c.fields.set_u32(UNIT_FIELD_LEVEL, 10);
        }
    });
    assert_eq!(refused(&map, gh, HOWL).await, Some(SpellCastResult::TooManySkills));
    assert_eq!(refused(&map, gh, GORE_2).await, None, "an upgrade");
    assert_eq!(refused(&map, gh, STAMINA_1).await, None, "a passive");

    map.query(move |m| crate::game::map::ai::die(m, pet, wolf))
        .await;
    assert_eq!(refused(&map, gh, GROWL).await, Some(SpellCastResult::TargetsDead));
    let _ = Kind::Hunter;
}
