//! Conditions where Thistle Tea evaluates them (loot, vendors, quests,
//! spell areas, reputation, escorts), the script commands added since, and
//! pooled nodes that swap on respawn.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::*;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const MERCHANT: u32 = 3000;
const BOAR: u32 = 3001;
/// Level at least 5: a fresh character fails it.
const LEVEL_FIVE: u32 = 1;
/// Level at least 1: anyone meets it.
const LEVEL_ONE: u32 = 2;

fn template(entry: u32, name: &str, faction: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        display_scales: [1.0, 0.0, 0.0, 0.0],
        level_min: 2,
        level_max: 2,
        faction,
        unit_class: 1,
        health_max: 50,
        ..Default::default()
    }
}

fn item(entry: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000,
        buy_price: 10,
        stackable: 1,
        ..Default::default()
    }
}

fn level_condition(entry: u32, level: i32) -> Condition {
    Condition {
        entry,
        kind: 15,
        values: [level, 1, 0, 0],
        flags: 0,
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    let mut merchant = template(MERCHANT, "Merchant", 35);
    merchant.npc_flags = crate::game::map::npc::NPC_FLAG_VENDOR | 0x2;
    d.creature_templates.insert(MERCHANT, merchant);
    let mut boar = template(BOAR, "Boar", 14);
    boar.loot_id = 77;
    d.creature_templates.insert(BOAR, boar);
    d.creature_spawns.insert(
        0,
        vec![
            CreatureSpawn {
                guid: 1,
                entries: vec![MERCHANT],
                map: 0,
                x: START.0 + 2.0,
                y: START.1,
                z: START.2,
                ..Default::default()
            },
            CreatureSpawn {
                guid: 2,
                entries: vec![BOAR],
                map: 0,
                x: START.0 - 2.0,
                y: START.1,
                z: START.2,
                ..Default::default()
            },
        ],
    );
    for e in [100, 101, 102, 103] {
        d.items.insert(e, item(e));
    }
    let vendor = |slot, item, condition| VendorItem {
        slot,
        item,
        condition,
        ..Default::default()
    };
    d.vendors.insert(
        MERCHANT,
        vec![
            vendor(0, 100, 0),
            vendor(1, 101, LEVEL_FIVE),
            vendor(2, 102, LEVEL_ONE),
        ],
    );
    let drop = |item, condition| LootEntry {
        item,
        chance: 100.0,
        min: 1,
        max: 1,
        condition,
        ..Default::default()
    };
    d.loot.creature.insert(
        77,
        vec![drop(100, 0), drop(101, LEVEL_FIVE), drop(103, LEVEL_ONE)],
    );
    d.scripts
        .conditions
        .insert(LEVEL_FIVE, level_condition(LEVEL_FIVE, 5));
    d.scripts
        .conditions
        .insert(LEVEL_ONE, level_condition(LEVEL_ONE, 1));
    d
}

async fn enter(realm: &std::sync::Arc<crate::game::Realm>) -> (Client, Guid) {
    let mut a = Client::connect(realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    (a, me)
}

#[tokio::test]
async fn a_vendor_shows_only_what_its_conditions_allow() {
    let realm = harness::realm_with(data());
    let (mut a, _me) = enter(&realm).await;
    let merchant = guid::unit(MERCHANT, 1);
    a.sees(merchant).await;
    a.send(CMSG_LIST_INVENTORY { guid: merchant }).await;
    let items = a
        .wait_for("SMSG_LIST_INVENTORY", |m| match m {
            ServerOpcodeMessage::SMSG_LIST_INVENTORY(l) => {
                Some(l.items.iter().map(|i| i.item).collect::<Vec<_>>())
            }
            _ => None,
        })
        .await;
    assert_eq!(items, vec![100, 102], "the level 5 item is held back");
}

#[tokio::test]
async fn loot_with_a_condition_shows_only_to_those_who_meet_it() {
    let realm = harness::realm_with(data());
    let (mut a, me) = enter(&realm).await;
    let boar = guid::unit(BOAR, 2);
    a.sees(boar).await;
    let map = realm.registry.online(me).unwrap().map();
    let shown = map
        .query(move |m| {
            crate::game::map::combat::deal_damage(m, me, boar, 1000);
            let loot = crate::game::map::loot::loot_ref(m, boar)?;
            let met = crate::game::map::loot::conditions_met(m, loot, boar, me);
            let frame = crate::game::map::loot::response(&m.realm.data, boar, loot, me, &[], &met);
            // Header, guid, kind, gold, then the count.
            Some((loot.items.len(), frame[4 + 8 + 1 + 4]))
        })
        .await
        .flatten();
    assert_eq!(shown, Some((3, 2)), "three drop, two are shown");
}

#[tokio::test]
async fn a_quests_required_condition_gates_it() {
    let mut d = data();
    for (id, condition) in [(400, LEVEL_FIVE), (401, LEVEL_ONE)] {
        d.npc.quests.insert(
            id,
            crate::game::data::npc::Quest {
                id,
                title: format!("Quest {id}"),
                ..Default::default()
            },
        );
        d.scripts.quest_condition.insert(id, condition);
    }
    d.npc.creature_starts.insert(MERCHANT, vec![400, 401]);
    let realm = harness::realm_with(d);
    let (mut a, me) = enter(&realm).await;
    let merchant = guid::unit(MERCHANT, 1);
    a.sees(merchant).await;
    let map = realm.registry.online(me).unwrap().map();
    let offered = map
        .query(move |m| {
            crate::game::map::quests::giver_quests(m, me, merchant)
                .iter()
                .map(|q| q.quest_id)
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    assert_eq!(offered, vec![401]);
}

#[tokio::test]
async fn a_spell_area_aura_comes_and_goes_with_its_quest() {
    let mut d = data();
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura: 29,
        base_points: 4,
        base_dice: 1,
        misc_value: 0,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(
        6000,
        SpellTemplate {
            id: 6000,
            effects,
            duration_index: 0,
            name: "Blessing of the Place".into(),
            ..Default::default()
        },
    );
    d.npc.quests.insert(
        300,
        crate::game::data::npc::Quest {
            id: 300,
            title: "Done Here".into(),
            ..Default::default()
        },
    );
    d.scripts.spell_areas.push(SpellArea {
        spell: 6000,
        quest_end: 300,
        gender: 2,
        autocast: true,
        ..Default::default()
    });
    let realm = harness::realm_with(d);
    let (mut a, me) = enter(&realm).await;
    a.sees(me).await;
    let map = realm.registry.online(me).unwrap().map();
    let has = |map: crate::game::map::MapHandle| async move {
        map.query(move |m| m.players[&me].auras.has_spell(6000))
            .await
            .unwrap()
    };
    assert!(has(map.clone()).await, "cast at login");
    let merchant = guid::unit(MERCHANT, 1);
    map.call(move |m| {
        if let Some(p) = m.players.get_mut(&me) {
            p.journal.log.push(crate::game::store::Logged {
                quest: 300,
                ..Default::default()
            });
        }
        crate::game::map::quests::choose_reward(m, me, merchant, 300, 0);
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!has(map.clone()).await, "gone with the quest done");
    let refused = map
        .query(move |m| !crate::game::map::spell_areas::allowed(m, me, 6000))
        .await
        .unwrap();
    assert!(refused, "nor castable now");
}

#[tokio::test]
async fn reputation_and_escort_conditions_read_the_world() {
    let mut d = data();
    // Friendly or better with Stormwind (72); a neutral start fails it.
    d.scripts.conditions.insert(
        10,
        Condition {
            entry: 10,
            kind: 5,
            values: [72, 4, 0, 0],
            flags: 0,
        },
    );
    // At most neutral: it holds.
    d.scripts.conditions.insert(
        11,
        Condition {
            entry: 11,
            kind: 30,
            values: [72, 3, 0, 0],
            flags: 0,
        },
    );
    // An escort fails with its target dead (0x2) or 5 yards off.
    d.scripts.conditions.insert(
        12,
        Condition {
            entry: 12,
            kind: 25,
            values: [0x2, 5, 0, 0],
            flags: 0,
        },
    );
    let realm = harness::realm_with(d);
    let (mut a, me) = enter(&realm).await;
    let merchant = guid::unit(MERCHANT, 1);
    a.sees(merchant).await;
    let map = realm.registry.online(me).unwrap().map();
    let (friendly, at_most, near, far) = map
        .query(move |m| {
            let c = |m: &crate::game::map::Map, id| {
                crate::game::map::conditions::met(m, id, Some(merchant), Some(me))
            };
            let (friendly, at_most, near) = (c(m, 10), c(m, 11), c(m, 12));
            if let Some(cr) = m.creature_mut(merchant) {
                cr.position.x += 20.0;
            }
            (friendly, at_most, near, c(m, 12))
        })
        .await
        .unwrap();
    assert!(!friendly);
    assert!(at_most);
    assert!(!near, "close and alive: the escort holds");
    assert!(far, "20 yards off: it fails");
}

#[tokio::test]
async fn the_added_script_commands_do_their_work() {
    let realm = harness::realm_with(data());
    let (mut a, me) = enter(&realm).await;
    let merchant = guid::unit(MERCHANT, 1);
    a.sees(merchant).await;
    let map = realm.registry.online(me).unwrap().map();
    let step = |command, datalong: [u32; 4]| ScriptStep {
        command,
        datalong,
        ..Default::default()
    };
    let (cooled, cleared, pvp) = map
        .query(move |m| {
            // The source's cooldowns.
            crate::game::map::scripts::execute(m, &step(57, [133, 60, 0, 0]), me, merchant);
            let cooled = m.players[&me].caster.cooldowns.contains_key(&133);
            crate::game::map::scripts::execute(m, &step(58, [133, 0, 0, 0]), me, merchant);
            let cleared = !m.players[&me].caster.cooldowns.contains_key(&133);
            crate::game::map::scripts::execute(m, &step(86, [1, 0, 0, 0]), merchant, me);
            let pvp = m.players[&me]
                .fields
                .has_flag(UNIT_FIELD_FLAGS, crate::game::map::player::UNIT_FLAG_PVP);
            (cooled, cleared, pvp)
        })
        .await
        .unwrap();
    assert!(cooled && cleared && pvp);
    a.wait_for("SMSG_CLEAR_COOLDOWN", |m| match m {
        ServerOpcodeMessage::SMSG_CLEAR_COOLDOWN(c) => (c.id == 133).then_some(()),
        _ => None,
    })
    .await;
    // Every command the world database uses but a handful is run.
    for c in [21, 30, 36, 38, 53, 57, 58, 72, 75, 86, 88] {
        assert!(crate::game::map::scripts::command_supported(c), "{c}");
    }
}

#[tokio::test]
async fn a_pooled_node_comes_back_as_one_of_its_pool() {
    let mut d = GameData::empty();
    d.gameobject_templates.insert(
        500,
        GameObjectTemplate {
            entry: 500,
            kind: 3,
            display_id: 1000,
            name: "Peacebloom".into(),
            size: 1.0,
            ..Default::default()
        },
    );
    d.gameobject_spawns.insert(
        0,
        (1..=4)
            .map(|g| GameObjectSpawn {
                guid: g,
                entry: 500,
                map: 0,
                x: START.0 + g as f32 * 3.0,
                y: START.1,
                z: START.2,
                rotation: [0.0, 0.0, 0.0, 1.0],
                state: 1,
                respawn_min: 1,
                ..Default::default()
            })
            .collect(),
    );
    d.scripts.pools.insert(
        9,
        Pool {
            entry: 9,
            max_limit: 1,
            objects: (1..=4).map(|g| (g, 0.0)).collect(),
            ..Default::default()
        },
    );
    for g in 1..=4 {
        d.scripts.object_pool.insert(g, 9);
    }
    let realm = harness::realm_with(d);
    let (_a, me) = enter(&realm).await;
    let map = realm.registry.online(me).unwrap().map();
    let up = |map: crate::game::map::MapHandle| async move {
        map.query(|m| {
            m.gameobjects
                .iter()
                .filter(|(_, o)| o.entry == 500)
                .filter_map(|(_, o)| o.spawn_guid)
                .collect::<Vec<_>>()
        })
        .await
        .unwrap()
    };
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..12 {
        let now = up(map.clone()).await;
        assert_eq!(now.len(), 1, "one of the pool at a time: {now:?}");
        seen.insert(now[0]);
        // Gathered: gone, and due back at once.
        map.query(move |m| {
            let g = m
                .gameobjects
                .iter()
                .find(|(_, o)| o.entry == 500)
                .map(|(_, o)| (o.guid, o.spawn_guid.unwrap()));
            if let Some((g, spawn)) = g {
                m.remove_gameobject(g);
                m.loot_state.respawns.push((m.now, spawn));
            }
        })
        .await;
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    assert!(seen.len() > 1, "it came back elsewhere: {seen:?}");
}

/// The real data: Thistle Tea's creature scripts all come in, and the
/// bosses' encounters with them.
#[test]
fn the_real_thistle_tea_scripts_come_in_whole() {
    let dir = std::path::Path::new("/local/home/wallenx/runs/tt-data/db");
    if !dir.join("vmangos.sqlite").exists() {
        return;
    }
    let mut data = crate::game::data::load::load(dir);
    // Onyxia, Ragnaros, Majordomo, Garr, Shazzrah, Omokk, Gandling.
    for entry in [10184, 11502, 12018, 12057, 12264, 9196, 1853] {
        assert!(
            data.scripts
                .ai_events
                .get(&entry)
                .is_some_and(|e| !e.is_empty()),
            "{entry}"
        );
    }
    // Every action names a script, and every step's sub-script one too.
    let events: Vec<AiEvent> = data.scripts.ai_events.values().flatten().cloned().collect();
    for e in events.iter().filter(|e| e.id > 100_000) {
        for a in e.actions.iter().filter(|a| **a != 0) {
            assert!(data.scripts.ai_scripts.contains_key(a), "{e:?}");
        }
    }
    assert!(
        data.scripts.routes.len() >= 40,
        "{}",
        data.scripts.routes.len()
    );
    let left = crate::game::data::tt_scripts::install(&mut data);
    assert!(left.is_empty(), "left out: {left:?}");
}

/// A raid of `map` to send a player into.
fn raid(d: &mut GameData, map: u32, name: &str) {
    d.maps.insert(
        map,
        MapInfo {
            id: map,
            internal_name: name.into(),
            name: name.into(),
            instance_type: 2,
            player_limit: 0,
        },
    );
}

/// Send `c` to `at` in `map`'s copy, and make them hard to kill.
async fn go_to(
    realm: &std::sync::Arc<crate::game::Realm>,
    c: &mut Client,
    map: u32,
    at: (f32, f32, f32),
) {
    let g = c.guid;
    let from = realm.registry.online(g).unwrap().map();
    from.call(move |m| {
        m.teleport(
            g,
            crate::game::map::MapKey::world(map),
            Vector3d {
                x: at.0,
                y: at.1,
                z: at.2,
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
    let to = realm.registry.online(g).unwrap().map();
    to.call(move |m| {
        if let Some(p) = m.players.get_mut(&g) {
            p.fields.set_u32(UNIT_FIELD_MAXHEALTH, 10_000_000);
            p.fields.set_u32(UNIT_FIELD_HEALTH, 10_000_000);
        }
    });
}

const ONYXIA: u32 = 10184;

/// Onyxia's flight, as Thistle Tea scripts it: at 65 percent she flies to
/// the middle of her lair, lifts off and circles to her first point; at 40
/// she lands and fights on the ground again.
#[tokio::test]
async fn onyxia_takes_to_the_air_and_lands_again() {
    let mut d = GameData::empty();
    raid(&mut d, 249, "OnyxiaLairInstance");
    let mut onyxia = template(ONYXIA, "Onyxia", 14);
    onyxia.health_max = 100_000;
    onyxia.level_min = 63;
    onyxia.level_max = 63;
    // Fast, so the flight is over in moments.
    onyxia.speed_run = 12.0;
    d.creature_templates.insert(ONYXIA, onyxia);
    d.creature_templates
        .insert(11262, template(11262, "Onyxian Whelp", 14));
    d.creature_spawns.insert(
        249,
        vec![CreatureSpawn {
            guid: 47572,
            entries: vec![ONYXIA],
            map: 249,
            x: -4.8689,
            y: -217.171,
            z: -86.7104,
            orientation: std::f32::consts::PI,
            ..Default::default()
        }],
    );
    // Here, without the world database's escort paths.
    let left = crate::game::data::tt_scripts::install(&mut d);
    assert!(left.keys().all(|k| k.starts_with("route of")), "{left:?}");
    let realm = harness::realm_with(d);
    let (mut a, me) = enter(&realm).await;
    go_to(&realm, &mut a, 249, (-10.0, -217.0, -86.7)).await;
    let map = realm.registry.online(me).unwrap().map();
    let onyxia = map
        .query(|m| {
            m.creatures
                .iter()
                .find(|(_, c)| c.entry == ONYXIA)
                .map(|(_, c)| c.guid)
        })
        .await
        .flatten()
        .expect("Onyxia");
    let state = |map: crate::game::map::MapHandle| async move {
        map.query(move |m| {
            let c = m.creature(onyxia).unwrap();
            (
                crate::game::map::event_ai::phase(m, onyxia),
                c.position,
                crate::game::map::event_ai::scripted(m, onyxia).is_some_and(|s| s.fly),
                c.combat.attacking,
            )
        })
        .await
        .unwrap()
    };
    map.call(move |m| {
        crate::game::map::ai::attacked(m, onyxia, me);
        let f = m.unit_fields_mut(onyxia).unwrap();
        let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
        f.set_u32(UNIT_FIELD_HEALTH, max * 64 / 100);
    });
    // Off to the middle, up, and round to her first point (7).
    let mut circled = false;
    for _ in 0..80 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (phase, _, fly, fighting) = state(map.clone()).await;
        assert!(fighting, "she fights on in the air");
        if phase == 17 {
            assert!(fly, "flying");
            circled = true;
            break;
        }
    }
    assert!(circled, "at point 7: {:?}", state(map.clone()).await);
    let (_, at, _, _) = state(map.clone()).await;
    assert!(
        crate::game::map::combat::dist(
            at,
            Vector3d {
                x: 22.8763,
                y: -217.152,
                z: -65.0548
            }
        ) < 3.0,
        "{at:?}"
    );
    // At 40 percent she comes down, on the north side from point 7.
    map.call(move |m| {
        let f = m.unit_fields_mut(onyxia).unwrap();
        let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
        f.set_u32(UNIT_FIELD_HEALTH, max * 39 / 100);
    });
    let mut landed = false;
    for _ in 0..80 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (phase, _, fly, _) = state(map.clone()).await;
        if phase == 5 {
            assert!(!fly, "on the ground");
            landed = true;
            break;
        }
    }
    assert!(landed, "landed: {:?}", state(map.clone()).await);
    assert!(
        map.query(move |m| crate::game::map::event_ai::combat_movement(m, onyxia))
            .await
            .unwrap(),
        "she chases again"
    );
}

const MAJORDOMO: u32 = 12018;

/// Majordomo Executus calls his guard when he comes; each of them that
/// falls takes his fight a phase on, and when the last is down he yields,
/// settles the encounter, and turns friendly.
#[tokio::test]
async fn majordomo_yields_when_his_guard_is_dead() {
    let mut d = GameData::empty();
    raid(&mut d, 409, "MoltenCore");
    d.scripts.map_templates.insert(
        409,
        MapTemplate {
            map: 409,
            map_type: 2,
            script_name: "instance_molten_core".into(),
            ..Default::default()
        },
    );
    let mut majordomo = template(MAJORDOMO, "Majordomo Executus", 14);
    majordomo.health_max = 100_000;
    d.creature_templates.insert(MAJORDOMO, majordomo);
    for e in [11663, 11664] {
        d.creature_templates
            .insert(e, template(e, "Flamewaker", 14));
    }
    d.creature_spawns.insert(
        409,
        vec![CreatureSpawn {
            guid: 56500,
            entries: vec![MAJORDOMO],
            map: 409,
            x: 758.089,
            y: -1176.71,
            z: -118.64,
            orientation: 3.12414,
            ..Default::default()
        }],
    );
    let _ = crate::game::data::tt_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (mut a, me) = enter(&realm).await;
    go_to(&realm, &mut a, 409, (750.0, -1176.0, -118.6)).await;
    let map = realm.registry.online(me).unwrap().map();
    let guard = |map: crate::game::map::MapHandle| async move {
        map.query(|m| {
            m.creatures
                .iter()
                .filter(|(_, c)| matches!(c.entry, 11663 | 11664) && c.alive())
                .map(|(_, c)| c.guid)
                .collect::<Vec<_>>()
        })
        .await
        .unwrap()
    };
    let mut guards = Vec::new();
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        guards = guard(map.clone()).await;
        if guards.len() == 8 {
            break;
        }
    }
    assert_eq!(guards.len(), 8, "his guard comes with him");
    let majordomo = map
        .query(|m| {
            m.creatures
                .iter()
                .find(|(_, c)| c.entry == MAJORDOMO)
                .map(|(_, c)| c.guid)
        })
        .await
        .flatten()
        .unwrap();
    // They fight as one.
    let first = guards[0];
    map.call(move |m| crate::game::map::ai::attacked(m, first, me));
    tokio::time::sleep(Duration::from_millis(200)).await;
    let fighting = map
        .query(move |m| m.creature(majordomo).unwrap().combat.attacking)
        .await
        .unwrap();
    assert!(fighting, "Majordomo joins his guard's fight");
    for (i, g) in guards.iter().copied().enumerate() {
        map.call(move |m| crate::game::map::combat::deal_damage(m, me, g, 1_000_000));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let phase = map
            .query(move |m| crate::game::map::event_ai::phase(m, majordomo))
            .await
            .unwrap();
        assert_eq!(phase as usize, i + 1, "a phase for each that falls");
    }
    let settled = map
        .query(|m| crate::game::map::instance_script::get_data(m, 8))
        .await
        .unwrap();
    assert_eq!(settled, 3, "the encounter is done");
    let mut friendly = false;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        friendly = map
            .query(move |m| {
                m.creature(majordomo)
                    .is_some_and(|c| c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE) == 1080)
            })
            .await
            .unwrap();
        if friendly {
            break;
        }
    }
    assert!(friendly, "he yields");
}
