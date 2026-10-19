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
use crate::game::data::npc::{TaxiNode, TaxiPathNode};
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const NONE_WITHIN: Duration = Duration::from_millis(300);

const FOOD: u32 = 433;
const FEIGN_DEATH: u32 = 5384;
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
    // Feign Death: its aura (66) on oneself, which an animation would end.
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura: 66,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(
        FEIGN_DEATH,
        SpellTemplate {
            id: FEIGN_DEATH,
            name: "Feign Death".into(),
            effects,
            aura_interrupt_flags: crate::game::map::auras::INTERRUPT_ENTER_COMBAT,
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
    cfg.logout_delay = crate::game::map::login::LOGOUT_DELAY;
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
        fall_time: fall_ms,
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

/// Feign Death holds the body (`UNIT_FLAG_PREVENT_ANIM`): a text emote, a
/// client's emote and a posture are refused while it lasts, nothing played
/// nor heard, and the feign stays (mangos-classic's `HandleTextEmoteOpcode`,
/// `HandleEmoteOpcode`, `HandleStandStateChangeOpcode`; Thistle Tea's emote
/// ends it). Once it ends, a wave is seen again.
#[tokio::test]
async fn feign_death_refuses_emotes_and_postures_and_stays() {
    let realm = players_realm(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let feigning = on_map(&realm, move |m| {
        crate::game::map::spells::cast_triggered(m, g, FEIGN_DEATH, Some(g));
        m.players[&g].auras.has_spell(FEIGN_DEATH)
    })
    .await;
    assert!(feigning, "feigning");
    a.clear();
    a.send(CMSG_TEXT_EMOTE {
        text_emote: TextEmote::Wave,
        emote: 0,
        target: Guid::zero(),
    })
    .await;
    a.send(CMSG_EMOTE {
        emote: Emote::OneshotWave,
    })
    .await;
    a.send(CMSG_STANDSTATECHANGE {
        animation_state: UnitStandState::Sit,
    })
    .await;
    let heard = a
        .gets(NONE_WITHIN, |m| match m {
            ServerOpcodeMessage::SMSG_TEXT_EMOTE(_) => Some("a text emote"),
            ServerOpcodeMessage::SMSG_EMOTE(_) => Some("an emote"),
            ServerOpcodeMessage::SMSG_STANDSTATE_UPDATE(_) => Some("a posture"),
            _ => None,
        })
        .await;
    assert_eq!(heard, None, "refused while feigning");
    let (still, posture) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (p.auras.has_spell(FEIGN_DEATH), p.stand_state())
    })
    .await;
    assert!(still, "the feign stays");
    assert_eq!(posture, 0, "not sat down");
    // Up again, a wave plays.
    on_map(&realm, move |m| {
        crate::game::map::auras::remove_spell(m, g, FEIGN_DEATH, None)
    })
    .await;
    a.send(CMSG_EMOTE {
        emote: Emote::OneshotWave,
    })
    .await;
    a.wait_for("the wave", |m| match m {
        ServerOpcodeMessage::SMSG_EMOTE(e) => (e.emote == Emote::OneshotWave).then_some(()),
        _ => None,
    })
    .await;
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

const FLIGHTMASTER: u32 = 4100;
const TRAINER: u32 = 4101;
const INNKEEPER: u32 = 4102;
const VENDOR: u32 = 4103;
const RANK_1: u32 = 9400;
const RANK_2: u32 = 9401;
const BLACKSMITHING: u32 = 9402;
const HEARTHSTONE: u32 = 6948;

fn npc(entry: u32, flags: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: format!("Npc {entry}"),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 30,
        level_max: 30,
        faction: 35,
        npc_flags: flags,
        unit_class: 1,
        health_multiplier: 1.0,
        health_max: 500,
        ..Default::default()
    }
}

fn services() -> GameData {
    let mut d = data();
    let mut spawns = Vec::new();
    for (i, (entry, flags)) in [
        (FLIGHTMASTER, 0x8),
        (TRAINER, 0x10),
        (INNKEEPER, 0x80),
        (VENDOR, 0x4),
    ]
    .into_iter()
    .enumerate()
    {
        let mut t = npc(entry, flags);
        if entry == TRAINER {
            t.trainer_type = 2;
        }
        d.creature_templates.insert(entry, t);
        spawns.push(CreatureSpawn {
            guid: 50 + i as u32,
            entries: vec![entry],
            map: 0,
            x: START.0 + 2.0,
            y: START.1 + i as f32,
            z: START.2,
            respawn_min: 600,
            respawn_max: 600,
            ..Default::default()
        });
    }
    d.creature_spawns.insert(0, spawns);
    for (id, x) in [
        (2, START.0 + 3.0),
        (3, START.0 + 64.0),
        (4, START.0 + 300.0),
    ] {
        d.npc.taxi_nodes.insert(
            id,
            TaxiNode {
                id,
                map: 0,
                x,
                y: START.1,
                z: START.2,
                name: format!("Node {id}"),
                mounts: [0, 0],
            },
        );
    }
    let path = |x: f32| TaxiPathNode {
        map: 0,
        x,
        y: START.1,
        z: START.2,
        ..Default::default()
    };
    d.npc.taxi_paths.insert((2, 3), (9, 5));
    d.npc.taxi_paths.insert((4, 3), (10, 5));
    d.npc
        .taxi_path_nodes
        .insert(9, vec![path(START.0 + 3.0), path(START.0 + 64.0)]);
    d.npc
        .taxi_path_nodes
        .insert(10, vec![path(START.0 + 300.0), path(START.0 + 64.0)]);
    // Two ranks of a spell, and a profession's first rank.
    for id in [RANK_1, RANK_2] {
        d.spells.spells.insert(
            id,
            SpellTemplate {
                id,
                name: format!("Spell {id}"),
                equipped_item_class: -1,
                ..Default::default()
            },
        );
    }
    d.spells.ranks.insert(
        RANK_2,
        crate::game::data::spells::SpellRank {
            prev: RANK_1,
            first: RANK_1,
            rank: 2,
            req: 0,
        },
    );
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 44,
        misc_value: 164,
        base_points: 0,
        ..Default::default()
    };
    d.spells.spells.insert(
        BLACKSMITHING,
        SpellTemplate {
            id: BLACKSMITHING,
            name: "Blacksmithing".into(),
            effects,
            equipped_item_class: -1,
            ..Default::default()
        },
    );
    d.npc.trainers.insert(
        TRAINER,
        [RANK_2, BLACKSMITHING]
            .into_iter()
            .map(|spell| crate::game::data::npc::TrainerSpell {
                spell,
                cost: 100,
                ..Default::default()
            })
            .collect(),
    );
    d.items.insert(
        HEARTHSTONE,
        ItemTemplate {
            entry: HEARTHSTONE,
            name: "Hearthstone".into(),
            allowable_class: -1,
            allowable_race: -1,
            stackable: 1,
            max_count: 1,
            ..Default::default()
        },
    );
    d
}

async fn served(realm: &Arc<Realm>, a: &mut Client) {
    for (entry, counter) in [
        (FLIGHTMASTER, 50),
        (TRAINER, 51),
        (INNKEEPER, 52),
        (VENDOR, 53),
    ] {
        a.sees(crate::game::guid::unit(entry, counter)).await;
    }
    let g = a.guid;
    on_map(realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.set_money(100_000);
        p.row.state.taxi_mask = vec![0b11100];
    })
    .await;
}

async fn taxi_reply(a: &mut Client, from: u32, to: u32) -> ActivateTaxiReply {
    a.clear();
    a.send(CMSG_ACTIVATETAXI {
        guid: crate::game::guid::unit(FLIGHTMASTER, 50),
        source_node: from,
        destination_node: to,
    })
    .await;
    a.wait_for("SMSG_ACTIVATETAXIREPLY", |m| match m {
        ServerOpcodeMessage::SMSG_ACTIVATETAXIREPLY(r) => Some(r.reply),
        _ => None,
    })
    .await
}

#[tokio::test]
async fn a_flight_waits_for_a_free_standing_rider_at_its_node() {
    let realm = harness::realm_with(services());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    served(&realm, &mut a).await;
    let set = |f: fn(&mut crate::game::map::player::Player)| {
        let realm = realm.clone();
        async move { on_map(&realm, move |m| f(m.players.get_mut(&g).unwrap())).await }
    };
    set(|p| p.fields.set_u32(UNIT_FIELD_MOUNTDISPLAYID, 2402)).await;
    assert_eq!(
        taxi_reply(&mut a, 2, 3).await,
        ActivateTaxiReply::PlayerAlreadyMounted
    );
    set(|p| {
        p.fields.set_u32(UNIT_FIELD_MOUNTDISPLAYID, 0);
        p.set_stand_state(1);
    })
    .await;
    assert_eq!(
        taxi_reply(&mut a, 2, 3).await,
        ActivateTaxiReply::NotStanding
    );
    set(|p| p.set_stand_state(0)).await;
    assert_eq!(
        taxi_reply(&mut a, 4, 3).await,
        ActivateTaxiReply::TooFarAway
    );
    set(|p| p.row.state.taxi_mask = vec![0b00100]).await;
    assert_eq!(
        taxi_reply(&mut a, 2, 3).await,
        ActivateTaxiReply::NotVisited
    );
    set(|p| p.row.state.taxi_mask = vec![0b11100]).await;
    assert_eq!(taxi_reply(&mut a, 2, 3).await, ActivateTaxiReply::Ok);
}

async fn trainer_list(a: &mut Client) -> Vec<TrainerSpell> {
    a.clear();
    a.send(CMSG_TRAINER_LIST {
        guid: crate::game::guid::unit(TRAINER, 51),
    })
    .await;
    a.wait_for("SMSG_TRAINER_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_TRAINER_LIST(l) => Some(l.spells.clone()),
        _ => None,
    })
    .await
}

#[tokio::test]
async fn a_trainer_wants_the_rank_before_and_a_free_profession() {
    let realm = harness::realm_with(services());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    served(&realm, &mut a).await;
    let spells = trainer_list(&mut a).await;
    let state = |list: &[TrainerSpell], id| {
        list.iter()
            .find(|s| s.spell == id)
            .map(|s| (s.state, s.talent_point_cost, s.first_rank))
    };
    assert_eq!(
        state(&spells, RANK_2),
        Some((TrainerSpellState::Red, 0, 0)),
        "rank 1 first"
    );
    assert_eq!(
        state(&spells, BLACKSMITHING),
        Some((TrainerSpellState::Green, 1, 1))
    );
    // Two professions already: the first rank of a third can't be learned.
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.spells.push(RANK_1);
        crate::game::map::skills::learn_skill(p, 171, 1);
        crate::game::map::skills::learn_skill(p, 182, 1);
    })
    .await;
    let spells = trainer_list(&mut a).await;
    assert_eq!(
        state(&spells, RANK_2),
        Some((TrainerSpellState::Green, 0, 0))
    );
    assert_eq!(
        state(&spells, BLACKSMITHING),
        Some((TrainerSpellState::Green, 0, 1))
    );
    a.send(CMSG_TRAINER_BUY_SPELL {
        guid: crate::game::guid::unit(TRAINER, 51),
        id: BLACKSMITHING,
    })
    .await;
    a.wait_for("SMSG_TRAINER_BUY_FAILED", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_TRAINER_BUY_FAILED(_)).then_some(())
    })
    .await;
}

#[tokio::test]
async fn an_innkeeper_binds_to_the_area_and_gives_a_hearthstone_and_the_dead_are_not_served() {
    let realm = harness::realm_with(services());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    served(&realm, &mut a).await;
    on_map(&realm, move |m| m.players.get_mut(&g).unwrap().area = 87).await;
    a.send(CMSG_BINDER_ACTIVATE {
        guid: crate::game::guid::unit(INNKEEPER, 52),
    })
    .await;
    let area = a
        .wait_for("SMSG_PLAYERBOUND", |m| match m {
            ServerOpcodeMessage::SMSG_PLAYERBOUND(b) => Some(b.area),
            _ => None,
        })
        .await;
    assert_eq!(area.as_int(), 87);
    let (bound, stones) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (p.row.state.bind.map(|b| b.zone), p.count_of(HEARTHSTONE))
    })
    .await;
    assert_eq!((bound, stones), (Some(87), 1));
    // Dead, no vendor will deal.
    on_map(&realm, move |m| crate::game::map::death::kill(m, g)).await;
    a.clear();
    a.send(CMSG_LIST_INVENTORY {
        guid: crate::game::guid::unit(VENDOR, 53),
    })
    .await;
    assert!(
        a.gets(NONE_WITHIN, |m| matches!(
            m,
            ServerOpcodeMessage::SMSG_LIST_INVENTORY(_)
        )
        .then_some(()))
            .await
            .is_none()
    );
}

const EXPLORE_QUEST: u32 = 9500;
const EXPLORE_TRIGGER: u32 = 9600;

#[tokio::test]
async fn a_ghost_doesnt_explore_for_a_quest() {
    let mut d = data();
    d.area_triggers.insert(
        EXPLORE_TRIGGER,
        AreaTrigger {
            id: EXPLORE_TRIGGER,
            map: 0,
            x: START.0,
            y: START.1,
            z: START.2,
            radius: 10.0,
            ..Default::default()
        },
    );
    d.npc
        .area_trigger_quests
        .insert(EXPLORE_TRIGGER, EXPLORE_QUEST);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let explored = |realm: Arc<Realm>| async move {
        on_map(&realm, move |m| {
            m.players[&g]
                .journal
                .log
                .first()
                .is_some_and(|l| l.explored)
        })
        .await
    };
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.journal.log.push(crate::game::store::Logged {
            quest: EXPLORE_QUEST,
            ..Default::default()
        });
        crate::game::map::death::kill(m, g);
    })
    .await;
    a.send(CMSG_AREATRIGGER {
        trigger_id: EXPLORE_TRIGGER,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!explored(realm.clone()).await, "the dead explore nothing");
    a.say(".revive").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    a.send(CMSG_AREATRIGGER {
        trigger_id: EXPLORE_TRIGGER,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(explored(realm.clone()).await);
}

#[tokio::test]
async fn a_read_mail_goes_in_three_days() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let id = crate::game::social::mail::post(
        &realm,
        crate::game::social::mail::Mail {
            receiver: crate::game::guid::counter(g),
            subject: "Hello".into(),
            deliver_at: crate::game::social::mail::now(),
            expires_at: crate::game::social::mail::now() + 30 * 86_400,
            ..Default::default()
        },
    );
    let expiry = |realm: &Arc<Realm>| {
        realm
            .social
            .mail
            .lock()
            .mails
            .get(&id)
            .map(|m| m.expires_at)
    };
    let before = expiry(&realm).unwrap();
    on_map(&realm, move |m| {
        crate::game::social::mail::mark_read(m, g, id)
    })
    .await;
    let after = expiry(&realm).unwrap();
    let now = crate::game::social::mail::now();
    assert!(before > now + 20 * 86_400, "thirty days unread");
    assert!(
        after <= now + 3 * 86_400 + 1 && after > now + 3 * 86_400 - 5,
        "three read"
    );
}

fn aura(id: u32, aura: u32, points: i32) -> SpellTemplate {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura,
        base_points: points - 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    SpellTemplate {
        id,
        name: format!("Spell {id}"),
        effects,
        equipped_item_class: -1,
        ..Default::default()
    }
}

#[tokio::test]
async fn swimming_takes_the_strongest_bonus_and_slows() {
    let mut d = data();
    d.spells.spells.insert(9701, aura(9701, 58, 100));
    d.spells.spells.insert(9702, aura(9702, 58, 50));
    d.spells.spells.insert(9703, aura(9703, 33, -50));
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let swim = |spells: &'static [u32]| {
        let realm = realm.clone();
        async move {
            on_map(&realm, move |m| {
                for s in spells {
                    crate::game::map::spells::cast_triggered(m, g, *s, Some(g));
                }
                crate::game::map::auras::recalc(m, g);
                (m.players[&g].speeds.swim, m.players[&g].speeds.swim_back)
            })
            .await
        }
    };
    let (both, back) = swim(&[9701, 9702]).await;
    assert!(
        (both - 9.444_444).abs() < 0.01,
        "the strongest, not the sum: {both}"
    );
    assert!((back - 2.5).abs() < 0.01);
    let (slowed, _) = swim(&[9703]).await;
    assert!(
        (slowed - 4.722_222).abs() < 0.01,
        "a slow halves it: {slowed}"
    );
}

const FREEZE: u32 = 9454;
const ARCANE_CHANNEL: u32 = 9710;
const WATER_AURA: u32 = 9711;
const SPRINT: u32 = 9712;
const AUCTIONEER: u32 = 4104;
const SWORD: u32 = 9713;

fn more_data() -> GameData {
    let mut d = services();
    d.spells.spells.insert(FREEZE, aura(FREEZE, 29, 1));
    let mut channel = aura(ARCANE_CHANNEL, 29, 1);
    channel.attributes[1] = 0x4;
    channel.channel_interrupt_flags = 0x20;
    channel.duration_index = 0;
    d.spells.spells.insert(ARCANE_CHANNEL, channel);
    let mut water = aura(WATER_AURA, 29, 1);
    water.aura_interrupt_flags = 0x80;
    d.spells.spells.insert(WATER_AURA, water);
    d.spells.spells.insert(SPRINT, aura(SPRINT, 31, 50));
    d.creature_templates
        .insert(AUCTIONEER, npc(AUCTIONEER, 0x1000));
    d.items.insert(
        SWORD,
        ItemTemplate {
            entry: SWORD,
            name: "Sword".into(),
            class: 2,
            inventory_type: 13,
            allowable_class: -1,
            allowable_race: -1,
            stackable: 1,
            ..Default::default()
        },
    );
    d.creature_spawns.get_mut(&0).unwrap().push(CreatureSpawn {
        guid: 54,
        entries: vec![AUCTIONEER],
        map: 0,
        x: START.0 + 2.0,
        y: START.1 - 1.0,
        z: START.2,
        respawn_min: 600,
        respawn_max: 600,
        ..Default::default()
    });
    d
}

#[tokio::test]
async fn a_gm_frozen_player_cant_log_out() {
    let realm = players_realm(more_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    on_map(&realm, move |m| {
        crate::game::map::spells::cast_triggered(m, g, FREEZE, Some(g))
    })
    .await;
    assert!(on_map(&realm, move |m| m.players[&g].auras.has_spell(FREEZE)).await);
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    assert_eq!(
        logout_result(&mut a).await.0,
        LogoutResult::FailureFrozenByGm
    );
}

#[tokio::test]
async fn a_dance_ends_a_channel_and_a_swim_ends_what_water_does() {
    let realm = harness::realm_with(more_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let channelling = on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().spells.push(ARCANE_CHANNEL);
        crate::game::map::spells::cast(m, g, ARCANE_CHANNEL, Default::default(), None);
        m.players[&g]
            .caster
            .casting
            .as_ref()
            .is_some_and(|c| c.channel_until.is_some())
    })
    .await;
    assert!(channelling);
    a.send(CMSG_TEXT_EMOTE {
        text_emote: TextEmote::Dance,
        emote: 0,
        target: Guid::zero(),
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        on_map(&realm, move |m| m.players[&g].caster.casting.is_none()).await,
        "the dance ends it"
    );
    // Into the water.
    on_map(&realm, move |m| {
        crate::game::map::spells::cast_triggered(m, g, WATER_AURA, Some(g))
    })
    .await;
    a.send(MSG_MOVE_START_SWIM_Client {
        info: movement(
            MovementInfo_MovementFlags::empty()
                .set_swimming(MovementInfo_MovementFlags_Swimming { pitch: 0.0 }),
            START.2,
            0,
        ),
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!on_map(&realm, move |m| m.players[&g].auras.has_spell(WATER_AURA)).await);
}

#[tokio::test]
async fn a_speed_change_reaches_those_who_see_the_player() {
    let realm = harness::realm_with(more_data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(ga).await;
    b.clear();
    on_map(&realm, move |m| {
        crate::game::map::spells::cast_triggered(m, ga, SPRINT, Some(ga));
        crate::game::map::auras::recalc(m, ga);
    })
    .await;
    let frame = b
        .wait_frame(
            "MSG_MOVE_SET_RUN_SPEED",
            crate::game::map::misc::MSG_MOVE_SET_RUN_SPEED,
            |_| true,
        )
        .await;
    let speed = f32::from_le_bytes(frame[frame.len() - 4..].try_into().unwrap());
    assert!((speed - 10.5).abs() < 0.01, "{speed}");
}

#[tokio::test]
async fn the_dead_list_no_auctions() {
    let realm = harness::realm_with(more_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let auctioneer = crate::game::guid::unit(AUCTIONEER, 54);
    a.sees(auctioneer).await;
    let list = || CMSG_AUCTION_LIST_ITEMS {
        auctioneer,
        list_start_item: 0,
        searched_name: String::new(),
        minimum_level: 0,
        maximum_level: 0,
        auction_slot_id: u32::MAX,
        auction_main_category: u32::MAX,
        auction_sub_category: u32::MAX,
        auction_quality: ItemQuality::Poor,
        usable: 0,
    };
    a.send(list()).await;
    a.wait_frame("SMSG_AUCTION_LIST_RESULT", 0x25C, |_| true)
        .await;
    on_map(&realm, move |m| crate::game::map::death::kill(m, g)).await;
    a.clear();
    a.send(list()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    a.clear();
    assert!(
        a.gets(NONE_WITHIN, |m| {
            matches!(m, ServerOpcodeMessage::SMSG_AUCTION_LIST_RESULT(_)).then_some(())
        })
        .await
        .is_none()
    );
}

#[tokio::test]
async fn a_login_tells_temporary_enchantments_time() {
    let realm = harness::realm_with(more_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let item = on_map(&realm, move |m| {
        crate::game::map::inventory::add_item(m, g, SWORD, 1, false).unwrap();
        let item = m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == SWORD)
            .unwrap()
            .guid;
        crate::game::map::enchant::enchant_item(m, g, item, 7, Some((600_000, 0)));
        item
    })
    .await;
    logged_out(&mut a).await;
    a.clear();
    a.login(g).await;
    let left = a
        .wait_for("SMSG_ITEM_ENCHANT_TIME_UPDATE", |m| match m {
            ServerOpcodeMessage::SMSG_ITEM_ENCHANT_TIME_UPDATE(t) if t.item == item => {
                Some(t.duration)
            }
            _ => None,
        })
        .await;
    assert!((590..=600).contains(&left), "{left}");
}

#[tokio::test]
async fn a_copy_holds_as_many_as_its_map_template_says() {
    let mut d = more_data();
    d.maps.insert(
        36,
        MapInfo {
            id: 36,
            internal_name: "DeadminesInstance".into(),
            name: "Deadmines".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    d.scripts.map_templates.insert(
        36,
        crate::game::data::scripts::MapTemplate {
            map: 36,
            map_type: 1,
            player_limit: 1,
            ..Default::default()
        },
    );
    let realm = players_realm(d);
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
    let inside = Vector3d {
        x: -16.4,
        y: -383.07,
        z: 61.78,
    };
    for g in [ga, gb] {
        map0(&realm).call(move |m| m.teleport(g, MapKey::world(36), inside, 0.0));
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    let reason = b
        .wait_for("SMSG_TRANSFER_ABORTED", |m| match m {
            ServerOpcodeMessage::SMSG_TRANSFER_ABORTED(t) => Some(t.reason),
            _ => None,
        })
        .await;
    assert_eq!(reason, TransferAbortReason::IsFull, "one to the copy");
}

/// A new character knows their race's languages as skills (Language:
/// Common's 98 from `SkillLineAbility`, as Thistle Tea gives it), so the
/// client doesn't garble its own player's speech.
#[tokio::test]
#[ignore = "extended"]
async fn a_new_character_has_their_languages_as_skills() {
    let db = &crate::game::harness::real_data("db");
    if !db.join("vmangos.sqlite").exists() {
        return;
    }
    let realm = harness::realm_with(crate::game::harness::load_real(db));
    let mut a = Client::connect(&realm, "TONGUES");
    let human = a.enter("Tonguesa", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "TONGUESB");
    let orc = b.enter("Tonguesb", Race::Orc, Class::Warrior).await;
    let skills = |m: &Map, g: Guid| {
        let p = &m.players[&g];
        [98, 109].map(|s| {
            (
                crate::game::map::skills::value(p, s),
                crate::game::map::skills::max_value(p, s),
            )
        })
    };
    let h = map0(&realm).query(move |m| skills(m, human)).await.unwrap();
    let o = realm
        .maps
        .get(&realm, MapKey::world(1))
        .query(move |m| skills(m, orc))
        .await
        .unwrap();
    assert_eq!(
        h,
        [(Some(300), Some(300)), (None, None)],
        "a human speaks Common"
    );
    assert_eq!(
        o,
        [(None, None), (Some(300), Some(300))],
        "an orc speaks Orcish"
    );
}
