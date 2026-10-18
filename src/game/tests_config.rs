//! The config's tunables, honoured: a kill's XP at `[rates] xp_kill`,
//! drops at their quality's rate, a corpse's decay and a respawn at their
//! settings, the aggro range at `[creatures] aggro_rate`, a new
//! character's level and money; and a reload that changes a live rate
//! and leaves what needs a restart, saying so.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::config::ServerConfig;
use crate::game::data::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;
use crate::game::map::creature::Life;
use crate::game::{Realm, RealmConfig};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const WOLF: u32 = 2000;
const BOAR: u32 = 2001;

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

/// A hostile wolf 60 yards east of the start, a neutral boar 3 yards east.
fn data() -> GameData {
    let mut d = GameData::empty();
    d.faction_templates.extend([
        faction(1, 3, 2, 12),
        faction(2, 5, 4, 10),
        faction(14, 8, 0, 1),
        faction(32, 0, 0, 0),
    ]);
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
        .insert(WOLF, template(WOLF, "Wolf", 14));
    d.creature_templates
        .insert(BOAR, template(BOAR, "Boar", 32));
    let spawn = |guid, entry, dx: f32| CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 60,
        respawn_max: 60,
        ..Default::default()
    };
    d.creature_spawns
        .insert(0, vec![spawn(10, WOLF, 60.0), spawn(11, BOAR, 3.0)]);
    d
}

fn boar() -> Guid {
    guid::unit(BOAR, 11)
}

fn realm_with(cfg: RealmConfig) -> Arc<Realm> {
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, data(), None)
}

/// The XP `ga` is given for killing the boar.
async fn kill_xp(realm: &Arc<Realm>, a: &mut Client, ga: Guid) -> u32 {
    let boar = boar();
    a.sees(boar).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    map.call(move |m| crate::game::map::combat::kill(m, ga, boar));
    a.wait_for("SMSG_LOG_XPGAIN", |m| match m {
        ServerOpcodeMessage::SMSG_LOG_XPGAIN(x) => Some(x.total_exp),
        _ => None,
    })
    .await
}

#[tokio::test]
async fn xp_kill_two_doubles_a_kills_xp() {
    let realm = realm_with(harness::test_config());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let base = kill_xp(&realm, &mut a, ga).await;
    assert!(base > 0);

    let mut cfg = harness::test_config();
    cfg.rates.xp_kill = 2.0;
    let realm = realm_with(cfg);
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    assert_eq!(kill_xp(&realm, &mut b, gb).await, 2 * base);
}

/// `.reload config`'s work: a live rate is in at the maps' next update,
/// what needs a restart is left as it was, and the report says which.
#[tokio::test]
async fn a_reload_changes_a_live_rate_and_leaves_what_needs_a_restart() {
    let running = ServerConfig::default();
    let mut cfg = RealmConfig::from_config(&running);
    cfg.everyone_gm = true;
    let realm = realm_with(cfg);
    realm.set_source(running.clone());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));

    let mut new = running.clone();
    new.rates.xp_kill = 2.0;
    new.network.max_packets_per_sec = 5000;
    new.world.characters_db = "elsewhere.sqlite".into();
    let report = realm.reload(crate::config::Loaded {
        config: new,
        warnings: Vec::new(),
    });
    assert_eq!(report.applied, vec!["[rates] xp_kill".to_owned()]);
    assert_eq!(
        report.restart,
        vec![
            "[network] max_packets_per_sec".to_owned(),
            "[world] characters_db".to_owned()
        ]
    );
    let text = report.to_string();
    assert!(
        text.contains("[rates] xp_kill") && text.contains("Needs a restart: [network]"),
        "{text}"
    );
    assert_eq!(realm.config().rates.xp_kill, 2.0);
    assert_eq!(
        realm.config().max_packets_per_sec,
        100,
        "left for a restart"
    );
    // The map takes it at its next update.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let seen = map.query(|m| m.cfg.rates.xp_kill).await;
        if seen == Some(2.0) {
            break;
        }
        assert!(Instant::now() < deadline, "the map never took the reload");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // A level-one human's kill of a level-two boar is 52 XP, doubled.
    assert_eq!(kill_xp(&realm, &mut a, ga).await, 104);
    // Reloading the same again changes nothing.
    let again = realm.reload(crate::config::Loaded {
        config: {
            let mut c = running.clone();
            c.rates.xp_kill = 2.0;
            c
        },
        warnings: Vec::new(),
    });
    assert!(
        again.applied.is_empty() && again.restart.is_empty(),
        "{again:?}"
    );
}

/// A corpse lies `[creatures] corpse_decay_normal`; a respawn takes
/// `[rates] respawn_time` of its spawn's time.
#[tokio::test]
async fn the_corpse_decay_and_the_respawn_time_are_the_configs() {
    let mut cfg = harness::test_config();
    cfg.creatures.corpse_decay_normal = 20;
    cfg.rates.respawn_time = 0.5;
    let realm = realm_with(cfg);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let boar = boar();
    a.sees(boar).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let life = map
        .query(move |m| {
            crate::game::map::ai::die(m, boar, ga);
            let now = m.now;
            m.creature(boar).map(|c| match c.life {
                Life::Dead {
                    corpse_until,
                    respawn_at,
                } => Some((corpse_until - now, respawn_at - now)),
                _ => None,
            })
        })
        .await
        .flatten()
        .flatten();
    // Its spawn's minute, halved; its corpse no longer than that.
    assert_eq!(
        life,
        Some((Duration::from_secs(20), Duration::from_secs(30)))
    );
}

/// `[creatures] aggro_rate` scales the distance a creature notices a
/// player at.
#[tokio::test]
async fn the_aggro_range_is_at_the_configs_rate() {
    let reach = |rate: f32| async move {
        let mut cfg = harness::test_config();
        cfg.creatures.aggro_rate = rate;
        let realm = realm_with(cfg);
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
        let wolf = guid::unit(WOLF, 10);
        a.sees(wolf).await;
        let map = realm.maps.get(&realm, MapKey::world(0));
        map.query(move |m| crate::game::map::ai::attack_distance(m, wolf, ga))
            .await
            .unwrap()
    };
    let base = reach(1.0).await;
    assert!(base > 0.0);
    assert_eq!(reach(1.5).await, base * 1.5);
}

/// Drops at their quality's rate: a grey item at a half chance, never at
/// a rate of 0, always at 2.
#[test]
fn drops_are_at_their_qualitys_rate() {
    use crate::game::map::loot::roll_rated;
    let data = GameData::empty();
    let table = [LootEntry {
        item: 1,
        chance: 50.0,
        group: 0,
        min: 1,
        max: 1,
        condition: 0,
    }];
    let drops = |rates: &crate::config::RatesConfig| {
        (0..200)
            .filter(|_| {
                let mut out = Vec::new();
                roll_rated(
                    &data,
                    &table,
                    &[],
                    &mut rand::rng(),
                    &mut out,
                    0,
                    &|_| false,
                    rates,
                );
                !out.is_empty()
            })
            .count()
    };
    let mut rates = crate::config::RatesConfig {
        drop_item_poor: 0.0,
        ..Default::default()
    };
    assert_eq!(drops(&rates), 0);
    rates.drop_item_poor = 2.0;
    assert_eq!(drops(&rates), 200);
    // Another quality's rate leaves it as it is.
    rates.drop_item_poor = 1.0;
    rates.drop_item_epic = 0.0;
    let n = drops(&rates);
    assert!((40..160).contains(&n), "{n} of 200 at a half chance");
}

/// A new character starts at `[players] start_level`, with `start_money`.
#[tokio::test]
async fn a_new_character_starts_at_the_configs_level_and_money() {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    cfg.players.start_level = 10;
    cfg.players.start_money = 12_345;
    let realm = realm_with(cfg);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let got = map
        .query(move |m| m.players.get(&ga).map(|p| (p.level(), p.money())))
        .await
        .flatten();
    assert_eq!(got, Some((10, 12_345)));
}
