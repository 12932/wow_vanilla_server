//! Round 7's pet tests, ported from Thistle Tea's `test/game/core/pet/`
//! (docs/parity/matrix.csv): Revive Pet and what it refuses, a new pet
//! that is new, and what a pet's master's death or departure ends.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::MapHandle;
use crate::game::map::pets;
use crate::game::tests_p5_ai_pets::tamed_with;
use crate::game::tests_pets::{
    self, BOAR, ENSLAVE_DEMON, FELHUNTER, STABLE_MASTER, SUMMON_IMP, TAME_BEAST, cast, enter,
    learn, pet_bar, place, stable_result,
};

/// Revive Pet at 0%, and at 399%: Thistle Tea's bounds.
const REVIVE_NONE: u32 = 990_700;
const REVIVE_OVER: u32 = 990_701;
/// Rebirth (a flat 400 health and 700 mana) and Goblin Jumper Cables (14%):
/// a player's resurrections.
const REBIRTH: u32 = 20_484;
const JUMPER_CABLES: u32 = 8342;

const POWERS: std::ops::Range<u16> = UNIT_FIELD_POWER1..UNIT_FIELD_POWER1 + 5;

fn revive_data() -> GameData {
    let mut d = tests_pets::data();
    let none = SpellEffect::default();
    for (id, points) in [(REVIVE_NONE, -1), (REVIVE_OVER, 398)] {
        d.spells.spells.insert(
            id,
            tests_pets::spell(
                id,
                "Revive Pet",
                [
                    tests_pets::effect(pets::EFFECT_SUMMON_DEAD_PET, points, 1),
                    none,
                    none,
                ],
            ),
        );
    }
    let mut rebirth = tests_pets::effect(113, 399, 6);
    rebirth.misc_value = 700;
    d.spells.spells.insert(
        REBIRTH,
        tests_pets::spell(REBIRTH, "Rebirth", [rebirth, none, none]),
    );
    d.spells.spells.insert(
        JUMPER_CABLES,
        tests_pets::spell(
            JUMPER_CABLES,
            "Goblin Jumper Cables",
            [tests_pets::effect(18, 13, 6), none, none],
        ),
    );
    d
}

/// The result of `spell`'s cast as its caster hears it: none for a cast
/// that went, else the failure.
async fn cast_result(c: &mut Client, spell: u32) -> Option<u8> {
    let b = c
        .wait_frame(
            "SMSG_CAST_RESULT",
            crate::game::map::spells::SMSG_CAST_RESULT,
            |b| b[..4] == spell.to_le_bytes(),
        )
        .await;
    (b[4] != 0).then(|| b[5])
}

/// A unit's health, its maximum, and its powers, if it is alive.
async fn vitals(map: &MapHandle, g: Guid) -> Option<(u32, u32, Vec<u32>)> {
    map.query(move |m| {
        let c = m.creature(g)?;
        c.alive().then(|| {
            (
                c.fields.get_u32(UNIT_FIELD_HEALTH),
                c.fields.get_u32(UNIT_FIELD_MAXHEALTH),
                POWERS.map(|f| c.fields.get_u32(f)).collect(),
            )
        })
    })
    .await
    .flatten()
}

/// Wait until `g` is up again, and what it has then.
async fn revived(map: &MapHandle, g: Guid) -> (u32, u32, Vec<u32>) {
    for _ in 0..200 {
        if let Some(v) = vitals(map, g).await {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{g} never rose");
}

/// Kill `victim` (by `killer`), and its powers as it fell.
async fn slay(map: &MapHandle, killer: Guid, victim: Guid) -> Vec<u32> {
    map.query(move |m| {
        crate::game::map::combat::kill(m, killer, victim);
        let c = m.creature(victim).unwrap();
        assert!(!c.alive(), "{victim} dead");
        POWERS.map(|f| c.fields.get_u32(f)).collect()
    })
    .await
    .unwrap()
}

/// Thistle Tea's `PetResurrection.revive` "clamps health without granting
/// the player resurrection mana amount", "revives summoned pets ...
/// without restoring mana" and "rejects ordinary creatures, ... and
/// percent resurrection": Revive Pet is refused while the pet lives, and a
/// creature that isn't the hunter's stays dead. A player's resurrections
/// raise no pet: Rebirth's flat health and mana and Jumper Cables'
/// percent alike (mangos-classic's `EffectResurrect` and
/// `EffectResurrectNew` take players only; Thistle Tea lets Rebirth raise
/// one). Revive Pet's percent is held to 1..100: none brings it up at one
/// health point, 399 at its whole health; and its powers are as they
/// fell, as a warlock's imp's mana is (mangos's `EffectSummonDeadPet` sets
/// the health alone).
#[tokio::test]
async fn revive_pet_raises_only_its_masters_pet_held_to_its_health_and_no_mana() {
    let (realm, mut h, gh, map, pet, wolf) = tamed_with(revive_data()).await;
    h.sees(pet).await;
    learn(
        &map,
        gh,
        &[REVIVE_NONE, REVIVE_OVER, REBIRTH, JUMPER_CABLES],
    );
    cast(&h, REVIVE_NONE, None).await;
    assert_eq!(
        cast_result(&mut h, REVIVE_NONE).await,
        Some(SpellCastResult::TargetNotDead.as_int()),
        "its pet alive"
    );
    // A dead wolf is no pet of hers: Revive Pet leaves it be.
    slay(&map, gh, wolf).await;
    cast(&h, REVIVE_NONE, None).await;
    assert!(cast_result(&mut h, REVIVE_NONE).await.is_some());
    assert!(vitals(&map, wolf).await.is_none(), "the wolf stays dead");
    let fell = slay(&map, wolf, pet).await;
    for spell in [REBIRTH, JUMPER_CABLES] {
        cast(&h, spell, Some(pet)).await;
        let _ = cast_result(&mut h, spell).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(vitals(&map, pet).await.is_none(), "{spell} raises no pet");
    }
    cast(&h, REVIVE_NONE, None).await;
    let (health, _, powers) = revived(&map, pet).await;
    assert_eq!(health, 1, "none: one point");
    assert_eq!(powers, fell, "its powers as they fell");
    slay(&map, wolf, pet).await;
    cast(&h, REVIVE_OVER, None).await;
    let (health, max, _) = revived(&map, pet).await;
    assert_eq!(health, max, "399: its whole health");
    // A warlock's imp, killed and revived within its corpse's five
    // seconds, has its mana as it fell.
    let (mut w, gw, wmap) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&wmap, gw, &[SUMMON_IMP, REVIVE_OVER]);
    cast(&w, SUMMON_IMP, None).await;
    let imp = pet_bar(&mut w).await;
    wmap.query(move |m| {
        m.creature_mut(imp)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_POWER1, 12);
    })
    .await;
    let fell = slay(&wmap, gw, imp).await;
    assert_eq!(fell[0], 12);
    cast(&w, REVIVE_OVER, None).await;
    let (health, max, powers) = revived(&wmap, imp).await;
    assert_eq!((health, powers), (max, fell), "the imp's mana as it fell");
}

/// Thistle Tea's `Companion.remember_death` "retains death through
/// suspension and clears it when revived": a hunter's pet that died is
/// dead still after its hunter logs out and in (no pet at login, Call Pet
/// refused), and Revive Pet brings it back from its record, alive, and so
/// it comes with her at the next login.
#[tokio::test]
async fn a_dead_pet_stays_dead_over_a_logout_until_revived() {
    let (_realm, mut h, gh, map, pet, wolf) = tamed_with(revive_data()).await;
    h.sees(pet).await;
    learn(&map, gh, &[REVIVE_OVER]);
    slay(&map, wolf, pet).await;
    map.query(move |m| {
        m.remove_creature(wolf);
        crate::game::map::combat::leave_combat(m, gh);
    })
    .await;
    let relog = async |h: &mut Client| {
        h.send(CMSG_LOGOUT_REQUEST {}).await;
        h.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
        })
        .await;
        h.clear();
        h.login(gh).await;
        h.sees(gh).await;
        crate::game::harness::an_update(&map).await;
        map.query(move |m| m.players[&gh].owned.pet).await.unwrap()
    };
    assert_eq!(relog(&mut h).await, None, "no dead pet at login");
    let dead = map
        .query(move |m| {
            m.players[&gh]
                .row
                .state
                .pets
                .current
                .as_ref()
                .map(|r| r.dead)
        })
        .await
        .unwrap();
    assert_eq!(dead, Some(true), "dead in its record");
    learn(&map, gh, &[tests_pets::CALL_PET]);
    cast(&h, tests_pets::CALL_PET, None).await;
    assert_eq!(
        cast_result(&mut h, tests_pets::CALL_PET).await,
        Some(SpellCastResult::TargetsDead.as_int()),
        "not called dead"
    );
    cast(&h, REVIVE_OVER, None).await;
    let back = pet_bar(&mut h).await;
    assert!(!back.is_zero(), "revived from its record");
    let (health, max, _) = revived(&map, back).await;
    assert_eq!(health, max);
    let again = relog(&mut h).await;
    assert!(again.is_some(), "alive, it comes back at login");
}

/// What a hunter's pet out now is: its name, happiness, stance and
/// autocast.
async fn identity(map: &MapHandle, pet: Guid) -> (String, u32, u8, Vec<u32>) {
    map.query(move |m| {
        let c = m.creature(pet).unwrap();
        let s = c.pet.as_deref().unwrap();
        (
            s.name.clone(),
            c.fields.get_u32(UNIT_FIELD_POWER1 + pets::POWER_HAPPINESS),
            s.react,
            s.autocast.clone(),
        )
    })
    .await
    .unwrap()
}

/// Thistle Tea's `PetNaming.initialize` "does not give a new pet the
/// previous pet's chosen name", `Companion.remember_controls` "resets
/// enabled spells when a different companion is activated" and
/// `remember_happiness` "forgets happiness when the pet is abandoned or
/// replaced": a hunter's boar, named Fang, happy, passive and with a spell
/// on autocast, goes to the stable; the boar she tames next has its
/// template's name, a tamed beast's mood, the defensive stance and no
/// autocast of the first's; and abandoned, the next after it is as new.
/// Fang, taken out again, is as he was.
#[tokio::test]
async fn a_new_pet_has_nothing_of_the_last() {
    let (_realm, mut h, gh, map, pet, _wolf) = tamed_with(revive_data()).await;
    h.sees(pet).await;
    let master = place(&map, gh, STABLE_MASTER, 0x70_0004, 2.0).await;
    h.sees(master).await;
    map.call(move |m| {
        if let Some(p) = m.players.get_mut(&gh) {
            p.set_money(10_000);
        }
    });
    let fresh = identity(&map, pet).await;
    assert_eq!(fresh.0, "Tame Boar");
    h.send(CMSG_PET_RENAME {
        pet,
        name: "Fang".into(),
    })
    .await;
    h.send(CMSG_PET_ACTION {
        pet,
        data: pets::button(u32::from(pets::REACT_PASSIVE), pets::ACT_REACTION),
        target: Guid::zero(),
    })
    .await;
    map.query(move |m| {
        let c = m.creature_mut(pet).unwrap();
        c.fields
            .set_u32(UNIT_FIELD_POWER1 + pets::POWER_HAPPINESS, 750_000);
        let s = c.pet.as_deref_mut().unwrap();
        s.spells.push(tests_pets::GORE);
        s.autocast.push(tests_pets::GORE);
    })
    .await;
    let named = loop {
        let now = identity(&map, pet).await;
        if now.0 == "Fang" && now.2 == pets::REACT_PASSIVE {
            break now;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    h.send(CMSG_BUY_STABLE_SLOT { npc: master }).await;
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessBuySlot);
    h.send(CMSG_STABLE_PET {
        stable_master: master,
    })
    .await;
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessStable);
    let tame = async |h: &mut Client, counter: u32| {
        let boar = place(&map, gh, BOAR, counter, 4.0).await;
        h.sees(boar).await;
        h.clear();
        cast(h, TAME_BEAST, Some(boar)).await;
        let next = pet_bar(h).await;
        assert!(!next.is_zero(), "tamed");
        next
    };
    let second = tame(&mut h, 0x70_0011).await;
    assert_ne!(second, pet);
    assert_eq!(identity(&map, second).await, fresh, "a new boar is new");
    h.send(CMSG_PET_ABANDON { pet: second }).await;
    pet_bar(&mut h).await;
    let third = tame(&mut h, 0x70_0012).await;
    assert_eq!(identity(&map, third).await, fresh, "and after an abandon");
    h.send(CMSG_STABLE_SWAP_PET {
        npc: master,
        pet_slot: map
            .query(move |m| m.players[&gh].row.state.pets.stable[0].1.number)
            .await
            .unwrap(),
    })
    .await;
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessUnstable);
    let fang = loop {
        let g = pet_bar(&mut h).await;
        if !g.is_zero() {
            break g;
        }
    };
    let back = identity(&map, fang).await;
    assert_eq!(
        (&back.0, back.2, &back.3),
        (&named.0, named.2, &named.3),
        "Fang as he was"
    );
    assert!(back.1 > 700_000, "happy still: {}", back.1);
}

/// Thistle Tea's `Companion.dismiss` "releases controlled companions
/// through the same transition", and `Companion.activate` "suspension and
/// process loss clear possession while retaining the hunter pet": a
/// warlock's enslaved demon is let go at her death, her charm field
/// cleared; and a hunter seeing through her pet's eyes sees through her
/// own again when she puts the pet away, and when she logs out, and her
/// pet is hers still, back at her call or at login, no longer possessed.
#[tokio::test]
async fn a_masters_death_lets_a_charm_go_and_putting_a_pet_away_ends_its_eyes() {
    use crate::game::map::auras;
    let realm = crate::game::harness::realm_with(revive_data());
    let (w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&map, gw, &[ENSLAVE_DEMON]);
    let demon = place(&map, gw, FELHUNTER, 0x70_0005, 5.0).await;
    cast(&w, ENSLAVE_DEMON, Some(demon)).await;
    for _ in 0..200 {
        let held = map
            .query(move |m| m.players[&gw].owned.charm == Some(demon))
            .await
            .unwrap();
        if held {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let after = map
        .query(move |m| {
            assert_eq!(m.players[&gw].owned.charm, Some(demon), "enslaved");
            crate::game::map::death::kill(m, gw);
            let p = &m.players[&gw];
            (
                p.owned.charm,
                p.fields.get_guid(UNIT_FIELD_CHARM),
                m.creature(demon).and_then(|c| c.owner),
                m.creature(demon)
                    .map(|c| c.fields.get_guid(UNIT_FIELD_CHARMEDBY)),
            )
        })
        .await
        .unwrap();
    assert_eq!(after, (None, Guid::zero(), None, Some(Guid::zero())));
    // Eyes of the Beast, ended by Dismiss Pet, then by a logout.
    let (_realm, mut h, gh, hmap, pet, _wolf) =
        crate::game::tests_pets2::tamed_on(realm.clone()).await;
    h.sees(pet).await;
    let look = move |m: &mut crate::game::map::Map, pet: Guid| {
        let mut eyes = crate::game::tests_pets::spell(
            990_702,
            "Eyes of the Beast",
            [
                tests_pets::effect(6, 0, 5),
                SpellEffect::default(),
                SpellEffect::default(),
            ],
        );
        eyes.effects[0].aura = auras::AURA_MOD_POSSESS_PET;
        let _ = auras::apply_spell(m, gh, pet, &eyes, &[(0, 1)], auras::Context::default());
        pets::set_mover(m, gh, pet);
        m.players[&gh].fields.get_guid(PLAYER_FARSIGHT) == pet
    };
    let seen = move |m: &crate::game::map::Map| {
        let p = &m.players[&gh];
        (
            p.fields.get_guid(PLAYER_FARSIGHT),
            p.owned.mover,
            p.fields.get_guid(UNIT_FIELD_CHARM),
        )
    };
    let away = hmap
        .query(move |m| {
            assert!(look(m, pet), "through its eyes");
            pets::dismiss(m, pet);
            seen(m)
        })
        .await
        .unwrap();
    assert_eq!(away, (Guid::zero(), None, Guid::zero()), "put away");
    learn(&hmap, gh, &[tests_pets::CALL_PET]);
    h.clear();
    cast(&h, tests_pets::CALL_PET, None).await;
    let called = pet_bar(&mut h).await;
    assert!(!called.is_zero(), "called back");
    let possessed = move |m: &crate::game::map::Map, g: Guid| {
        m.creature(g).is_some_and(|c| {
            c.fields
                .has_flag(UNIT_FIELD_FLAGS, pets::UNIT_FLAG_POSSESSED)
        })
    };
    assert!(
        !hmap.query(move |m| possessed(m, called)).await.unwrap(),
        "no longer possessed"
    );
    assert!(hmap.query(move |m| look(m, called)).await.unwrap());
    h.disconnect();
    // Gone and saved, before she comes back.
    for _ in 0..500 {
        if realm.registry.online(gh).is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let mut h = Client::connect(&realm, "H");
    h.login(gh).await;
    let at_login = pet_bar(&mut h).await;
    assert!(!at_login.is_zero(), "hers at login");
    crate::game::harness::an_update(&hmap).await;
    let (eyes, still) = hmap
        .query(move |m| (seen(m), possessed(m, at_login)))
        .await
        .unwrap();
    assert_eq!(eyes, (Guid::zero(), None, Guid::zero()), "her own eyes");
    assert!(!still, "no longer possessed");
}

/// Thistle Tea's `PetLearning.resolve` "uses the reference ten winning
/// outcomes out of 101": a roll of 0 to 100 teaches below 10 (mangos's
/// `Pet::CheckLearning`, `urand(0, 100) < 10`).
#[test]
fn ten_rolls_in_101_teach() {
    use crate::game::map::pets::testing::teaches;
    assert!(teaches(0) && teaches(9));
    assert!(!teaches(10) && !teaches(100));
    assert_eq!((0..=100).filter(|r| teaches(*r)).count(), 10);
}

/// Thistle Tea's `PetLearning.used` "only a living, owned hunter pet can
/// request discovery": with the rolls on, a hunter's pet using Gore
/// teaches her to teach it within some hundreds of uses, but not dead, nor
/// with a passive; a warlock's imp's Firebolt teaches her nothing; and
/// with the rolls fixed (certainty only), no use teaches.
#[tokio::test]
async fn only_a_living_hunters_pet_teaches_its_master_by_chance() {
    const TEACH_FIREBOLT: u32 = 990_703;
    const PASSIVE: u32 = 990_704;
    const TEACH_PASSIVE: u32 = 990_705;
    let mut d = revive_data();
    let none = SpellEffect::default();
    for id in [TEACH_FIREBOLT, TEACH_PASSIVE] {
        d.spells
            .spells
            .insert(id, tests_pets::spell(id, "Teaching", [none, none, none]));
    }
    let mut passive = tests_pets::spell(PASSIVE, "Passive", [none, none, none]);
    passive.attributes[0] = 0x40;
    d.spells.spells.insert(PASSIVE, passive);
    d.pets.teaching.insert(tests_pets::FIREBOLT, TEACH_FIREBOLT);
    d.pets.teaching.insert(PASSIVE, TEACH_PASSIVE);
    let (realm, mut h, gh, map, pet, _wolf) = tamed_with(d).await;
    h.sees(pet).await;
    let uses = move |m: &mut crate::game::map::Map, g: Guid, spell: u32, owner: Guid, teaching| {
        let t = m.realm.data.spells.get(spell).cloned().unwrap();
        for _ in 0..500 {
            pets::before_effects(m, g, &t, &Default::default());
            if m.players[&owner].spells.contains(&teaching) {
                return true;
            }
        }
        false
    };
    let gore = tests_pets::GORE;
    let train = tests_pets::TRAIN_GORE;
    let (fixed, passive, alive) = map
        .query(move |m| {
            let fixed = uses(m, pet, gore, gh, train);
            m.spells.rolls = Some(true);
            let passive = uses(m, pet, PASSIVE, gh, TEACH_PASSIVE);
            (fixed, passive, uses(m, pet, gore, gh, train))
        })
        .await
        .unwrap();
    assert!(!fixed, "the rolls fixed, nothing taught");
    assert!(!passive, "a passive teaches nothing");
    assert!(alive, "Gore taught");
    let dead = map
        .query(move |m| {
            m.players
                .get_mut(&gh)
                .unwrap()
                .spells
                .retain(|s| *s != train);
            crate::game::map::combat::kill(m, gh, pet);
            uses(m, pet, gore, gh, train)
        })
        .await
        .unwrap();
    assert!(!dead, "dead, it teaches nothing");
    let (mut w, gw, wmap) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&wmap, gw, &[SUMMON_IMP]);
    cast(&w, SUMMON_IMP, None).await;
    let imp = pet_bar(&mut w).await;
    let demon = wmap
        .query(move |m| {
            m.spells.rolls = Some(true);
            uses(m, imp, tests_pets::FIREBOLT, gw, TEACH_FIREBOLT)
        })
        .await
        .unwrap();
    assert!(!demon, "an imp teaches nothing");
}

/// Thistle Tea's `PetSpellModifiers.sync` "rejects stale owners and
/// respects spell family and mask" and "does not refresh temporary or
/// externally cast auras": a hunter's boar has a health passive of the
/// hunters' family (9) at nothing. Another hunter's modifier of it changes
/// nothing of the boar's; nor do its master's of another family, or of a
/// mask the passive's flags don't meet; one that touches it makes it 15,
/// and the other hunter's 10% of the same family on the boar, timed,
/// stays as it was (mangos's `ReapplyAffectedPassiveAuras`: the unit's own
/// permanent passives only).
#[tokio::test]
async fn a_masters_modifiers_reach_only_their_pets_passives_they_touch() {
    use crate::game::map::auras;
    const PASSIVE: u32 = 990_710;
    const TOUCHES: u32 = 990_711;
    const OTHER_FAMILY: u32 = 990_712;
    const OTHER_MASK: u32 = 990_713;
    const TIMED: u32 = 990_714;
    let mut d = revive_data();
    let none = SpellEffect::default();
    let mut health = tests_pets::effect(6, -1, 1);
    health.aura = 133;
    let mut passive = tests_pets::spell(PASSIVE, "Tamed Pet Passive", [health, none, none]);
    passive.attributes[0] = 0x40;
    passive.family_name = 9;
    passive.family_flags = 1;
    d.spells.spells.insert(PASSIVE, passive);
    d.pets.abilities.insert(
        270,
        vec![crate::game::data::pets::PetAbility {
            spell: PASSIVE,
            skill: 270,
            acquire: 2,
            ..Default::default()
        }],
    );
    for (id, family, mask) in [(TOUCHES, 9, 1), (OTHER_FAMILY, 5, 1), (OTHER_MASK, 9, 2)] {
        let mut e = tests_pets::effect(6, 14, 1);
        e.aura = auras::AURA_ADD_FLAT_MODIFIER;
        e.misc_value = 8;
        e.item_type = mask;
        let mut s = tests_pets::spell(id, "Endurance Training", [e, none, none]);
        s.family_name = family;
        d.spells.spells.insert(id, s);
    }
    let mut timed = tests_pets::effect(6, 9, 1);
    timed.aura = 133;
    let mut t = tests_pets::spell(TIMED, "Timed", [timed, none, none]);
    t.family_name = 9;
    t.family_flags = 1;
    t.duration_index = 4;
    d.spells.spells.insert(TIMED, t);
    let (realm, mut h, gh, map, pet, _wolf) = tamed_with(d).await;
    h.sees(pet).await;
    let (_o, go, _) = enter(&realm, "O", "Other", Race::Dwarf, Class::Hunter).await;
    let amount = move |m: &crate::game::map::Map, spell: u32| {
        auras::auras(m, pet).and_then(|a| {
            a.holders
                .iter()
                .find(|h| h.spell == spell)
                .map(|h| h.effects[0].amount)
        })
    };
    let on = move |m: &mut crate::game::map::Map, who: Guid, spell: u32| {
        let s = m.realm.data.spells.get(spell).cloned().unwrap();
        let ctx = auras::Context {
            duration: Some(None),
            ..Default::default()
        };
        let _ = auras::apply_spell(m, who, who, &s, &[(0, 15)], ctx);
    };
    let out = map
        .query(move |m| {
            let first = amount(m, PASSIVE);
            on(m, go, TOUCHES);
            let another_hunters = amount(m, PASSIVE);
            on(m, gh, OTHER_FAMILY);
            on(m, gh, OTHER_MASK);
            let missed = amount(m, PASSIVE);
            let t = m.realm.data.spells.get(TIMED).cloned().unwrap();
            let _ = auras::apply_spell(m, go, pet, &t, &[(0, 10)], Default::default());
            on(m, gh, TOUCHES);
            let count = auras::auras(m, pet).map_or(0, |a| {
                a.holders.iter().filter(|h| h.spell == PASSIVE).count()
            });
            (
                first,
                another_hunters,
                missed,
                amount(m, PASSIVE),
                count,
                amount(m, TIMED),
            )
        })
        .await
        .unwrap();
    assert_eq!(out.0, Some(0), "at nothing");
    assert_eq!(out.1, Some(0), "another hunter's modifier");
    assert_eq!(out.2, Some(0), "another family's, another mask's");
    assert_eq!((out.3, out.4), (Some(15), 1), "its master's, once");
    assert_eq!(out.5, Some(10), "the other's timed aura as it was");
}

/// Thistle Tea's `Totems.prepare` "snapshots owner timing modifiers for
/// the summon and its periodic aura", and `State.prepare_worldport`
/// "dismisses independent wards before changing world": a shaman's talent
/// takes two seconds off a Fire Nova Totem's five (mangos's
/// `CalculateSpellDuration` with her modifiers), and two off its pulse's
/// four (the activation time of the totem's periodic aura, her modifier as
/// its spell modifier owner); and her ward and her slotted totem go when
/// she goes to another map.
#[tokio::test]
async fn a_shamans_talents_time_her_totem_and_her_totems_stay_behind_a_worldport() {
    use crate::game::map::{MapKey, auras};
    const NOVA_TOTEM: u32 = 990_720;
    const NOVA_TOTEM_SPELL: u32 = 990_721;
    const NOVA: u32 = 990_722;
    const NOVA_BLAST: u32 = 990_723;
    const TALENT: u32 = 990_724;
    const WARD: u32 = 990_725;
    let mut d = revive_data();
    let none = SpellEffect::default();
    let mut t = tests_pets::template(NOVA_TOTEM, "Fire Nova Totem", 35, 1);
    t.creature_type = 11;
    d.creature_templates.insert(NOVA_TOTEM, t);
    d.pets.totem_spells.insert(NOVA_TOTEM, NOVA);
    d.spells.durations.insert(
        5,
        crate::game::data::spells::SpellDuration {
            base: 5000,
            ..Default::default()
        },
    );
    let mut summon = tests_pets::effect(pets::EFFECT_SUMMON_TOTEM_SLOT1, 4, 1);
    summon.misc_value = NOVA_TOTEM as i32;
    let mut totem = tests_pets::spell(NOVA_TOTEM_SPELL, "Fire Nova Totem", [summon, none, none]);
    totem.duration_index = 5;
    totem.family_name = 11;
    totem.family_flags = 0x2800_0000;
    let mut pulse = tests_pets::effect(6, 0, 1);
    pulse.aura = 23;
    pulse.amplitude = 4000;
    pulse.trigger_spell = NOVA_BLAST;
    let mut nova = tests_pets::spell(NOVA, "Fire Nova", [pulse, none, none]);
    nova.duration_index = 21;
    nova.family_name = 11;
    nova.family_flags = 0x20;
    let blast = tests_pets::spell(NOVA_BLAST, "Fire Nova", [none, none, none]);
    let modifier = |op: i32, mask: u64| {
        let mut e = tests_pets::effect(6, -2001, 1);
        e.aura = auras::AURA_ADD_FLAT_MODIFIER;
        e.misc_value = op;
        e.item_type = mask;
        e
    };
    let mut talent = tests_pets::spell(
        TALENT,
        "Improved Fire Nova Totem",
        [modifier(1, 0x0800_0000), modifier(19, 0x20), none],
    );
    talent.family_name = 11;
    talent.attributes[0] = 0x40;
    let mut ward_effect = tests_pets::effect(pets::EFFECT_SUMMON_TOTEM, 4, 1);
    ward_effect.misc_value = tests_pets::STONESKIN_TOTEM as i32;
    let mut ward = tests_pets::spell(WARD, "Ward", [ward_effect, none, none]);
    ward.duration_index = 4;
    for s in [totem, nova, blast, talent, ward] {
        d.spells.spells.insert(s.id, s);
    }
    let realm = crate::game::harness::realm_with(d);
    let (mut s, gs, map) = enter(&realm, "S", "Shaman", Race::Orc, Class::Shaman).await;
    learn(&map, gs, &[NOVA_TOTEM_SPELL, WARD]);
    map.query(move |m| crate::game::map::spells::learn(m, gs, TALENT))
        .await;
    cast(&s, NOVA_TOTEM_SPELL, None).await;
    let mut timing = None;
    for _ in 0..200 {
        timing = map
            .query(move |m| {
                let t = m.players[&gs].owned.totems[0]?;
                let left = pets::state(m, t)?.expires?.duration_since(m.now);
                let period = auras::auras(m, t)?
                    .holders
                    .iter()
                    .find(|h| h.spell == NOVA)?
                    .effects[0]
                    .period?;
                Some((left, period))
            })
            .await
            .flatten();
        if timing.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (left, period) = timing.expect("the totem and its pulse");
    assert!(
        left <= Duration::from_millis(3000) && left > Duration::from_millis(2500),
        "three seconds: {left:?}"
    );
    assert_eq!(period, Duration::from_millis(2000), "a pulse of two");
    cast(&s, WARD, None).await;
    let (slotted, wards) = loop {
        let now = map
            .query(move |m| {
                let o = &m.players[&gs].owned;
                (o.totems[0], o.loose_totems.clone())
            })
            .await
            .unwrap();
        if !now.1.is_empty() {
            break now;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let slotted = slotted.expect("the nova totem stands");
    // An orc's Kalimdor for Northshire.
    let to = Vector3d {
        x: -8_949.0,
        y: -132.0,
        z: 83.5,
    };
    map.call(move |m| m.teleport(gs, MapKey::world(0), to, 0.0));
    s.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    let left = map
        .query(move |m| {
            wards
                .iter()
                .chain([&slotted])
                .filter(|g| m.creature(**g).is_some())
                .count()
        })
        .await
        .unwrap();
    assert_eq!(left, 0, "her totems gone with her");
}

/// Thistle Tea's `MiniPet` "destination summons appear beside the owner
/// and face back": a companion comes two yards off its owner's front
/// left, facing them.
#[tokio::test]
async fn a_companion_comes_beside_its_owner_facing_them() {
    const CRITTER: u32 = 990_730;
    const CALL: u32 = 990_731;
    let mut d = revive_data();
    d.creature_templates
        .insert(CRITTER, tests_pets::template(CRITTER, "Critter", 35, 1));
    let mut e = tests_pets::effect(pets::EFFECT_SUMMON_CRITTER, 0, 32);
    e.misc_value = CRITTER as i32;
    d.spells.spells.insert(
        CALL,
        tests_pets::spell(
            CALL,
            "Call Critter",
            [e, SpellEffect::default(), SpellEffect::default()],
        ),
    );
    let realm = crate::game::harness::realm_with(d);
    let (_c, g, map) = enter(&realm, "C", "Owner", Race::Human, Class::Warrior).await;
    let (owner, critter) = map
        .query(move |m| {
            crate::game::map::spells::cast_triggered(m, g, CALL, Some(g));
            let p = &m.players[&g];
            let c = m.creature(p.owned.critter?)?;
            Some((
                (p.position(), p.movement.orientation),
                (c.position, c.orientation),
            ))
        })
        .await
        .flatten()
        .expect("a companion");
    let ((at, o), (pos, facing)) = (owner, critter);
    let a = o + std::f32::consts::FRAC_PI_4;
    assert!((pos.x - (at.x + 2.0 * a.cos())).abs() < 0.01, "{pos:?}");
    assert!((pos.y - (at.y + 2.0 * a.sin())).abs() < 0.01, "{pos:?}");
    let back = (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU);
    assert!((facing - back).abs() < 0.01, "facing {facing}, not {back}");
}

/// Thistle Tea's `PetProgression.gain` "preserves active buffs while
/// replacing canonical growth inputs": a hunter's pet with five stamina
/// and forty attack power from buffs levels up; its stats are the new
/// level's with the buffs on top (fifty health and forty attack power
/// more than without them), at its whole health, and the buffs still on.
#[tokio::test]
async fn a_pet_levelling_keeps_its_buffs_over_the_new_levels_stats() {
    use crate::game::map::auras;
    const STAMINA: u32 = 990_740;
    const MIGHT: u32 = 990_741;
    let mut d = revive_data();
    let none = SpellEffect::default();
    let mut stamina = tests_pets::effect(6, 4, 1);
    stamina.aura = 29;
    stamina.misc_value = 2;
    let mut might = tests_pets::effect(6, 39, 1);
    might.aura = 99;
    for (id, e) in [(STAMINA, stamina), (MIGHT, might)] {
        let mut s = tests_pets::spell(id, "Buff", [e, none, none]);
        s.duration_index = 4;
        d.spells.spells.insert(id, s);
    }
    let (_realm, mut h, gh, map, pet, _wolf) = tamed_with(d).await;
    h.sees(pet).await;
    let out = map
        .query(move |m| {
            let data = m.realm.data.clone();
            for (spell, amount) in [(STAMINA, 5), (MIGHT, 40)] {
                let s = data.spells.get(spell).cloned().unwrap();
                let _ = auras::apply_spell(m, gh, pet, &s, &[(0, amount)], Default::default());
            }
            let level = |m: &crate::game::map::Map| {
                let f = &m.creature(pet).unwrap().fields;
                (
                    f.get_u32(UNIT_FIELD_LEVEL),
                    f.get_u32(UNIT_FIELD_HEALTH),
                    f.get_u32(UNIT_FIELD_MAXHEALTH),
                    f.get_u32(UNIT_FIELD_ATTACK_POWER) as i32
                        + f.get_u32(UNIT_FIELD_ATTACK_POWER_MODS) as i32,
                )
            };
            let before = level(m);
            let need = crate::game::map::pets::testing::pet_xp_to_level(&data, before.0 as u8);
            crate::game::map::pets::testing::gain_xp(m, pet, need, 20);
            let buffed = level(m);
            let kept = [STAMINA, MIGHT].map(|s| auras::auras(m, pet).unwrap().has_spell(s));
            auras::remove_spell(m, pet, STAMINA, None);
            auras::remove_spell(m, pet, MIGHT, None);
            (before, buffed, kept, level(m))
        })
        .await
        .unwrap();
    let (before, buffed, kept, bare) = out;
    assert_eq!(buffed.0, before.0 + 1, "a level up");
    assert_eq!(buffed.1, buffed.2, "at its whole health");
    assert_eq!(kept, [true, true], "its buffs on");
    assert_eq!(buffed.2 - bare.2, 50, "five stamina's health");
    assert_eq!(buffed.3 - bare.3, 40, "forty attack power");
}

/// Thistle Tea's `PetControls.update` "restores moved buttons and filters
/// obsolete or ineligible spell settings": a hunter's boar's bar has its
/// Attack and Stay changed places and Gore moved to the last spell slot, a
/// spell that no longer is and one it doesn't know on it, and autocast on
/// for Gore, a passive and the two; dismissed and called back, its bar
/// keeps what was moved, the two are empty slots, and only Gore is on
/// autocast (mangos's `LoadPetActionBar` and `CleanupActionBar`).
#[tokio::test]
async fn a_pets_bar_comes_back_as_moved_without_what_it_cant_use() {
    use crate::game::map::pets::{ACT_COMMAND, ACT_DISABLED, ACT_ENABLED, button, unpack};
    const GONE: u32 = 990_750;
    const PASSIVE: u32 = 990_751;
    let mut d = revive_data();
    let none = SpellEffect::default();
    let mut passive = tests_pets::spell(PASSIVE, "Passive", [none, none, none]);
    passive.attributes[0] = 0x40;
    d.spells.spells.insert(PASSIVE, passive);
    let (_realm, mut h, gh, map, pet, _wolf) = tamed_with(d).await;
    h.sees(pet).await;
    let gore = tests_pets::GORE;
    let firebolt = tests_pets::FIREBOLT;
    map.query(move |m| {
        let s = pets::state_mut(m, pet).unwrap();
        s.spells.extend([gore, PASSIVE]);
        s.bar[0] = button(u32::from(pets::COMMAND_STAY), ACT_COMMAND);
        s.bar[2] = button(u32::from(pets::COMMAND_ATTACK), ACT_COMMAND);
        s.bar[3] = button(GONE, ACT_ENABLED);
        s.bar[4] = button(firebolt, ACT_ENABLED);
        s.bar[6] = button(gore, ACT_ENABLED);
        s.autocast = vec![gore, PASSIVE, GONE, firebolt];
        pets::dismiss(m, pet);
    })
    .await;
    learn(&map, gh, &[tests_pets::CALL_PET]);
    h.clear();
    cast(&h, tests_pets::CALL_PET, None).await;
    let back = pet_bar(&mut h).await;
    assert!(!back.is_zero());
    let (bar, autocast) = map
        .query(move |m| {
            let s = pets::state(m, back).unwrap();
            (s.bar.map(unpack), s.autocast.clone())
        })
        .await
        .unwrap();
    assert_eq!(bar[0], (u32::from(pets::COMMAND_STAY), ACT_COMMAND));
    assert_eq!(bar[2], (u32::from(pets::COMMAND_ATTACK), ACT_COMMAND));
    assert_eq!(bar[3], (0, ACT_DISABLED), "a spell that no longer is");
    assert_eq!(bar[4], (0, ACT_DISABLED), "one it doesn't know");
    assert_eq!(bar[6], (gore, ACT_ENABLED), "Gore where it was put");
    assert_eq!(autocast, vec![gore], "only Gore on autocast");
}

/// Thistle Tea's `WildSummon` "summons once on the caster with count and
/// destination", "uses the forward radius without a destination and
/// normalizes indefinite lifetime", "a corpse calls up a creature only
/// with a spell castable while dead", and `TemporarySummon.tick` "defers
/// timed death until combat ends without extending the deadline": a
/// creature's wild summon of three at its spell's database place has the
/// first there and the others within the radius about it, for five
/// seconds; one at the caster's own place (18) comes there; one of no
/// place comes its radius ahead, for good (mangos's `EffectSummonWild`).
/// Dead, the creature calls up none, but by a spell castable while dead.
/// A summon whose time comes in a fight lives through it, and dies as
/// soon as it is over.
#[tokio::test]
async fn a_wild_summon_comes_at_its_place_or_ahead_and_dies_after_its_fight() {
    use crate::game::map::event_ai;
    const SUMMONER: u32 = 990_760;
    const WILD: u32 = 990_761;
    const AT_PLACE: u32 = 990_762;
    const AT_SELF: u32 = 990_763;
    const AHEAD: u32 = 990_764;
    const WHILE_DEAD: u32 = 990_765;
    // A place in Elwynn Forest, where she is.
    const PLACE: (u32, f32, f32, f32, f32) = (0, -8_940.0, -130.0, 83.0, 0.0);
    let mut d = revive_data();
    d.spells.target_positions.insert(AT_PLACE, PLACE);
    for (entry, name) in [(SUMMONER, "Summoner"), (WILD, "Wild")] {
        d.creature_templates
            .insert(entry, tests_pets::template(entry, name, 14, 1));
    }
    d.spells.radii.insert(8, 2.0);
    d.spells.durations.insert(
        5,
        crate::game::data::spells::SpellDuration {
            base: 5000,
            ..Default::default()
        },
    );
    let none = SpellEffect::default();
    let wild = |id: u32, count: i32, target: u32, duration: u32| {
        let mut e = tests_pets::effect(pets::EFFECT_SUMMON_WILD, count - 1, target);
        e.misc_value = WILD as i32;
        e.radius_index = 8;
        let mut s = tests_pets::spell(id, "Summon", [e, none, none]);
        s.duration_index = duration;
        s
    };
    for s in [
        wild(AT_PLACE, 3, 17, 5),
        wild(AT_SELF, 1, 18, 5),
        wild(AHEAD, 1, 0, 21),
        {
            let mut s = wild(WHILE_DEAD, 1, 0, 21);
            s.attributes[0] |= crate::game::data::spells::ATTR_CASTABLE_WHILE_DEAD;
            s
        },
    ] {
        d.spells.spells.insert(s.id, s);
    }
    let realm = crate::game::harness::realm_with(d);
    let (_c, g, map) = enter(&realm, "C", "Watcher", Race::Human, Class::Warrior).await;
    let summoner = place(&map, g, SUMMONER, 0x70_0020, 10.0).await;
    let out = map
        .query(move |m| {
            let at = m.position_of(summoner).unwrap();
            let o = m.creature(summoner).unwrap().orientation;
            let place = Vector3d {
                x: PLACE.1,
                y: PLACE.2,
                z: PLACE.3,
            };
            let made = |m: &crate::game::map::Map| {
                event_ai::scripted(m, summoner).map_or(Vec::new(), |s| s.summoned.clone())
            };
            let new_ones = |m: &mut crate::game::map::Map, spell: u32| {
                let before = made(m);
                crate::game::map::spells::cast(m, summoner, spell, Default::default(), None);
                let now = m.now;
                made(m)
                    .into_iter()
                    .filter(|x| !before.contains(x))
                    .map(|x| {
                        let c = m.creature(x).unwrap();
                        let left = c
                            .scripted
                            .as_deref()
                            .and_then(|s| s.despawn_at)
                            .map(|t| t.duration_since(now));
                        (x, c.position, left)
                    })
                    .collect::<Vec<_>>()
            };
            let three = new_ones(m, AT_PLACE);
            let own = new_ones(m, AT_SELF);
            let ahead = new_ones(m, AHEAD);
            crate::game::map::combat::kill(m, g, summoner);
            let dead = new_ones(m, AHEAD);
            let dead_spell = new_ones(m, WHILE_DEAD);
            (
                at,
                o,
                place,
                three,
                own,
                ahead,
                dead.len(),
                dead_spell.len(),
            )
        })
        .await
        .unwrap();
    let (at, o, place, three, own, ahead, dead, dead_spell) = out;
    let d2 = |a: Vector3d, b: Vector3d| crate::game::map::combat::dist(a, b);
    assert_eq!(three.len(), 3, "three");
    assert!(d2(three[0].1, place) < 0.01, "the first at the place");
    for (_, p, left) in &three {
        assert!(d2(*p, place) <= 2.01, "within the radius: {p:?}");
        assert_eq!(*left, Some(Duration::from_secs(5)), "for five seconds");
    }
    assert!(d2(own[0].1, at) < 0.01, "at the caster's own place");
    let front = Vector3d {
        x: at.x + 2.0 * o.cos(),
        y: at.y + 2.0 * o.sin(),
        z: at.z,
    };
    assert!(d2(ahead[0].1, front) < 0.01, "its radius ahead");
    assert_eq!(ahead[0].2, None, "for good");
    assert_eq!(
        (dead, dead_spell),
        (0, 1),
        "dead, only by a spell cast dead"
    );
    // Its time in a fight: it lives; the fight over, it dies at once.
    let summon = three[1].0;
    let out = map
        .query(move |m| {
            crate::game::map::combat::engage(m, g, summon);
            let now = m.now;
            if let Some(s) = event_ai::scripted_mut(m, summon) {
                s.despawn_at = Some(now);
            }
            event_ai::update_despawns(m);
            let fighting = m.creature(summon).map(|c| c.alive());
            crate::game::map::combat::leave_combat(m, summon);
            crate::game::map::combat::leave_combat(m, g);
            fighting
        })
        .await
        .unwrap();
    assert_eq!(out, Some(true), "alive through its fight");
    let mut died = false;
    for _ in 0..100 {
        died = map
            .query(move |m| m.creature(summon).is_none_or(|c| !c.alive()))
            .await
            .unwrap();
        if died {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(died, "dead once it is over");
}

/// Thistle Tea's `Guardians.prepare` "direct casts toggle only matching
/// guardians" and "creature casters stop adding an entry once sixteen are
/// active", `Guardians.dismiss_all` "departure activates the retained
/// item cooldown" and "an old guardian cannot activate a newer cast",
/// `Guardians.on_death` "lethal damage releases the summoning cast once",
/// and `SpellEffect.receive` "executes once on its caster with count":
/// a player's guardian cast again sends those of its entry away and
/// summons none, her other guardian and her companion kept; a triggered
/// cast adds to them; one with a time and a category replaces them, three
/// for a count of three. A guardian whose spell's cooldown waits on it
/// (an engineer's Mechanical Dragonling) keeps the spell from a second
/// cast while it lives; its death starts the cooldown, once, its cast's
/// own time (mangos's `SendCooldownEvent`), and its fellow's going later
/// starts nothing more. A creature's guardians of one entry stop at
/// sixteen, but another entry's come.
#[tokio::test]
async fn a_guardian_recast_toggles_its_own_and_its_death_starts_its_cooldown() {
    use crate::game::map::spells::{self, Targets};
    const A: u32 = 990_770;
    const B: u32 = 990_771;
    const C: u32 = 990_772;
    const CRITTER: u32 = 990_773;
    const GUARD_A: u32 = 990_774;
    const GUARD_B: u32 = 990_775;
    const THREE: u32 = 990_776;
    const DRAGONLING: u32 = 990_777;
    const CALL: u32 = 990_778;
    const CASTER: u32 = 990_779;
    let mut d = revive_data();
    for (entry, name) in [
        (A, "Guardian A"),
        (B, "Guardian B"),
        (C, "Dragonling"),
        (CRITTER, "Critter"),
        (CASTER, "Caster"),
    ] {
        d.creature_templates
            .insert(entry, tests_pets::template(entry, name, 14, 1));
    }
    let none = SpellEffect::default();
    let guard = |id: u32, entry: u32, count: i32| {
        let mut e = tests_pets::effect(pets::EFFECT_SUMMON_GUARDIAN, count - 1, 32);
        e.misc_value = entry as i32;
        tests_pets::spell(id, "Guardian", [e, none, none])
    };
    let mut three = guard(THREE, A, 3);
    three.duration_index = 4;
    three.category = 1;
    let mut dragonling = guard(DRAGONLING, C, 2);
    dragonling.attributes[0] |= 0x200_0000;
    dragonling.recovery_time = 60_000;
    dragonling.duration_index = 4;
    let mut call = tests_pets::effect(pets::EFFECT_SUMMON_CRITTER, 0, 32);
    call.misc_value = CRITTER as i32;
    for s in [
        guard(GUARD_A, A, 1),
        guard(GUARD_B, B, 1),
        three,
        dragonling,
        tests_pets::spell(CALL, "Call", [call, none, none]),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    let realm = crate::game::harness::realm_with(d);
    let (mut c, g, map) = enter(&realm, "C", "Tinker", Race::Gnome, Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.players
                .get_mut(&g)
                .unwrap()
                .spells
                .extend([GUARD_A, GUARD_B, THREE, DRAGONLING, CALL]);
            let of = |m: &crate::game::map::Map, entry: u32| -> Vec<Guid> {
                m.players[&g]
                    .owned
                    .guardians
                    .iter()
                    .copied()
                    .filter(|x| m.creature(*x).is_some_and(|c| c.entry == entry))
                    .collect()
            };
            let direct = |m: &mut crate::game::map::Map, spell: u32| {
                spells::cast(m, g, spell, Targets::default(), None);
            };
            direct(m, CALL);
            direct(m, GUARD_A);
            direct(m, GUARD_B);
            let first = (of(m, A).len(), of(m, B).len());
            direct(m, GUARD_A);
            let toggled = (
                of(m, A).len(),
                of(m, B).len(),
                m.players[&g].owned.critter.is_some(),
            );
            spells::cast_triggered(m, g, GUARD_A, Some(g));
            spells::cast_triggered(m, g, GUARD_A, Some(g));
            let triggered = of(m, A);
            direct(m, THREE);
            let replaced = of(m, A);
            let timed = replaced
                .iter()
                .filter(|x| pets::state(m, **x).is_some_and(|s| s.expires.is_some()))
                .count();
            let kept = triggered.iter().filter(|x| replaced.contains(x)).count();
            (
                first,
                toggled,
                triggered.len(),
                replaced.len(),
                timed,
                kept,
                of(m, B).len(),
            )
        })
        .await
        .unwrap();
    assert_eq!(out.0, (1, 1), "one of each");
    assert_eq!(out.1, (0, 1, true), "a recast sends its own away, no other");
    assert_eq!(out.2, 2, "triggered casts add");
    assert_eq!(
        (out.3, out.4, out.5),
        (3, 3, 0),
        "a timed one of a category replaces, three"
    );
    assert_eq!(out.6, 1, "the other kept");
    // The Dragonling: its cooldown waits on it.
    c.clear();
    let (two, again, pending) = map
        .query(move |m| {
            spells::cast(m, g, DRAGONLING, Targets::default(), None);
            let two: Vec<Guid> = m.players[&g]
                .owned
                .guardians
                .iter()
                .copied()
                .filter(|x| m.creature(*x).is_some_and(|c| c.entry == C))
                .collect();
            let before = two.len();
            spells::cast(m, g, DRAGONLING, Targets::default(), None);
            let after = m.players[&g]
                .owned
                .guardians
                .iter()
                .filter(|x| m.creature(**x).is_some_and(|c| c.entry == C))
                .count();
            let pending = m.caster(g).unwrap().pending_cooldowns.contains(&DRAGONLING);
            (two, (before, after), pending)
        })
        .await
        .unwrap();
    assert_eq!(again, (2, 2), "two, and no second cast while they live");
    assert!(pending, "its cooldown waits");
    let wolf = two[0];
    let cooled = map
        .query(move |m| {
            crate::game::map::combat::kill(m, g, wolf);
            let left = m
                .caster(g)
                .unwrap()
                .cooldowns
                .get(&DRAGONLING)
                .map(|t| t.duration_since(m.now));
            let pending = m.caster(g).unwrap().pending_cooldowns.contains(&DRAGONLING);
            pets::dismiss(m, two[1]);
            let still = m
                .caster(g)
                .unwrap()
                .cooldowns
                .get(&DRAGONLING)
                .map(|t| t.duration_since(m.now));
            (left, pending, still == left)
        })
        .await
        .unwrap();
    assert_eq!(
        cooled.0,
        Some(Duration::from_secs(60)),
        "a minute from its death"
    );
    assert!(!cooled.1 && cooled.2, "once, not again with the other");
    c.wait_for("SMSG_COOLDOWN_EVENT", |m| match m {
        ServerOpcodeMessage::SMSG_COOLDOWN_EVENT(e) if e.id == DRAGONLING => Some(()),
        _ => None,
    })
    .await;
    // A creature's guardians: sixteen of an entry.
    let caster = place(&map, g, CASTER, 0x70_0030, 8.0).await;
    let counts = map
        .query(move |m| {
            for _ in 0..20 {
                spells::cast_triggered(m, caster, GUARD_A, Some(caster));
            }
            spells::cast_triggered(m, caster, GUARD_B, Some(caster));
            let of = |entry: u32| {
                m.creatures
                    .iter()
                    .filter(|(_, c)| {
                        c.entry == entry
                            && c.scripted.as_deref().and_then(|s| s.summoner) == Some(caster)
                    })
                    .count()
            };
            (of(A), of(B))
        })
        .await
        .unwrap();
    assert_eq!(counts, (16, 1), "sixteen of one entry, and another's");
}

/// Thistle Tea's `PlayerPossession`: "possession stops casting and attacks
/// and grants an empty spell list", "for_possession exposes attack and no
/// spells", "validate checks the rank's level cap and refuses existing
/// possession", "self possession does not acquire control", "replacing
/// controllers preserves the original faction", "release cannot restore
/// movement while fear remains" and "lethal damage releases control": a
/// priest's Mind Control is refused at herself and at a player above its
/// level; taken, it stops the victim's swing, and the priest's bar is
/// Attack and nothing else, no spells. A second priest's is refused, the
/// victim held still by the first. Feared as it ends, the victim has no
/// control back until the fear ends. Held again, killed, the victim is let
/// go, their own side again (mangos-classic's `CheckCast` for
/// `SPELL_AURA_MOD_POSSESS`, `InitPossessCreateSpells`,
/// `UpdateClientControl`).
#[tokio::test]
async fn mind_control_is_refused_where_it_should_be_and_holds_a_player_with_attack_alone() {
    use crate::game::map::auras;
    use crate::game::tests_pets2::{MIND_CONTROL, bring, control_update, flag_pvp, more};
    const FEAR: u32 = 990_780;
    let mut d = more();
    let none = SpellEffect::default();
    let mut fear = tests_pets::effect(6, 0, 6);
    fear.aura = auras::AURA_MOD_FEAR;
    let mut f = tests_pets::spell(FEAR, "Fear", [fear, none, none]);
    f.duration_index = 4;
    d.spells.spells.insert(FEAR, f);
    // Mind Control of its first rank: humans up to level 32.
    let mc = d.spells.spells.get_mut(&MIND_CONTROL).unwrap();
    mc.effects[0].base_points = 31;
    let realm = crate::game::harness::realm_with(d);
    let (mut a, ga, map) = enter(&realm, "A", "Priest", Race::Human, Class::Priest).await;
    let (mut b, gb, _) = enter(&realm, "B", "Undying", Race::Undead, Class::Warrior).await;
    let (mut c, gc, _) = enter(&realm, "C", "Other", Race::Undead, Class::Priest).await;
    learn(&map, ga, &[MIND_CONTROL]);
    learn(&map, gc, &[MIND_CONTROL]);
    bring(&map, gb, ga, 8.0).await;
    bring(&map, gc, ga, 3.0).await;
    a.sees(gb).await;
    c.sees(gb).await;
    flag_pvp(&map, vec![ga, gb, gc]);
    let before = map
        .query(move |m| m.players[&gb].fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE))
        .await
        .unwrap();
    cast(&a, MIND_CONTROL, Some(ga)).await;
    assert_eq!(
        cast_result(&mut a, MIND_CONTROL).await,
        Some(SpellCastResult::BadTargets.as_int()),
        "not herself"
    );
    map.query(move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&gb).unwrap();
        crate::game::map::stats::set_level(p, &data, 33);
    })
    .await;
    cast(&a, MIND_CONTROL, Some(gb)).await;
    assert_eq!(
        cast_result(&mut a, MIND_CONTROL).await,
        Some(SpellCastResult::Highlevel.as_int()),
        "not above 32"
    );
    let wolf = place(&map, gb, tests_pets::WOLF, 0x70_0040, 3.0).await;
    map.query(move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&gb).unwrap();
        crate::game::map::stats::set_level(p, &data, 32);
        crate::game::map::combat::start_attack(m, gb, wolf);
    })
    .await;
    a.clear();
    cast(&a, MIND_CONTROL, Some(gb)).await;
    assert!(!control_update(&mut b, gb).await, "the victim lets go");
    let bar = a
        .wait_for("the possess bar", |m| match m {
            ServerOpcodeMessage::SMSG_PET_SPELLS(s) if s.pet == gb => s.action_bars.clone(),
            _ => None,
        })
        .await;
    let mut want = [pets::button(0, pets::ACT_PASSIVE); 10];
    want[0] = pets::button(u32::from(pets::COMMAND_ATTACK), pets::ACT_COMMAND);
    assert_eq!(bar.action_bars, want, "Attack and nothing else");
    assert!(bar.spells.is_empty(), "no spells");
    let swinging = map
        .query(move |m| m.combat(gb).is_some_and(|c| c.attacking))
        .await
        .unwrap();
    assert!(!swinging, "the victim's swing stopped");
    cast(&c, MIND_CONTROL, Some(gb)).await;
    assert_eq!(
        cast_result(&mut c, MIND_CONTROL).await,
        Some(SpellCastResult::Charmed.as_int()),
        "one at a time"
    );
    let held = map
        .query(move |m| m.players[&gb].owned.possessed_by.map(|p| p.master))
        .await
        .unwrap();
    assert_eq!(held, Some(ga), "the first holds");
    // Feared, then let go: no control back until the fear ends.
    b.clear();
    map.query(move |m| {
        let data = m.realm.data.clone();
        let f = data.spells.get(FEAR).unwrap();
        let _ = auras::apply_spell(m, wolf, gb, f, &[(0, 0)], Default::default());
        auras::remove_spell(m, gb, MIND_CONTROL, None);
    })
    .await;
    let freed = b
        .gets(Duration::from_millis(300), |m| match m {
            ServerOpcodeMessage::SMSG_CLIENT_CONTROL_UPDATE(u)
                if u.guid == gb && u.allow_movement =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    assert!(freed.is_none(), "no control back under the fear");
    map.call(move |m| auras::remove_spell(m, gb, FEAR, None));
    assert!(control_update(&mut b, gb).await, "back as the fear ends");
    // Held again, and killed: let go, their own side.
    a.clear();
    cast(&a, MIND_CONTROL, Some(gb)).await;
    assert!(control_update(&mut a, gb).await, "held again");
    let after = map
        .query(move |m| {
            crate::game::map::combat::kill(m, wolf, gb);
            crate::game::map::pets::update(m);
            let p = &m.players[&gb];
            (
                p.owned.possessed_by.is_some(),
                p.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
                m.players[&ga].owned.charm,
            )
        })
        .await
        .unwrap();
    assert_eq!(after, (false, before, None), "let go at death");
}

/// Thistle Tea's `PlayerPossession.sync` "self possession does not
/// acquire control": a possession's aura a player puts on themselves
/// holds no one.
#[tokio::test]
async fn no_one_possesses_themselves() {
    use crate::game::map::auras;
    use crate::game::tests_pets2::{MIND_CONTROL, more};
    let realm = crate::game::harness::realm_with(more());
    let (_a, ga, map) = enter(&realm, "A", "Priest", Race::Human, Class::Priest).await;
    let held = map
        .query(move |m| {
            let s = m.realm.data.spells.get(MIND_CONTROL).cloned().unwrap();
            let _ = auras::apply_spell(m, ga, ga, &s, &[(0, 1)], Default::default());
            let p = &m.players[&ga];
            (p.owned.possessed_by.is_some(), p.owned.charm)
        })
        .await
        .unwrap();
    assert_eq!(held, (false, None));
}

/// Thistle Tea's `Input.allowed?` "suspends gameplay input while keeping
/// chat, queries, and acknowledgements available", and
/// `Companion.possession_guid` "ordinary charms leave the player's own
/// movement authority intact": a mind-controlled player's own client
/// starts no swing and no cast, but their words are heard; and a
/// warlock's enslaved demon is no unit her client may move.
#[tokio::test]
async fn a_held_player_plays_nothing_but_talks_and_a_charm_moves_no_one() {
    use crate::game::tests_pets2::{MIND_CONTROL, bring, control_update, flag_pvp, more};
    let realm = crate::game::harness::realm_with(more());
    let (mut a, ga, map) = enter(&realm, "A", "Priest", Race::Human, Class::Priest).await;
    let (mut b, gb, _) = enter(&realm, "B", "Undying", Race::Undead, Class::Warrior).await;
    learn(&map, ga, &[MIND_CONTROL]);
    learn(&map, gb, &[tests_pets::FIREBOLT]);
    bring(&map, gb, ga, 4.0).await;
    a.sees(gb).await;
    b.sees(ga).await;
    flag_pvp(&map, vec![ga, gb]);
    cast(&a, MIND_CONTROL, Some(gb)).await;
    assert!(!control_update(&mut b, gb).await, "the victim lets go");
    let wolf = place(&map, gb, tests_pets::WOLF, 0x70_0051, 3.0).await;
    b.sees(wolf).await;
    b.clear();
    b.send(CMSG_ATTACKSWING { guid: wolf }).await;
    cast(&b, tests_pets::FIREBOLT, Some(ga)).await;
    b.say("help").await;
    a.wait_for("the victim's words", |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c) if c.message == "help" => Some(()),
        _ => None,
    })
    .await;
    let (swinging, casting) = map
        .query(move |m| {
            (
                m.combat(gb).is_some_and(|c| c.attacking),
                m.caster(gb).is_some_and(|c| c.casting.is_some()),
            )
        })
        .await
        .unwrap();
    assert!(!swinging && !casting, "no swing, no cast of their own");
    let went = b
        .gets(Duration::from_millis(100), |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_START(s) if s.spell == tests_pets::FIREBOLT => Some(()),
            _ => None,
        })
        .await;
    assert!(went.is_none(), "the cast never started");
    // An enslaved demon: hers, but her client moves her alone.
    let (w, gw, wmap) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&wmap, gw, &[ENSLAVE_DEMON]);
    let demon = place(&wmap, gw, FELHUNTER, 0x70_0050, 5.0).await;
    cast(&w, ENSLAVE_DEMON, Some(demon)).await;
    for _ in 0..200 {
        if wmap
            .query(move |m| m.players[&gw].owned.charm == Some(demon))
            .await
            .unwrap()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    w.send(CMSG_SET_ACTIVE_MOVER { guid: demon }).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mover = wmap
        .query(move |m| {
            let p = &m.players[&gw];
            (p.owned.charm, p.owned.mover)
        })
        .await
        .unwrap();
    assert_eq!(mover, (Some(demon), None), "enslaved, moving no one");
}

/// Thistle Tea's `PlayerCharm`: "spells/1 chooses the highest learned
/// harmful rank and excludes unsuitable abilities", "casts a learned
/// ability at the controller's enemy", "out of mana casters approach for
/// melee", "never attacks another unit charmed by the same controller",
/// "stuns suspend the charm AI", "fear expiry cannot restore control while
/// charm remains", and `Charm.maintain` "controller death, combat end ...
/// release the charm": a mage a wolf charms has, of her spells, only her
/// Fireball's second rank to cast (not its first, nor one of no class,
/// a passive, a hidden one, one the AI mustn't cast, one a blow breaks,
/// or a heal), and casts it at the wolf's foe; stunned she does nothing;
/// out of mana she closes to strike it. Another the wolf charms is no foe
/// of hers. A fear on her ending gives her no control while the charm
/// holds; the wolf's fight over, the charm ends and she has it.
#[tokio::test]
async fn a_creature_charmed_mage_casts_her_best_at_its_foe_or_strikes_it() {
    use crate::game::data::spells::SpellRank;
    use crate::game::map::auras;
    use crate::game::tests_pets2::{bring, control_update, more};
    const CHARM: u32 = 9060;
    const FB1: u32 = 990_790;
    const FB2: u32 = 990_791;
    const PLAIN: u32 = 990_792;
    const PASSIVE: u32 = 990_793;
    const HIDDEN: u32 = 990_794;
    const NO_AI: u32 = 990_795;
    const BREAKS: u32 = 990_796;
    const HEAL: u32 = 990_797;
    const STUN: u32 = 990_798;
    const FEAR: u32 = 990_799;
    let mut d = more();
    let none = SpellEffect::default();
    let bolt = |id: u32, level: u32| {
        let mut s = tests_pets::spell(id, "Fireball", [tests_pets::effect(2, 9, 6), none, none]);
        s.family_name = 3;
        s.spell_level = level;
        s.mana_cost = 10;
        s.school = 2;
        s
    };
    let mut plain = bolt(PLAIN, 1);
    plain.family_name = 0;
    plain.name = "Shoot".into();
    let mut passive = bolt(PASSIVE, 1);
    passive.attributes[0] |= 0x40;
    passive.name = "Passive".into();
    let mut hidden = bolt(HIDDEN, 1);
    hidden.attributes[0] |= crate::game::data::spells::ATTR_HIDDEN;
    hidden.name = "Hidden".into();
    let mut no_ai = bolt(NO_AI, 1);
    no_ai.attributes[1] |= 0x2_0000;
    no_ai.name = "Manual".into();
    let mut breaks = bolt(BREAKS, 1);
    breaks.aura_interrupt_flags = auras::INTERRUPT_DAMAGE;
    breaks.name = "Polymorph".into();
    let mut heal = bolt(HEAL, 1);
    heal.effects[0] = tests_pets::effect(10, 9, 21);
    heal.name = "Heal".into();
    let aura_spell = |id: u32, aura: u32| {
        let mut e = tests_pets::effect(6, 0, 6);
        e.aura = aura;
        let mut s = tests_pets::spell(id, "Hold", [e, none, none]);
        s.duration_index = 4;
        s
    };
    for s in [
        bolt(FB1, 1),
        bolt(FB2, 10),
        plain,
        passive,
        hidden,
        no_ai,
        breaks,
        heal,
        aura_spell(STUN, auras::AURA_MOD_STUN),
        aura_spell(FEAR, auras::AURA_MOD_FEAR),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d.spells.ranks.insert(
        FB2,
        SpellRank {
            prev: FB1,
            first: FB1,
            rank: 2,
            req: 0,
        },
    );
    d.spells.ranks.insert(
        FB1,
        SpellRank {
            first: FB1,
            rank: 1,
            ..Default::default()
        },
    );
    let realm = crate::game::harness::realm_with(d);
    let (mut a, ga, map) = enter(&realm, "A", "Mage", Race::Human, Class::Mage).await;
    let (_b, gb, _) = enter(&realm, "B", "Foe", Race::Human, Class::Warrior).await;
    let (_c, gc, _) = enter(&realm, "C", "Fellow", Race::Human, Class::Warrior).await;
    bring(&map, gb, ga, 15.0).await;
    bring(&map, gc, ga, 4.0).await;
    let wolf = place(&map, ga, tests_pets::WOLF, 0x70_0060, 6.0).await;
    let spells = map
        .query(move |m| {
            m.players
                .get_mut(&ga)
                .unwrap()
                .spells
                .extend([FB1, FB2, PLAIN, PASSIVE, HIDDEN, NO_AI, BREAKS, HEAL]);
            crate::game::map::pets::possess::charm_spells(m, ga)
        })
        .await
        .unwrap();
    assert_eq!(spells, vec![FB2], "only her best harmful rank");
    // The wolf fights her friend B, and charms her and C.
    map.query(move |m| {
        crate::game::map::ai::attacked(m, wolf, gb);
        crate::game::map::spells::cast_triggered(m, wolf, CHARM, Some(gc));
        crate::game::map::spells::cast_triggered(m, wolf, CHARM, Some(ga));
        crate::game::map::ai::add_threat(m, wolf, gc, 1000.0);
        let p = m.players.get_mut(&ga).unwrap();
        p.fields.set_u32(UNIT_FIELD_MAXPOWER1, 1000);
        p.fields.set_u32(UNIT_FIELD_POWER1, 1000);
    })
    .await;
    assert!(!control_update(&mut a, ga).await, "charmed: no control");
    a.wait_for("her Fireball at the wolf's foe", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_START(s) if s.caster == ga && s.spell == FB2 => Some(()),
        _ => None,
    })
    .await;
    let on_fellow = map
        .query(move |m| m.combat(ga).is_some_and(|c| c.attacking && c.victim == gc))
        .await
        .unwrap();
    assert!(!on_fellow, "not on another of the wolf's");
    // Stunned: nothing.
    map.query(move |m| {
        crate::game::map::spells::interrupt(m, ga);
        let s = m.realm.data.spells.get(STUN).cloned().unwrap();
        let _ = auras::apply_spell(m, gb, ga, &s, &[(0, 0)], Default::default());
    })
    .await;
    a.clear();
    let cast_stunned = a
        .gets(Duration::from_millis(1700), |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_START(s) if s.caster == ga => Some(()),
            _ => None,
        })
        .await;
    assert!(cast_stunned.is_none(), "stunned, she does nothing");
    // No mana: she closes to strike.
    map.query(move |m| {
        auras::remove_spell(m, ga, STUN, None);
        m.players
            .get_mut(&ga)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_POWER1, 0);
    })
    .await;
    a.wait_for("her swing at the foe", |m| match m {
        ServerOpcodeMessage::SMSG_ATTACKSTART(x) if x.attacker == ga && x.victim == gb => Some(()),
        _ => None,
    })
    .await;
    // A fear ending under the charm gives her nothing back.
    a.clear();
    map.query(move |m| {
        let s = m.realm.data.spells.get(FEAR).cloned().unwrap();
        let _ = auras::apply_spell(m, gb, ga, &s, &[(0, 0)], Default::default());
        auras::remove_spell(m, ga, FEAR, None);
    })
    .await;
    let back = a
        .gets(Duration::from_millis(200), |m| match m {
            ServerOpcodeMessage::SMSG_CLIENT_CONTROL_UPDATE(u)
                if u.guid == ga && u.allow_movement =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    assert!(back.is_none(), "no control while the charm holds");
    // The wolf's fight over: the charm ends.
    map.query(move |m| {
        crate::game::map::combat::leave_combat(m, wolf);
        crate::game::map::ai::forget(m, gb);
    })
    .await;
    assert!(control_update(&mut a, ga).await, "hers again");
}
