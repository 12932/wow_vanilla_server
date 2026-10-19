//! Stages 2g and 2i against Thistle Tea, through the protocol. The world: a
//! speed change told to those watching once, at the change (a ghost's
//! release, a revive and `.speed` among them); water's interrupts by the
//! terrain's liquid; Bind's Hearthstone given by the spell. The second
//! pass, line by line: death and resurrection (the spirit healer, the
//! reclaim, self-resurrection, resurrection spells, the corpse), crafting
//! and disenchanting, locks (lockpicking, gathering, keys and charges),
//! skinning's reasons, fishing and the spell focus.

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
const SPIRIT_HEALER: u32 = 900_201;
const SPIRIT_HEALER_COUNTER: u32 = 71;
const SICKNESS: u32 = 15_007;
/// Reincarnation: the spell, its passive, Improved Reincarnation's second
/// rank, the Ankh.
const REINCARNATION: u32 = 21_169;
const REINCARNATION_PASSIVE: u32 = 20_608;
const IMPROVED_REINCARNATION: u32 = 16_209;
const ANKH: u32 = 17_030;
/// Resurrection's first rank (70 health, 135 mana), and a share of both
/// that doesn't wait out the reclaim delay.
const RESURRECTION: u32 = 2006;
const REBIRTH: u32 = 900_106;

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
        item_type: u64::from(item),
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
    let mut healer = d.creature_templates[&INNKEEPER].clone();
    healer.entry = SPIRIT_HEALER;
    healer.name = "Spirit Healer".into();
    healer.npc_flags = 0x20;
    d.creature_templates.insert(SPIRIT_HEALER, healer);
    let spawn = |guid, entry, dx: f32| CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 600,
        respawn_max: 600,
        ..Default::default()
    };
    d.creature_spawns.insert(
        0,
        vec![
            spawn(INNKEEPER_COUNTER, INNKEEPER, 2.0),
            spawn(SPIRIT_HEALER_COUNTER, SPIRIT_HEALER, -2.0),
        ],
    );
    let mut sickness = aura(SICKNESS, 4, 1);
    sickness.name = "Resurrection Sickness".into();
    d.spells.spells.insert(SICKNESS, sickness);
    // Reincarnation as the client's data has it.
    let mut reincarnation = SpellTemplate {
        id: REINCARNATION,
        name: "Reincarnation".into(),
        category: 1161,
        category_recovery_time: 3_600_000,
        family_name: 11,
        family_flags: 512,
        equipped_item_class: -1,
        ..Default::default()
    };
    reincarnation.reagents[0] = (ANKH, 1);
    reincarnation.effects[0] = SpellEffect {
        effect: 94,
        base_points: 19,
        base_dice: 1,
        ..Default::default()
    };
    d.spells.spells.insert(REINCARNATION, reincarnation);
    let mut passive = aura(REINCARNATION_PASSIVE, 4, 1);
    passive.attributes[0] = 0x40;
    d.spells.spells.insert(REINCARNATION_PASSIVE, passive);
    let mut improved = aura(IMPROVED_REINCARNATION, 107, -1_200_000);
    improved.attributes[0] = 0x40;
    improved.family_name = 11;
    improved.effects[0].misc_value = 11;
    improved.effects[0].item_type = 512;
    improved.effects[1] = SpellEffect {
        misc_value: 8,
        base_points: 19,
        ..improved.effects[0]
    };
    d.spells.spells.insert(IMPROVED_REINCARNATION, improved);
    d.items.insert(
        ANKH,
        ItemTemplate {
            entry: ANKH,
            name: "Ankh".into(),
            stackable: 20,
            ..Default::default()
        },
    );
    let mut resurrection = create_item(RESURRECTION, 21, 0);
    resurrection.effects[0] = SpellEffect {
        effect: 113,
        target_a: 21,
        base_points: 69,
        base_dice: 1,
        misc_value: 135,
        ..Default::default()
    };
    d.spells.spells.insert(RESURRECTION, resurrection);
    let mut rebirth = create_item(REBIRTH, 21, 0);
    rebirth.attributes[3] = 0x10;
    rebirth.effects[0] = SpellEffect {
        effect: 18,
        target_a: 21,
        base_points: 34,
        base_dice: 1,
        ..Default::default()
    };
    d.spells.spells.insert(REBIRTH, rebirth);
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

/// [`on_map`], on the map `g` is on.
async fn on_map_of<T: Send + 'static>(
    realm: &Arc<Realm>,
    g: Guid,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    let key = realm.registry.online(g).expect("online").map().key;
    realm
        .maps
        .get(realm, key)
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
        fall_time: 0,
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
async fn the_speed_commands_tell_those_watching_at_the_change_and_hold() {
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
    // The rate is the run's base: a sprint goes on from it.
    on_map(&realm, move |m| {
        crate::game::map::spells::cast_triggered(m, ga, SPRINT, Some(ga));
        crate::game::map::auras::recalc(m, ga);
    })
    .await;
    let run = told_speed(&mut b, ga, MSG_MOVE_SET_RUN_SPEED).await;
    assert!((run - 31.5).abs() < 0.01, "{run}");
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
#[ignore = "extended"]
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

/// A packet the map answers at once, after whatever `c` sent before it.
async fn fence(c: &mut Client) {
    c.send(MSG_LOOKING_FOR_GROUP_Client {}).await;
    c.wait_for("MSG_LOOKING_FOR_GROUP", |m| {
        matches!(m, ServerOpcodeMessage::MSG_LOOKING_FOR_GROUP(_)).then_some(())
    })
    .await;
}

fn death_of(m: &Map, g: Guid) -> crate::game::map::death::Death {
    m.players[&g].death
}

fn ghost(d: crate::game::map::death::Death) -> bool {
    matches!(d, crate::game::map::death::Death::Ghost { .. })
}

#[tokio::test]
async fn a_spirit_healer_serves_a_ghost_within_reach_and_no_one_else() {
    use crate::game::map::death::Death;
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.say(".levelup 19").await;
    // The living don't see a spirit healer; they may still name one.
    let healer = crate::game::guid::unit(SPIRIT_HEALER, SPIRIT_HEALER_COUNTER);
    let sick = move |m: &Map| m.players[&g].auras.has_spell(SICKNESS);
    // The living: nothing, no wear and no sickness.
    a.send(CMSG_SPIRIT_HEALER_ACTIVATE { guid: healer }).await;
    fence(&mut a).await;
    assert!(
        !on_map(&realm, move |m| sick(m)).await,
        "the living aren't served"
    );
    // The dead, not yet released: nothing.
    on_map(&realm, move |m| crate::game::map::death::kill(m, g)).await;
    a.send(CMSG_SPIRIT_HEALER_ACTIVATE { guid: healer }).await;
    fence(&mut a).await;
    assert!(matches!(
        on_map(&realm, move |m| death_of(m, g)).await,
        Death::Dead { .. }
    ));
    // A ghost ten yards off: out of reach.
    released(&mut a, g).await;
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().movement.position = Vector3d {
            x: START.0 - 12.0,
            y: START.1,
            z: START.2,
        };
    })
    .await;
    a.send(CMSG_SPIRIT_HEALER_ACTIVATE { guid: healer }).await;
    fence(&mut a).await;
    assert!(ghost(on_map(&realm, move |m| death_of(m, g)).await));
    // Beside it: risen, and sick.
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().movement.position = Vector3d {
            x: START.0 - 3.0,
            y: START.1,
            z: START.2,
        };
    })
    .await;
    a.send(CMSG_SPIRIT_HEALER_ACTIVATE { guid: healer }).await;
    fence(&mut a).await;
    let (death, sick) = on_map(&realm, move |m| (death_of(m, g), sick(m))).await;
    assert_eq!((death, sick), (Death::Alive, true));
}

/// Kill `g`, release them, and wait out the reclaim delay.
async fn dead_and_waited(realm: &Arc<Realm>, c: &mut Client, g: Guid) {
    on_map(realm, move |m| crate::game::map::death::kill(m, g)).await;
    released(c, g).await;
    fence(c).await;
    on_map(realm, move |m| {
        m.players.get_mut(&g).unwrap().death = crate::game::map::death::Death::Ghost {
            since: m.now - Duration::from_secs(60),
        };
    })
    .await;
}

fn powers(m: &Map, g: Guid) -> (u32, u32, u32, u32, u32) {
    use crate::game::fields::*;
    let f = &m.players[&g].fields;
    (
        f.get_u32(UNIT_FIELD_HEALTH),
        f.get_u32(UNIT_FIELD_MAXHEALTH),
        f.get_u32(UNIT_FIELD_POWER1 + 1),
        f.get_u32(UNIT_FIELD_POWER1 + 3),
        f.get_u32(UNIT_FIELD_MAXPOWER1 + 3),
    )
}

#[tokio::test]
async fn a_reclaimed_corpse_rises_with_no_rage_and_half_its_energy() {
    use crate::game::fields::*;
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Rogue).await;
    on_map(&realm, move |m| {
        let f = &mut m.players.get_mut(&ga).unwrap().fields;
        f.set_u32(UNIT_FIELD_POWER1 + 1, 500);
        let f = &mut m.players.get_mut(&gb).unwrap().fields;
        f.set_u32(UNIT_FIELD_POWER1 + 3, 20);
    })
    .await;
    for (c, g) in [(&mut a, ga), (&mut b, gb)] {
        dead_and_waited(&realm, c, g).await;
        c.send(CMSG_RECLAIM_CORPSE {
            guid: crate::game::guid::corpse(crate::game::guid::counter(g)),
        })
        .await;
        fence(c).await;
    }
    let (health, max, rage, _, _) = on_map(&realm, move |m| powers(m, ga)).await;
    assert_eq!((health, rage), ((max / 2).max(1), 0), "no rage");
    let (_, _, _, energy, max_energy) = on_map(&realm, move |m| powers(m, gb)).await;
    assert_eq!(energy, max_energy / 2, "half the energy");
}

#[tokio::test]
async fn a_battlegrounds_corpse_is_reclaimed_whole() {
    let mut d = data();
    super::tests_pvp::battleground(&mut d, 489, 2, "WarsongGulch", [900, 901]);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = super::tests_pvp::both_into(
        &realm,
        &mut a,
        &mut b,
        wow_world_messages::vanilla::Map::WarsongGulch,
    )
    .await;
    map.query(move |m| crate::game::map::death::kill(m, ga))
        .await
        .unwrap();
    released(&mut a, ga).await;
    fence(&mut a).await;
    map.query(move |m| {
        m.players.get_mut(&ga).unwrap().death = crate::game::map::death::Death::Ghost {
            since: m.now - Duration::from_secs(60),
        };
    })
    .await
    .unwrap();
    a.send(CMSG_RECLAIM_CORPSE {
        guid: crate::game::guid::corpse(crate::game::guid::counter(ga)),
    })
    .await;
    fence(&mut a).await;
    let (alive, health, max) = map
        .query(move |m| {
            let (health, max, ..) = powers(m, ga);
            (
                death_of(m, ga) == crate::game::map::death::Death::Alive,
                health,
                max,
            )
        })
        .await
        .unwrap();
    assert!(alive);
    assert_eq!(health, max, "whole in a battleground");
}

#[tokio::test]
async fn reincarnation_takes_its_ankh_its_talent_and_its_cooldown() {
    use crate::game::fields::*;
    use crate::game::map::death::Death;
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Thrall", Race::Orc, Class::Shaman).await;
    on_map_of(&realm, g, move |m| {
        crate::game::map::spells::learn(m, g, REINCARNATION_PASSIVE);
        crate::game::map::spells::learn(m, g, IMPROVED_REINCARNATION);
        let _ = crate::game::map::inventory::add_item(m, g, ANKH, 2, false);
        crate::game::map::death::kill(m, g);
    })
    .await;
    let offered = on_map_of(&realm, g, move |m| {
        m.players[&g].fields.get_u32(PLAYER_SELF_RES_SPELL)
    })
    .await;
    assert_eq!(offered, REINCARNATION);
    a.clear();
    a.send(CMSG_SELF_RES {}).await;
    // The cooldown, shortened by the talent's twenty minutes.
    let cooldown = a
        .wait_for("SMSG_SPELL_COOLDOWN", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_COOLDOWN(c) => c
                .cooldowns
                .iter()
                .find(|c| c.id == REINCARNATION)
                .map(|c| c.cooldown_time),
            _ => None,
        })
        .await;
    assert_eq!(cooldown, Duration::from_millis(2_400_000));
    fence(&mut a).await;
    let (death, health, max, ankhs) = on_map_of(&realm, g, move |m| {
        let p = &m.players[&g];
        (
            p.death,
            p.fields.get_u32(UNIT_FIELD_HEALTH),
            p.fields.get_u32(UNIT_FIELD_MAXHEALTH),
            p.count_of(ANKH),
        )
    })
    .await;
    assert_eq!(death, Death::Alive);
    assert_eq!(health, max * 40 / 100, "the talent's twenty more");
    assert_eq!(ankhs, 1);
    // Dead again within the hour: not offered.
    let offered = on_map_of(&realm, g, move |m| {
        crate::game::map::death::kill(m, g);
        m.players[&g].fields.get_u32(PLAYER_SELF_RES_SPELL)
    })
    .await;
    assert_eq!(offered, 0, "on its cooldown");
    // Off it, offered; the Ankh gone by the time it's taken up: nothing.
    on_map_of(&realm, g, move |m| {
        crate::game::map::death::resurrect(m, g, 1.0);
        m.players
            .get_mut(&g)
            .unwrap()
            .caster
            .category_cooldowns
            .clear();
        crate::game::map::death::kill(m, g);
        let i = m.players[&g]
            .items
            .iter()
            .position(|i| i.entry == ANKH)
            .unwrap();
        crate::game::map::inventory::take_item(m, g, i, 1);
    })
    .await;
    a.send(CMSG_SELF_RES {}).await;
    fence(&mut a).await;
    assert!(matches!(
        on_map_of(&realm, g, move |m| death_of(m, g)).await,
        Death::Dead { .. }
    ));
}

#[tokio::test]
async fn a_resurrection_offers_its_health_and_mana_and_is_spent_by_rising() {
    use crate::game::fields::*;
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Mage).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Priest).await;
    b.sees(ga).await;
    let offer = async |a: &mut Client, spell: u32| {
        on_map(&realm, move |m| {
            crate::game::map::spells::cast_triggered(m, gb, spell, Some(ga))
        })
        .await;
        a.wait_frame(
            "SMSG_RESURRECT_REQUEST",
            crate::game::map::spells::SMSG_RESURRECT_REQUEST_OPCODE,
            |_| true,
        )
        .await
    };
    // Resurrection: 70 health and 135 mana, after the reclaim delay.
    on_map(&realm, move |m| crate::game::map::death::kill(m, ga)).await;
    let request = offer(&mut a, RESURRECTION).await;
    assert_eq!(&request[..8], &gb.guid().to_le_bytes());
    // A player caster's name goes empty: the client knows it (mangos).
    assert_eq!(&request[8..], b"\x01\0\0\0\0\0\x01");
    a.send(CMSG_RESURRECT_RESPONSE {
        guid: gb,
        status: 1,
    })
    .await;
    fence(&mut a).await;
    let (health, mana) = on_map(&realm, move |m| {
        let f = &m.players[&ga].fields;
        (f.get_u32(UNIT_FIELD_HEALTH), f.get_u32(UNIT_FIELD_POWER1))
    })
    .await;
    let (max_health, max_mana) = on_map(&realm, move |m| {
        let f = &m.players[&ga].fields;
        (
            f.get_u32(UNIT_FIELD_MAXHEALTH),
            f.get_u32(UNIT_FIELD_MAXPOWER1),
        )
    })
    .await;
    assert_eq!((health, mana), (70.min(max_health), 135.min(max_mana)));
    // A share, without the delay.
    let max = on_map(&realm, move |m| {
        crate::game::map::death::kill(m, ga);
        m.players[&ga].fields.get_u32(UNIT_FIELD_MAXHEALTH)
    })
    .await;
    let request = offer(&mut a, REBIRTH).await;
    assert_eq!(&request[request.len() - 2..], &[0, 0]);
    a.send(CMSG_RESURRECT_RESPONSE {
        guid: gb,
        status: 1,
    })
    .await;
    fence(&mut a).await;
    let health = on_map(&realm, move |m| {
        m.players[&ga].fields.get_u32(UNIT_FIELD_HEALTH)
    })
    .await;
    assert_eq!(health, (max as f32 * 0.35) as u32);
    // An offer outlived by its rising is gone with it.
    on_map(&realm, move |m| {
        crate::game::map::death::kill(m, ga);
        m.players.get_mut(&ga).unwrap().resurrect_offer = Some((gb, 70, 0));
        crate::game::map::death::resurrect(m, ga, 0.5);
        crate::game::map::death::kill(m, ga);
    })
    .await;
    a.send(CMSG_RESURRECT_RESPONSE {
        guid: gb,
        status: 1,
    })
    .await;
    fence(&mut a).await;
    assert!(matches!(
        on_map(&realm, move |m| death_of(m, ga)).await,
        crate::game::map::death::Death::Dead { .. }
    ));
}

#[tokio::test]
async fn a_corpse_wears_its_items_inventory_types() {
    use crate::game::fields::*;
    let mut d = data();
    d.items.insert(
        900_400,
        ItemTemplate {
            entry: 900_400,
            name: "Robe".into(),
            inventory_type: 20,
            display_id: 1234,
            stackable: 1,
            ..Default::default()
        },
    );
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Mage).await;
    let field = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        // Worn on the chest, where the client shows it.
        p.fields
            .set_u32(PLAYER_VISIBLE_ITEM_1_0 + 4 * VISIBLE_ITEM_SIZE, 900_400);
        let data = m.realm.data.clone();
        let p = &m.players[&g];
        let at = p.position();
        crate::game::map::death::Corpse::new(&data, p, at, 0.0)
            .fields
            .get_u32(CORPSE_FIELD_ITEM + 4)
    })
    .await;
    assert_eq!(field >> 24, 20, "the inventory type over the look");
}

// Crafting and disenchanting, line by line with Thistle Tea.

const RECIPE: u32 = 900_110;
const TWICE_LISTED: u32 = 900_111;
const SMELT: u32 = 900_112;
const DISENCHANT: u32 = 13_262;
const ENCHANT: u32 = 900_113;
const CRAFTED: u32 = 900_500;
const FILLER: u32 = 900_501;
const REAGENT: u32 = 900_502;
const SWORD: u32 = 900_503;
const DUST: u32 = 900_504;
const DUSTLESS: u32 = 900_505;
const DUST_TABLE: u32 = 900;
const EMPTY_TABLE: u32 = 901;

fn crafting_data() -> GameData {
    use crate::game::data::LootEntry;
    use crate::game::data::spells::SkillAbility;
    let mut d = data();
    let made = |id, item| {
        let mut s = create_item(id, 1, item);
        s.effects[0].base_points = 0;
        s
    };
    let mut recipe = made(RECIPE, CRAFTED);
    recipe.reagents[0] = (REAGENT, 1);
    d.spells.spells.insert(RECIPE, recipe);
    d.spells
        .spells
        .insert(TWICE_LISTED, made(TWICE_LISTED, CRAFTED));
    d.spells.spells.insert(SMELT, made(SMELT, CRAFTED));
    let mut disenchant = create_item(DISENCHANT, 0, 0);
    disenchant.effects[0] = SpellEffect {
        effect: 99,
        ..Default::default()
    };
    d.spells.spells.insert(DISENCHANT, disenchant);
    let mut enchant = create_item(ENCHANT, 0, 0);
    enchant.effects[0] = SpellEffect {
        effect: 53,
        misc_value: 1,
        ..Default::default()
    };
    d.spells.spells.insert(ENCHANT, enchant);
    let row = |spell, skill, low, high| SkillAbility {
        skill,
        spell,
        trivial_low: low,
        trivial_high: high,
        ..Default::default()
    };
    d.spells
        .skill_abilities
        .insert(RECIPE, vec![row(RECIPE, 164, 5, 10)]);
    d.spells.skill_abilities.insert(
        TWICE_LISTED,
        vec![row(TWICE_LISTED, 164, 5, 10), row(TWICE_LISTED, 164, 5, 10)],
    );
    d.spells
        .skill_abilities
        .insert(SMELT, vec![row(SMELT, 186, 25, 70)]);
    // Disenchant's own row, as the client's data has it.
    d.spells
        .skill_abilities
        .insert(DISENCHANT, vec![row(DISENCHANT, 333, 20, 60)]);
    for (entry, stackable, disenchant_id) in [
        (CRAFTED, 20, 0),
        (FILLER, 1, 0),
        (REAGENT, 20, 0),
        (SWORD, 1, DUST_TABLE),
        (DUSTLESS, 1, EMPTY_TABLE),
        (DUST, 20, 0),
    ] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                name: format!("Item {entry}"),
                class: if disenchant_id != 0 { 2 } else { 7 },
                stackable,
                disenchant_id,
                quality: 2,
                ..Default::default()
            },
        );
    }
    d.loot.disenchant.insert(
        DUST_TABLE,
        vec![LootEntry {
            item: DUST,
            chance: 100.0,
            min: 1,
            max: 1,
            ..Default::default()
        }],
    );
    d.loot.disenchant.insert(EMPTY_TABLE, Vec::new());
    d
}

fn skill(m: &Map, g: Guid, skill: u32) -> u32 {
    crate::game::map::skills::value(&m.players[&g], skill).unwrap_or(0)
}

/// Cast `spell` as `g` would, from no cooldown.
fn cast(m: &mut Map, g: Guid, spell: u32, targets: crate::game::map::spells::Targets) {
    m.now = std::time::Instant::now();
    if let Some(p) = m.players.get_mut(&g) {
        p.caster.gcd_until = None;
        if !p.spells.contains(&spell) {
            p.spells.push(spell);
        }
    }
    crate::game::map::spells::cast(m, g, spell, targets, None);
}

/// An item put in the bank's first slot.
fn banked(m: &mut Map, g: Guid, entry: u32, count: u32) {
    let data = m.realm.data.clone();
    let counter = m
        .realm
        .next_item
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let item = crate::game::map::player::Item::new(
        &data,
        crate::game::guid::item(counter),
        g,
        entry,
        crate::game::store::BAG_PLAYER,
        crate::game::map::inventory::BANK_START,
        count,
    );
    m.players.get_mut(&g).unwrap().items.push(item);
}

/// The reason of the last failed `SMSG_CAST_RESULT` for `spell`.
async fn failed(c: &mut Client, spell: u32) -> u8 {
    let frame = c
        .wait_frame(
            "a failed SMSG_CAST_RESULT",
            crate::game::map::spells::SMSG_CAST_RESULT,
            |f| f[..4] == spell.to_le_bytes() && f[4] == 2,
        )
        .await;
    frame[5]
}

#[tokio::test]
async fn a_recipe_rises_once_its_product_is_stored_and_by_its_first_row() {
    let realm = harness::realm_with(crafting_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let (full, stored, twice, smelted) = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::skills::learn_skill(p, 164, 1);
        crate::game::map::skills::learn_skill(p, 186, 1);
        let _ = crate::game::map::inventory::add_item(m, g, REAGENT, 5, false);
        // The pack full: nothing made, the skill as it was.
        while crate::game::map::inventory::add_item(m, g, FILLER, 1, false).is_ok() {}
        cast(m, g, RECIPE, Default::default());
        let full = (skill(m, g, 164), m.players[&g].count_of(CRAFTED));
        let i = m.players[&g]
            .items
            .iter()
            .position(|i| i.entry == FILLER)
            .unwrap();
        crate::game::map::inventory::take_item(m, g, i, 1);
        cast(m, g, RECIPE, Default::default());
        let stored = (skill(m, g, 164), m.players[&g].count_of(CRAFTED));
        // Listed twice, one roll.
        cast(m, g, TWICE_LISTED, Default::default());
        let twice = skill(m, g, 164);
        // Smelting raises Mining.
        cast(m, g, SMELT, Default::default());
        (full, stored, twice, skill(m, g, 186))
    })
    .await;
    assert_eq!(full, (1, 0), "with the pack full, no product and no rise");
    assert_eq!(stored, (2, 1));
    assert_eq!(twice, 3, "a recipe rises once a cast");
    assert_eq!(smelted, 2, "smelting is Mining's");
}

#[tokio::test]
async fn reagents_come_from_the_bags_and_products_go_to_them() {
    let realm = harness::realm_with(crafting_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        // Not a GM, whom reagents don't bind.
        p.gm = false;
        crate::game::map::skills::learn_skill(p, 164, 1);
        banked(m, g, REAGENT, 5);
        cast(m, g, RECIPE, Default::default());
    })
    .await;
    assert_eq!(
        failed(&mut a, RECIPE).await,
        0x5C,
        "a reagent in the bank isn't to hand"
    );
    let (banked_reagents, made) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (p.count_of(REAGENT), p.count_of(CRAFTED))
    })
    .await;
    assert_eq!((banked_reagents, made), (5, 0));
    // A product doesn't join a stack in the bank.
    let in_bank = on_map(&realm, move |m| {
        banked(m, g, CRAFTED, 1);
        let _ = crate::game::map::inventory::add_item(m, g, REAGENT, 1, false);
        cast(m, g, RECIPE, Default::default());
        let p = &m.players[&g];
        let bank = p
            .items
            .iter()
            .find(|i| i.entry == CRAFTED && !i.carried())
            .map(|i| i.count());
        (bank, p.carried_count_of(CRAFTED))
    })
    .await;
    assert_eq!(in_bank, (Some(1), 1));
}

#[tokio::test]
async fn a_disenchant_rolls_once_shows_as_an_items_loot_and_needs_something_to_give() {
    let realm = harness::realm_with(crafting_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let target = |item| crate::game::map::spells::Targets {
        item: Some(item),
        ..Default::default()
    };
    let guid_of = move |m: &Map, entry| {
        m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == entry)
            .unwrap()
            .guid
    };
    // No Enchanting: too low.
    on_map(&realm, move |m| {
        let _ = crate::game::map::inventory::add_item(m, g, SWORD, 1, false);
        let _ = crate::game::map::inventory::add_item(m, g, DUSTLESS, 1, false);
        let sword = guid_of(m, SWORD);
        cast(m, g, DISENCHANT, target(sword));
    })
    .await;
    assert_eq!(failed(&mut a, DISENCHANT).await, 0x2C);
    // An enchanter: one rise for the one disenchant, and an item's window.
    a.clear();
    let (after, swords) = on_map(&realm, move |m| {
        crate::game::map::skills::learn_skill(m.players.get_mut(&g).unwrap(), 333, 1);
        let sword = guid_of(m, SWORD);
        cast(m, g, DISENCHANT, target(sword));
        (skill(m, g, 333), m.players[&g].count_of(SWORD))
    })
    .await;
    assert_eq!((after, swords), (2, 0), "one roll, the item gone");
    let window = a.wait_frame("SMSG_LOOT_RESPONSE", 0x160, |_| true).await;
    assert_eq!(window[8], 2, "an item's loot");
    // Nothing on its table: try again, the item kept, no rise.
    on_map(&realm, move |m| {
        let item = guid_of(m, DUSTLESS);
        cast(m, g, DISENCHANT, target(item));
    })
    .await;
    assert_eq!(failed(&mut a, DISENCHANT).await, 0x7A);
    let (after, kept) = on_map(&realm, move |m| {
        (skill(m, g, 333), m.players[&g].count_of(DUSTLESS))
    })
    .await;
    assert_eq!((after, kept), (2, 1));
    // An enchantment at an item not there: gone.
    on_map(&realm, move |m| {
        cast(m, g, ENCHANT, target(crate::game::guid::item(999_999)));
    })
    .await;
    assert_eq!(failed(&mut a, ENCHANT).await, 0x26);
}

// Locks: picking, gathering, keys and charges, line by line with Thistle
// Tea.

const PICK_LOCK: u32 = 1804;
const HERB_GATHERING: u32 = 2366;
const GOLDEN_KEY: u32 = 19_649;
const KEY_OPENING: u32 = 900_120;
const SMALL_CHARGE: u32 = 4056;
const JUNKBOX: u32 = 900_600;
const KEY: u32 = 900_601;
const SKELETON_KEY: u32 = 900_602;
const CHARGE: u32 = 900_603;
const TRINKET: u32 = 900_604;
/// Chests: one for a lockpicker of 80, one a key opens alone, one for a
/// charge of 250, one with no lock, and a herb for 50.
const CHEST: u32 = 900_700;
const KEY_CHEST: u32 = 900_701;
const BLAST_CHEST: u32 = 900_702;
const OPEN_CHEST: u32 = 900_703;
const HERB: u32 = 900_704;
const CHEST_LOOT: u32 = 900_710;

fn lock_data() -> GameData {
    use crate::game::data::crafts::LockKey;
    use crate::game::data::items::ItemSpell;
    use crate::game::data::{GameObjectSpawn, GameObjectTemplate, LootEntry};
    let mut d = crafting_data();
    // The spells as the client's data has them: Pick Lock's five over the
    // skill, Herb Gathering's none (its points by level aside), a key's and
    // a charge's own.
    let lock_spell = |id, kind: i32, base: i32| {
        let mut s = create_item(id, 0, 0);
        s.effects[0] = SpellEffect {
            effect: 33,
            misc_value: kind,
            base_points: base,
            base_dice: 1,
            die_sides: 1,
            ..Default::default()
        };
        s
    };
    d.spells
        .spells
        .insert(PICK_LOCK, lock_spell(PICK_LOCK, 1, 4));
    let mut herbs = lock_spell(HERB_GATHERING, 2, -1);
    herbs.effects[0].real_points_per_level = 5.0;
    d.spells.spells.insert(HERB_GATHERING, herbs);
    d.spells
        .spells
        .insert(GOLDEN_KEY, lock_spell(GOLDEN_KEY, 1, 124));
    d.spells
        .spells
        .insert(KEY_OPENING, lock_spell(KEY_OPENING, 0, -1));
    d.spells
        .spells
        .insert(SMALL_CHARGE, lock_spell(SMALL_CHARGE, 16, 149));
    let usable = |entry, name: &str, spell| {
        let mut t = ItemTemplate {
            entry,
            name: name.into(),
            stackable: 1,
            ..Default::default()
        };
        t.spells[0] = ItemSpell {
            id: spell,
            charges: 0,
            cooldown: -1,
            category_cooldown: -1,
            ..Default::default()
        };
        t
    };
    d.items.insert(KEY, usable(KEY, "Key", KEY_OPENING));
    d.items.insert(
        SKELETON_KEY,
        usable(SKELETON_KEY, "Golden Skeleton Key", GOLDEN_KEY),
    );
    d.items.insert(
        CHARGE,
        usable(CHARGE, "Small Seaforium Charge", SMALL_CHARGE),
    );
    d.items.insert(
        JUNKBOX,
        ItemTemplate {
            entry: JUNKBOX,
            name: "Junkbox".into(),
            stackable: 1,
            lock_id: 901,
            ..Default::default()
        },
    );
    d.items.insert(
        TRINKET,
        ItemTemplate {
            entry: TRINKET,
            name: "Trinket".into(),
            stackable: 20,
            ..Default::default()
        },
    );
    let found = vec![LootEntry {
        item: TRINKET,
        chance: 100.0,
        min: 1,
        max: 1,
        ..Default::default()
    }];
    d.loot.item.insert(JUNKBOX, found.clone());
    d.loot.gameobject.insert(CHEST_LOOT, found);
    let skill_key = |kind, skill| LockKey {
        kind: 2,
        index: kind,
        skill,
    };
    d.crafts.locks.insert(900, vec![skill_key(1, 80)]);
    d.crafts.locks.insert(901, vec![skill_key(1, 80)]);
    d.crafts.locks.insert(
        902,
        vec![LockKey {
            kind: 1,
            index: KEY,
            skill: 0,
        }],
    );
    d.crafts.locks.insert(903, vec![skill_key(16, 250)]);
    d.crafts.locks.insert(904, vec![skill_key(2, 50)]);
    let mut spawns = Vec::new();
    for (i, (entry, lock)) in [
        (CHEST, 900),
        (KEY_CHEST, 902),
        (BLAST_CHEST, 903),
        (OPEN_CHEST, 0),
        (HERB, 904),
    ]
    .into_iter()
    .enumerate()
    {
        let mut t = GameObjectTemplate {
            entry,
            kind: 3,
            display_id: 1,
            name: format!("Chest {entry}"),
            size: 1.0,
            ..Default::default()
        };
        t.data[0] = lock;
        t.data[1] = CHEST_LOOT;
        d.gameobject_templates.insert(entry, t);
        spawns.push(GameObjectSpawn {
            guid: 80 + i as u32,
            entry,
            map: 0,
            x: START.0 + 1.0,
            y: START.1 + 1.0 + i as f32 * 0.5,
            z: START.2,
            state: 1,
            respawn_min: 300,
            ..Default::default()
        });
    }
    d.gameobject_spawns.insert(0, spawns);
    d
}

/// The object of `entry` in the map.
fn object(m: &Map, entry: u32) -> Guid {
    m.gameobjects
        .iter()
        .find(|(_, o)| o.entry == entry)
        .map(|(_, o)| o.guid)
        .expect("the object")
}

/// A rogue of thirty, lockpicking at 100.
async fn rogue(realm: &Arc<Realm>) -> (Client, Guid) {
    let mut a = Client::connect(realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Rogue).await;
    a.say(".levelup 29").await;
    a.wait_for("SMSG_LEVELUP_INFO", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LEVELUP_INFO(_)).then_some(())
    })
    .await;
    on_map(realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.gm = false;
        crate::game::map::skills::learn_skill(p, 633, 1);
        crate::game::map::skills::set_value(p, 633, 100);
        crate::game::map::skills::learn_skill(p, 182, 1);
        crate::game::map::skills::set_value(p, 182, 45);
    })
    .await;
    (a, g)
}

fn at_object(object: Guid) -> crate::game::map::spells::Targets {
    crate::game::map::spells::Targets {
        object: Some(object),
        ..Default::default()
    }
}

#[tokio::test]
async fn pick_lock_opens_a_lockbox_in_the_bags_once() {
    let realm = harness::realm_with(lock_data());
    let (mut a, g) = rogue(&realm).await;
    let (unlocked, skill_after) = on_map(&realm, move |m| {
        let _ = crate::game::map::inventory::add_item(m, g, JUNKBOX, 1, false);
        let junkbox = m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == JUNKBOX)
            .unwrap()
            .guid;
        cast(
            m,
            g,
            PICK_LOCK,
            crate::game::map::spells::Targets {
                item: Some(junkbox),
                ..Default::default()
            },
        );
        let p = &m.players[&g];
        let i = p.items.iter().find(|i| i.guid == junkbox).unwrap();
        (
            i.fields.get_u32(crate::game::fields::ITEM_FIELD_FLAGS) & 0x4 != 0,
            skill(m, g, 633),
        )
    })
    .await;
    assert!(unlocked, "picked");
    assert_eq!(skill_after, 101, "the skill rises");
    // It opens at once, as a lock opened.
    let window = a.wait_frame("SMSG_LOOT_RESPONSE", 0x160, |_| true).await;
    assert_eq!(window[8], 2);
    // Picked already: it is open.
    on_map(&realm, move |m| {
        let junkbox = m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == JUNKBOX)
            .unwrap()
            .guid;
        cast(
            m,
            g,
            PICK_LOCK,
            crate::game::map::spells::Targets {
                item: Some(junkbox),
                ..Default::default()
            },
        );
    })
    .await;
    assert_eq!(failed(&mut a, PICK_LOCK).await, 0x06);
}

#[tokio::test]
async fn a_lockbox_can_be_missed_and_stays_locked_till_picked() {
    let realm = harness::realm_with(lock_data());
    let (mut a, g) = rogue(&realm).await;
    // At 75, five over it reaches the need of 80 exactly: a roll may miss
    // (25 in 63), and the box stays locked till one takes. Thirty times.
    let casts = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::skills::set_value(p, 633, 75);
        let _ = crate::game::map::inventory::add_item(m, g, JUNKBOX, 1, false);
        let junkbox = m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == JUNKBOX)
            .unwrap()
            .guid;
        let flags = move |m: &mut Map| {
            m.players
                .get_mut(&g)
                .unwrap()
                .items
                .iter_mut()
                .find(|i| i.guid == junkbox)
                .unwrap()
                .fields
                .get_u32(crate::game::fields::ITEM_FIELD_FLAGS)
        };
        let mut casts = 0;
        for _ in 0..30 {
            for _ in 0..100 {
                casts += 1;
                cast(
                    m,
                    g,
                    PICK_LOCK,
                    crate::game::map::spells::Targets {
                        item: Some(junkbox),
                        ..Default::default()
                    },
                );
                if flags(m) & 0x4 != 0 {
                    break;
                }
            }
            // Locked again, and back at 75, for the next.
            let p = m.players.get_mut(&g).unwrap();
            crate::game::map::skills::set_value(p, 633, 75);
            if let Some(i) = p.items.iter_mut().find(|i| i.guid == junkbox) {
                let f = i.fields.get_u32(crate::game::fields::ITEM_FIELD_FLAGS);
                i.fields
                    .set_u32(crate::game::fields::ITEM_FIELD_FLAGS, f & !0x4);
            }
        }
        casts
    })
    .await;
    assert!(
        (31..3000).contains(&casts),
        "{casts} casts for thirty picks"
    );
    assert_eq!(
        failed(&mut a, PICK_LOCK).await,
        0x7A,
        "a miss is tried again"
    );
}

#[tokio::test]
async fn a_lock_is_opened_by_the_skill_alone_the_key_cast_or_the_charges_strength() {
    let realm = harness::realm_with(lock_data());
    let (mut a, g) = rogue(&realm).await;
    // Herb Gathering at 45 against 50: too low, with no five from its points.
    on_map(&realm, move |m| {
        let herb = object(m, HERB);
        cast(m, g, HERB_GATHERING, at_object(herb));
    })
    .await;
    assert_eq!(failed(&mut a, HERB_GATHERING).await, 0x2C);
    // A key in the bags doesn't let Pick Lock at a lock only the key opens.
    on_map(&realm, move |m| {
        let _ = crate::game::map::inventory::add_item(m, g, KEY, 1, false);
        let chest = object(m, KEY_CHEST);
        cast(m, g, PICK_LOCK, at_object(chest));
    })
    .await;
    assert_eq!(failed(&mut a, PICK_LOCK).await, 0x0A);
    // The key's own spell, cast from it, does.
    let opened = on_map(&realm, move |m| {
        let chest = object(m, KEY_CHEST);
        let key = m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == KEY)
            .unwrap()
            .guid;
        m.players.get_mut(&g).unwrap().caster.gcd_until = None;
        crate::game::map::spells::cast(m, g, KEY_OPENING, at_object(chest), Some(key));
        m.gameobject(chest).is_some_and(|o| o.loot.is_some())
    })
    .await;
    assert!(opened, "the key opens its chest");
    // A skeleton key is its own strength, not the rogue's: 125 against 80
    // opens, but no skill comes of it.
    let (opened, skill_after) = on_map(&realm, move |m| {
        let _ = crate::game::map::inventory::add_item(m, g, SKELETON_KEY, 1, false);
        let chest = object(m, CHEST);
        let key = m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == SKELETON_KEY)
            .unwrap()
            .guid;
        m.players.get_mut(&g).unwrap().caster.gcd_until = None;
        crate::game::map::skills::set_value(m.players.get_mut(&g).unwrap(), 633, 1);
        crate::game::map::spells::cast(m, g, GOLDEN_KEY, at_object(chest), Some(key));
        (
            m.gameobject(chest).is_some_and(|o| o.loot.is_some()),
            skill(m, g, 633),
        )
    })
    .await;
    assert_eq!((opened, skill_after), (true, 1));
    // A charge of 150 doesn't blast a lock of 250.
    on_map(&realm, move |m| {
        let _ = crate::game::map::inventory::add_item(m, g, CHARGE, 1, false);
        let chest = object(m, BLAST_CHEST);
        let charge = m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == CHARGE)
            .unwrap()
            .guid;
        m.players.get_mut(&g).unwrap().caster.gcd_until = None;
        crate::game::map::spells::cast(m, g, SMALL_CHARGE, at_object(chest), Some(charge));
    })
    .await;
    assert_eq!(failed(&mut a, SMALL_CHARGE).await, 0x2C);
    // No lock: already open.
    on_map(&realm, move |m| {
        let chest = object(m, OPEN_CHEST);
        cast(m, g, PICK_LOCK, at_object(chest));
    })
    .await;
    assert_eq!(failed(&mut a, PICK_LOCK).await, 0x06);
    // Eight yards off: out of range.
    on_map(&realm, move |m| {
        let chest = object(m, CHEST);
        let at = m.gameobject(chest).unwrap().position;
        m.players.get_mut(&g).unwrap().movement.position = Vector3d {
            x: at.x + 8.0,
            ..at
        };
        cast(m, g, PICK_LOCK, at_object(chest));
    })
    .await;
    assert_eq!(failed(&mut a, PICK_LOCK).await, 0x59);
}

#[tokio::test]
async fn a_picked_chest_raises_the_skill_once_a_spawn_shows_as_opened_and_goes_when_emptied() {
    let realm = harness::realm_with(lock_data());
    let (mut a, g) = rogue(&realm).await;
    let chest = on_map(&realm, move |m| object(m, CHEST)).await;
    a.clear();
    let first = on_map(&realm, move |m| {
        cast(m, g, PICK_LOCK, at_object(chest));
        skill(m, g, 633)
    })
    .await;
    assert_eq!(first, 101);
    let window = a
        .wait_frame("SMSG_LOOT_RESPONSE", 0x160, |f| {
            f[..8] == chest.guid().to_le_bytes()
        })
        .await;
    assert_eq!(window[8], 2, "a lock opened");
    // Closed with the trinket left, picked again: no more skill from it.
    a.send(CMSG_LOOT_RELEASE { guid: chest }).await;
    fence(&mut a).await;
    let again = on_map(&realm, move |m| {
        cast(m, g, PICK_LOCK, at_object(chest));
        skill(m, g, 633)
    })
    .await;
    assert_eq!(again, 101, "once a spawn");
    // Emptied, it goes until its time.
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.send(CMSG_LOOT_RELEASE { guid: chest }).await;
    fence(&mut a).await;
    let gone = on_map(&realm, move |m| {
        (
            m.gameobject(chest).is_none(),
            m.players[&g].count_of(TRINKET),
        )
    })
    .await;
    assert_eq!(gone, (true, 1));
}

// Fishing, line by line with Thistle Tea.

const FISHING: u32 = 7620;
const BOBBER: u32 = 35_591;
const SCHOOL: u32 = 900_800;
const SCHOOL_LOOT: u32 = 900_801;
const FISH: u32 = 900_802;
static NAV: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("nav"));
/// Open water, ten yards deep, west of Elwynn.
const LAKE: (f32, f32, f32) = (-10_100.0, 460.0, 21.0);

fn fishing_data() -> GameData {
    use crate::game::data::{GameObjectSpawn, GameObjectTemplate, LootEntry};
    let mut d = data();
    let mut fishing = create_item(FISHING, 0, 0);
    fishing.attributes[1] = 0x4;
    fishing.effects[0] = SpellEffect {
        effect: 50,
        misc_value: BOBBER as i32,
        target_a: 39,
        ..Default::default()
    };
    d.spells.spells.insert(FISHING, fishing);
    for (entry, kind) in [(BOBBER, 17u8), (SCHOOL, 25)] {
        let mut t = GameObjectTemplate {
            entry,
            kind,
            display_id: 668,
            name: format!("Object {entry}"),
            size: 1.0,
            ..Default::default()
        };
        if entry == SCHOOL {
            // Thirty yards about it, two to four catches.
            t.data[0] = 30;
            t.data[1] = SCHOOL_LOOT;
            t.data[2] = 2;
            t.data[3] = 4;
        }
        d.gameobject_templates.insert(entry, t);
    }
    d.gameobject_spawns.insert(
        0,
        vec![GameObjectSpawn {
            guid: 90,
            entry: SCHOOL,
            map: 0,
            x: START.0,
            y: START.1,
            z: START.2,
            state: 1,
            respawn_min: 300,
            ..Default::default()
        }],
    );
    let fish = vec![LootEntry {
        item: FISH,
        chance: 100.0,
        min: 1,
        max: 1,
        ..Default::default()
    }];
    d.loot.gameobject.insert(SCHOOL_LOOT, fish.clone());
    d.loot.fishing.insert(0, fish);
    d.crafts.fishing_skill.insert(0, -100);
    d.items.insert(
        FISH,
        ItemTemplate {
            entry: FISH,
            name: "Fish".into(),
            stackable: 20,
            ..Default::default()
        },
    );
    d
}

/// A fisher, and their first line's bobber.
fn cast_a_line(m: &mut Map, g: Guid) -> Option<Guid> {
    if let Some(p) = m.players.get_mut(&g) {
        crate::game::map::skills::learn_skill(p, 356, 1);
    }
    cast(m, g, FISHING, Default::default());
    m.loot_state
        .bobbers
        .iter()
        .find(|b| b.owner == g)
        .map(|b| b.guid)
}

#[tokio::test]
async fn a_line_lasts_its_bite_and_comes_out_with_its_channel() {
    let realm = harness::realm_with(fishing_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.clear();
    let bobber = on_map(&realm, move |m| cast_a_line(m, g)).await.unwrap();
    // The channel is the bite's time and the catch's five seconds.
    let lasts = a
        .wait_for("MSG_CHANNEL_UPDATE", |m| match m {
            ServerOpcodeMessage::MSG_CHANNEL_UPDATE(u) if u.time != 0 => Some(u.time),
            _ => None,
        })
        .await;
    assert!([18_000, 22_000, 28_000, 32_000].contains(&lasts), "{lasts}");
    // Cancelled: the line comes out, nothing said of a fish.
    a.send(CMSG_CANCEL_CHANNELLING { id: FISHING }).await;
    fence(&mut a).await;
    let out = on_map(&realm, move |m| {
        m.gameobject(bobber).is_none() && m.loot_state.bobbers.is_empty()
    })
    .await;
    assert!(out, "the bobber goes with the channel");
    assert!(
        a.gets(Duration::ZERO, |m| {
            matches!(m, ServerOpcodeMessage::SMSG_FISH_NOT_HOOKED).then_some(())
        })
        .await
        .is_none()
    );
    // Left till its time: not hooked.
    on_map(&realm, move |m| {
        cast_a_line(m, g);
        for b in m.loot_state.bobbers.iter_mut() {
            b.bitten = true;
            b.gone_at = m.now;
        }
        crate::game::map::professions::respawn_nodes(m);
    })
    .await;
    a.wait_for("SMSG_FISH_NOT_HOOKED", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_FISH_NOT_HOOKED).then_some(())
    })
    .await;
    // Caught, the window closed with the fish left: the bobber goes.
    let bobber = on_map(&realm, move |m| {
        let bobber = cast_a_line(m, g).unwrap();
        for b in m.loot_state.bobbers.iter_mut() {
            b.bitten = true;
        }
        bobber
    })
    .await;
    a.send(CMSG_GAMEOBJ_USE { guid: bobber }).await;
    a.wait_frame("SMSG_LOOT_RESPONSE", 0x160, |_| true).await;
    a.send(CMSG_LOOT_RELEASE { guid: bobber }).await;
    fence(&mut a).await;
    assert!(on_map(&realm, move |m| m.gameobject(bobber).is_none()).await);
}

#[tokio::test]
async fn a_school_of_fish_holds_between_its_least_and_most_catches() {
    let realm = harness::realm_with(fishing_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let rounds = on_map(&realm, move |m| {
        let mut rounds = Vec::new();
        for _ in 0..12 {
            let mut catches = 0;
            while m.gameobjects.iter().any(|(_, o)| o.entry == SCHOOL) && catches < 10 {
                let bobber = cast_a_line(m, g).unwrap();
                for b in m.loot_state.bobbers.iter_mut() {
                    b.bitten = true;
                }
                crate::game::map::professions::use_object(m, g, bobber);
                crate::game::map::loot::release(m, g, bobber);
                catches += 1;
            }
            rounds.push(catches);
            // Back at once, for the next round.
            for r in m.loot_state.respawns.iter_mut() {
                r.0 = m.now;
            }
            crate::game::map::professions::respawn_nodes(m);
        }
        rounds
    })
    .await;
    assert!(rounds.iter().all(|n| (2..=4).contains(n)), "{rounds:?}");
    assert!(
        rounds.iter().any(|n| *n != 4),
        "not always the most: {rounds:?}"
    );
}

/// A realm with the real navmesh; `None` without it.
fn nav_realm() -> Option<Arc<Realm>> {
    if !NAV.as_path().join("Azeroth.map").exists() {
        return None;
    }
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(NAV.clone());
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Some(Realm::new(cfg, store, startup, fishing_data(), None))
}

#[tokio::test]
#[ignore = "extended"]
async fn a_line_lands_only_in_water_and_fishes_the_waters_zone() {
    let Some(realm) = nav_realm() else {
        return;
    };
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    // Where on the lake every cast lands in water: a spot with deep water
    // all round, to twenty-five yards.
    let found = on_map(&realm, move |m| {
        let nav = m.nav.as_mut().unwrap();
        let (x, y, z) = LAKE;
        for dx in -2..=2 {
            for dy in -2..=2 {
                let at = (x + dx as f32 * 10.0, y + dy as f32 * 10.0);
                let deep = (0..16).all(|k| {
                    let a = k as f32 * std::f32::consts::PI / 8.0;
                    [0.0f32, 10.0, 15.0, 20.0, 25.0].iter().all(|r| {
                        let p = Vector3d {
                            x: at.0 + r * a.cos(),
                            y: at.1 + r * a.sin(),
                            z: z + 30.0,
                        };
                        nav.liquid_surface(p).is_some_and(|s| {
                            nav.floor_under(p.x, p.y, s).is_some_and(|f| s - f >= 2.0)
                        })
                    })
                });
                if deep {
                    let surface = nav
                        .liquid_surface(Vector3d {
                            x: at.0,
                            y: at.1,
                            z: z + 30.0,
                        })
                        .unwrap();
                    return Some(Vector3d {
                        x: at.0,
                        y: at.1,
                        z: surface,
                    });
                }
            }
        }
        None
    })
    .await;
    let Some(spot) = found else {
        panic!("no open water on the lake");
    };
    // In the lake, standing off in the wrong zone: the line lands on the
    // water, and the water's zone is fished.
    let landed = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.movement.position = spot;
        p.zone = 1;
        p.area = 1;
        let bobber = cast_a_line(m, g)?;
        let b = m
            .loot_state
            .bobbers
            .iter()
            .find(|b| b.guid == bobber)?
            .clone();
        let at = m.gameobject(bobber)?.position;
        let water = crate::game::map::world::zone_and_area(m, at);
        Some((at.z, b.zone, b.area, water))
    })
    .await
    .expect("a line in the lake");
    let (z, zone, area, water) = landed;
    assert!((z - spot.z).abs() < 1.0, "on the surface: {z} {}", spot.z);
    assert_eq!(Some((zone, area)), water);
    assert_ne!(zone, 1);
    // On Goldshire's dry road: not fishable.
    a.clear();
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.movement.position = Vector3d {
            x: -9460.0,
            y: 62.0,
            z: 56.0,
        };
        for b in m.loot_state.bobbers.clone() {
            m.remove_gameobject(b.guid);
        }
        m.loot_state.bobbers.clear();
        m.players.get_mut(&g).unwrap().caster.casting = None;
        cast(m, g, FISHING, Default::default());
    })
    .await;
    assert_eq!(failed(&mut a, FISHING).await, 0x34);
}

// Skinning's reasons, and a chest in use when an opening completes.

const SKINNING: u32 = 8613;
const KNIFE: u32 = 7005;
const SKINNABLE_BOAR: u32 = 900_900;
const BARE_KOBOLD: u32 = 900_901;
const LEATHER: u32 = 900_902;

fn skinning_data() -> GameData {
    use crate::game::data::LootEntry;
    let mut d = lock_data();
    let mut skinning = create_item(SKINNING, 25, 0);
    skinning.effects[0] = SpellEffect {
        effect: 95,
        target_a: 25,
        ..Default::default()
    };
    skinning.totems = [KNIFE, 0];
    d.spells.spells.insert(SKINNING, skinning);
    for (entry, name) in [(KNIFE, "Skinning Knife"), (LEATHER, "Leather")] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                name: name.into(),
                stackable: 20,
                ..Default::default()
            },
        );
    }
    let mut spawns = d.creature_spawns.remove(&0).unwrap_or_default();
    for (i, (entry, skin)) in [(SKINNABLE_BOAR, true), (BARE_KOBOLD, false)]
        .into_iter()
        .enumerate()
    {
        let mut t = d.creature_templates[&INNKEEPER].clone();
        t.entry = entry;
        t.npc_flags = 0;
        t.faction = 32;
        t.level_min = 1;
        t.level_max = 1;
        t.loot_id = entry;
        t.skinning_loot_id = if skin { entry } else { 0 };
        d.creature_templates.insert(entry, t);
        let leather = vec![LootEntry {
            item: LEATHER,
            chance: 100.0,
            min: 1,
            max: 1,
            ..Default::default()
        }];
        d.loot.creature.insert(entry, leather.clone());
        d.loot.skinning.insert(entry, leather);
        spawns.push(CreatureSpawn {
            guid: 95 + i as u32,
            entries: vec![entry],
            map: 0,
            x: START.0 + 1.5,
            y: START.1 - 1.0 - i as f32,
            z: START.2,
            respawn_min: 600,
            respawn_max: 600,
            ..Default::default()
        });
    }
    d.creature_spawns.insert(0, spawns);
    d
}

fn at_unit(unit: Guid) -> crate::game::map::spells::Targets {
    crate::game::map::spells::Targets {
        unit: Some(unit),
        ..Default::default()
    }
}

#[tokio::test]
async fn skinning_tells_why_in_thistle_teas_order() {
    let realm = harness::realm_with(skinning_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let boar = crate::game::guid::unit(SKINNABLE_BOAR, 95);
    let kobold = crate::game::guid::unit(BARE_KOBOLD, 96);
    a.sees(boar).await;
    a.sees(kobold).await;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.gm = false;
        crate::game::map::skills::learn_skill(p, 393, 1);
    })
    .await;
    // Alive, and no knife either: alive is why.
    on_map(&realm, move |m| cast(m, g, SKINNING, at_unit(boar))).await;
    assert_eq!(failed(&mut a, SKINNING).await, 0x6E);
    // Dead, its loot on it: a kobold has no skin; a boar's loot is first.
    on_map(&realm, move |m| {
        crate::game::map::combat::kill(m, g, boar);
        crate::game::map::combat::kill(m, g, kobold);
        cast(m, g, SKINNING, at_unit(kobold));
    })
    .await;
    assert_eq!(failed(&mut a, SKINNING).await, 0x74);
    on_map(&realm, move |m| cast(m, g, SKINNING, at_unit(boar))).await;
    assert_eq!(failed(&mut a, SKINNING).await, 0x70);
    // Looted clean, the knife is what's missing.
    on_map(&realm, move |m| {
        if let Some(c) = m.creature_mut(boar) {
            c.loot = None;
        }
        cast(m, g, SKINNING, at_unit(boar));
    })
    .await;
    assert_eq!(failed(&mut a, SKINNING).await, 0x18);
}

#[tokio::test]
async fn an_opening_finds_the_chest_in_use_when_it_completes() {
    let realm = harness::realm_with(lock_data());
    let (_a, ga) = rogue(&realm).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Rogue).await;
    b.say(".levelup 29").await;
    b.wait_for("SMSG_LEVELUP_INFO", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LEVELUP_INFO(_)).then_some(())
    })
    .await;
    on_map(&realm, move |m| {
        let chest = object(m, CHEST);
        // Bob's cast, skilled enough, began before Alice's window opened;
        // it completes after.
        cast(m, ga, PICK_LOCK, at_object(chest));
        let p = m.players.get_mut(&gb).unwrap();
        crate::game::map::skills::learn_skill(p, 633, 1);
        crate::game::map::skills::set_value(p, 633, 100);
        let s = m.realm.data.spells.get(PICK_LOCK).unwrap().clone();
        crate::game::map::professions::execute(m, gb, &s, &at_object(chest), None);
    })
    .await;
    assert_eq!(failed(&mut b, PICK_LOCK).await, 0x15);
}

// A spell focus's reach.

const ANVIL_RECIPE: u32 = 900_130;
const BRAZIER_RECIPE: u32 = 900_131;
const ANVIL: u32 = 900_910;
const BRAZIER: u32 = 900_911;

fn focus_data() -> GameData {
    use crate::game::data::{GameObjectSpawn, GameObjectTemplate};
    let mut d = crafting_data();
    // An anvil reached from five yards, a brazier from two.
    for (recipe, focus, object, radius, dy) in [
        (ANVIL_RECIPE, 1, ANVIL, 5, 10.0),
        (BRAZIER_RECIPE, 2, BRAZIER, 2, 30.0),
    ] {
        let mut s = create_item(recipe, 1, CRAFTED);
        s.effects[0].base_points = 0;
        s.requires_spell_focus = focus;
        d.spells.spells.insert(recipe, s);
        let mut t = GameObjectTemplate {
            entry: object,
            kind: 8,
            display_id: 1,
            name: format!("Focus {object}"),
            size: 1.0,
            ..Default::default()
        };
        t.data[0] = focus;
        t.data[1] = radius;
        d.gameobject_templates.insert(object, t);
        d.gameobject_spawns
            .entry(0)
            .or_default()
            .push(GameObjectSpawn {
                guid: 98 + focus,
                entry: object,
                map: 0,
                x: START.0,
                y: START.1 + dy,
                z: START.2,
                state: 1,
                ..Default::default()
            });
    }
    d
}

#[tokio::test]
async fn a_spell_focus_reaches_past_both_bodies() {
    let realm = harness::realm_with(focus_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    // 5.6 yards from the anvil: within its five and both bodies'.
    let made = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.movement.position = Vector3d {
            x: START.0,
            y: START.1 + 10.0 - 5.6,
            z: START.2,
        };
        cast(m, g, ANVIL_RECIPE, Default::default());
        m.players[&g].count_of(CRAFTED)
    })
    .await;
    assert_eq!(made, 1, "at the anvil");
    // 4.9 yards from the brazier: beyond its two and the bodies'.
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.movement.position = Vector3d {
            x: START.0,
            y: START.1 + 30.0 - 4.9,
            z: START.2,
        };
        cast(m, g, BRAZIER_RECIPE, Default::default());
    })
    .await;
    assert_eq!(failed(&mut a, BRAZIER_RECIPE).await, 0x5E);
}

#[tokio::test]
async fn a_craft_is_told_as_created_and_a_stack_joined_as_no_one_slot() {
    let realm = harness::realm_with(crafting_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let push = async |a: &mut Client| {
        a.wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == CRAFTED => {
                Some((r.creation_type, r.item_slot))
            }
            _ => None,
        })
        .await
    };
    on_map(&realm, move |m| {
        crate::game::map::skills::learn_skill(m.players.get_mut(&g).unwrap(), 164, 1);
        cast(m, g, TWICE_LISTED, Default::default());
    })
    .await;
    let (created, slot) = push(&mut a).await;
    assert_eq!(created, NewItemCreationType::Created);
    assert_ne!(slot, u32::MAX, "a slot of its own");
    on_map(&realm, move |m| {
        cast(m, g, TWICE_LISTED, Default::default())
    })
    .await;
    let (_, slot) = push(&mut a).await;
    assert_eq!(slot, u32::MAX, "onto the stack");
}

#[tokio::test]
async fn a_picked_chests_loot_is_the_pickers_till_they_close_it() {
    let realm = harness::realm_with(lock_data());
    let (mut a, ga) = rogue(&realm).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    let chest = on_map(&realm, move |m| {
        let chest = object(m, CHEST);
        cast(m, ga, PICK_LOCK, at_object(chest));
        chest
    })
    .await;
    a.wait_frame("SMSG_LOOT_RESPONSE", 0x160, |f| {
        f[..8] == chest.guid().to_le_bytes()
    })
    .await;
    a.send(CMSG_LOOT_RELEASE { guid: chest }).await;
    fence(&mut a).await;
    // Neither Bob, who didn't pick it, nor Alice, who closed it, looks in.
    for c in [&mut b, &mut a] {
        c.clear();
        c.send(CMSG_LOOT { guid: chest }).await;
        c.wait_for("SMSG_LOOT_RELEASE_RESPONSE", |m| match m {
            ServerOpcodeMessage::SMSG_LOOT_RELEASE_RESPONSE(r) if r.guid == chest => Some(()),
            _ => None,
        })
        .await;
        fence(c).await;
        let shown = c
            .gets(Duration::ZERO, |m| {
                matches!(m, ServerOpcodeMessage::SMSG_LOOT_RESPONSE(_)).then_some(())
            })
            .await;
        assert!(shown.is_none() && !c.raw.iter().any(|f| f[2..4] == [0x60, 0x01]));
    }
}
