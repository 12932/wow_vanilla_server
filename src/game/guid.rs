//! Typed GUIDs. A vanilla GUID carries its object's kind in its top 16
//! bits; creatures, pets and game objects also carry their entry (bits
//! 24..48) over a 24-bit counter. The client reads those bits, so every
//! object gets the kind it is.

use wow_world_messages::Guid;

/// The kind of object a GUID names: its top 16 bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum HighGuid {
    Player = 0x0000,
    Item = 0x4000,
    Container = 0x4001,
    GameObject = 0xF110,
    Transport = 0xF120,
    Unit = 0xF130,
    Pet = 0xF140,
    DynamicObject = 0xF100,
    Corpse = 0xF101,
    MoTransport = 0x1FC0,
}

impl HighGuid {
    /// Whether GUIDs of this kind carry an entry over a 24-bit counter.
    pub const fn has_entry(self) -> bool {
        matches!(
            self,
            HighGuid::GameObject | HighGuid::Transport | HighGuid::Unit | HighGuid::Pet
        )
    }
}

/// The largest counter a GUID with an entry can hold.
pub const ENTRY_COUNTER_MAX: u32 = 0x00FF_FFFF;

/// A GUID of `high` kind. `entry` is ignored for kinds without one; the
/// counter is cut to 24 bits for kinds with one.
pub const fn make(high: HighGuid, entry: u32, counter: u32) -> Guid {
    let high_bits = (high as u64) << 48;
    if high.has_entry() {
        Guid::new(
            high_bits
                | (((entry as u64) & 0x00FF_FFFF) << 24)
                | (counter as u64 & ENTRY_COUNTER_MAX as u64),
        )
    } else {
        Guid::new(high_bits | counter as u64)
    }
}

pub const fn player(counter: u32) -> Guid {
    make(HighGuid::Player, 0, counter)
}

pub const fn item(counter: u32) -> Guid {
    make(HighGuid::Item, 0, counter)
}

pub const fn unit(entry: u32, counter: u32) -> Guid {
    make(HighGuid::Unit, entry, counter)
}

pub const fn game_object(entry: u32, counter: u32) -> Guid {
    make(HighGuid::GameObject, entry, counter)
}

pub const fn corpse(counter: u32) -> Guid {
    make(HighGuid::Corpse, 0, counter)
}

/// The kind a GUID names, if it is one the server makes.
pub fn high(guid: Guid) -> Option<HighGuid> {
    let bits = (guid.guid() >> 48) as u16;
    Some(match bits {
        0x0000 => HighGuid::Player,
        0x4000 => HighGuid::Item,
        0x4001 => HighGuid::Container,
        0xF110 => HighGuid::GameObject,
        0xF120 => HighGuid::Transport,
        0xF130 => HighGuid::Unit,
        0xF140 => HighGuid::Pet,
        0xF100 => HighGuid::DynamicObject,
        0xF101 => HighGuid::Corpse,
        0x1FC0 => HighGuid::MoTransport,
        _ => return None,
    })
}

pub fn is_player(guid: Guid) -> bool {
    !guid.is_zero() && high(guid) == Some(HighGuid::Player)
}

pub fn is_unit(guid: Guid) -> bool {
    matches!(high(guid), Some(HighGuid::Unit | HighGuid::Pet))
}

pub fn is_corpse(guid: Guid) -> bool {
    high(guid) == Some(HighGuid::Corpse)
}

pub fn is_game_object(guid: Guid) -> bool {
    matches!(
        high(guid),
        Some(HighGuid::GameObject | HighGuid::Transport | HighGuid::MoTransport)
    )
}

/// The entry a GUID carries; 0 for kinds without one.
pub fn entry(guid: Guid) -> u32 {
    match high(guid) {
        Some(h) if h.has_entry() => ((guid.guid() >> 24) & 0x00FF_FFFF) as u32,
        _ => 0,
    }
}

/// The counter part of a GUID.
pub fn counter(guid: Guid) -> u32 {
    match high(guid) {
        Some(h) if h.has_entry() => (guid.guid() & ENTRY_COUNTER_MAX as u64) as u32,
        _ => guid.guid() as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_creature_guid_carries_its_kind_entry_and_counter() {
        let g = unit(299, 1234);
        assert_eq!(g.guid(), 0xF130_0001_2B00_04D2);
        assert_eq!(high(g), Some(HighGuid::Unit));
        assert_eq!(entry(g), 299);
        assert_eq!(counter(g), 1234);
        assert!(is_unit(g) && !is_player(g));
    }

    #[test]
    fn a_player_guid_is_its_counter() {
        let g = player(42);
        assert_eq!(g.guid(), 42);
        assert!(is_player(g));
        assert_eq!(entry(g), 0);
        assert!(!is_player(Guid::zero()));
    }

    #[test]
    fn an_item_guid_has_the_item_kind() {
        assert_eq!(item(7).guid(), 0x4000_0000_0000_0007);
        assert_eq!(high(item(7)), Some(HighGuid::Item));
    }
}
