//! Creature combat as Thistle Tea runs it (docs/AI5_GAPS.md, "Creature
//! combat"): the chase and where it ends, `SMSG_ATTACKSTART`'s timing,
//! facing packets in a fight.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::combat::{self, dist};
use crate::game::map::creature::Motion;
use crate::game::map::{Map, MapHandle, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

const HUMANS: u32 = 1;
/// Hostile to humans.
const PACK: u32 = 14;
/// Hostile to humans, and answering calls for help.
const CALLERS: u32 = 15;

const KOBOLD: u32 = 4100;
/// A kobold that doesn't regenerate.
const WORN: u32 = 4101;
/// A kobold with a keen eye: it notices players forty yards off.
const KEEN: u32 = 4102;
/// One that calls its kind near for help.
const CALLER: u32 = 4103;
/// A group's leader and member; a summoner and what it summons, with
/// EventAI events.
const LEADER: u32 = 4110;
const MEMBER: u32 = 4111;
const SUMMONER: u32 = 4112;
const SUMMONED: u32 = 4113;

/// A burn over time, and a drain channelled.
const BURN: u32 = 95_001;
const DRAIN: u32 = 95_002;
/// Stealth, and two taunts.
const HIDE: u32 = 95_003;
const GOAD: u32 = 95_004;
const MOCK: u32 = 95_005;
/// Sap, a curse, a curse with no threat, a bolt.
const SAP: u32 = 95_006;
const CURSE: u32 = 95_007;
const QUIET_CURSE: u32 = 95_008;
const BOLT: u32 = 95_009;
/// A creature's charm of a player.
const DOMINATE: u32 = 95_010;
/// A mage's bolt, dear in mana.
const DEAR_BOLT: u32 = 95_011;
/// A buff; a debuff evading doesn't take.
const BOON: u32 = 95_012;
const LASTING: u32 = 95_013;
/// A creature's own buff.
const MOCK_BUFF: u32 = 95_014;

/// The spawns: (guid, entry, east, north).
const SPAWNS: &[(u32, u32, f32, f32)] = &[
    // Beside the start.
    (10, KOBOLD, 4.0, 0.0),
    // Off to the east, out of sight: one sitting at its fire, one worn.
    (11, KOBOLD, 30.0, 0.0),
    (12, WORN, 30.0, 6.0),
    // Its pack, near it.
    (13, KOBOLD, 33.0, 0.0),
    (14, KEEN, 30.0, -6.0),
    // Two that call each other, further off.
    (15, CALLER, 60.0, 0.0),
    (16, CALLER, 63.0, 0.0),
    // A leader and its member; a summoner.
    (30, LEADER, -40.0, 0.0),
    (31, MEMBER, -40.0, 3.0),
    (32, SUMMONER, -40.0, 20.0),
    // A row to cast at, out of each other's sight.
    (20, KOBOLD, 50.0, 20.0),
    (21, KOBOLD, 50.0, 30.0),
    (22, KOBOLD, 50.0, 40.0),
    (23, KOBOLD, 50.0, 50.0),
    (24, KOBOLD, 50.0, 60.0),
    (25, KOBOLD, 50.0, 70.0),
];

fn faction(id: u32, ours: u32, enemy_mask: u32) -> FactionTemplate {
    FactionTemplate {
        id,
        faction: id,
        ours,
        friend_mask: ours,
        enemy_mask,
        ..Default::default()
    }
}

fn creature(entry: u32, name: &str, faction: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 2,
        level_max: 2,
        faction,
        unit_class: 1,
        health_multiplier: 100.0,
        damage_multiplier: 0.01,
        base_attack_time: 2000,
        detection_range: 1.0,
        // A little slower than a running player: one who runs gets away.
        speed_run: 0.8,
        ..Default::default()
    }
}

fn at(dx: f32, dy: f32) -> Vector3d {
    Vector3d {
        x: START.0 + dx,
        y: START.1 + dy,
        z: START.2,
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.faction_templates.extend([
        (HUMANS, faction(HUMANS, 0x1, 0x2)),
        (PACK, faction(PACK, 0x2, 0x1)),
        (
            CALLERS,
            FactionTemplate {
                flags: 0x01,
                ..faction(CALLERS, 0x2, 0x1)
            },
        ),
    ]);
    d.class_level_stats.insert(
        (1, 2),
        ClassLevelStats {
            health: 100,
            melee_damage: 1.0,
            ..Default::default()
        },
    );
    d.creature_templates
        .insert(KOBOLD, creature(KOBOLD, "Kobold", PACK));
    let mut worn = creature(WORN, "Worn Kobold", PACK);
    worn.static_flags1 = 0x400;
    // And keeps its buffs through an evade.
    worn.flags_extra = 0x1000;
    d.creature_templates.insert(WORN, worn);
    // Sitting, weapon drawn, eating.
    d.creature_addons.insert(
        11,
        CreatureAddon {
            stand_state: 1,
            sheath_state: 1,
            emote_state: 10,
            ..Default::default()
        },
    );
    let spawns = SPAWNS
        .iter()
        .map(|(g, e, dx, dy)| {
            let p = at(*dx, *dy);
            CreatureSpawn {
                guid: *g,
                entries: vec![*e],
                map: 0,
                x: p.x,
                y: p.y,
                z: p.z,
                respawn_min: 60,
                respawn_max: 60,
                ..Default::default()
            }
        })
        .collect();
    d.creature_spawns.insert(0, spawns);
    let mut keen = creature(KEEN, "Keen Kobold", PACK);
    keen.detection_range = 40.0;
    d.creature_templates.insert(KEEN, keen);
    eventai(&mut d);
    let mut caller = creature(CALLER, "Kobold Caller", CALLERS);
    caller.call_for_help_range = 10.0;
    d.creature_templates.insert(CALLER, caller);
    let s = &mut d.spells;
    s.ranges.insert(1, (0.0, 30.0));
    s.durations.insert(
        9,
        crate::game::data::spells::SpellDuration {
            base: 30_000,
            ..Default::default()
        },
    );
    for (id, channeled) in [(BURN, false), (DRAIN, true)] {
        let mut t = crate::game::data::spells::SpellTemplate {
            id,
            name: format!("Spell {id}"),
            range_index: 1,
            equipped_item_class: -1,
            duration_index: 9,
            ..Default::default()
        };
        t.effects[0] = crate::game::data::spells::SpellEffect {
            effect: 6,
            aura: 3,
            base_points: 1,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            amplitude: 1000,
            ..Default::default()
        };
        if channeled {
            t.attributes[1] = crate::game::data::spells::ATTR_EX_CHANNELED;
        }
        s.spells.insert(id, t);
    }
    for id in [SAP, CURSE, QUIET_CURSE, BOLT, DEAR_BOLT] {
        let mut t = crate::game::data::spells::SpellTemplate {
            id,
            name: format!("Spell {id}"),
            range_index: 1,
            equipped_item_class: -1,
            duration_index: 9,
            ..Default::default()
        };
        t.effects[0] = if matches!(id, BOLT | DEAR_BOLT) {
            crate::game::data::spells::SpellEffect {
                effect: 2,
                base_points: 9,
                base_dice: 1,
                die_sides: 1,
                target_a: 6,
                ..Default::default()
            }
        } else {
            crate::game::data::spells::SpellEffect {
                effect: 6,
                aura: 33,
                base_points: -50,
                base_dice: 1,
                die_sides: 1,
                target_a: 6,
                ..Default::default()
            }
        };
        if id == SAP {
            t.family_name = 8;
            t.family_flags = 0x80;
        }
        if id == QUIET_CURSE {
            t.attributes[1] = 0x400;
        }
        if id == DEAR_BOLT {
            t.family_name = 3;
            t.mana_cost = 1000;
        }
        s.spells.insert(id, t);
    }
    for (id, aura) in [
        (HIDE, 16),
        (GOAD, 11),
        (MOCK, 11),
        (DOMINATE, 6),
        (BOON, 29),
        (LASTING, 27),
        (MOCK_BUFF, 29),
    ] {
        let mut t = crate::game::data::spells::SpellTemplate {
            id,
            name: format!("Spell {id}"),
            range_index: 1,
            equipped_item_class: -1,
            duration_index: 9,
            ..Default::default()
        };
        t.effects[0] = crate::game::data::spells::SpellEffect {
            effect: 6,
            aura,
            base_points: 100,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            ..Default::default()
        };
        s.spells.insert(id, t);
    }
    if let Some(t) = s.spells.get_mut(&LASTING) {
        t.custom_flags = crate::game::data::spells::CUSTOM_NOT_REMOVED_ON_EVADE;
    }
    // The buffs are cast at a friend.
    for id in [BOON, MOCK_BUFF] {
        if let Some(t) = s.spells.get_mut(&id) {
            t.effects[0].target_a = 21;
        }
    }
    d
}

/// The EventAI the order tests read: each event raises its creature's
/// phase by one, or yells a line (`EVENT_*`, script commands 44 and 0).
fn eventai(d: &mut GameData) {
    use crate::game::data::scripts::*;
    for (entry, name) in [
        (LEADER, "Leader"),
        (MEMBER, "Member"),
        (SUMMONER, "Summoner"),
        (SUMMONED, "Summoned"),
    ] {
        d.creature_templates
            .insert(entry, creature(entry, name, PACK));
    }
    let s = &mut d.scripts;
    s.groups.insert(
        30,
        vec![
            GroupMember {
                guid: 30,
                flags: GROUP_AGGRO_TOGETHER,
                ..Default::default()
            },
            GroupMember {
                guid: 31,
                flags: GROUP_AGGRO_TOGETHER,
                ..Default::default()
            },
        ],
    );
    s.group_of.insert(30, 30);
    s.group_of.insert(31, 30);
    // 1 raises the phase; 2 to 5 yell "leader", "member", "summoned",
    // "summoner".
    s.ai_scripts.insert(
        1,
        vec![ScriptStep {
            id: 1,
            command: 44,
            datalong: [1, 1, 0, 0],
            ..Default::default()
        }],
    );
    for (script, text) in [(2, 200), (3, 201), (4, 202), (5, 203)] {
        s.ai_scripts.insert(
            script,
            vec![ScriptStep {
                id: script,
                command: 0,
                dataint: [text, 0, 0, 0],
                ..Default::default()
            }],
        );
    }
    for (text, line) in [
        (200, "leader"),
        (201, "member"),
        (202, "summoned"),
        (203, "summoner"),
    ] {
        d.npc.broadcast_texts.insert(
            text as u32,
            crate::game::data::npc::BroadcastText {
                male: line.into(),
                chat_type: 1,
                ..Default::default()
            },
        );
    }
    let s = &mut d.scripts;
    let event = |id, creature, kind, params: [i32; 4], action| AiEvent {
        id,
        creature,
        kind,
        chance: 100,
        flags: EF_REPEATABLE,
        params,
        actions: [action, 0, 0],
        ..Default::default()
    };
    s.ai_events
        .insert(LEADER, vec![event(1, LEADER, EVENT_AGGRO, [0; 4], 2)]);
    s.ai_events.insert(
        MEMBER,
        vec![
            event(2, MEMBER, EVENT_AGGRO, [0; 4], 3),
            event(3, MEMBER, EVENT_GROUP_MEMBER_DIED, [0, 1, 0, 0], 1),
            event(4, MEMBER, EVENT_MOVEMENT_INFORM, [2, 7, 0, 0], 1),
        ],
    );
    s.ai_events.insert(
        SUMMONER,
        vec![
            event(
                5,
                SUMMONER,
                EVENT_SUMMONED_UNIT,
                [SUMMONED as i32, 0, 0, 0],
                5,
            ),
            event(
                6,
                SUMMONER,
                EVENT_SUMMONED_JUST_DESPAWN,
                [SUMMONED as i32, 0, 0, 0],
                1,
            ),
        ],
    );
    s.ai_events
        .insert(SUMMONED, vec![event(7, SUMMONED, EVENT_SPAWNED, [0; 4], 4)]);
}

async fn setup(class: Class) -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, class).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    a.sees(kobold(10)).await;
    // The heartbeat is in before a test moves the player itself.
    while map
        .query(move |m| dist(m.players[&ga].position(), at(0.0, 0.0)) > 0.01)
        .await
        .unwrap()
    {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    (realm, a, ga, map)
}

fn kobold(spawn: u32) -> Guid {
    guid::unit(KOBOLD, spawn)
}

/// The melee range between two units as the client measures it.
fn reach(m: &Map, a: Guid, b: Guid) -> f32 {
    combat::melee_range(&m.unit_view(a).unwrap(), &m.unit_view(b).unwrap())
}

/// The map's creatures take `steps` turns of 50 ms.
fn step(m: &mut Map, steps: u32) {
    for _ in 0..steps {
        m.now += Duration::from_millis(50);
        crate::game::map::ai::update_creatures(m);
        combat::update_creatures(m);
    }
}

fn movement(x: f32, orientation: f32, forward: bool) -> MovementInfo {
    MovementInfo {
        flags: if forward {
            MovementInfo_MovementFlags::new_forward()
        } else {
            MovementInfo_MovementFlags::empty()
        },
        timestamp: 0,
        position: Vector3d {
            x,
            y: START.1,
            z: START.2,
        },
        orientation,
        fall_time: 0.0,
    }
}

/// What a client saw of a creature: its moves (where from, where to, how
/// long), its swings at the client's player, as they came.
#[derive(Debug, Clone, Copy)]
enum Seen {
    Move {
        at: Instant,
        from: Vector3d,
        to: Vector3d,
        duration: Duration,
        stop: bool,
    },
    Swing(Instant),
}

/// A monster move's points and duration; none for a stop.
fn spline_of(m: &SMSG_MONSTER_MOVE_MonsterMoveType) -> (&[Vector3d], u32) {
    use SMSG_MONSTER_MOVE_MonsterMoveType as T;
    match m {
        T::Normal {
            splines, duration, ..
        }
        | T::FacingSpot {
            splines, duration, ..
        }
        | T::FacingTarget {
            splines, duration, ..
        }
        | T::FacingAngle {
            splines, duration, ..
        } => (splines, *duration),
        T::Stop => (&[], 0),
    }
}

async fn watch(c: &mut Client, creature: Guid, me: Guid, within: Duration) -> Vec<Seen> {
    let deadline = Instant::now() + within;
    c.raw.clear();
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        let got = c
            .gets(Duration::from_millis(3), |m| match m {
                ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv) if mv.guid == creature => {
                    let stop = matches!(mv.move_type, SMSG_MONSTER_MOVE_MonsterMoveType::Stop);
                    Some(Seen::Move {
                        at: Instant::now(),
                        from: mv.spline_point,
                        to: spline_of(&mv.move_type)
                            .0
                            .first()
                            .copied()
                            .unwrap_or(mv.spline_point),
                        duration: Duration::from_millis(u64::from(spline_of(&mv.move_type).1)),
                        stop,
                    })
                }
                ServerOpcodeMessage::SMSG_ATTACKERSTATEUPDATE(s)
                    if s.attacker == creature && s.target == me =>
                {
                    Some(Seen::Swing(Instant::now()))
                }
                _ => None,
            })
            .await;
        seen.extend(got);
        // What wow_messages can't read comes raw: a crit's swing (its hit
        // info), and a stop (1.12's ends at its type).
        for f in std::mem::take(&mut c.raw) {
            let opcode = u16::from_le_bytes([f[2], f[3]]);
            if opcode == combat::SMSG_ATTACKERSTATEUPDATE
                && packed_guids(&f[8..]) == Some((creature, me))
            {
                seen.push(Seen::Swing(Instant::now()));
            }
            if opcode == crate::game::map::ai::SMSG_MONSTER_MOVE
                && let Some((g, at)) = stop_of(&f[4..])
                && g == creature
            {
                seen.push(Seen::Move {
                    at: Instant::now(),
                    from: at,
                    to: at,
                    duration: Duration::ZERO,
                    stop: true,
                });
            }
        }
    }
    seen
}

/// A packed guid at `b[*i..]`, moving `i` past it.
fn packed_guid(b: &[u8], i: &mut usize) -> Option<Guid> {
    let mask = *b.get(*i)?;
    *i += 1;
    let mut v = 0u64;
    for bit in 0..8 {
        if mask & (1 << bit) != 0 {
            v |= u64::from(*b.get(*i)?) << (bit * 8);
            *i += 1;
        }
    }
    Some(Guid::new(v))
}

/// The two packed guids at the start of `b`.
fn packed_guids(b: &[u8]) -> Option<(Guid, Guid)> {
    let mut i = 0;
    Some((packed_guid(b, &mut i)?, packed_guid(b, &mut i)?))
}

/// A stop's monster move body: whose, and where.
fn stop_of(b: &[u8]) -> Option<(Guid, Vector3d)> {
    let mut i = 0;
    let g = packed_guid(b, &mut i)?;
    let f = |k: usize| {
        Some(f32::from_le_bytes(
            b.get(i + k..i + k + 4)?.try_into().ok()?,
        ))
    };
    let at = Vector3d {
        x: f(0)?,
        y: f(4)?,
        z: f(8)?,
    };
    (*b.get(i + 16)? == 1).then_some((g, at))
}

/// Where the client shows the creature at `t`: along its last move, as it
/// plays it from when it came.
fn shown_at(seen: &[Seen], t: Instant) -> Option<Vector3d> {
    let (at, from, to, duration) = seen.iter().rev().find_map(|s| match *s {
        Seen::Move {
            at,
            from,
            to,
            duration,
            ..
        } if at <= t => Some((at, from, to, duration)),
        _ => None,
    })?;
    let f = if duration.is_zero() {
        1.0
    } else {
        (t.duration_since(at).as_secs_f32() / duration.as_secs_f32()).min(1.0)
    };
    Some(Vector3d {
        x: from.x + (to.x - from.x) * f,
        y: from.y + (to.y - from.y) * f,
        z: from.z + (to.z - from.z) * f,
    })
}

/// The kobold's reach (the user's report): a rogue who strikes a kobold,
/// runs 25 yards and stops is chased, and the kobold's run ends where it
/// aimed, half a yard inside its reach, as the client plays it: it doesn't
/// stop on the reach's edge, where the client, showing a creature on the
/// move a little behind, would have it out of reach. Each swing after
/// comes from within reach as the client shows the kobold.
#[tokio::test]
async fn a_chased_rogue_is_struck_only_from_within_reach() {
    let (_realm, mut a, ga, map) = setup(Class::Rogue).await;
    let kobold = kobold(10);
    a.send(CMSG_ATTACKSWING { guid: kobold }).await;
    a.wait_for("the kobold's first swing", |m| match m {
        ServerOpcodeMessage::SMSG_ATTACKERSTATEUPDATE(s) if s.attacker == kobold => Some(()),
        _ => None,
    })
    .await;
    a.send(CMSG_ATTACKSTOP {}).await;
    // Run 25 yards west at 7 yards a second, and stop.
    let west = std::f32::consts::PI;
    a.send(MSG_MOVE_START_FORWARD_Client {
        info: movement(START.0, west, true),
    })
    .await;
    let mut x = START.0;
    for _ in 0..36 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        x -= 0.7;
        a.send(MSG_MOVE_HEARTBEAT_Client {
            info: movement(x, west, true),
        })
        .await;
    }
    a.clear();
    a.send(MSG_MOVE_STOP_Client {
        info: movement(x, west, false),
    })
    .await;
    let stopped = Instant::now();
    let seen = watch(&mut a, kobold, ga, Duration::from_secs(5)).await;
    let me = Vector3d {
        x,
        y: START.1,
        z: START.2,
    };
    let swings: Vec<Instant> = seen
        .iter()
        .filter_map(|s| match s {
            Seen::Swing(t) if *t > stopped + Duration::from_millis(150) => Some(*t),
            _ => None,
        })
        .collect();
    assert!(swings.len() >= 2, "it fights on: {seen:?}");
    let first = swings[0];
    let stops = seen
        .iter()
        .filter(|s| matches!(s, Seen::Move { at, stop: true, .. } if *at < first))
        .count();
    assert_eq!(
        stops, 0,
        "its run ends where it aimed, not stopped short: {seen:?}"
    );
    for t in &swings {
        // A little after, as the client shows the swing.
        let shown = shown_at(&seen, *t + Duration::from_millis(20)).expect("seen moving");
        assert!(
            dist(shown, me) <= 5.0 + 0.01,
            "a swing from {} yards: {seen:?}",
            dist(shown, me)
        );
    }
    let (resting, range) = map
        .query(move |m| {
            let c = m.creature(kobold).unwrap();
            (
                dist(c.position, m.players[&ga].position()),
                reach(m, kobold, ga),
            )
        })
        .await
        .unwrap();
    assert!(resting <= range, "it rests in reach: {resting} of {range}");
}

/// The chase's distances (Thistle Tea's `chase_target_distance` and
/// `chase_rechase_distance`): it aims half a yard inside its reach, and
/// paths again only once its victim is three quarters of the reach (less
/// its bounding radius) from where it aimed, or it has stopped.
#[tokio::test]
async fn a_chaser_aims_inside_its_reach_and_paths_again_only_when_its_victim_has_gone() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let kobold = kobold(10);
    let out = map
        .query(move |m| {
            let spline = |m: &Map| match &m.creature(kobold).unwrap().motion {
                Motion::Moving(w) => Some((w.spline_id, w.destination())),
                Motion::Idle { .. } => None,
            };
            let set = |m: &mut Map, dx: f32, dy: f32| {
                m.players.get_mut(&ga).unwrap().movement.position = at(dx, dy);
            };
            set(m, -16.0, 0.0);
            crate::game::map::ai::attacked(m, kobold, ga);
            step(m, 1);
            let range = reach(m, kobold, ga);
            let (first, goal) = spline(m).expect("it chases");
            let aimed = dist(goal, at(-16.0, 0.0));
            // Two yards: within three quarters of its reach, less the
            // victim's bounding radius.
            set(m, -16.0, 2.0);
            step(m, 1);
            let kept = spline(m).map(|s| s.0) == Some(first);
            set(m, -16.0, 4.0);
            step(m, 1);
            let again = spline(m).is_some_and(|s| s.0 != first);
            (range, aimed, kept, again)
        })
        .await
        .unwrap();
    let (range, aimed, kept, again) = out;
    assert!(
        (aimed - combat::chase_distance(range)).abs() < 0.05,
        "aims {aimed} of {range}"
    );
    assert!(kept, "two yards on, the same path");
    assert!(again, "four yards on, a new one");
}

#[test]
fn the_chase_distances_are_thistle_teas() {
    assert_eq!(combat::chase_distance(5.0), 4.5);
    assert_eq!(combat::chase_distance(0.3), 0.0);
    assert!((combat::rechase_distance(5.0, 0.388) - 3.362).abs() < 1e-4);
    assert_eq!(combat::rechase_distance(0.4, 1.0), 0.0);
}

/// What came to a client of a creature's moves and attack starts, in order.
async fn moves_and_starts(c: &mut Client, creature: Guid) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(s) = c
        .gets(Duration::from_millis(200), |m| match m {
            ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv) if mv.guid == creature => {
                let goes = spline_of(&mv.move_type)
                    .0
                    .first()
                    .is_some_and(|to| dist(*to, mv.spline_point) > 0.1);
                Some(match (&mv.move_type, goes) {
                    (SMSG_MONSTER_MOVE_MonsterMoveType::Stop, _) => "stop".to_owned(),
                    (SMSG_MONSTER_MOVE_MonsterMoveType::FacingTarget { target, .. }, false) => {
                        format!("face {}", target.guid())
                    }
                    (SMSG_MONSTER_MOVE_MonsterMoveType::FacingTarget { target, .. }, true) => {
                        format!("move facing {}", target.guid())
                    }
                    _ => "move".to_owned(),
                })
            }
            ServerOpcodeMessage::SMSG_ATTACKSTART(s) if s.attacker == creature => {
                Some(format!("start {}", s.victim.guid()))
            }
            _ => None,
        })
        .await
    {
        out.push(s);
    }
    out
}

/// `SMSG_ATTACKSTART` goes out at the creature's first swing in reach, not
/// as the fight begins, and again for a new victim once it reaches that
/// one (Thistle Tea's `attack_started`); its death ends it.
#[tokio::test]
async fn attack_start_comes_with_the_first_swing_in_reach_and_for_each_new_victim() {
    let (realm, mut a, ga, map) = setup(Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0 - 12.0, START.1, START.2).await;
    let kobold = kobold(10);
    a.clear();
    let out = map
        .query(move |m| {
            m.players.get_mut(&ga).unwrap().movement.position = at(-6.0, 0.0);
            m.players.get_mut(&gb).unwrap().movement.position = at(-12.0, 6.0);
            crate::game::map::ai::attacked(m, kobold, ga);
            step(m, 40);
            let reached_a =
                combat::in_melee_range(&m.unit_view(kobold).unwrap(), &m.unit_view(ga).unwrap());
            // Bob takes its attention.
            crate::game::map::ai::add_threat(m, kobold, gb, 1000.0);
            step(m, 60);
            let c = m.creature(kobold).unwrap();
            (reached_a, c.combat.victim == gb)
        })
        .await
        .unwrap();
    let (reached_a, on_bob) = out;
    assert!(reached_a && on_bob, "{out:?}");
    let seen = moves_and_starts(&mut a, kobold).await;
    let start_a = format!("start {}", ga.guid());
    let start_b = format!("start {}", gb.guid());
    let first_move = seen
        .iter()
        .position(|s| s.starts_with("move"))
        .expect("it chases");
    let at_a = seen
        .iter()
        .position(|s| *s == start_a)
        .expect("an attack start");
    let at_b = seen
        .iter()
        .position(|s| *s == start_b)
        .expect("an attack start on Bob");
    assert!(first_move < at_a, "after it closed in: {seen:?}");
    assert!(
        seen[at_a + 1..at_b].iter().any(|s| s.starts_with("move")),
        "Bob's after it went to him: {seen:?}"
    );
    // Its death ends its attack.
    map.query(move |m| combat::kill(m, ga, kobold)).await;
    a.wait_for("its SMSG_ATTACKSTOP as it dies", |m| match m {
        ServerOpcodeMessage::SMSG_ATTACKSTOP(s) if s.player == kobold => Some(()),
        _ => None,
    })
    .await;
}

/// A creature fighting faces its victim, and tells its observers (Thistle
/// Tea's `face_target` and `face_melee_victim`): its chase faces its
/// victim, and standing, it turns to its victim as its swing comes due.
#[tokio::test]
async fn a_fighting_creature_turns_to_face_its_victim() {
    let (_realm, mut a, ga, map) = setup(Class::Warrior).await;
    let kobold = kobold(10);
    let set = |dx: f32, dy: f32| {
        move |m: &mut Map| {
            m.players.get_mut(&ga).unwrap().movement.position = at(dx, dy);
        }
    };
    map.query(set(-6.0, 0.0)).await;
    a.clear();
    let settled = map
        .query(move |m| {
            crate::game::map::ai::attacked(m, kobold, ga);
            step(m, 60);
            matches!(m.creature(kobold).unwrap().motion, Motion::Idle { .. })
        })
        .await
        .unwrap();
    assert!(settled, "it stands to fight");
    let chased = moves_and_starts(&mut a, kobold).await;
    let chase = format!("move facing {}", ga.guid());
    assert!(
        chased.contains(&chase) && chased.iter().all(|s| s != "move"),
        "its chase faces its victim: {chased:?}"
    );
    // Its victim steps round it, still in reach: as its swing comes due,
    // it turns.
    let turned = map
        .query(move |m| {
            let k = m.creature(kobold).unwrap().position;
            m.players.get_mut(&ga).unwrap().movement.position = Vector3d { y: k.y + 3.0, ..k };
            step(m, 50);
            let c = m.creature(kobold).unwrap();
            (c.orientation - std::f32::consts::FRAC_PI_2).abs() < 0.05
        })
        .await
        .unwrap();
    assert!(turned, "it faces north, at its victim");
    let seen = moves_and_starts(&mut a, kobold).await;
    assert!(
        seen.contains(&format!("face {}", ga.guid())),
        "told: {seen:?}"
    );
}

fn posture(m: &Map, g: Guid) -> (u8, u8, u32) {
    let f = m.unit_fields(g).unwrap();
    (
        f.get_byte(UNIT_FIELD_BYTES_1, 0),
        f.get_byte(UNIT_FIELD_BYTES_2, 0),
        f.get_u32(UNIT_NPC_EMOTESTATE),
    )
}

/// A creature drawn into a fight stands up and stops its idle emote, and
/// home again sits back down to it (Thistle Tea's `Emote.reset` and
/// `Posture.restore`).
#[tokio::test]
async fn a_fight_stands_a_creature_up_and_home_it_rests_again() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let sitting = kobold(11);
    let out = map
        .query(move |m| {
            let resting = posture(m, sitting);
            m.players.get_mut(&ga).unwrap().movement.position = at(24.0, 0.0);
            crate::game::map::ai::attacked(m, sitting, ga);
            let fighting = posture(m, sitting);
            step(m, 20);
            crate::game::map::ai::enter_evade(m, sitting);
            step(m, 100);
            let c = m.creature(sitting).unwrap();
            (resting, fighting, c.evading, posture(m, sitting))
        })
        .await
        .unwrap();
    let (resting, fighting, evading, home) = out;
    assert_eq!(resting, (1, 1, 10), "it sits, eating");
    assert_eq!((fighting.0, fighting.2), (0, 0), "it stands to fight");
    assert!(!evading, "home");
    assert_eq!(home, resting, "and sits again");
}

/// A creature is whole again as it leashes, not once it is home, and only
/// one that regenerates (Thistle Tea's `heal_to_full`).
#[tokio::test]
async fn a_leashing_creature_is_healed_as_it_goes_if_it_regenerates() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (fresh, worn) = (kobold(11), guid::unit(WORN, 12));
    let out = map
        .query(move |m| {
            m.players.get_mut(&ga).unwrap().movement.position = at(24.0, 3.0);
            let health = |m: &Map, g: Guid| {
                let f = m.unit_fields(g).unwrap();
                (
                    f.get_u32(UNIT_FIELD_HEALTH),
                    f.get_u32(UNIT_FIELD_MAXHEALTH),
                )
            };
            let mut out = Vec::new();
            for g in [fresh, worn] {
                crate::game::map::ai::attacked(m, g, ga);
                step(m, 20);
                let max = health(m, g).1;
                m.unit_fields_mut(g)
                    .unwrap()
                    .set_u32(UNIT_FIELD_HEALTH, max / 2);
                crate::game::map::ai::enter_evade(m, g);
                let going = (m.creature(g).unwrap().evading, health(m, g).0);
                step(m, 100);
                let home = (m.creature(g).unwrap().evading, health(m, g).0);
                out.push((max, going, home));
            }
            out
        })
        .await
        .unwrap();
    let (max, going, home) = out[0];
    assert_eq!(going, (true, max), "whole as it goes");
    assert_eq!(home, (false, max));
    let (max, going, home) = out[1];
    assert_eq!(going, (true, max / 2), "a worn one stays worn");
    assert_eq!(home, (false, max / 2), "home too");
}

/// The leash as Thistle Tea extends it (`CombatLeash`): a blow on the
/// creature keeps it, landed or not, and a channel's tick, but not a burn's;
/// under control it is kept, so it doesn't go the moment it is free; those
/// its kind called share its clock; and its reach is half again its aggro
/// radius where that is more than `[creatures] leash_radius` (mangos's 30;
/// Thistle Tea's 50).
#[tokio::test]
async fn the_leash_holds_as_thistle_tea_holds_it() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (k, keen) = (kobold(11), guid::unit(KEEN, 14));
    let (caller, called) = (guid::unit(CALLER, 15), guid::unit(CALLER, 16));
    let out = map
        .query(move |m| {
            // Far from where its fight began, its victim further, nothing
            // done to it for thirteen seconds.
            let far = |m: &mut Map, g: Guid, yards: f32, theirs: f32| {
                let ago = m.now - Duration::from_secs(13);
                let c = m.creature_mut(g).unwrap();
                let origin = c.engagement.origin.unwrap();
                c.position = Vector3d {
                    x: origin.x + yards,
                    ..origin
                };
                c.engagement.last_hostile = Some(ago);
                m.players.get_mut(&ga).unwrap().movement.position = Vector3d {
                    x: origin.x + theirs,
                    ..origin
                };
            };
            let evades =
                |m: &Map, g: Guid| crate::game::map::creature_combat::should_evade(m, g, ga);
            let tick = |m: &mut Map, spell: u32| {
                let data = m.realm.data.clone();
                crate::game::map::auras::apply_spell(
                    m,
                    ga,
                    k,
                    data.spells.get(spell).unwrap(),
                    &[(0, 1)],
                    Default::default(),
                )
                .unwrap();
                let h = crate::game::map::auras::auras(m, k)
                    .unwrap()
                    .holders
                    .iter()
                    .find(|h| h.spell == spell)
                    .unwrap()
                    .clone();
                crate::game::map::spells::periodic_damage(m, &h, k, 1, false);
            };
            m.players.get_mut(&ga).unwrap().movement.position = at(26.0, 0.0);
            crate::game::map::ai::attacked(m, k, ga);
            far(m, k, 56.0, 55.0);
            let late = evades(m, k);
            tick(m, BURN);
            let burnt = evades(m, k);
            tick(m, DRAIN);
            let drained = evades(m, k);
            far(m, k, 56.0, 55.0);
            // A blow that misses as much as one that lands.
            combat::engage(m, ga, k);
            let struck = evades(m, k);
            // Stunned for three and a half seconds, then free.
            far(m, k, 56.0, 55.0);
            let stun = crate::game::map::player::UNIT_FLAG_STUNNED;
            m.unit_fields_mut(k)
                .unwrap()
                .set_flag(UNIT_FIELD_FLAGS, stun, true);
            step(m, 70);
            m.unit_fields_mut(k)
                .unwrap()
                .set_flag(UNIT_FIELD_FLAGS, stun, false);
            let held = evades(m, k);
            // The one its call brought shares its clock.
            m.players.get_mut(&ga).unwrap().movement.position = at(56.0, 0.0);
            crate::game::map::ai::attacked(m, caller, ga);
            step(m, 40);
            let came = m.creature(called).unwrap().combat.attacking;
            far(m, called, 56.0, 55.0);
            far(m, caller, 56.0, 55.0);
            combat::engage(m, ga, caller);
            let shared = evades(m, called);
            // Its reach of a level 1 player is 41 yards: 61 and a half.
            m.players.get_mut(&ga).unwrap().movement.position = at(26.0, -6.0);
            crate::game::map::ai::attacked(m, keen, ga);
            far(m, keen, 58.0, 59.0);
            let keen_held = evades(m, keen);
            (late, burnt, drained, struck, held, came, shared, keen_held)
        })
        .await
        .unwrap();
    let (late, burnt, drained, struck, held, came, shared, keen_held) = out;
    assert!(late, "far and late, it goes home");
    assert!(burnt, "a burn's tick doesn't keep it");
    assert!(!drained, "a channel's does");
    assert!(!struck, "a blow does");
    assert!(!held, "kept while stunned");
    assert!(came, "its kind came");
    assert!(!shared, "on its caller's leash");
    assert!(!keen_held, "half again its reach");
}

/// `spell` from `caster` on `target`, as its aura.
fn aura_on(m: &mut Map, caster: Guid, target: Guid, spell: u32, amount: i32) {
    let data = m.realm.data.clone();
    crate::game::map::auras::apply_spell(
        m,
        caster,
        target,
        data.spells.get(spell).unwrap(),
        &[(0, amount)],
        Default::default(),
    )
    .unwrap();
}

/// Whom a creature fights, as Thistle Tea's `Threat.reselect` chooses: one
/// far from its home as well as one near (the leash keeps it near home,
/// not the choice); not one it can't see, who comes off its list; and an
/// older taunt holds when the newest has gone.
#[tokio::test]
async fn a_creature_chooses_its_victim_as_thistle_tea_does() {
    let (realm, _a, ga, map) = setup(Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Cid", Race::Human, Class::Warrior).await;
    let k = kobold(11);
    let out = map
        .query(move |m| {
            let place = |m: &mut Map, g: Guid, dx: f32| {
                m.players.get_mut(&g).unwrap().movement.position = at(dx, 0.0);
            };
            let choose = |m: &mut Map, current: Guid| {
                m.creature_mut(k).unwrap().combat.victim = current;
                step(m, 1);
                m.creature(k).unwrap().combat.victim
            };
            place(m, ga, 28.0);
            crate::game::map::ai::attacked(m, k, ga);
            // Bob, far beyond its home, three times Alice's threat.
            place(m, gb, -40.0);
            m.creature_mut(k).unwrap().threat = vec![(ga, 10.0), (gb, 30.0)];
            let far = choose(m, ga);
            // Bob hidden from it: off its list, and Alice it is.
            aura_on(m, gb, gb, HIDE, 100);
            let hidden = choose(m, gb);
            let listed = m.creature(k).unwrap().threat.iter().any(|(t, _)| *t == gb);
            // Alice and Cid taunt it in turn, Bob holding the most threat:
            // with Cid's taunt gone, Alice's holds.
            place(m, gb, 28.0);
            place(m, gc, 28.0);
            crate::game::map::auras::remove_spell(m, gb, HIDE, None);
            m.creature_mut(k).unwrap().threat = vec![(ga, 10.0), (gb, 1000.0), (gc, 10.0)];
            aura_on(m, ga, k, GOAD, 1);
            aura_on(m, gc, k, MOCK, 1);
            crate::game::map::auras::remove_spell(m, k, MOCK, Some(gc));
            let taunted = choose(m, gb);
            (far, hidden, listed, taunted)
        })
        .await
        .unwrap();
    let (far, hidden, listed, taunted) = out;
    assert_eq!(far, gb, "far from home, still the one");
    assert_eq!(hidden, ga, "not one it can't see");
    assert!(!listed, "who comes off its list");
    assert_eq!(taunted, ga, "the older taunt holds");
}

/// A caster with a chase distance stands off within it only where it sees
/// its victim, and in sight closes only to that distance (Thistle Tea's
/// `CasterChase`): behind a wall it comes on.
#[tokio::test]
async fn a_caster_stands_off_only_where_it_sees_its_victim() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let k = kobold(11);
    let out = map
        .query(move |m| {
            let goal = |m: &Map| match &m.creature(k).unwrap().motion {
                Motion::Moving(w) => Some(w.destination()),
                Motion::Idle { .. } => None,
            };
            crate::game::map::event_ai::scripted_mut(m, k)
                .unwrap()
                .caster_chase_distance = Some(20.0);
            // Thirty yards off, in sight: it closes to twenty.
            m.players.get_mut(&ga).unwrap().movement.position = at(0.0, 0.0);
            crate::game::map::ai::attacked(m, k, ga);
            step(m, 1);
            let closing = goal(m).map(|p| dist(p, at(0.0, 0.0)));
            step(m, 60);
            let stood = (
                goal(m).is_none(),
                dist(m.creature(k).unwrap().position, at(0.0, 0.0)),
            );
            // A wall between: it comes on.
            let k_at = m.creature(k).unwrap().position;
            let mid = (k_at.x + START.0) / 2.0;
            m.spells.walls.push((
                Vector3d {
                    x: mid,
                    y: START.1 - 5.0,
                    z: START.2,
                },
                Vector3d {
                    x: mid,
                    y: START.1 + 5.0,
                    z: START.2,
                },
            ));
            step(m, 2);
            let blind = goal(m).map(|p| dist(p, at(0.0, 0.0)));
            m.spells.walls.clear();
            (closing, stood, blind)
        })
        .await
        .unwrap();
    let (closing, stood, blind) = out;
    assert!(
        closing.is_some_and(|d| (d - 20.0).abs() < 0.1),
        "it closes to twenty: {closing:?}"
    );
    assert!(
        stood.0 && (stood.1 - 20.0).abs() < 0.5,
        "and stands: {stood:?}"
    );
    assert!(
        blind.is_some_and(|d| d < 5.0),
        "unseen, it comes on: {blind:?}"
    );
}

/// Whether creature `g` is in a fight.
fn fighting(m: &Map, g: Guid) -> bool {
    m.creature(g).is_some_and(|c| {
        c.combat.attacking
            || c.fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            )
    })
}

/// A harmful spell puts a creature in the fight as Thistle Tea decides
/// (`Spell.Combat.decide`): not Sap, seen or not; not a spell from a
/// caster it doesn't see, nor one with no threat; one from a caster it
/// sees, yes; and damage, seen or not.
#[tokio::test]
async fn a_harmful_spell_starts_a_fight_as_thistle_tea_decides() {
    let (_realm, _a, ga, map) = setup(Class::Rogue).await;
    let out = map
        .query(move |m| {
            m.players
                .get_mut(&ga)
                .unwrap()
                .spells
                .extend([SAP, CURSE, QUIET_CURSE, BOLT]);
            let cast = |m: &mut Map, spawn: u32, dy: f32, spell: u32, hidden: bool| {
                let k = kobold(spawn);
                m.players.get_mut(&ga).unwrap().movement.position = at(45.0, dy);
                if hidden {
                    aura_on(m, ga, ga, HIDE, 100);
                } else {
                    crate::game::map::auras::remove_spell(m, ga, HIDE, None);
                }
                let sees = crate::game::map::spells::stealth::sees(m, k, ga);
                let went = crate::game::map::spells::cast(
                    m,
                    ga,
                    spell,
                    crate::game::map::spells::Targets {
                        unit: Some(k),
                        ..Default::default()
                    },
                    None,
                );
                (went, sees, fighting(m, k))
            };
            [
                cast(m, 20, 20.0, SAP, true),
                cast(m, 21, 30.0, SAP, false),
                cast(m, 22, 40.0, CURSE, true),
                cast(m, 23, 50.0, CURSE, false),
                cast(m, 24, 60.0, QUIET_CURSE, false),
                cast(m, 25, 70.0, BOLT, true),
            ]
        })
        .await
        .unwrap();
    let [sap_hidden, sap_seen, curse_hidden, curse_seen, quiet, bolt] = out;
    for (what, (went, _, _)) in [
        ("sap", sap_hidden),
        ("sap", sap_seen),
        ("curse", curse_hidden),
        ("curse", curse_seen),
        ("quiet", quiet),
        ("bolt", bolt),
    ] {
        assert!(went, "the {what} went");
    }
    assert!(
        !sap_hidden.1 && !curse_hidden.1 && !bolt.1,
        "hidden is hidden"
    );
    assert!(!sap_hidden.2, "Sap from stealth starts no fight");
    assert!(!sap_seen.2, "nor seen");
    assert!(!curse_hidden.2, "a curse from one unseen starts none");
    assert!(curse_seen.2, "seen, it does");
    assert!(!quiet.2, "one with no threat doesn't");
    assert!(bolt.2, "damage does, seen or not");
}

/// Creature `g` put at `p`, its grid cell with it.
fn put(m: &mut Map, g: Guid, p: Vector3d) {
    let c = m.creature_mut(g).unwrap();
    let old = c.cell;
    c.position = p;
    c.home = p;
    c.cell = crate::game::map::grid::cell_of(p.x, p.y);
    let new = c.cell;
    m.grid.relocate(g, old, new);
}

/// Creatures fighting one victim spread round it (Thistle Tea's
/// `maybe_spread`): one standing on another steps aside, one standing in
/// its victim steps back, each at a walk and still in reach.
#[tokio::test]
async fn creatures_spread_round_their_victim() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (k1, k2, k3) = (kobold(20), kobold(21), kobold(22));
    let out = map
        .query(move |m| {
            m.players.get_mut(&ga).unwrap().movement.position = at(40.0, 40.0);
            let me = at(40.0, 40.0);
            // Two on one spot, three yards off; one in the player.
            put(m, k1, at(43.0, 40.0));
            put(m, k2, at(43.0, 40.0));
            put(m, k3, at(40.3, 40.0));
            for k in [k1, k2, k3] {
                crate::game::map::ai::attacked(m, k, ga);
            }
            let mut walked = false;
            for _ in 0..200 {
                step(m, 1);
                walked |= [k1, k2, k3].iter().any(
                    |k| matches!(&m.creature(*k).unwrap().motion, Motion::Moving(w) if !w.run),
                );
            }
            let p = |g: Guid| m.creature(g).unwrap().position;
            let in_reach = [k1, k2, k3].iter().all(|k| {
                combat::in_melee_range(&m.unit_view(*k).unwrap(), &m.unit_view(ga).unwrap())
            });
            (walked, dist(p(k1), p(k2)), dist(p(k3), me), in_reach)
        })
        .await
        .unwrap();
    let (walked, apart, off, in_reach) = out;
    assert!(walked, "at a walk");
    // Apart past where Thistle Tea has them stacked, and the one in its
    // victim out of it.
    assert!(apart > 1.118, "the two step apart: {apart}");
    assert!(off > 1.25, "the one in its victim steps back: {off}");
    assert!(in_reach, "all still in reach");
}

/// A player a creature charms fights its charmer's foes, not whoever is
/// nearest, and is theirs again once its charmer's fight is over (Thistle
/// Tea's `BT.Charm` and `PlayerCharm.controller_active?`).
#[tokio::test]
async fn a_charmed_player_fights_its_charmers_foes_while_it_fights() {
    let (realm, _a, ga, map) = setup(Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Cid", Race::Human, Class::Warrior).await;
    let k = kobold(23);
    let out = map
        .query(move |m| {
            let turn = |m: &mut Map| {
                m.now += Duration::from_millis(50);
                crate::game::map::ai::update_creatures(m);
                combat::update_creatures(m);
                crate::game::map::pets::update(m);
                crate::game::map::player_motion::update(m);
            };
            for (g, dx) in [(ga, 40.0), (gb, 44.0), (gc, 39.0)] {
                m.players.get_mut(&g).unwrap().movement.position = at(dx, 50.0);
            }
            put(m, k, at(48.0, 50.0));
            crate::game::map::ai::attacked(m, k, gb);
            crate::game::map::spells::cast_triggered(m, k, DOMINATE, Some(ga));
            let charmed = m.players[&ga].owned.possessed_by.is_some();
            let mut struck = Vec::new();
            for _ in 0..40 {
                turn(m);
                let c = &m.players[&ga].combat;
                if c.attacking && !struck.contains(&c.victim) {
                    struck.push(c.victim);
                }
            }
            crate::game::map::ai::enter_evade(m, k);
            for _ in 0..4 {
                turn(m);
            }
            let freed = m.players[&ga].owned.possessed_by.is_none()
                && !crate::game::map::auras::auras(m, ga)
                    .unwrap()
                    .has_spell(DOMINATE);
            (charmed, struck, freed)
        })
        .await
        .unwrap();
    let (charmed, struck, freed) = out;
    assert!(charmed, "charmed");
    assert_eq!(struck, vec![gb], "its charmer's foe, not the nearest");
    assert!(freed, "theirs again once its charmer's fight is over");
}

/// A charmed caster casts what it can, and with nothing it can cast (no
/// mana) fights by blade till its next try (Thistle Tea's `try_spell` and
/// `melee_fallback?`).
#[tokio::test]
async fn a_charmed_caster_with_nothing_to_cast_fights_by_blade() {
    let (realm, _a, ga, map) = setup(Class::Mage).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let k = kobold(23);
    let struck = map
        .query(move |m| {
            let p = m.players.get_mut(&ga).unwrap();
            p.spells.push(DEAR_BOLT);
            p.fields.set_u32(UNIT_FIELD_POWER1, 0);
            m.players.get_mut(&ga).unwrap().movement.position = at(40.0, 50.0);
            m.players.get_mut(&gb).unwrap().movement.position = at(43.0, 50.0);
            put(m, k, at(48.0, 50.0));
            crate::game::map::ai::attacked(m, k, gb);
            crate::game::map::spells::cast_triggered(m, k, DOMINATE, Some(ga));
            for _ in 0..20 {
                m.now += Duration::from_millis(50);
                crate::game::map::ai::update_creatures(m);
                combat::update_creatures(m);
                crate::game::map::pets::update(m);
                crate::game::map::player_motion::update(m);
            }
            let c = &m.players[&ga].combat;
            c.attacking.then_some(c.victim)
        })
        .await
        .unwrap();
    assert_eq!(struck, Some(gb), "by blade");
}

/// A creature slowed on the move goes on at its new speed, sent again
/// from where it is (mangos's `propagateSpeedChange`, Thistle Tea's
/// `retime`); one a player's client moves hears of it as a player does,
/// and those who see it once that client has it (both of them).
#[tokio::test]
async fn a_slowed_creature_goes_on_slower_and_a_possessed_one_is_told_as_a_player() {
    let (realm, mut a, ga, map) = setup(Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0, START.1, START.2).await;
    let (k, held) = (kobold(11), kobold(13));
    b.sees(k).await;
    crate::game::harness::an_update(&map).await;
    a.clear();
    let out = map
        .query(move |m| {
            m.players.get_mut(&ga).unwrap().movement.position = at(10.0, 0.0);
            crate::game::map::ai::attacked(m, k, ga);
            step(m, 2);
            let speed = |m: &Map| match &m.creature(k).unwrap().motion {
                Motion::Moving(w) => Some((w.speed, w.spline_id)),
                Motion::Idle { .. } => None,
            };
            let before = speed(m);
            aura_on(m, gb, k, CURSE, -50);
            let after = speed(m);
            // Bob's client moves the other.
            m.players.get_mut(&gb).unwrap().owned.mover = Some(held);
            aura_on(m, ga, held, CURSE, -50);
            (before, after, m.creature(held).unwrap().speeds.run)
        })
        .await
        .unwrap();
    let (before, after, held_speed) = out;
    let (before, chase) = before.expect("it runs");
    let (after, again) = after.expect("it runs on");
    assert!(
        (after - before / 2.0).abs() < 0.01,
        "half as fast: {before} {after}"
    );
    assert_ne!(chase, again, "a new run");
    a.wait_for("its run again, from where it is", |m| match m {
        ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv) if mv.guid == k && mv.spline_id == again => {
            Some(())
        }
        _ => None,
    })
    .await;
    let forced = b
        .wait_for("the possessed one's speed, as a player's", |m| match m {
            ServerOpcodeMessage::SMSG_FORCE_RUN_SPEED_CHANGE(s) if s.guid == held => Some(s.speed),
            _ => None,
        })
        .await;
    assert!((forced - held_speed).abs() < 0.01);
    b.send(CMSG_FORCE_RUN_SPEED_CHANGE_ACK {
        guid: held,
        counter: 0,
        info: movement(START.0 + 33.0, 0.0, false),
        new_speed: forced,
    })
    .await;
    a.wait_frame(
        "those who see it hear of it",
        crate::game::map::misc::MSG_MOVE_SET_RUN_SPEED,
        |body| {
            let mut i = 0;
            packed_guid(body, &mut i) == Some(held)
        },
    )
    .await;
}

/// What an evade takes (Thistle Tea's `remove_on_evade`): its debuffs,
/// but not one marked to stay, nor a player's timed buff; its own buffs
/// too, unless its template keeps them.
#[tokio::test]
async fn an_evade_keeps_a_players_buffs_and_what_is_marked_to_stay() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (k, keeper) = (kobold(11), guid::unit(WORN, 12));
    let out = map
        .query(move |m| {
            m.players.get_mut(&ga).unwrap().movement.position = at(26.0, 3.0);
            let mut kept = Vec::new();
            for g in [k, keeper] {
                crate::game::map::ai::attacked(m, g, ga);
                aura_on(m, ga, g, BOON, 10);
                aura_on(m, ga, g, LASTING, -10);
                aura_on(m, ga, g, CURSE, -10);
                aura_on(m, g, g, MOCK_BUFF, 10);
                crate::game::map::ai::enter_evade(m, g);
                let a = crate::game::map::auras::auras(m, g).unwrap();
                kept.push([BOON, LASTING, CURSE, MOCK_BUFF].map(|s| a.has_spell(s)));
            }
            kept
        })
        .await
        .unwrap();
    assert_eq!(
        out[0],
        [true, true, false, false],
        "a player's buff, what stays; not its debuff, nor its own buff"
    );
    assert_eq!(out[1], [true, true, false, true], "its own buff, kept");
}

/// The lines creatures said to a client, in order.
async fn lines(c: &mut Client) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(line) = c
        .gets(Duration::from_millis(200), |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c)
                if matches!(c.chat_type, SMSG_MESSAGECHAT_ChatType::MonsterYell { .. }) =>
            {
                Some(c.message.clone())
            }
            _ => None,
        })
        .await
    {
        out.push(line);
    }
    out
}

/// EventAI's order, as Thistle Tea's: a group's member hears of its
/// leader's death only alive, and of reaching a point only alive; a
/// leader's own aggro events come before its group joins in; a summon's
/// spawned events before its summoner hears of it; and its summoner hears
/// it has gone however it goes, its corpse too.
#[tokio::test]
async fn eventai_hears_in_thistle_teas_order() {
    let (_realm, mut a, ga, map) = setup(Class::Warrior).await;
    let (leader, member, summoner) = (
        guid::unit(LEADER, 30),
        guid::unit(MEMBER, 31),
        guid::unit(SUMMONER, 32),
    );
    a.sees(leader).await;
    a.clear();
    let phase = |m: &Map, g: Guid| crate::game::map::event_ai::phase(m, g);
    m_aggro(&map, leader, ga).await;
    let fought = lines(&mut a).await;
    assert_eq!(
        fought,
        ["leader", "member"],
        "the leader's, then its group's"
    );
    let out = map
        .query(move |m| {
            // Dead, the member hears nothing.
            crate::game::map::ai::die(m, member, ga);
            crate::game::map::event_ai::movement_inform(m, member, 2, 7);
            crate::game::map::ai::die(m, leader, ga);
            let deaf = phase(m, member);
            // A summon: its own spawned events, then its summoner's.
            let at = m.creature(summoner).unwrap().position;
            let summoned =
                crate::game::map::scripts::summon(m, summoner, SUMMONED, at, 0.0, None).unwrap();
            crate::game::map::event_ai::update_despawns(m);
            // Its corpse gone, its summoner hears.
            crate::game::map::ai::die(m, summoned, ga);
            let now = m.now;
            if let Some(c) = m.creature_mut(summoned)
                && let crate::game::map::creature::Life::Dead { corpse_until, .. } = &mut c.life
            {
                *corpse_until = now;
            }
            crate::game::map::ai::update_dead(m);
            (deaf, phase(m, summoner))
        })
        .await
        .unwrap();
    let (deaf, heard) = out;
    assert_eq!(deaf, 0, "the dead member heard nothing");
    assert_eq!(heard, 1, "its corpse gone, the summoner heard");
    let summoned = lines(&mut a).await;
    assert_eq!(
        summoned,
        ["summoned", "summoner"],
        "its own, then its summoner's"
    );
}

/// `g` is attacked by `by`.
async fn m_aggro(map: &MapHandle, g: Guid, by: Guid) {
    map.query(move |m| crate::game::map::ai::attacked(m, g, by))
        .await
        .unwrap();
}
