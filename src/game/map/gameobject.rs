//! A game object in a map: a door, a chest, a herb, a mailbox, a chair.

use std::sync::Arc;

use ahash::AHashSet;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::items::Body;
use crate::game::data::{GameObjectSpawn, GameObjectTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::grid::{CellKey, cell_of};
use crate::game::update::{Create, Placement};

pub const SMSG_GAMEOBJECT_QUERY_RESPONSE: u16 = 0x05F;

#[derive(Debug)]
pub struct GameObject {
    pub guid: Guid,
    pub entry: u32,
    pub spawn_guid: Option<u32>,
    pub kind: u8,
    pub fields: Fields,
    pub position: Vector3d,
    pub orientation: f32,
    pub observers: AHashSet<Guid>,
    pub cell: CellKey,
    pub loot: Option<crate::game::map::loot::Loot>,
}

impl GameObject {
    pub fn from_spawn(t: &GameObjectTemplate, s: &GameObjectSpawn) -> Self {
        let g = guid::game_object(t.entry, s.guid);
        let mut f = Fields::new(TypeId::GameObject, g, t.entry);
        f.set_f32(
            OBJECT_FIELD_SCALE_X,
            if t.size > 0.0 { t.size } else { 1.0 },
        );
        f.set_u32(GAMEOBJECT_DISPLAYID, t.display_id);
        f.set_u32(GAMEOBJECT_FLAGS, t.flags);
        for (i, r) in s.rotation.iter().enumerate() {
            f.set_f32(GAMEOBJECT_ROTATION + i as u16, *r);
        }
        f.set_u32(GAMEOBJECT_STATE, u32::from(s.state));
        f.set_f32(GAMEOBJECT_POS_X, s.x);
        f.set_f32(GAMEOBJECT_POS_Y, s.y);
        f.set_f32(GAMEOBJECT_POS_Z, s.z);
        f.set_f32(GAMEOBJECT_FACING, s.orientation);
        f.set_u32(GAMEOBJECT_FACTION, t.faction);
        f.set_u32(GAMEOBJECT_TYPE_ID, u32::from(t.kind));
        f.set_u32(GAMEOBJECT_ANIMPROGRESS, u32::from(s.anim_progress));
        if t.kind == 11 {
            f.set_u32(GAMEOBJECT_LEVEL, crate::game::uptime_ms());
        }
        let position = Vector3d {
            x: s.x,
            y: s.y,
            z: s.z,
        };
        Self {
            guid: g,
            entry: t.entry,
            spawn_guid: Some(s.guid),
            kind: t.kind,
            fields: f,
            position,
            orientation: s.orientation,
            observers: AHashSet::new(),
            cell: cell_of(s.x, s.y),
            loot: None,
        }
    }

    pub fn create(&self) -> Create<'_> {
        Create {
            guid: self.guid,
            type_id: TypeId::GameObject,
            // Elevators and boats run on the server's clock.
            placement: if matches!(self.kind, 11 | 15) {
                Placement::Transport {
                    position: self.position,
                    orientation: self.orientation,
                    time: crate::game::uptime_ms(),
                }
            } else {
                Placement::Position {
                    position: self.position,
                    orientation: self.orientation,
                }
            },
            fields: &self.fields,
            is_self: false,
            melee_target: None,
        }
    }
}

/// `SMSG_GAMEOBJECT_QUERY_RESPONSE`, as 1.12 lays it out: the name, four
/// empty names, then the template's 24 data words.
pub fn query_response(entry: u32, t: Option<&GameObjectTemplate>) -> Arc<[u8]> {
    let mut b = Body::default();
    match t {
        Some(t) => {
            b.u32(entry)
                .u32(u32::from(t.kind))
                .u32(t.display_id)
                .cstr(&t.name)
                .u8(0)
                .u8(0)
                .u8(0)
                .u8(0);
            for d in t.data {
                b.u32(d);
            }
        }
        None => {
            b.u32(entry | 0x8000_0000);
        }
    }
    b.frame(SMSG_GAMEOBJECT_QUERY_RESPONSE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::fields::Visibility;
    use crate::game::update::{Batch, create_block};
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;

    #[test]
    fn a_game_object_create_parses() {
        let t = GameObjectTemplate {
            entry: 50,
            kind: 19,
            display_id: 3851,
            name: "Mailbox".into(),
            size: 1.0,
            ..Default::default()
        };
        let s = GameObjectSpawn {
            guid: 7,
            entry: 50,
            rotation: [0.0, 0.0, 0.0, 1.0],
            state: 1,
            anim_progress: 100,
            ..Default::default()
        };
        let g = GameObject::from_spawn(&t, &s);
        let mut b = Batch::new();
        b.push(&create_block(&g.create(), Visibility::Public));
        let frame = &b.finish()[0];
        let m = ServerOpcodeMessage::read_unencrypted(&frame[..]);
        assert!(m.is_ok(), "{m:?}");
    }
}
