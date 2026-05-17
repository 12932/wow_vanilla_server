//! Shared PvP state for a single bot.
//!
//! The bot's `read_fut` parses inbound movement opcodes (HEARTBEAT, START_*,
//! STOP, JUMP, SET_FACING) and feeds the `(guid, position)` pair into
//! `PvpState`. The `MovementDriver` (running on the same task) reads it to
//! pick targets and aim. Wrapped in `Arc<Mutex<>>` because the two futures
//! run inside the same `tokio::select!` and we want non-async access.
//!
//! Only used when the worker is started with `--pvp`. Without that flag the
//! bot doesn't allocate this state at all — the `Mode::Random` driver path
//! is unaffected.

use ahash::AHashMap;
use rand::seq::IteratorRandom;
use std::time::{Duration, Instant};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

/// Entries older than this are pruned at pick time. Stale targets aren't
/// worth chasing — by the time we'd reach the last-known position, the
/// real owner has long since moved on. 30 s matches the server's
/// per-channel ping cadence so any genuinely-active player will still be
/// in the cache.
const STALE_AFTER: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
pub struct PvpState {
    seen: AHashMap<Guid, (Vector3d, Instant)>,
    /// Server-side damage applied to *us* since the last respawn. The
    /// reader sums each inbound `SMSG_ATTACKERSTATEUPDATE` where
    /// `target == own_guid`. We compare against the server's HP cap
    /// (`PVP_MAX_HEALTH = 100`) to decide that we've died, instead of
    /// trying to parse the partial-mask `SMSG_UPDATE_OBJECT` payloads
    /// the server emits on hit. A few packet drops will desync this
    /// briefly, but the next tick of broadcasts puts us back on track —
    /// fine for a stress-test client.
    pub damage_taken: u32,
    /// Set when `damage_taken` first crosses the death threshold. The
    /// driver consults this to suspend movement / swings while dead and
    /// to schedule the post-`RESPAWN_DELAY` teleport.
    pub last_death_at: Option<Instant>,
}

impl PvpState {
    pub fn observe(&mut self, guid: Guid, pos: Vector3d) {
        self.seen.insert(guid, (pos, Instant::now()));
    }

    pub fn position_of(&self, guid: Guid) -> Option<Vector3d> {
        self.seen.get(&guid).map(|(p, _)| *p)
    }

    /// Pick a random non-stale target that isn't `exclude` (typically the
    /// caller's own guid). Returns `None` if the cache has nothing fresh.
    pub fn pick_random_target(&mut self, exclude: Guid) -> Option<(Guid, Vector3d)> {
        let now = Instant::now();
        self.seen
            .retain(|_, (_, seen)| now.duration_since(*seen) < STALE_AFTER);
        let mut rng = rand::rng();
        self.seen
            .iter()
            .filter(|(g, _)| **g != exclude)
            .choose(&mut rng)
            .map(|(g, (p, _))| (*g, *p))
    }

    /// Called from the reader for every inbound combat-log packet
    /// targeting us. We stop accumulating once we've crossed the death
    /// threshold so a flood of subsequent corpse-hits doesn't trip a
    /// second "death" event before the respawn timer fires.
    pub fn take_damage(&mut self, amount: u32) {
        if self.last_death_at.is_some() {
            return;
        }
        self.damage_taken = self.damage_taken.saturating_add(amount);
        if self.damage_taken >= PVP_MAX_HEALTH {
            self.last_death_at = Some(Instant::now());
        }
    }

    /// Reset death + damage state after we've teleported to a respawn
    /// position. Caller is responsible for actually moving the bot —
    /// we just clear the bookkeeping.
    pub fn mark_respawned(&mut self) {
        self.damage_taken = 0;
        self.last_death_at = None;
    }
}

/// Mirrors the server-side `PVP_MAX_HEALTH` in
/// `src/world/world_opcode_handler/character.rs`. Keep in sync — if the
/// server lowers it, bots will think they died with HP still on the bar.
pub const PVP_MAX_HEALTH: u32 = 100;
