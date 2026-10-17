//! The world around the player through the protocol: world states,
//! languages, events, item mechanics, timed and escort quests, points of
//! interest, flights resumed, the invasion.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::npc::{GossipOption, Quest};
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::world::{EventCreature, Poi, ScriptPoint};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, MapKey};
use crate::game::store::BAG_PLAYER;

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

const LANGUAGE_COMMON: u32 = 668;
const LANGUAGE_ORCISH: u32 = 669;
const CURSE_OF_TONGUES: u32 = 900_100;
const BOP_RING: u32 = 2100;
const BOE_CLOAK: u32 = 2101;
const POTION: u32 = 2102;
const WAND: u32 = 2103;
const CONJURED_BREAD: u32 = 2104;
const TIMED: u32 = 2105;
const KEY: u32 = 2106;
const QUIVER: u32 = 2107;
const ARROW: u32 = 2108;
const HEAL_SPELL: u32 = 900_101;
const ESCORTEE: u32 = 3584;
const ESCORT_QUEST: u32 = 945;
const GIVER: u32 = 3010;
const TIMED_QUEST: u32 = 610;
const EVENT_QUEST: u32 = 611;

fn spell(id: u32, effect: u32, misc: i32, aura: u32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect,
        misc_value: misc,
        aura,
        target_a: 1,
        ..Default::default()
    };
    s
}

fn item(entry: u32, class: u32, subclass: u32, inventory_type: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        class,
        subclass,
        inventory_type,
        name: format!("Item {entry}"),
        stackable: 1,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    for s in [
        spell(LANGUAGE_COMMON, 39, 7, 0),
        spell(LANGUAGE_ORCISH, 39, 1, 0),
        spell(CURSE_OF_TONGUES, 6, 1, 75),
        spell(HEAL_SPELL, 10, 0, 0),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    let mut ring = item(BOP_RING, 4, 0, 11);
    ring.bonding = 1;
    let mut cloak = item(BOE_CLOAK, 4, 1, 16);
    cloak.bonding = 2;
    let mut potion = item(POTION, 0, 1, 0);
    potion.stackable = 5;
    potion.spells[0] = crate::game::data::items::ItemSpell {
        id: HEAL_SPELL,
        trigger: 0,
        charges: -1,
        cooldown: -1,
        category: 4,
        category_cooldown: 120_000,
        ..Default::default()
    };
    let mut wand = item(WAND, 0, 0, 0);
    wand.spells[0] = crate::game::data::items::ItemSpell {
        id: HEAL_SPELL,
        trigger: 0,
        charges: 2,
        cooldown: -1,
        ..Default::default()
    };
    let mut bread = item(CONJURED_BREAD, 0, 0, 0);
    bread.flags = 0x2;
    let mut timed = item(TIMED, 0, 0, 0);
    timed.duration = 2;
    let mut key = item(KEY, 13, 0, 0);
    key.bag_family = 9;
    let mut quiver = item(QUIVER, 11, 2, 18);
    quiver.container_slots = 8;
    quiver.bag_family = 1;
    let mut arrow = item(ARROW, 6, 2, 24);
    arrow.bag_family = 1;
    arrow.stackable = 200;
    for t in [ring, cloak, potion, wand, bread, timed, key, quiver, arrow] {
        d.items.insert(t.entry, t);
    }
    // A quest giver near the start, with a timed quest and an event's, and
    // the escortee of an escort quest beside it.
    let npc = |entry, name: &str, npc_flags, gossip_menu_id| CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 10,
        level_max: 10,
        faction: 35,
        npc_flags,
        gossip_menu_id,
        unit_class: 1,
        health_max: 500,
        speed_walk: 1.0,
        speed_run: 1.0,
        ..Default::default()
    };
    d.creature_templates
        .insert(GIVER, npc(GIVER, "Giver", 0x1 | 0x2, 50));
    d.creature_templates
        .insert(ESCORTEE, npc(ESCORTEE, "Escortee", 0x2, 0));
    d.creature_templates.insert(3011, npc(3011, "Dressed Up", 0, 0));
    let spawn = |guid, entry, dx: f32| CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 300,
        respawn_max: 300,
        ..Default::default()
    };
    d.creature_spawns.insert(
        0,
        vec![spawn(30, GIVER, 1.5), spawn(31, ESCORTEE, -1.5), spawn(32, 3011, 3.0)],
    );
    d.world.event_creatures.insert(
        32,
        vec![EventCreature {
            event: 9,
            display: 4321,
            ..Default::default()
        }],
    );
    let quest = |id, limit_time, flags| Quest {
        id,
        title: format!("Quest {id}"),
        min_level: 1,
        level: 1,
        limit_time,
        flags,
        ..Default::default()
    };
    d.npc.quests.insert(TIMED_QUEST, quest(TIMED_QUEST, 1, 0));
    d.npc.quests.insert(EVENT_QUEST, quest(EVENT_QUEST, 0, 0));
    d.npc.quests.insert(ESCORT_QUEST, quest(ESCORT_QUEST, 0, 0x4));
    d.npc
        .creature_starts
        .insert(GIVER, vec![TIMED_QUEST, EVENT_QUEST]);
    d.npc.creature_starts.insert(ESCORTEE, vec![ESCORT_QUEST]);
    d.world.event_quests.insert(EVENT_QUEST, 9);
    // The escort's path: twenty yards east in two-yard steps.
    d.world.script_waypoints.insert(
        ESCORTEE,
        (0..20)
            .map(|i| ScriptPoint {
                point: i,
                x: START.0 - 1.5 + 2.0 * (i as f32 + 1.0),
                y: START.1 + 2.0,
                z: START.2,
                wait_ms: 0,
            })
            .collect(),
    );
    // A gossip option that marks the inn.
    d.npc.gossip_menus.insert(
        50,
        crate::game::data::npc::GossipMenu {
            texts: Vec::new(),
            options: vec![GossipOption {
                id: 0,
                icon: 0,
                text: "Where is the inn?".into(),
                kind: 1,
                action_poi: 7,
                ..Default::default()
            }],
        },
    );
    d.world.pois.insert(
        7,
        Poi {
            x: -9459.0,
            y: 42.0,
            icon: 7,
            flags: 99,
            data: 0,
            name: "Lion's Pride Inn".into(),
        },
    );
    d
}

async fn alone(name: &str) -> (std::sync::Arc<crate::game::Realm>, Client, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, name);
    a.enter(name, Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    (realm, a, map)
}

#[tokio::test]
async fn world_states_come_on_login_and_with_each_zone() {
    let realm = harness::realm_with(data());
    crate::game::map::world_states::set_global(&realm, 2219, 42);
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let has = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::SMSG_INIT_WORLD_STATES(w) => w
            .states
            .iter()
            .any(|s| s.state == 2219 && s.value == 42)
            .then_some(w.area),
        _ => None,
    };
    a.wait_for("the login's world states", has).await;
    a.send(CMSG_ZONEUPDATE { area: Area::Westfall0 }).await;
    let area = a.wait_for("a new zone's world states", has).await;
    assert_eq!(area, Area::Westfall0);
    // A change goes out to everyone.
    crate::game::map::world_states::set_global(&realm, 2219, 43);
    a.wait_for("SMSG_UPDATE_WORLD_STATE", |m| match m {
        ServerOpcodeMessage::SMSG_UPDATE_WORLD_STATE(u) if u.state.value == 43 => Some(()),
        _ => None,
    })
    .await;
}

fn said(m: &ServerOpcodeMessage) -> Option<(Language, String)> {
    match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c) if matches!(c.chat_type, SMSG_MESSAGECHAT_ChatType::Say { .. }) => {
            Some((c.language, c.message.clone()))
        }
        _ => None,
    }
}

#[tokio::test]
async fn players_speak_the_languages_they_know() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(ga).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    // Only GMs speak whatever they like; Alice is a player who knows Common.
    map.query(move |m| {
        if let Some(p) = m.players.get_mut(&ga) {
            p.gm_level = 0;
            p.spells.retain(|s| *s != LANGUAGE_ORCISH);
            if !p.spells.contains(&LANGUAGE_COMMON) {
                p.spells.push(LANGUAGE_COMMON);
            }
        }
    })
    .await;
    let say = |language| CMSG_MESSAGECHAT {
        chat_type: CMSG_MESSAGECHAT_ChatType::Say,
        language,
        message: format!("in {language:?}"),
    };
    a.send(say(Language::Orcish)).await;
    a.send(say(Language::Common)).await;
    let heard = b.wait_for("Alice's Common", said).await;
    assert_eq!(heard, (Language::Common, "in Common".to_owned()), "her Orcish isn't heard");
    // Cursed, she speaks Orcish whatever she chooses.
    map.query(move |m| {
        let s = m.realm.data.spells.get(CURSE_OF_TONGUES).cloned().unwrap();
        crate::game::map::auras::apply(m, gb, ga, &s, 0, 0);
    })
    .await;
    a.send(say(Language::Common)).await;
    let cursed = b.wait_for("Alice, cursed", said).await;
    assert_eq!(cursed.0, Language::Orcish);
}

#[test]
fn the_darkmoon_faire_keeps_its_calendar() {
    use chrono::NaiveDate;
    let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
    // October 2026: the first Monday is the 5th; Elwynn in even months.
    assert_eq!(crate::game::events::darkmoon(day(2026, 10, 1)), None);
    assert_eq!(crate::game::events::darkmoon(day(2026, 10, 2)), Some(23));
    assert_eq!(crate::game::events::darkmoon(day(2026, 10, 5)), Some(4));
    assert_eq!(crate::game::events::darkmoon(day(2026, 10, 11)), Some(4));
    assert_eq!(crate::game::events::darkmoon(day(2026, 10, 12)), None);
    // November: Mulgore, opening the 2nd, built from the 1st.
    assert_eq!(crate::game::events::darkmoon(day(2026, 11, 1)), Some(24));
    assert_eq!(crate::game::events::darkmoon(day(2026, 11, 3)), Some(5));
    // Fireworks on a holiday's night, for the first ten minutes.
    let at = |h, m| {
        chrono::NaiveDate::from_ymd_opt(2026, 12, 31)
            .unwrap()
            .and_hms_opt(h, m, 0)
            .unwrap()
            .and_utc()
            .timestamp()
    };
    let new_year: ahash::AHashSet<u32> = [34].into_iter().collect();
    assert!(crate::game::events::rules(at(23, 5), &new_year).contains(&6));
    assert!(!crate::game::events::rules(at(23, 15), &new_year).contains(&6));
    assert!(crate::game::events::rules(at(23, 15), &new_year).contains(&39));
    assert!(!crate::game::events::rules(at(12, 5), &new_year).contains(&6));
    assert!(!crate::game::events::rules(at(23, 5), &Default::default()).contains(&6));
}

#[tokio::test]
async fn an_event_brings_its_quest_and_its_looks() {
    let (realm, a, map) = alone("Alice").await;
    let g = a.guid;
    let giver = guid::unit(GIVER, 30);
    let offered = move |m: &mut Map| crate::game::map::quests::giver_quests(m, g, giver).len();
    assert_eq!(map.query(offered).await, Some(1), "the event's quest waits for it");
    let dressed = guid::unit(3011, 32);
    let display = move |m: &mut Map| {
        m.creatures
            .iter()
            .find(|(_, c)| c.spawn_guid == Some(32))
            .map(|(_, c)| c.fields.get_u32(UNIT_FIELD_DISPLAYID))
    };
    let before = map.query(display).await.flatten();
    let on: ahash::AHashSet<u32> = [9].into_iter().collect();
    *realm.social.events.lock() = on.clone();
    map.query(move |m| crate::game::events::apply(m, &on)).await;
    assert_eq!(map.query(offered).await, Some(2));
    assert_eq!(map.query(display).await.flatten(), Some(4321));
    // Over, it looks itself again.
    let off = ahash::AHashSet::new();
    *realm.social.events.lock() = off.clone();
    map.query(move |m| crate::game::events::apply(m, &off)).await;
    assert_eq!(map.query(display).await.flatten(), before);
    let _ = dressed;
}

fn flags_of(m: &Map, g: Guid, entry: u32) -> Option<u32> {
    let p = m.players.get(&g)?;
    p.items
        .iter()
        .find(|i| i.entry == entry)
        .map(|i| i.fields.get_u32(ITEM_FIELD_FLAGS))
}

#[tokio::test]
async fn items_bind_on_pickup_and_on_equip() {
    let (_realm, a, map) = alone("Alice").await;
    let g = a.guid;
    let (ring, cloak_before, cloak_after) = map
        .query(move |m| {
            crate::game::map::inventory::add_item(m, g, BOP_RING, 1, false).unwrap();
            crate::game::map::inventory::add_item(m, g, BOE_CLOAK, 1, false).unwrap();
            let ring = flags_of(m, g, BOP_RING).unwrap();
            let before = flags_of(m, g, BOE_CLOAK).unwrap();
            let at = {
                let p = &m.players[&g];
                let i = p.items.iter().find(|i| i.entry == BOE_CLOAK).unwrap();
                (i.bag, i.slot)
            };
            crate::game::map::inventory::move_item(m, g, at, (BAG_PLAYER, 14));
            (ring, before, flags_of(m, g, BOE_CLOAK).unwrap())
        })
        .await
        .unwrap();
    assert_eq!(ring & 1, 1, "bound on pickup");
    assert_eq!(cloak_before & 1, 0);
    assert_eq!(cloak_after & 1, 1, "bound when worn");
}

#[tokio::test]
async fn charges_are_spent_and_cooldowns_kept() {
    let (_realm, mut a, map) = alone("Alice").await;
    let g = a.guid;
    let use_item = |entry: u32| {
        move |m: &mut Map| {
            m.now = Instant::now();
            let at = {
                let p = &m.players[&g];
                let i = p.items.iter().find(|i| i.entry == entry).unwrap();
                (i.bag, i.slot)
            };
            if let Some(p) = m.players.get_mut(&g) {
                p.caster.gcd_until = None;
            }
            crate::game::map::spells::use_item(
                m,
                g,
                at.0,
                at.1,
                &SpellCastTargets {
                    target_flags: SpellCastTargets_SpellCastTargetFlags::empty(),
                },
            );
        }
    };
    map.query(move |m| {
        crate::game::map::inventory::add_item(m, g, POTION, 2, false).unwrap();
        crate::game::map::inventory::add_item(m, g, WAND, 1, false).unwrap();
    })
    .await;
    map.query(use_item(POTION)).await;
    a.wait_for("SMSG_ITEM_COOLDOWN", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_COOLDOWN(c) if c.id == HEAL_SPELL => Some(()),
        _ => None,
    })
    .await;
    let left = map.query(move |m| m.players[&g].count_of(POTION)).await;
    assert_eq!(left, Some(1), "a potion drunk is gone");
    // The category's cooldown holds the next.
    map.query(use_item(POTION)).await;
    assert_eq!(map.query(move |m| m.players[&g].count_of(POTION)).await, Some(1));
    // A wand's charges run down to nothing; the wand stays.
    let charges = move |m: &mut Map| {
        m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == WAND)
            .map(|i| i.fields.get_u32(ITEM_FIELD_SPELL_CHARGES))
    };
    assert_eq!(map.query(charges).await.flatten(), Some(2));
    map.query(use_item(WAND)).await;
    map.query(use_item(WAND)).await;
    map.query(use_item(WAND)).await;
    assert_eq!(map.query(charges).await.flatten(), Some(0));
    assert_eq!(map.query(move |m| m.players[&g].count_of(WAND)).await, Some(1));
}

#[tokio::test]
async fn timed_items_run_out_and_conjured_ones_fade() {
    let (realm, a, map) = alone("Alice").await;
    let g = a.guid;
    let left = map
        .query(move |m| {
            crate::game::map::inventory::add_item(m, g, TIMED, 1, false).unwrap();
            m.now = Instant::now();
            m.loot_state.items_tick_at = Some(m.now - Duration::from_secs(3));
            crate::game::map::items::update(m);
            m.players[&g].count_of(TIMED)
        })
        .await;
    assert_eq!(left, Some(0), "its two seconds are up");
    // Conjured bread is gone for one away a quarter of an hour.
    let data = realm.data.clone();
    let faded = map
        .query(move |m| {
            crate::game::map::inventory::add_item(m, g, CONJURED_BREAD, 1, false).unwrap();
            let p = m.players.get_mut(&g).unwrap();
            p.row.logout_time = crate::game::social::mail::now() - 16 * 60;
            crate::game::map::items::on_login(p, &data);
            p.count_of(CONJURED_BREAD)
        })
        .await;
    assert_eq!(faded, Some(0));
}

#[tokio::test]
async fn keys_go_on_the_keyring_and_arrows_in_the_quiver() {
    let (_realm, _a, map) = alone("Alice").await;
    let g = _a.guid;
    let places = map
        .query(move |m| {
            crate::game::map::inventory::add_item(m, g, KEY, 1, false).unwrap();
            crate::game::map::inventory::add_item(m, g, QUIVER, 1, false).unwrap();
            let at = {
                let p = &m.players[&g];
                let i = p.items.iter().find(|i| i.entry == QUIVER).unwrap();
                (i.bag, i.slot)
            };
            crate::game::map::inventory::move_item(m, g, at, (BAG_PLAYER, 19));
            crate::game::map::inventory::add_item(m, g, ARROW, 50, false).unwrap();
            crate::game::map::inventory::add_item(m, g, BOP_RING, 1, false).unwrap();
            let p = &m.players[&g];
            let where_ = |e| p.items.iter().find(|i| i.entry == e).map(|i| (i.bag, i.slot));
            (where_(KEY), where_(ARROW), where_(BOP_RING))
        })
        .await
        .unwrap();
    assert_eq!(places.0, Some((BAG_PLAYER, 81)), "the keyring");
    assert_eq!(places.1, Some((19, 0)), "the quiver");
    assert_ne!(places.2.map(|p| p.0), Some(19), "a ring isn't for the quiver");
}

#[tokio::test]
async fn a_timed_quest_fails_when_its_time_is_up() {
    let (_realm, mut a, map) = alone("Alice").await;
    let g = a.guid;
    let giver = guid::unit(GIVER, 30);
    map.query(move |m| crate::game::map::quests::accept(m, g, giver, TIMED_QUEST))
        .await;
    a.wait_for("SMSG_QUESTUPDATE_FAILEDTIMER", |m| match m {
        ServerOpcodeMessage::SMSG_QUESTUPDATE_FAILEDTIMER(f) if f.quest_id == TIMED_QUEST => Some(()),
        _ => None,
    })
    .await;
    let failed = map
        .query(move |m| {
            let p = &m.players[&g];
            p.journal.log.iter().any(|l| l.quest == TIMED_QUEST && l.failed)
        })
        .await;
    assert_eq!(failed, Some(true));
}

#[tokio::test]
async fn a_gossip_option_marks_its_point_of_interest() {
    let (_realm, mut a, _map) = alone("Alice").await;
    let giver = guid::unit(GIVER, 30);
    a.sees(giver).await;
    a.send(CMSG_GOSSIP_HELLO { guid: giver }).await;
    a.wait_for("SMSG_GOSSIP_MESSAGE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(_)).then_some(())
    })
    .await;
    a.send(CMSG_GOSSIP_SELECT_OPTION {
        guid: giver,
        gossip_list_id: 0,
        unknown: None,
    })
    .await;
    let name = a
        .wait_for("SMSG_GOSSIP_POI", |m| match m {
            ServerOpcodeMessage::SMSG_GOSSIP_POI(p) => Some(p.location_name.clone()),
            _ => None,
        })
        .await;
    assert_eq!(name, "Lion's Pride Inn");
}

#[tokio::test]
async fn a_flight_goes_on_after_a_relog() {
    let (_realm, _a, map) = alone("Alice").await;
    let g = _a.guid;
    let resumed = map
        .query(move |m| {
            let p = m.players.get_mut(&g).unwrap();
            p.row.state.flight = Some(crate::game::store::SavedFlight {
                map: 0,
                points: vec![(START.0 + 100.0, START.1, START.2 + 20.0)],
                destination: 2,
                mount: 1149,
            });
            crate::game::map::npc::resume_flight(m, g);
            let p = &m.players[&g];
            (
                p.flight.is_some(),
                p.fields.get_u32(UNIT_FIELD_MOUNTDISPLAYID),
            )
        })
        .await;
    assert_eq!(resumed, Some((true, 1149)));
    // And saved again mid-flight, it keeps the rest of the way.
    let saved = map
        .query(move |m| {
            let now = m.now;
            m.players.get_mut(&g).unwrap().to_row(now).state.flight
        })
        .await
        .flatten();
    assert!(saved.is_some_and(|f| f.destination == 2 && !f.points.is_empty()));
}

#[tokio::test]
async fn an_escort_walks_its_path_and_credits_the_quest() {
    let (_realm, _a, map) = alone("Alice").await;
    let g = _a.guid;
    let escortee = guid::unit(ESCORTEE, 31);
    map.query(move |m| {
        if let Some(c) = m.creature_mut(escortee) {
            c.speeds.walk = 200.0;
            c.speeds.run = 200.0;
        }
        crate::game::map::quests::accept(m, g, escortee, ESCORT_QUEST);
    })
    .await;
    let started = map.query(|m| m.escorts.len()).await;
    assert_eq!(started, Some(1));
    // Walk it along: the player follows a few yards behind.
    let mut done = false;
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(30)).await;
        done = map
            .query(move |m| {
                if let Some(at) = m.creature(escortee).map(|c| c.position)
                    && let Some(p) = m.players.get_mut(&g)
                {
                    p.movement.position = at;
                }
                for e in m.escorts.iter_mut() {
                    e.skip_start();
                }
                let p = &m.players[&g];
                p.journal
                    .log
                    .iter()
                    .any(|l| l.quest == ESCORT_QUEST && l.explored)
            })
            .await
            .unwrap_or(false);
        if done {
            break;
        }
    }
    assert!(done, "credited at the credit point");
}

#[tokio::test]
async fn an_escort_left_behind_fails() {
    let (_realm, mut a, map) = alone("Alice").await;
    let g = a.guid;
    let escortee = guid::unit(ESCORTEE, 31);
    map.query(move |m| {
        crate::game::map::quests::accept(m, g, escortee, ESCORT_QUEST);
        // Far away.
        if let Some(p) = m.players.get_mut(&g) {
            p.movement.position.x += 500.0;
        }
    })
    .await;
    a.wait_for("SMSG_QUESTUPDATE_FAILED", |m| match m {
        ServerOpcodeMessage::SMSG_QUESTUPDATE_FAILED(f) if f.quest_id == ESCORT_QUEST => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(map.query(|m| m.escorts.len()).await, Some(0));
}

#[tokio::test]
async fn the_scourge_invasion_counts_its_victories() {
    let realm = harness::realm_with(data());
    crate::game::invasion::enable(&realm);
    {
        let s = realm.social.invasion.lock();
        assert_eq!(s.active.len(), 6, "the first wave strikes everywhere");
    }
    assert!(realm.social.events.lock().contains(&crate::game::invasion::INVASION) || {
        // The event is driven; with no row for it in this world it can't run.
        realm.social.driven_events.lock().get(&crate::game::invasion::INVASION) == Some(&true)
    });
    assert!(
        crate::game::map::world_states::global(&realm)
            .iter()
            .any(|s| s.state == 2284 && s.value == 3),
        "Winterspring's three necropolises"
    );
    for _ in 0..3 {
        crate::game::invasion::necropolis_fell(&realm, 618);
    }
    {
        let mut s = realm.social.invasion.lock();
        assert_eq!(s.remaining.get(&618), Some(&0));
        s.attack_at.insert(618, 0);
        s.next_pass = 0;
    }
    crate::game::invasion::sweep(&realm);
    let s = realm.social.invasion.lock();
    assert_eq!(s.victories, 1);
    assert!(!s.active.contains(&618), "Winterspring is won");
}

/// The real world's points of interest, event quests and looks, escort
/// paths and elevator frames load, and every escort in the catalog has its
/// quest and a path to walk.
#[test]
fn the_real_world_data_loads() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    assert!(d.world.pois.len() > 100, "{}", d.world.pois.len());
    assert!(d.world.event_quests.len() > 30);
    assert!(!d.world.event_creatures.is_empty());
    assert!(d.world.script_waypoints.len() > 30);
    assert!(d.world.animations.len() > 5);
    assert!(d.events.get(&4).is_some_and(|e| e.hardcoded), "the Darkmoon Faire");
    let mut walkable = 0;
    for e in crate::game::map::escorts::catalog::ESCORTS {
        assert!(d.npc.quests.contains_key(&e.quest), "escort quest {}", e.quest);
        if e.path.is_some() || d.world.script_waypoints.contains_key(&e.entry) {
            walkable += 1;
        }
    }
    assert!(walkable >= 25, "{walkable} escorts with a path");
    for f in crate::game::map::escorts::catalog::FOLLOWERS {
        assert!(d.npc.quests.contains_key(&f.quest), "follower quest {}", f.quest);
    }
}

#[tokio::test]
async fn an_elemental_rift_calls_out_its_invaders() {
    let mut d = data();
    for (id, name) in [(13, "Elemental Invasion"), (68, "Fire rift")] {
        d.events.insert(
            id,
            GameEvent {
                id,
                name: name.into(),
                start: 0,
                end: 0,
                occurence: 0,
                length: 0,
                hardcoded: true,
            },
        );
    }
    d.gameobject_templates.insert(
        179_666,
        GameObjectTemplate {
            entry: 179_666,
            kind: 5,
            display_id: 1,
            name: "Elemental Rift".into(),
            size: 1.0,
            ..Default::default()
        },
    );
    d.gameobject_spawns.insert(
        0,
        vec![GameObjectSpawn {
            guid: 60,
            entry: 179_666,
            map: 0,
            x: START.0 + 5.0,
            y: START.1,
            z: START.2,
            state: 1,
            ..Default::default()
        }],
    );
    d.event_objects.insert(60, 68);
    let mut invader = d.creature_templates[&GIVER].clone();
    invader.entry = 14_460;
    invader.name = "Blazing Invader".into();
    invader.npc_flags = 0;
    d.creature_templates.insert(14_460, invader);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    // The world starts invaded: the rift comes with its event.
    crate::game::social::sweep(&realm);
    let rift = guid::game_object(179_666, 60);
    a.sees(rift).await;
    let invaders = map
        .query(|m| {
            m.rifts_checked_at = None;
            crate::game::elemental::update(m);
            m.rifts.values().map(|r| r.invaders.clone()).next().unwrap_or_default()
        })
        .await
        .unwrap();
    assert_eq!(invaders.len(), 3, "three invaders at the first stage");
    // One slain counts against the element.
    let first = invaders[0];
    map.query(move |m| crate::game::map::combat::kill(m, g, first)).await;
    assert_eq!(realm.social.elemental.lock().kills[0], 1);
}

#[tokio::test]
async fn twenty_fireworks_wake_omen() {
    let mut d = data();
    d.gameobject_templates.insert(
        crate::game::omen::LAUNCHER,
        GameObjectTemplate {
            entry: crate::game::omen::LAUNCHER,
            kind: 22,
            display_id: 1,
            name: "Cluster Launcher".into(),
            size: 1.0,
            ..Default::default()
        },
    );
    d.gameobject_spawns.insert(
        0,
        vec![GameObjectSpawn {
            guid: 70,
            entry: crate::game::omen::LAUNCHER,
            map: 0,
            x: START.0 + 2.0,
            y: START.1 + 2.0,
            z: START.2,
            state: 1,
            ..Default::default()
        }],
    );
    let mut omen = d.creature_templates[&GIVER].clone();
    omen.entry = crate::game::omen::OMEN;
    omen.name = "Omen".into();
    d.creature_templates.insert(omen.entry, omen);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let launcher = guid::game_object(crate::game::omen::LAUNCHER, 70);
    let out = map
        .query(move |m| {
            let mut seen = Vec::new();
            for _ in 0..20 {
                crate::game::map::world::use_object(m, g, launcher);
                seen.push(m.realm.social.omen.lock().omen.is_some());
            }
            seen
        })
        .await
        .unwrap();
    assert!(!out[18] && out[19], "Omen rises at the twentieth");
    assert!(realm.social.omen.lock().minions);
    assert_eq!(
        realm.social.driven_events.lock().get(&crate::game::omen::MINIONS_OF_OMEN),
        Some(&true)
    );
}
