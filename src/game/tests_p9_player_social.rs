//! Round 9's player: the login, copies, meeting stones, guilds and mail
//! (Thistle Tea's `cinematic_test`, `login_test`, `state_test`,
//! `meeting_stones_test`, `guilds_test` and `mail/client_projection_test`).

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::scripts::ScriptStep;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::inventory::PACK_START;
use crate::game::map::{Map, MapHandle, MapKey};
use crate::game::store::BAG_PLAYER;
use crate::game::tests_p3_combat::{BRUTE, START};

const STONE: u32 = 98_900;
const STONE_SPAWN: u32 = 890;
const PARCEL: u32 = 63_990;
const JUNK: u32 = 63_991;

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

async fn handled(c: &mut Client) {
    crate::game::tests_p3_player2_ammo::handled(c).await;
}

fn data() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.races.insert(
        1,
        RaceInfo {
            cinematic: 81,
            ..Default::default()
        },
    );
    d.misc.cinematic_waypoints.insert(
        81,
        vec![
            (0, START.0, START.1, START.2 + 10.0),
            (30_000, START.0 + 50.0, START.1, START.2 + 10.0),
        ],
    );
    for (id, name, kind) in [(36, "DeadminesInstance", 1)] {
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
    d.gameobject_templates.insert(
        STONE,
        crate::game::tests_p3_entity_objects::object(STONE, 5, "Stone", &[]),
    );
    d.gameobject_spawns
        .entry(0)
        .or_default()
        .push(GameObjectSpawn {
            guid: STONE_SPAWN,
            entry: STONE,
            map: 0,
            x: START.0 + 3.0,
            y: START.1,
            z: START.2,
            state: 1,
            ..Default::default()
        });
    // A script of the stone's: queue for the Deadmines (area 1581).
    d.scripts.gameobject.insert(
        STONE_SPAWN,
        vec![ScriptStep {
            command: 36,
            datalong: [1581, 0, 0, 0],
            ..Default::default()
        }],
    );
    for e in [PARCEL, JUNK] {
        d.items.insert(
            e,
            ItemTemplate {
                entry: e,
                name: format!("Item {e}"),
                display_id: 1000,
                stackable: 1,
                allowable_class: -1,
                allowable_race: -1,
                ..Default::default()
            },
        );
    }
    d
}

fn realm() -> Arc<Realm> {
    harness::realm_with(data())
}

/// Thistle Tea's "waits for the player to enter the world before its
/// clock starts": a first login's intro camera starts its clock as the
/// player enters the world, not as the character was made.
#[tokio::test]
async fn the_intro_cameras_clock_starts_at_the_login() {
    let realm = realm();
    let map = realm.maps.get(&realm, MapKey::world(0));
    let mut a = Client::connect(&realm, "A");
    a.send(CMSG_CHAR_CREATE {
        name: "Alice".into(),
        race: Race::Human,
        class: Class::Warrior,
        gender: Gender::Female,
        skin_color: 1,
        face: 1,
        hair_style: 1,
        hair_color: 1,
        facial_hair: 1,
    })
    .await;
    a.wait_for("SMSG_CHAR_CREATE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_CHAR_CREATE(_)).then_some(())
    })
    .await;
    a.send(CMSG_CHAR_ENUM {}).await;
    let g = a
        .wait_for("SMSG_CHAR_ENUM", |m| match m {
            ServerOpcodeMessage::SMSG_CHAR_ENUM(e) => e.characters.first().map(|c| c.guid),
            _ => None,
        })
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = on(&map, |m| m.now).await;
    a.login(g).await;
    a.wait_for("SMSG_TRIGGER_CINEMATIC", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_TRIGGER_CINEMATIC(_)).then_some(())
    })
    .await;
    let started = on(&map, move |m| {
        m.players[&g].misc.camera.as_ref().map(|c| c.started)
    })
    .await
    .expect("the camera under way");
    assert!(started >= before, "its clock from the login");
}

/// Thistle Tea's "keeps a saved pet suspended while a taxi route is
/// unfinished": a hunter back on a flight they logged out on has no pet
/// out, called neither out nor away, until the flight lands, when it
/// comes back.
#[tokio::test]
async fn a_pet_waits_out_a_flight_resumed_at_a_relog() {
    let realm = realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Hunter", Race::Dwarf, Class::Hunter).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    on(&map, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.row.state.pets.current = Some(crate::game::store::PetRecord {
            number: 5,
            entry: BRUTE,
            kind: 1,
            name: "Wolf".into(),
            level: 1,
            ..Default::default()
        });
        p.row.state.pets.active = true;
        p.row.state.flight = Some(crate::game::store::SavedFlight {
            map: 0,
            points: vec![(START.0 + 2000.0, START.1, START.2 + 20.0)],
            destination: 2,
            mount: 1149,
        });
        crate::game::map::npc::resume_flight(m, g);
        assert!(m.players[&g].flight.is_some());
    })
    .await;
    a.disconnect();
    harness::offline(&realm, g).await;
    let mut a = Client::connect(&realm, "A");
    a.login(g).await;
    let back = on(&map, move |m| {
        let p = &m.players[&g];
        (
            p.flight.is_some(),
            p.owned.pet.is_none(),
            p.owned.unsummoned,
        )
    })
    .await;
    assert_eq!(back, (true, true, true), "flying, the pet waiting");
    harness::an_update(&map).await;
    harness::an_update(&map).await;
    let creates: Vec<Guid> = a
        .captured()
        .iter()
        .filter_map(|f| ServerOpcodeMessage::read_unencrypted(&f[..]).ok())
        .flat_map(|m| harness::creates(&m))
        .filter(|c| guid::is_unit(*c) && !guid::is_player(*c))
        .collect();
    assert!(creates.is_empty(), "no pet called out: {creates:?}");
    // Landed: the pet comes back.
    on(&map, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.flight = None;
        p.fields.set_u32(UNIT_FIELD_MOUNTDISPLAYID, 0);
    })
    .await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while on(&map, move |m| m.players[&g].owned.pet.is_none()).await {
        assert!(tokio::time::Instant::now() < deadline, "the pet back");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Thistle Tea's "does not record an instance on entry": going into a
/// dungeon's copy from the open world tells the client of no copy left;
/// coming out of it does (`SMSG_UPDATE_LAST_INSTANCE`).
#[tokio::test]
async fn entering_a_copy_records_no_last_instance() {
    let realm = realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let to = |map: u32| {
        let realm = realm.clone();
        move || {
            realm.registry.online(g).unwrap().map().call(move |m| {
                m.teleport(g, MapKey::world(map), START_AT, 0.0);
            });
        }
    };
    const START_AT: Vector3d = Vector3d {
        x: START.0,
        y: START.1,
        z: START.2,
    };
    to(36)();
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    handled(&mut a).await;
    let told = a
        .gets(Duration::from_millis(200), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_UPDATE_LAST_INSTANCE(_)).then_some(())
        })
        .await;
    assert!(told.is_none(), "nothing left on entering");
    let copy = realm.registry.online(g).unwrap().map();
    assert!(on(&copy, move |m| m.players[&g].last_instance.is_none()).await);
    to(0)();
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    let left = a
        .wait_for("SMSG_UPDATE_LAST_INSTANCE", |m| match m {
            ServerOpcodeMessage::SMSG_UPDATE_LAST_INSTANCE(l) => Some(l.map.as_int()),
            _ => None,
        })
        .await;
    assert_eq!(left, 36, "the copy left, coming out");
}

/// Thistle Tea's "retains the source world and uses the explicit player
/// owner": a script's meeting-stone step (36), run for a player at an
/// object, queues the player for its area, told so; the realm-wide queue
/// holds them where they are.
#[tokio::test]
async fn a_scripts_meeting_stone_step_queues_its_player() {
    let realm = realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    a.clear();
    on(&map, move |m| {
        let stone = guid::game_object(STONE, STONE_SPAWN);
        crate::game::map::scripts::object_scripts(m, g, stone);
        crate::game::map::scripts::update(m);
    })
    .await;
    let queued = a
        .wait_for("SMSG_MEETINGSTONE_SETQUEUE", |m| match m {
            ServerOpcodeMessage::SMSG_MEETINGSTONE_SETQUEUE(q) => Some((q.area.as_int(), q.status)),
            _ => None,
        })
        .await;
    assert_eq!(queued, (1581, MeetingStoneStatus::JoinedQueue));
    let held = realm.social.meeting_stones.lock().of(g).map(|q| q.area);
    assert_eq!(held, Some(1581));
}

/// Thistle Tea's "rank changes and disband clear an offline member's
/// saved projection": an offline member promoted is of the new rank at
/// their next login; the guild disbanded while they are away, they come
/// back in none.
#[tokio::test]
async fn an_offline_members_rank_and_guild_follow_the_guilds_changes() {
    let realm = realm();
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.send(CMSG_GUILD_CREATE {
        guild_name: "Lions".into(),
    })
    .await;
    handled(&mut a).await;
    a.send(CMSG_GUILD_INVITE {
        invited_player: "Bob".into(),
    })
    .await;
    handled(&mut a).await;
    b.send(CMSG_GUILD_ACCEPT {}).await;
    handled(&mut b).await;
    let rank = |realm: &Arc<Realm>| {
        let guilds = realm.social.guilds.lock();
        guilds
            .of(gb)
            .and_then(|g| g.members.iter().find(|m| m.guid == guid::counter(gb)))
            .map(|m| m.rank)
    };
    assert_eq!(rank(&realm), Some(4), "an initiate");
    b.disconnect();
    harness::offline(&realm, gb).await;
    a.send(CMSG_GUILD_PROMOTE {
        player_name: "Bob".into(),
    })
    .await;
    handled(&mut a).await;
    assert_eq!(rank(&realm), Some(3), "promoted while away");
    let fields = |realm: Arc<Realm>| async move {
        let mut b = Client::connect(&realm, "B");
        b.login(gb).await;
        let map = realm.registry.online(gb).unwrap().map();
        let f = on(&map, move |m| {
            let p = &m.players[&gb];
            (
                p.fields.get_u32(PLAYER_GUILDID),
                p.fields.get_u32(PLAYER_GUILDRANK),
            )
        })
        .await;
        b.disconnect();
        harness::offline(&realm, gb).await;
        f
    };
    let (id, at) = fields(realm.clone()).await;
    assert!(id != 0 && at == 3, "{id} {at}");
    a.send(CMSG_GUILD_DISBAND {}).await;
    handled(&mut a).await;
    assert_eq!(rank(&realm), None);
    assert_eq!(fields(realm.clone()).await, (0, 0), "in no guild");
}

/// Thistle Tea's "maps semantic outcomes to protocol fields": a mail's
/// item taken with full bags is answered as an item taken with the
/// inventory's error (`ErrEquipError`, InventoryFull), and the mail keeps
/// it; with room, as taken.
#[tokio::test]
async fn a_mails_item_taken_with_full_bags_is_its_equip_error() {
    let realm = realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    on(&map, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items
            .retain(|i| i.bag != BAG_PLAYER || i.slot < PACK_START);
        p.refresh_inventory(&d);
        for _ in 0..16 {
            crate::game::map::inventory::add_item(m, g, JUNK, 1, false).unwrap();
        }
    })
    .await;
    let now = crate::game::social::mail::now();
    let id = crate::game::social::mail::post(
        &realm,
        crate::game::social::mail::Mail {
            kind: crate::game::social::mail::NORMAL,
            sender: guid::counter(g),
            receiver: guid::counter(g),
            subject: "A parcel".into(),
            item: Some(crate::game::store::ItemRow {
                guid: realm
                    .next_item
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                entry: PARCEL,
                count: 1,
                ..Default::default()
            }),
            deliver_at: now,
            expires_at: now + 86_400,
            ..Default::default()
        },
    );
    a.clear();
    a.send(CMSG_MAIL_TAKE_ITEM {
        mailbox: Guid::zero(),
        mail_id: id,
    })
    .await;
    let full = a
        .wait_for("SMSG_SEND_MAIL_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_SEND_MAIL_RESULT(r) if r.mail_id == id => {
                Some(r.action.clone())
            }
            _ => None,
        })
        .await;
    assert_eq!(
        full,
        SMSG_SEND_MAIL_RESULT_MailAction::ItemTaken {
            result: SMSG_SEND_MAIL_RESULT_MailResult::ErrEquipError {
                equip_error: InventoryResult::InventoryFull.as_int().into(),
            },
        }
    );
    assert!(realm.social.mail.lock().mails[&id].item.is_some(), "kept");
    on(&map, move |m| {
        let i = m.players[&g].item_at(BAG_PLAYER, PACK_START).unwrap();
        crate::game::map::inventory::take_item(m, g, i, 1);
    })
    .await;
    a.send(CMSG_MAIL_TAKE_ITEM {
        mailbox: Guid::zero(),
        mail_id: id,
    })
    .await;
    let taken = a
        .wait_for("SMSG_SEND_MAIL_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_SEND_MAIL_RESULT(r) if r.mail_id == id => {
                Some(r.action.clone())
            }
            _ => None,
        })
        .await;
    assert_eq!(
        taken,
        SMSG_SEND_MAIL_RESULT_MailAction::ItemTaken {
            result: SMSG_SEND_MAIL_RESULT_MailResult::Ok {
                item: PARCEL,
                item_count: 1,
            },
        }
    );
}
