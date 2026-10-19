//! Round 7's player tests of the GM's commands, ported from Thistle Tea's
//! `test/game/world/entity/player/dev_commands_test.exs`
//! (docs/parity/matrix.csv): `.go xyz` in a copy, `.instance data`,
//! `.debug position`, `.honor points`, `.debug events`, `.debug
//! reputation find` and `.battleground`'s usage.

use std::sync::Arc;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::misc::FactionDef;
use crate::game::data::scripts::MapTemplate;
use crate::game::data::{GameData, MapInfo};
use crate::game::fields::{PLAYER_BYTES_3, PLAYER_FIELD_BYTES};
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;
use crate::game::tests_p3_combat::BRUTE;
use crate::game::tests_p3_entity::on;

/// Shadowfang Keep's map, a dungeon.
const DUNGEON: u32 = 33;
const STORMWIND: u32 = 72;

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.maps.insert(
        DUNGEON,
        MapInfo {
            id: DUNGEON,
            internal_name: "Map33".into(),
            name: "Map 33".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    d.scripts.map_templates.insert(
        DUNGEON,
        MapTemplate {
            map: DUNGEON,
            map_type: 1,
            player_limit: 10,
            ..Default::default()
        },
    );
    d.misc.factions.insert(
        STORMWIND,
        FactionDef {
            id: STORMWIND,
            index: 19,
            parent: 0,
            name: "Stormwind".into(),
            variants: [(1, 0, 0, 1), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)],
        },
    );
    d
}

async fn gm(realm: &Arc<Realm>) -> Client {
    let mut a = Client::connect(realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a
}

fn key_of(realm: &Realm, c: &Client) -> MapKey {
    realm.registry.online(c.guid).unwrap().map().key
}

/// The GM's line said, and the system message that answers it.
async fn ask(a: &mut Client, line: &str, answer: &str) -> String {
    a.clear();
    a.say(line).await;
    a.system_message(answer).await
}

/// Into the dungeon's copy, landed.
async fn into_copy(realm: &Arc<Realm>, a: &mut Client) -> MapKey {
    let g = a.guid;
    realm
        .registry
        .online(g)
        .unwrap()
        .map()
        .call(move |m| m.teleport(g, MapKey::world(DUNGEON), Vector3d::default(), 0.0));
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(g).await;
    let key = key_of(realm, a);
    assert_eq!(key.map, DUNGEON);
    assert_ne!(key.instance, 0, "a copy");
    key
}

/// A near teleport's ack, and where `a` then stands and faces.
async fn landed(realm: &Arc<Realm>, a: &mut Client) -> (Vector3d, f32) {
    a.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    let g = a.guid;
    let map = realm.registry.online(g).unwrap().map();
    on(&map, move |m| {
        let p = &m.players[&g];
        (p.position(), p.movement.orientation)
    })
    .await
}

/// Thistle Tea's `.go xyz` "preserves the current instance copy when no
/// map is supplied", "faces the given angle in the current instance copy"
/// and "accepts here without a facing angle in the current instance
/// copy": inside a dungeon's copy, `.go xyz` with no map, or `here`, keeps
/// the GM in that copy (a near teleport), facing as told.
#[tokio::test]
async fn go_xyz_in_a_copy_keeps_the_copy() {
    let realm = harness::realm_with(world());
    let mut a = gm(&realm).await;
    let copy = into_copy(&realm, &mut a).await;
    a.clear();
    a.say(".go xyz 10 20 30").await;
    let (at, _) = landed(&realm, &mut a).await;
    assert_eq!((at.x, at.y, at.z), (10.0, 20.0, 30.0));
    assert_eq!(key_of(&realm, &a), copy, "no map: this copy");
    a.say(".go xyz 11 21 31 here 1.5").await;
    let (at, o) = landed(&realm, &mut a).await;
    assert_eq!((at.x, at.y, at.z), (11.0, 21.0, 31.0));
    assert!((o - 1.5).abs() < 0.01, "{o}");
    assert_eq!(key_of(&realm, &a), copy, "here: this copy");
    a.say(".go xyz 12 22 32 here").await;
    let (at, o) = landed(&realm, &mut a).await;
    assert_eq!((at.x, at.y, at.z), (12.0, 22.0, 32.0));
    assert!((o - 1.5).abs() < 0.01, "the facing kept: {o}");
    assert_eq!(key_of(&realm, &a), copy);
}

/// Thistle Tea's `.instance data` "reports registered data without
/// exposing a write path" and "distinguishes open worlds" (from copies),
/// and `.debug position` "reports open-world and instance positions with
/// orientation" and "reports missing entities and rejects malformed
/// guids": the copy's data, all of it or a field, read only; in the open
/// world, none; where an object (by guid) is and faces, in the world or
/// a copy; one not here missing; a guid that isn't one, the usage.
#[tokio::test]
async fn instance_data_and_positions_are_reported() {
    let realm = harness::realm_with(world());
    let mut a = gm(&realm).await;
    let g = a.guid;
    let open = realm.maps.get(&realm, MapKey::world(0));
    let brute = on(&open, move |m| {
        let b = crate::game::tests_p3_combat::spawn(m, g, BRUTE, 20, 5.0);
        m.creature_mut(b).unwrap().orientation = 2.5;
        b
    })
    .await;
    ask(
        &mut a,
        ".instance data 7",
        "Current world is not an instance copy.",
    )
    .await;
    let told = ask(
        &mut a,
        &format!(".debug position {}", brute.guid()),
        "orientation",
    )
    .await;
    assert!(told.contains("orientation 2.50 in map 0"), "{told}");
    let hex = ask(
        &mut a,
        &format!(".debug position 0x{:X}", brute.guid()),
        "orientation",
    )
    .await;
    assert_eq!(hex, told, "the same guid in hex");
    ask(
        &mut a,
        ".debug position 999999",
        "Entity 999999 is inactive or missing.",
    )
    .await;
    ask(&mut a, ".debug position nope", "Usage: .debug position").await;
    let copy = into_copy(&realm, &mut a).await;
    let inside = realm.maps.get(&realm, copy);
    on(&inside, move |m| {
        m.instance.data.insert(7, 2);
        m.players.get_mut(&g).unwrap().movement.orientation = 5.43;
    })
    .await;
    let all = ask(&mut a, ".instance data", "Instance data").await;
    assert!(all.ends_with(": 7=2"), "{all}");
    let one = ask(&mut a, ".instance data 5", "Instance data").await;
    assert!(one.ends_with(": 5=0"), "{one}");
    ask(&mut a, ".instance data 7 2", "Usage: .instance").await;
    let data = on(&inside, |m| m.instance.data.clone()).await;
    assert_eq!(data.into_iter().collect::<Vec<_>>(), [(7, 2)], "unwritten");
    let me = ask(
        &mut a,
        &format!(".debug position {}", g.guid()),
        "orientation",
    )
    .await;
    assert!(
        me.contains(&format!(
            "orientation 5.43 in map {DUNGEON} instance {}",
            copy.instance
        )),
        "{me}"
    );
}

/// Thistle Tea's `.debug honor` "changes the ledger and owner projection
/// while retaining the earned rank", `.debug events` "parses status and
/// event changes", `.debug reputation` "finds" a faction, and
/// `.battleground` "shows usage for unsupported subcommands": rank points
/// set make the rank and the highest rank, which a demotion keeps; a
/// start or stop of an event there is none of says so; a faction found by
/// part of its name; a battleground subcommand there is none of, the
/// usage.
#[tokio::test]
async fn honor_events_reputation_and_battleground_commands() {
    let realm = harness::realm_with(world());
    let mut a = gm(&realm).await;
    let g = a.guid;
    ask(&mut a, ".character level 60", "Level 60.").await;
    let ranks = || {
        let map = realm.maps.get(&realm, MapKey::world(0));
        async move {
            on(&map, move |m| {
                let f = &m.players[&g].fields;
                (
                    f.get_byte(PLAYER_BYTES_3, 3),
                    f.get_byte(PLAYER_FIELD_BYTES, 3),
                )
            })
            .await
        }
    };
    ask(&mut a, ".honor points 20000", "Rank points set").await;
    assert_eq!(ranks().await, (10, 10), "20,000 points: rank 10");
    ask(&mut a, ".honor points 0", "Rank points set").await;
    assert_eq!(ranks().await, (0, 10), "demoted, the highest kept");
    ask(&mut a, ".honor points many", "Use: .honor points").await;
    assert_eq!(ranks().await, (0, 10));
    for action in ["start", "stop"] {
        ask(
            &mut a,
            &format!(".debug events {action} 999999"),
            "No event 999999.",
        )
        .await;
    }
    ask(&mut a, ".debug events invalid", "Usage:").await;
    let found = ask(&mut a, ".debug reputation find storm", "Stormwind").await;
    assert_eq!(found, "72 Stormwind");
    ask(
        &mut a,
        ".battleground score alliance",
        "Usage: .bg join <warsong|arathi|alterac>|start|info|leave",
    )
    .await;
}
