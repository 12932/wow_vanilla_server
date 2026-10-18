//! Groups and raids, as mangos-zero runs them: invites, the roster and its
//! leader, assistants and subgroups, the loot method, ready checks, target
//! icons, rolls and pings, and the stats each member's map sends the
//! members who can't see them.

use std::sync::Arc;

use ahash::AHashMap;
use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::conn::encode;
use crate::game::data::items::Body;
use crate::game::registry::{Online, Team};
use crate::game::social::{send_frame, send_to};

pub const SMSG_PARTY_MEMBER_STATS: u16 = 0x07E;
pub const SMSG_PARTY_MEMBER_STATS_FULL: u16 = 0x2F2;
pub const MSG_RAID_TARGET_UPDATE: u16 = 0x321;
pub const MSG_RAID_READY_CHECK: u16 = 0x322;

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

// Member status bits.
pub const STATUS_ONLINE: u8 = 0x1;
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
        }
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
fn set_leader_flag(realm: &Realm, g: Guid, leader: bool) {
    use crate::game::fields::PLAYER_FLAGS;
    use crate::game::map::player::PLAYER_FLAGS_GROUP_LEADER;
    let Some(o) = realm.registry.online(g) else {
        return;
    };
    o.map().call(move |m| {
        if let Some(p) = m.players.get_mut(&g) {
            p.fields
                .set_flag(PLAYER_FLAGS, PLAYER_FLAGS_GROUP_LEADER, leader);
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

/// A member's stats as their map last sent them.
#[derive(Debug, Clone, Copy, PartialEq)]
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
}

impl MemberStats {
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
            let bits = (0..32)
                .filter(|i| self.auras[*i] != 0)
                .fold(0u32, |m, i| m | 1 << i);
            b.u32(bits);
            for a in self.auras[..32].iter().filter(|a| **a != 0) {
                b.u16(*a as u16);
            }
        }
        if mask & STAT_AURAS_2 != 0 {
            let bits = (0..16)
                .filter(|i| self.auras[32 + *i] != 0)
                .fold(0u16, |m, i| m | 1 << i);
            b.u16(bits);
            for a in self.auras[32..].iter().filter(|a| **a != 0) {
                b.u16(*a as u16);
            }
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

    /// Whether group `id` is still together.
    pub fn formed(&self, id: u32) -> bool {
        self.groups.get(&id).is_some_and(|gr| gr.formed)
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

    /// Take a member out. Returns what to tell: the group (if it stays),
    /// and whether it is disbanded.
    fn remove(&mut self, realm: &Realm, g: Guid, kicked: bool) {
        let Some(&id) = self.of.get(&g) else {
            return;
        };
        let Some(gr) = self.groups.get_mut(&id) else {
            return;
        };
        if gr.members.len() <= 2 {
            self.disband(realm, id);
            return;
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
        if gr.leader == g {
            gr.leader = gr.members[0].guid;
            if let Some(frame) = encode(&SMSG_GROUP_SET_LEADER {
                name: gr.members[0].name.to_string(),
            }) {
                gr.broadcast(realm, &frame, None);
            }
            crate::game::instances::group_changed(realm, Some((id, g)), Some((id, gr.leader)));
        }
        if gr.looter == g {
            gr.looter = gr.leader;
        }
        gr.send_update(realm);
    }

    fn disband(&mut self, realm: &Realm, id: u32) {
        let Some(gr) = self.groups.remove(&id) else {
            return;
        };
        if gr.formed {
            crate::game::instances::group_changed(realm, Some((id, gr.leader)), None);
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
    gr.send_update(realm);
    if forms {
        crate::game::instances::group_changed(realm, None, Some((id, gr.leader)));
    }
    groups.of.insert(me.guid, id);
    // The newcomer's stats go to everyone at the next push; theirs to them.
    let others = groups.others(me.guid);
    for o in others {
        if let Some(s) = groups.stats.get(&o) {
            me.outbound.send_frame(s.packet(o, STAT_PLAYER, true));
        }
    }
    groups.stats.remove(&me.guid);
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
        groups.remove(realm, t, true);
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
        groups.remove(realm, g, false);
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
    groups.remove(realm, me.guid, false);
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
    let id = gr.id;
    gr.leader = to;
    if let Some(s) = gr.members.iter_mut().find(|m| m.guid == to) {
        s.assistant = false;
    }
    crate::game::instances::group_changed(realm, Some((id, me.guid)), Some((id, to)));
    let name = gr.slot(to).map(|s| s.name.to_string()).unwrap_or_default();
    if let Some(frame) = encode(&SMSG_GROUP_SET_LEADER { name }) {
        gr.broadcast(realm, &frame, None);
    }
    gr.send_update(realm);
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
    gr.threshold = m.loot_threshold;
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
    gr.send_update(realm);
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
    let Some(frame) = encode(&SMSG_MESSAGECHAT {
        chat_type,
        language,
        message: text.to_owned(),
        tag,
    }) else {
        return;
    };
    let lists = realm.social.lists.lock();
    for m in &gr.members {
        if party_only && Some(m.subgroup) != subgroup {
            continue;
        }
        if !lists.ignores(m.guid, me.guid) {
            send_frame(realm, m.guid, &frame);
        }
    }
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
    if let Some(gr) = groups.group_of(g) {
        gr.send_update(realm);
        let offline = MemberStats::default();
        gr.broadcast(realm, &offline.packet(g, STAT_STATUS, false), Some(g));
    }
}

/// `CMSG_REQUEST_PARTY_MEMBER_STATS`: all of a member's stats.
pub fn request_stats(realm: &Realm, me: &Online, of: Guid) {
    let groups = realm.social.groups.lock();
    let frame = match groups.stats.get(&of) {
        Some(s) if realm.registry.online(of).is_some() => s.packet(of, STAT_PLAYER, true),
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
        }
    }
}
