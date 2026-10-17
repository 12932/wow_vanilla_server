//! Gurubashi Arena's last man standing, through the protocol.

use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::items::ItemTemplate;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::arena::{self, CENTRE};
use crate::game::map::death::Death;
use crate::game::map::{MapHandle, MapKey};

const TRINKET: u32 = 19024;

fn data() -> GameData {
    let mut d = GameData::empty();
    d.items.insert(
        TRINKET,
        ItemTemplate {
            entry: TRINKET,
            name: "Arena Grand Master".into(),
            stackable: 1,
            ..Default::default()
        },
    );
    // The ring's burn, as a debuff to see.
    d.spells.spells.insert(
        arena::SPELL_RING_BURN,
        crate::game::data::spells::SpellTemplate {
            id: arena::SPELL_RING_BURN,
            name: "Engulfing Flames".into(),
            attributes: [crate::game::data::spells::ATTR_DEBUFF, 0, 0, 0, 0],
            ..Default::default()
        },
    );
    d
}

/// A fighter, logged in and stood in the pit `off` yards east of its
/// centre.
async fn fighter(
    realm: &std::sync::Arc<crate::game::Realm>,
    name: &str,
    off: f32,
) -> (Client, Guid) {
    let mut c = Client::connect(realm, name);
    let g = c.enter(name, Race::Human, Class::Warrior).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    map.query(move |m| {
        let at = Vector3d {
            x: CENTRE.x + off,
            y: CENTRE.y,
            z: CENTRE.z,
        };
        m.teleport(g, m.key, at, 0.0);
    })
    .await;
    (c, g)
}

/// Waits for a yell or emote of the announcer's containing `text`.
async fn hears(c: &mut Client, text: &str) -> String {
    let text = text.to_owned();
    c.wait_for("the announcer", move |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(chat) if chat.message.contains(&text) => {
            Some(chat.message.clone())
        }
        _ => None,
    })
    .await
}

fn arena_map(realm: &std::sync::Arc<crate::game::Realm>) -> MapHandle {
    realm.maps.get(realm, MapKey::world(0))
}

#[tokio::test]
async fn the_last_one_standing_is_crowned_and_the_fallen_stay_down_until_then() {
    let realm = harness::realm_with(data());
    let (mut a, alice) = fighter(&realm, "Alice", 0.0).await;
    let (mut b, bob) = fighter(&realm, "Bob", 3.0).await;
    let (_c, cora) = fighter(&realm, "Cora", -3.0).await;
    let map = arena_map(&realm);
    a.say(".arena start 0").await;
    hears(&mut a, "3 fighters stand").await;
    hears(&mut a, "FIGHT").await;
    // Bob falls to Alice: called, and out.
    map.query(move |m| crate::game::map::combat::kill(m, alice, bob))
        .await;
    let line = hears(&mut b, "has slain Bob").await;
    assert_eq!(line, "Alice has slain Bob! 2 fighters remain.");
    // No coming back while it lasts.
    let still_dead = map
        .query(move |m| {
            crate::game::map::death::resurrect(m, bob, 1.0);
            m.players[&bob].death != Death::Alive
        })
        .await
        .unwrap();
    assert!(still_dead, "a fallen fighter can't be raised mid-battle");
    // His ghost goes up to the stands.
    let up = map
        .query(move |m| {
            crate::game::map::death::release(m, bob);
            let p = m.players[&bob].position();
            ((p.x - CENTRE.x).powi(2) + (p.y - CENTRE.y).powi(2)).sqrt()
        })
        .await
        .unwrap();
    assert!(
        up > arena::PIT_RADIUS,
        "Bob watches from the stands, {up} yards out"
    );
    let money_before = map
        .query(move |m| m.players[&alice].fields.get_u32(PLAYER_FIELD_COINAGE))
        .await
        .unwrap();
    map.query(move |m| crate::game::map::combat::kill(m, alice, cora))
        .await;
    let crowned = hears(&mut a, "Champion of Gurubashi Arena").await;
    assert!(
        crowned.starts_with("Alice the Human Warrior is the Champion"),
        "{crowned}"
    );
    a.system_message("The standings of Gurubashi Arena").await;
    let first = a.system_message("1. Alice").await;
    assert!(
        first.starts_with("1. Alice (Human Warrior): 2 kills"),
        "{first}"
    );
    let third = a.system_message("3. Bob").await;
    assert!(third.ends_with("slain by Alice"), "{third}");
    let (money, trinket, bob_alive) = map
        .query(move |m| {
            let p = &m.players[&alice];
            (
                p.fields.get_u32(PLAYER_FIELD_COINAGE),
                p.items.iter().any(|i| i.entry == TRINKET),
                m.players[&bob].death == Death::Alive,
            )
        })
        .await
        .unwrap();
    assert_eq!(money - money_before, 250_000, "the champion's gold");
    assert!(trinket, "the champion's trinket");
    assert!(bob_alive, "the fallen rise when it ends");
    let history = realm.social.arena.lock().results.clone();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].champion.as_deref(), Some("Alice"));
    assert_eq!(history[0].fighters, 3);
    a.say(".arena history").await;
    a.system_message("last champions").await;
    let h = a.system_message("Alice the Human Warrior").await;
    assert!(h.contains("of 3 fighters"), "{h}");
}

#[tokio::test]
async fn the_ring_of_fire_burns_whoever_waits_outside_it() {
    let realm = harness::realm_with(data());
    let (mut a, alice) = fighter(&realm, "Alice", 0.0).await;
    let (mut b, bob) = fighter(&realm, "Bob", 20.0).await;
    let map = arena_map(&realm);
    a.say(".arena start 0").await;
    hears(&mut a, "FIGHT").await;
    // The ring at its smallest and hottest, with Bob far outside.
    map.query(|m| {
        if let Some(b) = m.arena.battle.as_mut() {
            b.ring = 6.0;
            b.ring_steps = 10;
            b.next_shrink = None;
        }
    })
    .await;
    b.wait_for("the ring's burn", |m| match m {
        ServerOpcodeMessage::SMSG_ENVIRONMENTAL_DAMAGE_LOG(d)
            if d.damage_type == EnvironmentalDamageType::Fire =>
        {
            Some(())
        }
        _ => None,
    })
    .await;
    let burning = map
        .query(move |m| m.players[&bob].auras.has_spell(arena::SPELL_RING_BURN))
        .await
        .unwrap();
    assert!(burning, "the burn shows on Bob");
    let line = a
        .gets(Duration::from_secs(10), |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(chat) if chat.message.contains("consumed") => {
                Some(chat.message.clone())
            }
            _ => None,
        })
        .await
        .expect("Bob burns to death within ten seconds");
    assert_eq!(line, "Bob was consumed by the flames! 1 fighters remain.");
    hears(&mut a, "Alice the Human Warrior is the Champion").await;
    let alice_hurt = map
        .query(move |m| {
            let f = &m.players[&alice].fields;
            f.get_u32(UNIT_FIELD_HEALTH) < f.get_u32(UNIT_FIELD_MAXHEALTH)
        })
        .await
        .unwrap();
    assert!(!alice_hurt, "the centre doesn't burn");
}

#[tokio::test]
async fn no_blows_before_the_fight_and_spectators_keep_off_the_floor() {
    let realm = harness::realm_with(data());
    let (mut a, alice) = fighter(&realm, "Alice", 0.0).await;
    let (_b, bob) = fighter(&realm, "Bob", 3.0).await;
    let map = arena_map(&realm);
    a.say(".arena start 30").await;
    hears(&mut a, "2 fighters stand").await;
    let before = map
        .query(move |m| crate::game::map::combat::can_attack(m, alice, bob))
        .await
        .unwrap();
    assert!(!before, "no blows in the countdown");
    a.say(".arena status").await;
    let status = a.system_message("fighters wait").await;
    assert!(status.starts_with("2 fighters wait"), "{status}");
    // Dana comes down onto the floor: she isn't in it, and goes up.
    let (_d, dana) = fighter(&realm, "Dana", 5.0).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let out = map
        .query(move |m| {
            let p = m.players[&dana].position();
            ((p.x - CENTRE.x).powi(2) + (p.y - CENTRE.y).powi(2)).sqrt()
        })
        .await
        .unwrap();
    assert!(
        out > arena::PIT_RADIUS,
        "Dana went up to the stands, {out} yards out"
    );
    a.say(".arena stop").await;
    hears(&mut a, "called off").await;
    assert!(map.query(|m| m.arena.battle.is_none()).await.unwrap());
}

#[tokio::test]
async fn too_few_fighters_make_no_battle() {
    let realm = harness::realm_with(data());
    let (mut a, _alice) = fighter(&realm, "Alice", 0.0).await;
    a.say(".arena start 0").await;
    let refused = a.system_message("needs 2").await;
    assert!(
        refused.starts_with("1 fighters stand in the pit"),
        "{refused}"
    );
}

#[tokio::test]
async fn the_booty_runs_chest_falls_with_its_event_and_not_before() {
    let mut d = data();
    d.gameobject_templates.insert(
        179697,
        crate::game::data::GameObjectTemplate {
            entry: 179697,
            kind: 3,
            display_id: 1,
            size: 1.0,
            name: "Arena Treasure Chest".into(),
            ..Default::default()
        },
    );
    d.gameobject_spawns
        .entry(0)
        .or_default()
        .push(crate::game::data::GameObjectSpawn {
            guid: arena::BOOTY_CHEST_SPAWN,
            entry: 179697,
            map: 0,
            x: CENTRE.x,
            y: CENTRE.y,
            z: CENTRE.z,
            state: 1,
            despawned: true,
            ..Default::default()
        });
    let realm = harness::realm_with(d);
    let (_a, _alice) = fighter(&realm, "Alice", 0.0).await;
    let map = arena_map(&realm);
    let chest = |map: MapHandle| async move {
        map.query(|m| {
            m.gameobjects
                .iter()
                .any(|(_, g)| g.spawn_guid == Some(arena::BOOTY_CHEST_SPAWN))
        })
        .await
        .unwrap()
    };
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(!chest(map.clone()).await, "no chest before the Booty Run");
    realm.social.events.lock().insert(arena::BOOTY_RUN_EVENT);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        chest(map.clone()).await,
        "the chest falls with the Booty Run"
    );
    realm.social.events.lock().remove(&arena::BOOTY_RUN_EVENT);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(!chest(map.clone()).await, "and goes with it");
}

/// The pit's floor and stands, from the navmesh (run by hand).
#[test]
#[ignore]
fn probe_the_pit() {
    let dir = &crate::game::harness::real_data("nav");
    let Some(mut nav) = crate::game::map::nav::Nav::open(dir, "Azeroth") else {
        return;
    };
    for angle in (0..360).step_by(30) {
        let a = (angle as f32).to_radians();
        let mut line = format!("{angle:3}:");
        for r in (0..=80).step_by(5) {
            let (x, y) = (CENTRE.x + r as f32 * a.cos(), CENTRE.y + r as f32 * a.sin());
            let z = nav.ground(x, y, 22.0).unwrap_or(-1.0);
            let z40 = nav.ground(x, y, 40.0).unwrap_or(-1.0);
            line += &format!(" {r}:{z:.0}/{z40:.0}");
        }
        println!("{line}");
    }
    println!("centre area {:?}", nav.zone_and_area(CENTRE));
}

/// The shipped config's `[arena]` reads.
#[test]
fn the_shipped_config_has_the_arena() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config.toml");
    let cfg = crate::config::load(&path).expect("config.toml parses");
    assert_eq!(cfg.arena.countdown_secs, 30);
    assert_eq!(cfg.arena.reward_item, 19024);
    assert_eq!(
        cfg.arena.every_minutes, 0,
        "no scheduled battles by default"
    );
}
