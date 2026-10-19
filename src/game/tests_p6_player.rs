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
