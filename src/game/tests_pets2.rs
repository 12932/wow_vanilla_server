//! Pets, round two: pets in PvP (and their honor), Mind Control on a
//! player and the client's active mover, the Eye of Kilrogg's end, a pet's
//! cooldowns over a logout, its size, what it inherits of its master's,
//! totems' places and immunities, guardians, Beast Lore, a creature's
//! charm on a player, and the stacking trinkets.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;
use crate::game::map::pets::{self, ACT_COMMAND, COMMAND_ATTACK, Kind};
use crate::game::tests_pets::*;

const EYE: u32 = 4277;
const EYE_OF_KILROGG: u32 = 126;
pub(crate) const MIND_CONTROL: u32 = 605;
const IMPROVED_IMP: u32 = 18694;
const MASTER_DEMONOLOGIST: u32 = 23785;
const MASTER_DEMONOLOGIST_IMP: u32 = 23759;
const UNLEASHED_FURY: u32 = 19616;
const GUARDIAN: u32 = 3030;
const SUMMON_GUARDIAN: u32 = 9050;
const BEAST_LORE: u32 = 1462;
const CHARM_PLAYER: u32 = 9060;
const BURN: u32 = 9070;
const FIRE: u32 = 9071;

pub(super) fn more() -> GameData {
    let mut d = data();
    let none = SpellEffect::default();
    let mut eye = template(EYE, "Eye of Kilrogg", 35, 1);
    eye.creature_type = 3;
    d.creature_templates.insert(EYE, eye);
    d.creature_templates
        .insert(GUARDIAN, template(GUARDIAN, "Dragonling", 35, 1));
    d.pets.families.get_mut(&5).unwrap().min_scale = 0.6;
    d.pets.families.get_mut(&5).unwrap().max_scale = 1.0;
    d.pets.families.get_mut(&5).unwrap().min_scale_level = 1;
    d.pets.families.get_mut(&5).unwrap().max_scale_level = 60;
    d.pets
        .owner_auras
        .push((MASTER_DEMONOLOGIST, IMP, MASTER_DEMONOLOGIST_IMP));
    let s = &mut d.spells;
    s.durations.insert(5, tests_duration(400));
    // Eye of Kilrogg: a channel that summons the eye to possess.
    let mut summon_eye = effect(73, 0, 32);
    summon_eye.misc_value = EYE as i32;
    let mut eye_spell = spell(EYE_OF_KILROGG, "Eye of Kilrogg", [summon_eye, none, none]);
    eye_spell.attributes[1] = 0x4;
    eye_spell.duration_index = 5;
    // Mind Control: a channel whose aura possesses.
    let mut possess = effect(6, 0, 6);
    possess.aura = 2;
    let mut mc = spell(MIND_CONTROL, "Mind Control", [possess, none, none]);
    mc.attributes[1] = 0x4;
    mc.duration_index = 4;
    // A warlock's talent that changes the imp's spells, and one whose
    // aura gives the imp another.
    let mut modifier = effect(6, 9, 1);
    modifier.aura = 108;
    modifier.item_type = 0x1000;
    let mut improved = spell(IMPROVED_IMP, "Improved Imp", [modifier, none, none]);
    improved.attributes[0] = crate::game::data::spells::ATTR_PASSIVE;
    improved.family_name = 5;
    let mut dummy = effect(6, 0, 1);
    dummy.aura = 4;
    let mut demonologist = spell(
        MASTER_DEMONOLOGIST,
        "Master Demonologist",
        [dummy, none, none],
    );
    demonologist.attributes[0] = crate::game::data::spells::ATTR_PASSIVE;
    let mut threat = effect(6, -21, 1);
    threat.aura = 10;
    threat.misc_value = 127;
    let imp_aura = spell(
        MASTER_DEMONOLOGIST_IMP,
        "Master Demonologist",
        [threat, none, none],
    );
    // A hunter's talent aimed at the pet.
    let mut fury_effect = effect(119, 3, 1);
    fury_effect.aura = 79;
    fury_effect.misc_value = 1;
    let mut fury = spell(UNLEASHED_FURY, "Unleashed Fury", [fury_effect, none, none]);
    fury.attributes[0] = crate::game::data::spells::ATTR_PASSIVE;
    let mut summon_guardian = effect(42, 0, 32);
    summon_guardian.misc_value = GUARDIAN as i32;
    let mut guardian = spell(
        SUMMON_GUARDIAN,
        "Summon Dragonling",
        [summon_guardian, none, none],
    );
    guardian.duration_index = 4;
    // Timed and of a category, a recast replaces it (mangos-classic's
    // `EffectSummonGuardian`; without, a recast only sends it away).
    guardian.category = 4;
    let mut lore_effect = effect(6, 0, 6);
    lore_effect.aura = 121;
    let mut lore = spell(BEAST_LORE, "Beast Lore", [lore_effect, none, none]);
    lore.duration_index = 4;
    let mut charm_effect = effect(6, 0, 6);
    charm_effect.aura = 6;
    let mut charm = spell(CHARM_PLAYER, "Dominate Mind", [charm_effect, none, none]);
    charm.duration_index = 4;
    let mut dot_effect = effect(6, 3, 6);
    dot_effect.aura = 3;
    dot_effect.amplitude = 1000;
    let mut burn = spell(BURN, "Burn", [dot_effect, none, none]);
    burn.duration_index = 4;
    let mut fire = spell(FIRE, "Fire", [effect(2, 9, 6), none, none]);
    fire.school = 2;
    for t in [
        eye_spell,
        mc,
        improved,
        demonologist,
        imp_aura,
        fury,
        guardian,
        lore,
        charm,
        burn,
        fire,
    ] {
        s.spells.insert(t.id, t);
    }
    // Firebolt has a cooldown, to keep over a logout.
    s.spells.get_mut(&FIREBOLT).unwrap().recovery_time = 60_000;
    d
}

fn tests_duration(ms: i32) -> crate::game::data::spells::SpellDuration {
    crate::game::data::spells::SpellDuration {
        base: ms,
        ..Default::default()
    }
}

/// Bring `who` next to `to` in their map, `d` yards before them.
pub(crate) async fn bring(map: &MapHandle, who: Guid, to: Guid, d: f32) {
    map.query(move |m| {
        let p = &m.players[&to];
        let (pos, o) = (p.position(), p.movement.orientation);
        let at = Vector3d {
            x: pos.x + d * o.cos(),
            y: pos.y + d * o.sin(),
            z: pos.z,
        };
        crate::game::map::movement::teleport_within(m, who, at, o + std::f32::consts::PI);
        crate::game::map::movement::teleport_ack(m, who);
    })
    .await;
}

pub(crate) fn flag_pvp(map: &MapHandle, gs: Vec<Guid>) {
    map.call(move |m| {
        for g in &gs {
            crate::game::map::pvp::toggle(m, *g, Some(true));
        }
    });
}

pub(crate) async fn control_update(c: &mut Client, guid: Guid) -> bool {
    c.wait_for("SMSG_CLIENT_CONTROL_UPDATE", |m| match m {
        ServerOpcodeMessage::SMSG_CLIENT_CONTROL_UPDATE(u) if u.guid == guid => {
            Some(u.allow_movement)
        }
        _ => None,
    })
    .await
}

async fn position(map: &MapHandle, g: Guid) -> Vector3d {
    map.query(move |m| m.position_of(g))
        .await
        .flatten()
        .unwrap()
}

/// Wait until `check` holds on the map.
async fn until(
    map: &MapHandle,
    what: &str,
    check: impl Fn(&crate::game::map::Map) -> bool + Send + Clone + 'static,
) {
    for _ in 0..150 {
        let c = check.clone();
        if map.query(move |m| c(m)).await.unwrap() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("never: {what}");
}

#[tokio::test]
async fn a_pet_fights_a_flagged_enemy_and_its_kill_is_honor_for_its_master() {
    let realm = harness::realm_with(more());
    let (mut h, gh, map, pet, _wolf) = {
        let (r, h, gh, map, pet, wolf) = tamed_on(realm.clone()).await;
        let _ = r;
        (h, gh, map, pet, wolf)
    };
    let (mut u, gu, _) = enter(&realm, "U", "Undying", Race::Undead, Class::Warrior).await;
    bring(&map, gu, gh, 6.0).await;
    h.sees(gu).await;
    // Unflagged, the enemy can't be set on.
    h.send(CMSG_PET_ACTION {
        pet,
        data: pets::button(u32::from(COMMAND_ATTACK), ACT_COMMAND),
        target: gu,
    })
    .await;
    h.wait_for("SMSG_PET_ACTION_FEEDBACK", |m| match m {
        ServerOpcodeMessage::SMSG_PET_ACTION_FEEDBACK(f) => {
            (f.feedback == PetFeedback::CantAttackTarget).then_some(())
        }
        _ => None,
    })
    .await;
    flag_pvp(&map, vec![gh, gu]);
    map.query(move |m| {
        if let Some(p) = m.players.get_mut(&gu) {
            p.fields.set_u32(UNIT_FIELD_HEALTH, 3);
        }
    })
    .await;
    h.send(CMSG_PET_ACTION {
        pet,
        data: pets::button(u32::from(COMMAND_ATTACK), ACT_COMMAND),
        target: gu,
    })
    .await;
    h.wait_for("the pet's SMSG_ATTACKSTART on the enemy", |m| match m {
        ServerOpcodeMessage::SMSG_ATTACKSTART(a) if a.attacker == pet && a.victim == gu => Some(()),
        _ => None,
    })
    .await;
    // A chase and a swing or two (it may miss).
    let mut dead = false;
    for _ in 0..300 {
        dead = map
            .query(move |m| m.players[&gu].death != crate::game::map::death::Death::Alive)
            .await
            .unwrap();
        if dead {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(dead, "the pet killed the enemy");
    // The kill is the hunter's honor.
    h.wait_frame(
        "SMSG_PVP_CREDIT",
        crate::game::map::pvp::SMSG_PVP_CREDIT,
        |_| true,
    )
    .await;
    let (pvp, dead) = map
        .query(move |m| {
            (
                m.creature(pet)
                    .unwrap()
                    .fields
                    .has_flag(UNIT_FIELD_FLAGS, crate::game::map::player::UNIT_FLAG_PVP),
                m.players[&gu].death != crate::game::map::death::Death::Alive,
            )
        })
        .await
        .unwrap();
    assert!(dead);
    assert!(pvp, "the pet carries its master's PvP flag");
    let _ = u.guid;
    u.clear();
}

/// A hunter with a tamed boar on `realm`.
pub(crate) async fn tamed_on(
    realm: std::sync::Arc<crate::game::Realm>,
) -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Guid,
    MapHandle,
    Guid,
    Guid,
) {
    let (mut h, gh, map) = enter(&realm, "H", "Hunter", Race::Dwarf, Class::Hunter).await;
    learn(
        &map,
        gh,
        &[
            TAME_BEAST,
            CALL_PET,
            DISMISS_PET,
            UNLEASHED_FURY,
            BEAST_LORE,
        ],
    );
    let boar = place(&map, gh, BOAR, 0x70_0101, 4.0).await;
    let wolf = place(&map, gh, WOLF, 0x70_0102, 40.0).await;
    h.sees(boar).await;
    cast(&h, TAME_BEAST, Some(boar)).await;
    let pet = pet_bar(&mut h).await;
    assert!(!pet.is_zero());
    map.query(move |m| {
        crate::game::map::combat::leave_combat(m, gh);
        if let Some(p) = m.players.get_mut(&gh) {
            p.combat.attackers.clear();
        }
    })
    .await;
    (realm, h, gh, map, pet, wolf)
}

#[tokio::test]
async fn mind_control_moves_a_player_only_once_made_the_active_mover_and_ends_cleanly() {
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
    assert_eq!(pet_bar(&mut a).await, gb, "the possess bar");
    let (faction, mine) = map
        .query(move |m| {
            (
                m.players[&gb].fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
                m.players[&ga].fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
            )
        })
        .await
        .unwrap();
    assert_eq!(faction, mine, "on the priest's side");
    // Until the client makes the victim its active mover, its movement is
    // the priest's own.
    let a_at = position(&map, ga).await;
    let b_at = position(&map, gb).await;
    a.heartbeat(a_at.x + 1.0, a_at.y, a_at.z).await;
    until(&map, "the priest moved", move |m| {
        (m.players[&ga].position().x - (a_at.x + 1.0)).abs() < 0.01
    })
    .await;
    assert_eq!(position(&map, gb).await.x, b_at.x);
    a.send(CMSG_SET_ACTIVE_MOVER { guid: gb }).await;
    a.heartbeat(b_at.x + 2.0, b_at.y, b_at.z).await;
    until(&map, "the victim moved", move |m| {
        (m.players[&gb].position().x - (b_at.x + 2.0)).abs() < 0.01
    })
    .await;
    // The victim's own client moves nothing.
    b.heartbeat(b_at.x - 5.0, b_at.y, b_at.z).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!((position(&map, gb).await.x - (b_at.x + 2.0)).abs() < 0.01);
    // The aura ends: each has their own again.
    map.call(move |m| crate::game::map::auras::remove_spell(m, gb, MIND_CONTROL, None));
    assert!(
        control_update(&mut b, gb).await,
        "the victim has control again"
    );
    assert!(control_update(&mut a, ga).await, "the priest has their own");
    let (faction, mover, charm) = map
        .query(move |m| {
            (
                m.players[&gb].fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
                m.players[&ga].owned.mover,
                m.players[&ga].owned.charm,
            )
        })
        .await
        .unwrap();
    assert_eq!((faction, mover, charm), (before, None, None));
}

/// Review 5's combat finding 1: whoever struck a player's pet went on its
/// threat list, which nothing prunes, and so stayed in their fight while
/// both lived. A pet keeps no list (mangos's `CanHaveThreatList`): the
/// blow's PvP timer run out, the striker is out of it; and a pet fighting
/// one who vanishes stops, as the creatures on their list do.
#[tokio::test]
async fn whoever_strikes_a_pet_leaves_the_fight_once_its_timer_runs_out() {
    use crate::game::map::combat;
    use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
    let realm = harness::realm_with(more());
    let (realm, _h, gh, map, pet, _wolf) = tamed_on(realm).await;
    let (_b, gb, _) = enter(&realm, "B", "Undying", Race::Undead, Class::Warrior).await;
    bring(&map, gb, gh, 8.0).await;
    flag_pvp(&map, vec![gh, gb]);
    let fighting = |m: &crate::game::map::Map, g: Guid| {
        m.unit_fields(g)
            .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT))
    };
    let r = map
        .query(move |m| {
            // A blow, as a swing deals it.
            combat::engage(m, gb, pet);
            combat::deal_damage(m, gb, pet, 1);
            combat::enter(m, gb, true);
            let struck = fighting(m, gb);
            // The pet strikes no one back (a passive one), and the PvP
            // timer runs out.
            combat::stop_attack(m, pet);
            let listed = m.creature(pet).map(|c| c.threat.len());
            let now = m.now;
            let c = m.combat_mut(gb).unwrap();
            c.combat_until = Some(now);
            c.next_check = Some(now);
            combat::update_players(m);
            (struck, listed, fighting(m, gb))
        })
        .await
        .unwrap();
    assert_eq!(r, (true, Some(0), false), "out once nothing holds them");
    let attacking = map
        .query(move |m| {
            pets::attack_target(m, pet, gb);
            let before = m.combat(pet).is_some_and(|c| c.attacking);
            crate::game::map::ai::forget(m, gb);
            (before, m.combat(pet).is_some_and(|c| c.attacking))
        })
        .await
        .unwrap();
    assert_eq!(attacking, (true, false), "Vanish or Feign Death stops it");
}

/// The first movement a client sends of `unit` that is told to `to`.
async fn first_heartbeat_of(to: &mut Client, unit: Guid) -> Vector3d {
    to.wait_for("a heartbeat", |m| match m {
        ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(h) if h.guid == unit => Some(h.info.position),
        _ => None,
    })
    .await
}

/// Review 5's world finding 2: the controller's client put a possessed
/// unit anywhere, NaN too, told to everyone who saw it and saved, where
/// its own movement is checked. A place out of the world is dropped.
#[tokio::test]
async fn a_possessed_player_isnt_moved_out_of_the_world() {
    let realm = harness::realm_with(more());
    let (mut a, ga, map) = enter(&realm, "A", "Priest", Race::Human, Class::Priest).await;
    let (mut b, gb, _) = enter(&realm, "B", "Undying", Race::Undead, Class::Warrior).await;
    learn(&map, ga, &[MIND_CONTROL]);
    bring(&map, gb, ga, 8.0).await;
    a.sees(gb).await;
    flag_pvp(&map, vec![ga, gb]);
    cast(&a, MIND_CONTROL, Some(gb)).await;
    assert!(control_update(&mut a, gb).await, "the priest takes hold");
    a.send(CMSG_SET_ACTIVE_MOVER { guid: gb }).await;
    let at = position(&map, gb).await;
    b.clear();
    a.heartbeat(f32::NAN, at.y, at.z).await;
    a.heartbeat(at.x - 25_000.0, at.y, at.z).await;
    a.heartbeat(at.x + 2.0, at.y, at.z).await;
    // The victim's own client hears each move taken, in order.
    let told = first_heartbeat_of(&mut b, gb).await;
    assert_eq!(told.x, at.x + 2.0, "the first move told is the good one");
    until(&map, "the victim moved", move |m| {
        (m.players[&gb].position().x - (at.x + 2.0)).abs() < 0.01
    })
    .await;
}

#[tokio::test]
async fn the_eye_of_kilrogg_isnt_moved_out_of_the_world() {
    let realm = harness::realm_with(more());
    let (mut w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&map, gw, &[EYE_OF_KILROGG]);
    cast(&w, EYE_OF_KILROGG, None).await;
    let eye = w
        .wait_for("control of the eye", |m| match m {
            ServerOpcodeMessage::SMSG_CLIENT_CONTROL_UPDATE(u)
                if u.guid != gw && u.allow_movement =>
            {
                Some(u.guid)
            }
            _ => None,
        })
        .await;
    w.send(CMSG_SET_ACTIVE_MOVER { guid: eye }).await;
    let at = position(&map, eye).await;
    w.heartbeat(at.x, at.y, f32::NAN).await;
    w.heartbeat(at.x, at.y, at.z - 30_000.0).await;
    w.heartbeat(at.x + 3.0, at.y, at.z).await;
    let told = first_heartbeat_of(&mut w, eye).await;
    assert_eq!(told.x, at.x + 3.0, "the first move told is the good one");
}

#[tokio::test]
async fn the_eye_of_kilrogg_moves_for_its_master_and_goes_with_the_channel() {
    let realm = harness::realm_with(more());
    let (mut w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&map, gw, &[EYE_OF_KILROGG]);
    cast(&w, EYE_OF_KILROGG, None).await;
    let eye = w
        .wait_for("control of the eye", |m| match m {
            ServerOpcodeMessage::SMSG_CLIENT_CONTROL_UPDATE(u)
                if u.guid != gw && u.allow_movement =>
            {
                Some(u.guid)
            }
            _ => None,
        })
        .await;
    assert_eq!(guid::entry(eye), EYE);
    w.send(CMSG_SET_ACTIVE_MOVER { guid: eye }).await;
    let at = position(&map, eye).await;
    w.heartbeat(at.x + 3.0, at.y, at.z).await;
    until(&map, "the eye moved", move |m| {
        m.position_of(eye)
            .is_some_and(|p| (p.x - (at.x + 3.0)).abs() < 0.01)
    })
    .await;
    // The channel ends: the eye goes, and the warlock has their own back.
    assert!(control_update(&mut w, gw).await);
    until(&map, "the eye gone", move |m| {
        m.creature(eye).is_none() && m.players[&gw].owned.mover.is_none()
    })
    .await;
}

#[tokio::test]
async fn a_pets_cooldowns_last_over_a_logout() {
    let realm = harness::realm_with(more());
    let (mut w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&map, gw, &[SUMMON_IMP]);
    cast(&w, SUMMON_IMP, None).await;
    let imp = pet_bar(&mut w).await;
    map.query(move |m| {
        let now = m.now;
        if let Some(c) = m.caster_mut(imp) {
            c.cooldowns.insert(FIREBOLT, now + Duration::from_secs(60));
        }
    })
    .await;
    w.send(CMSG_LOGOUT_REQUEST {}).await;
    w.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    w.clear();
    w.login(gw).await;
    let back = pet_bar(&mut w).await;
    assert!(!back.is_zero());
    let left = map
        .query(move |m| {
            let now = m.now;
            m.caster(back)
                .and_then(|c| c.cooldowns.get(&FIREBOLT))
                .map(|t| t.saturating_duration_since(now))
        })
        .await
        .flatten();
    assert!(
        left.is_some_and(|l| l > Duration::from_secs(50)),
        "{left:?}"
    );
}

#[tokio::test]
async fn a_pet_has_its_masters_talents_and_its_familys_size() {
    let realm = harness::realm_with(more());
    let (mut w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&map, gw, &[SUMMON_IMP, IMPROVED_IMP, MASTER_DEMONOLOGIST]);
    map.query(move |m| {
        crate::game::map::spells::apply_passive(m, gw, IMPROVED_IMP);
        crate::game::map::spells::apply_passive(m, gw, MASTER_DEMONOLOGIST);
    })
    .await;
    cast(&w, SUMMON_IMP, None).await;
    let imp = pet_bar(&mut w).await;
    let inherited = move |m: &crate::game::map::Map, spell: u32| {
        crate::game::map::auras::auras(m, imp).is_some_and(|a| {
            a.holders
                .iter()
                .any(|h| h.spell == spell && h.caster == gw && h.item == Some(gw))
        })
    };
    until(&map, "Improved Imp on the imp", move |m| {
        inherited(m, IMPROVED_IMP)
    })
    .await;
    until(&map, "Master Demonologist's aura on the imp", move |m| {
        inherited(m, MASTER_DEMONOLOGIST_IMP)
    })
    .await;
    // The talent's aura off the master: off the imp.
    map.call(move |m| crate::game::map::auras::remove_spell(m, gw, MASTER_DEMONOLOGIST, None));
    until(&map, "Master Demonologist's aura gone", move |m| {
        !inherited(m, MASTER_DEMONOLOGIST_IMP)
    })
    .await;
    // A hunter's pet has the talent aimed at it, and is the size of its
    // family at its level.
    let (_realm, _h, gh, hmap, pet, _wolf) = tamed_on(realm.clone()).await;
    until(&hmap, "Unleashed Fury on the pet", move |m| {
        crate::game::map::auras::auras(m, pet).is_some_and(|a| {
            a.holders
                .iter()
                .any(|h| h.spell == UNLEASHED_FURY && h.caster == gh)
        })
    })
    .await;
    let scale = hmap
        .query(move |m| {
            m.creature(pet)
                .unwrap()
                .fields
                .get_f32(OBJECT_FIELD_SCALE_X)
        })
        .await
        .unwrap();
    assert!((scale - 0.6).abs() < 0.01, "{scale}");
}

#[tokio::test]
async fn totems_stand_by_element_and_shrug_off_harm() {
    let realm = harness::realm_with(more());
    let (s, gs, map) = enter(&realm, "S", "Shaman", Race::Orc, Class::Shaman).await;
    learn(&map, gs, &[STRENGTH_OF_EARTH_TOTEM]);
    cast(&s, STRENGTH_OF_EARTH_TOTEM, None).await;
    until(&map, "an earth totem", move |m| {
        m.players[&gs].owned.totems[1].is_some()
    })
    .await;
    let (totem, at, me, o) = map
        .query(move |m| {
            let t = m.players[&gs].owned.totems[1].unwrap();
            let p = &m.players[&gs];
            (
                t,
                m.position_of(t).unwrap(),
                p.position(),
                p.movement.orientation,
            )
        })
        .await
        .unwrap();
    let a = o + std::f32::consts::FRAC_PI_4 - std::f32::consts::FRAC_PI_2;
    assert!(
        (at.x - (me.x + 2.0 * a.cos())).abs() < 0.01
            && (at.y - (me.y + 2.0 * a.sin())).abs() < 0.01
    );
    assert_eq!(
        map.query(move |m| pets::state(m, totem).map(|s| s.kind))
            .await
            .unwrap(),
        Some(Kind::Totem)
    );
    // A burn from an enemy doesn't take.
    let wolf = place(&map, gs, WOLF, 0x70_0201, 10.0).await;
    let burned = map
        .query(move |m| {
            crate::game::map::spells::cast_triggered(m, wolf, BURN, Some(totem));
            crate::game::map::auras::auras(m, totem).is_some_and(|a| a.has_spell(BURN))
        })
        .await
        .unwrap();
    assert!(!burned, "a totem shrugs off a burn");
}

#[tokio::test]
async fn a_guardian_recast_replaces_the_last_and_beast_lore_shows_its_target() {
    let realm = harness::realm_with(more());
    let (h, gh, map) = enter(&realm, "H", "Hunter", Race::Dwarf, Class::Hunter).await;
    learn(&map, gh, &[SUMMON_GUARDIAN, BEAST_LORE]);
    cast(&h, SUMMON_GUARDIAN, None).await;
    until(&map, "a guardian", move |m| {
        m.players[&gh].owned.guardians.len() == 1
    })
    .await;
    let first = map
        .query(move |m| m.players[&gh].owned.guardians[0])
        .await
        .unwrap();
    map.call(move |m| {
        if let Some(c) = m.caster_mut(gh) {
            c.gcd_until = None;
        }
    });
    cast(&h, SUMMON_GUARDIAN, None).await;
    until(&map, "the second in its place", move |m| {
        m.creature(first).is_none() && m.players[&gh].owned.guardians.len() == 1
    })
    .await;
    let boar = place(&map, gh, BOAR, 0x70_0301, 5.0).await;
    map.call(move |m| {
        if let Some(c) = m.caster_mut(gh) {
            c.gcd_until = None;
        }
    });
    cast(&h, BEAST_LORE, Some(boar)).await;
    until(&map, "Beast Lore's flag", move |m| {
        m.creature(boar).is_some_and(|c| {
            c.fields
                .has_flag(UNIT_DYNAMIC_FLAGS, pets::DYNFLAG_SPECIALINFO)
        })
    })
    .await;
}

#[tokio::test]
async fn a_creature_charmed_player_turns_on_their_own() {
    let realm = harness::realm_with(more());
    let (mut a, ga, map) = enter(&realm, "A", "Warrior", Race::Human, Class::Warrior).await;
    let (_b, gb, _) = enter(&realm, "B", "Friend", Race::Human, Class::Warrior).await;
    bring(&map, gb, ga, 3.0).await;
    a.sees(gb).await;
    let wolf = place(&map, ga, WOLF, 0x70_0401, 12.0).await;
    map.query(move |m| {
        // The wolf fights the friend; its charm lasts while it does.
        crate::game::map::ai::attacked(m, wolf, gb);
        crate::game::map::spells::cast_triggered(m, wolf, CHARM_PLAYER, Some(ga));
    })
    .await;
    assert!(!control_update(&mut a, ga).await, "charmed: no control");
    a.wait_for(
        "the charmed player's SMSG_ATTACKSTART on their friend",
        |m| match m {
            ServerOpcodeMessage::SMSG_ATTACKSTART(x) if x.attacker == ga && x.victim == gb => {
                Some(())
            }
            _ => None,
        },
    )
    .await;
    map.call(move |m| crate::game::map::auras::remove_spell(m, ga, CHARM_PLAYER, None));
    assert!(control_update(&mut a, ga).await, "theirs again");
}

#[tokio::test]
async fn unstable_power_starts_full_and_spends_a_stack_a_spell() {
    use crate::game::map::trinkets::{UNSTABLE_POWER, UNSTABLE_POWER_BONUS};
    let mut d = more();
    let none = SpellEffect::default();
    let mut dummy = effect(6, 0, 1);
    dummy.aura = 4;
    let mut parent = spell(UNSTABLE_POWER, "Unstable Power", [dummy, none, none]);
    parent.duration_index = 4;
    let mut bonus_effect = effect(6, 16, 1);
    bonus_effect.aura = 13;
    bonus_effect.misc_value = 0x7e;
    let mut bonus: SpellTemplate = spell(
        UNSTABLE_POWER_BONUS,
        "Unstable Power",
        [bonus_effect, none, none],
    );
    bonus.duration_index = 4;
    bonus.stack_amount = 12;
    bonus.proc_flags = crate::game::map::procs::PROC_DONE_NEGATIVE_SPELL;
    bonus.proc_chance = 100;
    d.spells.spells.insert(parent.id, parent);
    d.spells.spells.insert(bonus.id, bonus);
    let realm = harness::realm_with(d);
    let (_m, gm, map) = enter(&realm, "M", "Mage", Race::Human, Class::Mage).await;
    let stacks = move |m: &crate::game::map::Map| {
        crate::game::map::auras::auras(m, gm).and_then(|a| {
            a.holders
                .iter()
                .find(|h| h.spell == UNSTABLE_POWER_BONUS)
                .map(|h| h.stacks)
        })
    };
    let full = map
        .query(move |m| {
            crate::game::map::spells::cast_triggered(m, gm, UNSTABLE_POWER, Some(gm));
            stacks(m)
        })
        .await
        .unwrap();
    assert_eq!(full, Some(12), "full at once");
    let wolf = place(&map, gm, WOLF, 0x70_0501, 10.0).await;
    let after = map
        .query(move |m| {
            crate::game::map::procs::trigger(
                m,
                crate::game::map::procs::Event {
                    actor: gm,
                    target: wolf,
                    actor_flags: crate::game::map::procs::PROC_DONE_NEGATIVE_SPELL,
                    ex: crate::game::map::procs::EX_NORMAL_HIT,
                    spell: Some(FIRE),
                    school: 4,
                    damage: 10,
                    ..Default::default()
                },
            );
            stacks(m)
        })
        .await
        .unwrap();
    assert_eq!(after, Some(11), "a stack a spell");
    let gone = map
        .query(move |m| {
            crate::game::map::auras::remove_spell(m, gm, UNSTABLE_POWER, None);
            stacks(m)
        })
        .await
        .unwrap();
    assert_eq!(gone, None, "the bonus goes with its parent");
}

#[test]
fn a_familys_size_runs_from_its_least_to_its_most() {
    let f = crate::game::data::pets::Family {
        min_scale: 0.6,
        max_scale: 1.0,
        min_scale_level: 1,
        max_scale_level: 60,
        ..Default::default()
    };
    assert_eq!(f.scale(1), Some(0.6));
    assert_eq!(f.scale(60), Some(1.0));
    assert!((f.scale(31).unwrap() - (0.6 + 30.0 / 60.0 * 0.4)).abs() < 0.001);
    assert_eq!(crate::game::data::pets::Family::default().scale(10), None);
}
