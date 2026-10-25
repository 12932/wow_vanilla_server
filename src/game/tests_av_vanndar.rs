//! Vanndar Stormpike and the marshals as vmangos's C++ has them
//! (`battleground_alterac.cpp`, decision 103), through the protocol:
//! Vanndar's global cooldown of a second between his three spells
//! (`npc_VanndarAI`'s `m_uiGlobalCooldown`), and the scripts' own notice
//! radii past both bounding radii (`MoveInLineOfSight`: 23 yards for
//! Vanndar, 22 for a marshal), the marshal charging the one it notices.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::{casts_by, index, left};
use super::tests_p8_scripts_av::{
    AV, casting, instants, no_gcd, pair, place, put, real_shaped_charge, spawn_at, spells_of,
};
use super::tests_pvp2::{creature, npc, valley};
use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::battleground::av_scripts::{self, AVATAR, STORM_BOLT, THUNDERCLAP};
use crate::game::map::{Map, MapHandle, ai, event_ai};

const VANNDAR: u32 = 11948;
const HOME: Vector3d = Vector3d {
    x: 722.4,
    y: -11.0,
    z: 50.7,
};
const MARSHAL: u32 = 14762;
const MARSHAL_AT: Vector3d = Vector3d {
    x: 100.0,
    y: 50.0,
    z: 0.0,
};

/// Vanndar spawned at his home (his leash is about it), of the real
/// template's level and detection range: by level he would notice a
/// level-60 player at 22 yards between centres.
fn vanndar_at_home(d: &mut GameData) {
    if let Some(s) = d
        .creature_spawns
        .get_mut(&AV)
        .and_then(|s| s.iter_mut().find(|s| s.entries == [VANNDAR]))
    {
        (s.x, s.y, s.z) = (HOME.x, HOME.y, HOME.z);
    }
    if let Some(t) = d.creature_templates.get_mut(&VANNDAR) {
        (t.level_min, t.level_max, t.detection_range) = (62, 62, 20.0);
    }
}

/// Faction 14, a monster's (the fixture's generals' and marshal's),
/// hostile to Alice's and Grom's, 1 and 2.
fn monsters(d: &mut GameData) {
    for (id, ours, enemy_mask) in [(14, 0x8, 0x7), (1, 0x3, 0x8), (2, 0x5, 0x8)] {
        d.faction_templates
            .entry(id)
            .or_insert(crate::game::data::FactionTemplate {
                id,
                faction: id,
                ours,
                enemy_mask,
                ..Default::default()
            });
    }
}

fn radius(m: &Map, u: Guid) -> f32 {
    m.unit_fields(u)
        .map_or(0.0, |f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS).max(0.0))
}

/// `g`'s timed events looked at now.
fn tick_now(m: &mut Map, g: Guid) {
    if let Some(s) = event_ai::scripted_mut(m, g) {
        s.next_tick = None;
    }
    event_ai::tick(m, g);
}

/// What is left of `g`'s script's global cooldown.
fn gcd_left(m: &Map, g: Guid) -> Option<Duration> {
    let s = event_ai::scripted(m, g)?;
    s.gcd_until.map(|t| t.saturating_duration_since(m.now))
}

/// Vanndar's spells go a second apart (vmangos's `npc_VanndarAI`): with
/// Avatar, Storm Bolt and Thunderclap all due, Avatar goes and sets his
/// global cooldown of a second; the others wait, due, while a millisecond
/// of it is left, and Storm Bolt goes once it is up, setting it again;
/// then Thunderclap. The map's clock is skipped: the cooldown is set to
/// what a second's passing leaves of it.
#[tokio::test]
async fn vanndars_three_spells_are_at_least_a_second_apart() {
    let mut d = valley();
    monsters(&mut d);
    vanndar_at_home(&mut d);
    instants(&mut d, &[STORM_BOLT], &[AVATAR, THUNDERCLAP]);
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (_a, _, mut b, grom, map) = pair(&realm).await;
    put(&map, &mut b, HOME.x + 2.0, HOME.y, HOME.z).await;
    let v = map.query(|m| creature(m, VANNDAR)).await.flatten().unwrap();
    b.sees_on(&map, v).await;
    let spells = [AVATAR, STORM_BOLT, THUNDERCLAP];
    let events = map
        .query(move |m| {
            ai::attacked(m, v, grom);
            place(m, v, HOME);
            spells.map(|s| casting(m, VANNDAR, s))
        })
        .await
        .unwrap();
    // All three due at once.
    b.clear();
    let first = map
        .query(move |m| {
            no_gcd(m, v);
            let now = m.now;
            let at = events.map(|e| index(m, v, e));
            if let Some(s) = event_ai::scripted_mut(m, v) {
                for i in at {
                    s.timers[i] = Some(now);
                }
            }
            tick_now(m, v);
            gcd_left(m, v)
        })
        .await
        .flatten();
    let mut seen = vec![(
        spells_of(&casts_by(&mut b, v, Duration::from_millis(300)).await),
        first,
    )];
    // A second less a millisecond on, then a second on, twice.
    for left_of_it in [1, 0, 1, 0] {
        b.clear();
        let gcd = map
            .query(move |m| {
                let until = m.now + Duration::from_millis(left_of_it);
                if let Some(c) = m.caster_mut(v) {
                    c.gcd_until = None;
                }
                if let Some(s) = event_ai::scripted_mut(m, v) {
                    s.gcd_until = Some(until);
                }
                tick_now(m, v);
                gcd_left(m, v)
            })
            .await
            .flatten();
        let went = spells_of(&casts_by(&mut b, v, Duration::from_millis(300)).await);
        seen.push((went, gcd.filter(|_| left_of_it == 0)));
    }
    let second = Some(Duration::from_secs(1));
    assert_eq!(
        seen,
        vec![
            (vec![AVATAR], second),
            (vec![], None),
            (vec![STORM_BOLT], second),
            (vec![], None),
            (vec![THUNDERCLAP], second),
        ],
        "(cast, the cooldown it set): one at a time, a second apart"
    );
}

/// Whether `c` is told `attacker` targets `victim` before `within` is up
/// (its `UNIT_FIELD_TARGET` in a values update).
async fn attacks(c: &mut Client, attacker: Guid, victim: Guid, within: Duration) -> bool {
    c.gets(within, move |m| {
        harness::values(m)
            .into_iter()
            .any(|(g, mask)| {
                g == attacker
                    && matches!(mask, UpdateMask::Unit(u) if u.unit_target() == Some(victim))
            })
            .then_some(())
    })
    .await
    .is_some()
}

/// `g` looks about at once, and the map runs on a few updates.
async fn look_about(map: &MapHandle, g: Guid) {
    map.query(move |m| {
        let now = m.now;
        if let Some(c) = m.creature_mut(g) {
            c.aggro_check_at = now;
        }
    })
    .await;
    for _ in 0..3 {
        harness::an_update(map).await;
    }
}

/// `who` put `gap` yards past both bounding radii from `g` at `at`, along
/// x, and `g` looks about: whether it went for them, and has them for its
/// victim.
async fn noticed_at(
    map: &MapHandle,
    c: &mut Client,
    who: Guid,
    g: Guid,
    at: Vector3d,
    gap: f32,
) -> (bool, bool) {
    let radii = map
        .query(move |m| radius(m, g) + radius(m, who))
        .await
        .unwrap();
    put(map, c, at.x + gap + radii, at.y, at.z).await;
    c.clear();
    look_about(map, g).await;
    let went = attacks(c, g, who, Duration::from_millis(300)).await;
    let victim = map
        .query(move |m| m.creature(g).map(|c| c.combat.victim))
        .await
        .flatten();
    (went, victim == Some(who))
}

/// vmangos's `npc_Vanndar::MoveInLineOfSight` notices a hostile unit short
/// of 23 yards past both bounding radii (`GetDistance(m_creature) < 23`):
/// a player at 22.5 is noticed, at 23.5 not. By his detection range and
/// level (20 and 62: 22 yards between centres for a level-60 player) the
/// first would not have been.
#[tokio::test]
async fn vanndar_notices_a_player_at_twenty_two_and_a_half_yards_not_at_twenty_three_and_a_half() {
    let mut d = valley();
    monsters(&mut d);
    vanndar_at_home(&mut d);
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (_a, _, mut b, grom, map) = pair(&realm).await;
    map.query(move |m| {
        if let Some(p) = m.players.get_mut(&grom) {
            p.fields.set_u32(UNIT_FIELD_LEVEL, 60);
        }
    })
    .await;
    put(&map, &mut b, HOME.x + 60.0, HOME.y, HOME.z).await;
    let v = map.query(|m| creature(m, VANNDAR)).await.flatten().unwrap();
    b.sees_on(&map, v).await;
    let mut seen = Vec::new();
    for gap in [23.5, 22.5] {
        seen.push(noticed_at(&map, &mut b, grom, v, HOME, gap).await);
    }
    assert_eq!(
        seen,
        vec![(false, false), (true, true)],
        "(went for him, his victim): at 23.5 yards, at 22.5"
    );
}

/// vmangos's `npc_WarMasterAI::MoveInLineOfSight`: a marshal notices a
/// unit short of 22 yards past both bounding radii, attacks and charges
/// it, and if the Charge goes its timer is 12 s on. Grom at 22.5 yards
/// isn't noticed; at 21.5 he is, and charged. Alice, coming as near once
/// it fights him, is not noticed: it has a victim.
#[tokio::test]
async fn a_marshal_notices_at_twenty_two_yards_and_charges_the_first_it_notices() {
    let mut d = valley();
    monsters(&mut d);
    npc(&mut d, MARSHAL, "Dun Baldar North Marshal", 14);
    if let Some(t) = d.creature_templates.get_mut(&MARSHAL) {
        t.detection_range = 20.0;
    }
    spawn_at(
        &mut d,
        170601,
        MARSHAL,
        (MARSHAL_AT.x, MARSHAL_AT.y, MARSHAL_AT.z),
    );
    instants(
        &mut d,
        &[av_scripts::CLEAVE],
        &[av_scripts::WHIRLWIND, av_scripts::DEMORALIZING_SHOUT],
    );
    real_shaped_charge(&mut d);
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (mut a, alice, mut b, grom, map) = pair(&realm).await;
    put(
        &map,
        &mut a,
        MARSHAL_AT.x,
        MARSHAL_AT.y + 60.0,
        MARSHAL_AT.z,
    )
    .await;
    put(
        &map,
        &mut b,
        MARSHAL_AT.x + 60.0,
        MARSHAL_AT.y,
        MARSHAL_AT.z,
    )
    .await;
    let marshal = map.query(|m| creature(m, MARSHAL)).await.flatten().unwrap();
    b.sees_on(&map, marshal).await;
    a.sees_on(&map, marshal).await;
    let charge = map
        .query(|m| casting(m, MARSHAL, av_scripts::CHARGE))
        .await
        .unwrap();
    let far = noticed_at(&map, &mut b, grom, marshal, MARSHAL_AT, 22.5).await;
    let radii = map
        .query(move |m| radius(m, marshal) + radius(m, grom))
        .await
        .unwrap();
    put(
        &map,
        &mut b,
        MARSHAL_AT.x + 21.5 + radii,
        MARSHAL_AT.y,
        MARSHAL_AT.z,
    )
    .await;
    b.clear();
    // The Charge goes as it notices him, in the same update.
    map.query(move |m| {
        let now = m.now;
        if let Some(c) = m.creature_mut(marshal) {
            c.aggro_check_at = now;
        }
    })
    .await;
    let casts = casts_by(&mut b, marshal, Duration::from_millis(600)).await;
    let (victim, timer) = map
        .query(move |m| {
            (
                m.creature(marshal).map(|c| c.combat.victim),
                left(m, marshal, charge),
            )
        })
        .await
        .unwrap();
    assert_eq!(far, (false, false), "not at 22.5 yards");
    assert!(casts.contains(&(av_scripts::CHARGE, grom)), "{casts:?}");
    assert_eq!(victim, Some(grom));
    assert!(
        timer.is_some_and(|t| (Duration::from_secs(10)..=Duration::from_secs(12)).contains(&t)),
        "its Charge 12 s on: {timer:?}"
    );
    // Alice beside it now, ten yards off: it fights Grom, and notices
    // no one else.
    let here = map
        .query(move |m| m.creature(marshal).map(|c| c.position))
        .await
        .flatten()
        .unwrap();
    a.clear();
    put(&map, &mut a, here.x, here.y + 10.0, here.z).await;
    look_about(&map, marshal).await;
    let later = casts_by(&mut a, marshal, Duration::from_millis(300)).await;
    let on_list = map
        .query(move |m| {
            m.creature(marshal)
                .is_some_and(|c| c.threat.iter().any(|(t, _)| *t == alice))
        })
        .await
        .unwrap();
    assert!(!later.contains(&(av_scripts::CHARGE, alice)), "{later:?}");
    assert!(!on_list, "Alice isn't noticed");
}

/// A scripted notice needs sight (decision 104, the user's call: "there's
/// supposed to be line of sight"): Vanndar doesn't notice Grom 15 yards
/// off behind a wall, and does once the wall is gone. vmangos's scripts
/// test the distance alone, so its generals and their guards notice
/// through walls; wvs's look as every creature's notice does.
#[tokio::test]
async fn vanndar_notices_no_one_behind_a_wall() {
    let mut d = valley();
    monsters(&mut d);
    vanndar_at_home(&mut d);
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (_a, _, mut b, grom, map) = pair(&realm).await;
    map.query(move |m| {
        if let Some(p) = m.players.get_mut(&grom) {
            p.fields.set_u32(UNIT_FIELD_LEVEL, 60);
        }
    })
    .await;
    put(&map, &mut b, HOME.x + 60.0, HOME.y, HOME.z).await;
    let v = map.query(|m| creature(m, VANNDAR)).await.flatten().unwrap();
    b.sees_on(&map, v).await;
    // A wall across the way between them, 7 yards from him.
    map.query(|m| {
        m.spells.walls.push((
            Vector3d { x: HOME.x + 7.0, y: HOME.y - 20.0, z: HOME.z },
            Vector3d { x: HOME.x + 7.0, y: HOME.y + 20.0, z: HOME.z },
        ));
    })
    .await;
    let behind = noticed_at(&map, &mut b, grom, v, HOME, 15.0).await;
    map.query(|m| m.spells.walls.clear()).await;
    let open = noticed_at(&map, &mut b, grom, v, HOME, 15.0).await;
    assert_eq!(
        (behind, open),
        ((false, false), (true, true)),
        "(went for him, his victim): behind the wall, then in the open"
    );
}
