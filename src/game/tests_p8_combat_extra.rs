//! Round 8's combat tests, extra attacks (Thistle Tea's
//! `extra_attacks_test.exs`): owed attacks go so close to the victim that
//! facing doesn't count, and wait, banked, while auto-attack is off.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::combat::{self, Outcome};
use crate::game::tests_p3_combat::{self as p3, BRUTE};
use crate::game::tests_p6_wspell::put;

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

/// Alice swinging at a brute a yard behind her, her hands not due for a
/// while.
fn behind(m: &mut Map, ga: Guid) -> Guid {
    p3::level(m, ga, 60);
    let brute = p3::spawn(m, ga, BRUTE, 60, 1.0);
    let me = m.players[&ga].position();
    let o = m.players[&ga].movement.orientation;
    put(
        m,
        brute,
        Vector3d {
            x: me.x - o.cos(),
            y: me.y - o.sin(),
            z: me.z,
        },
    );
    let now = m.now;
    if let Some(c) = m.combat_mut(ga) {
        c.victim = brute;
        c.attacking = true;
        c.next_swing = Some(now + Duration::from_secs(10));
        c.next_offhand = None;
    }
    brute
}

/// Thistle Tea's "ignores facing inside vanilla's overlap distance": the
/// attack a proc owes Alice lands on a brute a yard behind her, inside the
/// 1.4 yards where facing doesn't count for it (mangos's `DoExtraAttacks`
/// asks reach alone); her own swing, due, doesn't, her back turned.
#[tokio::test]
async fn an_owed_attack_lands_so_close_whatever_her_facing() {
    let (_realm, _a, ga, map) = p3::setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = behind(m, ga);
            let before = health(m, brute);
            combat::grant_extra_attacks(m, ga, 0, 1);
            combat::FORCED_OUTCOME.with(|f| f.set(Some(Outcome::Hit)));
            combat::try_swing(m, ga);
            let owed = (
                before - health(m, brute),
                m.combat(ga).unwrap().extra_attacks,
            );
            let now = m.now;
            m.combat_mut(ga).unwrap().next_swing = Some(now);
            let before = health(m, brute);
            combat::try_swing(m, ga);
            combat::FORCED_OUTCOME.with(|f| f.set(None));
            (owed, before - health(m, brute))
        })
        .await
        .unwrap();
    assert!(r.0.0 > 0, "the owed attack landed");
    assert_eq!(r.0.1, 0, "and is spent");
    assert_eq!(r.1, 0, "her own swing, facing away, doesn't");
}

/// Thistle Tea's "passive combat synchronization preserves banked attacks
/// while auto-attack is off": an attack owed while Alice isn't swinging
/// waits through the map's updates, and goes once she swings.
#[tokio::test]
async fn an_owed_attack_waits_banked_while_she_isnt_swinging() {
    let (_realm, a, ga, map) = p3::setup(Class::Warrior).await;
    let brute = map
        .query(move |m| {
            p3::level(m, ga, 60);
            let brute = p3::spawn(m, ga, BRUTE, 60, 2.0);
            combat::grant_extra_attacks(m, ga, 0, 1);
            brute
        })
        .await
        .unwrap();
    for _ in 0..5 {
        crate::game::harness::an_update(&map).await;
    }
    let (banked, before) = map
        .query(move |m| (m.combat(ga).unwrap().extra_attacks, health(m, brute)))
        .await
        .unwrap();
    assert_eq!(banked, 1, "banked through the updates");
    a.send(CMSG_ATTACKSWING { guid: brute }).await;
    let deadline = tokio::time::Instant::now() + crate::game::harness::WAIT;
    loop {
        let (owed, now) = map
            .query(move |m| (m.combat(ga).unwrap().extra_attacks, health(m, brute)))
            .await
            .unwrap();
        if owed == 0 && now < before {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "spent on her swing");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
