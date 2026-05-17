use crate::world::database::WorldDatabase;
use crate::world::world_opcode_handler::item::Item;
use wow_items::vanilla::lookup_item;
use wow_world_base::vanilla::{Guid, ItemSlot, StarterItem};
use wow_world_messages::vanilla::CharacterGear;

const AMOUNT_OF_SLOTS: usize = 113;

#[derive(Debug, Clone)]
pub struct Inventory {
    pub slots: [Option<Item>; AMOUNT_OF_SLOTS],
}

impl Inventory {
    pub const SLOT_COUNT: usize = AMOUNT_OF_SLOTS;
}

impl Inventory {
    pub fn new(starter_items: &[StarterItem], db: &mut WorldDatabase) -> Self {
        let slots = [(); AMOUNT_OF_SLOTS].map(|()| None);
        let mut s = Self { slots };

        for item in starter_items {
            let Some(static_item) = lookup_item(item.item) else {
                tracing::warn!(
                    "starter item {} (slot {:?}) not found in item DB; skipping",
                    item.item,
                    item.ty,
                );
                continue;
            };
            let i = Item::new(static_item, Guid::zero(), item.amount, db);
            s.set(item.ty, i);
        }

        s
    }

    pub fn swap(&mut self, source: ItemSlot, destination: ItemSlot) {
        let source_temp = self.take(source);
        let dest_temp = self.take(destination);

        *self.get_mut(source) = dest_temp;
        *self.get_mut(destination) = source_temp;
    }

    pub fn insert_into_first_slot(&mut self, item: Item) -> Option<ItemSlot> {
        let bag_start: usize = ItemSlot::Inventory0.as_int().into();
        let bag_end: usize = ItemSlot::Inventory15.as_int().into();
        let slots = &mut self.slots[bag_start..=bag_end];

        for (i, slot) in slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(item);

                let slot = bag_start + i;
                return Some(ItemSlot::try_from(slot as u8).expect("inventory slot index in bounds [0, AMOUNT_OF_SLOTS)"));
            }
        }

        None
    }

    pub fn all_slots(&self) -> [(Option<&Item>, ItemSlot); AMOUNT_OF_SLOTS] {
        let mut slots = [(); AMOUNT_OF_SLOTS].map(|()| (None, ItemSlot::default()));

        for (i, slot) in self.slots.iter().enumerate() {
            slots[i] = (slot.as_ref(), ItemSlot::try_from(i as u8).expect("inventory slot index in bounds [0, AMOUNT_OF_SLOTS)"));
        }

        slots
    }

    pub fn equipment(&self) -> [(Option<&Item>, ItemSlot); 19] {
        let mut slots = [(); 19].map(|()| (None, ItemSlot::default()));

        let inventory_start: usize = ItemSlot::Head.as_int().into();
        let inventory_end: usize = ItemSlot::Tabard.as_int().into();

        for (i, slot) in self.slots[inventory_start..=inventory_end]
            .iter()
            .enumerate()
        {
            slots[i] = (slot.as_ref(), ItemSlot::try_from(i as u8).expect("inventory slot index in bounds [0, AMOUNT_OF_SLOTS)"));
        }

        slots
    }

    pub fn to_character_gear(&self) -> [CharacterGear; 19] {
        let mut gear = [CharacterGear::default(); 19];

        for (i, (item, _)) in self.equipment().iter().enumerate() {
            if let Some(item) = item {
                let g = CharacterGear {
                    equipment_display_id: item.item.display_id(),
                    inventory_type: item.item.inventory_type(),
                };
                gear[i] = g;
            } else {
                gear[i] = CharacterGear {
                    equipment_display_id: 0,
                    inventory_type: Default::default(),
                };
            }
        }

        gear
    }

    pub fn set(&mut self, item_slot: ItemSlot, item: Item) {
        *self.get_mut(item_slot) = Some(item);
    }

    pub fn clear(&mut self, item_slot: ItemSlot) {
        *self.get_mut(item_slot) = None;
    }

    pub fn take(&mut self, item_slot: ItemSlot) -> Option<Item> {
        self.get_mut(item_slot).take()
    }

    pub fn get(&self, item_slot: ItemSlot) -> Option<&Item> {
        self.inner_get(item_slot).as_ref()
    }

    fn inner_get(&self, item_slot: ItemSlot) -> &Option<Item> {
        &self.slots[item_slot.as_int() as usize]
    }

    fn get_mut(&mut self, item_slot: ItemSlot) -> &mut Option<Item> {
        &mut self.slots[item_slot.as_int() as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wow_items::vanilla::all_items;

    fn empty_inventory() -> Inventory {
        Inventory {
            slots: [(); AMOUNT_OF_SLOTS].map(|()| None),
        }
    }

    // Pick two distinct real items from the wow_items data, just to have
    // concrete `Item` instances. The specific entries don't matter — these
    // tests exercise slot mechanics, not item semantics.
    fn two_items() -> (Item, Item) {
        let items = all_items();
        assert!(items.len() >= 2, "wow_items::vanilla has at least two items");
        (
            Item {
                item: &items[0],
                guid: Guid::new(1),
                amount: 1,
                creator: Guid::zero(),
            },
            Item {
                item: &items[1],
                guid: Guid::new(2),
                amount: 1,
                creator: Guid::zero(),
            },
        )
    }

    #[test]
    fn swap_exchanges_two_occupied_slots() {
        let mut inv = empty_inventory();
        let (a, b) = two_items();
        inv.set(ItemSlot::Inventory0, a);
        inv.set(ItemSlot::Inventory5, b);

        inv.swap(ItemSlot::Inventory0, ItemSlot::Inventory5);

        assert_eq!(inv.get(ItemSlot::Inventory0).map(|i| i.guid), Some(b.guid));
        assert_eq!(inv.get(ItemSlot::Inventory5).map(|i| i.guid), Some(a.guid));
    }

    #[test]
    fn swap_with_empty_destination_moves_item() {
        let mut inv = empty_inventory();
        let (a, _) = two_items();
        inv.set(ItemSlot::Inventory0, a);

        inv.swap(ItemSlot::Inventory0, ItemSlot::Inventory7);

        assert!(inv.get(ItemSlot::Inventory0).is_none());
        assert_eq!(inv.get(ItemSlot::Inventory7).map(|i| i.guid), Some(a.guid));
    }

    #[test]
    fn swap_same_slot_is_noop() {
        let mut inv = empty_inventory();
        let (a, _) = two_items();
        inv.set(ItemSlot::Inventory3, a);

        inv.swap(ItemSlot::Inventory3, ItemSlot::Inventory3);

        assert_eq!(inv.get(ItemSlot::Inventory3).map(|i| i.guid), Some(a.guid));
    }

    #[test]
    fn insert_into_first_slot_picks_inventory0_when_empty() {
        let mut inv = empty_inventory();
        let (a, _) = two_items();
        let slot = inv.insert_into_first_slot(a);
        assert_eq!(slot, Some(ItemSlot::Inventory0));
        assert_eq!(inv.get(ItemSlot::Inventory0).map(|i| i.guid), Some(a.guid));
    }

    #[test]
    fn insert_into_first_slot_skips_occupied_slots() {
        let mut inv = empty_inventory();
        let (a, b) = two_items();
        inv.set(ItemSlot::Inventory0, a);
        inv.set(ItemSlot::Inventory1, a);

        let slot = inv.insert_into_first_slot(b);
        assert_eq!(slot, Some(ItemSlot::Inventory2));
        assert_eq!(inv.get(ItemSlot::Inventory2).map(|i| i.guid), Some(b.guid));
    }

    #[test]
    fn insert_into_first_slot_returns_none_when_bag_is_full() {
        let mut inv = empty_inventory();
        let (a, b) = two_items();
        // Bag is Inventory0..=Inventory15 — 16 slots.
        for s in [
            ItemSlot::Inventory0, ItemSlot::Inventory1, ItemSlot::Inventory2,
            ItemSlot::Inventory3, ItemSlot::Inventory4, ItemSlot::Inventory5,
            ItemSlot::Inventory6, ItemSlot::Inventory7, ItemSlot::Inventory8,
            ItemSlot::Inventory9, ItemSlot::Inventory10, ItemSlot::Inventory11,
            ItemSlot::Inventory12, ItemSlot::Inventory13, ItemSlot::Inventory14,
            ItemSlot::Inventory15,
        ] {
            inv.set(s, a);
        }
        assert_eq!(inv.insert_into_first_slot(b), None);
    }

    #[test]
    fn insert_into_first_slot_uses_inventory15_as_last_slot() {
        // Boundary: with everything occupied except Inventory15, the function
        // must still place into the inclusive end of the bag range. Catches
        // an off-by-one if the slice ever became `bag_start..bag_end`
        // (exclusive) by accident.
        let mut inv = empty_inventory();
        let (a, b) = two_items();
        for s in [
            ItemSlot::Inventory0, ItemSlot::Inventory1, ItemSlot::Inventory2,
            ItemSlot::Inventory3, ItemSlot::Inventory4, ItemSlot::Inventory5,
            ItemSlot::Inventory6, ItemSlot::Inventory7, ItemSlot::Inventory8,
            ItemSlot::Inventory9, ItemSlot::Inventory10, ItemSlot::Inventory11,
            ItemSlot::Inventory12, ItemSlot::Inventory13, ItemSlot::Inventory14,
        ] {
            inv.set(s, a);
        }
        assert_eq!(inv.insert_into_first_slot(b), Some(ItemSlot::Inventory15));
        assert_eq!(inv.get(ItemSlot::Inventory15).map(|i| i.guid), Some(b.guid));
    }
}
