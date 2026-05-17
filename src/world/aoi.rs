use crate::world::world::client::Client;
use crate::world::world_opcode_handler::{write_message_test, write_server_test};
use slab::Slab;
use wow_world_base::vanilla::Map;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::{ServerMessage, Vector3d};

pub const AOI_RADIUS_YARDS: f32 = 400.0;

pub fn within_aoi(observer: &Vector3d, anchor: &Vector3d) -> bool {
    let dx = observer.x - anchor.x;
    let dy = observer.y - anchor.y;
    dx * dx + dy * dy <= AOI_RADIUS_YARDS * AOI_RADIUS_YARDS
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
/// `[size_BE u16][opcode_LE u16][body]` *once*, then clones that buffer
/// into each recipient's outbound channel. The writer task re-encrypts the
/// 4-byte header per recipient (encryption is stateful per stream).
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
    clients: &mut Slab<Client>,
) -> (usize, usize) {
    write_server_test(msg);

    // Serialize once into the wire framing the writer task expects.
    let mut frame = Vec::new();
    if let Err(e) = msg.write_unencrypted_server(&mut frame) {
        tracing::warn!("broadcast_opcode_within_aoi: serialize failed: {e}");
        return (0, 0);
    }
    let frame_bytes = frame.len();

    let mut recipients = 0_usize;
    for (_, c) in clients.iter_mut() {
        if Some(c.character().guid) == exclude_guid {
            continue;
        }
        if c.character().map == anchor_map && within_aoi(&c.character().info.position, &anchor) {
            c.try_queue_frame(frame.clone());
            recipients += 1;
        }
    }
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
        // 100 yards apart on x-axis, well inside 400 yd radius.
        assert!(within_aoi(&v(0.0, 0.0), &v(100.0, 0.0)));
    }

    #[test]
    fn within_aoi_just_outside_radius() {
        // 401 yards on x-axis — outside the 400 yard circle.
        assert!(!within_aoi(&v(0.0, 0.0), &v(401.0, 0.0)));
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
        // 300x + 300y = ~424 yards Euclidean — just outside.
        assert!(!within_aoi(&v(0.0, 0.0), &v(300.0, 300.0)));
        // 200x + 200y = ~283 yards — inside.
        assert!(within_aoi(&v(0.0, 0.0), &v(200.0, 200.0)));
    }
}
