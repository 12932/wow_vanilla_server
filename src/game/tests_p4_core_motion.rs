//! Phase 3, round 4, against Thistle Tea's `movement/falling_test.exs`: a
//! fall's damage from the highest point, capped at the health, what eases
//! it or stops it, what doesn't, and what ends a fall before it lands.
//! Through the client's movement packets.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::auras;
use crate::game::map::{Map, MapKey};
use crate::game::tests_p3_combat::{START, aura_of, data};

/// Long enough in the air to hurt, and too short.
const LONG: u32 = 2_000;

fn info(flags: MovementInfo_MovementFlags, z: f32, fall_ms: u32) -> MovementInfo {
    MovementInfo {
        flags,
        timestamp: 0,
        position: Vector3d {
            x: START.0,
            y: START.1,
            z,
        },
        orientation: 0.0,
        fall_time: fall_ms,
    }
}

fn falling() -> MovementInfo_MovementFlags {
    MovementInfo_MovementFlags::empty().set_fallingfar()
}

async fn jump(a: &Client, z: f32) {
    a.send(MSG_MOVE_JUMP_Client {
        info: info(falling(), z, 0),
    })
    .await;
}

async fn moving(a: &Client, flags: MovementInfo_MovementFlags, z: f32) {
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: info(flags, z, 0),
    })
    .await;
}

async fn land(a: &Client, z: f32, fall_ms: u32) {
    a.send(MSG_MOVE_FALL_LAND_Client {
        info: info(MovementInfo_MovementFlags::empty(), z, fall_ms),
    })
    .await;
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

/// Alice at START, at full health; her maximum.
async fn alice() -> (Arc<Realm>, Client, Guid, u32) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let max = heal(&realm, g).await;
    (realm, a, g, max)
}

async fn heal(realm: &Arc<Realm>, g: Guid) -> u32 {
    on_map(realm, move |m| {
        let f = m.unit_fields_mut(g).unwrap();
        let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
        f.set_u32(UNIT_FIELD_HEALTH, max);
        max
    })
    .await
}

async fn health(realm: &Arc<Realm>, g: Guid) -> u32 {
    on_map(realm, move |m| {
        m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_HEALTH)
    })
    .await
}

fn hurt(m: &ServerOpcodeMessage) -> Option<u32> {
    match m {
        ServerOpcodeMessage::SMSG_ENVIRONMENTAL_DAMAGE_LOG(d) => Some(d.damage),
        _ => None,
    }
}

/// Wait until the map has Alice at `z`: her packets come through her
/// session, a query straight to the map.
async fn at(realm: &Arc<Realm>, g: Guid, z: f32) {
    for _ in 0..500 {
        let here = on_map(realm, move |m| m.players[&g].movement.position.z).await;
        if (here - z).abs() < 0.01 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    panic!("Alice never at {z}");
}

const NONE_WITHIN: Duration = Duration::from_millis(300);

/// vmangos's fall: 1.8% of the maximum a yard, less a quarter of it.
fn fall_damage(yards: f32, max: u32) -> u32 {
    ((0.018 * yards - 0.2426) * max as f32) as u32
}

#[tokio::test]
async fn a_fall_hurts_from_its_highest_point_once_and_a_long_one_kills_for_no_more_than_the_health()
{
    let (realm, mut a, g, max) = alice().await;
    // Up to 35 above, down through 20: 35 yards fallen.
    jump(&a, START.2 + 30.0).await;
    jump(&a, START.2 + 35.0).await;
    jump(&a, START.2 + 20.0).await;
    land(&a, START.2, LONG).await;
    assert_eq!(a.wait_for("the fall", hurt).await, fall_damage(35.0, max));
    // Landing again is no fall.
    land(&a, START.2, LONG).await;
    assert!(a.gets(NONE_WITHIN, hurt).await.is_none());
    // Two hundred yards: all of her health, and she dies of it.
    heal(&realm, g).await;
    jump(&a, START.2 + 200.0).await;
    land(&a, START.2, LONG).await;
    assert_eq!(a.wait_for("the long fall", hurt).await, max);
    let dead = on_map(&realm, move |m| {
        m.players[&g].death != crate::game::map::death::Death::Alive
    })
    .await;
    assert!(dead);
    // The dead fall unhurt.
    jump(&a, START.2 + 200.0).await;
    land(&a, START.2, LONG).await;
    assert!(a.gets(NONE_WITHIN, hurt).await.is_none());
}

#[tokio::test]
async fn safe_fall_takes_yards_off_slow_fall_hover_and_immunity_take_it_all() {
    let (realm, mut a, g, max) = alice().await;
    on_map(&realm, move |m| {
        aura_of(m, g, 97_601, auras::AURA_SAFE_FALL, 0, 10)
    })
    .await;
    jump(&a, START.2 + 30.0).await;
    land(&a, START.2, LONG).await;
    assert_eq!(
        a.wait_for("a safer fall", hurt).await,
        fall_damage(20.0, max)
    );
    // Twenty, less ten: nothing.
    heal(&realm, g).await;
    jump(&a, START.2 + 20.0).await;
    land(&a, START.2, LONG).await;
    assert!(a.gets(NONE_WITHIN, hurt).await.is_none());
    for (id, kind, misc) in [
        (97_602, auras::AURA_FEATHER_FALL, 0),
        (97_603, auras::AURA_HOVER, 0),
        (97_604, auras::AURA_SCHOOL_IMMUNITY, 1),
    ] {
        on_map(&realm, move |m| {
            auras::remove_spell(m, g, 97_601, None);
            aura_of(m, g, id, kind, misc, 1);
        })
        .await;
        jump(&a, START.2 + 200.0).await;
        land(&a, START.2, LONG).await;
        assert!(a.gets(NONE_WITHIN, hurt).await.is_none(), "aura {kind}");
        on_map(&realm, move |m| auras::remove_spell(m, g, id, None)).await;
    }
    assert_eq!(health(&realm, g).await, max);
}

/// Thistle Tea's fall starts again where the next falling packet is (10
/// yards up: no harm); wvs's where the player last was, as a step off a
/// ledge starts at the ledge (20 yards). Either way the 200 before the
/// slow fall are gone.
#[tokio::test]
async fn slow_fall_in_the_air_forgets_the_fall_before_it() {
    let (realm, mut a, g, max) = alice().await;
    jump(&a, START.2 + 200.0).await;
    at(&realm, g, START.2 + 200.0).await;
    on_map(&realm, move |m| {
        aura_of(m, g, 97_602, auras::AURA_FEATHER_FALL, 0, 1)
    })
    .await;
    jump(&a, START.2 + 20.0).await;
    at(&realm, g, START.2 + 20.0).await;
    on_map(&realm, move |m| auras::remove_spell(m, g, 97_602, None)).await;
    jump(&a, START.2 + 10.0).await;
    land(&a, START.2, LONG).await;
    assert_eq!(
        a.wait_for("the fall after", hurt).await,
        fall_damage(20.0, max)
    );
}

#[tokio::test]
async fn shields_and_shares_dont_stop_a_fall_and_damage_taken_counts_once() {
    let (realm, mut a, g, max) = alice().await;
    on_map(&realm, move |m| {
        aura_of(m, g, 97_701, auras::AURA_SCHOOL_ABSORB, 1, 1_000)
    })
    .await;
    jump(&a, START.2 + 30.0).await;
    land(&a, START.2, LONG).await;
    assert_eq!(
        a.wait_for("a shielded fall", hurt).await,
        fall_damage(30.0, max)
    );
    let (left, shield) = on_map(&realm, move |m| {
        let shield = auras::auras(m, g)
            .and_then(|a| a.holders.iter().find(|h| h.spell == 97_701))
            .map(|h| h.effects[0].left);
        (m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_HEALTH), shield)
    })
    .await;
    assert_eq!(left, max - fall_damage(30.0, max));
    assert_eq!(shield, Some(1_000), "the shield untouched");
    // Half the physical damage taken: half the fall, once.
    heal(&realm, g).await;
    on_map(&realm, move |m| {
        auras::remove_spell(m, g, 97_701, None);
        aura_of(m, g, 97_702, auras::AURA_MOD_DAMAGE_PERCENT_TAKEN, 1, -50);
    })
    .await;
    jump(&a, START.2 + 30.0).await;
    land(&a, START.2, LONG).await;
    let halved = ((0.018 * 30.0 - 0.2426) * max as f32 * 0.5) as u32;
    assert_eq!(a.wait_for("a halved fall", hurt).await, halved);
    assert_eq!(health(&realm, g).await, max - halved);
}

#[tokio::test]
async fn the_ground_the_water_or_a_teleport_ends_a_fall_and_one_may_start_below_zero() {
    let (realm, mut a, g, max) = alice().await;
    // Standing a moment on a ledge, or swimming, mid-fall.
    for flags in [
        MovementInfo_MovementFlags::empty(),
        MovementInfo_MovementFlags::empty().set_swimming(Default::default()),
    ] {
        jump(&a, START.2 + 200.0).await;
        moving(&a, flags, START.2 + 20.0).await;
        land(&a, START.2, LONG).await;
        assert!(a.gets(NONE_WITHIN, hurt).await.is_none());
    }
    // Moved by the server mid-fall.
    jump(&a, START.2 + 200.0).await;
    at(&realm, g, START.2 + 200.0).await;
    on_map(&realm, move |m| {
        crate::game::map::movement::teleport_within(
            m,
            g,
            Vector3d {
                x: START.0,
                y: START.1,
                z: START.2,
            },
            0.0,
        )
    })
    .await;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
    land(&a, START.2, LONG).await;
    assert!(a.gets(NONE_WITHIN, hurt).await.is_none());
    assert_eq!(health(&realm, g).await, max);
    // From nothing to thirty below.
    moving(&a, MovementInfo_MovementFlags::empty(), 0.0).await;
    jump(&a, 0.0).await;
    land(&a, -30.0, LONG).await;
    assert_eq!(
        a.wait_for("a fall below zero", hurt).await,
        fall_damage(30.0, max)
    );
}

const KNOCK: u32 = 97_951;
const LAUNCH: u32 = 97_952;
const PULL: u32 = 97_953;
const BOLT: u32 = 97_954;

fn knock_world() -> crate::game::data::GameData {
    use crate::game::data::spells::{SpellEffect, SpellTemplate};
    let mut d = data();
    let effect = |effect: u32, misc: i32, points: i32, target_a: u32| SpellEffect {
        effect,
        misc_value: misc,
        base_points: points - 1,
        base_dice: 1,
        target_a,
        ..Default::default()
    };
    for (id, e) in [
        (KNOCK, effect(98, 120, 70, 6)),
        (LAUNCH, effect(98, 100, 100, 1)),
        (PULL, effect(124, 300, 0, 6)),
    ] {
        let mut s = SpellTemplate {
            id,
            name: format!("Spell {id}"),
            equipped_item_class: -1,
            range_index: 1,
            ..Default::default()
        };
        s.effects[0] = e;
        d.spells.spells.insert(id, s);
    }
    // A bolt of three seconds.
    d.spells.cast_times.insert(5, 3_000);
    let mut bolt = SpellTemplate {
        id: BOLT,
        name: "Bolt".into(),
        equipped_item_class: -1,
        range_index: 1,
        casting_time_index: 5,
        ..Default::default()
    };
    bolt.effects[0] = effect(2, 0, 10, 6);
    d.spells.spells.insert(BOLT, bolt);
    d
}

/// `SMSG_MOVE_KNOCK_BACK`'s cos, sin, horizontal and vertical speeds.
fn launch_of(b: &[u8]) -> [f32; 4] {
    let at = 1 + b[0].count_ones() as usize + 4;
    std::array::from_fn(|i| f32::from_le_bytes(b[at + i * 4..at + i * 4 + 4].try_into().unwrap()))
}

#[tokio::test]
async fn a_knockback_throws_a_player_away_from_its_caster_or_back_from_herself_breaking_her_cast() {
    use crate::game::map::spells;
    let realm = harness::realm_with(knock_world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    jump(&a, START.2 + 100.0).await;
    at(&realm, g, START.2 + 100.0).await;
    let r = on_map(&realm, move |m| {
        crate::game::tests_p3_combat::level(m, g, 60);
        // A brute ten yards south of her.
        let brute =
            crate::game::tests_p3_combat::spawn(m, g, crate::game::tests_p3_combat::BRUTE, 20, 0.0);
        let p = m.players[&g].position();
        m.creature_mut(brute).unwrap().position = Vector3d { y: p.y - 10.0, ..p };
        m.players.get_mut(&g).unwrap().spells.push(BOLT);
        let bolt = spells::cast(
            m,
            g,
            BOLT,
            spells::Targets {
                unit: Some(brute),
                ..Default::default()
            },
            None,
        );
        let casting = m.players[&g].caster.casting.is_some();
        spells::cast_triggered(m, brute, KNOCK, Some(g));
        let after = (
            m.players[&g].caster.casting.is_none(),
            m.players[&g].fall_from,
        );
        (bolt, casting, after)
    })
    .await;
    assert_eq!(
        r,
        (true, true, (true, None)),
        "the cast broken, the fall forgotten"
    );
    let b = a.wait_frame("the knockback", 0x0EF, |_| true).await;
    let [cos, sin, xy, z] = launch_of(&b);
    // North, away from the brute: 12 along, 7 up (sent as -7, as mangos).
    assert!(
        cos.abs() < 0.001 && (sin - 1.0).abs() < 0.001,
        "{cos} {sin}"
    );
    assert_eq!((xy, z), (12.0, -7.0));
    // Her own launch: backward, facing east.
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().movement.orientation = 0.0;
        spells::cast_triggered(m, g, LAUNCH, Some(g));
    })
    .await;
    let b = a.wait_frame("the launch", 0x0EF, |_| true).await;
    let [cos, sin, ..] = launch_of(&b);
    assert!(
        (cos + 1.0).abs() < 0.001 && sin.abs() < 0.001,
        "{cos} {sin}"
    );
}

#[tokio::test]
async fn a_rooted_stunned_or_dead_player_isnt_thrown_or_pulled_nor_one_pulled_at_herself() {
    use crate::game::map::spells;
    let realm = harness::realm_with(knock_world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let brute = on_map(&realm, move |m| {
        crate::game::tests_p3_combat::level(m, g, 60);
        crate::game::tests_p3_combat::spawn(m, g, crate::game::tests_p3_combat::BRUTE, 20, 20.0)
    })
    .await;
    for (n, kind) in [auras::AURA_MOD_ROOT, auras::AURA_MOD_STUN]
        .into_iter()
        .enumerate()
    {
        on_map(&realm, move |m| {
            aura_of(m, g, 97_961 + n as u32, kind, 0, 1);
            spells::cast_triggered(m, brute, KNOCK, Some(g));
            spells::cast_triggered(m, brute, PULL, Some(g));
            auras::remove_spell(m, g, 97_961 + n as u32, None);
        })
        .await;
    }
    on_map(&realm, move |m| {
        spells::cast_triggered(m, g, PULL, Some(g));
        crate::game::map::combat::deal_damage(m, brute, g, 10_000_000);
        spells::cast_triggered(m, brute, KNOCK, Some(g));
        spells::cast_triggered(m, brute, PULL, Some(g));
    })
    .await;
    let thrown = tokio::time::timeout(
        Duration::from_millis(300),
        a.wait_frame("a throw", 0x0EF, |_| true),
    )
    .await;
    assert!(thrown.is_err(), "none thrown");
}
