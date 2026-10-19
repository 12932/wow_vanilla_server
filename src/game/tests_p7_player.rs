//! Round 7's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/` (docs/parity/matrix.csv): conditions
//! read off the player as they stand, and who a meeting stone queues.

use std::time::Duration;

use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::scripts::{CF_SWAP_TARGETS, Condition};
use crate::game::fields::PLAYER_BYTES_3;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;
use crate::game::map::conditions::{Truth, check};
use crate::game::map::player::Item;
use crate::game::tests_p3_entity::on;

const EVENT_QUEST: u32 = 64_300;
const QUEST_EVENT: u32 = 12;
const FAIR: u32 = 7;
const SWORD: u32 = 64_200;
const FIREBALL: u32 = 133;

fn cond(d: &mut GameData, entry: u32, kind: i32, values: [i32; 4], flags: u32) {
    d.scripts.conditions.insert(
        entry,
        Condition {
            entry,
            kind,
            values,
            flags,
        },
    );
}

fn conditions_world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.npc.quests.insert(
        EVENT_QUEST,
        crate::game::data::npc::Quest {
            id: EVENT_QUEST,
            title: "Fair Game".into(),
            ..Default::default()
        },
    );
    d.world.event_quests.insert(EVENT_QUEST, QUEST_EVENT);
    d.items.insert(
        SWORD,
        crate::game::data::ItemTemplate {
            entry: SWORD,
            name: "Sword".into(),
            stackable: 1,
            allowable_class: -1,
            allowable_race: -1,
            inventory_type: 13,
            ..Default::default()
        },
    );
    cond(&mut d, 1, 19, [EVENT_QUEST as i32, 0, 0, 0], 0);
    cond(&mut d, 2, 12, [FAIR as i32, 0, 0, 0], 0);
    cond(&mut d, 3, 3, [SWORD as i32, 0, 0, 0], 0);
    // Elwynn Forest, Goldshire in it, and Westfall.
    cond(&mut d, 4, 4, [12, 0, 0, 0], 0);
    cond(&mut d, 5, 4, [87, 0, 0, 0], 0);
    cond(&mut d, 6, 4, [40, 0, 0, 0], 0);
    cond(&mut d, 7, 36, [QUEST_EVENT as i32, 0, 0, 0], 0);
    cond(&mut d, 8, 16, [38, 0, 0, 0], 0);
    // Fireball known, and not known.
    cond(&mut d, 9, 17, [FIREBALL as i32, 0, 0, 0], 0);
    cond(&mut d, 10, 17, [FIREBALL as i32, 1, 0, 0], 0);
    // The rank as shown at least High Warlord's 14, of the source.
    cond(&mut d, 11, 51, [14, 1, 0, 0], CF_SWAP_TARGETS);
    d
}

/// Thistle Tea's `ConditionContext.build`, its "quest availability alone
/// requests the active event snapshot", "collects requested item totals,
/// catalogs, and world facts once", "matches both authoritative zone and
/// sub-area IDs", "retains an explicitly absent source without world
/// lookups", "source-dependent conditions with an absent source are
/// unknown" and "uses owner-published rank for swapped player sources",
/// and `refresh_subject`'s "publishes known spell ids": each condition
/// read off the player and the world as they are when it is judged. A
/// quest of an event is available only while it runs; a game event; an
/// equipped item; the zone or the sub-area inside it; a map event, with no
/// source; a source's entry with none, unknown; a spell known; a swapped
/// player source's rank.
#[tokio::test]
async fn conditions_read_the_player_and_the_world_as_they_stand() {
    let realm = harness::realm_with(conditions_world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Mage).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let r = realm.clone();
    let seen = on(&map, move |m| {
        let c = |m: &crate::game::map::Map, id| check(m, id, None, Some(g));
        let quest_off = c(m, 1);
        r.social.events.lock().insert(QUEST_EVENT);
        let quest_on = c(m, 1);
        let fair_off = c(m, 2);
        r.social.events.lock().insert(FAIR);
        let fair_on = c(m, 2);
        r.social.events.lock().remove(&FAIR);
        r.social.events.lock().remove(&QUEST_EVENT);
        let unequipped = c(m, 3);
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.push(Item::new(
            &data,
            guid::item(964_200),
            g,
            SWORD,
            crate::game::store::BAG_PLAYER,
            15,
            1,
        ));
        let equipped = c(m, 3);
        let p = m.players.get_mut(&g).unwrap();
        p.zone = 12;
        p.area = 87;
        let areas = (c(m, 4), c(m, 5), c(m, 6));
        let no_map_event = c(m, 7);
        crate::game::map::map_events::start(m, QUEST_EVENT, g, g, Duration::from_secs(60));
        let map_event = c(m, 7);
        let no_source = c(m, 8);
        let p = m.players.get_mut(&g).unwrap();
        p.spells.retain(|s| *s != FIREBALL);
        let unknown_spell = (c(m, 9), c(m, 10));
        m.players.get_mut(&g).unwrap().spells.push(FIREBALL);
        let known_spell = (c(m, 9), c(m, 10));
        let unranked = check(m, 11, Some(g), None);
        let p = m.players.get_mut(&g).unwrap();
        p.fields.set_byte(PLAYER_BYTES_3, 3, 18);
        let ranked = check(m, 11, Some(g), None);
        (
            (quest_off, quest_on, fair_off, fair_on),
            (unequipped, equipped),
            areas,
            (no_map_event, map_event, no_source),
            (unknown_spell, known_spell),
            (unranked, ranked),
        )
    })
    .await;
    use Truth::*;
    assert_eq!(
        seen.0,
        (Unmet, Met, Unmet, Met),
        "the quest of an event, then a game event, off and on"
    );
    assert_eq!(seen.1, (Unmet, Met), "a sword in the main hand");
    assert_eq!(
        seen.2,
        (Met, Met, Unmet),
        "the zone and the area in it, not another"
    );
    assert_eq!(
        seen.3,
        (Unmet, Met, Unknown),
        "a map event with no source; a source's entry with none"
    );
    assert_eq!(seen.4, ((Unmet, Met), (Met, Unmet)), "Fireball known");
    assert_eq!(seen.5, (Unmet, Met), "High Warlord, the source's");
}

const STONE: u32 = 178_824;

/// The queue status `a` is told within a moment, if any.
async fn quiet(a: &mut Client) -> Option<MeetingStoneStatus> {
    a.gets(Duration::from_millis(150), |m| match m {
        wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_MEETINGSTONE_SETQUEUE(
            s,
        ) => Some(s.status),
        _ => None,
    })
    .await
}

fn stone_world() -> GameData {
    use crate::game::data::{GameObjectSpawn, GameObjectTemplate};
    use crate::game::tests_p3_combat::START;
    let mut d = crate::game::tests_p3_combat::data();
    d.gameobject_templates.insert(
        STONE,
        GameObjectTemplate {
            entry: STONE,
            kind: 23,
            display_id: 1,
            name: "Meeting Stone".into(),
            size: 1.0,
            ..Default::default()
        },
    );
    d.gameobject_spawns.insert(
        0,
        vec![GameObjectSpawn {
            guid: 30,
            entry: STONE,
            map: 0,
            x: START.0 - 2.0,
            y: START.1,
            z: START.2,
            rotation: [0.0, 0.0, 0.0, 1.0],
            state: 1,
            ..Default::default()
        }],
    );
    d.misc.meeting_stones.insert(
        STONE,
        crate::game::data::misc::MeetingStone {
            min_level: 1,
            max_level: 60,
            area: 1581,
        },
    );
    d
}

/// Thistle Tea's `MeetingStones.join` "queues at a live stone and rejects
/// remote, dead, disabled, or foreign targets": the dead, one too far
/// off, a stone that can't be used (`GAMEOBJECT_FLAGS` 0x10) and one not
/// on the player's map queue no one; at the stone, alive, they are
/// queued for its dungeon, and leaving tells them so.
#[tokio::test]
async fn a_meeting_stone_queues_only_the_living_beside_a_usable_stone() {
    use crate::game::fields::GAMEOBJECT_FLAGS;
    use crate::game::tests_p3_combat::START;
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    let realm = harness::realm_with(stone_world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let stone = guid::game_object(STONE, 30);
    a.sees(stone).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let queued = || realm.social.meeting_stones.lock().of(g).is_some();
    // Dead.
    on(&map, move |m| crate::game::map::death::kill(m, g)).await;
    a.clear();
    a.send(CMSG_MEETINGSTONE_JOIN { guid: stone }).await;
    assert_eq!(quiet(&mut a).await, None, "the dead");
    on(&map, move |m| {
        crate::game::map::death::resurrect(m, g, 1.0);
    })
    .await;
    // Too far.
    on(&map, move |m| {
        let at = Vector3d {
            x: START.0 + 30.0,
            y: START.1,
            z: START.2,
        };
        crate::game::map::movement::teleport_within(m, g, at, 0.0);
        crate::game::map::movement::teleport_ack(m, g);
    })
    .await;
    a.send(CMSG_MEETINGSTONE_JOIN { guid: stone }).await;
    assert_eq!(quiet(&mut a).await, None, "too far");
    on(&map, move |m| {
        let at = Vector3d {
            x: START.0,
            y: START.1,
            z: START.2,
        };
        crate::game::map::movement::teleport_within(m, g, at, 0.0);
        crate::game::map::movement::teleport_ack(m, g);
        let f = &mut m.gameobject_mut(stone).unwrap().fields;
        let flags = f.get_u32(GAMEOBJECT_FLAGS);
        f.set_u32(GAMEOBJECT_FLAGS, flags | 0x10);
    })
    .await;
    a.send(CMSG_MEETINGSTONE_JOIN { guid: stone }).await;
    assert_eq!(quiet(&mut a).await, None, "a stone that can't be used");
    // Another map's stone (none of this map's).
    a.send(CMSG_MEETINGSTONE_JOIN {
        guid: guid::game_object(STONE, 31),
    })
    .await;
    assert_eq!(quiet(&mut a).await, None, "not here");
    assert!(!queued());
    on(&map, move |m| {
        let f = &mut m.gameobject_mut(stone).unwrap().fields;
        let flags = f.get_u32(GAMEOBJECT_FLAGS);
        f.set_u32(GAMEOBJECT_FLAGS, flags & !0x10);
    })
    .await;
    a.send(CMSG_MEETINGSTONE_JOIN { guid: stone }).await;
    let joined = a
        .wait_for("SMSG_MEETINGSTONE_SETQUEUE", |m| match m {
            ServerOpcodeMessage::SMSG_MEETINGSTONE_SETQUEUE(s) => Some((s.area.as_int(), s.status)),
            _ => None,
        })
        .await;
    assert_eq!(joined, (1581, MeetingStoneStatus::JoinedQueue));
    assert!(queued());
    a.send(CMSG_MEETINGSTONE_LEAVE {}).await;
    let left = a
        .wait_for("SMSG_MEETINGSTONE_SETQUEUE", |m| match m {
            ServerOpcodeMessage::SMSG_MEETINGSTONE_SETQUEUE(s) => Some(s.status),
            _ => None,
        })
        .await;
    assert_eq!(left, MeetingStoneStatus::LeaveQueue);
    assert!(!queued());
}
