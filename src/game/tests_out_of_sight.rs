//! A creature gives up on a victim the moment it drops out of their sight
//! (`[creatures] evade_out_of_sight`): in the visibility step that tells
//! their client to forget it, it drops them from its fight, and goes home
//! if no one it fights is left in sight; off, the leash decides as before.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::{Map, MapHandle, ai};
use crate::game::tests_p3_combat::{BRUTE, START};
use crate::game::tests_p6_wspell::put;
use crate::game::tests_p7_combat::{creature_at, in_combat, off, setup_with};

/// `g` hardy, so a long fight doesn't kill it.
fn hardy(m: &mut Map, g: Guid) {
    let f = m.unit_fields_mut(g).unwrap();
    f.set_u32(UNIT_FIELD_MAXHEALTH, 10_000_000);
    f.set_u32(UNIT_FIELD_HEALTH, 10_000_000);
}

fn knows(m: &Map, player: Guid, g: Guid) -> bool {
    m.players.get(&player).is_some_and(|p| p.known.contains(&g))
}

/// The evade setting, as a reload would make it.
fn evade_out_of_sight(m: &mut Map, on: bool) {
    let mut cfg = (*m.cfg).clone();
    cfg.creatures.evade_out_of_sight = on;
    m.cfg = std::sync::Arc::new(cfg);
}

/// A brute 5 yards from Alice fighting her, Alice hardy, and its create
/// at her client.
async fn fight(map: &MapHandle, a: &mut Client, ga: Guid, on: bool) -> Guid {
    let brute = map
        .query(move |m| {
            evade_out_of_sight(m, on);
            hardy(m, ga);
            let b = creature_at(m, ga, BRUTE, off(5.0, 0.0, 0.0));
            hardy(m, b);
            ai::attacked(m, b, ga);
            b
        })
        .await
        .unwrap();
    a.sees(brute).await;
    let started = map
        .query(move |m| {
            m.creature(brute)
                .is_some_and(|c| c.combat.attacking && c.combat.victim == ga)
                && in_combat(m, ga)
        })
        .await
        .unwrap();
    assert!(started, "the fight began");
    brute
}

/// What the brute is doing, polled until Alice's client no longer knows it,
/// in the query that first finds it so: as of the update that forgot it.
#[derive(Debug, PartialEq)]
struct AtForget {
    evading: bool,
    attacking: bool,
    victim: Guid,
    threat: Vec<Guid>,
}

async fn at_forget(map: &MapHandle, ga: Guid, brute: Guid) -> AtForget {
    for _ in 0..1000 {
        let seen = map
            .query(move |m| {
                (!knows(m, ga, brute)).then(|| {
                    let c = m.creature(brute).unwrap();
                    AtForget {
                        evading: c.evading,
                        attacking: c.combat.attacking,
                        victim: c.combat.victim,
                        threat: c.threat.iter().map(|(t, _)| *t).collect(),
                    }
                })
            })
            .await
            .unwrap();
        if let Some(s) = seen {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("the client never forgot the brute");
}

/// Whether `g` leaves combat within `within`.
async fn leaves_combat(map: &MapHandle, g: Guid, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < deadline {
        if !map.query(move |m| in_combat(m, g)).await.unwrap() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// Alice outruns the brute's sight (150 yards off at once, past 100 and
/// its 10-yard margin): in the update her client is told to forget it, it
/// is off her, evading home with nothing on its list, and she is out of
/// combat at the next look at her fight, not at the leash's 11 to 15
/// seconds.
#[tokio::test]
async fn a_creature_whose_victim_outruns_its_sight_goes_home_as_their_client_forgets_it() {
    let (_realm, mut a, ga, map) =
        setup_with(crate::game::tests_p7_combat::data(), Class::Warrior).await;
    let brute = fight(&map, &mut a, ga, true).await;
    map.query(move |m| {
        put(m, ga, off(150.0, 0.0, 0.0));
        m.visibility_dirty.insert(ga);
    })
    .await
    .unwrap();
    let s = at_forget(&map, ga, brute).await;
    assert!(s.evading, "evading as it is forgotten: {s:?}");
    assert!(!s.attacking && s.threat.is_empty(), "{s:?}");
    a.forgets(brute).await;
    assert!(
        leaves_combat(&map, ga, Duration::from_secs(3)).await,
        "Alice out of combat"
    );
}

/// Teleported away (150 yards, too far for the teleport to keep her fight):
/// the same, at the visibility the teleport brings; before, the brute
/// kept her on its list and chased her for its leash's time.
#[tokio::test]
async fn a_creature_whose_victim_is_teleported_out_of_its_sight_goes_home() {
    let (_realm, mut a, ga, map) =
        setup_with(crate::game::tests_p7_combat::data(), Class::Warrior).await;
    let brute = fight(&map, &mut a, ga, true).await;
    map.query(move |m| m.near_teleport(ga, off(150.0, 0.0, 0.0), 0.0))
        .await
        .unwrap();
    let s = at_forget(&map, ga, brute).await;
    assert!(s.evading && s.threat.is_empty(), "{s:?}");
    a.forgets(brute).await;
    assert!(leaves_combat(&map, ga, Duration::from_secs(3)).await);
    // And stays out: nothing holds her.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let (fighting, held) = map
        .query(move |m| {
            (
                in_combat(m, ga),
                m.creature(brute)
                    .is_some_and(|c| c.threat.iter().any(|(t, _)| *t == ga)),
            )
        })
        .await
        .unwrap();
    assert!(!fighting && !held);
}

/// With Bob on its list too, still beside it, the brute drops Alice alone
/// as she goes out of sight, and fights Bob on.
#[tokio::test]
async fn a_creature_with_a_victim_still_in_sight_fights_that_one_on() {
    let (realm, mut a, ga, map) =
        setup_with(crate::game::tests_p7_combat::data(), Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0, START.1, START.2).await;
    let brute = fight(&map, &mut a, ga, true).await;
    b.sees(brute).await;
    map.query(move |m| {
        hardy(m, gb);
        let c = m.creature_mut(brute).unwrap();
        c.threat.retain(|(t, _)| *t == ga);
        c.threat[0].1 = 1000.0;
        c.threat.push((gb, 10.0));
        put(m, ga, off(150.0, 0.0, 0.0));
        m.visibility_dirty.insert(ga);
    })
    .await
    .unwrap();
    let s = at_forget(&map, ga, brute).await;
    assert_eq!(
        s,
        AtForget {
            evading: false,
            attacking: true,
            victim: gb,
            threat: vec![gb],
        }
    );
    assert!(leaves_combat(&map, ga, Duration::from_secs(3)).await);
    let bob = map
        .query(move |m| {
            in_combat(m, gb)
                && m.creature(brute)
                    .is_some_and(|c| !c.evading && c.combat.victim == gb)
        })
        .await
        .unwrap();
    assert!(bob, "Bob fought on");
}

/// Off, the leash rules: forgotten by Alice's client, the brute fights her
/// on, and she stays in combat (it goes home at the leash's time, which
/// its own tests keep).
#[tokio::test]
async fn with_the_setting_off_the_leash_decides() {
    let (_realm, mut a, ga, map) =
        setup_with(crate::game::tests_p7_combat::data(), Class::Warrior).await;
    let brute = fight(&map, &mut a, ga, false).await;
    map.query(move |m| {
        put(m, ga, off(150.0, 0.0, 0.0));
        m.visibility_dirty.insert(ga);
    })
    .await
    .unwrap();
    let s = at_forget(&map, ga, brute).await;
    assert!(!s.evading && s.attacking && s.victim == ga, "{s:?}");
    assert_eq!(s.threat, vec![ga]);
    a.forgets(brute).await;
    assert!(!leaves_combat(&map, ga, Duration::from_millis(1500)).await);
    let leash = map
        .query(move |m| crate::game::map::creature_combat::should_evade(m, brute, ga))
        .await
        .unwrap();
    assert!(!leash, "the leash's time hasn't run");
}

/// A creature hidden from its victim, not out of their sight (a script
/// conceals it), is forgotten by their client but fights on: only the
/// edge of sight ends the fight.
#[tokio::test]
async fn a_creature_hidden_from_its_victim_fights_on() {
    let (_realm, mut a, ga, map) =
        setup_with(crate::game::tests_p7_combat::data(), Class::Warrior).await;
    let brute = fight(&map, &mut a, ga, true).await;
    map.query(move |m| {
        m.creature_mut(brute)
            .unwrap()
            .scripted
            .get_or_insert_with(Default::default)
            .concealed = true;
        m.visibility_dirty.insert(brute);
    })
    .await
    .unwrap();
    let s = at_forget(&map, ga, brute).await;
    assert!(!s.evading && s.threat == vec![ga], "{s:?}");
}

/// A victim whose camera is away (an intro cinematic's, Far Sight's), not
/// themselves: their client forgets the brute, which fights them on.
/// (A fresh character's intro, its camera still followed after a teleport
/// to Onyxia's lair, had her give up on a player beside her.)
#[tokio::test]
async fn a_creature_whose_victims_camera_is_away_fights_on() {
    let (_realm, mut a, ga, map) =
        setup_with(crate::game::tests_p7_combat::data(), Class::Warrior).await;
    let brute = fight(&map, &mut a, ga, true).await;
    map.query(move |m| {
        m.players.get_mut(&ga).unwrap().misc.view = Some(off(500.0, 0.0, 0.0));
        m.visibility_dirty.insert(ga);
    })
    .await
    .unwrap();
    let s = at_forget(&map, ga, brute).await;
    assert!(
        !s.evading && s.threat == vec![ga] && s.victim == ga,
        "{s:?}"
    );
}
