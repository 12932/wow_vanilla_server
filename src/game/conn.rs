//! A client connection: a reader task that decodes packets, and a writer
//! task that encrypts headers and coalesces writes. Everything else talks
//! to the client through an [`Outbound`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc};
use tracing::{debug, warn};
use wow_srp::vanilla_header::{DecrypterHalf, EncrypterHalf, HeaderCrypto};
use wow_world_messages::Guid;
use wow_world_messages::errors::ExpectedOpcodeError;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::{SMSG_PONG, ServerMessage};

/// Packets written to all clients since start-up.
pub static PACKETS_OUT: AtomicU64 = AtomicU64::new(0);
/// Packets read from all clients since start-up.
pub static PACKETS_IN: AtomicU64 = AtomicU64::new(0);

/// Encode a server message as `[size BE][opcode LE][body]`, the shape the
/// writer task encrypts. Encode once and share the frame between clients.
/// None for a message too big for the size field (wow_messages would cut
/// the size short, and the client's stream would be out of step).
pub fn encode<M: ServerMessage>(m: &M) -> Option<Arc<[u8]>> {
    let body = m.size_without_header() as usize;
    if body > crate::game::update::MAX_FRAME_BODY {
        warn!(
            "not sending opcode {:#x}: its {body} bytes don't fit a frame",
            M::OPCODE
        );
        return None;
    }
    let mut buf = Vec::with_capacity(m.size_without_header() as usize + 4);
    m.write_unencrypted_server(&mut buf).ok()?;
    Some(Arc::from(buf))
}

/// What a queue carries: a frame, or a cork's frames for the client, in
/// order ([`Cork`]).
enum Item {
    One(Arc<[u8]>),
    Many(Vec<Arc<[u8]>>),
}

/// While one is alive, what this thread queues for a client waits, in the
/// order it was sent, and goes in one push a client when the last cork on
/// the thread is dropped (or at [`uncork`]): a map's burst of events and
/// its update hand each observer one batch, not a push (and a wake of its
/// writer) a packet. Only the push waits: the budget is taken, a hold
/// keeps what it keeps and a closed client takes nothing, at the send, as
/// uncorked. What other threads send isn't corked, so a map uncorks
/// before it hands a client, or word of one, to another task (see
/// `map::run`).
pub struct Cork {
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Cork {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        CORK.with_borrow_mut(|c| c.depth += 1);
        CORKED.set(true);
        Cork {
            _not_send: std::marker::PhantomData,
        }
    }
}

impl Drop for Cork {
    fn drop(&mut self) {
        let last = CORK.with_borrow_mut(|c| {
            c.depth -= 1;
            c.depth == 0
        });
        if last {
            uncork();
            CORKED.set(false);
        }
    }
}

/// Push what this thread's cork holds now, the cork staying on.
pub fn uncork() {
    // Uncorked, nothing waits: the last cork's drop pushed it.
    if !CORKED.get() {
        return;
    }
    let slots = CORK.with_borrow_mut(|c| {
        c.index.clear();
        std::mem::take(&mut c.slots)
    });
    let mut emptied = Vec::with_capacity(slots.len());
    for (inner, mut frames) in slots {
        if !inner.shared.closed.load(Ordering::Acquire) {
            let item = if frames.len() == 1 {
                Item::One(frames.pop().expect("one"))
            } else {
                Item::Many(std::mem::take(&mut frames))
            };
            let _ = inner.tx.send(item);
        }
        frames.clear();
        emptied.push((inner, frames));
    }
    // The slots' vectors kept for the next cork (the clients let go).
    CORK.with_borrow_mut(|c| {
        for (_, frames) in emptied {
            if frames.capacity() > 0 && c.spare.len() < MAX_SPARE {
                c.spare.push(frames);
            }
        }
    });
}

/// The vectors a thread's cork keeps for its next batches.
const MAX_SPARE: usize = 4096;

thread_local! {
    static CORKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static CORK: std::cell::RefCell<CorkState> = std::cell::RefCell::new(CorkState::default());
}

#[derive(Default)]
struct CorkState {
    depth: usize,
    /// Each client corked, in the order first sent to, and its frames.
    slots: Vec<(Arc<OutboundInner>, Vec<Arc<[u8]>>)>,
    /// A client's slot, by its queue's address.
    index: std::collections::HashMap<usize, usize, std::hash::BuildHasherDefault<AddressHasher>>,
    spare: Vec<Vec<Arc<[u8]>>>,
}

impl CorkState {
    fn push(&mut self, inner: &Arc<OutboundInner>, frame: Arc<[u8]>) {
        let key = Arc::as_ptr(inner) as usize;
        let slot = match self.index.get(&key) {
            Some(&i) => i,
            None => {
                let i = self.slots.len();
                let frames = self.spare.pop().unwrap_or_else(|| Vec::with_capacity(16));
                self.slots.push((inner.clone(), frames));
                self.index.insert(key, i);
                i
            }
        };
        self.slots[slot].1.push(frame);
    }
}

/// An address's hash: a multiply, its low bits (alignment) shifted out.
#[derive(Default)]
struct AddressHasher(u64);

impl std::hash::Hasher for AddressHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0 ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3);
        }
    }

    fn write_usize(&mut self, n: usize) {
        self.0 = ((n as u64) >> 4).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

/// Where packets for one client go: a queue the writer task drains, with
/// a byte budget. A client too slow to keep its queue under the budget is
/// disconnected: dropping some of its packets would leave it out of step
/// with the world.
#[derive(Clone)]
pub struct Outbound {
    inner: Arc<OutboundInner>,
}

struct OutboundInner {
    /// tokio's unbounded `mpsc`: its send is sync, from any thread, and
    /// the writer takes a batch at a time (`recv_many`).
    tx: mpsc::UnboundedSender<Item>,
    shared: Arc<Shared>,
    overflowed: AtomicBool,
    /// Frames wait in `held` (see [`Outbound::hold`]).
    holding: AtomicBool,
    held: parking_lot::Mutex<Vec<Arc<[u8]>>>,
    /// The connection's order counter and the forced movements it hasn't
    /// acknowledged (see [`Outbound::order`]).
    orders: parking_lot::Mutex<Orders>,
}

/// A forced movement a client has been sent and hasn't acknowledged, by
/// the unit it moves (Thistle Tea's `pending_movement_acks`): the client's
/// acknowledgement settles it only if it names the same unit and change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Order {
    Root {
        guid: Guid,
        root: bool,
    },
    Speed {
        guid: Guid,
        kind: SpeedKind,
        speed: f32,
    },
    WaterWalk {
        guid: Guid,
        on: bool,
    },
    FeatherFall {
        guid: Guid,
        on: bool,
    },
    Hover {
        guid: Guid,
        on: bool,
    },
    /// The launch: its direction's cosine and sine, and its horizontal and
    /// vertical speeds, as sent.
    KnockBack {
        guid: Guid,
        impulse: [f32; 4],
    },
    Teleport {
        guid: Guid,
    },
}

/// Which speed a forced speed change sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedKind {
    Run,
    RunBack,
    Swim,
    SwimBack,
    Walk,
    TurnRate,
}

/// How many orders wait for their acknowledgement at most: past it, the
/// oldest goes (a client that never acknowledges keeps no more).
const MAX_PENDING_ORDERS: usize = 64;

#[derive(Debug, Default)]
struct Orders {
    next: u32,
    pending: std::collections::VecDeque<(u32, Order)>,
}

impl Order {
    /// What it moves the client somewhere else with: a teleport, a launch.
    fn relocates(&self) -> bool {
        matches!(self, Order::Teleport { .. } | Order::KnockBack { .. })
    }
}

/// What a queue's two ends share.
struct Shared {
    /// Bytes queued and not yet taken by the writer, which takes back what
    /// it takes: a plain counter (nothing waits on it), one atomic add a
    /// frame.
    queued: AtomicUsize,
    /// The budget's bytes, all of them.
    capacity: usize,
    /// [`Outbound::close`] was called. tokio's senders can't close their
    /// channel, so from then on the queue's far end takes nothing more, and
    /// what was queued is dropped unwritten.
    closed: AtomicBool,
    /// Signalled at the close, for whoever waits on it
    /// ([`Shared::wait_closed`]): the writer, in a write that a client
    /// which doesn't read holds up; the reader, in its read; the session.
    closing: Notify,
}

impl Shared {
    /// Close, and wake whoever waits on it: true if it wasn't closed.
    fn close(&self) -> bool {
        if self.closed.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.closing.notify_waiters();
        true
    }

    /// Completes once the queue is closed.
    async fn wait_closed(&self) {
        let notified = self.closing.notified();
        let mut notified = std::pin::pin!(notified);
        // Waiting from here, so a close between the check and the wait
        // still wakes it.
        notified.as_mut().enable();
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }
}

impl std::fmt::Debug for Outbound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Outbound")
            .field("queued", &self.inner.shared.queued.load(Ordering::Relaxed))
            .finish()
    }
}

/// The far end of an [`Outbound`]: what the writer task drains, or what a
/// test reads ([`Outbound::detached`]). Once the queue is closed it hands
/// out nothing more.
pub struct Frames {
    rx: mpsc::UnboundedReceiver<Item>,
    /// The rest of a batch taken.
    batch: std::collections::VecDeque<Arc<[u8]>>,
    shared: Arc<Shared>,
}

impl Frames {
    fn closed(&self) -> bool {
        self.shared.closed.load(Ordering::Acquire)
    }

    /// The next frame, once there is one; `None` once the queue is closed
    /// or every sender has gone.
    pub async fn recv(&mut self) -> Option<Arc<[u8]>> {
        if self.closed() {
            return None;
        }
        if let Some(frame) = self.batch.pop_front() {
            return Some(frame);
        }
        let item = tokio::select! {
            item = self.rx.recv() => item?,
            () = self.shared.wait_closed() => return None,
        };
        self.unpack(item);
        if self.closed() {
            return None;
        }
        self.batch.pop_front()
    }

    /// The next frame, if one is queued.
    pub fn try_recv(&mut self) -> Option<Arc<[u8]>> {
        if self.closed() {
            return None;
        }
        if self.batch.is_empty() {
            let item = self.rx.try_recv().ok()?;
            self.unpack(item);
        }
        if self.closed() {
            return None;
        }
        self.batch.pop_front()
    }

    fn unpack(&mut self, item: Item) {
        match item {
            Item::One(f) => self.batch.push_back(f),
            Item::Many(v) => self.batch.extend(v),
        }
    }

    /// Frames queued and not yet taken.
    pub fn len(&mut self) -> usize {
        if self.closed() {
            return 0;
        }
        while let Ok(item) = self.rx.try_recv() {
            self.unpack(item);
        }
        self.batch.len()
    }

    pub fn is_empty(&mut self) -> bool {
        self.len() == 0
    }
}

/// An outbound queue with `budget` bytes.
fn queue(budget: usize) -> (Outbound, Frames) {
    let (tx, rx) = mpsc::unbounded_channel();
    let shared = Arc::new(Shared {
        queued: AtomicUsize::new(0),
        capacity: budget,
        closed: AtomicBool::new(false),
        closing: Notify::new(),
    });
    let outbound = Outbound {
        inner: Arc::new(OutboundInner {
            tx,
            shared: shared.clone(),
            overflowed: AtomicBool::new(false),
            holding: AtomicBool::new(false),
            held: parking_lot::Mutex::new(Vec::new()),
            orders: parking_lot::Mutex::new(Orders::default()),
        }),
    };
    (
        outbound,
        Frames {
            rx,
            batch: Default::default(),
            shared,
        },
    )
}

impl Outbound {
    /// The counter for the next forced movement this client is sent (a
    /// root, a forced speed, water walking, a knockback, a teleport): 0
    /// first, then one more each, as mangos-classic's session numbers its
    /// orders (`GetOrderCounter`, `IncrementOrderCounter`) and Thistle
    /// Tea's `movement_counter` stamps them, so the client's
    /// acknowledgement names the order it answers.
    pub fn next_order(&self) -> u32 {
        let mut o = self.inner.orders.lock();
        let counter = o.next;
        o.next = counter.wrapping_add(1);
        counter
    }

    /// [`Outbound::next_order`] for `order`, kept until the client
    /// acknowledges it ([`Outbound::settle`]). A teleport makes the
    /// teleports and launches before it stale, as Thistle Tea's
    /// `MovementControl` drops them: their acknowledgements name a place
    /// the client is no longer bound for.
    pub fn order(&self, order: Order) -> u32 {
        let mut o = self.inner.orders.lock();
        let counter = o.next;
        o.next = counter.wrapping_add(1);
        if matches!(order, Order::Teleport { .. }) {
            o.pending.retain(|(_, p)| !p.relocates());
        }
        if o.pending.len() >= MAX_PENDING_ORDERS {
            o.pending.pop_front();
        }
        o.pending.push_back((counter, order));
        counter
    }

    /// The client acknowledges the order `counter`: settled, and returned,
    /// if one is waiting under it and `matches` it; else nothing changes
    /// (a stale, repeated or forged acknowledgement).
    pub fn settle(&self, counter: u32, matches: impl FnOnce(&Order) -> bool) -> Option<Order> {
        let mut o = self.inner.orders.lock();
        let i = o.pending.iter().position(|(c, _)| *c == counter)?;
        if !matches(&o.pending[i].1) {
            return None;
        }
        o.pending.remove(i).map(|(_, order)| order)
    }

    /// The orders waiting go: the client is sent to another map
    /// (`SMSG_NEW_WORLD`), where none of them applies, as Thistle Tea
    /// clears them there. The counter goes on.
    pub fn forget_orders(&self) {
        self.inner.orders.lock().pending.clear();
    }

    /// The orders waiting for their acknowledgement, oldest first.
    pub fn pending_orders(&self) -> Vec<(u32, Order)> {
        self.inner.orders.lock().pending.iter().copied().collect()
    }

    /// A queue nothing drains, with an unlimited budget, and the receiving
    /// end for tests to read.
    pub fn detached() -> (Self, Frames) {
        queue(usize::MAX >> 1)
    }

    /// A queue with `budget` bytes, and its far end, for a writer
    /// ([`run_writer`]) or a test or benchmark to drain.
    pub fn with_budget(budget: usize) -> (Self, Frames) {
        queue(budget)
    }

    /// Queue a frame the budget has been taken for: false if the client is
    /// gone.
    fn enqueue(&self, frame: Arc<[u8]>) -> bool {
        if self.inner.shared.closed.load(Ordering::Acquire) {
            return false;
        }
        if CORKED.get() {
            CORK.with_borrow_mut(|c| c.push(&self.inner, frame));
            return !self.inner.tx.is_closed();
        }
        self.inner.tx.send(Item::One(frame)).is_ok()
    }

    /// Queue a framed packet. False if the client is gone or over budget;
    /// over budget, its connection is closed.
    pub fn send_frame(&self, frame: Arc<[u8]>) -> bool {
        if self.inner.holding.load(Ordering::Acquire) {
            let n = frame.len();
            let mut held = self.inner.held.lock();
            // Not released since: it waits behind what is held.
            if self.inner.holding.load(Ordering::Acquire) {
                if !self.take_budget(n) {
                    drop(held);
                    self.overflow();
                    return false;
                }
                held.push(frame);
                return !self.is_closed();
            }
        }
        self.send_frame_unheld(frame)
    }

    /// Queue a frame past any hold: for what names no object a create could
    /// be waiting for, and is waited on (a pong, a query's answer).
    pub fn send_frame_unheld(&self, frame: Arc<[u8]>) -> bool {
        if !self.take_budget(frame.len()) {
            self.overflow();
            return false;
        }
        self.enqueue(frame)
    }

    /// `n` bytes of the budget for a frame: false, and nothing taken, if
    /// they aren't left.
    fn take_budget(&self, n: usize) -> bool {
        let shared = &self.inner.shared;
        let before = shared.queued.fetch_add(n, Ordering::Relaxed);
        if before.saturating_add(n) <= shared.capacity {
            return true;
        }
        shared.queued.fetch_sub(n, Ordering::Relaxed);
        false
    }

    /// Over budget: the client is disconnected, and counted once.
    fn overflow(&self) {
        if !self.inner.overflowed.swap(true, Ordering::Relaxed) {
            crate::game::stats::BUDGET.fetch_add(1, Ordering::Relaxed);
            warn!("a client's outbound queue is over budget; disconnecting it");
        }
        self.close();
    }

    /// From now on, what is sent waits, in the order it was sent, for
    /// [`Outbound::release`]. A map holds a client it has queued creates
    /// for, so that nothing it sends naming those objects arrives before
    /// them. True if the client wasn't held already.
    pub fn hold(&self) -> bool {
        !self.inner.holding.swap(true, Ordering::AcqRel)
    }

    /// Whether frames are waiting on a hold.
    pub fn has_held(&self) -> bool {
        self.inner.holding.load(Ordering::Acquire) && !self.inner.held.lock().is_empty()
    }

    /// Queue `first` (the blocks the hold was for), then what waited; what
    /// is sent from now on goes at once again.
    pub fn release(&self, first: Vec<Arc<[u8]>>) {
        let mut held = self.inner.held.lock();
        for frame in first {
            self.send_frame_unheld(frame);
        }
        for frame in held.drain(..) {
            self.enqueue(frame);
        }
        self.inner.holding.store(false, Ordering::Release);
    }

    pub fn send<M: ServerMessage>(&self, m: &M) -> bool {
        match encode(m) {
            Some(frame) => self.send_frame(frame),
            None => false,
        }
    }

    /// [`Outbound::send`] past any hold ([`Outbound::send_frame_unheld`]).
    pub fn send_unheld<M: ServerMessage>(&self, m: &M) -> bool {
        match encode(m) {
            Some(frame) => self.send_frame_unheld(frame),
            None => false,
        }
    }

    /// Close the queue: nothing more is queued, what is queued and not yet
    /// taken is dropped, the writer task ends at once (a write under way
    /// left unfinished), the reader and the session end, and the
    /// connection closes.
    pub fn close(&self) {
        self.inner.shared.close();
    }

    /// Completes once the queue is closed or its writer has gone: what the
    /// reader and the session wait on beside their own work, to end with
    /// their connection.
    pub async fn closed(&self) {
        tokio::select! {
            () = self.inner.shared.wait_closed() => {}
            () = self.inner.tx.closed() => {}
        }
    }

    /// Bytes queued for the writer, not yet written.
    pub fn queued_bytes(&self) -> usize {
        self.inner.shared.queued.load(Ordering::Relaxed)
    }

    /// Closed, or the writer has gone.
    pub fn is_closed(&self) -> bool {
        self.inner.shared.closed.load(Ordering::Acquire) || self.inner.tx.is_closed()
    }

    /// Two handles to the same client.
    pub fn same(&self, other: &Outbound) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// A packet a client sent: one wow_messages reads, or (for those it lays
/// out differently from 1.12) its opcode and body.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)] // Each packet moves through the channel once.
pub enum Inbound {
    Msg(ClientOpcodeMessage),
    Raw(u16, Vec<u8>),
}

impl From<ClientOpcodeMessage> for Inbound {
    fn from(m: ClientOpcodeMessage) -> Self {
        Inbound::Msg(m)
    }
}

/// The client opcodes read by hand: `CMSG_SET_FACTION_ATWAR`,
/// `CMSG_SET_FACTION_INACTIVE` and `CMSG_SET_WATCHED_FACTION` carry a
/// 32-bit reputation index where wow_messages reads a faction,
/// wow_messages's decompression of `CMSG_UPDATE_ACCOUNT_DATA` fails, it
/// wants ten answers in `CMSG_GMSURVEY_SUBMIT` where the client stops
/// at a zero question.
pub const RAW_OPCODES: &[u32] = &[0x125, 0x317, 0x318, 0x20B, 0x32A];

/// A started connection: its outbound queue, and the packets it reads.
pub struct Connection {
    pub outbound: Outbound,
    pub inbound: mpsc::Receiver<Inbound>,
}

/// Start the reader and writer tasks for an authenticated stream.
pub fn start(stream: TcpStream, crypto: HeaderCrypto) -> Connection {
    let _ = stream.set_nodelay(true);
    let (read, write) = stream.into_split();
    let (encrypter, decrypter) = crypto.split();
    let network = &crate::config::config().network;
    let (outbound, frames) = queue(network.outbound_channel_bytes);
    // A client that takes nothing for as long as one may send nothing is
    // gone.
    tokio::spawn(run_writer(
        write,
        encrypter,
        frames,
        network.client_idle_timeout(),
    ));
    let (in_tx, in_rx) = mpsc::channel(256);
    tokio::spawn(run_reader(read, decrypter, in_tx, outbound.clone()));
    Connection {
        outbound,
        inbound: in_rx,
    }
}

/// Decode packets and pass them on. Pings are answered here, so their
/// round trip measures the network and not the world, and counted here
/// ([`PingRate`]).
pub(crate) async fn run_reader<R: AsyncRead + Unpin + Send>(
    mut read: R,
    mut decrypter: DecrypterHalf,
    tx: mpsc::Sender<Inbound>,
    outbound: Outbound,
) {
    let network = &crate::config::config().network;
    let idle = network.client_idle_timeout();
    let mut bad = BadPacketLimit::new();
    let mut pings = PingRate::new(network.max_overspeed_pings);
    loop {
        // Closed (over budget, taken over, its map gone), the connection
        // ends now, its socket with it: not at the client's next packet.
        let read_one = tokio::time::timeout(idle, read_packet(&mut read, &mut decrypter));
        let got = tokio::select! {
            biased;
            () = outbound.closed() => break,
            got = read_one => got,
        };
        let Ok(msg) = got else {
            debug!("client sent nothing for {idle:?}; disconnecting");
            break;
        };
        let msg = match msg {
            Ok(Some(Inbound::Msg(m))) => m,
            // wow_messages panicked reading it.
            Ok(None) => {
                if bad.exceeded() {
                    break;
                }
                continue;
            }
            Ok(Some(raw)) => {
                PACKETS_IN.fetch_add(1, Ordering::Relaxed);
                if tx.send(raw).await.is_err() {
                    break;
                }
                continue;
            }
            // wow_messages reads a packet's whole body before dispatching on
            // its opcode, so an unknown or unparseable packet is already
            // consumed: skip it.
            Err(ExpectedOpcodeError::Opcode { opcode, size, name }) => {
                debug!("unhandled opcode {name:?} (0x{opcode:X}, {size} bytes)");
                if bad.exceeded() {
                    break;
                }
                continue;
            }
            Err(ExpectedOpcodeError::Parse(p)) => {
                debug!("unparseable client packet: {p:?}");
                if bad.exceeded() {
                    break;
                }
                continue;
            }
            Err(ExpectedOpcodeError::Io(_)) => break,
        };
        PACKETS_IN.fetch_add(1, Ordering::Relaxed);
        if let ClientOpcodeMessage::CMSG_PING(ping) = &msg {
            if !pings.allow(Instant::now()) {
                warn!("a client pings too fast; disconnecting it");
                break;
            }
            outbound.send_unheld(&SMSG_PONG {
                sequence_id: ping.sequence_id,
            });
            continue;
        }
        if tx.send(Inbound::Msg(msg)).await.is_err() {
            break;
        }
        if outbound.is_closed() {
            break;
        }
    }
    outbound.close();
}

/// Read one packet: its header decrypted, its body whole; those read by
/// hand stay raw, the rest go to wow_messages, which reserves no more than
/// a body could hold however much it claims. `None` for a packet that
/// wow_messages panicked on, as a backstop.
async fn read_packet<R: AsyncRead + Unpin + Send>(
    read: &mut R,
    decrypter: &mut DecrypterHalf,
) -> Result<Option<Inbound>, ExpectedOpcodeError> {
    let mut header = [0u8; 6];
    read.read_exact(&mut header).await?;
    let header = decrypter.decrypt_client_header(header);
    let size = header.size.saturating_sub(4) as usize;
    let mut body = vec![0u8; size];
    read.read_exact(&mut body).await?;
    if RAW_OPCODES.contains(&header.opcode) {
        return Ok(Some(Inbound::Raw(header.opcode as u16, body)));
    }
    let size = body.len();
    let mut plain = Vec::with_capacity(6 + size);
    plain.extend_from_slice(&((size + 4) as u16).to_be_bytes());
    plain.extend_from_slice(&header.opcode.to_le_bytes());
    plain.extend_from_slice(&body);
    match crate::game::supervisor::catch(|| ClientOpcodeMessage::read_unencrypted(&plain[..])) {
        Ok(m) => m.map(|m| Some(Inbound::Msg(m))),
        Err(e) => {
            debug!(
                "wow_messages panicked reading client packet 0x{:X}: {e}; dropped",
                header.opcode
            );
            Ok(None)
        }
    }
}

/// The largest auth session the world handshake reads.
pub const MAX_AUTH_SESSION: usize = 8 * 1024;

/// Drain queued frames, encrypt each header in order (ARC4 is a stream
/// cipher) and write each burst at once. The writer ends at the close,
/// whether it waits for frames or is in a write, and when a write makes no
/// progress for `stall`: a client that doesn't read (its window shut, its
/// acknowledgements still coming) holds its socket and its frames no
/// longer than that, nor past the close.
pub async fn run_writer<W>(
    mut write: W,
    mut encrypter: EncrypterHalf,
    mut frames: Frames,
    stall: Duration,
) where
    W: AsyncWrite + Unpin,
{
    const BATCH_LIMIT: usize = 128;
    let mut staging: Vec<Item> = Vec::with_capacity(BATCH_LIMIT);
    let mut scratch: Vec<u8> = Vec::with_capacity(16 * 1024);
    let shared = frames.shared.clone();
    loop {
        staging.clear();
        // Waits for a frame, then takes what else is queued: none once every
        // sender has gone. Closed, what was taken goes unwritten.
        let taken = tokio::select! {
            n = frames.rx.recv_many(&mut staging, BATCH_LIMIT) => n,
            () = shared.wait_closed() => 0,
        };
        if taken == 0 || frames.closed() {
            break;
        }
        scratch.clear();
        let mut written = 0u64;
        let mut drained = 0usize;
        for item in staging.drain(..) {
            match item {
                Item::One(buf) => {
                    drained += buf.len();
                    written += put(&mut scratch, &mut encrypter, &buf);
                }
                Item::Many(bufs) => {
                    for buf in bufs {
                        drained += buf.len();
                        written += put(&mut scratch, &mut encrypter, &buf);
                    }
                }
            }
        }
        shared.queued.fetch_sub(drained, Ordering::Relaxed);
        if scratch.is_empty() {
            continue;
        }
        let wrote = tokio::select! {
            r = write_within(&mut write, &scratch, stall) => r,
            () = shared.wait_closed() => Err(std::io::ErrorKind::ConnectionAborted.into()),
        };
        if let Err(e) = wrote {
            debug!("a client's write ended: {e}");
            break;
        }
        PACKETS_OUT.fetch_add(written, Ordering::Relaxed);
        crate::game::stats::BYTES_OUT.fetch_add(scratch.len() as u64, Ordering::Relaxed);
        // A burst of creates doesn't keep its megabytes for good.
        if scratch.capacity() > SCRATCH_KEPT {
            scratch = Vec::with_capacity(16 * 1024);
        }
    }
    // The senders, the reader and the session see the queue closed, and
    // what it held is let go.
    shared.close();
    drop(staging);
    drop(scratch);
    drop(frames);
    // A TCP half's shutdown doesn't wait on the client; bounded all the
    // same, as the write was.
    let _ = tokio::time::timeout(stall, write.shutdown()).await;
}

/// A frame onto the burst: 1 if it went. Its first four bytes are the
/// header as the client reads it, size and opcode: copied whole, and the
/// header encrypted where it lands, in the order the frames are written
/// (ARC4 is a stream cipher).
fn put(scratch: &mut Vec<u8>, encrypter: &mut EncrypterHalf, buf: &[u8]) -> u64 {
    if buf.len() < 4 {
        return 0;
    }
    let size = u16::from_be_bytes([buf[0], buf[1]]);
    // A size that isn't the frame's would put the client's stream out of
    // step: what is built refuses one, and nothing gets past here.
    if usize::from(size) != buf.len() - 2 {
        let opcode = u16::from_le_bytes([buf[2], buf[3]]);
        warn!(
            "not writing opcode {opcode:#x}: its size field says {size}, its frame {}",
            buf.len()
        );
        return 0;
    }
    let at = scratch.len();
    scratch.extend_from_slice(buf);
    encrypter.encrypt(&mut scratch[at..at + 4]);
    1
}

/// The most of its buffer a writer keeps between bursts.
const SCRATCH_KEPT: usize = 1 << 20;

/// Write all of `buf`, or fail once a write has taken nothing for `stall`.
async fn write_within<W>(write: &mut W, mut buf: &[u8], stall: Duration) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    while !buf.is_empty() {
        match tokio::time::timeout(stall, write.write(buf)).await {
            Ok(Ok(0)) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(Ok(n)) => buf = &buf[n..],
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                crate::game::stats::STALL.fetch_add(1, Ordering::Relaxed);
                warn!("a client took nothing for {stall:?}; disconnecting it");
                return Err(std::io::ErrorKind::TimedOut.into());
            }
        }
    }
    Ok(())
}

/// mangos's rule for pings (`WorldSocket::HandlePing`): the reader answers
/// them before the session's flood guard, so they are counted here. One
/// less than [`PingRate::SOONEST`] after the last is too soon; more than
/// `max` too soon in a row disconnects (0 never does); one in time starts
/// the count again.
struct PingRate {
    max: u32,
    last: Option<Instant>,
    too_soon: u32,
}

impl PingRate {
    /// The client pings every 30 s.
    const SOONEST: Duration = Duration::from_secs(27);

    fn new(max: u32) -> Self {
        Self {
            max,
            last: None,
            too_soon: 0,
        }
    }

    /// Whether a ping at `now` is let through.
    fn allow(&mut self, now: Instant) -> bool {
        let Some(last) = self.last.replace(now) else {
            return true;
        };
        if now.saturating_duration_since(last) >= Self::SOONEST {
            self.too_soon = 0;
            return true;
        }
        self.too_soon = self.too_soon.saturating_add(1);
        self.max == 0 || self.too_soon <= self.max
    }
}

/// How many unknown or unparseable packets a client may send in a window
/// before it is disconnected.
const BAD_PACKETS_PER_WINDOW: u32 = 32;
const BAD_PACKET_WINDOW: Duration = Duration::from_secs(10);

struct BadPacketLimit {
    window_start: Instant,
    count: u32,
}

impl BadPacketLimit {
    fn new() -> Self {
        Self {
            window_start: Instant::now(),
            count: 0,
        }
    }

    fn exceeded(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.window_start) > BAD_PACKET_WINDOW {
            self.window_start = now;
            self.count = 0;
        }
        self.count += 1;
        if self.count > BAD_PACKETS_PER_WINDOW {
            warn!(
                "client sent {} bad packets in {BAD_PACKET_WINDOW:?}; disconnecting",
                self.count
            );
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, duplex};
    use wow_srp::normalized_string::NormalizedString;
    use wow_srp::vanilla_header::ProofSeed;

    pub(crate) fn paired_crypto() -> (HeaderCrypto, HeaderCrypto) {
        let session_key = [0u8; 40];
        let username = NormalizedString::new("test".to_string()).unwrap();
        let server_seed = ProofSeed::new();
        let server_seed_value = server_seed.seed();
        let client_seed = ProofSeed::new();
        let client_seed_value = client_seed.seed();
        let (client_proof, client) =
            client_seed.into_client_header_crypto(&username, session_key, server_seed_value);
        let server = server_seed
            .into_server_header_crypto(&username, session_key, client_proof, client_seed_value)
            .unwrap();
        (server, client)
    }

    fn frame(opcode: u16, body: &[u8]) -> Arc<[u8]> {
        crate::game::update::frame(opcode, body)
    }

    /// A stall bound no test reaches.
    const LONG: Duration = Duration::from_secs(600);

    /// Review 5's net 1: a writer in a write that a client which doesn't
    /// read holds up ends at the close, with every sender gone, while the
    /// client is still there: before, it stayed in the write, holding the
    /// socket and its frames, until the client went.
    #[tokio::test]
    async fn a_writer_on_a_client_that_doesnt_read_ends_at_its_close() {
        let (server, _) = paired_crypto();
        let (encrypter, _) = server.split();
        let (peer, b) = duplex(64);
        let (out, frames) = queue(1 << 20);
        let writer = tokio::spawn(run_writer(b, encrypter, frames, LONG));
        assert!(out.send_frame(frame(0x1F6, &[0; 16 * 1024])));
        // In the write: the frame taken, the pipe full.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while out.queued_bytes() > 0 {
            assert!(tokio::time::Instant::now() < deadline, "the writer takes");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!writer.is_finished(), "held up in the write");
        out.close();
        drop(out);
        tokio::time::timeout(Duration::from_secs(5), writer)
            .await
            .expect("the writer ends at the close, the client still there")
            .unwrap();
        drop(peer);
    }

    /// A write that takes nothing for the stall bound ends the writer, and
    /// the queue is closed for its senders, with no close asked for.
    #[tokio::test]
    async fn a_write_that_takes_nothing_ends_the_writer() {
        let (server, _) = paired_crypto();
        let (encrypter, _) = server.split();
        let (peer, b) = duplex(64);
        let (out, frames) = queue(1 << 20);
        let stall = Duration::from_millis(200);
        let writer = tokio::spawn(run_writer(b, encrypter, frames, stall));
        assert!(out.send_frame(frame(0x1F6, &[0; 16 * 1024])));
        tokio::time::timeout(Duration::from_secs(5), writer)
            .await
            .expect("the writer gives up")
            .unwrap();
        assert!(out.is_closed());
        tokio::time::timeout(Duration::from_secs(1), out.closed())
            .await
            .expect("and whoever waits on the close is woken");
        drop(peer);
    }

    /// A close wakes whoever waits on it, from before or after it.
    #[tokio::test]
    async fn a_close_wakes_its_waiters() {
        let (out, _frames) = queue(1 << 20);
        let waiting = {
            let out = out.clone();
            tokio::spawn(async move { out.closed().await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiting.is_finished());
        out.close();
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("woken")
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), out.closed())
            .await
            .expect("closed already");
    }

    #[tokio::test]
    async fn the_writer_keeps_the_cipher_in_step_across_bursts() {
        let (server, client) = paired_crypto();
        let (encrypter, _) = server.split();
        let (_, mut decrypter) = client.split();
        let (mut a, b) = duplex(64 * 1024);
        let (out, frames) = queue(1 << 20);
        let writer = tokio::spawn(run_writer(b, encrypter, frames, LONG));
        for burst in [1u16, 3, 2] {
            for i in 0..burst {
                assert!(out.send_frame(frame(100 + i, &[i as u8; 4])));
            }
            for i in 0..burst {
                let mut header = [0u8; 4];
                a.read_exact(&mut header).await.unwrap();
                let h = decrypter.decrypt_server_header(header);
                assert_eq!(h.opcode, 100 + i);
                assert_eq!(h.size, 6);
                let mut body = [0u8; 4];
                a.read_exact(&mut body).await.unwrap();
                assert_eq!(body, [i as u8; 4]);
            }
        }
        assert_eq!(out.queued_bytes(), 0, "the writer gave the budget back");
        drop(out);
        writer.await.unwrap();
    }

    /// Review 5's world finding 1: a body over 64 KB went out with its size
    /// field cut short, and the client read the rest of it as the packets
    /// after it. A frame too big isn't built, and the writer doesn't write
    /// one whose size field isn't its length.
    #[tokio::test]
    async fn a_frame_too_big_for_its_size_field_is_never_sent() {
        use crate::game::update::MAX_FRAME_BODY;
        assert!(frame(9, &vec![0; MAX_FRAME_BODY + 1]).is_empty());
        let fits = frame(9, &vec![0; MAX_FRAME_BODY]);
        assert_eq!(
            usize::from(u16::from_be_bytes([fits[0], fits[1]])),
            fits.len() - 2
        );
        let note = wow_world_messages::vanilla::SMSG_NOTIFICATION {
            notification: "a".repeat(MAX_FRAME_BODY),
        };
        assert!(encode(&note).is_none(), "wow_messages would cut its size");

        let (server, client) = paired_crypto();
        let (encrypter, _) = server.split();
        let (_, mut decrypter) = client.split();
        let (mut a, b) = duplex(256 * 1024);
        let (out, frames) = queue(1 << 20);
        let writer = tokio::spawn(run_writer(b, encrypter, frames, LONG));
        // 70,000 bytes under a size of 0xFFFF, as `frame` built it before.
        let mut lying = vec![0xFF, 0xFF, 0xDD, 0x00];
        lying.extend(std::iter::repeat_n(0xAB, 70_000));
        assert!(out.send_frame(Arc::from(lying)));
        assert!(out.send_frame(frame(100, &[7; 4])));
        let mut header = [0u8; 4];
        a.read_exact(&mut header).await.unwrap();
        let h = decrypter.decrypt_server_header(header);
        assert_eq!((h.opcode, h.size), (100, 6), "the next frame, in step");
        let mut body = [0u8; 4];
        a.read_exact(&mut body).await.unwrap();
        assert_eq!(body, [7; 4]);
        drop(out);
        writer.await.unwrap();
    }

    /// Closed, a queue takes nothing more and hands out nothing more, and
    /// its writer ends and closes the stream.
    #[tokio::test]
    async fn a_closed_queue_ends_its_writer_and_the_stream() {
        let (server, _) = paired_crypto();
        let (encrypter, _) = server.split();
        let (mut a, b) = duplex(64 * 1024);
        let (out, frames) = queue(1 << 20);
        let writer = tokio::spawn(run_writer(b, encrypter, frames, LONG));
        assert!(out.send_frame(frame(1, &[0; 4])));
        let mut first = [0u8; 8];
        a.read_exact(&mut first).await.unwrap();
        out.close();
        assert!(out.is_closed());
        assert!(!out.send_frame(frame(2, &[0; 4])));
        tokio::time::timeout(Duration::from_secs(10), writer)
            .await
            .expect("the writer ends")
            .unwrap();
        let mut rest = Vec::new();
        a.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty(), "nothing after the close: {rest:?}");

        let (out, mut frames) = Outbound::detached();
        assert!(out.send_frame(frame(1, &[0; 4])));
        out.close();
        assert!(frames.try_recv().is_none(), "what was queued is dropped");
        assert!(!out.send_frame(frame(2, &[0; 4])));
        assert!(frames.recv().await.is_none());
    }

    /// A writer that has gone (its client's socket closed) closes the
    /// queue for its senders.
    #[tokio::test]
    async fn a_queue_whose_writer_has_gone_is_closed() {
        let (server, _) = paired_crypto();
        let (encrypter, _) = server.split();
        let (a, b) = duplex(64);
        drop(a);
        let (out, frames) = queue(1 << 20);
        let writer = tokio::spawn(run_writer(b, encrypter, frames, LONG));
        assert!(out.send_frame(frame(1, &[0; 4])));
        tokio::time::timeout(Duration::from_secs(10), writer)
            .await
            .expect("the writer ends")
            .unwrap();
        assert!(out.is_closed());
        assert!(!out.send_frame(frame(2, &[0; 4])));
    }

    #[tokio::test]
    async fn the_reader_hands_on_packets_wow_messages_reads_and_raw_ones() {
        let (server, client) = paired_crypto();
        let (_, decrypter) = server.split();
        let (mut encrypter, _) = client.split();
        let (mut a, b) = duplex(64 * 1024);
        let (tx, mut rx) = mpsc::channel(8);
        let (outbound, _frames) = queue(1 << 20);
        tokio::spawn(run_reader(b, decrypter, tx, outbound));
        // CMSG_SET_WATCHED_FACTION, read by hand: index 3.
        let body = 3u32.to_le_bytes();
        a.write_all(&encrypter.encrypt_client_header(4 + 4, 0x318))
            .await
            .unwrap();
        a.write_all(&body).await.unwrap();
        // CMSG_LOGOUT_REQUEST, which wow_messages reads.
        a.write_all(&encrypter.encrypt_client_header(4, 0x4B))
            .await
            .unwrap();
        match rx.recv().await {
            Some(Inbound::Raw(0x318, b)) => assert_eq!(b, body),
            other => panic!("{other:?}"),
        }
        match rx.recv().await {
            Some(Inbound::Msg(ClientOpcodeMessage::CMSG_LOGOUT_REQUEST)) => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_client_over_budget_is_disconnected() {
        let (out, _frames) = queue(10);
        assert!(out.send_frame(frame(1, &[0; 4])));
        assert!(!out.send_frame(frame(1, &[0; 4])));
        assert!(out.is_closed());
    }

    #[test]
    fn what_is_sent_while_held_follows_what_the_hold_was_for_in_order() {
        let (out, mut rx) = Outbound::detached();
        assert!(out.send_frame(frame(1, &[0; 4])));
        assert!(out.hold());
        assert!(!out.hold(), "held once");
        assert!(!out.has_held());
        assert!(out.send_frame(frame(3, &[0; 4])));
        assert!(out.send_frame(frame(4, &[0; 4])));
        assert!(out.has_held());
        assert_eq!(rx.len(), 1, "the frames wait");
        out.release(vec![frame(2, &[0; 4])]);
        assert!(out.send_frame(frame(5, &[0; 4])));
        let opcodes: Vec<u16> = std::iter::from_fn(|| rx.try_recv())
            .map(|f| u16::from_le_bytes([f[2], f[3]]))
            .collect();
        assert_eq!(opcodes, [1, 2, 3, 4, 5]);
    }

    fn opcodes(rx: &mut Frames) -> Vec<u16> {
        std::iter::from_fn(|| rx.try_recv())
            .map(|f| u16::from_le_bytes([f[2], f[3]]))
            .collect()
    }

    /// Corked, what is sent waits for the cork's end and goes in one push,
    /// in order; the budget is taken at the send, as uncorked.
    #[test]
    fn a_cork_pushes_each_client_its_frames_at_its_end_in_order() {
        let (a, mut ra) = Outbound::detached();
        let (b, mut rb) = Outbound::detached();
        {
            let _cork = Cork::new();
            for i in 1..=3 {
                assert!(a.send_frame(frame(i, &[0; 4])));
                assert!(b.send_frame(frame(10 + i, &[0; 4])));
            }
            assert_eq!(a.queued_bytes(), 3 * 8, "the budget taken at the send");
            assert!(ra.rx.is_empty() && rb.rx.is_empty(), "nothing pushed yet");
            {
                let _inner = Cork::new();
                assert!(a.send_frame(frame(4, &[0; 4])));
            }
            assert!(ra.rx.is_empty(), "an inner cork's end pushes nothing");
        }
        assert_eq!(ra.rx.len(), 1, "one push");
        assert_eq!(opcodes(&mut ra), [1, 2, 3, 4]);
        assert_eq!(opcodes(&mut rb), [11, 12, 13]);
        // Uncorked again: each frame goes at once.
        assert!(a.send_frame(frame(5, &[0; 4])));
        assert_eq!(ra.rx.len(), 1);
    }

    /// `uncork` pushes what waits, the cork staying on.
    #[test]
    fn uncork_pushes_what_waits_and_the_cork_stays() {
        let (a, mut ra) = Outbound::detached();
        let _cork = Cork::new();
        a.send_frame(frame(1, &[0; 4]));
        uncork();
        assert_eq!(opcodes(&mut ra), [1]);
        a.send_frame(frame(2, &[0; 4]));
        assert!(ra.rx.is_empty(), "corked still");
        drop(_cork);
        assert_eq!(opcodes(&mut ra), [2]);
    }

    /// A hold under a cork keeps the order it keeps uncorked: what went
    /// before it, then the hold's blocks, then what waited, then the rest.
    #[test]
    fn a_hold_under_a_cork_keeps_its_order() {
        let (out, mut rx) = Outbound::detached();
        let cork = Cork::new();
        assert!(out.send_frame(frame(1, &[0; 4])));
        assert!(out.hold());
        assert!(out.send_frame(frame(3, &[0; 4])));
        assert!(out.send_unheld(&SMSG_PONG { sequence_id: 0 }));
        assert!(out.send_frame(frame(4, &[0; 4])));
        assert!(out.has_held());
        out.release(vec![frame(2, &[0; 4])]);
        assert!(out.send_frame(frame(5, &[0; 4])));
        drop(cork);
        assert_eq!(opcodes(&mut rx), [1, 0x1DD, 2, 3, 4, 5]);
    }

    /// Over budget corked, a client is disconnected at the send; a closed
    /// client's corked frames are dropped.
    #[test]
    fn a_corked_client_over_budget_or_closed_gets_nothing() {
        let (out, mut rx) = queue(10);
        let cork = Cork::new();
        assert!(out.send_frame(frame(1, &[0; 4])));
        assert!(!out.send_frame(frame(1, &[0; 4])));
        assert!(out.is_closed());
        drop(cork);
        assert!(rx.try_recv().is_none());
        assert!(rx.rx.is_empty(), "nothing pushed to a closed client");
    }

    /// Another thread's sends aren't corked by this one's cork.
    #[test]
    fn a_cork_is_the_threads_own() {
        let (out, mut rx) = Outbound::detached();
        let _cork = Cork::new();
        let other = out.clone();
        std::thread::spawn(move || assert!(other.send_frame(frame(7, &[0; 4]))))
            .join()
            .unwrap();
        assert_eq!(opcodes(&mut rx), [7]);
    }

    #[test]
    fn a_held_client_over_budget_is_disconnected() {
        let (out, _frames) = queue(10);
        out.hold();
        assert!(out.send_frame(frame(1, &[0; 4])));
        assert!(!out.send_frame(frame(1, &[0; 4])));
        assert!(out.is_closed());
    }

    /// mangos's ping rule: a ping every 30 s goes on for ever; more than
    /// `max` in a row each less than 27 s after the last disconnects, and
    /// one in time starts the count again; 0 never disconnects.
    #[test]
    fn pings_too_soon_in_a_row_disconnect_and_one_in_time_forgives() {
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);
        let mut steady = PingRate::new(2);
        assert!((0..100).all(|i| steady.allow(at(i * 30))));
        let mut fast = PingRate::new(2);
        assert!(fast.allow(at(0)));
        assert!(fast.allow(at(1)) && fast.allow(at(2)), "two too soon");
        assert!(fast.allow(at(40)), "one in time");
        assert!(fast.allow(at(41)) && fast.allow(at(42)));
        assert!(!fast.allow(at(43)), "the third too soon in a row");
        let mut off = PingRate::new(0);
        assert!((0..100).all(|_| off.allow(start)));
    }

    /// Review 5's net 5: a flood of pings ends the connection, where each
    /// was answered and none counted.
    #[tokio::test]
    async fn a_flood_of_pings_disconnects() {
        let (server, client) = paired_crypto();
        let (_, decrypter) = server.split();
        let (mut encrypter, _) = client.split();
        let (mut a, b) = duplex(1 << 20);
        let (tx, _rx) = mpsc::channel(8);
        let (outbound, mut frames) = queue(1 << 20);
        let reader = tokio::spawn(run_reader(b, decrypter, tx, outbound.clone()));
        for i in 0..1000u32 {
            let mut body = i.to_le_bytes().to_vec();
            body.extend_from_slice(&0u32.to_le_bytes());
            let mut packet = encrypter.encrypt_client_header(4 + 8, 0x1DC).to_vec();
            packet.extend_from_slice(&body);
            if a.write_all(&packet).await.is_err() {
                break;
            }
        }
        tokio::time::timeout(Duration::from_secs(10), reader)
            .await
            .expect("the reader ends")
            .unwrap();
        assert!(outbound.is_closed());
        let max = crate::config::config().network.max_overspeed_pings as usize;
        let mut pongs = 0;
        // Closed, the queue hands out nothing: what it held, counted.
        while let Ok(item) = frames.rx.try_recv() {
            pongs += match item {
                Item::One(_) => 1,
                Item::Many(v) => v.len(),
            };
        }
        assert_eq!(pongs, 1 + max, "the first, and those let through");
    }

    #[test]
    fn a_burst_of_bad_packets_disconnects_and_a_few_do_not() {
        let mut limit = BadPacketLimit::new();
        for _ in 0..BAD_PACKETS_PER_WINDOW {
            assert!(!limit.exceeded());
        }
        assert!(limit.exceeded());
    }
}
