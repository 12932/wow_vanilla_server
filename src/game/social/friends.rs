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
}

impl Lists {
    /// Whether `owner` ignores `other`.
    pub fn ignores(&self, owner: Guid, other: Guid) -> bool {
        self.ignores.get(&owner).is_some_and(|s| s.contains(&other))
    }

    /// Those online with `g` as a friend.
    fn watchers(&self, g: Guid) -> Vec<Guid> {
        self.friends
            .iter()
            .filter(|(_, f)| f.contains(&g))
            .map(|(owner, _)| *owner)
            .collect()
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

/// `CMSG_FRIEND_LIST`: the friend and ignore lists, as mangos's
/// HandleFriendListOpcode and Thistle Tea send them.
pub fn send_lists(map: &Map, g: Guid) {
    let realm = &map.realm;
    let (Some(p), Some(me)) = (map.players.get(&g), realm.registry.online(g)) else {
        return;
    };
    p.outbound.send(&SMSG_FRIEND_LIST {
        friends: p
            .row
            .state
            .friends
            .iter()
            .map(|c| friend_entry(realm, &me, guid::player(*c)))
            .collect(),
    });
    p.outbound.send(&SMSG_IGNORE_LIST {
        ignored: p
            .row
            .state
            .ignored
            .iter()
            .map(|c| guid::player(*c).guid())
            .collect(),
    });
}

/// `CMSG_CHAT_IGNORED`: the player's client didn't show `speaker`'s line,
/// ignoring them; they are told so, as mangos's HandleChatIgnoredOpcode
/// and Thistle Tea's `Social.ignored` tell them, if the player does ignore
/// them.
pub fn chat_ignored(map: &Map, g: Guid, speaker: Guid) {
    let realm = &map.realm;
    if !realm.social.lists.lock().ignores(g, speaker) {
        return;
    }
    let (Some(p), Some(o)) = (map.players.get(&g), realm.registry.online(speaker)) else {
        return;
    };
    o.outbound
        .send(&wow_world_messages::vanilla::SMSG_MESSAGECHAT {
            chat_type: wow_world_messages::vanilla::SMSG_MESSAGECHAT_ChatType::Ignored {
                sender2: g,
            },
            language: wow_world_messages::vanilla::Language::Universal,
            message: p.name.to_string(),
            tag: wow_world_messages::vanilla::PlayerChatTag::None,
        });
}

/// At login: the lists are sent, and those with the player as a friend
/// told.
pub fn logged_in(map: &mut Map, g: Guid) {
    let realm = map.realm.clone();
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let Some(me) = realm.registry.online(g) else {
        return;
    };
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
    let watchers = {
        let mut lists = realm.social.lists.lock();
        lists.friends.insert(g, friends);
        lists.ignores.insert(g, ignores);
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

/// Going offline: those with the player as a friend are told.
pub fn logged_out(realm: &Realm, g: Guid) {
    let watchers = {
        let mut lists = realm.social.lists.lock();
        lists.friends.remove(&g);
        lists.ignores.remove(&g);
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
        realm
            .social
            .lists
            .lock()
            .friends
            .entry(g)
            .or_default()
            .insert(f);
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
    if let Some(s) = realm.social.lists.lock().friends.get_mut(&g) {
        s.remove(&f);
    }
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
        realm
            .social
            .lists
            .lock()
            .ignores
            .entry(g)
            .or_default()
            .insert(i);
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
    if let Some(s) = realm.social.lists.lock().ignores.get_mut(&g) {
        s.remove(&i);
    }
    status(&realm, g, IGNORE_REMOVED, i);
}

/// A character deleted: off the friends and ignore lists of those online.
pub fn character_deleted(realm: &Realm, g: Guid) {
    let mut lists = realm.social.lists.lock();
    for f in lists.friends.values_mut() {
        f.remove(&g);
    }
    for i in lists.ignores.values_mut() {
        i.remove(&g);
    }
    lists.friends.remove(&g);
    lists.ignores.remove(&g);
}
