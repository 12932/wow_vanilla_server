//! Round 7's player tests of pets, ported from Thistle Tea's
//! `test/game/world/entity/player/pet_stable_test.exs` and
//! `pet_experience_test.exs` (docs/parity/matrix.csv): a pet's name and
//! stance kept through the stable and a logout, and a tamed beast's
//! level.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::{UNIT_FIELD_LEVEL, UNIT_FIELD_PETEXPERIENCE, UNIT_FIELD_POWER1};

const UNIT_FIELD_POWER5: u16 = UNIT_FIELD_POWER1 + 4;
use crate::game::harness::{self, Client};
use crate::game::map::pets::{ACT_REACTION, REACT_PASSIVE, button};
use crate::game::tests_pets::{
    BOAR, CALL_PET, DISMISS_PET, STABLE_MASTER, TAME_BEAST, cast, enter, learn, place,
    stable_result, tamed,
};

/// The next pet bar: the pet and its stance.
async fn bar(c: &mut Client) -> (Guid, Option<PetReactState>) {
    c.wait_for("SMSG_PET_SPELLS", |m| match m {
        ServerOpcodeMessage::SMSG_PET_SPELLS(s) => {
            Some((s.pet, s.action_bars.as_ref().map(|b| b.react)))
        }
        _ => None,
    })
    .await
}

async fn name_of(c: &mut Client, pet: Guid) -> String {
    c.send(CMSG_PET_NAME_QUERY {
        pet_number: 0,
        guid: pet,
    })
    .await;
    c.wait_for("SMSG_PET_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_PET_NAME_QUERY_RESPONSE(r) => Some(r.name.clone()),
        _ => None,
    })
    .await
}

/// Thistle Tea's `PetStable.transfer` "retains a committed name through
/// live suspension and stable storage", and `CompanionVisibility`
/// "projects the restored pet stance on the client pet bar": a hunter's
/// pet is named once, by the name rules (a bad name refused with
/// `SMSG_PET_NAME_INVALID`, another's creature not renamed, a second name
/// refused); made passive; and stabled and taken out, then brought back
/// after a logout, it keeps its name and its stance, on the bar.
#[tokio::test]
async fn a_pets_name_and_stance_last_through_the_stable_and_a_logout() {
    let (_realm, mut h, gh, map, pet, wolf) = tamed().await;
    let master = place(&map, gh, STABLE_MASTER, 0x70_0004, 2.0).await;
    h.sees(master).await;
    map.call(move |m| {
        if let Some(p) = m.players.get_mut(&gh) {
            p.set_money(10_000);
        }
    });
    h.send(CMSG_PET_RENAME {
        pet: wolf,
        name: "Fang".into(),
    })
    .await;
    h.send(CMSG_PET_RENAME {
        pet,
        name: "x1".into(),
    })
    .await;
    h.wait_frame("SMSG_PET_NAME_INVALID", 0x178, |_| true).await;
    assert_eq!(name_of(&mut h, pet).await, "Tame Boar", "not renamed yet");
    h.send(CMSG_PET_RENAME {
        pet,
        name: "fang".into(),
    })
    .await;
    assert_eq!(name_of(&mut h, pet).await, "Fang");
    h.send(CMSG_PET_RENAME {
        pet,
        name: "Claw".into(),
    })
    .await;
    assert_eq!(name_of(&mut h, pet).await, "Fang", "named once");
    h.send(CMSG_PET_ACTION {
        pet,
        data: button(u32::from(REACT_PASSIVE), ACT_REACTION),
        target: Guid::zero(),
    })
    .await;
    h.send(CMSG_BUY_STABLE_SLOT { npc: master }).await;
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessBuySlot);
    h.send(CMSG_STABLE_PET {
        stable_master: master,
    })
    .await;
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessStable);
    let number = map
        .query(move |m| m.players[&gh].row.state.pets.stable[0].1.number)
        .await
        .unwrap();
    h.clear();
    h.send(CMSG_UNSTABLE_PET {
        stable_master: master,
        pet_number: number,
    })
    .await;
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessUnstable);
    let (back, react) = bar(&mut h).await;
    assert!(!back.is_zero());
    assert_eq!(react, Some(PetReactState::Passive), "passive still");
    assert_eq!(name_of(&mut h, back).await, "Fang", "its name kept");
    // Out of the fight the taming started, so the logout is at once.
    // Its happiness and experience as it goes, too.
    map.query(move |m| {
        m.remove_creature(wolf);
        crate::game::map::combat::leave_combat(m, gh);
        crate::game::map::combat::leave_combat(m, back);
        let f = &mut m.creature_mut(back).unwrap().fields;
        f.set_u32(UNIT_FIELD_POWER5, 700_000);
        f.set_u32(UNIT_FIELD_PETEXPERIENCE, 50);
    })
    .await;
    h.send(CMSG_LOGOUT_REQUEST {}).await;
    h.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    h.clear();
    h.login(gh).await;
    let (again, react) = bar(&mut h).await;
    assert!(!again.is_zero(), "back after the logout");
    assert_eq!(react, Some(PetReactState::Passive), "on the bar");
    assert_eq!(name_of(&mut h, again).await, "Fang");
    let kept = map
        .query(move |m| {
            m.creature(again).map(|c| {
                (
                    c.fields.get_u32(UNIT_FIELD_POWER5),
                    c.fields.get_u32(UNIT_FIELD_PETEXPERIENCE),
                )
            })
        })
        .await
        .unwrap()
        .unwrap();
    assert!(
        (690_000..=700_000).contains(&kept.0),
        "happiness {}",
        kept.0
    );
    assert_eq!(kept.1, 50, "experience");
}

/// Thistle Tea's `PossessionOwner.reconcile` "controller process loss
/// removes the aura and restores the victim", and `State.suspend_companion`
/// "clears the possession camera and mover before retaining the
/// character": a priest mind-controlling a player goes away (their
/// client gone); the victim is let go at once, the aura off, their side
/// and control their own again; and the priest, back, holds no one and
/// moves only themselves.
#[tokio::test]
async fn a_mind_controlled_player_is_let_go_when_the_priest_goes() {
    use crate::game::fields::UNIT_FIELD_FACTIONTEMPLATE;
    use crate::game::tests_pets2::{MIND_CONTROL, bring, control_update, flag_pvp, more};
    let realm = harness::realm_with(more());
    let (mut a, ga, map) = enter(&realm, "A", "Priest", Race::Human, Class::Priest).await;
    let (mut b, gb, _) = enter(&realm, "B", "Undying", Race::Undead, Class::Warrior).await;
    learn(&map, ga, &[MIND_CONTROL]);
    bring(&map, gb, ga, 8.0).await;
    a.sees(gb).await;
    flag_pvp(&map, vec![ga, gb]);
    let before = map
        .query(move |m| m.players[&gb].fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE))
        .await
        .unwrap();
    cast(&a, MIND_CONTROL, Some(gb)).await;
    assert!(!control_update(&mut b, gb).await, "the victim lets go");
    assert!(control_update(&mut a, gb).await, "the priest takes hold");
    a.send(CMSG_SET_ACTIVE_MOVER { guid: gb }).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    a.disconnect();
    assert!(control_update(&mut b, gb).await, "control back");
    let victim = map
        .query(move |m| {
            let p = &m.players[&gb];
            (
                p.auras.has_spell(MIND_CONTROL),
                p.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
                p.owned.possessed_by.is_some(),
            )
        })
        .await
        .unwrap();
    assert_eq!(victim, (false, before, false), "let go");
    // Back once gone (a connection before that takes her over where she
    // is).
    crate::game::harness::offline(&realm, ga).await;
    let mut a = Client::connect(&realm, "A");
    a.login(ga).await;
    a.sees(ga).await;
    let priest = map
        .query(move |m| m.players.get(&ga).map(|p| (p.owned.charm, p.owned.mover)))
        .await
        .unwrap();
    assert_eq!(priest, Some((None, None)), "holding no one");
}

/// Thistle Tea's taming "forwards the wild creature's level to its new
/// owner": a hunter of 10 tames a beast of 8, and has a pet of 8.
#[tokio::test]
async fn a_tamed_beast_keeps_its_level() {
    let realm = harness::realm_with(crate::game::tests_pets::data());
    let (mut h, gh, map) = enter(&realm, "H", "Hunter", Race::Dwarf, Class::Hunter).await;
    learn(&map, gh, &[TAME_BEAST, CALL_PET, DISMISS_PET]);
    h.say(".character level 10").await;
    h.system_message("Level 10.").await;
    let boar = place(&map, gh, BOAR, 0x70_0001, 4.0).await;
    map.query(move |m| {
        m.creature_mut(boar)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_LEVEL, 8);
    })
    .await;
    h.sees(boar).await;
    cast(&h, TAME_BEAST, Some(boar)).await;
    let (pet, _) = bar(&mut h).await;
    assert!(!pet.is_zero());
    let level = map
        .query(move |m| m.creature(pet).map(|c| c.fields.get_u32(UNIT_FIELD_LEVEL)))
        .await
        .unwrap();
    assert_eq!(level, Some(8));
}
