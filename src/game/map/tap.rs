//! A creature's tap, as each viewer sees it: grey (`UNIT_DYNFLAG_TAPPED`)
//! and with nothing to loot for all but its tapper and the tapper's group,
//! as mangos-classic's per-viewer `UNIT_DYNAMIC_FLAGS` and Thistle Tea's
//! `Visibility.Tap` show it. The creature's own field never carries the
//! flag; a viewer outside the tap is sent its own value in the one block
//! (its create, or the update's values) it gets of the creature.

use ahash::AHashMap;
use wow_world_messages::Guid;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::ai::DYNFLAG_TAPPED;
use crate::game::map::creature::Creature;
use crate::game::map::loot::DYNFLAG_LOOTABLE;
use crate::game::update::{self, Batch};

/// Those whose kill a creature tapped by `tapper` for `group` is: that
/// group while it stands, though the tapper has left it or joined another
/// (mangos's `GetGroupLootRecipient`), else the tapper's group now; and
/// the tapper always (mangos's `IsTappedBy`, `m_lootRecipientGuid`).
pub fn recipients(realm: &Realm, tapper: Guid, group: Option<u32>) -> Vec<Guid> {
    let groups = realm.social.groups.lock();
    let mut out: Vec<Guid> = group
        .and_then(|id| groups.get(id))
        .or_else(|| groups.group_of(tapper))
        .map(|g| g.members.iter().map(|m| m.guid).collect())
        .unwrap_or_default();
    if !out.contains(&tapper) {
        out.push(tapper);
    }
    out
}

/// The group `c` was tapped for, if `tapper` holds its tap.
pub fn group_of(c: &Creature, tapper: Guid) -> Option<u32> {
    c.tap_group.filter(|_| c.tapped_by == Some(tapper))
}

/// `tapper` takes `c`'s tap, for the group they are in now (mangos's
/// `SetLootRecipient`).
pub fn take(realm: &Realm, c: &mut Creature, tapper: Guid) {
    c.tapped_by = Some(tapper);
    c.tap_group = realm.social.groups.lock().group_of(tapper).map(|g| g.id);
}

/// `c`'s tap let go.
pub fn clear(c: &mut Creature) {
    c.tapped_by = None;
    c.tap_group = None;
}

fn holders(realm: &Realm, c: &Creature) -> Option<Vec<Guid>> {
    c.tapped_by.map(|t| recipients(realm, t, c.tap_group))
}

/// Whether `g` holds `c`'s tap (no one's is everyone's): the tapper, or
/// one of the group it is for.
pub fn holds(map: &Map, c: &Creature, g: Guid) -> bool {
    holders(&map.realm, c).is_none_or(|h| h.contains(&g))
}

/// The dynamic flags a viewer outside the tap sees: grey, nothing to loot.
fn grey(flags: u32) -> u32 {
    (flags | DYNFLAG_TAPPED) & !DYNFLAG_LOOTABLE
}

/// `c`'s changes this update with `flags` for its dynamic flags, carried
/// whether they changed or not.
fn view_block(c: &Creature, flags: u32) -> Option<Vec<u8>> {
    let mut f = c.fields.clone();
    f.set_u32(UNIT_DYNAMIC_FLAGS, flags);
    f.touch(UNIT_DYNAMIC_FLAGS);
    update::values_block(c.guid, &f, Visibility::Public)
}

/// `c`'s values this update, each observer's own view of its tap in them:
/// each outside the tap its grey view; with `all` (a tap taken or gone),
/// each inside it the shared value too, changed or not. One block each,
/// as mangos-classic and Thistle Tea personalise the one update: a second
/// block about the creature in the same packet kept the client from
/// seeing the first's changes (a death's health: no fall, its frame at
/// full health).
pub fn tell(realm: &Realm, batches: &mut AHashMap<Guid, Batch>, c: &Creature, all: bool) {
    let flags = c.fields.get_u32(UNIT_DYNAMIC_FLAGS);
    let holders = holders(realm, c);
    let outside = view_block(c, grey(flags));
    let inside = if all {
        view_block(c, flags)
    } else {
        update::values_block(c.guid, &c.fields, Visibility::Public)
    };
    for o in &c.observers {
        let theirs = holders.as_ref().is_none_or(|h| h.contains(o));
        let block = if theirs { &inside } else { &outside };
        if let Some(b) = block {
            batches.entry(*o).or_default().push_for(c.guid, b);
        }
    }
}

/// `creature`'s create for `observer`, if the tap isn't theirs: its grey
/// view in it, rather than a second block after it.
pub fn create_for(map: &Map, observer: Guid, creature: Guid) -> Option<std::sync::Arc<[u8]>> {
    let c = map.creature(creature)?;
    if holders(&map.realm, c)?.contains(&observer) {
        return None;
    }
    let mut fields = c.fields.clone();
    fields.set_u32(
        UNIT_DYNAMIC_FLAGS,
        grey(c.fields.get_u32(UNIT_DYNAMIC_FLAGS)),
    );
    let mut create = c.create(map.now);
    create.fields = &fields;
    Some(std::sync::Arc::from(update::create_block(
        &create,
        Visibility::Public,
    )))
}
