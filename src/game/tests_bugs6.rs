//! Bugs the user found in the game, the sixth batch: Frost Nova that held
//! no Young Wolf, a critter's death the client never saw, a stable master
//! with nothing to say to a mage, and dead quest givers with their quests
//! still over their heads.

use std::collections::BTreeMap;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::fields::*;
use crate::game::harness;
use crate::game::map::creature::{Creature, Motion};
use crate::game::map::{Map, MapHandle, auras, combat};

const FROST_NOVA: u32 = 122;
const YOUNG_WOLF: u32 = 299;

/// The fields an update's mask carries, by index: wow_messages keeps them
/// to itself but for its `Debug` (`values: {index: value, ...}`).
fn fields(mask: &UpdateMask) -> BTreeMap<u16, u32> {
    let s = format!("{mask:?}");
    let Some(i) = s.find("values: {") else {
        return BTreeMap::new();
    };
    let rest = &s[i + "values: {".len()..];
    let rest = &rest[..rest.find('}').unwrap_or(rest.len())];
    rest.split(", ")
        .filter_map(|kv| {
            let (k, v) = kv.split_once(": ")?;
            Some((k.trim().parse().ok()?, v.trim().parse().ok()?))
        })
        .collect()
}

/// One block of an update packet, as the client reads it.
#[derive(Debug, Clone, PartialEq)]
enum Block {
    Create(Guid, BTreeMap<u16, u32>),
    Values(Guid, BTreeMap<u16, u32>),
    Forget(Guid),
}

/// The blocks of an update packet, in order.
fn blocks(m: &ServerOpcodeMessage) -> Vec<Block> {
    let objects = match m {
        ServerOpcodeMessage::SMSG_UPDATE_OBJECT(u) => &u.objects,
        ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(u) => &u.objects,
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for o in objects {
        match &o.update_type {
            Object_UpdateType::Values { guid1, mask1 } => {
                out.push(Block::Values(*guid1, fields(mask1)));
            }
            Object_UpdateType::CreateObject { guid3, mask2, .. }
            | Object_UpdateType::CreateObject2 { guid3, mask2, .. } => {
                out.push(Block::Create(*guid3, fields(mask2)));
            }
            Object_UpdateType::OutOfRangeObjects { guids } => {
                out.extend(guids.iter().map(|g| Block::Forget(*g)));
            }
            _ => {}
        }
    }
    out
}

/// A creature of `entry` as its first spawn in the world has it (its
/// level, health and wander), `yards` in front of `g` and facing them.
fn spawned_near(m: &mut Map, g: Guid, entry: u32, yards: f32, spawn_guid: u32) -> Guid {
    let data = m.realm.data.clone();
    let mut spawn = data
        .creature_spawns
        .values()
        .flatten()
        .find(|s| s.entries == [entry])
        .cloned()
        .expect("a spawn of it");
    let p = &m.players[&g];
    let (at, o) = (p.position(), p.movement.orientation);
    spawn.guid = spawn_guid;
    spawn.x = at.x + yards * o.cos();
    spawn.y = at.y + yards * o.sin();
    spawn.z = at.z;
    spawn.orientation = o + std::f32::consts::PI;
    let c = Creature::from_spawn(&data, &spawn, &mut rand::rng()).expect("the creature");
    m.add_creature(c)
}

/// Whether `a` is in melee reach of `b`.
fn in_reach(m: &Map, a: Guid, b: Guid) -> bool {
    let (Some(a), Some(b)) = (m.unit_view(a), m.unit_view(b)) else {
        return false;
    };
    combat::in_melee_range(&a, &b)
}

/// Wait until `pick` holds of the map, for up to a few seconds of its
/// updates.
async fn until(map: &MapHandle, what: &str, pick: impl Fn(&Map) -> bool + Send + Sync + 'static) {
    let pick = std::sync::Arc::new(pick);
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        let p = pick.clone();
        if map.query(move |m| p(m)).await.unwrap() {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "never: {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Frost Nova on a Young Wolf on its wander, one no one has fought yet:
/// the client is told the aura (the frost about its feet) and the root,
/// and its walk stops; it fights on where it stands, its victim the mage,
/// and once free it chases them (mangos's `HandleAuraModRoot` stops no
/// attack, and `SetImmobilizedState` sends `SMSG_SPLINE_MOVE_ROOT`). It
/// went back to its wander, the mage still its target.
#[tokio::test]
#[ignore = "extended"]
async fn frost_nova_holds_a_wandering_young_wolf_which_chases_once_free() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut c, mage, map) = player(&realm, "B6FN", "Frosty", Race::Human, Class::Mage).await;
    // Its first login's cinematic over: the mage sees from where she stands.
    c.send(CMSG_COMPLETE_CINEMATIC {}).await;
    let wolf = map
        .query(move |m| {
            ready(m, mage, 10, &[FROST_NOVA]);
            spawned_near(m, mage, YOUNG_WOLF, 8.0, 0x7F_0001)
        })
        .await
        .unwrap();
    c.sees_on(&map, wolf).await;
    // On its wander, minding its own business.
    until(&map, "the wolf walks", move |m| {
        m.creature(wolf)
            .is_some_and(|c| matches!(c.motion, Motion::Moving(_)))
    })
    .await;
    c.clear();
    let cast = map
        .query(move |m| cast_fresh(m, mage, FROST_NOVA, at(mage)))
        .await
        .unwrap();
    assert!(cast, "Frost Nova goes off");
    c.wait_for("the wolf rooted", |m| match m {
        ServerOpcodeMessage::SMSG_SPLINE_MOVE_ROOT(r) if r.guid == wolf => Some(()),
        _ => None,
    })
    .await;
    c.wait_for("the frost on the wolf", |m| {
        blocks(m).into_iter().find_map(|b| match b {
            Block::Values(g, f)
                if g == wolf
                    && (0..48).any(|i| f.get(&(UNIT_FIELD_AURA + i)) == Some(&FROST_NOVA)) =>
            {
                Some(())
            }
            _ => None,
        })
    })
    .await;
    let (rooted, fighting, target, moving) = map
        .query(move |m| {
            let c = m.creature(wolf).unwrap();
            (
                has(m, wolf, FROST_NOVA),
                c.combat.attacking && c.combat.victim == mage,
                c.fields.get_guid(UNIT_FIELD_TARGET),
                matches!(c.motion, Motion::Moving(_)),
            )
        })
        .await
        .unwrap();
    assert!(rooted, "Frost Nova's aura stays on the wolf");
    assert!(fighting, "the wolf fights the mage");
    assert_eq!(target, mage);
    assert!(!moving, "its wander stops");
    // Held: it stays where it is, update after update.
    let (from, updates) = map
        .query(move |m| (m.creature(wolf).unwrap().position, m.stats.updates))
        .await
        .unwrap();
    until(&map, "ten updates", move |m| {
        m.stats.updates >= updates + 10
    })
    .await;
    let (now, moving, reach) = map
        .query(move |m| {
            let c = m.creature(wolf).unwrap();
            (
                c.position,
                matches!(c.motion, Motion::Moving(_)),
                in_reach(m, wolf, mage),
            )
        })
        .await
        .unwrap();
    assert!(
        combat::dist(from, now) < 0.01 && !moving,
        "it stands, rooted"
    );
    assert!(!reach, "out of the mage's reach");
    // Free, it chases the mage down.
    c.clear();
    map.query(move |m| auras::remove_spell(m, wolf, FROST_NOVA, None))
        .await
        .unwrap();
    c.wait_for("the wolf free", |m| match m {
        ServerOpcodeMessage::SMSG_SPLINE_MOVE_UNROOT(r) if r.guid == wolf => Some(()),
        _ => None,
    })
    .await;
    until(&map, "the wolf comes at the mage", move |m| {
        in_reach(m, wolf, mage)
    })
    .await;
}

const RABBIT: u32 = 721;

/// The update packets `c` is told within `within`, as blocks about `of`
/// (a packet with none left out).
async fn packets_about(c: &mut harness::Client, of: Guid, within: Duration) -> Vec<Vec<Block>> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < deadline {
        if let Some(bs) = c
            .gets(Duration::from_millis(20), |m| {
                let bs: Vec<Block> = blocks(m)
                    .into_iter()
                    .filter(|b| match b {
                        Block::Create(g, _) | Block::Values(g, _) | Block::Forget(g) => *g == of,
                    })
                    .collect();
                (!bs.is_empty()).then_some(bs)
            })
            .await
        {
            out.push(bs);
        }
    }
    out
}

/// What a death tells `c` of `creature`: it must be one values block in
/// its packet, with no health, and nothing makes it anew or forgets it.
async fn sees_it_die(c: &mut harness::Client, creature: Guid, how: &str) {
    let packets = packets_about(c, creature, Duration::from_millis(1500)).await;
    let died = packets.iter().position(|p| {
        p.iter()
            .any(|b| matches!(b, Block::Values(_, f) if f.get(&UNIT_FIELD_HEALTH) == Some(&0)))
    });
    assert!(
        died.is_some(),
        "{how}: no values update with its death: {packets:?}"
    );
    for p in &packets {
        assert_eq!(p.len(), 1, "{how}: one block about it a packet: {p:?}");
        assert!(
            matches!(p[0], Block::Values(..)),
            "{how}: its death is no create or forget: {packets:?}"
        );
    }
}

/// A Rabbit's death, by a GM's `.kill` or a warrior's blow, is a values
/// update the client sees happen, at once: its health at none, alone in
/// its packet. Its tap, taken by the same blow, came in a second values
/// block about it in the same packet, and the client, though it kept the
/// health, neither played the death nor moved the target's frame from
/// full health (mangos-classic and Thistle Tea give each viewer its tap
/// in the one block).
#[tokio::test]
#[ignore = "extended"]
async fn a_rabbits_death_is_one_values_update_the_client_sees() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut c, me, map) = player(&realm, "B6RB", "Rabbiter", Race::Human, Class::Warrior).await;
    c.send(CMSG_COMPLETE_CINEMATIC {}).await;
    let (first, second) = map
        .query(move |m| {
            ready(m, me, 10, &[]);
            (
                spawned_near(m, me, RABBIT, 2.0, 0x7F_0002),
                spawned_near(m, me, RABBIT, 2.5, 0x7F_0003),
            )
        })
        .await
        .unwrap();
    c.sees_on(&map, first).await;
    c.sees_on(&map, second).await;
    harness::an_update(&map).await;
    c.clear();
    c.send(CMSG_SET_SELECTION { target: first }).await;
    c.say(".kill").await;
    sees_it_die(&mut c, first, ".kill").await;
    // A warrior's blow, sure to land, at one standing before her.
    map.query(move |m| {
        combat::FORCED_OUTCOME.with(|f| f.set(Some(combat::Outcome::Hit)));
        let p = &m.players[&me];
        let (at, o) = (p.position(), p.movement.orientation);
        let before = Vector3d {
            x: at.x + 2.0 * o.cos(),
            y: at.y + 2.0 * o.sin(),
            z: at.z,
        };
        let c = m.creature_mut(second).unwrap();
        c.movement_type = crate::game::map::creature::MOVEMENT_IDLE;
        crate::game::map::scripts::place(m, second, before, o + std::f32::consts::PI);
    })
    .await
    .unwrap();
    c.clear();
    c.send(CMSG_SET_SELECTION { target: second }).await;
    c.send(CMSG_ATTACKSWING { guid: second }).await;
    sees_it_die(&mut c, second, "a blow").await;
    map.query(|_| combat::FORCED_OUTCOME.with(|f| f.set(None)))
        .await
        .unwrap();
}
