//! The last sweep of Thistle Tea's rules through the protocol: logout's
//! admission and posture, emotes and postures, falls, the who list,
//! ready checks, the character screen, flights, trainers, innkeepers and
//! the NPCs a player may use, and instance copies' sizes.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const NONE_WITHIN: Duration = Duration::from_millis(300);

const FOOD: u32 = 433;
const DANCE: u32 = 10;

fn data() -> GameData {
    let mut d = GameData::empty();
    // /dance, a state; /wave, once.
    d.text_emote_anims.insert(0x22, DANCE);
    d.text_emote_anims.insert(0x65, 3);
    d.emote_states.insert(DANCE);
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura: 20,
        base_points: 9,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(
        FOOD,
        SpellTemplate {
            id: FOOD,
            name: "Food".into(),
            effects,
            aura_interrupt_flags: 0x40000,
            duration_index: 0,
            equipped_item_class: -1,
            ..Default::default()
        },
    );
    d
}

fn players_realm(d: GameData) -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, d, None)
}

fn map0(realm: &Arc<Realm>) -> MapHandle {
    realm.maps.get(realm, MapKey::world(0))
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map0(realm).query(f).await.expect("the map answers")
}

fn movement(flags: MovementInfo_MovementFlags, z: f32, fall_ms: u32) -> MovementInfo {
    MovementInfo {
        flags,
        timestamp: 0,
        position: Vector3d {
            x: START.0,
            y: START.1,
            z,
        },
        orientation: 0.0,
        fall_time: f32::from_bits(fall_ms),
    }
}

async fn logout_result(a: &mut Client) -> (LogoutResult, LogoutSpeed) {
    a.wait_for("SMSG_LOGOUT_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_LOGOUT_RESPONSE(r) => Some((r.result, r.speed)),
        _ => None,
    })
    .await
}

#[tokio::test]
async fn a_logout_is_refused_in_the_air_and_marks_the_player_logging_out() {
    let realm = players_realm(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.send(MSG_MOVE_JUMP_Client {
        info: movement(
            MovementInfo_MovementFlags::empty().set_fallingfar(),
            START.2 + 1.0,
            100,
        ),
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    assert_eq!(
        logout_result(&mut a).await.0,
        LogoutResult::FailureJumpingOrFalling
    );
    // Landed and mounted: the countdown, the logging-out flag, but no seat.
    a.send(MSG_MOVE_FALL_LAND_Client {
        info: movement(MovementInfo_MovementFlags::empty(), START.2, 100),
    })
    .await;
    on_map(&realm, move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_MOUNTDISPLAYID, 2402)
    })
    .await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    assert_eq!(
        logout_result(&mut a).await,
        (LogoutResult::Success, LogoutSpeed::Delayed)
    );
    let (bytes, stand) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (p.fields.get_byte(PLAYER_FIELD_BYTES, 0), p.stand_state())
    })
    .await;
    assert_eq!(bytes & 0x04, 0x04, "logging out");
    assert_eq!(stand, 0, "a rider isn't sat down");
    a.send(CMSG_LOGOUT_CANCEL {}).await;
    a.wait_for("SMSG_LOGOUT_CANCEL_ACK", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_CANCEL_ACK).then_some(())
    })
    .await;
    let bytes = on_map(&realm, move |m| {
        m.players[&g].fields.get_byte(PLAYER_FIELD_BYTES, 0)
    })
    .await;
    assert_eq!(bytes & 0x04, 0);
    // Frozen by a GM: no logout.
    on_map(&realm, move |m| {
        crate::game::map::spells::cast_triggered(m, g, 9454, Some(g))
    })
    .await;
    let frozen = on_map(&realm, move |m| m.players[&g].auras.has_spell(9454)).await;
    if frozen {
        a.send(CMSG_LOGOUT_REQUEST {}).await;
        assert_eq!(
            logout_result(&mut a).await.0,
            LogoutResult::FailureFrozenByGm
        );
    }
}

#[tokio::test]
async fn a_logout_on_a_flight_is_at_once() {
    let realm = players_realm(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    on_map(&realm, move |m| {
        let now = m.now;
        let p = m.players.get_mut(&g).unwrap();
        let walk = crate::game::map::creature::Walk::new(
            vec![
                p.position(),
                Vector3d {
                    x: START.0 + 100.0,
                    ..p.position()
                },
            ],
            32.0,
            now,
            1,
            true,
        );
        p.flight = Some(crate::game::map::npc::Flight {
            walk,
            destination: 2,
            mount: 1149,
        });
    })
    .await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    assert_eq!(logout_result(&mut a).await.1, LogoutSpeed::Instant);
}

fn emote_state(m: &Map, g: Guid) -> u32 {
    m.players[&g].fields.get_u32(UNIT_NPC_EMOTESTATE)
}

#[tokio::test]
async fn a_dance_stays_until_the_dancer_moves_and_the_client_plays_only_two_emotes() {
    let realm = players_realm(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.send(CMSG_TEXT_EMOTE {
        text_emote: TextEmote::Dance,
        emote: 0,
        target: Guid::zero(),
    })
    .await;
    a.wait_for("the dance's text", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_TEXT_EMOTE(_)).then_some(())
    })
    .await;
    assert_eq!(on_map(&realm, move |m| emote_state(m, g)).await, DANCE);
    // A step ends it.
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: MovementInfo {
            position: Vector3d {
                x: START.0 + 1.0,
                y: START.1,
                z: START.2,
            },
            ..movement(MovementInfo_MovementFlags::empty(), START.2, 0)
        },
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(on_map(&realm, move |m| emote_state(m, g)).await, 0);
    // The client's own emotes: a wave, not a dance.
    a.clear();
    a.send(CMSG_EMOTE {
        emote: Emote::StateDance,
    })
    .await;
    assert!(
        a.gets(NONE_WITHIN, |m| matches!(
            m,
            ServerOpcodeMessage::SMSG_EMOTE(_)
        )
        .then_some(()))
            .await
            .is_none()
    );
    a.send(CMSG_EMOTE {
        emote: Emote::OneshotWave,
    })
    .await;
    a.wait_for("the wave", |m| match m {
        ServerOpcodeMessage::SMSG_EMOTE(e) => (e.emote == Emote::OneshotWave).then_some(()),
        _ => None,
    })
    .await;
    // The dead don't dance.
    on_map(&realm, move |m| crate::game::map::death::kill(m, g)).await;
    a.clear();
    a.send(CMSG_TEXT_EMOTE {
        text_emote: TextEmote::Dance,
        emote: 0,
        target: Guid::zero(),
    })
    .await;
    assert!(
        a.gets(NONE_WITHIN, |m| matches!(
            m,
            ServerOpcodeMessage::SMSG_TEXT_EMOTE(_)
        )
        .then_some(()))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn standing_up_ends_a_meal_and_no_one_claims_to_be_dead() {
    let realm = players_realm(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.send(CMSG_STANDSTATECHANGE {
        animation_state: UnitStandState::Dead,
    })
    .await;
    a.send(CMSG_STANDSTATECHANGE {
        animation_state: UnitStandState::Sit,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let eating = on_map(&realm, move |m| {
        crate::game::map::spells::cast_triggered(m, g, FOOD, Some(g));
        (
            m.players[&g].stand_state(),
            m.players[&g].auras.has_spell(FOOD),
        )
    })
    .await;
    assert_eq!(eating, (1, true), "seated, not dead, and eating");
    a.send(CMSG_STANDSTATECHANGE {
        animation_state: UnitStandState::Stand,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!on_map(&realm, move |m| m.players[&g].auras.has_spell(FOOD)).await);
}

async fn fall(a: &mut Client, height: f32, fall_ms: u32) {
    a.send(MSG_MOVE_JUMP_Client {
        info: movement(
            MovementInfo_MovementFlags::empty().set_fallingfar(),
            START.2 + height,
            0,
        ),
    })
    .await;
    a.send(MSG_MOVE_FALL_LAND_Client {
        info: movement(MovementInfo_MovementFlags::empty(), START.2, fall_ms),
    })
    .await;
}

#[tokio::test]
async fn a_fall_hurts_only_when_long_and_never_a_god() {
    let realm = players_realm(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let hurt = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::SMSG_ENVIRONMENTAL_DAMAGE_LOG(d) => Some(d.damage),
        _ => None,
    };
    // Thirty yards in a moment (a packet's word for it): no harm.
    fall(&mut a, 30.0, 400).await;
    assert!(a.gets(NONE_WITHIN, hurt).await.is_none());
    // Long enough: 1.8% a yard less a quarter, of the maximum.
    fall(&mut a, 30.0, 2000).await;
    let damage = a.wait_for("the fall's damage", hurt).await;
    let max = on_map(&realm, move |m| {
        m.players[&g].fields.get_u32(UNIT_FIELD_MAXHEALTH)
    })
    .await;
    assert_eq!(damage, ((0.018 * 30.0 - 0.2426) * max as f32) as u32);
    // A god falls unhurt.
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.misc.god = true;
        p.fields.set_u32(UNIT_FIELD_HEALTH, max);
    })
    .await;
    fall(&mut a, 30.0, 2000).await;
    assert!(a.gets(NONE_WITHIN, hurt).await.is_none());
}

async fn who(a: &mut Client, guild: &str, words: &[&str]) -> (u32, u32, Vec<String>) {
    a.send(CMSG_WHO {
        minimum_level: Level::new(0),
        maximum_level: Level::new(100),
        player_name: String::new(),
        guild_name: guild.into(),
        race_mask: u32::MAX,
        class_mask: u32::MAX,
        zones: Vec::new(),
        search_strings: words.iter().map(|w| (*w).into()).collect(),
    })
    .await;
    let b = a.wait_frame("SMSG_WHO", 0x063, |_| true).await;
    let listed = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let online = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    let mut names = Vec::new();
    let mut at = 8;
    for _ in 0..listed {
        let end = at + b[at..].iter().position(|c| *c == 0).unwrap();
        names.push(String::from_utf8_lossy(&b[at..end]).into_owned());
        at = end + 1;
        let guild_end = at + b[at..].iter().position(|c| *c == 0).unwrap();
        at = guild_end + 1 + 16;
    }
    (listed, online, names)
}

#[tokio::test]
async fn who_filters_by_guild_and_any_word() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    a.send(CMSG_GUILD_CREATE {
        guild_name: "Knights".into(),
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (listed, online, names) = who(&mut a, "knig", &[]).await;
    assert_eq!((listed, online, names), (1, 1, vec!["Alice".to_owned()]));
    // Any word, of the name or the guild.
    let (_, _, mut names) = who(&mut a, "", &["bob", "knights"]).await;
    names.sort();
    assert_eq!(names, ["Alice", "Bob"]);
}

#[tokio::test]
async fn a_ready_check_hears_offline_members_are_not_ready() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
    b.wait_for("the invite", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    // Alice marks Bob with a skull, then Bob goes.
    a.send(MSG_RAID_TARGET_UPDATE_Client {
        target_index: MSG_RAID_TARGET_UPDATE_Client_RaidTargetIndex::Unknown7 { target: gb },
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    b.send(CMSG_LOGOUT_REQUEST {}).await;
    b.wait_for("logged out", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    a.clear();
    a.send(MSG_RAID_READY_CHECK_Client { answer: None }).await;
    a.wait_for("Bob not ready", |m| match m {
        ServerOpcodeMessage::MSG_RAID_READY_CHECK(r) => r
            .state_check
            .as_ref()
            .filter(|s| s.guid == gb && s.state == 0)
            .map(|_| ()),
        _ => None,
    })
    .await;
    // Back, Bob gets the markers with the roster.
    b.clear();
    b.login(gb).await;
    let marks = b.wait_frame("the markers", 0x321, |f| f[0] == 1).await;
    assert_eq!(marks[1], 7, "the skull");
    assert_eq!(&marks[2..10], &gb.guid().to_le_bytes());
    let _ = ga;
}

async fn logged_out(c: &mut Client) {
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
}

#[tokio::test]
async fn the_character_list_shows_the_guild_a_hidden_helm_and_a_ghost() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.send(CMSG_GUILD_CREATE {
        guild_name: "Knights".into(),
    })
    .await;
    a.send(CMSG_TOGGLE_HELM {}).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    logged_out(&mut a).await;
    a.send(CMSG_CHAR_ENUM {}).await;
    let (guild, flags) = a
        .wait_for("SMSG_CHAR_ENUM", |m| match m {
            ServerOpcodeMessage::SMSG_CHAR_ENUM(e) => {
                e.characters.first().map(|c| (c.guild_id, c.flags))
            }
            _ => None,
        })
        .await;
    assert_ne!(guild, 0);
    assert!(flags.is_hide_helm() && !flags.is_hide_cloak() && !flags.is_ghost());
    // Back in, the helm is still hidden.
    a.login(g).await;
    let hidden = on_map(&realm, move |m| {
        m.players[&g].fields.has_flag(
            PLAYER_FLAGS,
            crate::game::map::player::PLAYER_FLAGS_HIDE_HELM,
        )
    })
    .await;
    assert!(hidden);
    // Released, a ghost.
    a.say(".die").await;
    a.send(CMSG_REPOP_REQUEST {}).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    logged_out(&mut a).await;
    a.clear();
    a.send(CMSG_CHAR_ENUM {}).await;
    let flags = a
        .wait_for("SMSG_CHAR_ENUM", |m| match m {
            ServerOpcodeMessage::SMSG_CHAR_ENUM(e) => e.characters.first().map(|c| c.flags),
            _ => None,
        })
        .await;
    assert!(flags.is_ghost());
}

#[tokio::test]
async fn a_deleted_character_leaves_its_group_and_cod_comes_back() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Carol", Race::Human, Class::Warrior).await;
    for (who, name) in [(&mut b, "Bob"), (&mut c, "Carol")] {
        a.send(CMSG_GROUP_INVITE { name: name.into() }).await;
        who.wait_for("the invite", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
        who.send(CMSG_GROUP_ACCEPT {}).await;
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    // Alice sends Bob an item cash on delivery.
    let mail = crate::game::social::mail::Mail {
        kind: 0,
        sender: crate::game::guid::counter(ga),
        receiver: crate::game::guid::counter(gb),
        subject: "Pay up".into(),
        item: Some(crate::game::store::ItemRow {
            guid: 999_999,
            entry: 25,
            count: 1,
            ..Default::default()
        }),
        cod: 100,
        ..Default::default()
    };
    let id = crate::game::social::mail::post(&realm, mail);
    logged_out(&mut b).await;
    b.send(CMSG_CHAR_DELETE { guid: gb }).await;
    b.wait_for("SMSG_CHAR_DELETE", |m| match m {
        ServerOpcodeMessage::SMSG_CHAR_DELETE(d) => {
            (d.result == WorldResult::CharDeleteSuccess).then_some(())
        }
        _ => None,
    })
    .await;
    let in_group = realm.social.groups.lock().group_of(gb).is_some();
    assert!(!in_group, "Bob is out of the group");
    assert!(
        realm.social.groups.lock().group_of(gc).is_some(),
        "the others go on"
    );
    let back = realm
        .social
        .mail
        .lock()
        .mails
        .get(&id)
        .map(|m| (m.receiver, m.cod, m.item.is_some()));
    assert_eq!(back, Some((crate::game::guid::counter(ga), 0, true)));
}
