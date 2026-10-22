//! Round 12c's movement flags: what a player has no aura or order for is
//! taken off before the move is kept and told on (mangos's
//! `VerifyMovementFlags`); what it has a right to stays.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::harness::Client;
use crate::game::map::MapHandle;
use crate::game::map::auras::{self, AURA_WATER_WALK};
use crate::game::tests_p3_combat::START;
use crate::game::tests_p3_entity::{aura_effect, on, spell};
use crate::game::tests_p12c_speed::{at, beat, realm_on};

const WATER_WALK: u32 = 97_823;

/// Alice, not a GM, and Bob watching her.
async fn two() -> (
    Client,
    Guid,
    Client,
    MapHandle,
    std::sync::Arc<crate::game::Realm>,
) {
    let mut d = crate::game::tests_p12c_speed::data();
    d.spells.spells.insert(
        WATER_WALK,
        spell(WATER_WALK, &[aura_effect(AURA_WATER_WALK, 1, 0, 1)]),
    );
    let realm = realm_on(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let (b, _) = crate::game::tests_p3_entity::bob(&realm, g).await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    (a, g, b, map, realm)
}

/// The flags Bob is told Alice's heartbeat at `x` yards along has.
async fn told(b: &mut Client, x: f32) -> MovementInfo_MovementFlags {
    b.gets(Duration::from_secs(2), |m| match m {
        ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(h)
            if (h.info.position.x - (START.0 + x)).abs() < 0.01 =>
        {
            Some(h.info.flags)
        }
        _ => None,
    })
    .await
    .expect("the heartbeat is told on")
}

fn kept(map: &MapHandle, g: Guid) -> impl Future<Output = MovementInfo_MovementFlags> {
    on(map, move |m| m.players[&g].movement.flags)
}

/// survey 12c: the flags were kept and told on as they came, so a client
/// could walk on water, hover, levitate and fly with no aura. Each is taken
/// off, and the rest of the packet kept.
#[tokio::test]
async fn flags_without_a_right_to_them_are_taken_off() {
    let (a, g, mut b, map, _realm) = two().await;
    let cheats = [
        MovementInfo_MovementFlags::new_waterwalking(),
        MovementInfo_MovementFlags::new_hover(),
        MovementInfo_MovementFlags::new_levitating(),
        MovementInfo_MovementFlags::new_can_fly(),
        MovementInfo_MovementFlags::new_flying(),
        MovementInfo_MovementFlags::new_safe_fall(),
    ];
    for (i, f) in cheats.into_iter().enumerate() {
        let x = 0.5 * (i + 1) as f32;
        beat(&a, f.set_forward(), at(x, 0.0)).await;
        let seen = told(&mut b, x).await;
        assert_eq!(seen, MovementInfo_MovementFlags::new_forward(), "cheat {i}");
        assert_eq!(
            kept(&map, g).await,
            MovementInfo_MovementFlags::new_forward()
        );
    }
    // Swimming and flying at once: both go.
    let swim = MovementInfo_MovementFlags::new_swimming(MovementInfo_MovementFlags_Swimming {
        pitch: 0.0,
    });
    beat(&a, swim.set_flying(), at(4.0, 0.0)).await;
    assert_eq!(told(&mut b, 4.0).await, MovementInfo_MovementFlags::empty());
}

/// With the aura, water walking stays; as it goes, it stays until the
/// client has acknowledged it gone (the order on its way); then it goes.
#[tokio::test]
async fn water_walking_with_its_aura_or_its_order_on_the_way_stays() {
    let (mut a, g, mut b, map, _realm) = two().await;
    on(&map, move |m| {
        let s = m.realm.data.spells.get(WATER_WALK).cloned().unwrap();
        auras::apply(m, g, g, &s, 0, 1);
    })
    .await;
    let walk = MovementInfo_MovementFlags::new_waterwalking();
    beat(&a, walk, at(1.0, 0.0)).await;
    assert!(told(&mut b, 1.0).await.get_waterwalking(), "the aura's");
    on(&map, move |m| auras::remove_spell(m, g, WATER_WALK, None)).await;
    let counter = a
        .wait_for("SMSG_MOVE_LAND_WALK", |m| match m {
            ServerOpcodeMessage::SMSG_MOVE_LAND_WALK(w) if w.guid == g => Some(w.counter),
            _ => None,
        })
        .await;
    beat(&a, walk, at(2.0, 0.0)).await;
    assert!(
        told(&mut b, 2.0).await.get_waterwalking(),
        "the order on its way"
    );
    a.send(CMSG_MOVE_WATER_WALK_ACK {
        guid: g,
        movement_counter: counter,
        info: crate::game::tests_p12c_speed::info(
            MovementInfo_MovementFlags::empty(),
            at(2.0, 0.0),
        ),
        apply: 0,
    })
    .await;
    beat(&a, walk, at(3.0, 0.0)).await;
    assert!(!told(&mut b, 3.0).await.get_waterwalking(), "gone");
}

/// A GM keeps whatever flags it sends (mangos: "do not apply to gms").
#[tokio::test]
async fn a_gm_keeps_its_flags() {
    let (a, g, mut b, map, _realm) = two().await;
    on(&map, move |m| m.players.get_mut(&g).unwrap().gm = true).await;
    beat(&a, MovementInfo_MovementFlags::new_hover(), at(1.0, 0.0)).await;
    assert!(told(&mut b, 1.0).await.get_hover());
}

/// The bug hunt's map core: in a knockback's flight no flag was taken off,
/// so a client kept "launched" (its jumping flag on) could fly. Flying,
/// being able to fly and levitating go in a launch too; its jump stays.
#[tokio::test]
async fn a_knockbacks_flight_gives_no_flying() {
    let (mut a, g, mut b, map, _realm) = two().await;
    crate::game::tests_p12c_speed::knocked_back(&mut a, &map, g, 10.0).await;
    let jump = MovementInfo_MovementFlags::new_jumping(MovementInfo_MovementFlags_Jumping {
        cos_angle: 1.0,
        sin_angle: 0.0,
        xy_speed: 10.0,
        z_speed: 10.0,
    });
    let cheat = jump.set_flying().set_can_fly().set_levitating();
    beat(&a, cheat, at(1.0, 0.0)).await;
    let seen = told(&mut b, 1.0).await;
    assert!(seen.get_jumping().is_some(), "the launch's jump stays");
    assert!(
        !seen.get_flying() && !seen.get_can_fly() && !seen.get_levitating(),
        "{seen:?}"
    );
    let k = kept(&map, g).await;
    assert!(!k.get_flying() && !k.get_can_fly() && !k.get_levitating());
}
