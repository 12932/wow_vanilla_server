//! Round 6's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/` (docs/parity/matrix.csv): `/who`, the
//! weather and world states as a player moves, emotes and groups.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::harness::{self, Client};
use crate::game::map::MapKey;
use crate::game::tests_p3_combat::{BRUTE, START, data};
use crate::game::tests_p3_entity::{NONE_WITHIN, on};
use crate::game::tests_p3_player2_ammo::handled;

/// A realm of `d` whose players aren't GMs.
pub(super) fn players_realm(d: crate::game::data::GameData) -> std::sync::Arc<crate::game::Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    crate::game::Realm::new(cfg, store, startup, d, None)
}

/// `/who` with no filter: the names, guilds and zones listed.
async fn who(a: &mut Client) -> Vec<(String, String, u32)> {
    a.clear();
    a.send(CMSG_WHO {
        minimum_level: Level::new(0),
        maximum_level: Level::new(100),
        player_name: String::new(),
        guild_name: String::new(),
        race_mask: u32::MAX,
        class_mask: u32::MAX,
        zones: Vec::new(),
        search_strings: Vec::new(),
    })
    .await;
    let b = a.wait_frame("SMSG_WHO", 0x063, |_| true).await;
    let listed = u32::from_le_bytes(b[0..4].try_into().unwrap());
    let mut at = 8;
    let cstr = |at: &mut usize| {
        let end = *at + b[*at..].iter().position(|c| *c == 0).unwrap();
        let s = String::from_utf8_lossy(&b[*at..end]).into_owned();
        *at = end + 1;
        s
    };
    let mut rows = Vec::new();
    for _ in 0..listed {
        let name = cstr(&mut at);
        let guild = cstr(&mut at);
        // Level, class and race, then the zone.
        let zone = u32::from_le_bytes(b[at + 12..at + 16].try_into().unwrap());
        at += 16;
        rows.push((name, guild, zone));
    }
    rows.sort();
    rows
}

/// Thistle Tea's `WhoList.send` "lists online players of the asker's team
/// in their zone, with their guild": the asker's side alone (mangos's
/// `HandleWhoOpcode`, to a player), each with their guild's name and
/// their zone.
#[tokio::test]
async fn who_lists_the_askers_side_with_their_guild_and_zone() {
    let realm = players_realm(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    c.enter("Grom", Race::Orc, Class::Warrior).await;
    for x in [&a, &b, &c] {
        x.send(CMSG_ZONEUPDATE {
            area: Area::ElwynnForest,
        })
        .await;
    }
    crate::game::social::guilds::found(&realm, "Knights", b.guid, &[]).expect("founded");
    handled(&mut b).await;
    handled(&mut c).await;
    assert_eq!(
        who(&mut a).await,
        [
            ("Alice".to_owned(), String::new(), 12),
            ("Bob".to_owned(), "Knights".to_owned(), 12),
        ],
        "the horde's Grom isn't listed"
    );
    assert_eq!(
        who(&mut c).await,
        [("Grom".to_owned(), String::new(), 12)],
        "nor to him the alliance"
    );
}

/// The weather `c` is told next, if any within [`NONE_WITHIN`].
async fn weather(c: &mut Client) -> Option<WeatherType> {
    c.gets(NONE_WITHIN, |m| match m {
        ServerOpcodeMessage::SMSG_WEATHER(w) => Some(w.weather_type),
        _ => None,
    })
    .await
}

/// Rain in Elwynn Forest (zone 12), told to those in it.
fn rain(realm: &crate::game::Realm) {
    {
        let mut all = realm.social.weather.lock();
        let w = all.entry(12).or_default();
        w.kind = 1;
        w.grade = 0.9;
    }
    for o in realm.registry.all_online() {
        if o.zone() == 12 {
            crate::game::weather::tell(realm, &o, 12);
        }
    }
}

/// Thistle Tea's `Weather.refresh` "clears weather on zone exit and rejects
/// old subscriptions after reentry" and "unsubscribes on departure":
/// leaving a zone with weather, the client is told fine weather (mangos's
/// `UpdateZone`, `SendFineWeatherUpdateToPlayer`); the zone's changes are
/// then not theirs, and back in it they are told its weather again. One
/// logged out to the character screen hears no more.
#[tokio::test]
async fn the_weather_clears_on_leaving_its_zone_and_the_old_zone_is_not_heard() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.send(CMSG_ZONEUPDATE {
        area: Area::ElwynnForest,
    })
    .await;
    handled(&mut a).await;
    a.clear();
    rain(&realm);
    assert_eq!(weather(&mut a).await, Some(WeatherType::Rain));
    a.send(CMSG_ZONEUPDATE {
        area: Area::Westfall0,
    })
    .await;
    assert_eq!(weather(&mut a).await, Some(WeatherType::Fine), "cleared");
    rain(&realm);
    assert_eq!(weather(&mut a).await, None, "Elwynn's isn't hers now");
    a.send(CMSG_ZONEUPDATE {
        area: Area::ElwynnForest,
    })
    .await;
    assert_eq!(weather(&mut a).await, Some(WeatherType::Rain), "back");
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.clear();
    rain(&realm);
    assert_eq!(weather(&mut a).await, None, "logged out");
    let _ = g;
}

/// Thistle Tea's `WorldStates.build` "uses the destination map and zone":
/// across a worldport, the world states are the new map's and zone's.
#[tokio::test]
async fn world_states_after_a_worldport_name_the_new_map_and_zone() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let to = Vector3d {
        x: 1_629.0,
        y: -4_373.0,
        z: 31.0,
    };
    map.call(move |m| m.teleport(g, MapKey::world(1), to, 0.0));
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(g).await;
    a.clear();
    a.send(CMSG_ZONEUPDATE {
        area: Area::Durotar,
    })
    .await;
    let (at, area) = a
        .wait_for("SMSG_INIT_WORLD_STATES", |m| match m {
            ServerOpcodeMessage::SMSG_INIT_WORLD_STATES(w) => Some((w.map, w.area)),
            _ => None,
        })
        .await;
    assert_eq!((at, area), (Map::Kalimdor, Area::Durotar));
}

/// The emotes `b` is told of within [`NONE_WITHIN`]: the text emotes'
/// names, and the one-shot animations.
async fn emotes(b: &mut Client) -> (Vec<String>, Vec<Emote>) {
    let mut texts = Vec::new();
    let mut shots = Vec::new();
    while let Some(m) = b
        .gets(NONE_WITHIN, |m| match m {
            ServerOpcodeMessage::SMSG_TEXT_EMOTE(t) => Some(Ok(t.name.clone())),
            ServerOpcodeMessage::SMSG_EMOTE(e) => Some(Err(e.emote)),
            _ => None,
        })
        .await
    {
        match m {
            Ok(t) => texts.push(t),
            Err(e) => shots.push(e),
        }
    }
    (texts, shots)
}

/// Thistle Tea's `Emotes.text` "retains dancing for observers and emits
/// localized text without a one-shot", "names same-world targets" and
/// "ignores unknown text IDs": a dance is a state those near see, with
/// its text and no one-shot; a wave at a creature names it and plays its
/// one-shot; a text emote of an id the client's table hasn't is dropped
/// (mangos's `HandleTextEmoteOpcode`).
#[tokio::test]
async fn a_dance_is_a_state_a_wave_a_one_shot_and_an_unknown_emote_nothing() {
    let mut d = data();
    d.text_emote_anims.extend([(34, 10), (101, 3)]);
    d.emote_states.insert(10);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let (mut b, _gb) = crate::game::tests_p3_entity::bob(&realm, ga).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let brute = on(&map, move |m| {
        crate::game::tests_p3_combat::spawn(m, ga, BRUTE, 20, 4.0)
    })
    .await;
    b.clear();
    a.send(CMSG_TEXT_EMOTE {
        text_emote: TextEmote::Dance,
        emote: 0,
        target: Guid::zero(),
    })
    .await;
    assert_eq!(emotes(&mut b).await, (vec![String::new()], vec![]));
    let state = on(&map, move |m| {
        m.players[&ga]
            .fields
            .get_u32(crate::game::fields::UNIT_NPC_EMOTESTATE)
    })
    .await;
    assert_eq!(state, 10, "dancing");
    a.send(CMSG_TEXT_EMOTE {
        text_emote: TextEmote::Wave,
        emote: 0,
        target: brute,
    })
    .await;
    assert_eq!(
        emotes(&mut b).await,
        (vec!["Creature 3700".to_owned()], vec![Emote::OneshotWave])
    );
    // 9999 is no text emote: the packet is dropped.
    let mut body = 9_999u32.to_le_bytes().to_vec();
    body.extend(0u32.to_le_bytes());
    body.extend(0u64.to_le_bytes());
    a.send_raw(0x104, &body).await;
    handled(&mut a).await;
    assert_eq!(emotes(&mut b).await, (vec![], vec![]));
}

/// `leader` invites `member`, who accepts: a party.
async fn group(leader: &mut Client, member: &mut Client, name: &str) {
    leader.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    member
        .wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
    member.send(CMSG_GROUP_ACCEPT {}).await;
    member
        .wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) if !l.members.is_empty() => Some(()),
            _ => None,
        })
        .await;
}

/// The next group list `c` is told: its type, own flags and its members'
/// flags.
async fn group_list(c: &mut Client) -> (GroupType, u8, Vec<u8>) {
    c.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => Some((
            l.group_type,
            l.flags,
            l.members.iter().map(|m| m.flags).collect(),
        )),
        _ => None,
    })
    .await
}

/// Thistle Tea's `Groups.random_roll` "shares one server result from any
/// member across the whole raid" and `convert_raid` "projects conversion,
/// assistant flags, and subgroup changes to both members": a member's
/// roll is the same result to everyone in the raid, one in another map
/// and another subgroup too; only the leader makes the party a raid, and
/// the assistant's flag (0x80) and their subgroup reach both.
#[tokio::test]
async fn a_raid_is_the_leaders_and_its_rolls_and_flags_reach_every_member() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    group(&mut a, &mut b, "Bob").await;
    // Bob can't make it a raid; Alice can.
    b.send(CMSG_GROUP_RAID_CONVERT {}).await;
    handled(&mut b).await;
    assert!(
        realm
            .social
            .groups
            .lock()
            .group_of(ga)
            .is_some_and(|g| !g.raid)
    );
    a.clear();
    b.clear();
    a.send(CMSG_GROUP_RAID_CONVERT {}).await;
    a.wait_for("the conversion's result", |m| match m {
        ServerOpcodeMessage::SMSG_PARTY_COMMAND_RESULT(r) => {
            (r.result == PartyResult::Success).then_some(())
        }
        _ => None,
    })
    .await;
    for c in [&mut a, &mut b] {
        assert_eq!(group_list(c).await, (GroupType::Raid, 0, vec![0]));
    }
    a.send(CMSG_GROUP_ASSISTANT_LEADER {
        guid: gb,
        set_assistant: true,
    })
    .await;
    assert_eq!(group_list(&mut a).await, (GroupType::Raid, 0, vec![0x80]));
    assert_eq!(group_list(&mut b).await, (GroupType::Raid, 0x80, vec![0]));
    a.send(CMSG_GROUP_CHANGE_SUB_GROUP {
        name: "Bob".into(),
        group_number: 7,
    })
    .await;
    assert_eq!(group_list(&mut b).await, (GroupType::Raid, 0x87, vec![0]));
    // Bob in Kalimdor, Alice in Elwynn: his roll is hers too.
    let map = realm.maps.get(&realm, MapKey::world(0));
    let to = Vector3d {
        x: 1_629.0,
        y: -4_373.0,
        z: 31.0,
    };
    map.call(move |m| m.teleport(gb, MapKey::world(1), to, 0.0));
    b.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    b.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    b.sees(gb).await;
    a.clear();
    b.clear();
    b.send(MSG_RANDOM_ROLL_Client {
        minimum: 37,
        maximum: 91,
    })
    .await;
    let rolled = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::MSG_RANDOM_ROLL(r) => {
            Some((r.guid, r.minimum, r.maximum, r.actual_roll))
        }
        _ => None,
    };
    let theirs = b.wait_for("Bob's roll", rolled).await;
    let hers = a.wait_for("Bob's roll", rolled).await;
    assert_eq!(theirs, hers, "one result");
    assert_eq!((hers.0, hers.1, hers.2), (gb, 37, 91));
    assert!((37..=91).contains(&hers.3));
    assert_eq!(b.gets(NONE_WITHIN, rolled).await, None, "once");
}

/// Thistle Tea's `Groups.ready_check` "requires management to start,
/// includes offline members, and forwards answers to the leader": a member
/// can't start one; the leader's goes to everyone, the leader too
/// (mangos's `BroadcastPacket`), and the leader hears the one offline isn't
/// ready; an answer goes to the leader alone.
#[tokio::test]
async fn a_ready_check_is_the_leaders_and_its_answers_are_hers() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Cid", Race::Human, Class::Warrior).await;
    group(&mut a, &mut b, "Bob").await;
    group(&mut a, &mut c, "Cid").await;
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    let check = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::MSG_RAID_READY_CHECK(r) => {
            Some(r.state_check.as_ref().map(|s| (s.guid, s.state)))
        }
        _ => None,
    };
    a.clear();
    b.clear();
    b.send(MSG_RAID_READY_CHECK_Client { answer: None }).await;
    handled(&mut b).await;
    assert_eq!(a.gets(NONE_WITHIN, check).await, None, "not Bob's to start");
    assert_eq!(b.gets(Duration::from_millis(1), check).await, None);
    a.send(MSG_RAID_READY_CHECK_Client { answer: None }).await;
    assert_eq!(a.wait_for("her own check", check).await, None);
    assert_eq!(b.wait_for("the check", check).await, None);
    assert_eq!(
        a.wait_for("Cid not ready", check).await,
        Some((gc, 0)),
        "offline"
    );
    b.send(MSG_RAID_READY_CHECK_Client {
        answer: Some(MSG_RAID_READY_CHECK_Client_answer { state: 1 }),
    })
    .await;
    assert_eq!(a.wait_for("Bob's answer", check).await, Some((gb, 1)));
    assert_eq!(b.gets(NONE_WITHIN, check).await, None, "the leader's alone");
}

/// A creature with `npc_flags` beside `g`, `yards` along where they face,
/// of no faction: anyone may talk to it.
pub(super) fn npc_beside(
    m: &mut crate::game::map::Map,
    g: Guid,
    npc_flags: u32,
    yards: f32,
) -> Guid {
    let t = crate::game::data::CreatureTemplate {
        npc_flags,
        faction: 0,
        ..crate::game::tests_p3_combat::template(9_400, 7)
    };
    let p = &m.players[&g];
    let (at, o) = (p.position(), p.movement.orientation);
    let at = Vector3d {
        x: at.x + yards * o.cos(),
        y: at.y + yards * o.sin(),
        z: at.z,
    };
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let c = crate::game::map::creature::Creature::new(
        crate::game::guid::unit(9_400, counter | 0x80_0000),
        &t,
        20,
        at,
        o + std::f32::consts::PI,
    );
    m.add_creature(c.with_base())
}

/// The first word of the next `MSG_SAVE_GUILD_EMBLEM` `c` is told.
async fn emblem_result(c: &mut Client) -> u32 {
    let f = c.wait_frame("MSG_SAVE_GUILD_EMBLEM", 0x1F1, |_| true).await;
    u32::from_le_bytes(f[0..4].try_into().unwrap())
}

/// Thistle Tea's `Guilds.activate_tabard` "opens only at a nearby tabard
/// designer" and `save_emblem` "charges the leader and publishes the
/// updated guild" and "rejects nonleaders, invalid vendors, colors, and
/// insufficient money": the designer's window opens at one within reach
/// alone; an emblem is saved there alone (invalid vendor, 5, first, as
/// mangos's `HandleSaveGuildEmblemOpcode` checks it), by the leader (not
/// in a guild 2, not the leader 3) who has ten gold (4), which it costs;
/// and the leader is told the guild with its emblem.
#[tokio::test]
async fn an_emblem_is_the_leaders_at_a_tabard_designer_for_ten_gold() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    b.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (designer, other) = on(&map, move |m| {
        (
            npc_beside(m, ga, crate::game::map::npc::NPC_FLAG_TABARDDESIGNER, 2.0),
            npc_beside(m, ga, crate::game::map::npc::NPC_FLAG_VENDOR, 2.0),
        )
    })
    .await;
    a.sees(designer).await;
    let opens = async |a: &mut Client, vendor: Guid| {
        a.clear();
        a.send(MSG_TABARDVENDOR_ACTIVATE { guid: vendor }).await;
        handled(a).await;
        crate::game::tests_p3_entity::gets_frame(a, 0x1F2, |f| {
            f[..8] == vendor.guid().to_le_bytes()
        })
        .await
    };
    assert!(opens(&mut a, designer).await, "at the designer");
    assert!(!opens(&mut a, other).await, "not at another vendor");
    let emblem = |vendor| MSG_SAVE_GUILD_EMBLEM_Client {
        vendor,
        emblem_style: 3,
        emblem_color: 4,
        border_style: 5,
        border_color: 6,
        background_color: 7,
    };
    a.send(emblem(designer)).await;
    assert_eq!(emblem_result(&mut a).await, 2, "no guild");
    crate::game::social::guilds::found(&realm, "Knights", ga, &[gb]).expect("founded");
    on(&map, move |m| {
        for g in [ga, gb] {
            m.players.get_mut(&g).unwrap().set_money(100_000);
        }
    })
    .await;
    b.send(emblem(designer)).await;
    assert_eq!(emblem_result(&mut b).await, 3, "not the leader");
    a.send(emblem(other)).await;
    assert_eq!(emblem_result(&mut a).await, 5, "not a designer");
    on(&map, move |m| {
        let at = m.players[&ga].position();
        m.creature_mut(designer).unwrap().position = Vector3d {
            x: at.x + 40.0,
            ..at
        };
    })
    .await;
    a.send(emblem(designer)).await;
    assert_eq!(emblem_result(&mut a).await, 5, "a designer too far");
    assert!(!opens(&mut a, designer).await, "too far to open");
    on(&map, move |m| {
        let at = m.players[&ga].position();
        m.creature_mut(designer).unwrap().position = Vector3d {
            x: at.x + 2.0,
            ..at
        };
        m.players.get_mut(&ga).unwrap().set_money(99_999);
    })
    .await;
    a.send(emblem(designer)).await;
    assert_eq!(emblem_result(&mut a).await, 4, "short of ten gold");
    on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().set_money(100_000)
    })
    .await;
    a.clear();
    a.send(emblem(designer)).await;
    assert_eq!(emblem_result(&mut a).await, 0, "saved");
    let told = a
        .wait_for("SMSG_GUILD_QUERY_RESPONSE", |m| match m {
            ServerOpcodeMessage::SMSG_GUILD_QUERY_RESPONSE(q) => Some((
                q.emblem_style,
                q.emblem_color,
                q.border_style,
                q.border_color,
                q.background_color,
            )),
            _ => None,
        })
        .await;
    assert_eq!(told, (3, 4, 5, 6, 7));
    let money = on(&map, move |m| m.players[&ga].money()).await;
    assert_eq!(money, 0, "ten gold");
}

/// The charters `g` carries.
fn charters(m: &crate::game::map::Map, g: Guid) -> Vec<Guid> {
    m.players[&g]
        .items
        .iter()
        .filter(|i| i.entry == crate::game::social::guilds::CHARTER_ITEM)
        .map(|i| i.guid)
        .collect()
}

/// Thistle Tea's `Petitions.buy` "rejects remote purchase and insufficient
/// funds", `turn_in` "requires nine signatures and creates all guild
/// members" (wvs's `petition_signatures`, one in the tests) and `sign`
/// "refuses a signer with a pending guild invitation": a charter is sold
/// at a petitioner within reach alone, and one short of its price is told
/// so as a vendor tells it (mangos's `HandlePetitionBuyOpcode`); turned in
/// short of signatures it founds nothing; one invited to a guild may not
/// sign (mangos's `HandlePetitionSignOpcode`); with enough, the guild is
/// founded with its signers in it, the founder leading.
#[tokio::test]
async fn a_charter_is_bought_near_signed_by_the_uninvited_and_founds_with_enough() {
    let mut d = data();
    d.items.insert(
        crate::game::social::guilds::CHARTER_ITEM,
        crate::game::data::ItemTemplate {
            entry: crate::game::social::guilds::CHARTER_ITEM,
            name: "Guild Charter".into(),
            stackable: 1,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Cid", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let npc = on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().set_money(999);
        npc_beside(m, ga, crate::game::map::npc::NPC_FLAG_PETITIONER, 2.0)
    })
    .await;
    a.sees(npc).await;
    let buy = CMSG_PETITION_BUY {
        npc,
        name: "Grunts".into(),
        index: 1,
        ..Default::default()
    };
    a.clear();
    a.send(buy.clone()).await;
    let failed = a
        .wait_for("SMSG_BUY_FAILED", |m| match m {
            ServerOpcodeMessage::SMSG_BUY_FAILED(f) => Some((f.guid, f.item, f.result)),
            _ => None,
        })
        .await;
    assert_eq!(
        failed,
        (
            npc,
            crate::game::social::guilds::CHARTER_ITEM,
            BuyResult::NotEnoughtMoney
        )
    );
    on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().set_money(5_000);
        let at = m.players[&ga].position();
        m.creature_mut(npc).unwrap().position = Vector3d {
            x: at.x + 40.0,
            ..at
        };
    })
    .await;
    a.send(buy.clone()).await;
    handled(&mut a).await;
    assert!(
        on(&map, move |m| charters(m, ga)).await.is_empty(),
        "too far"
    );
    on(&map, move |m| {
        let at = m.players[&ga].position();
        m.creature_mut(npc).unwrap().position = Vector3d {
            x: at.x + 2.0,
            ..at
        };
    })
    .await;
    a.send(buy).await;
    handled(&mut a).await;
    let charter = on(&map, move |m| charters(m, ga)).await[0];
    let turned = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::SMSG_TURN_IN_PETITION_RESULTS(r) => Some(r.result),
        _ => None,
    };
    a.send(CMSG_TURN_IN_PETITION { petition: charter }).await;
    assert_eq!(
        a.wait_for("NeedMore", turned).await,
        PetitionResult::NeedMore
    );
    // Cid's guild invites Bob: he can't sign while it stands.
    crate::game::social::guilds::found(&realm, "Others", gc, &[]).expect("founded");
    c.send(CMSG_GUILD_INVITE {
        invited_player: "Bob".into(),
    })
    .await;
    b.wait_for("SMSG_GUILD_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GUILD_INVITE(_)).then_some(())
    })
    .await;
    b.clear();
    b.send(CMSG_PETITION_SIGN {
        petition: charter,
        unknown1: 0,
    })
    .await;
    let refused = b
        .wait_for("SMSG_GUILD_COMMAND_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_GUILD_COMMAND_RESULT(r) => {
                Some((r.command, r.string.clone(), r.result))
            }
            _ => None,
        })
        .await;
    assert_eq!(
        refused,
        (
            GuildCommand::Invite,
            "Bob".to_owned(),
            GuildCommandResult::AlreadyInvitedToGuildS
        )
    );
    let signed = |realm: &crate::game::Realm| {
        realm
            .social
            .guilds
            .lock()
            .petitions
            .get(&crate::game::guid::counter(charter))
            .map(|p| p.signers.len())
    };
    assert_eq!(signed(&realm), Some(0), "not signed");
    b.send(CMSG_GUILD_DECLINE {}).await;
    handled(&mut b).await;
    b.send(CMSG_PETITION_SIGN {
        petition: charter,
        unknown1: 0,
    })
    .await;
    b.wait_for("signed", |m| match m {
        ServerOpcodeMessage::SMSG_PETITION_SIGN_RESULTS(r) => {
            (r.result == PetitionResult::Ok).then_some(())
        }
        _ => None,
    })
    .await;
    a.send(CMSG_TURN_IN_PETITION { petition: charter }).await;
    assert_eq!(a.wait_for("founded", turned).await, PetitionResult::Ok);
    let (leader, members, lowest) = {
        let guilds = realm.social.guilds.lock();
        let g = guilds.of(ga).expect("Alice's guild");
        (
            g.leader,
            g.members
                .iter()
                .map(|m| (m.guid, m.rank))
                .collect::<Vec<_>>(),
            g.ranks.len() as u32 - 1,
        )
    };
    assert_eq!(leader, crate::game::guid::counter(ga));
    assert_eq!(lowest, 4, "of five ranks");
    assert!(members.contains(&(crate::game::guid::counter(ga), 0)));
    assert!(members.contains(&(crate::game::guid::counter(gb), lowest)));
    assert!(on(&map, move |m| charters(m, ga)).await.is_empty(), "taken");
}

/// A realm of `d` whose logouts take their twenty seconds.
fn slow_logouts(d: crate::game::data::GameData) -> std::sync::Arc<crate::game::Realm> {
    let mut cfg = harness::test_config();
    cfg.logout_delay = crate::game::map::login::LOGOUT_DELAY;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    crate::game::Realm::new(cfg, store, startup, d, None)
}

/// Whether `c` is told it is unrooted within [`NONE_WITHIN`].
async fn unrooted(c: &mut Client) -> bool {
    c.gets(NONE_WITHIN, |m| {
        matches!(m, ServerOpcodeMessage::SMSG_FORCE_MOVE_UNROOT(_)).then_some(())
    })
    .await
    .is_some()
}

/// Thistle Tea's `Logout.cancel` "does not release the death root": one
/// who dies during a logout's countdown, and cancels it, lies rooted still
/// (mangos's `HandleLogoutCancelOpcode` frees only one who can move); the
/// living are freed.
#[tokio::test]
async fn a_cancelled_logout_frees_the_living_but_not_the_dead() {
    let realm = slow_logouts(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let logout = async |a: &mut Client| {
        a.send(CMSG_LOGOUT_REQUEST {}).await;
        a.wait_for("SMSG_FORCE_MOVE_ROOT", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_FORCE_MOVE_ROOT(_)).then_some(())
        })
        .await;
    };
    let cancel = async |a: &mut Client| {
        a.clear();
        a.send(CMSG_LOGOUT_CANCEL {}).await;
        a.wait_for("SMSG_LOGOUT_CANCEL_ACK", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_CANCEL_ACK).then_some(())
        })
        .await;
    };
    logout(&mut a).await;
    cancel(&mut a).await;
    assert!(unrooted(&mut a).await, "the living walk again");
    logout(&mut a).await;
    on(&map, move |m| crate::game::map::death::kill(m, ga)).await;
    cancel(&mut a).await;
    assert!(!unrooted(&mut a).await, "the dead lie where they fell");
    let still = on(&map, move |m| {
        let p = &m.players[&ga];
        (
            p.fields
                .get_byte(crate::game::fields::PLAYER_FIELD_BYTES, 0)
                & 0x04,
            p.logout_at.is_some(),
        )
    })
    .await;
    assert_eq!(still, (0, false), "no longer logging out");
}

/// Thistle Tea's `Characters.delete` "refuses another account's character
/// and a guild leader": a guild's leader, alone in it or not, can't be
/// deleted (mangos's `HandleCharDeleteOpcode`,
/// `CHAR_DELETE_FAILED_GUILD_LEADER`), nor can another account delete a
/// character.
#[tokio::test]
async fn a_guild_leader_and_another_accounts_character_cant_be_deleted() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    crate::game::social::guilds::found(&realm, "Alone", ga, &[]).expect("founded");
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    let delete = async |c: &mut Client| {
        c.send(CMSG_CHAR_DELETE { guid: ga }).await;
        c.wait_for("SMSG_CHAR_DELETE", |m| match m {
            ServerOpcodeMessage::SMSG_CHAR_DELETE(d) => Some(d.result),
            _ => None,
        })
        .await
    };
    let mut other = Client::connect(&realm, "O");
    assert_eq!(delete(&mut other).await, WorldResult::CharDeleteFailed);
    assert_eq!(delete(&mut a).await, WorldResult::CharDeleteFailed);
    assert!(realm.registry.name(ga).is_some(), "still there");
}

/// Stormwind (72, index 19) and the Argent Dawn (529, index 13), neutral to
/// a human; a mount trainer for dwarves and a vendor, both of Stormwind.
const STORMWIND: u32 = 72;
const ARGENT_DAWN: u32 = 529;
const OF_STORMWIND: u32 = 1_200;
const RAM_TRAINER: u32 = 9_401;
const QUARTERMASTER: u32 = 9_402;
const RIDING: u32 = 826;
/// For friends of the Argent Dawn; for those revered by the vendor's
/// faction; for anyone.
const DAWN_ITEM: u32 = 63_101;
const REVERED_ITEM: u32 = 63_102;
const ANY_ITEM: u32 = 63_103;

fn standing_world() -> crate::game::data::GameData {
    use crate::game::data::misc::FactionDef;
    let mut d = data();
    for (id, index) in [(STORMWIND, 19), (ARGENT_DAWN, 13)] {
        d.misc.factions.insert(
            id,
            FactionDef {
                id,
                index,
                name: format!("Faction {id}"),
                variants: [(1, 0, 0, 0x11), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)],
                ..Default::default()
            },
        );
        d.misc.faction_by_index.insert(index, id);
    }
    d.faction_templates.insert(
        OF_STORMWIND,
        crate::game::data::FactionTemplate {
            faction: STORMWIND,
            ..crate::game::tests_p3_combat::faction(OF_STORMWIND, 0x1, 0x2)
        },
    );
    let npc = |entry, npc_flags| crate::game::data::CreatureTemplate {
        faction: OF_STORMWIND,
        npc_flags,
        ..crate::game::tests_p3_combat::template(entry, 7)
    };
    let mut trainer = npc(RAM_TRAINER, crate::game::map::npc::NPC_FLAG_TRAINER);
    trainer.trainer_type = 1;
    trainer.trainer_race = 3;
    d.creature_templates.insert(RAM_TRAINER, trainer);
    d.npc.trainers.insert(
        RAM_TRAINER,
        vec![crate::game::data::npc::TrainerSpell {
            spell: RIDING,
            cost: 10,
            required_level: 1,
            ..Default::default()
        }],
    );
    d.creature_templates.insert(
        QUARTERMASTER,
        npc(QUARTERMASTER, crate::game::map::npc::NPC_FLAG_VENDOR),
    );
    for (entry, faction, rank) in [
        (DAWN_ITEM, ARGENT_DAWN, 4),
        (REVERED_ITEM, 0, 6),
        (ANY_ITEM, 0, 0),
    ] {
        d.items.insert(
            entry,
            crate::game::data::ItemTemplate {
                entry,
                name: format!("Item {entry}"),
                stackable: 1,
                buy_count: 1,
                buy_price: 25,
                allowable_class: -1,
                allowable_race: -1,
                required_reputation_faction: faction,
                required_reputation_rank: rank,
                ..Default::default()
            },
        );
    }
    d.vendors.insert(
        QUARTERMASTER,
        [DAWN_ITEM, REVERED_ITEM, ANY_ITEM]
            .into_iter()
            .enumerate()
            .map(|(slot, item)| crate::game::data::VendorItem {
                slot: slot as u32,
                item,
                ..Default::default()
            })
            .collect(),
    );
    d
}

/// Whether `a` is shown `trainer`'s list.
async fn trains(a: &mut Client, trainer: Guid) -> bool {
    a.clear();
    a.send(CMSG_TRAINER_LIST { guid: trainer }).await;
    handled(a).await;
    a.gets(Duration::from_millis(1), |m| {
        matches!(m, ServerOpcodeMessage::SMSG_TRAINER_LIST(_)).then_some(())
    })
    .await
    .is_some()
}

/// Thistle Tea's `Reputation` "blocks unfriendly interaction and unlocks
/// cross-race trainers at exalted": one Unfriendly with a creature's
/// faction can't talk to it (mangos's `CanInteract`, both reactions above
/// unfriendly); a mount trainer of another race teaches one exalted with
/// its faction (`IsTrainerOf`), not one neutral.
#[tokio::test]
async fn the_unfriendly_are_refused_and_the_exalted_learn_another_races_mounts() {
    let realm = harness::realm_with(standing_world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (trainer, vendor) = on(&map, move |m| {
        (
            crate::game::tests_p3_combat::spawn(m, ga, RAM_TRAINER, 20, 2.0),
            crate::game::tests_p3_combat::spawn(m, ga, QUARTERMASTER, 20, 2.0),
        )
    })
    .await;
    a.sees(vendor).await;
    let listed = async |a: &mut Client| {
        a.clear();
        a.send(CMSG_LIST_INVENTORY { guid: vendor }).await;
        handled(a).await;
        a.gets(Duration::from_millis(1), |m| match m {
            ServerOpcodeMessage::SMSG_LIST_INVENTORY(l) => {
                Some(l.items.iter().map(|i| i.item).collect::<Vec<_>>())
            }
            _ => None,
        })
        .await
    };
    assert!(
        !trains(&mut a, trainer).await,
        "a dwarves' trainer, neutral"
    );
    assert!(listed(&mut a).await.is_some(), "neutral, served");
    on(&map, move |m| {
        crate::game::map::reputation::modify(m, ga, STORMWIND, -3_000, false)
    })
    .await;
    assert_eq!(listed(&mut a).await, None, "unfriendly, refused");
    on(&map, move |m| {
        crate::game::map::reputation::modify(m, ga, STORMWIND, 45_000, false)
    })
    .await;
    assert!(trains(&mut a, trainer).await, "exalted");
}

/// Thistle Tea's `Reputation.vendor_items` "shows explicit requirements,
/// hides unmet vendor requirements, and prices the result" and
/// `item_requirement_met?` "checks explicit item factions": an item for a
/// standing with a named faction is listed, and refused to one without it
/// (mangos's `BuyItemFromVendorSlot`); one for a rank with no faction is
/// the vendor's faction's, and hidden from one short of it (mangos's
/// `SendListInventory`); the honored pay a tenth less.
#[tokio::test]
async fn a_vendor_lists_named_standings_hides_its_own_unmet_and_discounts_the_honored() {
    let realm = harness::realm_with(standing_world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let vendor = on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().set_money(1_000);
        crate::game::map::reputation::modify(m, ga, STORMWIND, 9_000, false);
        crate::game::tests_p3_combat::spawn(m, ga, QUARTERMASTER, 20, 2.0)
    })
    .await;
    a.sees(vendor).await;
    a.clear();
    a.send(CMSG_LIST_INVENTORY { guid: vendor }).await;
    let items = a
        .wait_for("SMSG_LIST_INVENTORY", |m| match m {
            ServerOpcodeMessage::SMSG_LIST_INVENTORY(l) => Some(
                l.items
                    .iter()
                    .map(|i| (i.item_stack_count, i.item, i.price.as_int()))
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .await;
    assert_eq!(items, [(1, DAWN_ITEM, 23), (3, ANY_ITEM, 23)]);
    a.send(CMSG_BUY_ITEM {
        vendor,
        item: DAWN_ITEM,
        amount: 1,
        unknown1: 0,
    })
    .await;
    let refused = a
        .wait_for("SMSG_BUY_FAILED", |m| match m {
            ServerOpcodeMessage::SMSG_BUY_FAILED(f) => Some(f.result),
            _ => None,
        })
        .await;
    assert_eq!(refused, BuyResult::ReputationRequire);
    on(&map, move |m| {
        crate::game::map::reputation::modify(m, ga, ARGENT_DAWN, 3_000, false)
    })
    .await;
    a.send(CMSG_BUY_ITEM {
        vendor,
        item: DAWN_ITEM,
        amount: 1,
        unknown1: 0,
    })
    .await;
    a.wait_for("SMSG_BUY_ITEM", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_BUY_ITEM(_)).then_some(())
    })
    .await;
    let (money, has) = on(&map, move |m| {
        (m.players[&ga].money(), m.players[&ga].count_of(DAWN_ITEM))
    })
    .await;
    assert_eq!((money, has), (977, 1), "a tenth off 25, rounded");
}

/// Thistle Tea's `Summoning.request` "Evil Twin silently blocks incoming
/// summons until its aura expires": under Evil Twin (23445) no summons is
/// offered, and none is held to be accepted (mangos's
/// `EffectSummonPlayer`); without it, the summons is offered.
#[tokio::test]
async fn evil_twin_turns_summonses_away_unheard() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let twin = crate::game::map::misc::EVIL_TWIN;
    a.clear();
    let held = on(&map, move |m| {
        crate::game::tests_p3_combat::aura_of(m, ga, twin, 4, 0, 0);
        crate::game::map::misc::offer_summon(m, gb, ga);
        m.players[&ga].auras.has_spell(twin)
    })
    .await;
    assert!(held, "under Evil Twin");
    let offered = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::SMSG_SUMMON_REQUEST(s) => Some(s.summoner),
        _ => None,
    };
    handled(&mut a).await;
    assert_eq!(a.gets(NONE_WITHIN, offered).await, None, "unheard");
    let pending = on(&map, move |m| m.players[&ga].misc.summon.is_some()).await;
    assert!(!pending, "nothing to accept");
    on(&map, move |m| {
        crate::game::map::auras::remove_spell(m, ga, twin, None);
        crate::game::map::misc::offer_summon(m, gb, ga);
    })
    .await;
    assert_eq!(a.wait_for("the summons", offered).await, gb);
}

/// The house `a` is told `npc` is of, if it is told at all.
async fn house(a: &mut Client, npc: Guid) -> Option<u32> {
    a.clear();
    a.send(MSG_AUCTION_HELLO_Client { auctioneer: npc }).await;
    handled(a).await;
    let f = tokio::time::timeout(
        Duration::from_millis(50),
        a.wait_frame("MSG_AUCTION_HELLO", 0x255, |f| {
            f[..8] == npc.guid().to_le_bytes()
        }),
    )
    .await
    .ok()?;
    Some(u32::from_le_bytes(f[8..12].try_into().unwrap()))
}

/// Thistle Tea's `Auction.house` "opens a nearby live vanilla auctioneer
/// and resolves its market" and "rejects dead, ghost, remote, unflagged,
/// and absent auctioneers": an auctioneer of Stormwind's faction (12) is
/// of house 1, one of Booty Bay's (120) the goblins' 7 (mangos's
/// `GetAuctionHouseEntry`); none answers the dead or a ghost, from past its
/// reach, dead, unflagged or not there.
#[tokio::test]
async fn an_auctioneer_names_its_house_to_the_living_within_reach() {
    let mut d = data();
    for (id, ours) in [(12, 0x1), (120, 0)] {
        d.faction_templates
            .insert(id, crate::game::tests_p3_combat::faction(id, ours, 0));
    }
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let npc = on(&map, move |m| {
        let c = npc_beside(m, ga, crate::game::map::npc::NPC_FLAG_AUCTIONEER, 2.0);
        m.creature_mut(c)
            .unwrap()
            .fields
            .set_u32(crate::game::fields::UNIT_FIELD_FACTIONTEMPLATE, 12);
        c
    })
    .await;
    a.sees(npc).await;
    assert_eq!(house(&mut a, npc).await, Some(1), "Stormwind's");
    let set = |f: fn(&mut crate::game::map::Map, Guid, Guid)| {
        let map = map.clone();
        async move { on(&map, move |m| f(m, ga, npc)).await }
    };
    set(|m, _, npc| {
        m.creature_mut(npc)
            .unwrap()
            .fields
            .set_u32(crate::game::fields::UNIT_FIELD_FACTIONTEMPLATE, 120)
    })
    .await;
    assert_eq!(house(&mut a, npc).await, Some(7), "Booty Bay's");
    set(|m, ga, npc| {
        let at = m.players[&ga].position();
        m.creature_mut(npc).unwrap().position = Vector3d {
            x: at.x + 7.0,
            ..at
        };
    })
    .await;
    assert_eq!(house(&mut a, npc).await, None, "past reach");
    set(|m, ga, npc| {
        let at = m.players[&ga].position();
        m.creature_mut(npc).unwrap().position = Vector3d {
            x: at.x + 2.0,
            ..at
        };
        m.creature_mut(npc)
            .unwrap()
            .fields
            .set_u32(crate::game::fields::UNIT_NPC_FLAGS, 0);
    })
    .await;
    assert_eq!(house(&mut a, npc).await, None, "unflagged");
    set(|m, _, npc| {
        let later = m.now + Duration::from_secs(60);
        let c = m.creature_mut(npc).unwrap();
        c.fields.set_u32(
            crate::game::fields::UNIT_NPC_FLAGS,
            crate::game::map::npc::NPC_FLAG_AUCTIONEER,
        );
        c.life = crate::game::map::creature::Life::Dead {
            corpse_until: later,
            respawn_at: later,
        };
    })
    .await;
    assert_eq!(house(&mut a, npc).await, None, "a dead auctioneer");
    set(|m, _, npc| m.creature_mut(npc).unwrap().life = crate::game::map::creature::Life::Alive)
        .await;
    assert_eq!(house(&mut a, npc).await, Some(7), "alive again");
    let absent = crate::game::guid::unit(9_400, 0x7F_FFFF);
    assert_eq!(house(&mut a, absent).await, None, "no one there");
    set(|m, ga, _| crate::game::map::death::kill(m, ga)).await;
    assert_eq!(house(&mut a, npc).await, None, "to the dead");
    set(|m, ga, _| crate::game::map::death::release(m, ga)).await;
    assert_eq!(house(&mut a, npc).await, None, "to a ghost");
}

/// Thistle Tea's `Auction.sell` and `bid` "project escrow, owner listings,
/// buyout, and mail on the owning players": an item put up goes from the
/// seller's bags (its object forgotten by their client) and the deposit
/// from their money; it is in their own list; a buyout takes the buyer's
/// money and mails each side their part.
#[tokio::test]
async fn an_item_put_up_leaves_the_bags_lists_as_ones_own_and_a_buyout_mails_both() {
    use crate::game::tests_p3_core_auction as kit;
    let realm = harness::realm_with(kit::data());
    let mut a = kit::player(&realm, "A", "Alice").await;
    let mut b = kit::player(&realm, "B", "Bob").await;
    let item = kit::goods(&realm, a.guid).await;
    let (ga, gb) = (a.guid, b.guid);
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (id, started) = kit::sell(&mut a, item, 100, 1_000, 120).await;
    assert!(kit::ok(&started));
    a.wait_for("the item's object destroyed", |m| match m {
        ServerOpcodeMessage::SMSG_DESTROY_OBJECT(d) => (d.guid == item).then_some(()),
        _ => None,
    })
    .await;
    let (gone, money) = on(&map, move |m| {
        (
            m.players[&ga].item_by_guid(item).is_none(),
            m.players[&ga].money(),
        )
    })
    .await;
    assert_eq!(
        (gone, money),
        (true, 10_000 - 75),
        "the item, and the deposit"
    );
    a.send(CMSG_AUCTION_LIST_OWNER_ITEMS {
        auctioneer: kit::auctioneer(),
        list_from: 0,
    })
    .await;
    let mine = a
        .wait_for("SMSG_AUCTION_OWNER_LIST_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_AUCTION_OWNER_LIST_RESULT(l) => {
                Some(l.auctions.iter().map(|x| x.id).collect::<Vec<_>>())
            }
            _ => None,
        })
        .await;
    assert_eq!(mine, [id]);
    assert!(kit::ok(&kit::bid(&mut b, id, 1_000).await), "bought out");
    let money = on(&map, move |m| m.players[&gb].money()).await;
    assert_eq!(money, 9_000);
    let mails = |to: Guid| {
        realm
            .social
            .mail
            .lock()
            .mails
            .values()
            .filter(|m| m.receiver == crate::game::guid::counter(to))
            .map(|m| (m.money, usize::from(m.item.is_some())))
            .collect::<Vec<_>>()
    };
    assert_eq!(mails(gb), [(0, 1)], "the item to the buyer");
    // The goblins' cut is 15%: 850, and the deposit back.
    assert_eq!(mails(ga), [(850 + 75, 0)], "the proceeds to the seller");
}

/// A trigger at the start, sending its player ten yards on, under
/// condition `CONDITIONED`'s and `UNKNOWN`'s conditions.
const CONDITIONED: u32 = 9_000;
const UNKNOWN: u32 = 9_001;

fn trigger_world() -> crate::game::data::GameData {
    use crate::game::data::scripts::Condition;
    let mut d = data();
    for (id, condition, kind) in [(CONDITIONED, 1, 15), (UNKNOWN, 2, 9_999)] {
        d.area_triggers.insert(
            id,
            crate::game::data::AreaTrigger {
                id,
                map: 0,
                x: START.0,
                y: START.1,
                z: START.2,
                radius: 5.0,
                teleport: Some(crate::game::data::Teleport {
                    message: "Not you".into(),
                    map: 0,
                    x: START.0 + 10.0,
                    y: START.1,
                    z: START.2,
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        d.scripts.teleport_condition.insert(id, condition);
        d.scripts.conditions.insert(
            condition,
            Condition {
                entry: condition,
                kind,
                values: [10, 1, 0, 0],
                flags: 0,
            },
        );
    }
    d
}

/// Whether `a` stepping into `trigger` is sent on, or told why not.
async fn stepped(a: &mut Client, trigger: u32) -> Result<(), String> {
    a.clear();
    a.send(CMSG_AREATRIGGER {
        trigger_id: trigger,
    })
    .await;
    a.wait_for("a teleport or a refusal", |m| match m {
        ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_) => Some(Ok(())),
        ServerOpcodeMessage::SMSG_AREA_TRIGGER_MESSAGE(t) => Some(Err(t.message.clone())),
        _ => None,
    })
    .await
}

/// Thistle Tea's `AreaTriggers` "accepts or rejects a cached conditioned
/// teleport" and "rejects an unknown cached condition": a teleport whose
/// condition (level 10) the player meets sends them on, one short of it
/// is refused with the trigger's message; a condition of a kind wvs
/// doesn't know is unmet, refused.
#[tokio::test]
async fn a_conditioned_trigger_sends_on_those_who_meet_it_alone() {
    let realm = players_realm(trigger_world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    on(&map, move |m| crate::game::tests_p3_combat::level(m, ga, 9)).await;
    assert_eq!(stepped(&mut a, CONDITIONED).await, Err("Not you".into()));
    on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, ga, 10)
    })
    .await;
    assert_eq!(stepped(&mut a, CONDITIONED).await, Ok(()));
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: ga,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
    a.heartbeat(START.0, START.1, START.2).await;
    on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, ga, 60)
    })
    .await;
    assert_eq!(stepped(&mut a, UNKNOWN).await, Err("Not you".into()));
}

/// A buff of spell level 40, its only rank: it goes on no one below 30.
const HIGH_BUFF: u32 = 96_100;

fn buff_world() -> crate::game::data::GameData {
    use crate::game::data::spells::SpellRank;
    let mut d = data();
    let mut s = crate::game::tests_p3_entity::spell(
        HIGH_BUFF,
        &[crate::game::tests_p3_entity::aura_effect(29, 5, 2, 21)],
    );
    s.spell_level = 40;
    s.mana_cost = 10;
    d.spells.spells.insert(HIGH_BUFF, s);
    d.spells.ranks.insert(
        HIGH_BUFF,
        SpellRank {
            prev: 0,
            first: HIGH_BUFF,
            rank: 1,
            req: 0,
        },
    );
    d
}

/// Thistle Tea's `Spellcasting.cast_result` "rejects a target below the
/// first rank without spending mana": a buff with no rank low enough for
/// its target is refused LOWLEVEL (0x2B) with nothing spent (mangos's
/// `SelectAuraRankForLevel` finding none, then `CheckCast`); a target high
/// enough has it.
#[tokio::test]
async fn a_buff_with_no_rank_for_its_target_is_refused_lowlevel_for_nothing() {
    use crate::game::fields::UNIT_FIELD_POWER1;
    let realm = harness::realm_with(buff_world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Mage).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let (_b, gb) = crate::game::tests_p3_entity::bob(&realm, ga).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let cast = move |m: &mut crate::game::map::Map| {
        let p = m.players.get_mut(&ga).unwrap();
        p.fields.set_u32(UNIT_FIELD_POWER1, 500);
        crate::game::map::spells::cast(
            m,
            ga,
            HIGH_BUFF,
            crate::game::map::spells::Targets {
                unit: Some(gb),
                ..Default::default()
            },
            None,
        );
        (
            m.players[&ga].fields.get_u32(UNIT_FIELD_POWER1),
            m.players[&gb].auras.has_spell(HIGH_BUFF),
        )
    };
    let after = on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, ga, 60);
        crate::game::map::spells::learn(m, ga, HIGH_BUFF);
        crate::game::tests_p3_combat::level(m, gb, 29);
        a_clear_cooldowns(m, ga);
        cast(m)
    })
    .await;
    assert_eq!(after, (500, false), "nothing spent, nothing on");
    let result = a
        .wait_frame("SMSG_CAST_RESULT", 0x130, |f| {
            f[..4] == HIGH_BUFF.to_le_bytes()
        })
        .await;
    assert_eq!(&result[4..], &[2, 0x2B], "LOWLEVEL");
    let after = on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, gb, 30);
        a_clear_cooldowns(m, ga);
        cast(m)
    })
    .await;
    assert_eq!(after, (490, true), "at 30, it goes on");
}

fn a_clear_cooldowns(m: &mut crate::game::map::Map, g: Guid) {
    if let Some(c) = m.caster_mut(g) {
        c.cooldowns.clear();
        c.gcd_until = None;
    }
}

/// Thistle Tea's `SpellLoader.aura_rank` "stops at missing or cyclic
/// ancestors": the rank for a level walks back through the ranks before,
/// and finds none past a missing one, or round a loop.
#[test]
fn a_rank_for_a_level_stops_at_a_missing_or_looping_rank() {
    use crate::game::data::spells::SpellRank;
    use crate::game::map::spells::aura_rank::for_level;
    let mut d = buff_world();
    let low = HIGH_BUFF + 1;
    d.spells.ranks.insert(
        HIGH_BUFF,
        SpellRank {
            prev: low,
            first: low,
            rank: 2,
            req: 0,
        },
    );
    let high = d.spells.spells[&HIGH_BUFF].clone();
    assert_eq!(for_level(&d, &high, 1), None, "rank 1 missing");
    let mut lower = high.clone();
    lower.id = low;
    d.spells.spells.insert(low, lower);
    d.spells.ranks.insert(
        low,
        SpellRank {
            prev: HIGH_BUFF,
            first: low,
            rank: 1,
            req: 0,
        },
    );
    assert_eq!(for_level(&d, &high, 1), None, "round a loop");
    assert_eq!(for_level(&d, &high, 30), Some(HIGH_BUFF), "high enough");
}

/// Thistle Tea's `Spellcasting.cast_result` "self casts select a rank from
/// the owner's level": a buff cast on oneself is the rank one's own level
/// may have (mangos's self cast targets the caster, whose level
/// `SelectAuraRankForLevel` goes by).
#[tokio::test]
async fn a_buff_cast_on_oneself_is_the_rank_ones_level_may_have() {
    use crate::game::data::spells::SpellRank;
    let mut d = buff_world();
    let low = HIGH_BUFF + 1;
    let mut lower = d.spells.spells[&HIGH_BUFF].clone();
    lower.id = low;
    lower.spell_level = 1;
    d.spells.spells.insert(low, lower);
    for (id, prev, rank) in [(low, 0, 1), (HIGH_BUFF, low, 2)] {
        d.spells.ranks.insert(
            id,
            SpellRank {
                prev,
                first: low,
                rank,
                req: 0,
            },
        );
    }
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Mage).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let on_her = on(&map, move |m| {
        crate::game::map::spells::learn(m, ga, HIGH_BUFF);
        m.players
            .get_mut(&ga)
            .unwrap()
            .fields
            .set_u32(crate::game::fields::UNIT_FIELD_POWER1, 500);
        crate::game::map::spells::cast(m, ga, HIGH_BUFF, Default::default(), None);
        let p = &m.players[&ga];
        (p.auras.has_spell(low), p.auras.has_spell(HIGH_BUFF))
    })
    .await;
    assert_eq!(on_her, (true, false), "rank 1, at level 1");
}

/// Thistle Tea's `Spellcasting.cast_result` "rechecks the selected rank at
/// launch": a buff begun at a target high enough for it, who is too low
/// for it as the cast ends, fails LOWLEVEL with nothing spent (mangos's
/// `CheckCast` again at `Spell::cast`).
#[tokio::test]
async fn a_buff_too_high_for_its_target_at_the_casts_end_fails_lowlevel() {
    use crate::game::fields::UNIT_FIELD_POWER1;
    let mut d = buff_world();
    d.spells.cast_times.insert(14, 3_000);
    d.spells
        .spells
        .get_mut(&HIGH_BUFF)
        .unwrap()
        .casting_time_index = 14;
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Mage).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let (_b, gb) = crate::game::tests_p3_entity::bob(&realm, ga).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let casting = on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, ga, 60);
        crate::game::map::spells::learn(m, ga, HIGH_BUFF);
        crate::game::tests_p3_combat::level(m, gb, 30);
        m.players
            .get_mut(&ga)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_POWER1, 500);
        crate::game::map::spells::cast(
            m,
            ga,
            HIGH_BUFF,
            crate::game::map::spells::Targets {
                unit: Some(gb),
                ..Default::default()
            },
            None,
        );
        crate::game::tests_p3_combat::level(m, gb, 1);
        let now = m.now;
        m.caster_mut(ga)
            .and_then(|c| c.casting.as_mut())
            .map(|c| c.ends = now)
            .is_some()
    })
    .await;
    assert!(casting, "begun");
    a.clear();
    let result = a
        .wait_frame("SMSG_CAST_RESULT", 0x130, |f| {
            f[..4] == HIGH_BUFF.to_le_bytes()
        })
        .await;
    assert_eq!(&result[4..], &[2, 0x2B], "LOWLEVEL");
    let after = on(&map, move |m| {
        (
            m.players[&ga].fields.get_u32(UNIT_FIELD_POWER1),
            m.players[&gb].auras.has_spell(HIGH_BUFF),
        )
    })
    .await;
    assert_eq!(after, (500, false), "nothing spent, nothing on");
}

/// Thistle Tea's `Training.valid_trainer?` "rejects dead players,
/// inaccessible NPCs and wrong trainer classes": a trainer shows its list
/// to none dead; not dead itself, unflagged, fighting or unselectable
/// (mangos's `CanInteract`, `UNIT_FLAG_UNINTERACTIBLE`); a class trainer
/// to its class alone (`IsTrainerOf`).
#[tokio::test]
async fn a_trainer_teaches_the_living_its_class_and_none_while_it_cant_be_reached() {
    use crate::game::fields::{UNIT_FIELD_FLAGS, UNIT_NPC_FLAGS};
    let mut d = data();
    for (entry, class) in [(9_403, 0), (9_404, 8)] {
        d.creature_templates.insert(
            entry,
            crate::game::data::CreatureTemplate {
                faction: crate::game::tests_p3_combat::HUMANS,
                npc_flags: crate::game::map::npc::NPC_FLAG_TRAINER,
                trainer_class: class,
                ..crate::game::tests_p3_combat::template(entry, 7)
            },
        );
        d.npc.trainers.insert(
            entry,
            vec![crate::game::data::npc::TrainerSpell {
                spell: 2_275,
                cost: 10,
                required_level: 1,
                ..Default::default()
            }],
        );
    }
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (trainer, mages) = on(&map, move |m| {
        (
            crate::game::tests_p3_combat::spawn(m, ga, 9_403, 20, 2.0),
            crate::game::tests_p3_combat::spawn(m, ga, 9_404, 20, 2.0),
        )
    })
    .await;
    a.sees(trainer).await;
    assert!(trains(&mut a, trainer).await, "a trainer for all");
    assert!(!trains(&mut a, mages).await, "a mages' trainer");
    on(&map, move |m| {
        m.creature_mut(trainer)
            .unwrap()
            .fields
            .set_flag(UNIT_FIELD_FLAGS, 0x0200_0000, true)
    })
    .await;
    assert!(!trains(&mut a, trainer).await, "unselectable");
    on(&map, move |m| {
        let c = m.creature_mut(trainer).unwrap();
        c.fields.set_flag(UNIT_FIELD_FLAGS, 0x0200_0000, false);
        c.fields.set_u32(UNIT_NPC_FLAGS, 0);
    })
    .await;
    assert!(!trains(&mut a, trainer).await, "unflagged");
    on(&map, move |m| {
        m.creature_mut(trainer)
            .unwrap()
            .fields
            .set_u32(UNIT_NPC_FLAGS, crate::game::map::npc::NPC_FLAG_TRAINER);
        let brute = crate::game::tests_p3_combat::spawn(m, ga, BRUTE, 20, 4.0);
        if let Some(c) = m.combat_mut(trainer) {
            c.victim = brute;
            c.attacking = true;
        }
    })
    .await;
    assert!(!trains(&mut a, trainer).await, "fighting");
    on(&map, move |m| {
        if let Some(c) = m.combat_mut(trainer) {
            c.victim = Guid::zero();
            c.attacking = false;
        }
        let later = m.now + Duration::from_secs(60);
        m.creature_mut(trainer).unwrap().life = crate::game::map::creature::Life::Dead {
            corpse_until: later,
            respawn_at: later,
        };
    })
    .await;
    assert!(!trains(&mut a, trainer).await, "dead");
    on(&map, move |m| {
        m.creature_mut(trainer).unwrap().life = crate::game::map::creature::Life::Alive;
    })
    .await;
    assert!(trains(&mut a, trainer).await, "back");
    on(&map, move |m| crate::game::map::death::kill(m, ga)).await;
    assert!(!trains(&mut a, trainer).await, "to the dead");
}

/// Thistle Tea's `HomeBind.complete` "rejects a delayed bind after the
/// player dies or the innkeeper disappears": the dead are bound by no
/// innkeeper, nor anyone by one not there (mangos's
/// `HandleBinderActivateOpcode`, its bind a spell cast at once); the living
/// at one are.
#[tokio::test]
async fn an_innkeeper_binds_the_living_and_none_when_gone() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let inn = on(&map, move |m| {
        npc_beside(m, ga, crate::game::map::npc::NPC_FLAG_INNKEEPER, 2.0)
    })
    .await;
    a.sees(inn).await;
    let bound = async |a: &mut Client, npc: Guid| {
        a.clear();
        a.send(CMSG_BINDER_ACTIVATE { guid: npc }).await;
        handled(a).await;
        a.gets(NONE_WITHIN, |m| {
            matches!(m, ServerOpcodeMessage::SMSG_PLAYERBOUND(_)).then_some(())
        })
        .await
        .is_some()
    };
    on(&map, move |m| crate::game::map::death::kill(m, ga)).await;
    assert!(!bound(&mut a, inn).await, "the dead");
    on(&map, move |m| {
        crate::game::map::death::resurrect_with(m, ga, 100, 0);
        m.remove_creature(inn);
    })
    .await;
    assert!(!bound(&mut a, inn).await, "the innkeeper gone");
    let inn = on(&map, move |m| {
        npc_beside(m, ga, crate::game::map::npc::NPC_FLAG_INNKEEPER, 2.0)
    })
    .await;
    a.sees(inn).await;
    assert!(bound(&mut a, inn).await, "the living, at one");
}

/// The reclaim delay `a` is told within [`NONE_WITHIN`], in milliseconds.
async fn reclaim_delay(a: &mut Client) -> Option<u32> {
    tokio::time::timeout(
        NONE_WITHIN,
        a.wait_frame(
            "SMSG_CORPSE_RECLAIM_DELAY",
            crate::game::map::corpses::SMSG_CORPSE_RECLAIM_DELAY,
            |_| true,
        ),
    )
    .await
    .ok()
    .map(|b| u32::from_le_bytes(b[..4].try_into().unwrap()))
}

/// Thistle Tea's `Corpses.send_reclaim_delay` "projects remaining time on
/// reconnect without restarting it": a ghost logging back in is told what
/// is left of the wait since their release, not the wait anew; past it,
/// nothing; and nothing to the living.
#[tokio::test]
async fn a_ghost_back_is_told_the_wait_left_and_none_past_it() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    on(&map, move |m| crate::game::map::death::kill(m, ga)).await;
    // Released a moment after: a first death's thirty seconds.
    tokio::time::sleep(Duration::from_millis(20)).await;
    a.clear();
    a.send(CMSG_REPOP_REQUEST {}).await;
    let first = reclaim_delay(&mut a).await.expect("told the wait");
    assert!(first > 29_000 && first <= 30_000, "{first}");
    let relog = async |a: &mut Client, realm: &std::sync::Arc<crate::game::Realm>| {
        a.send(CMSG_LOGOUT_REQUEST {}).await;
        a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
        })
        .await;
        a.clear();
        a.login(ga).await;
        let _ = realm;
    };
    // Ten seconds on (the death and the release ten seconds back):
    // twenty left.
    let back = |by: i64| {
        let realm = realm.clone();
        async move {
            let map = realm.registry.online(ga).unwrap().map();
            on(&map, move |m| {
                let state = &mut m.players.get_mut(&ga).unwrap().row.state;
                state.released_at -= by;
                state.reclaim_until -= by;
            })
            .await;
        }
    };
    back(10_000).await;
    relog(&mut a, &realm).await;
    let left = reclaim_delay(&mut a).await.expect("told what is left");
    assert!(left > 19_000 && left <= 20_000, "{left}");
    back(25_000).await;
    relog(&mut a, &realm).await;
    assert_eq!(reclaim_delay(&mut a).await, None, "past it");
    let map = realm.registry.online(ga).unwrap().map();
    on(&map, move |m| {
        crate::game::map::death::resurrect_with(m, ga, 100, 0)
    })
    .await;
    relog(&mut a, &realm).await;
    assert_eq!(reclaim_delay(&mut a).await, None, "the living");
}

/// A warrior's talent of one rank, which brings an ability with it
/// (`spell_learn_spell`).
const TALENT_RANK: u32 = 96_200;
const TALENT_ABILITY: u32 = 96_201;

fn talent_world() -> crate::game::data::GameData {
    let mut d = data();
    d.spells.talents.insert(
        1,
        crate::game::data::spells::TalentInfo {
            id: 1,
            tab: 161,
            ranks: [TALENT_RANK, 0, 0, 0, 0],
            ..Default::default()
        },
    );
    d.spells.talent_tab_classes.insert(161, 0x1);
    let mut rank = crate::game::tests_p3_entity::spell(
        TALENT_RANK,
        &[crate::game::tests_p3_entity::aura_effect(29, 1, 0, 1)],
    );
    rank.attributes[0] = 0x40;
    d.spells.spells.insert(TALENT_RANK, rank);
    d.spells.spells.insert(
        TALENT_ABILITY,
        crate::game::tests_p3_entity::spell(TALENT_ABILITY, &[]),
    );
    d.spells
        .learn_with
        .insert(TALENT_RANK, vec![TALENT_ABILITY]);
    d
}

/// Thistle Tea's `Talents.learn` and `reset` "learns and resets
/// talent-dependent abilities", and `reset_if_overbudget` "resets talents
/// when the new level cannot support the spent points" and "keeps a valid
/// talent allocation": a talent brings its ability, and a reset takes both
/// (mangos's `removeSpell` takes a spell's dependents); a level the spent
/// points still fit keeps them, one they don't resets them (mangos's
/// `UpdateFreeTalentPoints`), the points given back.
#[tokio::test]
async fn a_talent_brings_its_ability_and_a_level_too_low_for_it_takes_both() {
    use crate::game::fields::PLAYER_CHARACTER_POINTS1;
    let realm = harness::realm_with(talent_world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    fn known(m: &crate::game::map::Map, ga: Guid) -> (bool, bool, u32) {
        let p = &m.players[&ga];
        (
            p.spells.contains(&TALENT_RANK),
            p.spells.contains(&TALENT_ABILITY),
            p.fields.get_u32(PLAYER_CHARACTER_POINTS1),
        )
    }
    let learned = on(&map, move |m| {
        // A player, as mangos leaves an administrator's own alone.
        m.players.get_mut(&ga).unwrap().gm_level = crate::accounts::PLAYER;
        crate::game::tests_p3_combat::level(m, ga, 10);
        crate::game::map::spells::learn_talent(m, ga, 1, 0);
        known(m, ga)
    })
    .await;
    assert_eq!(learned, (true, true, 0), "the talent and its ability");
    // A level set as `.character level` sets it.
    let set = |to: u8| {
        let map = map.clone();
        async move {
            on(&map, move |m| {
                crate::game::tests_p3_combat::level(m, ga, to);
                crate::game::map::misc::reset_if_overbudget(m, ga);
                known(m, ga)
            })
            .await
        }
    };
    assert_eq!(set(10).await, (true, true, 0), "it fits");
    assert_eq!(set(9).await, (false, false, 0), "too low: both gone");
    assert_eq!(set(12).await, (false, false, 3), "points back");
    // An administrator's own are kept, with no point free.
    on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().gm_level = crate::accounts::ADMINISTRATOR;
        crate::game::tests_p3_combat::level(m, ga, 10);
        crate::game::map::spells::learn_talent(m, ga, 1, 0);
    })
    .await;
    assert_eq!(set(9).await, (true, true, 0), "an administrator's");
}

/// Thistle Tea's `TalentReset.complete` "charges once, refunds points,
/// removes dependent buffs, and preserves other classes' spells": a
/// trainer's reset takes the talent, the ability that came with it and the
/// talent's aura, gives the point back and keeps the rest of the spellbook;
/// it costs a gold, once.
#[tokio::test]
async fn a_talent_reset_takes_the_talent_its_ability_and_aura_and_gives_the_point_back() {
    use crate::game::fields::PLAYER_CHARACTER_POINTS1;
    let mut d = talent_world();
    d.creature_templates.insert(
        9_405,
        crate::game::data::CreatureTemplate {
            faction: crate::game::tests_p3_combat::HUMANS,
            npc_flags: crate::game::map::npc::NPC_FLAG_TRAINER,
            trainer_class: 1,
            ..crate::game::tests_p3_combat::template(9_405, 7)
        },
    );
    d.spells.spells.insert(
        HIGH_BUFF,
        crate::game::tests_p3_entity::spell(HIGH_BUFF, &[]),
    );
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let trainer = on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, ga, 20);
        crate::game::map::spells::learn(m, ga, HIGH_BUFF);
        crate::game::map::spells::learn_talent(m, ga, 1, 0);
        m.players.get_mut(&ga).unwrap().set_money(30_000);
        crate::game::tests_p3_combat::spawn(m, ga, 9_405, 20, 2.0)
    })
    .await;
    a.sees(trainer).await;
    let before = on(&map, move |m| {
        let p = &m.players[&ga];
        (
            p.auras.has_spell(TALENT_RANK),
            p.fields.get_u32(PLAYER_CHARACTER_POINTS1),
        )
    })
    .await;
    assert_eq!(before, (true, 10), "the talent's aura on, a point spent");
    a.send(MSG_TALENT_WIPE_CONFIRM_Client {
        wiping_npc: trainer,
    })
    .await;
    handled(&mut a).await;
    let after = on(&map, move |m| {
        let p = &m.players[&ga];
        (
            p.spells.contains(&TALENT_RANK),
            p.spells.contains(&TALENT_ABILITY),
            p.auras.has_spell(TALENT_RANK),
            p.spells.contains(&HIGH_BUFF),
            p.fields.get_u32(PLAYER_CHARACTER_POINTS1),
            p.money(),
        )
    })
    .await;
    assert_eq!(
        after,
        (false, false, false, true, 11, 20_000),
        "talent, ability and aura gone, the rest kept, the point back, a gold paid"
    );
    a.send(MSG_TALENT_WIPE_CONFIRM_Client {
        wiping_npc: trainer,
    })
    .await;
    handled(&mut a).await;
    let money = on(&map, move |m| m.players[&ga].money()).await;
    assert_eq!(money, 20_000, "nothing to reset, nothing paid");
}
