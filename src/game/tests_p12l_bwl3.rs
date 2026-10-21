//! Blackwing Lair on the real world, as mangos-classic scripts it:
//! Chromaggus's breaths, shields and afflictions.

use std::time::Duration;

use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use super::tests_p12l_bwl::*;
use crate::game::map::bwl;
use crate::game::map::bwl::chromaggus as ch;
use crate::game::map::{ai, instance_script};

/// Before Chromaggus.
const KENNEL: Vector3d = Vector3d {
    x: -7510.0,
    y: -1028.0,
    z: 476.7,
};

/// Chromaggus: his two breaths picked for the copy, different; at the pull
/// a shield of one school and his shimmer; the left breath thirty seconds
/// on; an affliction on those he fights, and all five mutate; his frenzy
/// with its emote; enraged below 20 percent; his death opens the way on.
#[tokio::test]
#[ignore = "extended"]
async fn chromaggus_breathes_shields_and_afflicts_as_mangos_classic() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = hero_in(&realm, "Dogfood", BWL, KENNEL, 0.0, &[]).await;
    not_gm(&map, me).await;
    let dog = boss(&map, bwl::NPC_CHROMAGGUS).await;
    c.sees(dog).await;
    let (left, right) = map
        .query(|m| (field(m, bwl::CHROMA_LBREATH), field(m, bwl::CHROMA_RBREATH)))
        .await
        .unwrap();
    let li = ch::LEFT_BREATHS.iter().position(|s| *s == left);
    let ri = ch::RIGHT_BREATHS.iter().position(|s| *s == right);
    assert!(
        li.is_some() && ri.is_some(),
        "two breaths picked: {left} {right}"
    );
    assert_ne!(li, ri, "of two kinds");
    // A second, so he fights on while one is his.
    let (_o, other) =
        super::tests_p12l_bwl2::joins_as(&realm, "Dogtoy", Class::Mage, &map, KENNEL).await;
    map.query(move |m| {
        tough(m, dog);
        ai::attacked(m, dog, me);
        ai::attacked(m, dog, other);
    })
    .await;
    assert!(
        until(&map, 3000, move |m| ch::SHIELDS
            .iter()
            .any(|s| has(m, dog, *s))
            && field(m, bwl::CHROMAGGUS) == instance_script::IN_PROGRESS)
        .await,
        "a shield at the pull"
    );
    assert!(
        emoted(&mut c, "Chromaggus", Duration::from_secs(3))
            .await
            .is_some(),
        "his shimmer"
    );
    // The left breath, thirty seconds on.
    c.clear();
    map.query(|m| {
        for (t, _, _) in m.instance.creature_timers.iter_mut() {
            if let Some(sooner) = t.checked_sub(Duration::from_secs(30)) {
                *t = sooner;
            }
        }
        crate::game::map::instance_script::update(m);
    })
    .await;
    assert!(
        cast_any_of(&mut c, dog, left, Duration::from_secs(3))
            .await
            .is_some(),
        "the left breath"
    );
    map.query(move |m| crate::game::map::spells::interrupt(m, dog))
        .await;
    // An affliction; all five, mutated.
    let e = |n: u32| bwl::NPC_CHROMAGGUS * 100 + n;
    map.query(move |m| {
        if let Some(cr) = m.caster_mut(dog) {
            cr.gcd_until = None;
        }
        due(m, dog, e(3));
    })
    .await;
    assert!(
        until(&map, 3000, move |m| ch::AFFLICTIONS
            .iter()
            .any(|a| has(m, me, *a)))
        .await,
        "an affliction"
    );
    for _ in 0..20 {
        let all = map
            .query(move |m| {
                for a in ch::AFFLICTIONS {
                    if !has(m, me, a) {
                        crate::game::map::spells::cast_triggered(m, dog, a, Some(me));
                    }
                }
                ch::AFFLICTIONS.iter().all(|a| has(m, me, *a))
            })
            .await
            .unwrap();
        if all {
            break;
        }
        sleep_ms(50).await;
    }
    let held: Vec<bool> = map
        .query(move |m| ch::AFFLICTIONS.iter().map(|a| has(m, me, *a)).collect())
        .await
        .unwrap();
    assert!(held.iter().all(|h| *h), "all five on him: {held:?}");
    map.query(move |m| {
        if let Some(cr) = m.caster_mut(dog) {
            cr.gcd_until = None;
        }
        due(m, dog, e(3));
    })
    .await;
    assert!(
        until(&map, 3000, move |m| has(m, me, ch::CHROMATIC_MUTATION)).await,
        "all five, mutated"
    );
    c.clear();
    timed_cast(
        &mut c,
        &map,
        dog,
        e(4),
        ch::FRENZY,
        (15.0, 15.0),
        (15.0, 15.0),
        None,
    )
    .await;
    map.query(move |m| {
        set_pct(m, dog, 15);
        if let Some(s) = crate::game::map::event_ai::scripted_mut(m, dog) {
            s.next_tick = None;
        }
        crate::game::map::event_ai::tick(m, dog);
    })
    .await;
    assert!(
        until(&map, 3000, move |m| has(m, dog, ch::ENRAGE)).await,
        "enraged below 20 percent"
    );
    map.query(move |m| kill(m, me, dog)).await;
    assert!(
        until(&map, 2000, |m| field(m, bwl::CHROMAGGUS)
            == instance_script::DONE
            && door_state(m, bwl::DOOR_CHROMAGGUS_EXIT) == Some(0))
        .await,
        "his death opens the way on"
    );
}
