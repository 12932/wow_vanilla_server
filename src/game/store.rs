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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use rusqlite::{Connection, OptionalExtension, params};
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
/// highest GUIDs in use.
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
        {
            let queued = queued.clone();
            thread::Builder::new()
                .name("store".into())
                .spawn(move || run(conn, rx, &queued))
                .expect("spawn the store thread");
        }
        Ok((Store { tx, queued }, startup))
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
    let max_character_guid: u32 =
        conn.query_row("SELECT COALESCE(MAX(guid), 0) FROM characters", [], |r| {
            r.get(0)
        })?;
    let max_item_guid: u32 = conn.query_row(
        "SELECT COALESCE(MAX(guid), 0) FROM character_items",
        [],
        |r| r.get(0),
    )?;
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

/// The store's thread. It outlives a panic in what it does: that request
/// is lost (its caller hears nothing), not every save after it.
fn run(mut conn: Connection, mut rx: mpsc::UnboundedReceiver<Request>, queued: &AtomicUsize) {
    loop {
        match catch(|| serve_all(&mut conn, &mut rx, queued)) {
            Ok(()) => return,
            Err(e) => error!("the store panicked: {e}; going on"),
        }
    }
}

fn serve_all(
    conn: &mut Connection,
    rx: &mut mpsc::UnboundedReceiver<Request>,
    queued: &AtomicUsize,
) {
    while let Some(first) = rx.blocking_recv() {
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
            write_saves(conn, std::mem::take(&mut saves));
            if let Err(e) = catch(|| serve(conn, req)) {
                error!("a store request panicked: {e}; its caller hears nothing");
            }
        }
        write_saves(conn, saves);
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

/// Write saves in one transaction. If that fails (the database busy past
/// its timeout, a bad row, a panic), the busy are tried again, and the
/// rest each go alone: one bad save is lost, not those batched with it. A
/// save together is one request, so it goes alone whole: never in parts.
fn write_saves(conn: &mut Connection, saves: Vec<Request>) {
    if saves.is_empty() {
        return;
    }
    let mut tries = 0;
    let failed = loop {
        tries += 1;
        match catch(|| write_batch(conn, &saves)) {
            Ok(Ok(())) => return,
            Ok(Err(e)) if busy(&e) && tries < BUSY_TRIES => {
                warn!(
                    "the character store is busy ({e}); trying {} saves again",
                    saves.len()
                );
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Ok(Err(e)) => break e.to_string(),
            Err(e) => break format!("it panicked: {e}"),
        }
    };
    if saves.len() == 1 {
        error!("lost a save ({}): {failed}", describe(&saves[0]));
        return;
    }
    warn!(
        "saving {} records together failed ({failed}); saving each alone",
        saves.len()
    );
    for s in &saves {
        match catch(|| write_batch(conn, std::slice::from_ref(s))) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => error!("lost a save ({}): {e}", describe(s)),
            Err(e) => error!("lost a save ({}): it panicked: {e}", describe(s)),
        }
    }
}

fn busy(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}

fn write_batch(conn: &mut Connection, saves: &[Request]) -> rusqlite::Result<()> {
    let z = crate::zone!("store transaction");
    z.value(saves.len() as u64);
    {
        let tx = conn.transaction()?;
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

fn serve(conn: &Connection, req: Request) {
    let _z = crate::zone!("store request");
    match req {
        Request::Characters(account, reply) => {
            let rows = characters_for(conn, &account).unwrap_or_else(|e| {
                warn!("listing characters for {account}: {e}");
                Vec::new()
            });
            let _ = reply.send(rows);
        }
        Request::Load(guid, reply) => {
            let row = load_row(conn, guid).unwrap_or_else(|e| {
                warn!("loading character {guid}: {e}");
                None
            });
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
                save_row(conn, &row).map_err(|e| {
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
            let _ = reply.send(());
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
        let _ = std::fs::remove_file(&path);
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
        let _ = std::fs::remove_file(&path);
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
        let _ = std::fs::remove_file(&path);
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
        let _ = std::fs::remove_file(&path);
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
