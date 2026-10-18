//! The dungeon bosses Thistle Tea scripts (Blackrock Depths, Blackrock
//! Spire and Stratholme), on the real world: each pulled in an arena with
//! nothing about, its timed casts made due and seen as a player's client
//! sees them, its health events set off by its health.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use crate::game::Realm;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::{Map, MapHandle, ai, event_ai};

/// How long to wait for what a step brings: what the map does at once,
/// and its client hears, comes within an update or two on a quiet
/// machine and in seconds on a busy one. The waits end as soon as it
/// comes.
const SOON: Duration = crate::game::harness::WAIT;
const SOON_MS: u64 = SOON.as_millis() as u64;

/// Ragnaros's lair, in a copy of Molten Core: nothing about.
const ARENA: Vector3d = Vector3d {
    x: 840.0,
    y: -812.0,
    z: -229.6,
};

/// A boss of `entry` `dx` yards east of the arena's middle, facing it,
/// standing its ground, and lasting.
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

/// [`boss`], and its timers `ids` as its first look at its events rolls
/// them, read with it: the map's clock runs on between queries, seconds
/// on a busy machine.
async fn boss_timed(
    map: &MapHandle,
    entry: u32,
    dx: f32,
    ids: &'static [u32],
) -> (Guid, Vec<Option<Duration>>) {
    map.query(move |m| {
        let g = summon(m, entry, off(ARENA, dx, 0.0), std::f32::consts::PI);
        tough(m, g);
        if let Some(s) = event_ai::scripted_mut(m, g) {
            s.no_combat_movement = true;
        }
        // Its first look at its events, which the next update would take,
        // rolls them.
        event_ai::tick(m, g);
        (g, ids.iter().map(|id| left(m, g, *id)).collect())
    })
    .await
    .unwrap()
}

/// `g` pulled by `me`.
async fn pull(map: &MapHandle, g: Guid, me: Guid) {
    map.query(move |m| ai::attacked(m, g, me)).await;
}

/// Its health now `pct` percent, and its timed events looked at.
fn hurt_to(m: &mut Map, g: Guid, pct: u32) {
    set_pct(m, g, pct);
    // Out of its last cast's global cooldown, so what the event casts goes.
    if let Some(cr) = m.caster_mut(g) {
        cr.gcd_until = None;
    }
    if let Some(s) = event_ai::scripted_mut(m, g) {
        s.next_tick = None;
    }
    event_ai::tick(m, g);
}

fn living_of(m: &Map, entries: &[u32]) -> Vec<Guid> {
    entries.iter().flat_map(|e| living(m, *e)).collect()
}

/// A timed cast to check: its event, its spell, its first and repeat
/// times (from, to, in seconds), and whether it goes at the player.
type Cast = (u32, u32, (f32, f32), (f32, f32), bool);

/// `g` pulled by `me`, and each of its timed casts in turn, as
/// `timed_cast` checks them: (event, spell, first, repeat, at the player
/// or not).
async fn kit(c: &mut Client, map: &MapHandle, g: Guid, me: Guid, casts: &[Cast]) {
    // Each first timer as the pull set it, read with the pull: the map's
    // clock runs on between queries, seconds on a busy machine.
    let ids: Vec<u32> = casts.iter().map(|c| c.0).collect();
    let firsts = map
        .query(move |m| {
            ai::attacked(m, g, me);
            ids.iter().map(|id| left(m, g, *id)).collect::<Vec<_>>()
        })
        .await
        .unwrap();
    for ((id, _, first, _, _), t) in casts.iter().copied().zip(firsts) {
        assert!(
            between(t, first.0, first.1),
            "{id}: first in {t:?}, not {first:?}"
        );
    }
    for (id, spell, _, repeat, at_me) in casts.iter().copied() {
        timed_cast(
            c,
            map,
            g,
            id,
            spell,
            (0.0, 600.0),
            repeat,
            at_me.then_some(me),
        )
        .await;
    }
}

/// Blackrock Depths' bosses: High Interrogator Gerstahn's four spells;
/// Grizzle's Frenzy below half, every fifteen seconds; General
/// Angerforge's alarm below thirty percent (eight reservists and two
/// medics, again three minutes on); Plugger Spazzring's demon armor and
/// grumbling at peace; Emperor Dagran Thaurissan's rally and his death,
/// which leaves Moira shaken; and Moira's heal of an injured friend.
async fn blackrock_depths(realm: &Arc<Realm>) {
    let (mut c, me, map) = hero_in(realm, "Depths", 409, ARENA, 120.0, &[]).await;
    // Gerstahn.
    let g = boss(&map, 9018, 2.0).await;
    c.sees(g).await;
    kit(
        &mut c,
        &map,
        g,
        me,
        &[
            (901801, 14032, (4.0, 4.0), (7.0, 7.0), true),
            (901802, 14033, (14.0, 14.0), (10.0, 10.0), true),
            (901803, 13704, (32.0, 32.0), (30.0, 30.0), false),
            (901804, 12040, (8.0, 8.0), (25.0, 25.0), false),
        ],
    )
    .await;
    clear(&map, vec![g]);
    // Grizzle.
    let g = boss(&map, 9028, 2.0).await;
    c.sees(g).await;
    pull(&map, g, me).await;
    c.clear();
    let again = map
        .query(move |m| {
            hurt_to(m, g, 50);
            left(m, g, 902802)
        })
        .await
        .flatten();
    assert!(
        cast_of(&mut c, g, 8269, SOON).await.is_some(),
        "Grizzle's Frenzy below half"
    );
    assert!(emoted(&mut c, "Grizzle", SOON).await.is_some());
    assert!(about(again, 15.0), "every fifteen: {again:?}");
    clear(&map, vec![g]);
    // Angerforge.
    let g = boss(&map, 9033, 2.0).await;
    c.sees(g).await;
    pull(&map, g, me).await;
    let again = map
        .query(move |m| {
            hurt_to(m, g, 29);
            left(m, g, 903302)
        })
        .await
        .flatten();
    assert!(
        emoted(&mut c, "General Angerforge", SOON).await.is_some(),
        "his alarm"
    );
    let (reserves, medics) = map
        .query(|m| (living(m, 8901).len(), living(m, 8894).len()))
        .await
        .unwrap();
    assert_eq!(
        (reserves, medics),
        (8, 2),
        "eight reservists and two medics"
    );
    assert!(about(again, 180.0), "again three minutes on: {again:?}");
    let mut gone = map.query(|m| living_of(m, &[8901, 8894])).await.unwrap();
    gone.push(g);
    clear(&map, gone);
    // Plugger, at peace.
    let (g, firsts) = boss_timed(&map, 9499, 2.0, &[949905, 949906]).await;
    c.sees(g).await;
    assert!(about(firsts[0], 1.0), "{firsts:?}");
    assert!(about(firsts[1], 10.0), "{firsts:?}");
    fire(&map, g, 949905).await;
    assert!(
        until(&map, SOON_MS, move |m| has(m, g, 13787)).await,
        "his demon armor"
    );
    c.clear();
    let again = map
        .query(move |m| {
            due(m, g, 949906);
            left(m, g, 949906)
        })
        .await
        .flatten();
    assert!(heard(&mut c, g, SOON).await.is_some(), "his grumbling");
    assert!(between(again, 30.0, 35.0), "{again:?}");
    clear(&map, vec![g]);
    // The Emperor and Moira.
    let emperor = boss(&map, 9019, 2.0).await;
    let moira = boss(&map, 8929, 6.0).await;
    let friend = boss(&map, 9033, 60.0).await;
    c.sees(emperor).await;
    pull(&map, emperor, me).await;
    assert!(heard(&mut c, emperor, SOON).await.is_some(), "his yell");
    assert!(
        until(&map, SOON_MS, move |m| fighting(m, friend)).await,
        "his call brings a friend from sixty yards"
    );
    // Moira mends a friend's wounds.
    map.query(move |m| {
        crate::game::map::scripts::place(m, friend, off(ARENA, 10.0, 4.0), 0.0);
        ai::attacked(m, moira, me);
        set_pct(m, friend, 40);
    })
    .await;
    c.clear();
    fire(&map, moira, 892904).await;
    assert_eq!(
        cast_any_of(&mut c, moira, 15586, SOON).await,
        Some(friend),
        "Moira's heal on the injured"
    );
    map.query(move |m| crate::game::map::spells::interrupt(m, moira))
        .await;
    // His death: Moira gives up the fight.
    c.clear();
    map.query(move |m| kill(m, me, emperor)).await;
    let shaken = emoted(&mut c, "Princess Moira Bronzebeard", SOON).await;
    if shaken.is_none() {
        // What she and the player were, for whoever has to work it out.
        let state = map
            .query(move |m| {
                let at = m.players[&me].position();
                m.creature(moira).map(|c| {
                    (
                        c.alive(),
                        c.evading,
                        faction(m, moira),
                        crate::game::map::combat::dist(c.position, at),
                    )
                })
            })
            .await;
        panic!("Moira, shaken: (alive, evading, faction, yards off) {state:?}");
    }
    // Her evade may be over already: she is out of the fight either way.
    let after = map
        .query(move |m| (faction(m, moira), fighting(m, moira)))
        .await
        .unwrap();
    assert_eq!(after, (35, false), "Moira, friendly, out of the fight");
    clear(&map, vec![emperor, moira, friend]);
}

/// Blackrock Spire's: Omokk's six timers; Voone throws one axe, then the
/// other, and fights barehanded, his axes back at his evade; Wyrmthalak's
/// warlord and berserker at half, once; Halycon's death howls for Gizrul,
/// who comes for the party; The Beast's fiery aura (at its spawn and home
/// again) and its charge at its victim before anyone else; a Blackhand
/// Veteran's shield charge at its victim at once.
async fn blackrock_spire(realm: &Arc<Realm>) {
    let (mut c, me, map) = hero_in(realm, "Spire", 409, ARENA, 120.0, &[]).await;
    let g = boss(&map, 9196, 2.0).await;
    c.sees(g).await;
    kit(
        &mut c,
        &map,
        g,
        me,
        &[
            (919601, 24375, (15.0, 15.0), (14.0, 14.0), false),
            (919602, 18368, (10.0, 10.0), (10.0, 10.0), true),
            (919603, 18106, (14.0, 14.0), (18.0, 18.0), true),
            (919604, 24317, (2.0, 2.0), (25.0, 25.0), true),
            (919605, 20686, (18.0, 18.0), (12.0, 12.0), false),
            (919606, 22356, (24.0, 24.0), (18.0, 18.0), false),
        ],
    )
    .await;
    clear(&map, vec![g]);
    // Voone's axes.
    let g = boss(&map, 9237, 2.0).await;
    c.sees(g).await;
    let arms = move |m: &mut Map| {
        let f = m.unit_fields(g).unwrap();
        (
            f.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY),
            f.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + 1),
            event_ai::phase(m, g),
        )
    };
    let armed = map.query(arms).await.unwrap();
    assert!(armed.0 != 0 && armed.1 != 0, "both axes: {armed:?}");
    pull(&map, g, me).await;
    c.clear();
    fire(&map, g, 923706).await;
    assert!(
        cast_any_of(&mut c, g, 16075, SOON).await.is_some(),
        "the first axe"
    );
    let one = map.query(arms).await.unwrap();
    assert_eq!((one.1, one.2), (0, 1), "one axe left: {one:?}");
    assert!(one.0 != 0);
    map.query(move |m| crate::game::map::spells::interrupt(m, g))
        .await;
    c.clear();
    fire(&map, g, 923707).await;
    assert!(
        cast_any_of(&mut c, g, 16075, SOON).await.is_some(),
        "the second"
    );
    let none = map.query(arms).await.unwrap();
    assert_eq!(none, (0, 0, 2), "barehanded");
    assert!(
        until(&map, SOON_MS, move |m| has(m, g, 16076)).await,
        "his barehanded fighting"
    );
    map.query(move |m| ai::enter_evade(m, g)).await;
    let back = map.query(arms).await.unwrap();
    assert_eq!(
        (back.0, back.1, back.2),
        (armed.0, armed.1, 0),
        "his axes back"
    );
    clear(&map, vec![g]);
    // Wyrmthalak.
    let g = boss(&map, 9568, 2.0).await;
    pull(&map, g, me).await;
    map.query(move |m| hurt_to(m, g, 50)).await;
    assert!(
        until(&map, SOON_MS, |m| living(m, 9216).len() == 1
            && living(m, 9268).len() == 1)
        .await,
        "a warlord and a berserker"
    );
    assert!(map.query(move |m| spent(m, g, 956805)).await.unwrap());
    let mut gone = map.query(|m| living_of(m, &[9216, 9268])).await.unwrap();
    gone.push(g);
    clear(&map, gone);
    // Halycon's howl.
    let (_d, other) = joins(realm, "Party", &map, off(ARENA, -10.0, 10.0)).await;
    let g = boss(&map, 10220, 2.0).await;
    c.sees(g).await;
    pull(&map, g, me).await;
    c.clear();
    map.query(move |m| kill(m, me, g)).await;
    assert!(emoted(&mut c, "Halycon", SOON).await.is_some(), "her howl");
    let gizrul = map.query(|m| first(m, 10268)).await.flatten();
    let gizrul = gizrul.expect("Gizrul comes");
    assert!(
        until(&map, SOON_MS, move |m| m.creature(gizrul).is_some_and(
            |c| [me, other]
                .iter()
                .all(|p| c.threat.iter().any(|(t, _)| t == p))
        ))
        .await,
        "for the whole party"
    );
    clear(&map, vec![gizrul]);
    // The Beast.
    let g = boss(&map, 10430, 20.0).await;
    c.sees(g).await;
    assert!(
        until(&map, SOON_MS, move |m| has(m, g, 15506)).await,
        "its fiery aura from its spawn"
    );
    c.clear();
    // Its next timer as the pull set it, before the map's clock runs on
    // while the charge is waited for.
    let next = map
        .query(move |m| {
            ai::attacked(m, g, me);
            left(m, g, 1043006)
        })
        .await
        .flatten();
    assert_eq!(
        cast_any_of(&mut c, g, 16636, SOON).await,
        Some(me),
        "at its victim, at once"
    );
    assert!(
        map.query(move |m| spent(m, g, 1043005)).await.unwrap(),
        "once"
    );
    assert!(between(next, 15.0, 20.0), "{next:?}");
    map.query(move |m| {
        crate::game::map::auras::remove_spell(m, g, 15506, None);
        ai::enter_evade(m, g);
    })
    .await;
    assert!(
        until(&map, SOON_MS, move |m| at_peace(m, g) && has(m, g, 15506)).await,
        "the aura again, home"
    );
    clear(&map, vec![g]);
    // A Blackhand Veteran.
    let g = boss(&map, 9819, 15.0).await;
    c.sees(g).await;
    c.clear();
    pull(&map, g, me).await;
    assert_eq!(
        cast_any_of(&mut c, g, 15749, SOON).await,
        Some(me),
        "a shield charge at its victim"
    );
    assert!(map.query(move |m| spent(m, g, 981901)).await.unwrap());
    clear(&map, vec![g]);
}

/// Stratholme's: Barthilas's blows restart whether or not they land, and
/// Anastari curses only a victim not cursed.
async fn stratholme(realm: &Arc<Realm>) {
    let (mut c, me, map) = hero_in(realm, "Strath", 409, ARENA, 120.0, &[]).await;
    // Barthilas.
    let g = boss(&map, 10435, 2.0).await;
    c.sees(g).await;
    kit(
        &mut c,
        &map,
        g,
        me,
        &[
            (1043501, 16791, (5.0, 5.0), (4.0, 4.0), false),
            (1043502, 16793, (16.0, 16.0), (15.0, 15.0), true),
            (1043503, 10887, (12.0, 12.0), (15.0, 15.0), true),
            (1043504, 14099, (8.0, 8.0), (20.0, 20.0), true),
        ],
    )
    .await;
    // A blow that can't land (his victim away) waits its time all the same.
    map.query(move |m| put(m, me, off(ARENA, -25.0, 0.0))).await;
    c.clear();
    // Its timer as the blow set it, before the wait for nothing.
    let again = map
        .query(move |m| {
            due(m, g, 1043502);
            left(m, g, 1043502)
        })
        .await
        .flatten();
    assert!(
        cast_of(&mut c, g, 16793, Duration::from_millis(600))
            .await
            .is_none()
    );
    assert!(about(again, 15.0), "restarted all the same: {again:?}");
    map.query(move |m| put(m, me, ARENA)).await;
    clear(&map, vec![g]);
    // Anastari's curse.
    let g = boss(&map, 10436, 2.0).await;
    c.sees(g).await;
    pull(&map, g, me).await;
    c.clear();
    fire(&map, g, 1043602).await;
    assert_eq!(cast_of(&mut c, g, 16867, SOON).await, Some(me), "the curse");
    c.clear();
    fire(&map, g, 1043602).await;
    assert!(
        cast_of(&mut c, g, 16867, Duration::from_millis(600))
            .await
            .is_none(),
        "not on the cursed"
    );
    assert!(
        timer(&map, g, 1043602).await.is_some_and(|t| t.is_zero()),
        "and tried again"
    );
    clear(&map, vec![g]);
}

/// The dungeon bosses, on one load of the real world.
#[tokio::test]
#[ignore = "extended"]
async fn dungeon_bosses_fight_as_thistle_tea_scripts_them() {
    let Some(realm) = realm() else {
        return;
    };
    if wanted("brd") {
        blackrock_depths(&realm).await;
    }
    if wanted("brs") {
        blackrock_spire(&realm).await;
    }
    if wanted("strat") {
        stratholme(&realm).await;
    }
}
