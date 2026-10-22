//! Friends and ignore lists. Each character's lists are kept with it; the
//! lists of those online are also here, so a login can be told to those
//! who have the player as a friend, and a whisper or a channel line can
//! skip those who ignore its speaker.

use ahash::{AHashMap, AHashSet};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    Area, Class, Friend, Friend_FriendStatus, Level, SMSG_FRIEND_LIST, SMSG_IGNORE_LIST,
};

use crate::game::Realm;
use crate::game::data::items::Body;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::registry::{Online, Team};
use crate::game::social::send_frame;

pub const SMSG_FRIEND_STATUS: u16 = 0x068;
pub const MAX_FRIENDS: usize = 50;
pub const MAX_IGNORED: usize = 25;

// `FriendResult`.
const LIST_FULL: u8 = 0x01;
const ONLINE: u8 = 0x02;
const OFFLINE: u8 = 0x03;
const NOT_FOUND: u8 = 0x04;
const REMOVED: u8 = 0x05;
const ADDED_ONLINE: u8 = 0x06;
const ADDED_OFFLINE: u8 = 0x07;
const ALREADY: u8 = 0x08;
const SELF: u8 = 0x09;
const ENEMY: u8 = 0x0A;
const IGNORE_FULL: u8 = 0x0B;
const IGNORE_SELF: u8 = 0x0C;
const IGNORE_NOT_FOUND: u8 = 0x0D;
const IGNORE_ALREADY: u8 = 0x0E;
const IGNORE_ADDED: u8 = 0x0F;
const IGNORE_REMOVED: u8 = 0x10;

/// The lists of everyone online.
#[derive(Debug, Default)]
pub struct Lists {
    friends: AHashMap<Guid, AHashSet<Guid>>,
    ignores: AHashMap<Guid, AHashSet<Guid>>,
    /// `ignores` the other way round: those online ignoring a player, so
    /// that a line to a channel, a guild or a group asks once
    /// ([`Lists::ignorers`]), not once a recipient. Changed only with
    /// `ignores`, by the methods below.
    ignored_by: AHashMap<Guid, AHashSet<Guid>>,
    /// `friends` the other way round: those online with a player as a
    /// friend, so that their login and logout ask once
    /// ([`Lists::watchers`]), not once a player online. Changed only with
    /// `friends`, by the methods below.
    friended_by: AHashMap<Guid, AHashSet<Guid>>,
}

impl Lists {
    /// Whether `owner` ignores `other`.
    pub fn ignores(&self, owner: Guid, other: Guid) -> bool {
        self.ignores.get(&owner).is_some_and(|s| s.contains(&other))
    }

    /// Those online who ignore `g`.
    pub fn ignorers(&self, g: Guid) -> AHashSet<Guid> {
        self.ignored_by.get(&g).cloned().unwrap_or_default()
    }

    /// `owner` ignores `other`.
    fn ignore(&mut self, owner: Guid, other: Guid) {
        self.ignores.entry(owner).or_default().insert(other);
        self.ignored_by.entry(other).or_default().insert(owner);
    }

    /// `owner` no longer ignores `other`.
    fn unignore(&mut self, owner: Guid, other: Guid) {
        if let Some(s) = self.ignores.get_mut(&owner) {
            s.remove(&other);
        }
        self.stop_ignoring(owner, other);
    }

    /// `owner` off the list of those ignoring `other`.
    fn stop_ignoring(&mut self, owner: Guid, other: Guid) {
        if let Some(by) = self.ignored_by.get_mut(&other) {
            by.remove(&owner);
            if by.is_empty() {
                self.ignored_by.remove(&other);
            }
        }
    }

    /// `owner`'s ignore list, all of it, in place of what it was.
    fn set_ignores(&mut self, owner: Guid, ignores: AHashSet<Guid>) {
        self.drop_ignores(owner);
        for i in &ignores {
            self.ignored_by.entry(*i).or_default().insert(owner);
        }
        self.ignores.insert(owner, ignores);
    }

    /// `owner`'s ignore list gone (offline, or deleted).
    fn drop_ignores(&mut self, owner: Guid) {
        for i in self.ignores.remove(&owner).unwrap_or_default() {
            self.stop_ignoring(owner, i);
        }
    }

    /// `owner` has `other` as a friend.
    fn befriend(&mut self, owner: Guid, other: Guid) {
        self.friends.entry(owner).or_default().insert(other);
        self.friended_by.entry(other).or_default().insert(owner);
    }

    /// `owner` no longer has `other` as a friend.
    fn unfriend(&mut self, owner: Guid, other: Guid) {
        if let Some(s) = self.friends.get_mut(&owner) {
            s.remove(&other);
        }
        self.stop_watching(owner, other);
    }

    /// `owner` off the list of those with `other` as a friend.
    fn stop_watching(&mut self, owner: Guid, other: Guid) {
        if let Some(by) = self.friended_by.get_mut(&other) {
            by.remove(&owner);
            if by.is_empty() {
                self.friended_by.remove(&other);
            }
        }
    }

    /// `owner`'s friend list, all of it, in place of what it was.
    fn set_friends(&mut self, owner: Guid, friends: AHashSet<Guid>) {
        self.drop_friends(owner);
        for f in &friends {
            self.friended_by.entry(*f).or_default().insert(owner);
        }
        self.friends.insert(owner, friends);
    }

    /// `owner`'s friend list gone (offline, or deleted).
    fn drop_friends(&mut self, owner: Guid) {
        for f in self.friends.remove(&owner).unwrap_or_default() {
            self.stop_watching(owner, f);
        }
    }

    /// A character deleted: off everyone's lists, and its own gone.
    fn forget(&mut self, g: Guid) {
        for owner in self.friended_by.remove(&g).unwrap_or_default() {
            if let Some(f) = self.friends.get_mut(&owner) {
                f.remove(&g);
            }
        }
        for owner in self.ignored_by.remove(&g).unwrap_or_default() {
            if let Some(i) = self.ignores.get_mut(&owner) {
                i.remove(&g);
            }
        }
        self.drop_friends(g);
        self.drop_ignores(g);
    }

    /// Those online with `g` as a friend.
    fn watchers(&self, g: Guid) -> Vec<Guid> {
        self.friended_by
            .get(&g)
            .map(|by| by.iter().copied().collect())
            .unwrap_or_default()
    }
}

/// A friend's status byte: 1 online, 2 AFK, 4 DND.
fn status_of(o: &Online) -> u8 {
    use crate::game::map::player::{PLAYER_FLAGS_AFK, PLAYER_FLAGS_DND};
    let flags = o.flags.load(std::sync::atomic::Ordering::Relaxed);
    if flags & PLAYER_FLAGS_DND != 0 {
        4
    } else if flags & PLAYER_FLAGS_AFK != 0 {
        2
    } else {
        1
    }
}

/// `SMSG_FRIEND_STATUS`, with where and what the friend is when online.
fn status(realm: &Realm, to: Guid, result: u8, friend: Guid) {
    let mut b = Body::default();
    b.u8(result).u64(friend.guid());
    if matches!(result, ONLINE | ADDED_ONLINE) {
        let o = realm.registry.online(friend);
        b.u8(o.as_deref().map_or(1, status_of))
            .u32(o.as_deref().map_or(0, Online::zone))
            .u32(o.as_deref().map_or(1, |o| u32::from(o.level())))
            .u32(o.as_deref().map_or(0, |o| u32::from(o.class)));
    }
    send_frame(realm, to, &b.frame(SMSG_FRIEND_STATUS));
}

fn friend_entry(realm: &Realm, owner: &Online, g: Guid) -> Friend {
    let status = match realm.registry.online(g) {
        Some(o) if crate::game::social::same_side(owner, &o) => {
            let area = Area::try_from(o.zone()).unwrap_or_default();
            let class = Class::try_from(o.class).unwrap_or_default();
            let level = Level::new(o.level());
            match status_of(&o) {
                2 => Friend_FriendStatus::Afk { area, class, level },
                4 => Friend_FriendStatus::Dnd { area, class, level },
                _ => Friend_FriendStatus::Online { area, class, level },
            }
        }
        _ => Friend_FriendStatus::Offline,
    };
    Friend { guid: g, status }
}

/// The friend and ignore lists, sent: the characters since deleted gone
/// from both (Thistle Tea's `send_lists`). Their sets, for the lists of
/// those online.
fn send_lists(map: &mut Map, g: Guid) -> Option<(AHashSet<Guid>, AHashSet<Guid>)> {
    let realm = map.realm.clone();
    let me = realm.registry.online(g)?;
    let p = map.players.get_mut(&g)?;
    let exists = |c: &u32| realm.registry.name(guid::player(*c)).is_some();
    p.row.state.friends.retain(exists);
    p.row.state.ignored.retain(exists);
    let friends: AHashSet<Guid> = p
        .row
        .state
        .friends
        .iter()
        .map(|c| guid::player(*c))
        .collect();
    let ignores: AHashSet<Guid> = p
        .row
        .state
        .ignored
        .iter()
        .map(|c| guid::player(*c))
        .collect();
    p.outbound.send(&SMSG_FRIEND_LIST {
        friends: friends
            .iter()
            .map(|f| friend_entry(&realm, &me, *f))
            .collect(),
    });
    p.outbound.send(&SMSG_IGNORE_LIST {
        ignored: ignores.iter().map(|i| i.guid()).collect(),
    });
    Some((friends, ignores))
}

/// `CMSG_FRIEND_LIST`: the lists again, with each friend's status now
/// (mangos's `HandleFriendListOpcode`).
pub fn list(map: &mut Map, g: Guid) {
    if let Some((friends, ignores)) = send_lists(map, g) {
        let mut lists = map.realm.social.lists.lock();
        lists.set_friends(g, friends);
        lists.set_ignores(g, ignores);
    }
}

/// `CMSG_CHAT_IGNORED`: the client didn't show `other`'s line, as `g`
/// ignores them; `other` is told, if `g` does (mangos's
/// `HandleChatIgnoredOpcode`, Thistle Tea's `Social.ignored`).
pub fn chat_ignored(map: &Map, g: Guid, other: Guid) {
    let realm = &map.realm;
    if !realm.social.lists.lock().ignores(g, other) {
        return;
    }
    let Some(me) = realm.registry.name(g) else {
        return;
    };
    let Some(frame) = crate::game::conn::encode(&wow_world_messages::vanilla::SMSG_MESSAGECHAT {
        chat_type: wow_world_messages::vanilla::SMSG_MESSAGECHAT_ChatType::Ignored { sender2: g },
        language: wow_world_messages::vanilla::Language::Universal,
        message: me.name.to_string(),
        tag: wow_world_messages::vanilla::PlayerChatTag::None,
    }) else {
        return;
    };
    send_frame(realm, other, &frame);
}

/// At login: the lists are sent, and those with the player as a friend
/// told.
pub fn logged_in(map: &mut Map, g: Guid) {
    let realm = map.realm.clone();
    let Some(me) = realm.registry.online(g) else {
        return;
    };
    let Some((friends, ignores)) = send_lists(map, g) else {
        return;
    };
    let watchers = {
        let mut lists = realm.social.lists.lock();
        lists.set_friends(g, friends);
        lists.set_ignores(g, ignores);
        lists.watchers(g)
    };
    for w in watchers {
        if realm
            .registry
            .online(w)
            .is_some_and(|o| crate::game::social::same_side(&o, &me))
        {
            status(&realm, w, ONLINE, g);
        }
    }
}

/// The player's client has gone, they lingering in the world: those with
/// them as a friend are told they are offline (mangos's `SetOffline`), and
/// told again as they leave it ([`logged_out`]), as mangos's
/// `LogoutPlayer` tells them; back first, they are told online again.
pub fn told_offline(realm: &Realm, g: Guid) {
    let watchers = realm.social.lists.lock().watchers(g);
    for w in watchers {
        status(realm, w, OFFLINE, g);
    }
}

/// Going offline: those with the player as a friend are told.
pub fn logged_out(realm: &Realm, g: Guid) {
    let watchers = {
        let mut lists = realm.social.lists.lock();
        lists.drop_friends(g);
        lists.drop_ignores(g);
        lists.watchers(g)
    };
    for w in watchers {
        status(realm, w, OFFLINE, g);
    }
}

/// `CMSG_ADD_FRIEND`.
pub fn add_friend(map: &mut Map, g: Guid, name: &str) {
    let realm = map.realm.clone();
    let Some(me) = realm.registry.online(g) else {
        return;
    };
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let Some(entry) = realm
        .registry
        .guid_by_name(name)
        .and_then(|f| realm.registry.name(f))
    else {
        status(&realm, g, NOT_FOUND, Guid::zero());
        return;
    };
    let f = entry.guid;
    let counter = guid::counter(f);
    let result = if f == g {
        SELF
    } else if p.row.state.friends.contains(&counter) {
        ALREADY
    } else if Team::of_race(entry.race) != Team::of_race(me.race) && !me.gm {
        ENEMY
    } else if p.row.state.friends.len() >= MAX_FRIENDS {
        LIST_FULL
    } else {
        p.row.state.friends.push(counter);
        realm.social.lists.lock().befriend(g, f);
        let online = realm
            .registry
            .online(f)
            .is_some_and(|o| crate::game::social::same_side(&me, &o));
        if online { ADDED_ONLINE } else { ADDED_OFFLINE }
    };
    status(&realm, g, result, f);
}

/// `CMSG_DEL_FRIEND`.
pub fn del_friend(map: &mut Map, g: Guid, f: Guid) {
    let realm = map.realm.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    // Taken off, or not there: removed either way (mangos's
    // `HandleDelFriendOpcode`, Thistle Tea's idempotent `remove/3`).
    let counter = guid::counter(f);
    p.row.state.friends.retain(|c| *c != counter);
    realm.social.lists.lock().unfriend(g, f);
    status(&realm, g, REMOVED, f);
}

/// `CMSG_ADD_IGNORE`.
pub fn add_ignore(map: &mut Map, g: Guid, name: &str) {
    let realm = map.realm.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let Some(i) = realm.registry.guid_by_name(name) else {
        status(&realm, g, IGNORE_NOT_FOUND, Guid::zero());
        return;
    };
    let counter = guid::counter(i);
    let result = if i == g {
        IGNORE_SELF
    } else if p.row.state.ignored.contains(&counter) {
        IGNORE_ALREADY
    } else if p.row.state.ignored.len() >= MAX_IGNORED {
        IGNORE_FULL
    } else {
        p.row.state.ignored.push(counter);
        realm.social.lists.lock().ignore(g, i);
        IGNORE_ADDED
    };
    status(&realm, g, result, i);
}

/// `CMSG_DEL_IGNORE`.
pub fn del_ignore(map: &mut Map, g: Guid, i: Guid) {
    let realm = map.realm.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let counter = guid::counter(i);
    p.row.state.ignored.retain(|c| *c != counter);
    realm.social.lists.lock().unignore(g, i);
    status(&realm, g, IGNORE_REMOVED, i);
}

/// A character deleted: off the friends and ignore lists of those online.
pub fn character_deleted(realm: &Realm, g: Guid) {
    realm.social.lists.lock().forget(g);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ignored_by` is `ignores` the other way round, whatever changes it.
    #[test]
    fn those_ignoring_a_player_follow_the_lists() {
        let (a, b, c) = (guid::player(1), guid::player(2), guid::player(3));
        let mut l = Lists::default();
        let agree = |l: &Lists| {
            for g in [a, b, c] {
                let by: AHashSet<Guid> =
                    [a, b, c].into_iter().filter(|o| l.ignores(*o, g)).collect();
                assert_eq!(l.ignorers(g), by, "those ignoring {g:?}");
            }
        };
        l.ignore(a, c);
        l.ignore(b, c);
        agree(&l);
        assert_eq!(l.ignorers(c).len(), 2);
        l.unignore(a, c);
        agree(&l);
        l.set_ignores(b, [a].into_iter().collect());
        agree(&l);
        assert!(l.ignorers(c).is_empty());
        l.set_ignores(c, [a, b].into_iter().collect());
        l.drop_ignores(b);
        agree(&l);
        assert_eq!(l.ignorers(a), [c].into_iter().collect());
        // Deleted: off every list, and its own gone.
        l.forget(a);
        agree(&l);
        l.forget(c);
        agree(&l);
        assert!(l.ignored_by.is_empty());
    }

    /// `friended_by` is `friends` the other way round, whatever changes it.
    #[test]
    fn those_with_a_player_as_a_friend_follow_the_lists() {
        let (a, b, c) = (guid::player(1), guid::player(2), guid::player(3));
        let mut l = Lists::default();
        let agree = |l: &Lists| {
            for g in [a, b, c] {
                let by: AHashSet<Guid> = [a, b, c]
                    .into_iter()
                    .filter(|o| l.friends.get(o).is_some_and(|f| f.contains(&g)))
                    .collect();
                let watchers: AHashSet<Guid> = l.watchers(g).into_iter().collect();
                assert_eq!(watchers, by, "those with {g:?} as a friend");
            }
        };
        l.befriend(a, c);
        l.befriend(b, c);
        agree(&l);
        assert_eq!(l.watchers(c).len(), 2);
        l.unfriend(a, c);
        agree(&l);
        l.set_friends(b, [a].into_iter().collect());
        agree(&l);
        assert!(l.watchers(c).is_empty());
        l.set_friends(c, [a, b].into_iter().collect());
        l.drop_friends(b);
        agree(&l);
        assert_eq!(l.watchers(a), vec![c]);
        l.forget(a);
        agree(&l);
        l.forget(c);
        agree(&l);
        assert!(l.friended_by.is_empty());
    }

    /// A login storm asked each login of every online player's friend
    /// list, under the lock every whisper and channel line takes: 3,000
    /// logins of players with 50 friends each were 9 million lookups.
    #[test]
    fn a_login_storm_asks_each_login_once() {
        let mut l = Lists::default();
        let n = 3000u32;
        for i in 0..n {
            let friends = (1..=50).map(|k| guid::player((i + k) % n)).collect();
            l.set_friends(guid::player(i), friends);
        }
        let started = std::time::Instant::now();
        let told: usize = (0..n).map(|i| l.watchers(guid::player(i)).len()).sum();
        let took = started.elapsed();
        assert_eq!(told, 50 * n as usize);
        assert!(took < std::time::Duration::from_millis(100), "{took:?}");
    }
}
