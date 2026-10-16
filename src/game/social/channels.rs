//! Chat channels: the built-in ones from `ChatChannels.dbc` (General,
//! Trade, LocalDefense and the rest) and custom ones, one set per team,
//! with owners, moderators, passwords, mutes and bans, as mangos-zero runs
//! them. The client joins the built-in ones itself.

use std::sync::Arc;

use ahash::AHashMap;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    Language, PlayerChatTag, SMSG_MESSAGECHAT, SMSG_MESSAGECHAT_ChatType,
};

use crate::game::Realm;
use crate::game::conn::encode;
use crate::game::data::items::Body;
use crate::game::registry::{Online, Team};
use crate::game::social::send_frame;

pub const SMSG_CHANNEL_NOTIFY: u16 = 0x099;
pub const SMSG_CHANNEL_LIST: u16 = 0x09B;

// `SMSG_CHANNEL_NOTIFY` kinds.
pub const JOINED: u8 = 0x00;
pub const LEFT: u8 = 0x01;
pub const YOU_JOINED: u8 = 0x02;
pub const YOU_LEFT: u8 = 0x03;
pub const WRONG_PASSWORD: u8 = 0x04;
pub const NOT_MEMBER: u8 = 0x05;
pub const NOT_MODERATOR: u8 = 0x06;
pub const PASSWORD_CHANGED: u8 = 0x07;
pub const OWNER_CHANGED: u8 = 0x08;
pub const PLAYER_NOT_FOUND: u8 = 0x09;
pub const NOT_OWNER: u8 = 0x0A;
pub const CHANNEL_OWNER: u8 = 0x0B;
pub const MODE_CHANGE: u8 = 0x0C;
pub const ANNOUNCEMENTS_ON: u8 = 0x0D;
pub const ANNOUNCEMENTS_OFF: u8 = 0x0E;
pub const MODERATION_ON: u8 = 0x0F;
pub const MODERATION_OFF: u8 = 0x10;
pub const MUTED: u8 = 0x11;
pub const PLAYER_KICKED: u8 = 0x12;
pub const BANNED: u8 = 0x13;
pub const PLAYER_BANNED: u8 = 0x14;
pub const PLAYER_UNBANNED: u8 = 0x15;
pub const PLAYER_NOT_BANNED: u8 = 0x16;
pub const PLAYER_ALREADY_MEMBER: u8 = 0x17;
pub const INVITE: u8 = 0x18;
pub const INVITE_WRONG_FACTION: u8 = 0x19;
pub const PLAYER_INVITED: u8 = 0x1D;
pub const PLAYER_INVITE_BANNED: u8 = 0x1E;

// Member flags.
pub const MEMBER_OWNER: u8 = 0x01;
pub const MEMBER_MODERATOR: u8 = 0x02;
pub const MEMBER_MUTED: u8 = 0x08;

// Channel flags.
const FLAG_CUSTOM: u8 = 0x01;
const FLAG_TRADE: u8 = 0x04;
const FLAG_NOT_LFG: u8 = 0x08;
const FLAG_GENERAL: u8 = 0x10;
const FLAG_CITY: u8 = 0x20;
const FLAG_LFG: u8 = 0x40;

/// `ChatChannels.dbc`: id, flags, and the name before " - zone".
const BUILT_IN: [(u32, u32, &str); 6] = [
    (1, 0x3, "General"),
    (2, 0x3B, "Trade"),
    (22, 0x1_0003, "LocalDefense"),
    (23, 0x1_0004, "WorldDefense"),
    (24, 0x0, "LookingForGroup"),
    (25, 0x2_0032, "GuildRecruitment"),
];

/// The built-in channel a name is one of: its id and channel flags.
fn built_in(name: &str) -> Option<(u32, u8)> {
    let (id, dbc, _) = BUILT_IN.iter().find(|(_, _, prefix)| {
        name.eq_ignore_ascii_case(prefix)
            || name
                .get(..prefix.len() + 3)
                .is_some_and(|p| p.eq_ignore_ascii_case(&format!("{prefix} - ")))
    })?;
    let mut flags = FLAG_GENERAL;
    if dbc & 0x8 != 0 {
        flags |= FLAG_TRADE;
    }
    if dbc & 0x20 != 0 {
        flags |= FLAG_CITY;
    }
    if dbc & 0x4_0000 != 0 {
        flags |= FLAG_LFG;
    } else {
        flags |= FLAG_NOT_LFG;
    }
    Some((*id, flags))
}

#[derive(Debug, Clone)]
pub struct Member {
    pub guid: Guid,
    pub flags: u8,
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub name: String,
    /// Built-in channels have their DBC id; custom ones 0.
    pub id: u32,
    pub flags: u8,
    pub announce: bool,
    pub moderated: bool,
    pub password: String,
    pub owner: Guid,
    pub members: Vec<Member>,
    pub banned: Vec<Guid>,
}

impl Channel {
    fn new(name: &str) -> Self {
        let (id, flags, announce) = match built_in(name) {
            Some((id, flags)) => (id, flags, false),
            None => (0, FLAG_CUSTOM, !name.eq_ignore_ascii_case("world")),
        };
        Self {
            name: name.to_owned(),
            id,
            flags,
            announce,
            moderated: false,
            password: String::new(),
            owner: Guid::zero(),
            members: Vec::new(),
            banned: Vec::new(),
        }
    }

    fn constant(&self) -> bool {
        self.id != 0
    }

    fn member(&self, g: Guid) -> Option<&Member> {
        self.members.iter().find(|m| m.guid == g)
    }

    fn member_mut(&mut self, g: Guid) -> Option<&mut Member> {
        self.members.iter_mut().find(|m| m.guid == g)
    }

    fn moderator(&self, g: Guid) -> bool {
        self.member(g)
            .is_some_and(|m| m.flags & (MEMBER_MODERATOR | MEMBER_OWNER) != 0)
    }

    fn notify(&self, kind: u8) -> Body {
        let mut b = Body::default();
        b.u8(kind).cstr(&self.name);
        b
    }

    fn send_all(&self, realm: &Realm, frame: &Arc<[u8]>) {
        for m in &self.members {
            send_frame(realm, m.guid, frame);
        }
    }

    fn set_owner(&mut self, realm: &Realm, owner: Guid) {
        let old = self.owner;
        if let Some(m) = self.member_mut(old) {
            m.flags &= !MEMBER_OWNER;
        }
        self.owner = owner;
        let Some(m) = self.member_mut(owner) else {
            return;
        };
        let before = m.flags;
        m.flags |= MEMBER_OWNER | MEMBER_MODERATOR;
        let after = m.flags;
        let mut b = self.notify(MODE_CHANGE);
        b.u64(owner.guid()).u8(before).u8(after);
        self.send_all(realm, &b.frame(SMSG_CHANNEL_NOTIFY));
        let mut b = self.notify(OWNER_CHANGED);
        b.u64(owner.guid());
        self.send_all(realm, &b.frame(SMSG_CHANNEL_NOTIFY));
    }

    /// A member goes; ownership passes on.
    fn remove(&mut self, realm: &Realm, g: Guid) {
        let was_owner = self.owner == g;
        self.members.retain(|m| m.guid != g);
        if self.announce {
            let mut b = self.notify(LEFT);
            b.u64(g.guid());
            self.send_all(realm, &b.frame(SMSG_CHANNEL_NOTIFY));
        }
        if was_owner {
            self.owner = Guid::zero();
            if let Some(next) = self.members.first().map(|m| m.guid) {
                self.set_owner(realm, next);
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct Channels {
    by_key: AHashMap<(Team, String), Channel>,
}

fn key(me: &Online, name: &str) -> (Team, String) {
    (Team::of_race(me.race), name.to_lowercase())
}

fn to_one(realm: &Realm, g: Guid, b: &Body) {
    send_frame(realm, g, &b.frame(SMSG_CHANNEL_NOTIFY));
}

/// The notice for a channel the player isn't in.
fn not_member(realm: &Realm, me: &Online, name: &str) {
    let mut b = Body::default();
    b.u8(NOT_MEMBER).cstr(name);
    to_one(realm, me.guid, &b);
}

impl Channels {
    /// The channels a player is in.
    pub fn of(&self, g: Guid) -> Vec<String> {
        self.by_key
            .values()
            .filter(|c| c.member(g).is_some())
            .map(|c| c.name.clone())
            .collect()
    }

    pub fn get(&self, team: Team, name: &str) -> Option<&Channel> {
        self.by_key.get(&(team, name.to_lowercase()))
    }
}

/// `CMSG_JOIN_CHANNEL`.
pub fn join(realm: &Realm, me: &Online, name: &str, password: &str) {
    if name.is_empty() || name.len() > 100 {
        return;
    }
    let mut channels = realm.social.channels.lock();
    let c = channels
        .by_key
        .entry(key(me, name))
        .or_insert_with(|| Channel::new(name));
    if c.member(me.guid).is_some() {
        if !c.constant() {
            let mut b = c.notify(PLAYER_ALREADY_MEMBER);
            b.u64(me.guid.guid());
            to_one(realm, me.guid, &b);
        }
        return;
    }
    if c.banned.contains(&me.guid) {
        to_one(realm, me.guid, &c.notify(BANNED));
        return;
    }
    if !c.password.is_empty() && c.password != password {
        to_one(realm, me.guid, &c.notify(WRONG_PASSWORD));
        return;
    }
    if c.announce {
        let mut b = c.notify(JOINED);
        b.u64(me.guid.guid());
        c.send_all(realm, &b.frame(SMSG_CHANNEL_NOTIFY));
    }
    c.members.push(Member {
        guid: me.guid,
        flags: 0,
    });
    let mut b = c.notify(YOU_JOINED);
    b.u32(u32::from(c.flags)).u32(0).u8(0);
    to_one(realm, me.guid, &b);
    if !c.constant() && c.owner.is_zero() {
        c.set_owner(realm, me.guid);
    }
}

/// `CMSG_LEAVE_CHANNEL`; at logout, without the notice to the leaver.
pub fn leave(realm: &Realm, me: &Online, name: &str, tell: bool) {
    let mut channels = realm.social.channels.lock();
    let k = key(me, name);
    let Some(c) = channels.by_key.get_mut(&k) else {
        if tell {
            not_member(realm, me, name);
        }
        return;
    };
    if c.member(me.guid).is_none() {
        if tell {
            to_one(realm, me.guid, &c.notify(NOT_MEMBER));
        }
        return;
    }
    if tell {
        to_one(realm, me.guid, &c.notify(YOU_LEFT));
    }
    c.remove(realm, me.guid);
    if c.members.is_empty() && !c.constant() && c.password.is_empty() && c.banned.is_empty() {
        channels.by_key.remove(&k);
    }
}

/// A player going offline leaves every channel.
pub fn leave_all(realm: &Realm, me: &Online) {
    let names = realm.social.channels.lock().of(me.guid);
    for name in names {
        leave(realm, me, &name, false);
    }
}

/// A line in a channel, to every member who doesn't ignore the speaker.
pub fn say(
    realm: &Realm,
    me: &Online,
    name: &str,
    text: &str,
    language: Language,
    tag: PlayerChatTag,
) {
    let channels = realm.social.channels.lock();
    let Some(c) = channels.by_key.get(&key(me, name)) else {
        not_member(realm, me, name);
        return;
    };
    let Some(m) = c.member(me.guid) else {
        to_one(realm, me.guid, &c.notify(NOT_MEMBER));
        return;
    };
    // LocalDefense and WorldDefense are for GMs (and, in mangos, ranks
    // this server has none of).
    if m.flags & MEMBER_MUTED != 0 || (matches!(c.id, 22 | 23) && !me.gm) {
        to_one(realm, me.guid, &c.notify(MUTED));
        return;
    }
    if c.moderated && !c.moderator(me.guid) && !me.gm {
        to_one(realm, me.guid, &c.notify(NOT_MODERATOR));
        return;
    }
    let Some(frame) = encode(&SMSG_MESSAGECHAT {
        chat_type: SMSG_MESSAGECHAT_ChatType::Channel {
            channel_name: c.name.clone(),
            player: me.guid,
            player_rank: 0,
        },
        language,
        message: text.to_owned(),
        tag,
    }) else {
        return;
    };
    let lists = realm.social.lists.lock();
    for member in &c.members {
        if !lists.ignores(member.guid, me.guid) {
            send_frame(realm, member.guid, &frame);
        }
    }
}

/// `CMSG_CHANNEL_LIST`: the members, as mangos lists them.
pub fn list(realm: &Realm, me: &Online, name: &str) {
    let channels = realm.social.channels.lock();
    let Some(c) = channels
        .by_key
        .get(&key(me, name))
        .filter(|c| c.member(me.guid).is_some())
    else {
        not_member(realm, me, name);
        return;
    };
    let mut b = Body::default();
    b.cstr(&c.name).u8(c.flags).u32(c.members.len() as u32);
    for m in &c.members {
        b.u64(m.guid.guid()).u8(m.flags);
    }
    send_frame(realm, me.guid, &b.frame(SMSG_CHANNEL_LIST));
}

/// What a command on a channel needs: the channel, with the player in it.
fn with_member(realm: &Realm, me: &Online, name: &str, f: impl FnOnce(&mut Channel)) {
    let mut channels = realm.social.channels.lock();
    match channels.by_key.get_mut(&key(me, name)) {
        Some(c) if c.member(me.guid).is_some() => f(c),
        Some(c) => to_one(realm, me.guid, &c.notify(NOT_MEMBER)),
        None => not_member(realm, me, name),
    }
}

/// The target of a moderation command, by name: online, on the same team,
/// and (unless `any`) in the channel.
fn target(realm: &Realm, me: &Online, c: &Channel, name: &str, in_channel: bool) -> Option<Guid> {
    let found = realm
        .registry
        .online_by_name(name)
        .filter(|o| Team::of_race(o.race) == Team::of_race(me.race) || me.gm)
        .map(|o| o.guid)
        .filter(|g| !in_channel || c.member(*g).is_some());
    if found.is_none() {
        let mut b = c.notify(PLAYER_NOT_FOUND);
        b.cstr(name);
        to_one(realm, me.guid, &b);
    }
    found
}

/// `CMSG_CHANNEL_PASSWORD`.
pub fn password(realm: &Realm, me: &Online, name: &str, password: &str) {
    with_member(realm, me, name, |c| {
        if !c.moderator(me.guid) && !me.gm {
            to_one(realm, me.guid, &c.notify(NOT_MODERATOR));
            return;
        }
        c.password = password.to_owned();
        let mut b = c.notify(PASSWORD_CHANGED);
        b.u64(me.guid.guid());
        c.send_all(realm, &b.frame(SMSG_CHANNEL_NOTIFY));
    });
}

/// `CMSG_CHANNEL_OWNER`: who owns it.
pub fn owner(realm: &Realm, me: &Online, name: &str) {
    with_member(realm, me, name, |c| {
        let owner = realm
            .registry
            .name(c.owner)
            .map(|n| n.name.to_string())
            .unwrap_or_else(|| "PLAYER_NOT_FOUND".to_owned());
        let mut b = c.notify(CHANNEL_OWNER);
        b.cstr(&owner);
        to_one(realm, me.guid, &b);
    });
}

/// `CMSG_CHANNEL_SET_OWNER`.
pub fn set_owner(realm: &Realm, me: &Online, name: &str, new_owner: &str) {
    with_member(realm, me, name, |c| {
        if c.owner != me.guid && !me.gm {
            to_one(realm, me.guid, &c.notify(NOT_OWNER));
            return;
        }
        if let Some(t) = target(realm, me, c, new_owner, true) {
            c.set_owner(realm, t);
        }
    });
}

/// `CMSG_CHANNEL_MODERATOR` and the rest: a member's moderator or muted
/// flag, set or cleared.
pub fn set_mode(realm: &Realm, me: &Online, name: &str, who: &str, flag: u8, set: bool) {
    with_member(realm, me, name, |c| {
        if !c.moderator(me.guid) && !me.gm {
            to_one(realm, me.guid, &c.notify(NOT_MODERATOR));
            return;
        }
        let Some(t) = target(realm, me, c, who, true) else {
            return;
        };
        if t == c.owner && me.guid != c.owner && !me.gm {
            to_one(realm, me.guid, &c.notify(NOT_OWNER));
            return;
        }
        let Some(m) = c.member_mut(t) else {
            return;
        };
        let before = m.flags;
        if set {
            m.flags |= flag;
        } else {
            m.flags &= !flag;
        }
        let after = m.flags;
        if before != after {
            let mut b = c.notify(MODE_CHANGE);
            b.u64(t.guid()).u8(before).u8(after);
            c.send_all(realm, &b.frame(SMSG_CHANNEL_NOTIFY));
        }
    });
}

/// `CMSG_CHANNEL_KICK` and `CMSG_CHANNEL_BAN`.
pub fn kick(realm: &Realm, me: &Online, name: &str, who: &str, ban: bool) {
    with_member(realm, me, name, |c| {
        if !c.moderator(me.guid) && !me.gm {
            to_one(realm, me.guid, &c.notify(NOT_MODERATOR));
            return;
        }
        let Some(t) = target(realm, me, c, who, true) else {
            return;
        };
        if t == c.owner && me.guid != c.owner && !me.gm {
            to_one(realm, me.guid, &c.notify(NOT_OWNER));
            return;
        }
        if ban && !c.banned.contains(&t) {
            c.banned.push(t);
        }
        let mut b = c.notify(if ban { PLAYER_BANNED } else { PLAYER_KICKED });
        b.u64(t.guid()).u64(me.guid.guid());
        c.send_all(realm, &b.frame(SMSG_CHANNEL_NOTIFY));
        let announce = std::mem::replace(&mut c.announce, false);
        c.remove(realm, t);
        c.announce = announce;
    });
}

/// `CMSG_CHANNEL_UNBAN`.
pub fn unban(realm: &Realm, me: &Online, name: &str, who: &str) {
    with_member(realm, me, name, |c| {
        if !c.moderator(me.guid) && !me.gm {
            to_one(realm, me.guid, &c.notify(NOT_MODERATOR));
            return;
        }
        let banned = realm
            .registry
            .guid_by_name(who)
            .filter(|g| c.banned.contains(g));
        let Some(t) = banned else {
            let mut b = c.notify(PLAYER_NOT_BANNED);
            b.cstr(who);
            to_one(realm, me.guid, &b);
            return;
        };
        c.banned.retain(|g| *g != t);
        let mut b = c.notify(PLAYER_UNBANNED);
        b.u64(t.guid()).u64(me.guid.guid());
        c.send_all(realm, &b.frame(SMSG_CHANNEL_NOTIFY));
    });
}

/// `CMSG_CHANNEL_ANNOUNCEMENTS` and `CMSG_CHANNEL_MODERATE`: toggles.
pub fn toggle(realm: &Realm, me: &Online, name: &str, moderation: bool) {
    with_member(realm, me, name, |c| {
        if !c.moderator(me.guid) && !me.gm {
            to_one(realm, me.guid, &c.notify(NOT_MODERATOR));
            return;
        }
        let kind = if moderation {
            c.moderated = !c.moderated;
            if c.moderated {
                MODERATION_ON
            } else {
                MODERATION_OFF
            }
        } else {
            c.announce = !c.announce;
            if c.announce {
                ANNOUNCEMENTS_ON
            } else {
                ANNOUNCEMENTS_OFF
            }
        };
        let mut b = c.notify(kind);
        b.u64(me.guid.guid());
        c.send_all(realm, &b.frame(SMSG_CHANNEL_NOTIFY));
    });
}

/// `CMSG_CHANNEL_INVITE`.
pub fn invite(realm: &Realm, me: &Online, name: &str, who: &str) {
    with_member(realm, me, name, |c| {
        let Some(t) = realm.registry.online_by_name(who) else {
            let mut b = c.notify(PLAYER_NOT_FOUND);
            b.cstr(who);
            to_one(realm, me.guid, &b);
            return;
        };
        if Team::of_race(t.race) != Team::of_race(me.race) && !me.gm {
            to_one(realm, me.guid, &c.notify(INVITE_WRONG_FACTION));
            return;
        }
        if c.member(t.guid).is_some() {
            let mut b = c.notify(PLAYER_ALREADY_MEMBER);
            b.u64(t.guid.guid());
            to_one(realm, me.guid, &b);
            return;
        }
        if c.banned.contains(&t.guid) {
            let mut b = c.notify(PLAYER_INVITE_BANNED);
            b.cstr(who);
            to_one(realm, me.guid, &b);
            return;
        }
        if !realm.social.lists.lock().ignores(t.guid, me.guid) {
            let mut b = c.notify(INVITE);
            b.u64(me.guid.guid());
            to_one(realm, t.guid, &b);
        }
        let mut b = c.notify(PLAYER_INVITED);
        b.cstr(&t.name);
        to_one(realm, me.guid, &b);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_channels_are_known_by_their_prefix() {
        assert_eq!(built_in("General - Elwynn Forest"), Some((1, 0x18)));
        assert_eq!(built_in("Trade - City"), Some((2, 0x3C)));
        assert_eq!(built_in("WorldDefense"), Some((23, 0x18)));
        assert_eq!(built_in("GuildRecruitment - City"), Some((25, 0x38)));
        assert_eq!(built_in("peon"), None);
        assert_eq!(built_in("Generalist"), None);
    }
}
