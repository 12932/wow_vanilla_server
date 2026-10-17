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

/// The real world's data for the tests that read it, laid out as the server
/// looks for it under `data/`: `db` (`vmangos.sqlite` and `dbc.sqlite`),
/// `nav` (a rustigator bake) and `terrain` (Thistle Tea's tiles). From
/// `WVS_DATA` if it is set, else `data/` in the checkout. A test whose data
/// isn't there passes without running.
pub fn real_data(what: &str) -> std::path::PathBuf {
    std::env::var_os("WVS_DATA")
        .map_or_else(
            || std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data"),
            std::path::PathBuf::from,
        )
        .join(what)
}

pub fn test_config() -> RealmConfig {
    RealmConfig {
        visibility_yards: 100.0,
        save_interval: Duration::from_secs(300),
        motd: String::new(),
        everyone_gm: true,
        max_characters_per_account: 10,
        nav_dir: None,
        terrain_dir: None,
        petition_signatures: 1,
        mail_delay: Duration::ZERO,
        instance_unload: Duration::from_millis(200),
        // Logouts over at once (a test sets the real delay where it waits
        // on it).
        logout_delay: Duration::ZERO,
        bg_min_players: Some(1),
        bg_start_delay: Duration::ZERO,
        pvp_realm: false,
        max_packets_per_sec: 200,
        honor_reset_weekday: crate::game::honor::DEFAULT_RESET_WEEKDAY,
        spell_rolls: false,
        spell_travel: false,
        arena: Default::default(),
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
