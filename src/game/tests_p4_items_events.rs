//! Phase 4's world events against Thistle Tea's tests: an hour of the
//! Elemental Invasion raising each element a stage short of its lord, Omen's
//! minions staying until he leaves the world and his rest after he falls,
//! and the Scourge Invasion's zones fighting on, resting and its last
//! victory ending it.

use std::time::Duration;

use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::invasion::{self, ZONES};
use crate::game::map::MapKey;
use crate::game::map::creature::Life;
use crate::game::omen::{LAUNCHER, MINIONS_OF_OMEN, OMEN};

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

fn now() -> i64 {
    crate::game::social::mail::now()
}

#[test]
fn an_hour_raises_each_element_a_stage_short_of_its_lord() {
    let realm = harness::realm();
    let hour = |stages: [u32; 4]| {
        {
            let mut s = realm.social.elemental.lock();
            s.started = true;
            s.stages = stages;
            s.next_hour = 0;
            s.rest_until = None;
        }
        crate::game::elemental::sweep(&realm);
        realm.social.elemental.lock().stages
    };
    // Thistle Tea's `endured/1` over stages 1 to 6: [2, 3, 4, 5, 5, 6].
    assert_eq!(hour([1, 2, 3, 4]), [2, 3, 4, 5]);
    assert_eq!(
        hour([5, 6, 5, 6]),
        [5, 6, 5, 6],
        "the lord's stage and after"
    );
}

/// A world with Omen's cluster launcher beside the start, and Omen.
fn omen_world() -> GameData {
    let mut d = GameData::empty();
    d.gameobject_templates.insert(
        LAUNCHER,
        GameObjectTemplate {
            entry: LAUNCHER,
            kind: 22,
            display_id: 1,
            name: "Cluster Launcher".into(),
            size: 1.0,
            ..Default::default()
        },
    );
    d.gameobject_spawns.insert(
        0,
        vec![GameObjectSpawn {
            guid: 70,
            entry: LAUNCHER,
            map: 0,
            x: START.0 + 2.0,
            y: START.1 + 2.0,
            z: START.2,
            state: 1,
            ..Default::default()
        }],
    );
    d.creature_templates.insert(
        OMEN,
        CreatureTemplate {
            entry: OMEN,
            name: "Omen".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 60,
            level_max: 60,
            faction: 14,
            rank: 3,
            unit_class: 1,
            health_max: 500,
            speed_walk: 1.0,
            speed_run: 1.0,
            ..Default::default()
        },
    );
    d
}

fn minions_driven(realm: &crate::game::Realm) -> Option<bool> {
    realm
        .social
        .driven_events
        .lock()
        .get(&MINIONS_OF_OMEN)
        .copied()
}

#[tokio::test]
async fn omens_minions_stay_until_his_corpse_goes_and_he_rests_fifteen_minutes() {
    let realm = harness::realm_with(omen_world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let launcher = guid::game_object(LAUNCHER, 70);
    let launch = move |m: &mut crate::game::map::Map, n: usize| -> Vec<usize> {
        (0..n)
            .filter(|_| {
                let before = m.realm.social.omen.lock().omen;
                crate::game::map::world::use_object(m, g, launcher);
                before.is_none() && m.realm.social.omen.lock().omen.is_some()
            })
            .collect()
    };
    let called = map.query(move |m| launch(m, 20)).await.unwrap();
    assert_eq!(called, vec![19], "the twentieth calls Omen");
    let o = realm.social.omen.lock().omen.unwrap();
    // He falls: his minions stay with his corpse.
    map.query(move |m| crate::game::map::combat::kill(m, g, o))
        .await;
    {
        let w = realm.social.omen.lock();
        assert!(w.omen.is_none() && w.minions, "{w:?}");
        assert!(w.rests_until >= now() + 15 * 60 - 2);
    }
    assert_eq!(minions_driven(&realm), Some(true));
    // While he rests, twenty-five fireworks don't call him.
    assert!(map.query(move |m| launch(m, 25)).await.unwrap().is_empty());
    // His corpse goes: so do his minions.
    map.query(move |m| {
        let at = m.now;
        if let Some(c) = m.creature_mut(o)
            && let Life::Dead { corpse_until, .. } = &mut c.life
        {
            *corpse_until = at;
        }
        crate::game::map::ai::update_dead(m);
    })
    .await;
    assert!(!realm.social.omen.lock().minions);
    assert_eq!(minions_driven(&realm), Some(false));
    // Rested, the next firework calls him again.
    realm.social.omen.lock().rests_until = now() - 1;
    assert_eq!(map.query(move |m| launch(m, 1)).await.unwrap(), vec![0]);
}

#[tokio::test]
async fn omen_leaves_alive_after_two_hours_with_his_minions_and_may_be_called_at_once() {
    let realm = harness::realm_with(omen_world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let launcher = guid::game_object(LAUNCHER, 70);
    let launch = move |m: &mut crate::game::map::Map, n: usize| -> Vec<usize> {
        (0..n)
            .filter(|_| {
                let before = m.realm.social.omen.lock().omen;
                crate::game::map::world::use_object(m, g, launcher);
                before.is_none() && m.realm.social.omen.lock().omen.is_some()
            })
            .collect()
    };
    assert_eq!(map.query(move |m| launch(m, 20)).await.unwrap(), vec![19]);
    let o = realm.social.omen.lock().omen.unwrap();
    // An hour and more out: the elemental invaders' hour isn't his.
    let still = map
        .query(move |m| {
            let back = m.now.checked_sub(Duration::from_secs(3_700));
            if let Some(c) = m.creature_mut(o) {
                c.summoned_at = back.or(c.summoned_at);
            }
            m.rifts_checked_at = None;
            crate::game::elemental::update(m);
            m.creature(o).is_some()
        })
        .await
        .unwrap();
    assert!(still, "Omen stays past an hour");
    // His two hours up, he leaves.
    let gone = map
        .query(move |m| {
            let at = m.now;
            let s = crate::game::map::event_ai::scripted_mut(m, o).unwrap();
            assert_eq!(s.despawn_after, Some(Duration::from_secs(7_200)));
            s.despawn_at = Some(at);
            crate::game::map::event_ai::update_despawns(m);
            m.creature(o).is_none()
        })
        .await
        .unwrap();
    assert!(gone);
    {
        let w = realm.social.omen.lock();
        assert!(w.omen.is_none() && w.body.is_none() && !w.minions, "{w:?}");
    }
    assert_eq!(minions_driven(&realm), Some(false));
    // He never fell: twenty more call him again.
    assert_eq!(map.query(move |m| launch(m, 20)).await.unwrap(), vec![19]);
}

/// The invasion on, every zone attacked with its necropolises standing.
fn besieged(realm: &crate::game::Realm) {
    let mut s = realm.social.invasion.lock();
    s.enabled = true;
    for z in &ZONES {
        s.active.insert(z.zone);
        s.remaining.insert(z.zone, z.necropolises);
        s.attack_at.insert(z.zone, 0);
    }
    s.next_pass = 0;
}

#[test]
fn zones_still_holding_necropolises_fight_on() {
    let realm = harness::realm();
    besieged(&realm);
    invasion::sweep(&realm);
    let s = realm.social.invasion.lock();
    assert_eq!(s.active.len(), 6);
    assert_eq!(s.victories, 0);
    for z in &ZONES {
        assert_eq!(s.remaining[&z.zone], z.necropolises);
        assert_eq!(s.attack_at[&z.zone], 0, "zone {}", z.zone);
    }
}

#[test]
fn a_resting_zone_stays_clear_until_its_time_comes() {
    let realm = harness::realm();
    let t = now();
    {
        let mut s = realm.social.invasion.lock();
        s.enabled = true;
        s.victories = 1;
        for z in &ZONES {
            s.attack_at.insert(z.zone, t + 60);
        }
        s.next_pass = 0;
    }
    invasion::sweep(&realm);
    {
        let mut s = realm.social.invasion.lock();
        assert!(s.active.is_empty(), "{:?}", s.active);
        assert!(ZONES.iter().all(|z| s.attack_at[&z.zone] == t + 60));
        // Azshara's time comes.
        s.attack_at.insert(16, t - 1);
        s.next_pass = 0;
    }
    invasion::sweep(&realm);
    let s = realm.social.invasion.lock();
    assert_eq!(s.active.iter().copied().collect::<Vec<_>>(), vec![16]);
    assert_eq!(s.remaining[&16], 2);
}

#[test]
fn victories_open_the_argent_dawn_milestones_and_the_hundred_and_fiftieth_ends_the_invasion() {
    let milestones = |victories| {
        let s = invasion::State {
            enabled: true,
            victories,
            ..Default::default()
        };
        let mut on: Vec<u32> = invasion::driven(&s)
            .into_iter()
            .filter(|(e, on)| *on && (*e == invasion::INVASION || (96..=99).contains(e)))
            .map(|(e, _)| e)
            .collect();
        on.sort();
        on
    };
    assert_eq!(milestones(49), vec![17]);
    assert_eq!(milestones(50), vec![17, 96]);
    assert_eq!(milestones(100), vec![17, 97]);
    assert_eq!(milestones(150), vec![98, 99]);

    // Winterspring won for the hundred and fiftieth time: the invasion
    // ends, and the Scourge leave every zone.
    let realm = harness::realm();
    besieged(&realm);
    {
        let mut s = realm.social.invasion.lock();
        s.victories = 149;
        s.remaining.insert(618, 0);
    }
    invasion::sweep(&realm);
    assert_eq!(realm.social.invasion.lock().victories, 150);
    realm.social.invasion.lock().next_pass = 0;
    invasion::sweep(&realm);
    {
        let s = realm.social.invasion.lock();
        assert!(s.active.is_empty(), "{:?}", s.active);
        assert!(s.remaining.values().all(|&n| n == 0));
    }
    let driven = realm.social.driven_events.lock().clone();
    assert_eq!(driven.get(&invasion::INVASION), Some(&false));
    assert_eq!(driven.get(&98), Some(&true));
    assert_eq!(driven.get(&99), Some(&true));
    assert!(ZONES.iter().all(|z| driven.get(&z.event) == Some(&false)));
    // And no zone falls again.
    realm.social.invasion.lock().next_pass = 0;
    invasion::sweep(&realm);
    assert!(realm.social.invasion.lock().active.is_empty());
}
