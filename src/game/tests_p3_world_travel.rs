//! Phase 3's flights against Thistle Tea's tests: the flight map's nodes
//! as the client counts them (from one), the race's first flight points,
//! the flight master's node by the side's mount, each side's own mount, a
//! route with a leg missing, a warrior's stance in the air, and a
//! teleport ending a flight.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::npc::{TaxiNode, TaxiPathNode};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{MapHandle, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const MASTER: u32 = 90_400;
const MASTER_SPAWN: u32 = 9_400;
const GRYPHON: u32 = 90_401;
const BAT: u32 = 90_402;

fn creature(entry: u32, name: &str, npc_flags: u32, display: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [display, 0, 0, 0],
        display_scales: [1.0, 0.0, 0.0, 0.0],
        display_probabilities: [100, 0, 0, 0],
        level_min: 60,
        level_max: 60,
        faction: 35,
        npc_flags,
        health_max: 1000,
        speed_walk: 1.0,
        speed_run: 1.0,
        ..Default::default()
    }
}

/// A flight master by the human start, at node 2; nodes 3 and 4 on, with
/// paths 2 to 3 and 3 to 4 but not 4 to 5; node 5 flown from only by a
/// spell; node 6 by the master too, but only the Horde's (a bat) and
/// nearer, node 2 the Alliance's (a gryphon).
fn world() -> GameData {
    let mut d = GameData::empty();
    d.creature_templates
        .insert(MASTER, creature(MASTER, "Flight Master", 0x8, 1));
    d.creature_templates
        .insert(GRYPHON, creature(GRYPHON, "Riding Gryphon", 0, 1149));
    d.creature_templates
        .insert(BAT, creature(BAT, "Riding Bat", 0, 1566));
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: MASTER_SPAWN,
        entries: vec![MASTER],
        map: 0,
        x: START.0 + 2.0,
        y: START.1,
        z: START.2,
        ..Default::default()
    });
    for (id, dx, mounts) in [
        (2u32, 4.0f32, [GRYPHON, 0]),
        (3, 64.0, [GRYPHON, BAT]),
        (4, 124.0, [GRYPHON, BAT]),
        (5, 184.0, [GRYPHON, BAT]),
        (6, 2.5, [0, BAT]),
    ] {
        d.npc.taxi_nodes.insert(
            id,
            TaxiNode {
                id,
                map: 0,
                x: START.0 + dx,
                y: START.1,
                z: START.2,
                name: format!("Node {id}"),
                mounts,
            },
        );
    }
    for (from, to, path, cost) in [(2u32, 3u32, 9u32, 5u32), (3, 4, 10, 7), (5, 2, 11, 0)] {
        d.npc.taxi_paths.insert((from, to), (path, cost));
        let (x0, x1) = (d.npc.taxi_nodes[&from].x, d.npc.taxi_nodes[&to].x);
        d.npc.taxi_path_nodes.insert(
            path,
            (0..4)
                .map(|i| TaxiPathNode {
                    map: 0,
                    x: x0 + (x1 - x0) * i as f32 / 3.0,
                    y: START.1,
                    z: START.2 + 10.0,
                    ..Default::default()
                })
                .collect(),
        );
    }
    d.npc.spell_only_nodes.insert(5);
    d.races.insert(
        1,
        RaceInfo {
            male_display: 49,
            female_display: 50,
            faction: 1,
            cinematic: 0,
            starting_taxi_mask: 0b10,
        },
    );
    d
}

fn map(realm: &Arc<Realm>) -> MapHandle {
    realm.maps.get(realm, MapKey::world(0))
}

fn master() -> Guid {
    guid::unit(MASTER, MASTER_SPAWN)
}

async fn put(map: &MapHandle, c: &mut Client, (x, y, z): (f32, f32, f32)) {
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

async fn show(c: &mut Client) -> (u32, Vec<u32>) {
    c.send(CMSG_TAXIQUERYAVAILABLENODES { guid: master() })
        .await;
    c.wait_for("SMSG_SHOWTAXINODES", |m| match m {
        ServerOpcodeMessage::SMSG_SHOWTAXINODES(s) => Some((s.nearest_node, s.nodes.clone())),
        _ => None,
    })
    .await
}

#[tokio::test]
async fn the_flight_map_counts_nodes_from_one_and_shows_no_spells_node() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(master()).await;
    // Her race's first flight point is node 2: bit one, as the client
    // counts.
    let known = map(&realm)
        .query(move |m| m.players[&g].row.state.taxi_mask.clone())
        .await
        .unwrap();
    assert_eq!(known, vec![1 << 2], "wvs keeps node 2 at bit 2");
    // Node 3 and the spells' node 5 learned too: the map shows nodes 2 and
    // 3 at bits one and two, and not node 5.
    map(&realm)
        .query(move |m| {
            m.players.get_mut(&g).unwrap().row.state.taxi_mask[0] |= (1 << 3) | (1 << 5)
        })
        .await;
    let (nearest, nodes) = show(&mut a).await;
    assert_eq!(nearest, 2);

    assert_eq!(nodes[0], 0b110);
    // The status query says she knows the master's node.
    a.send(CMSG_TAXINODE_STATUS_QUERY { guid: master() }).await;
    let known = a
        .wait_for("SMSG_TAXINODE_STATUS", |m| match m {
            ServerOpcodeMessage::SMSG_TAXINODE_STATUS(s) => Some(s.taxi_mask_node_known),
            _ => None,
        })
        .await;
    assert!(known);
}

#[test]
fn masks_go_between_the_clients_count_and_wvss() {
    use crate::game::map::npc::{client_mask, mask_from_client};
    let d = GameData::empty();
    // Nodes 1, 32, 33 and 256 known.
    let ours = vec![(1 << 1), 1 | (1 << 1), 0, 0, 0, 0, 0, 0, 1];
    let client = client_mask(&d, &ours);
    assert_eq!(client, vec![0x8000_0001, 1, 0, 0, 0, 0, 0, 0x8000_0000]);
    assert_eq!(mask_from_client(&client), ours);
    // A race's starting mask: bits one and three are nodes 2 and 4.
    assert_eq!(mask_from_client(&[0b1010]), vec![(1 << 2) | (1 << 4)]);
}

#[test]
#[ignore = "extended"]
fn the_real_races_know_their_capitals_and_each_side_flies_its_own_mount() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    use crate::game::map::npc::mask_from_client;
    let knows = |race: u8, node: u32| {
        let m = mask_from_client(&[d.races[&race].starting_taxi_mask]);
        m.get((node / 32) as usize)
            .is_some_and(|w| w & (1 << (node % 32)) != 0)
    };
    // Humans Stormwind, orcs and trolls Orgrimmar, night elves Auberdine
    // and Rut'theran, the Forsaken Undercity, tauren Thunder Bluff,
    // dwarves and gnomes Ironforge.
    for (race, node) in [
        (1, 2),
        (2, 23),
        (8, 23),
        (4, 26),
        (4, 27),
        (5, 11),
        (6, 22),
        (3, 6),
        (7, 6),
    ] {
        assert!(knows(race, node), "race {race} knows node {node}");
    }
    assert!(!knows(1, 3));
    // Mounts: the Alliance's first, the Horde's second.
    let mounts = |node: u32| d.npc.taxi_nodes[&node].mounts;
    assert_eq!(mounts(2), [541, 0], "Stormwind's gryphon");
    assert_eq!(mounts(23)[1], 2224, "Orgrimmar's wind rider");
    assert_eq!(mounts(11)[1], 3574, "Undercity's bat");
    assert_eq!(mounts(27)[0], 3837, "Rut'theran's hippogryph");
    // Some nodes are only spells', none of them a capital's.
    assert!(!d.npc.spell_only_nodes.is_empty());
    for capital in [2, 6, 11, 22, 23, 26, 27] {
        assert!(!d.npc.spell_only_nodes.contains(&capital), "{capital}");
    }
}

#[tokio::test]
async fn the_flight_master_is_at_the_nearest_node_with_a_mount_for_ones_side() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Wynn", Race::Undead, Class::Warrior).await;
    a.sees(master()).await;
    put(&map(&realm), &mut b, (START.0, START.1 + 1.0, START.2)).await;
    b.sees(master()).await;
    // Node 6 is nearer, but only the Horde's.
    assert_eq!(show(&mut a).await.0, 2);
    assert_eq!(show(&mut b).await.0, 6);
}

async fn fly(c: &mut Client, nodes: Vec<u32>) -> ActivateTaxiReply {
    c.send(CMSG_ACTIVATETAXIEXPRESS {
        guid: master(),
        total_cost: 0,
        nodes,
    })
    .await;
    c.wait_for("SMSG_ACTIVATETAXIREPLY", |m| match m {
        ServerOpcodeMessage::SMSG_ACTIVATETAXIREPLY(r) => Some(r.reply),
        _ => None,
    })
    .await
}

#[tokio::test]
async fn a_route_with_a_leg_missing_is_no_path_and_a_flight_takes_its_sides_mount() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(master()).await;
    map(&realm)
        .query(move |m| {
            let p = m.players.get_mut(&g).unwrap();
            p.row.state.taxi_mask = vec![(1 << 2) | (1 << 3) | (1 << 4) | (1 << 5)];
            p.set_money(100);
        })
        .await;
    // Four to five has no path.
    assert_eq!(
        fly(&mut a, vec![2, 3, 4, 5]).await,
        ActivateTaxiReply::NoSuchPath
    );
    // Two to four flies, on the Alliance's gryphon.
    assert_eq!(fly(&mut a, vec![2, 3, 4]).await, ActivateTaxiReply::Ok);
    let mount = map(&realm)
        .query(move |m| m.players[&g].fields.get_u32(UNIT_FIELD_MOUNTDISPLAYID))
        .await
        .unwrap();
    assert_eq!(mount, 1149);
}

#[tokio::test]
async fn a_warriors_stance_holds_in_the_air_and_landing_frees_the_flier() {
    let mut d = world();
    super::tests_pvp::marker(&mut d, 2457);
    let s = d.spells.spells.get_mut(&2457).unwrap();
    s.effects[0].aura = crate::game::map::auras::AURA_MOD_SHAPESHIFT;
    s.effects[0].misc_value = 17;
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(master()).await;
    map(&realm)
        .query(move |m| {
            crate::game::map::spells::cast_triggered(m, g, 2457, Some(g));
            let p = m.players.get_mut(&g).unwrap();
            p.row.state.taxi_mask = vec![(1 << 2) | (1 << 3)];
            p.set_money(100);
        })
        .await;
    assert_eq!(fly(&mut a, vec![2, 3]).await, ActivateTaxiReply::Ok);
    let flags = UNIT_FLAG_TAXI | UNIT_FLAG_STILL;
    let (stance, flying_flags) = map(&realm)
        .query(move |m| {
            let p = &m.players[&g];
            (
                crate::game::map::auras::form(m, g),
                p.fields.get_u32(UNIT_FIELD_FLAGS) & flags,
            )
        })
        .await
        .unwrap();
    assert_eq!(
        (stance, flying_flags),
        (17, flags),
        "in Battle Stance, flying"
    );
    // Landed: hers to move, still in her stance.
    map(&realm)
        .query(move |m| {
            let f = m.players.get_mut(&g).unwrap().flight.as_mut().unwrap();
            f.walk.started = Instant::now() - Duration::from_secs(600);
        })
        .await;
    let mut landed = None;
    for _ in 0..100 {
        landed = map(&realm)
            .query(move |m| {
                let p = &m.players[&g];
                p.flight.is_none().then(|| {
                    (
                        crate::game::map::auras::form(m, g),
                        p.fields.get_u32(UNIT_FIELD_FLAGS) & flags,
                        p.fields.get_u32(UNIT_FIELD_MOUNTDISPLAYID),
                    )
                })
            })
            .await
            .flatten();
        if landed.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(landed, Some((17, 0, 0)));
}

/// The unit flags a flight sets: on a taxi, and not to be moved.
const UNIT_FLAG_TAXI: u32 = 0x0010_0000;
const UNIT_FLAG_STILL: u32 = 0x0000_0004;

#[tokio::test]
async fn a_teleport_mid_flight_ends_it_where_the_flier_is() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(master()).await;
    map(&realm)
        .query(move |m| {
            let p = m.players.get_mut(&g).unwrap();
            p.row.state.taxi_mask = vec![(1 << 2) | (1 << 3) | (1 << 4)];
            p.set_money(100);
        })
        .await;
    assert_eq!(fly(&mut a, vec![2, 3, 4]).await, ActivateTaxiReply::Ok);
    let away = (START.0 - 50.0, START.1, START.2);
    put(&map(&realm), &mut a, away).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (flying, mount, flags, at) = map(&realm)
        .query(move |m| {
            let p = &m.players[&g];
            (
                p.flight.is_some(),
                p.fields.get_u32(UNIT_FIELD_MOUNTDISPLAYID),
                p.fields.get_u32(UNIT_FIELD_FLAGS) & (UNIT_FLAG_TAXI | UNIT_FLAG_STILL),
                p.position(),
            )
        })
        .await
        .unwrap();
    assert_eq!((flying, mount, flags), (false, 0, 0));
    assert!(
        (at.x - away.0).abs() < 0.5,
        "stays where the teleport put her: {at:?}"
    );
}

#[tokio::test]
async fn a_flight_flown_by_logout_is_a_landing_and_a_dead_passengers_flight_isnt_taken_up() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(master()).await;
    map(&realm)
        .query(move |m| {
            let p = m.players.get_mut(&g).unwrap();
            p.row.state.taxi_mask = vec![(1 << 2) | (1 << 3)];
            p.set_money(100);
        })
        .await;
    assert_eq!(fly(&mut a, vec![2, 3]).await, ActivateTaxiReply::Ok);
    // Its time up, saved before the map lands her: saved at its end, no
    // flight to take up.
    let (row_x, saved, end) = map(&realm)
        .query(move |m| {
            let now = Instant::now();
            let p = m.players.get_mut(&g).unwrap();
            let f = p.flight.as_mut().unwrap();
            f.walk.started = now - Duration::from_secs(600);
            let end = f.walk.destination();
            let row = p.to_row(now);
            (row.x, row.state.flight.is_some(), end)
        })
        .await
        .unwrap();
    assert!((row_x - end.x).abs() < 0.01 && !saved, "{row_x} {end:?}");
    // A flight saved under way, but the passenger dead: not taken up.
    map(&realm)
        .query(move |m| {
            let p = m.players.get_mut(&g).unwrap();
            p.flight = None;
            p.row.state.flight = Some(crate::game::store::SavedFlight {
                map: 0,
                points: vec![(START.0 + 30.0, START.1, START.2 + 10.0)],
                destination: 3,
                mount: 1149,
            });
            crate::game::map::death::kill(m, g);
            crate::game::map::npc::resume_flight(m, g);
        })
        .await;
    let flying = map(&realm)
        .query(move |m| m.players[&g].flight.is_some())
        .await
        .unwrap();
    assert!(!flying);
}

/// A flier sees 400 yards (`[visibility] flight_yards`), measured on the
/// ground: Alice, flying from node 2 to node 3, is told of Bob 350 yards
/// off her path and 300 above it, as Carol, walking where she took off,
/// isn't. Landed, she sees as far as anyone again, and forgets him.
#[tokio::test]
async fn a_flier_sees_four_hundred_yards_and_landing_forgets_what_is_further() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    c.enter("Carol", Race::Human, Class::Warrior).await;
    a.sees(master()).await;
    a.sees(gb).await;
    c.sees(gb).await;
    let off = (START.0 + 34.0, START.1 + 350.0, START.2 + 300.0);
    put(&map(&realm), &mut b, off).await;
    a.forgets(gb).await;
    c.forgets(gb).await;
    map(&realm)
        .query(move |m| {
            let p = m.players.get_mut(&ga).unwrap();
            p.row.state.taxi_mask = vec![(1 << 2) | (1 << 3)];
            p.set_money(100);
        })
        .await;
    assert_eq!(fly(&mut a, vec![2, 3]).await, ActivateTaxiReply::Ok);
    a.sees(gb).await;
    assert!(
        !c.sees_within(gb, Duration::from_millis(300)).await,
        "Carol, walking, sees 100 yards"
    );
    // Landed at node 3, 351 yards from him.
    a.forgets(gb).await;
    let landed = map(&realm)
        .query(move |m| m.players[&ga].flight.is_none())
        .await
        .unwrap();
    assert!(landed);
}

#[test]
fn a_ship_and_a_zeppelin_are_open_to_the_sky_only_at_their_deck_spots() {
    use crate::game::map::spells::environment::on_open_deck;
    let v = |x, y, z| Vector3d { x, y, z };
    for at in [
        v(6.21, 0.12, 14.05),
        v(-10.46, 6.62, 17.77),
        v(1.55, -4.51, 11.30),
    ] {
        assert!(on_open_deck(3015, at), "{at:?}");
    }
    assert!(on_open_deck(3031, v(-21.77, -7.90, -13.27)));
    assert!(!on_open_deck(3015, v(6.21, 0.12, 12.0)), "below deck");
    assert!(!on_open_deck(3031, v(-21.77, -7.90, -15.0)));
    assert!(!on_open_deck(3015, v(0.0, 0.0, 0.0)));
    assert!(
        !on_open_deck(999, v(6.21, 0.12, 14.05)),
        "no such transport"
    );
}

#[test]
#[ignore = "extended"]
fn the_real_taxi_tables_have_stormwinds_node_the_spells_paths_and_the_bank_slots_prices() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    // Stormwind's node: where it is, and the Alliance's gryphon's look.
    let n = &d.npc.taxi_nodes[&2];
    assert_eq!(n.name, "Stormwind, Elwynn");
    assert!(
        (n.x + 8840.56).abs() < 0.01 && (n.y - 489.7).abs() < 0.01 && (n.z - 109.61).abs() < 0.01
    );
    let gryphon = d.creature_template(n.mounts[0]).unwrap();
    assert_eq!(
        gryphon.display_ids.iter().copied().find(|x| *x != 0),
        Some(6852)
    );
    // The paths spells send players along (SEND_TAXI).
    let mut spell_paths: Vec<u32> = d
        .spells
        .spells
        .values()
        .flat_map(|s| s.effects.iter())
        .filter(|e| e.effect == 123 && e.misc_value > 0)
        .map(|e| e.misc_value as u32)
        .collect();
    spell_paths.sort_unstable();
    spell_paths.dedup();
    assert_eq!(spell_paths, vec![315, 316, 472, 494, 495, 496]);
    // The bank's six bag slots' prices (the table's seventh a sentinel).
    let prices: Vec<Option<u32>> = (1..=6)
        .map(|i| d.npc.bank_slot_prices.get(&i).copied())
        .collect();
    assert_eq!(
        prices,
        vec![
            Some(1_000),
            Some(10_000),
            Some(100_000),
            Some(250_000),
            Some(500_000),
            Some(1_000_000)
        ]
    );
}

#[tokio::test]
async fn the_bank_sells_six_bag_slots_and_no_seventh() {
    let mut d = world();
    d.creature_templates
        .insert(90_410, creature(90_410, "Banker", 0x100, 1));
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: 9_410,
        entries: vec![90_410],
        map: 0,
        x: START.0 - 2.0,
        y: START.1,
        z: START.2,
        ..Default::default()
    });
    for (slot, price) in [(1, 1_000), (6, 1_000_000), (7, 999_999_999)] {
        d.npc.bank_slot_prices.insert(slot, price);
    }
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let banker = guid::unit(90_410, 9_410);
    a.sees(banker).await;
    map(&realm)
        .query(move |m| {
            let p = m.players.get_mut(&g).unwrap();
            p.fields.set_byte(PLAYER_BYTES_2, 2, 6);
            p.set_money(2_000_000_000);
        })
        .await;
    a.send(CMSG_BUY_BANK_SLOT { guid: banker }).await;
    let result = a
        .wait_for("SMSG_BUY_BANK_SLOT_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_BUY_BANK_SLOT_RESULT(r) => Some(r.result),
            _ => None,
        })
        .await;
    assert_eq!(result, BuyBankSlotResult::FailedTooMany);
    let (slots, money) = map(&realm)
        .query(move |m| {
            let p = &m.players[&g];
            (p.fields.get_byte(PLAYER_BYTES_2, 2), p.money())
        })
        .await
        .unwrap();
    assert_eq!((slots, money), (6, 2_000_000_000));
}
