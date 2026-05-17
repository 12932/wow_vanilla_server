use crate::world::world::client::Client;
use crate::world::world_opcode_handler::{write_message_test, write_server_test};
use slab::Slab;
use std::sync::Arc;
use wow_world_base::vanilla::Map;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::{ServerMessage, Vector3d};

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

    let mut body = Vec::with_capacity(msg.size_without_header() as usize);
    if let Err(e) = msg.write_into_vec(&mut body) {
        tracing::warn!("broadcast_within_aoi: serialize failed: {e}");
        return;
    }
    let opcode = M::OPCODE as u16;
    let body = body.as_slice();

    for (_, c) in clients.iter_mut() {
        if c.character().map == anchor_map && within_aoi(&c.character().info.position, &anchor) {
            c.send_raw(opcode, body).await;
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
    clients: &Slab<Client>,
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

    // Hoist the AOI radius out of the per-iter loop: `within_aoi`
    // otherwise reads `config()` (a `OnceCell`-backed static) on every
    // call. At 1400-bot density that's ~322k `OnceCell::get`s per tick.
    // Squared up here so the inner check is one multiply less per iter.
    let r = crate::config::config().network.aoi_radius_yards;
    let r_sq = r * r;

    let mut recipients = 0_usize;
    for (_, c) in clients.iter() {
        // Cache `c.character()` once per iter — the field accessor is
        // trivially `&self.character` but the compiler can't always
        // prove it across the three reads below, so spell it once.
        let ch = c.character();
        if ch.map != anchor_map {
            continue;
        }
        if !within_aoi_sq(&ch.info.position, &anchor, r_sq) {
            continue;
        }
        if Some(ch.guid) == exclude_guid {
            // Broadcaster themselves — 1-in-N rare; keep this check
            // last so the common path skips it via the early-continues
            // above on the cheaper map/AOI rejections.
            continue;
        }
        c.try_queue_frame(Arc::clone(&frame));
        recipients += 1;
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
