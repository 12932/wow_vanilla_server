//! Eyes of the Beast (`MOD_POSSESS_PET`), as Thistle Tea's `ControlSync`
//! possesses a pet: the hunter moves their own pet and sees through its
//! eyes; the pet stays theirs, passive and still while they do, and goes
//! back to what it was doing when the aura ends.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::SMSG_CLIENT_CONTROL_UPDATE;

use super::*;
use crate::game::map::Map;

/// `master` takes direct control of their own `pet`.
pub fn look(map: &mut Map, master: Guid, pet: Guid) {
    if !map
        .creature(pet)
        .and_then(|c| c.pet.as_deref())
        .is_some_and(|s| s.owner == master && s.eyes.is_none())
    {
        return;
    }
    // It stops where it has come to: the hunter moves it from there.
    crate::game::map::ai::rest(map, pet, None);
    let Some(c) = map.creature_mut(pet) else {
        return;
    };
    let position = c.position;
    let Some(s) = c.pet.as_deref_mut() else {
        return;
    };
    s.eyes = Some((s.react, s.command));
    s.react = REACT_PASSIVE;
    s.command = COMMAND_STAY;
    s.stay_at = Some(position);
    c.fields
        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_POSSESSED, true);
    crate::game::map::combat::stop_attack(map, pet);
    crate::game::map::spells::sight::grant(map, master, pet);
    if let Some(p) = map.players.get(&master) {
        p.outbound.send(&SMSG_CLIENT_CONTROL_UPDATE {
            guid: pet,
            allow_movement: true,
        });
    }
}

/// `master` lets go of `pet`: it is the pet it was.
pub fn look_away(map: &mut Map, master: Guid, pet: Guid) {
    let Some(c) = map.creature_mut(pet) else {
        return;
    };
    let Some(s) = c.pet.as_deref_mut() else {
        return;
    };
    let Some((react, command)) = s.eyes.take() else {
        return;
    };
    s.react = react;
    s.command = command;
    if command != COMMAND_STAY {
        s.stay_at = None;
    }
    c.fields
        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_POSSESSED, false);
    crate::game::map::spells::sight::release(map, master, pet);
    if let Some(p) = map.players.get_mut(&master) {
        if p.owned.mover == Some(pet) {
            p.owned.mover = None;
        }
        p.outbound.send(&SMSG_CLIENT_CONTROL_UPDATE {
            guid: master,
            allow_movement: true,
        });
    }
}

/// Whether `master` moves `pet` with Eyes of the Beast.
pub fn controls(map: &Map, master: Guid, pet: Guid) -> bool {
    state(map, pet).is_some_and(|s| s.owner == master && s.eyes.is_some())
}
