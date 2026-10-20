//! Who exists and who is online, across every map.
//!
//! Every character's name is here from start-up, so a name query is
//! answered from memory. A player online also has an entry with their
//! outbound queue, their session and their map, so a whisper, a group
//! invite or a service can reach them wherever they are.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

use ahash::AHashMap;
use parking_lot::{Mutex, RwLock};
use tokio::sync::mpsc;
use wow_world_messages::Guid;

use crate::game::conn::Outbound;
use crate::game::map::MapHandle;
use crate::game::session::Control;

/// A character, online or not.
#[derive(Debug)]
pub struct NameEntry {
    pub guid: Guid,
    pub account: Arc<str>,
    pub name: Arc<str>,
    pub race: u8,
    pub class: u8,
    pub gender: u8,
    pub level: AtomicU8,
}

/// A player in the world.
#[derive(Debug)]
pub struct Online {
    pub guid: Guid,
    pub account: Arc<str>,
    pub name: Arc<str>,
    pub race: u8,
    pub class: u8,
    pub gender: u8,
    pub level: AtomicU8,
    pub zone: AtomicU32,
    /// Chat flags: AFK, DND, GM.
    pub flags: AtomicU32,
    /// The AFK or DND message.
    pub away: Mutex<String>,
    pub gm: bool,
    /// The account's GM level: 0 a player, 1 a moderator, 2 a game master, 3
    /// an administrator.
    pub gm_level: u8,
    pub outbound: Outbound,
    pub session: mpsc::UnboundedSender<Control>,
    map: Mutex<MapHandle>,
}

impl Online {
    pub fn new(
        name: &NameEntry,
        gm_level: u8,
        outbound: Outbound,
        session: mpsc::UnboundedSender<Control>,
        map: MapHandle,
    ) -> Self {
        Self {
            guid: name.guid,
            account: name.account.clone(),
            name: name.name.clone(),
            race: name.race,
            class: name.class,
            gender: name.gender,
            level: AtomicU8::new(name.level.load(Ordering::Relaxed)),
            zone: AtomicU32::new(0),
            flags: AtomicU32::new(0),
            away: Mutex::new(String::new()),
            gm: gm_level >= crate::accounts::GAME_MASTER,
            gm_level,
            outbound,
            session,
            map: Mutex::new(map),
        }
    }

    pub fn map(&self) -> MapHandle {
        self.map.lock().clone()
    }

    pub fn set_map(&self, map: MapHandle) {
        *self.map.lock() = map;
    }

    pub fn zone(&self) -> u32 {
        self.zone.load(Ordering::Relaxed)
    }

    pub fn level(&self) -> u8 {
        self.level.load(Ordering::Relaxed)
    }
}

/// The two factions, by race.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Team {
    Alliance,
    Horde,
}

impl Team {
    pub fn of_race(race: u8) -> Team {
        // Human, Dwarf, Night Elf, Gnome.
        if matches!(race, 1 | 3 | 4 | 7) {
            Team::Alliance
        } else {
            Team::Horde
        }
    }
}

#[derive(Debug, Default)]
pub struct Registry {
    names: RwLock<AHashMap<Guid, Arc<NameEntry>>>,
    by_name: RwLock<AHashMap<String, Guid>>,
    online: RwLock<AHashMap<Guid, Arc<Online>>>,
    by_account: RwLock<AHashMap<Arc<str>, Guid>>,
    /// Each account's world session (its control channel), on the
    /// character screen or in the world: one an account, as mangos's
    /// `World::m_sessions` keeps them.
    sessions: Mutex<AHashMap<Arc<str>, mpsc::UnboundedSender<Control>>>,
}

fn key(name: &str) -> String {
    name.to_lowercase()
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_name(&self, entry: NameEntry) -> Arc<NameEntry> {
        let entry = Arc::new(entry);
        self.by_name.write().insert(key(&entry.name), entry.guid);
        self.names.write().insert(entry.guid, entry.clone());
        entry
    }

    pub fn remove_name(&self, guid: Guid) {
        if let Some(e) = self.names.write().remove(&guid) {
            self.by_name.write().remove(&key(&e.name));
        }
    }

    pub fn name(&self, guid: Guid) -> Option<Arc<NameEntry>> {
        self.names.read().get(&guid).cloned()
    }

    pub fn guid_by_name(&self, name: &str) -> Option<Guid> {
        self.by_name.read().get(&key(name)).copied()
    }

    pub fn name_in_use(&self, name: &str) -> bool {
        self.by_name.read().contains_key(&key(name))
    }

    pub fn character_count(&self) -> usize {
        self.names.read().len()
    }

    /// Put a player online. Fails if the character, or another character of
    /// the account, already is.
    pub fn go_online(&self, online: Arc<Online>) -> Result<(), Arc<Online>> {
        let mut by_account = self.by_account.write();
        let mut map = self.online.write();
        if let Some(existing) = by_account
            .get(&online.account)
            .and_then(|g| map.get(g))
            .or_else(|| map.get(&online.guid))
        {
            return Err(existing.clone());
        }
        by_account.insert(online.account.clone(), online.guid);
        map.insert(online.guid, online);
        Ok(())
    }

    /// `new` in `old`'s place, the same character from another session: if
    /// `old` is still the entry.
    pub fn replace(&self, old: &Arc<Online>, new: Arc<Online>) -> bool {
        let by_account = self.by_account.read();
        let mut map = self.online.write();
        let current = map.get(&old.guid).is_some_and(|o| Arc::ptr_eq(o, old));
        if !current || new.guid != old.guid || by_account.get(&new.account) != Some(&old.guid) {
            return false;
        }
        map.insert(new.guid, new);
        true
    }

    pub fn go_offline(&self, guid: Guid) {
        let mut by_account = self.by_account.write();
        if let Some(o) = self.online.write().remove(&guid)
            && by_account.get(&o.account) == Some(&guid)
        {
            by_account.remove(&o.account);
        }
    }

    pub fn online(&self, guid: Guid) -> Option<Arc<Online>> {
        self.online.read().get(&guid).cloned()
    }

    pub fn online_by_name(&self, name: &str) -> Option<Arc<Online>> {
        let guid = self.guid_by_name(name)?;
        self.online(guid)
    }

    /// `session` is the account's session from now: the one it had before,
    /// if any, to be let go.
    pub fn claim_session(
        &self,
        account: &Arc<str>,
        session: &mpsc::UnboundedSender<Control>,
    ) -> Option<mpsc::UnboundedSender<Control>> {
        self.sessions
            .lock()
            .insert(account.clone(), session.clone())
            .filter(|old| !old.same_channel(session))
    }

    /// The account's session goes, if it is still `session`: true if it
    /// was.
    pub fn release_session(&self, account: &str, session: &mpsc::UnboundedSender<Control>) -> bool {
        let mut sessions = self.sessions.lock();
        if sessions
            .get(account)
            .is_some_and(|s| s.same_channel(session))
        {
            sessions.remove(account);
            return true;
        }
        false
    }

    /// The account's session, on the character screen or in the world.
    pub fn session_for_account(&self, account: &str) -> Option<mpsc::UnboundedSender<Control>> {
        self.sessions.lock().get(account).cloned()
    }

    pub fn online_for_account(&self, account: &str) -> Option<Arc<Online>> {
        let guid = *self.by_account.read().get(account)?;
        self.online(guid)
    }

    pub fn online_count(&self) -> usize {
        self.online.read().len()
    }

    /// Everyone online, for a broadcast or a `/who`.
    pub fn all_online(&self) -> Vec<Arc<Online>> {
        self.online.read().values().cloned().collect()
    }

    pub fn set_level(&self, guid: Guid, level: u8) {
        if let Some(n) = self.name(guid) {
            n.level.store(level, Ordering::Relaxed);
        }
        if let Some(o) = self.online(guid) {
            o.level.store(level, Ordering::Relaxed);
        }
    }
}
