//! Blackwing Lair, on the real world, as mangos-classic's ScriptDevAI
//! scripts it: the copy's doors by its encounters, which no player works,
//! and its bosses' fights. A scenario makes a timed cast due rather than
//! waiting for it, and reads what came of it as a player's client does.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use crate::game::Realm;
use crate::game::fields::*;
use crate::game::map::bwl;
use crate::game::map::{Map, MapHandle, ai, instance_script};

pub(super) const BWL: u32 = 469;

pub(super) fn door_state(m: &Map, entry: u32) -> Option<u32> {
    m.gameobjects
        .iter()
        .find(|(_, o)| o.entry == entry)
        .map(|(_, o)| o.fields.get_u32(GAMEOBJECT_STATE))
}

pub(super) fn door_guid(m: &Map, entry: u32) -> Option<Guid> {
    m.gameobjects
        .iter()
        .find(|(_, o)| o.entry == entry)
        .map(|(_, o)| o.guid)
}

pub(super) fn field(m: &Map, f: u32) -> u32 {
    instance_script::get_data(m, f)
}

/// The spawned boss of `entry` in the copy.
pub(super) async fn boss(map: &MapHandle, entry: u32) -> Guid {
    map.query(move |m| first(m, entry))
        .await
        .flatten()
        .unwrap_or_else(|| panic!("{entry} in the lair"))
}

pub(super) async fn not_gm(map: &MapHandle, g: Guid) {
    map.query(move |m| m.players.get_mut(&g).unwrap().gm = false)
        .await;
}

/// Before Broodlord Lashlayer's door.
const LASHLAYER_HALL: Vector3d = Vector3d {
    x: -7566.0,
    y: -1030.0,
    z: 449.2,
};

/// The doors stand as the encounters have them, and no player works one:
/// the main gate and Nefarian's open, the bosses' own shut; Broodlord's
/// pull shuts the main gate, his death opens his door and the gate again.
#[tokio::test]
#[ignore = "extended"]
async fn blackwing_lairs_doors_follow_its_encounters() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = hero_in(&realm, "Lairdoors", BWL, LASHLAYER_HALL, 0.0, &[]).await;
    not_gm(&map, me).await;
    for (entry, state) in [
        (bwl::DOOR_RAZORGORE_ENTER, 0),
        (bwl::DOOR_RAZORGORE_EXIT, 1),
        (bwl::DOOR_VAELASTRASZ, 1),
        (bwl::DOOR_LASHLAYER, 1),
        (bwl::DOOR_CHROMAGGUS_EXIT, 1),
        (bwl::DOOR_NEFARIAN, 0),
    ] {
        assert_eq!(
            map.query(move |m| door_state(m, entry)).await.flatten(),
            Some(state),
            "door {entry}"
        );
    }
    // A player's click on Broodlord's door does nothing.
    let door = map
        .query(|m| door_guid(m, bwl::DOOR_LASHLAYER))
        .await
        .flatten()
        .unwrap();
    c.send(CMSG_GAMEOBJ_USE { guid: door }).await;
    sleep_ms(300).await;
    assert_eq!(
        map.query(|m| door_state(m, bwl::DOOR_LASHLAYER))
            .await
            .flatten(),
        Some(1),
        "the door is the copy's to work"
    );
    let lash = boss(&map, bwl::NPC_LASHLAYER).await;
    c.sees(lash).await;
    assert!(
        map.query(|m| m
            .gameobjects
            .iter()
            .any(|(_, o)| o.entry == bwl::GO_SUPPRESSION_DEVICE))
            .await
            .unwrap(),
        "the suppression devices while he lives"
    );
    map.query(move |m| {
        tough(m, lash);
        ai::attacked(m, lash, me);
    })
    .await;
    assert!(
        heard(&mut c, lash, Duration::from_secs(2)).await.is_some(),
        "his yell at the pull"
    );
    assert!(
        until(&map, 2000, |m| field(m, bwl::LASHLAYER)
            == instance_script::IN_PROGRESS
            && door_state(m, bwl::DOOR_RAZORGORE_ENTER) == Some(1))
        .await,
        "under way, the main gate shuts"
    );
    map.query(move |m| kill(m, me, lash)).await;
    assert!(
        until(&map, 2000, |m| field(m, bwl::LASHLAYER)
            == instance_script::DONE
            && door_state(m, bwl::DOOR_LASHLAYER) == Some(0)
            && door_state(m, bwl::DOOR_RAZORGORE_ENTER) == Some(0))
        .await,
        "his door opens, and the main gate again"
    );
    assert!(
        until(&map, 2000, |m| !m
            .gameobjects
            .iter()
            .any(|(_, o)| o.entry == bwl::GO_SUPPRESSION_DEVICE))
        .await,
        "his suppression devices gone with him"
    );
}

/// Broodlord Lashlayer: Cleave at eight seconds and every seven, Knock
/// Away at twelve and every fifteen to thirty, Blast Wave at twenty and
/// every eight to sixteen, Mortal Strike at thirty and every twenty-five
/// to thirty-five; taken below his hall's floor he gives up the fight,
/// saying so.
#[tokio::test]
#[ignore = "extended"]
async fn broodlord_lashlayer_fights_as_mangos_classic() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = hero_in(&realm, "Broodfoe", BWL, LASHLAYER_HALL, 0.0, &[]).await;
    let lash = boss(&map, bwl::NPC_LASHLAYER).await;
    c.sees(lash).await;
    map.query(move |m| {
        tough(m, lash);
        ai::attacked(m, lash, me);
    })
    .await;
    let e = |n: u32| bwl::NPC_LASHLAYER * 100 + n;
    timed_cast(
        &mut c,
        &map,
        lash,
        e(1),
        15284,
        (8.0, 8.0),
        (7.0, 7.0),
        Some(me),
    )
    .await;
    timed_cast(
        &mut c,
        &map,
        lash,
        e(2),
        18670,
        (12.0, 12.0),
        (15.0, 30.0),
        Some(me),
    )
    .await;
    timed_cast(
        &mut c,
        &map,
        lash,
        e(3),
        23331,
        (20.0, 20.0),
        (8.0, 16.0),
        None,
    )
    .await;
    timed_cast(
        &mut c,
        &map,
        lash,
        e(4),
        24573,
        (30.0, 30.0),
        (25.0, 35.0),
        Some(me),
    )
    .await;
    // Down the stairs he leashes.
    let state = map
        .query(move |m| {
            let c = m.creature(lash).unwrap();
            (c.combat.attacking, c.evading, c.alive())
        })
        .await
        .unwrap();
    assert_eq!(state, (true, false, true), "still fighting");
    map.query(move |m| {
        // Within a say's reach of the hero, but below the floor.
        let below = Vector3d {
            x: -7570.0,
            y: -1034.0,
            z: 440.0,
        };
        if let Some(cr) = m.creature_mut(lash) {
            cr.position = below;
        }
        crate::game::map::instance_script::cancel_for(m, lash);
        crate::game::map::instance_script::schedule_for(
            m,
            crate::game::map::instance_adapters::TIMERS + 6000,
            lash,
            Duration::ZERO,
        );
        hurry_copy(m, Duration::from_millis(1));
    })
    .await;
    assert!(
        heard(&mut c, lash, Duration::from_secs(2)).await.is_some(),
        "his leash's words"
    );
    assert!(
        until(&map, 2000, move |m| m
            .creature(lash)
            .is_some_and(|c| c.evading || !c.combat.attacking))
        .await,
        "he gives up the fight"
    );
}

async fn drake_in(
    realm: &Arc<Realm>,
    name: &str,
    entry: u32,
) -> (crate::game::harness::Client, Guid, MapHandle, Guid) {
    let at = match entry {
        bwl::NPC_FIREMAW => Vector3d {
            x: -7412.0,
            y: -950.0,
            z: 465.2,
        },
        bwl::NPC_EBONROC => Vector3d {
            x: -7374.0,
            y: -990.0,
            z: 477.3,
        },
        _ => Vector3d {
            x: -7404.0,
            y: -1025.0,
            z: 477.3,
        },
    };
    let keep: &'static [u32] = match entry {
        bwl::NPC_FIREMAW => &[bwl::NPC_FIREMAW],
        bwl::NPC_EBONROC => &[bwl::NPC_EBONROC],
        _ => &[bwl::NPC_FLAMEGOR],
    };
    let (mut c, me, map) = hero_in(realm, name, BWL, at, 60.0, keep).await;
    let g = boss(&map, entry).await;
    c.sees(g).await;
    map.query(move |m| {
        tough(m, g);
        ai::attacked(m, g, me);
    })
    .await;
    (c, me, map, g)
}

/// Wing Buffet's hit halves the threat of whom it hits.
async fn buffet_halves_threat(
    c: &mut crate::game::harness::Client,
    map: &MapHandle,
    g: Guid,
    me: Guid,
    entry: u32,
) {
    let before = map
        .query(move |m| {
            if let Some(cr) = m.creature_mut(g)
                && let Some(t) = cr.threat.iter_mut().find(|(u, _)| *u == me)
            {
                t.1 = 1000.0;
            }
            threat(m, g, me)
        })
        .await
        .unwrap();
    assert_eq!(before, 1000.0);
    // Its cone is where it faces, and it turns to its victim on a fight
    // turn of its own: the casts above, made due in queries, can all come
    // between two updates (a fight thirty seconds old in the game, a few
    // milliseconds here), before it has turned from its spawn's facing.
    // Out of its last cast, it turns on its next update; wait for it.
    map.query(move |m| crate::game::map::spells::interrupt(m, g))
        .await;
    assert!(
        until(map, 4000, move |m| {
            crate::game::map::spells::targets::in_cone(m, g, me, 60)
        })
        .await,
        "it turns to face its victim"
    );
    map.query(move |m| {
        if let Some(cr) = m.caster_mut(g) {
            cr.gcd_until = None;
        }
        due(
            m,
            g,
            entry * 100
                + match entry {
                    bwl::NPC_EBONROC | bwl::NPC_FLAMEGOR => 3,
                    _ => 2,
                },
        );
    })
    .await;
    assert!(
        cast_any_of(c, g, 23339, Duration::from_secs(4))
            .await
            .is_some(),
        "Wing Buffet"
    );
    assert!(
        until(map, 4000, move |m| {
            let t = threat(m, g, me);
            t > 0.0 && t <= 600.0
        })
        .await,
        "its hit halves the threat on whom it hits"
    );
    map.query(move |m| crate::game::map::spells::interrupt(m, g))
        .await;
}

/// Firemaw: Shadow Flame at eighteen seconds and every fifteen to
/// eighteen, Wing Buffet at thirty and every thirty to thirty-five, Flame
/// Buffet at five and every five, Thrash at six and every two to six.
#[tokio::test]
#[ignore = "extended"]
async fn firemaw_fights_as_mangos_classic() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, g) = drake_in(&realm, "Mawfoe", bwl::NPC_FIREMAW).await;
    let e = |n: u32| bwl::NPC_FIREMAW * 100 + n;
    timed_cast(
        &mut c,
        &map,
        g,
        e(1),
        22539,
        (18.0, 18.0),
        (15.0, 18.0),
        None,
    )
    .await;
    timed_cast(&mut c, &map, g, e(3), 23341, (5.0, 5.0), (5.0, 5.0), None).await;
    timed_cast(&mut c, &map, g, e(4), 3391, (6.0, 6.0), (2.0, 6.0), None).await;
    buffet_halves_threat(&mut c, &map, g, me, bwl::NPC_FIREMAW).await;
    map.query(move |m| kill(m, me, g)).await;
    assert!(
        until(&map, 2000, |m| field(m, bwl::FIREMAW)
            == instance_script::DONE)
        .await
    );
}

/// Ebonroc: Shadow of Ebonroc on his victim at forty-five seconds and
/// every twenty-five to thirty-five, Shadow Flame, Wing Buffet and Thrash
/// as Firemaw's.
#[tokio::test]
#[ignore = "extended"]
async fn ebonroc_fights_as_mangos_classic() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, g) = drake_in(&realm, "Ebonfoe", bwl::NPC_EBONROC).await;
    let e = |n: u32| bwl::NPC_EBONROC * 100 + n;
    timed_cast(
        &mut c,
        &map,
        g,
        e(1),
        23340,
        (45.0, 45.0),
        (25.0, 35.0),
        Some(me),
    )
    .await;
    timed_cast(
        &mut c,
        &map,
        g,
        e(2),
        22539,
        (18.0, 18.0),
        (15.0, 18.0),
        None,
    )
    .await;
    timed_cast(&mut c, &map, g, e(4), 3391, (6.0, 6.0), (2.0, 6.0), None).await;
    buffet_halves_threat(&mut c, &map, g, me, bwl::NPC_EBONROC).await;
    // Home from his fight, it failed.
    map.query(move |m| ai::enter_evade(m, g)).await;
    assert!(
        until(&map, 15000, |m| field(m, bwl::EBONROC)
            == instance_script::FAILED)
        .await
    );
}

/// Flamegor: Frenzy at ten seconds and every ten to fifteen, with his
/// emote; Shadow Flame, Wing Buffet and Thrash as Firemaw's.
#[tokio::test]
#[ignore = "extended"]
async fn flamegor_fights_as_mangos_classic() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, g) = drake_in(&realm, "Gorfoe", bwl::NPC_FLAMEGOR).await;
    let e = |n: u32| bwl::NPC_FLAMEGOR * 100 + n;
    timed_cast(
        &mut c,
        &map,
        g,
        e(1),
        23342,
        (10.0, 10.0),
        (10.0, 15.0),
        None,
    )
    .await;
    assert!(
        emoted(&mut c, "Flamegor", Duration::from_secs(2))
            .await
            .is_some(),
        "his frenzy's emote"
    );
    timed_cast(
        &mut c,
        &map,
        g,
        e(2),
        22539,
        (18.0, 18.0),
        (15.0, 18.0),
        None,
    )
    .await;
    timed_cast(&mut c, &map, g, e(4), 3391, (6.0, 6.0), (2.0, 6.0), None).await;
    buffet_halves_threat(&mut c, &map, g, me, bwl::NPC_FLAMEGOR).await;
}

// Razorgore the Untamed.

use crate::game::map::bwl::razorgore as rz;

/// At the Orb of Domination.
const ORB: Vector3d = Vector3d {
    x: -7612.0,
    y: -1028.0,
    z: 413.4,
};

fn object_flags(m: &Map, entry: u32) -> u32 {
    m.gameobjects
        .iter()
        .find(|(_, o)| o.entry == entry)
        .map_or(0, |(_, o)| o.fields.get_u32(GAMEOBJECT_FLAGS))
}

fn eggs(m: &Map) -> Vec<Guid> {
    m.gameobjects
        .iter()
        .filter(|(_, o)| o.entry == rz::GO_EGG)
        .map(|(_, o)| o.guid)
        .collect()
}

fn defenders(m: &Map) -> Vec<Guid> {
    m.creatures
        .iter()
        .filter(|(_, c)| {
            c.alive()
                && c.spawn_guid.is_none()
                && matches!(
                    c.entry,
                    rz::NPC_LEGIONNAIRE | rz::NPC_MAGE | rz::NPC_DRAGONSPAWN
                )
        })
        .map(|(_, c)| c.guid)
        .collect()
}

/// Razorgore: Grethok's pull starts it and brings the defenders forty
/// seconds on, four at a time; Grethok dead, the orb is used, and its user
/// takes Razorgore over, exhausted; Razorgore breaks an egg at his
/// controller's word; the last egg broken, he is free, healed, and fights
/// to the death; his death opens the way on.
#[tokio::test]
#[ignore = "extended"]
async fn razorgore_is_taken_over_breaks_the_eggs_and_falls() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = hero_in(&realm, "Orbholder", BWL, ORB, 0.0, &[]).await;
    not_gm(&map, me).await;
    let raz = boss(&map, bwl::NPC_RAZORGORE).await;
    let grethok = boss(&map, rz::NPC_GRETHOK).await;
    c.sees(raz).await;
    assert!(
        map.query(|m| object_flags(m, rz::GO_ORB) & 0x10 != 0)
            .await
            .unwrap(),
        "the orb is dark while Grethok lives"
    );
    map.query(move |m| {
        tough(m, grethok);
        ai::attacked(m, grethok, me);
    })
    .await;
    assert!(
        until(&map, 2000, |m| field(m, bwl::RAZORGORE)
            == instance_script::IN_PROGRESS
            && door_state(m, bwl::DOOR_RAZORGORE_ENTER) == Some(1))
        .await,
        "Grethok's pull starts it, the gate shuts"
    );
    // Forty seconds on, four defenders.
    map.query(|m| hurry_copy(m, Duration::from_secs(41))).await;
    assert!(
        until(&map, 2000, |m| defenders(m).len() == 4).await,
        "four defenders from the corners"
    );
    map.query(|m| hurry_copy(m, Duration::from_secs(16))).await;
    assert!(
        until(&map, 2000, |m| defenders(m).len() == 8).await,
        "and four more fifteen seconds on"
    );
    map.query(move |m| kill(m, me, grethok)).await;
    assert!(
        until(&map, 2000, |m| object_flags(m, rz::GO_ORB) & 0x10 == 0).await,
        "Grethok dead, the orb may be used"
    );
    let orb = map
        .query(|m| door_guid(m, rz::GO_ORB))
        .await
        .flatten()
        .unwrap();
    c.send(CMSG_GAMEOBJ_USE { guid: orb }).await;
    assert!(
        until(&map, 3000, move |m| m.creature(raz).and_then(|c| c.owner)
            == Some(me))
        .await,
        "the orb's user takes Razorgore over"
    );
    assert!(
        map.query(move |m| has(m, me, rz::MIND_EXHAUSTION))
            .await
            .unwrap(),
        "exhausted"
    );
    // At his controller's word he breaks an egg by him.
    let egg = map.query(|m| eggs(m)[0]).await.unwrap();
    map.query(move |m| {
        let at = m.gameobject(egg).unwrap().position;
        crate::game::map::scripts::place(m, raz, off(at, 2.0, 0.0), 0.0);
    })
    .await;
    c.send(CMSG_PET_CAST_SPELL {
        guid: raz,
        id: rz::DESTROY_EGG,
        targets: crate::game::map::spells::wire_targets(&crate::game::map::spells::Targets {
            object: Some(egg),
            ..Default::default()
        }),
    })
    .await;
    assert!(
        until(&map, 6000, move |m| m.gameobject(egg).is_none()).await,
        "the egg broken"
    );
    assert_eq!(
        map.query(|m| field(m, bwl::RAZORGORE)).await,
        Some(instance_script::IN_PROGRESS)
    );
    // The rest broken: the second stage.
    map.query(move |m| {
        for e in eggs(m) {
            crate::game::map::spells::cast_triggered_with(
                m,
                raz,
                rz::DESTROY_EGG,
                crate::game::map::spells::Targets {
                    object: Some(e),
                    ..Default::default()
                },
            );
        }
    })
    .await;
    assert!(
        until(&map, 2000, |m| field(m, bwl::RAZORGORE) == bwl::SPECIAL).await,
        "every egg broken, the second stage"
    );
    assert!(
        until(&map, 2000, move |m| m
            .creature(raz)
            .is_some_and(|c| c.owner.is_none())
            && object_flags(m, rz::GO_ORB) & 0x10 != 0)
        .await,
        "Razorgore is free, the orb dark"
    );
    let (hp, max) = map.query(move |m| health(m, raz)).await.unwrap();
    assert_eq!(hp, max, "healed");
    assert!(
        until(&map, 13000, |m| defenders(m).is_empty()).await,
        "the defenders flee and go"
    );
    map.query(move |m| kill(m, me, raz)).await;
    assert!(
        until(&map, 2000, |m| field(m, bwl::RAZORGORE)
            == instance_script::DONE
            && door_state(m, bwl::DOOR_RAZORGORE_EXIT) == Some(0)
            && door_state(m, bwl::DOOR_RAZORGORE_ENTER) == Some(0))
        .await,
        "dead, the way on opens"
    );
}

/// Razorgore brought to death before the eggs are broken: the encounter
/// fails; he says so and goes, and thirty seconds on he and his eggs are
/// back, and Grethok to start it again.
#[tokio::test]
#[ignore = "extended"]
async fn razorgore_brought_down_too_soon_fails_and_comes_back() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = hero_in(&realm, "Toosoon", BWL, ORB, 0.0, &[]).await;
    not_gm(&map, me).await;
    let raz = boss(&map, bwl::NPC_RAZORGORE).await;
    let grethok = boss(&map, rz::NPC_GRETHOK).await;
    c.sees(raz).await;
    map.query(move |m| {
        tough(m, grethok);
        ai::attacked(m, grethok, me);
    })
    .await;
    map.query(|m| hurry_copy(m, Duration::from_secs(41))).await;
    assert!(until(&map, 2000, |m| defenders(m).len() == 4).await);
    map.query(move |m| kill(m, me, grethok)).await;
    let egg = map.query(|m| eggs(m)[0]).await.unwrap();
    map.query(move |m| {
        crate::game::map::spells::cast_triggered_with(
            m,
            raz,
            rz::DESTROY_EGG,
            crate::game::map::spells::Targets {
                object: Some(egg),
                ..Default::default()
            },
        );
    })
    .await;
    assert!(until(&map, 2000, move |m| m.gameobject(egg).is_none()).await);
    // Brought to death: he doesn't die, and the try fails.
    map.query(move |m| kill(m, me, raz)).await;
    assert!(
        until(&map, 2000, |m| field(m, bwl::RAZORGORE)
            == instance_script::FAILED)
        .await,
        "failed"
    );
    assert!(
        heard(&mut c, raz, Duration::from_secs(2)).await.is_some(),
        "his last words"
    );
    assert!(
        until(&map, 2000, |m| defenders(m).is_empty()).await,
        "the defenders gone"
    );
    assert!(
        until(&map, 7000, move |m| m
            .creature(raz)
            .is_none_or(|c| !c.alive()))
        .await,
        "he goes"
    );
    map.query(|m| hurry_copy(m, Duration::from_secs(31))).await;
    assert!(
        until(&map, 3000, move |m| first(m, bwl::NPC_RAZORGORE).is_some()
            && first(m, rz::NPC_GRETHOK).is_some()
            && eggs(m).len() == 30)
        .await,
        "Razorgore, his eggs and Grethok back"
    );
}

/// Grethok pulled and the raid dead before Razorgore is ever fought: at
/// the next wave the try fails (mangos's leash for him: no living player
/// not a GM in the map), so the defenders go and the hall's gate opens
/// for the raid running back, rather than the waves going on and the gate
/// shut while anyone stays inside.
#[tokio::test]
#[ignore = "extended"]
async fn razorgore_fails_when_the_raid_dies_before_he_is_fought() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = hero_in(&realm, "Wiped", BWL, ORB, 0.0, &[]).await;
    not_gm(&map, me).await;
    let raz = boss(&map, bwl::NPC_RAZORGORE).await;
    let grethok = boss(&map, rz::NPC_GRETHOK).await;
    c.sees(raz).await;
    map.query(move |m| {
        tough(m, grethok);
        ai::attacked(m, grethok, me);
    })
    .await;
    map.query(|m| hurry_copy(m, Duration::from_secs(41))).await;
    assert!(until(&map, 2000, |m| defenders(m).len() == 4).await);
    map.query(move |m| crate::game::map::death::kill(m, me))
        .await;
    map.query(|m| hurry_copy(m, Duration::from_secs(16))).await;
    assert!(
        until(&map, 3000, |m| field(m, bwl::RAZORGORE)
            == instance_script::FAILED)
        .await,
        "failed"
    );
    assert!(
        until(&map, 2000, |m| defenders(m).is_empty()
            && door_state(m, bwl::DOOR_RAZORGORE_ENTER) == Some(0))
        .await,
        "the defenders gone, the way in open"
    );
}
