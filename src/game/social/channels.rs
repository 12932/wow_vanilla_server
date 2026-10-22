//! Chat channels: the built-in ones from `ChatChannels.dbc` (General,
//! Trade, LocalDefense and the rest) and custom ones, one set per team,
//! with owners, moderators, passwords, mutes and bans, as mangos-zero runs
//! them. The client joins the built-in ones itself at login; as a player
//! changes zone, the server moves them ([`zone_changed`]).
//!
//! Each channel has a lock of its own ([`Channels`]), and a line is fanned
//! out after it is let go ([`say`]): two channels never wait for each
//! other, and a channel's members are sent to without it held.

use std::sync::Arc;

use ahash::AHashMap;
use parking_lot::{Mutex, MutexGuard, RwLock};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    Language, PlayerChatTag, SMSG_MESSAGECHAT, SMSG_MESSAGECHAT_ChatType,
};

use crate::game::Realm;
use crate::game::conn::encode;
use crate::game::data::items::Body;
use crate::game::registry::{Online, Team};
use crate::game::social::{not_ignoring, send_frame, send_frames};

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

/// The built-in channels' ids.
pub const CHANNEL_GENERAL: u32 = 1;
pub const CHANNEL_TRADE: u32 = 2;
pub const CHANNEL_LOCAL_DEFENSE: u32 = 22;
pub const CHANNEL_WORLD_DEFENSE: u32 = 23;
pub const CHANNEL_GUILD_RECRUITMENT: u32 = 25;

/// The honor rank (as the client shows it, 0 to 14) one needs to speak in
/// WorldDefense (mangos's `Channel.h`; LocalDefense's is 0, anyone's).
pub const SPEAK_IN_WORLD_DEFENSE_RANK: u8 = 11;

/// `ChatChannels.dbc`: id, flags, and the name before " - zone".
const BUILT_IN: [(u32, u32, &str); 6] = [
    (1, 0x3, "General"),
    (2, 0x3B, "Trade"),
    (22, 0x1_0003, "LocalDefense"),
    (23, 0x1_0004, "WorldDefense"),
    (24, 0x0, "LookingForGroup"),
    (25, 0x2_0032, "GuildRecruitment"),
];

// `ChatChannels.dbc`'s flags.
const DBC_ZONE_DEPENDENT: u32 = 0x2;
const DBC_GLOBAL: u32 = 0x4;
const DBC_CITY_ONLY: u32 = 0x10;

/// The built-in channel a name is one of: its `BUILT_IN` row.
fn built_in_row(name: &str) -> Option<&'static (u32, u32, &'static str)> {
    BUILT_IN.iter().find(|(_, _, prefix)| {
        name.eq_ignore_ascii_case(prefix)
            || name
                .get(..prefix.len() + 3)
                .is_some_and(|p| p.eq_ignore_ascii_case(&format!("{prefix} - ")))
    })
}

/// The built-in channel a name is one of: its id and channel flags.
pub(crate) fn built_in(name: &str) -> Option<(u32, u8)> {
    let (id, dbc, _) = built_in_row(name)?;
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
    /// Taken out of the directory, its last member gone: one who found it
    /// just before finds no channel (`Channels::with`).
    dropped: bool,
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
            dropped: false,
        }
    }

    fn constant(&self) -> bool {
        self.id != 0
    }

    /// Its last member gone: it goes, its password and bans with it
    /// (`Channels::run`). A built-in one too, unlike mangos's
    /// `LeftChannel`: its name takes any zone after the prefix ("General -
    /// 1", "General - 2"), so kept, a client could make one for good with
    /// every join. It has no owner to have set anything worth keeping, and
    /// the next join makes it again.
    fn gone(&self) -> bool {
        self.members.is_empty()
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

    /// A notice to every member, under the channel's lock: a join, a leave
    /// or a command's, in order with the channel's changes.
    fn send_all(&self, realm: &Realm, frame: &Arc<[u8]>) {
        let members: Vec<Guid> = self.members.iter().map(|m| m.guid).collect();
        send_frames(realm, &members, frame);
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

    /// A member goes; ownership passes on, to the lowest GUID left (as
    /// mangos's members, kept by GUID, and Thistle Tea's `transfer_owner`).
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
            if let Some(next) = self.members.iter().map(|m| m.guid).min_by_key(|g| g.guid()) {
                self.set_owner(realm, next);
            }
        }
    }
}

/// A channel's key: its team, and its name lowercased.
type Key = (Team, String);

/// A channel, behind its own lock.
type Shared = Arc<Mutex<Channel>>;

/// Every channel, each behind a lock of its own: two channels' lines,
/// joins and commands never wait for each other. The directory is locked
/// only to find a channel (a read), or to put one in or take one out (a
/// write), never while waiting for a channel's lock: a command on one
/// channel, its notices sent under its lock, holds up no other.
#[derive(Debug, Default)]
pub struct Channels {
    directory: RwLock<Directory>,
}

/// The channels, by key.
#[derive(Debug, Default)]
pub struct Directory {
    by_key: AHashMap<Key, Shared>,
}

fn key(me: &Online, name: &str) -> Key {
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
    /// The directory, read, for a test to look at the channels.
    #[cfg(test)]
    pub fn lock(&self) -> parking_lot::RwLockReadGuard<'_, Directory> {
        self.directory.read()
    }

    fn find(&self, k: &Key) -> Option<Shared> {
        self.directory.read().by_key.get(k).cloned()
    }

    /// `f` on `k`'s channel, locked; `None` if there is none.
    fn with<R>(&self, k: &Key, f: impl FnOnce(&mut Channel) -> R) -> Option<R> {
        let c = self.find(k)?;
        let guard = c.lock();
        // Taken out after it was found: there is none.
        if guard.dropped {
            return None;
        }
        Some(self.run(k, &c, guard, f))
    }

    /// `f` on `k`'s channel, locked, made as `name` if there is none.
    fn with_made<R>(&self, k: Key, name: &str, f: impl FnOnce(&mut Channel) -> R) -> R {
        loop {
            if let Some(c) = self.find(&k) {
                let guard = c.lock();
                if !guard.dropped {
                    return self.run(&k, &c, guard, f);
                }
                // Its last leaver is about to take it out: out it goes now,
                // and a new one in.
                drop(guard);
                self.take_out(&k, &c);
                continue;
            }
            // Locked before it is in: no one finds it empty, before its
            // first member.
            let c: Shared = Arc::new(Mutex::new(Channel::new(name)));
            let guard = c.lock();
            {
                let mut directory = self.directory.write();
                if directory.by_key.contains_key(&k) {
                    // Made meanwhile: that one.
                    continue;
                }
                directory.by_key.insert(k.clone(), c.clone());
            }
            return self.run(&k, &c, guard, f);
        }
    }

    /// `f` on `c`, `k`'s channel, under `guard`. Left empty, it goes, its password and bans with it (mangos's
    /// `ChannelMgr::LeftChannel`, Thistle Tea's `Channels.put`): kept, an
    /// empty channel locked by a password nobody remembers would stay for
    /// good, one for every name a client cares to lock.
    fn run<R>(
        &self,
        k: &Key,
        c: &Shared,
        mut guard: MutexGuard<'_, Channel>,
        f: impl FnOnce(&mut Channel) -> R,
    ) -> R {
        let r = f(&mut guard);
        let gone = guard.gone();
        guard.dropped = gone;
        drop(guard);
        if gone {
            self.take_out(k, c);
        }
        r
    }

    /// `c`, dropped, out of the directory, if it is still `k`'s channel.
    fn take_out(&self, k: &Key, c: &Shared) {
        let mut directory = self.directory.write();
        if directory.by_key.get(k).is_some_and(|d| Arc::ptr_eq(d, c)) {
            directory.by_key.remove(k);
        }
    }

    /// The channels a player is in.
    pub fn of(&self, g: Guid) -> Vec<String> {
        let all: Vec<Shared> = self.directory.read().by_key.values().cloned().collect();
        all.iter()
            .filter_map(|c| {
                let c = c.lock();
                (!c.dropped && c.member(g).is_some()).then(|| c.name.clone())
            })
            .collect()
    }
}

#[cfg(test)]
impl Directory {
    /// How many channels there are.
    pub fn count(&self) -> usize {
        self.by_key.len()
    }

    /// The channels a player is in.
    pub fn of(&self, g: Guid) -> Vec<String> {
        self.by_key
            .values()
            .filter_map(|c| {
                let c = c.lock();
                c.member(g).is_some().then(|| c.name.clone())
            })
            .collect()
    }

    /// A channel, locked.
    pub fn get(&self, team: Team, name: &str) -> Option<MutexGuard<'_, Channel>> {
        self.by_key
            .get(&(team, name.to_lowercase()))
            .map(|c| c.lock())
    }
}

/// `CMSG_JOIN_CHANNEL`.
pub fn join(realm: &Realm, me: &Online, name: &str, password: &str) {
    if name.is_empty() || name.len() > 100 {
        return;
    }
    // Asked before the channel's lock.
    let in_guild = realm.social.guilds.lock().of(me.guid).is_some();
    realm.social.channels.with_made(key(me, name), name, |c| {
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
        // GuildRecruitment is for the unguilded: a guild's member isn't
        // let in, and isn't told (mangos's `Channel::Join`, its flags
        // 0x38).
        if in_guild && c.flags == FLAG_GENERAL | FLAG_CITY | FLAG_NOT_LFG {
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
    });
}

/// `CMSG_LEAVE_CHANNEL`; at logout, without the notice to the leaver.
pub fn leave(realm: &Realm, me: &Online, name: &str, tell: bool) {
    let found = realm.social.channels.with(&key(me, name), |c| {
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
    });
    if found.is_none() && tell {
        not_member(realm, me, name);
    }
}

/// A player going offline leaves every channel.
pub fn leave_all(realm: &Realm, me: &Online) {
    let names = realm.social.channels.of(me.guid);
    for name in names {
        leave(realm, me, &name, false);
    }
}

/// A player come into `zone`: each built-in channel of a zone they are in
/// (General, LocalDefense) is the new zone's, joined ("Changed Channel")
/// and the old one left without a word, as mangos's
/// `Player::UpdateLocalChannels` does it: the 1.12 client swaps the
/// channel on its side and sends no join or leave of its own ("recent
/// client version not send leave/join channel packets for built-in local
/// channels"). A city's (Trade, GuildRecruitment) is "<name> - City" in a
/// capital, and left outside one, as TrinityCore and vmangos take it
/// (mangos-classic's would be "Trade - Elwynn Forest"). The global ones
/// (WorldDefense, LookingForGroup) stay.
pub fn zone_changed(realm: &Realm, me: &Online, zone: u32) {
    let Some(area) = realm.data.area(zone) else {
        return;
    };
    let capital = area.flags & crate::game::data::AREA_FLAG_CAPITAL != 0;
    for old in realm.social.channels.of(me.guid) {
        let Some((_, dbc, prefix)) = built_in_row(&old) else {
            continue;
        };
        if dbc & DBC_GLOBAL != 0 || dbc & DBC_ZONE_DEPENDENT == 0 {
            continue;
        }
        let new = if dbc & DBC_CITY_ONLY != 0 {
            capital.then(|| format!("{prefix} - City"))
        } else {
            Some(format!("{prefix} - {}", area.name))
        };
        if new.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(&old)) {
            continue;
        }
        if let Some(new) = new {
            join(realm, me, &new, "");
        }
        leave(realm, me, &old, false);
    }
}

/// A line in a channel, to every member who doesn't ignore the speaker.
/// Under the channel's lock, only its checks and a copy of its members:
/// the line is encoded, the ignore lists asked and the members sent to
/// once it is let go, so the channel's other speakers, joins and commands
/// don't wait for the fan-out.
pub fn say(
    realm: &Realm,
    me: &Online,
    name: &str,
    text: &str,
    language: Language,
    tag: PlayerChatTag,
) {
    // The rank the client shows, asked before the channel's lock.
    let rank = crate::game::honor::visual_rank_of(
        realm
            .social
            .honor
            .lock()
            .rank(crate::game::guid::counter(me.guid)),
    );
    let checked = realm.social.channels.with(&key(me, name), |c| {
        let Some(m) = c.member(me.guid) else {
            to_one(realm, me.guid, &c.notify(NOT_MEMBER));
            return None;
        };
        // Anyone may speak in LocalDefense, and in WorldDefense from the
        // eleventh rank (mangos's `Channel::Say`,
        // `SPEAK_IN_LOCALDEFENSE_RANK` 0 and `SPEAK_IN_WORLDDEFENSE_RANK`
        // 11), GMs or not.
        let ranked = c.id != CHANNEL_WORLD_DEFENSE || rank >= SPEAK_IN_WORLD_DEFENSE_RANK;
        if m.flags & MEMBER_MUTED != 0 || !ranked {
            to_one(realm, me.guid, &c.notify(MUTED));
            return None;
        }
        if c.moderated && !c.moderator(me.guid) && !me.gm {
            to_one(realm, me.guid, &c.notify(NOT_MODERATOR));
            return None;
        }
        // A moderator's words reach even those ignoring them (mangos's
        // `Channel::Say`, Thistle Tea's).
        let heard_by_all = c.moderator(me.guid);
        let members: Vec<Guid> = c.members.iter().map(|m| m.guid).collect();
        Some((c.name.clone(), heard_by_all, members))
    });
    let Some(checked) = checked else {
        not_member(realm, me, name);
        return;
    };
    let Some((channel_name, heard_by_all, mut members)) = checked else {
        return;
    };
    let Some(frame) = encode(&SMSG_MESSAGECHAT {
        chat_type: SMSG_MESSAGECHAT_ChatType::Channel {
            channel_name,
            player: me.guid,
            player_rank: 0,
        },
        language,
        message: text.to_owned(),
        tag,
    }) else {
        return;
    };
    if !heard_by_all {
        not_ignoring(realm, me.guid, &mut members);
    }
    send_frames(realm, &members, &frame);
}

/// `CMSG_CHANNEL_LIST`: the members, as mangos lists them.
pub fn list(realm: &Realm, me: &Online, name: &str) {
    let listed = realm.social.channels.with(&key(me, name), |c| {
        if c.member(me.guid).is_none() {
            return false;
        }
        let mut b = Body::default();
        b.cstr(&c.name).u8(c.flags).u32(c.members.len() as u32);
        for m in &c.members {
            b.u64(m.guid.guid()).u8(m.flags);
        }
        send_frame(realm, me.guid, &b.frame(SMSG_CHANNEL_LIST));
        true
    });
    if listed != Some(true) {
        not_member(realm, me, name);
    }
}

/// What a command on a channel needs: the channel, with the player in it.
/// Its last member kicked out, it goes, as at a leave (`Channels::run`).
fn with_member(realm: &Realm, me: &Online, name: &str, f: impl FnOnce(&mut Channel)) {
    let found = realm.social.channels.with(&key(me, name), |c| {
        if c.member(me.guid).is_none() {
            to_one(realm, me.guid, &c.notify(NOT_MEMBER));
            return;
        }
        f(c);
    });
    if found.is_none() {
        not_member(realm, me, name);
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

    /// A player online with a queue the test reads.
    fn player(
        realm: &Realm,
        map: &crate::game::map::MapHandle,
        n: u32,
    ) -> (Arc<Online>, crate::game::conn::Frames) {
        let entry = realm.registry.add_name(crate::game::registry::NameEntry {
            guid: crate::game::guid::player(n),
            account: Arc::from(format!("A{n}")),
            name: Arc::from(format!("P{n}")),
            race: 1,
            class: 1,
            gender: 0,
            level: 60.into(),
        });
        let (outbound, frames) = crate::game::conn::Outbound::detached();
        let (session, _) = tokio::sync::mpsc::unbounded_channel();
        let o = Arc::new(Online::new(&entry, 0, outbound, session, map.clone()));
        realm.registry.go_online(o.clone()).unwrap();
        (o, frames)
    }

    /// The lines of `SMSG_MESSAGECHAT` in what `frames` holds, in order.
    fn lines(frames: &mut crate::game::conn::Frames) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(f) = frames.try_recv() {
            if f.len() < 4 || u16::from_le_bytes([f[2], f[3]]) != 0x096 {
                continue;
            }
            let at = f.windows(5).position(|w| w == b"line ").unwrap();
            let end = f[at..].iter().position(|b| *b == 0).unwrap();
            out.push(String::from_utf8_lossy(&f[at..at + end]).into_owned());
        }
        out
    }

    /// The `SMSG_CHANNEL_NOTIFY` kinds in what `frames` holds, in order.
    fn notices(frames: &mut crate::game::conn::Frames) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some(f) = frames.try_recv() {
            if f.len() > 4 && u16::from_le_bytes([f[2], f[3]]) == SMSG_CHANNEL_NOTIFY {
                out.push(f[4]);
            }
        }
        out
    }

    /// A line said in one channel while, on other threads, two players make
    /// and drop another channel over and over (each joining and leaving
    /// it, racing the other) and a third joins and leaves the speaker's:
    /// nothing waits for good, every line reaches every member in order,
    /// and every join and leave is one.
    #[tokio::test]
    async fn lines_said_while_channels_are_joined_elsewhere_all_arrive() {
        const LINES: usize = 2000;
        let realm = crate::game::harness::realm();
        let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
        let (alice, mut alice_frames) = player(&realm, &map, 1);
        let (bob, mut bob_frames) = player(&realm, &map, 2);
        let (carol, _carol_frames) = player(&realm, &map, 3);
        let (dave, dave_frames) = player(&realm, &map, 4);
        let (erin, erin_frames) = player(&realm, &map, 5);
        join(&realm, &alice, "Tea", "");
        join(&realm, &bob, "Tea", "");
        let (done, finished) = std::sync::mpsc::channel();
        let mut threads = vec![
            {
                let (realm, done) = (realm.clone(), done.clone());
                std::thread::spawn(move || {
                    for i in 0..LINES {
                        let line = format!("line {i}");
                        say(
                            &realm,
                            &alice,
                            "Tea",
                            &line,
                            Language::Universal,
                            PlayerChatTag::None,
                        );
                    }
                    done.send(()).unwrap();
                })
            },
            {
                let (realm, done) = (realm.clone(), done.clone());
                std::thread::spawn(move || {
                    for _ in 0..LINES {
                        join(&realm, &carol, "Tea", "");
                        leave_all(&realm, &carol);
                    }
                    done.send(()).unwrap();
                })
            },
        ];
        let mut makers = Vec::new();
        for (who, mut frames) in [(dave, dave_frames), (erin, erin_frames)] {
            let (realm, done) = (realm.clone(), done.clone());
            makers.push(std::thread::spawn(move || {
                let mut heard = Vec::new();
                for _ in 0..LINES {
                    join(&realm, &who, "Coffee", "");
                    leave(&realm, &who, "Coffee", true);
                    heard.extend(notices(&mut frames));
                }
                done.send(()).unwrap();
                heard
            }));
        }
        for _ in 0..threads.len() + makers.len() {
            finished
                .recv_timeout(std::time::Duration::from_secs(30))
                .expect("no thread waits for good");
        }
        for t in threads.drain(..) {
            t.join().unwrap();
        }
        for m in makers {
            let heard = m.join().unwrap();
            let count = |kind: u8| heard.iter().filter(|k| **k == kind).count();
            assert_eq!(
                (count(YOU_JOINED), count(YOU_LEFT), count(NOT_MEMBER)),
                (LINES, LINES, 0),
                "every join and leave of Coffee's is one"
            );
        }
        let expected: Vec<String> = (0..LINES).map(|i| format!("line {i}")).collect();
        assert_eq!(lines(&mut bob_frames), expected);
        assert_eq!(lines(&mut alice_frames), expected);
        let directory = realm.social.channels.lock();
        assert!(directory.get(Team::Alliance, "Coffee").is_none());
        assert_eq!(directory.of(bob.guid), vec!["Tea".to_owned()]);
        assert_eq!(directory.count(), 1);
    }
}
