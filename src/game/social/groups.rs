//! Groups and raids, as mangos-zero runs them: invites, the roster and its
//! leader, assistants and subgroups, the loot method, ready checks, target
//! icons, rolls and pings, and the stats each member's map sends the
//! members who can't see them.
//!
//! A group is kept in the store (a record of kind [`KIND_GROUP`]) as its
//! roster changes, and comes back at start-up, as mangos's `groups` and
//! `group_member` tables do: its members, leader, assistants, subgroups,
//! loot rules and icons. Invitations aren't kept.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use ahash::AHashMap;
use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::conn::encode;
use crate::game::data::items::Body;
use crate::game::registry::{Online, Team};
use crate::game::social::{not_ignoring, send_frame, send_frames, send_to};

pub const SMSG_PARTY_MEMBER_STATS: u16 = 0x07E;
pub const SMSG_PARTY_MEMBER_STATS_FULL: u16 = 0x2F2;
pub const MSG_RAID_TARGET_UPDATE: u16 = 0x321;
pub const MSG_RAID_READY_CHECK: u16 = 0x322;

/// A group's record in the store.
pub const KIND_GROUP: &str = "group";

pub const PARTY_SIZE: usize = 5;
pub const RAID_SIZE: usize = 40;
const SUBGROUPS: u8 = 8;

// Member stats flags.
pub const STAT_STATUS: u32 = 0x001;
pub const STAT_CUR_HP: u32 = 0x002;
pub const STAT_MAX_HP: u32 = 0x004;
pub const STAT_POWER_TYPE: u32 = 0x008;
pub const STAT_CUR_POWER: u32 = 0x010;
pub const STAT_MAX_POWER: u32 = 0x020;
pub const STAT_LEVEL: u32 = 0x040;
pub const STAT_ZONE: u32 = 0x080;
pub const STAT_POSITION: u32 = 0x100;
pub const STAT_AURAS: u32 = 0x200;
pub const STAT_AURAS_2: u32 = 0x400;
pub const STAT_PLAYER: u32 = 0x7FF;
// A member's pet's (mangos's `GROUP_UPDATE_FLAG_PET_*`).
pub const STAT_PET_GUID: u32 = 0x0800;
pub const STAT_PET_NAME: u32 = 0x1000;
pub const STAT_PET_MODEL: u32 = 0x2000;
pub const STAT_PET_CUR_HP: u32 = 0x4000;
pub const STAT_PET_MAX_HP: u32 = 0x8000;
pub const STAT_PET_POWER_TYPE: u32 = 0x1_0000;
pub const STAT_PET_CUR_POWER: u32 = 0x2_0000;
pub const STAT_PET_MAX_POWER: u32 = 0x4_0000;
pub const STAT_PET_AURAS: u32 = 0x8_0000;
pub const STAT_PET_AURAS_2: u32 = 0x10_0000;
pub const STAT_PET: u32 = 0x1F_F800;

// Member status bits.
pub const STATUS_ONLINE: u8 = 0x1;
pub const STATUS_PVP: u8 = 0x2;
pub const STATUS_PVP_FFA: u8 = 0x10;
pub const STATUS_DEAD: u8 = 0x4;
pub const STATUS_GHOST: u8 = 0x8;
pub const STATUS_AFK: u8 = 0x40;
pub const STATUS_DND: u8 = 0x80;

#[derive(Debug, Clone)]
pub struct Slot {
    pub guid: Guid,
    pub name: Arc<str>,
    pub subgroup: u8,
    pub assistant: bool,
}

#[derive(Debug, Clone)]
pub struct Group {
    pub id: u32,
    pub raid: bool,
    pub leader: Guid,
    pub members: Vec<Slot>,
    /// Formed when the first invite is taken.
    pub formed: bool,
    pub invited: Vec<Guid>,
    pub loot_method: GroupLootSetting,
    pub looter: Guid,
    pub threshold: ItemQuality,
    pub icons: [Guid; 8],
    /// Round robin's last looter.
    pub round_robin: Guid,
    /// Since when (Unix seconds) the leader has been offline, as far as
    /// the sweep knows (mangos's `m_leaderLastOnline`); `None` while they
    /// are on.
    pub leader_away_since: Option<i64>,
}

/// A group as the store keeps it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct GroupRecord {
    raid: bool,
    leader: u64,
    members: Vec<SlotRecord>,
    loot_method: u8,
    looter: u64,
    threshold: u8,
    icons: [u64; 8],
    round_robin: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct SlotRecord {
    guid: u64,
    name: String,
    subgroup: u8,
    assistant: bool,
}

impl Group {
    fn new(id: u32, leader: &Online) -> Self {
        Self {
            id,
            raid: false,
            leader: leader.guid,
            members: vec![Slot {
                guid: leader.guid,
                name: leader.name.clone(),
                subgroup: 0,
                assistant: false,
            }],
            formed: false,
            invited: Vec::new(),
            loot_method: GroupLootSetting::GroupLoot,
            looter: Guid::zero(),
            threshold: ItemQuality::Uncommon,
            icons: [Guid::zero(); 8],
            round_robin: Guid::zero(),
            leader_away_since: None,
        }
    }

    fn record(&self) -> GroupRecord {
        GroupRecord {
            raid: self.raid,
            leader: self.leader.guid(),
            members: self
                .members
                .iter()
                .map(|m| SlotRecord {
                    guid: m.guid.guid(),
                    name: m.name.to_string(),
                    subgroup: m.subgroup,
                    assistant: m.assistant,
                })
                .collect(),
            loot_method: self.loot_method.as_int(),
            looter: self.looter.guid(),
            threshold: self.threshold.as_int(),
            icons: self.icons.map(|g| g.guid()),
            round_robin: self.round_robin.guid(),
        }
    }

    /// The group a record kept, if it is one: two members at least, its
    /// leader among them.
    fn from_record(id: u32, r: GroupRecord, now: i64) -> Option<Self> {
        let members: Vec<Slot> = r
            .members
            .into_iter()
            .filter(|m| m.guid != 0)
            .map(|m| Slot {
                guid: Guid::new(m.guid),
                name: Arc::from(m.name),
                subgroup: m.subgroup.min(SUBGROUPS - 1),
                assistant: m.assistant,
            })
            .collect();
        let leader = Guid::new(r.leader);
        if members.len() < 2 || !members.iter().any(|m| m.guid == leader) {
            return None;
        }
        let looter = Guid::new(r.looter);
        Some(Self {
            id,
            raid: r.raid,
            leader,
            looter: if members.iter().any(|m| m.guid == looter) {
                looter
            } else {
                leader
            },
            members,
            formed: true,
            invited: Vec::new(),
            loot_method: GroupLootSetting::try_from(r.loot_method)
                .unwrap_or(GroupLootSetting::GroupLoot),
            threshold: threshold(
                ItemQuality::try_from(r.threshold).unwrap_or(ItemQuality::Uncommon),
            ),
            icons: r.icons.map(Guid::new),
            round_robin: Guid::new(r.round_robin),
            // Everyone is offline at start-up: the leader's five minutes
            // start now.
            leader_away_since: Some(now),
        })
    }

    /// Kept in the store as it is now.
    fn save(&self, realm: &Realm) {
        if self.formed {
            realm
                .store
                .put_record(KIND_GROUP, u64::from(self.id), &self.record());
        }
    }

    /// Who leads next, as mangos's `Group::_chooseLeader`: the first
    /// member online (in a raid, the first assistant online before them),
    /// else (unless `offline`, the leader replaced for being away, when
    /// no one is) the first member.
    fn next_leader(&self, realm: &Realm, offline: bool) -> Option<Guid> {
        let mut first_online = None;
        for m in self.members.iter().filter(|m| m.guid != self.leader) {
            if realm.registry.online(m.guid).is_none() {
                continue;
            }
            if self.raid && !m.assistant {
                first_online.get_or_insert(m.guid);
                continue;
            }
            return Some(m.guid);
        }
        first_online.or_else(|| {
            (!offline)
                .then(|| self.members.iter().find(|m| m.guid != self.leader))
                .flatten()
                .map(|m| m.guid)
        })
    }

    pub fn is_member(&self, g: Guid) -> bool {
        self.members.iter().any(|m| m.guid == g)
    }

    fn slot(&self, g: Guid) -> Option<&Slot> {
        self.members.iter().find(|m| m.guid == g)
    }

    fn can_lead(&self, g: Guid) -> bool {
        self.leader == g || self.slot(g).is_some_and(|s| s.assistant)
    }

    fn full(&self) -> bool {
        self.members.len() >= if self.raid { RAID_SIZE } else { PARTY_SIZE }
    }

    fn free_subgroup(&self) -> u8 {
        if !self.raid {
            return 0;
        }
        (0..SUBGROUPS)
            .find(|n| self.members.iter().filter(|m| m.subgroup == *n).count() < PARTY_SIZE)
            .unwrap_or(0)
    }

    fn broadcast(&self, realm: &Realm, frame: &Arc<[u8]>, except: Option<Guid>) {
        for m in &self.members {
            if Some(m.guid) != except {
                send_frame(realm, m.guid, frame);
            }
        }
    }

    /// `SMSG_GROUP_LIST` to each member online: everyone else, the leader
    /// and the loot rules.
    fn send_update(&self, realm: &Realm) {
        for me in &self.members {
            if realm.registry.online(me.guid).is_none() {
                continue;
            }
            set_leader_flag(realm, me.guid, me.guid == self.leader);
            if let Some(frame) = list_frame(&self.list_for(realm, me)) {
                send_frame(realm, me.guid, &frame);
            }
        }
    }

    fn list_for(&self, realm: &Realm, me: &Slot) -> SMSG_GROUP_LIST {
        let flags = |s: &Slot| s.subgroup | if s.assistant { 0x80 } else { 0 };
        let members: Vec<GroupListMember> = self
            .members
            .iter()
            .filter(|m| m.guid != me.guid)
            .map(|m| GroupListMember {
                name: m.name.to_string(),
                guid: m.guid,
                is_online: realm.registry.online(m.guid).is_some(),
                flags: flags(m),
            })
            .collect();
        let group_not_empty = (!members.is_empty()).then(|| SMSG_GROUP_LIST_group_not_empty {
            loot_setting: self.loot_method,
            master_loot: if self.loot_method == GroupLootSetting::MasterLoot {
                self.looter
            } else {
                Guid::zero()
            },
            loot_threshold: self.threshold,
        });
        SMSG_GROUP_LIST {
            group_type: if self.raid {
                GroupType::Raid
            } else {
                GroupType::Normal
            },
            flags: flags(me),
            members,
            leader: self.leader,
            group_not_empty,
        }
    }
}

/// `SMSG_GROUP_LIST` as Thistle Tea and mangos-classic end it: past the
/// loot rules, a byte for the dungeon difficulty (0, there being none in
/// 1.12), which wow_messages' layout and mangos-zero leave off.
fn list_frame(list: &SMSG_GROUP_LIST) -> Option<Arc<[u8]>> {
    let frame = encode(list)?;
    if list.group_not_empty.is_none() {
        return Some(frame);
    }
    let mut body = frame[4..].to_vec();
    body.push(0);
    Some(crate::game::update::frame(SMSG_GROUP_LIST_OPCODE, &body))
}

const SMSG_GROUP_LIST_OPCODE: u16 = 0x07D;

/// `PLAYER_FLAGS_GROUP_LEADER` on `g`, in whatever map they are: on for
/// the group's leader, off for everyone else, as each is sent the roster
/// (mangos's `UpdateGroupLeaderFlag`, Thistle Tea's `set_group_leader`).
/// What they see is worked out again: a group sees its ghosts.
fn set_leader_flag(realm: &Realm, g: Guid, leader: bool) {
    use crate::game::fields::PLAYER_FLAGS;
    use crate::game::map::player::PLAYER_FLAGS_GROUP_LEADER;
    crate::game::map::call_player(realm, g, move |m| {
        if let Some(p) = m.players.get_mut(&g) {
            p.fields
                .set_flag(PLAYER_FLAGS, PLAYER_FLAGS_GROUP_LEADER, leader);
            m.visibility_dirty.insert(g);
        }
    });
}

/// A player out of their group: no one on their roster, and no lead.
fn leave_roster(realm: &Realm, g: Guid) {
    set_leader_flag(realm, g, false);
    send_to(realm, g, &empty_list());
}

/// The roster a player out of a group gets: no one.
fn empty_list() -> SMSG_GROUP_LIST {
    SMSG_GROUP_LIST {
        group_type: GroupType::Normal,
        flags: 0,
        members: Vec::new(),
        leader: Guid::zero(),
        group_not_empty: None,
    }
}

/// A member's pet's stats (or the unit they charm), as mangos's
/// `BuildPartyMemberStatsChangedPacket` sends them.
#[derive(Debug, Clone, PartialEq)]
pub struct PetStats {
    pub guid: Guid,
    pub name: Arc<str>,
    pub model: u32,
    pub health: u32,
    pub max_health: u32,
    pub power_type: u8,
    pub power: u32,
    pub max_power: u32,
    pub auras: [u32; 48],
}

/// A member's stats as their map last sent them.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberStats {
    pub status: u8,
    pub health: u32,
    pub max_health: u32,
    pub power_type: u8,
    pub power: u32,
    pub max_power: u32,
    pub level: u32,
    pub zone: u32,
    pub x: f32,
    pub y: f32,
    pub auras: [u32; 48],
    pub pet: Option<PetStats>,
}

/// An aura mask and its spells, as 1.12 lays them out: a `u32` mask of the
/// first 32 slots, or a `u16` of the 16 past them, then each spell.
fn write_auras(b: &mut Body, auras: &[u32], wide: bool) {
    let bits = (0..auras.len())
        .filter(|i| auras[*i] != 0)
        .fold(0u32, |m, i| m | 1 << i);
    if wide {
        b.u32(bits);
    } else {
        b.u16(bits as u16);
    }
    for a in auras.iter().filter(|a| **a != 0) {
        b.u16(*a as u16);
    }
}

impl MemberStats {
    /// Every stat it has: the player's, and its pet's if one is out.
    pub fn all(&self) -> u32 {
        STAT_PLAYER | if self.pet.is_some() { STAT_PET } else { 0 }
    }

    /// What changed from `old`, as stats flags.
    pub fn changed(&self, old: &MemberStats) -> u32 {
        let mut mask = 0;
        let mut set = |c: bool, f: u32| {
            if c {
                mask |= f;
            }
        };
        set(self.status != old.status, STAT_STATUS);
        set(self.health != old.health, STAT_CUR_HP);
        set(self.max_health != old.max_health, STAT_MAX_HP);
        set(
            self.power_type != old.power_type,
            STAT_POWER_TYPE | STAT_CUR_POWER | STAT_MAX_POWER,
        );
        set(self.power != old.power, STAT_CUR_POWER);
        set(self.max_power != old.max_power, STAT_MAX_POWER);
        set(self.level != old.level, STAT_LEVEL);
        set(self.zone != old.zone, STAT_ZONE);
        set(
            (self.x - old.x).abs() >= 1.0 || (self.y - old.y).abs() >= 1.0,
            STAT_POSITION,
        );
        set(self.auras[..32] != old.auras[..32], STAT_AURAS);
        set(self.auras[32..] != old.auras[32..], STAT_AURAS_2);
        match (&self.pet, &old.pet) {
            (Some(p), Some(o)) if p.guid == o.guid => {
                set(p.name != o.name, STAT_PET_NAME);
                set(p.model != o.model, STAT_PET_MODEL);
                set(p.health != o.health, STAT_PET_CUR_HP);
                set(p.max_health != o.max_health, STAT_PET_MAX_HP);
                set(
                    p.power_type != o.power_type,
                    STAT_PET_POWER_TYPE | STAT_PET_CUR_POWER | STAT_PET_MAX_POWER,
                );
                set(p.power != o.power, STAT_PET_CUR_POWER);
                set(p.max_power != o.max_power, STAT_PET_MAX_POWER);
                set(p.auras[..32] != o.auras[..32], STAT_PET_AURAS);
                set(p.auras[32..] != o.auras[32..], STAT_PET_AURAS_2);
            }
            (None, None) => {}
            // A pet out, gone or another: all of it (zeros for none).
            _ => mask |= STAT_PET,
        }
        mask
    }

    /// `SMSG_PARTY_MEMBER_STATS` (or its full form) with the fields in
    /// `mask`, in 1.12's layout.
    pub fn packet(&self, g: Guid, mask: u32, full: bool) -> Arc<[u8]> {
        let mut b = Body::default();
        crate::game::update::write_packed_guid(&mut b.0, g);
        b.u32(mask);
        if mask & STAT_STATUS != 0 {
            b.u8(self.status);
        }
        if mask & STAT_CUR_HP != 0 {
            b.u16(self.health.min(0xFFFF) as u16);
        }
        if mask & STAT_MAX_HP != 0 {
            b.u16(self.max_health.min(0xFFFF) as u16);
        }
        if mask & STAT_POWER_TYPE != 0 {
            b.u8(self.power_type);
        }
        if mask & STAT_CUR_POWER != 0 {
            b.u16(self.power.min(0xFFFF) as u16);
        }
        if mask & STAT_MAX_POWER != 0 {
            b.u16(self.max_power.min(0xFFFF) as u16);
        }
        if mask & STAT_LEVEL != 0 {
            b.u16(self.level as u16);
        }
        if mask & STAT_ZONE != 0 {
            b.u16(self.zone as u16);
        }
        if mask & STAT_POSITION != 0 {
            b.u16(self.x as i16 as u16).u16(self.y as i16 as u16);
        }
        if mask & STAT_AURAS != 0 {
            write_auras(&mut b, &self.auras[..32], true);
        }
        if mask & STAT_AURAS_2 != 0 {
            write_auras(&mut b, &self.auras[32..], false);
        }
        // The pet's, zeros for none (mangos's: a pet gone is told so).
        let pet = self.pet.as_ref();
        let none = [0u32; 48];
        if mask & STAT_PET_GUID != 0 {
            b.u64(pet.map_or(0, |p| p.guid.guid()));
        }
        if mask & STAT_PET_NAME != 0 {
            b.cstr(pet.map_or("", |p| &p.name));
        }
        if mask & STAT_PET_MODEL != 0 {
            b.u16(pet.map_or(0, |p| p.model) as u16);
        }
        if mask & STAT_PET_CUR_HP != 0 {
            b.u16(pet.map_or(0, |p| p.health.min(0xFFFF)) as u16);
        }
        if mask & STAT_PET_MAX_HP != 0 {
            b.u16(pet.map_or(0, |p| p.max_health.min(0xFFFF)) as u16);
        }
        if mask & STAT_PET_POWER_TYPE != 0 {
            b.u8(pet.map_or(0, |p| p.power_type));
        }
        if mask & STAT_PET_CUR_POWER != 0 {
            b.u16(pet.map_or(0, |p| p.power.min(0xFFFF)) as u16);
        }
        if mask & STAT_PET_MAX_POWER != 0 {
            b.u16(pet.map_or(0, |p| p.max_power.min(0xFFFF)) as u16);
        }
        let pet_auras = pet.map_or(&none, |p| &p.auras);
        if mask & STAT_PET_AURAS != 0 {
            write_auras(&mut b, &pet_auras[..32], true);
        }
        if mask & STAT_PET_AURAS_2 != 0 {
            write_auras(&mut b, &pet_auras[32..], false);
        }
        b.frame(if full {
            SMSG_PARTY_MEMBER_STATS_FULL
        } else {
            SMSG_PARTY_MEMBER_STATS
        })
    }
}

#[derive(Debug, Default)]
pub struct Groups {
    next_id: u32,
    groups: AHashMap<u32, Group>,
    /// Members (and the leaders of groups not yet formed) to their group.
    of: AHashMap<Guid, u32>,
    /// Those invited, to the group they are invited to.
    invites: AHashMap<Guid, u32>,
    /// Each member's stats, as last sent.
    pub stats: AHashMap<Guid, MemberStats>,
    /// The members who could see each member at the last push: one who
    /// no longer can gets all of the stats.
    pub seen_by: AHashMap<Guid, Vec<Guid>>,
}

impl Groups {
    /// The groups kept in the store.
    pub fn load(records: &[(String, u64, String)]) -> Self {
        let mut g = Self::default();
        let now = now();
        for (_, id, data) in records.iter().filter(|(k, _, _)| k == KIND_GROUP) {
            // Every id counts, read or not (`Mails::load`).
            let id = *id as u32;
            g.next_id = g.next_id.max(id);
            let Some(r) =
                crate::game::store::parse_record::<GroupRecord>(KIND_GROUP, id.into(), data)
            else {
                continue;
            };
            let Some(gr) = Group::from_record(id, r, now) else {
                continue;
            };
            // A member in two (a record written twice over): the first.
            if gr.members.iter().any(|m| g.of.contains_key(&m.guid)) {
                continue;
            }
            for m in &gr.members {
                g.of.insert(m.guid, id);
            }
            g.groups.insert(id, gr);
        }
        g
    }

    pub fn group_of(&self, g: Guid) -> Option<&Group> {
        self.of
            .get(&g)
            .and_then(|id| self.groups.get(id))
            .filter(|gr| gr.formed)
    }

    pub fn group_of_mut(&mut self, g: Guid) -> Option<&mut Group> {
        let id = *self.of.get(&g)?;
        self.groups.get_mut(&id).filter(|gr| gr.formed)
    }

    /// Group `id`, while it is together.
    pub fn get(&self, id: u32) -> Option<&Group> {
        self.groups.get(&id).filter(|gr| gr.formed)
    }

    /// Whether group `id` is still together.
    pub fn formed(&self, id: u32) -> bool {
        self.groups.get(&id).is_some_and(|gr| gr.formed)
    }

    pub fn get_mut(&mut self, id: u32) -> Option<&mut Group> {
        self.groups.get_mut(&id).filter(|gr| gr.formed)
    }

    /// The other members of `g`'s group.
    pub fn others(&self, g: Guid) -> Vec<Guid> {
        self.group_of(g)
            .map(|gr| {
                gr.members
                    .iter()
                    .map(|m| m.guid)
                    .filter(|m| *m != g)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn mine(&mut self, g: Guid) -> Option<&mut Group> {
        let id = *self.of.get(&g)?;
        self.groups.get_mut(&id).filter(|gr| gr.formed)
    }

    /// Take an invite back; a group never formed goes when its last one does.
    fn drop_invite(&mut self, g: Guid) {
        let Some(id) = self.invites.remove(&g) else {
            return;
        };
        let Some(gr) = self.groups.get_mut(&id) else {
            return;
        };
        gr.invited.retain(|i| *i != g);
        if !gr.formed && gr.invited.is_empty() {
            let leader = gr.leader;
            self.groups.remove(&id);
            self.of.remove(&leader);
        }
    }

    /// Take a member out. Returns what a meeting stone's queue is to hear
    /// of it, once the lock is let go (`Left::tell`).
    #[must_use]
    fn remove(&mut self, realm: &Realm, g: Guid, kicked: bool) -> Left {
        let Some(&id) = self.of.get(&g) else {
            return Left::Nothing;
        };
        let Some(gr) = self.groups.get_mut(&id) else {
            return Left::Nothing;
        };
        if gr.members.len() <= 2 {
            let members = gr.members.iter().map(|m| m.guid).collect();
            self.disband(realm, id);
            return Left::Disbanded { id, members };
        }
        gr.members.retain(|m| m.guid != g);
        self.of.remove(&g);
        self.stats.remove(&g);
        self.seen_by.remove(&g);
        if kicked {
            send_to(realm, g, &SMSG_GROUP_UNINVITE {});
        }
        leave_roster(realm, g);
        let gr = &mut self.groups.get_mut(&id).unwrap();
        let leader_left = gr.leader == g;
        if leader_left {
            // To one online, an assistant first in a raid (mangos's
            // `_chooseLeader`), not whoever joined first.
            let next = gr.next_leader(realm, false).unwrap_or(gr.members[0].guid);
            gr.leader = next;
            gr.leader_away_since = None;
            if let Some(s) = gr.members.iter_mut().find(|m| m.guid == next) {
                s.assistant = false;
            }
            let name = gr
                .slot(next)
                .map(|s| s.name.to_string())
                .unwrap_or_default();
            if let Some(frame) = encode(&SMSG_GROUP_SET_LEADER { name }) {
                gr.broadcast(realm, &frame, None);
            }
            crate::game::instances::group_changed(realm, Some((id, g)), Some((id, gr.leader)));
        }
        if gr.looter == g {
            gr.looter = gr.leader;
        }
        gr.save(realm);
        gr.send_update(realm);
        Left::Removed {
            id,
            g,
            kicked,
            leader_left,
            rest: gr.members.iter().map(|m| m.guid).collect(),
        }
    }

    fn disband(&mut self, realm: &Realm, id: u32) {
        let Some(gr) = self.groups.remove(&id) else {
            return;
        };
        if gr.formed {
            crate::game::instances::group_changed(realm, Some((id, gr.leader)), None);
            realm.store.delete_record(KIND_GROUP, u64::from(id));
        }
        for m in &gr.members {
            self.of.remove(&m.guid);
            self.stats.remove(&m.guid);
            self.seen_by.remove(&m.guid);
            send_to(realm, m.guid, &SMSG_GROUP_DESTROYED {});
            leave_roster(realm, m.guid);
        }
        for i in gr.invited {
            self.invites.remove(&i);
        }
    }
}

/// A member out of a group, for a meeting stone's queue to follow.
#[derive(Debug)]
enum Left {
    Nothing,
    Removed {
        id: u32,
        g: Guid,
        kicked: bool,
        leader_left: bool,
        rest: Vec<Guid>,
    },
    Disbanded {
        id: u32,
        members: Vec<Guid>,
    },
}

impl Left {
    /// Tell the queue, with the groups' lock let go: a kick queues the one
    /// kicked again, which reads their group.
    fn tell(self, realm: &Realm) {
        use crate::game::map::meeting_stones as stones;
        match self {
            Left::Nothing => {}
            Left::Removed {
                id,
                g,
                kicked,
                leader_left,
                rest,
            } => stones::left_group(realm, id, g, kicked, leader_left, &rest),
            Left::Disbanded { id, members } => stones::group_disbanded(realm, id, &members),
        }
    }
}

/// A loot threshold as 1.12's client offers it, Uncommon to Artifact: a
/// Poor or Common one (a bad packet's) rolls on every grey.
fn threshold(q: ItemQuality) -> ItemQuality {
    q.max(ItemQuality::Uncommon)
}

fn result(realm: &Realm, to: Guid, operation: PartyOperation, member: &str, result: PartyResult) {
    send_to(
        realm,
        to,
        &SMSG_PARTY_COMMAND_RESULT {
            operation,
            member: member.to_owned(),
            result,
        },
    );
}

/// `CMSG_GROUP_INVITE`.
pub fn invite(realm: &Realm, me: &Online, name: &str) {
    let invitee = realm.registry.online_by_name(name);
    let Some(them) = invitee.filter(|o| o.guid != me.guid) else {
        result(
            realm,
            me.guid,
            PartyOperation::Invite,
            name,
            PartyResult::BadPlayerName,
        );
        return;
    };
    if Team::of_race(them.race) != Team::of_race(me.race) && !me.gm {
        result(
            realm,
            me.guid,
            PartyOperation::Invite,
            name,
            PartyResult::PlayerWrongFaction,
        );
        return;
    }
    // In two copies of one dungeon: refused, as mangos's
    // `HandleGroupInviteOpcode` refuses it ("not so appropriated but no
    // other option for classic").
    let (mine, theirs) = (me.map().key, them.map().key);
    if mine.instance != 0
        && theirs.instance != 0
        && mine.instance != theirs.instance
        && mine.map == theirs.map
    {
        result(
            realm,
            me.guid,
            PartyOperation::Invite,
            name,
            PartyResult::AlreadyInGroup,
        );
        return;
    }
    if realm.social.lists.lock().ignores(them.guid, me.guid) {
        result(
            realm,
            me.guid,
            PartyOperation::Invite,
            name,
            PartyResult::IgnoringYou,
        );
        return;
    }
    let mut groups = realm.social.groups.lock();
    if groups.of.contains_key(&them.guid) || groups.invites.contains_key(&them.guid) {
        result(
            realm,
            me.guid,
            PartyOperation::Invite,
            name,
            PartyResult::AlreadyInGroup,
        );
        return;
    }
    let id = match groups.of.get(&me.guid).copied() {
        Some(id) => {
            let gr = &groups.groups[&id];
            if gr.formed && !gr.can_lead(me.guid) {
                result(
                    realm,
                    me.guid,
                    PartyOperation::Invite,
                    "",
                    PartyResult::NotLeader,
                );
                return;
            }
            if gr.full() {
                result(
                    realm,
                    me.guid,
                    PartyOperation::Invite,
                    "",
                    PartyResult::GroupFull,
                );
                return;
            }
            id
        }
        // One with an invitation pending invites for the group they are
        // invited to, which isn't theirs to lead (mangos's
        // `HandleGroupInviteOpcode`, `GetGroupInvite`): refused once it is
        // formed, else nothing.
        None if groups.invites.contains_key(&me.guid) => {
            let formed = groups
                .invites
                .get(&me.guid)
                .and_then(|id| groups.groups.get(id))
                .is_some_and(|gr| gr.formed);
            drop(groups);
            if formed {
                result(
                    realm,
                    me.guid,
                    PartyOperation::Invite,
                    "",
                    PartyResult::NotLeader,
                );
            }
            return;
        }
        None => {
            groups.next_id += 1;
            let id = groups.next_id;
            groups.groups.insert(id, Group::new(id, me));
            groups.of.insert(me.guid, id);
            id
        }
    };
    if let Some(gr) = groups.groups.get_mut(&id) {
        gr.invited.push(them.guid);
    }
    groups.invites.insert(them.guid, id);
    drop(groups);
    them.outbound.send(&SMSG_GROUP_INVITE {
        name: me.name.to_string(),
    });
    result(
        realm,
        me.guid,
        PartyOperation::Invite,
        name,
        PartyResult::Success,
    );
}

/// `CMSG_GROUP_ACCEPT`.
pub fn accept(realm: &Realm, me: &Online) {
    let mut groups = realm.social.groups.lock();
    let Some(&id) = groups.invites.get(&me.guid) else {
        return;
    };
    groups.invites.remove(&me.guid);
    let Some(gr) = groups.groups.get_mut(&id) else {
        return;
    };
    gr.invited.retain(|i| *i != me.guid);
    if gr.full() {
        result(
            realm,
            me.guid,
            PartyOperation::Invite,
            "",
            PartyResult::GroupFull,
        );
        return;
    }
    let forms = !gr.formed;
    gr.formed = true;
    let subgroup = gr.free_subgroup();
    gr.members.push(Slot {
        guid: me.guid,
        name: me.name.clone(),
        subgroup,
        assistant: false,
    });
    if gr.looter.is_zero() {
        gr.looter = gr.leader;
    }
    gr.save(realm);
    gr.send_update(realm);
    if forms {
        crate::game::instances::group_changed(realm, None, Some((id, gr.leader)));
    }
    groups.of.insert(me.guid, id);
    // The newcomer's stats go to everyone at the next push; theirs to them.
    let others = groups.others(me.guid);
    for o in others {
        if let Some(s) = groups.stats.get(&o) {
            me.outbound.send_frame(s.packet(o, s.all(), true));
        }
    }
    groups.stats.remove(&me.guid);
    let members: Vec<Guid> = groups.groups[&id].members.iter().map(|m| m.guid).collect();
    drop(groups);
    crate::game::map::meeting_stones::joined_group(realm, id, me.guid, &members);
}

/// `CMSG_GROUP_DECLINE`.
pub fn decline(realm: &Realm, me: &Online) {
    let mut groups = realm.social.groups.lock();
    let Some(&id) = groups.invites.get(&me.guid) else {
        return;
    };
    let leader = groups.groups.get(&id).map(|g| g.leader);
    groups.drop_invite(me.guid);
    drop(groups);
    if let Some(leader) = leader {
        send_to(
            realm,
            leader,
            &SMSG_GROUP_DECLINE {
                name: me.name.to_string(),
            },
        );
    }
}

/// `CMSG_GROUP_UNINVITE` and `CMSG_GROUP_UNINVITE_GUID`: a member out, or
/// an invite back.
pub fn uninvite(realm: &Realm, me: &Online, target: Option<Guid>, name: &str) {
    let mut groups = realm.social.groups.lock();
    let Some(&id) = groups.of.get(&me.guid) else {
        result(
            realm,
            me.guid,
            PartyOperation::Leave,
            "",
            PartyResult::NotInGroup,
        );
        return;
    };
    let gr = &groups.groups[&id];
    if !gr.can_lead(me.guid) {
        result(
            realm,
            me.guid,
            PartyOperation::Leave,
            "",
            PartyResult::NotLeader,
        );
        return;
    }
    let target = target.or_else(|| {
        gr.members
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(name))
            .map(|m| m.guid)
            .or_else(|| realm.registry.guid_by_name(name))
    });
    let Some(t) = target.filter(|t| *t != me.guid) else {
        result(
            realm,
            me.guid,
            PartyOperation::Leave,
            name,
            PartyResult::TargetNotInGroup,
        );
        return;
    };
    if gr.is_member(t) {
        if gr.leader == t && me.guid != gr.leader {
            result(
                realm,
                me.guid,
                PartyOperation::Leave,
                "",
                PartyResult::NotLeader,
            );
            return;
        }
        let left = groups.remove(realm, t, true);
        drop(groups);
        left.tell(realm);
    } else if gr.invited.contains(&t) {
        groups.drop_invite(t);
    } else {
        result(
            realm,
            me.guid,
            PartyOperation::Leave,
            name,
            PartyResult::TargetNotInGroup,
        );
    }
}

/// A character deleted leaves their group, and an invitation to one.
pub fn character_deleted(realm: &Realm, g: Guid) {
    let mut groups = realm.social.groups.lock();
    groups.drop_invite(g);
    if groups.group_of(g).is_some() {
        let left = groups.remove(realm, g, false);
        drop(groups);
        left.tell(realm);
    }
}

/// `CMSG_GROUP_DISBAND`: leave.
pub fn leave(realm: &Realm, me: &Online) {
    let mut groups = realm.social.groups.lock();
    if groups.group_of(me.guid).is_none() {
        return;
    }
    result(
        realm,
        me.guid,
        PartyOperation::Leave,
        &me.name,
        PartyResult::Success,
    );
    let left = groups.remove(realm, me.guid, false);
    drop(groups);
    left.tell(realm);
}

/// `CMSG_GROUP_SET_LEADER`.
pub fn set_leader(realm: &Realm, me: &Online, to: Guid) {
    let mut groups = realm.social.groups.lock();
    let Some(gr) = groups.mine(me.guid) else {
        return;
    };
    if gr.leader != me.guid || !gr.is_member(to) || realm.registry.online(to).is_none() {
        return;
    }
    let id = change_leader(realm, gr, to);
    drop(groups);
    // A party queued at a meeting stone stays queued (mangos's
    // `ChangeLeader` leaves the LFG queue be).
    crate::game::map::meeting_stones::leader_changed(realm, id, to);
}

/// The lead to `to`, told to everyone, as mangos's `Group::ChangeLeader`.
/// The group's id.
fn change_leader(realm: &Realm, gr: &mut Group, to: Guid) -> u32 {
    let (id, from) = (gr.id, gr.leader);
    gr.leader = to;
    gr.leader_away_since = None;
    if let Some(s) = gr.members.iter_mut().find(|m| m.guid == to) {
        s.assistant = false;
    }
    crate::game::instances::group_changed(realm, Some((id, from)), Some((id, to)));
    let name = gr.slot(to).map(|s| s.name.to_string()).unwrap_or_default();
    if let Some(frame) = encode(&SMSG_GROUP_SET_LEADER { name }) {
        gr.broadcast(realm, &frame, None);
    }
    gr.save(realm);
    gr.send_update(realm);
    id
}

/// Unix seconds now.
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// The sweep: [`replace_offline_leaders`] now.
pub fn sweep(realm: &Realm) {
    replace_offline_leaders(realm, now());
}

/// A leader offline `[players] group_offline_leader_secs` (300) hands the
/// lead to a member online, as mangos's `Group::UpdateOfflineLeader`
/// (`Group.OfflineLeaderDelay`); with no one online, it waits. `now` in
/// Unix seconds.
pub fn replace_offline_leaders(realm: &Realm, now: i64) {
    let delay = realm.config().players.group_offline_leader_secs as i64;
    let mut changed = Vec::new();
    {
        let mut groups = realm.social.groups.lock();
        for gr in groups.groups.values_mut().filter(|gr| gr.formed) {
            if realm.registry.online(gr.leader).is_some() {
                gr.leader_away_since = None;
                continue;
            }
            let since = *gr.leader_away_since.get_or_insert(now);
            if now - since < delay {
                continue;
            }
            if let Some(to) = gr.next_leader(realm, true) {
                changed.push((change_leader(realm, gr, to), to));
            }
        }
    }
    for (id, to) in changed {
        crate::game::map::meeting_stones::leader_changed(realm, id, to);
    }
}

/// `CMSG_LOOT_METHOD`.
pub fn loot_method(realm: &Realm, me: &Online, m: &CMSG_LOOT_METHOD) {
    let mut groups = realm.social.groups.lock();
    let Some(gr) = groups.mine(me.guid) else {
        return;
    };
    if gr.leader != me.guid {
        return;
    }
    gr.loot_method = m.loot_setting;
    gr.looter = if gr.is_member(m.loot_master) {
        m.loot_master
    } else {
        gr.leader
    };
    gr.threshold = threshold(m.loot_threshold);
    gr.save(realm);
    gr.send_update(realm);
}

/// `CMSG_GROUP_RAID_CONVERT`.
pub fn convert_to_raid(realm: &Realm, me: &Online) {
    let mut groups = realm.social.groups.lock();
    let Some(gr) = groups.mine(me.guid) else {
        return;
    };
    if gr.leader != me.guid || gr.members.len() < 2 || gr.raid {
        return;
    }
    result(
        realm,
        me.guid,
        PartyOperation::Invite,
        "",
        PartyResult::Success,
    );
    gr.raid = true;
    gr.save(realm);
    gr.send_update(realm);
    let (id, members): (u32, Vec<Guid>) = (gr.id, gr.members.iter().map(|m| m.guid).collect());
    drop(groups);
    crate::game::map::meeting_stones::raid_converted(realm, id, &members);
}

/// `CMSG_GROUP_CHANGE_SUB_GROUP`.
pub fn change_subgroup(realm: &Realm, me: &Online, name: &str, subgroup: u8) {
    let mut groups = realm.social.groups.lock();
    let Some(gr) = groups.mine(me.guid) else {
        return;
    };
    if subgroup >= SUBGROUPS || !gr.raid || !gr.can_lead(me.guid) {
        return;
    }
    if gr.members.iter().filter(|m| m.subgroup == subgroup).count() >= PARTY_SIZE {
        return;
    }
    let Some(s) = gr
        .members
        .iter_mut()
        .find(|m| m.name.eq_ignore_ascii_case(name))
    else {
        return;
    };
    s.subgroup = subgroup;
    gr.save(realm);
    gr.send_update(realm);
}

/// `CMSG_GROUP_SWAP_SUB_GROUP`: two raid members trade subgroups.
pub fn swap_subgroups(realm: &Realm, me: &Online, a: &str, b: &str) {
    let mut groups = realm.social.groups.lock();
    let Some(gr) = groups.mine(me.guid) else {
        return;
    };
    if !gr.raid || !gr.can_lead(me.guid) {
        return;
    }
    let find = |gr: &Group, n: &str| {
        gr.members
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(n))
    };
    let (Some(i), Some(j)) = (find(gr, a), find(gr, b)) else {
        return;
    };
    let (si, sj) = (gr.members[i].subgroup, gr.members[j].subgroup);
    gr.members[i].subgroup = sj;
    gr.members[j].subgroup = si;
    gr.save(realm);
    gr.send_update(realm);
}

/// Put `member` straight into `leader`'s party (a meeting stone's match),
/// making one if the leader has none. False if either can't.
pub fn join_directly(realm: &Realm, leader: &Online, member: &Online) -> bool {
    let mut groups = realm.social.groups.lock();
    if groups.of.contains_key(&member.guid) || groups.invites.contains_key(&member.guid) {
        return false;
    }
    let id = match groups.of.get(&leader.guid).copied() {
        Some(id) => id,
        None => {
            groups.next_id += 1;
            let id = groups.next_id;
            groups.groups.insert(id, Group::new(id, leader));
            groups.of.insert(leader.guid, id);
            id
        }
    };
    let Some(gr) = groups.groups.get_mut(&id) else {
        return false;
    };
    if gr.full() || gr.raid {
        return false;
    }
    let forms = !gr.formed;
    gr.formed = true;
    let subgroup = gr.free_subgroup();
    gr.members.push(Slot {
        guid: member.guid,
        name: member.name.clone(),
        subgroup,
        assistant: false,
    });
    if gr.looter.is_zero() {
        gr.looter = gr.leader;
    }
    gr.save(realm);
    gr.send_update(realm);
    if forms {
        crate::game::instances::group_changed(realm, None, Some((id, gr.leader)));
    }
    groups.of.insert(member.guid, id);
    groups.stats.remove(&member.guid);
    true
}

/// `CMSG_GROUP_ASSISTANT_LEADER`.
pub fn set_assistant(realm: &Realm, me: &Online, who: Guid, on: bool) {
    let mut groups = realm.social.groups.lock();
    let Some(gr) = groups.mine(me.guid) else {
        return;
    };
    if gr.leader != me.guid || who == me.guid {
        return;
    }
    let Some(s) = gr.members.iter_mut().find(|m| m.guid == who) else {
        return;
    };
    s.assistant = on;
    gr.save(realm);
    gr.send_update(realm);
}

/// `MSG_RAID_READY_CHECK`: the leader asks, the members answer.
pub fn ready_check(realm: &Realm, me: &Online, answer: Option<u8>) {
    let groups = realm.social.groups.lock();
    let Some(gr) = groups.group_of(me.guid) else {
        return;
    };
    match answer {
        None => {
            if !gr.can_lead(me.guid) {
                return;
            }
            // To everyone, its asker too (mangos's `BroadcastPacket`,
            // Thistle Tea's `Notifier.broadcast`): their client shows it.
            if let Some(frame) = encode(&MSG_RAID_READY_CHECK_Server { state_check: None }) {
                gr.broadcast(realm, &frame, None);
            }
            // Those offline can't answer: not ready, the leader hears.
            for m in gr
                .members
                .iter()
                .filter(|m| realm.registry.online(m.guid).is_none())
            {
                if let Some(frame) = encode(&MSG_RAID_READY_CHECK_Server {
                    state_check: Some(MSG_RAID_READY_CHECK_Server_state_check {
                        guid: m.guid,
                        state: 0,
                    }),
                }) {
                    send_frame(realm, gr.leader, &frame);
                }
            }
        }
        Some(state) => {
            let Some(frame) = encode(&MSG_RAID_READY_CHECK_Server {
                state_check: Some(MSG_RAID_READY_CHECK_Server_state_check {
                    guid: me.guid,
                    state,
                }),
            }) else {
                return;
            };
            // To the leader alone, as Thistle Tea answers.
            send_frame(realm, gr.leader, &frame);
        }
    }
}

/// `MSG_RAID_TARGET_UPDATE`: icon `index` on `target`, or (with no
/// index) the list.
pub fn target_icon(realm: &Realm, me: &Online, update: Option<(u8, Guid)>) {
    let mut groups = realm.social.groups.lock();
    let Some(gr) = groups.mine(me.guid) else {
        return;
    };
    let Some((index, target)) = update else {
        let mut b = Body::default();
        b.u8(1);
        for (i, g) in gr.icons.iter().enumerate().filter(|(_, g)| !g.is_zero()) {
            b.u8(i as u8).u64(g.guid());
        }
        send_frame(realm, me.guid, &b.frame(MSG_RAID_TARGET_UPDATE));
        return;
    };
    if index >= 8 || !gr.can_lead(me.guid) {
        return;
    }
    // A target carries one icon.
    for (i, g) in gr.icons.iter_mut().enumerate() {
        if *g == target && !target.is_zero() && i != usize::from(index) {
            *g = Guid::zero();
        }
    }
    gr.icons[usize::from(index)] = target;
    gr.save(realm);
    let mut b = Body::default();
    b.u8(0).u8(index).u64(target.guid());
    gr.broadcast(realm, &b.frame(MSG_RAID_TARGET_UPDATE), None);
}

/// `MSG_RANDOM_ROLL`: to the group, or to the roller alone.
pub fn roll(realm: &Realm, me: &Online, minimum: u32, maximum: u32) {
    if minimum > maximum || maximum > 10_000 {
        return;
    }
    let Some(frame) = encode(&MSG_RANDOM_ROLL_Server {
        minimum,
        maximum,
        actual_roll: rand::rng().random_range(minimum..=maximum),
        guid: me.guid,
    }) else {
        return;
    };
    let groups = realm.social.groups.lock();
    match groups.group_of(me.guid) {
        Some(gr) => gr.broadcast(realm, &frame, None),
        None => {
            me.outbound.send_frame(frame);
        }
    }
}

/// `MSG_MINIMAP_PING`: to the rest of the group.
pub fn ping(realm: &Realm, me: &Online, x: f32, y: f32) {
    let groups = realm.social.groups.lock();
    let Some(gr) = groups.group_of(me.guid) else {
        return;
    };
    if let Some(frame) = encode(&MSG_MINIMAP_PING_Server {
        guid: me.guid,
        position_x: x,
        position_y: y,
    }) {
        gr.broadcast(realm, &frame, Some(me.guid));
    }
}

/// Party, raid and raid-warning chat, to every member who doesn't ignore
/// the speaker. Raid lines need a raid; raid-leader and warning lines a
/// leader or assistant.
pub fn chat(
    realm: &Realm,
    me: &Online,
    kind: CMSG_MESSAGECHAT_ChatType,
    text: &str,
    language: Language,
    tag: PlayerChatTag,
) {
    let groups = realm.social.groups.lock();
    let Some(gr) = groups.group_of(me.guid) else {
        return;
    };
    let chat_type = match kind {
        CMSG_MESSAGECHAT_ChatType::Party => SMSG_MESSAGECHAT_ChatType::Party {
            chat_credit: me.guid,
            speech_bubble_credit: me.guid,
        },
        CMSG_MESSAGECHAT_ChatType::Raid if gr.raid => {
            SMSG_MESSAGECHAT_ChatType::Raid { sender2: me.guid }
        }
        CMSG_MESSAGECHAT_ChatType::RaidLeader if gr.raid && gr.can_lead(me.guid) => {
            SMSG_MESSAGECHAT_ChatType::RaidLeader { sender2: me.guid }
        }
        CMSG_MESSAGECHAT_ChatType::RaidWarning if gr.raid && gr.can_lead(me.guid) => {
            SMSG_MESSAGECHAT_ChatType::RaidWarning { sender2: me.guid }
        }
        _ => return,
    };
    // In a party, raid chat is party chat (as the client sends it).
    let subgroup = gr.slot(me.guid).map(|s| s.subgroup);
    let party_only = matches!(chat_type, SMSG_MESSAGECHAT_ChatType::Party { .. }) && gr.raid;
    let mut hearers: Vec<Guid> = gr
        .members
        .iter()
        .filter(|m| !party_only || Some(m.subgroup) == subgroup)
        .map(|m| m.guid)
        .collect();
    drop(groups);
    let Some(frame) = encode(&SMSG_MESSAGECHAT {
        chat_type,
        language,
        message: text.to_owned(),
        tag,
    }) else {
        return;
    };
    not_ignoring(realm, me.guid, &mut hearers);
    send_frames(realm, &hearers, &frame);
}

/// At login: a member back gets the roster, and the others see them
/// online.
pub fn logged_in(realm: &Realm, g: Guid) {
    let groups = realm.social.groups.lock();
    if let Some(gr) = groups.group_of(g) {
        gr.send_update(realm);
        // And the target markers (Thistle Tea resends them).
        if gr.icons.iter().any(|i| !i.is_zero()) {
            let mut b = Body::default();
            b.u8(1);
            for (i, t) in gr.icons.iter().enumerate().filter(|(_, t)| !t.is_zero()) {
                b.u8(i as u8).u64(t.guid());
            }
            send_frame(realm, g, &b.frame(MSG_RAID_TARGET_UPDATE));
        }
    }
}

/// Going offline: invites go, and the others see the member offline.
pub fn logged_out(realm: &Realm, g: Guid) {
    let mut groups = realm.social.groups.lock();
    groups.drop_invite(g);
    // A group never formed, led by the one going, goes with them.
    if let Some(&id) = groups.of.get(&g)
        && groups.groups.get(&id).is_some_and(|gr| !gr.formed)
    {
        let invited = groups.groups[&id].invited.clone();
        for i in invited {
            groups.drop_invite(i);
        }
        groups.of.remove(&g);
        groups.groups.remove(&id);
    }
    groups.stats.remove(&g);
    groups.seen_by.remove(&g);
    if let Some(gr) = groups.group_of_mut(g) {
        if gr.leader == g {
            gr.leader_away_since = Some(now());
        }
        gr.send_update(realm);
        let offline = MemberStats::default();
        gr.broadcast(realm, &offline.packet(g, STAT_STATUS, false), Some(g));
    }
}

/// `CMSG_REQUEST_PARTY_MEMBER_STATS`: all of a member's stats.
pub fn request_stats(realm: &Realm, me: &Online, of: Guid) {
    let groups = realm.social.groups.lock();
    let frame = match groups.stats.get(&of) {
        Some(s) if realm.registry.online(of).is_some() => s.packet(of, s.all(), true),
        _ => MemberStats::default().packet(of, STAT_STATUS, true),
    };
    me.outbound.send_frame(frame);
}

impl Default for MemberStats {
    fn default() -> Self {
        Self {
            status: 0,
            health: 0,
            max_health: 0,
            power_type: 0,
            power: 0,
            max_power: 0,
            level: 0,
            zone: 0,
            x: 0.0,
            y: 0.0,
            auras: [0; 48],
            pet: None,
        }
    }
}
