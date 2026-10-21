//! Upper Blackrock Spire's stadium, on the real world: Rend Blackhand's
//! gauntlet as mangos-classic's `instance_blackrock_spire` runs it. A
//! player into area trigger 2026 starts it; seven waves come, each as soon
//! as the last is dead; then Gyth, who throws Rend off low on health; both
//! dead, the event is won. A wave's creature leaving its fight fails it,
//! and the next player in starts it over.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use crate::game::fields::*;
use crate::game::map::ubrs_stadium as st;
use crate::game::map::{Map, ai, event_ai, instance_script};

/// Inside the stadium's trigger, on its floor.
const STADIUM: Vector3d = Vector3d {
    x: 153.763,
    y: -419.833,
    z: 110.472,
};

fn door_state(m: &Map, entry: u32) -> Option<u32> {
    m.gameobjects
        .iter()
        .find(|(_, o)| o.entry == entry)
        .map(|(_, o)| o.fields.get_u32(GAMEOBJECT_STATE))
}

/// The event's summons alive: (entry, guid).
fn wave(m: &Map) -> Vec<(u32, Guid)> {
    let mut v: Vec<(u32, Guid)> = m
        .creatures
        .iter()
        .filter(|(_, c)| {
            c.alive()
                && c.spawn_guid.is_none()
                && matches!(c.entry, st::WHELP | st::DRAGONSPAWN | st::HANDLER)
        })
        .map(|(_, c)| (c.entry, c.guid))
        .collect();
    v.sort();
    v
}

fn summoned_alive(m: &Map, entry: u32) -> Vec<Guid> {
    m.creatures
        .iter()
        .filter(|(_, c)| c.alive() && c.spawn_guid.is_none() && c.entry == entry)
        .map(|(_, c)| c.guid)
        .collect()
}

fn balcony(m: &Map, entry: u32) -> Option<Guid> {
    m.creatures
        .iter()
        .find(|(_, c)| c.entry == entry && c.spawn_guid.is_some() && c.alive())
        .map(|(_, c)| c.guid)
}

/// A player as a player, not a GM (whom the trigger ignores).
async fn not_gm(map: &crate::game::map::MapHandle, g: Guid) {
    map.query(move |m| m.players.get_mut(&g).unwrap().gm = false)
        .await;
}

/// The next of the copy's timers due now.
fn hurry_all(m: &mut Map) {
    hurry_copy(m, Duration::from_secs(61));
}

async fn heard_text(
    realm: &std::sync::Arc<crate::game::Realm>,
    c: &mut crate::game::harness::Client,
    speaker: Guid,
    text: u32,
) -> bool {
    let line = realm
        .data
        .npc
        .broadcast_texts
        .get(&text)
        .map(|b| b.male.clone())
        .unwrap_or_default();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match heard(c, speaker, left).await {
            Some(l) if l == line => return true,
            Some(_) => continue,
            None => return false,
        }
    }
    false
}

/// The whole gauntlet: started by the trigger, its seven waves, Gyth and
/// Rend, and the gates.
#[tokio::test]
#[ignore = "extended"]
async fn rends_gauntlet_runs_as_mangos_classic() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = hero_in(&realm, "Rendslayer", 229, STADIUM, 0.0, &[]).await;
    not_gm(&map, me).await;
    let nefarius = map
        .query(|m| balcony(m, st::NEFARIUS))
        .await
        .flatten()
        .expect("Nefarius on the balcony");
    let rend = map
        .query(|m| balcony(m, st::REND))
        .await
        .flatten()
        .expect("Rend on the balcony");
    assert_eq!(
        map.query(|m| door_state(m, st::ENTRY_DOOR)).await.flatten(),
        Some(0),
        "the entry gate stands open"
    );
    c.send(CMSG_AREATRIGGER {
        trigger_id: st::AT_STADIUM,
    })
    .await;
    assert!(
        heard_text(&realm, &mut c, nefarius, st::SAY_INTRO_1).await,
        "Nefarius opens the show"
    );
    assert!(
        until(&map, 2000, |m| instance_script::get_data(m, st::STADIUM)
            == instance_script::IN_PROGRESS)
        .await
    );
    assert_eq!(
        map.query(|m| door_state(m, st::ENTRY_DOOR)).await.flatten(),
        Some(1),
        "the entry gate shuts behind them"
    );
    // Rend can't be struck on the balcony.
    assert!(
        map.query(move |m| unit_flags(m, rend) & 0x100 != 0)
            .await
            .unwrap()
    );
    // Seven seconds on, his second line; five more, the spectators and,
    // a second after, the first wave.
    map.query(hurry_all).await;
    assert!(heard_text(&realm, &mut c, nefarius, st::SAY_INTRO_2).await);
    map.query(hurry_all).await;
    assert!(
        until(&map, 2000, |m| m
            .creatures
            .iter()
            .filter(|(_, c)| c.alive() && c.spawn_guid.is_none() && matches!(c.entry, 9819 | 10317))
            .count()
            == 12)
        .await,
        "twelve spectators come to the balcony"
    );
    map.query(hurry_all).await;
    for (i, expected) in st::WAVES.iter().enumerate() {
        let mut want: Vec<u32> = expected.to_vec();
        want.sort();
        assert!(
            until(&map, 2000, move |m| {
                wave(m).iter().map(|(e, _)| *e).collect::<Vec<_>>() == want
            })
            .await,
            "wave {i}: {expected:?}"
        );
        // Killed at once, the next comes without waiting.
        map.query(move |m| {
            for (_, g) in wave(m) {
                kill(m, me, g);
            }
        })
        .await;
    }
    // The seventh dead: Nefarius loses patience, Rend answers.
    assert!(heard_text(&realm, &mut c, nefarius, st::SAY_LOSE).await);
    assert_eq!(
        map.query(|m| m.instance.scratch.get(&st::WAVES_SENT).copied())
            .await
            .flatten(),
        Some(st::MAX_WAVES + 1)
    );
    map.query(hurry_all).await;
    assert!(heard_text(&realm, &mut c, rend, st::SAY_REND_ATTACK).await);
    map.query(hurry_all).await;
    assert!(heard_text(&realm, &mut c, nefarius, st::SAY_WARCHIEF).await);
    // Rend rides off, gone five seconds on.
    assert!(
        until(&map, 7000, move |m| m
            .creature(rend)
            .is_none_or(|c| !c.alive()))
        .await,
        "Rend leaves the balcony"
    );
    // Thirty seconds on, Gyth.
    map.query(hurry_all).await;
    let gyth = {
        let mut g = None;
        for _ in 0..80 {
            g = map
                .query(|m| summoned_alive(m, st::GYTH).first().copied())
                .await
                .flatten();
            if g.is_some() {
                break;
            }
            sleep_ms(25).await;
        }
        g.expect("Gyth comes")
    };
    c.sees(gyth).await;
    map.query(move |m| {
        tough(m, gyth);
        ai::attacked(m, gyth, me);
    })
    .await;
    // Low on health he throws Rend off, at mangos-classic's 11 percent:
    // Rend, now a foe to strike.
    map.query(move |m| {
        set_pct(m, gyth, 15);
        if let Some(s) = event_ai::scripted_mut(m, gyth) {
            s.next_tick = None;
        }
        event_ai::tick(m, gyth);
    })
    .await;
    sleep_ms(300).await;
    assert!(
        map.query(|m| summoned_alive(m, st::REND).is_empty())
            .await
            .unwrap(),
        "not yet at 15 percent"
    );
    map.query(move |m| {
        set_pct(m, gyth, 10);
        if let Some(s) = event_ai::scripted_mut(m, gyth) {
            s.next_tick = None;
        }
        event_ai::tick(m, gyth);
    })
    .await;
    let thrown = {
        let mut g = None;
        for _ in 0..80 {
            g = map
                .query(|m| summoned_alive(m, st::REND).first().copied())
                .await
                .flatten();
            if g.is_some() {
                break;
            }
            sleep_ms(25).await;
        }
        g.expect("Rend thrown off Gyth")
    };
    assert!(
        until(&map, 2000, move |m| unit_flags(m, thrown) & 0x300 == 0).await,
        "Rend thrown is a foe to strike"
    );
    // Gyth dead: not yet won.
    map.query(move |m| kill(m, me, gyth)).await;
    assert_eq!(
        map.query(|m| instance_script::get_data(m, st::STADIUM))
            .await,
        Some(instance_script::IN_PROGRESS)
    );
    map.query(move |m| kill(m, me, thrown)).await;
    assert!(heard_text(&realm, &mut c, nefarius, st::SAY_VICTORY).await);
    assert_eq!(
        map.query(|m| instance_script::get_data(m, st::STADIUM))
            .await,
        Some(instance_script::DONE),
        "won"
    );
    assert_eq!(
        map.query(|m| door_state(m, st::EXIT_DOOR)).await.flatten(),
        Some(0),
        "the way on opens"
    );
    assert_eq!(
        map.query(|m| door_state(m, st::ENTRY_DOOR)).await.flatten(),
        Some(0),
        "and the way back"
    );
    // Won, the trigger starts nothing.
    c.send(CMSG_AREATRIGGER {
        trigger_id: st::AT_STADIUM,
    })
    .await;
    sleep_ms(200).await;
    assert_eq!(
        map.query(|m| instance_script::get_data(m, st::STADIUM))
            .await,
        Some(instance_script::DONE)
    );
}

/// A wave's creature that leaves its fight fails the event: Nefarius and
/// Rend leave the balcony, the gate opens, and the next player in starts
/// it again, the pair back.
#[tokio::test]
#[ignore = "extended"]
async fn rends_gauntlet_fails_on_an_evade_and_starts_again() {
    let Some(realm) = realm() else {
        return;
    };
    let (c, me, map) = hero_in(&realm, "Rendfailer", 229, STADIUM, 0.0, &[]).await;
    not_gm(&map, me).await;
    c.send(CMSG_AREATRIGGER {
        trigger_id: st::AT_STADIUM,
    })
    .await;
    assert!(
        until(&map, 2000, |m| instance_script::get_data(m, st::STADIUM)
            == instance_script::IN_PROGRESS)
        .await
    );
    for _ in 0..3 {
        map.query(hurry_all).await;
    }
    assert!(
        until(&map, 2000, |m| wave(m).len() == 4).await,
        "the first wave"
    );
    let one = map.query(|m| wave(m)[0].1).await.unwrap();
    map.query(move |m| crate::game::map::event_ai::evade(m, one))
        .await;
    assert_eq!(
        map.query(|m| instance_script::get_data(m, st::STADIUM))
            .await,
        Some(instance_script::FAILED)
    );
    assert!(
        map.query(|m| balcony(m, st::NEFARIUS).is_none() && balcony(m, st::REND).is_none())
            .await
            .unwrap(),
        "Nefarius and Rend gone"
    );
    assert_eq!(
        map.query(|m| door_state(m, st::ENTRY_DOOR)).await.flatten(),
        Some(0),
        "the gate opens again"
    );
    c.send(CMSG_AREATRIGGER {
        trigger_id: st::AT_STADIUM,
    })
    .await;
    assert!(
        until(&map, 2000, |m| instance_script::get_data(m, st::STADIUM)
            == instance_script::IN_PROGRESS
            && balcony(m, st::NEFARIUS).is_some()
            && balcony(m, st::REND).is_some())
        .await,
        "started again, the pair back"
    );
}
