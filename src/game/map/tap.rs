//! A creature's tap, as each viewer sees it: grey (`UNIT_DYNFLAG_TAPPED`)
//! and with nothing to loot for all but its tapper and the tapper's group,
//! as mangos-classic's per-viewer `UNIT_DYNAMIC_FLAGS` and Thistle Tea's
//! `Visibility.Tap` show it. The creature's own field never carries the
//! flag; a viewer outside the tap is sent its own value after the shared
//! one.

use wow_world_messages::Guid;

use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::ai::DYNFLAG_TAPPED;
use crate::game::map::loot::DYNFLAG_LOOTABLE;
use crate::game::update;

/// Those whose kill a creature tapped by `tapper` is: the tapper and the
/// tapper's group (mangos's `IsTappedBy`).
fn holders(map: &Map, tapper: Guid) -> Vec<Guid> {
    let mut out: Vec<Guid> = map
        .realm
        .social
        .groups
        .lock()
        .group_of(tapper)
        .map(|g| g.members.iter().map(|m| m.guid).collect())
        .unwrap_or_default();
    if !out.contains(&tapper) {
        out.push(tapper);
    }
    out
}

/// Whether `g` holds `tapper`'s tap: the tapper, or one of their group.
pub fn holds(map: &Map, tapper: Guid, g: Guid) -> bool {
    holders(map, tapper).contains(&g)
}

/// A values block with only `object`'s dynamic flags, at `value`.
fn flags_block(object: Guid, value: u32) -> Option<Vec<u8>> {
    let mut f = Fields::new(TypeId::Unit, object, crate::game::guid::entry(object));
    f.set_u32(UNIT_DYNAMIC_FLAGS, value ^ DYNFLAG_TAPPED);
    f.clear_dirty();
    f.set_u32(UNIT_DYNAMIC_FLAGS, value);
    update::values_block(object, &f, Visibility::Public)
}

/// The dynamic flags a viewer outside the tap sees: grey, nothing to loot.
fn grey(flags: u32) -> u32 {
    (flags | DYNFLAG_TAPPED) & !DYNFLAG_LOOTABLE
}

/// `creature`'s tap to its observers, after this update's shared values:
/// each outside it its grey view; with `all`, each inside it the shared
/// value again (a tap taken or gone).
pub fn tell(map: &mut Map, creature: Guid, all: bool) {
    let Some(c) = map.creature(creature) else {
        return;
    };
    let flags = c.fields.get_u32(UNIT_DYNAMIC_FLAGS);
    let observers: Vec<Guid> = c.observers.iter().copied().collect();
    let holders = c.tapped_by.map(|t| holders(map, t));
    let outside = flags_block(creature, grey(flags));
    let inside = all.then(|| flags_block(creature, flags)).flatten();
    for o in observers {
        let theirs = holders.as_ref().is_none_or(|h| h.contains(&o));
        let block = if theirs { &inside } else { &outside };
        if let Some(b) = block {
            map.batches.entry(o).or_default().push_for(creature, b);
        }
    }
}

/// `observer` is told of `creature`: its grey view, after the create, if
/// the tap isn't theirs.
pub fn introduced(map: &mut Map, observer: Guid, creature: Guid) {
    let Some(c) = map.creature(creature) else {
        return;
    };
    let Some(tapper) = c.tapped_by else {
        return;
    };
    let flags = c.fields.get_u32(UNIT_DYNAMIC_FLAGS);
    if holders(map, tapper).contains(&observer) {
        return;
    }
    if let Some(b) = flags_block(creature, grey(flags)) {
        map.batches
            .entry(observer)
            .or_default()
            .push_for(creature, &b);
    }
}
