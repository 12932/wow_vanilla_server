//! Orchestrator: accepts worker registrations + drives them from a stdin REPL.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ahash::AHashMap;
use slab::Slab;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::protocol::{ToOrchestrator, ToWorker, WorkerMetrics, read_frame, write_frame};

#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    pub bind: String,
}

struct Worker {
    worker_id: String,
    addr: std::net::SocketAddr,
    writer: Arc<Mutex<OwnedWriteHalf>>,
    last_metrics: Option<WorkerMetrics>,
    last_seen: Instant,
}

#[derive(Default)]
struct Registry {
    workers: Slab<Worker>,
    by_id: AHashMap<String, usize>,
}

impl Registry {
    fn insert(&mut self, w: Worker) -> usize {
        let id = w.worker_id.clone();
        // De-dupe by id — if a worker reconnects with the same id, drop the old.
        if let Some(&existing) = self.by_id.get(&id) {
            self.workers.try_remove(existing);
        }
        let key = self.workers.insert(w);
        self.by_id.insert(id, key);
        key
    }

    fn remove(&mut self, key: usize) {
        if let Some(w) = self.workers.try_remove(key) {
            self.by_id.remove(&w.worker_id);
        }
    }
}

pub async fn run(cfg: OrchestratorConfig) -> std::io::Result<()> {
    let registry = Arc::new(Mutex::new(Registry::default()));

    // Accept loop.
    let listener = TcpListener::bind(&cfg.bind).await?;
    info!("orchestrator listening on {}", cfg.bind);

    {
        let registry = registry.clone();
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, addr)) => {
                        let registry = registry.clone();
                        tokio::spawn(handle_worker(stream, addr, registry));
                    }
                    Err(e) => warn!("orchestrator accept: {e}"),
                }
            }
        });
    }

    // Aggregated status every 2s.
    {
        let registry = registry.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(2));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                let guard = registry.lock().await;
                let mut total_alive: u32 = 0;
                let mut total_auth_ok: u64 = 0;
                let mut total_auth_fail: u64 = 0;
                let mut total_world_ok: u64 = 0;
                let mut total_world_fail: u64 = 0;
                let mut total_in: u64 = 0;
                let mut total_out: u64 = 0;
                let mut total_send_err: u64 = 0;
                for (_, w) in guard.workers.iter() {
                    if let Some(m) = &w.last_metrics {
                        total_alive = total_alive.saturating_add(m.bots_alive);
                        total_auth_ok = total_auth_ok.saturating_add(m.auth_ok);
                        total_auth_fail = total_auth_fail.saturating_add(m.auth_fail);
                        total_world_ok = total_world_ok.saturating_add(m.world_ok);
                        total_world_fail = total_world_fail.saturating_add(m.world_fail);
                        total_in = total_in.saturating_add(m.messages_in);
                        total_out = total_out.saturating_add(m.messages_out);
                        total_send_err = total_send_err.saturating_add(m.send_errors);
                    }
                }
                info!(
                    workers = guard.workers.len(),
                    alive = total_alive,
                    auth_ok = total_auth_ok,
                    auth_fail = total_auth_fail,
                    world_ok = total_world_ok,
                    world_fail = total_world_fail,
                    msg_in = total_in,
                    msg_out = total_out,
                    send_err = total_send_err,
                    "orchestrator aggregate",
                );
            }
        });
    }

    // Stdin REPL.
    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    println!("orchestrator REPL ready. commands: spawn <N>, stop <N>, status, quit");
    while let Some(line) = lines.next_line().await.transpose() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                warn!("orchestrator stdin: {e}");
                break;
            }
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        let cmd = it.next().unwrap_or("");
        match cmd {
            "spawn" => {
                let n: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                if n == 0 {
                    println!("usage: spawn <N>");
                    continue;
                }
                broadcast(&registry, ToWorker::Spawn { count: n }).await;
                println!("sent spawn {n} to all workers");
            }
            "stop" => {
                let n: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                if n == 0 {
                    println!("usage: stop <N>");
                    continue;
                }
                broadcast(&registry, ToWorker::Stop { count: n }).await;
                println!("sent stop {n} to all workers");
            }
            "status" => {
                let guard = registry.lock().await;
                for (_, w) in guard.workers.iter() {
                    println!("{} @ {}: {:?}", w.worker_id, w.addr, w.last_metrics);
                }
            }
            "quit" => {
                println!("draining all workers");
                broadcast(&registry, ToWorker::Drain).await;
                break;
            }
            other => {
                println!("unknown command: {other}");
            }
        }
    }

    Ok(())
}

async fn broadcast(registry: &Arc<Mutex<Registry>>, msg: ToWorker) {
    let writers: Vec<Arc<Mutex<OwnedWriteHalf>>> = {
        let guard = registry.lock().await;
        guard.workers.iter().map(|(_, w)| w.writer.clone()).collect()
    };
    for w in writers {
        let mut guard = w.lock().await;
        if let Err(e) = write_frame(&mut *guard, &msg).await {
            warn!("orchestrator broadcast failed: {e}");
        }
    }
}

async fn handle_worker(
    stream: TcpStream,
    addr: std::net::SocketAddr,
    registry: Arc<Mutex<Registry>>,
) {
    let _ = stream.set_nodelay(true);
    let (mut r, w) = stream.into_split();
    let writer = Arc::new(Mutex::new(w));

    let hello: ToOrchestrator = match read_frame(&mut r).await {
        Ok(m) => m,
        Err(e) => {
            warn!("orchestrator: handshake from {addr} failed: {e}");
            return;
        }
    };
    let worker_id = match hello {
        ToOrchestrator::Hello { worker_id, .. } => worker_id,
        other => {
            warn!("orchestrator: expected Hello, got {other:?}");
            return;
        }
    };

    let worker = Worker {
        worker_id: worker_id.clone(),
        addr,
        writer: writer.clone(),
        last_metrics: None,
        last_seen: Instant::now(),
    };
    let key = registry.lock().await.insert(worker);
    info!("orchestrator: worker {worker_id} connected from {addr} (slot {key})");

    loop {
        let msg: ToOrchestrator = match read_frame(&mut r).await {
            Ok(m) => m,
            Err(e) => {
                info!("orchestrator: worker {worker_id} disconnected: {e}");
                break;
            }
        };
        match msg {
            ToOrchestrator::Metrics(m) => {
                let mut guard = registry.lock().await;
                if let Some(w) = guard.workers.get_mut(key) {
                    w.last_metrics = Some(m);
                    w.last_seen = Instant::now();
                }
            }
            ToOrchestrator::Drained => {
                info!("orchestrator: worker {worker_id} drained");
                break;
            }
            ToOrchestrator::Hello { .. } => {
                warn!("orchestrator: duplicate Hello from {worker_id}");
            }
        }
    }

    registry.lock().await.remove(key);
}
