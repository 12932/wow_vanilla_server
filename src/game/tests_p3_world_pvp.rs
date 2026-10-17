//! Phase 3's world PvP against Thistle Tea's tests: the capture meters'
//! arithmetic, who takes part at a tower, the towers' buffs, sliders,
//! credit and services, and silithyst's pickups, refreshes, turn-ins and
//! drops.

use std::time::{Duration, Instant};

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_pvp::{banner, marker};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;
use crate::game::map::outdoor_pvp::{
    self as opvp, Meter, PLAGUELANDS, Phase, SILITHUS, SPELL_CARRYING,
};

/// A Plaguelands meter, as the real towers have theirs.
fn meter() -> Meter {
    let mut t = GameObjectTemplate {
        entry: 181899,
        kind: 29,
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
    Meter::from_template(&t).unwrap()
}

#[test]
fn a_meter_stops_on_a_tie_keeping_its_progress_adds_up_split_ticks_and_caps_a_crowd() {
    // Three against one for a second; then two and two hold it still.
    let mut m = meter();
    m.advance(3, 1, 1000);
    assert_eq!(m.progress, 2000);
    let before = m;
    m.advance(2, 2, 10_000);
    assert_eq!((m.progress, m.phase), (before.progress, Phase::Neutral));
    let tied = m;
    m.advance(0, 0, 30_000);
    assert_eq!(
        (m.progress, m.phase, m.difference),
        (tied.progress, tied.phase, tied.difference)
    );
    // A thousand ticks of seven milliseconds are seven seconds' worth, and
    // no time is none.
    let mut split = meter();
    for _ in 0..1000 {
        split.advance(3, 1, 7);
    }
    let mut whole = meter();
    whole.advance(3, 1, 7000);
    assert_eq!((split.progress, split.phase), (whole.progress, whole.phase));
    let mut none = meter();
    none.advance(3, 1, 0);
    assert_eq!(none.progress, 0);
    // Ten thousand for a millisecond move it no faster than the quickest
    // capture allows.
    let mut crowd = meter();
    crowd.advance(10_000, 0, 1);
    assert_eq!(crowd.progress, 2500);
}

#[test]
fn the_real_capture_points_have_their_timings_and_slider_fields() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    let points: Vec<&GameObjectTemplate> = d
        .gameobject_templates
        .values()
        .filter(|t| t.kind == 29)
        .collect();
    assert_eq!(points.len(), 4);
    for t in points {
        let m = Meter::from_template(t).expect("a meter");
        assert_eq!(
            (m.radius, m.neutral_percent, m.min_time, m.max_time),
            (80.0, 20, 480, 1200),
            "{}",
            t.entry
        );
        assert_eq!(
            (m.display_state, m.position_state, m.neutral_state),
            (2426, 2427, 2428)
        );
    }
}

/// The four towers, their banners, the buffs of holding them, and what
/// the towers' holders put up.
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
    for s in [
        11413, 11414, 11415, 1386, 30880, 30683, 30682, 29520, 31951, 31954, 17327, 31309,
    ] {
        marker(&mut d, s);
    }
    for e in [
        180421, 180422, 181682, 181955, 180100, 180101, 181852, 181853,
    ] {
        banner(&mut d, e, 5);
    }
    for e in [18039, 17635, 17995, 17647, 17996, 17209] {
        super::tests_pvp2::npc(&mut d, e, &format!("Service {e}"), 35);
    }
    for (spell, aura) in [
        (90_016, crate::game::map::auras::AURA_MOD_STEALTH),
        (90_018, crate::game::map::auras::AURA_MOD_INVISIBILITY),
    ] {
        let mut s = crate::game::data::spells::SpellTemplate {
            id: spell,
            name: format!("Hidden {spell}"),
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0].effect = 6;
        s.effects[0].aura = aura;
        s.effects[0].target_a = 1;
        d.spells.spells.insert(spell, s);
    }
    // The towers' credits, as quests.
    for (quest, credit) in [(90_100, 17696), (90_101, 17690)] {
        let mut q = crate::game::data::npc::Quest {
            id: quest,
            ..Default::default()
        };
        q.required_kills[0] = (credit, 1);
        d.npc.quests.insert(quest, q);
    }
    d
}

/// The towers' places: Crown Guard, Eastwall, Northpass, Plaguewood.
const AT: [(f32, f32, f32); 4] = [
    (1860.85, -3731.23, 196.716),
    (2574.51, -4794.89, 144.704),
    (3181.08, -4379.36, 174.123),
    (2962.71, -3042.31, 154.789),
];

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

/// Players in the Eastern Kingdoms, the towers up.
async fn in_the_plaguelands(
    names: &[(&str, Race)],
) -> (std::sync::Arc<crate::game::Realm>, Vec<Client>, MapHandle) {
    let realm = harness::realm_with(plaguelands());
    let mut clients = Vec::new();
    for (name, race) in names {
        let mut c = Client::connect(&realm, name);
        c.enter(name, *race, Class::Warrior).await;
        clients.push(c);
    }
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    for _ in 0..200 {
        if map
            .query(|m| m.outdoor.as_ref().is_some_and(|o| o.towers.len() == 4))
            .await
            .unwrap_or(false)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    (realm, clients, map)
}

/// `g` in the Plaguelands at `at`, flagged for PvP.
fn stand(m: &mut crate::game::map::Map, g: Guid, at: (f32, f32, f32)) {
    m.teleport(
        g,
        m.key,
        Vector3d {
            x: at.0,
            y: at.1,
            z: at.2,
        },
        0.0,
    );
    let p = m.players.get_mut(&g).unwrap();
    p.zone = PLAGUELANDS;
    p.pvp.desired = true;
}

fn members(m: &mut crate::game::map::Map) -> Vec<(Guid, usize)> {
    let mut v: Vec<(Guid, usize)> = m
        .outdoor
        .as_ref()
        .unwrap()
        .members
        .iter()
        .map(|(g, i)| (*g, *i))
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn only_the_living_flagged_and_seen_take_part_at_a_tower() {
    let (_realm, clients, map) = in_the_plaguelands(&[("Alice", Race::Human)]).await;
    let g = clients[0].guid;
    map.query(move |m| stand(m, g, AT[2])).await;
    tick(&map).await;
    assert_eq!(map.query(members).await, Some(vec![(g, 2)]), "flagged");
    // Not flagged on a normal realm: no part.
    map.query(move |m| m.players.get_mut(&g).unwrap().pvp.desired = false)
        .await;
    tick(&map).await;
    assert_eq!(map.query(members).await, Some(vec![]), "not flagged");
    map.query(move |m| m.players.get_mut(&g).unwrap().pvp.desired = true)
        .await;
    // A GM, one flying, the stealthed and the invisible: none.
    let refusals: [fn(&mut crate::game::map::Map, Guid); 4] = [
        |m, g| {
            m.players.get_mut(&g).unwrap().fields.set_flag(
                PLAYER_FLAGS,
                crate::game::map::player::PLAYER_FLAGS_GM,
                true,
            )
        },
        |m, g| {
            let pos = m.players[&g].position();
            let far = Vector3d {
                x: pos.x + 1000.0,
                ..pos
            };
            m.players.get_mut(&g).unwrap().flight = Some(crate::game::map::npc::Flight {
                walk: crate::game::map::creature::Walk::new(
                    vec![pos, far],
                    1.0,
                    Instant::now(),
                    0,
                    true,
                ),
                destination: 0,
                mount: 0,
            });
        },
        |m, g| crate::game::map::spells::cast_triggered(m, g, 90_016, Some(g)),
        |m, g| crate::game::map::spells::cast_triggered(m, g, 90_018, Some(g)),
    ];
    let undo: [fn(&mut crate::game::map::Map, Guid); 4] = [
        |m, g| {
            m.players.get_mut(&g).unwrap().fields.set_flag(
                PLAYER_FLAGS,
                crate::game::map::player::PLAYER_FLAGS_GM,
                false,
            )
        },
        |m, g| m.players.get_mut(&g).unwrap().flight = None,
        |m, g| crate::game::map::auras::remove_spell(m, g, 90_016, None),
        |m, g| crate::game::map::auras::remove_spell(m, g, 90_018, None),
    ];
    for (i, (refuse, undo)) in refusals.into_iter().zip(undo).enumerate() {
        map.query(move |m| refuse(m, g)).await;
        tick(&map).await;
        assert_eq!(map.query(members).await, Some(vec![]), "refusal {i}");
        map.query(move |m| undo(m, g)).await;
        tick(&map).await;
        assert_eq!(map.query(members).await, Some(vec![(g, 2)]), "back {i}");
    }
    // Dead, and a ghost: none.
    map.query(move |m| crate::game::map::death::kill(m, g))
        .await;
    tick(&map).await;
    assert_eq!(map.query(members).await, Some(vec![]), "dead");
    map.query(move |m| crate::game::map::death::release(m, g))
        .await;
    tick(&map).await;
    assert_eq!(map.query(members).await, Some(vec![]), "a ghost");
}

async fn slider(c: &mut Client) -> Vec<(u32, u32)> {
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut out = Vec::new();
    while let Some(w) = c
        .gets(Duration::from_millis(20), |m| match m {
            ServerOpcodeMessage::SMSG_UPDATE_WORLD_STATE(w) => Some((w.state.state, w.state.value)),
            _ => None,
        })
        .await
    {
        out.push(w);
    }
    out
}

#[tokio::test]
async fn a_tower_lost_takes_its_buff_once_and_a_far_towers_change_resends_ones_own_slider_last() {
    let (_realm, mut clients, map) = in_the_plaguelands(&[("Alice", Race::Human)]).await;
    let alice = clients[0].guid;
    let buffed = move |m: &mut crate::game::map::Map| m.players[&alice].auras.has_spell(11413);
    // Northpass at the edge of the neutral band, Alice at it: the
    // Alliance's, and its buff hers.
    map.query(move |m| {
        stand(m, alice, AT[2]);
        m.outdoor.as_mut().unwrap().towers[2].meter.progress = 240_000;
    })
    .await;
    tick(&map).await;
    assert_eq!(map.query(buffed).await, Some(true), "one tower held");
    // She walks off, in the zone still, and it slips back into the band:
    // lost, the buff gone; another second, nothing more.
    map.query(move |m| {
        stand(m, alice, (2800.0, -4000.0, 150.0));
        m.outdoor.as_mut().unwrap().towers[2].meter.progress = 239_500;
    })
    .await;
    tick(&map).await;
    let after = map
        .query(move |m| (buffed(m), m.outdoor.as_ref().unwrap().towers[2].meter.phase))
        .await
        .unwrap();
    assert_eq!(after, (false, Phase::Neutral), "lost: no buff");
    tick(&map).await;
    assert_eq!(map.query(buffed).await, Some(false));
    // Back at Northpass, her slider shown; then Eastwall taken far off:
    // she's told the towers' states, and her own slider again, last.
    map.query(move |m| stand(m, alice, AT[2])).await;
    tick(&map).await;
    clients[0].clear();
    map.query(|m| m.outdoor.as_mut().unwrap().towers[1].meter.progress = -1_200_000)
        .await;
    tick(&map).await;
    let states = slider(&mut clients[0]).await;
    assert!(states.contains(&(2328, 1)), "{states:?}");
    assert_eq!(states.last().map(|s| s.0), Some(2427), "{states:?}");
}

#[tokio::test]
async fn a_towers_credit_reaches_its_sides_flagged_players_within_a_hundred_yards() {
    let (_realm, clients, map) = in_the_plaguelands(&[
        ("Alice", Race::Human),
        ("Carl", Race::Human),
        ("Dana", Race::Human),
    ])
    .await;
    let (alice, carl, dana) = (clients[0].guid, clients[1].guid, clients[2].guid);
    // Alice at the tower; Carl ninety yards off, out of its meter but in
    // the credit's reach; Dana a hundred and one off.
    let (x, y, z) = (3180.54, -4379.31, 175.275);
    map.query(move |m| {
        stand(m, alice, AT[2]);
        stand(m, carl, (x + 90.0, y, z));
        stand(m, dana, (x + 101.0, y, z));
        for g in [alice, carl, dana] {
            m.players
                .get_mut(&g)
                .unwrap()
                .journal
                .log
                .push(crate::game::store::Logged {
                    quest: 90_100,
                    ..Default::default()
                });
        }
        let o = m.outdoor.as_mut().unwrap();
        o.towers[2].meter.progress = 239_999;
    })
    .await;
    tick(&map).await;
    let credits = map
        .query(move |m| [alice, carl, dana].map(|g| m.players[&g].journal.log[0].kills[0]))
        .await
        .unwrap();
    assert_eq!(credits, [1, 1, 0]);
}

#[tokio::test]
async fn the_hordes_towers_put_up_the_hordes_services_and_reinforcements() {
    let (_realm, _clients, map) = in_the_plaguelands(&[("Grom", Race::Orc)]).await;
    map.query(|m| {
        let o = m.outdoor.as_mut().unwrap();
        for t in o.towers.iter_mut() {
            t.meter.progress = -240_000;
        }
        // Eastwall held whole: its flare too.
        o.towers[1].meter.progress = -1_200_000;
    })
    .await;
    tick(&map).await;
    let services = map
        .query(|m| {
            let o = m.outdoor.as_ref().unwrap();
            let mut v: Vec<((usize, u8), u32)> =
                o.services.iter().map(|(k, (e, _))| (*k, *e)).collect();
            v.sort();
            let factions: Vec<u32> = o
                .services
                .values()
                .filter(|(e, _)| *e == 17209 || *e == 18039)
                .map(|(_, g)| {
                    m.creature(*g)
                        .unwrap()
                        .fields
                        .get_u32(UNIT_FIELD_FACTIONTEMPLATE)
                })
                .collect();
            let auras: Vec<bool> = o
                .services
                .values()
                .filter(|(e, _)| *e == 18039)
                .map(|(_, g)| m.creature(*g).unwrap().auras.has_spell(31951))
                .collect();
            (v, factions, auras)
        })
        .await
        .unwrap();
    let (v, factions, auras) = services;
    let of = |k: (usize, u8)| v.iter().find(|(key, _)| *key == k).map(|(_, e)| *e);
    // Crown Guard's aura object and spirit, Eastwall's squad, Northpass's
    // shrine, Plaguewood's flight master: the Horde's own.
    assert_eq!(of((0, 0)), Some(180422));
    assert_eq!((of((1, 0)), of((1, 4))), (Some(17995), Some(17996)));
    assert_eq!(of((1, 9)), Some(181853), "Eastwall's flare");
    assert_eq!(of((2, 0)), Some(181955));
    assert_eq!(of((3, 0)), Some(17209));
    assert!(factions.iter().all(|f| *f == 775), "{factions:?}");
    assert_eq!(auras, vec![true]);
}

// Silithyst.

const MOUND: u32 = 181597;
const GEYSER: u32 = 181598;

fn silithus() -> GameData {
    let mut d = GameData::empty();
    banner(&mut d, GEYSER, 10);
    banner(&mut d, MOUND, 10);
    for s in [
        SPELL_CARRYING,
        29894,
        29895,
        30754,
        29534,
        31420,
        31247,
        30176,
    ] {
        marker(&mut d, s);
    }
    for (spell, aura) in [
        (90_016, crate::game::map::auras::AURA_MOD_STEALTH),
        (90_018, crate::game::map::auras::AURA_MOD_INVISIBILITY),
    ] {
        let mut s = crate::game::data::spells::SpellTemplate {
            id: spell,
            name: format!("Hidden {spell}"),
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0].effect = 6;
        s.effects[0].aura = aura;
        s.effects[0].target_a = 1;
        d.spells.spells.insert(spell, s);
    }
    for (trigger, x) in [(4162u32, 10.0f32), (4168, 20.0)] {
        d.area_triggers.insert(
            trigger,
            AreaTrigger {
                id: trigger,
                map: 1,
                x,
                y: 0.0,
                z: 0.0,
                radius: 50.0,
                ..Default::default()
            },
        );
    }
    d
}

/// A geyser by `g`, in Silithus.
fn geyser_by(m: &mut crate::game::map::Map, g: Guid) -> Guid {
    m.players.get_mut(&g).unwrap().zone = SILITHUS;
    let pos = m.players[&g].position();
    let t = m.realm.data.gameobject_templates[&GEYSER].clone();
    let go = crate::game::map::gameobject::GameObject::from_spawn(
        &t,
        &GameObjectSpawn {
            guid: 0x7E_0000 + m.gameobjects.len() as u32,
            entry: GEYSER,
            map: 1,
            x: pos.x + 1.0,
            y: pos.y,
            z: pos.z,
            state: 1,
            ..Default::default()
        },
    );
    m.add_gameobject(go)
}

fn carrying(m: &crate::game::map::Map, g: Guid) -> bool {
    m.players
        .get(&g)
        .is_some_and(|p| p.auras.has_spell(SPELL_CARRYING))
}

fn mounds(m: &crate::game::map::Map) -> usize {
    m.gameobjects
        .iter()
        .filter(|(_, o)| o.entry == MOUND)
        .count()
}

async fn kalimdor(
    realm: &std::sync::Arc<crate::game::Realm>,
    name: &str,
    race: Race,
) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, name);
    let g = c.enter(name, race, Class::Warrior).await;
    let key = realm.registry.online(g).unwrap().map().key;
    assert_eq!(key.map, 1);
    (c, g, realm.maps.get(realm, key))
}

#[tokio::test]
async fn silithyst_is_one_to_a_living_carrier_and_picking_it_up_contests_them() {
    let realm = harness::realm_with(silithus());
    let (a, g, map) = kalimdor(&realm, "Alice", Race::NightElf).await;
    let first = map.query(move |m| geyser_by(m, g)).await.unwrap();
    a.send(CMSG_GAMEOBJ_USE { guid: first }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (carries, contested, flagged, desired) = map
        .query(move |m| {
            let p = &m.players[&g];
            (
                carrying(m, g),
                p.pvp.contested_until.is_some(),
                p.fields
                    .has_flag(UNIT_FIELD_FLAGS, crate::game::map::player::UNIT_FLAG_PVP),
                p.pvp.desired,
            )
        })
        .await
        .unwrap();
    assert!(
        carries && contested && flagged,
        "carried, contested and flagged"
    );
    assert!(!desired, "not by her own wish");
    // A second geyser, and a mound: she can't take more.
    let second = map.query(move |m| geyser_by(m, g)).await.unwrap();
    a.send(CMSG_GAMEOBJ_USE { guid: second }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        map.query(move |m| m.gameobject(second).is_some())
            .await
            .unwrap(),
        "the geyser untouched"
    );
    // Dead, she can't pick one up.
    map.query(move |m| {
        crate::game::map::auras::remove_spell(m, g, SPELL_CARRYING, None);
        crate::game::map::death::kill(m, g);
    })
    .await;
    let third = map.query(move |m| geyser_by(m, g)).await.unwrap();
    a.send(CMSG_GAMEOBJ_USE { guid: third }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        map.query(move |m| carrying(m, g)).await,
        Some(false),
        "not the dead"
    );
}

#[tokio::test]
async fn the_carriers_pulse_keeps_them_contested_only_while_they_carry() {
    let realm = harness::realm_with(silithus());
    let (a, g, map) = kalimdor(&realm, "Alice", Race::NightElf).await;
    let geyser = map.query(move |m| geyser_by(m, g)).await.unwrap();
    a.send(CMSG_GAMEOBJ_USE { guid: geyser }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let pulse = realm.data.spells.get(30176).unwrap().clone();
    // The contest nearly over, the pulse renews it.
    let renewed = map
        .query(move |m| {
            m.players.get_mut(&g).unwrap().pvp.contested_until = Some(Instant::now());
            opvp::carrier_pulse(m, g, &pulse);
            m.players[&g].pvp.contested_until
        })
        .await
        .flatten()
        .unwrap();
    assert!(renewed > Instant::now() + Duration::from_secs(20));
    // Not carrying: the pulse does nothing.
    let pulse = realm.data.spells.get(30176).unwrap().clone();
    let after = map
        .query(move |m| {
            crate::game::map::auras::remove_spell(m, g, SPELL_CARRYING, None);
            let at = Instant::now();
            m.players.get_mut(&g).unwrap().pvp.contested_until = Some(at);
            opvp::carrier_pulse(m, g, &pulse);
            (m.players[&g].pvp.contested_until, at)
        })
        .await
        .unwrap();
    assert_eq!(after.0, Some(after.1));
}

#[tokio::test]
async fn silithyst_is_handed_in_only_at_ones_own_camp_alive_and_carrying() {
    let realm = harness::realm_with(silithus());
    let (a, g, map) = kalimdor(&realm, "Alice", Race::NightElf).await;
    let brought = |m: &mut crate::game::map::Map| {
        m.outdoor
            .as_ref()
            .unwrap()
            .silithus
            .as_ref()
            .unwrap()
            .brought
    };
    let geyser = map.query(move |m| geyser_by(m, g)).await.unwrap();
    a.send(CMSG_GAMEOBJ_USE { guid: geyser }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    // At the Horde's camp: nothing.
    map.query(move |m| opvp::area_trigger(m, g, 4168)).await;
    assert_eq!(map.query(brought).await, Some([0, 0]));
    assert_eq!(map.query(move |m| carrying(m, g)).await, Some(true));
    // At her own, dead: nothing.
    map.query(move |m| {
        m.players.get_mut(&g).unwrap().death = crate::game::map::death::Death::Dead {
            since: Instant::now(),
        };
        opvp::area_trigger(m, g, 4162);
        m.players.get_mut(&g).unwrap().death = crate::game::map::death::Death::Alive;
    })
    .await;
    assert_eq!(map.query(brought).await, Some([0, 0]));
    // Alive: handed in, once.
    map.query(move |m| {
        opvp::area_trigger(m, g, 4162);
        opvp::area_trigger(m, g, 4162);
    })
    .await;
    assert_eq!(map.query(brought).await, Some([1, 0]));
    assert_eq!(
        map.query(move |m| (carrying(m, g), mounds(m))).await,
        Some((false, 0)),
        "no mound left"
    );
    // Out of Kalimdor, the camps' trigger is no camp.
    let ek = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    assert!(
        ek.query(|m| m.outdoor.as_ref().is_none_or(|o| o.silithus.is_none()))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn silithyst_falls_once_at_death_stealth_or_invisibility_and_not_at_a_refresh() {
    let realm = harness::realm_with(silithus());
    let (a, g, map) = kalimdor(&realm, "Alice", Race::NightElf).await;
    let take = || {
        let a = &a;
        let map = &map;
        async move {
            let geyser = map.query(move |m| geyser_by(m, g)).await.unwrap();
            a.send(CMSG_GAMEOBJ_USE { guid: geyser }).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(map.query(move |m| carrying(m, g)).await, Some(true));
        }
    };
    // Refreshed: the same aura again drops nothing.
    take().await;
    map.query(move |m| crate::game::map::spells::cast_triggered(m, g, SPELL_CARRYING, Some(g)))
        .await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        map.query(move |m| (carrying(m, g), mounds(m))).await,
        Some((true, 0)),
        "refreshed"
    );
    // Stealth, then invisibility: dropped, one mound each.
    for (n, hide) in [(1, 90_016), (2, 90_018)] {
        map.query(move |m| crate::game::map::spells::cast_triggered(m, g, hide, Some(g)))
            .await;
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert_eq!(
            map.query(move |m| (carrying(m, g), mounds(m))).await,
            Some((false, n)),
            "hidden {hide}"
        );
        map.query(move |m| crate::game::map::auras::remove_spell(m, g, hide, None))
            .await;
        take().await;
    }
    // Death: the same, once.
    map.query(move |m| crate::game::map::combat::kill(m, g, g))
        .await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        map.query(move |m| (carrying(m, g), mounds(m))).await,
        Some((false, 3)),
        "dead"
    );
}

#[tokio::test]
async fn the_cenarion_favor_follows_its_side_into_both_of_ahnqirajs_zones() {
    let mut d = silithus();
    for (id, name) in [(531, "AhnQiraj"), (509, "AhnQirajRuins")] {
        d.maps.insert(
            id,
            MapInfo {
                id,
                internal_name: name.into(),
                name: name.into(),
                instance_type: 2,
                player_limit: 40,
            },
        );
    }
    let realm = harness::realm_with(d);
    // The Alliance won Silithus: Kalimdor keeps the realm's tally.
    let kalimdor = realm.maps.get(&realm, crate::game::map::MapKey::world(1));
    kalimdor
        .query(|m| {
            let o = m.outdoor.as_mut().unwrap();
            o.silithus.as_mut().unwrap().controller = Some(0);
        })
        .await;
    tick(&kalimdor).await;
    assert_eq!(realm.social.outdoor.lock().favor, Some(0));
    for (n, (map_id, zone)) in [(531u32, 3428u32), (509, 3429)].into_iter().enumerate() {
        let mut favored = Vec::new();
        for (name, race) in [("Alice", Race::Human), ("Grom", Race::Orc)] {
            let name = format!("{name}{}", ["a", "b"][n]);
            let mut c = Client::connect(&realm, &name);
            let g = c.enter(&name, race, Class::Warrior).await;
            let home = realm
                .maps
                .get(&realm, realm.registry.online(g).unwrap().map().key);
            home.call(move |m| {
                m.teleport(
                    g,
                    crate::game::map::MapKey::world(map_id),
                    Vector3d {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    0.0,
                )
            });
            c.wait_for("SMSG_NEW_WORLD", |m| {
                matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
            })
            .await;
            c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
            c.sees(g).await;
            let key = realm.registry.online(g).unwrap().map().key;
            assert_eq!(key.map, map_id);
            let copy = realm.maps.get(&realm, key);
            copy.query(move |m| m.players.get_mut(&g).unwrap().zone = zone)
                .await;
            let mut has = false;
            for _ in 0..30 {
                tick(&copy).await;
                has = copy
                    .query(move |m| m.players[&g].auras.has_spell(30754))
                    .await
                    .unwrap();
                if has {
                    break;
                }
            }
            favored.push(has);
        }
        assert_eq!(favored, vec![true, false], "map {map_id}");
    }
}
