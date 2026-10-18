//! The whole world awake, round two (docs/PERFORMANCE.md): the 500 ms
//! timers each on a phase of its own, and the awake set pruned once an
//! update.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::guid;
use crate::game::harness::Client;
use crate::game::map::{Map, ai, environment, next_beat, visibility};
use crate::game::tests_p3_combat::*;

/// How many of `waits` fall in each fifth of `every`, (0, every].
fn fifths(waits: &[Duration], every: Duration) -> [usize; 5] {
    let mut bins = [0; 5];
    for w in waits {
        assert!(
            !w.is_zero() && *w <= every,
            "{w:?}: due again within the period"
        );
        let i = ((w.as_secs_f64() / every.as_secs_f64()) * 5.0).ceil() as usize;
        bins[i.clamp(1, 5) - 1] += 1;
    }
    bins
}

#[test]
fn a_timers_beats_keep_its_period_each_on_a_phase_of_its_own() {
    let every = Duration::from_millis(500);
    let start = Instant::now();
    let now = start + Duration::from_millis(7_321);
    let guids: Vec<Guid> = (1..=100).map(|i| guid::unit(BRUTE, i)).collect();
    let next: Vec<Instant> = guids
        .iter()
        .map(|&g| next_beat(start, g, every, now))
        .collect();
    let waits: Vec<Duration> = next.iter().map(|n| *n - now).collect();
    let bins = fifths(&waits, every);
    assert!(
        bins.iter().all(|&b| b >= 5),
        "spread over the period: {bins:?}"
    );
    // From a beat, the next is a period on: the period is kept.
    for (&g, &n) in guids.iter().zip(&next) {
        assert_eq!(next_beat(start, g, every, n), n + every);
    }
    // Before its first beat, that is the next.
    let g = guids[0];
    let first = next_beat(start, g, every, start);
    assert!(first > start && first <= start + every);
    assert_eq!(
        next_beat(start, g, every, first - Duration::from_millis(1)),
        first
    );
}

/// A spawn `yards` east of the start, its aggro check due now, seen.
fn seen_brute(m: &mut Map, ga: Guid, yards: f32) -> Guid {
    let g = spawn(m, ga, BRUTE, 20, yards);
    visibility::refresh(m, g);
    let now = m.now;
    m.creature_mut(g).unwrap().aggro_check_at = now;
    g
}

#[tokio::test]
async fn creatures_that_came_into_view_together_look_again_each_on_a_phase_of_its_own() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (waits, observed) = map
        .query(move |m| {
            let gs: Vec<Guid> = (0..40)
                .map(|i| seen_brute(m, ga, 30.0 + i as f32 * 0.5))
                .collect();
            let now = m.now;
            ai::update_creatures(m);
            let waits: Vec<Duration> = gs
                .iter()
                .map(|&g| m.creature(g).unwrap().aggro_check_at - now)
                .collect();
            let observed = gs
                .iter()
                .all(|&g| !m.creature(g).unwrap().observers.is_empty());
            (waits, observed)
        })
        .await
        .unwrap();
    assert!(observed, "every one is seen, so each looked");
    // They looked together; they look again spread over the next 500 ms,
    // not all in one update 500 ms on.
    let bins = fifths(&waits, Duration::from_millis(500));
    assert!(bins.iter().all(|&b| b <= 20), "spread: {bins:?}");
}

#[tokio::test]
async fn players_environment_checks_come_each_on_a_phase_of_its_own() {
    let (realm, _a, ga, map) = setup(Class::Warrior).await;
    let mut clients = Vec::new();
    let mut guids = vec![ga];
    for i in 0..9 {
        let name = format!("Pl{}", (b'a' + i) as char);
        let mut c = Client::connect(&realm, &name);
        guids.push(c.enter(&name, Race::Human, Class::Warrior).await);
        c.heartbeat(START.0, START.1, START.2).await;
        clients.push(c);
    }
    let waits = map
        .query(move |m| {
            for g in &guids {
                m.players.get_mut(g).unwrap().misc.env_checked_at = None;
            }
            let now = m.now;
            environment::update(m);
            guids
                .iter()
                .map(|g| {
                    let checked = m.players[g].misc.env_checked_at.unwrap();
                    assert!(checked <= now, "checked as of now or before");
                    checked + Duration::from_millis(500) - now
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    // Checked together, each is due again on its own phase.
    let bins = fifths(&waits, Duration::from_millis(500));
    assert!(bins.iter().all(|&b| b <= 7), "spread: {bins:?}");
}

/// The awake creatures a look finds, as `Map::awake_where` answers (which
/// debug builds check against a look through every creature).
fn awake_now(m: &mut Map) -> Vec<Guid> {
    m.awake_where(crate::game::map::awake, |_, c| c.guid)
}

#[tokio::test]
async fn a_creature_woken_between_an_updates_looks_is_found_by_the_later_ones() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    map.query(move |m| {
        // Out of sight, and asleep.
        let far = spawn(m, ga, BRUTE, 20, 400.0);
        let seen = seen_brute(m, ga, 30.0);
        m.stats.updates += 1;
        // The update's first look prunes the set.
        assert_eq!(awake_now(m), vec![seen]);
        assert!(!m.awake.contains(&far));
        // Another's turn draws it into a fight, unseen: attacked, not yet
        // fighting back.
        m.combat_mut(far).unwrap().attackers.insert(ga);
        let fighting = m.awake_where(
            |c| c.alive() && (c.combat.attacking || !c.combat.attackers.is_empty()),
            |_, c| c.guid,
        );
        assert_eq!(fighting, vec![far]);
        assert_eq!(awake_now(m).len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_creature_gone_and_its_key_taken_or_back_under_another_key_is_found_as_it_now_is() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    map.query(move |m| {
        let gone = seen_brute(m, ga, 30.0);
        m.stats.updates += 1;
        assert_eq!(awake_now(m), vec![gone]);
        let key = m.creature_by_guid[&gone];
        let mut back = m.remove_creature(gone).unwrap();
        // Its key is another's now, one asleep: not awake, it isn't found.
        let other = spawn(m, ga, BRUTE, 20, 400.0);
        assert_eq!(
            m.creature_by_guid[&other], key,
            "the slab gave its key again"
        );
        assert!(awake_now(m).is_empty());
        // Woken, it is.
        m.note_fighting(other);
        m.creature_mut(other).unwrap().combat.attacking = true;
        assert_eq!(awake_now(m), vec![other]);
        // The first comes back, fighting, under a key of its own.
        back.combat.attacking = true;
        m.add_creature(back);
        assert_ne!(m.creature_by_guid[&gone], key);
        let mut both = vec![other, gone];
        both.sort_by_key(|g| m.creature_by_guid[g]);
        assert_eq!(awake_now(m), both);
        // A creature that was a member, back asleep, is found once woken.
        let mut again = m.remove_creature(gone).unwrap();
        again.combat.attacking = false;
        m.add_creature(again);
        assert_eq!(awake_now(m), vec![other]);
        m.note_fighting(gone);
        m.creature_mut(gone).unwrap().combat.attacking = true;
        assert_eq!(awake_now(m).len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn the_awake_set_lets_go_of_those_no_longer_awake_at_the_next_updates_first_look() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    map.query(move |m| {
        let g = spawn(m, ga, BRUTE, 20, 400.0);
        m.note_fighting(g);
        m.creature_mut(g).unwrap().combat.attacking = true;
        m.stats.updates += 1;
        assert_eq!(awake_now(m), vec![g]);
        m.creature_mut(g).unwrap().combat.attacking = false;
        assert!(awake_now(m).is_empty(), "asleep, it isn't found");
        m.stats.updates += 1;
        assert!(awake_now(m).is_empty());
        assert!(!m.awake.contains(&g), "and is let go");
    })
    .await
    .unwrap();
}

static BAKE: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("nav"));
static TERRAIN: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("terrain"));

/// Goldshire's road, and a lake south of it (as `tests_terrain` has them).
const GOLDSHIRE: Vector3d = Vector3d {
    x: -9460.0,
    y: 62.0,
    z: 56.0,
};
const ELWYNN_LAKE: Vector3d = Vector3d {
    x: -9640.0,
    y: -1040.0,
    z: 37.0,
};

/// Until `done`, in real time: what the readers' threads are reading takes
/// the disk's time.
fn until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < deadline, "{what} within 60 s");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "extended"]
fn a_navmesh_tile_read_off_the_maps_thread_answers_as_no_navmesh_until_it_is_in() {
    use crate::game::map::nav::Nav;
    let (from, to) = (
        GOLDSHIRE,
        Vector3d {
            x: GOLDSHIRE.x + 30.0,
            ..GOLDSHIRE
        },
    );
    let mut at_once = Nav::open(&BAKE, "Azeroth").expect("the bake");
    let (path, ground) = (
        at_once.path(from, to),
        at_once.ground(from.x, from.y, from.z),
    );
    assert!(path.as_ref().is_some_and(|p| p.len() >= 2) && ground.is_some());
    let mut nav = Nav::open(&BAKE, "Azeroth").unwrap();
    nav.in_background();
    // Asked for, it is read meanwhile: a path is straight, a height
    // unknown, and the way clear, as with no navmesh.
    assert_eq!(nav.path(from, to), Some(vec![from, to]));
    assert_eq!(nav.ground(from.x, from.y, from.z), None);
    assert!(nav.line_of_sight(from, to));
    assert_eq!((nav.tiles_in(), nav.tiles_loading()), (0, 1));
    until("the tile", || {
        nav.receive(Instant::now());
        nav.tiles_in() == 1
    });
    // In, it answers as the tile read at once does.
    assert_eq!(nav.path(from, to), path);
    assert_eq!(nav.ground(from.x, from.y, from.z), ground);
}

#[test]
#[ignore = "extended"]
fn a_terrain_tile_read_off_the_maps_thread_has_no_liquid_until_it_is_in() {
    use crate::game::map::terrain::Terrain;
    let mut at_once = Terrain::open(&TERRAIN, 0).expect("the tiles");
    let lake = at_once.liquid(ELWYNN_LAKE);
    assert!(lake.is_some(), "Elwynn's lake");
    let mut terrain = Terrain::open(&TERRAIN, 0).unwrap();
    terrain.in_background();
    assert_eq!(terrain.liquid(ELWYNN_LAKE), None, "being read");
    assert_eq!(terrain.tiles_loading(), 1);
    until("the tile", || {
        terrain.receive(Instant::now());
        terrain.tiles_read() == 1
    });
    assert_eq!(terrain.liquid(ELWYNN_LAKE), lake);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "extended"]
async fn a_map_asks_ahead_for_the_tiles_around_its_players() {
    let mut cfg = crate::game::harness::test_config();
    cfg.nav_dir = Some(BAKE.clone());
    cfg.terrain_dir = Some(TERRAIN.clone());
    cfg.tiles_in_background = true;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(
        cfg,
        store,
        startup,
        crate::game::data::GameData::empty(),
        None,
    );
    let mut a = Client::connect(&realm, "A");
    // In Northshire, where a human starts: 117 yards from the tile to its
    // west, and so within what they see and a margin of it.
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let (here, ahead) = map
        .query(move |m| {
            let p = m.players[&g].position();
            let reach = m.cfg.visibility_yards + 20.0;
            (
                crate::game::map::nav::tile_of(p.x, p.y),
                crate::game::map::tiles::tiles_around(p.x, p.y, reach).to_vec(),
            )
        })
        .await
        .unwrap();
    assert!(ahead.len() >= 2 && ahead.contains(&here), "{ahead:?}");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        // Nothing near them asks about the tile to the west: they see
        // nothing there yet.
        let want = ahead.clone();
        let (nav, terrain) = map
            .query(move |m| {
                (
                    want.iter()
                        .all(|t| m.nav.as_ref().is_some_and(|n| n.has_tile(*t))),
                    m.terrain.as_ref().map_or(0, |t| t.tiles_read()),
                )
            })
            .await
            .unwrap();
        if nav && terrain >= ahead.len() {
            break;
        }
        assert!(Instant::now() < deadline, "the tiles within 60 s");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
