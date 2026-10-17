//! Phase 3's combat tests, part four (Thistle Tea's `ai/bt/mob_test.exs`
//! and the aggro rows of `hostility_test.exs`): a creature's fight ending
//! when its victims vanish or leave, its way home, and whom it notices.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::creature::Creature;
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::{Map, ai, auras, combat, visibility};
use crate::game::tests_p3_combat::{self as base, *};

/// A level-5 creature of the pack that notices players ten yards off; one
/// of its kind that notices no one; a neutral one; and a human one.
const WATCHER: u32 = 3710;
const PEER: u32 = 3711;
const BYSTANDER: u32 = 3712;
const PREY: u32 = 3713;
const NEUTRAL: u32 = 7;

fn data() -> GameData {
    let mut d = base::data();
    d.faction_templates.insert(
        NEUTRAL,
        FactionTemplate {
            id: NEUTRAL,
            faction: NEUTRAL,
            ..Default::default()
        },
    );
    for (entry, faction, detection) in [
        (WATCHER, PACK, 10.0),
        (PEER, PACK, 0.1),
        (BYSTANDER, NEUTRAL, 0.1),
        (PREY, HUMANS, 0.1),
    ] {
        let mut t = template(entry, 7);
        t.faction = faction;
        t.detection_range = detection;
        t.level_min = 5;
        t.level_max = 5;
        d.creature_templates.insert(entry, t);
    }
    d
}

/// A creature of `entry` at `level`, at `at` facing `o`, seen by those
/// near.
fn spawn_at(m: &mut Map, entry: u32, level: u8, at: Vector3d, o: f32) -> Guid {
    let data = m.realm.data.clone();
    let t = data
        .creature_template(entry)
        .cloned()
        .expect("the template");
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let mut c = Creature::new(guid::unit(entry, counter | 0x80_0000), &t, level, at, o);
    c.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1000);
    c.fields.set_u32(UNIT_FIELD_HEALTH, 1000);
    let c = c.with_base();
    let g = m.add_creature(c);
    visibility::refresh(m, g);
    g
}

/// `yards` east of the start.
fn east(yards: f32) -> Vector3d {
    Vector3d {
        x: START.0 + yards,
        y: START.1,
        z: START.2,
    }
}

/// `g` fighting `victim`, with `list` for its threat.
fn fighting(m: &mut Map, g: Guid, victim: Guid, list: &[(Guid, f32)]) {
    let now = m.now;
    let c = m.creature_mut(g).unwrap();
    c.combat.victim = victim;
    c.combat.attacking = true;
    c.combat_start = Some(now);
    c.threat = list.to_vec();
    c.fields.set_guid(UNIT_FIELD_TARGET, victim);
    c.fields
        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, true);
}

/// A creature's turn now, its aggro check due.
fn turn(m: &mut Map, g: Guid) {
    let now = m.now;
    if let Some(c) = m.creature_mut(g) {
        c.aggro_check_at = now;
    }
    ai::update_creatures(m);
}

#[tokio::test]
async fn a_vanish_takes_a_creature_out_of_its_fight_and_home_whole_or_to_the_next_on_its_list() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Rogue).await;
    let brute = map
        .query(move |m| {
            let brute = spawn_at(m, BRUTE, 20, east(3.0), 0.0);
            brute
        })
        .await
        .unwrap();
    a.sees(brute).await;
    let r = map
        .query(move |m| {
            // Off its spawn, hurt, and fighting the rogue alone.
            fighting(m, brute, ga, &[(ga, 100.0)]);
            let c = m.creature_mut(brute).unwrap();
            c.position = east(13.0);
            c.fields.set_u32(UNIT_FIELD_HEALTH, 400);
            ai::forget(m, ga);
            let c = m.creature(brute).unwrap();
            let left = (
                c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT),
                c.threat.is_empty(),
                c.fields.get_guid(UNIT_FIELD_TARGET).is_zero(),
                c.combat.attacking,
                c.evading,
                c.fields.get_u32(UNIT_FIELD_HEALTH),
            );
            // With another on its list, it turns to them and fights on.
            let helper = spawn_at(m, HELPER, 20, east(4.0), 0.0);
            let other = spawn_at(m, BRUTE, 20, east(3.0), 0.0);
            fighting(m, other, ga, &[(ga, 100.0), (helper, 50.0)]);
            ai::forget(m, ga);
            let c = m.creature(other).unwrap();
            let next = (
                c.combat.victim == helper,
                c.fields.get_guid(UNIT_FIELD_TARGET) == helper,
                c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT),
                c.threat.iter().all(|(t, _)| *t != ga),
            );
            // One that never knew the rogue is as it was.
            let idle = spawn_at(m, BRUTE, 20, east(5.0), 0.0);
            ai::forget(m, ga);
            let c = m.creature(idle).unwrap();
            let untouched = !c.evading && c.threat.is_empty() && !c.combat.attacking;
            (left, next, untouched)
        })
        .await
        .unwrap();
    assert_eq!(r.0, (false, true, true, false, true, 1000));
    assert_eq!(r.1, (true, true, true, true));
    assert!(r.2);
    a.wait_for("SMSG_ATTACKSTOP", |m| match m {
        wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_ATTACKSTOP(s)
            if s.player == brute =>
        {
            Some(())
        }
        _ => None,
    })
    .await;
}

#[tokio::test]
async fn a_corpse_neither_evades_nor_loses_its_loot_when_its_killer_vanishes() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Rogue).await;
    let r = map
        .query(move |m| {
            let brute = spawn_at(m, BRUTE, 20, east(3.0), 0.0);
            fighting(m, brute, ga, &[(ga, 100.0)]);
            combat::kill(m, ga, brute);
            let flags = m.unit_fields(brute).unwrap().get_u32(UNIT_DYNAMIC_FLAGS);
            // A stale reference on the corpse goes, and nothing else.
            m.creature_mut(brute).unwrap().threat.push((ga, 100.0));
            ai::forget(m, ga);
            let c = m.creature(brute).unwrap();
            (
                c.alive(),
                c.evading,
                c.threat.is_empty(),
                c.fields.get_u32(UNIT_DYNAMIC_FLAGS) == flags,
                c.fields.get_u32(UNIT_FIELD_HEALTH),
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (false, false, true, true, 0));
}

#[tokio::test]
async fn at_home_when_its_last_enemy_goes_a_creature_turns_back_to_its_spawns_facing() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Rogue).await;
    let brute = map
        .query(move |m| spawn_at(m, BRUTE, 20, east(3.0), 2.5))
        .await
        .unwrap();
    a.sees(brute).await;
    let facing = map
        .query(move |m| {
            fighting(m, brute, ga, &[(ga, 100.0)]);
            m.creature_mut(brute).unwrap().orientation = 1.0;
            ai::forget(m, ga);
            ai::update_creatures(m);
            m.creature(brute).unwrap().orientation
        })
        .await
        .unwrap();
    assert_eq!(facing, 2.5);
    a.wait_frame("the turn home", ai::SMSG_MONSTER_MOVE, |b| {
        face_angle(b) == Some(2.5)
    })
    .await;
}

/// The angle of a facing monster move, if it is one.
fn face_angle(b: &[u8]) -> Option<f32> {
    let at = 1 + b[0].count_ones() as usize + 16;
    (b.get(at) == Some(&4)).then(|| f32::from_le_bytes(b[at + 1..at + 5].try_into().unwrap()))
}

#[tokio::test]
async fn a_creature_leaves_its_fight_when_its_victim_leaves_the_world_or_none_is_left() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            // Its only victim gone from the world.
            let brute = spawn_at(m, BRUTE, 20, east(3.0), 0.0);
            let gone = guid::player(9_999);
            fighting(m, brute, gone, &[(gone, 100.0)]);
            ai::update_creatures(m);
            let c = m.creature(brute).unwrap();
            let left = (
                c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT),
                c.threat.is_empty(),
                c.fields.get_guid(UNIT_FIELD_TARGET).is_zero(),
                c.combat.attacking,
            );
            // In a fight with no one at all on its list.
            let other = spawn_at(m, BRUTE, 20, east(4.0), 0.0);
            let now = m.now;
            let c = m.creature_mut(other).unwrap();
            c.fields
                .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, true);
            c.combat_start = Some(now);
            let _ = ga;
            ai::update_creatures(m);
            let alone = m
                .creature(other)
                .unwrap()
                .fields
                .has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT);
            (left, alone)
        })
        .await
        .unwrap();
    assert_eq!(r.0, (false, true, true, false));
    assert!(!r.1);
}

#[tokio::test]
async fn a_creature_notices_only_enemies_and_neither_a_neutral_one_nor_an_unattackable_one_starts_a_fight()
 {
    let (_realm, _a, _ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            // Twenty-five yards off the player, with its own kind and a
            // neutral beside it.
            let watcher = spawn_at(m, WATCHER, 5, east(25.0), 0.0);
            spawn_at(m, PEER, 5, east(28.0), 0.0);
            spawn_at(m, BYSTANDER, 5, east(27.0), 0.0);
            turn(m, watcher);
            let quiet = !m.creature(watcher).unwrap().combat.attacking;
            // Unattackable itself, it starts nothing with an enemy there.
            let prey = spawn_at(m, PREY, 5, east(29.0), 0.0);
            m.unit_fields_mut(watcher)
                .unwrap()
                .set_flag(UNIT_FIELD_FLAGS, 0x2, true);
            turn(m, watcher);
            let held = !m.creature(watcher).unwrap().combat.attacking;
            m.unit_fields_mut(watcher)
                .unwrap()
                .set_flag(UNIT_FIELD_FLAGS, 0x2, false);
            // Nor does the neutral, an enemy beside it.
            let bystander = spawn_at(m, BYSTANDER, 5, east(30.0), 0.0);
            m.creature_mut(bystander).unwrap().aggro_check_at = m.now;
            turn(m, bystander);
            let neutral = !m.creature(bystander).unwrap().combat.attacking;
            // Free of the flag, it goes for the enemy.
            turn(m, watcher);
            let c = m.creature(watcher).unwrap();
            (
                quiet,
                held,
                neutral,
                c.combat.attacking && c.combat.victim == prey,
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (true, true, true, true));
    assert_eq!(ai::aggro_radius(0.9, 10, 10), 0.0, "under a yard: never");
}

#[tokio::test]
async fn a_stealthed_player_goes_unnoticed_unless_a_detection_aura_sees_them_in_front() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Rogue).await;
    let r = map
        .query(move |m| {
            // A level-5 creature five yards off a level-5 rogue in stealth
            // of skill 25, facing them.
            let data = m.realm.data.clone();
            crate::game::map::stats::set_level(m.players.get_mut(&ga).unwrap(), &data, 5);
            let watcher = spawn_at(m, WATCHER, 5, east(5.0), std::f32::consts::PI);
            aura_of(m, ga, 90_100, auras::AURA_MOD_STEALTH, 0, 25);
            turn(m, watcher);
            let hidden = !m.creature(watcher).unwrap().combat.attacking;
            // Thirty more detection: seen in front, not behind.
            aura_of(m, watcher, 90_101, auras::AURA_MOD_STEALTH_DETECT, 0, 30);
            let behind = {
                m.creature_mut(watcher).unwrap().orientation = 0.0;
                crate::game::map::spells::stealth::sees(m, watcher, ga)
            };
            m.creature_mut(watcher).unwrap().orientation = std::f32::consts::PI;
            turn(m, watcher);
            let c = m.creature(watcher).unwrap();
            (hidden, behind, c.combat.attacking && c.combat.victim == ga)
        })
        .await
        .unwrap();
    assert_eq!(r, (true, false, true));
}
