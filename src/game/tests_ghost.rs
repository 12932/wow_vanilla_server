//! Ghosts: they walk on water (mangos-classic's `Aura::HandleAuraGhost`),
//! and see only what is within 45 yards of their corpse, their group and
//! the ghosts of their side (mangos-classic's `IsVisibleInGridForPlayer`,
//! and the user's rule for everything else).

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage as S;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, MapKey, auras, death, grid};

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const SHORT: Duration = Duration::from_millis(300);

fn east(dx: f32) -> Vector3d {
    Vector3d {
        x: START.0 + dx,
        y: START.1,
        z: START.2,
    }
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

async fn enter(realm: &Arc<Realm>, account: &str, race: Race) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c
        .enter(
            &format!("Ghost{}", account.to_lowercase()),
            race,
            Class::Warrior,
        )
        .await;
    let map = realm.registry.online(g).unwrap().map();
    (c, g, map)
}

/// Move player `g` to `dx` yards east of the start, as the map sees it.
fn put(m: &mut Map, g: Guid, dx: f32) {
    let at = east(dx);
    let Some(p) = m.players.get_mut(&g) else {
        return;
    };
    p.movement.position = at;
    p.move_check = None;
    let (old, new) = (p.cell, grid::cell_of(at.x, at.y));
    p.cell = new;
    m.grid.relocate(g, old, new);
    m.visibility_dirty.insert(g);
}

/// Kill `c`'s player and release its spirit, where it stands (the test
/// world has no graveyard), acknowledging the teleport.
async fn die_and_release(c: &mut Client, map: &MapHandle) {
    let g = c.guid;
    on(map, move |m| death::kill(m, g)).await;
    c.send(CMSG_REPOP_REQUEST {}).await;
    c.wait_for("the teleport to the graveyard", |m| {
        matches!(m, S::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
}

async fn rise(map: &MapHandle, g: Guid) {
    assert!(on(map, move |m| death::resurrect(m, g, 1.0)).await);
}

async fn water_walk(c: &mut Client) {
    c.wait_for("SMSG_MOVE_WATER_WALK", |m| {
        matches!(m, S::SMSG_MOVE_WATER_WALK(_)).then_some(())
    })
    .await;
}

async fn land_walk(c: &mut Client) {
    c.wait_for("SMSG_MOVE_LAND_WALK", |m| {
        matches!(m, S::SMSG_MOVE_LAND_WALK(_)).then_some(())
    })
    .await;
}

async fn no_land_walk(c: &mut Client, when: &str) {
    let got = c
        .gets(SHORT, |m| {
            matches!(m, S::SMSG_MOVE_LAND_WALK(_)).then_some(())
        })
        .await;
    assert!(got.is_none(), "no SMSG_MOVE_LAND_WALK {when}");
}

/// Wait, a short poll up to a limit, until the map has `what`.
async fn until(map: &MapHandle, what: &str, f: impl Fn(&Map) -> bool + Send + Clone + 'static) {
    for _ in 0..400 {
        let f = f.clone();
        if on(map, move |m| f(m)).await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("never: {what}");
}

async fn knows(map: &MapHandle, g: Guid, of: Guid) -> bool {
    on(map, move |m| m.players[&g].known.contains(&of)).await
}

/// A released player walks on water; risen, on land again.
#[tokio::test]
async fn a_ghost_walks_on_water_until_it_rises() {
    let realm = harness::realm();
    let (mut a, g, map) = enter(&realm, "A", Race::Human).await;
    die_and_release(&mut a, &map).await;
    water_walk(&mut a).await;
    rise(&map, g).await;
    land_walk(&mut a).await;
}

/// A night elf's wisp walks on water too.
#[tokio::test]
async fn a_wisp_walks_on_water() {
    let realm = harness::realm();
    let (mut a, g, map) = enter(&realm, "A", Race::NightElf).await;
    die_and_release(&mut a, &map).await;
    water_walk(&mut a).await;
    let wisp = on(&map, move |m| {
        m.players[&g]
            .fields
            .get_u32(crate::game::fields::UNIT_FIELD_AURA + 1)
    })
    .await;
    assert_eq!(wisp, death::SPELL_WISP);
}

const WATER_WALKING: u32 = 900_600;

fn water_walking_data() -> GameData {
    let mut d = GameData::empty();
    let mut s = SpellTemplate {
        id: WATER_WALKING,
        name: "Water Walking".into(),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    // Kept through a death, so that a ghost can have it.
    s.attributes[3] |= crate::game::data::spells::ATTR_EX3_DEATH_PERSISTENT;
    s.effects[0] = SpellEffect {
        effect: 6,
        aura: auras::AURA_WATER_WALK,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(WATER_WALKING, s);
    d
}

/// A water-walking aura keeps a risen ghost on the water, and its end
/// leaves a ghost on the water: only when neither holds does the player
/// walk on land (mangos's `HandleAuraGhost` and `HandleAuraWaterWalk`).
#[tokio::test]
async fn a_water_walking_aura_and_a_ghost_each_keep_the_water() {
    let realm = harness::realm_with(water_walking_data());
    let (mut a, g, map) = enter(&realm, "A", Race::Human).await;
    let apply = move |m: &mut Map| {
        let data = m.realm.data.clone();
        let s = data.spells.get(WATER_WALKING).unwrap();
        let _ = auras::apply_spell(m, g, g, s, &[(0, 0)], auras::Context::default());
        auras::auras(m, g).is_some_and(|x| x.has_spell(WATER_WALKING))
    };
    assert!(on(&map, apply).await, "the aura is on");
    water_walk(&mut a).await;

    die_and_release(&mut a, &map).await;
    water_walk(&mut a).await;
    rise(&map, g).await;
    no_land_walk(&mut a, "at a resurrection with the aura on").await;

    die_and_release(&mut a, &map).await;
    water_walk(&mut a).await;
    on(&map, move |m| {
        auras::remove_spell(m, g, WATER_WALKING, None)
    })
    .await;
    no_land_walk(&mut a, "at the aura's end on a ghost").await;
    rise(&map, g).await;
    land_walk(&mut a).await;
}

/// A ghost logging back in walks on water.
#[tokio::test]
async fn a_ghost_logging_in_walks_on_water() {
    let realm = harness::realm();
    let (mut a, g, map) = enter(&realm, "A", Race::Human).await;
    die_and_release(&mut a, &map).await;
    water_walk(&mut a).await;
    a.disconnect();
    for _ in 0..500 {
        if realm.registry.online(g).is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(realm.registry.online(g).is_none(), "logged out");
    realm.store.flush().await;
    let mut a = Client::connect(&realm, "A");
    a.login(g).await;
    water_walk(&mut a).await;
}

/// A ghost sent to another map walks on water there once its client has
/// loaded it (mangos's `SendInitialPacketsAfterAddToMap`).
#[tokio::test]
async fn a_ghost_changing_maps_walks_on_water() {
    let realm = harness::realm();
    let (mut a, g, map) = enter(&realm, "A", Race::Human).await;
    die_and_release(&mut a, &map).await;
    water_walk(&mut a).await;
    on(&map, move |m| {
        let to = Vector3d {
            x: 1629.0,
            y: -4373.0,
            z: 31.0,
        };
        m.teleport(g, MapKey::world(1), to, 0.0);
    })
    .await;
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, S::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.clear();
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    water_walk(&mut a).await;
}

/// A ghost sees a living player 30 yards from its corpse and not one 60
/// yards from it; the living don't see the ghost; risen, it sees both, and
/// they it.
#[tokio::test]
async fn a_ghost_sees_the_living_only_near_its_corpse() {
    let realm = harness::realm();
    let (mut a, g, map) = enter(&realm, "A", Race::Human).await;
    let (mut b, near, _) = enter(&realm, "B", Race::Human).await;
    let (mut c, far, _) = enter(&realm, "C", Race::Human).await;
    on(&map, move |m| {
        put(m, near, 30.0);
        put(m, far, 60.0);
    })
    .await;
    a.sees(near).await;
    a.sees(far).await;
    b.sees(g).await;
    c.sees(g).await;
    die_and_release(&mut a, &map).await;
    a.forgets(far).await;
    b.forgets(g).await;
    c.forgets(g).await;
    assert!(knows(&map, g, near).await, "the one by the corpse is seen");
    assert!(!knows(&map, g, far).await, "the one 60 yards off isn't");

    rise(&map, g).await;
    a.sees(far).await;
    b.sees(g).await;
    c.sees(g).await;
    assert!(knows(&map, g, near).await);
}

async fn group(leader: &mut Client, member: &mut Client, name: &str) {
    leader
        .send(CMSG_GROUP_INVITE {
            name: name.to_owned(),
        })
        .await;
    member
        .wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, S::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
    member.send(CMSG_GROUP_ACCEPT {}).await;
    member
        .wait_for("SMSG_GROUP_LIST", |m| match m {
            S::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
            _ => None,
        })
        .await;
}

/// A ghost sees its group, far from its corpse, and they it; and the
/// ghosts of its side, but not the other side's.
#[tokio::test]
async fn a_ghost_sees_its_group_and_its_sides_ghosts() {
    let realm = harness::realm();
    let (mut a, g, map) = enter(&realm, "A", Race::Human).await;
    let (mut e, friend, _) = enter(&realm, "E", Race::Human).await;
    let (mut d, ghost, _) = enter(&realm, "D", Race::Human).await;
    let (mut o, orc, _) = enter(&realm, "O", Race::Orc).await;
    group(&mut a, &mut e, "Ghoste").await;
    // The orc, from Durotar to Northshire.
    let orcs = realm.registry.online(orc).unwrap().map();
    on(&orcs, move |m| {
        m.teleport(orc, MapKey::world(0), east(20.0), 0.0)
    })
    .await;
    o.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, S::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    o.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    let mut arrived = false;
    for _ in 0..400 {
        arrived = on(&map, move |m| {
            m.players.get(&orc).is_some_and(|p| !p.awaiting_worldport)
        })
        .await;
        if arrived {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(arrived, "the orc is in Northshire");
    on(&map, move |m| {
        put(m, friend, 80.0);
        put(m, ghost, 70.0);
    })
    .await;
    die_and_release(&mut d, &map).await;
    die_and_release(&mut o, &map).await;
    until(&map, "the living forget the ghosts", move |m| {
        let known = &m.players[&g].known;
        !known.contains(&ghost) && !known.contains(&orc)
    })
    .await;

    die_and_release(&mut a, &map).await;
    until(&map, "the ghosts of a side see each other", move |m| {
        m.players[&g].known.contains(&ghost) && m.players[&ghost].known.contains(&g)
    })
    .await;
    tokio::time::sleep(SHORT).await;
    assert!(
        !knows(&map, g, orc).await,
        "the other side's ghost is unseen"
    );
    assert!(knows(&map, g, friend).await, "the group, 80 yards off");
    assert!(knows(&map, friend, g).await, "the group sees its ghost");
    let forgot = e
        .gets(SHORT, |m| harness::forgotten(m).contains(&g).then_some(()))
        .await;
    assert!(forgot.is_none(), "the group isn't told to forget its ghost");
}

const NEAR_WOLF: u32 = 900_610;
const FAR_WOLF: u32 = 900_611;
const NEAR_CRATE: u32 = 900_612;
const FAR_CRATE: u32 = 900_613;

fn wolf(entry: u32, name: &str) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 10,
        level_max: 10,
        faction: 35,
        unit_class: 1,
        health_max: 500,
        detection_range: 0.1,
        ..Default::default()
    }
}

/// Wolves and crates 30 and 60 yards east of the start; a wolf 30 yards
/// west.
fn things_data() -> GameData {
    let mut d = GameData::empty();
    d.creature_templates
        .insert(NEAR_WOLF, wolf(NEAR_WOLF, "Near Wolf"));
    d.creature_templates
        .insert(FAR_WOLF, wolf(FAR_WOLF, "Far Wolf"));
    let spawn = |guid, entry, dx: f32| CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        ..Default::default()
    };
    d.creature_spawns.insert(
        0,
        vec![
            spawn(60, NEAR_WOLF, 30.0),
            spawn(61, FAR_WOLF, 60.0),
            spawn(62, NEAR_WOLF, -30.0),
        ],
    );
    for (entry, name) in [(NEAR_CRATE, "Near Crate"), (FAR_CRATE, "Far Crate")] {
        d.gameobject_templates.insert(
            entry,
            GameObjectTemplate {
                entry,
                kind: 5,
                display_id: 1,
                name: name.into(),
                size: 1.0,
                ..Default::default()
            },
        );
    }
    let object = |guid, entry, dx: f32| GameObjectSpawn {
        guid,
        entry,
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        state: 1,
        ..Default::default()
    };
    d.gameobject_spawns.insert(
        0,
        vec![object(700, NEAR_CRATE, 30.0), object(701, FAR_CRATE, 60.0)],
    );
    d
}

async fn creature_at(map: &MapHandle, entry: u32, dx: f32) -> Guid {
    on(map, move |m| {
        m.creatures
            .iter()
            .find(|(_, c)| c.entry == entry && (c.position.x - (START.0 + dx)).abs() < 1.0)
            .map(|(_, c)| c.guid)
    })
    .await
    .expect("spawned")
}

async fn object_of(map: &MapHandle, entry: u32) -> Guid {
    on(map, move |m| {
        m.gameobjects
            .iter()
            .find(|(_, o)| o.entry == entry)
            .map(|(_, o)| o.guid)
    })
    .await
    .expect("spawned")
}

/// Creatures, objects and others' corpses follow the same 45 yards of the
/// corpse; risen, the ghost sees them all again.
#[tokio::test]
async fn a_ghost_sees_creatures_objects_and_corpses_only_near_its_corpse() {
    let realm = harness::realm_with(things_data());
    let (mut a, g, map) = enter(&realm, "A", Race::Human).await;
    let (mut b, other, _) = enter(&realm, "B", Race::Human).await;
    let near = creature_at(&map, NEAR_WOLF, 30.0).await;
    let far = creature_at(&map, FAR_WOLF, 60.0).await;
    let (near_crate, far_crate) = (
        object_of(&map, NEAR_CRATE).await,
        object_of(&map, FAR_CRATE).await,
    );
    on(&map, move |m| put(m, other, 60.0)).await;
    die_and_release(&mut b, &map).await;
    let others_corpse = guid::corpse(guid::counter(other));
    for x in [near, far, near_crate, far_crate, others_corpse] {
        a.sees(x).await;
    }

    die_and_release(&mut a, &map).await;
    let own_corpse = guid::corpse(guid::counter(g));
    a.sees(own_corpse).await;
    for x in [far, far_crate, others_corpse] {
        a.forgets(x).await;
    }
    for x in [near, near_crate, own_corpse] {
        assert!(knows(&map, g, x).await, "{x} by the corpse is seen");
    }

    rise(&map, g).await;
    for x in [far, far_crate, others_corpse] {
        a.sees(x).await;
    }
}

/// A ghost running back to its corpse sees what is by the corpse as it
/// comes into range, never what is far from the corpse though it passes
/// by; what leaves the corpse's 45 yards goes.
#[tokio::test]
async fn a_ghost_running_to_its_corpse_sees_what_is_by_it() {
    let realm = harness::realm_with(things_data());
    let (mut a, g, map) = enter(&realm, "A", Race::Human).await;
    let west = creature_at(&map, NEAR_WOLF, -30.0).await;
    let far = creature_at(&map, FAR_WOLF, 60.0).await;
    let (_b, by, _) = enter(&realm, "B", Race::Human).await;
    let (_c, off, _) = enter(&realm, "C", Race::Human).await;
    on(&map, move |m| {
        put(m, by, -30.0);
        put(m, off, 60.0);
    })
    .await;
    for x in [west, far, by, off] {
        a.sees(x).await;
    }
    die_and_release(&mut a, &map).await;
    a.forgets(far).await;
    a.forgets(off).await;
    // Off 200 yards east, out of sight of everything.
    on(&map, move |m| put(m, g, 200.0)).await;
    a.forgets(west).await;
    a.forgets(by).await;
    // Back west, past the far wolf, to the corpse.
    for dx in [150.0, 110.0, 70.0, 60.0, 50.0, 20.0, 0.0] {
        on(&map, move |m| {
            if let Some(p) = m.players.get_mut(&g) {
                p.move_check = None;
            }
        })
        .await;
        let at = east(dx);
        a.heartbeat(at.x, at.y, at.z).await;
        let mut taken = false;
        for _ in 0..200 {
            let here = on(&map, move |m| m.players[&g].position().x).await;
            if (here - at.x).abs() < 0.5 {
                taken = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(taken, "the move to {dx} is taken");
    }
    a.sees(west).await;
    a.sees(by).await;
    assert!(
        !a.sees_within(far, SHORT).await,
        "the wolf 60 yards from the corpse stays unseen"
    );
    assert!(
        !a.sees_within(off, SHORT).await,
        "the player 60 yards from the corpse stays unseen"
    );
    // The west wolf wanders off, 60 yards from the corpse.
    on(&map, move |m| {
        if let Some(c) = m.creature_mut(west) {
            c.position = east(-60.0);
            let (old, new) = (c.cell, grid::cell_of(c.position.x, c.position.y));
            c.cell = new;
            m.grid.relocate(west, old, new);
        }
        m.visibility_dirty.insert(west);
    })
    .await;
    a.forgets(west).await;
}
