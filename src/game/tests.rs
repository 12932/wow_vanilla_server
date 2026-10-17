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
    two_near_on(harness::realm()).await
}

/// Alice and Bob, side by side, on `realm`.
async fn two_near_on(
    realm: std::sync::Arc<crate::game::Realm>,
) -> (std::sync::Arc<crate::game::Realm>, Client, Client) {
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
    cfg.logout_delay = crate::game::map::login::LOGOUT_DELAY;
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
    // A non-GM's "." line is only chat.
    a.say(".gps").await;
    a.wait_for("the line as chat", |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => (c.message == ".gps").then_some(()),
        _ => None,
    })
    .await;
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
    pub fn data() -> GameData {
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
            xp_multiplier: Some(1.0),
            detection_range: 20.0,
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
        // Far past its leash: it gives up and goes home, once it has
        // chased more than 50 yards from where the fight began with no
        // blow for 12 seconds (Thistle Tea's leash).
        a.heartbeat(START.0 + 200.0, START.1, START.2).await;
        let map = realm.maps.get(&realm, MapKey::world(0));
        map.call(move |m| {
            if let Some(c) = m.creature_mut(wolf)
                && let Some(origin) = c.engagement.origin
            {
                c.position.x = origin.x + 60.0;
                c.engagement.last_hostile = Some(Instant::now() - Duration::from_secs(13));
            }
        });
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

mod items {
    use super::world::START;
    use super::*;
    use crate::game::data::*;
    use crate::game::map::MapKey;

    pub fn item(
        entry: u32,
        name: &str,
        class: u32,
        inventory_type: u32,
        price: u32,
    ) -> ItemTemplate {
        ItemTemplate {
            entry,
            name: name.into(),
            class,
            display_id: 1000 + entry,
            quality: 1,
            buy_count: 1,
            buy_price: price,
            sell_price: price / 4,
            inventory_type,
            allowable_class: -1,
            allowable_race: -1,
            stackable: if class == 0 { 20 } else { 1 },
            max_durability: if class == 2 || class == 4 { 25 } else { 0 },
            delay: if class == 2 { 2000 } else { 0 },
            damages: [
                (2.0, 5.0, 0),
                (0.0, 0.0, 0),
                (0.0, 0.0, 0),
                (0.0, 0.0, 0),
                (0.0, 0.0, 0),
            ],
            stats: {
                let mut s = [(0u32, 0i32); 10];
                if class == 4 {
                    s[0] = (7, 3);
                }
                s
            },
            ..Default::default()
        }
    }

    pub fn data() -> GameData {
        let mut d = GameData::empty();
        for t in [
            item(25, "Worn Shortsword", 2, 21, 100),
            item(117, "Tough Jerky", 0, 0, 25),
            item(2961, "Burnished Tunic", 4, 5, 300),
        ] {
            d.items.insert(t.entry, t);
        }
        d.start.insert(
            (1, 1),
            StartInfo {
                map: 0,
                zone: 12,
                x: START.0,
                y: START.1,
                z: START.2,
                orientation: 0.0,
                items: vec![(25, u8::MAX, 1), (117, u8::MAX, 4)],
                spells: vec![6603],
                actions: vec![(0, 6603, 0)],
            },
        );
        d.class_level_stats.insert(
            (1, 1),
            ClassLevelStats {
                health: 10,
                melee_damage: 1.0,
                ..Default::default()
            },
        );
        let npc = |entry, name: &str, npc_flags| CreatureTemplate {
            entry,
            name: name.into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 1,
            level_max: 1,
            faction: 35,
            npc_flags,
            unit_class: 1,
            health_multiplier: 1.0,
            loot_id: if npc_flags == 0 { entry } else { 0 },
            gold_min: 7,
            gold_max: 7,
            ..Default::default()
        };
        d.creature_templates
            .insert(3000, npc(3000, "Vendor", 0x4 | 0x4000));
        d.creature_templates.insert(3001, npc(3001, "Rabbit", 0));
        d.vendors.insert(
            3000,
            vec![
                VendorItem {
                    slot: 1,
                    item: 117,
                    ..Default::default()
                },
                VendorItem {
                    slot: 2,
                    item: 2961,
                    max_count: 1,
                    restock_secs: 600,
                    ..Default::default()
                },
            ],
        );
        d.loot.creature.insert(
            3001,
            vec![LootEntry {
                item: 117,
                chance: 100.0,
                min: 2,
                max: 2,
                ..Default::default()
            }],
        );
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
        d.creature_spawns
            .insert(0, vec![spawn(20, 3000, 2.0), spawn(21, 3001, 3.0)]);
        d
    }

    fn own_fields(m: &ServerOpcodeMessage, of: Guid) -> Option<UpdatePlayer> {
        values(m).into_iter().find_map(|(g, mask)| match mask {
            UpdateMask::Player(p) if g == of => Some(p),
            _ => None,
        })
    }

    #[tokio::test]
    async fn starting_gear_is_worn_and_the_rest_carried() {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
        let map = realm.maps.get(&realm, MapKey::world(0));
        let (main_hand, food) = map
            .query(move |m| {
                let p = m.players.get(&ga)?;
                Some((
                    p.item_at(crate::game::store::BAG_PLAYER, 15)
                        .map(|i| p.items[i].entry),
                    p.item_at(crate::game::store::BAG_PLAYER, 23)
                        .map(|i| p.items[i].count()),
                ))
            })
            .await
            .flatten()
            .unwrap();
        assert_eq!(main_hand, Some(25));
        assert_eq!(food, Some(4));
        // The item query is answered from the template.
        a.send(CMSG_ITEM_QUERY_SINGLE {
            item: 2961,
            guid: Guid::zero(),
        })
        .await;
        let name = a
            .wait_for("SMSG_ITEM_QUERY_SINGLE_RESPONSE", |m| match m {
                ServerOpcodeMessage::SMSG_ITEM_QUERY_SINGLE_RESPONSE(r) => {
                    r.found.as_ref().map(|f| f.name1.clone())
                }
                _ => None,
            })
            .await;
        assert_eq!(name, "Burnished Tunic");
    }

    #[tokio::test]
    async fn a_kill_can_be_looted_items_and_money() {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
        let rabbit = guid::unit(3001, 21);
        a.sees(rabbit).await;
        let map = realm.maps.get(&realm, MapKey::world(0));
        map.call(move |m| crate::game::map::combat::kill(m, ga, rabbit));
        tokio::time::sleep(Duration::from_millis(60)).await;
        a.send(CMSG_LOOT { guid: rabbit }).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        let parsed = a
            .gets(NONE_WITHIN, |m| {
                matches!(m, ServerOpcodeMessage::SMSG_LOOT_RESPONSE(_)).then_some(())
            })
            .await
            .is_some();
        let raw = a.raw.iter().any(|f| {
            u16::from_le_bytes([f[2], f[3]]) == crate::game::map::loot::SMSG_LOOT_RESPONSE
        });
        assert!(parsed || raw, "a loot window");
        a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
        a.wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == 117 => Some(()),
            _ => None,
        })
        .await;
        a.send(CMSG_LOOT_MONEY {}).await;
        a.wait_for("the money", |m| {
            own_fields(m, ga)
                .and_then(|p| p.player_field_coinage())
                .filter(|c| *c == 7)
        })
        .await;
        a.send(CMSG_LOOT_RELEASE { guid: rabbit }).await;
        a.wait_for("SMSG_LOOT_RELEASE_RESPONSE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_LOOT_RELEASE_RESPONSE(_)).then_some(())
        })
        .await;
        // Six jerky now: four carried, two looted onto the stack.
        let jerky = map
            .query(move |m| m.players.get(&ga).map(|p| p.count_of(117)))
            .await
            .flatten();
        assert_eq!(jerky, Some(6));
    }

    #[tokio::test]
    async fn a_vendor_lists_sells_buys_and_buys_back() {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
        let vendor = guid::unit(3000, 20);
        a.sees(vendor).await;
        let map = realm.maps.get(&realm, MapKey::world(0));
        map.call(move |m| {
            if let Some(p) = m.players.get_mut(&ga) {
                p.set_money(1000);
            }
        });
        a.send(CMSG_LIST_INVENTORY { guid: vendor }).await;
        let n = a
            .wait_for("SMSG_LIST_INVENTORY", |m| match m {
                ServerOpcodeMessage::SMSG_LIST_INVENTORY(l) => Some(l.items.len()),
                _ => None,
            })
            .await;
        assert_eq!(n, 2);
        a.send(CMSG_BUY_ITEM {
            vendor,
            item: 2961,
            amount: 1,
            unknown1: 0,
        })
        .await;
        let left = a
            .wait_for("SMSG_BUY_ITEM", |m| match m {
                ServerOpcodeMessage::SMSG_BUY_ITEM(b) => Some(b.amount_for_sale),
                _ => None,
            })
            .await;
        assert_eq!(left, 0);
        // Sold out.
        a.send(CMSG_BUY_ITEM {
            vendor,
            item: 2961,
            amount: 1,
            unknown1: 0,
        })
        .await;
        a.wait_for("SMSG_BUY_FAILED", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_BUY_FAILED(_)).then_some(())
        })
        .await;
        let (money, tunic) = map
            .query(move |m| {
                let p = m.players.get(&ga)?;
                let tunic = p.items.iter().find(|i| i.entry == 2961).map(|i| i.guid)?;
                Some((p.money(), tunic))
            })
            .await
            .flatten()
            .unwrap();
        assert_eq!(money, 700);
        // Wear it: others see it, and it adds stamina.
        let stamina_before = map
            .query(move |m| {
                m.players
                    .get(&ga)
                    .map(|p| p.fields.get_u32(UNIT_FIELD_STAT0 + 2))
            })
            .await
            .flatten()
            .unwrap();
        let at = map
            .query(move |m| {
                let p = m.players.get(&ga)?;
                p.item_by_guid(tunic)
                    .map(|i| (p.items[i].bag, p.items[i].slot))
            })
            .await
            .flatten()
            .unwrap();
        a.send(CMSG_AUTOEQUIP_ITEM {
            source_bag: at.0,
            source_slot: at.1,
        })
        .await;
        a.wait_for("the tunic worn", |m| {
            own_fields(m, ga)
                .and_then(|p| p.unit_stamina())
                .filter(|s| *s as u32 == stamina_before + 3)
        })
        .await;
        // Sell the jerky; buy it back.
        let jerky = map
            .query(move |m| {
                let p = m.players.get(&ga)?;
                p.items.iter().find(|i| i.entry == 117).map(|i| i.guid)
            })
            .await
            .flatten()
            .unwrap();
        a.send(CMSG_SELL_ITEM {
            vendor,
            item: jerky,
            amount: 0,
        })
        .await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        let after_sale = map
            .query(move |m| m.players.get(&ga).map(|p| (p.money(), p.count_of(117))))
            .await
            .flatten()
            .unwrap();
        assert_eq!(after_sale, (700 + 4 * 6, 0));
        a.send(CMSG_BUYBACK_ITEM {
            guid: vendor,
            slot: BuybackSlot::Slot1,
        })
        .await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        let after = map
            .query(move |m| m.players.get(&ga).map(|p| (p.money(), p.count_of(117))))
            .await
            .flatten()
            .unwrap();
        assert_eq!(after, (700, 4));
    }

    #[tokio::test]
    async fn items_survive_a_logout() {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
        let map = realm.maps.get(&realm, MapKey::world(0));
        map.call(move |m| {
            let _ = crate::game::map::inventory::add_item(m, ga, 2961, 1, true);
        });
        a.wait_for("SMSG_ITEM_PUSH_RESULT", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(_)).then_some(())
        })
        .await;
        a.send(CMSG_LOGOUT_REQUEST {}).await;
        a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
        })
        .await;
        a.login(ga).await;
        let has = map
            .query(move |m| m.players.get(&ga).map(|p| p.count_of(2961)))
            .await
            .flatten();
        assert_eq!(has, Some(1));
    }
}

mod npcs {
    use super::world::START;
    use super::*;
    use crate::game::data::npc::*;
    use crate::game::data::*;
    use crate::game::map::MapKey;

    fn creature(entry: u32, name: &str, npc_flags: u32) -> CreatureTemplate {
        CreatureTemplate {
            entry,
            name: name.into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 5,
            level_max: 5,
            faction: 35,
            npc_flags,
            unit_class: 1,
            health_multiplier: 1.0,
            ..Default::default()
        }
    }

    fn data() -> GameData {
        let mut d = GameData::empty();
        let mut add = |t: CreatureTemplate, guid, dx: f32| {
            let entry = t.entry;
            d.creature_templates.insert(entry, t);
            d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
                guid,
                entries: vec![entry],
                map: 0,
                x: START.0 + dx,
                y: START.1 + 1.0,
                z: START.2,
                respawn_min: 300,
                respawn_max: 300,
                ..Default::default()
            });
        };
        let mut gossip = creature(4000, "Talker", 0x1 | 0x4);
        gossip.gossip_menu_id = 77;
        add(gossip, 40, 1.0);
        let mut trainer = creature(4001, "Trainer", 0x10);
        trainer.trainer_class = 1;
        add(trainer, 41, 2.0);
        add(creature(4002, "Innkeeper", 0x80 | 0x100), 42, 3.0);
        add(creature(4003, "Gryphon Master", 0x8), 43, 4.0);
        add(creature(4004, "Quest Giver", 0x2), 44, 5.0);
        let mut target = creature(4005, "Kobold", 0);
        target.faction = 14;
        add(target, 45, 6.0);
        add(creature(4006, "Battlemaster", 0x800), 46, 0.5);
        d.items.insert(
            5000,
            ItemTemplate {
                entry: 5000,
                name: "Reward Sword".into(),
                class: 2,
                inventory_type: 13,
                allowable_class: -1,
                allowable_race: -1,
                stackable: 1,
                ..Default::default()
            },
        );
        d.npc.gossip_menus.insert(
            77,
            GossipMenu {
                texts: vec![(500, 0)],
                options: vec![GossipOption {
                    id: 0,
                    icon: 1,
                    text: "Let me browse your goods.".into(),
                    kind: 3,
                    npc_flag: 0x4,
                    ..Default::default()
                }],
            },
        );
        d.npc.npc_texts.insert(
            500,
            std::array::from_fn(|i| NpcTextChoice {
                broadcast: if i == 0 { 900 } else { 0 },
                probability: if i == 0 { 1.0 } else { 0.0 },
            }),
        );
        d.npc.broadcast_texts.insert(
            900,
            BroadcastText {
                male: "Hello, traveller.".into(),
                ..Default::default()
            },
        );
        d.npc.trainers.insert(
            4001,
            vec![crate::game::data::npc::TrainerSpell {
                spell: 100,
                cost: 10,
                required_level: 4,
                ..Default::default()
            }],
        );
        d.npc.taxi_nodes.insert(
            2,
            TaxiNode {
                id: 2,
                map: 0,
                x: START.0 + 4.0,
                y: START.1 + 1.0,
                z: START.2,
                name: "Here".into(),
                mounts: [0, 0],
            },
        );
        d.npc.taxi_nodes.insert(
            3,
            TaxiNode {
                id: 3,
                map: 0,
                x: START.0 + 64.0,
                y: START.1,
                z: START.2,
                name: "There".into(),
                mounts: [0, 0],
            },
        );
        d.npc.taxi_paths.insert((2, 3), (9, 5));
        d.npc.taxi_path_nodes.insert(
            9,
            vec![
                TaxiPathNode {
                    map: 0,
                    x: START.0 + 4.0,
                    y: START.1,
                    z: START.2 + 10.0,
                    ..Default::default()
                },
                TaxiPathNode {
                    map: 0,
                    x: START.0 + 64.0,
                    y: START.1,
                    z: START.2,
                    ..Default::default()
                },
            ],
        );
        d.npc.bank_slot_prices.insert(1, 1000);
        let quest = |id, kills: [(i32, u32); 4], next| Quest {
            id,
            level: 5,
            min_level: 1,
            title: format!("Quest {id}"),
            details: "Kill a kobold.".into(),
            objectives: "Kill it.".into(),
            required_kills: kills,
            rewards: [(5000, 1), (0, 0), (0, 0), (0, 0)],
            reward_xp: 250,
            reward_money: 30,
            next_in_chain: next,
            ..Default::default()
        };
        d.npc
            .quests
            .insert(600, quest(600, [(4005, 1), (0, 0), (0, 0), (0, 0)], 601));
        let mut second = quest(601, [(0, 0); 4], 0);
        second.prev_quest = 600;
        second.rewards = [(0, 0); 4];
        d.npc.quests.insert(601, second);
        d.npc.creature_starts.insert(4004, vec![600, 601]);
        d.npc.creature_ends.insert(4004, vec![600, 601]);
        d.battlemasters.insert(4006, 2);
        d
    }

    async fn setup() -> (std::sync::Arc<crate::game::Realm>, Client, Guid) {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        let g = a.enter("Alice", Race::Human, Class::Warrior).await;
        for (entry, counter) in [
            (4000, 40),
            (4001, 41),
            (4002, 42),
            (4003, 43),
            (4004, 44),
            (4005, 45),
            (4006, 46),
        ] {
            a.sees(guid::unit(entry, counter)).await;
        }
        let map = realm.maps.get(&realm, MapKey::world(0));
        map.call(move |m| {
            if let Some(p) = m.players.get_mut(&g) {
                p.set_money(10_000);
            }
        });
        (realm, a, g)
    }

    #[tokio::test]
    async fn gossip_shows_its_menu_and_text_and_an_option_opens_the_vendor() {
        let (_realm, mut a, _) = setup().await;
        let npc = guid::unit(4000, 40);
        a.send(CMSG_GOSSIP_HELLO { guid: npc }).await;
        let (text, options) = a
            .wait_for("SMSG_GOSSIP_MESSAGE", |m| match m {
                ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) => {
                    Some((g.title_text_id, g.gossips.len()))
                }
                _ => None,
            })
            .await;
        assert_eq!((text, options), (500, 1));
        a.send(CMSG_NPC_TEXT_QUERY {
            text_id: 500,
            guid: npc,
        })
        .await;
        let line = a
            .wait_for("SMSG_NPC_TEXT_UPDATE", |m| match m {
                ServerOpcodeMessage::SMSG_NPC_TEXT_UPDATE(t) => Some(t.texts[0].texts[0].clone()),
                _ => None,
            })
            .await;
        assert_eq!(line, "Hello, traveller.");
        a.send(CMSG_GOSSIP_SELECT_OPTION {
            guid: npc,
            gossip_list_id: 0,
            unknown: None,
        })
        .await;
        a.wait_for("SMSG_LIST_INVENTORY", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_LIST_INVENTORY(_)).then_some(())
        })
        .await;
    }

    #[tokio::test]
    async fn a_trainer_lists_and_teaches_when_the_level_is_there() {
        let (_realm, mut a, _) = setup().await;
        let npc = guid::unit(4001, 41);
        a.send(CMSG_TRAINER_LIST { guid: npc }).await;
        let state = a
            .wait_for("SMSG_TRAINER_LIST", |m| match m {
                ServerOpcodeMessage::SMSG_TRAINER_LIST(t) => t.spells.first().map(|s| s.state),
                _ => None,
            })
            .await;
        assert_eq!(state, TrainerSpellState::Red);
        a.say(".levelup 4").await;
        a.send(CMSG_TRAINER_BUY_SPELL { guid: npc, id: 100 }).await;
        a.wait_for("SMSG_TRAINER_BUY_SUCCEEDED", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_TRAINER_BUY_SUCCEEDED(_)).then_some(())
        })
        .await;
        a.wait_for("SMSG_LEARNED_SPELL", |m| match m {
            ServerOpcodeMessage::SMSG_LEARNED_SPELL(s) if s.id == 100 => Some(()),
            _ => None,
        })
        .await;
    }

    #[tokio::test]
    async fn the_innkeeper_binds_and_the_bank_opens_and_sells_a_slot() {
        let (_realm, mut a, _) = setup().await;
        let npc = guid::unit(4002, 42);
        a.send(CMSG_BINDER_ACTIVATE { guid: npc }).await;
        a.wait_for("SMSG_PLAYERBOUND", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_PLAYERBOUND(_)).then_some(())
        })
        .await;
        a.send(CMSG_BANKER_ACTIVATE { guid: npc }).await;
        a.wait_for("SMSG_SHOW_BANK", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_SHOW_BANK(_)).then_some(())
        })
        .await;
        a.send(CMSG_BUY_BANK_SLOT { guid: npc }).await;
        let result = a
            .wait_for("SMSG_BUY_BANK_SLOT_RESULT", |m| match m {
                ServerOpcodeMessage::SMSG_BUY_BANK_SLOT_RESULT(r) => Some(r.result),
                _ => None,
            })
            .await;
        assert_eq!(result, BuyBankSlotResult::Ok);
    }

    #[tokio::test]
    async fn a_flight_master_learns_its_node_and_flies_to_a_known_one() {
        let (realm, mut a, g) = setup().await;
        let npc = guid::unit(4003, 43);
        a.send(CMSG_TAXIQUERYAVAILABLENODES { guid: npc }).await;
        a.wait_for("SMSG_SHOWTAXINODES", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_SHOWTAXINODES(_)).then_some(())
        })
        .await;
        let map = realm.maps.get(&realm, MapKey::world(0));
        // Node 3 already known.
        map.call(move |m| {
            if let Some(p) = m.players.get_mut(&g) {
                p.row.state.taxi_mask[0] |= 1 << 3;
            }
        });
        a.send(CMSG_ACTIVATETAXI {
            guid: npc,
            source_node: 2,
            destination_node: 3,
        })
        .await;
        let reply = a
            .wait_for("SMSG_ACTIVATETAXIREPLY", |m| match m {
                ServerOpcodeMessage::SMSG_ACTIVATETAXIREPLY(r) => Some(r.reply),
                _ => None,
            })
            .await;
        assert_eq!(reply, ActivateTaxiReply::Ok);
        // 60 yards at 32 a second: landed within three.
        let landed = a
            .wait_for("the landing", |m| match m {
                ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => Some(t.info.position),
                _ => None,
            })
            .await;
        assert!((landed.x - (START.0 + 64.0)).abs() < 0.5);
    }

    #[tokio::test]
    async fn a_quest_is_taken_done_and_rewarded_and_the_next_offered() {
        let (realm, mut a, g) = setup().await;
        let giver = guid::unit(4004, 44);
        a.send(CMSG_QUESTGIVER_STATUS_QUERY { guid: giver }).await;
        let s = a
            .wait_for("SMSG_QUESTGIVER_STATUS", |m| match m {
                ServerOpcodeMessage::SMSG_QUESTGIVER_STATUS(s) => Some(s.status),
                _ => None,
            })
            .await;
        assert_eq!(s, QuestGiverStatus::Available);
        a.send(CMSG_QUESTGIVER_HELLO { guid: giver }).await;
        a.wait_for("the quest's details", |m| match m {
            ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_DETAILS(d) if d.quest_id == 600 => Some(()),
            _ => None,
        })
        .await;
        a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
            guid: giver,
            quest_id: 600,
        })
        .await;
        a.send(CMSG_QUEST_QUERY { quest_id: 600 }).await;
        // 1.12's layout, which upstream wow_messages reads differently.
        a.wait_frame("SMSG_QUEST_QUERY_RESPONSE", 0x5D, |_| true)
            .await;
        // Kill the kobold.
        let kobold = guid::unit(4005, 45);
        let map = realm.maps.get(&realm, MapKey::world(0));
        map.call(move |m| crate::game::map::combat::kill(m, g, kobold));
        a.wait_for("SMSG_QUESTUPDATE_ADD_KILL", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_QUESTUPDATE_ADD_KILL(_)).then_some(())
        })
        .await;
        a.wait_for("SMSG_QUESTUPDATE_COMPLETE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_QUESTUPDATE_COMPLETE(_)).then_some(())
        })
        .await;
        a.send(CMSG_QUESTGIVER_COMPLETE_QUEST {
            guid: giver,
            quest_id: 600,
        })
        .await;
        a.wait_for("SMSG_QUESTGIVER_OFFER_REWARD", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_QUESTGIVER_OFFER_REWARD(_)).then_some(())
        })
        .await;
        a.send(CMSG_QUESTGIVER_CHOOSE_REWARD {
            guid: giver,
            quest_id: 600,
            reward: 0,
        })
        .await;
        let xp = a
            .wait_for("SMSG_QUESTGIVER_QUEST_COMPLETE", |m| match m {
                ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_COMPLETE(c) => Some(c.experience_reward),
                _ => None,
            })
            .await;
        assert_eq!(xp, 250);
        a.wait_for("the reward sword", |m| match m {
            ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == 5000 => Some(()),
            _ => None,
        })
        .await;
        // The chain goes on.
        a.wait_for("the next quest", |m| match m {
            ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_DETAILS(d) if d.quest_id == 601 => Some(()),
            _ => None,
        })
        .await;
    }

    #[tokio::test]
    async fn a_battlemaster_and_an_auctioneer_answer() {
        let (_realm, mut a, _) = setup().await;
        a.send(CMSG_BATTLEMASTER_HELLO {
            guid: guid::unit(4006, 46),
        })
        .await;
        let map = a
            .wait_for("SMSG_BATTLEFIELD_LIST", |m| match m {
                ServerOpcodeMessage::SMSG_BATTLEFIELD_LIST(l) => Some(l.map),
                _ => None,
            })
            .await;
        assert_eq!(map, Map::WarsongGulch);
    }
}

mod spells {
    use super::world::START;
    use super::*;
    use crate::game::data::spells::{CreatureSpell, SpellDuration, SpellEffect, SpellTemplate};
    use crate::game::data::*;
    use crate::game::map::spells::{Targets, wire_targets};
    use crate::game::map::{MapHandle, MapKey};

    fn effect(effect: u32, base_points: i32, target_a: u32) -> SpellEffect {
        SpellEffect {
            effect,
            base_points,
            base_dice: 1,
            die_sides: 1,
            target_a,
            ..Default::default()
        }
    }

    /// The fight data, with a few spells: a bolt that takes 200 ms, an
    /// instant heal, a burn that ticks, a strike with a cooldown, a passive
    /// toughness, and a spitter that casts at whoever comes near.
    fn data() -> GameData {
        let mut d = super::combat::data();
        let s = &mut d.spells;
        s.cast_times.insert(2, 200);
        s.ranges.insert(1, (0.0, 30.0));
        s.durations.insert(
            3,
            SpellDuration {
                base: 350,
                ..Default::default()
            },
        );
        let spell = |id, name: &str, effects: [SpellEffect; 3]| SpellTemplate {
            id,
            name: name.into(),
            range_index: 1,
            effects,
            ..Default::default()
        };
        let none = SpellEffect::default();
        let mut bolt = spell(100, "Bolt", [effect(2, 9, 6), none, none]);
        bolt.casting_time_index = 2;
        bolt.mana_cost = 5;
        bolt.school = 2;
        bolt.interrupt_flags = 0x1;
        let heal = spell(101, "Mend", [effect(10, 19, 21), none, none]);
        let mut burn_effect = effect(6, 3, 6);
        burn_effect.aura = 3;
        burn_effect.amplitude = 100;
        let mut burn = spell(102, "Burn", [burn_effect, none, none]);
        burn.duration_index = 3;
        let mut strike = spell(103, "Strike", [effect(2, 4, 6), none, none]);
        strike.recovery_time = 10_000;
        let mut tough_effect = effect(6, 4, 1);
        tough_effect.aura = 29;
        tough_effect.misc_value = 2;
        let mut tough = spell(104, "Toughness", [tough_effect, none, none]);
        tough.attributes[0] = crate::game::data::spells::ATTR_PASSIVE;
        let mut spit = spell(105, "Spit", [effect(2, 2, 6), none, none]);
        spit.casting_time_index = 2;
        for t in [bolt, heal, burn, strike, tough, spit] {
            s.spells.insert(t.id, t);
        }
        s.talents.insert(
            1,
            crate::game::data::spells::TalentInfo {
                id: 1,
                tab: 1,
                ranks: [104, 0, 0, 0, 0],
                ..Default::default()
            },
        );
        s.creature_spells.insert(
            7,
            vec![CreatureSpell {
                spell: 105,
                probability: 100,
                target: 1,
                delay_repeat: (5000, 5000),
                ..Default::default()
            }],
        );
        let mut spitter = d.creature_templates[&2000].clone();
        spitter.entry = 2002;
        spitter.name = "Spitter".into();
        spitter.spell_list_id = 7;
        d.creature_templates.insert(2002, spitter);
        d.creature_spawns.get_mut(&0).unwrap().push(CreatureSpawn {
            guid: 12,
            entries: vec![2002],
            map: 0,
            x: START.0 - 60.0,
            y: START.1,
            z: START.2,
            respawn_min: 1,
            respawn_max: 1,
            ..Default::default()
        });
        d
    }

    const BOAR: (u32, u32) = (2001, 11);

    /// A mage with mana and the spells, next to the boar.
    async fn setup() -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Mage).await;
        a.sees(guid::unit(BOAR.0, BOAR.1)).await;
        let map = realm.maps.get(&realm, MapKey::world(0));
        map.query(move |m| {
            if let Some(p) = m.players.get_mut(&ga) {
                p.fields.set_u32(UNIT_FIELD_MAXPOWER1, 100);
                p.fields.set_u32(UNIT_FIELD_POWER1, 100);
                p.spells.extend([100, 101, 102, 103]);
            }
        })
        .await;
        (realm, a, ga, map)
    }

    fn at(target: Guid) -> SpellCastTargets {
        wire_targets(&Targets {
            unit: Some(target),
            ..Default::default()
        })
    }

    #[tokio::test]
    async fn a_bolt_takes_its_cast_time_costs_mana_and_hurts() {
        let (_realm, mut a, ga, map) = setup().await;
        let boar = guid::unit(BOAR.0, BOAR.1);
        let started = Instant::now();
        a.send(CMSG_CAST_SPELL {
            spell: 100,
            targets: at(boar),
        })
        .await;
        let timer = a
            .wait_for("SMSG_SPELL_START", |m| match m {
                ServerOpcodeMessage::SMSG_SPELL_START(s) if s.spell == 100 => Some(s.timer),
                _ => None,
            })
            .await;
        assert_eq!(timer, 200);
        a.wait_for("SMSG_SPELL_GO", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == 100 => Some(()),
            _ => None,
        })
        .await;
        assert!(started.elapsed() >= Duration::from_millis(190));
        let damage = a
            .wait_for("the damage", |m| match m {
                ServerOpcodeMessage::SMSG_SPELLNONMELEEDAMAGELOG(d) if d.target == boar => {
                    Some(d.damage)
                }
                _ => None,
            })
            .await;
        assert_eq!(damage, 10);
        let (health, mana, max) = map
            .query(move |m| {
                let c = m.creature(boar).unwrap();
                let p = &m.players[&ga];
                (
                    c.fields.get_u32(UNIT_FIELD_HEALTH),
                    p.fields.get_u32(UNIT_FIELD_POWER1),
                    c.fields.get_u32(UNIT_FIELD_MAXHEALTH),
                )
            })
            .await
            .unwrap();
        assert_eq!(health, max - 10);
        assert_eq!(mana, 95);
        // The boar fights back.
        a.wait_for("SMSG_ATTACKSTART from the boar", |m| match m {
            ServerOpcodeMessage::SMSG_ATTACKSTART(s) if s.attacker == boar => Some(()),
            _ => None,
        })
        .await;
    }

    #[tokio::test]
    async fn a_heal_lands_and_a_burn_ticks_and_wears_off() {
        let (_realm, mut a, ga, map) = setup().await;
        let boar = guid::unit(BOAR.0, BOAR.1);
        map.query(move |m| {
            m.players
                .get_mut(&ga)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_HEALTH, 5)
        })
        .await;
        a.send(CMSG_CAST_SPELL {
            spell: 101,
            targets: wire_targets(&Targets::default()),
        })
        .await;
        let healed = a
            .wait_for("SMSG_SPELLHEALLOG", |m| match m {
                ServerOpcodeMessage::SMSG_SPELLHEALLOG(h) if h.victim == ga => Some(h.damage),
                _ => None,
            })
            .await;
        assert_eq!(healed, 20);
        a.send(CMSG_CAST_SPELL {
            spell: 102,
            targets: at(boar),
        })
        .await;
        for _ in 0..2 {
            a.wait_for("a tick of the burn", |m| match m {
                ServerOpcodeMessage::SMSG_PERIODICAURALOG(l) if l.target == boar => Some(()),
                _ => None,
            })
            .await;
        }
        let on = map
            .query(move |m| {
                m.creature(boar)
                    .map(|c| c.fields.get_u32(UNIT_FIELD_AURA + 32))
            })
            .await
            .flatten();
        assert_eq!(on, Some(102));
        let mut gone = false;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let state = map
                .query(move |m| {
                    m.creature(boar).map(|c| {
                        (
                            c.auras.holders.is_empty(),
                            c.fields.get_u32(UNIT_FIELD_AURA + 32),
                        )
                    })
                })
                .await
                .flatten();
            if state == Some((true, 0)) {
                gone = true;
                break;
            }
        }
        assert!(gone, "the burn wears off");
    }

    #[tokio::test]
    async fn a_cooldown_holds_and_moving_breaks_a_cast() {
        let (_realm, mut a, _ga, _map) = setup().await;
        let boar = guid::unit(BOAR.0, BOAR.1);
        for _ in 0..2 {
            a.send(CMSG_CAST_SPELL {
                spell: 103,
                targets: at(boar),
            })
            .await;
        }
        // wow_messages reads a failure's status but not its reason.
        a.wait_for("the second strike failing", |m| match m {
            ServerOpcodeMessage::SMSG_CAST_RESULT(r)
                if r.spell == 103
                    && r.result == SMSG_CAST_RESULT_SimpleSpellCastResult::Failure =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
        a.send(CMSG_CAST_SPELL {
            spell: 100,
            targets: at(boar),
        })
        .await;
        a.wait_for("SMSG_SPELL_START", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_START(s) if s.spell == 100 => Some(()),
            _ => None,
        })
        .await;
        a.heartbeat(START.0 - 2.0, START.1, START.2).await;
        a.wait_for("SMSG_SPELL_FAILURE", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_FAILURE(f) if f.spell == 100 => Some(()),
            _ => None,
        })
        .await;
    }

    #[tokio::test]
    async fn a_passive_spell_holds_once_learned() {
        let (_realm, _a, ga, map) = setup().await;
        let (before, after, held) = map
            .query(move |m| {
                let before = m.players[&ga].fields.get_u32(UNIT_FIELD_STAT0 + 2);
                crate::game::map::spells::learn(m, ga, 104);
                let p = &m.players[&ga];
                (
                    before,
                    p.fields.get_u32(UNIT_FIELD_STAT0 + 2),
                    p.auras.holders.iter().any(|a| a.spell == 104),
                )
            })
            .await
            .unwrap();
        assert!(held);
        assert_eq!(after, before + 5);
    }

    #[tokio::test]
    async fn a_talent_takes_a_point_from_level_ten() {
        let (_realm, _a, ga, map) = setup().await;
        let learned = map
            .query(move |m| {
                // Level 1: no point yet.
                crate::game::map::spells::learn_talent(m, ga, 1, 0);
                let before = m.players[&ga].spells.contains(&104);
                let data = m.realm.data.clone();
                let p = m.players.get_mut(&ga).unwrap();
                crate::game::map::stats::set_level(p, &data, 10);
                let points = p.fields.get_u32(PLAYER_CHARACTER_POINTS1);
                crate::game::map::spells::learn_talent(m, ga, 1, 0);
                let p = &m.players[&ga];
                (
                    before,
                    points,
                    p.spells.contains(&104),
                    p.fields.get_u32(PLAYER_CHARACTER_POINTS1),
                )
            })
            .await
            .unwrap();
        assert_eq!(learned, (false, 1, true, 0));
    }

    #[tokio::test]
    async fn a_creature_casts_from_its_list() {
        let (_realm, mut a, ga, _map) = setup().await;
        let spitter = guid::unit(2002, 12);
        a.sees(spitter).await;
        a.heartbeat(START.0 - 50.0, START.1, START.2).await;
        let target = a
            .wait_for("the spitter's SMSG_SPELL_START", |m| match m {
                ServerOpcodeMessage::SMSG_SPELL_START(s)
                    if s.caster == spitter && s.spell == 105 =>
                {
                    s.targets.target_flags.get_unit().map(|u| u.unit_target)
                }
                _ => None,
            })
            .await;
        assert_eq!(target, ga);
        a.wait_for("the spit landing", |m| match m {
            ServerOpcodeMessage::SMSG_SPELLNONMELEEDAMAGELOG(d) if d.attacker == spitter => {
                Some(())
            }
            _ => None,
        })
        .await;
    }
}

/// Against the real data, when it is here (it isn't in CI).
mod real {
    use super::*;
    use crate::game::map::creature::Creature;
    use crate::game::map::spells::{self, Targets};
    use crate::game::map::{Map, MapKey, auras};

    /// Effects a fight shows: damage, heals, auras, power.
    const FIGHTING: &[u32] = &[2, 6, 9, 10, 17, 30, 31, 35, 58, 80, 121];

    /// What a cast can have changed: the kobold's health and auras, and the
    /// caster's health, auras and combo points.
    fn snapshot(m: &Map, g: Guid, kobold: Guid) -> (u32, usize, u32, usize, u8) {
        let c = m.creature(kobold);
        let p = &m.players[&g];
        (
            c.map_or(0, |c| c.fields.get_u32(UNIT_FIELD_HEALTH)),
            c.map_or(0, |c| c.auras.holders.len()),
            p.fields.get_u32(UNIT_FIELD_HEALTH),
            p.auras.holders.len(),
            p.caster.combo_points,
        )
    }

    /// Each fighting spell the player starts with, cast at a fresh kobold
    /// (or, if helpful, at the player): whether it took effect. Grovel (a
    /// stun on oneself) counts as harmful and hurts nobody, so it is left out.
    fn try_starting_spells(m: &mut Map, g: Guid) -> Vec<(u32, String, bool, bool)> {
        let data = m.realm.data.clone();
        let template = data.creature_template(6).cloned().expect("Kobold Vermin");
        let p = &m.players[&g];
        let (pos, o) = (p.position(), p.movement.orientation);
        // Shoot wants a wand, which no one starts with.
        let ranged = p
            .items
            .iter()
            .any(|i| i.bag == crate::game::store::BAG_PLAYER && i.slot == 17);
        let castable: Vec<u32> = p
            .spells
            .iter()
            .copied()
            .filter(|s| {
                data.spells.get(*s).is_some_and(|t| {
                    !t.passive()
                        && t.effects.iter().any(|e| FIGHTING.contains(&e.effect))
                        && *s != 7267
                        && (!t.auto_repeat() || ranged)
                })
            })
            .collect();
        let mut out = Vec::new();
        for spell in castable {
            // What the last spell left (Grovel kneels, and holds) goes, and
            // so does its fight.
            auras::clear_on_death(m, g);
            crate::game::map::combat::leave_combat(m, g);
            let s = data.spells.get(spell).unwrap();
            let counter = m.next_creature_counter;
            m.next_creature_counter += 1;
            // Near, or just past a ranged spell's minimum.
            let d = (data.spells.range(s).0 + 2.0).max(3.0);
            let at = Vector3d {
                x: pos.x + d * o.cos(),
                y: pos.y + d * o.sin(),
                z: pos.z,
            };
            let kobold = m.add_creature(Creature::new(
                guid::unit(6, counter | 0x80_0000),
                &template,
                1,
                at,
                o + std::f32::consts::PI,
            ));
            let p = m.players.get_mut(&g).unwrap();
            p.caster = Default::default();
            // A finishing move wants a combo point on its target.
            if s.finisher() {
                p.caster.combo_target = kobold;
                p.caster.combo_points = 1;
            }
            for i in 0..5 {
                p.fields.set_u32(UNIT_FIELD_MAXPOWER1 + i, 1000);
                p.fields.set_u32(UNIT_FIELD_POWER1 + i, 1000);
            }
            let max = p.fields.get_u32(UNIT_FIELD_MAXHEALTH);
            p.fields.set_u32(UNIT_FIELD_HEALTH, max / 2);
            let helpful = auras::positive(s);
            let before = snapshot(m, g, kobold);
            let targets = Targets {
                unit: (!helpful).then_some(kobold),
                ..Default::default()
            };
            let mods = spells::mods::for_spell(m, g, s);
            let refused = spells::checks::check(m, g, s, &targets, None, false, &mods);
            spells::cast(m, g, spell, targets, None);
            let now = m.now;
            if let Some(c) = m
                .players
                .get_mut(&g)
                .and_then(|p| p.caster.casting.as_mut())
            {
                c.ends = now;
            }
            spells::update(m);
            // A next-swing spell goes off at the swing, an auto-repeat one
            // at its first shot.
            if s.on_next_swing() {
                crate::game::map::combat::start_attack(m, g, kobold);
                crate::game::map::combat::stop_attack(m, g);
            }
            if s.auto_repeat() {
                if let Some(p) = m.players.get_mut(&g) {
                    p.derived.ranged_hit = 100.0;
                    if let Some(a) = p.combat.auto_repeat.as_mut() {
                        a.next_at = now;
                    }
                }
                crate::game::map::ranged::update(m);
                crate::game::map::ranged::cancel(m, g, false);
            }
            let after = snapshot(m, g, kobold);
            // Harmful: did it hurt the kobold. Helpful: did it do anything.
            let took = if helpful {
                before != after
            } else {
                after.0 < before.0 || after.1 > before.1
            };
            let name = match refused {
                Some(r) => format!("{} (refused {r:#x})", s.name),
                None => s.name.clone(),
            };
            out.push((spell, name, !helpful, took));
            m.remove_creature(kobold);
        }
        out
    }

    #[tokio::test]
    async fn every_class_fights_with_its_starting_spells() {
        let dir = std::path::Path::new("/local/home/wallenx/runs/tt-data/db");
        if !dir.join("vmangos.sqlite").exists() {
            return;
        }
        let realm = harness::realm_with(crate::game::data::load::load(dir));
        let classes = [
            ("Wara", Race::Human, Class::Warrior),
            ("Pala", Race::Human, Class::Paladin),
            ("Hunta", Race::Orc, Class::Hunter),
            ("Rogua", Race::Human, Class::Rogue),
            ("Priesta", Race::Human, Class::Priest),
            ("Shama", Race::Orc, Class::Shaman),
            ("Maga", Race::Human, Class::Mage),
            ("Locka", Race::Human, Class::Warlock),
            ("Druida", Race::NightElf, Class::Druid),
        ];
        let mut report = Vec::new();
        let mut failed = Vec::new();
        for (i, (name, race, class)) in classes.into_iter().enumerate() {
            let mut c = Client::connect(&realm, &format!("CLASS{i}"));
            let g = c.enter(name, race, class).await;
            let map_id = if matches!(race, Race::Human) { 0 } else { 1 };
            let map = realm.maps.get(&realm, MapKey::world(map_id));
            let tried = map.query(move |m| try_starting_spells(m, g)).await.unwrap();
            let hurts = tried.iter().any(|t| t.2 && t.3);
            for (spell, spell_name, harmful, took) in &tried {
                report.push(format!(
                    "{class:?} {spell} {spell_name}: harmful {harmful}, took effect {took}"
                ));
                if !took {
                    failed.push(format!("{class:?} {spell} {spell_name}"));
                }
            }
            if !hurts && !matches!(class, Class::Paladin) {
                failed.push(format!("{class:?}: nothing that hurts"));
            }
        }
        println!("{}", report.join("\n"));
        assert!(failed.is_empty(), "{}", failed.join("\n"));
    }
}

mod social {
    use super::*;
    use crate::game::social::channels;
    use crate::game::social::groups;

    fn chat(text: &str, chat_type: CMSG_MESSAGECHAT_ChatType) -> CMSG_MESSAGECHAT {
        CMSG_MESSAGECHAT {
            chat_type,
            language: Language::Common,
            message: text.to_owned(),
        }
    }

    fn u64_at(b: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
    }

    /// The bytes after a notify's kind and channel name.
    fn notify_rest<'a>(b: &'a [u8], kind: u8, channel: &str) -> Option<&'a [u8]> {
        let name = channel.as_bytes();
        (b.first() == Some(&kind)
            && b.get(1..1 + name.len()) == Some(name)
            && b.get(1 + name.len()) == Some(&0))
        .then(|| &b[2 + name.len()..])
    }

    #[tokio::test]
    async fn a_whisper_reaches_its_player_with_an_echo_and_an_away_message() {
        let (_realm, mut a, mut b) = two_near().await;
        let (ga, gb) = (a.guid, b.guid);
        b.send(chat("gone fishing", CMSG_MESSAGECHAT_ChatType::Afk))
            .await;
        a.send(chat(
            "psst",
            CMSG_MESSAGECHAT_ChatType::Whisper {
                target_player: "bob".into(),
            },
        ))
        .await;
        b.wait_for("the whisper", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => match c.chat_type {
                SMSG_MESSAGECHAT_ChatType::Whisper { sender2 } if sender2 == ga => {
                    (c.message == "psst").then_some(())
                }
                _ => None,
            },
            _ => None,
        })
        .await;
        a.wait_for("the echo", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => match c.chat_type {
                SMSG_MESSAGECHAT_ChatType::WhisperInform { sender2 } if sender2 == gb => Some(()),
                _ => None,
            },
            _ => None,
        })
        .await;
        let away = a
            .wait_for("Bob's AFK message", |m| match m {
                ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => match c.chat_type {
                    SMSG_MESSAGECHAT_ChatType::Afk { sender2 } if sender2 == gb => {
                        Some(c.message.clone())
                    }
                    _ => None,
                },
                _ => None,
            })
            .await;
        assert_eq!(away, "gone fishing");
        a.send(chat(
            "hello?",
            CMSG_MESSAGECHAT_ChatType::Whisper {
                target_player: "Nobody".into(),
            },
        ))
        .await;
        a.wait_for("SMSG_CHAT_PLAYER_NOT_FOUND", |m| match m {
            ServerOpcodeMessage::SMSG_CHAT_PLAYER_NOT_FOUND(n) => {
                (n.name == "Nobody").then_some(())
            }
            _ => None,
        })
        .await;
    }

    #[tokio::test]
    async fn a_channel_is_joined_spoken_in_listed_and_moderated() {
        // Players, not GMs, who may do anything.
        let mut cfg = harness::test_config();
        cfg.everyone_gm = false;
        let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
        let realm = crate::game::Realm::new(cfg, store, startup, Default::default(), None);
        let (_realm, mut a, mut b) = two_near_on(realm).await;
        let (ga, gb) = (a.guid, b.guid);
        let join = || CMSG_JOIN_CHANNEL {
            channel_name: "peon".into(),
            channel_password: String::new(),
        };
        a.send(join()).await;
        a.wait_frame("YOU_JOINED", channels::SMSG_CHANNEL_NOTIFY, |f| {
            notify_rest(f, channels::YOU_JOINED, "peon").is_some()
        })
        .await;
        b.send(join()).await;
        // Alice, the first in, owns it, and hears Bob come.
        let joined = a
            .wait_frame("Bob joining", channels::SMSG_CHANNEL_NOTIFY, |f| {
                notify_rest(f, channels::JOINED, "peon").is_some()
            })
            .await;
        assert_eq!(u64_at(&joined, 6), gb.guid());
        b.send(chat(
            "hi all",
            CMSG_MESSAGECHAT_ChatType::Channel {
                channel: "peon".into(),
            },
        ))
        .await;
        for c in [&mut a, &mut b] {
            c.wait_for("the channel line", |m| match m {
                ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => match &c.chat_type {
                    SMSG_MESSAGECHAT_ChatType::Channel {
                        channel_name,
                        player,
                        ..
                    } if channel_name == "peon" && *player == gb => Some(()),
                    _ => None,
                },
                _ => None,
            })
            .await;
        }
        a.send(CMSG_CHANNEL_LIST {
            channel_name: "peon".into(),
        })
        .await;
        let members = a
            .wait_for("SMSG_CHANNEL_LIST", |m| match m {
                ServerOpcodeMessage::SMSG_CHANNEL_LIST(l) => Some(l.members.clone()),
                _ => None,
            })
            .await;
        assert_eq!(members.len(), 2);
        // Bob can't kick; Alice can.
        b.send(CMSG_CHANNEL_KICK {
            channel_name: "peon".into(),
            player_name: "Alice".into(),
        })
        .await;
        b.wait_frame("NOT_MODERATOR", channels::SMSG_CHANNEL_NOTIFY, |f| {
            notify_rest(f, channels::NOT_MODERATOR, "peon").is_some()
        })
        .await;
        a.send(CMSG_CHANNEL_KICK {
            channel_name: "peon".into(),
            player_name: "Bob".into(),
        })
        .await;
        let kicked = b
            .wait_frame("being kicked", channels::SMSG_CHANNEL_NOTIFY, |f| {
                notify_rest(f, channels::PLAYER_KICKED, "peon").is_some()
            })
            .await;
        assert_eq!(
            (u64_at(&kicked, 6), u64_at(&kicked, 14)),
            (gb.guid(), ga.guid())
        );
        b.send(chat(
            "still here?",
            CMSG_MESSAGECHAT_ChatType::Channel {
                channel: "peon".into(),
            },
        ))
        .await;
        b.wait_frame("NOT_MEMBER", channels::SMSG_CHANNEL_NOTIFY, |f| {
            notify_rest(f, channels::NOT_MEMBER, "peon").is_some()
        })
        .await;
    }

    #[tokio::test]
    async fn friends_hear_of_logins_and_logouts_and_the_ignored_are_not_heard() {
        let (realm, mut a, mut b) = two_near().await;
        let gb = b.guid;
        a.send(CMSG_ADD_FRIEND { name: "Bob".into() }).await;
        let added = a
            .wait_frame(
                "ADDED_ONLINE",
                crate::game::social::friends::SMSG_FRIEND_STATUS,
                |f| f[0] == 0x06,
            )
            .await;
        assert_eq!(u64_at(&added, 1), gb.guid());
        b.send(CMSG_LOGOUT_REQUEST {}).await;
        a.wait_frame(
            "Bob going offline",
            crate::game::social::friends::SMSG_FRIEND_STATUS,
            |f| f[0] == 0x03 && u64_at(f, 1) == gb.guid(),
        )
        .await;
        b.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
        })
        .await;
        b.login(gb).await;
        a.wait_frame(
            "Bob coming back",
            crate::game::social::friends::SMSG_FRIEND_STATUS,
            |f| f[0] == 0x02 && u64_at(f, 1) == gb.guid(),
        )
        .await;
        // Bob ignores Alice: her whispers don't reach him.
        b.send(CMSG_ADD_IGNORE {
            name: "Alice".into(),
        })
        .await;
        b.wait_frame(
            "IGNORE_ADDED",
            crate::game::social::friends::SMSG_FRIEND_STATUS,
            |f| f[0] == 0x0F,
        )
        .await;
        a.send(chat(
            "hey",
            CMSG_MESSAGECHAT_ChatType::Whisper {
                target_player: "Bob".into(),
            },
        ))
        .await;
        a.wait_for("SMSG_MESSAGECHAT ignored", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => {
                matches!(c.chat_type, SMSG_MESSAGECHAT_ChatType::Ignored { .. }).then_some(())
            }
            _ => None,
        })
        .await;
        // The lists are kept with the character.
        let ga = a.guid;
        let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
        let kept = map
            .query(move |m| m.players.get(&ga).map(|p| p.row.state.friends.clone()))
            .await
            .flatten();
        assert_eq!(kept, Some(vec![guid::counter(gb)]));
    }

    #[tokio::test]
    async fn who_lists_those_online_and_inspect_is_heard() {
        let (_realm, mut a, mut b) = two_near().await;
        let ga = a.guid;
        a.send(CMSG_WHO {
            minimum_level: Level::new(1),
            maximum_level: Level::new(60),
            player_name: String::new(),
            guild_name: String::new(),
            race_mask: u32::MAX,
            class_mask: u32::MAX,
            zones: Vec::new(),
            search_strings: Vec::new(),
        })
        .await;
        let who = a
            .wait_frame("SMSG_WHO", crate::game::map::social::SMSG_WHO, |_| true)
            .await;
        assert_eq!(&who[..8], &[2, 0, 0, 0, 2, 0, 0, 0]);
        a.send(CMSG_WHO {
            minimum_level: Level::new(1),
            maximum_level: Level::new(60),
            player_name: "bo".into(),
            guild_name: String::new(),
            race_mask: u32::MAX,
            class_mask: u32::MAX,
            zones: Vec::new(),
            search_strings: Vec::new(),
        })
        .await;
        let who = a
            .wait_frame(
                "SMSG_WHO for bo",
                crate::game::map::social::SMSG_WHO,
                |_| true,
            )
            .await;
        assert_eq!(&who[..12], b"\x01\0\0\0\x01\0\0\0Bob\0");
        a.send(CMSG_INSPECT { guid: b.guid }).await;
        let by = b
            .wait_for("SMSG_INSPECT", |m| match m {
                ServerOpcodeMessage::SMSG_INSPECT(i) => Some(i.guid),
                _ => None,
            })
            .await;
        assert_eq!(by, ga);
    }

    #[tokio::test]
    async fn a_party_forms_talks_rolls_and_disbands() {
        let (_realm, mut a, mut b) = two_near().await;
        let (ga, gb) = (a.guid, b.guid);
        a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
        let from = b
            .wait_for("SMSG_GROUP_INVITE", |m| match m {
                ServerOpcodeMessage::SMSG_GROUP_INVITE(i) => Some(i.name.clone()),
                _ => None,
            })
            .await;
        assert_eq!(from, "Alice");
        b.send(CMSG_GROUP_ACCEPT {}).await;
        for (c, other) in [(&mut a, gb), (&mut b, ga)] {
            let (members, leader) = c
                .wait_for("SMSG_GROUP_LIST", |m| match m {
                    ServerOpcodeMessage::SMSG_GROUP_LIST(l) if !l.members.is_empty() => Some((
                        l.members.iter().map(|m| m.guid).collect::<Vec<_>>(),
                        l.leader,
                    )),
                    _ => None,
                })
                .await;
            assert_eq!((members, leader), (vec![other], ga));
        }
        b.send(chat("inc", CMSG_MESSAGECHAT_ChatType::Party)).await;
        a.wait_for("party chat", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => match c.chat_type {
                SMSG_MESSAGECHAT_ChatType::Party { chat_credit, .. } if chat_credit == gb => {
                    Some(())
                }
                _ => None,
            },
            _ => None,
        })
        .await;
        a.send(MSG_RANDOM_ROLL_Client {
            minimum: 1,
            maximum: 100,
        })
        .await;
        let roll = b
            .wait_for("MSG_RANDOM_ROLL", |m| match m {
                ServerOpcodeMessage::MSG_RANDOM_ROLL(r) if r.guid == ga => Some(r.actual_roll),
                _ => None,
            })
            .await;
        assert!((1..=100).contains(&roll));
        a.send(CMSG_GROUP_SET_LEADER { guid: gb }).await;
        a.wait_for("SMSG_GROUP_SET_LEADER", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_SET_LEADER(l) => (l.name == "Bob").then_some(()),
            _ => None,
        })
        .await;
        // Two make a party: one leaving ends it.
        a.send(CMSG_GROUP_DISBAND {}).await;
        for c in [&mut a, &mut b] {
            c.wait_for("SMSG_GROUP_DESTROYED", |m| {
                matches!(m, ServerOpcodeMessage::SMSG_GROUP_DESTROYED).then_some(())
            })
            .await;
        }
    }

    #[tokio::test]
    async fn members_apart_get_each_others_stats() {
        let (_realm, mut a, mut b) = two_near().await;
        let gb = b.guid;
        a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
        b.wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
        b.send(CMSG_GROUP_ACCEPT {}).await;
        // Bob walks out of Alice's sight.
        let (x, y, z) = super::world::START;
        b.heartbeat(x + 300.0, y, z).await;
        a.forgets(gb).await;
        let stats = a
            .wait_frame_of(
                "Bob's stats",
                &[
                    groups::SMSG_PARTY_MEMBER_STATS,
                    groups::SMSG_PARTY_MEMBER_STATS_FULL,
                ],
                |_| true,
            )
            .await;
        // Bob's packed GUID.
        assert_eq!(&stats[..2], &[1, guid::counter(gb) as u8]);
        a.send(CMSG_REQUEST_PARTY_MEMBER_STATS { guid: gb }).await;
        let full = a
            .wait_frame(
                "Bob's full stats",
                crate::game::social::groups::SMSG_PARTY_MEMBER_STATS_FULL,
                |_| true,
            )
            .await;
        // A packed GUID, then every player field.
        let mask_at = 1 + (full[0].count_ones() as usize);
        assert_eq!(
            u32::from_le_bytes(full[mask_at..mask_at + 4].try_into().unwrap()),
            crate::game::social::groups::STAT_PLAYER
        );
    }

    #[tokio::test]
    async fn a_raid_has_icons_and_a_ready_check() {
        let (_realm, mut a, mut b) = two_near().await;
        let (ga, gb) = (a.guid, b.guid);
        a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
        b.wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
        b.send(CMSG_GROUP_ACCEPT {}).await;
        a.send(CMSG_GROUP_RAID_CONVERT {}).await;
        b.wait_for("a raid roster", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => {
                (l.group_type == GroupType::Raid).then_some(())
            }
            _ => None,
        })
        .await;
        a.send(MSG_RAID_TARGET_UPDATE_Client {
            target_index: MSG_RAID_TARGET_UPDATE_Client_RaidTargetIndex::Unknown7 { target: gb },
        })
        .await;
        let icon = b
            .wait_frame(
                "the skull",
                crate::game::social::groups::MSG_RAID_TARGET_UPDATE,
                |f| f[0] == 0,
            )
            .await;
        assert_eq!((icon[1], u64_at(&icon, 2)), (7, gb.guid()));
        a.send(MSG_RAID_READY_CHECK_Client { answer: None }).await;
        b.wait_for("the ready check", |m| match m {
            ServerOpcodeMessage::MSG_RAID_READY_CHECK(r) => r.state_check.is_none().then_some(()),
            _ => None,
        })
        .await;
        b.send(MSG_RAID_READY_CHECK_Client {
            answer: Some(MSG_RAID_READY_CHECK_Client_answer { state: 1 }),
        })
        .await;
        let answer = a
            .wait_for("Bob's answer", |m| match m {
                ServerOpcodeMessage::MSG_RAID_READY_CHECK(r) => {
                    r.state_check.as_ref().map(|s| (s.guid, s.state))
                }
                _ => None,
            })
            .await;
        assert_eq!(answer, (gb, 1));
        let _ = ga;
    }
}

mod group_kills {
    use super::*;
    use crate::game::map::MapKey;

    #[tokio::test]
    async fn a_group_near_the_kill_shares_its_xp() {
        let realm = harness::realm_with(super::combat::data());
        let (_realm, mut a, mut b) = two_near_on(realm.clone()).await;
        let (ga, gb) = (a.guid, b.guid);
        a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
        b.wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
        b.send(CMSG_GROUP_ACCEPT {}).await;
        a.wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
            _ => None,
        })
        .await;
        let boar = guid::unit(2001, 11);
        let map = realm.maps.get(&realm, MapKey::world(0));
        map.query(move |m| {
            if let Some(c) = m.creature_mut(boar) {
                c.fields.set_u32(UNIT_FIELD_HEALTH, 1);
            }
            crate::game::map::combat::deal_damage(m, ga, boar, 5);
        })
        .await;
        let mut xp = Vec::new();
        for c in [&mut a, &mut b] {
            xp.push(
                c.wait_for("SMSG_LOG_XPGAIN", |m| match m {
                    ServerOpcodeMessage::SMSG_LOG_XPGAIN(x) if x.target == boar => {
                        Some(x.total_exp)
                    }
                    _ => None,
                })
                .await,
            );
        }
        // Two of a level: half each.
        let alone = crate::game::map::combat::kill_xp(1, 2, false, 1.0);
        assert_eq!(xp, vec![alone / 2, alone / 2]);
        let _ = gb;
    }
}

mod commerce {
    use super::world::START;
    use super::*;
    use crate::game::data::spells::{SpellEffect, SpellTemplate};
    use crate::game::data::*;
    use crate::game::map::MapKey;

    /// The items' data, with a charter, an auctioneer next to the start
    /// and the Duel spell.
    fn data() -> GameData {
        let mut d = super::items::data();
        let charter = super::items::item(5863, "Guild Charter", 12, 0, 0);
        d.items.insert(5863, charter);
        let mut auctioneer = d.creature_templates[&3000].clone();
        auctioneer.entry = 3002;
        auctioneer.name = "Auctioneer".into();
        auctioneer.npc_flags = crate::game::map::npc::NPC_FLAG_AUCTIONEER;
        d.creature_templates.insert(3002, auctioneer);
        d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
            guid: 40,
            entries: vec![3002],
            map: 0,
            x: START.0 + 2.0,
            y: START.1,
            z: START.2,
            respawn_min: 1,
            respawn_max: 1,
            ..Default::default()
        });
        d.spells.spells.insert(
            7266,
            SpellTemplate {
                id: 7266,
                name: "Duel".into(),
                effects: [
                    SpellEffect {
                        effect: 83,
                        target_a: 25,
                        ..Default::default()
                    },
                    SpellEffect::default(),
                    SpellEffect::default(),
                ],
                ..Default::default()
            },
        );
        d
    }

    async fn two_warriors(realm: std::sync::Arc<crate::game::Realm>) -> (Client, Client) {
        let mut a = Client::connect(&realm, "A");
        a.enter("Alice", Race::Human, Class::Warrior).await;
        let mut b = Client::connect(&realm, "B");
        b.enter("Bob", Race::Human, Class::Warrior).await;
        a.sees(b.guid).await;
        b.sees(a.guid).await;
        (a, b)
    }

    fn map0(realm: &std::sync::Arc<crate::game::Realm>) -> crate::game::map::MapHandle {
        realm.maps.get(realm, MapKey::world(0))
    }

    async fn item_of(
        realm: &std::sync::Arc<crate::game::Realm>,
        g: Guid,
        entry: u32,
    ) -> Option<Guid> {
        map0(realm)
            .query(move |m| {
                m.players
                    .get(&g)
                    .and_then(|p| p.items.iter().find(|i| i.entry == entry).map(|i| i.guid))
            })
            .await
            .flatten()
    }

    async fn holds(realm: &std::sync::Arc<crate::game::Realm>, g: Guid, item: Guid) -> bool {
        map0(realm)
            .query(move |m| {
                m.players
                    .get(&g)
                    .is_some_and(|p| p.item_by_guid(item).is_some())
            })
            .await
            .unwrap_or(false)
    }

    async fn money_of(realm: &std::sync::Arc<crate::game::Realm>, g: Guid) -> u32 {
        map0(realm)
            .query(move |m| m.players.get(&g).map_or(0, |p| p.money()))
            .await
            .unwrap_or(0)
    }

    async fn set_money(realm: &std::sync::Arc<crate::game::Realm>, g: Guid, money: u32) {
        map0(realm)
            .query(move |m| {
                if let Some(p) = m.players.get_mut(&g) {
                    p.set_money(money);
                }
            })
            .await;
    }

    #[tokio::test]
    async fn a_guild_is_made_joined_ranked_spoken_in_and_left() {
        let realm = harness::realm_with(data());
        let (mut a, mut b) = two_warriors(realm.clone()).await;
        let (ga, gb) = (a.guid, b.guid);
        a.send(CMSG_GUILD_CREATE {
            guild_name: "Peons".into(),
        })
        .await;
        a.send(CMSG_GUILD_INVITE {
            invited_player: "Bob".into(),
        })
        .await;
        let (from, guild) = b
            .wait_for("SMSG_GUILD_INVITE", |m| match m {
                ServerOpcodeMessage::SMSG_GUILD_INVITE(i) => {
                    Some((i.player_name.clone(), i.guild_name.clone()))
                }
                _ => None,
            })
            .await;
        assert_eq!((from.as_str(), guild.as_str()), ("Alice", "Peons"));
        b.send(CMSG_GUILD_ACCEPT {}).await;
        a.wait_for("Bob joining", |m| match m {
            ServerOpcodeMessage::SMSG_GUILD_EVENT(e) if e.event == GuildEvent::Joined => Some(()),
            _ => None,
        })
        .await;
        a.send(CMSG_GUILD_ROSTER {}).await;
        let members = a
            .wait_for("SMSG_GUILD_ROSTER", |m| match m {
                ServerOpcodeMessage::SMSG_GUILD_ROSTER(r) => Some(r.members.len()),
                _ => None,
            })
            .await;
        assert_eq!(members, 2);
        a.send(CMSG_GUILD_PROMOTE {
            player_name: "Bob".into(),
        })
        .await;
        b.wait_for("the promotion", |m| match m {
            ServerOpcodeMessage::SMSG_GUILD_EVENT(e) if e.event == GuildEvent::Promotion => {
                Some(())
            }
            _ => None,
        })
        .await;
        b.send(CMSG_MESSAGECHAT {
            chat_type: CMSG_MESSAGECHAT_ChatType::Guild,
            language: Language::Common,
            message: "for the guild".into(),
        })
        .await;
        a.wait_for("guild chat", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => match c.chat_type {
                SMSG_MESSAGECHAT_ChatType::Guild { sender2 } if sender2 == gb => Some(()),
                _ => None,
            },
            _ => None,
        })
        .await;
        let id = map0(&realm)
            .query(move |m| m.players[&gb].fields.get_u32(PLAYER_GUILDID))
            .await
            .unwrap();
        assert!(id > 0);
        a.send(CMSG_GUILD_QUERY { guild_id: id }).await;
        let name = a
            .wait_for("SMSG_GUILD_QUERY_RESPONSE", |m| match m {
                ServerOpcodeMessage::SMSG_GUILD_QUERY_RESPONSE(q) => Some(q.name.clone()),
                _ => None,
            })
            .await;
        assert_eq!(name, "Peons");
        b.send(CMSG_GUILD_LEAVE {}).await;
        a.wait_for("Bob leaving", |m| match m {
            ServerOpcodeMessage::SMSG_GUILD_EVENT(e) if e.event == GuildEvent::Left => Some(()),
            _ => None,
        })
        .await;
        let left = map0(&realm)
            .query(move |m| m.players[&gb].fields.get_u32(PLAYER_GUILDID))
            .await
            .unwrap();
        assert_eq!(left, 0);
        let _ = ga;
    }

    #[tokio::test]
    async fn a_charter_signed_enough_founds_a_guild() {
        let realm = harness::realm_with(data());
        let (mut a, mut b) = two_warriors(realm.clone()).await;
        let (ga, gb) = (a.guid, b.guid);
        set_money(&realm, ga, 5000).await;
        a.send(CMSG_PETITION_BUY {
            npc: Guid::zero(),
            name: "Grunts".into(),
            index: 1,
            ..Default::default()
        })
        .await;
        let mut charter = None;
        for _ in 0..100 {
            charter = item_of(&realm, ga, 5863).await;
            if charter.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let charter = charter.expect("the charter");
        assert_eq!(money_of(&realm, ga).await, 4000);
        a.send(CMSG_OFFER_PETITION {
            petition: charter,
            target: gb,
        })
        .await;
        b.wait_for("the charter shown", |m| match m {
            ServerOpcodeMessage::SMSG_PETITION_SHOW_SIGNATURES(s) => {
                (s.item == charter).then_some(())
            }
            _ => None,
        })
        .await;
        b.send(CMSG_PETITION_SIGN {
            petition: charter,
            unknown1: 0,
        })
        .await;
        a.wait_for("Bob's signature", |m| match m {
            ServerOpcodeMessage::SMSG_PETITION_SIGN_RESULTS(r) => {
                (r.result == PetitionResult::Ok && r.owner == gb).then_some(())
            }
            _ => None,
        })
        .await;
        a.send(CMSG_TURN_IN_PETITION { petition: charter }).await;
        a.wait_for("the guild founded", |m| match m {
            ServerOpcodeMessage::SMSG_TURN_IN_PETITION_RESULTS(r) => {
                (r.result == PetitionResult::Ok).then_some(())
            }
            _ => None,
        })
        .await;
        let (ia, ib, has) = map0(&realm)
            .query(move |m| {
                (
                    m.players[&ga].fields.get_u32(PLAYER_GUILDID),
                    m.players[&gb].fields.get_u32(PLAYER_GUILDID),
                    m.players[&ga].items.iter().any(|i| i.entry == 5863),
                )
            })
            .await
            .unwrap();
        assert!(ia > 0 && ia == ib, "{ia} {ib}");
        assert!(!has, "the charter is taken");
    }

    #[tokio::test]
    async fn mail_carries_money_an_item_and_cash_on_delivery() {
        let realm = harness::realm_with(data());
        let (mut a, mut b) = two_warriors(realm.clone()).await;
        let (ga, gb) = (a.guid, b.guid);
        set_money(&realm, ga, 1000).await;
        set_money(&realm, gb, 1000).await;
        let sword = item_of(&realm, ga, 25).await.expect("Alice's sword");
        // Alice wears the sword; the jerky she carries can go.
        let jerky = item_of(&realm, ga, 117).await.expect("Alice's jerky");
        let _ = sword;
        a.send(CMSG_SEND_MAIL {
            mailbox: Guid::zero(),
            receiver: "Bob".into(),
            subject: "lunch".into(),
            body: "enjoy".into(),
            unknown1: 0,
            unknown2: 0,
            item: jerky,
            money: Gold::new(100),
            cash_on_delivery_amount: 50,
            unknown3: 0,
            unknown4: 0,
        })
        .await;
        a.wait_for("SMSG_SEND_MAIL_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_SEND_MAIL_RESULT(r) => matches!(
                r.action,
                SMSG_SEND_MAIL_RESULT_MailAction::Send {
                    result2: SMSG_SEND_MAIL_RESULT_MailResultTwo::Ok
                }
            )
            .then_some(()),
            _ => None,
        })
        .await;
        // 100 sent, 60 postage.
        assert_eq!(money_of(&realm, ga).await, 840);
        b.wait_for("SMSG_RECEIVED_MAIL", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_RECEIVED_MAIL(_)).then_some(())
        })
        .await;
        b.send(CMSG_GET_MAIL_LIST {
            mailbox: Guid::zero(),
        })
        .await;
        let (id, item, cod, body) = b
            .wait_for("SMSG_MAIL_LIST_RESULT", |m| match m {
                ServerOpcodeMessage::SMSG_MAIL_LIST_RESULT(l) => l.mails.first().map(|m| {
                    (
                        m.message_id,
                        m.item,
                        m.cash_on_delivery_amount,
                        m.item_text_id,
                    )
                }),
                _ => None,
            })
            .await;
        assert_eq!((item, cod), (117, 50));
        b.send(CMSG_ITEM_TEXT_QUERY {
            item_text_id: body,
            mail_id: id,
            unknown1: 0,
        })
        .await;
        let text = b
            .wait_for("the mail's text", |m| match m {
                ServerOpcodeMessage::SMSG_ITEM_TEXT_QUERY_RESPONSE(t) => Some(t.text.clone()),
                _ => None,
            })
            .await;
        assert_eq!(text, "enjoy");
        b.send(CMSG_MAIL_TAKE_MONEY {
            mailbox: Guid::zero(),
            mail_id: id,
        })
        .await;
        b.send(CMSG_MAIL_TAKE_ITEM {
            mailbox: Guid::zero(),
            mail_id: id,
        })
        .await;
        b.wait_for("the item taken", |m| match m {
            ServerOpcodeMessage::SMSG_SEND_MAIL_RESULT(r) => matches!(
                r.action,
                SMSG_SEND_MAIL_RESULT_MailAction::ItemTaken {
                    result: SMSG_SEND_MAIL_RESULT_MailResult::Ok { .. }
                }
            )
            .then_some(()),
            _ => None,
        })
        .await;
        assert!(holds(&realm, gb, jerky).await);
        assert!(!holds(&realm, ga, jerky).await);
        // Bob paid 50 and took 100.
        assert_eq!(money_of(&realm, gb).await, 1050);
        // Alice gets the 50.
        a.wait_for("the payment's mail", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_RECEIVED_MAIL(_)).then_some(())
        })
        .await;
        let paid = realm
            .social
            .mail
            .lock()
            .mails
            .values()
            .find(|m| m.receiver == guid::counter(ga))
            .map(|m| (m.money, m.flags));
        assert_eq!(paid, Some((50, crate::game::social::mail::COD_PAYMENT)));
    }

    #[tokio::test]
    async fn an_auction_is_bought_out_and_settled_by_mail() {
        let realm = harness::realm_with(data());
        let (mut a, mut b) = two_warriors(realm.clone()).await;
        let (ga, gb) = (a.guid, b.guid);
        let auctioneer = guid::unit(3002, 40);
        a.sees(auctioneer).await;
        set_money(&realm, ga, 1000).await;
        set_money(&realm, gb, 1000).await;
        let jerky = item_of(&realm, ga, 117).await.unwrap();
        a.send(CMSG_AUCTION_SELL_ITEM {
            auctioneer,
            item: jerky,
            starting_bid: 100,
            buyout: 200,
            auction_duration_in_minutes: 120,
        })
        .await;
        let id = a
            .wait_for("the auction started", |m| match m {
                ServerOpcodeMessage::SMSG_AUCTION_COMMAND_RESULT(r) => Some(r.auction_id),
                _ => None,
            })
            .await;
        assert!(id > 0);
        b.send(CMSG_AUCTION_LIST_ITEMS {
            auctioneer,
            list_start_item: 0,
            searched_name: "jerky".into(),
            minimum_level: 0,
            maximum_level: 0,
            auction_slot_id: u32::MAX,
            auction_main_category: u32::MAX,
            auction_sub_category: u32::MAX,
            auction_quality: ItemQuality::Poor,
            usable: 0,
        })
        .await;
        let found = b
            .wait_for("SMSG_AUCTION_LIST_RESULT", |m| match m {
                ServerOpcodeMessage::SMSG_AUCTION_LIST_RESULT(l) => Some(
                    l.auctions
                        .iter()
                        .map(|a| (a.id, a.buyout_amount))
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .await;
        assert_eq!(found, vec![(id, 200)]);
        b.send(CMSG_AUCTION_PLACE_BID {
            auctioneer,
            auction_id: id,
            price: Gold::new(200),
        })
        .await;
        b.wait_for("the buyout", |m| match m {
            ServerOpcodeMessage::SMSG_AUCTION_COMMAND_RESULT(r) if r.auction_id == id => Some(()),
            _ => None,
        })
        .await;
        assert_eq!(money_of(&realm, gb).await, 800);
        b.wait_for("the item's mail", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_RECEIVED_MAIL(_)).then_some(())
        })
        .await;
        let (won, sold) = {
            let mails = realm.social.mail.lock();
            let of = |g: Guid| {
                mails
                    .mails
                    .values()
                    .find(|m| m.receiver == guid::counter(g))
                    .cloned()
            };
            (of(gb), of(ga))
        };
        assert_eq!(
            won.and_then(|m| m.item).map(|i| i.guid),
            Some(guid::counter(jerky))
        );
        // The goblins: 15% of 200 cut, the deposit back.
        let sold = sold.expect("the seller's mail");
        let deposit = crate::game::social::auction::deposit(25 / 4, 4, 7200, 7);
        assert_eq!(sold.money, 200 - 30 + deposit);
    }

    #[tokio::test]
    async fn a_trade_swaps_items_and_money_when_both_accept() {
        let realm = harness::realm_with(data());
        let (mut a, mut b) = two_warriors(realm.clone()).await;
        let (ga, gb) = (a.guid, b.guid);
        set_money(&realm, ga, 1000).await;
        let jerky = item_of(&realm, ga, 117).await.unwrap();
        let (bag, slot) = map0(&realm)
            .query(move |m| {
                let p = &m.players[&ga];
                let i = p.item_by_guid(jerky).unwrap();
                (p.items[i].bag, p.items[i].slot)
            })
            .await
            .unwrap();
        a.send(CMSG_INITIATE_TRADE { guid: gb }).await;
        b.wait_for("the trade asked", |m| match m {
            ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
                matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::BeginTrade { .. }).then_some(())
            }
            _ => None,
        })
        .await;
        b.send(CMSG_BEGIN_TRADE {}).await;
        a.send(CMSG_SET_TRADE_ITEM {
            trade_slot: 0,
            bag,
            slot,
        })
        .await;
        a.send(CMSG_SET_TRADE_GOLD {
            gold: Gold::new(300),
        })
        .await;
        b.wait_for("Alice's offer", |m| match m {
            ServerOpcodeMessage::SMSG_TRADE_STATUS_EXTENDED(t) => {
                (t.money_in_trade.as_int() == 300 && t.trade_slots[0].item == 117).then_some(())
            }
            _ => None,
        })
        .await;
        a.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
        b.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
        for c in [&mut a, &mut b] {
            c.wait_for("TRADE_COMPLETE", |m| match m {
                ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
                    matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::TradeComplete).then_some(())
                }
                _ => None,
            })
            .await;
        }
        assert!(holds(&realm, gb, jerky).await);
        assert!(!holds(&realm, ga, jerky).await);
        assert_eq!(money_of(&realm, ga).await, 700);
        assert_eq!(money_of(&realm, gb).await, 300);
    }

    #[tokio::test]
    async fn a_duel_ends_when_one_would_fall() {
        let realm = harness::realm_with(data());
        let (mut a, mut b) = two_warriors(realm.clone()).await;
        let (ga, gb) = (a.guid, b.guid);
        map0(&realm)
            .query(move |m| m.players.get_mut(&ga).unwrap().spells.push(7266))
            .await;
        a.send(CMSG_CAST_SPELL {
            spell: 7266,
            targets: crate::game::map::spells::wire_targets(&crate::game::map::spells::Targets {
                unit: Some(gb),
                ..Default::default()
            }),
        })
        .await;
        let flag = b
            .wait_for("SMSG_DUEL_REQUESTED", |m| match m {
                ServerOpcodeMessage::SMSG_DUEL_REQUESTED(r) if r.target == ga => Some(r.initiator),
                _ => None,
            })
            .await;
        b.send(CMSG_DUEL_ACCEPTED { guid: flag }).await;
        a.wait_for("SMSG_DUEL_COUNTDOWN", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_DUEL_COUNTDOWN(_)).then_some(())
        })
        .await;
        // Skip the countdown.
        map0(&realm)
            .query(move |m| {
                for g in [ga, gb] {
                    if let Some(d) = m.players.get_mut(&g).and_then(|p| p.duel.as_mut()) {
                        d.phase = crate::game::map::duel::Phase::Countdown {
                            until: std::time::Instant::now(),
                        };
                    }
                }
            })
            .await;
        let mut fighting = false;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            if map0(&realm)
                .query(move |m| crate::game::map::duel::dueling(m, ga, gb))
                .await
                == Some(true)
            {
                fighting = true;
                break;
            }
        }
        assert!(fighting);
        let health = map0(&realm)
            .query(move |m| {
                crate::game::map::combat::deal_damage(m, ga, gb, 100_000);
                m.players[&gb].fields.get_u32(UNIT_FIELD_HEALTH)
            })
            .await
            .unwrap();
        assert_eq!(health, 1);
        let (winner, loser) = a
            .wait_for("SMSG_DUEL_WINNER", |m| match m {
                ServerOpcodeMessage::SMSG_DUEL_WINNER(w) => {
                    Some((w.initiator_name.clone(), w.opponent_name.clone()))
                }
                _ => None,
            })
            .await;
        assert_eq!((winner.as_str(), loser.as_str()), ("Alice", "Bob"));
        b.wait_for("SMSG_DUEL_COMPLETE", |m| match m {
            ServerOpcodeMessage::SMSG_DUEL_COMPLETE(c) => {
                c.ended_without_interruption.then_some(())
            }
            _ => None,
        })
        .await;
        a.forgets(flag).await;
    }

    #[tokio::test]
    async fn expired_mail_goes_back_and_an_unsold_auction_comes_home() {
        use crate::game::social::{auction, mail};
        let realm = harness::realm_with(data());
        let (a, b) = two_warriors(realm.clone()).await;
        let (ga, gb) = (guid::counter(a.guid), guid::counter(b.guid));
        let past = mail::now() - 1;
        let item = crate::game::store::ItemRow {
            guid: 9_000,
            entry: 117,
            count: 1,
            ..Default::default()
        };
        let sent = mail::post(
            &realm,
            mail::Mail {
                kind: mail::NORMAL,
                sender: ga,
                receiver: gb,
                item: Some(item.clone()),
                deliver_at: past,
                expires_at: past,
                ..Default::default()
            },
        );
        realm.social.auctions.lock().auctions.insert(
            77,
            auction::Auction {
                id: 77,
                market: 7,
                house: 7,
                item: crate::game::store::ItemRow {
                    guid: 9_001,
                    ..item.clone()
                },
                owner: gb,
                start_bid: 10,
                expires_at: past,
                ..Default::default()
            },
        );
        crate::game::social::sweep(&realm);
        let mails = realm.social.mail.lock();
        let back = &mails.mails[&sent];
        assert_eq!((back.receiver, back.flags), (ga, mail::RETURNED));
        let home = mails
            .mails
            .values()
            .find(|m| m.auction == 77)
            .expect("the auction's mail");
        assert_eq!(
            (home.receiver, home.item.as_ref().map(|i| i.guid)),
            (gb, Some(9_001))
        );
        assert!(realm.social.auctions.lock().auctions.is_empty());
    }

    #[tokio::test]
    async fn guilds_mail_and_auctions_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("wvs-restart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("characters.sqlite");
        let (store, startup) = crate::game::store::Store::open(&db).unwrap();
        let realm = crate::game::Realm::new(harness::test_config(), store, startup, data(), None);
        let (mut a, b) = two_warriors(realm.clone()).await;
        let (ga, gb) = (a.guid, b.guid);
        a.send(CMSG_GUILD_CREATE {
            guild_name: "Keepers".into(),
        })
        .await;
        set_money(&realm, ga, 1000).await;
        let jerky = item_of(&realm, ga, 117).await.unwrap();
        a.send(CMSG_SEND_MAIL {
            mailbox: Guid::zero(),
            receiver: "Bob".into(),
            subject: "kept".into(),
            body: String::new(),
            unknown1: 0,
            unknown2: 0,
            item: jerky,
            money: Gold::new(0),
            cash_on_delivery_amount: 0,
            unknown3: 0,
            unknown4: 0,
        })
        .await;
        a.wait_for("SMSG_SEND_MAIL_RESULT", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_SEND_MAIL_RESULT(_)).then_some(())
        })
        .await;
        realm.shutdown().await;
        drop((a, b));
        let (_store, startup) = crate::game::store::Store::open(&db).unwrap();
        let social = crate::game::social::Social::load(&startup.records);
        assert_eq!(
            social
                .guilds
                .lock()
                .guilds
                .values()
                .map(|g| g.name.clone())
                .collect::<Vec<_>>(),
            vec!["Keepers".to_owned()]
        );
        let mails = social.mail.lock();
        let kept = mails.mails.values().next().expect("the mail");
        assert_eq!(kept.receiver, guid::counter(gb));
        assert_eq!(
            kept.item.as_ref().map(|i| i.guid),
            Some(guid::counter(jerky))
        );
        drop(mails);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

mod instances {
    use super::*;
    use crate::game::data::*;
    use crate::game::map::MapKey;

    fn data() -> GameData {
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
        d
    }

    /// Send `c` into `map`'s copy and wait for it to land.
    async fn go(realm: &std::sync::Arc<crate::game::Realm>, c: &mut Client, map: u32) {
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
    }

    fn key_of(realm: &crate::game::Realm, g: Guid) -> MapKey {
        realm.registry.online(g).unwrap().map().key
    }

    #[tokio::test]
    async fn a_group_shares_its_copy_and_a_player_alone_has_their_own() {
        let realm = harness::realm_with(data());
        let (_realm, mut a, mut b) = two_near_on(realm.clone()).await;
        let mut c = Client::connect(&realm, "C");
        c.enter("Carol", Race::Human, Class::Priest).await;
        a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
        b.wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
        b.send(CMSG_GROUP_ACCEPT {}).await;
        a.wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
            _ => None,
        })
        .await;
        go(&realm, &mut a, 36).await;
        go(&realm, &mut b, 36).await;
        go(&realm, &mut c, 36).await;
        let (ka, kb, kc) = (
            key_of(&realm, a.guid),
            key_of(&realm, b.guid),
            key_of(&realm, c.guid),
        );
        assert_eq!(ka.map, 36);
        assert!(ka.instance > 0);
        assert_eq!(ka, kb, "the group's copy");
        assert_ne!(ka, kc, "Carol's own");
        // In it, Alice and Bob see each other, and not Carol.
        a.sees(b.guid).await;
        assert!(!a.sees_within(c.guid, NONE_WITHIN).await);
    }

    #[tokio::test]
    async fn an_empty_copy_unloads_and_a_reset_starts_afresh() {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        a.enter("Alice", Race::Human, Class::Warrior).await;
        go(&realm, &mut a, 36).await;
        let first = key_of(&realm, a.guid);
        go(&realm, &mut a, 0).await;
        // Empty, it unloads; the next visit is a new copy.
        let mut gone = false;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if !realm.maps.all().iter().any(|h| h.key == first) {
                gone = true;
                break;
            }
        }
        assert!(gone, "the copy unloads");
        go(&realm, &mut a, 36).await;
        let second = key_of(&realm, a.guid);
        assert_ne!(first, second);
        go(&realm, &mut a, 0).await;
        a.send(CMSG_RESET_INSTANCES {}).await;
        let reset = a
            .wait_for("SMSG_INSTANCE_RESET", |m| match m {
                ServerOpcodeMessage::SMSG_INSTANCE_RESET(r) => Some(r.map),
                _ => None,
            })
            .await;
        assert_eq!(reset.as_int(), 36);
    }

    #[tokio::test]
    async fn a_raid_needs_a_raid_group_and_going_in_binds_no_one() {
        let mut cfg = harness::test_config();
        cfg.everyone_gm = false;
        let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
        let realm = crate::game::Realm::new(cfg, store, startup, data(), None);
        let (_realm, mut a, mut b) = two_near_on(realm.clone()).await;
        let ga = a.guid;
        realm.registry.online(ga).unwrap().map().call(move |m| {
            m.teleport(
                ga,
                MapKey::world(409),
                Vector3d {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                0.0,
            )
        });
        // Refused as Thistle Tea refuses it: a raid group only.
        a.wait_for("the raid's refusal", |m| match m {
            ServerOpcodeMessage::SMSG_RAID_GROUP_ONLY(r) => {
                (r.homebind_timer == 0 && r.error == RaidGroupError::Required).then_some(())
            }
            _ => None,
        })
        .await;
        a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
        b.wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
        b.send(CMSG_GROUP_ACCEPT {}).await;
        a.send(CMSG_GROUP_RAID_CONVERT {}).await;
        a.wait_for("a raid roster", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => {
                (l.group_type == GroupType::Raid).then_some(())
            }
            _ => None,
        })
        .await;
        go(&realm, &mut a, 409).await;
        assert!(key_of(&realm, ga).instance > 0);
        // A raid binds when a boss that locks it dies (tests_scripts), not
        // on the way in.
        a.send(CMSG_REQUEST_RAID_INFO {}).await;
        let infos = a
            .wait_for("SMSG_RAID_INSTANCE_INFO", |m| match m {
                ServerOpcodeMessage::SMSG_RAID_INSTANCE_INFO(r) => Some(
                    r.raid_infos
                        .iter()
                        .map(|i| i.instance_id)
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .await;
        assert!(infos.is_empty());
    }
}

mod warsong {
    use super::*;
    use crate::game::data::*;
    use crate::game::map::MapKey;
    use crate::game::map::battleground::{CAPTURE_TRIGGERS, Flag};

    fn data() -> GameData {
        let mut d = GameData::empty();
        d.maps.insert(
            489,
            MapInfo {
                id: 489,
                internal_name: "PVPZone03".into(),
                name: "Warsong Gulch".into(),
                instance_type: 3,
                player_limit: 10,
            },
        );
        d.battlegrounds.insert(
            2,
            BgTemplate {
                id: 2,
                min_players: 5,
                max_players: 10,
                min_level: 1,
                max_level: 60,
                starts: [769, 770],
                ..Default::default()
            },
        );
        d.safe_locs.insert(769, (489, 1519.53, 1481.87, 352.02));
        d.safe_locs.insert(770, (489, 933.33, 1433.72, 345.54));
        for (id, (x, y, z)) in [
            (3646, (1540.57, 1481.47, 351.83)),
            (3647, (916.17, 1434.47, 345.42)),
        ] {
            d.area_triggers.insert(
                id,
                AreaTrigger {
                    id,
                    map: 489,
                    x,
                    y,
                    z,
                    radius: 5.0,
                    ..Default::default()
                },
            );
        }
        d
    }

    async fn queue(c: &Client) {
        c.send(CMSG_BATTLEMASTER_JOIN {
            guid: Guid::zero(),
            map: Map::WarsongGulch,
            instance_id: 0,
            join_as_group: false,
        })
        .await;
    }

    async fn into_the_gulch(realm: &std::sync::Arc<crate::game::Realm>, c: &mut Client) {
        c.wait_frame(
            "the invitation",
            crate::game::battlegrounds::SMSG_BATTLEFIELD_STATUS,
            |f| f.len() >= 17 && f[13] == 2,
        )
        .await;
        c.send(CMSG_BATTLEFIELD_PORT {
            map: Map::WarsongGulch,
            action: BattlefieldPortAction::EnterBattle,
        })
        .await;
        c.wait_for("SMSG_NEW_WORLD", |m| match m {
            ServerOpcodeMessage::SMSG_NEW_WORLD(w) => (w.map == Map::WarsongGulch).then_some(()),
            _ => None,
        })
        .await;
        c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
        let g = c.guid;
        c.sees(g).await;
        let _ = realm;
    }

    /// Move `g` within the match's map, at once.
    async fn put(map: &crate::game::map::MapHandle, c: &mut Client, x: f32, y: f32, z: f32) {
        let g = c.guid;
        map.query(move |m| m.teleport(g, m.key, Vector3d { x, y, z }, 0.0))
            .await;
        c.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
            matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
        })
        .await;
        c.send(MSG_MOVE_TELEPORT_ACK_Client {
            guid: g,
            movement_counter: 0,
            time: Default::default(),
        })
        .await;
    }

    #[tokio::test]
    async fn a_warsong_gulch_match_runs_to_a_win() {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        a.enter("Alice", Race::Human, Class::Warrior).await;
        let mut b = Client::connect(&realm, "B");
        b.enter("Grom", Race::Orc, Class::Warrior).await;
        queue(&a).await;
        queue(&b).await;
        into_the_gulch(&realm, &mut a).await;
        into_the_gulch(&realm, &mut b).await;
        let key = realm.registry.online(a.guid).unwrap().map().key;
        assert_eq!(key.map, 489);
        assert_eq!(key, realm.registry.online(b.guid).unwrap().map().key);
        let map = realm.maps.get(&realm, key);
        // The flags go up.
        let mut horde_flag = None;
        for _ in 0..100 {
            horde_flag = map
                .query(|m| match m.battleground.as_ref().and_then(|b| b.flags[1]) {
                    Some(Flag::AtBase(go)) => Some(go),
                    _ => None,
                })
                .await
                .flatten();
            if horde_flag.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(horde_flag.is_some(), "the flags go up");
        for capture in 1..=3 {
            // After a capture the flags stand again 23 seconds on: now.
            let mut flag = None;
            for _ in 0..100 {
                flag = map
                    .query(|m| {
                        let b = m.battleground.as_mut()?;
                        for t in b.timers.iter_mut() {
                            t.0 = std::time::Instant::now();
                        }
                        match b.flags[1] {
                            Some(Flag::AtBase(go)) => Some(go),
                            _ => None,
                        }
                    })
                    .await
                    .flatten();
                if flag.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let flag = flag.expect("the Horde's flag at its base");
            put(&map, &mut a, 916.0, 1434.0, 345.5).await;
            a.send(CMSG_GAMEOBJ_USE { guid: flag }).await;
            // The Horde's flag carried: the Alliance's side shows it (2339).
            a.wait_for("the flag taken", |m| match m {
                ServerOpcodeMessage::SMSG_UPDATE_WORLD_STATE(w) => {
                    (w.state.state == 2339 && w.state.value == 2).then_some(())
                }
                _ => None,
            })
            .await;
            put(&map, &mut a, 1540.5, 1481.5, 351.8).await;
            a.send(CMSG_AREATRIGGER {
                trigger_id: CAPTURE_TRIGGERS[0],
            })
            .await;
            // Every change sends all the states: the capture's count.
            let score = a
                .wait_for("the capture", |m| match m {
                    ServerOpcodeMessage::SMSG_UPDATE_WORLD_STATE(w)
                        if w.state.state == 1581 && w.state.value == capture =>
                    {
                        Some(w.state.value)
                    }
                    _ => None,
                })
                .await;
            assert_eq!(score, capture);
        }
        for c in [&mut a, &mut b] {
            let winner = c
                .wait_for("the end", |m| match m {
                    ServerOpcodeMessage::MSG_PVP_LOG_DATA(l) => match &l.status {
                        MSG_PVP_LOG_DATA_Server_BattlegroundEndStatus::Ended { winner } => {
                            Some(*winner)
                        }
                        _ => None,
                    },
                    _ => None,
                })
                .await;
            assert_eq!(winner, BattlegroundWinner::Alliance);
        }
        // Grom leaves; the match is over, so no deserter.
        b.send(CMSG_LEAVE_BATTLEFIELD {
            map: Map::WarsongGulch,
        })
        .await;
        b.wait_for("home again", |m| match m {
            ServerOpcodeMessage::SMSG_NEW_WORLD(w) => (w.map == Map::Kalimdor).then_some(()),
            _ => None,
        })
        .await;
        let _ = MapKey::world(0);
    }
}

mod transports {
    use super::world::START;
    use super::*;
    use crate::game::data::npc::TaxiPathNode;
    use crate::game::data::*;

    const BOAT: u32 = 9999;

    /// A boat that sails 100 yards from the start on Eastern Kingdoms,
    /// crosses to Kalimdor, sails 100 more, and back, every 1.6 seconds.
    fn data() -> GameData {
        let mut d = GameData::empty();
        let mut data = [0u32; 24];
        data[0] = 77;
        // Fast, so the test's round is short.
        data[1] = 1000;
        data[2] = 1000;
        d.gameobject_templates.insert(
            BOAT,
            GameObjectTemplate {
                entry: BOAT,
                kind: 15,
                display_id: 3015,
                name: "The Test Boat".into(),
                size: 1.0,
                data,
                ..Default::default()
            },
        );
        d.transports.push(TransportRoute {
            entry: BOAT,
            name: "Here and There".into(),
            period: 1600,
        });
        let node = |map, x: f32, y: f32| TaxiPathNode {
            map,
            x,
            y,
            z: START.2,
            flags: 0,
            delay: 0,
        };
        d.npc.taxi_path_nodes.insert(
            77,
            vec![
                node(0, START.0, START.1),
                node(0, START.0 + 30.0, START.1),
                node(0, START.0 + 60.0, START.1),
                node(0, START.0 + 100.0, START.1),
                node(1, 990.0, 1000.0),
                node(1, 1000.0, 1000.0),
                node(1, 1010.0, 1000.0),
                node(1, 1020.0, 1000.0),
            ],
        );
        d
    }

    #[tokio::test]
    async fn a_boat_crossing_carries_its_passengers() {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
        let boat = crate::game::guid::make(crate::game::guid::HighGuid::MoTransport, 0, BOAT);
        // Everyone on its map knows the boat, however far.
        assert!(
            a.sees_within(boat, Duration::from_secs(3)).await,
            "the boat"
        );
        // Aboard, a yard off its middle.
        a.send(MSG_MOVE_HEARTBEAT_Client {
            info: MovementInfo {
                flags: MovementInfo_MovementFlags::empty().set_on_transport(
                    MovementInfo_MovementFlags_OnTransport {
                        transport: TransportInfo {
                            guid: boat,
                            position: Vector3d {
                                x: 1.0,
                                y: 0.0,
                                z: 0.0,
                            },
                            orientation: 0.0,
                            timestamp: 0,
                        },
                    },
                ),
                timestamp: 0,
                position: Vector3d {
                    x: START.0 + 1.0,
                    y: START.1,
                    z: START.2,
                },
                orientation: 0.0,
                fall_time: 0.0,
            },
        })
        .await;
        let by = a
            .wait_for("SMSG_TRANSFER_PENDING aboard", |m| match m {
                ServerOpcodeMessage::SMSG_TRANSFER_PENDING(t) => {
                    t.has_transport.as_ref().map(|h| (t.map, h.transport))
                }
                _ => None,
            })
            .await;
        assert_eq!(by, (Map::Kalimdor, BOAT));
        let at = a
            .wait_for("SMSG_NEW_WORLD", |m| match m {
                ServerOpcodeMessage::SMSG_NEW_WORLD(w) => Some((w.map, w.position)),
                _ => None,
            })
            .await;
        assert_eq!(at.0, Map::Kalimdor);
        assert!((at.1.x - 1001.0).abs() < 30.0, "on deck: {:?}", at.1);
        a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
        a.sees(ga).await;
        assert_eq!(realm.registry.online(ga).unwrap().map().key.map, 1);
    }
}

mod world_around {
    use super::world::START;
    use super::*;
    use crate::game::data::*;
    use crate::game::map::MapKey;

    fn data() -> GameData {
        let mut d = super::combat::data();
        let now = chrono::Utc::now().timestamp();
        d.events.insert(
            9,
            GameEvent {
                id: 9,
                name: "Always".into(),
                start: now - 60,
                end: now + 86_400,
                occurence: 1440,
                length: 1440,
                hardcoded: false,
            },
        );
        let spawn = |guid, dx: f32| CreatureSpawn {
            guid,
            entries: vec![2001],
            map: 0,
            x: START.0 + dx,
            y: START.1 + 5.0,
            z: START.2,
            respawn_min: 1,
            respawn_max: 1,
            ..Default::default()
        };
        d.creature_spawns
            .get_mut(&0)
            .unwrap()
            .extend([spawn(20, 1.0), spawn(21, 2.0)]);
        // 20 comes with the event, 21 goes with it.
        d.event_creatures.insert(20, 9);
        d.event_creatures.insert(21, -9);
        d.weather.insert(12, [[100, 0, 0]; 4]);
        d
    }

    #[tokio::test]
    async fn event_spawns_come_and_go_with_their_event() {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        a.enter("Alice", Race::Human, Class::Warrior).await;
        let (with, without) = (guid::unit(2001, 20), guid::unit(2001, 21));
        a.sees(with).await;
        assert!(!a.sees_within(without, NONE_WITHIN).await);
        // The event ends.
        let map = realm.maps.get(&realm, MapKey::world(0));
        map.query(|m| crate::game::events::apply(m, &Default::default()))
            .await;
        a.forgets(with).await;
        a.sees(without).await;
    }

    #[tokio::test]
    async fn the_weather_changes_by_its_chances_and_the_zone_hears() {
        let realm = harness::realm_with(data());
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
        // Into Elwynn Forest, where it always rains when it changes.
        a.send(CMSG_ZONEUPDATE {
            area: Area::ElwynnForest,
        })
        .await;
        a.wait_for("the weather on entering", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_WEATHER(_)).then_some(())
        })
        .await;
        for _ in 0..50 {
            crate::game::weather::sweep(&realm);
            if let Some(w) = realm.social.weather.lock().get(&12).copied()
                && w.kind == 1
            {
                break;
            }
            realm.social.weather.lock().entry(12).or_default().next = 0;
        }
        let rain = a
            .wait_for("rain", |m| match m {
                ServerOpcodeMessage::SMSG_WEATHER(w) => {
                    (w.weather_type == WeatherType::Rain).then_some(())
                }
                _ => None,
            })
            .await;
        let _ = (rain, ga);
    }
}

mod security {
    use super::world::START;
    use super::*;

    /// Players, not GMs.
    fn players_realm() -> std::sync::Arc<crate::game::Realm> {
        let mut cfg = harness::test_config();
        cfg.everyone_gm = false;
        cfg.max_packets_per_sec = 50;
        let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
        crate::game::Realm::new(cfg, store, startup, Default::default(), None)
    }

    #[tokio::test]
    async fn a_player_cant_run_a_gm_command_and_a_moderator_only_some() {
        let realm = players_realm();
        let (_realm, mut a, mut b) = two_near_on(realm.clone()).await;
        a.say(".go -9000 -100 90").await;
        // It's only chat.
        let heard = b
            .wait_for("Alice's line", |m| match m {
                ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => Some(c.message.clone()),
                _ => None,
            })
            .await;
        assert_eq!(heard, ".go -9000 -100 90");
        assert!(
            a.gets(NONE_WITHIN, |m| {
                matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
            })
            .await
            .is_none()
        );
        // A moderator may look, not teleport.
        let ga = a.guid;
        let map = realm.registry.online(ga).unwrap().map();
        map.query(move |m| m.players.get_mut(&ga).unwrap().gm_level = crate::accounts::MODERATOR)
            .await;
        a.say(".go -9000 -100 90").await;
        a.system_message("permission").await;
        a.say(".gps").await;
        a.system_message("map 0").await;
    }

    #[tokio::test]
    async fn a_client_moving_too_fast_is_pulled_back() {
        let realm = players_realm();
        let mut a = Client::connect(&realm, "A");
        a.enter("Alice", Race::Human, Class::Warrior).await;
        // A step, then five hundred yards in no time.
        a.heartbeat(START.0 + 1.0, START.1, START.2).await;
        a.heartbeat(START.0 + 500.0, START.1, START.2).await;
        let back = a
            .wait_for("MSG_MOVE_TELEPORT_ACK", |m| match m {
                ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => Some(t.info.position),
                _ => None,
            })
            .await;
        assert!((back.x - (START.0 + 1.0)).abs() < 0.01, "{back:?}");
    }

    #[tokio::test]
    async fn a_packet_flood_is_cut_off() {
        let realm = players_realm();
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
        for _ in 0..400 {
            if !a.try_send(CMSG_SET_SELECTION { target: ga }).await {
                break;
            }
        }
        let mut gone = false;
        for _ in 0..200 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            if realm.registry.online(ga).is_none() {
                gone = true;
                break;
            }
        }
        assert!(gone, "the flooder is disconnected");
    }

    #[tokio::test]
    async fn chat_floods_are_muted() {
        let realm = players_realm();
        let mut a = Client::connect(&realm, "A");
        a.enter("Alice", Race::Human, Class::Warrior).await;
        for i in 0..12 {
            a.say(&format!("spam {i}")).await;
        }
        a.system_message("muted").await;
    }

    #[tokio::test]
    async fn names_follow_the_rules() {
        let realm = players_realm();
        let mut a = Client::connect(&realm, "A");
        for (name, result) in [
            ("Aabb", WorldResult::CharCreateSuccess),
            // mangos-zero allows three of a letter in a row (TBC's cores don't).
            ("Aaaab", WorldResult::CharCreateSuccess),
            ("Bob1", WorldResult::CharNameOnlyLetters),
            ("B", WorldResult::CharNameTooShort),
            ("Bobbobbobbobb", WorldResult::CharNameTooLong),
        ] {
            a.send(CMSG_CHAR_CREATE {
                name: name.into(),
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
            let got = a
                .wait_for("SMSG_CHAR_CREATE", |m| match m {
                    ServerOpcodeMessage::SMSG_CHAR_CREATE(c) => Some(c.result),
                    _ => None,
                })
                .await;
            assert_eq!(got, result, "{name}");
        }
    }

    #[tokio::test]
    async fn a_panic_on_a_map_costs_one_event_not_the_map() {
        let realm = harness::realm();
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
        let map = realm.registry.online(ga).unwrap().map();
        map.call(|_| panic!("a bug"));
        let still = map.query(move |m| m.players.contains_key(&ga)).await;
        assert_eq!(still, Some(true));
    }

    #[tokio::test]
    async fn a_long_fall_hurts() {
        let realm = players_realm();
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
        let at = |z: f32, falling: bool| MSG_MOVE_HEARTBEAT_Client {
            info: MovementInfo {
                flags: if falling {
                    MovementInfo_MovementFlags::empty().set_fallingfar()
                } else {
                    MovementInfo_MovementFlags::empty()
                },
                timestamp: 0,
                position: Vector3d {
                    x: START.0,
                    y: START.1,
                    z,
                },
                orientation: 0.0,
                // The client's fall time, in ms (a u32 wow_messages reads as a
                // float): long enough to hurt.
                fall_time: f32::from_bits(2000),
            },
        };
        a.send(at(START.2 + 30.0, true)).await;
        a.send(at(START.2, false)).await;
        let (who, damage) = a
            .wait_for("SMSG_ENVIRONMENTAL_DAMAGE_LOG", |m| match m {
                ServerOpcodeMessage::SMSG_ENVIRONMENTAL_DAMAGE_LOG(d) => Some((d.guid, d.damage)),
                _ => None,
            })
            .await;
        assert_eq!(who, ga);
        assert!(damage > 0);
    }
}
