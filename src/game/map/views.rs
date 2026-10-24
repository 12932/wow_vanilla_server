//! What each viewer sees of a unit where it isn't what everyone sees, as
//! mangos-classic builds a values block for each viewer
//! (`Object::BuildValuesUpdate`, `GetUpdateFieldFlagsForTarget`):
//! - a creature's tap (`map::tap`): grey, nothing to loot, outside it;
//! - a Hunter's Mark's (Mind Vision's) tracking (`UNIT_DYNFLAG_TRACK_UNIT`)
//!   for the one whose mark it is alone ("hunters mark effects should only
//!   be visible to owners and not all players");
//! - a creature's owner-only fields (its damage, stats, resistances, attack
//!   power, a pet's experience and training points) for its master
//!   (`UF_FLAG_OWNER_ONLY`);
//! - its damage and resistances for whoever's Empathy (Beast Lore) is on
//!   it (`UF_FLAG_SPECIAL_INFO`; Thistle Tea's `Empathy.visible_to?`).
//!
//! Each observer gets one block of a unit an update (`update::Batch`'s
//! rule), built once for all who see it alike.

use ahash::AHashMap;
use wow_world_messages::Guid;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras::{AURA_EMPATHY, AURA_MOD_STALKED, Auras, DYNFLAG_TRACK_UNIT};
use crate::game::map::creature::Creature;
use crate::game::map::loot::DYNFLAG_LOOTABLE;
use crate::game::map::observers::ClientTable;
use crate::game::map::player::Player;
use crate::game::map::values::Shared;
use crate::game::update::{self, Batch};

/// Those who see a unit otherwise than a stranger does, but for its tap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Viewers {
    /// A creature's master, a player: its charmer, else its owner
    /// (mangos's `GetMasterGuid`).
    pub master: Option<Guid>,
    /// Whose Hunter's Marks (`SPELL_AURA_MOD_STALKED`) are on it.
    pub markers: Vec<Guid>,
    /// Whose Empathy auras are on it (a creature's).
    pub empathic: Vec<Guid>,
    /// Who may open a lootable corpse's loot, where not everyone may.
    pub loot: Option<LootRights>,
}

/// Who may open a corpse's loot (`Loot::may_loot`): its owner alone, or
/// the group's members near at the kill, and not one with nothing in it
/// of theirs (a quest drop not for their quests, the items the current
/// looter hasn't let go). Another sees no loot on it (mangos's `Loot::CanLoot` in
/// its per-viewer `UNIT_DYNFLAG_LOOTABLE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LootRights {
    owner: Option<Guid>,
    allowed: Vec<Guid>,
    /// Those of them with nothing in it of theirs (`Loot::quiet`).
    quiet: Vec<Guid>,
}

impl LootRights {
    fn of(c: &Creature) -> Option<Self> {
        let l = c.loot.as_ref()?;
        if c.fields.get_u32(UNIT_DYNAMIC_FLAGS) & DYNFLAG_LOOTABLE == 0
            || (l.owner.is_none() && l.allowed.is_empty())
        {
            return None;
        }
        let mut allowed = l.allowed.clone();
        allowed.sort_unstable();
        Some(Self {
            owner: l.owner,
            allowed,
            quiet: l.quiet.clone(),
        })
    }

    /// Whether `g` may not open it, or would find nothing of theirs.
    fn bars(&self, g: Guid) -> bool {
        if self.quiet.contains(&g) {
            return true;
        }
        if !self.allowed.is_empty() {
            return !self.allowed.contains(&g);
        }
        self.owner.is_some_and(|o| o != g)
    }
}

impl Viewers {
    pub fn is_empty(&self) -> bool {
        self.master.is_none()
            && self.markers.is_empty()
            && self.empathic.is_empty()
            && self.loot.is_none()
    }

    pub fn of_creature(c: &Creature) -> Self {
        let charmer = c.fields.get_guid(UNIT_FIELD_CHARMEDBY);
        let master = if charmer.is_zero() {
            c.owner
        } else {
            Some(charmer)
        }
        .filter(|m| guid::is_player(*m));
        Self {
            master,
            markers: casters(&c.auras, AURA_MOD_STALKED),
            empathic: casters(&c.auras, AURA_EMPATHY),
            loot: LootRights::of(c),
        }
    }

    /// A player's: only their markers (their own fields go to them alone,
    /// whatever their charmer sees).
    pub fn of_player(p: &Player) -> Self {
        Self {
            markers: casters(&p.auras, AURA_MOD_STALKED),
            ..Default::default()
        }
    }
}

fn casters(auras: &Auras, kind: u32) -> Vec<Guid> {
    let mut out: Vec<Guid> = auras
        .holders
        .iter()
        .filter(|h| h.has(kind))
        .map(|h| h.caster)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// One observer's view of a unit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct View {
    /// Outside its tap.
    pub grey: bool,
    /// Its loot isn't theirs to open.
    pub barred: bool,
    /// Its Hunter's Mark is theirs.
    pub marker: bool,
    /// Its master.
    pub master: bool,
    /// Their Empathy is on it.
    pub empathy: bool,
    /// A player's group's member, with quests in their log to show.
    pub group: bool,
}

impl View {
    fn visibility(self) -> Visibility {
        if self.master {
            Visibility::Master
        } else if self.empathy {
            Visibility::Empathy
        } else if self.group {
            Visibility::Group
        } else {
            Visibility::Public
        }
    }

    /// The dynamic flags it shows of `raw`, the unit's own.
    pub fn flags(self, raw: u32) -> u32 {
        let mut flags = raw;
        if self.grey {
            flags = crate::game::map::tap::grey(flags);
        }
        if self.barred {
            flags &= !DYNFLAG_LOOTABLE;
        }
        if !self.marker {
            flags &= !DYNFLAG_TRACK_UNIT;
        }
        flags
    }

    /// The ranges of fields it sees and a stranger doesn't.
    fn extra(self) -> &'static [(u16, u16)] {
        if self.master {
            UNIT_MASTER_ONLY
        } else if self.empathy {
            UNIT_SPECIAL_INFO
        } else {
            &[]
        }
    }
}

/// `observer`'s view of `object`, a unit or a player.
pub fn view_of(map: &Map, observer: Guid, object: Guid) -> View {
    if let Some(p) = map.players.get(&object) {
        return View {
            marker: p
                .auras
                .holders
                .iter()
                .any(|h| h.caster == observer && h.has(AURA_MOD_STALKED)),
            group: has_group_only(&p.fields)
                && crate::game::map::objects::same_group(map, observer, object),
            ..Default::default()
        };
    }
    let Some(c) = map.creature(object) else {
        return View::default();
    };
    let viewers = Viewers::of_creature(c);
    View {
        grey: !crate::game::map::tap::holds(map, c, observer),
        ..creature_view(&viewers, observer)
    }
}

fn creature_view(viewers: &Viewers, observer: Guid) -> View {
    View {
        barred: viewers.loot.as_ref().is_some_and(|l| l.bars(observer)),
        marker: viewers.markers.contains(&observer),
        master: viewers.master == Some(observer),
        empathy: viewers.empathic.contains(&observer),
        ..Default::default()
    }
}

/// Whether a stranger's view of `fields` is them as they are: no tracking
/// flag to take off.
fn plain(fields: &Fields) -> bool {
    fields.get_u32(UNIT_DYNAMIC_FLAGS) & DYNFLAG_TRACK_UNIT == 0
}

/// `object`'s create for `observer`: as a stranger's (`Map::public_create`,
/// built once an update), or with their own view in it.
pub fn create_for(map: &mut Map, observer: Guid, object: Guid) -> Option<std::sync::Arc<[u8]>> {
    let view = view_of(map, observer, object);
    if view == View::default() {
        return map.public_create(object);
    }
    let now = map.now;
    let (create, fields) = if let Some(p) = map.players.get(&object) {
        (p.create_for_others(now), &p.fields)
    } else {
        let c = map.creature(object)?;
        (c.create(now), &c.fields)
    };
    let mut fields = fields.clone();
    fields.set_u32(
        UNIT_DYNAMIC_FLAGS,
        view.flags(fields.get_u32(UNIT_DYNAMIC_FLAGS)),
    );
    let create = update::Create {
        fields: &fields,
        ..create
    };
    Some(std::sync::Arc::from(update::create_block(
        &create,
        view.visibility(),
    )))
}

/// A unit's or player's create as a stranger sees it: its tracking flag
/// off.
pub fn stranger_create(c: update::Create) -> Vec<u8> {
    if plain(c.fields) {
        return update::create_block(&c, Visibility::Public);
    }
    let mut fields = c.fields.clone();
    fields.set_u32(
        UNIT_DYNAMIC_FLAGS,
        View::default().flags(fields.get_u32(UNIT_DYNAMIC_FLAGS)),
    );
    update::create_block(
        &update::Create {
            fields: &fields,
            ..c
        },
        Visibility::Public,
    )
}

/// The values block of `fields` for `view`: their changes it sees; with
/// `told` (who sees it otherwise has changed), the dynamic flags as it
/// sees them, changed or not; with `fresh` (it has just come to see more
/// of them: a pet's master, an Empathy's caster), what more it sees, as
/// mangos's `HandleAuraEmpathy` sends its caster.
fn block(guid: Guid, fields: &Fields, view: View, told: bool, fresh: bool) -> Option<Vec<u8>> {
    let raw = fields.get_u32(UNIT_DYNAMIC_FLAGS);
    let flags = view.flags(raw);
    let send_flags = told || fields.dirty().get(UNIT_DYNAMIC_FLAGS);
    // Its changes as they are: no flags of its own to send, nor more.
    if !fresh && !(send_flags && (told || flags != raw)) {
        return update::values_block(guid, fields, view.visibility());
    }
    let mut f = fields.clone();
    if send_flags {
        f.set_u32(UNIT_DYNAMIC_FLAGS, flags);
        f.touch(UNIT_DYNAMIC_FLAGS);
    }
    if fresh {
        for &(from, to) in view.extra() {
            for i in from..to {
                if f.get_u32(i) != 0 {
                    f.touch(i);
                }
            }
        }
    }
    update::values_block(guid, &f, view.visibility())
}

/// Each observer's block of `guid`'s `fields` this update, `view` giving
/// each theirs (and whether they are `fresh` to it), built once a view.
fn tell_each(
    batches: &mut AHashMap<Guid, Batch>,
    guid: Guid,
    fields: &Fields,
    observers: impl Iterator<Item = Guid>,
    told: bool,
    view: impl Fn(Guid) -> (View, bool),
) {
    type Built = ((View, bool), Option<Vec<u8>>);
    let mut built: Vec<Built> = Vec::new();
    for o in observers {
        let key = view(o);
        let i = match built.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                built.push((key, block(guid, fields, key.0, told, key.1)));
                built.len() - 1
            }
        };
        if let Some(b) = &built[i].1 {
            batches.entry(o).or_default().push_for(guid, b);
        }
    }
}

/// A creature's values this update, each observer's own view in them
/// (`tap_changed`: its tap taken or let go). The viewers it was told by
/// are `c.views_told`; the ones now come back.
pub fn tell_creature(
    realm: &Realm,
    batches: &mut AHashMap<Guid, Batch>,
    shared: &mut Shared,
    table: &ClientTable,
    c: &Creature,
    tap_changed: bool,
) -> Viewers {
    let viewers = Viewers::of_creature(c);
    let told = tap_changed || viewers != c.views_told;
    let flags_sent = c.fields.dirty().get(UNIT_DYNAMIC_FLAGS);
    // Each sees it otherwise only if who sees it otherwise changed, its
    // flags go to some otherwise than they are, or what only its master
    // or an Empathy sees changed: else the one block for all.
    let personal = told
        || (flags_sent && (c.tapped_by.is_some() || viewers.loot.is_some() || !plain(&c.fields)))
        || ((viewers.master.is_some() || !viewers.empathic.is_empty())
            && owner_only_dirty(&c.fields));
    if !personal {
        // The one block for all, shared among them (`values`).
        if let Some(b) = update::values_block(c.guid, &c.fields, Visibility::Public) {
            shared.tell(c.guid, None, c.cell, b, c.observers.clients(table));
        }
        return viewers;
    }
    let holders = c
        .tapped_by
        .map(|t| crate::game::map::tap::recipients(realm, t, c.tap_group));
    let was = &c.views_told;
    tell_each(
        batches,
        c.guid,
        &c.fields,
        c.observers.iter().copied(),
        told,
        |o| {
            let view = View {
                grey: holders.as_ref().is_some_and(|h| !h.contains(&o)),
                ..creature_view(&viewers, o)
            };
            let fresh = (view.master && was.master != Some(o))
                || (view.empathy && !was.empathic.contains(&o));
            (view, fresh)
        },
    );
    viewers
}

/// Whether a field only a creature's master or an Empathy sees changed.
fn owner_only_dirty(f: &Fields) -> bool {
    UNIT_MASTER_ONLY
        .iter()
        .any(|&(from, to)| (from..to).any(|i| f.dirty().get(i)))
}

/// A player's values this update for those who see them, `mates` their
/// group's members when a quest log id changed: each's own view of their
/// tracking flag in them. The markers they were told by are
/// `p.views_told`; the ones now come back.
pub fn tell_player(
    batches: &mut AHashMap<Guid, Batch>,
    shared: &mut Shared,
    table: &ClientTable,
    p: &Player,
    mates: &[Guid],
) -> Viewers {
    let viewers = Viewers::of_player(p);
    let group = !mates.is_empty();
    let told = viewers != p.views_told;
    let flags_sent = p.fields.dirty().get(UNIT_DYNAMIC_FLAGS);
    if !told && !(flags_sent && !plain(&p.fields)) {
        let public = update::values_block(p.guid, &p.fields, Visibility::Public);
        let grouped = group
            .then(|| update::values_block(p.guid, &p.fields, Visibility::Group))
            .flatten();
        // The public block shared among those who get it (`values`); the
        // group's, to each of its members who sees them.
        if let Some(b) = public {
            let mut to = p.observers.clients(table);
            if group {
                let mut but = (*to).clone();
                for m in mates {
                    if let Some(i) = table.get(*m) {
                        but.remove(i);
                    }
                }
                to = std::sync::Arc::new(but);
            }
            shared.tell(p.guid, table.get(p.guid), p.cell, b, to);
        }
        if let Some(b) = &grouped {
            for o in mates.iter().filter(|m| p.observers.contains(*m)) {
                batches.entry(*o).or_default().push(b);
            }
        }
        return viewers;
    }
    tell_each(
        batches,
        p.guid,
        &p.fields,
        p.observers.iter().copied(),
        told,
        |o| {
            (
                View {
                    marker: viewers.markers.contains(&o),
                    group: mates.contains(&o),
                    ..Default::default()
                },
                false,
            )
        },
    );
    viewers
}
