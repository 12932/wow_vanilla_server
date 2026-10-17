//! Phase 3's creature AI tests, pets (Thistle Tea's `ai/bt/pet*` and
//! `companion_scripts_test.exs`): whom a pet takes on and in what order, a
//! recall, its autocast of a spell that kills, and a script's commands to
//! it. The pet is the hunter's boar of
//! `tests_pets::tamed`.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::*;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::map::pets::{self, ACT_COMMAND, COMMAND_ATTACK, COMMAND_FOLLOW, REACT_AGGRESSIVE};
use crate::game::map::{Map, ai, combat};
use crate::game::tests_pets::{WOLF, place, tamed};

/// `a` fights `b`.
fn attacks(m: &mut Map, a: Guid, b: Guid) {
    combat::start_attack(m, a, b);
    combat::engage(m, a, b);
}

fn victim(m: &Map, g: Guid) -> Option<Guid> {
    m.combat(g).filter(|c| c.attacking).map(|c| c.victim)
}

/// A few of the pet's updates, a second apart.
fn think(m: &mut Map, times: u32) {
    for _ in 0..times {
        m.now += Duration::from_secs(1);
        pets::update(m);
    }
}

/// A pet takes on those fighting it first, then its master's victim, then
/// those fighting its master, whatever their distances (mangos's
/// `AttackedBy` and `getAttackerForHelper`, Thistle Tea's
/// `TargetSelection.next`).
#[tokio::test]
async fn a_pet_takes_on_its_own_attackers_then_its_masters_victim_then_its_masters_attackers() {
    let (_realm, _h, gh, map, pet, wolf) = tamed().await;
    // The farthest fights the pet, the nearest the hunter.
    let near = place(&map, gh, WOLF, 0x70_0010, 3.0).await;
    let mid = place(&map, gh, WOLF, 0x70_0011, 8.0).await;
    let order = map
        .query(move |m| {
            attacks(m, wolf, pet);
            attacks(m, gh, mid);
            attacks(m, near, gh);
            let mut order = Vec::new();
            for _ in 0..3 {
                think(m, 1);
                let v = victim(m, pet);
                order.push(v);
                if let Some(v) = v {
                    ai::die(m, v, gh);
                }
            }
            order
        })
        .await
        .unwrap();
    assert_eq!(order, [Some(wolf), Some(mid), Some(near)]);
}

/// Called back by Follow, a pet takes no fight up on its way, nor hits
/// back at what strikes it; back at its master's side it does, and an
/// Attack ends the recall at once.
#[tokio::test]
async fn a_pet_called_back_takes_no_fight_up_until_it_is_back() {
    let (_realm, h, gh, map, pet, wolf) = tamed().await;
    let far = map
        .query(move |m| {
            pets::state_mut(m, pet).unwrap().react = REACT_AGGRESSIVE;
            let at = m.players[&gh].position();
            let far = wow_world_messages::vanilla::Vector3d {
                x: at.x + 30.0,
                ..at
            };
            crate::game::map::scripts::place(m, pet, far, 0.0);
            far
        })
        .await
        .unwrap();
    h.send(CMSG_PET_ACTION {
        pet,
        data: pets::button(u32::from(COMMAND_FOLLOW), ACT_COMMAND),
        target: Guid::zero(),
    })
    .await;
    let mut recalled = false;
    for _ in 0..100 {
        recalled = map
            .query(move |m| pets::state(m, pet).is_some_and(|s| s.returning))
            .await
            .unwrap();
        if recalled {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let out = map
        .query(move |m| {
            // Struck on the way, far from its master.
            crate::game::map::scripts::place(m, pet, far, 0.0);
            attacks(m, wolf, pet);
            crate::game::map::pets::on_attacked(m, pet, wolf);
            let struck = victim(m, pet);
            crate::game::map::scripts::place(m, pet, far, 0.0);
            pets::update(m);
            let away = victim(m, pet);
            // Back at its side, it takes the fight.
            let at = m.players[&gh].position();
            crate::game::map::scripts::place(m, pet, at, 0.0);
            think(m, 2);
            (struck, away, victim(m, pet))
        })
        .await
        .unwrap();
    assert!(recalled, "called back");
    assert_eq!(out, (None, None, Some(wolf)));
    // Called back again, then told to attack: at once.
    h.send(CMSG_PET_ACTION {
        pet,
        data: pets::button(u32::from(COMMAND_FOLLOW), ACT_COMMAND),
        target: Guid::zero(),
    })
    .await;
    h.send(CMSG_PET_ACTION {
        pet,
        data: pets::button(u32::from(COMMAND_ATTACK), ACT_COMMAND),
        target: wolf,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let out = map
        .query(move |m| (pets::state(m, pet).map(|s| s.returning), victim(m, pet)))
        .await
        .unwrap();
    assert_eq!(out, (Some(false), Some(wolf)), "an attack ends the recall");
}

/// A spell that kills (Sacrifice's instant death, here of the pet itself)
/// is for a fight: a pet doesn't autocast it while idle, and does in one.
#[tokio::test]
async fn a_pet_keeps_a_spell_that_kills_for_a_fight() {
    const SACRIFICE: u32 = 96_400;
    let mut d = crate::game::tests_pets::data();
    let mut s = SpellTemplate {
        id: SACRIFICE,
        name: "Sacrifice".into(),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura: crate::game::map::auras::AURA_DUMMY,
        target_a: 1,
        ..Default::default()
    };
    s.effects[1] = SpellEffect {
        effect: 1,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(SACRIFICE, s);
    let realm = crate::game::harness::realm_with(d);
    use crate::game::tests_pets::{
        BOAR, CALL_PET, DISMISS_PET, TAME_BEAST, cast, enter, learn, pet_bar,
    };
    let (mut h, gh, map) = enter(&realm, "H", "Hunter", Race::Dwarf, Class::Hunter).await;
    learn(&map, gh, &[TAME_BEAST, CALL_PET, DISMISS_PET]);
    let boar = place(&map, gh, BOAR, 0x70_0001, 4.0).await;
    let wolf = place(&map, gh, WOLF, 0x70_0002, 14.0).await;
    h.sees(boar).await;
    cast(&h, TAME_BEAST, Some(boar)).await;
    let pet = pet_bar(&mut h).await;
    let out = map
        .query(move |m| {
            let st = pets::state_mut(m, pet).unwrap();
            st.spells.push(SACRIFICE);
            st.autocast = vec![SACRIFICE];
            let alive = |m: &Map| m.creature(pet).is_some_and(|c| c.alive());
            think(m, 3);
            let idle = alive(m);
            attacks(m, wolf, pet);
            crate::game::map::pets::on_attacked(m, pet, wolf);
            think(m, 3);
            (idle, alive(m))
        })
        .await
        .unwrap();
    assert_eq!(out, (true, false), "not while idle; in the fight, cast");
}

/// Script command 88: a hunter's pet isn't dismissed by it (mangos's "no
/// action for hunter pets"), and a command there is none of, or a creature
/// that is no one's pet, fails no script.
#[tokio::test]
async fn a_scripts_dismiss_spares_a_hunters_pet_and_fails_nothing_it_cant_do() {
    let (_realm, _h, _gh, map, pet, wolf) = tamed().await;
    let out = map
        .query(move |m| {
            let step = |d1| ScriptStep {
                command: 88,
                datalong: [d1, 0, 0, 0],
                data_flags: SF_ABORT_ON_FAILURE,
                ..Default::default()
            };
            let dismissed = crate::game::map::scripts::execute(m, &step(3), pet, Guid::zero());
            let still = m.creature(pet).is_some();
            let unknown = crate::game::map::scripts::execute(m, &step(4), pet, Guid::zero());
            let no_pet = crate::game::map::scripts::execute(m, &step(1), wolf, Guid::zero());
            (dismissed, still, unknown, no_pet)
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true, true, true));
}
