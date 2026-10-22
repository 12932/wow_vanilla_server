//! Nefarian's lair on the real world, as mangos-classic scripts it: Lord
//! Victor Nefarius's game, the drakonids, and Nefarian.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use super::tests_p12l_bwl::*;
use super::tests_p12l_bwl2::{joins_as, says, summoned};
use crate::game::map::bwl;
use crate::game::map::bwl::nefarian as nf;
use crate::game::map::{Map, MapHandle, ai, instance_script};

/// At Nefarius's table.
const TABLE: Vector3d = Vector3d {
    x: -7586.0,
    y: -1260.5,
    z: 482.0,
};

fn living_drakonids(m: &Map) -> Vec<(u32, Guid)> {
    let mut v: Vec<(u32, Guid)> = m
        .creatures
        .iter()
        .filter(|(_, c)| {
            c.alive()
                && matches!(
                    c.entry,
                    nf::NPC_BLUE_DRAKONID
                        | nf::NPC_GREEN_DRAKONID
                        | nf::NPC_BRONZE_DRAKONID
                        | nf::NPC_RED_DRAKONID
                        | nf::NPC_BLACK_DRAKONID
                        | nf::NPC_CHROMATIC_DRAKONID
                )
        })
        .map(|(_, c)| (c.entry, c.guid))
        .collect();
    v.sort();
    v
}

/// The spawned Nefarius at his table.
fn table_nefarius(m: &Map) -> Option<Guid> {
    m.creatures
        .iter()
        .find(|(_, c)| c.entry == bwl::NPC_VICTOR_NEFARIUS && c.spawn_guid.is_some() && c.alive())
        .map(|(_, c)| c.guid)
}

/// Through Nefarius's three menus, Chromaggus dead: his game begins.
async fn start_the_game(
    realm: &std::sync::Arc<crate::game::Realm>,
    c: &mut crate::game::harness::Client,
    map: &MapHandle,
) -> Guid {
    let nef = map
        .query(|m| table_nefarius(m))
        .await
        .flatten()
        .expect("Nefarius at his table");
    c.sees(nef).await;
    map.query(|m| bwl::set_data(m, bwl::CHROMAGGUS, instance_script::DONE))
        .await;
    hello(c, nef).await;
    let o = option(c, nef, "I've made no mistakes.").await;
    choose(c, nef, o).await;
    let o = option(c, nef, "You have lost your mind").await;
    choose(c, nef, o).await;
    let o = option(c, nef, "Please do.").await;
    choose(c, nef, o).await;
    assert!(
        says(realm, c, nef, nf::SAY_GAMESBEGIN_1).await,
        "let the games begin"
    );
    map.query(|m| hurry_copy(m, Duration::from_secs(5))).await;
    assert!(says(realm, c, nef, nf::SAY_GAMESBEGIN_2).await);
    map.query(|m| hurry_copy(m, Duration::from_secs(6))).await;
    assert!(
        until(map, 3000, move |m| field(m, bwl::NEFARIAN)
            == instance_script::IN_PROGRESS
            && has(m, nef, nf::NEFARIUS_BARRIER)
            && faction(m, nef) == 103
            && door_state(m, bwl::DOOR_NEFARIAN) == Some(1))
        .await,
        "he rises behind his barrier, the door shut"
    );
    nef
}

/// Nefarius's game and Nefarian: the menus start it; drakonids of the
/// copy's two kinds from the tunnels, and the chromatic ones; 42 dead,
/// Nefarius leaves and Nefarian flies in, lands and fights; his class
/// call reaches its class alone; below 20 percent he raises the bones; his
/// death wins the lair.
#[tokio::test]
#[ignore = "extended"]
async fn nefarians_lair_runs_as_mangos_classic() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = hero_in(&realm, "Nefslayer", BWL, TABLE, 0.0, &[]).await;
    not_gm(&map, me).await;
    let (_mc, mage) = joins_as(&realm, "Nefmage", Class::Mage, &map, TABLE).await;
    let nef = start_the_game(&realm, &mut c, &map).await;
    let (left, right) = map
        .query(|m| (field(m, bwl::NEFA_LTUNNEL), field(m, bwl::NEFA_RTUNNEL)))
        .await
        .unwrap();
    assert!(nf::TUNNEL_KINDS.contains(&left) && nf::TUNNEL_KINDS.contains(&right));
    assert_ne!(left, right, "two kinds");
    map.query(|m| hurry_copy(m, Duration::from_secs(6))).await;
    let mut kinds = vec![left, right];
    kinds.sort();
    let want = kinds.clone();
    assert!(
        until(&map, 3000, move |m| living_drakonids(m)
            .iter()
            .map(|(e, _)| *e)
            .collect::<Vec<_>>()
            == want)
        .await,
        "one of each kind from the tunnels"
    );
    map.query(|m| hurry_copy(m, Duration::from_secs(4))).await;
    assert!(
        until(&map, 3000, |m| living_drakonids(m)
            .iter()
            .filter(|(e, _)| *e == nf::NPC_CHROMATIC_DRAKONID)
            .count()
            == 2)
        .await,
        "and a chromatic one from each"
    );
    // Forty-two dead.
    for _ in 0..30 {
        let dead = map
            .query(move |m| {
                for (_, g) in living_drakonids(m) {
                    kill(m, me, g);
                }
                hurry_copy(m, Duration::from_secs(7));
                m.instance
                    .scratch
                    .get(&nf::DEAD_DRAKONIDS)
                    .copied()
                    .unwrap_or(0)
            })
            .await
            .unwrap();
        if dead >= nf::MAX_DRAKONID_SUMMONS {
            break;
        }
        sleep_ms(30).await;
    }
    let state = map
        .query(|m| {
            (
                field(m, bwl::NEFARIAN),
                m.instance.scratch.get(&nf::DEAD_DRAKONIDS).copied(),
                living_drakonids(m).len(),
            )
        })
        .await;
    assert!(
        until(&map, 2000, |m| field(m, bwl::NEFARIAN) == bwl::SPECIAL).await,
        "42 dead, the second stage: {state:?}"
    );
    map.query(|m| {
        for (_, g) in living_drakonids(m) {
            crate::game::map::scripts::despawn(m, g);
        }
        hurry_copy(m, Duration::from_secs(5));
    })
    .await;
    let nefarian = {
        let mut g = None;
        for _ in 0..80 {
            g = map
                .query(|m| summoned(m, bwl::NPC_NEFARIAN))
                .await
                .flatten();
            if g.is_some() {
                break;
            }
            sleep_ms(25).await;
        }
        g.expect("Nefarian comes")
    };
    assert!(
        until(&map, 4000, move |m| m
            .creature(nef)
            .is_none_or(|c| !c.alive()))
        .await,
        "Nefarius blinks away"
    );
    c.sees(nefarian).await;
    map.query(move |m| {
        tough(m, nefarian);
        hurry_copy(m, Duration::from_secs(7));
    })
    .await;
    assert!(
        says(&realm, &mut c, nefarian, nf::SAY_AGGRO).await,
        "his words"
    );
    map.query(|m| hurry_copy(m, Duration::from_secs(5))).await;
    map.query(|m| hurry_copy(m, Duration::from_secs(5))).await;
    assert!(
        until(&map, 3000, move |m| m
            .creature(nefarian)
            .is_some_and(|c| c.combat.attacking)
            && unit_flags(m, nefarian) & 0x0200_0000 == 0)
        .await,
        "landed, he fights"
    );
    // His Cleave seven seconds into it.
    let e = |n: u32| bwl::NPC_NEFARIAN * 100 + n;
    map.query(move |m| ai::attacked(m, nefarian, me)).await;
    timed_cast(
        &mut c,
        &map,
        nefarian,
        e(4),
        nf::CLEAVE,
        (7.0, 7.0),
        (7.0, 7.0),
        None,
    )
    .await;
    // A class call, on the warrior's or the mage's class: that class alone.
    c.clear();
    map.query(move |m| {
        for (t, k, g) in m.instance.creature_timers.iter_mut() {
            if *g == nefarian && *k == crate::game::map::instance_adapters::TIMERS + 6414 {
                *t = m.now;
            }
        }
        if let Some(cr) = m.caster_mut(nefarian) {
            cr.gcd_until = None;
        }
        crate::game::map::instance_script::update(m);
    })
    .await;
    let warrior = 23397;
    let wild_magic = 23410;
    assert!(
        until(&map, 3000, move |m| has(m, me, warrior)
            || has(m, mage, wild_magic))
        .await,
        "a class call"
    );
    let (w_on_mage, m_on_warrior) = map
        .query(move |m| (has(m, mage, warrior), has(m, me, wild_magic)))
        .await
        .unwrap();
    assert!(!w_on_mage && !m_on_warrior, "its class alone");
    // Below 20 percent, the bones raised.
    map.query(move |m| {
        set_pct(m, nefarian, 15);
        if let Some(cr) = m.caster_mut(nefarian) {
            cr.gcd_until = None;
            cr.casting = None;
        }
        if let Some(s) = crate::game::map::event_ai::scripted_mut(m, nefarian) {
            s.next_tick = None;
        }
        crate::game::map::event_ai::tick(m, nefarian);
    })
    .await;
    assert!(
        says(&realm, &mut c, nefarian, nf::SAY_RAISE_SKELETONS).await,
        "he raises the bones"
    );
    map.query(move |m| kill(m, me, nefarian)).await;
    assert!(
        until(&map, 2000, |m| field(m, bwl::NEFARIAN)
            == instance_script::DONE
            && door_state(m, bwl::DOOR_NEFARIAN) == Some(0))
        .await,
        "his death wins the lair"
    );
}

/// Nefarian out of his fight: lost; the drakonids and their count go, and
/// Nefarius is back at his table to begin again.
#[tokio::test]
#[ignore = "extended"]
async fn nefarians_lair_lost_is_put_back() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = hero_in(&realm, "Neflost", BWL, TABLE, 0.0, &[]).await;
    not_gm(&map, me).await;
    let nef = start_the_game(&realm, &mut c, &map).await;
    map.query(|m| hurry_copy(m, Duration::from_secs(10))).await;
    assert!(until(&map, 3000, |m| !living_drakonids(m).is_empty()).await);
    // Nefarius home from his fight: lost.
    map.query(move |m| crate::game::map::ai::enter_evade(m, nef))
        .await;
    assert!(
        until(&map, 15_000, |m| field(m, bwl::NEFARIAN)
            == instance_script::FAILED)
        .await,
        "lost"
    );
    assert!(
        until(&map, 2000, |m| living_drakonids(m).is_empty()
            && door_state(m, bwl::DOOR_NEFARIAN) == Some(0))
        .await,
        "the drakonids gone, the door open"
    );
    assert!(
        until(&map, 3000, move |m| m.unit_fields(nef).is_some_and(|f| f
            .get_u32(crate::game::fields::UNIT_NPC_FLAGS)
            & 1
            != 0))
        .await,
        "Nefarius at his table to begin again"
    );
    // As he was before the fight: friendly, not to be struck, and his
    // menu to begin the fight again offered.
    let table = map
        .query(move |m| {
            m.unit_fields(nef).map(|f| {
                (
                    f.get_u32(crate::game::fields::UNIT_FIELD_FACTIONTEMPLATE),
                    f.get_u32(crate::game::fields::UNIT_FIELD_FLAGS) & 0x100,
                )
            })
        })
        .await
        .flatten();
    assert_eq!(table, Some((35, 0x100)), "friendly, immune to players");
    hello(&mut c, nef).await;
    option(&mut c, nef, "I've made no mistakes.").await;
}
