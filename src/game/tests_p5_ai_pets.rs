//! Round 5's pet tests, ported from Thistle Tea's `test/game/core/pet/`
//! (docs/parity/matrix.csv): the stable's slots and swaps, training and
//! untraining a hunter's pet, its happiness and loyalty, and Revive Pet.
//! The pet is the hunter's boar of `tests_pets::tamed`, on data of this
//! file's own where it needs more.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
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
    assert_eq!(
        stable_result(&mut h).await,
        StableResult::ErrStable,
        "no third"
    );
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
    map.query(move |m| crate::game::map::combat::kill(m, wolf, pet))
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
/// A family's passive, and another's stamina buff.
const FAMILY: u32 = 9701;
const FORTITUDE: u32 = 9801;
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
    add(
        FAMILY,
        "Family Passive",
        tests_pets::effect(6, 0, 1),
        0,
        211,
        1,
    );
    let mut fortitude = tests_pets::spell(FORTITUDE, "Fortitude", [stamina(3), none, none]);
    fortitude.duration_index = 4;
    d.spells.spells.insert(FORTITUDE, fortitude);
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
    map.call(move |m| {
        let s = pets::state_mut(m, pet).unwrap();
        s.training_points = 10;
        if let Some(c) = m.creature_mut(pet) {
            c.fields.set_u32(UNIT_FIELD_LEVEL, 10);
        }
    });
    teach(&h, &map, gh, pet, GORE).await;
    map.call(move |m| {
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
    assert_eq!(
        refused(&map, gh, GORE_2).await,
        Some(SpellCastResult::SpellLearned)
    );
    assert_eq!(
        refused(&map, gh, GORE).await,
        Some(SpellCastResult::SpellLearned)
    );

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
    assert_eq!(
        refused(&map, gh, SCREECH).await,
        Some(SpellCastResult::BadTargets)
    );
    assert_eq!(
        refused(&map, gh, GROWL).await,
        None,
        "a free ability of any pet's"
    );
    assert_eq!(
        refused(&map, gh, GORE_2).await,
        Some(SpellCastResult::Lowlevel)
    );
    assert_eq!(
        refused(&map, gh, NOTHING).await,
        Some(SpellCastResult::NotKnown)
    );
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
    assert_eq!(
        refused(&map, gh, HOWL).await,
        Some(SpellCastResult::TooManySkills)
    );
    assert_eq!(refused(&map, gh, GORE_2).await, None, "an upgrade");
    assert_eq!(refused(&map, gh, STAMINA_1).await, None, "a passive");

    map.query(move |m| crate::game::map::combat::kill(m, wolf, pet))
        .await;
    assert_eq!(
        refused(&map, gh, GROWL).await,
        Some(SpellCastResult::TargetsDead)
    );
    let _ = Kind::Hunter;
}

/// The next untraining price this client is quoted.
async fn quote(h: &mut Client) -> u32 {
    h.wait_for("SMSG_PET_UNLEARN_CONFIRM", |m| match m {
        ServerOpcodeMessage::SMSG_PET_UNLEARN_CONFIRM(c) => Some(c.talent_reset_cost),
        _ => None,
    })
    .await
}

/// Untraining refunds the points the pet's loyalty has brought it, and
/// takes what it was taught away with the stats it gave, but leaves its
/// family's passives and another's buff on it; it is 10 silver, then 50.
/// Without the money nothing changes, a dead pet is untrained dead, and a
/// pet with one spell or none, or another's, isn't offered it (Thistle
/// Tea's `PetUntraining`, mangos's `HandlePetUnlearnOpcode`).
#[tokio::test]
async fn untraining_refunds_the_points_and_takes_the_taught_and_their_stats() {
    let (_realm, mut h, gh, map, pet, wolf) = tamed_with(training_data()).await;
    h.sees(pet).await;
    map.call(move |m| {
        let s = pets::state_mut(m, pet).unwrap();
        s.training_points = 10;
        s.spells.push(FAMILY);
        s.family_spells = vec![FAMILY];
    });
    // A pet that knows one spell has nothing to forget.
    map.call(move |m| {
        let s = pets::state_mut(m, pet).unwrap();
        s.spells = vec![FAMILY];
        pets::untrain_offer(m, gh);
    });
    let offered = h
        .gets(Duration::from_millis(200), |m| match m {
            ServerOpcodeMessage::SMSG_PET_UNLEARN_CONFIRM(_) => Some(()),
            _ => None,
        })
        .await;
    assert!(offered.is_none(), "one spell: nothing to untrain");
    teach(&h, &map, gh, pet, GORE).await;
    teach(&h, &map, gh, pet, STAMINA_1).await;
    let fortitude = map
        .query(move |m| {
            let t = m.realm.data.spells.get(FORTITUDE).cloned().unwrap();
            crate::game::map::auras::apply_spell(m, gh, pet, &t, &[(0, 3)], Default::default())
                .is_ok()
        })
        .await
        .unwrap();
    assert!(fortitude);
    let stats = || async {
        map.query(move |m| {
            let c = m.creature(pet).expect("the pet");
            let s = c.pet.as_deref().unwrap();
            (
                c.fields.get_u32(UNIT_FIELD_STAT0 + 2),
                c.fields.get_u32(UNIT_FIELD_MAXHEALTH),
                c.fields.get_u32(UNIT_FIELD_HEALTH),
                s.training_points,
                s.spells.clone(),
                s.autocast.clone(),
                [STAMINA_1, FORTITUDE].map(|x| c.auras.has_spell(x)),
            )
        })
        .await
        .unwrap()
    };
    let (stamina, health, ..) = stats().await;
    // Loyal to the fourth rank at level 10: 30 points to come back.
    map.call(move |m| {
        if let Some(c) = m.creature_mut(pet) {
            c.fields.set_byte(UNIT_FIELD_BYTES_1, 1, 4);
            c.fields.set_u32(UNIT_FIELD_LEVEL, 10);
        }
    });
    money(&map, gh, 1_000);
    map.call(move |m| pets::untrain_offer(m, gh));
    assert_eq!(quote(&mut h).await, 1_000, "10 silver");
    // Without the money: refused, the pet as it was.
    money(&map, gh, 999);
    h.send(CMSG_PET_UNLEARN { pet }).await;
    h.wait_for("SMSG_BUY_FAILED", |m| match m {
        ServerOpcodeMessage::SMSG_BUY_FAILED(f) => Some(f.result),
        _ => None,
    })
    .await;
    assert_eq!(stats().await.3, 7, "1 and 2 spent");
    money(&map, gh, 1_000);
    h.send(CMSG_PET_UNLEARN { pet }).await;
    let mut after = stats().await;
    for _ in 0..50 {
        if after.3 == 30 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        after = stats().await;
    }
    let (s, max, _, points, spells, autocast, auras) = after;
    assert_eq!(points, 30, "10 levels at the fourth rank");
    assert_eq!(spells, [FAMILY], "the family's passive kept");
    assert!(autocast.is_empty());
    assert_eq!(
        auras,
        [false, true],
        "the taught passive goes, another's buff stays"
    );
    assert_eq!((s, max), (stamina - 5, health - 50));
    let left = map.query(move |m| m.players[&gh].money()).await.unwrap();
    assert_eq!(left, 0);
    // A second time: 50 silver. A dead pet is untrained, and stays dead.
    teach(&h, &map, gh, pet, GROWL).await;
    map.query(move |m| crate::game::map::combat::kill(m, wolf, pet))
        .await;
    map.call(move |m| pets::untrain_offer(m, gh));
    assert_eq!(quote(&mut h).await, 5_000);
    money(&map, gh, 5_000);
    h.send(CMSG_PET_UNLEARN { pet }).await;
    let mut dead = stats().await;
    for _ in 0..50 {
        if dead.4.len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        dead = stats().await;
    }
    assert_eq!(
        (dead.2, dead.3, dead.4.len()),
        (0, 30, 1),
        "untrained, still dead"
    );
}

const REVIVE_PET: u32 = 982;
const HAPPINESS: u16 = UNIT_FIELD_POWER1 + pets::POWER_HAPPINESS;

/// The pet data, with Revive Pet (half health) beside it.
fn revive_data() -> GameData {
    let mut d = tests_pets::data();
    let none = SpellEffect::default();
    let revive = tests_pets::spell(
        REVIVE_PET,
        "Revive Pet",
        [
            tests_pets::effect(pets::EFFECT_SUMMON_DEAD_PET, 49, 1),
            none,
            none,
        ],
    );
    d.spells.spells.insert(REVIVE_PET, revive);
    d
}

/// Happiness falls every 7.5 seconds and loyalty moves every 12, each on
/// its own clock; neither moves while the pet lies dead, nor does its
/// health come back; and revived, each waits its full interval again
/// rather than falling due at once (Thistle Tea's `PetHappiness.tick`,
/// `PetLoyalty.tick` "death clears the deadline ... and revival starts a
/// full interval", `Regen.initialize`).
#[tokio::test]
async fn a_pets_hunger_and_loyalty_keep_their_own_clocks_stop_in_death_and_start_afresh() {
    let (_realm, mut h, gh, map, pet, wolf) = tamed_with(revive_data()).await;
    h.sees(pet).await;
    let log = map
        .query(move |m| {
            let read = |m: &Map| {
                let c = m.creature(pet).unwrap();
                (
                    c.fields.get_u32(HAPPINESS),
                    c.pet.as_deref().unwrap().loyalty_points,
                    c.fields.get_u32(UNIT_FIELD_HEALTH),
                )
            };
            let start = m.now;
            let at = |m: &mut Map, secs: f32| {
                m.now = start + Duration::from_secs_f32(secs);
                pets::update(m);
            };
            {
                let c = m.creature_mut(pet).unwrap();
                c.fields.set_u32(HAPPINESS, 700_000);
                let s = c.pet.as_deref_mut().unwrap();
                s.happiness_at = start + hunter::HAPPINESS_TICK;
                s.loyalty_at = start + Duration::from_secs(12);
                s.loyalty_points = 1_000;
            }
            let mut log = Vec::new();
            for t in [7.4, 7.5, 11.9, 12.0] {
                at(m, t);
                log.push(read(m));
            }
            crate::game::map::combat::kill(m, wolf, pet);
            log.push(read(m));
            for t in [20.0, 30.0, 40.0, 50.0] {
                at(m, t);
            }
            log.push(read(m));
            hunter::revive(m, gh, 50);
            log.push(read(m));
            for t in [51.9, 57.4, 57.5, 61.9, 62.0] {
                at(m, t);
                log.push(read(m));
            }
            log
        })
        .await
        .unwrap();
    let max = log[0].2;
    let half = max / 2;
    assert_eq!(
        &log[..4],
        &[
            (700_000, 1_000, max),
            (691_250, 1_000, max),
            (691_250, 1_000, max),
            (691_250, 1_020, max)
        ],
        "a happy pet's loyalty gains 20"
    );
    assert_eq!(log[4], (358_250, 1_020, 0), "death costs a third");
    assert_eq!(log[5], log[4], "nothing while dead");
    assert_eq!(log[6], (358_250, 1_020, half), "revived at half");
    assert_eq!(log[7], (358_250, 1_020, half), "no health back before 2 s");
    assert_eq!(log[8].0, 358_250, "no hunger before 7.5 s");
    assert_eq!(log[9].0, 349_500);
    assert_eq!(log[10].1, 1_020, "no loyalty before 12 s");
    assert_eq!(log[11].1, 1_030, "a content pet's gains 10");
}

/// Revive Pet brings the corpse to its master's side, at half its health,
/// with its number, level, happiness and training as they were (mangos's
/// `EffectSummonDeadPet`, Thistle Tea's `PetResurrection.revive`).
#[tokio::test]
async fn revive_pet_brings_the_corpse_to_its_masters_side_as_it_was() {
    let (_realm, mut h, gh, map, pet, wolf) = tamed_with(revive_data()).await;
    h.sees(pet).await;
    learn(&map, gh, &[REVIVE_PET]);
    let before = map
        .query(move |m| {
            let s = pets::state_mut(m, pet).unwrap();
            s.training_points = 7;
            let c = m.creature_mut(pet).unwrap();
            c.fields.set_u32(HAPPINESS, 500_000);
            let at = m.players[&gh].position();
            let far = Vector3d {
                x: at.x + 15.0,
                ..at
            };
            crate::game::map::scripts::place(m, pet, far, 0.0);
            crate::game::map::combat::kill(m, wolf, pet);
            let c = m.creature(pet).unwrap();
            (
                c.fields.get_u32(UNIT_FIELD_PETNUMBER),
                c.fields.get_u32(UNIT_FIELD_LEVEL),
                c.fields.get_u32(HAPPINESS),
                c.fields.get_u32(UNIT_FIELD_MAXHEALTH),
            )
        })
        .await
        .unwrap();
    cast(&h, REVIVE_PET, None).await;
    let mut after = None;
    for _ in 0..100 {
        after = map
            .query(move |m| {
                let c = m.creature(pet)?;
                c.alive().then(|| {
                    (
                        c.fields.get_u32(UNIT_FIELD_PETNUMBER),
                        c.fields.get_u32(UNIT_FIELD_LEVEL),
                        c.fields.get_u32(HAPPINESS),
                        c.fields.get_u32(UNIT_FIELD_HEALTH),
                        c.pet.as_deref().unwrap().training_points,
                        crate::game::map::combat::dist(c.position, m.players[&gh].position()),
                    )
                })
            })
            .await
            .flatten();
        if after.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let (number, level, happiness, health, points, far) = after.expect("revived");
    assert_eq!(
        (number, level, happiness, health, points),
        (before.0, before.1, before.2, before.3 / 2, 7)
    );
    assert!(far < 3.0, "at its master's side: {far}");
}

/// A hunter's pet's mood scales the flat damage its buffs add to its
/// blows, as it does its weapon's: three quarters unhappy, a quarter more
/// happy. A warlock's imp has no mood to scale it (Thistle Tea's
/// `PetHappiness.change`, mangos's
/// `Pet::GetConditionalTotalPhysicalDamageModifier`).
#[tokio::test]
async fn a_pets_mood_scales_the_flat_damage_its_buffs_add() {
    let mut d = tests_pets::data();
    let mut might = tests_pets::effect(6, 19, 1);
    might.aura = 13;
    might.misc_value = 1;
    let mut s = tests_pets::spell(
        MIGHT,
        "Might",
        [might, SpellEffect::default(), SpellEffect::default()],
    );
    s.duration_index = 4;
    d.spells.spells.insert(MIGHT, s);
    let (realm, mut h, gh, map, pet, wolf) = tamed_with(d).await;
    h.sees(pet).await;
    let (mut w, gw, _) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&map, gw, &[tests_pets::SUMMON_IMP]);
    cast(&w, tests_pets::SUMMON_IMP, None).await;
    let imp = pet_bar(&mut w).await;
    let blows = map
        .query(move |m| {
            let t = m.realm.data.spells.get(MIGHT).cloned().unwrap();
            for (caster, g) in [(gh, pet), (gw, imp)] {
                crate::game::map::auras::apply_spell(
                    m,
                    caster,
                    g,
                    &t,
                    &[(0, 20)],
                    Default::default(),
                )
                .unwrap();
            }
            let blow = |m: &mut Map, g: Guid, happiness: Option<u32>| {
                if let Some(happiness) = happiness {
                    let now = m.creature(g).unwrap().fields.get_u32(HAPPINESS);
                    hunter::change_happiness(m, g, happiness as i32 - now as i32);
                }
                let c = m.creature_mut(g).unwrap();
                c.fields.set_f32(UNIT_FIELD_MINDAMAGE, 40.0);
                c.fields.set_f32(UNIT_FIELD_MAXDAMAGE, 40.0);
                crate::game::map::combat::weapon_damage(
                    m,
                    g,
                    wolf,
                    crate::game::map::combat::Hand::Main,
                    &mut rand::rng(),
                )
                .0
            };
            [
                blow(m, pet, Some(100_000)),
                blow(m, pet, Some(400_000)),
                blow(m, pet, Some(800_000)),
                blow(m, imp, None),
            ]
        })
        .await
        .unwrap();
    assert_eq!(blows, [55, 60, 65, 60]);
}

const MIGHT: u32 = 9901;

/// After a logout the pet comes back as loyal as it was, with its loyalty
/// points and training points, its loyalty's clock starting afresh rather
/// than owed for the time away (Thistle Tea's `PetLoyalty.initialize`).
#[tokio::test]
async fn a_pet_comes_back_from_a_logout_as_loyal_as_it_left() {
    let (realm, mut h, gh, map, pet, wolf) = tamed_with(tests_pets::data()).await;
    h.sees(pet).await;
    map.query(move |m| {
        let c = m.creature_mut(pet).unwrap();
        c.fields.set_byte(UNIT_FIELD_BYTES_1, 1, 4);
        // Shown below level 10 as none (vmangos's `GetDispTP`).
        c.fields.set_u32(UNIT_FIELD_LEVEL, 10);
        let s = c.pet.as_deref_mut().unwrap();
        s.loyalty_points = 12_345;
        s.training_points = 45;
        pets::sync_record(m, gh);
        // Out of the taming's fight, to log out at once.
        for g in [gh, pet, wolf] {
            crate::game::map::combat::leave_combat(m, g);
        }
    })
    .await;
    h.send(CMSG_LOGOUT_REQUEST {}).await;
    h.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    h.clear();
    h.login(gh).await;
    let back = pet_bar(&mut h).await;
    assert!(!back.is_zero());
    let map = realm.registry.online(gh).unwrap().map();
    let restored = map
        .query(move |m| {
            let c = m.creature(back)?;
            let s = c.pet.as_deref()?;
            Some((
                c.fields.get_byte(UNIT_FIELD_BYTES_1, 1),
                s.loyalty_points,
                s.training_points,
                c.fields.get_u32(UNIT_TRAINING_POINTS),
                s.loyalty_at > m.now + Duration::from_secs(11),
            ))
        })
        .await
        .flatten();
    assert_eq!(restored, Some((4, 12_345, 45, (-46i32) as u32, true)));
}

/// The pet's bar, by slot: each button's action and kind.
async fn bar(map: &MapHandle, pet: Guid) -> Vec<(u32, u32)> {
    map.query(move |m| {
        pets::state(m, pet)
            .unwrap()
            .bar
            .iter()
            .map(|b| (b & 0x00FF_FFFF, b >> 24))
            .collect()
    })
    .await
    .unwrap()
}

async fn set_action(h: &mut Client, pet: Guid, first: (u32, u32), second: Option<(u32, u32)>) {
    h.clear();
    h.send(CMSG_PET_SET_ACTION {
        guid: pet,
        position1: first.0,
        data1: first.1,
        extra: second.map(|(position2, data2)| CMSG_PET_SET_ACTION_extra { position2, data2 }),
    })
    .await;
    // Handled before what is asked next.
    h.send(CMSG_REQUEST_PET_INFO {}).await;
    pet_bar(h).await;
}

/// `msg`, handled.
async fn handled(
    h: &mut Client,
    msg: impl Into<wow_world_messages::vanilla::opcodes::ClientOpcodeMessage>,
) {
    h.clear();
    h.send(msg).await;
    h.send(CMSG_REQUEST_PET_INFO {}).await;
    pet_bar(h).await;
}

/// The pet's bar is edited whole or not at all: a command may change
/// places with a spell, but not come off it, nor be doubled; an edit with
/// a slot off the bar, a spell it doesn't know or one in a state it can't
/// be in changes neither half. Autocast turns on and off for every copy of
/// the spell on the bar, and not for one the pet doesn't know; nobody but
/// its master edits it (Thistle Tea's `PetControls.update`, mangos's
/// `HandlePetSetAction` and `HandlePetSpellAutocastOpcode`).
#[tokio::test]
async fn a_pets_bar_is_edited_whole_and_its_commands_kept() {
    let (realm, mut h, gh, map, pet, _wolf) = tamed_with(training_data()).await;
    h.sees(pet).await;
    map.call(move |m| pets::state_mut(m, pet).unwrap().training_points = 10);
    teach(&h, &map, gh, pet, GORE).await;
    teach(&h, &map, gh, pet, GROWL).await;
    let start = bar(&map, pet).await;
    let attack = (u32::from(pets::COMMAND_ATTACK), pets::ACT_COMMAND);
    assert_eq!(start[0], attack);
    let gore_at = start.iter().position(|b| b.0 == GORE).unwrap() as u32;
    let gore = |kind| pets::button(GORE, kind);
    // Attack and Gore change places.
    set_action(
        &mut h,
        pet,
        (gore_at, pets::button(attack.0, attack.1)),
        Some((0, gore(pets::ACT_DISABLED))),
    )
    .await;
    let swapped = bar(&map, pet).await;
    assert_eq!(swapped[gore_at as usize], attack);
    assert_eq!(swapped[0].0, GORE);
    // Attack taken off, or doubled: refused.
    for (edit, second) in [
        ((gore_at, 0), None),
        ((5, pets::button(attack.0, attack.1)), None),
        // A good half with a slot off the bar, a spell it doesn't know, or
        // Growl in no state of a spell's.
        (
            (6, gore(pets::ACT_ENABLED)),
            Some((10, gore(pets::ACT_ENABLED))),
        ),
        (
            (6, gore(pets::ACT_ENABLED)),
            Some((5, pets::button(9_999, pets::ACT_DISABLED))),
        ),
        (
            (6, gore(pets::ACT_ENABLED)),
            Some((5, pets::button(GROWL, 0xFF))),
        ),
        (
            (6, gore(pets::ACT_ENABLED)),
            Some((6, gore(pets::ACT_DISABLED))),
        ),
    ] {
        set_action(&mut h, pet, edit, second).await;
        assert_eq!(bar(&map, pet).await, swapped, "{edit:?} {second:?}");
    }
    // Autocast, on and off, for every copy on the bar.
    set_action(&mut h, pet, (6, gore(pets::ACT_DISABLED)), None).await;
    let toggle = |id: u32, on: bool| CMSG_PET_SPELL_AUTOCAST {
        guid: pet,
        id,
        autocast_enabled: on,
    };
    handled(&mut h, toggle(GORE, true)).await;
    let on = bar(&map, pet).await;
    assert_eq!(
        (on[0], on[6]),
        ((GORE, pets::ACT_ENABLED), (GORE, pets::ACT_ENABLED))
    );
    h.send(toggle(GORE, false)).await;
    handled(&mut h, toggle(9_999, true)).await;
    let off = bar(&map, pet).await;
    assert_eq!(
        (off[0], off[6]),
        ((GORE, pets::ACT_DISABLED), (GORE, pets::ACT_DISABLED))
    );
    let autocast = map
        .query(move |m| pets::state(m, pet).unwrap().autocast.clone())
        .await
        .unwrap();
    assert!(!autocast.contains(&9_999) && !autocast.contains(&GORE));
    // Another's edit.
    let (o, go, _) = enter(&realm, "O", "Other", Race::Dwarf, Class::Hunter).await;
    o.send(CMSG_PET_SET_ACTION {
        guid: pet,
        position1: 6,
        data1: 0,
        extra: None,
    })
    .await;
    o.send(toggle(GORE, true)).await;
    // Handled: their name asked after.
    o.send(CMSG_NAME_QUERY { guid: go }).await;
    sleep(200).await;
    assert_eq!(bar(&map, pet).await, off, "not theirs");
}

async fn sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}
