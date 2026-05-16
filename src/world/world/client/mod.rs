//! Per-connected-player state.
//!
//! Split into three types:
//!
//! - [`PlayerSession`]: network half — owns the outbound channel sender, the
//!   reader task handle, the inbound mpsc receiver, the account name, and
//!   protocol-flow state like `in_process_of_teleport`. Anything I/O-shaped
//!   lives here.
//! - [`Player`]: game half — wraps the [`Character`] and game-state geometry
//!   methods. AI / combat / spell systems should take `&mut Player` so they
//!   structurally cannot send packets from a CPU loop by accident.
//! - [`Client`]: thin composition of the two, preserved as the public surface
//!   call-sites use today.
//!
//! ## Outbound channel model
//!
//! Each connection has a per-client bounded `mpsc::Sender<Vec<u8>>` and a
//! dedicated writer task that owns the socket write half + ARC4 encrypter and
//! drains the channel. World-tick code calls `send_message` / `send_raw` /
//! `send_opcode` which serialize the message (unencrypted header + body) and
//! `try_send` the bytes — non-blocking. The writer task encrypts the 4-byte
//! header and writes header+body to the socket; if a slow client backs up its
//! TCP buffer, only that client's writer task stalls, not the world tick.
//!
//! When the channel fills (i.e., a single client is so far behind that we've
//! buffered `CHANNEL_CAPACITY` packets for it), `try_send` returns `Full` and
//! we drop the packet — there's no point queueing minutes of stale state. The
//! per-client `dropped_packets` counter is logged occasionally so an operator
//! can tell which clients are flailing.

pub(crate) mod character_screen_client;

use crate::world::world_opcode_handler::character::Character;
use crate::world::world_opcode_handler::{write_message_test, write_server_test};
use character_screen_client::{CharacterScreenClient, CharacterScreenProgress};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;

/// Monotonically increasing counter of successful socket writes (any opcode,
/// any client). Sampled per-tick by `World::tick` to compute packets-per-second
/// for Tracy. Crate-private; external readers should call
/// [`outgoing_packet_count`].
pub(crate) static OUTGOING_PACKETS: AtomicU64 = AtomicU64::new(0);

/// Snapshot of the lifetime outgoing-packet counter. Cheap (one atomic load).
pub fn outgoing_packet_count() -> u64 {
    OUTGOING_PACKETS.load(Ordering::Relaxed)
}

/// Per-client outbound channel depth. With 30 Hz ticks and ~50 broadcasts per
/// tick at peak, 512 lets a writer task fall ~10 seconds behind before we
/// start shedding packets — well beyond what a healthy client ever needs.
pub(crate) const OUTBOUND_CHANNEL_CAPACITY: usize = 512;

use wow_world_base::geometry::distance_between;
use wow_world_base::vanilla::position::Position;
use wow_world_messages::vanilla::opcodes::{ClientOpcodeMessage, ServerOpcodeMessage};
use wow_world_messages::vanilla::{
    Language, MovementInfo, PlayerChatTag, SMSG_MESSAGECHAT, SMSG_MESSAGECHAT_ChatType,
    ServerMessage, Vector3d,
};
use wow_world_messages::Guid;

/// Sender half of the per-client outbound channel. Cloned by the reader task
/// (for pongs) and held by the `PlayerSession` / `CharacterScreenClient` (for
/// world-tick sends).
pub(crate) type OutboundTx = mpsc::Sender<Vec<u8>>;

/// Network half of a connected player: outbound channel, reader task, inbound
/// channel, account name, and protocol-flow flags. Game-state mutations
/// (combat, AI, spells) should never need this — take `&mut Player` instead.
#[derive(Debug)]
pub struct PlayerSession {
    pub(crate) account_name: String,
    pub(crate) outbound: OutboundTx,
    pub(crate) dropped_packets: Arc<AtomicU64>,
    pub(crate) received_messages: Receiver<ClientOpcodeMessage>,
    pub(crate) reader_handle: JoinHandle<()>,
    pub(crate) writer_handle: JoinHandle<()>,
    /// `true` between sending a transfer/teleport message and receiving the
    /// matching `MSG_MOVE_WORLDPORT_ACK`. The opcode handler uses it to
    /// suppress duplicate ACK processing.
    pub in_process_of_teleport: bool,
}

impl PlayerSession {
    /// Serialize `m` to its `[size BE u16][opcode LE u16][body]` wire form
    /// and queue it on the outbound channel. The header bytes are
    /// re-encrypted inside the writer task — encryption is stateful and must
    /// stay sequential, but only with respect to that one client.
    pub async fn send_message(&mut self, m: impl ServerMessage + Sync) {
        write_message_test(&m);
        let mut buf = Vec::with_capacity(m.size_without_header() as usize + 4);
        if m.write_unencrypted_server(&mut buf).is_err() {
            return;
        }
        self.queue_buf(buf);
    }

    pub async fn send_opcode(&mut self, m: &ServerOpcodeMessage) {
        write_server_test(m);
        let mut buf = Vec::new();
        if m.write_unencrypted_server(&mut buf).is_err() {
            return;
        }
        self.queue_buf(buf);
    }

    /// Send a pre-serialized body. Caller supplies the body and opcode; we
    /// frame the wire header here and push to the channel. Used by the
    /// serialize-once broadcast path in [`crate::world::aoi`].
    pub(crate) async fn send_raw(&mut self, opcode: u16, body: &[u8]) -> bool {
        let size_for_header = (body.len() as u16).saturating_add(2);
        let mut buf = Vec::with_capacity(4 + body.len());
        buf.extend_from_slice(&size_for_header.to_be_bytes());
        buf.extend_from_slice(&opcode.to_le_bytes());
        buf.extend_from_slice(body);
        self.queue_buf(buf)
    }

    /// Queue a fully-framed `[size_BE u16][opcode_LE u16][body]` buffer
    /// directly onto the outbound channel. The writer task re-encrypts the
    /// 4-byte header before writing to the socket. Used by the opcode-enum
    /// broadcast path so we serialize once and clone the framed buffer per
    /// recipient instead of re-framing for each.
    pub(crate) fn try_queue_frame(&self, buf: Vec<u8>) -> bool {
        self.queue_buf(buf)
    }

    /// Returns `true` if the buffer was queued, `false` if the channel was
    /// full (slow client) or closed (disconnected). Drops are counted so we
    /// can spot which clients are falling behind.
    fn queue_buf(&self, buf: Vec<u8>) -> bool {
        match self.outbound.try_send(buf) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.dropped_packets.fetch_add(1, Ordering::Relaxed);
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    pub async fn send_system_message(&mut self, s: impl Into<String>) {
        self.send_message(SMSG_MESSAGECHAT {
            chat_type: SMSG_MESSAGECHAT_ChatType::System {
                sender2: Guid::new(0),
            },
            language: Language::Universal,
            message: s.into(),
            tag: PlayerChatTag::None,
        })
        .await;
    }

    pub fn reader_is_finished(&self) -> bool {
        self.reader_handle.is_finished()
    }
}

/// Game-state half of a connected player. Holds the `Character` plus
/// game-only helpers. Doesn't know anything about sockets.
#[derive(Debug)]
pub struct Player {
    pub(crate) character: Character,
}

impl Player {
    pub fn character(&self) -> &Character {
        &self.character
    }

    pub fn character_mut(&mut self) -> &mut Character {
        &mut self.character
    }

    pub fn set_movement_info(&mut self, info: MovementInfo) {
        self.character.info = info;
    }

    pub fn position(&self) -> Position {
        Position::new(
            self.character.map,
            self.character.info.position.x,
            self.character.info.position.y,
            self.character.info.position.z,
            self.character.info.orientation,
        )
    }

    pub fn distance_to_position(&self, position: &Position) -> Option<f32> {
        if self.character.map != position.map {
            return None;
        }
        let here = Vector3d {
            x: self.character.info.position.x,
            y: self.character.info.position.y,
            z: self.character.info.position.z,
        };
        let there = Vector3d {
            x: position.x,
            y: position.y,
            z: position.z,
        };
        Some(distance_between(here, there))
    }
}

/// Composition of [`PlayerSession`] and [`Player`]. Preserved as the public
/// type call-sites use today; delegates every previous method to the
/// appropriate half.
#[derive(Debug)]
pub struct Client {
    pub session: PlayerSession,
    pub player: Player,
}

impl Client {
    pub(crate) fn into_character_screen_client(self) -> CharacterScreenClient {
        CharacterScreenClient {
            status: CharacterScreenProgress::CharacterScreen,
            received_messages: self.session.received_messages,
            outbound: self.session.outbound,
            dropped_packets: self.session.dropped_packets,
            account_name: self.session.account_name,
            reader_handle: self.session.reader_handle,
            writer_handle: self.session.writer_handle,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        character: Character,
        received_messages: Receiver<ClientOpcodeMessage>,
        outbound: OutboundTx,
        dropped_packets: Arc<AtomicU64>,
        account_name: String,
        reader_handle: JoinHandle<()>,
        writer_handle: JoinHandle<()>,
    ) -> Self {
        Self {
            session: PlayerSession {
                account_name,
                outbound,
                dropped_packets,
                received_messages,
                reader_handle,
                writer_handle,
                in_process_of_teleport: false,
            },
            player: Player { character },
        }
    }

    pub fn character(&self) -> &Character {
        self.player.character()
    }

    pub fn character_mut(&mut self) -> &mut Character {
        self.player.character_mut()
    }

    pub fn set_movement_info(&mut self, info: MovementInfo) {
        self.player.set_movement_info(info);
    }

    pub fn received_messages(&mut self) -> &mut Receiver<ClientOpcodeMessage> {
        &mut self.session.received_messages
    }

    pub async fn send_message(&mut self, m: impl ServerMessage + Sync) {
        self.session.send_message(m).await;
    }

    pub async fn send_opcode(&mut self, m: &ServerOpcodeMessage) {
        self.session.send_opcode(m).await;
    }

    pub(crate) async fn send_raw(&mut self, opcode: u16, body: &[u8]) -> bool {
        self.session.send_raw(opcode, body).await
    }

    pub(crate) fn try_queue_frame(&self, buf: Vec<u8>) -> bool {
        self.session.try_queue_frame(buf)
    }

    pub async fn send_system_message(&mut self, s: impl Into<String>) {
        self.session.send_system_message(s).await;
    }

    pub fn position(&self) -> Position {
        self.player.position()
    }

    pub fn distance_to_center(&self, other: &Self) -> Option<f32> {
        let position = other.position();
        self.player.distance_to_position(&position)
    }

    pub fn distance_to_position(&self, position: &Position) -> Option<f32> {
        self.player.distance_to_position(position)
    }

    pub fn reader_is_finished(&self) -> bool {
        self.session.reader_is_finished()
    }

    pub fn in_process_of_teleport(&self) -> bool {
        self.session.in_process_of_teleport
    }

    pub fn set_in_process_of_teleport(&mut self, v: bool) {
        self.session.in_process_of_teleport = v;
    }
}
