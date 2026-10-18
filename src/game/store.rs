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
use std::sync::mpsc as std_mpsc;
use std::thread;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
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
    /// Characters and records an item or money moved between: written in
    /// one transaction, never apart.
    Together(Vec<CharacterRow>, Vec<RecordOp>),
    Flush(oneshot::Sender<()>),
    /// A record as stored, for the tests to read back.
    #[cfg(test)]
    Record(&'static str, u64, oneshot::Sender<Option<String>>),
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
    tx: std_mpsc::Sender<Request>,
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
        let conn = Connection::open(path.as_ref())?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&conn)?;
        let startup = read_startup(&conn)?;
        info!(
            "character store {}: {} characters, schema version {SCHEMA_VERSION}",
            path.as_ref().display(),
            startup.names.len()
        );
        let (tx, rx) = std_mpsc::channel();
        thread::Builder::new()
            .name("store".into())
            .spawn(move || run(conn, rx))
            .expect("spawn the store thread");
        Ok((Store { tx }, startup))
    }

    pub async fn characters(&self, account: &str) -> Vec<CharacterRow> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(Request::Characters(account.to_owned(), tx));
        rx.await.unwrap_or_default()
    }

    pub async fn load(&self, guid: u32) -> Option<CharacterRow> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(Request::Load(guid, tx));
        rx.await.ok().flatten()
    }

    pub async fn create(&self, row: CharacterRow) -> Result<(), CreateError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(Request::Create(Box::new(row), tx));
        rx.await.unwrap_or(Err(CreateError::Failed))
    }

    pub async fn delete(&self, account: &str, guid: u32) -> bool {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(Request::Delete(account.to_owned(), guid, tx));
        rx.await.unwrap_or(false)
    }

    /// Save a character, items, spells and actions included, replacing what
    /// was stored.
    pub fn save(&self, row: CharacterRow) {
        let _ = self.tx.send(Request::Save(Box::new(row)));
    }

    /// The account's data blocks: `(type, time, data)`.
    pub async fn account_data(&self, account: &str) -> Vec<(u8, u32, Vec<u8>)> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(Request::AccountData(account.to_owned(), tx));
        rx.await.unwrap_or_default()
    }

    pub fn set_account_data(&self, account: &str, kind: u8, time: u32, data: Vec<u8>) {
        let _ = self.tx.send(Request::SetAccountData(
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
                let _ = self.tx.send(Request::PutRecord(kind, id, data));
            }
            Err(e) => warn!("encoding {kind} {id}: {e}"),
        }
    }

    pub fn delete_record(&self, kind: &'static str, id: u64) {
        let _ = self.tx.send(Request::DeleteRecord(kind, id));
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
        let _ = self.tx.send(Request::Together(rows, records.0));
    }

    /// A record as stored (once what was sent before is written).
    #[cfg(test)]
    pub async fn record(&self, kind: &'static str, id: u64) -> Option<String> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(Request::Record(kind, id, tx));
        rx.await.ok().flatten()
    }

    /// Wait until everything sent before is written.
    pub async fn flush(&self) {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(Request::Flush(tx));
        let _ = rx.await;
    }

    #[cfg(test)]
    fn send_test(&self, req: Request) {
        let _ = self.tx.send(req);
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
    Ok(Startup {
        names,
        max_character_guid,
        max_item_guid,
        records,
    })
}

/// The store's thread. It outlives a panic in what it does: that request
/// is lost (its caller hears nothing), not every save after it.
fn run(mut conn: Connection, rx: std_mpsc::Receiver<Request>) {
    loop {
        match catch(|| serve_all(&mut conn, &rx)) {
            Ok(()) => return,
            Err(e) => error!("the store panicked: {e}; going on"),
        }
    }
}

fn serve_all(conn: &mut Connection, rx: &std_mpsc::Receiver<Request>) {
    while let Ok(first) = rx.recv() {
        // Saves queued together go in one transaction.
        let mut batch = vec![first];
        while let Ok(more) = rx.try_recv() {
            batch.push(more);
            if batch.len() >= 512 {
                break;
            }
        }
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

fn delete_record(conn: &Connection, kind: &str, id: u64) -> rusqlite::Result<()> {
    conn.prepare_cached("DELETE FROM records WHERE kind = ?1 AND id = ?2")?
        .execute(params![kind, id as i64])?;
    Ok(())
}

fn serve(conn: &Connection, req: Request) {
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
        Request::Panic => panic!("the store's test panic"),
        #[cfg(test)]
        Request::Sleep(d) => std::thread::sleep(d),
        #[cfg(test)]
        Request::BadSave => {}
        Request::Save(_)
        | Request::SetAccountData(..)
        | Request::PutRecord(..)
        | Request::DeleteRecord(..)
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
        "SELECT guid, entry, bag, slot, count, durability, flags, data FROM character_items \
         WHERE owner = ?1",
    )?;
    row.items = stmt
        .query_map(params![row.guid], |r| {
            Ok(ItemRow {
                guid: r.get(0)?,
                entry: r.get(1)?,
                bag: r.get(2)?,
                slot: r.get(3)?,
                count: r.get(4)?,
                durability: r.get(5)?,
                flags: r.get(6)?,
                data: r.get(7)?,
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
         durability, flags, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
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
            i.data
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
