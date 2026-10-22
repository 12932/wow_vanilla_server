//! Dynamic objects: what a client draws for a spell on the ground
//! (Blizzard, Rain of Fire, Consecration, a trap's area) and the point a
//! far sight looks from. Each is an object of its own in the map's grid,
//! made as vmangos's `DynamicObject::Create` (and Thistle Tea's
//! `DynamicObject.build`) makes it, and gone when its spell is.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::fields::*;
use crate::game::guid::{self, HighGuid};
use crate::game::map::Map;
use crate::game::map::grid::{CellKey, cell_of};
use crate::game::update::{Create, Placement};

/// `DynamicObjectType`.
pub const KIND_PORTAL: u8 = 0;
pub const KIND_AREA_SPELL: u8 = 1;
pub const KIND_FARSIGHT: u8 = 2;

#[derive(Debug, Clone)]
pub struct DynamicObject {
    pub guid: Guid,
    pub caster: Guid,
    pub spell: u32,
    pub kind: u8,
    pub position: Vector3d,
    pub orientation: f32,
    pub radius: f32,
    pub fields: Fields,
    pub observers: crate::game::map::observers::Observers,
    pub cell: CellKey,
    /// When it goes; `None` while its spell (a channel) holds it.
    pub ends: Option<Instant>,
}

impl DynamicObject {
    pub fn create(&self) -> Create<'_> {
        Create {
            guid: self.guid,
            type_id: TypeId::DynamicObject,
            placement: Placement::Position {
                position: self.position,
                orientation: self.orientation,
            },
            fields: &self.fields,
            is_self: false,
            melee_target: None,
        }
    }
}

/// Put a dynamic object of `spell` from `caster` at `at`, for `ms` (or
/// until taken away).
pub fn spawn(
    map: &mut Map,
    caster: Guid,
    spell: u32,
    kind: u8,
    at: Vector3d,
    radius: f32,
    ms: Option<u32>,
) -> Guid {
    map.spells.next_dynamic = map.spells.next_dynamic.wrapping_add(1).max(1);
    let g = guid::make(HighGuid::DynamicObject, 0, map.spells.next_dynamic);
    let orientation = map.unit_view(caster).map(|u| u.orientation).unwrap_or(0.0);
    let mut f = Fields::new(TypeId::DynamicObject, g, spell);
    f.set_f32(OBJECT_FIELD_SCALE_X, 1.0);
    f.set_guid(DYNAMICOBJECT_CASTER, caster);
    f.set_byte(DYNAMICOBJECT_BYTES, 0, kind);
    f.set_u32(DYNAMICOBJECT_SPELLID, spell);
    f.set_f32(DYNAMICOBJECT_RADIUS, radius);
    f.set_f32(DYNAMICOBJECT_POS_X, at.x);
    f.set_f32(DYNAMICOBJECT_POS_X + 1, at.y);
    f.set_f32(DYNAMICOBJECT_POS_X + 2, at.z);
    f.set_f32(DYNAMICOBJECT_FACING, orientation);
    let cell = cell_of(at.x, at.y);
    let now = map.now;
    let d = DynamicObject {
        guid: g,
        caster,
        spell,
        kind,
        position: at,
        orientation,
        radius,
        fields: f,
        observers: Default::default(),
        cell,
        ends: ms.map(|ms| now + Duration::from_millis(u64::from(ms))),
    };
    map.grid.insert(g, cell);
    map.spells.dynamic.insert(g, d);
    map.visibility_dirty.insert(g);
    map.hold_near(g, at);
    g
}

/// Take a dynamic object away: every client that knew it forgets it.
pub fn remove(map: &mut Map, g: Guid) {
    let Some(mut d) = map.spells.dynamic.remove(&g) else {
        return;
    };
    map.grid.remove(g, d.cell);
    // Those who see it watch it fade, as mangos-classic's
    // `DynamicObject::Delete` and Thistle Tea's show it go.
    if let Some(frame) =
        crate::game::conn::encode(&wow_world_messages::vanilla::SMSG_GAMEOBJECT_DESPAWN_ANIM {
            guid: g,
        })
    {
        for o in &d.observers {
            if let Some(p) = map.players.get(o) {
                p.outbound.send_frame(frame.clone());
            }
        }
    }
    for o in d.observers.drain() {
        if let Some(p) = map.players.get_mut(&o) {
            p.known.remove(&g);
            map.forget.entry(o).or_default().push(g);
        }
    }
    // A far sight's viewer looks from their own eyes again.
    if d.kind == KIND_FARSIGHT {
        super::sight::release(map, d.caster, g);
    }
}

/// Take away a caster's dynamic objects of `spell`.
pub fn remove_spell(map: &mut Map, caster: Guid, spell: u32) {
    let gone: Vec<Guid> = map
        .spells
        .dynamic
        .values()
        .filter(|d| d.caster == caster && d.spell == spell)
        .map(|d| d.guid)
        .collect();
    for g in gone {
        remove(map, g);
    }
}

/// Dynamic objects whose time is up go, and those of casters no longer in
/// the map.
pub fn update(map: &mut Map) {
    let now = map.now;
    let gone: Vec<Guid> = map
        .spells
        .dynamic
        .values()
        .filter(|d| d.ends.is_some_and(|t| t <= now) || map.position_of(d.caster).is_none())
        .map(|d| d.guid)
        .collect();
    for g in gone {
        remove(map, g);
    }
}

pub fn is_dynamic_object(g: Guid) -> bool {
    guid::high(g) == Some(HighGuid::DynamicObject)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::fields::Visibility;
    use crate::game::update::{Batch, create_block};
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    use wow_world_messages::vanilla::{Object_UpdateType, UpdateMask};

    #[test]
    fn a_dynamic_objects_create_block_parses() {
        let g = guid::make(HighGuid::DynamicObject, 0, 7);
        let caster = guid::player(3);
        let mut f = Fields::new(TypeId::DynamicObject, g, 10);
        f.set_guid(DYNAMICOBJECT_CASTER, caster);
        f.set_byte(DYNAMICOBJECT_BYTES, 0, KIND_AREA_SPELL);
        f.set_u32(DYNAMICOBJECT_SPELLID, 10);
        f.set_f32(DYNAMICOBJECT_RADIUS, 8.0);
        let at = Vector3d {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        };
        let d = DynamicObject {
            guid: g,
            caster,
            spell: 10,
            kind: KIND_AREA_SPELL,
            position: at,
            orientation: 0.5,
            radius: 8.0,
            fields: f,
            observers: Default::default(),
            cell: cell_of(at.x, at.y),
            ends: None,
        };
        let mut batch = Batch::new();
        batch.push(&create_block(&d.create(), Visibility::Public));
        let frame = &batch.finish()[0];
        let Ok(ServerOpcodeMessage::SMSG_UPDATE_OBJECT(m)) =
            ServerOpcodeMessage::read_unencrypted(&frame[..])
        else {
            panic!("an update object");
        };
        let Object_UpdateType::CreateObject { guid3, mask2, .. } = &m.objects[0].update_type else {
            panic!("a create: {:?}", m.objects[0].update_type);
        };
        assert_eq!(*guid3, g);
        let UpdateMask::DynamicObject(o) = mask2 else {
            panic!("a dynamic object's mask");
        };
        assert_eq!(o.dynamicobject_spellid(), Some(10));
    }
}
