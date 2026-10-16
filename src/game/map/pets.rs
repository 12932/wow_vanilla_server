//! Pets, guardians, totems and charmed units: units a player or creature
//! controls.

use wow_world_messages::Guid;

use crate::game::map::Map;

/// `master` takes control of `unit` (charm or possession).
pub fn charmed(map: &mut Map, master: Guid, unit: Guid, possess: bool) {
    let _ = possess;
    if let Some(c) = map.creature_mut(unit) {
        c.owner = Some(master);
    }
}

/// `master` lets `unit` go.
pub fn uncharmed(map: &mut Map, master: Guid, unit: Guid) {
    if let Some(c) = map.creature_mut(unit)
        && c.owner == Some(master)
    {
        c.owner = None;
    }
}
