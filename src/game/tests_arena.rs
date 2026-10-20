//! Gurubashi Arena's last man standing, through the protocol.

use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::items::ItemTemplate;
use crate::game::data::{AREA_FLAG_ARENA, AreaInfo, GameData};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::arena::{self, CENTRE};
use crate::game::map::combat::can_attack;
use crate::game::map::death::{self, Death};
use crate::game::map::player::PLAYER_FLAGS_FFA_PVP;
use crate::game::map::{Map, MapHandle, MapKey, pvp};

const TRINKET: u32 = 19024;
/// The arena's stands, around the pit: not free-for-all.
const STANDS: u32 = 1741;

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

/// rev5-data 10: a GM's huge countdown or length is held to the config's
/// ranges, where `now + countdown` panicked the command.
#[tokio::test]
async fn a_huge_countdown_or_length_is_held_to_its_range() {
    let realm = harness::realm_with(data());
    let (mut a, _alice) = fighter(&realm, "Alice", 0.0).await;
    let (_b, _bob) = fighter(&realm, "Bob", 3.0).await;
    a.say(".arena start 18446744073709551615 18446744073709551615")
        .await;
    let on = a.system_message("The battle is on").await;
    assert!(on.contains("3600 s to the fight"), "{on}");
    let map = arena_map(&realm);
    let (fight_in, cap_in) = map
        .query(|m| {
            let b = m.arena.battle.as_ref().expect("a battle");
            let arena::Phase::Countdown { fight_at } = b.phase else {
                panic!("in its countdown");
            };
            (fight_at - m.now, b.cap_at - m.now)
        })
        .await
        .expect("the map");
    let hour = Duration::from_secs(arena::MAX_SECS);
    assert!(
        fight_in <= hour && fight_in > hour - Duration::from_secs(10),
        "{fight_in:?}"
    );
    assert_eq!(
        cap_in - fight_in,
        Duration::from_secs(arena::MAX_MINUTES * 60)
    );
}

/// rev5-data 10: `[arena]`'s times have ranges, so a typo can't panic the
/// arena's step every update.
#[test]
fn the_arenas_times_are_clamped() {
    let l = crate::config::resolve(
        "[arena]\nevery_minutes = 300000000000000000\ncountdown_secs = 99999999\n\
         ring_after_secs = 99999999\nmax_minutes = 300000000000000000\n",
        "t",
        &[],
    )
    .unwrap();
    let a = &l.config.arena;
    assert_eq!(a.every_minutes, arena::MAX_MINUTES);
    assert_eq!(a.max_minutes, arena::MAX_MINUTES);
    assert_eq!(a.countdown_secs, arena::MAX_SECS);
    assert_eq!(a.ring_after_secs, arena::MAX_SECS);
    assert_eq!(l.warnings.len(), 4, "{:?}", l.warnings);
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
#[ignore = "a probe, run by hand"]
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
            let area = nav
                .zone_and_area(Vector3d { x, y, z: z40 })
                .map_or(0, |(_, a)| a);
            line += &format!(" {r}:{z:.0}/{z40:.0}@{area}");
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

/// Two of one side, a Human and a Dwarf, may strike each other once the
/// fight is on: the battle marks its fighters free-for-all, which is what
/// the client goes by, even where the server knows no area (no navmesh or
/// terrain). Before the fight and after it, they are friends again.
#[tokio::test]
async fn fighters_of_one_side_are_free_for_all_for_the_fight() {
    let realm = harness::realm_with(data());
    let (mut a, alice) = fighter(&realm, "Alice", 0.0).await;
    let mut d = Client::connect(&realm, "Dora");
    let dora = d.enter("Dora", Race::Dwarf, Class::Warrior).await;
    let map = arena_map(&realm);
    map.query(move |m| {
        let at = Vector3d {
            x: CENTRE.x + 3.0,
            y: CENTRE.y,
            z: CENTRE.z,
        };
        m.teleport(dora, m.key, at, 0.0);
    })
    .await;
    let flags = move |m: &crate::game::map::Map| {
        [alice, dora].map(|g| {
            m.players[&g]
                .fields
                .has_flag(PLAYER_FLAGS, crate::game::map::player::PLAYER_FLAGS_FFA_PVP)
        })
    };
    let before = map.query(move |m| flags(m)).await.unwrap();
    assert_eq!(before, [false, false], "one side, no area: friends");
    a.say(".arena start 1").await;
    hears(&mut a, "FIGHT").await;
    let (during, may) = map
        .query(move |m| {
            (
                flags(m),
                crate::game::map::combat::can_attack(m, alice, dora),
            )
        })
        .await
        .unwrap();
    assert_eq!(during, [true, true], "both free-for-all for the fight");
    assert!(may, "and the server lets them");
    map.query(move |m| crate::game::map::combat::kill(m, alice, dora))
        .await;
    hears(&mut a, "Champion of Gurubashi Arena").await;
    let after = map.query(move |m| flags(m)).await.unwrap();
    assert_eq!(after, [false, false], "friends again once it's over");
}

/// [`data`] with the pit (an arena's area), its stands and Stranglethorn,
/// as AreaTable has them.
fn with_areas(mut d: GameData) -> GameData {
    for (id, zone, flags) in [
        (arena::AREA_PIT, arena::ZONE_STRANGLETHORN, AREA_FLAG_ARENA),
        (STANDS, arena::ZONE_STRANGLETHORN, 0),
        (arena::ZONE_STRANGLETHORN, 0, 0),
    ] {
        d.areas.insert(
            id,
            AreaInfo {
                id,
                zone,
                flags,
                name: format!("Area {id}"),
                ..Default::default()
            },
        );
    }
    d
}

/// Two of one side, Alice and Bob, logged in.
async fn two(realm: &std::sync::Arc<crate::game::Realm>) -> ([Client; 2], Guid, Guid) {
    let mut a = Client::connect(realm, "Alice");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(realm, "Bob");
    let bob = b.enter("Bob", Race::Human, Class::Warrior).await;
    ([a, b], alice, bob)
}

/// `g` found to stand in `area`, as `world::update_zone` finds it from the
/// navmesh (which the fast suite has none of).
fn stand_in(m: &mut Map, g: Guid, area: u32) {
    m.players.get_mut(&g).unwrap().area = area;
    pvp::free_for_all(m, g);
}

fn ffa(m: &Map, g: Guid) -> bool {
    m.players[&g]
        .fields
        .has_flag(PLAYER_FLAGS, PLAYER_FLAGS_FFA_PVP)
}

/// The map's clock `secs` on, and its PvP step run.
fn later(m: &mut Map, secs: u64) {
    m.now += Duration::from_secs(secs);
    pvp::update(m);
}

/// Out of the pit, Bob is free-for-all twenty seconds more (`[arena]
/// ffa_grace_secs`): Alice, inside, may strike him, and he her, until they
/// run out; then they are of one side again.
#[tokio::test]
async fn stepping_out_of_the_pit_keeps_one_free_for_all_twenty_seconds() {
    let realm = harness::realm_with(with_areas(data()));
    let (_clients, alice, bob) = two(&realm).await;
    let seen = arena_map(&realm)
        .query(move |m| {
            stand_in(m, alice, arena::AREA_PIT);
            stand_in(m, bob, arena::AREA_PIT);
            let inside = can_attack(m, alice, bob);
            stand_in(m, bob, STANDS);
            let out = (can_attack(m, alice, bob), can_attack(m, bob, alice));
            later(m, 19);
            let at_19 = can_attack(m, alice, bob);
            later(m, 2);
            let at_21 = (can_attack(m, alice, bob), can_attack(m, bob, alice));
            (inside, out, at_19, at_21, ffa(m, bob), ffa(m, alice))
        })
        .await
        .unwrap();
    assert!(seen.0, "free for all in the pit");
    assert_eq!(seen.1, (true, true), "just out: still");
    assert!(seen.2, "nineteen seconds out: still");
    assert_eq!(seen.3, (false, false), "twenty-one seconds out: no more");
    assert_eq!(
        (seen.4, seen.5),
        (false, true),
        "Bob's flag off, Alice's on"
    );
}

/// Back in the pit within the twenty seconds, the flag carries on without
/// a gap; out again, it has twenty seconds from then.
#[tokio::test]
async fn stepping_back_into_the_pit_carries_free_for_all_on() {
    let realm = harness::realm_with(with_areas(data()));
    let (_clients, alice, bob) = two(&realm).await;
    let seen = arena_map(&realm)
        .query(move |m| {
            stand_in(m, alice, arena::AREA_PIT);
            stand_in(m, bob, arena::AREA_PIT);
            stand_in(m, bob, STANDS);
            let mut seen = Vec::new();
            later(m, 15);
            seen.push(ffa(m, bob));
            stand_in(m, bob, arena::AREA_PIT);
            seen.push(ffa(m, bob));
            // Twenty-five seconds after first stepping out, in the pit.
            later(m, 10);
            seen.push(ffa(m, bob));
            stand_in(m, bob, STANDS);
            later(m, 19);
            seen.push(can_attack(m, alice, bob));
            later(m, 2);
            seen.push(can_attack(m, alice, bob));
            seen
        })
        .await
        .unwrap();
    assert_eq!(seen, [true, true, true, true, false]);
}

/// A death ends the twenty seconds, as does a teleport away (a
/// hearthstone's); a Blink out of the pit doesn't, as a walk out doesn't.
#[tokio::test]
async fn a_death_or_a_teleport_ends_the_free_for_all_grace() {
    let realm = harness::realm_with(with_areas(data()));
    let (_clients, alice, bob) = two(&realm).await;
    let seen = arena_map(&realm)
        .query(move |m| {
            stand_in(m, alice, arena::AREA_PIT);
            stand_in(m, bob, arena::AREA_PIT);
            stand_in(m, bob, STANDS);
            later(m, 5);
            death::kill(m, bob);
            let dead = ffa(m, bob);
            death::resurrect(m, bob, 1.0);
            let raised = (ffa(m, bob), can_attack(m, alice, bob));
            stand_in(m, bob, arena::AREA_PIT);
            stand_in(m, bob, STANDS);
            let at = m.players[&bob].position();
            m.near_teleport(
                bob,
                Vector3d {
                    x: at.x + 20.0,
                    ..at
                },
                0.0,
            );
            let blinked = can_attack(m, alice, bob);
            m.teleport(
                bob,
                m.key,
                Vector3d {
                    x: at.x + 500.0,
                    ..at
                },
                0.0,
            );
            (dead, raised, blinked, ffa(m, bob))
        })
        .await
        .unwrap();
    assert!(!seen.0, "dead, no longer free for all");
    assert_eq!(seen.1, (false, false), "and not once raised");
    assert!(seen.2, "a Blink keeps it");
    assert!(!seen.3, "a teleport away ends it");
}

/// On the real navmesh (`world::update_zone`): Bob gone up from the pit's
/// floor into the stands is free-for-all twenty seconds, and not after.
#[tokio::test]
#[ignore = "extended"]
async fn the_navmesh_finds_one_out_of_the_pit_and_the_grace_runs() {
    let nav = harness::real_data("nav");
    if !nav.join("Azeroth.map").exists() {
        eprintln!("no navmesh at {}: skipped", nav.display());
        return;
    }
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(nav);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, with_areas(data()), None);
    let (_clients, alice, bob) = two(&realm).await;
    let seen = arena_map(&realm)
        .query(move |m| {
            let put = |m: &mut Map, g: Guid, at: Vector3d| {
                m.players.get_mut(&g).unwrap().movement.position = at;
                crate::game::map::world::update_zone(m, g);
                (m.players[&g].area, ffa(m, g))
            };
            // The stands' rows seventy yards east of the centre.
            let (x, y) = (CENTRE.x + 70.0, CENTRE.y);
            let z = m.nav.as_mut().and_then(|n| n.ground(x, y, 40.0));
            let stands = Vector3d {
                x,
                y,
                z: z.unwrap_or(43.0),
            };
            put(m, alice, CENTRE);
            let floor = put(m, bob, CENTRE);
            let up = put(m, bob, stands);
            let out = can_attack(m, alice, bob);
            later(m, 19);
            let at_19 = can_attack(m, alice, bob);
            later(m, 2);
            (floor, up, out, at_19, can_attack(m, alice, bob))
        })
        .await
        .unwrap();
    assert_eq!(seen.0, (arena::AREA_PIT, true), "on the floor");
    assert_eq!(seen.1, (STANDS, true), "up in the stands, still");
    assert!(seen.2 && seen.3, "struck from the pit for twenty seconds");
    assert!(!seen.4, "and not after");
}
