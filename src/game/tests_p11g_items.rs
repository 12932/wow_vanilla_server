//! Round 11, against Thistle Tea's `item_transformation_test.exs`
//! ("replaces equipment, projects packets and stats, and survives
//! restore"): an item changed into another is a new item of a GUID of its
//! own, the old one destroyed, its temporary enchantment kept with its
//! time and charges, and so after a relog (mangos-classic's
//! `Player::ConvertItem`: `Item::CreateItem`, each enchantment copied with
//! its duration and charges).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::harness::{self, Client};
use super::tests_p3_cast_kit::*;
use crate::game::data::GameData;
use crate::game::map::Map;
use crate::game::map::enchant::{self, Enchant};
use crate::game::map::inventory::SLOT_MAIN_HAND;
use crate::game::store::BAG_PLAYER;

const OLD: u32 = 60_920;
const NEW: u32 = 60_921;
/// A poison's coating: three charges, ten minutes.
const COATING: Enchant = Enchant {
    id: 60_922,
    duration_ms: 600_000,
    charges: 3,
};

fn data(d: &mut GameData) {
    for (entry, name) in [(OLD, "Old Blade"), (NEW, "New Blade")] {
        d.items.insert(
            entry,
            crate::game::data::items::ItemTemplate {
                entry,
                class: 2,
                subclass: 7,
                inventory_type: 13,
                name: name.into(),
                display_id: 1000,
                stackable: 1,
                max_durability: 40,
                allowable_class: -1,
                allowable_race: -1,
                ..Default::default()
            },
        );
    }
}

/// What `g` wields: its GUID, entry and temporary enchantment.
fn wielded(m: &Map, g: Guid) -> Option<(Guid, u32, Enchant)> {
    let p = m.players.get(&g)?;
    let i = p.item_at(BAG_PLAYER, SLOT_MAIN_HAND)?;
    let it = &p.items[i];
    Some((it.guid, it.entry, enchant::get(it, enchant::TEMP_SLOT)))
}

/// A coated blade in the main hand changed into another: of a new GUID,
/// the old destroyed and the new created, the coating's time told under
/// it; after a relog, the same.
#[tokio::test]
async fn a_changed_item_is_a_new_one_its_coating_kept_through_a_relog() {
    let realm = fast_realm(data);
    let (mut c, g, map) = human(&realm, "P11GITEM", "Changer", Class::Rogue).await;
    let old = map
        .query(move |m| {
            crate::game::map::inventory::add_item(m, g, OLD, 1, false).expect("given");
            let d = m.realm.data.clone();
            let p = m.players.get_mut(&g).unwrap();
            // In the main hand, the rogue's dagger gone.
            if let Some(w) = p.item_at(BAG_PLAYER, SLOT_MAIN_HAND) {
                p.items.remove(w);
            }
            let i = p.items.iter().position(|x| x.entry == OLD).unwrap();
            p.items[i].bag = BAG_PLAYER;
            p.items[i].slot = SLOT_MAIN_HAND;
            enchant::set(&mut p.items[i], enchant::TEMP_SLOT, COATING);
            p.refresh_inventory(&d);
            p.items[i].guid
        })
        .await
        .unwrap();
    c.clear();
    let new = map
        .query(move |m| {
            crate::game::map::extra_effects::change_item(m, g, old, NEW);
            wielded(m, g)
        })
        .await
        .unwrap()
        .expect("a blade");
    assert_ne!(new.0, old, "a GUID of its own");
    assert_eq!((new.1, new.2), (NEW, COATING), "the new blade, coated");
    // The old destroyed, the new created, its coating's time told.
    c.wait_for("SMSG_DESTROY_OBJECT", |m| match m {
        ServerOpcodeMessage::SMSG_DESTROY_OBJECT(d) if d.guid == old => Some(()),
        _ => None,
    })
    .await;
    c.wait_for("the new blade's create", |m| {
        harness::creates(m).contains(&new.0).then_some(())
    })
    .await;
    let told = c
        .wait_for("SMSG_ITEM_ENCHANT_TIME_UPDATE", |m| match m {
            ServerOpcodeMessage::SMSG_ITEM_ENCHANT_TIME_UPDATE(u) => Some((u.item, u.duration)),
            _ => None,
        })
        .await;
    assert_eq!(told, (new.0, 600));
    // Logged out and in: the new blade, of its GUID, coated.
    relog(&mut c, g).await;
    let back = realm
        .registry
        .online(g)
        .unwrap()
        .map()
        .query(move |m| wielded(m, g))
        .await
        .unwrap()
        .expect("a blade");
    assert_eq!((back.0, back.1), (new.0, NEW));
    assert_eq!((back.2.id, back.2.charges), (COATING.id, COATING.charges));
}

async fn relog(c: &mut Client, g: Guid) {
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    c.clear();
    c.login(g).await;
}
