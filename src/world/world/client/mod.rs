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
//! Each connection has a per-client `mpsc::UnboundedSender<Arc<[u8]>>` and a
//! dedicated writer task that owns the socket write half + ARC4 encrypter and
//! drains the channel. World-tick code calls `send_message` / `send_raw` /
//! `send_opcode` which serialize the message (unencrypted header + body) and
//! `try_send` an `Arc<[u8]>` — non-blocking. The broadcast fan-out
//! (`aoi::broadcast_opcode_within_aoi`) serializes once and refcount-bumps
//! the same `Arc` into each recipient's channel — no per-recipient alloc.
//! The writer task drains up to 64 queued buffers per wake via `recv_many`,
//! encrypts each 4-byte header into a stack scratch buffer (the shared
//! `Arc<[u8]>` body is read-only), concatenates header + body[4..] into a
//! reusable batch scratch, and emits the whole batch with a single
//! `write_all` — one syscall per burst instead of two per packet. If a slow
//! client backs up its TCP buffer, only that client's writer task stalls,
//! not the world tick.
//!
//! When the **byte budget** fills (i.e., a single client is so far behind
//! that we've buffered `OUTBOUND_CHANNEL_BYTES` of pending payload for it),
//! `try_send` returns false and we drop the packet — there's no point
//! queueing minutes of stale state. Sizing is by bytes rather than by
//! message count because message sizes range from ~30 bytes (movement
//! heartbeats) to ~11 KB (a mass `SMSG_UPDATE_OBJECT` with thousands of
//! `OutOfRangeObjects` guids); a count-based cap of 512 messages was fine
//! at typical traffic but couldn't admit the big destroy-broadcast under
//! a 1400-bot mass-disconnect with the channel already partly full. The
//! per-client `dropped_packets` counter is logged on transition from
//! 0 → nonzero so an operator can tell which clients are flailing.

pub(crate) mod character_screen_client;
pub mod test_support;

use crate::world::world_opcode_handler::character::Character;
use crate::world::world_opcode_handler::{write_message_test, write_server_test};
use character_screen_client::{CharacterScreenClient, CharacterScreenProgress};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc::Receiver;
use tokio::sync::Semaphore;
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

// `OUTBOUND_CHANNEL_BYTES` lives in config (`[network] outbound_channel_bytes`).
// Default 1 MiB per client; at 10 000 clients that's ~10 GiB worst-case
// buffer memory. Sized by bytes (not messages) so a single huge
// `SMSG_UPDATE_OBJECT` (e.g. mass-disconnect `OutOfRangeObjects` carrying
// thousands of guids) always fits as long as there's budget remaining,
// regardless of how many small heartbeat frames are queued ahead of it.

use wow_world_base::geometry::distance_between;
use wow_world_base::vanilla::position::Position;
use wow_world_messages::vanilla::opcodes::{ClientOpcodeMessage, ServerOpcodeMessage};
use wow_world_messages::vanilla::{
    Language, MovementInfo, PlayerChatTag, SMSG_MESSAGECHAT, SMSG_MESSAGECHAT_ChatType,
    ServerMessage, Vector3d,
};
use wow_world_messages::Guid;

/// Sender half of the per-client outbound channel paired with a per-client
/// byte budget. Cloned by the reader task (for pongs) and held by the
/// `PlayerSession` / `CharacterScreenClient` (for world-tick sends).
///
/// Channel is **unbounded** at the mpsc layer; backpressure is enforced
/// via the shared `Semaphore` of `OUTBOUND_CHANNEL_BYTES` permits. Each
/// `try_send` acquires `buf.len()` permits up front (forgetting them so
/// they don't auto-release on drop); the writer task `add_permits`
/// the same count after popping each buffer. Net result: at most
/// `OUTBOUND_CHANNEL_BYTES` of payload is pending per client, regardless
/// of whether the queue is one 1 MiB packet or 30 000 30-byte heartbeats.
#[derive(Clone)]
pub struct OutboundTx {
    sender: kanal::AsyncSender<Arc<[u8]>>,
    byte_budget: Arc<Semaphore>,
}

impl OutboundTx {
    pub fn new(
        sender: kanal::AsyncSender<Arc<[u8]>>,
        byte_budget: Arc<Semaphore>,
    ) -> Self {
        Self { sender, byte_budget }
    }

    pub(crate) fn try_send(&self, buf: Arc<[u8]>) -> bool {
        let n = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        match self.byte_budget.try_acquire_many(n) {
            Ok(permit) => {
                permit.forget();
                self.sender.try_send(buf).is_ok()
            }
            Err(_) => false,
        }
    }
}

impl std::fmt::Debug for OutboundTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutboundTx")
            .field("byte_budget_available", &self.byte_budget.available_permits())
            .finish()
    }
}

/// Network half of a connected player: outbound channel, reader task, inbound
/// channel, account name, and protocol-flow flags. Game-state mutations
/// (combat, AI, spells) should never need this — take `&mut Player` instead.
#[derive(Debug)]
pub struct PlayerSession {
    /// `Arc<str>` so the broadcast view (`BroadcastTarget`) can clone
    /// this for free — cloning a `String` per recipient per tick would
    /// be a fresh heap alloc + memcpy at 2500-bot density.
    pub(crate) account_name: Arc<str>,
    pub(crate) outbound: OutboundTx,
    pub(crate) dropped_packets: Arc<AtomicU64>,
    pub(crate) received_messages: Receiver<ClientOpcodeMessage>,
    pub(crate) reader_handle: JoinHandle<()>,
    pub(crate) writer_handle: JoinHandle<()>,
    /// `true` between sending a transfer/teleport message and receiving the
    /// matching `MSG_MOVE_WORLDPORT_ACK`. The opcode handler uses it to
    /// suppress duplicate ACK processing.
    pub in_process_of_teleport: bool,
    /// Guids the client has been told about and not yet despawned. Updated
    /// every tick by the AOI-transitions phase: anything in this set that
    /// is no longer within `AOI_RADIUS_YARDS` on the same map gets sent as
    /// `OutOfRangeObjects`; anything newly in range gets sent as a fresh
    /// `CreateObject2`. Without this tracking a player who walks past the
    /// edge of AOI lingers on observer clients as a stationary ghost
    /// because no despawn opcode is ever emitted.
    pub(crate) visible_entities: ahash::AHashSet<Guid>,
    /// Per-guid timestamp of this observer's last AOI transition (either
    /// direction) for that guid. The AOI-transitions phase consults this
    /// to suppress flapping: while standing on the boundary and strafing,
    /// a mob within ~AOI_RADIUS_YARDS oscillates in/out of range each
    /// tick — without this gate that produces 10+ pairs of
    /// CreateObject/OutOfRangeObjects per second per oscillating entity.
    /// Once a transition fires, the guid is pinned to its post-transition
    /// state for `AOI_FLAP_COOLDOWN` regardless of subsequent jitter.
    pub(crate) aoi_transition_at: ahash::AHashMap<Guid, std::time::Instant>,
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
        self.queue_buf(Arc::<[u8]>::from(buf));
    }

    pub async fn send_opcode(&mut self, m: &ServerOpcodeMessage) {
        write_server_test(m);
        let mut buf = Vec::new();
        if m.write_unencrypted_server(&mut buf).is_err() {
            return;
        }
        self.queue_buf(Arc::<[u8]>::from(buf));
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
        self.queue_buf(Arc::<[u8]>::from(buf))
    }

    /// Returns `true` if the buffer was queued, `false` if the byte budget
    /// was exhausted or the channel was closed (disconnected). Drops are
    /// counted so we can spot which clients are falling behind. On the
    /// first drop per client we log a warning so the problem surfaces
    /// without an operator having to scrape the counter.
    #[inline]
    fn queue_buf(&self, buf: Arc<[u8]>) -> bool {
        if self.outbound.try_send(buf) {
            return true;
        }
        self.queue_buf_dropped();
        false
    }

    /// Cold tail of [`queue_buf`]: bump the dropped-packet counter and
    /// log on the first drop per client. Splitting this out keeps the
    /// success path of `queue_buf` straight-line — important because
    /// it's called 1400× per broadcast at full density, and the branch
    /// predictor + i-cache benefit from a tight hot body.
    #[cold]
    #[inline(never)]
    fn queue_buf_dropped(&self) {
        let prior = self.dropped_packets.fetch_add(1, Ordering::Relaxed);
        if prior == 0 {
            tracing::warn!(
                "outbound byte budget exhausted for {} ({}); dropping packets — \
                 client is falling behind",
                self.account_name,
                std::any::type_name::<Self>(),
            );
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
    #[inline]
    pub fn character(&self) -> &Character {
        &self.character
    }

    #[inline]
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
    pub fn from_parts(
        character: Character,
        received_messages: Receiver<ClientOpcodeMessage>,
        outbound: OutboundTx,
        dropped_packets: Arc<AtomicU64>,
        account_name: Arc<str>,
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
                visible_entities: ahash::AHashSet::default(),
                aoi_transition_at: ahash::AHashMap::default(),
            },
            player: Player { character },
        }
    }

    #[inline]
    pub fn character(&self) -> &Character {
        self.player.character()
    }

    #[inline]
    pub fn character_mut(&mut self) -> &mut Character {
        self.player.character_mut()
    }

    /// Snapshot a [`crate::world::aoi::BroadcastTarget`] for this
    /// client. Built once per tick at the top of the broadcast phase
    /// — the resulting `Vec<BroadcastTarget>` is the rayon-iterable,
    /// `Sync`-safe view that the fan-out loop consumes instead of
    /// `&Slab<Client>` (which is `!Sync` because `Client` embeds a
    /// `tokio::sync::mpsc::Receiver`).
    #[inline]
    pub fn broadcast_target(&self) -> crate::world::aoi::BroadcastTarget {
        let ch = self.character();
        crate::world::aoi::BroadcastTarget {
            map: ch.map,
            position: ch.info.position,
            guid: ch.guid,
            outbound: self.session.outbound.clone(),
            dropped_packets: Arc::clone(&self.session.dropped_packets),
            account_name: Arc::clone(&self.session.account_name),
        }
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
