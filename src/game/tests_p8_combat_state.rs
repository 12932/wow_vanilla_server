//! Round 8's combat tests, entering combat (Thistle Tea's
//! `combat_state_test.exs`): a creature's or a pet's cast not for a fight
//! stops as one begins, however the fight finds it.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{ATTR_NOT_IN_COMBAT, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::map::spells::{self, Targets};
use crate::game::map::{Map, ai};
use crate::game::tests_p3_combat::{self as p3, BRUTE, START};
use crate::game::tests_p7_combat::BEAST_WATCHER;

const PEACEFUL: u32 = 96_200;
const ORDINARY: u32 = 96_201;

fn ten_seconds(id: u32, peaceful: bool) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        range_index: 1,
        equipped_item_class: -1,
        casting_time_index: 91,
        ..Default::default()
    };
    if peaceful {
        s.attributes[0] |= ATTR_NOT_IN_COMBAT;
    }
    s.effects[0] = SpellEffect {
        effect: 3,
        target_a: 1,
        ..Default::default()
    };
    s
}

fn data() -> GameData {
    let mut d = crate::game::tests_p7_combat::data();
    d.spells.cast_times.insert(91, 10_000);
    for s in [ten_seconds(PEACEFUL, true), ten_seconds(ORDINARY, false)] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn begin(m: &mut Map, g: Guid, spell: u32) {
    if let Some(c) = m.caster_mut(g) {
        c.gcd_until = None;
    }
    spells::cast(
        m,
        g,
        spell,
        Targets {
            unit: Some(g),
            ..Default::default()
        },
        None,
    );
}

fn casting(m: &Map, g: Guid) -> Option<u32> {
    m.caster(g)
        .and_then(|c| c.casting.as_ref())
        .map(|c| c.spell)
}

/// Thistle Tea's "creature engagement and pet references share
/// interruption", as mangos's `SetInCombatState` has it for every unit: a
/// creature casting a spell not for a fight stops as a player's attack
/// puts it in one, and a hunter's pet as it is set on; a fight's spell
/// goes on.
#[tokio::test]
async fn a_creatures_or_pets_cast_not_for_a_fight_stops_as_one_begins() {
    let realm = crate::game::harness::realm_with(data());
    let mut a = crate::game::harness::Client::connect(&realm, "A");
    let ga = a.enter("Hunt", Race::Dwarf, Class::Hunter).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let r = map
        .query(move |m| {
            p3::level(m, ga, 60);
            let brute = p3::spawn(m, ga, BRUTE, 60, 3.0);
            begin(m, brute, PEACEFUL);
            let began = casting(m, brute);
            ai::attacked(m, brute, ga);
            let struck = casting(m, brute);
            let other = p3::spawn(m, ga, BRUTE, 60, 4.0);
            begin(m, other, ORDINARY);
            ai::attacked(m, other, ga);
            let ordinary = casting(m, other);
            // A pet set on by a foe of its own.
            crate::game::map::pets::give_pet(m, ga, BEAST_WATCHER, 60).expect("a pet");
            let pet = crate::game::map::pets::pet_of(m, ga).expect("the pet");
            begin(m, pet, PEACEFUL);
            let pet_began = casting(m, pet);
            let foe = p3::spawn(m, ga, BRUTE, 60, 5.0);
            crate::game::map::combat::engage(m, foe, pet);
            let pet_struck = casting(m, pet);
            (began, struck, ordinary, pet_began, pet_struck)
        })
        .await
        .unwrap();
    assert_eq!(r.0, Some(PEACEFUL), "the creature's cast begun");
    assert_eq!(r.1, None, "and stopped by the fight");
    assert_eq!(r.2, Some(ORDINARY), "a fight's goes on");
    assert_eq!(r.3, Some(PEACEFUL), "the pet's begun");
    assert_eq!(r.4, None, "and stopped by its fight");
}
