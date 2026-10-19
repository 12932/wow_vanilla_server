//! Round 8's pets (Thistle Tea's `wild_summon_test.exs`): the engineers'
//! Target Dummies, vmangos's `npc_target_dummy`.

use std::time::Duration;

use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::map::{ai, combat};
use crate::game::tests_p3_combat::{self as p3, BRUTE};
use crate::game::tests_p6_wspell::put;
use crate::game::tests_p7_combat::{creature_at, in_combat, off};

/// The Target Dummy, the spell that sets it up, its passive and its spawn.
const DUMMY: u32 = 2673;
const SET_UP: u32 = 4071;
const PASSIVE: u32 = 4044;
const SPAWN: u32 = 4507;

fn data() -> GameData {
    let mut d = crate::game::tests_p8_pets_owner::data();
    d.creature_templates.insert(DUMMY, p3::template(DUMMY, 9));
    let spell = |id: u32, effect: SpellEffect| {
        let mut s = SpellTemplate {
            id,
            name: format!("Spell {id}"),
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = effect;
        s
    };
    let set_up = spell(
        SET_UP,
        SpellEffect {
            effect: crate::game::map::pets::EFFECT_SUMMON_GUARDIAN,
            misc_value: DUMMY as i32,
            target_a: 1,
            ..Default::default()
        },
    );
    let aura = |id: u32| {
        spell(
            id,
            SpellEffect {
                effect: 6,
                aura: crate::game::map::auras::AURA_DUMMY,
                target_a: 1,
                ..Default::default()
            },
        )
    };
    for s in [set_up, aura(PASSIVE), aura(SPAWN)] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

/// Thistle Tea's "passive creatures never chase or melee and schedule
/// their death deadline" and "decoys die during combat and clear combat
/// through the shared transition": a Target Dummy comes with its passive
/// and its spawn's spell on it, stands where it came as its engineer walks
/// off, answers no blow, and falls fifteen seconds on though struck at,
/// its foe turning to the next on its list.
#[tokio::test]
async fn a_target_dummy_stands_answers_no_blow_and_falls_at_fifteen_seconds_in_a_fight() {
    let realm = crate::game::harness::realm_with(data());
    let mut h = crate::game::harness::Client::connect(&realm, "H");
    let gh = h
        .enter(
            "Gnoma",
            wow_world_messages::vanilla::Race::Gnome,
            wow_world_messages::vanilla::Class::Warrior,
        )
        .await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let r = map
        .query(move |m| {
            p3::level(m, gh, 60);
            put(m, gh, off(0.0, 0.0, 0.0));
            crate::game::map::spells::cast_triggered(m, gh, SET_UP, Some(gh));
            let dummy = m.players[&gh].owned.guardians[0];
            let auras = [PASSIVE, SPAWN]
                .map(|s| crate::game::map::auras::auras(m, dummy).is_some_and(|a| a.has_spell(s)));
            let at = m.position_of(dummy).unwrap();
            // She walks off; a brute takes it on and strikes it.
            put(m, gh, off(10.0, 0.0, 0.0));
            let b = creature_at(m, gh, BRUTE, at);
            ai::attacked(m, b, dummy);
            p3::forced_swing(m, b, dummy, combat::Outcome::Miss);
            let mut answered = false;
            for _ in 0..28 {
                m.now += Duration::from_millis(500);
                crate::game::map::pets::update(m);
                answered |= m.combat(dummy).is_some_and(|c| c.attacking);
            }
            let fourteen = (
                m.unit_view(dummy).is_some_and(|u| u.alive),
                in_combat(m, dummy),
                m.position_of(dummy).unwrap() == at,
            );
            for _ in 0..3 {
                m.now += Duration::from_millis(500);
                crate::game::map::pets::update(m);
            }
            let fifteen = (
                m.unit_view(dummy).is_some_and(|u| u.alive),
                m.creature(b).unwrap().combat.victim == gh,
            );
            (auras, answered, fourteen, fifteen)
        })
        .await
        .unwrap();
    assert_eq!(r.0, [true; 2], "its passive and its spawn's");
    assert!(!r.1, "it answers no blow");
    assert_eq!(
        r.2,
        (true, true, true),
        "fourteen seconds: up, fought, where it came"
    );
    assert_eq!(r.3, (false, true), "fifteen: fallen, its foe on her");
}
