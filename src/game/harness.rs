//! A test harness: a realm with an in-memory store, and clients that speak
//! to it over channels, through the real session and maps. What a client
//! gets back is parsed with wow_messages, which checks the encoding too.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::{ClientOpcodeMessage, ServerOpcodeMessage};
use wow_world_messages::vanilla::*;

use crate::game::conn::{Connection, Outbound};
use crate::game::data::GameData;
use crate::game::store::Store;
use crate::game::{Realm, RealmConfig, session};

/// How long a test waits for what it expects. Generous, as only a
/// failing test waits it out, and a loaded machine (several suites at
/// once) can take seconds over a character creation.
pub const WAIT: Duration = Duration::from_secs(20);

/// How many of a map's updates a test waits for what the map does at once
/// to reach a client ([`Client::sees_on`]): 20 s of a quiet map's time,
/// for a busy machine's updates that come slowly.
pub const UPDATES: u64 = 400;

/// The real world's data for the tests that read it, laid out as the server
/// looks for it under `data/`: `db` (`vmangos.sqlite` and `dbc.sqlite`),
/// `nav` (a rustigator bake) and `terrain` (Thistle Tea's tiles). From
/// `WVS_DATA` if it is set, else `data/` in the checkout. A test whose data
/// isn't there passes without running.
pub fn real_data(what: &str) -> std::path::PathBuf {
    extended_only(what);
    std::env::var_os("WVS_DATA")
        .map_or_else(
            || std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data"),
            std::path::PathBuf::from,
        )
        .join(what)
}

/// The real world is for the extended suite only: under nextest, any other
/// profile fails the test that asks for it, so that a test needing the real
/// world is never quietly in the fast suite (or quietly passing there
/// without running). See docs/TESTING.md.
fn extended_only(what: &str) {
    if let Ok(profile) = std::env::var("NEXTEST_PROFILE")
        && profile != "extended"
    {
        panic!(
            "this test reads the real world (data/{what}), which is for the extended suite only \
             (nextest profile `{profile}`): mark it `#[ignore = \"extended\"]` and run it with \
             `cargo extended` (`cargo nextest run --lib --profile extended --run-ignored all`); \
             see docs/TESTING.md"
        );
    }
}

/// [`load::load`](crate::game::data::load::load) of the real world in `dir`
/// (`real_data("db")`), for the tests: from a snapshot of an earlier test's
/// load, when one was made from these databases by this test binary. Each
/// test is a process of its own, and the snapshot reads in a fraction of
/// the load's time.
///
/// The snapshot's name is a hash of the databases' paths, sizes and
/// modification times and the test binary's, so a change to either (a
/// rebuild, for one to the loader) is a new snapshot, made by the first
/// test to want it while the others wait on its lock. It is kept beside the
/// test binary, in `target/`, and older ones are removed.
pub fn load_real(dir: &std::path::Path) -> GameData {
    use crate::game::data::load::load;
    let Some((at, name)) = snapshot_path(dir) else {
        return load(dir);
    };
    let path = at.join(&name);
    let read = || {
        let bytes = std::fs::read(&path).ok()?;
        postcard::from_bytes::<GameData>(&bytes).ok()
    };
    if let Some(data) = read() {
        return data;
    }
    // One test loads and writes it; any others wanting it meanwhile wait.
    let Ok(lock) = std::fs::File::create(at.join("wvs-real-world.lock")) else {
        return load(dir);
    };
    if lock.lock().is_err() {
        return load(dir);
    }
    if let Some(data) = read() {
        return data;
    }
    let data = load(dir);
    if let Ok(bytes) = postcard::to_stdvec(&data) {
        let tmp = at.join(format!("{name}.{}", std::process::id()));
        if std::fs::write(&tmp, &bytes).is_ok() && std::fs::rename(&tmp, &path).is_ok() {
            // The snapshots of earlier builds, or databases, are stale.
            for old in std::fs::read_dir(&at).into_iter().flatten().flatten() {
                let n = old.file_name();
                let n = n.to_string_lossy();
                if n.starts_with("wvs-real-world-") && n.ends_with(".bin") && *n != *name {
                    let _ = std::fs::remove_file(old.path());
                }
            }
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    data
}

/// Where the snapshot of the real world in `dir` is (the test binary's
/// directory) and its name; `None` if a file it is keyed on can't be read.
fn snapshot_path(dir: &std::path::Path) -> Option<(std::path::PathBuf, String)> {
    use std::hash::{Hash, Hasher};
    let exe = std::env::current_exe().ok()?;
    let mut h = std::hash::DefaultHasher::new();
    for file in [
        dir.join("vmangos.sqlite"),
        dir.join("dbc.sqlite"),
        exe.clone(),
    ] {
        let file = std::fs::canonicalize(file).ok()?;
        let meta = std::fs::metadata(&file).ok()?;
        file.hash(&mut h);
        meta.len().hash(&mut h);
        meta.modified().ok()?.hash(&mut h);
    }
    let at = exe.parent()?.to_path_buf();
    Some((at, format!("wvs-real-world-{:016x}.bin", h.finish())))
}

pub fn test_config() -> RealmConfig {
    let visibility = crate::config::VisibilityConfig::default();
    RealmConfig {
        visibility_yards: visibility.continents_yards,
        instance_visibility_yards: visibility.instances_yards,
        battleground_visibility_yards: visibility.battleground_yards,
        save_interval: Duration::from_secs(300),
        motd: String::new(),
        everyone_gm: true,
        max_characters_per_account: 10,
        nav_dir: None,
        terrain_dir: None,
        tile_unload: Duration::from_secs(300),
        petition_signatures: 1,
        mail_delay: Duration::ZERO,
        instance_unload: Duration::from_millis(200),
        // Logouts over at once (a test sets the real delay where it waits
        // on it).
        logout_delay: Duration::ZERO,
        bg_min_players: Some(1),
        bg_start_delay: Duration::ZERO,
        bg_auto_leave: crate::game::battlegrounds::AUTO_LEAVE,
        bg_invite_for: crate::game::battlegrounds::INVITE_FOR,
        pvp_realm: false,
        max_packets_per_sec: 200,
        max_queries_per_sec: 2000,
        honor_reset_weekday: crate::game::honor::DEFAULT_RESET_WEEKDAY,
        spell_rolls: false,
        spell_travel: false,
        tiles_in_background: false,
        arena: Default::default(),
        rates: Default::default(),
        creatures: Default::default(),
        guards: Default::default(),
        // The tests' players fight with their weapon skills and Defense at
        // their caps, at every level a test gives them (mangos's
        // `AlwaysMaxSkillForLevel`): a new skill starts at 1 otherwise. A
        // test of the skills' own rise turns it off, or sets the value.
        players: crate::config::PlayersConfig {
            always_max_skill_for_level: true,
            ..Default::default()
        },
        anticheat: Default::default(),
        gm: Default::default(),
    }
}

pub fn realm_with(data: GameData) -> Arc<Realm> {
    let (store, startup) = Store::open(":memory:").expect("an in-memory store");
    Realm::new(test_config(), store, startup, data, None)
}

pub fn realm() -> Arc<Realm> {
    realm_with(GameData::empty())
}

/// A client connected to `realm`, as `account`.
pub struct Client {
    pub account: String,
    tx: mpsc::Sender<crate::game::conn::Inbound>,
    rx: kanal::Receiver<Arc<[u8]>>,
    pub guid: Guid,
    /// Everything received and not yet looked at.
    pub backlog: Vec<ServerOpcodeMessage>,
    /// Packets that didn't parse, as they came.
    pub raw: Vec<Arc<[u8]>>,
    /// Every frame, parsed or not, for [`Client::wait_frame`].
    frames: Vec<Arc<[u8]>>,
    /// Every frame ever received, kept for the client-emulation check
    /// (`tools/client_check`): the raw bytes as the writer framed them.
    /// Unlike `frames`, [`Client::clear`] does not drop these.
    captured: Vec<Arc<[u8]>>,
    /// Creates and forgets received and not yet waited for, by object:
    /// one packet can carry many.
    created: ahash::AHashMap<Guid, u32>,
    forgot: ahash::AHashMap<Guid, u32>,
}

impl Client {
    pub fn connect(realm: &Arc<Realm>, account: &str) -> Self {
        let (outbound, rx) = Outbound::detached();
        let (tx, inbound) = mpsc::channel(256);
        tokio::spawn(session::run(
            realm.clone(),
            account.to_owned(),
            Connection { outbound, inbound },
        ));
        Self {
            account: account.to_owned(),
            tx,
            rx,
            guid: Guid::zero(),
            backlog: Vec::new(),
            raw: Vec::new(),
            frames: Vec::new(),
            captured: Vec::new(),
            created: ahash::AHashMap::new(),
            forgot: ahash::AHashMap::new(),
        }
    }

    pub async fn send(&self, msg: impl Into<ClientOpcodeMessage>) {
        self.tx
            .send(crate::game::conn::Inbound::Msg(msg.into()))
            .await
            .expect("the session is running");
    }

    /// Send a packet as raw bytes, as the session gets those wow_messages
    /// can't read.
    pub async fn send_raw(&self, opcode: u16, body: &[u8]) {
        self.tx
            .send(crate::game::conn::Inbound::Raw(opcode, body.to_vec()))
            .await
            .expect("the session is running");
    }

    /// [`Client::send_raw`], or false if the session has gone.
    pub async fn try_send_raw(&self, opcode: u16, body: &[u8]) -> bool {
        self.tx
            .send(crate::game::conn::Inbound::Raw(opcode, body.to_vec()))
            .await
            .is_ok()
    }

    /// Send, or false if the session has gone.
    pub async fn try_send(&self, msg: impl Into<ClientOpcodeMessage>) -> bool {
        self.tx
            .send(crate::game::conn::Inbound::Msg(msg.into()))
            .await
            .is_ok()
    }

    /// Close the connection, as a client that goes away.
    pub fn disconnect(self) {}

    fn drain(&mut self) {
        while let Ok(Some(frame)) = self.rx.try_recv() {
            self.frames.push(frame.clone());
            self.captured.push(frame.clone());
            let parsed = ServerOpcodeMessage::read_unencrypted(&frame[..]);
            match parsed {
                Ok(m) => {
                    for g in creates(&m) {
                        *self.created.entry(g).or_default() += 1;
                    }
                    for g in forgotten(&m) {
                        *self.forgot.entry(g).or_default() += 1;
                    }
                    self.backlog.push(m)
                }
                // Packets wow_messages lays out differently from 1.12
                // (the game object query) are kept raw.
                Err(e) => {
                    eprintln!(
                        "unparsed 0x{:03X}: {e:?}",
                        u16::from_le_bytes([frame[2], frame[3]])
                    );
                    self.raw.push(frame)
                }
            }
        }
    }

    /// The first message `pick` takes, waiting up to [`WAIT`]; the messages
    /// before it stay in the backlog.
    pub async fn wait_for<T>(
        &mut self,
        what: &str,
        mut pick: impl FnMut(&ServerOpcodeMessage) -> Option<T>,
    ) -> T {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            self.drain();
            if let Some(i) = self.backlog.iter().position(|m| pick(m).is_some()) {
                let m = self.backlog.remove(i);
                return pick(&m).expect("picked once");
            }
            if tokio::time::Instant::now() >= deadline {
                let seen: Vec<String> = self.backlog.iter().map(|m| m.to_string()).collect();
                panic!("{}: no {what} within {WAIT:?}; got {seen:?}", self.account);
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    /// The body of the first frame with `opcode` that `pick` takes, parsed
    /// or not (for the packets laid out by hand), waiting up to [`WAIT`].
    pub async fn wait_frame(
        &mut self,
        what: &str,
        opcode: u16,
        pick: impl FnMut(&[u8]) -> bool,
    ) -> Vec<u8> {
        self.wait_frame_of(what, &[opcode], pick).await
    }

    /// [`Client::wait_frame`], for any of `opcodes`.
    pub async fn wait_frame_of(
        &mut self,
        what: &str,
        opcodes: &[u16],
        mut pick: impl FnMut(&[u8]) -> bool,
    ) -> Vec<u8> {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            self.drain();
            let found = self
                .frames
                .iter()
                .position(|f| opcodes.contains(&u16::from_le_bytes([f[2], f[3]])) && pick(&f[4..]));
            if let Some(i) = found {
                return self.frames.remove(i)[4..].to_vec();
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("{}: no {what} within {WAIT:?}", self.account);
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    /// Whether a message `pick` takes arrives within `within`.
    pub async fn gets<T>(
        &mut self,
        within: Duration,
        mut pick: impl FnMut(&ServerOpcodeMessage) -> Option<T>,
    ) -> Option<T> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            self.drain();
            if let Some(i) = self.backlog.iter().position(|m| pick(m).is_some()) {
                let m = self.backlog.remove(i);
                return pick(&m);
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    pub fn clear(&mut self) {
        self.drain();
        self.backlog.clear();
        self.frames.clear();
    }

    /// Every frame this client has received, as the writer task framed it
    /// (`[size u16 BE][opcode u16 LE][body]`), for the client-emulation
    /// check. Drains the socket first, so it catches up to now.
    pub fn captured(&mut self) -> Vec<Arc<[u8]>> {
        self.drain();
        self.captured.clone()
    }

    /// Create a character and log it in, waiting for the world.
    pub async fn enter(&mut self, name: &str, race: Race, class: Class) -> Guid {
        self.send(CMSG_CHAR_CREATE {
            name: name.to_owned(),
            race,
            class,
            gender: Gender::Female,
            skin_color: 1,
            face: 1,
            hair_style: 1,
            hair_color: 1,
            facial_hair: 1,
        })
        .await;
        let result = self
            .wait_for("SMSG_CHAR_CREATE", |m| match m {
                ServerOpcodeMessage::SMSG_CHAR_CREATE(c) => Some(c.result),
                _ => None,
            })
            .await;
        assert_eq!(result, WorldResult::CharCreateSuccess, "creating {name}");
        self.send(CMSG_CHAR_ENUM {}).await;
        let guid = self
            .wait_for("SMSG_CHAR_ENUM", |m| match m {
                ServerOpcodeMessage::SMSG_CHAR_ENUM(e) => {
                    e.characters.iter().find(|c| c.name == name).map(|c| c.guid)
                }
                _ => None,
            })
            .await;
        self.login(guid).await;
        guid
    }

    pub async fn login(&mut self, guid: Guid) {
        self.send(CMSG_PLAYER_LOGIN { guid }).await;
        self.wait_for("SMSG_LOGIN_VERIFY_WORLD", |m| match m {
            ServerOpcodeMessage::SMSG_LOGIN_VERIFY_WORLD(_) => Some(()),
            ServerOpcodeMessage::SMSG_CHARACTER_LOGIN_FAILED(f) => {
                panic!("login failed: {:?}", f.result)
            }
            _ => None,
        })
        .await;
        self.guid = guid;
        self.send(CMSG_SET_ACTIVE_MOVER { guid }).await;
    }

    /// Wait until this client is told of `object` (a create block for it).
    pub async fn sees(&mut self, object: Guid) {
        assert!(
            self.sees_within(object, WAIT).await,
            "{}: never told of {object}",
            self.account
        );
    }

    /// [`Client::sees`], on a map a busy machine may slow: it waits until
    /// this client is told of `object`, or both [`WAIT`] has gone by and
    /// `map` has run [`UPDATES`] updates since, however long they take.
    pub async fn sees_on(&mut self, map: &crate::game::map::MapHandle, object: Guid) {
        let updates = || map.query(|m| m.stats.updates);
        let from = updates().await.unwrap_or(0);
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            if self.sees_within(object, Duration::from_millis(100)).await {
                return;
            }
            let ran = updates().await.unwrap_or(u64::MAX).saturating_sub(from);
            if ran >= UPDATES && tokio::time::Instant::now() >= deadline {
                break;
            }
        }
        panic!(
            "{}: never told of {object} in {UPDATES} updates",
            self.account
        );
    }

    /// Whether this client is told of `object` within `within`.
    pub async fn sees_within(&mut self, object: Guid, within: Duration) -> bool {
        self.counted(object, within, true).await
    }

    /// Wait until this client is told to forget `object`.
    pub async fn forgets(&mut self, object: Guid) {
        assert!(
            self.counted(object, WAIT, false).await,
            "{}: never told to forget {object}",
            self.account
        );
    }

    async fn counted(&mut self, object: Guid, within: Duration, create: bool) -> bool {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            self.drain();
            let counts = if create {
                &mut self.created
            } else {
                &mut self.forgot
            };
            if let Some(n) = counts.get_mut(&object)
                && *n > 0
            {
                *n -= 1;
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    pub async fn say(&self, text: &str) {
        self.send(CMSG_MESSAGECHAT {
            chat_type: CMSG_MESSAGECHAT_ChatType::Say,
            language: Language::Universal,
            message: text.to_owned(),
        })
        .await;
    }

    pub async fn heartbeat(&self, x: f32, y: f32, z: f32) {
        self.send(MSG_MOVE_HEARTBEAT_Client {
            info: MovementInfo {
                flags: MovementInfo_MovementFlags::empty(),
                timestamp: 0,
                position: Vector3d { x, y, z },
                orientation: 0.0,
                fall_time: 0.0,
            },
        })
        .await;
    }

    pub async fn system_message(&mut self, containing: &str) -> String {
        let containing = containing.to_owned();
        self.wait_for("a system message", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c)
                if matches!(c.chat_type, SMSG_MESSAGECHAT_ChatType::System { .. })
                    && c.message.contains(&containing) =>
            {
                Some(c.message.clone())
            }
            _ => None,
        })
        .await
    }
}

fn objects(m: &ServerOpcodeMessage) -> &[Object] {
    match m {
        ServerOpcodeMessage::SMSG_UPDATE_OBJECT(u) => &u.objects,
        ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(u) => &u.objects,
        _ => &[],
    }
}

/// Wait until `map` has run an update from now. A client is told of what
/// comes into the map at the end of the event that brought it, before an
/// update has run its first turn (its spawned events): a test that wants
/// those waits for this.
pub async fn an_update(map: &crate::game::map::MapHandle) {
    let from = map.query(|m| m.stats.updates).await.unwrap_or(0);
    let deadline = tokio::time::Instant::now() + WAIT;
    while map.query(|m| m.stats.updates).await.unwrap_or(u64::MAX) <= from {
        assert!(tokio::time::Instant::now() < deadline, "no update");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// The objects an update packet creates.
pub fn creates(m: &ServerOpcodeMessage) -> Vec<Guid> {
    objects(m)
        .iter()
        .filter_map(|o| match &o.update_type {
            Object_UpdateType::CreateObject { guid3, .. }
            | Object_UpdateType::CreateObject2 { guid3, .. } => Some(*guid3),
            _ => None,
        })
        .collect()
}

/// The objects an update packet tells the client to forget.
pub fn forgotten(m: &ServerOpcodeMessage) -> Vec<Guid> {
    objects(m)
        .iter()
        .flat_map(|o| match &o.update_type {
            Object_UpdateType::OutOfRangeObjects { guids } => guids.clone(),
            _ => Vec::new(),
        })
        .collect()
}

/// The values updates in an update packet, by object.
pub fn values(m: &ServerOpcodeMessage) -> Vec<(Guid, UpdateMask)> {
    objects(m)
        .iter()
        .filter_map(|o| match &o.update_type {
            Object_UpdateType::Values { guid1, mask1 } => Some((*guid1, mask1.clone())),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    /// The real world's snapshot is a new one when either database changes,
    /// in size or in time, so a snapshot never outlives what it was made
    /// from (the test binary is keyed the same way).
    #[test]
    fn a_real_world_snapshot_is_keyed_on_the_databases() {
        let dir = std::env::temp_dir().join(format!("wvs-snapshot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |db: &str, body: &str, secs: u64| {
            let path = dir.join(db);
            std::fs::write(&path, body).unwrap();
            let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(at)
                .unwrap();
        };
        write("vmangos.sqlite", "world", 1_000);
        write("dbc.sqlite", "client", 1_000);
        let key = || super::snapshot_path(&dir).map(|(_, name)| name);
        let first = key().expect("keyed");
        assert!(
            first.starts_with("wvs-real-world-") && first.ends_with(".bin"),
            "{first}"
        );
        assert_eq!(key().as_ref(), Some(&first), "the same files, the same key");
        write("vmangos.sqlite", "world!", 1_000);
        let resized = key().unwrap();
        assert_ne!(resized, first, "a database's size");
        write("vmangos.sqlite", "world", 2_000);
        let retimed = key().unwrap();
        assert_ne!(retimed, first, "a database's time");
        assert_ne!(retimed, resized);
        write("vmangos.sqlite", "world", 1_000);
        write("dbc.sqlite", "client", 2_000);
        assert_ne!(key().unwrap(), first, "the client's database too");
        std::fs::remove_file(dir.join("dbc.sqlite")).unwrap();
        assert_eq!(key(), None, "no snapshot without its databases");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
