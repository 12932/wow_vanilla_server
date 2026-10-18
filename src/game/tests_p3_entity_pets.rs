//! Phase 3's pet tests, ported from Thistle Tea's
//! `test/game/world/entity/mob/pet_*` and `control_release_test.exs`
//! (docs/parity/matrix.csv): a pet's commands and casts are its master's
//! alone, and a controlled creature let go.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::Map;
use crate::game::map::auras::{AURA_MOD_RESISTANCE, AURA_PERIODIC_DAMAGE};
use crate::game::map::pets;
use crate::game::tests_p3_entity::{NONE_WITHIN, gets_frame, on};
use crate::game::tests_pets::{
    SUMMON_IMP, WOLF, cast, data, effect, enter, learn, pet_bar, place, spell,
};

/// A helpful aura on the imp, and a harmful one.
const ARMOR: u32 = 98_100;
const POISON: u32 = 98_101;
/// The imp's: helpful, three seconds to cast, 20 mana, five seconds'
/// cooldown; and a passive.
const MEND: u32 = 98_102;
const SKIN: u32 = 98_103;

const SMSG_PET_ACTION_FEEDBACK: u16 = 0x2C6;
const SMSG_PET_CAST_FAILED: u16 = 0x138;
const SMSG_CLEAR_COOLDOWN: u16 = 0x1DE;

fn world() -> crate::game::data::GameData {
    let mut d = data();
    let mut armor = spell(
        ARMOR,
        "Armor",
        [effect(6, 29, 1), effect(0, 0, 0), effect(0, 0, 0)],
    );
    armor.effects[0].aura = AURA_MOD_RESISTANCE;
    armor.effects[0].misc_value = 1;
    let mut poison = spell(
        POISON,
        "Poison",
        [effect(6, 3, 6), effect(0, 0, 0), effect(0, 0, 0)],
    );
    poison.effects[0].aura = AURA_PERIODIC_DAMAGE;
    poison.effects[0].amplitude = 60_000;
    poison.attributes[0] = crate::game::data::spells::ATTR_DEBUFF;
    let mut mend = spell(
        MEND,
        "Mend",
        [effect(3, 0, 1), effect(0, 0, 0), effect(0, 0, 0)],
    );
    mend.casting_time_index = 14;
    mend.mana_cost = 20;
    mend.recovery_time = 5_000;
    let mut skin = spell(
        SKIN,
        "Skin",
        [effect(6, 0, 1), effect(0, 0, 0), effect(0, 0, 0)],
    );
    skin.attributes[0] = crate::game::data::spells::ATTR_PASSIVE;
    d.spells.cast_times.insert(14, 3000);
    for s in [armor, poison, mend, skin] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

/// A warlock, Wanda, with her imp out; a stranger, Bob, beside her.
async fn warlock() -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Guid,
    crate::game::map::MapHandle,
    Guid,
    Client,
) {
    let realm = harness::realm_with(world());
    let (mut w, gw, map) = enter(&realm, "W", "Wanda", Race::Human, Class::Warlock).await;
    learn(&map, gw, &[SUMMON_IMP]);
    cast(&w, SUMMON_IMP, None).await;
    let imp = pet_bar(&mut w).await;
    let (b, _gb, _) = enter(&realm, "B", "Bob", Race::Human, Class::Warrior).await;
    (realm, w, gw, map, imp, b)
}

/// What the map has done with the packets `c` sent before.
async fn settled(c: &mut Client) {
    c.send(CMSG_QUERY_TIME {}).await;
    c.wait_for("SMSG_QUERY_TIME_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_QUERY_TIME_RESPONSE(_)).then_some(())
    })
    .await;
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    crate::game::map::auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

/// Thistle Tea's `PetCommands.stop_attack` and `cancel_aura`: only the
/// pet's master stops its attack or takes a buff off it; a harmful aura
/// isn't cancelled, and cancelling one leaves the rest; and a dead pet's
/// master is told it's dead.
#[tokio::test]
async fn a_pets_stop_and_cancelled_buffs_are_its_masters_alone() {
    let (_realm, mut w, gw, map, imp, mut b) = warlock().await;
    let wolf = place(&map, gw, WOLF, 0x70_0005, 10.0).await;
    w.send(CMSG_PET_ACTION {
        pet: imp,
        data: pets::button(u32::from(pets::COMMAND_ATTACK), pets::ACT_COMMAND),
        target: wolf,
    })
    .await;
    crate::game::tests_p3_entity_objects::until(&map, "the imp attacks", move |m| {
        m.creature(imp).is_some_and(|c| c.combat.attacking)
    })
    .await;
    b.send(CMSG_PET_STOP_ATTACK { pet: imp }).await;
    settled(&mut b).await;
    let attacking = move |m: &mut Map| m.creature(imp).is_some_and(|c| c.combat.attacking);
    assert!(on(&map, attacking).await, "Bob's word is nothing to it");
    w.send(CMSG_PET_STOP_ATTACK { pet: imp }).await;
    settled(&mut w).await;
    assert!(!on(&map, attacking).await, "Wanda's stops it");
    // Her armor and a poison on it.
    on(&map, move |m| {
        for (s, by) in [(ARMOR, gw), (POISON, wolf)] {
            let t = m.realm.data.spells.get(s).cloned().unwrap();
            crate::game::map::auras::apply(m, by, imp, &t, 0, 30);
        }
    })
    .await;
    b.send(CMSG_PET_CANCEL_AURA {
        guid: imp,
        id: ARMOR,
    })
    .await;
    settled(&mut b).await;
    for id in [POISON, 999] {
        w.send(CMSG_PET_CANCEL_AURA { guid: imp, id }).await;
    }
    settled(&mut w).await;
    let kept = on(&map, move |m| (has(m, imp, ARMOR), has(m, imp, POISON))).await;
    assert_eq!(
        kept,
        (true, true),
        "not by Bob, not a poison, not one it hasn't"
    );
    w.send(CMSG_PET_CANCEL_AURA {
        guid: imp,
        id: ARMOR,
    })
    .await;
    settled(&mut w).await;
    let kept = on(&map, move |m| (has(m, imp, ARMOR), has(m, imp, POISON))).await;
    assert_eq!(kept, (false, true), "the armor off, the poison left");
    // Dead, it can't: Wanda hears why.
    on(&map, move |m| {
        crate::game::map::combat::deal_damage(m, wolf, imp, 1_000_000)
    })
    .await;
    w.clear();
    w.send(CMSG_PET_CANCEL_AURA {
        guid: imp,
        id: ARMOR,
    })
    .await;
    let dead = w
        .wait_frame("SMSG_PET_ACTION_FEEDBACK", SMSG_PET_ACTION_FEEDBACK, |_| {
            true
        })
        .await;
    assert_eq!(dead, vec![1], "PET_DEAD");
}

/// Thistle Tea's `PetCasting.cast`: a pet casts at its master's word only,
/// a spell it knows and not a passive; a second cast while one is under
/// way is refused as in progress and its button let go; one it has no
/// mana for is refused, nothing spent; and one cooling down is refused
/// with its cooldown kept.
#[tokio::test]
async fn a_pets_cast_is_its_masters_and_refused_as_it_should_be() {
    let (_realm, mut w, _gw, map, imp, mut b) = warlock().await;
    on(&map, move |m| {
        for s in [MEND, SKIN] {
            pets::state_mut(m, imp).unwrap().spells.push(s);
        }
        let f = &mut m.creature_mut(imp).unwrap().fields;
        f.set_u32(UNIT_FIELD_MAXPOWER1, 100);
        f.set_u32(UNIT_FIELD_POWER1, 100);
    })
    .await;
    let order = |id| CMSG_PET_CAST_SPELL {
        guid: imp,
        id,
        targets: crate::game::tests_pets::at(imp),
    };
    let casting = move |m: &mut Map| {
        m.caster(imp)
            .and_then(|c| c.casting.as_ref())
            .map(|c| c.spell)
    };
    // Bob's word, a spell it doesn't know, a passive: nothing.
    b.send(order(MEND)).await;
    settled(&mut b).await;
    w.send(order(999)).await;
    w.send(order(SKIN)).await;
    settled(&mut w).await;
    assert_eq!(on(&map, casting).await, None, "nothing cast");
    // Wanda's: it casts, and a second is in progress.
    w.clear();
    w.send(order(MEND)).await;
    settled(&mut w).await;
    assert_eq!(on(&map, casting).await, Some(MEND));
    w.send(order(MEND)).await;
    let failed = w
        .wait_frame("SMSG_PET_CAST_FAILED", SMSG_PET_CAST_FAILED, |_| true)
        .await;
    assert_eq!(
        failed,
        [&MEND.to_le_bytes()[..], &[2, 0x61]].concat(),
        "in progress"
    );
    let cleared = w
        .wait_frame("SMSG_CLEAR_COOLDOWN", SMSG_CLEAR_COOLDOWN, |_| true)
        .await;
    assert_eq!(
        cleared[4..12],
        imp.guid().to_le_bytes(),
        "its button let go"
    );
    // No mana: refused, nothing spent.
    on(&map, move |m| {
        crate::game::map::spells::interrupt(m, imp);
        m.creature_mut(imp)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_POWER1, 5);
    })
    .await;
    w.clear();
    w.send(order(MEND)).await;
    let failed = w
        .wait_frame("SMSG_PET_CAST_FAILED", SMSG_PET_CAST_FAILED, |_| true)
        .await;
    assert_eq!(failed[4..], [2, 0x4D], "no power");
    let mana = on(&map, move |m| {
        m.creature(imp).unwrap().fields.get_u32(UNIT_FIELD_POWER1)
    })
    .await;
    assert_eq!(mana, 5, "nothing spent");
    // Cooling down: refused, the cooldown kept.
    on(&map, move |m| {
        let now = m.now;
        m.creature_mut(imp)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_POWER1, 100);
        m.caster_mut(imp)
            .unwrap()
            .cooldowns
            .insert(MEND, now + Duration::from_secs(5));
    })
    .await;
    w.clear();
    w.send(order(MEND)).await;
    let failed = w
        .wait_frame("SMSG_PET_CAST_FAILED", SMSG_PET_CAST_FAILED, |_| true)
        .await;
    assert_eq!(failed[4..], [2, 0x3C], "not ready");
    assert!(
        !gets_frame(&mut w, SMSG_CLEAR_COOLDOWN, |_| true).await,
        "its cooldown stays on the bar"
    );
}

/// Thistle Tea's `release_control` and `turn_on_controller`: a mind
/// controlled creature let go by its controller is itself again, alive and
/// whole, not gone; another's word lets nothing go; and one friendly to
/// its controller, freed, stays calm.
#[tokio::test]
async fn a_controlled_creature_let_go_is_itself_again() {
    let (_realm, _w, gw, map, _imp, b) = warlock().await;
    let gb = b.guid;
    let out = on(&map, move |m| {
        let wolf = crate::game::tests_p3_combat::spawn(m, gw, WOLF, 1, 4.0);
        let health = m.creature(wolf).unwrap().fields.get_u32(UNIT_FIELD_HEALTH);
        pets::charmed(m, gw, wolf, true);
        pets::uncharmed(m, gb, wolf);
        let held = pets::state(m, wolf).is_some_and(|s| s.owner == gw);
        pets::uncharmed(m, gw, wolf);
        let c = m.creature(wolf);
        (
            held,
            c.is_some_and(|c| c.alive() && c.pet.is_none() && c.owner.is_none()),
            c.map(|c| c.fields.get_u32(UNIT_FIELD_HEALTH)) == Some(health),
        )
    })
    .await;
    assert!(out.0, "Bob can't let it go");
    assert!(out.1, "Wanda's release frees it, alive");
    assert!(out.2, "whole");
    // A creature of her own side, freed, doesn't turn on her.
    let calm = on(&map, move |m| {
        let friend =
            crate::game::tests_p3_combat::spawn(m, gw, crate::game::tests_pets::IMP, 1, 4.0);
        let ours = m.players[&gw].fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE);
        m.creature_mut(friend)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_FACTIONTEMPLATE, ours);
        pets::charmed(m, gw, friend, false);
        pets::uncharmed(m, gw, friend);
        let c = m.creature(friend).unwrap();
        c.threat.is_empty() && !c.combat.attacking
    })
    .await;
    assert!(calm, "a friend stays calm");
    let _ = NONE_WITHIN;
}

/// A sap: a stun that damage breaks.
const SAP: u32 = 98_104;

fn victim_of(m: &Map, g: Guid) -> Option<Guid> {
    m.combat(g).filter(|c| c.attacking).map(|c| c.victim)
}

/// Thistle Tea's `PetTargeting`: a pet struck fights back, and keeps a
/// living victim when another strikes it or its master; its victim dead,
/// it takes the next of its master's foes; it leaves one held by crowd
/// control that damage would break until it is free; and told to stay, it
/// takes on of its own accord only what is within its reach, though a
/// blow that lands nothing still sets it on one in reach.
#[tokio::test]
async fn a_pet_picks_its_fights_as_its_master_and_its_stay_allow() {
    let mut d = world();
    let mut sap = spell(
        SAP,
        "Sap",
        [effect(6, 0, 6), effect(0, 0, 0), effect(0, 0, 0)],
    );
    sap.effects[0].aura = crate::game::map::auras::AURA_MOD_STUN;
    sap.aura_interrupt_flags = crate::game::map::auras::INTERRUPT_DAMAGE;
    sap.duration_index = 0;
    d.spells.spells.insert(SAP, sap);
    let realm = harness::realm_with(d);
    let (mut w, gw, map) = enter(&realm, "W", "Wanda", Race::Human, Class::Warlock).await;
    learn(&map, gw, &[SUMMON_IMP]);
    cast(&w, SUMMON_IMP, None).await;
    let imp = pet_bar(&mut w).await;
    let wolves: Vec<Guid> = {
        let mut v = Vec::new();
        for (n, yards) in [(1u32, 3.0f32), (2, 3.5), (3, 4.0), (4, 4.5), (5, 5.0)] {
            v.push(place(&map, gw, WOLF, 0x70_0100 + n, yards).await);
        }
        v
    };
    let (a, b, c, dd, e) = (wolves[0], wolves[1], wolves[2], wolves[3], wolves[4]);
    // A strikes the imp: it fights back; B's blow keeps it on A.
    on(&map, move |m| {
        for wolf in [a, b, c, dd, e] {
            if let Some(x) = m.creature_mut(wolf) {
                x.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
                x.fields.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
            }
        }
        crate::game::map::combat::engage(m, a, imp);
    })
    .await;
    crate::game::tests_p3_entity_objects::until(&map, "the imp on A", move |m| {
        victim_of(m, imp) == Some(a)
    })
    .await;
    on(&map, move |m| crate::game::map::combat::engage(m, b, imp)).await;
    // C goes for Wanda: the imp stays on A.
    on(&map, move |m| crate::game::map::ai::attacked(m, c, gw)).await;
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    assert_eq!(
        on(&map, move |m| victim_of(m, imp)).await,
        Some(a),
        "kept on A"
    );
    // A and B dead: the imp takes C, its master's foe.
    on(&map, move |m| {
        for wolf in [a, b] {
            crate::game::map::combat::deal_damage(m, gw, wolf, 10_000_000);
        }
    })
    .await;
    crate::game::tests_p3_entity_objects::until(&map, "the imp on C", move |m| {
        victim_of(m, imp) == Some(c)
    })
    .await;
    // C sapped: let be; free, taken again.
    on(&map, move |m| {
        crate::game::map::combat::stop_attack(m, imp);
        let sap = m.realm.data.spells.get(SAP).cloned().unwrap();
        crate::game::map::auras::apply(m, gw, c, &sap, 0, 1);
        // Still in Wanda's fight, as a sapped foe is.
        crate::game::map::combat::engage(m, c, gw);
    })
    .await;
    for _ in 0..12 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (victim, sapped) = on(&map, move |m| {
            (
                victim_of(m, imp),
                crate::game::map::auras::auras(m, c).is_some_and(|a| a.has_spell(SAP)),
            )
        })
        .await;
        assert_eq!(victim, None, "the sapped is let be");
        assert!(sapped, "and stays sapped");
    }
    on(&map, move |m| {
        crate::game::map::auras::remove_spell(m, c, SAP, None)
    })
    .await;
    crate::game::tests_p3_entity_objects::until(&map, "the imp on C again", move |m| {
        victim_of(m, imp) == Some(c)
    })
    .await;
    // Told to stay where it is, beside Wanda; C dies.
    w.send(CMSG_PET_ACTION {
        pet: imp,
        data: pets::button(u32::from(pets::COMMAND_STAY), pets::ACT_COMMAND),
        target: Guid::zero(),
    })
    .await;
    settled(&mut w).await;
    let far = on(&map, move |m| {
        crate::game::map::combat::deal_damage(m, gw, c, 10_000_000);
        // The imp stays fifteen yards from Wanda; D goes for her.
        let at = m.players[&gw].position();
        let there = Vector3d {
            x: at.x + 15.0,
            ..at
        };
        m.creature_mut(imp).unwrap().position = there;
        pets::state_mut(m, imp).unwrap().stay_at = Some(there);
        crate::game::map::ai::attacked(m, dd, gw);
        dd
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    assert_eq!(
        on(&map, move |m| victim_of(m, imp)).await,
        None,
        "staying, it lets the far one be"
    );
    // E, in its reach, touches it for nothing: it takes E on.
    on(&map, move |m| {
        let at = m.position_of(imp).unwrap();
        m.creature_mut(e).unwrap().position = Vector3d {
            x: at.x + 1.0,
            ..at
        };
        crate::game::map::combat::engage(m, e, imp);
    })
    .await;
    crate::game::tests_p3_entity_objects::until(&map, "the imp on E", move |m| {
        victim_of(m, imp) == Some(e)
    })
    .await;
    let _ = far;
}
