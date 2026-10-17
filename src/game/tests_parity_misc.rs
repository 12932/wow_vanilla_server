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
    let healer = crate::game::guid::unit(SPIRIT_HEALER, SPIRIT_HEALER_COUNTER);
    a.sees(healer).await;
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
    assert_eq!(&request[request.len() - 6..], b"Bob\0\0\x01");
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
