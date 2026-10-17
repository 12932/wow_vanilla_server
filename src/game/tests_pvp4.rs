//! Battleground and instance admission as Thistle Tea has it, through the
//! protocol: a group queued by its leader and placed together, deserters
//! kept out, the match picked from the list, a place kept for one who
//! logs out; and a copy that's no longer one's own: a minute's grace,
//! then home.

use std::time::{Duration, Instant};

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_pvp::{battleground, into, queue};
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::{MapHandle, MapKey};

const WSG: u32 = 489;

fn gulch() -> GameData {
    let mut d = GameData::empty();
    battleground(&mut d, WSG, 2, "PVPZone03", [770, 771]);
    // Deserter, an aura that stays.
    let mut s = crate::game::data::spells::SpellTemplate {
        id: crate::game::map::battleground::SPELL_DESERTER,
        name: "Deserter".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0].effect = 6;
    s.effects[0].aura = 4;
    s.effects[0].target_a = 1;
    d.spells.spells.insert(s.id, s);
    d
}

async fn join(c: &Client, as_group: bool, instance_id: u32) {
    c.send(CMSG_BATTLEMASTER_JOIN {
        guid: Guid::zero(),
        map: Map::WarsongGulch,
        instance_id,
        join_as_group: as_group,
    })
    .await;
}

async fn joined(c: &mut Client) -> BgTypeId {
    c.wait_for("SMSG_GROUP_JOINED_BATTLEGROUND", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_JOINED_BATTLEGROUND(j) => Some(j.id),
        _ => None,
    })
    .await
}

/// Nothing of the queue within a while.
async fn not_joined(c: &mut Client) {
    let heard = tokio::time::timeout(
        Duration::from_millis(300),
        c.wait_for("SMSG_GROUP_JOINED_BATTLEGROUND", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_JOINED_BATTLEGROUND(_)).then_some(())
        }),
    )
    .await;
    assert!(heard.is_err(), "told of a queue");
}

async fn group(leader: &mut Client, member: &mut Client, name: &str) {
    leader.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    member
        .wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
    member.send(CMSG_GROUP_ACCEPT {}).await;
    leader
        .wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
            _ => None,
        })
        .await;
}

fn map_of(realm: &crate::game::Realm, g: Guid) -> MapHandle {
    realm.registry.online(g).unwrap().map()
}

/// The match `g` is invited to or in.
fn match_of(realm: &crate::game::Realm, g: Guid) -> Option<u32> {
    realm.social.battlegrounds.lock().match_of(g).map(|m| m.id)
}

async fn deserter(realm: &crate::game::Realm, g: Guid) {
    map_of(realm, g)
        .query(move |m| {
            crate::game::map::spells::cast_triggered(
                m,
                g,
                crate::game::map::battleground::SPELL_DESERTER,
                Some(g),
            )
        })
        .await;
}

#[tokio::test]
async fn a_leader_queues_the_whole_group_and_it_goes_in_together() {
    let realm = harness::realm_with(gulch());
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let bob = b.enter("Bob", Race::Dwarf, Class::Paladin).await;
    group(&mut a, &mut b, "Bob").await;
    // Bob isn't the leader: his group join does nothing.
    join(&b, true, 0).await;
    not_joined(&mut b).await;
    // Alice's queues them both, and both hear it.
    join(&a, true, 0).await;
    assert_eq!(joined(&mut a).await, BgTypeId::QueuedForWsg);
    assert_eq!(joined(&mut b).await, BgTypeId::QueuedForWsg);
    // Grom, of the Horde: a match, with Alice and Bob in it together.
    let mut g = Client::connect(&realm, "G");
    let grom = g.enter("Grom", Race::Orc, Class::Warrior).await;
    queue(&g, Map::WarsongGulch).await;
    assert_eq!(joined(&mut g).await, BgTypeId::QueuedForWsg);
    for _ in 0..100 {
        if match_of(&realm, grom).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (ma, mb, mg) = (
        match_of(&realm, alice),
        match_of(&realm, bob),
        match_of(&realm, grom),
    );
    assert!(ma.is_some());
    assert_eq!(ma, mb, "the group together");
    assert_eq!(ma, mg);
}

#[tokio::test]
async fn a_group_too_big_for_a_side_isnt_queued() {
    let mut d = gulch();
    d.battlegrounds.get_mut(&2).unwrap().max_players = 1;
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Dwarf, Class::Paladin).await;
    group(&mut a, &mut b, "Bob").await;
    // Two, and a side holds one.
    join(&a, true, 0).await;
    not_joined(&mut a).await;
    // Alone, Alice may.
    join(&a, false, 0).await;
    assert_eq!(joined(&mut a).await, BgTypeId::QueuedForWsg);
}

#[tokio::test]
async fn a_deserter_is_told_so_and_kept_out_and_so_is_their_group() {
    let realm = harness::realm_with(gulch());
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let bob = b.enter("Bob", Race::Dwarf, Class::Paladin).await;
    deserter(&realm, alice).await;
    join(&a, false, 0).await;
    assert_eq!(joined(&mut a).await, BgTypeId::RemoveFromQueue);
    assert!(match_of(&realm, alice).is_none());
    // Bob a deserter (as his group's stats have it): Alice can't take the
    // group in either.
    map_of(&realm, alice)
        .query(move |m| {
            crate::game::map::auras::remove_spell(
                m,
                alice,
                crate::game::map::battleground::SPELL_DESERTER,
                None,
            )
        })
        .await;
    group(&mut a, &mut b, "Bob").await;
    deserter(&realm, bob).await;
    // His stats go out once a second.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    a.clear();
    join(&a, true, 0).await;
    assert_eq!(joined(&mut a).await, BgTypeId::RemoveFromQueue);
    let queued = realm.social.battlegrounds.lock().match_of(alice).is_some();
    assert!(!queued);
}

#[tokio::test]
async fn a_deserter_invited_cant_go_in() {
    let realm = harness::realm_with(gulch());
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut g = Client::connect(&realm, "G");
    g.enter("Grom", Race::Orc, Class::Warrior).await;
    queue(&a, Map::WarsongGulch).await;
    queue(&g, Map::WarsongGulch).await;
    a.wait_frame(
        "the invitation",
        crate::game::battlegrounds::SMSG_BATTLEFIELD_STATUS,
        |f| f.len() >= 17 && f[13] == 2,
    )
    .await;
    deserter(&realm, alice).await;
    a.clear();
    a.send(CMSG_BATTLEFIELD_PORT {
        map: Map::WarsongGulch,
        action: BattlefieldPortAction::EnterBattle,
    })
    .await;
    assert_eq!(joined(&mut a).await, BgTypeId::RemoveFromQueue);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(match_of(&realm, alice).is_none(), "the invitation declined");
    assert_ne!(map_of(&realm, alice).key.map, WSG);
}

#[tokio::test]
async fn the_match_picked_from_the_list_is_joined_if_it_has_room() {
    let realm = harness::realm_with(gulch());
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut g = Client::connect(&realm, "G");
    g.enter("Grom", Race::Orc, Class::Warrior).await;
    queue(&a, Map::WarsongGulch).await;
    queue(&g, Map::WarsongGulch).await;
    into(&realm, &mut a, Map::WarsongGulch).await;
    let id = match_of(&realm, alice).unwrap();
    let mut c = Client::connect(&realm, "C");
    let carol = c.enter("Carol", Race::Gnome, Class::Mage).await;
    // A match that isn't there: nothing.
    join(&c, false, id + 100).await;
    not_joined(&mut c).await;
    // Alice's: straight in.
    join(&c, false, id).await;
    assert_eq!(joined(&mut c).await, BgTypeId::QueuedForWsg);
    assert_eq!(match_of(&realm, carol), Some(id));
}

#[tokio::test]
async fn one_who_logs_out_in_a_match_comes_back_into_it() {
    let realm = harness::realm_with(gulch());
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut g = Client::connect(&realm, "G");
    g.enter("Grom", Race::Orc, Class::Warrior).await;
    queue(&a, Map::WarsongGulch).await;
    queue(&g, Map::WarsongGulch).await;
    let inside = into(&realm, &mut a, Map::WarsongGulch).await;
    let key = inside.key;
    a.disconnect();
    for _ in 0..200 {
        if realm.registry.online(alice).is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(realm.registry.online(alice).is_none());
    assert!(match_of(&realm, alice).is_some(), "her place kept");
    realm.store.flush().await;
    let mut a2 = Client::connect(&realm, "A");
    a2.login(alice).await;
    a2.sees(alice).await;
    assert_eq!(map_of(&realm, alice).key, key, "back in the match");
    let member = realm
        .social
        .battlegrounds
        .lock()
        .matches
        .get(&key.instance)
        .is_some_and(|m| m.members.contains_key(&alice));
    assert!(member);
}

/// A dungeon, Deadmines' copy, for players who aren't game masters.
fn dungeon() -> std::sync::Arc<crate::game::Realm> {
    let mut d = GameData::empty();
    for (id, name, kind) in [(36, "DeadminesInstance", 1), (409, "MoltenCore", 2)] {
        d.maps.insert(
            id,
            MapInfo {
                id,
                internal_name: name.into(),
                name: name.into(),
                instance_type: kind,
                player_limit: 0,
            },
        );
    }
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    crate::game::Realm::new(cfg, store, startup, d, None)
}

async fn go(realm: &std::sync::Arc<crate::game::Realm>, c: &mut Client, map: u32) {
    let g = c.guid;
    map_of(realm, g).call(move |m| {
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
}

async fn grace(c: &mut Client) -> u32 {
    c.wait_for("SMSG_RAID_GROUP_ONLY", |m| match m {
        ServerOpcodeMessage::SMSG_RAID_GROUP_ONLY(r) => Some(r.homebind_timer),
        _ => None,
    })
    .await
}

#[tokio::test]
async fn leaving_the_group_in_its_copy_gives_a_minute_then_home() {
    let realm = dungeon();
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let bob = b.enter("Bob", Race::Human, Class::Mage).await;
    let mut c = Client::connect(&realm, "C");
    let carol = c.enter("Carol", Race::Human, Class::Priest).await;
    group(&mut a, &mut b, "Bob").await;
    group(&mut a, &mut c, "Carol").await;
    for cl in [&mut a, &mut b, &mut c] {
        go(&realm, cl, 36).await;
    }
    let copy = map_of(&realm, alice).key;
    assert_eq!(map_of(&realm, carol).key, copy);
    // Carol's home, in Elwynn.
    map_of(&realm, carol)
        .query(move |m| {
            m.players.get_mut(&carol).unwrap().row.state.bind = Some(crate::game::store::Bind {
                map: 0,
                zone: 12,
                x: -8949.0,
                y: -132.0,
                z: 83.5,
            });
        })
        .await;
    // She leaves the group: a minute, and Alice and Bob stay.
    c.send(CMSG_GROUP_DISBAND {}).await;
    assert_eq!(grace(&mut c).await, 60_000);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let pending = map_of(&realm, alice)
        .query(move |m| {
            (
                m.players[&alice].eviction.is_some(),
                m.players[&bob].eviction.is_some(),
                m.players[&carol].eviction.is_some(),
            )
        })
        .await
        .unwrap();
    assert_eq!(pending, (false, false, true));
    // The minute over: home, and told of the copy left.
    map_of(&realm, carol)
        .query(move |m| {
            m.players.get_mut(&carol).unwrap().eviction = Some(Instant::now());
        })
        .await;
    c.wait_for("SMSG_NEW_WORLD", |m| match m {
        ServerOpcodeMessage::SMSG_NEW_WORLD(w) => (w.map == Map::EasternKingdoms).then_some(()),
        _ => None,
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    let left = c
        .wait_for("SMSG_UPDATE_LAST_INSTANCE", |m| match m {
            ServerOpcodeMessage::SMSG_UPDATE_LAST_INSTANCE(l) => Some(l.map),
            _ => None,
        })
        .await;
    assert_eq!(left.as_int(), 36);
    assert_eq!(map_of(&realm, carol).key, MapKey::world(0));
}

#[tokio::test]
async fn back_in_the_group_in_time_one_stays() {
    let realm = dungeon();
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Mage).await;
    let mut c = Client::connect(&realm, "C");
    let carol = c.enter("Carol", Race::Human, Class::Priest).await;
    group(&mut a, &mut b, "Bob").await;
    group(&mut a, &mut c, "Carol").await;
    for cl in [&mut a, &mut c] {
        go(&realm, cl, 36).await;
    }
    c.send(CMSG_GROUP_DISBAND {}).await;
    assert_eq!(grace(&mut c).await, 60_000);
    group(&mut a, &mut c, "Carol").await;
    assert_eq!(grace(&mut c).await, 0, "all clear");
    let pending = map_of(&realm, alice)
        .query(move |m| m.players[&carol].eviction.is_some())
        .await
        .unwrap();
    assert!(!pending);
}

#[tokio::test]
async fn a_group_formed_in_its_leaders_copy_takes_it_over() {
    let realm = dungeon();
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let bob = b.enter("Bob", Race::Human, Class::Mage).await;
    go(&realm, &mut a, 36).await;
    let copy = map_of(&realm, alice).key;
    group(&mut a, &mut b, "Bob").await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let pending = map_of(&realm, alice)
        .query(move |m| m.players[&alice].eviction.is_some())
        .await
        .unwrap();
    assert!(!pending, "hers is the group's now");
    go(&realm, &mut b, 36).await;
    assert_eq!(map_of(&realm, bob).key, copy, "Bob goes into it");
}

#[tokio::test]
async fn a_raid_turns_away_one_not_in_a_raid_group_as_thistle_tea_does() {
    let realm = dungeon();
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    // Saved to no raid, as login says.
    let saved = a
        .wait_for("SMSG_UPDATE_INSTANCE_OWNERSHIP", |m| match m {
            ServerOpcodeMessage::SMSG_UPDATE_INSTANCE_OWNERSHIP(o) => {
                Some(o.player_is_saved_to_a_raid)
            }
            _ => None,
        })
        .await;
    assert!(!saved);
    map_of(&realm, alice).call(move |m| {
        m.teleport(
            alice,
            MapKey::world(409),
            Vector3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
        )
    });
    assert_eq!(grace(&mut a).await, 0);
    assert_eq!(map_of(&realm, alice).key.instance, 0, "she stays out");
}

#[tokio::test]
async fn one_still_queued_can_leave_the_queue() {
    let realm = harness::realm_with(gulch());
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    queue(&a, Map::WarsongGulch).await;
    assert_eq!(joined(&mut a).await, BgTypeId::QueuedForWsg);
    a.send(CMSG_BATTLEFIELD_PORT {
        map: Map::WarsongGulch,
        action: BattlefieldPortAction::LeaveQueue,
    })
    .await;
    a.wait_frame(
        "the status cleared",
        crate::game::battlegrounds::SMSG_BATTLEFIELD_STATUS,
        |f| f.len() == 8,
    )
    .await;
    // Out of it: Grom's queueing makes no match with her.
    let mut g = Client::connect(&realm, "G");
    let grom = g.enter("Grom", Race::Orc, Class::Warrior).await;
    queue(&g, Map::WarsongGulch).await;
    joined(&mut g).await;
    assert!(match_of(&realm, alice).is_none());
    assert!(match_of(&realm, grom).is_none());
}
