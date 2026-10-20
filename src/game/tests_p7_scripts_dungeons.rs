//! Round 7's creature script tests, continued: Gnomeregan's Emi Shortfuse
//! and Thermaplugg, Dire Maul's Mizzle and Stratholme's Balnazzar, on the
//! real world as a player's client sees them.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use super::tests_p3_scripts::*;
use super::tests_p4_scripts_dungeons::{data, inside, set};
use super::tests_p6_scripts::*;
use super::tests_p7_scripts::*;
use super::tests_p7_scripts_quests::{menu_of, until_some};
use crate::game::map::assistance::Reaction;
use crate::game::map::instance_script::{DONE, FAILED, IN_PROGRESS, NOT_STARTED};
use crate::game::map::{Map, MapHandle, event_ai, map_events};

const EMI: u32 = 7998;
const GRUBBIS: u32 = 7361;
const CHOMPER: u32 = 6215;
const TROGG: u32 = 6207;
const THERMAPLUGG: u32 = 7800;
const BEGIN: &str = "I am ready to begin.";

/// The troggs a script brought here, alive.
fn troggs(m: &Map) -> Vec<Guid> {
    m.creatures
        .iter()
        .filter(|(_, c)| c.entry == TROGG && c.spawn_guid.is_none() && c.alive())
        .map(|(_, c)| c.guid)
        .collect()
}

fn reaction(m: &Map, g: Guid) -> Option<Reaction> {
    event_ai::scripted(m, g).and_then(|s| s.reaction)
}

/// How long `g` waits where it stands, if it waits for a time.
fn waits(m: &Map, g: Guid) -> Option<f32> {
    match m.creature(g).map(|c| &c.motion) {
        Some(crate::game::map::creature::Motion::Idle { until: Some(t) }) => {
            Some(t.saturating_duration_since(m.now).as_secs_f32())
        }
        _ => None,
    }
}

/// Emi's event begun at her word: the client, the player, the copy, Emi.
async fn begun(name: &str) -> Option<(crate::game::harness::Client, Guid, MapHandle, Guid)> {
    let realm = realm()?;
    let (mut c, me, map) = inside(&realm, name, 90, EMI).await;
    let emi = map.query(|m| first(m, EMI)).await.flatten().unwrap();
    c.sees(emi).await;
    let (_, options) = menu_of(&mut c, emi).await;
    let id = options
        .iter()
        .find(|(_, t)| t == BEGIN)
        .map(|(i, _)| *i)
        .expect("Emi offers to begin");
    choose(&c, emi, id).await;
    assert!(
        until(&map, 2_000, move |m| map_events::active(m, EMI)
            && data(m, 0) == IN_PROGRESS)
        .await,
        "Grubbis's event under way"
    );
    let _ = me;
    Some((c, me, map, emi))
}

/// Emi offers to begin while Grubbis is not beaten (not begun, or
/// failed), and not while the event runs or once it is done. At her word
/// the event starts; a second on she says her piece, turns escortee and
/// offers nothing, and nine and a half seconds on sets out (Thistle Tea's
/// `gnomeregan` gossip).
#[tokio::test]
#[ignore = "extended"]
async fn emi_offers_to_begin_while_grubbis_is_not_beaten() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, _me, map) = inside(&realm, "Asker", 90, EMI).await;
    let emi = map.query(|m| first(m, EMI)).await.flatten().unwrap();
    c.sees(emi).await;
    for (state, offered) in [
        (NOT_STARTED, true),
        (IN_PROGRESS, false),
        (FAILED, true),
        (DONE, false),
    ] {
        map.query(move |m| set(m, 0, state)).await;
        let (_, options) = menu_of(&mut c, emi).await;
        assert_eq!(
            options.iter().any(|(_, t)| t == BEGIN),
            offered,
            "at {state}"
        );
    }
    drop(c);
    let Some((mut c, _me, map, emi)) = begun("Beginner").await else {
        return;
    };
    let realm = map.query(|m| m.realm.clone()).await.unwrap();
    hurry_all(&map, 1.0).await;
    assert!(hears(&mut c, &realm, emi, 4050).await, "her piece");
    assert_eq!(
        map.query(move |m| (faction(m, emi), npc_flags(m, emi)))
            .await,
        Some((113, 0)),
        "an escortee, offering nothing"
    );
    assert!(
        map.query(move |m| event_ai::scripted(m, emi).is_none_or(|s| s.route.is_none()))
            .await
            .unwrap(),
        "not yet on her way"
    );
    hurry_all(&map, 8.5).await;
    assert!(
        until(&map, 2_000, move |m| event_ai::phase(m, emi) == 1
            && event_ai::scripted(m, emi)
                .and_then(|s| s.route.as_ref())
                .is_some_and(|r| r.points.len() == 19))
        .await,
        "on her way"
    );
}

/// Emi's walk: she holds at each scene (five seconds at the first, until
/// it plays out at the others). At the cave-in the troggs pour in toward
/// it as it opens, and she goes on; she won't fight while she plants a
/// charge, which stands once planted; at the southern tunnel she blows it
/// shut before opening the northern one; at the last, Grubbis and Chomper
/// come up from below. Grubbis's death sets off the northern charges and
/// the fireworks, and the event ends well (Thistle Tea's `gnomeregan`
/// routes and events).
#[tokio::test]
#[ignore = "extended"]
async fn emi_holds_at_each_scene_and_grubbis_falls() {
    let Some((mut c, me, map, emi)) = begun("Shortfuse").await else {
        return;
    };
    let realm = map.query(|m| m.realm.clone()).await.unwrap();
    // She lasts out the troggs' blows.
    map.query(move |m| {
        tough(m, emi);
        if let Some(s) = event_ai::scripted_mut(m, emi) {
            s.invincible_hp = Some(1);
        }
    })
    .await;
    hurry_all(&map, 9.5).await;
    assert!(
        until(&map, 2_000, move |m| event_ai::scripted(m, emi)
            .is_some_and(|s| s.route.is_some()))
        .await,
        "on her way"
    );
    // Five seconds at the first scene.
    map.query(move |m| bring(m, me, emi, 3)).await;
    assert!(until(&map, 3_000, move |m| reached(m, emi, 3)).await);
    let w = map.query(move |m| waits(m, emi)).await.flatten();
    assert!(w.is_some_and(|w| (4.0..=5.0).contains(&w)), "{w:?}");
    // The cave-in: held until it plays out.
    map.query(move |m| bring(m, me, emi, 8)).await;
    assert!(until(&map, 3_000, move |m| reached(m, emi, 8)).await);
    let w = map.query(move |m| waits(m, emi)).await.flatten();
    assert!(w.is_some_and(|w| w > 1000.0), "held: {w:?}");
    let before = map.query(|m| troggs(m).len()).await.unwrap();
    hurry_all(&map, 27.0).await;
    let (pour, open, on) = map
        .query(move |m| {
            let t = troggs(m);
            let moving = t
                .iter()
                .filter(|g| event_ai::scripted(m, **g).is_some_and(|s| s.scripted_move.is_some()))
                .count();
            (
                (t.len() - before, moving),
                data(m, 3),
                waits(m, emi).is_none_or(|w| w < 1.0),
            )
        })
        .await
        .unwrap();
    assert_eq!(pour.0, 9, "nine troggs");
    assert!(pour.1 >= 7, "pouring in: {pour:?}");
    assert_eq!(open, 1, "the cave-in open");
    assert!(on, "and she goes on");
    // A charge.
    map.query(move |m| bring(m, me, emi, 10)).await;
    assert!(until(&map, 3_000, move |m| reached(m, emi, 10)).await);
    assert_eq!(
        map.query(move |m| reaction(m, emi)).await.flatten(),
        Some(Reaction::Passive),
        "she won't fight while she plants it"
    );
    hurry_all(&map, 15.0).await;
    assert!(
        until(&map, 2_000, move |m| data(m, 2) == 1
            && reaction(m, emi) == Some(Reaction::Aggressive))
        .await,
        "planted, and she fights again"
    );
    // The southern tunnel blown before the northern opened.
    c.clear();
    map.query(move |m| bring(m, me, emi, 14)).await;
    assert!(until(&map, 3_000, move |m| reached(m, emi, 14)).await);
    map.query(move |m| set(m, 3, 1)).await;
    hurry_all(&map, 11.0).await;
    assert!(
        cast_any_of(&mut c, emi, 12159, Duration::from_secs(2))
            .await
            .is_some(),
        "the blast"
    );
    hurry_all(&map, 0.5).await;
    assert_eq!(
        map.query(|m| (data(m, 3), data(m, 4))).await,
        Some((0, 0)),
        "the south shut, the north not yet open"
    );
    hurry_all(&map, 31.0).await;
    assert!(
        until(&map, 2_000, |m| data(m, 4) == 1).await,
        "then the north opened"
    );
    // The last scene: Grubbis and Chomper (the troggs put down, or she
    // stops to fight them).
    c.clear();
    map.query(move |m| {
        for g in troggs(m) {
            kill(m, me, g);
        }
        crate::game::map::combat::leave_combat(m, emi);
        bring(m, me, emi, 18)
    })
    .await;
    assert!(
        until(&map, 3_000, move |m| reached(m, emi, 18)).await,
        "at the last scene: {:?}",
        map.query(move |m| m.creature(emi).map(|c| (
            c.position,
            c.movement_type,
            c.waypoint,
            c.last_waypoint,
            c.combat.attacking,
            c.evading,
            format!("{:?}", c.motion)
                .chars()
                .take(200)
                .collect::<String>(),
            c.alive(),
        )))
        .await
    );
    hurry_all(&map, 9.0).await;
    let grubbis = until_some(&map, 2_000, |m| first(m, GRUBBIS))
        .await
        .expect("Grubbis comes");
    assert!(
        map.query(|m| first(m, CHOMPER)).await.flatten().is_some(),
        "and Chomper"
    );
    assert_eq!(map.query(move |m| event_ai::phase(m, emi)).await, Some(2));
    assert!(hears(&mut c, &realm, grubbis, 4328).await, "Grubbis below");
    // He falls.
    c.clear();
    map.query(move |m| kill(m, me, grubbis)).await;
    assert!(
        until(&map, 2_000, move |m| event_ai::phase(m, emi) == 3).await,
        "Emi hears of it"
    );
    hurry_all(&map, 14.0).await;
    assert!(
        cast_any_of(&mut c, emi, 12158, Duration::from_secs(2))
            .await
            .is_some(),
        "the northern charges"
    );
    hurry_all(&map, 8.5).await;
    assert!(
        cast_any_of(&mut c, emi, 11542, Duration::from_secs(2))
            .await
            .is_some(),
        "the fireworks"
    );
    assert!(hears(&mut c, &realm, emi, 4446).await);
    assert!(
        until(&map, 2_000, |m| !map_events::active(m, EMI)
            && data(m, 0) == DONE)
        .await,
        "the event over, well"
    );
}

/// Emi's death fails Grubbis's event.
#[tokio::test]
#[ignore = "extended"]
async fn emis_death_fails_the_event() {
    let Some((_c, me, map, emi)) = begun("Mourner").await else {
        return;
    };
    map.query(move |m| kill(m, me, emi)).await;
    assert!(
        until(&map, 2_000, |m| !map_events::active(m, EMI)
            && data(m, 0) == FAILED)
        .await,
        "failed"
    );
}

/// The event of `entry`'s that casts `spell` first.
fn casting(m: &Map, entry: u32, spell: u32) -> u32 {
    let data = &m.realm.data.scripts;
    data.ai_events[&entry]
        .iter()
        .find(|e| {
            data.ai_scripts
                .get(&e.actions[0])
                .and_then(|s| s.first())
                .is_some_and(|s| s.command == 15 && s.datalong[0] == spell)
        })
        .map(|e| e.id)
        .unwrap_or_else(|| panic!("{entry} casts {spell}"))
}

/// Thermaplugg knocks his target away in his first half, and everyone
/// about him from half health (calling it out), back to the first as he
/// gives up; and each bomb he sets off opens one of the six faces at
/// random, on its timer.
#[tokio::test]
#[ignore = "extended"]
async fn thermaplugg_knocks_away_and_his_bombs_open_the_faces() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = inside(&realm, "Plugger", 90, THERMAPLUGG).await;
    let t = map
        .query(|m| first(m, THERMAPLUGG))
        .await
        .flatten()
        .unwrap();
    c.sees(t).await;
    map.query(move |m| crate::game::map::ai::attacked(m, t, me))
        .await;
    let (knock, all_knock, bomb) = map
        .query(move |m| {
            (
                casting(m, THERMAPLUGG, 10101),
                casting(m, THERMAPLUGG, 11130),
                casting(m, THERMAPLUGG, 11511),
            )
        })
        .await
        .unwrap();
    let cast_now =
        async |c: &mut crate::game::harness::Client, id: u32, spell: u32, within: u64| {
            c.clear();
            map.query(move |m| {
                if let Some(cr) = m.caster_mut(t) {
                    cr.gcd_until = None;
                }
                due(m, t, id);
            })
            .await;
            let seen = cast_any_of(c, t, spell, Duration::from_millis(within)).await;
            map.query(move |m| crate::game::map::spells::interrupt(m, t))
                .await;
            seen
        };
    assert_eq!(
        cast_now(&mut c, knock, 10101, 3_000).await,
        Some(me),
        "at his target"
    );
    assert!(
        cast_now(&mut c, all_knock, 11130, 600).await.is_none(),
        "not everyone, yet"
    );
    // Half health.
    c.clear();
    map.query(move |m| {
        set_pct(m, t, 50);
        if let Some(s) = event_ai::scripted_mut(m, t) {
            s.next_tick = None;
        }
        event_ai::tick(m, t);
    })
    .await;
    assert!(hears(&mut c, &realm, t, 6174).await, "he calls it");
    assert_eq!(map.query(move |m| event_ai::phase(m, t)).await, Some(1));
    assert!(
        cast_now(&mut c, all_knock, 11130, 3_000).await.is_some(),
        "everyone about him"
    );
    assert!(
        cast_now(&mut c, knock, 10101, 600).await.is_none(),
        "no more his target alone"
    );
    // His bombs: each opens a face, at random.
    let mut opened = std::collections::BTreeSet::new();
    for _ in 0..8 {
        let (before, after, next) = map
            .query(move |m| {
                let faces = |m: &Map| (10..=15).filter(|f| data(m, *f) == 1).collect::<Vec<_>>();
                for f in 10..=15 {
                    set(m, f, 0);
                }
                let before = faces(m);
                event_ai::set_phase(m, t, 0);
                if let Some(cr) = m.caster_mut(t) {
                    cr.gcd_until = None;
                }
                due(m, t, bomb);
                hurry_runs(m, Some(t), Duration::from_secs(5));
                (before, faces(m), left(m, t, bomb))
            })
            .await
            .unwrap();
        assert!(before.is_empty());
        assert_eq!(after.len(), 1, "one face opened: {after:?}");
        assert!(between(next, 12.0, 17.0), "again in 12 to 17 s: {next:?}");
        opened.extend(after);
        map.query(move |m| crate::game::map::spells::interrupt(m, t))
            .await;
    }
    assert!(opened.len() > 1, "at random: {opened:?}");
    // He gives up: his first half again.
    map.query(move |m| {
        event_ai::set_phase(m, t, 1);
        crate::game::map::ai::enter_evade(m, t);
    })
    .await;
    assert!(
        until(&map, 2_000, move |m| event_ai::phase(m, t) == 0).await,
        "back to his first half"
    );
}

const KING_GORDOK: u32 = 11501;
const MIZZLE: u32 = 14353;

/// The king fallen, Mizzle hails the new one, walks back to the throne,
/// and only there offers to talk (Thistle Tea's `dire_maul`; AI5_GAPS
/// D25, a script's walk home arriving).
#[tokio::test]
#[ignore = "extended"]
async fn mizzle_hails_the_new_king_and_talks_only_at_the_throne() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = inside(&realm, "Kingslayer", 429, KING_GORDOK).await;
    c.clear();
    map.query(move |m| {
        for g in living(m, KING_GORDOK) {
            kill(m, me, g);
        }
    })
    .await;
    let mizzle = until_some(&map, 2_000, |m| first(m, MIZZLE))
        .await
        .expect("Mizzle comes");
    assert!(
        hears(&mut c, &realm, mizzle, 9348).await,
        "he hails the king"
    );
    let throne = Vector3d {
        x: 816.3,
        y: 481.8,
        z: 37.3,
    };
    let (home, flags) = map
        .query(move |m| (m.creature(mizzle).map(|c| c.home), npc_flags(m, mizzle)))
        .await
        .unwrap();
    assert!(
        home.is_some_and(|h| crate::game::map::combat::dist(h, throne) < 1.0),
        "his home the throne: {home:?}"
    );
    assert_eq!(flags & 0x1, 0, "not talking on his way");
    c.clear();
    assert!(
        until(&map, 8_000, move |m| {
            // Waiting for him there (a copy sees 170 yards).
            stand(m, me, off(throne, 3.0, 0.0));
            hasten(m, mizzle);
            npc_flags(m, mizzle) & 0x1 != 0
        })
        .await,
        "at the throne he offers to talk"
    );
    assert!(
        map.query(move |m| m
            .position_of(mizzle)
            .is_some_and(|p| crate::game::map::combat::dist(p, throne) < 3.0))
            .await
            .unwrap(),
        "there"
    );
    assert!(hears(&mut c, &realm, mizzle, 9411).await, "and says so");
}

const DATHROHAN: u32 = 10812;
const BALNAZZAR: u32 = 10813;

/// Balnazzar's shadow magic, each on its vmangos timer, sending itself
/// again: Mind Blast at his victim every 11 seconds, Sleep at an attacker
/// other than his top every 15, Psychic Scream every 20, and Domination
/// at his second-highest threat every 25 to 30 (Thistle Tea's
/// `stratholme`).
#[tokio::test]
#[ignore = "extended"]
async fn balnazzars_shadow_magic_repeats_on_its_timers() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = inside(&realm, "Purifier", 329, DATHROHAN).await;
    let d = map.query(|m| first(m, DATHROHAN)).await.flatten().unwrap();
    let at = map
        .query(move |m| m.position_of(d))
        .await
        .flatten()
        .unwrap();
    let (_c2, second) = joins(&realm, "Second", &map, off(at, 3.0, 0.0)).await;
    c.sees(d).await;
    map.query(move |m| {
        crate::game::map::ai::attacked(m, d, me);
        crate::game::map::ai::add_threat(m, d, me, 10_000.0);
        crate::game::map::ai::add_threat(m, d, second, 5_000.0);
        tough(m, d);
        set_pct(m, d, 39);
        if let Some(s) = event_ai::scripted_mut(m, d) {
            s.next_tick = None;
        }
        event_ai::tick(m, d);
    })
    .await;
    assert!(
        until(&map, 3_000, move |m| m.creature(d).map(|c| c.entry)
            == Some(BALNAZZAR)
            && event_ai::phase(m, d) == 1)
        .await,
        "Balnazzar"
    );
    // His own sending, put by: each is sent here.
    map.query(move |m| crate::game::map::scripts::stop_for(m, d))
        .await;
    let ready = move |m: &mut Map| {
        crate::game::map::spells::interrupt(m, d);
        if let Some(cr) = m.caster_mut(d) {
            cr.gcd_until = None;
            cr.cooldowns.clear();
            cr.category_cooldowns.clear();
        }
    };
    // Each sends itself again at its soonest and latest: Domination picks
    // 25, 27.5 or 30 seconds (a third each); the rest have one time.
    for (event, spell, at, soonest, latest) in [
        (1, 17399, Some(me), 11.0, 11.0),
        (2, 12098, Some(second), 15.0, 15.0),
        (3, 13704, None, 20.0, 20.0),
        (4, 17405, Some(second), 25.0, 30.0),
    ] {
        c.clear();
        map.query(move |m| {
            ready(m);
            event_ai::script_event(m, d, event, 0, None);
        })
        .await;
        let aimed = cast_any_of(&mut c, d, spell, Duration::from_secs(3)).await;
        assert!(aimed.is_some(), "{spell} cast");
        if let Some(at) = at {
            assert_eq!(aimed, Some(at), "{spell} aimed");
        }
        // Not before its time; by its latest, again. (The hurry to 27
        // seconds the test had sent Domination at once a third of the time.)
        c.clear();
        map.query(move |m| {
            ready(m);
            hurry_runs(m, Some(d), Duration::from_secs_f32(soonest - 3.0));
        })
        .await;
        assert!(
            cast_any_of(&mut c, d, spell, Duration::from_millis(300))
                .await
                .is_none(),
            "{spell} not yet"
        );
        map.query(move |m| {
            ready(m);
            hurry_runs(m, Some(d), Duration::from_secs_f32(latest - soonest + 3.0));
        })
        .await;
        assert!(
            cast_any_of(&mut c, d, spell, Duration::from_secs(3))
                .await
                .is_some(),
            "{spell} again {soonest} to {latest} seconds on"
        );
        map.query(move |m| crate::game::map::scripts::stop_for(m, d))
            .await;
    }
}
