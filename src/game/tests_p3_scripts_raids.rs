//! Thistle Tea's raid and world bosses as it scripts them, on the real
//! world: Onyxia in a copy of her lair. Her flight's arrivals and sends are
//! driven as her movement and her own script events tell them, and the
//! waits between skipped.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use crate::game::Realm;
use crate::game::map::combat::dist;
use crate::game::map::{Map, ai, event_ai, scripts};

const ONYXIA: u32 = 10184;
const WHELP: u32 = 11262;
const HOVER: u32 = 17131;
const BELLOWING_ROAR: u32 = 18431;
const DEEP_BREATH_3: u32 = 18351;

/// Her flight's points 2, 3 and 4 (of eight, round her chamber).
const POINT_2: Vector3d = Vector3d {
    x: -63.5156,
    y: -240.096,
    z: -65.477,
};
const POINT_3: Vector3d = Vector3d {
    x: -65.8444,
    y: -213.809,
    z: -65.2985,
};
const POINT_4: Vector3d = Vector3d {
    x: -58.2509,
    y: -189.02,
    z: -65.79,
};

/// Where a creature is headed: the end of its walk, if it walks.
fn heading(m: &Map, g: Guid) -> Option<Vector3d> {
    match &m.creature(g)?.motion {
        crate::game::map::creature::Motion::Moving(w) => w.points.last().copied(),
        _ => None,
    }
}

fn flies(m: &Map, g: Guid) -> bool {
    event_ai::scripted(m, g).is_some_and(|s| s.fly)
}

fn runs_of(m: &Map, g: Guid) -> usize {
    m.scripts
        .runs
        .iter()
        .filter(|r| r.source == g && r.next != usize::MAX)
        .count()
}

/// Onyxia hung at point 3: in her phase for it, still, flying.
fn hang_at_3(m: &mut Map, g: Guid) {
    scripts::stop_for(m, g);
    scripts::place(m, g, POINT_3, 0.0);
    if let Some(s) = event_ai::scripted_mut(m, g) {
        s.fly = true;
        s.no_combat_movement = true;
        s.no_melee = true;
    }
    event_ai::set_phase(m, g, 13);
}

/// Onyxia: her pull wakes her with her words and draws the whole lair into
/// the fight; at a point of her flight she hangs until she sends herself
/// on; from point 3 she flies on to a neighbour (2 or 4) or breathes across
/// the chamber; a send left over from another point does nothing; landed,
/// she roars over whatever she is casting, and again later; an evade puts
/// her on the ground in her first phase, her whelps gone. Her whelps draw
/// the lair into the fight too.
async fn onyxia(realm: &Arc<Realm>) {
    let at = Vector3d {
        x: -14.0,
        y: -217.0,
        z: -86.7,
    };
    let (mut c, me, map) = hero_in(realm, "Onyxian", 249, at, 0.0, &[]).await;
    let far = Vector3d {
        x: -90.0,
        y: -100.0,
        z: -86.7,
    };
    let (_d, other) = joins(realm, "Distant", &map, far).await;
    let ony = map
        .query(|m| first(m, ONYXIA))
        .await
        .flatten()
        .expect("Onyxia");
    c.sees(ony).await;
    map.query(move |m| {
        tough(m, ony);
        ai::attacked(m, ony, me);
    })
    .await;
    assert!(
        heard(&mut c, ony, Duration::from_secs(2)).await.is_some(),
        "she wakes with her words"
    );
    assert!(
        until(&map, 2000, move |m| threat_listed(m, ony, other)).await,
        "the player far off in her lair drawn in"
    );
    assert_eq!(map.query(move |m| event_ai::phase(m, ony)).await, Some(0));
    // At a point she hangs until she sends herself on.
    map.query(move |m| {
        hang_at_3(m, ony);
        event_ai::set_phase(m, ony, 2);
        event_ai::movement_inform(m, ony, event_ai::POINT_MOTION, 3);
    })
    .await;
    let (phase, waiting, going) = map
        .query(move |m| (event_ai::phase(m, ony), runs_of(m, ony), heading(m, ony)))
        .await
        .unwrap();
    assert_eq!((phase, waiting, going), (13, 1, None), "hanging at point 3");
    sleep_ms(500).await;
    assert_eq!(
        map.query(move |m| heading(m, ony)).await,
        Some(None),
        "still there"
    );
    hurry(&map, ony, 25.1).await;
    assert!(
        until(&map, 1000, move |m| event_ai::phase(m, ony) == 2).await,
        "she sent herself on"
    );
    // From point 3: on to 2 or 4, or a deep breath across.
    let (mut to_2, mut to_4, mut breath) = (0, 0, 0);
    for _ in 0..40 {
        map.query(move |m| hang_at_3(m, ony)).await;
        c.clear();
        map.query(move |m| event_ai::script_event(m, ony, 1, 3, Some(ony)))
            .await;
        let going = map.query(move |m| heading(m, ony)).await.flatten();
        let phase = map.query(move |m| event_ai::phase(m, ony)).await.unwrap();
        assert_eq!(phase, 2, "flying on");
        match going {
            Some(p) if dist(p, POINT_2) < 1.0 => to_2 += 1,
            Some(p) if dist(p, POINT_4) < 1.0 => to_4 += 1,
            Some(p) => panic!("headed for {p:?}"),
            None => {
                assert!(
                    cast_start_of(&mut c, ony, DEEP_BREATH_3, Duration::from_secs(1))
                        .await
                        .is_some(),
                    "a deep breath"
                );
                breath += 1;
            }
        }
        if to_2 > 0 && to_4 > 0 && breath > 0 {
            break;
        }
    }
    assert!(
        to_2 > 0 && to_4 > 0 && breath > 0,
        "each way: {to_2}, {to_4}, {breath}"
    );
    // A send from point 5, at point 3, does nothing.
    map.query(move |m| {
        hang_at_3(m, ony);
        event_ai::script_event(m, ony, 1, 5, Some(ony));
    })
    .await;
    assert_eq!(
        map.query(move |m| (event_ai::phase(m, ony), heading(m, ony)))
            .await,
        Some((13, None)),
        "a send of another point's does nothing"
    );
    // Landed: Bellowing Roar over what she casts, and again later.
    let casting = map
        .query(move |m| {
            scripts::stop_for(m, ony);
            event_ai::set_phase(m, ony, 5);
            let now = m.now;
            if let Some(cr) = m.caster_mut(ony) {
                cr.casting = Some(crate::game::map::spells::Casting {
                    spell: 18435,
                    targets: Default::default(),
                    ends: now + Duration::from_secs(5),
                    item: None,
                    pushbacks: 0,
                    channel_until: None,
                    duration: 5000,
                });
            }
            event_ai::script_event(m, ony, 3, 0, Some(ony));
            runs_of(m, ony)
        })
        .await
        .unwrap();
    assert!(
        cast_start_of(&mut c, ony, BELLOWING_ROAR, Duration::from_secs(1))
            .await
            .is_some(),
        "she roars over her cast"
    );
    assert_eq!(
        map.query(move |m| m.creature(ony).and_then(|c| c
            .caster
            .casting
            .as_ref()
            .map(|x| x.spell)))
            .await
            .flatten(),
        Some(BELLOWING_ROAR),
        "the breath cut off for it"
    );
    assert_eq!(casting, 1, "and roars again later");
    c.clear();
    map.query(move |m| crate::game::map::spells::interrupt(m, ony))
        .await;
    hurry(&map, ony, 30.1).await;
    assert!(
        cast_start_of(&mut c, ony, BELLOWING_ROAR, Duration::from_secs(1))
            .await
            .is_some(),
        "again"
    );
    // A brood out, in the air: an evade puts her down and sends them off.
    let whelps = map
        .query(move |m| {
            hang_at_3(m, ony);
            crate::game::map::spells::cast_triggered(m, ony, HOVER, Some(ony));
            event_ai::script_event(m, ony, 2, 1, Some(ony));
            m.scripts.runs.len()
        })
        .await
        .unwrap();
    assert!(whelps > 0);
    hurry(&map, ony, 8.0).await;
    assert!(
        until(&map, 2000, |m| living(m, WHELP).len() >= 8).await,
        "her brood"
    );
    map.query(move |m| ai::enter_evade(m, ony)).await;
    let after = map
        .query(move |m| {
            (
                event_ai::phase(m, ony),
                flies(m, ony),
                has(m, ony, HOVER),
                living(m, WHELP).len(),
                runs_of(m, ony),
            )
        })
        .await
        .unwrap();
    assert_eq!(after, (0, false, false, 0, 0), "down, her whelps gone");
    assert!(until(&map, 10_000, move |m| at_peace(m, ony)).await);
    // A whelp's pull draws the lair in.
    let whelp = map
        .query(move |m| {
            let g = summon(m, WHELP, off(at, 5.0, 0.0), 0.0);
            ai::attacked(m, g, me);
            g
        })
        .await
        .unwrap();
    assert!(
        until(&map, 2000, move |m| threat_listed(m, whelp, other)).await,
        "a whelp draws in the player far off"
    );
    clear(&map, vec![whelp]);
}

fn threat_listed(m: &Map, g: Guid, who: Guid) -> bool {
    m.creature(g)
        .is_some_and(|c| c.threat.iter().any(|(t, _)| *t == who))
}

const YSONDRE: u32 = 14887;
const LETHON: u32 = 14888;
const EMERISS: u32 = 14889;
const TAERAR: u32 = 14890;
const DREAM_FOG: u32 = 15224;
const DEMENTED_DRUID: u32 = 15260;
const SPIRIT_SHADE: u32 = 15261;
const SHADE_OF_TAERAR: u32 = 15302;
const NOT_SELECTABLE: u32 = 0x0200_0000;

/// An arena with nothing about: Ragnaros's lair, in a copy of Molten Core.
const ARENA: Vector3d = Vector3d {
    x: 840.0,
    y: -812.0,
    z: -229.6,
};

/// A boss of `entry` at the arena, standing its ground, and lasting.
async fn boss(map: &crate::game::map::MapHandle, entry: u32, dx: f32) -> Guid {
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

/// Its health now `pct` percent, and its timed events looked at.
fn hurt_to(m: &mut Map, g: Guid, pct: u32) {
    set_pct(m, g, pct);
    if let Some(s) = event_ai::scripted_mut(m, g) {
        s.next_tick = None;
    }
    event_ai::tick(m, g);
}

/// The Emerald Dream's dragons: each marks the players around it on the
/// pull, and an evade ends what it has under way. Ysondre's druids, three
/// for every four players fighting her (three at the least); Lethon's
/// Draw Spirit raises a shade where it strikes a player; Taerar banishes
/// himself until his three shades fall, and a banishment's timeout ends
/// only its own; a Dream Fog drifts to another player its owner fights.
async fn dream_dragons(realm: &Arc<Realm>) {
    let (mut c, me, map) = hero_in(realm, "Dreamer", 409, ARENA, 120.0, &[]).await;
    let (_d, other) = joins(realm, "Sleeper", &map, off(ARENA, 0.0, 6.0)).await;
    for entry in [YSONDRE, LETHON, EMERISS, TAERAR] {
        let d = boss(&map, entry, 10.0).await;
        c.sees(d).await;
        map.query(move |m| ai::attacked(m, d, me)).await;
        assert!(
            until(&map, 1500, move |m| has(m, d, 25041)).await,
            "{entry}: Mark of Nature at the pull"
        );
        // Its first mark on those about ten seconds on (the first dragon's
        // waited for).
        if entry == YSONDRE {
            assert!(
                until(&map, 11_000, move |m| has(m, me, 25042)
                    && has(m, other, 25042))
                .await,
                "{entry}: the players about marked"
            );
        }
        if entry == TAERAR {
            map.query(move |m| hurt_to(m, d, 74)).await;
            assert!(
                until(&map, 1000, move |m| runs_of(m, d) > 0).await,
                "his banishment's timeout under way"
            );
        }
        map.query(move |m| ai::enter_evade(m, d)).await;
        assert_eq!(
            map.query(move |m| (runs_of(m, d), event_ai::phase(m, d)))
                .await,
            Some((0, 0)),
            "{entry}: its evade ends what it had under way"
        );
        clear(&map, vec![d]);
        sleep_ms(100).await;
        map.query(move |m| {
            for p in [me, other] {
                crate::game::map::auras::remove_spell(m, p, 25042, None);
            }
        })
        .await;
    }
    // Ysondre's druids: three for two players; three for every four of six.
    let ysondre = boss(&map, YSONDRE, 10.0).await;
    map.query(move |m| {
        ai::attacked(m, ysondre, me);
        ai::add_threat(m, ysondre, other, 1.0);
        hurt_to(m, ysondre, 74);
    })
    .await;
    let druids = |map: &crate::game::map::MapHandle| {
        let map = map.clone();
        async move { map.query(|m| living(m, DEMENTED_DRUID)).await.unwrap() }
    };
    assert!(
        until(&map, 1500, |m| living(m, DEMENTED_DRUID).len() == 3).await,
        "three druids for two: {:?}",
        druids(&map).await.len()
    );
    let mut more = Vec::new();
    for i in 0..4 {
        let name = ["Dozea", "Dozeb", "Dozec", "Dozed"][i];
        more.push(joins(realm, name, &map, off(ARENA, 0.0, -6.0)).await);
    }
    let extra: Vec<Guid> = more.iter().map(|(_, g)| *g).collect();
    let hunting = map
        .query(move |m| {
            let targets: Vec<Guid> = living(m, DEMENTED_DRUID)
                .into_iter()
                .map(|g| victim(m, g))
                .collect();
            for g in living(m, DEMENTED_DRUID) {
                scripts::despawn(m, g);
            }
            for p in &extra {
                ai::add_threat(m, ysondre, *p, 1.0);
            }
            hurt_to(m, ysondre, 49);
            targets
        })
        .await
        .unwrap();
    assert!(
        hunting.iter().all(|t| *t == me || *t == other),
        "each hunting a player: {hunting:?}"
    );
    assert!(
        until(&map, 1500, |m| living(m, DEMENTED_DRUID).len() == 4).await,
        "four for six: {}",
        druids(&map).await.len()
    );
    // Her Dream Fog: a retarget puts another of her players on its list.
    let fog = map
        .query(move |m| {
            for g in living(m, DEMENTED_DRUID) {
                scripts::despawn(m, g);
            }
            let at = m.position_of(ysondre).unwrap();
            let fog = scripts::summon(m, ysondre, DREAM_FOG, off(at, 3.0, 3.0), 0.0, None).unwrap();
            ai::attacked(m, fog, me);
            fog
        })
        .await
        .unwrap();
    fire(&map, fog, 1522402).await;
    assert!(
        map.query(move |m| m
            .creature(fog)
            .unwrap()
            .threat
            .iter()
            .any(|(t, _)| *t != me))
            .await
            .unwrap(),
        "drifts to another she fights"
    );
    drop(more);
    clear(&map, vec![ysondre, fog]);
    // Lethon's Draw Spirit raises a shade where it strikes.
    let lethon = boss(&map, LETHON, 10.0).await;
    map.query(move |m| {
        ai::attacked(m, lethon, me);
        hurt_to(m, lethon, 74);
    })
    .await;
    assert!(
        until(&map, 2000, move |m| {
            let at = m.position_of(me).unwrap();
            living(m, SPIRIT_SHADE)
                .iter()
                .any(|s| m.position_of(*s).is_some_and(|p| dist(p, at) < 2.0))
        })
        .await,
        "a shade where the player stood"
    );
    let shades = map.query(|m| living(m, SPIRIT_SHADE)).await.unwrap();
    let mut gone = shades;
    gone.push(lethon);
    clear(&map, gone);
    // Taerar's banishments.
    let taerar = boss(&map, TAERAR, 10.0).await;
    map.query(move |m| {
        ai::attacked(m, taerar, me);
        hurt_to(m, taerar, 74);
    })
    .await;
    assert!(
        until(&map, 2000, move |m| living(m, SHADE_OF_TAERAR).len() == 3).await,
        "three shades"
    );
    let state = move |m: &mut Map| {
        (
            event_ai::phase(m, taerar),
            unit_flags(m, taerar) & NOT_SELECTABLE != 0,
            has(m, taerar, 24883),
        )
    };
    assert_eq!(map.query(state).await, Some((1, true, true)), "banished");
    let shades = map.query(|m| living(m, SHADE_OF_TAERAR)).await.unwrap();
    for (i, s) in shades.iter().copied().enumerate() {
        map.query(move |m| kill(m, me, s)).await;
        sleep_ms(100).await;
        let now = map.query(state).await.unwrap();
        if i < 2 {
            assert_eq!(now, (i as u32 + 2, true, true), "shade {i} down");
        } else {
            assert_eq!(now, (0, false, false), "the last down: he wakes");
        }
    }
    // His second banishment: the first's timeout leaves it; its own ends it.
    map.query(move |m| hurt_to(m, taerar, 49)).await;
    assert!(until(&map, 2000, move |m| event_ai::phase(m, taerar) == 4).await);
    map.query(move |m| event_ai::script_event(m, taerar, 1, 1, Some(taerar)))
        .await;
    assert_eq!(
        map.query(state).await,
        Some((4, true, true)),
        "another banishment's timeout"
    );
    hurry(&map, taerar, 120.5).await;
    assert_eq!(
        map.query(state).await,
        Some((0, false, false)),
        "its own timeout wakes him"
    );
    let shades = map.query(|m| living(m, SHADE_OF_TAERAR)).await.unwrap();
    let mut gone = shades;
    gone.push(taerar);
    clear(&map, gone);
}

/// Emberstrife: Cleave and Flame Breath at his victim on their timers; below
/// sixty percent a Frenzy and its emote, again every two minutes; below
/// eleven his line, once.
async fn emberstrife(realm: &Arc<Realm>) {
    let (mut c, me, map) = hero_in(realm, "Ember", 409, ARENA, 120.0, &[]).await;
    let e = boss(&map, 10321, 2.0).await;
    c.sees(e).await;
    map.query(move |m| ai::attacked(m, e, me)).await;
    timed_cast(
        &mut c,
        &map,
        e,
        1032101,
        19983,
        (6.0, 8.0),
        (6.0, 8.0),
        Some(me),
    )
    .await;
    timed_cast(
        &mut c,
        &map,
        e,
        1032102,
        9573,
        (8.0, 12.0),
        (8.0, 12.0),
        Some(me),
    )
    .await;
    c.clear();
    map.query(move |m| hurt_to(m, e, 59)).await;
    assert!(
        cast_of(&mut c, e, 8269, Duration::from_secs(1))
            .await
            .is_some(),
        "Frenzy below sixty"
    );
    assert!(
        emoted(&mut c, "Emberstrife", Duration::from_millis(800))
            .await
            .is_some(),
        "his frenzy's emote"
    );
    assert!(
        about(timer(&map, e, 1032103).await, 120.5),
        "again in two minutes"
    );
    c.clear();
    map.query(move |m| hurt_to(m, e, 10)).await;
    assert!(
        emoted(&mut c, "Emberstrife", Duration::from_secs(1))
            .await
            .is_some(),
        "weakened, below eleven"
    );
    assert!(
        map.query(move |m| spent(m, e, 1032104)).await.unwrap(),
        "once"
    );
    clear(&map, vec![e]);
}

/// The Twilight Corrupter: his shout at the pull; Soul Corruption on the
/// room and Creature of Nightmare at a random player on their timers; each
/// player he kills feeds him a soul (his emote at them, Swell of Souls), a
/// creature none.
async fn twilight_corrupter(realm: &Arc<Realm>) {
    let (mut c, me, map) = hero_in(realm, "Corrupt", 409, ARENA, 120.0, &[]).await;
    let (_d, victim_p) = joins(realm, "Fodder", &map, off(ARENA, 3.0, 3.0)).await;
    let t = boss(&map, 15625, 6.0).await;
    c.sees(t).await;
    map.query(move |m| ai::attacked(m, t, me)).await;
    assert!(
        heard(&mut c, t, Duration::from_secs(1)).await.is_some(),
        "his shout"
    );
    timed_cast(
        &mut c,
        &map,
        t,
        1562502,
        25805,
        (6.0, 18.0),
        (20.0, 30.0),
        None,
    )
    .await;
    timed_cast(
        &mut c,
        &map,
        t,
        1562503,
        25806,
        (10.0, 20.0),
        (35.0, 40.0),
        None,
    )
    .await;
    // A creature killed: nothing.
    let foe = map.query(|m| dummy(m, off(ARENA, 6.0, 4.0))).await.unwrap();
    c.clear();
    map.query(move |m| crate::game::map::combat::deal_damage(m, t, foe, 50_000_000))
        .await;
    assert!(
        cast_of(&mut c, t, 21307, Duration::from_millis(600))
            .await
            .is_none(),
        "no soul from a creature"
    );
    // A player killed: his emote at them, and a soul.
    map.query(move |m| {
        let p = m.players.get_mut(&victim_p).unwrap();
        p.misc.god = false;
        ai::add_threat(m, t, victim_p, 1.0);
        crate::game::map::combat::deal_damage(m, t, victim_p, 50_000_000);
    })
    .await;
    assert!(
        cast_of(&mut c, t, 21307, Duration::from_secs(1))
            .await
            .is_some(),
        "Swell of Souls"
    );
    assert_eq!(
        emoted(&mut c, "Twilight Corrupter", Duration::from_millis(800))
            .await
            .map(|(_, at)| at),
        Some(victim_p),
        "his emote at them"
    );
    clear(&map, vec![t, foe]);
}

/// The raid bosses, on one load of the real world.
#[tokio::test]
async fn raid_bosses_fight_as_thistle_tea_scripts_them() {
    let Some(realm) = realm() else {
        return;
    };
    if wanted("onyxia") {
        onyxia(&realm).await;
    }
    if wanted("dragons") {
        dream_dragons(&realm).await;
    }
    if wanted("emberstrife") {
        emberstrife(&realm).await;
    }
    if wanted("corrupter") {
        twilight_corrupter(&realm).await;
    }
}
