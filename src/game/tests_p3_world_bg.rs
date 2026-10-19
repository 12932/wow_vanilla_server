//! Phase 3's battlegrounds against Thistle Tea's tests, through the
//! protocol: Warsong Gulch's countdown behind its gates and its start, a
//! flag returned by its own side, captures and their rewards, the kills on
//! the scoreboard, a declined invitation, the battlegrounds' portals, and
//! Arathi Basin's and the control points' rules.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_pvp::{battleground, bound_object, into, marker, queue};
use crate::game::Realm;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;
use crate::game::map::battleground::{self as bg, Flag, Phase, wsg};
use crate::game::map::death::{self, Death};

const WSG: u32 = 489;
const DOOR: u32 = 95_000;
const GHOST_GATE: u32 = 95_001;

fn text(d: &mut GameData, id: u32, line: &str) {
    d.npc.broadcast_texts.insert(
        id,
        crate::game::data::npc::BroadcastText {
            male: line.into(),
            ..Default::default()
        },
    );
}

/// Warsong Gulch with its capture triggers, its texts, a door of the
/// doors' event and a ghost gate of the ghost gates'.
pub(super) fn gulch() -> GameData {
    let mut d = GameData::empty();
    battleground(&mut d, WSG, 2, "PVPZone03", [769, 770]);
    for (s, id) in wsg::CAPTURE_TRIGGERS.into_iter().enumerate() {
        let (x, y, z, _) = wsg::FLAG_AT[s];
        d.area_triggers.insert(
            id,
            AreaTrigger {
                id,
                map: WSG,
                x,
                y,
                z,
                radius: 5.0,
                ..Default::default()
            },
        );
    }
    for (id, line) in [
        (9804, "The Alliance Flag was picked up by $n!"),
        (9807, "The Horde Flag was picked up by $n!"),
        (9805, "The Alliance Flag was dropped by $n!"),
        (9806, "The Horde Flag was dropped by $n!"),
        (9808, "The Alliance Flag was returned to its base by $n!"),
        (10022, "The Alliance Flag is now placed at its base."),
        (10023, "The Horde Flag has returned to its base!"),
        (9801, "$n captured the Horde Flag!"),
        (9803, "The flags are now placed at their bases."),
        (10014, "The battle for Warsong Gulch has begun!"),
        (10015, "The battle for Warsong Gulch begins in 1 minute."),
        (
            10016,
            "The battle for Warsong Gulch begins in 30 seconds. Prepare yourselves!",
        ),
    ] {
        text(&mut d, id, line);
    }
    marker(&mut d, wsg::SPELL_FLAG_CARRIED[0]);
    marker(&mut d, wsg::SPELL_FLAG_CARRIED[1]);
    for (entry, name) in [(DOOR, "Gate"), (GHOST_GATE, "Ghost Gate")] {
        d.gameobject_templates.insert(
            entry,
            GameObjectTemplate {
                entry,
                kind: 0,
                display_id: 411,
                size: 1.0,
                name: name.into(),
                ..Default::default()
            },
        );
    }
    bound_object(
        &mut d,
        WSG,
        95_100,
        DOOR,
        (1500.0, 1480.0),
        (bg::EVENT_DOORS, 0),
    );
    bound_object(
        &mut d,
        WSG,
        95_101,
        GHOST_GATE,
        (1510.0, 1480.0),
        (bg::EVENT_GHOST_GATES, 0),
    );
    d
}

/// A realm whose battles start `delay` after the first arrives.
fn delayed(d: GameData, delay: Duration) -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.bg_start_delay = delay;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, d, None)
}

/// Move `c` within the match's map, at once, to a height.
pub(super) async fn put(map: &MapHandle, c: &mut Client, x: f32, y: f32, z: f32) {
    let g = c.guid;
    map.query(move |m| m.teleport(g, m.key, Vector3d { x, y, z }, 0.0))
        .await;
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

async fn said(c: &mut Client, containing: &str) -> String {
    c.wait_for(containing, |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(chat) if chat.message.contains(containing) => {
            Some(chat.message.clone())
        }
        _ => None,
    })
    .await
}

async fn state(c: &mut Client, field: u32, value: u32) {
    c.wait_for("the world state", |m| match m {
        ServerOpcodeMessage::SMSG_UPDATE_WORLD_STATE(w)
            if w.state.state == field && w.state.value == value =>
        {
            Some(())
        }
        _ => None,
    })
    .await
}

fn flags(m: &crate::game::map::Map) -> [Option<Flag>; 2] {
    m.battleground.as_ref().map_or([None, None], |b| b.flags)
}

/// The match's timers, due now.
async fn hurry(map: &MapHandle) {
    map.query(|m| {
        if let Some(b) = m.battleground.as_mut() {
            for t in b.timers.iter_mut() {
                t.0 = Instant::now();
            }
        }
    })
    .await;
    tokio::time::sleep(Duration::from_millis(120)).await;
}

/// Until `f` holds on the map, or panic.
async fn until(
    map: &MapHandle,
    what: &str,
    f: impl Fn(&crate::game::map::Map) -> bool + Send + Clone + 'static,
) {
    for _ in 0..300 {
        let f = f.clone();
        if map.query(move |m| f(m)).await.unwrap_or(false) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never: {what}");
}

pub(super) async fn player(realm: &Arc<Realm>, account: &str, name: &str, race: Race) -> Client {
    let mut c = Client::connect(realm, account);
    c.enter(name, race, Class::Warrior).await;
    c
}

/// Each of `clients` queues and goes in, and the battle is on.
pub(super) async fn all_in(realm: &Arc<Realm>, clients: &mut [&mut Client]) -> MapHandle {
    for c in clients.iter() {
        queue(c, Map::WarsongGulch).await;
    }
    let mut handle = None;
    for c in clients.iter_mut() {
        handle = Some(into(realm, c, Map::WarsongGulch).await);
    }
    let handle = handle.expect("someone went in");
    until(&handle, "the battle on", |m| {
        m.battleground.as_ref().is_some_and(|b| b.playing())
    })
    .await;
    handle
}

/// The stand of side `s`'s flag, once it's up.
async fn stand(map: &MapHandle, s: usize) -> Guid {
    until(map, "the flag up", move |m| {
        matches!(flags(m)[s], Some(Flag::AtBase(_)))
    })
    .await;
    match map.query(move |m| flags(m)[s]).await.flatten() {
        Some(Flag::AtBase(go)) => go,
        f => panic!("{f:?}"),
    }
}

/// `c` takes side `s`'s flag from its stand.
async fn take(map: &MapHandle, c: &mut Client, s: usize) {
    let go = stand(map, s).await;
    let (x, y, z, _) = wsg::FLAG_AT[s];
    put(map, c, x, y, z).await;
    c.send(CMSG_GAMEOBJ_USE { guid: go }).await;
    let g = c.guid;
    until(map, "the flag carried", move |m| {
        flags(m)[s] == Some(Flag::Carried(g))
    })
    .await;
}

pub(super) fn score(m: &crate::game::map::Map, g: Guid) -> bg::Score {
    m.battleground.as_ref().unwrap().scores[&g]
}

#[tokio::test]
async fn warsong_gulch_counts_down_behind_shut_gates_then_opens_them_and_says_where_it_stands() {
    let realm = delayed(gulch(), Duration::from_secs(120));
    let mut a = player(&realm, "A", "Alice", Race::Human).await;
    let mut b = player(&realm, "B", "Grom", Race::Orc).await;
    queue(&a, Map::WarsongGulch).await;
    queue(&b, Map::WarsongGulch).await;
    let map = into(&realm, &mut a, Map::WarsongGulch).await;
    into(&realm, &mut b, Map::WarsongGulch).await;
    // Two minutes to go, behind a shut gate, its ghost gate up.
    let (left, door, ghost) = map
        .query(|m| {
            let at = match m.battleground.as_ref().unwrap().phase {
                Phase::Starting { at } => at,
                p => panic!("{p:?}"),
            };
            let door = m
                .gameobjects
                .iter()
                .map(|(_, o)| o)
                .find(|o| o.entry == DOOR)
                .map(|o| o.fields.get_u32(GAMEOBJECT_STATE));
            let ghost = m
                .gameobjects
                .iter()
                .map(|(_, o)| o)
                .any(|o| o.entry == GHOST_GATE);
            (at.saturating_duration_since(Instant::now()), door, ghost)
        })
        .await
        .unwrap();
    assert!(left > Duration::from_secs(110), "{left:?}");
    assert_eq!(door, Some(1), "the gate shut");
    assert!(ghost, "the ghost gate up");
    // A minute to go, then half of one: each said once.
    let soon = |secs: u64| {
        move |m: &mut crate::game::map::Map| {
            if let Some(b) = m.battleground.as_mut() {
                b.phase = Phase::Starting {
                    at: Instant::now() + Duration::from_secs(secs),
                };
            }
        }
    };
    map.query(soon(59)).await;
    said(&mut a, "begins in 1 minute").await;
    map.query(soon(29)).await;
    said(&mut b, "begins in 30 seconds").await;
    // The start: the gate open, the ghost gate gone, the battle begun and
    // its world states told.
    a.clear();
    map.query(soon(0)).await;
    said(&mut a, "has begun").await;
    state(&mut a, wsg::WS_CAPTURES_MAX, wsg::CAPTURES_TO_WIN).await;
    state(&mut a, wsg::WS_CAPTURES[0], 0).await;
    let (door, ghost) = map
        .query(|m| {
            (
                m.gameobjects
                    .iter()
                    .map(|(_, o)| o)
                    .find(|o| o.entry == DOOR)
                    .map(|o| o.fields.get_u32(GAMEOBJECT_STATE)),
                m.gameobjects
                    .iter()
                    .map(|(_, o)| o)
                    .any(|o| o.entry == GHOST_GATE),
            )
        })
        .await
        .unwrap();
    assert_eq!(door, Some(0), "the gate open");
    assert!(!ghost, "the ghost gate gone");
    // A second in: the battle's status (in progress) and the scoreboard,
    // not ended.
    // In progress, for more than nothing (the battle's time since it
    // began, after the time left to stay).
    let status = a
        .wait_frame(
            "the status in progress",
            crate::game::battlegrounds::SMSG_BATTLEFIELD_STATUS,
            |f| f.len() >= 25 && f[13] == 3,
        )
        .await;
    let since = u32::from_le_bytes([status[21], status[22], status[23], status[24]]);
    assert!(since > 0, "{since}");
    a.wait_for("the scoreboard", |m| match m {
        ServerOpcodeMessage::MSG_PVP_LOG_DATA(l) => matches!(
            l.status,
            MSG_PVP_LOG_DATA_Server_BattlegroundEndStatus::NotEnded
        )
        .then_some(()),
        _ => None,
    })
    .await;
}

#[tokio::test]
async fn a_flag_returned_by_its_own_side_counts_for_them_and_its_timer_goes_stale() {
    let realm = harness::realm_with(gulch());
    let mut a = player(&realm, "A", "Alice", Race::Human).await;
    let mut b = player(&realm, "B", "Grom", Race::Orc).await;
    let map = all_in(&realm, &mut [&mut a, &mut b]).await;
    let (alice, grom) = (a.guid, b.guid);
    // Grom takes the Alliance's flag and falls with it.
    take(&map, &mut b, 0).await;
    map.query(move |m| bg::died(m, grom, alice)).await;
    said(&mut a, "The Alliance Flag was dropped by Grom").await;
    let dropped = match map.query(|m| flags(m)[0]).await.flatten() {
        Some(Flag::Dropped(go)) => go,
        f => panic!("{f:?}"),
    };
    // Alice, by it, returns it: it's home, a return to her name.
    let (x, y, z, _) = wsg::FLAG_AT[0];
    put(&map, &mut a, x, y, z).await;
    a.send(CMSG_GAMEOBJ_USE { guid: dropped }).await;
    said(&mut a, "returned to its base by Alice").await;
    until(&map, "the flag home", |m| {
        matches!(flags(m)[0], Some(Flag::AtBase(_)))
    })
    .await;
    let home = map.query(|m| flags(m)[0]).await.flatten();
    assert_eq!(
        map.query(move |m| score(m, alice).objectives[1]).await,
        Some(1),
        "a return"
    );
    // The ten seconds' timer, run now, finds it home and does nothing.
    a.clear();
    hurry(&map).await;
    assert_eq!(map.query(|m| flags(m)[0]).await.flatten(), home);
    let went_home = a
        .gets(Duration::from_millis(200), |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => {
                c.message.contains("placed at its base").then_some(())
            }
            _ => None,
        })
        .await;
    assert!(went_home.is_none(), "no second return");
}

#[tokio::test]
async fn a_carrier_scores_only_with_their_own_flag_home_and_the_reward_is_for_those_in_the_match() {
    let realm = harness::realm_with(gulch());
    let mut a = player(&realm, "A", "Alice", Race::Human).await;
    let mut c = player(&realm, "C", "Carl", Race::Human).await;
    let mut d = player(&realm, "D", "Dana", Race::Human).await;
    let mut b = player(&realm, "B", "Grom", Race::Orc).await;
    let map = all_in(&realm, &mut [&mut a, &mut c, &mut d, &mut b]).await;
    let (alice, carl, dana, grom) = (a.guid, c.guid, d.guid, b.guid);
    map.query(|m| m.battleground.as_mut().unwrap().bracket = 5)
        .await;
    // Dana goes offline: her place is kept, but she isn't there.
    d.disconnect();
    until(&map, "Dana gone", move |m| !m.players.contains_key(&dana)).await;
    // Alice has the Horde's flag, Grom the Alliance's: she can't score.
    take(&map, &mut a, 1).await;
    take(&map, &mut b, 0).await;
    let (x, y, z, _) = wsg::FLAG_AT[0];
    put(&map, &mut a, x, y, z).await;
    a.send(CMSG_AREATRIGGER {
        trigger_id: wsg::CAPTURE_TRIGGERS[0],
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let (scored, carried) = map
        .query(move |m| (m.battleground.as_ref().unwrap().score, flags(m)[1]))
        .await
        .unwrap();
    assert_eq!(scored, [0, 0], "her own flag away: no capture");
    assert_eq!(carried, Some(Flag::Carried(alice)));
    // Grom's flag goes home (he drops it, Carl returns it): she scores, and
    // the capture's honor (396 at sixty) is Alice's and Carl's, not
    // Dana's, away, nor Grom's.
    map.query(move |m| bg::died(m, grom, carl)).await;
    let dropped = match map.query(|m| flags(m)[0]).await.flatten() {
        Some(Flag::Dropped(go)) => go,
        f => panic!("{f:?}"),
    };
    let at = map
        .query(move |m| m.gameobject(dropped).unwrap().position)
        .await
        .unwrap();
    put(&map, &mut c, at.x, at.y, at.z).await;
    c.send(CMSG_GAMEOBJ_USE { guid: dropped }).await;
    until(&map, "the flag home", |m| {
        matches!(flags(m)[0], Some(Flag::AtBase(_)))
    })
    .await;
    a.send(CMSG_AREATRIGGER {
        trigger_id: wsg::CAPTURE_TRIGGERS[0],
    })
    .await;
    until(&map, "the capture", |m| {
        m.battleground.as_ref().unwrap().score[0] == 1
    })
    .await;
    let honor = map
        .query(move |m| {
            [alice, carl, dana, grom]
                .map(|g| m.battleground.as_ref().unwrap().scores[&g].bonus_honor)
        })
        .await
        .unwrap();
    assert_eq!(honor, [396, 396, 0, 0]);
    // The trigger again at once: no flag, no capture.
    a.send(CMSG_AREATRIGGER {
        trigger_id: wsg::CAPTURE_TRIGGERS[0],
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let scored = map
        .query(|m| m.battleground.as_ref().unwrap().score)
        .await
        .unwrap();
    assert_eq!(scored, [1, 0]);
}

#[tokio::test]
async fn the_scoreboard_counts_an_enemys_killing_blow_and_the_kill_for_their_side_near() {
    let realm = harness::realm_with(gulch());
    let mut a = player(&realm, "A", "Alice", Race::Human).await;
    let mut c = player(&realm, "C", "Carl", Race::Human).await;
    let mut b = player(&realm, "B", "Grom", Race::Orc).await;
    let mut t = player(&realm, "T", "Thrall", Race::Orc).await;
    let mut f = player(&realm, "F", "Fargo", Race::Orc).await;
    let mut s = player(&realm, "S", "Saurfang", Race::Orc).await;
    let map = all_in(
        &realm,
        &mut [&mut a, &mut c, &mut b, &mut t, &mut f, &mut s],
    )
    .await;
    let (alice, carl, grom, thrall, fargo, saurfang) =
        (a.guid, c.guid, b.guid, t.guid, f.guid, s.guid);
    // Everyone together, but Fargo far off, and Saurfang dead near with
    // his corpse there too.
    for cl in [&mut a, &mut c, &mut b, &mut t, &mut s] {
        put(&map, cl, 1200.0, 1400.0, 340.0).await;
    }
    put(&map, &mut f, 1200.0, 1600.0, 340.0).await;
    map.query(move |m| {
        death::kill(m, saurfang);
        death::release(m, saurfang);
    })
    .await;
    // Grom kills Alice: his killing blow, and the honorable kill his, and
    // Thrall's and dead Saurfang's, there; not Fargo's, far off, nor the
    // Alliance's.
    map.query(move |m| bg::died(m, alice, grom)).await;
    // A suicide and a teammate's kill are deaths, and no one's credit.
    map.query(move |m| {
        bg::died(m, carl, carl);
        bg::died(m, thrall, grom);
    })
    .await;
    let scores = map
        .query(move |m| {
            [alice, carl, grom, thrall, fargo, saurfang].map(|g| {
                let s = score(m, g);
                (s.killing_blows, s.honorable_kills, s.deaths)
            })
        })
        .await
        .unwrap();
    assert_eq!(
        scores,
        [
            (0, 0, 1),
            (0, 0, 1),
            (1, 1, 0),
            (0, 1, 1),
            (0, 0, 0),
            (0, 1, 0)
        ],
        "Alice, Carl, Grom, Thrall, Fargo, Saurfang"
    );
    // One not in the match (no score of theirs) falls: nothing counted.
    map.query(move |m| {
        m.battleground.as_mut().unwrap().scores.remove(&carl);
        bg::died(m, carl, grom);
    })
    .await;
    let (carl_scored, grom_kb) = map
        .query(move |m| {
            let b = m.battleground.as_ref().unwrap();
            (b.scores.contains_key(&carl), b.scores[&grom].killing_blows)
        })
        .await
        .unwrap();
    assert!(!carl_scored);
    assert_eq!(grom_kb, 1);
    // Before the gates open, and after the win, kills don't count.
    for phase in [
        Phase::Starting {
            at: Instant::now() + Duration::from_secs(60),
        },
        Phase::Ended {
            winner: 0,
            at: Instant::now(),
        },
    ] {
        let before = map.query(move |m| score(m, grom)).await.unwrap();
        map.query(move |m| {
            let saved = m.battleground.as_ref().unwrap().phase;
            m.battleground.as_mut().unwrap().phase = phase;
            bg::died(m, alice, grom);
            m.battleground.as_mut().unwrap().phase = saved;
        })
        .await;
        let after = map.query(move |m| score(m, grom)).await.unwrap();
        assert_eq!(
            (after.killing_blows, after.honorable_kills),
            (before.killing_blows, before.honorable_kills),
            "{phase:?}"
        );
        let alice_deaths = map.query(move |m| score(m, alice).deaths).await.unwrap();
        assert_eq!(alice_deaths, 1, "{phase:?}");
    }
}

#[tokio::test]
async fn a_priests_spirit_of_redemption_drops_the_flag_and_gives_the_killing_blow_before_it_dies() {
    let realm = harness::realm_with(gulch());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Priest).await;
    let mut b = player(&realm, "B", "Grom", Race::Orc).await;
    let map = all_in(&realm, &mut [&mut a, &mut b]).await;
    let (alice, grom) = (a.guid, b.guid);
    take(&map, &mut a, 1).await;
    map.query(move |m| {
        m.players
            .get_mut(&alice)
            .unwrap()
            .spells
            .push(crate::game::map::spells::redemption::TALENT);
    })
    .await;
    // Grom's blow would kill her: she's a spirit instead, alive, and the
    // flag falls; his killing blow and kill, not her death.
    map.query(move |m| {
        let health = m.players[&alice].fields.get_u32(UNIT_FIELD_HEALTH);
        crate::game::map::combat::deal_damage(m, grom, alice, health);
    })
    .await;
    said(&mut a, "The Horde Flag was dropped by Alice").await;
    let (spirit_alive, flag, alice_score, grom_score) = map
        .query(move |m| {
            (
                m.players[&alice].death == Death::Alive,
                flags(m)[1],
                score(m, alice),
                score(m, grom),
            )
        })
        .await
        .unwrap();
    assert!(spirit_alive, "a spirit, not dead");
    assert!(matches!(flag, Some(Flag::Dropped(_))), "{flag:?}");
    assert_eq!(alice_score.deaths, 0);
    assert_eq!(
        (grom_score.killing_blows, grom_score.honorable_kills),
        (1, 1)
    );
    // The spirit's end is her death, by her own hand: a death, no second
    // blow.
    map.query(move |m| crate::game::map::combat::kill(m, alice, alice))
        .await;
    let (alice_score, grom_score) = map
        .query(move |m| (score(m, alice), score(m, grom)))
        .await
        .unwrap();
    assert_eq!(alice_score.deaths, 1);
    assert_eq!(
        (grom_score.killing_blows, grom_score.honorable_kills),
        (1, 1)
    );
}

#[tokio::test]
async fn an_invitation_declined_leaves_the_match_without_a_word_to_those_in_it() {
    let realm = harness::realm_with(gulch());
    let mut a = player(&realm, "A", "Alice", Race::Human).await;
    let mut b = player(&realm, "B", "Grom", Race::Orc).await;
    let map = all_in(&realm, &mut [&mut a, &mut b]).await;
    // Carl is invited into the match, and declines.
    let mut c = player(&realm, "C", "Carl", Race::Human).await;
    let carl = c.guid;
    queue(&c, Map::WarsongGulch).await;
    c.wait_frame(
        "the invitation",
        crate::game::battlegrounds::SMSG_BATTLEFIELD_STATUS,
        |f| f.len() >= 17 && f[13] == 2,
    )
    .await;
    a.clear();
    c.send(CMSG_BATTLEFIELD_PORT {
        map: Map::WarsongGulch,
        action: BattlefieldPortAction::LeaveQueue,
    })
    .await;
    c.wait_frame(
        "no status",
        crate::game::battlegrounds::SMSG_BATTLEFIELD_STATUS,
        |f| f.len() == 8,
    )
    .await;
    let in_match = realm
        .social
        .battlegrounds
        .lock()
        .matches
        .values()
        .any(|m| m.invited.contains_key(&carl) || m.members.contains_key(&carl));
    assert!(!in_match, "his place gone");
    let left = a
        .gets(Duration::from_millis(300), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_BATTLEGROUND_PLAYER_LEFT(_)).then_some(())
        })
        .await;
    assert!(left.is_none(), "no one left");
    assert!(
        map.query(move |m| !m.battleground.as_ref().unwrap().scores.contains_key(&carl))
            .await
            .unwrap()
    );
}

// Arathi Basin.

/// Arathi Basin's bases, its factions and its rallying spells.
fn basin() -> GameData {
    let mut d = super::tests_pvp::arathi();
    super::tests_pvp2::faction(&mut d, 509, 60);
    super::tests_pvp2::faction(&mut d, 510, 61);
    marker(&mut d, 24061);
    marker(&mut d, 24064);
    d
}

fn ab_of(m: &mut crate::game::map::Map) -> &mut crate::game::map::battleground::ab::Ab {
    m.battleground.as_mut().unwrap().ab.as_mut().unwrap()
}

fn node_state(m: &crate::game::map::Map, node: usize) -> u8 {
    m.battleground
        .as_ref()
        .unwrap()
        .ab
        .as_ref()
        .unwrap()
        .state(node)
}

/// The match's timers of one kind, due now.
async fn fire(map: &MapHandle, which: fn(&bg::Due) -> bool) {
    map.query(move |m| {
        let now = Instant::now();
        if let Some(b) = m.battleground.as_mut() {
            for t in b.timers.iter_mut() {
                if which(&t.1) {
                    t.0 = now;
                }
            }
            // An assault's minute, up.
            if which(&bg::Due::Capture {
                node: 0,
                revision: 0,
            }) {
                let points = b.ab.iter_mut().flat_map(|ab| ab.nodes.iter_mut()).chain(
                    b.av.iter_mut()
                        .flat_map(|av| av.nodes.iter_mut().map(|n| &mut n.point)),
                );
                for p in points {
                    if p.capture_at.is_some() {
                        p.capture_at = Some(now);
                    }
                }
            }
        }
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
}

fn banner_due(d: &bg::Due) -> bool {
    matches!(d, bg::Due::Banner { .. })
}

fn capture_due(d: &bg::Due) -> bool {
    matches!(d, bg::Due::Capture { .. })
}

/// Alice and Grom in Arathi Basin, at sixty's bracket.
async fn in_the_basin() -> (Arc<Realm>, Client, Client, MapHandle) {
    let realm = harness::realm_with(basin());
    let mut a = player(&realm, "A", "Alice", Race::Human).await;
    let mut b = player(&realm, "B", "Grom", Race::Orc).await;
    let map = super::tests_pvp::both_into(&realm, &mut a, &mut b, Map::ArathiBasin).await;
    map.query(|m| m.battleground.as_mut().unwrap().bracket = 5)
        .await;
    (realm, a, b, map)
}

/// `c` uses the banner of `spawn` at base `node`.
async fn use_banner(map: &MapHandle, c: &mut Client, node: u32, spawn: u32) -> Guid {
    let banner = super::tests_pvp::object_of(map, spawn).await;
    super::tests_pvp::put(map, c, 50.0 * node as f32, 50.0).await;
    c.send(CMSG_GAMEOBJ_USE { guid: banner }).await;
    banner
}

#[tokio::test]
async fn arathi_banners_refuse_the_stale_the_absent_and_their_own_side_and_a_counter_claim_waits_a_new_minute()
 {
    let (_realm, mut a, mut b, map) = in_the_basin().await;
    let (alice, grom) = (a.guid, b.guid);
    // Alice claims the Stables.
    let neutral = use_banner(&map, &mut a, 0, 90000).await;
    said(&mut a, "claims the Stables").await;
    // The neutral banner, gone now, does nothing for Grom.
    super::tests_pvp::put(&map, &mut b, 0.0, 50.0).await;
    b.send(CMSG_GAMEOBJ_USE { guid: neutral }).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(map.query(|m| node_state(m, 0)).await, Some(1));
    // Its contested banner up: Alice's own side's does nothing for her;
    // nor for Grom, once out of the match, nor in the countdown or after
    // the end.
    fire(&map, banner_due).await;
    let contested = super::tests_pvp::object_of(&map, 90001).await;
    a.send(CMSG_GAMEOBJ_USE { guid: contested }).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let alice_assaults = map
        .query(move |m| score(m, alice).objectives[0])
        .await
        .unwrap();
    assert_eq!(alice_assaults, 1, "her own claim, once");
    for refusal in 0..3 {
        let state = map
            .query(move |m| {
                let saved = (
                    m.battleground.as_ref().unwrap().phase,
                    m.battleground.as_ref().unwrap().scores[&grom],
                );
                match refusal {
                    0 => {
                        m.battleground.as_mut().unwrap().scores.remove(&grom);
                    }
                    1 => {
                        m.battleground.as_mut().unwrap().phase =
                            Phase::Starting { at: Instant::now() };
                    }
                    _ => {
                        m.battleground.as_mut().unwrap().phase = Phase::Ended {
                            winner: 0,
                            at: Instant::now(),
                        };
                    }
                }
                crate::game::map::battleground::use_object(m, grom, contested);
                let state = node_state(m, 0);
                m.battleground.as_mut().unwrap().phase = saved.0;
                m.battleground
                    .as_mut()
                    .unwrap()
                    .scores
                    .insert(grom, saved.1);
                state
            })
            .await
            .unwrap();
        assert_eq!(state, 1, "refusal {refusal}");
    }
    // Grom counter-claims at once: the Alliance's minute no longer counts,
    // and the Horde's own minute does; an assault, not a defense.
    b.send(CMSG_GAMEOBJ_USE { guid: contested }).await;
    said(&mut b, "Grom has assaulted the Stables").await;
    let stale_capture = map
        .query(|m| {
            m.battleground
                .as_ref()
                .unwrap()
                .timers
                .iter()
                .filter(|t| capture_due(&t.1))
                .count()
        })
        .await
        .unwrap();
    assert_eq!(stale_capture, 2, "the Alliance's, and the Horde's");
    // The Alliance's timer, due first, captures nothing.
    map.query(|m| {
        let b = m.battleground.as_mut().unwrap();
        let first = b.timers.iter_mut().find(|t| capture_due(&t.1)).unwrap();
        first.0 = Instant::now();
        b.ab.as_mut().unwrap().nodes[0].capture_at = Some(Instant::now());
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        map.query(|m| node_state(m, 0)).await,
        Some(2),
        "still the Horde's to take"
    );
    fire(&map, capture_due).await;
    assert_eq!(
        map.query(|m| node_state(m, 0)).await,
        Some(4),
        "the Horde's"
    );
    let grom_columns = map.query(move |m| score(m, grom).objectives).await.unwrap();
    assert_eq!((grom_columns[0], grom_columns[1]), (1, 0));
}

#[tokio::test]
async fn four_and_five_bases_rally_their_side_only_once_theyre_held() {
    let (_realm, mut a, _b, map) = in_the_basin().await;
    let alice = a.guid;
    map.query(|m| {
        for n in 0..3 {
            ab_of(m).nodes[n].owner = Some(0);
        }
    })
    .await;
    let auras = move |m: &mut crate::game::map::Map| {
        let p = &m.players[&alice];
        (p.auras.has_spell(24061), p.auras.has_spell(24064))
    };
    // The fourth claimed: not yet.
    use_banner(&map, &mut a, 3, 90030).await;
    said(&mut a, "claims the Lumber Mill").await;
    assert_eq!(map.query(auras).await, Some((false, false)));
    // Taken: the four's.
    fire(&map, capture_due).await;
    said(&mut a, "has taken the Lumber Mill").await;
    assert_eq!(map.query(auras).await, Some((true, false)));
    // The fifth taken: the four's and the five's.
    fire(&map, banner_due).await;
    use_banner(&map, &mut a, 4, 90040).await;
    said(&mut a, "claims the Gold Mine").await;
    fire(&map, capture_due).await;
    said(&mut a, "has taken the Gold Mine").await;
    assert_eq!(map.query(auras).await, Some((true, true)));
}

#[tokio::test]
async fn arathi_resources_pause_while_a_base_is_contested_and_keep_their_progress() {
    let (_realm, _a, _b, map) = in_the_basin().await;
    let wait = || tokio::time::sleep(Duration::from_millis(150));
    // The Stables held 11 of the 12 seconds a base takes, then assaulted:
    // thirty seconds on, nothing, and the 11 seconds kept.
    map.query(|m| {
        let ab = ab_of(m);
        ab.nodes[0].owner = Some(0);
        ab.nodes[0].assaulting = Some(1);
        ab.elapsed[0] = 11_000;
        ab.last_resources = Some(Instant::now() - Duration::from_secs(30));
    })
    .await;
    wait().await;
    let (score, elapsed) = map
        .query(|m| (m.battleground.as_ref().unwrap().score, ab_of(m).elapsed[0]))
        .await
        .unwrap();
    assert_eq!((score, elapsed), ([0, 0], 11_000));
    // Defended: a second and a half more makes the twelve.
    map.query(|m| {
        let ab = ab_of(m);
        ab.nodes[0].assaulting = None;
        ab.last_resources = Some(Instant::now() - Duration::from_millis(1_500));
    })
    .await;
    wait().await;
    assert_eq!(
        map.query(|m| m.battleground.as_ref().unwrap().score).await,
        Some([10, 0])
    );
    // Two bases (nine seconds each tick), eight seconds in, then one of
    // them contested: the eight carry over to the one base's twelve.
    map.query(|m| {
        let b = m.battleground.as_mut().unwrap();
        b.score = [0, 0];
        let ab = b.ab.as_mut().unwrap();
        ab.nodes[1].owner = Some(0);
        ab.elapsed[0] = 0;
        ab.last_resources = Some(Instant::now() - Duration::from_secs(8));
    })
    .await;
    wait().await;
    map.query(|m| {
        let ab = ab_of(m);
        ab.nodes[1].assaulting = Some(1);
        ab.last_resources = Some(Instant::now() - Duration::from_millis(3_500));
    })
    .await;
    wait().await;
    assert_eq!(
        map.query(|m| m.battleground.as_ref().unwrap().score).await,
        Some([0, 0]),
        "eleven and a half of twelve"
    );
    map.query(|m| {
        ab_of(m).last_resources = Some(Instant::now() - Duration::from_millis(1_000));
    })
    .await;
    wait().await;
    assert_eq!(
        map.query(|m| m.battleground.as_ref().unwrap().score).await,
        Some([10, 0])
    );
}

#[tokio::test]
async fn arathi_honor_and_reputation_come_at_their_own_thresholds_and_double_on_a_weekend() {
    let (realm, a, b, map) = in_the_basin().await;
    let (alice, grom) = (a.guid, b.guid);
    // Two bases each for 297 seconds: 330 resources apiece, a share of
    // honor (198 at sixty) and of reputation (10 for each 200, 130 toward
    // the next).
    map.query(|m| {
        let ab = ab_of(m);
        for n in [0, 1] {
            ab.nodes[n].owner = Some(0);
        }
        for n in [2, 3] {
            ab.nodes[n].owner = Some(1);
        }
        ab.last_resources = Some(Instant::now() - Duration::from_millis(297_400));
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let data = realm.data.clone();
    let (score, honor, progress, standing) = map
        .query(move |m| {
            let b = m.battleground.as_ref().unwrap();
            let ab = b.ab.as_ref().unwrap();
            (
                b.score,
                [b.scores[&alice].bonus_honor, b.scores[&grom].bonus_honor],
                ab.reputation_progress,
                [
                    crate::game::map::reputation::standing(&m.players[&alice], &data, 509),
                    crate::game::map::reputation::standing(&m.players[&grom], &data, 510),
                ],
            )
        })
        .await
        .unwrap();
    assert_eq!(score, [330, 330]);
    assert_eq!(honor, [198, 198]);
    assert_eq!(progress, [130, 130]);
    assert!(standing.iter().all(|s| *s > 0), "{standing:?}");
    // A weekend: all five held to the end, honor each 200 resources and
    // the win's doubled, 2,376 in all for the Alliance.
    map.query(move |m| {
        let b = m.battleground.as_mut().unwrap();
        b.weekend = true;
        b.score = [0, 0];
        b.scores.get_mut(&alice).unwrap().bonus_honor = 0;
        let ab = b.ab.as_mut().unwrap();
        ab.honor_progress = [0, 0];
        ab.reputation_progress = [0, 0];
        for n in 0..5 {
            ab.nodes[n].owner = Some(0);
        }
        ab.last_resources = Some(Instant::now() - Duration::from_millis(67_400));
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let (phase, honor) = map
        .query(move |m| {
            let b = m.battleground.as_ref().unwrap();
            (b.phase, b.scores[&alice].bonus_honor)
        })
        .await
        .unwrap();
    assert!(matches!(phase, Phase::Ended { winner: 0, .. }), "{phase:?}");
    assert_eq!(honor, 2_376);
}

#[tokio::test]
async fn an_arathi_ghost_rises_at_the_nearest_base_its_side_holds() {
    let (_realm, mut a, _b, map) = in_the_basin().await;
    let alice = a.guid;
    // The bases' graveyards are at x = their ids: the Stables 895, the
    // Blacksmith 894, the Farm 893.
    map.query(|m| {
        let ab = ab_of(m);
        ab.nodes[0].owner = Some(0);
        ab.nodes[1].owner = Some(0);
        ab.nodes[2].owner = Some(1);
    })
    .await;
    super::tests_pvp::put(&map, &mut a, 893.5, 0.0).await;
    let at = move |m: &mut crate::game::map::Map| bg::graveyard(m, alice).map(|(p, _)| p.x);
    assert_eq!(map.query(at).await.flatten(), Some(894.0), "the Blacksmith");
    map.query(|m| ab_of(m).nodes[1].assaulting = Some(1)).await;
    assert_eq!(
        map.query(at).await.flatten(),
        Some(895.0),
        "contested: the Stables"
    );
    map.query(|m| ab_of(m).nodes[0].assaulting = Some(1)).await;
    assert_eq!(
        map.query(at).await.flatten(),
        Some(898.0),
        "none: the Alliance's own"
    );
    map.query(|m| {
        m.battleground.as_mut().unwrap().phase = Phase::Starting {
            at: Instant::now() + Duration::from_secs(60),
        }
    })
    .await;
    assert_eq!(
        map.query(at).await.flatten(),
        Some(0.0),
        "the countdown: the start"
    );
}

#[tokio::test]
async fn one_coming_into_arathi_basin_is_told_every_bases_state() {
    let realm = harness::realm_with(basin());
    let mut a = player(&realm, "A", "Alice", Race::Human).await;
    let b = player(&realm, "B", "Grom", Race::Orc).await;
    queue(&a, Map::ArathiBasin).await;
    queue(&b, Map::ArathiBasin).await;
    into(&realm, &mut a, Map::ArathiBasin).await;
    let states = a
        .wait_for("the basin's states", |m| match m {
            ServerOpcodeMessage::SMSG_INIT_WORLD_STATES(w) if w.map == Map::ArathiBasin => {
                Some(w.states.clone())
            }
            _ => None,
        })
        .await;
    let of = |id: u32| states.iter().find(|w| w.state == id).map(|w| w.value);
    assert_eq!(states.len(), 32);
    assert_eq!((of(1780), of(1955)), (Some(2000), Some(1800)));
    for neutral in [1842, 1846, 1845, 1844, 1843] {
        assert_eq!(of(neutral), Some(1), "{neutral}");
    }
    for held in [1767, 1782, 1772, 1792, 1787] {
        assert_eq!(of(held), Some(0), "{held}");
    }
}

#[tokio::test]
async fn one_who_logs_out_of_arathi_basin_keeps_their_scores_and_leaves_the_resurrection_queue() {
    let (realm, mut a, _b, map) = in_the_basin().await;
    let alice = a.guid;
    use_banner(&map, &mut a, 0, 90000).await;
    said(&mut a, "claims the Stables").await;
    // Dead and queued at a spirit guide, she logs out: out of the queue.
    map.query(move |m| {
        death::kill(m, alice);
        death::release(m, alice);
        bg::queue_resurrection(m, alice);
    })
    .await;
    assert_eq!(
        map.query(move |m| m
            .battleground
            .as_ref()
            .unwrap()
            .resurrect_queue
            .contains(&alice))
            .await,
        Some(true)
    );
    a.disconnect();
    until(&map, "Alice gone", move |m| !m.players.contains_key(&alice)).await;
    assert_eq!(
        map.query(move |m| m
            .battleground
            .as_ref()
            .unwrap()
            .resurrect_queue
            .contains(&alice))
            .await,
        Some(false)
    );
    // Back, her base assaulted still to her name.
    let mut a = Client::connect(&realm, "A");
    a.login(alice).await;
    until(&map, "Alice back", move |m| m.players.contains_key(&alice)).await;
    assert_eq!(
        map.query(move |m| score(m, alice).objectives[0]).await,
        Some(1)
    );
}

// The portals into the battlegrounds, and the deserter.

/// The real spell `id` in `d`, with its cast time, duration, range and
/// radii.
pub(super) fn copy_spell(d: &mut GameData, real: &GameData, id: u32) {
    let Some(s) = real.spells.spells.get(&id) else {
        return;
    };
    let r = &real.spells;
    if let Some(c) = r.cast_times.get(&s.casting_time_index) {
        d.spells.cast_times.insert(s.casting_time_index, *c);
    }
    if let Some(du) = r.durations.get(&s.duration_index) {
        d.spells.durations.insert(s.duration_index, *du);
    }
    if let Some(range) = r.ranges.get(&s.range_index) {
        d.spells.ranges.insert(s.range_index, *range);
    }
    for e in &s.effects {
        if let Some(radius) = r.radii.get(&e.radius_index) {
            d.spells.radii.insert(e.radius_index, *radius);
        }
    }
    d.spells.spells.insert(id, s.clone());
}

/// Warsong Gulch from tenth level, Alterac Valley from fifty-first, and
/// three portals into them by the start: the Alliance's into the Gulch,
/// and the Horde's into each.
fn portals() -> GameData {
    let mut d = GameData::empty();
    battleground(&mut d, WSG, 2, "PVPZone03", [769, 770]);
    battleground(&mut d, 30, 1, "PVPZone01", [611, 610]);
    d.battlegrounds.get_mut(&2).unwrap().min_level = 10;
    d.battlegrounds.get_mut(&1).unwrap().min_level = 51;
    for (trigger, bg, team, x, map) in [
        (3650, 2, 469, 100.0, 0),
        (3651, 2, 67, 200.0, 0),
        (3652, 1, 67, 300.0, 1),
    ] {
        d.area_triggers.insert(
            trigger,
            AreaTrigger {
                id: trigger,
                map,
                x,
                y: 0.0,
                z: 0.0,
                radius: 5.0,
                ..Default::default()
            },
        );
        d.bg_entrances.insert(
            trigger,
            BgEntrance {
                trigger,
                bg,
                team,
                exit_map: map,
                exit: (x + 20.0, 0.0, 0.0, 1.5),
            },
        );
    }
    d
}

async fn walk_into(map: &MapHandle, c: &mut Client, trigger: u32, x: f32) {
    put(map, c, x, 0.0, 0.0).await;
    c.send(CMSG_AREATRIGGER {
        trigger_id: trigger,
    })
    .await;
}

#[tokio::test]
async fn a_battleground_portal_lists_its_battle_for_its_side_and_levels_and_says_what_it_wants() {
    let realm = harness::realm_with(portals());
    let mut a = player(&realm, "A", "Alice", Race::Human).await;
    let mut b = player(&realm, "B", "Grom", Race::Orc).await;
    let (alice, grom) = (a.guid, b.guid);
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let level = |g: Guid, l: u32| {
        move |m: &mut crate::game::map::Map| {
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_LEVEL, l)
        }
    };
    let gmap = realm
        .maps
        .get(&realm, realm.registry.online(grom).unwrap().map().key);
    map.query(level(alice, 9)).await;
    gmap.query(level(grom, 50)).await;
    // At ninth level, the Alliance's portal is closed to Alice, and the
    // Horde's to her at any level.
    walk_into(&map, &mut a, 3650, 100.0).await;
    assert_eq!(
        a.system_message("to enter").await,
        "You must be in the Alliance and at least 10th level to enter."
    );
    map.query(level(alice, 10)).await;
    walk_into(&map, &mut a, 3651, 200.0).await;
    assert_eq!(
        a.system_message("to enter").await,
        "You must be in the Horde and at least 10th level to enter."
    );
    // At tenth, her own lists the Gulch's battles, and she's out at its
    // exit.
    a.clear();
    walk_into(&map, &mut a, 3650, 100.0).await;
    let listed = a
        .wait_for("the list", |m| match m {
            ServerOpcodeMessage::SMSG_BATTLEFIELD_LIST(l) => Some(l.map),
            _ => None,
        })
        .await;
    assert_eq!(listed, Map::WarsongGulch);
    a.wait_for("out at the exit", |m| match m {
        ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => (t.info.position.x == 120.0).then_some(()),
        _ => None,
    })
    .await;
    // Fiftieth is short of Alterac Valley's fifty-first for Grom.
    walk_into(&gmap, &mut b, 3652, 300.0).await;
    assert_eq!(
        b.system_message("to enter").await,
        "You must be in the Horde and at least 51st level to enter."
    );
}

#[test]
#[ignore = "extended"]
fn the_real_portals_say_their_side_battle_and_exit() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    let e = &d.bg_entrances[&3650];
    assert_eq!((e.team, e.bg, e.exit_map), (469, 2, 1));
    let (x, y, z, o) = e.exit;
    assert!(
        (x - 1454.12).abs() < 0.01 && (y + 1858.47).abs() < 0.01,
        "{x} {y}"
    );
    assert!(
        (z - 126.402).abs() < 0.01 && (o - 6.194).abs() < 0.01,
        "{z} {o}"
    );
    assert!(d.bg_entrances.values().any(|e| e.team == 67));
}

#[tokio::test]
#[ignore = "extended"]
async fn deserter_is_a_harmful_quarter_hour_that_cant_be_cancelled_and_outlasts_death() {
    let Some(real) = crate::game::data::load::real_data() else {
        return;
    };
    let mut d = GameData::empty();
    copy_spell(&mut d, real, bg::SPELL_DESERTER);
    let realm = harness::realm_with(d);
    let a = player(&realm, "A", "Alice", Race::Human).await;
    let alice = a.guid;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let holder = move |m: &mut crate::game::map::Map| {
        m.players[&alice]
            .auras
            .holders
            .iter()
            .find(|h| h.spell == bg::SPELL_DESERTER)
            .map(|h| (h.positive, h.expires))
    };
    map.query(move |m| {
        crate::game::map::spells::cast_triggered(m, alice, bg::SPELL_DESERTER, Some(alice))
    })
    .await;
    let (positive, expires) = map.query(holder).await.flatten().expect("a deserter");
    assert!(!positive);
    let left = expires.unwrap().saturating_duration_since(Instant::now());
    assert!(
        left > Duration::from_secs(890) && left <= Duration::from_secs(900),
        "{left:?}"
    );
    // Not hers to cancel.
    a.send(CMSG_CANCEL_AURA {
        id: bg::SPELL_DESERTER,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(map.query(holder).await.flatten().is_some());
    // She dies: it stays, to the same end.
    map.query(move |m| crate::game::map::combat::kill(m, alice, alice))
        .await;
    let (_, after) = map
        .query(holder)
        .await
        .flatten()
        .expect("kept through death");
    assert_eq!(after, expires);
}

// Alterac Valley.

const AV: u32 = 30;
const GALVANGAR: u32 = 90_049;
const BALINDA: u32 = 90_048;
const COMMANDER: u32 = 90_057;
const LIEUTENANT: u32 = 90_069;
const KEEPERS: [u32; 3] = [90_046, 90_146, 90_246];

/// The valley with Stonehearth Graveyard's banners (node 2) and
/// Snowfall's (node 3) in each state, the captains, a Horde commander,
/// two Horde lieutenants, and Irondeep's keepers: its own, the Alliance's
/// and the Horde's.
fn vale() -> GameData {
    let mut d = super::tests_pvp2::valley();
    for (node, x) in [(2u8, 40.0), (3, 50.0)] {
        for state in [0u8, 1, 2, 3, 5] {
            bound_object(
                &mut d,
                AV,
                161_000 + u32::from(node) * 10 + u32::from(state),
                178365,
                (x, 0.0),
                (node, state),
            );
        }
    }
    let mut creature = |guid: u32, entry: u32, name: &str, x: f32, event: (u8, u8)| {
        super::tests_pvp2::npc(&mut d, entry, name, 14);
        super::tests_pvp2::spawn(&mut d, guid, entry, x);
        d.bg_creature_events.entry(guid).or_default().push(event);
    };
    creature(171_049, GALVANGAR, "Galvangar", 60.0, (49, 0));
    creature(171_048, BALINDA, "Balinda Stonehearth", 61.0, (48, 0));
    creature(171_057, COMMANDER, "Commander", 62.0, (57, 0));
    creature(171_069, LIEUTENANT, "Lieutenant", 63.0, (69, 0));
    creature(171_070, LIEUTENANT, "Lieutenant", 64.0, (69, 0));
    creature(171_146, KEEPERS[0], "Irondeep's Own", 65.0, (46, 2));
    creature(171_246, KEEPERS[1], "Irondeep's Alliance", 66.0, (46, 0));
    creature(171_346, KEEPERS[2], "Irondeep's Horde", 67.0, (46, 1));
    for spell in [22751, 23693, 23658] {
        marker(&mut d, spell);
    }
    // The assault credit's quest, for both.
    let mut q = crate::game::data::npc::Quest {
        id: 90_000,
        ..Default::default()
    };
    q.required_kills[0] = (13756, 5);
    d.npc.quests.insert(90_000, q);
    d
}

fn av_of(m: &mut crate::game::map::Map) -> &mut crate::game::map::battleground::av::Av {
    m.battleground.as_mut().unwrap().av.as_mut().unwrap()
}

/// Alice and Grom in the valley, each with the assault quest.
async fn in_the_valley(d: GameData) -> (Arc<Realm>, Client, Client, MapHandle) {
    let realm = harness::realm_with(d);
    let mut a = player(&realm, "A", "Alice", Race::Human).await;
    let mut b = player(&realm, "B", "Grom", Race::Orc).await;
    let map = super::tests_pvp::both_into(&realm, &mut a, &mut b, Map::AlteracValley).await;
    let (alice, grom) = (a.guid, b.guid);
    map.query(move |m| {
        for g in [alice, grom] {
            m.players
                .get_mut(&g)
                .unwrap()
                .journal
                .log
                .push(crate::game::store::Logged {
                    quest: 90_000,
                    ..Default::default()
                });
        }
    })
    .await;
    (realm, a, b, map)
}

fn credit(m: &crate::game::map::Map, g: Guid) -> u32 {
    m.players[&g].journal.log[0].kills[0]
}

fn event(m: &crate::game::map::Map, e: u8) -> Option<Option<u8>> {
    m.battleground.as_ref().unwrap().events.get(&e).copied()
}

/// The creature of `spawn`, killed by `killer`.
fn slay(m: &mut crate::game::map::Map, spawn: u32, killer: Guid) {
    let c = m
        .creatures
        .iter()
        .find(|(_, c)| c.spawn_guid == Some(spawn))
        .map(|(_, c)| c.guid)
        .expect("the creature");
    crate::game::map::combat::kill(m, killer, c);
}

#[tokio::test]
async fn an_alterac_graveyard_assaulted_stops_its_defenders_respawns_and_its_defense_brings_them_back()
 {
    let (_realm, mut a, mut b, map) = in_the_valley(vale()).await;
    let (alice, grom) = (a.guid, b.guid);
    // Grom assaults Stonehearth Graveyard, the Alliance's: its defenders
    // (event 17) stop coming back, and he has the assault's credit.
    let held = super::tests_pvp::object_of(&map, 161_021).await;
    super::tests_pvp::put(&map, &mut b, 40.0, 0.0).await;
    b.send(CMSG_GAMEOBJ_USE { guid: held }).await;
    said(&mut b, "The Horde has assaulted Stonehearth Graveyard").await;
    let (defenders, assaulted, credited) = map
        .query(move |m| (event(m, 17), score(m, grom).objectives[0], credit(m, grom)))
        .await
        .unwrap();
    assert_eq!(defenders, Some(None), "no respawns");
    assert_eq!((assaulted, credited), (1, 1));
    // Alice takes it back at once: the defenders return, a defense and no
    // credit; the stale banner and capture timers do nothing.
    fire(&map, banner_due).await;
    let contested = super::tests_pvp::object_of(&map, 161_022).await;
    super::tests_pvp::put(&map, &mut a, 40.0, 0.0).await;
    a.send(CMSG_GAMEOBJ_USE { guid: contested }).await;
    said(&mut a, "The Alliance has defended Stonehearth Graveyard").await;
    map.query(|m| {
        let b = m.battleground.as_mut().unwrap();
        let now = Instant::now();
        for t in b.timers.iter_mut() {
            t.0 = now;
        }
        b.av.as_mut().unwrap().nodes[2].point.capture_at = Some(now);
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let (defenders, state, columns, credited) = map
        .query(move |m| {
            (
                event(m, 17),
                av_of(m).event_state(2),
                [score(m, alice).objectives, score(m, grom).objectives],
                credit(m, alice),
            )
        })
        .await
        .unwrap();
    assert_eq!(defenders, Some(Some(0)));
    assert_eq!(state, 1, "the Alliance's");
    assert_eq!(columns, [[0, 1, 0, 0, 0, 0, 0], [1, 0, 0, 0, 0, 0, 0]]);
    assert_eq!(credited, 0);
}

#[tokio::test]
async fn snowfall_claimed_gives_no_assault_credit_until_its_taken_from_an_enemy() {
    let (_realm, mut a, mut b, map) = in_the_valley(vale()).await;
    let (alice, grom) = (a.guid, b.guid);
    let neutral = super::tests_pvp::object_of(&map, 161_035).await;
    super::tests_pvp::put(&map, &mut a, 50.0, 0.0).await;
    a.send(CMSG_GAMEOBJ_USE { guid: neutral }).await;
    said(&mut a, "The Alliance has assaulted Snowfall Graveyard").await;
    fire(&map, capture_due).await;
    said(&mut a, "The Alliance has taken Snowfall Graveyard").await;
    assert_eq!(map.query(move |m| credit(m, alice)).await, Some(0));
    fire(&map, banner_due).await;
    let held = super::tests_pvp::object_of(&map, 161_031).await;
    super::tests_pvp::put(&map, &mut b, 50.0, 0.0).await;
    b.send(CMSG_GAMEOBJ_USE { guid: held }).await;
    said(&mut b, "The Horde has assaulted Snowfall Graveyard").await;
    assert_eq!(map.query(move |m| credit(m, grom)).await, Some(1));
}

#[tokio::test]
async fn alterac_banners_refuse_the_countdown_the_absent_and_the_stale() {
    let (_realm, _a, mut b, map) = in_the_valley(vale()).await;
    let grom = b.guid;
    let held = super::tests_pvp::object_of(&map, 161_021).await;
    super::tests_pvp::put(&map, &mut b, 40.0, 0.0).await;
    for refusal in 0..3 {
        let state = map
            .query(move |m| {
                let b = m.battleground.as_mut().unwrap();
                let saved = (b.phase, b.scores[&grom]);
                match refusal {
                    0 => b.phase = Phase::Starting { at: Instant::now() },
                    1 => {
                        b.scores.remove(&grom);
                    }
                    // Stale: the banner's state is no longer the node's.
                    _ => b.av.as_mut().unwrap().nodes[2].point.owner = Some(1),
                }
                bg::use_object(m, grom, held);
                let state = av_of(m).event_state(2);
                let b = m.battleground.as_mut().unwrap();
                b.phase = saved.0;
                b.scores.insert(grom, saved.1);
                b.av.as_mut().unwrap().nodes[2].point.owner = Some(0);
                (state, b.av.as_ref().unwrap().nodes[2].point.assaulting)
            })
            .await
            .unwrap();
        assert_eq!(state.1, None, "refusal {refusal}: not assaulted");
    }
}

#[tokio::test]
async fn a_slain_captain_rallies_no_more_and_counts_once_and_lieutenants_count_each_time() {
    let (_realm, a, b, map) = in_the_valley(vale()).await;
    let (alice, grom) = (a.guid, b.guid);
    // Galvangar's rally, before.
    map.query(|m| crate::game::map::battleground::av::due(m, bg::Due::CaptainBuff(1)))
        .await;
    assert_eq!(
        map.query(move |m| m.players[&grom].auras.has_spell(22751))
            .await,
        Some(true)
    );
    map.query(move |m| {
        crate::game::map::auras::remove_spell(m, grom, 22751, None);
        slay(m, 171_049, alice);
    })
    .await;
    let (honor, leaders, respawns, dead) = map
        .query(move |m| {
            (
                score(m, alice).bonus_honor,
                score(m, alice).objectives[5],
                event(m, 49),
                event(m, 64),
            )
        })
        .await
        .unwrap();
    assert_eq!((honor, leaders), (594, 1));
    assert_eq!((respawns, dead), (Some(None), Some(Some(0))));
    // He rallies no more; Balinda still does.
    map.query(|m| {
        crate::game::map::battleground::av::due(m, bg::Due::CaptainBuff(1));
        crate::game::map::battleground::av::due(m, bg::Due::CaptainBuff(0));
    })
    .await;
    let (horde, alliance) = map
        .query(move |m| {
            (
                m.players[&grom].auras.has_spell(22751),
                m.players[&alice].auras.has_spell(23693),
            )
        })
        .await
        .unwrap();
    assert!(!horde && alliance);
    // The commander once; each lieutenant, 198 apiece.
    map.query(move |m| {
        slay(m, 171_057, alice);
        slay(m, 171_069, alice);
        slay(m, 171_070, alice);
    })
    .await;
    let (honor, leaders) = map
        .query(move |m| (score(m, alice).bonus_honor, score(m, alice).objectives[5]))
        .await
        .unwrap();
    assert_eq!((honor, leaders), (594 + 3 * 198, 4));
    // A commander's second coming counts for nothing.
    map.query(move |m| {
        let b = m.battleground.as_mut().unwrap();
        b.events.insert(57, Some(0));
        let data = m.realm.data.clone();
        let spawn = data.creature_spawns[&AV]
            .iter()
            .find(|s| s.guid == 171_057)
            .unwrap()
            .clone();
        let mut c =
            crate::game::map::creature::Creature::from_spawn(&data, &spawn, &mut rand::rng())
                .unwrap();
        c.guid = crate::game::guid::unit(COMMANDER, 0x7F_0000);
        let g = m.add_creature(c);
        crate::game::map::combat::kill(m, alice, g);
    })
    .await;
    assert_eq!(
        map.query(move |m| score(m, alice).bonus_honor).await,
        Some(594 + 3 * 198)
    );
}

#[tokio::test]
async fn irondeep_goes_to_whoever_kills_its_keeper_and_back_to_its_own_after_twenty_minutes() {
    let (_realm, mut a, b, map) = in_the_valley(vale()).await;
    let (alice, grom) = (a.guid, b.guid);
    let fields = |m: &mut crate::game::map::Map| {
        m.battleground
            .as_ref()
            .unwrap()
            .world_states()
            .into_iter()
            .filter(|w| (1358..=1360).contains(&w.state))
            .map(|w| w.value)
            .collect::<Vec<u32>>()
    };
    assert_eq!(map.query(fields).await, Some(vec![0, 0, 1]), "its own");
    // Alice kills its own keeper: the Alliance's, its keeper and miners
    // (events 46 and 50) the Alliance's.
    map.query(move |m| slay(m, 171_146, alice)).await;
    said(&mut a, "The Alliance has taken Irondeep Mine").await;
    let (owner, events, mines) = map
        .query(move |m| {
            (
                av_of(m).mines[0].owner,
                (event(m, 46), event(m, 50)),
                score(m, alice).objectives[4],
            )
        })
        .await
        .unwrap();
    assert_eq!(
        (owner, events, mines),
        (Some(0), (Some(Some(0)), Some(Some(0))), 1)
    );
    assert_eq!(map.query(fields).await, Some(vec![1, 0, 0]));
    // Alice kills the Alliance's keeper again: nothing more, no new timer.
    let reclaims = |m: &mut crate::game::map::Map| {
        m.battleground
            .as_ref()
            .unwrap()
            .timers
            .iter()
            .filter(|t| matches!(t.1, bg::Due::MineReclaim { .. }))
            .count()
    };
    map.query(move |m| slay(m, 171_246, alice)).await;
    assert_eq!(map.query(reclaims).await, Some(1));
    // Grom takes it: the first timer is stale; the Horde's twenty minutes
    // give it back to its own.
    map.query(move |m| {
        m.battleground.as_mut().unwrap().events.insert(46, Some(0));
        let data = m.realm.data.clone();
        let spawn = data.creature_spawns[&AV]
            .iter()
            .find(|s| s.guid == 171_246)
            .unwrap()
            .clone();
        let mut c =
            crate::game::map::creature::Creature::from_spawn(&data, &spawn, &mut rand::rng())
                .unwrap();
        c.guid = crate::game::guid::unit(KEEPERS[1], 0x7F_0001);
        let g = m.add_creature(c);
        crate::game::map::combat::kill(m, grom, g);
    })
    .await;
    assert_eq!(map.query(|m| av_of(m).mines[0].owner).await, Some(Some(1)));
    assert_eq!(map.query(fields).await, Some(vec![0, 1, 0]));
    map.query(|m| {
        let b = m.battleground.as_mut().unwrap();
        let first = b
            .timers
            .iter_mut()
            .find(|t| matches!(t.1, bg::Due::MineReclaim { revision: 1, .. }))
            .unwrap();
        first.0 = Instant::now();
    })
    .await;
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(
        map.query(|m| av_of(m).mines[0].owner).await,
        Some(Some(1)),
        "stale"
    );
    fire(&map, |d| matches!(d, bg::Due::MineReclaim { .. })).await;
    said(
        &mut a,
        "Irondeep Mine has been reclaimed by its original inhabitants",
    )
    .await;
    let (owner, events, mines) = map
        .query(move |m| {
            (
                av_of(m).mines[0].owner,
                (event(m, 46), event(m, 50)),
                score(m, grom).objectives[4],
            )
        })
        .await
        .unwrap();
    assert_eq!(
        (owner, events, mines),
        (None, (Some(Some(2)), Some(Some(2))), 1)
    );
    assert_eq!(map.query(fields).await, Some(vec![0, 0, 1]));
}

#[tokio::test]
async fn alterac_valleys_scoreboard_has_its_seven_columns_in_order() {
    let (_realm, mut a, _b, map) = in_the_valley(vale()).await;
    let alice = a.guid;
    map.query(move |m| {
        let s = m
            .battleground
            .as_mut()
            .unwrap()
            .scores
            .get_mut(&alice)
            .unwrap();
        s.objectives = [0, 0, 0, 0, 2, 3, 4];
    })
    .await;
    a.clear();
    a.send(MSG_PVP_LOG_DATA_Client {}).await;
    let fields = a
        .wait_for("the scoreboard", |m| match m {
            ServerOpcodeMessage::MSG_PVP_LOG_DATA(l) => l
                .players
                .iter()
                .find(|p| p.player == alice)
                .map(|p| p.fields.clone()),
            _ => None,
        })
        .await;
    assert_eq!(fields, vec![0, 0, 0, 0, 2, 3, 4]);
}

#[test]
fn alteracs_map_shows_one_icon_per_objective() {
    use crate::game::map::battleground::av::CAPTURE;
    let now = Instant::now();
    let m = bg::Match::new(bg::BG_AV, now);
    let states = m.world_states();
    assert_eq!(states.len(), 67);
    assert_eq!(states.iter().filter(|w| w.value == 1).count(), 17);
    let mut ids: Vec<u32> = states.iter().map(|w| w.state).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 67);
    assert!(
        !states
            .iter()
            .any(|w| [3127, 3128, 3133, 3134].contains(&w.state))
    );
    // Each node, assaulted by the other side and then taken, shows one icon
    // at each step, and a different one.
    let home = [0usize, 0, 0, 2, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1];
    for (id, home) in home.into_iter().enumerate() {
        let enemy = if home == 0 { 1 } else { 0 };
        let mut m = bg::Match::new(bg::BG_AV, now);
        let icons = |m: &bg::Match| -> Vec<(u32, u32)> {
            m.world_states()
                .into_iter()
                .map(|w| (w.state, w.value))
                .collect()
        };
        let before = icons(&m);
        let av = m.av.as_mut().unwrap();
        av.nodes[id].point.assault(enemy, now, CAPTURE);
        let attacked = icons(&m);
        let av = m.av.as_mut().unwrap();
        let rev = av.nodes[id].point.revision;
        assert!(av.nodes[id].point.capture(rev, now + CAPTURE));
        let captured = icons(&m);
        for states in [&before, &attacked, &captured] {
            assert_eq!(states.iter().filter(|s| s.1 == 1).count(), 17, "node {id}");
            assert!(states.iter().all(|s| s.1 <= 1));
        }
        assert_ne!(before, attacked, "node {id}");
        assert_ne!(attacked, captured, "node {id}");
    }
}

#[test]
fn snowfalls_counterassault_waits_its_full_five_minutes_and_a_defense_is_at_once() {
    use crate::game::map::battleground::av::{Av, CAPTURE};
    let t0 = Instant::now();
    let mut av = Av::new();
    let p = &mut av.nodes[3].point;
    assert_eq!(p.assault(0, t0, CAPTURE), bg::Assault::Assaulted);
    let alliance = p.revision;
    let t1 = t0 + Duration::from_secs(299);
    assert_eq!(p.assault(1, t1, CAPTURE), bg::Assault::Assaulted);
    assert_eq!((p.owner, p.capture_at), (None, Some(t1 + CAPTURE)));
    let horde = p.revision;
    assert!(
        !p.capture(alliance, t0 + CAPTURE),
        "the Alliance's minute is stale"
    );
    assert!(!p.capture(horde, t1 + CAPTURE - Duration::from_millis(1)));
    assert!(p.capture(horde, t1 + CAPTURE));
    assert_eq!(av.controlled_by(3), Some(1));
    let p = &mut av.nodes[3].point;
    assert_eq!(p.assault(0, t1 + CAPTURE, CAPTURE), bg::Assault::Assaulted);
    assert_eq!(p.assault(1, t1 + CAPTURE, CAPTURE), bg::Assault::Defended);
    assert_eq!(av.controlled_by(3), Some(1));
}

/// Drek'Thar falls to Alice after `setup`: Alice's and Grom's bonus
/// honor, and Alice's standing with the Stormpike Guard.
async fn drekthar_falls(setup: fn(&mut crate::game::map::Map)) -> (u32, u32, i32) {
    let (realm, a, b, map) = in_the_valley(vale()).await;
    let (alice, grom) = (a.guid, b.guid);
    map.query(move |m| {
        setup(m);
        slay(m, 150_100, alice);
    })
    .await;
    let data = realm.data.clone();
    map.query(move |m| {
        assert!(matches!(
            m.battleground.as_ref().unwrap().phase,
            Phase::Ended { winner: 0, .. }
        ));
        (
            score(m, alice).bonus_honor,
            score(m, grom).bonus_honor,
            crate::game::map::reputation::standing(&m.players[&alice], &data, 730),
        )
    })
    .await
    .unwrap()
}

fn an_hour_in(m: &mut crate::game::map::Map) {
    m.battleground.as_mut().unwrap().started_at = Some(Instant::now() - Duration::from_secs(3600));
}

#[tokio::test]
async fn a_generals_fall_pays_by_the_matchs_length_and_what_each_side_still_holds() {
    // An hour in: the general's six kills' worth, and each side's four
    // towers, captain and two graveyards, in full.
    let (alice, grom, rep) = drekthar_falls(an_hour_in).await;
    assert_eq!(
        (alice, grom),
        (198 * (6 + 4 * 3 + 3 + 2), 198 * (4 * 3 + 3 + 2))
    );
    assert_eq!(rep, 350 + 4 * 12 + 125 + 2 * 12);
    // At once: the general, towers and graveyards a sixtieth, the captain
    // whole.
    let (alice, grom, _) = drekthar_falls(|_| {}).await;
    assert_eq!((alice, grom), (19 + 39 + 594 + 6, 39 + 594 + 6));
    // A tower contested and the Alliance's captain dead: neither pays.
    let (alice, grom, _) = drekthar_falls(|m| {
        an_hour_in(m);
        let av = av_of(m);
        av.nodes[7].point.assaulting = Some(1);
        av.defeated.insert(48);
    })
    .await;
    assert_eq!(
        (alice, grom),
        (198 * (6 + 3 * 3 + 2), 198 * (4 * 3 + 3 + 2))
    );
    // On a weekend, 1,584 more each and 396 the winner's, and the general's
    // reputation 525.
    let (alice, grom, rep) = drekthar_falls(|m| {
        an_hour_in(m);
        m.battleground.as_mut().unwrap().weekend = true;
        let av = av_of(m);
        av.nodes[7].point.assaulting = Some(1);
        av.defeated.insert(48);
    })
    .await;
    assert_eq!(
        (alice, grom),
        (198 * (6 + 3 * 3 + 2) + 1_980, 198 * (4 * 3 + 3 + 2) + 1_584)
    );
    assert_eq!(rep, 525 + 3 * 18 + 2 * 18);
}

#[tokio::test]
async fn a_group_of_two_brackets_isnt_queued_and_a_copy_left_empty_closes() {
    let realm = harness::realm_with(gulch());
    let mut a = player(&realm, "A", "Alice", Race::Human).await;
    let mut c = player(&realm, "C", "Carl", Race::Human).await;
    let (alice, carl) = (a.guid, c.guid);
    // Alice at sixty and Carl at nineteen: their group joins nothing.
    let home = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    home.query(move |m| {
        m.players
            .get_mut(&alice)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_LEVEL, 60);
        m.players
            .get_mut(&carl)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_LEVEL, 19);
    })
    .await;
    a.send(CMSG_GROUP_INVITE {
        name: "Carl".into(),
    })
    .await;
    c.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    c.send(CMSG_GROUP_ACCEPT {}).await;
    a.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
        _ => None,
    })
    .await;
    a.send(CMSG_BATTLEMASTER_JOIN {
        guid: Guid::zero(),
        map: Map::WarsongGulch,
        instance_id: 0,
        join_as_group: true,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let queued = {
        let bgs = realm.social.battlegrounds.lock();
        [alice, carl].map(|g| bgs.match_of(g).is_some())
    };
    assert_eq!(queued, [false, false], "neither in a match");
    let heard = c
        .gets(Duration::from_millis(200), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_JOINED_BATTLEGROUND(j) if j.id != BgTypeId::RemoveFromQueue)
                .then_some(())
        })
        .await;
    assert!(heard.is_none(), "Carl isn't queued");
    // A match whose last player leaves closes its copy.
    let mut b = player(&realm, "B", "Grom", Race::Orc).await;
    let mut d = player(&realm, "D", "Dana", Race::Human).await;
    let map = all_in(&realm, &mut [&mut d, &mut b]).await;
    let key = map.key;
    for cl in [&d, &b] {
        cl.send(CMSG_LEAVE_BATTLEFIELD {
            map: Map::WarsongGulch,
        })
        .await;
    }
    let mut closed = false;
    for _ in 0..300 {
        closed = !realm
            .social
            .battlegrounds
            .lock()
            .matches
            .contains_key(&key.instance);
        if closed {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(closed, "the copy closed");
}

/// In a battleground a player sees out to the battleground's own distance
/// (400 yards by default), where in the world they see 100: two players
/// 300 yards apart know each other in Warsong Gulch, and not in the world.
#[tokio::test]
async fn a_battleground_is_seen_further_than_the_world() {
    let realm = delayed(gulch(), Duration::from_secs(120));
    let mut a = player(&realm, "A", "Alice", Race::Human).await;
    let mut b = player(&realm, "B", "Grom", Race::Orc).await;
    let c = player(&realm, "C", "Cara", Race::Human).await;
    let apart = |ga: Guid, gb: Guid| {
        move |m: &mut crate::game::map::Map| {
            let at = m.players[&ga].position();
            let p = m.players.get_mut(&gb)?;
            p.movement.position = Vector3d {
                x: at.x + 300.0,
                ..at
            };
            let to = p.movement.position;
            crate::game::map::visibility::moved(m, gb, to, to);
            crate::game::map::visibility::refresh(m, gb);
            crate::game::map::visibility::refresh(m, ga);
            Some((
                m.players[&ga].known.contains(&gb),
                crate::game::map::visibility::radius(m),
            ))
        }
    };
    // Apart in the world: 300 yards, beyond its 100.
    let world = realm.registry.online(a.guid).unwrap().map();
    assert_eq!(
        world.query(apart(a.guid, c.guid)).await.flatten(),
        Some((false, 100.0))
    );
    queue(&a, Map::WarsongGulch).await;
    queue(&b, Map::WarsongGulch).await;
    let map = into(&realm, &mut a, Map::WarsongGulch).await;
    into(&realm, &mut b, Map::WarsongGulch).await;
    let mut seen = None;
    for _ in 0..200 {
        seen = map.query(apart(a.guid, b.guid)).await.flatten();
        if seen.is_some_and(|(knows, _)| knows) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(seen, Some((true, 400.0)), "300 yards off in the gulch");
}
