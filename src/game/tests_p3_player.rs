//! Phase 3 against Thistle Tea's player tests (`test/game/core/player`):
//! experience and its formulas, rested XP, played time, the player flags,
//! the action bar, talents, trainers and drink. Through the map where it
//! is a behaviour, as a unit test where it is a formula.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::npc::Quest;
use crate::game::data::{AreaTrigger, CreatureSpawn, CreatureTemplate, GameData};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::combat::{self, KillXp};
use crate::game::map::pets::{Kind, PetState};
use crate::game::map::player::{
    PLAYER_FLAGS_GROUP_LEADER, PLAYER_FLAGS_HIDE_HELM, PLAYER_FLAGS_RESTING, Player,
};
use crate::game::map::rest::{self, RestType};
use crate::game::map::{Map, MapKey, stats};

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

/// Creatures to kill, as each kind of kill: an elite, and an ordinary one.
const ELITE: u32 = 960_001;
const PLAIN: u32 = 960_002;
const ELITE_A: u32 = 96_001;
const PLAIN_A: u32 = 96_002;
const PLAIN_B: u32 = 96_003;
const PLAIN_C: u32 = 96_004;

fn creature(entry: u32, rank: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: format!("Creature {entry}"),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 20,
        level_max: 20,
        faction: 14,
        unit_class: 1,
        rank,
        health_multiplier: 1.0,
        damage_multiplier: 1.0,
        base_attack_time: 2000,
        health_max: 500,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.creature_templates.insert(ELITE, creature(ELITE, 1));
    d.creature_templates.insert(PLAIN, creature(PLAIN, 0));
    let spawn = |guid, entry, dx: f32| CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1 + 30.0,
        z: START.2,
        respawn_min: 600,
        respawn_max: 600,
        ..Default::default()
    };
    d.creature_spawns.insert(
        0,
        vec![
            spawn(ELITE_A, ELITE, 0.0),
            spawn(PLAIN_A, PLAIN, 3.0),
            spawn(PLAIN_B, PLAIN, 6.0),
            spawn(PLAIN_C, PLAIN, 9.0),
        ],
    );
    d
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

/// A realm on [`data`], and Alice, a human warrior in Northshire.
async fn alice() -> (Arc<Realm>, Client, Guid) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    (realm, a, ga)
}

/// The levels `SMSG_LEVELUP_INFO` announced, in order, until none come
/// for a moment.
async fn level_ups(c: &mut Client) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(l) = c
        .gets(Duration::from_millis(300), |m| match m {
            ServerOpcodeMessage::SMSG_LEVELUP_INFO(l) => Some(l.new_level.as_int()),
            _ => None,
        })
        .await
    {
        out.push(l);
    }
    out
}

const SMSG_LOG_XPGAIN: u16 = 0x1D0;

/// Wait until the map has handled what `c` sent so far: it answers
/// `CMSG_PLAYED_TIME` in turn.
async fn synced(c: &mut Client) {
    c.send(CMSG_PLAYED_TIME {}).await;
    c.wait_for("SMSG_PLAYED_TIME", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_PLAYED_TIME(_)).then_some(())
    })
    .await;
}

#[tokio::test]
async fn experience_chains_levels_from_one_grant_stops_at_the_top_and_takes_nothing_there() {
    let (realm, mut a, ga) = alice().await;
    a.clear();
    // Enough at level one for two levels and fifty over: level three,
    // fifty in, a level-up each.
    let (level, xp) = on_map(&realm, move |m| {
        let data = m.realm.data.clone();
        let two = data.xp_to_level(1) + data.xp_to_level(2) + 50;
        stats::give_xp(m, ga, two, None);
        let p = &m.players[&ga];
        (p.level(), p.fields.get_u32(PLAYER_XP))
    })
    .await;
    assert_eq!((level, xp), (3, 50));
    assert!(told_xp(&mut a).await);
    assert_eq!(level_ups(&mut a).await, vec![2, 3]);
    // Nothing for nothing: no gain told, no XP.
    a.clear();
    let xp = on_map(&realm, move |m| {
        stats::give_xp(m, ga, 0, None);
        m.players[&ga].fields.get_u32(PLAYER_XP)
    })
    .await;
    assert_eq!(xp, 50);
    // At 59, far more than a level: 60, and the XP gone.
    let (level, xp, next) = on_map(&realm, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&ga).unwrap();
        stats::set_level(p, &data, 59);
        stats::give_xp(m, ga, 10_000_000, None);
        let p = &m.players[&ga];
        (p.level(), p.fields.get_u32(PLAYER_XP), data.xp_to_level(59))
    })
    .await;
    assert!(next > 0);
    assert_eq!((level, xp), (60, 0));
    // At the top, a grant is not even told.
    a.clear();
    a.raw.clear();
    let xp = on_map(&realm, move |m| {
        stats::give_xp(m, ga, 500, None);
        m.players[&ga].fields.get_u32(PLAYER_XP)
    })
    .await;
    assert_eq!(xp, 0);
    assert!(!told_xp(&mut a).await);
}

/// Whether `SMSG_LOG_XPGAIN` came in a moment, read or raw.
async fn told_xp(c: &mut Client) -> bool {
    let read = c
        .gets(Duration::from_millis(300), |m| match m {
            ServerOpcodeMessage::SMSG_LOG_XPGAIN(_) => Some(()),
            _ => None,
        })
        .await
        .is_some();
    let before = c.raw.len();
    c.raw
        .retain(|f| u16::from_le_bytes([f[2], f[3]]) != SMSG_LOG_XPGAIN);
    read || c.raw.len() < before
}

#[test]
fn a_quests_xp_is_whole_to_five_levels_over_then_less_rounded_up_and_none_without_a_reward() {
    let quest = |level, reward_xp| Quest {
        level,
        reward_xp,
        ..Default::default()
    };
    let q = quest(2, 900);
    let xp = |level| crate::game::map::quests::quest_xp(&q, level);
    assert_eq!((xp(1), xp(7)), (900, 900));
    assert_eq!(
        [xp(8), xp(9), xp(10), xp(11), xp(12)],
        [720, 540, 360, 180, 90]
    );
    // A share that doesn't come out whole is rounded up: 905 * 0.1.
    let odd = quest(2, 905);
    assert_eq!(crate::game::map::quests::quest_xp(&odd, 30), 91);
    assert_eq!(crate::game::map::quests::quest_xp(&quest(10, 0), 10), 0);
}

#[test]
fn kill_xp_caps_a_higher_levels_bonus_at_four_and_a_pets_kill_is_three_quarters_without_elites() {
    let plain = KillXp::default();
    assert_eq!(
        combat::kill_xp_with(10, 10, 14, &plain),
        combat::kill_xp_with(10, 10, 15, &plain)
    );
    assert!(combat::kill_xp_with(10, 10, 14, &plain) > combat::kill_xp_with(10, 10, 13, &plain));
    let pet = KillXp {
        pet_victim: true,
        ..Default::default()
    };
    assert_eq!(combat::kill_xp_with(24, 24, 24, &pet), 124);
    // An elite pet, in a dungeon, is a pet: no elite bonus.
    let elite_pet = KillXp {
        elite: true,
        dungeon: true,
        ..pet
    };
    assert_eq!(combat::kill_xp_with(24, 24, 24, &elite_pet), 124);
    let halved = KillXp {
        multiplier: 2.0,
        damage_share: 0.5,
        ..pet
    };
    assert_eq!(combat::kill_xp_with(24, 24, 24, &halved), 124);
    // Its base from its owner's level, the level factor from its own.
    assert_eq!(combat::kill_xp_with(24, 20, 24, &pet), 149);
}

#[test]
fn three_share_a_kill_at_a_sixth_more_and_a_kill_grey_to_all_gives_none() {
    let plain = KillXp::default();
    let (shares, top) = combat::group_shares(&[10, 10, 10], 10, &plain);
    let base = combat::kill_xp(10, 10, false, 1.0) as f32;
    let each = (base * 1.166 * 10.0 / 30.0) as u32;
    assert_eq!(top, Some(10));
    assert_eq!(shares, vec![each; 3]);
    let (shares, top) = combat::group_shares(&[60, 60], 1, &plain);
    assert_eq!((shares, top), (vec![0, 0], None));
    let (shares, _) = combat::group_shares(&[], 10, &plain);
    assert!(shares.is_empty());
}

#[tokio::test]
async fn a_kills_options_tell_npc_pets_players_pets_and_charmed_units_flags_and_summons() {
    let (realm, _a, ga) = alice().await;
    let elite = guid::unit(ELITE, ELITE_A);
    let (a, b, c) = (
        guid::unit(PLAIN, PLAIN_A),
        guid::unit(PLAIN, PLAIN_B),
        guid::unit(PLAIN, PLAIN_C),
    );
    let xp = move |m: &Map, g: Guid| combat::kill_xp_with(20, 20, 20, &combat::kill_options(m, g));
    let got = on_map(&realm, move |m| {
        let now = m.now;
        let mut out = Vec::new();
        // An elite as it is; as an NPC's pet; as a player's.
        out.push(xp(m, elite));
        let own = |m: &mut Map, unit: Guid, owner: Guid, kind: Kind| {
            let c = m.creature_mut(unit).unwrap();
            c.owner = Some(owner);
            c.pet = Some(Box::new(PetState::new(kind, owner, now)));
        };
        own(m, elite, a, Kind::Demon);
        out.push(xp(m, elite));
        own(m, elite, ga, Kind::Hunter);
        out.push(xp(m, elite));
        // Charmed by a player.
        crate::game::map::pets::charmed(m, ga, b, false);
        out.push(xp(m, b));
        out
    })
    .await;
    // 20 * 5 + 45, doubled for an elite; three quarters, no elite bonus,
    // as a pet; nothing as a player's.
    assert_eq!(got, vec![290, 109, 0, 0]);
    // The no-XP flag; always running (flags_extra 0x40) is no bar. A
    // summoned critter, unspecified creature or totem, or one of a tenth
    // of the health or less, gives nothing; one of more, its XP.
    let mut d = data();
    for (entry, creature_type, health) in [
        (PLAIN, 10, 1.0),
        (PLAIN + 10, 8, 1.0),
        (PLAIN + 11, 11, 1.0),
        (PLAIN + 12, 7, 0.1),
        (PLAIN + 13, 7, 0.11),
    ] {
        let mut t = creature(entry, 0);
        t.creature_type = creature_type;
        t.health_multiplier = health;
        d.creature_templates.insert(entry, t);
    }
    let mut runs = creature(PLAIN + 20, 0);
    runs.flags_extra = 0x40;
    d.creature_templates.insert(PLAIN + 20, runs);
    let mut no_xp = creature(PLAIN + 21, 0);
    no_xp.static_flags1 = 0x2;
    d.creature_templates.insert(PLAIN + 21, no_xp);
    let realm = harness::realm_with(d);
    let mut z = Client::connect(&realm, "Z");
    z.enter("Zed", Race::Human, Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        let mut out = Vec::new();
        for entry in [PLAIN, PLAIN + 10, PLAIN + 11, PLAIN + 12, PLAIN + 13] {
            let unit = m.creature_mut(c).unwrap();
            unit.entry = entry;
            unit.fields.set_u32(OBJECT_FIELD_ENTRY, entry);
            unit.fields.set_u32(UNIT_CREATED_BY_SPELL, 0);
            let wild = xp(m, c);
            m.creature_mut(c)
                .unwrap()
                .fields
                .set_u32(UNIT_CREATED_BY_SPELL, 1);
            out.push((entry, wild, xp(m, c)));
        }
        for entry in [PLAIN + 20, PLAIN + 21] {
            let unit = m.creature_mut(a).unwrap();
            unit.entry = entry;
            out.push((entry, xp(m, a), 0));
        }
        out
    })
    .await;
    assert_eq!(
        got,
        vec![
            (PLAIN, 145, 0),
            (PLAIN + 10, 145, 0),
            (PLAIN + 11, 145, 0),
            (PLAIN + 12, 145, 0),
            (PLAIN + 13, 145, 145),
            (PLAIN + 20, 145, 0),
            (PLAIN + 21, 0, 0),
        ]
    );
}

/// An inn about Northshire's start, its trigger ten yards across.
const TAVERN: u32 = 9_601;

fn with_inn(mut d: GameData) -> GameData {
    d.npc.taverns.insert(TAVERN);
    d.area_triggers.insert(
        TAVERN,
        AreaTrigger {
            id: TAVERN,
            map: 0,
            x: START.0,
            y: START.1,
            z: START.2,
            radius: 10.0,
            ..Default::default()
        },
    );
    d
}

/// A level's XP a second, so a second's rest is one XP.
const LEVEL: u32 = 1_152_000;

#[tokio::test]
async fn rest_fills_at_a_levels_xp_over_1152000_a_second_to_three_quarters_flipping_the_rested_state()
 {
    let (realm, _a, ga) = alice().await;
    let got = on_map(&realm, move |m| {
        let now = m.now;
        let p = m.players.get_mut(&ga).unwrap();
        p.fields.set_u32(PLAYER_NEXT_LEVEL_XP, LEVEL);
        let state = |p: &Player| p.fields.get_byte(PLAYER_BYTES_2, 3);
        let pool = |p: &Player| p.fields.get_u32(PLAYER_REST_STATE_EXPERIENCE);
        let mut out = Vec::new();
        // Not resting, nothing comes, however long.
        let spent = rest::spend(p, 50, now + Duration::from_secs(100));
        out.push((spent, pool(p), state(p)));
        // Resting, what is owed is counted before a kill spends it.
        rest::start(p, RestType::City, now);
        let spent = rest::spend(p, 50, now + Duration::from_secs(100));
        out.push((spent, pool(p), state(p)));
        // Five and eleven: rested above ten, and only back to normal at
        // one or below.
        rest::stop(p, now + Duration::from_secs(100));
        rest::add_bonus(p, -45.0);
        out.push((0, pool(p), state(p)));
        rest::add_bonus(p, 6.0);
        out.push((0, pool(p), state(p)));
        rest::add_bonus(p, -6.0);
        out.push((0, pool(p), state(p)));
        rest::add_bonus(p, -4.0);
        out.push((0, pool(p), state(p)));
        // Three quarters of a level at most.
        rest::add_bonus(p, 2.0e12);
        out.push((0, pool(p), state(p)));
        out
    })
    .await;
    assert_eq!(
        got,
        vec![
            (0, 0, 2),
            (50, 50, 1),
            (0, 5, 1),
            (0, 11, 1),
            (0, 5, 1),
            (0, 1, 2),
            (0, LEVEL * 3 / 4, 1),
        ]
    );
}

#[tokio::test]
async fn rest_while_logged_out_is_a_rest_areas_rate_or_a_quarter_in_the_wild_and_none_for_the_new()
{
    let (realm, _a, ga) = alice().await;
    let got = on_map(&realm, move |m| {
        let now = m.now;
        let wall = chrono::Utc::now().timestamp();
        let p = m.players.get_mut(&ga).unwrap();
        p.fields.set_u32(PLAYER_NEXT_LEVEL_XP, LEVEL);
        let mut out = Vec::new();
        let mut back = |p: &mut Player, bonus: f32, kind, away: i64| {
            p.row.state.rest_bonus = bonus;
            p.row.state.rest_type = kind;
            p.row.logout_time = if away == 0 { 0 } else { wall - away };
            rest::restore(p, now);
            out.push((
                p.row.state.rest_bonus,
                p.fields.has_flag(PLAYER_FLAGS, PLAYER_FLAGS_RESTING),
            ));
        };
        // An inn's or a city's rest: a second's XP a second, resting on.
        back(p, 100.0, Some(RestType::City), 100);
        back(p, 100.0, Some(RestType::Tavern(TAVERN)), 100);
        // The wild: a quarter, not resting; a fraction kept.
        back(p, 0.25, None, 1);
        back(p, 0.0, None, 100);
        // The cap holds.
        back(p, 0.0, None, 100_000_000);
        // A new character, and a logout in the future: nothing.
        back(p, 0.0, Some(RestType::City), 0);
        back(p, 0.0, Some(RestType::City), -1_000);
        out
    })
    .await;
    let near = |a: f32, b: f32| (a - b).abs() < 1.5;
    assert!(near(got[0].0, 200.0) && got[0].1, "{got:?}");
    assert!(near(got[1].0, 200.0) && got[1].1, "{got:?}");
    assert!((got[2].0 - 0.5).abs() < 0.3 && !got[2].1, "{got:?}");
    assert!(near(got[3].0, 25.0) && !got[3].1, "{got:?}");
    assert_eq!(got[4].0, LEVEL as f32 * 0.75);
    assert_eq!((got[5].0, got[6].0), (0.0, 0.0));
    // At the top level there is no next to rest toward: an inn's day
    // gives nothing.
    let top = on_map(&realm, move |m| {
        let now = m.now;
        let data = m.realm.data.clone();
        let wall = chrono::Utc::now().timestamp();
        let p = m.players.get_mut(&ga).unwrap();
        stats::set_level(p, &data, 60);
        p.row.state.rest_bonus = 0.0;
        p.row.state.rest_type = Some(RestType::City);
        p.row.logout_time = wall - 86_400;
        rest::restore(p, now);
        let spent = rest::spend(p, 100, now + Duration::from_secs(3_600));
        (
            p.row.state.rest_bonus,
            p.fields.get_u32(PLAYER_REST_STATE_EXPERIENCE),
            spent,
        )
    })
    .await;
    assert_eq!(top, (0.0, 0, 0));
}

#[tokio::test]
async fn an_inns_rest_is_counted_every_ten_seconds_kept_at_logout_and_ends_on_leaving() {
    let realm = harness::realm_with(with_inn(data()));
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    a.send(CMSG_AREATRIGGER { trigger_id: TAVERN }).await;
    synced(&mut a).await;
    let got = on_map(&realm, move |m| {
        let now = m.now;
        let p = m.players.get_mut(&ga).unwrap();
        let resting = p.fields.has_flag(PLAYER_FLAGS, PLAYER_FLAGS_RESTING);
        p.fields.set_u32(PLAYER_NEXT_LEVEL_XP, LEVEL);
        p.row.state.rest_bonus = 0.0;
        // Short of ten seconds, nothing is counted yet; at ten, ten.
        p.misc.rest_since = Some(now - Duration::from_millis(9_900));
        p.misc.rest_checked_at = None;
        rest::update(m);
        let early = m.players[&ga].row.state.rest_bonus;
        let p = m.players.get_mut(&ga).unwrap();
        p.misc.rest_since = Some(now - Duration::from_secs(10));
        p.misc.rest_checked_at = None;
        rest::update(m);
        let p = m.players.get_mut(&ga).unwrap();
        let ten = (
            p.row.state.rest_bonus,
            p.fields.get_u32(PLAYER_REST_STATE_EXPERIENCE),
        );
        // A save (as at logout) counts what is owed since.
        p.misc.rest_since = Some(now - Duration::from_secs(7));
        let saved = p.to_row(now).state.rest_bonus;
        (resting, early, ten, saved)
    })
    .await;
    let (resting, early, ten, saved) = got;
    assert!(resting, "the inn's trigger starts the rest");
    assert_eq!(early, 0.0);
    assert!((ten.0 - 10.0).abs() < 0.01 && ten.1 == 10, "{ten:?}");
    assert!((saved - 17.0).abs() < 0.01, "{saved}");
    // Out of the inn, the rest ends.
    a.heartbeat(START.0 + 40.0, START.1, START.2).await;
    synced(&mut a).await;
    let left = on_map(&realm, move |m| {
        m.players.get_mut(&ga).unwrap().misc.rest_checked_at = None;
        rest::update(m);
        let p = &m.players[&ga];
        (
            p.fields.has_flag(PLAYER_FLAGS, PLAYER_FLAGS_RESTING),
            p.row.state.rest_type,
        )
    })
    .await;
    assert_eq!(left, (false, None));
}

#[tokio::test]
async fn played_time_counts_once_and_a_new_level_starts_its_own_count() {
    let (realm, mut a, ga) = alice().await;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.played_at -= Duration::from_secs(65);
    })
    .await;
    let ask = async |a: &mut Client| {
        a.send(CMSG_PLAYED_TIME {}).await;
        a.wait_for("SMSG_PLAYED_TIME", |m| match m {
            ServerOpcodeMessage::SMSG_PLAYED_TIME(t) => {
                Some((t.total_played_time, t.level_played_time))
            }
            _ => None,
        })
        .await
    };
    assert_eq!(ask(&mut a).await, (65, 65));
    assert_eq!(ask(&mut a).await, (65, 65));
    // Thirty seconds on, a level; thirty more: the new level has its
    // thirty, the total all.
    on_map(&realm, move |m| {
        m.players.get_mut(&ga).unwrap().played_at -= Duration::from_secs(30);
        let next = m.realm.data.xp_to_level(1);
        stats::give_xp(m, ga, next, None);
        m.players.get_mut(&ga).unwrap().played_at -= Duration::from_secs(30);
    })
    .await;
    assert_eq!(ask(&mut a).await, (125, 30));
}

#[tokio::test]
async fn the_group_leaders_flag_goes_on_and_off_alone() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    a.send(CMSG_TOGGLE_HELM {}).await;
    synced(&mut a).await;
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
    let flags = async |realm: &Arc<Realm>| {
        // The flag is set by a call to the map; one more call after it.
        on_map(realm, |_| ()).await;
        on_map(realm, move |m| m.players[&ga].fields.get_u32(PLAYER_FLAGS)).await
    };
    let led = flags(&realm).await;
    a.send(CMSG_GROUP_DISBAND {}).await;
    let mut left = flags(&realm).await;
    for _ in 0..50 {
        if left & PLAYER_FLAGS_GROUP_LEADER == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        left = flags(&realm).await;
    }
    assert_eq!(
        led & (PLAYER_FLAGS_GROUP_LEADER | PLAYER_FLAGS_HIDE_HELM),
        0x401
    );
    assert_eq!(
        left & (PLAYER_FLAGS_GROUP_LEADER | PLAYER_FLAGS_HIDE_HELM),
        0x400
    );
}

/// A warrior's tree (tab 161) as Thistle Tea's test catalogue has it, and
/// a mage's talent (tab 999).
const WARRIOR_TAB: u32 = 161;
const MAGE_TAB: u32 = 999;

fn with_talents(mut d: GameData) -> GameData {
    use crate::game::data::spells::{SpellTemplate, TalentInfo};
    let talent = |id, tab, row, column, ranks: &[u32], depends_on, depends_rank| {
        let mut all = [0; 5];
        all[..ranks.len()].copy_from_slice(ranks);
        TalentInfo {
            id,
            tab,
            row,
            column,
            ranks: all,
            depends_on,
            depends_rank,
        }
    };
    for t in [
        talent(100, WARRIOR_TAB, 0, 0, &[1_001, 1_002, 1_003], 0, 0),
        talent(101, WARRIOR_TAB, 1, 0, &[2_001], 0, 0),
        talent(102, WARRIOR_TAB, 1, 1, &[3_001], 100, 2),
        talent(103, WARRIOR_TAB, 0, 1, &[5_001, 5_002, 5_003], 0, 0),
        talent(200, MAGE_TAB, 0, 0, &[4_001], 0, 0),
    ] {
        for spell in t.ranks.into_iter().filter(|s| *s != 0) {
            d.spells.spells.insert(
                spell,
                SpellTemplate {
                    id: spell,
                    name: format!("Talent {spell}"),
                    equipped_item_class: -1,
                    ..Default::default()
                },
            );
        }
        d.spells.talents.insert(t.id, t);
    }
    d.spells.talent_tab_classes.insert(WARRIOR_TAB, 0x1);
    d.spells.talent_tab_classes.insert(MAGE_TAB, 0x80);
    d
}

#[tokio::test]
async fn talents_cost_a_point_a_rank_jumped_open_a_tier_at_five_and_want_their_prerequisites_rank()
{
    let realm = harness::realm_with(with_talents(data()));
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        use crate::game::map::spells;
        let data = m.realm.data.clone();
        let mut out = Vec::new();
        // Learn `talent`'s `rank` at `level` knowing `known`: what is
        // known after.
        let mut try_learn = |m: &mut Map, level, known: &[u32], talent, rank| {
            let p = m.players.get_mut(&ga).unwrap();
            p.spells.retain(|s| !(1_000..6_000).contains(s));
            p.spells.extend_from_slice(known);
            stats::set_level(p, &data, level);
            spells::learn_talent(m, ga, talent, rank);
            let mut have: Vec<u32> = m.players[&ga]
                .spells
                .iter()
                .copied()
                .filter(|s| (1_000..6_000).contains(s))
                .collect();
            have.sort_unstable();
            out.push(have);
        };
        // A jump of a rank wants a point for each.
        try_learn(m, 10, &[], 100, 1);
        try_learn(m, 11, &[], 100, 1);
        // Row two wants five in the tree.
        try_learn(m, 20, &[1_003], 101, 0);
        try_learn(m, 20, &[1_003, 5_001], 101, 0);
        try_learn(m, 20, &[1_003, 5_002], 101, 0);
        // Its prerequisite's third rank.
        try_learn(m, 20, &[1_002, 5_003], 102, 0);
        try_learn(m, 20, &[1_003, 5_002], 102, 0);
        // Another class's tree, and a rank known.
        try_learn(m, 60, &[], 200, 0);
        try_learn(m, 20, &[1_001], 100, 0);
        // Spent points are the highest ranks known (of eleven at 20).
        let p = m.players.get_mut(&ga).unwrap();
        let mut spent = Vec::new();
        for known in [&[1_002, 2_001][..], &[1_001, 1_002], &[9_999]] {
            p.spells.retain(|s| !(1_000..10_000).contains(s));
            p.spells.extend_from_slice(known);
            spent.push(11 - spells::free_talent_points(p, &data));
        }
        (out, spent)
    })
    .await;
    let (learned, spent) = got;
    assert_eq!(
        learned,
        vec![
            vec![],
            vec![1_002],
            vec![1_003],
            vec![1_003, 5_001],
            vec![1_003, 2_001, 5_002],
            vec![1_002, 5_003],
            vec![1_003, 3_001, 5_002],
            vec![],
            vec![1_001],
        ]
    );
    assert_eq!(spent, vec![3, 2, 0]);
}

/// A warriors' trainer beside Northshire's start, and what it teaches.
const TRAINER: u32 = 960_010;
const TRAINER_A: u32 = 96_010;
const KNOWN: u32 = 960_101;
const WANTS_SPELL: u32 = 960_102;
const WANTED: u32 = 960_103;
const WANTS_SKILL: u32 = 960_104;
const WANTS_COOKING: u32 = 960_105;
const APPRENTICE_MINING: u32 = 960_106;
const FOR_MAGES: u32 = 960_107;
const FOR_HUMANS: u32 = 960_108;
const FOR_EITHER: u32 = 960_109;
const FOR_ORCS: u32 = 960_110;
const FOR_ALL: u32 = 960_111;
const TALENT_SPELL: u32 = 1_001;

fn with_trainer(mut d: GameData) -> GameData {
    use crate::game::data::npc::TrainerSpell;
    use crate::game::data::spells::{SkillAbility, SpellEffect, SpellRank, SpellTemplate};
    let mut t = creature(TRAINER, 0);
    t.name = "Trainer".into();
    t.faction = 12;
    t.npc_flags = 0x10;
    t.trainer_class = 1;
    d.creature_templates.insert(TRAINER, t);
    d.creature_spawns.get_mut(&0).unwrap().push(CreatureSpawn {
        guid: TRAINER_A,
        entries: vec![TRAINER],
        map: 0,
        x: START.0 + 2.0,
        y: START.1,
        z: START.2,
        respawn_min: 600,
        respawn_max: 600,
        ..Default::default()
    });
    let all = [
        KNOWN,
        WANTS_SPELL,
        WANTED,
        WANTS_SKILL,
        WANTS_COOKING,
        APPRENTICE_MINING,
        FOR_MAGES,
        FOR_HUMANS,
        FOR_EITHER,
        FOR_ORCS,
        FOR_ALL,
    ];
    for id in all {
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
    d.spells.spells.get_mut(&APPRENTICE_MINING).unwrap().effects[0] = SpellEffect {
        effect: 44,
        misc_value: 186,
        base_points: 0,
        ..Default::default()
    };
    d.spells.ranks.insert(
        WANTS_SPELL,
        SpellRank {
            first: WANTS_SPELL,
            rank: 1,
            req: WANTED,
            ..Default::default()
        },
    );
    let line = |spell, class_mask, race_mask| SkillAbility {
        skill: 26,
        spell,
        class_mask,
        race_mask,
        ..Default::default()
    };
    d.spells
        .skill_abilities
        .insert(FOR_MAGES, vec![line(FOR_MAGES, 0x80, 0)]);
    d.spells
        .skill_abilities
        .insert(FOR_HUMANS, vec![line(FOR_HUMANS, 0, 0x1)]);
    d.spells.skill_abilities.insert(
        FOR_EITHER,
        vec![line(FOR_EITHER, 0x1, 0), line(FOR_EITHER, 0x80, 0)],
    );
    d.spells
        .skill_abilities
        .insert(FOR_ORCS, vec![line(FOR_ORCS, 0, 0x2)]);
    let teach = |spell, required_skill, required_skill_value| TrainerSpell {
        spell,
        cost: 10,
        required_skill,
        required_skill_value,
        required_level: 1,
    };
    d.npc.trainers.insert(
        TRAINER,
        vec![
            teach(KNOWN, 0, 0),
            teach(WANTS_SPELL, 0, 0),
            teach(WANTS_SKILL, 171, 50),
            teach(WANTS_COOKING, 185, 50),
            teach(APPRENTICE_MINING, 0, 0),
            teach(FOR_MAGES, 0, 0),
            teach(FOR_HUMANS, 0, 0),
            teach(FOR_EITHER, 0, 0),
            teach(FOR_ORCS, 0, 0),
            teach(FOR_ALL, 0, 0),
        ],
    );
    d
}

async fn trainer_list(a: &mut Client) -> Vec<TrainerSpell> {
    a.clear();
    a.send(CMSG_TRAINER_LIST {
        guid: guid::unit(TRAINER, TRAINER_A),
    })
    .await;
    a.wait_for("SMSG_TRAINER_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_TRAINER_LIST(l) => Some(l.spells.clone()),
        _ => None,
    })
    .await
}

#[tokio::test]
async fn a_trainers_list_greys_what_is_known_reds_what_wants_a_spell_or_skill_and_fits_class_and_race()
 {
    let realm = harness::realm_with(with_trainer(data()));
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(guid::unit(TRAINER, TRAINER_A)).await;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.spells.push(KNOWN);
        // Journeyman mining: its apprentice rank is behind it.
        crate::game::map::skills::learn_skill(p, 186, 2);
        crate::game::map::skills::learn_skill(p, 185, 1);
        crate::game::map::skills::set_value(p, 185, 50);
    })
    .await;
    let list = trainer_list(&mut a).await;
    let state = |list: &[TrainerSpell], id| list.iter().find(|s| s.spell == id).map(|s| s.state);
    assert_eq!(state(&list, KNOWN), Some(TrainerSpellState::Gray));
    assert_eq!(
        state(&list, APPRENTICE_MINING),
        Some(TrainerSpellState::Gray)
    );
    assert_eq!(state(&list, WANTS_SPELL), Some(TrainerSpellState::Red));
    assert_eq!(state(&list, WANTS_SKILL), Some(TrainerSpellState::Red));
    assert_eq!(state(&list, WANTS_COOKING), Some(TrainerSpellState::Green));
    // The masks: a mage's spell isn't for a warrior, an orc's not for a
    // human; either of two lines will do, and no lines is anyone's.
    assert_eq!(state(&list, FOR_MAGES), None);
    assert_eq!(state(&list, FOR_ORCS), None);
    assert_eq!(state(&list, FOR_HUMANS), Some(TrainerSpellState::Green));
    assert_eq!(state(&list, FOR_EITHER), Some(TrainerSpellState::Green));
    assert_eq!(state(&list, FOR_ALL), Some(TrainerSpellState::Green));
    let fits = |spell, class, race| {
        crate::game::map::npc::fits_class_race(&realm.data, spell, class, race)
    };
    assert!(fits(FOR_MAGES, 8, 1) && fits(FOR_HUMANS, 8, 1) && !fits(FOR_HUMANS, 8, 2));
    assert!(fits(FOR_EITHER, 8, 1) && fits(FOR_EITHER, 1, 2) && fits(FOR_ALL, 1, 2));
    // The spell it wants known, it is green; bought, grey.
    on_map(&realm, move |m| {
        m.players.get_mut(&ga).unwrap().spells.push(WANTED)
    })
    .await;
    let list = trainer_list(&mut a).await;
    assert_eq!(state(&list, WANTS_SPELL), Some(TrainerSpellState::Green));
}

#[tokio::test]
async fn a_talent_reset_wants_a_class_trainer_and_level_ten_and_a_failed_payment_changes_nothing() {
    let realm = harness::realm_with(with_trainer(with_talents(data())));
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let trainer = guid::unit(TRAINER, TRAINER_A);
    a.sees(trainer).await;
    let history = async |realm: &Arc<Realm>| {
        on_map(realm, move |m| {
            let p = &m.players[&ga];
            (
                p.row.state.talent_wipes,
                p.spells.contains(&TALENT_SPELL),
                p.money(),
            )
        })
        .await
    };
    let confirm = async |a: &mut Client| {
        a.clear();
        a.send(MSG_TALENT_WIPE_CONFIRM_Client {
            wiping_npc: trainer,
        })
        .await;
        synced(a).await;
    };
    let wipe_reply = |a: &mut Client| {
        a.backlog.iter().find_map(|m| match m {
            ServerOpcodeMessage::MSG_TALENT_WIPE_CONFIRM(c) => {
                Some((c.wiping_npc, c.cost_in_copper))
            }
            _ => None,
        })
    };
    let buy_failed = |a: &mut Client| {
        a.backlog.iter().find_map(|m| match m {
            ServerOpcodeMessage::SMSG_BUY_FAILED(f) => Some(f.result),
            _ => None,
        })
    };
    // Level one: not offered, not done.
    on_map(&realm, move |m| {
        crate::game::map::spells::learn(m, ga, TALENT_SPELL);
        m.players.get_mut(&ga).unwrap().set_money(5_000);
        crate::game::map::misc::talent_wipe_offer(m, ga, trainer);
    })
    .await;
    synced(&mut a).await;
    assert_eq!(wipe_reply(&mut a), None);
    confirm(&mut a).await;
    assert_eq!(wipe_reply(&mut a), None);
    assert_eq!(history(&realm).await, (0, true, 5_000));
    // Level ten, with too little: a failed purchase, nothing reset.
    on_map(&realm, move |m| {
        let data = m.realm.data.clone();
        stats::set_level(m.players.get_mut(&ga).unwrap(), &data, 10);
        crate::game::map::misc::talent_wipe_offer(m, ga, trainer);
    })
    .await;
    synced(&mut a).await;
    assert_eq!(wipe_reply(&mut a), Some((trainer, 10_000)));
    confirm(&mut a).await;
    assert_eq!(buy_failed(&mut a), Some(BuyResult::NotEnoughtMoney));
    assert_eq!(wipe_reply(&mut a), Some((Guid::zero(), 0)));
    assert_eq!(history(&realm).await, (0, true, 5_000));
    // Paid, it is done.
    on_map(&realm, move |m| {
        m.players.get_mut(&ga).unwrap().set_money(15_000)
    })
    .await;
    confirm(&mut a).await;
    assert_eq!(history(&realm).await, (10_000, false, 5_000));
    // Nothing left to reset: told so, not charged.
    confirm(&mut a).await;
    assert_eq!(buy_failed(&mut a), None);
    assert_eq!(wipe_reply(&mut a), Some((Guid::zero(), 0)));
    assert_eq!(history(&realm).await, (10_000, false, 5_000));
}
