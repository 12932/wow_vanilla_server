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
