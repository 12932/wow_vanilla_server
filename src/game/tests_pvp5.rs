//! World PvP as Thistle Tea has it, id by id: silithyst lost every way but
//! at the camp falls where its carrier was and lies three minutes, a geyser
//! used up comes back, opening it picks it up; Silithus's tallies and the
//! towers' states for those in the zones and none for those gone; the
//! buffs on the living in their zones; the towers and their banners in
//! their holders' colours.

use std::time::{Duration, Instant};

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_pvp::{banner, marker};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;
use crate::game::map::outdoor_pvp::{self as opvp, PLAGUELANDS, SILITHUS, SPELL_CARRYING};

const MOUND: u32 = 181597;
const GEYSER: u32 = 181598;
const FAVOR: u32 = 30754;

fn silithus() -> GameData {
    let mut d = GameData::empty();
    banner(&mut d, GEYSER, 10);
    banner(&mut d, MOUND, 10);
    for s in [SPELL_CARRYING, 29894, 29895, FAVOR] {
        marker(&mut d, s);
    }
    d.gameobject_spawns
        .entry(1)
        .or_default()
        .push(GameObjectSpawn {
            guid: 49336,
            entry: GEYSER,
            map: 1,
            respawn_min: 180,
            respawn_max: 180,
            ..Default::default()
        });
    d
}

/// Alice, a night elf in Kalimdor, by a geyser.
async fn by_a_geyser(
    realm: &std::sync::Arc<crate::game::Realm>,
) -> (Client, Guid, MapHandle, Guid) {
    let mut a = Client::connect(realm, "A");
    let g = a.enter("Alice", Race::NightElf, Class::Warrior).await;
    let key = realm.registry.online(g).unwrap().map().key;
    let map = realm.maps.get(realm, key);
    let geyser = map
        .query(move |m| {
            m.players.get_mut(&g).unwrap().zone = SILITHUS;
            let pos = m.players[&g].position();
            let t = m.realm.data.gameobject_templates[&GEYSER].clone();
            let mut go = crate::game::map::gameobject::GameObject::from_spawn(
                &t,
                &GameObjectSpawn {
                    guid: 49336,
                    entry: GEYSER,
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
    (a, g, map, geyser)
}

fn carrying(m: &crate::game::map::Map, g: Guid) -> bool {
    m.players
        .get(&g)
        .is_some_and(|p| p.auras.has_spell(SPELL_CARRYING))
}

fn mounds(m: &crate::game::map::Map) -> Vec<(Guid, Vector3d)> {
    m.gameobjects
        .iter()
        .filter(|(_, o)| o.entry == MOUND)
        .map(|(_, o)| (o.guid, o.position))
        .collect()
}

/// The next second's world PvP, now.
async fn tick(map: &MapHandle) {
    map.query(|m| {
        if let Some(o) = m.outdoor.as_mut() {
            o.next_tick = Some(Instant::now());
        }
    })
    .await;
    tokio::time::sleep(Duration::from_millis(120)).await;
}

#[tokio::test]
async fn silithyst_lost_by_mounting_falls_where_its_carrier_was_for_three_minutes() {
    let realm = harness::realm_with(silithus());
    let (a, g, map, geyser) = by_a_geyser(&realm).await;
    a.send(CMSG_GAMEOBJ_USE { guid: geyser }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (has, team, geyser_left, respawn) = map
        .query(move |m| {
            (
                carrying(m, g),
                m.players[&g].auras.has_spell(29894),
                m.gameobject(geyser).is_some(),
                m.loot_state.respawns.iter().any(|(_, s)| *s == 49336),
            )
        })
        .await
        .unwrap();
    assert!(has && team, "carried, with the Alliance's aura");
    assert!(!geyser_left && respawn, "the geyser used up, to come back");
    // She mounts: it's lost, and falls where she is.
    let at = map
        .query(move |m| {
            let p = m.players.get_mut(&g).unwrap();
            p.fields.set_u32(UNIT_FIELD_MOUNTDISPLAYID, 1234);
            p.position()
        })
        .await
        .unwrap();
    tick(&map).await;
    let (has, team, dropped, lasts) = map
        .query(move |m| {
            let dropped = mounds(m);
            let lasts = dropped.first().and_then(|(d, _)| {
                m.object_resets
                    .iter()
                    .find(|(_, o, s)| o == d && *s == u32::MAX)
                    .map(|(t, _, _)| t.saturating_duration_since(Instant::now()))
            });
            (
                carrying(m, g),
                m.players[&g].auras.has_spell(29894),
                dropped,
                lasts,
            )
        })
        .await
        .unwrap();
    assert!(!has && !team);
    assert_eq!(dropped.len(), 1);
    assert!(crate::game::map::combat::dist(dropped[0].1, at) < 0.01);
    let lasts = lasts.expect("it goes in time");
    assert!(lasts > Duration::from_secs(170) && lasts <= opvp::MOUND_LASTS);
}

#[tokio::test]
async fn opening_silithyst_picks_it_up_and_a_carrier_leaving_drops_it_where_last_seen() {
    let realm = harness::realm_with(silithus());
    let (a, g, map, geyser) = by_a_geyser(&realm).await;
    // The client's Opening of it.
    map.query(move |m| crate::game::map::battleground::opened(m, g, geyser))
        .await;
    assert!(map.query(move |m| carrying(m, g)).await.unwrap());
    tick(&map).await;
    let at = map.query(move |m| m.players[&g].position()).await.unwrap();
    a.disconnect();
    for _ in 0..200 {
        if realm.registry.online(g).is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tick(&map).await;
    let dropped = map.query(|m| mounds(m)).await.unwrap();
    assert_eq!(dropped.len(), 1, "dropped as she went");
    assert!(crate::game::map::combat::dist(dropped[0].1, at) < 0.01);
}

#[tokio::test]
async fn a_cancelled_carry_drops_it_and_a_mound_picked_up_is_gone() {
    let realm = harness::realm_with(silithus());
    let (a, g, map, geyser) = by_a_geyser(&realm).await;
    a.send(CMSG_GAMEOBJ_USE { guid: geyser }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    a.send(CMSG_CANCEL_AURA { id: SPELL_CARRYING }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    tick(&map).await;
    let dropped = map.query(|m| mounds(m)).await.unwrap();
    assert_eq!(dropped.len(), 1);
    a.send(CMSG_GAMEOBJ_USE { guid: dropped[0].0 }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (has, left) = map
        .query(move |m| (carrying(m, g), mounds(m).len()))
        .await
        .unwrap();
    assert!(has);
    assert_eq!(left, 0, "picked up, it's gone");
}

async fn state_of(c: &mut Client, field: u32) -> u32 {
    c.wait_for("the world state", |m| match m {
        ServerOpcodeMessage::SMSG_UPDATE_WORLD_STATE(w) if w.state.state == field => {
            Some(w.state.value)
        }
        _ => None,
    })
    .await
}

#[tokio::test]
async fn silithus_tallies_go_to_those_in_it_and_the_favor_to_its_living_winners() {
    let realm = harness::realm_with(silithus());
    let (mut a, g, map, _) = by_a_geyser(&realm).await;
    map.query(|m| {
        let si = m.outdoor.as_mut().unwrap().silithus.as_mut().unwrap();
        si.brought = [7, 3];
        si.controller = Some(0);
        m.outdoor.as_mut().unwrap().watching.clear();
    })
    .await;
    tick(&map).await;
    assert_eq!(state_of(&mut a, 2313).await, 7);
    let favored = map
        .query(move |m| m.players[&g].auras.has_spell(FAVOR))
        .await
        .unwrap();
    assert!(favored, "the Alliance won it");
    // Out of the zone: no tallies, no favor.
    a.clear();
    map.query(move |m| m.players.get_mut(&g).unwrap().zone = 440)
        .await;
    tick(&map).await;
    assert_eq!(state_of(&mut a, 2313).await, 0);
    let favored = map
        .query(move |m| m.players[&g].auras.has_spell(FAVOR))
        .await
        .unwrap();
    assert!(!favored);
    // Back, and dead: none for the dead.
    map.query(move |m| {
        m.players.get_mut(&g).unwrap().zone = SILITHUS;
        crate::game::map::death::kill(m, g);
    })
    .await;
    tick(&map).await;
    let favored = map
        .query(move |m| m.players[&g].auras.has_spell(FAVOR))
        .await
        .unwrap();
    assert!(!favored);
    // A zone's init states carry its objectives.
    let init = map
        .query(|m| (opvp::zone_states(m, SILITHUS), opvp::zone_states(m, 141)))
        .await
        .unwrap();
    assert_eq!(init.0, vec![(2313, 7), (2314, 3), (2317, 200)]);
    assert!(init.1.is_empty());
}

/// The four towers, and their banners.
fn plaguelands() -> GameData {
    let mut d = GameData::empty();
    for entry in [182096, 182097, 181899, 182098] {
        let mut t = GameObjectTemplate {
            entry,
            kind: 29,
            display_id: 1,
            size: 1.0,
            name: format!("Tower {entry}"),
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
        d.gameobject_templates.insert(entry, t);
    }
    banner(&mut d, 182106, 5);
    for s in [11413, 11414, 11415, 1386] {
        marker(&mut d, s);
    }
    d
}

#[tokio::test]
async fn the_towers_wear_their_holders_colours_and_their_buff_follows_the_zones() {
    let realm = harness::realm_with(plaguelands());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let key = realm.registry.online(g).unwrap().map().key;
    let map = realm.maps.get(&realm, key);
    for _ in 0..100 {
        if map
            .query(|m| m.outdoor.as_ref().is_some_and(|o| o.towers.len() == 4))
            .await
            .unwrap_or(false)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let kits = |map: MapHandle| async move {
        map.query(|m| {
            let o = m.outdoor.as_ref().unwrap();
            let t = &o.towers[0];
            t.object
                .into_iter()
                .chain(t.banners.iter().copied())
                .map(|g| m.gameobject(g).unwrap().fields.get_u32(GAMEOBJECT_ARTKIT))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap()
    };
    let banners = map
        .query(|m| {
            m.gameobjects
                .iter()
                .filter(|(_, o)| o.entry == 182106)
                .count()
        })
        .await
        .unwrap();
    assert_eq!(banners, 8, "two by each tower");
    assert_eq!(kits(map.clone()).await, vec![21, 21, 21], "neutral");
    // Crown Guard the Alliance's: its colours, and Alice's buff in the zone.
    map.query(move |m| {
        m.players.get_mut(&g).unwrap().zone = PLAGUELANDS;
        let o = m.outdoor.as_mut().unwrap();
        o.towers[0].meter.progress = o.towers[0].meter.max_time * 1000;
    })
    .await;
    tick(&map).await;
    assert_eq!(kits(map.clone()).await, vec![2, 2, 2]);
    let buffed = map
        .query(move |m| m.players[&g].auras.has_spell(11413))
        .await
        .unwrap();
    assert!(buffed, "one tower held");
    let init = map
        .query(|m| opvp::zone_states(m, PLAGUELANDS))
        .await
        .unwrap();
    assert!(init.contains(&(2327, 1)) && init.contains(&(2426, 0)));
    // Out of the zones: her buff goes, and its states.
    a.clear();
    map.query(move |m| m.players.get_mut(&g).unwrap().zone = 12)
        .await;
    tick(&map).await;
    assert_eq!(state_of(&mut a, 2327).await, 0);
    let buffed = map
        .query(move |m| m.players[&g].auras.has_spell(11413))
        .await
        .unwrap();
    assert!(!buffed);
}

fn creature_template(entry: u32, civilian: bool, xp: f32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: format!("Peasant {entry}"),
        display_ids: [1, 0, 0, 0],
        display_scales: [1.0, 0.0, 0.0, 0.0],
        display_probabilities: [100, 0, 0, 0],
        level_min: 1,
        level_max: 1,
        faction: 35,
        health_max: 10,
        civilian,
        xp_multiplier: xp,
        ..Default::default()
    }
}

#[tokio::test]
async fn a_civilian_worth_nothing_is_a_dishonorable_kill_for_the_group_near() {
    let mut d = GameData::empty();
    d.creature_templates
        .insert(700, creature_template(700, true, 0.0));
    d.creature_templates
        .insert(701, creature_template(701, true, 1.0));
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let bob = b.enter("Bob", Race::Human, Class::Mage).await;
    a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
    b.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    a.wait_for("SMSG_GROUP_LIST", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_LIST(_)).then_some(())
    })
    .await;
    let key = realm.registry.online(alice).unwrap().map().key;
    let map = realm.maps.get(&realm, key);
    let dishonorable = move |m: &mut crate::game::map::Map| {
        let h = m.realm.social.honor.lock();
        [alice, bob].map(|g| {
            h.project(crate::game::guid::counter(g))
                .lifetime_dishonorable
        })
    };
    // A civilian worth experience: no shame in it.
    map.query(move |m| {
        let at = m.players[&alice].position();
        let c = crate::game::map::scripts::summon(m, alice, 701, at, 0.0, None).unwrap();
        crate::game::map::ai::die(m, c, alice);
    })
    .await;
    assert_eq!(map.query(dishonorable).await.unwrap(), [0, 0]);
    // One worth nothing: a dishonorable kill, for Bob too.
    map.query(move |m| {
        let at = m.players[&alice].position();
        let c = crate::game::map::scripts::summon(m, alice, 700, at, 0.0, None).unwrap();
        crate::game::map::ai::die(m, c, alice);
    })
    .await;
    assert_eq!(map.query(dishonorable).await.unwrap(), [1, 1]);
}

#[tokio::test]
async fn arriving_in_an_enemy_capital_is_honorless_a_while() {
    let mut d = GameData::empty();
    marker(&mut d, crate::game::map::pvp::SPELL_HONORLESS_TARGET);
    d.areas.insert(
        1519,
        AreaInfo {
            id: 1519,
            map: 0,
            flags: crate::game::data::AREA_FLAG_CAPITAL,
            team: 2,
            name: "Stormwind City".into(),
            ..Default::default()
        },
    );
    let realm = harness::realm_with(d);
    let mut g = Client::connect(&realm, "G");
    let grom = g.enter("Grom", Race::Undead, Class::Warrior).await;
    let key = realm.registry.online(grom).unwrap().map().key;
    let map = realm.maps.get(&realm, key);
    map.query(move |m| {
        m.players.get_mut(&grom).unwrap().zone = 1519;
        let at = m.players[&grom].position();
        m.teleport(
            grom,
            m.key,
            Vector3d {
                x: at.x + 5.0,
                ..at
            },
            0.0,
        );
    })
    .await;
    g.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    g.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: grom,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let honorless = map
        .query(move |m| {
            m.players[&grom]
                .auras
                .has_spell(crate::game::map::pvp::SPELL_HONORLESS_TARGET)
        })
        .await
        .unwrap();
    assert!(honorless);
}

#[tokio::test]
async fn a_duel_is_refused_where_duels_arent_fought_and_by_one_ignoring() {
    let mut d = GameData::empty();
    for (id, flags) in [(12, crate::game::map::duel::AREA_FLAG_DUELS), (1519, 0x138)] {
        d.areas.insert(
            id,
            AreaInfo {
                id,
                map: 0,
                flags,
                ..Default::default()
            },
        );
    }
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let bob = b.enter("Bob", Race::Human, Class::Mage).await;
    let key = realm.registry.online(alice).unwrap().map().key;
    let map = realm.maps.get(&realm, key);
    let check = move |area: u32| {
        move |m: &mut crate::game::map::Map| {
            for g in [alice, bob] {
                m.players.get_mut(&g).unwrap().area = area;
            }
            let mut s = crate::game::data::spells::SpellTemplate {
                id: 7266,
                ..Default::default()
            };
            s.effects[0].effect = crate::game::map::spells::EFFECT_DUEL;
            crate::game::map::duel::check(m, alice, &s, Some(bob))
        }
    };
    assert_eq!(map.query(check(12)).await.unwrap(), None, "Elwynn: fine");
    assert_eq!(
        map.query(check(1519)).await.unwrap(),
        Some(0x47),
        "not in a city"
    );
    // Bob ignores her.
    b.send(CMSG_ADD_IGNORE {
        name: "Alice".into(),
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(map.query(check(12)).await.unwrap(), Some(0x0a));
}
