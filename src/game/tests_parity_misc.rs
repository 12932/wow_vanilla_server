//! Stage 2g's world against Thistle Tea, through the protocol: a speed
//! change told to those watching once, at the change (a ghost's release, a
//! revive and `.speed` among them); water's interrupts by the terrain's
//! liquid; and Bind's Hearthstone given by the spell.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::{CreatureSpawn, CreatureTemplate, GameData, ItemTemplate};
use crate::game::harness::{self, Client};
use crate::game::map::misc::{MSG_MOVE_SET_RUN_SPEED, MSG_MOVE_SET_SWIM_SPEED};
use crate::game::map::{Map, MapKey};

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

const SPRINT: u32 = 900_100;
const BIND: u32 = 3286;
const HEARTHSTONE: u32 = 6948;
const INNKEEPER: u32 = 900_200;
/// A creature's gift: a create-item spell at any unit, and one naming no
/// target.
const GIFT: u32 = 900_101;
const GIFT_UNTARGETED: u32 = 900_102;
/// A player's own: a create-item spell on themselves.
const CONJURE: u32 = 900_103;
const TOKEN: u32 = 900_300;
const INNKEEPER_COUNTER: u32 = 70;
/// Auras water breaks (a mount's), and leaving it (Aquatic Form's).
const MOUNTED: u32 = 900_104;
const AQUATIC: u32 = 900_105;

static TERRAIN: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("terrain"));
/// Out at sea west of Stranglethorn, and Goldshire's dry road.
const HIGH_SEA: (f32, f32, f32) = (-15000.0, 1024.0, -5.0);
const GOLDSHIRE: (f32, f32, f32) = (-9460.0, 62.0, 56.0);

fn aura(id: u32, aura: u32, points: i32) -> SpellTemplate {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura,
        base_points: points - 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    SpellTemplate {
        id,
        name: format!("Spell {id}"),
        effects,
        equipped_item_class: -1,
        ..Default::default()
    }
}

fn create_item(id: u32, target_a: u32, item: u32) -> SpellTemplate {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 24,
        target_a,
        item_type: item,
        base_points: 0,
        base_dice: 1,
        ..Default::default()
    };
    SpellTemplate {
        id,
        name: format!("Spell {id}"),
        effects,
        equipped_item_class: -1,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.spells.spells.insert(SPRINT, aura(SPRINT, 31, 50));
    // Bind as the client's data has it: bind, then a Hearthstone, both
    // naming no target.
    let mut bind = create_item(BIND, 0, 0);
    bind.name = "Bind".into();
    bind.effects[0] = SpellEffect {
        effect: 11,
        ..Default::default()
    };
    bind.effects[1] = create_item(0, 0, HEARTHSTONE).effects[0];
    d.spells.spells.insert(BIND, bind);
    d.spells.spells.insert(GIFT, create_item(GIFT, 25, TOKEN));
    d.spells
        .spells
        .insert(GIFT_UNTARGETED, create_item(GIFT_UNTARGETED, 0, TOKEN));
    d.spells
        .spells
        .insert(CONJURE, create_item(CONJURE, 1, TOKEN));
    for (id, flags) in [(MOUNTED, 0x80), (AQUATIC, 0x100)] {
        let mut s = aura(id, 4, 1);
        s.aura_interrupt_flags = flags;
        d.spells.spells.insert(id, s);
    }
    for (entry, name, max_count) in [(HEARTHSTONE, "Hearthstone", 1), (TOKEN, "Token", 0)] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                name: name.into(),
                stackable: 20,
                max_count,
                ..Default::default()
            },
        );
    }
    d.creature_templates.insert(
        INNKEEPER,
        CreatureTemplate {
            entry: INNKEEPER,
            name: "Innkeeper".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 30,
            level_max: 30,
            faction: 35,
            npc_flags: 0x80,
            unit_class: 1,
            health_multiplier: 1.0,
            health_max: 500,
            ..Default::default()
        },
    );
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: INNKEEPER_COUNTER,
            entries: vec![INNKEEPER],
            map: 0,
            x: START.0 + 2.0,
            y: START.1,
            z: START.2,
            respawn_min: 600,
            respawn_max: 600,
            ..Default::default()
        }],
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

/// The speed an `MSG_MOVE_SET_*_SPEED` carries, last.
fn speed_of(frame: &[u8]) -> f32 {
    f32::from_le_bytes(frame[frame.len() - 4..].try_into().unwrap())
}

/// The guid an observer's `MSG_MOVE_SET_*_SPEED` is for, packed first.
fn speed_for(frame: &[u8]) -> Guid {
    let mask = frame[0];
    let mut raw = [0u8; 8];
    let mut at = 1;
    for (i, byte) in raw.iter_mut().enumerate() {
        if mask & (1 << i) != 0 {
            *byte = frame[at];
            at += 1;
        }
    }
    Guid::new(u64::from_le_bytes(raw))
}

/// The speeds `watcher` is told of for `of` by `opcode`, waiting for one.
async fn told_speed(watcher: &mut Client, of: Guid, opcode: u16) -> f32 {
    let frame = watcher
        .wait_frame("MSG_MOVE_SET_*_SPEED", opcode, |f| speed_for(f) == of)
        .await;
    speed_of(&frame)
}

/// Whether any `MSG_MOVE_SET_*_SPEED` of `opcodes` reached `watcher`
/// before `of`'s next heartbeat, which `mover` sends now: the map handles
/// a client's packets in order, so what an earlier one sent is in first.
async fn told_before_heartbeat(
    mover: &Client,
    watcher: &mut Client,
    of: Guid,
    opcodes: &[u16],
) -> bool {
    mover.heartbeat(START.0, START.1, START.2).await;
    watcher
        .wait_for("the heartbeat", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(h) if h.guid == of => Some(()),
            _ => None,
        })
        .await;
    // The observer speed packets aren't wow_messages', so come raw.
    watcher.raw.iter().any(|f| {
        let opcode = u16::from_le_bytes([f[2], f[3]]);
        opcodes.contains(&opcode) && speed_for(&f[4..]) == of
    })
}

fn here() -> MovementInfo {
    MovementInfo {
        flags: MovementInfo_MovementFlags::empty(),
        timestamp: 10,
        position: Vector3d {
            x: START.0,
            y: START.1,
            z: START.2,
        },
        orientation: 0.0,
        fall_time: 0.0,
    }
}

#[tokio::test]
async fn a_speed_change_is_told_once_at_the_change_not_at_the_acknowledgement() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(ga).await;
    b.clear();
    on_map(&realm, move |m| {
        crate::game::map::spells::cast_triggered(m, ga, SPRINT, Some(ga));
        crate::game::map::auras::recalc(m, ga);
    })
    .await;
    let speed = told_speed(&mut b, ga, MSG_MOVE_SET_RUN_SPEED).await;
    assert!((speed - 10.5).abs() < 0.01, "{speed}");
    b.raw.clear();
    // Alice's client takes it up; Bob hears nothing more of it (nor of a
    // speed her client makes up).
    a.send(CMSG_FORCE_RUN_SPEED_CHANGE_ACK {
        guid: ga,
        counter: 0,
        info: here(),
        new_speed: 10.5,
    })
    .await;
    a.send(CMSG_FORCE_SWIM_SPEED_CHANGE_ACK {
        guid: ga,
        counter: 0,
        info: here(),
        new_speed: 50.0,
    })
    .await;
    let again = told_before_heartbeat(
        &a,
        &mut b,
        ga,
        &[MSG_MOVE_SET_RUN_SPEED, MSG_MOVE_SET_SWIM_SPEED],
    )
    .await;
    assert!(!again, "the acknowledgement isn't passed on");
}

/// Release the dead `who`, and take up the teleport to where the ghost
/// goes (where they are, without graveyards).
async fn released(who: &mut Client, g: Guid) {
    who.send(CMSG_REPOP_REQUEST {}).await;
    who.wait_for("the teleport to the graveyard", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    who.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
}

#[tokio::test]
async fn a_ghosts_release_and_revive_tell_their_speeds_at_the_change() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(ga).await;
    // Bob is a ghost already, so he sees hers.
    on_map(&realm, move |m| crate::game::map::death::kill(m, gb)).await;
    released(&mut b, gb).await;
    on_map(&realm, move |m| crate::game::map::death::kill(m, ga)).await;
    b.clear();
    // Her release: the ghost's quarter faster, told without her client
    // acknowledging anything.
    released(&mut a, ga).await;
    let run = told_speed(&mut b, ga, MSG_MOVE_SET_RUN_SPEED).await;
    assert!((run - 8.75).abs() < 0.01, "{run}");
    let swim = told_speed(&mut b, ga, MSG_MOVE_SET_SWIM_SPEED).await;
    assert!((swim - 4.722_222 * 1.25).abs() < 0.01, "{swim}");
    // Her revive: back to a run.
    on_map(&realm, move |m| {
        crate::game::map::death::resurrect(m, ga, 1.0);
    })
    .await;
    let run = told_speed(&mut b, ga, MSG_MOVE_SET_RUN_SPEED).await;
    assert!((run - 7.0).abs() < 0.01, "{run}");
    let swim = told_speed(&mut b, ga, MSG_MOVE_SET_SWIM_SPEED).await;
    assert!((swim - 4.722_222).abs() < 0.01, "{swim}");
}

#[tokio::test]
async fn the_speed_commands_tell_those_watching_at_the_change() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(ga).await;
    b.clear();
    a.say(".speed 2").await;
    let run = told_speed(&mut b, ga, MSG_MOVE_SET_RUN_SPEED).await;
    assert!((run - 14.0).abs() < 0.01, "{run}");
    a.say(".modify speed 3").await;
    let run = told_speed(&mut b, ga, MSG_MOVE_SET_RUN_SPEED).await;
    assert!((run - 21.0).abs() < 0.01, "{run}");
}

/// How many of `entry` `g` carries, and where they are bound.
async fn carried_and_bound(
    realm: &Arc<Realm>,
    g: Guid,
    entry: u32,
) -> (u32, Option<crate::game::store::Bind>) {
    on_map(realm, move |m| {
        let p = &m.players[&g];
        (p.count_of(entry), p.row.state.bind)
    })
    .await
}

#[tokio::test]
async fn an_innkeepers_bind_binds_its_target_and_gives_the_hearthstone() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let innkeeper = crate::game::guid::unit(INNKEEPER, INNKEEPER_COUNTER);
    a.sees(innkeeper).await;
    on_map(&realm, move |m| m.players.get_mut(&g).unwrap().area = 87).await;
    // Bind, cast by the innkeeper on her: both its effects are hers.
    on_map(&realm, move |m| {
        crate::game::map::spells::cast_triggered(m, innkeeper, BIND, Some(g))
    })
    .await;
    let (by, area) = a
        .wait_for("SMSG_PLAYERBOUND", |m| match m {
            ServerOpcodeMessage::SMSG_PLAYERBOUND(b) => Some((b.guid, b.area)),
            _ => None,
        })
        .await;
    assert_eq!((by, area.as_int()), (innkeeper, 87));
    let (stones, bind) = carried_and_bound(&realm, g, HEARTHSTONE).await;
    assert_eq!(stones, 1, "the Hearthstone is the one bound's");
    assert_eq!(bind.map(|b| b.zone), Some(87));
    // At the innkeeper's, the same spell: bound again, and no second stone.
    a.clear();
    a.send(CMSG_BINDER_ACTIVATE { guid: innkeeper }).await;
    a.wait_for("SMSG_PLAYERBOUND", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_PLAYERBOUND(_)).then_some(())
    })
    .await;
    a.wait_for("SMSG_GOSSIP_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
    })
    .await;
    let twice = a
        .gets(Duration::from_millis(200), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_PLAYERBOUND(_)).then_some(())
        })
        .await;
    assert!(twice.is_none(), "one binding");
    assert_eq!(carried_and_bound(&realm, g, HEARTHSTONE).await.0, 1);
}

#[tokio::test]
async fn a_creatures_create_item_effect_gives_its_target_the_item() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let innkeeper = crate::game::guid::unit(INNKEEPER, INNKEEPER_COUNTER);
    a.sees(innkeeper).await;
    for spell in [GIFT, GIFT_UNTARGETED] {
        on_map(&realm, move |m| {
            crate::game::map::spells::cast_triggered(m, innkeeper, spell, Some(g))
        })
        .await;
    }
    assert_eq!(carried_and_bound(&realm, g, TOKEN).await.0, 2);
    // Her own on herself is hers too.
    on_map(&realm, move |m| {
        crate::game::map::spells::cast_triggered(m, g, CONJURE, Some(g))
    })
    .await;
    assert_eq!(carried_and_bound(&realm, g, TOKEN).await.0, 3);
}

/// A realm with the real terrain tiles; `None` without them.
fn terrain_realm() -> Option<Arc<Realm>> {
    crate::game::map::terrain::Terrain::open(TERRAIN.as_path(), 0)?;
    let mut cfg = harness::test_config();
    cfg.terrain_dir = Some(TERRAIN.clone());
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Some(Realm::new(cfg, store, startup, data(), None))
}

/// Put `g` at `at` and look at their environment, as the update does.
fn place_and_check(m: &mut Map, g: Guid, at: Vector3d) {
    let p = m.players.get_mut(&g).unwrap();
    p.movement.position = at;
    p.misc.env_checked_at = None;
    crate::game::map::environment::update(m);
}

#[tokio::test]
async fn the_terrains_water_ends_what_water_breaks_without_the_swimming_flag() {
    let Some(realm) = terrain_realm() else {
        return;
    };
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let (on_land, in_sea, out_again) = on_map(&realm, move |m| {
        let has = |m: &Map, spell| m.players[&g].auras.has_spell(spell);
        let sea = m
            .terrain
            .as_mut()
            .unwrap()
            .liquid(Vector3d {
                x: HIGH_SEA.0,
                y: HIGH_SEA.1,
                z: HIGH_SEA.2,
            })
            .expect("the open sea");
        let dry = Vector3d {
            x: GOLDSHIRE.0,
            y: GOLDSHIRE.1,
            z: GOLDSHIRE.2,
        };
        // Mounted on dry land: it stays.
        place_and_check(m, g, dry);
        crate::game::map::spells::cast_triggered(m, g, MOUNTED, Some(g));
        place_and_check(m, g, dry);
        let on_land = has(m, MOUNTED);
        // Into the sea, a yard under, the client saying nothing of swimming.
        let under = Vector3d {
            x: HIGH_SEA.0,
            y: HIGH_SEA.1,
            z: sea.surface - 1.0,
        };
        place_and_check(m, g, under);
        let in_sea = has(m, MOUNTED);
        // In the water's form, back on land.
        crate::game::map::spells::cast_triggered(m, g, AQUATIC, Some(g));
        place_and_check(m, g, under);
        assert!(has(m, AQUATIC), "it stays in the water");
        place_and_check(m, g, dry);
        (on_land, in_sea, has(m, AQUATIC))
    })
    .await;
    assert!(on_land, "dry land breaks nothing");
    assert!(!in_sea, "the sea dismounts");
    assert!(!out_again, "leaving the water ends the water's form");
}
