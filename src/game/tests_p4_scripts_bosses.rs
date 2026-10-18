//! Thistle Tea's creature scripts for dungeon bosses this server imports
//! (`data/tt_scripts.rs`), on the real world: Stratholme's, Blackrock
//! Depths', Zul'Farrak's and the Wailing Caverns', each pulled in Molten
//! Core's empty arena, its events made due and what came of them seen.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use crate::game::Realm;
use crate::game::harness::Client;
use crate::game::map::{Map, MapHandle, ai, event_ai};

/// Ragnaros's lair, in a copy of Molten Core: nothing about.
const ARENA: Vector3d = Vector3d {
    x: 840.0,
    y: -812.0,
    z: -229.6,
};

/// A boss of `entry` `dx` yards east of the arena's middle, standing its
/// ground and lasting.
async fn boss(map: &MapHandle, entry: u32, dx: f32) -> Guid {
    map.query(move |m| {
        let g = summon(m, entry, off(ARENA, dx, 0.0), std::f32::consts::PI);
        tough(m, g);
        if let Some(s) = event_ai::scripted_mut(m, g) {
            s.no_combat_movement = true;
        }
        g
    })
    .await
    .unwrap()
}

async fn arena(realm: &Arc<Realm>, name: &str) -> (Client, Guid, MapHandle) {
    hero_in(realm, name, 409, ARENA, 120.0, &[]).await
}

/// The id of `entry`'s event whose first parameter is `p1` (and, if
/// given, whose kind is `kind`).
fn event_with(m: &Map, entry: u32, p1: i32) -> u32 {
    m.realm.data.scripts.ai_events[&entry]
        .iter()
        .find(|e| e.params[0] == p1)
        .map(|e| e.id)
        .unwrap_or_else(|| panic!("{entry}: an event at {p1}"))
}

/// The living creatures of `entry` that `of` summoned.
fn summoned_by(m: &Map, of: Guid, entry: u32) -> Vec<Guid> {
    living(m, entry)
        .into_iter()
        .filter(|g| event_ai::scripted(m, *g).and_then(|s| s.summoner) == Some(of))
        .collect()
}

/// Nerub'enkan raises a swarm of four, six or eight crypt scarabs, or one
/// undead scarab, about him (ten yards), each set on his attacker. A crypt
/// scarab's own EventAI (the world database's, which Thistle Tea runs too)
/// despawns it once within five yards of its victim: from beside him, a
/// swarm is gone as it reaches one, which is why it seemed none came.
#[tokio::test]
#[ignore = "extended"]
async fn nerubenkan_raises_his_scarabs() {
    const NERUBENKAN: u32 = 10437;
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = arena(&realm, "Scarabs").await;
    // Thirty yards off, so a swarm doesn't reach his attacker at once.
    let g = boss(&map, NERUBENKAN, 30.0).await;
    map.query(move |m| ai::attacked(m, g, me)).await;
    let id = map
        .query(move |m| event_with(m, NERUBENKAN, 3000))
        .await
        .unwrap();
    // His events started (at the update after his spawn and pull).
    assert!(until(&map, 2000, move |m| left(m, g, id).is_some()).await);
    let mut seen = std::collections::BTreeSet::new();
    for round in 0..8 {
        let n = map
            .query(move |m| {
                if !fighting(m, g) {
                    ai::attacked(m, g, me);
                }
                crate::game::map::spells::interrupt(m, g);
                if let Some(cr) = m.caster_mut(g) {
                    cr.gcd_until = None;
                }
                due(m, g, id);
                let crypt = summoned_by(m, g, 10577);
                let undead = summoned_by(m, g, 10876);
                let near = crypt.iter().chain(&undead).all(|s| {
                    m.position_of(*s)
                        .zip(m.position_of(g))
                        .is_some_and(|(a, b)| crate::game::map::combat::dist(a, b) <= 10.5)
                });
                assert!(near, "about him");
                (crypt.len(), undead.len())
            })
            .await
            .unwrap();
        assert!(
            matches!(n, (4 | 6 | 8, 0) | (0, 1)),
            "round {round}: {n:?} scarabs"
        );
        seen.insert(n);
        let at_me = until(&map, 1000, move |m| {
            let all: Vec<Guid> = summoned_by(m, g, 10577)
                .into_iter()
                .chain(summoned_by(m, g, 10876))
                .collect();
            !all.is_empty() && all.iter().all(|s| victim(m, *s) == me)
        })
        .await;
        assert!(at_me, "round {round}: set on his attacker");
        // A crypt scarab that reaches him goes.
        if n.0 > 0 {
            // One brought beside him (again at each look, as it moves
            // on its own).
            assert!(
                until(&map, 5000, move |m| {
                    let left = summoned_by(m, g, 10577);
                    if left.len() < n.0 {
                        return true;
                    }
                    let at = m.position_of(me).unwrap();
                    if let Some(c) = m.creature_mut(left[0]) {
                        c.position = off(at, 2.0, 0.0);
                    }
                    false
                })
                .await,
                "a crypt scarab gone as it reaches its victim"
            );
        }
        map.query(move |m| {
            for e in [10577, 10876] {
                for s in summoned_by(m, g, e) {
                    crate::game::map::scripts::despawn(m, s);
                }
            }
        })
        .await;
    }
    eprintln!("nerub'enkan's swarms: {seen:?}");
}

/// Doom'rel calls his voidwalkers once, below half his health.
#[tokio::test]
#[ignore = "extended"]
async fn doomrel_calls_his_voidwalkers_at_half_health() {
    const DOOMREL: u32 = 9039;
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = arena(&realm, "Doomrel").await;
    let g = boss(&map, DOOMREL, 5.0).await;
    c.sees(g).await;
    map.query(move |m| ai::attacked(m, g, me)).await;
    let id = map
        .query(move |m| {
            m.realm.data.scripts.ai_events[&DOOMREL]
                .iter()
                .find(|e| e.kind == crate::game::data::scripts::EVENT_HP && e.params[0] == 50)
                .map(|e| e.id)
                .expect("his half-health event")
        })
        .await
        .unwrap();
    // His events started, at the update after his pull.
    assert!(
        until(&map, 2000, move |m| event_ai::scripted(m, g)
            .is_some_and(|s| !s.timers.is_empty()))
        .await
    );
    c.clear();
    // Below half, in his fight (pulled again if a loaded machine let him
    // slip out of it): the event goes, once.
    assert!(
        until(&map, 5000, move |m| {
            if !spent(m, g, id) {
                if !fighting(m, g) {
                    ai::attacked(m, g, me);
                }
                hurt_to(m, g, 49);
            }
            spent(m, g, id)
        })
        .await,
        "his half-health event"
    );
    assert!(
        cast_of(&mut c, g, 15092, Duration::from_secs(3))
            .await
            .is_some(),
        "his voidwalkers"
    );
    assert!(map.query(move |m| spent(m, g, id)).await.unwrap(), "once");
}

/// Grimstone's points in the Ring of Law: at the first, seven seconds on
/// the beast gate's stage, then two packs of four of one kind each (ten and
/// twenty-nine seconds on); at the fourth, five seconds on the champion's
/// stage and, ten on, one of the six champions; at the fifth, the ring
/// won.
#[tokio::test]
#[ignore = "extended"]
async fn grimstone_calls_the_rings_packs_and_champion() {
    use super::tests_p4_scripts_dungeons::{data, inside};
    const GRIMSTONE: u32 = 10096;
    const BEASTS: [u32; 6] = [8925, 8926, 8927, 8928, 8932, 8933];
    const CHAMPIONS: [u32; 6] = [9027, 9028, 9029, 9030, 9031, 9032];
    let Some(realm) = realm() else {
        return;
    };
    let points: Vec<(u32, u32)> = realm
        .data
        .scripts
        .routes
        .iter()
        .filter(|((e, _), _)| *e == GRIMSTONE)
        .flat_map(|(_, p)| p.iter().map(|w| (w.point, w.script_id)))
        .filter(|(_, s)| *s != 0)
        .collect();
    let script_at = |n: u32| points.iter().find(|(p, _)| *p == n).unwrap().1;
    let (_c, me, map) = inside(&realm, "Grimstone", 230, 161523).await;
    let g = map
        .query(move |m| {
            let at = m.position_of(me).unwrap();
            summon(m, GRIMSTONE, at, 0.0)
        })
        .await
        .unwrap();
    let pack = move |m: &Map| -> Vec<u32> {
        BEASTS
            .iter()
            .flat_map(|e| summoned_by(m, g, *e).into_iter().map(|_| *e))
            .collect()
    };
    let (one, four, five) = (script_at(1), script_at(4), script_at(5));
    map.query(move |m| {
        crate::game::map::scripts::start(
            m,
            crate::game::data::scripts::ScriptTable::Movement,
            one,
            g,
            g,
        );
        // Its scripts, and those they start, on by half seconds.
        let on = |m: &mut Map, ms: u64| {
            for _ in 0..ms / 500 {
                hurry_runs(m, None, Duration::from_millis(500));
            }
        };
        on(m, 6500);
        assert_eq!(data(m, 46), 0);
        on(m, 1000);
        assert_eq!(data(m, 46), 1, "the beast gate's stage");
        on(m, 10_000);
        let first = pack(m);
        assert_eq!(first.len(), 4, "the first pack: {first:?}");
        assert!(first.iter().all(|e| *e == first[0]), "of one kind");
        on(m, 19_000);
        assert_eq!(pack(m).len(), 8, "the second");
        crate::game::map::scripts::start(
            m,
            crate::game::data::scripts::ScriptTable::Movement,
            four,
            g,
            g,
        );
        on(m, 5500);
        assert_eq!(data(m, 46), 2, "the champion's stage");
        on(m, 5000);
        let champions: Vec<u32> = CHAMPIONS
            .iter()
            .flat_map(|e| living(m, *e).into_iter().map(|_| *e))
            .collect();
        assert_eq!(champions.len(), 1, "one champion: {champions:?}");
        crate::game::map::scripts::start(
            m,
            crate::game::data::scripts::ScriptTable::Movement,
            five,
            g,
            g,
        );
        on(m, 500);
        assert_eq!(data(m, 0), 3, "the ring won");
    })
    .await
    .unwrap();
}

/// `g`'s timed event `id` made due, in its fight with `me` (pulled again if
/// a loaded machine let it slip out), out of any cast, until it goes: its
/// timer set anew.
async fn fire_in_fight(map: &MapHandle, g: Guid, me: Guid, id: u32) {
    assert!(
        until(map, 5000, move |m| {
            if !fighting(m, g) {
                ai::attacked(m, g, me);
            }
            crate::game::map::spells::interrupt(m, g);
            if let Some(cr) = m.caster_mut(g) {
                cr.gcd_until = None;
            }
            due(m, g, id);
            left(m, g, id).is_some_and(|t| t > Duration::from_millis(500))
        })
        .await,
        "event {id} goes"
    );
}

/// `g` at `pct` percent and its events looked at, out of any cast.
fn hurt_to(m: &mut Map, g: Guid, pct: u32) {
    set_pct(m, g, pct);
    crate::game::map::spells::interrupt(m, g);
    if let Some(s) = event_ai::scripted_mut(m, g) {
        s.next_tick = None;
    }
    event_ai::tick(m, g);
}

/// A boss of `entry` pulled by `me`, its events started.
async fn pulled(map: &MapHandle, entry: u32, me: Guid) -> Guid {
    let g = boss(map, entry, 5.0).await;
    map.query(move |m| ai::attacked(m, g, me)).await;
    assert!(
        until(map, 2000, move |m| event_ai::scripted(m, g)
            .is_some_and(|s| !s.timers.is_empty()))
        .await,
        "{entry}'s events started"
    );
    g
}

/// Stratholme's bosses: Timmy enrages once below a tenth; Ramstein's
/// Knockout takes his victim's threat; Dathrohan becomes Balnazzar at 40
/// percent and is Dathrohan again once he leaves the fight, and his death
/// raises a skeleton at each of 32 posts; Willey calls three riflemen.
#[tokio::test]
#[ignore = "extended"]
async fn stratholmes_bosses_fight_as_thistle_tea_scripts_them() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = arena(&realm, "Strath").await;
    // Timmy.
    let timmy = pulled(&map, 10808, me).await;
    c.sees(timmy).await;
    c.clear();
    map.query(move |m| hurt_to(m, timmy, 9)).await;
    assert!(
        cast_of(&mut c, timmy, 8599, Duration::from_secs(2))
            .await
            .is_some(),
        "Timmy enrages"
    );
    c.clear();
    map.query(move |m| hurt_to(m, timmy, 8)).await;
    assert!(
        cast_of(&mut c, timmy, 8599, Duration::from_millis(600))
            .await
            .is_none(),
        "once"
    );
    clear(&map, vec![timmy]);
    // Ramstein's Knockout.
    let ramstein = pulled(&map, 10439, me).await;
    // Seen first, as Timmy is, or his cast goes to no one.
    c.sees(ramstein).await;
    let knockout = map
        .query(move |m| event_with(m, 10439, 12000))
        .await
        .unwrap();
    map.query(move |m| ai::add_threat(m, ramstein, me, 1000.0))
        .await;
    c.clear();
    fire_in_fight(&map, ramstein, me, knockout).await;
    assert_eq!(
        cast_any_of(&mut c, ramstein, 17307, Duration::from_secs(3)).await,
        Some(me),
        "Knockout"
    );
    assert!(
        until(&map, 3000, move |m| threat(m, ramstein, me) < 1000.0).await,
        "his victim's threat taken"
    );
    clear(&map, vec![ramstein]);
    // Dathrohan, and Balnazzar.
    let dathrohan = pulled(&map, 10812, me).await;
    map.query(move |m| hurt_to(m, dathrohan, 39)).await;
    assert!(
        until(&map, 3000, move |m| m.creature(dathrohan).map(|c| c.entry)
            == Some(10813))
        .await,
        "Balnazzar"
    );
    map.query(move |m| ai::enter_evade(m, dathrohan)).await;
    assert!(
        until(&map, 6000, move |m| m.creature(dathrohan).map(|c| c.entry)
            == Some(10812))
        .await,
        "Dathrohan again, out of the fight"
    );
    let before = map
        .query(|m| living(m, 10390).len() + living(m, 10391).len())
        .await
        .unwrap();
    map.query(move |m| {
        ai::attacked(m, dathrohan, me);
        kill(m, me, dathrohan);
    })
    .await;
    assert!(
        until(&map, 3000, move |m| living(m, 10390).len()
            + living(m, 10391).len()
            == before + 32)
        .await,
        "32 skeletons"
    );
    // Willey's riflemen.
    let willey = pulled(&map, 10997, me).await;
    let call = map
        .query(move |m| {
            m.realm.data.scripts.ai_events[&10997]
                .iter()
                .find(|e| e.params[0] == 5000 && e.params[1] == 5000)
                .map(|e| e.id)
                .unwrap()
        })
        .await
        .unwrap();
    let riflemen = map.query(|m| living(m, 11054).len()).await.unwrap();
    fire_in_fight(&map, willey, me, call).await;
    assert!(
        until(&map, 2000, move |m| living(m, 11054).len() == riflemen + 3).await,
        "three riflemen"
    );
}

/// The Wailing Caverns' disciple: he greets (698) and offers nothing while
/// a Fanglord lives; with all four dead (field four special) he greets
/// with 699 and offers to begin, and begun he takes the party's side and
/// walks his path of 32 points, holding at the seventh, fifteenth and
/// thirtieth; at the thirtieth his ritual calls the moccasins (12
/// seconds), seven ectoplasms (52) and Mutanus last (102).
#[tokio::test]
#[ignore = "extended"]
async fn the_disciple_begins_once_the_fanglords_are_dead() {
    use super::tests_p4_scripts_dungeons::{inside, set};
    const DISCIPLE: u32 = 3678;
    let Some(realm) = realm() else {
        return;
    };
    let route = realm
        .data
        .scripts
        .routes
        .iter()
        .find(|((e, _), _)| *e == DISCIPLE)
        .map(|(_, p)| p.clone())
        .expect("his route");
    assert_eq!(route.len(), 32);
    let holds: Vec<u32> = route
        .iter()
        .filter(|p| p.wait_ms > 60_000)
        .map(|p| p.point)
        .collect();
    assert_eq!(holds, [7, 15, 30]);
    let ritual = route.iter().find(|p| p.point == 30).unwrap().script_id;
    let (mut c, _, map) = inside(&realm, "Disciple", 43, DISCIPLE).await;
    let d = super::tests_p4_scripts_dungeons::alive(&map, DISCIPLE).await[0];
    c.sees(d).await;
    c.clear();
    hello(&c, d).await;
    let before = c
        .wait_for("his menu", |m| match m {
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g)
                if g.guid == d =>
            {
                Some((g.title_text_id, g.gossips.len()))
            }
            _ => None,
        })
        .await;
    assert_eq!(before, (698, 0), "nothing while a Fanglord lives");
    map.query(|m| set(m, 4, 4)).await;
    c.clear();
    hello(&c, d).await;
    let begin = option(&mut c, d, "Let the event begin!").await;
    choose(&c, d, begin).await;
    assert!(
        until(&map, 2000, move |m| faction(m, d) == 250
            && m.creature(d).map(|c| c.movement_type)
                == Some(crate::game::map::creature::MOVEMENT_WAYPOINT))
        .await,
        "on the party's side, on his way"
    );
    // The ritual.
    let count = |m: &Map, e: u32| living(m, e).len();
    map.query(move |m| {
        let before = [5762, 5763, 3654].map(|e| count(m, e));
        crate::game::map::scripts::start(
            m,
            crate::game::data::scripts::ScriptTable::Movement,
            ritual,
            d,
            d,
        );
        let on = |m: &mut Map, ms: u64| {
            for _ in 0..ms / 500 {
                hurry_runs(m, None, Duration::from_millis(500));
            }
        };
        on(m, 13_000);
        assert!(count(m, 5762) > before[0], "the moccasins");
        assert_eq!(count(m, 5763), before[1]);
        on(m, 40_000);
        assert_eq!(count(m, 5763), before[1] + 7, "seven ectoplasms");
        assert_eq!(count(m, 3654), before[2], "Mutanus last");
        on(m, 50_000);
        assert_eq!(count(m, 3654), before[2] + 1, "Mutanus");
    })
    .await
    .unwrap();
}
