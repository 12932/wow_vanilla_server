//! The bank, open at a banker: what is in it moves only while that banker
//! still serves the player (alive, friendly, within reach), as Thistle
//! Tea's `Bank.authorize` has it. mangos-classic checks a banker only to
//! buy a bag slot; the client shuts its bank window as its player walks
//! off, and the server refuses a client that sends bank moves anyway (the
//! orchestrator's call of 2026-10-19).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::InventoryResult;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;

use crate::game::map::Map;
use crate::game::map::inventory::{BANK_BAG_END, BANK_BAG_START, BANK_START};
use crate::game::map::npc::{NPC_FLAG_BANKER, npc_entry};
use crate::game::store::BAG_PLAYER;

/// Whether `(bag, slot)` is the bank's: one of its slots or bag slots, or
/// in one of its bags.
pub fn in_bank(bag: u8, slot: u8) -> bool {
    (BANK_BAG_START..BANK_BAG_END).contains(&bag)
        || (bag == BAG_PLAYER && (BANK_START..BANK_BAG_END).contains(&slot))
}

/// The bank shown at `banker` (`SMSG_SHOW_BANK`): open, while they serve.
pub fn show(map: &mut Map, player: Guid, banker: Guid) {
    if let Some(p) = map.players.get_mut(&player) {
        p.misc.banker = Some(banker);
        p.outbound
            .send(&wow_world_messages::vanilla::SMSG_SHOW_BANK { guid: banker });
    }
}

/// The bank closed: a banker that won't serve was asked.
pub fn close(map: &mut Map, player: Guid) {
    if let Some(p) = map.players.get_mut(&player) {
        p.misc.banker = None;
    }
}

/// Whether the player's bank is open: the banker it was opened at serves
/// them still. If not, it is closed (Thistle Tea's stale capability
/// dropped).
pub fn is_open(map: &mut Map, player: Guid) -> bool {
    let banker = map.players.get(&player).and_then(|p| p.misc.banker);
    let open = banker.is_some_and(|b| npc_entry(map, player, b, NPC_FLAG_BANKER).is_some());
    if !open {
        close(map, player);
    }
    open
}

/// A client's item packet that touches the bank while no bank is open:
/// refused, the client told it is too far from the bank (Thistle Tea's
/// inventory result 35).
pub fn refused(map: &mut Map, player: Guid, msg: &ClientOpcodeMessage) -> bool {
    use ClientOpcodeMessage as C;
    let Some(p) = map.players.get(&player) else {
        return false;
    };
    let item = |at: (u8, u8)| p.item_at(at.0, at.1).map(|i| p.items[i].guid);
    let banked = |at: (u8, u8)| in_bank(at.0, at.1);
    // Whether it touches the bank, and the item it names (the first
    // slot's, else the second's).
    let pair = |a: (u8, u8), b: (u8, u8)| (banked(a) || banked(b), item(a).or(item(b)));
    let (touches, named) = match msg {
        C::CMSG_SWAP_INV_ITEM(m) => pair(
            (BAG_PLAYER, m.source_slot.as_int()),
            (BAG_PLAYER, m.destination_slot.as_int()),
        ),
        C::CMSG_SWAP_ITEM(m) => pair(
            (m.source_bag, m.source_slot),
            (m.destination_bag, m.destionation_slot),
        ),
        C::CMSG_SPLIT_ITEM(m) => pair(
            (m.source_bag, m.source_slot),
            (m.destination_bag, m.destination_slot),
        ),
        C::CMSG_WRAP_ITEM(m) => pair(
            (m.item_bag_index, m.item_slot),
            (m.gift_bag_index, m.gift_slot),
        ),
        C::CMSG_AUTOEQUIP_ITEM(m) => {
            let from = (m.source_bag, m.source_slot);
            (banked(from), item(from))
        }
        C::CMSG_DESTROYITEM(m) => {
            let at = (m.bag, m.slot);
            (banked(at), item(at))
        }
        // From anywhere into one of the bank's bags.
        C::CMSG_AUTOSTORE_BAG_ITEM(m) => {
            let from = (m.source_bag, m.source_slot);
            let into_bank = (BANK_BAG_START..BANK_BAG_END).contains(&m.destination_bag);
            (banked(from) || into_bank, item(from))
        }
        // Into the bank, or out of it.
        C::CMSG_AUTOBANK_ITEM(m) => (true, item((m.bag_index, m.slot_index))),
        C::CMSG_AUTOSTORE_BANK_ITEM(m) => (true, item((m.bag_index, m.slot_index))),
        C::CMSG_AUTOEQUIP_ITEM_SLOT(m) => {
            let at = p
                .item_by_guid(m.guid)
                .map(|i| (p.items[i].bag, p.items[i].slot));
            (at.is_some_and(banked), Some(m.guid))
        }
        _ => return false,
    };
    if !touches || is_open(map, player) {
        return false;
    }
    if let Some(p) = map.players.get(&player) {
        crate::game::map::items_misc::fail(
            p,
            InventoryResult::TooFarAwayFromBank,
            named.unwrap_or_default(),
        );
    }
    true
}
