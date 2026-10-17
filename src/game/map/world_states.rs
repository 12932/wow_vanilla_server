//! World states: the numbers and icons the client's map and frames show.
//! A player is sent their map's and zone's on entering the world and on
//! every change of zone: a battleground match's, the zone's outdoor PvP
//! objectives', and the server-wide ones events publish (an invasion's
//! tallies), which also go to everyone when they change.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    Area, SMSG_INIT_WORLD_STATES, SMSG_UPDATE_WORLD_STATE, WorldState,
};

use crate::game::Realm;
use crate::game::map::Map;

/// The world states for a player in `zone` of `map`.
pub fn states_for(map: &Map, zone: u32) -> Vec<WorldState> {
    let mut states = Vec::new();
    if let Some(m) = map.battleground.as_ref() {
        states.extend(m.world_states());
    }
    states.extend(zone_states(map, zone));
    states.extend(global(&map.realm));
    states
}

/// A zone's own world states outside a battleground (outdoor PvP's
/// objectives).
pub fn zone_states(map: &Map, zone: u32) -> Vec<WorldState> {
    let _ = (map, zone);
    Vec::new()
}

/// The server-wide world states, as events last published them.
pub fn global(realm: &Realm) -> Vec<WorldState> {
    let mut states: Vec<WorldState> = realm
        .social
        .world_states
        .lock()
        .iter()
        .map(|(state, value)| WorldState {
            state: *state,
            value: *value,
        })
        .collect();
    states.sort_by_key(|s| s.state);
    states
}

/// `SMSG_INIT_WORLD_STATES` to `g` for where they are.
pub fn send_init(map: &Map, g: Guid) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    p.outbound.send(&SMSG_INIT_WORLD_STATES {
        map: crate::game::map::movement::map_enum(map.key.map),
        area: Area::try_from(p.zone).unwrap_or_default(),
        states: states_for(map, p.zone),
    });
}

/// A server-wide world state changes: kept for those who come later, and
/// sent to everyone online.
pub fn set_global(realm: &Realm, state: u32, value: u32) {
    let changed = realm.social.world_states.lock().insert(state, value) != Some(value);
    if !changed {
        return;
    }
    let Some(frame) = crate::game::conn::encode(&SMSG_UPDATE_WORLD_STATE {
        state: WorldState { state, value },
    }) else {
        return;
    };
    for online in realm.registry.all_online() {
        online.outbound.send_frame(frame.clone());
    }
}
