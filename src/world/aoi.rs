use crate::world::world::client::{Client, OutboundTx};
use crate::world::world_opcode_handler::{write_message_test, write_server_test};
use ahash::AHashMap;
use rayon::prelude::*;
use slab::Slab;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use wow_world_base::vanilla::Map;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::{Object, ServerMessage, Vector3d};

/// `Sync`-safe per-client broadcast handle. Snapshotted from every
/// `Client` at the top of the broadcast phase so the fan-out loop can
/// run on a rayon thread pool without needing `Client: Sync` — `Client`
/// embeds a `tokio::sync::mpsc::Receiver` which is single-consumer and
/// therefore `!Sync`, blocking direct rayon iteration over `&Slab<Client>`.
///
/// Every field is either `Copy` (Map, Vector3d, Guid) or `Arc`-backed
/// (`OutboundTx` is `Clone` with internally-Arc'd channel + semaphore,
/// `dropped_packets` is shared with the source `Client`, `account_name`
/// is `Arc<str>`). Building one is a handful of refcount bumps —
/// negligible vs the broadcast loop it feeds.
#[derive(Debug)]
pub struct BroadcastTarget {
    pub map: Map,
    pub position: Vector3d,
    pub guid: Guid,
    pub outbound: OutboundTx,
    /// Shared with the source `Client::session.dropped_packets`. The
    /// broadcast view bumps it on `try_send` failure, exactly like
    /// `PlayerSession::queue_buf` does. Same Arc, same counter.
    pub dropped_packets: Arc<AtomicU64>,
    /// Used only inside the cold-path warn — first drop per client
    /// emits a `tracing::warn` naming the account.
    pub account_name: Arc<str>,
}

impl BroadcastTarget {
    /// Try to queue a pre-framed buffer on this client's outbound
    /// channel. Mirrors `PlayerSession::queue_buf`: success → return
    /// `true`; failure → bump the shared dropped-packet counter and
    /// (on the first drop) log a warn, then return `false`.
    ///
    /// Marked `#[inline]` because the broadcast loop calls this once
    /// per recipient — at high density it's the inner-most hot loop
    /// on the broadcast path.
    #[inline]
    pub fn try_queue_frame(&self, buf: Arc<[u8]>) -> bool {
        if self.outbound.try_send(buf) {
            return true;
        }
        self.note_drop();
        false
    }

    /// Cold tail of [`Self::try_queue_frame`]. Same first-drop-only
    /// warn behavior as `PlayerSession::queue_buf_dropped`; pulled out
    /// so LLVM keeps the success path straight-line.
    #[cold]
    #[inline(never)]
    fn note_drop(&self) {
        let prior = self.dropped_packets.fetch_add(1, Ordering::Relaxed);
        if prior == 0 {
            tracing::warn!(
                "outbound byte budget exhausted for {} (BroadcastTarget); \
                 dropping packets — client is falling behind",
                self.account_name,
            );
        }
    }
}

/// Lightweight position+guid snapshot of a single creature for the
/// cross-region AoI discovery scan. Has only what the diff scan reads
/// — position for the radius check, guid to record in `visible_entities`
/// and to look up in [`GlobalAoiSnapshot::create_object_by_guid`] when
/// building the entered-objects packet.
#[derive(Debug, Clone, Copy)]
pub struct CreatureView {
    pub guid: Guid,
    pub position: Vector3d,
}

/// Cross-region AoI snapshot built once per `World::tick` from
/// end-of-last-tick state. Each per-region [`tick_aoi_transitions`]
/// pass reads this instead of its own `broadcast_view` /
/// `creature_cells` so observers at a region boundary discover the
/// entities past the boundary.
///
/// Tradeoff vs. always-fresh: cross-region entities appear one tick
/// stale to the diff scan (33 ms at 30 Hz). Local entities are still
/// included in the snapshot so the local-vs-cross-region treatment is
/// uniform — every observer reads from the same view.
///
/// Built at the top of `World::tick`, before per-region tasks spawn,
/// by locking each region briefly and copying out the relevant fields.
/// Wrapped in `Arc` for cheap distribution to the spawned tasks.
pub struct GlobalAoiSnapshot {
    /// Every connected client across every region.
    pub broadcast_view: Vec<BroadcastTarget>,
    /// 250-yd cell-keyed creature index. Diff scan's 3×3 window
    /// (1×CREATURE_GRID_CELL_YD) lands cleanly across region
    /// boundaries because the snapshot is keyed by (Map, cx, cy),
    /// not by region.
    pub creature_cells: AHashMap<(Map, i32, i32), Vec<CreatureView>>,
    /// Pre-built CreateObject2 for every entity (player + creature)
    /// in the snapshot. Diff scan looks up here to build
    /// `entered_objects` without needing live access to neighbor
    /// regions' slabs.
    pub create_object_by_guid: AHashMap<Guid, Object>,
}

impl GlobalAoiSnapshot {
    pub fn empty() -> Self {
        Self {
            broadcast_view: Vec::new(),
            creature_cells: AHashMap::new(),
            create_object_by_guid: AHashMap::new(),
        }
    }
}

/// Horizontal radius (yards) at which players are mutually visible.
/// Z is **deliberately ignored**: a target 200 units above me is still
/// in AOI as long as the horizontal projection is within range. The
/// effective value comes from `[network] aoi_radius_yards` in
/// `config.toml`; this fn reads the global config once per call.
///
/// Inside hot loops that call this hundreds of thousands of times per
/// tick (broadcast fan-out, `tick_aoi_transitions`), prefer
/// [`within_aoi_sq`] with a pre-squared radius hoisted out of the
/// loop — same arithmetic, no per-iter `OnceCell` read.
#[inline]
pub fn within_aoi(observer: &Vector3d, anchor: &Vector3d) -> bool {
    let r = crate::config::config().network.aoi_radius_yards;
    within_aoi_sq(observer, anchor, r * r)
}

/// Squared-radius variant for hot loops. Caller hoists
/// `let r = config().network.aoi_radius_yards; let r_sq = r * r;` out
/// of the loop and passes `r_sq` in. Halves the work per iter (no
/// config lookup, no square) and lets LLVM keep `r_sq` in a register.
#[inline]
pub fn within_aoi_sq(observer: &Vector3d, anchor: &Vector3d, r_sq: f32) -> bool {
    let dx = observer.x - anchor.x;
    let dy = observer.y - anchor.y;
    dx * dx + dy * dy <= r_sq
}

/// Broadcast a message to every client within AOI of `anchor` on `anchor_map`.
///
/// Serializes the message body **once** into a scratch buffer and reuses it
/// for every recipient; only the 4-byte size+opcode header gets re-encrypted
/// per viewer (necessary because each client's `EncrypterHalf` has its own
/// stream cipher state). Replaces the older per-viewer `msg.clone()` +
/// `tokio_write_encrypted_server` path, which serialized the same payload N
/// times for N viewers.
#[tracing::instrument(level = "info", skip_all, name = "broadcast_within_aoi")]
pub async fn broadcast_within_aoi<M: ServerMessage + Sync>(
    msg: M,
    anchor: Vector3d,
    anchor_map: Map,
    clients: &mut Slab<Client>,
) {
    write_message_test(&msg);

    // Serialize the body once.
    let mut body = Vec::with_capacity(msg.size_without_header() as usize);
    if let Err(e) = msg.write_into_vec(&mut body) {
        tracing::warn!("broadcast_within_aoi: serialize failed: {e}");
        return;
    }
    let opcode = M::OPCODE as u16;

    // Wrap a complete wire frame `[size_BE u16][opcode_LE u16][body]` in
    // `Arc<[u8]>` ONCE, so the local fan-out + cross-region delivery
    // both share a single allocation. Per-recipient cost is an atomic
    // refcount bump.
    let size_for_header = (body.len() as u16).saturating_add(2);
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&size_for_header.to_be_bytes());
    buf.extend_from_slice(&opcode.to_le_bytes());
    buf.extend_from_slice(&body);
    let frame: Arc<[u8]> = Arc::from(buf);
    let frame_bytes = frame.len();

    // Local fan-out: every client in the source region within AOI gets
    // a refcount-bump of the shared frame.
    for (_, c) in clients.iter_mut() {
        if c.character().map == anchor_map && within_aoi(&c.character().info.position, &anchor) {
            c.try_queue_frame(Arc::clone(&frame));
        }
    }

    // ── Cross-region post-fanout ──
    //
    // Mirror of the post-fanout in `broadcast_opcode_within_aoi`: any
    // neighbor region whose interior overlaps this anchor's AOI disc
    // gets the same `Arc<[u8]>` cloned into its inbox. Neighbor's next
    // broadcast phase drains the inbox and fans out to its own clients
    // via `aoi::fanout_frame`. Used by combat / HP updates / spawn /
    // despawn — these all go through `broadcast_within_aoi`.
    //
    // No effect when there are no neighbor regions (single-region world
    // or anchor deep inside its region's interior).
    let aoi_r = crate::config::config().network.aoi_radius_yards;
    let anchor_region = crate::world::region::RegionKey::from_position(
        anchor_map, anchor.x, anchor.y,
    );
    let neighbors = crate::world::region::regions_within_aoi(anchor, anchor_map, aoi_r);
    if neighbors.len() > 1 {
        let table = crate::world::region::routing().load();
        for neighbor in neighbors.iter().filter(|n| **n != anchor_region) {
            if let Some(inbox) = table.inboxes.get(neighbor) {
                let send = inbox.cross_region_tx.try_send(
                    crate::world::region::CrossRegionMsg::Frame(
                        crate::world::region::CrossRegionFrame {
                            anchor,
                            anchor_map,
                            exclude_guid: None,
                            frame: Arc::clone(&frame),
                            frame_bytes,
                        },
                    ),
                );
                match send {
                    Ok(true) => {
                        crate::world::region::CROSS_REGION_EMITTED
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    Ok(false) | Err(_) => {
                        crate::world::region::CROSS_REGION_DROPPED
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
        }
    }
}

/// Broadcast a [`ServerOpcodeMessage`] (the enum) to every client in AOI,
/// optionally skipping a specific source guid. Used by the per-tick movement
/// flush: the source client's own movement opcodes must NOT be echoed back
/// to them — the client treats an inbound `MSG_MOVE_*_Server` for its own
/// guid as a position correction and snaps the local character, producing
/// rubber-band / "laggy movement" symptoms.
///
/// Serializes the message into a complete unencrypted server frame
/// `[size_BE u16][opcode_LE u16][body]` *once*, wraps it in an `Arc<[u8]>`,
/// and refcount-bumps that Arc into each recipient's outbound channel. The
/// writer task re-encrypts the 4-byte header per recipient (encryption is
/// stateful per stream) by writing the encrypted header bytes alongside the
/// shared body slice — see `run_writer`.
///
/// Pre-A2 this did `frame.clone()` per recipient — one `Vec<u8>`
/// allocation + memcpy per observer. At 1000-bot Gurubashi density that
/// was ~500k allocs/sec on the broadcast path; the Arc-shared version
/// replaces those with refcount bumps and pays the one alloc upstream.
///
/// Returns `(recipients, frame_bytes)` so the caller can aggregate
/// per-tick throughput plots without re-walking the slab. `frame_bytes`
/// is the per-recipient cost — total bytes broadcast is
/// `recipients * frame_bytes`.
#[tracing::instrument(level = "info", skip_all, name = "broadcast_opcode_within_aoi")]
pub fn broadcast_opcode_within_aoi(
    msg: &ServerOpcodeMessage,
    anchor: Vector3d,
    anchor_map: Map,
    exclude_guid: Option<Guid>,
    targets: &[BroadcastTarget],
) -> (usize, usize) {
    write_server_test(msg);

    // Pre-allocate for a typical movement opcode (heartbeat ~50 B,
    // transitions ~60 B). Avoids the 0→8→16→32→64-byte growth ladder
    // the empty `Vec::new()` would walk while `write_unencrypted_server`
    // pushes bytes. Trims a handful of reallocs off the serialize phase.
    let mut frame = Vec::with_capacity(96);
    if msg.write_unencrypted_server(&mut frame).is_err() {
        return broadcast_serialize_failed();
    }
    let frame_bytes = frame.len();
    // Convert Vec<u8> to Arc<[u8]> once. The `From<Vec<u8>>` impl goes
    // via `Box<[u8]>` which reuses the Vec's allocation (no body memcpy
    // here as long as the Vec doesn't need to shrink). After this point
    // every recipient receives an `Arc::clone` — an atomic refcount bump
    // — instead of a fresh allocation + memcpy of `frame_bytes` bytes.
    let frame: Arc<[u8]> = Arc::from(frame);

    let recipients = fanout_frame(
        Arc::clone(&frame),
        frame_bytes,
        anchor,
        anchor_map,
        exclude_guid,
        targets,
    ).0;

    // ── Cross-region post-fanout ──
    //
    // After the local fanout, look up the neighbor regions whose
    // interior intersects this broadcast's AOI disc. For each
    // neighbor (skipping the anchor's own region), Arc::clone the
    // frame and try_send it to that region's inbox. The receiving
    // region drains the inbox at the top of its next broadcast phase
    // and applies the same `try_queue_frame` fan-out to its own
    // local clients.
    //
    // Until Stage 3 partition lands, the routing table is empty and
    // this is a no-op (zero allocations beyond the iterator setup —
    // `regions_within_aoi` always returns the anchor's own region as
    // the first entry, and the routing-table lookup short-circuits
    // on a missing inbox).
    let aoi_r = crate::config::config().network.aoi_radius_yards;
    let anchor_region = crate::world::region::RegionKey::from_position(
        anchor_map,
        anchor.x,
        anchor.y,
    );
    let neighbors = crate::world::region::regions_within_aoi(anchor, anchor_map, aoi_r);
    if neighbors.len() > 1 {
        let table = crate::world::region::routing().load();
        for neighbor in neighbors.iter().filter(|n| **n != anchor_region) {
            if let Some(inbox) = table.inboxes.get(neighbor) {
                let send = inbox.cross_region_tx.try_send(
                    crate::world::region::CrossRegionMsg::Frame(
                        crate::world::region::CrossRegionFrame {
                            anchor,
                            anchor_map,
                            exclude_guid,
                            frame: Arc::clone(&frame),
                            frame_bytes,
                        },
                    ),
                );
                match send {
                    Ok(true) => {
                        crate::world::region::CROSS_REGION_EMITTED
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    Ok(false) | Err(_) => {
                        crate::world::region::CROSS_REGION_DROPPED
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
        }
    }
    (recipients, frame_bytes)
}

/// Cold helper for the serialize-error branch. Pulling the warn out of
/// the hot fn lets LLVM keep the success path's code straight and
/// register-clean; `#[cold]` further nudges branch prediction.
#[cold]
#[inline(never)]
fn broadcast_serialize_failed() -> (usize, usize) {
    tracing::warn!("broadcast_opcode_within_aoi: serialize failed");
    (0, 0)
}

/// Fan-out a *pre-serialized* frame to AOI targets. Factored out of
/// [`broadcast_opcode_within_aoi`] so the per-region cross-region
/// inbox drain can reuse the same parallel-filter + try_queue_frame
/// loop without re-serializing.
///
/// The caller has already produced the `Arc<[u8]>` frame (either
/// freshly via `write_unencrypted_server` or by `Arc::clone`'ing a
/// [`crate::world::region::CrossRegionFrame`]). `targets` is the
/// receiving region's `broadcast_view`. Returns `(recipients,
/// frame_bytes)` so the caller can aggregate Tracy plots.
pub fn fanout_frame(
    frame: Arc<[u8]>,
    frame_bytes: usize,
    anchor: Vector3d,
    anchor_map: Map,
    exclude_guid: Option<Guid>,
    targets: &[BroadcastTarget],
) -> (usize, usize) {
    let r = crate::config::config().network.aoi_radius_yards;
    let r_sq = r * r;
    let recipients: usize = targets
        .par_iter()
        .filter_map(|t| {
            if t.map != anchor_map {
                return None;
            }
            if !within_aoi_sq(&t.position, &anchor, r_sq) {
                return None;
            }
            if Some(t.guid) == exclude_guid {
                return None;
            }
            t.try_queue_frame(Arc::clone(&frame));
            Some(1_usize)
        })
        .sum();
    (recipients, frame_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32) -> Vector3d {
        Vector3d { x, y, z: 0.0 }
    }

    #[test]
    fn within_aoi_self_is_in_range() {
        assert!(within_aoi(&v(100.0, 100.0), &v(100.0, 100.0)));
    }

    #[test]
    fn within_aoi_inside_radius() {
        // 100 yards apart on x-axis, well inside 200 yd radius.
        assert!(within_aoi(&v(0.0, 0.0), &v(100.0, 0.0)));
    }

    #[test]
    fn within_aoi_just_outside_radius() {
        // 201 yards on x-axis — outside the 200 yard circle.
        assert!(!within_aoi(&v(0.0, 0.0), &v(201.0, 0.0)));
    }

    #[test]
    fn within_aoi_ignores_z() {
        // 100 yd horizontal separation, huge z gap — still in range.
        let a = Vector3d {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let b = Vector3d {
            x: 100.0,
            y: 0.0,
            z: 1000.0,
        };
        assert!(within_aoi(&a, &b));
    }

    #[test]
    fn within_aoi_diagonal() {
        // 150x + 150y = ~212 yards Euclidean — just outside.
        assert!(!within_aoi(&v(0.0, 0.0), &v(150.0, 150.0)));
        // 100x + 100y = ~141 yards — inside.
        assert!(within_aoi(&v(0.0, 0.0), &v(100.0, 100.0)));
    }
}
