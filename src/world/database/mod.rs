use crate::world::world_opcode_handler::character::Character;
use ahash::AHashMap;
use wow_world_base::vanilla::{PlayerGender, RaceClass};
use wow_world_messages::Guid;

#[derive(Debug, Clone)]
pub struct WorldDatabase {
    characters_for_all_accounts: Vec<Character>,
    /// Reverse index from character guid to row index. Maintained in lockstep
    /// with `characters_for_all_accounts` on every push/replace/remove. Routes
    /// `get` / `replace` / `delete` to O(1) instead of a linear scan.
    by_guid: AHashMap<Guid, usize>,
    /// Reverse index from account name to row indices. CMSG_CHAR_ENUM at
    /// burst-login scale (one per bot, all in one tick) would otherwise
    /// linear-scan the whole character table per call.
    by_account: AHashMap<String, Vec<usize>>,
    next_guid: u64,
}

impl Default for WorldDatabase {
    fn default() -> Self {
        Self::new()
    }
}

/// Canonical account name for the seeded test characters (`Dev`, `HumOne`,
/// `HumTwo`). Real WoW clients uppercase the account name before sending,
/// so a user typing "Dev" lands on this account.
pub const DEV_ACCOUNT: &str = "DEV";

impl WorldDatabase {
    pub fn new() -> Self {
        let mut db = Self {
            characters_for_all_accounts: vec![],
            by_guid: AHashMap::new(),
            by_account: AHashMap::new(),
            next_guid: 0,
        };

        let c = Character::test_character(
            &mut db,
            "Dev",
            RaceClass::HumanWarrior,
            PlayerGender::Female,
        );
        db.create_character_in_account(DEV_ACCOUNT, c);
        let c = Character::test_character(
            &mut db,
            "HumOne",
            RaceClass::HumanWarrior,
            PlayerGender::Female,
        );
        db.create_character_in_account(DEV_ACCOUNT, c);
        let c = Character::test_character(
            &mut db,
            "HumTwo",
            RaceClass::HumanWarrior,
            PlayerGender::Male,
        );
        db.create_character_in_account(DEV_ACCOUNT, c);

        db
    }

    pub fn get_characters_for_account(&self, account_name: &str) -> Vec<Character> {
        let Some(indices) = self.by_account.get(account_name) else {
            return Vec::new();
        };
        indices
            .iter()
            .filter_map(|&i| self.characters_for_all_accounts.get(i).cloned())
            .collect()
    }

    pub fn create_character_in_account(&mut self, account_name: &str, mut character: Character) {
        character.account = account_name.to_string();
        let guid = character.guid;
        let idx = self.characters_for_all_accounts.len();
        self.characters_for_all_accounts.push(character);
        self.by_guid.insert(guid, idx);
        self.by_account
            .entry(account_name.to_string())
            .or_default()
            .push(idx);
    }

    /// Returns the character only if it belongs to the given account.
    /// Defense in depth — refuses `CMSG_PLAYER_LOGIN` for guids that belong
    /// to someone else's account, even if the requesting client somehow
    /// learned the foreign guid.
    pub fn get_character_for_account(&self, account_name: &str, guid: Guid) -> Option<Character> {
        let idx = *self.by_guid.get(&guid)?;
        let c = self.characters_for_all_accounts.get(idx)?;
        if c.account == account_name {
            Some(c.clone())
        } else {
            None
        }
    }

    pub fn new_guid(&mut self) -> u64 {
        let g = self.next_guid;
        self.next_guid += 1;
        g
    }

    pub fn next_guid(&self) -> u64 {
        self.next_guid
    }

    pub fn all_characters(&self) -> &[Character] {
        &self.characters_for_all_accounts
    }

    pub fn from_parts(characters: Vec<Character>, next_guid: u64) -> Self {
        let by_guid = characters
            .iter()
            .enumerate()
            .map(|(i, c)| (c.guid, i))
            .collect();
        let mut by_account: AHashMap<String, Vec<usize>> = AHashMap::new();
        for (i, c) in characters.iter().enumerate() {
            by_account.entry(c.account.clone()).or_default().push(i);
        }
        Self {
            characters_for_all_accounts: characters,
            by_guid,
            by_account,
            next_guid,
        }
    }

    /// Returns the character with the given guid, or `None` if no such guid
    /// is in the DB. Callers must reply to the client with the appropriate
    /// `CharLoginFailed` / error response instead of panicking — a malformed
    /// or stale guid from any client used to take down the world task.
    pub fn get_character_by_guid(&self, guid: Guid) -> Option<Character> {
        let idx = *self.by_guid.get(&guid)?;
        self.characters_for_all_accounts.get(idx).cloned()
    }

    /// Returns `true` if the row was found and replaced.
    pub fn replace_character_data(&mut self, c: Character) -> bool {
        let Some(idx) = self.by_guid.get(&c.guid).copied() else {
            return false;
        };
        self.characters_for_all_accounts[idx] = c;
        true
    }

    /// Returns `true` if a row was removed. Refuses to delete a character
    /// that doesn't belong to the requesting account.
    pub fn delete_character_by_guid(&mut self, account_name: &str, guid: Guid) -> bool {
        let Some(&idx) = self.by_guid.get(&guid) else {
            return false;
        };
        if self.characters_for_all_accounts[idx].account != account_name {
            return false;
        }
        self.by_guid.remove(&guid);
        // Drop `idx` from the deleted character's account bucket.
        if let Some(bucket) = self.by_account.get_mut(account_name)
            && let Some(pos) = bucket.iter().position(|&i| i == idx)
        {
            bucket.swap_remove(pos);
        }
        let last_idx = self.characters_for_all_accounts.len() - 1;
        self.characters_for_all_accounts.swap_remove(idx);
        // `swap_remove` moved the formerly-last row into position `idx` (if
        // there was one). Update both reverse maps: the moved row's guid now
        // points to `idx`, and the moved row's account bucket entry for
        // `last_idx` needs to be rewritten to `idx`.
        if let Some(moved) = self.characters_for_all_accounts.get(idx) {
            self.by_guid.insert(moved.guid, idx);
            if let Some(bucket) = self.by_account.get_mut(&moved.account)
                && let Some(pos) = bucket.iter().position(|&i| i == last_idx)
            {
                bucket[pos] = idx;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Building a fresh DB and looking up its seeded characters should hit
    /// the index, not the linear-scan fallback.
    #[test]
    fn lookup_finds_seeded_characters() {
        let db = WorldDatabase::new();
        let first = db
            .all_characters()
            .first()
            .expect("seeded with at least one character");
        let guid = first.guid;
        let found = db.get_character_by_guid(guid).expect("guid must resolve");
        assert_eq!(found.guid, guid);
    }

    #[test]
    fn lookup_missing_returns_none() {
        let db = WorldDatabase::new();
        assert!(db.get_character_by_guid(Guid::new(99_999)).is_none());
    }

    #[test]
    fn delete_then_lookup_is_none() {
        let mut db = WorldDatabase::new();
        let guid = db.all_characters().first().unwrap().guid;
        assert!(db.delete_character_by_guid(DEV_ACCOUNT, guid));
        assert!(db.get_character_by_guid(guid).is_none());
    }

    #[test]
    fn delete_keeps_other_indexes_consistent() {
        // Removing the first character should still leave the others reachable.
        let mut db = WorldDatabase::new();
        let all_guids: Vec<Guid> = db.all_characters().iter().map(|c| c.guid).collect();
        let first = all_guids[0];
        let rest = &all_guids[1..];
        assert!(db.delete_character_by_guid(DEV_ACCOUNT, first));
        for g in rest {
            assert!(
                db.get_character_by_guid(*g).is_some(),
                "remaining guid {g:?} must still resolve after delete"
            );
        }
    }

    #[test]
    fn char_enum_filters_by_account() {
        let mut db = WorldDatabase::new();
        // Add a foreign-account character.
        let bot = Character::test_character(
            &mut db,
            "Botxyz",
            RaceClass::OrcWarrior,
            PlayerGender::Male,
        );
        db.create_character_in_account("BOT0000", bot);

        let dev_chars = db.get_characters_for_account(DEV_ACCOUNT);
        assert!(dev_chars.iter().any(|c| c.name == "Dev"));
        assert!(!dev_chars.iter().any(|c| c.name == "Botxyz"));

        let bot_chars = db.get_characters_for_account("BOT0000");
        assert_eq!(bot_chars.len(), 1);
        assert_eq!(bot_chars[0].name, "Botxyz");
    }

    #[test]
    fn get_character_for_account_refuses_foreign_guid() {
        let db = WorldDatabase::new();
        let dev_guid = db
            .all_characters()
            .iter()
            .find(|c| c.name == "Dev")
            .unwrap()
            .guid;
        // Some other account should not be able to load DEV's character.
        assert!(db.get_character_for_account("BOT0001", dev_guid).is_none());
        assert!(db.get_character_for_account(DEV_ACCOUNT, dev_guid).is_some());
    }
}
