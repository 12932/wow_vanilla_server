//! Round 6's mob tests, ported from Thistle Tea's
//! `test/game/world/entity/mob/*_test.exs` (docs/parity/matrix.csv): a
//! demon's bar, a creature's pet going with it, a passive pet's spell
//! starting its fight, a tamed beast leaving its fight, a charge's
//! arrival at a target gone.

use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness;
use crate::game::map::pets::{self, COMMAND_STAY, REACT_DEFENSIVE, REACT_PASSIVE};
use crate::game::tests_pets::{
    self, BOAR, GORE, IMP, SUMMON_IMP, WOLF, cast, enter, learn, place, tamed,
};

/// A warlock's demon comes out defensive, following, under its master's
/// control (`UNIT_FLAG_PLAYER_CONTROLLED`, 0x8), its bar sent at once
/// saying so; a second summons replaces it, as mangos's Summon spells
/// do (`SPELL_ATTR_EX_DISMISS_PET_FIRST`) (Thistle Tea's "attaches before
/// the next request and publishes the defensive pet bar").
#[tokio::test]
async fn a_demon_comes_out_defensive_and_controlled_with_its_bar() {
    let realm = harness::realm_with(tests_pets::data());
    let (mut w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&map, gw, &[SUMMON_IMP]);
    cast(&w, SUMMON_IMP, None).await;
    let (imp, react, command) = w
        .wait_for("the imp's bar", |m| match m {
            ServerOpcodeMessage::SMSG_PET_SPELLS(s) if !s.pet.is_zero() => {
                s.action_bars.as_ref().map(|b| (s.pet, b.react, b.command))
            }
            _ => None,
        })
        .await;
    assert_eq!(
        (react, command),
        (PetReactState::Defensive, PetCommandState::Follow)
    );
    let (flags, state) = map
        .query(move |m| {
            let c = m.creature(imp).unwrap();
            let s = c.pet.as_deref().unwrap();
            (
                c.fields.get_u32(UNIT_FIELD_FLAGS) & 0x8,
                (s.react, s.owner == gw),
            )
        })
        .await
        .unwrap();
    assert_eq!(flags, 0x8, "under its master's control");
    assert_eq!(state, (REACT_DEFENSIVE, true));
}

/// A creature's Summon Pet makes its demon a guardian of its own, at its
/// level and of its side; a second cast makes a second beside the first
/// (mangos-classic's `EffectSummonPet`: a `GUARDIAN_PET` for a caster no
/// player, `AddGuardian`). Its summoner leaving the world takes them
/// (Thistle Tea's "owner termination releases its supervised child";
/// mangos's `Unit::RemoveFromWorld`, `RemoveGuardians`).
#[tokio::test]
async fn a_creatures_demons_are_its_guardians_and_go_when_it_leaves() {
    let realm = harness::realm_with(tests_pets::data());
    let (_w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    let summoner = place(&map, gw, WOLF, 0x70_0100, 10.0).await;
    let imps = map
        .query(move |m| {
            for _ in 0..2 {
                crate::game::map::spells::cast_triggered(m, summoner, SUMMON_IMP, None);
            }
            m.creatures
                .iter()
                .filter(|(_, c)| {
                    c.entry == IMP
                        && c.scripted.as_deref().and_then(|s| s.summoner) == Some(summoner)
                })
                .map(|(_, c)| {
                    let level = c.fields.get_u32(UNIT_FIELD_LEVEL);
                    let faction = c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE);
                    (c.guid, level, faction)
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    assert_eq!(imps.len(), 2, "two imps: {imps:?}");
    for (_, level, faction) in &imps {
        assert_eq!((*level, *faction), (1, 14), "at its level, of its side");
    }
    map.query(move |m| {
        m.remove_creature(summoner);
    })
    .await;
    for _ in 0..200 {
        let imps = imps.clone();
        let left = map
            .query(move |m| {
                imps.iter()
                    .filter(|(g, _, _)| m.creature(*g).is_some())
                    .count()
            })
            .await
            .unwrap();
        if left == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("its summoner gone, the imps stay");
}

/// A passive pet told to stay still fights the one its master's spell
/// for it struck, staying passive and told to stay (Thistle Tea's "a
/// completed pet spell orders a passive pet to attack through its owner").
#[tokio::test]
async fn a_passive_pets_harmful_spell_starts_its_fight_and_leaves_its_orders() {
    let (_realm, h, _gh, map, pet, wolf) = tamed().await;
    map.query(move |m| {
        let s = pets::state_mut(m, pet).unwrap();
        s.react = REACT_PASSIVE;
        s.command = COMMAND_STAY;
        s.spells.push(GORE);
        for g in [pet, wolf] {
            crate::game::map::combat::leave_combat(m, g);
        }
    })
    .await;
    h.send(CMSG_PET_ACTION {
        pet,
        data: pets::button(GORE, pets::ACT_ENABLED),
        target: wolf,
    })
    .await;
    for _ in 0..200 {
        let now = map
            .query(move |m| {
                let c = m.creature(pet)?;
                let s = c.pet.as_deref()?;
                Some((
                    c.combat.attacking && c.combat.victim == wolf,
                    s.react,
                    s.command,
                ))
            })
            .await
            .flatten();
        if let Some((true, react, command)) = now {
            assert_eq!((react, command), (REACT_PASSIVE, COMMAND_STAY));
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the pet never fought the wolf it gored");
}

/// The beast tamed leaves the world, and its fight with it: no one fights
/// it, nor it them, and its master's pet is a new creature (Thistle Tea's
/// "stops the wild creature without leaving threat or world projections").
#[tokio::test]
async fn a_tamed_beast_leaves_the_world_and_its_fight() {
    let (_realm, mut h, gh, map, pet, _wolf) = tamed().await;
    let boar = guid::unit(BOAR, 0x70_0001);
    h.forgets(boar).await;
    let left = map
        .query(move |m| {
            let fighting_it =
                m.creatures.iter().any(|(_, c)| {
                    c.threat.iter().any(|(g, _)| *g == boar) || c.combat.victim == boar
                }) || m.players[&gh].combat.attackers.contains(&boar)
                    || m.players[&gh].combat.victim == boar;
            // Gone, or waiting dead for its spawn's respawn.
            let gone = m.creature(boar).is_none_or(|c| !c.alive());
            (gone, fighting_it, pet != boar)
        })
        .await
        .unwrap();
    assert_eq!(left, (true, false, true));
}

/// A charge whose target died on the way swings at nothing on arriving
/// (Thistle Tea's "target death, respawn and world changes prevent
/// arrival combat"; a target gone from the map is the same).
#[tokio::test]
async fn a_charge_at_one_who_died_on_the_way_swings_at_nothing() {
    use crate::game::tests_parity_motion as motion;
    let mut d = motion::wolf_world();
    d.creature_spawns.get_mut(&0).unwrap()[0] =
        motion::spawn(80, motion::WOLF, motion::at(20.0, 0.0));
    let (_realm, mut a, ga, map) = motion::enter(d).await;
    let g = motion::wolf();
    a.sees(g).await;
    let after = map
        .query(move |m| {
            crate::game::map::charge::start(m, ga, g, true);
            let charging = m.players[&ga].charge.is_some();
            crate::game::map::combat::kill(m, ga, g);
            m.now += Duration::from_secs(5);
            crate::game::map::charge::update(m);
            let p = &m.players[&ga];
            (charging, p.charge.is_some(), p.combat.attacking)
        })
        .await
        .unwrap();
    assert_eq!(after, (true, false, false), "charged, arrived, no swing");
}
