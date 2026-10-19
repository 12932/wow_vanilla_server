//! Battlegrounds, honor and world PvP through the protocol.

use std::time::{Duration, Instant};

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;

const AB: u32 = 529;
const AV: u32 = 30;

/// A battleground on its map, with each team's start.
pub(super) fn battleground(d: &mut GameData, map: u32, bg: u32, name: &str, starts: [u32; 2]) {
    d.maps.insert(
        map,
        MapInfo {
            id: map,
            internal_name: name.into(),
            name: name.into(),
            instance_type: 3,
            player_limit: 40,
        },
    );
    d.battlegrounds.insert(
        bg,
        BgTemplate {
            id: bg,
            min_players: 1,
            max_players: 40,
            min_level: 1,
            max_level: 60,
            starts,
            ..Default::default()
        },
    );
    d.safe_locs.insert(starts[0], (map, 0.0, 0.0, 0.0));
    d.safe_locs.insert(starts[1], (map, 100.0, 100.0, 0.0));
}

pub(super) fn banner(d: &mut GameData, entry: u32, kind: u8) {
    d.gameobject_templates.insert(
        entry,
        GameObjectTemplate {
            entry,
            kind,
            display_id: 1,
            size: 1.0,
            name: format!("Banner {entry}"),
            ..Default::default()
        },
    );
}

/// An object of the battleground, in an event's state.
pub(super) fn bound_object(
    d: &mut GameData,
    map: u32,
    guid: u32,
    entry: u32,
    at: (f32, f32),
    event: (u8, u8),
) {
    d.gameobject_spawns
        .entry(map)
        .or_default()
        .push(GameObjectSpawn {
            guid,
            entry,
            map,
            x: at.0,
            y: at.1,
            z: 0.0,
            state: 1,
            ..Default::default()
        });
    d.bg_object_events.entry(guid).or_default().push(event);
}

/// The five bases of Arathi Basin, each with a banner for each of its
/// states.
pub(super) fn arathi() -> GameData {
    let mut d = GameData::empty();
    battleground(&mut d, AB, 3, "PVPzone04", [890, 889]);
    for id in [895, 894, 893, 897, 896, 898, 899] {
        d.safe_locs.insert(id, (AB, id as f32, 0.0, 0.0));
    }
    for e in [180087, 180088, 180089, 180090, 180091] {
        banner(&mut d, e, 10);
    }
    for e in [180058, 180059, 180060, 180061] {
        banner(&mut d, e, 1);
    }
    for node in 0..5u8 {
        let at = (50.0 * f32::from(node), 50.0);
        let base = 90000 + u32::from(node) * 10;
        bound_object(&mut d, AB, base, 180087 + u32::from(node), at, (node, 0));
        bound_object(&mut d, AB, base + 1, 180059, at, (node, 1));
        bound_object(&mut d, AB, base + 2, 180061, at, (node, 2));
        bound_object(&mut d, AB, base + 3, 180058, at, (node, 3));
        bound_object(&mut d, AB, base + 4, 180060, at, (node, 4));
    }
    d
}

pub(super) async fn queue(c: &Client, map: Map) {
    c.send(CMSG_BATTLEMASTER_JOIN {
        guid: Guid::zero(),
        map,
        instance_id: 0,
        join_as_group: false,
    })
    .await;
}

/// Alice and Grom queue, and go in.
pub(super) async fn both_into(
    realm: &std::sync::Arc<crate::game::Realm>,
    a: &mut Client,
    b: &mut Client,
    map: Map,
) -> MapHandle {
    queue(a, map).await;
    queue(b, map).await;
    let handle = into(realm, a, map).await;
    into(realm, b, map).await;
    // The battle begins at the next update.
    for _ in 0..200 {
        let playing = handle
            .query(|m| m.battleground.as_ref().is_some_and(|b| b.playing()))
            .await
            .unwrap_or(false);
        if playing {
            return handle;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the battle never began");
}

pub(super) async fn into(
    realm: &std::sync::Arc<crate::game::Realm>,
    c: &mut Client,
    map: Map,
) -> MapHandle {
    c.wait_frame(
        "the invitation",
        crate::game::battlegrounds::SMSG_BATTLEFIELD_STATUS,
        |f| f.len() >= 17 && f[13] == 2,
    )
    .await;
    c.send(CMSG_BATTLEFIELD_PORT {
        map,
        action: BattlefieldPortAction::EnterBattle,
    })
    .await;
    c.wait_for("SMSG_NEW_WORLD", |m| match m {
        ServerOpcodeMessage::SMSG_NEW_WORLD(w) => (w.map == map).then_some(()),
        _ => None,
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    let g = c.guid;
    c.sees(g).await;
    let key = realm.registry.online(g).unwrap().map().key;
    realm.maps.get(realm, key)
}

/// Move `c` within the match's map, at once.
pub(super) async fn put(map: &MapHandle, c: &mut Client, x: f32, y: f32) {
    let g = c.guid;
    map.query(move |m| m.teleport(g, m.key, Vector3d { x, y, z: 0.0 }, 0.0))
        .await;
    c.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: c.teleport_order(),
        time: Default::default(),
    })
    .await;
}

/// The match's timers, and its assaults' deadlines, due now.
pub(super) async fn hurry(map: &MapHandle) {
    map.query(|m| {
        if let Some(b) = m.battleground.as_mut() {
            let now = Instant::now();
            for t in b.timers.iter_mut() {
                t.0 = now;
            }
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
    })
    .await;
}

/// The object of a spawn, once it's in the map.
pub(super) async fn object_of(map: &MapHandle, spawn: u32) -> Guid {
    for _ in 0..200 {
        let g = map
            .query(move |m| {
                m.gameobjects
                    .iter()
                    .find(|(_, o)| o.spawn_guid == Some(spawn))
                    .map(|(_, o)| o.guid)
            })
            .await
            .flatten();
        if let Some(g) = g {
            return g;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("spawn {spawn} never came");
}

pub(super) async fn world_state(c: &mut Client, what: &str, state: u32, value: u32) {
    c.wait_for(what, |m| match m {
        ServerOpcodeMessage::SMSG_UPDATE_WORLD_STATE(w) => {
            (w.state.state == state && w.state.value == value).then_some(())
        }
        _ => None,
    })
    .await;
}

pub(super) async fn said(c: &mut Client, what: &str, containing: &str) {
    c.wait_for(what, |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => c.message.contains(containing).then_some(()),
        _ => None,
    })
    .await;
}

pub(super) async fn winner(c: &mut Client) -> BattlegroundWinner {
    c.wait_for("the end", |m| match m {
        ServerOpcodeMessage::MSG_PVP_LOG_DATA(l) => match &l.status {
            MSG_PVP_LOG_DATA_Server_BattlegroundEndStatus::Ended { winner } => Some(*winner),
            _ => None,
        },
        _ => None,
    })
    .await
}

#[tokio::test]
async fn arathi_basin_runs_to_two_thousand_resources() {
    let realm = harness::realm_with(arathi());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::ArathiBasin).await;
    // The level 60 bracket, where Arathi Basin gives honor.
    map.query(|m| m.battleground.as_mut().unwrap().bracket = 5)
        .await;
    for node in 0..5u32 {
        // The neutral banner: claimed, and the base the Alliance's in a
        // minute.
        let flag = object_of(&map, 90000 + node * 10).await;
        put(&map, &mut a, 50.0 * node as f32, 50.0).await;
        a.send(CMSG_GAMEOBJ_USE { guid: flag }).await;
        said(&mut a, "the claim", "claims").await;
        hurry(&map).await;
        world_state(&mut a, "a base held", 1779, node + 1).await;
        // Its banner goes up as the Alliance's.
        hurry(&map).await;
        object_of(&map, 90000 + node * 10 + 3).await;
    }
    // A minute and more of five bases: 30 resources a second.
    map.query(|m| {
        let ab = m.battleground.as_mut().unwrap().ab.as_mut().unwrap();
        ab.last_resources = Some(Instant::now() - Duration::from_secs(70));
    })
    .await;
    world_state(&mut a, "two thousand", 1776, 2000).await;
    assert_eq!(winner(&mut a).await, BattlegroundWinner::Alliance);
    assert_eq!(winner(&mut b).await, BattlegroundWinner::Alliance);
    let alice = a.guid;
    let (assaulted, bonus) = map
        .query(move |m| {
            let s = m.battleground.as_ref().unwrap().scores[&alice];
            (s.objectives[0], s.bonus_honor)
        })
        .await
        .unwrap();
    assert_eq!(assaulted, 5);
    // 198 for each 330 resources (six), and 198 for the win.
    assert_eq!(bonus, 198 * 7);
}

#[tokio::test]
async fn an_arathi_base_is_defended_before_it_falls() {
    let realm = harness::realm_with(arathi());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::ArathiBasin).await;
    let flag = object_of(&map, 90000).await;
    put(&map, &mut a, 0.0, 50.0).await;
    a.send(CMSG_GAMEOBJ_USE { guid: flag }).await;
    said(&mut a, "the claim", "claims").await;
    hurry(&map).await;
    world_state(&mut a, "the Stables held", 1779, 1).await;
    // The Horde assaults the Alliance's banner.
    hurry(&map).await;
    let held = object_of(&map, 90003).await;
    put(&map, &mut b, 0.0, 50.0).await;
    b.send(CMSG_GAMEOBJ_USE { guid: held }).await;
    world_state(&mut a, "the Stables contested", 1779, 0).await;
    // The banner goes up as the Horde's contested one, and Alice takes it
    // back.
    map.query(|m| {
        for t in m.battleground.as_mut().unwrap().timers.iter_mut() {
            if matches!(t.1, crate::game::map::battleground::Due::Banner { .. }) {
                t.0 = Instant::now();
            }
        }
    })
    .await;
    let contested = object_of(&map, 90002).await;
    a.send(CMSG_GAMEOBJ_USE { guid: contested }).await;
    said(&mut a, "the defense", "defended").await;
    world_state(&mut a, "the Stables held again", 1779, 1).await;
    // The Horde's capture timer, stale now, does nothing.
    hurry(&map).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let state = map
        .query(|m| {
            m.battleground
                .as_ref()
                .unwrap()
                .ab
                .as_ref()
                .unwrap()
                .state(0)
        })
        .await
        .unwrap();
    assert_eq!(state, 3);
}

/// Alterac Valley's generals and one Horde tower.
pub(super) fn alterac() -> GameData {
    let mut d = GameData::empty();
    battleground(&mut d, AV, 1, "PVPZone01", [611, 610]);
    for (entry, name) in [(11946, "Drek'Thar"), (11948, "Vanndar Stormpike")] {
        d.creature_templates.insert(
            entry,
            CreatureTemplate {
                entry,
                name: name.into(),
                display_ids: [1, 0, 0, 0],
                display_scales: [1.0, 0.0, 0.0, 0.0],
                display_probabilities: [100, 0, 0, 0],
                level_min: 60,
                level_max: 60,
                faction: 14,
                health_max: 100,
                rank: 3,
                ..Default::default()
            },
        );
    }
    let mut spawn = |guid: u32, entry: u32, x: f32, event: (u8, u8)| {
        d.creature_spawns
            .entry(AV)
            .or_default()
            .push(CreatureSpawn {
                guid,
                entries: vec![entry],
                map: AV,
                x,
                y: 0.0,
                z: 0.0,
                respawn_min: 600,
                respawn_max: 600,
                ..Default::default()
            });
        d.bg_creature_events.entry(guid).or_default().push(event);
    };
    // Drek'Thar is the Horde's general (event 62), Vanndar the
    // Alliance's (61).
    spawn(150100, 11946, 10.0, (62, 0));
    spawn(150101, 11948, 20.0, (61, 0));
    for e in [178364, 178365] {
        banner(&mut d, e, 1);
    }
    // Iceblood Tower: its banner as the Horde holds it, as the Alliance
    // assaults it, and burnt by them.
    bound_object(&mut d, AV, 160011, 178364, (30.0, 0.0), (11, 3));
    bound_object(&mut d, AV, 160012, 178365, (30.0, 0.0), (11, 0));
    bound_object(&mut d, AV, 160013, 178365, (30.0, 0.0), (11, 1));
    d
}

#[tokio::test]
async fn alterac_valley_ends_when_a_general_falls() {
    let realm = harness::realm_with(alterac());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::AlteracValley).await;
    // Alice burns Iceblood Tower: assaulted, then five minutes on, gone.
    let banner = object_of(&map, 160011).await;
    put(&map, &mut a, 30.0, 0.0).await;
    a.send(CMSG_GAMEOBJ_USE { guid: banner }).await;
    said(&mut a, "the assault", "assaulted Iceblood Tower").await;
    // Iceblood's Alliance-assaulted field is 1368.
    world_state(&mut a, "Iceblood assaulted", 1368, 1).await;
    hurry(&map).await;
    said(&mut a, "the tower burns", "destroyed Iceblood Tower").await;
    let destroyed = map
        .query(|m| m.battleground.as_ref().unwrap().av.as_ref().unwrap().nodes[11].destroyed)
        .await
        .unwrap();
    assert!(destroyed);
    // Drek'Thar falls to Alice.
    let general = map
        .query(|m| {
            m.creatures
                .iter()
                .find(|(_, c)| c.entry == 11946)
                .map(|(_, c)| c.guid)
        })
        .await
        .flatten()
        .expect("Drek'Thar is in the valley");
    let alice = a.guid;
    map.query(move |m| crate::game::map::combat::kill(m, alice, general))
        .await;
    assert_eq!(winner(&mut a).await, BattlegroundWinner::Alliance);
    let (leaders, bonus) = map
        .query(move |m| {
            let s = m.battleground.as_ref().unwrap().scores[&alice];
            (s.objectives[5], s.bonus_honor)
        })
        .await
        .unwrap();
    assert_eq!(leaders, 1);
    assert!(bonus > 0, "the tower's and the general's honor");
}

/// A battlemaster for Arathi Basin, by the start.
fn with_battlemaster() -> GameData {
    let mut d = arathi();
    d.creature_templates.insert(
        857,
        CreatureTemplate {
            entry: 857,
            name: "Donald Osgood".into(),
            display_ids: [1, 0, 0, 0],
            display_scales: [1.0, 0.0, 0.0, 0.0],
            display_probabilities: [100, 0, 0, 0],
            level_min: 60,
            level_max: 60,
            faction: 35,
            npc_flags: crate::game::map::npc::NPC_FLAG_BATTLEMASTER,
            health_max: 100,
            ..Default::default()
        },
    );
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: 77,
        entries: vec![857],
        map: 0,
        x: -8948.0,
        y: -132.5,
        z: 83.5,
        ..Default::default()
    });
    d.battlemasters.insert(857, 3);
    d
}

#[tokio::test]
async fn a_battlemaster_lists_the_matches_and_one_joins_from_the_list() {
    let realm = harness::realm_with(with_battlemaster());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let npc = crate::game::guid::unit(857, 77);
    a.sees(npc).await;
    a.send(CMSG_BATTLEMASTER_HELLO { guid: npc }).await;
    let listed = a
        .wait_for("SMSG_BATTLEFIELD_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_BATTLEFIELD_LIST(l) => Some((l.battlemaster, l.map)),
            _ => None,
        })
        .await;
    assert_eq!(listed, (npc, Map::ArathiBasin));
    a.send(CMSG_BATTLEFIELD_JOIN {
        map: Map::ArathiBasin,
    })
    .await;
    // In the queue: the status frame names Arathi Basin.
    a.wait_frame(
        "queued",
        crate::game::battlegrounds::SMSG_BATTLEFIELD_STATUS,
        |f| f.len() >= 17 && u32::from_le_bytes([f[4], f[5], f[6], f[7]]) == AB,
    )
    .await;
    // The window's list, from anywhere.
    a.send(CMSG_BATTLEFIELD_LIST {
        map: Map::ArathiBasin,
    })
    .await;
    a.wait_for("the window's list", |m| match m {
        ServerOpcodeMessage::SMSG_BATTLEFIELD_LIST(l) => {
            (l.battlemaster == Guid::zero()).then_some(())
        }
        _ => None,
    })
    .await;
}

#[tokio::test]
async fn an_honorable_kill_is_worth_less_each_time_today() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    // Grom is Forsaken: his start is on Alice's continent.
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Undead, Class::Warrior).await;
    let (alice, grom) = (a.guid, b.guid);
    let key = realm.registry.online(alice).unwrap().map().key;
    let map = realm.maps.get(&realm, key);
    let mut worth = Vec::new();
    for _ in 0..2 {
        map.query(move |m| {
            crate::game::map::death::resurrect(m, grom, 1.0);
            crate::game::map::combat::kill(m, alice, grom)
        })
        .await;
        let credit = a
            .wait_frame(
                "SMSG_PVP_CREDIT",
                crate::game::map::pvp::SMSG_PVP_CREDIT,
                |_| true,
            )
            .await;
        worth.push(u32::from_le_bytes([
            credit[0], credit[1], credit[2], credit[3],
        ]));
    }
    // Level 1s: 0.1212 of 188.3, then a tenth less.
    assert_eq!(worth, vec![22, 20]);
    let (today, lifetime) = map
        .query(move |m| {
            let f = &m.players[&alice].fields;
            (
                f.get_u32(PLAYER_FIELD_SESSION_KILLS) & 0xFFFF,
                f.get_u32(PLAYER_FIELD_LIFETIME_HONORABLE_KILLS),
            )
        })
        .await
        .unwrap();
    assert_eq!((today, lifetime), (2, 2));
}

#[tokio::test]
async fn pvp_toggles_on_and_lingers_off() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let key = realm.registry.online(g).unwrap().map().key;
    let map = realm.maps.get(&realm, key);
    let flagged = |map: MapHandle| async move {
        map.query(move |m| {
            let f = &m.players[&g].fields;
            (
                f.has_flag(UNIT_FIELD_FLAGS, crate::game::map::player::UNIT_FLAG_PVP),
                f.has_flag(
                    PLAYER_FLAGS,
                    crate::game::map::pvp::PLAYER_FLAGS_PVP_DESIRED,
                ),
            )
        })
        .await
        .unwrap()
    };
    a.send(CMSG_TOGGLE_PVP { set: None }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(flagged(map.clone()).await, (true, true));
    a.send(CMSG_TOGGLE_PVP { set: None }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Off, but the flag stays its five minutes.
    assert_eq!(flagged(map.clone()).await, (true, false));
    map.query(move |m| {
        m.players.get_mut(&g).unwrap().pvp.until = Some(Instant::now());
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(flagged(map).await, (false, false));
}

#[test]
fn a_week_of_kills_ranks_by_standing() {
    use crate::game::honor::*;
    let mut l = Ledger::new(100, 3);
    let start = l.week_start;
    l.register(1, 0, 60);
    l.register(2, 0, 60);
    l.register(3, 1, 60);
    // Alice kills fifteen Horde players for 1,000 honor; Bob only five.
    for i in 0..15 {
        l.award(
            1,
            Award {
                kind: AwardKind::Honorable,
                points: 1000.0 / 15.0,
                victim: Some(1000 + i),
                victim_rank: 0,
            },
        );
    }
    for i in 0..5 {
        l.award(
            2,
            Award {
                kind: AwardKind::Honorable,
                points: 100.0,
                victim: Some(2000 + i),
                victim_rank: 0,
            },
        );
    }
    assert!(l.advance(start + 7), "the week turns");
    let alice = &l.entries[&1];
    // Alone in her side's ranking: the 3,000 the fourth bracket earns.
    assert_eq!(alice.last_standing, 1);
    assert!(
        (alice.rank_points - 3000.0).abs() < 1.0,
        "{}",
        alice.rank_points
    );
    assert_eq!(rank_number(alice.rank_points), 6);
    assert_eq!(l.project(1).last_week_kills, 15);
    // Too few kills to be ranked: Bob's points only decay.
    assert_eq!(l.entries[&2].last_standing, 0);
    // A week with nothing: Alice decays.
    l.advance(start + 14);
    assert!((l.entries[&1].rank_points - 2700.0).abs() < 1.0);
    // Kept and loaded again.
    let records: Vec<(String, u64, String)> = l
        .entries
        .values()
        .map(|e| {
            (
                KIND.to_owned(),
                u64::from(e.guid),
                serde_json::to_string(e).unwrap(),
            )
        })
        .collect();
    let again = Ledger::load(&records, start + 14, 3);
    assert_eq!(again.entries[&1], l.entries[&1]);
}

/// A spell that only marks its target with a dummy aura.
pub(super) fn marker(d: &mut GameData, id: u32) {
    let mut s = crate::game::data::spells::SpellTemplate {
        id,
        name: format!("Marker {id}"),
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0].effect = 6;
    s.effects[0].aura = crate::game::map::auras::AURA_DUMMY;
    s.effects[0].target_a = 1;
    d.spells.spells.insert(id, s);
}

#[tokio::test]
async fn silithyst_carried_home_counts_for_the_side() {
    let mut d = GameData::empty();
    banner(&mut d, 181598, 10);
    marker(&mut d, crate::game::map::outdoor_pvp::SPELL_CARRYING);
    marker(&mut d, 29894);
    let realm = harness::realm_with(d);
    // Alice is a night elf: Kalimdor.
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::NightElf, Class::Warrior).await;
    let key = realm.registry.online(g).unwrap().map().key;
    assert_eq!(key.map, 1);
    let map = realm.maps.get(&realm, key);
    let pos = map.query(move |m| m.players[&g].position()).await.unwrap();
    let geyser = map
        .query(move |m| {
            let t = m.realm.data.gameobject_templates[&181598].clone();
            let mut go = crate::game::map::gameobject::GameObject::from_spawn(
                &t,
                &GameObjectSpawn {
                    guid: 49336,
                    entry: 181598,
                    map: 1,
                    x: pos.x + 1.0,
                    y: pos.y,
                    z: pos.z,
                    state: 1,
                    ..Default::default()
                },
            );
            go.spawn_guid = Some(49336);
            m.add_gameobject(go)
        })
        .await
        .unwrap();
    a.send(CMSG_GAMEOBJ_USE { guid: geyser }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let carrying = map
        .query(move |m| {
            m.players[&g]
                .auras
                .has_spell(crate::game::map::outdoor_pvp::SPELL_CARRYING)
        })
        .await
        .unwrap();
    assert!(carrying, "silithyst picked up");
    // Home at the Alliance's camp (trigger 4162), in Silithus.
    map.query(move |m| {
        m.players.get_mut(&g).unwrap().zone = crate::game::map::outdoor_pvp::SILITHUS;
        crate::game::map::outdoor_pvp::area_trigger(m, g, 4162);
    })
    .await;
    world_state(&mut a, "one brought home", 2313, 1).await;
    let still = map
        .query(move |m| {
            m.players[&g]
                .auras
                .has_spell(crate::game::map::outdoor_pvp::SPELL_CARRYING)
        })
        .await
        .unwrap();
    assert!(!still, "handed in");
}

#[tokio::test]
async fn a_plaguelands_tower_falls_to_those_standing_at_it() {
    let mut d = GameData::empty();
    let mut t = GameObjectTemplate {
        entry: 182096,
        kind: 29,
        display_id: 1,
        size: 1.0,
        name: "Crown Guard".into(),
        ..Default::default()
    };
    for (i, v) in [
        (0, 80),
        (2, 2426),
        (3, 2427),
        (12, 20),
        (13, 2428),
        (16, 480),
        (17, 1200),
    ] {
        t.data[i] = v;
    }
    d.gameobject_templates.insert(182096, t);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let key = realm.registry.online(g).unwrap().map().key;
    assert_eq!(key.map, 0);
    let map = realm.maps.get(&realm, key);
    a.send(CMSG_TOGGLE_PVP {
        set: Some(CMSG_TOGGLE_PVP_set { enable_pvp: true }),
    })
    .await;
    // Alice at Crown Guard Tower, in the Eastern Plaguelands.
    map.query(move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.movement.position = Vector3d {
            x: 1860.0,
            y: -3730.0,
            z: 196.7,
        };
        p.zone = crate::game::map::outdoor_pvp::PLAGUELANDS;
    })
    .await;
    // The tower goes up, and Alice watches its slider.
    a.wait_for("the slider", |m| match m {
        ServerOpcodeMessage::SMSG_UPDATE_WORLD_STATE(w) => {
            (w.state.state == 2426 && w.state.value == 1).then_some(())
        }
        _ => None,
    })
    .await;
    // Ten minutes alone at the tower: the Alliance's.
    map.query(|m| {
        let o = m.outdoor.as_mut().unwrap();
        o.last_tick = Some(Instant::now() - Duration::from_secs(600));
        o.next_tick = Some(Instant::now());
    })
    .await;
    world_state(&mut a, "a tower held", 2327, 1).await;
}

/// With vmangos's data, each battleground starts with the spawns of its
/// first events only: Arathi Basin's five neutral banners, every Alterac
/// Valley node's banner in its first state, its generals but not their
/// captains' graves.
#[test]
#[ignore = "extended"]
fn the_real_battlegrounds_start_with_their_first_events() {
    use crate::game::map::battleground::{ab, av, initial_events};
    let dir = &crate::game::harness::real_data("db");
    if !dir.join("vmangos.sqlite").exists() {
        return;
    }
    let data = crate::game::harness::load_real(dir);
    let on = |bg: u32, bindings: Option<&Vec<(u8, u8)>>| {
        let ev = initial_events(bg);
        bindings.is_none_or(|b| b.iter().any(|(e, s)| ev.get(e) == Some(&Some(*s))))
    };
    let mut ab_banners: Vec<u32> = data.gameobject_spawns[&529]
        .iter()
        .filter(|s| ab::BANNERS.contains(&s.entry) && on(3, data.bg_object_events.get(&s.guid)))
        .map(|s| s.entry)
        .collect();
    ab_banners.sort_unstable();
    assert_eq!(ab_banners, vec![180087, 180088, 180089, 180090, 180091]);
    let av_banners = data.gameobject_spawns[&30]
        .iter()
        .filter(|s| av::BANNERS.contains(&s.entry) && on(1, data.bg_object_events.get(&s.guid)))
        .count();
    assert_eq!(av_banners, 15, "one banner a node");
    let bound_to = |events: &[u8]| {
        data.creature_spawns[&30]
            .iter()
            .filter(|s| {
                data.bg_creature_events
                    .get(&s.guid)
                    .is_some_and(|b| b.iter().any(|(e, _)| events.contains(e)))
            })
            .map(|s| on(1, data.bg_creature_events.get(&s.guid)))
            .collect::<Vec<bool>>()
    };
    let generals = bound_to(&[61, 62]);
    assert!(!generals.is_empty() && generals.iter().all(|b| *b));
    let captain_dead = bound_to(&[63, 64]);
    assert!(captain_dead.iter().all(|b| !*b));
}

#[tokio::test]
async fn the_battlefield_status_tells_a_queued_player_where_they_stand() {
    let realm = harness::realm_with(arathi());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    // Nothing yet.
    a.send(CMSG_BATTLEFIELD_STATUS {}).await;
    a.wait_frame(
        "no status",
        crate::game::battlegrounds::SMSG_BATTLEFIELD_STATUS,
        |f| f.len() == 8 && f[4..8] == [0, 0, 0, 0],
    )
    .await;
    queue(&a, Map::ArathiBasin).await;
    a.send(CMSG_BATTLEFIELD_STATUS {}).await;
    for _ in 0..2 {
        a.wait_frame(
            "queued",
            crate::game::battlegrounds::SMSG_BATTLEFIELD_STATUS,
            |f| f.len() >= 17 && u32::from_le_bytes([f[4], f[5], f[6], f[7]]) == AB && f[13] == 1,
        )
        .await;
    }
}

#[tokio::test]
async fn a_gm_sets_rank_points_and_the_rank_shows_as_a_title() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.say(".honor points 5000").await;
    a.system_message("Rank points set").await;
    let key = realm.registry.online(g).unwrap().map().key;
    let map = realm.maps.get(&realm, key);
    let (rank, highest) = map
        .query(move |m| {
            let f = &m.players[&g].fields;
            (
                f.get_byte(PLAYER_BYTES_3, 3),
                f.get_byte(PLAYER_FIELD_BYTES, 3),
            )
        })
        .await
        .unwrap();
    // 5,000 points: rank 7, Sergeant.
    assert_eq!((rank, highest), (7, 7));
}
