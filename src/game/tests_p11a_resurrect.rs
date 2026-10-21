//! Round 11a: a resurrection cast at a body, as mangos-classic's
//! `EffectResurrect` takes a corpse target and Thistle Tea's
//! `Player.Resurrection` has it: the body's owner reached on any map, the
//! body checked before the cast and again as it goes off, the dead brought
//! to the caster and risen as they land (`DELAYED_RESURRECT_PLAYER`), and
//! risen where they are when they can't go there.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit as kit;
use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::MapInfo;
use crate::game::data::scripts::MapTemplate;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::death::{self, Death};
use crate::game::map::spells::fail;
use crate::game::map::{Map, MapHandle, MapKey, spells};
use crate::game::tests_p3_entity::{gets_frame, on};
use crate::game::tests_p3_entity_objects::until;

const RAISE: u32 = 997_811;
const THIRTY_YARDS: u32 = 911;
const RAID: u32 = 409;
const SMSG_RESURRECT_REQUEST: u16 = 0x15B;
const SMSG_SPELL_START: u16 = 0x131;

/// A resurrection of 70 health and 135 mana, cast in 2 seconds at 30 yards
/// for 10 mana, as Thistle Tea's test has its own; and a raid.
fn data() -> GameData {
    data_with(|d| {
        d.spells.ranges.insert(THIRTY_YARDS, (0.0, 30.0));
        let mut s = bare(RAISE, 2);
        s.range_index = THIRTY_YARDS;
        s.casting_time_index = CAST_2S;
        s.power_type = 0;
        s.mana_cost = 10;
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: spells::EFFECT_RESURRECT_NEW,
                base_points: 69,
                base_dice: 1,
                misc_value: 135,
                ..Default::default()
            },
        );
        insert(d, [s]);
        d.maps.insert(
            RAID,
            MapInfo {
                id: RAID,
                internal_name: "Raid".into(),
                name: "Raid".into(),
                instance_type: 2,
                player_limit: 0,
            },
        );
        d.scripts.map_templates.insert(
            RAID,
            MapTemplate {
                map: RAID,
                map_type: 2,
                player_limit: 40,
                ..Default::default()
            },
        );
    })
}

/// Players who are no GMs, so a raid turns away one not in a raid group.
fn realm() -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, data(), None)
}

/// Alice the fallen (a mage), and Bob the priest beside her who raises.
async fn two(realm: &Arc<Realm>, n: u32) -> (Client, Guid, Client, Guid, MapHandle) {
    let letter = char::from(b'a' + n as u8);
    let (a, alice, map) = human(
        realm,
        &format!("R11A{n}A"),
        &format!("Fallen{letter}"),
        Class::Mage,
    )
    .await;
    let (b, bob, _) = human(
        realm,
        &format!("R11A{n}B"),
        &format!("Raiser{letter}"),
        Class::Priest,
    )
    .await;
    on(&map, move |m| kit::ready(m, bob, 60, &[RAISE])).await;
    (a, alice, b, bob, map)
}

fn body_of(g: Guid) -> Guid {
    guid::corpse(guid::counter(g))
}

/// The targets the 1.12.1 client sends for a cast at a body.
fn at_body(corpse: Guid) -> SpellCastTargets {
    SpellCastTargets {
        target_flags: SpellCastTargets_SpellCastTargetFlags::empty()
            .set_unit(SpellCastTargets_SpellCastTargetFlags_Unit::Corpse { corpse }),
    }
}

/// `g` dies and releases their spirit, where they are.
async fn dies_and_releases(map: &MapHandle, g: Guid) {
    on(map, move |m| {
        death::kill(m, g);
        death::release(m, g);
    })
    .await;
}

/// Send `c` to `key` at the origin, and wait for it to land.
async fn go(realm: &Arc<Realm>, c: &mut Client, key: MapKey) -> MapHandle {
    let g = c.guid;
    let from = realm.registry.online(g).unwrap().map();
    from.call(move |m| m.teleport(g, key, Vector3d::default(), 0.0));
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
    realm.registry.online(g).unwrap().map()
}

/// The cast under way ends now.
fn finish_cast(m: &mut Map, g: Guid) {
    if let Some(c) = m
        .players
        .get_mut(&g)
        .and_then(|p| p.caster.casting.as_mut())
    {
        c.ends = m.now;
    }
    spells::update(m);
}

/// Bob casts at a body: the result his client is told, `None` for a cast
/// begun.
async fn cast_at_body(b: &mut Client, corpse: Guid) -> Option<u8> {
    b.clear();
    b.send(CMSG_CAST_SPELL {
        spell: RAISE,
        targets: at_body(corpse),
    })
    .await;
    // Refused, the result; begun, its start (the caster's packed GUIDs,
    // then the spell).
    let spell = RAISE.to_le_bytes();
    let r = b
        .wait_frame_of(
            "SMSG_CAST_RESULT or SMSG_SPELL_START",
            &[spells::SMSG_CAST_RESULT, SMSG_SPELL_START],
            |r| r[..4] == spell || r.windows(4).any(|w| w == spell),
        )
        .await;
    (r[..4] == spell && r[4] != 0).then(|| r[5])
}

/// Bob's cast at the body, from its start to its end: what his client is
/// told at the start, and his mana after.
async fn raise(b: &mut Client, map: &MapHandle, corpse: Guid) -> (Option<u8>, u32) {
    let bob = b.guid;
    let started = cast_at_body(b, corpse).await;
    if started.is_none() {
        until(map, "Bob casting", move |m| {
            kit::casting(m, bob) == Some(RAISE)
        })
        .await;
    }
    let mana = on(map, move |m| {
        finish_cast(m, bob);
        kit::power(m, bob, 0)
    })
    .await;
    (started, mana)
}

fn alive(m: &Map, g: Guid) -> bool {
    m.players.get(&g).is_some_and(|p| p.death == Death::Alive)
}

/// Thistle Tea's `validate_repeat/3` "casts at a released body while its
/// owner is on another map", `complete/2` "launches across the ghost's
/// world", and `respond/3` "cross-map recovery waits for its worldport":
/// Bob's cast at the body of Alice, a ghost on Kalimdor, costs its mana and
/// offers her it there; taken, she is sent to Bob's map and rises as she
/// lands, with its health and mana, her corpse bones.
#[tokio::test]
async fn a_resurrection_cast_at_a_body_reaches_its_ghost_on_another_map_and_raises_them_on_arrival()
{
    let realm = realm();
    let (mut a, alice, mut b, bob, map) = two(&realm, 1).await;
    dies_and_releases(&map, alice).await;
    let away = go(&realm, &mut a, MapKey::world(1)).await;
    assert_eq!(away.key, MapKey::world(1));
    let cast_from = on(&map, move |m| m.players[&bob].position()).await;
    let (started, mana) = raise(&mut b, &map, body_of(alice)).await;
    assert_eq!(started, None, "the body is a target");
    assert_eq!(mana, 100_000 - 10, "its cost paid");
    let request = a
        .wait_frame("SMSG_RESURRECT_REQUEST", SMSG_RESURRECT_REQUEST, |_| true)
        .await;
    assert_eq!(&request[..8], &bob.guid().to_le_bytes());
    let offer = on(&away, move |m| {
        let p = &m.players[&alice];
        (p.resurrect_offer, p.resurrect_map)
    })
    .await;
    assert_eq!(offer, (Some((bob, 70, 135)), Some(MapKey::world(0))));
    a.send(CMSG_RESURRECT_RESPONSE {
        guid: bob,
        status: 1,
    })
    .await;
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    // On her way: a ghost still, until her client has the map.
    let waiting = on(&map, move |m| {
        m.players
            .get(&alice)
            .map(|p| (p.death != Death::Alive, p.resurrect_on_arrival))
    })
    .await;
    assert_eq!(waiting, Some((true, Some((70, 135)))));
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    until(&map, "Alice risen on arrival", move |m| alive(m, alice)).await;
    let risen = on(&map, move |m| {
        let p = &m.players[&alice];
        let here = p.position();
        (
            p.fields.get_u32(UNIT_FIELD_HEALTH),
            p.fields.get_u32(UNIT_FIELD_MAXHEALTH),
            p.fields.get_u32(UNIT_FIELD_POWER1),
            p.fields.get_u32(UNIT_FIELD_MAXPOWER1),
            p.resurrect_offer.is_none() && p.resurrect_on_arrival.is_none(),
            p.row.state.corpse.is_none(),
            m.corpses.contains_key(&body_of(alice)),
            (here.x - cast_from.x).abs() < 0.5 && (here.y - cast_from.y).abs() < 0.5,
        )
    })
    .await;
    let (health, max_health, mana, max_mana, spent, no_corpse, body_here, beside_bob) = risen;
    assert_eq!(health, 70.min(max_health));
    assert_eq!(mana, 135.min(max_mana));
    assert!(spent && no_corpse, "the offer spent, the corpse forgotten");
    assert!(!body_here, "the body gone to bones");
    assert!(beside_bob, "where Bob cast it");
}

/// Thistle Tea's `respond/3` "matching arrival revives once and removes
/// all corpse projections": on the same map, Alice taken her resurrection
/// is sent to Bob and rises at her client's acknowledgement of it, not
/// before nor at a stale one, and once.
#[tokio::test]
async fn a_resurrection_taken_on_the_map_raises_at_the_teleports_acknowledgement_once() {
    let realm = realm();
    let (mut a, alice, mut b, bob, map) = two(&realm, 2).await;
    dies_and_releases(&map, alice).await;
    let (started, _) = raise(&mut b, &map, body_of(alice)).await;
    assert_eq!(started, None);
    a.wait_frame("SMSG_RESURRECT_REQUEST", SMSG_RESURRECT_REQUEST, |_| true)
        .await;
    // The release's own teleport, answered first.
    a.clear();
    a.send(CMSG_RESURRECT_RESPONSE {
        guid: bob,
        status: 1,
    })
    .await;
    a.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    let order = a.teleport_order();
    let ack = |counter: u32| MSG_MOVE_TELEPORT_ACK_Client {
        guid: alice,
        movement_counter: counter,
        time: Default::default(),
    };
    let ghost = on(&map, move |m| {
        let p = &m.players[&alice];
        (
            p.death != Death::Alive,
            m.corpses.contains_key(&body_of(alice)),
        )
    })
    .await;
    assert_eq!(ghost, (true, true), "a ghost until she lands");
    a.send(ack(order + 1)).await;
    harness::an_update(&map).await;
    assert!(
        !on(&map, move |m| alive(m, alice)).await,
        "not at a stale one"
    );
    a.send(ack(order)).await;
    until(&map, "Alice risen as she lands", move |m| alive(m, alice)).await;
    let (health, max, body_here) = on(&map, move |m| {
        let p = &m.players[&alice];
        (
            p.fields.get_u32(UNIT_FIELD_HEALTH),
            p.fields.get_u32(UNIT_FIELD_MAXHEALTH),
            m.corpses.contains_key(&body_of(alice)),
        )
    })
    .await;
    assert_eq!(health, 70.min(max));
    assert!(!body_here, "the body gone to bones");
    // Once: the same acknowledgement again raises nothing more.
    on(&map, move |m| kit::set_health(m, alice, 33)).await;
    a.send(ack(order)).await;
    harness::an_update(&map).await;
    assert_eq!(on(&map, move |m| kit::health(m, alice)).await, 33);
}

/// Thistle Tea's `validate_repeat/3` "rejects missing, mismatched, living,
/// and distant bodies": a GUID of no body here, a body out of range, and
/// bones are refused before the cast; a body whose owner has risen on
/// another map offers them nothing.
#[tokio::test]
async fn a_body_aimed_at_must_lie_here_in_reach_and_its_owner_be_dead() {
    let realm = realm();
    let (mut a, alice, mut b, bob, map) = two(&realm, 3).await;
    dies_and_releases(&map, alice).await;
    let away = go(&realm, &mut a, MapKey::world(1)).await;
    // No body of that GUID here.
    let nobody = guid::corpse(guid::counter(alice) + 999);
    assert_eq!(cast_at_body(&mut b, nobody).await, Some(fail::BAD_TARGETS));
    // Out of range.
    on(&map, move |m| {
        m.players.get_mut(&bob).unwrap().movement.position.x += 100.0;
    })
    .await;
    assert_eq!(
        cast_at_body(&mut b, body_of(alice)).await,
        Some(fail::OUT_OF_RANGE)
    );
    on(&map, move |m| {
        m.players.get_mut(&bob).unwrap().movement.position.x -= 100.0;
    })
    .await;
    // Risen on Kalimdor: the body is still here, but she is offered nothing.
    on(&away, move |m| {
        death::resurrect(m, alice, 0.5);
    })
    .await;
    let (started, _) = raise(&mut b, &map, body_of(alice)).await;
    assert_eq!(started, None);
    assert!(
        !gets_frame(&mut a, SMSG_RESURRECT_REQUEST, |_| true).await,
        "the living are offered nothing"
    );
    // Back here, the body is bones: no target.
    go(&realm, &mut a, MapKey::world(0)).await;
    until(&map, "the body bones", move |m| {
        !m.corpses.contains_key(&body_of(alice))
    })
    .await;
    on(&map, move |m| kit::fresh(m, bob)).await;
    assert_eq!(
        cast_at_body(&mut b, body_of(alice)).await,
        Some(fail::BAD_TARGETS)
    );
    let bones = on(&map, move |m| {
        m.corpses
            .values()
            .find(|c| c.owner == alice)
            .map(|c| c.guid)
    })
    .await
    .expect("her bones");
    assert_eq!(cast_at_body(&mut b, bones).await, Some(fail::BAD_TARGETS));
}

/// Thistle Tea's `complete/2` "aborts removed bodies before costs": a body
/// gone while the cast is under way fails it as it ends, before its mana,
/// and offers nothing.
#[tokio::test]
async fn a_body_gone_during_the_cast_fails_it_before_its_cost() {
    let realm = realm();
    let (mut a, alice, mut b, bob, map) = two(&realm, 4).await;
    dies_and_releases(&map, alice).await;
    go(&realm, &mut a, MapKey::world(1)).await;
    assert_eq!(cast_at_body(&mut b, body_of(alice)).await, None);
    until(&map, "Bob casting", move |m| {
        kit::casting(m, bob) == Some(RAISE)
    })
    .await;
    let mana = on(&map, move |m| {
        m.remove_corpse(body_of(alice));
        finish_cast(m, bob);
        kit::power(m, bob, 0)
    })
    .await;
    assert_eq!(mana, 100_000, "nothing paid");
    let failure = b
        .wait_for("SMSG_SPELL_FAILURE", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_FAILURE(f) if f.spell == RAISE => Some(f.result),
            _ => None,
        })
        .await;
    assert_eq!(failure, SpellCastResult::BadTargets);
    assert!(
        !gets_frame(&mut a, SMSG_RESURRECT_REQUEST, |_| true).await,
        "no offer"
    );
}

/// Thistle Tea's `start/2` "denied admission revives in place and clears
/// the corpse": Alice's body lies in a raid's copy, Bob's; she, out of any
/// raid group, takes his resurrection from Stormwind's side of the world,
/// is turned away from the raid, and rises where she is, her body in the
/// raid bones.
#[tokio::test]
async fn one_turned_away_from_the_casters_copy_rises_where_they_are() {
    let realm = realm();
    let (mut a, alice, mut b, bob, _) = two(&realm, 5).await;
    let copy = MapKey {
        map: RAID,
        instance: 1,
    };
    let raid = go(&realm, &mut a, copy).await;
    go(&realm, &mut b, copy).await;
    dies_and_releases(&raid, alice).await;
    let home = go(&realm, &mut a, MapKey::world(0)).await;
    let (started, _) = raise(&mut b, &raid, body_of(alice)).await;
    assert_eq!(started, None);
    a.wait_frame("SMSG_RESURRECT_REQUEST", SMSG_RESURRECT_REQUEST, |_| true)
        .await;
    let before = on(&home, move |m| m.players[&alice].position()).await;
    a.send(CMSG_RESURRECT_RESPONSE {
        guid: bob,
        status: 1,
    })
    .await;
    a.wait_for("SMSG_RAID_GROUP_ONLY", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_RAID_GROUP_ONLY(_)).then_some(())
    })
    .await;
    until(&home, "Alice risen where she is", move |m| alive(m, alice)).await;
    let (here, health, max, waiting) = on(&home, move |m| {
        let p = &m.players[&alice];
        (
            p.position(),
            p.fields.get_u32(UNIT_FIELD_HEALTH),
            p.fields.get_u32(UNIT_FIELD_MAXHEALTH),
            p.resurrect_on_arrival.is_some() || p.resurrect_offer.is_some(),
        )
    })
    .await;
    assert_eq!((here.x, here.y), (before.x, before.y));
    assert_eq!(health, 70.min(max), "its health");
    assert!(!waiting);
    until(&raid, "her body in the raid bones", move |m| {
        !m.corpses.contains_key(&body_of(alice)) && m.corpses.values().any(|c| c.owner == alice)
    })
    .await;
}
