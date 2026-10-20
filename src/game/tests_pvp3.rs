//! Alterac Valley's NPCs as Thistle Tea scripts them (the summoners and
//! their ritual, the wing commanders and their war riders, the cavalry and
//! the ground assault, the generals in their keeps), its beacons planted
//! and disarmed and its spells kept to the battle, and the Plaguelands'
//! squad and spirit, through the protocol.

use std::time::{Duration, Instant};

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_pvp::{both_into, said};
use super::tests_pvp2::{arrive, choose, creature, npc, options, turn_in, valley};
use crate::game::data::scripts::GroupMember;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;
use crate::game::map::battleground::{self as bg, av_routes, av_scripts};
use crate::game::map::event_ai;

const AV: u32 = 30;

fn spawn_at(d: &mut GameData, guid: u32, entry: u32, at: (f32, f32, f32)) {
    d.creature_spawns
        .entry(AV)
        .or_default()
        .push(CreatureSpawn {
            guid,
            entries: vec![entry],
            map: AV,
            x: at.0,
            y: at.1,
            z: at.2,
            ..Default::default()
        });
}

/// A route of `n` points from `from`, `step` apart along x.
fn line_from(from: (f32, f32, f32), n: usize, step: f32) -> Vec<(f32, f32, f32)> {
    (0..n)
        .map(|i| (from.0 + i as f32 * step, from.1, from.2))
        .collect()
}

/// A summoning ritual's object, needing `needed` of its side.
fn ritual(d: &mut GameData, entry: u32, needed: u32) {
    let mut data = [0; 24];
    data[0] = needed;
    data[2] = 11206;
    d.gameobject_templates.insert(
        entry,
        GameObjectTemplate {
            entry,
            kind: 18,
            display_id: 1,
            size: 1.0,
            name: "Altar".into(),
            data,
            ..Default::default()
        },
    );
}

fn text(d: &mut GameData, id: u32, line: &str) {
    d.npc.broadcast_texts.insert(
        id,
        crate::game::data::npc::BroadcastText {
            male: line.into(),
            chat_type: 6,
            ..Default::default()
        },
    );
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
        movement_counter: c.teleport_order(),
        time: Default::default(),
    })
    .await;
}

async fn until(
    map: &MapHandle,
    what: impl Fn(&crate::game::map::Map) -> bool + Send + Clone + 'static,
) {
    for _ in 0..200 {
        let w = what.clone();
        if map.query(move |m| w(m)).await.unwrap_or(false) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn mount_of(m: &crate::game::map::Map, g: Guid) -> u32 {
    m.unit_fields(g)
        .map_or(0, |f| f.get_u32(UNIT_FIELD_MOUNTDISPLAYID))
}

fn all_of(m: &crate::game::map::Map, entry: u32) -> Vec<Guid> {
    let mut out: Vec<Guid> = m
        .creatures
        .iter()
        .filter(|(_, c)| c.entry == entry && c.alive())
        .map(|(_, c)| c.guid)
        .collect();
    out.sort_by_key(|g| g.guid());
    out
}

async fn pair(
    realm: &std::sync::Arc<crate::game::Realm>,
) -> (Client, Guid, Client, Guid, MapHandle) {
    let mut a = Client::connect(realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(realm, "B");
    let gb = b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(realm, &mut a, &mut b, Map::AlteracValley).await;
    (a, ga, b, gb, map)
}

#[tokio::test]
async fn thurloga_marches_with_her_escorts_and_the_ritual_raises_lokholar() {
    let mut d = valley();
    npc(&mut d, 13236, "Primalist Thurloga", 35);
    npc(&mut d, 13284, "Frostwolf Shaman", 35);
    npc(&mut d, 13256, "Lokholar the Ice Lord", 35);
    let start = (-1319.6, -342.75, 0.0);
    spawn_at(&mut d, 170101, 13236, start);
    let mut members = vec![GroupMember {
        guid: 170101,
        distance: 0.0,
        angle: 0.0,
        flags: 15,
    }];
    for (i, g) in [170102, 170103, 170104].into_iter().enumerate() {
        spawn_at(&mut d, g, 13284, (start.0 - 3.0, start.1 + i as f32, 0.0));
        members.push(GroupMember {
            guid: g,
            distance: 4.0,
            angle: 150.0 + 30.0 * i as f32,
            flags: 15,
        });
    }
    for m in &members {
        d.scripts.group_of.insert(m.guid, 170101);
    }
    d.scripts.groups.insert(170101, members);
    d.bg_routes.insert(13236, line_from(start, 45, 1.0));
    d.bg_routes
        .insert(13256, line_from((-360.0, -133.0, 26.0), 40, 1.0));
    ritual(&mut d, 178465, 1);
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (mut a, _, mut b, grom, map) = pair(&realm).await;
    // Two hundred offerings: she sets out.
    turn_in(&map, grom, 7385, 40).await;
    // At her sixth point she mounts and runs, and her escorts with her.
    let (mount, runs, escorts) = map
        .query(|m| {
            let g = creature(m, 13236).unwrap();
            av_routes::hurry_until(m, |m| event_ai::phase(m, g) == 2);
            let escorts: Vec<u32> = all_of(m, 13284)
                .into_iter()
                .map(|e| mount_of(m, e))
                .collect();
            (
                mount_of(m, g),
                event_ai::scripted(m, g).is_some_and(|s| s.run),
                escorts,
            )
        })
        .await
        .unwrap();
    assert_eq!(mount, 12242);
    assert!(runs);
    assert_eq!(escorts, vec![1166, 1166, 1166], "her escorts ride too");
    // On foot at the altar: she and they invoke, the altar down.
    arrive(&map).await;
    let (phase, mount, escorts, altar) = map
        .query(|m| {
            let g = creature(m, 13236).unwrap();
            let escorts: Vec<u32> = all_of(m, 13284)
                .into_iter()
                .map(|e| event_ai::phase(m, e))
                .collect();
            let altar = m
                .gameobjects
                .iter()
                .find(|(_, o)| o.entry == 178465)
                .map(|(_, o)| (o.guid, o.position));
            (event_ai::phase(m, g), mount_of(m, g), escorts, altar)
        })
        .await
        .unwrap();
    assert_eq!((phase, mount), (3, 0), "invoking, on foot");
    assert_eq!(escorts, vec![3, 3, 3]);
    let (altar, at) = altar.expect("the Altar of Summoning");
    // Alice, of the Alliance, can't join the Horde's ritual; Grom can.
    put(&map, &mut a, at.x, at.y, at.z).await;
    a.send(CMSG_GAMEOBJ_USE { guid: altar }).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let none = map.query(|m| creature(m, 13256)).await.flatten();
    assert!(none.is_none());
    put(&map, &mut b, at.x + 1.0, at.y, at.z).await;
    b.send(CMSG_GAMEOBJ_USE { guid: altar }).await;
    until(&map, |m| creature(m, 13256).is_some()).await;
    let (lord, phase, altar_left) = map
        .query(move |m| {
            (
                creature(m, 13256),
                creature(m, 13236).map(|g| event_ai::phase(m, g)),
                m.gameobject(altar).is_some(),
            )
        })
        .await
        .unwrap();
    let lord = lord.expect("Lokholar rose");
    assert_eq!(phase, Some(4), "her work is done");
    assert!(!altar_left, "she took up the altar");
    // Lokholar marches to his thirty-second point and stands there,
    // home; Thurloga and her escorts are gone.
    arrive(&map).await;
    let (at, home, walking, left) = map
        .query(move |m| {
            let c = m.creature(lord).unwrap();
            (
                c.position,
                c.home,
                av_routes::walking(m, lord),
                (all_of(m, 13236).len(), all_of(m, 13284).len()),
            )
        })
        .await
        .unwrap();
    assert!(!walking);
    assert!((at.x - (-360.0 + 31.0)).abs() < 0.01, "{at:?}");
    assert!((home.x - at.x).abs() < 0.01);
    assert_eq!(left, (0, 0));
}

#[tokio::test]
async fn ichman_goes_home_then_rises_on_his_gryphon_and_sends_it_at_frostwolf_keep() {
    let mut d = valley();
    npc(&mut d, 14947, "Ichman's Gryphon", 14);
    // The gryphon's faction is the players' enemy; Grom's, an orc's.
    for (id, faction, ours, enemy_mask) in [(14, 15, 8, 1), (2, 76, 5, 0)] {
        d.faction_templates.insert(
            id,
            FactionTemplate {
                id,
                faction,
                ours,
                enemy_mask,
                ..Default::default()
            },
        );
    }
    d.creature_templates.get_mut(&13437).unwrap().npc_flags = 3;
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (mut a, alice, mut b, _, map) = pair(&realm).await;
    let ichman = map.query(|m| creature(m, 13437)).await.flatten().unwrap();
    // A prisoner, he sits.
    until(&map, move |m| {
        m.unit_fields(ichman)
            .is_some_and(|f| f.get_byte(UNIT_FIELD_BYTES_1, 0) == 1)
    })
    .await;
    // Freed, he stands, gives no quests, and runs home.
    a.send(CMSG_GOSSIP_HELLO { guid: ichman }).await;
    until(&map, move |m| av_routes::walking(m, ichman)).await;
    let (stands, quests, runs) = map
        .query(move |m| {
            let f = m.unit_fields(ichman).unwrap();
            (
                f.get_byte(UNIT_FIELD_BYTES_1, 0) == 0,
                f.get_u32(UNIT_NPC_FLAGS) & 2 != 0,
                event_ai::scripted(m, ichman).is_some_and(|s| s.run),
            )
        })
        .await
        .unwrap();
    assert_eq!((stands, quests, runs), (true, false, true));
    arrive(&map).await;
    let quests = map
        .query(move |m| m.unit_fields(ichman).unwrap().get_u32(UNIT_NPC_FLAGS) & 2 != 0)
        .await
        .unwrap();
    assert!(quests, "home, he gives his quests again");
    turn_in(&map, alice, 6943, 30).await;
    assert_eq!(options(&mut a, ichman).await, 2);
    // Grom waits at Frostwolf Keep.
    put(&map, &mut b, -1311.0, -350.0, 128.0).await;
    choose(&a, ichman, 1).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    // He mounts, rises, and sends out his gryphon, which flies at
    // Frostwolf Keep and fights once there.
    arrive(&map).await;
    let (display, flies, gryphon) = map
        .query(move |m| {
            (
                m.unit_fields(ichman).unwrap().get_u32(UNIT_FIELD_DISPLAYID),
                av_routes::flies(m, ichman),
                creature(m, 14947),
            )
        })
        .await
        .unwrap();
    assert_eq!(display, 1148);
    assert!(flies);
    let gryphon = gryphon.expect("his gryphon");
    let (home, flies, passive, move_flags) = map
        .query(move |m| {
            let c = m.creature(gryphon).unwrap();
            (
                c.home,
                av_routes::flies(m, gryphon),
                event_ai::scripted(m, gryphon).is_some_and(|s| s.passive),
                c.move_flags,
            )
        })
        .await
        .unwrap();
    assert!((home.x - -1311.53).abs() < 0.01 && (home.z - 130.93).abs() < 0.01);
    assert!(flies && move_flags & av_routes::MOVE_FLYING != 0);
    assert!(!passive, "there, it fights");
    let gb = b.guid;
    until(&map, move |m| {
        m.creature(gryphon)
            .is_some_and(|c| c.combat.attacking && c.combat.victim == gb)
    })
    .await;
    let fights = map
        .query(move |m| m.creature(gryphon).unwrap().combat.victim == gb)
        .await
        .unwrap();
    assert!(fights, "it goes for Grom");
}

/// The Stormpike rams and the Field Marshal's commandos, with the
/// quartermaster by where they assemble.
fn assaults() -> GameData {
    let mut d = valley();
    for (entry, name) in [
        (13577, "Stormpike Ram Rider Commander"),
        (13576, "Stormpike Ram Rider"),
        (12096, "Stormpike Quartermaster"),
        (13446, "Field Marshal Teravaine"),
        (13524, "Stormpike Commando"),
        (13525, "Seasoned Commando"),
    ] {
        npc(&mut d, entry, name, 35);
    }
    spawn_at(&mut d, 170201, 13577, (612.0, -30.0, 45.0));
    spawn_at(&mut d, 170202, 12096, (-246.0, -429.0, 20.0));
    d.bg_routes
        .insert(13577, line_from((612.0, -30.0, 45.0), 93, 0.05));
    d.bg_routes
        .insert(13446, line_from((-243.75, -431.32, 20.0), 50, 0.5));
    d.bg_routes
        .insert(13525, line_from((-240.0, -440.0, 20.0), 50, 0.5));
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
    av_scripts::install(&mut d);
    d
}

#[tokio::test]
async fn the_rams_ride_out_eight_strong_and_fall_in_behind_their_commander() {
    let realm = harness::realm_with(assaults());
    let (mut a, alice, _b, _, map) = pair(&realm).await;
    map.query(move |m| crate::game::map::reputation::modify(m, alice, 730, 9000, false))
        .await;
    let commander = map.query(|m| creature(m, 13577)).await.flatten().unwrap();
    // Mounted where he waits.
    until(&map, move |m| mount_of(m, commander) == 2786).await;
    turn_in(&map, alice, 7026, 25).await;
    turn_in(&map, alice, 7027, 25).await;
    put(&map, &mut a, 614.0, -30.0, 45.0).await;
    assert_eq!(options(&mut a, commander).await, 1);
    choose(&a, commander, 0).await;
    until(&map, |m| all_of(m, 13576).len() == 8).await;
    // His riders, mounted, wait on him until his call at the rally.
    let (mounts, phases, formation) = map
        .query(move |m| {
            av_routes::hurry_until(m, |m| {
                m.battleground
                    .as_ref()
                    .and_then(|b| b.av.as_ref())
                    .and_then(|av| av.extras.routes.walkers.values().next())
                    .is_some_and(|w| w.held.is_some())
            });
            let riders = all_of(m, 13576);
            (
                riders.iter().map(|r| mount_of(m, *r)).collect::<Vec<_>>(),
                riders
                    .iter()
                    .map(|r| event_ai::phase(m, *r))
                    .collect::<Vec<_>>(),
                av_routes::members(m, commander).len(),
            )
        })
        .await
        .unwrap();
    assert_eq!(mounts, vec![2786; 8]);
    assert_eq!(phases, vec![0; 8]);
    assert_eq!(formation, 0, "not yet in formation");
    // At his call they fall in, and ride behind him to the end.
    arrive(&map).await;
    let (phases, formation, at, behind) = map
        .query(move |m| {
            let riders = all_of(m, 13576);
            let at = m.creature(commander).unwrap().position;
            (
                riders
                    .iter()
                    .map(|r| event_ai::phase(m, *r))
                    .collect::<Vec<_>>(),
                av_routes::members(m, commander).len(),
                at,
                riders.iter().all(|r| {
                    crate::game::map::combat::dist(m.creature(*r).unwrap().position, at) < 25.0
                }),
            )
        })
        .await
        .unwrap();
    assert_eq!(phases, vec![2; 8]);
    assert_eq!(formation, 8);
    assert!((at.x - (612.0 + 92.0 * 0.05)).abs() < 0.01, "{at:?}");
    assert!(behind);
    // He falls: they fight on a while, then go.
    map.query(move |m| crate::game::map::ai::die(m, commander, alice))
        .await;
    let (phases, going) = map
        .query(|m| {
            let riders = all_of(m, 13576);
            (
                riders
                    .iter()
                    .map(|r| event_ai::phase(m, *r))
                    .collect::<Vec<_>>(),
                riders
                    .iter()
                    .all(|r| event_ai::scripted(m, *r).is_some_and(|s| s.despawn_at.is_some())),
            )
        })
        .await
        .unwrap();
    assert_eq!(phases, vec![1; 8]);
    assert!(going);
}

#[tokio::test]
async fn the_commandos_assemble_at_their_tier_and_go_on_alone_when_their_marshal_falls() {
    let realm = harness::realm_with(assaults());
    let (mut a, alice, _b, _, map) = pair(&realm).await;
    map.query(move |m| {
        crate::game::map::reputation::modify(m, alice, 730, 9000, false);
        m.battleground
            .as_mut()
            .unwrap()
            .av
            .as_mut()
            .unwrap()
            .extras
            .armor[0]
            .tier = 1;
    })
    .await;
    turn_in(&map, alice, 5892, 28).await;
    let quartermaster = map.query(|m| creature(m, 12096)).await.flatten().unwrap();
    put(&map, &mut a, -248.0, -429.0, 20.0).await;
    assert_eq!(options(&mut a, quartermaster).await, 3);
    choose(&a, quartermaster, 1).await;
    until(&map, |m| all_of(m, 13525).len() == 10).await;
    let (marshal, at, troops) = map
        .query(|m| {
            let g = creature(m, 13446);
            (
                g,
                g.and_then(|g| m.creature(g)).map(|c| c.position),
                all_of(m, 13525).len(),
            )
        })
        .await
        .unwrap();
    let marshal = marshal.expect("the Field Marshal");
    let at = at.unwrap();
    assert!((at.x - -243.75).abs() < 0.01 && (at.y - -431.32).abs() < 0.01);
    assert_eq!(troops, 10, "ten Seasoned Commandos, at the side's tier");
    // The orders turned in: he marches; he falls before his rally.
    turn_in(&map, alice, 6846, 1).await;
    let marching = map
        .query(move |m| av_routes::walking(m, marshal))
        .await
        .unwrap();
    assert!(marching);
    map.query(move |m| crate::game::map::ai::die(m, marshal, alice))
        .await;
    let (alone, phase) = map
        .query(|m| {
            let troops = all_of(m, 13525);
            (
                troops.iter().filter(|t| av_routes::walking(m, **t)).count(),
                m.battleground
                    .as_ref()
                    .unwrap()
                    .av
                    .as_ref()
                    .unwrap()
                    .extras
                    .ground[0]
                    .phase,
            )
        })
        .await
        .unwrap();
    assert_eq!(alone, 10, "each walks its own road");
    assert_eq!(phase, bg::av_quests::March::Dead);
}

#[tokio::test]
async fn a_beacon_planted_by_its_spell_belongs_to_no_one_and_only_the_enemy_disarms_it() {
    let mut d = valley();
    let mut s = crate::game::data::spells::SpellTemplate {
        id: 21728,
        name: "Planting Ichman's Beacon".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0].effect = 50;
    s.effects[0].misc_value = 178726;
    s.effects[0].target_a = 18;
    s.effects[1].effect = 41;
    s.effects[1].misc_value = 14029;
    s.effects[1].target_a = 22;
    s.effects[1].target_b = 32;
    d.spells.spells.insert(21728, s);
    super::tests_pvp::banner(&mut d, 178726, 10);
    npc(&mut d, 14029, "Trigger Ichman", 35);
    let realm = harness::realm_with(d);
    let (_a, alice, _b, grom, map) = pair(&realm).await;
    // The battle's and the side's.
    let checks = map
        .query(move |m| {
            let s = m.realm.data.spells.get(21728).unwrap().clone();
            (bg::spell_check(m, alice, &s), bg::spell_check(m, grom, &s))
        })
        .await
        .unwrap();
    assert_eq!(checks, (None, Some(0x5d)));
    map.query(move |m| crate::game::map::spells::cast_triggered(m, alice, 21728, None))
        .await;
    let beacon = map
        .query(|m| {
            m.gameobjects
                .iter()
                .find(|(_, o)| o.entry == 178726)
                .map(|(_, o)| {
                    (
                        o.guid,
                        o.fields.get_u32(GAMEOBJECT_FACTION),
                        o.fields.get_u32(GAMEOBJECT_LEVEL),
                        o.fields.get_guid(OBJECT_FIELD_CREATED_BY),
                    )
                })
        })
        .await
        .flatten();
    let (beacon, faction, level, owner) = beacon.expect("the beacon");
    assert_eq!((faction, level, owner), (83, 0, Guid::zero()));
    let triggers = map.query(|m| creature(m, 14029)).await.flatten();
    assert!(triggers.is_none(), "no trigger follows Alice about");
    until(&map, move |m| bg::av_quests::is_beacon(m, beacon)).await;
    // Alice's own side can't; Grom's Attacking cast does.
    map.query(move |m| bg::opened(m, alice, beacon)).await;
    let still = map
        .query(move |m| m.gameobject(beacon).is_some())
        .await
        .unwrap();
    assert!(still);
    map.query(move |m| bg::opened(m, grom, beacon)).await;
    let still = map
        .query(move |m| m.gameobject(beacon).is_some())
        .await
        .unwrap();
    assert!(!still);
}

#[tokio::test]
async fn battleground_spells_are_cast_only_in_their_battle() {
    let mut d = GameData::empty();
    for (id, ex3) in [(21728, 0), (22563, 0), (20000, 0x800), (20001, 0)] {
        let mut s = crate::game::data::spells::SpellTemplate {
            id,
            equipped_item_class: -1,
            ..Default::default()
        };
        s.attributes[3] = ex3;
        d.spells.spells.insert(id, s);
    }
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let key = realm.registry.online(g).unwrap().map().key;
    let map = realm.maps.get(&realm, key);
    let checks = map
        .query(move |m| {
            [21728, 22563, 20000, 20001].map(|id| {
                let s = m.realm.data.spells.get(id).unwrap().clone();
                bg::spell_check(m, g, &s)
            })
        })
        .await
        .unwrap();
    assert_eq!(checks, [Some(0x5d), Some(0x5d), Some(0x87), None]);
}

#[tokio::test]
async fn vanndar_calls_his_marshals_and_wont_be_drawn_from_his_keep() {
    let mut d = valley();
    npc(&mut d, 14762, "Dun Baldar North Marshal", 14);
    d.creature_templates.get_mut(&11948).unwrap().faction = 14;
    if let Some(v) = d
        .creature_spawns
        .get_mut(&AV)
        .and_then(|s| s.iter_mut().find(|s| s.entries == [11948]))
    {
        (v.x, v.y, v.z) = (722.4, -11.0, 50.7);
    }
    spawn_at(&mut d, 170301, 14762, (730.0, -11.0, 50.7));
    text(
        &mut d,
        10373,
        "You'll never get me out of me bunker, heathens!",
    );
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (mut a, alice, _b, _, map) = pair(&realm).await;
    put(&map, &mut a, 715.0, -11.0, 50.7).await;
    let (vanndar, marshal) = map
        .query(|m| (creature(m, 11948).unwrap(), creature(m, 14762).unwrap()))
        .await
        .unwrap();
    map.query(move |m| crate::game::map::ai::attacked(m, vanndar, alice))
        .await;
    // His marshal comes for Alice.
    until(&map, move |m| {
        m.creature(marshal)
            .is_some_and(|c| c.combat.attacking && c.combat.victim == alice)
    })
    .await;
    let joined = map
        .query(move |m| m.creature(marshal).unwrap().combat.victim == alice)
        .await
        .unwrap();
    assert!(joined);
    // Drawn out past thirty-five yards, he goes home and says so.
    map.query(move |m| {
        let c = m.creature_mut(vanndar).unwrap();
        c.position.x = 680.0;
    })
    .await;
    said(&mut a, "his leash", "bunker").await;
    let (evading, phase) = map
        .query(move |m| {
            (
                m.creature(vanndar).unwrap().evading,
                event_ai::phase(m, vanndar),
            )
        })
        .await
        .unwrap();
    assert!(evading);
    assert_eq!(phase, 0, "fighting again next time");
}

#[tokio::test]
async fn eastwall_held_brings_a_linked_squad_and_crown_guard_a_walking_spirit() {
    let mut d = GameData::empty();
    for (entry, name) in [(182096, "Crown Guard"), (182097, "Eastwall")] {
        let mut t = GameObjectTemplate {
            entry,
            kind: 29,
            display_id: 1,
            size: 1.0,
            name: name.into(),
            ..Default::default()
        };
        for (i, v) in [(0, 80), (12, 20), (16, 480), (17, 1200)] {
            t.data[i] = v;
        }
        d.gameobject_templates.insert(entry, t);
    }
    for (entry, name) in [
        (17635, "Lordaeron Commander"),
        (17647, "Lordaeron Soldier"),
        (18039, "Spirit of Victory"),
    ] {
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
                faction: 35,
                health_max: 100,
                ..Default::default()
            },
        );
    }
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let key = realm.registry.online(g).unwrap().map().key;
    let map = realm.maps.get(&realm, key);
    until(&map, |m| {
        m.outdoor.as_ref().is_some_and(|o| o.towers.len() == 2)
    })
    .await;
    // Both towers the Alliance's.
    map.query(|m| {
        let o = m.outdoor.as_mut().unwrap();
        for t in o.towers.iter_mut() {
            t.meter.progress = t.meter.max_time * 1000;
        }
        o.next_tick = Some(Instant::now());
    })
    .await;
    until(&map, |m| all_of(m, 17647).len() == 4).await;
    let (squad, spirit) = map
        .query(|m| {
            let commander = creature(m, 17635).unwrap();
            let soldiers = all_of(m, 17647);
            let linked = soldiers
                .iter()
                .filter(|s| {
                    m.script_groups
                        .get(s)
                        .is_some_and(|(l, d, _, _)| *l == commander && (*d - 5.0).abs() < 0.01)
                })
                .count();
            let spirit = creature(m, 18039)
                .and_then(|s| m.creature(s))
                .map(|c| c.movement_type);
            (linked, spirit)
        })
        .await
        .unwrap();
    assert_eq!(squad, 4, "each soldier at the commander's side");
    assert_eq!(
        spirit,
        Some(crate::game::map::creature::MOVEMENT_WAYPOINT),
        "the spirit walks its waypoints"
    );
}

/// The real valley: its NPCs' fighting is Thistle Tea's, their routes are
/// there, the beacons' plantings and the altars are vmangos's.
#[test]
#[ignore = "extended"]
fn the_real_valley_is_scripted_as_thistle_tea_scripts_it() {
    let dir = &crate::game::harness::real_data("db");
    if !dir.join("vmangos.sqlite").exists() {
        return;
    }
    let data = crate::game::harness::load_real(dir);
    for entry in [
        11948, 11946, 13236, 13256, 13419, 13284, 14943, 13441, 14762,
    ] {
        let events = data.scripts.ai_events.get(&entry).expect("its events");
        assert!(
            events.iter().all(|e| e.id > 91_000_000),
            "{entry}'s are Thistle Tea's"
        );
    }
    for (entry, points) in [
        (13236, 45),
        (13440, 81),
        (13576, 93),
        (13524, 50),
        (13531, 55),
    ] {
        assert_eq!(
            data.bg_routes.get(&entry).map_or(0, Vec::len),
            points,
            "{entry}"
        );
    }
    let plant = data.spells.get(21728).unwrap();
    assert!(bg::plants(plant));
    assert_eq!(
        (plant.effects[0].effect, plant.effects[0].misc_value),
        (50, 178726)
    );
    for altar in [178465, 178670] {
        assert_eq!(
            data.gameobject_templates[&altar].data[0], 10,
            "ten to summon"
        );
    }
}

#[tokio::test]
async fn alice_takes_a_fallen_horde_players_insignia_and_his_corpse_is_bones() {
    let mut d = valley();
    let mut s = crate::game::data::spells::SpellTemplate {
        id: 22027,
        name: "Remove Insignia".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0].effect = bg::insignia::EFFECT_SKIN_PLAYER_CORPSE;
    d.spells.spells.insert(22027, s);
    let realm = harness::realm_with(d);
    let (mut a, alice, mut b, grom, map) = pair(&realm).await;
    let at = map
        .query(move |m| m.players[&alice].position())
        .await
        .unwrap();
    put(&map, &mut b, at.x + 2.0, at.y, at.z).await;
    // Grom falls: his is a body with insignia to take.
    map.query(move |m| {
        m.players
            .get_mut(&grom)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_LEVEL, 60);
        bg::died(m, grom, alice);
        crate::game::map::death::kill(m, grom);
    })
    .await;
    let skinnable = map
        .query(move |m| {
            m.players[&grom]
                .fields
                .has_flag(UNIT_FIELD_FLAGS, bg::insignia::UNIT_FLAG_SKINNABLE)
        })
        .await
        .unwrap();
    assert!(skinnable);
    // Alice takes it: he is made to release, and his corpse is bones she
    // finds gold on.
    a.clear();
    map.query(move |m| crate::game::map::spells::cast_triggered(m, alice, 22027, Some(grom)))
        .await;
    let frame = a.wait_frame("SMSG_LOOT_RESPONSE", 0x160, |_| true).await;
    assert!(frame.len() > 13);
    let (ghost, corpse, bones) = map
        .query(move |m| {
            let p = &m.players[&grom];
            let bones: Vec<(u32, u32)> = m
                .corpses
                .values()
                .map(|c| {
                    (
                        c.fields.get_u32(CORPSE_FIELD_FLAGS),
                        c.fields.get_u32(CORPSE_FIELD_DYNAMIC_FLAGS),
                    )
                })
                .collect();
            (
                matches!(p.death, crate::game::map::death::Death::Ghost { .. }),
                p.row.state.corpse.is_some(),
                bones,
            )
        })
        .await
        .unwrap();
    assert!(ghost, "made to release");
    assert!(!corpse, "no corpse to take back");
    assert_eq!(bones, vec![(0x05, 0x01)]);
    // They lie an hour, and, a battleground copy's, aren't kept across a
    // restart.
    let left: Vec<Duration> = map
        .query(|m| {
            m.bones
                .iter()
                .map(|(t, _)| t.saturating_duration_since(m.now))
                .collect()
        })
        .await
        .unwrap();
    let hour = Duration::from_secs(3600);
    assert!(
        left.len() == 1 && left[0] <= hour && left[0] > hour - Duration::from_secs(30),
        "{left:?}"
    );
    assert!(realm.store.bones_for_test().await.is_empty(), "not kept");
    b.wait_for("SMSG_PLAYER_SKINNED", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_PLAYER_SKINNED(_)).then_some(())
    })
    .await;
    let before = map.query(move |m| m.players[&alice].money()).await.unwrap();
    a.send(CMSG_LOOT_MONEY {}).await;
    until(&map, move |m| m.players[&alice].money() > before).await;
    let after = map.query(move |m| m.players[&alice].money()).await.unwrap();
    assert!(after - before >= bg::insignia::gold(60, 50));
}

#[tokio::test]
async fn a_released_players_corpse_still_has_its_insignia_for_the_enemy() {
    let mut d = valley();
    let mut s = crate::game::data::spells::SpellTemplate {
        id: 22027,
        name: "Remove Insignia".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0].effect = bg::insignia::EFFECT_SKIN_PLAYER_CORPSE;
    d.spells.spells.insert(22027, s);
    let realm = harness::realm_with(d);
    let (mut a, alice, _b, grom, map) = pair(&realm).await;
    // Grom falls and releases; his corpse lies where he fell.
    let corpse = map
        .query(move |m| {
            bg::died(m, grom, alice);
            crate::game::map::death::kill(m, grom);
            crate::game::map::death::release(m, grom);
            m.corpses
                .values()
                .find(|c| c.owner == grom)
                .map(|c| (c.guid, c.position, c.fields.get_u32(CORPSE_FIELD_FLAGS)))
        })
        .await
        .flatten();
    let (corpse, at, flags) = corpse.expect("his corpse");
    assert_eq!(flags, 0x24, "with insignia");
    put(&map, &mut a, at.x + 1.0, at.y, at.z).await;
    a.clear();
    map.query(move |m| crate::game::map::spells::cast_triggered(m, alice, 22027, Some(corpse)))
        .await;
    a.wait_frame("SMSG_LOOT_RESPONSE", 0x160, |_| true).await;
    let left = map
        .query(move |m| {
            (
                m.corpses.contains_key(&corpse),
                m.corpses
                    .values()
                    .filter(|c| c.fields.get_u32(CORPSE_FIELD_FLAGS) == 0x05)
                    .count(),
            )
        })
        .await
        .unwrap();
    assert_eq!(left, (false, 1), "bones in its place");
}

#[test]
fn insignia_gold_grows_with_the_fallens_level() {
    use crate::game::map::battleground::insignia::gold;
    assert_eq!(gold(1, 150), 0);
    assert_eq!(gold(60, 100), 560);
    assert!(gold(30, 100) < gold(60, 100));
}
