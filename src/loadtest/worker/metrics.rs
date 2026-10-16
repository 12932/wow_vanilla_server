use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::protocol::WorkerMetrics;

#[derive(Debug, Default)]
pub struct Metrics {
    pub bots_alive: AtomicU32,
    /// Total bots the worker has been *asked* to spawn (cumulative across
    /// spawn_n calls). Used by the per-second printer to show ramp progress.
    pub target_bots: AtomicU32,
    pub auth_ok: AtomicU64,
    pub auth_fail: AtomicU64,
    pub world_ok: AtomicU64,
    pub world_fail: AtomicU64,
    pub bytes_in: AtomicU64,
    pub bytes_out: AtomicU64,
    pub send_errors: AtomicU64,
    pub messages_in: AtomicU64,
    pub messages_out: AtomicU64,
    /// Server packets skipped because wow_messages doesn't model their
    /// opcode, or the bot didn't need them during the handshake.
    pub skipped_in: AtomicU64,
    /// Server packets wow_messages couldn't parse. Skipped too: the framing
    /// is intact, since the body was read before parsing.
    pub parse_errors: AtomicU64,
    /// Bots that gave up because their login took longer than the deadline.
    pub login_timeouts: AtomicU64,
    /// Other players' movement broadcasts received (`MSG_MOVE_*`): proof the
    /// server sees the bots and fans their movement out.
    pub movement_in: AtomicU64,
}

impl Metrics {
    pub fn snapshot(&self) -> WorkerMetrics {
        WorkerMetrics {
            bots_alive: self.bots_alive.load(Ordering::Relaxed),
            auth_ok: self.auth_ok.load(Ordering::Relaxed),
            auth_fail: self.auth_fail.load(Ordering::Relaxed),
            world_ok: self.world_ok.load(Ordering::Relaxed),
            world_fail: self.world_fail.load(Ordering::Relaxed),
            bytes_in_total: self.bytes_in.load(Ordering::Relaxed),
            bytes_out_total: self.bytes_out.load(Ordering::Relaxed),
            send_errors: self.send_errors.load(Ordering::Relaxed),
            messages_in: self.messages_in.load(Ordering::Relaxed),
            messages_out: self.messages_out.load(Ordering::Relaxed),
            skipped_in: self.skipped_in.load(Ordering::Relaxed),
            parse_errors: self.parse_errors.load(Ordering::Relaxed),
            login_timeouts: self.login_timeouts.load(Ordering::Relaxed),
            movement_in: self.movement_in.load(Ordering::Relaxed),
        }
    }
}
