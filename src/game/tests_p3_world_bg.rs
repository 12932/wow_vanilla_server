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
fn gulch() -> GameData {
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
async fn put(map: &MapHandle, c: &mut Client, x: f32, y: f32, z: f32) {
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

async fn player(realm: &Arc<Realm>, account: &str, name: &str, race: Race) -> Client {
    let mut c = Client::connect(realm, account);
    c.enter(name, race, Class::Warrior).await;
    c
}

/// Each of `clients` queues and goes in, and the battle is on.
async fn all_in(realm: &Arc<Realm>, clients: &mut [&mut Client]) -> MapHandle {
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

fn score(m: &crate::game::map::Map, g: Guid) -> bg::Score {
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
    a.wait_frame(
        "the status in progress",
        crate::game::battlegrounds::SMSG_BATTLEFIELD_STATUS,
        |f| f.len() >= 17 && f[13] == 3,
    )
    .await;
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
        .iter()
        .map(|(_, o)| o)
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
