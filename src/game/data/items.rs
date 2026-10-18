//! Item templates, from vmangos's `item_template`, and their query
//! response, encoded by hand: wow_messages types its fields as enums that
//! don't name every value the database has.

use std::sync::Arc;

use crate::game::update::frame;

pub const SMSG_ITEM_QUERY_SINGLE_RESPONSE: u16 = 0x058;

#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct ItemSpell {
    pub id: u32,
    pub trigger: u32,
    pub charges: i32,
    pub ppm_rate: f32,
    pub cooldown: i32,
    pub category: u32,
    pub category_cooldown: i32,
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct ItemTemplate {
    pub entry: u32,
    pub class: u32,
    pub subclass: u32,
    pub name: String,
    pub description: String,
    pub display_id: u32,
    pub quality: u32,
    pub flags: u32,
    pub buy_count: u32,
    pub buy_price: u32,
    pub sell_price: u32,
    pub inventory_type: u32,
    pub allowable_class: i32,
    pub allowable_race: i32,
    pub item_level: u32,
    pub required_level: u32,
    pub required_skill: u32,
    pub required_skill_rank: u32,
    pub required_spell: u32,
    pub required_honor_rank: u32,
    pub required_city_rank: u32,
    pub required_reputation_faction: u32,
    pub required_reputation_rank: u32,
    pub max_count: u32,
    pub stackable: u32,
    pub container_slots: u32,
    pub stats: [(u32, i32); 10],
    pub delay: u32,
    pub range_mod: f32,
    pub ammo_type: u32,
    pub damages: [(f32, f32, u32); 5],
    pub block: u32,
    pub armor: i32,
    pub resistances: [i32; 6],
    pub spells: [ItemSpell; 5],
    pub bonding: u32,
    pub page_text: u32,
    pub page_language: u32,
    pub page_material: u32,
    pub start_quest: u32,
    pub lock_id: u32,
    pub material: i32,
    pub sheath: u32,
    pub random_property: u32,
    pub set_id: u32,
    pub max_durability: u32,
    pub area_bound: u32,
    pub map_bound: u32,
    pub duration: u32,
    pub bag_family: u32,
    pub disenchant_id: u32,
    pub food_type: u32,
    pub min_money_loot: u32,
    pub max_money_loot: u32,
    pub extra_flags: u32,
}

/// A little-endian packet body.
#[derive(Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct Body(pub Vec<u8>);

impl Body {
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn f32(&mut self, v: f32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    /// A NUL-terminated string.
    pub fn cstr(&mut self, s: &str) -> &mut Self {
        self.0.extend(s.bytes().filter(|b| *b != 0));
        self.0.push(0);
        self
    }
    pub fn frame(&self, opcode: u16) -> Arc<[u8]> {
        frame(opcode, &self.0)
    }
}

impl ItemTemplate {
    /// `SMSG_ITEM_QUERY_SINGLE_RESPONSE` for this item.
    pub fn query_response(&self) -> Arc<[u8]> {
        let mut b = Body::default();
        b.u32(self.entry)
            .u32(self.class)
            .u32(self.subclass)
            .cstr(&self.name)
            .u8(0)
            .u8(0)
            .u8(0)
            .u32(self.display_id)
            .u32(self.quality)
            .u32(self.flags)
            .u32(self.buy_price)
            .u32(self.sell_price)
            .u32(self.inventory_type)
            .i32(self.allowable_class)
            .i32(self.allowable_race)
            .u32(self.item_level)
            .u32(self.required_level)
            .u32(self.required_skill)
            .u32(self.required_skill_rank)
            .u32(self.required_spell)
            .u32(self.required_honor_rank)
            .u32(self.required_city_rank)
            .u32(self.required_reputation_faction)
            .u32(self.required_reputation_rank)
            .u32(self.max_count)
            .u32(self.stackable)
            .u32(self.container_slots);
        for (kind, value) in self.stats {
            b.u32(kind).i32(value);
        }
        for (min, max, school) in self.damages {
            b.f32(min).f32(max).u32(school);
        }
        b.i32(self.armor);
        for r in self.resistances {
            b.i32(r);
        }
        b.u32(self.delay).u32(self.ammo_type).f32(self.range_mod);
        for s in self.spells {
            b.u32(s.id)
                .u32(s.trigger)
                .i32(s.charges)
                .i32(s.cooldown)
                .u32(s.category)
                .i32(s.category_cooldown);
        }
        b.u32(self.bonding)
            .cstr(&self.description)
            .u32(self.page_text)
            .u32(self.page_language)
            .u32(self.page_material)
            .u32(self.start_quest)
            .u32(self.lock_id)
            .i32(self.material)
            .u32(self.sheath)
            .u32(self.random_property)
            .u32(self.block)
            .u32(self.set_id)
            .u32(self.max_durability)
            .u32(self.area_bound)
            .u32(self.map_bound)
            .u32(self.bag_family);
        b.frame(SMSG_ITEM_QUERY_SINGLE_RESPONSE)
    }

    /// The equipment slot an item of this inventory type goes in.
    pub fn equipment_slot(&self) -> Option<u8> {
        Some(match self.inventory_type {
            1 => 0,                  // head
            2 => 1,                  // neck
            3 => 2,                  // shoulders
            4 => 3,                  // shirt
            5 | 20 => 4,             // chest, robe
            6 => 5,                  // waist
            7 => 6,                  // legs
            8 => 7,                  // feet
            9 => 8,                  // wrists
            10 => 9,                 // hands
            11 => 10,                // finger
            12 => 12,                // trinket
            13 | 17 | 21 => 15,      // one-hand, two-hand, main hand
            14 | 22 | 23 => 16,      // shield, off hand, held in off hand
            15 | 25 | 26 | 28 => 17, // ranged, thrown, ranged right, relic
            16 => 14,                // back
            19 => 18,                // tabard
            18 => 19,                // bag
            _ => return None,
        })
    }
}

/// The reply for an item there is no template for.
pub fn unknown_item_response(entry: u32) -> Arc<[u8]> {
    let mut b = Body::default();
    b.u32(entry | 0x8000_0000);
    b.frame(SMSG_ITEM_QUERY_SINGLE_RESPONSE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;

    #[test]
    fn a_hand_encoded_item_response_parses_in_wow_messages() {
        let t = ItemTemplate {
            entry: 25,
            class: 2,
            subclass: 7,
            name: "Worn Shortsword".into(),
            display_id: 1542,
            quality: 1,
            inventory_type: 21,
            allowable_class: -1,
            allowable_race: -1,
            item_level: 2,
            required_level: 1,
            stackable: 1,
            delay: 1900,
            damages: [
                (1.0, 3.0, 0),
                (0.0, 0.0, 0),
                (0.0, 0.0, 0),
                (0.0, 0.0, 0),
                (0.0, 0.0, 0),
            ],
            bonding: 0,
            material: 1,
            sheath: 3,
            max_durability: 20,
            ..Default::default()
        };
        let frame = t.query_response();
        let ServerOpcodeMessage::SMSG_ITEM_QUERY_SINGLE_RESPONSE(m) =
            ServerOpcodeMessage::read_unencrypted(&frame[..]).unwrap()
        else {
            panic!("not an item response");
        };
        let found = m.found.unwrap();
        assert_eq!(m.item, 25);
        assert_eq!(found.name1, "Worn Shortsword");
        assert_eq!(found.display_id, 1542);
        assert_eq!(found.max_durability, 20);
        let unknown = unknown_item_response(7);
        let ServerOpcodeMessage::SMSG_ITEM_QUERY_SINGLE_RESPONSE(m) =
            ServerOpcodeMessage::read_unencrypted(&unknown[..]).unwrap()
        else {
            panic!("not an item response");
        };
        assert!(m.found.is_none());
    }
}
