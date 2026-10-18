//! Phase 3, round 5, against Thistle Tea's `hunter_test.exs`: Tame Beast's
//! refusals, told the client as the tame failure; Feed Pet's and Revive
//! Pet's, by the pet out.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_pets::{BOAR, TAME_BEAST, WOLF, cast, data, enter, learn, pet_bar, place};
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::pets::{EFFECT_FEED_PET, EFFECT_SUMMON_DEAD_PET};
use crate::game::map::spells::{Targets, checks, fail, mods::Mods};
use crate::game::map::{Map, pets};

const FEED_PET: u32 = 990_950;
const REVIVE_PET: u32 = 990_951;

fn hunter_data() -> crate::game::data::GameData {
    let mut d = data();
    let one = |id: u32, effect: u32| {
        let mut s = SpellTemplate {
            id,
            name: format!("Made-up {id}"),
            range_index: 1,
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect,
            base_points: 0,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        };
        s
    };
    for s in [
        one(FEED_PET, EFFECT_FEED_PET),
        one(REVIVE_PET, EFFECT_SUMMON_DEAD_PET),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

async fn tame_failure(c: &mut Client) -> Option<PetTameFailureReason> {
    c.gets(Duration::from_millis(500), |m| match m {
        ServerOpcodeMessage::SMSG_PET_TAME_FAILURE(f) => Some(f.reason),
        _ => None,
    })
    .await
}

fn why(m: &mut Map, g: Guid, spell: u32) -> Option<u8> {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    checks::check(m, g, &s, &Targets::default(), None, false, &Mods::default())
}

/// Tame Beast wants a tameable beast no higher than the hunter, and no pet
/// out; a beast of the hunter's own level is tamed (Thistle Tea's
/// `validate_tame/3`).
#[tokio::test]
async fn tame_beast_wants_a_tameable_beast_no_higher_and_no_pet_out() {
    let realm = crate::game::harness::realm_with(hunter_data());
    let (mut h, gh, map) = enter(&realm, "P5TH", "Tamer", Race::Dwarf, Class::Hunter).await;
    learn(&map, gh, &[TAME_BEAST]);
    let wolf = place(&map, gh, WOLF, 0x71_0001, 4.0).await;
    let high = place(&map, gh, BOAR, 0x71_0002, 4.0).await;
    let boar = place(&map, gh, BOAR, 0x71_0003, 4.0).await;
    let other = place(&map, gh, BOAR, 0x71_0004, 4.0).await;
    map.query(move |m| {
        if let Some(f) = m.unit_fields_mut(high) {
            f.set_u32(UNIT_FIELD_LEVEL, 2);
        }
    })
    .await
    .unwrap();
    for g in [wolf, high, boar, other] {
        h.sees(g).await;
    }
    cast(&h, TAME_BEAST, Some(wolf)).await;
    let not_tameable = tame_failure(&mut h).await;
    cast(&h, TAME_BEAST, Some(high)).await;
    let too_high = tame_failure(&mut h).await;
    cast(&h, TAME_BEAST, Some(boar)).await;
    let pet = pet_bar(&mut h).await;
    // With a pet out.
    let ready = map
        .query(move |m| {
            if let Some(c) = m.caster_mut(gh) {
                c.casting = None;
                c.gcd_until = None;
                c.cooldowns.clear();
            }
        })
        .await;
    assert!(ready.is_some());
    cast(&h, TAME_BEAST, Some(other)).await;
    let another = tame_failure(&mut h).await;
    assert_eq!(not_tameable, Some(PetTameFailureReason::NotTameable));
    assert_eq!(too_high, Some(PetTameFailureReason::TooHighLevel));
    assert!(!pet.is_zero(), "its own level tamed");
    assert_eq!(another, Some(PetTameFailureReason::AnotherSummonActive));
}

/// Feed Pet is refused with no pet, a dead one or one fighting; Revive Pet
/// with a living one (Thistle Tea's `validate_feed/2` and
/// `validate_companion/2`).
#[tokio::test]
async fn feed_pet_and_revive_pet_go_by_the_pet_out() {
    let realm = crate::game::harness::realm_with(hunter_data());
    let (mut h, gh, map) = enter(&realm, "P5TI", "Feeder", Race::Dwarf, Class::Hunter).await;
    learn(&map, gh, &[TAME_BEAST, FEED_PET, REVIVE_PET]);
    let none = map.query(move |m| why(m, gh, FEED_PET)).await.unwrap();
    let boar = place(&map, gh, BOAR, 0x72_0001, 4.0).await;
    h.sees(boar).await;
    cast(&h, TAME_BEAST, Some(boar)).await;
    let pet = pet_bar(&mut h).await;
    assert!(!pet.is_zero());
    let out = map
        .query(move |m| {
            let living = why(m, gh, REVIVE_PET);
            if let Some(f) = m.unit_fields_mut(pet) {
                let flags = f.get_u32(UNIT_FIELD_FLAGS);
                f.set_u32(
                    UNIT_FIELD_FLAGS,
                    flags | crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                );
            }
            let fighting = why(m, gh, FEED_PET);
            crate::game::map::combat::deal_damage(m, gh, pet, 1_000_000);
            let dead = why(m, gh, FEED_PET);
            let revive_dead = why(m, gh, REVIVE_PET);
            let out = pets::pet_of(m, gh).is_some();
            (living, fighting, dead, revive_dead, out)
        })
        .await
        .unwrap();
    let (living, fighting, dead, revive_dead, out) = out;
    assert_eq!(none, Some(SpellCastResult::NoPet.as_int()), "no pet");
    assert_eq!(living, Some(fail::TARGET_NOT_DEAD), "revive a living pet");
    assert_eq!(fighting, Some(SpellCastResult::AffectingCombat.as_int()));
    if out {
        assert_eq!(dead, Some(SpellCastResult::TargetsDead.as_int()));
    } else {
        assert_eq!(dead, Some(SpellCastResult::NoPet.as_int()), "gone at death");
    }
    assert_eq!(revive_dead, None, "a dead pet revived");
}
