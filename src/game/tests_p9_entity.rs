//! Round 9's entity tests of what wvs already did, ported from Thistle
//! Tea's `test/game/world/entity/*_test.exs` (docs/parity/matrix.csv): a
//! script's move over a patrol, a walk begun as a creature is put
//! elsewhere, whose level a creature notices, how far a friendly event
//! looks, a dungeon's pulse and its pets, a near spell teleport's fight, a
//! packet between two updates, a respawn's spawned events, a summon's
//! going, and a delayed summon by a moving summoner.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::*;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::combat::{Outcome, dist};
use crate::game::map::creature::Motion;
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::{Map, MapKey, ai, event_ai, pets, scripts};
use crate::game::tests_p3_combat::{
    BEAST, BRUTE, HELPER, START, data, forced_swing, level, setup_with, spawn, template,
};

fn at(dx: f32, dy: f32) -> Vector3d {
    Vector3d {
        x: START.0 + dx,
        y: START.1 + dy,
        z: START.2,
    }
}

/// Where a creature's walk goes, if it walks.
fn bound_for(m: &Map, g: Guid) -> Option<Vector3d> {
    match &m.creature(g)?.motion {
        Motion::Moving(w) => Some(w.destination()),
        Motion::Idle { .. } => None,
    }
}

/// What is left of a creature's walk.
fn walk_left(m: &Map, g: Guid) -> Duration {
    match m.creature(g).map(|c| &c.motion) {
        Some(Motion::Moving(w)) => w.duration.saturating_sub(m.now.duration_since(w.started)),
        _ => Duration::ZERO,
    }
}

fn near(a: Option<Vector3d>, b: Vector3d) -> bool {
    a.is_some_and(|a| dist(a, b) < 0.1)
}

fn in_combat(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT))
}

fn on_threat(m: &Map, creature: Guid, g: Guid) -> bool {
    m.creature(creature)
        .is_some_and(|c| c.threat.iter().any(|(t, _)| *t == g))
}

/// A script's move (step 3) at a creature walking its patrol takes over
/// the walk: it goes to the step's point, and the waypoint it was bound
/// for stays the one it is bound for, reached by nothing; rested there, it
/// goes on with its patrol. A charge that takes it off its patrol (the
/// Deadmines' pirates, `instance_script::pirate_charges`, made idle before
/// their move) stays where it ran to (Thistle Tea's "scripted point
/// movement can take over a running patrol": its plain `move_to` keeps
/// the patrol, `stop_patrol?` ends it).
#[tokio::test]
async fn a_scripts_move_takes_over_a_patrol_and_a_charge_ends_it() {
    use crate::game::map::creature::{MOVEMENT_IDLE, MOVEMENT_WAYPOINT};
    use crate::game::map::world::step_creature_key;
    const PATROL: u32 = 3790;
    let mut d = data();
    d.creature_templates.insert(PATROL, template(PATROL, 7));
    let waypoint = |point: u32, p: Vector3d| Waypoint {
        point,
        x: p.x,
        y: p.y,
        z: p.z,
        orientation: None,
        wait_ms: 0,
        script_id: 0,
    };
    let (first, second) = (at(20.0, 0.0), at(20.0, 20.0));
    d.waypoints_by_entry
        .insert(PATROL, vec![waypoint(1, first), waypoint(2, second)]);
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let r = map
        .query(move |m| {
            let g = spawn(m, ga, PATROL, 20, 5.0);
            let key = m.creature_by_guid[&g];
            let now = m.now;
            let c = m.creature_mut(g).unwrap();
            c.movement_type = MOVEMENT_WAYPOINT;
            c.waypoint = 0;
            c.motion = Motion::Idle { until: Some(now) };
            step_creature_key(m, key);
            let patrolling = bound_for(m, g);
            // The script's move, under way.
            let to = at(-10.0, 5.0);
            let step = ScriptStep {
                command: 3,
                x: to.x,
                y: to.y,
                z: to.z,
                ..Default::default()
            };
            let moved = scripts::execute(m, &step, g, g);
            let scripted = (bound_for(m, g), m.creature(g).unwrap().waypoint);
            // There: no waypoint reached.
            m.now += walk_left(m, g) + Duration::from_millis(10);
            step_creature_key(m, key);
            let c = m.creature(g).unwrap();
            let there = (c.position, c.waypoint, c.last_waypoint);
            // Rested, it patrols on.
            m.now += Duration::from_secs(60);
            step_creature_key(m, key);
            let resumed = bound_for(m, g);
            // A charge off its patrol.
            let charge = at(-20.0, -5.0);
            m.creature_mut(g).unwrap().movement_type = MOVEMENT_IDLE;
            let step = ScriptStep {
                command: 3,
                x: charge.x,
                y: charge.y,
                z: charge.z,
                ..Default::default()
            };
            scripts::execute(m, &step, g, g);
            m.now += walk_left(m, g) + Duration::from_millis(10);
            step_creature_key(m, key);
            m.now += Duration::from_secs(60);
            step_creature_key(m, key);
            let charged = (m.creature(g).unwrap().position, bound_for(m, g));
            (
                patrolling, moved, scripted, there, resumed, charged, to, charge,
            )
        })
        .await
        .unwrap();
    let (patrolling, moved, scripted, there, resumed, charged, to, charge) = r;
    assert!(near(patrolling, first), "on its patrol: {patrolling:?}");
    assert!(moved, "the step moves it");
    assert!(near(scripted.0, to), "to the step's point: {scripted:?}");
    assert_eq!(scripted.1, 0, "still bound for its first waypoint");
    assert!(dist(there.0, to) < 0.1, "at the point: {:?}", there.0);
    assert_eq!((there.1, there.2), (0, None), "no waypoint reached");
    assert!(near(resumed, first), "its patrol goes on: {resumed:?}");
    assert!(dist(charged.0, charge) < 0.1, "charged: {:?}", charged.0);
    assert_eq!(charged.1, None, "and stays, off its patrol");
}

/// A walk begun in the same event as a creature is put elsewhere (a
/// script's teleport) starts from where it was put, and those who see it
/// are told so: its spline's start is the new place (Thistle Tea's
/// "retains and publishes a later same-batch spline from the
/// destination").
#[tokio::test]
async fn a_walk_begun_as_a_creature_is_put_elsewhere_starts_there() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    let g = map
        .query(move |m| spawn(m, ga, BRUTE, 20, 5.0))
        .await
        .unwrap();
    a.sees(g).await;
    a.clear();
    let (put, to, first) = map
        .query(move |m| {
            let here = m.position_of(g).unwrap();
            let put = Vector3d {
                x: here.x + 10.0,
                ..here
            };
            let to = Vector3d {
                x: put.x + 10.0,
                ..put
            };
            scripts::place(m, g, put, 1.0);
            scripts::move_with(m, g, to, scripts::Move::default());
            let first = match &m.creature(g).unwrap().motion {
                Motion::Moving(w) => w.points.first().copied(),
                Motion::Idle { .. } => None,
            };
            (put, to, first)
        })
        .await
        .unwrap();
    assert!(
        near(first, put),
        "the walk from where it was put: {first:?}"
    );
    let (start, end) = a
        .wait_for("its walk", |m| match m {
            ServerOpcodeMessage::SMSG_MONSTER_MOVE(x)
                if x.guid == g
                    && matches!(
                        x.move_type,
                        SMSG_MONSTER_MOVE_MonsterMoveType::Normal { .. }
                    ) =>
            {
                let SMSG_MONSTER_MOVE_MonsterMoveType::Normal { splines, .. } = &x.move_type else {
                    return None;
                };
                Some((x.spline_point, splines.last().copied()))
            }
            _ => None,
        })
        .await;
    assert!(dist(start, put) < 0.01, "told it starts there: {start:?}");
    assert!(near(end, to), "to its end: {end:?}");
}

/// The level a creature notices a unit at (its aggro reach) is the
/// player's whose the unit is: a pet's, its master's; a charmed unit's,
/// its charmer's, as it changes; its own once it is no one's (Thistle
/// Tea's "snapshots the current player controller level without replacing
/// creature level": the unit's own level stays its own). mangos's
/// `GetAttackDistance` takes the unit's own level
/// (`Unit::GetLevelForTarget`); wvs follows Thistle Tea.
#[tokio::test]
async fn a_controlled_unit_is_noticed_at_its_masters_level() {
    const WATCHER: u32 = 3791;
    let mut d = data();
    let mut t = template(WATCHER, 7);
    t.detection_range = 20.0;
    d.creature_templates.insert(WATCHER, t);
    let (realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0, START.1 + 3.0, START.2).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            level(m, gb, 40);
            let watcher = spawn(m, ga, WATCHER, 60, 30.0);
            let unit = spawn(m, ga, HELPER, 10, 3.0);
            let reach = |m: &mut Map, owner: Option<Guid>| {
                m.creature_mut(unit).unwrap().owner = owner;
                let r = ai::attack_distance(m, watcher, unit);
                let own = m
                    .unit_fields(unit)
                    .map(|f| f.get_u32(UNIT_FIELD_LEVEL))
                    .unwrap_or(0);
                (r, own)
            };
            [reach(m, Some(ga)), reach(m, Some(gb)), reach(m, None)]
        })
        .await
        .unwrap();
    // At 60 against 60, twenty yards; at 40, forty; at its own 10, the
    // 25 levels' most, 45.
    assert_eq!(r, [(20.0, 10), (40.0, 10), (45.0, 10)]);
}

const HEALER: u32 = 3792;
const FRIEND: u32 = 3793;
const SET_PHASE_1: u32 = 9_301;

/// A friendly-health event looks as far as its radius says (its second
/// parameter): a friend in a fight 100 yards off, hurt, is found by one
/// of 150; at 160, not (Thistle Tea's "expands the snapshot for
/// behavior-declared observation ranges").
#[tokio::test]
async fn a_friendly_health_event_looks_as_far_as_its_radius() {
    let mut d = data();
    d.creature_templates.insert(HEALER, template(HEALER, 7));
    d.creature_templates.insert(FRIEND, template(FRIEND, 7));
    d.scripts.ai_events.insert(
        HEALER,
        vec![AiEvent {
            kind: EVENT_FRIENDLY_HP,
            chance: 100,
            flags: EF_REPEATABLE,
            params: [0, 150, 0, 0],
            actions: [SET_PHASE_1, 0, 0],
            ..Default::default()
        }],
    );
    d.scripts.ai_scripts.insert(
        SET_PHASE_1,
        vec![ScriptStep {
            id: SET_PHASE_1,
            command: 44,
            datalong: [1, 0, 0, 0],
            ..Default::default()
        }],
    );
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let r = map
        .query(move |m| {
            let healer = spawn(m, ga, HEALER, 20, 3.0);
            ai::attacked(m, healer, ga);
            let friend = spawn(m, ga, FRIEND, 20, 3.0);
            let f = &mut m.creature_mut(friend).unwrap().fields;
            f.set_u32(UNIT_FIELD_HEALTH, 50_000);
            f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, true);
            let from = m.position_of(healer).unwrap();
            let look = |m: &mut Map, yards: f32| {
                let to = Vector3d {
                    y: from.y + yards,
                    ..from
                };
                scripts::place(m, friend, to, 0.0);
                event_ai::set_phase(m, healer, 0);
                if let Some(s) = event_ai::scripted_mut(m, healer) {
                    s.next_tick = None;
                }
                event_ai::tick(m, healer);
                scripts::update(m);
                event_ai::phase(m, healer)
            };
            [look(m, 100.0), look(m, 160.0)]
        })
        .await
        .unwrap();
    assert_eq!(r, [1, 0], "found at 100 yards, not at 160");
}

/// In a dungeon, a creature's zone pulse draws in every player there and
/// their pets: on its threat list, in its fight; its evade lets them all
/// go (Thistle Tea's "pulse and reset deliver incarnation-scoped combat
/// references to every admitted owner").
#[tokio::test]
async fn a_dungeon_pulse_draws_in_the_pets_and_its_evade_lets_them_go() {
    let mut d = data();
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
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    realm
        .registry
        .online(ga)
        .unwrap()
        .map()
        .call(move |m| m.teleport(ga, MapKey::world(36), Vector3d::default(), 0.0));
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(ga).await;
    let dungeon = realm.registry.online(ga).unwrap().map();
    let r = dungeon
        .query(move |m| {
            let boss = spawn(m, ga, BRUTE, 20, 20.0);
            let here = m.players[&ga].position();
            let pet = pets::testing::spawn(m, ga, BEAST, pets::Kind::Demon, 20, here, 0.0);
            m.players.get_mut(&ga).unwrap().owned.pet = Some(pet);
            let pulsed = crate::game::map::creature_combat::script_zone_pulse(m, boss, true);
            let drawn = [ga, pet].map(|g| (on_threat(m, boss, g), in_combat(m, g)));
            ai::enter_evade(m, boss);
            let gone = [ga, pet].map(|g| {
                (
                    on_threat(m, boss, g),
                    m.combat(g).is_some_and(|c| c.attackers.contains(&boss)),
                )
            });
            (pulsed, drawn, gone)
        })
        .await
        .unwrap();
    assert!(r.0, "it pulses");
    assert_eq!(
        r.1,
        [(true, true), (true, true)],
        "Alice and her pet drawn in"
    );
    assert_eq!(r.2, [(false, false), (false, false)], "and let go");
}

/// A near spell teleport (a creature's teleport of its target to it, Blink)
/// keeps the player's fight as it was: in combat, on the creature's threat
/// list, their target the same, their pet out and its bar kept; the client
/// is told of the teleport at the facing it was given (Thistle Tea's
/// "combat relocation preserves combat, threat references, target, and
/// pet"; the user's "a teleport within the map ends the fight but a near
/// spell teleport").
#[tokio::test]
async fn a_near_spell_teleport_keeps_the_fight_its_target_and_pet() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    let (brute, pet) = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            ai::attacked(m, brute, ga);
            forced_swing(m, brute, ga, Outcome::Miss);
            m.players
                .get_mut(&ga)
                .unwrap()
                .fields
                .set_guid(UNIT_FIELD_TARGET, brute);
            let here = m.players[&ga].position();
            let pet = pets::testing::spawn(m, ga, BEAST, pets::Kind::Demon, 20, here, 0.0);
            let p = m.players.get_mut(&ga).unwrap();
            p.owned.pet = Some(pet);
            p.fields.set_guid(UNIT_FIELD_SUMMON, pet);
            (brute, pet)
        })
        .await
        .unwrap();
    a.clear();
    let r = map
        .query(move |m| {
            let to = at(30.0, 0.0);
            m.near_teleport(ga, to, 1.25);
            let p = &m.players[&ga];
            (
                in_combat(m, ga),
                on_threat(m, brute, ga),
                p.fields.get_guid(UNIT_FIELD_TARGET) == brute,
                p.owned.pet == Some(pet) && m.creature(pet).is_some(),
                p.fields.get_guid(UNIT_FIELD_SUMMON) == pet,
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (true, true, true, true, true));
    let facing = a
        .wait_for("MSG_MOVE_TELEPORT_ACK", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => Some(t.info.orientation),
            _ => None,
        })
        .await;
    assert!((facing - 1.25).abs() < 0.001, "{facing}");
    let bar_cleared = a
        .gets(Duration::from_millis(300), |m| match m {
            ServerOpcodeMessage::SMSG_PET_SPELLS(s) if s.pet.is_zero() => Some(()),
            _ => None,
        })
        .await;
    assert!(bar_cleared.is_none(), "the pet's bar stays");
}

/// A packet sent between two updates reaches the client between their
/// blocks: after the one before it, before the one behind it, none merged
/// over it (Thistle Tea's "stops draining at the first other packet and
/// leaves the rest queued" and "writes a packet queued between updates
/// before the update behind it").
#[tokio::test]
async fn a_packet_between_two_updates_comes_between_their_blocks() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    let g = map
        .query(move |m| spawn(m, ga, BRUTE, 20, 5.0))
        .await
        .unwrap();
    a.sees(g).await;
    let emote = |state: u32| {
        let map = map.clone();
        async move {
            map.query(move |m| {
                m.creature_mut(g)
                    .unwrap()
                    .fields
                    .set_u32(UNIT_NPC_EMOTESTATE, state);
            })
            .await
            .unwrap();
        }
    };
    emote(1).await;
    harness::an_update(&map).await;
    // Between the updates: a packet, and a change the next will tell.
    map.query(move |m| {
        m.players[&ga].outbound.send(&SMSG_EMOTE {
            emote: Emote::OneshotWave,
            guid: g,
        });
        m.creature_mut(g)
            .unwrap()
            .fields
            .set_u32(UNIT_NPC_EMOTESTATE, 2);
    })
    .await
    .unwrap();
    let state_of = |m: &ServerOpcodeMessage| {
        harness::values(m).into_iter().find_map(|(of, mask)| {
            (of == g)
                .then(|| {
                    crate::game::tests_bugs6::fields(&mask)
                        .get(&UNIT_NPC_EMOTESTATE)
                        .copied()
                })
                .flatten()
        })
    };
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    let order = loop {
        let sent: Vec<ServerOpcodeMessage> = a
            .captured()
            .iter()
            .filter_map(|f| ServerOpcodeMessage::read_unencrypted(&f[..]).ok())
            .collect();
        let first = sent.iter().position(|m| state_of(m) == Some(1));
        let wave = sent
            .iter()
            .position(|m| matches!(m, ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == g));
        let second = sent.iter().position(|m| state_of(m) == Some(2));
        if let (Some(f), Some(w), Some(s)) = (first, wave, second) {
            break (f, w, s);
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{first:?} {wave:?} {second:?}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert!(order.0 < order.1 && order.1 < order.2, "{order:?}");
}

const SPAWNER: u32 = 3794;
const LANDMARK: u32 = 3795;
const SPAWNER_SPAWN: u32 = 79_300;
const ALONE: u32 = 9_302;
const SET_PHASE_2: u32 = 9_303;

/// A creature's spawned events at its respawn see it where it respawns,
/// its spawn point, not where it died: alone there, its event's condition
/// (no landmark within 30 yards) holds though it died beside one; with a
/// landmark by its spawn point, it doesn't (Thistle Tea's "publishes the
/// respawn position before spawn events read their surroundings").
#[tokio::test]
async fn a_respawns_spawned_events_see_it_at_its_spawn_point() {
    let mut d = data();
    d.creature_templates.insert(SPAWNER, template(SPAWNER, 7));
    d.creature_templates.insert(LANDMARK, template(LANDMARK, 7));
    let home = at(10.0, 0.0);
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: SPAWNER_SPAWN,
            entries: vec![SPAWNER],
            map: 0,
            x: home.x,
            y: home.y,
            z: home.z,
            respawn_min: 300,
            respawn_max: 300,
            ..Default::default()
        }],
    );
    d.scripts.conditions.insert(
        ALONE,
        Condition {
            entry: ALONE,
            kind: 20,
            values: [LANDMARK as i32, 30, 0, 0],
            flags: CF_REVERSE_RESULT,
        },
    );
    d.scripts.ai_events.insert(
        SPAWNER,
        vec![AiEvent {
            kind: EVENT_SPAWNED,
            chance: 100,
            condition: ALONE,
            actions: [SET_PHASE_2, 0, 0],
            ..Default::default()
        }],
    );
    d.scripts.ai_scripts.insert(
        SET_PHASE_2,
        vec![ScriptStep {
            id: SET_PHASE_2,
            command: 44,
            datalong: [2, 0, 0, 0],
            ..Default::default()
        }],
    );
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let r = map
        .query(move |m| {
            let die_far_and_respawn = |m: &mut Map| {
                let g = scripts::creature_of_spawn(m, SPAWNER_SPAWN).expect("the spawn");
                let far = Vector3d {
                    x: home.x + 100.0,
                    ..home
                };
                scripts::place(m, g, far, 0.0);
                event_ai::set_phase(m, g, 0);
                ai::quiet_death_for(m, g, Duration::from_secs(300));
                ai::respawn_now(m, g);
                event_ai::update_despawns(m);
                let g = scripts::creature_of_spawn(m, SPAWNER_SPAWN).expect("back");
                (m.position_of(g).unwrap(), event_ai::phase(m, g))
            };
            // A landmark where it dies.
            let by_corpse = spawn(m, ga, LANDMARK, 20, 1.0);
            scripts::place(
                m,
                by_corpse,
                Vector3d {
                    x: home.x + 102.0,
                    ..home
                },
                0.0,
            );
            let alone = die_far_and_respawn(m);
            // A landmark by its spawn point.
            let by_home = spawn(m, ga, LANDMARK, 20, 1.0);
            scripts::place(
                m,
                by_home,
                Vector3d {
                    x: home.x + 2.0,
                    ..home
                },
                0.0,
            );
            let watched = die_far_and_respawn(m);
            (alone, watched)
        })
        .await
        .unwrap();
    let (alone, watched) = r;
    assert!(
        dist(alone.0, home) < 0.1,
        "back at its spawn: {:?}",
        alone.0
    );
    assert_eq!(alone.1, 2, "alone there, though it died by a landmark");
    assert_eq!(watched.1, 0, "a landmark by its spawn point");
}

const SUMMONER: u32 = 3796;
const MINION: u32 = 3797;
const MINION_ALIVE: u32 = 9_304;
const SET_PHASE_3: u32 = 9_305;

/// A summoner hears of its summon's going (`EVENT_SUMMONED_JUST_DESPAWN`)
/// while the summon is still there: the event's condition on it (alive)
/// reads it as it was, as vmangos's `SummonedCreatureDespawn` comes
/// before the summon leaves the world (Thistle Tea's "a removed summon
/// retains condition facts"). wvs told it after the summon had gone, and
/// a condition on it was never met. Another copy's summon can't reach it:
/// a summoner hears only its own map's summons.
#[tokio::test]
async fn a_summons_going_is_heard_with_the_summon_as_it_was() {
    let mut d = data();
    d.creature_templates.insert(SUMMONER, template(SUMMONER, 7));
    d.creature_templates.insert(MINION, template(MINION, 7));
    d.scripts.conditions.insert(
        MINION_ALIVE,
        Condition {
            entry: MINION_ALIVE,
            kind: 46,
            values: [0; 4],
            flags: 0,
        },
    );
    d.scripts.ai_events.insert(
        SUMMONER,
        vec![AiEvent {
            kind: EVENT_SUMMONED_JUST_DESPAWN,
            chance: 100,
            condition: MINION_ALIVE,
            flags: EF_REPEATABLE,
            params: [MINION as i32, 0, 0, 0],
            actions: [SET_PHASE_3, 0, 0],
            ..Default::default()
        }],
    );
    d.scripts.ai_scripts.insert(
        SET_PHASE_3,
        vec![ScriptStep {
            id: SET_PHASE_3,
            command: 44,
            datalong: [3, 0, 0, 0],
            ..Default::default()
        }],
    );
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let r = map
        .query(move |m| {
            let summoner = spawn(m, ga, SUMMONER, 20, 3.0);
            let here = m.position_of(summoner).unwrap();
            let minion = scripts::summon(m, summoner, MINION, here, 0.0, None).unwrap();
            m.remove_creature(minion);
            scripts::update(m);
            let heard = event_ai::phase(m, summoner);
            // Once: it has gone.
            event_ai::set_phase(m, summoner, 0);
            m.remove_creature(minion);
            scripts::update(m);
            (heard, event_ai::phase(m, summoner))
        })
        .await
        .unwrap();
    assert_eq!(r, (3, 0));
}

const CALLER: u32 = 3798;
const DELAYED_SUMMON: u32 = 9_306;

/// A summon a script's step makes after a delay comes where its summoner
/// is when the step runs: one that has walked on since the script began
/// summons at the end of its walk, not where it began (Thistle Tea's "a
/// delayed summon uses the owner's current spline position").
#[tokio::test]
async fn a_delayed_summon_comes_where_its_summoner_has_walked_to() {
    let mut d = data();
    d.creature_templates.insert(CALLER, template(CALLER, 7));
    d.scripts.generic.insert(
        DELAYED_SUMMON,
        vec![ScriptStep {
            id: DELAYED_SUMMON,
            delay_ms: 1_000,
            command: 10,
            datalong: [HELPER, 0, 0, 0],
            ..Default::default()
        }],
    );
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    let caller = map
        .query(move |m| spawn(m, ga, CALLER, 20, 3.0))
        .await
        .unwrap();
    a.sees(caller).await;
    let (from, to) = map
        .query(move |m| {
            let from = m.position_of(caller).unwrap();
            let to = Vector3d {
                x: from.x + 8.0,
                ..from
            };
            scripts::start(
                m,
                ScriptTable::Generic,
                DELAYED_SUMMON,
                caller,
                Guid::zero(),
            );
            scripts::move_with(
                m,
                caller,
                to,
                scripts::Move {
                    time: Some(Duration::from_millis(400)),
                    ..Default::default()
                },
            );
            (from, to)
        })
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    let summoned = loop {
        let found = map
            .query(move |m| {
                m.creatures
                    .iter()
                    .find(|(_, c)| c.entry == HELPER)
                    .map(|(_, c)| c.position)
            })
            .await
            .unwrap();
        if let Some(p) = found {
            break p;
        }
        assert!(tokio::time::Instant::now() < deadline, "no summon");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert!(
        dist(summoned, to) < 0.5,
        "at the walk's end {to:?}, not its start {from:?}: {summoned:?}"
    );
}

/// A script's object summon (step 76) takes a coordinate the step leaves 0
/// from its source, one by one, and its facing too; the rest are the
/// step's (Thistle Tea's "preserves zero coordinates and fills only
/// unspecified values": its script steps make a 0 unspecified,
/// `unspecified_coordinate`, before the summon fills them).
#[tokio::test]
async fn a_script_object_summon_fills_only_what_its_step_leaves_out() {
    const BONFIRE: u32 = 179_970;
    let mut d = data();
    d.gameobject_templates.insert(
        BONFIRE,
        crate::game::tests_p3_entity_objects::object(BONFIRE, 8, "Bonfire", &[]),
    );
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let r = map
        .query(move |m| {
            let source = spawn(m, ga, BRUTE, 20, 3.0);
            scripts::place(m, source, at(1.0, 2.0), 1.5);
            let here = m.position_of(source).unwrap();
            let step = ScriptStep {
                command: 76,
                datalong: [BONFIRE, 0, 0, 0],
                x: here.x + 5.0,
                ..Default::default()
            };
            let done = scripts::execute(m, &step, source, ga);
            let (p, o) = m
                .gameobjects
                .iter()
                .find(|(_, o)| o.entry == BONFIRE)
                .map(|(_, o)| (o.position, o.orientation))
                .unwrap();
            (done, here, p, o)
        })
        .await
        .unwrap();
    let (done, here, p, o) = r;
    assert!(done);
    assert!((p.x - (here.x + 5.0)).abs() < 0.01, "its x: {p:?}");
    assert!(
        (p.y - here.y).abs() < 0.01 && (p.z - here.z).abs() < 0.01,
        "{p:?}"
    );
    assert!((o - 1.5).abs() < 0.01, "its facing: {o}");
}

/// Opening a lock: an attempt that fails its roll ("try again") gives no
/// skill and opens nothing, no loot window and no mark of a gain had
/// there; a vein out of reach and one no longer there give nothing either
/// (Thistle Tea's "failed attempts and stale targets do not claim a gain
/// or create access"). An attempt that takes opens its vein and is its
/// gain (`tests_loot::a_miner_gathers_a_vein_and_learns_from_it`).
#[tokio::test]
async fn a_failed_or_stale_lock_attempt_gives_no_skill_or_access() {
    // tests_loot's world: a copper vein (1731, lock 38, skill 1), Mining
    // (2575, skill 186) and a pick (2901).
    const VEIN: u32 = 1731;
    const MINING: u32 = 2575;
    const SKILL_MINING: u32 = 186;
    let (_realm, mut a, map) = crate::game::tests_loot::alone("Miner", Class::Warrior).await;
    let g = a.guid;
    a.sees(crate::game::guid::game_object(VEIN, 50)).await;
    map.query(move |m| {
        crate::game::tests_loot::learn(m, g, MINING, SKILL_MINING);
        let _ = crate::game::map::inventory::add_item(m, g, 2901, 1, false);
    })
    .await
    .unwrap();
    // What a vein's attempt could change.
    let state = move |m: &Map, vein: Guid| {
        let p = &m.players[&g];
        (
            crate::game::map::skills::value(p, SKILL_MINING),
            p.looting,
            m.loot_state
                .skilled
                .get(&vein)
                .is_some_and(|who| who.contains(&g)),
            m.gameobject(vein)
                .and_then(|o| o.loot.as_ref())
                .is_some_and(|l| l.looters.contains(&g)),
        )
    };
    // A fresh vein `yards` off, and an attempt at it from a fresh start.
    let attempt = move |m: &mut Map, yards: f32, gone: bool| {
        let t = m.realm.data.gameobject_templates[&VEIN].clone();
        let here = m.players[&g].position();
        let counter = m.object_counter();
        let go = crate::game::map::gameobject::GameObject::from_spawn(
            &t,
            &GameObjectSpawn {
                guid: counter,
                entry: VEIN,
                map: 0,
                x: here.x + yards,
                y: here.y,
                z: here.z,
                state: 1,
                ..Default::default()
            },
        );
        let vein = m.add_gameobject(go);
        if gone {
            m.remove_gameobject(vein);
        }
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::skills::set_value(p, SKILL_MINING, 1);
        p.looting = None;
        p.caster.gcd_until = None;
        p.caster.casting = None;
        let before = state(m, vein);
        crate::game::map::spells::cast(
            m,
            g,
            MINING,
            crate::game::map::spells::Targets {
                object: Some(vein),
                ..Default::default()
            },
            None,
        );
        (vein, before, state(m, vein))
    };
    let (far, gone) = map
        .query(move |m| (attempt(m, 40.0, false), attempt(m, 2.0, true)))
        .await
        .unwrap();
    for (what, (_, before, after)) in [("out of reach", far), ("gone", gone)] {
        assert_eq!(after, before, "nothing from a vein {what}");
    }
    // Attempts at fresh veins until one fails its roll (25 in 63 at a
    // skill of 1, the vein's need).
    let mut failed = None;
    for _ in 0..200 {
        let (vein, before, after) = map.query(move |m| attempt(m, 2.0, false)).await.unwrap();
        if after.1 != Some(vein) {
            failed = Some((before, after));
            break;
        }
    }
    let (before, after) = failed.expect("an attempt fails its roll");
    assert_eq!(before, (Some(1), None, false, false));
    assert_eq!(after, before, "a failed attempt changes nothing");
}
