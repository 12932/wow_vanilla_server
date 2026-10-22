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
        flight_visibility_yards: visibility.flight_yards,
        // Every refresh in its update, however slow the machine (a test of
        // the budget sets one).
        visibility_budget: Duration::from_secs(1),
        visibility_threads: visibility.refresh_threads,
        distant_move_yards: visibility.distant_move_yards,
        distant_move_every: visibility.distant_move_every,
        parallel_moves: crate::config::WorldConfig::default().parallel_moves,
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
        // A client gone leaves the world at once (a test of the linger
        // sets it).
        disconnect_linger: Duration::ZERO,
        bg_min_players: Some(1),
        bg_start_delay: Duration::ZERO,
        bg_auto_leave: crate::game::battlegrounds::AUTO_LEAVE,
        bg_invite_for: crate::game::battlegrounds::INVITE_FOR,
        pvp_realm: false,
        max_packets_per_sec: 200,
        max_queries_per_sec: 2000,
        max_queued_packets: 64,
        memory_guard_fraction: 0.8,
        // A test asks `/who` as often as it likes.
        packet_cooldown: Duration::ZERO,
        honor_reset_weekday: crate::game::honor::DEFAULT_RESET_WEEKDAY,
        spell_rolls: false,
        spell_travel: false,
        tiles_in_background: false,
        bg_cast_deserter: true,
        bg_premature_finish: Duration::from_secs(300),
        min_honor_kills: crate::game::honor::MINIMUM_KILLS,
        max_who_list_returns: crate::game::map::social::WHO_LIMIT,
        weather: true,
        weather_change: crate::game::weather::CHANGE_EVERY,
        auction_deposit_min: 0,
        arena: Default::default(),
        chat: Default::default(),
        rates: Default::default(),
        // A flat twelve-second leash, looked at every update: the leash's
        // tests time it to the millisecond (those of its level brackets and
        // its looks set them).
        creatures: crate::config::CreaturesConfig {
            leash_timeout_by_level: vec!["1:12".to_owned()],
            leash_check_ms: 0,
            // No grace after a spawn: the tests pull at once (its own test
            // sets it).
            spawn_grace_ms: 0,
            ..Default::default()
        },
        guards: Default::default(),
        // The tests' players fight with their weapon skills and Defense at
        // their caps, at every level a test gives them (mangos's
        // `AlwaysMaxSkillForLevel`): a new skill starts at 1 otherwise. A
        // test of the skills' own rise turns it off, or sets the value.
        players: crate::config::PlayersConfig {
            always_max_skill_for_level: true,
            ..Default::default()
        },
        environment: Default::default(),
        quests: Default::default(),
        spells: Default::default(),
        instances: Default::default(),
        outdoor_pvp: Default::default(),
        events: Default::default(),
        anticheat: Default::default(),
        gm: Default::default(),
        broadcast_batch: Duration::ZERO,
    }
}

/// A banker at `g`'s side and the bank opened at it, as
/// `CMSG_BANKER_ACTIVATE` opens it: for a test that moves things into or
/// out of the bank (`map/bank.rs`). The banker's guid.
pub async fn open_bank(map: &crate::game::map::MapHandle, g: Guid) -> Guid {
    map.query(move |m| {
        let template = crate::game::data::CreatureTemplate {
            entry: 2455,
            name: "Olivia Burnside".into(),
            npc_flags: crate::game::map::npc::NPC_FLAG_BANKER,
            // Friendly to all.
            faction: 35,
            health_max: 100,
            ..Default::default()
        };
        let p = &m.players[&g];
        let at = Vector3d {
            x: p.position().x + 1.0,
            ..p.position()
        };
        let counter = m.creature_counter();
        let c = crate::game::map::creature::Creature::new(
            crate::game::guid::unit(template.entry, counter),
            &template,
            30,
            at,
            0.0,
        );
        let banker = m.add_creature(c);
        crate::game::map::npc::banker(m, g, banker);
        assert_eq!(m.players[&g].misc.banker, Some(banker), "the bank opens");
        banker
    })
    .await
    .expect("the map answers")
}

/// A mailbox at `g`'s side, for a test that uses the mail (whose opcodes
/// want one within reach, as mangos's `CheckMailBox`): its guid.
pub fn mailbox(m: &mut crate::game::map::Map, g: Guid) -> Guid {
    let template = crate::game::data::GameObjectTemplate {
        entry: 32_349,
        kind: crate::game::map::social::GAMEOBJECT_TYPE_MAILBOX,
        display_id: 3_771,
        name: "Mailbox".into(),
        size: 1.0,
        ..Default::default()
    };
    let at = m.players[&g].position();
    let counter = m.object_counter();
    let spawn = crate::game::data::GameObjectSpawn {
        guid: counter,
        entry: template.entry,
        map: m.key.map,
        x: at.x + 1.0,
        y: at.y,
        z: at.z,
        state: 1,
        ..Default::default()
    };
    let mut go = crate::game::map::gameobject::GameObject::from_spawn(&template, &spawn);
    go.spawn_guid = None;
    m.add_gameobject(go)
}

/// [`mailbox`], beside `g` wherever they are.
pub async fn mailbox_for(realm: &Arc<Realm>, g: Guid) -> Guid {
    let map = realm.registry.online(g).expect("online").map();
    map.query(move |m| mailbox(m, g))
        .await
        .expect("the map answers")
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
    rx: crate::game::conn::Frames,
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
    /// The order counter of the last teleport received, for its
    /// acknowledgement ([`Client::teleport_order`]).
    teleport_order: u32,
    realm: Arc<Realm>,
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
            teleport_order: 0,
            realm: realm.clone(),
        }
    }

    /// The order counter `MSG_MOVE_TELEPORT_ACK` answers: the last teleport
    /// received's, as a client echoes it (a stale one is refused).
    pub fn teleport_order(&self) -> u32 {
        self.teleport_order
    }

    pub async fn send(&self, msg: impl Into<ClientOpcodeMessage>) {
        let mut msg = msg.into();
        self.at_a_mailbox(&mut msg).await;
        self.tx
            .send(crate::game::conn::Inbound::Msg(msg))
            .await
            .expect("the session is running");
    }

    /// A mail opcode naming no mailbox is sent from one: a mailbox is put
    /// at the player's side first ([`mailbox_for`]), as the mail is used
    /// at one alone (mangos's `CheckMailBox`). A test of the check names
    /// a mailbox of its own, or something else.
    async fn at_a_mailbox(&self, msg: &mut ClientOpcodeMessage) {
        use ClientOpcodeMessage as C;
        let slot = match msg {
            C::CMSG_GET_MAIL_LIST(m) => &mut m.mailbox,
            C::CMSG_SEND_MAIL(m) => &mut m.mailbox,
            C::CMSG_MAIL_TAKE_MONEY(m) => &mut m.mailbox,
            C::CMSG_MAIL_TAKE_ITEM(m) => &mut m.mailbox,
            C::CMSG_MAIL_MARK_AS_READ(m) => &mut m.mailbox,
            C::CMSG_MAIL_RETURN_TO_SENDER(m) => &mut m.mailbox_id,
            C::CMSG_MAIL_DELETE(m) => &mut m.mailbox_id,
            C::CMSG_MAIL_CREATE_TEXT_ITEM(m) => &mut m.mailbox,
            _ => return,
        };
        if slot.is_zero() && self.realm.registry.online(self.guid).is_some() {
            *slot = mailbox_for(&self.realm, self.guid).await;
        }
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

    pub(crate) fn drain(&mut self) {
        while let Some(frame) = self.rx.try_recv() {
            self.frames.push(frame.clone());
            self.captured.push(frame.clone());
            match ServerOpcodeMessage::read_unencrypted(&frame[..]) {
                Ok(m) => {
                    if let ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) = &m {
                        self.teleport_order = t.movement_counter;
                    }
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

    /// The next `SMSG_SPELL_GO` of `spell` that lists a miss: the target
    /// it missed and why, as 1.12's reason byte (2 a resist, 7 immune, 11
    /// a reflection), the miss told in the go as mangos-classic's
    /// `WriteSpellGoTargets` tells it.
    pub async fn go_miss(&mut self, spell: u32) -> (Guid, u8) {
        self.wait_for("a miss in SMSG_SPELL_GO", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(g) if g.spell == spell => g
                .misses
                .first()
                .map(|x| (x.target, miss_reason(&x.miss_info))),
            _ => None,
        })
        .await
    }

    /// The first miss of `spell` this client was told of: in the go's
    /// misses (rolled as the spell went, mangos-classic's
    /// `WriteSpellGoTargets`) or in `SMSG_SPELLLOGMISS` (one that came
    /// about as it landed, `SendSpellMiss`). Its target and reason byte.
    pub async fn told_miss(&mut self, spell: u32) -> (Guid, u8) {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            self.drain();
            let found = self.frames.iter().enumerate().find_map(|(i, f)| {
                let body = &f[4..];
                match u16::from_le_bytes([f[2], f[3]]) {
                    0x132 => go_miss_of(f, spell),
                    0x24B if body.len() >= 26 && body[..4] == spell.to_le_bytes() => Some((
                        Guid::new(u64::from_le_bytes(body[17..25].try_into().ok()?)),
                        body[25],
                    )),
                    _ => None,
                }
                .map(|m| (i, m))
            });
            if let Some((i, m)) = found {
                self.frames.remove(i);
                return m;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("{}: no miss of {spell} within {WAIT:?}", self.account);
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

    /// The bodies of every frame of `opcode` received so far and not yet
    /// taken, in order, taken now: none is no failure.
    pub fn take_frames(&mut self, opcode: u16) -> Vec<Vec<u8>> {
        self.drain();
        let mut out = Vec::new();
        self.frames.retain(|f| {
            let of = u16::from_le_bytes([f[2], f[3]]) == opcode;
            if of {
                out.push(f[4..].to_vec());
            }
            !of
        });
        out
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
                fall_time: 0,
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

/// Wait until `g` has left the world, saved and offline, as their client
/// going away has them leave it. A connection of the account before that
/// takes them over where they are (one session an account), with no
/// logout: a test of what a logout keeps waits for this before it comes
/// back.
pub async fn offline(realm: &Arc<Realm>, g: Guid) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while realm.registry.online(g).is_some() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "{g:?} never went offline"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
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

/// A go's miss as 1.12's reason byte (`SpellMissInfo`).
pub fn miss_reason(m: &SpellMiss_SpellMissInfo) -> u8 {
    match m {
        SpellMiss_SpellMissInfo::None => 0,
        SpellMiss_SpellMissInfo::Miss => 1,
        SpellMiss_SpellMissInfo::Resist => 2,
        SpellMiss_SpellMissInfo::Dodge => 3,
        SpellMiss_SpellMissInfo::Parry => 4,
        SpellMiss_SpellMissInfo::Block => 5,
        SpellMiss_SpellMissInfo::Evade => 6,
        SpellMiss_SpellMissInfo::Immune => 7,
        SpellMiss_SpellMissInfo::Immune2 => 8,
        SpellMiss_SpellMissInfo::Deflect => 9,
        SpellMiss_SpellMissInfo::Absorb => 10,
        SpellMiss_SpellMissInfo::Reflect { .. } => 11,
    }
}

/// The first miss an `SMSG_SPELL_GO` frame of `spell` lists.
fn go_miss_of(frame: &[u8], spell: u32) -> Option<(Guid, u8)> {
    match ServerOpcodeMessage::read_unencrypted(frame).ok()? {
        ServerOpcodeMessage::SMSG_SPELL_GO(g) if g.spell == spell => g
            .misses
            .first()
            .map(|x| (x.target, miss_reason(&x.miss_info))),
        _ => None,
    }
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
