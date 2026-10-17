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

/// `g` pulled by `me`.
async fn pull(map: &MapHandle, g: Guid, me: Guid) {
    map.query(move |m| ai::attacked(m, g, me)).await;
}

/// Its health now `pct` percent, and its timed events looked at.
fn hurt_to(m: &mut Map, g: Guid, pct: u32) {
    set_pct(m, g, pct);
    if let Some(s) = event_ai::scripted_mut(m, g) {
        s.next_tick = None;
    }
    event_ai::tick(m, g);
}

fn living_of(m: &Map, entries: &[u32]) -> Vec<Guid> {
    entries.iter().flat_map(|e| living(m, *e)).collect()
}

/// Each of `g`'s timed casts in turn, as `timed_cast` checks them: (event,
/// spell, first, repeat, at the player or not).
async fn kit(
    c: &mut Client,
    map: &MapHandle,
    g: Guid,
    me: Guid,
    casts: &[(u32, u32, (f32, f32), (f32, f32), bool)],
) {
    // Each first timer as the pull set it, before any goes.
    for (id, _, first, _, _) in casts.iter().copied() {
        let t = timer(map, g, id).await;
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
/// medics, again three minutes on); Doom'rel's voidwalkers at half, once;
/// Plugger Spazzring's demon armor and grumbling at peace; Emperor
/// Dagran Thaurissan's rally and his death, which leaves Moira shaken; and
/// Moira's heal of an injured friend.
async fn blackrock_depths(realm: &Arc<Realm>) {
    let (mut c, me, map) = hero_in(realm, "Depths", 409, ARENA, 120.0, &[]).await;
    // Gerstahn.
    let g = boss(&map, 9018, 2.0).await;
    c.sees(g).await;
    pull(&map, g, me).await;
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
    map.query(move |m| hurt_to(m, g, 50)).await;
    assert!(
        cast_of(&mut c, g, 8269, Duration::from_secs(1))
            .await
            .is_some(),
        "Grizzle's Frenzy below half"
    );
    assert!(
        emoted(&mut c, "Grizzle", Duration::from_millis(600))
            .await
            .is_some()
    );
    assert!(about(timer(&map, g, 902802).await, 15.0), "every fifteen");
    clear(&map, vec![g]);
    // Angerforge.
    let g = boss(&map, 9033, 2.0).await;
    c.sees(g).await;
    pull(&map, g, me).await;
    map.query(move |m| hurt_to(m, g, 29)).await;
    assert!(
        emoted(&mut c, "General Angerforge", Duration::from_secs(1))
            .await
            .is_some(),
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
    assert!(
        about(timer(&map, g, 903302).await, 180.0),
        "again three minutes on"
    );
    let mut gone = map.query(|m| living_of(m, &[8901, 8894])).await.unwrap();
    gone.push(g);
    clear(&map, gone);
    // Doom'rel.
    let g = boss(&map, 9039, 2.0).await;
    c.sees(g).await;
    pull(&map, g, me).await;
    c.clear();
    map.query(move |m| hurt_to(m, g, 50)).await;
    assert!(
        cast_of(&mut c, g, 15092, Duration::from_secs(1))
            .await
            .is_some(),
        "his voidwalkers at half"
    );
    assert!(
        map.query(move |m| spent(m, g, 903905)).await.unwrap(),
        "once"
    );
    let mut gone = map
        .query(move |m| {
            m.creatures
                .iter()
                .filter(|(_, c)| c.scripted.as_deref().and_then(|s| s.summoner) == Some(g))
                .map(|(_, c)| c.guid)
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    gone.push(g);
    clear(&map, gone);
    // Plugger, at peace.
    let g = boss(&map, 9499, 2.0).await;
    c.sees(g).await;
    assert!(about(timer(&map, g, 949905).await, 1.0));
    assert!(about(timer(&map, g, 949906).await, 10.0));
    fire(&map, g, 949905).await;
    assert!(
        until(&map, 1000, move |m| has(m, g, 13787)).await,
        "his demon armor"
    );
    c.clear();
    fire(&map, g, 949906).await;
    assert!(
        heard(&mut c, g, Duration::from_secs(1)).await.is_some(),
        "his grumbling"
    );
    assert!(between(timer(&map, g, 949906).await, 30.0, 35.0));
    clear(&map, vec![g]);
    // The Emperor and Moira.
    let emperor = boss(&map, 9019, 2.0).await;
    let moira = boss(&map, 8929, 6.0).await;
    let friend = boss(&map, 9033, 60.0).await;
    c.sees(emperor).await;
    pull(&map, emperor, me).await;
    assert!(
        heard(&mut c, emperor, Duration::from_secs(1))
            .await
            .is_some(),
        "his yell"
    );
    assert!(
        until(&map, 2000, move |m| fighting(m, friend)).await,
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
        cast_any_of(&mut c, moira, 15586, Duration::from_secs(1)).await,
        Some(friend),
        "Moira's heal on the injured"
    );
    map.query(move |m| crate::game::map::spells::interrupt(m, moira))
        .await;
    // His death: Moira gives up the fight.
    c.clear();
    map.query(move |m| kill(m, me, emperor)).await;
    assert!(
        emoted(&mut c, "Princess Moira Bronzebeard", Duration::from_secs(1))
            .await
            .is_some(),
        "Moira, shaken"
    );
    let after = map
        .query(move |m| (faction(m, moira), m.creature(moira).unwrap().evading))
        .await
        .unwrap();
    assert_eq!(after, (35, true), "Moira, friendly, goes home");
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
    pull(&map, g, me).await;
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
        cast_any_of(&mut c, g, 16075, Duration::from_secs(2))
            .await
            .is_some(),
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
        cast_any_of(&mut c, g, 16075, Duration::from_secs(2))
            .await
            .is_some(),
        "the second"
    );
    let none = map.query(arms).await.unwrap();
    assert_eq!(none, (0, 0, 2), "barehanded");
    assert!(
        until(&map, 1000, move |m| has(m, g, 16076)).await,
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
        until(&map, 1000, |m| living(m, 9216).len() == 1
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
    assert!(
        emoted(&mut c, "Halycon", Duration::from_secs(1))
            .await
            .is_some(),
        "her howl"
    );
    let gizrul = map.query(|m| first(m, 10268)).await.flatten();
    let gizrul = gizrul.expect("Gizrul comes");
    assert!(
        until(&map, 2000, move |m| m.creature(gizrul).is_some_and(|c| [
            me, other
        ]
        .iter()
        .all(|p| c.threat.iter().any(|(t, _)| t == p))))
        .await,
        "for the whole party"
    );
    clear(&map, vec![gizrul]);
    // The Beast.
    let g = boss(&map, 10430, 20.0).await;
    c.sees(g).await;
    assert!(
        until(&map, 1000, move |m| has(m, g, 15506)).await,
        "its fiery aura from its spawn"
    );
    c.clear();
    pull(&map, g, me).await;
    assert_eq!(
        cast_any_of(&mut c, g, 16636, Duration::from_secs(2)).await,
        Some(me),
        "at its victim, at once"
    );
    assert!(
        map.query(move |m| spent(m, g, 1043005)).await.unwrap(),
        "once"
    );
    assert!(between(timer(&map, g, 1043006).await, 15.0, 20.0));
    map.query(move |m| {
        crate::game::map::auras::remove_spell(m, g, 15506, None);
        ai::enter_evade(m, g);
    })
    .await;
    assert!(
        until(&map, 15_000, move |m| at_peace(m, g) && has(m, g, 15506)).await,
        "the aura again, home"
    );
    clear(&map, vec![g]);
    // A Blackhand Veteran.
    let g = boss(&map, 9819, 15.0).await;
    c.sees(g).await;
    c.clear();
    pull(&map, g, me).await;
    assert_eq!(
        cast_any_of(&mut c, g, 15749, Duration::from_secs(2)).await,
        Some(me),
        "a shield charge at its victim"
    );
    assert!(map.query(move |m| spent(m, g, 981901)).await.unwrap());
    clear(&map, vec![g]);
}

/// The dungeon bosses, on one load of the real world.
#[tokio::test]
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
}
