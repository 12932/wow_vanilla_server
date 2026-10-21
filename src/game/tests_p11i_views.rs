//! Round 11's test-only rows on who is sent what (Thistle Tea's
//! `update_object_test.exs`, `world_test.exs`, `power_transfer_test.exs`
//! and `event_sink/movement_test.exs`): a pet's experience to its master
//! alone, a broadcast that leaves out its source and stays in its own
//! copy, the power logs to every observer on the map and no one off it,
//! and a feared player's run and stop to those who see them.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::{GameData, MapInfo};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::{MapHandle, MapKey};
use crate::game::tests_bugs6::fields;
use crate::game::tests_pets::{WOLF, enter};

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut crate::game::map::Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

/// The fields of `c`'s create of `of`.
async fn create_of(c: &mut Client, of: Guid) -> std::collections::BTreeMap<u16, u32> {
    c.wait_for("its create", |m| {
        let objects = match m {
            ServerOpcodeMessage::SMSG_UPDATE_OBJECT(u) => &u.objects,
            ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(u) => &u.objects,
            _ => return None,
        };
        objects.iter().find_map(|o| match &o.update_type {
            Object_UpdateType::CreateObject { guid3, mask2, .. }
            | Object_UpdateType::CreateObject2 { guid3, mask2, .. }
                if *guid3 == of =>
            {
                Some(fields(mask2))
            }
            _ => None,
        })
    })
    .await
}

/// Thistle Tea's "sends full pet stats to its owner and public fields to
/// other players": a pet's experience and the experience to its next
/// level (`UNIT_FIELD_PETEXPERIENCE`, `UNIT_FIELD_PETNEXTLEVELEXP`,
/// mangos's `UF_FLAG_OWNER_ONLY`) are in its master's create and changes,
/// and in no one else's; its health is everyone's.
#[tokio::test]
async fn a_pets_experience_goes_to_its_master_alone() {
    let realm = harness::realm_with(crate::game::tests_pets::data());
    let (mut a, ga, map) = enter(&realm, "A", "Ari", Race::Dwarf, Class::Hunter).await;
    let (mut b, _gb, _) = enter(&realm, "B", "Bo", Race::Dwarf, Class::Hunter).await;
    let pet = on(&map, move |m| {
        let t = m.realm.data.creature_template(WOLF).cloned().unwrap();
        let at = m.players[&ga].position();
        let mut c = Creature::new(guid::unit(WOLF, 0x71_1001), &t, 1, at, 0.0);
        c.owner = Some(ga);
        c.fields.set_guid(UNIT_FIELD_SUMMONEDBY, ga);
        c.fields.set_u32(UNIT_FIELD_PETEXPERIENCE, 123);
        c.fields.set_u32(UNIT_FIELD_PETNEXTLEVELEXP, 35_300);
        m.add_creature(c)
    })
    .await;
    let mine = create_of(&mut a, pet).await;
    let seen = create_of(&mut b, pet).await;
    assert_eq!(mine.get(&UNIT_FIELD_PETEXPERIENCE), Some(&123), "{mine:?}");
    assert_eq!(mine.get(&UNIT_FIELD_PETNEXTLEVELEXP), Some(&35_300));
    assert!(seen.contains_key(&UNIT_FIELD_HEALTH), "{seen:?}");
    assert!(
        !seen.contains_key(&UNIT_FIELD_PETEXPERIENCE)
            && !seen.contains_key(&UNIT_FIELD_PETNEXTLEVELEXP),
        "another's: {seen:?}"
    );
    a.clear();
    b.clear();
    on(&map, move |m| {
        let f = &mut m.creature_mut(pet).unwrap().fields;
        f.set_u32(UNIT_FIELD_PETEXPERIENCE, 200);
        f.set_u32(UNIT_FIELD_HEALTH, 3);
    })
    .await;
    let told = a
        .wait_for("its experience", |m| {
            harness::values(m)
                .into_iter()
                .filter(|(g, _)| *g == pet)
                .map(|(_, mask)| fields(&mask))
                .find(|f| f.contains_key(&UNIT_FIELD_PETEXPERIENCE))
        })
        .await;
    assert_eq!(told.get(&UNIT_FIELD_PETEXPERIENCE), Some(&200));
    let theirs = b
        .wait_for("its health", |m| {
            harness::values(m)
                .into_iter()
                .filter(|(g, _)| *g == pet)
                .map(|(_, mask)| fields(&mask))
                .find(|f| f.contains_key(&UNIT_FIELD_HEALTH))
        })
        .await;
    assert!(
        !theirs.contains_key(&UNIT_FIELD_PETEXPERIENCE),
        "not another: {theirs:?}"
    );
}

const DEADMINES: u32 = 36;

fn dungeon() -> GameData {
    let mut d = GameData::empty();
    d.maps.insert(
        DEADMINES,
        MapInfo {
            id: DEADMINES,
            internal_name: "DeadminesInstance".into(),
            name: "The Deadmines".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    d
}

/// Send `c` to `map` at the origin (its copy, for a dungeon) and wait for
/// it to land.
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

fn emote(m: &ServerOpcodeMessage, of: Guid, id: Emote) -> Option<()> {
    match m {
        ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == of && e.emote == id => Some(()),
        _ => None,
    }
}

/// Thistle Tea's "integer sources retain observer attribution and
/// self-exclusion in their own world": a broadcast about a player
/// (`Map::send_to_observers`) reaches those who see them in their own
/// copy, them too or not as it says, and never a player in another copy
/// of the same dungeon at the same spot (each copy is its own map actor).
#[tokio::test]
async fn a_broadcast_leaves_out_its_source_when_asked_and_stays_in_its_copy() {
    let realm = harness::realm_with(dungeon());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    c.enter("Cid", Race::Human, Class::Warrior).await;
    group(&mut a, &mut b, "Bob").await;
    let copy = go(&realm, &mut a, DEADMINES).await;
    assert_eq!(
        go(&realm, &mut b, DEADMINES).await,
        copy,
        "the party's copy"
    );
    let other = go(&realm, &mut c, DEADMINES).await;
    assert_ne!(other, copy, "Cid's own copy");
    b.sees(ga).await;
    let map = realm.registry.online(ga).unwrap().map();
    a.clear();
    b.clear();
    c.clear();
    on(&map, move |m| {
        let frame =
            |emote| crate::game::conn::encode(&SMSG_EMOTE { emote, guid: ga }).expect("encoded");
        m.send_to_observers(ga, &frame(Emote::OneshotWave), true);
        m.send_to_observers(ga, &frame(Emote::OneshotBow), false);
    })
    .await;
    b.wait_for("the first", |m| emote(m, ga, Emote::OneshotWave))
        .await;
    b.wait_for("the second", |m| emote(m, ga, Emote::OneshotBow))
        .await;
    a.wait_for("her own first", |m| emote(m, ga, Emote::OneshotWave))
        .await;
    let quiet = Duration::from_millis(300);
    assert!(
        a.gets(quiet, |m| emote(m, ga, Emote::OneshotBow))
            .await
            .is_none(),
        "not her own second"
    );
    assert!(
        c.gets(quiet, |m| match m {
            ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == ga || e.guid == gb => Some(()),
            _ => None,
        })
        .await
        .is_none(),
        "nothing in another copy"
    );
}

/// Thistle Tea's "resource combat logs reach the owner and observers only
/// in the same world": an energize's `SMSG_SPELLENERGIZELOG`, a drain's
/// `SMSG_SPELLLOGEXECUTE` and a mana leech's tick
/// (`SMSG_PERIODICAURALOG`) reach the caster and a player beside them,
/// and not a player at the same spot on another map.
#[tokio::test]
async fn the_power_logs_reach_the_caster_and_observers_on_their_map_alone() {
    use crate::game::tests_p3_combat3_power::{
        DRAIN, ENERGIZE, LEECHING, data, mana_user, set_pools, trigger,
    };
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Mage).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Mage).await;
    let mut c = Client::connect(&realm, "C");
    c.enter("Cid", Race::Human, Class::Mage).await;
    let map = realm.registry.online(ga).unwrap().map();
    // Cid at Alice's spot, on Kalimdor.
    let at = on(&map, move |m| m.players[&ga].position()).await;
    let gc = c.guid;
    map.call(move |m| m.teleport(gc, MapKey::world(1), at, 0.0));
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(gc).await;
    let target = on(&map, move |m| {
        let t = mana_user(m, ga, 0);
        set_pools(m, t, 1000, (80, 150));
        t
    })
    .await;
    a.sees(target).await;
    b.sees(target).await;
    c.clear();
    on(&map, move |m| {
        trigger(m, ga, ENERGIZE, target);
        trigger(m, ga, DRAIN, target);
        trigger(m, ga, LEECHING, target);
        crate::game::tests_p3_spells_kit::tick_now(m, target, LEECHING);
    })
    .await;
    let logs = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::SMSG_SPELLENERGIZELOG(l) if l.spell == ENERGIZE => Some(1),
        ServerOpcodeMessage::SMSG_SPELLLOGEXECUTE(l) if l.spell == DRAIN => Some(2),
        ServerOpcodeMessage::SMSG_PERIODICAURALOG(l) if l.spell == LEECHING => Some(3),
        _ => None,
    };
    for (who, client) in [("the caster", &mut a), ("an observer", &mut b)] {
        for want in 1..=3 {
            client
                .wait_for(&format!("log {want} for {who}"), |m| {
                    logs(m).filter(|n| *n == want)
                })
                .await;
        }
    }
    assert_eq!(
        c.gets(Duration::from_millis(300), logs).await,
        None,
        "not another map"
    );
}

/// Thistle Tea's "player control uses its explicit owner and movement
/// reaches owner and observers": a feared player's control goes from
/// their own client alone; the run the server drives them on
/// (`SMSG_MONSTER_MOVE`) and its stop at the fear's end reach both them
/// and a player who sees them.
#[tokio::test]
async fn a_feared_players_run_and_stop_reach_their_observers() {
    use crate::game::tests_combat3::{DUMMY, SCARE, START, apply, control, data, unit};
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    b.heartbeat(START.0, START.1, START.2).await;
    a.sees(unit(DUMMY)).await;
    b.sees(ga).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    b.clear();
    let dummy = unit(DUMMY);
    on(&map, move |m| apply(m, dummy, ga, SCARE, 0)).await;
    assert!(!a.wait_for("control taken", |m| control(m, ga)).await);
    let run = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv)
            if mv.guid == ga
                && !matches!(mv.move_type, SMSG_MONSTER_MOVE_MonsterMoveType::Stop) =>
        {
            Some(())
        }
        _ => None,
    };
    let stop = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv)
            if mv.guid == ga && matches!(mv.move_type, SMSG_MONSTER_MOVE_MonsterMoveType::Stop) =>
        {
            Some(())
        }
        _ => None,
    };
    a.wait_for("her run", run).await;
    b.wait_for("her run, for him", run).await;
    assert_eq!(
        b.gets(Duration::from_millis(100), |m| control(m, ga)).await,
        None,
        "her control is her client's alone"
    );
    on(&map, move |m| {
        crate::game::map::auras::remove_spell(m, ga, SCARE, None)
    })
    .await;
    a.wait_for("her stop", stop).await;
    b.wait_for("her stop, for him", stop).await;
    assert!(a.wait_for("control back", |m| control(m, ga)).await);
}
