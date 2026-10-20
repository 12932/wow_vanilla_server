//! Round 9's player: a player another moves, a launch, a timed companion
//! (Thistle Tea's `mover_test`, `possession_owner_test`, `knockback_test`
//! and `mini_pet_owner_test`).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellDuration, SpellEffect};
use crate::game::harness::{self, Client};
use crate::game::map::auras::{self, AURA_MOD_FEAR};
use crate::game::map::{Map, MapHandle, movement};
use crate::game::tests_p3_combat::{BRUTE, START, spawn};
use crate::game::tests_p3_entity::{NONE_WITHIN, aura_effect, on, spell};
use crate::game::tests_pets::{cast, enter, learn};
use crate::game::tests_pets2::{MIND_CONTROL, bring, control_update, flag_pvp, more};

const FEAR: u32 = 97_910;
const KNOCK: u32 = 97_911;
const SLOW: u32 = 97_912;
const COMPANION: u32 = 97_913;
const CRITTER: u32 = 3_970;

fn data() -> crate::game::data::GameData {
    let mut d = more();
    d.spells
        .spells
        .insert(FEAR, spell(FEAR, &[aura_effect(AURA_MOD_FEAR, 1, 0, 6)]));
    d.spells.spells.insert(
        KNOCK,
        spell(
            KNOCK,
            &[SpellEffect {
                effect: crate::game::map::spells::EFFECT_KNOCK_BACK,
                base_points: 69,
                base_dice: 1,
                misc_value: 120,
                target_a: 25,
                ..Default::default()
            }],
        ),
    );
    let mut slow = spell(SLOW, &[aura_effect(auras::AURA_DUMMY, 1, 0, 1)]);
    slow.casting_time_index = 9;
    d.spells.cast_times.insert(9, 10_000);
    d.spells.spells.insert(SLOW, slow);
    // A companion called for two seconds.
    for entry in [CRITTER, BRUTE] {
        d.creature_templates
            .entry(entry)
            .or_insert_with(|| crate::game::data::CreatureTemplate {
                entry,
                name: format!("Creature {entry}"),
                display_ids: [903, 0, 0, 0],
                display_probabilities: [1, 0, 0, 0],
                level_min: 1,
                level_max: 1,
                faction: 14,
                unit_class: 1,
                health_max: 100,
                health_multiplier: 1.0,
                damage_multiplier: 1.0,
                base_attack_time: 2000,
                ..Default::default()
            });
    }
    let mut companion = spell(
        COMPANION,
        &[SpellEffect {
            effect: crate::game::map::pets::EFFECT_SUMMON_CRITTER,
            misc_value: CRITTER as i32,
            target_a: 1,
            ..Default::default()
        }],
    );
    companion.duration_index = 77;
    d.spells.durations.insert(
        77,
        SpellDuration {
            base: 2_000,
            per_level: 0,
            max: 2_000,
        },
    );
    d.spells.spells.insert(COMPANION, companion);
    d
}

/// A priest holding an undead warrior by Mind Control, her client moving
/// him.
async fn held() -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Guid,
    Client,
    Guid,
    MapHandle,
) {
    let realm = harness::realm_with(data());
    let (mut a, ga, map) = enter(&realm, "A", "Priest", Race::Human, Class::Priest).await;
    let (mut b, gb, _) = enter(&realm, "B", "Undying", Race::Undead, Class::Warrior).await;
    learn(&map, ga, &[MIND_CONTROL]);
    bring(&map, gb, ga, 8.0).await;
    a.sees(gb).await;
    flag_pvp(&map, vec![ga, gb]);
    cast(&a, MIND_CONTROL, Some(gb)).await;
    assert!(!control_update(&mut b, gb).await, "the victim lets go");
    assert!(control_update(&mut a, gb).await, "the priest takes hold");
    a.send(CMSG_SET_ACTIVE_MOVER { guid: gb }).await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while on(&map, move |m| m.players[&ga].owned.mover).await != Some(gb) {
        assert!(tokio::time::Instant::now() < deadline, "she moves him");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    (realm, a, ga, b, gb, map)
}

fn still(info: Vector3d) -> MovementInfo {
    MovementInfo {
        flags: MovementInfo_MovementFlags::empty(),
        timestamp: 0,
        position: info,
        orientation: 0.0,
        fall_time: 0,
    }
}

/// Thistle Tea's "rejects a stale guid and a remote mover that remains
/// controlled": the possessor's client saying it no longer moves another
/// unit, or the one it holds still, changes nothing; she moves him until
/// control changes (mangos's `HandleMoveNotActiveMover`).
#[tokio::test]
async fn a_not_active_mover_for_a_stale_or_held_unit_changes_nothing() {
    let (_realm, mut a, ga, _b, gb, map) = held().await;
    let here = on(&map, move |m| m.players[&ga].position()).await;
    for old in [Guid::new(0xF130_0000_0000_0099), gb] {
        a.send(CMSG_MOVE_NOT_ACTIVE_MOVER {
            old_mover: old,
            info: still(here),
        })
        .await;
    }
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    let mover = on(&map, move |m| m.players[&ga].owned.mover).await;
    assert_eq!(mover, Some(gb));
}

/// Thistle Tea's "stale releases cannot end a different controller's
/// spell": Mind Control taken off its victim by a caster not its own, or
/// another spell gone from him, leaves him held; its own caster's ends it.
#[tokio::test]
async fn a_release_by_another_caster_or_spell_leaves_him_held() {
    let (realm, _a, ga, mut b, gb, map) = held().await;
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Other", Race::Human, Class::Priest).await;
    let kept = on(&map, move |m| {
        auras::remove_spell(m, gb, MIND_CONTROL, Some(gc));
        auras::remove_spell(m, gb, FEAR, Some(ga));
        crate::game::map::pets::possess::update(m);
        m.players[&gb].owned.possessed_by.map(|p| p.master)
    })
    .await;
    assert_eq!(kept, Some(ga));
    on(&map, move |m| {
        auras::remove_spell(m, gb, MIND_CONTROL, Some(ga));
    })
    .await;
    assert!(control_update(&mut b, gb).await, "his own again");
    assert!(on(&map, move |m| m.players[&gb].owned.possessed_by.is_none()).await);
}

/// Thistle Tea's "stops a controlled player's melee without cancelling
/// its ordinary spell": the possessor's stop (`CMSG_PET_STOP_ATTACK`) ends
/// his swings and leaves his cast going; another's does nothing.
#[tokio::test]
async fn a_possessors_stop_ends_his_swings_and_leaves_his_cast() {
    let (realm, mut a, ga, _b, gb, map) = held().await;
    let mut c = Client::connect(&realm, "C");
    c.enter("Other", Race::Human, Class::Priest).await;
    on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 10, 3.0);
        crate::game::map::combat::start_attack(m, gb, brute);
        let ends = m.now + Duration::from_secs(10);
        if let Some(caster) = m.caster_mut(gb) {
            caster.casting = Some(crate::game::map::spells::Casting {
                spell: SLOW,
                targets: Default::default(),
                ends,
                item: None,
                pushbacks: 0,
                channel_until: None,
                duration: 10_000,
            });
        }
    })
    .await;
    let attacking = move |m: &Map| m.combat(gb).is_some_and(|c| c.attacking);
    c.send(CMSG_PET_STOP_ATTACK { pet: gb }).await;
    crate::game::tests_p3_player2_ammo::handled(&mut c).await;
    assert!(
        on(&map, move |m| attacking(m)).await,
        "another's stop: nothing"
    );
    a.send(CMSG_PET_STOP_ATTACK { pet: gb }).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    let after = on(&map, move |m| {
        (
            attacking(m),
            m.caster(gb)
                .and_then(|c| c.casting.as_ref())
                .map(|c| c.spell),
        )
    })
    .await;
    assert_eq!(after, (false, Some(SLOW)));
}

/// Thistle Tea's "root and speed instructions go to the controller" and
/// "a controlled root acknowledgement never relocates the caster": a held
/// player's root is told to the client moving him, not his own; that
/// client's acknowledgement settles it and moves no one.
#[tokio::test]
async fn a_held_players_root_goes_to_his_mover_and_its_ack_moves_no_one() {
    let (_realm, mut a, ga, mut b, gb, map) = held().await;
    a.clear();
    b.clear();
    on(&map, move |m| movement::send_root(m, gb, true)).await;
    let counter = a
        .wait_for("SMSG_FORCE_MOVE_ROOT", |m| match m {
            ServerOpcodeMessage::SMSG_FORCE_MOVE_ROOT(r) if r.guid == gb => Some(r.counter),
            _ => None,
        })
        .await;
    let theirs = b
        .gets(NONE_WITHIN, |m| {
            matches!(m, ServerOpcodeMessage::SMSG_FORCE_MOVE_ROOT(_)).then_some(())
        })
        .await;
    assert!(theirs.is_none(), "not to his own client");
    let (priest, warrior) = on(&map, move |m| {
        (m.players[&ga].position(), m.players[&gb].position())
    })
    .await;
    let far = Vector3d {
        x: priest.x + 40.0,
        ..priest
    };
    a.send(CMSG_FORCE_MOVE_ROOT_ACK {
        guid: gb,
        movement_counter: counter,
        info: still(far),
    })
    .await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    let after = on(&map, move |m| {
        (
            m.players[&ga].position(),
            m.players[&gb].position(),
            m.players[&ga].outbound.pending_orders().is_empty(),
        )
    })
    .await;
    assert_eq!((after.0, after.1), (priest, warrior), "no one moved");
    assert!(after.2, "settled");
}

/// Thistle Tea's "fear recovery keeps the victim locked while restoring
/// the controller": a fear on a held player, ended, gives control back to
/// the client moving him, never to his own.
#[tokio::test]
async fn a_fear_ended_on_a_held_player_gives_him_back_to_his_mover() {
    let (_realm, mut a, ga, mut b, gb, map) = held().await;
    on(&map, move |m| {
        // Another's fear: the priest's own cast would end her channel.
        let brute = spawn(m, ga, BRUTE, 10, 3.0);
        crate::game::map::spells::cast_triggered(m, brute, FEAR, Some(gb));
    })
    .await;
    harness::an_update(&map).await;
    a.clear();
    b.clear();
    let feared = on(&map, move |m| {
        let driven = crate::game::map::player_motion::driven(m, gb);
        auras::remove_spell(m, gb, FEAR, None);
        (driven, m.players[&gb].owned.possessed_by.is_some())
    })
    .await;
    assert_eq!(feared, (true, true), "feared while held");
    harness::an_update(&map).await;
    assert!(
        control_update(&mut a, gb).await,
        "the priest moves him again"
    );
    let theirs = b
        .gets(NONE_WITHIN, |m| match m {
            ServerOpcodeMessage::SMSG_CLIENT_CONTROL_UPDATE(u) if u.allow_movement => Some(()),
            _ => None,
        })
        .await;
    assert!(theirs.is_none(), "his own client stays locked");
}

/// Thistle Tea's "relocates, projects the launch, then accepts a normal
/// landing": a launch's acknowledgement puts its player where the client
/// says (in the grid too) and those who see him watch him fly; his fall is
/// from the launch, and a landing a yard down hurts nothing.
#[tokio::test]
async fn a_launch_acknowledged_places_him_and_a_short_landing_is_safe() {
    let realm = harness::realm_with(data());
    let (mut a, ga, map) = enter(&realm, "A", "Alice", Race::Human, Class::Warrior).await;
    let (mut b, gb, _) = enter(&realm, "B", "Bob", Race::Human, Class::Warrior).await;
    bring(&map, gb, ga, 3.0).await;
    a.sees(gb).await;
    b.clear();
    on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, ga, KNOCK, Some(gb));
    })
    .await;
    let body = b.wait_frame("SMSG_MOVE_KNOCK_BACK", 0x0EF, |_| true).await;
    let mask = body[0];
    let at = 1 + mask.count_ones() as usize;
    let counter = u32::from_le_bytes(body[at..at + 4].try_into().unwrap());
    let f = |n: usize| f32::from_le_bytes(body[at + 4 + 4 * n..at + 8 + 4 * n].try_into().unwrap());
    let impulse = [f(0), f(1), f(2), f(3)];
    let from = on(&map, move |m| m.players[&gb].position()).await;
    let up = Vector3d {
        x: from.x + 6.0,
        z: from.z + 1.0,
        ..from
    };
    a.clear();
    b.send(CMSG_MOVE_KNOCK_BACK_ACK {
        guid: gb,
        counter,
        info: MovementInfo {
            flags: MovementInfo_MovementFlags::new_jumping(MovementInfo_MovementFlags_Jumping {
                cos_angle: impulse[0],
                sin_angle: impulse[1],
                xy_speed: impulse[2],
                z_speed: impulse[3],
            }),
            timestamp: 0,
            position: up,
            orientation: 0.0,
            fall_time: 0,
        },
    })
    .await;
    a.wait_frame("MSG_MOVE_KNOCK_BACK", 0x0F1, |_| true).await;
    let placed = on(&map, move |m| {
        let p = &m.players[&gb];
        (
            p.position(),
            p.cell == crate::game::map::grid::cell_of(up.x, up.y),
            p.fields.get_u32(crate::game::fields::UNIT_FIELD_HEALTH),
        )
    })
    .await;
    assert_eq!((placed.0, placed.1), (up, true), "where his client says");
    let landed = Vector3d {
        x: up.x + 2.0,
        z: from.z,
        ..up
    };
    b.send(MSG_MOVE_FALL_LAND_Client {
        info: MovementInfo {
            flags: MovementInfo_MovementFlags::empty(),
            timestamp: 0,
            position: landed,
            orientation: 0.0,
            fall_time: 1000,
        },
    })
    .await;
    crate::game::tests_p3_player2_ammo::handled(&mut b).await;
    let after = on(&map, move |m| {
        let p = &m.players[&gb];
        (
            p.position(),
            p.fields.get_u32(crate::game::fields::UNIT_FIELD_HEALTH),
        )
    })
    .await;
    assert_eq!(after, (landed, placed.2), "landed, unhurt");
}

/// Thistle Tea's "timed expiry removes process and clears the matching
/// owner slot": a companion called for two seconds goes at their end, and
/// its owner's companion slot is empty, so the next call is a new one.
#[tokio::test]
async fn a_timed_companion_goes_at_its_end_and_frees_its_slot() {
    let realm = harness::realm_with(data());
    let (mut a, g, map) = enter(&realm, "A", "Alice", Race::Human, Class::Warrior).await;
    let critter = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, g, COMPANION, Some(g));
        m.players[&g].owned.critter
    })
    .await
    .expect("called");
    a.sees(critter).await;
    on(&map, move |m| {
        let now = m.now;
        if let Some(s) = m.creature_mut(critter).and_then(|c| c.pet.as_deref_mut()) {
            s.expires = Some(now);
        }
    })
    .await;
    a.forgets(critter).await;
    let after = on(&map, move |m| {
        (m.creature(critter).is_some(), m.players[&g].owned.critter)
    })
    .await;
    assert_eq!(after, (false, None));
    let again = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, g, COMPANION, Some(g));
        m.players[&g].owned.critter
    })
    .await;
    assert!(again.is_some_and(|c| c != critter), "a new one");
    let _ = START;
}
