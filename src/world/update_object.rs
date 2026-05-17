use crate::world::aoi;
use crate::world::world::client::Client;
use slab::Slab;
use wow_world_base::vanilla::Map;
use wow_world_messages::vanilla::{
    Object, Object_UpdateType, Vector3d, SMSG_COMPRESSED_UPDATE_OBJECT, SMSG_UPDATE_OBJECT,
};

/// Compress when batching multiple objects OR when shipping any full create:
/// create masks are large (~400–1000 bytes) and compress well. Single tiny
/// partial-update (Values) messages stay plain — zlib overhead would inflate them.
fn should_compress(objects: &[Object]) -> bool {
    objects.len() >= 2
        || objects.iter().any(|o| {
            matches!(
                o.update_type,
                Object_UpdateType::CreateObject { .. } | Object_UpdateType::CreateObject2 { .. }
            )
        })
}

pub enum UpdateObject {
    Plain(SMSG_UPDATE_OBJECT),
    Compressed(SMSG_COMPRESSED_UPDATE_OBJECT),
}

impl UpdateObject {
    pub fn from_objects(objects: Vec<Object>) -> Option<Self> {
        if objects.is_empty() {
            return None;
        }
        Some(if should_compress(&objects) {
            Self::Compressed(SMSG_COMPRESSED_UPDATE_OBJECT {
                has_transport: 0,
                objects,
            })
        } else {
            Self::Plain(SMSG_UPDATE_OBJECT {
                has_transport: 0,
                objects,
            })
        })
    }

    pub async fn send(self, client: &mut Client) {
        match self {
            Self::Plain(m) => client.send_message(m).await,
            Self::Compressed(m) => client.send_message(m).await,
        }
    }

    /// Maximum `Object` entries packed into a single `SMSG_UPDATE_OBJECT`
    /// (or its compressed sibling). The wire protocol's size header is
    /// `u16` (max 65535 bytes). A worst-case CreateObject2 entry can run
    /// 400–700 bytes (creature mask is the biggest), so 75 keeps an
    /// uncompressed body comfortably under 64 KB even before zlib. The
    /// `as u16` truncation in `ServerMessage::server_size()` is silent;
    /// hitting it desyncs the per-stream ARC4 cipher and the client is
    /// dead-but-TCP-open from that point on.
    const CHUNK_SIZE: usize = 75;

    /// Split `objects` across as many `SMSG_UPDATE_OBJECT` packets as
    /// needed to stay under the wire-protocol u16 size cap, then send
    /// each. Used by the high-density spawn paths (`promote_logged_in`,
    /// `tick_aoi_transitions`' entered batch, `MSG_MOVE_WORLDPORT_ACK`)
    /// where the visible-object Vec can exceed several hundred entries.
    ///
    /// Each chunk is independently `should_compress`-classified so the
    /// compression decision stays the same as it would for a small batch.
    pub async fn send_chunked(objects: Vec<Object>, client: &mut Client) {
        let total = objects.len();
        if total == 0 {
            return;
        }
        if total <= Self::CHUNK_SIZE {
            if let Some(msg) = Self::from_objects(objects) {
                msg.send(client).await;
            }
            return;
        }
        let mut remaining = objects;
        while !remaining.is_empty() {
            let take = remaining.len().min(Self::CHUNK_SIZE);
            // `split_off(at)` keeps `[0..at]` in `remaining`, returns
            // `[at..]`. We want the inverse — head goes into the
            // packet, tail keeps iterating. Use `drain(..take)` so we
            // don't reallocate the source.
            let chunk: Vec<Object> = remaining.drain(..take).collect();
            if let Some(msg) = Self::from_objects(chunk) {
                msg.send(client).await;
            }
        }
    }

    pub async fn broadcast_within_aoi(
        self,
        anchor: Vector3d,
        anchor_map: Map,
        clients: &mut Slab<Client>,
    ) {
        match self {
            Self::Plain(m) => aoi::broadcast_within_aoi(m, anchor, anchor_map, clients).await,
            Self::Compressed(m) => aoi::broadcast_within_aoi(m, anchor, anchor_map, clients).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wow_world_messages::vanilla::{
        MovementBlock, MovementBlock_UpdateFlag, ObjectType, UpdateMask, UpdatePlayerBuilder,
    };
    use wow_world_messages::Guid;

    fn values_object() -> Object {
        Object {
            update_type: Object_UpdateType::Values {
                guid1: Guid::new(1),
                mask1: UpdateMask::Player(UpdatePlayerBuilder::new().finalize()),
            },
        }
    }

    fn create_object() -> Object {
        Object {
            update_type: Object_UpdateType::CreateObject {
                guid3: Guid::new(2),
                mask2: UpdateMask::Player(UpdatePlayerBuilder::new().finalize()),
                movement2: MovementBlock {
                    update_flag: MovementBlock_UpdateFlag::empty(),
                },
                object_type: ObjectType::Player,
            },
        }
    }

    #[test]
    fn empty_objects_returns_none() {
        assert!(UpdateObject::from_objects(Vec::new()).is_none());
    }

    #[test]
    fn single_values_stays_plain() {
        // Single partial update — zlib overhead would inflate it. Don't
        // compress.
        let r = UpdateObject::from_objects(vec![values_object()]).unwrap();
        assert!(matches!(r, UpdateObject::Plain(_)));
    }

    #[test]
    fn single_create_object_is_compressed() {
        // Create masks are large and compress well even alone.
        let r = UpdateObject::from_objects(vec![create_object()]).unwrap();
        assert!(matches!(r, UpdateObject::Compressed(_)));
    }

    #[test]
    fn two_values_objects_are_compressed() {
        // Batching multiple updates passes the >=2 threshold regardless of
        // their update_type.
        let r = UpdateObject::from_objects(vec![values_object(), values_object()]).unwrap();
        assert!(matches!(r, UpdateObject::Compressed(_)));
    }

    #[test]
    fn mixed_batch_with_create_is_compressed() {
        let r = UpdateObject::from_objects(vec![values_object(), create_object()]).unwrap();
        assert!(matches!(r, UpdateObject::Compressed(_)));
    }
}
