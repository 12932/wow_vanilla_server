//! Phase 3, round 4, against Thistle Tea's entity component tests
//! (`unit_test.exs`, `corpse_test.exs`, `player_quest_slots_test.exs`,
//! `player_update_fields_test.exs`): what a unit's byte fields pack, a
//! corpse's GUID, look and equipment, a quest slot cleared, and the bank's
//! fields, its bag slots bought kept through a logout.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::items::ItemTemplate;
use crate::game::fields::*;
use crate::game::guid::{self, HighGuid};
use crate::game::harness::{self, Client};
use crate::game::map::inventory::{BANK_BAG_START, BANK_START, BUYBACK_START};
use crate::game::map::player::Item;
use crate::game::map::{Map, MapKey, combat};
use crate::game::store::{BAG_PLAYER, Logged};

const HELM: u32 = 963_001;
const ROBE: u32 = 963_002;
const BAG: u32 = 963_003;

fn data() -> GameData {
    let mut d = GameData::empty();
    for (entry, display_id, inventory_type, slots) in
        [(HELM, 1000, 1, 0), (ROBE, 2000, 5, 0), (BAG, 3000, 18, 8)]
    {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                class: if slots > 0 { 1 } else { 4 },
                subclass: 1,
                name: format!("Item {entry}"),
                display_id,
                inventory_type,
                container_slots: slots,
                stackable: 1,
                ..Default::default()
            },
        );
    }
    d
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    g: Guid,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    let key = realm
        .registry
        .online(g)
        .map_or(MapKey::world(0), |o| o.map().key);
    realm
        .maps
        .get(realm, key)
        .query(f)
        .await
        .expect("the map answers")
}

fn put(m: &mut Map, g: Guid, entry: u32, slot: u8) -> Guid {
    let data = m.realm.data.clone();
    let counter = m
        .realm
        .next_item
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let item = guid::item(counter);
    let p = m.players.get_mut(&g).unwrap();
    p.items
        .push(Item::new(&data, item, g, entry, BAG_PLAYER, slot, 1));
    p.refresh_inventory(&data);
    item
}

#[tokio::test]
async fn a_units_bytes_pack_race_class_gender_and_power_then_stand_and_form_then_sheath() {
    let realm = harness::realm_with(data());
    let (mut out, mut clients) = (Vec::new(), Vec::new());
    // The harness's characters are women.
    for (n, (race, class)) in [
        (Race::Human, Class::Paladin),
        (Race::Orc, Class::Warrior),
        (Race::NightElf, Class::Rogue),
    ]
    .into_iter()
    .enumerate()
    {
        let mut c = Client::connect(&realm, &format!("A{n}"));
        let g = c.enter(["Ana", "Bea", "Cat"][n], race, class).await;
        let bytes = on_map(&realm, g, move |m| {
            let f = |m: &Map, i: u16| m.players[&g].fields.get_u32(i).to_le_bytes();
            let before = (f(m, UNIT_FIELD_BYTES_0), f(m, UNIT_FIELD_BYTES_1));
            combat::sheathe(m, g, 1);
            (before, f(m, UNIT_FIELD_BYTES_2))
        })
        .await;
        out.push(bytes);
        clients.push(c);
    }
    // Race, class, gender, power: mana, rage, energy.
    assert_eq!(out[0].0.0, [1, 2, 1, 0]);
    assert_eq!(out[1].0.0, [2, 1, 1, 1]);
    assert_eq!(out[2].0.0, [4, 4, 1, 3]);
    // Standing, no loyalty, no form, no visibility flag.
    for (bytes, _) in &out {
        assert_eq!(bytes.1, [0, 0, 0, 0]);
    }
    // The sheath in the first byte, nothing else set, the last byte zero.
    assert_eq!(out[1].1, [1, 0, 0, 0]);
}

#[tokio::test]
async fn a_corpse_takes_its_owners_counter_native_look_and_the_worn_items_display_and_kind() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::NightElf, Class::Warrior).await;
    let got = on_map(&realm, g, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.retain(|i| !(i.bag == BAG_PLAYER && i.slot < 19));
        p.refresh_inventory(&data);
        (p.row.skin, p.row.face, p.row.hair_style) = (1, 2, 3);
        (p.row.hair_color, p.row.facial_hair) = (4, 5);
        p.fields.set_u32(UNIT_FIELD_DISPLAYID, 10_045);
        let native = p.fields.get_u32(UNIT_FIELD_NATIVEDISPLAYID);
        let at = p.position();
        let bare = crate::game::map::death::Corpse::new(&data, &m.players[&g], at, 0.5);
        let bare: Vec<u32> = (0..19)
            .map(|s| bare.fields.get_u32(CORPSE_FIELD_ITEM + s))
            .collect();
        put(m, g, HELM, 0);
        put(m, g, ROBE, 4);
        let c = crate::game::map::death::Corpse::new(&data, &m.players[&g], at, 0.5);
        let f = &c.fields;
        (
            c.guid,
            native,
            f.get_u32(CORPSE_FIELD_DISPLAY_ID),
            f.get_u32(CORPSE_FIELD_BYTES_1).to_le_bytes(),
            f.get_u32(CORPSE_FIELD_BYTES_2).to_le_bytes(),
            [0u16, 1, 4].map(|s| f.get_u32(CORPSE_FIELD_ITEM + s)),
            f.get_guid(CORPSE_FIELD_OWNER),
            f.get_f32(CORPSE_FIELD_FACING),
            bare,
        )
    })
    .await;
    let (corpse, native, display, bytes_1, bytes_2, items, owner, facing, bare) = got;
    assert_eq!(guid::high(corpse), Some(HighGuid::Corpse));
    assert_eq!(guid::counter(corpse), guid::counter(g));
    assert_eq!(owner, g);
    assert_eq!(facing, 0.5);
    // Its own model, not the one an illusion put on.
    assert_eq!(display, native);
    assert_ne!(display, 10_045);
    assert_eq!(bytes_1, [0, 4, 1, 1]);
    assert_eq!(bytes_2, [2, 3, 4, 5]);
    assert_eq!(items, [1000 | 1 << 24, 0, 2000 | 5 << 24]);
    assert!(bare.iter().all(|w| *w == 0));
}

#[tokio::test]
async fn a_quest_slot_emptied_is_sent_as_zeros() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let got = on_map(&realm, g, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.journal.log.push(Logged {
            quest: 783,
            kills: [2, 0, 0, 0],
            ..Default::default()
        });
        crate::game::map::quests::refresh_log(p, &data);
        let slot = |p: &crate::game::map::player::Player| {
            (0..3)
                .map(|k| p.fields.get_u32(PLAYER_QUEST_LOG_1_1 + k))
                .collect::<Vec<_>>()
        };
        let held = slot(p);
        p.fields.clear_dirty();
        p.journal.log.clear();
        crate::game::map::quests::refresh_log(p, &data);
        let dirty = (0..2).all(|k| p.fields.dirty().get(PLAYER_QUEST_LOG_1_1 + k));
        (held, slot(p), dirty)
    })
    .await;
    assert_eq!(got.0, vec![783, 2, 0]);
    assert_eq!(got.1, vec![0, 0, 0]);
    assert!(got.2, "the cleared words go to the client");
}

#[tokio::test]
async fn the_bank_its_bags_and_the_buyback_have_their_fields_and_bought_bag_slots_stay() {
    // 1.12's offsets: the bank's 24, its 6 bags, the buyback's 12, far sight.
    assert_eq!(PLAYER_FIELD_BANK_SLOT_1, 0x234);
    assert_eq!(PLAYER_FIELD_BANK_SLOT_1 + 23 * 2, 0x262);
    assert_eq!(PLAYER_FIELD_BANKBAG_SLOT_1, 0x264);
    assert_eq!(PLAYER_FIELD_BANKBAG_SLOT_1 + 5 * 2, 0x26E);
    assert_eq!(PLAYER_FIELD_VENDORBUYBACK_SLOT_1, 0x270);
    assert_eq!(PLAYER_FIELD_VENDORBUYBACK_SLOT_1 + 11 * 2, 0x286);
    assert_eq!(PLAYER_FARSIGHT, 0x2C8);
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let got = on_map(&realm, g, move |m| {
        let first = put(m, g, HELM, BANK_START);
        let last = put(m, g, ROBE, BANK_START + 23);
        let bag = put(m, g, BAG, BANK_BAG_START + 5);
        let sold = put(m, g, HELM, BUYBACK_START);
        let p = m.players.get_mut(&g).unwrap();
        p.fields.set_byte(PLAYER_BYTES_2, 2, 6);
        let f = &p.fields;
        (
            [first, last, bag, sold],
            [
                f.get_guid(PLAYER_FIELD_BANK_SLOT_1),
                f.get_guid(PLAYER_FIELD_BANK_SLOT_1 + 46),
                f.get_guid(PLAYER_FIELD_BANKBAG_SLOT_1 + 10),
                f.get_guid(PLAYER_FIELD_VENDORBUYBACK_SLOT_1),
            ],
            f.get_u32(PLAYER_BYTES_2).to_le_bytes(),
        )
    })
    .await;
    let (items, fields, bytes) = got;
    assert_eq!(items, fields);
    // Facial hair, nothing, the six bag slots, the rest state.
    assert_eq!(bytes[2], 6);
    // The slots bought are the character's after a logout.
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.login(g).await;
    let after = on_map(&realm, g, move |m| {
        crate::game::map::inventory::bank_bag_slots(&m.players[&g])
    })
    .await;
    assert_eq!(after, 6);
}
