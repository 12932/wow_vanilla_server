use crate::world::database::WorldDatabase;
use crate::world::world::client::Client;
use wow_world_base::vanilla::{
    BagFamily, NewItemChatAlert, NewItemCreationType, NewItemSource, ObjectType,
};
use wow_world_messages::vanilla::{
    MovementBlock, MovementBlock_UpdateFlag, Object, Object_UpdateType, UpdateItemBuilder,
    UpdatePlayerBuilder, SMSG_ITEM_PUSH_RESULT, SMSG_UPDATE_OBJECT,
};
use wow_world_messages::Guid;

#[derive(Debug, Clone, Copy)]
pub struct Item {
    pub item: &'static wow_world_base::vanilla::Item,
    pub guid: Guid,
    pub amount: u8,
    pub creator: Guid,
}

impl Item {
    pub fn new(
        item: &'static wow_world_base::vanilla::Item,
        creator: Guid,
        amount: u8,
        db: &mut WorldDatabase,
    ) -> Self {
        Self {
            item,
            guid: db.new_guid().into(),
            amount,
            creator,
        }
    }

    pub fn to_create_item_object(self, item_owner: Guid) -> Object {
        let object_type = match self.item.bag_family() {
            BagFamily::None => ObjectType::Item,
            _ => ObjectType::Container,
        };

        Object {
            update_type: Object_UpdateType::CreateObject {
                guid3: self.guid,
                mask2: UpdateItemBuilder::new()
                    .set_object_guid(self.guid)
                    .set_object_entry(self.item.entry() as i32)
                    .set_object_scale_x(1.0)
                    .set_item_owner(item_owner)
                    .set_item_contained(item_owner)
                    .set_item_stack_count(self.amount as i32)
                    .set_item_durability(self.item.max_durability())
                    .set_item_maxdurability(self.item.max_durability())
                    .set_item_creator(self.creator)
                    .set_item_stack_count(self.amount as i32)
                    .finalize()
                    .into(),
                movement2: MovementBlock {
                    update_flag: MovementBlock_UpdateFlag::empty(),
                },
                object_type,
            },
        }
    }
}

pub(crate) async fn award_item(
    item: Item,
    client: &mut Client,
    clients: &mut slab::Slab<Client>,
) {
    let item_slot = client
        .character_mut()
        .inventory
        .insert_into_first_slot(item);
    let Some(item_slot) = item_slot else {
        client
            .send_system_message("Unable to add item. No free slots available.")
            .await;
        return;
    };

    client
        .send_opcode(
            &SMSG_UPDATE_OBJECT {
                has_transport: 0,
                objects: vec![
                    item.to_create_item_object(client.character().guid),
                    Object {
                        update_type: Object_UpdateType::Values {
                            guid1: client.character().guid,
                            mask1: UpdatePlayerBuilder::new()
                                .set_player_field_inv(item_slot, item.guid)
                                .finalize()
                                .into(),
                        },
                    },
                ],
            }
            .into(),
        )
        .await;

    let item_push_result = SMSG_ITEM_PUSH_RESULT {
        guid: client.character().guid,
        source: NewItemSource::Looted,
        creation_type: NewItemCreationType::Created,
        alert_chat: NewItemChatAlert::Show,
        bag_slot: 0xff,
        item_slot: item_slot.as_int() as u32,
        item: item.item.entry(),
        item_suffix_factor: 0,
        item_random_property_id: 0,
        item_count: item.amount.into(),
    };

    client.send_opcode(&item_push_result.into()).await;

    for (_, c) in clients.iter_mut() {
        c.send_opcode(&item_push_result.into()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wow_items::vanilla::all_items;

    fn first_item_where<F>(pred: F) -> Option<&'static wow_world_base::vanilla::Item>
    where
        F: Fn(&wow_world_base::vanilla::Item) -> bool,
    {
        all_items().iter().find(|i| pred(i))
    }

    fn build_item(static_item: &'static wow_world_base::vanilla::Item) -> Item {
        Item {
            item: static_item,
            guid: Guid::new(0x4000_0000_0000_0001),
            amount: 1,
            creator: Guid::zero(),
        }
    }

    fn assert_create(o: &Object) -> &Object_UpdateType {
        // CreateObject is the only variant `to_create_item_object` produces;
        // assert that and yield the inner type for further checks.
        match &o.update_type {
            Object_UpdateType::CreateObject { .. } => &o.update_type,
            _ => panic!("expected Object_UpdateType::CreateObject, got {:?}", o.update_type),
        }
    }

    #[test]
    fn non_bag_item_maps_to_object_type_item() {
        let static_item = first_item_where(|i| matches!(i.bag_family(), BagFamily::None))
            .expect("wow_items::vanilla contains a non-bag item");
        let item = build_item(static_item);
        let obj = item.to_create_item_object(Guid::new(99));
        let Object_UpdateType::CreateObject { object_type, .. } = assert_create(&obj) else {
            unreachable!()
        };
        assert_eq!(*object_type, ObjectType::Item);
    }

    #[test]
    fn bag_family_item_maps_to_object_type_container() {
        let Some(static_item) = first_item_where(|i| !matches!(i.bag_family(), BagFamily::None))
        else {
            // Vanilla's item DB does include bags. If this ever ships
            // without one we want to know — fail loudly rather than silently
            // skipping the test.
            panic!("wow_items::vanilla has no non-None BagFamily item; data may be incomplete");
        };
        let item = build_item(static_item);
        let obj = item.to_create_item_object(Guid::new(99));
        let Object_UpdateType::CreateObject { object_type, .. } = assert_create(&obj) else {
            unreachable!()
        };
        assert_eq!(*object_type, ObjectType::Container);
    }

    #[test]
    fn create_object_carries_correct_guids_and_amount() {
        // Constructor-tier sanity: the item's own guid is written as guid3
        // and into the mask's object_guid field; item_owner flows through.
        let static_item = first_item_where(|i| matches!(i.bag_family(), BagFamily::None))
            .expect("wow_items::vanilla contains a non-bag item");
        let mut item = build_item(static_item);
        item.amount = 7;
        let owner = Guid::new(0xABCD);
        let obj = item.to_create_item_object(owner);
        let Object_UpdateType::CreateObject { guid3, .. } = assert_create(&obj) else {
            unreachable!()
        };
        assert_eq!(*guid3, item.guid);
    }
}
