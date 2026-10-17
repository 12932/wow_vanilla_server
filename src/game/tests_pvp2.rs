//! Battleground reputation, Alterac Valley's quest-driven extras, the
//! Plaguelands' tower rewards, the contested flag and rank rewards,
//! through the protocol.

use std::time::{Duration, Instant};

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_pvp::{alterac, both_into, said};
use crate::game::data::misc::FactionDef;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;

const AV: u32 = 30;

pub(super) fn faction(d: &mut GameData, id: u32, index: u32) {
    d.misc.factions.insert(
        id,
        FactionDef {
            id,
            index,
            name: format!("Faction {id}"),
            ..Default::default()
        },
    );
    d.misc.faction_by_index.insert(index, id);
}

pub(super) fn npc(d: &mut GameData, entry: u32, name: &str, faction: u32) {
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
            faction,
            npc_flags: 1,
            health_max: 100,
            ..Default::default()
        },
    );
}

pub(super) fn spawn(d: &mut GameData, guid: u32, entry: u32, x: f32) {
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
            ..Default::default()
        });
}

/// Alterac Valley with its two factions, a smith, a summoner and a wing
/// commander by the Alliance's start, and the routes they walk.
pub(super) fn valley() -> GameData {
    let mut d = alterac();
    faction(&mut d, 730, 10);
    faction(&mut d, 729, 11);
    npc(&mut d, 13257, "Murgot Deepforge", 35);
    npc(&mut d, 13442, "Arch Druid Renferal", 35);
    npc(&mut d, 13419, "Ivus the Forest Lord", 35);
    npc(&mut d, 13437, "Wing Commander Ichman", 35);
    npc(&mut d, 14947, "Ichman's Gryphon", 35);
    spawn(&mut d, 170001, 13257, 1.0);
    spawn(&mut d, 170002, 13442, 2.0);
    spawn(&mut d, 170003, 13437, 3.0);
    // Renferal reaches her altar at her fiftieth point; Ichman is home at
    // his ninety-third.
    d.bg_routes.insert(13442, line(50, 0.1));
    d.bg_routes.insert(13419, line(25, 0.5));
    d.bg_routes.insert(13437, line(93, 0.05));
    d.gameobject_templates.insert(
        178670,
        GameObjectTemplate {
            entry: 178670,
            kind: 18,
            display_id: 1,
            size: 1.0,
            name: "Circle of Calling".into(),
            data: {
                let mut data = [0; 24];
                data[0] = 1;
                data[1] = 21648;
                data[2] = 11206;
                data
            },
            ..Default::default()
        },
    );
    d
}

/// A route of `n` points `step` apart along the x axis.
pub(super) fn line(n: usize, step: f32) -> Vec<(f32, f32, f32)> {
    (0..n).map(|i| (i as f32 * step, 0.0, 0.0)).collect()
}

/// The valley's routes walked, and its scripts' waits over, at once.
pub(super) async fn arrive(map: &MapHandle) {
    map.query(|m| {
        crate::game::map::battleground::av_scripts::hurry(m);
        let av = m.battleground.as_mut().unwrap().av.as_mut().unwrap();
        av.extras.hurry(Instant::now());
    })
    .await;
    tokio::time::sleep(Duration::from_millis(120)).await;
}

pub(super) async fn turn_in(map: &MapHandle, g: Guid, quest: u32, times: u32) {
    map.query(move |m| {
        for _ in 0..times {
            crate::game::map::battleground::quest_rewarded(m, g, quest);
        }
    })
    .await;
}

pub(super) fn creature(m: &crate::game::map::Map, entry: u32) -> Option<Guid> {
    m.creatures
        .iter()
        .find(|(_, c)| c.entry == entry)
        .map(|(_, c)| c.guid)
}

pub(super) async fn options(c: &mut Client, npc: Guid) -> usize {
    c.send(CMSG_GOSSIP_HELLO { guid: npc }).await;
    c.wait_for("the menu", |m| match m {
        ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) if g.guid == npc => Some(g.gossips.len()),
        _ => None,
    })
    .await
}

pub(super) async fn choose(c: &Client, npc: Guid, option: u32) {
    c.send(CMSG_GOSSIP_SELECT_OPTION {
        guid: npc,
        gossip_list_id: option,
        unknown: None,
    })
    .await;
}

#[tokio::test]
async fn armor_scraps_turned_in_upgrade_the_defenders_with_reputation() {
    let realm = harness::realm_with(valley());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::AlteracValley).await;
    // Twenty-five turn-ins of scraps: 500, the first upgrade's, and a
    // point of Stormpike reputation each.
    turn_in(&map, g, 7223, 25).await;
    let (scraps, standing) = map
        .query(move |m| {
            let av = m.battleground.as_ref().unwrap().av.as_ref().unwrap();
            let p = &m.players[&g];
            (
                av.extras.armor[0].scraps,
                crate::game::map::reputation::standing(p, &m.realm.data, 730),
            )
        })
        .await
        .unwrap();
    assert_eq!((scraps, standing), (500, 25));
    let smith = map.query(|m| creature(m, 13257)).await.flatten().unwrap();
    // Not yet honored: only the status.
    assert_eq!(options(&mut a, smith).await, 1);
    map.query(move |m| crate::game::map::reputation::modify(m, g, 730, 9000, false))
        .await;
    assert_eq!(options(&mut a, smith).await, 2);
    choose(&a, smith, 1).await;
    said(
        &mut a,
        "the upgrade",
        "Seasoned units are entering the battle!",
    )
    .await;
    let tier = map
        .query(|m| {
            crate::game::map::battleground::av_quests::tier(m.battleground.as_ref().unwrap(), 0)
        })
        .await
        .unwrap();
    assert_eq!(tier, 1);
    // The Alliance's graveyard defenders are of the new tier.
    let defenders = map
        .query(|m| {
            m.battleground
                .as_ref()
                .unwrap()
                .av
                .as_ref()
                .unwrap()
                .defender_events(0)
        })
        .await
        .unwrap();
    assert_eq!(defenders, vec![(15, 1)]);
}

#[tokio::test]
async fn two_hundred_offerings_raise_ivus_the_forest_lord() {
    let realm = harness::realm_with(valley());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::AlteracValley).await;
    turn_in(&map, g, 7386, 39).await;
    let none = map.query(|m| creature(m, 13419)).await.flatten();
    assert!(none.is_none(), "195 isn't enough");
    turn_in(&map, g, 7386, 1).await;
    // The summoner walks to the altar and puts down the circle; Ivus rises
    // when the ritual is done.
    arrive(&map).await;
    let circle = map
        .query(|m| {
            m.gameobjects
                .iter()
                .find(|(_, o)| o.entry == 178670)
                .map(|(_, o)| (o.guid, o.position))
        })
        .await
        .flatten()
        .expect("the Circle of Calling");
    super::tests_pvp::put(&map, &mut a, circle.1.x, circle.1.y).await;
    a.send(CMSG_GAMEOBJ_USE { guid: circle.0 }).await;
    for _ in 0..100 {
        if map.query(|m| creature(m, 13419)).await.flatten().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let ivus = map.query(|m| creature(m, 13419)).await.flatten();
    assert!(ivus.is_some());
}

#[tokio::test]
async fn a_rescued_wing_commander_launches_an_air_attack() {
    let realm = harness::realm_with(valley());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::AlteracValley).await;
    let ichman = map.query(|m| creature(m, 13437)).await.flatten().unwrap();
    // Talking to him frees him: he runs home.
    a.send(CMSG_GOSSIP_HELLO { guid: ichman }).await;
    a.wait_for("SMSG_GOSSIP_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
    })
    .await;
    arrive(&map).await;
    // Thirty turn-ins of his supplies, and he can launch.
    turn_in(&map, g, 6943, 30).await;
    assert_eq!(options(&mut a, ichman).await, 2);
    choose(&a, ichman, 1).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    arrive(&map).await;
    let rider = map.query(|m| creature(m, 14947)).await.flatten();
    assert!(rider.is_some(), "his gryphon is out");
}

#[tokio::test]
async fn attacking_a_player_contests_the_attacker() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let grom = b.enter("Grom", Race::Undead, Class::Warrior).await;
    let key = realm.registry.online(alice).unwrap().map().key;
    let map = realm.maps.get(&realm, key);
    let contested = map
        .query(move |m| {
            crate::game::map::combat::deal_damage(m, alice, grom, 5);
            m.players[&alice].fields.has_flag(
                PLAYER_FLAGS,
                crate::game::map::pvp::PLAYER_FLAGS_CONTESTED_PVP,
            )
        })
        .await
        .unwrap();
    assert!(contested);
    map.query(move |m| {
        m.players.get_mut(&alice).unwrap().pvp.contested_until = Some(Instant::now());
    })
    .await;
    tokio::time::sleep(Duration::from_millis(120)).await;
    let still = map
        .query(move |m| {
            m.players[&alice].fields.has_flag(
                PLAYER_FLAGS,
                crate::game::map::pvp::PLAYER_FLAGS_CONTESTED_PVP,
            )
        })
        .await
        .unwrap();
    assert!(!still, "thirty seconds on, no longer");
}

#[tokio::test]
async fn rank_rewards_are_worn_at_the_rank_reached() {
    let mut d = GameData::empty();
    d.items.insert(
        16437,
        ItemTemplate {
            entry: 16437,
            class: 4,
            subclass: 1,
            name: "Marshal's Silk Vestments".into(),
            inventory_type: 5,
            required_honor_rank: 7,
            ..Default::default()
        },
    );
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let key = realm.registry.online(g).unwrap().map().key;
    let map = realm.maps.get(&realm, key);
    let check = |map: MapHandle| async move {
        map.query(move |m| {
            let t = m.realm.data.item(16437).unwrap().clone();
            let p = &m.players[&g];
            (
                crate::game::map::inventory::can_equip(p, &t, 4).is_ok(),
                crate::game::map::pvp::rank_allows_buy(p, &t),
            )
        })
        .await
        .unwrap()
    };
    assert_eq!(check(map.clone()).await, (false, false));
    a.say(".honor points 5000").await;
    a.system_message("Rank points set").await;
    assert_eq!(check(map.clone()).await, (true, true));
    // Fallen back a rank, the item is still worn but no longer bought.
    a.say(".honor points 2500").await;
    a.system_message("Rank points set").await;
    assert_eq!(check(map).await, (true, false));
}

#[tokio::test]
async fn crown_guard_tower_held_opens_its_graveyard_and_puts_up_its_banner() {
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
    d.gameobject_templates.insert(
        180421,
        GameObjectTemplate {
            entry: 180421,
            kind: 5,
            display_id: 1,
            size: 1.0,
            name: "Alliance Banner".into(),
            ..Default::default()
        },
    );
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let key = realm.registry.online(g).unwrap().map().key;
    let map = realm.maps.get(&realm, key);
    a.send(CMSG_TOGGLE_PVP {
        set: Some(CMSG_TOGGLE_PVP_set { enable_pvp: true }),
    })
    .await;
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
    let open = |map: MapHandle| async move {
        map.query(|m| {
            (
                crate::game::map::outdoor_pvp::graveyard_open(m, 927, 469),
                crate::game::map::outdoor_pvp::graveyard_open(m, 927, 67),
            )
        })
        .await
        .unwrap()
    };
    a.wait_for("the slider", |m| match m {
        ServerOpcodeMessage::SMSG_UPDATE_WORLD_STATE(w) => {
            (w.state.state == 2426 && w.state.value == 1).then_some(())
        }
        _ => None,
    })
    .await;
    assert_eq!(open(map.clone()).await, (false, false));
    map.query(|m| {
        let o = m.outdoor.as_mut().unwrap();
        o.last_tick = Some(Instant::now() - Duration::from_secs(600));
        o.next_tick = Some(Instant::now());
    })
    .await;
    for _ in 0..100 {
        if open(map.clone()).await.0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(open(map.clone()).await, (true, false));
    let banner = map
        .query(|m| m.gameobjects.iter().any(|(_, o)| o.entry == 180421))
        .await
        .unwrap();
    assert!(banner, "the Alliance's banner over the graveyard");
}

#[tokio::test]
async fn a_battleground_quest_spell_gives_its_honor() {
    let mut d = GameData::empty();
    let mut s = crate::game::data::spells::SpellTemplate {
        id: 24961,
        name: "Honor Points +82".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0].effect = crate::game::map::spells::EFFECT_ADD_HONOR;
    s.effects[0].base_points = 81;
    s.effects[0].target_a = 1;
    d.spells.spells.insert(24961, s);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let key = realm.registry.online(g).unwrap().map().key;
    let map = realm.maps.get(&realm, key);
    map.query(move |m| crate::game::map::spells::cast_triggered(m, g, 24961, Some(g)))
        .await;
    let credit = a
        .wait_frame(
            "SMSG_PVP_CREDIT",
            crate::game::map::pvp::SMSG_PVP_CREDIT,
            |_| true,
        )
        .await;
    assert_eq!(
        u32::from_le_bytes([credit[0], credit[1], credit[2], credit[3]]),
        82
    );
    let week = map
        .query(move |m| {
            m.players[&g]
                .fields
                .get_u32(PLAYER_FIELD_THIS_WEEK_CONTRIBUTION)
        })
        .await
        .unwrap();
    assert_eq!(week, 82);
}

/// The valley with a cavalry commander, a quartermaster, the orders and
/// the ground commander's route.
fn assaults() -> GameData {
    let mut d = valley();
    npc(&mut d, 13577, "Stormpike Ram Rider Commander", 35);
    npc(&mut d, 12096, "Stormpike Quartermaster", 35);
    npc(&mut d, 13446, "Field Marshal Teravaine", 35);
    npc(&mut d, 13576, "Stormpike Ram Rider", 35);
    spawn(&mut d, 170004, 13577, 4.0);
    spawn(&mut d, 170005, 12096, 5.0);
    d.bg_routes.insert(13577, vec![(30.0, 0.0, 0.0)]);
    d.bg_routes.insert(13446, vec![(40.0, 0.0, 0.0)]);
    d.items.insert(
        17353,
        ItemTemplate {
            entry: 17353,
            class: 12,
            name: "Stormpike Assault Orders".into(),
            stackable: 1,
            ..Default::default()
        },
    );
    d
}

#[tokio::test]
async fn the_cavalry_charges_and_the_ground_assault_marches() {
    let realm = harness::realm_with(assaults());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::AlteracValley).await;
    map.query(move |m| crate::game::map::reputation::modify(m, g, 730, 9000, false))
        .await;
    // Twenty-five hides and twenty-five mounts: the stables fill.
    turn_in(&map, g, 7026, 25).await;
    turn_in(&map, g, 7027, 25).await;
    let stable = map
        .query(|m| m.battleground.as_ref().unwrap().events.get(&90).copied())
        .await
        .unwrap();
    assert_eq!(stable, Some(Some(0)), "the first stable's rams");
    let commander = map.query(|m| creature(m, 13577)).await.flatten().unwrap();
    assert_eq!(options(&mut a, commander).await, 1);
    choose(&a, commander, 0).await;
    for _ in 0..100 {
        let riders = map
            .query(|m| m.creatures.iter().filter(|(_, c)| c.entry == 13576).count())
            .await
            .unwrap();
        if riders == 8 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let riders = map
        .query(|m| m.creatures.iter().filter(|(_, c)| c.entry == 13576).count())
        .await
        .unwrap();
    assert_eq!(riders, 8, "eight riders ride with him");
    let stable = map
        .query(|m| m.battleground.as_ref().unwrap().events.get(&90).copied())
        .await
        .unwrap();
    assert_eq!(stable, Some(Some(2)), "the stables empty");
    // Twenty-eight Irondeep supplies: orders, and the field marshal.
    turn_in(&map, g, 5892, 28).await;
    let quartermaster = map.query(|m| creature(m, 12096)).await.flatten().unwrap();
    assert_eq!(
        options(&mut a, quartermaster).await,
        3,
        "status, orders, goods"
    );
    choose(&a, quartermaster, 1).await;
    for _ in 0..100 {
        if map.query(|m| creature(m, 13446)).await.flatten().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let has_orders = map
        .query(move |m| m.players[&g].count_of(17353))
        .await
        .unwrap();
    assert_eq!(has_orders, 1);
    // The orders turned in: he marches.
    turn_in(&map, g, 6846, 1).await;
    let marching = map
        .query(|m| {
            m.battleground
                .as_ref()
                .unwrap()
                .av
                .as_ref()
                .unwrap()
                .extras
                .ground[0]
                .phase
                == crate::game::map::battleground::av_quests::March::Marching
        })
        .await
        .unwrap();
    assert!(marching);
}

#[tokio::test]
async fn a_beacon_left_a_minute_calls_down_its_gryphon_and_the_enemy_can_stop_one() {
    let mut d = valley();
    npc(&mut d, 13161, "Aerie Gryphon", 35);
    for e in [178724, 178725] {
        super::tests_pvp::banner(&mut d, e, 10);
    }
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::AlteracValley).await;
    // Two Alliance beacons planted where Grom is.
    let at = map
        .query({
            let gb = b.guid;
            move |m| m.players[&gb].position()
        })
        .await
        .unwrap();
    let (left, stopped) = map
        .query(move |m| {
            let mut place = |entry: u32, dx: f32| {
                let t = m.realm.data.gameobject_templates[&entry].clone();
                let mut go = crate::game::map::gameobject::GameObject::from_spawn(
                    &t,
                    &GameObjectSpawn {
                        guid: 0x80_0100 + entry,
                        entry,
                        map: AV,
                        x: at.x + dx,
                        y: at.y,
                        z: at.z,
                        state: 1,
                        ..Default::default()
                    },
                );
                go.spawn_guid = None;
                m.add_gameobject(go)
            };
            (place(178724, 1.0), place(178725, 2.0))
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    // Grom, of the Horde, disarms one: using it does nothing, his
    // Attacking cast does.
    b.send(CMSG_GAMEOBJ_USE { guid: stopped }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let still = map.query(move |m| m.gameobject(stopped).is_some()).await.unwrap();
    assert!(still);
    let gb = b.guid;
    map.query(move |m| crate::game::map::battleground::opened(m, gb, stopped))
        .await;
    map.query(|m| {
        let x = &mut m.battleground.as_mut().unwrap().av.as_mut().unwrap().extras;
        for b in x.beacons.iter_mut() {
            b.ready_at = Instant::now();
        }
        x.hurry(Instant::now());
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let (gryphons, objects) = map
        .query(move |m| {
            (
                m.creatures.iter().filter(|(_, c)| c.entry == 13161).count(),
                (
                    m.gameobject(left).is_some(),
                    m.gameobject(stopped).is_some(),
                ),
            )
        })
        .await
        .unwrap();
    assert_eq!(gryphons, 1, "only the beacon left alone called");
    assert_eq!(objects, (false, false));
}
