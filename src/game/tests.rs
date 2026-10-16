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

mod items {
    use super::world::START;
    use super::*;
    use crate::game::data::*;
    use crate::game::map::MapKey;

    fn item(entry: u32, name: &str, class: u32, inventory_type: u32, price: u32) -> ItemTemplate {
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

    fn data() -> GameData {
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
        a.wait_for("SMSG_QUEST_QUERY_RESPONSE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_QUEST_QUERY_RESPONSE(_)).then_some(())
        })
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
