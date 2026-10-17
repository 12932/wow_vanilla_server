//! Thistle Tea's inbound tests (Phase 3, `game/inbound`) that need the real
//! world, through the protocol when its data is here: attacks started and
//! stopped, and Ragefire Chasm's way in and out.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::spells::{self, Targets, wire_targets};
use crate::game::map::{Map, MapHandle, MapKey};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

fn realm() -> Arc<Realm> {
    harness::realm_with(crate::game::data::load::load(DB.as_path()))
}

/// The real world with players who aren't GMs, to be refused what a GM
/// may do.
fn players_realm() -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(
        cfg,
        store,
        startup,
        crate::game::data::load::load(DB.as_path()),
        None,
    )
}

/// A creature of `entry` at `level`, `yards` in front of `g`, with
/// `health`.
fn spawn(m: &mut Map, g: Guid, entry: u32, level: u8, yards: f32, health: u32) -> Guid {
    let data = m.realm.data.clone();
    let template = data
        .creature_template(entry)
        .cloned()
        .expect("the creature");
    let p = &m.players[&g];
    let (pos, o) = (p.position(), p.movement.orientation);
    let at = Vector3d {
        x: pos.x + yards * o.cos(),
        y: pos.y + yards * o.sin(),
        z: pos.z,
    };
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let mut c = Creature::new(
        guid::unit(entry, counter | 0x80_0000),
        &template,
        level,
        at,
        o + std::f32::consts::PI,
    );
    c.fields.set_u32(UNIT_FIELD_MAXHEALTH, health);
    c.fields.set_u32(UNIT_FIELD_HEALTH, health);
    let c = c.with_base();
    m.add_creature(c)
}

fn level(m: &mut Map, g: Guid, level: u8) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::stats::set_level(p, &data, level);
}

async fn on(map: &MapHandle, f: impl FnOnce(&mut Map) + Send + 'static) {
    map.query(f).await.expect("the map answers");
}

/// Wait until the map has handled everything sent before.
async fn settled(c: &mut Client) {
    c.send(CMSG_QUERY_TIME {}).await;
    c.wait_for("SMSG_QUERY_TIME_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_QUERY_TIME_RESPONSE(_)).then_some(())
    })
    .await;
}

/// Thistle Tea's cmsg_attackswing_test and cmsg_attackstop_test: attacking
/// a creature out of reach aims at it without a fight; stopping an attack
/// keeps the swing's timer, lets a waiting Heroic Strike go with its
/// button told it was interrupted, and leaves Auto Shot running.
#[tokio::test]
async fn an_attack_aims_and_stopping_it_keeps_the_timer_and_drops_heroic_strike() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Fighter", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (far, near) = map
        .query(move |m| {
            level(m, g, 10);
            spells::learn(m, g, 78);
            let p = m.players.get_mut(&g).unwrap();
            p.fields.set_u32(UNIT_FIELD_MAXPOWER1 + 1, 1000);
            p.fields.set_u32(UNIT_FIELD_POWER1 + 1, 1000);
            // A kobold too low to mind, out of reach; and one beside.
            (
                spawn(m, g, 6, 1, 25.0, 100_000),
                spawn(m, g, 6, 1, 2.0, 100_000),
            )
        })
        .await
        .unwrap();
    // Out of reach: aimed at, no fight.
    a.send(CMSG_ATTACKSWING { guid: far }).await;
    settled(&mut a).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (attacking, target, fighting) = map
        .query(move |m| {
            let p = &m.players[&g];
            (
                p.combat.attacking && p.combat.victim == far,
                p.fields.get_guid(UNIT_FIELD_TARGET),
                p.fields.has_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                ),
            )
        })
        .await
        .unwrap();
    assert!(attacking, "attacking the far kobold");
    assert_eq!(target, far);
    assert!(!fighting, "no fight before a swing lands");
    a.send(CMSG_ATTACKSTOP {}).await;
    settled(&mut a).await;
    // Beside: Heroic Strike waits for the swing, and Auto Shot runs.
    let swing_at = std::time::Instant::now() + Duration::from_secs(60);
    on(&map, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.combat.next_swing = Some(swing_at);
        p.combat.next_offhand = Some(swing_at);
    })
    .await;
    a.send(CMSG_ATTACKSWING { guid: near }).await;
    a.send(CMSG_CAST_SPELL {
        spell: 78,
        targets: wire_targets(&Targets {
            unit: Some(near),
            ..Default::default()
        }),
    })
    .await;
    settled(&mut a).await;
    let queued = map
        .query(move |m| {
            let p = m.players.get_mut(&g).unwrap();
            p.combat.auto_repeat = Some(crate::game::map::combat::AutoRepeat {
                spell: 75,
                target: near,
                next_at: swing_at,
            });
            p.combat.queued
        })
        .await
        .unwrap();
    assert_eq!(queued, Some(78), "Heroic Strike waits");
    a.clear();
    a.send(CMSG_ATTACKSTOP {}).await;
    a.wait_for("SMSG_ATTACKSTOP", |m| match m {
        ServerOpcodeMessage::SMSG_ATTACKSTOP(s) if s.player == g => Some(()),
        _ => None,
    })
    .await;
    let result = a
        .wait_frame("Heroic Strike's SMSG_CAST_RESULT", 0x130, |f| {
            f[..4] == 78u32.to_le_bytes()
        })
        .await;
    assert_eq!(&result[4..], &[2, 0x23], "failed: interrupted");
    let (attacking, next_swing, queued, auto) = map
        .query(move |m| {
            let c = &m.players[&g].combat;
            (
                c.attacking,
                c.next_swing,
                c.queued,
                c.auto_repeat.map(|a| a.spell),
            )
        })
        .await
        .unwrap();
    assert!(!attacking);
    assert_eq!(next_swing, Some(swing_at), "the swing's timer is kept");
    assert_eq!(queued, None);
    assert_eq!(auto, Some(75), "Auto Shot goes on");
}

/// `c` to `at` on `map_id` (another map by a teleport, this one by a move),
/// and the map they are then on.
async fn go(realm: &Arc<Realm>, c: &mut Client, map_id: u32, at: Vector3d) -> MapHandle {
    let g = c.guid;
    let from = realm.registry.online(g).expect("online").map();
    let here = from.query(|m| m.key.map).await.unwrap();
    if here == map_id {
        on(&from, move |m| m.teleport(g, m.key, at, 0.0)).await;
        c.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
            matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
        })
        .await;
        c.send(MSG_MOVE_TELEPORT_ACK_Client {
            guid: g,
            movement_counter: 0,
            time: Default::default(),
        })
        .await;
        settled(c).await;
        return from;
    }
    from.call(move |m| m.teleport(g, MapKey::world(map_id), at, 0.0));
    arrive(realm, c).await
}

/// The client told of a new world goes there: the map it is now on.
async fn arrive(realm: &Arc<Realm>, c: &mut Client) -> MapHandle {
    let g = c.guid;
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
    realm.registry.online(g).expect("online").map()
}

fn trigger_at(realm: &Realm, id: u32) -> (u32, Vector3d) {
    let t = realm.data.area_triggers.get(&id).expect("the trigger");
    (
        t.map,
        Vector3d {
            x: t.x,
            y: t.y,
            z: t.z,
        },
    )
}

/// Thistle Tea's cmsg_areatrigger_test: Ragefire Chasm turns away a player
/// below level 8 with its message, takes one of 8 into a copy at its
/// entrance, and its exit goes back to Orgrimmar.
#[tokio::test]
async fn ragefire_chasm_turns_the_young_away_and_its_exit_leads_out() {
    if !have_data() {
        return;
    }
    let realm = players_realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Delver", Race::Orc, Class::Warrior).await;
    let (id, at) = trigger_at(&realm, 2230);
    let map = go(&realm, &mut a, id, at).await;
    on(&map, move |m| level(m, g, 7)).await;
    a.clear();
    a.send(CMSG_AREATRIGGER { trigger_id: 2230 }).await;
    let said = a
        .wait_for("SMSG_AREA_TRIGGER_MESSAGE", |m| match m {
            ServerOpcodeMessage::SMSG_AREA_TRIGGER_MESSAGE(t) => Some(t.message.clone()),
            _ => None,
        })
        .await;
    assert_eq!(said, "You must be at least level 8 to enter.");
    assert!(
        a.gets(Duration::from_millis(300), |m| match m {
            ServerOpcodeMessage::SMSG_NEW_WORLD(_)
            | ServerOpcodeMessage::SMSG_TRANSFER_PENDING(_) => Some(()),
            _ => None,
        })
        .await
        .is_none(),
        "not let in"
    );
    on(&map, move |m| level(m, g, 8)).await;
    a.send(CMSG_AREATRIGGER { trigger_id: 2230 }).await;
    let (to, pos) = a
        .wait_for("SMSG_NEW_WORLD", |m| match m {
            ServerOpcodeMessage::SMSG_NEW_WORLD(w) => Some((w.map, w.position)),
            _ => None,
        })
        .await;
    assert_eq!(to.as_int(), 389, "Ragefire Chasm");
    assert!((pos.x - 0.797643).abs() < 0.01 && (pos.y + 8.23429).abs() < 0.01);
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(g).await;
    let copy = realm.registry.online(g).expect("online").map();
    let key = copy.query(|m| m.key).await.unwrap();
    assert_eq!(key.map, 389);
    assert_ne!(key.instance, 0, "a copy of its own");
    // The exit, back to the open world.
    let (id, at) = trigger_at(&realm, 2226);
    go(&realm, &mut a, id, at).await;
    a.clear();
    a.send(CMSG_AREATRIGGER { trigger_id: 2226 }).await;
    let (to, pos) = a
        .wait_for("SMSG_NEW_WORLD", |m| match m {
            ServerOpcodeMessage::SMSG_NEW_WORLD(w) => Some((w.map, w.position)),
            _ => None,
        })
        .await;
    assert_eq!(to.as_int(), 1, "Kalimdor");
    assert!((pos.x - 1814.99).abs() < 0.01 && (pos.y + 4419.23).abs() < 0.01);
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(g).await;
    let out = realm.registry.online(g).expect("online").map();
    assert_eq!(out.query(|m| m.key).await.unwrap(), MapKey::world(1));
}
