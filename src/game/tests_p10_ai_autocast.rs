//! Round 10's pet autocast tests, ported from Thistle Tea's
//! `test/game/core/ai/bt/pet/targets_test.exs` and
//! `bt/pet/autocast_test.exs` (docs/parity/matrix.csv): a pet's Devour
//! Magic goes on an enemy's buff, else a friend's affliction, and on no
//! one with nothing to take; told at a friend, it takes their affliction;
//! a held effect is judged by its index.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::map::pets;
use crate::game::map::{Map, auras, combat};
use crate::game::tests_p5_ai_pets::tamed_with;
use crate::game::tests_pets::{effect, spell};

/// A dispel of magic at any unit, of no cost, with a cooldown.
const DEVOUR: u32 = 95_390;
/// A buff of magic, on oneself; a curse of magic, on an enemy.
const BUFF: u32 = 95_391;
const CURSE: u32 = 95_392;
/// A self buff of two effects, its cooldown a second.
const PAIR: u32 = 95_393;
/// A self buff after Devour Magic on the pet's bar.
const AFTER: u32 = 95_394;

fn world() -> GameData {
    let mut d = crate::game::tests_pets::data();
    let none = SpellEffect::default();
    let mut devour = spell(
        DEVOUR,
        "Devour Magic",
        [
            SpellEffect {
                misc_value: 1,
                ..effect(38, 0, 25)
            },
            none,
            none,
        ],
    );
    devour.recovery_time = 8_000;
    let aura = |id, name: &str, target| {
        let mut s = spell(
            id,
            name,
            [
                SpellEffect {
                    aura: 22,
                    misc_value: 1,
                    ..effect(6, 5, target)
                },
                none,
                none,
            ],
        );
        s.dispel = 1;
        s.duration_index = 4;
        s
    };
    let mut pair = spell(
        PAIR,
        "Second",
        [
            none,
            SpellEffect {
                aura: 22,
                misc_value: 2,
                ..effect(6, 5, 1)
            },
            none,
        ],
    );
    pair.recovery_time = 1_000;
    pair.duration_index = 4;
    for s in [
        devour,
        aura(BUFF, "Buff", 1),
        aura(CURSE, "Curse", 6),
        pair,
        aura(AFTER, "After", 1),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

/// The pet thinks once, two seconds on, free to cast.
fn think(m: &mut Map, pet: Guid) {
    m.now += Duration::from_secs(2);
    if let Some(c) = m.caster_mut(pet) {
        c.casting = None;
        c.gcd_until = None;
    }
    let now = m.now;
    pets::state_mut(m, pet).unwrap().next_think = now;
    pets::update(m);
}

/// Whether the pet cast `spell` at its last thought (its cooldown runs).
fn cast_now(m: &Map, pet: Guid, spell: u32) -> bool {
    m.caster(pet)
        .and_then(|c| c.cooldowns.get(&spell))
        .is_some_and(|t| *t > m.now)
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

/// `pet` knows and autocasts `spells`, passive, its cooldowns cleared.
fn autocasts(m: &mut Map, pet: Guid, spells: &[u32]) {
    let s = pets::state_mut(m, pet).unwrap();
    s.spells.extend_from_slice(spells);
    s.autocast = spells.to_vec();
    s.react = pets::REACT_PASSIVE;
    if let Some(c) = m.caster_mut(pet) {
        c.cooldowns.clear();
    }
}

/// Thistle Tea's `try_cast/4` "Devour Magic prefers an enemy buff and falls
/// back to an allied debuff" and "Devour Magic observes an attacker even
/// without a pet or owner victim": a pet's Devour Magic takes its master's
/// victim's buff before its master's curse; the victim's buffs gone (a
/// curse on it isn't the pet's to take), its master's curse; with nothing
/// to take it casts nothing, and its next spell goes; and it takes the
/// buff of one fighting its master who fights no one.
#[tokio::test]
async fn a_pets_devour_magic_takes_an_enemys_buff_else_its_masters_curse_else_nothing() {
    let (_realm, _h, gh, map, pet, wolf) = tamed_with(world()).await;
    let out = map
        .query(move |m| {
            autocasts(m, pet, &[DEVOUR]);
            crate::game::map::spells::cast_triggered(m, wolf, BUFF, Some(wolf));
            crate::game::map::spells::cast_triggered(m, wolf, CURSE, Some(gh));
            combat::start_attack(m, gh, wolf);
            think(m, pet);
            let enemy = (has(m, wolf, BUFF), has(m, gh, CURSE));
            // The wolf cursed, its buff gone: its master's curse.
            crate::game::map::spells::cast_triggered(m, gh, CURSE, Some(wolf));
            autocasts(m, pet, &[]);
            autocasts(m, pet, &[DEVOUR]);
            think(m, pet);
            let friend = (has(m, wolf, CURSE), has(m, gh, CURSE));
            // Nothing for it to take: no cast, and the next spell goes.
            autocasts(m, pet, &[]);
            autocasts(m, pet, &[DEVOUR, AFTER]);
            think(m, pet);
            let nothing = (cast_now(m, pet, DEVOUR), has(m, pet, AFTER));
            // Its master fights no one; the wolf fights her, buffed.
            combat::stop_attack(m, gh);
            crate::game::map::spells::cast_triggered(m, wolf, BUFF, Some(wolf));
            combat::start_attack(m, wolf, gh);
            combat::engage(m, wolf, gh);
            autocasts(m, pet, &[]);
            autocasts(m, pet, &[DEVOUR]);
            think(m, pet);
            let attacker = has(m, wolf, BUFF);
            (enemy, friend, nothing, attacker)
        })
        .await
        .unwrap();
    let (enemy, friend, nothing, attacker) = out;
    assert_eq!(enemy, (false, true), "the victim's buff first");
    assert_eq!(friend, (true, false), "then its master's curse");
    assert_eq!(nothing, (false, true), "nothing to take: the next spell");
    assert!(!attacker, "the buff of one fighting its master");
}

/// Thistle Tea's `try_cast/4` "manual dispels use the same observed aura
/// polarity": a pet told to cast its dispel at its master takes her curse.
#[tokio::test]
async fn a_pets_dispel_told_at_its_master_takes_her_curse() {
    let (_realm, _h, gh, map, pet, wolf) = tamed_with(world()).await;
    let out = map
        .query(move |m| {
            autocasts(m, pet, &[]);
            pets::state_mut(m, pet).unwrap().spells.push(DEVOUR);
            crate::game::map::spells::cast_triggered(m, wolf, CURSE, Some(gh));
            let cursed = has(m, gh, CURSE);
            pets::handle(
                m,
                gh,
                ClientOpcodeMessage::CMSG_PET_ACTION(CMSG_PET_ACTION {
                    pet,
                    data: DEVOUR,
                    target: gh,
                }),
            );
            (cursed, has(m, gh, CURSE))
        })
        .await
        .unwrap();
    assert_eq!(out, (true, false));
}

/// Thistle Tea's `allowed?/4` "does not refresh a held effect and allows a
/// partially removed effect" (mangos's `CanAutoCast` by effect index): a
/// pet's buff (its aura its second effect) isn't cast again while it is
/// on it, but is once the holder of the spell it has holds no second
/// effect (another of its effects alone).
#[tokio::test]
async fn a_pets_buff_is_cast_again_only_for_an_effect_it_has_lost() {
    let (_realm, _h, _gh, map, pet, _wolf) = tamed_with(world()).await;
    let out = map
        .query(move |m| {
            autocasts(m, pet, &[PAIR]);
            think(m, pet);
            let first = cast_now(m, pet, PAIR) && has(m, pet, PAIR);
            think(m, pet);
            let held = cast_now(m, pet, PAIR);
            // The holder's second effect gone, another of it left.
            let c = m.creature_mut(pet).unwrap();
            let h = c
                .auras
                .holders
                .iter_mut()
                .find(|h| h.spell == PAIR)
                .unwrap();
            for a in h.effects.iter_mut() {
                a.effect = 0;
            }
            think(m, pet);
            (first, held, cast_now(m, pet, PAIR))
        })
        .await
        .unwrap();
    assert_eq!(out, (true, false, true));
}
