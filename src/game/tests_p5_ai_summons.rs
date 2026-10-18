//! Round 5's summon tests, ported from Thistle Tea's
//! `test/game/core/pet/{totems,guardians,wild_summon,mini_pet}_test.exs`
//! and `ai/bt/{totem,guardian,mini_pet}_test.exs` (docs/parity/matrix.csv):
//! a shaman's totems and wards, their slots and their going, guardians and
//! companions and what ends them.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::map::pets::{self, Kind};
use crate::game::map::MapHandle;
use crate::game::tests_pets::{
    self, STRENGTH_OF_EARTH, STRENGTH_OF_EARTH_TOTEM, WOLF, cast, enter, learn, place,
};

/// A ward: a totem of no element, 1,500 health, a minute long, holding a
/// resistance aura over its shaman's party.
const WARD: u32 = 9950;
const WARD_TOTEM: u32 = 9951;
const WARD_AURA: u32 = 9952;

fn data() -> GameData {
    let mut d = tests_pets::data();
    let none = SpellEffect::default();
    let mut t = tests_pets::template(WARD_TOTEM, "Ward", 35, 1);
    t.creature_type = 11;
    d.creature_templates.insert(WARD_TOTEM, t);
    let mut e = tests_pets::effect(pets::EFFECT_SUMMON_TOTEM, 1_499, 41);
    e.misc_value = WARD_TOTEM as i32;
    let mut ward = tests_pets::spell(WARD, "Ward", [e, none, none]);
    ward.duration_index = 4;
    d.spells.spells.insert(WARD, ward);
    let mut a = tests_pets::effect(35, 9, 1);
    a.aura = 22;
    a.misc_value = 4;
    a.radius_index = 9;
    let mut aura = tests_pets::spell(WARD_AURA, "Ward", [a, none, none]);
    aura.duration_index = 21;
    d.spells.spells.insert(WARD_AURA, aura);
    d.pets.totem_spells.insert(WARD_TOTEM, WARD_AURA);
    d
}

/// What `g` has standing: its earth totem and its wards.
async fn standing(map: &MapHandle, g: Guid) -> (Option<Guid>, Vec<Guid>) {
    map.query(move |m| {
        let o = &m.players[&g].owned;
        (o.totems[1], o.loose_totems.clone())
    })
    .await
    .unwrap()
}

/// Wait for `g` to have `earth` earth totems and `wards` wards.
async fn wait_standing(map: &MapHandle, g: Guid, earth: bool, wards: usize) -> (Option<Guid>, Vec<Guid>) {
    for _ in 0..100 {
        let s = standing(map, g).await;
        if s.0.is_some() == earth && s.1.len() == wards {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("never {earth} and {wards}: {:?}", standing(map, g).await);
}

async fn has_aura(map: &MapHandle, g: Guid, spell: u32) -> bool {
    for _ in 0..60 {
        if map
            .query(move |m| m.players[&g].auras.has_spell(spell))
            .await
            .unwrap()
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// Wards stand beside an element's totem and beside each other, each with
/// the health its spell gives, its spell's time and its creator; one struck
/// down sinks at once, with no corpse to loot or respawn, its aura lifted,
/// the others standing. A shaman's death takes every totem and ward, and
/// so does a logout (Thistle Tea's `Totems.started`, `stopped`,
/// `dismiss_all`, `leave_world`, `handle_continue`; mangos's
/// `Totem::Update` and `UnSummon`).
#[tokio::test]
async fn wards_stand_beside_the_elements_and_go_alone_when_struck_and_all_with_their_shaman() {
    let realm = crate::game::harness::realm_with(data());
    let (s, gs, map) = enter(&realm, "S", "Shaman", Race::Orc, Class::Shaman).await;
    let (mut t, gt, _) = enter(&realm, "T", "Other", Race::Orc, Class::Shaman).await;
    learn(&map, gs, &[STRENGTH_OF_EARTH_TOTEM, WARD]);
    learn(&map, gt, &[STRENGTH_OF_EARTH_TOTEM, WARD]);
    let wolf = place(&map, gs, WOLF, 0x70_0020, 30.0).await;
    cast(&s, STRENGTH_OF_EARTH_TOTEM, None).await;
    wait_standing(&map, gs, true, 0).await;
    cast(&s, WARD, None).await;
    wait_standing(&map, gs, true, 1).await;
    cast(&s, WARD, None).await;
    let (earth, wards) = wait_standing(&map, gs, true, 2).await;
    let earth = earth.unwrap();
    assert!(has_aura(&map, gs, WARD_AURA).await, "the wards' aura");
    assert!(has_aura(&map, gs, STRENGTH_OF_EARTH).await);
    let (ward_now, first) = map
        .query(move |m| {
            let c = m.creature(wards[0]).unwrap();
            let s = c.pet.as_deref().unwrap();
            (
                (
                    c.entry,
                    c.fields.get_u32(UNIT_FIELD_HEALTH),
                    c.fields.get_u32(UNIT_FIELD_MAXHEALTH),
                    c.fields.get_u32(UNIT_CREATED_BY_SPELL),
                    s.kind,
                    s.totem_slot,
                    s.expires.map(|t| t.duration_since(m.now).as_secs()),
                ),
                wards[0],
            )
        })
        .await
        .unwrap();
    assert_eq!(
        ward_now,
        (WARD_TOTEM, 1_500, 1_500, WARD, Kind::Totem, None, Some(59))
    );
    // The first ward struck down.
    map.query(move |m| crate::game::map::combat::kill(m, wolf, first))
        .await;
    let (still_earth, left) = wait_standing(&map, gs, true, 1).await;
    assert_eq!(still_earth, Some(earth), "the earth totem stands");
    assert_ne!(left[0], first);
    let gone = map
        .query(move |m| m.creature(first).is_none())
        .await
        .unwrap();
    assert!(gone, "no corpse to loot, nothing to respawn");
    assert!(has_aura(&map, gs, WARD_AURA).await, "the other ward's aura holds");

    // The other shaman's, at their logout.
    cast(&t, STRENGTH_OF_EARTH_TOTEM, None).await;
    cast(&t, WARD, None).await;
    let (theirs, their_wards) = wait_standing(&map, gt, true, 1).await;
    t.send(CMSG_LOGOUT_REQUEST {}).await;
    t.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    let after_logout = map
        .query(move |m| [theirs.unwrap(), their_wards[0]].map(|g| m.creature(g).is_some()))
        .await
        .unwrap();
    assert_eq!(after_logout, [false, false], "gone at logout");

    // The shaman's death: the rest go.
    let rest = left[0];
    map.query(move |m| crate::game::map::combat::kill(m, wolf, gs))
        .await;
    for _ in 0..100 {
        let up = map
            .query(move |m| [earth, rest].map(|g| m.creature(g).is_some()))
            .await
            .unwrap();
        if up == [false, false] {
            assert_eq!(standing(&map, gs).await, (None, vec![]));
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the shaman's totems outlived them");
}

/// A companion called for a minute, and its entry.
const CALL_TIMED: u32 = 9960;
const TIMED_CRITTER: u32 = 9961;

fn critter_data() -> GameData {
    let mut d = data();
    d.creature_templates.insert(
        TIMED_CRITTER,
        tests_pets::template(TIMED_CRITTER, "Critter", 35, 1),
    );
    let mut e = tests_pets::effect(pets::EFFECT_SUMMON_CRITTER, 0, 1);
    e.misc_value = TIMED_CRITTER as i32;
    let none = SpellEffect::default();
    let mut s = tests_pets::spell(CALL_TIMED, "Call Critter", [e, none, none]);
    s.duration_index = 4;
    d.spells.spells.insert(CALL_TIMED, s);
    d
}

async fn critter_of(map: &MapHandle, g: Guid) -> Option<Guid> {
    for _ in 0..100 {
        if let Some(c) = map.query(move |m| m.players[&g].owned.critter).await.flatten() {
            return Some(c);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    None
}

/// A companion whose spell has a time goes when it is up, the pet staying
/// out; a death takes the companion and the pet both (mangos's
/// `EffectSummonCritter` `SetDuration`, Thistle Tea's `MiniPet` "emits a
/// timed critter request" and "the shared lethal damage transition
/// dismisses both pet slots").
#[tokio::test]
async fn a_timed_companion_goes_when_its_time_is_up_and_a_death_takes_both_pets() {
    let (_realm, mut h, gh, map, pet, wolf) =
        crate::game::tests_p5_ai_pets::tamed_with(critter_data()).await;
    h.sees(pet).await;
    learn(&map, gh, &[CALL_TIMED]);
    cast(&h, CALL_TIMED, None).await;
    let critter = critter_of(&map, gh).await.expect("the companion");
    let (at_59, at_60, pet_out) = map
        .query(move |m| {
            let start = m.now;
            m.now = start + Duration::from_secs(59);
            pets::update(m);
            let at_59 = m.creature(critter).is_some();
            m.now = start + Duration::from_secs(61);
            pets::update(m);
            (
                at_59,
                m.creature(critter).is_some(),
                m.players[&gh].owned.pet == Some(pet),
            )
        })
        .await
        .unwrap();
    assert_eq!((at_59, at_60, pet_out), (true, false, true));
    cast(&h, CALL_TIMED, None).await;
    let critter = critter_of(&map, gh).await.expect("called again");
    map.query(move |m| crate::game::map::combat::kill(m, wolf, gh))
        .await;
    for _ in 0..100 {
        let left = map
            .query(move |m| (m.creature(critter).is_some(), m.players[&gh].owned.pet.is_some()))
            .await
            .unwrap();
        if left == (false, false) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("a pet outlived its hunter's death");
}

/// A ward's time runs out whether it is fighting or not, where a
/// guardian's waits for its fight (Thistle Tea's `TotemBT.lifetime`
/// "expires independently of combat", mangos's `Totem::Update`).
#[tokio::test]
async fn a_ward_goes_at_its_time_even_in_a_fight() {
    let realm = crate::game::harness::realm_with(data());
    let (s, gs, map) = enter(&realm, "S", "Shaman", Race::Orc, Class::Shaman).await;
    learn(&map, gs, &[WARD]);
    cast(&s, WARD, None).await;
    let (_, wards) = wait_standing(&map, gs, false, 1).await;
    let ward = wards[0];
    let gone = map
        .query(move |m| {
            let start = m.now;
            if let Some(c) = m.creature_mut(ward) {
                c.fields.set_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                    true,
                );
            }
            m.now = start + Duration::from_secs(61);
            pets::update(m);
            m.creature(ward).is_none()
        })
        .await
        .unwrap();
    assert!(gone);
}
