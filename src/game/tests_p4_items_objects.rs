//! Phase 4's game object tests, Thistle Tea's `GameObjectActionsTest`,
//! `GameObjectInteractionTest`, `GooberTest` and `UseRequirementTest`
//! ported: a script's actions on an object (destroyed and rebuilt, an
//! animation, a use, gone, made inert and usable again), the closes of
//! older uses that a newer state outlives, a quest giver's use breaking
//! the auras a use breaks and barred under an immunity, the interaction
//! reach (an object's rotation from its facing, its origin, its box scaled,
//! turned and widened), a quest object's custom animation, its cooldown
//! while inert and its use after a reset, and a use requirement on a spawn
//! not loaded.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::auras::{AURA_DUMMY, AURA_SCHOOL_IMMUNITY};
use crate::game::map::objects::*;
use crate::game::map::{Map, MapKey};
use crate::game::tests_p3_entity::{aura_effect, spell};

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

/// A door that closes three seconds after it opens; a button; a quest
/// giver barred under an immunity, and one not; a quest object that opens
/// for three seconds and cools down five; one that plays its animation
/// for three; a door waiting on a creature spawn and one on an object
/// spawn, neither loaded.
const DOOR: u32 = 97_200;
const BUTTON: u32 = 97_201;
const GIVER: u32 = 97_202;
const OPEN_GIVER: u32 = 97_203;
const GOOBER: u32 = 97_204;
const ANIMATED: u32 = 97_205;
const WAITING_DOOR: u32 = 97_206;
const WAITING_GATE: u32 = 97_207;
/// A dummy a use breaks, one it doesn't, and a school immunity.
const BROKEN: u32 = 97_210;
const KEPT: u32 = 97_211;
const SHIELD: u32 = 97_212;

/// A door's flags as it rests (`GO_FLAG_NODESPAWN`).
const NODESPAWN: u32 = 0x20;

fn object(entry: u32, kind: u8, data: &[(usize, u32)]) -> GameObjectTemplate {
    let mut t = GameObjectTemplate {
        entry,
        kind,
        display_id: 1,
        name: format!("Object {entry}"),
        size: 1.0,
        ..Default::default()
    };
    for (i, v) in data {
        t.data[*i] = *v;
    }
    t
}

fn data() -> GameData {
    let mut d = GameData::empty();
    let three = 3 * 0x10000;
    let mut door = object(DOOR, GO_DOOR, &[(2, three)]);
    door.flags = NODESPAWN;
    for t in [
        door,
        object(BUTTON, GO_BUTTON, &[]),
        object(GIVER, crate::game::map::world::GO_QUESTGIVER, &[(5, 1)]),
        object(OPEN_GIVER, crate::game::map::world::GO_QUESTGIVER, &[]),
        object(GOOBER, GO_GOOBER, &[(3, three), (6, 5)]),
        object(ANIMATED, GO_GOOBER, &[(3, three), (4, 1)]),
        object(WAITING_DOOR, GO_DOOR, &[]),
        object(WAITING_GATE, GO_DOOR, &[]),
    ] {
        d.gameobject_templates.insert(t.entry, t);
    }
    let spawns = [
        DOOR,
        BUTTON,
        GIVER,
        OPEN_GIVER,
        GOOBER,
        ANIMATED,
        WAITING_DOOR,
        WAITING_GATE,
    ]
    .iter()
    .enumerate()
    .map(|(i, entry)| GameObjectSpawn {
        guid: *entry,
        entry: *entry,
        map: 0,
        x: START.0 + 1.0 + (i % 3) as f32,
        y: START.1 + 1.0 + (i / 3) as f32,
        z: START.2,
        state: 1,
        respawn_min: 60,
        ..Default::default()
    })
    .collect();
    d.gameobject_spawns.insert(0, spawns);
    // Spawns not on the map: a creature and an object.
    d.object_requirements.insert(WAITING_DOOR, (0, 99_990));
    d.object_requirements.insert(WAITING_GATE, (1, 99_991));
    let mut broken = spell(BROKEN, &[aura_effect(AURA_DUMMY, 1, 0, 1)]);
    broken.aura_interrupt_flags = crate::game::map::auras::INTERRUPT_USE;
    let kept = spell(KEPT, &[aura_effect(AURA_DUMMY, 1, 0, 1)]);
    let shield = spell(SHIELD, &[aura_effect(AURA_SCHOOL_IMMUNITY, 1, 127, 1)]);
    for s in [broken, kept, shield] {
        d.spells.spells.insert(s.id, s);
    }
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

async fn setup() -> (Arc<Realm>, Client, Guid) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    (realm, a, g)
}

fn spawned(entry: u32) -> Guid {
    guid::game_object(entry, entry)
}

fn state(m: &Map, o: Guid) -> (u32, u32) {
    let x = m.gameobject(o).unwrap();
    (
        x.fields.get_u32(GAMEOBJECT_STATE),
        x.fields.get_u32(GAMEOBJECT_FLAGS),
    )
}

/// The objects' clocks `by` on: what waits on them comes sooner, and
/// what is due happens.
fn skip(m: &mut Map, by: Duration) {
    let back = |t: &mut std::time::Instant| {
        if let Some(sooner) = t.checked_sub(by) {
            *t = sooner;
        }
    };
    for (t, _, _) in m.objects.closes.iter_mut() {
        back(t);
    }
    for (t, _, _) in m.object_resets.iter_mut() {
        back(t);
    }
    for g in m.objects.goobers.values_mut() {
        g.ready_at.as_mut().map(back);
        g.active_until.as_mut().map(back);
    }
    crate::game::map::objects::update(m);
    crate::game::map::scripts::update_objects(m);
}

async fn wait_until(what: &str, mut f: impl AsyncFnMut() -> bool) {
    for _ in 0..200 {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never: {what}");
}

async fn has_aura(realm: &Arc<Realm>, g: Guid, spell: u32) -> bool {
    on_map(realm, move |m| {
        crate::game::map::auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
    })
    .await
}

/// Thistle Tea's object actions: destroyed (the alternative state, in
/// use) and rebuilt, it rests as it was, its flags its own; an animation
/// is played to those who see it; a use opens a door; removal takes it;
/// an action undefined (11, 23) does nothing.
#[tokio::test]
async fn a_scripts_actions_destroy_rebuild_animate_use_and_remove() {
    let (realm, mut a, g) = setup().await;
    let (door, button) = (spawned(DOOR), spawned(BUTTON));
    a.sees(door).await;
    let seen = on_map(&realm, move |m| {
        activate(m, g, door, 12);
        let destroyed = state(m, door);
        activate(m, g, door, 13);
        let rebuilt = state(m, door);
        activate(m, g, door, 11);
        activate(m, g, door, 23);
        let undefined = state(m, door);
        activate(m, g, door, 4);
        activate(m, g, door, 8);
        let used = state(m, door);
        activate(m, g, button, 15);
        (
            destroyed,
            rebuilt,
            undefined,
            used,
            m.gameobject(button).is_none(),
        )
    })
    .await;
    assert_eq!(
        seen,
        (
            (GO_STATE_ACTIVE_ALTERNATIVE, NODESPAWN | GO_FLAG_IN_USE),
            (GO_STATE_READY, NODESPAWN),
            (GO_STATE_READY, NODESPAWN),
            (GO_STATE_ACTIVE, NODESPAWN | GO_FLAG_IN_USE),
            true
        )
    );
    let animation = a
        .wait_for("the animation", |m| match m {
            ServerOpcodeMessage::SMSG_GAMEOBJECT_CUSTOM_ANIM(x) if x.guid == door => {
                Some(x.animation_id)
            }
            _ => None,
        })
        .await;
    assert_eq!(animation, 3, "action 4 plays animation 3");
}

/// Thistle Tea's revisions: a door used, reset and used again closes on
/// its second use's time, not its first's; a script's state (its flags as
/// they are) outlives the close of a use before it.
#[tokio::test]
async fn an_older_uses_close_doesnt_undo_a_newer_state() {
    let (realm, _a, g) = setup().await;
    let door = spawned(DOOR);
    let seen = on_map(&realm, move |m| {
        // Used, two seconds go by, reset, used again: the first close
        // would be in a second, the second's in three.
        activate(m, g, door, 8);
        skip(m, Duration::from_secs(2));
        activate(m, g, door, 13);
        activate(m, g, door, 8);
        skip(m, Duration::from_millis(1500));
        let reopened = state(m, door).0;
        skip(m, Duration::from_secs(2));
        let closed = state(m, door).0;
        // Used, then a script's state: it stays so past the use's close.
        activate(m, g, door, 8);
        let flags = state(m, door).1;
        crate::game::map::scripts::set_object_state(m, door, GO_STATE_ACTIVE_ALTERNATIVE);
        let scripted = state(m, door);
        skip(m, Duration::from_secs(4));
        (
            reopened,
            closed,
            scripted == (GO_STATE_ACTIVE_ALTERNATIVE, flags),
            state(m, door).0,
        )
    })
    .await;
    assert_eq!(
        seen,
        (
            GO_STATE_ACTIVE,
            GO_STATE_READY,
            true,
            GO_STATE_ACTIVE_ALTERNATIVE
        )
    );
}

/// Thistle Tea's inert and active: made inert, an object has the
/// no-interact flag, its state as it was, and isn't used; made active
/// again, it is.
#[tokio::test]
async fn an_object_made_inert_isnt_used_until_made_active() {
    let (realm, a, g) = setup().await;
    let door = spawned(DOOR);
    let inert = on_map(&realm, move |m| {
        activate(m, g, door, 16);
        state(m, door)
    })
    .await;
    assert_eq!(inert, (GO_STATE_READY, NODESPAWN | GO_FLAG_NO_INTERACT));
    a.send(CMSG_GAMEOBJ_USE { guid: door }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let still = on_map(&realm, move |m| state(m, door)).await;
    assert_eq!(still, inert, "not used");
    let active = on_map(&realm, move |m| {
        activate(m, g, door, 17);
        state(m, door)
    })
    .await;
    assert_eq!(active, (GO_STATE_READY, NODESPAWN));
    a.send(CMSG_GAMEOBJ_USE { guid: door }).await;
    wait_until("the door open", async || {
        on_map(&realm, move |m| state(m, door).0).await == GO_STATE_ACTIVE
    })
    .await;
}

/// Thistle Tea's quest giver use: under a school immunity, one that bars
/// it isn't used, and nothing on the user breaks; one made inert isn't
/// either; else a use takes the auras a use breaks, and only those.
#[tokio::test]
async fn using_a_quest_giver_breaks_auras_but_not_under_an_immunity_it_bars() {
    let (realm, a, g) = setup().await;
    let (giver, open) = (spawned(GIVER), spawned(OPEN_GIVER));
    on_map(&realm, move |m| {
        for s in [BROKEN, KEPT, SHIELD] {
            crate::game::map::spells::cast_triggered(m, g, s, Some(g));
        }
    })
    .await;
    assert!(has_aura(&realm, g, SHIELD).await);
    a.send(CMSG_GAMEOBJ_USE { guid: giver }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        has_aura(&realm, g, BROKEN).await,
        "barred under Divine Shield"
    );
    on_map(&realm, move |m| {
        crate::game::map::auras::remove_spell(m, g, SHIELD, None);
        if let Some(o) = m.gameobject_mut(open) {
            o.fields
                .set_flag(GAMEOBJECT_FLAGS, GO_FLAG_NO_INTERACT, true);
        }
    })
    .await;
    a.send(CMSG_GAMEOBJ_USE { guid: open }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(has_aura(&realm, g, BROKEN).await, "an inert giver");
    a.send(CMSG_GAMEOBJ_USE { guid: giver }).await;
    wait_until("the aura broken", async || {
        !has_aura(&realm, g, BROKEN).await
    })
    .await;
    assert!(has_aura(&realm, g, KEPT).await, "one a use doesn't break");
}

/// The same under an immunity for an object that doesn't bar it: it is
/// used, the aura broken.
#[tokio::test]
async fn a_quest_giver_that_doesnt_bar_an_immunity_is_used_under_one() {
    let (realm, a, g) = setup().await;
    let open = spawned(OPEN_GIVER);
    on_map(&realm, move |m| {
        for s in [BROKEN, SHIELD] {
            crate::game::map::spells::cast_triggered(m, g, s, Some(g));
        }
    })
    .await;
    a.send(CMSG_GAMEOBJ_USE { guid: open }).await;
    wait_until("the aura broken", async || {
        !has_aura(&realm, g, BROKEN).await
    })
    .await;
}

/// Thistle Tea's quest objects: one with a custom animation keeps its
/// state, in use, and plays animation 0; one made inert isn't used, and
/// its cooldown isn't spent; one reset after a use is used again once its
/// cooldown is over, and closes on that use's time.
#[tokio::test]
async fn a_quest_objects_animation_inert_cooldown_and_use_after_a_reset() {
    let (realm, mut a, g) = setup().await;
    let (animated, goober) = (spawned(ANIMATED), spawned(GOOBER));
    a.sees(animated).await;
    a.send(CMSG_GAMEOBJ_USE { guid: animated }).await;
    let animation = a
        .wait_for("the animation", |m| match m {
            ServerOpcodeMessage::SMSG_GAMEOBJECT_CUSTOM_ANIM(x) if x.guid == animated => {
                Some(x.animation_id)
            }
            _ => None,
        })
        .await;
    assert_eq!(animation, 0);
    let seen = on_map(&realm, move |m| {
        let custom = state(m, animated);
        // Inert: not used, no cooldown.
        activate(m, g, goober, 16);
        crate::game::map::world::use_object(m, g, goober);
        let inert = (
            state(m, goober).0,
            m.objects.goobers.get(&goober).and_then(|x| x.ready_at),
        );
        activate(m, g, goober, 17);
        // Used, two seconds go by, reset: used again only at three.
        crate::game::map::world::use_object(m, g, goober);
        let first = state(m, goober).0;
        skip(m, Duration::from_secs(2));
        activate(m, g, goober, 13);
        crate::game::map::world::use_object(m, g, goober);
        let cooling = state(m, goober).0;
        skip(m, Duration::from_secs(1));
        crate::game::map::world::use_object(m, g, goober);
        let again = state(m, goober).0;
        skip(m, Duration::from_millis(2500));
        let still = state(m, goober).0;
        skip(m, Duration::from_secs(1));
        (
            custom,
            inert,
            first,
            cooling,
            again,
            still,
            state(m, goober).0,
        )
    })
    .await;
    assert_eq!(
        seen,
        (
            (GO_STATE_READY, GO_FLAG_IN_USE),
            (GO_STATE_READY, None),
            GO_STATE_ACTIVE,
            GO_STATE_READY,
            GO_STATE_ACTIVE,
            GO_STATE_ACTIVE,
            GO_STATE_READY
        )
    );
}

/// Thistle Tea's use requirement: one on a creature spawn, or an object
/// spawn, not loaded doesn't block.
#[tokio::test]
async fn a_use_requirement_on_a_spawn_not_loaded_doesnt_block() {
    let (realm, _a, g) = setup().await;
    let seen = on_map(&realm, move |m| {
        [WAITING_DOOR, WAITING_GATE].map(|e| {
            let o = spawned(e);
            let met = requirement_met(m, o);
            crate::game::map::world::use_object(m, g, o);
            (met, state(m, o).0)
        })
    })
    .await;
    assert_eq!(seen, [(true, GO_STATE_ACTIVE); 2]);
}

fn at(x: f32, y: f32, z: f32) -> Vector3d {
    Vector3d { x, y, z }
}

const ORIGIN: Vector3d = Vector3d {
    x: 0.0,
    y: 0.0,
    z: 0.0,
};

/// Thistle Tea's `within?`, at the origin, of radius 5, unscaled, not
/// turned.
fn near(p: Vector3d, bounds: Option<([f32; 3], [f32; 3])>) -> bool {
    near_with(p, bounds, 1.0, [0.0, 0.0, 0.0, 1.0])
}

fn near_with(
    p: Vector3d,
    bounds: Option<([f32; 3], [f32; 3])>,
    scale: f32,
    rotation: [f32; 4],
) -> bool {
    within(p, ORIGIN, rotation, scale, bounds, 5.0)
}

/// Thistle Tea's rotation: with no z and w, from the facing; else as
/// stored.
#[test]
fn an_objects_rotation_comes_from_its_facing_when_it_has_none() {
    let [x, y, z, w] = rotation([0.0; 4], std::f32::consts::PI);
    assert_eq!((x, y), (0.0, 0.0));
    assert!((z - 1.0).abs() < 1e-5 && w.abs() < 1e-5, "{z} {w}");
    assert_eq!(rotation([0.0, 0.0, 0.5, 0.5], 0.0), [0.0, 0.0, 0.5, 0.5]);
}

/// Thistle Tea's interaction distance: from the origin in three
/// dimensions with no box; a box scaled and widened by the radius
/// unscaled; turned about each axis; an origin elsewhere, a rotation of
/// nothing, a box of nothing.
#[test]
fn the_interaction_reach_is_the_origins_or_the_turned_scaled_box() {
    assert!(near(at(3.0, 4.0, 0.0), None));
    assert!(!near(at(3.0, 4.0, 0.1), None));
    assert!(!near(at(0.0, 0.0, 5.1), None));
    // Scaled twice: four yards out and three up become eight and six, and
    // the radius five more.
    let b = Some(([-2.0, -1.0, 0.0], [4.0, 1.0, 3.0]));
    let id = [0.0, 0.0, 0.0, 1.0];
    assert!(near_with(at(13.0, 0.0, 0.0), b, 2.0, id));
    assert!(!near_with(at(13.1, 0.0, 0.0), b, 2.0, id));
    assert!(near_with(at(0.0, 0.0, 11.0), b, 2.0, id));
    assert!(!near_with(at(0.0, 0.0, 11.1), b, 2.0, id));
    // A long box turned a quarter about z, then about y.
    let b = Some(([-1.0, -1.0, -1.0], [10.0, 1.0, 1.0]));
    let half = 0.5f32.sqrt();
    let (about_z, about_y) = ([0.0, 0.0, half, half], [0.0, half, 0.0, half]);
    assert!(near_with(at(0.0, 14.0, 0.0), b, 1.0, about_z));
    assert!(!near_with(at(14.0, 0.0, 0.0), b, 1.0, about_z));
    assert!(near_with(at(0.0, 0.0, -14.0), b, 1.0, about_y));
    assert!(!near_with(at(0.0, 0.0, 14.0), b, 1.0, about_y));
    assert!(within(
        at(12.0, 20.0, 30.0),
        at(10.0, 20.0, 30.0),
        [0.0; 4],
        1.0,
        None,
        2.0
    ));
    let nothing = Some(([0.0; 3], [0.0; 3]));
    assert!(near(at(5.0, 0.0, 0.0), nothing));
    assert!(!near(at(5.1, 0.0, 0.0), nothing));
    assert!(near_with(
        at(14.0, 0.0, 0.0),
        Some(([0.0; 3], [10.0, 0.0, 0.0])),
        1.0,
        [0.0; 4]
    ));
}
