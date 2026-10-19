//! Round 7's world tests, ported from Thistle Tea's
//! `test/game/world/system/` (docs/parity/matrix.csv): a battleground's
//! pickups and its end.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameObjectTemplate;
use crate::game::harness::{self, Client};
use crate::game::map::battleground::{self as bg, BUFF_ENTRIES, BUFF_RESPAWN, Due};
use crate::game::map::{Map as WorldMap, MapHandle};
use crate::game::tests_p3_world_bg::player;
use crate::game::tests_pvp::{arathi, both_into};

/// What `map`'s query of `f` makes of its match, waiting until `f` says
/// something.
async fn until<T: Send + 'static>(
    map: &MapHandle,
    what: &str,
    f: impl Fn(&WorldMap) -> Option<T> + Send + Sync + Clone + 'static,
) -> T {
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        let f = f.clone();
        if let Some(t) = map.query(move |m| f(m)).await.flatten() {
            return t;
        }
        assert!(tokio::time::Instant::now() < deadline, "never {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The pickup at place `slot`: its object and entry.
fn pickup(m: &WorldMap, slot: usize) -> Option<(Guid, u32)> {
    let g = m.battleground.as_ref()?.buffs.get(slot)?.object?;
    m.gameobject(g).map(|o| (g, o.entry))
}

/// `c` at `at` in the match, at once.
async fn put(map: &MapHandle, c: &mut Client, at: Vector3d) {
    let g = c.guid;
    map.query(move |m| m.teleport(g, m.key, at, 0.0)).await;
    c.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
}

/// Thistle Tea's `Battleground.Buffs` "rotates consumed pickups after the
/// delay and cleans up when the match ends", and its effect sink
/// "publishes the victory exit countdown immediately": each of Arathi
/// Basin's five places has one of its three pickups as the battle begins;
/// Alice on one takes it, and it is back, one of the three, when its
/// three minutes are up; the match won, the pickups go, and each inside
/// is told at once the two minutes left before they are sent home.
#[tokio::test]
async fn a_pickup_taken_comes_back_in_three_minutes_and_all_go_at_the_end() {
    let mut d = arathi();
    for entry in BUFF_ENTRIES {
        d.gameobject_templates.insert(
            entry,
            GameObjectTemplate {
                entry,
                kind: 22,
                display_id: 5_931,
                name: format!("Pickup {entry}"),
                size: 1.0,
                ..Default::default()
            },
        );
    }
    let realm = harness::realm_with(d);
    let mut a = player(&realm, "A", "Alice", Race::Human).await;
    let mut b = player(&realm, "B", "Grom", Race::Orc).await;
    let map = both_into(&realm, &mut a, &mut b, Map::ArathiBasin).await;
    let all = until(&map, "the pickups out", |m| {
        let all: Vec<(Guid, u32)> = (0..5).filter_map(|i| pickup(m, i)).collect();
        (all.len() == 5).then_some(all)
    })
    .await;
    assert!(all.iter().all(|(_, e)| BUFF_ENTRIES.contains(e)), "{all:?}");
    let first = all[0].0;
    let at = map
        .query(move |m| m.gameobject(first).map(|o| o.position))
        .await
        .flatten()
        .unwrap();
    put(&map, &mut a, at).await;
    let back_at = until(&map, "the pickup taken", move |m| {
        let b = m.battleground.as_ref()?;
        (b.buffs[0].object.is_none() && m.gameobject(first).is_none())
            .then(|| {
                b.timers
                    .iter()
                    .find(|(_, d)| matches!(d, Due::Buff(0)))
                    .map(|(t, _)| *t)
            })
            .flatten()
    })
    .await;
    let wait = back_at.saturating_duration_since(Instant::now());
    assert!(
        wait > BUFF_RESPAWN - Duration::from_secs(10) && wait <= BUFF_RESPAWN,
        "back in three minutes: {wait:?}"
    );
    // Alice away from the place; its three minutes up.
    put(
        &map,
        &mut a,
        Vector3d {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    )
    .await;
    map.query(|m| {
        if let Some(b) = m.battleground.as_mut() {
            for t in b.timers.iter_mut().filter(|t| matches!(t.1, Due::Buff(_))) {
                t.0 = Instant::now();
            }
        }
    })
    .await;
    let (again, entry) = until(&map, "the pickup back", |m| pickup(m, 0)).await;
    assert_ne!(again, first, "a new one");
    assert!(BUFF_ENTRIES.contains(&entry), "one of the three: {entry}");
    let mut out: Vec<Guid> = all.iter().map(|(g, _)| *g).collect();
    out[0] = again;
    a.clear();
    let gone = map
        .query(move |m| {
            bg::finish(m, 0);
            out.iter().all(|g| m.gameobject(*g).is_none())
                && m.battleground.as_ref().is_some_and(|b| b.buffs.is_empty())
        })
        .await
        .unwrap();
    assert!(gone, "the pickups go with the match");
    let status = a
        .wait_frame(
            "the countdown home",
            crate::game::battlegrounds::SMSG_BATTLEFIELD_STATUS,
            |f| f.len() >= 25 && f[13] == 3,
        )
        .await;
    let leave = u32::from_le_bytes(status[17..21].try_into().unwrap());
    let auto = crate::game::battlegrounds::AUTO_LEAVE.as_millis() as u32;
    assert!(
        leave > auto - 5_000 && leave <= auto,
        "{leave} ms of {auto} left"
    );
}

/// Each side's captain's rally in an Alterac match: when it is next due,
/// from now.
fn rallies(m: &WorldMap) -> Option<Vec<Duration>> {
    let b = m.battleground.as_ref()?;
    let now = Instant::now();
    let mut due: Vec<(usize, Duration)> = b
        .timers
        .iter()
        .filter_map(|(t, d)| match d {
            Due::CaptainBuff(s) => Some((*s, t.saturating_duration_since(now))),
            _ => None,
        })
        .collect();
    due.sort_by_key(|(s, _)| *s);
    (due.len() == 2).then(|| due.into_iter().map(|(_, d)| d).collect())
}

/// Thistle Tea's battleground effect sink "schedules random-delay timers
/// on the explicitly supplied match owner": as Alterac's battle begins,
/// each side's captain rallies them in two to six minutes, on the match's
/// own clock; rallied, again in two to six.
#[tokio::test]
async fn alteracs_captains_rally_their_sides_every_two_to_six_minutes() {
    let realm = harness::realm_with(crate::game::tests_pvp::alterac());
    let mut a = player(&realm, "A", "Alice", Race::Human).await;
    let mut b = player(&realm, "B", "Grom", Race::Orc).await;
    let map = both_into(&realm, &mut a, &mut b, Map::AlteracValley).await;
    let within = |d: &Duration| *d > Duration::from_secs(110) && *d <= Duration::from_secs(360);
    let first = until(&map, "the rallies due", rallies).await;
    assert!(first.iter().all(within), "{first:?}");
    map.query(|m| {
        if let Some(b) = m.battleground.as_mut() {
            for t in b
                .timers
                .iter_mut()
                .filter(|t| matches!(t.1, Due::CaptainBuff(_)))
            {
                t.0 = Instant::now();
            }
        }
    })
    .await;
    let again = until(&map, "the rallies due again", |m| {
        rallies(m).filter(|d| d.iter().all(|d| *d > Duration::from_secs(60)))
    })
    .await;
    assert!(again.iter().all(within), "{again:?}");
}
