//! Round 7's creature script tests, continued: The Nightmare Manifests'
//! creatures (Eranikus, Keeper Remulos, Tyrande), each part of the
//! scene driven on the real world as a player's client sees it.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use super::tests_p6_scripts::*;
use super::tests_p7_scripts::*;
use super::tests_p7_scripts_quests::until_some;
use crate::game::harness::Client;
use crate::game::map::combat::dist;
use crate::game::map::{Map, MapHandle, event_ai, map_events, scripts};

const REMULOS: u32 = 11832;
const ERANIKUS: u32 = 15491;
const TYRANDE: u32 = 15633;
const PRIESTESS: u32 = 15634;
const PHANTASM: u32 = 15629;
const NIGHTMARE: u32 = 8736;
const HOVER: u32 = 17131;
const NOT_ATTACKABLE: u32 = 0x2;

fn at(x: f32, y: f32, z: f32) -> Vector3d {
    Vector3d { x, y, z }
}

/// Where Eranikus flies up to, and where he lands.
const PERCH: Vector3d = Vector3d {
    x: 7929.86,
    y: -2574.88,
    z: 505.35,
};
const LANDING: Vector3d = Vector3d {
    x: 7912.98,
    y: -2568.99,
    z: 488.71,
};

/// Whether `c` hears `speaker` say text `id` soon, in either of its
/// lines (a woman says its female line).
async fn hears_her(c: &mut Client, realm: &crate::game::Realm, speaker: Guid, id: u32) -> bool {
    let npc = &realm.data.npc;
    let heads: Vec<String> = [false, true]
        .into_iter()
        .filter_map(|f| npc.broadcast_line(id, f))
        .filter(|l| !l.is_empty())
        .map(|l| l.split('$').next().unwrap_or_default().to_owned())
        .collect();
    assert!(!heads.is_empty(), "text {id}");
    c.gets(Duration::from_secs(3), move |m| match m {
        wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_MESSAGECHAT(s) => {
            match &s.chat_type {
                SMSG_MESSAGECHAT_ChatType::MonsterSay { sender1, .. }
                | SMSG_MESSAGECHAT_ChatType::MonsterYell { sender1, .. }
                    if *sender1 == speaker && heads.iter().any(|h| s.message.starts_with(h)) =>
                {
                    Some(())
                }
                _ => None,
            }
        }
        _ => None,
    })
    .await
    .is_some()
}

/// `g`'s timed events looked at now.
fn tick_now(m: &mut Map, g: Guid) {
    if let Some(s) = event_ai::scripted_mut(m, g) {
        s.next_tick = None;
    }
    event_ai::tick(m, g);
}

/// Wait for `g` to come within two yards of `to`, its walks hurried.
async fn comes_to(map: &MapHandle, g: Guid, to: Vector3d) -> bool {
    until(map, 5_000, move |m| {
        hasten(m, g);
        m.position_of(g).is_some_and(|p| dist(p, to) < 2.0)
    })
    .await
}

/// The Nightmare Manifests' event under way for the player, Remulos by the
/// shrine's clearing and Eranikus summoned above it, hovering: the client,
/// the player, the map, Remulos and Eranikus.
async fn the_nightmare(
    realm: &std::sync::Arc<crate::game::Realm>,
    name: &str,
) -> (Client, Guid, MapHandle, Guid, Guid) {
    let (mut c, me, map, remulos) = hero_by(realm, name, REMULOS).await;
    let eranikus = map
        .query(move |m| {
            logs(m, me, NIGHTMARE);
            // Down from his shrine, as the escort brings him (his home
            // with him, or he would leash home from the fight).
            scripts::place(m, remulos, at(7900.0, -2566.0, 488.0), 0.0);
            // and of the faction his escort gives him.
            if let Some(c) = m.creature_mut(remulos) {
                c.home = c.position;
                c.fields
                    .set_u32(crate::game::fields::UNIT_FIELD_FACTIONTEMPLATE, 1254);
            }
            tough(m, remulos);
            if let Some(s) = event_ai::scripted_mut(m, remulos) {
                s.invincible_hp = Some(1);
            }
            stand(m, me, at(7904.0, -2560.0, 488.0));
            map_events::start(m, NIGHTMARE, remulos, me, Duration::from_secs(3600));
            let e = summon(m, ERANIKUS, off(PERCH, 6.0, 0.0), 0.0);
            event_ai::update_despawns(m);
            e
        })
        .await
        .unwrap();
    c.sees(eranikus).await;
    (c, me, map, remulos, eranikus)
}

/// [`the_nightmare`], Eranikus down from his hover and at Remulos.
async fn landed(
    realm: &std::sync::Arc<crate::game::Realm>,
    name: &str,
) -> (Client, Guid, MapHandle, Guid, Guid) {
    let (c, me, map, remulos, eranikus) = the_nightmare(realm, name).await;
    map.query(move |m| event_ai::script_event(m, eranikus, 2, 0, None))
        .await;
    assert!(comes_to(&map, eranikus, LANDING).await, "he lands");
    sleep_ms(200).await;
    hurry_all(&map, 1.0).await;
    assert!(
        until(&map, 3_000, move |m| victim(m, eranikus) == remulos).await,
        "at Remulos"
    );
    (c, me, map, remulos, eranikus)
}

/// Eranikus comes hovering out of reach (his hover's aura, flying, not to
/// be attacked, and not to be brought below a fifth), one of the quest's
/// event's. At Remulos's first signal he flies up to his perch and turns
/// to Remulos; at the second he gives up his hover and comes down to land,
/// says his piece, and a moment on, open to attack, falls on Remulos.
/// Should he give up the fight the quest's event fails, and he, Tyrande
/// and her priestesses and the phantasms go with it (Thistle Tea's
/// `eranikus`).
#[tokio::test]
#[ignore = "extended"]
async fn eranikus_hovers_answers_remulos_and_lands_to_fight() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, _me, map, remulos, eranikus) = the_nightmare(&realm, "Dreamer").await;
    let hovering = map
        .query(move |m| {
            let s = event_ai::scripted(m, eranikus).unwrap();
            (
                has(m, eranikus, HOVER),
                s.fly,
                unit_flags(m, eranikus) & NOT_ATTACKABLE != 0,
                s.invincible_hp.is_some(),
                map_events::targets(m, NIGHTMARE).contains(&eranikus),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        hovering,
        (true, true, true, true, true),
        "hovering out of reach, of the event"
    );
    // The first signal.
    map.query(move |m| event_ai::script_event(m, eranikus, 1, 0, None))
        .await;
    assert!(comes_to(&map, eranikus, PERCH).await, "up to his perch");
    assert!(
        until(&map, 2_000, move |m| {
            let (Some(e), Some(r)) = (m.creature(eranikus), m.position_of(remulos)) else {
                return false;
            };
            let to = (r.y - e.position.y).atan2(r.x - e.position.x);
            let off = (e.orientation - to).rem_euclid(std::f32::consts::TAU);
            off.min(std::f32::consts::TAU - off) < 0.2
        })
        .await,
        "facing Remulos"
    );
    // The second.
    c.clear();
    map.query(move |m| event_ai::script_event(m, eranikus, 2, 0, None))
        .await;
    assert!(
        !map.query(move |m| has(m, eranikus, HOVER)).await.unwrap(),
        "his hover given up"
    );
    assert!(comes_to(&map, eranikus, LANDING).await, "down to land");
    assert!(hears(&mut c, &realm, eranikus, 11305).await, "his piece");
    assert!(
        !map.query(move |m| event_ai::scripted(m, eranikus).is_some_and(|s| s.fly))
            .await
            .unwrap(),
        "on the ground"
    );
    hurry_all(&map, 1.0).await;
    assert!(
        until(&map, 3_000, move |m| unit_flags(m, eranikus)
            & NOT_ATTACKABLE
            == 0
            && victim(m, eranikus) == remulos)
        .await,
        "open to attack, at Remulos"
    );
    // He gives up the fight, with Tyrande's company and a phantasm about.
    let company = map
        .query(move |m| {
            let here = m.position_of(eranikus).unwrap();
            let out: Vec<Guid> = [TYRANDE, PRIESTESS, PHANTASM]
                .into_iter()
                .map(|e| summon(m, e, off(here, 8.0, 0.0), 0.0))
                .collect();
            crate::game::map::ai::enter_evade(m, eranikus);
            out
        })
        .await
        .unwrap();
    let company2 = company.clone();
    assert!(
        until(&map, 3_000, move |m| !map_events::active(m, NIGHTMARE)
            && gone(m, eranikus)
            && company2.iter().all(|g| gone(m, *g)))
        .await,
        "the event failed, and he and the rest gone with it"
    );
}

/// In the fight (his first phase) Eranikus breathes and casts his volleys
/// on their timers; at 85 percent he calls Tyrande, who rides in with
/// seven priestesses for the escort, rides to the shrine, gets down and
/// channels her absolution. Remulos heals and casts Starfire only in the
/// escort's phase. At a fifth Eranikus is redeemed: his phase changes, the
/// fight stops, the priestesses go, and his breath is no more; his
/// farewell, at the end of his walk, has Remulos credit the quest and end
/// its event well.
#[tokio::test]
#[ignore = "extended"]
async fn eranikus_is_redeemed_and_remulos_credits_the_quest() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, remulos, eranikus) = landed(&realm, "Redeemer").await;
    // His kit, in the fight.
    for (id, spell) in [(1549108, 24818), (1549109, 25586), (1549110, 24839)] {
        c.clear();
        map.query(move |m| {
            if let Some(cr) = m.caster_mut(eranikus) {
                cr.gcd_until = None;
            }
            due(m, eranikus, id);
        })
        .await;
        assert!(
            cast_any_of(&mut c, eranikus, spell, Duration::from_secs(3))
                .await
                .is_some(),
            "{spell} in the fight"
        );
        map.query(move |m| crate::game::map::spells::interrupt(m, eranikus))
            .await;
    }
    // Remulos's, only in the escort's phase.
    c.clear();
    map.query(move |m| {
        crate::game::map::ai::attacked(m, remulos, eranikus);
        event_ai::set_phase(m, remulos, 0);
        due(m, remulos, 1183202);
    })
    .await;
    assert!(
        cast_any_of(&mut c, remulos, 21668, Duration::from_millis(800))
            .await
            .is_none(),
        "no Starfire out of the escort's phase"
    );
    let dbg = map
        .query(move |m| {
            event_ai::set_phase(m, remulos, 1);
            if let Some(cr) = m.caster_mut(remulos) {
                cr.gcd_until = None;
            }
            due(m, remulos, 1183202);
            let c = m.creature(remulos).unwrap();
            (
                c.combat.attacking,
                c.threat.clone(),
                c.caster.casting.as_ref().map(|x| x.spell),
                left(m, remulos, 1183202),
                m.creature(eranikus).map(|e| e.combat.victim),
            )
        })
        .await;
    assert!(
        cast_any_of(&mut c, remulos, 21668, Duration::from_secs(3))
            .await
            .is_some(),
        "Starfire in it: {dbg:?}"
    );
    // 85 percent: Tyrande.
    c.clear();
    map.query(move |m| {
        set_pct(m, eranikus, 85);
        tick_now(m, eranikus);
    })
    .await;
    assert!(hears(&mut c, &realm, eranikus, 11306).await, "he calls her");
    let tyrande = until_some(&map, 2_000, |m| first(m, TYRANDE))
        .await
        .expect("Tyrande comes");
    hurry_all(&map, 3.0).await;
    assert!(
        until(&map, 2_000, |m| {
            let p = living(m, PRIESTESS);
            p.len() == 7 && p.iter().all(|g| faction(m, *g) == 495)
        })
        .await,
        "seven priestesses for the escort"
    );
    // Her ride, followed.
    let shrine = at(7888.32, -2566.25, 487.02);
    let kneel = at(7901.83, -2565.24, 488.04);
    // A watcher rides beside her, so that she goes on, and the player
    // waits where she is bound.
    let (mut w, watcher, _) = hero(&realm, "Watcher").await;
    let from = map
        .query(move |m| m.position_of(tyrande))
        .await
        .flatten()
        .unwrap();
    travel(&realm, &mut w, 1, off(from, 2.0, 0.0)).await;
    let followed = map.clone();
    let follow = move |to: Vector3d| {
        let map = followed.clone();
        async move {
            until(&map, 8_000, move |m| {
                if let Some(at) = m.position_of(tyrande) {
                    stand(m, watcher, off(at, 2.0, 0.0));
                }
                stand(m, me, off(to, 3.0, 0.0));
                hasten(m, tyrande);
                m.position_of(tyrande).is_some_and(|p| dist(p, to) < 2.0)
            })
            .await
        }
    };
    c.clear();
    assert!(follow(shrine).await, "Tyrande rides to the shrine");
    let spoke = hears_her(&mut c, &realm, tyrande, 11317).await;
    assert!(
        spoke,
        "and speaks: {:?}",
        map.query(move |m| m.creature(tyrande).map(|t| (
            t.position,
            t.scripted
                .as_deref()
                .and_then(|s| s.scripted_move)
                .map(|mv| (mv.point, mv.to)),
            format!("{:?}", t.motion)
                .chars()
                .take(200)
                .collect::<String>(),
            has(m, tyrande, 16056),
            m.scripts
                .runs
                .iter()
                .filter(|r| r.source == tyrande)
                .count(),
        )))
        .await
    );
    assert!(
        until(&map, 2_000, move |m| !has(m, tyrande, 16056)).await,
        "down from her mount"
    );
    hurry_all(&map, 5.0).await;
    c.clear();
    assert!(follow(kneel).await, "to Eranikus's side");
    assert!(
        cast_any_of(&mut c, tyrande, 23017, Duration::from_secs(3))
            .await
            .is_some(),
        "her absolution"
    );
    // A fifth: redeemed.
    c.clear();
    map.query(move |m| {
        stand(m, me, at(7904.0, -2560.0, 488.0));
        set_pct(m, eranikus, 20);
        tick_now(m, eranikus);
    })
    .await;
    assert!(hears(&mut c, &realm, eranikus, 11316).await, "redeemed");
    let after = map
        .query(move |m| {
            (
                event_ai::phase(m, eranikus),
                faction(m, eranikus),
                fighting(m, eranikus),
                living(m, PRIESTESS).len(),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        after,
        (1, 35, false, 0),
        "his phase, peace, the escort gone"
    );
    c.clear();
    map.query(move |m| {
        if let Some(cr) = m.caster_mut(eranikus) {
            cr.gcd_until = None;
        }
        due(m, eranikus, 1549108);
    })
    .await;
    assert!(
        cast_any_of(&mut c, eranikus, 24818, Duration::from_millis(600))
            .await
            .is_none(),
        "his breath no more"
    );
    // His farewell, and Remulos's credit.
    hurry_all(&map, 21.0).await;
    let farewell = at(7906.57, -2565.63, 488.39);
    c.clear();
    assert!(comes_to(&map, eranikus, farewell).await, "his last walk");
    assert!(hears(&mut c, &realm, eranikus, 11323).await, "his farewell");
    hurry_all(&map, 42.0).await;
    assert!(
        until(&map, 2_000, move |m| explored(m, me, NIGHTMARE)).await,
        "Remulos credits the quest"
    );
    hurry_all(&map, 6.0).await;
    assert!(
        until(&map, 2_000, move |m| !map_events::active(m, NIGHTMARE)).await,
        "and ends its event"
    );
    assert!(
        !map.query(move |m| failed(m, me, NIGHTMARE)).await.unwrap(),
        "well"
    );
}
