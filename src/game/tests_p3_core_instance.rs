//! Phase 3's dungeon copies, Thistle Tea's `InstanceTest` ported: a copy
//! moved out of empties and goes, and is the player's to come back to; a
//! former party's copy isn't entered alone; a copy's data added as
//! unsigned 32 bits; and no switching into a copy there isn't.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;

const DEADMINES: u32 = 36;
const SHADOWFANG: u32 = 33;

fn data() -> GameData {
    let mut d = GameData::empty();
    for (id, name) in [(DEADMINES, "DeadminesInstance"), (SHADOWFANG, "Shadowfang")] {
        d.maps.insert(
            id,
            MapInfo {
                id,
                internal_name: name.into(),
                name: name.into(),
                instance_type: 1,
                player_limit: 0,
            },
        );
    }
    d
}

/// Send `c` to `map` (its copy, for a dungeon) and wait for it to land.
async fn go(realm: &Arc<Realm>, c: &mut Client, map: u32) -> MapKey {
    let g = c.guid;
    let from = realm.registry.online(g).unwrap().map();
    from.call(move |m| {
        m.teleport(
            g,
            MapKey::world(map),
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
    realm.registry.online(g).unwrap().map().key
}

/// Thistle Tea's `enter/4` and `destroy_empty/2`: a copy its last player
/// leaves for another dungeon is the same copy to come back to while it is
/// kept, then empties and is gone (wvs's `instance_unload`); a copy of a
/// party one was in isn't the one entered alone after.
#[tokio::test]
async fn a_copy_left_empties_and_a_partys_isnt_entered_alone() {
    // Empty copies kept two seconds.
    let mut cfg = harness::test_config();
    cfg.instance_unload = Duration::from_secs(2);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = Realm::new(cfg, store, startup, data(), None);
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let first = go(&realm, &mut a, DEADMINES).await;
    assert_ne!(first.instance, 0);
    let other = go(&realm, &mut a, SHADOWFANG).await;
    assert_ne!(other, first);
    assert_eq!(
        go(&realm, &mut a, DEADMINES).await,
        first,
        "hers to come back to"
    );
    go(&realm, &mut a, SHADOWFANG).await;
    let mut gone = false;
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        if !realm.maps.all().iter().any(|h| h.key == first) {
            gone = true;
            break;
        }
    }
    assert!(gone, "the emptied copy unloaded");
    assert_ne!(go(&realm, &mut a, DEADMINES).await, first, "gone for good");

    // With Bob in a party, into his copy; alone after, not into it.
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    go(&realm, &mut a, 0).await;
    b.send(CMSG_GROUP_INVITE {
        name: "Alice".into(),
    })
    .await;
    a.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    a.send(CMSG_GROUP_ACCEPT {}).await;
    a.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
        _ => None,
    })
    .await;
    let party = go(&realm, &mut b, SHADOWFANG).await;
    assert_eq!(go(&realm, &mut a, SHADOWFANG).await, party, "the party's");
    go(&realm, &mut a, 0).await;
    a.send(CMSG_GROUP_DISBAND {}).await;
    a.wait_for("SMSG_GROUP_DESTROYED", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_DESTROYED).then_some(())
    })
    .await;
    assert_ne!(
        go(&realm, &mut a, SHADOWFANG).await,
        party,
        "alone, not the party's"
    );
}

/// Thistle Tea's script data: an increment wraps as unsigned 32 bits, a
/// decrement stops at none.
#[tokio::test]
async fn a_copys_data_adds_as_unsigned_32_bits() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let key = go(&realm, &mut a, DEADMINES).await;
    let seen = realm
        .maps
        .get(&realm, key)
        .query(|m| {
            use crate::game::map::instance_script::{Script, get_data, set_data};
            m.instance.script = Some(Script::Unported);
            set_data(m, 7, 1, 0);
            set_data(m, 7, 0xFFFF_FFFF, 1);
            let wrapped = get_data(m, 7);
            set_data(m, 7, 8, 0);
            set_data(m, 7, 20, 2);
            (wrapped, get_data(m, 7))
        })
        .await
        .unwrap();
    assert_eq!(seen, (0, 0));
}

/// Thistle Tea's `join_copy/3`: a GM's `.instance switch` into a copy
/// there isn't is refused, and they stay.
#[tokio::test]
async fn no_switching_into_a_copy_there_isnt() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let key = go(&realm, &mut a, DEADMINES).await;
    a.clear();
    a.say(".instance switch 99").await;
    a.system_message("No copy 99").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(realm.registry.online(a.guid).unwrap().map().key, key);
}
