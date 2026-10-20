//! `test/game/core/movement/{control_movement,fear}_test.exs`, round 8: a
//! control (fear, confusion) back at login with its aura, and a unit with
//! nowhere on the navmesh to run to, which stands and tries again later.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Motion;
use crate::game::map::creature_combat;
use crate::game::map::player_motion::{self, Mode};
use crate::game::map::{Map, MapKey};
use crate::game::tests_parity_motion as motion;

/// tests_parity_motion's spells: fear and confusion, ten seconds each.
const FEAR: u32 = 92_001;
const CONFUSE: u32 = 92_002;

/// Whether `c` was told its client's control, given or taken.
fn control(m: &ServerOpcodeMessage, g: Guid) -> Option<bool> {
    match m {
        ServerOpcodeMessage::SMSG_CLIENT_CONTROL_UPDATE(u) if u.guid == g => Some(u.allow_movement),
        _ => None,
    }
}

/// A confused player who logs out and back in is confused still, and the
/// server moves her again: her client is told it has no control, and her
/// confusion is about where she came back (Thistle Tea's `restore/2`,
/// "rebuilds active control after behavior initialization").
#[tokio::test]
async fn a_control_comes_back_with_its_aura_at_login() {
    let (realm, mut a, ga, map) = motion::enter(motion::wolf_world()).await;
    let wolf = motion::wolf();
    a.sees(wolf).await;
    map.query(move |m| motion::apply(m, wolf, ga, CONFUSE))
        .await
        .unwrap();
    assert!(!a.wait_for("control taken", |m| control(m, ga)).await);
    // Gone (a disconnect saves her), and back.
    let account = a.account.clone();
    a.disconnect();
    for _ in 0..500 {
        if realm.registry.online(ga).is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(realm.registry.online(ga).is_none(), "logged out");
    realm.store.flush().await;
    let mut a = Client::connect(&realm, &account);
    a.login(ga).await;
    assert!(
        !a.wait_for("control taken again", |m| control(m, ga)).await,
        "her client has no control"
    );
    let (mode, at) = map
        .query(move |m| {
            (
                player_motion::driven_by(m, ga),
                m.players.get(&ga).map(|p| p.position()),
            )
        })
        .await
        .unwrap();
    let at = at.expect("in the world");
    assert_eq!(
        mode,
        Some(Mode::Confusion { anchor: at }),
        "confused, about where she is"
    );
}

/// [`motion::wolf_world`] on the real navmesh, or `None` without it.
fn nav_realm() -> Option<Arc<Realm>> {
    let nav = harness::real_data("nav");
    if !nav.join("Azeroth.map").exists() {
        eprintln!("no navmesh at {}: skipped", nav.display());
        return None;
    }
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(nav);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Some(Realm::new(cfg, store, startup, motion::wolf_world(), None))
}

/// So high over Northshire's ground that the navmesh has nothing near.
const UP: f32 = 60.0;

fn up(p: Vector3d) -> Vector3d {
    Vector3d {
        x: p.x,
        y: p.y,
        z: p.z + UP,
    }
}

/// The wolf's walk under way, if any.
fn walking(m: &Map, g: Guid) -> bool {
    matches!(m.creature(g).map(|c| &c.motion), Some(Motion::Moving(_)))
}

/// A feared or confused unit with no point of the navmesh to run to (high
/// in the air over it) stands, its control kept, and looks again only
/// after a pause: a second for a fear, half for a confusion, as Thistle
/// Tea's "retries unavailable navigation without attacking or spinning"
/// and "failed navigation retries without a busy loop" have it. Before
/// then a look does nothing (no run, no turn, no swing); after, it looks
/// again and waits again.
#[tokio::test]
#[ignore = "extended"]
async fn a_unit_with_nowhere_to_run_stands_and_tries_again_later() {
    let Some(realm) = nav_realm() else {
        return;
    };
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(motion::START.0, motion::START.1, motion::START.2)
        .await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let wolf = motion::wolf();
    a.sees(wolf).await;
    // The wolf, in a fight with Alice, high in the air; then feared by her.
    map.query(move |m| {
        let at = up(m.creature(wolf).unwrap().position);
        m.creature_mut(wolf).unwrap().position = at;
        crate::game::map::ai::attacked(m, wolf, ga);
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    a.clear();
    let nowhere = map
        .query(move |m| {
            let at = m.creature(wolf).unwrap().position;
            let mut rng = rand::rng();
            (0..10).all(|_| {
                player_motion::fear_point(m, at, 5.0, &mut rng).is_none()
                    && player_motion::confusion_point(m, at, &mut rng).is_none()
            })
        })
        .await
        .unwrap();
    assert!(nowhere, "the navmesh has no point near up there");
    let waits = map
        .query(move |m| {
            motion::apply(m, ga, wolf, FEAR);
            let start = m.now;
            let mut out = Vec::new();
            for _ in 0..3 {
                creature_combat::controlled_step(m, wolf);
                let next = m.creature(wolf).unwrap().engagement.next_move_at.unwrap();
                let held = !walking(m, wolf);
                // Halfway through the pause: nothing.
                let before = m.now;
                m.now = before + (next - before) / 2;
                creature_combat::controlled_step(m, wolf);
                let still = m.creature(wolf).unwrap().engagement.next_move_at == Some(next)
                    && !walking(m, wolf)
                    && !m.creature(wolf).unwrap().combat.attacking;
                out.push((next - before, held, still));
                m.now = next;
            }
            let feared = creature_combat::control_changed(m, wolf);
            m.now = start;
            (out, feared)
        })
        .await
        .unwrap();
    let (waits, feared) = waits;
    for &(pause, held, still) in &waits {
        assert!(
            (Duration::from_millis(1000)..Duration::from_millis(1500)).contains(&pause),
            "a second or more: {pause:?}"
        );
        assert!(held && still, "it stands, and does nothing till then");
    }
    assert!(
        matches!(feared, Some(creature_combat::Control::Fear)),
        "still feared: {feared:?}"
    );
    // Alice, feared and then confused, high in the air.
    let pauses = map
        .query(move |m| {
            let at = up(m.players[&ga].position());
            m.players.get_mut(&ga).unwrap().movement.position = at;
            let mut out = Vec::new();
            for (spell, other) in [(FEAR, CONFUSE), (CONFUSE, FEAR)] {
                crate::game::map::auras::remove_spell(m, ga, other, None);
                motion::apply(m, wolf, ga, spell);
                for _ in 0..2 {
                    let before = m.now;
                    player_motion::update(m);
                    let d = m.players[&ga].driven.as_ref().unwrap();
                    out.push((spell, d.next_move_at - before, d.walk.is_none()));
                    m.now = d.next_move_at;
                }
            }
            out
        })
        .await
        .unwrap();
    for (spell, pause, stands) in pauses {
        let least = if spell == FEAR { 1000 } else { 500 };
        assert!(
            (Duration::from_millis(least)..Duration::from_millis(1500)).contains(&pause),
            "{spell}: {pause:?}"
        );
        assert!(stands, "{spell}: she stands");
    }
    // Nor was anything turned or run for either (the wolf's chase was
    // stopped as the fear took it, and that is all).
    let moved = a
        .gets(Duration::from_millis(200), |m| match m {
            ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv)
                if (mv.guid == wolf || mv.guid == ga)
                    && !matches!(mv.move_type, SMSG_MONSTER_MOVE_MonsterMoveType::Stop) =>
            {
                Some(format!("{mv:?}"))
            }
            _ => None,
        })
        .await;
    assert!(moved.is_none(), "no run nor turn sent: {moved:?}");
}

/// Beside Northshire Abbey's west wall: from here a walk 3.8 yards west
/// goes round what is in the way, more than four yards from here.
const ANCHOR: Vector3d = Vector3d {
    x: -8994.95,
    y: -177.493,
    z: 77.755_905,
};
const ROUND: Vector3d = Vector3d {
    x: -8998.75,
    y: -177.493,
    z: 77.978_264,
};

/// A confused unit's walk keeps within four yards of where its confusion
/// began: a way to its next point that goes further round (a detour the
/// navmesh gives) is refused, and it pauses instead, as Thistle Tea's
/// "rejects detours that leave the confusion circle" has it. A wolf
/// confused there walks a while and never leaves the circle.
#[tokio::test]
#[ignore = "extended"]
async fn a_confused_units_detour_out_of_its_circle_is_refused() {
    use crate::game::map::combat::dist;
    let Some(realm) = nav_realm() else {
        return;
    };
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let wolf = motion::wolf();
    let out = map
        .query(move |m| {
            let path = player_motion::control_path(m, ANCHOR, ROUND, 8.0);
            let far = path
                .as_ref()
                .map(|p| p.iter().map(|x| dist(*x, ANCHOR)).fold(0.0f32, f32::max));
            let refused = player_motion::confusion_route(m, ANCHOR, ANCHOR, ROUND).is_none();
            // Another way, straight and within the circle, is taken.
            let east = Vector3d {
                x: ANCHOR.x + 2.0,
                ..ANCHOR
            };
            let east = Vector3d {
                z: m.nav
                    .as_mut()
                    .and_then(|n| n.ground(east.x, east.y, east.z))
                    .unwrap(),
                ..east
            };
            let taken = player_motion::confusion_route(m, ANCHOR, ANCHOR, east).is_some();
            // The wolf, confused there: each walk it takes.
            let c = m.creature_mut(wolf).unwrap();
            c.position = ANCHOR;
            c.home = ANCHOR;
            motion::apply(m, ga, wolf, CONFUSE);
            let mut farthest = 0.0f32;
            let mut walks = 0;
            for _ in 0..40 {
                creature_combat::controlled_step(m, wolf);
                if let Some(Motion::Moving(w)) = m.creature(wolf).map(|c| &c.motion) {
                    walks += 1;
                    let d = w.destination();
                    farthest = farthest.max(dist(d, ANCHOR));
                    for p in &w.points {
                        farthest = farthest.max(dist(*p, ANCHOR));
                    }
                }
                let next = m.creature(wolf).unwrap().engagement.next_move_at.unwrap();
                m.now = next.max(m.now + Duration::from_millis(1));
                crate::game::map::ai::advance_motion(m, wolf);
            }
            (far, refused, taken, walks, farthest)
        })
        .await
        .unwrap();
    let (far, refused, taken, walks, farthest) = out;
    assert!(
        far.is_some_and(|f| f > 4.5),
        "the way round goes out: {far:?}"
    );
    assert!(refused, "and is refused");
    assert!(taken, "a way within is taken");
    assert!(walks > 0, "the wolf walks");
    assert!(farthest <= 4.01, "and keeps within the circle: {farthest}");
}

/// By Northshire Abbey, a place the navmesh keeps apart: a fear's point to
/// run to is found from here, but no way to it.
const CUT_OFF: Vector3d = Vector3d {
    x: -8975.95,
    y: -176.493,
    z: 85.380_615,
};

/// A feared wolf whose run has a point but no way to it keeps its fear and
/// stands 0.8 to 1.5 seconds before it looks again, with no run, as
/// Thistle Tea's "failed paths retain fear and wait before retrying" has
/// it.
#[tokio::test]
#[ignore = "extended"]
async fn a_feared_units_run_with_no_way_waits_and_tries_again() {
    let Some(realm) = nav_realm() else {
        return;
    };
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let wolf = motion::wolf();
    let out = map
        .query(move |m| {
            let mut rng = rand::rng();
            let tries: Vec<_> = (0..10)
                .map(|_| {
                    let to = player_motion::fear_point(m, CUT_OFF, 5.0, &mut rng);
                    (
                        to,
                        to.map(|to| player_motion::control_path(m, CUT_OFF, to, 30.0)),
                    )
                })
                .collect();
            // Mostly a point with no way to it; now and then no point (a
            // pause too); never a way.
            let cut = |path: &Option<Option<Vec<Vector3d>>>| matches!(path, Some(None));
            let no_way = tries.iter().all(|(to, path)| to.is_none() || cut(path))
                && tries.iter().filter(|(_, path)| cut(path)).count() >= 5;
            if !no_way {
                eprintln!("{tries:?}");
            }
            let c = m.creature_mut(wolf).unwrap();
            c.position = CUT_OFF;
            c.home = CUT_OFF;
            motion::apply(m, ga, wolf, FEAR);
            let mut rounds = Vec::new();
            for _ in 0..3 {
                let before = m.now;
                creature_combat::controlled_step(m, wolf);
                let next = m.creature(wolf).unwrap().engagement.next_move_at.unwrap();
                rounds.push((next - before, !walking(m, wolf)));
                m.now = next;
            }
            let feared = creature_combat::control_changed(m, wolf);
            (no_way, rounds, feared)
        })
        .await
        .unwrap();
    let (no_way, rounds, feared) = out;
    assert!(no_way, "a point, and no way to it");
    for (pause, stands) in rounds {
        assert!(
            (Duration::from_millis(800)..Duration::from_millis(1500)).contains(&pause),
            "{pause:?}"
        );
        assert!(stands, "no run");
    }
    assert!(
        matches!(feared, Some(creature_combat::Control::Fear)),
        "still feared: {feared:?}"
    );
}
