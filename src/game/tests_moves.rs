//! A burst's movement across the threads (`map/moves.rs`) against the same
//! packets one by one: two crowds alike, the same bursts, and after each
//! every player's state, every client's packets, visibility's marks and
//! the grid the same; then what the update's visibility makes of it.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::RealmConfig;
use crate::game::harness;
use crate::game::map::vis_bench::Crowd;
use crate::game::map::{Map, auras, death, movement, moves, visibility};

fn crowd(n: usize) -> Crowd {
    Crowd::with_config(
        RealmConfig {
            visibility_threads: 4,
            parallel_moves: true,
            ..harness::test_config()
        },
        n,
        40.0,
    )
}

/// A small, fixed generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Where the generator has each player going: its own idea, the same for
/// both crowds whatever the server made of the last packet.
#[derive(Clone, Copy)]
struct Walker {
    at: Vector3d,
    o: f32,
}

fn info(w: &Walker, flags: MovementInfo_MovementFlags, fall_time: u32) -> MovementInfo {
    MovementInfo {
        flags,
        timestamp: 0,
        position: w.at,
        orientation: w.o,
        fall_time,
    }
}

/// The next packet of a walker: mostly steps of a run, with starts,
/// stops, turns, jumps and their landings, a long fall now and then, and
/// a leap no one could make.
fn packet(rng: &mut Rng, w: &mut Walker) -> ClientOpcodeMessage {
    use ClientOpcodeMessage as C;
    let none = MovementInfo_MovementFlags::empty();
    let roll = rng.below(1000);
    let step = 0.3 + rng.unit();
    let ahead = |w: &mut Walker, d: f32| {
        w.at.x += d * w.o.cos();
        w.at.y += d * w.o.sin();
    };
    match roll {
        0..=599 => {
            ahead(w, step);
            C::MSG_MOVE_HEARTBEAT(MSG_MOVE_HEARTBEAT_Client {
                info: info(w, none.set_forward(), 0),
            })
        }
        600..=699 => C::MSG_MOVE_START_FORWARD(MSG_MOVE_START_FORWARD_Client {
            info: info(w, none.set_forward(), 0),
        }),
        700..=779 => C::MSG_MOVE_STOP(MSG_MOVE_STOP_Client {
            info: info(w, none, 0),
        }),
        780..=869 => {
            w.o = (w.o + rng.unit() * 2.0 - 1.0).rem_euclid(std::f32::consts::TAU);
            C::MSG_MOVE_SET_FACING(MSG_MOVE_SET_FACING_Client {
                info: info(w, none, 0),
            })
        }
        870..=929 => {
            ahead(w, step);
            C::MSG_MOVE_JUMP(MSG_MOVE_JUMP_Client {
                info: info(w, none.set_forward().set_jumping(Default::default()), 0),
            })
        }
        930..=979 => C::MSG_MOVE_FALL_LAND(MSG_MOVE_FALL_LAND_Client {
            info: info(w, none, 400),
        }),
        // A long fall's landing, from 40 yards up: it hurts.
        980..=989 => {
            w.at.z += 40.0;
            let up = C::MSG_MOVE_HEARTBEAT(MSG_MOVE_HEARTBEAT_Client {
                info: info(w, none.set_fallingfar(), 0),
            });
            w.at.z -= 40.0;
            up
        }
        990..=994 => C::MSG_MOVE_FALL_LAND(MSG_MOVE_FALL_LAND_Client {
            info: info(w, none, 2_500),
        }),
        // A leap no run could make: pulled back.
        _ => {
            let mut far = *w;
            far.at.x += 60.0;
            C::MSG_MOVE_HEARTBEAT(MSG_MOVE_HEARTBEAT_Client {
                info: info(&far, none.set_forward(), 0),
            })
        }
    }
}

/// What a crowd's players are, as far as movement goes, and their marks.
fn state(c: &Crowd) -> Vec<String> {
    let m = &c.map;
    let mut out: Vec<String> = c
        .guids
        .iter()
        .filter_map(|g| m.players.get(g))
        .map(|p| {
            format!(
                "{} {:?} {:?} {:?} {:?} {:?} {} {} {:?} {:?} {} {} {} {:?} {}",
                p.guid,
                p.movement,
                p.move_check.map(|(at, _)| at),
                p.cell,
                p.fall_from,
                p.misc.move_guard,
                p.misc.fall_far,
                p.teleport.is_some(),
                p.safe_position,
                p.vis_anchor,
                p.stand_state(),
                p.auras.holders.len(),
                p.fields.get_u32(crate::game::fields::UNIT_FIELD_HEALTH),
                p.death,
                m.visibility_dirty.kind(&p.guid).map_or(0, |k| k as u8 + 1),
            )
        })
        .collect();
    out.push(format!("holding {}", m.holding.len()));
    out
}

/// A frame as two ways of working must agree on it: an update's forgets
/// in order of their guids (their list is a hash set's order, the map's
/// own, which no client is the worse for).
fn plain(mut f: Vec<u8>) -> Vec<u8> {
    // `[size][opcode]`, the block count, then the first block's type.
    if f.len() < 14 || u16::from_le_bytes([f[2], f[3]]) != 0xA9 || f[9] != 4 {
        return f;
    }
    let n = u32::from_le_bytes([f[10], f[11], f[12], f[13]]) as usize;
    let mut at = 14;
    let mut guids = Vec::with_capacity(n);
    for _ in 0..n {
        let Some(&mask) = f.get(at) else {
            return f;
        };
        let len = 1 + mask.count_ones() as usize;
        let Some(g) = f.get(at..at + len) else {
            return f;
        };
        guids.push(g.to_vec());
        at += len;
    }
    guids.sort();
    let sorted: Vec<u8> = guids.concat();
    f[14..at].copy_from_slice(&sorted);
    f
}

/// A burst of `packets` on `m`: one by one, as the map's run had them, or
/// kept and worked out at once.
fn burst(m: &mut Map, packets: &[(Guid, ClientOpcodeMessage)], across: bool) {
    let cork = crate::game::conn::Cork::new();
    cork.feed(&m.clients.feed);
    let mut kept = Vec::new();
    for (g, msg) in packets {
        let Ok((info, relay)) = movement::movement_of(msg.clone()) else {
            unreachable!("movement");
        };
        if across {
            kept.push(moves::Move {
                guid: *g,
                info,
                relay,
                room: None,
                name: "",
            });
        } else {
            movement::on_move(m, *g, info, relay);
            crate::game::map::after_event(m);
        }
    }
    if across {
        moves::handle_all(m, &mut kept);
        crate::game::map::after_event(m);
    }
    drop(cork);
}

/// Something that changes a player between bursts, alike in both crowds.
fn change(m: &mut Map, g: Guid, what: usize) {
    const ROOT: u32 = 97_900;
    const FOOD: u32 = 97_901;
    match what {
        0 => crate::game::tests_p3_combat::aura_of(m, g, ROOT, auras::AURA_MOD_ROOT, 0, 1),
        1 => auras::remove_spell(m, g, ROOT, None),
        2 => {
            if let Some(p) = m.players.get_mut(&g) {
                p.set_stand_state(1);
            }
        }
        // An aura moving ends.
        3 => {
            let mut s = crate::game::data::spells::SpellTemplate {
                id: FOOD,
                name: "Test Food".into(),
                equipped_item_class: -1,
                aura_interrupt_flags: auras::INTERRUPT_MOVE,
                ..Default::default()
            };
            s.effects[0] = crate::game::data::spells::SpellEffect {
                effect: 6,
                aura: auras::AURA_PERIODIC_HEAL,
                base_points: 1,
                base_dice: 1,
                target_a: 1,
                amplitude: 1000,
                ..Default::default()
            };
            let _ = auras::apply_spell(m, g, g, &s, &[(0, 2)], auras::Context::default());
        }
        4 => {
            let now = m.now;
            if let Some(p) = m.players.get_mut(&g) {
                p.death = death::Death::Dead { since: now };
            }
        }
        5 => {
            if let Some(p) = m.players.get_mut(&g) {
                p.death = death::Death::Alive;
            }
        }
        _ => {
            if let Some(p) = m.players.get_mut(&g) {
                p.gm = !p.gm;
            }
        }
    }
}

/// The same bursts on two crowds, one by one and across the threads: the
/// same in every way after each, with enough of them taken across the
/// threads (and enough left to be handled alone) to have tried both.
#[test]
fn a_burst_across_the_threads_comes_out_as_one_by_one() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let _in = rt.enter();
    for seed in [7u64, 1234, 99_991] {
        let n = 150;
        let mut one = crowd(n);
        let mut all = crowd(n);
        let start = Instant::now();
        let mut walkers: Vec<Walker> = one
            .guids
            .iter()
            .map(|g| {
                let p = &one.map.players[g];
                Walker {
                    at: p.position(),
                    o: (g.guid() % 6) as f32,
                }
            })
            .collect();
        let mut rng = Rng(seed);
        for b in 0..40u32 {
            let now = start + Duration::from_millis(250 * u64::from(b + 1));
            one.map.now = now;
            all.map.now = now;
            // Between bursts, a few players changed alike.
            for _ in 0..3 {
                let i = rng.below(n);
                let what = rng.below(7);
                change(&mut one.map, one.guids[i], what);
                change(&mut all.map, all.guids[i], what);
            }
            let mut packets = Vec::new();
            for _ in 0..(120 + rng.below(160)) {
                let i = rng.below(n);
                packets.push((one.guids[i], packet(&mut rng, &mut walkers[i])));
            }
            burst(&mut one.map, &packets, false);
            burst(&mut all.map, &packets, true);
            assert_eq!(
                state(&one),
                state(&all),
                "seed {seed}, burst {b}: the players"
            );
            for i in 0..n {
                let x: Vec<Vec<u8>> = one.take_frames(i).into_iter().map(plain).collect();
                let y: Vec<Vec<u8>> = all.take_frames(i).into_iter().map(plain).collect();
                if x != y {
                    let ops = |f: &Vec<Vec<u8>>| -> Vec<String> {
                        f.iter()
                            .map(|f| {
                                format!("{:x}:{:?}", u16::from_le_bytes([f[2], f[3]]), &f[4..6])
                            })
                            .collect()
                    };
                    let k = x
                        .iter()
                        .zip(&y)
                        .position(|(a, b)| a != b)
                        .unwrap_or(x.len().min(y.len()));
                    panic!(
                        "seed {seed}, burst {b}: what client {i} was sent: {} and {} frames, first unlike {k}:\n{:?}\n{:?}\n{:?}",
                        x.len(),
                        y.len(),
                        x.get(k),
                        y.get(k),
                        ops(&x)[k.saturating_sub(2)..(k + 2).min(x.len())].to_vec()
                    );
                }
            }
            // The update's visibility, alike.
            visibility::refresh_dirty(&mut one.map);
            visibility::refresh_dirty(&mut all.map);
            assert_eq!(one.knowledge(), all.knowledge(), "seed {seed}, burst {b}");
            one.drain();
            all.drain();
        }
        let s = &all.map.stats;
        assert!(
            s.moves_across > 1000 && s.moves_alone > 100,
            "seed {seed}: both ways tried ({} across, {} alone)",
            s.moves_across,
            s.moves_alone
        );
    }
}

/// A probe, by hand (`--run-ignored all probe_moves`): a crowd of 2,000's
/// bursts one by one and across the threads, timed.
#[test]
#[ignore = "a probe"]
fn probe_moves() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let _in = rt.enter();
    let n = 2000;
    for threads in [1usize, 4] {
        let mut c = Crowd::with_config(
            RealmConfig {
                visibility_threads: threads,
                parallel_moves: true,
                ..harness::test_config()
            },
            n,
            60.0,
        );
        let start = Instant::now();
        let mut walkers: Vec<Walker> = c
            .guids
            .iter()
            .map(|g| Walker {
                at: c.map.players[g].position(),
                o: (g.guid() % 6) as f32,
            })
            .collect();
        let mut rng = Rng(5);
        let (mut one_t, mut all_t) = (Duration::ZERO, Duration::ZERO);
        for b in 0..60u32 {
            c.map.now = start + Duration::from_millis(100 * u64::from(b + 1));
            let mut packets = Vec::new();
            for _ in 0..400 {
                let i = rng.below(n);
                // Steps alone: the plain path.
                walkers[i].at.x += 0.2 * walkers[i].o.cos();
                walkers[i].at.y += 0.2 * walkers[i].o.sin();
                packets.push((
                    c.guids[i],
                    ClientOpcodeMessage::MSG_MOVE_HEARTBEAT(MSG_MOVE_HEARTBEAT_Client {
                        info: info(
                            &walkers[i],
                            MovementInfo_MovementFlags::empty().set_forward(),
                            0,
                        ),
                    }),
                ));
            }
            let across = b % 2 == 1;
            let t = Instant::now();
            burst(&mut c.map, &packets, across);
            let d = t.elapsed();
            if across {
                all_t += d;
            } else {
                one_t += d;
            }
            c.drain();
        }
        eprintln!(
            "threads {threads}: one by one {:?} a packet, across {:?} ({} across, {} alone)",
            one_t / (30 * 400),
            all_t / (30 * 400),
            c.map.stats.moves_across,
            c.map.stats.moves_alone
        );
    }
}
