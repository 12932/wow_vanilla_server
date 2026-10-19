//! Round 7's dungeon-wide fights (Thistle Tea's `zone_combat_test.exs`):
//! a script's first pulse and whom it starts on, who may pulse at all,
//! the pulses' three seconds, a hard leash holding them, an aggro event's
//! own pulse, and the script command's abort.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::{AiEvent, EVENT_AGGRO, SF_ABORT_ON_FAILURE, ScriptStep};
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, MapKey, ai, combat, creature_combat};
use crate::game::tests_p3_combat::{self as p3, BRUTE};
use crate::game::tests_p3_entity::step;
use crate::game::tests_p6_wspell::put;

/// It fights its whole dungeon; one with a hard leash of 20 yards; one
/// whose aggro raises its phase and pulses again.
const BOSS: u32 = 3770;
const LEASHED: u32 = 3771;
const CALLER: u32 = 3772;
/// It keeps no threat list.
const LISTLESS: u32 = 3773;
const CALLER_SCRIPT: u32 = 377_201;

fn data() -> GameData {
    let mut d = crate::game::tests_p7_combat::data();
    let mut boss = p3::template(BOSS, 7);
    boss.static_flags2 = 0x2;
    d.creature_templates.insert(BOSS, boss.clone());
    let mut leashed = boss;
    leashed.entry = LEASHED;
    leashed.leash_range = 20.0;
    d.creature_templates.insert(LEASHED, leashed);
    d.creature_templates.insert(CALLER, p3::template(CALLER, 7));
    let mut listless = p3::template(LISTLESS, 7);
    listless.flags_extra = 0x800;
    d.creature_templates.insert(LISTLESS, listless);
    d.scripts.ai_events.insert(
        CALLER,
        vec![AiEvent {
            id: 1,
            creature: CALLER,
            kind: EVENT_AGGRO,
            chance: 100,
            actions: [CALLER_SCRIPT, 0, 0],
            ..Default::default()
        }],
    );
    d.scripts.ai_scripts.insert(
        CALLER_SCRIPT,
        vec![step(49, [1, 0, 0, 0]), step(44, [1, 1, 0, 0])],
    );
    d.maps.insert(
        36,
        MapInfo {
            id: 36,
            internal_name: "DeadminesInstance".into(),
            name: "The Deadmines".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    d
}

/// Alice and Bob (a hunter, with his pet), grouped, in the dungeon; its
/// map.
async fn in_the_dungeon() -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Client,
    MapHandle,
) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Dwarf, Class::Hunter).await;
    a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
    b.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    a.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) if !l.members.is_empty() => Some(()),
        _ => None,
    })
    .await;
    for c in [&mut a, &mut b] {
        let g = c.guid;
        realm
            .registry
            .online(g)
            .unwrap()
            .map()
            .call(move |m| m.teleport(g, MapKey::world(36), at(0.0), 0.0));
        c.wait_for("SMSG_NEW_WORLD", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
        })
        .await;
        c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
        c.sees(g).await;
    }
    let dungeon = realm.registry.online(a.guid).unwrap().map();
    let gb = b.guid;
    dungeon
        .query(move |m| {
            p3::level(m, gb, 60);
            crate::game::map::pets::give_pet(
                m,
                gb,
                crate::game::tests_p7_combat::BEAST_WATCHER,
                60,
            )
            .expect("a pet");
        })
        .await
        .unwrap();
    (realm, a, b, dungeon)
}

fn at(x: f32) -> Vector3d {
    Vector3d { x, y: 0.0, z: 0.0 }
}

/// A creature of `entry`, `x` yards along.
fn foe(m: &mut Map, by: Guid, entry: u32, x: f32) -> Guid {
    let g = p3::spawn(m, by, entry, 20, 1.0);
    put(m, g, at(x));
    if let Some(c) = m.creature_mut(g) {
        c.home = at(x);
    }
    g
}

fn on_list(m: &Map, g: Guid) -> Vec<(Guid, f32)> {
    let mut l = m.creature(g).map(|c| c.threat.clone()).unwrap_or_default();
    l.sort_by_key(|(t, _)| t.guid());
    l
}

fn pet_of(m: &Map, g: Guid) -> Guid {
    crate::game::map::pets::pet_of(m, g).expect("the pet")
}

/// Thistle Tea's "starts on the nearest hostile player and enrolls
/// distant players and their pets": a quiet creature's first scripted
/// pulse fights the nearest player, and puts everyone in the dungeon on
/// its list with no threat, the far and their pets too.
#[tokio::test]
async fn a_scripts_first_pulse_starts_on_the_nearest_and_draws_in_the_far_and_their_pets() {
    let (_realm, a, b, dungeon) = in_the_dungeon().await;
    let (ga, gb) = (a.guid, b.guid);
    let r = dungeon
        .query(move |m| {
            put(m, ga, at(10.0));
            put(m, gb, at(700.0));
            let pet = pet_of(m, gb);
            put(m, pet, at(710.0));
            let brute = foe(m, ga, BRUTE, 0.0);
            let ok = creature_combat::script_zone_pulse(m, brute, true);
            let c = m.creature(brute).unwrap();
            let mut want = vec![(ga, 0.0), (gb, 0.0), (pet, 0.0)];
            want.sort_by_key(|(t, _)| t.guid());
            (
                ok,
                c.combat.victim == ga,
                c.fields.get_guid(crate::game::fields::UNIT_FIELD_TARGET) == ga,
                on_list(m, brute) == want,
                crate::game::tests_p7_combat::in_combat(m, brute),
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (true, true, true, true, true));
}

/// Thistle Tea's "requires a living threat-list owner and a dungeon
/// snapshot" and "dead and non-creature sources honor abort": a dead
/// creature, one a player controls and one keeping no threat list draw no
/// one in, nor does any creature outside a dungeon; one a creature owns
/// does. The command fails for a dead source or a player, and with its
/// abort flag ends the script there.
#[tokio::test]
async fn only_a_living_listed_creature_no_player_controls_pulses_and_only_in_a_dungeon() {
    let (realm, a, _b, dungeon) = in_the_dungeon().await;
    let ga = a.guid;
    let r = dungeon
        .query(move |m| {
            put(m, ga, at(0.0));
            let dead = foe(m, ga, BRUTE, 5.0);
            combat::kill(m, dead, dead);
            let charmed = foe(m, ga, BRUTE, 6.0);
            crate::game::map::pets::charmed(m, ga, charmed, false);
            let listless = foe(m, ga, LISTLESS, 7.0);
            let owner = foe(m, ga, CALLER, 30.0);
            let owned = foe(m, ga, BRUTE, 8.0);
            m.creature_mut(owned).unwrap().owner = Some(owner);
            let mut out = Vec::new();
            for g in [dead, charmed, listless, owned] {
                let ok = creature_combat::script_zone_pulse(m, g, true);
                out.push((ok, !on_list(m, g).is_empty()));
            }
            // The command's abort: a dead source, and a player.
            let pulse = |flags| ScriptStep {
                data_flags: flags,
                ..step(49, [1, 0, 0, 0])
            };
            let aborts = [
                crate::game::map::scripts::execute(m, &pulse(SF_ABORT_ON_FAILURE), dead, ga),
                crate::game::map::scripts::execute(m, &pulse(0), dead, ga),
                crate::game::map::scripts::execute(m, &pulse(SF_ABORT_ON_FAILURE), ga, ga),
                crate::game::map::scripts::execute(m, &pulse(0), ga, ga),
            ];
            (out, aborts)
        })
        .await
        .unwrap();
    assert_eq!(
        r.0,
        vec![(false, false), (true, false), (true, false), (true, true)],
        "(dead, charmed, listless, a creature's)"
    );
    assert_eq!(r.1, [false, true, false, true], "abort stops; else on");
    // Outside a dungeon, no one.
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Carol", Race::Human, Class::Warrior).await;
    let world = realm.maps.get(&realm, MapKey::world(0));
    let outside = world
        .query(move |m| {
            let brute = p3::spawn(m, gc, BRUTE, 20, 3.0);
            creature_combat::script_zone_pulse(m, brute, true);
            on_list(m, brute).is_empty()
        })
        .await
        .unwrap();
    assert!(outside);
}

/// Thistle Tea's "schedules three-second pulses and advances an empty
/// snapshot without spinning" and "a hard leash blocks further
/// recruitment until the creature resets": a boss pulled pulses three
/// seconds later, not before, drawing in who wasn't there; a pulse with no
/// one new goes on three seconds more; and one pulled past its hard leash
/// draws no one in.
#[tokio::test]
async fn pulses_come_every_three_seconds_and_a_hard_leash_holds_them() {
    let (_realm, a, b, dungeon) = in_the_dungeon().await;
    let (ga, gb) = (a.guid, b.guid);
    let r = dungeon
        .query(move |m| {
            put(m, ga, at(0.0));
            put(m, gb, at(5.0));
            // Bob lies dead as Alice pulls.
            let boss = foe(m, ga, BOSS, 2.0);
            crate::game::map::death::kill(m, gb);
            let start = m.now;
            ai::attacked(m, boss, ga);
            let next = m.creature(boss).unwrap().engagement.zone_pulse_at;
            crate::game::map::death::resurrect(m, gb, 1.0);
            let bob_in = |m: &Map| on_list(m, boss).iter().any(|(t, _)| *t == gb);
            m.now = start + Duration::from_millis(2999);
            creature_combat::update(m);
            let early = bob_in(m);
            m.now = start + Duration::from_secs(3);
            creature_combat::update(m);
            let on_time = (
                bob_in(m),
                m.creature(boss).unwrap().engagement.zone_pulse_at,
            );
            m.now = start + Duration::from_secs(6);
            creature_combat::update(m);
            let empty = m.creature(boss).unwrap().engagement.zone_pulse_at;
            ai::enter_evade(m, boss);
            combat::leave_combat(m, ga);
            combat::leave_combat(m, gb);
            // Pulled, then 30 yards from where it began: past 20.
            crate::game::map::death::kill(m, gb);
            let leashed = foe(m, ga, LEASHED, 2.0);
            ai::attacked(m, leashed, ga);
            crate::game::map::death::resurrect(m, gb, 1.0);
            put(m, leashed, at(32.0));
            let now = m.now;
            m.creature_mut(leashed).unwrap().engagement.zone_pulse_at = Some(now);
            creature_combat::update(m);
            let held = (
                on_list(m, leashed).iter().any(|(t, _)| *t == gb),
                m.creature(leashed).unwrap().engagement.zone_pulse_at,
            );
            (start, next, early, on_time, empty, now, held)
        })
        .await
        .unwrap();
    let (start, next, early, on_time, empty, now, held) = r;
    let secs = |s: u64| Some(start + Duration::from_secs(s));
    assert_eq!(next, secs(3), "the next pulse three seconds on");
    assert!(!early, "not before");
    assert_eq!(on_time, (true, secs(6)), "Bob drawn in then, and the next");
    assert_eq!(empty, secs(9), "a pulse with no one new goes on");
    assert_eq!(
        held,
        (false, Some(now + Duration::from_secs(3))),
        "past its hard leash, no one; its pulses go on"
    );
}

/// Thistle Tea's "runs aggro events once even when their action requests
/// another pulse": a creature whose aggro event pulses its dungeon and
/// raises its phase, pulled, draws everyone in and raises its phase
/// once, the pulse's own engagements firing no second aggro.
#[tokio::test]
async fn an_aggro_events_own_pulse_fires_the_aggro_once() {
    let (_realm, a, b, dungeon) = in_the_dungeon().await;
    let (ga, gb) = (a.guid, b.guid);
    let r = dungeon
        .query(move |m| {
            put(m, ga, at(0.0));
            put(m, gb, at(50.0));
            let caller = foe(m, ga, CALLER, 2.0);
            ai::attacked(m, caller, ga);
            let pet = pet_of(m, gb);
            let list: Vec<Guid> = on_list(m, caller).iter().map(|(t, _)| *t).collect();
            (
                crate::game::map::event_ai::phase(m, caller),
                [ga, gb, pet].iter().all(|g| list.contains(g)),
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (1, true));
}

/// Thistle Tea's "requests dungeon observations for nested scripts and
/// preserves blackboard changes": a generic script started by another
/// pulses the dungeon, and the phase the first set before it stays.
#[tokio::test]
async fn a_nested_scripts_pulse_draws_the_dungeon_in_and_keeps_what_came_before() {
    let mut d = data();
    d.scripts.generic.insert(
        377_300,
        vec![step(44, [4, 0, 0, 0]), step(39, [377_301, 0, 0, 0])],
    );
    d.scripts
        .generic
        .insert(377_301, vec![step(49, [1, 0, 0, 0])]);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    realm
        .registry
        .online(ga)
        .unwrap()
        .map()
        .call(move |m| m.teleport(ga, MapKey::world(36), at(0.0), 0.0));
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(ga).await;
    let dungeon = realm.registry.online(ga).unwrap().map();
    let r = dungeon
        .query(move |m| {
            let brute = foe(m, ga, BRUTE, 5.0);
            crate::game::map::scripts::start(
                m,
                crate::game::data::scripts::ScriptTable::Generic,
                377_300,
                brute,
                Guid::zero(),
            );
            (
                crate::game::map::event_ai::phase(m, brute),
                m.creature(brute).unwrap().combat.victim == ga,
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (4, true));
}

/// Thistle Tea's "loads General Rajaxx's initial combat pulse": his
/// `creature_ai_scripts` 1534101 has the pulse, with those fighting
/// already.
#[test]
#[ignore = "extended"]
fn general_rajaxx_pulses_his_dungeon_at_first() {
    let db = harness::real_data("db");
    if !db.join("vmangos.sqlite").exists() {
        return;
    }
    let data = harness::load_real(&db);
    let steps = data
        .scripts
        .ai_scripts
        .get(&1_534_101)
        .expect("Rajaxx's script");
    assert!(
        steps.iter().any(|s| s.command == 49 && s.datalong[0] == 1),
        "{steps:?}"
    );
    assert!(crate::game::map::scripts::command_supported(49));
}
