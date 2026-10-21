//! Round 11, against Thistle Tea's `item_location_limits_test.exs`
//! ("login commits removal of a banked item before inventory is sent"):
//! an item bound to another map, lying in the bank, is gone at login
//! before the client is told of anything, never created for it nor
//! destroyed, and gone for good (mangos-classic's `_LoadInventory`
//! deleting an item `IsLimitedToAnotherMapOrZone` as it loads).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::harness::{self, Client};
use super::tests_p3_cast_kit::*;
use crate::game::data::GameData;
use crate::game::map::inventory::BANK_START;
use crate::game::store::BAG_PLAYER;

/// A beacon for Alterac Valley alone (map 30), and a stone for anywhere.
const BEACON: u32 = 60_930;
const STONE: u32 = 60_931;

fn data(d: &mut GameData) {
    for (entry, name, map_bound) in [(BEACON, "Beacon", 30), (STONE, "Stone", 0)] {
        d.items.insert(
            entry,
            crate::game::data::items::ItemTemplate {
                entry,
                name: name.into(),
                display_id: 1000,
                stackable: 1,
                map_bound,
                ..Default::default()
            },
        );
    }
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

/// What `g` carries of `entry`.
async fn carried(realm: &std::sync::Arc<crate::game::Realm>, g: Guid, entry: u32) -> Vec<Guid> {
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        m.players[&g]
            .items
            .iter()
            .filter(|i| i.entry == entry)
            .map(|i| i.guid)
            .collect()
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn a_banked_item_bound_elsewhere_is_gone_at_login_before_the_client_hears_of_it() {
    let realm = fast_realm(data);
    let (mut c, g, map) = human(&realm, "P11GLOGIN", "Banker", Class::Warrior).await;
    // The beacon and the stone in the bank, the login's own look at the
    // bank long done.
    let (beacon, stone) = map
        .query(move |m| {
            let mut put = |entry: u32, slot: u8| {
                crate::game::map::inventory::add_item(m, g, entry, 1, false).expect("given");
                let d = m.realm.data.clone();
                let p = m.players.get_mut(&g).unwrap();
                let i = p.items.iter().position(|i| i.entry == entry).unwrap();
                p.items[i].bag = BAG_PLAYER;
                p.items[i].slot = slot;
                p.refresh_inventory(&d);
                p.items[i].guid
            };
            let beacon = put(BEACON, BANK_START);
            let stone = put(STONE, BANK_START + 1);
            m.players.get_mut(&g).unwrap().misc.items_login_checked = true;
            (beacon, stone)
        })
        .await
        .unwrap();
    assert_eq!(
        carried(&realm, g, BEACON).await,
        vec![beacon],
        "kept in the bank"
    );
    relog(&mut c, g).await;
    assert!(carried(&realm, g, BEACON).await.is_empty(), "gone at login");
    assert_eq!(
        carried(&realm, g, STONE).await,
        vec![stone],
        "the stone kept"
    );
    // Its client never heard of it: neither created nor destroyed.
    let heard = c
        .gets(Duration::from_millis(1_300), |m| match m {
            ServerOpcodeMessage::SMSG_DESTROY_OBJECT(d) if d.guid == beacon => Some("destroyed"),
            m if harness::creates(m).contains(&beacon) => Some("created"),
            _ => None,
        })
        .await;
    assert_eq!(heard, None);
    let stone_created = c
        .gets(Duration::from_millis(10), |m| {
            harness::creates(m).contains(&stone).then_some(())
        })
        .await;
    assert!(stone_created.is_some(), "the stone created as ever");
    // Gone for good: not back at the next login.
    relog(&mut c, g).await;
    assert!(carried(&realm, g, BEACON).await.is_empty());
}
