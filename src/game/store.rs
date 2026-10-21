//! The character store: `characters.sqlite`, behind one thread.
//!
//! Every read and write goes through the store's thread in order, so a
//! read after a save (the character list after a logout, a login after a
//! save) sees it. Saves are fire-and-forget and are applied in batches, in
//! one transaction. The schema is versioned with `PRAGMA user_version` and
//! migrated at start-up.
//!
//! A character's columns hold what is listed or looked up; the rest of its
//! state is JSON in `state`, whose fields all have defaults, so adding one
//! needs no migration.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::thread;

use rusqlite::{Connection, OptionalExtension, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info, warn};

use crate::game::supervisor::catch;

/// The schema's migrations, in order: migration `i` takes the schema from
/// version `i` to `i + 1`.
const MIGRATIONS: &[&str] = &[
    // 1: characters, their items, spells and action bars; account data.
    r#"
    CREATE TABLE characters (
        guid INTEGER PRIMARY KEY,
        account TEXT NOT NULL,
        name TEXT NOT NULL UNIQUE COLLATE NOCASE,
        race INTEGER NOT NULL,
        class INTEGER NOT NULL,
        gender INTEGER NOT NULL,
        skin INTEGER NOT NULL,
        face INTEGER NOT NULL,
        hair_style INTEGER NOT NULL,
        hair_color INTEGER NOT NULL,
        facial_hair INTEGER NOT NULL,
        level INTEGER NOT NULL,
        xp INTEGER NOT NULL,
        money INTEGER NOT NULL,
        map INTEGER NOT NULL,
        x REAL NOT NULL,
        y REAL NOT NULL,
        z REAL NOT NULL,
        orientation REAL NOT NULL,
        zone INTEGER NOT NULL,
        played_total INTEGER NOT NULL,
        played_level INTEGER NOT NULL,
        logout_time INTEGER NOT NULL,
        at_login INTEGER NOT NULL,
        state TEXT NOT NULL
    );
    CREATE INDEX characters_account ON characters (account);
    CREATE TABLE character_items (
        guid INTEGER PRIMARY KEY,
        owner INTEGER NOT NULL,
        entry INTEGER NOT NULL,
        bag INTEGER NOT NULL,
        slot INTEGER NOT NULL,
        count INTEGER NOT NULL,
        durability INTEGER NOT NULL,
        flags INTEGER NOT NULL,
        data TEXT NOT NULL
    );
    CREATE INDEX character_items_owner ON character_items (owner);
    CREATE TABLE character_spells (
        owner INTEGER NOT NULL,
        spell INTEGER NOT NULL,
        PRIMARY KEY (owner, spell)
    );
    CREATE TABLE character_actions (
        owner INTEGER NOT NULL,
        button INTEGER NOT NULL,
        action INTEGER NOT NULL,
        type INTEGER NOT NULL,
        PRIMARY KEY (owner, button)
    );
    CREATE TABLE account_data (
        account TEXT NOT NULL,
        type INTEGER NOT NULL,
        time INTEGER NOT NULL,
        data BLOB NOT NULL,
        PRIMARY KEY (account, type)
    );
    "#,
    // 2: guilds, petitions, mail and auctions, each a JSON record by kind and id.
    r#"
    CREATE TABLE records (
        kind TEXT NOT NULL,
        id INTEGER NOT NULL,
        data TEXT NOT NULL,
        PRIMARY KEY (kind, id)
    );
    "#,
    // 3: an item's spell charges, five numbers apart by spaces as mangos's
    // `item_instance.charges`; NULL for an item saved before, which loads
    // with its template's.
    r#"
    ALTER TABLE character_items ADD COLUMN charges TEXT;
    "#,
    // 4: bones lying in the open world (`map::bones`), kept across a
    // restart: where they lie, whose they were, what a client shows of
    // them, and when (Unix seconds) they go.
    r#"
    CREATE TABLE bones (
        id INTEGER PRIMARY KEY,
        map INTEGER NOT NULL,
        x REAL NOT NULL,
        y REAL NOT NULL,
        z REAL NOT NULL,
        orientation REAL NOT NULL,
        owner INTEGER NOT NULL,
        display INTEGER NOT NULL,
        bytes_1 INTEGER NOT NULL,
        bytes_2 INTEGER NOT NULL,
        guild INTEGER NOT NULL,
        flags INTEGER NOT NULL,
        dynamic_flags INTEGER NOT NULL,
        expires_at INTEGER NOT NULL
    );
    "#,
    // 5: the next character and item GUIDs to hand out, by name
    // (`character`, `item`), kept with every write: a GUID is never given
    // twice, though what had it is gone (a deleted character, an item
    // destroyed), and what is still kept under it is never inherited.
    r#"
    CREATE TABLE counters (
        name TEXT PRIMARY KEY,
        next INTEGER NOT NULL
    );
    "#,
];

pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

/// The bag of items in the player's own slots (equipment, bags, backpack,
/// bank).
pub const BAG_PLAYER: u8 = 255;

/// A character's state beyond its columns. Every field has a default.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CharacterState {
    pub health: u32,
    pub powers: [u32; 5],
    pub tutorials: [u32; 8],
    /// The client's caches, loaded at login (kept in `account_data`).
    #[serde(skip)]
    pub caches: Vec<(u8, Vec<u8>)>,
    pub action_bar_toggles: u8,
    /// The bank's bag slots bought (`PLAYER_BYTES_2`'s third byte).
    pub bank_bag_slots: u8,
    pub explored_zones: Vec<u32>,
    pub taxi_mask: Vec<u32>,
    pub bind: Option<Bind>,
    pub flags: u32,
    pub dead: bool,
    /// Where the corpse of a released character lies.
    pub corpse: Option<CorpseState>,
    /// Until when (Unix ms) repeated deaths lengthen the wait to reclaim a
    /// corpse, and when (Unix ms, 0 for not) the spirit was released.
    pub reclaim_until: i64,
    pub released_at: i64,
    pub quests: Journal,
    /// Friends and ignored characters, by GUID counter.
    pub friends: Vec<u32>,
    pub ignored: Vec<u32>,
    /// A flight under way at logout, to take up again.
    pub flight: Option<SavedFlight>,
    /// Aboard a transport at logout: its entry, and the place on deck
    /// (x, y, z, facing).
    pub transport: Option<(u32, f32, f32, f32, f32)>,
    /// Auras and cooldowns kept across a logout.
    pub auras: Vec<crate::game::map::spells::cooldowns::SavedAura>,
    pub cooldowns: Vec<(u32, u32)>,
    pub category_cooldowns: Vec<(u32, u32)>,
    /// Where those came from: (spell, the item's entry or 0, category).
    pub cooldown_sources: Vec<(u32, u32, u32)>,
    /// Standings with factions, as offsets from the base.
    pub reputation: Vec<crate::game::map::reputation::Standing>,
    /// The reputation index watched on the experience bar.
    pub watched_faction: Option<u32>,
    /// How drunk the player was at logout (`map::intoxication`).
    pub drunk: u32,
    /// The ammunition chosen (`PLAYER_AMMO_ID`), as mangos keeps `ammoId`.
    pub ammo: u32,
    /// Rested XP waiting, and the rest the player was in at logout.
    pub rest_bonus: f32,
    pub rest_type: Option<crate::game::map::rest::RestType>,
    /// Talent resets: how many, and when the last was (Unix seconds).
    pub talent_wipes: u32,
    pub last_talent_wipe: i64,
    /// The hunter's pets and the warlock's demon.
    pub pets: PetsState,
    /// Each skill's value (skill, value): its cap and step come again from
    /// the level and the spells. None for a character saved before skills
    /// were, whose weapon skills stay at their caps (`skills::init`).
    pub skills: Option<Vec<(u32, u32)>>,
    /// The weapon skills a forgotten spell took (skill, value), each back
    /// at its value when the spell is learned again (mangos's
    /// `m_forgottenSkills`, 1.11's rule for Two-Handed Axes and Maces).
    pub forgotten_skills: Vec<(u32, u32)>,
}

/// A pet as it is kept: the one out (or dismissed), and those stabled.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PetRecord {
    pub number: u32,
    pub entry: u32,
    /// 1 a hunter's pet, 2 a summoned demon.
    pub kind: u8,
    pub name: String,
    pub renamed: bool,
    pub level: u8,
    pub xp: u32,
    pub health: u32,
    pub power: u32,
    pub happiness: u32,
    pub loyalty: u8,
    pub loyalty_points: i32,
    pub training_points: i32,
    pub spells: Vec<u32>,
    pub autocast: Vec<u32>,
    pub bar: Vec<u32>,
    pub react: u8,
    pub command: u8,
    pub dead: bool,
    pub summon_spell: u32,
    pub display_id: u32,
    pub family: u32,
    /// Spells still cooling down, and until when (Unix milliseconds).
    pub cooldowns: Vec<(u32, i64)>,
}

/// A character's pets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PetsState {
    /// The current pet: out, or dismissed (a hunter's).
    pub current: Option<PetRecord>,
    /// Whether the current pet was out, to bring back on login.
    pub active: bool,
    /// The stabled pets, each with its slot (1 or 2).
    pub stable: Vec<(u8, PetRecord)>,
    pub stable_slots: u8,
    pub next_number: u32,
    /// What untraining the pet last cost, and when (Unix seconds).
    pub untrain_cost: u32,
    pub untrain_at: i64,
    /// The names the warlock's demons drew, by entry: each keeps its own
    /// when summoned again (mangos's `character_pet`, Thistle Tea's
    /// remembered names).
    pub demon_names: Vec<(u32, String)>,
}

/// A flight's rest: the points still to fly, where it lands, and the
/// mount.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedFlight {
    pub map: u32,
    pub points: Vec<(f32, f32, f32)>,
    pub destination: u32,
    pub mount: u32,
}

/// A quest in a player's log.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Logged {
    pub quest: u32,
    pub kills: [u32; 4],
    pub explored: bool,
    pub failed: bool,
    /// The quest's reputation objective is met.
    pub rep_met: bool,
    /// A timed quest's end, in Unix seconds; 0 for none.
    #[serde(default)]
    pub expires: i64,
}

/// A player's quests: the log, and those rewarded.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Journal {
    pub log: Vec<Logged>,
    pub rewarded: std::collections::BTreeSet<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct CorpseState {
    pub map: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub orientation: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Bind {
    pub map: u32,
    pub zone: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemRow {
    pub guid: u32,
    pub entry: u32,
    pub bag: u8,
    pub slot: u8,
    pub count: u32,
    pub durability: u32,
    pub flags: u32,
    pub data: String,
    /// The charges left of each of the item's five spells
    /// (`ITEM_FIELD_SPELL_CHARGES`); `None` for an item kept before they
    /// were, which has its template's.
    pub charges: Option<[i32; 5]>,
}

impl ItemRow {
    /// `charges` as the store's column keeps them: `"a b c d e"`.
    pub fn charges_text(&self) -> Option<String> {
        self.charges
            .map(|c| c.iter().map(i32::to_string).collect::<Vec<_>>().join(" "))
    }

    /// The column read back: five numbers, or none (mangos's
    /// `LoadFromDB` takes them only when there are five).
    pub fn parse_charges(text: Option<&str>) -> Option<[i32; 5]> {
        let v: Vec<i32> = text?
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        v.try_into().ok()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ActionRow {
    pub button: u8,
    pub action: u16,
    pub kind: u8,
}

/// A character as stored.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CharacterRow {
    pub guid: u32,
    pub account: String,
    pub name: String,
    pub race: u8,
    pub class: u8,
    pub gender: u8,
    pub skin: u8,
    pub face: u8,
    pub hair_style: u8,
    pub hair_color: u8,
    pub facial_hair: u8,
    pub level: u8,
    pub xp: u32,
    pub money: u32,
    pub map: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub orientation: f32,
    pub zone: u32,
    pub played_total: u32,
    pub played_level: u32,
    pub logout_time: i64,
    pub at_login: u32,
    pub state: CharacterState,
    pub items: Vec<ItemRow>,
    pub spells: Vec<u32>,
    pub actions: Vec<ActionRow>,
}

/// `at_login` flag: the character has never entered the world.
pub const AT_LOGIN_FIRST: u32 = 0x1;

/// What the store knows at start-up: every character's name, and the
/// highest GUIDs in use or ever handed out (the `counters`).
#[derive(Debug, Default)]
pub struct Startup {
    pub names: Vec<NameRow>,
    pub max_character_guid: u32,
    pub max_item_guid: u32,
    /// Every record: kind, id and JSON.
    pub records: Vec<(String, u64, String)>,
    /// The bones yet to go (those gone were swept as the store opened),
    /// and the highest of their ids.
    pub bones: Vec<BonesRow>,
    pub max_bones_id: u32,
}

/// Bones lying in the open world, as kept (`map::bones`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BonesRow {
    /// The bones' id, unique in the realm: their GUID's counter.
    pub id: u32,
    pub map: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub orientation: f32,
    /// Whose they were, by GUID counter.
    pub owner: u32,
    /// `CORPSE_FIELD_DISPLAY_ID`, `CORPSE_FIELD_BYTES_1` (race, gender,
    /// skin), `CORPSE_FIELD_BYTES_2` (face, hair style and colour, facial
    /// hair), `CORPSE_FIELD_GUILD`, `CORPSE_FIELD_FLAGS` and
    /// `CORPSE_FIELD_DYNAMIC_FLAGS`.
    pub display: u32,
    pub bytes_1: u32,
    pub bytes_2: u32,
    pub guild: u32,
    pub flags: u32,
    pub dynamic_flags: u32,
    /// When they go, in Unix seconds.
    pub expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct NameRow {
    pub guid: u32,
    pub account: String,
    pub name: String,
    pub race: u8,
    pub class: u8,
    pub gender: u8,
    pub level: u8,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CreateError {
    NameInUse,
    Failed,
}

enum Request {
    Characters(String, oneshot::Sender<Vec<CharacterRow>>),
    Load(u32, oneshot::Sender<Option<CharacterRow>>),
    Create(Box<CharacterRow>, oneshot::Sender<Result<(), CreateError>>),
    Delete(String, u32, oneshot::Sender<bool>),
    Save(Box<CharacterRow>),
    AccountData(String, oneshot::Sender<Vec<(u8, u32, Vec<u8>)>>),
    SetAccountData(String, u8, u32, Vec<u8>),
    PutRecord(&'static str, u64, String),
    DeleteRecord(&'static str, u64),
    PutBones(Box<BonesRow>),
    DeleteBones(u32),
    /// Characters and records an item or money moved between: written in
    /// one transaction, never apart.
    Together(Vec<CharacterRow>, Vec<RecordOp>),
    /// The realm's next character and item GUIDs, to keep with each write.
    KeepCounters(Arc<AtomicU32>, Arc<AtomicU32>),
    Flush(oneshot::Sender<()>),
    /// A record as stored, for the tests to read back.
    #[cfg(test)]
    Record(&'static str, u64, oneshot::Sender<Option<String>>),
    /// The bones as stored, for the tests.
    #[cfg(test)]
    Bones(oneshot::Sender<Vec<BonesRow>>),
    /// The thread panics on it, as a bug would.
    #[cfg(test)]
    Panic,
    /// The thread sleeps a while, so what comes next queues up.
    #[cfg(test)]
    Sleep(std::time::Duration),
    /// A save whose writing panics.
    #[cfg(test)]
    BadSave,
    /// The store can't write (`PRAGMA query_only`), as on a full disk.
    #[cfg(test)]
    ReadOnly(bool),
}

/// A change to a record, kept to be written with others.
#[derive(Debug)]
enum RecordOp {
    Put(&'static str, u64, String),
    Delete(&'static str, u64),
    /// A change whose writing panics, as a bad one would.
    #[cfg(test)]
    Bad,
}

/// Records changed with characters, for `Store::save_together`: an item or
/// money moved between a character and a mail, an auction or a guild.
/// Each is encoded as it is added, so it is the record as it was then.
#[derive(Debug, Default)]
pub struct Records(Vec<RecordOp>);

impl Records {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keep `value` under its kind and id, as `Store::put_record` does.
    pub fn put<T: Serialize>(&mut self, kind: &'static str, id: u64, value: &T) {
        match serde_json::to_string(value) {
            Ok(data) => self.0.push(RecordOp::Put(kind, id, data)),
            Err(e) => warn!("encoding {kind} {id}: {e}"),
        }
    }

    pub fn delete(&mut self, kind: &'static str, id: u64) {
        self.0.push(RecordOp::Delete(kind, id));
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// A change that fails as it is written, and with it the rest.
    #[cfg(test)]
    pub fn bad_for_test(&mut self) {
        self.0.push(RecordOp::Bad);
    }
}

/// A handle to the store's thread.
#[derive(Clone)]
pub struct Store {
    /// tokio's unbounded `mpsc`, its thread waiting in `blocking_recv`.
    tx: mpsc::UnboundedSender<Request>,
    /// Requests sent and not yet taken by the thread.
    queued: Arc<AtomicUsize>,
    health: Arc<Health>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Store")
    }
}

impl Store {
    /// Open (or create) the store at `path`, migrate it, and start its
    /// thread. `:memory:` gives a store that lasts as long as the process.
    pub fn open(path: impl AsRef<Path>) -> rusqlite::Result<(Store, Startup)> {
        let _z = crate::zone!("load: the character store");
        let conn = Connection::open(path.as_ref())?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        backup_before_migrating(&conn, path.as_ref())?;
        migrate(&conn)?;
        sweep_bones(&conn, crate::game::social::mail::now())?;
        accounts_in_capitals(&conn)?;
        let startup = read_startup(&conn)?;
        info!(
            "character store {}: {} characters, schema version {SCHEMA_VERSION}",
            path.as_ref().display(),
            startup.names.len()
        );
        let (tx, rx) = mpsc::unbounded_channel();
        let queued = Arc::new(AtomicUsize::new(0));
        let health = Arc::new(Health::default());
        {
            let queued = queued.clone();
            let health = health.clone();
            thread::Builder::new()
                .name("store".into())
                .spawn(move || store_thread(conn, rx, &queued, health))
                .expect("spawn the store thread");
        }
        Ok((Store { tx, queued, health }, startup))
    }

    /// Whether the store can't write now: what it couldn't write is kept,
    /// and tried again, and logins wait.
    pub fn failing(&self) -> bool {
        self.health.failing.load(Ordering::Relaxed)
    }

    /// Whether the store started or stopped failing since this was last
    /// asked: `Some(failing)` once for each change, for the GMs.
    pub fn news(&self) -> Option<bool> {
        let failing = self.failing();
        (self.health.told.swap(failing, Ordering::Relaxed) != failing).then_some(failing)
    }

    fn send(&self, req: Request) {
        self.queued.fetch_add(1, Ordering::Relaxed);
        if self.tx.send(req).is_err() {
            self.queued.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Requests waiting for the store's thread.
    pub fn queued(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }

    pub async fn characters(&self, account: &str) -> Vec<CharacterRow> {
        let (tx, rx) = oneshot::channel();
        self.send(Request::Characters(account.to_owned(), tx));
        rx.await.unwrap_or_default()
    }

    pub async fn load(&self, guid: u32) -> Option<CharacterRow> {
        let (tx, rx) = oneshot::channel();
        self.send(Request::Load(guid, tx));
        rx.await.ok().flatten()
    }

    pub async fn create(&self, row: CharacterRow) -> Result<(), CreateError> {
        let (tx, rx) = oneshot::channel();
        self.send(Request::Create(Box::new(row), tx));
        rx.await.unwrap_or(Err(CreateError::Failed))
    }

    pub async fn delete(&self, account: &str, guid: u32) -> bool {
        let (tx, rx) = oneshot::channel();
        self.send(Request::Delete(account.to_owned(), guid, tx));
        rx.await.unwrap_or(false)
    }

    /// Save a character, items, spells and actions included, replacing what
    /// was stored.
    pub fn save(&self, row: CharacterRow) {
        self.send(Request::Save(Box::new(row)));
    }

    /// The account's data blocks: `(type, time, data)`.
    pub async fn account_data(&self, account: &str) -> Vec<(u8, u32, Vec<u8>)> {
        let (tx, rx) = oneshot::channel();
        self.send(Request::AccountData(account.to_owned(), tx));
        rx.await.unwrap_or_default()
    }

    pub fn set_account_data(&self, account: &str, kind: u8, time: u32, data: Vec<u8>) {
        self.send(Request::SetAccountData(
            account.to_owned(),
            kind,
            time,
            data,
        ));
    }

    /// Keep a record (a guild, a mail, an auction) as JSON, replacing what
    /// was kept under its kind and id.
    pub fn put_record<T: Serialize>(&self, kind: &'static str, id: u64, value: &T) {
        match serde_json::to_string(value) {
            Ok(data) => {
                self.send(Request::PutRecord(kind, id, data));
            }
            Err(e) => warn!("encoding {kind} {id}: {e}"),
        }
    }

    pub fn delete_record(&self, kind: &'static str, id: u64) {
        self.send(Request::DeleteRecord(kind, id));
    }

    /// Keep bones, replacing any of their id.
    pub fn put_bones(&self, row: BonesRow) {
        self.send(Request::PutBones(Box::new(row)));
    }

    /// Bones gone.
    pub fn delete_bones(&self, id: u32) {
        self.send(Request::DeleteBones(id));
    }

    /// Save characters and records in one transaction: what moved between
    /// them (an item, money) reaches the database on every side or on
    /// none. A failure is tried again as other saves are, and is never
    /// split into its parts (mangos-classic's `BeginTransaction` around
    /// `SaveInventoryAndGoldToDB` and the mail or auction row).
    pub fn save_together(&self, rows: Vec<CharacterRow>, records: Records) {
        if rows.is_empty() && records.is_empty() {
            return;
        }
        self.send(Request::Together(rows, records.0));
    }

    /// A record as stored (once what was sent before is written).
    #[cfg(test)]
    pub async fn record(&self, kind: &'static str, id: u64) -> Option<String> {
        let (tx, rx) = oneshot::channel();
        self.send(Request::Record(kind, id, tx));
        rx.await.ok().flatten()
    }

    /// The bones as stored (once what was sent before is written).
    #[cfg(test)]
    pub async fn bones_for_test(&self) -> Vec<BonesRow> {
        let (tx, rx) = oneshot::channel();
        self.send(Request::Bones(tx));
        rx.await.unwrap_or_default()
    }

    /// Keep the realm's next character and item GUIDs with every write from
    /// now on, so that none is handed out twice, across restarts too.
    pub fn keep_counters(&self, character: Arc<AtomicU32>, item: Arc<AtomicU32>) {
        self.send(Request::KeepCounters(character, item));
    }

    /// Wait until everything sent before is written.
    pub async fn flush(&self) {
        let (tx, rx) = oneshot::channel();
        self.send(Request::Flush(tx));
        let _ = rx.await;
    }

    #[cfg(test)]
    fn send_test(&self, req: Request) {
        self.send(req);
    }

    /// Make the store's thread panic, as a bug would.
    #[cfg(test)]
    pub fn panic_for_test(&self) {
        self.send_test(Request::Panic);
    }

    /// Hold the store's thread a while, so what is sent meanwhile is
    /// batched.
    #[cfg(test)]
    pub fn hold_for_test(&self, hold: std::time::Duration) {
        self.send_test(Request::Sleep(hold));
    }

    /// A save whose writing panics.
    #[cfg(test)]
    pub fn bad_save_for_test(&self) {
        self.send_test(Request::BadSave);
    }

    /// Make the store unable to write (`on`), as a full disk or a file
    /// made read-only would, or able again.
    #[cfg(test)]
    pub fn read_only_for_test(&self, on: bool) {
        self.send_test(Request::ReadOnly(on));
    }
}

/// A copy of a store about to be migrated, beside it
/// (`characters.sqlite.v4.bak`, a version number past it if one is there
/// already): a migration that goes wrong can be undone by hand.
fn backup_before_migrating(conn: &Connection, path: &Path) -> rusqlite::Result<()> {
    let version: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version == 0 || version >= SCHEMA_VERSION || path == Path::new(":memory:") {
        return Ok(());
    }
    let named = |n: u32| {
        let mut name = path.as_os_str().to_owned();
        match n {
            0 => name.push(format!(".v{version}.bak")),
            n => name.push(format!(".v{version}.{n}.bak")),
        }
        std::path::PathBuf::from(name)
    };
    let Some(to) = (0..1000).map(named).find(|p| !p.exists()) else {
        warn!("the character store has a thousand backups beside it; migrating without one");
        return Ok(());
    };
    info!(
        "copying the character store at schema version {version} to {} before migrating it",
        to.display()
    );
    conn.execute("VACUUM INTO ?1", params![to.to_string_lossy()])?;
    Ok(())
}

/// The store's lock, held for as long as the server runs: one server a
/// store. A second, started before the first has gone, would migrate the
/// store under the first and run the realm's sweeps on its own copy of the
/// records (mail returned and auctions ended twice, copies with players
/// inside deleted).
#[derive(Debug)]
pub struct Lock {
    _file: Option<std::fs::File>,
}

/// Take [`Lock`] on the store at `path` (an advisory lock on
/// `characters.sqlite.lock`, beside it, which the system lets go when the
/// process ends however it ends); an error if another server holds it.
pub fn lock(path: &Path) -> Result<Lock, String> {
    if path == Path::new(":memory:") {
        return Ok(Lock { _file: None });
    }
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    let at = std::path::PathBuf::from(name);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&at)
        .map_err(|e| format!("can't open the store's lock {}: {e}", at.display()))?;
    match file.try_lock() {
        Ok(()) => {
            use std::io::Write;
            let mut f = &file;
            let _ = f
                .set_len(0)
                .and_then(|()| writeln!(f, "{}", std::process::id()));
            Ok(Lock { _file: Some(file) })
        }
        Err(std::fs::TryLockError::WouldBlock) => {
            let pid = std::fs::read_to_string(&at).unwrap_or_default();
            let pid = pid.trim();
            let by = if pid.is_empty() {
                String::new()
            } else {
                format!(" (process {pid})")
            };
            Err(format!(
                "another server{by} is running on the character store {}: stop it first, \
                 or give this one another [world] characters_db",
                path.display()
            ))
        }
        Err(std::fs::TryLockError::Error(e)) => {
            Err(format!("can't lock the store's lock {}: {e}", at.display()))
        }
    }
}

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let version: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > SCHEMA_VERSION {
        panic!(
            "the character store is at schema version {version}, newer than this server's \
             {SCHEMA_VERSION}; refusing to touch it"
        );
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let to = i as u32 + 1;
        info!("migrating the character store to schema version {to}");
        conn.execute_batch(&format!(
            "BEGIN; {sql}; PRAGMA user_version = {to}; COMMIT;"
        ))?;
    }
    Ok(())
}

fn read_startup(conn: &Connection) -> rusqlite::Result<Startup> {
    let mut stmt =
        conn.prepare("SELECT guid, account, name, race, class, gender, level FROM characters")?;
    let names = stmt
        .query_map([], |r| {
            Ok(NameRow {
                guid: r.get(0)?,
                account: r.get(1)?,
                name: r.get(2)?,
                race: r.get(3)?,
                class: r.get(4)?,
                gender: r.get(5)?,
                level: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let handed_out = |name: &str| -> rusqlite::Result<u32> {
        let next: Option<i64> = conn
            .query_row(
                "SELECT next FROM counters WHERE name = ?1",
                params![name],
                |r| r.get(0),
            )
            .optional()?;
        Ok(next.map_or(0, |n| {
            u32::try_from(n.saturating_sub(1).max(0)).unwrap_or(u32::MAX)
        }))
    };
    let max_character_guid: u32 = conn
        .query_row("SELECT COALESCE(MAX(guid), 0) FROM characters", [], |r| {
            r.get::<_, u32>(0)
        })?
        .max(handed_out(COUNTER_CHARACTER)?);
    let max_item_guid: u32 = conn
        .query_row(
            "SELECT COALESCE(MAX(guid), 0) FROM character_items",
            [],
            |r| r.get::<_, u32>(0),
        )?
        .max(handed_out(COUNTER_ITEM)?);
    let mut stmt = conn.prepare("SELECT kind, id, data FROM records")?;
    let records = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get::<_, i64>(1)? as u64, r.get(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let bones = read_bones(conn)?;
    let max_bones_id = bones.iter().map(|b| b.id).max().unwrap_or(0);
    Ok(Startup {
        names,
        max_character_guid,
        max_item_guid,
        records,
        bones,
        max_bones_id,
    })
}

fn read_bones(conn: &Connection) -> rusqlite::Result<Vec<BonesRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, map, x, y, z, orientation, owner, display, bytes_1, bytes_2, guild, flags, \
         dynamic_flags, expires_at FROM bones ORDER BY id",
    )?;
    stmt.query_map([], |r| {
        Ok(BonesRow {
            id: r.get(0)?,
            map: r.get(1)?,
            x: r.get(2)?,
            y: r.get(3)?,
            z: r.get(4)?,
            orientation: r.get(5)?,
            owner: r.get(6)?,
            display: r.get(7)?,
            bytes_1: r.get(8)?,
            bytes_2: r.get(9)?,
            guild: r.get(10)?,
            flags: r.get(11)?,
            dynamic_flags: r.get(12)?,
            expires_at: r.get(13)?,
        })
    })?
    .collect()
}

/// Bones whose time was up by `now` (Unix seconds) go: those gone while
/// the server was down, or in a map that never started.
fn sweep_bones(conn: &Connection, now: i64) -> rusqlite::Result<()> {
    let gone = conn.execute("DELETE FROM bones WHERE expires_at <= ?1", params![now])?;
    if gone > 0 {
        info!("the character store: {gone} bones gone while it was closed");
    }
    Ok(())
}

/// Characters and caches kept under an account's name in another case
/// (made before the name was put in capitals at the login,
/// `auth::account_name`) are the account's: its characters join its list,
/// and where both cases have a cache of one kind, the capitals' stays.
/// Nothing to do once done.
fn accounts_in_capitals(conn: &Connection) -> rusqlite::Result<()> {
    let moved = conn.execute(
        "UPDATE characters SET account = UPPER(account) WHERE account <> UPPER(account)",
        [],
    )?;
    conn.execute(
        "UPDATE OR IGNORE account_data SET account = UPPER(account) WHERE account <> UPPER(account)",
        [],
    )?;
    conn.execute(
        "DELETE FROM account_data WHERE account <> UPPER(account)",
        [],
    )?;
    if moved > 0 {
        info!("the character store: {moved} characters' account names put in capitals");
    }
    Ok(())
}

/// The `counters` rows.
const COUNTER_CHARACTER: &str = "character";
const COUNTER_ITEM: &str = "item";

/// The realm's next character and item GUIDs, kept by the store with each
/// write ([`Store::keep_counters`]): a GUID in a write was handed out
/// before it, so the counters written with it are past it.
struct Counters {
    character: Arc<AtomicU32>,
    item: Arc<AtomicU32>,
    /// What was last written of each.
    written: (u32, u32),
}

impl Counters {
    /// The counters now, if they moved since they were last written.
    fn moved(&self) -> Option<(u32, u32)> {
        let now = (
            self.character.load(Ordering::Relaxed),
            self.item.load(Ordering::Relaxed),
        );
        (now != self.written).then_some(now)
    }
}

fn write_counters(conn: &Connection, (character, item): (u32, u32)) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO counters (name, next) VALUES (?1, ?2) \
         ON CONFLICT (name) DO UPDATE SET next = MAX(next, excluded.next)",
    )?;
    stmt.execute(params![COUNTER_CHARACTER, character])?;
    stmt.execute(params![COUNTER_ITEM, item])?;
    Ok(())
}

/// Whether the store writes, for the realm: logins wait while it can't,
/// and the GMs are told (`social::store_watch`).
#[derive(Debug, Default)]
pub struct Health {
    failing: AtomicBool,
    /// What the GMs were last told.
    told: AtomicBool,
}

/// What the store's thread keeps between requests.
#[derive(Default)]
struct Kept {
    counters: Option<Counters>,
    /// Saves the store couldn't write, oldest first, written ahead of the
    /// next (`write_saves`).
    pending: Vec<Request>,
    /// Since when it couldn't write, and when that was last logged.
    failing_since: Option<std::time::Instant>,
    logged: Option<std::time::Instant>,
    health: Arc<Health>,
}

/// The store's thread. It outlives a panic in what it does: that request
/// is lost (its caller hears nothing), not every save after it.
fn store_thread(
    mut conn: Connection,
    mut rx: mpsc::UnboundedReceiver<Request>,
    queued: &AtomicUsize,
    health: Arc<Health>,
) {
    let mut kept = Kept {
        health,
        ..Default::default()
    };
    loop {
        match catch(|| serve_all(&mut conn, &mut kept, &mut rx, queued)) {
            Ok(()) => return,
            Err(e) => error!("the store panicked: {e}; going on"),
        }
    }
}

fn serve_all(
    conn: &mut Connection,
    kept: &mut Kept,
    rx: &mut mpsc::UnboundedReceiver<Request>,
    queued: &AtomicUsize,
) {
    loop {
        // While saves are kept that the store couldn't write, they are
        // tried again every so often, new saves or not.
        let first = if kept.pending.is_empty() {
            rx.blocking_recv()
        } else {
            match next_within(rx, RETRY_EVERY) {
                Next::Request(r) => Some(r),
                Next::TimeUp => {
                    write_saves(conn, kept, Vec::new());
                    continue;
                }
                Next::Closed => None,
            }
        };
        let Some(first) = first else {
            write_saves(conn, kept, Vec::new());
            if !kept.pending.is_empty() {
                error!(
                    "the character store couldn't write {} saves; they are lost",
                    kept.pending.len()
                );
            }
            return;
        };
        // Saves queued together go in one transaction.
        let mut batch = vec![first];
        while let Ok(more) = rx.try_recv() {
            batch.push(more);
            if batch.len() >= 512 {
                break;
            }
        }
        queued.fetch_sub(batch.len(), Ordering::Relaxed);
        let mut saves: Vec<Request> = Vec::new();
        for req in batch {
            if is_save(&req) {
                saves.push(req);
                continue;
            }
            // Keep order: what was saved before a read is written before
            // it.
            write_saves(conn, kept, std::mem::take(&mut saves));
            if let Err(e) = catch(|| serve(conn, kept, req)) {
                error!("a store request panicked: {e}; its caller hears nothing");
            }
        }
        write_saves(conn, kept, saves);
    }
}

fn is_save(req: &Request) -> bool {
    #[cfg(test)]
    if matches!(req, Request::BadSave) {
        return true;
    }
    matches!(
        req,
        Request::Save(_)
            | Request::SetAccountData(..)
            | Request::PutRecord(..)
            | Request::DeleteRecord(..)
            | Request::PutBones(_)
            | Request::DeleteBones(_)
            | Request::Together(..)
    )
}

/// What a save is of, for the log.
fn describe(req: &Request) -> String {
    match req {
        Request::Save(row) => format!("character {} ({})", row.name, row.guid),
        Request::SetAccountData(account, kind, ..) => format!("{account}'s account data {kind}"),
        Request::PutRecord(kind, id, _) => format!("{kind} {id}"),
        Request::DeleteRecord(kind, id) => format!("deleting {kind} {id}"),
        Request::PutBones(b) => format!("bones {} (of {} on map {})", b.id, b.owner, b.map),
        Request::DeleteBones(id) => format!("deleting bones {id}"),
        Request::Together(rows, records) => {
            let names: Vec<String> = rows
                .iter()
                .map(|r| format!("{} ({})", r.name, r.guid))
                .collect();
            let changes: Vec<String> = records
                .iter()
                .map(|r| match r {
                    RecordOp::Put(kind, id, _) => format!("{kind} {id}"),
                    RecordOp::Delete(kind, id) => format!("deleting {kind} {id}"),
                    #[cfg(test)]
                    RecordOp::Bad => "a bad change".to_owned(),
                })
                .collect();
            format!(
                "together: characters [{}], records [{}]",
                names.join(", "),
                changes.join(", ")
            )
        }
        _ => "a request".to_owned(),
    }
}

/// How many times a batch the database was too busy for is tried.
const BUSY_TRIES: u32 = 3;

/// Write saves in one transaction, behind those kept from before (the
/// store couldn't write them). If that fails because the store can't
/// write (a full disk, an I/O error, the file read-only, busy past its
/// tries), every save is kept, to be written ahead of the next ones. If it
/// fails otherwise (a bad row, a panic), each goes alone: one bad save is
/// lost, not those batched with it. A save together is one request, so it
/// goes alone whole: never in parts.
fn write_saves(conn: &mut Connection, kept: &mut Kept, saves: Vec<Request>) {
    if saves.is_empty() && kept.pending.is_empty() {
        return;
    }
    let batch = if kept.pending.is_empty() {
        saves
    } else {
        let mut batch = std::mem::take(&mut kept.pending);
        batch.extend(saves);
        batch
    };
    let counters = kept.counters.as_ref().and_then(Counters::moved);
    let written = |kept: &mut Kept| {
        if let (Some(c), Some(now)) = (&mut kept.counters, counters) {
            c.written = now;
        }
    };
    let mut tries = 0;
    let failed = loop {
        tries += 1;
        match catch(|| write_batch(conn, counters, &batch)) {
            Ok(Ok(())) => {
                written(kept);
                return kept.writes_again();
            }
            Ok(Err(e)) if busy(&e) && tries < BUSY_TRIES => {
                warn!(
                    "the character store is busy ({e}); trying {} saves again",
                    batch.len()
                );
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Ok(Err(e)) if cant_write(&e) => return kept.keep(batch, &e.to_string()),
            Ok(Err(e)) => break e.to_string(),
            Err(e) => break format!("it panicked: {e}"),
        }
    };
    if batch.len() == 1 {
        error!("lost a save ({}): {failed}", describe(&batch[0]));
        return kept.writes_again();
    }
    warn!(
        "saving {} records together failed ({failed}); saving each alone",
        batch.len()
    );
    let mut unwritten = Vec::new();
    let mut why = None;
    for s in batch {
        match catch(|| write_batch(conn, counters, std::slice::from_ref(&s))) {
            Ok(Ok(())) => written(kept),
            Ok(Err(e)) if cant_write(&e) => {
                why = Some(e.to_string());
                unwritten.push(s);
            }
            Ok(Err(e)) => error!("lost a save ({}): {e}", describe(&s)),
            Err(e) => error!("lost a save ({}): it panicked: {e}", describe(&s)),
        }
    }
    match why {
        Some(why) => kept.keep(unwritten, &why),
        None => kept.writes_again(),
    }
}

/// Whether a write failed because the store can't write now, not because
/// of what was written: worth keeping, to write when it can.
fn cant_write(e: &rusqlite::Error) -> bool {
    use rusqlite::ErrorCode::*;
    matches!(
        e.sqlite_error_code(),
        Some(
            DatabaseBusy
                | DatabaseLocked
                | DiskFull
                | SystemIoFailure
                | ReadOnly
                | CannotOpen
                | FileLockingProtocolFailed
                | OutOfMemory
                | DatabaseCorrupt
                | NotADatabase
        )
    )
}

/// What a save writes over, for a later one of the same to take the place
/// of one kept: a character's row, an account's data, a record, bones.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Character(u32),
    AccountData(String, u8),
    Record(&'static str, u64),
    Bones(u32),
}

fn key(req: &Request) -> Option<Key> {
    match req {
        Request::Save(row) => Some(Key::Character(row.guid)),
        Request::SetAccountData(account, kind, ..) => {
            Some(Key::AccountData(account.clone(), *kind))
        }
        Request::PutRecord(kind, id, _) | Request::DeleteRecord(kind, id) => {
            Some(Key::Record(kind, *id))
        }
        Request::PutBones(b) => Some(Key::Bones(b.id)),
        Request::DeleteBones(id) => Some(Key::Bones(*id)),
        _ => None,
    }
}

/// How often a store that can't write tries what it keeps again, with no
/// new save to write.
const RETRY_EVERY: std::time::Duration = std::time::Duration::from_secs(5);
/// How often a store that can't write says so in the log.
const LOG_EVERY: std::time::Duration = std::time::Duration::from_secs(60);
/// The most saves kept while the store can't write; the oldest go first.
const MAX_KEPT: usize = 100_000;

impl Kept {
    /// Saves the store couldn't write, kept in order to be written ahead of
    /// the next, each without the older saves of what it writes over.
    fn keep(&mut self, saves: Vec<Request>, why: &str) {
        self.pending.extend(saves);
        let mut seen = ahash::AHashSet::new();
        let mut newest_first: Vec<Request> = Vec::with_capacity(self.pending.len());
        for s in self.pending.drain(..).rev() {
            // A later save of the same key writes over it: it goes.
            if key(&s).is_none_or(|k| seen.insert(k)) {
                newest_first.push(s);
            }
        }
        newest_first.reverse();
        self.pending = newest_first;
        if self.pending.len() > MAX_KEPT {
            let over = self.pending.len() - MAX_KEPT;
            for s in self.pending.drain(..over) {
                error!(
                    "lost a save ({}): kept too long while the store couldn't write",
                    describe(&s)
                );
            }
        }
        let now = std::time::Instant::now();
        let first = self.failing_since.is_none();
        if first {
            self.failing_since = Some(now);
            self.health.failing.store(true, Ordering::Relaxed);
        }
        if first
            || self
                .logged
                .is_none_or(|t| now.duration_since(t) >= LOG_EVERY)
        {
            error!(
                "the character store can't write ({why}): {} saves kept to write when it can; \
                 logins wait meanwhile",
                self.pending.len()
            );
            self.logged = Some(now);
        }
    }

    /// The store wrote what it kept, if it kept anything.
    fn writes_again(&mut self) {
        if !self.pending.is_empty() {
            return;
        }
        if let Some(since) = self.failing_since.take() {
            self.logged = None;
            self.health.failing.store(false, Ordering::Relaxed);
            info!(
                "the character store writes again, after {:.0} s: what it kept is written",
                since.elapsed().as_secs_f32()
            );
        }
    }

    /// The latest of a character's rows kept, not yet written: what a read
    /// of them must see.
    fn kept_row(&self, guid: u32) -> Option<&CharacterRow> {
        self.pending.iter().rev().find_map(|s| match s {
            Request::Save(row) if row.guid == guid => Some(&**row),
            Request::Together(rows, _) => rows.iter().find(|r| r.guid == guid),
            _ => None,
        })
    }
}

/// What a wait for the next request came to.
enum Next {
    Request(Request),
    TimeUp,
    Closed,
}

/// The next request, or `TimeUp` after `limit`.
fn next_within(rx: &mut mpsc::UnboundedReceiver<Request>, limit: std::time::Duration) -> Next {
    let deadline = std::time::Instant::now() + limit;
    loop {
        match rx.try_recv() {
            Ok(r) => return Next::Request(r),
            Err(mpsc::error::TryRecvError::Disconnected) => return Next::Closed,
            Err(mpsc::error::TryRecvError::Empty) if std::time::Instant::now() >= deadline => {
                return Next::TimeUp;
            }
            Err(mpsc::error::TryRecvError::Empty) => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
        }
    }
}

fn busy(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}

/// Write `saves` in one transaction, with the counters if they moved.
fn write_batch(
    conn: &mut Connection,
    counters: Option<(u32, u32)>,
    saves: &[Request],
) -> rusqlite::Result<()> {
    let z = crate::zone!("store transaction");
    z.value(saves.len() as u64);
    {
        let tx = conn.transaction()?;
        if let Some(counters) = counters {
            write_counters(&tx, counters)?;
        }
        for s in saves {
            match s {
                Request::Save(row) => save_row(&tx, row)?,
                Request::SetAccountData(account, kind, time, data) => {
                    tx.execute(
                        "INSERT OR REPLACE INTO account_data (account, type, time, data) \
                         VALUES (?1, ?2, ?3, ?4)",
                        params![account, kind, time, data],
                    )?;
                }
                Request::PutRecord(kind, id, data) => put_record(&tx, kind, *id, data)?,
                Request::DeleteRecord(kind, id) => delete_record(&tx, kind, *id)?,
                Request::PutBones(b) => put_bones(&tx, b)?,
                Request::DeleteBones(id) => {
                    tx.prepare_cached("DELETE FROM bones WHERE id = ?1")?
                        .execute(params![id])?;
                }
                Request::Together(rows, records) => {
                    for row in rows {
                        save_row(&tx, row)?;
                    }
                    for r in records {
                        match r {
                            RecordOp::Put(kind, id, data) => put_record(&tx, kind, *id, data)?,
                            RecordOp::Delete(kind, id) => delete_record(&tx, kind, *id)?,
                            #[cfg(test)]
                            RecordOp::Bad => panic!("the store's test change panics"),
                        }
                    }
                }
                #[cfg(test)]
                Request::BadSave => panic!("the store's test save panics"),
                _ => {}
            }
        }
        tx.commit()
    }
}

fn put_record(conn: &Connection, kind: &str, id: u64, data: &str) -> rusqlite::Result<()> {
    conn.prepare_cached("INSERT OR REPLACE INTO records (kind, id, data) VALUES (?1, ?2, ?3)")?
        .execute(params![kind, id as i64, data])?;
    Ok(())
}

fn put_bones(conn: &Connection, b: &BonesRow) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "INSERT OR REPLACE INTO bones (id, map, x, y, z, orientation, owner, display, bytes_1, \
         bytes_2, guild, flags, dynamic_flags, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, \
         ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
    )?
    .execute(params![
        b.id,
        b.map,
        b.x,
        b.y,
        b.z,
        b.orientation,
        b.owner,
        b.display,
        b.bytes_1,
        b.bytes_2,
        b.guild,
        b.flags,
        b.dynamic_flags,
        b.expires_at,
    ])?;
    Ok(())
}

fn delete_record(conn: &Connection, kind: &str, id: u64) -> rusqlite::Result<()> {
    conn.prepare_cached("DELETE FROM records WHERE kind = ?1 AND id = ?2")?
        .execute(params![kind, id as i64])?;
    Ok(())
}

fn serve(conn: &Connection, kept: &mut Kept, req: Request) {
    let _z = crate::zone!("store request");
    match req {
        Request::KeepCounters(character, item) => {
            kept.counters = Some(Counters {
                character,
                item,
                written: (0, 0),
            });
        }
        Request::Characters(account, reply) => {
            let mut rows = characters_for(conn, &account).unwrap_or_else(|e| {
                warn!("listing characters for {account}: {e}");
                Vec::new()
            });
            // As saved, though not yet written.
            for row in &mut rows {
                if let Some(kept) = kept.kept_row(row.guid) {
                    *row = kept.clone();
                }
            }
            let _ = reply.send(rows);
        }
        Request::Load(guid, reply) => {
            let row = match kept.kept_row(guid) {
                Some(kept) => Some(kept.clone()),
                None => load_row(conn, guid).unwrap_or_else(|e| {
                    warn!("loading character {guid}: {e}");
                    None
                }),
            };
            let _ = reply.send(row);
        }
        Request::Create(row, reply) => {
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM characters WHERE name = ?1",
                    params![row.name],
                    |_| Ok(true),
                )
                .optional()
                .ok()
                .flatten()
                .unwrap_or(false);
            let result = if exists {
                Err(CreateError::NameInUse)
            } else {
                // With the counters past its GUID and its items'.
                let counters = kept.counters.as_ref().and_then(Counters::moved);
                (|| {
                    let tx = conn.unchecked_transaction()?;
                    save_row(&tx, &row)?;
                    if let Some(c) = counters {
                        write_counters(&tx, c)?;
                    }
                    tx.commit()
                })()
                .map(|()| {
                    if let (Some(c), Some(now)) = (&mut kept.counters, counters) {
                        c.written = now;
                    }
                })
                .map_err(|e| {
                    warn!("creating character {}: {e}", row.name);
                    CreateError::Failed
                })
            };
            let _ = reply.send(result);
        }
        Request::Delete(account, guid, reply) => {
            let deleted = (|| -> rusqlite::Result<bool> {
                let n = conn.execute(
                    "DELETE FROM characters WHERE guid = ?1 AND account = ?2",
                    params![guid, account],
                )?;
                if n > 0 {
                    conn.execute(
                        "DELETE FROM character_items WHERE owner = ?1",
                        params![guid],
                    )?;
                    conn.execute(
                        "DELETE FROM character_spells WHERE owner = ?1",
                        params![guid],
                    )?;
                    conn.execute(
                        "DELETE FROM character_actions WHERE owner = ?1",
                        params![guid],
                    )?;
                }
                Ok(n > 0)
            })()
            .unwrap_or(false);
            let _ = reply.send(deleted);
        }
        Request::AccountData(account, reply) => {
            let rows = (|| -> rusqlite::Result<Vec<(u8, u32, Vec<u8>)>> {
                let mut stmt =
                    conn.prepare("SELECT type, time, data FROM account_data WHERE account = ?1")?;
                stmt.query_map(params![account], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect()
            })()
            .unwrap_or_default();
            let _ = reply.send(rows);
        }
        Request::Flush(reply) => {
            if !kept.pending.is_empty() {
                error!(
                    "the character store still can't write {} saves: they are lost if the server \
                     stops now",
                    kept.pending.len()
                );
            }
            let _ = reply.send(());
        }
        #[cfg(test)]
        Request::ReadOnly(on) => {
            if let Err(e) = conn.pragma_update(None, "query_only", on) {
                error!("the store's test read-only: {e}");
            }
        }
        #[cfg(test)]
        Request::Record(kind, id, reply) => {
            let data = conn
                .query_row(
                    "SELECT data FROM records WHERE kind = ?1 AND id = ?2",
                    params![kind, id as i64],
                    |r| r.get(0),
                )
                .optional()
                .ok()
                .flatten();
            let _ = reply.send(data);
        }
        #[cfg(test)]
        Request::Bones(reply) => {
            let _ = reply.send(read_bones(conn).unwrap_or_default());
        }
        #[cfg(test)]
        Request::Panic => panic!("the store's test panic"),
        #[cfg(test)]
        Request::Sleep(d) => std::thread::sleep(d),
        #[cfg(test)]
        Request::BadSave => {}
        Request::Save(_)
        | Request::SetAccountData(..)
        | Request::PutRecord(..)
        | Request::DeleteRecord(..)
        | Request::PutBones(_)
        | Request::DeleteBones(_)
        | Request::Together(..) => {}
    }
}

const CHARACTER_COLUMNS: &str = "guid, account, name, race, class, gender, skin, face, \
     hair_style, hair_color, facial_hair, level, xp, money, map, x, y, z, orientation, zone, \
     played_total, played_level, logout_time, at_login, state";

/// A character's state from its JSON. A field that won't parse goes back
/// to its default, and only it: the rest are kept. (A float that was NaN
/// is saved as `null`, which an `f32` won't take; before, it cost the
/// whole state: quests, reputation, pets, the hearthstone's place.)
fn parse_state(json: &str) -> CharacterState {
    let e = match serde_json::from_str(json) {
        Ok(state) => return state,
        Err(e) => e,
    };
    let Ok(serde_json::Value::Object(mut fields)) = serde_json::from_str(json) else {
        warn!("a character's state doesn't parse ({e}); using defaults");
        return CharacterState::default();
    };
    let names: Vec<String> = fields.keys().cloned().collect();
    for name in names {
        let alone = serde_json::Value::Object(
            std::iter::once((name.clone(), fields[&name].clone())).collect(),
        );
        if let Err(e) = serde_json::from_value::<CharacterState>(alone) {
            warn!("a character's {name} doesn't parse ({e}); it goes back to its default");
            fields.remove(&name);
        }
    }
    serde_json::from_value(serde_json::Value::Object(fields)).unwrap_or_default()
}

/// A record from its JSON (a mail, an auction, a guild, an honor entry),
/// as [`parse_state`] reads a character's state: a field that won't parse
/// (a NaN, saved as `null`) goes back to its default, and only it; the
/// rest are kept. `None`, logged at `error`, for a record that isn't an
/// object at all: it is left in the store as it is, and its id isn't
/// handed out again (the loaders count every record's id, read or not).
pub fn parse_record<T: DeserializeOwned + Serialize + Default>(
    kind: &str,
    id: u64,
    json: &str,
) -> Option<T> {
    use serde_json::Value;
    let e = match serde_json::from_str(json) {
        Ok(record) => return Some(record),
        Err(e) => e,
    };
    let (Ok(Value::Object(fields)), Ok(Value::Object(base))) = (
        serde_json::from_str::<Value>(json),
        serde_json::to_value(T::default()),
    ) else {
        error!("{kind} {id} doesn't parse ({e}); left as it is in the store");
        return None;
    };
    let mut kept = base.clone();
    for (name, value) in fields {
        let mut alone = base.clone();
        alone.insert(name.clone(), value.clone());
        match serde_json::from_value::<T>(Value::Object(alone)) {
            Ok(_) => {
                kept.insert(name, value);
            }
            Err(e) => {
                error!("{kind} {id}: its {name} doesn't parse ({e}); it goes back to its default")
            }
        }
    }
    serde_json::from_value(Value::Object(kept))
        .inspect_err(|e| error!("{kind} {id} doesn't parse ({e}); left as it is in the store"))
        .ok()
}

fn row_from(r: &rusqlite::Row) -> rusqlite::Result<CharacterRow> {
    let state: String = r.get(24)?;
    Ok(CharacterRow {
        guid: r.get(0)?,
        account: r.get(1)?,
        name: r.get(2)?,
        race: r.get(3)?,
        class: r.get(4)?,
        gender: r.get(5)?,
        skin: r.get(6)?,
        face: r.get(7)?,
        hair_style: r.get(8)?,
        hair_color: r.get(9)?,
        facial_hair: r.get(10)?,
        level: r.get(11)?,
        xp: r.get(12)?,
        money: r.get(13)?,
        map: r.get(14)?,
        x: r.get(15)?,
        y: r.get(16)?,
        z: r.get(17)?,
        orientation: r.get(18)?,
        zone: r.get(19)?,
        played_total: r.get(20)?,
        played_level: r.get(21)?,
        logout_time: r.get(22)?,
        at_login: r.get(23)?,
        state: parse_state(&state),
        items: Vec::new(),
        spells: Vec::new(),
        actions: Vec::new(),
    })
}

fn fill_children(conn: &Connection, row: &mut CharacterRow) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare_cached(
        "SELECT guid, entry, bag, slot, count, durability, flags, data, charges \
         FROM character_items WHERE owner = ?1",
    )?;
    row.items = stmt
        .query_map(params![row.guid], |r| {
            let charges: Option<String> = r.get(8)?;
            Ok(ItemRow {
                guid: r.get(0)?,
                entry: r.get(1)?,
                bag: r.get(2)?,
                slot: r.get(3)?,
                count: r.get(4)?,
                durability: r.get(5)?,
                flags: r.get(6)?,
                data: r.get(7)?,
                charges: ItemRow::parse_charges(charges.as_deref()),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut stmt = conn.prepare_cached("SELECT spell FROM character_spells WHERE owner = ?1")?;
    row.spells = stmt
        .query_map(params![row.guid], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut stmt =
        conn.prepare_cached("SELECT button, action, type FROM character_actions WHERE owner = ?1")?;
    row.actions = stmt
        .query_map(params![row.guid], |r| {
            Ok(ActionRow {
                button: r.get(0)?,
                action: r.get(1)?,
                kind: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(())
}

fn characters_for(conn: &Connection, account: &str) -> rusqlite::Result<Vec<CharacterRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {CHARACTER_COLUMNS} FROM characters WHERE account = ?1 ORDER BY guid"
    ))?;
    let mut rows = stmt
        .query_map(params![account], row_from)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for row in &mut rows {
        fill_children(conn, row)?;
    }
    Ok(rows)
}

fn load_row(conn: &Connection, guid: u32) -> rusqlite::Result<Option<CharacterRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {CHARACTER_COLUMNS} FROM characters WHERE guid = ?1"
    ))?;
    let Some(mut row) = stmt.query_row(params![guid], row_from).optional()? else {
        return Ok(None);
    };
    fill_children(conn, &mut row)?;
    Ok(Some(row))
}

fn save_row(conn: &Connection, row: &CharacterRow) -> rusqlite::Result<()> {
    let state = serde_json::to_string(&row.state).unwrap_or_else(|_| "{}".to_owned());
    conn.prepare_cached(&format!(
        "INSERT OR REPLACE INTO characters ({CHARACTER_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, \
         ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, \
         ?25)"
    ))?
    .execute(params![
        row.guid,
        row.account,
        row.name,
        row.race,
        row.class,
        row.gender,
        row.skin,
        row.face,
        row.hair_style,
        row.hair_color,
        row.facial_hair,
        row.level,
        row.xp,
        row.money,
        row.map,
        row.x,
        row.y,
        row.z,
        row.orientation,
        row.zone,
        row.played_total,
        row.played_level,
        row.logout_time,
        row.at_login,
        state,
    ])?;
    conn.prepare_cached("DELETE FROM character_items WHERE owner = ?1")?
        .execute(params![row.guid])?;
    let mut insert = conn.prepare_cached(
        "INSERT OR REPLACE INTO character_items (guid, owner, entry, bag, slot, count, \
         durability, flags, data, charges) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?;
    for i in &row.items {
        insert.execute(params![
            i.guid,
            row.guid,
            i.entry,
            i.bag,
            i.slot,
            i.count,
            i.durability,
            i.flags,
            i.data,
            i.charges_text(),
        ])?;
    }
    conn.prepare_cached("DELETE FROM character_spells WHERE owner = ?1")?
        .execute(params![row.guid])?;
    let mut insert =
        // A spell twice, or a button twice, would fail the save (and the
        // others in its transaction): the second is the same spell, or
        // the button's latest.
        conn.prepare_cached("INSERT OR IGNORE INTO character_spells (owner, spell) VALUES (?1, ?2)")?;
    for s in &row.spells {
        insert.execute(params![row.guid, s])?;
    }
    conn.prepare_cached("DELETE FROM character_actions WHERE owner = ?1")?
        .execute(params![row.guid])?;
    let mut insert = conn.prepare_cached(
        "INSERT OR REPLACE INTO character_actions (owner, button, action, type) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for a in &row.actions {
        insert.execute(params![row.guid, a.button, a.action, a.kind])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test's store file gone, with the copies made before migrating it
    /// and SQLite's own files beside it.
    fn remove_store(path: &Path) {
        for suffix in ["", "-wal", "-shm", ".v2.bak", ".v3.bak", ".v4.bak"] {
            let mut name = path.as_os_str().to_owned();
            name.push(suffix);
            let _ = std::fs::remove_file(std::path::PathBuf::from(name));
        }
    }

    fn row(guid: u32, name: &str) -> CharacterRow {
        CharacterRow {
            guid,
            account: "ACC".into(),
            name: name.into(),
            race: 1,
            class: 1,
            level: 3,
            x: 1.5,
            state: CharacterState {
                health: 77,
                ..Default::default()
            },
            items: vec![ItemRow {
                guid: guid * 10,
                entry: 25,
                bag: BAG_PLAYER,
                slot: 15,
                count: 1,
                ..Default::default()
            }],
            spells: vec![78, 2457],
            actions: vec![ActionRow {
                button: 0,
                action: 6603,
                kind: 0,
            }],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_character_survives_a_save_and_a_reopen() {
        let dir = std::env::temp_dir().join(format!("wvs-store-{}", std::process::id()));
        let _ = std::fs::remove_file(&dir);
        {
            let (store, startup) = Store::open(&dir).unwrap();
            assert!(startup.names.is_empty());
            store.create(row(1, "Alice")).await.unwrap();
            assert_eq!(
                store.create(row(2, "alice")).await,
                Err(CreateError::NameInUse)
            );
            let mut r = row(1, "Alice");
            r.level = 9;
            store.save(r);
            store.flush().await;
        }
        let (store, startup) = Store::open(&dir).unwrap();
        assert_eq!(startup.names.len(), 1);
        assert_eq!(startup.max_character_guid, 1);
        assert_eq!(startup.max_item_guid, 10);
        let loaded = store.load(1).await.unwrap();
        assert_eq!(loaded.level, 9);
        assert_eq!(loaded.state.health, 77);
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.spells, vec![78, 2457]);
        assert_eq!(store.characters("ACC").await.len(), 1);
        assert!(store.delete("ACC", 1).await);
        assert!(store.load(1).await.is_none());
        drop(store);
        let _ = std::fs::remove_file(&dir);
    }

    /// A store of version 2 (no charges) migrates: its items load with no
    /// charges stored, and a save after keeps them.
    #[tokio::test]
    async fn a_store_before_charges_migrates_and_keeps_them_after() {
        let path = std::env::temp_dir().join(format!("wvs-store-v2-{}", std::process::id()));
        remove_store(&path);
        {
            let conn = Connection::open(&path).unwrap();
            for (i, sql) in MIGRATIONS[..2].iter().enumerate() {
                conn.execute_batch(&format!(
                    "BEGIN; {sql}; PRAGMA user_version = {}; COMMIT;",
                    i + 1
                ))
                .unwrap();
            }
            conn.execute_batch(
                "INSERT INTO characters (guid, account, name, race, class, gender, skin, face, \
                 hair_style, hair_color, facial_hair, level, xp, money, map, x, y, z, \
                 orientation, zone, played_total, played_level, logout_time, at_login, state) \
                 VALUES (1, 'ACC', 'Alice', 1, 1, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, \
                 0, 0, 0, 0, '{}'); \
                 INSERT INTO character_items (guid, owner, entry, bag, slot, count, durability, \
                 flags, data) VALUES (10, 1, 25, 255, 15, 1, 0, 0, '');",
            )
            .unwrap();
        }
        let (store, _) = Store::open(&path).unwrap();
        let loaded = store.load(1).await.expect("Alice");
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].charges, None, "none stored before");
        let mut r = loaded;
        r.items[0].charges = Some([1, -2, 0, 0, 0]);
        store.save(r);
        store.flush().await;
        drop(store);
        let (store, _) = Store::open(&path).unwrap();
        let loaded = store.load(1).await.expect("Alice");
        assert_eq!(loaded.items[0].charges, Some([1, -2, 0, 0, 0]));
        drop(store);
        remove_store(&path);
    }

    fn bones(id: u32, expires_at: i64) -> BonesRow {
        BonesRow {
            id,
            map: 1,
            x: -618.5,
            y: -4251.25,
            z: 38.75,
            orientation: 1.5,
            owner: 7,
            display: 1563,
            bytes_1: 0x0001_0200,
            bytes_2: 0x0304_0506,
            guild: 3,
            flags: 0x05,
            dynamic_flags: 0x01,
            expires_at,
        }
    }

    /// A store of version 3 (no bones) migrates, its characters kept; the
    /// bones kept in it come back as they went in, those whose time is up
    /// swept as it opens, from the table too.
    #[tokio::test]
    async fn a_store_before_bones_migrates_and_sweeps_the_gone_as_it_opens() {
        let path = std::env::temp_dir().join(format!("wvs-store-v3-{}", std::process::id()));
        remove_store(&path);
        {
            let conn = Connection::open(&path).unwrap();
            for (i, sql) in MIGRATIONS[..3].iter().enumerate() {
                conn.execute_batch(&format!(
                    "BEGIN; {sql}; PRAGMA user_version = {}; COMMIT;",
                    i + 1
                ))
                .unwrap();
            }
            conn.execute_batch(
                "INSERT INTO characters (guid, account, name, race, class, gender, skin, face, \
                 hair_style, hair_color, facial_hair, level, xp, money, map, x, y, z, \
                 orientation, zone, played_total, played_level, logout_time, at_login, state) \
                 VALUES (1, 'ACC', 'Alice', 1, 1, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, \
                 0, 0, 0, 0, '{}');",
            )
            .unwrap();
        }
        let now = crate::game::social::mail::now();
        let (store, startup) = Store::open(&path).unwrap();
        assert_eq!(startup.names.len(), 1, "her character kept");
        assert!(startup.bones.is_empty());
        assert_eq!(startup.max_bones_id, 0);
        // Copied as it was before it was migrated (review 5 #21).
        {
            let mut bak = path.as_os_str().to_owned();
            bak.push(".v3.bak");
            let bak = Connection::open(std::path::PathBuf::from(bak)).unwrap();
            let version: u32 = bak
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
            let alice: String = bak
                .query_row("SELECT name FROM characters", [], |r| r.get(0))
                .unwrap();
            assert_eq!((version, alice.as_str()), (3, "Alice"));
        }
        store.put_bones(bones(5, now + 3600));
        store.put_bones(bones(9, now - 1));
        store.put_bones(bones(6, now + 60));
        store.delete_bones(6);
        store.flush().await;
        drop(store);
        let (store, startup) = Store::open(&path).unwrap();
        assert_eq!(startup.bones, vec![bones(5, now + 3600)], "as kept");
        assert_eq!(startup.max_bones_id, 5);
        assert_eq!(store.bones_for_test().await.len(), 1, "the gone swept");
        drop(store);
        let conn = Connection::open(&path).unwrap();
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        drop(conn);
        remove_store(&path);
    }

    #[test]
    fn charges_are_kept_as_mangos_keeps_them() {
        let row = ItemRow {
            charges: Some([1, -2, 0, 0, 0]),
            ..Default::default()
        };
        assert_eq!(row.charges_text().as_deref(), Some("1 -2 0 0 0"));
        assert_eq!(
            ItemRow::parse_charges(Some("1 -2 0 0 0 ")),
            Some([1, -2, 0, 0, 0])
        );
        assert_eq!(ItemRow::parse_charges(Some("1 2")), None, "not five");
        assert_eq!(ItemRow::parse_charges(Some("a b c d e")), None);
        assert_eq!(ItemRow::parse_charges(None), None);
    }

    /// A save together lands whole or not at all: with one bad part, none
    /// of its characters or records is written, while the saves batched
    /// with it still are; and it isn't split when another in its batch
    /// fails.
    #[tokio::test]
    async fn a_save_together_lands_whole_or_not_at_all() {
        let (store, _) = Store::open(":memory:").unwrap();
        store.create(row(1, "Alice")).await.unwrap();
        store.create(row(2, "Bob")).await.unwrap();
        let level = |r: Option<CharacterRow>| r.map(|r| r.level);

        // A bad part: neither character, nor the record, is written.
        store.hold_for_test(std::time::Duration::from_millis(100));
        let mut alone = row(2, "Bob");
        alone.level = 5;
        store.save(alone);
        let (mut a, mut b) = (row(1, "Alice"), row(2, "Bob"));
        a.level = 20;
        b.level = 30;
        let mut records = Records::new();
        records.put("mail", 7, &"a letter");
        records.bad_for_test();
        store.save_together(vec![a, b], records);
        store.put_record("mail", 8, &"another");
        assert_eq!(level(store.load(1).await), Some(3));
        assert_eq!(level(store.load(2).await), Some(5), "the save beside it");
        assert_eq!(store.record("mail", 7).await, None);
        assert_eq!(
            store.record("mail", 8).await.as_deref(),
            Some("\"another\"")
        );

        // Whole: both characters and the record, and a bad save batched
        // with it costs it nothing.
        store.hold_for_test(std::time::Duration::from_millis(100));
        store.bad_save_for_test();
        let (mut a, mut b) = (row(1, "Alice"), row(2, "Bob"));
        a.level = 21;
        b.level = 31;
        let mut records = Records::new();
        records.put("mail", 7, &"a letter");
        records.delete("mail", 8);
        store.save_together(vec![a, b], records);
        assert_eq!(level(store.load(1).await), Some(21));
        assert_eq!(level(store.load(2).await), Some(31));
        assert_eq!(
            store.record("mail", 7).await.as_deref(),
            Some("\"a letter\"")
        );
        assert_eq!(store.record("mail", 8).await, None);
    }
}
