//! Round 9's core movement, ported from Thistle Tea's
//! `test/game/core/movement_test.exs`, `entity/component/movement_block_test.exs`,
//! `effects/boundary_result_test.exs` and `world_test.exs`
//! (docs/parity/matrix.csv): a charge setting off where its charger
//! stands; a zeppelin passenger's movement read, and let go of once they
//! leave; a cast through a turn and a jump on the spot; a script's point
//! told only on arrival (not after a stop, a teleport or a new path; late
//! for a run slowed; on the next update for one already reached); a death's
//! stop for one running backwards and sideways; a script's teleport mid
//! walk, and one that only turns; and a dungeon copy closing as the boats
//! sail on.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncWriteExt, DuplexStream, duplex};
use tokio::sync::mpsc;
use wow_srp::normalized_string::NormalizedString;
use wow_srp::vanilla_header::{EncrypterHalf, ProofSeed};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::{ClientOpcodeMessage, ServerOpcodeMessage};
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::conn::{self, Inbound, Outbound};
use crate::game::data::GameData;
use crate::game::data::scripts::{AiEvent, EF_REPEATABLE, EVENT_MOVEMENT_INFORM, ScriptStep};
use crate::game::guid::{self, HighGuid};
use crate::game::harness::{self, Client};
use crate::game::map::auras::{self, AURA_DUMMY, AURA_MOD_DECREASE_SPEED, INTERRUPT_MOVE};
use crate::game::map::combat::dist;
use crate::game::map::creature::Motion;
use crate::game::map::event_ai::POINT_MOTION;
use crate::game::map::{Map, MapHandle, MapKey, ai, charge, scripts};
use crate::game::tests_p3_combat::{PACK, START, data, setup_with, spawn, template};
use crate::game::tests_p3_entity::{NONE_WITHIN, aura_effect, bob, on, spell};
use crate::game::tests_p3_entity_objects::until;
use crate::game::tests_parity_motion as motion;

fn start() -> Vector3d {
    Vector3d {
        x: START.0,
        y: START.1,
        z: START.2,
    }
}

/// The map's creatures take `n` turns of 50 ms.
fn turns(m: &mut Map, n: u32) {
    for _ in 0..n {
        m.now += Duration::from_millis(50);
        ai::update_creatures(m);
        crate::game::map::combat::update_creatures(m);
    }
}

/// Thistle Tea's "applies a resolved charge path atomically": a charge
/// sets off at once from where its charger stands, on a run at four times
/// its run speed (24 yards a second at most), timed for the path's length,
/// and the charger isn't moved yet: the run's first point is where it
/// stands. Its client and those who see it are told the run, from there.
#[tokio::test]
async fn a_charge_sets_off_at_once_from_where_its_charger_stands_on_a_run() {
    let mut d = motion::wolf_world();
    d.creature_spawns.get_mut(&0).unwrap()[0] =
        motion::spawn(80, motion::WOLF, motion::at(20.0, 0.0));
    let (realm, mut a, ga, map) = motion::enter(d).await;
    let g = motion::wolf();
    let mut b = Client::connect(&realm, "B");
    b.enter("Bea", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0, START.1 + 2.0, START.2).await;
    b.sees(ga).await;
    a.sees(g).await;
    a.clear();
    b.clear();
    let (from, still, walk, started_now, arrives_in, run) = on(&map, move |m| {
        let now = m.now;
        let from = m.players[&ga].position();
        charge::start(m, ga, g, true);
        let p = &m.players[&ga];
        let c = p.charge.as_ref().expect("it charges");
        let w = c.walk.clone().expect("its run");
        (
            from,
            p.position(),
            w.clone(),
            w.started == now,
            c.arrives_at.duration_since(now),
            p.speeds.run,
        )
    })
    .await;
    assert!(dist(still, from) < 1e-4, "not moved yet: {still:?}");
    assert_eq!(
        walk.points[0], from,
        "the run sets off from where it stands"
    );
    assert!(walk.run, "running");
    assert!(started_now, "at once");
    let speed = (run * 4.0).min(24.0);
    assert!((walk.speed - speed).abs() < 1e-4, "{} {speed}", walk.speed);
    let length: f32 = walk.points.windows(2).map(|w| dist(w[0], w[1])).sum();
    let ms = length / speed * 1000.0;
    assert!(
        (walk.duration.as_secs_f32() * 1000.0 - ms).abs() < 1.0,
        "timed for its length: {:?} {ms}",
        walk.duration
    );
    assert_eq!(arrives_in, walk.duration, "it arrives as the run ends");
    let to = walk.destination();
    for c in [&mut a, &mut b] {
        let (at, flags, duration, end) = c
            .wait_for("the charge's run", |msg| match msg {
                ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv) if mv.guid == ga => {
                    match &mv.move_type {
                        SMSG_MONSTER_MOVE_MonsterMoveType::Normal {
                            duration,
                            spline_flags,
                            splines,
                        } => Some((mv.spline_point, *spline_flags, *duration, splines[0])),
                        _ => None,
                    }
                }
                _ => None,
            })
            .await;
        assert!(dist(at, from) < 1e-4, "told from where it stands: {at:?}");
        assert!(flags.is_run_mode(), "a run");
        assert_eq!(duration, walk.duration.as_millis() as u32);
        assert!(dist(end, to) < 0.01, "to its end: {end:?} {to:?}");
    }
}

struct Reader {
    to: DuplexStream,
    encrypter: EncrypterHalf,
    rx: mpsc::Receiver<Inbound>,
    _out: conn::Frames,
}

/// wvs's connection reader on one end of a pipe, the client's encryption
/// on the other (as `tests_p3_network_wire`'s).
fn reader() -> Reader {
    let key = [7u8; 40];
    let name = NormalizedString::new("WIRE").unwrap();
    let (server_seed, client_seed) = (ProofSeed::new(), ProofSeed::new());
    let (s, c) = (server_seed.seed(), client_seed.seed());
    let (proof, client) = client_seed.into_client_header_crypto(&name, key, s);
    let server = server_seed
        .into_server_header_crypto(&name, key, proof, c)
        .unwrap();
    let (_, decrypter) = server.split();
    let (encrypter, _) = client.split();
    let (to, from) = duplex(64 * 1024);
    let (tx, rx) = mpsc::channel(64);
    let (outbound, out) = Outbound::detached();
    tokio::spawn(conn::run_reader(from, decrypter, tx, outbound));
    Reader {
        to,
        encrypter,
        rx,
        _out: out,
    }
}

impl Reader {
    /// What the reader passes on of `body` sent as `opcode`.
    async fn msg(&mut self, opcode: u16, body: &[u8]) -> ClientOpcodeMessage {
        let header = self
            .encrypter
            .encrypt_client_header(body.len() as u16 + 4, u32::from(opcode));
        self.to.write_all(&header).await.unwrap();
        self.to.write_all(body).await.unwrap();
        let next = tokio::time::timeout(Duration::from_secs(10), self.rx.recv())
            .await
            .expect("the reader passes something on")
            .expect("the reader is running");
        match next {
            Inbound::Msg(m) => m,
            Inbound::Raw(o, _) => panic!("0x{o:X} came raw"),
        }
    }
}

/// Thistle Tea's "parses a captured zeppelin movement packet": a real
/// client's heartbeat aboard a zeppelin, running forward and strafing
/// right, reads as aboard that zeppelin, at its place on deck and facing,
/// with its fall time, where it stands in the world and when.
#[tokio::test]
async fn a_captured_zeppelin_passengers_heartbeat_reads_as_aboard_at_its_place_on_deck() {
    const CAPTURED: [u8; 52] = [
        9, 0, 0, 2, 24, 11, 112, 5, 1, 127, 171, 68, 221, 230, 144, 197, 90, 254, 91, 66, 32, 139,
        142, 62, 232, 171, 2, 0, 0, 0, 192, 31, 22, 148, 45, 193, 127, 162, 193, 192, 241, 227,
        134, 193, 94, 136, 140, 64, 24, 5, 0, 0,
    ];
    let mut r = reader();
    let info = match r.msg(0xEE, &CAPTURED).await {
        ClientOpcodeMessage::MSG_MOVE_HEARTBEAT(m) => m.info,
        other => panic!("{other}"),
    };
    let near = |a: f32, b: f32, by: f32| (a - b).abs() < by;
    let t = info.flags.get_on_transport().expect("aboard").transport;
    assert_eq!(t.guid, Guid::new(0x1FC0_0000_0002_ABE8));
    let deck = (t.position.x, t.position.y, t.position.z, t.orientation);
    assert!(
        near(deck.0, -10.848_654, 1e-5)
            && near(deck.1, -6.051_086, 1e-5)
            && near(deck.2, -16.861_3, 1e-5)
            && near(deck.3, 4.391_646, 1e-5),
        "{deck:?}"
    );
    assert_eq!(info.fall_time, 1304);
    let f = &info.flags;
    assert!(
        f.get_forward() && f.get_strafe_right(),
        "forward, strafing right"
    );
    assert!(
        !f.get_backward() && !f.get_strafe_left() && f.get_jumping().is_none(),
        "nothing else"
    );
    assert_eq!(info.timestamp, 91_228_952);
    let p = info.position;
    assert!(
        near(p.x, 1371.969, 1e-3)
            && near(p.y, -4636.858, 1e-3)
            && near(p.z, 54.998, 1e-3)
            && near(info.orientation, 0.278_405, 1e-5),
        "{p:?} {}",
        info.orientation
    );
}

/// A heartbeat aboard `transport` at `offset` on deck, standing at `at`.
async fn aboard(c: &Client, transport: Guid, offset: Vector3d, at: Vector3d) {
    c.send(MSG_MOVE_HEARTBEAT_Client {
        info: MovementInfo {
            flags: MovementInfo_MovementFlags::empty().set_on_transport(
                MovementInfo_MovementFlags_OnTransport {
                    transport: TransportInfo {
                        guid: transport,
                        position: offset,
                        orientation: 0.0,
                    },
                },
            ),
            timestamp: 0,
            position: at,
            orientation: 0.0,
            fall_time: 0,
        },
    })
    .await;
}

/// `g`'s place on deck, if it is aboard anything.
fn deck(m: &Map, g: Guid) -> Option<(Guid, Vector3d)> {
    m.players[&g]
        .movement
        .flags
        .get_on_transport()
        .map(|t| (t.transport.guid, t.transport.position))
}

/// The transport of the next heartbeat of `g`'s that `c` is told.
async fn told_aboard(c: &mut Client, g: Guid) -> Option<Guid> {
    c.wait_for("a heartbeat", |m| match m {
        ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(s) if s.guid == g => {
            Some(s.info.flags.get_on_transport().map(|t| t.transport.guid))
        }
        _ => None,
    })
    .await
}

/// Thistle Tea's "clears stale transport data after leaving a transport":
/// a passenger's movement aboard is taken with its transport and place on
/// deck; the next, aboard nothing, leaves them aboard nothing, no place on
/// deck kept, and those who see them are told each as it is.
#[tokio::test]
async fn a_movement_aboard_nothing_after_one_aboard_lets_the_transport_go() {
    let (realm, a, ga, map) = setup_with(data(), Class::Warrior).await;
    let (mut b, _) = bob(&realm, ga).await;
    b.clear();
    let boat = guid::make(HighGuid::MoTransport, 0, 1);
    let place = Vector3d {
        x: 1.0,
        y: 2.0,
        z: 3.0,
    };
    let there = Vector3d {
        x: START.0 + 2.0,
        ..start()
    };
    aboard(&a, boat, place, there).await;
    until(&map, "aboard", move |m| deck(m, ga) == Some((boat, place))).await;
    assert_eq!(told_aboard(&mut b, ga).await, Some(boat), "told aboard");
    a.heartbeat(there.x, there.y, there.z).await;
    until(&map, "aboard nothing", move |m| deck(m, ga).is_none()).await;
    assert_eq!(told_aboard(&mut b, ga).await, None, "told aboard nothing");
}

/// A two-second cast that moving breaks, and an aura that moving ends.
const STILL_CAST: u32 = 997_901;
const STILL_AURA: u32 = 997_902;

fn still_data(d: &mut GameData) {
    use crate::game::tests_p3_cast_kit::*;
    let mut cast = bare(STILL_CAST, 0);
    cast.casting_time_index = CAST_2S;
    cast.interrupt_flags = 0x01;
    effect(&mut cast, 0, damage(10));
    let mut still = bare(STILL_AURA, 0);
    still.duration_index = LASTS_LONG;
    still.aura_interrupt_flags = INTERRUPT_MOVE;
    effect(&mut still, 0, aura(AURA_DUMMY, 0, 1, 1));
    insert(d, [cast, still]);
}

/// Thistle Tea's "ignores turning and jumping in place": turning either
/// way, facing about and jumping where one stands isn't moving: a cast that
/// moving breaks goes off, and an aura that moving ends stays (mangos's
/// `Spell::update` cancels at a changed position only). A step ends it.
#[tokio::test]
async fn turning_and_jumping_on_the_spot_neither_break_a_cast_nor_end_what_moving_ends() {
    use crate::game::tests_p3_spells_kit as kit;
    let realm = crate::game::tests_p3_cast_kit::fast_realm(still_data);
    let (mut c, a, map) =
        crate::game::tests_p3_cast_kit::human(&realm, "P9MO4", "Stillness", Class::Mage).await;
    let (o, here) = on(&map, move |m| {
        kit::ready(m, a, 60, &[STILL_CAST]);
        let kobold = kit::dummy(m, a, 60, 5.0);
        kit::apply(m, a, a, STILL_AURA);
        assert!(
            kit::cast_fresh(m, a, STILL_CAST, kit::at(kobold)),
            "the cast begins"
        );
        let p = &m.players[&a];
        (p.movement.orientation, p.position())
    })
    .await;
    let info = |flags: MovementInfo_MovementFlags, o: f32, timestamp: u32| MovementInfo {
        flags,
        timestamp,
        position: here,
        orientation: o,
        fall_time: 0,
    };
    let none = MovementInfo_MovementFlags::empty;
    c.send(MSG_MOVE_START_TURN_LEFT_Client {
        info: info(none().set_turn_left(), o, 1),
    })
    .await;
    c.send(MSG_MOVE_STOP_TURN_Client {
        info: info(none(), o + 0.5, 2),
    })
    .await;
    c.send(MSG_MOVE_START_TURN_RIGHT_Client {
        info: info(none().set_turn_right(), o + 0.5, 3),
    })
    .await;
    c.send(MSG_MOVE_SET_FACING_Client {
        info: info(none(), o, 4),
    })
    .await;
    c.send(MSG_MOVE_JUMP_Client {
        info: info(none().set_jumping(Default::default()), o, 5),
    })
    .await;
    c.send(MSG_MOVE_FALL_LAND_Client {
        info: info(none(), o, 6),
    })
    .await;
    until(&map, "the landing taken", move |m| {
        m.players[&a].movement.timestamp == 6
    })
    .await;
    let (casting, held) = on(&map, move |m| {
        (kit::casting(m, a), kit::has(m, a, STILL_AURA))
    })
    .await;
    assert_eq!(casting, Some(STILL_CAST), "the cast goes on");
    assert!(held, "the aura stays");
    on(&map, move |m| kit::pass(m, 2_100)).await;
    let went = c
        .wait_for("the cast's end", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(g) if g.spell == STILL_CAST => Some(true),
            ServerOpcodeMessage::SMSG_SPELL_FAILURE(f) if f.spell == STILL_CAST => Some(false),
            _ => None,
        })
        .await;
    assert!(went, "it goes off");
    // A step: the aura goes.
    c.send(MSG_MOVE_HEARTBEAT_Client {
        info: MovementInfo {
            position: Vector3d {
                x: here.x + 1.0,
                ..here
            },
            ..info(none(), o, 7)
        },
    })
    .await;
    until(&map, "the step ends the aura", move |m| {
        !kit::has(m, a, STILL_AURA)
    })
    .await;
}

/// A creature whose EventAI yells "point N" when its point N is reached.
const WALKER: u32 = 3_795;
/// A slow by half, on oneself.
const SLOW: u32 = 97_890;
/// What a walker says when a test has it say so, before and after.
const BEFORE: i32 = 470;
const AFTER: i32 = 471;

fn yell(d: &mut GameData, text: i32, line: &str) {
    d.npc.broadcast_texts.insert(
        text as u32,
        crate::game::data::npc::BroadcastText {
            male: line.into(),
            chat_type: 1,
            ..Default::default()
        },
    );
}

fn says(text: i32) -> ScriptStep {
    ScriptStep {
        command: 0,
        dataint: [text, 0, 0, 0],
        ..Default::default()
    }
}

fn walker_world() -> GameData {
    let mut d = data();
    let mut t = template(WALKER, 7);
    t.name = "Walker".into();
    t.faction = PACK;
    d.creature_templates.insert(WALKER, t);
    let mut events = Vec::new();
    for point in 1..=7u32 {
        let script = 440 + point;
        let text = 440 + point as i32;
        d.scripts.ai_scripts.insert(
            script,
            vec![ScriptStep {
                id: script,
                ..says(text)
            }],
        );
        yell(&mut d, text, &format!("point {point}"));
        events.push(AiEvent {
            id: point,
            creature: WALKER,
            kind: EVENT_MOVEMENT_INFORM,
            chance: 100,
            flags: EF_REPEATABLE,
            params: [POINT_MOTION as i32, point as i32, 0, 0],
            actions: [script, 0, 0],
            ..Default::default()
        });
    }
    d.scripts.ai_events.insert(WALKER, events);
    yell(&mut d, BEFORE, "before");
    yell(&mut d, AFTER, "after");
    d.spells.spells.insert(
        SLOW,
        spell(SLOW, &[aura_effect(AURA_MOD_DECREASE_SPEED, -50, 0, 1)]),
    );
    d
}

/// The lines creatures yelled to a client, in order.
async fn lines(c: &mut Client) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(line) = c
        .gets(NONE_WITHIN, |m| match m {
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

/// A run to `to` whose arrival tells `point`.
fn run_to(m: &mut Map, g: Guid, to: Vector3d, point: Option<u32>) {
    scripts::move_with(
        m,
        g,
        to,
        scripts::Move {
            run: true,
            point,
            ..Default::default()
        },
    );
}

fn east(m: &Map, g: Guid, yards: f32) -> Vector3d {
    let from = m.creature(g).unwrap().position;
    Vector3d {
        x: from.x + yards,
        ..from
    }
}

/// Walkers `yards` in front of Alice, seen by her.
async fn walkers<const N: usize>(
    a: &mut Client,
    map: &MapHandle,
    ga: Guid,
    yards: [f32; N],
) -> [Guid; N] {
    let gs = on(map, move |m| yards.map(|y| spawn(m, ga, WALKER, 20, y))).await;
    for g in gs {
        a.sees(g).await;
    }
    a.clear();
    gs
}

/// Thistle Tea's "stops, teleports, and replacement paths discard
/// unfinished callbacks": a script's run stopped short, a creature put
/// elsewhere by a teleport (onto its run's very end) and one sent on
/// another path tell no point, ever; one left to arrive tells its own,
/// once. None has a script's move left waiting.
#[tokio::test]
async fn a_scripts_point_is_told_on_arrival_and_not_after_a_stop_a_teleport_or_a_new_path() {
    let (_realm, mut a, ga, map) = setup_with(walker_world(), Class::Mage).await;
    let gs = walkers(&mut a, &map, ga, [3.0, 4.0, 5.0, 6.0]).await;
    let waiting = on(&map, move |m| {
        let [stopped, teleported, replaced, _arrives] = gs;
        let mut ends = Vec::new();
        for (point, g) in (1..).zip(gs) {
            let to = east(m, g, 10.0);
            run_to(m, g, to, Some(point));
            ends.push(to);
        }
        // Half a second on, a third of the way.
        turns(m, 10);
        ai::rest(m, stopped, None);
        scripts::place(m, teleported, ends[1], 0.0);
        let from = m.creature(replaced).unwrap().position;
        run_to(
            m,
            replaced,
            Vector3d {
                y: from.y + 10.0,
                ..from
            },
            None,
        );
        turns(m, 200);
        gs.map(|g| {
            m.creature(g)
                .unwrap()
                .scripted
                .as_deref()
                .is_some_and(|s| s.scripted_move.is_some())
        })
    })
    .await;
    assert_eq!(waiting, [false; 4], "no move left waiting");
    assert_eq!(
        lines(&mut a).await,
        ["point 4"],
        "only the one that arrived, once"
    );
}

/// Thistle Tea's "retiming retains identity and moves the completion
/// deadline": a script's run of two seconds slowed by half at one second
/// runs on under a new spline that is still the script's move, and its
/// point is told as it gets there at three seconds, not at two.
#[tokio::test]
async fn a_scripts_run_slowed_tells_its_point_when_it_gets_there_later() {
    let (_realm, mut a, ga, map) = setup_with(walker_world(), Class::Mage).await;
    let [g] = walkers(&mut a, &map, ga, [4.0]).await;
    let (first, second, still_its, speeds) = on(&map, move |m| {
        let run = m.creature(g).unwrap().speeds.run;
        let to = east(m, g, 2.0 * run);
        run_to(m, g, to, Some(5));
        let spline = |m: &Map| match &m.creature(g).unwrap().motion {
            Motion::Moving(w) => Some((w.spline_id, w.speed)),
            Motion::Idle { .. } => None,
        };
        let first = spline(m).expect("it runs");
        turns(m, 20);
        let slow = m.realm.data.spells.get(SLOW).cloned().unwrap();
        auras::apply(m, g, g, &slow, 0, -50);
        let second = spline(m).expect("it runs on");
        let still_its = m
            .creature(g)
            .unwrap()
            .scripted
            .as_deref()
            .and_then(|s| s.scripted_move)
            .is_some_and(|s| s.spline_id == second.0);
        // 2.9 seconds in, and 3.2.
        turns(m, 38);
        scripts::execute(m, &says(BEFORE), g, g);
        turns(m, 6);
        scripts::execute(m, &says(AFTER), g, g);
        turns(m, 40);
        (first.0, second.0, still_its, (run, second.1))
    })
    .await;
    assert_ne!(first, second, "a new spline");
    assert!(still_its, "still the script's move");
    assert!(
        (speeds.1 - speeds.0 / 2.0).abs() < 0.01,
        "at half its speed: {speeds:?}"
    );
    assert_eq!(
        lines(&mut a).await,
        ["before", "point 5", "after"],
        "told between 2.9 and 3.2 seconds, once"
    );
}

/// Thistle Tea's "an already reached point completes on the next update":
/// a script's move to where its creature stands tells its point on the
/// creature's next turn, not as the move is made.
#[tokio::test]
async fn a_point_already_reached_is_told_on_the_next_turn() {
    let (_realm, mut a, ga, map) = setup_with(walker_world(), Class::Mage).await;
    let [g] = walkers(&mut a, &map, ga, [4.0]).await;
    on(&map, move |m| {
        let here = m.creature(g).unwrap().position;
        run_to(m, g, here, Some(6));
        scripts::execute(m, &says(BEFORE), g, g);
        turns(m, 1);
        scripts::execute(m, &says(AFTER), g, g);
        turns(m, 20);
    })
    .await;
    assert_eq!(lines(&mut a).await, ["before", "point 6", "after"]);
}

/// A movement of `flags` at `at`, facing east.
fn going(flags: MovementInfo_MovementFlags, at: Vector3d) -> MovementInfo {
    MovementInfo {
        flags,
        timestamp: 0,
        position: at,
        orientation: 0.0,
        fall_time: 0,
    }
}

/// Where `g` is told it stopped.
fn stop_of(g: Guid) -> impl FnMut(&ServerOpcodeMessage) -> Option<Vector3d> {
    move |m| match m {
        ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv)
            if mv.guid == g && matches!(mv.move_type, SMSG_MONSTER_MOVE_MonsterMoveType::Stop) =>
        {
            Some(mv.spline_point)
        }
        _ => None,
    }
}

/// Thistle Tea's "enqueues a stop for translating client movement": one
/// running backwards and strafing both ways who dies is stopped: their
/// run's flags gone, those who see them told a stop where they fell, and
/// what their client says after told on without the run.
#[tokio::test]
async fn one_dying_backwards_and_sideways_is_stopped_where_they_fell() {
    let (realm, a, ga, map) = setup_with(data(), Class::Warrior).await;
    let (mut b, _) = bob(&realm, ga).await;
    let flags = MovementInfo_MovementFlags::new_backward()
        .set_strafe_left()
        .set_strafe_right();
    a.send(MSG_MOVE_START_BACKWARD_Client {
        info: going(flags, start()),
    })
    .await;
    b.wait_for("her run", |m| match m {
        ServerOpcodeMessage::MSG_MOVE_START_BACKWARD(s) if s.guid == ga => Some(()),
        _ => None,
    })
    .await;
    b.clear();
    let left = on(&map, move |m| {
        crate::game::map::death::kill(m, ga);
        let f = m.players[&ga].movement.flags;
        (f.get_backward(), f.get_strafe_left(), f.get_strafe_right())
    })
    .await;
    assert_eq!(left, (false, false, false), "no run left");
    let at = b.wait_for("her stop", stop_of(ga)).await;
    assert!(dist(at, start()) < 0.1, "stopped where she fell: {at:?}");
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: going(flags, start()),
    })
    .await;
    let told = b
        .wait_for("her heartbeat", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(s) if s.guid == ga => Some(s.info.flags),
            _ => None,
        })
        .await;
    assert!(
        !told.get_backward() && !told.get_strafe_left() && !told.get_strafe_right(),
        "told on without the run: {told:?}"
    );
}

/// A script's teleport (command 6) to `at`, facing `o`.
fn teleport_to(at: Vector3d, o: f32) -> ScriptStep {
    ScriptStep {
        command: 6,
        x: at.x,
        y: at.y,
        z: at.z,
        o,
        ..Default::default()
    }
}

/// Where `g` is next told it is put and which way it then faces, by a
/// monster move that isn't a walk.
async fn told_put(c: &mut Client, g: Guid) -> (Vector3d, Option<f32>) {
    c.wait_for("where it is put", |m| match m {
        ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv) if mv.guid == g => match &mv.move_type {
            SMSG_MONSTER_MOVE_MonsterMoveType::Stop => Some((mv.spline_point, None)),
            SMSG_MONSTER_MOVE_MonsterMoveType::FacingAngle { angle, .. } => {
                Some((mv.spline_point, Some(*angle)))
            }
            _ => None,
        },
        _ => None,
    })
    .await
}

/// Thistle Tea's "interrupts an active spline at its interpolated pose and
/// installs the exact destination": a script's teleport of a creature
/// partway along a run ends the run: it is at the row's place and facing
/// exactly, standing, its script's move let go, in the grid there; those
/// who see it are told it is there; and it goes no further along the old
/// path, nor tells its point.
#[tokio::test]
async fn a_scripts_teleport_mid_run_ends_the_run_at_the_exact_destination() {
    let (_realm, mut a, ga, map) = setup_with(walker_world(), Class::Mage).await;
    let [g] = walkers(&mut a, &map, ga, [5.0]).await;
    let to = Vector3d {
        x: START.0 + 4.0,
        y: START.1 + 12.0,
        z: START.2 + 1.5,
    };
    let o = 5.42797;
    let (moved, put, later) = on(&map, move |m| {
        let from = m.creature(g).unwrap().position;
        let end = east(m, g, 20.0);
        run_to(m, g, end, Some(1));
        turns(m, 10);
        let moved = dist(m.creature(g).unwrap().position, from);
        scripts::execute(m, &teleport_to(to, o), g, Guid::zero());
        let c = m.creature(g).unwrap();
        let put = (
            matches!(c.motion, Motion::Idle { .. }),
            c.position,
            c.orientation,
            c.scripted
                .as_deref()
                .is_none_or(|s| s.scripted_move.is_none()),
            c.cell == crate::game::map::grid::cell_of(to.x, to.y),
            m.grid.around(to.x, to.y, 1.0).any(|x| x == g),
        );
        turns(m, 100);
        (moved, put, m.creature(g).unwrap().position)
    })
    .await;
    assert!(moved > 1.0, "partway along: {moved}");
    let (standing, at, facing, let_go, cell, in_grid) = put;
    assert!(standing, "standing");
    assert_eq!(
        (at, facing),
        (to, o),
        "exactly at the row's place and facing"
    );
    assert!(let_go, "its script's move let go");
    assert!(cell && in_grid, "in the grid there");
    assert_eq!(later, to, "no further along the old path");
    let (told, told_facing) = told_put(&mut a, g).await;
    assert!(dist(told, to) < 0.01, "told it is there: {told:?}");
    assert!(
        told_facing.is_some_and(|f| (f - o).abs() < 1e-4),
        "and facing so: {told_facing:?}"
    );
    assert!(lines(&mut a).await.is_empty(), "no point told");
}

/// Thistle Tea's "still records an exact same-position orientation change
/// while stationary": a script's teleport of a creature standing still to
/// where it stands, facing another way, turns it there, and those who see
/// it are told the way it faces (mangos's `NearTeleportTo` tells them with
/// a heartbeat; Thistle Tea with a teleport's movement).
#[tokio::test]
async fn a_scripts_teleport_to_where_a_creature_stands_turns_it_for_all_to_see() {
    let (_realm, mut a, ga, map) = setup_with(walker_world(), Class::Mage).await;
    let [g] = walkers(&mut a, &map, ga, [5.0]).await;
    let (here, turned, after) = on(&map, move |m| {
        let c = m.creature(g).unwrap();
        let (here, o) = (c.position, c.orientation);
        let turned = (o + 1.0).rem_euclid(std::f32::consts::TAU);
        scripts::execute(m, &teleport_to(here, turned), g, Guid::zero());
        let c = m.creature(g).unwrap();
        (here, turned, (c.position, c.orientation))
    })
    .await;
    assert_eq!(after, (here, turned), "where it was, facing the new way");
    let (at, facing) = told_put(&mut a, g).await;
    assert!(dist(at, here) < 0.01, "told it is where it was: {at:?}");
    assert!(
        facing.is_some_and(|f| (f - turned).abs() < 1e-4),
        "told the way it faces: {facing:?} {turned}"
    );
}

/// What a creature's teleports send those who see it, mid-run and on the
/// spot, passes the 1.12 client.
#[tokio::test]
#[ignore = "extended"]
async fn a_scripts_teleports_pass_the_client() {
    use crate::game::tests_client_emu::{capture, checker, run_checker};
    let (check_py, wow_exe) = checker().expect(
        "the client-emulation check needs tools/client_check/WoW.exe and `uv`: see docs/TESTING.md",
    );
    let (_realm, mut a, ga, map) = setup_with(walker_world(), Class::Mage).await;
    let [g] = walkers(&mut a, &map, ga, [5.0]).await;
    on(&map, move |m| {
        let end = east(m, g, 20.0);
        run_to(m, g, end, None);
        turns(m, 10);
        let to = east(m, g, 3.0);
        scripts::execute(m, &teleport_to(to, 2.0), g, Guid::zero());
        scripts::execute(m, &teleport_to(to, 4.0), g, Guid::zero());
    })
    .await;
    told_put(&mut a, g).await;
    told_put(&mut a, g).await;
    let (capture, counts) = capture(vec![&mut a]);
    assert!(
        counts.get(&0x0DD).copied().unwrap_or(0) >= 3,
        "the run and both teleports were captured: {counts:?}"
    );
    run_checker(&check_py, &wow_exe, &capture);
}

const BOAT: u32 = 9_997;
const DEADMINES: u32 = 36;

/// A boat that sails 100 yards from the start, crosses to Kalimdor and
/// back, every 1.6 seconds; and the Deadmines, a dungeon.
fn harbour() -> GameData {
    use crate::game::data::npc::TaxiPathNode;
    use crate::game::data::{GameObjectTemplate, MapInfo, TransportRoute};
    let mut d = data();
    let mut fields = [0u32; 24];
    fields[0] = 79;
    fields[1] = 1000;
    fields[2] = 1000;
    d.gameobject_templates.insert(
        BOAT,
        GameObjectTemplate {
            entry: BOAT,
            kind: 15,
            display_id: 3015,
            name: "The Test Boat".into(),
            size: 1.0,
            data: fields,
            ..Default::default()
        },
    );
    d.transports.push(TransportRoute {
        entry: BOAT,
        name: "Here and There".into(),
        period: 1600,
    });
    let node = |map, x: f32, y: f32| TaxiPathNode {
        map,
        x,
        y,
        z: START.2,
        flags: 0,
        delay: 0,
    };
    d.npc.taxi_path_nodes.insert(
        79,
        vec![
            node(0, START.0, START.1),
            node(0, START.0 + 30.0, START.1),
            node(0, START.0 + 60.0, START.1),
            node(0, START.0 + 100.0, START.1),
            node(1, 990.0, 1000.0),
            node(1, 1000.0, 1000.0),
            node(1, 1010.0, 1000.0),
            node(1, 1020.0, 1000.0),
        ],
    );
    d.maps.insert(
        DEADMINES,
        MapInfo {
            id: DEADMINES,
            internal_name: "DeadminesInstance".into(),
            name: "The Deadmines".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    d
}

/// Send `c` to `map` (its copy, for a dungeon) at `at`, and wait for it to
/// land: where it is.
async fn go(realm: &Arc<Realm>, c: &mut Client, map: u32, at: Vector3d) -> MapKey {
    let g = c.guid;
    let from = realm.registry.online(g).unwrap().map();
    from.call(move |m| m.teleport(g, MapKey::world(map), at, 0.0));
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
    realm.registry.online(g).unwrap().map().key
}

/// Thistle Tea's "keeps global transports and stops instanced transports":
/// the boats and zeppelins are the world's, never a dungeon copy's (one
/// built again gets none); a copy emptied closes, and all it had goes with
/// it, while the boat sails on and is seen again on its map.
#[tokio::test]
async fn a_dungeon_copy_closes_with_all_it_had_while_the_boats_sail_on() {
    let mut cfg = harness::test_config();
    cfg.instance_unload = Duration::from_secs(1);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = Realm::new(cfg, store, startup, harbour(), None);
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let boat = guid::make(HighGuid::MoTransport, 0, BOAT);
    assert!(
        a.sees_within(boat, Duration::from_secs(3)).await,
        "the boat"
    );
    let origin = Vector3d {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    let copy = go(&realm, &mut a, DEADMINES, origin).await;
    assert_ne!(copy.instance, 0, "a copy");
    let (globals, has_boat) = realm
        .maps
        .get(&realm, copy)
        .query(move |m| {
            crate::game::transports::restore(m);
            (m.globals.clone(), m.gameobject(boat).is_some())
        })
        .await
        .unwrap();
    assert!(
        globals.is_empty() && !has_boat,
        "no boat in a copy: {globals:?}"
    );
    go(&realm, &mut a, 0, start()).await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while realm.maps.all().iter().any(|h| h.key == copy) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the copy never closed"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    a.clear();
    assert!(
        a.sees_within(boat, Duration::from_secs(3)).await,
        "the boat sails on"
    );
}
