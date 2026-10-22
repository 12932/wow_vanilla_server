//! A creature's tap, as each viewer sees it: grey (`UNIT_DYNFLAG_TAPPED`)
//! and with nothing to loot for all but its tapper and the tapper's group,
//! as mangos-classic's per-viewer `UNIT_DYNAMIC_FLAGS` and Thistle Tea's
//! `Visibility.Tap` show it. The creature's own field never carries the
//! flag; a viewer outside the tap is sent its own value (`map::views`).

use wow_world_messages::Guid;

use crate::game::Realm;
use crate::game::map::Map;
use crate::game::map::ai::DYNFLAG_TAPPED;
use crate::game::map::creature::Creature;
use crate::game::map::loot::DYNFLAG_LOOTABLE;

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

/// `attacker`'s damage landed on `victim`: an untapped creature no one
/// controls is tapped by the player whose side the blow is (a pet's or a
/// guardian's master, a possessed player's controller: mangos's
/// `GetBeneficiaryPlayer`), for their group. Only damage taps, melee, a
/// spell or a tick alike (mangos's `Unit::DealDamage`, after the absorb:
/// a blow absorbed whole taps no one); an aggro, a taunt, a heal's threat
/// don't. An NPC's blow leaves it untapped.
pub fn damaged(map: &mut Map, victim: Guid, attacker: Guid) {
    if map
        .creature(victim)
        .is_none_or(|c| c.tapped_by.is_some() || c.owner.is_some())
    {
        return;
    }
    let by = crate::game::map::pets::master(map, attacker);
    if !crate::game::guid::is_player(by) {
        return;
    }
    let realm = map.realm.clone();
    if let Some(c) = map.creature_mut(victim) {
        take(&realm, c, by);
    }
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

/// Whether `c` dying at `killer`'s blow is `g`'s kill (mangos's
/// `IsTappedBy` after `DealDamage` set the loot recipient): its tap's, or,
/// untapped, the tap the blow takes, a player's (or their pet's) for them
/// and their group; an NPC's blow takes none.
pub fn tapped_at_death(map: &Map, c: &Creature, killer: Guid, g: Guid) -> bool {
    match holders(&map.realm, c) {
        Some(h) => h.contains(&g),
        None => {
            let by = crate::game::map::pets::master(map, killer);
            crate::game::guid::is_player(by) && recipients(&map.realm, by, None).contains(&g)
        }
    }
}

/// The dynamic flags a viewer outside the tap sees: grey, nothing to loot.
/// Each viewer's own is in the one block (its create, or the update's
/// values) it gets of the creature (`map::views`): a second block about
/// the creature in the same packet kept the client from seeing the
/// first's changes (a death's health: no fall, its frame at full health).
pub fn grey(flags: u32) -> u32 {
    (flags | DYNFLAG_TAPPED) & !DYNFLAG_LOOTABLE
}
