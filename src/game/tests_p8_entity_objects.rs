//! Round 8's entity tests of game objects, ported from Thistle Tea's
//! `test/game/world/entity/game_object/*_test.exs` (docs/parity/matrix.csv):
//! the Ghost Magnet's call of Magrami Spectres, and the cheer speakers'
//! fireworks shows.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;

use crate::game::cheer_sites::{CHEERS, FIREWORKS};
use crate::game::cheer_speakers::{self as cheer, SPEAKER};
use crate::game::data::GameData;
use crate::game::ghost_magnet::{self as gm, AURA, MAGNET, SPECTRE};
use crate::game::map::Map;
use crate::game::map::combat::dist;
use crate::game::map::creature::Motion;
use crate::game::map::objects;
use crate::game::tests_p3_combat::{data, setup_with, template};
use crate::game::tests_p3_entity_objects::object;

/// Another object, of no behaviour of its own.
const OTHER: u32 = 177_748;

fn world() -> GameData {
    let mut d = data();
    for t in [
        object(MAGNET, 5, "Ghost Magnet", &[(3, AURA)]),
        object(AURA, 6, "Ghost Magnet Aura", &[]),
        object(OTHER, 5, "Other", &[]),
        object(SPEAKER, 1, "Cheer Speaker", &[]),
    ] {
        d.gameobject_templates.insert(t.entry, t);
    }
    for f in FIREWORKS {
        d.gameobject_templates
            .insert(f, object(f, 10, "Firework", &[]));
    }
    let mut spectre = template(SPECTRE, 6);
    spectre.name = "Magrami Spectre".into();
    d.creature_templates.insert(SPECTRE, spectre);
    d
}

/// A magnet (or another object) set down at `g`'s feet, as the Place
/// Ghost Magnet spell's wild summon does (no owner, its spell's time).
fn set_down(m: &mut Map, g: Guid, entry: u32) -> Guid {
    let at = m.players[&g].position();
    objects::summon(m, None, entry, at, 0.0, 600_000, 19_588, 0, None).unwrap()
}

fn objects_of(m: &Map, entry: u32) -> Vec<Guid> {
    m.gameobjects
        .iter()
        .filter(|(_, o)| o.entry == entry)
        .map(|(_, o)| o.guid)
        .collect()
}

fn spectres(m: &Map) -> Vec<Guid> {
    m.creatures
        .iter()
        .filter(|(_, c)| c.entry == SPECTRE)
        .map(|(_, c)| c.guid)
        .collect()
}

/// A magnet set down raises its aura where it stands, for two minutes, and
/// readies eight spectres for those two minutes, the first five seconds
/// on; another object starts nothing; a magnet set within thirty yards of
/// a glowing aura starts nothing either (Thistle Tea's "a magnet raises
/// its aura and readies eight spectres for two minutes" and "other objects
/// are left alone"; vmangos's `go_ghost_magnetAI`).
#[tokio::test]
async fn a_magnet_raises_its_aura_and_readies_eight_spectres_for_two_minutes() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let seen = map
        .query(move |m| {
            let now = m.now;
            let magnet = set_down(m, ga, MAGNET);
            let at = m.gameobject(magnet).unwrap().position;
            let state = m.magnets[&magnet].clone();
            let auras = objects_of(m, AURA);
            let aura = auras.first().copied();
            let glow = aura.map(|a| {
                let s = m.objects.summoned.iter().find(|s| s.guid == a).unwrap();
                (
                    dist(m.gameobject(a).unwrap().position, at),
                    s.owner,
                    s.expires == Some(now + gm::CALLING),
                )
            });
            let other = set_down(m, ga, OTHER);
            let second = set_down(m, ga, MAGNET);
            (
                (state.remaining, state.until == now + gm::CALLING),
                state.next_call == Some(now + gm::FIRST_CALL),
                auras.len(),
                glow,
                m.magnets.contains_key(&other),
                m.magnets.contains_key(&second),
                objects_of(m, AURA).len(),
            )
        })
        .await
        .unwrap();
    assert_eq!(seen.0, (8, true), "eight, for two minutes");
    assert!(seen.1, "the first five seconds on");
    assert_eq!(seen.2, 1, "one aura");
    assert_eq!(
        seen.3,
        Some((0.0, None, true)),
        "at it, no one's, two minutes"
    );
    assert!(!seen.4, "another object starts nothing");
    assert!(!seen.5, "nor a magnet by a glowing aura");
    assert_eq!(seen.6, 1, "which raises no aura");
}

/// A magnet's call brings a spectre from within forty yards, out for two
/// minutes (timed or dead), which makes the spot a yard and a half short
/// of the magnet on its side its home and walks there to the point its
/// script turns on (2); the magnet has seven left. Eight called, it calls
/// no more; nor once its two minutes are up (Thistle Tea's "calls a spectre
/// that makes the magnet its home and walks to it" and "stops once its
/// eight spectres are out or its two minutes are up").
#[tokio::test]
async fn a_magnets_spectres_come_to_it_eight_in_its_two_minutes() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let seen = map
        .query(move |m| {
            let now = m.now;
            let magnet = set_down(m, ga, MAGNET);
            let at = m.gameobject(magnet).unwrap().position;
            // Not yet: its first is five seconds on.
            gm::update(m);
            let early = spectres(m).len();
            m.magnets.get_mut(&magnet).unwrap().next_call = Some(now);
            gm::update(m);
            let left = m.magnets[&magnet].remaining;
            let gap = m.magnets[&magnet].next_call.map(|t| t - now);
            let s = spectres(m)[0];
            let c = m.creature(s).unwrap();
            let from = c.position;
            let scripted = c.scripted.as_deref().unwrap();
            let walking = match &c.motion {
                Motion::Moving(w) => w.points.last().copied(),
                _ => None,
            };
            let called = (
                scripted.summoner == Some(magnet),
                scripted.despawn_type,
                scripted.despawn_after == Some(gm::SPECTRE_LIFETIME),
                scripted.despawn_at == Some(now + gm::SPECTRE_LIFETIME),
                scripted.scripted_move.map(|x| x.point),
            );
            let home = c.home;
            let near = dist(from, at) <= gm::CALL_RADIUS + 0.01;
            let short = dist(home, at);
            let toward = dist(home, from) + short - dist(from, at);
            for _ in 0..12 {
                m.magnets.get_mut(&magnet).unwrap().next_call = Some(now);
                gm::update(m);
            }
            let all = spectres(m).len();
            // A fresh magnet past its two minutes calls none.
            for s in spectres(m) {
                crate::game::map::scripts::despawn(m, s);
            }
            for o in objects_of(m, AURA) {
                objects::despawn(m, o);
            }
            objects::despawn(m, magnet);
            let late = set_down(m, ga, MAGNET);
            let mg = m.magnets.get_mut(&late).unwrap();
            mg.next_call = Some(now);
            mg.until = now;
            gm::update(m);
            (
                early,
                left,
                gap.is_some_and(|g| (Duration::from_secs(3)..=Duration::from_secs(8)).contains(&g)),
                called,
                near,
                (short - gm::CONTACT).abs() < 0.01 || short < 0.01,
                toward < 0.01,
                walking.is_some_and(|w| dist(w, home) < 0.01),
                all,
                spectres(m).len(),
                m.magnets.contains_key(&late),
            )
        })
        .await
        .unwrap();
    assert_eq!(seen.0, 0, "none before five seconds");
    assert_eq!(seen.1, 7, "seven left");
    assert!(seen.2, "the next three to eight seconds on");
    assert_eq!(
        seen.3,
        (true, 1, true, true, Some(Some(gm::MAGNET_POINT))),
        "the magnet's, timed or dead for two minutes, walking to its point"
    );
    assert!(seen.4, "from within forty yards");
    assert!(seen.5, "home a yard and a half short of the magnet");
    assert!(seen.6, "on the spectre's side");
    assert!(seen.7, "walking home");
    assert_eq!(seen.8, 8, "eight in all");
    assert_eq!(seen.9, 0, "none past its two minutes");
    assert!(!seen.10, "and it is done");
}

/// A slain spectre is replaced twenty seconds on while the magnet still
/// calls, over its eight; once the magnet's two minutes are up, not
/// (Thistle Tea's "a slain spectre is replaced only while the magnet still
/// calls").
#[tokio::test]
async fn a_slain_spectre_is_replaced_while_the_magnet_calls() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let seen = map
        .query(move |m| {
            let now = m.now;
            let magnet = set_down(m, ga, MAGNET);
            for _ in 0..8 {
                m.magnets.get_mut(&magnet).unwrap().next_call = Some(now);
                gm::update(m);
            }
            let out = spectres(m);
            crate::game::map::combat::kill(m, ga, out[0]);
            let due = m.magnets[&magnet].replacements.clone();
            gm::update(m);
            let before = spectres(m).len();
            m.magnets.get_mut(&magnet).unwrap().replacements = vec![now];
            gm::update(m);
            let replaced = spectres(m).len();
            // Past its time, a death brings none.
            crate::game::map::combat::kill(m, ga, out[1]);
            let mg = m.magnets.get_mut(&magnet).unwrap();
            mg.replacements = vec![now];
            mg.until = now;
            gm::update(m);
            (
                due == vec![now + gm::REPLACE_AFTER],
                before,
                replaced,
                spectres(m).len(),
            )
        })
        .await
        .unwrap();
    assert!(seen.0, "twenty seconds on");
    assert_eq!(seen.1, 8, "not at once (the slain one's corpse counted)");
    assert_eq!(seen.2, 9, "a ninth, replacing it");
    assert_eq!(seen.3, 9, "none once its time is up");
}

/// A cheer speaker's show: as it comes, its first firework is one to two
/// seconds off, and with it its cheer, one of the four, to those who have
/// it; another object has no show; leaving (its event over), it cheers
/// again (Thistle Tea's "cheers and schedules the first firework" and
/// "leaves other objects alone"; vmangos's `go_cheer_speaker`).
#[tokio::test]
async fn a_cheer_speaker_cheers_its_show_in_and_out() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (speaker, other, wait) = map
        .query(move |m| {
            let now = m.now;
            let speaker = set_down(m, ga, SPEAKER);
            let other = set_down(m, ga, OTHER);
            let wait = m.speakers[&speaker].next_launch - now;
            (speaker, other, wait)
        })
        .await
        .unwrap();
    assert!(
        (Duration::from_secs(1)..=Duration::from_secs(2)).contains(&wait),
        "{wait:?}"
    );
    let has_other = map
        .query(move |m| m.speakers.contains_key(&other))
        .await
        .unwrap();
    assert!(!has_other, "another object has no show");
    a.sees(speaker).await;
    let sound = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::SMSG_PLAY_OBJECT_SOUND(s) if s.guid == speaker => Some(s.sound_id),
        _ => None,
    };
    let opening = a.wait_for("the opening cheer", sound).await;
    assert!(CHEERS.contains(&opening), "{opening}");
    map.query(move |m| crate::game::map::objects::despawn(m, speaker))
        .await
        .unwrap();
    let closing = a.wait_for("the closing cheer", sound).await;
    assert!(CHEERS.contains(&closing), "{closing}");
    let left = map.query(|m| m.speakers.len()).await.unwrap();
    assert_eq!(left, 0);
}

/// A speaker in Stormwind sends up one of the city's fireworks, for half a
/// second, from one of Stormwind's launch sites, the next due one to two
/// seconds on; one far from every city sends nothing up (Thistle Tea's
/// "sends up a city firework and schedules the next").
#[tokio::test]
async fn a_cheer_speaker_sends_a_city_firework_up_and_the_next_soon() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let seen = map
        .query(move |m| {
            let now = m.now;
            let far = set_down(m, ga, SPEAKER);
            m.speakers.get_mut(&far).unwrap().next_launch = now;
            cheer::update(m);
            let none = m
                .gameobjects
                .iter()
                .filter(|(_, o)| FIREWORKS.contains(&o.entry))
                .count();
            let trade_district = Vector3d {
                x: -8862.0,
                y: 654.0,
                z: 96.0,
            };
            let speaker =
                objects::summon(m, None, SPEAKER, trade_district, 0.0, 0, 0, 0, None).unwrap();
            m.speakers.get_mut(&speaker).unwrap().next_launch = now;
            cheer::update(m);
            let sites = cheer::sites(0, trade_district);
            let sent: Vec<(u32, Vector3d, Option<std::time::Instant>)> = m
                .gameobjects
                .iter()
                .filter(|(_, o)| FIREWORKS.contains(&o.entry))
                .map(|(_, o)| {
                    let s = m.objects.summoned.iter().find(|s| s.guid == o.guid);
                    (o.entry, o.position, s.and_then(|s| s.expires))
                })
                .collect();
            let next = m.speakers[&speaker].next_launch - now;
            let at_a_site = sent.iter().all(|(_, p, _)| {
                sites.iter().any(|s| {
                    dist(
                        *p,
                        Vector3d {
                            x: s[0],
                            y: s[1],
                            z: s[2],
                        },
                    ) < 0.01
                })
            });
            (
                none,
                sites.len(),
                sent.len(),
                at_a_site,
                sent.iter().all(|s| {
                    s.2 == Some(now + Duration::from_millis(cheer::FIREWORK_LIFETIME_MS.into()))
                }),
                (Duration::from_secs(1)..=Duration::from_secs(2)).contains(&next),
            )
        })
        .await
        .unwrap();
    assert_eq!(seen.0, 0, "none from far from a city");
    assert_eq!(seen.1, 117, "Stormwind's sites");
    assert_eq!(seen.2, 1, "one firework");
    assert!(seen.3, "at one of them");
    assert!(seen.4, "for half a second");
    assert!(seen.5, "the next one to two seconds on");
}
