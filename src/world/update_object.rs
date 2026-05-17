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
