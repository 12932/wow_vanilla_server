//! Hot-loop benchmarks for the WoW vanilla server.
//!
//! Run all: `cargo bench --bench hot_loops`
//! Save baseline before a change: `cargo bench --bench hot_loops -- --save-baseline before`
//! Compare after: `cargo bench --bench hot_loops -- --baseline before`
//!
//! Six benches across two tiers:
//!
//! ## Micro (no World fixture, deterministic by input)
//! 1. `within_aoi` — scalar distance check.
//! 2. `build_player_mask_observer` — observer-side update mask.
//! 3. `get_update_object_player_self` — extended SELF-side update mask
//!    (login path; fatter field set).
//! 4. `broadcast_fanout/{100,500,1000}` — full `broadcast_opcode_within_aoi`
//!    against an N-client `Slab`. The bench A2 is supposed to win on.
//!
//! ## Stateful (synthetic World built via `World::for_test`)
//! 5. `tick_aoi_transitions/{200,500}` — one async pass of the
//!    per-tick AOI diff.
//! 6. `world_tick_end_to_end` — one full `World::tick` at modest scale.
//!    Catches system-level regressions in any phase.
//!
//! Fixtures use a fixed-seed RNG (`StdRng::seed_from_u64(0xBEEF)`) so
//! positions are byte-identical across runs.

use std::time::Duration;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

// Bench binary needs its own `#[global_allocator]` declaration to use
// mimalloc — `#[global_allocator]` is per-final-binary, not per-crate.
// Without this the bench would still measure against the default
// allocator and silently miss the production perf.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::RngExt;
use slab::Slab;
use tokio::runtime::Runtime;
use wow_world_base::vanilla::{Map, PlayerGender, RaceClass};
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::{
    MSG_MOVE_HEARTBEAT_Server, MovementInfo, MovementInfo_MovementFlags, Vector3d,
};

use wow_vanilla_server::world::aoi::{broadcast_opcode_within_aoi, within_aoi};
use wow_vanilla_server::world::database::WorldDatabase;
use wow_vanilla_server::world::world::client::Client;
use wow_vanilla_server::world::world::client::test_support::synthetic_client;
use wow_vanilla_server::world::world::{
    World, build_player_mask_observer, get_update_object_player_self,
};
use wow_vanilla_server::world::world_opcode_handler::character::Character;
use wow_vanilla_server::world::world_opcode_handler::creature::Creature;

// Fixed bench-wide seed so positions are reproducible across runs.
const SEED: u64 = 0xBEEF;
const BENCH_MAP: Map = Map::EasternKingdoms;
// Cluster radius (yards) we sample positions inside. Comfortably under
// the default AOI radius (200 yd) so most pairs are in range — that's
// the worst case for fan-out (everyone's a recipient).
const CLUSTER_RADIUS: f32 = 60.0;

fn make_position(rng: &mut StdRng) -> Vector3d {
    Vector3d {
        x: rng.random_range(-CLUSTER_RADIUS..CLUSTER_RADIUS),
        y: rng.random_range(-CLUSTER_RADIUS..CLUSTER_RADIUS),
        z: 0.0,
    }
}

/// Build a level-60 troll-warrior character at the given position. Uses
/// `Character::test_character` (which seeds starter inventory + race/class
/// data) and overrides the position.
fn make_character(db: &mut WorldDatabase, name: &str, pos: Vector3d, map: Map) -> Character {
    let mut c = Character::test_character(
        db,
        name.to_string(),
        RaceClass::TrollWarrior,
        PlayerGender::Male,
    );
    c.map = map;
    c.info.position = pos;
    c.account = "BENCH".to_string();
    c
}

/// Build `n` synthetic clients clustered near the origin. Requires a
/// Tokio runtime context (spawns one writer task per client).
fn build_clients(n: usize) -> Slab<Client> {
    let mut rng = StdRng::seed_from_u64(SEED);
    let mut db = WorldDatabase::new();
    let mut clients: Slab<Client> = Slab::with_capacity(n);
    for i in 0..n {
        let pos = make_position(&mut rng);
        let character = make_character(&mut db, &format!("Bot{i}"), pos, BENCH_MAP);
        clients.insert(synthetic_client(character, "BENCH"));
    }
    clients
}

/// Build `n` characters + `m` creatures for the stateful World benches.
fn build_characters_and_creatures(
    n_clients: usize,
    m_creatures: usize,
) -> (Vec<Character>, Vec<Creature>) {
    let mut rng = StdRng::seed_from_u64(SEED);
    let mut db = WorldDatabase::new();

    let characters: Vec<Character> = (0..n_clients)
        .map(|i| {
            let pos = make_position(&mut rng);
            make_character(&mut db, &format!("Bot{i}"), pos, BENCH_MAP)
        })
        .collect();

    let creatures: Vec<Creature> = (0..m_creatures)
        .map(|i| {
            let mut c = Creature::new(format!("M{i}"), wow_world_base::shared::Guid::new(
                0x4000_0000 + i as u64,
            ));
            c.map = BENCH_MAP;
            c.info.position = make_position(&mut rng);
            c.spawn_position = c.info.position;
            c
        })
        .collect();

    (characters, creatures)
}

// ────────────────────────────────────────────────────────────────────
// 1. within_aoi
// ────────────────────────────────────────────────────────────────────
fn bench_within_aoi(c: &mut Criterion) {
    let a = Vector3d { x: 0.0, y: 0.0, z: 0.0 };
    let b_in = Vector3d { x: 50.0, y: 50.0, z: 0.0 };
    let b_out = Vector3d { x: 500.0, y: 500.0, z: 0.0 };

    let mut group = c.benchmark_group("within_aoi");
    group.bench_function("in_range", |bench| {
        bench.iter(|| within_aoi(std::hint::black_box(&a), std::hint::black_box(&b_in)));
    });
    group.bench_function("out_of_range", |bench| {
        bench.iter(|| within_aoi(std::hint::black_box(&a), std::hint::black_box(&b_out)));
    });
    group.finish();
}

// ────────────────────────────────────────────────────────────────────
// 2 & 3. update-mask builders
// ────────────────────────────────────────────────────────────────────
fn bench_mask_builders(c: &mut Criterion) {
    let mut db = WorldDatabase::new();
    let character = Character::test_character(
        &mut db,
        "MaskBench",
        RaceClass::TrollRogue,
        PlayerGender::Male,
    );

    let mut group = c.benchmark_group("update_mask");
    group.bench_function("observer", |bench| {
        bench.iter(|| {
            let builder = build_player_mask_observer(std::hint::black_box(&character));
            std::hint::black_box(builder.finalize());
        });
    });
    group.bench_function("self", |bench| {
        bench.iter(|| {
            std::hint::black_box(get_update_object_player_self(std::hint::black_box(&character)));
        });
    });
    group.finish();
}

// ────────────────────────────────────────────────────────────────────
// 4. broadcast fan-out (parameterized)
// ────────────────────────────────────────────────────────────────────
fn bench_broadcast_fanout(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    // Construct the message once — same content used for every N.
    let info = MovementInfo {
        flags: MovementInfo_MovementFlags::empty(),
        timestamp: 0,
        position: Vector3d { x: 0.0, y: 0.0, z: 0.0 },
        orientation: 0.0,
        fall_time: 0.0,
    };
    let msg: ServerOpcodeMessage = MSG_MOVE_HEARTBEAT_Server {
        guid: wow_world_base::shared::Guid::new(1),
        info,
    }
    .into();
    let anchor = Vector3d { x: 0.0, y: 0.0, z: 0.0 };

    let mut group = c.benchmark_group("broadcast_fanout");
    // Bumped from defaults (100 samples / 5 s) so per-size confidence
    // intervals are tight enough to distinguish ±5 % effects. At
    // N=1000 each sample is ~700–1000 µs; ~25 s of measurement at
    // 250 samples puts the median's 95 % CI inside ±3 % under steady
    // load.
    group.sample_size(250);
    group.measurement_time(Duration::from_secs(25));
    group.warm_up_time(Duration::from_secs(5));
    for &n in &[100usize, 500, 1000] {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |bench, &n| {
            bench.iter_batched(
                || {
                    // Setup per iteration: spawn N synthetic clients inside
                    // the runtime so the writer tasks are valid. iter_batched
                    // excludes setup from measurement.
                    rt.block_on(async { build_clients(n) })
                },
                |clients| {
                    broadcast_opcode_within_aoi(
                        std::hint::black_box(&msg),
                        anchor,
                        BENCH_MAP,
                        None,
                        &clients,
                    );
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

// ────────────────────────────────────────────────────────────────────
// 5. tick_aoi_transitions (async, stateful)
// ────────────────────────────────────────────────────────────────────
fn bench_tick_aoi_transitions(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let mut group = c.benchmark_group("tick_aoi_transitions");
    // Stateful work-per-sample is larger — fewer samples, more measurement time.
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(8));

    // The third tier (100c, 50000m) matches the real-server ratio (51k
    // creatures, ~100s of clients) where the incremental grid pays
    // off — the old per-tick rebuild iterated all 50k creatures every
    // tick, the maintained grid only touches the few that moved.
    for &(n_clients, m_creatures) in &[(200usize, 2_000usize), (500, 5_000), (100, 50_000)] {
        group.throughput(Throughput::Elements(n_clients as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{n_clients}c_{m_creatures}m")),
            &(n_clients, m_creatures),
            |bench, &(n, m)| {
                bench.iter_batched(
                    || {
                        rt.block_on(async {
                            let (chars, creatures) = build_characters_and_creatures(n, m);
                            World::for_test(chars, creatures)
                        })
                    },
                    |mut world| {
                        rt.block_on(async {
                            let _ = world.tick_aoi_transitions().await;
                        });
                    },
                    BatchSize::PerIteration,
                );
            },
        );
    }
    group.finish();
}

// ────────────────────────────────────────────────────────────────────
// 6. world_tick — end-to-end, smaller scale
// ────────────────────────────────────────────────────────────────────
fn bench_world_tick(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let mut group = c.benchmark_group("world_tick");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(8));

    group.bench_function("100c_500m", |bench| {
        bench.iter_batched(
            || {
                rt.block_on(async {
                    let (chars, creatures) = build_characters_and_creatures(100, 500);
                    let world = World::for_test(chars, creatures);
                    let db = WorldDatabase::new();
                    (world, db)
                })
            },
            |(mut world, mut db)| {
                rt.block_on(async {
                    world.tick(&mut db, Duration::from_millis(200)).await;
                });
            },
            BatchSize::PerIteration,
        );
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_within_aoi,
    bench_mask_builders,
    bench_broadcast_fanout,
    bench_tick_aoi_transitions,
    bench_world_tick,
);
criterion_main!(benches);
