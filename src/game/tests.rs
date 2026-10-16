//! The world through the protocol: clients on the harness, against real
//! sessions and maps.

use std::time::{Duration, Instant};

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client, values};

const NONE_WITHIN: Duration = Duration::from_millis(300);

async fn two_near() -> (std::sync::Arc<crate::game::Realm>, Client, Client) {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Mage).await;
    let (ga, gb) = (a.guid, b.guid);
    a.sees(gb).await;
    b.sees(ga).await;
    (realm, a, b)
}

#[tokio::test]
async fn a_new_character_enters_the_world_with_the_login_packets() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    for what in [
        "SMSG_ACCOUNT_DATA_TIMES",
        "SMSG_TUTORIAL_FLAGS",
        "SMSG_INITIAL_SPELLS",
        "SMSG_ACTION_BUTTONS",
        "SMSG_LOGIN_SETTIMESPEED",
    ] {
        a.wait_for(what, |m| (m.to_string() == what).then_some(()))
            .await;
    }
    // Its own create block, as the client's own player.
    a.sees(g).await;
    assert!(guid::is_player(g));
}

#[tokio::test]
async fn players_near_each_other_see_each_other_and_their_movement_at_once() {
    let (_realm, mut a, b) = two_near().await;
    a.clear();
    let started = Instant::now();
    b.heartbeat(-8949.0, -132.0, 83.5).await;
    let from = a
        .wait_for("a heartbeat", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(h) => Some(h.guid),
            _ => None,
        })
        .await;
    assert_eq!(from, b.guid);
    // Relayed as it arrives, not on an update.
    assert!(
        started.elapsed() < Duration::from_millis(40),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_say_reaches_players_near_and_not_far() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    c.enter("Carol", Race::Human, Class::Warrior).await;
    // Carol walks 60 yards off: she still sees Alice, but can't hear a say.
    c.heartbeat(-8949.0 + 60.0, -132.0, 83.5).await;
    tokio::time::sleep(Duration::from_millis(120)).await;
    c.clear();
    b.clear();
    a.say("hello there").await;
    let heard = b
        .wait_for("a say", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => Some(c.message.clone()),
            _ => None,
        })
        .await;
    assert_eq!(heard, "hello there");
    let far = c
        .gets(NONE_WITHIN, |m| {
            matches!(m, ServerOpcodeMessage::SMSG_MESSAGECHAT(_)).then_some(())
        })
        .await;
    assert!(far.is_none());
    // A yell carries.
    a.send(CMSG_MESSAGECHAT {
        chat_type: CMSG_MESSAGECHAT_ChatType::Yell,
        language: Language::Common,
        message: "over here".into(),
    })
    .await;
    c.wait_for("a yell", |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c) if c.message == "over here" => Some(()),
        _ => None,
    })
    .await;
}

#[tokio::test]
async fn a_name_query_is_answered_from_memory() {
    let (_realm, mut a, b) = two_near().await;
    let started = Instant::now();
    a.send(CMSG_NAME_QUERY { guid: b.guid }).await;
    let name = a
        .wait_for("a name", |m| match m {
            ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(r) => Some(r.character_name.clone()),
            _ => None,
        })
        .await;
    assert_eq!(name, "Bob");
    assert!(started.elapsed() < Duration::from_millis(40));
}

#[tokio::test]
async fn a_player_who_walks_away_is_forgotten_and_seen_again_on_return() {
    let (_realm, mut a, b) = two_near().await;
    b.heartbeat(-8949.0 + 300.0, -132.0, 83.5).await;
    a.forgets(b.guid).await;
    b.heartbeat(-8949.0 + 5.0, -132.0, 83.5).await;
    a.sees(b.guid).await;
}

#[tokio::test]
async fn sitting_and_targeting_reach_observers_as_field_updates() {
    let (_realm, mut a, mut b) = two_near().await;
    a.clear();
    b.send(CMSG_STANDSTATECHANGE {
        animation_state: UnitStandState::Sit,
    })
    .await;
    b.wait_for("SMSG_STANDSTATE_UPDATE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_STANDSTATE_UPDATE(_)).then_some(())
    })
    .await;
    let bg = b.guid;
    a.wait_for("Bob sitting", |m| {
        values(m).iter().find_map(|(g, mask)| match mask {
            UpdateMask::Player(p) if *g == bg => p.unit_bytes_1().filter(|b| b.0 == 1).map(|_| ()),
            _ => None,
        })
    })
    .await;
    let ag = a.guid;
    b.send(CMSG_SET_SELECTION { target: ag }).await;
    a.wait_for("Bob's target", |m| {
        values(m).iter().find_map(|(g, mask)| match mask {
            UpdateMask::Player(p) if *g == bg => p.unit_target().filter(|t| *t == ag),
            _ => None,
        })
    })
    .await;
}

#[tokio::test]
async fn a_logged_out_character_keeps_where_it_was() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(-8900.0, -100.0, 84.0).await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.send(CMSG_CHAR_ENUM {}).await;
    let pos = a
        .wait_for("SMSG_CHAR_ENUM", |m| match m {
            ServerOpcodeMessage::SMSG_CHAR_ENUM(e) => e.characters.first().map(|c| c.position),
            _ => None,
        })
        .await;
    assert_eq!((pos.x, pos.y), (-8900.0, -100.0));
    // And logs back in where it left.
    a.login(g).await;
}

#[tokio::test]
async fn a_logout_waits_twenty_seconds_outside_an_inn_and_can_be_cancelled() {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, Default::default(), None);
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    let speed = a
        .wait_for("SMSG_LOGOUT_RESPONSE", |m| match m {
            ServerOpcodeMessage::SMSG_LOGOUT_RESPONSE(r) => Some(r.speed),
            _ => None,
        })
        .await;
    assert_eq!(speed, LogoutSpeed::Delayed);
    a.wait_for("a root", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_FORCE_MOVE_ROOT(_)).then_some(())
    })
    .await;
    a.send(CMSG_LOGOUT_CANCEL {}).await;
    a.wait_for("SMSG_LOGOUT_CANCEL_ACK", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_CANCEL_ACK).then_some(())
    })
    .await;
    // A non-GM can't use commands.
    a.say(".gps").await;
    a.system_message("can't use commands").await;
}

#[tokio::test]
async fn a_disconnected_player_is_forgotten_by_everyone() {
    let (_realm, mut a, b) = two_near().await;
    let bg = b.guid;
    b.disconnect();
    a.forgets(bg).await;
}

#[tokio::test]
async fn a_second_login_on_the_account_kicks_the_first() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut again = Client::connect(&realm, "A");
    again.send(CMSG_PLAYER_LOGIN { guid: g }).await;
    let result = again
        .wait_for("SMSG_CHARACTER_LOGIN_FAILED", |m| match m {
            ServerOpcodeMessage::SMSG_CHARACTER_LOGIN_FAILED(f) => Some(f.result),
            _ => None,
        })
        .await;
    assert_eq!(result, WorldResult::CharLoginDuplicateCharacter);
    // The first is gone from the world; the second gets in.
    tokio::time::sleep(Duration::from_millis(100)).await;
    again.login(g).await;
}

#[tokio::test]
async fn go_teleports_within_a_map_and_to_another() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.say(".go -9000 -100 90").await;
    let to = a
        .wait_for("MSG_MOVE_TELEPORT_ACK", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => Some(t.info.position),
            _ => None,
        })
        .await;
    assert_eq!((to.x, to.y), (-9000.0, -100.0));
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: a.guid,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
    a.say(".go xyz 1629 -4373 31 1").await;
    a.wait_for("SMSG_TRANSFER_PENDING", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_TRANSFER_PENDING(_)).then_some(())
    })
    .await;
    let map = a
        .wait_for("SMSG_NEW_WORLD", |m| match m {
            ServerOpcodeMessage::SMSG_NEW_WORLD(w) => Some(w.map),
            _ => None,
        })
        .await;
    assert_eq!(map, Map::Kalimdor);
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    let g = a.guid;
    a.sees(g).await;
    a.say(".gps").await;
    let gps = a.system_message("map 1").await;
    assert!(gps.contains("x 1629.00"), "{gps}");
}

#[tokio::test]
async fn a_spawned_creature_is_seen_and_its_name_answered() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.clear();
    a.say(".spawn 0").await;
    let created = a
        .wait_for("a creature", |m| {
            harness::creates(m).into_iter().find(|g| guid::is_unit(*g))
        })
        .await;
    a.send(CMSG_CREATURE_QUERY {
        creature: guid::entry(created),
        guid: created,
    })
    .await;
    a.wait_for("SMSG_CREATURE_QUERY_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_CREATURE_QUERY_RESPONSE(_)).then_some(())
    })
    .await;
}

#[tokio::test]
async fn emotes_and_text_emotes_reach_those_near() {
    let (_realm, mut a, b) = two_near().await;
    a.clear();
    b.send(CMSG_TEXT_EMOTE {
        text_emote: TextEmote::Wave,
        emote: 0,
        target: a.guid,
    })
    .await;
    let name = a
        .wait_for("SMSG_TEXT_EMOTE", |m| match m {
            ServerOpcodeMessage::SMSG_TEXT_EMOTE(e) => Some(e.name.clone()),
            _ => None,
        })
        .await;
    assert_eq!(name, "Alice");
    b.send(CMSG_EMOTE {
        emote: Emote::OneshotWave,
    })
    .await;
    a.wait_for("SMSG_EMOTE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_EMOTE(_)).then_some(())
    })
    .await;
}

#[tokio::test]
async fn played_time_and_server_time_are_answered() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.send(CMSG_PLAYED_TIME {}).await;
    a.wait_for("SMSG_PLAYED_TIME", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_PLAYED_TIME(_)).then_some(())
    })
    .await;
    a.send(CMSG_QUERY_TIME {}).await;
    a.wait_for("SMSG_QUERY_TIME_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_QUERY_TIME_RESPONSE(_)).then_some(())
    })
    .await;
}

#[tokio::test]
async fn a_character_can_be_deleted_but_not_while_online() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut other = Client::connect(&realm, "A2");
    other.send(CMSG_CHAR_DELETE { guid: g }).await;
    let r = other
        .wait_for("SMSG_CHAR_DELETE", |m| match m {
            ServerOpcodeMessage::SMSG_CHAR_DELETE(d) => Some(d.result),
            _ => None,
        })
        .await;
    assert_eq!(r, WorldResult::CharDeleteFailed);
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.send(CMSG_CHAR_DELETE { guid: g }).await;
    let r = a
        .wait_for("SMSG_CHAR_DELETE", |m| match m {
            ServerOpcodeMessage::SMSG_CHAR_DELETE(d) => Some(d.result),
            _ => None,
        })
        .await;
    assert_eq!(r, WorldResult::CharDeleteSuccess);
    assert!(realm.registry.name(g).is_none());
}

#[tokio::test]
async fn a_shutdown_saves_everyone() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(-8901.0, -101.0, 84.0).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    realm.shutdown().await;
    let row = realm.store.load(guid::counter(g)).await.unwrap();
    assert_eq!((row.x, row.y), (-8901.0, -101.0));
}

#[test]
fn every_public_player_field_a_client_draws_is_public() {
    // The fields another client needs to draw a player.
    let mut f = Fields::new(TypeId::Player, guid::player(1), 0);
    for i in [
        UNIT_FIELD_HEALTH,
        UNIT_FIELD_MAXHEALTH,
        UNIT_FIELD_LEVEL,
        UNIT_FIELD_DISPLAYID,
        UNIT_FIELD_BYTES_0,
        PLAYER_BYTES,
        PLAYER_BYTES_2,
        PLAYER_VISIBLE_ITEM_1_0,
    ] {
        f.set_u32(i, 1);
    }
    let (_, values) = f.select(Visibility::Public, false);
    // The object fields plus the eight.
    assert!(values.len() >= 8);
}

mod world {
    use super::*;
    use crate::game::data::*;

    /// Northshire, where a human starts.
    pub const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

    pub fn test_data() -> GameData {
        data()
    }

    fn data() -> GameData {
        let mut d = GameData::empty();
        d.creature_templates.insert(
            1000,
            CreatureTemplate {
                entry: 1000,
                name: "Test Wolf".into(),
                display_ids: [903, 0, 0, 0],
                display_probabilities: [1, 0, 0, 0],
                display_scales: [1.0, 0.0, 0.0, 0.0],
                level_min: 2,
                level_max: 3,
                faction: 32,
                unit_class: 1,
                health_max: 50,
                ..Default::default()
            },
        );
        let spawn = |guid, movement_type, dx: f32| CreatureSpawn {
            guid,
            entries: vec![1000],
            map: 0,
            x: START.0 + dx,
            y: START.1,
            z: START.2,
            wander_distance: if movement_type == 1 { 6.0 } else { 0.0 },
            movement_type,
            ..Default::default()
        };
        d.creature_spawns.insert(
            0,
            vec![spawn(1, 0, 5.0), spawn(2, 1, 10.0), spawn(3, 0, 400.0)],
        );
        d.gameobject_templates.insert(
            50,
            GameObjectTemplate {
                entry: 50,
                kind: 19,
                display_id: 3851,
                name: "Mailbox".into(),
                size: 1.0,
                ..Default::default()
            },
        );
        d.gameobject_spawns.insert(
            0,
            vec![GameObjectSpawn {
                guid: 7,
                entry: 50,
                map: 0,
                x: START.0 - 4.0,
                y: START.1,
                z: START.2,
                rotation: [0.0, 0.0, 0.0, 1.0],
                state: 1,
                anim_progress: 100,
                ..Default::default()
            }],
        );
        d.graveyards.push(Graveyard {
            id: 1,
            map: 0,
            x: START.0 + 50.0,
            y: START.1,
            z: START.2,
            orientation: 0.0,
            zone: 0,
            faction: 0,
        });
        d.creature_templates.insert(
            6491,
            CreatureTemplate {
                entry: 6491,
                name: "Spirit Healer".into(),
                display_ids: [5233, 0, 0, 0],
                display_probabilities: [1, 0, 0, 0],
                npc_flags: 0x21,
                faction: 35,
                level_min: 60,
                level_max: 60,
                health_max: 5000,
                ..Default::default()
            },
        );
        d.creature_spawns.get_mut(&0).unwrap().push(CreatureSpawn {
            guid: 4,
            entries: vec![6491],
            map: 0,
            x: START.0 + 52.0,
            y: START.1,
            z: START.2,
            ..Default::default()
        });
        d.area_triggers.insert(
            9,
            AreaTrigger {
                id: 9,
                map: 0,
                x: START.0,
                y: START.1,
                z: START.2,
                radius: 3.0,
                teleport: Some(Teleport {
                    map: 0,
                    x: -9000.0,
                    y: -200.0,
                    z: 90.0,
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        d
    }

    #[tokio::test]
    async fn a_player_sees_the_creatures_and_objects_near_and_not_far_ones() {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        a.enter("Alice", Race::Human, Class::Warrior).await;
        let near = guid::unit(1000, 1);
        let far = guid::unit(1000, 3);
        let mailbox = guid::game_object(50, 7);
        a.sees(near).await;
        a.sees(mailbox).await;
        assert!(!a.sees_within(far, NONE_WITHIN).await);
        a.send(CMSG_CREATURE_QUERY {
            creature: 1000,
            guid: near,
        })
        .await;
        let name = a
            .wait_for("SMSG_CREATURE_QUERY_RESPONSE", |m| match m {
                ServerOpcodeMessage::SMSG_CREATURE_QUERY_RESPONSE(r) => {
                    r.found.as_ref().map(|f| f.name1.clone())
                }
                _ => None,
            })
            .await;
        assert_eq!(name, "Test Wolf");
        a.send(CMSG_GAMEOBJECT_QUERY {
            entry_id: 50,
            guid: mailbox,
        })
        .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let parsed = a
            .gets(NONE_WITHIN, |m| {
                matches!(m, ServerOpcodeMessage::SMSG_GAMEOBJECT_QUERY_RESPONSE(_)).then_some(())
            })
            .await
            .is_some();
        let raw = a.raw.iter().any(|f| {
            u16::from_le_bytes([f[2], f[3]])
                == crate::game::map::gameobject::SMSG_GAMEOBJECT_QUERY_RESPONSE
        });
        assert!(parsed || raw, "a game object query response");
    }

    #[tokio::test]
    async fn a_wandering_creature_walks_where_its_watchers_see() {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        a.enter("Alice", Race::Human, Class::Warrior).await;
        let wolf = guid::unit(1000, 2);
        a.sees(wolf).await;
        // Due to walk now.
        let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
        map.call(move |m| {
            if let Some(c) = m.creature_mut(wolf) {
                c.motion = crate::game::map::creature::Motion::Idle {
                    until: Some(Instant::now()),
                };
            }
        });
        let to = a
            .wait_for("SMSG_MONSTER_MOVE", |m| match m {
                ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv) if mv.guid == wolf => {
                    mv.splines.first().copied()
                }
                _ => None,
            })
            .await;
        let home = (START.0 + 10.0, START.1);
        assert!(((to.x - home.0).powi(2) + (to.y - home.1).powi(2)).sqrt() <= 6.01);
    }

    #[tokio::test]
    async fn an_area_trigger_teleports() {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        a.enter("Alice", Race::Human, Class::Warrior).await;
        a.send(CMSG_AREATRIGGER { trigger_id: 9 }).await;
        let to = a
            .wait_for("MSG_MOVE_TELEPORT_ACK", |m| match m {
                ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => Some(t.info.position),
                _ => None,
            })
            .await;
        assert_eq!((to.x, to.y), (-9000.0, -200.0));
    }
}

mod death {
    use super::world::START;
    use super::*;
    use crate::game::map::MapKey;
    use crate::game::map::death::Death;

    fn player_fields(m: &ServerOpcodeMessage, of: Guid) -> Option<UpdatePlayer> {
        values(m).into_iter().find_map(|(g, mask)| match mask {
            UpdateMask::Player(p) if g == of => Some(p),
            _ => None,
        })
    }

    #[tokio::test]
    async fn a_dead_player_releases_runs_back_and_reclaims() {
        let realm = harness::realm_with(super::world::test_data());
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
        let mut b = Client::connect(&realm, "B");
        let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
        b.sees(ga).await;
        a.say(".die").await;
        a.wait_for("health 0", |m| {
            player_fields(m, ga)
                .and_then(|p| p.unit_health())
                .filter(|h| *h == 0)
        })
        .await;
        a.send(CMSG_REPOP_REQUEST {}).await;
        let to = a
            .wait_for("the teleport to the graveyard", |m| match m {
                ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => Some(t.info.position),
                _ => None,
            })
            .await;
        assert!((to.x - (-8899.95)).abs() < 0.1, "{to:?}");
        a.send(MSG_MOVE_TELEPORT_ACK_Client {
            guid: ga,
            movement_counter: 0,
            time: Default::default(),
        })
        .await;
        // Bob, alive, no longer sees her; he sees her corpse.
        b.forgets(ga).await;
        let corpse = guid::corpse(guid::counter(ga));
        b.sees(corpse).await;
        // A corpse query finds it.
        a.send(MSG_CORPSE_QUERY_Client {}).await;
        a.wait_for("the corpse", |m| match m {
            ServerOpcodeMessage::MSG_CORPSE_QUERY(q) => matches!(
                q.result,
                MSG_CORPSE_QUERY_Server_CorpseQueryResult::Found { .. }
            )
            .then_some(()),
            _ => None,
        })
        .await;
        // Back at the corpse, too soon: nothing.
        a.heartbeat(START.0 + 1.0, START.1, START.2).await;
        a.send(CMSG_RECLAIM_CORPSE { guid: corpse }).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let map = realm.maps.get(&realm, MapKey::world(0));
        let dead = map
            .query(move |m| m.players.get(&ga).map(|p| p.death != Death::Alive))
            .await
            .flatten();
        assert_eq!(dead, Some(true));
        // After the delay.
        map.call(move |m| {
            if let Some(p) = m.players.get_mut(&ga) {
                p.death = Death::Ghost {
                    since: Instant::now() - Duration::from_secs(60),
                };
            }
        });
        a.send(CMSG_RECLAIM_CORPSE { guid: corpse }).await;
        a.wait_for("half health", |m| {
            player_fields(m, ga)
                .and_then(|p| p.unit_health())
                .filter(|h| *h > 1)
        })
        .await;
        b.sees(ga).await;
        b.forgets(corpse).await;
        let _ = gb;
    }

    #[tokio::test]
    async fn a_spirit_healer_resurrects_a_ghost() {
        let realm = harness::realm_with(super::world::test_data());
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
        a.say(".die").await;
        a.send(CMSG_REPOP_REQUEST {}).await;
        a.wait_for("the teleport", |m| {
            matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
        })
        .await;
        a.send(MSG_MOVE_TELEPORT_ACK_Client {
            guid: ga,
            movement_counter: 0,
            time: Default::default(),
        })
        .await;
        let healer = guid::unit(6491, 4);
        a.sees(healer).await;
        a.send(CMSG_GOSSIP_HELLO { guid: healer }).await;
        a.wait_for("SMSG_SPIRIT_HEALER_CONFIRM", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_SPIRIT_HEALER_CONFIRM(_)).then_some(())
        })
        .await;
        a.send(CMSG_SPIRIT_HEALER_ACTIVATE { guid: healer }).await;
        a.wait_for("alive", |m| {
            player_fields(m, ga)
                .and_then(|p| p.unit_health())
                .filter(|h| *h > 1)
        })
        .await;
    }

    #[tokio::test]
    async fn levelling_up_raises_the_stats_and_says_so() {
        let realm = harness::realm();
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
        a.say(".levelup 9").await;
        let info = a
            .wait_for("SMSG_LEVELUP_INFO", |m| match m {
                ServerOpcodeMessage::SMSG_LEVELUP_INFO(l) => Some((l.new_level.as_int(), l.health)),
                _ => None,
            })
            .await;
        assert_eq!(info.0, 10);
        assert!(info.1 > 0);
        a.wait_for("level 10", |m| {
            player_fields(m, ga)
                .and_then(|p| p.unit_level())
                .filter(|l| *l == 10)
        })
        .await;
        assert_eq!(
            realm
                .registry
                .name(ga)
                .unwrap()
                .level
                .load(std::sync::atomic::Ordering::Relaxed),
            10
        );
    }

    #[tokio::test]
    async fn health_regenerates_out_of_combat() {
        let realm = harness::realm();
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
        let map = realm.maps.get(&realm, MapKey::world(0));
        map.call(move |m| {
            if let Some(p) = m.players.get_mut(&ga) {
                p.fields.set_u32(UNIT_FIELD_HEALTH, 10);
                p.regen_at = Instant::now();
            }
        });
        a.wait_for("more health", |m| {
            player_fields(m, ga)
                .and_then(|p| p.unit_health())
                .filter(|h| *h > 10)
        })
        .await;
    }
}

mod combat {
    use super::world::START;
    use super::*;
    use crate::game::data::*;
    use crate::game::map::MapKey;

    fn faction(id: u32, ours: u32, friend_mask: u32, enemy_mask: u32) -> (u32, FactionTemplate) {
        (
            id,
            FactionTemplate {
                id,
                faction: id,
                ours,
                friend_mask,
                enemy_mask,
                ..Default::default()
            },
        )
    }

    /// A wolf (hostile) and a boar (neutral) near the start, and the
    /// players' and monsters' factions.
    fn data() -> GameData {
        let mut d = GameData::empty();
        d.faction_templates.extend([
            faction(1, 3, 2, 12),
            faction(2, 5, 4, 10),
            faction(14, 8, 0, 1),
            faction(32, 0, 0, 0),
        ]);
        d.class_level_stats.insert(
            (1, 2),
            ClassLevelStats {
                health: 40,
                melee_damage: 3.0,
                armor: 20,
                ..Default::default()
            },
        );
        let template = |entry, name: &str, faction| CreatureTemplate {
            entry,
            name: name.into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 2,
            level_max: 2,
            faction,
            unit_class: 1,
            health_multiplier: 1.0,
            damage_multiplier: 1.0,
            base_attack_time: 2000,
            xp_multiplier: 1.0,
            ..Default::default()
        };
        d.creature_templates
            .insert(2000, template(2000, "Wolf", 14));
        d.creature_templates
            .insert(2001, template(2001, "Boar", 32));
        let spawn = |guid, entry, dx: f32| CreatureSpawn {
            guid,
            entries: vec![entry],
            map: 0,
            x: START.0 + dx,
            y: START.1,
            z: START.2,
            respawn_min: 1,
            respawn_max: 1,
            ..Default::default()
        };
        d.creature_spawns
            .insert(0, vec![spawn(10, 2000, 60.0), spawn(11, 2001, 3.0)]);
        d
    }

    fn swing_frames(c: &mut Client) -> usize {
        c.clear();
        c.raw
            .iter()
            .filter(|f| {
                u16::from_le_bytes([f[2], f[3]])
                    == crate::game::map::combat::SMSG_ATTACKERSTATEUPDATE
            })
            .count()
    }

    async fn swings_within(c: &mut Client, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            let parsed = c
                .gets(Duration::from_millis(5), |m| {
                    matches!(m, ServerOpcodeMessage::SMSG_ATTACKERSTATEUPDATE(_)).then_some(())
                })
                .await
                .is_some();
            if parsed || swing_frames(c) > 0 {
                return true;
            }
        }
        false
    }

    #[tokio::test]
    async fn the_first_swing_lands_at_once_and_the_creature_fights_back_and_dies() {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
        let boar = guid::unit(2001, 11);
        a.sees(boar).await;
        // Face the boar (east), next to it.
        a.heartbeat(START.0 + 1.0, START.1, START.2).await;
        let started = Instant::now();
        a.send(CMSG_ATTACKSWING { guid: boar }).await;
        a.wait_for("SMSG_ATTACKSTART", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_ATTACKSTART(_)).then_some(())
        })
        .await;
        assert!(swings_within(&mut a, Duration::from_millis(500)).await);
        assert!(started.elapsed() < Duration::from_millis(500));
        // Kill it quickly.
        let map = realm.maps.get(&realm, MapKey::world(0));
        map.call(move |m| {
            if let Some(c) = m.creature_mut(boar) {
                c.fields.set_u32(UNIT_FIELD_HEALTH, 1);
            }
            if let Some(p) = m.players.get_mut(&ga) {
                p.fields.set_f32(UNIT_FIELD_MINDAMAGE, 50.0);
                p.fields.set_f32(UNIT_FIELD_MAXDAMAGE, 50.0);
            }
        });
        let xp = a
            .wait_for("SMSG_LOG_XPGAIN", |m| match m {
                ServerOpcodeMessage::SMSG_LOG_XPGAIN(x) => Some(x.total_exp),
                _ => None,
            })
            .await;
        assert!(xp > 0);
        let dead = map
            .query(move |m| m.creature(boar).map(|c| !c.alive()))
            .await
            .flatten();
        assert_eq!(dead, Some(true));
        // It comes back.
        map.call(move |m| {
            if let Some(c) = m.creature_mut(boar)
                && let crate::game::map::creature::Life::Dead { corpse_until, .. } = c.life
            {
                c.life = crate::game::map::creature::Life::Dead {
                    corpse_until,
                    respawn_at: Instant::now(),
                };
            }
        });
        a.forgets(boar).await;
        a.sees(boar).await;
    }

    #[tokio::test]
    async fn a_hostile_creature_attacks_a_player_who_comes_near_and_leashes_home() {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
        let wolf = guid::unit(2000, 10);
        a.sees(wolf).await;
        // Walk up to it.
        a.heartbeat(START.0 + 44.0, START.1, START.2).await;
        let attacker = a
            .wait_for("SMSG_ATTACKSTART from the wolf", |m| match m {
                ServerOpcodeMessage::SMSG_ATTACKSTART(s) if s.victim == ga => Some(s.attacker),
                _ => None,
            })
            .await;
        assert_eq!(attacker, wolf);
        // It chases.
        a.wait_for("the wolf moving", |m| match m {
            ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv) if mv.guid == wolf => Some(()),
            _ => None,
        })
        .await;
        // Far past its leash: it gives up and goes home.
        a.heartbeat(START.0 + 200.0, START.1, START.2).await;
        let map = realm.maps.get(&realm, MapKey::world(0));
        let mut evaded = false;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if map
                .query(move |m| m.creature(wolf).map(|c| c.evading || !c.combat.attacking))
                .await
                .flatten()
                == Some(true)
            {
                evaded = true;
                break;
            }
        }
        assert!(evaded);
    }

    #[tokio::test]
    async fn swinging_out_of_range_says_so() {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        a.enter("Alice", Race::Human, Class::Warrior).await;
        let boar = guid::unit(2001, 11);
        a.sees(boar).await;
        a.heartbeat(START.0 - 20.0, START.1, START.2).await;
        a.send(CMSG_ATTACKSWING { guid: boar }).await;
        a.wait_for("SMSG_ATTACKSWING_NOTINRANGE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_ATTACKSWING_NOTINRANGE).then_some(())
        })
        .await;
    }

    #[tokio::test]
    async fn players_in_a_free_for_all_fight_and_one_dies() {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
        let mut b = Client::connect(&realm, "B");
        let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
        a.sees(gb).await;
        let map = realm.maps.get(&realm, MapKey::world(0));
        // Both in an arena, next to each other.
        map.call(move |m| {
            for g in [ga, gb] {
                if let Some(p) = m.players.get_mut(&g) {
                    p.fields.set_flag(
                        PLAYER_FLAGS,
                        crate::game::map::player::PLAYER_FLAGS_FFA_PVP,
                        true,
                    );
                    p.fields.set_f32(UNIT_FIELD_MINDAMAGE, 500.0);
                    p.fields.set_f32(UNIT_FIELD_MAXDAMAGE, 500.0);
                }
            }
        });
        a.heartbeat(START.0 - 1.0, START.1, START.2).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        // Swing until a blow lands.
        for _ in 0..40 {
            a.send(CMSG_ATTACKSWING { guid: gb }).await;
            tokio::time::sleep(Duration::from_millis(60)).await;
            let dead = map
                .query(move |m| {
                    m.players
                        .get(&gb)
                        .map(|p| p.death != crate::game::map::death::Death::Alive)
                })
                .await
                .flatten();
            if dead == Some(true) {
                return;
            }
            map.call(move |m| {
                if let Some(p) = m.players.get_mut(&ga) {
                    p.combat.next_swing = Some(Instant::now());
                }
            });
        }
        panic!("Bob never died");
    }
}
